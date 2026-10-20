//! Tests for regeneration through the combined dispatcher: a player's
//! event 3 on the real timer queue, run by `d2_sim::tick::tick` through
//! the action wiring's [`crate::wiring::action::ActionSim`], fills mana
//! on the real stat lists every frame and reschedules itself; the vitals
//! add no tick hook of their own.

use crate::tick::events::event;
use crate::units::UnitType;

use super::skill_use::Fx;
use super::st;

#[test]
fn player_regeneration_runs_every_frame_through_the_tick() {
    let mut fx = Fx::with(super::stat_data(), super::skill_use::skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    let max = 9472;
    fx.set(p, &[(st::MAXMANA, max), (st::MANA, 0)]);
    fx.game
        .schedule_event(p, u32::from(event::STAT_REGEN), 1, None, 0, 0)
        .unwrap();
    for n in 1..=3 {
        crate::tick::tick(&mut fx.game, &mut fx.sim);
        // max(max mana / (ManaRegen 4 · 25), 1) = 94 per frame.
        let mana = fx.sim.sys.stats.unit_total(p, st::MANA, 0);
        assert_eq!(mana, n * (max / 100));
        let t = &fx.game.timers;
        let next: Vec<_> = t
            .unit_timers(p)
            .into_iter()
            .filter(|&id| t.event(id).is_some_and(|e| e.0 == event::STAT_REGEN))
            .filter_map(|id| t.expire(id))
            .collect();
        assert_eq!(next, [n + 1]);
    }
    fx.assert_clean();
}
