//! The hireling rules ([`crate::world::hirelings`]) on the desk's
//! providers: [`HirelingWorld`] on the unit records (GUID, type, class,
//! mode, flags, room), the unit lists (GUID lookup, the players), the
//! stat lists (stats, maxima, states) and the transport of the quests'
//! rest; the calls with no provider yet stay a seam ([`HirelingRest`]).
//! The game's hireling state and tables live in
//! [`super::InteractionState`] (`hirelings`, `hireling_tables`).
//!
//! The `NpcWorld` mercenary calls of `world::npc` (hire, resurrect, quest
//! mercenary) land here: `init_mercenary` → [`life::init`],
//! `revive_mercenary` → [`life::revive`], `pet(7, any)` → the hireling pet
//! list.

use super::{Desk, InteractionError, PlayerQuestsRef};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};
use crate::wiring::economy::QuestRest;
use crate::world::hirelings::{life, HirelingState, HirelingTables, HirelingWorld, Slot, NEW_HIRE};
use crate::world::npc::MercInit;

use super::NpcRest;

/// State 7 (`playerbody`): players in it are skipped by the broadcast.
const STATE_BROADCAST_SKIP: u32 = 7;

/// Umod 19 `hireable`.
const UMOD_HIREABLE: u8 = 19;

/// The hireling calls with no provider yet, each left to the host.
pub trait HirelingRest {
    /// A mode change (monster modes).
    fn set_mode(&mut self, unit: UnitId, mode: u8);
    /// `skills` count and `reqlevel` (read signed) (skill tables).
    fn skill_count(&self) -> u32;
    fn skill_reqlevel(&self, skill: u32) -> Option<i16>;
    /// Sets the monster's skill level (skills).
    fn set_skill_level(&mut self, unit: UnitId, skill: u32, level: i32);
    /// The largest `petmax` of the player's skills of `pettype`
    /// `pet_type` (the skill list and the `petmax` calc). The default
    /// answers `None` (no such skill): `skills.txt` has no skill of
    /// `pettype` `hireable`, so a host without the skill provider gets
    /// `basemax`.
    fn skill_pet_max(&self, player: UnitId, pet_type: u8) -> Option<i32> {
        let _ = (player, pet_type);
        None
    }
    /// Sets and reads the AI control's minion owner (monster AI).
    fn set_owner(&mut self, merc: UnitId, guid: u32, unit_type: u8);
    fn owner(&self, merc: UnitId) -> Option<(u32, u8)>;
    /// Queues the room's removal notice and frees the unit record.
    fn free_unit(&mut self, unit: UnitId);
    /// Queues the room's removal notice only.
    fn queue_room_removal(&mut self, unit: UnitId);
    /// The death mode request on a unit in a room (a monster mode request
    /// at the unit's death target).
    fn death_event(&mut self, unit: UnitId);
    /// Dismisses the pet: a kill or death mode request.
    fn dismiss(&mut self, unit: UnitId);
    /// Warps `pet` to `player` (rooms, path).
    fn warp_to(&mut self, pet: UnitId, player: UnitId);
    /// Unit event 12; the desk queues the event 0x5B first.
    fn level_events(&mut self, player: UnitId, merc: UnitId);
    /// Re-applies the item stats of `merc` (item stats).
    fn reapply_item_stats(&mut self, merc: UnitId);
    /// The unit's position as the follow rule reads it (the path's x / y,
    /// an object's static path; no path → (0, 0)): the path records are
    /// the action wiring's, not the desk's. The default answers (0, 0)
    /// ("no path"); only the `range` follow branch reads it, which
    /// `pettype` row 7 (`range` 0) never takes.
    fn hireling_position(&self, unit: UnitId) -> (i32, i32) {
        let _ = unit;
        (0, 0)
    }
    /// Creates the unit's inventory when it has none (the inventory model
    /// is the inventory wiring's, not the desk's).
    ///
    /// The default does nothing; a host with the inventory parts creates
    /// the inventory (`d2-server` `WiredWorld::restore_hireling` does,
    /// on its inventory model).
    fn ensure_inventory(&mut self, unit: UnitId) {
        let _ = unit;
    }
}

/// The hireling world of one call: the desk without its hireling state
/// (taken out for the call so the rules can borrow both).
pub struct HireView<'v, 'd, 'a, H, R> {
    pub desk: &'v mut Desk<'d, 'a, H, R>,
}

impl<H: LifecycleHooks, R: NpcRest + QuestRest + PlayerQuestsRef> HirelingWorld
    for HireView<'_, '_, '_, H, R>
{
    fn difficulty(&self) -> u8 {
        self.desk.econ.fields.difficulty
    }
    fn expansion(&self) -> bool {
        self.desk.econ.fields.expansion
    }
    /// The player list in order, without players in state 7.
    fn players(&self) -> Vec<UnitId> {
        let e = &self.desk.econ;
        e.game
            .lists
            .units_of_type(UnitType::Player)
            .into_iter()
            .filter(|&p| !e.stats.has_state(p, STATE_BROADCAST_SKIP))
            .collect()
    }
    fn send(&mut self, player: UnitId, bytes: &[u8]) {
        if let Some((joiner, held)) = &mut self.desk.state.join_capture {
            // The add broadcasts only (0x81, 0x7A action 1).
            let add = matches!(bytes, [0x81, ..] | [0x7A, 1, ..]);
            if *joiner == player && add {
                held.push(bytes.to_vec());
                return;
            }
        }
        QuestRest::send(&mut *self.desk.rest, player, bytes);
    }
    /// Onto [`super::InteractionState::unit_stats`].
    fn queue_stat(&mut self, unit: UnitId, stat: u16, value: u32) {
        self.desk
            .state
            .unit_stats
            .entry(unit)
            .or_default()
            .push((stat, value));
        // The unit's message list, flushed by the client pass.
        self.desk.econ.hooks.queue_unit_stat(unit, stat, value);
        let _ = self.desk.econ.game.lists.queue_update(unit);
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.desk.econ.units.get(unit).map_or(u32::MAX, |r| r.guid)
    }
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.desk.econ.game.lists.find_unit(UnitType::Monster, guid)
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.desk.econ.game.lists.find_unit(UnitType::Player, guid)
    }
    fn unit_type(&self, unit: UnitId) -> u8 {
        self.desk
            .econ
            .game
            .lists
            .unit(unit)
            .map_or(0xFF, |e| e.ty as u8)
    }
    fn class(&self, unit: UnitId) -> u32 {
        self.desk.econ.units.get(unit).map_or(0, |r| r.class)
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.desk.econ.units.get(unit).map_or(0, |r| r.mode)
    }
    fn set_mode(&mut self, unit: UnitId, mode: u8) {
        HirelingRest::set_mode(&mut *self.desk.rest, unit, mode);
    }
    fn flags(&self, unit: UnitId) -> u32 {
        self.desk.econ.units.get(unit).map_or(0, |r| r.flags)
    }
    fn set_flags(&mut self, unit: UnitId, flags: u32) {
        if let Some(r) = self.desk.econ.units.get_mut(unit) {
            r.flags = flags;
        }
    }
    fn flags2(&self, unit: UnitId) -> u32 {
        self.desk.econ.units.get(unit).map_or(0, |r| r.flags2)
    }
    fn set_flags2(&mut self, unit: UnitId, flags: u32) {
        if let Some(r) = self.desk.econ.units.get_mut(unit) {
            r.flags2 = flags;
        }
    }
    fn in_room(&self, unit: UnitId) -> bool {
        self.desk
            .econ
            .game
            .lists
            .unit(unit)
            .is_some_and(|e| e.room().is_some())
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.desk.rest.hireling_position(unit)
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.desk.econ.stats.unit_total(unit, stat, 0)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.desk.econ.stats.unit_base(unit, stat, 0)
    }
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        let e = &mut *self.desk.econ;
        e.stats.unit_set(&mut *e.hooks, unit, stat, value, 0);
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        self.desk.econ.stats.max_life(unit)
    }
    /// Sets the alignment stat (172) in the state 105 list, with the
    /// allied mark of a good alignment (`UnitEntry::allied`). Only this
    /// pair is written through here (the init's one call); another state
    /// or stat has no caller.
    fn set_state_stat(&mut self, unit: UnitId, state: u16, stat: u16, value: i32) {
        debug_assert!(state == crate::combat::range::STATE_ALIGNMENT);
        debug_assert!(stat == crate::world::hirelings::stat::ALIGNMENT);
        let e = &mut *self.desk.econ;
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        if let Ok(a) = u8::try_from(value) {
            e.hooks.set_alignment(&mut sim, unit, a);
        }
    }
    /// Frees the state's stat list.
    fn remove_state(&mut self, unit: UnitId, state: u16) {
        let e = &mut *self.desk.econ;
        e.stats
            .free_state_list(&mut *e.hooks, unit, u32::from(state));
    }
    /// The hooks' `skills` table when they hold one, else the rest's.
    fn skill_count(&self) -> u32 {
        match self.desk.econ.hooks.skill_tables() {
            Some(t) => u32::try_from(t.skills.len()).unwrap_or(u32::MAX),
            None => self.desk.rest.skill_count(),
        }
    }
    /// `reqlevel` (read signed) from the hooks' `skills` table when they
    /// hold one, else the rest's.
    fn skill_reqlevel(&self, skill: u32) -> Option<i16> {
        match self.desk.econ.hooks.skill_tables() {
            Some(t) => {
                let r = t.skill(i32::try_from(skill).ok()?)?;
                Some(r.reqlevel as i16)
            }
            None => self.desk.rest.skill_reqlevel(skill),
        }
    }
    /// Sets the monster's skill entry on the hooks
    /// ([`LifecycleHooks::set_monster_skill_level`]); a host without
    /// them leaves it to the rest.
    fn set_skill_level(&mut self, unit: UnitId, skill: u32, level: i32) {
        let e = &mut *self.desk.econ;
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        let done = i32::try_from(skill)
            .is_ok_and(|k| e.hooks.set_monster_skill_level(&mut sim, unit, k, level));
        if !done {
            self.desk.rest.set_skill_level(unit, skill, level);
        }
    }
    fn skill_pet_max(&self, player: UnitId, pet_type: u8) -> Option<i32> {
        self.desk.rest.skill_pet_max(player, pet_type)
    }
    /// Sets the AI control's minion owner (the Hireable AI reads it), and
    /// the rest's copy.
    fn set_owner(&mut self, merc: UnitId, guid: u32, unit_type: u8) {
        self.desk.econ.hooks.set_ai_owner(merc, unit_type, guid);
        self.desk.rest.set_owner(merc, guid, unit_type);
    }
    fn owner(&self, merc: UnitId) -> Option<(u32, u8)> {
        self.desk.rest.owner(merc)
    }
    /// The merc joins its player's target list: only a player or monster
    /// in no list (node index 11) joins, and only into a player list; the
    /// node goes right after the list's head and the merc's node index :=
    /// the list's code. The head is the player (the host's list head).
    fn join_team(&mut self, merc: UnitId, player: UnitId) {
        let e = &mut *self.desk.econ;
        let Some(group) = e.units.get(player).map(|r| r.node_index) else {
            return;
        };
        let Some(m) = e.units.get_mut(merc) else {
            return;
        };
        if crate::game::node::player_list(group).is_none()
            || m.node_index != crate::game::node::NONE
            || !matches!(m.ty, UnitType::Player | UnitType::Monster)
        {
            return;
        }
        m.node_index = group;
        e.game.target_nodes.push_front(group as i32, merc);
    }
    /// Assigns umod 19 `hireable` to the merc, then clears the monster
    /// data bytes of the hireling components.
    fn hireling_ai(&mut self, merc: UnitId) {
        let e = &mut *self.desk.econ;
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        e.hooks.assign_umod(&mut sim, merc, UMOD_HIREABLE);
        e.hooks.clear_hireling_components(merc);
    }
    /// The room's removal notice, then the unit record freed (the rest's
    /// part first).
    fn free_unit(&mut self, unit: UnitId) {
        self.desk.rest.free_unit(unit);
        let e = &mut *self.desk.econ;
        if let Some((ty, guid)) = e.units.get(unit).map(|r| (r.ty as u8, r.guid)) {
            e.hooks.queue_removal_notice(ty, guid);
        }
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        let _ = crate::units::lifecycle::remove(&mut sim, &mut *e.hooks, unit);
    }
    fn queue_room_removal(&mut self, unit: UnitId) {
        self.desk.rest.queue_room_removal(unit);
    }
    fn death_event(&mut self, unit: UnitId) {
        self.desk.rest.death_event(unit);
    }
    /// Cancels the unit's timer events of type `ty` on the game's timer
    /// queue; a = 0: any argument.
    fn cancel_timers(&mut self, unit: UnitId, ty: u8, a: u32) {
        let arg = (a != 0).then_some(a);
        self.desk.econ.game.timers.cancel_unit_events(unit, ty, arg);
    }
    fn ensure_inventory(&mut self, unit: UnitId) {
        self.desk.rest.ensure_inventory(unit);
    }
    fn dismiss(&mut self, unit: UnitId) {
        self.desk.rest.dismiss(unit);
    }
    /// Warps the pet on the hooks' path code
    /// ([`LifecycleHooks::warp_pet`]), then the rest's part.
    fn warp_to(&mut self, pet: UnitId, player: UnitId) {
        let e = &mut *self.desk.econ;
        let mut sim = crate::units::hooks::Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        e.hooks.warp_pet(&mut sim, pet, player);
        self.desk.rest.warp_to(pet, player);
    }
    /// The level-up event 0x5B, queued on the merc for its player and
    /// sent as S→C 0x2C by the client pass of the tick (`units::sound`).
    /// Recorded: `w2-09-merc-levelup-kill` frame 314,
    /// `2c 01 0d000000 5b00`; then the rest's part (unit event 12).
    fn level_events(&mut self, player: UnitId, merc: UnitId) {
        let _ =
            crate::units::sound::queue_sound(&mut *self.desk.econ.game, merc, 0x5B, Some(player));
        self.desk.rest.level_events(player, merc);
    }
    fn reapply_item_stats(&mut self, merc: UnitId) {
        self.desk.rest.reapply_item_stats(merc);
    }
}

impl<'a, H: LifecycleHooks, R: NpcRest + QuestRest + PlayerQuestsRef> Desk<'_, 'a, H, R> {
    /// Runs `f` on the hireling world with the game's hireling state and
    /// tables; no tables → [`InteractionError::NoHirelingTables`], nothing
    /// runs.
    pub fn with_hirelings<T>(
        &mut self,
        f: impl FnOnce(&mut HireView<'_, '_, 'a, H, R>, &HirelingTables, &mut HirelingState) -> T,
    ) -> Option<T> {
        let Some(t) = self.state.hireling_tables.take() else {
            self.state.errors.push(InteractionError::NoHirelingTables);
            return None;
        };
        let mut st = std::mem::take(&mut self.state.hirelings);
        let out = f(&mut HireView { desk: self }, &t, &mut st);
        self.state.hirelings = st;
        self.state.hireling_tables = Some(t);
        Some(out)
    }

    /// Runs the hireling init for a new hire (`saved_id` 0xFFFF). The
    /// offer of `init` is not passed: the init re-runs the offer itself.
    pub(super) fn hireling_init(&mut self, player: UnitId, merc: UnitId, init: &MercInit) {
        let slot = Slot {
            name: init.name,
            seed: init.seed,
        };
        let r = self
            .with_hirelings(|w, t, st| life::init(w, t, st, player, merc, NEW_HIRE, slot, false));
        if let Some(Err(e)) = r {
            self.state.errors.push(InteractionError::Hireling(e));
        }
    }

    /// Revives a dead mercenary.
    pub(super) fn hireling_revive(&mut self, player: UnitId, merc: UnitId) {
        let r = self.with_hirelings(|w, t, st| life::revive(w, t, st, player, merc));
        if let Some(Err(e)) = r {
            self.state.errors.push(InteractionError::Hireling(e));
        }
    }

    /// The player's first hireling pet node: any, or the first with bit 0
    /// clear.
    pub(super) fn hireling_pet(&self, player: UnitId, any: bool) -> Option<UnitId> {
        let node = self.state.hirelings.first_node(player, any)?;
        self.econ.game.lists.find_unit(UnitType::Monster, node.guid)
    }
}
