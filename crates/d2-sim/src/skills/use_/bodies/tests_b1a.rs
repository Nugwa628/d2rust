//! Coverage tests of the batch 3 helpers and bodies (with
//! their edge cases) on [`super::fake::BodyFake`]. Values are computed
//! by hand from the rules.

use super::fake::{BodyFake, FList};
use super::tests2::{body_rec, monster, tabs, world, Code};
use super::*;
use crate::combat::{CombatTables, DamageRecord};
use crate::rng::Seed;
use crate::skills::fake::{combat_tables, monster_rec, FItem, FUnit};
use crate::skills::{SkillEntry, SkillTables, SkillUnits};
use crate::units::UnitType;

/// A live list of `state` on `u` with `stats`.
fn give_list(f: &mut BodyFake, u: usize, state: i32, stats: &[(i32, i32)]) -> usize {
    let mut l = FList {
        state,
        unit: Some(u),
        ..FList::default()
    };
    for &(k, v) in stats {
        l.stats.insert(k, v);
    }
    f.lists.push(l);
    f.lists.len() - 1
}

/// The seed after one `step`.
fn stepped(s: Seed) -> Seed {
    let mut s = s;
    s.step();
    s
}

/// A seed for the unit whose first draw `step % 100` falls in `want`.
fn seed_with(want: std::ops::Range<u32>) -> Seed {
    let lo = (0u32..)
        .find(|&lo| want.contains(&(Seed::new(lo, 0).step() % 100)))
        .unwrap();
    Seed::new(lo, 0)
}

/// A seed for which a melee hit of `u` on `m` is a plain hit.
fn hit_seed(f: &BodyFake, t: &SkillTables, ct: &CombatTables, u: usize, m: usize) -> Seed {
    (1u32..)
        .map(|lo| Seed::new(lo, 0))
        .find(|&s| {
            let mut g = f.clone();
            g.c.units[u].seed = s;
            crate::combat::melee_result(&mut g.c, t, ct, Some(u), Some(m), 100_000, 0) & 1 != 0
        })
        .unwrap()
}

fn hit_world() -> (BodyFake, usize, usize) {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(u, 12, 1);
    f.c.set(m, 12, 1);
    f.c.set(u, 19, 100_000);
    (f, u, m)
}

// -----------------------------------------------------------------

#[test]
fn minion_damage_reads_the_owners_skill() {
    let mut r = body_rec();
    r.mindam = 10;
    r.maxdam = 20;
    r.hitshift = 8;
    r.tohit = 100;
    r.levtohit = 10;
    let t = tabs(r, Code::new(), 1);
    let mut mr = monster_rec();
    mr.skilldamage = 1;
    let ct = combat_tables(vec![mr]);
    let (mut f, o) = world();
    let m = monster(&mut f, (5, 5));
    // No minion owner: S stays zero.
    assert_eq!(minion_damage(&mut f, &t, &ct, m), (0, 0, 0));
    f.minion_owner.insert(m, o);
    // Owner has no entry of the skill: level 0 (to-hit of level 0 is 0).
    f.c.units[o].skills.clear();
    assert_eq!(minion_damage(&mut f, &t, &ct, m), (10, 20, 0));
    // Base level 5, native entry: level_bonus is added (owner GUID −1).
    let e = SkillEntry {
        skill: 1,
        base: 5,
        owner_guid: -1,
        level_bonus: 3,
        ..SkillEntry::default()
    };
    f.c.units[o].skills = vec![e];
    assert_eq!(minion_damage(&mut f, &t, &ct, m), (10, 20, 100 + 7 * 10));
    // An item-granted entry (owner GUID ≠ −1) takes no bonus level.
    f.c.units[o].skills[0].owner_guid = 7;
    assert_eq!(minion_damage(&mut f, &t, &ct, m), (10, 20, 100 + 4 * 10));
    // The level is capped at the maximum character level.
    f.c.units[o].skills[0].owner_guid = -1;
    f.c.units[o].skills[0].base = 500;
    f.c.units[o].skills[0].level_bonus = 0;
    let cap = t.level_cap;
    assert_eq!(
        minion_damage(&mut f, &t, &ct, m).2,
        100 + (cap - 1) * 10,
        "capped"
    );
    // k out of range (0 or above the count) leaves S zero.
    let mut mr0 = monster_rec();
    mr0.skilldamage = 0;
    let ct0 = combat_tables(vec![mr0]);
    assert_eq!(minion_damage(&mut f, &t, &ct0, m), (0, 0, 0));
    let mut mr3 = monster_rec();
    mr3.skilldamage = 3; // count is 2
    let ct3 = combat_tables(vec![mr3]);
    assert_eq!(minion_damage(&mut f, &t, &ct3, m), (0, 0, 0));
    // The count itself is accepted (a skill without a record: min 1, max 2
    // >> 8 = 0).
    let mut mr2 = monster_rec();
    mr2.skilldamage = 2;
    let ct2 = combat_tables(vec![mr2]);
    assert_eq!(minion_damage(&mut f, &t, &ct2, m).0, 0);
    // A non-monster has none.
    assert_eq!(minion_damage(&mut f, &t, &ct, o), (0, 0, 0));
}

// -----------------------------------------------------------------

fn kick_world(mindam: u32, maxdam: u32) -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut r = body_rec();
    r.mindam = mindam;
    r.maxdam = maxdam;
    r.hitshift = 8;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    (f, t, ct, u, m)
}

#[test]
fn kick_damage_rolls_the_physical_part() {
    // Fixed damage 10: lo = hi, no draw. E = 50 %: 2560 + 1280 = 3840.
    let (mut f, t, ct, u, m) = kick_world(10, 10);
    let seed0 = f.c.units[u].seed;
    let mut rec = DamageRecord {
        enh_pct: 50,
        ..DamageRecord::default()
    };
    helpers3::kick_damage(&mut f, &t, &ct, u, &mut rec, m, 1, 1);
    assert_eq!(rec.physical, 3840);
    assert_eq!(rec.hit_flags & 3, 3, "hit flags |= 3");
    assert_eq!(f.c.units[u].seed, seed0, "d < 1: no draw");
    // Kick stats: KICKDAMAGE 4 adds to both, shifted by 8, with E2 = E.
    let (mut f, t, ct, u, m) = kick_world(10, 10);
    f.c.set(u, 137, 4);
    let mut rec = DamageRecord {
        enh_pct: 50,
        ..DamageRecord::default()
    };
    helpers3::kick_damage(&mut f, &t, &ct, u, &mut rec, m, 1, 1);
    let kick = (4 << 8) + (4 << 8) * 50 / 100;
    assert_eq!(rec.physical, 3840 + kick);
    // Min 10, max 20: d = 2560 > 0, one draw x = roll(d) on the unit's
    // seed; physical += lo + x.
    let (mut f, t, ct, u, m) = kick_world(10, 20);
    let mut s = f.c.units[u].seed;
    let x = s.roll(2560) as i32;
    let mut rec = DamageRecord::default();
    helpers3::kick_damage(&mut f, &t, &ct, u, &mut rec, m, 1, 1);
    assert_eq!(rec.physical, 2560 + x);
    assert_eq!(f.c.units[u].seed, s, "exactly one draw");
    // A power-of-two range uses `lo' & (d − 1)`: min 0, max 1 (<< 8).
    let (mut f, t, ct, u, m) = kick_world(0, 1);
    let mut s = f.c.units[u].seed;
    let x = s.roll(256) as i32;
    assert_eq!(x as u32, stepped(f.c.units[u].seed).lo & 255);
    let mut rec = DamageRecord::default();
    helpers3::kick_damage(&mut f, &t, &ct, u, &mut rec, m, 1, 1);
    assert_eq!(rec.physical, x);
}

#[test]
fn kick_damage_toggles_the_weapons() {
    let (mut f, t, ct, u, m) = kick_world(10, 10);
    f.inventory = true;
    let w = f.c.add_item(FItem {
        types: vec![45],
        ..FItem::default()
    });
    let o = f.c.add_item(FItem {
        types: vec![45],
        ..FItem::default()
    });
    f.c.units[u].weapon = Some(w);
    f.c.units[u].items.insert(5, o);
    f.take_log();
    let mut rec = DamageRecord::default();
    helpers3::kick_damage(&mut f, &t, &ct, u, &mut rec, m, 1, 1);
    let log = f.take_log();
    let off = |i: usize| format!("ItemLists {{ u: {u}, item: {i}, on: false }}");
    let on = |i: usize| format!("ItemLists {{ u: {u}, item: {i}, on: true }}");
    let pos = |s: &str| log.iter().position(|l| l == s);
    assert!(pos(&off(w)).is_some() && pos(&off(o)).is_some(), "{log:?}");
    assert!(pos(&on(w)).is_some(), "the weapon is toggled back on");
    assert!(pos(&on(o)).is_none(), "the other hand is not (Edge case 2)");
    assert!(pos(&off(w)) < pos(&on(w)));
    // A weapon of another item type is neither toggled off nor on.
    let (mut f, t, ct, u, m) = kick_world(10, 10);
    f.inventory = true;
    let w = f.c.add_item(FItem {
        types: vec![46],
        ..FItem::default()
    });
    f.c.units[u].weapon = Some(w);
    f.take_log();
    helpers3::kick_damage(&mut f, &t, &ct, u, &mut DamageRecord::default(), m, 1, 1);
    assert!(
        !f.take_log().iter().any(|l| l.starts_with("ItemLists")),
        "type 46 is not a weapon of item type 45"
    );
}

// -----------------------------------------------------------------

#[test]
fn knock_chance_column_by_target_kind() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(11);
    r.calc2 = c.f(22);
    r.calc3 = c.f(33);
    r.calc4 = c.f(44);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    // Ordinary monster: calc1, or the given value (no formula).
    assert_eq!(knock_chance(&mut f, &t, u, m, 1, 1, None), 11);
    assert_eq!(knock_chance(&mut f, &t, u, m, 1, 1, Some(100)), 100);
    // Type flag 8 (unique): calc2.
    f.c.units[m].flags = 8;
    assert_eq!(knock_chance(&mut f, &t, u, m, 1, 1, None), 22);
    assert_eq!(knock_chance(&mut f, &t, u, m, 1, 1, Some(100)), 22);
    // Boss: calc3 (before the unique test).
    f.c.units[m].boss = true;
    assert_eq!(knock_chance(&mut f, &t, u, m, 1, 1, Some(100)), 33);
    // A hireling: calc4; a player: calc4.
    f.c.units[m].hireling = true;
    assert_eq!(knock_chance(&mut f, &t, u, m, 1, 1, Some(100)), 44);
    let p = f.add(FUnit::new(UnitType::Player, 0), (2, 2));
    assert_eq!(knock_chance(&mut f, &t, u, p, 1, 1, None), 44);
    // The level is passed through unchanged (vestigial clamp): calc with
    // the skill level is the same for every L.
    assert_eq!(knock_chance(&mut f, &t, u, p, 1, 40, None), 44);
}

// -----------------------------------------------------------------

#[test]
fn set_uninterruptable_ends_death_delay() {
    let (mut f, u) = world();
    let m = monster(&mut f, (3, 3));
    set_uninterruptable(&mut f, m, true);
    assert!(f.c.has_state(m, 54));
    // v = 1 never touches death_delay.
    f.c.units[m].states.push(92);
    set_uninterruptable(&mut f, m, true);
    assert!(f.c.has_state(m, 92));
    f.take_log();
    // v = 0 with death_delay and no last attacker: kill (u, 0, 1).
    set_uninterruptable(&mut f, m, false);
    assert!(!f.c.has_state(m, 54));
    assert!(!f.c.has_state(m, 92), "death_delay off");
    let log = f.take_log();
    assert!(
        log.iter()
            .any(|l| l == &format!("Kill {{ u: {m}, a: 0, b: 1 }}")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l == &format!("deltimers {m} 2 0")),
        "a monster's type-2 timers are deleted"
    );
    // A player: no timer deletion; without death_delay nothing else.
    set_uninterruptable(&mut f, u, false);
    let log = f.take_log();
    assert!(!log.iter().any(|l| l.starts_with("deltimers")), "{log:?}");
    assert!(!log.iter().any(|l| l.starts_with("Kill")));
}

// -----------------------------------------------------------------

#[test]
fn nearest_takes_the_first_of_equal_distances() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.c.hostile = true;
    let far = monster(&mut f, (6, 0));
    let a = monster(&mut f, (0, 3));
    let b = monster(&mut f, (3, 0));
    let rejected = monster(&mut f, (1, 0));
    for x in [far, a, b, rejected] {
        f.c.units[x].flags = 0xC; // scan filter 0x80 / 0x400 flag tests
    }
    f.scan = vec![far, a, b, rejected];
    let ok = |_: &mut BodyFake, x: usize| x != rejected;
    // d² 36, 9, 9: the first of the equally near units (strictly <).
    assert_eq!(nearest(&mut f, &t, &ct, u, 10, &ok), Some(a));
    f.scan = vec![far, b, a, rejected];
    assert_eq!(nearest(&mut f, &t, &ct, u, 10, &ok), Some(b));
    // The test rejects every unit: none. r = 0 scans nothing.
    assert_eq!(nearest(&mut f, &t, &ct, u, 10, &|_, _| false), None);
    assert_eq!(nearest(&mut f, &t, &ct, u, 0, &ok), None);
}

// ------------------------------------------------------------------

#[test]
fn prog_count_picks_the_calc_by_the_stat() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.progressive = true;
    r.aurastate = 60;
    r.aurastat1 = 100;
    r.prgcalc1 = c.f(5);
    r.prgcalc2 = c.f(6);
    r.prgcalc3 = c.f(7);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    // No list of the state: prgcalc1.
    assert_eq!(prog_count(&mut f, &t, u, 1, 1), 5);
    // list[aurastat1] clamped to 1…3 → prgcalc_n.
    let l = give_list(&mut f, u, 60, &[(100, 0)]);
    assert_eq!(prog_count(&mut f, &t, u, 1, 1), 5, "0 clamps to 1");
    f.lists[l].stats.insert(100, 2);
    assert_eq!(prog_count(&mut f, &t, u, 1, 1), 6);
    f.lists[l].stats.insert(100, 3);
    assert_eq!(prog_count(&mut f, &t, u, 1, 1), 7);
    f.lists[l].stats.insert(100, 9);
    assert_eq!(prog_count(&mut f, &t, u, 1, 1), 7, "clamped to 3");
    // Not progressive: prgcalc1 in every case.
    let mut r = body_rec();
    r.progressive = false;
    r.aurastate = 60;
    r.aurastat1 = 100;
    let mut c = Code::new();
    r.prgcalc1 = c.f(5);
    r.prgcalc2 = c.f(6);
    let t2 = tabs(r, c, 1);
    f.lists[l].stats.insert(100, 2);
    assert_eq!(prog_count(&mut f, &t2, u, 1, 1), 5);
    // Invalid skill: 0.
    assert_eq!(prog_count(&mut f, &t, u, 99, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn claw_hit_adds_enhanced_damage_and_stores_the_entry() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(40);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.c.set(u, 325, 100_000);
    helpers3::claw_hit(&mut f, &t, &ct, u, m, 1, 1);
    let e = f.c.units[u].combat.last().expect("start_combat stored");
    assert_eq!(e.record.enh_pct, 40, "enh % += eval(calc1)");
    assert_eq!(e.record.hit_flags & 2, 2, "hit flags |= 2");
    assert_eq!(e.record.result & 1, 1);
    // A miss still stores an entry: no tohit at all, not in range.
    let (mut f, u, m) = hit_world();
    f.c.in_range = false;
    helpers3::claw_hit(&mut f, &t, &ct, u, m, 1, 1);
    let e = f.c.units[u].combat.last().expect("also on a miss");
    assert_eq!(e.record.result & 1, 0);
    assert_eq!(e.record.enh_pct, 0);
    // R invalid: nothing.
    let (mut f, u, m) = hit_world();
    helpers3::claw_hit(&mut f, &t, &ct, u, m, 99, 1);
    assert!(f.c.units[u].combat.is_empty());
}

// ------------------------------------------------------------------

#[test]
fn wear_quantity_and_durability() {
    let (mut f, u) = world();
    let wpn = f.c.add_item(FItem {
        types: vec![45],
        durability: true,
        ..FItem::default()
    });
    f.c.units[u].seed = seed_with(0..10);
    f.item_stats.insert((wpn, 72), 20);
    f.take_log();
    // r < chance: durability 20 − 5 = 15, stat message.
    helpers3::wear(&mut f, u, wpn, 10, 5);
    assert_eq!(f.item_stats[&(wpn, 72)], 15);
    // r ≥ chance: nothing (one draw consumed either way).
    f.c.units[u].seed = seed_with(50..100);
    let before = f.c.units[u].seed;
    helpers3::wear(&mut f, u, wpn, 10, 5);
    assert_eq!(f.item_stats[&(wpn, 72)], 15);
    assert_eq!(f.c.units[u].seed, stepped(before), "one draw");
    // d ≤ 0: the item breaks.
    f.c.units[u].seed = seed_with(0..10);
    f.take_log();
    helpers3::wear(&mut f, u, wpn, 10, 15);
    let log = f.take_log();
    assert!(
        log.iter()
            .any(|l| l == &format!("BreakItem {{ u: {u}, item: {wpn} }}")),
        "{log:?}"
    );
    // Not item type 45: nothing, no draw.
    let other = f.c.add_item(FItem {
        types: vec![46],
        durability: true,
        ..FItem::default()
    });
    let before = f.c.units[u].seed;
    helpers3::wear(&mut f, u, other, 100, 1);
    assert_eq!(f.c.units[u].seed, before);
    // A stack with quantity 0: no draw; with quantity > 0 one draw.
    let stack = f.c.add_item(FItem {
        types: vec![45],
        throw: true,
        ..FItem::default()
    });
    helpers3::wear(&mut f, u, stack, 100, 1);
    assert_eq!(f.c.units[u].seed, before, "quantity 0: no draw");
    f.item_stats.insert((stack, 70), 3);
    f.c.units[u].items.insert(4, stack);
    f.c.units[u].weapon = Some(stack);
    helpers3::wear(&mut f, u, stack, 100, 1);
    assert_eq!(f.c.units[u].seed, stepped(before), "one draw");
    assert_eq!(f.item_stats[&(stack, 70)], 2, "dec_quantity");
}

// ------------------------------------------------------------------

/// The stored record of `a` against `d` (kept when `a` is dead-moded, so
/// `apply_melee` returns before it frees the entry).
fn stored_record(f: &BodyFake, a: usize, d: usize) -> Option<DamageRecord> {
    let id = crate::combat::CombatWorld::ident(&f.c, d);
    f.c.units[a]
        .combat
        .iter()
        .find(|e| e.defender == id)
        .map(|e| e.record)
}

#[test]
fn skill_result_and_set_len() {
    // ResultFlags bit 0 (value 1) set: result := ResultFlags, no roll
    // (Rabies has ResultFlags 1).
    let mut r = body_rec();
    r.resultflags = 1 | 8;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    let seed0 = f.c.units[u].seed;
    let mut rec = DamageRecord::default();
    let res = helpers3::skill_result(&mut f, &t, &ct, u, m, 1, 1, &mut rec, 0);
    assert_eq!((res, rec.result), (9, 9));
    assert_eq!(f.c.units[u].seed, seed0, "no roll");
    // Bit 1 (value 2) alone does not skip the roll.
    let mut r = body_rec();
    r.resultflags = 2;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u, m) = hit_world();
    let seed0 = f.c.units[u].seed;
    helpers3::skill_result(&mut f, &t, &ct, u, m, 1, 1, &mut rec, 0);
    assert_ne!(f.c.units[u].seed, seed0, "rolled");
    // Bit clear: melee_result, hit → | ResultFlags; miss → as rolled.
    let mut r = body_rec();
    r.resultflags = 8;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u, m) = hit_world();
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    let res = helpers3::skill_result(&mut f, &t, &ct, u, m, 1, 1, &mut rec, 0);
    assert_eq!(res & 9, 9, "hit | ResultFlags");
    f.c.in_range = false;
    let res = helpers3::skill_result(&mut f, &t, &ct, u, m, 1, 1, &mut rec, 0);
    assert_eq!((res, rec.result), (0, 0), "a miss keeps no ResultFlags");
    // set_len by EType.
    let len = |etype: u8| {
        let mut r = body_rec();
        r.etype = etype;
        let t = tabs(r, Code::new(), 1);
        let mut rec = DamageRecord::default();
        helpers3::set_len(&t, &mut rec, 77, 1);
        (
            rec.cold_len,
            rec.poison_len,
            rec.stun_len,
            rec.burn_len,
            rec.freeze_len,
        )
    };
    assert_eq!(len(4), (77, 0, 0, 0, 0));
    assert_eq!(len(5), (0, 77, 0, 0, 0));
    assert_eq!(len(9), (0, 0, 77, 0, 0));
    assert_eq!(len(11), (0, 0, 0, 77, 0));
    assert_eq!(len(12), (0, 0, 0, 0, 77));
    assert_eq!(len(1), (0, 0, 0, 0, 0));
}

// ------------------------------------------------------------------

#[test]
fn plague_infects_once_and_spreads() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.auratargetstate = 50;
    r.srvmissilea = 1;
    r.aurastat1 = 100;
    r.aurastatcalc1 = c.f(9);
    let t = tabs(r, c, 2);
    let (mut f, u) = world();
    let tg = monster(&mut f, (4, 4));
    f.c.frame = 1000;
    // L < 1: nothing.
    assert_eq!(helpers3::plague(&mut f, &t, u, tg, 50, 1, 0), 0);
    assert!(f.lists.is_empty() && f.missiles.is_empty());
    assert_eq!(helpers3::plague(&mut f, &t, u, tg, 50, 1, 3), 1);
    // Infect: flags 2, expire F + len, state on T, callback,
    // timer 12 at e, aura stats filled.
    let l = f.state_list(tg, 50).expect("list of the state");
    let li = &f.lists[l];
    assert_eq!((li.flags, li.expire, li.owner), (2, 1050, Some(tg)));
    assert_eq!(li.callback, callback::DEFAULT);
    assert!(f.c.has_state(tg, 50));
    assert!(f.c.log.contains(&format!("timer {tg} 12 1050")));
    assert_eq!(f.list_get(l, 100), 9, "aura_fill on the caster's formulas");
    // Spreader: flags 0x8000, owner = origin = T, class m, range len, then
    // the caster's type / GUID in the missile data.
    assert_eq!(f.missiles.len(), 1);
    let q = f.missiles[0];
    assert_eq!(
        (q.flags, q.class, q.skill, q.level, q.range),
        (0x8000, 1, 1, 3, 50)
    );
    assert_eq!((q.owner, q.origin), (tg, Some(tg)));
    let log = f.take_log();
    assert!(log
        .iter()
        .any(|s| s.starts_with("MissileData28") && s.contains("v: 0")));
    assert!(log
        .iter()
        .any(|s| s.starts_with("MissileData2C") && s.contains("v: 0")));
    // Already infected: no new list, no spreader.
    f.missiles.clear();
    assert_eq!(helpers3::plague(&mut f, &t, u, tg, 50, 1, 3), 1);
    assert!(f.missiles.is_empty());
    assert_eq!(f.lists.len(), 1);
    // A state outside 0…count: no infection (the count itself is accepted).
    let mut r = body_rec();
    r.auratargetstate = 201;
    let t2 = tabs(r, Code::new(), 1);
    helpers3::plague(&mut f, &t2, u, tg, 50, 1, 3);
    assert_eq!(f.lists.len(), 1);
}

// ------------------------------------------------------------------

#[test]
fn base_roll_stats_by_weapon() {
    let (mut f, u) = world();
    // No weapon: max(21, 1) << 8 = 256, max(22, 2) << 8 = 512: roll(256).
    let mut s = f.c.units[u].seed;
    let x = s.roll(256) as i32;
    assert_eq!(helpers3::base_roll(&mut f, u), 256 + x);
    assert_eq!(f.c.units[u].seed, s, "one draw");
    // A weapon (not wield type 2): stats 21 / 22.
    f.c.set(u, 21, 10);
    f.c.set(u, 22, 20);
    f.c.set(u, 23, 3);
    f.c.set(u, 24, 3);
    let w = f.c.add_item(FItem::default());
    f.c.units[u].weapon = Some(w);
    f.c.units[u].seed = Seed::new(1, 0);
    let mut s = Seed::new(1, 0);
    let x = s.roll(2560) as i32;
    assert_eq!(helpers3::base_roll(&mut f, u), 2560 + x);
    // Wield type 2: stats 23 / 24; max ≤ min → max = min + 256.
    f.c.items[w].wield = 2;
    f.c.units[u].seed = Seed::new(1, 0);
    let mut s = Seed::new(1, 0);
    let x = s.roll(256) as i32;
    assert_eq!(helpers3::base_roll(&mut f, u), 768 + x);
    // A weapon with a zero minimum: 256 (the "< 1" rule), no max(…, 1).
    f.c.set(u, 23, 0);
    f.c.set(u, 24, 0);
    f.c.units[u].seed = Seed::new(1, 0);
    let mut s = Seed::new(1, 0);
    let x = s.roll(256) as i32;
    assert_eq!(helpers3::base_roll(&mut f, u), 256 + x);
}

// ------------------------------------------------------------------

#[test]
fn next_unit_and_count_units() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.c.hostile = true;
    let ms: Vec<usize> = (0..4)
        .map(|i| {
            let m = monster(&mut f, (i + 1, 0));
            f.c.units[m].flags = 0xC;
            m
        })
        .collect();
    // GUIDs are the fake's indices (1..=4); scan order is reversed.
    f.scan = ms.iter().rev().copied().collect();
    let c = helpers3::count_units(&mut f, &t, &ct, u, (0, 0), 20, 0);
    assert_eq!(c, 4);
    // Smallest GUID above g; the count covers every accepted unit.
    let (k, n) = helpers3::next_unit(&mut f, &t, &ct, u, (0, 0), 20, 0, ms[1] as u32);
    assert_eq!((k, n), (Some(ms[2]), 4));
    // Nothing above: wraps to the smallest GUID.
    let (k, n) = helpers3::next_unit(&mut f, &t, &ct, u, (0, 0), 20, 0, ms[3] as u32);
    assert_eq!((k, n), (Some(ms[0]), 4));
    // Out of range units are not counted.
    let (_, n) = helpers3::next_unit(&mut f, &t, &ct, u, (0, 0), 1, 0, 0);
    assert_eq!(n, 1);
    let (k, n) = helpers3::next_unit(&mut f, &t, &ct, u, (0, 0), 0, 0, 0);
    assert_eq!((k, n), (None, 0));
}

// ------------------------------------------------------------------

#[test]
fn pack_alignment_and_target_list_helpers() {
    let (mut f, _u) = world();
    let leader = monster(&mut f, (2, 2));
    let minion = monster(&mut f, (3, 3));
    // T's owner is a monster: another unit → leave the leader's list.
    f.minion_owner.insert(minion, leader);
    f.take_log();
    helpers3::leave_pack(&mut f, minion);
    assert_eq!(f.take_log(), [format!("LeaveLeader({minion})")]);
    // T itself is its pack owner → the pack is dissolved.
    f.minion_owner.insert(leader, leader);
    helpers3::leave_pack(&mut f, leader);
    assert_eq!(f.take_log(), [format!("DissolvePack({leader})")]);
    // Owner not a monster / no owner: nothing.
    f.minion_owner.insert(minion, 0);
    helpers3::leave_pack(&mut f, minion);
    f.minion_owner.remove(&minion);
    helpers3::leave_pack(&mut f, minion);
    assert!(f.take_log().is_empty());
    // Alignment: the callbacks set 0 (evil) and unlink the target list.
    remove_alignment(&mut f, minion, 53);
    let log = f.take_log();
    assert!(log.contains(&format!("Alignment {{ u: {minion}, a: 0, v: 1 }}")));
    assert!(log.contains(&format!("NodeRemove({minion})")));
    // Target-list prepend only for a player / monster in no list.
    node_prepend(&mut f, minion, 8);
    assert_eq!(
        f.take_log(),
        [format!("NodePrepend {{ u: {minion}, slot: 8 }}")]
    );
    f.node.insert(minion, 8);
    node_prepend(&mut f, minion, 9);
    let item = f.add(FUnit::new(UnitType::Item, 0), (0, 0));
    node_prepend(&mut f, item, 9);
    assert!(f.take_log().is_empty());
}

// ------------------------------------------------------------------

#[test]
fn frenzy_charge_stacks_to_the_level() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 70;
    r.auralencalc = c.f(100);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    f.c.frame = 500;
    // E param 1 = 0 (the last swing missed): nothing.
    assert_eq!(frenzy_charge(&mut f, &t, u, 1, 3), 0);
    assert!(f.lists.is_empty());
    f.set_entry_param_of(u, &e, 1, 1);
    assert_eq!(frenzy_charge(&mut f, &t, u, 1, 2), 1);
    let l = f.state_list(u, 70).expect("list");
    let li = &f.lists[l];
    assert_eq!((li.flags, li.expire, li.owner), (2, 600, Some(u)));
    assert_eq!(li.callback, callback::DEFAULT);
    assert_eq!(f.list_get(l, 169), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("timer {u} 12 600")), "{log:?}");
    assert!(log.contains(&format!("changed {u} 70")));
    f.c.frame = 510;
    frenzy_charge(&mut f, &t, u, 1, 2);
    frenzy_charge(&mut f, &t, u, 1, 2);
    assert_eq!(f.list_get(l, 169), 2, "min(count + 1, L)");
    assert_eq!(f.lists[l].expire, 610);
    assert_eq!(f.lists.len(), 1, "the same list is reused");
    // Invalid skill: 0.
    assert_eq!(frenzy_charge(&mut f, &t, u, 99, 2), 0);
}

#[test]
fn frenzy_swing_hit_and_miss() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(30);
    r.hitflags = 0x10;
    r.hitclass = 3;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    let e = f.c.units[u].used.unwrap();
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.c.units[u].mode = 0; // keeps the stored entry after apply_melee
    assert_eq!(frenzy_swing(&mut f, &t, &ct, u, Some(m), 1, 1), 1);
    let rec = stored_record(&f, u, m).expect("entry");
    assert_eq!(
        (rec.enh_pct, rec.hit_flags & 0x10, rec.hit_class),
        (30, 0x10, 3)
    );
    assert_eq!(f.entry_param(u, &e, 1), 1, "E param 1 := 1 on a hit");
    assert!(f
        .take_log()
        .iter()
        .any(|s| s == &format!("path {u} TargetUnit(Some({m}))")));
    // A miss: param 1 := 0.
    let (mut f, u, m) = hit_world();
    f.c.in_range = false;
    f.set_entry_param_of(u, &e, 1, 1);
    assert_eq!(frenzy_swing(&mut f, &t, &ct, u, Some(m), 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 1), 0);
    // T none or R invalid: 0.
    assert_eq!(frenzy_swing(&mut f, &t, &ct, u, None, 1, 1), 0);
    assert_eq!(frenzy_swing(&mut f, &t, &ct, u, Some(m), 99, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn blade_pulse_hits_every_unit_in_range() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurarangecalc = c.f(6);
    r.mindam = 4;
    r.maxdam = 4;
    r.hitshift = 8;
    r.hitflags = 0x10;
    r.hitclass = 5;
    r.resultflags = 2 | 1; // bit 1 set: no roll, always the flags
    r.aurafilter = 0x8783;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.c.hostile = true;
    f.c.in_range = true;
    let near = monster(&mut f, (2, 0));
    let far = monster(&mut f, (9, 0));
    for x in [near, far] {
        f.c.units[x].flags = 0xC;
    }
    f.c.units[u].mode = 0;
    f.scan = vec![near, far];
    f.take_log();
    assert_eq!(blade_pulse(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = stored_record(&f, u, near).expect("the near unit is hit");
    assert_eq!(rec.hit_flags & 0x12, 0x12, "hit flags 2 | HitFlags");
    assert_eq!(rec.hit_class, 5);
    assert_eq!(rec.physical, 4 << 8, "roll_physical");
    assert_eq!(rec.result & 3, 3);
    assert!(
        stored_record(&f, u, far).is_none(),
        "r = eval(aurarangecalc)"
    );
    // Invalid skill: 0.
    assert_eq!(blade_pulse(&mut f, &t, &ct, u, 99, 1), 0);
}

// ---------------------------------------------------------------

#[test]
fn jab_rolls_on_a_hit_and_stores_the_entry() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(25);
    r.etype = 1;
    r.calc4 = c.f(50);
    r.emin = 7;
    r.emax = 7;
    r.hitshift = 8;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.c.units[u].mode = 0; // keeps the stored entry after apply_melee
    assert_eq!(b3_lvl01::jab(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = stored_record(&f, u, m).expect("start_combat stored");
    assert_eq!(rec.result & 1, 1);
    assert_eq!(rec.enh_pct, 25, "enhanced damage % := eval(calc1)");
    assert_eq!((rec.conv_pct, rec.conv_elem), (50, 1), "EType conversion");
    // A miss: no rolls, nothing stored for the apply.
    let (mut f, u, _m) = hit_world();
    f.c.in_range = false;
    let seed0 = f.c.units[u].seed;
    assert_eq!(b3_lvl01::jab(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].seed, seed0, "no roll_elemental on a miss");
    // R invalid / T none: 0.
    assert_eq!(b3_lvl01::jab(&mut f, &t, &ct, u, 99, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl01::jab(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn charged_bolt_makes_n_jittered_missiles() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(3);
    r.srvmissilea = 1;
    let t = tabs(r, c, 2);
    let (mut f, u) = world();
    f.tpos.insert(u, (12, 7));
    assert_eq!(b3_lvl01::charged_bolt(&mut f, &t, u, 1, 4), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40, "unit flags |= 0x40");
    assert_eq!(f.missiles.len(), 3);
    for (i, q) in f.missiles.iter().enumerate() {
        assert_eq!(q.flags, 0x21);
        assert_eq!((q.owner, q.class, q.skill, q.level), (u, 1, 1, 4));
        assert_eq!((q.x, q.y), (0, 0), "the unit's position");
        assert_eq!((q.target_x, q.target_y), (12, 7));
        assert_eq!(q.init, Some((init_cb::JITTER, i as u32)));
    }
    // A zero coordinate: that i makes nothing; still returns 1.
    f.missiles.clear();
    f.tpos.insert(u, (12, 0));
    assert_eq!(b3_lvl01::charged_bolt(&mut f, &t, u, 1, 4), 1);
    assert!(f.missiles.is_empty());
    // n ≤ 0: returns 1, nothing.
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(0);
    r.srvmissilea = 1;
    let t0 = tabs(r, c, 2);
    f.tpos.insert(u, (12, 7));
    assert_eq!(b3_lvl01::charged_bolt(&mut f, &t0, u, 1, 4), 1);
    assert!(f.missiles.is_empty());
    // Missile out of range (≥ count): 0; the flag was set before.
    f.c.units[u].flags = 0;
    let mut r = body_rec();
    r.srvmissilea = 5;
    let t5 = tabs(r, Code::new(), 2);
    assert_eq!(b3_lvl01::charged_bolt(&mut f, &t5, u, 1, 4), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40, "flag set before the test");
    // R invalid: 0.
    assert_eq!(b3_lvl01::charged_bolt(&mut f, &t, u, 99, 4), 0);
}

#[test]
fn raven_spawns_without_a_target_node() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 0;
    r.summode = 1;
    r.pettype = 3;
    r.petmax = c.f(4);
    r.calc2 = c.f(2);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.monlvl = vec![crate::skills::fake::blank::<d2_data::tables::Monlvl>(); 10];
    for row in &mut f.monlvl {
        row.ac = 7;
        row.th = 11;
    }
    f.c.units[u].flags = 0;
    assert_eq!(b3_lvl01::raven(&mut f, &t, &ct, u, 1, 3), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    let log = f.take_log();
    assert!(log.iter().any(|s| s.starts_with("monster 1 (0, 0) 0 1 -1")));
    let m = 1; // the spawned monster
    assert!(log.contains(&format!("set {m} 74 0")), "hpregen := 0");
    assert!(log.contains(&format!("set {m} 12 2")), "base_stats level");
    assert!(log.contains(&format!("add {m} 31 7")));
    assert!(log.contains(&format!("add {m} 19 11")));
    assert!(log
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 3, max: 4")));
    assert!(!log.iter().any(|s| s.starts_with("NodeInsert")), "no node");
    assert_eq!(f.c.units[m].flags & 0x4, 0, "m flags &= ~0x4");
    // pettype out of range: 0, no flag; spawn failure: 0 with the flag.
    f.c.units[u].flags = 0;
    f.pettypes = 3;
    assert_eq!(b3_lvl01::raven(&mut f, &t, &ct, u, 1, 3), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    f.pettypes = 15;
    f.no_monsters = true;
    assert_eq!(b3_lvl01::raven(&mut f, &t, &ct, u, 1, 3), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // R invalid: 0.
    assert_eq!(b3_lvl01::raven(&mut f, &t, &ct, u, 99, 3), 0);
}

#[test]
fn firestorm_is_a_fan_with_one_straight_missile() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(4);
    r.srvmissilea = 1;
    let t = tabs(r, c, 2);
    let (mut f, u) = world();
    f.tpos.insert(u, (12, 7));
    assert_eq!(b3_lvl01::firestorm(&mut f, &t, u, 1, 5), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    let log = f.take_log();
    let _ = log;
    assert_eq!(f.missiles.len(), 4, "1 straight + (n − 1) jittered");
    assert_eq!(f.missiles[0].init, None, "the straight skill_missile");
    for (i, q) in f.missiles[1..].iter().enumerate() {
        assert_eq!(q.init, Some((init_cb::JITTER, i as u32)), "argument 0…n−2");
        assert_eq!((q.target_x, q.target_y), (12, 7));
    }
    // n ≤ 0: 0 (after the flag); an invalid missile: 0 (before the flag).
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(0);
    r.srvmissilea = 1;
    let t0 = tabs(r, c, 2);
    assert_eq!(b3_lvl01::firestorm(&mut f, &t0, u, 1, 5), 0);
    f.c.units[u].flags = 0;
    let mut r = body_rec();
    r.srvmissilea = 9;
    let t9 = tabs(r, Code::new(), 2);
    assert_eq!(b3_lvl01::firestorm(&mut f, &t9, u, 1, 5), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    assert_eq!(b3_lvl01::firestorm(&mut f, &t, u, 99, 5), 0);
}

#[test]
fn psychic_hammer_start_tests_target_and_towns() {
    let (mut f, u) = world();
    assert_eq!(b3_lvl01::psychic_hammer_start(&mut f, u), 0, "T none");
    let m = monster(&mut f, (3, 3));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl01::psychic_hammer_start(&mut f, u), 1);
    // T's room in town → 0; the unit's room in town → 0.
    f.room_of.insert(m, 2);
    f.town.insert(2);
    assert_eq!(b3_lvl01::psychic_hammer_start(&mut f, u), 0);
    f.room_of.insert(m, 1);
    f.town.clear();
    f.town.insert(1);
    f.room_of.insert(m, 2);
    assert_eq!(b3_lvl01::psychic_hammer_start(&mut f, u), 0);
    f.town.clear();
    // A target that is neither player nor monster: 0.
    let o = f.add(FUnit::new(UnitType::Object, 0), (4, 4));
    f.targets.insert(u, o);
    assert_eq!(b3_lvl01::psychic_hammer_start(&mut f, u), 0);
    let p = f.add(FUnit::new(UnitType::Player, 0), (5, 5));
    f.targets.insert(u, p);
    assert_eq!(b3_lvl01::psychic_hammer_start(&mut f, u), 1);
}

#[test]
fn psychic_hammer_applies_then_flags_the_reaction() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.mindam = 1;
    r.maxdam = 1;
    r.hitshift = 8;
    r.calc1 = c.f(100);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (3, 3));
    f.targets.insert(u, m);
    f.c.hostile = true;
    f.c.set(m, 6, 10_000);
    let seed0 = f.c.units[u].seed;
    assert_eq!(b3_lvl01::psychic_hammer(&mut f, &t, &ct, u, 1, 1), 1);
    // k = calc1 = 100 > 0: one draw, r < 100 → knockback; life ≥ 256 →
    // no will-die; result |= 1.
    let (a, d, r) = *f.c.reactions.last().expect("reaction");
    let res = r.result;
    assert_eq!((a, d, res), (u, m, 1 | 8));
    assert!(f.c.get(m, 6) < 10_000, "the damage was applied first");
    // roll_physical (1 draw) + the knock draw.
    assert_ne!(f.c.units[u].seed, seed0);
    // Life below 256 after the apply: will die (|= 2).
    f.c.set(m, 6, 300);
    b3_lvl01::psychic_hammer(&mut f, &t, &ct, u, 1, 1);
    let res = f.c.reactions.last().unwrap().2.result;
    assert_eq!(res & 2, 2, "life < 256 → will die");
    // k ≤ 0: no knock draw, no knockback bit.
    let mut r = body_rec();
    r.mindam = 1;
    r.maxdam = 1;
    r.hitshift = 8;
    let t0 = tabs(r, Code::new(), 1);
    f.c.set(m, 6, 10_000);
    b3_lvl01::psychic_hammer(&mut f, &t0, &ct, u, 1, 1);
    let res = f.c.reactions.last().unwrap().2.result;
    assert_eq!(res & 8, 0);
    // The start test failing (T none): 0, nothing applied.
    f.targets.clear();
    let n = f.c.reactions.len();
    assert_eq!(b3_lvl01::psychic_hammer(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.reactions.len(), n);
    assert_eq!(b3_lvl01::psychic_hammer(&mut f, &t, &ct, u, 99, 1), 0);
}

#[test]
fn dragon_talon_do_counts_down_the_kicks() {
    let mut r = body_rec();
    r.param1 = 5;
    r.param2 = 7;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    let e = f.c.units[u].used.unwrap();
    f.c.units[u].mode = 0; // keeps the kick's entry after apply_melee
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.set_entry_param_of(u, &e, 1, 3);
    f.c.units[u].flags = 0x40;
    f.take_log();
    assert_eq!(b3_lvl01::dragon_talon(&mut f, &t, &ct, u, 1, 1), 1);
    // n = 3 − 1 = 2 > 0: stored, flag 0x40 cleared, rewind(100), no knock.
    assert_eq!(f.entry_param(u, &e, 1), 2);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    assert!(f.take_log().contains(&format!("rewind {u} 100")));
    let rec = stored_record(&f, u, m).expect("kick entry");
    assert_eq!((rec.result & 8, rec.hit_class), (0, 1), "no knockback yet");
    // The last kick: ordinary target → chance 100 (no formula) → knock.
    f.c.units[u].combat.clear();
    f.set_entry_param_of(u, &e, 1, 1);
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    assert_eq!(b3_lvl01::dragon_talon(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 1), 0);
    let rec = stored_record(&f, u, m).expect("kick entry");
    assert_eq!(rec.result & 0xC, 0xC, "get-hit and knockback");
    assert!(!f.take_log().contains(&format!("rewind {u} 100")));
    // A unique target with calc2 = 0: k < 1, no draw, no knock.
    f.c.units[u].combat.clear();
    f.c.units[m].flags = 8;
    f.set_entry_param_of(u, &e, 1, 1);
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    b3_lvl01::dragon_talon(&mut f, &t, &ct, u, 1, 1);
    let rec = stored_record(&f, u, m).expect("kick entry");
    assert_eq!(rec.result & 8, 0);
    // Param 1 − 1 < 0: 0. T none / R invalid: 0.
    f.set_entry_param_of(u, &e, 1, 0);
    assert_eq!(b3_lvl01::dragon_talon(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(b3_lvl01::dragon_talon(&mut f, &t, &ct, u, 99, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl01::dragon_talon(&mut f, &t, &ct, u, 1, 1), 0);
}

// -----------------------------------------------------------------

/// A monster (class 0) with a base list (flag 1) holding a stale stat, and
/// a one-row monlvl; returns the world, the monster, the list and tables.
fn mode_world(
    edit: impl FnOnce(&mut d2_data::tables::Monstats),
) -> (BodyFake, usize, usize, SkillTables, CombatTables) {
    let mut ms = monster_rec();
    ms.noratio = true;
    ms.a1mind = 10;
    ms.a1maxd = 20;
    ms.a1th = 100;
    ms.a2mind = 30;
    ms.a2maxd = 40;
    ms.a2th = 110;
    ms.s1mind = 50;
    ms.s1maxd = 60;
    ms.s1th = 120;
    edit(&mut ms);
    let ct = combat_tables(vec![ms]);
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, _u) = world();
    f.monlvl = vec![crate::skills::fake::blank::<d2_data::tables::Monlvl>(); 1];
    let m = monster(&mut f, (4, 4));
    let l = give_list(&mut f, m, 0, &[(99, 1)]);
    f.lists[l].flags = 1;
    (f, m, l, t, ct)
}

fn stats3(f: &BodyFake, l: usize) -> (i32, i32, i32) {
    (f.list_get(l, 21), f.list_get(l, 22), f.list_get(l, 19))
}

#[test]
fn mode_damage_rewrites_the_base_stats() {
    let (mut f, m, l, t, ct) = mode_world(|_| {});
    // Step 3: all stats of the base list are removed first; mode 5 → A2,
    // 7 / 8 → S1, other (4, and 6 block in the base game) → A1; noRatio columns.
    mode_damage(&mut f, &t, &ct, m, 5);
    assert_eq!(f.list_get(l, 99), 0, "stale stats removed");
    assert_eq!(stats3(&f, l), (30, 40, 110));
    mode_damage(&mut f, &t, &ct, m, 7);
    assert_eq!(stats3(&f, l), (50, 60, 120));
    mode_damage(&mut f, &t, &ct, m, 8);
    assert_eq!(stats3(&f, l), (50, 60, 120));
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (10, 20, 100));
    mode_damage(&mut f, &t, &ct, m, 6);
    assert_eq!(stats3(&f, l), (10, 20, 100), "block takes the A1 row");
    // Step 1: not a monster / no list / no record: nothing.
    let (mut f2, m2, l2, t2, ct2) = mode_world(|_| {});
    f2.lists[l2].flags = 2;
    mode_damage(&mut f2, &t2, &ct2, m2, 4);
    assert_eq!(f2.list_get(l2, 99), 1, "no base list: untouched");
    f2.lists[l2].flags = 1;
    mode_damage(&mut f2, &t2, &ct2, 0, 4);
    assert_eq!(f2.list_get(l2, 99), 1, "a player is not a monster");
    let ct_none = combat_tables(vec![]);
    mode_damage(&mut f2, &t2, &ct_none, m2, 4);
    assert_eq!(f2.list_get(l2, 99), 1, "no monstats record");
    // Without noRatio: pct(monlvl DM, value, 100) and TH likewise.
    let (mut f, m, l, t, ct) = mode_world(|ms| ms.noratio = false);
    f.monlvl[0].dm = 150;
    f.monlvl[0].th = 200;
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (15, 30, 200));
    // The L-flag picks the L-DM / L-TH columns.
    f.l_flag = true;
    f.monlvl[0].l_dm = 300;
    f.monlvl[0].l_th = 50;
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (30, 60, 50));
}

#[test]
fn mode_damage_difficulty_and_player_count() {
    // Classic (not expansion), d = 1, Align ≠ 1: 10/12 and 10/15.
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.a1mind_n = 120;
        ms.a1maxd_n = 240;
        ms.a1th_n = 150;
    });
    f.c.difficulty = 1;
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (100, 200, 100));
    // Align = 1 (neutral) is exempt.
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.a1mind_n = 120;
        ms.a1maxd_n = 240;
        ms.a1th_n = 150;
        ms.align = 1;
    });
    f.c.difficulty = 1;
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (120, 240, 150));
    // Expansion, 3 players on Nightmare: mult = 16, v += trunc(v·16/128).
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.a1mind_n = 80;
        ms.a1maxd_n = 130;
        ms.a1th_n = 100;
    });
    f.c.difficulty = 1;
    f.c.expansion = true;
    f.c.set(m, 100, 3);
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (90, 146, 112));
    // Normal difficulty: no multiplier whatever the count.
    f.c.difficulty = 0;
    f.c.set(m, 100, 5);
    mode_damage(&mut f, &t, &ct, m, 4);
    assert_eq!(stats3(&f, l), (10, 20, 100));
}

#[test]
fn mode_damage_element_slots() {
    // Slot 1 cold on mode 5 (El1 columns, scaled by the monlvl DM), slot 2
    // poison on mode 5 reading the El1 columns too (Edge case 1), slot 3
    // on another mode: skipped.
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.noratio = false;
        ms.el1mode = 5;
        ms.el1type = 4;
        ms.el1pct = 100;
        ms.el1mind = 4;
        ms.el1maxd = 8;
        ms.el1dur = 30;
        ms.el2mode = 5;
        ms.el2type = 5;
        ms.el2pct = 100;
        ms.el3mode = 4;
        ms.el3type = 1;
        ms.el3pct = 100;
    });
    f.monlvl[0].dm = 150;
    f.monlvl[0].th = 100;
    mode_damage(&mut f, &t, &ct, m, 5);
    assert_eq!(
        (f.list_get(l, 54), f.list_get(l, 55), f.list_get(l, 56)),
        (6, 12, 30)
    );
    assert_eq!(
        (f.list_get(l, 57), f.list_get(l, 58), f.list_get(l, 59)),
        (60, 120, 60),
        "poison: 10·min, 10·max, 2·len"
    );
    assert_eq!(f.list_get(l, 48), 0, "slot 3 has another mode");
    // Expansion with mult ≠ 0: min, max, len each += trunc(v·mult/128).
    f.c.expansion = true;
    f.c.difficulty = 1;
    f.c.set(m, 100, 3);
    f.lists[l].stats.clear();
    f.monlvl[0].dm_n = 100;
    let mut ct2 = ct.clone();
    ct2.monstats[0].el1pct_n = 100;
    ct2.monstats[0].el2pct_n = 100;
    ct2.monstats[0].el1mind_n = 64;
    ct2.monstats[0].el1maxd_n = 128;
    ct2.monstats[0].el1dur_n = 64;
    mode_damage(&mut f, &t, &ct2, m, 5);
    assert_eq!(
        (f.list_get(l, 54), f.list_get(l, 55), f.list_get(l, 56)),
        (72, 144, 72)
    );
    // noRatio: min = max = 0 and len = El1Dur (fire has no length).
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.el1mode = 5;
        ms.el1type = 11;
        ms.el1pct = 100;
        ms.el1mind = 4;
        ms.el1maxd = 8;
        ms.el1dur = 30;
    });
    mode_damage(&mut f, &t, &ct, m, 5);
    assert_eq!(
        (f.list_get(l, 316), f.list_get(l, 317), f.list_get(l, 315)),
        (0, 0, 30)
    );
    // The chance: p < 100 draws once on the unit's seed; r ≥ p skips.
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.el1mode = 5;
        ms.el1type = 11;
        ms.el1pct = 50;
        ms.el1dur = 30;
    });
    f.c.units[m].seed = seed_with(0..50);
    mode_damage(&mut f, &t, &ct, m, 5);
    assert_eq!(f.list_get(l, 315), 30, "r < p: applied");
    f.lists[l].stats.clear();
    f.c.units[m].seed = seed_with(50..100);
    let before = f.c.units[m].seed;
    mode_damage(&mut f, &t, &ct, m, 5);
    assert_eq!(f.list_get(l, 315), 0, "r ≥ p: skipped");
    assert_eq!(f.c.units[m].seed, stepped(before), "exactly one draw");
    // Type 10 (random): one more draw, t = roll(5) + 1; len 0 → 25.
    let (mut f, m, l, t, ct) = mode_world(|ms| {
        ms.el1mode = 5;
        ms.el1type = 10;
        ms.el1pct = 100;
        ms.el1dur = 0;
    });
    let mut s = f.c.units[m].seed;
    let ty = s.roll(5) as i32 + 1;
    mode_damage(&mut f, &t, &ct, m, 5);
    assert_eq!(f.c.units[m].seed, s, "one draw (pct 100 draws none)");
    if ty == 4 {
        assert_eq!(f.list_get(l, 56), 25);
    }
    if ty == 5 {
        assert_eq!(f.list_get(l, 59), 50);
    }
    if ty == 9 {
        assert_eq!(f.list_get(l, 66), 25);
    }
}

#[test]
fn jab_of_a_monster_sets_the_s1_damage() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, m, l, _t, ct) = mode_world(|_| {});
    let p = 0; // the player of the world
    f.targets.insert(m, p);
    f.c.set(m, 12, 1);
    f.c.set(p, 12, 1);
    f.c.hostile = true;
    f.c.in_range = true;
    assert_eq!(b3_lvl01::jab(&mut f, &t, &ct, m, 1, 1), 1);
    assert_eq!(stats3(&f, l), (50, 60, 120), "mode_damage(unit, 8)");
}

// ---------------------------------------------------------------

/// Static Field record: lightning, 50 %, floor 512, range 10.
fn static_rec(c: &mut Code) -> d2_data::tables::Skills {
    let mut r = body_rec();
    r.calc1 = c.f(50);
    r.calc2 = c.f(512);
    r.aurarangecalc = c.f(10);
    r.aurafilter = 0x8783;
    r.etype = 2;
    r
}

fn static_world(life: &[i32]) -> (BodyFake, usize, Vec<usize>) {
    let (mut f, u) = world();
    f.c.hostile = true;
    f.c.in_range = true;
    let ms: Vec<usize> = life
        .iter()
        .enumerate()
        .map(|(i, &l)| {
            let m = monster(&mut f, (1 + i as i32, 0));
            f.c.units[m].flags = 0xC;
            f.c.set(m, 6, l);
            f.c.set(m, 7, 1000 << 8);
            m
        })
        .collect();
    f.scan = ms.clone();
    (f, u, ms)
}

#[test]
fn static_field_takes_a_percentage_of_the_current_life() {
    let mut c = Code::new();
    let r = static_rec(&mut c);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    // h = 100: v = 50 % = 50 → 50 << 8 = 12800. h = 3: v = 3 would leave
    // 0, so v = h − 1 = 2 → 512 (also the floor). h = 1: v = 0 → the floor
    // 512 kills. h = 0 (life 100 < 256): skipped, untouched.
    let (mut f, u, ms) = static_world(&[100 << 8, 3 << 8, 300, 100]);
    assert_eq!(b3_lvl06::static_field(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.get(ms[0], 6), 100 * 256 - 12800);
    assert_eq!(f.c.get(ms[1], 6), 256, "h − v < 1: v = h − 1");
    assert_eq!(f.c.get(ms[2], 6), 0, "v below the floor is raised to it");
    assert_eq!(f.c.get(ms[3], 6), 100, "h < 1: skipped");
    assert!(f.c.reactions.iter().any(|&(a, d, _)| (a, d) == (u, ms[0])));
    assert!(
        !f.c.reactions.iter().any(|&(_, d, _)| d == ms[3]),
        "no reaction for the skipped unit"
    );
    let rr = f.c.reactions.iter().find(|r| r.1 == ms[0]).unwrap().2;
    let res = rr.result;
    assert_eq!(rr.hit_class & 0xD, 0xD, "hit class |= 0xD");
    assert_eq!(rr.hit_class_fixed, 1);
    assert_eq!(rr.lightning, 12800, "the element, not the probe");
    assert_eq!(res & 0x4001, 0x4001, "result 0x4001");
    // A negative resistance does not raise the damage (pre-divided).
    let (mut f, u, ms) = static_world(&[100 << 8]);
    f.c.set(ms[0], 41, -50);
    b3_lvl06::static_field(&mut f, &t, &ct, u, 1, 1);
    let dealt = 100 * 256 - f.c.get(ms[0], 6);
    assert!((12798..=12800).contains(&dealt), "dealt {dealt}");
    // Classic: no cap. Expansion: units with h ≤ pct(max, StaticFieldMin)
    // are skipped.
    let mut ct2 = ct.clone();
    ct2.difficultylevels[0].staticfieldmin = 25;
    let (mut f, u, ms) = static_world(&[200 << 8, 300 << 8]);
    b3_lvl06::static_field(&mut f, &t, &ct2, u, 1, 1);
    assert!(f.c.get(ms[0], 6) < 200 << 8, "classic: cap 0");
    let (mut f, u, ms) = static_world(&[200 << 8, 300 << 8]);
    f.c.expansion = true;
    b3_lvl06::static_field(&mut f, &t, &ct2, u, 1, 1);
    assert_eq!(f.c.get(ms[0], 6), 200 << 8, "h = 200 ≤ 25 % of 1000");
    assert!(f.c.get(ms[1], 6) < 300 << 8, "h = 300 > 250");
    // A random element (EType 10) draws once on the source's seed in the
    // probe; the second add_element keeps that element.
    let mut c = Code::new();
    let mut r = static_rec(&mut c);
    r.etype = 10;
    let t10 = tabs(r, c, 1);
    let (mut f, u, ms) = static_world(&[100 << 8]);
    f.c.units[u].seed = seed_with(0..100);
    let before = f.c.units[u].seed;
    b3_lvl06::static_field(&mut f, &t10, &ct, u, 1, 1);
    assert_eq!(f.c.units[u].seed, stepped(before), "one draw only");
    let rr = f.c.reactions.iter().find(|r| r.1 == ms[0]).unwrap().2;
    // The chosen element is {fire, lightning, cold, poison}[lo' & 3].
    let chosen = [rr.fire, rr.lightning, 0, rr.cold, rr.poison][match stepped(before).lo & 3 {
        0 => 0,
        1 => 1,
        2 => 3,
        _ => 4,
    }];
    assert!(chosen > 0, "the drawn element carries the damage: {rr:?}");
    // R invalid: 0.
    assert_eq!(b3_lvl06::static_field(&mut f, &t, &ct, u, 99, 1), 0);
}

#[test]
fn telekinesis_by_target_type() {
    let mut r = body_rec();
    r.mindam = 2;
    r.maxdam = 2;
    r.hitshift = 8;
    r.param2 = 100;
    r.resultflags = 0x20;
    r.hitflags = 0x10;
    r.hitclass = 3;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (3, 3));
    f.c.hostile = true;
    f.targets.insert(u, m);
    f.c.set(m, 6, 10_000);
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // Param2 = 100: always knockback; | ResultFlags; the damage is applied.
    let res = f.c.reactions.last().unwrap().2.result;
    assert_eq!(res & 0x29, 0x29);
    assert_eq!(f.c.get(m, 6), 10_000 - 512);
    // Param2 = 0: no knockback (the draw is still made).
    let mut r = body_rec();
    r.param2 = 0;
    let t0 = tabs(r, Code::new(), 1);
    let seed0 = f.c.units[u].seed;
    b3_lvl06::telekinesis(&mut f, &t0, &ct, u, 1, 1);
    assert_ne!(f.c.units[u].seed, seed0);
    assert_eq!(f.c.reactions.last().unwrap().2.result & 8, 0);
    // Not hostile / town: 0 (after the 0x40 flag).
    f.c.units[u].flags = 0;
    f.c.hostile = false;
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    f.c.hostile = true;
    f.room_of.insert(m, 2);
    f.town.insert(2);
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 0);
    f.town.clear();
    // An object: operate. An item of type 22: pickup; another: sound 0x13.
    let o = f.add(FUnit::new(UnitType::Object, 0), (4, 4));
    f.targets.insert(u, o);
    f.take_log();
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(
        f.take_log(),
        [format!("OperateObject {{ u: {u}, object: {o} }}")]
    );
    let scroll = f.c.add_item(FItem {
        types: vec![22],
        ..FItem::default()
    });
    let sword = f.c.add_item(FItem {
        types: vec![3],
        ..FItem::default()
    });
    let i1 = f.add(FUnit::new(UnitType::Item, scroll as i32), (5, 5));
    let i2 = f.add(FUnit::new(UnitType::Item, sword as i32), (5, 5));
    f.targets.insert(u, i1);
    b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(
        f.take_log(),
        [format!("AutoPickup {{ u: {u}, item: {i1} }}")]
    );
    f.targets.insert(u, i2);
    b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.take_log(), [format!("Sound {{ u: {u}, id: 19 }}")]);
    // A missile target: 1, nothing. Busy / T none / a monster caster: 0.
    let ms = f.add(FUnit::new(UnitType::Missile, 0), (6, 6));
    f.targets.insert(u, ms);
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 1);
    f.busy = true;
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 0);
    f.busy = false;
    f.targets.clear();
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(m, u);
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, m, 1, 1), 0);
    assert_eq!(b3_lvl06::telekinesis(&mut f, &t, &ct, u, 99, 1), 0);
}

#[test]
fn poison_dagger_start_and_do() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(30);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.c.units[u].mode = 0;
    assert_eq!(b3_lvl06::poison_dagger_start(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = stored_record(&f, u, m).expect("start_combat stored");
    assert_eq!((rec.result & 1, rec.enh_pct), (1, 30));
    assert_eq!(rec.conv_pct, 0, "no EType: no conversion");
    // T none, T in town, R invalid: 0.
    assert_eq!(b3_lvl06::poison_dagger_start(&mut f, &t, &ct, u, 99, 1), 0);
    f.room_of.insert(m, 2);
    f.town.insert(2);
    assert_eq!(b3_lvl06::poison_dagger_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl06::poison_dagger_start(&mut f, &t, &ct, u, 1, 1), 0);
    // The do: flag 0x40, T none → 0 (flag set), else apply_melee → 1.
    let (mut f, u, m) = hit_world();
    f.c.set(m, 6, 10_000);
    let rec = DamageRecord {
        result: 1,
        physical: 512,
        total: 512,
        ..DamageRecord::default()
    };
    super::fake::stored(&mut f, u, m, rec);
    assert_eq!(b3_lvl06::poison_dagger(&mut f, &ct, u), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert!(f.c.units[u].combat.is_empty(), "applied and freed");
    assert!(f.c.get(m, 6) < 10_000);
    f.targets.clear();
    f.c.units[u].flags = 0;
    assert_eq!(b3_lvl06::poison_dagger(&mut f, &ct, u), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40, "flag set first");
}

#[test]
fn leap_start_player_reserves_the_landing() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurarangecalc = c.f(10);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    // No target position: 0.
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.tpos.insert(u, (5, 5));
    f.free_shift = Some((0, 0));
    f.take_log();
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log
        .iter()
        .any(|s| s.starts_with("PatternStamp") && s.contains("mask: 128")));
    assert!(f.c.has_state(u, 54), "uninterruptable");
    assert!(f.c.has_state(u, 18), "skill_move");
    assert_eq!((f.entry_param(u, &e, 1), f.entry_param(u, &e, 2)), (5, 5));
    assert_eq!(f.entry_flags(u, &e), 0x80);
    // The room test fails (no room at the point): 0. The clamp fails: 0.
    f.point_rooms.insert((5, 5), None);
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.point_rooms.clear();
    f.free_shift = None;
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, u, 1, 1), 0);
    // R invalid / E none: 0.
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, u, 99, 1), 0);
    f.c.units[u].used = None;
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn leap_start_monster_jumps_beyond_the_target() {
    let mut r = body_rec();
    r.intown = true;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, _p) = world();
    let m = monster(&mut f, (10, 10));
    let tg = monster(&mut f, (14, 12));
    let e = SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    f.c.units[m].used = Some(e);
    f.c.units[m].skills.push(e);
    f.targets.insert(m, tg);
    f.free_shift = Some((0, 0));
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(m, 12, 1);
    f.c.set(tg, 12, 1);
    f.take_log();
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, m, 1, 1), 1);
    // (x, y) = 2·T − the unit = (18, 14); params 1–4 and flags 0x80.
    assert_eq!((f.entry_param(m, &e, 1), f.entry_param(m, &e, 2)), (18, 14));
    assert_eq!(f.entry_param(m, &e, 3), 1, "T type (monster)");
    assert_eq!(f.entry_param(m, &e, 4), f.c.units[tg].guid as i32);
    assert_eq!(f.entry_flags(m, &e), 0x80);
    // No free point: the used skill is cleared, 0.
    f.free_shift = None;
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, m, 1, 1), 0);
    assert!(f.take_log().contains(&format!("used {m} None")));
    // T none and no target position: used skill none, 0. With a position:
    // E param 4 := −1.
    f.c.units[m].used = Some(e);
    f.targets.clear();
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, m, 1, 1), 0);
    assert!(f.take_log().contains(&format!("used {m} None")));
    f.c.units[m].used = Some(e);
    f.tpos.insert(m, (30, 30));
    f.free_shift = Some((0, 0));
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, m, 1, 1), 1);
    assert_eq!(f.entry_param(m, &e, 4), -1);
    assert_eq!((f.entry_param(m, &e, 1), f.entry_param(m, &e, 2)), (30, 30));
}

#[test]
fn leap_do_by_phase() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(10);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    f.c.hostile = true;
    let m = monster(&mut f, (3, 0));
    f.c.units[m].flags = 0xC;
    f.scan = vec![m];
    // In flight and landing at the unit's position: landed, area damage.
    f.set_entry_flags(u, &e, 0x1101);
    f.set_entry_param_of(u, &e, 1, 0);
    f.set_entry_param_of(u, &e, 2, 0);
    f.take_log();
    assert_eq!(b3_lvl06::leap(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("deltimers {u} 1 0")), "type-1 timers");
    assert_eq!(f.entry_flags(u, &e), 0x200, "landed");
    assert!(
        log.contains(&format!("reaction {u} {m}")),
        "area damage reached the unit within calc1: {log:?}"
    );
    // Launch (0x80): the launch result (a path exists → 1; E flags 0x1101).
    f.set_entry_flags(u, &e, 0x80);
    f.set_entry_param_of(u, &e, 1, 9);
    f.set_entry_param_of(u, &e, 2, 9);
    assert_eq!(b3_lvl06::leap(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_flags(u, &e), 0x1101);
    // Otherwise a player's flags are cleared; return 1.
    f.set_entry_flags(u, &e, 0x200);
    assert_eq!(b3_lvl06::leap(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
    // Not yet landed in flight: 1 without area damage.
    f.set_entry_flags(u, &e, 0x1101);
    f.pos.insert(u, (1, 1));
    f.take_log();
    assert_eq!(b3_lvl06::leap(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(!f.take_log().iter().any(|s| s.starts_with("reaction")));
    // R invalid / no entry: 0.
    assert_eq!(b3_lvl06::leap(&mut f, &t, &ct, u, 99, 1), 0);
    f.c.units[u].skills.clear();
    assert_eq!(b3_lvl06::leap(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn double_swing_switches_target_on_odd_events() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m1) = hit_world();
    let m2 = monster(&mut f, (2, 0));
    for x in [m1, m2] {
        f.c.units[x].flags = 0xC;
        f.c.set(x, 12, 1);
    }
    f.scan = vec![m1, m2];
    f.c.units[u].mode = 0; // keeps the entries after apply_melee
                           // Even event: no flag change, the swing is at T.
    f.frame_index.insert(u, 0);
    assert_eq!(b3_lvl06::double_swing(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    assert!(stored_record(&f, u, m1).is_some());
    assert!(stored_record(&f, u, m2).is_none());
    // Odd event: flag 0x40, the next unit by GUID, path target := it.
    f.c.units[u].combat.clear();
    f.frame_index.insert(u, 1);
    f.take_log();
    assert_eq!(b3_lvl06::double_swing(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert!(f
        .take_log()
        .contains(&format!("path {u} TargetUnit(Some({m2}))")));
    assert!(stored_record(&f, u, m2).is_some());
    // Odd event with nobody else in range: the next unit wraps to the
    // smallest GUID above … only T itself remains → T itself is taken.
    f.scan = vec![];
    f.c.units[u].flags = 0;
    assert_eq!(b3_lvl06::double_swing(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40, "set before the lookup");
    // T none: 0.
    f.targets.clear();
    assert_eq!(b3_lvl06::double_swing(&mut f, &t, &ct, u, 1, 1), 0);
}

// -----------------------------------------------------------------

fn taunt_world() -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.auratargetstate = 27;
    r.auralencalc = c.f(100);
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2[0].isatt = true;
    let (mut f, u) = world();
    f.c.hostile = true;
    let m = monster(&mut f, (3, 0));
    f.c.units[m].flags = 0xE; // scan / curse unit flags …
    f.c.mflags.insert(m, 0); // … but no unique / super unique type flags
    f.c.set(m, 12, 1);
    f.targets.insert(u, m);
    f.scan = vec![m];
    (f, t, ct, u, m)
}

#[test]
fn taunt_curses_and_retargets() {
    let (mut f, t, ct, u, m) = taunt_world();
    f.c.frame = 700;
    f.take_log();
    assert_eq!(b3_lvl06::taunt(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    // The curse unit callback: AI state 12 for state 27; state list.
    assert!(log.contains(&format!("ai {m} 12")), "{log:?}");
    let l = f.state_list(m, 27).expect("taunt state list");
    assert_eq!(f.lists[l].callback, callback::AI_CURSE);
    // AI params unchanged, leash owner none, type-2 timers, path target.
    assert!(log
        .iter()
        .any(|s| s.starts_with("AiParams") && s.contains("-666")));
    assert!(log.iter().any(|s| s.starts_with("LeashOwner")));
    assert!(log.contains(&format!("deltimers {m} 2 0")));
    assert!(log.contains(&format!("schedule {m} 2 701 0 0")));
    assert!(log.contains(&format!("path {m} TargetUnit(Some({u}))")));
    // A target that fails the taunt test (not a monster) falls back to the
    // nearest accepted monster within 20.
    let (mut f, t, ct, u, m) = taunt_world();
    let p = f.add(FUnit::new(UnitType::Player, 0), (1, 0));
    f.targets.insert(u, p);
    f.scan = vec![p, m];
    assert_eq!(b3_lvl06::taunt(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.state_list(m, 27).is_some(), "the nearest monster");
    assert!(f.state_list(p, 27).is_none());
    // Nothing to taunt: 0. State out of range: 0. R invalid: 0.
    f.scan.clear();
    f.targets.clear();
    assert_eq!(b3_lvl06::taunt(&mut f, &t, &ct, u, 1, 1), 0);
    let mut r = body_rec();
    r.auratargetstate = 200;
    let t200 = tabs(r, Code::new(), 1);
    assert_eq!(b3_lvl06::taunt(&mut f, &t200, &ct, u, 1, 1), 0);
    assert_eq!(b3_lvl06::taunt(&mut f, &t, &ct, u, 99, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn blade_sentinel_is_placed_and_commanded() {
    let mut r = body_rec();
    r.summon = 0;
    r.summode = 1;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.pos.insert(u, (6, 8));
    f.c.set(u, 12, 17);
    f.tpos.insert(u, (20, 21));
    f.c.frame = 300;
    f.take_log();
    assert_eq!(b3_lvl06::blade_sentinel(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    let log = f.take_log();
    let m = 1; // the spawned sentinel
    assert_eq!(f.c.get(m, 12), 17, "base level := the unit's level");
    assert!(log.contains(&format!("place {m} Some(1) (6, 8)")));
    assert!(
        log.contains(&format!(
            "AiCommand {{ m: {m}, kind: 0, x: 6, y: 8, tx: 20, ty: 21 }}"
        )),
        "{log:?}"
    );
    assert!(log.contains(&format!("deltimers {m} 2 0")));
    assert!(log.contains(&format!("schedule {m} 2 301 0 0")));
    assert_eq!(f.c.units[m].flags & 0x8, 0, "flags &= ~0x8");
    // No target position: 0 after the flag. No summon class: 0 before.
    f.c.units[u].flags = 0;
    f.tpos.clear();
    assert_eq!(b3_lvl06::blade_sentinel(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    f.c.units[u].flags = 0;
    let mut r = body_rec();
    r.summon = 0xFFFF;
    let tn = tabs(r, Code::new(), 1);
    assert_eq!(b3_lvl06::blade_sentinel(&mut f, &tn, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    assert_eq!(b3_lvl06::blade_sentinel(&mut f, &t, &ct, u, 99, 1), 0);
}

#[test]
fn dragon_claw_start_and_do() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(35);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    assert_eq!(b3_lvl06::dragon_claw_start(&mut f, u), 1);
    f.c.units[u].mode = 0;
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.frame_index.insert(u, 1);
    assert_eq!(b3_lvl06::dragon_claw(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = stored_record(&f, u, m).expect("claw entry");
    assert_eq!(rec.enh_pct, 35, "claw_hit");
    assert_eq!(f.c.units[u].flags & 0x40, 0x40, "odd event: flag set");
    f.frame_index.insert(u, 2);
    f.c.units[u].combat.clear();
    b3_lvl06::dragon_claw(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0, "even event: flag cleared");
    f.targets.clear();
    assert_eq!(b3_lvl06::dragon_claw_start(&mut f, u), 0);
    assert_eq!(b3_lvl06::dragon_claw(&mut f, &t, &ct, u, 1, 1), 0);
}

// ---------------------------------------------------------------

#[test]
fn impale_wears_the_weapon_and_skips_the_physical_roll() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(40);
    r.calc2 = c.f(100);
    r.calc3 = c.f(5);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    let wpn = f.c.add_item(FItem {
        types: vec![45],
        durability: true,
        ..FItem::default()
    });
    f.c.units[u].weapon = Some(wpn);
    f.item_stats.insert((wpn, 72), 20);
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    assert_eq!(b3_lvl12::impale(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = stored_record(&f, u, m).expect("start_combat stored");
    assert_eq!(rec.result & 1, 1);
    assert_eq!(rec.hit_flags & 1, 1, "hit flags := 1 (skip the roll)");
    assert_eq!(
        f.item_stats[&(wpn, 72)],
        15,
        "wear(calc2 = 100 %, calc3 = 5)"
    );
    // A miss stores a miss record and wears nothing.
    let (mut f, u, _m) = hit_world();
    f.c.in_range = false;
    f.c.units[u].weapon = Some(wpn);
    f.c.items.push(FItem::default());
    f.item_stats.insert((wpn, 72), 20);
    assert_eq!(b3_lvl12::impale(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.item_stats[&(wpn, 72)], 20);
    // Not hostile, T none, R invalid: 0.
    let (mut f, u, _m) = hit_world();
    f.c.hostile = false;
    assert_eq!(b3_lvl12::impale(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.hostile = true;
    assert_eq!(b3_lvl12::impale(&mut f, &t, &ct, u, 99, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl12::impale(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn bone_wall_spawns_a_segment_and_two_wall_makers() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 0;
    r.summode = 1;
    r.pettype = 3;
    r.petmax = c.f(6);
    r.calc2 = c.f(8);
    r.srvmissilea = 1;
    let t = tabs(r, c, 2);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.tpos.insert(u, (20, 20));
    f.take_log();
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    let m = 1; // the first segment, at the target position
    assert_eq!(f.pos[&m], (20, 20));
    let log = f.take_log();
    assert!(
        log.contains(&"monster 1 (20, 20) 0 1 -1".to_string()),
        "{log:?}"
    );
    assert!(log.contains(&format!(
        "OwnerData {{ m: {m}, owner: Some({u}), a: 0, b: 1 }}"
    )));
    assert!(log.contains(&format!("Umod {{ m: {m}, umod: 15, arg: 0 }}")));
    assert!(log.contains(&format!("NodePrepend {{ u: {m}, slot: 9 }}")));
    assert!(log
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 3, max: 6")));
    // Two missiles perpendicular to the line of sight; n = 8 / 2 = 4.
    let targets: Vec<_> = f
        .missiles
        .iter()
        .map(|q| (q.target_x, q.target_y))
        .collect();
    assert_eq!(targets, [(40, 0), (0, 40)]);
    assert!(f
        .missiles
        .iter()
        .all(|q| q.flags == 0x21 && q.class == 1 && (q.x, q.y) == (20, 20)));
    assert_eq!(
        log.iter()
            .filter(|s| s.starts_with("MissileData28") && s.contains("v: 1"))
            .count(),
        2
    );
    assert_eq!(
        log.iter()
            .filter(|s| s.starts_with("MissileData2C") && s.contains("v: 4"))
            .count(),
        2
    );
    // The caster on the segment: dx = 1.
    let (mut f, u) = world();
    f.pos.insert(u, (20, 20));
    f.tpos.insert(u, (20, 20));
    b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3);
    let targets: Vec<_> = f
        .missiles
        .iter()
        .map(|q| (q.target_x, q.target_y))
        .collect();
    assert_eq!(targets, [(20, 21), (20, 19)]);
    // n ≤ 1 (calc2 = 2): the segment only; srvmissilea out of range too.
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 0;
    r.calc2 = c.f(2);
    r.srvmissilea = 1;
    let t2 = tabs(r, c, 2);
    let (mut f, u) = world();
    f.tpos.insert(u, (20, 20));
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t2, &ct, u, 1, 3), 1);
    assert!(f.missiles.is_empty());
    // Failures: skill 0 (after the flag), pettype ≥ count, no room /
    // town (before the flag), no class, no target position.
    let (mut f, u) = world();
    f.tpos.insert(u, (20, 20));
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 0, 3), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    f.pettypes = 3;
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3), 0);
    f.pettypes = 15;
    f.c.units[u].flags = 0;
    f.point_rooms.insert((20, 20), Some(5));
    f.town.insert(5);
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0, "room test before the flag");
    f.point_rooms.insert((20, 20), None);
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3), 0);
    f.point_rooms.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3), 0);
    f.tpos.insert(u, (20, 20));
    f.no_monsters = true;
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 3), 0);
    let mut r = body_rec();
    r.summon = 0xFFFF;
    let tn = tabs(r, Code::new(), 2);
    f.no_monsters = false;
    assert_eq!(b3_lvl12::bone_wall(&mut f, &tn, &ct, u, 1, 3), 0);
}

// -----------------------------------------------------------------

fn charge_world() -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(45);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    f.c.units[m].flags = 0xC;
    f.scan = vec![m];
    (f, t, ct, u, m)
}

#[test]
fn charge_do_hits_the_target_at_the_hit_frame() {
    let (mut f, t, ct, u, m) = charge_world();
    let e = f.c.units[u].used.unwrap();
    f.c.units[u].mode = 0; // keeps the stored entry after apply_melee
    f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
    f.set_entry_param_of(u, &e, 1, 1); // T type (monster)
    f.set_entry_param_of(u, &e, 2, f.c.units[m].guid as i32);
    f.set_entry_flags(u, &e, 0x1000);
    f.take_log();
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    // E flags := 0, params cleared, landing message, animation from the
    // hit frame (7, the fake has no sequence), flag 0x40.
    assert_eq!(f.entry_flags(u, &e), 0);
    assert_eq!((f.entry_param(u, &e, 1), f.entry_param(u, &e, 2)), (0, 0));
    assert!(log.iter().any(|s| s.starts_with("MsgA5")));
    assert!(log.contains(&format!("animfrom {u} 7")));
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    // A player: melee_result | 8, enhanced damage, hit class 0x70, the
    // overlay 147 on K.
    let rec = stored_record(&f, u, m).expect("entry");
    assert_eq!((rec.result & 9, rec.enh_pct, rec.hit_class), (9, 45, 0x70));
    assert!(log.contains(&format!("overlay {m} 147")));
    // A monster attacker: result := 9 without a roll.
    let (mut f, t, ct, u, m) = charge_world();
    let mm = monster(&mut f, (0, 1));
    f.c.units[mm].flags = 0xC;
    f.c.units[mm].mode = 0;
    f.c.set(mm, 12, 1);
    let em = SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    f.c.units[mm].used = Some(em);
    f.c.units[mm].skills.push(em);
    f.targets.insert(mm, u);
    f.set_entry_param_of(mm, &em, 1, 0);
    f.set_entry_param_of(mm, &em, 2, f.c.units[u].guid as i32);
    f.scan = vec![u];
    f.c.set(u, 12, 1);
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, mm, 1, 1), 1);
    let rec = stored_record(&f, mm, u).expect("monster entry");
    assert_eq!(rec.result & 9, 9);
    let _ = m;
    // No K (param 1 = 6) and nothing to find: timers, 0.
    let (mut f, t, ct, u, _m) = charge_world();
    let e = f.c.units[u].used.unwrap();
    f.set_entry_param_of(u, &e, 1, 6);
    f.scan.clear();
    f.c.frame = 50;
    f.take_log();
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 0);
    let log = f.take_log();
    assert!(log.contains(&format!("deltimers {u} 1 0")));
    assert!(log.contains(&format!("schedule {u} 1 51 0 0")));
    // No K but a unit near by: it is the target.
    let (mut f, t, ct, u, m) = charge_world();
    f.set_entry_param_of(u, &e, 1, 6);
    f.c.units[u].mode = 0;
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(stored_record(&f, u, m).is_some());
    // R invalid / no entry: 0.
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 99, 1), 0);
    f.c.units[u].used = None;
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn charge_do_while_moving() {
    let (mut f, t, ct, u, m) = charge_world();
    let e = f.c.units[u].used.unwrap();
    let setup = |f: &mut BodyFake, flags: u32| {
        f.set_entry_param_of(u, &e, 1, 1);
        f.set_entry_param_of(u, &e, 2, f.c.units[m].guid as i32);
        f.set_entry_flags(u, &e, flags);
        f.c.units[u].flags = 0x40;
    };
    // K in melee range: flags := 0, animation from f + 1, landing.
    setup(&mut f, 0x1001);
    f.take_log();
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert_eq!(f.entry_flags(u, &e), 0);
    assert!(log.contains(&format!("animfrom {u} 8")));
    assert!(log.iter().any(|s| s.starts_with("MsgA5")));
    // Out of range and still moving (flags & 2 = 0): the animation
    // restarts at frame 0 once F − (anim frame >> 8) ≥ f; type-0 timer at
    // F + 1 with arguments (1, 0); flag 0x40 cleared.
    f.c.in_range = false;
    setup(&mut f, 0x1001);
    f.c.frame = 100;
    f.anim_frame.insert(u, 0);
    f.take_log();
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("animfrom {u} 0")));
    assert_eq!(f.entry_flags(u, &e), 1);
    assert!(log.contains(&format!("deltimers {u} 0 0")));
    assert!(log.contains(&format!("schedule {u} 0 101 1 0")));
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    // Not yet at the frame: no restart, the timer is still set.
    setup(&mut f, 0x1001);
    f.c.frame = 100;
    f.anim_frame.insert(u, 100 << 8);
    f.take_log();
    b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1);
    let log = f.take_log();
    assert!(!log.contains(&format!("animfrom {u} 0")));
    assert_eq!(f.entry_flags(u, &e), 0x1001);
    assert!(log.contains(&format!("schedule {u} 0 101 1 0")));
    // Arrived without a target in range (flags & 2): K' by GUID order in
    // melee range takes over; else timers and 0.
    setup(&mut f, 0x1003);
    f.c.in_range = false;
    f.take_log();
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 0);
    let log = f.take_log();
    assert_eq!(f.entry_flags(u, &e), 0);
    assert!(log.contains(&format!("deltimers {u} 1 0")));
    assert!(log.contains(&format!("schedule {u} 1 101 0 0")));
    let m2 = monster(&mut f, (2, 0));
    f.c.units[m2].flags = 0xC;
    f.scan = vec![m, m2];
    f.c.in_range = true;
    setup(&mut f, 0x1003);
    // K = m is in range for r = 0 → the first rule (K in range) wins.
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 1);
    // Out of range for r = 0 only through K being absent: param 1 = 6.
    f.set_entry_param_of(u, &e, 1, 6);
    f.set_entry_param_of(u, &e, 2, f.c.units[m].guid as i32);
    f.set_entry_flags(u, &e, 0x1003);
    assert_eq!(b3_lvl12::charge(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 1), 1, "K' type");
    assert_eq!(
        f.entry_param(u, &e, 2),
        f.c.units[m2].guid as i32,
        "next GUID"
    );
}

#[test]
fn double_throw_boosts_the_missile() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(15);
    r.tohit = 100;
    let t = tabs(r, c, 3);
    let (mut f, u) = world();
    // No weapon: 0.
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 1, 1), 0);
    let wpn = f.c.add_item(FItem {
        throw: true,
        types: vec![45],
        ..FItem::default()
    });
    f.c.units[u].weapon = Some(wpn);
    f.c.units[u].items.insert(4, wpn);
    f.item_stats.insert((wpn, 70), 5);
    f.tpos.insert(u, (9, 9));
    // Missile type 0 or ≥ count: 0.
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 1, 1), 0);
    f.item_missiles.insert(wpn, 3);
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 1, 1), 0);
    f.item_missiles.insert(wpn, 2);
    f.take_log();
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 1, 1), 1);
    let q = f.missiles[0];
    assert_eq!((q.class, q.flags), (2, 0x21), "a straight missile");
    let m = f.c.units.len() - 1;
    let log = f.take_log();
    assert!(log.contains(&format!("add {m} 19 100")), "tohit += to_hit");
    assert!(
        log.contains(&format!("add {m} 25 15")),
        "damagepercent += calc1"
    );
    assert_eq!(f.item_stats[&(wpn, 70)], 4, "quant 1: one thrown");
    // A missile potion (item type 38): lob.
    f.missiles.clear();
    f.c.items[wpn].types = vec![45, 38];
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.missiles[0].flags, 0x420);
    // No missile made: still 1, nothing added.
    f.no_missiles = true;
    f.take_log();
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 1, 1), 1);
    assert!(!f.take_log().iter().any(|s| s.starts_with("add")));
    assert_eq!(b3_lvl12::double_throw(&mut f, &t, u, 99, 1), 0);
}

#[test]
fn find_item_start_tests_the_corpse_without_soft() {
    let mut ms2: d2_data::tables::Monstats2 = crate::skills::fake::blank();
    ms2.corpsesel = true; // `soft` stays false
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    assert_eq!(b3_lvl12::find_item_start(&mut f, &ct, u), 0, "T none");
    let k = monster(&mut f, (3, 3));
    f.targets.insert(u, k);
    assert_eq!(b3_lvl12::find_item_start(&mut f, &ct, u), 0, "alive");
    f.c.units[k].mode = 12;
    assert_eq!(b3_lvl12::find_item_start(&mut f, &ct, u), 1);
    // No corpseSel: 0.
    let ct0 = combat_tables(vec![monster_rec()]);
    assert_eq!(b3_lvl12::find_item_start(&mut f, &ct0, u), 0);
    // A udead-group state: 0.
    f.state_flags.insert((77, super::helpers::group::UDEAD));
    f.c.units[k].states.push(77);
    assert_eq!(b3_lvl12::find_item_start(&mut f, &ct, u), 0);
}

// -----------------------------------------------------------------

fn cloak_world() -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 80;
    r.auralencalc = c.f(200);
    r.passivestat1 = 90;
    r.passivecalc1 = c.f(7);
    r.passivestat2 = 91;
    r.passivecalc2 = c.f(0);
    r.passivestat3 = 92;
    r.passivecalc3 = c.f(3);
    r.auratargetstate = 81;
    r.aurastat1 = 100;
    r.aurastatcalc1 = c.f(11);
    r.aurastat2 = 101;
    r.aurastatcalc2 = c.f(0);
    r.aurarangecalc = c.f(10);
    r.aurafilter = 0x8783;
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2[0].isatt = true;
    let (mut f, u) = world();
    f.c.hostile = true;
    let m = monster(&mut f, (3, 0));
    f.c.units[m].flags = 0xE;
    f.c.mflags.insert(m, 0);
    f.scan = vec![m];
    (f, t, ct, u, m)
}

#[test]
fn cloak_of_shadows_self_state_and_curse() {
    let (mut f, t, ct, u, m) = cloak_world();
    f.c.frame = 1000;
    f.take_log();
    assert_eq!(b3_lvl12::cloak(&mut f, &t, &ct, u, 1, 4), 1);
    // The caster's own list: stat 90 := 7, 91 skipped (0), 92 := 3, then
    // 350 := skill and 351 := L; the state is marked changed.
    let l = f.state_list(u, 80).expect("self state");
    assert_eq!(f.lists[l].expire, 1200);
    assert_eq!(f.lists[l].callback, callback::DEFAULT);
    assert_eq!((f.list_get(l, 90), f.list_get(l, 92)), (7, 3));
    assert!(!f.lists[l].stats.contains_key(&91), "a 0 value is skipped");
    assert_eq!((f.list_get(l, 350), f.list_get(l, 351)), (1, 4));
    let log = f.take_log();
    assert!(log.contains(&format!("changed {u} 80")));
    // The target: stat 100 := 11 (stats[1] > 0), 101 := 0 is set anyway,
    // the AI curse callback, AI special state 10.
    let lm = f.state_list(m, 81).expect("curse state");
    assert_eq!(f.lists[lm].callback, callback::AI_CURSE);
    assert_eq!(f.list_get(lm, 100), 11);
    assert!(f.lists[lm].stats.contains_key(&101), "also a 0 value");
    assert!(log.contains(&format!("ai {m} 10")));
    // Already in the state: 0 and nothing changes.
    let n = f.lists.len();
    assert_eq!(b3_lvl12::cloak(&mut f, &t, &ct, u, 1, 4), 0);
    assert_eq!(f.lists.len(), n);
    // A dead unit in range is skipped.
    let (mut f, t, ct, u, m) = cloak_world();
    f.dead.insert(m);
    f.c.units[m].mode = 1;
    assert_eq!(b3_lvl12::cloak(&mut f, &t, &ct, u, 1, 4), 1);
    assert!(f.state_list(m, 81).is_none());
    // auratargetstate out of range: 0 but the caster keeps its state.
    let (mut f, _t, ct, u, m) = cloak_world();
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 80;
    r.auralencalc = c.f(200);
    r.auratargetstate = 250;
    let t2 = tabs(r, c, 1);
    assert_eq!(b3_lvl12::cloak(&mut f, &t2, &ct, u, 1, 4), 0);
    assert!(f.state_list(u, 80).is_some(), "the caster keeps the state");
    assert!(f.state_list(m, 81).is_none());
    // aurastate out of range / R invalid: 0.
    let mut r = body_rec();
    r.aurastate = 200;
    let t3 = tabs(r, Code::new(), 1);
    assert_eq!(b3_lvl12::cloak(&mut f, &t3, &ct, u, 1, 4), 0);
    assert_eq!(b3_lvl12::cloak(&mut f, &t2, &ct, u, 99, 4), 0);
}

// ---------------------------------------------------------------- edge cases

#[test]
fn edge_sacrifice_without_a_stored_entry_applies_nothing() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc2 = c.f(8);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.hostile = true;
    f.c.set(u, 6, 5000);
    let life = f.c.get(u, 6);
    f.take_log();
    assert_eq!(b3_lvl01::sacrifice(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.take_log().is_empty(), "no apply, no reaction");
    assert_eq!(f.c.get(u, 6), life);
    assert!(f.c.reactions.is_empty());
}

#[test]
fn edge_failed_chance_still_uses_the_corpse() {
    // Find Potion: chance 0 → returns 1; Find Item: returns 0; both mark
    // the corpse (state 118) before the draw.
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(0);
    let t = tabs(r, c, 1);
    let mut ms2: d2_data::tables::Monstats2 = crate::skills::fake::blank();
    ms2.corpsesel = true;
    ms2.soft = true;
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2 = vec![ms2];
    for find_item in [false, true] {
        let (mut f, u) = world();
        let k = monster(&mut f, (3, 3));
        f.c.units[k].mode = 12;
        f.targets.insert(u, k);
        let seed0 = f.c.units[u].seed;
        let ret = if find_item {
            b3_lvl12::find_item(&mut f, &t, &ct, u, 1, 1)
        } else {
            b3_lvl01::find_potion(&mut f, &t, &ct, u, 1, 1)
        };
        assert_eq!(ret, i32::from(!find_item));
        assert!(f.c.has_state(k, 118), "used up either way");
        assert_eq!(f.c.units[u].seed, stepped(seed0), "one chance draw");
        assert!(!f
            .take_log()
            .iter()
            .any(|s| s.starts_with("DropItem") || s.starts_with("TreasureDrop")));
    }
}

#[test]
fn edge_charged_bolt_returns_1_firestorm_returns_0() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(0);
    r.srvmissilea = 1;
    let t = tabs(r, c, 2);
    let (mut f, u) = world();
    f.tpos.insert(u, (12, 7));
    assert_eq!(b3_lvl01::charged_bolt(&mut f, &t, u, 1, 1), 1, "n ≤ 0");
    assert_eq!(b3_lvl01::firestorm(&mut f, &t, u, 1, 1), 0, "n ≤ 0");
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(2);
    r.srvmissilea = 1;
    let t2 = tabs(r, c, 2);
    f.tpos.clear();
    assert_eq!(
        b3_lvl01::charged_bolt(&mut f, &t2, u, 1, 1),
        1,
        "no target position"
    );
}

#[test]
fn edge_shock_field_reseeds_the_caster_from_the_target_x() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.progressive = false;
    r.prgcalc1 = c.f(1);
    r.aurarangecalc = c.f(5);
    let t = tabs(r, c, 2);
    for tx in [12, 33] {
        let (mut f, u) = world();
        f.tpos.insert(u, (tx, 7));
        assert_eq!(b3_lvl06::shock_field(&mut f, &t, u, 1, 1), 1);
        assert_eq!(
            f.c.units[u].seed,
            Seed::init_low(tx as u32),
            "n = 1: the seed is the re-seed itself"
        );
    }
}

#[test]
fn edge_corpse_explosion_outer_ring_and_corpse_seed() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurarangecalc = c.f(7);
    r.calc1 = c.f(50);
    r.calc2 = c.f(100);
    r.calc3 = c.f(50);
    r.etype = 1;
    r.aurafilter = 0x8783;
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.minhp = 10;
    ms.maxhp = 20;
    let mut ms2: d2_data::tables::Monstats2 = crate::skills::fake::blank();
    ms2.corpsesel = true;
    ms2.soft = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    f.monlvl = vec![crate::skills::fake::blank::<d2_data::tables::Monlvl>(); 1];
    f.monlvl[0].hp = 100;
    let k = monster(&mut f, (10, 10));
    f.c.units[k].mode = 12;
    f.targets.insert(u, k);
    f.c.hostile = true;
    let near = monster(&mut f, (13, 10));
    let far = monster(&mut f, (14, 10));
    for x in [near, far] {
        f.c.units[x].flags = 0xC;
        f.c.set(x, 6, 1_000_000);
    }
    f.scan = vec![near, far];
    // h = (10 + 20) << 7 = 3840; lo = 50 % = 1920, hi = 3840: one draw
    // roll(1920) on the corpse's seed, none on the caster's.
    let mut s = f.c.units[k].seed;
    let x = s.roll(1920) as i32;
    let useed = f.c.units[u].seed;
    assert_eq!(b3_lvl06::corpse_explosion(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[k].seed, s, "T's seed");
    assert_eq!(f.c.units[u].seed, useed, "not the caster's");
    let v = 1920 + x;
    let of = |d: usize| f.c.reactions.iter().find(|r| r.1 == d).unwrap().2;
    // p = 50 %: fire = pct(v, 50), physical = pct(v, 50) inside r1² = 9.
    assert_eq!(of(near).fire, v / 2);
    assert_eq!(of(near).physical, v / 2);
    // Range 7 (odd): = 3, = 4; d² = 16 > 9 → the element only.
    assert_eq!(of(far).fire, v / 2);
    assert_eq!(of(far).physical, 0);
}

#[test]
fn edge_double_swing_does_not_test_r() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, _m) = hit_world();
    f.frame_index.insert(u, 0);
    // An invalid skill: the Bash start refuses (0), the do still returns 1.
    assert_eq!(b3_lvl06::double_swing(&mut f, &t, &ct, u, 99, 1), 1);
    assert!(f.c.units[u].combat.is_empty());
}

#[test]
fn edge_monster_leap_attacks_with_the_a1_damage_first() {
    let (mut f, m, l, t, ct) = mode_world(|_| {});
    let tg = monster(&mut f, (14, 12));
    let e = SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    f.c.units[m].used = Some(e);
    f.c.units[m].skills.push(e);
    f.targets.insert(m, tg);
    f.free_shift = Some((0, 0));
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(m, 12, 1);
    f.c.set(tg, 12, 1);
    assert_eq!(b3_lvl06::leap_start(&mut f, &t, &ct, m, 1, 1), 1);
    assert_eq!(stats3(&f, l), (10, 20, 100), "mode_damage(unit, 4)");
    assert!(
        f.c.log.iter().any(|l| l.starts_with("event ")),
        "the pre-hit was started and applied (melee events)"
    );
}

#[test]
fn edge_hit_frame_rereads_record_zero() {
    let (mut f, u) = world();
    // Any record with a non-zero event byte in record 0 → 7, whatever the
    // later records hold; a zero event byte in record 0 → −1 even when
    // a later record has an event.
    f.sequence = Some(vec![[0, 0, 0, 0, 0, 1], [0; 6], [0; 6]]);
    assert_eq!(hit_frame(&f, u), 7);
    f.sequence = Some(vec![[0; 6], [0, 0, 0, 0, 0, 1], [0, 0, 0, 0, 0, 1]]);
    assert_eq!(hit_frame(&f, u), -1);
    f.sequence = Some(vec![]);
    assert_eq!(hit_frame(&f, u), 7, "count ≤ 0");
    f.sequence = None;
    assert_eq!(hit_frame(&f, u), 7, "no sequence");
}

#[test]
fn edge_charge_baseid_436_has_no_early_exit() {
    // A moving monster that arrived (flags & 2) with nobody near: the
    // normal exit (landing, timers, 0) for BaseId 436 and for any other
    // class alike.
    let run = |baseid: u16| {
        let mut ms = monster_rec();
        ms.baseid = baseid;
        let ct = combat_tables(vec![ms]);
        let mut c = Code::new();
        let mut r = body_rec();
        r.calc1 = c.f(10);
        let t = tabs(r, c, 1);
        let (mut f, _p) = world();
        let m = monster(&mut f, (5, 5));
        let e = SkillEntry {
            skill: 1,
            base: 1,
            owner_guid: -1,
            ..SkillEntry::default()
        };
        f.c.units[m].used = Some(e);
        f.c.units[m].skills.push(e);
        f.set_entry_param_of(m, &e, 1, 6);
        f.set_entry_flags(m, &e, 0x1003);
        f.c.frame = 40;
        f.take_log();
        let ret = b3_lvl12::charge(&mut f, &t, &ct, m, 1, 1);
        (ret, f.take_log(), f.entry_flags(m, &e))
    };
    let a = run(436);
    assert_eq!(a, run(0));
    assert_eq!(a.0, 0);
    assert_eq!(a.2, 0);
}

#[test]
fn edge_cloak_stat_zero_is_no_stat() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 80;
    r.auralencalc = c.f(200);
    r.auratargetstate = 81;
    r.aurastat1 = 0; // strength: treated as "no stat"
    r.aurastatcalc1 = c.f(11);
    r.aurastat2 = 101;
    r.aurastatcalc2 = c.f(4);
    r.aurarangecalc = c.f(10);
    r.aurafilter = 0x8783;
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2[0].isatt = true;
    let (mut f, u) = world();
    f.c.hostile = true;
    let m = monster(&mut f, (3, 0));
    f.c.units[m].flags = 0xE;
    f.c.mflags.insert(m, 0);
    f.scan = vec![m];
    assert_eq!(b3_lvl12::cloak(&mut f, &t, &ct, u, 1, 4), 1);
    let lm = f.state_list(m, 81).expect("curse state");
    assert!(!f.lists[lm].stats.contains_key(&0), "stat 0 is not set");
    assert_eq!(f.list_get(lm, 101), 4, "stats 2…6 are");
}

#[test]
fn edge_impale_passes_the_raw_srcdam() {
    let ct = combat_tables(vec![monster_rec()]);
    let run = |srcdam: u8| {
        let mut c = Code::new();
        let mut r = body_rec();
        r.calc1 = c.f(0);
        r.calc2 = c.f(0);
        r.calc3 = c.f(0);
        r.srcdam = srcdam;
        let t = tabs(r, c, 1);
        let (mut f, u, m) = hit_world();
        f.c.set(u, 21, 10);
        f.c.set(u, 22, 10);
        f.c.units[u].seed = hit_seed(&f, &t, &ct, u, m);
        assert_eq!(b3_lvl12::impale(&mut f, &t, &ct, u, 1, 1), 1);
        stored_record(&f, u, m).expect("entry").physical
    };
    assert_eq!(run(0), 0, "SrcDam 0 gives no physical damage");
    assert!(run(128) > 0, "SrcDam 128 does");
}

#[test]
fn edge_conversion_remove_callbacks_differ_in_the_max_life_shift() {
    for mind_blast in [false, true] {
        let (mut f, _u) = world();
        let m = monster(&mut f, (2, 2));
        // Converted at level 40 → 20: saved level 40, maximum 400 points.
        f.c.set(m, 12, 20);
        f.c.set(m, 7, 200 << 8);
        f.c.set(m, 6, 100 << 8);
        give_list(&mut f, m, 53, &[]);
        let sl = give_list(&mut f, m, 109, &[(176, 40), (177, 400)]);
        remove_conversion(&mut f, m, 53, mind_blast);
        // v = pct(400, h 100, m 200) = 200; base level := 40; stat 6 := v << 8.
        assert_eq!(f.c.get(m, 12), 40);
        assert_eq!(f.c.get(m, 6), 200 << 8);
        assert_eq!(
            f.c.get(m, 7),
            if mind_blast { 400 << 8 } else { 400 },
            "Conversion stores the unshifted value (Edge case 22)"
        );
        assert!(f.lists[sl].freed, "the saved-stats list is freed");
        assert!(!f.c.has_state(m, 53));
    }
}

#[test]
fn edge_monster_whirlwind_pacing_is_a_coin_flip() {
    let (mut f, _u) = world();
    let m = monster(&mut f, (2, 2));
    let e = SkillEntry {
        skill: 1,
        ..SkillEntry::default()
    };
    for lo in 1u32..40 {
        f.c.units[m].seed = Seed::new(lo, 0);
        let mut s = Seed::new(lo, 0);
        let want = ((s.step() & 1) ^ 1) as i32;
        assert_eq!(ww_pacing(&mut f, m, &e), want);
        assert!(want == 0 || want == 1);
        assert_eq!(f.c.units[m].seed, s, "exactly one step");
    }
}

#[test]
fn edge_blade_shield_pulse_is_not_a_table_slot() {
    // Table slot srvdo 142 has no body of its own: the pulse is called
    // directly by srvdo 54.
    assert!(!DO_BODIES.contains(&142));
    assert!(DO_BODIES.contains(&54));
}

// ---------------------------------------------------------------- edge cases of bodies-2b

/// A formula `rand(a, b)`: push a, push b, call function 2, end.
fn rand_formula(c: &mut Code, a: i16, b: i16) -> u32 {
    let at = c.0.len() as u32;
    c.0.push(0x08);
    c.0.extend(a.to_le_bytes());
    c.0.push(0x08);
    c.0.extend(b.to_le_bytes());
    c.0.extend([0x01, 2, 0x00]);
    at
}

/// A switchable monster class (`SwitchAI`, attackable) and a world with a
/// hostile player and one such monster in scan range.
fn switch_world(
    edit: impl FnOnce(&mut d2_data::tables::Skills, &mut Code),
) -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut c = Code::new();
    let mut r = body_rec();
    edit(&mut r, &mut c);
    let t = tabs(r, c, 2);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2[0].isatt = true;
    let (mut f, u) = world();
    f.c.hostile = true;
    f.c.in_range = true;
    let m = monster(&mut f, (3, 0));
    f.c.units[m].flags = 0xE;
    f.c.mflags.insert(m, 0);
    f.c.set(m, 12, 1);
    f.c.set(u, 12, 1);
    f.scan = vec![m];
    (f, t, ct, u, m)
}

#[test]
fn edge_leap_attack_start_does_nothing_for_a_state_54_target() {
    let (mut f, _t, _ct, u, m) = switch_world(|_, _| {});
    f.c.in_range = false;
    f.pos.insert(m, (10, 0));
    f.c.units[m].states.push(54);
    f.targets.insert(u, m);
    f.take_log();
    assert_eq!(b3_lvl18::leap_attack_start(&mut f, u, 1), 1);
    let e = f.c.units[u].used.unwrap();
    assert_eq!(f.entry_flags(u, &e), 0x1080);
    assert_eq!(
        (f.entry_param(u, &e, 3), f.entry_param(u, &e, 4)),
        (1, m as i32)
    );
    assert_eq!(f.c.units[m].states, [54], "the target is untouched");
    let log = f.take_log();
    for p in [
        "state",
        "update",
        "schedule",
        "path",
        "deltimers",
        "overlay",
        "event",
    ] {
        assert!(
            !log.iter().any(|l| l.starts_with(&format!("{p} {m} "))),
            "{p}: {log:?}"
        );
    }
}

#[test]
fn edge_confuse_slot_one_gives_no_stat() {
    let (mut f, t, ct, u, m) = switch_world(|r, c| {
        r.auratargetstate = 81;
        r.auralencalc = c.f(100);
        r.aurarangecalc = c.f(10);
        r.aurafilter = 0x8783;
        r.aurastat1 = 100; // never read: the context starts at aurastat2
        r.aurastatcalc1 = c.f(9);
        r.aurastat2 = 101;
        r.aurastatcalc2 = c.f(4);
    });
    f.tpos.insert(u, (3, 0));
    f.c.frame = 20;
    f.take_log();
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.state_list(m, 81).expect("confuse state");
    assert_eq!(f.lists[l].callback, callback::CONFUSE);
    assert!(!f.lists[l].stats.contains_key(&100), "aurastat1 is skipped");
    assert!(!f.lists[l].stats.contains_key(&0), "slot 1 is (stat 0, 0)");
    assert_eq!(f.list_get(l, 101), 4);
    let log = f.take_log();
    assert!(log.contains(&format!("Alignment {{ u: {m}, a: 1, v: 1 }}")));
    assert!(log.contains(&format!("schedule {m} 10 120 0 0")));
}

#[test]
fn edge_dragon_tail_explodes_when_the_kicker_is_alive() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(50);
    r.aurarangecalc = c.f(6);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    for dead in [false, true] {
        let (mut f, u, m) = hit_world();
        let x = monster(&mut f, (2, 0));
        f.c.units[x].flags = 0xC;
        f.c.set(x, 6, 1_000_000);
        f.c.set(x, 12, 1);
        f.scan = vec![x];
        f.c.set(m, 6, 1_000_000);
        let rec = DamageRecord {
            result: 1,
            physical: 2560,
            total: 2560,
            ..DamageRecord::default()
        };
        super::fake::stored(&mut f, u, m, rec);
        // The alive test is on the kicker: a dead target
        // still gets the explosion, a dead kicker never does.
        for who in [m, u] {
            let mut f = f.clone();
            if dead {
                f.dead.insert(who);
            }
            f.take_log();
            assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 1, 1), 1);
            let hit = f.c.reactions.iter().any(|r| r.1 == x);
            let want = !(dead && who == u);
            assert_eq!(
                hit,
                want,
                "dead: {dead}, dead unit is the kicker: {}",
                who == u
            );
        }
    }
}

#[test]
fn edge_rabies_start_evaluates_and_discards() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = rand_formula(&mut c, 0, 100);
    r.calc4 = rand_formula(&mut c, 0, 100);
    r.etype = 1;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u, m) = hit_world();
    let e = f.c.units[u].used.unwrap();
    // A seed for which the hit (bonus 0, as the start rolls it) lands.
    let s0 = (1u32..)
        .map(|lo| Seed::new(lo, 0))
        .find(|&s| {
            let mut g = f.clone();
            g.c.units[u].seed = s;
            crate::combat::melee_result(&mut g.c, &t, &ct, Some(u), Some(m), 0, 0) & 1 != 0
        })
        .unwrap();
    f.c.units[u].seed = s0;
    // The seed after the melee roll, then the two discarded evaluations.
    let mut g = f.clone();
    crate::combat::melee_result(&mut g.c, &t, &ct, Some(u), Some(m), 0, 0);
    let mut want = g.c.units[u].seed;
    want.step();
    want.step();
    assert_eq!(b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 1), 1);
    assert_eq!(f.c.units[u].seed, want, "calc1 and calc4 were evaluated");
    assert!(f.c.units[u].combat.is_empty(), "no damage record");
    // A miss: 0 and E param 1 stays 0.
    let (mut f, u, _m) = hit_world();
    f.c.in_range = false;
    assert_eq!(b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.entry_param(u, &e, 1), 0);
}

#[test]
fn edge_teleport_tests_neither_r_nor_the_position() {
    let (mut f, u) = world();
    f.teleport = Some(1);
    f.take_log();
    // No target position at all: the unit is placed at (0, 0).
    assert_eq!(b3_lvl18::teleport(&mut f, u), 1);
    assert!(f.take_log().contains(&format!("place {u} None (0, 0)")));
    f.teleport = Some(0);
    assert_eq!(b3_lvl18::teleport(&mut f, u), 0);
    f.teleport = Some(2);
    f.blocked = true;
    assert_eq!(b3_lvl18::teleport(&mut f, u), 0, "line of sight");
}

#[test]
fn edge_mind_blast_converts_on_equal_conversion_does_not() {
    let k = 40;
    let seed = (1u32..)
        .map(|lo| Seed::new(lo, 0))
        .find(|s| {
            let mut s = *s;
            s.roll(100) == k
        })
        .unwrap();
    // Conversion (srvdo 79): `roll(100) < calc1`, equal fails.
    let (mut f, t, ct, u, m) = switch_world(|r, c| {
        r.calc1 = c.f(k as i16);
        r.auratargetstate = 60;
        r.auralencalc = c.f(100);
    });
    f.c.units[u].seed = seed;
    f.targets.insert(u, m);
    super::fake::stored(
        &mut f,
        u,
        m,
        DamageRecord {
            result: 1,
            ..DamageRecord::default()
        },
    );
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.state_list(m, 60).is_none(), "equal is not a conversion");
    // Mind Blast (srvdo 51): `roll(100) ≤ chance`, equal converts.
    let (mut f, t, ct, u, m) = switch_world(|r, c| {
        r.param3 = 10;
        r.param4 = 5;
        r.param5 = k as i32 as _;
        r.param6 = k as i32 as _;
        r.aurarangecalc = c.f(10);
        r.aurafilter = 0x8783;
    });
    f.tpos.insert(u, (3, 3));
    f.c.units[u].seed = seed;
    assert_eq!(b3_lvl24::mind_blast(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.state_list(m, 53).expect("conversion state");
    assert_eq!(f.lists[l].callback, callback::MIND_BLAST);
}

#[test]
fn edge_strafe_last_arrow_returns_0() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.calc2 = c.f(3);
    r.aurarangecalc = c.f(10);
    r.aurafilter = 0x8783;
    let t = tabs(r, c, 2);
    let ct = combat_tables(vec![monster_rec()]);
    for (n, want, left) in [(2, 1, 1), (1, 0, 0)] {
        let (mut f, u, m) = hit_world();
        f.c.units[m].flags = 0xC;
        f.scan = vec![m];
        let e = f.c.units[u].used.unwrap();
        f.set_entry_param_of(u, &e, 1, n);
        f.set_entry_param_of(u, &e, 2, 1);
        f.set_entry_param_of(u, &e, 3, f.c.units[m].guid as i32);
        assert_eq!(b3_lvl24::strafe(&mut f, &t, &ct, u, 1, 1), want);
        assert_eq!(f.missiles.len(), 1, "the arrow is made either way");
        assert_eq!(f.entry_param(u, &e, 1), left);
    }
}

#[test]
fn edge_dopplezon_keeps_a_bad_pettype_as_0_raven_refuses() {
    let mut r = body_rec();
    r.summon = 0;
    r.pettype = 20; // ≥ the pettype count 15
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.monlvl = vec![crate::skills::fake::blank::<d2_data::tables::Monlvl>(); 4];
    f.take_log();
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(
        f.take_log()
            .iter()
            .any(|s| s.starts_with("PetAdd") && s.contains("t: 0")),
        "pet type kept as 0"
    );
    assert_eq!(
        b3_lvl01::raven(&mut f, &t, &ct, u, 1, 1),
        0,
        "Raven refuses"
    );
}

#[test]
fn edge_revive_goes_on_without_a_free_point() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.pettype = 3;
    r.summon = 0;
    r.petmax = c.f(4);
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2[0].revive = true;
    ct.monstats2[0].corpsesel = true;
    let (mut f, u) = world();
    let k = monster(&mut f, (6, 6));
    f.c.units[k].mode = 12;
    f.alive.remove(&k);
    f.c.units[k].flags = 4; // a dead unit still passes can_switch
    f.c.mflags.insert(k, 0);
    f.targets.insert(u, k);
    f.free_shift = None; // no free point
    f.take_log();
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(!log.iter().any(|s| s.starts_with("place ")), "not stood up");
    assert!(f.c.has_state(k, 96), "revive state still set");
    assert!(log
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 3")));
    assert!(log
        .iter()
        .any(|s| s.starts_with("OwnerData") && s.contains(&format!("owner: Some({u})"))));
}

#[test]
fn edge_redemption_pays_for_corpses_fist_accepts_overlay_0() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 40;
    r.aurarangecalc = c.f(5);
    r.calc1 = c.f(100);
    r.calc2 = c.f(1);
    r.calc3 = c.f(0);
    r.mana = 1;
    r.manashift = 8;
    r.perdelay = 0xFFFF_FFFF;
    r.aurafilter = 0x1002;
    let t = tabs(r, c, 1);
    let mut ms2: d2_data::tables::Monstats2 = crate::skills::fake::blank();
    ms2.corpsesel = true;
    ms2.soft = true;
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    f.c.set(u, 8, 1000);
    f.c.set(u, 9, 5000);
    f.c.set(u, 7, 5000);
    let k = monster(&mut f, (2, 2));
    f.c.units[k].mode = 12;
    f.scan = vec![k];
    assert_eq!(b3_lvl30::redemption(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.c.has_state(k, 99), "redeemed corpse counted");
    assert_eq!(f.c.get(u, 8), 1000 - 256, "the mana cost is paid");
    // Fist of the Heavens: an overlay id of 0 is accepted.
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.srvoverlay = 0;
    let t2 = tabs(r, Code::new(), 2);
    let (mut f, u) = world();
    let m = monster(&mut f, (4, 4));
    f.targets.insert(u, m);
    f.take_log();
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t2, u, 1, 1), 1);
    assert!(f.take_log().contains(&format!("overlay {m} 0")));
}
