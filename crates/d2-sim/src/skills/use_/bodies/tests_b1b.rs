//! Coverage tests of the shared helpers on [`BodyFake`].

use super::fake::BodyFake;
use super::tests2::{body_rec, monster, tabs, world, Code};
use super::*;
use crate::combat::{CombatEntry, DamageRecord};
use crate::skills::fake::{combat_tables, monster_rec, FItem, FUnit};
use crate::skills::{SkillEntry, SkillUnits};
use crate::units::UnitType;

#[test]
fn target_is_the_path_target_unless_it_is_the_unit() {
    let (mut f, u) = world();
    let m = monster(&mut f, (5, 5));
    assert_eq!(target(&f, u), None);
    f.targets.insert(u, m);
    assert_eq!(target(&f, u), Some(m));
    f.targets.insert(u, u);
    assert_eq!(target(&f, u), None);
}

#[test]
fn pair_record_is_the_first_entry_of_the_pair() {
    let (mut f, u) = world();
    let a = monster(&mut f, (1, 1));
    let b = monster(&mut f, (2, 2));
    let ident = |f: &BodyFake, x: usize| (f.c.units[x].kind, f.c.units[x].guid);
    let mk = |att, def, hit: i32| CombatEntry {
        attacker: att,
        defender: def,
        record: DamageRecord {
            enh_pct: hit,
            ..DamageRecord::default()
        },
    };
    let (iu, ia, ib) = (ident(&f, u), ident(&f, a), ident(&f, b));
    f.c.units[u].combat = vec![mk(iu, ia, 1), mk(iu, ib, 2), mk(iu, ib, 3)];
    assert_eq!(helpers::pair_record(&mut f, u, a), Some(0));
    assert_eq!(helpers::pair_record(&mut f, u, b), Some(1));
    assert_eq!(helpers::pair_damage(&mut f, u, b).unwrap().enh_pct, 2);
    // No entry for the reverse pair.
    assert_eq!(helpers::pair_record(&mut f, b, u), None);
}

#[test]
fn bow_class_and_missile_choice() {
    let (mut f, u) = world();
    for (c, bow) in [(0, false), (1, true), (2, false), (7, true), (8, false)] {
        f.composit_class = c;
        assert_eq!(is_bow(&f, u), bow, "class {c}");
    }
    // Player: hand class 1 arrows, 7 bolts, else −1.
    f.hand_class = 1;
    assert_eq!(bow_missile(&f, u), (0, None));
    f.hand_class = 7;
    assert_eq!(bow_missile(&f, u), (31, None));
    f.hand_class = 3;
    assert_eq!(bow_missile(&f, u), (-1, None));
    // Magic arrow beats explosive arrow and replaces the level.
    f.hand_class = 7;
    f.c.set(u, 158, 4);
    assert_eq!(bow_missile(&f, u), (41, Some(4)));
    f.c.set(u, 157, 6);
    assert_eq!(bow_missile(&f, u), (27, Some(6)));
    // Another unit type shoots arrows whatever its hand class.
    let m = monster(&mut f, (1, 1));
    assert_eq!(bow_missile(&f, m), (0, None));
    f.c.set(m, 158, 2);
    assert_eq!(bow_missile(&f, m), (41, Some(2)));
}

fn player_with_stack(f: &mut BodyFake, u: usize) -> usize {
    let i = f.c.add_item(FItem {
        throw: true,
        ..FItem::default()
    });
    f.c.units[u].weapon = Some(i);
    f.c.units[u].items.insert(4, i);
    i
}

#[test]
fn ammunition_check_and_quantity_rule() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    assert!(has_ammo(&mut f, m), "non-players always have ammo");
    // No weapon → 0.
    assert!(!has_ammo(&mut f, u));
    let i = player_with_stack(&mut f, u);
    // `any`: the stack counts. Quantity 0 and a stack maximum > 0 → 0.
    f.max_stack.insert(i, 10);
    assert!(!has_ammo(&mut f, u));
    f.item_stats.insert((i, 70), 5);
    assert!(has_ammo(&mut f, u));
    // No used skill → 0.
    let used = f.c.units[u].used.take();
    assert!(!has_ammo(&mut f, u));
    f.c.units[u].used = used;
    // has_qty: quantity 0 but item_throwable ≠ 0 → quantity := 0, 1.
    f.item_stats.insert((i, 70), 0);
    f.item_stats.insert((i, 125), 1);
    assert!(has_qty(&mut f, i));
    assert!(f.take_log().contains(&format!("itemstat {i} 70 0")));
    f.item_stats.insert((i, 125), 0);
    assert!(!has_qty(&mut f, i));
    // Maximum stack ≤ 0 → 1.
    f.max_stack.insert(i, 0);
    assert!(has_qty(&mut f, i));
    // A shooting weapon: only W itself counts (any = 0); a stack in the
    // other hand is not accepted.
    f.max_stack.insert(i, 10);
    f.shoots.insert(i);
    let q = f.c.add_item(FItem {
        throw: true,
        ..FItem::default()
    });
    f.c.units[u].items.insert(5, q);
    f.item_stats.insert((q, 70), 9);
    assert!(!has_ammo(&mut f, u));
    // Used skill 0 (Attack) and W has item_magicarrow → 1.
    f.c.units[u].used = Some(SkillEntry {
        skill: 0,
        ..f.c.units[u].used.unwrap()
    });
    f.item_stats.insert((i, 157), 1);
    assert!(has_ammo(&mut f, u));
    // Without magic arrows, W with quantity → 1.
    f.item_stats.insert((i, 157), 0);
    f.item_stats.insert((i, 70), 2);
    assert!(has_ammo(&mut f, u));
}

#[test]
fn skill_missile_record_by_kind() {
    let (mut f, u) = world();
    f.pos.insert(u, (100, 200));
    assert!(helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (2, -3),
        (50, 60),
        false,
        false
    ));
    let r = f.missiles[0];
    assert_eq!((r.flags, r.class, r.skill, r.level), (0x21, 3, 1, 4));
    assert_eq!((r.x, r.y, r.target_x, r.target_y), (102, 197, 50, 60));
    assert_eq!(r.origin, None);
    assert!(helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (2, -3),
        (50, 60),
        false,
        true
    ));
    let r = f.missiles[1];
    assert_eq!((r.flags, r.origin), (0x420, Some(u)));
    // A monster with tohit: straight gets flag 0x1000 and the bonus; the
    // lob does not.
    let m = monster(&mut f, (10, 10));
    f.c.set(m, 19, 33);
    assert!(helpers::skill_missile(
        &mut f,
        3,
        m,
        1,
        4,
        (0, 0),
        (5, 5),
        false,
        false
    ));
    let r = f.missiles[2];
    assert_eq!((r.flags, r.attack_bonus), (0x21 | 0x1000, 33));
    assert!(helpers::skill_missile(
        &mut f,
        3,
        m,
        1,
        4,
        (0, 0),
        (5, 5),
        false,
        true
    ));
    assert_eq!(
        (f.missiles[3].flags, f.missiles[3].attack_bonus),
        (0x420, 0)
    );
    // tx or ty = 0 → the target position; none → no missile.
    assert!(!helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (0, 0),
        (0, 7),
        false,
        false
    ));
    f.tpos.insert(u, (8, 9));
    assert!(helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (0, 0),
        (0, 7),
        false,
        false
    ));
    let r = f.missiles[4];
    assert_eq!((r.target_x, r.target_y), (8, 9));
    // A target position with a 0 coordinate fails.
    f.tpos.insert(u, (8, 0));
    assert!(!helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (0, 0),
        (0, 0),
        false,
        false
    ));
    // quant: a player pays one; no quantity → no missile.
    let n = f.missiles.len();
    assert!(!helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (0, 0),
        (5, 5),
        true,
        false
    ));
    assert_eq!(f.missiles.len(), n);
    let i = player_with_stack(&mut f, u);
    f.item_stats.insert((i, 70), 3);
    assert!(helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (0, 0),
        (5, 5),
        true,
        false
    ));
    assert_eq!(f.item_stats[&(i, 70)], 2);
    // The missile itself failing → none.
    f.no_missiles = true;
    assert!(!helpers::skill_missile(
        &mut f,
        3,
        u,
        1,
        4,
        (0, 0),
        (5, 5),
        false,
        false
    ));
}

#[test]
fn dec_quantity_returns_zero_without_a_stack() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    assert_eq!(helpers::dec_quantity(&mut f, m), 0, "non-player");
    // No item in either hand.
    assert_eq!(helpers::dec_quantity(&mut f, u), 0);
    // A weapon that is not a stack and no other item.
    let w = f.c.add_item(FItem::default());
    f.c.units[u].weapon = Some(w);
    f.c.units[u].items.insert(4, w);
    assert_eq!(helpers::dec_quantity(&mut f, u), 0);
    // A stack that is not the weapon: weapon_only → skipped.
    let q = f.c.add_item(FItem {
        throw: true,
        ..FItem::default()
    });
    f.c.units[u].items.insert(5, q);
    assert_eq!(helpers::dec_quantity(&mut f, u), 0);
    // A bow (item type 27) as the weapon lifts weapon_only: the quiver
    // in the other hand is used.
    f.c.items[w].types.push(27);
    f.item_stats.insert((q, 70), 4);
    assert_eq!(helpers::dec_quantity(&mut f, u), 4);
    assert_eq!(f.item_stats[&(q, 70)], 3);
}

fn calc_tables() -> (crate::skills::SkillTables, i16) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastat1 = 68;
    r.aurastatcalc1 = c.f(10);
    r.aurastat2 = 25;
    r.aurastatcalc2 = c.f(0);
    r.aurastat3 = 26;
    r.aurastatcalc3 = c.f(-4);
    r.aurastat4 = 5000;
    r.aurastatcalc4 = c.f(7);
    r.aurastat5 = 0xFFFF;
    r.aurastatcalc5 = c.f(7);
    r.aurastat6 = 27;
    r.aurastatcalc6 = c.f(6);
    r.passivestat1 = 30;
    r.passivecalc1 = c.f(3);
    r.passivestat2 = 31;
    r.passivecalc2 = c.f(0);
    r.passivestat5 = 32;
    r.passivecalc5 = c.f(9);
    (tabs(r, c, 1), 1)
}

#[test]
fn aura_and_passive_fills_set_nonzero_valid_stats() {
    let (t, sk) = calc_tables();
    let (mut f, u) = world();
    let l = f.alloc_list(0, 0, None).unwrap();
    helpers::aura_fill(&mut f, &t, u, l, i32::from(sk), 1);
    assert_eq!(f.list_get(l, 68), 10);
    assert_eq!(f.list_get(l, 69), 10, "attackrate also sets 69");
    assert_eq!(f.list_get(l, 26), -4);
    assert_eq!(f.list_get(l, 27), 6);
    // Zero values, invalid and none stats are not written.
    assert!(!f.lists[l].stats.contains_key(&25));
    assert!(!f.lists[l].stats.contains_key(&5000));
    assert_eq!(f.lists[l].stats.len(), 4);
    assert!(f.take_log().contains(&format!("anim {u}")));
    let p = f.alloc_list(0, 0, None).unwrap();
    helpers::passive_fill(&mut f, &t, u, p, i32::from(sk), 1);
    assert_eq!(f.list_get(p, 30), 3);
    assert_eq!(f.list_get(p, 32), 9);
    assert_eq!(f.lists[p].stats.len(), 2);
    // An invalid skill does nothing (not even the refresh).
    let z = f.alloc_list(0, 0, None).unwrap();
    f.take_log();
    helpers::aura_fill(&mut f, &t, u, z, 99, 1);
    helpers::passive_fill(&mut f, &t, u, z, -1, 1);
    assert!(f.lists[z].stats.is_empty());
    assert!(f.take_log().is_empty());
}

fn ct2() -> crate::combat::CombatTables {
    let mut ct = combat_tables(vec![monster_rec(), monster_rec(), monster_rec()]);
    ct.monstats2[0].isatt = true;
    ct.monstats[1].npc = true;
    ct.monstats[2].monstatsex = 2;
    ct
}

#[test]
fn apply_state_refuses_bad_states_and_non_attackable_monsters() {
    let ct = ct2();
    let (mut f, u) = world();
    f.states = 10;
    let mk = |target, state| StateRequest {
        source: u,
        target,
        skill: 5,
        level: 2,
        duration: 30,
        stat: 7,
        value: 11,
        state,
        callback: 0,
    };
    // A state outside 0 ≤ s < states count.
    assert!(apply_state(&mut f, &ct, mk(u, -1)).is_none());
    assert!(apply_state(&mut f, &ct, mk(u, 10)).is_none());
    // Monsters: npc, or no isAtt → none (class 1 is npc; class 2
    // reads the blank monstats2 row 2 → isatt clear; class 3 has no
    // record).
    let ok = f.add(FUnit::new(UnitType::Monster, 0), (1, 1));
    let npc = f.add(FUnit::new(UnitType::Monster, 1), (1, 1));
    let noatt = f.add(FUnit::new(UnitType::Monster, 2), (1, 1));
    let none = f.add(FUnit::new(UnitType::Monster, 3), (1, 1));
    for m in [npc, noatt, none] {
        assert!(apply_state(&mut f, &ct, mk(m, 3)).is_none());
    }
    // The request fields are used as laid out.
    f.c.frame = 100;
    let l = apply_state(&mut f, &ct, mk(ok, 3)).expect("list");
    let l = &f.lists[l];
    assert_eq!((l.state, l.skill, l.lvl), (3, 5, 2));
    assert_eq!(l.stats.get(&7), Some(&11));
    assert_eq!((l.expire, l.flags & 2), (130, 2));
    assert_eq!(l.callback, callback::DEFAULT);
    assert_eq!(l.unit, Some(ok));
    let m2 = apply_state(
        &mut f,
        &ct,
        StateRequest {
            callback: 0x1234,
            state: 4,
            stat: -1,
            ..mk(ok, 4)
        },
    )
    .unwrap();
    assert_eq!(f.lists[m2].callback, 0x1234);
    assert!(f.lists[m2].stats.is_empty());
}

fn scan_world() -> (BodyFake, usize, usize, usize, usize) {
    let (mut f, u) = world();
    let a = f.add(FUnit::new(UnitType::Player, 0), (10, 0));
    let b = monster(&mut f, (0, 6));
    let c = monster(&mut f, (30, 40));
    f.scan = vec![u, a, b, c];
    (f, u, a, b, c)
}

fn run_scan(
    f: &mut BodyFake,
    u: usize,
    at: (i32, i32),
    r: i32,
    flt: u32,
    noaura: bool,
    ret: i32,
) -> (i32, Vec<usize>) {
    let t = crate::skills::fake::skill_tables(vec![]);
    let ct = ct2();
    let mut seen = Vec::new();
    let n = scan_unit(f, &t, &ct, u, at, r, flt, noaura, &mut |_, x| {
        seen.push(x);
        if x == 1 {
            0
        } else {
            ret
        }
    });
    (n, seen)
}

#[test]
fn scan_unit_visits_accepted_units_in_range() {
    let (mut f, u, a, b, _c) = scan_world();
    // R ≤ 0 → nothing; no room → nothing.
    assert_eq!(run_scan(&mut f, u, (0, 0), 0, 3, false, 1), (0, vec![]));
    assert_eq!(run_scan(&mut f, u, (0, 0), -5, 3, false, 1), (0, vec![]));
    f.room_of.remove(&u);
    assert_eq!(run_scan(&mut f, u, (0, 0), 100, 3, false, 1), (0, vec![]));
    f.room_of.insert(u, 1);
    // The source is excluded; d² ≤ r² (10² = 100: the player at
    // (10, 0) is exactly in); (30, 40) is 50 away.
    let (n, seen) = run_scan(&mut f, u, (0, 0), 10, 3, false, 1);
    assert_eq!(seen, vec![a, b]);
    // `a` has unit id 1 and its callback returns 0: only `b` counts.
    assert_eq!(n, 1);
    // One short of the player: d² = 100 > 81.
    assert_eq!(run_scan(&mut f, u, (0, 0), 9, 3, false, 1).1, vec![b]);
    // x or y = 0 means the source's position; an explicit centre moves it.
    f.pos.insert(u, (30, 0));
    assert_eq!(
        run_scan(&mut f, u, (0, 0), 7, 3, false, 1).1,
        Vec::<usize>::new()
    );
    assert_eq!(
        run_scan(&mut f, u, (30, 40), 5, 3, false, 1).1,
        vec![f.scan[3]]
    );
    f.pos.insert(u, (0, 0));
    // f = 0 is replaced by 0x583, which needs the unit flags 0x4 and 0x8.
    assert_eq!(
        run_scan(&mut f, u, (0, 0), 10, 0, false, 1).1,
        Vec::<usize>::new()
    );
    for x in [a, b] {
        f.c.units[x].flags = 0xC;
    }
    assert_eq!(run_scan(&mut f, u, (0, 0), 10, 0, false, 1).1, vec![a, b]);
    // f only selects what `accepts` accepts: monsters only.
    assert_eq!(run_scan(&mut f, u, (0, 0), 10, 2, false, 1).1, vec![b]);
    // Town rooms are skipped with 0x2000 only.
    let t = f.add(FUnit::new(UnitType::Monster, 0), (1, 1));
    f.scan_town = vec![t];
    assert_eq!(
        run_scan(&mut f, u, (0, 0), 10, 3, false, 1).1,
        vec![a, b, t]
    );
    assert_eq!(
        run_scan(&mut f, u, (0, 0), 10, 3 | 0x2000, false, 1).1,
        vec![a, b]
    );
    // noaura skips a monster whose class has monstats `noaura`.
    let mut ct = ct2();
    ct.monstats[0].noaura = true;
    let tb = crate::skills::fake::skill_tables(vec![]);
    let mut seen = Vec::new();
    scan_unit(&mut f, &tb, &ct, u, (0, 0), 10, 3, true, &mut |_, x| {
        seen.push(x);
        1
    });
    assert_eq!(seen, vec![a], "noaura drops the class-0 monsters");
    seen.clear();
    scan_unit(&mut f, &tb, &ct, u, (0, 0), 10, 3, false, &mut |_, x| {
        seen.push(x);
        1
    });
    assert_eq!(seen, vec![a, b, t]);
}

#[test]
fn scan_point_uses_the_target_point_and_keeps_f() {
    let (mut f, u, a, b, _c) = scan_world();
    let (t, ct) = (crate::skills::fake::skill_tables(vec![]), ct2());
    let mut seen = Vec::new();
    // No target position: returns 0 and calls nothing.
    assert_eq!(
        scan_point(&mut f, &t, &ct, 3, u, 100, &mut |_, x| {
            seen.push(x);
            1
        }),
        0
    );
    assert!(seen.is_empty());
    // Centre (0, 6): the source is not excluded; f = 0 is not replaced
    // (nothing is accepted).
    f.tpos.insert(u, (0, 6));
    assert_eq!(
        scan_point(&mut f, &t, &ct, 0, u, 7, &mut |_, x| {
            seen.push(x);
            1
        }),
        1
    );
    assert!(seen.is_empty());
    assert_eq!(
        scan_point(&mut f, &t, &ct, 3, u, 7, &mut |_, x| {
            seen.push(x);
            1
        }),
        1
    );
    assert_eq!(seen, vec![u, b], "the source at distance 6 is visited");
    seen.clear();
    f.tpos.insert(u, (10, 0));
    scan_point(&mut f, &t, &ct, 3, u, 0, &mut |_, x| {
        seen.push(x);
        1
    });
    assert_eq!(seen, vec![a], "r = 0 keeps d² = 0 only");
}

// ---------------------------------------------------------------

/// A world where the player (unit 0) attacks a player in run mode
/// (always hit), hostile and in melee range, with a fixed damage roll.
fn duel() -> (BodyFake, usize, usize) {
    let (mut f, u) = world();
    let mut d = FUnit::new(UnitType::Player, 1);
    d.mode = 3;
    let m = f.add(d, (1, 0));
    f.c.hostile = true;
    f.c.in_range = true;
    f.targets.insert(u, m);
    f.c.set(u, 21, 100);
    f.c.set(u, 22, 100);
    (f, u, m)
}

fn last_entry(f: &mut BodyFake, u: usize) -> CombatEntry {
    f.c.units[u].combat[0].clone()
}

fn ct3() -> crate::combat::CombatTables {
    combat_tables(vec![monster_rec(), monster_rec()])
}

#[test]
fn attack_start_frame_meleeonly_and_ammo() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = ct3();
    let (mut f, u, m) = duel();
    // The animation frame := frame bonus << 8.
    f.frame_bonus_v = 3;
    assert_eq!(starts::attack(&mut f, &t, &ct, u, 1), 1);
    assert_eq!(f.anim_frame[&u], 768);
    // A plain attack (no bow, no meleeonly state) needs nothing else.
    assert!(f.c.units[u].combat.is_empty());
    // A bow without ammunition is refused and the cleanups run.
    f.composit_class = 1;
    f.take_log();
    assert_eq!(starts::attack(&mut f, &t, &ct, u, 1), 0);
    let log = f.take_log();
    let i = log.iter().position(|l| *l == format!("attackcleanup {u}"));
    assert_eq!(
        log.iter().position(|l| *l == format!("weaponcleanup {u}")),
        i.map(|i| i + 1)
    );
    assert!(i.is_some());
    // …with ammunition, 1.
    player_with_stack(&mut f, u);
    f.item_stats.insert((0, 70), 9);
    assert_eq!(starts::attack(&mut f, &t, &ct, u, 1), 1);
    assert!(f.take_log().iter().all(|l| !l.contains("cleanup")));
    // A unit in a meleeonly (group 38) state runs the shape start,
    // even with a bow without ammunition.
    f.item_stats.insert((0, 70), 0);
    f.max_stack.insert(0, 10);
    f.state_flags.insert((139, group::MELEEONLY));
    f.c.units[u].states.push(139);
    f.targets.remove(&u);
    assert_eq!(starts::attack(&mut f, &t, &ct, u, 1), 0, "no target");
    f.targets.insert(u, m);
    assert_eq!(starts::attack(&mut f, &t, &ct, u, 1), 1);
    assert_eq!(f.c.units[u].combat.len(), 1);
    assert_eq!(last_entry(&mut f, u).record.hit_class, 1);
}

#[test]
fn kick_makes_a_forced_hit_record() {
    let mut r = body_rec();
    r.mindam = 5;
    r.hitshift = 3;
    let t = tabs(r, Code::new(), 1);
    let ct = ct3();
    let (mut f, u, m) = duel();
    f.targets.remove(&u);
    assert_eq!(starts::kick(&mut f, &t, &ct, u, 1), 0);
    f.targets.insert(u, m);
    // The body hands `start_combat` the forced-hit record: compare with
    // the same call made by hand on a copy of the world.
    let mut g = f.clone();
    let mut want = DamageRecord {
        hit_flags: 2,
        result: 9,
        physical: 5 << 3,
        hit_class: 1,
        ..DamageRecord::default()
    };
    crate::combat::start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 128);
    assert_eq!(starts::kick(&mut f, &t, &ct, u, 1), 1);
    let e = last_entry(&mut f, u);
    assert_eq!(e.record, want);
    assert_eq!((e.record.hit_class, e.record.hit_flags & 2), (1, 2));
    assert_eq!(e.record.result & 9, 9, "hit + knockback");
    // An invalid skill reads no record (the base game is fatal).
    assert_eq!(starts::kick(&mut f, &t, &ct, u, 99), 0);
}

#[test]
fn unsummon_needs_the_owner_and_an_unsummonable_pet() {
    let (mut f, u) = world();
    let m = monster(&mut f, (5, 5));
    f.targets.insert(u, m);
    assert_eq!(starts::unsummon(&mut f, u), 0, "not the owner");
    f.minion_owner.insert(m, u);
    assert_eq!(starts::unsummon(&mut f, u), 0, "pet type not unsummonable");
    f.unsummon_ok = true;
    assert_eq!(starts::unsummon(&mut f, u), 1);
    assert_eq!(f.entries[&(u, 1, 1)], f.c.units[m].guid as i32);
    // No used skill entry → 0.
    f.c.units[u].used = None;
    assert_eq!(starts::unsummon(&mut f, u), 0);
    f.c.units[u].used = f.c.units[u].skills.first().copied();
    // The caster must be a player, the target a monster.
    let p = f.add(FUnit::new(UnitType::Player, 2), (9, 9));
    f.targets.insert(u, p);
    f.minion_owner.insert(p, u);
    assert_eq!(starts::unsummon(&mut f, u), 0);
    f.targets.remove(&u);
    assert_eq!(starts::unsummon(&mut f, u), 0);
    f.targets.insert(m, m);
    f.minion_owner.insert(u, m);
    f.c.units[m].skills = f.c.units[u].skills.clone();
    f.c.units[m].used = f.c.units[u].used;
    f.targets.insert(m, u);
    assert_eq!(starts::unsummon(&mut f, m), 0, "a monster caster");
}

#[test]
fn arrow_bolt_and_throw_starts_need_ammunition() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = ct3();
    for slot in [4u16, 65] {
        let (mut f, u) = world();
        assert_eq!(run_start(&mut f, &t, &ct, slot, u, 1, 1), Some(0));
        let log = f.take_log();
        assert!(log.contains(&format!("attackcleanup {u}")), "slot {slot}");
        assert!(log.contains(&format!("weaponcleanup {u}")), "slot {slot}");
        let i = player_with_stack(&mut f, u);
        f.item_stats.insert((i, 70), 3);
        assert_eq!(run_start(&mut f, &t, &ct, slot, u, 1, 1), Some(1));
        assert!(f.take_log().iter().all(|l| !l.contains("cleanup")));
    }
}

#[test]
fn jab_start_needs_a_target() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, u) = world();
    let m = monster(&mut f, (5, 5));
    assert_eq!(starts::jab(&mut f, &t, u, 1), 0);
    f.targets.insert(u, m);
    assert_eq!(starts::jab(&mut f, &t, u, 1), 1);
    assert_eq!(starts::jab(&mut f, &t, u, 77), 0, "R invalid");
}

fn corpse_world() -> (BodyFake, usize, usize, crate::combat::CombatTables) {
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2[0].corpsesel = true;
    ct.monstats2[0].soft = true;
    let (mut f, u) = world();
    let mut c = FUnit::new(UnitType::Monster, 0);
    c.mode = 12;
    let m = f.add(c, (4, 4));
    f.targets.insert(u, m);
    (f, u, m, ct)
}

#[test]
fn raise_start_needs_a_raisable_corpse() {
    let (mut f, u, m, mut ct) = corpse_world();
    assert_eq!(starts::raise(&mut f, &ct, u), 1);
    // In a town room → 0.
    f.town.insert(1);
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
    f.town.clear();
    // Not dead (mode 12), a udead state (group 33), no corpseSel, no
    // Velocity, not a monster, no target.
    f.c.units[m].mode = 1;
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
    f.c.units[m].mode = 12;
    f.state_flags.insert((77, group::UDEAD));
    f.c.units[m].states.push(77);
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
    f.c.units[m].states.clear();
    ct.monstats[0].velocity = 0;
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
    ct.monstats[0].velocity = 1;
    ct.monstats2[0].corpsesel = false;
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
    ct.monstats2[0].corpsesel = true;
    f.targets.remove(&u);
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
}

#[test]
fn find_potion_start_needs_a_soft_unmarked_corpse() {
    let (mut f, u, m, mut ct) = corpse_world();
    assert_eq!(starts::find_potion(&mut f, &ct, u), 1);
    f.c.units[m].states.push(118);
    assert_eq!(starts::find_potion(&mut f, &ct, u), 0, "corpse_noselect");
    f.c.units[m].states.clear();
    ct.monstats2[0].soft = false;
    assert_eq!(starts::find_potion(&mut f, &ct, u), 0);
    ct.monstats2[0].soft = true;
    f.c.units[m].mode = 1;
    assert_eq!(starts::find_potion(&mut f, &ct, u), 0);
    f.c.units[m].mode = 12;
    f.targets.remove(&u);
    assert_eq!(starts::find_potion(&mut f, &ct, u), 0);
}

#[test]
fn andrial_spray_start_stores_the_target_position() {
    let (mut f, u) = world();
    let m = monster(&mut f, (123, -45));
    assert_eq!(starts::andrial_spray(&mut f, u), 0);
    f.targets.insert(u, m);
    assert_eq!(starts::andrial_spray(&mut f, u), 1);
    assert_eq!((f.entries[&(u, 1, 1)], f.entries[&(u, 1, 2)]), (123, -45));
    f.c.units[u].used = None;
    assert_eq!(starts::andrial_spray(&mut f, u), 0);
}

/// The skills record the Sacrifice / Bash tests use: formulas 40, 3,
/// −25 and 30 in calc1–calc4, EType 3, SrcDam `src`.
fn strike_tables(src: u8, hitflags: u32, hitclass: u32) -> (crate::skills::SkillTables, Code) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(40);
    r.calc2 = c.f(3);
    r.calc3 = c.f(-25);
    r.calc4 = c.f(30);
    r.etype = 3;
    r.srcdam = src;
    r.resultflags = 0x0400;
    r.hitflags = hitflags;
    r.hitclass = hitclass;
    let t = tabs(r, Code(c.0.clone()), 1);
    (t, c)
}

#[test]
fn sacrifice_start_rolls_a_melee_hit_and_always_starts_combat() {
    use crate::combat::{bonuses, melee_result, start_combat};
    let ct = ct3();
    let (t, _) = strike_tables(0, 2, 0);
    // Refusals: R invalid, no used skill, no target, not in range.
    let (mut f, u, m) = duel();
    assert_eq!(starts::sacrifice(&mut f, &t, &ct, u, 99, 1), 0);
    f.c.in_range = false;
    assert_eq!(starts::sacrifice(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.in_range = true;
    f.targets.remove(&u);
    assert_eq!(starts::sacrifice(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(u, m);
    let used = f.c.units[u].used.take();
    assert_eq!(starts::sacrifice(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.units[u].used = used;
    assert!(f.c.units[u].combat.is_empty());
    // A hit: physical from bonuses(get, pct = calc1, s = SrcDam), the
    // conversion (EType ≠ 0, calc4 > 0), roll_elemental, ResultFlags,
    // hit flags = HitFlags | 1; replayed by hand on a copy.
    let mut g = f.clone();
    let bonus = crate::skills::to_hit(&mut g, &t, Some(u), 1, 1);
    let mut want = DamageRecord {
        result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), bonus, 0),
        ..DamageRecord::default()
    };
    assert_eq!(want.result & 1, 1, "run mode: always a hit");
    want.physical = bonuses(&mut g.c, &t, u, true, None, 0, 0, 40, 0, 0);
    want.conv_pct = 30;
    want.conv_elem = 3;
    crate::skills::roll_elemental(&mut g, &t, u, &mut want, 1, 1);
    want.result |= 0x0400;
    want.hit_flags = 2 | 1;
    start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 128);
    assert_eq!(starts::sacrifice(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(last_entry(&mut f, u).record, want);
    assert_eq!(f.c.units[u].seed, g.c.units[u].seed, "same draws");
    // SrcDam is passed on to bonuses (and 0 stays 0).
    let (t2, _) = strike_tables(64, 2, 0);
    let mut g = f.clone();
    g.c.units[u].combat.clear();
    f.c.units[u].combat.clear();
    assert_eq!(starts::sacrifice(&mut f, &t2, &ct, u, 1, 1), 1);
    let want_phys = bonuses(&mut g.c, &t2, u, true, None, 0, 0, 40, 0, 64);
    assert_ne!(want_phys, 0);
    assert_eq!(last_entry(&mut f, u).record.physical, {
        let mut r = DamageRecord {
            physical: want_phys,
            ..DamageRecord::default()
        };
        crate::combat::totals(&mut g.c, &ct, Some(u), m, &mut r);
        r.physical
    });
    // A miss: nothing but the melee result is stored, and combat still
    // starts (an entry exists).
    f.c.hostile = false;
    f.c.units[u].combat.clear();
    assert_eq!(starts::sacrifice(&mut f, &t, &ct, u, 1, 1), 1);
    let e = last_entry(&mut f, u);
    assert_eq!(
        (e.record.result, e.record.physical, e.record.hit_flags),
        (0, 0, 0)
    );
}

#[test]
fn bash_start_attackrate_list_record_pair_bonus_and_aura_state() {
    use crate::combat::{melee_result, start_combat};
    let ct = ct3();
    let (mut t, _) = strike_tables(0, 2, 7);
    // aurastate 40 with aurastat1 = 25 (value calc2 = 3).
    t.skills[1].aurastate = 40;
    t.skills[1].aurastat1 = 25;
    t.skills[1].aurastatcalc1 = t.skills[1].calc2;
    // Refusals: R invalid, no target, target in a town room.
    let (mut f, u, m) = duel();
    assert_eq!(starts::bash(&mut f, &t, &ct, u, 99, 1), 0);
    f.town.insert(1);
    assert_eq!(starts::bash(&mut f, &t, &ct, u, 1, 1), 0);
    f.town.clear();
    f.targets.remove(&u);
    assert_eq!(starts::bash(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(u, m);
    assert!(f.lists.is_empty() && f.c.units[u].combat.is_empty());
    // The full run, replayed by hand on a copy.
    let mut g = f.clone();
    let bonus = crate::skills::to_hit(&mut g, &t, Some(u), 1, 1);
    let mut want = DamageRecord {
        result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), bonus, 0),
        ..DamageRecord::default()
    };
    assert_eq!(want.result & 1, 1);
    want.result |= 0x0400;
    want.hit_flags |= 2;
    want.hit_class = 7;
    want.enh_pct = 40;
    want.conv_pct = 30;
    want.conv_elem = 3;
    crate::skills::roll_elemental(&mut g, &t, u, &mut want, 1, 1);
    start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 128);
    want.physical += 3 << 8;
    f.take_log();
    assert_eq!(starts::bash(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(
        last_entry(&mut f, u).record,
        want,
        "pair bonus after the roll"
    );
    // The temporary attack-rate list: flags 4, expire 0, no state.
    let tmp = f
        .lists
        .iter()
        .find(|l| l.stats.get(&68) == Some(&-25))
        .unwrap();
    assert_eq!(
        (tmp.flags, tmp.expire, tmp.owner, tmp.unit),
        (4, 0, Some(u), Some(u))
    );
    assert!(f.take_log().contains(&format!("anim {u}")));
    // The aurastate list: state 40, flags 4, default callback, on the
    // unit, filled by aura_fill; the state is on.
    let a = f.list_of(u, 40).expect("aura list");
    assert_eq!((a.flags, a.expire, a.callback), (4, 0, callback::DEFAULT));
    assert_eq!(a.stats.get(&25), Some(&3));
    assert!(f.has_state(u, 40));
    // A second run reuses the list.
    let n = f.lists.len();
    assert_eq!(starts::bash(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.lists.iter().filter(|l| l.state == 40).count(), 1);
    assert_eq!(f.lists.len(), n + 1, "only the attack-rate list is new");
    // SrcDam 0 → 128; a miss stores a bare record (no hit work).
    f.c.hostile = false;
    f.c.units[u].combat.clear();
    assert_eq!(starts::bash(&mut f, &t, &ct, u, 1, 1), 1);
    let e = last_entry(&mut f, u);
    assert_eq!(
        (e.record.result, e.record.hit_class, e.record.enh_pct),
        (0, 0, 0)
    );
    // aurastate invalid (−1): no state list.
    let mut t3 = t.clone();
    t3.skills[1].aurastate = 0xFFFF;
    let (mut f, u, _) = duel();
    assert_eq!(starts::bash(&mut f, &t3, &ct, u, 1, 1), 1);
    assert!(f.lists.iter().all(|l| l.state != 40));
}

// ---------------------------------------------------------------

fn hit_entry(f: &mut BodyFake, u: usize, m: usize, result: u16) {
    let e = CombatEntry {
        attacker: (f.c.units[u].kind, f.c.units[u].guid),
        defender: (f.c.units[m].kind, f.c.units[m].guid),
        record: DamageRecord {
            result,
            ..DamageRecord::default()
        },
    };
    f.c.units[u].combat.insert(0, e);
}

fn life_of(f: &BodyFake, u: usize) -> i32 {
    f.c.get(u, 6)
}

fn pgsv_list(f: &mut BodyFake, u: usize, state: i32, skill: i32) -> usize {
    f.state_flags.insert((state, group::PGSV));
    f.c.units[u].states.push(state as u16);
    let l = f.alloc_list(0, 0, None).unwrap();
    f.set_list_state(l, state);
    f.attach(u, l);
    f.list_set(l, 350, skill);
    l
}

#[test]
fn attack_do_flag_potion_bow_and_target() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = ct3();
    let (mut f, u, m) = duel();
    // No target → 0;: unit flags |= 0x40 first.
    f.targets.remove(&u);
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // A missile potion weapon (item type 38) → 0 before any attack.
    f.targets.insert(u, m);
    f.c.units[u].flags = 0;
    let w = f.c.add_item(FItem {
        types: vec![38],
        ..FItem::default()
    });
    f.c.units[u].weapon = Some(w);
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert!(f.c.units[u].combat.is_empty());
    f.c.units[u].weapon = None;
    // A bow shoots: arrows (class 0) cost one quantity (quant = 1);
    // the call returns 1 whatever the missile creation gave.
    f.composit_class = 1;
    f.tpos.insert(u, (40, 50));
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 3, 7), 1);
    assert!(f.missiles.is_empty(), "no quantity: no missile");
    let i = player_with_stack(&mut f, u);
    f.item_stats.insert((i, 70), 2);
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 3, 7), 1);
    let r = f.missiles[0];
    assert_eq!((r.class, r.skill, r.level, r.flags), (0, 3, 7, 0x21));
    assert_eq!((r.target_x, r.target_y), (40, 50));
    assert_eq!(f.item_stats[&(i, 70)], 1);
    // Magic arrows (class 27) replace the level and cost nothing.
    f.c.set(u, 157, 5);
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 3, 7), 1);
    let r = f.missiles[1];
    assert_eq!((r.class, r.level), (27, 5));
    assert_eq!(f.item_stats[&(i, 70)], 1);
    // Bolts: hand class 7 → class 31.
    f.c.set(u, 157, 0);
    f.hand_class = 7;
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 3, 7), 1);
    assert_eq!(f.missiles[2].class, 31);
    // A bow never melees.
    assert!(f.c.units[u].combat.is_empty());
}

#[test]
fn attack_do_melee_pipeline_and_wolf_bear_path() {
    use crate::combat::{apply_melee, fill, melee_result, start_combat};
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = ct3();
    let (mut f, u, m) = duel();
    f.c.set(u, 325, 17);
    f.c.set(m, 6, 100_000);
    // A progressive-charge list: the finisher detaches and frees it
    // (record of k missing → step 6) and clears the group.
    let p = pgsv_list(&mut f, u, 122, 99);
    // Replay of steps 5–8 by hand on a copy.
    let mut g = f.clone();
    let mut want = DamageRecord {
        result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), 17, 0),
        ..DamageRecord::default()
    };
    want.hit_flags |= 2;
    helpers::charges_before(&mut g, &t, u, &mut want);
    fill(&mut g.c, &t, &ct, u, m, &mut want, false, 128);
    helpers::charges_after(&mut g, &t, u, &mut want);
    start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 128);
    let stored = g.c.units[u].combat[0].record;
    assert_eq!(stored, want);
    apply_melee(&mut g.c, &ct, u, m);
    assert!(life_of(&g, m) < 100_000, "the hit lands");
    f.take_log();
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(life_of(&f, m), life_of(&g, m), "r5–r6, r8: same damage");
    assert_eq!(f.c.units[u].seed, g.c.units[u].seed);
    assert!(
        f.c.units[u].combat.is_empty(),
        "apply_melee frees the record"
    );
    // The finisher ran: flag 0x40, list freed, group cleared.
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert!(f.lists[p].freed);
    assert!(f.take_log().contains(&format!("cleargroup {u} 4")));
    // Wolf / bear (group 38): the melee is applied at once, with no
    // finisher.
    let (mut f, u, m) = duel();
    f.c.set(m, 6, 100_000);
    let p = pgsv_list(&mut f, u, 122, 99);
    f.state_flags.insert((139, group::MELEEONLY));
    f.c.units[u].states.push(139);
    f.take_log();
    assert_eq!(dos::attack(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(life_of(&f, m) < 100_000, "applied at once");
    assert!(!f.lists[p].freed);
    assert!(!f.take_log().iter().any(|l| l.starts_with("cleargroup")));
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
}

/// Skills record for the melee-with-state do: overlay 9, target
/// state 30 (length calc 50, aurastat1 25 = 4), self state 31.
fn melee_state_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvoverlay = 9;
    r.auratargetstate = 30;
    r.aurastate = 31;
    r.auralencalc = c.f(50);
    r.aurastat1 = 25;
    r.aurastatcalc1 = c.f(4);
    tabs(r, c, 1)
}

#[test]
fn melee_state_overlay_and_dispatch() {
    let t = melee_state_tables();
    let ct = ct3();
    let (mut f, u, m) = duel();
    f.c.set(m, 6, 100_000);
    // Slot 2 is this body (`functions.tsv`).
    assert_eq!(
        run_do(&mut f, &t, &ct, 2, u, 99, 1),
        Some(0),
        "r1: R invalid"
    );
    // No target → 0, flag not set.
    f.targets.remove(&u);
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    // No pair record → return 1 and the rest is skipped (no flag 0x40,
    // no state on the unit).
    f.targets.insert(u, m);
    assert_eq!(run_do(&mut f, &t, &ct, 2, u, 1, 1), Some(1));
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    assert!(!f.has_state(u, 31));
    // A pair record that is a miss: no overlay, but the rest goes on.
    hit_entry(&mut f, u, m, 0);
    f.take_log();
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(!f.take_log().iter().any(|l| l.starts_with("overlay")));
    assert!(f.has_state(u, 31));
    // A hit: overlay 9 on the target.
    f.c.units[u].combat.clear();
    hit_entry(&mut f, u, m, 1);
    f.take_log();
    dos::melee_state(&mut f, &t, &ct, u, 1, 1);
    assert!(f.take_log().contains(&format!("overlay {m} 9")));
    // The overlay counts 1…overlay count inclusive.
    for (ov, shown) in [(0u16, false), (200, true), (201, false)] {
        let mut t2 = t.clone();
        t2.skills[1].srvoverlay = ov;
        t2.skills[1].auratargetstate = 0xFFFF;
        f.c.units[u].combat.clear();
        hit_entry(&mut f, u, m, 1);
        f.take_log();
        dos::melee_state(&mut f, &t2, &ct, u, 1, 1);
        let log = f.take_log();
        assert_eq!(
            log.contains(&format!("overlay {m} {ov}")),
            shown,
            "overlay {ov}"
        );
    }
    // No target with an overlay in range → 0 (also for the target state).
    f.targets.remove(&u);
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn melee_state_target_state_list() {
    let mut t = melee_state_tables();
    t.skills[1].srvoverlay = 0;
    t.skills[1].aurastate = 0xFFFF;
    let ct = ct3();
    let (mut f, u, m) = duel();
    f.c.frame = 100;
    // T none → 0.
    f.targets.remove(&u);
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(u, m);
    // p none → return 1, nothing done.
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.lists.is_empty());
    // p a miss: nothing for the target state.
    hit_entry(&mut f, u, m, 0);
    dos::melee_state(&mut f, &t, &ct, u, 1, 1);
    assert!(f.lists.is_empty());
    // p a hit: a new list (flags 2, expire F + len, owner T), timer 12,
    // state on, callback default, aura_fill formulas on T.
    f.c.units[u].combat.clear();
    hit_entry(&mut f, u, m, 1);
    f.take_log();
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(m, 30).expect("target list").clone();
    assert_eq!((l.flags, l.expire, l.owner), (2, 150, Some(m)));
    assert_eq!(
        (l.callback, l.stats.get(&25)),
        (callback::DEFAULT, Some(&4))
    );
    assert!(f.has_state(m, 30));
    assert!(f.take_log().contains(&format!("timer {m} 12 150")));
    // An existing list keeps its old expiry; a length below 1 counts as 1.
    f.c.frame = 120;
    f.c.units[u].combat.clear();
    hit_entry(&mut f, u, m, 1);
    dos::melee_state(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.list_of(m, 30).unwrap().expire, 150);
    assert_eq!(f.lists.iter().filter(|l| l.state == 30).count(), 1);
    let mut c = Code::new();
    t.skills[1].auralencalc = c.f(-7);
    t.skills_code = c.0.clone();
    t.skills[1].aurastat1 = 0xFFFF;
    let (mut f, u, m) = duel();
    f.c.frame = 100;
    hit_entry(&mut f, u, m, 1);
    dos::melee_state(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.list_of(m, 30).unwrap().expire, 101);
}

#[test]
fn melee_state_self_state_and_the_melee() {
    let mut t = melee_state_tables();
    t.skills[1].srvoverlay = 0;
    t.skills[1].auratargetstate = 0xFFFF;
    let ct = ct3();
    let (mut f, u, m) = duel();
    f.c.set(m, 6, 100_000);
    hit_entry(&mut f, u, m, 1);
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 1);
    // A self list: flags 4, expire 0, callback default, aura_fill on the
    // unit, state on; reused by a second run.
    let l = f.list_of(u, 31).expect("self list");
    assert_eq!((l.flags, l.expire, l.callback), (4, 0, callback::DEFAULT));
    assert_eq!((l.owner, l.stats.get(&25)), (Some(u), Some(&4)));
    assert!(f.has_state(u, 31));
    // Flag 0x40 and apply_melee (the hit pair record is consumed).
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert!(f.c.units[u].combat.is_empty());
    hit_entry(&mut f, u, m, 1);
    dos::melee_state(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.lists.iter().filter(|l| l.state == 31).count(), 1);
    // aurastate outside 1…count − 1: no list (0, count, none).
    for st in [0u16, 200, 0xFFFF] {
        let mut t2 = t.clone();
        t2.skills[1].aurastate = st;
        let (mut f, u, m) = duel();
        assert_eq!(dos::melee_state(&mut f, &t2, &ct, u, 1, 1), 1);
        assert!(f.lists.is_empty(), "aurastate {st}");
        let _ = m;
    }
    // T none → 0 after the self state and the flag.
    let (mut f, u, _) = duel();
    f.targets.remove(&u);
    assert_eq!(dos::melee_state(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.has_state(u, 31));
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
}

#[test]
fn buff_registers_up_to_three_aura_events() {
    let mut r = body_rec();
    r.aurastate = 20;
    r.auraevent1 = 5;
    r.auraeventfunc1 = 3;
    r.auraevent2 = 6;
    r.auraeventfunc2 = 4;
    r.auraevent3 = 0xFFFF;
    r.auraeventfunc3 = 9;
    let t = tabs(r, Code::new(), 1);
    let ct = ct3();
    let (mut f, u) = world();
    f.take_log();
    assert_eq!(dos::buff(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    let at = |s: &str| log.iter().position(|l| l == s);
    // Unregister (1, state) first, then the pairs in order; the third
    // (event −1) ends the loop.
    let (un, h1, h2) = (
        at(&format!("unhandle {u} 1 20")).expect("unregister"),
        at(&format!("handler {u} 5 3 20")).expect("first"),
        at(&format!("handler {u} 6 4 20")).expect("second"),
    );
    assert!(un < h1 && h1 < h2);
    assert_eq!(log.iter().filter(|l| l.starts_with("handler")).count(), 2);
    assert!(log.contains(&format!("changed {u} 20")));
    // auraevent1 < 0: nothing is unregistered or registered, even when
    // events 2 / 3 are set.
    let mut t2 = t.clone();
    t2.skills[1].auraevent1 = 0xFFFF;
    dos::buff(&mut f, &t2, &ct, u, 1, 1);
    let log = f.take_log();
    assert!(!log
        .iter()
        .any(|l| l.starts_with("handler") || l.starts_with("unhandle {u} 1")));
    // A function outside 1…31 registers nothing.
    let mut t3 = t.clone();
    t3.skills[1].auraeventfunc1 = 40;
    t3.skills[1].auraeventfunc2 = 0;
    dos::buff(&mut f, &t3, &ct, u, 1, 1);
    assert!(!f.take_log().iter().any(|l| l.starts_with("handler")));
}

/// A curse skill (record 1): state `st`, aurastat1 = 25 (calc 5), length
/// 40, range 100, filter 3 (players and monsters).
fn curse_tables(st: u16) -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.auratargetstate = st;
    r.aurastat1 = 25;
    r.aurastatcalc1 = c.f(5);
    r.auralencalc = c.f(40);
    r.aurarangecalc = c.f(100);
    r.aurafilter = 3;
    // The other five slots: calc formulas for the tests to enable.
    for x in [
        &mut r.aurastatcalc2,
        &mut r.aurastatcalc3,
        &mut r.aurastatcalc4,
        &mut r.aurastatcalc5,
        &mut r.aurastatcalc6,
    ] {
        *x = c.f(7);
    }
    tabs(r, c, 1)
}

fn curse_ct() -> crate::combat::CombatTables {
    let mut ct = combat_tables(vec![monster_rec(), monster_rec()]);
    ct.monstats2[0].isatt = true;
    ct.monstats[0].switchai = true;
    ct
}

/// The caster (unit 0) aiming at (10, 10) and one monster there with the
/// flags 0xE, alive and hostile.
fn curse_world() -> (BodyFake, usize, usize) {
    let (mut f, u) = world();
    f.tpos.insert(u, (10, 10));
    let mut m = FUnit::new(UnitType::Monster, 0);
    m.type_flags = Some(0);
    m.flags = 0xE;
    let m = f.add(m, (10, 10));
    f.scan = vec![m];
    f.c.hostile = true;
    (f, u, m)
}

#[test]
fn curse_unit_conditions_and_state_list() {
    let (t, ct) = (curse_tables(30), curse_ct());
    let (mut f, u, m) = curse_world();
    f.c.frame = 100;
    // Slot 30 is the curse body.
    assert_eq!(run_do(&mut f, &t, &ct, 30, u, 1, 1), Some(1));
    let l = f.list_of(m, 30).expect("curse list").clone();
    assert_eq!(
        (l.expire, l.callback, l.skill, l.lvl),
        (140, callback::DEFAULT, 1, 1)
    );
    assert_eq!(l.stats.get(&25), Some(&5), "stat1 = value1");
    assert!(f.has_state(m, 30));
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // Flags 0x4, 0x8 and 0x2 must all be set; alive; hostile; a
    // monster needs a walk mode and no type flag 0x20.
    let reject = |mut f: BodyFake, why: &str| {
        let u = 0;
        assert_eq!(dos::curse(&mut f, &t, &ct, u, 1, 1), 1, "{why}");
        assert!(f.lists.is_empty(), "{why}");
    };
    for bit in [0x4u32, 0x8, 0x2] {
        let (mut f, _, m) = curse_world();
        f.c.units[m].flags = 0xE & !bit;
        reject(f, &format!("flags without {bit:#x}"));
    }
    let (mut f, _, m) = curse_world();
    f.alive.remove(&m);
    reject(f, "dead");
    let (mut f, _, _) = curse_world();
    f.c.hostile = false;
    reject(f, "not hostile");
    let (mut f, _, m) = curse_world();
    f.c.units[m].no_walk = true;
    reject(f, "no walk mode");
    let (mut f, _, m) = curse_world();
    f.c.units[m].type_flags = Some(0x20);
    reject(f, "possessed");
    // A player target needs neither the walk mode nor the type flag.
    let (mut f, u, _) = curse_world();
    let mut p = FUnit::new(UnitType::Player, 1);
    p.flags = 0xE;
    p.no_walk = true;
    p.type_flags = Some(0x20);
    let p = f.add(p, (10, 10));
    f.scan = vec![p];
    assert_eq!(dos::curse(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(p, 30).is_some());
    // A Monster that cannot be afflicted (apply_state refuses an npc)
    // gets no list and the scan still returns 1.
    let mut ct = ct;
    ct.monstats[1].npc = true;
    let (mut f, u, m) = curse_world();
    f.c.units[m].class = 1;
    assert_eq!(dos::curse(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.lists.is_empty());
}

#[test]
fn curse_unit_scales_and_sets_stats() {
    let ct = curse_ct();
    // Stat1 a resistance, value ≤ 0, base ≥ 100 on a non-player →
    // value / 5; v1 = 0 skips the unit; no aurastat1 goes on with 0.
    let mut c = Code::new();
    let mut r = body_rec();
    r.auratargetstate = 30;
    r.aurastat1 = 39;
    r.aurastatcalc1 = c.f(-50);
    r.auralencalc = c.f(40);
    r.aurarangecalc = c.f(100);
    r.aurafilter = 3;
    r.aurastat2 = 26;
    r.aurastatcalc2 = c.f(7);
    r.aurastat3 = 27;
    r.aurastatcalc3 = c.f(0);
    r.aurastat4 = 28;
    r.aurastatcalc4 = c.f(-100);
    r.aurastat5 = 0xFFFF;
    r.aurastatcalc5 = c.f(9);
    r.aurastat6 = 29;
    r.aurastatcalc6 = c.f(9);
    let t = tabs(r, c, 1);
    let (mut f, u, m) = curse_world();
    f.c.set(m, 39, 120);
    dos::curse(&mut f, &t, &ct, u, 1, 1);
    let l = f.list_of(m, 30).expect("list");
    assert_eq!(l.stats.get(&39), Some(&-10), "−50 / 5");
    // Stats 2…6: valid, non-zero values set; a zero value is not
    // set; an invalid stat (−1) ends the list, so slot 6 is never read.
    assert_eq!(l.stats.get(&26), Some(&7));
    assert!(!l.stats.contains_key(&27));
    assert_eq!(l.stats.get(&28), Some(&-100));
    assert!(!l.stats.contains_key(&29));
    // The same value with a base resistance below 100 is unchanged.
    let (mut f, u, m) = curse_world();
    f.c.set(m, 39, 99);
    dos::curse(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.list_of(m, 30).unwrap().stats.get(&39), Some(&-50));
    // A zero v1 skips the unit.
    let mut t0 = t.clone();
    let mut c = Code(t0.skills_code.clone());
    t0.skills[1].aurastatcalc1 = c.f(0);
    t0.skills_code = c.0;
    let (mut f, u, _) = curse_world();
    dos::curse(&mut f, &t0, &ct, u, 1, 1);
    assert!(f.lists.is_empty());
    // aurastat1 = −1 (none) goes on with v1 = 0: the state is applied
    // with no stat and slots 2…6 are not read (the stop is at slot 1).
    let mut tn = t.clone();
    tn.skills[1].aurastat1 = 0xFFFF;
    let (mut f, u, m) = curse_world();
    dos::curse(&mut f, &tn, &ct, u, 1, 1);
    assert!(f.list_of(m, 30).expect("applied").stats.is_empty());
    // A stat with `updateanimrate` refreshes the target's animation.
    let (mut f, u, m) = curse_world();
    f.stat_infos.insert(
        26,
        BodyStat {
            updateanimrate: true,
            maxstat: -1,
            ..BodyStat::default()
        },
    );
    f.take_log();
    dos::curse(&mut f, &t, &ct, u, 1, 1);
    assert!(f.take_log().contains(&format!("anim {m}")));
    let (mut f, u, m) = curse_world();
    f.take_log();
    dos::curse(&mut f, &t, &ct, u, 1, 1);
    assert!(!f.take_log().contains(&format!("anim {m}")));
}

#[test]
fn curse_unit_registers_the_first_three_event_pairs() {
    let ct = curse_ct();
    let mut t = curse_tables(30);
    let s = &mut t.skills[1];
    s.auraevent1 = 5;
    s.auraeventfunc1 = 3;
    s.auraevent2 = 6;
    s.auraeventfunc2 = 0;
    s.auraevent3 = 7;
    s.auraeventfunc3 = 4;
    let (mut f, u, m) = curse_world();
    f.take_log();
    dos::curse(&mut f, &t, &ct, u, 1, 1);
    let log = f.take_log();
    let at = |s: String| log.iter().position(|l| *l == s);
    let un = at(format!("unhandle {m} 1 30")).expect("unregister");
    let a = at(format!("handler {m} 5 3 30")).expect("pair 1");
    let c = at(format!("handler {m} 7 4 30")).expect("pair 3");
    assert!(un < a && a < c);
    // Pair 2 has function 0: refused by register. Three attempts → two
    // handlers.
    assert_eq!(log.iter().filter(|l| l.starts_with("handler")).count(), 2);
    // Event 1 < 0, or function 1 = 0: nothing at all (even for pairs 2
    // and 3).
    for (e1, f1) in [(0xFFFFu16, 3u16), (5, 0)] {
        let mut t2 = t.clone();
        t2.skills[1].auraevent1 = e1;
        t2.skills[1].auraeventfunc1 = f1;
        let (mut f, u, m) = curse_world();
        f.take_log();
        dos::curse(&mut f, &t2, &ct, u, 1, 1);
        let log = f.take_log();
        assert!(!log.iter().any(|l| l.starts_with("handler")));
        assert!(!log.contains(&format!("unhandle {m} 1 30")));
    }
}

#[test]
fn curse_unit_ai_curses_switch_the_monster_ai() {
    let mut ct = curse_ct();
    ct.difficultylevels[0].aicursedivisor = 4;
    for (st, k) in [(23u16, 10), (56, 11)] {
        let t = curse_tables(st);
        let (mut f, u, m) = curse_world();
        f.c.frame = 50;
        f.take_log();
        assert_eq!(dos::curse(&mut f, &t, &ct, u, 1, 1), 1);
        let l = f.list_of(m, i32::from(st)).expect("ai curse list");
        // The AI curse callback; the AI control switched to k.
        assert_eq!(l.callback, callback::AI_CURSE);
        assert!(f.take_log().contains(&format!("ai {m} {k}")));
        // Duration: 40 / AiCurseDivisor.
        assert_eq!(f.list_of(m, i32::from(st)).unwrap().expire, 60);
        // A non-AI curse state keeps the full length and sets no AI.
        let t = curse_tables(30);
        let (mut f, u, m) = curse_world();
        f.c.frame = 50;
        f.take_log();
        dos::curse(&mut f, &t, &ct, u, 1, 1);
        assert_eq!(f.list_of(m, 30).unwrap().expire, 90);
        assert!(!f.take_log().iter().any(|l| l.starts_with("ai ")));
    }
    // The unit must be a monster of alignment ≠ 1 that can switch.
    let t = curse_tables(23);
    let none = |f: BodyFake, why: &str| {
        let mut f = f;
        assert_eq!(dos::curse(&mut f, &t, &ct, 0, 1, 1), 1);
        assert!(f.lists.is_empty(), "{why}");
    };
    let (mut f, u, _) = curse_world();
    let mut p = FUnit::new(UnitType::Player, 1);
    p.flags = 0xE;
    let p = f.add(p, (10, 10));
    f.scan = vec![p];
    let _ = u;
    none(f, "player");
    let (mut f, _, m) = curse_world();
    f.c.units[m].align = 1;
    none(f, "alignment 1");
    let (mut f, _, m) = curse_world();
    f.c.units[m].type_flags = Some(2);
    none(f, "superunique");
    let (mut f, _, m) = curse_world();
    f.c.units[m].type_flags = Some(8);
    none(f, "unique");
    let (mut f, _, m) = curse_world();
    f.c.units[m].class = 1;
    none(f, "class without switchai (and npc)");
    let (mut f, _, m) = curse_world();
    f.c.units[m].states.push(54);
    none(f, "uninterruptable");
}

#[test]
fn can_switch_follows_the_original_conditions() {
    let mut ct = curse_ct();
    let (mut f, _, m) = curse_world();
    for k in [10, 11, 12, 19, 0, 5] {
        assert!(dos::can_switch(&mut f, &ct, m, k), "k = {k}");
    }
    assert!(!dos::can_switch(&mut f, &ct, m, 20), "k > 19");
    // Base class 492 with state 143 (attached) → 0.
    ct.monstats[0].baseid = 492;
    assert!(dos::can_switch(&mut f, &ct, m, 19));
    f.c.units[m].states.push(143);
    assert!(!dos::can_switch(&mut f, &ct, m, 19));
    f.c.units[m].states.clear();
    ct.monstats[0].baseid = 0;
    // State 54, no switchai, boss, no walk mode.
    f.c.units[m].states.push(54);
    assert!(!dos::can_switch(&mut f, &ct, m, 19));
    f.c.units[m].states.clear();
    ct.monstats[0].switchai = false;
    assert!(!dos::can_switch(&mut f, &ct, m, 19));
    ct.monstats[0].switchai = true;
    ct.monstats[0].boss = true;
    assert!(!dos::can_switch(&mut f, &ct, m, 19));
    ct.monstats[0].boss = false;
    f.c.units[m].no_walk = true;
    assert!(!dos::can_switch(&mut f, &ct, m, 19));
    f.c.units[m].no_walk = false;
    // Unit flag 0x4 unset and alive → 0; dead or flag set → ok.
    f.c.units[m].flags = 0;
    f.alive.insert(m);
    assert!(!dos::can_switch(&mut f, &ct, m, 19));
    f.alive.remove(&m);
    assert!(dos::can_switch(&mut f, &ct, m, 19));
    f.alive.insert(m);
    f.c.units[m].flags = 4;
    assert!(dos::can_switch(&mut f, &ct, m, 19));
    // Superunique (2) / unique (8) type flags → 0, for every k.
    for fl in [2u32, 8] {
        f.c.units[m].type_flags = Some(fl);
        for k in [10, 19, 0] {
            assert!(!dos::can_switch(&mut f, &ct, m, k), "flag {fl} k {k}");
        }
    }
    // A non-monster never switches.
    let p = f.add(FUnit::new(UnitType::Player, 0), (1, 1));
    assert!(!dos::can_switch(&mut f, &ct, p, 19));
}

// ---------------------------------------------------------------

#[test]
fn srvmissile_path_of_the_do_core() {
    use crate::skills::use_::tests::{caster, tables, F, U};
    use crate::skills::use_::{do_core, MissileAim};
    let t = tables(
        4,
        &[
            (1, &|r| r.srvmissile = 0),
            (2, &|r| {
                r.srvmissile = 0;
                r.lob = true;
            }),
            (3, &|r| r.srvmissile = 5),
        ],
    );
    // Srvmissile < 0 or without a missiles record: skipped (the
    // result is the do function's, here none → 0).
    for skill in [0, 3] {
        let mut f = F::new();
        let p = caster(&mut f, skill, 1, 0);
        assert_eq!(do_core(&mut f, &t, p, skill, 1, false, false, false), 0);
        assert!(f.units[p].flags & 0x40 == 0 && f.take_log().is_empty());
    }
    // Flags |= 0x40, the straight / lob creation, result 1
    // whatever the creation gave (the seam returns nothing).
    for (skill, lob) in [(1, false), (2, true)] {
        let mut f = F::new();
        let p = caster(&mut f, skill, 1, 0);
        assert_eq!(do_core(&mut f, &t, p, skill, 1, false, false, false), 1);
        assert_eq!(f.units[p].flags & 0x40, 0x40);
        assert_eq!(
            f.take_log(),
            [format!("missile {p} {skill} 1 0 {lob} None")]
        );
    }
    // Item and aim: the target position gives the offset and the aim
    // point (2 × target − unit); no item or no aim → position 0.
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 0);
    let mut tg = U::new(UnitType::Monster, 0);
    tg.pos = (140, 160);
    let tg = f.add(tg);
    f.units[p].target = Some(tg);
    f.units[p].pos = (100, 100);
    for (item, aim, at) in [
        (
            true,
            true,
            MissileAim::At {
                offset: (40, 60),
                aim: (180, 220),
            },
        ),
        (true, false, MissileAim::None),
        (false, true, MissileAim::None),
    ] {
        f.take_log();
        assert_eq!(do_core(&mut f, &t, p, 1, 1, false, item, aim), 1);
        assert_eq!(f.take_log(), [format!("missile {p} 1 1 0 false {at:?}")]);
    }
    // A target position with a 0 coordinate fails.
    f.units[tg].pos = (0, 160);
    f.take_log();
    do_core(&mut f, &t, p, 1, 1, false, true, true);
    assert_eq!(f.take_log(), [format!("missile {p} 1 1 0 false None")]);
}

// ---------------------------------------------------------------

/// Tables over `recs` (record 0 blank), the code buffer and one missile.
fn tabs_n(recs: Vec<d2_data::tables::Skills>, code: Code) -> crate::skills::SkillTables {
    let mut t = crate::skills::fake::skill_tables(recs);
    t.skills_code = code.0;
    t
}

#[test]
fn node_insert_guards() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    f.node.insert(m, 11);
    f.take_log();
    helpers2::node_insert(&mut f, m, 3);
    assert_eq!(f.take_log(), [format!("NodeInsert {{ m: {m}, slot: 3 }}")]);
    // slot ≥ 8 or negative, m already in a list, m not a player or
    // monster: nothing.
    helpers2::node_insert(&mut f, m, 8);
    helpers2::node_insert(&mut f, m, -1);
    f.node.insert(m, 2);
    helpers2::node_insert(&mut f, m, 3);
    let o = f.add(FUnit::new(UnitType::Object, 0), (1, 1));
    f.node.insert(o, 11);
    helpers2::node_insert(&mut f, o, 3);
    assert!(f.take_log().is_empty());
    // The player is allowed too; the owner's node index gives the slot.
    f.node.insert(u, 5);
    f.node.insert(m, 11);
    helpers2::node_insert_owner(&mut f, m, u);
    assert_eq!(f.take_log(), [format!("NodeInsert {{ m: {m}, slot: 5 }}")]);
    f.node.insert(u, 11);
    f.node.insert(m, 11);
    helpers2::node_insert_owner(&mut f, m, u);
    assert!(f.take_log().is_empty(), "the owner in no list: slot 11");
}

#[test]
fn prog_missile_picks_the_column_by_the_charge_count() {
    let mut r = body_rec();
    r.progressive = true;
    r.aurastate = 40;
    r.aurastat1 = 25;
    r.srvmissilea = 10;
    r.srvmissileb = 11;
    r.srvmissilec = 12;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u) = world();
    assert_eq!(helpers2::prog_missile(&mut f, &t, u, 99), -1, "R invalid");
    assert_eq!(helpers2::prog_missile(&mut f, &t, u, 1), 10, "no list");
    let l = f.alloc_list(0, 0, None).unwrap();
    f.set_list_state(l, 40);
    f.attach(u, l);
    for (n, want) in [(0, 10), (1, 10), (2, 11), (3, 12), (7, 12)] {
        f.list_set(l, 25, n);
        assert_eq!(helpers2::prog_missile(&mut f, &t, u, 1), want, "n = {n}");
    }
    // Not `progressive`, or an invalid aurastate / aurastat1: srvmissilea.
    f.list_set(l, 25, 3);
    for edit in [0, 1, 2] {
        let mut t2 = t.clone();
        match edit {
            0 => t2.skills[1].progressive = false,
            1 => t2.skills[1].aurastate = 0xFFFF,
            _ => t2.skills[1].aurastat1 = 0xFFFF,
        }
        assert_eq!(helpers2::prog_missile(&mut f, &t2, u, 1), 10, "edit {edit}");
    }
}

/// The skill record of the summon stats test: record 1 (the summon skill) and
/// record 2 (an aura skill the summon learns).
fn summon_skill_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.passivestat1 = 30;
    r.passivecalc1 = c.f(4);
    r.passivestat2 = 0xFFFF;
    r.passivecalc2 = c.f(5);
    r.passivestat3 = 31;
    r.passivecalc3 = c.f(6);
    r.aurastat1 = 26;
    r.aurastatcalc1 = c.f(8);
    r.aurastat2 = 27;
    r.aurastatcalc2 = c.f(0);
    r.aurastat3 = 28;
    r.aurastatcalc3 = c.f(2);
    r.aurastate = 40;
    r.calc1 = c.f(50);
    r.sumskill1 = 2;
    r.sumsk1calc = c.f(3);
    r.sumskill2 = 3;
    r.sumsk2calc = c.f(0);
    r.sumskill3 = 0;
    r.sumsk3calc = c.f(5);
    r.sumskill4 = 9;
    r.sumsk4calc = c.f(5);
    r.sumskill5 = 3;
    r.sumsk5calc = c.f(-1);
    r.auraevent1 = 5;
    r.auraeventfunc1 = 3;
    r.sumumod = 7;
    r.sumoverlay = 9;
    let mut aura = body_rec();
    aura.aura = true;
    let plain = body_rec();
    tabs_n(vec![body_rec(), r, aura, plain], c)
}

fn summoner() -> (BodyFake, usize, usize) {
    let (mut f, u) = world();
    f.c.set(u, 12, 30);
    let m = monster(&mut f, (5, 5));
    f.c.set(m, 7, 200);
    (f, u, m)
}

#[test]
fn skill_stats_passive_and_aura_stats_with_stat_events() {
    let t = summon_skill_tables();
    let (mut f, u, m) = summoner();
    let key = f.c.units[m].guid as i32;
    // Stat 30 carries two item events, stat 28 none.
    f.stat_infos.insert(
        30,
        BodyStat {
            itemevent: [5, 6],
            itemeventfunc: [3, 4],
            maxstat: -1,
            ..BodyStat::default()
        },
    );
    // Skill 0 → 0 and nothing happens; an invalid skill is refused.
    f.take_log();
    assert_eq!(helpers2::skill_stats(&mut f, &t, u, m, 0, 1, 0), 0);
    assert_eq!(helpers2::skill_stats(&mut f, &t, u, m, 99, 1, 0), 0);
    assert!(f.take_log().is_empty());
    assert_eq!(helpers2::skill_stats(&mut f, &t, u, m, 1, 4, 0), 1);
    // Passive stats added to m (invalid slot skipped).
    assert_eq!(f.c.get(m, 30), 4);
    assert_eq!(f.c.get(m, 31), 6);
    // …and the item events of stat 30 registered with `s << 16`, type 2,
    // key m's GUID, once.
    let hs = &f.handlers[&m];
    let ev: Vec<_> = hs
        .iter()
        .filter(|h| h.skill == 30 << 16)
        .map(|h| (h.event, h.func, h.key_type, h.key, h.level))
        .collect();
    assert!(ev.contains(&(5, 3, 2, key, 0)) && ev.contains(&(6, 4, 2, key, 0)));
    // The lookup is by `s`, the registration by `s << 16` (Edge case 10):
    // a second call registers the pair again.
    let n = hs.len();
    helpers2::skill_stats(&mut f, &t, u, m, 1, 4, 0);
    assert_eq!(f.handlers[&m].len(), n + 2);
    // A handler with the skill field `s` (type 2, key m) suppresses the
    // registration for that stat.
    let (mut f2, u2, m2) = summoner();
    f2.stat_infos.insert(30, f.stat_infos[&30]);
    let key2 = f2.c.units[m2].guid as i32;
    f2.handlers.entry(m2).or_default().push(Handler {
        event: 9,
        key_type: 2,
        key: key2,
        skill: 30,
        level: 0,
        func: 1,
    });
    helpers2::skill_stats(&mut f2, &t, u2, m2, 1, 4, 0);
    assert!(f2.handlers[&m2].iter().all(|h| h.skill != 30 << 16));
    // Aura stats: v ≠ 0 into one list (flags 0, expire 0, owner m,
    // attached); the zero value is not set; its state id := aurastate.
    let l = f.list_of(m, 40).expect("list with the state id").clone();
    assert_eq!(
        (l.flags, l.expire, l.owner, l.unit),
        (0, 0, Some(m), Some(m))
    );
    assert_eq!((l.stats.get(&26), l.stats.get(&28)), (Some(&8), Some(&2)));
    assert!(!l.stats.contains_key(&27));
    assert!(f.has_state(m, 40));
}

#[test]
fn skill_stats_state_life_skills_events() {
    let t = summon_skill_tables();
    let (mut f, u, m) = summoner();
    f.take_log();
    helpers2::skill_stats(&mut f, &t, u, m, 1, 4, 0);
    let log = f.take_log();
    // Maxhp := h + pct(h, calc1 = 50, 100) = 300; hitpoints too.
    assert_eq!((f.c.get(m, 7), f.c.get(m, 6)), (300, 300));
    // Sumskill 1 (v 3) and the aura skill → right skill := k; skill 2
    // has aura (SelectSkill); k = 3 (v 0 / −1), k = 0 and k = 9 (≥ count)
    // are skipped.
    assert!(log.contains(&format!("SetSkill {{ m: {m}, skill: 2, lvl: 3 }}")));
    assert!(log.contains(&format!(
        "SelectSkill {{ u: {m}, side: 0, skill: 2, owner: -1 }}"
    )));
    assert_eq!(log.iter().filter(|l| l.starts_with("SetSkill")).count(), 1);
    // Auraevent1 ≥ 0: unregister (1, aurastate), then register the
    // events.
    assert!(log.contains(&format!("unhandle {m} 1 40")));
    assert!(log.contains(&format!("handler {m} 5 3 40")));
    // Aurastate in 1…count (the count itself accepted), else no state.
    for (st, on) in [(0u16, false), (200, true), (201, false)] {
        let mut t2 = t.clone();
        t2.skills[1].aurastate = st;
        let (mut f, u, m) = summoner();
        helpers2::skill_stats(&mut f, &t2, u, m, 1, 4, 0);
        assert_eq!(f.has_state(m, i32::from(st) as u16), on, "state {st}");
    }
    // A aurastate of 0xFFFF registers no events.
    let mut t3 = t.clone();
    t3.skills[1].auraevent1 = 0xFFFF;
    let (mut f, u, m) = summoner();
    helpers2::skill_stats(&mut f, &t3, u, m, 1, 4, 0);
    assert!(!f.take_log().iter().any(|l| l.starts_with("handler {m} 5")));
}

#[test]
fn skill_stats_umod_overlay_and_equipment_level() {
    let t = summon_skill_tables();
    let eq = |f: &mut BodyFake| -> Vec<String> {
        f.take_log()
            .into_iter()
            .filter(|l| l.starts_with("Equipment") || l.starts_with("Umod"))
            .collect()
    };
    // Sumumod 1…42 and overlay 1…count − 1.
    for (um, shown) in [(0u16, false), (1, true), (42, true), (43, false)] {
        let mut t2 = t.clone();
        t2.skills[1].sumumod = um;
        let (mut f, u, m) = summoner();
        helpers2::skill_stats(&mut f, &t2, u, m, 1, 4, 0);
        let log = eq(&mut f);
        assert_eq!(
            log.contains(&format!("Umod {{ m: {m}, umod: {um}, arg: 1 }}")),
            shown,
            "umod {um}"
        );
    }
    for (ov, shown) in [(0u16, false), (1, true), (199, true), (200, false)] {
        let mut t2 = t.clone();
        t2.skills[1].sumoverlay = ov;
        let (mut f, u, m) = summoner();
        helpers2::skill_stats(&mut f, &t2, u, m, 1, 4, 0);
        assert_eq!(
            f.take_log().contains(&format!("overlay {m} {ov}")),
            shown,
            "overlay {ov}"
        );
    }
    // Ilvl 0 → 3L, at least 1, at most the owner's level (30); a
    // non-zero ilvl is passed unchanged.
    for (lvl, ilvl, want) in [
        (4, 0, 12),
        (20, 0, 30),
        (10, 0, 30),
        (-2, 0, 1),
        (4, 99, 99),
    ] {
        let (mut f, u, m) = summoner();
        helpers2::skill_stats(&mut f, &t, u, m, 1, lvl, ilvl);
        let log = eq(&mut f);
        let want =
            format!("Equipment {{ owner: {u}, m: {m}, skill: 1, lvl: {lvl}, ilvl: {want} }}");
        assert!(log.contains(&want), "{lvl}/{ilvl}: {log:?}");
    }
}

#[test]
fn sentry_spawns_for_the_owning_player() {
    let ct = ct3();
    let mut r = body_rec();
    r.summon = 1;
    r.summode = 5;
    r.pettype = 3;
    r.intown = false;
    let mut c = Code::new();
    r.petmax = c.f(2);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.c.set(u, 12, 30);
    let log_since = |f: &mut BodyFake| f.take_log();
    // Summon class −1 → none.
    let mut t_bad = t.clone();
    t_bad.skills[1].summon = 0xFFFF;
    assert_eq!(
        helpers2::sentry(&mut f, &t_bad, &ct, u, (50, 60), 1, 4),
        None
    );
    // in the normal case: position given.
    f.take_log();
    let m = helpers2::sentry(&mut f, &t, &ct, u, (50, 60), 1, 4).expect("sentry");
    assert_eq!(f.pos[&m], (50, 60));
    assert_eq!(f.c.units[m].class, 1);
    assert_eq!(f.c.units[m].flags & 0x2_0000, 0x2_0000);
    let log = log_since(&mut f);
    // Pet type 3 (valid), petmax 2 → the pet list add.
    assert!(log.contains(&format!("PetAdd {{ owner: {u}, pet: {m}, t: 3, max: 2 }}")));
    // Base_stats: level 4 + 3·30/4 = 26; skill_stats ran (Equipment).
    assert_eq!(f.c.get(m, 12), 26);
    assert!(log.iter().any(|l| l.starts_with("Equipment")));
    // Alignment (2, 1) and the mode change to `mode`.
    assert!(log.contains(&format!("Alignment {{ u: {m}, a: 2, v: 1 }}")));
    assert!(log.contains(&format!("moderequest {m} 5 None")));
    // A pettype outside 0…count − 1 uses 0.
    let mut t2 = t.clone();
    t2.skills[1].pettype = 99;
    f.take_log();
    let m2 = helpers2::sentry(&mut f, &t2, &ct, u, (50, 60), 1, 4).unwrap();
    assert!(f
        .take_log()
        .contains(&format!("PetAdd {{ owner: {u}, pet: {m2}, t: 0, max: 2 }}")));
    // X or y = 0 → the unit's position;: still 0 → the owner's
    // target position; failure → none.
    f.pos.insert(u, (7, 8));
    let m3 = helpers2::sentry(&mut f, &t, &ct, u, (0, 9), 1, 4).unwrap();
    assert_eq!(f.pos[&m3], (7, 8));
    f.pos.insert(u, (0, 0));
    assert_eq!(helpers2::sentry(&mut f, &t, &ct, u, (0, 0), 1, 4), None);
    f.tpos.insert(u, (21, 22));
    let m4 = helpers2::sentry(&mut f, &t, &ct, u, (0, 0), 1, 4).unwrap();
    assert_eq!(f.pos[&m4], (21, 22));
    // A monster caster lays the trap for its minion owner; none → 0.
    let mon = monster(&mut f, (1, 1));
    assert_eq!(helpers2::sentry(&mut f, &t, &ct, mon, (50, 60), 1, 4), None);
    f.minion_owner.insert(mon, u);
    f.take_log();
    let m5 = helpers2::sentry(&mut f, &t, &ct, mon, (50, 60), 1, 4).unwrap();
    assert!(f
        .take_log()
        .contains(&format!("PetAdd {{ owner: {u}, pet: {m5}, t: 3, max: 2 }}")));
    // No room at the point → none; a town room without InTown →
    // none, with it → ok.
    f.point_rooms.insert((50, 61), None);
    assert_eq!(helpers2::sentry(&mut f, &t, &ct, u, (50, 61), 1, 4), None);
    f.town.insert(1);
    assert_eq!(helpers2::sentry(&mut f, &t, &ct, u, (50, 60), 1, 4), None);
    let mut t3 = t.clone();
    t3.skills[1].intown = true;
    assert!(helpers2::sentry(&mut f, &t3, &ct, u, (50, 60), 1, 4).is_some());
    // The spawn failing → none.
    f.town.clear();
    f.no_monsters = true;
    assert_eq!(helpers2::sentry(&mut f, &t, &ct, u, (50, 60), 1, 4), None);
}

#[test]
fn golem_stats_and_summon_resistance() {
    let t = summon_skill_tables();
    let (mut f, u, m) = summoner();
    // No passive_summon_resist: no list at all.
    f.take_log();
    helpers2::summon_resist(&mut f, u, m);
    assert!(f.lists.is_empty());
    f.c.set(u, 349, 35);
    helpers2::summon_resist(&mut f, u, m);
    let l = &f.lists[0];
    assert_eq!(
        (l.flags, l.expire, l.owner, l.unit),
        (0, 0, Some(m), Some(m))
    );
    assert_eq!(
        (
            l.stats.get(&39),
            l.stats.get(&41),
            l.stats.get(&43),
            l.stats.get(&45)
        ),
        (Some(&35), Some(&35), Some(&35), Some(&35))
    );
    // An absorb > 0 keeps its resistance out of the list; 45 always.
    f.lists.clear();
    f.c.set(m, 142, 5);
    f.c.set(m, 148, 1);
    helpers2::summon_resist(&mut f, u, m);
    let l = &f.lists[0];
    assert_eq!(
        (
            l.stats.get(&39),
            l.stats.get(&41),
            l.stats.get(&43),
            l.stats.get(&45)
        ),
        (None, Some(&35), None, Some(&35))
    );
    // golem_stats = base_stats, skill_stats, summon_resist in that order.
    let (mut f, u, m) = summoner();
    f.c.set(u, 349, 10);
    f.take_log();
    helpers2::golem_stats(&mut f, &t, u, m, 1, 4);
    assert_eq!(f.c.get(m, 12), 26, "base_stats");
    assert_eq!((f.c.get(m, 7), f.c.get(m, 6)), (300, 300), "skill_stats");
    assert!(
        f.lists.iter().any(|l| l.stats.get(&45) == Some(&10)),
        "resist"
    );
}

#[test]
fn shadow_stats_use_the_second_formula_on_the_shadow() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.param1 = 10;
    r.aurastat1 = 26;
    r.aurastatcalc1 = c.f(1);
    r.aurastat2 = 27;
    r.aurastatcalc2 = c.f(9);
    r.aurastat3 = 0xFFFF;
    r.aurastat4 = 28;
    r.passivestat1 = 30;
    r.passivecalc1 = c.f(2);
    r.passivecalc2 = c.f(4);
    r.passivestat2 = 0xFFFF;
    r.sumumod = 5;
    let t = tabs(r, c, 1);
    // L ≤ 1 → nothing.
    let (mut f, _, m) = summoner();
    f.take_log();
    helpers2::shadow_stats(&mut f, &t, m, 1, 1);
    helpers2::shadow_stats(&mut f, &t, m, 1, 0);
    assert!(f.lists.is_empty() && f.take_log().is_empty());
    // Maxhp := h + pct(h, (L − 1)·Param1, 100): 200 + 80 = 280.
    helpers2::shadow_stats(&mut f, &t, m, 1, 5);
    assert_eq!((f.c.get(m, 7), f.c.get(m, 6)), (280, 280));
    // One list on m (flags 0, expire 0), every valid stat set from
    // the second formula (aurastatcalc2 = 9, passivecalc2 = 4).
    let l = &f.lists[0];
    assert_eq!(
        (l.flags, l.expire, l.owner, l.unit),
        (0, 0, Some(m), Some(m))
    );
    assert_eq!((l.stats.get(&26), l.stats.get(&27)), (Some(&9), Some(&9)));
    assert_eq!(l.stats.get(&28), Some(&9), "slot 4 after an invalid slot 3");
    assert_eq!(l.stats.get(&30), Some(&4));
    // Sumumod in 1…42.
    assert!(f
        .take_log()
        .contains(&format!("Umod {{ m: {m}, umod: 5, arg: 1 }}")));
}

// ---------------------------------------------------------------

#[test]
fn power_strike_start_record_and_combat() {
    use crate::combat::{melee_result, start_combat};
    let ct = ct3();
    for (etype, c4, src) in [(3u8, 30i16, 64u8), (3, -5, 0), (0, 30, 0)] {
        let mut c = Code::new();
        let mut r = body_rec();
        r.calc1 = c.f(40);
        r.calc4 = c.f(c4);
        r.etype = etype;
        r.srcdam = src;
        let t = tabs(r, c, 1);
        let (mut f, u, m) = duel();
        // No target / invalid skill → 0.
        assert_eq!(starts2::power_strike(&mut f, &t, &ct, u, 99, 1), 0);
        f.targets.remove(&u);
        assert_eq!(starts2::power_strike(&mut f, &t, &ct, u, 1, 1), 0);
        f.targets.insert(u, m);
        assert!(f.c.units[u].combat.is_empty());
        // replayed by hand.
        let mut g = f.clone();
        let mut want = DamageRecord {
            result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), 0, 0),
            ..DamageRecord::default()
        };
        assert_eq!(want.result & 1, 1);
        want.enh_pct = 40;
        if etype != 0 {
            want.conv_pct = i32::from(c4);
            if c4 > 0 {
                want.conv_elem = etype as i8;
            }
        }
        crate::skills::roll_elemental(&mut g, &t, u, &mut want, 1, 1);
        start_combat(
            &mut g.c,
            &t,
            &ct,
            Some(u),
            Some(m),
            &mut want,
            i32::from(src),
        );
        assert_eq!(starts2::power_strike(&mut f, &t, &ct, u, 1, 1), 1);
        let e = last_entry(&mut f, u);
        assert_eq!(e.record, want, "etype {etype} calc4 {c4}");
        assert_eq!(e.record.enh_pct, 40);
        assert_eq!(f.c.units[u].seed, g.c.units[u].seed);
        // A miss stores the bare record.
        f.c.hostile = false;
        f.c.units[u].combat.clear();
        assert_eq!(starts2::power_strike(&mut f, &t, &ct, u, 1, 1), 1);
        let e = last_entry(&mut f, u);
        assert_eq!(
            (e.record.result, e.record.enh_pct, e.record.conv_pct),
            (0, 0, 0)
        );
    }
}

#[test]
fn corpse_explosion_start_needs_a_corpse_in_the_field() {
    let (mut f, u, m, mut ct) = corpse_world();
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 1);
    // No Velocity test, unlike Raise.
    ct.monstats[0].velocity = 0;
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 1);
    assert_eq!(starts::raise(&mut f, &ct, u), 0);
    ct.monstats[0].velocity = 1;
    f.town.insert(1);
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 0, "town");
    f.town.clear();
    f.c.units[m].mode = 1;
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 0, "not dead");
    f.c.units[m].mode = 12;
    f.state_flags.insert((77, group::UDEAD));
    f.c.units[m].states.push(77);
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 0, "udead");
    f.c.units[m].states.clear();
    ct.monstats2[0].corpsesel = false;
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 0, "no corpseSel");
    ct.monstats2[0].corpsesel = true;
    let p = f.add(FUnit::new(UnitType::Player, 1), (4, 4));
    f.targets.insert(u, p);
    assert_eq!(
        starts2::corpse_explosion(&mut f, &ct, u),
        0,
        "not a monster"
    );
    f.targets.remove(&u);
    assert_eq!(starts2::corpse_explosion(&mut f, &ct, u), 0);
}

#[test]
fn feral_rage_start_records_the_hit_in_param_one() {
    use crate::combat::{melee_result, start_combat};
    let ct = ct3();
    let (t, _) = strike_tables(0, 2, 0);
    let (mut f, u, m) = duel();
    // Invalid skill, no used entry, no target → 0.
    assert_eq!(starts2::feral_rage(&mut f, &t, &ct, u, 99, 1), 0);
    let used = f.c.units[u].used.take();
    assert_eq!(starts2::feral_rage(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.units[u].used = used;
    f.targets.remove(&u);
    assert_eq!(starts2::feral_rage(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(u, m);
    assert!(f.entries.is_empty() && f.c.units[u].combat.is_empty());
    // replayed by hand: conversion (EType 3, calc4 30), enhanced
    // damage 40, SrcDam 0 → 128, param 1 := 1 on a hit.
    let mut g = f.clone();
    let bonus = crate::skills::to_hit(&mut g, &t, Some(u), 1, 1);
    let mut want = DamageRecord {
        result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), bonus, 0),
        ..DamageRecord::default()
    };
    want.conv_pct = 30;
    want.conv_elem = 3;
    want.enh_pct = 40;
    start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 128);
    assert_eq!(starts2::feral_rage(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(last_entry(&mut f, u).record, want);
    assert_eq!(f.entries[&(u, 1, 1)], 1);
    // No roll_elemental: the seed is untouched by the body beyond the
    // replay (which made no such draw either).
    assert_eq!(f.c.units[u].seed, g.c.units[u].seed);
    // A miss: param 1 := 0 and the record has no conversion or damage
    // percent.
    f.c.hostile = false;
    f.c.units[u].combat.clear();
    assert_eq!(starts2::feral_rage(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entries[&(u, 1, 1)], 0);
    let e = last_entry(&mut f, u);
    assert_eq!(
        (e.record.enh_pct, e.record.conv_pct, e.record.result),
        (0, 0, 0)
    );
}

// ---------------------------------------------------------------

/// A summon skill (record 1): class 1, mode 5, pet type 3, pet max 2,
/// `calc2` = `c2`, a state 40 of length 100 and a missile row.
fn summon_tabs(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 1;
    r.summode = 5;
    r.pettype = 3;
    r.petmax = c.f(2);
    r.calc2 = c.f(20);
    edit(&mut r, &mut c);
    tabs(r, c, 1)
}

/// The player (unit 0, level 30, node slot 5) aiming at (30, 40).
fn druid() -> (BodyFake, usize) {
    let (mut f, u) = world();
    f.c.set(u, 12, 30);
    f.node.insert(u, 5);
    f.tpos.insert(u, (30, 40));
    (f, u)
}

fn has(log: &[String], s: String) -> bool {
    log.contains(&s)
}

/// The monster the last summon made (the highest unit id).
fn newest(f: &BodyFake) -> usize {
    f.c.units.len() - 1
}

#[test]
fn druid_summon_steps() {
    let ct = ct3();
    let t = summon_tabs(|_, _| {});
    // R invalid.: no summon class.: pet type outside the count
    // (signed byte: 0xFF is −1).
    let (mut f, u) = druid();
    assert_eq!(dos2::druid_summon(&mut f, &t, &ct, u, 99, 1), 0);
    let bad = |e: &dyn Fn(&mut d2_data::tables::Skills)| {
        let mut t2 = t.clone();
        e(&mut t2.skills[1]);
        t2
    };
    let (mut f2, u2) = druid();
    let t2 = bad(&|r| r.summon = 0xFFFF);
    assert_eq!(dos2::druid_summon(&mut f2, &t2, &ct, u2, 1, 1), 0);
    for pet in [15u8, 0xFF] {
        let (mut f3, u3) = druid();
        let t3 = bad(&|r| r.pettype = pet);
        assert_eq!(
            dos2::druid_summon(&mut f3, &t3, &ct, u3, 1, 1),
            0,
            "pet {pet}"
        );
        assert_eq!(f3.c.units[u3].flags & 0x40, 0, "r4 comes after r3");
    }
    // The flag is set; the target position failing → 0.
    let (mut f4, u4) = druid();
    f4.tpos.remove(&u4);
    assert_eq!(dos2::druid_summon(&mut f4, &t, &ct, u4, 1, 1), 0);
    assert_eq!(f4.c.units[u4].flags & 0x40, 0x40);
    // The spawn failing → 0.
    f.no_monsters = true;
    assert_eq!(dos2::druid_summon(&mut f, &t, &ct, u, 1, 1), 0);
    f.no_monsters = false;
    // A pet of class 1 at the owner's target point (flags 0: no
    // position given), pet type 3 / max 2, then the node insert, base
    // stats with p = max(calc2, 1) and the skill stats.
    f.take_log();
    assert_eq!(dos2::druid_summon(&mut f, &t, &ct, u, 1, 3), 1);
    let m = newest(&f);
    assert_eq!((f.c.units[m].class, f.pos[&m]), (1, (30, 40)));
    let log = f.take_log();
    let at = |s: String| {
        log.iter()
            .position(|l| *l == s)
            .unwrap_or_else(|| panic!("{s}"))
    };
    let pet = at(format!("PetAdd {{ owner: {u}, pet: {m}, t: 3, max: 2 }}"));
    let node = at(format!("NodeInsert {{ m: {m}, slot: 5 }}"));
    let eq = log.iter().position(|l| l.starts_with("Equipment")).unwrap();
    assert!(pet < node && node < eq);
    assert_eq!(f.c.get(m, 12), 20, "calc2 = 20 is the base level p");
    // calc2 ≤ 0 → p = 1.
    let mut c = Code::new();
    let t0 = {
        let mut r = t.skills[1].clone();
        r.calc2 = c.f(-4);
        r.petmax = c.f(2);
        tabs(r, c, 1)
    };
    let (mut f, u) = druid();
    dos2::druid_summon(&mut f, &t0, &ct, u, 1, 3);
    assert_eq!(f.c.get(newest(&f), 12), 1);
}

#[test]
fn sentry_do_flag_target_point_and_result() {
    let ct = ct3();
    let t = summon_tabs(|r, _| r.intown = true);
    let (mut f, u) = druid();
    assert_eq!(dos2::sentry_do(&mut f, &t, &ct, u, 99, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0, "r1 before r2");
    // No target position → 0 (flag set).
    f.tpos.remove(&u);
    assert_eq!(dos2::sentry_do(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // 1 when the sentry made a unit, at the target point.
    f.tpos.insert(u, (30, 40));
    assert_eq!(dos2::sentry_do(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.pos[&newest(&f)], (30, 40));
    f.no_monsters = true;
    assert_eq!(dos2::sentry_do(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn nova_attack_ring_velocity() {
    let mut t = summon_tabs(|r, c| {
        r.calc1 = c.f(5);
        r.srvmissilea = 0;
    });
    t.missiles[0].vel = 10;
    t.missiles[0].vellev = 3;
    let (mut f, u) = druid();
    // The flag is set first;: R invalid → 0.
    assert_eq!(dos2::nova(&mut f, &t, u, 99, 9), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // No missile (srvmissilea = none) → 0.
    let mut t2 = t.clone();
    t2.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(dos2::nova(&mut f, &t2, u, 1, 9), 0);
    assert!(f.missiles.is_empty());
    // V = Vel + VelLev·L / 8 + calc1 = 10 + 27/8 + 5 = 18; the
    // ring of 64 missiles carries flag 4 and the velocity.
    assert_eq!(dos2::nova(&mut f, &t, u, 1, 9), 1);
    assert_eq!(f.missiles.len(), 64);
    assert!(f.missiles.iter().all(|m| m.flags == 7 && m.velocity == 18));
}

#[test]
fn vines_summon_in_mode_8_with_the_vine_state() {
    let ct = ct3();
    let t = summon_tabs(|_, _| {});
    let (mut f, u) = druid();
    assert_eq!(dos2::vines(&mut f, &t, &ct, u, 99, 1), 0);
    let mut t2 = t.clone();
    t2.skills[1].summon = 0xFFFF;
    assert_eq!(dos2::vines(&mut f, &t2, &ct, u, 1, 1), 0);
    let mut t3 = t.clone();
    t3.skills[1].pettype = 15;
    assert_eq!(dos2::vines(&mut f, &t3, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0, "r4 after r2 and r3");
    f.no_monsters = true;
    assert_eq!(dos2::vines(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40, "r4");
    f.no_monsters = false;
    f.take_log();
    assert_eq!(dos2::vines(&mut f, &t, &ct, u, 1, 1), 1);
    let m = newest(&f);
    let log = f.take_log();
    // The mode is 8 whatever `summode` says (5 here).
    assert!(
        has(&log, "monster 1 (30, 40) 1 8 -1".to_string()),
        "{log:?}"
    );
    // Node insert;: state 150 on m.
    assert!(has(&log, format!("NodeInsert {{ m: {m}, slot: 5 }}")));
    assert!(f.has_state(m, 150));
    // Level := max(calc2, 1) with no monlvl bonuses;: skill stats.
    assert_eq!(f.c.get(m, 12), 20);
    assert!(log.iter().any(|l| l.starts_with("Equipment")));
    assert_eq!(f.c.get(m, 31), 0, "no base_stats AC");
    let mut c = Code::new();
    let mut r = t.skills[1].clone();
    r.calc2 = c.f(0);
    r.petmax = c.f(2);
    let t0 = tabs(r, c, 1);
    let (mut f, u) = druid();
    dos2::vines(&mut f, &t0, &ct, u, 1, 1);
    assert_eq!(f.c.get(newest(&f), 12), 1);
}

#[test]
fn golem_summon_steps() {
    let ct = ct3();
    let t = summon_tabs(|_, _| {});
    // Skill 0 → 0, after the flag.
    let (mut f, u) = druid();
    assert_eq!(dos2::golem(&mut f, &t, &ct, u, 0, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // An out-of-range skill is refused too.
    assert_eq!(dos2::golem(&mut f, &t, &ct, u, 99, 1), 0);
    // Class outside 0…count − 1; no summon_class fallback.
    f.spawn_class = Some(1);
    let mut t2 = t.clone();
    t2.skills[1].summon = 2;
    assert_eq!(dos2::golem(&mut f, &t2, &ct, u, 1, 1), 0);
    t2.skills[1].summon = 0xFFFF;
    assert_eq!(dos2::golem(&mut f, &t2, &ct, u, 1, 1), 0);
    // The spawn failing → 0.
    f.no_monsters = true;
    assert_eq!(dos2::golem(&mut f, &t, &ct, u, 1, 1), 0);
    f.no_monsters = false;
    f.c.set(u, 349, 25);
    f.take_log();
    assert_eq!(dos2::golem(&mut f, &t, &ct, u, 1, 4), 1);
    let m = newest(&f);
    let log = f.take_log();
    let at = |s: &str| {
        log.iter()
            .position(|l| l.starts_with(s))
            .unwrap_or(usize::MAX)
    };
    assert!(has(
        &log,
        format!("PetAdd {{ owner: {u}, pet: {m}, t: 3, max: 2 }}")
    ));
    // Golem_stats (base level 4 + 22 = 26, the skill stats, the
    // summon resistance list): the ally message: the node insert.
    assert_eq!(f.c.get(m, 12), 26);
    assert!(f.lists.iter().any(|l| l.stats.get(&45) == Some(&25)));
    assert!(has(&log, format!("AllyInfo {{ u: {u}, m: {m} }}")));
    assert!(at("Equipment") < at("AllyInfo") && at("AllyInfo") < at("NodeInsert"));
    assert!(has(&log, format!("NodeInsert {{ m: {m}, slot: 5 }}")));
    // Pet type unsigned: ≥ count → 0 (signed −1 would also be ≥); a
    // mode ≥ 16 → 1.
    let mut t3 = t.clone();
    t3.skills[1].pettype = 0xFF;
    t3.skills[1].summode = 16;
    f.take_log();
    dos2::golem(&mut f, &t3, &ct, u, 1, 4);
    let log = f.take_log();
    let m = newest(&f);
    assert!(has(
        &log,
        format!("PetAdd {{ owner: {u}, pet: {m}, t: 0, max: 2 }}")
    ));
    assert!(
        has(&log, "monster 1 (30, 40) 1 1 -1".to_string()),
        "{log:?}"
    );
}

/// A formula giving the skill level the evaluation runs at (parameter 16).
fn level_formula(c: &mut Code) -> u32 {
    let at = c.0.len() as u32;
    c.0.extend([0x04, 16, 0x00]);
    at
}

#[test]
fn inferno_cast_checks_the_missile_then_runs_inferno_do() {
    let ct = ct3();
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(5);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (5, 5));
    assert_eq!(
        dos2::inferno_cast(&mut f, &t, &ct, u, 99, 1),
        0,
        "R invalid"
    );
    let mut t2 = t.clone();
    t2.skills[1].srvmissilea = 3;
    assert_eq!(
        dos2::inferno_cast(&mut f, &t2, &ct, u, 1, 1),
        0,
        "m out of range"
    );
    t2.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(dos2::inferno_cast(&mut f, &t2, &ct, u, 1, 1), 0, "m = −1");
    assert!(f.missiles.is_empty());
    // The first call after a fresh start (param 1 = 0) creates nothing,
    // the next one a missile of class 0 with range max(calc1, 1).
    assert_eq!(dos2::inferno_cast(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.missiles.is_empty());
    assert_eq!(dos2::inferno_cast(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.missiles[0].class, f.missiles[0].range), (0, 5));
}

fn blaze_tabs(edit: impl Fn(&mut d2_data::tables::Skills)) -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 40;
    r.aurastat1 = 26;
    r.aurastatcalc1 = c.f(3);
    r.auralencalc = c.f(100);
    r.passivestat1 = 30;
    r.passivecalc1 = c.f(7);
    r.auraevent1 = 5;
    r.auraeventfunc1 = 3;
    edit(&mut r);
    tabs(r, c, 1)
}

#[test]
fn blaze_buff_without_flag_or_group_removal() {
    let ct = ct3();
    let t = blaze_tabs(|_| {});
    let (mut f, u) = world();
    // R invalid, aurastat1 outside −1…count − 1, aurastate invalid.
    assert_eq!(dos2::blaze(&mut f, &t, &ct, u, 99, 1), 0);
    for (a1, st) in [(0xFFFEu16, 40u16), (359, 40), (26, 0xFFFF), (26, 200)] {
        let mut t2 = t.clone();
        t2.skills[1].aurastat1 = a1;
        t2.skills[1].aurastate = st;
        assert_eq!(dos2::blaze(&mut f, &t2, &ct, u, 1, 1), 0, "{a1} {st}");
    }
    assert!(f.lists.is_empty());
    // aurastat1 = −1 is valid; no same-group removal; no 0x40.
    let mut t3 = t.clone();
    t3.skills[1].aurastat1 = 0xFFFF;
    f.state_groups.insert(40, 9);
    f.state_groups.insert(41, 9);
    f.c.units[u].states.push(41);
    f.c.frame = 10;
    f.take_log();
    assert_eq!(dos2::blaze(&mut f, &t3, &ct, u, 1, 3), 1);
    let l = f.list_of(u, 40).expect("list").clone();
    assert_eq!(
        (l.flags & 2, l.expire, l.callback),
        (2, 110, callback::DEFAULT)
    );
    // passive_fill, 350 / 351; the aura stat is not filled.
    assert_eq!(l.stats.get(&30), Some(&7));
    assert_eq!((l.stats.get(&350), l.stats.get(&351)), (Some(&1), Some(&3)));
    assert!(!l.stats.contains_key(&26));
    assert!(f.has_state(u, 41), "no group removal");
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    // Unregister (1, state) then the events.
    let log = f.take_log();
    assert!(has(&log, format!("unhandle {u} 1 40")));
    assert!(has(&log, format!("handler {u} 5 3 40")));
    // apply_state refusing (a curse state with full resistance) → 0.
    let (mut f, u) = world();
    f.state_flags.insert((40, group::CURSE));
    f.c.set(u, 109, 100);
    assert_eq!(dos2::blaze(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn feral_rage_do_charges_up_to_calc2() {
    let ct = ct3();
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 40;
    r.auralencalc = c.f(50);
    r.calc2 = c.f(2);
    r.aurastat1 = 26;
    r.aurastatcalc1 = level_formula(&mut c);
    let t = tabs(r, c, 1);
    let (mut f, u, m) = duel();
    // R invalid, aurastate invalid, no used entry / another skill.
    assert_eq!(dos2::feral_rage(&mut f, &t, &ct, u, 99, 1), 0);
    let mut t2 = t.clone();
    t2.skills[1].aurastate = 0xFFFF;
    assert_eq!(dos2::feral_rage(&mut f, &t2, &ct, u, 1, 1), 0);
    f.c.units[u].used = Some(SkillEntry {
        skill: 0,
        ..f.c.units[u].used.unwrap()
    });
    assert_eq!(dos2::feral_rage(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0, "r2 comes after r1");
    let skill1 = SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    f.c.units[u].used = Some(skill1);
    // The flag, the melee on the target, the start missed → 1.
    f.c.set(m, 6, 100_000);
    hit_entry(&mut f, u, m, 1);
    assert_eq!(dos2::feral_rage(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert!(f.c.units[u].combat.is_empty(), "apply_melee ran");
    assert!(f.lists.is_empty(), "param 1 = 0: nothing else");
    // with the start hit: the list (flags 2, expire F + 50), the
    // timer, the state change, the count n = min(calc2 = 2, count + 1)
    // and aura_fill at level n.
    f.entries.insert((u, 1, 1), 1);
    f.c.frame = 7;
    f.targets.remove(&u);
    for (run, n) in [(1, 1), (2, 2), (3, 2)] {
        f.take_log();
        assert_eq!(dos2::feral_rage(&mut f, &t, &ct, u, 1, 5), 1, "run {run}");
        let l = f.list_of(u, 40).expect("list").clone();
        assert_eq!((l.flags & 2, l.expire), (2, 57));
        assert_eq!(l.callback, callback::DEFAULT);
        assert_eq!(l.stats.get(&169), Some(&n), "run {run}");
        assert_eq!((l.stats.get(&350), l.stats.get(&351)), (Some(&1), Some(&5)));
        assert_eq!(l.stats.get(&26), Some(&n), "the level is the charge count");
        let log = f.take_log();
        assert!(has(&log, format!("timer {u} 12 57")));
        assert!(has(&log, format!("changed {u} 40")));
    }
    assert_eq!(f.lists.iter().filter(|l| l.state == 40).count(), 1);
}

#[test]
fn shadow_warrior_steps() {
    let ct = ct3();
    let mk = |edit: &dyn Fn(&mut d2_data::tables::Skills, &mut Code)| {
        let mut c = Code::new();
        let mut r = body_rec();
        r.summon = 1;
        r.summode = 5;
        r.pettype = 3;
        r.petmax = c.f(2);
        r.aurastate = 40;
        r.auralencalc = c.f(60);
        r.param1 = 10;
        r.param5 = 3;
        r.param6 = 2;
        edit(&mut r, &mut c);
        tabs(r, c, 1)
    };
    let t = mk(&|_, _| {});
    let (mut f, u) = druid();
    // The flag is set even for an invalid skill.
    assert_eq!(dos2::shadow(&mut f, &t, &ct, u, 99, 4), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // No class.: pet type ≥ count (unsigned).: spawn fails.
    let none = mk(&|r, _| r.summon = 0xFFFF);
    assert_eq!(dos2::shadow(&mut f, &none, &ct, u, 1, 4), 0);
    let pet = mk(&|r, _| r.pettype = 0xFF);
    assert_eq!(dos2::shadow(&mut f, &pet, &ct, u, 1, 4), 0);
    f.no_monsters = true;
    assert_eq!(dos2::shadow(&mut f, &t, &ct, u, 1, 4), 0);
    f.no_monsters = false;
    f.c.frame = 100;
    f.c.set(u, 12, 30);
    f.take_log();
    assert_eq!(dos2::shadow(&mut f, &t, &ct, u, 1, 4), 1);
    let m = newest(&f);
    let log = f.take_log();
    assert!(
        has(&log, "monster 1 (30, 40) 1 5 -1".to_string()),
        "{log:?}"
    );
    // M's level := the owner's.: shadow stats (Param1 = 10:
    // 200 → no maxhp in the fake, so check the list instead).
    assert_eq!(f.c.get(m, 12), 30);
    // Ilvl = Param5 + (L − 1)·Param6 = 3 + 6 = 9, a single
    // Equipment (no skill_stats).
    let eq: Vec<_> = log.iter().filter(|l| l.starts_with("Equipment")).collect();
    assert_eq!(
        eq,
        [&format!(
            "Equipment {{ owner: {u}, m: {m}, skill: 1, lvl: 4, ilvl: 9 }}"
        )]
    );
    // Type-2 timers deleted, then a type-2 timer at F + 20.
    let at = |s: String| {
        log.iter()
            .position(|l| *l == s)
            .unwrap_or_else(|| panic!("{s}"))
    };
    assert!(at(format!("deltimers {m} 2 0")) < at(format!("schedule {m} 2 120 0 0")));
    // Aurastate 40 on m.: the source link.: d = 60 → a type-7
    // timer at F + 60 and umod 21 (argument 0).: the node insert.
    assert!(f.has_state(m, 40));
    assert!(has(
        &log,
        format!("SourceFields {{ m: {m}, owner: Some({u}) }}")
    ));
    assert!(has(&log, format!("schedule {m} 7 160 0 0")));
    assert!(has(&log, format!("Umod {{ m: {m}, umod: 21, arg: 0 }}")));
    assert!(has(&log, format!("NodeInsert {{ m: {m}, slot: 5 }}")));
    // No base_stats: no armor class from the (absent) monlvl rows and the
    // level is the owner's, not the formula.
    assert_eq!(f.c.get(m, 31), 0);
    // ilvl bounds: L ≤ 0 gives 0 → 1; above the level cap → the cap.
    for (edit_l, lvl, want) in [(3u32, 0, 1), (500, 4, 99)] {
        let t2 = mk(&|r, _| r.param5 = edit_l);
        let (mut f, u) = druid();
        dos2::shadow(&mut f, &t2, &ct, u, 1, lvl);
        let m = newest(&f);
        assert!(
            f.take_log().contains(&format!(
                "Equipment {{ owner: {u}, m: {m}, skill: 1, lvl: {lvl}, ilvl: {want} }}"
            )),
            "lvl {lvl}"
        );
    }
    // aurastate outside 1…count − 1 sets no state; a length ≤ 0 sets no
    // timer / umod 21.
    let t3 = mk(&|r, c| {
        r.aurastate = 0;
        r.auralencalc = c.f(0);
    });
    let (mut f, u) = druid();
    dos2::shadow(&mut f, &t3, &ct, u, 1, 4);
    let m = newest(&f);
    let log = f.take_log();
    assert!(!f.has_state(m, 0));
    assert!(!log
        .iter()
        .any(|l| l.starts_with(&format!("schedule {m} 7")) || l.contains("umod: 21")));
}

// ---------------------------------------------------------------- and edge cases

#[test]
fn shape_start_adds_each_shape_skills_to_hit_at_the_attack_level() {
    use crate::combat::{melee_result, start_combat};
    let ct = ct3();
    let mut a = body_rec();
    a.tohit = 10;
    a.levtohit = 5;
    a.srcdam = 64;
    let mut b = body_rec();
    b.tohit = 4;
    let t = tabs_n(vec![body_rec(), body_rec(), a, b], Code::new());
    let (mut f, u, m) = duel();
    // T none → 0.
    f.targets.remove(&u);
    assert_eq!(helpers::shape_start(&mut f, &t, &ct, u, 3), 0);
    f.targets.insert(u, m);
    assert!(f.c.units[u].combat.is_empty());
    // Two disguise states (flag group 16) with lists naming skills 2 and
    // 3; a third, non-disguise state is ignored.
    for (s, k) in [(140, 2), (141, 3), (142, 2)] {
        if s != 142 {
            f.state_flags.insert((s, group::DISGUISE));
        }
        f.c.units[u].states.push(s as u16);
        let l = f.alloc_list(0, 0, None).unwrap();
        f.set_list_state(l, s);
        f.attach(u, l);
        f.list_set(l, 350, k);
    }
    // Skill 2 at the Attack level 3 is 10 + 2·5; skill 3 is 4: the bonus
    // is their sum (not the shape skills' own levels); SrcDam 64 from
    // skill 2 (skill 3 has 0, which keeps it).
    assert_eq!(crate::skills::to_hit(&mut f, &t, Some(u), 2, 3), 20);
    assert_eq!(crate::skills::to_hit(&mut f, &t, Some(u), 3, 3), 4);
    let mut g = f.clone();
    let mut want = DamageRecord {
        result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), 24, 0),
        hit_class: 1,
        ..DamageRecord::default()
    };
    start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 64);
    assert_eq!(helpers::shape_start(&mut f, &t, &ct, u, 3), 1);
    assert_eq!(last_entry(&mut f, u).record, want);
    assert_eq!(f.c.units[u].seed, g.c.units[u].seed);
    // Without a disguise state: bonus 0, SrcDam 128.
    let (mut f, u, m) = duel();
    let mut g = f.clone();
    let mut want = DamageRecord {
        result: melee_result(&mut g.c, &t, &ct, Some(u), Some(m), 0, 0),
        hit_class: 1,
        ..DamageRecord::default()
    };
    start_combat(&mut g.c, &t, &ct, Some(u), Some(m), &mut want, 128);
    assert_eq!(helpers::shape_start(&mut f, &t, &ct, u, 3), 1);
    assert_eq!(last_entry(&mut f, u).record, want);
}

#[test]
fn aura_below_cost_refreshes_lists_without_stats() {
    let ct = ct3();
    let mut c = Code::new();
    let mut r = body_rec();
    r.mana = 2;
    r.manashift = 8;
    r.aurastate = 40;
    r.aurastat1 = 26;
    r.aurastatcalc1 = c.f(8);
    r.passivestat1 = 30;
    r.passivecalc1 = c.f(4);
    let t = tabs(r, c, 1);
    assert_eq!(crate::skills::mana_cost(&t.skills[1], 1), 512);
    for (mana, aura, passive) in [(511, false, false), (512, true, false), (513, true, true)] {
        let (mut f, u) = world();
        f.c.set(u, 8, mana);
        assert_eq!(dos::aura(&mut f, &t, &ct, u, 1, 1), 1);
        // The state list exists whatever the mana.
        let l = f
            .list_of(u, 40)
            .unwrap_or_else(|| panic!("mana {mana}: no list"));
        assert_eq!(l.stats.get(&26) == Some(&8), aura, "mana {mana}");
        assert_eq!(l.stats.get(&30) == Some(&4), passive, "mana {mana}");
    }
}

#[test]
fn apply_state_refuses_lower_levels_and_replaces_with_higher() {
    let ct = ct2();
    let (mut f, u) = world();
    let q = |level| StateRequest {
        source: u,
        target: u,
        skill: 5,
        level,
        duration: 0,
        stat: -1,
        value: 0,
        state: 3,
        callback: 0,
    };
    let first = apply_state(&mut f, &ct, q(3)).expect("list");
    // Same skill, lower level: refused; same level: the list is kept.
    assert!(apply_state(&mut f, &ct, q(2)).is_none());
    assert_eq!(apply_state(&mut f, &ct, q(3)), Some(first));
    // A higher level frees the old list and makes a new one.
    let second = apply_state(&mut f, &ct, q(4)).expect("replacement");
    assert_ne!(second, first);
    assert!(f.lists[first].freed);
    assert_eq!(f.lists[second].lvl, 4);
}

#[test]
fn damage_aura_never_counts_targets_and_needs_mana_for_target_values() {
    let ct = curse_ct();
    let mk = |c: &mut Code| {
        let mut r = body_rec();
        r.mana = 2;
        r.manashift = 8;
        r.aurastate = 40;
        r.auratargetstate = 41;
        r.aurastat1 = 26;
        r.aurastatcalc1 = c.f(8);
        r.aurarangecalc = c.f(100);
        r.aurafilter = 3;
        r
    };
    let mut c = Code::new();
    let r = mk(&mut c);
    let t = tabs(r, c, 1);
    // A player with enough mana: the target gets its list, state 85 ends
    // off (it was on) and no mana is paid.
    let (mut f, u, m) = curse_world();
    f.c.set(u, 8, 1000);
    f.c.units[u].states.push(85);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, false), 1);
    assert_eq!(f.list_of(m, 41).map(|l| l.stats.get(&26)), Some(Some(&8)));
    assert!(!f.has_state(u, 85));
    assert_eq!(f.c.get(u, 8), 1000);
    // A monster caster below the cost: the target values are 0, so no
    // target list is made — the mana test applies to monsters too.
    let (mut f, u, m) = curse_world();
    let c0 = f.add(FUnit::new(UnitType::Monster, 0), (10, 10));
    f.c.units[c0].flags = 0xE;
    f.scan = vec![m];
    f.c.hostile = true;
    f.c.set(c0, 8, 100);
    f.tpos.insert(c0, (10, 10));
    f.pos.insert(c0, (10, 10));
    let _ = u;
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, c0, 1, 1, false), 1);
    assert!(f.list_of(m, 41).is_none());
}

#[test]
fn summon_class_falls_back_to_the_spawn_class_and_refuses_otherwise() {
    let ct = ct3();
    let mut r = body_rec();
    r.summon = 0xFFFF;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u) = world();
    assert_eq!(helpers2::summon_class(&f, &t, &ct, u, 1).0, -1);
    f.spawn_class = Some(1);
    assert_eq!(helpers2::summon_class(&f, &t, &ct, u, 1).0, 1);
    // A spawn class outside 1…count − 1 is no class.
    f.spawn_class = Some(0);
    assert_eq!(helpers2::summon_class(&f, &t, &ct, u, 1).0, -1);
    f.spawn_class = Some(2);
    assert_eq!(helpers2::summon_class(&f, &t, &ct, u, 1).0, -1);
}

#[test]
fn charge_hit_leaves_flag_40_and_claws_set_or_clear_it() {
    let ct = ct3();
    let mut r = body_rec();
    r.aurastate = 40;
    r.aurastat1 = 25;
    let t = tabs(r, Code::new(), 1);
    for start in [0u32, 0x40] {
        let (mut f, u, _) = duel();
        f.c.units[u].flags = start;
        assert_eq!(dos2::charge_hit(&mut f, &t, &ct, u, 1, 1), 1);
        assert_eq!(f.c.units[u].flags & 0x40, start, "flag as it was");
    }
    // Claws: the flag is set unless two different claws are wielded and
    // the frame event index is even (then it is cleared).
    let (mut f, u, _) = duel();
    f.inventory = true;
    let a = f.c.add_item(FItem {
        types: vec![45],
        ..FItem::default()
    });
    let b = f.c.add_item(FItem {
        types: vec![45],
        ..FItem::default()
    });
    f.c.units[u].items.insert(4, a);
    f.c.units[u].items.insert(5, b);
    for (idx, flag) in [(1, 0x40), (2, 0), (3, 0x40)] {
        f.c.units[u].flags = 0x40;
        f.frame_index.insert(u, idx);
        assert_eq!(dos2::claws(&mut f, &t, &ct, u, 1, 1), 1);
        assert_eq!(f.c.units[u].flags & 0x40, flag, "index {idx}");
    }
    // One claw only, or a non-claw: always set.
    f.c.units[u].items.remove(&5);
    f.c.units[u].flags = 0;
    f.frame_index.insert(u, 2);
    dos2::claws(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
}

#[test]
fn multiple_shot_target_failure_returns_one_and_fend_returns_zero() {
    let ct = ct3();
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(3);
    r.calc2 = c.f(1);
    r.srvmissilea = 0;
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    // No target position: 1 and nothing made (the flag is set).
    assert_eq!(dos2::multiple_shot(&mut f, &t, u, 1, 1), 1);
    assert!(f.missiles.is_empty());
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    f.tpos.insert(u, (40, 7));
    assert_eq!(dos2::multiple_shot(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 3);
    // Fend returns 0 after a hit.
    let (mut f, u, m) = duel();
    f.entries.insert((u, 1, 1), 1);
    f.entries.insert((u, 1, 2), 0);
    f.entries.insert((u, 1, 3), f.c.units[m].guid as i32);
    assert_eq!(dos2::fend(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.c.units[u].combat.is_empty(), "the melee was applied");
}

#[test]
fn monster_inferno_always_ends_after_one_call() {
    let mut ct = ct3();
    ct.monstats2[0].infernolen = 9;
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(5);
    let t = tabs(r, c, 1);
    let (mut f, _) = world();
    let mut mon = FUnit::new(UnitType::Monster, 0);
    mon.skills.push(SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    });
    mon.used = mon.skills.first().copied();
    mon.states.push(12);
    let m = f.add(mon, (0, 0));
    f.tpos.insert(m, (5, 5));
    f.c.frame = 100;
    f.take_log();
    assert_eq!(dos2::inferno_cast(&mut f, &t, &ct, m, 1, 1), 1);
    let log = f.take_log();
    // Param 1 was set to 1 first, so F < param 1 never holds: state 12
    // off, type-0 timers deleted, ENDANIM (type 1) at F + InfernoLen.
    assert!(!f.has_state(m, 12));
    assert!(has(&log, format!("deltimers {m} 0 0")));
    assert!(has(&log, format!("schedule {m} 1 109 0 0")));
    assert!(!log
        .iter()
        .any(|l| l.starts_with(&format!("schedule {m} 0"))));
    assert_eq!(f.anim_frame[&m], 0xB00);
}

#[test]
fn raise_skeleton_penalty_hits_a_paladin_even_when_the_raise_fails() {
    let (mut f, u, _, ct) = corpse_world();
    f.c.units[u].class = 3;
    f.c.set(u, 6, 1000);
    f.c.set(u, 7, 800);
    // R invalid: the raise fails after the corpse test and the penalty.
    let t = tabs(body_rec(), Code::new(), 1);
    let mut g = f.clone();
    let mut want = DamageRecord {
        hit_flags: 0x1000,
        result: 4,
        physical: 100,
        total: 100,
        ..DamageRecord::default()
    };
    crate::combat::apply(&mut g.c, &ct, u, u, false, &mut want);
    let before = f.c.get(u, 6);
    assert_eq!(dos2::raise_skeleton(&mut f, &t, &ct, u, 99, 1), 0);
    assert!(f.c.get(u, 6) < before, "max life / 8 taken");
    assert_eq!(f.c.get(u, 6), g.c.get(u, 6));
    // Another class takes nothing.
    let (mut f2, u2, _, ct2_) = corpse_world();
    f2.c.set(u2, 6, 1000);
    f2.c.set(u2, 7, 800);
    assert_eq!(dos2::raise_skeleton(&mut f2, &t, &ct2_, u2, 99, 1), 0);
    assert_eq!(f2.c.get(u2, 6), 1000);
    // No corpse: no penalty either.
    let (mut f3, u3, m3, ct3_) = corpse_world();
    f3.c.units[u3].class = 3;
    f3.c.set(u3, 6, 1000);
    f3.c.set(u3, 7, 800);
    f3.c.units[m3].mode = 1;
    assert_eq!(dos2::raise_skeleton(&mut f3, &t, &ct3_, u3, 1, 1), 0);
    assert_eq!(f3.c.get(u3, 6), 1000);
}

#[test]
fn defensive_buff_is_slot_18() {
    let mut r = body_rec();
    r.aurastate = 20;
    let t = tabs(r, Code::new(), 1);
    let ct = ct3();
    let (mut f, u) = world();
    assert_eq!(run_do(&mut f, &t, &ct, 18, u, 1, 1), Some(1));
    assert!(f.has_state(u, 20) && f.c.units[u].flags & 0x40 == 0x40);
    assert_eq!(run_do(&mut f, &t, &ct, 18, u, 99, 1), Some(0));
}

#[test]
fn pet_resync_takes_the_largest_petmax_then_basemax() {
    let mut c = Code::new();
    let pet = |c: &mut Code, pt: u8, v: i16| {
        let mut r = body_rec();
        r.pettype = pt;
        r.petmax = c.f(v);
        r
    };
    let recs = vec![
        body_rec(),
        pet(&mut c, 2, 3),
        pet(&mut c, 2, 7),
        pet(&mut c, 4, -2),
        pet(&mut c, 0, 9),
        pet(&mut c, 9, 9),
        pet(&mut c, 2, 5),
    ];
    let t = tabs_n(recs, c);
    let (mut f, u) = world();
    f.pettypes = 5;
    f.c.units[u].skills = (1..=6)
        .map(|k| SkillEntry {
            skill: k,
            base: 1,
            owner_guid: -1,
            ..SkillEntry::default()
        })
        .collect();
    let rows = |t: i32| (t != 3).then_some(10 + t);
    let mut calls = Vec::new();
    helpers::pet_resync(&mut f, &t, u, &rows, &mut |_, t, v| calls.push((t, v)));
    // Skills in list order; a larger value raises the maximum (type
    // 2: 3, then 7; the later 5 does not); −2 counts as 1; type 0 and the
    // invalid type 9 are skipped:
    // types with no value get basemax when they have a row (0, 1;
    // type 3 has none).
    assert_eq!(calls, [(2, 3), (2, 7), (4, 1), (0, 10), (1, 11)]);
    // Not a player, or no skill list: nothing.
    let m = monster(&mut f, (1, 1));
    let mut calls = Vec::new();
    helpers::pet_resync(&mut f, &t, m, &rows, &mut |_, t, v| calls.push((t, v)));
    f.c.units[u].skills.clear();
    helpers::pet_resync(&mut f, &t, u, &rows, &mut |_, t, v| calls.push((t, v)));
    assert!(calls.is_empty());
}

#[test]
fn terror_install_puts_a_switch_capable_monster_in_terror() {
    let ct = curse_ct();
    let (mut f, u, m) = curse_world();
    f.c.frame = 50;
    dos::terror_install(&mut f, &ct, u, m, 25);
    let l = f.list_of(m, 56).expect("terror list").clone();
    assert_eq!((l.skill, l.lvl, l.expire, l.owner), (77, 1, 75, Some(u)));
    assert_eq!(f.targets.get(&m), Some(&u));
    let log = f.take_log();
    let at = |s: String| {
        log.iter()
            .position(|l| *l == s)
            .unwrap_or_else(|| panic!("{s}: {log:?}"))
    };
    assert!(at(format!("ai {m} 11")) < at(format!("deltimers {m} 2 0")));
    assert!(at(format!("deltimers {m} 2 0")) < at(format!("schedule {m} 2 51 0 0")));
}

#[test]
fn terror_install_skips_the_wrong_targets() {
    let none = |f: &mut BodyFake, ct: &crate::combat::CombatTables, u, m, why: &str| {
        dos::terror_install(f, ct, u, m, 25);
        assert!(f.list_of(m, 56).is_none(), "{why}");
        assert!(
            !f.take_log()
                .iter()
                .any(|l| l.starts_with(&format!("ai {m}"))),
            "{why}"
        );
    };
    let ct = curse_ct();
    let (mut f, u, m) = curse_world();
    f.alive.remove(&m);
    none(&mut f, &ct, u, m, "dead");
    let (mut f, u, m) = curse_world();
    f.c.units[m].states.push(54);
    none(&mut f, &ct, u, m, "uninterruptable");
    let (mut f, u, m) = curse_world();
    let mut ct2 = ct.clone();
    ct2.monstats[0].switchai = false;
    none(&mut f, &ct2, u, m, "no switchai");
    let mut ct2 = ct.clone();
    ct2.monstats[0].boss = true;
    none(&mut f, &ct2, u, m, "boss");
    f.c.units[m].no_walk = true;
    none(&mut f, &ct, u, m, "no walk mode");
    f.c.units[m].no_walk = false;
    for fl in [2u32, 8] {
        f.c.units[m].type_flags = Some(fl);
        none(&mut f, &ct, u, m, "unique / superunique");
    }
    let p = f.add(FUnit::new(UnitType::Player, 0), (1, 1));
    none(&mut f, &ct, u, p, "player");
}

#[test]
fn terror_install_has_no_attached_imp_test() {
    let mut ct = curse_ct();
    ct.monstats[0].baseid = 492;
    let (mut f, u, m) = curse_world();
    f.c.units[m].states.push(143);
    dos::terror_install(&mut f, &ct, u, m, 25);
    assert!(f.list_of(m, 56).is_some());
}
