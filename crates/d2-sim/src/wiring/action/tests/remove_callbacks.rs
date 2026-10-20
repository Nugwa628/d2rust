//! Tests for the remove callbacks that only turn a state off: the missile
//! just-hit list and the Charge counter. Once the list expires the state
//! must be off, or the unit stays "just hit" for good.

use super::*;
use crate::skills::use_::bodies::callback;
use crate::stats::lists::RemoveCallback;
use crate::stats::StatHost;
use crate::units::hooks::{Sim, UnitHooks};

const STATE_JUSTHIT: u32 = 86;
const STATE_CHARGE: u32 = 4;

fn state_off_after_expiry(cb: u32, state: u32) {
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    fx.sim
        .with(&mut fx.game, |_, v| v.set_state(m, state as u16, true));
    assert!(fx.sim.sys.stats.has_state(m, state));
    fx.sim.with(&mut fx.game, |g, v| {
        let mut sim = Sim {
            game: g,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        let list = sim.stats.unit_list(m).expect("unit list");
        v.h.list_removed(sim.stats, m, state, list, RemoveCallback(cb));
        v.h.lists_expired(&mut sim, m);
    });
    assert!(!fx.sim.sys.stats.has_state(m, state));
}

#[test]
fn the_justhit_callback_turns_state_86_off() {
    state_off_after_expiry(callback::JUSTHIT, STATE_JUSTHIT);
}

#[test]
fn the_charge_callback_turns_its_state_off() {
    state_off_after_expiry(callback::CHARGE, STATE_CHARGE);
}

/// The shrine stamina callback: after the default, stamina
/// is set to the unit's max stamina, so a unit below its maximum is
/// refilled (a clamp would leave it as is).
#[test]
fn shrine_stamina_end_refills() {
    use crate::stats::stat;
    use crate::world::objects::shrines::STAMINA_REMOVE;
    const STATE: u32 = 136;
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    fx.sim.with(&mut fx.game, |g, v| {
        let mut sim = Sim {
            game: g,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        sim.stats
            .unit_set(&mut *v.h, p, stat::MAXSTAMINA, 92 << 8, 0);
        sim.stats.unit_set(&mut *v.h, p, stat::STAMINA, 10 << 8, 0);
        let list = sim.stats.unit_list(p).expect("unit list");
        v.h.list_removed(sim.stats, p, STATE, list, RemoveCallback(STAMINA_REMOVE));
        v.h.lists_expired(&mut sim, p);
    });
    assert_eq!(fx.sim.sys.stats.unit_total(p, stat::STAMINA, 0), 92 << 8);
}
