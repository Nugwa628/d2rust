use super::act2::{act_row, give_skill, logged, param_of, point_mode, seeded, set_param_of, world};
use super::act6::{grow, own};
use super::npc::{seed_with, steps_since, unit_mode};
use super::*;

/// A melee NecroPet owned by the player: good alignment so the searches
/// use `good_target_search`, owner 3 tiles east.
fn pet() -> World {
    let mut w = world(act_row(67, &[]));
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, (103, 100));
    w
}

/// The wander point of `wander'` after `skip` draws of `lo` around the
/// pet's own position (100, 100), radius 4.
fn wander4(lo: u32, skip: usize) -> String {
    let mut s = Seed::init_low(lo);
    for _ in 0..skip {
        s.step();
    }
    let (x, y) = wander_point(&mut s, (100, 100), 4);
    point_mode(mode::WALK, x, y)
}

#[test]
fn necro_melee_owner_view_replaces_a_far_search_target_and_lunges() {
    // E = 10 > 6 (search distance): S := 0, T0 = the enemy within 36 →
    // S := T0 with M as the search wrote it (0): one draw (the follow's r),
    // no follow (S set, D ≤ 80), no wander, lunge: velocity (0, 0, 12)
    // and a walk at S.
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |v| v[0] >= 15);
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, (103, 100));
    let e = w.add_unit(UnitType::Monster, (112, 100));
    w.fake.good = Some((e, 10));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, e)]);
    assert_eq!(w.vel_request().steps, 12);
    assert_eq!(steps_since(&w, lo), 1);
    // E = 50: beyond the cap (24) → no search target; T0 is the same
    // enemy but 49 away (≥ 36) → S stays 0 → the quiet follow (r < 15)
    // returns 0 → wander' 4.
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |v| v[0] < 15);
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, (103, 100));
    let e = w.add_unit(UnitType::Monster, (150, 100));
    w.fake.good = Some((e, 50));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
}

#[test]
fn necro_melee_without_direct_reach_drops_the_target() {
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |v| v[0] < 15);
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, (103, 100));
    let e = w.add_unit(UnitType::Monster, (104, 100));
    w.fake.good = Some((e, 4));
    w.fake.reach_fails = true;
    w.think_with(None, 0, false);
    // S := 0: wander', not a lunge at the enemy.
    assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
    // Reachable: the lunge.
    let mut w = pet();
    w.seed(lo);
    let e = w.add_unit(UnitType::Monster, (104, 100));
    w.fake.good = Some((e, 4));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, e)]);
}

#[test]
fn necro_melee_wanders_without_a_target_or_in_town() {
    // No S: wander' 4.
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |v| v[0] < 15);
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, (103, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
    // S set but the unit's room is a town: wander' 4 as well.
    let mut w = pet();
    w.seed(lo);
    w.fake.town.insert(w.room);
    let e = w.add_unit(UnitType::Monster, (104, 100));
    w.fake.good = Some((e, 4));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
}

#[test]
fn necro_melee_in_contact_attacks_or_idles() {
    // M ≠ 0 (the enemy is in melee range): roll(100) < 80 → A1 at S, else
    // idle 10. Draws: the follow's r, then the roll.
    for (hit, want_attack) in [(true, true), (false, false)] {
        let (mut w, lo) = seeded(act_row(67, &[]), 2, |v| (v[1] < 80) == hit);
        own(&mut w);
        w.fake.align = 1;
        w.fake.pos.insert(w.player, (103, 100));
        let e = w.add_unit(UnitType::Monster, (104, 100));
        w.fake.good = Some((e, 4));
        w.fake.melee.insert(e);
        w.think_with(None, 0, false);
        if want_attack {
            assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, e)]);
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [10]);
        }
        assert_eq!(steps_since(&w, lo), 2);
    }
}

#[test]
fn necro_follow_is_quiet_below_15_with_n_8_else_loud_with_n_7() {
    // Owner 9 east: D = 8. r < 15: quiet, n = 8 → R = 8, D ≤ R → k 2 →
    // quiet returns 0 → wander'. r ≥ 15: not quiet, n = 7 → R = 7 < D →
    // k 1, never the k 2 loiter (idle 15 after a draw ≥ 10).
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |v| v[0] < 15);
    own(&mut w);
    w.fake.align = 1;
    w.fake.pos.insert(w.player, (109, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
    let lo = seed_with(2, |v| v[0] >= 15 && v[1] >= 10);
    let mut w = pet();
    w.fake.pos.insert(w.player, (109, 100));
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_ne!(w.thinks(), [15], "k 1 (n = 7), not the k 2 loiter (n = 8)");
    // With n = 8 and r ≥ 15 the same world would have loitered: R = 8.
    let mut w = pet();
    w.fake.pos.insert(w.player, (108, 100));
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15], "D = 7 ≤ R = 7: k 2, loud: loiter idle 15");
}

#[test]
fn necro_ranged_owner_view_drops_pet_ignore_targets_and_wanders_without_s() {
    // Ranged (param 0 = 1). No secondary target: S := T0 when T0 is in
    // line and < 20 away; with unit flag 0x40000000 (petIgnore) T0 := 0.
    let lo = seed_with(2, |v| v[0] < 15 && v[1] < 80);
    for (flag, shoots) in [(0u32, true), (0x4000_0000, false)] {
        let mut w = pet();
        w.seed(lo);
        give_skill(&mut w, 1, 50, 10);
        set_param_of(&mut w, 0, 1);
        let e = w.add_unit(UnitType::Monster, (112, 100));
        w.fake.good = Some((e, 10));
        w.fake.x.flags.insert(e, flag);
        w.think_with(None, 0, false);
        if shoots {
            assert_eq!(w.fake.modes(), [unit_mode(10, e)], "flag {flag:#x}");
        } else {
            // No S: wander' 4 after the follow's draw.
            assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
        }
    }
    // S set but the room is a town: wander' 4 (the same draws).
    let mut w = pet();
    w.seed(lo);
    give_skill(&mut w, 1, 50, 10);
    set_param_of(&mut w, 0, 1);
    w.fake.town.insert(w.room);
    let e = w.add_unit(UnitType::Monster, (104, 100));
    w.fake.secondary = Some((e, 4));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander4(lo, 1)]);
    let _ = (
        param_of(&w, 0),
        logged(&w, ""),
        grow as fn(&mut World, usize),
    );
}

// ---- Towner, Hireable -------------------------------------------------

use super::act3::seed_raw;
use super::act6::own as own_player;

/// A hireling of class `cls` (monstats `aip1` = `aip1`) owned by the
/// player, no target, owner at (103, 100).
fn hireling(cls: i32, aip1: i16) -> World {
    let mut w = world(act_row(61, &[aip1]));
    grow(&mut w, 600);
    let mon = w.mon;
    w.fake.class.insert(mon, cls);
    own_player(&mut w);
    w.fake.pos.insert(w.player, (103, 100));
    w
}

fn wander_at(lo: u32, skip: usize, center: (i32, i32), n: i32) -> String {
    let mut s = Seed::init_low(lo);
    for _ in 0..skip {
        s.step();
    }
    let (x, y) = wander_point(&mut s, center, n);
    point_mode(mode::WALK, x, y)
}

#[test]
fn hireling_collision_wander_threshold_by_class_and_the_late_wander() {
    // B (the unit's cell collides with mask 0x40): `lo' & 127` < 12 for the
    // ranged hirelings (q = 1), < 6 for 338 / 560 / 561 (q = 0) → wander 5.
    // A draw of 8 wanders the first, not the second.
    let lo = seed_raw(1, |v| v[0] & 127 == 8);
    for (cls, aip1, wanders) in [(271, 0, true), (338, 1, false), (560, 1, false)] {
        let mut w = hireling(cls, aip1);
        w.fake.x.pattern_collides = true;
        w.seed(lo);
        // A secondary target in range, so a non-wander goes on to the
        // hireling attack (idle 10: no pet node).
        let s = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.secondary = Some((s, 9));
        w.think_with(None, 0, false);
        if wanders {
            assert_eq!(
                w.fake.modes(),
                [wander_at(lo, 1, (100, 100), 5)],
                "cls {cls}"
            );
        } else {
            assert!(w.fake.modes().is_empty(), "cls {cls}");
            assert_eq!(w.thinks(), [10], "cls {cls}");
        }
    }
    // A draw of 3 wanders every class.
    let lo3 = seed_raw(1, |v| v[0] & 127 == 3);
    let mut w = hireling(338, 1);
    w.fake.x.pattern_collides = true;
    w.seed(lo3);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander_at(lo3, 1, (100, 100), 5)]);
    // Step 9: B with no target and a draw above the threshold still wanders
    // 5 (after the secondary search found nothing); without B it does not.
    let mut w = hireling(338, 1);
    w.fake.x.pattern_collides = true;
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander_at(lo, 1, (100, 100), 5)]);
    let mut w = hireling(338, 1);
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(w.fake.modes().is_empty(), "no B: no wander (idle 5)");
    assert_eq!(w.thinks(), [5]);
}

#[test]
fn hireling_in_another_coordinate_area_catches_up_with_move_0() {
    let mut w = hireling(338, 1);
    w.seed(1);
    w.fake.y.coord.insert((103, 100), 7);
    // Q must lie in O's coordinate area to be tried.
    w.fake.y.coord.insert((130, 108), 7);
    w.fake.y.final_point.insert(w.player, (130, 100));
    w.think_with(None, 0, false);
    // Hireling move k 0 (1, 60, 0): each try is velocity (0, 60, 40) and
    // a walk to Q = F + 8 × direction (the run flag is ignored).
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 130, 108)]);
    assert_eq!((w.vel_request().speed, w.vel_request().steps), (60, 40));
}

#[test]
fn hireling_next_to_its_owner_steps_aside() {
    // D2 ≤ 1: h := (5 or p >> 1) − 1 as a byte; velocity (7, 0, 0);
    // wander near O by h.
    for (p, h) in [(0, 4), (18, 8)] {
        let mut w = hireling(338, 1);
        w.seed(1);
        set_param_of(&mut w, 0, p);
        w.fake.pos.insert(w.player, (101, 100));
        w.think_with(None, 0, false);
        assert_eq!(w.fake.modes(), [wander_at(1, 0, (101, 100), h)], "p {p}");
        assert_eq!(w.vel_request().method, 7);
    }
    // Wander fails: delete thinks, escape from O by h fails, walk to F
    // with velocity (0, 0, 40).
    let mut w = hireling(338, 1);
    w.seed(1);
    w.fake.walk_fails = true;
    w.fake.pos.insert(w.player, (101, 100));
    w.fake.y.final_point.insert(w.player, (120, 90));
    w.think_with(None, 0, false);
    assert_eq!(
        w.fake.modes().last().unwrap(),
        &point_mode(mode::WALK, 120, 90)
    );
    assert_eq!(w.vel_request().steps, 40);
    assert!(w.thinks().is_empty(), "thinks deleted");
}

#[test]
fn hireling_attack_chance_by_class_and_frustration() {
    // ok = draw < a; a = 98 for 338 / 560 / 561, else p + 40 + 2 × level,
    // at most 95. ok → p := 0; a miss → p += 10.
    // (cls, aip1, p, level, a)
    for (cls, aip1, p, level, a) in [
        (338, 1, 0, 20, 98),
        (561, 1, 30, 1, 98),
        (271, 0, 0, 20, 80),
        (271, 0, 10, 20, 90),
        (359, 0, 40, 20, 95),
    ] {
        for (draw, ok) in [(a - 1, true), (a, false)] {
            let mut w = hireling(cls, aip1);
            w.seed(seed_with(1, |v| v[0] == draw as u32));
            set_param_of(&mut w, 0, p);
            w.fake.stats.insert((w.mon, 12), level);
            // S out of reach of the ranged wander (d ≥ 4), in melee range.
            let s = w.add_unit(UnitType::Monster, (110, 100));
            w.fake.secondary = Some((s, 9));
            w.fake.melee.insert(s);
            w.fake.y.hire_id = Some(1);
            w.fake.y.hire_row = Some(HireRow::default());
            w.think_with(None, 0, false);
            let expect = if ok { 0 } else { p + 10 };
            assert_eq!(param_of(&w, 0), expect, "cls {cls} p {p} draw {draw}");
        }
    }
}

#[test]
fn hireling_melee_attacks_only_within_distance_3() {
    // d from the full-size distance U→S: 2 and in range → hireling skill
    // (A1 fallback); 3 → run to S with flags 1.
    for (sx, runs) in [(103, false), (104, true)] {
        let mut w = hireling(338, 1);
        w.seed(seed_with(1, |v| v[0] < 98));
        let s = w.add_unit(UnitType::Monster, (sx, 100));
        w.fake.secondary = Some((s, 5));
        w.fake.melee.insert(s);
        w.fake.y.hire_id = Some(1);
        w.fake.y.hire_row = Some(HireRow::default());
        w.think_with(None, 0, false);
        let want = if runs { mode::RUN } else { mode::ATTACK1 };
        assert_eq!(w.fake.modes(), [unit_mode(want, s)], "S at x {sx}");
    }
}

#[test]
fn hireling_ranged_attack_close_in_wanders_or_escapes() {
    let ranged = |w: &mut World| {
        give_skill(w, 1, 50, 10);
        w.fake.y.hire_id = Some(1);
        w.fake.y.hire_row = Some(HireRow::default());
    };
    // d < 4 and the second draw < 50 → wander near O 4, started → end.
    let lo = seed_raw(2, |v| v[0] % 100 < 60 && v[1] % 100 < 50);
    let mut w = hireling(271, 0);
    ranged(&mut w);
    w.seed(lo);
    w.fake.stats.insert((w.mon, 12), 20);
    let s = w.add_unit(UnitType::Monster, (102, 100));
    w.fake.secondary = Some((s, 2));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander_at(lo, 2, (103, 100), 4)]);
    // The wander fails: delete thinks, escape from S by 4 (started) → end.
    let mut w = hireling(271, 0);
    ranged(&mut w);
    w.seed(lo);
    w.fake.stats.insert((w.mon, 12), 20);
    let s = w.add_unit(UnitType::Monster, (102, 100));
    w.fake.secondary = Some((s, 2));
    w.fake.fail_points.insert((0, 0));
    let first = wander_at(lo, 2, (103, 100), 4);
    // Make exactly the wander point fail.
    let pt = first
        .trim_start_matches("mode 2 Point(")
        .trim_end_matches(')')
        .split(", ")
        .map(|v| v.parse::<i32>().unwrap())
        .collect::<Vec<_>>();
    w.fake.fail_points.insert((pt[0], pt[1]));
    w.think_with(None, 0, false);
    let modes = w.fake.modes();
    assert_eq!(modes[0], first);
    assert_eq!(modes.len(), 2, "wander, then the escape from S: {modes:?}");
    assert!(w.thinks().is_empty());
    // d ≥ 4: ok → hireling skill (`Skill1` of roguehire at S); a miss →
    // idle 10 (the ok draw is the first).
    for (ok, want_skill) in [(true, true), (false, false)] {
        let lo = seed_raw(1, |v| (v[0] % 100 < 80) == ok);
        let mut w = hireling(271, 0);
        ranged(&mut w);
        w.seed(lo);
        w.fake.stats.insert((w.mon, 12), 20);
        let s = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.secondary = Some((s, 9));
        w.think_with(None, 0, false);
        if want_skill {
            assert_eq!(w.fake.modes(), [unit_mode(10, s)]);
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [10]);
        }
    }
}

#[test]
fn towner_runs_the_commands_then_the_map_ai_then_idles_12() {
    let home = AiCommand {
        params: [10, 100, 100, 0, 0],
    };
    // Home first: nothing set → the home command is made and the think
    // ends idling 20.
    let mut w = world(act_row(41, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    // Commands second: a wander command (type 5, count 1, distance 3)
    // wanders and ends the think (no idle 12).
    let (mut w, lo) = seeded(act_row(41, &[]), 1, |_| true);
    w.store.control_mut(w.mon).unwrap().commands = vec![
        home,
        AiCommand {
            params: [5, 1, 3, 0, 0],
        },
    ];
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander_at(lo, 0, (100, 100), 3)]);
    assert!(w.thinks().is_empty());
    // Map AI third: a node walk when the draw is < 66, else (nothing
    // handled) idle 12 — Towner's own idle, not Npc's 8.
    let nodes = vec![MapNode {
        action: 1,
        x: 110,
        y: 100,
    }];
    for (draw_lt_66, handled) in [(true, true), (false, false)] {
        let (mut w, _) = seeded(act_row(41, &[]), 1, |v| (v[0] < 66) == draw_lt_66);
        let c = w.store.control_mut(w.mon).unwrap();
        c.commands = vec![home];
        c.map_ai = Some(nodes.clone());
        w.think_with(None, 0, false);
        if handled {
            assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 110, 100)]);
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [12]);
        }
    }
    // Nothing at all: idle 12.
    let mut w = world(act_row(41, &[]));
    w.store.control_mut(w.mon).unwrap().commands = vec![home];
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [12]);
}

// -------------------------------------------------------------------------

#[test]
fn elemental_beast_chases_or_wanders_by_aip1() {
    // s = 1 (awake), no contact: `roll(100)` < aip1 [20] → walk to T with
    // flags 0; else wander 8.
    let lo = seed_with(1, |v| v[0] < 20);
    let mut w = world(act_row(46, &[20, 16, 20]));
    set_param_of(&mut w, 0, 1);
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    let lo = seed_with(1, |v| v[0] >= 20);
    let mut w = world(act_row(46, &[20, 16, 20]));
    set_param_of(&mut w, 0, 1);
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [wander_at(lo, 1, (100, 100), 8)]);
}

#[test]
fn npc_stationary_waits_while_the_player_or_the_npc_is_busy() {
    // P is the nearest interacting player; P busy (= 1) or the
    // NPC's own interaction list non-empty → idle 10 (not 20), the
    // greeting countdown untouched.
    let near = |busy: bool, interacting: bool| {
        let mut w = world(act_row(54, &[]));
        w.fake.nearest = Some((w.player, true));
        if busy {
            w.fake.busy.insert(w.player);
        }
        w.fake.interacting = interacting;
        set_param_of(&mut w, 1, 5);
        w.think_with(None, 0, false);
        w
    };
    for (busy, interacting) in [(true, false), (false, true)] {
        let w = near(busy, interacting);
        assert_eq!(w.thinks(), [10], "busy {busy} interacting {interacting}");
        assert_eq!(param_of(&w, 1), 5);
    }
    // Neither: the countdown (g ≠ 0 → g − 1) and idle 20.
    let w = near(false, false);
    assert_eq!((w.thinks(), param_of(&w, 1)), (vec![20], 4));
}

/// A sentry with an owner outside town, `c` charges and `Skill1` 5 in
/// mode 10.
fn sentry(c: i32) -> World {
    let mut w = world(act_row(101, &[100, 10, 15, 25]));
    give_skill(&mut w, 1, 5, 10);
    own(&mut w);
    set_param_of(&mut w, 1, c);
    w
}

#[test]
fn sentry_without_its_skill_entry_dies_and_clears_state_12() {
    // Charges left (c = 3) so step 1 passes; the unit has no entry of
    // Skill1: death at (0, 0).
    let mut w = sentry(3);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::DEATH, 0, 0)]);
    // With the entry: state 12 is switched off, then the secondary search
    // (none) idles aip3.
    let mut w = sentry(3);
    w.fake.x.skill_entry.insert(5, (5, 10));
    w.fake.states.insert((w.mon, 12));
    w.think_with(None, 0, false);
    assert!(!w.fake.states.contains(&(w.mon, 12)));
    assert_eq!(w.thinks(), [15]);
    assert!(w.fake.modes().is_empty());
}

#[test]
fn tentacle_kills_itself_with_its_dead_owners_target_as_killer() {
    // O in mode 12 and draw < 40: kill with killer O's path target unit.
    let lo = seed_with(1, |v| v[0] < 40);
    let mut w = world(act_row(56, &[70, 5, 16, 12, 20, 12]));
    give_skill(&mut w, 1, 20, 14);
    own(&mut w);
    let killer = w.add_unit(UnitType::Monster, (120, 100));
    w.fake.path_target = Some(killer);
    w.fake.anim.insert(w.player, mode::DEAD);
    w.seed(lo);
    w.think_with(Some(w.player), 5, false);
    assert!(logged(&w, &format!("kill {} Some({})", w.mon.0, killer.0)));
    assert_eq!(steps_since(&w, lo), 1);
    // A draw ≥ 40: the think goes on (s = 0: submerge).
    let lo = seed_with(1, |v| v[0] >= 40);
    let mut w = world(act_row(56, &[70, 5, 16, 12, 20, 12]));
    give_skill(&mut w, 1, 20, 14);
    own(&mut w);
    w.fake.anim.insert(w.player, mode::DEAD);
    w.seed(lo);
    w.think_with(Some(w.player), 5, false);
    assert!(!w.fake.log.iter().any(|l| l.starts_with("kill")));
    assert_eq!(param_of(&w, 2), 1);
}

/// A Totem owned by the player, brackets [20, 30, 30, 20].
fn totem() -> World {
    let mut w = world(act_row(109, &[20, 30, 30, 20]));
    own(&mut w);
    w.fake.pos.insert(w.player, (103, 100));
    w
}

#[test]
fn totem_escapes_a_monster_in_contact() {
    // M ≠ 0 and S ≠ 0 (the evil search finds the player at 3, in melee
    // range): a draw < aip1 [20] with an escape from S by 6 that starts →
    // end (one draw).
    let setup = || {
        let mut w = totem();
        w.fake.nodes = vec![vec![w.player]];
        w.fake.align = 0;
        w.fake.melee.insert(w.player);
        w
    };
    let lo = seed_with(1, |v| v[0] < 20);
    let mut w = setup();
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 94, 100)]);
    assert_eq!(steps_since(&w, lo), 1);
    // A draw ≥ 20: no escape; the think goes on (step 3's draw, ...).
    let lo = seed_with(1, |v| v[0] >= 20);
    let mut w = setup();
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(w
        .fake
        .modes()
        .iter()
        .all(|m| m != &point_mode(mode::WALK, 94, 100)));
    assert!(steps_since(&w, lo) >= 2);
}

#[test]
fn totem_may_forget_its_target_then_follows_or_idles_25() {
    // S = the player at 3 (no contact, so step 2 draws nothing). Step 3's
    // draw < aip2 [30] clears S: the follow (no S, O 2 away, loud k 2)
    // loiters with idle 15 and ends the think. A draw ≥ 30 keeps S:
    // the follow returns 0 (S set, out of town, D ≤ 80) → idle 25.
    let setup = || {
        let mut w = totem();
        w.fake.nodes = vec![vec![w.player]];
        w.fake.align = 0;
        w
    };
    let lo = seed_with(2, |v| v[0] < 30 && v[1] >= 10);
    let mut w = setup();
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
    let lo = seed_with(1, |v| v[0] >= 30);
    let mut w = setup();
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [25]);
    assert_eq!(steps_since(&w, lo), 1, "only step 3's draw");
}

#[test]
fn totem_catches_up_with_a_walking_or_running_owner() {
    // d > aip4 [20], O walking (mode 2): pet move k 0 (0, 0, 0) started →
    // end; O running (mode 3): k 0 with speed 60; O neutral: no move.
    for (om, speed) in [(2u8, 0), (3, 60)] {
        let mut w = totem();
        w.fake.pos.insert(w.player, (124, 100));
        w.fake.y.final_point.insert(w.player, (124, 100));
        w.fake.anim.insert(w.player, om);
        w.seed(1);
        w.think_with(None, 0, false);
        assert_eq!(
            w.fake.modes(),
            [point_mode(mode::WALK, 124, 108)],
            "O mode {om}"
        );
        assert_eq!(w.vel_request().speed, speed, "O mode {om}");
    }
    let mut w = totem();
    w.fake.pos.insert(w.player, (124, 100));
    w.fake.y.final_point.insert(w.player, (124, 100));
    w.seed(1);
    w.think_with(None, 0, false);
    assert!(
        w.fake
            .modes()
            .iter()
            .all(|m| m != &point_mode(mode::WALK, 124, 108)),
        "a neutral owner is not chased by k 0"
    );
}

// ---- DruidWolf, CycleOfLife ------------------------------------------

/// A druid wolf (class 420 spirit wolf, 421 fenris) owned by the player at
/// (103, 100), good alignment; `Skill1` / `Skill2` are Teleport (mode 10).
fn druid(class: i32) -> World {
    let aips: [i16; 5] = if class == 420 {
        [22, 20, 14, 20, 26]
    } else {
        [22, 20, 25, 24, 30]
    };
    let mut w = world(act_row(108, &aips));
    grow(&mut w, 422);
    w.fake.class.insert(w.mon, class);
    give_skill(&mut w, if class == 420 { 1 } else { 2 }, 54, 10);
    own(&mut w);
    w.fake.align = 1;
    put_owner(&mut w, (103, 100));
    w
}

fn put_owner(w: &mut World, at: (i32, i32)) {
    w.fake.pos.insert(w.player, at);
    w.fake.y.final_point.insert(w.player, at);
    w.fake.y.target_point.insert(w.player, at);
    w.fake.y.last_placed = (200, 200);
}

fn has_event0(w: &World, at: i32) -> bool {
    let t = &w.game.timers;
    t.unit_timers(w.mon)
        .into_iter()
        .any(|id| t.event(id).map(|e| e.0) == Some(0) && t.expire(id) == Some(at))
}

#[test]
fn spirit_wolf_search_cap_and_the_owner_view_bug() {
    // r (aip4) = 20 caps the search: E = 20 keeps S, E = 21 drops it. S
    // (reachable, M = 0, reach(O, S) < r) → run to S.
    for (e, runs) in [(20, true), (21, false)] {
        let mut w = druid(420);
        let t = w.add_unit(UnitType::Monster, (108, 100));
        w.fake.good_for.insert(w.mon, (t, e));
        w.seed(seed_with(1, |v| v[0] >= 20));
        w.think_with(None, 0, false);
        assert_eq!(
            w.fake.modes().contains(&unit_mode(mode::RUN, t)),
            runs,
            "E {e}"
        );
    }
    // Step 5, bug kept: S = 0 (nothing within r) and T0 (the owner's search)
    // within r of the wolf: S := T0 only when T0 is NOT directly reachable.
    for (reach_fails, runs) in [(true, true), (false, false)] {
        let mut w = druid(420);
        let t = w.add_unit(UnitType::Monster, (115, 100)); // 14 from the wolf
        w.fake.good_for.insert(w.player, (t, 40));
        w.fake.reach_fails = reach_fails;
        w.seed(seed_with(1, |v| v[0] >= 20));
        w.think_with(None, 0, false);
        assert_eq!(
            w.fake.modes().contains(&unit_mode(mode::RUN, t)),
            runs,
            "reach_fails {reach_fails}"
        );
    }
    // T0 at 29 (≥ r): never taken.
    let mut w = druid(420);
    let t = w.add_unit(UnitType::Monster, (130, 100));
    w.fake.good_for.insert(w.player, (t, 40));
    w.fake.reach_fails = true;
    w.seed(seed_with(1, |v| v[0] >= 20));
    w.think_with(None, 0, false);
    assert!(!w.fake.modes().contains(&unit_mode(mode::RUN, t)));
}

#[test]
fn spirit_wolf_ports_to_a_far_owner() {
    // d > 50 and Skill1 ≥ 0: port to O's position, MODECHANGE at frame + 4,
    // wait 10.
    let mut w = druid(420);
    put_owner(&mut w, (160, 100));
    w.game.frame = 7;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(10, 160, 100)]);
    assert!(has_event0(&w, 11));
    assert_eq!(w.thinks(), [17]);
    // Without the skill: no port.
    let mut w = druid(420);
    w.monstats[0].skill1 = 0xFFFF;
    put_owner(&mut w, (160, 100));
    w.think_with(None, 0, false);
    assert!(!w.fake.modes().contains(&point_mode(10, 160, 100)));
}

#[test]
fn spirit_wolf_catches_up_by_distance_and_owner_mode() {
    // d > aip5 [26]: pet move k 0 (run 1, speed 100) → end (run to Q).
    let mut w = druid(420);
    put_owner(&mut w, (135, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::RUN, 135, 108)]);
    assert_eq!(w.vel_request().speed, 100);
    // aip3 [14] < d ≤ aip5: O walking (2) → k 0 (0, 0, 0); O running (3) →
    // k 0 (1, 100, 0).
    for (om, want, speed) in [(2u8, mode::WALK, 0), (3, mode::RUN, 100)] {
        let mut w = druid(420);
        put_owner(&mut w, (123, 100));
        w.fake.anim.insert(w.player, om);
        w.think_with(None, 0, false);
        assert_eq!(w.fake.modes(), [point_mode(want, 123, 108)], "O mode {om}");
        assert_eq!(w.vel_request().speed, speed, "O mode {om}");
    }
    // The pet follow is evaluated whatever d is: d ≤ 1 beside a neutral O
    // is pet move k 5 (wander near O by n = 6), not the later steps.
    let mut w = druid(420);
    put_owner(&mut w, (101, 100));
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander_at(1, 0, (101, 100), 6)]);
}

#[test]
fn spirit_wolf_with_a_target_attacks_runs_or_waits() {
    let with_target = |owner_at: (i32, i32), at: (i32, i32), melee: bool| {
        let mut w = druid(420);
        put_owner(&mut w, owner_at);
        let t = w.add_unit(UnitType::Monster, at);
        w.fake.good_for.insert(w.mon, (t, 5));
        if melee {
            w.fake.melee.insert(t);
        }
        w.seed(1);
        w.think_with(None, 0, false);
        (w, t)
    };
    // M ≠ 0: A1 at S, wait aip1 [22].
    let (w, t) = with_target((103, 100), (105, 100), true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, t)]);
    assert_eq!(w.thinks(), [22]);
    // M = 0 and reach(O, S) < r [20]: velocity (0, v, 0), run to S.
    let (w, t) = with_target((103, 100), (108, 100), false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::RUN, t)]);
    assert_eq!(w.vel_request().speed, 100);
    // reach(O, S) ≥ r and d > 10: walk in radius of O (8, 6).
    let (w, _) = with_target((112, 100), (135, 100), false);
    assert!(logged(&w, "radius 8 6"));
    // reach(O, S) ≥ r and d ≤ 10: idle 15.
    let (w, _) = with_target((103, 100), (135, 100), false);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn fenris_target_reachability_and_owner_leash() {
    // S = 0 (the wolf's own search finds nothing); T0 (the owner's
    // search) at distance 10 < r [24] and directly reachable → S := T0.
    for (reach_fails, runs) in [(false, true), (true, false)] {
        let mut w = druid(421);
        let t = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.good_for.insert(w.player, (t, 10));
        w.fake.reach_fails = reach_fails;
        w.seed(seed_with(1, |v| v[0] >= 25));
        w.think_with(None, 0, false);
        assert_eq!(
            w.fake.modes().contains(&unit_mode(mode::RUN, t)),
            runs,
            "reach_fails {reach_fails}"
        );
    }
    // T0's search distance 30 ≥ r: not taken.
    let mut w = druid(421);
    let t = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.good_for.insert(w.player, (t, 30));
    w.seed(seed_with(1, |v| v[0] >= 25));
    w.think_with(None, 0, false);
    assert!(!w.fake.modes().contains(&unit_mode(mode::RUN, t)));
    // S set, d > r and reach(O, S) > r → S := 0. O 26 away (≤ aip5):
    // S at 16 from O stays (run to it); S at 34 from O is dropped.
    for (sx, runs) in [(110, true), (160, false)] {
        let mut w = druid(421);
        put_owner(&mut w, (126, 100));
        let t = w.add_unit(UnitType::Monster, (sx, 100));
        w.fake.good_for.insert(w.mon, (t, 10));
        w.seed(seed_with(1, |v| v[0] >= 25));
        w.think_with(None, 0, false);
        assert_eq!(
            w.fake.modes().contains(&unit_mode(mode::RUN, t)),
            runs,
            "S at {sx}"
        );
    }
}

#[test]
fn fenris_ports_catches_up_and_follows() {
    // Skill2 ≥ 0 and d > 50: port to O, MODECHANGE at frame + 2, wait 10.
    let mut w = druid(421);
    put_owner(&mut w, (160, 100));
    w.game.frame = 5;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(10, 160, 100)]);
    assert!(has_event0(&w, 7));
    assert_eq!(w.thinks(), [15]);
    // D > aip5 [30]: k 0 (1, 100, 0), end whatever it returned.
    let mut w = druid(421);
    put_owner(&mut w, (140, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::RUN, 140, 108)]);
    assert_eq!(w.vel_request().speed, 100);
    // d > r [24]: O running (3) → the same; O walking (2) → k 0 (0, 0, 0).
    for (om, want, speed) in [(3u8, mode::RUN, 100), (2, mode::WALK, 0)] {
        let mut w = druid(421);
        put_owner(&mut w, (128, 100));
        w.fake.anim.insert(w.player, om);
        w.think_with(None, 0, false);
        assert_eq!(w.fake.modes(), [point_mode(want, 128, 108)], "O mode {om}");
        assert_eq!(w.vel_request().speed, speed, "O mode {om}");
    }
    // The quiet follow ≠ 0 ends the think: d ≤ 1 beside a neutral O →
    // k 5 (wander near O).
    let mut w = druid(421);
    put_owner(&mut w, (101, 100));
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [wander_at(1, 0, (101, 100), 6)]);
}

#[test]
fn fenris_rage_by_skill_state_and_corpse() {
    let rage = |w: &mut World| {
        give_skill(w, 1, 60, 10);
    };
    // Corpse K within r / 2 [12] of the unit, not in melee range, M = 0:
    // run to K, param 0 := 1 (the existing vector); in melee range: Skill1
    // at K, param 0 := 0.
    let lo = seed_with(1, |v| v[0] < 25);
    let mut w = druid(421);
    rage(&mut w);
    let k = w.add_unit(UnitType::Monster, (105, 100));
    w.fake.y.corpse = Some(k);
    w.fake.melee.insert(k);
    w.seed(lo);
    set_param_of(&mut w, 0, 1);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, k)]);
    assert_eq!(param_of(&w, 0), 0);
    // K too far (≥ r / 2): no rage cast, step 11.
    let mut w = druid(421);
    rage(&mut w);
    let k = w.add_unit(UnitType::Monster, (115, 100));
    w.fake.y.corpse = Some(k);
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(!w.fake.modes().contains(&unit_mode(10, k)));
    assert!(!w.fake.modes().contains(&unit_mode(mode::RUN, k)));
    // Skill1 < 0, or state 138: no corpse search at all.
    let mut w = druid(421);
    let k = w.add_unit(UnitType::Monster, (105, 100));
    w.fake.y.corpse = Some(k);
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(!logged(&w, "corpsefind 10"));
    let mut w = druid(421);
    rage(&mut w);
    w.fake.states.insert((w.mon, 138));
    let k = w.add_unit(UnitType::Monster, (105, 100));
    w.fake.y.corpse = Some(k);
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(!logged(&w, "corpsefind 10"));
    // M ≠ 0 (a monster in contact) with a corpse near: step 11's A1 branch.
    let mut w = druid(421);
    rage(&mut w);
    let k = w.add_unit(UnitType::Monster, (105, 100));
    w.fake.y.corpse = Some(k);
    let t = w.add_unit(UnitType::Monster, (104, 100));
    w.fake.good_for.insert(w.mon, (t, 4));
    w.fake.melee.insert(t);
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, t)]);
}

/// A cycleoflife (426) with `Skill1` 7 (the corpse cycler) at level 3, its
/// range calc worth `calc`, owner beside it.
fn cycle(calc: i32) -> World {
    let mut w = world(act_row(111, &[50, 20, 25, 10, 35]));
    grow(&mut w, 500);
    w.fake.class.insert(w.mon, 426);
    give_skill(&mut w, 1, 7, 8);
    w.skills = vec![Skills::decode(&vec![0u8; Skills::SIZE]); 8];
    w.fake.x.skill_level.insert(7, 3);
    w.fake.y.calc = calc;
    own(&mut w);
    put_owner(&mut w, (103, 100));
    w
}

#[test]
fn cycle_of_life_far_from_its_owner_teleports_to_it() {
    // d ≥ aip5 [35]: pet move k 3 (free spot near O, place, idle 5) ≠ 0 →
    // end (no corpse search).
    let mut w = cycle(12);
    put_owner(&mut w, (140, 100));
    let room = w.room;
    w.fake.y.free_spot = Some((110, 110));
    w.fake.x.room_at = Some(room);
    w.fake.x.place_ok = true;
    w.think_with(None, 0, false);
    assert!(logged(&w, "place 110 110"));
    assert_eq!(w.thinks(), [5]);
    assert!(!logged(&w, "corpsefind 12"));
    // d = 34 < aip5: the think goes on.
    let mut w = cycle(12);
    put_owner(&mut w, (134, 100));
    w.think_with(None, 0, false);
    assert!(logged(&w, "corpsefind 12"));
}

#[test]
fn cycle_of_life_corpse_search_range_and_conditions() {
    // n := the range calc at the entry's level, clamped to 5..50.
    for (calc, n) in [(2, 5), (12, 12), (100, 50)] {
        let mut w = cycle(calc);
        w.think_with(None, 0, false);
        assert!(logged(&w, &format!("corpsefind {n}")), "calc {calc}");
    }
    // Skill1 = 0 (not > 0), no skills row, or no entry: no search.
    let mut w = cycle(12);
    w.monstats[0].skill1 = 0;
    w.think_with(None, 0, false);
    assert!(!w.fake.log.iter().any(|l| l.starts_with("corpsefind")));
    let mut w = cycle(12);
    w.skills.clear();
    w.think_with(None, 0, false);
    assert!(!w.fake.log.iter().any(|l| l.starts_with("corpsefind")));
    let mut w = cycle(12);
    w.fake.x.skill_level.clear();
    w.think_with(None, 0, false);
    assert!(!w.fake.log.iter().any(|l| l.starts_with("corpsefind")));
}

/// A cycleoflife with a corpse K at `k_at`, no melee contact (so the cast
/// of step 8 is off) and a target `T` at (110, 100).
fn cycle_with_corpse(k_at: (i32, i32)) -> (World, UnitId, UnitId) {
    let mut w = cycle(12);
    let k = w.add_unit(UnitType::Monster, k_at);
    w.fake.y.corpse = Some(k);
    let t = w.add_unit(UnitType::Monster, (110, 100));
    (w, k, t)
}

#[test]
fn cycle_of_life_keeps_a_near_corpse_walks_to_it_or_idles() {
    // dK = 5 < aip2 [20]: K kept: walk to K (flags 7).
    let (mut w, k, _) = cycle_with_corpse((105, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, k)]);
    // dK = 25 ≥ 20: K := 0: idle aip3 [25]. (The owner is far enough for
    // the follow's k 3, which finds no free spot and returns 0; with a
    // near owner the follow itself would loiter and end the think.)
    let (mut w, _, _) = cycle_with_corpse((125, 100));
    put_owner(&mut w, (160, 100));
    w.think_with(None, 0, false);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [25]);
    // No corpse at all: idle 25.
    let mut w = cycle(12);
    put_owner(&mut w, (160, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [25]);
}

#[test]
fn cycle_of_life_backs_away_from_a_live_target_in_contact() {
    // C, T alive and `roll(100)` < 25: escape from T by aip4 [10], no delete.
    let lo = seed_with(1, |v| v[0] < 25);
    let (mut w, k, t) = cycle_with_corpse((105, 100));
    w.seed(lo);
    w.think_with(Some(t), 10, true);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 90, 100)]);
    let _ = k;
    // A draw ≥ 25: step 10 / 11 (walk to K).
    let lo = seed_with(1, |v| v[0] >= 25);
    let (mut w, k, t) = cycle_with_corpse((105, 100));
    w.seed(lo);
    w.think_with(Some(t), 10, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, k)]);
    // A dead T clears T and C: no escape even with a draw < 25.
    let (mut w, k, t) = cycle_with_corpse((105, 100));
    w.seed(seed_with(1, |v| v[0] < 25));
    w.fake.dead.insert(t);
    w.think_with(Some(t), 10, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, k)]);
}

#[test]
fn cycle_of_life_casts_on_a_corpse_in_contact_when_the_owner_needs_it() {
    // K in melee range, frame > f + aip1 [50]: class 426 needs O hurt
    // (life < max life), class 427 needs O's mana below max.
    let cast = |class: i32, life: i32, mana: i32, frame: i32| {
        let (mut w, k, _) = cycle_with_corpse((104, 100));
        w.fake.class.insert(w.mon, class);
        w.fake.melee.insert(k);
        w.fake.stats.insert((w.player, 6), life);
        w.fake.x.max_life.insert(w.player, 100);
        w.fake.stats.insert((w.player, 8), mana);
        w.fake.x.max_mana.insert(w.player, 50);
        w.game.frame = frame;
        w.think_with(None, 0, false);
        let cast = w.fake.modes() == [unit_mode(8, k)];
        (cast, param_of(&w, 1))
    };
    assert_eq!(cast(426, 40, 50, 60), (true, 60), "hurt owner, due");
    assert!(!cast(426, 100, 0, 60).0, "full life: no need");
    assert!(!cast(426, 40, 50, 50).0, "frame = f + aip1: not due");
    assert_eq!(cast(427, 100, 10, 60), (true, 60), "low mana");
    assert!(!cast(427, 0, 50, 60).0, "full mana: no need");
    // Skill 1 mode is the fixed 8 (the row's own mode is another).
    let (mut w, k, _) = cycle_with_corpse((104, 100));
    w.modes[0][0] = 3;
    w.fake.melee.insert(k);
    w.fake.stats.insert((w.player, 6), 1);
    w.fake.x.max_life.insert(w.player, 100);
    w.game.frame = 60;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(8, k)]);
}
