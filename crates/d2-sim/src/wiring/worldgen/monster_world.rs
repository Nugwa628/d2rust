//! [`WorldState`] as the action hooks' [`MonsterWorld`]: the monster type
//! init, the umod dispatcher and the world state's part of a unit free,
//! each on a [`WorldHost`] built over the call's unit side and the world
//! state. [`WorldSim`] lends its world state to the action hooks around
//! every timer event and tick hook it forwards ([`WorldSim::lend`]), so
//! the action adapters (missiles, AI, combat, skill use, the kill) reach
//! it wherever they allocate, remove, change a monster's mode or query
//! monster data.
//!
//! Population and init calls ([`WorldSim::host`]) hold the world state
//! directly instead: the action hooks then have no world, so a monster
//! population allocates gets its type init from population's own call
//! ([`super::population_init`]), not a second one from the allocator's
//! hook.

use std::any::Any;

use crate::game::Game;
use crate::monsters::init::{self, MonsterData};
use crate::monsters::population::{self as pop, placement, preset, room, spawn};
use crate::units::hooks::Sim;
use crate::units::RoomId;
use crate::units::UnitId;

use super::super::action::{ActionHooks, ActionSim, MonsterWorld};

/// The fixed umods whose init runs before a unit's own list
/// (they restore a boss's fixed modifiers first).
const RESTORE_FIXED_UMODS: [u8; 4] = [1, 2, 3, 4];
use super::{View, WorldHost, WorldPending, WorldSim, WorldState};

impl WorldState {
    /// A world state with the same tables, level types and game info and
    /// nothing else: what [`WorldSim::world`] holds while the real one is
    /// lent.
    pub(super) fn placeholder(&self) -> Self {
        Self::new(self.types.clone(), self.tables.clone(), self.init_info)
    }
}

impl<X: WorldPending> MonsterWorld<X> for WorldState {
    fn region_classes(&self, level: u32) -> Option<Vec<i32>> {
        let r = self.pop.regions.get(level as i32)?;
        let n = usize::from(r.mon_count).min(r.entries.len());
        Some(r.entries[..n].iter().map(|e| i32::from(e.class)).collect())
    }
    fn level_mon_list(&self, level: u32) -> Option<Vec<i32>> {
        let l = self.tables.pop.level(i32::try_from(level).ok()?)?;
        Some(l.mon.iter().map(|&c| i32::from(c)).collect())
    }
    fn class_is_spawn(&self, class: i32) -> Option<bool> {
        self.tables.pop.mon(class).map(|m| m.is_spawn)
    }
    fn count_death(&mut self, unit: UnitId, alignment: u8) {
        let Some(m) = self.monsters.get_mut(unit) else {
            return;
        };
        if m.death_counted || m.not_counted || alignment != 0 {
            return;
        }
        m.death_counted = true;
        self.pop
            .regions
            .count_kill(m.level_id, false, alignment, true);
    }
    fn den_counts(&self) -> Option<(u32, u32, u32, u32)> {
        let r = self.pop.regions.get(8)?;
        Some((
            r.evil_spawned as u32,
            r.evil_killed as u32,
            r.rooms_visited as u32,
            u32::try_from(r.room_count).unwrap_or(u32::MAX),
        ))
    }
    fn monstats_count(&self) -> u32 {
        self.tables.monstats.len() as u32
    }
    fn type_init(&mut self, sim: &mut Sim<'_>, h: &mut ActionHooks<X>, unit: UnitId) {
        let t = self.tables.clone();
        let mut wh = host(sim, h, self);
        init::type_init(&t.init(), &mut wh, unit);
    }

    fn reinit(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        class: i32,
        mode: u32,
    ) -> bool {
        let t = self.tables.clone();
        let mut wh = host(sim, h, self);
        init::reinit(&t.init(), &mut wh, unit, class, mode)
    }

    fn umods(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        arg: Option<UnitId>,
        mode: u8,
    ) {
        let t = self.tables.clone();
        let mut wh = host(sim, h, self);
        init::dispatch(&t.init(), &mut wh, unit, arg, mode);
    }

    fn assign_umod(&mut self, sim: &mut Sim<'_>, h: &mut ActionHooks<X>, unit: UnitId, umod: u8) {
        let t = self.tables.clone();
        let mut wh = host(sim, h, self);
        init::assign_umod(&t.init(), &mut wh, unit, umod, false);
    }

    fn assign_umod_arg(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        umod: u8,
        unique: bool,
    ) {
        let t = self.tables.clone();
        let mut wh = host(sim, h, self);
        init::assign_umod(&t.init(), &mut wh, unit, umod, unique);
    }

    fn forget(&mut self, unit: UnitId) {
        WorldState::forget(self, unit);
    }
    /// A boss through the boss-modifier init
    /// (no minion spawn on a restore), a minion
    /// through its own walk.
    fn restore_umod_init(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        unique: bool,
    ) -> bool {
        let t = self.tables.clone();
        let cx = t.init();
        let mut wh = host(sim, h, self);
        if unique {
            init::boss_minions_and_init(&cx, &mut wh, unit, 0, 0, None, false);
        } else {
            let own = wh.w.monsters.get(unit).map(|m| m.umod_list().to_vec());
            for u in RESTORE_FIXED_UMODS
                .into_iter()
                .chain(own.unwrap_or_default())
            {
                init::run_umod_init(&cx, &mut wh, unit, u, false);
            }
        }
        true
    }

    fn monster(&self, unit: UnitId) -> Option<&MonsterData> {
        self.monsters.get(unit)
    }

    fn monster_mut(&mut self, unit: UnitId) -> Option<&mut MonsterData> {
        self.monsters.get_mut(unit)
    }

    fn component_counts(&self, class: u32) -> Option<[u8; 16]> {
        let m = self.tables.monstats.get(class as usize)?;
        self.tables
            .components
            .get(usize::from(m.monstatsex))
            .copied()
    }

    /// Off-tick population of one
    /// room from inside an action step: [`WorldSim`]'s room-pass hooks
    /// on this world state.
    fn populate_off_tick(&mut self, sim: &mut Sim<'_>, h: &mut ActionHooks<X>, r: RoomId) -> bool {
        if self.pop_lent {
            return false;
        }
        h.as_world_holder(|h| {
            // Rule 1: ambient spawns on every call.
            host(sim, h, self).population(|cx| room::ambient(cx, r));
            let (populated, active) = sim
                .game
                .lists
                .room(r)
                .map_or((true, true), |e| (e.populated, e.units_active));
            if populated {
                if !active {
                    restore_off_tick(self, sim, h, r);
                    if let Some(e) = sim.game.lists.room_mut(r) {
                        e.units_active = true;
                    }
                }
                return;
            }
            // Presets: the first walk with the world lent to the hooks
            // (a quest object's init may allocate a monster), then the
            // monster walk.
            let taken = {
                let WorldHost { game, v, .. } = &mut host(sim, h, self);
                v.take_presets(game, r)
            };
            if taken {
                let held = std::mem::replace(self, self.placeholder());
                let out = h.relend_monster_world(Box::new(held));
                View::of(&mut *sim.units, &mut *sim.stats, sim.data, h)
                    .spawn_preset_units(sim.game, r);
                match h.take_relent_monster_world::<WorldState>(out) {
                    Some(w) => *self = w,
                    None => panic!("populate_off_tick: the lent world state did not come back"),
                }
                host(sim, h, self).population(|cx| preset::place_presets(cx, r));
            }
            restore_off_tick(self, sim, h, r);
            {
                let mut wh = host(sim, h, self);
                let created = {
                    let WorldHost { game, v, .. } = &mut wh;
                    v.spawn_host_objects(game, r);
                    v.spawn_host_monsters(game, r)
                };
                for (u, _) in created {
                    wh.start_host_ai(u);
                }
                pop::MonsterInit::populate_objects(&mut wh, r);
                wh.population(|cx| room::populate_room(cx, r));
            }
            if let Some(e) = sim.game.lists.room_mut(r) {
                e.populated = true;
                e.units_active = true;
            }
        });
        true
    }

    /// Spawns a monster at a point through population's placement and creation
    /// ([`placement::place_at`]). The population
    /// state lent elsewhere (a population call in progress): `None`.
    fn spawn_at(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: crate::units::RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        if self.pop_lent {
            return None;
        }
        let placed = h.as_world_holder(|h| {
            let mut wh = host(sim, h, self);
            wh.population(|cx| {
                placement::place_at(cx, room, None, x, y, class, mode, spread, flags)
            })
        });
        Some(placed.unit())
    }

    /// The plain-record creation of a restore through population
    /// ([`spawn::restore_plain`]).
    #[allow(clippy::too_many_arguments)]
    fn restore_spawn(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: crate::units::RoomId,
        x: i32,
        y: i32,
        guid: u32,
        class: i32,
        mode: u8,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        if self.pop_lent {
            return None;
        }
        Some(h.as_world_holder(|h| {
            let mut wh = host(sim, h, self);
            wh.population(|cx| spawn::restore_plain(cx, room, x, y, guid, class, mode, flags))
        }))
    }

    /// Spawns at a level's spawn point through population
    /// ([`placement::spawn_point`]; no cl, no warp
    /// check).
    fn spawn_point(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: crate::units::RoomId,
        class: i32,
    ) -> Option<Option<(i32, i32)>> {
        if self.pop_lent {
            return None;
        }
        Some(h.as_world_holder(|h| {
            let mut wh = host(sim, h, self);
            wh.population(|cx| placement::spawn_point(cx, room, None, class, false))
        }))
    }

    /// Spawns near a unit through population's placement
    /// ([`placement::place_near`]).
    #[allow(clippy::too_many_arguments)]
    fn spawn_near(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        if self.pop_lent {
            return None;
        }
        let placed = h.as_world_holder(|h| {
            let mut wh = host(sim, h, self);
            wh.population(|cx| placement::place_near(cx, None, unit, class, mode, spread, flags))
        });
        Some(placed.unit())
    }

    /// A random boss through population
    /// ([`spawn::random_boss`], champion allowed, warp check).
    fn spawn_random_boss(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: crate::units::RoomId,
        class: i32,
    ) -> Option<Option<UnitId>> {
        if self.pop_lent {
            return None;
        }
        let made = h.as_world_holder(|h| {
            let mut wh = host(sim, h, self);
            wh.population(|cx| spawn::random_boss(cx, room, None, class, true, 0, 0, true))
        });
        Some(made)
    }

    /// A preset spawn through population
    /// ([`preset::preset_spawn`]); population's state lent elsewhere:
    /// `None`.
    fn spawn_preset(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: crate::units::RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
    ) -> Option<Option<UnitId>> {
        if self.pop_lent {
            return None;
        }
        let made = h.as_world_holder(|h| {
            let mut wh = host(sim, h, self);
            wh.population(|cx| preset::preset_spawn(cx, room, class, x, y, mode))
        });
        Some(made)
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// The [`WorldHost`] of a lent-world call.
fn host<'a, X>(
    sim: &'a mut Sim<'_>,
    h: &'a mut ActionHooks<X>,
    w: &'a mut WorldState,
) -> WorldHost<'a, X> {
    WorldHost {
        game: &mut *sim.game,
        v: View::of(&mut *sim.units, &mut *sim.stats, sim.data, h),
        w,
    }
}

/// The inactive restore of
/// [`MonsterWorld::populate_off_tick`]: the action wiring's store with the
/// world lent back (a restored monster's re-init), else the host's.
fn restore_off_tick<X: WorldPending>(
    w: &mut WorldState,
    sim: &mut Sim<'_>,
    h: &mut ActionHooks<X>,
    r: RoomId,
) {
    if h.inactive.is_some() {
        let held = std::mem::replace(w, w.placeholder());
        let out = h.relend_monster_world(Box::new(held));
        View::of(&mut *sim.units, &mut *sim.stats, sim.data, h).restore_inactive(sim.game, r);
        match h.take_relent_monster_world::<WorldState>(out) {
            Some(back) => *w = back,
            None => panic!("populate_off_tick: the lent world state did not come back"),
        }
    } else {
        h.x.restore_inactive_units(r);
        View::of(&mut *sim.units, &mut *sim.stats, sim.data, h).restore_fallback_tiles(sim.game, r);
    }
}

impl<X: WorldPending> WorldSim<X> {
    /// Runs `f` on the action systems with the world state lent to the
    /// action hooks ([`ActionHooks::monster_world`]): the entry for a host
    /// that drives the action systems directly (messages, skill use)
    /// and still wants the monster routes. When the hooks already hold a
    /// world (a host lent its own), `f` runs with that one.
    pub fn lend<R>(&mut self, f: impl FnOnce(&mut ActionSim<X>) -> R) -> R {
        let hooks = &mut self.action.sys.hooks;
        if hooks.monster_world.is_some() {
            return f(&mut self.action);
        }
        let placeholder = self.world.placeholder();
        let held = std::mem::replace(&mut self.world, placeholder);
        hooks.monster_world = Some(Box::new(held));
        let r = f(&mut self.action);
        match self
            .action
            .sys
            .hooks
            .monster_world
            .take()
            .map(|w| w.into_any().downcast::<WorldState>())
        {
            Some(Ok(w)) => self.world = *w,
            // `f` took the world out of the hooks or put another one in:
            // the game's monster state is gone (API misuse, fatal).
            _ => panic!("WorldSim::lend: the lent world state did not come back"),
        }
        r
    }

    /// [`ActionSim::with`] with the world state lent (allocation, removal,
    /// mode changes and queries through the monster routes).
    pub fn with<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut Game, &mut View<'_, X>) -> R,
    ) -> R {
        self.lend(|a| a.with(game, f))
    }
}

#[cfg(test)]
mod restore_umod_tests {
    use super::super::tests::{isle_ds1s, Fx, ISLE};
    use crate::drlg::TileRect;
    use crate::monsters::init::type_flag;
    use crate::stats::stat;
    use crate::units::lifecycle::AllocRequest;
    use crate::units::{UnitId, UnitType};

    /// A unique monster of class 0 on [`ISLE`]'s room at (8000, 8000),
    /// allocated by action code with the world state lent, with an empty
    /// umod list (the fixed umod 4's init adds 3 to the level).
    fn boss(fx: &mut Fx) -> UnitId {
        let (_, rooms) = fx.generate(ISLE).unwrap();
        let r = *rooms
            .iter()
            .find(|&&r| fx.drlg().room(r).rect == TileRect::new(8000, 8000, 8, 8))
            .unwrap();
        let room = fx.stream(&[r]).unwrap()[0];
        fx.sim.create_regions();
        let req = AllocRequest {
            ty: UnitType::Monster,
            class: 0,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: false,
        };
        let u = fx
            .sim
            .with(&mut fx.game, |g, v| v.allocate(g, &req, 40010, 40010))
            .expect("monster");
        let m = fx.sim.world.monsters.get_mut(u).unwrap();
        m.type_flags |= type_flag::UNIQUE | type_flag::BOSS;
        u
    }

    fn level(fx: &Fx, u: UnitId) -> i32 {
        fx.sim.action.sys.stats.unit_base(u, stat::LEVEL, 0)
    }

    fn restore(fx: &mut Fx, u: UnitId, lent: bool) -> bool {
        let game = &mut fx.game;
        let mut run = |s: &mut crate::wiring::action::ActionSim<_>| {
            s.with(game, |g, v| {
                let mut sim = crate::units::hooks::Sim {
                    game: g,
                    units: &mut *v.units,
                    stats: &mut *v.stats,
                    data: v.data,
                };
                v.h.restore_umod_init(&mut sim, u, true)
            })
        };
        if lent {
            fx.sim.lend(run)
        } else {
            run(&mut fx.sim.action)
        }
    }

    #[test]
    fn a_restored_boss_runs_its_umod_init_again() {
        let mut fx = Fx::new(isle_ds1s());
        let u = boss(&mut fx);
        let before = level(&fx, u);
        assert!(restore(&mut fx, u, true));
        assert_eq!(level(&fx, u), before + 3);
        // No world lent: nothing runs (the caller keeps the copied ids).
        assert!(!restore(&mut fx, u, false));
        assert_eq!(level(&fx, u), before + 3);
        fx.assert_clean();
    }
}
