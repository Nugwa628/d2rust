//! [`ActionSim`]: the one dispatcher the tick runs. Timer events go to the
//! unit dispatch (the per-kind handler tables and the monster freeze
//! drop), whose hooks run the missile class handler, the AI think and the
//! AI reset ([`super::units`]). The tick's room-inactivity hooks, the
//! compress and the client room change run the DRLG ([`super::rooms`]);
//! every other tick hook keeps its default.

use std::sync::Arc;

use crate::game::Game;
use crate::missiles;
use crate::monsters::ai;
use crate::stats::StatData;
use crate::tick::timer::TimerRun;
use crate::tick::{EventDispatch, TickHooks};
use crate::units::dispatch::UnitSystem;
use crate::units::hooks::UnitData;
use crate::units::lists::client_state;
use crate::units::{ClientId, RoomId, UnitId, UnitType};
use crate::world::objects::{ObjectControl, ObjectTables};

use super::combat::CombatView;
use super::objects::{ObjectCase, ObjectState, ObjectView};
use super::vitals_sync;
use super::waypoints::WaypointView;
use super::{ActionHooks, ActionTables, Pending, View, WiringError};

/// The action systems of one game, as one [`EventDispatch`] and
/// [`TickHooks`] for [`crate::tick::tick`].
pub struct ActionSim<X> {
    /// The unit system with the action hooks.
    pub sys: UnitSystem<ActionHooks<X>>,
}

impl<X: Pending> ActionSim<X> {
    /// Sends `m` to the player `p` after the host's item update pass when
    /// the player has item messages pending (unit flag-ex bit 0) and the
    /// host defers ([`ActionHooks::defer_player_tail`]): in the game the
    /// item messages are part of the player's unit update, before the rest
    /// of the client update.
    fn send_after_items(&mut self, p: UnitId, m: &[u8]) {
        let h = &mut self.sys.hooks;
        let pending = self.sys.units.get(p).is_some_and(|r| r.flags2 & 1 != 0);
        if h.defer_player_tail && pending {
            h.player_tail.push((p, m.to_vec()));
        } else {
            h.x.send(p, m);
        }
    }

    /// Sends the player's state / stat messages held by
    /// [`ActionHooks::defer_player_tail`]: the host calls it after its item
    /// update pass.
    pub fn flush_player_tail(&mut self) {
        for (receiver, m) in std::mem::take(&mut self.sys.hooks.player_tail) {
            self.sys.hooks.x.send(receiver, &m);
        }
        // The room clean-up's state-changed-bit step for the players whose
        // update has now run.
        for u in std::mem::take(&mut self.sys.hooks.player_state_done) {
            self.sys.stats.clear_states_changed(u);
        }
    }

    /// Creates the action systems of a game.
    pub fn new(stat_data: Arc<StatData>, data: UnitData, hooks: ActionHooks<X>) -> Self {
        Self {
            sys: UnitSystem::new(stat_data, data, hooks),
        }
    }

    /// The shared state.
    pub fn hooks(&mut self) -> &mut ActionHooks<X> {
        &mut self.sys.hooks
    }

    /// Runs `f` on a [`View`] of the unit side (allocation, removal,
    /// stats, states) with the game.
    pub fn with<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut Game, &mut View<'_, X>) -> R,
    ) -> R {
        let s = &mut self.sys;
        let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
        f(game, &mut v)
    }

    /// The join's Iron Golem re-summon for `player`
    /// ([`Pending::golem_resummon`]).
    pub fn golem_resummon(&mut self, game: &mut Game, player: UnitId) -> bool {
        let s = &mut self.sys;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        X::golem_resummon(&mut s.hooks, &mut sim, player)
    }

    /// The save load's right-skill aura start of `player`
    /// ([`Pending::right_aura_select`]).
    pub fn right_aura_select(&mut self, game: &mut Game, player: UnitId) {
        let s = &mut self.sys;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        X::right_aura_select(&mut s.hooks, &mut sim, player)
    }

    /// The save load's passive states of `unit`
    /// ([`Pending::passive_refresh_all`]).
    pub fn passive_refresh_all(&mut self, game: &mut Game, unit: UnitId) {
        let s = &mut self.sys;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        X::passive_refresh_all(&mut s.hooks, &mut sim, unit)
    }

    /// The passive states' unit bits at the end of the join
    /// ([`Pending::passive_states_on`]).
    pub fn passive_states_on(&mut self, game: &mut Game, unit: UnitId) {
        let s = &mut self.sys;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        X::passive_states_on(&mut s.hooks, &mut sim, unit)
    }

    /// The pet follow of the summoned pet types
    /// ([`Pending::summon_follow`]).
    pub fn summon_follow(&mut self, game: &mut Game, player: UnitId) {
        let s = &mut self.sys;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        X::summon_follow(&mut s.hooks, &mut sim, player)
    }

    /// The aura of the loaded right skill ([`Pending::assign_right_aura`]),
    /// after the load selected the hands.
    pub fn assign_right_aura(&mut self, game: &mut Game, unit: UnitId) {
        let s = &mut self.sys;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        X::assign_right_aura(&mut s.hooks, &mut sim, unit)
    }

    /// Runs `f` with the missile code's context (creation from skills,
    /// tests).
    pub fn missiles<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut Game, &mut missiles::Ctx<'_, View<'_, X>>) -> R,
    ) -> Option<R> {
        let s = &mut self.sys;
        let mut store = s.hooks.missiles.take()?;
        let t = s.hooks.tables.clone();
        let r = {
            let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut v,
            };
            f(game, &mut cx)
        };
        s.hooks.missiles = Some(store);
        Some(r)
    }

    /// Runs `f` with the AI code's context (AI install at monster
    /// creation; tests).
    pub fn ai<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut Game, &mut ai::Ctx<'_, View<'_, X>>) -> R,
    ) -> Option<R> {
        let s = &mut self.sys;
        let mut store = s.hooks.ai.take()?;
        let t = s.hooks.tables.clone();
        let info = s.hooks.ai_info;
        let r = {
            let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
            let mut cx = ai::Ctx {
                tables: ai::AiTables {
                    monstats: &t.combat.monstats,
                    monstats2: &t.combat.monstats2,
                    levels: &t.levels,
                    skill_modes: &t.skill_modes,
                    skills: &t.skills.skills,
                    missiles: &t.skills.missiles,
                },
                info,
                store: &mut store,
                world: &mut v,
            };
            f(game, &mut cx)
        };
        s.hooks.ai = Some(store);
        Some(r)
    }

    /// Runs `f` with combat's view and tables (skill functions, tests).
    pub fn combat<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut CombatView<'_, X>, &ActionTables) -> R,
    ) -> R {
        let s = &mut self.sys;
        let t = s.hooks.tables.clone();
        let mut w = CombatView {
            game,
            v: View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks),
        };
        f(&mut w, &t)
    }

    /// Runs `f` with the waypoint code's view (object init/operate, the
    /// C→S 0x49 handler).
    pub fn waypoints<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut WaypointView<'_, X>) -> R,
    ) -> R {
        let s = &mut self.sys;
        let mut w = WaypointView {
            game,
            v: View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks),
        };
        f(&mut w)
    }

    /// Game creation's object control: one step of the game seed ([`ActionHooks::game_seed`]),
    /// the control, the shrine lists and the level regions from `tables`.
    ///
    /// Game creation derives, in order, the monster regions, this control, the NPC control and
    /// the quest control, each from one game-seed step; `WorldSim::create_game` runs that
    /// sequence.
    pub fn create_objects(&mut self, tables: Arc<ObjectTables>) {
        let h = &mut self.sys.hooks;
        h.objects = Some(ObjectState::new(&mut h.game_seed, tables));
    }

    /// A host holding the quest control takes the object module's quest
    /// routes from a queue ([`ActionSim::take_quest_calls`]) instead of
    /// [`super::Pending::object_route`]; call right after
    /// [`ActionSim::create_objects`]. No object state: nothing.
    pub fn route_quest_objects(&mut self) {
        if let Some(st) = self.sys.hooks.objects.as_mut() {
            st.route_quests();
        }
    }

    /// The queued quest routes ([`super::QuestObjectCall`]), in order.
    pub fn take_quest_calls(&mut self) -> Vec<super::QuestObjectCall> {
        self.sys
            .hooks
            .objects
            .as_mut()
            .map(|s| s.take_quest_calls())
            .unwrap_or_default()
    }

    /// Runs `f` with the object control, the tables and the object
    /// code's view (tests, skills that call the dispatch directly).
    /// `None`: no object state.
    pub fn objects<R>(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut ObjectControl, &ObjectTables, &mut ObjectView<'_, X>) -> R,
    ) -> Option<R> {
        let s = &mut self.sys;
        let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
        super::objects::with_objects(game, &mut v, f)
    }

    /// The C→S 0x13 object case ([`View::object_message`]). `None`: no object state.
    pub fn operate_object_message(
        &mut self,
        game: &mut Game,
        player: UnitId,
        guid: u32,
    ) -> Option<ObjectCase> {
        self.with(game, |g, v| v.object_message(g, player, guid))
    }

    /// The C→S 0x13 tile case ([`View::warp_tile_message`]). `None`: no
    /// path provider.
    pub fn warp_tile_message(&mut self, game: &mut Game, player: UnitId, guid: u32) -> Option<u32> {
        self.with(game, |g, v| v.warp_tile_message(g, player, guid))
    }

    /// The Town Portal cast of `player` without an item
    /// ([`View::town_portal_cast`]): the pair's
    /// units (object 1 next to the player, object 2 in town). `None`:
    /// refused or not made.
    pub fn open_town_portal(
        &mut self,
        game: &mut Game,
        player: UnitId,
    ) -> Option<(UnitId, UnitId)> {
        self.with(game, |g, v| {
            let (made, _) = v.town_portal_cast(g, player);
            if made == 0 {
                return None;
            }
            let g1 = v.h.portals.player_portal(player)?;
            let o1 = g.lists.find_unit(crate::units::UnitType::Object, g1)?;
            Some((o1, v.portal_partner(g, o1)?))
        })
    }

    fn log(&mut self, r: Result<(), WiringError>) {
        if let Err(e) = r {
            self.sys.hooks.errors.push(e);
        }
    }
}

impl<X: Pending> EventDispatch for ActionSim<X> {
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        self.sys.run_event(game, run);
    }
}

/// Flag-ex bit 21: the per-client update's inventory refresh (set by the
/// join's item messages).
pub const INVENTORY_REFRESH_EX: u32 = 0x0020_0000;

impl<X: Pending> ActionSim<X> {
    /// The freed ground items' removal records (a picked-up gold pile or
    /// potion gets its S→C 0x0A in the next per-client update).
    fn send_removed_items(&mut self, game: &mut Game, client: ClientId) {
        let h = &mut self.sys.hooks;
        if h.removed_items.is_empty() {
            return;
        }
        let Some(c) = game.lists.client(client) else {
            return;
        };
        let Some(player) = c.player else {
            return;
        };
        let adjacent = c
            .room
            .and_then(|r| game.lists.room(r))
            .map(|r| r.adjacent.clone())
            .unwrap_or_default();
        for &(guid, room) in &h.removed_items {
            if adjacent.contains(&room) {
                let m = crate::units::messages::remove_unit(UnitType::Item as u8, guid);
                h.x.send(player, &m);
            }
        }
    }
}

impl<X: Pending> TickHooks for ActionSim<X> {
    /// Per-client update removals: S→C 0x0A (`messages::remove_unit`) to the client's player
    /// for each removal record in the client room's adjacent rooms: the records are the freed
    /// ground items' (`ActionHooks::removed_items`).
    fn send_removed_units(&mut self, game: &mut Game, client: ClientId) {
        self.send_removed_items(game, client);
        // The room delete lists ([`View::send_room_deletes`]).
        self.with(game, |g, v| v.send_room_deletes(g, client));
    }

    /// Room clean-up: the room's removal records are freed.
    fn free_removal_records(&mut self, _: &mut Game, room: RoomId) {
        self.sys.hooks.removed_items.retain(|&(_, r)| r != room);
        self.sys.hooks.room_deletes.remove(&room);
    }

    /// The first step of the tick: the act's environment record advanced
    /// with `A` = the act index. An act no join has built yet
    /// ([`crate::units::lists::ActEntry::built`]) does not exist in the
    /// game and is skipped.
    fn advance_environment(&mut self, game: &mut Game, act: u8) -> bool {
        let changed = game
            .lists
            .act_mut(act)
            .filter(|a| a.built)
            .is_some_and(|a| a.environment.server_advance(act));
        if let Some(t) = game
            .lists
            .act(act)
            .filter(|a| a.built)
            .map(|a| a.environment.time())
        {
            // Players and monsters (a hireling's worn by-time items): the
            // game takes t from any stat-list owner standing in an act,
            // whatever its type.
            for ty in [UnitType::Player, UnitType::Monster] {
                for u in game.lists.units_of_type(ty) {
                    let in_act = game
                        .lists
                        .unit(u)
                        .and_then(|e| e.room())
                        .and_then(|r| game.lists.room(r))
                        .is_some_and(|r| r.act == act);
                    if in_act {
                        self.sys.hooks.act_times.insert(u, t);
                    }
                }
            }
            // Freed units keep no t.
            self.sys
                .hooks
                .act_times
                .retain(|u, _| game.lists.unit(*u).is_some());
        }
        changed
    }

    /// The environment change per client: the player's items refreshed
    /// ([`Pending::environment_refresh_items`]), then the 0x53 with the
    /// record's values when the client
    /// is in game (state 4) in that act (its room's act).
    fn environment_changed(&mut self, game: &mut Game, act: u8, client: ClientId) {
        let Some(c) = game.lists.client(client) else {
            return;
        };
        let (state, room) = (c.state, c.room);
        let Some(player) = c.player else {
            return;
        };
        self.sys.hooks.x.environment_refresh_items(player);
        // The refresh ends in the owner refresh: queue the player for
        // update and set its flag-ex bits 0 and 1, so the item pass of the
        // client pass sends 0x47 / 0x48 (after the tick's 0x53).
        let _ = game.lists.queue_update(player);
        if let Some(r) = self.sys.units.get_mut(player) {
            r.flags2 |= 3;
        }
        let in_act = room
            .and_then(|r| game.lists.room(r))
            .is_some_and(|r| r.act == act);
        if state != client_state::IN_GAME || !in_act {
            return;
        }
        if let Some(a) = game.lists.act(act) {
            let m = a.environment.message();
            self.sys.hooks.x.send(player, &m);
        }
    }

    /// The tick's room-inactivity check.
    fn room_inactivity(&mut self, game: &mut Game, room: RoomId) -> u32 {
        match self.sys.hooks.drlg.room_inactivity(game, room) {
            Ok(n) => n,
            Err(e) => {
                self.sys.hooks.errors.push(e);
                0
            }
        }
    }

    /// The tick's check whether the act allows room removal.
    fn act_allows_room_removal(&mut self, _: &mut Game, act: u8, room: RoomId) -> bool {
        match self.sys.hooks.drlg.allows_removal(act, room) {
            Ok(b) => b,
            Err(e) => {
                self.sys.hooks.errors.push(e);
                false
            }
        }
    }

    /// The tick's room deactivation, the rest of it.
    fn room_deactivated(&mut self, game: &mut Game, act: u8, room: RoomId) {
        let r = self
            .sys
            .hooks
            .drlg
            .remove_active_room(&mut game.lists, act, room);
        self.log(r);
    }

    /// The tick's compress of one unit on the inactive store
    /// ([`ActionSim::compress`]; only warp tiles while the store is off).
    fn compress_unit(&mut self, game: &mut Game, unit: UnitId) {
        self.compress(game, unit);
    }

    /// The tick's freeing of inactive rooms of an act.
    fn free_inactive_rooms(&mut self, _: &mut Game, act: u8) {
        let r = self.sys.hooks.drlg.free_inactive_rooms(act);
        self.log(r);
    }

    /// Per-client update: with the path provider on, a player's movement
    /// messages ([`crate::wiring::path::walk::update_messages`]) and a
    /// monster's mode message ([`View::monster_update`]); objects: the
    /// object update pass ([`View::object_update`]) to the client's
    /// player. Without the provider no unit has a path record,
    /// so no player or monster message is built.
    fn update_room_end(&mut self, game: &mut Game, client: ClientId, room: RoomId) {
        let h = &mut self.sys.hooks;
        if h.item_marks && h.store_anchor.is_some_and(|a| a.0 == room) {
            h.store_anchor = None;
            if let Some(p) = game.lists.client(client).and_then(|c| c.player) {
                let mut m = [0u8; 5];
                m[0] = super::STORE_ITEMS_MARK;
                h.x.send(p, &m);
            }
        }
    }

    fn send_unit_update(&mut self, game: &mut Game, client: ClientId, unit: UnitId) {
        let s = &mut self.sys;
        let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
        // A unit not yet announced (unit flag 0x10), other than the
        // client's player: a missile sends nothing at all; any other type
        // its add messages first. A monster's are sent by its update
        // ([`View::monster_update`], which also reads "announced" for its
        // changed-state messages).
        let receiver = game.lists.client(client).and_then(|c| c.player);
        // An item in the walk is announced at its place in the queue order;
        // the host does it from this mark.
        if v.h.item_marks {
            if let (Some(p), Some(r)) = (receiver, v.units.get(unit)) {
                if r.ty == UnitType::Item {
                    let mut m = [super::GROUND_ITEM_MARK; 5];
                    m[1..].copy_from_slice(&r.guid.to_le_bytes());
                    v.h.x.send(p, &m);
                }
            }
        }
        if v.h.item_marks && v.h.store_anchor.is_some_and(|a| a.1 == Some(unit)) {
            v.h.store_anchor = None;
            if let Some(p) = receiver {
                let mut m = [0u8; 5];
                m[0] = super::STORE_ITEMS_MARK;
                v.h.x.send(p, &m);
            }
        }
        let new = v
            .units
            .get(unit)
            .filter(|r| r.flags & crate::units::record::flags::SEED_SET != 0)
            .map(|r| r.ty);
        if let (Some(ty), Some(p)) = (new, receiver) {
            if p != unit {
                match ty {
                    UnitType::Missile => return,
                    UnitType::Monster => {}
                    // An item's first announcement is the walk mark above.
                    UnitType::Item => {}
                    _ => v.add_messages(game, p, unit),
                }
            }
        }
        let is_player = v.units.get(unit).is_some_and(|r| r.ty == UnitType::Player);
        // The changed-state messages of a unit that is not new to the
        // client (a player's: below; a monster's: at the end of its update,
        // after the mode message).
        let monster_update = v.h.paths.is_some()
            && game
                .lists
                .unit(unit)
                .is_some_and(|e| e.ty == UnitType::Monster);
        if !is_player {
            // A hireling's or disguised unit's skill stat callbacks: refresh
            // all before its state messages, as a player's below.
            drain_skill_refresh(&mut v, game, unit);
        }
        if let (Some(p), None, false, false) = (receiver, new, is_player, monster_update) {
            v.state_change_messages(p, unit);
        }
        if is_player {
            // The player's update order: the path part, the event records,
            // sound, overhead text and soft hit, then the changed-state
            // messages (any state-changed bit, whether announced or not),
            // the overlay and the stat sends.
            let paths = v.h.paths.is_some();
            if paths {
                crate::wiring::path::walk::update_messages(&mut v, game, client, unit);
            }
            if let Some(p) = receiver {
                // The pending event records, e.g. the 0xA5 landing message of
                // a failed Whirlwind start; then the sound (0x400), the
                // overhead text (0x100) and the soft hit (0x8000), in that
                // order.
                v.send_event_records(game, p, unit);
                if paths {
                    crate::wiring::path::walk::sound_mark(&mut v, game, client, unit);
                }
                // Unit flag 0x100: the overhead text (set by C->S 0x14,
                // cleared by the stat sends).
                if v.units.get(unit).is_some_and(|r| r.flags & 0x100 != 0) {
                    if let Some(e) = game.lists.unit(unit) {
                        let (ty, guid) = (e.ty as u8, e.guid);
                        v.overhead_message(p, unit, ty, guid);
                    }
                }
                if paths {
                    crate::wiring::path::walk::soft_hit_message(&mut v, game, client, unit);
                }
                // A unit still new to the client keeps its join order; so
                // does one without item messages pending (update bit 0).
                // Skill stat callbacks (stats 83, 126, 127, 188) raised
                // since the last update: refresh all, then the messages.
                drain_skill_refresh(&mut v, game, unit);
                v.h.capture_tail =
                    new.is_none() && v.units.get(unit).is_some_and(|r| r.flags2 & 1 != 0);
                v.state_change_messages(p, unit);
                // The overlay flag: S→C 0x11 with the overlay list's stat 178 when it is in
                // 0..=overlay count, the unit's own type and GUID.
                v.player_overlay_message(p, unit);
                v.player_stat_sends(p, unit);
                v.h.capture_tail = false;
                if v.h.defer_player_tail && !v.h.player_state_done.contains(&unit) {
                    v.h.player_state_done.push(unit);
                }
            }
            return;
        }
        if game
            .lists
            .unit(unit)
            .is_some_and(|e| e.ty == UnitType::Object)
        {
            if let Some(receiver) = game.lists.client(client).and_then(|c| c.player) {
                v.object_update(game, receiver, unit);
                // Unit flag 0x400: the sound message.
                if let Some(m) = crate::units::sound::sound_message(game, unit, receiver) {
                    v.h.x.send(receiver, &m);
                }
                // The pending event records are always sent.
                v.send_event_records(game, receiver, unit);
            }
            return;
        }
        if v.h.paths.is_none() {
            return;
        }
        if game
            .lists
            .unit(unit)
            .is_some_and(|e| e.ty == UnitType::Monster)
        {
            v.monster_update(game, client, unit);
        }
    }

    /// Per-client update, after the unit updates: the player's
    /// stat-change messages, the flush of its changed-stat array
    /// ([`vitals_sync::mod_stat_messages`]); then, only when the player's
    /// flag-ex bit 21 is set, the inventory refresh, whose send ends with
    /// S→C 0x48 (type 0, arg 0, the player's GUID).
    ///
    /// The refresh's item and skill steps are not run here: they belong to
    /// the host's inventory model (the move's inventory pass), which the
    /// tick hooks do not hold; its 0x48 is sent at the game's place.
    /// For example, equipping a +2 all skills cap gives 0x9D 0x06, 0x47,
    /// one 0x48 (type 0, the player's GUID), then the 0xA8 of the player's
    /// re-applied states, and no skill message (0x22 / 0x23).
    fn client_update_messages(&mut self, game: &mut Game, client: ClientId) {
        let Some(p) = game.lists.client(client).and_then(|c| c.player) else {
            return;
        };
        let Some(r) = self.sys.units.get(p) else {
            return;
        };
        let (guid, refresh) = (r.guid, r.flags2 & INVENTORY_REFRESH_EX != 0);
        // The removal notices of the units the host freed: sent here, once,
        // in the next per-client update (for example the hireling a
        // resurrect frees gets its 0x0A from the adjacent-room walk).
        // PROVISIONAL: to every player; the game sends a record only to the
        // clients whose room's adjacent rooms hold it. Equal
        // in single player, where the freed hireling is always beside its
        // player (it follows every warp, measured); a second client is
        // the oracle left.
        for (ty, g) in std::mem::take(&mut self.sys.hooks.removal_notices) {
            let m = crate::units::messages::remove_unit(ty, g);
            for q in game.lists.units_of_type(UnitType::Player) {
                self.sys.hooks.x.send(q, &m);
            }
        }
        let msgs = vitals_sync::mod_stat_messages(&self.sys.stats.mod_values(p));
        for m in &msgs {
            self.send_after_items(p, m);
        }
        if refresh {
            self.sys.hooks.x.send(
                p,
                &crate::items::moves::layouts::relator2(UnitType::Player as u8, 0, guid),
            );
        }
    }

    /// Per-client update: with arena flag 0x400 raised, S→C 0x65 for the
    /// client's player when its arena record's flag is set.
    fn arena_sync(&mut self, game: &mut Game, client: ClientId) {
        let Some(st) = self.sys.hooks.arena.as_ref().filter(|s| s.flag_400) else {
            return;
        };
        let Some(p) = game.lists.client(client).and_then(|c| c.player) else {
            return;
        };
        let Some(&(score, true)) = st.records.get(&p) else {
            return;
        };
        let Some(guid) = self.sys.units.get(p).map(|r| r.guid) else {
            return;
        };
        let m = crate::units::messages::player_kill_count(guid, score as u16);
        self.send_after_items(p, &m);
    }

    /// Last in the tick's unit pass: arena flag 0x400:= 0.
    fn clear_arena_flag(&mut self, _: &mut Game) {
        if let Some(st) = self.sys.hooks.arena.as_mut() {
            st.flag_400 = false;
        }
    }

    /// The end of the tick: a mod game's periodic event hook, when due
    /// ([`ActionSim::periodic_events`]).
    fn end_of_tick(&mut self, game: &mut Game) {
        self.periodic_events(game);
    }

    /// The unit pass of the room clean-up: the flag part
    /// ([`View::room_cleanup`]) and unit flag 0x400, the sound slot
    /// ([`crate::units::sound`]). A player's slot is read by its unit
    /// update after the item messages, which the server runs after the tick
    /// (`d2-server` `handlers::items::moves::update_pass`); that pass
    /// clears it.
    fn unit_update(&mut self, game: &mut Game, unit: UnitId) {
        if game
            .lists
            .unit(unit)
            .is_some_and(|e| e.ty != UnitType::Player)
        {
            game.sounds.clear(unit);
        }
        let s = &mut self.sys;
        View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks).room_cleanup(unit);
    }

    /// Per-client update: the player's room differs from the client's. When the two rooms'
    /// level ids differ, quest event 3 CHANGEDLEVEL (`from`, `to`, `player`) runs first, on the
    /// host's lent quest control ([`super::objects::QuestObjectHost::changed_level`]; none
    /// lent: nothing); then the room switch to the player's room ([`View::room_switch`]).
    ///
    /// Then the town-leave refresh (`from`, `to`): its vendor part is queued for the host that
    /// holds the vendor records ([`super::ActionHooks::town_leaves`]), its quest part runs on
    /// the lent quest control ([`super::objects::QuestObjectHost::town_leave`]).
    fn client_level_change(&mut self, game: &mut Game, client: ClientId) {
        let Some(c) = game.lists.client(client) else {
            return;
        };
        let (player, client_room) = (c.player, c.room);
        let new = player
            .and_then(|p| game.lists.unit(p))
            .and_then(|u| u.room());
        let level =
            |r: Option<RoomId>, h: &ActionHooks<X>| r.and_then(|r| h.drlg.level_id(game, r));
        let (from, to) = (
            level(client_room, &self.sys.hooks),
            level(new, &self.sys.hooks),
        );
        if game.coverage.enabled {
            if let (Some(_), Some(to)) = (player, to) {
                if from != Some(to) {
                    game.coverage.hit("areas_loaded", || to);
                }
            }
        }
        if let (Some(p), Some(from), Some(to)) = (player, from, to) {
            if from != to {
                self.with(game, |g, v| {
                    if let Some(mut host) = v.h.quest_host.take() {
                        host.changed_level(g, v, p, from, to);
                        v.h.quest_host = Some(host);
                    }
                    let empty = super::town_left_empty(g, v.stats, v.h, from);
                    v.h.town_leaves.push((p, from, empty));
                    if let Some(mut host) = v.h.quest_host.take() {
                        host.town_leave(g, v, p, to);
                        v.h.quest_host = Some(host);
                    }
                });
            }
        }
        self.with(game, |g, v| v.room_switch(g, client, new));
        // A mod game's level hook, after D2RUST's level change.
        if let (Some(p), Some(to)) = (player, to) {
            if from != Some(to) {
                self.mod_event(game, |e, h| e.level_entered(h, p, from, to));
            }
        }
    }

    /// The client's room is ready ([`View::client_room_ready`]).
    fn client_room_ready(&mut self, game: &mut Game, client: ClientId) -> bool {
        self.with(game, |g, v| v.client_room_ready(g, client))
    }

    /// After the client reaches state 4: the inventory refresh (the
    /// second 0x48 of the join), whose send ends with S→C 0x48 (type 0, arg 0, the
    /// player's GUID). The pass's item steps run on the host (see
    /// `client_update_messages`).
    fn refresh_inventory(&mut self, game: &mut Game, client: ClientId) {
        let Some(p) = game.lists.client(client).and_then(|c| c.player) else {
            return;
        };
        let Some(guid) = self.sys.units.get(p).map(|r| r.guid) else {
            return;
        };
        let m = crate::items::moves::layouts::relator2(UnitType::Player as u8, 0, guid);
        self.sys.hooks.x.send(p, &m);
    }

    /// S→C 0x04 LoadComplete to the client's player.
    fn send_load_complete(&mut self, game: &mut Game, client: ClientId) {
        if let Some(p) = game.lists.client(client).and_then(|e| e.player) {
            self.sys
                .hooks
                .x
                .send(p, &crate::units::messages::LOAD_COMPLETE);
        }
    }

    /// Step 5, the join sequence, J = the joiner:
    /// for each other client C with a player (any state,
    /// client-list order) S→C 0x5B of P(C) to J, 0x5B of P(J) to C, then
    /// to J one 0x8E CorpseAssign per corpse of P(C); then 0x5B of P(J)
    /// to J. Then to J one 0x65 per state-4 client's player
    /// (list order). Then to J one 0x8D (GUID, party word; D2RUST
    /// has no parties) per player unit without state 7 (hash-bucket
    /// order). Then the join 0x5A (code 2, when the name has a NUL in its
    /// 16 bytes) to every state-4 client, the joiner included. A
    /// single-player join sends 0x5B, 0x65, 0x8D, 0x5A to the one client.
    /// The 0x8E flag byte is the constant 1; bytes 2–5 are the corpse owner's GUID, 6–9
    /// the corpse id.
    fn join_sequence(&mut self, game: &mut Game, client: ClientId) {
        use crate::units::lists::client_state::IN_GAME;
        use crate::units::messages as m;
        let Some(j) = game.lists.client(client).and_then(|e| e.player) else {
            return;
        };
        let clients: Vec<(ClientId, Option<UnitId>, u32)> = game
            .lists
            .clients()
            .into_iter()
            .filter_map(|c| game.lists.client(c).map(|e| (c, e.player, e.state)))
            .collect();
        let joined = |a: &Self, p: UnitId| -> Option<Vec<u8>> {
            let r = a.sys.units.get(p)?;
            let level = a.sys.stats.unit_total(p, 12, 0) as u16;
            let name = a
                .sys
                .hooks
                .session
                .names
                .get(&p)
                .copied()
                .unwrap_or([0; 16]);
            Some(m::player_joined(
                r.guid,
                r.class as u8,
                &name,
                level,
                m::NO_PARTY,
            ))
        };
        let Some(own) = joined(self, j) else {
            return;
        };
        let guid_of = |a: &Self, p: UnitId| a.sys.units.get(p).map_or(0, |r| r.guid);
        // Each other client with a player.
        for &(c, pc, _) in &clients {
            let Some(pc) = pc else { continue };
            if c == client {
                continue;
            }
            if let Some(theirs) = joined(self, pc) {
                self.sys.hooks.x.send(j, &theirs);
                pet_join_sync(self, game, j, pc);
            }
            self.sys.hooks.x.send(pc, &own);
            let owner = guid_of(self, pc);
            let corpses: Vec<u32> = self
                .sys
                .hooks
                .death
                .owners
                .iter()
                .filter(|&(_, &o)| o == owner)
                .map(|(&u, _)| guid_of(self, u))
                .collect();
            for g in corpses {
                let mut b = vec![0x8E, 1];
                b.extend_from_slice(&owner.to_le_bytes());
                b.extend_from_slice(&g.to_le_bytes());
                self.sys.hooks.x.send(j, &b);
            }
        }
        self.sys.hooks.x.send(j, &own);
        pet_join_sync(self, game, j, j);
        // The 0x5B tail: the joiner's hireling.
        if let Some(pets) = self.sys.hooks.session.join_pets.remove(&j) {
            for m in &pets {
                self.sys.hooks.x.send(j, m);
            }
        }
        // One kill count per in-game client's player.
        for &(_, pc, state) in &clients {
            if let (Some(pc), true) = (pc, state == IN_GAME) {
                let kills = m::player_kill_count(guid_of(self, pc), 0);
                self.sys.hooks.x.send(j, &kills);
            }
        }
        // One 0x8D per player unit without state 7.
        for q in game.lists.units_of_type(UnitType::Player) {
            if self.sys.stats.has_state(q, super::death::STATE_PLAYERBODY) {
                continue;
            }
            let party = m::assign_player_to_party(guid_of(self, q), m::NO_PARTY);
            self.sys.hooks.x.send(j, &party);
        }
        // The join 0x5A to every state-4 client.
        let name = self
            .sys
            .hooks
            .session
            .names
            .get(&j)
            .copied()
            .unwrap_or([0; 16]);
        if name.contains(&0) {
            let e = m::player_event(2, &name);
            for &(_, pc, state) in &clients {
                if let (Some(pc), true) = (pc, state == IN_GAME) {
                    self.sys.hooks.x.send(pc, &e);
                }
            }
        }
        // A mod game's join hook, after the join's messages.
        self.mod_event(game, |e, h| e.player_joined(h, j));
    }
}

/// The 0x5B tail of the join: for
/// every pet type in row order, every living node whose unit exists
/// sends 0x7A action 1 to the joiner `to` (type 7 sends 0x81, which
/// the hireling code owns). Recorded: `gen-fmt-load-golem` frame 2
/// (0x7A after the join's 0x5B).
fn pet_join_sync<X: Pending>(a: &mut ActionSim<X>, game: &Game, to: UnitId, player: UnitId) {
    let Some(owner) = a.sys.units.get(player).map(|r| r.guid) else {
        return;
    };
    let Some(lists) = a.sys.hooks.pet_lists.get(&player) else {
        return;
    };
    let mut out = Vec::new();
    for (t, e) in lists.entries.iter().enumerate().filter(|(t, _)| *t != 7) {
        for n in e.nodes.iter().filter(|n| n.flags & 1 == 0) {
            let Some(u) = u32::try_from(n.guid)
                .ok()
                .and_then(|g| game.lists.find_unit(UnitType::Monster, g))
            else {
                continue;
            };
            let class = a.sys.units.get(u).map_or(0, |r| r.class as u16);
            out.push(crate::world::hirelings::pets::pet_action(
                1,
                t as u8,
                class,
                n.guid as u32,
                owner,
            ));
        }
    }
    for m in out {
        a.sys.hooks.x.send(to, &m);
    }
}

/// The skill stat callbacks of stats 83, 126, 127 and 188 queued for
/// `unit` since its last update (refresh all), in their order; other units' stay
/// queued.
fn drain_skill_refresh<X: Pending>(v: &mut View<'_, X>, game: &mut Game, unit: UnitId) {
    if !v.h.skill_refresh.iter().any(|&(u, ..)| u == unit) {
        return;
    }
    let q = std::mem::take(&mut v.h.skill_refresh);
    for (u, stat, layer) in q {
        if u == unit {
            super::missiles::with_sim(v, game, |h, sim| {
                X::skill_stat_refresh(h, sim, u, stat, layer)
            });
        } else {
            v.h.skill_refresh.push((u, stat, layer));
        }
    }
}
