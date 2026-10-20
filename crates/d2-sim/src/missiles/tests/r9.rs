//! The server-do / server-hit bodies of `bodies` on the missile fake:
//! synthetic rows, the seams scripted by `Bodies`.

use super::*;
use crate::combat::DamageRecord;
use crate::missiles::bodies::{elem_len, elem_roll, next_unit, unit_distance};

/// A world with one live missile of row 0 (`r`) and an optional row 1.
fn world(r: MissileRow, extra: Option<MissileRow>) -> (World, UnitId) {
    let mut w = World::new(r);
    if let Some(e) = extra {
        w.tables.push(e);
    }
    let mut p = w.params();
    p.level = 10;
    p.skill = 7;
    let m = w.create(&p).expect("missile");
    w.fake.log.clear();
    w.fake.calls.clear();
    (w, m)
}

fn srv_do(w: &mut World, i: i16, m: UnitId) -> i32 {
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    catalogue::run_srv_do(&mut w.game, &mut cx, i, m)
}

fn srv_hit(w: &mut World, i: i16, m: UnitId, u: Option<UnitId>) -> i32 {
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    catalogue::run_srv_hit(&mut w.game, &mut cx, i, m, u, 0)
}

#[test]
fn srvdo_2_sub_missile_on_a_new_step() {
    let mut r = row();
    r.submissile1 = 0;
    r.srvcalc1 = 4;
    let (mut w, m) = world(r, None);
    // No new step: the formula is still evaluated, nothing is created.
    w.fake.mb.calc = 3;
    srv_do(&mut w, 2, m);
    assert_eq!(w.fake.logged("calc"), 1);
    assert_eq!(w.fake.logged("alloc"), 0);
    // A new step: a static sub-missile with loops 2 × 10 − 2 (flag 8).
    w.fake.mb.new_step = true;
    let before = w.store.missiles().count();
    srv_do(&mut w, 2, m);
    assert_eq!(w.fake.logged("alloc 0"), 1);
    assert_eq!(w.store.missiles().count(), before + 1);
    // SubMissile1 < 0: no formula, no sub-missile.
    let mut r = row();
    r.submissile1 = 0xFFFF;
    let (mut w, m) = world(r, None);
    w.fake.mb.new_step = true;
    srv_do(&mut w, 2, m);
    assert_eq!((w.fake.logged("calc"), w.fake.logged("alloc")), (0, 0));
}

#[test]
fn srvdo_3_marks_the_cell_only_when_still() {
    let (mut w, m) = world(row(), None);
    w.fake.paths.entry(m).or_default().velocity = 5;
    srv_do(&mut w, 3, m);
    assert_eq!(w.fake.logged("or "), 0);
    w.fake.paths.entry(m).or_default().velocity = 0;
    srv_do(&mut w, 3, m);
    let (x, y) = w.fake.pos[&m];
    assert!(w.fake.log.contains(&format!("or {x} {y} 0x40")));
}

#[test]
fn srvdo_5_animation_loop() {
    let mut r = row();
    (r.substart, r.substop) = (4, 8);
    let (mut w, m) = world(r, None);
    // f = S − 1: (S − 1 + roll(E − S)) << 8, one draw.
    w.fake.mb.frames.insert(m, 3 << 8);
    let mut s = w.fake.seeds[&m];
    let want = (3 + s.roll(4) as i32) << 8;
    srv_do(&mut w, 5, m);
    assert_eq!(w.fake.mb.frames[&m], want);
    assert_eq!(w.fake.seeds[&m], s);
    assert!(w.fake.log.contains(&format!("stamp {} 0x40", m.0)));
    // Frames left = S: (S − 3) << 8.
    w.store.get_mut(m).unwrap().current = 4;
    w.fake.mb.frames.insert(m, 6 << 8);
    srv_do(&mut w, 5, m);
    assert_eq!(w.fake.mb.frames[&m], 1 << 8);
    // Frames left < S: max(f − 2, 0) << 8.
    w.store.get_mut(m).unwrap().current = 2;
    w.fake.mb.frames.insert(m, 1 << 8);
    srv_do(&mut w, 5, m);
    assert_eq!(w.fake.mb.frames[&m], 0);
}

#[test]
fn srvdo_7_homing() {
    let mut r = row();
    r.param1 = 0;
    let (mut w, m) = world(r, None);
    // Not homing: no rebuild.
    w.fake.mb.target = Some(w.monster);
    srv_do(&mut w, 7, m);
    assert!(!w.fake.calls.contains(&"build".to_string()));
    // Homing, frames left a multiple of 5 (Param1 0 → 5), distance 4…24.
    w.store.get_mut(m).unwrap().target.0 = 1;
    w.store.get_mut(m).unwrap().current = 10;
    w.fake.pos.insert(m, (100, 100));
    w.fake.pos.insert(w.monster, (110, 100));
    // Sizes 1: no reduction; (2 × 10 + 0) / 2.
    assert_eq!(unit_distance(&w.fake, m, w.monster), 10);
    w.fake.calls.clear();
    srv_do(&mut w, 7, m);
    assert_eq!(w.fake.calls.iter().filter(|c| *c == "build").count(), 1);
    // Out of range (distance 30): no rebuild.
    w.fake.pos.insert(w.monster, (131, 100));
    w.fake.calls.clear();
    srv_do(&mut w, 7, m);
    assert!(!w.fake.calls.contains(&"build".to_string()));
    // Owner dead → removed.
    w.fake.mb.dead.insert(w.owner);
    assert_eq!(srv_do(&mut w, 7, m), 2);
}

#[test]
fn srvdo_8_interval_and_range() {
    let mut r = row();
    (r.param1, r.param2, r.param3, r.submissile1) = (3, 4, 4, 0);
    let (mut w, m) = world(r, None);
    // q = 10 / 4 = 2; interval max(4 − 2, 3) = 3: elapsed 1 creates
    // nothing, elapsed 3 re-seeds and creates.
    let d = w.store.get_mut(m).unwrap();
    d.current = d.total - 1;
    srv_do(&mut w, 8, m);
    assert_eq!(w.fake.logged("alloc"), 0);
    let d = w.store.get_mut(m).unwrap();
    d.current = d.total - 3;
    let x = w.fake.pos[&m].0;
    srv_do(&mut w, 8, m);
    assert_eq!(w.fake.logged("alloc 0"), 1);
    // The helper re-seeded {x + 3, 666} and drew two rolls of 2(5 − 1).
    let mut s = Seed::init_low((x + 3) as u32);
    s.roll(8);
    s.roll(8);
    assert_eq!(w.fake.seeds[&m], s);
}

#[test]
fn srvdo_10_and_25_need_the_skill() {
    let mut r = row();
    r.submissile1 = 0;
    let (mut w, m) = world(r.clone(), None);
    assert_eq!(srv_do(&mut w, 10, m), 2);
    // With the skill: calc1 then calc2.
    w.fake.mb.skills.insert(7, [2, 1, 0, 0, 0]);
    srv_do(&mut w, 10, m);
    let order: Vec<_> = w
        .fake
        .log
        .iter()
        .filter(|l| l.starts_with("skillcalc"))
        .cloned()
        .collect();
    assert_eq!(order, ["skillcalc Calc1", "skillcalc Calc2"]);
    // 25 with SubMissile1 = 0 → removed, before the skill is read.
    w.fake.log.clear();
    assert_eq!(srv_do(&mut w, 25, m), 2);
    assert_eq!(w.fake.logged("skillcalc"), 0);
}

#[test]
fn elem_roll_and_len_by_etype() {
    let mut r = row();
    r.etype = 0;
    let (mut w, m) = world(r, None);
    for (s, v) in [(21, 10), (22, 10), (25, 50), (141, 1)] {
        w.fake.stats.insert((m, s), v);
    }
    let mut rec = DamageRecord::default();
    let mon = w.monster;
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    // 10 + 50 % = 15, deadly strike × 2 and 0x2000; the return is the
    // final physical (the server-hit return value), not the `EType`.
    assert_eq!(elem_roll(&mut cx, m, Some(mon), &mut rec), 30);
    assert_eq!((rec.physical, rec.result), (30, 0x2000));
    let mut rec = DamageRecord::default();
    elem_len(&mut rec, 4, 25);
    elem_len(&mut rec, 3, 99);
    assert_eq!((rec.cold_len, rec.magic), (25, 0));
}

#[test]
fn srvhit_1_area_fire() {
    let mut r = row();
    (r.etype, r.resultflags) = (1, 1);
    let (mut w, m) = world(r, None);
    w.fake.stats.insert((m, 48), 7);
    w.fake.stats.insert((m, 49), 7);
    // sHitPar1 0 and no skill: no explosion.
    assert_eq!(srv_hit(&mut w, 1, m, None), 1);
    assert_eq!(w.fake.logged("scan"), 0);
    // With the skill: r = max(calc1 0, 1) = 1; f 0 → 0x8583.
    w.fake.mb.skills.insert(7, [0, 0, 0, 0, 0]);
    w.fake.mb.area = vec![w.monster];
    assert_eq!(srv_hit(&mut w, 1, m, None), 1);
    let (x, y) = w.fake.pos[&m];
    assert!(w.fake.log.contains(&format!("scan ({x}, {y}) 1 0x8583")));
    assert!(w
        .fake
        .log
        .contains(&format!("areahit {} 7 0 0x1", w.monster.0)));
}

#[test]
fn srvhit_4_sub_missiles_hit_at_once() {
    let mut r = row();
    (r.hitsubmissile1, r.hitsubmissile2, r.shitpar1) = (1, 0, 1);
    let (mut w, m) = world(r, Some(row()));
    let mon = w.monster;
    assert_eq!(srv_hit(&mut w, 4, m, Some(mon)), 3);
    assert_eq!(w.fake.logged("alloc 1"), 1);
    // The new missile's hit handler ran on the unit (event 0).
    assert!(w.fake.logged("event0") >= 1);
}

#[test]
fn srvhit_12_chains_in_guid_order() {
    let mut r = row();
    r.shitpar1 = 10;
    let (mut w, m) = world(r, None);
    let room = w.fake.room;
    let a = w.game.spawn_unit(UnitType::Monster, room, false).unwrap();
    w.fake.pos.insert(a, (120, 100));
    // One bounce left: nothing.
    w.store.get_mut(m).unwrap().target.0 = 1;
    let mon = w.monster;
    assert_eq!(srv_hit(&mut w, 12, m, Some(mon)), 3);
    assert_eq!(w.fake.logged("scan"), 0);
    // Three: the next GUID after the hit unit gets a copy with two.
    w.store.get_mut(m).unwrap().target.0 = 3;
    w.fake.mb.area = vec![w.monster, a];
    let before: Vec<UnitId> = w.store.missiles().collect();
    let mon = w.monster;
    assert_eq!(srv_hit(&mut w, 12, m, Some(mon)), 3);
    let new: Vec<UnitId> = w.store.missiles().filter(|x| !before.contains(x)).collect();
    assert_eq!(new.len(), 1);
    assert_eq!(w.store.get(new[0]).unwrap().target.0, 2);
    assert!(w.fake.log.iter().any(|l| l.ends_with("0x8a783")));
    // next_unit wraps to the smallest GUID when none is above.
    let ga = w.game.lists.unit(a).unwrap().guid;
    let gm = w.game.lists.unit(w.monster).unwrap().guid;
    let owner = w.owner;
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    let top = ga.max(gm);
    let low = if ga < gm { a } else { w.monster };
    assert_eq!(
        next_unit(&mut w.game, &mut cx, owner, (0, 0), 5, 0, top),
        Some(low)
    );
}

#[test]
fn srvhit_13_length_and_result_one() {
    let mut r = row();
    (r.etype, r.shitpar1, r.shitpar2) = (4, 3, 30);
    let (mut w, m) = world(r, None);
    w.fake.stats.insert((m, 54), 4);
    w.fake.stats.insert((m, 55), 4);
    w.fake.mb.skills.insert(7, [0, 0, 0, 0, 0]);
    w.fake.mb.area = vec![w.monster];
    assert_eq!(srv_hit(&mut w, 13, m, None), 1);
    assert!(w
        .fake
        .log
        .contains(&format!("areahit {} 0 30 0x0", w.monster.0)));
    // No skill record: 1, nothing.
    w.fake.mb.skills.clear();
    w.fake.log.clear();
    assert_eq!(srv_hit(&mut w, 13, m, None), 1);
    assert_eq!(w.fake.logged("scan"), 0);
}
