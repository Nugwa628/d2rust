//! One game's simulation state: the frame counter, the unit/room/client
//! lists, the timer queue and the monster target lists, plus the
//! operations that touch more than one of them.

use thiserror::Error;

use crate::tick::timer::{CallbackId, TimerError, TimerId, TimerOwner, TimerQueue};
use crate::tick::TickRate;
use crate::units::sound::SoundEvents;
use crate::units::{ListError, RoomId, UnitId, UnitLists, UnitType};

/// Errors of the game-level operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GameError {
    #[error(transparent)]
    List(#[from] ListError),
    #[error(transparent)]
    Timer(#[from] TimerError),
}

/// A game.
#[derive(Clone, Debug, Default)]
pub struct Game {
    /// Frame counter: signed, 0 at creation, +1 at the start of every
    /// tick.
    pub frame: i32,
    /// Diagnostics excluded from deterministic snapshots.
    pub coverage: crate::debug::coverage::Counters,
    /// Ticks per second this game runs at; speeds and durations from the
    /// tables are stated at the base rate and converted through it.
    pub tick_rate: TickRate,
    pub lists: UnitLists,
    pub timers: TimerQueue,
    /// Unit sound-event slots, for units with the sound-event flag
    /// ([`crate::units::sound`]).
    pub sounds: SoundEvents,
    /// The client pass asked for the character save of every client with
    /// a player (the client pass's autosave rule): the save writes a
    /// file, so the host (`d2-server`'s character storage) runs it and
    /// clears this after the tick.
    pub character_save_due: bool,
    /// The units joined to the monster target lists ([`TargetNodes`]).
    pub target_nodes: TargetNodes,
    /// Level 134 preset-object counter (0 when the game is created): the
    /// level's object presets are counted and the one that brings the
    /// count to `init seed % 3 + 3` is skipped (monster population).
    pub sands_preset_objects: u32,
}

/// The target-node list a unit belongs to, as its node index holds it.
///
/// Monster AI finds its targets on the game's target-node lists: one list
/// per player (the player at its head, then the units attached to it: its
/// mercenary and its summons), one list of good NPCs and one of bone walls.
/// A unit's node index names its list:
///
/// - 0–7: the first eight player lists;
/// - 8: good NPCs; 9: bone walls; 11: no list;
/// - 12 and up: the player lists past the eighth (list 8 is code 12), so a
///   game takes as many players as its limit allows and the codes of the
///   first eight players, and every rule that reads them, stay as they are.
pub mod node {
    /// The good NPCs' list.
    pub const GOOD_NPC: u32 = 8;
    /// The bone walls' list.
    pub const BONE_WALLS: u32 = 9;
    /// In no list (a unit's node index when it is created).
    pub const NONE: u32 = 11;
    /// The code of player list 8; the later lists follow it.
    pub const MORE_PLAYERS: u32 = 12;

    /// The code of player list `list`.
    pub fn player(list: u32) -> u32 {
        if list < GOOD_NPC {
            list
        } else {
            MORE_PLAYERS + (list - GOOD_NPC)
        }
    }

    /// The player list a code names, if it names one.
    pub fn player_list(code: u32) -> Option<u32> {
        match code {
            0..GOOD_NPC => Some(code),
            c if c >= MORE_PLAYERS => Some(c - MORE_PLAYERS + GOOD_NPC),
            _ => None,
        }
    }
}

/// The target-node lists in the order monster AI reads them: every player
/// list (list 0 first), then the good NPCs, then the bone walls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetLists {
    /// One list per player list index; an empty list is a free index.
    pub players: Vec<Vec<UnitId>>,
    /// The good NPCs (newest first).
    pub good: Vec<UnitId>,
    /// The bone walls (newest first).
    pub walls: Vec<UnitId>,
}

impl TargetLists {
    /// The list a node-index `code` names (player lists are added as
    /// needed); `None` for "no list" and unknown codes.
    pub fn list_mut(&mut self, code: u32) -> Option<&mut Vec<UnitId>> {
        if let Some(l) = node::player_list(code) {
            let l = l as usize;
            if self.players.len() <= l {
                self.players.resize_with(l + 1, Vec::new);
            }
            return self.players.get_mut(l);
        }
        match code {
            node::GOOD_NPC => Some(&mut self.good),
            node::BONE_WALLS => Some(&mut self.walls),
            _ => None,
        }
    }

    /// The list a code names, in list order (empty when there is none).
    pub fn list(&self, code: u32) -> &[UnitId] {
        let list = match node::player_list(code) {
            Some(l) => self.players.get(l as usize),
            None => match code {
                node::GOOD_NPC => Some(&self.good),
                node::BONE_WALLS => Some(&self.walls),
                _ => None,
            },
        };
        list.map_or(&[], Vec::as_slice)
    }

    /// Lists built from codes in order: `lists[i]` is the list of code
    /// `i` (0–7 players, 8 good NPCs, 9 bone walls, 12 and up players).
    pub fn from_codes(lists: &[Vec<UnitId>]) -> Self {
        let mut out = Self::default();
        for (code, l) in lists.iter().enumerate() {
            if let Some(dst) = out.list_mut(code as u32) {
                dst.extend_from_slice(l);
            }
        }
        out
    }
}

/// The nodes units joined to the game's target-node lists (good NPCs,
/// bone walls, or a player's attached units), in list order. A player
/// list's head (the player itself) is the host's
/// (`AiTargets::target_nodes`); the head is not stored here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetNodes {
    lists: TargetLists,
}

impl TargetNodes {
    /// The node pushed at the head of the list `slot` names (so the good
    /// NPCs' and bone walls' lists run newest first). A code naming no
    /// list adds nothing.
    pub fn push_front(&mut self, slot: i32, unit: UnitId) {
        if let Some(list) = u32::try_from(slot)
            .ok()
            .and_then(|s| self.lists.list_mut(s))
        {
            list.insert(0, unit);
        }
    }

    /// The unit's node unlinked from whichever list holds it.
    pub fn remove(&mut self, unit: UnitId) {
        for list in self.lists.players.iter_mut() {
            list.retain(|&u| u != unit);
        }
        self.lists.good.retain(|&u| u != unit);
        self.lists.walls.retain(|&u| u != unit);
    }

    /// The inserted nodes of the list `slot` names, in list order.
    pub fn slot(&self, slot: usize) -> &[UnitId] {
        u32::try_from(slot).map_or(&[], |s| self.lists.list(s))
    }

    /// Every inserted list.
    pub fn lists(&self) -> &TargetLists {
        &self.lists
    }
}

impl Game {
    pub fn new() -> Self {
        Self::default()
    }

    /// A duration of `base_ticks` base-rate ticks as ticks at this game's
    /// rate ([`TickRate::ticks`]).
    pub fn ticks(&self, base_ticks: i32) -> i32 {
        self.tick_rate.ticks(base_ticks)
    }

    /// The frame `base_ticks` base-rate ticks from now: the expire frame
    /// of a timer or state of that duration.
    pub fn after(&self, base_ticks: i32) -> i32 {
        self.frame.wrapping_add(self.ticks(base_ticks))
    }

    /// Allocates a GUID and adds the unit to its lists. Deriving the
    /// unit's seed, which comes first, is the unit allocator's job.
    pub fn spawn_unit(
        &mut self,
        ty: UnitType,
        room: Option<RoomId>,
        allied: bool,
    ) -> Result<UnitId, GameError> {
        let guid = self.lists.guids.alloc(ty);
        Ok(self.lists.add_unit(ty, guid, room, allied)?)
    }

    /// Unit removal: list unlinks,
    /// then all the unit's timers are cancelled. Immediate, also during
    /// the timer run.
    pub fn remove_unit(&mut self, unit: UnitId) -> Result<(), GameError> {
        self.lists.remove_unit(unit)?;
        self.timers.remove_unit(unit);
        self.sounds.clear(unit);
        self.target_nodes.remove(unit);
        Ok(())
    }

    /// The timer owner record of a unit.
    pub fn owner(&self, unit: UnitId) -> Result<TimerOwner, GameError> {
        let e = self.lists.unit(unit).ok_or(ListError::UnknownUnit(unit))?;
        Ok(TimerOwner {
            unit,
            unit_type: e.ty,
            guid: e.guid,
        })
    }

    /// Schedules an event for `unit` at the current frame
    /// ([`TimerQueue::schedule`]).
    pub fn schedule_event(
        &mut self,
        unit: UnitId,
        event: u32,
        expire: i32,
        callback: Option<CallbackId>,
        arg1: u32,
        arg2: u32,
    ) -> Result<Option<TimerId>, GameError> {
        let owner = self.owner(unit)?;
        Ok(self
            .timers
            .schedule(self.frame, owner, event, expire, callback, arg1, arg2)?)
    }

    /// For players or monsters (other types: nothing), calls `f` for each unit in hash iteration order
    /// that `has_player_body_state` rejects, and stops at the first call
    /// returning true, returning that unit. The next link is read after
    /// the call: `f` must not remove the unit it is given.
    ///
    /// `has_player_body_state` answers "has state 7"; TODO(states spec):
    /// read it from the unit's states once they exist.
    pub fn find_unit_of_type(
        &mut self,
        ty: UnitType,
        mut has_player_body_state: impl FnMut(&Game, UnitId) -> bool,
        mut f: impl FnMut(&mut Game, UnitId) -> bool,
    ) -> Option<UnitId> {
        if !matches!(ty, UnitType::Player | UnitType::Monster) {
            return None;
        }
        let mut cur = self.lists.first_of_type(ty);
        while let Some(u) = cur {
            if !has_player_body_state(self, u) && f(self, u) {
                return Some(u);
            }
            cur = self.lists.next_of_type(u);
        }
        None
    }

    /// Calls `f` for each player
    /// without state 7, in hash iteration order, without an early stop.
    /// Same next-link rule as [`Self::find_unit_of_type`].
    pub fn for_each_player(
        &mut self,
        mut has_player_body_state: impl FnMut(&Game, UnitId) -> bool,
        mut f: impl FnMut(&mut Game, UnitId),
    ) {
        let mut cur = self.lists.first_of_type(UnitType::Player);
        while let Some(u) = cur {
            if !has_player_body_state(self, u) {
                f(self, u);
            }
            cur = self.lists.next_of_type(u);
        }
    }
}

/// Allocation apart from adding the unit to its lists (see
/// `units::lists` `alloc`): the type's init may schedule timers in
/// between (missile setup).
impl Game {
    /// Unit allocation: a GUID and a
    /// unit in no list yet. [`Game::spawn_unit`] also adds it.
    pub fn alloc_unit(&mut self, ty: UnitType, allied: bool) -> UnitId {
        let guid = self.lists.guids.alloc(ty);
        self.lists.alloc_unit(ty, guid, allied)
    }
}

#[cfg(test)]
mod alloc_tests {
    use super::*;
    use crate::tick::timer::TimerClass;

    #[test]
    fn timer_scheduled_before_the_unit_is_added() {
        // Missile setup: allocate, schedule the every-tick event, add.
        let mut g = Game::new();
        g.lists.ensure_act(0).unwrap();
        let room = g.lists.create_room(0).unwrap();
        g.lists.activate_room(room).unwrap();
        let m = g.alloc_unit(UnitType::Missile, false);
        let t = g.schedule_event(m, 0, -1, None, 0, 0).unwrap().unwrap();
        g.lists.add_allocated(m, Some(room)).unwrap();
        assert_eq!(g.lists.unit(m).unwrap().guid, 1);
        assert_eq!(g.timers.every_tick(TimerClass::Missile), [t]);
        assert_eq!(g.timers.unit_timers(m), [t]);
        assert_eq!(g.lists.room_units(room), [m]);
    }
}

#[cfg(test)]
mod tick_rate_tests {
    use super::*;

    #[test]
    fn durations_are_base_ticks_at_any_rate() {
        let mut g = Game::new();
        g.frame = 1000;
        // The base rate: a duration is its own tick count.
        assert_eq!((g.ticks(25), g.after(25), g.after(0)), (25, 1025, 1000));
        // 60 Hz: 25 base ticks (1 s) are 60 ticks; 5 (200 ms) are 12.
        g.tick_rate = TickRate::new(60).unwrap();
        assert_eq!((g.ticks(25), g.ticks(5), g.after(25)), (60, 12, 1060));
        // A timer of that duration fires one real second later.
        let u = g.alloc_unit(UnitType::Monster, false);
        let at = g.after(25);
        let t = g.schedule_event(u, 2, at, None, 0, 0).unwrap().unwrap();
        assert_eq!(g.timers.expire(t), Some(1060));
    }
}

#[cfg(test)]
mod node_tests {
    use super::*;

    #[test]
    fn the_first_eight_player_lists_keep_codes_0_to_7() {
        for l in 0..8 {
            assert_eq!(node::player(l), l);
            assert_eq!(node::player_list(l), Some(l));
        }
        for code in [node::GOOD_NPC, node::BONE_WALLS, 10, node::NONE] {
            assert_eq!(node::player_list(code), None, "code {code}");
        }
    }

    #[test]
    fn player_lists_past_the_eighth_round_trip_up_to_64() {
        for l in 8..64 {
            let code = node::player(l);
            assert!(code >= node::MORE_PLAYERS);
            assert!(u8::try_from(code).is_ok(), "a node index fits a byte");
            assert_eq!(node::player_list(code), Some(l));
        }
    }

    #[test]
    fn nodes_join_any_player_list_and_leave_it() {
        let mut n = TargetNodes::default();
        let (a, b, c) = (UnitId(1), UnitId(2), UnitId(3));
        let code_40 = node::player(40) as i32;
        n.push_front(code_40, a);
        n.push_front(code_40, b);
        n.push_front(node::GOOD_NPC as i32, c);
        n.push_front(node::NONE as i32, c);
        assert_eq!(n.slot(code_40 as usize), &[b, a]);
        assert_eq!(n.lists().players.len(), 41);
        assert_eq!(n.slot(node::GOOD_NPC as usize), &[c]);
        n.remove(b);
        n.remove(c);
        assert_eq!(n.slot(code_40 as usize), &[a]);
        assert!(n.slot(node::GOOD_NPC as usize).is_empty());
    }

    #[test]
    fn lists_from_codes_keep_the_code_order() {
        let l = TargetLists::from_codes(&[
            vec![UnitId(1)],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![UnitId(8)],
            vec![UnitId(9)],
        ]);
        assert_eq!(l.players.len(), 8);
        assert_eq!(l.players[0], [UnitId(1)]);
        assert_eq!(l.good, [UnitId(8)]);
        assert_eq!(l.walls, [UnitId(9)]);
    }
}
