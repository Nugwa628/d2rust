//! Tests for the whole tick: `d2_sim::tick::tick` runs the combined dispatcher for a
//! few ticks — a client in game, a monster thinking, its missile flying
//! through the room grid and hitting the player — twice from the same
//! seeds with the same result.

use super::*;
use crate::drlg::collision::bits;
use crate::missiles::{create_missile, param_flags, unit_flag, MissileParams};
use crate::monsters::ai::{install, AiControl};
use crate::stats::stat as st;
use crate::tick::events::event;
use crate::units::lists::client_state;

/// What a run leaves behind: frame, player life, unit seeds, every unit's
/// timers, the game seed, the act's rooms, the adapter log.
type Digest = (
    i32,
    i32,
    Vec<Seed>,
    Vec<Vec<(u8, i32)>>,
    Seed,
    usize,
    Vec<String>,
);

fn run() -> Digest {
    let mut fx = Fx::new();
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 13, 10);
    let m = fx.spawn(UnitType::Monster, 0, a, 10, 10);
    fx.stats(m, &[(19, 300), (12, 10)]);
    fx.stats(
        p,
        &[
            (31, 100),
            (12, 8),
            (st::MAXHP, 25600),
            (st::HITPOINTS, 25600),
        ],
    );
    fx.sim.sys.units.get_mut(p).unwrap().flags |=
        unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
    fx.mark(p, bits::PLAYER);
    let c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.sim
        .ai(&mut fx.game, |g, cx| {
            cx.store.entry(m).control = Some(AiControl::default());
            install(g, cx, m, 0);
        })
        .unwrap();
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), 2, None, 0, 0)
        .unwrap();
    let params = MissileParams {
        owner: Some(m),
        origin: Some(m),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: 13,
        target_y: 10,
        ..MissileParams::default()
    };
    let missile = fx
        .sim
        .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &params))
        .unwrap()
        .unwrap();
    fx.stats(missile, &[(21, 2560), (22, 2560)]);
    for _ in 0..30 {
        fx.tick();
    }
    // The missile hit on its third run (tick 3) and is gone; the player
    // lost 10 life (the to-hit draw on the owner's seed, derived from the
    // game seed 1234 at allocation, is below the chance 83).
    assert!(fx.game.lists.unit(missile).is_none());
    let life = fx.stat(p, st::HITPOINTS);
    assert_eq!(life, 25600 - 2560);
    assert_eq!(
        fx.sim.sys.hooks.x.log,
        [
            format!("event 0 Some({})", p.0),
            format!("event 11 Some({})", p.0),
            format!("event 2 Some({})", p.0),
            format!("reaction {} {} 0x1", m.0, p.0),
        ]
    );
    // Tick 1's room switch joined room A, the client is its only one, so
    // its monster's AI was rescheduled at 1 + 2: the monster thought at
    // tick 3 (Idle: next think at 203), not at the staged tick 2.
    assert_eq!(fx.timers(m), vec![(event::AI_THINK, 203)]);
    // The client joined room A; both rooms stay active.
    assert_eq!(fx.game.lists.client(c).unwrap().room, Some(a));
    let mut rooms = 0;
    let mut cur = fx.game.lists.room_first(0);
    while let Some(r) = cur {
        rooms += 1;
        cur = fx.game.lists.room_next(r);
    }
    assert_eq!(rooms, 2);
    fx.assert_clean();
    let seeds = [p, m]
        .iter()
        .map(|&u| fx.sim.sys.units.get(u).unwrap().seed)
        .collect();
    let timers = [p, m, missile].iter().map(|&u| fx.timers(u)).collect();
    (
        fx.game.frame,
        life,
        seeds,
        timers,
        fx.sim.sys.hooks.game_seed,
        rooms,
        fx.sim.sys.hooks.x.log.clone(),
    )
}

#[test]
fn tick_runs_the_combined_dispatcher_deterministically() {
    let first = run();
    assert_eq!(first.0, 30);
    assert_eq!(first, run());
}
