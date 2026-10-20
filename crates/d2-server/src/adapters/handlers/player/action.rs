//! [`ActionPlayer`]: the [`PlayerWorld`] of a game wired on
//! `d2_sim::wiring::action::ActionSim`. The unit records (mode, flags),
//! the unit lists (GUID lookup, rooms, update queue), the stat lists
//! (stats, maxima, states), the timer queue, the tables (skills,
//! monstats), the AI store (commands), the DRLG (current level) and the
//! path provider (warp, mode request) answer directly; every call with
//! no `d2-sim` implementation yet goes to the wiring's `Pending` value
//! (its player-handler block, and its sound, skill-list, busy and warp
//! calls). It is the provider the server's player handlers
//! ([`super::run`]) run on.

use d2_sim::game::Game;
use d2_sim::skills::SkillEntry;
use d2_sim::units::hooks::Sim;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{Pending, View};

use super::{HostFacts, Outcome, PlayerWorld, Run};
use crate::adapters::handlers::world::ActionEvents;

/// The action wiring's game state as a [`PlayerWorld`], for one call.
pub struct ActionPlayer<'a, 'v, X> {
    pub game: &'a mut Game,
    pub v: &'a mut View<'v, X>,
    /// What the host answered beside the action wiring.
    pub facts: HostFacts,
}

/// Runs the handler of `r` on the action wiring of `events`.
pub fn run<D: ActionEvents>(
    game: &mut Game,
    events: &mut D,
    r: &Run<'_>,
    facts: HostFacts,
) -> Outcome {
    events.action().with(game, |g, v| {
        let mut w = ActionPlayer { game: g, v, facts };
        super::run(&mut w, r)
    })
}

impl<X: Pending> ActionPlayer<'_, '_, X> {
    fn sim(&mut self) -> (Sim<'_>, &mut d2_sim::wiring::action::ActionHooks<X>) {
        let v = &mut *self.v;
        (
            Sim {
                game: &mut *self.game,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            },
            &mut *v.h,
        )
    }
}

impl<X: Pending> PlayerWorld for ActionPlayer<'_, '_, X> {
    fn frame(&self) -> i32 {
        self.game.frame
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.game.tick_rate
    }
    /// `UnitData::expansion`, the creation field's home.
    fn expansion(&self) -> bool {
        self.v.data.expansion
    }
    /// `ActionHooks::ai_info`, the creation field's home.
    fn difficulty(&self) -> u8 {
        self.v.h.ai_info.difficulty
    }
    fn mode(&self, u: UnitId) -> u32 {
        self.v.units.get(u).map_or(0, |r| r.mode)
    }
    fn set_mode(&mut self, u: UnitId, mode: u32) {
        let r = {
            let (mut sim, h) = self.sim();
            d2_sim::units::mode_set::set_mode_only(&mut sim, h, u, mode)
        };
        if let Err(e) = r {
            self.v.unit_error(e);
        }
    }
    fn flags(&self, u: UnitId) -> u32 {
        self.v.units.get(u).map_or(0, |r| r.flags)
    }
    fn or_flags(&mut self, u: UnitId, bits: u32) {
        if let Some(r) = self.v.units.get_mut(u) {
            r.flags |= bits;
        }
    }
    fn or_flags_ex(&mut self, u: UnitId, bits: u32) {
        if let Some(r) = self.v.units.get_mut(u) {
            r.flags2 |= bits;
        }
    }
    fn stat(&self, u: UnitId, stat: u16) -> i32 {
        self.v.stat(u, stat)
    }
    fn set_state(&mut self, u: UnitId, state: u16, on: bool) {
        self.v.set_state(u, state, on);
        self.queue_update(u);
    }
    fn queue_update(&mut self, u: UnitId) {
        if let Err(e) = self.game.lists.queue_update(u) {
            self.v
                .unit_error(d2_sim::units::modes::UnitError::Game(e.into()));
        }
    }
    fn schedule_event(&mut self, u: UnitId, event: u32, expire: i32) {
        if let Err(e) = self.game.schedule_event(u, event, expire, None, 0, 0) {
            self.v.unit_error(d2_sim::units::modes::UnitError::Game(e));
        }
    }
    /// `Pending::play_sound` (the sound system is not implemented yet).
    fn sound(&mut self, u: UnitId, sound: u32, to: Option<UnitId>) {
        self.v.h.x.play_sound(self.game, u, sound, to);
    }
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.game.lists.find_unit(ty, guid)
    }
    /// The unit's room's adjacency list (`RoomEntry::adjacent`, the room
    /// itself included) holds the player's room.
    fn room_in_list_of(&self, player: UnitId, unit: UnitId) -> bool {
        let l = &self.game.lists;
        let (Some(pr), Some(ur)) = (
            l.unit(player).and_then(|e| e.room()),
            l.unit(unit).and_then(|e| e.room()),
        ) else {
            return false;
        };
        l.room(ur).is_some_and(|r| r.adjacent.contains(&pr))
    }
    fn is_dead(&self, u: UnitId) -> bool {
        self.v.units.is_dead(u)
    }
    /// `Pending::used_skill`.
    fn has_used_skill(&self, u: UnitId) -> bool {
        self.v.h.x.used_skill(u).is_some()
    }
    fn skill_count(&self) -> u32 {
        self.v.h.tables.skills.skills.len() as u32
    }
    /// An entry of the unit's skill list (`ActionHooks::skill_list_of`:
    /// the wired list, else `Pending::skill_list`) with the skill and
    /// owner GUID `item`.
    fn has_skill(&self, u: UnitId, skill: i32, item: u32) -> bool {
        self.v
            .h
            .skill_list_of(u)
            .iter()
            .any(|e| e.skill == skill && e.owner_guid == item as i32)
    }
    fn monstats_count(&self) -> u32 {
        self.v.h.tables.combat.monstats.len() as u32
    }
    fn overhead_text_test(&self, text: &[u8]) -> bool {
        self.v.h.x.overhead_text_test(text)
    }
    /// `View::replace_overhead`: the unit's overhead timeout frame
    /// (`UnitRecord::hover`, read by event 6) and the record the overhead
    /// 0x26 sends.
    fn replace_overhead(&mut self, u: UnitId, text: &[u8], byte8: u8, end: i32) {
        self.v.replace_overhead(u, text, byte8, end);
    }
    fn highlight_door(&mut self, player: UnitId, guid: u32) {
        self.v.h.x.highlight_door(self.game, player, guid);
    }
    fn send(&mut self, to: UnitId, bytes: &[u8]) {
        self.v.h.x.send(to, bytes);
    }
    fn relation(&self, a: UnitId, b: UnitId, flag: u32) -> bool {
        self.v.h.x.player_relation(a, b, flag)
    }
    fn chat_command(&mut self, p: UnitId, text: &[u8]) -> bool {
        if self.v.h.rules.events().is_none() {
            return false;
        }
        let mut cv = self.v.combat(self.game);
        d2_sim::wiring::action::mod_events::fire(&mut cv, |e, h| e.chat_command(h, p, text))
            .unwrap_or(false)
    }
    /// The interact info on the unit record, then the player list (type 0).
    fn interacting_player(&self, u: UnitId) -> Option<UnitId> {
        match self.v.units.get(u)?.interact.get()? {
            (0, guid) => self.game.lists.find_unit(UnitType::Player, guid),
            _ => None,
        }
    }
    fn hardcore(&self, player: UnitId) -> bool {
        self.v.h.x.client_hardcore(player)
    }
    fn drop_client(&mut self, player: UnitId, reason: u32) {
        self.v.h.x.drop_client(player, reason);
    }
    /// `Pending::skill_list`.
    fn skill_entries(&self, u: UnitId) -> Vec<SkillEntry> {
        self.v.h.x.skill_list(u)
    }
    fn passive_state(&self, skill: i32) -> i32 {
        usize::try_from(skill)
            .ok()
            .and_then(|i| self.v.h.tables.skills.skills.get(i))
            .map_or(-1, |r| i32::from(r.passivestate as i16))
    }
    /// `Pending::passive_state_apply`.
    fn passive_state_apply(&mut self, u: UnitId, e: &SkillEntry) {
        self.v.h.x.passive_state_apply(u, e);
    }
    fn stat_max(&self, u: UnitId, stat: u16) -> i32 {
        let s = &self.v.stats;
        match stat {
            super::STAT_LIFE => s.max_life(u),
            super::STAT_MANA => s.max_mana(u),
            super::STAT_STAMINA => s.max_stamina(u),
            _ => 0,
        }
    }
    /// The set on the stat lists, then `Pending::stat_sent`.
    fn set_stat_send(&mut self, u: UnitId, stat: u16, value: i32) {
        self.v.set_base(u, stat, value);
        self.v.h.x.stat_sent(u, stat, value as u32);
    }
    /// The DRLG level of the unit's room (`ActionHooks::drlg`).
    fn current_level(&self, u: UnitId) -> u32 {
        self.game
            .lists
            .unit(u)
            .and_then(|e| e.room())
            .and_then(|r| self.v.h.drlg.level_id(self.game, r))
            .unwrap_or(0)
    }
    /// As the waypoints' warp (`ActionSim`'s waypoint view): with the
    /// path provider the same-act warp; an act change, or no provider,
    /// goes to `Pending::warp`.
    fn warp(&mut self, player: UnitId, level: u32, arg: u32) {
        if self.v.h.paths.is_some() {
            let c = d2_sim::wiring::path::PathCtx::of(self.v, self.game);
            if d2_sim::wiring::path::place::level_warp(c, player, level, arg).is_some() {
                return;
            }
        }
        self.v.h.x.warp(self.game, player, level, arg as u8);
    }
    /// The path provider's mode request.
    ///
    /// Without the path provider no mode request runs (none is wired
    /// outside it; a D2RUST wiring choice).
    fn start_mode_skip_gate(&mut self, player: UnitId, mode: u32) {
        if self.v.h.paths.is_some() {
            d2_sim::wiring::path::walk::request_skip_gate(self.v, self.game, player, mode, 0, 0);
        }
    }
    fn reselect_hand_skills(&mut self, player: UnitId) {
        self.v.h.x.reselect_hand_skills(self.game, player);
    }
    /// The interact info on the player's unit record
    /// (active), then `Pending::inventory_busy`.
    fn busy(&self, player: UnitId) -> bool {
        self.v.units.get(player).is_some_and(|r| r.interact.active)
            || self.v.h.x.inventory_busy(player)
    }
    fn trading(&self, player: UnitId) -> bool {
        self.v.h.x.player_trading(player)
    }
    fn staff_in_orifice(
        &mut self,
        player: UnitId,
        object: u32,
        item: u32,
        action: u16,
    ) -> Option<u32> {
        self.v
            .h
            .x
            .staff_in_orifice(self.game, player, object, item, action)
    }
    /// The host's hireling list when it has one, else
    /// `Pending::player_hireling`.
    fn hireling(&self, player: UnitId) -> Option<UnitId> {
        match self.facts.hireling {
            Some(h) => h,
            None => self.v.h.x.player_hireling(player),
        }
    }
    /// On the AI store's control record (`AiControl::commands`): all
    /// freed, then the one command current. A monster without a control
    /// record (or the store lent out) is left alone.
    fn replace_ai_commands(&mut self, monster: UnitId, command: [i32; 5]) {
        let Some(c) = self.v.h.ai.as_mut().and_then(|s| s.control_mut(monster)) else {
            return;
        };
        c.commands.clear();
        c.commands
            .push(d2_sim::monsters::ai::AiCommand { params: command });
        c.cur = 0;
    }
    /// `Pending::object_player_busy` (the player data's busy byte set).
    fn busy_flag(&self, player: UnitId) -> bool {
        self.v.h.x.object_player_busy(player)
    }
    fn clear_busy_flag(&mut self, player: UnitId) {
        self.v.h.x.clear_player_busy(player);
    }
    fn clear_npc_intro(&mut self, player: UnitId, difficulty: u8, class: u16) {
        self.v.h.x.clear_npc_intro(player, difficulty, class);
    }
    fn weapon_switch(&mut self, player: UnitId) -> Option<(u32, bool)> {
        self.v.h.x.weapon_switch(self.game, player)
    }
}
