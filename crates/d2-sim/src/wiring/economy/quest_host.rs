//! Quest world on the wired host: [`HostQuests`]. Every [`QuestWorld`] call
//! goes to [`EconomyQuests`] (the economy plus the rest), except those the
//! action wiring provides:
//!
//! - the object calls on an object with object data (the object state,
//!   `wiring::action::objects`): its mode, the mode set
//!   ([`View::object_set_mode`]), the animation length (`objects.txt`
//!   `FrameCnt1`, raw), object events through the timer queue (event 7 for
//!   the quest event), the Act I object allocation with a mode
//!   ([`View::create_object`]) and the collision free (the object code's
//!   footprint free, `View::free_object_footprint`);
//! - the level of a unit in a DRLG room (`DrlgWorld::level_id`);
//! - identifying an item of the game's item store;
//! - `missiles.txt` `Range` from the action tables;
//! - the chest treasure on the object drop state (`ActionHooks::object_drops`,
//!   [`super::object_chest_drop`]);
//! - the quest helpers' host calls: the path target (path provider), the
//!   trade button, the obelisk's 0x44 cancel, the town portal GUID /
//!   partner / removal (the action wiring's portal seams), the monster mode
//!   request (the monster mode change), the item level;
//!
//! and those of the host parts a caller lends ([`HostQuests::inventory`]: the
//! reward, [`super::quest_reward`], and the cube close's inventory pass;
//! [`HostQuests::chats`]: the chat-node frees).
//!
//! Left to the rest: the Steeg Stone release (d2-sim has no object data for
//! class 337, so nothing here states that object).
//!
//! An object without object data, a unit outside a DRLG room and an item
//! outside the store keep the rest's answer.
//!
//! Status: wired, not yet verified in play.

use crate::drlg::TileRect;
use crate::missiles::{self, MissileParams};
use crate::path::place_seams::CollisionView;
use crate::rng::Seed;
use crate::tick::events::event;
use crate::units::hooks::UnitHooks;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::{ActionHooks, Pending, View};
use crate::wiring::path::place::Rooms;
use crate::world::quests::helpers::{self, QuestMissile};
use crate::world::quests::{PlayerQuests, QuestChain, QuestWorld, UnitKind};

use super::quest_reward::{self, Placed, QuestInventory};
use super::{EconomyQuests, QuestRest};
use crate::items::moves::Spot;
use crate::monsters::ai::seams::{AiModes, ModeTarget};
use crate::world::cube::trade_action;
use crate::world::npc::InteractionList;
use crate::world::quests::act2;
use std::collections::BTreeMap;

/// Stat 6 (`hitpoints`), zeroed when Nihlathak is killed in town.
const STAT_HITPOINTS: u16 = 6;
/// Unit mode 12 (dead).
const MODE_DEAD: u32 = 12;
/// Fire, lightning, cold, poison resist stats.
const RESIST_STATS: [u16; 4] = [39, 41, 43, 45];

/// [`EconomyQuests`] on the action wiring's hooks, with the calls the
/// action wiring provides answered there (module doc), and the host
/// parts a caller may lend for the call.
pub struct HostQuests<'e, 'a, X, R> {
    /// The economy quest world the host answers from.
    pub inner: EconomyQuests<'e, 'a, ActionHooks<X>, R>,
    /// The host's inventory model (the reward, the cube close's inventory
    /// pass); `None`: the rest's answers.
    pub inventory: Option<&'e mut dyn QuestInventory<ActionHooks<X>>>,
    /// The NPCs' interaction lists (`InteractionState::lists`); `None`: the
    /// chat-node calls are reported unhandled.
    pub chats: Option<&'e mut BTreeMap<UnitId, InteractionList>>,
    /// The object under its quest init and its init point
    /// ([`QuestWorld::set_init_point`]).
    pub init_point: Option<(UnitId, i32, i32, RoomId)>,
}

impl<'e, 'a, X: Pending, R: QuestRest> HostQuests<'e, 'a, X, R> {
    /// A host quest world over `inner`, with no inventory model or chat
    /// lists lent.
    pub fn new(inner: EconomyQuests<'e, 'a, ActionHooks<X>, R>) -> Self {
        Self {
            inner,
            inventory: None,
            chats: None,
            init_point: None,
        }
    }

    /// The object has object data (a created object state knows it).
    fn known(&self, object: UnitId) -> bool {
        self.inner
            .econ
            .hooks
            .objects
            .as_ref()
            .is_some_and(|s| s.control.data.contains_key(&object))
    }

    /// The active room has a DRLG room on this host.
    fn drlg_room(&self, room: RoomId) -> bool {
        let e = &*self.inner.econ;
        e.hooks.drlg.drlg_room(e.game, room).is_some()
    }

    /// Population's placement search and creation through the lent monster
    /// world (`MonsterWorld::spawn_at`): `None` when no world is lent (or
    /// population's state is out), else the placement's result.
    #[allow(clippy::too_many_arguments)]
    fn place_unit(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        r: i32,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        self.view(|g, v| {
            let mut sim = crate::units::hooks::Sim {
                game: g,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            };
            v.h.with_monster_world(|w, h| {
                w.spawn_at(&mut sim, h, room, x, y, i32::from(class), mode, r, flags)
            })
            .flatten()
        })
    }

    /// The preset-monster spawn through the lent monster world
    /// (`MonsterWorld::spawn_preset`): `None` when no world is lent (or
    /// population's state is out).
    fn preset_unit(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
    ) -> Option<Option<UnitId>> {
        self.view(|g, v| {
            let mut sim = crate::units::hooks::Sim {
                game: g,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            };
            v.h.with_monster_world(|w, h| w.spawn_preset(&mut sim, h, room, x, y, class, mode))
                .flatten()
        })
    }

    /// Runs a drop helper with the game's drop state
    /// (`ActionHooks::object_drops`) lent out and the action tables'
    /// `levels`; the economy's item store, game seed and unique bits go
    /// back to the hooks for the call (as [`Self::object_treasure`]).
    /// `None`: no drop state.
    fn with_drop_state<T>(
        &mut self,
        f: impl FnOnce(
            &mut crate::wiring::action::ActionHooks<X>,
            &mut crate::units::hooks::Sim<'_>,
            &mut super::DeathDrops,
            &[d2_data::tables::Levels],
            &mut super::StartSpot,
        ) -> T,
    ) -> Option<T> {
        let e = &mut *self.inner.econ;
        let mut d = e.hooks.object_drops.take()?;
        let at = e.hooks.tables.clone();
        std::mem::swap(&mut e.hooks.items, &mut *e.items);
        e.hooks.game_seed = e.fields.seed;
        e.hooks.uniques = std::mem::take(&mut e.fields.uniques);
        let out = {
            let mut sim = crate::units::hooks::Sim {
                game: &mut *e.game,
                units: &mut *e.units,
                stats: &mut *e.stats,
                data: e.data,
            };
            f(
                &mut *e.hooks,
                &mut sim,
                &mut d,
                &at.levels,
                &mut super::StartSpot,
            )
        };
        std::mem::swap(&mut e.hooks.items, &mut *e.items);
        e.fields.seed = e.hooks.game_seed;
        e.fields.uniques = std::mem::take(&mut e.hooks.uniques);
        e.hooks.object_drops = Some(d);
        Some(out)
    }

    /// Runs `f` on the action wiring's [`View`]. The economy holds the
    /// game's item store, game seed and unique bits for the quest call
    /// (`QuestLoan::on_world`): they go back to their home in the hooks
    /// for the call and are taken again after it, so an allocation made
    /// here (Hratli from his dummy's init, a monster's equipment) keeps
    /// its seed draws. Without this the draws were overwritten when the
    /// call ended (the Act III start's game seed fell one step behind).
    fn view<T>(&mut self, f: impl FnOnce(&mut crate::game::Game, &mut View<'_, X>) -> T) -> T {
        let e = &mut *self.inner.econ;
        std::mem::swap(&mut e.hooks.items, &mut *e.items);
        e.hooks.game_seed = e.fields.seed;
        e.hooks.uniques = std::mem::take(&mut e.fields.uniques);
        let out = {
            let mut v = View::of(&mut *e.units, &mut *e.stats, e.data, &mut *e.hooks);
            f(&mut *e.game, &mut v)
        };
        std::mem::swap(&mut e.hooks.items, &mut *e.items);
        e.fields.seed = e.hooks.game_seed;
        e.fields.uniques = std::mem::take(&mut e.hooks.uniques);
        out
    }

    /// [`Self::view`] with the economy's game seed lent to the hooks for
    /// the call (the portal pair's allocations draw from it) and taken back
    /// after it.
    fn view_seeded<T>(
        &mut self,
        f: impl FnOnce(&mut crate::game::Game, &mut View<'_, X>) -> T,
    ) -> T {
        let e = &mut *self.inner.econ;
        e.hooks.game_seed = e.fields.seed;
        let r = {
            let mut v = View::of(&mut *e.units, &mut *e.stats, e.data, &mut *e.hooks);
            f(&mut *e.game, &mut v)
        };
        e.fields.seed = e.hooks.game_seed;
        r
    }

    /// Opens a portal pair for the quest calls of the `create_portal` form
    /// ([`View::create_portal_pair`]): at (x, y) in `unit`'s room.
    ///
    /// The owner and `exact` follow the callers: a player unit is the owner
    /// with exact 0 (Tyrael's and Andariel's portals), any other unit (Anya,
    /// dummy 459) gives no owner with exact 1 (the temple portal).
    fn quest_pair(
        &mut self,
        unit: UnitId,
        (x, y): (i32, i32),
        class: u16,
        level: u32,
        tyrael: bool,
    ) -> bool {
        let room = self.inner.econ.game.lists.unit(unit).and_then(|e| e.room());
        let player = self
            .inner
            .econ
            .units
            .get(unit)
            .is_some_and(|r| r.ty == UnitType::Player);
        let owner = player.then_some(unit);
        let (made, _) = self.view_seeded(|g, v| {
            v.create_portal_pair(
                g,
                owner,
                room,
                (x, y),
                level,
                u32::from(class),
                !player,
                tyrael,
            )
        });
        made != 0
    }

    /// The quest drop ([`super::unit_quest_drop`]) with the drop code `code`
    /// when the game holds the drop state (`ActionHooks::object_drops`) and
    /// `unit` has a record; `None`: no drop state (the caller keeps the
    /// rest's answer).
    fn econ_quest_drop(
        &mut self,
        unit: UnitId,
        code: [u8; 4],
        quality: u8,
        p7: i32,
    ) -> Option<Option<UnitId>> {
        self.inner.econ.units.get(unit)?;
        self.with_drop_state(|h, sim, d, _, spots| {
            super::unit_quest_drop(h, sim, d, spots, unit, Some(code), quality, -1, p7)
        })
    }

    /// A monster of `class` allocated in `room` at (x, y) in `mode`; NPC
    /// classes count as allied. The chains of its creation-time boss mods
    /// are linked.
    fn spawn_unit(&mut self, room: RoomId, x: i32, y: i32, class: u16, mode: u8) -> Option<UnitId> {
        let allied = self
            .inner
            .econ
            .hooks
            .tables
            .combat
            .monstats
            .get(usize::from(class))
            .is_some_and(|m| m.npc);
        let req = crate::units::lifecycle::AllocRequest {
            ty: UnitType::Monster,
            class: u32::from(class),
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: u32::from(mode),
            allied,
        };
        self.view(|g, v| {
            let u = v.allocate(g, &req, x, y)?;
            v.link_boss_chains(u, req.class);
            Some(u)
        })
    }

    /// Sets mode 12 (for a monster), then the unit leaves its room with its
    /// collision freed and is kept ([`View::leave_room`]). False: not in the
    /// lists.
    fn leave_dead(&mut self, unit: UnitId) -> bool {
        let Some(ty) = self.inner.econ.game.lists.unit(unit).map(|u| u.ty) else {
            return false;
        };
        self.view(|g, v| {
            if ty == UnitType::Monster {
                v.monster_set_mode(g, unit, MODE_DEAD);
            }
            v.leave_room(g, unit, true)
        })
    }

    /// The unit's interaction ended: its record's interact info is reset.
    fn end_unit_interaction(&mut self, unit: UnitId) {
        if let Some(r) = self.inner.econ.units.get_mut(unit) {
            r.interact.reset();
        }
    }

    /// The vitals tables with a non-empty experience table and the player's
    /// class.
    fn vitals_of(
        &self,
        player: UnitId,
    ) -> Option<(std::sync::Arc<crate::combat::vitals::VitalsTables>, i32)> {
        let t = self.inner.econ.hooks.vitals.clone()?;
        let class = self.inner.econ.units.get(player)?.class as i32;
        (!t.experience.is_empty()).then_some((t, class))
    }

    /// Schedules an object event on the game's timer queue; a refused
    /// schedule is a unit error of the action wiring.
    fn schedule(&mut self, object: UnitId, ev: u8, frame: i32) {
        let e = &mut *self.inner.econ;
        if let Err(err) = e
            .game
            .schedule_event(object, u32::from(ev), frame, None, 0, 0)
        {
            let mut v = View::of(&mut *e.units, &mut *e.stats, e.data, &mut *e.hooks);
            v.unit_error(err.into());
        }
    }
}

impl<X: Pending, R: QuestRest> HostQuests<'_, '_, X, R> {
    /// Whether the player is locked out of trading: true when the unit's
    /// flags 2 has bit 0x400000 or 0x800000, or any GUID on its inventory
    /// update list names a live item unit.
    pub(super) fn trade_locked(&mut self, player: UnitId) -> bool {
        const TRADE_BITS: u32 = 0x40_0000 | 0x80_0000;
        if self
            .inner
            .econ
            .units
            .get(player)
            .is_some_and(|r| r.flags2 & TRADE_BITS != 0)
        {
            return true;
        }
        let guids = self
            .inventory
            .as_deref()
            .map(|i| i.update_guids(player))
            .unwrap_or_default();
        guids.into_iter().any(|g| self.unit_by_guid(4, g).is_some())
    }

    /// The quest reward on the lent inventory model ([`quest_reward`]):
    /// create, place, else drop next to the player when droppable, else
    /// free.
    fn reward_on_host(
        &mut self,
        player: UnitId,
        code: [u8; 4],
        level: i32,
        quality: u8,
        droppable: bool,
    ) -> Option<UnitId> {
        let made = quest_reward::create_reward(&mut *self.inner.econ, player, code, level, quality);
        let item = match made {
            Ok(Some(item)) => item,
            Ok(None) => return None,
            Err(e) => {
                if let Some(inv) = self.inventory.as_deref_mut() {
                    inv.fault(e);
                }
                return None;
            }
        };
        let inv = self.inventory.as_deref_mut()?;
        if let Placed::Stored(item) =
            quest_reward::place_reward(&mut *self.inner.econ, inv, player, item)
        {
            return Some(item);
        }
        let spot = if droppable {
            match self.reward_spot(player) {
                // The search found nothing: its out room is none and the drop does
                // nothing; the item stays allocated in no room and is still
                // returned.
                Some(None) => return Some(item),
                Some(Some(spot)) => Some(spot),
                None => None,
            }
        } else {
            None
        };
        let inv = self.inventory.as_deref_mut()?;
        quest_reward::drop_or_free(&mut *self.inner.econ, inv, item, spot)
    }

    /// The item search over the lent inventory model's lists; none lent:
    /// the rest's.
    fn find_on_host(&self, player: UnitId, code: [u8; 4]) -> Option<UnitId> {
        match self.inventory.as_deref() {
            Some(inv) => self
                .inner
                .find_item_in(inv.cursor_of(player), inv.items_of(player), code),
            None => self.inner.find_item(player, code),
        }
    }

    /// The reward's drop spot: [`helpers::free_spot`] from the player's path
    /// position and room, size 1, mask 0x3E01, limit 100. Outer `None`: the
    /// player has no position (the reward is freed). `Some(None)`: the search
    /// accepted nothing (or the player's room has no DRLG room here): the out
    /// room is none and the drop does nothing.
    fn reward_spot(&mut self, player: UnitId) -> Option<Option<Spot>> {
        let (x, y, room) = self.unit_position(player)?;
        let found = if self.drlg_room(room) {
            helpers::free_spot(
                self,
                room,
                x,
                y,
                quest_reward::DROP_SIZE,
                quest_reward::DROP_MASK,
                quest_reward::DROP_LIMIT,
            )
        } else {
            None
        };
        Some(found.map(|(x, y, room)| Spot { room, x, y }))
    }
}

impl<X: Pending, R: QuestRest> QuestWorld for HostQuests<'_, '_, X, R> {
    /// Applies a stored map-AI path (handle = index + 1 in the hooks'
    /// [`ActionHooks::map_ai_paths`]) as the unit's map-AI nodes. False when
    /// the handle is unknown or its path empty. The handle table stands for
    /// the game's pool copy: a pool allocation with no draw, whose pointer
    /// nothing reads but this apply, so the two are equivalent (Larzuk's
    /// Harrogath arrival walk comes out the same).
    fn apply_map_ai(&mut self, unit: UnitId, map_ai: u32) -> bool {
        let hooks = &mut *self.inner.econ.hooks;
        let Some(nodes) = (map_ai as usize)
            .checked_sub(1)
            .and_then(|i| hooks.map_ai_paths.get(i))
            .filter(|n| !n.is_empty())
            .cloned()
        else {
            return false;
        };
        let Some(c) = hooks.ai.as_mut().and_then(|ai| ai.control_mut(unit)) else {
            return false;
        };
        c.map_ai = Some(nodes);
        true
    }
    /// The mode of an object with object data.
    fn object_mode(&self, object: UnitId) -> i32 {
        if self.known(object) {
            return self
                .inner
                .econ
                .units
                .get(object)
                .map_or(0, |r| r.mode as i32);
        }
        self.inner.object_mode(object)
    }
    /// Sets the mode of an object with object data.
    fn set_object_mode(&mut self, object: UnitId, mode: i32) {
        if let (true, Ok(m)) = (self.known(object), u8::try_from(mode)) {
            if self.view(|g, v| v.object_set_mode(g, object, m)) {
                return;
            }
        }
        self.inner.set_object_mode(object, mode)
    }
    /// The animation length: the class's `objects.txt` `FrameCnt1` as stored
    /// (× 256).
    fn object_anim_length(&self, object: UnitId) -> i32 {
        if let Some(st) = self.inner.econ.hooks.objects.as_ref() {
            if let Some(d) = st.control.data.get(&object) {
                if let Ok(o) = st.tables.object(d.class) {
                    return o.framecnt1 as i32;
                }
            }
        }
        self.inner.object_anim_length(object)
    }
    fn schedule_object_event(&mut self, object: UnitId, ev: u8, frame: i32) {
        if self.known(object) {
            return self.schedule(object, ev, frame);
        }
        self.inner.schedule_object_event(object, ev, frame)
    }
    /// Schedules object event 7 (the quest event).
    fn schedule_quest_event(&mut self, object: UnitId, frame: i32) {
        if self.known(object) {
            return self.schedule(object, 7, frame);
        }
        self.inner.schedule_quest_event(object, frame)
    }
    /// Allocates an object through the object state ([`View::create_object`]:
    /// allocation and init); no object state: the rest's.
    fn spawn_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: i32,
    ) -> Option<UnitId> {
        if let (true, Ok(m)) = (self.inner.econ.hooks.objects.is_some(), u8::try_from(mode)) {
            return self.view(|g, v| v.create_object(g, room, u32::from(class), x, y, m));
        }
        self.inner.spawn_object(room, x, y, class, mode)
    }
    /// The object code's footprint free (`View::free_object_footprint`).
    fn free_object_collision(&mut self, object: UnitId) {
        if self.known(object) {
            self.view(|g, v| v.free_object_footprint(g, object));
            return;
        }
        self.inner.free_object_collision(object)
    }
    /// The room's level when the unit is in a DRLG room. An object inside
    /// its quest init is not in its room yet: the init record's room.
    fn unit_level(&self, unit: UnitId) -> Option<u32> {
        let e = &*self.inner.econ;
        let room = e
            .game
            .lists
            .unit(unit)
            .and_then(|u| u.room())
            .or(match self.init_point {
                Some((o, _, _, r)) if o == unit => Some(r),
                _ => None,
            });
        room.and_then(|r| e.hooks.drlg.level_id(e.game, r))
            .or_else(|| self.inner.unit_level(unit))
    }
    fn interact_unit(&mut self, player: UnitId) -> Option<(u8, u32)> {
        self.inner.interact_unit(player)
    }
    fn set_interact_unit(&mut self, player: UnitId, unit: Option<(u8, u32)>) {
        self.inner.set_interact_unit(player, unit)
    }
    /// Sets the identified flag on the item.
    fn identify_item(&mut self, item: UnitId) {
        match self.inner.econ.items.get_mut(item) {
            Some(i) => i.flags |= crate::items::flag::IDENTIFIED,
            None => self.inner.identify_item(item),
        }
    }

    fn frame(&self) -> i32 {
        self.inner.frame()
    }
    fn tick_rate(&self) -> crate::tick::TickRate {
        self.inner.tick_rate()
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        self.inner.rules()
    }
    fn difficulty(&self) -> u8 {
        self.inner.difficulty()
    }
    fn expansion(&self) -> bool {
        self.inner.expansion()
    }
    fn game_type(&self) -> u8 {
        self.inner.game_type()
    }
    fn has_act2(&self) -> bool {
        self.inner.has_act2()
    }
    fn players(&self) -> Vec<UnitId> {
        self.inner.players()
    }
    fn first_client_player(&self) -> Option<UnitId> {
        self.inner.first_client_player()
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.inner.guid(unit)
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.inner.player_by_guid(guid)
    }
    fn quests(&mut self, player: UnitId) -> Option<&mut PlayerQuests> {
        self.inner.quests(player)
    }
    fn unit_act(&self, unit: UnitId) -> Option<u8> {
        self.inner.unit_act(unit)
    }
    fn player_class(&self, player: UnitId) -> u8 {
        self.inner.player_class(player)
    }
    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed {
        self.inner.unit_seed(unit)
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.inner.stat(unit, stat)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.inner.base_stat(unit, stat)
    }
    fn add_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        self.inner.add_stat(unit, stat, delta)
    }
    fn attach_sound(&mut self, player: UnitId, sound: u16) {
        self.inner.attach_sound(player, sound)
    }
    fn player_byte_4c(&self, player: UnitId) -> u8 {
        self.inner.player_byte_4c(player)
    }
    fn set_player_byte_4c(&mut self, player: UnitId, v: u8) {
        self.inner.set_player_byte_4c(player, v)
    }
    fn quest_chain(&mut self, unit: UnitId) -> Option<&mut QuestChain> {
        self.inner.quest_chain(unit)
    }
    fn unit_kind(&self, unit: UnitId) -> UnitKind {
        self.inner.unit_kind(unit)
    }
    fn monster_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        self.inner.monster_by_guid(guid)
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        self.inner.monster_class(unit)
    }
    fn players_near(&self, unit: UnitId) -> Vec<UnitId> {
        self.inner.players_near(unit)
    }
    fn party_members(&self, player: UnitId) -> Option<Vec<UnitId>> {
        self.inner.party_members(player)
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.inner.send(player, msg)
    }
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, list: &[(u16, u32)]) {
        self.inner.send_text_list(player, npc, list)
    }
    fn has_item(&self, player: UnitId, code: [u8; 4]) -> bool {
        self.find_on_host(player, code).is_some()
    }
    /// The items record code of the weapon in use; none without an
    /// inventory model.
    fn wielded_weapon_code(&mut self, player: UnitId) -> Option<[u8; 4]> {
        let inv = self.inventory.as_deref()?;
        let w = inv.weapon_in_use(&*self.inner.econ, player)?;
        self.inner.item_code(w)
    }
    fn item_code(&self, item: UnitId) -> Option<[u8; 4]> {
        self.inner.item_code(item)
    }
    /// Whether `item` is the inventory model's cursor item; without one,
    /// the rest's answer.
    fn is_cursor_item(&self, player: UnitId, item: UnitId) -> bool {
        match self.inventory.as_deref() {
            Some(inv) => inv.cursor_of(player) == Some(item),
            None => self.inner.is_cursor_item(player, item),
        }
    }
    /// Deletes the found item through the lent inventory model; none lent:
    /// the rest's.
    fn delete_item(&mut self, player: UnitId, code: [u8; 4]) {
        if self.inventory.is_none() {
            return self.inner.delete_item(player, code);
        }
        if let Some(item) = self.find_on_host(player, code) {
            if let Some(inv) = self.inventory.as_deref_mut() {
                inv.delete(&mut *self.inner.econ, player, item);
            }
        }
    }
    fn reward_item(
        &mut self,
        player: UnitId,
        code: [u8; 4],
        level: i32,
        quality: u8,
        droppable: bool,
    ) -> Option<UnitId> {
        if self.inventory.is_none() {
            return self
                .inner
                .reward_item(player, code, level, quality, droppable);
        }
        self.reward_on_host(player, code, level, quality, droppable)
    }
    /// The quest drop with the drop code `code` (`Self::econ_quest_drop`);
    /// without the drop state, the rest's answer.
    fn drop_item_at(&mut self, unit: UnitId, code: [u8; 4], quality: u8) -> bool {
        match self.econ_quest_drop(unit, code, quality, 0) {
            Some(item) => {
                // The item is created at the floor-search spot and the drop's placer
                // (`death.rs` `Spots::placed`) puts it into its static path there.
                // Quest items are identified by creation itself (`hfh ` has `quest` 25,
                // `questdiffcheck` 1).
                item.is_some()
            }
            None => self.inner.drop_item_at(unit, code, quality),
        }
    }
    fn quest_items(&self, player: UnitId) -> Vec<(UnitId, u8)> {
        self.inner.quest_items(player)
    }
    fn den_region(&self) -> (u32, u32, u32, u32) {
        self.inner
            .econ
            .hooks
            .monster_world
            .as_ref()
            .and_then(|w| w.den_counts())
            .unwrap_or_else(|| self.inner.den_region())
    }
    fn true_tomb_level(&self) -> u32 {
        // The Act II DRLG's staff-tomb level (act index 1).
        match self.inner.econ.hooks.drlg.staff_tomb(1) {
            Some(level) if level != 0 => level,
            _ => self.inner.true_tomb_level(),
        }
    }
    /// [`helpers::free_spot`] from the player's path position and room when
    /// the player is in a DRLG room; else the rest's.
    fn free_spot(
        &mut self,
        player: UnitId,
        size: u32,
        mask: u32,
        radius: u32,
        limit: u32,
    ) -> Option<(i32, i32)> {
        if let Some((x, y, room)) = self.unit_position(player) {
            if self.drlg_room(room) {
                return helpers::free_spot(self, room, x, y, size, mask, limit).map(|p| (p.0, p.1));
            }
        }
        self.inner.free_spot(player, size, mask, radius, limit)
    }
    fn create_portal(&mut self, player: UnitId, x: i32, y: i32, class: u16, level: u32) -> bool {
        if self.inner.econ.hooks.objects.is_some() {
            return self.quest_pair(player, (x, y), class, level, false);
        }
        self.inner.create_portal(player, x, y, class, level)
    }
    /// Tyrael's call: the arrival hook places object 2 in Lut Gholein.
    fn create_tyrael_portal(
        &mut self,
        player: UnitId,
        x: i32,
        y: i32,
        class: u16,
        level: u32,
    ) -> bool {
        if self.inner.econ.hooks.objects.is_some() {
            return self.quest_pair(player, (x, y), class, level, true);
        }
        self.inner.create_portal(player, x, y, class, level)
    }
    fn object_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        self.inner.object_by_guid(guid)
    }
    fn mercenary_reward(&mut self, player: UnitId, npc: u16) {
        self.inner.mercenary_reward(player, npc)
    }
    /// A unit's path position and list room, for a unit of the game's lists
    /// (an object's static path; `None` for a unit left in a freed room).
    fn unit_position(&self, unit: UnitId) -> Option<(i32, i32, RoomId)> {
        let e = &*self.inner.econ;
        match e.game.lists.unit(unit) {
            Some(u) => {
                // An object inside its quest init is not in its room's
                // list yet (the init runs inside the allocation): its
                // init point is its position.
                let room = match (u.room(), self.init_point) {
                    (Some(r), _) => r,
                    (None, Some((o, x, y, r))) if o == unit => return Some((x, y, r)),
                    (None, _) => return None,
                };
                let (x, y) = e.hooks.path_position(unit);
                Some((x, y, room))
            }
            None => self.inner.unit_position(unit),
        }
    }
    /// The path position of a unit of the game's lists, with or without
    /// a room.
    fn unit_xy(&self, unit: UnitId) -> Option<(i32, i32)> {
        let e = &*self.inner.econ;
        match e.game.lists.unit(unit) {
            Some(u) if u.room().is_none() => match e.hooks.object_alloc_spot(unit) {
                Some((x, y, _)) => Some((x, y)),
                None => Some(e.hooks.path_position(unit)),
            },
            Some(_) => Some(e.hooks.path_position(unit)),
            None => self.inner.unit_position(unit).map(|(x, y, _)| (x, y)),
        }
    }
    /// The active room's sub-tile box, the last row and column excluded.
    fn room_contains(&self, room: RoomId, x: i32, y: i32) -> bool {
        let e = &*self.inner.econ;
        match e.hooks.drlg.subtiles(e.game, room) {
            Some(t) => t.contains(x, y),
            None => self.inner.room_contains(room, x, y),
        }
    }
    /// `DrlgWorld::find_room` from a room with a DRLG room.
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        let e = &*self.inner.econ;
        if e.hooks.drlg.drlg_room(e.game, room).is_some() {
            return e.hooks.drlg.find_room(e.game, room, x, y);
        }
        self.inner.room_at(room, x, y)
    }
    /// [`helpers::free_spot`] from a DRLG room; else the rest's.
    #[allow(clippy::too_many_arguments)]
    fn free_spot_at(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        size: u32,
        mask: u32,
        radius: u32,
        limit: u32,
    ) -> Option<(i32, i32, RoomId)> {
        if self.drlg_room(room) {
            return helpers::free_spot(self, room, x, y, size, mask, limit);
        }
        self.inner
            .free_spot_at(room, x, y, size, mask, radius, limit)
    }
    /// Population's placement search and creation in a DRLG room on the lent
    /// monster world; without one, the plain allocation at (x, y); else the
    /// rest's.
    fn spawn_monster(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        r: u32,
    ) -> Option<UnitId> {
        if self.drlg_room(room) {
            if let Some(u) = self.place_unit(room, x, y, class, mode, r as i32, 0) {
                return u;
            }
            return self.spawn_unit(room, x, y, class, mode);
        }
        self.inner.spawn_monster(room, x, y, class, mode, r)
    }
    fn or_unit_flags(&mut self, unit: UnitId, flags: u32) {
        self.inner.or_unit_flags(unit, flags)
    }
    fn monsters(&self) -> Vec<UnitId> {
        self.inner.monsters()
    }
    /// The players of the NPC's interaction list nodes (lent with
    /// [`HostQuests::chats`]), in list order; `None` for an empty list or a
    /// unit without one.
    fn npc_chat_clients(&self, npc: UnitId) -> Option<Vec<UnitId>> {
        let Some(chats) = self.chats.as_deref() else {
            return self.inner.npc_chat_clients(npc);
        };
        chats
            .get(&npc)
            .filter(|l| !l.nodes.is_empty())
            .map(|l| l.nodes.iter().map(|n| n.0).collect())
    }
    /// Removes a monster by its death mode 0: the monster runs its death and
    /// stays as a corpse, through the action wiring's monster mode change at
    /// its own position (the cleared sanctum minions take mode 0 at the
    /// clear and mode 12 nineteen frames later); a unit that is not a
    /// monster of the lists: the rest's.
    fn remove_monster(&mut self, monster: UnitId) {
        let is_monster = self
            .inner
            .econ
            .units
            .get(monster)
            .is_some_and(|r| r.ty == UnitType::Monster);
        if !is_monster {
            return self.inner.remove_monster(monster);
        }
        // The builder clears the used skill and leaves no target unit and
        // point (0, 0); flag 1 clears the skill again once the mode is set.
        self.view(|g, v| {
            AiModes::clear_current_skill(v, monster);
            AiModes::change_mode(v, g, monster, 0, ModeTarget::Point(0, 0));
            AiModes::clear_current_skill(v, monster);
        });
    }
    fn drop_preset_monster(&mut self, act: u8, class: u16) {
        self.inner.drop_preset_monster(act, class)
    }
    /// The first object of `class` in the rooms of `object`'s room's
    /// adjacency array (the room itself included), in array order, each
    /// room's unit list in order; else the rest's.
    fn find_object_near(&self, object: UnitId, class: u16) -> Option<UnitId> {
        use crate::path::collision::CollisionRooms;
        let e = &*self.inner.econ;
        let room = e.game.lists.unit(object).and_then(|u| u.room());
        let Some(room) = room.filter(|&r| self.drlg_room(r)) else {
            return self.inner.find_object_near(object, class);
        };
        let d = &e.hooks.drlg;
        (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .flat_map(|r| e.game.lists.room_units(r))
            .find(|&u| {
                e.units
                    .get(u)
                    .is_some_and(|r| r.ty == UnitType::Object && r.class == u32::from(class))
            })
    }
    fn create_object(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        if self.inner.econ.hooks.objects.is_some() && self.drlg_room(room) {
            return self.view(|g, v| v.create_object(g, room, u32::from(class), x, y, 0));
        }
        self.inner.create_object(room, x, y, class)
    }
    fn open_quest_message(&mut self, player: UnitId, object: UnitId, msg: u16) {
        self.inner.open_quest_message(player, object, msg)
    }
    /// [`Self::spawn_monster`] with the spawn flags: through population
    /// (spread and flags applied) when the monster world is lent, which every
    /// server quest call does (`with_economy` → `lend_world`); else, on a
    /// host without population, the plain allocation.
    #[allow(clippy::too_many_arguments)]
    fn spawn_monster_flags(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        spread: i32,
        flags: u32,
    ) -> Option<UnitId> {
        if self.drlg_room(room) {
            if let Some(u) = self.place_unit(room, x, y, class, mode, spread, flags as u16) {
                return u;
            }
            return self.spawn_unit(room, x, y, class, mode);
        }
        self.inner
            .spawn_monster_flags(room, x, y, class, mode, spread, flags)
    }
    /// Opens a portal pair ([`View::create_portal_pair`]): object 1 when the
    /// pair was made.
    #[allow(clippy::too_many_arguments)]
    fn open_portal(
        &mut self,
        owner: Option<UnitId>,
        room: RoomId,
        x: i32,
        y: i32,
        level: u32,
        class: u16,
        exact: bool,
    ) -> Option<UnitId> {
        if self.inner.econ.hooks.objects.is_none() || !self.drlg_room(room) {
            return self
                .inner
                .open_portal(owner, room, x, y, level, class, exact);
        }
        let (made, o1) = self.view_seeded(|g, v| {
            v.create_portal_pair(
                g,
                owner,
                Some(room),
                (x, y),
                level,
                u32::from(class),
                exact,
                false,
            )
        });
        o1.filter(|_| made != 0)
    }
    /// A missile at a point ([`helpers::missile_at_point`]) on the missile
    /// store when the action wiring holds one; else the rest's.
    fn create_missile(
        &mut self,
        owner: UnitId,
        skill: u16,
        level: u8,
        class: u16,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        if self.inner.econ.hooks.missiles.is_some() {
            return helpers::missile_at_point(self, owner, skill, level, class, x, y);
        }
        self.inner.create_missile(owner, skill, level, class, x, y)
    }
    /// Sets the target of a missile of the store.
    fn set_missile_target(&mut self, missile: UnitId, a: u32, b: u32) {
        if let Some(d) = self
            .inner
            .econ
            .hooks
            .missiles
            .as_mut()
            .and_then(|s| s.get_mut(missile))
        {
            d.target = (a as i32, b as i32);
            return;
        }
        self.inner.set_missile_target(missile, a, b)
    }
    /// Refreshes the unit's room
    /// ([`crate::wiring::action::DrlgWorld::refresh_room`]); a unit of the
    /// lists without a room: nothing.
    fn refresh_room(&mut self, unit: UnitId) {
        let e = &mut *self.inner.econ;
        if let Some(u) = e.game.lists.unit(unit) {
            let Some(room) = u.room() else { return };
            if e.hooks.drlg.refresh_room(e.game, room, false) {
                return;
            }
        }
        self.inner.refresh_room(unit)
    }
    fn client_save_flags(&self, player: UnitId) -> Option<u16> {
        self.inner.client_save_flags(player)
    }
    fn set_client_save_flags(&mut self, player: UnitId, flags: u16) {
        self.inner.set_client_save_flags(player, flags)
    }
    fn unhandled(&mut self, chain: u8, function: u32) {
        self.inner.unhandled(chain, function)
    }
    fn set_init_point(&mut self, point: Option<(UnitId, i32, i32, RoomId)>) {
        self.init_point = point;
    }
    fn client_in_act(&mut self, player: UnitId, act: u8) -> bool {
        self.inner.client_in_act(player, act)
    }
    fn start_tainted_sun(&mut self, act: u8) {
        self.inner.start_tainted_sun(act)
    }
    fn end_tainted_sun(&mut self) {
        self.inner.end_tainted_sun()
    }
    /// The chest gate over this host's own object view: it reads `Mode1`
    /// and the animation length of objects with object data, which the inner
    /// economy world cannot see.
    fn quest_chest_gate(&mut self, object: UnitId, _player: UnitId) -> bool {
        crate::world::quests::helpers::quest_chest_gate(self, object)
    }
    fn quest_drop(
        &mut self,
        unit: UnitId,
        code: [u8; 4],
        quality: u8,
        level: Option<i32>,
        droppable: bool,
    ) -> Option<UnitId> {
        // `level` is an out-parameter the drop overwrites before it reads it:
        // not an input here.
        match self.econ_quest_drop(unit, code, quality, i32::from(droppable)) {
            Some(item) => item,
            None => self.inner.quest_drop(unit, code, quality, level, droppable),
        }
    }
    /// The chest treasure of kind `kind` on an object with object data when
    /// the game holds the object drop state (`ActionHooks::object_drops`,
    /// [`super::object_chest_drop`] with the object tables' `levels`).
    fn object_treasure(&mut self, object: UnitId, operator: UnitId, kind: u8) {
        let known = self.known(object);
        let e = &mut *self.inner.econ;
        let tables = e.hooks.objects.as_ref().map(|s| s.tables.clone());
        let (true, Some(t), true) = (known, tables, e.hooks.object_drops.is_some()) else {
            return self.inner.object_treasure(object, operator, kind);
        };
        let Some(mut d) = e.hooks.object_drops.take() else {
            return;
        };
        // The economy holds the game's item store, game seed and unique
        // bits for this call: hand them back to their home in the hooks
        // for the drop, and take them again after it.
        std::mem::swap(&mut e.hooks.items, &mut *e.items);
        e.hooks.game_seed = e.fields.seed;
        e.hooks.uniques = std::mem::take(&mut e.fields.uniques);
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        super::object_chest_drop(
            &mut *e.hooks,
            &mut sim,
            &mut d,
            &t.levels,
            &mut super::StartSpot,
            object,
            Some(operator),
            kind,
        );
        std::mem::swap(&mut e.hooks.items, &mut *e.items);
        e.fields.seed = e.hooks.game_seed;
        e.fields.uniques = std::mem::take(&mut e.hooks.uniques);
        e.hooks.object_drops = Some(d);
    }
    /// Drops gold from the object: the code drop of `gld ` with quality 2
    /// ([`super::drop_helpers::code_drop`]).
    fn drop_gold(&mut self, object: UnitId) {
        let gold = u32::from_le_bytes(*b"gld ");
        if self
            .with_drop_state(|h, sim, d, levels, spots| {
                super::drop_helpers::code_drop(h, sim, d, levels, spots, object, gold, 2)
            })
            .is_none()
        {
            self.inner.drop_gold(object)
        }
    }
    /// Refreshes the room's DRLG room
    /// ([`crate::wiring::action::DrlgWorld::refresh_room`]: `on` is the
    /// nonzero argument, which clears the flag); else the rest's.
    fn set_room_portal(&mut self, room: RoomId, on: bool) {
        let e = &mut *self.inner.econ;
        if e.hooks.drlg.refresh_room(e.game, room, on) {
            return;
        }
        self.inner.set_room_portal(room, on)
    }
    /// Allocates an object (the Act II call form) through the object state;
    /// else the rest's.
    fn spawn_quest_object(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        if self.inner.econ.hooks.objects.is_some() && self.drlg_room(room) {
            return self.view(|g, v| v.create_object(g, room, u32::from(class), x, y, 0));
        }
        self.inner.spawn_quest_object(room, x, y, class)
    }
    // -- Act IV, Terror's End (q-a4-endgame).
    /// `FrameCnt1` of the object's `objects.txt` row (the raw column is
    /// × 256).
    fn object_frame_count1(&mut self, object: UnitId) -> i32 {
        self.object_anim_length(object) >> 8
    }
    /// Whether the unit is dead, from its unit record.
    fn unit_dead(&mut self, unit: UnitId) -> bool {
        self.inner.econ.units.is_dead(unit)
    }
    /// The unit's alignment (the action wiring's [`Pending::alignment`]).
    fn alignment(&mut self, unit: UnitId) -> u32 {
        u32::from(self.inner.econ.hooks.x.alignment(unit))
    }
    /// The monsters of `level`'s active rooms in room-list order, each
    /// room's in its unit-list order (newest arrival first). Measured: check
    /// `w2-14-sanctum-clear` frame 272, the minions' 0x69 / 0x9C order.
    fn level_monsters(&mut self, level: u32) -> Vec<UnitId> {
        let e = &*self.inner.econ;
        let lists = &e.game.lists;
        (0..5u8)
            .flat_map(|act| lists.active_rooms(act))
            .filter(|&r| e.hooks.drlg.level_id(e.game, r) == Some(level))
            .flat_map(|r| lists.room_units(r))
            .filter(|&u| lists.unit(u).is_some_and(|x| x.ty == UnitType::Monster))
            .collect()
    }
    /// The hcIdx → `superuniques.txt` row map is the identity in live data
    /// for entries 36–38.
    fn superunique_id(&mut self, n: u8) -> u16 {
        u16::from(n)
    }
    /// The preset spawn of a superunique: through the lent monster world,
    /// whose creation gives the quest links. A game without a lent world
    /// (the synthetic one): the class from the drop tables' `superuniques`
    /// rows (else the id), allocated plain, hcIdx 36–38 linked to chain 23.
    fn preset_superunique_spawn(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        superunique: u16,
    ) -> Option<UnitId> {
        let rows = self.inner.econ.hooks.tables.combat.monstats.len() as i32;
        if let Some(made) = self.preset_unit(room, x, y, rows + i32::from(superunique), 1) {
            return made;
        }
        let class = self
            .inner
            .econ
            .hooks
            .object_drops
            .as_ref()
            .and_then(|d| d.tables.superuniques.get(usize::from(superunique)))
            .map_or(superunique, |r| r.class as u16);
        let unit = self.spawn_unit(room, x, y, class, 1)?;
        if (36..=38).contains(&superunique) {
            // Applied at the end of this quest call (`quest_objects::link_items`).
            self.inner.econ.items.push_quest_link(unit, 23);
        }
        Some(unit)
    }
    /// Object allocation with the flag arguments through the object state.
    fn place_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        flags: [u8; 3],
    ) -> Option<UnitId> {
        // Flags & 1 adds the unit to the world; the second flag byte is the
        // object's mode, set before its init.
        let (add, mode) = (flags[0] & 1 != 0, flags[1]);
        if self.inner.econ.hooks.objects.is_some() && self.drlg_room(room) {
            return self
                .view(|g, v| v.create_object_flags(g, room, u32::from(class), x, y, mode, add));
        }
        self.inner.spawn_quest_object(room, x, y, class)
    }
    /// The client exists and is not busy.
    fn client_idle(&mut self, player: UnitId) -> bool {
        !self.inner.player_busy(player)
    }
    /// The host's act change queue (`ActionHooks::act_changes`).
    fn act_change(&mut self, player: UnitId, level: u32, arg: u32) {
        use crate::units::lifecycle::LifecycleHooks;
        self.inner.econ.hooks.request_act_change(player, level, arg);
    }
    /// Sets the level's waypoint index in the player's record of `difficulty`
    /// (the host's records, [`ActionHooks::waypoints`]). A level without a
    /// waypoint: nothing.
    fn activate_waypoint(&mut self, player: UnitId, level: u32, difficulty: u8) {
        use crate::world::waypoints::WaypointMap;
        let h = &mut self.inner.econ.hooks;
        let Some(idx) = WaypointMap::new(&h.tables.levels).index_of_level(level) else {
            return;
        };
        let rec = h.waypoints.entry(player).or_default();
        let _ = rec.get_mut(difficulty.min(2)).set(u32::from(idx));
    }
    fn player_busy(&mut self, player: UnitId) -> bool {
        self.inner.player_busy(player)
    }
    fn open_insert_dialog(&mut self, player: UnitId, object: UnitId) {
        self.inner.open_insert_dialog(player, object)
    }
    /// `missiles.txt` `Range` (u16 +0x96) of the action tables' row
    /// (the Arcane Sanctuary lair timer). D2RUST's own: a table
    /// without row 338 (the synthetic game) answers the live value 440
    /// (lair timer period 18).
    fn missile_range(&mut self, row: u32) -> Option<i32> {
        let t = &self.inner.econ.hooks.tables.missiles;
        match t.get(row as usize) {
            Some(m) => Some(i32::from(m.range)),
            None if t.is_empty() => Some(440),
            None => None,
        }
    }
    fn is_trading(&mut self, player: UnitId) -> bool {
        self.inner.is_trading(player)
    }
    fn remove_unit(&mut self, unit: UnitId) {
        self.inner.remove_unit(unit)
    }
    /// Holds the NPC's chat on its interaction list (lent with
    /// [`HostQuests::chats`]): for each node in list order, 0x62 (1, the
    /// NPC's GUID) to its player, whose interaction is cleared only while it
    /// still names this NPC (type 1, the GUID); then the list is emptied.
    /// False for an empty list.
    fn npc_hold_chat(&mut self, npc: UnitId) -> bool {
        let Some(chats) = self.chats.as_deref_mut() else {
            return self.inner.npc_hold_chat(npc);
        };
        let players: Vec<UnitId> = match chats.get_mut(&npc) {
            Some(l) if !l.nodes.is_empty() => l.nodes.drain(..).map(|n| n.0).collect(),
            _ => return false,
        };
        let g = self.guid(npc);
        for p in players {
            self.send(p, &crate::world::quests::helpers::msg_end_interaction(1, g));
            if self.interact_unit(p) == Some((1, g)) {
                self.set_interact_unit(p, None);
            }
        }
        true
    }
    fn spawn_location(&mut self, act: u8, level: u32, kind: u8) -> Option<(i32, i32, RoomId)> {
        self.inner.spawn_location(act, level, kind)
    }
    fn free_spot_near(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        size: u32,
        mask: u32,
        radius: u32,
    ) -> Option<(i32, i32, RoomId)> {
        self.inner.free_spot_near(room, x, y, size, mask, radius)
    }
    fn npc_intro_heard(&mut self, player: UnitId, class: u16) -> bool {
        self.inner.npc_intro_heard(player, class)
    }
    fn set_npc_intro(&mut self, player: UnitId, class: u16) {
        self.inner.set_npc_intro(player, class)
    }

    // -- The quest helpers' narrow seams on the action wiring.

    /// The object control's seed of the lent object state.
    fn object_seed(&mut self) -> Option<&mut Seed> {
        self.inner
            .econ
            .hooks
            .objects
            .as_mut()
            .map(|s| &mut s.control.seed)
    }
    /// The DRLG room's sub-tile box.
    fn room_box(&mut self, room: RoomId) -> Option<TileRect> {
        let e = &*self.inner.econ;
        match e.hooks.drlg.subtiles(e.game, room) {
            Some(t) => Some(t),
            None => self.inner.room_box(room),
        }
    }
    /// Whether a `size` × `size` box at (x, y) collides under `mask` on the
    /// DRLG collision ([`Rooms`]).
    fn box_collides(&mut self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> bool {
        if self.drlg_room(room) {
            let e = &*self.inner.econ;
            let n = size as u32;
            return Rooms(&e.hooks.drlg).box_query(room, x, y, n, n, mask) != 0;
        }
        self.inner.box_collides(room, x, y, size, mask)
    }
    /// Creates a quest missile on the action wiring's missile store; no
    /// store: the rest's answer.
    fn spawn_missile(&mut self, rec: QuestMissile) -> Option<UnitId> {
        let e = &mut *self.inner.econ;
        let Some(mut store) = e.hooks.missiles.take() else {
            return self.inner.spawn_missile(rec);
        };
        let p = MissileParams {
            flags: rec.flags,
            owner: Some(rec.owner),
            origin: rec.origin,
            class: i32::from(rec.class),
            x: rec.x,
            y: rec.y,
            target_x: rec.target_x,
            target_y: rec.target_y,
            skill: i32::from(rec.skill),
            level: i32::from(rec.level),
            ..MissileParams::default()
        };
        let t = e.hooks.tables.clone();
        // The allocation's unit seed steps the game seed, which the economy
        // holds for the quest call: lend it as [`Self::view_seeded`] does
        // (Baal's missile 625, gen-qkill-baal).
        e.hooks.game_seed = e.fields.seed;
        let made = {
            let mut v = View::of(&mut *e.units, &mut *e.stats, e.data, &mut *e.hooks);
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut v,
            };
            missiles::create_missile(&mut *e.game, &mut cx, &p)
        };
        e.fields.seed = e.hooks.game_seed;
        e.hooks.missiles = Some(store);
        made
    }
    /// Sets the first target field of a missile of the store.
    fn set_missile_guid(&mut self, missile: UnitId, v: u32) {
        if let Some(d) = self
            .inner
            .econ
            .hooks
            .missiles
            .as_mut()
            .and_then(|s| s.get_mut(missile))
        {
            d.target.0 = v as i32;
            return;
        }
        self.inner.set_missile_guid(missile, v)
    }
    /// Looks up a unit by kind and GUID in the game's unit lists.
    fn unit_by_guid(&mut self, kind: u8, guid: u32) -> Option<UnitId> {
        let ty = UnitType::ALL.into_iter().find(|&t| t as u8 == kind)?;
        self.inner.econ.game.lists.find_unit(ty, guid)
    }
    /// `objects.txt` `Mode1` of an object with object data.
    fn object_mode1(&mut self, object: UnitId) -> Option<bool> {
        if let Some(st) = self.inner.econ.hooks.objects.as_ref() {
            if let Some(d) = st.control.data.get(&object) {
                if let Ok(o) = st.tables.object(d.class) {
                    return Some(o.mode1 != 0);
                }
            }
        }
        self.inner.object_mode1(object)
    }
    /// Clears `flags` in a unit record's flags.
    fn clear_unit_flags(&mut self, unit: UnitId, flags: u32) {
        match self.inner.econ.units.get_mut(unit) {
            Some(r) => r.flags &= !flags,
            None => self.inner.clear_unit_flags(unit, flags),
        }
    }
    /// The type-5 units of the object's room unit list, in list order.
    fn room_warp_tiles(&mut self, object: UnitId) -> Vec<UnitId> {
        let lists = &self.inner.econ.game.lists;
        let Some(room) = lists.unit(object).and_then(|u| u.room()) else {
            return Vec::new();
        };
        lists
            .room_units(room)
            .into_iter()
            .filter(|&u| lists.unit(u).is_some_and(|e| e.ty == UnitType::Tile))
            .collect()
    }
    /// Warps the player through the tile
    /// ([`crate::wiring::path::place::warp_player`]).
    fn warp_through(&mut self, player: UnitId, tile: UnitId) {
        let e = &*self.inner.econ;
        let room = e.game.lists.unit(tile).and_then(|u| u.room());
        let class = e.units.get(tile).map(|r| r.class);
        let (Some(room), Some(class)) = (room, class) else {
            return;
        };
        self.view(|g, v| {
            crate::wiring::path::place::warp_player(
                crate::wiring::path::PathCtx::of(v, g),
                player,
                room,
                class,
            )
        });
    }

    // -- The quest helpers' host seams on the action wiring and the lent host
    // parts.

    /// The item level of a player ([`quest_reward::item_level`]) on the
    /// unit's record and stats.
    fn quest_item_level(&mut self, player: UnitId) -> i32 {
        match quest_reward::item_level(&*self.inner.econ, player) {
            Some(v) => v,
            None => self.inner.quest_item_level(player),
        }
    }
    /// The path target on the path provider: the path target unit's
    /// position, else the target point; `None` when a coordinate is 0. No
    /// path provider: the rest's.
    fn path_target_xy(&mut self, unit: UnitId) -> Option<(i32, i32)> {
        match self.view(|g, v| v.path_target_position(g, unit)) {
            Some(p) => p,
            None => self.inner.path_target_xy(unit),
        }
    }
    /// A trade button press: no player → nothing; no active interaction →
    /// S→C 0x77 0x0C; buttons 0x12–0x14, 0x17, 0x18 (stash, cube) → their
    /// owners (the rest); an interaction other than a player → 0x77 0x0D; a
    /// trade partner gone → 0x77 0x0C, unless the player is trade-locked
    /// (`Self::trade_locked`), which sends nothing; with the partner,
    /// buttons 5 and 6 and those outside 2–8 do nothing, and the other trade
    /// buttons are the rest's (the player-trade flow is not written here).
    fn trade_button(&mut self, player: UnitId, button: u8) {
        if self.inner.econ.units.get(player).is_none() {
            return;
        }
        let Some((ty, guid)) = self.interact_unit(player) else {
            return self.send(player, &trade_action(0x0C));
        };
        match button {
            0x12..=0x14 | 0x17 | 0x18 => self.inner.trade_button(player, button),
            _ if ty != 0 => self.send(player, &trade_action(0x0D)),
            _ if self.player_by_guid(guid).is_none() => {
                if !self.trade_locked(player) {
                    self.send(player, &trade_action(0x0C));
                }
            }
            2..=4 | 7 | 8 => self.inner.trade_button(player, button),
            _ => {}
        }
    }
    /// Unlinks the player's node from the NPC's interaction list (lent with
    /// [`HostQuests::chats`]); nothing for a unit without a list (not an
    /// `interact` monster).
    fn free_chat_node(&mut self, npc: UnitId, player: UnitId) {
        let Some(chats) = self.chats.as_deref_mut() else {
            return self.inner.free_chat_node(npc, player);
        };
        if let Some(l) = chats.get_mut(&npc) {
            if let Some(i) = l.nodes.iter().position(|n| n.0 == player) {
                l.nodes.remove(i);
            }
        }
    }
    /// Ends the NPC's chats on its interaction list: every node freed, the
    /// list emptied.
    fn clear_npc_chats(&mut self, npc: UnitId) {
        let Some(chats) = self.chats.as_deref_mut() else {
            return self.inner.clear_npc_chats(npc);
        };
        if let Some(l) = chats.get_mut(&npc) {
            l.nodes.clear();
        }
    }
    /// Inserts the C→S 0x44 cancel for the player and the object
    /// ([`act2::q6::insert_cancel`]).
    fn obelisk_close(&mut self, player: UnitId, object: UnitId) {
        let g = self.guid(object);
        act2::q6::insert_cancel(self, player, object, g);
    }
    /// The inventory pass on the lent inventory model; none lent: the
    /// rest's.
    fn close_cube(&mut self, player: UnitId) {
        match self.inventory.as_deref_mut() {
            Some(inv) => inv.inventory_pass(&mut *self.inner.econ, player),
            None => self.inner.close_cube(player),
        }
    }
    /// The player's town portal GUID on the action wiring's portal links
    /// (`PortalLinks::player_portal`, written by the Town Portal cast; when
    /// nothing was written, `Pending::object_portal_guid`); a unit other
    /// than a player has none.
    fn town_portal_guid(&mut self, player: UnitId) -> Option<u32> {
        let h = &self.inner.econ.hooks;
        match self.inner.econ.units.get(player).map(|r| r.ty) {
            Some(UnitType::Player) => Some(
                h.portals
                    .player_portal(player)
                    .unwrap_or_else(|| h.x.object_portal_guid(player)),
            ),
            Some(_) => None,
            None => self.inner.town_portal_guid(player),
        }
    }
    /// The partner of a portal on the action wiring's portal links
    /// ([`View::portal_partner`]); a portal without a link:
    /// `Pending::object_portal_partner`.
    fn portal_partner(&mut self, portal: UnitId) -> Option<UnitId> {
        self.view(|g, v| {
            if v.h.portals.link(portal).is_some() {
                return v.portal_partner(g, portal);
            }
            v.h.x.object_portal_partner(g, portal)
        })
    }
    /// Frees a town portal object: the portal removal
    /// ([`View::remove_portal_object`]), after the rest's seam
    /// (`Pending::object_remove_portal`).
    fn free_portal_object(&mut self, portal: UnitId) {
        self.view(|g, v| {
            v.h.x.object_remove_portal(g, portal);
            v.remove_portal_object(g, portal);
        });
    }
    // -- Act IV / V seams on the action wiring.

    /// The distance between two units ([`missiles::bodies::unit_distance`])
    /// on their path positions and sizes; `None` when either unit is not in
    /// the game's lists.
    fn distance_between(&mut self, a: UnitId, b: UnitId) -> Option<i32> {
        let lists = &self.inner.econ.game.lists;
        if lists.unit(a).is_none() || lists.unit(b).is_none() {
            return self.inner.distance_between(a, b);
        }
        Some(self.view(|_, v| missiles::bodies::unit_distance(v, a, b)))
    }
    /// Whether the victim is a champion or unique (monster flag 12) or a boss,
    /// as the combat wiring reads them.
    fn special_monster(&mut self, victim: UnitId) -> bool {
        let h = &*self.inner.econ.hooks;
        h.monster_flag(victim, 12) || h.x.is_boss(victim)
    }
    fn has_act3(&mut self) -> bool {
        self.inner.econ.hooks.drlg.dungeon.acts[2].is_some()
    }
    /// The Act III active room whose sub-tile rectangle holds (x, y).
    fn room_covering(&mut self, x: i32, y: i32) -> Option<RoomId> {
        let e = &*self.inner.econ;
        e.game.lists.active_rooms(2).into_iter().find(|&r| {
            e.hooks
                .drlg
                .subtiles(e.game, r)
                .is_some_and(|s| x >= s.x && x < s.x + s.w && y >= s.y && y < s.y + s.h)
        })
    }
    /// A player in `room` or a room of its adjacency array
    /// ([`Self::adjacent_units`]).
    fn player_in_rooms(&mut self, room: RoomId) -> bool {
        self.adjacent_units(room).into_iter().any(|u| {
            self.inner
                .econ
                .game
                .lists
                .unit(u)
                .is_some_and(|e| e.ty == UnitType::Player)
        })
    }
    /// Spawns a random boss of `class` in `room` through the lent monster
    /// world.
    fn spawn_monster_in_room(&mut self, room: RoomId, class: u16) -> Option<UnitId> {
        self.view(|g, v| {
            let mut sim = crate::units::hooks::Sim {
                game: g,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            };
            v.h.with_monster_world(|w, h| w.spawn_random_boss(&mut sim, h, room, i32::from(class)))
        })
        .flatten()
        .flatten()
    }
    /// The units of every room of the DRLG room's adjacency array (the room
    /// itself included), in array order, each room's unit list in order.
    fn adjacent_units(&mut self, room: RoomId) -> Vec<UnitId> {
        use crate::path::collision::CollisionRooms;
        if !self.drlg_room(room) {
            return self.inner.adjacent_units(room);
        }
        let e = &*self.inner.econ;
        let d = &e.hooks.drlg;
        (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .flat_map(|r| e.game.lists.room_units(r))
            .collect()
    }
    /// The mode of a unit record.
    fn unit_mode(&mut self, unit: UnitId) -> i32 {
        match self.inner.econ.units.get(unit) {
            Some(r) => r.mode as i32,
            None => self.inner.unit_mode(unit),
        }
    }
    /// "Kill in place": the interaction ended, mode 12
    /// ([`View::monster_set_mode`]), then the unit leaves its room and is kept
    /// ([`View::leave_room`]).
    fn kill_in_place(&mut self, unit: UnitId) {
        self.end_unit_interaction(unit);
        if !self.leave_dead(unit) {
            self.inner.kill_in_place(unit)
        }
    }
    /// Town cleanup: Anya's interaction ended, mode 12, she leaves her room
    /// and is kept (the steps of [`Self::kill_in_place`]).
    fn npc_leave_town(&mut self, unit: UnitId) {
        self.end_unit_interaction(unit);
        if !self.leave_dead(unit) {
            self.inner.npc_leave_town(unit)
        }
    }
    /// Nihlathak killed in town: interaction ended, path stopped, his
    /// AI-think events deleted, stat 6 := 0, mode 12
    /// ([`View::monster_set_mode`]), room refresh, unit flags |= 1. He stays
    /// as a corpse (REC-791).
    fn kill_in_town(&mut self, unit: UnitId) {
        let is_monster = self
            .inner
            .econ
            .units
            .get(unit)
            .is_some_and(|r| r.ty == UnitType::Monster);
        if !is_monster || self.inner.econ.game.lists.unit(unit).is_none() {
            return self.inner.kill_in_town(unit);
        }
        self.end_unit_interaction(unit);
        self.inner.econ.hooks.stop_path_now(unit);
        self.inner
            .econ
            .game
            .timers
            .cancel_unit_events(unit, event::AI_THINK, None);
        self.view(|g, v| {
            v.set_base(unit, STAT_HITPOINTS, 0);
            v.monster_set_mode(g, unit, MODE_DEAD);
        });
        self.refresh_room(unit);
        if let Some(r) = self.inner.econ.units.get_mut(unit) {
            r.flags |= 1;
        }
    }
    /// Thaw step 1: the frozen object leaves its room and is kept; its
    /// collision was freed at step 0.
    fn object_leave_room(&mut self, object: UnitId) {
        if !self.view(|g, v| v.leave_room(g, object, false)) {
            self.inner.object_leave_room(object)
        }
    }
    /// `items.txt` `dropsound` of an item of the game's store.
    fn item_drop_sound(&mut self, item: UnitId) -> i32 {
        let e = &*self.inner.econ;
        match e.items.get(item).and_then(|i| e.tables.item(i.record)) {
            Some(r) => i32::from(r.dropsound),
            None => self.inner.item_drop_sound(item),
        }
    }
    /// Adds a new stat list (flags 0, owner = the player's type and GUID) to
    /// the player with the four stats 39, 41, 43, 45 := v, and sends them: a
    /// new list each call, so they stack.
    // The send is the written value through `Pending::stat_sent`, one
    // message per stat in id order: the game calls it four times, once per
    // stat (REC-792).
    fn add_resist_list(&mut self, player: UnitId, v: i32) {
        let Some((ty, guid)) = self.inner.econ.units.get(player).map(|r| (r.ty, r.guid)) else {
            return self.inner.add_resist_list(player, v);
        };
        self.view(|_, view| {
            let l = view.stats.alloc(0, 0, ty.index() as u32, guid);
            view.stats.attach(&mut *view.h, player, l, true);
            for s in RESIST_STATS {
                view.set_list_stat(l, s, v);
            }
            for s in RESIST_STATS {
                view.h.x.stat_sent(player, s, v as u32);
            }
        });
    }
    /// Whether the level's waypoint is active in the player's record of the
    /// game's difficulty ([`ActionHooks::waypoints`], the inverse of
    /// [`Self::activate_waypoint`]). A level without a waypoint or a player
    /// without records: not active.
    fn waypoint_active(&mut self, player: UnitId, level: u32) -> bool {
        use crate::world::waypoints::WaypointMap;
        let d = usize::from(self.difficulty().min(2));
        let h = &self.inner.econ.hooks;
        let Some(idx) = WaypointMap::new(&h.tables.levels).index_of_level(level) else {
            return false;
        };
        h.waypoints
            .get(&player)
            .is_some_and(|r| r.0[d].test(u32::from(idx)).unwrap_or(false))
    }
    /// Clears state 54; an Ancient in a room then takes mode 12, leaves it
    /// and is kept (the steps of [`Self::kill_in_place`]).
    fn remove_ancient(&mut self, monster: UnitId) {
        let Some(in_room) = self
            .inner
            .econ
            .game
            .lists
            .unit(monster)
            .map(|u| u.room().is_some())
        else {
            return self.inner.remove_ancient(monster);
        };
        self.view(|g, v| crate::wiring::action::units::clear_uninterruptable(v, g, monster));
        if in_room {
            self.leave_dead(monster);
        }
    }
    /// `experience.txt` row 0 of the player's class: the maximum level.
    fn max_level(&mut self, player: UnitId) -> i32 {
        match self.vitals_of(player) {
            Some((t, class)) => t.max_level(class) as i32,
            None => self.inner.max_level(player),
        }
    }
    /// `experience.txt` row `level + 1` of the player's class.
    fn experience_threshold(&mut self, player: UnitId, level: i32) -> u32 {
        match self.vitals_of(player) {
            Some((t, class)) if level >= 0 && (level as usize) + 1 < t.experience.len() => {
                t.threshold(class, level as u32)
            }
            _ => self.inner.experience_threshold(player, level),
        }
    }
    /// A level-up ([`crate::combat::vitals::level_up`]) on the action
    /// wiring's view.
    fn level_up(&mut self, player: UnitId) {
        let Some((t, _)) = self.vitals_of(player) else {
            return self.inner.level_up(player);
        };
        self.view(|_, v| {
            crate::combat::vitals::level_up(v, &t, player);
        });
    }
    /// One gold pile of `amount` at the unit on the lent inventory model
    /// ([`QuestInventory::gold_pile`]).
    // The single-pile call is the multi-pile gold drop with a maximum of 32
    // piles: one pile for any amount below 2e9, as this max-1 call (REC-793).
    fn drop_gold_amount(&mut self, at: UnitId, amount: u32) {
        let done = match self.inventory.as_deref_mut() {
            Some(inv) => inv.gold_pile(&mut *self.inner.econ, at, amount as i32),
            None => false,
        };
        if !done {
            self.inner.drop_gold_amount(at, amount)
        }
    }
    /// `monstats.txt` row count of the action tables.
    fn monstats_rows(&mut self) -> u32 {
        match self.inner.econ.hooks.tables.combat.monstats.len() {
            0 => self.inner.monstats_rows(),
            n => n as u32,
        }
    }
    /// The `zoo` column of the class's `monstats.txt` row; a class past the
    /// table: not eligible.
    fn zoo_eligible(&mut self, class: u32) -> bool {
        let m = &self.inner.econ.hooks.tables.combat.monstats;
        if m.is_empty() {
            return self.inner.zoo_eligible(class);
        }
        m.get(class as usize).is_some_and(|r| r.zoo)
    }
    /// Warps the player to a level through the action wiring's level warp
    /// ([`crate::wiring::action::waypoints::level_warp`], the waypoints'
    /// warp), `arg` the tile code.
    fn warp_to_level(&mut self, player: UnitId, level: u32, arg: u32) {
        if self.inner.econ.game.lists.unit(player).is_none() {
            return self.inner.warp_to_level(player, level, arg);
        }
        self.view(|g, v| {
            crate::wiring::action::waypoints::level_warp(v, g, player, level, arg as u8)
        });
    }

    /// The monster mode request at a point (no path step set) on the action
    /// wiring's monster mode change; a unit that is not a monster: the rest's.
    fn monster_mode_at(&mut self, monster: UnitId, mode: u8, x: i32, y: i32) {
        let is_monster = self
            .inner
            .econ
            .units
            .get(monster)
            .is_some_and(|r| r.ty == UnitType::Monster);
        if !is_monster {
            return self.inner.monster_mode_at(monster, mode, x, y);
        }
        self.view(|g, v| {
            AiModes::change_mode(v, g, monster, mode, ModeTarget::Point(x, y));
        });
    }
}
