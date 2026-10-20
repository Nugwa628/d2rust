//! Tests of the batch 3 bodies and helpers on [`super::fake::BodyFake`].

use super::fake::{stored, BodyFake};
use super::tests2::{body_rec, monster, tabs, world, Code};
use super::*;
use crate::combat::DamageRecord;
use crate::rng::Seed;
use crate::skills::fake::{blank, combat_tables, monster_rec, FItem, FUnit};
use crate::skills::SkillUnits;
use crate::units::UnitType;
use d2_data::tables::Monstats2;

fn corpse_tables() -> crate::combat::CombatTables {
    let mut ms2: Monstats2 = blank();
    ms2.corpsesel = true;
    ms2.soft = true;
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.monstats2 = vec![ms2];
    ct
}

fn corpse(f: &mut BodyFake, at: (i32, i32)) -> usize {
    let m = monster(f, at);
    f.c.units[m].mode = 12;
    m
}

// ---------------------------------------------------------------

#[test]
fn player_count_multiplier() {
    assert_eq!(player_mult(1, 3), 16);
    assert_eq!(player_mult(1, 9), 56);
    assert_eq!(player_mult(1, 1), 0);
    assert_eq!(player_mult(0, 8), 0);
}

#[test]
fn potion_code_rows() {
    let mut r = body_rec();
    r.param3 = 30;
    r.param4 = 10;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u) = world();
    // Act 0 (the fake's room act), Normal: row 0.
    let code = |f: &mut BodyFake, lo: u32| {
        f.c.units[u].seed = Seed::new(lo, 0);
        potion_code(f, &t, u, 1)
    };
    // A seed whose next step gives r mod 100: find one per band.
    let pick = |want: std::ops::Range<u32>| {
        (0u32..)
            .find(|&lo| want.contains(&(Seed::new(lo, 0).step() % 100)))
            .unwrap()
    };
    assert_eq!(code(&mut f, pick(0..30)), Some(*b"mp2 "));
    assert_eq!(code(&mut f, pick(30..40)), Some(*b"rvs "));
    assert_eq!(code(&mut f, pick(40..100)), Some(*b"hp2 "));
    f.c.difficulty = 1;
    // Act 0 + 5: row 5 (hp4 / mp4 / rvl).
    assert_eq!(code(&mut f, pick(40..100)), Some(*b"hp4 "));
}

#[test]
fn jitter_caps_frames_and_reseeds() {
    #[derive(Default)]
    struct M {
        frames: i32,
        log: Vec<String>,
    }
    impl JitterMissile for M {
        type Missile = ();
        fn total_frames(&self, _: ()) -> i32 {
            self.frames
        }
        fn set_frames(&mut self, _: (), v: i32) {
            self.log.push(format!("frames {v}"));
        }
        fn path_target_x(&self, _: ()) -> i32 {
            40
        }
        fn set_seed(&mut self, _: (), s: Seed) {
            self.log.push(format!("seed {} {}", s.lo, s.hi));
        }
        fn set_path_type(&mut self, _: (), ty: u8) {
            self.log.push(format!("type {ty}"));
        }
        fn set_steps(&mut self, _: (), n: i32) {
            self.log.push(format!("steps {n}"));
        }
        fn compute_path(&mut self, _: ()) {
            self.log.push("compute".into());
        }
    }
    let mut m = M {
        frames: 90,
        ..M::default()
    };
    jitter(&mut m, (), 3);
    assert_eq!(
        m.log,
        ["frames 77", "seed 43 666", "type 10", "steps 77", "compute"]
    );
}

#[test]
fn scatter_with_one_missile_aims_at_the_target() {
    let (mut f, u) = world();
    f.tpos.insert(u, (12, 7));
    assert_eq!(scatter(&mut f, u, 0, 1, 5, 1, 1), 1);
    assert_eq!(f.missiles.len(), 1);
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (12, 7));
    assert_eq!(f.c.units[u].seed, Seed::init_low(12), "re-seeded from tx");
    f.missiles.clear();
    scatter(&mut f, u, 0, 3, 1, 1, 1);
    assert_eq!(f.missiles.len(), 1, "r < 2: one missile");
}

#[test]
fn leap_clamp_and_candidates() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurarangecalc = c.f(10);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    // No free point: the three candidates are tried and fail.
    assert_eq!(leap_clamp(&mut f, &t, u, 1, 1, (20, 0)), None);
    f.free_shift = Some((0, 0));
    assert_eq!(leap_clamp(&mut f, &t, u, 1, 1, (20, 0)), Some((10, 0)));
    // Without a room at (10, 0) the side candidates (10, −2), (10, 2)
    // are 11 away (> r = 10): no landing point.
    f.point_rooms.insert((10, 0), None);
    assert_eq!(leap_clamp(&mut f, &t, u, 1, 1, (20, 0)), None);
    // Unclamped (8, 0) without a room: the second candidate (8, −2),
    // 9 away, is taken.
    f.point_rooms.insert((8, 0), None);
    assert_eq!(leap_clamp(&mut f, &t, u, 1, 1, (8, 0)), Some((8, -2)));
}

#[test]
fn charge_hit_frame_reads_record_zero() {
    let (mut f, u) = world();
    assert_eq!(hit_frame(&f, u), 7);
    f.sequence = Some(vec![[0, 0, 0, 0, 0, 1], [0, 0, 0, 0, 0, 0]]);
    assert_eq!(hit_frame(&f, u), 7);
    f.sequence = Some(vec![[0; 6], [0, 0, 0, 0, 0, 1]]);
    assert_eq!(hit_frame(&f, u), -1);
}

#[test]
fn poison_explosion_burst_offsets() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    let ct = corpse_tables();
    let (mut f, u) = world();
    let k = corpse(&mut f, (50, 50));
    f.targets.insert(u, k);
    assert_eq!(b3_lvl18::poison_explosion(&mut f, &t, &ct, u, 1, 1), 1);
    let got: Vec<_> = f
        .missiles
        .iter()
        .map(|m| (m.target_x, m.target_y))
        .collect();
    assert_eq!(
        got,
        [
            (0, 2),
            (2, 2),
            (2, 0),
            (2, -2),
            (0, -2),
            (-2, -2),
            (-2, 0),
            (-2, 2)
        ]
    );
    assert!(f
        .missiles
        .iter()
        .all(|m| m.flags == 0x17 && (m.x, m.y) == (50, 50)));
    assert!(f.c.has_state(k, 118));
}

#[test]
fn conversion_scales_a_higher_level_monster() {
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 1));
    f.c.set(m, 12, 40);
    f.c.set(u, 12, 20);
    f.c.set(m, 7, 400 << 8); // stat_max(6) is stat 7 in the fake
    f.c.set(m, 6, 200 << 8);
    assert_eq!(
        conversion(&mut f, m, u, 53, 100, callback::CONVERSION, false),
        1
    );
    let l = f.state_list(m, 109).expect("conversion_save");
    assert_eq!((f.list_get(l, 176), f.list_get(l, 177)), (40, 400));
    assert_eq!(
        (f.c.get(m, 12), f.c.get(m, 7), f.c.get(m, 6)),
        (20, 200 << 8, 100 << 8)
    );
}

#[test]
fn whirlwind_gaps_and_pacing() {
    assert_eq!([26, 25, 11].map(ww_gap), [16, 14, 4]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let w = f.c.add_item(FItem::default());
    f.c.units[u].weapon = Some(w);
    f.c.expansion = true;
    f.frames = Some(11);
    f.c.frame = 100;
    assert_eq!(ww_pacing(&mut f, u, &e), 1, "first call");
    assert_eq!(f.entry_param(u, &e, 4), 104);
    f.set_entry_param_of(u, &e, 4, 100);
    assert_eq!(ww_pacing(&mut f, u, &e), 1);
    assert_eq!(f.entry_param(u, &e, 4), 104);
}

// ---------------------------------------------------------------

#[test]
fn sacrifice_self_damage_from_the_stored_hit() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc2 = c.f(8);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl01::sacrifice(&mut f, &t, &ct, u, 1, 1), 1, "no entry");
    assert!(f.take_log().is_empty());
    f.c.set(m, 6, 1280);
    stored(
        &mut f,
        u,
        m,
        DamageRecord {
            physical: 2560,
            ..DamageRecord::default()
        },
    );
    assert_eq!(b3_lvl01::sacrifice(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.take_log().contains(&format!("reaction {u} {u}")));
}

#[test]
fn smite_hit_class_and_no_shield() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.in_range = true;
    assert_eq!(
        b3_lvl01::smite(&mut f, &t, &ct, u, 1, 1),
        0,
        "player without shield"
    );
    // A monster caster smites (A2 damage); the stored entry is applied
    // at once by `apply_melee`.
    let mm = monster(&mut f, (2, 0));
    f.targets.insert(mm, u);
    f.c.set(mm, 12, 1);
    f.c.set(mm, 19, 100_000);
    f.c.set(u, 12, 1);
    assert_eq!(b3_lvl01::smite(&mut f, &t, &ct, mm, 1, 1), 1);
    assert!(f.c.units[mm].combat.is_empty());
}

#[test]
fn find_potion_uses_the_corpse_even_on_a_failed_roll() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(0);
    let t = tabs(r, c, 1);
    let ct = corpse_tables();
    let (mut f, u) = world();
    let k = corpse(&mut f, (3, 3));
    f.targets.insert(u, k);
    assert_eq!(b3_lvl01::find_potion(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.c.has_state(k, 118));
    assert!(!f.take_log().iter().any(|s| s.starts_with("DropItem")));
    assert_eq!(
        b3_lvl01::find_potion(&mut f, &t, &ct, u, 1, 1),
        0,
        "used up"
    );
}

#[test]
fn dragon_talon_start_stores_the_kicks_left() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(2);
    r.param1 = 5;
    r.param2 = 7;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    assert_eq!(b3_lvl01::dragon_talon_start(&mut f, &t, &ct, u, 1, 6), 0);
    assert_eq!(f.entry_param(u, &e, 1), 0);
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.in_range = true;
    f.c.set(u, 12, 1);
    f.c.set(m, 12, 1);
    assert_eq!(b3_lvl01::dragon_talon_start(&mut f, &t, &ct, u, 1, 6), 1);
    assert_eq!(f.entry_param(u, &e, 1), 1);
    assert_eq!(ln12(&t, 1, 1), 5);
    assert_eq!(ln12(&t, 1, 5), 33);
}

// ---------------------------------------------------------------

#[test]
fn corpse_explosion_radius_split() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.aurarangecalc = c.f(7);
    r.calc1 = c.f(100);
    r.calc2 = c.f(100);
    r.calc3 = c.f(0);
    let t = tabs(r, c, 1);
    let ct = corpse_tables();
    let (mut f, u) = world();
    let k = corpse(&mut f, (10, 10));
    f.c.units[k].kind = UnitType::Player; // base maxhp path
    f.c.set(k, 7, 1000);
    // The player kind fails the corpse test: refused.
    f.targets.insert(u, k);
    assert_eq!(b3_lvl06::corpse_explosion(&mut f, &t, &ct, u, 1, 1), 0);
    let k = corpse(&mut f, (10, 10));
    f.targets.insert(u, k);
    let near = monster(&mut f, (13, 10));
    let far = monster(&mut f, (14, 10));
    f.scan = vec![near, far];
    f.c.hostile = true;
    assert_eq!(b3_lvl06::corpse_explosion(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.c.has_state(k, 104));
}

#[test]
fn shock_field_one_missile() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.prgcalc1 = c.f(1);
    r.aurarangecalc = c.f(5);
    let t = tabs(r, c, 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (8, 8));
    assert_eq!(b3_lvl06::shock_field(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 1);
}

// ---------------------------------------------------------------

#[test]
fn bone_wall_makers_perpendicular() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.summon = 0;
    r.srvmissilea = 0;
    r.calc2 = c.f(8);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.tpos.insert(u, (10, 1));
    f.pos.insert(u, (0, 1));
    assert_eq!(b3_lvl12::bone_wall(&mut f, &t, &ct, u, 1, 1), 1);
    let got: Vec<_> = f
        .missiles
        .iter()
        .map(|m| (m.target_x, m.target_y))
        .collect();
    assert_eq!(got, [(10, -9), (10, 11)]);
    assert!(f
        .take_log()
        .iter()
        .any(|s| s.contains("MissileData2C") && s.ends_with("v: 4 }")));
}

#[test]
fn charge_velocity_vector() {
    let mut r = body_rec();
    r.param1 = 150;
    let t = tabs(r, Code::new(), 1);
    let mut ct = combat_tables(vec![monster_rec()]);
    ct.charstats[0].runvelocity = 9;
    let (mut f, u) = world();
    f.c.set(u, 67, 100);
    assert_eq!(b3_lvl12::charge_start(&mut f, &t, &ct, u, 1), 1);
    assert!(f.take_log().contains(&format!("path {u} Velocity(5760)")));
}

#[test]
fn find_item_bands() {
    let p = [5, 60, 30, 5];
    assert_eq!(b3_lvl12::find_item_quality(4, p), 1);
    assert_eq!(b3_lvl12::find_item_quality(5, p), 2);
    assert_eq!(b3_lvl12::find_item_quality(64, p), 2);
    assert_eq!(b3_lvl12::find_item_quality(65, p), 3);
    assert_eq!(b3_lvl12::find_item_quality(95, p), 4);
    assert_eq!(b3_lvl12::find_item_quality(99, p), 4);
}

// ---------------------------------------------------------------

#[test]
fn charged_strike_bolts_from_the_target() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(2);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let m = monster(&mut f, (4, 2));
    f.targets.insert(u, m);
    assert_eq!(b3_lvl18::charged_strike(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 2);
    let m0 = f.missiles[0];
    assert_eq!((m0.x, m0.y, m0.target_x, m0.target_y), (4, 2, 8, 4));
}

#[test]
fn fire_wall_targets() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    let (mut f, u) = world();
    f.tpos.insert(u, (10, 1));
    f.path_target = (10, 1);
    f.pos.insert(u, (0, 1));
    assert_eq!(b3_lvl18::fire_wall(&mut f, &t, u, 1, 1), 1);
    let got: Vec<_> = f
        .missiles
        .iter()
        .map(|m| (m.target_x, m.target_y))
        .collect();
    assert_eq!(got, [(10, -9), (10, 11)]);
}

#[test]
fn vengeance_hit_classes_cycle() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.set(u, 12, 99);
    f.c.set(u, 19, 100_000);
    f.c.set(m, 12, 1);
    let mut classes = Vec::new();
    for _ in 0..4 {
        f.c.units[u].combat.clear();
        b3_lvl18::vengeance(&mut f, &t, &ct, u, 1, 1);
        let rec = f.c.units[u].combat.first().map(|x| x.record);
        classes.push(rec.map_or(0, |r| r.hit_class));
    }
    let hits = classes.iter().filter(|&&c| c != 0).count();
    // Every hit advances the cycle 0x20, 0x30, 0x40.
    assert_eq!(f.entry_param(u, &e, 1), (hits as i32) % 3);
}

#[test]
fn blade_fury_one_blade_per_gap() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.prgcalc1 = c.f(3);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    f.tpos.insert(u, (5, 5));
    f.c.frame = 100;
    assert_eq!(b3_lvl18::blade_fury(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 1), 102);
    f.c.frame = 102;
    assert_eq!(b3_lvl18::blade_fury(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.frame = 103;
    assert_eq!(b3_lvl18::blade_fury(&mut f, &t, &ct, u, 1, 1), 1);
}

#[test]
fn leap_attack_aim_beyond_the_target() {
    let (mut f, u) = world();
    let m = monster(&mut f, (10, 0));
    assert_eq!(leap_aim(&mut f, u, Some(m)), Some((12, 0)));
    f.blocked = true;
    assert_eq!(leap_aim(&mut f, u, Some(m)), None);
}

// ---------------------------------------------------------------

#[test]
fn strafe_and_fend_counts() {
    assert_eq!(b3_lvl24::strafe_count(7, 2, 5), 5);
    assert_eq!(b3_lvl24::strafe_count(1, 2, 5), 2);
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(5);
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let ms: Vec<_> = (0..3).map(|i| monster(&mut f, (1, i))).collect();
    for &m in &ms {
        f.c.units[m].flags = 0xC;
    }
    f.c.hostile = true;
    f.c.in_range = true;
    f.scan = ms.clone();
    f.targets.insert(u, ms[0]);
    assert_eq!(b3_lvl24::fend_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 1), 3);
}

#[test]
fn bone_prison_segment_positions() {
    let mut r = body_rec();
    r.summon = 0;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    f.tpos.insert(u, (50, 50));
    assert_eq!(b3_lvl24::bone_prison(&mut f, &t, &ct, u, 1, 1), 1);
    let at: Vec<_> = f
        .take_log()
        .into_iter()
        .filter(|s| s.starts_with("monster"))
        .collect();
    assert_eq!(at.len(), 12);
    assert!(at[0].starts_with("monster 1 (49, 46)"));
    assert!(at[11].starts_with("monster 1 (47, 47)"));
}

#[test]
fn volcano_stores_its_seed_word() {
    let mut r = body_rec();
    r.srvmissilea = 1;
    let t = tabs(r, Code::new(), 2);
    let (mut f, u) = world();
    f.tpos.insert(u, (6, 6));
    assert_eq!(b3_lvl24::volcano(&mut f, &t, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.iter().any(|s| s.starts_with("MissileData28")));
    assert!(log.iter().any(|s| s.starts_with("MsgA3")));
}

// ---------------------------------------------------------------

#[test]
fn hydra_positions() {
    let mut r = body_rec();
    r.summon = 0;
    r.pettype = 3;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec(), monster_rec(), monster_rec()]);
    let (mut f, u) = world();
    f.tpos.insert(u, (20, 20));
    assert_eq!(b3_lvl30::hydra(&mut f, &t, &ct, u, 1, 1), 1);
    let at: Vec<_> = f
        .take_log()
        .into_iter()
        .filter(|s| s.starts_with("monster"))
        .collect();
    assert_eq!(
        at,
        [
            "monster 1 (19, 19) 0 0 -1",
            "monster 1 (20, 20) 1 0 -1",
            "monster 1 (21, 19) 2 0 -1"
        ]
    );
}

#[test]
fn redemption_counts_corpses_and_pays() {
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
    let t = tabs(r, c, 1);
    let ct = corpse_tables();
    let (mut f, u) = world();
    f.c.set(u, 8, 1000);
    f.c.set(u, 9, 5000);
    f.c.set(u, 7, 5000);
    let k = corpse(&mut f, (2, 2));
    f.scan = vec![k];
    // The filter: monsters in mode 12 need 0x1000.
    let mut t = t;
    t.skills[1].aurafilter = 0x1002;
    assert_eq!(b3_lvl30::redemption(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.c.has_state(k, 99));
    assert!(f.c.has_state(u, 85));
    assert_eq!(f.c.get(u, 8), 1000 - 256);
    assert_eq!(f.c.get(u, 6), 256);
}

#[test]
fn whirlwind_start_in_melee_range_swings() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    assert_eq!(b3_lvl30::whirlwind_start(&mut f, &t, &ct, u, 1, 1), 0);
    assert!(f.take_log().iter().any(|s| s.starts_with("MsgA5")));
    let m = monster(&mut f, (1, 1));
    f.targets.insert(u, m);
    f.c.in_range = true;
    assert_eq!(b3_lvl30::whirlwind_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f
        .take_log()
        .iter()
        .any(|s| s.starts_with("UnitModeRequest")));
}

// ---------------------------------------------------------------- settled answers

#[test]
fn vengeance_negative_index_keeps_c_remainder() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let m = monster(&mut f, (1, 0));
    f.targets.insert(u, m);
    f.c.in_range = true;
    f.c.hostile = true;
    f.c.set(u, 12, 99);
    f.c.set(u, 19, 100_000);
    f.c.set(m, 12, 1);
    f.set_entry_param_of(u, &e, 1, -2);
    let mut class = None;
    for _ in 0..64 {
        f.c.units[u].combat.clear();
        b3_lvl18::vengeance(&mut f, &t, &ct, u, 1, 1);
        if f.entry_param(u, &e, 1) != -2 {
            class = f.c.units[u].combat.first().map(|x| x.record.hit_class);
            break;
        }
    }
    // k = −2: no hit class; E param 1 := (−1) rem 3 = −1 (not 2).
    assert_eq!(class, Some(0));
    assert_eq!(f.entry_param(u, &e, 1), -1);
}

#[test]
fn strafe_rewinds_only_with_a_next_target() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.aurarangecalc = c.f(10);
    r.param6 = 3;
    let t = tabs(r, c, 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, u) = world();
    let e = f.c.units[u].used.unwrap();
    let k = monster(&mut f, (2, 0));
    f.c.units[k].guid = 5;
    f.set_entry_param_of(u, &e, 1, 3);
    f.set_entry_param_of(u, &e, 2, 1);
    f.set_entry_param_of(u, &e, 3, 5);
    // No other unit: no K2, no rewind; the body still returns 1.
    assert_eq!(b3_lvl24::strafe(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(!f.take_log().iter().any(|s| s.starts_with("rewind")));
    let k2 = monster(&mut f, (3, 0));
    f.c.units[k2].guid = 9;
    f.c.units[k2].flags = 0xC;
    f.c.hostile = true;
    f.scan = vec![k2];
    assert_eq!(b3_lvl24::strafe(&mut f, &t, &ct, u, 1, 1), 1);
    let log = f.take_log();
    assert!(log.iter().any(|s| s == &format!("rewind {u} 3")), "{log:?}");
}

#[test]
fn burst_rings_follow_the_steps() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, u) = world();
    assert_eq!(burst(&mut f, &t, u, u, 0, 1, 1, (2, 4, 0)), 1);
    // Ring 1: i = 0, 4, 8, 12; ring 2: i = 0, 2, …, 14.
    assert_eq!(f.missiles.len(), 4 + 8);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "negative burst step2")]
fn burst_negative_step_is_fatal() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, u) = world();
    burst(&mut f, &t, u, u, 0, 1, 1, (-1, 4, 0));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "negative skeleton mastery level")]
fn skeleton_negative_mastery_is_fatal() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, u) = world();
    let m = f.add(FUnit::new(UnitType::Monster, 363), (1, 1));
    f.c.units[u].states.push(97);
    let mut l = super::fake::FList {
        state: 97,
        unit: Some(u),
        ..Default::default()
    };
    l.stats.insert(351, -1);
    f.lists.push(l);
    components(&mut f, &t, u, m, 1, 1);
}

#[test]
fn imp_teleport_riding_uses_the_written_point() {
    let mut r = body_rec();
    r.aurastate = 143;
    let t = tabs(r, Code::new(), 1);
    let ct = combat_tables(vec![monster_rec()]);
    let (mut f, o) = world();
    let imp = monster(&mut f, (1, 1));
    f.c8.insert(imp, 0x400);
    f.missile_owners.insert(imp, o);
    // No target unit: the path's target point, even with a 0 coordinate.
    f.path_target = (7, 0);
    assert_eq!(b4_more::imp_teleport(&mut f, &t, &ct, imp, 1, 1), 1);
    assert!(f.take_log().contains(&format!("place {imp} None (7, 0)")));
}

#[test]
fn baal_tentacle_spawn_info_for_other_keys() {
    let mut rows = vec![monster_rec(); 300];
    for (i, m) in rows.iter_mut().enumerate() {
        m.baseid = i as u16;
        m.nextinclass = 0xFFFF;
    }
    rows[15].nextinclass = 16;
    rows[16].baseid = 15;
    rows[50].baseid = 206;
    rows[51].baseid = 267;
    let ct = combat_tables(rows);
    let spawned = |f: &mut BodyFake| -> Vec<(i32, i32)> {
        f.take_log()
            .iter()
            .filter_map(|s| {
                let s = s.strip_prefix("At { ")?;
                let class = s.split("class: ").nth(1)?.split(',').next()?.parse().ok()?;
                let mode = s.split("mode: ").nth(1)?.split(',').next()?.parse().ok()?;
                Some((class, mode))
            })
            .collect()
    };
    let (mut f, _) = world();
    // 206 crownest1: chain(15 foulcrow1) at the unit's chain position 1.
    let u = f.add(FUnit::new(UnitType::Monster, 50), (0, 0));
    f.chains.insert(50, 1);
    let mut s = f.c.units[u].seed;
    let n = (s.step() % 3) as usize + 2;
    assert_eq!(b4_more::baal_tentacle(&mut f, &ct, u), 1);
    assert_eq!(spawned(&mut f), vec![(16, 1); n]);
    // 267 bloodraven: clamp(6), mode 8.
    let u = f.add(FUnit::new(UnitType::Monster, 51), (0, 0));
    let mut s = f.c.units[u].seed;
    let n = (s.step() % 3) as usize + 2;
    assert_eq!(b4_more::baal_tentacle(&mut f, &ct, u), 1);
    assert_eq!(spawned(&mut f), vec![(6, 8); n]);
}

#[test]
fn lightning_fan_with_a_step_of_zero_or_less_stops_and_reports() {
    // Edge case 1 (D2RUST decision): the base game never ends
    // the loop for a step ≤ 0; D2RUST creates the i = 0 missile the
    // base game makes first, no further one, and reports the fault.
    let t = tabs(body_rec(), Code::new(), 600);
    for (step, ring) in [(0, false), (-3, false), (0, true), (-1, true)] {
        let (mut f, u) = world();
        b4_helpers::zigzag(&mut f, &t, u, 5, (10, 10), 1, 1, step, 7, ring);
        assert_eq!(f.missiles.len(), 1, "step {step}");
        let (ox, oy) = ring_offset(0);
        assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (ox, oy));
        let log = f.take_log();
        let fault = format!(
            "{:?}",
            BodyEffect::<usize, usize, usize>::EndlessProgressive {
                unit: u,
                skill: 1,
                step
            }
        );
        assert_eq!(log.last(), Some(&fault), "step {step}");
    }
    // A positive step makes 64 / step missiles and no fault.
    let (mut f, u) = world();
    b4_helpers::zigzag(&mut f, &t, u, 5, (10, 10), 1, 1, 16, 7, false);
    assert_eq!(f.missiles.len(), 4);
    assert!(!f
        .take_log()
        .iter()
        .any(|l| l.starts_with("EndlessProgressive")));
}
