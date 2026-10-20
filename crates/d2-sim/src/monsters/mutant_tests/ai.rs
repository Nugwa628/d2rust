// fakes from the parent test modules)
use super::*;
use crate::monsters::ai::target::Search;

/// Distinct aip values: `aipN` of difficulty d = 100·N + 10·d + 1.
fn aip_value(n: usize, d: u8) -> i32 {
    100 * n as i32 + 10 * i32::from(d) + 1
}

#[test]
fn every_aip_column_by_difficulty() {
    let mut row = monstats(3, [0; 5], 15);
    let cols: [[&mut u16; 3]; 8] = [
        [&mut row.aip1, &mut row.aip1_n, &mut row.aip1_h],
        [&mut row.aip2, &mut row.aip2_n, &mut row.aip2_h],
        [&mut row.aip3, &mut row.aip3_n, &mut row.aip3_h],
        [&mut row.aip4, &mut row.aip4_n, &mut row.aip4_h],
        [&mut row.aip5, &mut row.aip5_n, &mut row.aip5_h],
        [&mut row.aip6, &mut row.aip6_n, &mut row.aip6_h],
        [&mut row.aip7, &mut row.aip7_n, &mut row.aip7_h],
        [&mut row.aip8, &mut row.aip8_n, &mut row.aip8_h],
    ];
    for (i, c) in cols.into_iter().enumerate() {
        for (d, v) in c.into_iter().enumerate() {
            *v = aip_value(i + 1, d as u8) as u16;
        }
    }
    let mut w = World::new(row);
    let p = param(None, 0, false);
    for d in 0..3u8 {
        let got: Vec<i32> = w.with(|_, cx| {
            cx.info.difficulty = d;
            (1..=8).map(|n| cx.aip(&p, n)).collect()
        });
        let want: Vec<i32> = (1..=8).map(|n| aip_value(n, d)).collect();
        assert_eq!(got, want, "difficulty {d}");
    }
}

#[test]
fn aidel_normal_column_on_a_typed_game() {
    // With game +0x6A non-zero the column follows the difficulty: Normal
    // reads `aidel`, not the Hell column.
    let mut row = monstats(3, [0; 5], 11);
    row.aidel_n = 12;
    row.aidel_h = 13;
    let mut w = World::new(row);
    for (d, want) in [(0, 11), (1, 12), (2, 13)] {
        let got = w.with(|_, cx| {
            cx.info.game_type = 1;
            cx.info.difficulty = d;
            cx.aidel(0)
        });
        assert_eq!(got, want, "difficulty {d}");
    }
}

// ----: who schedules a think -----------------------------------------

#[test]
fn update_ai_callback_death_stops_even_for_listed_bases() {
    // Death (0) or dead (12) stops before the base-class rule: base 110
    // gets no think in those modes.
    for m in [mode::DEATH, mode::DEAD] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.monstats[0].baseid = 110;
        let mon = w.mon;
        w.fake.anim.insert(mon, m);
        w.with(|g, cx| update_ai_callback(g, cx, mon));
        assert!(w.thinks().is_empty(), "mode {m}");
    }
}

#[test]
fn npc_interaction_needs_npc_and_interact() {
    // Both bits: delete + 1; the start also sets AI param 0 := 40.
    for start in [true, false] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.monstats[0].npc = true;
        w.monstats[0].interact = true;
        w.game.frame = 10;
        let mon = w.mon;
        w.game.schedule_event(mon, 2, 30, None, 0, 0).unwrap();
        w.with(|g, cx| npc_interaction(g, cx, mon, start));
        assert_eq!(w.thinks(), [11], "start {start}");
        let want = if start { 40 } else { 0 };
        assert_eq!(w.control().params[0], want, "start {start}");
    }
    // One bit only: nothing.
    for (npc, interact) in [(true, false), (false, true)] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.monstats[0].npc = npc;
        w.monstats[0].interact = interact;
        w.game.frame = 10;
        let mon = w.mon;
        w.game.schedule_event(mon, 2, 30, None, 0, 0).unwrap();
        w.with(|g, cx| npc_interaction(g, cx, mon, true));
        assert_eq!(w.thinks(), [30], "{npc} {interact}");
        assert_eq!(w.control().params[0], 0, "{npc} {interact}");
    }
}

#[test]
fn freeze_apply_reschedules_after_length() {
    // Delete + schedule at frame + length + 1.
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 5;
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 50, None, 0, 0).unwrap();
    w.with(|g, cx| schedule_after_freeze(g, cx, mon, 10));
    assert_eq!(w.thinks(), [16]);
}

// ----: inline thinks -------------------------------------------------

#[test]
fn spl_end_generic_cases() {
    // (base class, mode that ended, SplEndGeneric, frozen, inline think).
    // Idle AI: an inline think schedules +200; otherwise the end requests
    // neutral and schedules nothing itself.
    let cases: &[(u16, u8, u8, bool, bool)] = &[
        (110, 8, 1, false, true),
        (110, 9, 1, false, false),
        (110, 8, 0, false, false),
        (247, 14, 1, false, true),
        (247, 8, 1, false, false),
        (136, 10, 1, false, true),
        (136, 11, 1, false, true),
        (136, 9, 1, false, false),
        (230, 4, 1, false, true),
        (231, 5, 1, false, true),
        (403, 9, 1, false, true),
        (118, 4, 1, false, false),
        (3, 8, 1, false, false),
        // Frozen and alive: the inline case runs no think and requests
        // nothing.
        (230, 4, 1, true, false),
    ];
    for &(base, ended, generic, frozen, inline) in cases {
        let mut w = World::new(monstats(1, [0; 5], 15));
        w.monstats[0].baseid = base;
        w.monstats[0].splendgeneric = generic;
        let mon = w.mon;
        if frozen {
            w.fake.states.insert((mon, state::FREEZE));
        }
        w.with(|g, cx| mode_end(g, cx, mon, ended));
        let case = format!("{base} {ended} {generic} {frozen}");
        let want: &[i32] = if inline { &[200] } else { &[] };
        assert_eq!(w.thinks(), want, "{case}");
        // Every case that is not inline requests neutral, except the
        // frozen inline case, which does neither. No path target unit
        // here: the request targets the point (0, 0).
        let neutral = w.logged(&format!("mode {} Point(0, 0)", mode::NEUTRAL));
        assert_eq!(neutral, !inline && !frozen, "{case}");
    }
}

#[test]
fn spl_end_generic_is_tested_before_the_walk_table() {
    // Willowisp (118) at the end of walk: the SplEndGeneric branch runs
    // first and leaves the anim mode as it is (still walk at the think, so
    // the Idle AI's idle requests neutral); without SplEndGeneric the
    // table-1 branch sets neutral first (no request). Both think inline
    // (Idle AI → +200).
    for (generic, request) in [(1, true), (0, false)] {
        let mut w = World::new(monstats(1, [0; 5], 15));
        w.monstats[0].baseid = 118;
        w.monstats[0].splendgeneric = generic;
        let mon = w.mon;
        w.fake.anim.insert(mon, mode::WALK);
        w.with(|g, cx| mode_end(g, cx, mon, mode::WALK));
        assert_eq!(w.thinks(), [200], "generic {generic}");
        let neutral = w.logged(&at_unit(mode::NEUTRAL, mon));
        assert_eq!(neutral, request, "generic {generic}");
    }
}

// ----: installing an AI ----------------------------------------------

#[test]
fn install_ignores_state_18_and_non_monsters() {
    // State ≥ 18 on a monster: nothing (params not reset, state kept).
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.control().params[0] = 5;
    let function = w.control().function;
    w.with(|g, cx| install(g, cx, mon, 18));
    let c = w.control().clone();
    assert_eq!((c.params[0], c.special_state, c.function), (5, 0, function));
    // A player with state 0: nothing, not even the missing-control log.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let pl = w.player;
    w.store.unhandled.clear();
    w.with(|g, cx| install(g, cx, pl, 0));
    assert!(w.store.unhandled.is_empty());
}

/// Raven (107): its init writes
/// param 0 := −1 and draws one step, so a call is visible.
const RAVEN: u16 = 107;

/// The first AI table index whose record has no init function.
fn ai_without_init() -> u16 {
    table::AI_TABLE
        .iter()
        .position(|r| r.init == 0 && r.think != 0)
        .expect("an AI") as u16
}

fn init_ran(w: &World) -> bool {
    w.store.control(w.mon).unwrap().params[0] == -1
}

#[test]
fn install_init_only_with_a_function_and_both_records() {
    assert!(INIT_IMPLEMENTED.contains(&table::AI_TABLE[RAVEN as usize].init));
    // No init function: nothing is called.
    let w = World::new(monstats(ai_without_init(), [0; 5], 15));
    assert!(!init_ran(&w) && w.store.unhandled.is_empty());
    // An init function with both records: called.
    let w = World::new(monstats(RAVEN, [0; 5], 15));
    assert!(init_ran(&w));
    // An init function, but `MonStatsEx` = the monstats2 count (no record):
    // not called.
    let mut row = monstats(RAVEN, [0; 5], 15);
    row.monstatsex = 1;
    let w = World::new(row);
    assert!(!init_ran(&w));
}

// ----: per-AI behaviours -----------------------------------------------

#[test]
fn function_0_does_nothing() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.store.unhandled.clear();
    w.fake.log.clear();
    w.seed(1);
    let p = param(Some(w.player), 5, false);
    let mon = w.mon;
    w.with(|g, cx| run_function(g, cx, 0x005B_0CC0, mon, &p));
    assert!(w.store.unhandled.is_empty());
    assert!(w.fake.log.is_empty());
    assert!(w.thinks().is_empty());
    assert_eq!(draws(&w, 1), 0);
}

#[test]
fn zombie_at_aip2_distance_wanders_without_the_draw() {
    // D < aip2 is strict: at D = aip2 [10] no P(aip1) draw, only the
    // wander. The seed's first draw (0) would pass P(30).
    let lo = 4_014_346_870;
    let mut w = World::new(monstats(3, [30, 10, 0, 20, 0], 15));
    w.seed(lo);
    w.run(false, 10);
    let ((x, y), k) = wander_after(lo, 0, (100, 100), 3);
    assert_eq!(last_mode(&w), walk_point(x, y));
    assert_eq!(draws(&w, lo), k);
}

#[test]
fn fallen_pack_command_needs_d_below_15() {
    // A pack leader at D = 15: no command (step 5.2 needs D < 15); step 5.3
    // with D > aip2 [10]: the first draw (0) passes pct(30) → wander 3.
    let lo = 4_014_346_870;
    let mut w = fallen_world();
    let me = w.guid(w.mon);
    let m = w.spawn(UnitType::Monster, (0, 0));
    w.store.entry(m).control = Some(AiControl::default());
    let mg = w.guid(m);
    let c = w.control();
    c.minion_owner = Some(UnitRef {
        ty: UnitType::Monster,
        guid: me,
    });
    c.minions = vec![mg];
    w.seed(lo);
    w.run(false, 15);
    assert!(w.store.control(m).unwrap().commands.is_empty());
    let ((x, y), _) = wander_after(lo, 1, (100, 100), 3);
    assert_eq!(w.fake.modes(), [walk_point(x, y)]);
}

// ----: the think event ----------------------------------------------

#[test]
fn think_event_runs_the_think() {
    // A type-2 event on a monster that is not frozen runs the think (Idle
    // AI: idle 200) and is not handed on.
    struct Count(u32);
    impl EventDispatch for Count {
        fn run_event(&mut self, _: &mut Game, _: &TimerRun) {
            self.0 += 1;
        }
    }
    let mut w = World::new(monstats(1, [0; 5], 15));
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 1, None, 0, 0).unwrap();
    w.game.frame = 1;
    let mut next = Count(0);
    let mut d = MonsterDispatch {
        cx: Ctx {
            tables: AiTables {
                monstats: &w.monstats,
                monstats2: &w.monstats2,
                levels: &w.levels,
                skill_modes: &w.modes,
                skills: &w.skills,
                missiles: &w.missiles,
            },
            info: GameInfo::default(),
            store: &mut w.store,
            world: &mut w.fake,
        },
        next: &mut next,
    };
    crate::tick::run_timer_events(&mut w.game, &mut d);
    assert_eq!(next.0, 0);
    assert_eq!(w.thinks(), [201]);
}

#[test]
fn lancer_at_aip5_distance_does_not_run() {
    // D > aip5 is strict: at D = aip5 [15] step 1 does not run (AI param
    // 0 stays 0).
    let mut w = lancer_world();
    w.seed(1);
    w.run(false, 15);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    assert!(!w.logged(&at_unit(mode::RUN, w.player)));
}

#[test]
fn archer_no_s_draw_49_circles() {
    // `lo' % 100 > 49` → idle; a draw of exactly 49 circles.
    let lo = seed_where(1, |v| v[0] % 100 == 49);
    let (mut w, _) = archer_world();
    w.seed(lo);
    w.run(false, 9);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
}

/// The archer with S at distance `e`; aip1, aip2 = 0 and aip8 as given,
/// aip4 = 100, aip5 = 15. Returns (modes, thinks, draws from seed 1).
fn archer_at(e: i32, aip1: u16, aip8: u16) -> (Vec<String>, Vec<i32>, usize) {
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, e));
    (w.monstats[0].aip1, w.monstats[0].aip4, w.monstats[0].aip8) = (aip1, 100, aip8);
    w.seed(1);
    w.run(false, 9);
    (w.fake.modes(), w.thinks(), draws(&w, 1))
}

#[test]
fn archer_escape_needs_e_below_6() {
    // E = 6: no aip4 draw and no escape; on to step 6 (P(aip2) = 0 fails,
    // idle aip3 [8]).
    assert_eq!(archer_at(6, 0, 0), (vec![], vec![8], 1));
}

#[test]
fn archer_walk_needs_0_below_aip8_below_e() {
    // aip8 = 0, or aip8 = E: no aip1 draw and no walk; idle aip3.
    assert_eq!(archer_at(10, 100, 0), (vec![], vec![8], 1));
    assert_eq!(archer_at(10, 100, 10), (vec![], vec![8], 1));
}

#[test]
fn archer_run_needs_e_above_aip5() {
    // E = aip5 [15]: no run; idle aip3.
    assert_eq!(archer_at(15, 0, 0), (vec![], vec![8], 1));
}

#[test]
fn archer_skill1_id_0_is_a_skill() {
    // `Skill1` < 0 is "no skill": id 0 is used in `Sk1mode` (8) at S.
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 10));
    let r = &mut w.monstats[0];
    (r.skill1, r.skill2, r.skill3) = (0, 0xFFFF, 0xFFFF);
    r.aip2 = 100;
    w.run(true, 9);
    assert!(w.logged("skill 0"));
    assert_eq!(last_mode(&w), at_unit(8, s));
}

// ----: prechecks B and C ---------------------------------------

/// The first AI table index whose record has target mode `m`.
fn ai_with_target_mode(m: u32) -> u16 {
    table::AI_TABLE
        .iter()
        .position(|r| r.target_mode == m && r.think != 0)
        .expect("an AI") as u16
}

#[test]
fn precheck_b_modes_2_and_4() {
    // Mode 2 with no target found: continue with target 0.
    let mut w = World::new(monstats(ai_with_target_mode(2), [0; 5], 15));
    let mon = w.mon;
    let mut p = param(None, 0, false);
    assert!(!w.with(|g, cx| precheck_b(g, cx, mon, &mut p)));
    assert_eq!(p.target, None);
    // Mode 4 with a target found: continue with it.
    let mut w = World::new(monstats(ai_with_target_mode(4), [0; 5], 15));
    w.fake.nodes = vec![vec![w.player]];
    let mon = w.mon;
    let mut p = param(None, 0, false);
    assert!(!w.with(|g, cx| precheck_b(g, cx, mon, &mut p)));
    assert_eq!(p.target, Some(w.player));
}

#[test]
fn boss_sound_for_class_250_below_20() {
    // Class 250 (summoner), neither unique nor boss: the sound plays at
    // D < 20, not at D = 20.
    for (d, sound) in [(19, true), (20, false)] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        let mon = w.mon;
        w.fake.class.insert(mon, 250);
        let mut p = param(Some(w.player), d, false);
        let stop = w.with(|g, cx| precheck_c(g, cx, mon, &mut p));
        assert_eq!(stop, sound, "D {d}");
        assert_eq!(w.logged("sound 16"), sound, "D {d}");
    }
}

#[test]
fn teleport_hurt_means_life_below_30() {
    // A melee monster at life 30% is not hurt: no `roll(100) < 15` and no
    // spot search; at 29% it is.
    let lo = seed_where(2, |v| pc(v[0]) < 40 && pc(v[1]) < 15);
    for (life, searched) in [(30, false), (29, true)] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.monstats[0].ismelee = true;
        w.control().flags |= flag::MAY_TELEPORT;
        w.fake.life = life;
        w.seed(lo);
        let mon = w.mon;
        let mut p = param(Some(w.player), 30, false);
        w.with(|g, cx| precheck_c(g, cx, mon, &mut p));
        assert_eq!(w.logged("find_spot"), searched, "life {life}");
    }
}

// ----: main search, slots 8 and 9 and the result ----------------------

/// A monster at (100, 100) with the line-of-sight flag T = 1 (control flag
/// 0x40) and `nodes`.
fn search(w: &mut World, nodes: Vec<Vec<UnitId>>) -> Search {
    w.fake.nodes = nodes;
    w.control().flags |= flag::FORCE_LOS;
    let mon = w.mon;
    w.with(|g, cx| main_search(g, cx, mon))
}

#[test]
fn slot_8_same_act_below_b_and_passing() {
    let none = Vec::new;
    // Same act, d = 34 < B (35): chosen, distance 34.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let n = w.spawn(UnitType::Monster, (134, 100));
    let mut nodes = vec![none(); 8];
    nodes.push(vec![n]);
    let s = search(&mut w, nodes.clone());
    assert_eq!((s.target, s.distance), (Some(n), 34));
    // d = B: not chosen.
    w.fake.pos.insert(n, (135, 100));
    assert_eq!(search(&mut w, nodes.clone()).target, None);
    // Another act: skipped.
    w.fake.pos.insert(n, (110, 100));
    w.fake.acts.insert(n, 1);
    assert_eq!(search(&mut w, nodes.clone()).target, None);
    // Same act, close, but the collision test fails with T = 1.
    w.fake.acts.remove(&n);
    w.fake.line_blocked.insert(n);
    assert_eq!(search(&mut w, nodes).target, None);
}

#[test]
fn slot_9_nearest_same_act_passing_is_the_alternative() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let far = w.spawn(UnitType::Monster, (110, 100));
    let tie = w.spawn(UnitType::Monster, (100, 110));
    let other_act = w.spawn(UnitType::Monster, (101, 100));
    let blocked = w.spawn(UnitType::Monster, (102, 100));
    let near = w.spawn(UnitType::Monster, (105, 100));
    w.fake.acts.insert(other_act, 1);
    w.fake.line_blocked.insert(blocked);
    let mut nodes = vec![Vec::new(); 9];
    nodes.push(vec![far, tie, other_act, blocked, near]);
    // Not taken: the alternative offered is `near` (d 5), the target stays
    // none.
    let s = search(&mut w, nodes.clone());
    assert_eq!(s.target, None);
    assert_eq!(w.fake.log, [format!("alt {}", near.0)]);
    // Taken: the alternative becomes the target.
    w.fake.log.clear();
    w.fake.take_alt = true;
    let s = search(&mut w, nodes);
    assert_eq!((s.target, s.distance), (Some(near), 5));
    // Ties keep the earlier node: two at d 10.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let a = w.spawn(UnitType::Monster, (110, 100));
    let b = w.spawn(UnitType::Monster, (100, 110));
    let mut nodes = vec![Vec::new(); 9];
    nodes.push(vec![a, b]);
    search(&mut w, nodes);
    assert_eq!(w.fake.log, [format!("alt {}", a.0)]);
}

#[test]
fn good_alignment_does_not_mark_seen() {
    // Alignment good (2): the target is returned without control flag
    // 0x08; evil (0) sets it.
    for (align, seen) in [(2u8, false), (0, true)] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.fake.align = align;
        w.fake.good = Some((w.player, 5));
        w.fake.nodes = vec![vec![w.player]];
        let mon = w.mon;
        let s = w.with(|g, cx| main_search(g, cx, mon));
        assert_eq!(s.target, Some(w.player), "align {align}");
        let f = w.control().flags & flag::TARGET_SEEN;
        assert_eq!(f != 0, seen, "align {align}");
    }
}

#[test]
fn vision_token_toggles_only_when_loaded() {
    // (record's +0x24, flag 0x08 already set) -> value written by step 7.
    // Record absent: no write. Token 0 with flag clear: writes 1.
    // Token 1 with flag clear (read, T = 0): writes 0. Flag set: S = 0
    // (not read), writes 1.
    for (vision, seen_flag, expect) in [
        (None, false, None),
        (Some(0), false, Some(1)),
        (Some(1), false, Some(0)),
        (Some(1), true, Some(1)),
    ] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.fake.no_los_draw = true;
        w.fake.vision = vision;
        w.fake.nodes = vec![vec![w.player]];
        if seen_flag {
            let mon = w.mon;
            w.store.control_mut(mon).unwrap().flags |= flag::TARGET_SEEN;
        }
        let mon = w.mon;
        let s = w.with(|g, cx| main_search(g, cx, mon));
        assert_eq!(s.target, Some(w.player), "{vision:?} {seen_flag}");
        assert_eq!(
            w.fake.marks.last().copied(),
            expect,
            "{vision:?} {seen_flag}"
        );
    }
}

// ----: movement requests ---------------------------------------------

#[test]
fn decrepify_turns_only_run_into_walk() {
    // Run with state 60: velocity request reset, walk instead.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.fake.states.insert((mon, state::DECREPIFY));
    w.with(|_, cx| set_velocity(cx, mon, 0, 50, 9));
    w.with(|g, cx| move_to(g, cx, mon, ModeTarget::Point(1, 2), mode::RUN, 1, 0));
    assert_eq!(last_mode(&w), walk_point(1, 2));
    assert_eq!(w.velocity(), VelocityRequest::default());
    // Walk with state 60: the request stays.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.fake.states.insert((mon, state::DECREPIFY));
    w.with(|_, cx| set_velocity(cx, mon, 0, 50, 9));
    w.with(|g, cx| move_to(g, cx, mon, ModeTarget::Point(1, 2), mode::WALK, 1, 0));
    assert_eq!(last_mode(&w), walk_point(1, 2));
    assert_eq!((w.velocity().speed, w.velocity().steps), (50, 9));
}

#[test]
fn escape_steps_above_5_and_both_axes() {
    // n = 5: no velocity request; walk to own + sign(own − t)·n per axis.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let (mon, pl) = (w.mon, w.player);
    w.fake.pos.insert(pl, (105, 103));
    w.with(|g, cx| escape(g, cx, mon, Some(pl), 5, false, false));
    assert_eq!(w.velocity().steps, 0);
    assert_eq!(last_mode(&w), walk_point(95, 95));
    // n = 12: steps 12, (88, 88).
    let mut w = World::new(monstats(3, [0; 5], 15));
    let (mon, pl) = (w.mon, w.player);
    w.fake.pos.insert(pl, (105, 103));
    w.with(|g, cx| escape(g, cx, mon, Some(pl), 12, false, false));
    assert_eq!(w.velocity().steps, 12);
    assert_eq!(last_mode(&w), walk_point(88, 88));
}

#[test]
fn circle_low_byte_128_is_method_6() {
    let lo = seed_where(1, |v| v[0] & 0xFF == 128);
    let mut w = World::new(monstats(3, [0; 5], 15));
    let (mon, pl) = (w.mon, w.player);
    w.seed(lo);
    w.with(|g, cx| circle(g, cx, mon, Some(pl), 3, false));
    assert_eq!(w.velocity().method, 6);
}

#[test]
fn install_switches_to_alternate_only_when_nonzero() {
    // The current record's think is the control's function but its
    // alternate is 0: not step 3; step 4 resets the params and keeps the
    // think.
    let ai = table::AI_TABLE
        .iter()
        .position(|r| r.think != 0 && r.alt == 0)
        .expect("an AI") as u16;
    let mut w = World::new(monstats(ai, [0; 5], 15));
    let mon = w.mon;
    let think = w.control().function;
    assert_ne!(think, 0);
    w.control().params[0] = 5;
    w.with(|g, cx| install(g, cx, mon, 0));
    let c = w.control().clone();
    assert_eq!((c.params[0], c.function), (0, think));
}
