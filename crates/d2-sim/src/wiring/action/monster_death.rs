//! The monster death clean-up, run from the death-start (DT) and dead-start
//! (DD) mode changes, and the room's dead-GUID ring.
//!
//! The clean-up runs in this order: the overhead record freed, the
//! pack-leader handover, the target-node list leave, flags &= ~0x800C, the
//! stat-list death, the state keep mask, the `hide` flag, the dead-body
//! footprint (unless monstats2 flag 0x13), the path direction snap and,
//! last, the unit's think (type 2) and type-3 events cancelled, so no think
//! runs on the dead unit (otherwise killed monsters return to mode 1 with
//! hp 0).
//!
//! A host's DT start ([`Pending::monster_death_start`]) sets mode 0 and
//! calls [`death_cleanup`] before its treasure gate; for a host that does
//! not, the wiring runs it after the host's start
//! (`ActionHooks::monster_death`).
//!
//! Seams: the slot heads of the target-node lists are the host's
//! ([`Pending::target_nodes`]); the node leaves [`Game::target_nodes`]
//! and the unit's node index is reset here; the pack handover acts on the AI
//! store's controls and is skipped while the store is lent (a kill inside a
//! think).

use crate::game::Game;
use crate::monsters::ai::{self, AiStore, UnitRef};
use crate::stats::states::group;
use crate::tick::events::event;
use crate::units::hooks::{Sim, UnitHooks};
use crate::units::modes::{self, monster_mode};
use crate::units::record::{flags, INITIAL_NODE_INDEX};
use crate::units::{UnitId, UnitType};

use super::{ActionHooks, Pending, View, WiringError};

/// Unit flags the clean-up clears.
const CLEARED_FLAGS: u32 = 0x800C;
/// AI control flag bits the pack handover reads.
const CONTROL_LEADER: u16 = 0x1;
const CONTROL_HANDOVER: u16 = 0x2;

impl<X: Pending> ActionHooks<X> {
    /// The DT start through the host ([`Pending::monster_death_start`])
    /// with the clean-up guaranteed and the dead-GUID ring after it.
    pub(super) fn monster_death(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        target: Option<UnitId>,
    ) -> bool {
        self.death_cleaned = None;
        let started = X::monster_death_start(self, sim, unit, target);
        if started {
            if sim.game.coverage.enabled {
                if let Some(record) = sim.units.get(unit) {
                    sim.game.coverage.hit("monsters_killed", || record.class);
                }
            }
            if self.death_cleaned != Some(unit) {
                death_cleanup(self, sim, unit);
            }
            record_dead(sim.game, unit);
        }
        self.death_cleaned = None;
        started
    }

    /// The DD start: a unit in a mode other than 0 gets the clean-up; mode 12
    /// is set by the plain mode set; the unit's events of types 8 and 9 are
    /// cancelled.
    pub(super) fn monster_dead_start(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        if sim
            .units
            .get(unit)
            .is_some_and(|r| r.mode != monster_mode::DT)
        {
            death_cleanup(self, sim, unit);
        }
        if let Err(e) = modes::set_mode(sim, self, unit, monster_mode::DD) {
            self.errors.push(WiringError::Unit(e));
        }
        for ty in [event::PERIODIC_SKILLS, event::PERIODIC_STATS] {
            sim.game.timers.cancel_unit_events(unit, ty, None);
        }
    }
}

/// The death clean-up of `unit`. The final path direction is the one the
/// kill writes: toward the mode change's target ([`ActionHooks::mode_target`])
/// when there is one, else the unit's current direction.
pub fn death_cleanup<X: Pending>(h: &mut ActionHooks<X>, sim: &mut Sim<'_>, unit: UnitId) {
    h.death_cleaned = Some(unit);
    let dir = death_direction(h, unit);
    // The overhead record.
    if sim.units.get(unit).is_some_and(|r| r.hover.is_some()) {
        h.free_hover(sim, unit);
        h.session.overheads.remove(&unit);
        if let Some(r) = sim.units.get_mut(unit) {
            r.hover = None;
            r.flags |= flags::HOVER_FREED;
        }
        queue_update(h, sim.game, unit);
    }
    // Pack-leader handover.
    if let Some(store) = h.ai.as_mut() {
        pack_handover(store, sim.game, unit);
    }
    // Leave the target-node list.
    sim.game.target_nodes.remove(unit);
    if let Some(r) = sim.units.get_mut(unit) {
        r.node_index = INITIAL_NODE_INDEX;
        r.flags &= !CLEARED_FLAGS;
    }
    // Stat-list death, then the remove callbacks of the freed lists.
    sim.stats.death(h, unit);
    h.lists_expired(sim, unit);
    // Keep only the states that stay through death (boss, monster or player set).
    let keep = match sim.units.get(unit).map(|r| r.ty) {
        Some(UnitType::Monster) if h.is_boss(unit) => group::BOSS_STAY_DEATH,
        Some(UnitType::Monster) => group::MON_STAY_DEATH,
        _ => group::PLR_STAY_DEATH,
    };
    sim.stats.clear_states_except(unit, keep);
    queue_update(h, sim.game, unit);
    // A unit in a `hide` state drops its tile flag.
    if sim.stats.has_group(unit, group::HIDE) {
        if let Some(r) = sim.units.get_mut(unit) {
            r.flags &= !flags::TILE;
        }
    }
    // The dead-body footprint, unless monstats2 flag 0x13 (checked inside).
    View::of(sim.units, sim.stats, sim.data, h).death_footprint(unit);
    // Snap the path direction.
    if let (Some(d), Some(p)) = (dir, h.paths.as_mut().and_then(|p| p.dynamic_mut(unit))) {
        p.direction = d;
        p.new_direction = d;
    }
    // Cancel the think and regen events.
    ai::cancel_think_and_regen(sim.game, unit);
}

/// Records the unit's GUID in its room's dead-GUID ring; a unit with no room
/// does nothing.
pub fn record_dead(game: &mut Game, unit: UnitId) {
    let Some((room, guid)) = game
        .lists
        .unit(unit)
        .and_then(|e| Some((e.room()?, e.guid)))
    else {
        return;
    };
    if let Some(r) = game.lists.room_mut(room) {
        r.record_dead(guid);
    }
}

/// The direction byte the kill writes: toward the attacker when there is
/// one, else the unit's path direction. Both positions are path sub-tiles,
/// turned into sub-tile centres x * 0x10000 + 0x8000 before the direction
/// vector is taken. `None` without a dynamic path (nothing to snap).
fn death_direction<X: Pending>(h: &ActionHooks<X>, unit: UnitId) -> Option<u8> {
    let current = h.paths.as_ref()?.dynamic(unit)?.direction;
    let Some(a) = h.mode_target else {
        return Some(current);
    };
    let p = h.paths.as_ref()?;
    let centre = |v: i32| (v as u32).wrapping_mul(0x10000).wrapping_add(0x8000);
    let (from, at) = (h.path_position(unit), h.path_position(a));
    let (_, d) = crate::path::walk::geom::direction_vector(
        &p.tables,
        (centre(from.0), centre(from.1)),
        (centre(at.0), centre(at.1)),
    );
    Some(d & 63)
}

fn queue_update<X: Pending>(h: &mut ActionHooks<X>, game: &mut Game, unit: UnitId) {
    if let Err(e) = game.lists.queue_update(unit) {
        h.errors
            .push(WiringError::Unit(crate::units::modes::UnitError::Game(
                e.into(),
            )));
    }
}

/// The pack handover when a leader dies. The minion list is read
/// from its head: new minions belong at the head where D2RUST appends,
/// so the head is the vector's end.
fn pack_handover(store: &mut AiStore, game: &Game, unit: UnitId) {
    let Some(c) = store.control(unit) else {
        return;
    };
    if c.flags & CONTROL_LEADER == 0 {
        return;
    }
    let list: Vec<u32> = c.minions.iter().rev().copied().collect();
    let resolve = |g: u32| game.lists.find_unit(UnitType::Monster, g);
    if c.flags & CONTROL_HANDOVER == 0 {
        // The pack is released.
        for m in list.iter().filter_map(|&g| resolve(g)) {
            if let Some(mc) = store.control_mut(m) {
                mc.minion_owner = None;
            }
        }
        return;
    }
    // The first entry that resolves to a unit becomes the new leader.
    let Some(n) = list.iter().find_map(|&g| resolve(g)) else {
        return;
    };
    let Some(n_guid) = game.lists.unit(n).map(|e| e.guid) else {
        return;
    };
    let n_ref = Some(UnitRef {
        ty: UnitType::Monster,
        guid: n_guid,
    });
    if let Some(nc) = store.control_mut(n) {
        nc.flags |= CONTROL_HANDOVER | CONTROL_LEADER;
        nc.minion_owner = n_ref;
    }
    if let Some(c) = store.control_mut(unit) {
        c.minion_owner = n_ref;
    }
    let mut joining = Vec::new();
    if let Some(g) = game.lists.unit(unit).map(|e| e.guid) {
        joining.push(g);
    }
    for &g in &list {
        let Some(m) = resolve(g).filter(|&m| m != n) else {
            continue;
        };
        if let Some(mc) = store.control_mut(m) {
            mc.minion_owner = n_ref;
        }
        joining.push(g);
    }
    if let Some(nc) = store.control_mut(n) {
        nc.minions.extend(joining);
    }
}
