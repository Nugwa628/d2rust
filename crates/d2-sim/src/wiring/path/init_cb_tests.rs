//! Tests for the skills code's missile init callbacks, which missile
//! creation runs last, on the wired units and the path provider: frames
//! capped at 77, the missile seed re-initialised and drawn in the
//! callbacks' order, and the path each one leaves.

use crate::drlg::TileRect;
use crate::missiles::{create_missile, param_flags, stat, MissileParams};
use crate::path::DynamicPath;
use crate::rng::Seed;
use crate::skills::use_::bodies::b4_helpers::{dir8, ZIG_X, ZIG_Y};
use crate::skills::use_::bodies::init_cb;
use crate::units::{UnitId, UnitType};
use crate::wiring::action::tests::{Fx, LEVEL};

/// The owner's sub-tile and the target's y (east of the owner).
const OX: i32 = 41;
const OY: i32 = 30;

fn fx(on: bool) -> Fx {
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
    ]);
    if on {
        fx.sim.hooks().enable_paths().expect("embedded tables");
    }
    fx
}

/// Creates the action fixture's arrow (class 0, range 50) from (41, 30)
/// toward (tx, 30) with `range` frames (none: the row's) and the init
/// callback; the missile's path, seed and data frames.
fn fire(
    fx: &mut Fx,
    tx: i32,
    range: Option<i32>,
    init: (u32, u32),
) -> (UnitId, Option<DynamicPath>, Seed, (i16, i16)) {
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, OX, OY);
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE | range.map_or(0, |_| param_flags::RANGE),
        target_x: tx,
        target_y: OY,
        range: range.unwrap_or(0),
        init: Some(init),
        ..MissileParams::default()
    };
    let (m, frames) = fx
        .sim
        .missiles(&mut fx.game, |g, cx| {
            let m = create_missile(g, cx, &p)?;
            let d = cx.store.get(m)?;
            Some((m, (d.total, d.current)))
        })
        .unwrap()
        .expect("created");
    fx.assert_clean();
    let d = fx
        .sim
        .hooks()
        .paths
        .as_ref()
        .and_then(|p| p.dynamic(m).cloned());
    let seed = fx.sim.sys.units.get(m).unwrap().seed;
    (m, d, seed, frames)
}

fn stepped(mut s: Seed, n: usize) -> Seed {
    for _ in 0..n {
        s.step();
    }
    s
}

fn points(d: &DynamicPath) -> Vec<(i32, i32)> {
    (0..d.point_count as usize)
        .map(|k| (d.point(k).x, d.point(k).y))
        .collect()
}

#[test]
fn jitter_caps_frames_reseeds_from_the_target_x_and_builds_a_charged_bolt() {
    // Range 100: frames capped at 77 (total and left); seed :=
    // init_low(target x 79 + a 3); type 10, step counts 77; the
    // charged-bolt compute draws n = 77 >> 1 = 38 times and leaves 39
    // points from the start cell.
    let mut f = fx(true);
    let (_, d, seed, frames) = fire(&mut f, 79, Some(100), (init_cb::JITTER, 3));
    let d = d.unwrap();
    assert_eq!(frames, (77, 77));
    assert_eq!(d.path_type, 10);
    assert_eq!((d.dist_budget, d.max_distance), (77, 77));
    assert_eq!(d.point_count, 39);
    assert_eq!(seed, stepped(Seed::init_low(82), 38));
    // Under the cap the frames stay (the row's 50) and the steps follow.
    let mut f = fx(true);
    let (_, d, seed, frames) = fire(&mut f, 79, None, (init_cb::JITTER, 3));
    let d = d.unwrap();
    assert_eq!(frames, (50, 50));
    assert_eq!((d.dist_budget, d.point_count), (50, 26));
    assert_eq!(seed, stepped(Seed::init_low(82), 25));
    // Another argument, another seed and another path.
    let mut f = fx(true);
    let (_, d4, _, _) = fire(&mut f, 79, None, (init_cb::JITTER, 4));
    assert_ne!(points(&d), points(&d4.unwrap()));
}

#[test]
fn diab_wall_draws_once_and_jitters_four_in_five() {
    // Seed := init_low(target x + a); r = one draw mod 100; r ≥ 20 →
    // type 10 rebuilt (25 more draws for 50 frames), r < 20 → the
    // straight one-point path of creation stays.
    let r = |a: u32| Seed::init_low(79 + a).step() % 100;
    let straight = (0..64).find(|&a| r(a) < 20).expect("an r < 20");
    let jittered = (0..64).find(|&a| r(a) >= 20).unwrap();
    let mut f = fx(true);
    let (_, d, seed, _) = fire(&mut f, 79, None, (init_cb::DIAB_WALL, straight));
    let d = d.unwrap();
    assert_eq!((d.point_count, d.dist_budget), (1, 0));
    assert_ne!(d.path_type, 10);
    assert_eq!(seed, stepped(Seed::init_low(79 + straight), 1));
    let mut f = fx(true);
    let (_, d, seed, _) = fire(&mut f, 79, None, (init_cb::DIAB_WALL, jittered));
    let d = d.unwrap();
    assert_eq!((d.path_type, d.dist_budget, d.point_count), (10, 50, 26));
    assert_eq!(seed, stepped(Seed::init_low(79 + jittered), 26));
}

#[test]
fn lightning_fan_writes_its_zigzag_point_by_point() {
    // Seed := init_low(a); d = dir64 toward the target; s, k from two
    // draws; each point two sub-tiles in dir8(d += k·s); every 15th
    // point (i = 0, 15, 30, 45) flips s and draws a new k. 50 frames:
    // 50 points, 2 + 4 draws. The model below restates that rule.
    let a = 0x1234_5678;
    let mut f = fx(true);
    let (m, d, seed, _) = fire(&mut f, 79, None, (init_cb::ZIGZAG, a));
    let d = d.unwrap();
    assert_eq!((d.path_type, d.dist_budget, d.max_distance), (10, 50, 50));
    // East of the owner: the 64-step direction toward the target.
    let dir = f
        .sim
        .with(&mut f.game, |_, v| v.path_dir64(m, (79, OY)))
        .unwrap();
    let mut s = Seed::init_low(a);
    let mut sign = if s.step() & 1 != 0 { 1 } else { -1 };
    let mut k = (s.step() % 3) as i32 + 2;
    let (mut px, mut py, mut dd) = (OX, OY, dir);
    let mut want = Vec::new();
    for i in 0..50 {
        dd += k * sign;
        let e = dir8(dd & !0xC0 & 0xFF) as usize;
        px += ZIG_X[e];
        py += ZIG_Y[e];
        want.push((px, py));
        if i % 15 == 0 {
            sign = -sign;
            k = (s.step() % 3) as i32 + 2;
        }
    }
    assert_eq!(points(&d), want);
    assert_eq!(seed, s);
    assert_eq!(seed, stepped(Seed::init_low(a), 6));
    // Another argument zigzags differently.
    let mut f = fx(true);
    let (_, d2, _, _) = fire(&mut f, 79, None, (init_cb::ZIGZAG, a + 1));
    assert_ne!(points(&d), points(&d2.unwrap()));
}

#[test]
fn dir64_is_the_direction_vector_from_the_missile_sub_tile() {
    // The direction vector on the embedded `tan` table toward the eight
    // neighbours at distance 10: +x 56,
    // +x+y 0, +y 7, −x+y 15, −x 23, −x−y 32, −y 40, +x−y 47; dir8
    // spreads them over the eight 8-way directions.
    let mut f = fx(true);
    let (m, _, _, _) = fire(&mut f, 79, None, (init_cb::JITTER, 0));
    let at = f.sim.hooks().path_position(m);
    let to = [
        (10, 0),
        (10, 10),
        (0, 10),
        (-10, 10),
        (-10, 0),
        (-10, -10),
        (0, -10),
        (10, -10),
    ];
    let dirs: Vec<i32> = to
        .iter()
        .map(|&(dx, dy)| {
            f.sim
                .with(&mut f.game, |_, v| v.path_dir64(m, (at.0 + dx, at.1 + dy)))
                .unwrap()
        })
        .collect();
    assert_eq!(dirs, [56, 0, 7, 15, 23, 32, 40, 47]);
    let mut e: Vec<i32> = dirs.iter().map(|&d| dir8(d)).collect();
    e.sort_unstable();
    assert_eq!(e, (0..8).collect::<Vec<_>>());
    // Scale-free: the same ratio at another distance, the same direction.
    let far = f
        .sim
        .with(&mut f.game, |_, v| v.path_dir64(m, (at.0 + 30, at.1 + 30)))
        .unwrap();
    assert_eq!(far, 0);
}

#[test]
fn lightning_ring_reseeds_from_the_argument_and_computes() {
    // As the jitter with seed := init_low(a) (no target x).
    let mut f = fx(true);
    let (_, d, seed, frames) = fire(&mut f, 79, Some(90), (init_cb::ZIGZAG_RING, 7));
    let d = d.unwrap();
    assert_eq!(frames, (77, 77));
    assert_eq!((d.path_type, d.dist_budget, d.point_count), (10, 77, 39));
    assert_eq!(seed, stepped(Seed::init_low(7), 38));
}

#[test]
fn damage_percent_callback_adds_its_argument() {
    let mut f = fx(true);
    let (m, _, _, _) = fire(&mut f, 79, None, (init_cb::DAMAGE_PERCENT, 37));
    assert_eq!(f.stat(m, stat::DAMAGEPERCENT), 37);
    let mut f = fx(true);
    let (m, _, _, _) = fire(&mut f, 79, None, (init_cb::DAMAGE_PERCENT, 0));
    assert_eq!(f.stat(m, stat::DAMAGEPERCENT), 0);
}

#[test]
fn without_the_provider_the_frames_cap_and_the_reseed_still_run() {
    // No path provider: the target x reads 0, so seed := init_low(a);
    // the path seams do nothing.
    let mut f = fx(false);
    let (_, d, seed, frames) = fire(&mut f, 79, Some(100), (init_cb::JITTER, 5));
    assert!(d.is_none());
    assert_eq!(frames, (77, 77));
    assert_eq!(seed, Seed::init_low(5));
}
