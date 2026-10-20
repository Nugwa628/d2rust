use super::act2::{act_row, give_skill, logged, param_of, point_mode, set_param_of, world};
use super::act6::{own, sentry_world};
use super::npc::{seed_with, steps_since, unit_mode};
use super::*;
use crate::monsters::ai::bodies7::shadow_usable;

#[test]
fn jar_jar_with_the_door_open_and_no_home_idles_120() {
    // The door seam ≠ 0 and no home command (H = 0: the unit has no AI
    // control record to hold one): idle 120, nothing else.
    let mut w = world(act_row(81, &[]));
    w.fake.y.hooks.insert("PalaceDoorOpen".into());
    let mon = w.mon;
    w.store.entry(mon).control = None;
    let p = TickParam {
        target: None,
        distance: 0,
        combat: false,
        class: 0,
        class2: 0,
    };
    w.with(|g, cx| run_function(g, cx, AI_TABLE[81].think, mon, &p));
    assert_eq!(w.thinks(), [120]);
    assert!(w.fake.modes().is_empty());
}

#[test]
fn dark_wanderer_in_another_phase_idles_40() {
    let mut w = world(act_row(91, &[]));
    w.fake.y.wanderer = Some((130, 100));
    set_param_of(&mut w, 0, 5);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.thinks(), [40]);
    assert!(w.fake.modes().is_empty());
    assert_eq!(param_of(&w, 0), 5);
}

#[test]
fn death_sentry_corpse_radius_is_half_of_param3_plus_level_steps_of_param4() {
    // `Skill1` param3 = 10, param4 = 2 at level 5: r = (10 + 4 × 2) / 2 = 9
    // (strict): a corpse 8 from S is taken, one 9 away is not (the lightning
    // or idle follows); the search is made for (S, Skill1, level 5).
    let ds = [30, 0, 50, 16];
    for (kx, taken) in [(118, true), (119, false)] {
        let mut w = sentry_world(104, &ds);
        let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
        sk.param3 = 10;
        sk.param4 = 2;
        w.skills = vec![sk.clone(), sk.clone(), sk];
        set_param_of(&mut w, 1, 2);
        let s = w.add_unit(UnitType::Monster, (110, 100));
        let k = w.add_unit(UnitType::Monster, (kx, 100));
        w.fake.secondary = Some((s, 10));
        w.fake.x.corpse = Some(k);
        w.seed(seed_with(1, |v| v[0] >= 50));
        w.think_with(None, 0, false);
        assert!(logged(&w, "corpse search 1 5"));
        let used = w.fake.modes() == [unit_mode(mode::SKILL2, k)];
        assert_eq!(used, taken, "corpse at {kx}");
    }
}

/// A shadow warrior world whose owner is the player; K1 = `k1`, K2 = `k2`
/// (aip8 columns); `Skill` row 1 has charclass 0 (the owner's class) and the
/// given mana fields.
fn shadow(k1: i16, k2: i16, mana: (u16, u16, u16)) -> World {
    let mut row = act_row(105, &[40, 30, 60, 1]);
    row.aip8_n = k1 as u16;
    row.aip8_h = k2 as u16;
    let mut w = world(row);
    own(&mut w);
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.mana = mana.0;
    sk.lvlmana = mana.1;
    sk.manashift = mana.2;
    w.skills = vec![sk.clone(), sk.clone(), sk];
    w.fake.x.skill_level.insert(1, 1);
    w
}

fn usable(w: &mut World, skill: i32, c: bool) -> bool {
    let (u, o, t) = (w.mon, w.player, w.player);
    let p = TickParam {
        target: Some(t),
        distance: 5,
        combat: c,
        class: 0,
        class2: 0,
    };
    w.with(|g, cx| shadow_usable(g, cx, u, &p, o, skill, Some(t), c))
}

#[test]
fn shadow_usable_is_refused_when_the_skill_is_not_allowed() {
    // A summon skill whose id is the unit's own class fails the pet test:
    // refused with no draw.
    let mut w = shadow(1, 64, (0, 0, 0));
    w.skills[1].summon = 999;
    w.fake.class.insert(w.mon, 999);
    w.seed(7);
    assert!(!usable(&mut w, 1, false));
    assert_eq!(steps_since(&w, 7), 0);
    // Not a summon: passes the pet test and goes on to the draws.
    let mut w = shadow(1, 64, (0, 0, 0));
    w.seed(7);
    let _ = usable(&mut w, 1, false);
    assert!(steps_since(&w, 7) >= 1);
    // Also refused: the close-range type needs C, the others need no C.
    let mut w = shadow(1, 64, (0, 0, 0));
    w.skills[1].aitype = 4;
    w.seed(7);
    assert!(!usable(&mut w, 1, false));
    assert_eq!(steps_since(&w, 7), 0);
}

#[test]
fn shadow_usable_mana_cost_gates_the_skill() {
    // c = ((lvlmana × (lvl − 1) + mana) << manashift) >> 8, at least 0;
    // `roll(100)` > 100 − 160 c / 100 → refused. mana 10, shift 8: c = 10,
    // threshold 84.
    let mut got = Vec::new();
    for lo in [seed_with(1, |v| v[0] == 84), seed_with(1, |v| v[0] == 85)] {
        let mut w = shadow(1, 64, (10, 0, 8));
        w.seed(lo);
        // m below lo → m := 1: a = roll(1) = 0, b ≥ 0: passes if reached.
        got.push(usable(&mut w, 1, false));
    }
    assert_eq!(got, [true, false]);
    // A negative total clamps to 0: no refusal even for a draw of 99.
    let mut w = shadow(1, 64, (0, 0, 0));
    w.skills[1].lvlmana = 0xFFFF; // −1 per level above 1
    w.fake.x.skill_level.insert(1, 5);
    w.seed(seed_with(1, |v| v[0] == 99));
    assert!(usable(&mut w, 1, false));
    // lvlmana × (lvl − 1) counts: lvlmana 5, level 3, mana 0, shift 8 → 10.
    let mut w = shadow(1, 64, (0, 5, 8));
    w.fake.x.skill_level.insert(1, 3);
    w.seed(seed_with(1, |v| v[0] == 85));
    assert!(!usable(&mut w, 1, false));
}

#[test]
fn shadow_usable_waits_for_the_next_skill_frame() {
    let mut w = shadow(1, 64, (0, 0, 0));
    set_param_of(&mut w, 0, 100);
    w.game.frame = 99;
    w.seed(1);
    assert!(!usable(&mut w, 1, false));
    // The mana draw was made, the pool draws were not.
    assert_eq!(steps_since(&w, 1), 1);
    w.game.frame = 100;
    w.seed(1);
    assert!(usable(&mut w, 1, false));
}

#[test]
fn shadow_usable_mana_pool_bounds_from_k1_and_k2() {
    // lo := K1 clamped 1..128, hi := K2 clamped 1..256; m < lo or m > 32 hi
    // → m := lo (written back).
    for (k1, k2, m, want) in [
        (5, 64, 3, 5),         // below lo
        (5, 64, 100, 100),     // inside
        (5, 2, 65, 5),         // above 32 × 2
        (5, 2, 64, 64),        // exactly 32 hi: kept
        (500, 64, 3, 128),     // K1 clamped to 128
        (0, 64, 0, 1),         // K1 clamped to 1
        (5, 9999, 8191, 8191), // K2 clamped to 256: 32 × 256 = 8192
        (5, 9999, 8193, 5),
    ] {
        let mut w = shadow(k1, k2, (0, 0, 0));
        set_param_of(&mut w, 1, m);
        w.seed(seed_with(1, |_| true));
        let _ = usable(&mut w, 1, false);
        // After the call m is `want`, plus the step-8 increase (0: c = 0).
        assert_eq!(param_of(&w, 1), want, "K1 {k1} K2 {k2} m {m}");
    }
}

#[test]
fn shadow_usable_second_roll_must_not_be_below_the_first() {
    // a := roll(m), b := roll(100); b < a → refused. Replay the draws.
    for lo in [1u32, 12345, 3_735_928_559, 4_014_346_870] {
        let m = 50;
        let mut s = Seed::init_low(lo);
        s.roll(100); // the mana-cost roll (cost 0: never refuses)
        let a = s.roll(m);
        let b = s.roll(100);
        let mut w = shadow(1, 64, (0, 0, 0));
        set_param_of(&mut w, 1, m);
        w.seed(lo);
        assert_eq!(usable(&mut w, 1, false), b >= a, "seed {lo}: a {a} b {b}");
    }
    // Both a and b are drawn on every pass: three draws in all.
    let mut w = shadow(1, 64, (0, 0, 0));
    set_param_of(&mut w, 1, 50);
    w.seed(1);
    let _ = usable(&mut w, 1, false);
    assert!(steps_since(&w, 1) >= 3);
}

#[test]
fn shadow_usable_raises_the_pool_by_the_mana_cost_over_lambda() {
    // m += (320 − λ) × c / (λ + 100), signed: c = 10, λ = 1: 319 × 10 / 101
    // = 31; λ = 220: 100 × 10 / 320 = 3.
    for (lambda, add) in [(1, 31), (220, 3), (400, -80 * 10 / 500)] {
        let mut w = shadow(1, 64, (10, 0, 8));
        set_param_of(&mut w, 1, 40);
        set_param_of(&mut w, 2, lambda);
        // The cost roll must pass (≤ 84) and b ≥ a: a = roll(40) = 0 when
        // the draw is a multiple; pick the first seed that passes.
        let lo = (1u32..)
            .find(|&lo| {
                let mut s = Seed::init_low(lo);
                let r = s.roll(100);
                let a = s.roll(40);
                let b = s.roll(100);
                r <= 84 && b >= a
            })
            .unwrap();
        w.seed(lo);
        assert!(usable(&mut w, 1, false));
        assert_eq!(param_of(&w, 1), 40 + add, "λ {lambda}");
    }
}

#[test]
fn shadow_warrior_drops_a_far_target() {
    // T := 0 when D > aip1 [40] or the distance to the owner > aip2 [30]:
    // then the pet follow runs (loiter, idle 15) and ends the think before
    // any skill. With T kept (D = 40, dO = 30) the skills run: the attack.
    let setup = |owner_at: (i32, i32)| {
        let mut w = shadow(5, 64, (0, 0, 0));
        w.fake.x.hand.insert((w.player, true), (1, 9));
        w.fake.x.hand.insert((w.player, false), (1, 9));
        w.skills[1].charclass = 0;
        w.fake.x.skill_level.insert(0, 1);
        w.fake.pos.insert(w.player, owner_at);
        w.fake.y.final_point.insert(w.player, owner_at);
        w.fake.y.target_point.insert(w.player, owner_at);
        w.fake.y.last_placed = (300, 300);
        let t = w.add_unit(UnitType::Monster, (120, 100));
        (w, t)
    };
    let attacks = |w: &World, t: UnitId| {
        w.fake
            .modes()
            .iter()
            .any(|m| m == &unit_mode(mode::ATTACK1, t))
    };
    let lo = seed_with(1, |v| v[0] >= 10);
    let run = |owner_at: (i32, i32), d: i32| {
        let (mut w, t) = setup(owner_at);
        w.seed(lo);
        w.think_with(Some(t), d, true);
        let a = attacks(&w, t);
        (a, w.thinks())
    };
    let (kept, _) = run((130, 100), 40);
    assert!(kept, "D = 40 and dO = 30 keep T");
    let (far_d, thinks) = run((130, 100), 41);
    assert!(!far_d, "D > aip1 drops T");
    let _ = thinks;
    let (far_o, thinks) = run((131, 100), 40);
    assert!(!far_o, "dO > aip2 drops T");
    let _ = thinks;
}

// ---- Raven, Vines, DruidBear, ------------------------------

/// A druidhawk (107) owned by the player, brackets [10, 6, 5, 75, 35], hits
/// left c = 2, next attack frame 1000, orbit side σ = 1; owner at `at`.
fn raven_world(at: (i32, i32)) -> World {
    let mut w = world(act_row(107, &[10, 6, 5, 75, 35]));
    own(&mut w);
    w.fake.pos.insert(w.player, at);
    w.fake.y.final_point.insert(w.player, at);
    w.fake.y.target_point.insert(w.player, at);
    w.fake.y.last_placed = (300, 300);
    set_param_of(&mut w, 0, 2);
    set_param_of(&mut w, 1, 1000);
    set_param_of(&mut w, 2, 1);
    w
}

#[test]
fn raven_runs_to_a_far_owner_at_the_run_ratio() {
    // v := Run × 100 / Velocity − 100; 100 when Velocity ≤ 0 or v ≥ 100
    // (negative kept). dO > 28 → pet move k 0 (run 0, speed v, n 0).
    for (run, vel, v) in [(9, 6, 50), (3, 6, -50), (12, 6, 100), (9, 0, 100)] {
        let mut w = raven_world((130, 100));
        w.monstats[0].run = run;
        w.monstats[0].velocity = vel;
        w.think_with(Some(w.player), 5, false);
        assert_eq!(
            w.fake.modes(),
            [point_mode(mode::WALK, 130, 108)],
            "run {run}"
        );
        assert_eq!(w.vel_request().speed, v, "run {run} velocity {vel}");
    }
    // dO = 28 is not far: no pet move k 0 to Q.
    let mut w = raven_world((129, 100));
    w.think_with(Some(w.player), 5, false);
    assert!(!w.fake.modes().contains(&point_mode(mode::WALK, 129, 108)));
}

#[test]
fn raven_looks_at_the_owners_target_without_using_it() {
    // The owner-view search runs (its result is computed and unused): the
    // good-target search is asked for O, and the think goes on as if it had
    // found nothing.
    let mut w = raven_world((120, 100));
    w.fake.align = 1;
    let e = w.add_unit(UnitType::Monster, (115, 100));
    w.fake.good_for.insert(w.player, (e, 5));
    w.think_with(None, 0, false);
    assert!(w.fake.log.iter().any(|l| l.starts_with("good")));
    assert!(!w.fake.modes().contains(&unit_mode(mode::WALK, e)));
    assert!(!w.fake.modes().contains(&unit_mode(mode::ATTACK1, e)));
}

#[test]
fn raven_walks_to_the_mid_radius_of_its_owner_or_retraces() {
    // mid := (aip2 + aip1) / 2 and the point O + (U − O) × mid / d: raven
    // at (100, 100), O 24 away (outside the orbit band).
    for (aips, mid) in [([10i16, 6, 5, 75, 35], 8), ([20, 2, 5, 75, 35], 11)] {
        let mut w = world(act_row(107, &aips));
        own(&mut w);
        let at = (124, 100);
        w.fake.pos.insert(w.player, at);
        w.fake.y.final_point.insert(w.player, at);
        w.fake.y.target_point.insert(w.player, at);
        w.fake.y.last_placed = (300, 300);
        set_param_of(&mut w, 0, 2);
        set_param_of(&mut w, 1, 1000);
        w.think_with(Some(w.player), 5, false);
        assert_eq!(
            w.fake.modes(),
            [point_mode(mode::WALK, 124 - mid, 100)],
            "aips {aips:?}"
        );
    }
    // d = 0 (on top of the owner): not started → pet move k 1 (velocity
    // (15, 0, 0), a wander' of radius 4 after the history draw).
    let lo = seed_with(1, |_| true);
    let mut w = raven_world((100, 100));
    w.seed(lo);
    w.think_with(Some(w.player), 5, false);
    let mut s = Seed::init_low(lo);
    s.step();
    let (x, y) = wander_point(&mut s, (100, 100), 4);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, x, y)]);
    assert_eq!(w.vel_request().method, 15);
}

#[test]
fn raven_attacks_or_chases_its_target_when_due() {
    let due = |lo: u32, d: i32, c: bool, f: i32| {
        let mut w = raven_world((103, 100));
        set_param_of(&mut w, 1, f);
        w.game.frame = 10;
        w.seed(lo);
        w.think_with(Some(w.player), d, c);
        w
    };
    let hit = seed_with(1, |v| v[0] < 75);
    let miss = seed_with(1, |v| v[0] >= 75);
    // C: A1 at T, c −= 1, f := frame + aip3 × 10.
    let w = due(hit, 5, true, 5);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, 60));
    // Not C: walk to T (flags 0), counters untouched.
    let w = due(hit, 5, false, 5);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (2, 5));
    // f ≥ frame: not due, no draw; roll ≥ aip4: no attack (one draw); D ≥
    // aip5 [35]: the roll is drawn and no attack.
    let w = due(hit, 5, true, 10);
    assert!(!w.fake.modes().contains(&unit_mode(mode::ATTACK1, w.player)));
    assert_eq!(steps_since(&w, hit), 0);
    let w = due(miss, 5, true, 5);
    assert!(!w.fake.modes().contains(&unit_mode(mode::ATTACK1, w.player)));
    assert_eq!(steps_since(&w, miss), 1);
    let w = due(hit, 35, true, 5);
    assert!(!w.fake.modes().contains(&unit_mode(mode::ATTACK1, w.player)));
    assert_eq!(steps_since(&w, hit), 1);
}

#[test]
fn raven_orbits_its_owner_inside_the_band_and_flips_sides_when_refused() {
    // Band aip2 [6] ≤ dO ≤ aip1 [10]: velocity (σ ? 5 : 6, 0, 4) and mode 2
    // toward O.
    for (sigma, method) in [(1, 5), (0, 6)] {
        let mut w = raven_world((108, 100));
        set_param_of(&mut w, 2, sigma);
        w.think_with(Some(w.player), 40, false);
        assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
        assert_eq!((w.vel_request().method, w.vel_request().steps), (method, 4));
        assert_eq!(param_of(&w, 2), sigma);
    }
    // Refused: σ := ¬σ and one more try with the new side.
    let mut w = raven_world((108, 100));
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 40, false);
    let m = w.fake.modes();
    let orbit = unit_mode(mode::WALK, w.player);
    assert_eq!(m[..2], [orbit.clone(), orbit]);
    assert_eq!(param_of(&w, 2), 0);
    // Outside the band (dO = 11 > aip1): no orbit request toward O at all.
    let mut w = raven_world((112, 100));
    w.think_with(Some(w.player), 40, false);
    assert!(!w.fake.modes().contains(&unit_mode(mode::WALK, w.player)));
}

/// A plaguepoppy (110), brackets [100, 20, 25, 10, 35], owned by the player.
fn vines_world(owner_at: (i32, i32)) -> World {
    let mut w = world(act_row(110, &[100, 20, 25, 10, 35]));
    own(&mut w);
    w.fake.pos.insert(w.player, owner_at);
    w.fake.y.final_point.insert(w.player, owner_at);
    w.fake.y.target_point.insert(w.player, owner_at);
    w.fake.y.last_placed = (300, 300);
    w
}

#[test]
fn vines_far_from_the_owner_teleport_to_it() {
    let mut w = vines_world((136, 100));
    let room = w.room;
    w.fake.y.free_spot = Some((110, 110));
    w.fake.x.room_at = Some(room);
    w.fake.x.place_ok = true;
    w.think_with(None, 0, false);
    assert!(logged(&w, "place 110 110"));
    assert_eq!(w.thinks(), [5]);
    // 34 < aip5: no catch-up teleport.
    let mut w = vines_world((134, 100));
    w.fake.y.free_spot = Some((110, 110));
    w.fake.x.room_at = Some(room);
    w.fake.x.place_ok = true;
    w.think_with(None, 0, false);
    assert!(!logged(&w, "place 110 110"));
}

#[test]
fn vines_in_town_follow_or_idle() {
    // Far (> 50): the follow's k 3 fails (no free spot) → idle aip3 [25].
    let mut w = vines_world((170, 100));
    let room = w.room;
    w.fake.town.insert(room);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [25]);
    // Near: the follow loiters (idle 15) and ends the think.
    let mut w = vines_world((103, 100));
    w.fake.town.insert(room);
    w.seed(seed_with(1, |v| v[0] >= 10));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn vines_forget_a_far_secondary_target_then_follow() {
    // E ≥ aip2 [20] → S := 0: the follow (no S, O beside it) loiters and
    // ends the think; with E = 19 the target stays, the follow returns
    // 0 (S set, D ≤ 80) and the poisoned target is escaped (step 6).
    let setup = |e: i32| {
        let mut w = vines_world((103, 100));
        let s = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.secondary = Some((s, e));
        w.fake.states.insert((s, 2));
        w.seed(seed_with(1, |v| v[0] >= 10));
        w.think_with(None, 0, false);
        w
    };
    let w = setup(19);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 90, 100)]);
    let w = setup(20);
    assert_eq!(w.thinks(), [15]);
    assert!(w.fake.modes().is_empty());
}

/// A druidbear (112), brackets [15, 40, 50], owned by the player; good
/// alignment.
fn bear_world(owner_at: (i32, i32)) -> World {
    let mut w = world(act_row(112, &[15, 40, 50]));
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, owner_at);
    w.fake.y.final_point.insert(w.player, owner_at);
    w.fake.y.target_point.insert(w.player, owner_at);
    w.fake.y.last_placed = (300, 300);
    w
}

#[test]
fn druid_bear_remembers_its_owner() {
    let mut w = bear_world((103, 100));
    w.think_with(None, 0, false);
    let g = w.game.lists.unit(w.player).unwrap().guid as i32;
    assert_eq!(param_of(&w, 2), g);
}

#[test]
fn druid_bear_catches_up_with_a_walking_or_running_owner() {
    // dO > 18 (≤ 28): O walking (2) or town-walking (6) → k 0 (0, 0, 0); O
    // running (3) → k 0 (0, 100, 0). Otherwise the think goes on.
    for (om, speed) in [(2u8, 0), (6, 0), (3, 100)] {
        let mut w = bear_world((122, 100));
        w.fake.anim.insert(w.player, om);
        w.think_with(None, 0, false);
        assert_eq!(
            w.fake.modes(),
            [point_mode(mode::WALK, 122, 108)],
            "O mode {om}"
        );
        assert_eq!(w.vel_request().speed, speed, "O mode {om}");
    }
}

#[test]
fn druid_bear_target_from_its_own_search_or_the_owner_view() {
    let lo = seed_with(1, |v| v[0] < 40);
    let walks_to = |w: &World, t: UnitId| w.fake.modes().contains(&unit_mode(mode::WALK, t));
    // S from its own capped search (28): E = 28 kept, 29 dropped.
    for (e, kept) in [(28, true), (29, false)] {
        let mut w = bear_world((103, 100));
        let t = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.good_for.insert(w.mon, (t, e));
        w.seed(lo);
        w.think_with(None, 0, false);
        assert_eq!(walks_to(&w, t), kept, "E {e}");
    }
    // S not directly reachable → 0 (and T0 equally unreachable).
    let mut w = bear_world((103, 100));
    let t = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.good_for.insert(w.mon, (t, 10));
    w.fake.reach_fails = true;
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(!walks_to(&w, t));
    // S = 0: T0 (the owner's search) within 28 and reachable → S := T0.
    for (tx, reach_fails, taken) in [(115, false, true), (115, true, false), (130, false, false)] {
        let mut w = bear_world((103, 100));
        let t = w.add_unit(UnitType::Monster, (tx, 100));
        w.fake.good_for.insert(w.player, (t, 10));
        w.fake.reach_fails = reach_fails;
        w.seed(lo);
        w.think_with(None, 0, false);
        assert_eq!(
            walks_to(&w, t),
            taken,
            "T0 at {tx} reach_fails {reach_fails}"
        );
    }
}

#[test]
fn druid_bear_closes_on_a_target_out_of_contact() {
    // M = 0, S: `roll(100)` < aip2 [40] → velocity (0, v, 40) and walk to S;
    // a draw ≥ 40 → no walk (step 9: dO < 17 → idle 15).
    let setup = |lo: u32| {
        let mut w = bear_world((103, 100));
        w.monstats[0].run = 9;
        w.monstats[0].velocity = 6;
        let t = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.good_for.insert(w.mon, (t, 10));
        w.seed(lo);
        w.think_with(None, 0, false);
        (w, t)
    };
    let (w, t) = setup(seed_with(1, |v| v[0] < 40));
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, t)]);
    assert_eq!((w.vel_request().speed, w.vel_request().steps), (50, 40));
    let (w, _) = setup(seed_with(1, |v| v[0] >= 40));
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn druid_bear_in_contact_smites_or_strikes() {
    // M ≠ 0, S: `roll(100)` < aip3 [50] → sequence skill `Skill1` at S;
    // else A1 at S and wait aip1 [15].
    let setup = |lo: u32| {
        let mut w = bear_world((103, 100));
        give_skill(&mut w, 1, 77, 14);
        let t = w.add_unit(UnitType::Monster, (105, 100));
        w.fake.good_for.insert(w.mon, (t, 5));
        w.fake.melee.insert(t);
        w.seed(lo);
        w.think_with(None, 0, false);
        (w, t)
    };
    let (w, t) = setup(seed_with(1, |v| v[0] < 50));
    assert_eq!(w.fake.modes(), [unit_mode(mode::SEQUENCE, t)]);
    assert!(logged(&w, "skill 77"));
    let (w, t) = setup(seed_with(1, |v| v[0] >= 50));
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, t)]);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn generic_spawner_far_target_idles_20() {
    // D ≥ 21 → idle 20 even when the nest is due and under its cap; D = 20
    // goes on to the nest.
    let hut = [80, 0, 15];
    for (d, nests) in [(21, false), (20, true)] {
        let mut w = world(act_row(129, &hut));
        w.fake.footprint = true;
        w.game.frame = 80;
        w.think_with(Some(w.player), d, false);
        assert_eq!(w.fake.modes().len(), usize::from(nests), "D {d}");
        if !nests {
            assert_eq!(w.thinks(), [100]);
            assert_eq!(param_of(&w, 1), 0);
        }
    }
}
