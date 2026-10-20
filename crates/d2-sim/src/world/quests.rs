//! The quest system: flag records, quest control and records, game
//! entry, event dispatch, the updater and its timers, status messages,
//! NPC dialog hooks, act transitions and portals, helpers. The quest
//! state machines are in [`act1`] (which also routes every quest
//! function), [`act2`], [`act3`], [`act4`] and [`act5`]; a mod game's
//! quest functions come from [`QuestProvider`].
//!
//! Units, items, NPC chat, monsters and levels belong to other modules
//! are reached through [`QuestWorld`]. Message bytes leave through
//! `QuestWorld::send`. Quest functions D2RUST does not have are
//! reported through `QuestWorld::unhandled` instead of being guessed.

pub mod act1;
pub mod act2;
pub mod act3;
pub mod act4;
pub mod act5;
pub mod helpers;
pub mod late;
pub mod placeholders;
pub mod provider;
pub mod tables;

#[cfg(test)]
mod act1_answers_tests;
#[cfg(test)]
mod act1_rest_misc_tests;
#[cfg(test)]
mod act1_rest_q4_tests;
#[cfg(test)]
mod act1_tests;
#[cfg(test)]
mod act3_tests;
#[cfg(test)]
mod gaps_tests;
#[cfg(test)]
mod mod_quest_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_lworld_world;

use std::collections::BTreeSet;

use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

pub use provider::QuestProvider;
pub use tables::{MessageEntry, QuestRow, QuestTables};

/// Bytes of a flag record.
pub const FLAG_BYTES: usize = 0x60;
/// Quest slots.
pub const SLOTS: u8 = 42;
/// Init-table rows and intro records.
pub const TABLE_ROWS: usize = 37;
pub const INTRO_ROWS: usize = 4;

/// Slot bits.
pub mod bit {
    pub const REWARD_GRANTED: u8 = 0;
    pub const REWARD_PENDING: u8 = 1;
    pub const STARTED: u8 = 2;
    pub const LEAVE_TOWN: u8 = 3;
    pub const ENTER_AREA: u8 = 4;
    pub const CUSTOM1: u8 = 5;
    pub const UPDATE_QUEST_LOG: u8 = 12;
    pub const PRIMARY_GOAL_DONE: u8 = 13;
    pub const COMPLETED_NOW: u8 = 14;
    pub const COMPLETED_BEFORE: u8 = 15;
}

/// Event ids.
pub mod event {
    pub const NPC_ACTIVATE: u8 = 0;
    pub const NPC_DEACTIVATE: u8 = 2;
    pub const CHANGED_LEVEL: u8 = 3;
    pub const ITEM_PICKED_UP: u8 = 4;
    pub const ITEM_DROPPED: u8 = 5;
    pub const EVENT6: u8 = 6;
    pub const MONSTER_KILLED: u8 = 8;
    pub const PLAYER_DROPPED_WITH_QUEST_ITEM: u8 = 9;
    pub const PLAYER_LEAVES_GAME: u8 = 10;
    pub const SCROLL_MESSAGE: u8 = 11;
    pub const PLAYER_STARTED_GAME: u8 = 13;
    pub const PLAYER_JOINED_GAME: u8 = 14;
}

/// NPC class ids used by the quest code.
pub mod npc {
    pub const AKARA: u16 = 148;
    pub const KASHYA: u16 = 150;
    pub const CHARSI: u16 = 154;
    pub const WARRIV1: u16 = 155;
    pub const MESHIF1: u16 = 210;
    pub const CAIN5: u16 = 265;
    pub const NAVI: u16 = 266;
    pub const TYRAEL2: u16 = 367;
}

/// Fatal asserts of the quest code, returned instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuestError {
    #[error("flag record copy of {0} bytes (want 0x60)")]
    Size(usize),
    #[error("quest set not picked")]
    NotPicked,
    #[error("no quest record for chain {0}")]
    NoRecord(u8),
    #[error("chain {0} has no sequence function")]
    NoSequenceFn(u8),
    #[error("timer created while the updater runs")]
    TimerWhileExecuting,
    #[error("default status rule on filter {0} > 40")]
    Filter(u8),
    #[error("quest table: {0}")]
    Table(#[from] crate::world::TsvError),
    #[error("quests.tsv: {0}")]
    TableShape(&'static str),
    /// A fatal assert inside a quest callback: the function's id.
    #[error("quest callback {0:#x}: fatal assert")]
    Fatal(u32),
}

// ------------------------------------------------------------------ flags

/// A quest flag record: 42 slots × 16 bits, LSB first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestFlags(pub [u8; FLAG_BYTES]);

impl Default for QuestFlags {
    /// Zeroed.
    fn default() -> Self {
        Self([0; FLAG_BYTES])
    }
}

impl QuestFlags {
    fn pos(q: u8, b: u8) -> (usize, u8) {
        let n = 16 * usize::from(q) + usize::from(b);
        (n >> 3, 1 << (n & 7))
    }

    /// Bit `b` of slot `q`.
    pub fn get(&self, q: u8, b: u8) -> bool {
        let (i, m) = Self::pos(q, b);
        self.0[i] & m != 0
    }

    /// Set bit `b` of slot `q`.
    pub fn set(&mut self, q: u8, b: u8) {
        let (i, m) = Self::pos(q, b);
        if self.0[i] & m == 0 {
            crate::cov!(Quest, q, b);
        }
        self.0[i] |= m;
    }

    /// Clear bit `b` of slot `q`.
    pub fn clear(&mut self, q: u8, b: u8) {
        let (i, m) = Self::pos(q, b);
        self.0[i] &= !m;
    }

    /// Clear bits 2..11 of slot q.
    pub fn reset_progress(&mut self, q: u8) {
        for b in 2..=11 {
            self.clear(q, b);
        }
    }

    /// Slot q's word (little-endian u16).
    pub fn word(&self, q: u8) -> u16 {
        let i = 2 * usize::from(q);
        u16::from_le_bytes([self.0[i], self.0[i + 1]])
    }

    /// Copy in. With `normalize`, for every slot 0..41
    /// clear bits 13 and 14, then set bit 15 if bit 1 is set.
    pub fn copy_in(buf: &[u8], normalize: bool) -> Result<Self, QuestError> {
        let bytes: [u8; FLAG_BYTES] = buf.try_into().map_err(|_| QuestError::Size(buf.len()))?;
        let mut r = Self(bytes);
        if normalize {
            for q in 0..SLOTS {
                r.clear(q, bit::PRIMARY_GOAL_DONE);
                r.clear(q, bit::COMPLETED_NOW);
                if r.get(q, bit::REWARD_PENDING) {
                    r.set(q, bit::COMPLETED_BEFORE);
                }
            }
        }
        Ok(r)
    }

    /// Copy out.
    pub fn copy_out(&self) -> [u8; FLAG_BYTES] {
        self.0
    }
}

/// A player's quest state (in the player data).
/// Owned by the player unit (provider: the units group).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerQuests {
    /// Flag record per difficulty.
    pub flags: [QuestFlags; 3],
    /// NPC intro record per difficulty: NPC class ids introduced (the
    /// save's field B).
    pub intro: [BTreeSet<u16>; 3],
    /// The first-talk bits per difficulty (the save's field A): set by
    /// [`PlayerQuests::hear`], tested by [`PlayerQuests::heard`]; bit by
    /// NPC class through [`d2_formats::d2s::npc_bit`].
    pub first_talk: [[u8; 8]; 3],
}

impl PlayerQuests {
    /// The first-talk bit of NPC `class` in difficulty `d`.
    pub fn heard(&self, d: usize, class: u16) -> bool {
        let n = d2_formats::d2s::npc_bit(i32::from(class));
        self.first_talk
            .get(d)
            .is_some_and(|f| f[usize::from(n >> 3)] & (1 << (n & 7)) != 0)
    }

    /// Sets the first-talk bit of NPC `class` in difficulty
    /// `d` (the first matching pair's bit, else bit 0).
    pub fn hear(&mut self, d: usize, class: u16) {
        let n = d2_formats::d2s::npc_bit(i32::from(class));
        if let Some(f) = self.first_talk.get_mut(d) {
            f[usize::from(n >> 3)] |= 1 << (n & 7);
        }
    }

    /// The save writer's field B of difficulty `d`: the
    /// bit of every introduced class.
    pub fn intro_bits(&self, d: usize) -> [u8; 8] {
        let mut f = [0u8; 8];
        for &c in self.intro.get(d).into_iter().flatten() {
            let n = d2_formats::d2s::npc_bit(i32::from(c));
            f[usize::from(n >> 3)] |= 1 << (n & 7);
        }
        f
    }

    /// The save reader's field B of difficulty `d`: every
    /// class of the act lists ([`INTRO_NPCS`], the only classes the
    /// record is set for) whose bit is set is introduced.
    pub fn set_intro_bits(&mut self, d: usize, bits: [u8; 8]) {
        let Some(set) = self.intro.get_mut(d) else {
            return;
        };
        set.clear();
        for &c in INTRO_NPCS.iter().copied().flatten() {
            let n = d2_formats::d2s::npc_bit(i32::from(c));
            if bits[usize::from(n >> 3)] & (1 << (n & 7)) != 0 {
                set.insert(c);
            }
        }
    }
}

// ------------------------------------------------------------------ records

/// A unit's quest chain: chain ids, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestChain(pub Vec<u8>);

/// A list of up to 32 player GUIDs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuidList(pub Vec<u32>);

impl GuidList {
    /// Add; ignores duplicates and a full list.
    pub fn add(&mut self, guid: u32) {
        if self.0.len() < 32 && !self.0.contains(&guid) {
            self.0.push(guid);
        }
    }

    /// Remove by swapping with the last.
    pub fn remove(&mut self, guid: u32) {
        if let Some(i) = self.0.iter().position(|&g| g == guid) {
            self.0.swap_remove(i);
        }
    }

    /// Whether `guid` is in the list.
    pub fn contains(&self, guid: u32) -> bool {
        self.0.contains(&guid)
    }
}

/// A quest record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestRecord {
    pub chain: u8,
    pub act: u8,
    /// Not an intro record.
    pub not_intro: bool,
    /// Active.
    pub active: bool,
    /// Status shown to clients.
    pub status: u8,
    /// Quest state.
    pub state: u8,
    /// Init number.
    pub init_no: u8,
    /// Sequence target chain.
    pub seq_id: Option<u8>,
    /// Flags (low byte sent in 0x5D).
    pub flags: u8,
    /// Filter: the record's flag slot.
    pub filter: u8,
    /// Second flag column.
    pub flag2: Option<u8>,
    /// Player GUIDs.
    pub guids: GuidList,
    /// Bit per event id with a callback.
    pub callbacks: u16,
    pub status_fn: Option<u32>,
    pub active_fn: Option<u32>,
    pub seq_fn: Option<u32>,
    /// NPC message table.
    pub msgs: Option<u32>,
    /// Per-quest extra data.
    pub extra: act1::Extra,
}

impl QuestRecord {
    pub fn has_callback(&self, ev: u8) -> bool {
        self.callbacks & (1 << ev) != 0
    }

    pub fn clear_callback(&mut self, ev: u8) {
        self.callbacks &= !(1 << ev);
    }
}

/// What a quest timer runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerFn {
    /// Den of Evil status 5 while state 4; returns 1.
    DenOfEvilStatus,
    /// Burial Grounds status 3; returns 1.
    BurialStatus,
    /// The Tristram Cain removal walk; returns 1.
    CainRemoval,
    /// The Tristram portal at the class-17 stone; kept
    /// until created or the stone is gone.
    TristramPortal,
    /// Forgotten Tower status 13; returns 1.
    TowerStatus,
    /// Andariel's portals and status 3.
    AndarielPortals,
    /// Chain 6's sequence timer, state 0 → 1.
    SlaughterOpen,
    /// An Act II timer.
    Act2(act2::Timer),
    /// An Act III timer.
    Act3(act3::Timer),
    /// Act IV timers.
    Act4(act4::Timer),
    /// Act V timers.
    Act5(act5::Timer),
    /// Test probe: logs through `unhandled(chain, tick)`, never removed.
    #[cfg(test)]
    Probe,
}

/// A quest timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestTimer {
    pub func: TimerFn,
    pub chain: u8,
    pub due: u32,
    pub period: u32,
}

/// The quest control (one per game).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestControl {
    /// Records in list order: newest first.
    pub records: Vec<QuestRecord>,
    pub executing: bool,
    pub picked: bool,
    /// Game quest flag record (never saved).
    pub game: QuestFlags,
    /// Timer list, head first.
    pub timers: Vec<QuestTimer>,
    pub tick: u32,
    pub seed: Seed,
    /// FX byte for 0x89.
    pub fx: u8,
    /// Init-table rows (for game entry and 0x5E).
    pub rows: Vec<QuestRow>,
    /// NPC message tables.
    pub messages: Vec<MessageEntry>,
    /// Fatal asserts the callbacks reached ([`QuestError::Fatal`]), in
    /// order; the quest code would abort at the first.
    pub faults: Vec<QuestError>,
    /// Host calls the quest rules raised, in call order: the host
    /// (`d2-server`) runs them after the current frame's quest step
    /// ([`Self::take_host_requests`]).
    pub host_requests: Vec<HostRequest>,
}

/// A host call raised by a quest rule: `d2-sim`
/// changes no game state for it; the host runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRequest {
    /// End the game (drop the last in-game client that passes the
    /// client check, else the first in-game client).
    EndGame,
    /// The host save pass.
    SavePass,
}

/// The seam to the rest of the game. Expected providers in brackets.
pub trait QuestWorld {
    // Game.
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// The rules the game runs under; a mod game's quest functions come
    /// from them ([`QuestProvider`]). Default: D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }
    /// The game's difficulty.
    fn difficulty(&self) -> u8;
    /// The game is an expansion game.
    fn expansion(&self) -> bool;
    /// The game type.
    fn game_type(&self) -> u8;
    /// The game has an Act II (DRLG).
    fn has_act2(&self) -> bool;

    // Players and units (units group).
    /// Every player in the game's walk order (hash buckets 0–127, each
    /// from its head; players with state 7 skipped).
    fn players(&self) -> Vec<UnitId>;
    /// The first client's player.
    fn first_client_player(&self) -> Option<UnitId>;
    fn guid(&self, unit: UnitId) -> u32;
    fn player_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// The player's quest records (player data; `None`: missing).
    fn quests(&mut self, player: UnitId) -> Option<&mut PlayerQuests>;
    /// Act of the unit's room's level (`None`: no room).
    fn unit_act(&self, unit: UnitId) -> Option<u8>;
    /// Level id of the unit's room.
    fn unit_level(&self, unit: UnitId) -> Option<u32>;
    fn player_class(&self, player: UnitId) -> u8;
    /// The unit seed.
    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed;
    fn stat(&self, unit: UnitId, stat: u16) -> i32;
    /// The unit's base stat, layer 0 (A1Q3's level test).
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32;
    /// Add to a stat.
    fn add_stat(&mut self, unit: UnitId, stat: u16, delta: i32);
    /// Attach a sound to the player.
    fn attach_sound(&mut self, player: UnitId, sound: u16);
    /// A player data byte the quests keep.
    fn player_byte_4c(&self, player: UnitId) -> u8;
    fn set_player_byte_4c(&mut self, player: UnitId, v: u8);
    /// The unit's quest chain.
    fn quest_chain(&mut self, unit: UnitId) -> Option<&mut QuestChain>;
    /// What a killer or victim is.
    fn unit_kind(&self, unit: UnitId) -> UnitKind;
    /// The monster with this GUID and its class (NPC lookup).
    fn monster_by_guid(&self, guid: u32) -> Option<(UnitId, u16)>;
    /// A monster unit's class id (NPC class), if it is a monster.
    fn monster_class(&self, unit: UnitId) -> Option<u16>;
    /// The players P with a room for which A1Q2's J3 test holds
    /// P's room is the unit's room, or the unit's room is in
    /// P's room's room list. Empty
    /// when the unit has no room.
    fn players_near(&self, unit: UnitId) -> Vec<UnitId>;
    /// The members of the player's party in list order, GUID lookups
    /// that fail skipped; `None` when the party id is 0xFFFF (no party;
    /// a single player is in none).
    fn party_members(&self, player: UnitId) -> Option<Vec<UnitId>>;

    // Messages (server transport).
    fn send(&mut self, player: UnitId, msg: &[u8]);
    /// The 0x27 text list.
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, list: &[(u16, u32)]);

    // Items (items / inventory).
    fn has_item(&self, player: UnitId, code: [u8; 4]) -> bool;
    /// An item unit's code (`None`: not an item).
    fn item_code(&self, item: UnitId) -> Option<[u8; 4]>;
    /// The cursor item check of the C→S 0x44 gate: `item` is the
    /// player's cursor item. Default (no inventory model): true.
    fn is_cursor_item(&self, player: UnitId, item: UnitId) -> bool {
        let _ = (player, item);
        true
    }
    /// Delete the player's item with `code`.
    fn delete_item(&mut self, player: UnitId, code: [u8; 4]);
    /// Create, place or drop a reward item.
    fn reward_item(
        &mut self,
        player: UnitId,
        code: [u8; 4],
        level: i32,
        quality: u8,
        droppable: bool,
    ) -> Option<UnitId>;
    /// Drop an item of `code` at a unit (normal quality 2).
    fn drop_item_at(&mut self, unit: UnitId, code: [u8; 4], quality: u8) -> bool;
    /// Player inventory items with their items record `quest` byte ≠ 0,
    /// in inventory order.
    fn quest_items(&self, player: UnitId) -> Vec<(UnitId, u8)>;

    // Levels and objects (DRLG / objects).
    /// Level 8's monster region: (spawn count, kill count, room count,
    /// populated rooms).
    fn den_region(&self) -> (u32, u32, u32, u32);
    /// Level id of the true tomb.
    fn true_tomb_level(&self) -> u32;
    /// Free spot near the player (size, mask, radius, limit).
    fn free_spot(
        &mut self,
        player: UnitId,
        size: u32,
        mask: u32,
        radius: u32,
        limit: u32,
    ) -> Option<(i32, i32)>;
    /// A portal object of `class` to `level` at (x, y).
    fn create_portal(&mut self, player: UnitId, x: i32, y: i32, class: u16, level: u32) -> bool;
    /// Tyrael's portal: the same with chain 13's tomb-portal flag set
    /// during the call, so the arrival hook places the far end.
    /// Default: [`QuestWorld::create_portal`].
    fn create_tyrael_portal(
        &mut self,
        player: UnitId,
        x: i32,
        y: i32,
        class: u16,
        level: u32,
    ) -> bool {
        self.create_portal(player, x, y, class, level)
    }
    /// Schedule object timer event 7 (QUESTFN) at `frame` (tick).
    fn schedule_quest_event(&mut self, object: UnitId, frame: i32);
    /// An object's mode; 0 when there is no object.
    fn object_mode(&self, object: UnitId) -> i32;
    /// Set an object's mode (objects).
    fn set_object_mode(&mut self, object: UnitId, mode: i32);
    /// The object with this GUID and its class.
    fn object_by_guid(&self, guid: u32) -> Option<(UnitId, u16)>;
    /// The mercenary reward of `npc`.
    fn mercenary_reward(&mut self, player: UnitId, npc: u16);

    // Act I quest seams (paths, rooms, monsters, objects).
    /// The unit's position and room (`None`: no room).
    fn unit_position(&self, unit: UnitId) -> Option<(i32, i32, RoomId)>;
    /// The object whose quest init function runs now and its init point
    /// (room, x, y: where it was created), `None` after it. A host whose
    /// objects are linked into their room only after the init answers
    /// [`QuestWorld::unit_position`] for it from this point. Default:
    /// ignored.
    fn set_init_point(&mut self, point: Option<(UnitId, i32, i32, RoomId)>) {
        let _ = point;
    }
    /// The unit's position alone (an object's static path), which an
    /// object keeps when its room is freed. `None`: no unit. Default:
    /// [`Self::unit_position`]'s.
    fn unit_xy(&self, unit: UnitId) -> Option<(i32, i32)> {
        self.unit_position(unit).map(|(x, y, _)| (x, y))
    }
    /// (x, y) inside the room's tile rectangle, the last
    /// row and column excluded.
    fn room_contains(&self, room: RoomId, x: i32, y: i32) -> bool;
    /// The room holding (x, y), searched from `room`.
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId>;
    /// A free spot from a point (size, mask, radius, limit).
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
    ) -> Option<(i32, i32, RoomId)>;
    /// Spawn a monster of `class` at (x, y) in `room`.
    #[allow(clippy::too_many_arguments)]
    fn spawn_monster(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: u8,
        r: u32,
    ) -> Option<UnitId>;
    /// Set unit flags `flags`.
    fn or_unit_flags(&mut self, unit: UnitId, flags: u32);
    /// Every monster, in the game's walk order.
    fn monsters(&self) -> Vec<UnitId>;
    /// The players chatting with `npc`;
    /// `None` when no chat is open with it.
    fn npc_chat_clients(&self, npc: UnitId) -> Option<Vec<UnitId>>;
    /// Request the monster's removal mode.
    fn remove_monster(&mut self, monster: UnitId);
    /// Drop act `act`'s stored preset
    /// of monster `class`.
    fn drop_preset_monster(&mut self, act: u8, class: u16);
    /// The first object of `class` in the rooms of `object`'s room list
    /// (the Cairn stone operate).
    fn find_object_near(&self, object: UnitId, class: u16) -> Option<UnitId>;
    /// An object of `class` at (x, y),
    /// its room refreshed.
    fn create_object(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId>;
    /// The object's animation length (>> 8 gives frames).
    fn object_anim_length(&self, object: UnitId) -> i32;
    /// Schedule object event `ev` at `frame`.
    fn schedule_object_event(&mut self, object: UnitId, ev: u8, frame: i32);
    /// Open quest message `msg` for the player at the object.
    fn open_quest_message(&mut self, player: UnitId, object: UnitId, msg: u16);

    // Act I remainder seams.
    /// Spawn a monster with spawn flags (mode, spread, flags).
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
    ) -> Option<UnitId>;
    /// A portal object of `class` to `level`; `exact` = at (x, y) only,
    /// else a free spot is searched.
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
    ) -> Option<UnitId>;
    /// Create a missile.
    #[allow(clippy::too_many_arguments)]
    fn create_missile(
        &mut self,
        owner: UnitId,
        skill: u16,
        level: u8,
        class: u16,
        x: i32,
        y: i32,
    ) -> Option<UnitId>;
    /// Two missile data fields.
    fn set_missile_target(&mut self, missile: UnitId, a: u32, b: u32);
    /// Refresh the unit's room.
    fn refresh_room(&mut self, unit: UnitId);
    /// Allocate an object of
    /// `class` at (x, y) in `room` with `mode`.
    fn spawn_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        mode: i32,
    ) -> Option<UnitId>;
    /// The save flags of the player's client; `None`: no client.
    fn client_save_flags(&self, player: UnitId) -> Option<u16>;
    fn set_client_save_flags(&mut self, player: UnitId, flags: u16);

    // Act III quest seams (the Act II seams below are
    // shared with it). The default bodies report the function through
    // `unhandled` (chain 0xFF) and do nothing, until a host provides them.
    /// The game has an Act III (DRLG).
    fn has_act3(&mut self) -> bool {
        self.unhandled(0xFF, 0x005B_9A30);
        false
    }
    /// Spawn a monster of `class` in `room` (mode 1).
    fn spawn_monster_in_room(&mut self, room: RoomId, class: u16) -> Option<UnitId> {
        let _ = (room, class);
        self.unhandled(0xFF, 0x005A_43E0);
        None
    }
    /// Spawn a monster at a unit's room and position: spread −1
    /// through [`Self::spawn_monster_flags`]. A unit without a room:
    /// nothing.
    ///
    /// The flags argument is 0 (the orb's call passes flags 0, mode 1,
    /// class 366).
    fn spawn_monster_at_unit(&mut self, unit: UnitId, class: u16, mode: u8) -> Option<UnitId> {
        let (x, y, room) = self.unit_position(unit)?;
        self.spawn_monster_flags(room, x, y, class, mode, -1, 0)
    }
    /// The AI mode request at a point (the orb kill passes mode 0 at
    /// (0, 0), then runs [`helpers::orb_missile`]).
    fn monster_mode_at(&mut self, monster: UnitId, mode: u8, x: i32, y: i32) {
        let _ = (monster, mode, x, y);
        self.unhandled(0xFF, 0x005D_DFC0);
    }
    /// The room covering (x, y).
    fn room_covering(&mut self, x: i32, y: i32) -> Option<RoomId> {
        let _ = (x, y);
        self.unhandled(0xFF, 0x0061_9DA0);
        None
    }
    /// A unit of type 0 (player) in `room` or a room of its room list
    /// scanned in list order.
    fn player_in_rooms(&mut self, room: RoomId) -> bool {
        let _ = room;
        self.unhandled(0xFF, 0x0061_9790);
        false
    }
    /// The stairs' warp: every
    /// type-5 warp tile of the object's room warps the
    /// player. The same function as [`Self::object_stairs_warp`].
    fn stairs_warp(&mut self, object: UnitId, player: UnitId) {
        self.object_stairs_warp(player, object);
    }
    /// The first player in the object's room unit list closer than
    /// `dist`.
    fn player_near_object(&mut self, object: UnitId, dist: i32) -> Option<UnitId> {
        let _ = (object, dist);
        self.unhandled(0xFF, 0x0064_1530);
        None
    }
    /// The unique / champion / boss tests of a victim.
    fn special_monster(&mut self, victim: UnitId) -> bool {
        let _ = victim;
        self.unhandled(0xFF, 0x005A_0180);
        false
    }
    /// Collision at (x, y) in `room` under `mask`: nonzero = blocked.
    fn blocked(&mut self, room: RoomId, x: i32, y: i32, mask: u32) -> bool {
        let _ = (room, x, y, mask);
        self.unhandled(0xFF, 0x0062_29F0);
        false
    }
    /// The code of the player's weapon.
    fn weapon_code(&mut self, player: UnitId) -> Option<[u8; 4]> {
        let _ = player;
        self.unhandled(0xFF, 0x0063_BEF0);
        None
    }

    /// A quest function D2RUST does not have was reached; the
    /// host logs it.
    fn unhandled(&mut self, chain: u8, function: u32);

    // Act II quest seams. Each default reports
    // the function through `unhandled` (chain 0xFF) and returns
    // the neutral value, so hosts that do not provide one yet still build.

    /// The player's client is in act `act` (0-based).
    fn client_in_act(&mut self, player: UnitId, act: u8) -> bool {
        let _ = (player, act);
        self.unhandled(0xFF, 0x0053_82B0);
        false
    }
    /// Start the Tainted Sun on act `act` (environment).
    fn start_tainted_sun(&mut self, act: u8) {
        let _ = act;
        self.unhandled(0xFF, 0x0061_C450);
    }
    /// End the Tainted Sun on Act II (environment).
    fn end_tainted_sun(&mut self) {
        self.unhandled(0xFF, 0x0061_C4D0);
    }
    /// The shared quest-chest gate ([`helpers::quest_chest_gate`]).
    fn quest_chest_gate(&mut self, object: UnitId, player: UnitId) -> bool {
        let _ = player;
        helpers::quest_chest_gate(self, object)
    }
    /// Set the object's (or monster's) drop code to `code`, then drop one
    /// item of `quality`. `level`: the value in the level variable
    /// (`None`: not known). Returns the item.
    fn quest_drop(
        &mut self,
        unit: UnitId,
        code: [u8; 4],
        quality: u8,
        level: Option<i32>,
        droppable: bool,
    ) -> Option<UnitId> {
        let _ = (unit, code, quality, level, droppable);
        self.unhandled(0xFF, 0x0055_9A30);
        None
    }
    /// Mark an item identified.
    fn identify_item(&mut self, item: UnitId) {
        let _ = item;
        self.unhandled(0xFF, 0x0062_80D0);
    }
    /// The chest's own treasure with the operate record: the object
    /// and its operator (the
    /// recipient `R`).
    fn object_treasure(&mut self, object: UnitId, operator: UnitId, kind: u8) {
        let _ = (object, operator, kind);
        self.unhandled(0xFF, 0x0058_5B90);
    }
    /// One normal gold pile at the object.
    fn drop_gold(&mut self, object: UnitId) {
        let _ = object;
        self.unhandled(0xFF, 0x0058_5970);
    }
    /// Set or clear a room's has-portal flag.
    fn set_room_portal(&mut self, room: RoomId, on: bool) {
        let _ = (room, on);
        self.unhandled(0xFF, 0x0061_AED0);
    }
    /// An object at (x, y) in `room` with flags 1
    /// (the Act II call form; `spawn_object` is the Act I form with a mode).
    fn spawn_quest_object(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        let _ = (room, x, y, class);
        self.unhandled(0xFF, 0x0055_5230);
        None
    }
    /// Free an object's collision.
    fn free_object_collision(&mut self, object: UnitId) {
        let _ = object;
        self.unhandled(0xFF, 0x0062_3830);
    }
    /// The player is busy.
    fn player_busy(&mut self, player: UnitId) -> bool {
        let _ = player;
        self.unhandled(0xFF, 0x0053_5060);
        false
    }
    /// The player's interact unit (type, GUID), if any.
    fn interact_unit(&mut self, player: UnitId) -> Option<(u8, u32)> {
        let _ = player;
        self.unhandled(0xFF, 0x0055_4120);
        None
    }
    /// Set (`Some`) or clear (`None`) the player's interact unit.
    fn set_interact_unit(&mut self, player: UnitId, unit: Option<(u8, u32)>) {
        let _ = (player, unit);
        self.unhandled(0xFF, 0x0055_4120);
    }
    /// S→C 0x58, the orifice insert dialog.
    fn open_insert_dialog(&mut self, player: UnitId, object: UnitId) {
        let _ = (player, object);
        self.unhandled(0xFF, 0x0053_D8D0);
    }
    /// `Range` of `missiles.txt` row `row`; `None` when the table has
    /// no such row.
    fn missile_range(&mut self, row: u32) -> Option<i32> {
        let _ = row;
        self.unhandled(0xFF, 0x0059_DD80);
        None
    }
    /// The player is trading.
    fn is_trading(&mut self, player: UnitId) -> bool {
        let _ = player;
        self.unhandled(0xFF, 0x0056_78A0);
        false
    }
    /// Remove a unit.
    fn remove_unit(&mut self, unit: UnitId) {
        let _ = unit;
        self.unhandled(0xFF, 0x0052_E050);
    }
    /// End the interaction with the NPC when it has an interact unit:
    /// true when it did.
    fn npc_hold_chat(&mut self, npc: UnitId) -> bool {
        let _ = npc;
        self.unhandled(0xFF, 0x0057_2DC0);
        false
    }
    /// A spawn location of `kind` in `level`.
    fn spawn_location(&mut self, act: u8, level: u32, kind: u8) -> Option<(i32, i32, RoomId)> {
        let _ = (act, level, kind);
        self.unhandled(0xFF, 0x0061_B060);
        None
    }
    /// A free spot from a point (size, mask, radius).
    fn free_spot_near(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        size: u32,
        mask: u32,
        radius: u32,
    ) -> Option<(i32, i32, RoomId)> {
        let _ = (room, x, y, size, mask, radius);
        self.unhandled(0xFF, 0x0064_E7E0);
        None
    }
    /// The player heard NPC `class`'s first-talk text
    /// (chain 38; [`PlayerQuests::heard`], field A).
    fn npc_intro_heard(&mut self, player: UnitId, class: u16) -> bool {
        let d = usize::from(self.difficulty());
        self.quests(player).is_some_and(|q| q.heard(d, class))
    }
    /// Set NPC `class`'s first-talk bit (chain 38;
    /// [`PlayerQuests::hear`]).
    fn set_npc_intro(&mut self, player: UnitId, class: u16) {
        let d = usize::from(self.difficulty());
        if let Some(q) = self.quests(player) {
            q.hear(d, class);
        }
    }
    /// Set the object's drop code to `code`: the item drop reads it on
    /// every call and never clears it. The Act II chests store it once,
    /// before they count the players.
    fn set_drop_code(&mut self, object: UnitId, code: [u8; 4]) {
        let _ = (object, code);
        self.unhandled(0xFF, 0x0059_9C28);
    }

    // Act IV / V seams. Each has a default that reports the
    // function through `unhandled` (chain 0xFE) so hosts that do not
    // provide it yet keep building.

    // -- Act IV, Fallen Angel / Hell's Forge / gossip.

    /// The room covering (x, y) on the DRLG of act `act` (0-based), if
    /// any (the spawn room of Izual's ghost).
    fn room_in_act_at(&mut self, act: u8, x: i32, y: i32) -> Option<RoomId> {
        let _ = (act, x, y);
        self.unhandled(0xFE, 0x0061_9DA0);
        None
    }

    /// The distance between two units (`None`: not known; Izual's ghost,
    /// the Act V barbarians).
    fn distance_between(&mut self, a: UnitId, b: UnitId) -> Option<i32> {
        let _ = (a, b);
        self.unhandled(0xFE, 0x0064_16D0);
        None
    }

    /// The code of the
    /// weapon the player wields; `None` without an inventory or a weapon
    /// (the Hellforge's hammer test).
    fn wielded_weapon_code(&mut self, player: UnitId) -> Option<[u8; 4]> {
        let _ = player;
        self.unhandled(0xFE, 0x0063_BEF0);
        None
    }
    // -- end Act IV q1/q3 seams.

    // -- Act IV, Terror's End.
    /// The object's `objects.txt` `FrameCnt1` column value, read by the
    /// seal activation for its
    /// end-animation event at f + 2·fc1.
    fn object_frame_count1(&mut self, object: UnitId) -> i32 {
        let _ = object;
        self.unhandled(0xFE, 0x005B_5630);
        0
    }
    /// The u16 entry `n` of the data tables' superunique-boss list
    /// (entries 36–38 = the seal bosses).
    fn superunique_id(&mut self, n: u8) -> u16 {
        let _ = n;
        self.unhandled(0xFE, 0x0058_6B30);
        0
    }
    /// Spawn superunique `id` at (x, y) beside the dummy object (`arg` 2
    /// here; [`helpers::spawn_superunique`]).
    fn spawn_superunique(
        &mut self,
        dummy: UnitId,
        x: i32,
        y: i32,
        arg: u32,
        id: u16,
    ) -> Option<UnitId> {
        helpers::spawn_superunique(self, dummy, x, y, arg, id)
    }
    /// The units of type 1 (monsters) of each active room of the Act IV
    /// DRLG whose level is
    /// `level`, in room-list then unit-list order (the next unit is
    /// read before the unit is handled).
    fn level_monsters(&mut self, level: u32) -> Vec<UnitId> {
        let _ = level;
        self.unhandled(0xFE, 0x0061_A180);
        Vec::new()
    }
    /// The unit is dead (≠ 0).
    fn unit_dead(&mut self, unit: UnitId) -> bool {
        let _ = unit;
        self.unhandled(0xFE, 0x0055_41B0);
        false
    }
    /// The unit's alignment (0 = evil).
    fn alignment(&mut self, unit: UnitId) -> u32 {
        let _ = unit;
        self.unhandled(0xFE, 0x0062_59B0);
        0
    }
    /// End the player's interaction (classic end of game;
    /// [`helpers::end_interaction`]).
    fn end_interaction(&mut self, player: UnitId) {
        helpers::end_interaction(self, player);
    }
    /// Level warp of the player to `level`.
    fn warp_to_level(&mut self, player: UnitId, level: u32, arg: u32) {
        let _ = (player, level, arg);
        self.unhandled(0xFE, 0x0053_AEC0);
    }
    /// The player's client exists and is not busy.
    fn client_idle(&mut self, player: UnitId) -> bool {
        let _ = player;
        self.unhandled(0xFE, 0x0053_5060);
        false
    }
    /// Act change to `level`.
    fn act_change(&mut self, player: UnitId, level: u32, arg: u32) {
        let _ = (player, level, arg);
        self.unhandled(0xFE, 0x0054_B830);
    }
    /// Activate the level's waypoint in the player's waypoint record
    /// of `difficulty`.
    fn activate_waypoint(&mut self, player: UnitId, level: u32, difficulty: u8) {
        let _ = (player, level, difficulty);
        self.unhandled(0xFE, 0x005B_4FF0);
    }
    // -- end Act IV q2 seams.

    // -- Act V part 1.
    /// An object
    /// of `class` at (x, y) in `room` with the three flag arguments
    /// ("flags 1, 1, 0"). `None`: not created.
    fn place_object(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
        flags: [u8; 3],
    ) -> Option<UnitId> {
        let _ = (room, x, y, class, flags);
        self.unhandled(0xFE, 0x0055_5230);
        None
    }
    /// The units of every room in `room`'s adjacent-room list
    /// (the room itself included as the list holds it),
    /// room by room in list order, each room's unit list in order.
    fn adjacent_units(&mut self, room: RoomId) -> Vec<UnitId> {
        let _ = room;
        self.unhandled(0xFE, 0x0061_9790);
        Vec::new()
    }
    /// A unit's mode (monsters and players); 0 when there is no unit.
    fn unit_mode(&mut self, unit: UnitId) -> i32 {
        let _ = unit;
        self.unhandled(0xFE, 0x0058_8040);
        0
    }
    /// "Critical spawn" ([`helpers::critical_spawn`]).
    fn critical_spawn(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        helpers::critical_spawn(self, room, x, y, class)
    }
    /// "Kill in place": the unit's interaction
    /// is ended, it is put in mode 12, then leaves its room and is kept
    /// (not freed).
    fn kill_in_place(&mut self, unit: UnitId) {
        let _ = unit;
        self.unhandled(0xFE, 0x005A_7E60);
    }
    /// Apply a stored NPC map AI (the
    /// map-AI record `map_ai`, a handle the map-AI store passed) to the
    /// unit. False (nothing applied) when the record holds no AI.
    fn apply_map_ai(&mut self, unit: UnitId, map_ai: u32) -> bool {
        let _ = (unit, map_ai);
        self.unhandled(0xFE, 0x0058_F000);
        false
    }
    /// Town cleanup: Anya's interaction ends, she
    /// takes mode 12 and leaves her room, kept (the "kill in place"
    /// steps).
    fn npc_leave_town(&mut self, unit: UnitId) {
        let _ = unit;
        self.unhandled(0xFE, 0x0058_93E0);
    }
    /// Nihlathak killed in town (interaction ended, path
    /// freed, AI event 2 deleted, stat 6 := 0, mode 12, refresh, unit
    /// flags |= 1).
    fn kill_in_town(&mut self, unit: UnitId) {
        let _ = unit;
        self.unhandled(0xFE, 0x0058_9340);
    }
    /// Thaw step 1: the frozen object leaves its
    /// room and is kept.
    fn object_leave_room(&mut self, object: UnitId) {
        let _ = object;
        self.unhandled(0xFE, 0x0058_AAB0);
    }
    /// The item level of Anya's rare item.
    fn quest_item_level(&mut self, player: UnitId) -> i32 {
        let _ = player;
        self.unhandled(0xFE, 0x0055_8200);
        0
    }
    /// The item's `items.txt` drop sound.
    fn item_drop_sound(&mut self, item: UnitId) -> i32 {
        let _ = item;
        self.unhandled(0xFE, 0x0058_9580);
        0
    }
    /// A new stat list on
    /// the player with stats 39, 41, 43, 45 := `v`, the four sent
    /// (lists stack).
    fn add_resist_list(&mut self, player: UnitId, v: i32) {
        let _ = (player, v);
        self.unhandled(0xFE, 0x0062_51F0);
    }
    /// Preset spawn of superunique `superunique`'s monster
    /// at (x, y) in `room`.
    fn preset_superunique_spawn(
        &mut self,
        room: RoomId,
        x: i32,
        y: i32,
        superunique: u16,
    ) -> Option<UnitId> {
        let _ = (room, x, y, superunique);
        self.unhandled(0xFE, 0x0054_E600);
        None
    }
    // -- end Act V part 1 seams.

    // -- Act V part 2.
    /// The player's waypoint of `level` is active.
    fn waypoint_active(&mut self, player: UnitId, level: u32) -> bool {
        let _ = (player, level);
        self.unhandled(0xFE, 0x0066_0E50);
        false
    }
    /// Spawn the superunique at the unit's own position.
    /// The same function as `spawn_superunique` with an explicit spot.
    fn spawn_superunique_at_unit(&mut self, at: UnitId, superunique: u16) -> Option<UnitId> {
        let (x, y) = self.unit_xy(at)?;
        helpers::spawn_superunique(self, at, x, y, 2, superunique)
    }
    /// Missile `missile` from `from` towards the statue
    /// `to` (flags 0x420, level 1;
    /// [`helpers::quest_missile`]).
    fn quest_missile(&mut self, from: UnitId, to: UnitId, missile: u16, flags: u32, level: u8) {
        helpers::quest_missile(self, from, to, missile, flags, level);
    }
    /// Remove a spawned Ancient (unit state 54 →
    /// removed; in a room → mode 12, out of the room and kept,
    /// collision freed).
    fn remove_ancient(&mut self, monster: UnitId) {
        let _ = monster;
        self.unhandled(0xFE, 0x0058_BEC0);
    }
    /// The player class's maximum level.
    fn max_level(&mut self, player: UnitId) -> i32 {
        let _ = player;
        self.unhandled(0xFE, 0x0061_1830);
        0
    }
    /// The experience threshold T(level) of the player's
    /// class.
    fn experience_threshold(&mut self, player: UnitId, level: i32) -> u32 {
        let _ = (player, level);
        self.unhandled(0xFE, 0x0061_1800);
        0
    }
    /// Level up.
    fn level_up(&mut self, player: UnitId) {
        let _ = player;
        self.unhandled(0xFE, 0x0057_0880);
    }
    /// The stairs' warp of `object` for the player: each type-5 unit
    /// (warp tile) of the object's room unit list warps the player.
    fn object_stairs_warp(&mut self, player: UnitId, object: UnitId) {
        helpers::stairs_warp(self, player, object);
    }
    /// The object allocation of object `class` at
    /// the unit's position with flags (`flags`, 0, 0) (object 561).
    /// Not the missile wrapper.
    /// Default: [`Self::place_object`] in the
    /// unit's room.
    fn create_object_at(&mut self, at: UnitId, class: u16, flags: u32) -> Option<UnitId> {
        let (x, y, room) = self.unit_position(at)?;
        self.place_object(room, x, y, class, [flags as u8, 0, 0])
    }
    /// A missile (skill 0, level 1) at the unit's
    /// position (Baal's missile 625).
    fn create_missile_at(&mut self, at: UnitId, class: u16) -> Option<UnitId> {
        let (x, y) = self.unit_xy(at).unwrap_or((0, 0));
        self.create_missile(at, 0, 1, class, x, y)
    }
    /// A gold pile of `amount` at the unit.
    fn drop_gold_amount(&mut self, at: UnitId, amount: u32) {
        let _ = (at, amount);
        self.unhandled(0xFE, 0x0055_B030);
    }
    /// `monstats.txt` row count.
    fn monstats_rows(&mut self) -> u32 {
        self.unhandled(0xFE, 0x0058_E830);
        0
    }
    /// The `monstats.txt` flag of `class` the zoo test reads (flags byte
    /// bit 6).
    fn zoo_eligible(&mut self, class: u32) -> bool {
        let _ = class;
        self.unhandled(0xFE, 0x0058_E830);
        false
    }
    // -- end Act V part 2 seams.

    // -- Narrow seams of the shared helpers ([`helpers`]). Each default
    // reports its
    // function through `unhandled` (chain 0xFE) and returns the neutral
    // value.

    /// The object control's seed: the stream
    /// the object inits draw from. `None`: no object state.
    fn object_seed(&mut self) -> Option<&mut Seed> {
        None
    }
    /// The room's sub-tile box (X, Y, W, H). `None`: not
    /// known (the helpers then use the all-zero box of
    /// a null room).
    fn room_box(&mut self, room: RoomId) -> Option<crate::drlg::TileRect> {
        let _ = room;
        self.unhandled(0xFE, 0x0061_9730);
        None
    }
    /// Some cell of the (x, y, size, size) box of `room` has collision
    /// bits in `mask`. Default: collides.
    fn box_collides(&mut self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> bool {
        let _ = (room, x, y, size, mask);
        self.unhandled(0xFE, 0x0064_D800);
        true
    }
    /// Create a missile from a
    /// zeroed creation record.
    fn spawn_missile(&mut self, rec: helpers::QuestMissile) -> Option<UnitId> {
        let _ = rec;
        self.unhandled(0xFE, 0x0059_FA30);
        None
    }
    /// Set the missile's target field to `v` (the second untouched).
    fn set_missile_guid(&mut self, missile: UnitId, v: u32) {
        let _ = (missile, v);
        self.unhandled(0xFE, 0x0064_A710);
    }
    /// The position of the unit's path target unit.
    /// `None`: none.
    fn path_target_xy(&mut self, unit: UnitId) -> Option<(i32, i32)> {
        let _ = unit;
        self.unhandled(0xFE, 0x0056_D2C0);
        None
    }
    /// The unit of that type and GUID.
    /// Default: the player, monster and object lookups; other types
    /// reported.
    fn unit_by_guid(&mut self, kind: u8, guid: u32) -> Option<UnitId> {
        match kind {
            0 => self.player_by_guid(guid),
            1 => self.monster_by_guid(guid).map(|m| m.0),
            2 => self.object_by_guid(guid).map(|o| o.0),
            _ => {
                self.unhandled(0xFE, 0x0055_2F60);
                None
            }
        }
    }
    /// The C→S 0x4F trade body
    /// (button 6 = cancel).
    fn trade_button(&mut self, player: UnitId, button: u8) {
        let _ = (player, button);
        self.unhandled(0xFE, 0x0056_8060);
    }
    /// Unlink and free the player's chat node from the NPC's
    /// interaction list (nothing for a non-monster).
    fn free_chat_node(&mut self, npc: UnitId, player: UnitId) {
        let _ = (npc, player);
        self.unhandled(0xFE, 0x0057_2E00);
    }
    /// The end of a chat close: every chat node of the NPC freed and its
    /// interaction list emptied (after the per-player 0x62, interaction
    /// reset and callback).
    fn clear_npc_chats(&mut self, npc: UnitId) {
        let _ = npc;
        self.unhandled(0xFE, 0x0057_3180);
    }
    /// The obelisk's
    /// C→S 0x44 body.
    fn obelisk_close(&mut self, player: UnitId, object: UnitId) {
        let _ = (player, object);
        self.unhandled(0xFE, 0x0058_52E0);
    }
    /// The Steeg Stone's holder GUID := −1 when it holds
    /// the player's GUID.
    fn steeg_release(&mut self, object: UnitId, player: UnitId) {
        let _ = (object, player);
        self.unhandled(0xFE, 0x0058_4820);
    }
    /// Close the player's Horadric Cube
    /// (button 0x17).
    fn close_cube(&mut self, player: UnitId) {
        let _ = player;
        self.unhandled(0xFE, 0x0056_7330);
    }
    /// `objects.txt` `Mode1` of the object's class ≠ 0.
    fn object_mode1(&mut self, object: UnitId) -> Option<bool> {
        let _ = object;
        self.unhandled(0xFF, 0x0064_0E90);
        None
    }
    /// Clear unit flags `flags`.
    fn clear_unit_flags(&mut self, unit: UnitId, flags: u32) {
        let _ = (unit, flags);
        self.unhandled(0xFF, 0x0054_5850);
    }
    /// The units of type 5 (warp tiles) of the object's room unit list
    /// in list order; none without a room.
    fn room_warp_tiles(&mut self, object: UnitId) -> Vec<UnitId> {
        let _ = object;
        self.unhandled(0xFE, 0x0059_D9D0);
        Vec::new()
    }
    /// Warp the player through the warp tile.
    fn warp_through(&mut self, player: UnitId, tile: UnitId) {
        let _ = (player, tile);
        self.unhandled(0xFE, 0x0055_50B0);
    }
    /// The GUID of the player's town portal.
    /// `None`: no player data.
    fn town_portal_guid(&mut self, player: UnitId) -> Option<u32> {
        let _ = player;
        self.unhandled(0xFE, 0x0053_53F0);
        None
    }
    /// The portal's partner (the room at its destination
    /// streamed in, then the unit of the stored type and GUID).
    fn portal_partner(&mut self, portal: UnitId) -> Option<UnitId> {
        let _ = portal;
        self.unhandled(0xFE, 0x0055_3720);
        None
    }
    /// The portal leaves its room,
    /// is freed and its room (possibly
    /// null) is refreshed.
    fn free_portal_object(&mut self, portal: UnitId) {
        let _ = portal;
        self.unhandled(0xFE, 0x0055_5600);
    }
    // -- end helper seams.
}

/// A unit as the kill parse sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Player,
    Monster {
        class: u32,
        /// `superuniques.txt` hcIdx, if a superunique.
        superunique: Option<u32>,
        /// The owning player, if any.
        owner: Option<UnitId>,
    },
    Other,
}

/// Event arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventArgs {
    pub event: u8,
    pub target: Option<UnitId>,
    pub player: Option<UnitId>,
    /// Event 3: old level; event 11: NPC class.
    pub a: u32,
    /// Event 3: new level; event 11: message index.
    pub b: u32,
}

/// A text list being built for 0x27: (message index, menu).
pub type TextList = Vec<(u16, u32)>;

/// Players' current flag record.
fn flags_of<W: QuestWorld>(w: &mut W, player: UnitId) -> Option<&mut QuestFlags> {
    let d = usize::from(w.difficulty());
    w.quests(player).map(|q| &mut q.flags[d])
}

impl QuestControl {
    /// Every record, then the quest seed from one
    /// game-seed step, then an empty game record.
    pub fn new(tables: &QuestTables, game_seed: &mut Seed) -> Result<Self, QuestError> {
        if tables.rows.len() != TABLE_ROWS + INTRO_ROWS {
            return Err(QuestError::TableShape("want 37 + 4 rows"));
        }
        let mut made = Vec::new();
        for (i, row) in tables.rows.iter().enumerate() {
            if usize::from(row.index) != i || (i < TABLE_ROWS) == row.flag.is_none() {
                return Err(QuestError::TableShape("index / flag columns"));
            }
            let intro = i >= TABLE_ROWS;
            let mut r = QuestRecord {
                chain: row.chain,
                act: row.act,
                not_intro: !intro,
                active: intro,
                status: 0,
                state: 0,
                init_no: row.init_no.unwrap_or(0),
                seq_id: row.seq_id,
                flags: 0,
                // Every row states its filter (row 40 included); `-` would
                // be 0.
                filter: row.filter.unwrap_or(0),
                flag2: row.flag2,
                guids: GuidList::default(),
                callbacks: row.callbacks.iter().fold(0, |m, &(ev, _)| m | 1 << ev),
                status_fn: row.status_fn,
                active_fn: row.active_fn,
                seq_fn: row.seq_fn,
                msgs: row.msgs,
                extra: act1::Extra::default(),
            };
            act1::init(&mut r);
            act2::init(&mut r);
            act3::init(&mut r);
            made.push(r);
        }
        made.reverse();
        Ok(Self {
            records: made,
            executing: false,
            picked: false,
            game: QuestFlags::default(),
            timers: Vec::new(),
            tick: 0,
            seed: game_seed.derive(),
            fx: 0,
            rows: tables.rows.clone(),
            messages: tables.messages.clone(),
            faults: Vec::new(),
            host_requests: Vec::new(),
        })
    }

    /// An empty control with this one's tables (a placeholder while the
    /// host lends the control out, `wiring::economy::QuestLoan`).
    pub fn emptied(&self) -> Self {
        Self {
            records: Vec::new(),
            executing: false,
            picked: false,
            game: QuestFlags::default(),
            timers: Vec::new(),
            tick: 0,
            seed: self.seed,
            fx: 0,
            rows: Vec::new(),
            messages: Vec::new(),
            faults: Vec::new(),
            host_requests: Vec::new(),
        }
    }

    /// End the game: raised as a host
    /// request.
    pub fn end_game(&mut self) {
        self.host_requests.push(HostRequest::EndGame);
    }

    /// The save pass: raised as a host
    /// request.
    pub fn save_pass(&mut self) {
        self.host_requests.push(HostRequest::SavePass);
    }

    /// The host requests raised so far, in call order (emptied).
    pub fn take_host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host_requests)
    }

    /// The record index of `chain`.
    pub fn find(&self, chain: u8) -> Option<usize> {
        self.records.iter().position(|r| r.chain == chain)
    }

    /// The not-intro test the
    /// monster population and missile bodies call. The quest set must
    /// be picked (else the fatal 0x7C5, [`QuestError::NotPicked`]); a
    /// chain with no record is true; else the record's not-intro byte.
    /// No draw.
    pub fn not_intro_test(&self, chain: u8) -> Result<bool, QuestError> {
        if !self.picked {
            return Err(QuestError::NotPicked);
        }
        Ok(self.record(chain).is_none_or(|r| r.not_intro))
    }

    pub fn record(&self, chain: u8) -> Option<&QuestRecord> {
        self.find(chain).map(|i| &self.records[i])
    }

    pub fn record_mut(&mut self, chain: u8) -> Option<&mut QuestRecord> {
        self.find(chain).map(|i| &mut self.records[i])
    }

    /// Prepend a timer.
    pub fn add_timer(&mut self, chain: u8, func: TimerFn, period: u32) -> Result<(), QuestError> {
        if self.executing {
            return Err(QuestError::TimerWhileExecuting);
        }
        self.timers.insert(
            0,
            QuestTimer {
                func,
                chain,
                due: self.tick.wrapping_add(period),
                period,
            },
        );
        Ok(())
    }

    /// The updater, run at tick step 8 when frame % 20
    /// = 0.
    pub fn update<W: QuestWorld>(&mut self, w: &mut W) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick == u32::MAX {
            for t in &mut self.timers {
                t.due = u32::MAX - t.due;
            }
        }
        self.executing = true;
        let mut i = 0;
        while i < self.timers.len() {
            let t = self.timers[i];
            if t.due < self.tick {
                if act1::run_timer(self, w, t.func, t.chain) {
                    self.timers.remove(i);
                    continue;
                }
                self.timers[i].due = self.tick.wrapping_add(t.period);
            }
            i += 1;
        }
        self.executing = false;
    }

    // ------------------------------------------------------------ game entry

    /// A player enters the game. `mode` is 1 or 0 by the entry path
    /// (the two paths a player joins by).
    pub fn player_enters<W: QuestWorld>(
        &mut self,
        w: &mut W,
        player: UnitId,
        mode: u8,
    ) -> Result<(), QuestError> {
        let args = EventArgs {
            player: Some(player),
            target: Some(player),
            ..EventArgs::default()
        };
        if self.picked {
            self.dispatch_list(w, event::PLAYER_JOINED_GAME, args, true, None);
        } else {
            self.picked = true;
            if mode == 0 {
                // A game-wide switch that is always 1.
                let pf = flags_of(w, player).copied().unwrap_or_default();
                for k in 0..TABLE_ROWS {
                    let row = &self.rows[k];
                    let (chain, no_set_state) = (row.chain, row.no_set_state);
                    let Some(slot) = row.flag else { continue };
                    if no_set_state == Some(false)
                        && (pf.get(slot, bit::REWARD_GRANTED)
                            || pf.get(slot, bit::COMPLETED_BEFORE))
                    {
                        let r = self.record_mut(chain).ok_or(QuestError::NoRecord(chain))?;
                        r.not_intro = false;
                        r.active = false;
                        r.status = 0;
                        self.game.set(slot, bit::COMPLETED_BEFORE);
                    }
                }
                self.dispatch_list(w, event::PLAYER_STARTED_GAME, args, true, None);
            }
            for chain in [1, 8, 18, 22, 31] {
                let r = self.record(chain).ok_or(QuestError::NoRecord(chain))?;
                if r.seq_fn.is_none() {
                    return Err(QuestError::NoSequenceFn(chain));
                }
                act1::sequence(self, w, chain);
            }
        }
        // Steps 5–8.
        let mut m = vec![0x5E];
        for row in &self.rows[..TABLE_ROWS] {
            let r = self
                .record(row.chain)
                .ok_or(QuestError::NoRecord(row.chain))?;
            m.push(u8::from(r.not_intro));
        }
        w.send(player, &m);
        send_player_flags(w, player, 6, 0);
        self.send_game_flags(w, player);
        if self.record(1).is_some_and(|r| r.not_intro && r.state >= 4) {
            w.send(player, &[0x89, 0x00]);
        }
        Ok(())
    }

    /// S→C 0x29 (97 bytes).
    pub fn send_game_flags<W: QuestWorld>(&self, w: &mut W, player: UnitId) {
        let mut m = Vec::with_capacity(97);
        m.push(0x29);
        m.extend_from_slice(&self.game.0);
        w.send(player, &m);
    }

    // ------------------------------------------------------------ dispatch

    /// Dispatch to all records. `by_act`: filter by
    /// the act of `args.player`'s room. `list`: event 0's text list.
    pub fn dispatch_list<W: QuestWorld>(
        &mut self,
        w: &mut W,
        ev: u8,
        mut args: EventArgs,
        by_act: bool,
        mut list: Option<&mut TextList>,
    ) {
        args.event = ev;
        let act = if by_act {
            args.player.and_then(|p| w.unit_act(p))
        } else {
            None
        };
        // All callers pass ignore_active = 1.
        for i in 0..self.records.len() {
            let r = &self.records[i];
            if !r.has_callback(ev) || act.is_some_and(|a| a != r.act) {
                continue;
            }
            act1::callback(self, w, i, args, list.as_deref_mut(), false);
        }
    }

    /// Dispatch along the target unit's chain.
    pub fn dispatch_chain<W: QuestWorld>(
        &mut self,
        w: &mut W,
        ev: u8,
        mut args: EventArgs,
        force: bool,
    ) {
        args.event = ev;
        let Some(target) = args.target else { return };
        let Some(chain) = w.quest_chain(target).map(|c| c.0.clone()) else {
            return;
        };
        for c in chain {
            let Some(i) = self.find(c) else { continue };
            let r = &self.records[i];
            if r.has_callback(ev) && (r.active || force) {
                act1::callback(self, w, i, args, None, force);
            }
        }
    }

    /// Add a link for `chain` to the unit. `special`
    /// names the special case the caller found (chain 4 with an object of
    /// class 61; chains 8 and 12: empty stubs); it runs
    /// before the link is added.
    pub fn add_link<W: QuestWorld>(
        &mut self,
        w: &mut W,
        unit: UnitId,
        chain: u8,
        special: Option<u32>,
    ) -> bool {
        if self.find(chain).is_none() {
            return false;
        }
        if let Some(f) = special {
            match chain {
                4 => act1::q4::link_object(self, w, Some(unit)),
                8 | 12 => {}
                _ => w.unhandled(chain, f),
            }
        }
        let Some(c) = w.quest_chain(unit) else {
            return false;
        };
        if c.0.contains(&chain) {
            return false;
        }
        c.0.insert(0, chain);
        true
    }

    /// Link a new monster to its level's `Quest` chain.
    pub fn link_monster<W: QuestWorld>(&mut self, w: &mut W, unit: UnitId, level_quest: u8) {
        if level_quest != 0 {
            self.add_link(w, unit, level_quest, None);
        }
    }

    /// Kill parse.
    pub fn monster_killed<W: QuestWorld>(
        &mut self,
        w: &mut W,
        victim: UnitId,
        killer: Option<UnitId>,
    ) {
        if w.quest_chain(victim).is_none_or(|c| c.0.is_empty()) {
            return;
        }
        let killer = match killer {
            None if w.game_type() == 3 => w.first_client_player(),
            k => k,
        };
        let player = killer.and_then(|k| match w.unit_kind(k) {
            UnitKind::Player => Some(k),
            UnitKind::Monster { owner, .. } => owner,
            UnitKind::Other => None,
        });
        let force = match w.unit_kind(victim) {
            UnitKind::Monster {
                class, superunique, ..
            } => {
                superunique.is_some_and(|s| matches!(s, 26 | 27 | 29 | 36..=39 | 42..=45 | 60))
                    || matches!(class, 242 | 243 | 391 | 544)
            }
            _ => false,
        };
        let args = EventArgs {
            target: Some(victim),
            player,
            ..EventArgs::default()
        };
        self.dispatch_chain(w, event::MONSTER_KILLED, args, force);
    }

    /// A player leaves.
    pub fn player_leaves<W: QuestWorld>(&mut self, w: &mut W, player: UnitId) {
        for (item, quest) in w.quest_items(player) {
            let Some(i) = self.find(quest.wrapping_sub(1)) else {
                continue;
            };
            if self.records[i].has_callback(event::PLAYER_DROPPED_WITH_QUEST_ITEM) {
                let args = EventArgs {
                    event: event::PLAYER_DROPPED_WITH_QUEST_ITEM,
                    target: Some(item),
                    player: Some(player),
                    ..EventArgs::default()
                };
                act1::callback(self, w, i, args, None, false);
            }
        }
        let args = EventArgs {
            player: Some(player),
            target: Some(player),
            ..EventArgs::default()
        };
        self.dispatch_list(w, event::PLAYER_LEAVES_GAME, args, false, None);
    }

    /// Level change (event 3, (1, 0)).
    pub fn changed_level<W: QuestWorld>(&mut self, w: &mut W, player: UnitId, old: u32, new: u32) {
        let args = EventArgs {
            target: Some(player),
            player: Some(player),
            a: old,
            b: new,
            ..EventArgs::default()
        };
        self.dispatch_list(w, event::CHANGED_LEVEL, args, false, None);
    }

    /// The quest part of the town-leave refresh: the NPC intro list of
    /// the town's act (acts 0, 1, 2, 4), and for Lut Gholein (level 40)
    /// the act change (chain 4).
    pub fn town_leave<W: QuestWorld>(&mut self, w: &mut W, player: UnitId, to: u32) {
        match to {
            1 => npc_gossip(w, player, 0),
            40 => {
                npc_gossip(w, player, 1);
                act1::q4::act_change(self, w, player);
            }
            75 => npc_gossip(w, player, 2),
            109 => npc_gossip(w, player, 4),
            _ => {}
        }
    }

    /// NPC chat end (event 2, (1, 1)).
    pub fn npc_deactivate<W: QuestWorld>(&mut self, w: &mut W, player: UnitId, npc: UnitId) {
        let args = EventArgs {
            target: Some(npc),
            player: Some(player),
            ..EventArgs::default()
        };
        self.dispatch_list(w, event::NPC_DEACTIVATE, args, true, None);
    }

    /// NPC chat start (event 0, (1, 1)); quest records add
    /// their lines to `list`.
    pub fn npc_activate<W: QuestWorld>(
        &mut self,
        w: &mut W,
        player: UnitId,
        npc: UnitId,
        list: &mut TextList,
    ) {
        let args = EventArgs {
            target: Some(npc),
            player: Some(player),
            ..EventArgs::default()
        };
        self.dispatch_list(w, event::NPC_ACTIVATE, args, true, Some(list));
    }

    /// Item picked up (4) or dropped (5),
    /// along the item's chain, force 0.
    pub fn item_event<W: QuestWorld>(&mut self, w: &mut W, ev: u8, player: UnitId, item: UnitId) {
        let args = EventArgs {
            target: Some(item),
            player: Some(player),
            ..EventArgs::default()
        };
        self.dispatch_chain(w, ev, args, false);
    }

    /// Refresh an NPC's text after a state change.
    pub fn refresh_text<W: QuestWorld>(&mut self, w: &mut W, player: UnitId, npc: UnitId) {
        let mut list = TextList::new();
        self.npc_activate(w, player, npc, &mut list);
        w.send_text_list(player, npc, &list);
        self.send_game_flags(w, player);
    }

    /// Add the table's lines for (state, npc).
    pub fn add_messages(&self, table: u32, list: &mut TextList, npc: u16, state: u8) {
        let mut entries: Vec<&MessageEntry> = self
            .messages
            .iter()
            .filter(|m| m.table == table && m.state == state && m.npc == npc)
            .collect();
        entries.sort_by_key(|m| m.slot);
        for m in entries {
            list.push((m.string, if m.menu == 1 { 0 } else { m.menu }));
        }
    }

    // ------------------------------------------------------------ status

    /// The default status rule.
    pub fn default_status(r: &QuestRecord, pf: &QuestFlags) -> Result<u8, QuestError> {
        let q = r.filter;
        let (s, n, l) = (r.state, r.init_no, r.status);
        let done = pf.get(q, bit::PRIMARY_GOAL_DONE);
        let now = pf.get(q, bit::COMPLETED_NOW);
        Ok(if s < n {
            if now || (r.chain == 4 && l == 6 && !done) {
                12
            } else {
                l
            }
        } else if done {
            if q > 40 {
                return Err(QuestError::Filter(q));
            }
            l
        } else if r.chain == 4 && now {
            if s == 6 {
                12
            } else {
                l
            }
        } else if r.chain == 10 {
            if l == 4 {
                4
            } else {
                12
            }
        } else {
            12
        })
    }

    /// The status a record reports to a player: its status function, or
    /// the default rule. `None`: nothing reported.
    fn status_for<W: QuestWorld>(
        &self,
        w: &mut W,
        i: usize,
        player: UnitId,
    ) -> Result<Option<u8>, QuestError> {
        let r = &self.records[i];
        let pf = flags_of(w, player).copied().unwrap_or_default();
        match r.status_fn {
            Some(f) => Ok(act1::status_fn(self, w, i, player, &pf, f)),
            None => Self::default_status(r, &pf).map(Some),
        }
    }

    /// C→S 0x40: 0x28, maybe 0x50, then 0x52.
    pub fn request_quest_data<W: QuestWorld>(
        &self,
        w: &mut W,
        player: UnitId,
    ) -> Result<(), QuestError> {
        send_player_flags(w, player, 6, 0);
        let mut list = [0u8; 41];
        for i in 0..self.records.len() {
            let r = &self.records[i];
            if r.status == 0 {
                continue;
            }
            if r.chain > 40 {
                return Err(QuestError::NoRecord(r.chain));
            }
            if let Some(s) = self.status_for(w, i, player)? {
                if let Some(slot) = list.get_mut(usize::from(r.filter)) {
                    *slot = s;
                }
            }
        }
        let pf = flags_of(w, player).copied().unwrap_or_default();
        let staff = (pf.get(12, 0) || pf.get(12, 13)) && w.has_act2();
        if list[1] != 0 || list[36] != 0 || staff {
            let mut m = [0u8; 15];
            m[0] = 0x50;
            m[1..3].copy_from_slice(&1u16.to_le_bytes());
            if list[1] != 0 {
                let left = self.record(1).map_or(0, act1::den_monsters_left);
                m[3..5].copy_from_slice(&left.to_le_bytes());
            }
            if staff {
                let tomb = (w.true_tomb_level() as i32 - 66) as i16;
                m[5..7].copy_from_slice(&tomb.to_le_bytes());
            }
            if list[36] != 0 {
                let left = act5::barbarians_left(self, w);
                m[7..9].copy_from_slice(&left.to_le_bytes());
            }
            w.send(player, &m);
        }
        let mut m = vec![0x52];
        m.extend_from_slice(&list);
        w.send(player, &m);
        Ok(())
    }

    /// S→C 0x5D for one quest to one player.
    pub fn send_status<W: QuestWorld>(
        &self,
        w: &mut W,
        player: UnitId,
        chain: u8,
    ) -> Result<(), QuestError> {
        let Some(i) = self.find(chain) else {
            return Ok(());
        };
        let r = &self.records[i];
        if w.unit_act(player) != Some(r.act) {
            return Ok(());
        }
        // The status byte starts at 0 and takes the function's out byte
        // only when it returns 1: a status function
        // returning 0 sends status 0.
        let status = self.status_for(w, i, player)?.unwrap_or(0);
        let extra = match r.filter {
            1 => act1::den_monsters_left(r),
            36 => act5::barbarians_left(self, w),
            _ => 0,
        };
        let mut m = [0u8; 6];
        m[0] = 0x5D;
        m[1] = chain;
        m[2] = r.flags;
        m[3] = status;
        m[4..6].copy_from_slice(&extra.to_le_bytes());
        w.send(player, &m);
        Ok(())
    }

    /// Status with iterate: set the status byte, then 0x5D to every
    /// player. The per-quest iterate functions' flag tests are not
    /// known; D2RUST sends to every player.
    pub fn set_status_all<W: QuestWorld>(
        &mut self,
        w: &mut W,
        chain: u8,
        status: u8,
    ) -> Result<(), QuestError> {
        let i = self.find(chain).ok_or(QuestError::NoRecord(chain))?;
        self.records[i].status = status;
        for p in w.players() {
            self.send_status(w, p, chain)?;
        }
        Ok(())
    }

    /// The active test: the NPC AI's
    /// interact gate. `interact`: the monstats `interact` flag of the
    /// NPC's class; without it, false (no walk, no send). Else the
    /// records of the player's act are walked in list order (newest →
    /// oldest: chains 40 … 37, then 36 … 0); the first whose
    /// active fn is true sends S→C `8A 01 <npc GUID>` to the player and
    /// the result is true. None true: false.
    ///
    /// The act is the player unit's act (`unit_act`), not the act byte
    /// of the player's client: D2RUST's sim has no client act. The two
    /// agree.
    pub fn npc_wants_interact<W: QuestWorld>(
        &self,
        w: &mut W,
        player: UnitId,
        npc: UnitId,
        npc_class: u16,
        interact: bool,
    ) -> Result<bool, QuestError> {
        if !interact {
            return Ok(false);
        }
        if !self.picked {
            return Err(QuestError::NotPicked);
        }
        let act = w.unit_act(player);
        // `records` is in list order (newest first).
        for i in 0..self.records.len() {
            let r = &self.records[i];
            let Some(f) = r.active_fn else { continue };
            if Some(r.act) != act {
                continue;
            }
            if act1::active_fn(self, w, i, player, npc_class, f) {
                let mut m = [0u8; 6];
                m[0] = 0x8A;
                m[1] = 1;
                m[2..6].copy_from_slice(&w.guid(npc).to_le_bytes());
                w.send(player, &m);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Store b, then 0x28 and `89 b` to every player.
    pub fn unique_event<W: QuestWorld>(&mut self, w: &mut W, b: u8) {
        self.fx = b;
        for p in w.players() {
            send_player_flags(w, p, 6, 0);
            w.send(p, &[0x89, b]);
        }
    }

    // ------------------------------------------------------------ NPC dialog

    /// C→S 0x31. Returns the result
    /// code.
    pub fn quest_message<W: QuestWorld>(&mut self, w: &mut W, player: UnitId, msg: &[u8]) -> u32 {
        if msg.len() != 9 {
            return 3;
        }
        let guid = u32::from_le_bytes(msg[1..5].try_into().expect("4 bytes"));
        let index = u16::from_le_bytes([msg[5], msg[6]]);
        let npc = if guid != u32::MAX {
            w.monster_by_guid(guid)
        } else {
            None
        };
        crate::cov!(
            NpcTopic,
            npc.map_or(0, |n| n.1),
            (2u32 << 16) | u32::from(index)
        );
        let args = EventArgs {
            target: npc.map(|n| n.0),
            player: Some(player),
            a: u32::from(npc.map_or(0, |n| n.1)),
            b: u32::from(index),
            ..EventArgs::default()
        };
        self.dispatch_list(w, event::SCROLL_MESSAGE, args, true, None);
        0
    }

    /// C→S 0x58. Returns the result code.
    pub fn quest_completed<W: QuestWorld>(&self, w: &mut W, player: UnitId, msg: &[u8]) -> u32 {
        if msg.len() != 3 {
            return 3;
        }
        let q = u16::from_le_bytes([msg[1], msg[2]]);
        if q >= u16::from(SLOTS) {
            return 2;
        }
        if let Some(f) = flags_of(w, player) {
            f.set(q as u8, bit::UPDATE_QUEST_LOG);
        }
        0
    }

    // ------------------------------------------------------------ act travel

    /// NPC act travel, before the act change.
    pub fn act_completion<W: QuestWorld>(
        &mut self,
        w: &mut W,
        player: UnitId,
        npc_class: u16,
    ) -> Result<(), QuestError> {
        match npc_class {
            npc::WARRIV1 => {
                let Some(f) = flags_of(w, player) else {
                    return Ok(());
                };
                if !f.get(7, 0) {
                    f.set(7, 0);
                    f.set(7, 13);
                    send_player_flags(w, player, 6, 0);
                    can_go_to_act(w, player, 2);
                    set_intro_flags(w, player, 0);
                }
                let f = flags_of(w, player).copied().unwrap_or_default();
                if f.get(6, 13) && f.get(6, 0) {
                    if let Some(i) = self.find(6).filter(|&i| self.records[i].not_intro) {
                        // Chain 6's callback 3 with a = 1, b = 40.
                        let args = EventArgs {
                            event: event::CHANGED_LEVEL,
                            player: Some(player),
                            a: 1,
                            b: 40,
                            ..EventArgs::default()
                        };
                        act1::callback(self, w, i, args, None, false);
                    }
                }
                // Chain 4's act change runs inside the travel's
                // town-leave refresh ([`Self::town_leave`]).
            }
            npc::MESHIF1 => {
                let Some(f) = flags_of(w, player) else {
                    return Ok(());
                };
                if !f.get(15, 0) {
                    if !f.get(10, 0) {
                        f.set(10, 0);
                        f.set(10, 13);
                        w.delete_item(player, *b"msf ");
                        w.delete_item(player, *b"vip ");
                    }
                    let f = flags_of(w, player).expect("checked");
                    f.set(15, 0);
                    f.set(15, 13);
                    set_intro_flags(w, player, 1);
                    send_player_flags(w, player, 6, 0);
                    can_go_to_act(w, player, 3);
                }
            }
            npc::TYRAEL2 => {
                let expansion = w.expansion();
                let Some(f) = flags_of(w, player) else {
                    return Ok(());
                };
                if !f.get(28, 0) && expansion {
                    f.set(28, 0);
                    f.set(28, 13);
                    // The Act II list (sic).
                    set_intro_flags(w, player, 1);
                    send_player_flags(w, player, 6, 0);
                    if w.player_byte_4c(player) != 1 {
                        w.send(player, &[0x5D, 0x17, 2, 0, 0, 0]);
                        can_go_to_act(w, player, 5);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Object warp to `level`. Level 102 is the
    /// Durance; others go to the A3Q6 warp (not in D2RUST).
    pub fn object_warp<W: QuestWorld>(&mut self, w: &mut W, player: UnitId, level: u32) {
        if level != 102 {
            act3::durance_warp(self, w);
            return;
        }
        let Some(f) = flags_of(w, player) else { return };
        if f.get(23, 0) {
            return;
        }
        if !f.get(18, 0) {
            f.set(18, 0);
            f.set(18, 13);
            for code in [*b"qey ", *b"qhr ", *b"qbr ", *b"qf1 ", *b"qf2 "] {
                w.delete_item(player, code);
            }
        }
        let f = flags_of(w, player).expect("checked");
        f.set(23, 0);
        f.set(23, 13);
        set_intro_flags(w, player, 2);
        send_player_flags(w, player, 6, 0);
        can_go_to_act(w, player, 4);
    }
}

/// S→C 0x28 (103 bytes) with the player's current record.
pub fn send_player_flags<W: QuestWorld>(w: &mut W, player: UnitId, unit_type: u8, guid: u32) {
    let rec = flags_of(w, player).copied().unwrap_or_default();
    let mut m = Vec::with_capacity(103);
    m.push(0x28);
    m.push(unit_type);
    m.extend_from_slice(&guid.to_le_bytes());
    m.push(0);
    m.extend_from_slice(&rec.0);
    w.send(player, &m);
}

/// The progression on the save flags: bits 8–12 hold the progression
/// p; n = m ·
/// difficulty + step with m = 5 for an expansion character (bit 5), else
/// 4; p is raised to n, never lowered. n is or-ed in unmasked.
pub fn progression(flags: u16, step: u8, difficulty: u8) -> u16 {
    let m = ((u32::from(flags) & 0x20) | 0x80) >> 5;
    let n = m * u32::from(difficulty) + u32::from(step);
    let p = (u32::from(flags) >> 8) & 0x1F;
    if n < p {
        return flags;
    }
    ((u32::from(flags) & 0xE0FF) | (n << 8)) as u16
}

/// [`progression`] on the player's client; `false` (and
/// nothing changed) when the host has no client for it (the caller
/// reports it).
pub fn raise_progression<W: QuestWorld>(
    w: &mut W,
    player: UnitId,
    step: u8,
    difficulty: u8,
) -> bool {
    match w.client_save_flags(player) {
        Some(f) => {
            w.set_client_save_flags(player, progression(f, step, difficulty));
            true
        }
        None => false,
    }
}

/// The player's act byte ≠ 1 → set it and send `61 act`.
fn can_go_to_act<W: QuestWorld>(w: &mut W, player: UnitId, act: u8) {
    if w.player_byte_4c(player) != 1 {
        w.set_player_byte_4c(player, 1);
        w.send(player, &[0x61, act]);
    }
}

/// NPC intro lists by act (Act IV has none).
pub const INTRO_NPCS: [&[u16]; 5] = [
    &[148, 154, 150, 147, 155, 265],
    &[175, 176, 177, 178, 202, 200, 210, 201, 198, 199, 244],
    &[245, 252, 253, 254, 255, 264, 297],
    &[],
    &[520, 521, 511, 512, 513, 514, 515],
];

/// Set the intro bit of every NPC of the act's list.
pub fn set_intro_flags<W: QuestWorld>(w: &mut W, player: UnitId, act: u8) {
    let d = usize::from(w.difficulty());
    if let Some(q) = w.quests(player) {
        for &n in INTRO_NPCS[usize::from(act)] {
            q.intro[d].insert(n);
        }
    }
}

/// S→C 0x91 for acts 0, 1, 2, 4; introduced NPCs
/// packed at the front; sent only if one is set.
pub fn npc_gossip<W: QuestWorld>(w: &mut W, player: UnitId, act: u8) {
    if act == 3 || act > 4 {
        return;
    }
    let d = usize::from(w.difficulty());
    let heard = w
        .quests(player)
        .map(|q| q.intro[d].clone())
        .unwrap_or_default();
    let mut m = [0xFFu8; 26];
    m[0] = 0x91;
    m[1] = act;
    let mut n = 0;
    for &c in INTRO_NPCS[usize::from(act)] {
        if heard.contains(&c) {
            m[2 + 2 * n..4 + 2 * n].copy_from_slice(&c.to_le_bytes());
            n += 1;
        }
    }
    if n > 0 {
        w.send(player, &m);
    }
}

/// For each GUID whose player exists and lacks bits
/// 0 and 1 of `slot`: set bits 13 and 1; attach `sound` if ≠ 0.
pub fn grant_pending<W: QuestWorld>(w: &mut W, list: &GuidList, slot: u8, sound: u16) {
    for &g in &list.0 {
        let Some(p) = w.player_by_guid(g) else {
            continue;
        };
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(slot, bit::REWARD_GRANTED) || f.get(slot, bit::REWARD_PENDING) {
            continue;
        }
        f.set(slot, bit::PRIMARY_GOAL_DONE);
        f.set(slot, bit::REWARD_PENDING);
        if sound != 0 {
            w.attach_sound(p, sound);
        }
    }
}

/// Result of a warp or portal check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpCheck {
    Open,
    /// Decided by an Act II–V function D2RUST does not have.
    Delegate(u32),
}

/// The warp check.
pub fn warp_check(from: u32, to: u32) -> WarpCheck {
    match to {
        73 => WarpCheck::Delegate(0x0059_DB20),
        100 => WarpCheck::Delegate(0x005B_BFA0),
        118 | 128 if from == 120 => WarpCheck::Delegate(0x0058_D090),
        132 => WarpCheck::Delegate(0x0058_E640),
        _ => WarpCheck::Open,
    }
}

/// The portal check.
pub fn portal_check(level: u32) -> WarpCheck {
    if level == 73 {
        WarpCheck::Delegate(0x0059_DFD0)
    } else {
        WarpCheck::Open
    }
}

/// The Cow portal (cube output kind 1).
pub fn cow_portal<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) -> bool {
    let expansion = w.expansion();
    let in_town = w.unit_level(player) == Some(1);
    let pf = flags_of(w, player).copied().unwrap_or_default();
    let refused = ctl.game.get(4, 11)
        || pf.get(4, 10)
        || (!expansion && !pf.get(26, 0))
        || (expansion && !pf.get(40, 0))
        || !in_town;
    if !refused {
        if let Some((x, y)) = w.free_spot(player, 3, 0x400, 4, 100) {
            if w.create_portal(player, x, y, 60, 39) {
                ctl.game.set(4, 11);
                return true;
            }
        }
    }
    // Every failure (a refusal test, no free spot, no
    // portal) takes the same exit: the sound.
    w.attach_sound(player, u16::from(crate::world::cube::SOUND_COW_REFUSED));
    false
}

/// C→S 0x3E read a clue item (after the handler's
/// item checks). `bkd ` → the Cairn stone order; `trs ` →
/// `true_tomb_clue`.
pub fn read_clue<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId, code: [u8; 4]) {
    match &code {
        b"bkd " => act1::send_stone_order(ctl, w, player),
        b"trs " => true_tomb_clue(ctl, w, player),
        _ => {}
    }
}

/// 0x50 with u16 13 and the true tomb's level − 66
/// (0 when the level is 0); a nonzero level is kept at chain 13's extra
/// (its tomb level). Bytes 5–14 are never written (stack); 0 here.
fn true_tomb_clue<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let level = w.true_tomb_level();
    let tomb = if level == 0 { 0 } else { level as i32 - 66 };
    if level != 0 {
        if let Some(r) = ctl.record_mut(13) {
            r.extra.tomb_level = level;
        }
    }
    let mut m = [0u8; 15];
    m[0] = 0x50;
    m[1..3].copy_from_slice(&13u16.to_le_bytes());
    m[3..5].copy_from_slice(&(tomb as i16).to_le_bytes());
    w.send(player, &m);
}

/// Object timer event 7 by object class. The Act
/// II–V functions D2RUST does not have are reported.
pub fn object_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId, class: u16) {
    match class {
        // Cain's gibbet.
        0x1A => act1::q4::gibbet_event(ctl, w, object),
        0x7A => act2::q4::harem_blocker(ctl, w, object),
        0x83 => {
            let Some(level) = w.unit_level(object) else {
                return;
            };
            if w.object_mode(object) == 1 {
                w.set_object_mode(object, 2);
            }
            match level {
                76 => w.unhandled(0xFF, 0x005B_23C0),
                108 => act4::q2::dummy_event(ctl, w, object),
                _ => {}
            }
        }
        0xBD => match (w.unit_act(object), w.unit_level(object)) {
            (Some(0), _) => act1::q4::cain_portal_event(ctl, w, object),
            (_, Some(l)) if l == 109 || l >= 113 => {
                if ctl.find(33).is_some() {
                    act5::q3::portal_event(ctl, w, object);
                }
            }
            _ => {
                if ctl.find(32).is_some() {
                    act5::q2::portal_event(ctl, w, object);
                }
            }
        },
        act1::WIRT_BODY => act1::wirt_body(ctl, w, object),
        0x155 => act3::bridge_event(ctl, w, object),
        0x16F => act3::lever_event(ctl, w, object),
        0x173 => act1::q5::chest_event(ctl, w, object),
        0x178 if ctl.find(24).is_some() => act4::q3::forge_event(ctl, w, object),
        0x1CB => act5::q4::temple_portal_event(ctl, w, object),
        0x1CC => act5::q3::anya_dummy_event(ctl, w, object),
        0x1CD => act5::q3::nihlathak_dummy_event(ctl, w, object),
        0x1DA..=0x1DC if ctl.find(35).is_some() => act5::q5::statue_event(ctl, w, object),
        _ => {}
    }
}

#[cfg(test)]
mod mutant_tests;
