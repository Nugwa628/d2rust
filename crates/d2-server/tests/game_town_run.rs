//! Integration test of the run in the town on the game's live tables
//! (`#[ignore]`, `D2_GAME_DIR`; part of the game-file tier next to
//! `game_wired_host.rs`): a new
//! paladin in the Rogue Encampment (the wired single-player host of
//! `test_fixtures::host::Session`, the Act I creation seed `INIT`) sends
//! C→S 0x03 to a point 10 sub-tiles east, then 0x01 to the same offset.
//! 12 sub-tiles east (4901, 5634) is a Torch1 Tiki preset (objects.txt
//! class 37: 1 × 1, HasCollision in every mode): the size-2 pattern meets
//! its footprint from x 4900 to 4902, so target preparation steps the
//! request back to the first free cell, 4899,
//! and that run stops 2 short of the request.
//!
//! Expected values: the run is mode 3 with the run stat list's velocity
//! (run speed 100·9/6 − 100 = 50 percent on top of the walk: 0x600 · 150 / 100
//! = 0x900), the town walk (mode 6) is at 0x600;
//! in the town the run drains no stamina. The tick counts
//! are invariants of those velocities: the run reaches the point in at
//! most ⌈10 · 0x10000 / 0x9000⌉ + 1 ticks, fewer than the walk needs.
//! Before the run list was wired the run moved at the walk's velocity, a
//! third slower than the play client's run prediction, so every server
//! position check snapped the client back.

use std::sync::OnceLock;

use d2_data::bin;
use d2_formats::mpq::ArchiveSet;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup, MOVING};

fn live() -> &'static GameData {
    static L: OnceLock<GameData> = OnceLock::new();
    L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bins = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        GameData::load(bins, &set).unwrap_or_else(|e| panic!("game data: {e}"))
    })
}

/// The Act I creation seed and the Rogue Encampment; class 3 (paladin).
const INIT: u32 = 644_409_375;
const TOWN: u32 = 1;
const PALADIN: u32 = 3;
/// Stamina stat id.
const STAT_STAMINA: u16 = 10;
/// The free leg, and the leg whose target is the torch.
const LEG: i32 = 10;
const TORCH_LEG: i32 = 12;

/// One leg: (mode and velocity after the request, ticks until it
/// stopped, the end position, the target, stamina before and after).
type Leg = ((u32, i32), usize, (i32, i32), (i32, i32), (i32, i32));

/// One leg of `id`, `dx` sub-tiles east of the player ([`Leg`]).
fn leg(id: u8, dx: i32) -> Leg {
    let mut fx = Session::new(
        live(),
        &Setup {
            creation: ActCreation::Full,
            init_seed: INIT,
            town: TOWN,
            game_seed: 1234,
            class: PALADIN,
            known_waypoints: Vec::new(),
        },
    );
    for _ in 0..30 {
        fx.frame();
    }
    let p = fx.player;
    let stamina = |fx: &mut Session| {
        fx.sim()
            .events
            .action
            .sys
            .stats
            .unit_total(p, STAT_STAMINA, 0)
    };
    let s0 = stamina(&mut fx);
    let start = fx.pos();
    let to = (start.0 + dx, start.1);
    let mut m = vec![id];
    m.extend_from_slice(&(to.0 as u16).to_le_bytes());
    m.extend_from_slice(&(to.1 as u16).to_le_bytes());
    fx.send(&m);
    let velocity = fx
        .sim()
        .events
        .action
        .hooks()
        .paths
        .as_ref()
        .and_then(|s| s.dynamic(p))
        .map(|d| d.velocity)
        .expect("player path");
    let mode = fx.mode();
    let mut ticks = 1;
    while MOVING.contains(&fx.mode()) && ticks < 100 {
        fx.frame();
        ticks += 1;
    }
    fx.assert_clean("town leg");
    let s1 = stamina(&mut fx);
    ((mode, velocity), ticks, fx.pos(), to, (s0, s1))
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn town_run_moves_at_the_run_velocity() {
    let ((mode, v), run_ticks, end, to, (s0, s1)) = leg(0x03, LEG);
    println!("run: mode {mode} velocity {v:#x}, {run_ticks} ticks, end {end:?} (target {to:?})");
    assert_eq!((mode, v), (3, 0x900), "§1.5 r2 (stamina {s0}), §8.2");
    assert_eq!(end, to, "the run reaches its point");
    assert!(s0 > 0 && s0 == s1, "§9.9 r1: no drain in the town");
    let max = (LEG as usize * 0x10000).div_ceil(0x9000) + 1;
    assert!(run_ticks <= max, "{run_ticks} ticks > {max}");

    let ((mode, v), walk_ticks, end, to, _) = leg(0x01, LEG);
    println!("walk: mode {mode} velocity {v:#x}, {walk_ticks} ticks, end {end:?}");
    assert_eq!((mode, v), (6, 0x600), "§1.5 r2 (town walk), §8.1");
    assert_eq!(end, to, "the walk reaches its point");
    assert!(run_ticks < walk_ticks, "the run is faster than the walk");
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn a_run_onto_the_torch_stops_at_the_first_free_cell() {
    let ((mode, v), _, end, to, _) = leg(0x03, TORCH_LEG);
    println!("run: mode {mode} velocity {v:#x}, end {end:?} (request {to:?})");
    assert_eq!((mode, v), (3, 0x900), "§1.5 r2, §8.2");
    assert_eq!(
        end,
        (to.0 - 2, to.1),
        "§4 r2–3: p0 steps back off the torch"
    );
}
