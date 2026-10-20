use super::act2::{act_row, give_skill, logged, param_of, point_mode, seeded, set_param_of, world};
use super::act3::seed_raw;
use super::npc::{steps_since, unit_mode};
use super::*;

// ---- VileDog, DoomKnight -----------------------------------------

#[test]
fn vile_dog_vectors() {
    let dog = [80, 10, 80];
    // Param 0 = 1, C: 85 (≥ 80) → idle 10.
    let (mut w, lo) = seeded(act_row(69, &dog), 1, |v| v[0] >= 80);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [10]);
    assert_eq!(steps_since(&w, lo), 1);
    // Param 0 = 0: woken, idle 5, no draw.
    let mut w = world(act_row(69, &dog));
    w.seed(1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!((param_of(&w, 0), w.thinks()), (1, vec![5]));
    assert_eq!(steps_since(&w, 1), 0);
    // Not C: P(aip3) → walk flags 7.
    let (mut w, _) = seeded(act_row(69, &dog), 1, |v| v[0] < 80);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
}

#[test]
fn doom_knight_vectors() {
    let dk = [90, 10, 80, 10];
    // C = 0: 50 (< 80) → lunge (velocity 13, walk flags 7).
    let (mut w, _) = seeded(act_row(72, &dk), 1, |v| v[0] < 80);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.vel_request().method, 13);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // C: P(aip1) fails → idle aip2.
    let (mut w, _) = seeded(act_row(72, &dk), 1, |v| v[0] >= 90);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [10]);
}

// ---- Megademon ------------------------------------------------------

#[test]
fn megademon_vectors() {
    let mega = [50, 40, 80, 70, 50, 75];
    // C, frame ≤ param 0: 60 (< 80) → A1 (no inferno draw).
    let (mut w, lo) = seeded(act_row(89, &mega), 1, |v| v[0] < 80);
    give_skill(&mut w, 1, 160, 4);
    set_param_of(&mut w, 0, 5);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(steps_since(&w, lo), 1);
    // Not C, D < R, not burning, draw < aip1: cooldown frame + aip6.
    let (mut w, _) = seeded(act_row(89, &mega), 1, |v| v[0] < 50);
    give_skill(&mut w, 1, 160, 4);
    w.fake.x.skill_level.insert(160, 5);
    w.game.frame = 10;
    w.think_with(Some(w.player), 3, false);
    assert_eq!(param_of(&w, 0), 85);
    assert_eq!(w.fake.modes(), [unit_mode(4, w.player)]);
    // Burning: state 12 off, then step 3.
    let (mut w, _) = seeded(act_row(89, &mega), 1, |v| v[0] >= 70);
    give_skill(&mut w, 1, 160, 4);
    w.fake.x.skill_level.insert(160, 5);
    w.fake.states.insert((w.mon, 12));
    w.think_with(Some(w.player), 3, false);
    assert!(!w.fake.states.contains(&(w.mon, 12)));
    assert_eq!(w.thinks(), [10]);
}

// ---- Izual ----------------------------------------------------------

#[test]
fn izual_vectors() {
    let izual = [45, 50, 66, 0, 20, 3];
    // Params 0 = 1, 1 = 0, 2 = 2, C: no draws, w := 1, A1.
    let mut w = world(act_row(55, &izual));
    w.seed(1);
    w.store.control_mut(w.mon).unwrap().params = [1, 0, 2];
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 2), 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
    // First think: the quest seam; s pending → idle s, s := 0.
    let mut w = world(act_row(55, &izual));
    set_param_of(&mut w, 1, 20);
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, "quest IzualActivated"));
    assert_eq!((param_of(&w, 1), w.thinks()), (0, vec![20]));
    // Not C, D < 10, draw < aip3: the nova, s := aip5, w := aip6.
    let (mut w, _) = seeded(act_row(55, &izual), 1, |v| v[0] < 66);
    give_skill(&mut w, 1, 170, 10);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!((param_of(&w, 1), param_of(&w, 2)), (20, 3));
    // C, w = 0, P(aip1) fails, no nova (aip4 = 0): idle `aidel`.
    let (mut w, _) = seeded(act_row(55, &izual), 2, |v| v[0] >= 45);
    give_skill(&mut w, 1, 170, 10);
    set_param_of(&mut w, 0, 1);
    w.monstats[0].aidel = 12;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [12]);
    // Not C, far, draw ≥ aip2: walk in radius (6, 9).
    let (mut w, _) = seeded(act_row(55, &izual), 1, |v| v[0] >= 50);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 20, false);
    assert!(logged(&w, "radius 6 9"));
}

// ---- AbyssKnight, OblivionKnight ---------------------------------

#[test]
fn abyss_knight_vectors() {
    let dk2 = [40, 80, 90, 10, 6, 1, 70, 40];
    // No armor cast, C = 0, D = 10, c = 1: 50 (< 70), next `lo'` odd → c :=
    // 0; velocity (2, 0, 7), walk to T flags 7.
    let lo = seed_raw(2, |v| v[0] % 100 < 70 && v[1] & 1 == 1);
    let mut w = world(act_row(73, &dk2));
    w.seed(lo);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 0);
    let v = w.vel_request();
    assert_eq!((v.method, v.speed, v.steps), (2, 0, 7));
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // D < aip5, c ≤ 0 → c := aip6 = 1; no shot (c ≠ 0); c := 0.
    let (mut w, _) = seeded(act_row(73, &dk2), 1, |v| v[0] >= 70);
    give_skill(&mut w, 1, 180, 8);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // c = 0, D ≥ aip5: the shot at T, c := aip6.
    let mut w = world(act_row(73, &dk2));
    give_skill(&mut w, 1, 180, 8);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(8, w.player)]);
    assert_eq!(param_of(&w, 0), 1);
    // Component byte ≥ 4: no shot.
    let (mut w, _) = seeded(act_row(73, &dk2), 1, |v| v[0] >= 70);
    give_skill(&mut w, 1, 180, 8);
    w.fake.x.component = 4;
    w.think_with(Some(w.player), 50, false);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn abyss_knight_bone_armor() {
    let dk2 = [40, 80, 90, 10, 6, 1, 70, 40];
    let (mut w, _) = seeded(act_row(73, &dk2), 1, |v| v[0] < 80);
    give_skill(&mut w, 2, 68, 10);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.aurastate = 14;
    w.skills = vec![sk.clone(); 69];
    w.fake.life = 30;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [point_mode(10, 0, 0)]);
    // Already armored: no draw.
    let mut w = world(act_row(73, &dk2));
    w.seed(1);
    give_skill(&mut w, 2, 68, 10);
    w.skills = vec![sk; 69];
    w.fake.life = 30;
    w.fake.states.insert((w.mon, 14));
    w.think_with(Some(w.player), 1, true);
    assert_eq!(steps_since(&w, 1), 1);
}

#[test]
fn oblivion_knight_rules() {
    let dk3 = [6, 25, 500, 50, 80, 30, 30, 9];
    // D < aip1: Decrepify at T (Skill4 > 0, target state > 0), cooldown.
    let mut w = world(act_row(74, &dk3));
    give_skill(&mut w, 4, 87, 10);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.auratargetstate = 60;
    w.skills = vec![sk; 88];
    w.game.frame = 10;
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(param_of(&w, 0), 510);
    // A knight farther than D: walk to it with flags 4.
    let mut w = world(act_row(74, &dk3));
    w.monstats[0].baseid = 310;
    let k = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.x.flags.insert(k, 0x2);
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, k)]);
    // Else escape by 10 with delete (velocity 2, 50).
    let mut w = world(act_row(74, &dk3));
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.vel_request().method, 2);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 90, 100)]);
    // S within aip2: the curse by draw < aip4.
    let (mut w, _) = seeded(act_row(74, &dk3), 1, |v| v[0] < 50);
    give_skill(&mut w, 6, 90, 10);
    w.fake.secondary = Some((w.player, 10));
    w.game.frame = 10;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 510);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    // Nothing: draw ≥ 70 → idle 10.
    let (mut w, _) = seeded(act_row(74, &dk3), 1, |v| v[0] >= 70);
    w.think_with(Some(w.player), 8, false);
    assert_eq!(w.thinks(), [10]);
}

// ---- FingerMage, Regurgitator -------------------------------------

#[test]
fn finger_mage_rules() {
    let fm = [40, 40, 50, 30, 15, 5, 40, 12];
    // No minion owner, L < aip4: f := 1, escape by 9 without delete.
    let mut w = world(act_row(70, &fm));
    w.fake.life = 20;
    w.think_with(Some(w.player), 20, false);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, 0));
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 91, 100)]);
    // f = 1, L ≤ aip3, draw ≥ 25, n ≤ aip6, D < 14: velocity, escape by 14.
    let (mut w, _) = seeded(act_row(70, &fm), 1, |v| v[0] >= 25);
    set_param_of(&mut w, 0, 1);
    w.fake.life = 20;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 1), 1);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 86, 100)]);
    // f = 1, L > aip3: f := 0, idle 15, no draw.
    let mut w = world(act_row(70, &fm));
    w.seed(1);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!((param_of(&w, 0), w.thinks()), (0, vec![15]));
    assert_eq!(steps_since(&w, 1), 0);
    // Not C, D < aip5, T cursed (state 84): walk to T flags 0.
    let mut w = world(act_row(70, &fm));
    w.fake.states.insert((w.player, 84));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // AI state 3, C: f := 0, A1.
    let mut w = world(act_row(70, &fm));
    set_param_of(&mut w, 0, 1);
    w.fake.ai_state = 3;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
}

#[test]
fn regurgitator_eats_corpses() {
    let reg = [70, 20, 40, 10, 3, 25];
    let corpse_world = || {
        let mut w = world(act_row(71, &reg));
        w.monstats2[0].soft = true;
        let c = w.add_unit(UnitType::Monster, (104, 100));
        w.fake.anim.insert(c, mode::DEAD);
        w.fake.x.flags.insert(c, 0x2);
        (w, c)
    };
    // s = 1, a corpse at squared distance 16 > 2: g := its GUID, walk 1
    // step, s := 2.
    let (mut w, c) = corpse_world();
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 10, false);
    let g = w.game.lists.unit(c).unwrap().guid as i32;
    assert_eq!(w.store.control(w.mon).unwrap().params, [2, g, 0]);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, c)]);
    // s = 2, far: walk 1 step, k += 1, then s := 3 and idle 8 (both).
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params, [3, g, 1]);
    assert_eq!(w.thinks(), [8]);
    // s = 3: eat it, s := 4.
    give_skill(&mut w, 1, 190, 8);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes().last().unwrap(), &unit_mode(8, c));
    assert_eq!(param_of(&w, 0), 4);
    // s = 4, not C: A2, s := 5; s = 5: escape by 16, s := 0.
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 5);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 0);
    // s = 1, no corpse, draw < 20: reset with nothing scheduled.
    let (mut w, _) = seeded(act_row(71, &reg), 1, |v| v[0] < 20);
    w.store.control_mut(w.mon).unwrap().params = [1, 7, 3];
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params, [0; 3]);
    assert!(w.thinks().is_empty() && w.fake.modes().is_empty());
    // s = 2 with the corpse gone: reset, wander 8.
    let mut w = world(act_row(71, &reg));
    w.store.control_mut(w.mon).unwrap().params = [2, 999, 0];
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params, [0; 3]);
    // s = 0, C, no corpse: draw < aip1 → A1.
    let (mut w, _) = seeded(act_row(71, &reg), 1, |v| v[0] < 70);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
}

// ---- VileMother -----------------------------------------------------

#[test]
fn vile_mother_births() {
    let vm = [16, 5, 30, 80, 70, 30, 10];
    // P(aip3), no children: the birth at the first free of the 8 offsets
    // (direction 0 → k 0 → (3, 0)); b += 1.
    let (mut w, _) = seeded(act_row(68, &vm), 1, |v| v[0] < 30);
    give_skill(&mut w, 1, 200, 14);
    w.fake.footprint = true;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [point_mode(14, 103, 100)]);
    assert_eq!(param_of(&w, 0), 1);
    // Direction 20 → 8-direction 3 → k 3 → (−3, 0).
    let (mut w, _) = seeded(act_row(68, &vm), 1, |v| v[0] < 30);
    give_skill(&mut w, 1, 200, 14);
    w.fake.footprint = true;
    w.fake.x.direction = 20;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [point_mode(14, 97, 100)]);
    // All 8 footprints fail: no birth, the think goes on to 3–5:
    // not C, draw < aip5 → walk to T after 2 draws
    // (gen-su-34 frame 73).
    let (mut w, lo) = seeded(act_row(68, &vm), 2, |v| v[0] < 30 && v[1] < 70);
    give_skill(&mut w, 1, 200, 14);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, lo), 2);
    // Count ≥ aip2 (aip2 0) also goes on: draw ≥ aip5, draw ≥ aip6 →
    // idle 15, 3 draws, no birth.
    let vm0 = [16, 0, 30, 80, 70, 30, 10];
    let (mut w, lo) = seeded(act_row(68, &vm0), 3, |v| {
        v[0] < 30 && v[1] >= 70 && v[2] >= 30
    });
    give_skill(&mut w, 1, 200, 14);
    w.fake.footprint = true;
    w.think_with(Some(w.player), 10, false);
    assert!(w.fake.modes().is_empty());
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(steps_since(&w, lo), 3);
    // At the cap aip1: C → P(aip4) → A1.
    let (mut w, _) = seeded(act_row(68, &vm), 1, |v| v[0] < 80);
    set_param_of(&mut w, 0, 16);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // At the cap, not C: walk to T flags 7, no draw.
    let mut w = world(act_row(68, &vm));
    w.seed(1);
    set_param_of(&mut w, 0, 16);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
}

// ---- Diablo ---------------------------------------------------------

/// A Diablo world: the player is the target-node head of slot 0.
fn diablo_world() -> World {
    let mut w = world(act_row(51, &[]));
    w.fake.nodes = vec![vec![w.player]];
    w.monstats[0].threat = 5;
    w
}

#[test]
fn diablo_choice_vectors() {
    // X in melee, clear, life ≥ 20 %, no cold, f39 = f41, not near: Σ = 229,
    // r = `lo' % 229` = 100 → cumulative 40, 110 → k = 3 (A2).
    let lo = seed_raw(1, |v| v[0] % 229 == 100);
    let mut w = diablo_world();
    w.seed(lo);
    w.fake.melee.insert(w.player);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    assert_eq!(param_of(&w, 0), 0);
    // The boundaries: r = 109 is still k = 3, r = 110 is k = 5 (`Skill1`
    // 40 + 70 = 110; with no `Skill3`, idle 2).
    for (r, want) in [
        (109, unit_mode(mode::ATTACK2, w.player)),
        (110, String::new()),
    ] {
        let lo = seed_raw(1, |v| v[0] % 229 == r);
        let mut w2 = diablo_world();
        w2.seed(lo);
        w2.fake.melee.insert(w2.player);
        w2.think_with(None, 0, false);
        let got = w2.fake.modes().first().cloned().unwrap_or_default();
        assert_eq!(got, want, "r {r}");
        if r == 110 {
            assert_eq!(w2.thinks(), [2]);
        }
    }
    // The home command was made at the own position.
    let c = &w.store.control(w.mon).unwrap().commands;
    assert_eq!(c[0].params, [10, 100, 100, 0, 0]);
    // X = 0, no collision: `lo' % 1000` = 0 → k = 4 (S4 at X = 0).
    let lo = seed_raw(1, |v| v[0] % 1000 == 0);
    let mut w = world(act_row(51, &[]));
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(11, 0, 0)]);
    // X = 0 on a 0x40 cell: k = 16, velocity 20, wander 5; no choice draw.
    let mut w = world(act_row(51, &[]));
    w.fake.x.pattern_collides = true;
    w.think_with(None, 0, false);
    assert_eq!(w.vel_request().speed, 20);
    // A pending choice 5 is returned without rescoring: `Skill1` at X,
    // param 0 stays 5.
    let mut w = diablo_world();
    w.seed(1);
    give_skill(&mut w, 1, 210, 10);
    set_param_of(&mut w, 0, 5);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(param_of(&w, 0), 5);
    assert_eq!(steps_since(&w, 1), 0);
    // Choice 5 with state 12 set: state off, idle 2, param 0 := 0.
    w.fake.states.insert((w.mon, 12));
    w.think_with(None, 0, false);
    assert!(!w.fake.states.contains(&(w.mon, 12)));
    assert_eq!((param_of(&w, 0), w.thinks()), (0, vec![2]));
    // Choice 11 (default): idle 12 / 8 / 4 by difficulty.
    let mut w = diablo_world();
    set_param_of(&mut w, 0, 11);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [12]);
}

#[test]
fn boss_score_formula() {
    let mut w = diablo_world();
    let (mon, p) = (w.mon, w.player);
    // Not in melee, clear line: G = 75; no stats: (5 × 75) / 22 = 17.
    let s = w.with(|g, cx| bodies4::score(g, cx, mon, p, bodies4::Score::Diablo(None)));
    assert_eq!(s, 17);
    // Melee (G = 100), low life (+300), cold (+400), coldresist 30 read:
    // (300 + 30 / 15 + 500 + 2 × 200) / 22 = 54.
    w.fake.melee.insert(p);
    w.fake.life_of.insert(p, 19);
    w.fake.states.insert((p, 11));
    w.fake.stats.insert((p, 43), 30);
    let s = w.with(|g, cx| bodies4::score(g, cx, mon, p, bodies4::Score::Diablo(None)));
    assert_eq!(s, (300 + 2 + 500 + 400) / 22);
    // Out of melee is 0 for Diablo but read for Baal.
    w.fake.melee.clear();
    w.fake.line_blocked.insert(p);
    let d = w.with(|g, cx| bodies4::score(g, cx, mon, p, bodies4::Score::Diablo(None)));
    let b = w.with(|g, cx| bodies4::score(g, cx, mon, p, bodies4::Score::Baal));
    assert_eq!((d, b), ((300 + 400) / 22, (300 + 2 + 400) / 22));
    // A monster with threat < 2 scores 0; a score of 0 becomes 1.
    let m = w.add_unit(UnitType::Monster, (110, 100));
    w.monstats[0].threat = 1;
    let s = w.with(|g, cx| bodies4::score(g, cx, mon, m, bodies4::Score::Baal));
    assert_eq!(s, 0);
    w.monstats[0].threat = 2;
    w.fake.line_blocked.insert(m);
    let s = w.with(|g, cx| bodies4::score(g, cx, mon, m, bodies4::Score::Baal));
    assert_eq!(s, 1);
}

#[test]
fn diablo_alternate_keeps_home() {
    let mut w = diablo_world();
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().commands = vec![AiCommand {
        params: [10, 70, 80, 0, 0],
    }];
    w.store.control_mut(mon).unwrap().function = 0x005E_8480;
    w.fake.states.insert((mon, 12));
    w.think_with(None, 0, false);
    let c = w.store.control(mon).unwrap();
    assert_eq!(c.function, 0x005E_9170);
    assert_eq!(c.commands.len(), 1);
    assert_eq!(c.commands[0].params, [10, 70, 80, 0, 0]);
    assert!(!w.fake.states.contains(&(mon, 12)));
    assert_eq!(w.thinks(), [1]);
}

#[test]
fn boss_pick_slots() {
    // Slot 8: a node in the boss's act is scored and counted; one in
    // another act is not.
    let mut w = diablo_world();
    let (mon, p) = (w.mon, w.player);
    let a = w.add_unit(UnitType::Monster, (110, 100));
    let b = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.acts.insert(b, 1);
    w.fake.nodes = vec![
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![a, b],
    ];
    let (best, max, n) = w.with(|g, cx| {
        bodies4::boss_pick(g, cx, mon, bodies4::Cull::Act, bodies4::Score::Diablo(None))
    });
    assert_eq!((best, max, n), (Some(a), 17, 1));
    // Slot 9: the alternative within 5 replaces the best when no path
    // exists and the boss is not in melee range of it.
    let mut w = diablo_world();
    let c = w.add_unit(UnitType::Monster, (103, 100));
    let mut nodes = vec![vec![p]];
    nodes.resize(9, vec![]);
    nodes.push(vec![c]);
    w.fake.nodes = nodes.clone();
    w.fake.life_of.insert(c, 10); // low life: a higher score than p
    let (best, _, n) = w.with(|g, cx| {
        bodies4::boss_pick(g, cx, mon, bodies4::Cull::Act, bodies4::Score::Diablo(None))
    });
    assert_eq!((best, n), (Some(c), 1));
    assert!(logged(&w, &format!("pathcompute {}", p.0)));
    // A path exists: the best stays.
    let mut w = diablo_world();
    let c = w.add_unit(UnitType::Monster, (103, 100));
    let mut nodes = vec![vec![w.player]];
    nodes.resize(9, vec![]);
    nodes.push(vec![c]);
    w.fake.nodes = nodes;
    w.fake.x.path_points = true;
    let (best, _, _) = w.with(|g, cx| {
        bodies4::boss_pick(g, cx, mon, bodies4::Cull::Act, bodies4::Score::Diablo(None))
    });
    assert_eq!(best, Some(w.player));
    // The attack rank: `attackrank` × level of the left and right skills.
    let mut w = diablo_world();
    let p = w.player;
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.attackrank = 8;
    w.skills = vec![sk; 3];
    w.fake.x.hand.insert((p, false), (1, 10));
    w.fake.x.hand.insert((p, true), (2, 1));
    // K = 88: (5 × 75 + 2 × (88 / 4)) / 22 = 19.
    let s = w.with(|g, cx| bodies4::score(g, cx, mon, p, bodies4::Score::Diablo(None)));
    assert_eq!(s, (375 + 44) / 22);
}
