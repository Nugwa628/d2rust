//! The quest updater step of the tick, on the game's [`QuestControl`] with the
//! economy's quest world ([`EconomyQuests`]): [`QuestTick`] wraps the tick
//! hooks of a game (any [`TickHooks`] with a unit side, e.g. the action or
//! world dispatcher) and runs [`QuestControl::update`] as its
//! `update_quests`. The tick decides when (`frame % 20 = 0`); every other
//! hook and every timer event is the wrapped dispatcher's, followed by the
//! object module's quest routes it queued
//! ([`QuestTick::run_quest_objects`]).

use crate::game::Game;
use crate::items::ItemTables;
use crate::stats::StatLists;
use crate::tick::timer::TimerRun;
use crate::tick::{EventDispatch, TickHooks};
use crate::units::dispatch::UnitSystem;
use crate::units::hooks::UnitData;
use crate::units::lifecycle::LifecycleHooks;
use crate::units::record::Units;
use crate::units::{ClientId, RoomId, UnitId};
use crate::wiring::action::{ActionHooks, ActionSim, ObjectRoute, Pending, QuestObjectCall};
use crate::wiring::worldgen::{WorldPending, WorldSim};
use crate::world::quests::QuestControl;

use super::{Economy, EconomyQuests, GameFields, ItemStore, QuestRest};

/// The unit side an [`Economy`] is built on: unit records, stat lists,
/// unit tables and the lifecycle hooks of a unit system.
pub trait UnitSide {
    type Hooks: LifecycleHooks;
    fn unit_side(&mut self) -> (&mut Units, &mut StatLists, &UnitData, &mut Self::Hooks);
    /// Runs `f` on the unit side with whatever the side lends its hooks
    /// for the call (a world side lends its monster world, so a monster a
    /// quest function allocates gets its type init).
    /// Default: [`Self::unit_side`].
    fn with_unit_side<T>(
        &mut self,
        f: impl FnOnce(&mut Units, &mut StatLists, &UnitData, &mut Self::Hooks) -> T,
    ) -> T {
        let (u, s, d, h) = self.unit_side();
        f(u, s, d, h)
    }
    /// The object module's queued quest routes (`ActionSim::take_quest_calls`;
    /// none without an object state).
    fn take_quest_calls(&mut self) -> Vec<QuestObjectCall> {
        Vec::new()
    }
    /// A route the quest code has no function for, for `Pending::object_route`.
    fn hand_back(&mut self, game: &mut Game, route: ObjectRoute) {
        let _ = (game, route);
    }
}

impl<H: LifecycleHooks> UnitSide for UnitSystem<H> {
    type Hooks = H;
    fn unit_side(&mut self) -> (&mut Units, &mut StatLists, &UnitData, &mut H) {
        (
            &mut self.units,
            &mut self.stats,
            &self.data,
            &mut self.hooks,
        )
    }
}

impl<X: Pending> UnitSide for ActionSim<X> {
    type Hooks = ActionHooks<X>;
    fn unit_side(&mut self) -> (&mut Units, &mut StatLists, &UnitData, &mut ActionHooks<X>) {
        self.sys.unit_side()
    }
    fn take_quest_calls(&mut self) -> Vec<QuestObjectCall> {
        ActionSim::take_quest_calls(self)
    }
    fn hand_back(&mut self, game: &mut Game, route: ObjectRoute) {
        self.sys.hooks.x.object_route(game, route);
    }
}

impl<X: WorldPending> UnitSide for WorldSim<X> {
    type Hooks = ActionHooks<X>;
    fn unit_side(&mut self) -> (&mut Units, &mut StatLists, &UnitData, &mut ActionHooks<X>) {
        self.action.sys.unit_side()
    }
    fn with_unit_side<T>(
        &mut self,
        f: impl FnOnce(&mut Units, &mut StatLists, &UnitData, &mut ActionHooks<X>) -> T,
    ) -> T {
        self.lend(|a| {
            let (u, s, d, h) = a.sys.unit_side();
            f(u, s, d, h)
        })
    }
    fn take_quest_calls(&mut self) -> Vec<QuestObjectCall> {
        self.action.take_quest_calls()
    }
    fn hand_back(&mut self, game: &mut Game, route: ObjectRoute) {
        self.action.sys.hooks.x.object_route(game, route);
    }
}

/// A game's tick hooks plus its quests: the economy's parts beside the
/// unit side, the quest control block and the quests' rest.
pub struct QuestTick<'q, S, R> {
    /// The wrapped game tick hooks.
    pub sim: &'q mut S,
    /// The game-creation fields.
    pub fields: &'q mut GameFields,
    /// The item tables.
    pub tables: &'q ItemTables,
    /// The item store.
    pub items: &'q mut ItemStore,
    /// The quest control block.
    pub quests: &'q mut QuestControl,
    /// The quests' rest.
    pub rest: &'q mut R,
}

impl<S: UnitSide, R: QuestRest> QuestTick<'_, S, R> {
    /// Runs the object module's queued quest routes on the quests
    /// ([`super::quest_objects`]) until none is left (a quest function
    /// may allocate an object whose init queues another); routes the quest
    /// code has no function for go back to `Pending::object_route`.
    pub fn run_quest_objects(&mut self, game: &mut Game) {
        loop {
            let calls = self.sim.take_quest_calls();
            if calls.is_empty() {
                return;
            }
            let (fields, tables, items, quests, rest) = (
                &mut *self.fields,
                self.tables,
                &mut *self.items,
                &mut *self.quests,
                &mut *self.rest,
            );
            let back = self.sim.with_unit_side(|units, stats, data, hooks| {
                let mut econ = Economy {
                    game: &mut *game,
                    units,
                    stats,
                    data,
                    hooks,
                    fields,
                    tables,
                    items,
                };
                let mut w = EconomyQuests::new(&mut econ, rest);
                super::quest_objects::run_all(quests, &mut w, calls)
            });
            for r in back {
                self.sim.hand_back(game, r);
            }
        }
    }
}

/// The wrapped dispatcher's timer event, then the quest routes it queued
/// (an object event 7 runs its quest function right after the event).
impl<S: EventDispatch + UnitSide, R: QuestRest> EventDispatch for QuestTick<'_, S, R> {
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        self.sim.run_event(game, run);
        self.run_quest_objects(game);
    }
}

impl<S: TickHooks + UnitSide, R: QuestRest> TickHooks for QuestTick<'_, S, R> {
    /// Runs the quest updater.
    fn update_quests(&mut self, game: &mut Game) {
        let (units, stats, data, hooks) = self.sim.unit_side();
        let mut econ = Economy {
            game,
            units,
            stats,
            data,
            hooks,
            fields: &mut *self.fields,
            tables: self.tables,
            items: &mut *self.items,
        };
        let mut w = EconomyQuests::new(&mut econ, &mut *self.rest);
        self.quests.update(&mut w);
        self.run_quest_objects(game);
    }

    fn advance_environment(&mut self, game: &mut Game, act: u8) -> bool {
        self.sim.advance_environment(game, act)
    }
    fn environment_changed(&mut self, game: &mut Game, act: u8, client: ClientId) {
        self.sim.environment_changed(game, act, client)
    }
    fn ambient_spawns(&mut self, game: &mut Game, room: RoomId) {
        self.sim.ambient_spawns(game, room)
    }
    fn spawn_presets(&mut self, game: &mut Game, room: RoomId) {
        self.sim.spawn_presets(game, room);
        self.run_quest_objects(game);
    }
    fn restore_inactive_units(&mut self, game: &mut Game, room: RoomId) {
        self.sim.restore_inactive_units(game, room);
        self.run_quest_objects(game);
    }
    fn populate_objects(&mut self, game: &mut Game, room: RoomId) {
        self.sim.populate_objects(game, room);
        self.run_quest_objects(game);
    }
    fn populate_monsters(&mut self, game: &mut Game, room: RoomId) {
        self.sim.populate_monsters(game, room);
        self.run_quest_objects(game);
    }
    fn client_room_ready(&mut self, game: &mut Game, client: ClientId) -> bool {
        self.sim.client_room_ready(game, client)
    }
    fn send_load_complete(&mut self, game: &mut Game, client: ClientId) {
        self.sim.send_load_complete(game, client)
    }
    fn refresh_inventory(&mut self, game: &mut Game, client: ClientId) {
        self.sim.refresh_inventory(game, client)
    }
    fn join_sequence(&mut self, game: &mut Game, client: ClientId) {
        self.sim.join_sequence(game, client)
    }
    fn send_removed_units(&mut self, game: &mut Game, client: ClientId) {
        self.sim.send_removed_units(game, client)
    }
    fn send_unit_update(&mut self, game: &mut Game, client: ClientId, unit: UnitId) {
        self.sim.send_unit_update(game, client, unit)
    }
    fn update_room_end(&mut self, game: &mut Game, client: ClientId, room: RoomId) {
        self.sim.update_room_end(game, client, room)
    }
    fn client_update_messages(&mut self, game: &mut Game, client: ClientId) {
        self.sim.client_update_messages(game, client)
    }
    fn client_level_change(&mut self, game: &mut Game, client: ClientId) {
        self.sim.client_level_change(game, client)
    }
    fn arena_sync(&mut self, game: &mut Game, client: ClientId) {
        self.sim.arena_sync(game, client)
    }
    fn unit_update(&mut self, game: &mut Game, unit: UnitId) {
        self.sim.unit_update(game, unit)
    }
    fn clear_arena_flag(&mut self, game: &mut Game) {
        self.sim.clear_arena_flag(game)
    }
    fn end_of_tick(&mut self, game: &mut Game) {
        self.sim.end_of_tick(game)
    }
    fn free_removal_records(&mut self, game: &mut Game, room: RoomId) {
        self.sim.free_removal_records(game, room)
    }
    fn room_inactivity(&mut self, game: &mut Game, room: RoomId) -> u32 {
        self.sim.room_inactivity(game, room)
    }
    fn act_allows_room_removal(&mut self, game: &mut Game, act: u8, room: RoomId) -> bool {
        self.sim.act_allows_room_removal(game, act, room)
    }
    fn compress_unit(&mut self, game: &mut Game, unit: UnitId) {
        self.sim.compress_unit(game, unit)
    }
    fn room_deactivated(&mut self, game: &mut Game, act: u8, room: RoomId) {
        self.sim.room_deactivated(game, act, room)
    }
    fn free_inactive_rooms(&mut self, game: &mut Game, act: u8) {
        self.sim.free_inactive_rooms(game, act)
    }
    fn delete_inactive_items(&mut self, game: &mut Game, act: u8) {
        self.sim.delete_inactive_items(game, act)
    }
    fn expire_inactive_unit_items(&mut self, game: &mut Game, act: u8) {
        self.sim.expire_inactive_unit_items(game, act)
    }
}
