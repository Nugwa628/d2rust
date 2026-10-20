//! Tests for the by-time stat base time on the action wiring: every stat-list
//! owner that stands in an act gets the act's environment time, not only
//! players. Runs through the `Fx` test fixture.

use super::*;
use crate::tick::TickHooks;

/// t is taken from the owner's act with no test of the owner's unit type, so
/// a hireling's by-time items get t as a player's do; a freed unit keeps none.
#[test]
fn the_advance_gives_t_to_players_and_monsters_in_the_act() {
    use crate::stats::StatHost;
    let mut fx = Fx::new();
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 5, 5);
    let m = fx.spawn(UnitType::Monster, 0, a, 6, 6);
    let gone = fx.spawn(UnitType::Monster, 0, a, 7, 7);
    fx.game.lists.act_mut(0).unwrap().built = true;
    fx.sim.hooks().act_times.insert(gone, 77);
    fx.sim.hooks().enable_inactive_store();
    let mut g = std::mem::take(&mut fx.game);
    // An unsaved monster is freed by the compress.
    fx.sim.compress_unit(&mut g, gone);
    // Enough advances for the environment time to move off 0.
    for _ in 0..200 {
        fx.sim.advance_environment(&mut g, 0);
    }
    fx.game = g;
    let t = fx.game.lists.act(0).unwrap().environment.time();
    assert!(t > 0, "the environment time moved");
    assert_eq!(fx.sim.hooks().act_time(p), Some(t));
    assert_eq!(fx.sim.hooks().act_time(m), Some(t));
    assert_eq!(fx.sim.hooks().act_time(gone), None);
}
