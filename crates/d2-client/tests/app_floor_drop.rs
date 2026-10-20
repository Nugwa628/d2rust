//! The play game's drop spot: the path provider carries the real
//! `ExpField.D2` walk-back field, so a monster's or chest's drop searches
//! its floor spot instead of taking the start spot as is.
//! In the game, Cold Plains (a known run): a
//! fallen dead at (5170, 4660) drops at (5172, 4663) = (x + 2, y + 3), the
//! next one, dead at (5171, 4659) with its start cell taken, at
//! (5172, 4662).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::bridge::Bridge;
use d2_sim::path::coords::Point;
use d2_sim::wiring::path::place::floor_drop;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The joined play game on the real data: the server link and the local
/// player.
fn joined() -> (app_support::Server<StepClock>, d2_sim::units::UnitId) {
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let server = Arc::new(std::sync::Mutex::new(link));
    let mut bridge = Bridge::new(app_support::SharedLink(server.clone())).unwrap();
    let frame = |b: &mut Bridge<_>| {
        ms.fetch_add(40, Ordering::SeqCst);
        b.frame().unwrap()
    };
    bridge
        .send_bytes(&single_player::create_request().encode())
        .unwrap();
    frame(&mut bridge);
    frame(&mut bridge);
    bridge.send_bytes(&[0x6B]).unwrap();
    for _ in 0..3 {
        frame(&mut bridge);
    }
    let (player, _) = app_support::local_player(&server).expect("joined");
    (server, player)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_game_drops_on_the_floor_drop_spot_of_the_real_field() {
    let (server, player) = joined();
    let r = app_support::with(&server, move |l| {
        let sim = &mut l.host_mut().game;
        let game = &sim.game;
        let room = game.lists.unit(player).and_then(|e| e.room());
        let h = sim.events.action.hooks();
        let field = h
            .paths
            .as_ref()
            .and_then(|p| p.field.clone())
            .expect("the field is on the path provider");
        let at = h.path_position(player);
        let start = Point::new(at.0 + 2, at.1 + 3);
        let free = room
            .and_then(|r| h.drlg.collision(game, r, start.x, start.y))
            .is_some_and(|c| c & 0x3E01 == 0);
        let out = floor_drop(&h.drlg, &field, room, Point::new(at.0, at.1), 1, true).unwrap();
        (field.width, field.height, at, free, out.1)
    });
    let (w, h, at, free, out) = r;
    assert_eq!((w, h), (256, 256));
    assert!(free, "the town cell {:?} + (2, 3) is open", at);
    assert_eq!(out, Point::new(at.0 + 2, at.1 + 3));
}

/// A monster on the start cell (x + 2, y + 3): the game puts the drop one
/// cell west of the start, (x + 1, y + 3). Seen twice: Blood Moor,
/// `traces/checks/combat-kill-fallen.check` frame 36 (fallen dead at
/// (5145, 4265), fallen 1:20 standing on (5147, 4268), gold at
/// (5146, 4268)), and `facts/items/a1-cold-plains-poke-kills.tsv` (dead
/// at (5171, 4659), drop at (5172, 4662)). The ring search
/// visits the west column first and keeps its
/// d = 1 cell. Here on an open town cell with a monster's centre bits on
/// the start cell.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_monster_on_the_start_cell_moves_the_drop_one_cell_west() {
    let (server, player) = joined();
    let r = app_support::with(&server, move |l| {
        let sim = &mut l.host_mut().game;
        let game = &sim.game;
        let room = game.lists.unit(player).and_then(|e| e.room());
        let h = sim.events.action.hooks();
        let field = h.paths.as_ref().and_then(|p| p.field.clone()).unwrap();
        let at = h.path_position(player);
        let r = room.unwrap();
        let open = |x: i32, y: i32| {
            h.drlg
                .collision(game, r, x, y)
                .is_some_and(|c| c & 0x3E01 == 0)
        };
        // An origin near the player whose start has an open 5 × 5 around it.
        let origin = (-8..=8)
            .flat_map(|dy| (-8..=8).map(move |dx| (at.0 + dx, at.1 + dy)))
            .find(|&(x, y)| (-2..=2).all(|j| (-2..=2).all(|i| open(x + 2 + i, y + 3 + j))));
        let Some(o) = origin else {
            return (false, Point::new(at.0, at.1), Point::new(0, 0));
        };
        let start = Point::new(o.0 + 2, o.1 + 3);
        // A monster on the start: its footprint's centre cell (pattern 1,
        // the placement flags: 0x100 and NO_PATH 0x1000, in 0x3E01).
        *h.drlg.collision_mut(game, r, start.x, start.y).unwrap() |= 0x1100;
        let out = floor_drop(&h.drlg, &field, room, Point::new(o.0, o.1), 1, true).unwrap();
        *h.drlg.collision_mut(game, r, start.x, start.y).unwrap() &= !0x1100;
        (true, start, out.1)
    });
    let (open, start, out) = r;
    assert!(open, "an open 5 × 5 near the player at {:?}", start);
    assert_eq!(out, Point::new(start.x - 1, start.y));
}
