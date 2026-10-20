use super::npc::{seed_with, steps_since, unit_mode};
use super::*;

/// A monstats row with AI `ai` and Normal aip1..aipN.
fn row(ai: u16, aips: &[i16]) -> Monstats {
    let mut r = monstats(ai, [0; 5], 15);
    let mut v = [0i16; 8];
    v[..aips.len()].copy_from_slice(aips);
    (r.aip1, r.aip2, r.aip3, r.aip4) = (v[0] as u16, v[1] as u16, v[2] as u16, v[3] as u16);
    (r.aip5, r.aip6, r.aip7, r.aip8) = (v[4] as u16, v[5] as u16, v[6] as u16, v[7] as u16);
    r
}

/// Runs one think per seed of `SEEDS` on a fresh world from `setup` and
/// returns each world.
fn per_seed(setup: impl Fn() -> World, run: impl Fn(&mut World)) -> Vec<World> {
    SEEDS
        .iter()
        .map(|&s| {
            let mut w = setup();
            w.seed(s);
            run(&mut w);
            w
        })
        .collect()
}

fn other(w: &mut World, at: (i32, i32)) -> UnitId {
    w.add_unit(UnitType::Monster, at)
}

// ---- CorruptRogue ----------------------------------------------

#[test]
fn corrupt_rogue_vectors() {
    let setup = || World::new(row(10, &[60, 15, 75, 100, 20]));
    // C, D = 10: 51 < 75 → A1 | 87 → idle 15 | 53 → A1 | 0 → A1.
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, true));
    for (i, w) in ws.iter().enumerate() {
        if i == 1 {
            assert_eq!(w.thinks(), [15]);
        } else {
            assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
        }
    }
    // Not C, D = 10: 51 < 60; 31 ≥ 20 → walk flags 7 | 87 → idle 15 |
    // 53; 46 → walk | 0; 42 → walk.
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        if i == 1 {
            assert_eq!(w.thinks(), [15]);
        } else {
            assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
            assert_eq!(steps_since(w, SEEDS[i]), 2);
        }
    }
    // D > L (20 in Normal, 14 in Hell): run, velocity (13, aip4, 0), 3
    // steps, no draw.
    let mut w = setup();
    w.seed(1);
    w.think_with(Some(w.player), 21, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::RUN, w.player)]);
    assert!(w.fake.log.contains(&"steps 3".to_string()));
    assert_eq!((w.vel_request().method, w.vel_request().speed), (13, 100));
    assert_eq!(steps_since(&w, 1), 0);
    let mut w = setup();
    w.seed(1);
    let mon = w.mon;
    let player = w.player;
    w.with(|g, cx| {
        cx.info.difficulty = 2;
        let p = TickParam {
            target: Some(player),
            distance: 15,
            combat: false,
            class: 0,
            class2: 0,
        };
        let f = cx.store.control(mon).unwrap().function;
        run_function(g, cx, f, mon, &p);
    });
    assert_eq!(w.fake.modes(), [unit_mode(mode::RUN, w.player)]);
}

// ---- SkeletonBow -----------------------------------------------

#[test]
fn skeleton_bow_vectors() {
    let setup = || {
        let mut w = World::new(row(37, &[75, 15, 50, 5, 6]));
        let s = other(&mut w, (110, 100));
        w.fake.secondary = Some((s, 10));
        w
    };
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        let s = w.fake.secondary.unwrap().0;
        if i == 1 {
            // 87; 64 ≥ 20 → idle 15.
            assert_eq!(w.thinks(), [15]);
        } else {
            assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
        }
    }
    // No S: 51, 87, 53 ≥ 50 → idle 20 | 0 → walk in radius (5, 6) of T.
    let setup = || World::new(row(37, &[75, 15, 50, 5, 6]));
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        if i == 3 {
            assert_eq!(w.fake.log.last().unwrap(), "radius 5 6");
        } else {
            assert_eq!(w.thinks(), [20]);
        }
    }
    // E ≥ 20 counts as no S.
    let mut w = setup();
    let s = other(&mut w, (130, 100));
    w.fake.secondary = Some((s, 20));
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [20]);
    // AI state 3/19 with S: A1 at S, no draw.
    let mut w = setup();
    let s = other(&mut w, (110, 100));
    w.fake.secondary = Some((s, 30));
    w.fake.ai_state = 19;
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
    assert_eq!(steps_since(&w, 1), 0);
    // P(aip1) fails, then `lo' % 100` < 20 → circle 3 at T.
    let lo = seed_with(3, |v| v[0] >= 75 && v[1] < 20);
    let mut w = setup();
    let s = other(&mut w, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(w.vel_request().steps, 3);
    assert_eq!(steps_since(&w, lo), 3);
}

// ---- FoulCrowNest ----------------------------------------------

#[test]
fn foul_crow_nest_vectors() {
    // D ≤ 20, summon not due: idle `lo' % 10` + 20 = 21, 27, 23, 20.
    let setup = || World::new(row(43, &[100, 0, 6]));
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    let idles: Vec<Vec<i32>> = ws.iter().map(|w| w.thinks()).collect();
    assert_eq!(idles, [[21], [27], [23], [20]]);
    // D > 20 → idle 25.
    let mut w = setup();
    w.think_with(Some(w.player), 21, false);
    assert_eq!(w.thinks(), [25]);
    // Quota reached: no-drop flag and death mode at (0, 0).
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[1] = 6;
    w.think_with(Some(w.player), 10, false);
    assert!(w.fake.log.contains(&"flag 0x20000".to_string()));
    assert_eq!(w.fake.modes(), ["mode 0 Point(0, 0)"]);
    // Due (|frame − param 0| ≥ aip1) and the footprint passes: summon
    // `Skill1` at T, param 0 := frame, param 1 += 1. The init set param 0
    // to the install frame (0).
    let mut w = setup();
    w.monstats[0].skill1 = 9;
    w.modes[0] = [10, 0, 0, 0, 0, 0, 0, 0];
    w.fake.footprint = true;
    w.game.frame = 100;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [100, 1]);
    // Not due at frame 199: idle (back to neutral from the skill mode).
    w.game.frame = 199;
    w.fake.log.clear();
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::NEUTRAL, w.mon)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [100, 1]);
    // Footprint fails: param 0 still moves, then idle.
    let mut w = setup();
    w.monstats[0].skill1 = 9;
    w.game.frame = 100;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [100, 0]);
    assert!(w.fake.modes().is_empty());
}

// ---- BloodRaven ------------------------------------------------

fn raven() -> World {
    let mut w = World::new(row(59, &[]));
    w.monstats[0].skill1 = 9;
    w.modes[0] = [14, 0, 0, 0, 0, 0, 0, 0];
    w
}

#[test]
fn blood_raven_raise_vector() {
    // D = 8, at home, not C, param 0 = 0 → 3, param 1 = 0: 51 ≥ 3, no
    // raise (… ) | 0 < 3: L = 7, raise at (T.x − 7, T.y − 6), param 1 = 1.
    let ws = per_seed(raven, |w| w.think_with(Some(w.player), 8, false));
    for (i, w) in ws.iter().enumerate() {
        let c = w.store.control(w.mon).unwrap();
        // Step 1 made the home command at its own position.
        assert_eq!(c.commands[0].params[..3], [10, 100, 100]);
        if i == 3 {
            assert_eq!(w.fake.modes(), ["mode 14 Point(98, 94)"]);
            assert_eq!(c.params[..2], [0, 1]);
        } else {
            assert_eq!(c.params[0], 3, "seed {}", SEEDS[i]);
            assert_eq!(c.params[1], 0);
            assert!(!w.fake.modes().iter().any(|m| m.starts_with("mode 14")));
        }
    }
}

#[test]
fn blood_raven_leash_and_moves() {
    // D > 45 → idle 5.
    let mut w = raven();
    w.think_with(Some(w.player), 46, false);
    assert_eq!(w.thinks(), [5]);
    // Far from home (> 50): run home, param 2 := 1.
    let mut w = raven();
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [10, 160, 100, 0, 0],
    }];
    w.think_with(Some(w.player), 8, false);
    assert_eq!(w.fake.modes(), ["mode 15 Point(160, 100)"]);
    assert_eq!(w.store.control(w.mon).unwrap().params[2], 1);
    assert_eq!((w.vel_request().method, w.vel_request().speed), (7, 100));
    // Returning (param 2) and farther than 5: run home again.
    let mut w = raven();
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [10, 110, 100, 0, 0],
    }];
    w.store.control_mut(w.mon).unwrap().params[2] = 1;
    w.think_with(Some(w.player), 8, false);
    assert_eq!(w.fake.modes(), ["mode 15 Point(110, 100)"]);
    // D > 20: run near T by max(D / 2, 12) (wander draws around T).
    let mut w = raven();
    w.seed(1);
    w.think_with(Some(w.player), 30, false);
    let mut s = Seed::init_low(1);
    let (x, y) = wander_point(&mut s, (105, 100), 15);
    assert_eq!(w.fake.modes(), [format!("mode 15 Point({x}, {y})")]);
    // Step 7: D ≤ 5, the 30 % back-off fails → A1 at T.
    let lo = seed_with(1, |v| v[0] >= 30);
    let mut w = raven();
    w.monstats[0].skill1 = 0xFFFF;
    w.seed(lo);
    w.think_with(Some(w.player), 3, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // Step 6: `lo' % 100` < 30 and D < 12 → run away by 12 − D.
    let lo = seed_with(1, |v| v[0] < 30);
    let mut w = raven();
    w.monstats[0].skill1 = 0xFFFF;
    w.seed(lo);
    w.think_with(Some(w.player), 3, true);
    assert_eq!(w.fake.modes(), ["mode 15 Point(91, 100)"]);
    // Step 5.2: S, roll < 80 → A1 at S (no `Skill2`).
    let lo = seed_with(3, |v| v[0] >= 5 && v[1] < 80);
    let mut w = raven();
    w.monstats[0].skill1 = 0xFFFF;
    let s = other(&mut w, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
}

// ---- SkeletonMage ----------------------------------------------

#[test]
fn skeleton_mage_vectors() {
    let setup = || {
        let mut w = World::new(row(64, &[35, 9, 30, 5, 0, 18, 20, 5]));
        let s = other(&mut w, (112, 100));
        w.fake.secondary = Some((s, 12));
        w
    };
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    let s = |w: &World| w.fake.secondary.unwrap().0;
    // 51 ≥ 30; 31 < 35 → A1 at S.
    assert_eq!(ws[0].fake.modes(), [unit_mode(mode::ATTACK1, s(&ws[0]))]);
    // 87; 64; step 2: 71 ≥ 30; 25 ≥ 20 → idle 5.
    assert_eq!(ws[1].thinks(), [5]);
    // 53; 46; step 2: 20 < 30 → walk to T, 9 steps.
    assert_eq!(ws[2].fake.modes(), [unit_mode(mode::WALK, ws[2].player)]);
    assert!(ws[2].fake.log.contains(&"steps 9".to_string()));
    // 0 < 30 → walk to S, 9 steps, speed 10.
    assert_eq!(ws[3].fake.modes(), [unit_mode(mode::WALK, s(&ws[3]))]);
    assert_eq!(ws[3].vel_request().speed, 10);
    // E ≤ aip4 and P(aip5): escape from S; a failed escape → A1 at T.
    let mut w = World::new(row(64, &[35, 9, 30, 5, 100, 18, 20, 5]));
    let s = other(&mut w, (103, 100));
    w.fake.secondary = Some((s, 3));
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes()[1], unit_mode(mode::ATTACK1, w.player));
    assert_eq!(w.vel_request().speed, 25);
    // No S: E = 0x7FFFFFFF, the step-2 test always draws.
    let mut w = World::new(row(64, &[35, 9, 30, 5, 0, 18, 20, 5]));
    w.seed(4_014_346_870);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
}

// ---- Arach -----------------------------------------------------

#[test]
fn arach_vectors() {
    // State 0, not C, AI state 0, params 1, 2 = 0: param 2 → 1;
    // 51 ≥ 15; 31 ≥ 20 → idle 15 | 87; 64 | 53; 46 | 0 < 15 → lunge.
    let setup = || World::new(row(26, &[45, 33, 15, 8, 25]));
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        assert_eq!(w.store.control(w.mon).unwrap().params[2], 1);
        if i == 3 {
            assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
            assert_eq!(w.vel_request().method, 13);
            assert_eq!(w.store.control(w.mon).unwrap().params[1], 1);
        } else {
            assert_eq!(w.thinks(), [15]);
        }
    }
    // Param 2 wraps to 0 past 20; the engage roll only at param 2 = 1.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params = [0, 0, 20];
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[2], 0);
    assert_eq!(steps_since(&w, 1), 1);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn arach_retreat_and_combat() {
    let setup = || World::new(row(26, &[45, 33, 15, 8, 25]));
    // State 1, healthy (> 75): state 0; P(aip3) fails → circle 6.
    let lo = seed_with(2, |v| v[0] >= 15);
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params = [1, 1, 0];
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [0, 0]);
    assert_eq!(w.vel_request().steps, 6);
    // State 1, hurt, far: state 0, circle 12.
    let mut w = setup();
    w.fake.life = 50;
    w.store.control_mut(w.mon).unwrap().params[0] = 1;
    w.think_with(Some(w.player), 8, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    assert_eq!(w.vel_request().steps, 12);
    // State 1, hurt, close: escape by 4.
    let mut w = setup();
    w.fake.life = 50;
    w.store.control_mut(w.mon).unwrap().params[0] = 1;
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes(), ["mode 2 Point(96, 100)"]);
    // C, P(aip1) fails, hurt below aip5: state 1, lay (`Skill1`, (0, 0)).
    let lo = seed_with(1, |v| v[0] >= 45);
    let mut w = setup();
    w.monstats[0].skill1 = 9;
    w.modes[0] = [5, 0, 0, 0, 0, 0, 0, 0];
    w.fake.life = 20;
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
    assert_eq!(w.fake.modes(), ["mode 5 Point(0, 0)"]);
    // … with state 22: escape by 8 instead.
    let mut w = setup();
    w.monstats[0].skill1 = 9;
    w.fake.life = 20;
    w.fake.states.insert((w.mon, 22));
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), ["mode 2 Point(92, 100)"]);
    // C, P(aip1) passes → state 2, A1.
    let mut w = setup();
    w.seed(4_014_346_870);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 2);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
}

// ---- Fetish ----------------------------------------------------

#[test]
fn fetish_vectors() {
    // State 2, D = 15, param 1 = 0: param 1 → 1; 51, 87, 53 → idle 10 |
    // 0 < 20 → circle 4 (low byte 46 → method 5).
    let setup = || {
        let mut w = World::new(row(30, &[100, 10, 4, 33]));
        w.store.control_mut(w.mon).unwrap().params[0] = 2;
        w
    };
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 15, false));
    for (i, w) in ws.iter().enumerate() {
        assert_eq!(w.store.control(w.mon).unwrap().params[..2], [2, 1]);
        if i == 3 {
            assert_eq!(w.vel_request().method, 5);
            assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
        } else {
            assert_eq!(w.thinks(), [10]);
        }
    }
    // The second far think resets state and counter.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[1] = 1;
    w.think_with(Some(w.player), 15, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [0, 0]);
    // Close (D ≤ 12): escape by 14; failed → state 0, idle 10.
    let mut w = setup();
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [0, 0]);
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn fetish_rhythm_and_commands() {
    let setup = || World::new(row(30, &[100, 10, 4, 33]));
    // A type-1 command naming a unit: walk to it, free the command.
    let mut w = setup();
    let leader = other(&mut w, (120, 100));
    let guid = w.game.lists.unit(leader).unwrap().guid as i32;
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [1, guid, 1, 0, 0],
    }];
    w.store.control_mut(w.mon).unwrap().params = [1, 3, 0];
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, leader)]);
    assert!(w.commands().is_empty());
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [0, 0]);
    // Another command is freed and the think goes on (state 0, C → A1).
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [5, 0, 0, 0, 0],
    }];
    w.think_with(Some(w.player), 1, true);
    assert!(w.commands().is_empty());
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
    // State 1 past aip3 thinks with T above aip4 % life: back off.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params = [1, 4, 0];
    w.fake.life_of.insert(w.player, 50);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [2, 0]);
    assert_eq!(w.fake.modes(), ["mode 2 Point(86, 100)"]);
    // … T at or below aip4: keep attacking.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params = [1, 4, 0];
    w.fake.life_of.insert(w.player, 33);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // Not C: velocity (13, 50, 0), walk flags 7.
    let mut w = setup();
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!((w.vel_request().method, w.vel_request().speed), (13, 50));
}

// ---- Vampire ---------------------------------------------------

#[test]
fn vampire_vectors() {
    // State 0, not C, D = 10, L ≥ 33, no S, param 2 = 0: 51 ≥ 40; 31 < 50
    // → circle 4 | 87; 64 → idle 10 | 53; 46 → circle 4 | 0 < 40; no F2 /
    // F4; no S → walk to T flags 7.
    let setup = || World::new(row(28, &[85, 40, 28, 25, 1]));
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
        match i {
            1 => assert_eq!(w.thinks(), [10]),
            3 => assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]),
            _ => {
                assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
                assert_eq!(w.vel_request().steps, 4);
            }
        }
    }
}

#[test]
fn vampire_bolts_upgrades_and_flight() {
    let setup = || {
        let mut w = World::new(row(28, &[85, 40, 28, 25, 7]));
        w.monstats[0].skill1 = 11;
        w.monstats[0].skill2 = 12;
        w.monstats[0].skill4 = 0xFFFF; // −1: not tested (edge 15)
        w.modes[0] = [8, 9, 10, 11, 0, 0, 0, 0];
        w
    };
    // Cooldown counts down.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[2] = 5;
    w.fake.anim.insert(w.mon, mode::WALK);
    w.think_with(Some(w.player), 40, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[2], 4);
    // AI state, C, draw ≤ 30 with F1 → bolt: roll < 50 → Skill1, else
    // Skill4 (−1, requested anyway).
    let lo = seed_with(2, |v| v[0] <= 30 && v[1] >= 50);
    let mut w = setup();
    w.fake.ai_state = 3;
    w.seed(lo);
    w.think_with(Some(w.player), 5, true);
    assert!(w.fake.log.contains(&"skill -1".to_string()));
    assert_eq!(w.fake.modes(), [unit_mode(11, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [1, 5]);
    // Fleeing (state 2), close: escape by 8 with the run bonus speed.
    let mut w = setup();
    w.monstats[0].velocity = 5;
    w.monstats[0].run = 9;
    w.fake.life = 50;
    w.store.control_mut(w.mon).unwrap().params[0] = 2;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.vel_request().speed, 80);
    assert_eq!(w.fake.modes(), ["mode 2 Point(92, 100)"]);
    // Fleeing, healed (≥ 75): state 1, walk to T.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[0] = 2;
    w.think_with(Some(w.player), 20, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // Upgrade (not C, roll < aip2, F2 and no cooldown, roll < aip4):
    // `Skill2` at T, cooldown 11.
    let lo = seed_with(2, |v| v[0] < 40 && v[1] < 25);
    let mut w = setup();
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(9, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[2], 11);
    // Hurt below 33: state 2 and escape by 8.
    let mut w = setup();
    w.fake.life = 30;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 2);
    assert_eq!(w.fake.modes(), ["mode 2 Point(92, 100)"]);
}

// ---- Bighead, BloodHawk, HellMeteor -----------------

#[test]
fn bighead_vectors() {
    // Hurt, D = 10, no S: 51, 87, 53 ≥ 40 → idle 10 | 0 → circle 3.
    let setup = || {
        let mut w = World::new(row(4, &[88, 40, 0, 60]));
        w.fake.life = 50;
        w
    };
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        if i == 3 {
            assert_eq!(w.vel_request().steps, 3);
            assert_eq!(w.vel_request().method, 5);
        } else {
            assert_eq!(w.thinks(), [10]);
        }
        assert_eq!(steps_since(w, SEEDS[i]), if i == 3 { 2 } else { 1 });
    }
    // AI state, not C → A2 at T.
    let mut w = setup();
    w.fake.ai_state = 3;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    // Healthy: C → A1; D < 15 with S and roll < aip3 (0) fails → walk.
    let mut w = World::new(row(4, &[88, 40, 0, 60]));
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    let mut w = World::new(row(4, &[88, 40, 0, 60]));
    let s = other(&mut w, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 1), 1);
    // Hurt and close: escape by 5; failed → A2. Far: walk 6 steps.
    let mut w = setup();
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 2, false);
    assert_eq!(w.fake.modes()[1], unit_mode(mode::ATTACK2, w.player));
    let mut w = setup();
    w.think_with(Some(w.player), 16, false);
    assert!(w.fake.log.contains(&"steps 6".to_string()));
    // Hurt, S and roll < aip4 → A2 at T (not S).
    let mut w = setup();
    let s = other(&mut w, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(1); // 51 < 60
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
}

#[test]
fn blood_hawk_vectors() {
    // Not C, D = 10, param 0 = 0: 51 ≥ 30; 31 < 90 → speed −50, wander 4
    // | 87; 64 → wander 4 | 53; 46 → wander 4 | 0 < 30 → charge.
    let setup = || World::new(row(5, &[30, 90, 5, 50, 100]));
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 10, false));
    for (i, w) in ws.iter().enumerate() {
        if i == 3 {
            let v = w.vel_request();
            assert_eq!((v.speed, v.steps), (100, 10));
            assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
            assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
        } else {
            assert_eq!(w.vel_request().speed, -50);
            let mut s = Seed::init_low(SEEDS[i]);
            s.step();
            s.step();
            let (x, y) = wander_point(&mut s, (100, 100), 4);
            assert_eq!(w.fake.modes(), [format!("mode 2 Point({x}, {y})")]);
        }
    }
    // Charged and C → A1, param 0 := 0, no draw.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[0] = 1;
    w.seed(1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
    // C, P(aip3) fails → back off (speed aip4, escape by 4).
    let mut w = setup();
    w.seed(1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.vel_request().speed, 50);
    assert_eq!(w.fake.modes(), ["mode 2 Point(96, 100)"]);
    // Not C, D ≤ 3, no charge → back off; a failed escape → A1.
    let mut w = setup();
    w.fake.walk_fails = true;
    w.seed(1);
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes()[1], unit_mode(mode::ATTACK1, w.player));
    // P(aip2) fails: velocity (0, 0, 0) writes nothing, wander 3.
    let lo = seed_with(2, |v| v[0] >= 30 && v[1] >= 90);
    let mut w = setup();
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.vel_request(), VelocityRequest::default());
}

#[test]
fn hell_meteor_vectors() {
    // 51, 87, 53 ≥ 50 → idle 50 | 0 < 50: roll(20) 2, 13 → `Skill1` at
    // (92, 103).
    let setup = || {
        let mut w = World::new(row(33, &[50, 50, 10]));
        w.monstats[0].skill1 = 9;
        w.modes[0] = [4, 0, 0, 0, 0, 0, 0, 0];
        w
    };
    let ws = per_seed(setup, |w| w.think_with(None, 0, false));
    for (i, w) in ws.iter().enumerate() {
        if i == 3 {
            assert_eq!(w.fake.modes(), ["mode 4 Point(92, 103)"]);
        } else {
            assert_eq!(w.thinks(), [50]);
        }
    }
    // No `Skill1`: idle aip2 without a draw.
    let mut w = World::new(row(33, &[50, 50, 10]));
    w.seed(4_014_346_870);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [50]);
    assert_eq!(steps_since(&w, 4_014_346_870), 0);
}

// ---- SandRaider ------------------------------------------------

fn raider() -> World {
    let mut w = World::new(row(8, &[40, 70, 75, 70, 18, 0, 50]));
    w.monstats[0].skill1 = 9;
    w.modes[0] = [8, 0, 0, 0, 0, 0, 0, 0];
    w
}

#[test]
fn sand_raider_charges_glows_and_hits() {
    // A fresh counter clears states 90 and 91.
    let mut w = raider();
    w.fake.states.insert((w.mon, 90));
    w.fake.states.insert((w.mon, 91));
    w.think_with(Some(w.player), 1, true);
    assert!(!w.fake.states.contains(&(w.mon, 90)));
    assert!(!w.fake.states.contains(&(w.mon, 91)));
    // Counter reaches aip5 (18): the red overlay 46 (aip6 ≠ 1), idle
    // aidel + 1.
    let mut w = raider();
    w.store.control_mut(w.mon).unwrap().params[0] = 17;
    w.think_with(Some(w.player), 1, true);
    assert!(w.fake.log.contains(&"overlay 46".to_string()));
    assert_eq!(w.thinks(), [16]);
    // Past aip5: red state 91 on, charged; C → `Skill1` at T, reset.
    let mut w = raider();
    w.store.control_mut(w.mon).unwrap().params[0] = 18;
    w.think_with(Some(w.player), 1, true);
    assert!(w.fake.states.contains(&(w.mon, 91)));
    assert_eq!(w.fake.modes(), [unit_mode(8, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [0, 0]);
    // aip6 = 1: blue (overlay 150, state 90).
    let mut w = raider();
    w.monstats[0].aip6 = 1;
    w.store.control_mut(w.mon).unwrap().params[0] = 17;
    w.think_with(Some(w.player), 1, true);
    assert!(w.fake.log.contains(&"overlay 150".to_string()));
    let mut w = raider();
    w.monstats[0].aip6 = 1;
    w.store.control_mut(w.mon).unwrap().params[0] = 30;
    w.monstats[0].skill1 = 0xFFFF;
    w.think_with(Some(w.player), 1, false);
    assert!(w.fake.states.contains(&(w.mon, 90)));
    // Not charged, C: P(aip3) → roll < aip7 → A2, else A1.
    let lo = seed_with(2, |v| v[0] < 75 && v[1] < 50);
    let mut w = raider();
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
}

#[test]
fn sand_raider_help_circle_and_rest() {
    // Hurt below aip1: walk to the nearest evil monster.
    let mut w = raider();
    let m = other(&mut w, (120, 100));
    w.fake.evil_monster = Some(m);
    w.fake.life = 30;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, m)]);
    // None found: param 2 += 1; at 7 no more searches.
    let mut w = raider();
    w.fake.life = 30;
    w.store.control_mut(w.mon).unwrap().params[2] = 6;
    w.seed(1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[2], 7);
    w.fake.log.clear();
    w.think_with(Some(w.player), 1, true);
    assert!(!w.fake.log.contains(&"help scan".to_string()));
    // D > 4, not charged, P(aip2) → circle 0.
    let mut w = raider();
    w.seed(4_014_346_870);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 4_014_346_870), 2);
    // Not C, not charged, P(aip4) fails → rest: idle 15; the counter
    // resets after aip5 + max(24 − aip5, 6) = 24 thinks.
    let lo = seed_with(2, |v| v[0] >= 70 && v[1] >= 70);
    let mut w = raider();
    w.store.control_mut(w.mon).unwrap().params[0] = 5;
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [15]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 6);
    let mut w = raider();
    w.monstats[0].skill1 = 0xFFFF;
    w.store.control_mut(w.mon).unwrap().params = [24, 1, 0];
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [0, 0]);
    assert_eq!(w.thinks(), [15]);
}

// ---- Baboon ----------------------------------------------------

#[test]
fn baboon_fights_and_starts_regenerating() {
    let setup = || {
        let mut w = World::new(row(11, &[33, 20, 55, 0, 1]));
        w.monstats[0].velocity = 5;
        w.monstats[0].run = 9;
        w
    };
    // Not C → lunge (velocity method 13, walk flags 7), no draw.
    let mut w = setup();
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(w.vel_request().method, 13);
    assert_eq!(steps_since(&w, 1), 0);
    // AI state, hurt below aip1, roll < 50: regenerate. param 0 :=
    // roll(5) + 2, hpregen R = 80 → bonus aip5 × R / 8 = 10.
    let lo = seed_with(1, |v| v[0] < 50);
    let mut w = setup();
    w.fake.ai_state = 3;
    w.fake.life = 20;
    w.fake.stats.insert((w.mon, 74), 80);
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    let mut s = Seed::init_low(lo);
    s.step();
    let n = (s.roll(5) + 2) as i32;
    assert_eq!(w.store.control(w.mon).unwrap().params, [n, 0, 10]);
    assert_eq!(w.fake.stats[&(w.mon, 74)], 90);
    assert_eq!((w.vel_request().method, w.vel_request().speed), (2, 80));
    assert_eq!(w.fake.modes(), ["mode 2 Point(85, 100)"]);
    // C, attacked flag clear: param 1 := 1; P(aip4) (0) fails → A2.
    let mut w = setup();
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    // Attacked, P(aip3) fails, roll < aip2 → circle 3, param 1 := 0, idle.
    let lo = seed_with(2, |v| v[0] >= 55 && v[1] < 20);
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[1] = 1;
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 0);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn baboon_regeneration_countdown() {
    let setup = || {
        let mut w = World::new(row(11, &[33, 20, 55, 0, 1]));
        w.fake.stats.insert((w.mon, 74), 90);
        w.store.control_mut(w.mon).unwrap().params = [1, 1, 10];
        w
    };
    // Last regen think: hpregen − param 2; not C, hurt → step 4: far and
    // not in AI state → (circle) idle 20.
    let mut w = setup();
    w.fake.life = 50;
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.fake.stats[&(w.mon, 74)], 80);
    assert_eq!(w.store.control(w.mon).unwrap().params, [0, 0, 10]);
    assert_eq!(w.thinks(), [20]);
    // Healed (> 75), not C: stop regenerating and lunge.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[0] = 3;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.stats[&(w.mon, 74)], 80);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // Regenerating, C, draw < 33 → A1 / A2 by aip4.
    let lo = seed_with(2, |v| v[0] < 33);
    let mut w = setup();
    w.fake.life = 50;
    w.store.control_mut(w.mon).unwrap().params[0] = 3;
    w.seed(lo);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    // Close: escape by 15 with think delete; failed, not C → wander 5.
    let mut w = setup();
    w.fake.life = 50;
    w.fake.walk_fails = true;
    w.store.control_mut(w.mon).unwrap().params[0] = 3;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes().len(), 2);
}

// ---- SandMaggot ------------------------------------------------

fn maggot() -> World {
    let mut w = World::new(row(15, &[35, 35, 2, 75, 120]));
    (
        w.monstats[0].skill1,
        w.monstats[0].skill2,
        w.monstats[0].skill3,
    ) = (21, 22, 23);
    w.modes[0] = [8, 9, 10, 0, 0, 0, 0, 0];
    w
}

#[test]
fn sand_maggot_burrows_and_surfaces() {
    // Above ground, no T, no S, frame past param 1: burrow at (0, 0).
    let mut w = maggot();
    w.game.frame = 10;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), ["mode 9 Point(0, 0)"]);
    assert_eq!(w.thinks(), [40]);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [3, 130]);
    // Burrowed without T or S near: wait 20 (replaces the
    // pending think at 40, due later than frame + 20).
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [30]);
    // A pending think due earlier is kept.
    w.game.frame = 15;
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [30]);
    // Burrowed with T, before param 1: wait 20.
    let mut w = maggot();
    w.store.control_mut(w.mon).unwrap().params = [3, 50, 0];
    w.game.frame = 40;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [60]);
    // … after: surface (`Skill1` at T), wait 25, state 1.
    let mut w = maggot();
    w.store.control_mut(w.mon).unwrap().params = [3, 50, 0];
    w.game.frame = 60;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(8, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[..2], [1, 180]);
    assert_eq!(w.thinks(), [85]);
}

#[test]
fn sand_maggot_above_ground() {
    // C and P(aip4) → A1.
    let mut w = maggot();
    w.seed(4_014_346_870);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // `lo' % 100` < 20 → circle 6.
    let lo = seed_with(1, |v| v[0] < 20);
    let mut w = maggot();
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.vel_request().steps, 6);
    // Lay: param 2 < aip3, roll < aip1: state 2 → `Skill3`, state 1.
    let lo = seed_with(3, |v| v[0] >= 20 && v[1] < 35);
    let mut w = maggot();
    w.store.control_mut(w.mon).unwrap().params = [2, 0, 0];
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params, [1, 0, 1]);
    // … other states circle and become 2.
    let mut w = maggot();
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 2);
    // Otherwise wait 12.
    let lo = seed_with(2, |v| v[0] >= 20 && v[1] >= 35);
    let mut w = maggot();
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [12]);
    // Hurt below 25 with S under 7: roll < 20 → burrow at T.
    let lo = seed_with(1, |v| v[0] < 20);
    let mut w = maggot();
    w.fake.life = 20;
    let s = other(&mut w, (103, 100));
    w.fake.secondary = Some((s, 5));
    w.game.frame = 10;
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(9, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 3);
}

#[test]
fn sand_maggot_alternate() {
    // Re-installed while running: the alternate. Command 14 with param 4
    // = 1 → `Skill1` at the path target, wait 30, param 4 := 0.
    let mut w = maggot();
    let mon = w.mon;
    w.with(|g, cx| install(g, cx, mon, 0));
    assert_eq!(w.store.control(mon).unwrap().function, 0x005F_1750);
    let t = other(&mut w, (110, 100));
    w.fake.path_target = Some(t);
    w.store.control_mut(mon).unwrap().commands = vec![AiCommand {
        params: [14, 0, 0, 0, 1],
    }];
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(8, t)]);
    assert_eq!(w.commands(), [[14, 0, 0, 0, 0]]);
    assert_eq!(w.thinks(), [30]);
    // Otherwise: re-install (back to the think), keeping param 2; wait 1.
    delete_thinks(&mut w.game, mon);
    w.store.control_mut(mon).unwrap().params = [3, 9, 2];
    w.game.frame = 50;
    w.think_with(None, 0, false);
    let c = w.store.control(mon).unwrap();
    assert_eq!(c.function, 0x005F_1800);
    assert_eq!(c.params, [0, 0, 2]);
    assert_eq!(w.thinks(), [51]);
}

// ---- Scarab ----------------------------------------------------

#[test]
fn scarab_vectors() {
    // C, D = 25, no command: 51 < 75; 31 < 35 → `Skill1` at T | 87 → idle
    // 15 | 53; 46; 20 < 50 → A1 | 0; 42; 13 → A1.
    let setup = || {
        let mut w = World::new(row(20, &[75, 50, 15, 35, 20]));
        w.monstats[0].skill1 = 9;
        w.modes[0] = [10, 0, 0, 0, 0, 0, 0, 0];
        w
    };
    let ws = per_seed(setup, |w| w.think_with(Some(w.player), 25, true));
    assert_eq!(ws[0].fake.modes(), [unit_mode(10, ws[0].player)]);
    assert_eq!(ws[1].thinks(), [15]);
    for w in &ws[2..] {
        assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    }
    // Own pack leader, D < 20, P(aip5): command 1 to the minions and to
    // itself; then C → free it, `Skill1` at T.
    let mut w = setup();
    let mon = w.mon;
    let guid = w.game.lists.unit(mon).unwrap().guid;
    let minion = other(&mut w, (101, 100));
    let mguid = w.game.lists.unit(minion).unwrap().guid;
    w.store.entry(minion).control = Some(AiControl::default());
    {
        let c = w.store.control_mut(mon).unwrap();
        c.minion_owner = Some(UnitRef {
            ty: UnitType::Monster,
            guid,
        });
        c.minions = vec![mguid];
    }
    w.seed(4_014_346_870);
    w.think_with(Some(w.player), 5, true);
    assert_eq!(w.store.control(minion).unwrap().commands.len(), 1);
    assert!(w.commands().is_empty());
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
}

#[test]
fn scarab_commands_and_circling() {
    let setup = || World::new(row(20, &[75, 50, 15, 35, 20]));
    // A type-1 command, not C: velocity (2, 100, 0), walk to T; a failed
    // walk frees it.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [1, 0, 0, 0, 0],
    }];
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 30, false);
    assert!(w.commands().is_empty());
    assert_eq!((w.vel_request().method, w.vel_request().speed), (2, 100));
    // Another type stays; not C, not circled: circle 0, param 0 := 1.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [5, 0, 0, 0, 0],
    }];
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.commands().len(), 1);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
    // Circled: velocity (2, 0, 4), walk flags 7, `lo' % 100` > 10 →
    // param 0 := 0.
    let mut w = setup();
    w.store.control_mut(w.mon).unwrap().params[0] = 1;
    w.seed(1);
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(w.vel_request().steps, 4);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
}

// ---- Npc --------------------------------------------------------

fn npc_world(nodes: Option<Vec<MapNode>>) -> World {
    let mut w = World::new(row(32, &[]));
    let c = w.store.control_mut(w.mon).unwrap();
    // Home already set (step 1 is the first think).
    c.commands = vec![AiCommand {
        params: [10, 100, 100, 0, 0],
    }];
    c.map_ai = nodes;
    w
}

#[test]
fn npc_map_ai_vector() {
    // Map AI with 3 nodes: 51 < 66; node 0 | 87 ≥ 66 → 0, idle 8 | 53;
    // node 0 | 0; node 2.
    let nodes = vec![
        MapNode {
            action: 1,
            x: 110,
            y: 100,
        },
        MapNode {
            action: 1,
            x: 90,
            y: 100,
        },
        MapNode {
            action: 2,
            x: 100,
            y: 120,
        },
    ];
    let ws = per_seed(
        || npc_world(Some(nodes.clone())),
        |w| w.think_with(None, 0, false),
    );
    for (i, w) in ws.iter().enumerate() {
        match i {
            1 => {
                assert!(w.fake.modes().is_empty());
                assert_eq!(w.thinks(), [8]);
            }
            3 => {
                assert_eq!(w.fake.modes(), ["mode 2 Point(100, 120)"]);
                assert!(w.commands().contains(&[4, 100, 120, 20, 10]));
            }
            _ => {
                assert_eq!(w.fake.modes(), ["mode 2 Point(110, 100)"]);
                assert!(w.commands().contains(&[4, 110, 100, 12, 10]));
                assert!(w.fake.log.contains(&"steps 0".to_string()));
            }
        }
    }
    // No record: no draw, idle 8.
    let mut w = npc_world(None);
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(steps_since(&w, 1), 0);
    assert_eq!(w.thinks(), [8]);
    // Actions 4 / 5: command 7 (8 or 9 when the class has it, else 1).
    let mut w = npc_world(Some(vec![MapNode {
        action: 5,
        x: 110,
        y: 100,
    }]));
    w.seed(1);
    w.think_with(None, 0, false);
    assert!(w.commands().contains(&[7, 9, 110, 100, 4]));
    assert!(w.commands().contains(&[4, 110, 100, 12, 10]));
    // At the node (path distance 0): action 1 does nothing → idle 8.
    let mut w = npc_world(Some(vec![MapNode {
        action: 1,
        x: 100,
        y: 100,
    }]));
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [8]);
}

#[test]
fn npc_commands_walk_and_wander() {
    // Command 4 far: velocity method by G (0 → 5, then 1 → 7), walk step
    // 0, G += 1, tries − 1.
    let mut w = npc_world(None);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(AiCommand {
            params: [4, 110, 100, 2, 10],
        });
    w.think_with(None, 0, false);
    assert_eq!(w.vel_request().method, 5);
    assert_eq!(w.store.npc_walk_counter, 1);
    assert!(w.commands().contains(&[4, 110, 100, 1, 10]));
    w.think_with(None, 0, false);
    assert_eq!(w.vel_request().method, 7);
    assert!(w.commands().contains(&[4, 110, 100, 0, 10]));
    // Tries used up: falls through → idle 8.
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [8]);
    // Near (≤ 3): idle delay, tries − 1.
    let mut w = npc_world(None);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(AiCommand {
            params: [4, 102, 100, 12, 10],
        });
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [10]);
    assert!(w.commands().contains(&[4, 102, 100, 11, 10]));
    // Command 5: odd count → wander w; even → idle t while idles last.
    let mut w = npc_world(None);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(AiCommand {
            params: [5, 3, 4, 1, 30],
        });
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes().len(), 1);
    assert!(w.commands().contains(&[5, 2, 4, 1, 30]));
    w.think_with(None, 0, false);
    assert!(w.commands().contains(&[5, 1, 4, 0, 30]));
    assert_eq!(w.thinks(), [30]);
}

#[test]
fn npc_command_7_mode_actions() {
    let cmd7 = |m: i32, n: i32| AiCommand {
        params: [7, m, 100, 100, n],
    };
    // Invalid mode → m := 0, idle 50.
    let mut w = npc_world(None);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(cmd7(3, 0));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [50]);
    assert!(w.commands().contains(&[7, 0, 100, 100, 0]));
    // At the point: charsi faces 56, mode 8 on itself, m := 0.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 154);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(cmd7(8, 4));
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"facing 56".to_string()));
    assert_eq!(w.fake.modes(), [unit_mode(8, w.mon)]);
    assert!(w.commands().contains(&[7, 0, 100, 100, 4]));
    // Already in that mode: idle 50.
    let mut w = npc_world(None);
    w.fake.anim.insert(w.mon, 8);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(cmd7(8, 4));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [50]);
    // Away with tries: velocity (7, 0, 0), walk step 0 there, tries − 1.
    let mut w = npc_world(None);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(AiCommand {
            params: [7, 9, 110, 100, 2],
        });
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), ["mode 2 Point(110, 100)"]);
    assert!(w.commands().contains(&[7, 9, 110, 100, 1]));
    // larzuk: roll(100) > 4 → m := 0, idle 50.
    let lo = seed_with(1, |v| v[0] > 4);
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 511);
    w.store
        .control_mut(w.mon)
        .unwrap()
        .commands
        .push(cmd7(8, 4));
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [50]);
    assert!(w.commands().contains(&[7, 0, 100, 100, 4]));
}

#[test]
fn npc_class_cases() {
    // jerhyn, palace inactive → idle 40.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 201);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [40]);
    // jerhyn, active, b ≠ 0 (idle 20), a = 0 → end.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 201);
    w.fake.jerhyn = Some((0, 1, false));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    // jerhyn, guard moving → home := (x + 9, y), idle 50.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 201);
    w.fake.jerhyn = Some((1, 0, true));
    w.think_with(None, 0, false);
    assert!(w.commands().contains(&[10, 109, 100, 0, 0]));
    assert_eq!(w.thinks(), [50]);
    // alkor with the bird: mode 8 at (0, 0), reset.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 254);
    w.fake.alkor_bird = true;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), ["mode 8 Point(0, 0)"]);
    assert!(w.fake.log.contains(&"quest alkor reset".to_string()));
    // ormus: far → walk to the altar; near → mode 8 and altar mode.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 255);
    w.fake.ormus_altar = Some((110, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), ["mode 2 Point(110, 100)"]);
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 255);
    w.fake.ormus_altar = Some((103, 100));
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"quest ormus altar".to_string()));
    // cain5 near the portal point: activated, then the steps go on.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 265);
    w.fake.cain_town = Some((101, 100));
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"quest cain activated".to_string()));
    assert_eq!(w.thinks(), [8]);
    // drehya: Anya's portal, then the steps go on.
    let mut w = npc_world(None);
    w.fake.class.insert(w.mon, 512);
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"quest anya portal".to_string()));
    assert_eq!(w.thinks(), [8]);
}

// ---- boundaries (each failed a deliberate off-by-one) -------------

#[test]
fn boundaries() {
    // BloodRaven raises at most 2 × difficulty + 8 times (8 in Normal).
    for (done, raises) in [(7, true), (8, false)] {
        let mut w = raven();
        w.store.control_mut(w.mon).unwrap().params[1] = done;
        w.seed(4_014_346_870);
        w.think_with(Some(w.player), 8, false);
        let raised = w.fake.modes().iter().any(|m| m.starts_with("mode 14"));
        assert_eq!(raised, raises, "raises done {done}");
    }
    // BloodRaven step 3: D = 21 → max(10, 12) = 12 around T.
    let mut w = raven();
    w.seed(1);
    w.think_with(Some(w.player), 21, false);
    let mut s = Seed::init_low(1);
    let (x, y) = wander_point(&mut s, (105, 100), 12);
    assert_eq!(w.fake.modes(), [format!("mode 15 Point({x}, {y})")]);
    // Arach's think counter reaches 20 before it wraps.
    let mut w = World::new(row(26, &[45, 33, 15, 8, 25]));
    w.store.control_mut(w.mon).unwrap().params = [0, 0, 19];
    w.seed(1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[2], 20);
    // Scarab keeps "circled" only when the draw is ≤ 10.
    for (draw, kept) in [(10, 1), (11, 0)] {
        let lo = seed_with(1, |v| v[0] == draw);
        let mut w = World::new(row(20, &[75, 50, 15, 35, 20]));
        w.store.control_mut(w.mon).unwrap().params[0] = 1;
        w.seed(lo);
        w.think_with(Some(w.player), 30, false);
        assert_eq!(w.store.control(w.mon).unwrap().params[0], kept);
    }
    // Npc command 4: method 5 only when G's low two bits are 0.
    for (g, method) in [(2, 7), (4, 5)] {
        let mut w = npc_world(None);
        w.store.npc_walk_counter = g;
        w.store
            .control_mut(w.mon)
            .unwrap()
            .commands
            .push(AiCommand {
                params: [4, 110, 100, 2, 10],
            });
        w.think_with(None, 0, false);
        assert_eq!(w.vel_request().method, method, "G {g}");
    }
    // Map AI: a draw of 66 ends it (idle 8), 65 walks.
    for (draw, walks) in [(65, true), (66, false)] {
        let lo = seed_with(1, |v| v[0] == draw);
        let mut w = npc_world(Some(vec![MapNode {
            action: 1,
            x: 110,
            y: 100,
        }]));
        w.seed(lo);
        w.think_with(None, 0, false);
        assert_eq!(!w.fake.modes().is_empty(), walks, "draw {draw}");
    }
}
