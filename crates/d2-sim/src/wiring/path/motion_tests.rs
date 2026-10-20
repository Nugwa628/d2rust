//! Motion on the path provider (`enable_paths`): a missile built and
//! flown by the real creation code, and monsters walking through the
//! real monster mode set,
//! the every-tick event 0 and the mode end.

use std::sync::Arc;

use crate::drlg::TileRect;
use crate::missiles::{create_missile, param_flags, MissileParams};
use crate::monsters::ai::seams::{AiModes, AiUnits};
use crate::monsters::ai::ModeTarget;
use crate::path::record::{flags, path_types};
use crate::tick::events::event;
use crate::units::{UnitId, UnitType};
use crate::wiring::action::tests::{Fx, LEVEL};

/// `velocitypercent`.
const STAT_VELOCITY: u16 = 67;
/// Monster modes.
const WALK: u8 = 2;
const ATTACK1: u8 = 4;
const BLOCK: u8 = 6;

fn fx_paths(on: bool) -> Fx {
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
    ]);
    let h = fx.sim.hooks();
    if on {
        h.enable_paths().expect("embedded tables");
    }
    // monstats `Velocity` 6: velocity 0x600 at 100 %, the same speed as
    // the player's walk.
    // monstats2 `SizeX` 2: pattern 1, a footprint,
    // as in `coarse_free_box_avoids_a_monster_footprint`.
    let t = Arc::make_mut(&mut h.tables);
    t.combat.monstats[0].velocity = 6;
    let ex = usize::from(t.combat.monstats[0].monstatsex);
    t.combat.monstats2[ex].sizex = 2;
    fx
}

fn fx() -> Fx {
    fx_paths(true)
}

/// AnimData records for monster class 0 in `modes`: (mode, frames) at
/// speed 256 (one frame a tick), so a scheduling mode has its events.
fn with_anims(fx: &mut Fx, modes: &[(u32, u32)]) {
    use d2_formats::animdata::{self, AnimData, AnimRecord};
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    let h = fx.sim.hooks();
    for &(mode, frames) in modes {
        let s = format!("M0{mode:02}HTH");
        let mut name = [0u8; 8];
        name[..7].copy_from_slice(s.as_bytes());
        a.buckets[animdata::hash(&name[..7])].push(AnimRecord {
            name,
            frames,
            speed: 256,
            events: [0; animdata::EVENTS],
        });
        h.x.names.insert((UnitType::Monster, mode), name);
    }
    h.anim_data = Some(Arc::new(a));
}

fn monster(fx: &mut Fx, x: i32, y: i32) -> UnitId {
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, x, y);
    fx.stats(m, &[(STAT_VELOCITY, 100)]);
    // The creation's first think (covered by
    // `a_created_monster_gets_its_first_think_at_f_plus_2`) is not under
    // test here: the tests start from a monster with no pending think.
    fx.game.timers.cancel_unit_events(m, event::AI_THINK, None);
    m
}

#[test]
fn a_created_monster_gets_its_first_think_at_f_plus_2() {
    // The creation mode (NU) through the monster mode set
    // schedules the think at f + aidel (0 → 15); the think restart
    // restart deletes it and schedules f + 2 (the
    // "+15, cancel, +2" pair a new monster's timers show).
    // The room has a client, so the think-restart gate holds.
    let mut fx = fx();
    let a = fx.a;
    fx.add_room_client(a);
    let f = fx.game.frame;
    let m = fx.spawn(UnitType::Monster, 0, a, 26, 10);
    assert_eq!(mode(&fx, m), 1);
    let thinks: Vec<_> = fx
        .timers(m)
        .into_iter()
        .filter(|&(e, _)| e == event::AI_THINK)
        .collect();
    assert_eq!(thinks, [(event::AI_THINK, f + 2)]);
    fx.assert_clean();
}

#[test]
fn a_monster_created_where_no_client_is_gets_no_think() {
    // The think-restart gate is false (the room's client count is 0): no think restart;
    // the room clean-up runs instead, so the creation mode's
    // think at f + aidel is the only one left.
    let mut fx = fx();
    let a = fx.a;
    let f = fx.game.frame;
    let m = fx.spawn(UnitType::Monster, 0, a, 26, 10);
    assert_eq!(mode(&fx, m), 1);
    let thinks: Vec<_> = fx
        .timers(m)
        .into_iter()
        .filter(|&(e, _)| e == event::AI_THINK)
        .collect();
    assert!(thinks.iter().all(|&(_, at)| at != f + 2), "{thinks:?}");
    fx.assert_clean();
}

/// An AI mode change (the AI's mode request plus the monster mode set).
fn change(fx: &mut Fx, m: UnitId, mode: u8, t: ModeTarget) -> bool {
    fx.sim
        .with(&mut fx.game, |g, v| v.change_mode(g, m, mode, t))
}

fn mode(fx: &Fx, u: UnitId) -> u32 {
    fx.sim.sys.units.get(u).unwrap().mode
}

/// Ticks while the unit stays in `moving`, recording (precise x,
/// precise y, mode) after each tick.
fn ticks(fx: &mut Fx, u: UnitId, moving: u32, max: usize) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..max {
        fx.tick();
        let d = dynamic(fx, u);
        let m = mode(fx, u);
        out.push((d.precise_x, d.precise_y, m));
        if m != moving {
            break;
        }
    }
    out
}

fn fire(fx: &mut Fx, owner: UnitId, tx: i32, ty: i32) -> UnitId {
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: tx,
        target_y: ty,
        ..MissileParams::default()
    };
    fx.sim
        .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
        .unwrap()
        .expect("created")
}

fn dynamic(fx: &mut Fx, u: UnitId) -> crate::path::DynamicPath {
    fx.sim
        .hooks()
        .paths
        .as_ref()
        .unwrap()
        .dynamic(u)
        .unwrap()
        .clone()
}

#[test]
fn missile_path_is_built_and_flown_on_the_provider() {
    // WP1: the missile's path has type 4 and flag 0x40000 from its
    // allocation, so the build goes to the straight missile path:
    // one point (the target), flag 0x20. The arrow's Vel 1 gives 0x100,
    // 75 % → 0xC0; the every-tick event moves it
    // (0x400 · 0xC0) >> 6 = 0xC00 a frame along x (the
    // direction vector is (4096, 0)).
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let m = fire(&mut fx, owner, 79, 30);
    let d = dynamic(&mut fx, m);
    assert_eq!(d.path_type, path_types::MISSILE);
    assert_ne!(d.flags & flags::MISSILE, 0);
    assert_ne!(d.flags & flags::ACTIVE, 0);
    assert_eq!((d.point_count, d.point(0).x, d.point(0).y), (1, 79, 30));
    assert_eq!(d.velocity, 0xC0);
    for k in 1..=30u32 {
        fx.frame();
        let d = dynamic(&mut fx, m);
        assert_eq!(
            (d.precise_x, d.precise_y),
            (0x298000 + k * 0xC00, 0x1E8000),
            "frame {k}"
        );
    }
    assert_eq!(fx.sim.hooks().path_position(m), (42, 30));
    fx.assert_clean();
}

#[test]
fn frames_from_distance_reads_the_path_target_distance() {
    // Flag 0x400 (the lob flags 0x420 of `skill_missile`): the current
    // frame is (d << 16) / (v << 4) with d = the missile distance from the path
    // position (41, 30) to the target (49, 34): max 8 + min 4 / 2 = 10;
    // v = 0xC0 (as above): 0xA0000 / 0xC00 = 213. Before the provider's
    // distance d read 0 (→ 1): 21 frames, the lob missile died at once.
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE | param_flags::FRAMES_FROM_DISTANCE,
        target_x: 49,
        target_y: 34,
        ..MissileParams::default()
    };
    let current = fx
        .sim
        .missiles(&mut fx.game, |g, cx| {
            let m = create_missile(g, cx, &p)?;
            Some(cx.store.get(m)?.current)
        })
        .unwrap()
        .expect("created");
    assert_eq!(current, 213);
    fx.assert_clean();
}

#[test]
fn missile_out_of_range_gets_no_point() {
    // Missile range check: a target 100 sub-tiles away fails the create
    // before any path; 99 away is built with one point.
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 1, 30);
    let m = fire(&mut fx, owner, 100, 30);
    assert_eq!(dynamic(&mut fx, m).point_count, 1);
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: 101,
        target_y: 30,
        ..MissileParams::default()
    };
    let r = fx
        .sim
        .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
        .unwrap();
    assert!(r.is_none());
}

#[test]
fn monster_walks_to_a_point_sub_tile_by_sub_tile() {
    // The player's walk test for a monster: monstats Velocity 6 at 100 % → 0x600;
    // the mode set targets the point, gives the re-path budget 20 and
    // path type 13 (request byte 101), computes one point; event 0 every
    // tick moves +0x6000 a tick, the 14th lands on (31, 10) and the mode
    // end sets the anim mode neutral.
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    assert!(change(&mut fx, m, WALK, ModeTarget::Point(31, 10)));
    assert_eq!(mode(&fx, m), 2);
    let d = dynamic(&mut fx, m);
    assert_eq!(d.path_type, path_types::TOWARD_FINISH);
    assert_eq!((d.target_x, d.target_y, d.target_unit), (31, 10, None));
    assert_eq!(d.repath_budget, 20);
    assert_eq!(d.velocity, 0x600);
    assert!(d.point_count >= 1);
    assert_eq!(
        (
            d.point(d.point_count as usize - 1).x,
            d.point(d.point_count as usize - 1).y
        ),
        (31, 10)
    );
    assert!(fx.timers(m).contains(&(event::MODE_CHANGE, -1)));
    let t = ticks(&mut fx, m, 2, 20);
    assert_eq!(t.len(), 14);
    for (k, &(x, y, md)) in t.iter().take(13).enumerate() {
        assert_eq!(x, 0x1AE000 + k as u32 * 0x6000, "tick {}", k + 1);
        assert_eq!((y, md), (0xA8000, 2), "tick {}", k + 1);
    }
    assert_eq!(t[13], (0x1F8000, 0xA8000, 1));
    assert_eq!(fx.sim.hooks().path_position(m), (31, 10));
    // The footprint moved with the monster (mask 0x100).
    let a = fx.a;
    let cell = |fx: &mut Fx, x, y| {
        crate::path::collision::point_value(&fx.sim.hooks().drlg, Some(a), x, y, 0xFFFF)
    };
    assert_eq!(cell(&mut fx, 26, 10) & 0x100, 0);
    assert_ne!(cell(&mut fx, 31, 10) & 0x100, 0);
    fx.assert_clean();
}

#[test]
fn a_walk_without_a_path_point_reports_a_failed_mode_change() {
    // A walk to the monster's own cell computes no point; the WL start
    // returns 0, the neutral start runs instead and the monster mode set reports
    // failure, so the
    // AI's failure branch runs.
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    assert!(!change(&mut fx, m, WALK, ModeTarget::Point(26, 10)));
    assert_eq!(mode(&fx, m), 1);
    assert_eq!(dynamic(&mut fx, m).point_count, 0);
    fx.assert_clean();
}

#[test]
fn without_the_provider_the_monster_does_not_move() {
    // The same mode change with the provider off sets the mode and
    // the every-tick event, but nothing steps the monster.
    let mut fx = fx_paths(false);
    let m = monster(&mut fx, 26, 10);
    let before = fx.sim.hooks().path_position(m);
    // The walk start fails, so the mode change reports failure.
    assert!(!change(&mut fx, m, WALK, ModeTarget::Point(31, 10)));
    for _ in 0..20 {
        fx.tick();
    }
    // The walk start's mode set is the provider's (module doc).
    assert_eq!(mode(&fx, m), 1);
    assert_eq!(fx.sim.hooks().path_position(m), before);
}

#[test]
fn a_mode_that_does_not_move_targets_the_unit_and_computes_nothing() {
    // Request byte 100: nothing is written to the path (the
    // the type keeps the allocation's, no point); the request's unit is
    // the path target, so the AI's path target reads it.
    let mut fx = fx();
    with_anims(&mut fx, &[(6, 8)]);
    let m = monster(&mut fx, 26, 10);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 28, 10);
    let b = dynamic(&mut fx, m);
    change(&mut fx, m, BLOCK, ModeTarget::Unit(p));
    let d = dynamic(&mut fx, m);
    assert_eq!((d.path_type, d.point_count), (b.path_type, 0));
    assert_eq!(
        (d.dist_budget, d.max_distance),
        (b.dist_budget, b.max_distance)
    );
    assert_eq!(d.target_unit.map(|t| t.unit), Some(p));
    assert_eq!(d.repath_budget, 20);
    let target = fx.sim.with(&mut fx.game, |_, v| AiUnits::path_target(v, m));
    assert_eq!(target, Some(p));
    fx.assert_clean();
}

#[test]
fn a_failed_type_13_compute_retries_with_type_15() {
    // A target in no room: the type-13 compute gives 0 points
    // and the set-up retries with type 15, which fails the same way
    // (an open target keeps 13, test above).
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    change(&mut fx, m, WALK, ModeTarget::Point(26, 60));
    let d = dynamic(&mut fx, m);
    assert_eq!((d.path_type, d.point_count), (path_types::WALL_FOLLOW, 0));
    fx.assert_clean();
}

#[test]
fn path_blocked_reads_path_flag_0x800() {
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    let blocked = |fx: &mut Fx| {
        fx.sim
            .with(&mut fx.game, |_, v| AiModes::path_blocked(v, m))
    };
    assert!(!blocked(&mut fx));
    fx.sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(m)
        .unwrap()
        .flags |= 0x800;
    assert!(blocked(&mut fx));
}

#[test]
fn an_attack_end_requests_neutral() {
    // Event 1 of A1 is the mode end: not an inline-think
    // mode, so a mode change to neutral (which schedules the think at
    // f + aidel).
    let mut fx = fx();
    with_anims(&mut fx, &[(4, 8)]);
    fx.sim.sys.data.monsters[0].moves = 0;
    let m = monster(&mut fx, 26, 10);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 28, 10);
    change(&mut fx, m, ATTACK1, ModeTarget::Unit(p));
    // The attack start sets mode 4.
    assert_eq!(mode(&fx, m), 4);
    fx.sim.with(&mut fx.game, |g, v| {
        let mut sim = crate::units::hooks::Sim {
            game: g,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        crate::units::modes::monster_event(&mut sim, &mut *v.h, m, true).unwrap();
    });
    assert_eq!(mode(&fx, m), 1);
    let f = fx.game.frame;
    assert!(fx.timers(m).contains(&(event::AI_THINK, f + 15)));
    fx.assert_clean();
}

/// Installs the AI of class 0 (Idle) on `m`.
fn install_ai(fx: &mut Fx, m: UnitId) {
    fx.sim
        .ai(&mut fx.game, |g, cx| {
            cx.store.entry(m).control = Some(crate::monsters::ai::AiControl::default());
            crate::monsters::ai::install(g, cx, m, 0);
        })
        .unwrap();
}

#[test]
fn an_ai_walk_to_a_unit_moves_then_rethinks_at_the_path_end() {
    // The AI's walk to a unit through the real tactic,
    // mode set, every-tick step and mode end: the monster walks toward
    // the player and, the frame its path ends, the inline think runs
    // (Idle: neutral and the next think 200 frames later).
    let mut fx = fx();
    let m = monster(&mut fx, 20, 30);
    install_ai(&mut fx, m);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 30, 30);
    let ok = fx
        .sim
        .ai(&mut fx.game, |g, cx| {
            crate::monsters::ai::walk_to(g, cx, m, Some(p), 0)
        })
        .unwrap();
    assert!(ok);
    assert_eq!(mode(&fx, m), 2);
    let t = ticks(&mut fx, m, 2, 60);
    // Seven sub-tiles at 0x6000 a tick: the 19th tick lands on (27, 30)
    // and stops; the think runs in that tick (f + 200, Idle).
    assert_eq!(t.len(), 19);
    for w in t.windows(2).take(17) {
        assert_eq!(w[1].0 - w[0].0, 0x6000);
    }
    assert_eq!(t[18], (0x1B8000, 0x1E8000, 1));
    assert_eq!(fx.sim.hooks().path_position(m), (27, 30));
    let f = fx.game.frame;
    assert!(fx.timers(m).contains(&(event::AI_THINK, f + 200)));
    fx.assert_clean();
}

#[test]
fn zigzag_init_rebuilds_a_charged_bolt_path_on_the_provider() {
    // The `zigzag` init callback on the
    // wired seams: seed := init_low(target x), type 10, step counts :=
    // min(total 50, 77), rebuild: n = 50 >> 1 = 25 draws, 26 points from
    // the start cell, each two sub-tiles in one of 8 directions.
    let fire_zig = |tx: i32| {
        let mut fx = fx();
        let a = fx.a;
        let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
        let p = MissileParams {
            owner: Some(owner),
            origin: Some(owner),
            class: 0,
            flags: param_flags::TARGET_ABSOLUTE,
            target_x: tx,
            target_y: 30,
            init: Some((crate::missiles::bodies_ext::ZIGZAG_CALLBACK, 0)),
            ..MissileParams::default()
        };
        let m = fx
            .sim
            .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
            .unwrap()
            .expect("created");
        fx.assert_clean();
        let d = dynamic(&mut fx, m);
        let seed = fx.sim.sys.units.get(m).unwrap().seed;
        (d, seed)
    };
    let (d, seed) = fire_zig(79);
    assert_eq!(d.path_type, 10);
    assert_eq!((d.dist_budget, d.max_distance), (50, 50));
    assert_eq!(d.point_count, 26);
    assert_eq!((d.point(0).x, d.point(0).y), (41, 30));
    for k in 1..26 {
        let (p, q) = (d.point(k - 1), d.point(k));
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        assert!(
            matches!(dx, -2 | 0 | 2) && matches!(dy, -2 | 0 | 2) && (dx, dy) != (0, 0),
            "point {k}: ({dx}, {dy})"
        );
    }
    let mut want = crate::rng::Seed::init_low(79);
    for _ in 0..25 {
        want.step();
    }
    assert_eq!(seed, want);
    // Another target x re-seeds differently.
    let (d2, _) = fire_zig(78);
    assert_ne!(
        (0..26).map(|k| d.point(k)).collect::<Vec<_>>(),
        (0..26).map(|k| d2.point(k)).collect::<Vec<_>>()
    );
}

#[test]
fn a_charged_bolt_path_snaps_to_its_start_point_on_the_first_step() {
    // Point 0 is the
    // start cell, so the first step snaps onto it (Δ = (0, 0)), takes
    // index 1 and aims at point 1; the next step moves toward point 1
    // (a Druid tornado moves this way on its first two frames).
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: 79,
        target_y: 30,
        init: Some((crate::missiles::bodies_ext::ZIGZAG_CALLBACK, 0)),
        ..MissileParams::default()
    };
    let m = fx
        .sim
        .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
        .unwrap()
        .expect("created");
    let start = dynamic(&mut fx, m);
    assert_eq!(start.cur_point, 0);
    fx.tick();
    let d = dynamic(&mut fx, m);
    assert_eq!(
        (d.precise_x, d.precise_y),
        (start.precise_x, start.precise_y)
    );
    assert_eq!(d.cur_point, 1);
    let p1 = d.point(1);
    fx.tick();
    let e = dynamic(&mut fx, m);
    let toward = |from: u32, to: i32| (i64::from(to) * 0x10000 + 0x8000 - i64::from(from)).signum();
    assert_eq!(
        (
            (i64::from(e.precise_x) - i64::from(d.precise_x)).signum(),
            (i64::from(e.precise_y) - i64::from(d.precise_y)).signum()
        ),
        (toward(d.precise_x, p1.x), toward(d.precise_y, p1.y))
    );
    fx.assert_clean();
}

#[test]
fn a_straight_missile_keeps_its_direction_past_the_aim_point() {
    // Path type 4 never snaps onto its point: the Gloam's bolt (class 320)
    // in `diff-a4-nm-unique` flies on past the aim point for several frames.
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let m = fire(&mut fx, owner, 42, 30);
    let mut far = 0u32;
    for _ in 0..40 {
        fx.tick();
        let live = fx.sim.hooks().paths.as_ref().unwrap().dynamic(m);
        match live {
            Some(d) => far = far.max(d.precise_x),
            None => break,
        }
    }
    assert!(i64::from(far) > 43 * 0x10000, "it must pass the aim point");
}

#[test]
fn missile_body_teleport_moves_the_path_and_clears_the_points() {
    let mut fx = fx();
    let (a, b) = (fx.a, fx.b);
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let m = fire(&mut fx, owner, 79, 30);
    assert_eq!(dynamic(&mut fx, m).point_count, 1);
    fx.sim.with(&mut fx.game, |g, v| {
        crate::missiles::MissileBodies::path_teleport(v, g, m, Some(b), 50, 32)
    });
    let d = dynamic(&mut fx, m);
    assert_eq!((d.x(), d.y(), d.room, d.point_count), (50, 32, Some(b), 0));
    let target = fx.sim.with(&mut fx.game, |_, v| {
        crate::missiles::MissileBodies::path_target_point(v, m)
    });
    assert_eq!(target, (79, 30));
    fx.assert_clean();
}

#[test]
fn every_request_but_get_hit_refills_the_budget_and_retargets() {
    // GH (mode 3) changes neither the target nor the budget;
    // any other mode, moving or not, sets the target and 20.
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 28, 10);
    change(&mut fx, m, BLOCK, ModeTarget::Unit(p));
    let drain = |fx: &mut Fx| {
        let d = fx
            .sim
            .hooks()
            .paths
            .as_mut()
            .unwrap()
            .dynamic_mut(m)
            .unwrap();
        d.repath_budget = 7;
    };
    drain(&mut fx);
    change(&mut fx, m, 3, ModeTarget::Point(40, 40));
    let d = dynamic(&mut fx, m);
    assert_eq!(d.repath_budget, 7);
    assert_eq!(d.target_unit.map(|t| t.unit), Some(p));
    // Neutral (not moving): the point target and the budget again.
    change(&mut fx, m, 1, ModeTarget::Point(27, 11));
    let d = dynamic(&mut fx, m);
    assert_eq!(d.repath_budget, 20);
    assert_eq!((d.target_x, d.target_y, d.target_unit), (27, 11, None));
}

#[test]
fn step_counts_use_the_low_byte_capped_at_77() {
    use crate::wiring::path::missiles::step_count_byte;
    // Stop distance: −1 → 255 → 77, −256 → 0, −200 → 56; 300 → 44 (low
    // byte); 77 and 50 as given.
    for (n, b) in [
        (-1, 77),
        (-256, 0),
        (-200, 56),
        (300, 44),
        (77, 77),
        (50, 50),
        (78, 77),
    ] {
        assert_eq!(step_count_byte(n), b, "n {n}");
    }
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let m = fire(&mut fx, owner, 79, 30);
    fx.sim.with(&mut fx.game, |_, v| {
        crate::missiles::MissileBodies::set_path_distance(v, m, -200)
    });
    let d = dynamic(&mut fx, m);
    assert_eq!((d.dist_budget, d.max_distance), (56, 56));
}

#[test]
fn target_position_clears_a_stale_target_and_reads_the_point() {
    use crate::path::TargetUnit;
    let mut fx = fx();
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 41, 30);
    let p = fx.spawn(UnitType::Player, 0, a, 50, 33);
    let m = fire(&mut fx, owner, 79, 30);
    let guid = fx.game.lists.unit(p).unwrap().guid;
    let set_target = |fx: &mut Fx, g: u32| {
        fx.sim
            .hooks()
            .paths
            .as_mut()
            .unwrap()
            .dynamic_mut(m)
            .unwrap()
            .target_unit = Some(TargetUnit {
            unit: p,
            ty: UnitType::Player,
            guid: g,
        });
    };
    let pos = |fx: &mut Fx| {
        fx.sim.with(&mut fx.game, |g, v| {
            crate::missiles::MissileBodies::target_position(v, g, m)
        })
    };
    // A live target: its position, the target kept.
    set_target(&mut fx, guid);
    assert_eq!(pos(&mut fx), Some((50, 33)));
    assert!(dynamic(&mut fx, m).target_unit.is_some());
    // A stale one (its GUID no longer resolves to it): cleared, and the
    // stored point (79, 30) is read instead.
    set_target(&mut fx, guid.wrapping_add(1000));
    assert_eq!(pos(&mut fx), Some((79, 30)));
    assert!(dynamic(&mut fx, m).target_unit.is_none());
    // A point with x = 0: result 0.
    fx.sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(m)
        .unwrap()
        .target_x = 0;
    assert_eq!(pos(&mut fx), None);
}

// ---- monster mode start and event functions ------------------------------

/// Runs the monster's event 0 (`false`) or event 1 (`true`) function.
fn mode_event(fx: &mut Fx, m: UnitId, end: bool) {
    fx.sim.with(&mut fx.game, |g, v| {
        let mut sim = crate::units::hooks::Sim {
            game: g,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        crate::units::modes::monster_event(&mut sim, &mut *v.h, m, end).unwrap();
    });
}

fn log(fx: &mut Fx) -> Vec<String> {
    std::mem::take(&mut fx.sim.hooks().x.log)
}

#[test]
fn a_walk_start_without_a_point_falls_into_neutral() {
    // Step 5: the compute (type 13, then 15) finds no point for a target
    // in no room → the WL start returns 0 → the neutral start: mode 1 and
    // the think at f + aidel (0 → 15). An open target enters mode 2
    // (`monster_walks_to_a_point_sub_tile_by_sub_tile`).
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    let f = fx.game.frame;
    change(&mut fx, m, WALK, ModeTarget::Point(26, 60));
    assert_eq!(dynamic(&mut fx, m).point_count, 0);
    assert_eq!(mode(&fx, m), 1);
    assert!(fx.timers(m).contains(&(event::AI_THINK, f + 15)));
    fx.assert_clean();
}

#[test]
fn a_run_moves_on_its_event_0_and_ends_at_the_point() {
    // RN start with a point → mode 15; the RN event 0
    // steps every tick (WL's body without the state calls)
    // and its stop runs the mode end. Velocity 0x600 (100 %, no run stat
    // list for a monster): the walk's 14 ticks to (31, 10).
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    assert!(change(&mut fx, m, 15, ModeTarget::Point(31, 10)));
    assert_eq!(mode(&fx, m), 15);
    let t = ticks(&mut fx, m, 15, 20);
    assert_eq!(t.len(), 14);
    assert_eq!(t[0].0, 0x1AE000);
    assert_eq!(t[13], (0x1F8000, 0xA8000, 1));
    assert_eq!(fx.sim.hooks().path_position(m), (31, 10));
    fx.assert_clean();
}

#[test]
fn the_gethit_start_sets_mode_3_unless_dead_or_missing() {
    let mut fx = fx();
    with_anims(&mut fx, &[(3, 8)]);
    let m = monster(&mut fx, 26, 10);
    change(&mut fx, m, 3, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 3);
    // In mode 12 (or 0): 1, mode unchanged.
    fx.sim.sys.units.get_mut(m).unwrap().mode = 12;
    change(&mut fx, m, 3, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 12);
    // A class without mode 3: 0 → neutral.
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    fx.sim.hooks().x.missing_modes = vec![3];
    let f = fx.game.frame;
    change(&mut fx, m, 3, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 1);
    assert!(fx.timers(m).contains(&(event::AI_THINK, f + 15)));
    fx.assert_clean();
}

#[test]
fn the_attack_start_sets_its_mode_and_starts_the_used_skill() {
    let mut fx = fx();
    with_anims(&mut fx, &[(4, 8)]);
    // The fixture's A1 moves (monstats2 `A1mv`); not here.
    fx.sim.sys.data.monsters[0].moves = 0;
    let m = monster(&mut fx, 26, 10);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 28, 10);
    // A non-moving A1 without a used skill: mode 4, no skill start.
    change(&mut fx, m, ATTACK1, ModeTarget::Unit(p));
    assert_eq!(mode(&fx, m), 4);
    assert!(!log(&mut fx).iter().any(|l| l.starts_with("skill start")));
    // With a used skill: the skill start runs (its result ignored: 0).
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    fx.sim
        .hooks()
        .x
        .used
        .insert(m, crate::skills::SkillEntry::default());
    change(&mut fx, m, ATTACK1, ModeTarget::Unit(p));
    assert_eq!(mode(&fx, m), 4);
    assert_eq!(log(&mut fx), [format!("skill start {}", m.0)]);
    fx.assert_clean();
}

#[test]
fn a_moving_attack_without_a_point_fails_except_for_vultures() {
    // monstats2 `A1mv`: A1 moves. A target in no room gives no point:
    // the start returns 0 → neutral; BaseId 110 (vulture1) sets the mode
    // with no skill start. An open target computes again and sets
    // the mode with the skill start.
    let setup = |base: u16| {
        let mut fx = fx();
        assert_ne!(fx.sim.sys.data.monsters[0].moves & 1 << 4, 0);
        Arc::make_mut(&mut fx.sim.hooks().tables).combat.monstats[0].baseid = base;
        let m = monster(&mut fx, 26, 10);
        fx.sim
            .hooks()
            .x
            .used
            .insert(m, crate::skills::SkillEntry::default());
        (fx, m)
    };
    let (mut fx, m) = setup(0);
    change(&mut fx, m, ATTACK1, ModeTarget::Point(26, 60));
    assert_eq!(mode(&fx, m), 1);
    assert!(log(&mut fx).is_empty());
    let (mut fx, m) = setup(110);
    change(&mut fx, m, ATTACK1, ModeTarget::Point(26, 60));
    assert_eq!(mode(&fx, m), 4);
    assert!(log(&mut fx).is_empty());
    let (mut fx, m) = setup(0);
    change(&mut fx, m, ATTACK1, ModeTarget::Point(31, 10));
    assert_eq!(mode(&fx, m), 4);
    assert!(dynamic(&mut fx, m).point_count >= 1);
    assert_eq!(log(&mut fx), [format!("skill start {}", m.0)]);
    fx.assert_clean();
}

#[test]
fn block_s3_and_s4_starts() {
    let mut fx = fx();
    with_anims(&mut fx, &[(6, 8)]);
    let m = monster(&mut fx, 26, 10);
    change(&mut fx, m, BLOCK, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 6);
    // S3: mode 10, no schedule; its event 0 steps, refreshes and, the
    // animation complete (no record: 0 frames), sets mode 11.
    change(&mut fx, m, 10, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 10);
    assert!(!fx.timers(m).iter().any(|t| t.0 == event::MODE_CHANGE));
    let r = fx.sim.sys.units.get_mut(m).unwrap();
    // Two speeds short of the end: the refresh advances one, and the
    // animation is then complete (frame + speed ≥ count).
    r.anim.frame = r.anim.frame_count - 2 * i32::from(r.anim.speed);
    mode_event(&mut fx, m, false);
    assert_eq!(mode(&fx, m), 11);
    // S4: mode 11 and the think at f + 15.
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    fx.game.frame += 3;
    let f = fx.game.frame;
    change(&mut fx, m, 11, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 11);
    assert!(fx.timers(m).contains(&(event::AI_THINK, f + 15)));
    fx.assert_clean();
}

#[test]
fn knockback_start_event_and_end() {
    // Step 9: path type 8, distance budget 5 (10 for BaseId 78), compute,
    // mode 13. Step 13: the KB event 0 steps and, the animation complete
    // (no record), runs KB event 1: the path type reset, then the AI's
    // knockback end: a class with GH and not BaseId 78 → a GH request.
    let mut fx = fx();
    with_anims(&mut fx, &[(3, 8)]);
    let m = monster(&mut fx, 26, 10);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 24, 10);
    change(&mut fx, m, 13, ModeTarget::Unit(p));
    assert_eq!(mode(&fx, m), 13);
    let d = dynamic(&mut fx, m);
    assert_eq!(
        (d.path_type, d.dist_budget),
        (path_types::KNOCKBACK_SERVER, 5)
    );
    fx.tick();
    assert_eq!(mode(&fx, m), 3);
    assert_ne!(dynamic(&mut fx, m).path_type, path_types::KNOCKBACK_SERVER);
    fx.assert_clean();
    // BaseId 78: budget 10; its KB end thinks at f + 15 (not stunned).
    let mut fx = fx_paths(true);
    Arc::make_mut(&mut fx.sim.hooks().tables).combat.monstats[0].baseid = 78;
    with_anims(&mut fx, &[(0, 8)]);
    let m = monster(&mut fx, 26, 10);
    change(&mut fx, m, 13, ModeTarget::Point(30, 10));
    assert_eq!(dynamic(&mut fx, m).dist_budget, 10);
    let f = fx.game.frame;
    mode_event(&mut fx, m, true);
    assert!(fx.timers(m).contains(&(event::AI_THINK, f + 15)));
    // In mode 0: 1, unchanged; a class without mode 13: neutral.
    fx.sim.sys.units.get_mut(m).unwrap().mode = 0;
    change(&mut fx, m, 13, ModeTarget::Point(30, 10));
    assert_eq!(mode(&fx, m), 0);
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    fx.sim.hooks().x.missing_modes = vec![13];
    change(&mut fx, m, 13, ModeTarget::Point(30, 10));
    assert_eq!(mode(&fx, m), 1);
    fx.assert_clean();
}

#[test]
fn sequence_start_and_event_0() {
    // Step 10: mode 14, unit flag 0x40 off, the skill start's result: 0
    // → neutral. Step 13: not complete → the skill part, the refresh;
    // complete → the mode end (neutral).
    let mut fx = fx();
    with_anims(&mut fx, &[(14, 8)]);
    let m = monster(&mut fx, 26, 10);
    change(&mut fx, m, 14, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 1);
    assert_eq!(log(&mut fx), [format!("skill start {}", m.0)]);
    fx.sim.hooks().x.skill_start = 1;
    fx.sim.sys.units.get_mut(m).unwrap().flags |= 0x40;
    change(&mut fx, m, 14, ModeTarget::Point(0, 0));
    assert_eq!(mode(&fx, m), 14);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().flags & 0x40, 0);
    log(&mut fx);
    fx.tick();
    assert_eq!(log(&mut fx), [format!("sequence frame {}", m.0)]);
    assert_eq!(mode(&fx, m), 14);
    let r = fx.sim.sys.units.get_mut(m).unwrap();
    r.anim.frame = r.anim.frame_count;
    fx.tick();
    assert!(log(&mut fx).is_empty());
    assert_eq!(mode(&fx, m), 1);
    fx.assert_clean();
}

// ---- AI movement set-up ---------------------------------------------------

fn velocity_request(fx: &mut Fx, m: UnitId) -> crate::monsters::ai::VelocityRequest {
    fx.sim.hooks().ai_store().entry(m).velocity
}

fn set_velocity_request(fx: &mut Fx, m: UnitId, method: i32, speed: i32, steps: i32) {
    fx.sim.hooks().ai_store().entry(m).velocity = crate::monsters::ai::VelocityRequest {
        method,
        speed,
        steps,
    };
}

fn setup_of(fx: &mut Fx, m: UnitId) -> crate::wiring::path::monsters::MoveSetup {
    fx.sim.hooks().paths.as_ref().unwrap().setup[&m]
}

#[test]
fn the_movement_set_up_consumes_the_velocity_request() {
    // Step 4.1: method 7 replaces the path type, speed 3 → P +0x10, steps
    // 9 → the step counts; the request is zeroed.
    // Step 4.4: no target unit → P +0x14 = −1.
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    set_velocity_request(&mut fx, m, 7, 3, 9);
    change(&mut fx, m, WALK, ModeTarget::Point(31, 10));
    let d = dynamic(&mut fx, m);
    assert_eq!(d.path_type, path_types::STRAIGHT);
    assert_eq!((d.dist_budget, d.max_distance), (9, 9));
    assert!(d.point_count >= 1);
    assert_eq!(
        velocity_request(&mut fx, m),
        crate::monsters::ai::VelocityRequest::default()
    );
    let s = setup_of(&mut fx, m);
    assert_eq!((s.speed, s.wait), (3, -1));
    assert_eq!((s.cache_unit, s.cache_x, s.cache_y), (None, 31, 10));
    // No request: type 13 and the default 5 steps; a target unit with a
    // type-13 result → P +0x14 = 10.
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 30, 12);
    change(&mut fx, m, WALK, ModeTarget::Unit(p));
    let d = dynamic(&mut fx, m);
    assert_eq!(d.path_type, path_types::TOWARD_FINISH);
    assert_eq!((d.dist_budget, d.max_distance), (5, 5));
    let s = setup_of(&mut fx, m);
    assert_eq!((s.speed, s.wait), (0, 10));
    assert_eq!(s.cache_unit, Some(p));
    fx.assert_clean();
}

#[test]
fn a_velocity_method_paths_even_a_non_moving_request_and_gh_keeps_it() {
    // Step 4.1: the method replaces a 100 byte too: BL with method 7
    // computes a straight path. Step 1: GH consumes nothing.
    let mut fx = fx();
    with_anims(&mut fx, &[(3, 8), (6, 8)]);
    let m = monster(&mut fx, 26, 10);
    set_velocity_request(&mut fx, m, 7, 0, 0);
    change(&mut fx, m, 3, ModeTarget::Point(31, 10));
    assert_eq!(velocity_request(&mut fx, m).method, 7);
    change(&mut fx, m, BLOCK, ModeTarget::Point(31, 10));
    let d = dynamic(&mut fx, m);
    assert_eq!(d.path_type, path_types::STRAIGHT);
    assert!(d.point_count >= 1);
    assert_eq!(velocity_request(&mut fx, m).method, 0);
    fx.assert_clean();
}

#[test]
fn an_ai_request_inside_a_think_consumes_the_velocity_request() {
    // The AI store is lent out during AI code: the request reaches the
    // set-up through the staged copy (`request_mode`) and is zeroed in
    // the store.
    let mut fx = fx();
    let m = monster(&mut fx, 20, 30);
    install_ai(&mut fx, m);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 30, 30);
    set_velocity_request(&mut fx, m, 7, 0, 12);
    fx.sim
        .ai(&mut fx.game, |g, cx| {
            crate::monsters::ai::walk_to(g, cx, m, Some(p), 0)
        })
        .unwrap();
    let d = dynamic(&mut fx, m);
    assert_eq!(d.path_type, path_types::STRAIGHT);
    assert_eq!((d.dist_budget, d.max_distance), (12, 12));
    assert_eq!(
        velocity_request(&mut fx, m),
        crate::monsters::ai::VelocityRequest::default()
    );
    fx.assert_clean();
}

#[test]
fn a_mode_set_without_a_record_target_aims_at_0_0() {
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 28, 10);
    change(&mut fx, m, 1, ModeTarget::Unit(p));
    assert_eq!(dynamic(&mut fx, m).target_unit.map(|t| t.unit), Some(p));
    fx.sim.sys.with(&mut fx.game, |sim, hooks| {
        crate::units::modes::monster_set_mode(sim, hooks, m, 1).unwrap()
    });
    let d = dynamic(&mut fx, m);
    assert_eq!((d.target_x, d.target_y, d.target_unit), (0, 0, None));
    fx.assert_clean();
}

#[test]
fn path_step_count_is_the_stop_distance_and_stop_path_clears_the_points() {
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    let stop = |fx: &mut Fx, n: i32| {
        fx.sim
            .with(&mut fx.game, |_, v| AiModes::set_path_steps(v, m, n));
        dynamic(fx, m).stop_distance
    };
    assert_eq!(stop(&mut fx, 1), 0);
    assert_eq!(stop(&mut fx, 5), 4);
    assert_eq!(stop(&mut fx, 19), 18);
    assert_eq!(stop(&mut fx, 20), 0);
    assert_eq!(stop(&mut fx, -3), 0);
    change(&mut fx, m, WALK, ModeTarget::Point(31, 10));
    let b = dynamic(&mut fx, m);
    assert!(b.point_count >= 1);
    fx.sim.with(&mut fx.game, |_, v| AiModes::stop_path(v, m));
    let d = dynamic(&mut fx, m);
    assert_eq!((d.point_count, d.flags & flags::ACTIVE), (0, 0));
    assert_eq!(
        (d.target_x, d.target_y, d.path_type, d.repath_budget),
        (b.target_x, b.target_y, b.path_type, b.repath_budget)
    );
}

#[test]
fn the_nearest_client_player_within_15_is_found() {
    use crate::monsters::ai::seams::{AiTargets, AiUnits as _};
    let mut fx = fx();
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 20, 10);
    // Every monster has an interaction block.
    assert!(fx.sim.with(&mut fx.game, |_, v| v.has_interaction_block(m)));
    // The full-size distance subtracts the NPC's size per
    // axis (clamped at 0).
    let s = fx.sim.with(&mut fx.game, |_, v| v.size(m));
    assert!((1..=3).contains(&s), "size {s}");
    let far = fx.spawn(UnitType::Player, 0, a, 20 + 16 + s, 10);
    fx.game.lists.add_client(Some(far), Some(a), 0);
    // Full-size distance 16: none, the unit itself.
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (m, false));
    let edge = fx.spawn(UnitType::Player, 0, a, 20, 10 + 15 + s);
    fx.game.lists.add_client(Some(edge), Some(a), 0);
    // Full-size distance 15: found, not "close".
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (edge, false));
    // A player unit without a client is not scanned.
    let _ = fx.spawn(UnitType::Player, 0, a, 21, 10);
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (edge, false));
}

/// Two client players near monster `m` of `fx` at full-size distances
/// `first` and `second`, `first` earlier in scan order (room A's list).
fn two_players(fx: &mut Fx, m: UnitId, first: i32, second: i32) -> (UnitId, UnitId) {
    use crate::monsters::ai::seams::AiUnits as _;
    let a = fx.a;
    let s = fx.sim.with(&mut fx.game, |_, v| v.size(m));
    // The room's unit list is prepended to: the
    // later spawn comes first in scan order.
    let q = fx.spawn(UnitType::Player, 0, a, 20 - second - s, 10);
    let p = fx.spawn(UnitType::Player, 0, a, 20 + first + s, 10);
    fx.game.lists.add_client(Some(q), Some(a), 0);
    fx.game.lists.add_client(Some(p), Some(a), 0);
    let order: Vec<UnitId> = fx
        .game
        .lists
        .room_units(a)
        .into_iter()
        .filter(|&u| u == p || u == q)
        .collect();
    assert_eq!(order, [p, q], "scan order");
    (p, q)
}

// The NPC's scan callback takes the first qualifying player in scan
// order (taking stops the scan), not the nearest.
#[test]
fn a_non_interact_npc_takes_the_first_player_in_scan_order() {
    use crate::monsters::ai::seams::AiTargets;
    let mut fx = fx();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 20, 10);
    let (first, _) = two_players(&mut fx, m, 10, 3);
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    fx.sim.sys.hooks.quest_host = Some(Box::new(Gate {
        pass: Vec::new(),
        calls: calls.clone(),
    }));
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (first, false));
    // No interact flag: the quest test is not called.
    assert!(calls.borrow().is_empty());
}

/// The lent quest host of the interact-gate tests: the active test is
/// true for the players in `pass` (and then "sends" 8A 01 <npc GUID>
/// to `TestPending::sent`); every call is logged.
struct Gate {
    pass: Vec<UnitId>,
    calls: std::rc::Rc<std::cell::RefCell<Vec<(UnitId, u16, bool)>>>,
}

impl crate::wiring::action::QuestObjectHost<crate::wiring::action::tests::TestPending> for Gate {
    fn run(
        &mut self,
        _: &mut crate::game::Game,
        _: &mut crate::wiring::action::View<'_, crate::wiring::action::tests::TestPending>,
        _: crate::wiring::action::QuestObjectCall,
    ) -> Option<crate::wiring::action::ObjectRoute> {
        None
    }
    fn npc_wants_interact(
        &mut self,
        game: &mut crate::game::Game,
        v: &mut crate::wiring::action::View<'_, crate::wiring::action::tests::TestPending>,
        player: UnitId,
        npc: UnitId,
        class: u16,
        interact: bool,
    ) -> bool {
        self.calls.borrow_mut().push((player, class, interact));
        if !self.pass.contains(&player) {
            return false;
        }
        let guid = game.lists.unit(npc).expect("npc").guid;
        let mut m = vec![0x8A, 1];
        m.extend_from_slice(&guid.to_le_bytes());
        v.h.x.sent.push((player, m));
        true
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// [`fx`] with monster class 0 an `interact` NPC (monstats flag bit 9).
fn fx_interact() -> Fx {
    let mut fx = fx();
    Arc::make_mut(&mut fx.sim.hooks().tables).combat.monstats[0].interact = true;
    fx
}

// The interact gate (AI scan 2): an
// `interact` NPC takes the first player within 15 for whom the quest
// active test is true; each true call sends 0x8A; none: the NPC itself.
#[test]
fn an_interact_npc_takes_the_first_player_the_quest_test_passes() {
    use crate::monsters::ai::seams::AiTargets;
    // A player at distance 10 passing the test: taken, one 8A.
    let mut fx = fx_interact();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 20, 10);
    let (first, second) = two_players(&mut fx, m, 10, 3);
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    fx.sim.sys.hooks.quest_host = Some(Box::new(Gate {
        pass: vec![first],
        calls: calls.clone(),
    }));
    let guid = fx.game.lists.unit(m).unwrap().guid;
    let mut want = vec![0x8A, 1];
    want.extend_from_slice(&guid.to_le_bytes());
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (first, false));
    assert_eq!(*calls.borrow(), [(first, 0, true)]);
    assert_eq!(fx.sim.hooks().x.sent, [(first, want.clone())]);
    // Neither passes: the NPC itself, both tested in scan order, no 8A.
    calls.borrow_mut().clear();
    fx.sim.hooks().x.sent.clear();
    fx.sim.sys.hooks.quest_host = Some(Box::new(Gate {
        pass: Vec::new(),
        calls: calls.clone(),
    }));
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (m, false));
    assert_eq!(*calls.borrow(), [(first, 0, true), (second, 0, true)]);
    assert!(fx.sim.hooks().x.sent.is_empty());
    // Both pass: the first in scan order is taken, 8A only to it (the
    // nearer second one, distance 3, is never tested).
    calls.borrow_mut().clear();
    fx.sim.sys.hooks.quest_host = Some(Box::new(Gate {
        pass: vec![first, second],
        calls: calls.clone(),
    }));
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (first, false));
    assert_eq!(*calls.borrow(), [(first, 0, true)]);
    assert_eq!(fx.sim.hooks().x.sent, [(first, want)]);
    // Only the second passes: taken, "close" (distance 3 < 4).
    calls.borrow_mut().clear();
    fx.sim.hooks().x.sent.clear();
    fx.sim.sys.hooks.quest_host = Some(Box::new(Gate {
        pass: vec![second],
        calls: calls.clone(),
    }));
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (second, true));
    assert_eq!(fx.sim.hooks().x.sent.len(), 1);
    // No lent quest control: the test is false, the NPC itself.
    fx.sim.sys.hooks.quest_host = None;
    let got = fx.sim.with(&mut fx.game, |g, v| v.nearest_player(g, m));
    assert_eq!(got, (m, false));
}

#[test]
fn the_speed_bonus_is_a_4004_list_with_stat_67_set_to_the_request_speed() {
    // Speed bonus: v = 50 → a new list of flags 0x4004 (TEMPONLY) on
    // the unit, stat 67 := 50; v = 0 → no list.
    use crate::stats::lists::flag;
    let mut fx = fx();
    let m = monster(&mut fx, 26, 10);
    let bonus_lists = |fx: &Fx| -> Vec<(u32, i32)> {
        let st = &fx.sim.sys.stats;
        st.unit_list(m)
            .map(|root| st.active_chain(root))
            .unwrap_or_default()
            .into_iter()
            .filter(|&l| st.flags(l) & 0x4000 != 0)
            .map(|l| (st.flags(l) & (0x4000 | flag::TEMPONLY), st.base(l, 67, 0)))
            .collect()
    };
    change(&mut fx, m, WALK, ModeTarget::Point(31, 10));
    assert_eq!(bonus_lists(&fx), []);
    set_velocity_request(&mut fx, m, 7, 50, 9);
    change(&mut fx, m, WALK, ModeTarget::Point(36, 10));
    assert_eq!(bonus_lists(&fx), [(0x4004, 50)]);
    fx.assert_clean();
}

#[test]
fn a_plain_mode_set_drops_the_sequence_and_takes_the_record_count() {
    // The animation setup, a mode other than the sequence mode: the sequence record
    // := null, the rate, then the frame count := the new record's frames · 256,
    // with no animated start run.
    use crate::units::hooks::Sim;
    use crate::units::modes;
    use crate::units::record::Sequence;
    let mut fx = fx();
    with_anims(&mut fx, &[(1, 8), (2, 12)]);
    let m = monster(&mut fx, 26, 10);
    let seq = Sequence {
        frame_count: 0x900,
        speed: 0x100,
        pos: 0,
        events: vec![0; 9],
        drawn: Vec::new(),
    };
    fx.sim.sys.units.get_mut(m).unwrap().anim.sequence = Some(seq);
    let s = &mut fx.sim.sys;
    let mut sim = Sim {
        game: &mut fx.game,
        units: &mut s.units,
        stats: &mut s.stats,
        data: &s.data,
    };
    modes::set_mode(&mut sim, &mut s.hooks, m, 2).unwrap();
    let a = &fx.sim.sys.units.get(m).unwrap().anim;
    assert!(a.sequence.is_none());
    assert_eq!(a.frame_count, 12 * 256);
    assert_eq!(a.record.map(|r| r.frames), Some(12));
    assert_eq!(a.action_frame, 0);
}

#[test]
fn a_dropped_item_stamps_its_footprint_and_the_pickup_frees_it() {
    // The drop stamps the size-1, mask-0x200 box
    // at the spot; the pickup's collision free
    // clears it, so the next drop elsewhere leaves no bit.
    use crate::drlg::collision::bits;
    use crate::units::lifecycle::LifecycleHooks;
    let mut fx = fx();
    let a = fx.a;
    let item = fx.spawn(UnitType::Item, 0, a, 20, 10);
    let cell = |fx: &mut Fx, x, y| {
        crate::path::collision::point_value(&fx.sim.hooks().drlg, Some(a), x, y, 0xFFFF)
            & bits::ITEM
    };
    fx.sim.hooks().ground_item_placed(item, a, 22, 12);
    assert_ne!(cell(&mut fx, 22, 12), 0);
    fx.sim.hooks().ground_item_lifted(item);
    assert_eq!(cell(&mut fx, 22, 12), 0);
    fx.sim.hooks().ground_item_placed(item, a, 24, 12);
    assert_eq!(cell(&mut fx, 22, 12), 0);
    assert_ne!(cell(&mut fx, 24, 12), 0);
}
