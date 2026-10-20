//! Tests of the batch 3 bodies of required level 18 and 24 (the rules
//! `tests3` does not claim) on [`super::fake::BodyFake`].

use super::fake::{stored, BodyFake};
use super::tests2::{body_rec, monster, tabs, world, Code};
use super::*;
use crate::combat::DamageRecord;
use crate::skills::fake::{blank, combat_tables, monster_rec, FUnit};
use crate::skills::SkillUnits;
use crate::units::UnitType;
use d2_data::tables::{Monstats, Monstats2};

// -----------------------------------------------------------------

fn enchant_tables(c: &mut Code) -> crate::skills::SkillTables {
    let mut r = body_rec();
    r.aurastate = 33;
    r.auralencalc = c.f(100);
    r.aurastat1 = 20;
    r.aurastatcalc1 = c.f(5);
    r.aurastat2 = 21;
    r.aurastatcalc2 = c.f(0);
    r.aurastat3 = 22;
    r.aurastatcalc3 = c.f(9);
    r.aurastat4 = 0xFFFF;
    tabs(r, Code(c.0.clone()), 1)
}

#[test]
fn enchant_refuses_bad_records() {
    let mut c = Code::new();
    let ct = combat_tables(vec![monster_rec()]);
    let t = enchant_tables(&mut c);
    let (mut f, u) = world();
    assert_eq!(b3_lvl18::enchant(&mut f, &t, &ct, u, 1, 1), 1);
    // R invalid.
    assert_eq!(b3_lvl18::enchant(&mut f, &t, &ct, u, 9, 1), 0);
    // aurastat1 < -1, >= count; aurastate out of range.
    let bad = |edit: &dyn Fn(&mut d2_data::tables::Skills)| {
        let (mut f, u) = world();
        let mut c = Code::new();
        let mut t = enchant_tables(&mut c);
        edit(&mut t.skills[1]);
        b3_lvl18::enchant(&mut f, &t, &ct, u, 1, 1)
    };
    assert_eq!(bad(&|r| r.aurastat1 = 0xFFFE), 0);
    assert_eq!(bad(&|r| r.aurastat1 = 359), 0);
    assert_eq!(bad(&|r| r.aurastat1 = 358), 1);
    assert_eq!(bad(&|r| r.aurastate = 200), 0);
    assert_eq!(bad(&|r| r.aurastate = 0xFFFF), 0);
    assert_eq!(bad(&|r| r.aurastate = 199), 1);
}

#[test]
fn enchant_applies_the_state_to_an_ally_or_the_caster() {
    let mut c = Code::new();
    let ct = combat_tables(vec![monster_rec()]);
    let t = enchant_tables(&mut c);
    let (mut f, u) = world();
    let ally = f.add(FUnit::new(UnitType::Player, 0), (3, 3));
    let foe = f.add(FUnit::new(UnitType::Player, 0), (4, 4));
    f.allies.insert((u, ally));
    f.c.frame = 50;
    // Target an ally: U = T.
    f.targets.insert(u, ally);
    assert_eq!(b3_lvl18::enchant(&mut f, &t, &ct, u, 1, 3), 1);
    let l = f.list_of(ally, 33).expect("state list on the ally");
    assert_eq!(l.expire, 150);
    assert_eq!((l.skill, l.lvl), (1, 3));
    assert_eq!(l.owner, Some(u));
    assert_eq!(l.callback, callback::DEFAULT);
    // Only non-zero aurastatcalc_i values are set.
    assert_eq!(l.stats.get(&20), Some(&5));
    assert_eq!(l.stats.get(&21), None);
    assert_eq!(l.stats.get(&22), Some(&9));
    assert!(f.list_of(u, 33).is_none());
    assert!(f.take_log().contains(&format!("changed {ally} 33")));
    // A non-ally target: the caster itself.
    f.targets.insert(u, foe);
    assert_eq!(b3_lvl18::enchant(&mut f, &t, &ct, u, 1, 3), 1);
    assert!(f.list_of(u, 33).is_some());
    assert!(f.list_of(foe, 33).is_none());
    assert!(f.take_log().contains(&format!("changed {u} 33")));
}

// -----------------------------------------------------------------

#[test]
fn chain_lightning_sets_the_jump_count() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(4);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (6, 2));
    // R invalid; srvmissilea invalid.
    assert_eq!(b3_lvl18::chain_lightning(&mut f, &t, u, 9, 1), 0);
    let mut t2 = t.clone();
    t2.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(b3_lvl18::chain_lightning(&mut f, &t2, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    // Straight missile at the target; M data +0x28 := n.
    assert_eq!(b3_lvl18::chain_lightning(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    assert_eq!(f.missiles.len(), 1);
    assert_eq!(
        (
            f.missiles[0].flags,
            f.missiles[0].target_x,
            f.missiles[0].target_y
        ),
        (0x21, 6, 2)
    );
    assert!(f
        .take_log()
        .iter()
        .any(|s| s.contains("MissileData28") && s.ends_with("v: 4 }")));
    // No missile created: 0, but the unit flag is already set.
    f.c.units[u].flags = 0;
    f.no_missiles = true;
    assert_eq!(b3_lvl18::chain_lightning(&mut f, &t, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
}

// -----------------------------------------------------------------

#[test]
fn teleport_checks_the_level_rule() {
    let (mut f, u) = world();
    f.tpos.insert(u, (12, 7));
    // R is not read: skill 1 has no record in an empty table, still places.
    f.teleport = Some(1);
    assert_eq!(b3_lvl18::teleport(&mut f, u), 1);
    assert!(f.take_log().contains(&format!("place {u} None (12, 7)")));
    // Level rule none / 0: refused.
    f.teleport = None;
    assert_eq!(b3_lvl18::teleport(&mut f, u), 0);
    f.teleport = Some(0);
    assert_eq!(b3_lvl18::teleport(&mut f, u), 0);
    // 2: refused when the line is blocked, placed otherwise.
    f.teleport = Some(2);
    f.blocked = true;
    assert_eq!(b3_lvl18::teleport(&mut f, u), 0);
    f.blocked = false;
    assert_eq!(b3_lvl18::teleport(&mut f, u), 1);
    // 1 does not look at the line.
    f.teleport = Some(1);
    f.blocked = true;
    assert_eq!(b3_lvl18::teleport(&mut f, u), 1);
    // No room: 0.
    f.room_of.remove(&u);
    assert_eq!(b3_lvl18::teleport(&mut f, u), 0);
}

// -----------------------------------------------------------------

fn confuse_setup() -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastat1 = 0xFFFF;
    r.auratargetstate = 40;
    r.aurafilter = 0x103;
    r.aurarangecalc = c.f(10);
    r.auralencalc = c.f(100);
    r.aurastat2 = 25;
    r.aurastatcalc2 = c.f(11);
    r.aurastat3 = 26;
    r.aurastatcalc3 = c.f(0);
    r.aurastat4 = 27;
    r.aurastatcalc4 = c.f(13);
    r.aurastat5 = 0xFFFF;
    r.aurastatcalc5 = c.f(15);
    r.aurastat6 = 29;
    r.aurastatcalc6 = c.f(16);
    let t = tabs(r, c, 1);
    let mut ms: Monstats = monster_rec();
    ms.switchai = true;
    let mut ms2: Monstats2 = blank();
    ms2.isatt = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    let m = monster(&mut f, (2, 0));
    // isAtt monsters carry unit flag 0x4 from creation.
    f.c.units[m].flags = 4;
    f.tpos.insert(u, (2, 0));
    f.scan = vec![m];
    f.c.hostile = true;
    f.c.units[m].mode = 1;
    f.c.set(m, 12, 1);
    (t, ct, f, u, m)
}

#[test]
fn confuse_refuses_bad_records() {
    let (t, ct, mut f, u, _) = confuse_setup();
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 9, 1), 0);
    let try_with = |edit: &dyn Fn(&mut d2_data::tables::Skills)| {
        let (mut t, ct, mut f, u, _) = confuse_setup();
        edit(&mut t.skills[1]);
        b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1)
    };
    assert_eq!(try_with(&|r| r.aurastat1 = 0xFFFE), 0);
    assert_eq!(try_with(&|r| r.aurastat1 = 359), 0);
    assert_eq!(try_with(&|r| r.auratargetstate = 200), 0);
    assert_eq!(try_with(&|r| r.auratargetstate = 0xFFFF), 0);
    assert_eq!(try_with(&|_| {}), 1);
}

#[test]
fn confuse_context_and_scan() {
    let (t, mut ct, mut f, u, m) = confuse_setup();
    ct.difficultylevels[0].aicursedivisor = 4;
    f.c.frame = 1000;
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    // Unit flags |= 0x40.
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    // D = 100 / AiCurseDivisor 4.
    let l = f
        .list_of(m, 40)
        .expect("confuse state on the monster")
        .clone();
    assert_eq!(l.expire, 1025);
    assert!(f.take_log().contains(&format!("schedule {m} 10 1025 0 0")));
    // The stats come from aurastat2…, slot 1 is (0, 0): no stat set
    // by the state request; slots 2 and 3 (value 0: not set) and 4; the
    // list stops at the invalid aurastat5.
    assert_eq!(l.stats.get(&25), Some(&11));
    assert_eq!(l.stats.get(&26), None);
    assert_eq!(l.stats.get(&27), Some(&13));
    assert_eq!(l.stats.get(&29), None, "stop at the first invalid stat");
    assert_eq!(l.stats.len(), 2);
    assert_eq!(l.callback, callback::CONFUSE);
    // A unit the scan does not reach is not touched.
    let far = monster(&mut f, (500, 500));
    assert!(f.list_of(far, 40).is_none());
}

#[test]
fn confuse_without_a_divisor_keeps_the_duration() {
    let (t, ct, mut f, u, m) = confuse_setup();
    f.c.frame = 10;
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.list_of(m, 40).unwrap().expire, 110);
}

#[test]
fn confuse_test_rejects_the_wrong_targets() {
    let (t, ct, mut f, u, m) = confuse_setup();
    // Not hostile (the scan itself still returns 1).
    f.c.hostile = false;
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 40).is_none());
    f.c.hostile = true;
    // Dead.
    f.alive.remove(&m);
    b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1);
    assert!(f.list_of(m, 40).is_none());
    f.alive.insert(m);
    // Not a monster.
    let p = f.add(FUnit::new(UnitType::Player, 0), (2, 0));
    f.scan = vec![p];
    b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1);
    assert!(f.list_of(p, 40).is_none());
    // `can_switch(U, 11)` fails for a monster without `switchai`.
    f.scan = vec![m];
    let mut ct2 = ct.clone();
    ct2.monstats[0].switchai = false;
    b3_lvl18::confuse(&mut f, &t, &ct2, u, 1, 1);
    assert!(f.list_of(m, 40).is_none());
}

#[test]
fn confuse_slot_one_stays_empty() {
    let (t, ct, mut f, u, m) = confuse_setup();
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    // Stat 0 / value 0 → v = 0 → request stat −1: nothing set for it.
    let l = f.list_of(m, 40).unwrap();
    assert_eq!(l.stats.get(&0), None);
    assert_eq!(l.owner, Some(u));
    assert_eq!((l.skill, l.lvl), (1, 1));
}

#[test]
fn confuse_updateanimrate_refreshes_the_animation() {
    let (t, ct, mut f, u, m) = confuse_setup();
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(!f.take_log().contains(&format!("anim {m}")));
    let (t, ct, mut f, u, m) = confuse_setup();
    f.stat_infos.insert(
        25,
        BodyStat {
            updateanimrate: true,
            maxstat: -1,
            ..BodyStat::default()
        },
    );
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.take_log().contains(&format!("anim {m}")));
}

#[test]
fn confuse_turns_the_monster_and_arms_the_timer() {
    let (t, ct, mut f, u, m) = confuse_setup();
    f.c.frame = 7;
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("Alignment {{ u: {m}, a: 1, v: 1 }}")));
    assert!(log.contains(&format!("NodePrepend {{ u: {m}, slot: 9 }}")));
    assert!(log.contains(&format!("TargetOverride {{ m: {m}, kind: 3, guid: 0 }}")));
    assert!(log.contains(&format!("schedule {m} 10 107 0 0")));
    // A monster already in a list is not prepended again.
    let (t, ct, mut f, u, m) = confuse_setup();
    f.node.insert(m, 3);
    assert_eq!(b3_lvl18::confuse(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(!f.take_log().iter().any(|s| s.starts_with("NodePrepend")));
}

#[test]
fn confuse_remove_callback_restores_the_alignment() {
    let (_, _, mut f, _, m) = confuse_setup();
    f.state_on(m, 40, true);
    helpers3::remove_alignment(&mut f, m, 40);
    assert!(!f.has_state(m, 40));
    let log = f.take_log();
    assert!(log.contains(&format!("Alignment {{ u: {m}, a: 0, v: 1 }}")));
    assert!(log.contains(&format!("NodeRemove({m})")));
}

// -----------------------------------------------------------------

fn hammer_tables(c: &mut Code) -> crate::skills::SkillTables {
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.param1 = 16;
    tabs(r, Code(c.0.clone()), 2)
}

#[test]
fn blessed_hammer_refuses() {
    let t = hammer_tables(&mut Code::new());
    let (mut f, u) = world();
    f.tpos.insert(u, (5, 6));
    // R invalid.
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t, u, 9, 1), 0);
    // m <= 0: missile 0 is refused, too.
    let mut t0 = t.clone();
    t0.skills[1].srvmissilea = 0;
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t0, u, 1, 1), 0);
    t0.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t0, u, 1, 1), 0);
    // No target position.
    f.tpos.clear();
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn blessed_hammer_missile_and_path() {
    let t = hammer_tables(&mut Code::new());
    let (mut f, u) = world();
    f.tpos.insert(u, (5, 6));
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t, u, 1, 3), 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    let m = f.missiles[0];
    assert_eq!(
        (m.flags, m.owner, m.origin, m.class, m.target_x, m.target_y),
        (0x20, u, Some(u), 1, 5, 6)
    );
    assert_eq!((m.skill, m.level), (1, 3));
    let log = f.take_log();
    let at = log
        .iter()
        .position(|s| s.ends_with("Type(14)"))
        .expect("type 14");
    assert!(log[at + 1].ends_with("Compute"));
    // No missile created: 0.
    f.no_missiles = true;
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t, u, 1, 3), 0);
}

#[test]
fn blessed_hammer_scales_with_concentration() {
    let t = hammer_tables(&mut Code::new());
    let (mut f, u) = world();
    f.tpos.insert(u, (5, 6));
    f.missile_base = vec![(52, 10), (53, 20)];
    // No Concentration: untouched.
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t, u, 1, 1), 1);
    let m = f.c.units.len() - 1;
    assert_eq!((f.c.get(m, 52), f.c.get(m, 53)), (10, 20));
    // State 42 with damagepercent 40, Param1 16: c = 40 * 16 / 8 = 80.
    let l = f.alloc_list(0, 0, None).unwrap();
    f.set_list_state(l, 42);
    f.list_set(l, 25, 40);
    f.attach(u, l);
    f.state_on(u, 42, true);
    assert_eq!(b3_lvl18::blessed_hammer(&mut f, &t, u, 1, 1), 1);
    let m = f.c.units.len() - 1;
    assert_eq!((f.c.get(m, 52), f.c.get(m, 53)), (18, 36));
}

// ------------------------------------------------------------------

fn freeze_world(
    lo: [u32; 2],
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    [usize; 2],
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 50;
    r.auratargetstate = 51;
    r.aurastat1 = 20;
    r.aurastatcalc1 = c.f(7);
    r.passivestat1 = 21;
    r.passivecalc1 = c.f(3);
    r.aurarangecalc = c.f(10);
    r.auralencalc = c.f(100);
    r.aurafilter = 0x103;
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.coldeffect = 0xFF;
    let mut ms2: Monstats2 = blank();
    ms2.isatt = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    f.tpos.insert(u, (2, 0));
    f.c.hostile = true;
    let mut ms = [0; 2];
    for (i, m) in ms.iter_mut().enumerate() {
        *m = monster(&mut f, (2, i as i32));
        f.c.units[*m].mode = 1;
        // isAtt: unit flag 0x4 from creation.
        f.c.units[*m].flags = 4;
        f.c.units[*m].seed = crate::rng::Seed::new(lo[i], 0);
        f.c.set(*m, 12, 1);
    }
    f.scan = ms.to_vec();
    (t, ct, f, u, ms)
}

fn seed_with(want: std::ops::Range<u32>) -> u32 {
    (0u32..)
        .find(|&lo| want.contains(&(crate::rng::Seed::new(lo, 0).step() % 100)))
        .unwrap()
}

#[test]
fn holy_freeze_self_list_has_its_own_remove_callback() {
    let (t, ct, mut f, u, _) = freeze_world([0, 0]);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, true), 1);
    let l = f.list_of(u, 50).expect("self list");
    assert_eq!(l.callback, callback::HOLY_FREEZE);
    // The srvdo 66 form keeps the default self callback.
    let (t, ct, mut f, u, _) = freeze_world([0, 0]);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, false), 1);
    assert_ne!(f.list_of(u, 50).unwrap().callback, callback::HOLY_FREEZE);
}

#[test]
fn holy_freeze_only_affects_cold_affected_monsters() {
    let (t, mut ct, mut f, u, [m, other]) = freeze_world([0, 0]);
    f.c.units[other].class = 1;
    let mut ms = monster_rec();
    ms.coldeffect = 0;
    ct.monstats.push(ms);
    ct.monstats2.push(blank());
    let p = f.add(FUnit::new(UnitType::Player, 0), (2, 5));
    f.c.units[p].mode = 1;
    f.scan.push(p);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, true), 1);
    assert!(f.list_of(m, 51).is_some(), "ColdEffect < 0");
    assert!(f.list_of(other, 51).is_none(), "ColdEffect >= 0");
    assert!(f.list_of(p, 51).is_some(), "other unit types pass");
    // The plain form has no such test.
    let (t, mut ct, mut f, u, [m, other]) = freeze_world([0, 0]);
    f.c.units[other].class = 1;
    ct.monstats.push(monster_rec());
    ct.monstats2.push(blank());
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, false), 1);
    assert!(f.list_of(m, 51).is_some() && f.list_of(other, 51).is_some());
}

#[test]
fn holy_freeze_applies_the_aura_state_and_the_hit() {
    let (t, ct, mut f, u, [m, _]) = freeze_world([0, 0]);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, true), 1);
    let l = f.list_of(m, 51).expect("target state");
    assert_eq!(l.stats.get(&20), Some(&7));
    assert_eq!((l.skill, l.lvl), (1, 1));
    // The hit record goes through apply and the reaction on the monster.
    assert!(f.take_log().iter().any(|s| s.contains(&format!("{m}"))));
}

#[test]
fn holy_freeze_shatter_is_a_one_in_five_draw() {
    let (t, ct, mut f, u, [a, b]) = freeze_world([seed_with(0..20), seed_with(20..100)]);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, true), 1);
    assert!(f.has_state(a, 107), "draw < 20: shatter on");
    assert!(!f.has_state(b, 107), "draw >= 20: shatter off");
}

#[test]
fn holy_freeze_remove_callback() {
    let (_, _, mut f, _, [a, _]) = freeze_world([0, 0]);
    f.state_on(a, 50, true);
    f.state_on(a, 107, true);
    helpers3::remove_holy_freeze(&mut f, a, 50);
    assert!(!f.has_state(a, 50) && !f.has_state(a, 107));
    assert!(f.take_log().contains(&format!("anim {a}")));
    // Dead (and not "stay on death"): the state goes off, shatter stays.
    f.state_on(a, 50, true);
    f.state_on(a, 107, true);
    f.alive.remove(&a);
    helpers3::remove_holy_freeze(&mut f, a, 50);
    assert!(!f.has_state(a, 50) && f.has_state(a, 107));
}

// ------------------------------------------------------------------

#[test]
fn leap_attack_do_dispatches_on_the_entry_flags() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    // No entry for the skill: 0.
    assert_eq!(b3_lvl18::leap_attack(&mut f, &t, &ct, u, 7, 1), 0);
    // No flags: flags := 0x1000, return 1.
    assert_eq!(b3_lvl18::leap_attack(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_flags(u, &e), 0x1000);
    // 0x80 is the Launch helper's result; 0x200 the Strike helper's;
    // 0x100 wins over 0x80 (in flight), and a failed Land returns 1.
    for (flags, which) in [(0x80u32, 0), (0x200, 1), (0x100, 2), (0x180, 2)] {
        let (mut f, u) = world();
        let e = f.c.units[u].used.unwrap();
        f.set_entry_flags(u, &e, flags);
        let got = b3_lvl18::leap_attack(&mut f, &t, &ct, u, 1, 1);
        let log = f.take_log();
        let (mut g, u) = world();
        g.set_entry_flags(u, &e, flags);
        let want = match which {
            0 => leap_launch(&mut g, &ct, u, &e),
            1 => leap_strike(&mut g, &t, &ct, u, 1, 1),
            _ => {
                // Landed and a target picked: animation from frame 16.
                assert_eq!(leap_land(&mut g, &ct, u, &e), 1);
                assert!(log.contains(&format!("animfrom {u} 16")), "{log:?}");
                g.take_log();
                g.take_log();
                got
            }
        };
        assert_eq!(got, want, "flags {flags:#x}");
        if which != 2 {
            assert_eq!(log, g.take_log(), "flags {flags:#x}");
        }
    }
}

// ------------------------------------------------------------------

fn rabies_world(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    edit(&mut r, &mut c);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    // An attackable monster: unit flag 0x4 from creation.
    f.c.units[m].flags = 4;
    f.targets.insert(u, m);
    f.c.in_range = true;
    f.c.hostile = true;
    f.c.set(u, 12, 99);
    f.c.set(u, 19, 100_000);
    f.c.set(m, 12, 1);
    (t, ct, f, u, m)
}

#[test]
fn rabies_start_refusals() {
    let (t, ct, mut f, u, _) = rabies_world(|_, _| {});
    // R invalid.
    assert_eq!(b3_lvl18::rabies_start(&mut f, &t, &ct, u, 9, 1), 0);
    // E's skill differs from `skill` (the used entry is skill 1).
    assert_eq!(b3_lvl18::rabies_start(&mut f, &t, &ct, u, 0, 1), 0);
    // T none.
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn rabies_start_needs_the_shape_state() {
    let (t, ct, mut f, u, _) = rabies_world(|r, _| r.aurastate = 60);
    assert_eq!(b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.state_on(u, 60, true);
    let mut hits = 0;
    for _ in 0..20 {
        hits += b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1);
    }
    assert!(hits > 0, "with the state the melee roll can hit");
    // An out-of-range aurastate is not tested.
    let (t, ct, mut f, u, _) = rabies_world(|r, _| r.aurastate = 0xFFFF);
    let mut hits = 0;
    for _ in 0..20 {
        hits += b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1);
    }
    assert!(hits > 0);
}

#[test]
fn rabies_start_arms_the_do_on_a_hit_only() {
    let (t, ct, mut f, u, m) = rabies_world(|r, c| {
        r.calc1 = c.f(5);
        r.etype = 5;
        r.calc4 = c.f(6);
    });
    let e = f.c.units[u].used.unwrap();
    let (mut hit, mut miss) = (false, false);
    f.c.set(u, 19, 0);
    f.c.set(m, 31, 100_000);
    for _ in 0..400 {
        f.set_entry_param_of(u, &e, 1, 9);
        let r = b3_lvl18::rabies_start(&mut f, &t, &ct, u, 1, 1);
        if r == 1 {
            hit = true;
            assert_eq!(f.entry_param(u, &e, 1), 1);
        } else {
            miss = true;
            // Step 3 cleared the param before the roll.
            assert_eq!(f.entry_param(u, &e, 1), 0);
        }
        assert!(f.c.units[u].combat.is_empty(), "no damage record");
    }
    assert!(hit && miss);
}

// ------------------------------------------------------------------

#[test]
fn rabies_do_refusals() {
    let (t, ct, mut f, u, _) = rabies_world(|_, _| {});
    let e = f.c.units[u].used.unwrap();
    f.set_entry_param_of(u, &e, 1, 1);
    assert_eq!(b3_lvl18::rabies(&mut f, &t, &ct, u, 9, 1), 0, "R invalid");
    assert_eq!(b3_lvl18::rabies(&mut f, &t, &ct, u, 0, 1), 0, "E skill");
    // E param 1 = 0.
    f.set_entry_param_of(u, &e, 1, 0);
    assert_eq!(b3_lvl18::rabies(&mut f, &t, &ct, u, 1, 1), 0);
    // T none.
    f.set_entry_param_of(u, &e, 1, 1);
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl18::rabies(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.entry_param(u, &e, 1), 1, "refused before step 2");
}

#[test]
fn rabies_do_infects_and_hits_again() {
    let (t, ct, mut f, u, m) = rabies_world(|r, c| {
        r.auratargetstate = 70;
        r.etype = 5;
        r.param1 = 20;
        r.calc1 = c.f(33);
        r.resultflags = 3;
    });
    let e = f.c.units[u].used.unwrap();
    f.set_entry_param_of(u, &e, 1, 1);
    f.c.frame = 100;
    assert_eq!(b3_lvl18::rabies(&mut f, &t, &ct, u, 1, 1), 1);
    // The arm is spent.
    assert_eq!(f.entry_param(u, &e, 1), 0);
    // Plague puts the poison state on T, length from elem_len (≥ 10 for
    // the second hit).
    assert!(f.list_of(m, 70).is_some(), "infected");
    // The hit went through apply_melee.
    let log = f.take_log();
    assert!(log.iter().any(|s| s.contains("event")), "{log:?}");
}

// ------------------------------------------------------------------

fn claws_world(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    rabies_world(|r, c| {
        r.resultflags = 3;
        edit(r, c)
    })
}

#[test]
fn fire_claws_refusals() {
    let (t, ct, mut f, u, _) = claws_world(|_, _| {});
    assert_eq!(b3_lvl18::fire_claws(&mut f, &t, &ct, u, 9, 1), 0);
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl18::fire_claws(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.c.units[u].combat.is_empty());
}

#[test]
fn fire_claws_record() {
    let (t, ct, mut f, u, _) = claws_world(|r, c| {
        r.hitflags = 0x100;
        r.hitclass = 0x20;
        r.calc1 = c.f(45);
    });
    assert_eq!(b3_lvl18::fire_claws(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = f.c.units[u].combat.first().expect("combat entry").record;
    assert_eq!(rec.result & 1, 1);
    // |= 2 | HitFlags.
    assert_eq!(rec.hit_flags & 0x102, 0x102);
    assert_eq!(rec.hit_class, 0x20);
    assert_eq!(rec.enh_pct, 45);
    // HitClass 0: the record's own class stays.
    let (t, ct, mut f, u, _) = claws_world(|_, _| {});
    assert_eq!(b3_lvl18::fire_claws(&mut f, &t, &ct, u, 1, 1), 1);
    let rec = f.c.units[u].combat.first().unwrap().record;
    assert_eq!(rec.enh_pct, 0);
}

#[test]
fn fire_claws_returns_one_on_a_miss_without_an_entry() {
    let (t, ct, mut f, u, m) = claws_world(|r, _| r.resultflags = 0);
    f.c.set(u, 19, 0);
    f.c.set(m, 31, 100_000);
    let mut missed = false;
    for _ in 0..400 {
        f.c.units[u].combat.clear();
        assert_eq!(b3_lvl18::fire_claws(&mut f, &t, &ct, u, 1, 1), 1);
        missed |= f.c.units[u].combat.is_empty();
    }
    assert!(missed);
}

// ------------------------------------------------------------------

fn fury_world(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.prgcalc1 = c.f(3);
    edit(&mut r, &mut c);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.tpos.insert(u, (5, 5));
    (t, ct, f, u)
}

#[test]
fn blade_fury_start_refusals() {
    let (t, ct, mut f, u) = fury_world(|_, _| {});
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 9, 1), 0);
    let mut t2 = t.clone();
    t2.skills[1].srvmissilea = 0xFFFE;
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t2, &ct, u, 1, 1), 0);
    // No entry for the skill (skill 0 has a record but the unit lacks it).
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 0, 1), 0);
}

#[test]
fn blade_fury_start_opens_the_channel() {
    let (t, ct, mut f, u) = fury_world(|r, _| r.startmana = 5);
    let e = f.c.units[u].used.unwrap();
    f.c.frame = 100;
    // startmana 5 needs 5 << 8 mana.
    f.c.set(u, 8, (5 << 8) - 1);
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.set(u, 8, 5 << 8);
    f.set_entry_param_of(u, &e, 1, 77);
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(u, 12).expect("channel list");
    assert_eq!(l.expire, 121);
    assert_eq!(l.flags, 2);
    assert_eq!(l.owner, Some(u));
    assert_eq!(l.callback, callback::BLADE_FURY);
    assert!(f.has_state(u, 12));
    assert_eq!(f.entry_param(u, &e, 1), 0);
    assert!(f.take_log().contains(&format!("timer {u} 12 121")));
}

#[test]
fn blade_fury_start_while_channelling() {
    let (t, ct, mut f, u) = fury_world(|_, _| {});
    let e = f.c.units[u].used.unwrap();
    f.c.frame = 100;
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 1, 1), 1);
    f.take_log();
    // Param 1 > F: only rewound, return 1; expiry := F + 7, timer too.
    f.c.frame = 110;
    f.set_entry_param_of(u, &e, 1, 111);
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.list_of(u, 12).unwrap().expire, 117);
    let log = f.take_log();
    assert!(log.contains(&format!("timer {u} 12 117")));
    assert!(log.contains(&format!("restart {u} 1")));
    // Param 1 <= F: the do runs; a blade made returns 1 and pays mana.
    // (Param 1 = F: the do makes no blade, returns 0: no mana.)
    f.set_entry_param_of(u, &e, 1, 110);
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.set_entry_param_of(u, &e, 1, 109);
    assert_eq!(b3_lvl18::blade_fury_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 1);
    assert_eq!(f.entry_param(u, &e, 1), 112);
}

// ------------------------------------------------------------------

fn tail_world(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    rabies_world(edit)
}

#[test]
fn dragon_tail_start_attack_rate_list_and_refusals() {
    let (t, ct, mut f, u, _) = tail_world(|r, _| r.param4 = 40);
    assert_eq!(b3_lvl18::dragon_tail_start(&mut f, &t, &ct, u, 9, 1), 0);
    assert!(f.lists.is_empty(), "R invalid: before the list");
    // T none: the attack-rate list is made, the result 0.
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl18::dragon_tail_start(&mut f, &t, &ct, u, 1, 1), 0);
    let l = f.lists.last().expect("attack rate list");
    assert_eq!(l.flags, 4);
    assert_eq!(l.unit, Some(u));
    assert_eq!(l.stats.get(&(stat::ATTACKRATE)), Some(&40));
    assert!(f.take_log().contains(&format!("anim {u}")));
}

#[test]
fn dragon_tail_start_hit_stores_the_kick() {
    let (t, ct, mut f, u, m) = tail_world(|r, _| r.srcdam = 0);
    let (mut hit, mut miss) = (false, false);
    f.c.set(u, 19, 0);
    f.c.set(m, 31, 100_000);
    for _ in 0..400 {
        f.c.units[u].combat.clear();
        let r = b3_lvl18::dragon_tail_start(&mut f, &t, &ct, u, 1, 1);
        if r == 1 {
            hit = true;
            assert_eq!(f.c.units[u].combat.len(), 1);
            assert_eq!(f.c.units[u].combat[0].record.result & 1, 1);
        } else {
            miss = true;
            assert!(f.c.units[u].combat.is_empty());
        }
    }
    assert!(hit && miss);
}

// ------------------------------------------------------------------

fn tail_do_world() -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
    usize,
) {
    let (t, ct, mut f, u, m) = tail_world(|r, c| {
        r.calc1 = c.f(30);
        r.aurarangecalc = c.f(10);
        r.aurafilter = 0;
    });
    let rec = DamageRecord {
        result: 1,
        physical: 1000,
        ..DamageRecord::default()
    };
    stored(&mut f, u, m, rec);
    let m2 = monster(&mut f, (2, 0));
    f.c.units[m2].mode = 1;
    f.c.units[m2].flags = 0xC;
    f.c.set(m2, 12, 1);
    f.c.set(m2, 6, 10_000_000);
    f.c.set(m, 6, 10_000_000);
    f.scan = vec![m2];
    (t, ct, f, u, m, m2)
}

#[test]
fn dragon_tail_do_refusals() {
    let (t, ct, mut f, u, _, _) = tail_do_world();
    assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 9, 1), 0);
    // No pair record.
    f.c.units[u].combat.clear();
    assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 1, 1), 0);
    // T none.
    let (t, ct, mut f, u, _, _) = tail_do_world();
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn dragon_tail_do_finishes_and_splashes_fire() {
    let (t, ct, mut f, u, _m, m2) = tail_do_world();
    f.c.set(u, 329, 20);
    f.take_log();
    assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 1, 1), 1);
    // The pair record was applied to T (apply_melee).
    assert!(f.c.units[u].combat.is_empty());
    let log = f.take_log();
    // The finisher ran (it clears the pgsv group) before the melee hit.
    assert!(log.contains(&"cleargroup 0 4".to_string()));
    // D fire = pct(1000, 30 + 20, 100) = 500 on the unit in range.
    assert_eq!(f.c.get(m2, 6), 10_000_000 - 500);
    // T dead after the hit: still the splash (tests the
    // unit).
    let (t, ct, mut f, u, m, m2) = tail_do_world();
    f.c.set(u, 329, 20);
    f.alive.remove(&m);
    assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.get(m2, 6), 10_000_000 - 500);
    // The kicker dead: no splash.
    let (t, ct, mut f, u, _m, m2) = tail_do_world();
    f.alive.remove(&u);
    assert_eq!(b3_lvl18::dragon_tail(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.get(m2, 6), 10_000_000);
}

// -----------------------------------------------------------------

fn doppel_world(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 0;
    r.summode = 1;
    r.petmax = c.f(2);
    r.calc2 = c.f(500);
    r.calc3 = c.f(25);
    edit(&mut r, &mut c);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.tpos.insert(u, (7, 9));
    (t, ct, f, u)
}

#[test]
fn dopplezon_class_and_pettype() {
    let (t, ct, mut f, u) = doppel_world(|r, _| r.pettype = 3);
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 9, 1), 0);
    let (t0, ..) = doppel_world(|r, _| r.summon = 0xFFFF);
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t0, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f
        .take_log()
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 3")));
    // pettype >= count: pt = 0, no refusal.
    let (t, ct, mut f, u) = doppel_world(|r, _| r.pettype = 20);
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f
        .take_log()
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 0")));
}

#[test]
fn dopplezon_spawns_and_links() {
    let (t, ct, mut f, u) = doppel_world(|_, _| {});
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    let m = f.c.units.len() - 1;
    let log = f.take_log();
    // flags 0, class 0, mode 1, at the target position (7, 9).
    assert!(log.iter().any(|s| s.starts_with("monster 1 (7, 9) 0 1")));
    assert!(log.iter().any(|s| s.starts_with("PetAdd")
        && s.contains(&format!("pet: {m}"))
        && s.contains("max: 2")));
    assert!(log
        .iter()
        .any(|s| s.starts_with("SourceFields") && s.contains(&format!("m: {m}"))));
    // Spawn failing: 0.
    f.no_monsters = true;
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn dopplezon_life_level_and_timer() {
    let (t, ct, mut f, u) = doppel_world(|_, _| {});
    f.c.set(u, 12, 30);
    f.c.set(u, 7, 2000);
    f.c.frame = 40;
    assert_eq!(b3_lvl24::dopplezon(&mut f, &t, &ct, u, 1, 5), 1);
    let m = f.c.units.len() - 1;
    // h = pct(max life, 25, 100); the summon's level from base_stats
    // (5 + 30 * 3 / 4 = 27, capped at the owner's 30).
    assert_eq!(f.c.get(m, 6), 500);
    assert_eq!(f.c.get(m, 7), 500);
    assert_eq!(f.c.get(m, 12), 27);
    let log = f.take_log();
    assert!(log.contains(&format!("schedule {m} 7 540 0 0")));
    assert!(log.contains(&format!("Umod {{ m: {m}, umod: 21, arg: 0 }}")));
    assert!(log
        .iter()
        .any(|s| s.contains("171") && s.contains(&format!("{m}"))));
}

// ----------------------------------------------------------------------

#[test]
fn thunder_storm_start_sets_the_entry() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    assert_eq!(b3_lvl24::thunder_storm_start(&mut f, &t, u, 9), 0);
    // The unit lacks an entry of skill 0.
    assert_eq!(b3_lvl24::thunder_storm_start(&mut f, &t, u, 0), 0);
    assert_eq!(b3_lvl24::thunder_storm_start(&mut f, &t, u, 1), 1);
    assert_eq!((f.entry_param(u, &e, 1), f.entry_param(u, &e, 2)), (-1, 1));
}

fn storm_world(
    edit: impl Fn(&mut d2_data::tables::Skills, &mut Code),
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.aurastate = 55;
    r.auralencalc = c.f(100);
    r.param7 = 20;
    edit(&mut r, &mut c);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let k = monster(&mut f, (3, 0));
    f.c.units[k].mode = 1;
    // Flags 0x4 / 0x8 and hostility: the extra bits `next_unit` adds.
    f.c.units[k].flags = 0xC;
    f.c.hostile = true;
    f.scan = vec![k];
    (t, ct, f, u, k)
}

#[test]
fn thunder_storm_refusals() {
    let (t, ct, mut f, u, _) = storm_world(|_, _| {});
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 9, 1), 0);
    for edit in [
        (&|r: &mut d2_data::tables::Skills| r.srvmissilea = 0xFFFF)
            as &dyn Fn(&mut d2_data::tables::Skills),
        &|r| r.aurastate = 0xFFFF,
        &|r| r.aurastate = 200,
    ] {
        let (mut t2, ..) = storm_world(|_, _| {});
        edit(&mut t2.skills[1]);
        assert_eq!(b3_lvl24::thunder_storm(&mut f, &t2, &ct, u, 1, 1), 0);
    }
    // No entry for the skill.
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 0, 1), 0);
}

#[test]
fn thunder_storm_state_run() {
    let (t, ct, mut f, u, _) = storm_world(|_, _| {});
    let e = f.c.units[u].used.unwrap();
    f.c.frame = 10;
    f.set_entry_param_of(u, &e, 2, 1);
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 1, 4), 1);
    let l = f.list_of(u, 55).expect("storm state").clone();
    assert_eq!(l.expire, 110);
    assert_eq!(l.callback, callback::DEFAULT);
    assert_eq!(l.stats.get(&350), Some(&1));
    assert_eq!(l.stats.get(&351), Some(&4));
    // E param 2 := 0, and no strike happened.
    assert_eq!(f.entry_param(u, &e, 2), 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn thunder_storm_strikes_the_next_unit() {
    let (t, ct, mut f, u, k) = storm_world(|_, _| {});
    let e = f.c.units[u].used.unwrap();
    f.state_on(u, 55, true);
    f.set_entry_param_of(u, &e, 1, -1);
    f.set_entry_param_of(u, &e, 2, 0);
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 1, 4), 1);
    // A missile at K, handled once and removed, message to the client.
    assert_eq!(f.missiles.len(), 1);
    assert_eq!((f.missiles[0].x, f.missiles[0].y), (3, 0));
    let log = f.take_log();
    assert!(log
        .iter()
        .any(|s| s.starts_with("MissileHit") && s.contains(&format!("unit: {k}"))));
    assert!(log.iter().any(|s| s.starts_with("RemoveUnit")));
    assert!(log.iter().any(|s| s.starts_with("MsgA3")));
    assert_eq!(f.entry_param(u, &e, 1), f.c.units[k].guid as i32);
    // Wraps round to the same unit when it is the only one.
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 1, 4), 1);
    assert_eq!(f.entry_param(u, &e, 1), f.c.units[k].guid as i32);
    // No accepted unit: E param 1 := -1.
    f.scan.clear();
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 1, 4), 1);
    assert_eq!(f.entry_param(u, &e, 1), -1);
    // The caster in town (K is not, or the scan would not accept it): no
    // missile, the GUID still advances.
    f.scan = vec![k];
    let n = f.missiles.len();
    f.set_entry_param_of(u, &e, 1, -1);
    f.room_of.insert(k, 2);
    f.town.insert(1);
    assert_eq!(b3_lvl24::thunder_storm(&mut f, &t, &ct, u, 1, 4), 1);
    assert_eq!(f.missiles.len(), n);
    assert_eq!(f.entry_param(u, &e, 1), f.c.units[k].guid as i32);
}

#[test]
fn attract_start_always_succeeds() {
    assert_eq!(start(18), Some(1));
    assert_eq!(start(19), None);
}

// -----------------------------------------------------------------

fn attract_world() -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastat1 = 0xFFFF;
    r.auratargetstate = 41;
    r.aurafilter = 0x103;
    r.aurarangecalc = c.f(10);
    r.auralencalc = c.f(100);
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ms2: Monstats2 = blank();
    ms2.isatt = true;
    let mut ct = combat_tables(vec![ms]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    f.c.hostile = true;
    let tg = monster(&mut f, (2, 0));
    let other = monster(&mut f, (3, 0));
    for m in [tg, other] {
        f.c.units[m].mode = 1;
        // isAtt: unit flag 0x4 from creation.
        f.c.units[m].flags = 4;
    }
    f.targets.insert(u, tg);
    f.scan = vec![tg, other];
    (t, ct, f, u, tg, other)
}

#[test]
fn attract_refusals() {
    let (t, ct, mut f, u, tg, _) = attract_world();
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 9, 1), 0);
    // The attract test.
    f.c.hostile = false;
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.hostile = true;
    f.alive.remove(&tg);
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 0);
    f.alive.insert(tg);
    f.room_of.insert(tg, 2);
    f.town.insert(2);
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 0);
    f.town.clear();
    let mut ct2 = ct.clone();
    ct2.monstats[0].switchai = false;
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct2, u, 1, 1), 0);
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.lists.is_empty());
    // The record checks.
    let (t, ct, mut f, u, ..) = attract_world();
    for edit in [
        (&|r: &mut d2_data::tables::Skills| r.aurastat1 = 0xFFFE)
            as &dyn Fn(&mut d2_data::tables::Skills),
        &|r| r.aurastat1 = 359,
        &|r| r.auratargetstate = 200,
        &|r| r.auratargetstate = 0xFFFF,
    ] {
        let mut t2 = t.clone();
        edit(&mut t2.skills[1]);
        assert_eq!(b3_lvl24::attract(&mut f, &t2, &ct, u, 1, 1), 0);
    }
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
}

#[test]
fn attract_sets_the_target_up() {
    let (t, ct, mut f, u, tg, other) = attract_world();
    f.minion_owner.insert(tg, other);
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    let log = f.take_log();
    // T leaves its pack (a leader's pack would be dissolved instead).
    assert!(log.contains(&format!("LeaveLeader({tg})")), "{log:?}");
    assert!(log.contains(&format!("Alignment {{ u: {tg}, a: 1, v: 1 }}")));
    assert!(log.contains(&format!("NodePrepend {{ u: {tg}, slot: 9 }}")));
}

#[test]
fn attract_dissolves_a_leaders_pack() {
    let (t, ct, mut f, u, tg, _) = attract_world();
    f.minion_owner.insert(tg, tg);
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.take_log().contains(&format!("DissolvePack({tg})")));
}

#[test]
fn attract_pulls_the_monsters_in_range() {
    let (t, mut ct, mut f, u, tg, other) = attract_world();
    ct.difficultylevels[0].aicursedivisor = 4;
    f.c.frame = 1000;
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    let g = f.c.units[tg].guid;
    // d = 100 / 4: the other monster targets T (a monster: kind 2).
    assert!(log.contains(&format!(
        "TargetOverride {{ m: {other}, kind: 2, guid: {g} }}"
    )));
    assert!(log.contains(&format!("schedule {other} 10 1025 0 0")));
    // A monster the attract test rejects is left alone.
    let (t, ct, mut f, u, _, other) = attract_world();
    f.alive.remove(&other);
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(!f
        .take_log()
        .iter()
        .any(|s| s.starts_with("TargetOverride") && s.contains(&format!("m: {other},"))));
}

#[test]
fn attract_marks_t_with_the_state_and_its_remove_callback() {
    let (t, ct, mut f, u, tg, _) = attract_world();
    f.c.frame = 10;
    assert_eq!(b3_lvl24::attract(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(tg, 41).expect("state on T");
    assert_eq!(l.expire, 110);
    assert_eq!(l.callback, callback::ATTRACT);
    assert_eq!(l.owner, Some(u));
    // The remove callback: alignment 0, state off, target list remove.
    f.take_log();
    helpers3::remove_alignment(&mut f, tg, 41);
    assert!(!f.has_state(tg, 41));
    let log = f.take_log();
    assert!(log.contains(&format!("Alignment {{ u: {tg}, a: 0, v: 1 }}")));
    assert!(log.contains(&format!("NodeRemove({tg})")));
}

// -----------------------------------------------------------------

#[test]
fn bone_prison_start_needs_a_field_target() {
    let (mut f, u) = world();
    assert_eq!(b3_lvl24::bone_prison_start(&mut f, u), 0, "no target");
    let m = monster(&mut f, (2, 2));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl24::bone_prison_start(&mut f, u), 1);
    f.room_of.insert(m, 2);
    f.town.insert(2);
    assert_eq!(b3_lvl24::bone_prison_start(&mut f, u), 0);
}

// ------------------------------------------------------------------

fn golem_world() -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 0;
    r.summode = 1;
    r.pettype = 3;
    r.petmax = c.f(2);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let item = f.add(FUnit::new(UnitType::Item, 0), (6, 4));
    f.targets.insert(u, item);
    (t, ct, f, u, item)
}

#[test]
fn iron_golem_needs_the_item_and_flags_the_unit() {
    let (t, ct, mut f, u, _) = golem_world();
    assert_eq!(b3_lvl24::iron_golem_start(&mut f, u), 1);
    f.targets.clear();
    assert_eq!(b3_lvl24::iron_golem_start(&mut f, u), 0);
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    // A non-item target fails the start test too.
    let m = monster(&mut f, (1, 1));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t, &ct, u, 1, 1), 0);
    // The item: the flag is set (even when a later step refuses).
    let (t, ct, mut f, u, _) = golem_world();
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t, &ct, u, 0, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    let _ = (t, ct);
}

#[test]
fn iron_golem_summon_class_and_pettype() {
    let (t, ct, mut f, u, _) = golem_world();
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t, &ct, u, 9, 1), 0);
    let mut t2 = t.clone();
    t2.skills[1].summon = 0xFFFF;
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t2, &ct, u, 1, 1), 0);
    // pettype >= count is a refusal here (unlike Dopplezon).
    let mut t3 = t.clone();
    t3.skills[1].pettype = 15;
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t3, &ct, u, 1, 1), 0);
    assert!(!f.take_log().iter().any(|s| s.starts_with("monster")));
}

// The join's re-summon: the body runs with no
// item unit, so the golem is spawned and added as a pet and no item moves.
#[test]
fn golem_summon_without_an_item_unit() {
    let (t, ct, mut f, u, item) = golem_world();
    assert_eq!(b3_lvl24::golem_summon(&mut f, &t, &ct, u, 1, 1, None), 1);
    let log = f.take_log();
    assert!(log
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 3")));
    assert!(!log.iter().any(|s| s.contains(&format!("{item}"))
        && (s.starts_with("ItemLeaveRoom") || s.starts_with("ItemMode"))));
}

#[test]
fn iron_golem_wears_the_item() {
    let (t, ct, mut f, u, item) = golem_world();
    f.no_monsters = true;
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t, &ct, u, 1, 1), 0);
    f.no_monsters = false;
    assert_eq!(b3_lvl24::iron_golem(&mut f, &t, &ct, u, 1, 1), 1);
    let m = f.c.units.len() - 1;
    let log = f.take_log();
    // Flags 1: at T's position, pet type 3, max 2.
    assert!(log.iter().any(|s| s.starts_with("monster 1 (6, 4) 0 1")));
    assert!(log
        .iter()
        .any(|s| s.starts_with("PetAdd") && s.contains("t: 3") && s.contains("max: 2")));
    let at = |p: String| log.iter().position(|s| *s == p);
    let leave = at(format!("ItemLeaveRoom({item})")).expect("T leaves its room");
    let mode = at(format!("ItemMode {{ item: {item}, mode: 4 }}")).expect("mode 4");
    let equip = at(format!("Equip {{ m: {m}, item: {item}, loc: 4 }}")).expect("equip");
    assert!(leave < mode && mode < equip);
    assert!(log.contains(&format!("AllyInfo {{ u: {u}, m: {m} }}")));
}

// ------------------------------------------------------------------

fn conv_world(
    p: i16,
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.auratargetstate = 53;
    r.calc1 = c.f(p);
    r.auralencalc = c.f(100);
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let ct = combat_tables(vec![ms]);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    // An attackable monster: unit flag 0x4 from creation.
    f.c.units[m].flags = 4;
    f.targets.insert(u, m);
    f.c.in_range = true;
    f.c.hostile = true;
    f.c.set(m, 6, 10_000_000);
    stored(
        &mut f,
        u,
        m,
        DamageRecord {
            result: 1,
            ..DamageRecord::default()
        },
    );
    (t, ct, f, u, m)
}

#[test]
fn conversion_refusals() {
    let (t, ct, mut f, u, _) = conv_world(100);
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 9, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
}

#[test]
fn conversion_chance_gates_the_state() {
    // p = 100: roll(100) < 100 always.
    let (t, ct, mut f, u, m) = conv_world(100);
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 53).is_some());
    // p = 0: never; the stored hit is applied instead.
    let (t, ct, mut f, u, m) = conv_world(0);
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 53).is_none());
    assert!(f.c.units[u].combat.is_empty(), "apply_melee consumed it");
    // No pair record: ok = 0 with no draw.
    let (t, ct, mut f, u, m) = conv_world(100);
    f.c.units[u].combat.clear();
    let seed = f.c.units[u].seed;
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 53).is_none());
    assert_eq!(f.c.units[u].seed, seed);
    // The target tests: a hireling, a non-switchable monster, an invalid
    // target state.
    let (t, ct, mut f, u, m) = conv_world(100);
    f.c.units[m].hireling = true;
    b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1);
    assert!(f.list_of(m, 53).is_none());
    let (t, mut ct, mut f, u, m) = conv_world(100);
    ct.monstats[0].switchai = false;
    b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1);
    assert!(f.list_of(m, 53).is_none());
    let (mut t, ct, mut f, u, m) = conv_world(100);
    t.skills[1].auratargetstate = 0xFFFF;
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 53).is_none());
}

#[test]
fn conversion_expiry_and_callback() {
    let (t, ct, mut f, u, m) = conv_world(100);
    f.c.frame = 50;
    assert_eq!(b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(m, 53).unwrap();
    assert_eq!((l.expire, l.callback), (150, callback::CONVERSION));
    assert!(
        !f.c.units[u].combat.is_empty(),
        "the stored hit is not applied"
    );
    // A zero length still lasts one frame.
    let (mut t, ct, mut f, u, m) = conv_world(100);
    t.skills[1].auralencalc = 0xFFFF_FFFF;
    f.c.frame = 50;
    b3_lvl24::conversion_do(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.list_of(m, 53).unwrap().expire, 51);
}

// ------------------------------------------------------------------

#[test]
fn holy_shield_start_needs_a_shield() {
    let (mut f, u) = world();
    assert_eq!(b3_lvl24::holy_shield_start(&mut f, u), 0);
    f.inventory = true;
    assert_eq!(b3_lvl24::holy_shield_start(&mut f, u), 0);
    f.shield = Some(0);
    assert_eq!(b3_lvl24::holy_shield_start(&mut f, u), 1);
    f.inventory = false;
    assert_eq!(b3_lvl24::holy_shield_start(&mut f, u), 0);
}

// ------------------------------------------------------------------

#[test]
fn frenzy_even_and_odd_frames() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    // T none.
    assert_eq!(b3_lvl24::frenzy(&mut f, &t, &ct, u, 1, 1), 0);
    let m = monster(&mut f, (1, 0));
    let m2 = monster(&mut f, (2, 0));
    for x in [m, m2] {
        f.c.units[x].mode = 1;
        f.c.units[x].flags = 0xC;
    }
    f.scan = vec![m, m2];
    f.c.hostile = true;
    f.targets.insert(u, m);
    f.c.in_range = true;
    f.c.set(u, 12, 99);
    f.c.set(u, 19, 100_000);
    f.c.set(m, 12, 1);
    // Even: the unit's combat records for T are freed, cleanup, swing on T.
    f.frame_index.insert(u, 2);
    assert_eq!(b3_lvl24::frenzy(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("FreeCombat {{ u: {u}, target: {m} }}")));
    assert!(log.contains(&format!("attackcleanup {u}")));
    assert!(log.contains(&format!("weaponcleanup {u}")));
    assert!(log.contains(&format!("path {u} TargetUnit(Some({m}))")));
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    // Odd: the flag, no free, swing on the next unit by GUID.
    f.frame_index.insert(u, 3);
    assert_eq!(b3_lvl24::frenzy(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    assert!(!log.iter().any(|s| s.starts_with("FreeCombat")));
    assert!(log.contains(&format!("attackcleanup {u}")));
    assert!(log.contains(&format!("path {u} TargetUnit(Some({m2}))")));
}

// ------------------------------------------------------------------

fn ward_world(
    large: bool,
    small: bool,
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.srvmissileb = 1;
    r.srvmissilec = 2;
    let t = tabs(r, Code::new(), 3);
    let mut ms2: Monstats2 = blank();
    ms2.corpsesel = true;
    ms2.soft = true;
    ms2.large = large;
    ms2.small = small;
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    let m = monster(&mut f, (8, 9));
    f.c.units[m].mode = 12;
    f.targets.insert(u, m);
    f.free_shift = Some((3, 2));
    (t, ct, f, u, m)
}

#[test]
fn grim_ward_needs_a_soft_corpse() {
    let (t, ct, mut f, u, m) = ward_world(false, false);
    assert_eq!(b3_lvl24::grim_ward(&mut f, &t, &ct, u, 9, 1), 0);
    f.c.units[m].mode = 1;
    assert_eq!(
        b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1),
        0,
        "not a corpse"
    );
    f.c.units[m].mode = 12;
    f.targets.clear();
    assert_eq!(b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1), 0, "T none");
    assert!(f.missiles.is_empty());
}

#[test]
fn grim_ward_needs_a_room_and_a_free_point() {
    let (t, ct, mut f, u, m) = ward_world(false, false);
    f.point_rooms.insert((8, 9), None);
    assert_eq!(
        b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1),
        0,
        "no room at the point"
    );
    f.point_rooms.clear();
    f.room_of.remove(&m);
    assert_eq!(
        b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1),
        0,
        "T's room none"
    );
    f.room_of.insert(m, 1);
    f.free_shift = None;
    assert_eq!(
        b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1),
        0,
        "no free point"
    );
    assert!(f.missiles.is_empty());
}

#[test]
fn grim_ward_missile_by_size_and_the_corpse_states() {
    for (large, small, want) in [
        (false, false, 0),
        (false, true, 1),
        (true, true, 2),
        (true, false, 2),
    ] {
        let (t, ct, mut f, u, m) = ward_world(large, small);
        assert_eq!(b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 3), 1);
        let q = f.missiles[0];
        assert_eq!(q.class, want, "large {large} small {small}");
        // Flags 1 at the free point (8 + 3, 9 + 2).
        assert_eq!((q.flags, q.x, q.y, q.skill, q.level), (1, 11, 11, 1, 3));
        assert!(f.has_state(m, 104) && f.has_state(m, 118));
        assert!(f.take_log().contains(&format!("update {m}")));
    }
    // The chosen missile invalid.
    let (mut t, ct, mut f, u, _) = ward_world(true, false);
    t.skills[1].srvmissilec = 0xFFFF;
    assert_eq!(b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1), 0);
    // No monstats2 record for the class.
    let (t, mut ct, mut f, u, _) = ward_world(false, false);
    ct.monstats2.clear();
    assert_eq!(b3_lvl24::grim_ward(&mut f, &t, &ct, u, 1, 1), 0);
}

// ------------------------------------------------------------------

fn hunger_run(p: i16, l: i16, m: i16, miss: bool) -> (i32, BodyFake, usize, usize, i32) {
    let (t, ct, mut f, u, tg) = claws_world(|r, c| {
        r.calc1 = c.f(p);
        r.calc2 = c.f(l);
        r.calc3 = c.f(m);
        if miss {
            r.resultflags = 0;
        }
    });
    f.c.set(u, 21, 10);
    f.c.set(u, 22, 10);
    f.c.set(tg, 6, 10_000_000);
    f.c.set(u, 6, 100);
    f.c.set(u, 7, 10_000_000);
    f.c.set(u, 8, 0);
    f.c.set(u, 9, 10_000_000);
    if miss {
        f.c.set(u, 19, 0);
        f.c.set(tg, 31, 100_000);
    }
    let r = b3_lvl24::hunger(&mut f, &t, &ct, u, 1, 1);
    (r, f, u, tg, 10_000_000)
}

#[test]
fn hunger_refusals_and_flag() {
    let (t, ct, mut f, u, _) = claws_world(|_, _| {});
    assert_eq!(b3_lvl24::hunger(&mut f, &t, &ct, u, 9, 1), 0);
    f.targets.clear();
    f.tpos.clear();
    assert_eq!(b3_lvl24::hunger(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    let (r, f, u, ..) = hunger_run(0, 0, 0, false);
    assert_eq!(r, 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
}

#[test]
fn hunger_adds_physical_and_leech() {
    let (_, f0, _, tg, full) = hunger_run(0, 0, 0, false);
    let base = full - f0.c.get(tg, 6);
    assert!(base > 0);
    // calc1 = 100: physical doubles.
    let (_, f1, _, tg, _) = hunger_run(100, 0, 0, false);
    assert_eq!(full - f1.c.get(tg, 6), 2 * base);
    // calc2 = 20 / calc3 = 30: leeches of the damage done.
    let (_, f2, u, _, _) = hunger_run(0, 20, 30, false);
    assert_eq!(f2.c.get(u, 6) - 100, base * 20 / 100);
    assert_eq!(f2.c.get(u, 8), base * 30 / 100);
}

#[test]
fn hunger_returns_one_on_a_miss() {
    let mut missed = false;
    for _ in 0..50 {
        let (r, f, _, tg, full) = hunger_run(0, 0, 0, true);
        assert_eq!(r, 1);
        missed |= f.c.get(tg, 6) == full;
    }
    assert!(missed);
}

// ------------------------------------------------------------------

fn blast_world() -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurafilter = 0x103;
    r.aurarangecalc = c.f(10);
    r.param3 = 100;
    r.param4 = 50;
    r.param5 = 100;
    r.param6 = 100;
    let t = tabs(r, c, 1);
    let mut ms = monster_rec();
    ms.switchai = true;
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats = vec![ms];
    let (mut f, u) = world();
    f.tpos.insert(u, (4, 1));
    f.c.hostile = true;
    let m = monster(&mut f, (4, 2));
    f.c.units[m].mode = 1;
    // An attackable monster: unit flag 0x4 from creation.
    f.c.units[m].flags = 4;
    f.c.set(m, 6, 10_000_000);
    f.scan = vec![m];
    (t, ct, f, u, m)
}

#[test]
fn mind_blast_refusals_and_range() {
    let (t, ct, mut f, u, m) = blast_world();
    assert_eq!(b3_lvl24::mind_blast(&mut f, &t, &ct, u, 9, 1), 0);
    f.tpos.clear();
    assert_eq!(b3_lvl24::mind_blast(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.lists.is_empty());
    // The range is aurarangecalc (10): a monster 500 away is not reached.
    f.tpos.insert(u, (500, 500));
    assert_eq!(b3_lvl24::mind_blast(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 53).is_none());
}

#[test]
fn mind_blast_converts_with_the_chance_or_hits() {
    // chance DM(1, 100, 100) = 100: always converted.
    let (t, ct, mut f, u, m) = blast_world();
    f.c.frame = 1000;
    assert_eq!(b3_lvl24::mind_blast(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(m, 53).expect("conversion state");
    assert!(
        (1100..1150).contains(&l.expire),
        "e = roll(Param4) + F + Param3"
    );
    assert_eq!(l.callback, callback::MIND_BLAST);
    assert!(
        !f.take_log().iter().any(|s| s.starts_with("reaction")),
        "no damage to a converted unit"
    );
    // A hireling cannot be converted: the damage record is applied.
    let (t, ct, mut f, u, m) = blast_world();
    f.c.units[m].hireling = true;
    assert_eq!(b3_lvl24::mind_blast(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.list_of(m, 53).is_none());
    assert!(f.take_log().contains(&format!("reaction {u} {m}")));
    // A monster that cannot be switched likewise.
    let (t, mut ct, mut f, u, m) = blast_world();
    ct.monstats[0].switchai = false;
    b3_lvl24::mind_blast(&mut f, &t, &ct, u, 1, 1);
    assert!(f.list_of(m, 53).is_none());
}

// ------------------------------------------------------------------

fn flight_world(
    p1: i16,
) -> (
    crate::skills::SkillTables,
    crate::combat::CombatTables,
    BodyFake,
    usize,
    usize,
) {
    let (t, ct, mut f, u, tg) = claws_world(|r, _| {
        r.resultflags = 3;
        r.param1 = i32::from(p1) as _;
        r.param2 = 5;
    });
    f.tpos.insert(u, (30, 40));
    f.c.set(tg, 6, 10_000_000);
    f.c.set(u, 137, 20);
    (t, ct, f, u, tg)
}

#[test]
fn dragon_flight_refusals() {
    let (t, ct, mut f, u, _) = flight_world(0);
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 9, 1), 0);
    f.targets.clear();
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn dragon_flight_even_frames_teleport() {
    let (t, ct, mut f, u, _) = flight_world(0);
    f.frame_index.insert(u, 0);
    f.c.units[u].flags = FLAG_40;
    // The level rule: none / 0 refused.
    f.teleport = None;
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 1), 0);
    f.teleport = Some(0);
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 1), 0);
    // 2 with a blocked line: refused; the flag was cleared on the way.
    f.teleport = Some(2);
    f.blocked = true;
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    f.blocked = false;
    f.take_log();
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.take_log().contains(&format!("place {u} None (30, 40)")));
    // No room: refused.
    f.room_of.remove(&u);
    assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn dragon_flight_odd_frames_kick() {
    let run = |p1| {
        let (t, ct, mut f, u, tg) = flight_world(p1);
        f.frame_index.insert(u, 1);
        assert_eq!(b3_lvl24::dragon_flight(&mut f, &t, &ct, u, 1, 3), 1);
        (10_000_000 - f.c.get(tg, 6), f.take_log())
    };
    let (base, log) = run(0);
    assert!(base > 0, "kick damage dealt");
    // The finisher ran (it clears the pgsv group), and the hit was applied.
    assert!(log.contains(&"cleargroup 0 4".to_string()));
    // Enhanced damage % = Param1 + (L - 1) * Param2 = 0 + 10 at L = 3:
    // 20 kick damage + 10% = 22 (in 1/256 life units).
    assert_eq!(base, 22 * 256);
    // Param1 = 100: 110%.
    let (more, _) = run(100);
    assert_eq!(more, 42 * 256);
}

// ---------------------------------------------------------------- passive

#[test]
fn passive_refresh_sets_the_stats_on_the_passiveitype_layer() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.passivestate = 7;
    r.passiveitype = 0x0F;
    r.passivestat1 = 20;
    r.passivecalc1 = c.f(9);
    r.passivestat2 = 0xFFFF;
    let t = tabs(r, Code(c.0.clone()), 1);
    let (mut f, u) = world();
    passive::refresh(&mut f, &t, u, 1);
    let l = f.lists.iter().find(|l| l.state == 7).expect("state 7 list");
    // Passivestat1 on layer `passiveitype`, markers on 0.
    assert_eq!(l.stats.get(&(0x0F << 16 | 20)), Some(&9));
    assert_eq!(l.stats.get(&20), None);
    assert_eq!(l.stats.get(&passive::PASSIVE_SKILL), Some(&1));
    // `passiveitype` ≤ 0 → layer 0.
    let mut c = Code::new();
    let mut r = body_rec();
    r.passivestate = 7;
    r.passiveitype = 0xFFFF;
    r.passivestat1 = 20;
    r.passivecalc1 = c.f(9);
    r.passivestat2 = 0xFFFF;
    let t = tabs(r, Code(c.0.clone()), 1);
    let (mut f, u) = world();
    passive::refresh(&mut f, &t, u, 1);
    let l = f.lists.iter().find(|l| l.state == 7).expect("state 7 list");
    assert_eq!(l.stats.get(&20), Some(&9));
}
