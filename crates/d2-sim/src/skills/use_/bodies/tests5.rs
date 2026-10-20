//! Tests of the batch 3 bodies of required level 30 on
//! [`super::fake::BodyFake`].

use super::fake::BodyFake;
use super::tests2::{body_rec, monster, tabs, world, Code};
use super::*;
use crate::rng::Seed;
use crate::skills::fake::{blank, combat_tables, monster_rec};
use crate::skills::SkillUnits;
use d2_data::tables::{Monlvl, Monstats2};

/// Combat tables with `n` monster classes.
fn classes(n: usize) -> crate::combat::CombatTables {
    combat_tables(vec![monster_rec(); n])
}

/// A seed word whose first step gives a roll below 5: every hit test
/// (chance 5…95) hits.
fn hit_lo() -> u32 {
    (0u32..)
        .find(|&lo| Seed::new(lo, 0).step() % 100 < 5)
        .unwrap()
}

/// The caster (unit 0) hostile to and in melee range of a monster at
/// (1, 0) that is its target; both level 10; the caster's next hit test
/// hits.
fn hit_world() -> (BodyFake, usize, usize) {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(u, 12, 10);
    f.c.set(m, 12, 10);
    f.c.set(u, 19, 100_000);
    f.c.units[u].seed = Seed::new(hit_lo(), 0);
    (f, u, m)
}

fn has(log: &[String], s: &str) -> bool {
    log.iter().any(|l| l == s)
}

fn pos(log: &[String], s: &str) -> usize {
    log.iter()
        .position(|l| l == s)
        .unwrap_or_else(|| panic!("{s} not in {log:?}"))
}

// -----------------------------------------------------------------

fn valkyrie_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 4;
    r.summode = 2;
    r.pettype = 6;
    r.petmax = c.f(3);
    r.calc2 = c.f(7);
    tabs(r, c, 1)
}

#[test]
fn valkyrie_refusals_and_pet_type() {
    let t = valkyrie_tables();
    let ct = classes(8);
    let (mut f, u) = world();
    // R invalid.
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, u, 9, 3), 0);
    // The unit not a player.
    let mon = monster(&mut f, (5, 5));
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, mon, 1, 3), 0);
    // `summon_class` < 0.
    let mut t2 = valkyrie_tables();
    t2.skills[1].summon = 0xFFFF;
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t2, &ct, u, 1, 3), 0);
    assert!(f.take_log().iter().all(|l| !l.starts_with("monster")));
    // pettype (unsigned) ≥ count → 0.
    let mut t3 = valkyrie_tables();
    t3.skills[1].pettype = 200;
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t3, &ct, u, 1, 3), 1);
    let log = f.take_log();
    assert!(log
        .iter()
        .any(|l| l.contains("PetAdd") && l.contains("t: 0,")));
    t3.skills[1].pettype = 6;
    b3_lvl30::valkyrie(&mut f, &t3, &ct, u, 1, 3);
    assert!(f
        .take_log()
        .iter()
        .any(|l| l.contains("PetAdd") && l.contains("t: 6,")));
}

#[test]
fn valkyrie_sets_unit_flag_40() {
    let t = valkyrie_tables();
    let ct = classes(8);
    let (mut f, u) = world();
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, u, 1, 3), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
}

#[test]
fn valkyrie_spawn_request() {
    let t = valkyrie_tables();
    let ct = classes(8);
    let (mut f, u) = world();
    // Flags 0, no position given: the target position, else (0, 0).
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, u, 1, 3), 1);
    let log = f.take_log();
    assert!(has(&log, "monster 1 (0, 0) 4 2 -1"), "{log:?}");
    assert!(log
        .iter()
        .any(|l| l.contains("PetAdd { owner: 0, pet: 1, t: 6, max: 3 }")));
    assert!(log
        .iter()
        .any(|l| l.contains("OwnerData { m: 1, owner: Some(0)")));
    // A failed spawn → 0.
    f.no_monsters = true;
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, u, 1, 3), 0);
}

#[test]
fn valkyrie_stats_use_the_item_level_calc() {
    let t = valkyrie_tables();
    let ct = classes(8);
    let (mut f, u) = world();
    f.c.set(u, 12, 40);
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, u, 1, 3), 1);
    // base_stats(game, unit, m, 0, L): p = L + 40 · 3 / 4 = 33.
    assert_eq!(f.c.get(1, 12), 33);
    let log = f.take_log();
    // skill_stats(…, ilvl = eval(calc2) = 7), non-zero: passed unchanged.
    assert!(
        has(
            &log,
            "Equipment { owner: 0, m: 1, skill: 1, lvl: 3, ilvl: 7 }"
        ),
        "{log:?}"
    );
}

#[test]
fn valkyrie_timer_state_and_links() {
    let t = valkyrie_tables();
    let ct = classes(8);
    let (mut f, u) = world();
    f.c.frame = 100;
    f.node.insert(u, 3);
    assert_eq!(b3_lvl30::valkyrie(&mut f, &t, &ct, u, 1, 3), 1);
    assert!(f.c.has_state(1, 93));
    let log = f.take_log();
    // Type-2 timers deleted, then one at F + 20 (after the spawn's F + 25).
    let del = log.iter().rposition(|l| l == "deltimers 1 2 0").unwrap();
    let sch = pos(&log, "schedule 1 2 120 0 0");
    assert!(del < sch);
    assert!(sch > pos(&log, "schedule 1 2 125 0 0"));
    assert!(log
        .iter()
        .any(|l| l.contains("SourceFields { m: 1, owner: Some(0) }")));
    assert!(has(&log, "NodeInsert { m: 1, slot: 3 }"));
}

// -----------------------------------------------------------------

fn strike_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.emin = 2;
    r.emax = 6;
    r.hitshift = 8;
    r.calc1 = c.f(0);
    tabs(r, c, 1)
}

/// The first (stored) record of the caster's combat list.
fn stored_record(f: &BodyFake, u: usize) -> crate::combat::DamageRecord {
    f.c.units[u].combat.first().expect("a stored record").record
}

#[test]
fn lightning_strike_start_refusals() {
    let t = strike_tables();
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 9, 1),
        0
    );
    // E none.
    let used = f.c.units[u].used.take();
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        0
    );
    f.c.units[u].used = used;
    // T none.
    f.targets.clear();
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        0
    );
    assert!(f.c.units[u].combat.is_empty());
}

#[test]
fn lightning_strike_start_stores_a_miss_without_damage() {
    let t = strike_tables();
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    f.c.in_range = false;
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        1
    );
    let r = stored_record(&f, u);
    assert_eq!((r.result, r.lightning, r.enh_pct), (0, 0, 0));
}

#[test]
fn lightning_strike_start_rolls_lightning_between_min_and_max() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.emin = 2;
    r.emax = 6;
    r.hitshift = 8;
    r.calc1 = c.f(40);
    let t = tabs(r, c, 1);
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    // One draw for the hit test, then roll(b − a) with a = 512, b = 1536.
    let mut s = f.c.units[u].seed;
    s.step();
    let want = 512 + s.roll(1024) as i32;
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        1
    );
    let r = stored_record(&f, u);
    assert_eq!(r.result & 1, 1);
    assert_eq!(r.lightning, want);
    assert_eq!(r.enh_pct, 40);
    assert_eq!(f.c.units[u].seed, s, "exactly one roll");
}

#[test]
fn lightning_strike_start_no_draw_when_the_range_is_empty() {
    let mut r = body_rec();
    r.emin = 3;
    r.emax = 3;
    r.hitshift = 8;
    let t = tabs(r, Code::new(), 1);
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    let mut s = f.c.units[u].seed;
    s.step();
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        1
    );
    assert_eq!(stored_record(&f, u).lightning, 768);
    assert_eq!(f.c.units[u].seed, s, "b − a < 1: no draw");
}

#[test]
fn lightning_strike_start_converts_with_etype() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.emin = 3;
    r.emax = 3;
    r.hitshift = 8;
    r.etype = 4;
    r.calc4 = c.f(25);
    let t = tabs(r, c, 1);
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1);
    let r = stored_record(&f, u);
    assert_eq!((r.conv_pct, r.conv_elem), (25, 4));
}

#[test]
fn lightning_strike_start_uses_srcdam_or_128() {
    let ct = classes(1);
    let mut t = strike_tables();
    t.skills[1].emin = 3;
    t.skills[1].emax = 3;
    let (mut f, u, _) = hit_world();
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        1
    );
    // SrcDam 0 → 128: unscaled.
    assert_eq!(stored_record(&f, u).lightning, 768);
    let (mut f, u, _) = hit_world();
    t.skills[1].srcdam = 64;
    assert_eq!(
        b3_lvl30::lightning_strike_start(&mut f, &t, &ct, u, 1, 1),
        1
    );
    assert_eq!(stored_record(&f, u).lightning, 384);
}

// -----------------------------------------------------------------

/// Caster 0 at (0, 0); K1 (GUID 1) at (12, 10), T (GUID 2) at (10, 10),
/// K3 (GUID 3) at (13, 10); K1 and K3 are scan candidates, T is not
/// (flags 0).
fn chain_world() -> (BodyFake, usize, usize, usize, usize) {
    let (mut f, u) = world();
    let k1 = monster(&mut f, (12, 10));
    let t = monster(&mut f, (10, 10));
    let k3 = monster(&mut f, (13, 10));
    f.c.units[k1].flags = 0xC;
    f.c.units[k3].flags = 0xC;
    f.c.hostile = true;
    f.c.in_range = true;
    f.scan = vec![k1, k3, t];
    f.targets.insert(u, t);
    (f, k1, t, k3, u)
}

fn chain_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.calc1 = c.f(5);
    r.calc2 = c.f(4);
    tabs(r, c, 2)
}

#[test]
fn lightning_strike_refusals() {
    let t = chain_tables();
    let ct = classes(1);
    let (mut f, _, _, _, u) = chain_world();
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 9, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn lightning_strike_applies_the_melee_record_first() {
    let mut t = chain_tables();
    // No candidate in range (K none → 0): the stored record is still applied.
    t.skills[1].calc1 = Code::new().f(1);
    t.skills_code = {
        let mut c = Code::new();
        c.f(1);
        c.0
    };
    let ct = classes(1);
    let (mut f, _, tg, _, u) = chain_world();
    super::fake::stored(&mut f, u, tg, crate::combat::DamageRecord::default());
    assert_eq!(f.c.units[u].combat.len(), 1);
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.c.units[u].combat.is_empty(), "apply_melee freed it");
    assert!(f
        .take_log()
        .iter()
        .any(|l| l.starts_with("event ") && l.ends_with(&format!(" {u} {tg}"))));
}

#[test]
fn lightning_strike_picks_the_next_guid_around_t() {
    let t = chain_tables();
    let ct = classes(1);
    let (mut f, k1, _, _k3, u) = chain_world();
    // Range n = 5 around T (10, 10): K3 (GUID 3 > 2) is the smallest above.
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (13, 10));
    // Without a GUID above, the smallest at or below T's.
    f.scan = vec![k1];
    f.missiles.clear();
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (12, 10));
    // Out of range n: none → 0, no missile.
    f.c.units[k1].flags = 0xC;
    f.pos.insert(k1, (40, 10));
    f.missiles.clear();
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn lightning_strike_needs_a_valid_progressive_missile() {
    let ct = classes(1);
    let (mut f, _, _, _, u) = chain_world();
    let mut t = chain_tables();
    t.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 0);
    t.skills[1].srvmissilea = 2;
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn lightning_strike_missile_record_and_chain_count() {
    let t = chain_tables();
    let ct = classes(1);
    let (mut f, _, tg, _, u) = chain_world();
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 3), 1);
    let m = f.missiles[0];
    assert_eq!(
        (m.flags, m.owner, m.origin, m.class, m.skill, m.level),
        (0x20, u, Some(tg), 1, 1, 3)
    );
    assert_eq!((m.x, m.y, m.target), (0, 0, None));
    let log = f.take_log();
    let made = f.missiles.len();
    assert_eq!(made, 1);
    // j = eval(calc2) = 4 goes to the new missile's data +0x28.
    assert!(log
        .iter()
        .any(|l| l.starts_with("MissileData28 { missile: ") && l.ends_with("v: 4 }")));
    // Return 1 also when no missile was made.
    f.no_missiles = true;
    assert_eq!(b3_lvl30::lightning_strike(&mut f, &t, &ct, u, 1, 3), 1);
}

// -----------------------------------------------------------------

#[test]
fn hydra_start_tests_the_target_room() {
    let (mut f, u) = world();
    f.tpos.insert(u, (20, 20));
    assert_eq!(b3_lvl30::hydra_start(&mut f, u), 1);
    // Target position fails.
    f.tpos.clear();
    assert_eq!(b3_lvl30::hydra_start(&mut f, u), 0);
    // No room at the target.
    f.tpos.insert(u, (20, 20));
    f.point_rooms.insert((20, 20), None);
    assert_eq!(b3_lvl30::hydra_start(&mut f, u), 0);
    // The room at the target is in town.
    f.point_rooms.clear();
    f.town.insert(1);
    assert_eq!(b3_lvl30::hydra_start(&mut f, u), 0);
    f.town.clear();
    // The unit's room none.
    f.room_of.remove(&u);
    assert_eq!(b3_lvl30::hydra_start(&mut f, u), 0);
}

// -----------------------------------------------------------------

fn revive_tables() -> crate::combat::CombatTables {
    let mut ms2: Monstats2 = blank();
    ms2.corpsesel = true;
    ms2.revive = true;
    let mut m = monster_rec();
    m.switchai = true;
    let mut ct = combat_tables(vec![m]);
    ct.monstats2 = vec![ms2];
    ct
}

/// A revivable corpse at `at`.
fn revive_corpse(f: &mut BodyFake, at: (i32, i32)) -> usize {
    let m = monster(f, at);
    f.c.units[m].mode = 12;
    f.c.units[m].flags = 4;
    f.alive.remove(&m);
    m
}

#[test]
fn revive_start_runs_the_revive_test() {
    let ct = revive_tables();
    let (mut f, u) = world();
    // T none.
    assert_eq!(b3_lvl30::revive_start(&mut f, &ct, u), 0);
    let m = revive_corpse(&mut f, (3, 3));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl30::revive_start(&mut f, &ct, u), 1);
    // Each clause of the test on its own.
    let mut bad = ct.clone();
    bad.monstats2[0].revive = false;
    assert_eq!(b3_lvl30::revive_start(&mut f, &bad, u), 0, "revive flag");
    let mut bad = ct.clone();
    bad.monstats[0].velocity = 0;
    assert_eq!(
        b3_lvl30::revive_start(&mut f, &bad, u),
        0,
        "raise corpse test"
    );
    let mut bad = ct.clone();
    bad.monstats[0].switchai = false;
    assert_eq!(
        b3_lvl30::revive_start(&mut f, &bad, u),
        0,
        "can_switch(T, 7)"
    );
    f.alive.insert(m);
    assert_eq!(b3_lvl30::revive_start(&mut f, &ct, u), 0, "not dead");
    f.alive.remove(&m);
    let p = f.add(
        crate::skills::fake::FUnit::new(crate::units::UnitType::Player, 0),
        (4, 4),
    );
    f.targets.insert(u, p);
    assert_eq!(b3_lvl30::revive_start(&mut f, &ct, u), 0, "not a monster");
}

// -----------------------------------------------------------------

fn revive_skill_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.pettype = 6;
    r.petmax = c.f(2);
    r.calc2 = c.f(30);
    tabs(r, c, 1)
}

/// Caster 0 (level 10) and a revivable corpse (level 20, GUID 1) at
/// (5, 5) with a free point one step right and two down; monstats
/// minHP 10, maxHP 20 on a monlvl row of 100%.
fn revive_world() -> (BodyFake, usize, usize) {
    let (mut f, u) = world();
    let m = revive_corpse(&mut f, (5, 5));
    f.targets.insert(u, m);
    f.c.set(u, 12, 10);
    f.c.set(m, 12, 20);
    f.free_shift = Some((1, 2));
    let mut row: Monlvl = blank();
    row.hp = 100;
    f.monlvl = vec![row];
    (f, u, m)
}

fn revive_ct() -> crate::combat::CombatTables {
    let mut ct = revive_tables();
    ct.monstats[0].minhp = 10;
    ct.monstats[0].maxhp = 20;
    ct
}

/// The life the corpse gets: (10 + roll(11)) << 8 on its own seed.
fn revive_life(f: &BodyFake, m: usize) -> i32 {
    let mut s = f.c.units[m].seed;
    (10 + s.roll(11) as i32) << 8
}

#[test]
fn revive_refuses_skill_zero_and_a_bad_pet_type() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, _) = revive_world();
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 0, 1), 0);
    let mut t2 = revive_skill_tables();
    t2.skills[1].pettype = 15;
    assert_eq!(b3_lvl30::revive(&mut f, &t2, &ct, u, 1, 1), 0);
    t2.skills[1].pettype = 200;
    assert_eq!(b3_lvl30::revive(&mut f, &t2, &ct, u, 1, 1), 0);
    assert!(f.take_log().is_empty());
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
}

#[test]
fn revive_refuses_without_a_revivable_target() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    f.targets.clear();
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(u, m);
    f.alive.insert(m);
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.take_log().is_empty(), "nothing happened");
}

#[test]
fn revive_charges_a_paladin_the_raise_penalty() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, _) = revive_world();
    f.c.set(u, 6, 25_600);
    f.c.set(u, 7, 25_600);
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.get(u, 6), 25_600, "not a paladin");
    let (mut f, u, _) = revive_world();
    f.c.units[u].class = 3;
    f.c.set(u, 6, 25_600);
    f.c.set(u, 7, 25_600);
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(
        f.c.get(u, 6),
        25_600 - 25_600 / 8,
        "an eighth of the maximum life"
    );
}

#[test]
fn revive_stands_the_corpse_up() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    let clear = log
        .iter()
        .position(|l| {
            l.starts_with("PatternClear") && l.contains("x: 5, y: 5") && l.contains("32768")
        })
        .expect("pattern clear at the corpse's position with 0x8000");
    let ai = pos(&log, &format!("AiRefresh({m})"));
    let req = pos(&log, &format!("moderequest {m} 1 None"));
    let timers = pos(&log, &format!("deltimers {m} 8 0"));
    let place = pos(&log, &format!("place {m} Some(1) (6, 7)"));
    assert!(
        clear < ai && ai < req && req < timers && timers < place,
        "{log:?}"
    );
    assert_eq!(f.c.units[m].flags & 0x0402_000E, 0x0402_000E);
    assert!(
        log.iter().all(|l| !l.starts_with("Op643C50")),
        "no right skill"
    );
}

#[test]
fn revive_without_a_free_point_still_finishes() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    f.free_shift = None;
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.iter().all(|l| !l.starts_with("place")));
    assert!(log.iter().all(|l| !l.starts_with("moderequest")));
    assert_eq!(
        f.c.units[m].flags,
        4 | 0x8000_0000,
        "only the setup flag, not 0x0402000E"
    );
    // Step 5 onward still ran.
    assert!(f.c.has_state(m, 96));
    assert_ne!(f.c.get(m, 6), 0);
}

#[test]
fn revive_life_is_rolled_on_the_corpses_seed() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    // The caster's level 30 ≥ the corpse's 20: no scaling.
    f.c.set(u, 12, 30);
    let h = revive_life(&f, m);
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.c.get(m, 7), f.c.get(m, 6)), (h, h));
    assert_eq!(f.c.get(m, 12), 20);
    let mut s = Seed::new(1, 0);
    s.roll(11);
    assert_eq!(f.c.units[m].seed, s, "one draw on T's seed");
}

#[test]
fn revive_scales_life_to_a_lower_caster_level() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    // cl = 10 < tl = 20: h × 10 / 20; T's level := 10.
    let h = revive_life(&f, m);
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.c.get(m, 7), f.c.get(m, 6)), (h / 2, h / 2));
    assert_eq!(f.c.get(m, 12), 10);
    // tl = 0: not scaled, level stays 0.
    let (mut f, u, m) = revive_world();
    f.c.set(m, 12, 0);
    let h = revive_life(&f, m);
    b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.c.get(m, 6), h);
    assert_eq!(f.c.get(m, 12), 0);
    // At least 1.
    let (mut f, u, m) = revive_world();
    f.c.set(u, 12, 0);
    b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1);
    assert_eq!((f.c.get(m, 7), f.c.get(m, 6)), (1, 1));
}

#[test]
fn revive_applies_the_skill_stats_with_item_level_zero() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    // ilvl 0 → 3 · L = 12, at most the caster's level 10.
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 4), 1);
    let log = f.take_log();
    assert!(
        has(
            &log,
            &format!("Equipment {{ owner: {u}, m: {m}, skill: 1, lvl: 4, ilvl: 10 }}")
        ),
        "{log:?}"
    );
}

#[test]
fn revive_setup_binds_the_corpse_to_the_caster() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    f.c.frame = 200;
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.iter().any(|l| l.contains(&format!(
        "OwnerData {{ m: {m}, owner: Some({u}), a: 0, b: 0 }}"
    ))));
    assert!(has(
        &log,
        &format!("LeashOwner {{ m: {m}, owner: Some({u}) }}")
    ));
    let del = log
        .iter()
        .rposition(|l| l == &format!("deltimers {m} 2 0"))
        .unwrap();
    assert!(del < pos(&log, &format!("schedule {m} 2 215 0 0")));
    assert!(has(&log, &format!("Alignment {{ u: {m}, a: 2, v: 1 }}")));
    assert!(has(&log, &format!("ClearTargetOverride({m})")));
    assert_eq!(f.c.units[m].flags & 0x8000_0000, 0x8000_0000);
    assert!(f.c.has_state(m, 96));
    // d = eval(calc2) = 30 > 0: umod 21 and a type-7 timer at F + 30.
    assert!(log
        .iter()
        .any(|l| l.contains(&format!("Umod {{ m: {m}, umod: 21"))));
    assert!(has(&log, &format!("schedule {m} 7 230 0 0")));
}

#[test]
fn revive_without_a_duration_has_no_umod_or_timer() {
    let mut t = revive_skill_tables();
    let mut c = Code::new();
    t.skills[1].petmax = c.f(2);
    t.skills[1].calc2 = c.f(0);
    t.skills_code = c.0;
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.iter().all(|l| !l.starts_with("Umod")));
    assert!(log
        .iter()
        .all(|l| !l.starts_with(&format!("schedule {m} 7"))));
}

#[test]
fn revive_adds_the_corpse_to_the_pets() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(has(
        &log,
        &format!("PetAdd {{ owner: {u}, pet: {m}, t: 6, max: 2 }}")
    ));
}

#[test]
fn revive_inserts_the_node_and_resets_the_path() {
    let t = revive_skill_tables();
    let ct = revive_ct();
    let (mut f, u, m) = revive_world();
    f.node.insert(u, 3);
    assert_eq!(b3_lvl30::revive(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    let pet = log.iter().position(|l| l.starts_with("PetAdd")).unwrap();
    let node = pos(&log, &format!("NodeInsert {{ m: {m}, slot: 3 }}"));
    let path = pos(&log, &format!("path {m} Reset"));
    assert!(pet < node && node < path);
}

// -----------------------------------------------------------------

fn fist_tables() -> crate::skills::SkillTables {
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.srvoverlay = 5;
    tabs(r, Code::new(), 2)
}

#[test]
fn fist_refusals() {
    let (mut f, u) = world();
    let m = monster(&mut f, (7, 8));
    f.targets.insert(u, m);
    let t = fist_tables();
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t, u, 9, 1), 0);
    // m ≤ 0 (class 0 is not a missile here).
    let mut t0 = fist_tables();
    t0.skills[1].srvmissilea = 0;
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t0, u, 1, 1), 0);
    t0.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t0, u, 1, 1), 0);
    // T none.
    f.targets.clear();
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t, u, 1, 1), 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn fist_missile_record() {
    let t = fist_tables();
    let (mut f, u) = world();
    let m = monster(&mut f, (7, 8));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t, u, 1, 3), 1);
    let r = f.missiles[0];
    assert_eq!(
        (r.flags, r.owner, r.x, r.y, r.class, r.skill, r.level),
        (1, u, 7, 8, 1, 1, 3)
    );
    assert_eq!((r.origin, r.target), (None, None));
    // Not created → 0.
    f.no_missiles = true;
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t, u, 1, 3), 0);
}

#[test]
fn fist_stores_the_target_identity() {
    let t = fist_tables();
    let (mut f, u) = world();
    let m = monster(&mut f, (7, 8));
    f.targets.insert(u, m);
    b3_lvl30::fist_of_heavens(&mut f, &t, u, 1, 3);
    let log = f.take_log();
    let mm = f.missiles.len(); // one missile, unit id follows the two units
    assert_eq!(mm, 1);
    assert!(log
        .iter()
        .any(|l| l.starts_with("MissileData28") && l.ends_with("v: 1 }")));
    assert!(log
        .iter()
        .any(|l| l.starts_with("MissileData2C") && l.ends_with("v: 1 }")));
}

#[test]
fn fist_overlay_range() {
    let (mut f, u) = world();
    let m = monster(&mut f, (7, 8));
    f.targets.insert(u, m);
    let mut t = fist_tables();
    assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t, u, 1, 3), 1);
    assert!(has(&f.take_log(), &format!("overlay {m} 5")));
    // 0 … count − 1 (200 overlays): 0 and 199 in, −1 and 200 out.
    for (v, on) in [(0u16, true), (199, true), (200, false), (0xFFFF, false)] {
        t.skills[1].srvoverlay = v;
        assert_eq!(b3_lvl30::fist_of_heavens(&mut f, &t, u, 1, 3), 1);
        let any = f.take_log().iter().any(|l| l.starts_with("overlay"));
        assert_eq!(any, on, "overlay {v}");
    }
}

// ------------------------------------------------------------------

/// A whirlwind-ing player (unit 0, entry of skill 1 moving, target
/// (10, 11)) with a hostile candidate at (3, 0) (GUID 1) and one at
/// (4, 0) (GUID 2).
fn ww_world() -> (BodyFake, usize, crate::skills::SkillEntry, usize, usize) {
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let k1 = monster(&mut f, (3, 0));
    let k2 = monster(&mut f, (4, 0));
    for k in [k1, k2] {
        f.c.units[k].flags = 0xC;
        f.c.set(k, 12, 10);
        f.c.set(k, 6, 100 << 8);
    }
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(u, 12, 10);
    f.c.set(u, 19, 100_000);
    f.c.units[u].seed = Seed::new(hit_lo(), 0);
    f.set_entry_param_of(u, &e, 1, 10);
    f.set_entry_param_of(u, &e, 2, 11);
    f.set_entry_param_of(u, &e, 3, -1);
    f.set_entry_flags(u, &e, 1);
    f.scan = vec![k1, k2];
    (f, u, e, k1, k2)
}

fn ww_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(100);
    tabs(r, c, 1)
}

#[test]
fn whirlwind_do_refusals() {
    let t = ww_tables();
    let ct = classes(1);
    let (mut f, u, _, _, _) = ww_world();
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 9, 1), 0);
    f.c.units[u].skills.clear();
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn whirlwind_do_ends_without_a_destination() {
    let t = ww_tables();
    let ct = classes(1);
    for (x, y) in [(0, 11), (10, 0)] {
        let (mut f, u, e, _, _) = ww_world();
        f.set_entry_param_of(u, &e, 1, x);
        f.set_entry_param_of(u, &e, 2, y);
        assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 0);
        assert_eq!(f.entry_flags(u, &e), 0, "End cleared the flags");
        let log = f.take_log();
        assert!(
            log.iter().any(|l| l.starts_with("MsgA5")),
            "landing message"
        );
        assert!(log.iter().all(|l| !l.starts_with("event")), "no hits");
    }
}

#[test]
fn whirlwind_do_ends_on_arrival_and_returns_1() {
    let t = ww_tables();
    let ct = classes(1);
    let (mut f, u, e, _, _) = ww_world();
    f.set_entry_flags(u, &e, 3);
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
    let log = f.take_log();
    assert!(log.iter().any(|l| l.starts_with("MsgA5")));
    assert!(log.iter().all(|l| !l.starts_with("event")), "no hits");
}

#[test]
fn whirlwind_do_does_nothing_when_not_moving_or_dead() {
    let t = ww_tables();
    let ct = classes(1);
    let (mut f, u, e, _, _) = ww_world();
    f.set_entry_flags(u, &e, 0x2000);
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.entry_flags(u, &e), 0x2000);
    f.set_entry_flags(u, &e, 1);
    f.alive.remove(&u);
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.take_log().iter().all(|l| !l.starts_with("event")));
}

#[test]
fn whirlwind_do_player_animates_from_frame_3_and_hits_the_next_guid() {
    let t = ww_tables();
    let ct = classes(1);
    let (mut f, u, e, k1, k2) = ww_world();
    f.c.set(u, 21, 10);
    f.c.set(u, 22, 10);
    // E param 3 = GUID 1: the next GUID above it is K2's.
    f.set_entry_param_of(u, &e, 3, 1);
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 3), 2);
    let log = f.take_log();
    assert!(has(&log, &format!("animfrom {u} 3")));
    // Only K2 was hit (apply_melee ran on it, not on K1).
    assert!(log
        .iter()
        .any(|l| l.starts_with("event ") && l.ends_with(&format!(" {u} {k2}"))));
    assert!(log.iter().all(|l| !l.ends_with(&format!(" {u} {k1}"))));
    assert!(f.c.get(k2, 6) < 100 << 8, "K2 lost life");
    assert_eq!(f.c.get(k1, 6), 100 << 8);
    // The hand toggles each round.
    assert_eq!(f.entry_flags(u, &e), 1 | 0x2000);
}

#[test]
fn whirlwind_do_enhanced_damage_comes_from_calc1() {
    let ct = classes(1);
    let mut life = Vec::new();
    for pct in [0i16, 100] {
        let mut c = Code::new();
        let mut r = body_rec();
        r.calc1 = c.f(pct);
        let t = tabs(r, c, 1);
        let (mut f, u, _, k1, _) = ww_world();
        f.scan = vec![k1];
        f.c.set(u, 21, 10);
        f.c.set(u, 22, 10);
        assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 1);
        life.push((100 << 8) - f.c.get(k1, 6));
    }
    // Damage 10 points = 2560; +100% doubles it.
    assert_eq!(life, [2560, 5120]);
}

#[test]
fn whirlwind_do_without_a_unit_resets_param_3_and_stops() {
    let t = ww_tables();
    let ct = classes(1);
    let (mut f, u, e, _, _) = ww_world();
    f.scan.clear();
    f.set_entry_param_of(u, &e, 3, 7);
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 3), -1);
    assert_eq!(f.entry_flags(u, &e), 1 | 0x2000, "toggled even without K");
}

#[test]
fn whirlwind_do_monster_resets_the_frame_event() {
    let t = ww_tables();
    let ct = classes(1);
    let (mut f, _, _, _, _) = ww_world();
    let e = crate::skills::SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..Default::default()
    };
    let m = monster(&mut f, (0, 0));
    f.c.units[m].skills.push(e);
    f.set_entry_flags(m, &e, 1);
    f.set_entry_param_of(m, &e, 1, 10);
    f.set_entry_param_of(m, &e, 2, 11);
    f.frame_index.insert(m, 9);
    f.scan.clear();
    assert_eq!(b3_lvl30::whirlwind(&mut f, &t, &ct, m, 1, 1), 1);
    assert_eq!(f.frame_index[&m], 0);
    assert_eq!(f.frame_count[&m], 0x400);
    assert!(f.take_log().iter().all(|l| !l.starts_with("animfrom")));
}

// ------------------------------------------------------------------

fn berserk_tables() -> crate::skills::SkillTables {
    berserk_tables_conv(0)
}

fn berserk_tables_conv(conv: i16) -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(50);
    r.calc2 = c.f(25);
    r.resultflags = 0x4000;
    r.hitflags = 0x80;
    r.hitclass = 5;
    r.etype = 3;
    r.calc4 = c.f(conv);
    r.emin = 2;
    r.emax = 2;
    r.hitshift = 8;
    r.aurastate = 40;
    r.aurastat1 = 20;
    r.aurastatcalc1 = c.f(5);
    tabs(r, c, 1)
}

#[test]
fn berserk_refusals() {
    let t = berserk_tables();
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    assert_eq!(b3_lvl30::berserk(&mut f, &t, &ct, u, 9, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.c.units[u].combat.is_empty());
    assert!(f.lists.is_empty());
}

#[test]
fn berserk_hit_fills_the_record() {
    let t = berserk_tables();
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    assert_eq!(b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1), 1);
    let r = stored_record(&f, u);
    assert_eq!(r.result & 1, 1);
    assert_eq!(r.result & 0x4000, 0x4000, "ResultFlags");
    assert_eq!(r.hit_flags & 0x80, 0x80, "HitFlags");
    assert_eq!(r.hit_class, 5, "HitClass");
    assert_eq!(r.enh_pct, 50);
    assert_eq!(r.magic, 512, "roll_elemental: 2 << 8, no spread");
    assert_eq!(
        (r.conv_pct, r.conv_elem),
        (0, 0),
        "calc4 = 0: no conversion"
    );
}

#[test]
fn berserk_etype_conversion() {
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    b3_lvl30::berserk(&mut f, &berserk_tables(), &ct, u, 1, 1);
    let plain = stored_record(&f, u);
    let (mut f, u, _) = hit_world();
    b3_lvl30::berserk(&mut f, &berserk_tables_conv(30), &ct, u, 1, 1);
    let r = stored_record(&f, u);
    assert_eq!((r.conv_pct, r.conv_elem), (30, 3), "EType conversion");
    // The conversion step moves 30% of the physical damage to magic.
    let c = plain.physical * 30 / 100;
    assert!(c > 0);
    assert_eq!((r.physical, r.magic), (plain.physical - c, 512 + c));
}

#[test]
fn berserk_miss_stays_plain() {
    let t = berserk_tables();
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    f.c.in_range = false;
    assert_eq!(b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1), 1);
    let r = stored_record(&f, u);
    assert_eq!(
        (r.result, r.hit_flags, r.hit_class, r.enh_pct, r.magic),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(r.conv_pct, 0);
}

#[test]
fn berserk_uses_srcdam_or_128() {
    let ct = classes(1);
    let t = berserk_tables();
    let (mut f, u, _) = hit_world();
    b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(stored_record(&f, u).magic, 512);
    let mut t = berserk_tables();
    t.skills[1].srcdam = 64;
    let (mut f, u, _) = hit_world();
    b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(stored_record(&f, u).magic, 256);
}

#[test]
fn berserk_aura_state_list() {
    let t = berserk_tables();
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    f.c.frame = 100;
    assert_eq!(b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(u, 40).expect("a state list");
    assert_eq!(l.flags, 2);
    assert_eq!(l.expire, 125, "F + eval(calc2)");
    assert_eq!(l.owner, Some(u));
    assert_eq!(l.callback, callback::DEFAULT);
    assert_eq!(l.stats.get(&20), Some(&5), "aura_fill");
    assert!(f.c.has_state(u, 40));
    assert!(has(&f.take_log(), &format!("timer {u} 12 125")));
    // An existing list: the expiry is replaced.
    f.c.frame = 110;
    b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.lists.len(), 1);
    assert_eq!(f.lists[0].expire, 135);
}

#[test]
fn berserk_non_positive_duration_is_ten_frames() {
    let mut c = Code::new();
    let mut t = berserk_tables();
    t.skills[1].calc2 = c.f(-3);
    t.skills_code = c.0;
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    f.c.frame = 100;
    b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.list_of(u, 40).unwrap().expire, 110);
    assert!(has(&f.take_log(), &format!("timer {u} 12 110")));
}

#[test]
fn berserk_returns_1_without_an_aura_state() {
    let mut t = berserk_tables();
    t.skills[1].aurastate = 0xFFFF;
    let ct = classes(1);
    let (mut f, u, _) = hit_world();
    assert_eq!(b3_lvl30::berserk(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.lists.is_empty());
    assert_eq!(f.c.units[u].combat.len(), 1);
}

// ------------------------------------------------------------------

fn shield_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.auralencalc = c.f(50);
    r.aurastate = 41;
    r.aurastat1 = 20;
    r.aurastatcalc1 = c.f(7);
    tabs(r, c, 2)
}

#[test]
fn blade_shield_start_refusals() {
    let ct = classes(1);
    let t = shield_tables();
    let (mut f, u) = world();
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 9, 1), 0);
    // prog_missile ≤ 0.
    let mut t0 = shield_tables();
    t0.skills[1].srvmissilea = 0;
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t0, &ct, u, 1, 1), 0);
    t0.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t0, &ct, u, 1, 1), 0);
    // E none.
    f.c.units[u].skills.clear();
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.lists.is_empty());
}

#[test]
fn blade_shield_start_needs_a_duration_and_a_state() {
    let ct = classes(1);
    let (mut f, u) = world();
    let mut t = shield_tables();
    t.skills[1].aurastate = 0xFFFF;
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 1), 0);
    let mut t = shield_tables();
    let mut c = Code::new();
    t.skills[1].auralencalc = c.f(0);
    t.skills_code = c.0;
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 1), 0);
    let mut c = Code::new();
    t.skills[1].auralencalc = c.f(-4);
    t.skills_code = c.0;
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.lists.is_empty());
}

#[test]
fn blade_shield_start_clears_the_group_and_applies_the_state() {
    let ct = classes(1);
    let t = shield_tables();
    let (mut f, u) = world();
    f.c.frame = 300;
    f.state_groups.insert(41, 7);
    f.state_groups.insert(42, 7);
    f.c.units[u].states.push(42);
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 2), 1);
    assert!(!f.c.has_state(u, 42), "same group cleared");
    let l = f.list_of(u, 41).expect("the state list");
    assert_eq!(l.expire, 350, "F + eval(auralencalc)");
    assert_eq!(l.callback, callback::DEFAULT);
    assert_eq!((l.skill, l.lvl, l.owner), (1, 2, Some(u)));
    assert!(f.c.has_state(u, 41));
    // apply_state refused (a curse-group state at 100% resistance) → 0,
    // after the group was cleared.
    let (mut f, u) = world();
    f.state_flags.insert((41, group::CURSE));
    f.c.set(u, 109, 100);
    f.state_groups.insert(41, 7);
    f.state_groups.insert(42, 7);
    f.c.units[u].states.push(42);
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 2), 0);
    assert!(!f.c.has_state(u, 42));
    assert!(f.lists.is_empty());
}

#[test]
fn blade_shield_start_fills_the_list_and_marks_the_state() {
    let ct = classes(1);
    let t = shield_tables();
    let (mut f, u) = world();
    assert_eq!(b3_lvl30::blade_shield_start(&mut f, &t, &ct, u, 1, 2), 1);
    let l = f.list_of(u, 41).unwrap();
    assert_eq!(l.stats.get(&20), Some(&7), "aura_fill");
    assert_eq!(l.stats.get(&350), Some(&1), "set 350 := skill");
    assert_eq!(l.stats.get(&351), Some(&2), "set 351 := L");
    assert!(has(&f.take_log(), &format!("changed {u} 41")));
}

// ------------------------------------------------------------------

fn pulse_world() -> (BodyFake, usize, usize) {
    let (mut f, u, _) = hit_world();
    // hit_world's monster is at (1, 0); give it the scan flags.
    let k = 1;
    f.c.units[k].flags = 0xC;
    f.scan = vec![k];
    (f, u, k)
}

fn pulse_tables() -> crate::skills::SkillTables {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 41;
    r.aurarangecalc = c.f(5);
    tabs(r, c, 1)
}

#[test]
fn blade_shield_pulses_outside_town_only() {
    let ct = classes(1);
    let t = pulse_tables();
    let (mut f, u, k) = pulse_world();
    assert_eq!(b3_lvl30::blade_shield(&mut f, &t, &ct, u, 9, 1), 0);
    let mut bad = pulse_tables();
    bad.skills[1].aurastate = 0xFFFF;
    assert_eq!(b3_lvl30::blade_shield(&mut f, &bad, &ct, u, 1, 1), 0);
    f.c.units[u].skills.clear();
    assert_eq!(b3_lvl30::blade_shield(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(
        f.take_log().iter().all(|l| !l.starts_with("event")),
        "no pulse"
    );
    // E present, outside town: the pulse hits the unit in range.
    let (mut f, u, k2) = pulse_world();
    assert_eq!(k, k2);
    assert_eq!(b3_lvl30::blade_shield(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f
        .take_log()
        .iter()
        .any(|l| l.starts_with("event ") && l.ends_with(&format!(" {u} {k}"))));
    // In town: return 1 without a pulse.
    let (mut f, u, _) = pulse_world();
    f.town.insert(1);
    assert_eq!(b3_lvl30::blade_shield(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.take_log().iter().all(|l| !l.starts_with("event")));
}
