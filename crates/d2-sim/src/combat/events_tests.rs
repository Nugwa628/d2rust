//! Tests for the unit event functions and the item cast, run on the body fake
//! (`crate::skills::use_::bodies::fake::BodyFake`). Each test pins the result of one
//! event function, or one part of the item cast, for hand-computed inputs.

use super::events::*;
use super::{CombatTables, DamageRecord};
use crate::skills::fake::{combat_tables, monster_rec, FUnit};
use crate::skills::use_::bodies::fake::BodyFake;
use crate::skills::use_::bodies::tests2::{body_rec, tabs, Code};
use crate::skills::use_::bodies::{BodyWorld, Handler};
use crate::skills::SkillTables;
use crate::units::UnitType;

impl EventWorld for BodyFake {
    fn layer_split(&self) -> (u32, u32) {
        (6, 0x3F)
    }
    fn list_owner(&self, l: usize) -> Option<usize> {
        self.lists[l].owner
    }
    fn guid(&self, u: usize) -> u32 {
        self.c.units[u].guid
    }
    fn terror(&mut self, s: usize, u: usize, skill: i32, a: i32, b: i32) {
        self.c.log.push(format!("terror {s} {u} {skill} {a} {b}"));
    }
    fn point_free(&self, _: usize, at: (i32, i32)) -> bool {
        !self.collides || at == self.path_target
    }
    fn corpse_near(&mut self, _: usize) -> Option<usize> {
        self.found.first().copied()
    }
    fn queue_item_cast(&mut self, u: usize, msg: ItemCastMsg) {
        self.c.log.push(format!("cast {u} {msg:?}"));
    }
    fn raise_test(&self, _: usize) -> bool {
        true
    }
    fn clear_pattern(&mut self, v: usize) {
        self.c.log.push(format!("clearpattern {v}"));
    }
    fn raise_step(&mut self, n: usize, step: RaiseStep<usize>) {
        self.c.log.push(format!("raise {n} {step:?}"));
    }
    fn handlers_of(&self, u: usize) -> Vec<Handler> {
        self.handlers.get(&u).cloned().unwrap_or_default()
    }
    fn remove_handler(&mut self, u: usize, h: &Handler) {
        if let Some(v) = self.handlers.get_mut(&u) {
            if let Some(i) = v.iter().position(|x| x == h) {
                v.remove(i);
            }
        }
    }
}

fn tb<'a>(t: &'a SkillTables, ct: &'a CombatTables) -> EventTables<'a> {
    EventTables {
        skills: t,
        combat: ct,
    }
}

/// Attackable monsters (monstats2 `isatt`, for `apply_state`).
fn monsters(n: usize) -> CombatTables {
    let mut ct = combat_tables(vec![monster_rec(); n]);
    for m in &mut ct.monstats2 {
        m.isatt = true;
    }
    ct
}

fn plain() -> (SkillTables, CombatTables) {
    (tabs(body_rec(), Code::new(), 1), monsters(300))
}

/// k for an item event: stat << 16 | layer.
fn k(stat: u16, layer: u16) -> i32 {
    (i32::from(stat) << 16) | i32::from(layer)
}

fn unit(f: &mut BodyFake, ty: UnitType, stats: &[(u16, u16, i32)]) -> usize {
    let mut u = FUnit::new(ty, 0);
    for &(s, l, v) in stats {
        u.stats.insert((s, l), v);
    }
    f.add(u, (0, 0))
}

fn ev<U>(event: i32, h: Option<U>, o: Option<U>, k: i32, l: i32) -> EventCall<U> {
    EventCall { event, h, o, k, l }
}

#[test]
fn attacker_takes_records() {
    // Fn 12, H level 30, O level 20, v 10.
    let r = attacker_takes_record(12, 10, 30, 20);
    assert_eq!((r.cold, r.cold_len, r.hit_class), (2560, 125, 0x3D));
    assert_eq!(r.result, 0x4021);
    // H not higher: 25.
    assert_eq!(attacker_takes_record(12, 10, 20, 20).cold_len, 25);
    assert_eq!(
        (
            attacker_takes_record(6, 3, 1, 1).physical,
            attacker_takes_record(6, 3, 1, 1).hit_class
        ),
        (768, 0x8D)
    );
    assert_eq!(attacker_takes_record(10, 3, 1, 1).lightning, 768);
    assert_eq!(attacker_takes_record(11, 3, 1, 1).hit_class, 0x2D);
    // O needs unit flag 0x4 and v > 0.
    let (t, ct) = plain();
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[(78, 0, 5)]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            6,
            ev(2, Some(h), Some(o), k(78, 0), 0),
            None
        ),
        0
    );
    f.c.units[o].flags |= 4;
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            6,
            ev(2, Some(h), Some(o), k(78, 0), 0),
            None
        ),
        1
    );
    assert!(f
        .c
        .log
        .iter()
        .any(|l| l.starts_with(&format!("reaction {h} {o}"))));
}

#[test]
fn freeze_vectors() {
    // Fn 14: v 5, H 20, O 25, event 5: c = 105 → 100; r = 37 → 151.
    assert_eq!(freeze_chance(5, 20, 25, false), 100);
    assert_eq!(freeze_length(100, 37), 151);
    // Event 6: 5 × (16 − 25 + 14 + 10) / 3 = 25; r = 30 → nothing.
    assert_eq!(freeze_chance(5, 20, 25, true), 25);
    // Clamps.
    assert_eq!(freeze_chance(0, 1, 99, false), 0);
    assert_eq!(freeze_length(100, 0), 225);
    assert_eq!(freeze_length(99, 99), 25);
}

#[test]
fn stupidity_vectors() {
    // Fn 9: H 30, O 40, v 2, event 5: c = 20; r = 4 → d = 4.
    assert_eq!(stupidity_chance(30, 40, 2, false), 20);
    assert_eq!(stupidity_level(20, 4), 4);
    // Event 6 divides a positive c; the clamp is 1…99, d 1…20.
    assert_eq!(stupidity_chance(30, 40, 2, true), 6);
    assert_eq!(stupidity_chance(1, 99, 0, false), 1);
    assert_eq!(stupidity_chance(99, 1, 10, false), 99);
    assert_eq!(stupidity_level(99, 0), 20);
}

#[test]
fn golem_drain_percent() {
    // Fn 23 f: Param1 0, Param2 100, L 10 → (10 × 110) / 16 = 68.
    let mut r = body_rec();
    r.param1 = 0;
    r.param2 = 100;
    let t = tabs(r, Code::new(), 1);
    assert_eq!(golem_drain_pct(&t, 10, 1), 68);
    assert_eq!(golem_drain_pct(&t, 0, 1), 0);
    assert_eq!(golem_drain_pct(&t, 10, 99), 0);
}

#[test]
fn energy_shield_row_vector() {
    // Fn 24: w = 2560, p 50, m 5120, q 16 → x = 1280, m = 3840.
    assert_eq!(shield_row(2560, 50, 5120, 16), (1280, 3840));
    // Mana-bound: pct(m, 16, q) caps x; m never goes below 0.
    assert_eq!(shield_row(10_000, 100, 64, 32), (32, 0));
}

#[test]
fn knockback_draw_against_size() {
    let (t, mut ct) = plain();
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[(81, 0, 1)]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    // One `mask(128)` draw on H's seed; < 0x40 for a medium monster.
    let mut s = f.c.units[h].seed;
    let want = (s.mask(128) as i32) < 0x40;
    let mut r = DamageRecord::default();
    let got = call(
        &mut f,
        tb(&t, &ct),
        7,
        ev(5, Some(h), Some(o), k(81, 0), 0),
        Some(&mut r),
    );
    assert_eq!(got == 1, want);
    assert_eq!(r.result & 8 != 0, want);
    assert_eq!(f.c.units[h].seed, s);
    // A large monster: t = 0x20; no record → 0 without a draw.
    ct.monstats2[0].large = true;
    let before = f.c.units[h].seed;
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            7,
            ev(5, Some(h), Some(o), k(81, 0), 0),
            None
        ),
        0
    );
    assert_eq!(f.c.units[h].seed, before);
}

#[test]
fn mana_and_life_after_events() {
    let (t, ct) = plain();
    let mut f = BodyFake::new();
    // Mana 8 at 100, max (stat 9 in the fake) 1000; v 50 % of total 400.
    let h = unit(
        &mut f,
        UnitType::Player,
        &[
            (8, 0, 100),
            (9, 0, 1000),
            (114, 0, 50),
            (6, 0, 500),
            (7, 0, 600),
            (138, 0, 1),
        ],
    );
    let o = unit(&mut f, UnitType::Monster, &[]);
    let mut r = DamageRecord {
        total: 400,
        ..DamageRecord::default()
    };
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            13,
            ev(1, Some(h), Some(o), k(114, 0), 0),
            Some(&mut r)
        ),
        1
    );
    assert_eq!(f.c.units[h].stats[&(8, 0)], 300);
    assert!(f.c.log.contains(&format!("overlay {h} 152")));
    // 17: + (v << 8), capped at max mana.
    call(
        &mut f,
        tb(&t, &ct),
        17,
        ev(9, Some(h), Some(o), k(138, 0), 0),
        None,
    );
    assert_eq!(f.c.units[h].stats[&(8, 0)], 556);
    // 18 needs a demon; 28 does not.
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            18,
            ev(9, Some(h), Some(o), k(138, 0), 0),
            None
        ),
        0
    );
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            28,
            ev(9, Some(h), Some(o), k(138, 0), 0),
            None
        ),
        1
    );
    assert_eq!(f.c.units[h].stats[&(6, 0)], 600);
    // At the maximum: 0.
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            28,
            ev(9, Some(h), Some(o), k(138, 0), 0),
            None
        ),
        0
    );
}

#[test]
fn slow_caps_and_state_list() {
    let (t, ct) = plain();
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[(150, 0, 95)]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            19,
            ev(5, Some(h), Some(o), k(150, 0), 0),
            None
        ),
        1
    );
    let l = f.list_of(o, 24).expect("slowed list");
    assert_eq!(
        (l.stats[&67], l.stats[&68], l.stats[&69], l.expire),
        (-90, -90, -90, 750)
    );
    assert!(f.c.log.contains(&format!("anim {o}")));
    // A player target: 50; Clay Golem reads stat 150 itself.
    let p = unit(&mut f, UnitType::Player, &[]);
    call(&mut f, tb(&t, &ct), 27, ev(1, Some(h), Some(p), 0, 0), None);
    assert_eq!(f.list_of(p, 24).unwrap().stats[&67], -50);
    // A boss: 50 for 19, 90 for 27.
    let b = unit(&mut f, UnitType::Monster, &[]);
    f.c.units[b].boss = true;
    call(&mut f, tb(&t, &ct), 27, ev(1, Some(h), Some(b), 0, 0), None);
    assert_eq!(f.list_of(b, 24).unwrap().stats[&67], -90);
    // Other unit types: 0.
    let m = unit(&mut f, UnitType::Object, &[]);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            19,
            ev(5, Some(h), Some(m), k(150, 0), 0),
            None
        ),
        0
    );
}

#[test]
fn bone_and_cyclone_armor_absorb() {
    let mut r = body_rec();
    r.aurastate = 10;
    r.aurastat1 = 100;
    r.aurastat2 = 101;
    let t = tabs(r, Code::new(), 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[]);
    let l = f.alloc_list(0, 0, None).unwrap();
    f.set_list_state(l, 10);
    f.attach(h, l);
    f.list_set(l, 100, 300);
    f.list_set(l, 101, 1000);
    f.list_set(l, 84, 30);
    // Bone Armor: 200 physical absorbed; q = 10, |10 − 30| ≥ 5 → marked.
    let mut rec = DamageRecord {
        physical: 200,
        ..DamageRecord::default()
    };
    call(
        &mut f,
        tb(&t, &ct),
        22,
        ev(4, Some(h), None, 1, 1),
        Some(&mut rec),
    );
    assert_eq!(
        (rec.physical, f.list_get(l, 100), f.list_get(l, 84)),
        (0, 100, 10)
    );
    assert!(f.c.log.contains(&format!("changed {h} 10")));
    // Cyclone Armor: fire, then cold, then lightning.
    f.c.log.clear();
    let mut rec = DamageRecord {
        fire: 30,
        cold: 40,
        lightning: 50,
        ..DamageRecord::default()
    };
    call(
        &mut f,
        tb(&t, &ct),
        25,
        ev(4, Some(h), None, 1, 1),
        Some(&mut rec),
    );
    // a = 100: fire 30, cold 40, then 30 of the lightning's 50.
    assert_eq!((rec.fire, rec.cold, rec.lightning), (0, 0, 20));
    assert_eq!(f.list_get(l, 100), 0);
    // a ≤ 0: the state goes off and the list is freed.
    assert!(f.c.log.contains(&format!("free {h} {l}")));
}

#[test]
fn reanimate_registers_a_run_once_raise() {
    let (t, ct) = plain();
    let mut f = BodyFake::new();
    let p = unit(&mut f, UnitType::Player, &[]);
    let h = unit(&mut f, UnitType::Monster, &[]);
    f.minion_owner.insert(h, p);
    let o = unit(&mut f, UnitType::Monster, &[]);
    // v = 100 (always), class layer 7.
    f.c.units[h].stats.insert((175, 7), 100);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            31,
            ev(9, Some(h), Some(o), k(175, 7), 0),
            None
        ),
        1
    );
    let hs = f.handlers_of(o);
    assert_eq!(
        hs,
        [Handler {
            event: 13,
            key_type: 0,
            key: p as i32,
            skill: 7,
            level: p as i32,
            func: RAISE_FUNC,
        }]
    );
    // Champions and uniques (type flags 4, 8) are skipped.
    let u = unit(&mut f, UnitType::Monster, &[]);
    f.c.units[u].flags = 8;
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            31,
            ev(9, Some(h), Some(u), k(175, 0), 0),
            None
        ),
        0
    );
    // The iteration runs the key-type-0 raise once and drops it.
    let ct2 = monsters(8);
    let before = f.handlers_of(o).len();
    assert_eq!(before, 1);
    run(&mut f, tb(&t, &ct2), 13, Some(o), None, None);
    assert!(f.handlers_of(o).is_empty());
    assert!(f.c.log.iter().any(|l| l == &format!("clearpattern {o}")));
    assert!(f
        .c
        .log
        .iter()
        .any(|l| l.starts_with("raise ") && l.ends_with("Umod21")));
}

#[test]
fn item_cast_core_needs_item_effect_and_restores_the_target() {
    let mut r = body_rec();
    r.itemeffect = 0;
    let t = tabs(r, Code::new(), 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    let u = unit(&mut f, UnitType::Player, &[]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    // `ItemEffect` 0: nothing, no message.
    assert_eq!(cast(&mut f, tb(&t, &ct), Some(u), 1, 1, Some(o), false), 0);
    assert!(!f.c.log.iter().any(|l| l.starts_with("cast ")));
    // No target unit or a caster with state 54: 0 before the core.
    assert_eq!(cast(&mut f, tb(&t, &ct), Some(u), 1, 1, None, false), 0);
    f.c.units[u].states.push(54);
    assert_eq!(
        cast_point(&mut f, tb(&t, &ct), Some(u), 1, 1, (5, 5), false),
        0
    );
    // Kind 1 (self): the core targets U, then restores the saved point.
    let mut r = body_rec();
    r.itemeffect = 1;
    r.itemtarget = 1;
    let t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    let u = unit(&mut f, UnitType::Player, &[]);
    let (_, out) = core(&mut f, tb(&t, &ct), u, 1, 1, None, (5, 6), false);
    assert_eq!(out.unit, (0, f.c.units[u].guid));
    let log = f.take_log();
    let paths: Vec<_> = log.iter().filter(|l| l.starts_with("path ")).collect();
    assert_eq!(paths[0], &format!("path {u} TargetPoint(5, 6)"));
    assert_eq!(paths[1], &format!("path {u} TargetUnit(Some({u}))"));
    assert_eq!(
        paths.last().unwrap(),
        &&format!("path {u} TargetPoint(0, 0)")
    );
}

#[test]
fn iteration_continues_with_the_live_records_ahead() {
    let h = |key: i32| Handler {
        event: 1,
        key_type: 1,
        key,
        skill: 0,
        level: 0,
        func: 1,
    };
    let (a, b, c, d, n) = (h(1), h(2), h(3), h(4), h(9));
    // Nothing changed: the whole tail is still ahead.
    assert_eq!(surviving_tail(&[a, b, c, d], &[c, d]), 2);
    // A record ahead unlinked by the call is not reached.
    assert_eq!(surviving_tail(&[a, b, d], &[c, d]), 1);
    // A record prepended by the call lies behind the walk.
    assert_eq!(surviving_tail(&[n, a, b, c, d], &[c, d]), 2);
    // The current record unregistered during its own call, the rest kept.
    assert_eq!(surviving_tail(&[a, c, d], &[c, d]), 2);
    // Every record ahead gone.
    assert_eq!(surviving_tail(&[n, a, b], &[c, d]), 0);
    assert_eq!(surviving_tail(&[], &[c, d]), 0);
}

#[test]
fn chilling_armor_return_fire() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let mut t = tabs(r, Code::new(), 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    f.c.hostile = true;
    let h = unit(&mut f, UnitType::Player, &[]);
    let p = unit(&mut f, UnitType::Monster, &[]);
    f.pos.insert(p, (40, 50));
    let m = unit(&mut f, UnitType::Missile, &[]);
    f.missile_owners.insert(m, p);
    let go = |f: &mut BodyFake, t: &SkillTables| {
        call(f, tb(t, &ct), 1, ev(0, Some(h), Some(m), 1, 3), None)
    };
    // The missile row lacks ReturnFire: nothing.
    assert_eq!(go(&mut f, &t), 0);
    assert!(f.missiles.is_empty());
    // With ReturnFire: flags 0x20, owner and origin H, target P's
    // position, skill k and level L.
    t.missiles[0].returnfire = true;
    assert_eq!(go(&mut f, &t), 1);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.owner, q.origin), (0x20, h, Some(h)));
    assert_eq!((q.class, q.target_x, q.target_y), (0, 40, 50));
    assert_eq!((q.skill, q.level), (1, 3));
    // H may not attack P: nothing more.
    f.c.hostile = false;
    assert_eq!(go(&mut f, &t), 0);
    assert_eq!(f.missiles.len(), 1);
    // Level < 0 reads as 1.
    f.c.hostile = true;
    call(&mut f, tb(&t, &ct), 1, ev(0, Some(h), Some(m), 1, -5), None);
    assert_eq!(f.missiles[1].level, 1);
}

#[test]
fn frozen_armor_freezes_the_attacker() {
    let mut r = body_rec();
    r.cltoverlaya = 7;
    let mut code = Code::new();
    r.calc1 = code.f(60);
    let t = tabs(r, code, 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    f.c.hostile = true;
    let h = unit(
        &mut f,
        UnitType::Player,
        &[(6, 0, 100_000), (7, 0, 100_000)],
    );
    let o = unit(
        &mut f,
        UnitType::Player,
        &[(6, 0, 100_000), (7, 0, 100_000)],
    );
    let obj = unit(&mut f, UnitType::Object, &[]);
    let run = |f: &mut BodyFake, o, phys| {
        let mut rec = DamageRecord {
            physical: phys,
            ..DamageRecord::default()
        };
        call(
            f,
            tb(&t, &ct),
            2,
            ev(1, Some(h), Some(o), 1, 1),
            Some(&mut rec),
        )
    };
    // R physical must be > 0; O a player or monster.
    assert_eq!(run(&mut f, o, 0), 0);
    assert_eq!(run(&mut f, obj, 5), 0);
    assert!(f.c.log.is_empty());
    // Freeze length := eval(calc1) = 60 (a player gets the cold rule);
    // overlay 7 on O.
    assert_eq!(run(&mut f, o, 5), 1);
    assert!(f.c.log.contains(&format!("overlay {o} 7")), "{:?}", f.c.log);
    // No record at all also acts.
    f.c.log.clear();
    assert_eq!(
        call(&mut f, tb(&t, &ct), 2, ev(1, Some(h), Some(o), 1, 1), None),
        1
    );
}

#[test]
fn shiver_armor_elemental_hit() {
    let mut r = body_rec();
    r.cltoverlaya = 9;
    let t = tabs(r, Code::new(), 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    f.c.hostile = true;
    let h = unit(&mut f, UnitType::Player, &[]);
    let o = unit(&mut f, UnitType::Monster, &[(6, 0, 100_000)]);
    let obj = unit(&mut f, UnitType::Object, &[]);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            3,
            ev(3, Some(h), Some(obj), 1, 1),
            None
        ),
        0
    );
    assert_eq!(
        call(&mut f, tb(&t, &ct), 3, ev(3, None, Some(o), 1, 1), None),
        0
    );
    assert_eq!(
        call(&mut f, tb(&t, &ct), 3, ev(3, Some(h), Some(o), 1, 1), None),
        1
    );
    // Reaction on O after the apply (attacker H), then overlay 9.
    let log = &f.c.log;
    let re = log
        .iter()
        .position(|l| l.starts_with(&format!("reaction {h} {o}")));
    let ov = log.iter().position(|l| *l == format!("overlay {o} 9"));
    assert!(re.unwrap() < ov.unwrap(), "{log:?}");
}

#[test]
fn iron_maiden_reflects_physical() {
    let mut r = body_rec();
    let mut code = Code::new();
    r.calc1 = code.f(50);
    r.calc2 = code.f(30);
    r.calc3 = code.f(10);
    r.auratargetstate = 12;
    r.resultflags = 0;
    let t = tabs(r, code, 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    f.c.hostile = true;
    // H: a player carrying the state; its list's owner C is a player.
    let h = unit(
        &mut f,
        UnitType::Player,
        &[(6, 0, 100_000), (7, 0, 100_000)],
    );
    let o = unit(
        &mut f,
        UnitType::Monster,
        &[(6, 0, 100_000), (7, 0, 100_000)],
    );
    let rec0 = DamageRecord {
        physical: 1_000,
        ..DamageRecord::default()
    };
    let run = |f: &mut BodyFake| {
        let mut rec = rec0;
        call(
            f,
            tb(&t, &ct),
            4,
            ev(5, Some(h), Some(o), 1, 1),
            Some(&mut rec),
        )
    };
    // No state on H: nothing.
    assert_eq!(run(&mut f), 0);
    let l = f.alloc_list(0, 0, Some(h)).unwrap();
    f.set_list_state(l, 12);
    f.attach(h, l);
    f.c.units[h].states.push(12);
    // calc1 (H a plain player): p = 50 % of 1000 = 500, taken by H with
    // O as the attacker.
    assert_eq!(run(&mut f), 1);
    assert_eq!(f.c.get(h, 6), 100_000 - 500);
    assert_eq!(f.c.get(o, 6), 100_000);
    // R physical must be > 0.
    let mut zero = DamageRecord::default();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            4,
            ev(5, Some(h), Some(o), 1, 1),
            Some(&mut zero)
        ),
        0
    );
}

#[test]
fn life_tap_heals_the_victim() {
    let mut r = body_rec();
    let mut code = Code::new();
    r.calc1 = code.f(50);
    r.auratargetstate = 12;
    r.prgoverlay = 4;
    let t = tabs(r, code, 1);
    let ct = monsters(1);
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Monster, &[]);
    let o = unit(&mut f, UnitType::Player, &[(6, 0, 300), (7, 0, 1_000)]);
    let l = f.alloc_list(0, 0, Some(o)).unwrap();
    f.set_list_state(l, 12);
    f.attach(h, l);
    f.c.units[h].states.push(12);
    let mut rec = DamageRecord {
        physical: 1_000,
        ..DamageRecord::default()
    };
    // x = 50 % of 1000 = 500: life 300 + 500 = 800 (of 1000).
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            5,
            ev(1, Some(h), Some(o), 1, 1),
            Some(&mut rec)
        ),
        1
    );
    assert_eq!(f.c.get(o, 6), 800);
    assert!(f.c.log.contains(&format!("overlay {o} 4")));
    // Clamped to the maximum.
    call(
        &mut f,
        tb(&t, &ct),
        5,
        ev(1, Some(h), Some(o), 1, 1),
        Some(&mut rec),
    );
    assert_eq!(f.c.get(o, 6), 1_000);
    // R physical 0, or a dead O: 0.
    let mut zero = DamageRecord::default();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            5,
            ev(1, Some(h), Some(o), 1, 1),
            Some(&mut zero)
        ),
        0
    );
    f.dead.insert(o);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            5,
            ev(1, Some(h), Some(o), 1, 1),
            Some(&mut rec)
        ),
        0
    );
}

#[test]
fn howl_terrifies_plain_monsters() {
    let (t, ct) = plain();
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[(112, 0, 128)]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    // v = 128 > any mask(128) draw: always terror (skill 130, 20, 20).
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            8,
            ev(5, Some(h), Some(o), k(112, 0), 0),
            None
        ),
        1
    );
    assert!(
        f.c.log.contains(&format!("terror {h} {o} 130 20 20")),
        "{:?}",
        f.c.log
    );
    // v = 0: nothing; a champion / unique (flag 4 or 8): nothing.
    f.c.log.clear();
    let h0 = unit(&mut f, UnitType::Player, &[(112, 0, 0)]);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            8,
            ev(5, Some(h0), Some(o), k(112, 0), 0),
            None
        ),
        0
    );
    f.c.units[o].flags |= 4;
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            8,
            ev(5, Some(h), Some(o), k(112, 0), 0),
            None
        ),
        0
    );
    assert!(f.c.log.is_empty());
}

fn item_cast_tables(check_start: bool, target: u8) -> SkillTables {
    let mut r = body_rec();
    r.itemeffect = 1;
    r.itemtarget = target;
    r.itemcheckstart = check_start;
    r.srvdofunc = 1;
    r.intown = false;
    tabs(r, Code::new(), 1)
}

#[test]
fn item_cast_core_start_and_do() {
    let ct = monsters(1);
    // The do core runs with the item flag; its result is the core's. Unit flag 0x40 is
    // saved and restored around it.
    for saved in [0u32, 0x40] {
        let t = item_cast_tables(false, 1);
        let mut f = BodyFake::new();
        f.srvdo_result = 1;
        let u = unit(&mut f, UnitType::Player, &[]);
        f.c.units[u].flags |= saved;
        let (res, out) = core(&mut f, tb(&t, &ct), u, 1, 2, None, (5, 6), true);
        assert_eq!(res, 1);
        assert_eq!(out.unit, (0, f.c.units[u].guid));
        assert!(
            f.c.log.contains(&format!("srvdo 1 {u} 1 2")),
            "{:?}",
            f.c.log
        );
        assert_eq!(f.c.units[u].flags & 0x40, saved);
    }
    // `ItemCheckStart`: the start core runs first; failing it, the result is 0 and the do
    // core does not run.
    let t = item_cast_tables(true, 1);
    let mut f = BodyFake::new();
    f.srvdo_result = 1;
    // Not `InTown`, in a town room: the start core fails.
    f.town.insert(1);
    let u = unit(&mut f, UnitType::Player, &[]);
    let (res, _) = core(&mut f, tb(&t, &ct), u, 1, 1, None, (5, 6), false);
    assert_eq!(res, 0);
    assert!(!f.c.log.iter().any(|l| l.starts_with("srvdo")));
}

#[test]
fn item_cast_queues_the_client_messages() {
    let ct = monsters(1);
    // cast: kind 1 chooses U itself; the 0x99 entry names that unit.
    let t = item_cast_tables(false, 1);
    let mut f = BodyFake::new();
    f.srvdo_result = 1;
    let u = unit(&mut f, UnitType::Player, &[]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    assert_eq!(cast(&mut f, tb(&t, &ct), Some(u), 1, 4, Some(o), true), 1);
    let want = format!(
        "cast {u} {:?}",
        ItemCastMsg::Unit {
            skill: 1,
            level: 4,
            target: (0, f.c.units[u].guid),
            aim: true
        }
    );
    assert!(f.c.log.contains(&want), "{:?}", f.c.log);
    // Kind 0 chooses nothing: the entry names T.
    let t = item_cast_tables(false, 0);
    f.c.log.clear();
    assert_eq!(cast(&mut f, tb(&t, &ct), Some(u), 1, 4, Some(o), false), 1);
    let want = format!(
        "cast {u} {:?}",
        ItemCastMsg::Unit {
            skill: 1,
            level: 4,
            target: (1, f.c.units[o].guid),
            aim: false
        }
    );
    assert!(f.c.log.contains(&want), "{:?}", f.c.log);
    // cast_point: the core's point when its x is non-zero, else (x, y).
    f.c.log.clear();
    assert_eq!(
        cast_point(&mut f, tb(&t, &ct), Some(u), 1, 4, (7, 8), false),
        1
    );
    let want = format!(
        "cast {u} {:?}",
        ItemCastMsg::Point {
            skill: 1,
            level: 4,
            at: (7, 8),
            aim: false
        }
    );
    assert!(f.c.log.contains(&want), "{:?}", f.c.log);
    // A failed core queues nothing.
    f.c.log.clear();
    f.srvdo_result = 0;
    assert_eq!(cast(&mut f, tb(&t, &ct), Some(u), 1, 4, Some(o), false), 0);
    assert!(!f.c.log.iter().any(|l| l.starts_with("cast ")));
}

#[test]
fn skill_on_attack_kill_hit() {
    let ct = monsters(1);
    let mut r = body_rec();
    r.itemeffect = 1;
    r.itemtarget = 1;
    r.srvdofunc = 1;
    r.intown = false;
    let mut t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    f.srvdo_result = 1;
    // Layer = skill 1 << 6 | level 3 (split (6, 0x3F)); v = 100 always.
    let lay = (1 << 6) | 3;
    let h = unit(&mut f, UnitType::Player, &[(195, lay, 100)]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    let kk = k(195, lay);
    let casts = |f: &BodyFake| f.c.log.iter().filter(|l| l.starts_with("cast ")).count();
    // 20 needs hit flag 0x20 on a present record.
    let mut r0 = DamageRecord::default();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            20,
            ev(7, Some(h), Some(o), kk, 0),
            Some(&mut r0)
        ),
        0
    );
    r0.hit_flags = 0x20;
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            20,
            ev(7, Some(h), Some(o), kk, 0),
            Some(&mut r0)
        ),
        1
    );
    assert_eq!(casts(&f), 1);
    assert!(f
        .c
        .log
        .iter()
        .any(|l| l.starts_with(&format!("cast {h} Unit"))));
    assert!(f.c.log.iter().any(|l| l.contains("aim: true")));
    // 30: no aim; without O a point cast at H's path target.
    f.c.log.clear();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            30,
            ev(10, Some(h), Some(o), kk, 0),
            None
        ),
        1
    );
    assert!(f.c.log.iter().any(|l| l.contains("aim: false")));
    f.c.log.clear();
    assert_eq!(
        call(&mut f, tb(&t, &ct), 30, ev(12, Some(h), None, kk, 0), None),
        1
    );
    assert!(f
        .c
        .log
        .iter()
        .any(|l| l.starts_with(&format!("cast {h} Point"))));
    // v = 0: nothing.
    let h0 = unit(&mut f, UnitType::Player, &[(195, lay, 0)]);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            30,
            ev(10, Some(h0), Some(o), kk, 0),
            None
        ),
        0
    );
    // `ItemTgtDo` on function 20 makes O cast on itself.
    t.skills[1].itemtgtdo = true;
    f.c.log.clear();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            20,
            ev(7, Some(h), Some(o), kk, 0),
            Some(&mut r0)
        ),
        1
    );
    assert!(f
        .c
        .log
        .iter()
        .any(|l| l.starts_with(&format!("cast {o} Unit"))));
    // ...and with no O nothing is cast.
    f.c.log.clear();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            20,
            ev(7, Some(h), None, kk, 0),
            Some(&mut r0)
        ),
        1
    );
    assert_eq!(casts(&f), 0);
}

#[test]
fn skill_on_get_hit_needs_a_get_hit_result() {
    let ct = monsters(1);
    let mut r = body_rec();
    r.itemeffect = 1;
    r.itemtarget = 1;
    r.srvdofunc = 1;
    r.intown = false;
    let t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    f.srvdo_result = 1;
    let lay = (1 << 6) | 3;
    let h = unit(&mut f, UnitType::Player, &[(196, lay, 100)]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    let kk = k(196, lay);
    let mut rec = DamageRecord::default();
    let run = |f: &mut BodyFake, rec: &mut DamageRecord, o| {
        call(f, tb(&t, &ct), 21, ev(1, Some(h), o, kk, 0), Some(rec))
    };
    // Result without get-hit (4): nothing.
    assert_eq!(run(&mut f, &mut rec, Some(o)), 0);
    rec.result = 4;
    assert_eq!(run(&mut f, &mut rec, Some(o)), 1);
    assert!(f.c.log.iter().any(|l| l.contains("aim: false")));
    // No O: a point cast.
    f.c.log.clear();
    assert_eq!(run(&mut f, &mut rec, None), 1);
    assert!(f
        .c
        .log
        .iter()
        .any(|l| l.starts_with(&format!("cast {h} Point"))));
    // No record: 0.
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            21,
            ev(1, Some(h), Some(o), kk, 0),
            None
        ),
        0
    );
}

#[test]
fn blood_golem_shares_damage_with_its_owner() {
    let mut ct = monsters(300);
    ct.monstats[290].skill1 = 1;
    let mut r = body_rec();
    r.param5 = 30;
    let t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    let q = unit(&mut f, UnitType::Player, &[(6, 0, 1_000)]);
    let h = f.add(FUnit::new(UnitType::Monster, 290), (0, 0));
    f.minion_owner.insert(h, q);
    let mut rec = DamageRecord {
        total: 1_000,
        ..DamageRecord::default()
    };
    // x = 30 % of 1000 = 300: Q 1000 -> 700, R total 1000 -> 700.
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            26,
            ev(1, Some(h), None, 1, 1),
            Some(&mut rec)
        ),
        1
    );
    assert_eq!((f.c.get(q, 6), rec.total), (700, 700));
    // Q's life floors at 256.
    rec.total = 10_000;
    call(
        &mut f,
        tb(&t, &ct),
        26,
        ev(1, Some(h), None, 1, 1),
        Some(&mut rec),
    );
    assert_eq!((f.c.get(q, 6), rec.total), (256, 7_000));
    // Q below 256 life, or R total 0: 0.
    let mut rec = DamageRecord {
        total: 100,
        ..DamageRecord::default()
    };
    f.c.set(q, 6, 255);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            26,
            ev(1, Some(h), None, 1, 1),
            Some(&mut rec)
        ),
        0
    );
    let mut zero = DamageRecord::default();
    f.c.set(q, 6, 5_000);
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            26,
            ev(1, Some(h), None, 1, 1),
            Some(&mut zero)
        ),
        0
    );
}

#[test]
fn rest_in_peace_marks_the_victim() {
    let (t, ct) = plain();
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    assert_eq!(
        call(&mut f, tb(&t, &ct), 29, ev(9, Some(h), None, 1, 1), None),
        0
    );
    assert_eq!(
        call(&mut f, tb(&t, &ct), 29, ev(9, Some(h), Some(o), 1, 1), None),
        1
    );
    assert!(f.c.units[o].states.contains(&172));
    assert!(!f.c.units[h].states.contains(&172));
}

#[test]
fn function_table_dispatch() {
    let (t, ct) = plain();
    // Table slot 0 and every index past 31 are no function: 0, no effect.
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    for func in [0, 32, 33, 40, -1] {
        assert_eq!(
            call(
                &mut f,
                tb(&t, &ct),
                func,
                ev(1, Some(h), Some(o), 1, 1),
                None
            ),
            0
        );
    }
    assert!(f.c.log.is_empty());
    // With a holder, another unit, a valid skill and no record, only
    // Frozen Armor (2), Shiver Armor (3: no record needed) and Rest in
    // peace (29) act; every other slot is a distinct function that needs
    // its inputs.
    let acting: Vec<i32> = (1..=31)
        .filter(|&func| {
            let mut f = BodyFake::new();
            let h = unit(&mut f, UnitType::Player, &[]);
            let o = unit(&mut f, UnitType::Monster, &[]);
            call(
                &mut f,
                tb(&t, &ct),
                func,
                ev(1, Some(h), Some(o), 1, 1),
                None,
            ) != 0
        })
        .collect();
    assert_eq!(acting, [2, 3, 29]);
    // Function 16 (crushing blow, implemented in `damage`) is reached through the table;
    // with no chance stat it returns 0.
    let mut f = BodyFake::new();
    let h = unit(&mut f, UnitType::Player, &[]);
    let o = unit(&mut f, UnitType::Monster, &[]);
    let mut rec = DamageRecord::default();
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            16,
            ev(5, Some(h), Some(o), 1, 1),
            Some(&mut rec)
        ),
        0
    );
}

#[test]
fn iron_maiden_blood_golem_heals_holder_and_owner_not_the_golem() {
    let mut ct = monsters(300);
    ct.monstats[290].drain = 100;
    let mut r = body_rec();
    let mut code = Code::new();
    r.calc1 = code.f(50);
    r.auratargetstate = 12;
    let t = tabs(r, code, 1);
    let mut f = BodyFake::new();
    f.c.hostile = true;
    let q = unit(&mut f, UnitType::Player, &[(6, 0, 100), (7, 0, 1_000)]);
    let h = unit(&mut f, UnitType::Player, &[(6, 0, 1_000), (7, 0, 1_000)]);
    let o = f.add(
        FUnit::new(UnitType::Monster, 290)
            .with(6, 5_000)
            .with(7, 5_000),
        (0, 0),
    );
    f.minion_owner.insert(h, q);
    let l = f.alloc_list(0, 0, Some(h)).unwrap();
    f.set_list_state(l, 12);
    f.attach(h, l);
    f.c.units[h].states.push(12);
    let mut rec = DamageRecord {
        physical: 1_000,
        ..DamageRecord::default()
    };
    assert_eq!(
        call(
            &mut f,
            tb(&t, &ct),
            4,
            ev(5, Some(h), Some(o), 1, 1),
            Some(&mut rec)
        ),
        1
    );
    // Reflected 500 to H (1000 -> 500). x = 20 % of 500 = 100: Q gets
    // half (50), H the rest (50). The golem is not healed.
    assert_eq!(f.c.get(q, 6), 150);
    assert_eq!(f.c.get(h, 6), 550);
    assert_eq!(f.c.get(o, 6), 5_000);
    assert!(f.c.log.contains(&format!("overlay {h} 151")));
    assert!(f.c.log.contains(&format!("overlay {q} 151")));
}

#[test]
fn item_cast_free_point_is_tested_in_the_casters_room() {
    let ct = monsters(1);
    let t = item_cast_tables(false, 2);
    let mut f = BodyFake::new();
    f.srvdo_result = 1;
    let u = f.add(FUnit::new(UnitType::Player, 0), (100, 100));
    // The first candidate: two seed draws, x then y, each `mod 40 - 20`
    // from U's position; far from U, in no room check of its own.
    let mut seed = f.c.units[u].seed;
    let x = 100 + (seed.step() % 40) as i32 - 20;
    let y = 100 + (seed.step() % 40) as i32 - 20;
    f.collides = true;
    f.path_target = (x, y);
    let (res, out) = core(&mut f, tb(&t, &ct), u, 1, 1, None, (0, 0), false);
    assert_eq!(res, 1);
    assert_eq!(out.at, (x, y));
}
