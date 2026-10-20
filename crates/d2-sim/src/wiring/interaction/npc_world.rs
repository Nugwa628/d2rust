//! [`NpcWorld`] on the real providers: unit records (GUID, class, mode,
//! act, flags), the unit lists (GUID lookup), the stat lists (stats,
//! maxima, states, state lists), the timer queue (the AI think), the item
//! data of the economy wiring (page, flags, sockets, seeds) and the game's
//! [`crate::world::quests::QuestControl`] (text list, flag messages, chat
//! end, Act I hooks, act completion). Everything else is [`NpcRest`].

use super::{Desk, PlayerQuestsRef};
use crate::items::create;
use crate::rng::Seed;
use crate::tick::events::event;
use crate::units::lifecycle::{ActTail, LifecycleHooks};
use crate::units::{UnitId, UnitType};
use crate::wiring::economy::QuestRest;
use crate::world::npc::{
    ImbueMods, InteractionList, InvEntry, ItemFacts, MercInit, NpcWorld, UNIT_MONSTER,
};
use crate::world::quests::{self, act1, QuestFlags, TextList};

/// State flag bit `curable` (`tables/data/fields.tsv`: states column 14,
/// bit 12).
pub const STATE_CURABLE: usize = 12;
/// State 54 (`uninterruptable`), checked before a think is scheduled.
const STATE_UNINTERRUPTABLE: u32 = 54;

/// The NPC calls with no provider yet, each left to the host.
pub trait NpcRest: super::HirelingRest {
    /// The item format of the game (game creation; not in `GameFields`).
    fn item_format(&self) -> u16;
    // ---- positions and paths (movement)
    fn distance(&self, a: UnitId, b: UnitId) -> i32;
    fn axis_check(&self, player: UnitId, npc: UnitId) -> u32;
    fn unit_check(&self, player: UnitId, guid: u32) -> u32;
    fn clear_path(&mut self, unit: UnitId);
    fn approach(&mut self, player: UnitId, npc: UnitId);
    // ---- player data
    fn player_busy(&self, player: UnitId) -> u32;
    /// Whether the player may start an interaction with the NPC (open
    /// question: the rule is not written yet).
    fn start_allowed(&self, player: UnitId, npc: UnitId) -> bool;
    fn tristram_cain_busy(&self, player: UnitId, npc: UnitId) -> bool;
    fn pet(&self, player: UnitId, kind: u8, arg: u8) -> Option<UnitId>;
    fn pets(&self, player: UnitId) -> Vec<UnitId>;
    fn player_name(&self, player: UnitId) -> Vec<u8>;
    /// Stat reset and skill reset (see [`NpcWorld::reset_stats`]).
    fn reset_stats(&mut self, player: UnitId);
    fn reset_skills(&mut self, player: UnitId);
    fn act_change(&mut self, player: UnitId, level: u32, arg: u32);
    fn activate_waypoint(&mut self, player: UnitId, level: u32);
    // ---- monster data (open question)
    fn npc_ai_param(&mut self, npc: UnitId, param: u32);
    // ---- messages and sounds (transport; `send` and `attach_sound` are
    // the quests' [`QuestRest`] ones)
    fn respec_sound(&mut self, player: UnitId);
    /// Encodes the text list as a 34-byte message.
    fn encode_text_list(&self, list: &TextList) -> [u8; 34];
    // ---- Act V quest hooks
    fn socket_granted(&mut self, player: UnitId);
    fn personalize_granted(&mut self, player: UnitId);
    // ---- inventory and item functions (the inventory model)
    /// The player's items in inventory order with their places.
    fn inventory_entries(&self, player: UnitId) -> Vec<InvEntry>;
    /// A host whose inventory model is not the rest's hands the rest the
    /// player's entries before an NPC call (default: ignored).
    fn stage_inventory(&mut self, player: UnitId, entries: Vec<InvEntry>) {
        let _ = (player, entries);
    }
    /// The items the NPC call identified since the last take, for the
    /// host to apply on its inventory model (default: none).
    fn take_identified(&mut self) -> Vec<UnitId> {
        Vec::new()
    }
    fn identify(&mut self, item: UnitId);
    fn cursor_item(&self, player: UnitId) -> Option<UnitId>;
    fn item_facts(&self, item: UnitId) -> ItemFacts;
    fn put_back(&mut self, player: UnitId, item: UnitId);
    fn remove_cursor_item(&mut self, player: UnitId, item: UnitId) -> bool;
    fn duplicate(&mut self, player: UnitId, item: UnitId) -> Option<UnitId>;
    fn create_imbued(&mut self, player: UnitId, input: UnitId, mods: &ImbueMods) -> Option<UnitId>;
    fn item_refresh(&mut self, item: UnitId);
    fn personal_name(&self, item: UnitId) -> Vec<u8>;
    fn set_personal_name(&mut self, item: UnitId, name: &[u8]);
    fn place_or_drop(&mut self, player: UnitId, item: UnitId);
    // ---- mercenaries (monster creation; the mode set is
    // [`super::HirelingRest::set_mode`])
    fn spawn_mercenary(&mut self, near: UnitId, class: u32, mode: u8) -> Option<UnitId>;
}

impl<'a, H: LifecycleHooks, R: NpcRest + QuestRest + PlayerQuestsRef> Desk<'_, 'a, H, R> {
    fn is_monster(&self, unit: UnitId) -> bool {
        self.econ
            .game
            .lists
            .unit(unit)
            .is_some_and(|e| e.ty == UnitType::Monster)
    }

    fn record_mut(&mut self, unit: UnitId) -> Option<&mut crate::units::record::UnitRecord> {
        self.econ.units.get_mut(unit)
    }
}

impl<'a, H: LifecycleHooks, R: NpcRest + QuestRest + PlayerQuestsRef> NpcWorld
    for Desk<'_, 'a, H, R>
{
    fn item_format(&self) -> u16 {
        self.rest.item_format()
    }
    /// The game's rules, from the unit hooks.
    fn rules(&self) -> &crate::rules::Ruleset {
        self.econ.hooks.rules()
    }

    // ---- units

    fn guid(&self, unit: UnitId) -> u32 {
        self.econ.units.get(unit).map_or(u32::MAX, |r| r.guid)
    }
    /// Looks the GUID up in the monster hash.
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.econ.game.lists.find_unit(UnitType::Monster, guid)
    }
    /// TODO: the message's "unit with GUID" names no unit type; read as
    /// the monster hash (only a monster passes the next check; a GUID of
    /// another type answers 1 here, but would answer 3 if that lookup
    /// searched every type).
    fn unit_by_guid(&self, guid: u32) -> Option<(u8, UnitId)> {
        self.monster_by_guid(guid).map(|u| (UNIT_MONSTER, u))
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        if !self.is_monster(unit) {
            return None;
        }
        self.econ.units.get(unit).map(|r| r.class as u16)
    }
    fn mode(&self, unit: UnitId) -> u8 {
        self.econ.units.get(unit).map_or(0, |r| r.mode as u8)
    }
    fn set_mode(&mut self, unit: UnitId, mode: u8) {
        super::HirelingRest::set_mode(&mut *self.rest, unit, mode);
    }
    /// Clears `flag` in the unit record's flags.
    fn clear_unit_flag(&mut self, unit: UnitId, flag: u32) {
        if let Some(r) = self.record_mut(unit) {
            r.flags &= !flag;
        }
    }
    fn distance(&self, a: UnitId, b: UnitId) -> i32 {
        self.rest.distance(a, b)
    }
    fn axis_check(&self, player: UnitId, npc: UnitId) -> u32 {
        self.rest.axis_check(player, npc)
    }
    /// The units' act fields are equal.
    fn same_act(&self, player: UnitId, npc: UnitId) -> bool {
        let act = |u| self.econ.units.get(u).map(|r| r.act);
        act(player).is_some() && act(player) == act(npc)
    }
    fn unit_check(&self, player: UnitId, guid: u32) -> u32 {
        self.rest.unit_check(player, guid)
    }
    fn player_busy(&self, player: UnitId) -> u32 {
        self.rest.player_busy(player)
    }
    fn start_allowed(&self, player: UnitId, npc: UnitId) -> bool {
        self.rest.start_allowed(player, npc)
    }
    fn tristram_cain_busy(&self, player: UnitId, npc: UnitId) -> bool {
        self.rest.tristram_cain_busy(player, npc)
    }
    /// The game's path provider ([`crate::units::hooks::UnitHooks::stop_path_now`]),
    /// else the rest.
    fn clear_path(&mut self, unit: UnitId) {
        if !self.econ.hooks.stop_path_now(unit) {
            self.rest.clear_path(unit);
        }
    }
    /// The game's AI store ([`crate::units::hooks::UnitHooks::set_ai_param0`]),
    /// else the rest.
    fn npc_ai_param(&mut self, npc: UnitId, param: u32) {
        if !self.econ.hooks.set_ai_param0(npc, param as i32) {
            self.rest.npc_ai_param(npc, param);
        }
    }
    /// AI params 1 and 2 (C→S 0x59): the game's AI store; the rest keeps
    /// no AI params.
    fn npc_ai_point(&mut self, npc: UnitId, x: u32, y: u32) {
        self.econ.hooks.set_ai_param(npc, 1, x as i32);
        self.econ.hooks.set_ai_param(npc, 2, y as i32);
    }
    /// Cancels the NPC's type-2 events, then schedules one at the next
    /// frame, after the state-54 check (as the monster neutral start
    /// does).
    ///
    /// TODO: the think's arguments are not written; 0, 0 as every other
    /// AI-think schedule.
    fn reschedule_ai_think(&mut self, npc: UnitId) {
        let f = self.econ.game.frame;
        self.econ
            .game
            .timers
            .cancel_unit_events(npc, event::AI_THINK, None);
        if self.econ.stats.has_state(npc, STATE_UNINTERRUPTABLE) {
            let (mut sim, hooks) = self.econ.split();
            hooks.uninterruptable_check(&mut sim, npc);
        }
        let r = self.econ.game.schedule_event(
            npc,
            u32::from(event::AI_THINK),
            f.wrapping_add(1),
            None,
            0,
            0,
        );
        if let Err(e) = r {
            self.state
                .errors
                .push(super::InteractionError::Economy(e.into()));
        }
    }
    fn approach(&mut self, player: UnitId, npc: UnitId) {
        self.state.approaches.push((player, npc));
        self.rest.approach(player, npc);
    }
    fn interaction(&mut self, npc: UnitId) -> Option<&mut InteractionList> {
        self.state.lists.get_mut(&npc)
    }
    /// The player's interact unit, read from the player's unit record
    /// ([`crate::units::record::InteractInfo`]); no record → none.
    fn interact_unit(&self, player: UnitId) -> Option<(u8, u32)> {
        self.econ.units.get(player)?.interact.get()
    }
    /// Sets the interact unit (ignored while active).
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        if let Some(r) = self.record_mut(player) {
            r.interact.set(unit_type, guid);
        }
    }
    /// Resets the interact unit.
    fn reset_interact(&mut self, player: UnitId) {
        if let Some(r) = self.record_mut(player) {
            r.interact.reset();
        }
    }
    /// Kind 7: the hireling list; other kinds are the rest's.
    fn pet(&self, player: UnitId, kind: u8, arg: u8) -> Option<UnitId> {
        if kind == crate::world::hirelings::PET_HIRELING {
            return self.hireling_pet(player, arg != 0);
        }
        self.rest.pet(player, kind, arg)
    }
    fn pets(&self, player: UnitId) -> Vec<UnitId> {
        self.rest.pets(player)
    }

    // ---- stats and states

    fn stat(&self, unit: UnitId, stat: u16) -> u32 {
        self.econ.stats.unit_total(unit, stat, 0) as u32
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> u32 {
        self.econ.stats.unit_base(unit, stat, 0) as u32
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: u32) {
        self.econ
            .stats
            .unit_set(&mut *self.econ.hooks, unit, stat, value as i32, 0);
    }
    /// The stat set, then its message to the player's client at once
    /// (SetStat 0x1D / 0x1E / 0x1F by the value's size). Recorded:
    /// `a2-npc-fara-heal` frame 14, `1e 06 00 32` inside the 0x2F
    /// handling, before the client's 0x31.
    fn set_stat_send(&mut self, player: UnitId, stat: u16, value: u32) {
        NpcWorld::set_stat(self, player, stat, value);
        if let Some(m) = crate::wiring::action::vitals_sync::stat_message(stat, value as i32) {
            QuestRest::send(&mut *self.rest, player, &m);
        }
    }
    fn max_life(&self, unit: UnitId) -> u32 {
        self.econ.stats.max_life(unit) as u32
    }
    fn max_mana(&self, unit: UnitId) -> u32 {
        self.econ.stats.max_mana(unit) as u32
    }
    fn max_stamina(&self, unit: UnitId) -> u32 {
        self.econ.stats.max_stamina(unit) as u32
    }
    fn states_count(&self) -> u16 {
        self.econ.stats.data().states.count() as u16
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.econ.stats.has_state(unit, u32::from(state))
    }
    /// The state's `curable` flag bit.
    fn curable(&self, state: u16) -> bool {
        self.econ
            .stats
            .data()
            .states
            .has_flag(u32::from(state), STATE_CURABLE)
    }
    /// Whether the unit has a stat list of `state`.
    fn has_state_list(&self, unit: UnitId, state: u16) -> bool {
        let s = &self.econ.stats;
        s.unit_list(unit)
            .and_then(|r| s.list_of_state(r, u32::from(state)))
            .is_some()
    }
    fn remove_state_list(&mut self, unit: UnitId, state: u16) {
        self.econ
            .stats
            .free_state_list(&mut *self.econ.hooks, unit, u32::from(state));
    }
    /// The unit's sound event, sent as S→C 0x2C by the client pass of the
    /// tick (`units::sound`). Recorded: `a2-npc-fara-heal` frame 15,
    /// `2c 01 0a000000 0a00`.
    fn attach_sound(&mut self, unit: UnitId, sound: u16) {
        // No target: the sound goes to every client.
        let _ = crate::units::sound::queue_sound(&mut *self.econ.game, unit, sound, None);
    }
    fn attach_sound_to(&mut self, unit: UnitId, sound: u16, target: UnitId) {
        let _ = crate::units::sound::queue_sound(&mut *self.econ.game, unit, sound, Some(target));
    }

    fn record_menu(&mut self, class: u32) {
        self.econ.game.coverage.hit("npc_menus_opened", || class);
    }

    fn send(&mut self, player: UnitId, msg: &[u8]) {
        QuestRest::send(&mut *self.rest, player, msg);
    }

    // ---- quests

    fn quest_flags(&self, player: UnitId) -> QuestFlags {
        let d = usize::from(self.econ.fields.difficulty);
        self.rest
            .quests_ref(player)
            .map(|q| q.flags[d])
            .unwrap_or_default()
    }
    fn quest_text_list(&mut self, player: UnitId, npc: UnitId) -> TextList {
        let mut list = TextList::new();
        // The chat's held-item tests read the host's inventory model; the
        // rest has no item list.
        let held = self
            .inv
            .as_deref()
            .map(|i| (i.cursor_item(player), i.items_of(player)));
        let (ctl, mut w) = self.quest_world();
        w.held = held;
        ctl.npc_activate(&mut w, player, npc, &mut list);
        list
    }
    fn encode_text_list(&self, list: &TextList) -> [u8; 34] {
        self.rest.encode_text_list(list)
    }
    fn send_game_quests(&mut self, player: UnitId) {
        let (ctl, mut w) = self.quest_world();
        ctl.send_game_flags(&mut w, player);
    }
    fn send_player_quests(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        let (_, mut w) = self.quest_world();
        quests::send_player_flags(&mut w, player, unit_type, guid);
    }
    fn quest_chat_end(&mut self, player: UnitId, npc: UnitId) {
        if self.state.defer_chat_end {
            self.state.chat_ends.push((player, npc));
            return;
        }
        let (ctl, mut w) = self.quest_world();
        ctl.npc_deactivate(&mut w, player, npc);
    }
    fn respec_offer(&mut self, player: UnitId) {
        let (_, mut w) = self.quest_world();
        act1::respec_offer(&mut w, player);
    }
    fn respec_done(&mut self, player: UnitId) {
        let (ctl, mut w) = self.quest_world();
        act1::respec_done(ctl, &mut w, player);
    }
    fn imbue_granted(&mut self, player: UnitId) {
        let (ctl, mut w) = self.quest_world();
        act1::imbue_granted(ctl, &mut w, player);
    }
    fn socket_granted(&mut self, player: UnitId) {
        self.rest.socket_granted(player);
    }
    fn personalize_granted(&mut self, player: UnitId) {
        self.rest.personalize_granted(player);
    }
    /// `QuestControl::act_completion` takes the NPC's class; `level` and
    /// `from` are not read on the quests side.
    fn act_completion(&mut self, player: UnitId, npc: UnitId, level: u32, from: u32) {
        if self.econ.hooks.hold_act_tail(ActTail::Completion {
            player,
            npc,
            level,
            from,
        }) {
            return;
        }
        let class = NpcWorld::monster_class(self, npc).unwrap_or(0);
        let r = {
            let (ctl, mut w) = self.quest_world();
            ctl.act_completion(&mut w, player, class)
        };
        if let Err(e) = r {
            self.state.errors.push(super::InteractionError::Quest(e));
        }
    }

    // ---- player

    /// The stat reset. TODO(wiring): not yet run through
    /// `combat::vitals::reset_stats`: the desk holds no `VitalsTables`,
    /// and the skill reset that runs just before it has no d2-sim body
    /// yet; both stay on the rest.
    fn reset_stats(&mut self, player: UnitId) {
        self.rest.reset_stats(player);
    }
    fn reset_skills(&mut self, player: UnitId) {
        self.rest.reset_skills(player);
    }
    fn respec_sound(&mut self, player: UnitId) {
        self.rest.respec_sound(player);
    }
    fn player_name(&self, player: UnitId) -> Vec<u8> {
        self.rest.player_name(player)
    }
    fn act_change(&mut self, player: UnitId, level: u32, arg: u32) {
        self.rest.act_change(player, level, arg);
        self.econ.hooks.request_act_change(player, level, arg);
    }
    fn activate_waypoint(&mut self, player: UnitId, level: u32) {
        if self
            .econ
            .hooks
            .hold_act_tail(ActTail::Waypoint { player, level })
        {
            return;
        }
        self.rest.activate_waypoint(player, level);
    }

    // ---- items

    fn inventory(&self, player: UnitId) -> Vec<InvEntry> {
        self.rest.inventory_entries(player)
    }
    fn identify(&mut self, item: UnitId) {
        self.rest.identify(item);
    }
    fn cursor_item(&self, player: UnitId) -> Option<UnitId> {
        match &self.inv {
            Some(v) => v.cursor_item(player),
            None => self.rest.cursor_item(player),
        }
    }
    fn item_facts(&self, item: UnitId) -> ItemFacts {
        match self.inv {
            Some(_) => self.facts_of(item).unwrap_or_default(),
            None => self.rest.item_facts(item),
        }
    }
    fn put_back(&mut self, player: UnitId, item: UnitId) {
        self.rest.put_back(player, item);
    }
    fn remove_cursor_item(&mut self, player: UnitId, item: UnitId) -> bool {
        match self.inv.as_deref_mut() {
            Some(v) => v.remove_cursor_item(&mut *self.econ, player, item),
            None => self.rest.remove_cursor_item(player, item),
        }
    }
    fn duplicate(&mut self, player: UnitId, item: UnitId) -> Option<UnitId> {
        self.rest.duplicate(player, item)
    }
    fn create_imbued(&mut self, player: UnitId, input: UnitId, mods: &ImbueMods) -> Option<UnitId> {
        if self.inv.is_none() {
            return self.rest.create_imbued(player, input, mods);
        }
        // The input has left the cursor; its record is still in the item
        // store until the new item is made. It is freed after.
        let new = self.imbue_from(input, mods);
        if let Err(e) = self.econ.free_item(input) {
            self.state.errors.push(super::InteractionError::Economy(e));
        }
        new
    }
    /// Sets item flag 0x10 (identified) first, then the owner's refresh.
    /// The imbue's rare item, left unidentified by its quality routine,
    /// is sent identified (item flags 0x802010).
    fn item_refresh(&mut self, item: UnitId) {
        if let Some(i) = self.econ.items.get_mut(item) {
            i.flags |= 0x10;
        }
        self.rest.item_refresh(item);
    }
    /// Item data inventory page.
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        if let Some(i) = self.econ.items.get_mut(item) {
            i.inv_page = page;
        }
    }
    /// Item data flags.
    fn set_item_flag(&mut self, item: UnitId, flag: u32) {
        if let Some(i) = self.econ.items.get_mut(item) {
            i.flags |= flag;
        }
    }
    fn personal_name(&self, item: UnitId) -> Vec<u8> {
        self.rest.personal_name(item)
    }
    fn set_personal_name(&mut self, item: UnitId, name: &[u8]) {
        self.rest.set_personal_name(item, name);
    }
    fn place_or_drop(&mut self, player: UnitId, item: UnitId) {
        match self.inv.as_deref_mut() {
            // D2RUST's own, unverified: with no free spot the item stays on
            // the cursor (the ground drop near the player is unwired).
            Some(v) => {
                if !v.place(&mut *self.econ, player, item) {
                    self.rest.place_or_drop(player, item);
                }
            }
            None => self.rest.place_or_drop(player, item),
        }
    }
    /// The most sockets the item's record and level allow.
    fn max_sockets(&self, item: UnitId) -> u32 {
        let Some(i) = self.econ.items.get(item) else {
            return 0;
        };
        let mut probe = crate::items::Item::new(i.record, i.format, super::NoStats);
        probe.ilvl = i.ilvl;
        create::max_sockets(self.econ.tables, &probe) as u32
    }
    /// Adds `n` sockets to the assembled item.
    fn add_sockets(&mut self, item: UnitId, n: u32) {
        let r = self
            .econ
            .with_item(item, |s| create::socket_count(s.tables, s.item, n as i32));
        if let Err(e) = r {
            self.state.errors.push(super::InteractionError::Economy(e));
        }
    }
    /// The item's unit seed in its unit record.
    ///
    /// Panics when the unit has no record: the services ask only for the
    /// items they hold.
    fn item_seed(&mut self, item: UnitId) -> &mut Seed {
        &mut self
            .econ
            .units
            .get_mut(item)
            .expect("item unit record")
            .seed
    }

    // ---- mercenaries

    fn spawn_mercenary(&mut self, near: UnitId, class: u32, mode: u8) -> Option<UnitId> {
        self.spawn_mercenary_spread(near, class, mode, 4)
    }
    fn spawn_mercenary_spread(
        &mut self,
        near: UnitId,
        class: u32,
        mode: u8,
        spread: i32,
    ) -> Option<UnitId> {
        let mut sim = crate::units::hooks::Sim {
            game: &mut *self.econ.game,
            units: &mut *self.econ.units,
            stats: &mut *self.econ.stats,
            data: self.econ.data,
        };
        let seed = &mut self.econ.fields.seed;
        if let Some(u) = self
            .econ
            .hooks
            .spawn_near_spread(&mut sim, seed, near, class, mode, spread)
        {
            return Some(u);
        }
        // The rest knows only the hire's spread.
        if spread != 4 {
            return None;
        }
        self.rest.spawn_mercenary(near, class, mode)
    }
    /// Initialises a newly hired mercenary.
    fn init_mercenary(&mut self, player: UnitId, merc: UnitId, init: &MercInit) {
        self.hireling_init(player, merc, init);
    }
    /// Revives a dead mercenary.
    fn revive_mercenary(&mut self, player: UnitId, merc: UnitId) {
        self.hireling_revive(player, merc);
    }
}
