//! [`WorldSim`]: the action systems ([`ActionSim`]) plus the world state,
//! as one [`EventDispatch`] and [`TickHooks`] for [`crate::tick::tick`].
//! Timer events run the action systems' unit dispatch with the world
//! state lent to the action hooks ([`super::monster_world`]: monster
//! event 7, monster init on allocation, the world state's part of a unit
//! free, the umod callbacks, monster-data queries). Every tick hook goes
//! to the action systems, with the world state lent too, except the
//! room-pass hooks, which run population on a [`WorldHost`] holding the
//! world state: ambient spawns, presets, inactive restore, objects,
//! monster population.

use std::sync::Arc;

use crate::game::Game;
use crate::monsters::init;
use crate::monsters::population::{self as pop, preset, room};
use crate::stats::StatData;
use crate::tick::timer::TimerRun;
use crate::tick::{EventDispatch, TickHooks};
use crate::units::hooks::UnitData;
use crate::units::{ClientId, RoomId, UnitId};

use super::super::action::{ActionHooks, ActionSim};
use super::{View, WorldHost, WorldPending, WorldState, WorldgenError};

/// The action systems and the world-generation state of one game.
pub struct WorldSim<X> {
    pub action: ActionSim<X>,
    pub world: WorldState,
}

impl<X: WorldPending> WorldSim<X> {
    pub fn new(
        stat_data: Arc<StatData>,
        data: UnitData,
        hooks: ActionHooks<X>,
        world: WorldState,
    ) -> Self {
        Self {
            action: ActionSim::new(stat_data, data, hooks),
            world,
        }
    }

    /// Runs `f` on a [`WorldHost`] over the game.
    pub fn host<R>(&mut self, game: &mut Game, f: impl FnOnce(&mut WorldHost<'_, X>) -> R) -> R {
        let s = &mut self.action.sys;
        let mut h = WorldHost {
            game,
            v: View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks),
            w: &mut self.world,
        };
        f(&mut h)
    }

    /// Runs `f` with population's context (game creation, tests).
    pub fn population<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut pop::Ctx<'_, WorldHost<'_, X>>) -> R,
    ) -> R {
        self.host(game, |h| h.population(f))
    }

    /// Runs `f` with init's tables and host (event 7, the mode-change and
    /// missile callbacks, tests).
    pub fn init<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&init::Ctx<'_>, &mut WorldHost<'_, X>) -> R,
    ) -> R {
        self.host(game, |h| h.init(f))
    }

    /// Game creation: the regions on the game seed of the action state.
    pub fn create_regions(&mut self) {
        let mut seed = self.action.sys.hooks.game_seed;
        self.world.create_regions(&mut seed);
        self.action.sys.hooks.game_seed = seed;
    }

    /// Game creation's object control ([`ActionSim::create_objects`]): call
    /// right after [`WorldSim::create_regions`] (the game-seed order).
    pub fn create_objects(&mut self, tables: Arc<crate::world::objects::ObjectTables>) {
        self.action.create_objects(tables);
    }

    /// Every error so far: world adapters, level types, action adapters
    /// and the unit dispatch.
    pub fn errors(&self) -> Vec<String> {
        let mut out: Vec<String> = self.world.errors.iter().map(|e| format!("{e:?}")).collect();
        out.extend(
            self.world
                .types
                .borrow()
                .errors
                .iter()
                .map(|e: &WorldgenError| format!("{e:?}")),
        );
        out.extend(
            self.action
                .sys
                .hooks
                .errors
                .iter()
                .map(|e| format!("{e:?}")),
        );
        out.extend(self.action.sys.errors.iter().map(|e| format!("{e:?}")));
        out
    }
}

impl<X: WorldPending> EventDispatch for WorldSim<X> {
    /// The unit dispatch of the action systems with the world state lent
    /// to the action hooks ([`super::events`]: monster event 7, the world
    /// state's part of a unit free, the monster routes of the action
    /// adapters).
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        self.lend(|a| a.run_event(game, run));
    }
}

/// Tick hooks forwarded to the action systems with the world state lent.
macro_rules! lent {
    ($( $(#[$m:meta])* fn $name:ident(&mut self, game: &mut Game $(, $a:ident: $t:ty)*) $(-> $r:ty)?; )*) => {
        $(
            $(#[$m])*
            fn $name(&mut self, game: &mut Game $(, $a: $t)*) $(-> $r)? {
                self.lend(|s| s.$name(game $(, $a)*))
            }
        )*
    };
}

impl<X: WorldPending> TickHooks for WorldSim<X> {
    /// Room pass: ambient spawns.
    fn ambient_spawns(&mut self, game: &mut Game, r: RoomId) {
        self.population(game, |cx| room::ambient(cx, r));
    }
    /// Room pass: preset spawns.
    /// The DRLG room hands its preset list out once: a later call places
    /// nothing. The first walk's warp tiles come before the monster walk.
    fn spawn_presets(&mut self, game: &mut Game, r: RoomId) {
        let taken = self.host(game, |h| {
            let WorldHost { game, v, .. } = h;
            v.take_presets(game, r)
        });
        if taken {
            // The first walk: every non-monster preset (objects, warp tiles)
            // in list order (`View::spawn_preset_units`), then the monster
            // walk. With the world state lent to the action hooks: a quest
            // object's init runs inside its allocation and may allocate a
            // monster (Larzuk, the caged barbarians), whose type init needs
            // the monster world.
            self.with(game, |g, v| {
                v.spawn_preset_units(g, r);
            });
            self.population(game, |cx| preset::place_presets(cx, r));
        }
        self.host(game, |h| {
            let created = {
                let WorldHost { game, v, .. } = &mut *h;
                v.spawn_host_objects(game, r);
                // D2RUST's own, play preview only: the monsters a
                // level types provider lists for the host
                // (`HOST_MONSTER_PRESET`). Real levels need no such type: the
                // Burial Grounds create Blood Raven by the DS1 preset
                // walk (check
                // `w3-01-burial-blood-raven`): none on real levels.
                v.spawn_host_monsters(game, r)
            };
            // Every created monster gets the creation-time AI, whatever
            // its class.
            for (u, _) in created {
                h.start_host_ai(u);
            }
        });
    }
    /// Room pass: restores inactive units, from the action wiring's inactive
    /// store when it is on, else the host's.
    fn restore_inactive_units(&mut self, game: &mut Game, r: RoomId) {
        if !self.lend(|a| a.restore(game, r)) {
            self.host(game, |h| {
                h.v.h.x.restore_inactive_units(r);
                // A D2RUST-only path (REC-230): without the inactive store
                // the room's warp tiles were stored and freed on their own
                // at its deactivation; they come back after the host's
                // monsters and items, as the store's other records do. The game's rule is the store,
                // which play turns on.
                let WorldHost { game, v, .. } = h;
                v.restore_fallback_tiles(game, r);
            });
        }
    }
    /// Room pass: object population, on the action
    /// wiring's object state when the game has one, else the
    /// [`WorldPending`] answer (the host's `populate_objects`; calling the
    /// pending seam directly left every room of a game with object state
    /// unpopulated, q-fixture-migrate).
    fn populate_objects(&mut self, game: &mut Game, r: RoomId) {
        self.host(game, |h| pop::MonsterInit::populate_objects(h, r));
    }
    /// Room pass: monster population.
    fn populate_monsters(&mut self, game: &mut Game, r: RoomId) {
        self.population(game, |cx| room::populate_room(cx, r));
    }

    lent! {
        fn advance_environment(&mut self, game: &mut Game, act: u8) -> bool;
        fn environment_changed(&mut self, game: &mut Game, act: u8, client: ClientId);
        fn client_room_ready(&mut self, game: &mut Game, client: ClientId) -> bool;
        fn send_load_complete(&mut self, game: &mut Game, client: ClientId);
        fn refresh_inventory(&mut self, game: &mut Game, client: ClientId);
        fn join_sequence(&mut self, game: &mut Game, client: ClientId);
        fn send_removed_units(&mut self, game: &mut Game, client: ClientId);
        fn send_unit_update(&mut self, game: &mut Game, client: ClientId, unit: UnitId);
        fn update_room_end(&mut self, game: &mut Game, client: ClientId, r: RoomId);
        fn client_update_messages(&mut self, game: &mut Game, client: ClientId);
        fn client_level_change(&mut self, game: &mut Game, client: ClientId);
        fn arena_sync(&mut self, game: &mut Game, client: ClientId);
        fn unit_update(&mut self, game: &mut Game, unit: UnitId);
        fn clear_arena_flag(&mut self, game: &mut Game);
        fn free_removal_records(&mut self, game: &mut Game, r: RoomId);
        fn update_quests(&mut self, game: &mut Game);
        fn room_inactivity(&mut self, game: &mut Game, r: RoomId) -> u32;
        fn act_allows_room_removal(&mut self, game: &mut Game, act: u8, r: RoomId) -> bool;
        fn compress_unit(&mut self, game: &mut Game, unit: UnitId);
        fn room_deactivated(&mut self, game: &mut Game, act: u8, r: RoomId);
        fn free_inactive_rooms(&mut self, game: &mut Game, act: u8);
        fn delete_inactive_items(&mut self, game: &mut Game, act: u8);
        fn expire_inactive_unit_items(&mut self, game: &mut Game, act: u8);
        fn end_of_tick(&mut self, game: &mut Game);
    }
}
