//! Tests of the batch 2 bodies and helpers on [`super::fake::BodyFake`].

use super::fake::BodyFake;
use super::*;
use crate::combat::DamageRecord;
use crate::skills::fake::{combat_tables, monster_rec, skill_rec, skill_tables, FUnit};
use crate::skills::{SkillEntry, SkillTables, SkillUnits};
use crate::units::UnitType;
use d2_data::tables::Skills;

/// Formula constants: `formula(v)` is the offset of `push i16 v; end`.
pub(crate) struct Code(pub Vec<u8>);

impl Code {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    /// The offset of a formula giving `v`.
    pub fn f(&mut self, v: i16) -> u32 {
        let at = self.0.len() as u32;
        self.0.push(0x08);
        self.0.extend(v.to_le_bytes());
        self.0.push(0x00);
        at
    }
}

/// A skills record with no formulas, states, stats, events or missiles.
pub(crate) fn body_rec() -> Skills {
    let mut r = skill_rec();
    for s in [
        &mut r.srvmissile,
        &mut r.srvmissilea,
        &mut r.srvmissileb,
        &mut r.srvmissilec,
        &mut r.aurastate,
        &mut r.auratargetstate,
        &mut r.srvoverlay,
        &mut r.tgtoverlay,
        &mut r.passivestate,
        &mut r.aurastat1,
        &mut r.aurastat2,
        &mut r.aurastat3,
        &mut r.aurastat4,
        &mut r.aurastat5,
        &mut r.aurastat6,
        &mut r.passivestat1,
        &mut r.passivestat2,
        &mut r.passivestat3,
        &mut r.passivestat4,
        &mut r.passivestat5,
        &mut r.auraevent1,
        &mut r.auraevent2,
        &mut r.auraevent3,
        &mut r.summon,
        &mut r.sumskill1,
        &mut r.sumskill2,
        &mut r.sumskill3,
        &mut r.sumskill4,
        &mut r.sumskill5,
        &mut r.sumumod,
        &mut r.sumoverlay,
    ] {
        *s = 0xFFFF;
    }
    for c in [
        &mut r.aurastatcalc2,
        &mut r.aurastatcalc3,
        &mut r.aurastatcalc4,
        &mut r.aurastatcalc5,
        &mut r.aurastatcalc6,
        &mut r.prgcalc1,
        &mut r.prgcalc2,
        &mut r.prgcalc3,
        &mut r.sumsk1calc,
        &mut r.sumsk2calc,
        &mut r.sumsk3calc,
        &mut r.sumsk4calc,
        &mut r.sumsk5calc,
        &mut r.delay,
    ] {
        *c = 0xFFFF_FFFF;
    }
    r.intown = true;
    r
}

/// Tables with skill 0 (blank) and skill 1 = `r`, the code buffer and
/// `missiles` blank missile rows.
pub(crate) fn tabs(r: Skills, code: Code, missiles: usize) -> SkillTables {
    let mut t = skill_tables(vec![body_rec(), r]);
    t.skills_code = code.0;
    t.missiles = vec![crate::skills::fake::missile_rec(); missiles];
    t
}

/// A world with a player caster (unit 0) at (0, 0) that uses skill 1.
pub(crate) fn world() -> (BodyFake, usize) {
    let mut f = BodyFake::new();
    let mut p = FUnit::new(UnitType::Player, 0);
    p.guid = 0;
    let e = SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    p.skills.push(e);
    p.used = Some(e);
    let u = f.add(p, (0, 0));
    (f, u)
}

/// A monster at `at`.
pub(crate) fn monster(f: &mut BodyFake, at: (i32, i32)) -> usize {
    f.add(FUnit::new(UnitType::Monster, 0), at)
}

// ---------------------------------------------------------------

#[test]
fn ring_offsets_match_the_vectors() {
    assert_eq!(ring_offset(0), (30, 0));
    assert_eq!(ring_offset(8), (21, 21));
    assert_eq!(ring_offset(16), (0, 30));
    assert_eq!(ring_offset(40), (-21, -21));
    assert_eq!(ring_offset(63), (29, -2));
}

#[test]
fn shout_makes_a_ring_and_the_caster_state() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.aurastate = 32;
    r.auralencalc = c.f(100);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.pos.insert(u, (10, 20));
    f.c.frame = 50;
    assert_eq!(dos2::shout(&mut f, &t, u, 1, 3), 1);
    assert_eq!(f.missiles.len(), 64);
    assert!(f
        .missiles
        .iter()
        .all(|m| m.flags == 3 && (m.x, m.y) == (10, 20)));
    assert_eq!((f.missiles[16].target_x, f.missiles[16].target_y), (0, 30));
    let l = f.list_of(u, 32).expect("shout list");
    assert_eq!(l.expire, 150);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
    assert!(f.take_log().contains(&format!("timer {u} 12 150")));
}

#[test]
fn base_stats_level_from_the_owner() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    f.c.set(u, 12, 30);
    // No monlvl rows: the level is set, then 0 (no index).
    assert_eq!(base_stats(&mut f, u, m, 0, 5), 0);
    assert_eq!(f.c.get(m, 12), 27);
    f.c.set(u, 12, 2);
    base_stats(&mut f, u, m, 0, 5);
    assert_eq!(f.c.get(m, 12), 2);
}

#[test]
fn base_stats_adds_monlvl_ac_and_tohit() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    let mut row: d2_data::tables::Monlvl = crate::skills::fake::blank();
    row.ac = 7;
    row.th = 9;
    row.l_ac = 70;
    row.l_th = 90;
    f.monlvl = vec![
        crate::skills::fake::blank(),
        crate::skills::fake::blank(),
        row,
    ];
    assert_eq!(base_stats(&mut f, u, m, 2, 1), 1);
    assert_eq!((f.c.get(m, 31), f.c.get(m, 19)), (7, 9));
    f.l_flag = true;
    base_stats(&mut f, u, m, 2, 1);
    assert_eq!((f.c.get(m, 31), f.c.get(m, 19)), (77, 99));
}

#[test]
fn charge_add_counts_to_three() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.auralencalc = c.f(40);
    r.aurastat2 = 25;
    r.aurastatcalc2 = c.f(10);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    assert_eq!(charge_add(&mut f, &t, u, 1, 1, (60, 100)), 1);
    let l = f.state_list(u, 60).expect("charge list");
    f.list_set(l, 100, 2);
    assert_eq!(charge_add(&mut f, &t, u, 1, 1, (60, 100)), 1);
    assert_eq!(f.list_get(l, 100), 3);
    let added = f.list_get(l, 25);
    f.c.frame = 10;
    assert_eq!(charge_add(&mut f, &t, u, 1, 1, (60, 100)), 1);
    assert_eq!((f.list_get(l, 100), f.list_get(l, 25)), (3, added));
    // The expiry is refreshed even at 3.
    assert_eq!(f.list_expire(l), 50);
    assert_eq!(f.lists[l].callback, callback::CHARGE);
}

#[test]
fn summon_class_from_the_record_or_the_ai_control() {
    let mut r = body_rec();
    r.summon = 1;
    r.summode = 20;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec(), monster_rec()]);
    let (mut f, u) = world();
    assert_eq!(summon_class(&f, &t, &ct, u, 1), (1, 1));
    let mut r = body_rec();
    r.summon = 9;
    r.summode = 3;
    let t = tabs(r, Code::new(), 1);
    // `summon` outside the monstats rows: `mode` is not written (the
    // caller's 0), also on the AI-control fallback.
    assert_eq!(summon_class(&f, &t, &ct, u, 1), (-1, 0));
    f.spawn_class = Some(1);
    assert_eq!(summon_class(&f, &t, &ct, u, 1), (1, 0));
}

#[test]
fn spawn_retries_with_spread_four_and_sets_the_flags() {
    let (mut f, u) = world();
    let m = spawn(
        &mut f,
        Summon {
            flags: 1,
            owner: u,
            class: 5,
            ai: 0,
            mode: 1,
            x: 7,
            y: 8,
            pet_type: 3,
            pet_max: 0,
        },
    )
    .expect("summon");
    assert_eq!(f.c.units[m].flags, 0x8002_0000);
    let log = f.take_log();
    assert_eq!(log[0], "monster 1 (7, 8) 5 1 -1");
    assert!(log.contains(&format!("PetAdd {{ owner: {u}, pet: {m}, t: 3, max: 1 }}")));
    assert!(log.contains(&format!("schedule {m} 2 25 0 0")));
    f.no_monsters = true;
    let q = Summon {
        flags: 5,
        owner: u,
        class: 5,
        ai: 0,
        mode: 1,
        x: 7,
        y: 8,
        pet_type: 3,
        pet_max: 2,
    };
    assert_eq!(spawn(&mut f, q), None);
    assert_eq!(f.take_log().len(), 1, "flag 4: no second try");
    assert_eq!(spawn(&mut f, Summon { flags: 1, ..q }), None);
    assert_eq!(f.take_log()[1], "monster 1 (7, 8) 5 1 4");
}

#[test]
fn skeleton_components_by_mastery_level() {
    let mut r = body_rec();
    r.param1 = 100;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u) = world();
    let m = f.add(FUnit::new(UnitType::Monster, 363), (1, 1));
    components(&mut f, &t, u, m, 1, 3);
    // Row 1 is all 0; the shield draw (p 100) always passes.
    let log = f.take_log();
    assert_eq!(log.len(), 9);
    assert!(log.contains(&format!("Component {{ m: {m}, k: 7, v: 0 }}")));
    let mage = f.add(FUnit::new(UnitType::Monster, 364), (1, 1));
    components(&mut f, &t, u, mage, 1, 3);
    let log = f.take_log();
    assert!(log.last().unwrap().starts_with("AiParams"));
}

#[test]
fn inferno_and_arctic_blast_aim_beyond_finite_range_without_moving() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(5);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    // Both skills share start 11 / do 19. A target beyond the stream's
    // finite reach supplies its aim, not a destination for the caster.
    let before = f.position(u);
    f.tpos.insert(u, (1_000, 1));
    assert_eq!(starts2::inferno(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.has_state(u, 12));
    assert_eq!(f.entry_param(u, &f.c.units[u].used.unwrap(), 1), 0);
    assert_eq!(dos2::inferno_cast(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.missiles.is_empty());
    assert_eq!(dos2::inferno_cast(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 1);
    assert_eq!((f.missiles[0].flags, f.missiles[0].range), (0x8020, 5));
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (1_000, 1));
    assert_eq!(f.position(u), before);
}

#[test]
fn link_source_sets_and_clears() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    link_source(&mut f, m, Some(u));
    let l = f.state_list(m, 98).expect("source list");
    assert_eq!((f.list_get(l, 353), f.list_get(l, 354)), (0, 0));
    assert_eq!(f.unit_c8(m), 0x400);
    link_source(&mut f, m, None);
    assert_eq!(f.unit_c8(m), 0);
    assert!(f.state_list(m, 98).is_none());
}

// ---------------------------------------------------------------

#[test]
fn charge_strike_and_telekinesis_starts() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurarangecalc = c.f(3);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    assert_eq!(starts2::charge_strike(&mut f, u), 0);
    let m = monster(&mut f, (3, 0));
    f.targets.insert(u, m);
    f.c.in_range = true;
    assert_eq!(starts2::charge_strike(&mut f, u), 1);
    f.c.hostile = true;
    assert_eq!(starts2::telekinesis(&mut f, &t, u, 1, 1), 1);
    f.pos.insert(m, (3, 1));
    assert_eq!(starts2::telekinesis(&mut f, &t, u, 1, 1), 0, "d² 10 > 9");
}

#[test]
fn zeal_start_stores_the_hits_and_the_target() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(4);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    assert_eq!(starts2::zeal(&mut f, &t, &ct, u, 1, 1), 0, "nothing near");
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    assert_eq!(starts2::zeal(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(
        (1..=3).map(|i| f.entry_param(u, &e, i)).collect::<Vec<_>>(),
        [4, 1, f.c.units[m].guid as i32]
    );
}

// ---------------------------------------------------------------

#[test]
fn multiple_shot_vector() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(5);
    r.calc3 = c.f(1);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (10, 1));
    // Target (10, 1) keeps the step (0, −1) (dx = 10, dy = 1).
    assert_eq!(dos2::side_step((10, 0)), (0, -1));
    f.tpos.insert(u, (10, 0));
    // A target at y = 0 fails the target position: return 1, nothing.
    assert_eq!(dos2::multiple_shot(&mut f, &t, u, 1, 1), 1);
    assert!(f.missiles.is_empty());
    f.pos.insert(u, (0, 1));
    f.tpos.insert(u, (10, 1));
    assert_eq!(dos2::multiple_shot(&mut f, &t, u, 1, 1), 1);
    let got: Vec<_> = f
        .missiles
        .iter()
        .map(|m| (m.target_x, m.target_y, m.flags & 0x1_0000 != 0))
        .collect();
    assert_eq!(
        got,
        [
            (10, 3, true),
            (10, 2, true),
            (10, 1, false),
            (10, 0, true),
            (10, -1, true)
        ]
    );
}

#[test]
fn dual_claws_clear_then_set_flag_40() {
    let mut r = body_rec();
    r.aurastate = 30;
    r.aurastat1 = 100;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let a = f.c.add_item(crate::skills::fake::FItem {
        types: vec![45],
        ..Default::default()
    });
    let b = f.c.add_item(crate::skills::fake::FItem {
        types: vec![45],
        ..Default::default()
    });
    f.c.units[u].items.insert(4, a);
    f.c.units[u].items.insert(5, b);
    f.inventory = true;
    f.c.units[u].flags = FLAG_40;
    f.frame_index.insert(u, 0);
    assert_eq!(dos2::claws(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, 0);
    f.frame_index.insert(u, 1);
    dos2::claws(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.c.units[u].flags & FLAG_40, FLAG_40);
}

#[test]
fn inner_sight_applies_the_state_to_scanned_units() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.auratargetstate = 40;
    r.aurastat1 = 31;
    r.aurastatcalc1 = c.f(-50);
    r.auralencalc = c.f(100);
    r.aurarangecalc = c.f(10);
    r.aurafilter = 2;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (2, 2));
    f.c.units[m].mode = 1;
    let mut ms2: d2_data::tables::Monstats2 = crate::skills::fake::blank();
    ms2.isatt = true;
    let mut ct = ct;
    ct.monstats2 = vec![ms2];
    f.scan = vec![u, m];
    assert_eq!(dos2::inner_sight(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.list_of(m, 40).expect("inner sight list");
    assert_eq!(l.stats.get(&31), Some(&-50));
    assert_eq!(f.c.units[u].flags & FLAG_40, 0, "no flag 0x40");
}

#[test]
fn shape_shift_twice_sets_the_delay() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 139;
    r.auralencalc = c.f(500);
    r.delay = c.f(25);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.state_groups.insert(139, 5);
    assert_eq!(dos2::shape_shift(&mut f, &t, u, 1, 1), 1);
    let l = f.state_list(u, 139).expect("shape list");
    assert_eq!(f.lists[l].callback, callback::SHAPE);
    assert!(f.take_log().contains(&format!("entrymode {u} 1 10")));
    assert_eq!(dos2::shape_shift(&mut f, &t, u, 1, 1), 0);
    assert!(f.take_log().iter().any(|s| s.starts_with("delay")));
}

#[test]
fn damage_aura_never_charges_a_player() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 35;
    r.auratargetstate = 36;
    r.aurarangecalc = c.f(5);
    r.mana = 1;
    r.manashift = 8;
    r.perdelay = 0xFFFF_FFFF;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.c.set(u, 8, 10_000);
    assert_eq!(dos2::damage_aura(&mut f, &t, &ct, u, 1, 1, false), 1);
    // Edge case 9: state 85 off, mana unchanged.
    assert!(!f.has_state(u, 85));
    assert_eq!(f.c.get(u, 8), 10_000);
}

#[test]
fn meteor_needs_a_free_point_within_100() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (40, 30));
    f.collides = true;
    assert_eq!(dos2::meteor(&mut f, &t, u, 1, 1), 0);
    f.collides = false;
    assert_eq!(dos2::meteor(&mut f, &t, u, 1, 1), 1);
    assert_eq!((f.missiles[0].flags, f.missiles[0].x), (1, 40));
    f.tpos.insert(u, (101, 1));
    assert_eq!(dos2::meteor(&mut f, &t, u, 1, 1), 0, "distance 101");
}

#[test]
fn fend_returns_zero_and_rewinds_for_the_next_hit() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc2 = c.f(0);
    r.param2 = 7;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let m = monster(&mut f, (1, 0));
    let m2 = monster(&mut f, (1, 1));
    f.c.in_range = true;
    f.c.hostile = true;
    f.scan = vec![m, m2];
    for x in [u, m, m2] {
        f.c.set(x, 12, 1);
    }
    // Flags 0x4 and 0x8: the 0x80 / 0x400 tests of `next_unit`.
    f.c.units[m2].flags = 0xC;
    f.set_entry_param_of(u, &e, 1, 2);
    f.set_entry_param_of(u, &e, 2, 1);
    f.set_entry_param_of(u, &e, 3, f.c.units[m].guid as i32);
    assert_eq!(dos2::fend(&mut f, &t, &ct, u, 1, 1), 0);
    assert_eq!(f.entry_param(u, &e, 1), 1);
    assert_eq!(f.entry_param(u, &e, 3), f.c.units[m2].guid as i32);
    assert!(f.take_log().contains(&format!("rewind {u} 7")));
}

#[test]
fn guided_arrow_marks_homing_and_packs_the_offset() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(0);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.pos.insert(u, (5, 5));
    f.tpos.insert(u, (3, 9));
    assert_eq!(dos2::guided_arrow(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.missiles[0].flags, 0x420);
    let log = f.take_log();
    let packed = 4 * 0x10000 - 2; // dx = -2 borrows from the high half
    assert!(log
        .iter()
        .any(|s| s.contains("MissileData28") && s.ends_with("v: 2 }")));
    assert!(log.iter().any(|s| s.contains(&format!("v: {packed} }}"))));
}

#[test]
fn armageddon_draws_and_schedules_the_active_timer() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurastate = 50;
    r.auralencalc = c.f(0);
    r.param4 = 6;
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.c.frame = 100;
    let before = f.c.units[u].seed;
    assert_eq!(dos2::armageddon(&mut f, &t, u, 1, 2), 1);
    assert_ne!(f.c.units[u].seed, before, "one draw");
    let l = f.state_list(u, 50).unwrap();
    assert_eq!(f.lists[l].expire, 101, "max(len, 1)");
    let log = f.take_log();
    assert!(log.contains(&format!("schedule {u} 5 106 1 2")));
}

#[test]
fn raise_skeleton_removes_the_corpse_and_spawns_at_it() {
    let mut r = body_rec();
    r.summon = 0;
    r.pettype = 3;
    let t = tabs(r, Code::new(), 1);
    let mut ms2: d2_data::tables::Monstats2 = crate::skills::fake::blank();
    ms2.corpsesel = true;
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2 = vec![ms2];
    let (mut f, u) = world();
    let corpse = monster(&mut f, (4, 6));
    f.c.units[corpse].mode = 12;
    f.targets.insert(u, corpse);
    assert_eq!(dos2::raise_skeleton(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("RoomDelete({corpse})")));
    assert!(log.contains(&format!("RemoveUnit({corpse})")));
    assert!(log.iter().any(|s| s.starts_with("monster 1 (4, 6) 0")));
}

#[test]
fn twister_fans_jittered_missiles() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(3);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (9, 9));
    assert_eq!(dos2::twister(&mut f, &t, u, 1, 1), 1);
    let args: Vec<_> = f.missiles.iter().map(|m| m.init).collect();
    assert_eq!(args, [0, 1, 2].map(|i| Some((init_cb::JITTER, i))));
    let _ = DamageRecord::default();
}
