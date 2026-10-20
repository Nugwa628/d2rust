use super::act2::{act_row, give_skill, logged, param_of, point_mode, seeded, set_param_of, world};
use super::act3::seed_raw;
use super::act6::{grow, own, sentry_world};
use super::npc::{seed_with, steps_since, unit_mode};
use super::*;

fn death() -> String {
    point_mode(mode::DEATH, 0, 0)
}

// ---- Sarcophagus, FlyingScimitar, GargoyleTrap ---------------------

#[test]
fn sarcophagus_vectors() {
    let sarc = [125, 0, 16];
    // D = 10, n = 0, frame − param 0 = 125, footprint free: n := 1,
    // param 0 := frame, Nest at T.
    let mut w = world(act_row(45, &sarc));
    give_skill(&mut w, 1, 140, 14);
    w.fake.footprint = true;
    w.game.frame = 125;
    w.think_with(Some(w.player), 10, false);
    assert_eq!((param_of(&w, 1), param_of(&w, 0)), (1, 125));
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    // A blocked footprint: param 0 unchanged, idle `lo' % 10` + 20.
    let mut w = world(act_row(45, &sarc));
    give_skill(&mut w, 1, 140, 14);
    w.game.frame = 125;
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 0);
    let mut s = Seed::init_low(1);
    assert_eq!(w.thinks(), [125 + (s.step() % 10) as i32 + 20]);
    // n > aip3 (the cap test is >): death; n = aip3 is not.
    let mut w = world(act_row(45, &sarc));
    set_param_of(&mut w, 1, 17);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [death()]);
    // D > 25: idle 25.
    let mut w = world(act_row(45, &sarc));
    w.think_with(Some(w.player), 26, false);
    assert_eq!(w.thinks(), [25]);
}

#[test]
fn flying_scimitar_reads_aip1_twice() {
    let fs = [90, 90, 8, 40];
    // Not C: draw < aip1 → walk in radius (8, 1).
    let (mut w, _) = seeded(act_row(47, &fs), 1, |v| v[0] < 90);
    w.think_with(Some(w.player), 9, false);
    assert!(logged(&w, "radius 8 1"));
    // Not C: draw ≥ 90, second draw < aip1 (90, not aip2) → walk flags 7.
    let mut row = act_row(47, &fs);
    row.aip2 = 0;
    let (mut w, _) = seeded(row, 2, |v| v[0] >= 90 && v[1] < 90);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // C: draw ≥ aip2, draw < aip4 → circle 2.
    let (mut w, lo) = seeded(act_row(47, &fs), 2, |v| v[0] >= 90 && v[1] < 40);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, lo), 3);
}

// T = 0 is unreachable in a target-mode-1 body and asserted (no mode-1
// or mode-4 body runs without a target).
#[test]
#[should_panic(expected = "GargoyleTrap think without a target")]
fn gargoyle_trap_asserts_a_target() {
    let mut w = world(act_row(63, &[24, 20, 12, 15]));
    w.think_with(None, 0, false);
}

#[test]
fn gargoyle_trap_vectors() {
    let g = [24, 20, 12, 15];
    // s = 0, T 3 east 10 south, D 10, draw 10 (< 20): P := (ux, ty); fire;
    // s := 12.
    let (mut w, _) = seeded(act_row(63, &g), 1, |v| v[0] == 10);
    give_skill(&mut w, 1, 150, 14);
    w.fake.pos.insert(w.player, (103, 110));
    w.fake.y.dir = 16;
    w.think_with(Some(w.player), 10, false);
    assert!(logged(&w, &format!("pathtarget {}", w.player.0)));
    assert!(logged(&w, "snap 49"));
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    assert_eq!(param_of(&w, 0), 12);
    // s > 0: idle s, s := 0.
    w.think_with(Some(w.player), 10, false);
    assert_eq!((w.thinks(), param_of(&w, 0)), (vec![12], 0));
    // Off both axes by more than 5: idle aip4.
    let mut w = world(act_row(63, &g));
    w.fake.pos.insert(w.player, (110, 110));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [15]);
}

// ---- arrow traps, skill traps -----------------------------------------

#[test]
fn arrow_traps_by_axis_and_kind() {
    let arrow = [5, 15, 30, 7];
    // Act I level: kind 0 (no draw) → A1; f := frame + aip3.
    let mut w = world(act_row(78, &arrow));
    w.seed(1);
    w.fake.pos.insert(w.player, (101, 110));
    w.game.frame = 5;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert!(logged(&w, "trapkind 0"));
    assert_eq!((param_of(&w, 0), steps_since(&w, 1)), (35, 0));
    // A later level: kind := roll(3), drawn once and cached; kind 1 with
    // `Skill1`: param 2 := frame + aip4 (never read), `Skill1` at T.
    let lo = seed_raw(1, |v| v[0] % 3 == 1);
    let mut w = world(act_row(79, &arrow));
    give_skill(&mut w, 1, 160, 10);
    w.fake.level = 50;
    w.seed(lo);
    w.fake.pos.insert(w.player, (110, 101));
    w.game.frame = 5;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(param_of(&w, 2), 12);
    assert_eq!(w.fake.y.trap_kind, Some(1));
    // The cached kind: no further draw.
    w.game.frame = 40;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(steps_since(&w, lo), 1);
    // Off axis (78 compares x): idle 30; out of the window: idle 40.
    let mut w = world(act_row(78, &arrow));
    w.fake.pos.insert(w.player, (110, 101));
    w.game.frame = 5;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [35]);
    w.think_with(Some(w.player), 4, false);
    assert_eq!(w.thinks(), [45]);
}

#[test]
fn skill_traps_fire_then_die() {
    let pc = [20, 1, 15];
    let mut w = world(act_row(80, &pc));
    give_skill(&mut w, 1, 170, 10);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, 1));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes().last().unwrap(), &death());
    // Without `Skill1`: toggle := 0, idle aip3 (Trap-Nova, the same body).
    let mut w = world(act_row(92, &pc));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [15]);
}

// ---- JarJar, InvisoSpawner, BoneWall, ---------------------

#[test]
fn jar_jar_keeps_the_door() {
    // First think: the home point, idle 20.
    let mut w = world(act_row(81, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    // Door closed, away from home: walk back, w := frame.
    w.fake.pos.insert(w.mon, (110, 100));
    w.game.frame = 300;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 100, 100)]);
    assert_eq!(w.commands()[0][4], 300);
    // Within 200 frames of the last walk: idle 20.
    w.game.frame = 350;
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [370]);
    // Door open, far from (H.x, H.y − 3): walk there.
    let mut w = world(act_row(81, &[]));
    w.think_with(None, 0, false);
    w.fake.y.hooks.insert("PalaceDoorOpen".into());
    w.fake.pos.insert(w.mon, (100, 110));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 100, 97)]);
    // Door open, at the point, nobody near: idle 120.
    w.fake.pos.insert(w.mon, (100, 97));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [120]);
}

#[test]
fn inviso_spawner_rules() {
    let inv = [4, 5, 125];
    // f = 0: n := aip1; spawn class for level of mummy1 at a free spot.
    let mut w = world(act_row(82, &inv));
    grow(&mut w, 97);
    let c = w.add_unit(UnitType::Monster, (90, 90));
    w.fake.x.spawn = Some(c);
    w.fake.y.free_spot = Some((90, 90));
    w.think_with(Some(w.player), 3, false);
    assert!(logged(&w, "freespot2 0 1096"));
    assert!(logged(&w, "spawn 1096 90 90 1 2 0x42"));
    assert_eq!((param_of(&w, 1), param_of(&w, 0)), (3, 125));
    assert_eq!(w.thinks(), [15]);
    // D > aip2: idle 15; n < 1: death.
    let mut w = world(act_row(82, &inv));
    w.think_with(Some(w.player), 6, false);
    assert_eq!(w.thinks(), [15]);
    let mut w = world(act_row(82, &inv));
    set_param_of(&mut w, 0, 9);
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes(), [death()]);
}

#[test]
fn bone_wall_trap_melee_and_seven_tombs() {
    // BoneWall: frame ≤ param 0 → idle 15; later → death.
    let mut w = world(act_row(84, &[]));
    set_param_of(&mut w, 0, 10);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
    w.game.frame = 11;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [death()]);
    // Trap-Melee: C, draw 69 (< 70) → A1; not C → idle 40.
    let (mut w, _) = seeded(act_row(87, &[70, 15]), 1, |v| v[0] == 69);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    let mut w = world(act_row(87, &[70, 15]));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [40]);
    // 7TIllusion: mode 4 at (x − 10, y), param 1 := 1; then death.
    let mut w = world(act_row(88, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::ATTACK1, 90, 100)]);
    assert_eq!(param_of(&w, 1), 1);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes()[1], death());
    assert!(logged(&w, "flag 0x20000"));
}

// ---- DarkWanderer, ArcaneTower, Spirit --------------------------

#[test]
fn dark_wanderer_walks_then_leaves() {
    let mut w = world(act_row(91, &[]));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [10]);
    w.fake.y.wanderer = Some((130, 100));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 130, 100)]);
    assert_eq!(param_of(&w, 0), 2);
    // Not there yet: a retry with velocity method 7.
    w.think_with(Some(w.player), 5, false);
    assert_eq!(param_of(&w, 1), 1);
    assert_eq!(w.vel_request().method, 7);
    // Arrived: flag, death, the minion hook.
    w.fake.pos.insert(w.mon, (130, 100));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes().last().unwrap(), &death());
    assert!(logged(&w, "hook DarkWandererGone"));
}

#[test]
fn arcane_tower_vector() {
    let at = [1, 150, 33, 4, 150, 50];
    // p = 0, n = 1, frame ≥ f: `Skill1`; p := 1, n := 4, f := frame + 33.
    let mut w = world(act_row(93, &at));
    give_skill(&mut w, 1, 180, 10);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(
        (param_of(&w, 0), param_of(&w, 1), param_of(&w, 2)),
        (1, 4, 33)
    );
    // Then A1 bolts: n 4 → 3, f := frame + aip6.
    w.game.frame = 33;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes()[1], unit_mode(mode::ATTACK1, w.player));
    assert_eq!((param_of(&w, 1), param_of(&w, 2)), (3, 83));
    // Too early: idle 10.
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [43]);
}

#[test]
fn spirit_strikes_once() {
    let mut w = world(act_row(97, &[]));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [10]);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [50]);
}

// ---- BladeCreeper, InvisoPet, DeathSentry -----------------------

#[test]
fn blade_creeper_shuttles() {
    // The init: −1, 1, 0.
    let mut w = world(act_row(102, &[]));
    assert_eq!(w.store.control(w.mon).unwrap().params, [-1, 1, 0]);
    // No entry: death.
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [death()]);
    // With the entry: expiry := frame + calc4; the missile from the owner;
    // the leg param 1 = 1 walks to (params 3, 4) first.
    let mut w = sentry_world(102, &[]);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.srvmissilea = 0;
    w.skills = vec![sk.clone(), sk];
    w.missiles = vec![Missiles::decode(&vec![0u8; Missiles::SIZE])];
    w.fake.y.calc = 50;
    let m = w.add_unit(UnitType::Missile, (100, 100));
    w.fake.y.missile = Some(m);
    w.fake.stats.insert((w.player, 119), 7);
    w.store.control_mut(w.mon).unwrap().params = [-1, 1, 0];
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [0, 110, 100, 90, 100],
    }];
    w.game.frame = 5;
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 0), 55);
    assert!(logged(&w, "missile 1 5 0 100 100"));
    assert!(logged(&w, &format!("link {} {}", m.0, w.mon.0)));
    assert_eq!(w.fake.stats[&(w.mon, 119)], 7);
    assert_eq!(param_of(&w, 2), 1);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 90, 100)]);
    assert_eq!(w.vel_request().steps, 20);
    // The method is the wall follow (`mov edx, 0xF` before).
    assert_eq!(w.vel_request().method, 15);
    // No command: idle 3.
    w.store.control_mut(w.mon).unwrap().commands.clear();
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [8]);
}

#[test]
fn inviso_pet_teleports_near_its_owner() {
    let ip = [16, 15];
    // frame > f: x, y := O ± roll(32) − 16; Teleport 2 in mode 9.
    let mut w = world(act_row(103, &ip));
    own(&mut w);
    w.seed(1);
    w.game.frame = 1;
    w.think_with(None, 0, false);
    let mut s = Seed::init_low(1);
    let x = 105 + (s.step() % 32) as i32 - 16;
    let y = 100 + (s.step() % 32) as i32 - 16;
    assert_eq!(w.fake.modes(), [point_mode(mode::SKILL2, x, y)]);
    assert!(logged(&w, "skill 292"));
    assert_eq!(param_of(&w, 0), 16);
    // Not due, owner not running: idle aip2.
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [16]);
}

#[test]
fn death_sentry_explodes_corpses() {
    let ds = [30, 0, 50, 16];
    // Charges left but no entry: nothing scheduled.
    let mut w = world(act_row(104, &ds));
    own(&mut w);
    give_skill(&mut w, 1, 1, 9);
    w.skills = vec![Skills::decode(&vec![0u8; Skills::SIZE]); 2];
    set_param_of(&mut w, 1, 2);
    w.think_with(None, 0, false);
    assert!(w.thinks().is_empty() && w.fake.modes().is_empty());
    // A corpse within (Param3 + (L − 1) Param4) / 2 of S: a charge, g := its
    // GUID, `Skill1` in mode 9 at it.
    let mut w = sentry_world(104, &ds);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.param3 = 10;
    sk.param4 = 2;
    w.skills = vec![sk.clone(), sk];
    w.fake.y.calc = 3;
    let s = w.add_unit(UnitType::Monster, (110, 100));
    let k = w.add_unit(UnitType::Monster, (112, 100));
    w.fake.secondary = Some((s, 10));
    w.fake.x.corpse = Some(k);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::SKILL2, k)]);
    let g = w.game.lists.unit(k).unwrap().guid as i32;
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (g, 2));
    // The same corpse again: the lightning by E2 < aip4 and roll < aip3.
    w.seed(seed_with(1, |v| v[0] < 50));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes()[1], unit_mode(mode::SEQUENCE, s));
}

// ---- ShadowWarrior ------------------------------------------------------

#[test]
fn shadow_warrior_rules() {
    // No owner: idle 100.
    let mut w = world(act_row(105, &[40, 30, 60, 1]));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [100]);
    // The mana pool: m += −1 − aip4; outside 0..64·K2 (K2 = aip8(H), not
    // the difficulty's column) → 0.
    let mut row = act_row(105, &[40, 30, 60, 1]);
    row.aip8_h = 64;
    let mut w = world(row);
    own(&mut w);
    w.fake.y.final_point.insert(w.player, (105, 100));
    w.fake.y.target_point.insert(w.player, (105, 100));
    w.fake.y.last_placed = (300, 300);
    set_param_of(&mut w, 1, 100);
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 1), 98);
    set_param_of(&mut w, 1, 0);
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 1), 0);
    // Then the pet follow loiters (pet move k 2) and ends the think.
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn shadow_warrior_mimics_its_owner() {
    // Owner skills R (id 1) and L (id 2): both assigned at the mimic level;
    // roll(2) picks; a skill of another class is not usable, so the attack
    // (skill 0) is used.
    let mut row = act_row(105, &[40, 30, 60, 1]);
    row.aip8_n = 5;
    row.aip8_h = 64;
    let mut w = world(row);
    own(&mut w);
    w.fake.y.final_point.insert(w.player, (105, 100));
    w.fake.y.target_point.insert(w.player, (105, 100));
    w.fake.y.last_placed = (300, 300);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.charclass = 3;
    w.skills = vec![sk.clone(), sk.clone(), sk];
    w.fake.x.hand.insert((w.player, true), (1, 9));
    w.fake.x.hand.insert((w.player, false), (2, 6));
    w.fake.x.skill_level.insert(0, 1);
    w.fake.x.skill_level.insert(1, 4);
    w.fake.x.skill_level.insert(2, 4);
    set_param_of(&mut w, 2, 3);
    w.seed(1);
    w.think_with(Some(w.player), 5, true);
    assert!(logged(&w, "assign 1 5") && logged(&w, "assign 2 4"));
    assert_eq!(
        w.fake.modes().last().unwrap(),
        &unit_mode(mode::ATTACK1, w.player)
    );
}

#[test]
fn shadow_warrior_pet_test_reads_pettype_as_a_signed_index() {
    use crate::monsters::ai::bodies7::pet_test;
    // summon ≠ 0 and ≠ the unit's class; pettype valid (≥ 0 and below the
    // pettype row count 10) and O ≠ 0 → pass when the unit's pet type ≠
    // pettype; an invalid pettype (negative, or ≥ the count) → pass.
    // (pettype byte, the unit's pet type, pass)
    for (pettype, pet_type, pass) in [
        (3u8, 3, false),
        (3, 2, true),
        (12, 12, true),
        (0xFF, -1, true),
        (0x80, -128, true),
    ] {
        let mut w = world(act_row(105, &[40, 30, 60, 1]));
        let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
        sk.summon = 999;
        sk.pettype = pettype;
        w.skills = vec![sk];
        w.fake.y.pettype_count = 10;
        w.fake.y.pet_type = pet_type;
        let (u, o) = (w.mon, w.player);
        let r = w.with(|g, cx| pet_test(g, cx, Some(o), u, 0));
        assert_eq!(r, pass, "pettype {pettype:#x}, pet type {pet_type}");
    }
}

// ---- Raven, Vines, DruidBear -------------------------------------

#[test]
fn raven_init_and_hits() {
    // The init: c := −1, f := aip3 + 1, σ := one step's bit 0.
    let mut w = world(act_row(107, &[10, 6, 5, 75, 35]));
    let mon = w.mon;
    w.seed(12345);
    w.with(|g, cx| run_init(g, cx, 0x005E_CB70, mon));
    let mut s = Seed::init_low(12345);
    let sigma = (s.step() & 1) as i32;
    assert_eq!(w.store.control(mon).unwrap().params, [-1, 6, sigma]);
    // No owner: idle 10.
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [10]);
    // c = 0: killed by itself.
    let mut w = world(act_row(107, &[10, 6, 5, 75, 35]));
    own(&mut w);
    set_param_of(&mut w, 0, 0);
    w.think_with(Some(w.player), 5, false);
    assert!(logged(&w, &format!("kill {} Some({})", w.mon.0, w.mon.0)));
    // c = −1 without `Skill1`: c := 3; then the hits (C, draws pass): A1,
    // c := 2, f := frame + aip3 × 10.
    let lo = seed_raw(1, |v| v[0] % 100 < 75);
    let mut w = world(act_row(107, &[10, 6, 5, 75, 35]));
    own(&mut w);
    set_param_of(&mut w, 0, -1);
    set_param_of(&mut w, 1, 0);
    w.seed(lo);
    w.game.frame = 10;
    w.think_with(Some(w.player), 4, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (2, 60));
}

#[test]
fn vines_rules() {
    let pp = [100, 20, 25, 10, 35];
    let mut w = world(act_row(110, &pp));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [25]);
    // A poisoned S: escape by aip4 (no delete).
    let mut w = world(act_row(110, &pp));
    own(&mut w);
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 5));
    w.fake.states.insert((s, 2));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 90, 100)]);
}

#[test]
fn druid_bear_rules() {
    let db = [15, 40, 50];
    let mut w = world(act_row(112, &db));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [10]);
    // dO > 50: pet move k 3.
    let mut w = world(act_row(112, &db));
    own(&mut w);
    w.fake.pos.insert(w.player, (161, 100));
    w.think_with(None, 0, false);
    assert!(logged(&w, "freespot2 0 -1"));
    // dO < 17, no target: idle 15.
    let mut w = world(act_row(112, &db));
    own(&mut w);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
}

// ---- SiegeTower, GenericSpawner, Wussie --------------------------

#[test]
fn siege_tower_vectors() {
    let mut w = world(act_row(113, &[40]));
    w.fake.x.owners.insert(w.mon, w.player);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [40]);
    // No owner, a free imp1 rider within 400: its mount := this tower.
    let mut w = world(act_row(113, &[40]));
    grow(&mut w, 493);
    w.monstats[492].baseid = 492;
    let imp = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.class.insert(imp, 492);
    w.store.entry(imp).control = Some(AiControl {
        params: [-1, 0, 0],
        ..AiControl::default()
    });
    w.think_with(Some(w.player), 5, false);
    let g = w.game.lists.unit(w.mon).unwrap().guid as i32;
    assert_eq!(w.store.control(imp).unwrap().params[0], g);
    assert_eq!(w.thinks(), [40]);
}

#[test]
fn generic_spawner_rules() {
    let hut = [80, 0, 15];
    // The init: FoulCrowNest's, spawn class −1.
    let mut w = world(act_row(129, &hut));
    assert_eq!(w.store.control(w.mon).unwrap().spawn_class, -1);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    // The pick: entry roll(n) with `genericSpawn`, else the first flagged.
    grow(&mut w, 500);
    w.monstats[7].genericspawn = true;
    w.fake.y.regions = vec![3, 7, 9];
    w.seed(seed_with(1, |v| v[0] % 3 != 1));
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.store.control(w.mon).unwrap().spawn_class, 7);
    // Due, footprint free: param 1 += 1, Nest (167) as a sequence skill.
    let mut w = world(act_row(129, &hut));
    w.fake.footprint = true;
    w.game.frame = 80;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::SEQUENCE, w.player)]);
    assert!(logged(&w, "skill 167"));
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (80, 1));
    // At the cap: death.
    set_param_of(&mut w, 1, 15);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes().last().unwrap(), &death());
}

#[test]
fn wussie_rules() {
    // A portal hook with P = 0: idle 25.
    let mut w = world(act_row(131, &[]));
    w.fake.y.portal = Some(None);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [25]);
    // P within 4: leaves (removed).
    let mut w = world(act_row(131, &[]));
    let p = w.add_unit(UnitType::Object, (102, 100));
    w.fake.y.portal = Some(Some(p));
    w.think_with(None, 0, false);
    assert!(logged(&w, "hook WussieLeave") && logged(&w, &format!("remove {}", w.mon.0)));
    // A player who can rescue: rescue, slot 8, idle 25.
    let mut w = world(act_row(131, &[]));
    w.fake.nearest = Some((w.player, false));
    w.fake.y.hooks.insert("WussieCanRescue".into());
    w.think_with(None, 0, false);
    assert!(logged(&w, "hook WussieRescue") && logged(&w, "node 8"));
    assert_eq!(w.thinks(), [25]);
}

// ---- UberIzual, the Ubers, ShadowMaster --------------------------

#[test]
fn uber_izual_armor_and_teleport() {
    let ui = [45, 50, 66, 0, 20, 3];
    // `Skill2` with `aurastate` 5 missing on the unit: the armor.
    let mut w = world(act_row(144, &ui));
    give_skill(&mut w, 2, 1, 10);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.aurastate = 5;
    w.skills = vec![sk.clone(), sk];
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [point_mode(10, 0, 0)]);
    // A blocked line (mask 6): `Skill3` teleport to T's position.
    let mut w = world(act_row(144, &ui));
    give_skill(&mut w, 3, 7, 11);
    w.fake.y.blocked6 = true;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [point_mode(11, 105, 100)]);
    // Else Izual's steps: s pending → idle s, no quest call.
    let mut w = world(act_row(144, &ui));
    set_param_of(&mut w, 1, 7);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [7]);
    assert!(!logged(&w, "quest IzualActivated"));
}

#[test]
fn uber_thinks_are_empty() {
    for ai in [145u16, 146, 147] {
        let mut w = world(act_row(ai, &[]));
        w.seed(1);
        w.think_with(Some(w.player), 1, true);
        assert!(w.thinks().is_empty() && w.fake.log.is_empty(), "AI {ai}");
        assert!(w.store.unhandled.is_empty());
        assert_eq!(steps_since(&w, 1), 0);
    }
}

#[test]
fn shadow_master_basics() {
    // No skill list: idle 100.
    let mut w = world(act_row(106, &[]));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [100]);
    // A skill list, no target, no owner: the buff pass finds nothing → idle 25.
    let mut w = world(act_row(106, &[]));
    w.fake.y.skill_list = true;
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [25]);
    // ShadowMasterNoInit shares the think.
    assert_eq!(AI_TABLE[143].think, AI_TABLE[106].think);
}

#[test]
fn shadow_master_self_target_candidates_never_fire() {
    // One `aitype` 8 skill at L ≤ 66 (target: the unit itself): it is a
    // candidate (score > 0) but the use helper refuses it; the attack at T
    // (slot 0) or step 14's attack follows.
    let mut row = act_row(106, &[0, 10]);
    row.aip2_n = 36;
    let mut w = world(row);
    w.fake.y.skill_list = true;
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.aitype = 8;
    sk.aibonus = 20;
    w.skills = vec![Skills::decode(&vec![0u8; Skills::SIZE]), sk];
    w.fake.y.unit_skills = vec![(1, 5)];
    w.fake.x.skill_level.insert(0, 1);
    w.fake.x.skill_level.insert(1, 5);
    w.fake.life = 50;
    w.fake.x.flags.insert(w.player, 0x4);
    w.seed(1);
    w.think_with(Some(w.player), 5, false);
    let modes = w.fake.modes();
    assert!(!modes.iter().any(|m| m.contains(&format!("{:?}", w.mon))));
    assert_eq!(modes.last().unwrap(), &unit_mode(mode::ATTACK1, w.player));
}

#[test]
fn shadow_master_scan_ignores_units_beyond_1024() {
    // The base game: near, best and notable sit inside the n test.
    for (dx, dy, n) in [(44, 10, 0), (30, 10, 1)] {
        let mut w = world(act_row(106, &[]));
        let (mon, p) = (w.mon, w.player);
        w.fake.pos.insert(p, (100 + dx, 100 + dy));
        w.fake.x.flags.insert(p, 0x4);
        w.fake.x.hostile.insert(p);
        let r = w.with(|g, cx| crate::monsters::ai::bodies7::master_scan(g, cx, mon, None));
        assert_eq!((r.n, r.near), (n, 0), "d² {}", dx * dx + dy * dy);
        assert_eq!(r.best, (n == 1).then_some(p));
    }
}

#[test]
fn shadow_master_init_gives_the_attack() {
    let mut w = world(act_row(106, &[]));
    own(&mut w);
    let mon = w.mon;
    w.fake.y.class_skills = vec![4];
    w.skills = vec![Skills::decode(&vec![0u8; Skills::SIZE]); 5];
    w.with(|g, cx| run_init(g, cx, 0x005E_B490, mon));
    assert!(logged(&w, "assign 0 1") && logged(&w, "hand 0 false") && logged(&w, "hand 0 true"));
    // The class skill at clamp(v / 2 + λ / 2, 1, 24) with v = 1, λ = 1.
    assert!(logged(&w, "assign 4 1"));
    // 143: no class skills.
    let mut w = world(act_row(143, &[]));
    own(&mut w);
    let mon = w.mon;
    w.fake.y.class_skills = vec![4];
    w.with(|g, cx| run_init(g, cx, 0x005E_B5C0, mon));
    assert!(logged(&w, "assign 0 1") && !logged(&w, "assign 4 1"));
    let _ = grow;
}
