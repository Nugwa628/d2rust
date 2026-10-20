//! Client world model: what the S→C messages have told the client. Plain
//! Rust, no Bevy. Not game state: `d2-sim` on the server is.
//!
//! [`ClientWorld`] holds the client model's world fields plus the
//! bridge's own counters; [`ClientUnit`] the per-unit fields, with the
//! per-kind data. The tables and the visibility predicate the
//! message rules read are inputs ([`ModelInputs`]), not model fields.

pub use super::monster_anim::MonsterSequences;
pub use super::msg::pets::HirelingRows;
pub use crate::app::single_player::rows::{
    Critters, LevelRow, MonsterClass, MonsterSetup, ObjectRow, SkillDescRow, SkillRow, StatSend,
    StateOverlayLinks, StateRow, UnitRows,
};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use super::objects::{ClientObjects, ObjClientInputs};

use super::drlg::{ClientDrlg, DrlgRoomId, DrlgSource};
use super::skills::SkillList;
use crate::rules::lighting::environment::Environment;
use crate::rules::lighting::overrides::Overrides;
use crate::rules::lighting::records::{LightError, LightList, LightRooms, Owner, RoomId};
use d2_proto::transport::server_message;

/// Unit types.
pub const PLAYER: u8 = 0;
/// Unit type of monsters.
pub const MONSTER: u8 = 1;
pub const OBJECT: u8 = 2;
pub const MISSILE: u8 = 3;
pub const ITEM: u8 = 4;
pub const TILE: u8 = 5;

/// What the client skill start put on the caster's path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastTarget {
    Point(u16, u16),
    Unit(UnitKey),
}

/// A unit's identity: (unit type, GUID).
/// Ordered by type, then GUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitKey {
    pub unit_type: u8,
    pub guid: u32,
}

impl UnitKey {
    pub const fn new(unit_type: u8, guid: u32) -> Self {
        Self { unit_type, guid }
    }
}

/// Bit 31 of a GUID in a view key: the unit is a client-made monster of
/// set C, not of set S (server GUIDs stay below it).
pub const VIEW_CLIENT_BIT: u32 = 0x8000_0000;

/// The key the view knows a set C unit by.
pub fn view_key(key: UnitKey) -> UnitKey {
    UnitKey::new(key.unit_type, key.guid | VIEW_CLIENT_BIT)
}

/// A mode request handed to the unit-type mode machines: `code` and
/// the 7-value record; entries the base game leaves unset are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRequest {
    pub code: u8,
    pub record: [i32; 7],
}

/// The last cast request (codes 0x15 / 0x16) a unit's mode machine ran:
/// D2RUST bookkeeping (no base-game field). The base game starts the cast's client
/// effects inside the request itself, so
/// a later request in the same tick (a 0x13 soft hit) must not hide it
/// from the effect layer the way it replaces `last_mode_request`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CastRequest {
    pub request: ModeRequest,
    /// `ClientUnit::mode_requests` counting this request: tells a repeat
    /// of the same cast from the one already seen.
    pub count: u32,
    /// [`ClientUnit::request_late`] of this request.
    pub late: bool,
    /// [`ClientWorld::server_ticks`] when the request ran: the tick the
    /// cast mode (and its overlay) started.
    pub tick: u64,
}

/// One hit overlay create with (U, id, 2, 0, 0, 0, 0, 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitOverlay {
    pub id: u16,
    /// `ClientUnit::hit_overlays` counting this create.
    pub count: u32,
    /// [`ClientWorld::server_ticks`] when the create ran.
    pub tick: u64,
}

/// Player data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerData {
    /// The 16 name bytes of 0x59 (zero-padded).
    pub name: [u8; 16],
    /// The inventory's cursor item (GUID of an item unit).
    /// TODO: which item actions set it; 0x42 and 0x58 code 5
    /// clear it.
    pub cursor_item: Option<u32>,
    /// Player data +0x2C, written by S→C 0x5F;
    /// its reader is an open question.
    pub f2c: u32,
}

/// Monster data from 0xAC (4, 5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterData {
    pub name_seed: u16,
    /// Type flags: champion 4, unique 8, superunique 2, minion 0x10,
    /// ghostly 0x40.
    pub flags: u8,
    pub hc_idx: u16,
    /// The umod bytes as read (the 9-byte buffer, zero after the list).
    pub umods: [u8; 9],
    /// The 16 component choices (0 when not sent).
    pub components: [u8; 16],
    /// −1 when not sent.
    pub value: i32,
    /// The optional 31-bit value (open question 3: what it sets).
    pub v31: Option<u32>,
    /// The stat list with flag 0x40: (stat, param) → value.
    pub stat_list: Option<BTreeMap<(u16, u16), i32>>,
    /// Monster data +0x40, written by S→C 0x98
    /// (−1 for 0xFFFF); `None` until written. Meaning: an open question.
    pub f40: Option<i32>,
    /// Monster data +0x30: the critter AI's think timer T
    /// (0 at the create).
    pub think: i32,
    /// T as the first critter AI call read it (the `think0` record of
    /// `record_client_creations.py`); `None` before that call.
    pub first_think: Option<i32>,
    /// Monster data +0x34: the critter AI's flag F (0 at the create).
    pub critter_flag: i32,
    /// The owner relation as the 0xAC create read it
    /// (`Utrans` 255; `msg/units.rs`
    /// `pet_shift_relation`).
    pub owner_relation: bool,
}

/// The last item message an item unit received, until the item stream
/// is specified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRecord {
    /// 0x9C or 0x9D.
    pub id: u8,
    pub action: u8,
    pub category: u8,
    /// The owner of 0x9D.
    pub owner: Option<UnitKey>,
    pub stream: Vec<u8>,
    /// D2RUST's own, unverified: [`ClientWorld::store_serial`] when this
    /// store-shown record (action 0x0B) arrived; 0 for every other
    /// action. Lets the shop panel tell the items of the current trade
    /// from those of an earlier one.
    pub seq: u32,
}

/// Item data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemData {
    pub last: Option<ItemRecord>,
    /// The item data the last record's reader wrote.
    pub fields: Option<super::item_lists::ItemFields>,
    /// Item flag 4 (0x3F, 0x3E, 0x40).
    pub flags4: bool,
    /// The other bits of the item flag word (item data +0x18) as the
    /// model rules write them (0x3E, 0x40, 0x7D); bit 4 is `flags4`.
    pub flags: u32,
    /// The properties of the last record's lists (`bridge::item_lists`).
    pub props: Vec<super::item_lists::ItemProp>,
    /// The item is a charm (type `char`).
    pub charm: bool,
    /// S→C 0x92 took the item out of its owner's body slot and detached
    /// its stat list; the next 0x9C / 0x9D
    /// record about the item re-adds it (`item_lists::refresh` clears
    /// this).
    pub unlinked: bool,
    /// The dual-wield stat toggle turned the item's list off
    /// (the item list toggle with (U, item, 0)): attached but not
    /// counted in the owner's totals until the toggle turns it on again
    /// or a new record re-adds it (`bridge::attack_weapon`).
    pub stats_off: bool,
}

impl ItemData {
    /// The flag set with (item, mask, on) on the item flag word.
    pub fn set_flags(&mut self, mask: u32, on: bool) {
        if mask & 4 != 0 {
            self.flags4 = on;
        }
        let rest = mask & !4;
        if on {
            self.flags |= rest;
        } else {
            self.flags &= !rest;
        }
    }
}

/// Object data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectData {
    /// +0x04: interact (0x51), the destination level (0x60).
    pub interact: u8,
    /// +0x05: the portal flags (0x60 ORs bits 0–1).
    pub portal_flags: u8,
    /// +0x28: the owner name of a portal (0x82, 16 bytes).
    pub owner_name: Option<[u8; 16]>,
    /// The shrine record's `Code` (shrine data +0x08,
    /// step 1); `None`: no shrine record. Set by 0x51 for a shrine
    /// (the shrines record of
    /// index interact).
    pub shrine: Option<u8>,
    /// The object's footprint is on the client grid: stamped by the
    /// object init (when objects
    /// `HasCollision[mode]` +0x120 + mode ≠ 0),
    /// freed. The
    /// client path stamps it ([`crate::world_view::walk::other_objects`]).
    pub footprint: bool,
    /// The mode whose objects `BlocksLight` (+0x118 + mode) last wrote
    /// unit flag +0xC4 0x20 (set when it is 0: no shadow): the object init
    /// and the 0x0E mode change
    /// write it; the mode-1 finish, the door finish
    /// and the day refresh keep it.
    /// `None`: never written.
    pub shadow_mode: Option<u32>,
}

/// The per-kind data at unit +0x14 (model).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KindData {
    #[default]
    None,
    Player(PlayerData),
    Monster(Box<MonsterData>),
    Object(ObjectData),
    Item(ItemData),
}

/// A unit as the client knows it (model).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientUnit {
    pub key: UnitKey,
    pub class: u32,
    /// Set only by creation and the mode machines.
    pub mode: u32,
    /// Subtile cell; `None` = not placed (created at (0, 0)).
    pub position: Option<(u16, u16)>,
    /// The last point given to the position check.
    pub server_point: (u16, u16),
    /// How many times the position check took the server's point for the
    /// local player while the play preview predicts its walk
    /// (`check::Checked::Followed`): the prediction snaps on a change.
    /// D2RUST's own, PROVISIONAL; settled by a capture of the client
    /// state over a held walk.
    pub follows: u32,
    /// Stat list layer 0 base values.
    pub stats: BTreeMap<u16, i32>,
    /// Client copy of the unit seed {lo, hi}. `None` when it
    /// was derived from a client room's seed with no client DRLG in the
    /// model (no DRLG source).
    pub seed: Option<(u32, u32)>,
    /// How many times the unit seed was set anew rather than stepped (a
    /// client do's):
    /// D2RUST bookkeeping (no base-game field), so a follower of the seed
    /// (`audio::driver::ClientSeed`) sees a re-derived seed that equals
    /// the value it last read.
    pub seed_sets: u32,
    /// Queued unit-handler messages.
    pub queue: Vec<Vec<u8>>,
    pub last_mode_request: Option<ModeRequest>,
    /// The request came from a scripted click that reached the model after
    /// the update the base game runs it before ([`ClientWorld::late_input`]): effects
    /// it starts count their age from one tick earlier.
    pub request_late: bool,
    /// How many mode requests the unit has had: D2RUST
    /// bookkeeping (no base-game field), so a reader can tell a new request
    /// from a repeat of the same one.
    pub mode_requests: u32,
    /// The last cast request of the unit ([`CastRequest`]).
    pub last_cast: Option<CastRequest>,
    /// The last hit overlay created on the unit
    /// ([`HitOverlay`]), for the effect layer to draw.
    pub last_hit_overlay: Option<HitOverlay>,
    /// How many hit overlays the unit has had: D2RUST bookkeeping (no base-game
    /// field), so a reader can tell a new one from the one already seen.
    pub hit_overlays: u32,
    /// The hit flash counter U +0x58:
    /// 3 after a monster get-hit (code 0x06) or soft hit (0x13);
    /// each unit draw
    /// of a non-zero counter takes one and draws the
    /// highlight pre-pass ([`ClientWorld::take_hit_flashes`]).
    pub hit_flash: u8,
    /// The class's `monstats` `interact` flag, read when the hit flash
    /// is armed: the pre-pass colour is blue for it.
    pub hit_flash_interact: bool,
    /// A player's last mode request other than a 0x13 soft hit, with its
    /// `mode_requests` count: D2RUST bookkeeping (no base-game field). The
    /// base game resets the path and
    /// sets a mode for every code but 0x13, which only starts the hit
    /// overlay and sound: a soft hit in the same tick or later
    /// leaves the facing, path and skill sequence of the request before it.
    pub last_path_request: Option<(ModeRequest, u32)>,
    /// How many 0x15 placements moved the unit: D2RUST bookkeeping, so the
    /// view can tell a teleport (no turn in the base game) from a walk step.
    pub placements: u32,
    /// How many times the mode set restarted the unit's animation
    /// ([`super::monster_anim::mode_init`]): D2RUST
    /// bookkeeping, so the sound pass sees a restart whose mode changes
    /// cancel out within one frame (2 → 1 → 2).
    pub anim_restarts: u32,
    /// How many of those restarts changed the mode (returns 1
    /// only for a mode other than the unit's, and only then does
    /// run the mode sounds): D2RUST bookkeeping.
    pub mode_changes: u32,
    /// How many mode ends ([`super::monster_anim::mode_end`])
    /// of the unit's own update set a mode: D2RUST bookkeeping, so the sound
    /// pass can tell a walk the update ended (its path arrived) from one a
    /// server message stopped.
    pub mode_ends: u32,
    /// The path direction (+0x64) a monster of set S was created with
    /// (D2RUST bookkeeping: the model holds no
    /// client path record): 6 bits.
    pub path_dir: Option<u8>,
    /// A player's pending turn: the new direction and the turn step
    /// (path +0x65, +0x66) that `critter_path::face_point` set; the player
    /// update turns `path_dir` toward it.
    pub path_turn: Option<(u8, u8)>,
    /// D2RUST bookkeeping: the single-player copy already ran this tick's
    /// turn of a walking player ([`super::critter_path::player_aim`]), so
    /// the next player update skips its own (one turn per tick).
    pub copy_turned: bool,
    /// How many times `critter_path::face_point` turned this player
    /// (D2RUST bookkeeping: the walk prediction adopts a later turn as its
    /// facing, B025).
    pub faces: u32,
    /// The target the client skill start left on the unit's path: a
    /// point (0x15: path +0x10 / +0x12) or a unit (0x16: the path's
    /// target unit), read for the `cltmissile` create.
    pub cast_target: Option<CastTarget>,
    /// The player path's target point (+0x10 / +0x12): the point of the
    /// last walk to a point or skill start.
    /// D2RUST's own bookkeeping for the cstate dump.
    pub path_target: Option<(u16, u16)>,
    /// A walk to a unit: the path target follows the unit's point.
    pub path_follow: Option<UnitKey>,
    /// An S player's path step result (≠ 0):
    /// the single-player copy's path still moving with points
    /// left. `None`: no copy holds the path words.
    pub path_live: Option<bool>,
    /// The timed arc of a Leap Attack's takeoff:
    /// the client motion record whose
    /// height the do's landing test reads, stepped once per
    /// player update.
    pub leap_arc: Option<crate::rules::unit_composite::MotionRecord>,
    pub kind: KindData,
    /// The skill list (+0xA8); `None` = no
    /// list.
    pub skills: Option<SkillList>,
    /// Unit flag +0xC4 bit 0x2 cleared by S→C 0x5D.
    pub quest_untargetable: bool,
    /// Unit flag +0xC4 bit 0x2 as last written by a model rule: 0x5D
    /// clears it (with `quest_untargetable`), 0x28 and 0x62 set it.
    /// `None`: never written; the initial
    /// value per kind is an open question.
    pub flag_2: Option<bool>,
    /// The state bits that are on.
    pub states: BTreeSet<u8>,
    /// The unit palette index (+0x6C): set by
    /// the colour call only, which state on / off runs for a
    /// state with `colorshift` ≠ 0.
    pub palette_index: u8,
    /// The stat list of each state:
    /// (stat, param) → value, attached to the unit.
    pub state_lists: BTreeMap<u8, BTreeMap<(u16, u16), i32>>,
    /// The expire counter (in client unit updates) of each
    /// state list the client made itself with flag 2 (NEWLENGTH): the
    /// `cltstfunc` 15 list of state 12. The unit update's first step
    /// with (U, 0) takes one
    /// off each; at ≤ 0 the list is freed and its callback clears the
    /// state bit ([`expire_timed_lists`]).
    pub timed_lists: BTreeMap<u8, i32>,
    /// The stateless TEMPONLY lists (flags 4) the client attached to the
    /// unit, each (stat, param) → value: the attack-rate lists of
    /// `cltstfunc` 9 and 27,
    /// counted in the totals. Freed by the unit's next real
    /// mode change.
    pub temp_lists: Vec<BTreeMap<(u16, u16), i32>>,
    /// The unit this unit was last turned toward by a model rule
    /// (on its path toward that
    /// unit's position). The model holds no
    /// client path record, so the turn's input is kept, not the
    /// direction.
    pub turned_toward: Option<UnitKey>,
    /// (path flag 0x20 cleared, point count := 0) was called
    /// on the unit's path by a model rule.
    pub path_stopped: bool,
    /// The unit whose path direction this unit's path took
    /// (a corpse takes its player's). The model holds no client path
    /// record, so
    /// the source is kept, not the direction.
    pub direction_of: Option<UnitKey>,
    /// Unit flag `+0xC4` 0x800000: set only by the client room free.
    pub room_freed: bool,
    /// Unit flag `+0xC4` 0x200: set by the client hireling setup of 0x81;
    /// 0xAB skips such a unit.
    pub flag_200: bool,
    /// Unit flag `+0xC4` 0x100000 (a flat unit):
    /// set by a client object's `OrderFlag2` step.
    pub flag_flat: bool,
    /// Unit flag `+0xC4` 0x20: no shadow.
    /// Set up from `monstats2` `shadow` = 0 (class
    /// code 7); set when a monster's death
    /// animation (mode 0) is held at its last frame, and no
    /// other mode change clears it.
    pub flag_20: bool,
    /// +0xD4 (u32, 0 at creation): the interact stamp of a monster
    /// and the timer T of the client object functions.
    pub interact_ms: u32,
    /// +0x44: the animation frame (signed, 8.8 fixed point).
    pub frame: i32,
    /// Unit +0x38 >> 8: the frame event index.
    /// The play host writes the local player's; 0 else.
    pub event_index: u32,
    /// The precise path position (16.16 sub-tiles) of a walking monster,
    /// written by the client track ([`super::motion`]); the draw uses it
    /// while its cell is `position`. `None`: the cell centre.
    pub precise: Option<(u32, u32)>,
    /// +0x48: the animation's frame count (8.8), written by a monster's
    /// mode set ([`super::monster_anim::mode_set`]); 0 otherwise.
    pub frame_count: i32,
    /// +0x4C: an object's animation speed (8.8 per update), set by the
    /// animation set-up; `None`: no
    /// set-up ran, the generic step uses `FrameDelta[mode]`.
    pub speed: Option<i32>,
    /// +0xC8: flag-ex. Bit 0x2000000 := `expansion` ≠ 0 at creation;
    /// the other bits are written by the rules that own
    /// them (0x40000 cleared for a hireling in mode 1, 0x400 the source-unit
    /// link; 0x20 set by the client room free).
    pub flag_ex: u32,
    /// An item holds its ground overlay 70 (kind 6):
    /// created once by the item update and never ended
    /// by the advance, so later updates find it and draw
    /// nothing. Held as the server tick of the create and its start frame
    /// (+0x18, the a = 8 draw `roll(8 × 256)`), which the effect layer
    /// builds the record from (`world_view::missiles`).
    pub ground_overlay: Option<(u64, i32)>,
    /// The local player's model seed just before that create's draws: the
    /// effect layer maps it onto the shared client seed and redraws the
    /// start there.
    pub ground_overlay_seed: Option<(u32, u32)>,
    /// Unit flag +0xC4 bit 0x4, read by the interact sender
    /// and the hover's corpse test: a monster's is
    /// `monstats2` `isAtt` at set-up.
    pub flag_4: bool,
    /// +0xB0: the last hit class, written by the player
    /// mode machine (`+0xB0` :=); 0 at creation.
    pub hit_class: u32,
    /// Unit flag +0xC4 bit 0x40: the client do ran in this mode;
    /// cleared by the client mode set,
    /// set by the do.
    pub flag_40: bool,
    /// +0x4E: the event byte (1–4) of the frames the latest frame
    /// advance crossed, else 0 (the frame advance);
    /// written by a player's or monster's client update
    /// ([`super::player_anim`], [`super::monster_anim::advance`]).
    pub action_frame: u8,
    /// The AnimData event bytes of a player's or monster's current mode,
    /// set by its mode set ([`super::player_anim::mode_set`],
    /// [`super::monster_anim::mode_init`]); `None`: no record.
    pub anim_events: Option<[u8; d2_formats::animdata::EVENTS]>,
    /// The used skill a monster's 0x15 / 0x16 request set:
    /// the skill the sequence load of mode 14
    /// looks up. `None`: never set.
    pub used_skill: Option<i32>,
    /// The level the same request assigned the used skill at:
    /// the level of the monster's client do.
    pub used_skill_level: i32,
    /// A monster's animation sequence (+0x30…+0x40), loaded by the mode
    /// set of mode 14; `None`:
    /// no sequence (the frame steps by the speed).
    pub sequence: Option<MonsterSequence>,
}

/// The sequence fields of a client monster:
/// the frame list (+0x30), its length · 256 (+0x34), the position (+0x38,
/// 8.8), the step (+0x3C) and the drawn mode (+0x40, the mode of the
/// frame at the position). The drawn frame is the unit's `frame` (+0x44),
/// the count `frame_count` (+0x48), the event `action_frame` (+0x4E).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonsterSequence {
    pub frames: Vec<super::monster_anim::SeqFrame>,
    pub length: i32,
    pub pos: i32,
    pub step: i32,
    pub drawn_mode: u32,
}

/// The reserved `outgoing` slot of 0x28's dialog branch (,
/// open question 10 decided as A): C→S 0x31 of case B2 goes here,
/// right after the handler's 0x2F, so the bytes keep the base game's order.
pub const DIALOG_REPLY_SLOT: Vec<u8> = Vec::new();

impl ClientUnit {
    /// A unit with only its key: class 0, mode 0, not placed, no stats,
    /// seed {1, 666} (`init`), empty queue, no kind data.
    pub fn new(key: UnitKey) -> Self {
        Self {
            key,
            class: 0,
            mode: 0,
            position: None,
            server_point: (0, 0),
            follows: 0,
            stats: BTreeMap::new(),
            seed: Some(INIT_SEED),
            seed_sets: 0,
            queue: Vec::new(),
            last_mode_request: None,
            request_late: false,
            mode_requests: 0,
            last_cast: None,
            last_hit_overlay: None,
            hit_overlays: 0,
            hit_flash: 0,
            hit_flash_interact: false,
            last_path_request: None,
            placements: 0,
            anim_restarts: 0,
            mode_changes: 0,
            mode_ends: 0,
            path_dir: None,
            path_turn: None,
            copy_turned: false,
            faces: 0,
            cast_target: None,
            path_target: None,
            path_follow: None,
            path_live: None,
            leap_arc: None,
            kind: KindData::None,
            skills: None,
            quest_untargetable: false,
            flag_2: None,
            states: BTreeSet::new(),
            palette_index: 0,
            state_lists: BTreeMap::new(),
            timed_lists: BTreeMap::new(),
            temp_lists: Vec::new(),
            turned_toward: None,
            path_stopped: false,
            direction_of: None,
            room_freed: false,
            flag_200: false,
            flag_flat: false,
            flag_20: false,
            interact_ms: 0,
            frame: 0,
            event_index: 0,
            precise: None,
            frame_count: 0,
            speed: None,
            flag_ex: 0,
            ground_overlay: None,
            ground_overlay_seed: None,
            flag_4: false,
            hit_class: 0,
            flag_40: false,
            action_frame: 0,
            anim_events: None,
            used_skill: None,
            used_skill_level: 0,
            sequence: None,
        }
    }

    /// Base value of `stat` (0 when absent).
    pub fn stat(&self, stat: u16) -> i32 {
        self.stats.get(&stat).copied().unwrap_or(0)
    }

    /// The position as a cell, (0, 0) when not placed.
    pub fn cell(&self) -> (u16, u16) {
        self.position.unwrap_or((0, 0))
    }

    /// Dead: player mode 0 or 0x11, monster mode 0 or 0xC.
    /// PROVISIONAL (settled once the flag setter's and clear's callers
    /// are known): D2RUST keeps no flag 0x10000 and reads the mode alone, which
    /// differs from the base game only for a dead unit that a request moves to
    /// another mode without the clear.
    pub fn is_dead(&self) -> bool {
        match self.key.unit_type {
            PLAYER => matches!(self.mode, 0 | 0x11),
            MONSTER => matches!(self.mode, 0 | 0xC),
            _ => false,
        }
    }
}

/// `init` = {1, 666}.
pub const INIT_SEED: (u32, u32) = (1, 666);

/// The client DRLG act of 0x03.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActLoad {
    pub act: u8,
    pub init_seed: u32,
    pub town_level: u16,
    /// u32@8.
    pub f8: u32,
}

/// A session message the loading flow cares about, in arrival order
/// (L9, L10). D2RUST's own: the model keeps edges
/// only, so a repeated same-act 0x03 or a 0x05 / 0x04 pair inside one
/// frame would otherwise be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMark {
    /// S→C 0x03 of this act.
    LoadAct(u8),
    /// S→C 0x04.
    LoadComplete,
    /// S→C 0x05.
    Unload,
    /// S→C 0x61 with this video id.
    Video(u8),
}

/// Marks kept in [`ClientWorld::session_log`].
pub const SESSION_LOG_CAP: usize = 64;

/// One room-in-sight message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomSight {
    /// 0x07 (true) or 0x08 (false).
    pub show: bool,
    pub level: u8,
    pub x: u16,
    pub y: u16,
}

/// One active room of the client act: its sub-tile
/// rectangle (active room +0x4C, +0x50, +0x54, +0x58), the id of its
/// level and its DRLG room (active room +0x10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveRoom {
    pub x0: i32,
    pub y0: i32,
    pub w: i32,
    pub h: i32,
    pub level: u16,
    pub room: DrlgRoomId,
}

impl ActiveRoom {
    /// x0 ≤ x < x0 + w and y0 ≤ y < y0 + h (signed compares).
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.x0 <= x && x < self.x0 + self.w && self.y0 <= y && y < self.y0 + self.h
    }
}

/// The act lookup (b): the first active room in
/// list order whose rectangle contains (x, y).
pub fn room_of_point(rooms: &[ActiveRoom], x: i32, y: i32) -> Option<&ActiveRoom> {
    rooms.iter().find(|r| r.contains(x, y))
}

/// One record of the pet list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PetRecord {
    /// +0x00.
    pub class: u16,
    /// +0x04.
    pub pet_type: u8,
    /// +0x08.
    pub pet: u32,
    /// +0x0C.
    pub owner: u32,
    /// +0x1C: 100 at creation.
    pub f1c: u32,
    /// +0x20.
    pub gone: bool,
    /// +0x24…: the three values of 0x81; `None` until a 0x81
    /// sets them. +0x24 := u32@0xC (hire seed), +0x28 := u32@0x10 (hire
    /// id), +0x2C := 0 are the only writes past +0x20; +0x1C
    /// stays 100 and is read only by UI draws (an open question;
    /// recorded hire: `81 07 0f01 01000000 0d000000
    /// 083bd951 530d0000` gives seed 0x51D93B08, id 0x0D53).
    pub extra: Option<[u32; 3]>,
}

/// `PingState`: the fields 0x8F writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PingState {
    /// Last round trip in ms (0 until a 0x8F; D2RUST sends
    /// no ping, so it stays 0).
    pub rtt: u32,
    pub samples: u32,
    pub mean: u32,
    ///….
    pub pong: [u32; 8],
}

/// The pet type of a hireling.
pub const PET_HIRELING: u8 = 7;

/// The pet type the 0x75 pet pass visits.
pub const PET_TYPE_PASS: u8 = 4;

/// One player roster record (0xD8 bytes in
/// the base game, the UI handle +0x34 and the formatted string +0x66 are UI
/// fields and not held).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RosterRecord {
    /// +0x00 (16 bytes, NUL-terminated).
    pub name: [u8; 16],
    /// +0x10.
    pub guid: u32,
    /// +0x14: life percent.
    pub life: u32,
    /// +0x18: kill count.
    pub kills: i32,
    /// +0x1C.
    pub class: u32,
    /// +0x20, +0x22.
    pub f20: u16,
    pub f22: u16,
    /// +0x30.
    pub f30: u16,
    /// +0x38: the corpse list, head first.
    pub corpses: Vec<u32>,
    /// +0x3C, +0x40: portal GUIDs.
    pub portals: (u32, u32),
    /// +0x44.
    pub f44: u16,
    /// The bytes written from +0x46 on by the two string copies of 0x5B
    /// (string 1 at +0x46, string 2 at +0x4A, each with its NUL);
    /// unwritten bytes are 0.
    pub strings: Vec<u8>,
}

/// The use-item cursor of 0x3F.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UseCursor {
    pub item: UnitKey,
    pub code: u8,
}

/// The units of each client active room (the unit order): the room
/// list at active room +0x74 (head first, insert =
/// prepend, remove), the client's only link
/// from a unit to a room. A unit at (0, 0) or with no room is in no list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomUnits {
    lists: BTreeMap<DrlgRoomId, Vec<UnitKey>>,
    rooms: BTreeMap<UnitKey, DrlgRoomId>,
    /// Players whose recache changed their room since their last player
    /// update: path flag 0x2 and the previous room (path +0x20) the
    /// recache sets, read and
    /// cleared by the player update.
    changed: BTreeMap<UnitKey, Option<DrlgRoomId>>,
}

impl RoomUnits {
    /// The room's list, head first (empty for a room with no units).
    pub fn list(&self, room: DrlgRoomId) -> &[UnitKey] {
        self.lists.get(&room).map_or(&[], Vec::as_slice)
    }

    /// The room whose list holds `key`.
    pub fn room_of(&self, key: UnitKey) -> Option<DrlgRoomId> {
        self.rooms.get(&key).copied()
    }

    /// Unit leaves its room; not in a list:
    /// nothing.
    pub fn leave(&mut self, key: UnitKey) {
        let Some(room) = self.rooms.remove(&key) else {
            return;
        };
        if let Some(list) = self.lists.get_mut(&room) {
            list.retain(|&k| k != key);
            if list.is_empty() {
                self.lists.remove(&room);
            }
        }
    }

    /// The room recache (step 6): leave the old room, then
    /// insert at the head of `room`'s list when it is not none (step 2).
    /// A player whose room changes gets the room-change record
    /// ([`Self::take_change`]).
    pub fn place(&mut self, key: UnitKey, room: Option<DrlgRoomId>) {
        let old = self.room_of(key);
        self.insert(key, room);
        if key.unit_type == PLAYER && old != room {
            self.changed.insert(key, old);
        }
    }

    /// The creation insert (and the other creation sites of
    /// step 6): as [`Self::place`], but no room-change record (the path
    /// set-up leaves path flag 0x2 clear).
    pub fn insert(&mut self, key: UnitKey, room: Option<DrlgRoomId>) {
        self.leave(key);
        if let Some(room) = room {
            self.lists.entry(room).or_default().insert(0, key);
            self.rooms.insert(key, room);
        }
    }

    /// The player update's room-change test (then cleared):
    /// `Some(previous room)` when the player's room
    /// changed since the last call.
    pub fn take_change(&mut self, key: UnitKey) -> Option<Option<DrlgRoomId>> {
        self.changed.remove(&key)
    }

    /// The client room free (step 6 table): each unit of
    /// the room leaves it. Returns them in list order.
    pub fn free_room(&mut self, room: DrlgRoomId) -> Vec<UnitKey> {
        let units = self.lists.remove(&room).unwrap_or_default();
        for k in &units {
            self.rooms.remove(k);
        }
        units
    }

    /// The draw's Y sort written back (step 7): `order` is the room's list
    /// after the stable sort the draw ran on it. Refused (`false`, nothing
    /// changed) unless it holds exactly the list's units.
    pub fn set_order(&mut self, room: DrlgRoomId, order: &[UnitKey]) -> bool {
        let Some(list) = self.lists.get_mut(&room) else {
            return order.is_empty();
        };
        let mut a = list.clone();
        let mut b = order.to_vec();
        a.sort();
        b.sort();
        if a != b {
            return false;
        }
        list.copy_from_slice(order);
        true
    }
}

/// How far (sub-tiles per axis) a walk prediction may lead the model
/// position before it is treated as stale: a walk step is a few cells, a
/// server teleport is not.
pub const MAX_PREDICTION_CELLS: i32 = 32;

/// The play preview's walk prediction of the local player as the
/// position check sees it ([`ClientWorld::local_walk`]). D2RUST's own,
/// unverified. PROVISIONAL; settled by a capture of the client state
/// over a held walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalWalk {
    /// The model position when the cell was recorded: a placement since
    /// then wins over the cell.
    pub at: (u16, u16),
    /// The predicted precise (16.16) position.
    pub pos: (u32, u32),
    /// The mode the prediction moves the player in (2 walk, 3 run);
    /// `None`: standing, the model's mode stands.
    pub mode: Option<u32>,
}

impl LocalWalk {
    /// The predicted sub-tile: the high 16 bits of [`Self::pos`].
    pub fn cell(&self) -> (u16, u16) {
        ((self.pos.0 >> 16) as u16, (self.pos.1 >> 16) as u16)
    }
}

/// What the client reads of the last drawn frame's camera:
/// the origin getters = cx_u −
/// shiftX and cy_u − 8, the display width
/// = W and the play height = H − 40.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitOrigin {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub play_height: i32,
}

impl UnitOrigin {
    /// The getters of `cam`.
    pub fn of(cam: &crate::rules::camera::Camera) -> Self {
        UnitOrigin {
            x: cam.unit.x - cam.view.shift_x,
            y: cam.unit.y - 8,
            width: cam.size.width,
            play_height: cam.size.play_height(),
        }
    }
}

/// A screen shake started by a model rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShakeStart {
    pub shake: crate::rules::camera::Shake,
    pub start_tick: u64,
}

/// The monster graphics-ready flags of a drawn frame
/// (the graphics-ready test):
/// by `monstats` class, bit m set when mode m is ready. A host input:
/// the preload scan sets the flags on the
/// host's wall clock and on async loads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GfxReady {
    pub masks: BTreeMap<u32, u32>,
}

impl ClientWorld {
    /// The draw mark of U at the start of each unit draw:
    /// every unit in `drawn` (the
    /// units the drawn frame called the unit draw for, once each) with a
    /// non-zero hit flash counter takes one from it.
    pub fn take_hit_flashes(&mut self, drawn: &[UnitKey]) {
        for key in drawn {
            if let Some(u) = self.units.get_mut(key) {
                u.hit_flash = u.hit_flash.saturating_sub(1);
            }
        }
    }

    /// The graphics-ready flags of the draw that follows this tick's
    /// update: the recorded frame of the latest drawn tick at or before
    /// [`Self::server_ticks`] (a tick without a frame keeps the flags of
    /// the frame before). `None`: not recorded, or no frame yet.
    pub fn gfx_ready_now(&self) -> Option<&GfxReady> {
        let map = self.gfx_ready.as_ref()?;
        map.range(..=self.server_ticks).next_back().map(|(_, r)| r)
    }
}

impl GfxReady {
    /// The flag of `class` in `mode` (a class or mode not listed: 0).
    pub fn ready(&self, class: u32, mode: u32) -> bool {
        mode < 32 && self.masks.get(&class).is_some_and(|m| m & (1 << mode) != 0)
    }
}

/// The client world model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientWorld {
    /// Session messages seen so far (count, never reset).
    pub session_total: u64,
    /// The latest authority view acknowledgement: sequence, width, height.
    pub view_ready: Option<(u32, u32, u32)>,
    /// Drawn tick → the graphics-ready flags at the frame's start, where a
    /// check run recorded them; `None`: not
    /// recorded (living monsters draw as ready, the dead snap's write-back
    /// as not ready). A handle like `item_tables`.
    pub gfx_ready: Option<std::sync::Arc<BTreeMap<u64, GfxReady>>>,
    /// The last [`SESSION_LOG_CAP`] of them, oldest first.
    pub session_log: Vec<SessionMark>,
    /// Bridge frames run.
    pub frames: u64,
    /// The item tables the streams are decoded with
    /// (`bridge::item_lists`).
    pub item_tables: super::item_lists::ItemTablesRef,
    /// Per class: charstats life per vitality, stamina per vitality, mana
    /// per energy (`class_bonus`).
    pub class_vitals: Vec<[u8; 3]>,
    /// Per stat: its op-target table entries(T) as (source stat, op)
    /// (`op_bonus`).
    pub stat_ops: Vec<Vec<(u16, u8)>>,
    /// The animation lookup the item tips' speed line queries;
    /// a handle like `item_tables`.
    pub tip_anims: super::player_anim::PlayerAnimsRef,
    /// D2RUST's own, unverified: S→C 0x9C action 0x0B records received (the
    /// store items a trade open shows).
    pub store_serial: u32,
    /// D2RUST's own: the order the state bits were turned on in (a counter per
    /// that set a new bit). The state sounds of a unit new to
    /// the sound layer follow it: the base game requests them in packet order, at
    /// unit creation.
    pub state_on_seq: BTreeMap<(UnitKey, u8), u64>,
    pub state_on_next: u64,
    /// Bridge frames whose pump ran a server tick: each is one step of the
    /// client model.
    pub server_ticks: u64,
    /// The game's tick rate ([`super::link::ServerLink::tick_rate`]), set
    /// when the bridge is built. The client model steps once per server
    /// tick, so its per-tick amounts (walk steps, path velocities) are
    /// shares of the tables' base-rate amounts at this rate, as in the
    /// simulation.
    pub tick_rate: d2_sim::tick::TickRate,
    /// Authoritative game frame after the last pump, when the link exposes it.
    pub server_frame: Option<i32>,
    /// The script's clicks of this pass reach the model after the client
    /// update that the base game would have run them before (`present::script_tick`
    /// shift): a click's mode request is followed by that update's player
    /// step ([`super::click::apply`]).
    pub late_input: bool,
    /// The tables of the hover's corpse test ([`super::hover::CorpseTables`]),
    /// kept in step with the bridge's tables.
    pub corpse_tables: super::hover::CorpseTables,
    /// The flag byte of every 0x15 that placed the local player, in order
    /// (each runs with (flag), whose
    /// weather side the weather view replays on the shared client seed).
    pub local_places: Vec<u8>,
    /// How many units reached the placement's teleport step:
    /// the automap's reveal countdown
    /// resets on each.
    pub placed_units: u32,
    /// Set S: the units the server announced, in key order.
    pub units: BTreeMap<UnitKey, ClientUnit>,
    pub local_player: Option<UnitKey>,
    /// The local player's predicted precise (16.16) position, set by the
    /// play preview's walk prediction each frame (D2RUST bookkeeping,
    /// `None` without one). The model's own position of the local player
    /// is the last point the server gave it, which the walk leaves behind;
    /// what the base-game client unit's own position is. Read where a unit
    /// faces the local player or the local player faces a target (B025).
    pub local_predicted: Option<(u32, u32)>,
    /// The server monsters that faced the local player in this frame's
    /// update pass, with the new direction the face gave them (D2RUST
    /// bookkeeping). The base game steps the local player in its player update,
    /// before the monsters; the play preview's walk runs after the bridge
    /// frame, so these faces saw last tick's position and are redone with
    /// the fresh one ([`super::critter_path::refresh_local_faces`]).
    pub local_faces: Vec<(UnitKey, u8)>,
    /// The NPC whose dialog the UI layer last opened (the interact NPC):
    /// the holder of the store
    /// items shown by 0x9C action 11 (their +0x5C inventory is the NPC's).
    pub interact_npc: Option<UnitKey>,
    /// Monster classes whose `palshift` block is loaded: an S→C 0xAC of
    /// the class arrived (called only by the 0xAC handler;
    /// class `palshift` data). Client-only
    /// monsters never add to it.
    pub palshift_classes: BTreeSet<u32>,
    pub difficulty: u8,
    pub expansion: u32,
    pub ladder: u8,
    pub game_flags: u32,
    pub act: Option<ActLoad>,
    pub in_game: bool,
    pub unloaded: bool,
    pub exit_requested: bool,
    /// `connected`: 0xAF sets, 0xB0 clears.
    pub connected: bool,
    /// The ping state, written by 0x8F.
    pub ping: PingState,
    /// Palette level of pet monsters set by the 0x75 pet pass (render
    /// state; measured 2026-10-15: the raised
    /// skeleton's rows match).
    pub pet_palette: BTreeMap<UnitKey, u8>,
    /// The belt column-ready bytes + c]`,
    /// written only by 0x9C 0x0E, 0x0F and
    /// 0x15 mode 2.
    pub belt_ready: [bool; 4],
    pub rooms_in_sight: Vec<RoomSight>,
    /// C→S messages the client sends on its own,
    /// until the bridge hands them to its send path. An empty entry is a
    /// reserved slot ([`DIALOG_REPLY_SLOT`]): nothing after it is sent
    /// until the UI's answer fills or clears it.
    pub outgoing: Vec<Vec<u8>>,
    /// The play preview's own-walk cell of the local player, with the
    /// model position it was recorded against ([`Self::set_local_walk`],
    /// read by [`Self::predicted`]). `None`: no prediction (the strict
    /// path, synthetic data without speeds).
    pub local_walk: Option<LocalWalk>,
    /// How many `dir64` steps the local path direction turns a client
    /// update: 4 after a walk started from standing, 1 while it walks on.
    pub local_turn: u8,
    /// 0x3F's use-item cursor.
    pub use_cursor: Option<UseCursor>,
    /// The pet list, newest first.
    pub pets: Vec<PetRecord>,
    /// S→C 0x03 act loads handled (count, never reset): each one replaces
    /// the client act through, which sets the post-draw clear
    /// counter.
    pub act_loads: u64,
    /// The act whose palette is loaded (4): the act of 0x03,
    /// replaced by the Levels `Pal` of the new level on a room change.
    pub palette_act: Option<u8>,
    /// The client DRLG act: built by 0x03
    /// from [`ModelInputs::drlg`], rooms set in sight by 0x07 / 0x08.
    /// `None` before 0x03 or without a DRLG source.
    pub drlg: Option<ClientDrlg>,
    /// The active rooms of the client act in list order,
    /// derived from `drlg` and refreshed by the handlers that change it
    /// (0x03, 0x07, 0x08). `None`: no client DRLG (no DRLG source: the
    /// headless configuration of `ModelInputs::default`), so a placement
    /// is taken as in a room and the level is unknown.
    pub active_rooms: Option<Vec<ActiveRoom>>,
    /// The client room unit lists.
    pub room_units: RoomUnits,
    /// The client's light list. Its records
    /// are created by unit code no model rule runs yet; the act room
    /// callback runs over it for every new active
    /// room.
    pub lights: LightList,
    /// Each unit's cast light, held in its stat list,
    /// apart from the unit's own light
    /// (`+0x64`, e.g. the player light): the mode requests and the client
    /// skill end detach and remove only this one. No model rule creates
    /// one yet.
    pub cast_lights: BTreeMap<UnitKey, crate::rules::lighting::records::LightId>,
    /// The lit overlay records of every unit, newest first (each unit's
    /// list order; [`super::overlay_lights`]).
    pub overlay_lights: Vec<super::overlay_lights::OverlayLight>,
    /// The update mark of the overlay walk:
    /// one more per [`super::overlay_lights::advance`].
    pub overlay_stamp: u64,
    /// The hit overlay creates not yet taken by the effect layer, in
    /// order ([`super::hit_overlays`]).
    pub hit_overlays: Vec<super::hit_overlays::HitOverlay>,
    /// The unit origin of the last drawn frame,
    /// read by the client missile function 2;
    /// set by the play app on each drawn
    /// frame (`Bridge::set_unit_origin`), `None` before the first.
    pub unit_origin: Option<UnitOrigin>,
    /// The running screen shake (one at a time,
    /// restarts it), started on server tick `start_tick` (the
    /// D2RUST time base); `None` before the first.
    pub shake: Option<ShakeStart>,
    /// Client updates that ran the DRLG part
    /// (last paragraph: += 1 first, level free on every
    /// multiple of 13).
    pub drlg_updates: u32,
    /// The local player's skill-use lock:
    /// set by
    /// the client do tail to C + min(delay, 12),
    /// cleared by setfunc 19 / remfunc 12; while
    /// [`Self::drlg_updates`] is below it the use check reads 8.
    pub skill_lock: u32,
    /// The allied count (+0x28) of each client room as stat 172 changes
    /// it.
    pub room_allied: BTreeMap<DrlgRoomId, i32>,
    /// The environment record of the client act (act +0x04):
    /// created with the act by 0x03,
    /// set by 0x53.
    pub environment: Option<Environment>,
    /// The day-period cache of the 0x53 object refresh;
    /// a zero-initialised global.
    pub env_period_cache: i32,
    /// The eclipse pending flag:
    /// set by 0x5D without a client act.
    pub eclipse_pending: bool,
    /// The skill-tree flag: `Some(0)` once 0x21 cleared it;
    /// no other model rule writes it
    /// (open question 3).
    pub skill_tree_flag: Option<u32>,
    /// The scripted ambient override globals,
    /// written by S→C 0x89.
    pub overrides: Overrides,
    /// The 37 bytes of the last S→C 0x5E (the UI layer's, with a
    /// received flag set once one arrived), held by the client for the
    /// light rules that read client quest byte 1 (the monster light's
    /// override state). `None` until the first 0x5E.
    pub quest_availability: Option<[u8; 37]>,
    /// The client quest state the level-entry check reads:
    /// the player's flag record P (S→C
    /// 0x28 type 6), the game's G (0x29), the status list
    /// S (0x52) and the counters D, Y, B of 0x50 type 1. `None` until the
    /// message arrives (the UI layer keeps its own copy).
    pub quest_player: Option<[u8; 96]>,
    pub quest_game: Option<[u8; 96]>,
    pub quest_status: Option<[u8; 41]>,
    pub quest_counters: [i32; 3],
    /// Active rooms created since the last client update's lighting part,
    /// for the Den lights of a level-8 room loaded after the Den flag.
    pub rooms_loaded: Vec<DrlgRoomId>,
    /// The active player roster, head first.
    pub roster: Vec<RosterRecord>,
    /// The inactive roster, head first.
    pub roster_inactive: Vec<RosterRecord>,
    /// The active weapon set (0 / 1) of the local player.
    pub weapon_set: u8,
    /// The runtime item table `[0x0096CA9C]` as S→C 0xA6 writes it:
    /// entries of 0x120 bytes. Entries
    /// built at load are not held (open question 7).
    pub item_table_ext: Vec<Vec<u8>>,
    /// Set C and the client latches of the object functions.
    pub objclient: ClientObjects,
    /// Units freed since the receive path last moved them out, in free
    /// order: each becomes one `UnitFreed` output, with whether it was
    /// a client-only unit (set C).
    pub freed: Vec<(UnitKey, bool)>,
}

impl ClientWorld {
    /// Records a session message for the loading flow.
    pub fn mark_session(&mut self, m: SessionMark) {
        self.session_total += 1;
        self.session_log.push(m);
        if self.session_log.len() > SESSION_LOG_CAP {
            self.session_log.remove(0);
        }
    }

    /// The local player unit, if the model has one.
    pub fn local(&self) -> Option<&ClientUnit> {
        self.units.get(&self.local_player?)
    }

    /// Records the play preview's predicted precise position of the local
    /// player and the mode it moves in ([`Self::local_walk`]); `None`
    /// clears it.
    pub fn set_local_walk(&mut self, pos: Option<(u32, u32)>, mode: Option<u32>) {
        let at = self.local().and_then(|u| u.position);
        self.local_walk = at.zip(pos).map(|(at, pos)| LocalWalk { at, pos, mode });
    }

    /// Whether a walk prediction is stale: further than
    /// [`MAX_PREDICTION_CELLS`] from the local player's model cell, which no
    /// walk step is. A server teleport or an act load moved the player and
    /// the predictor has not followed; the server's placement is
    /// authoritative then.
    pub fn is_stale_prediction(&self, pos: (u32, u32)) -> bool {
        let Some((ax, ay)) = self.local().and_then(|u| u.position) else {
            return false;
        };
        let (x, y) = ((pos.0 >> 16) as i32, (pos.1 >> 16) as i32);
        (x - i32::from(ax)).abs() > MAX_PREDICTION_CELLS
            || (y - i32::from(ay)).abs() > MAX_PREDICTION_CELLS
    }

    /// The local player `unit`'s walk prediction, while the model
    /// position is still the one it was recorded against (a placement
    /// since then wins). The position check's step 8 reads it
    /// (`bridge::check`).
    pub fn predicted(&self, unit: &ClientUnit) -> Option<LocalWalk> {
        self.local_walk
            .filter(|w| self.local_player == Some(unit.key) && unit.position == Some(w.at))
    }

    /// The local player's own precise (16.16) position this frame:
    /// the walk prediction
    /// while it stands ([`Self::predicted`]), else the model cell's
    /// centre. Every reader of "the local
    /// player's position" between placements reads this one value: the
    /// draw, the click and hover cameras, labels, the automap, the near
    /// rooms and the interaction distances.
    /// `None`: no local player or no position.
    pub fn local_position(&self) -> Option<(u32, u32)> {
        let p = self.local()?;
        if let Some(w) = self.predicted(p) {
            return Some(w.pos);
        }
        let (x, y) = p.position?;
        Some(((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000))
    }

    /// The sub-tile of [`Self::local_position`].
    pub fn local_cell(&self) -> Option<(u16, u16)> {
        self.local_position()
            .map(|(x, y)| ((x >> 16) as u16, (y >> 16) as u16))
    }

    /// Adds `unit`: an existing unit with the same key is
    /// removed first (step 5: its queue is dropped, and it stops being the
    /// local player).
    pub fn add(&mut self, unit: ClientUnit) {
        self.remove(unit.key);
        self.units.insert(unit.key, unit);
    }

    /// Removes the unit `key`: its queue is dropped
    /// unapplied; the local player is cleared if it was that unit; the
    /// free is recorded for its `UnitFreed` output.
    /// A key not in the set: nothing.
    pub fn remove(&mut self, key: UnitKey) -> Option<ClientUnit> {
        let unit = self.units.remove(&key)?;
        self.objclient.s_paths.remove(&key);
        self.freed.push((key, false));
        // The free removes the unit's light first (the same body as the
        // light remove).
        let owner = Owner {
            unit_type: u32::from(key.unit_type),
            guid: key.guid,
            client_only: false,
        };
        let lit: Vec<_> = self
            .lights
            .iter()
            .filter(|(_, r)| r.owner() == Some(owner))
            .map(|(id, _)| id)
            .collect();
        for id in lit {
            let _ = self.lights.remove(id);
        }
        self.cast_lights.remove(&key);
        super::overlay_lights::forget(self, key);
        // The unit free leaves the room list.
        self.room_units.leave(key);
        self.room_units.take_change(key);
        if self.local_player == Some(key) {
            self.local_player = None;
        }
        Some(unit)
    }

    /// The hireling GUID of `player` (the pet lookup with (player, 7,
    /// 1)): the first pet record in list order with type 7 and owner
    /// `player` (gone records included); none → −1.
    pub fn hireling_guid(&self, player: Option<UnitKey>) -> u32 {
        let Some(p) = player else {
            return u32::MAX;
        };
        self.pets
            .iter()
            .find(|r| r.pet_type == PET_HIRELING && r.owner == p.guid)
            .map_or(u32::MAX, |r| r.pet)
    }

    /// The local player's room (on its position); `None` with
    /// no client DRLG, no local player or an unplaced one.
    pub fn local_room(&self) -> Option<&ActiveRoom> {
        self.unit_room(self.local_player?)
    }

    /// Phase shared by simulation and prediction, including signed frame wrap.
    pub fn share_frame(&self) -> i32 {
        self.server_frame
            .unwrap_or_else(|| (self.server_ticks % u64::from(self.tick_rate.hz())) as i32)
    }

    /// With (A, t1, t2, t3): a shake with
    /// peak A, attack t1, sustain t2 and release t3 (ms) starts now,
    /// replacing a running one; t2 = 0 starts nothing.
    pub fn start_shake(&mut self, peak: u32, attack: u32, sustain: u32, release: u32) {
        if let Some(shake) = crate::rules::camera::Shake::start(peak, attack, sustain, release) {
            self.shake = Some(ShakeStart {
                shake,
                start_tick: self.server_ticks,
            });
        }
    }

    /// Room of a point for a point
    /// other than (0, 0): the lookup of [`ClientWorld::room_from`] from
    /// the local player's room. `None` when no room is found (fatal 0x13C
    /// for the caller) or with no client DRLG.
    pub fn room_at(&self, x: u16, y: u16) -> Option<ActiveRoom> {
        self.room_from(self.local_room(), x, y)
    }

    /// (a) The cell lookup from `start` (that room if it contains the
    /// point, else the first room of its
    /// adjacency array that does), then (b) the act lookup over the
    /// active rooms in list order. `None` when neither finds
    /// one or with no client DRLG.
    pub fn room_from(&self, start: Option<&ActiveRoom>, x: u16, y: u16) -> Option<ActiveRoom> {
        let rooms = self.active_rooms.as_deref()?;
        let (x, y) = (i32::from(x), i32::from(y));
        if let Some(found) = start.and_then(|own| self.cell_lookup(own, x, y)) {
            return Some(found);
        }
        room_of_point(rooms, x, y).copied()
    }

    /// The play preview's own-walk room recache (the walk prediction): the local
    /// player moved by the client's walk prediction to sub-tile `(x, y)`
    /// is linked to the room of that point (from its current
    /// room, or the act lookup when it has none) through the room recache
    /// of the unit order, as the base game's
    /// client path step does. The model's `position`
    /// is not written (the prediction keeps it). No room at the point
    /// (not in sight yet) or the same room: unchanged, `false`.
    ///
    /// D2RUST's own, unverified. PROVISIONAL (an open question); settled
    /// by a capture of the client state over a held walk.
    pub fn recache_local_room(&mut self, x: u16, y: u16) -> bool {
        let Some(key) = self.local_player else {
            return false;
        };
        if !self.units.contains_key(&key) {
            return false;
        }
        let current = self.local_room().copied();
        let Some(found) = self.room_from(current.as_ref(), x, y) else {
            return false;
        };
        if current.is_some_and(|c| c.room == found.room) {
            return false;
        }
        self.room_units.place(key, Some(found.room));
        true
    }

    /// The room of a unit: the active room whose unit list holds it
    /// (the client's only unit → room
    /// link); `None` when it is in no list or with no client DRLG.
    pub fn unit_room(&self, key: UnitKey) -> Option<&ActiveRoom> {
        let room = self.room_units.room_of(key)?;
        self.active_rooms
            .as_deref()?
            .iter()
            .find(|r| r.room == room)
    }

    /// The client active-room free:
    /// each unit linked in the room leaves it;
    /// flags-2 0x20 (server units: no flag 0x400000) and flag 0x800000
    /// are set on it.
    pub fn free_active_room(&mut self, room: DrlgRoomId) {
        for k in self.room_units.free_room(room) {
            if let Some(u) = self.units.get_mut(&k) {
                u.flag_ex |= 0x20;
                u.room_freed = true;
                // A dynamic-path unit (player,
                // monster, missile) has its precise and client x, y and its
                // point count zeroed.
                if matches!(k.unit_type, 0 | 1 | 3) {
                    u.position = Some((0, 0));
                    u.precise = Some((0, 0));
                }
            }
            for paths in [&mut self.objclient.s_paths, &mut self.objclient.c_paths] {
                if let Some(p) = paths.get_mut(&k) {
                    p.precise_x = 0;
                    p.precise_y = 0;
                    p.point_count = 0;
                    p.update_client();
                }
            }
        }
    }

    /// Refreshes [`ClientWorld::active_rooms`] from the client DRLG after
    /// a DRLG change: the units of a room that is no longer active leave
    /// its list (the client room free),
    /// then the act room callback runs for each new active room in
    /// creation order (:
    /// the light cache). The callbacks run after the whole change:
    /// the callback reads only the owners' rooms (unchanged by a DRLG
    /// change) and the cell lookup from them, whose answer for the new
    /// room is the same once the change ends (rooms do not overlap, and
    /// a set-in-sight or a timed build removes no active room).
    pub fn refresh_active_rooms(&mut self) -> Result<(), LightError> {
        let old = self.active_rooms.take().unwrap_or_default();
        self.active_rooms = self.drlg.as_ref().map(ClientDrlg::active_rooms);
        let gone: Vec<DrlgRoomId> = {
            let now = self.active_rooms.as_deref().unwrap_or(&[]);
            old.iter()
                .map(|r| r.room)
                .filter(|&r| !now.iter().any(|n| n.room == r))
                .collect()
        };
        // Each unit of a freed room (server units:
        // no flag 0x400000) gets flags-2 0x20, then flag 0x800000, then
        // leaves the room.
        for &room in &gone {
            self.free_active_room(room);
        }
        // A client-only unit (set C) left in a freed room
        // carries flag 0x800000 and is removed by the client update that
        // follows. The room is the one whose rectangle holds
        // the unit's cell.
        let freed: Vec<UnitKey> = self
            .objclient
            .set_c
            .values()
            .filter(|u| {
                u.position.is_some_and(|(x, y)| {
                    old.iter()
                        .any(|r| gone.contains(&r.room) && r.contains(i32::from(x), i32::from(y)))
                })
            })
            .map(|u| u.key)
            .collect();
        for key in freed {
            super::objects::remove_client_unit(self, key);
        }
        let created = self
            .drlg
            .as_mut()
            .map_or_else(Vec::new, ClientDrlg::take_created);
        self.rooms_loaded.extend(created.iter().copied());
        let mut lights = std::mem::take(&mut self.lights);
        let mut result = Ok(());
        for room in created {
            if let Err(e) = lights.room_created(light_room(room), &*self) {
                result = Err(e);
                break;
            }
        }
        self.lights = lights;
        result
    }

    /// The cell lookup with (room, x, y):
    /// `start` itself if it contains the point, else the first
    /// room of its adjacency array that does; none otherwise.
    pub fn cell_lookup(&self, start: &ActiveRoom, x: i32, y: i32) -> Option<ActiveRoom> {
        let rooms = self.active_rooms.as_deref()?;
        if start.contains(x, y) {
            return Some(*start);
        }
        let adjacency = self
            .drlg
            .as_ref()
            .map_or_else(Vec::new, |d| d.adjacency(start.room));
        adjacency
            .iter()
            .filter_map(|&n| rooms.iter().find(|r| r.room == n))
            .find(|r| r.contains(x, y))
            .copied()
    }

    /// `base(unit, stat, layer)`:
    /// the unit's base value. The model holds the layer-0
    /// base array (`ClientUnit::stats`); every base write is layer 0, so
    /// another layer has no entry (0). A unit not in S reads 0 (a null
    /// unit).
    pub fn base(&self, key: UnitKey, stat: u16, layer: u16) -> i32 {
        match self.units.get(&key) {
            Some(u) if layer == 0 => u.stat(stat),
            _ => 0,
        }
    }

    /// `total(unit, stat, layer)`:
    /// the full array of the unit's list: the base plus the
    /// lists attached to the unit (the state lists of 0xA7–0xAA
    /// and the passive-state lists, `ClientUnit::state_lists`, each
    /// propagating its stats into the owner's full array) and the lists
    /// of the attached items
    /// (`bridge::item_lists`).
    pub fn total(&self, key: UnitKey, stat: u16, layer: u16) -> i32 {
        let Some(u) = self.units.get(&key) else {
            return 0;
        };
        let t = u
            .state_lists
            .values()
            .chain(&u.temp_lists)
            .filter_map(|l| l.get(&(stat, layer)))
            .fold(self.base(key, stat, layer), |a, &v| a.wrapping_add(v))
            .wrapping_add(self.item_lists_total(key, stat, layer));
        t.wrapping_add(self.class_bonus(key, u.class, stat, layer))
            .wrapping_add(self.op_bonus(key, stat, t))
    }

    /// The percent ops of the op loop (ops 1 and 11)
    /// on a player's or monster's full array: prev = the sum `v`; for each
    /// entry of entries(T) in order with op 1 (guard: the list has an
    /// owner, true for a unit's list) or op 11 (guard: prev ≠ 0), r := the
    /// total of the entry's source stat at layer 0, and r ≠ 0 adds
    /// MulDiv(prev, r, 100). Increased Stamina's stat 163 (op 1 →
    /// maxstamina) is one. Ops 8 / 9 are `class_bonus`; the per-level,
    /// by-time and item-base ops (2–7, 13) are not applied here.
    fn op_bonus(&self, key: UnitKey, stat: u16, prev: i32) -> i32 {
        if !matches!(key.unit_type, PLAYER | MONSTER) || prev == 0 {
            return 0;
        }
        let Some(entries) = self.stat_ops.get(usize::from(stat)) else {
            return 0;
        };
        entries
            .iter()
            .take(16)
            .filter(|&&(_, op)| matches!(op, 1 | 11))
            .fold(0i32, |a, &(source, _)| match self.total(key, source, 0) {
                0 => a,
                r => a.wrapping_add(percent_of(prev, r)),
            })
    }

    /// The charstats bonus of a player's full array
    /// (`CharstatManaBonus` / `CharstatVitBonus`): max life, max mana
    /// and max stamina gain `per × d << 6`, d = the attached lists' change
    /// of vitality (life, stamina) or energy (mana).
    fn class_bonus(&self, key: UnitKey, class: u32, stat: u16, layer: u16) -> i32 {
        if key.unit_type != PLAYER || layer != 0 {
            return 0;
        }
        let Some(v) = self.class_vitals.get(class as usize) else {
            return 0;
        };
        // [life per vitality, stamina per vitality, mana per energy]
        let (source, per) = match stat {
            7 => (3, v[0]),
            11 => (3, v[1]),
            9 => (1, v[2]),
            _ => return 0,
        };
        let Some(u) = self.units.get(&key) else {
            return 0;
        };
        let d = u
            .state_lists
            .values()
            .chain(&u.temp_lists)
            .filter_map(|l| l.get(&(source, 0)))
            .fold(0i32, |a, &v| a.wrapping_add(v))
            .wrapping_add(self.item_lists_total(key, source, 0));
        i32::from(per).wrapping_mul(d) << 6
    }

    /// The roster lookup with (GUID):
    /// GUID −1 → none; else the first active record with that GUID or
    /// holding it in its corpse list.
    pub fn roster_find(&self, guid: u32) -> Option<usize> {
        if guid == u32::MAX {
            return None;
        }
        self.roster
            .iter()
            .position(|r| r.guid == guid || r.corpses.contains(&guid))
    }

    /// The local player's level (5): the level id of its
    /// room; none while it has no room.
    pub fn player_level(&self) -> Option<u16> {
        self.local_room().map(|r| r.level)
    }

    /// Client quest byte `i`: `A[i]` of the last S→C
    /// 0x5E; `None` while no 0x5E has
    /// arrived (= 0, where the base game is fatal 0x60).
    pub fn client_quest_byte(&self, i: usize) -> Option<u8> {
        self.quest_availability.and_then(|a| a.get(i).copied())
    }

    /// The level of the active room holding `key` (its room list);
    /// `None` without one.
    pub fn unit_level(&self, key: UnitKey) -> Option<u16> {
        let room = self.room_units.room_of(key)?;
        self.active_rooms
            .as_deref()?
            .iter()
            .find(|r| r.room == room)
            .map(|r| r.level)
    }
}

/// The light code's room handle of a DRLG room (one active room per
/// DRLG room at a time).
pub fn light_room(room: DrlgRoomId) -> RoomId {
    room.0
}

/// The client world as the light code's new-room rule reads it:
/// owners of set S in `units`, of set C
/// (client-only) in `objclient.set_c`.
impl LightRooms for ClientWorld {
    fn owner_position(&self, owner: &Owner) -> Option<(i32, i32)> {
        if let Some((x, y)) = self.client_missile_pos(owner) {
            return Some((x as i32, y as i32));
        }
        let (x, y) = self.light_owner(owner)?.cell();
        let centre = |c: u16| (i32::from(c) << 16) | 0x8000;
        Some((centre(x), centre(y)))
    }

    fn owner_subtile(&self, owner: &Owner) -> Option<(i32, i32)> {
        if let Some((x, y)) = self.client_missile_pos(owner) {
            return Some(((x >> 16) as i32, (y >> 16) as i32));
        }
        let (x, y) = self.light_owner(owner)?.cell();
        Some((i32::from(x), i32::from(y)))
    }

    fn owner_room(&self, owner: &Owner) -> Option<RoomId> {
        let u = self.light_owner(owner)?;
        if owner.client_only {
            // A client missile is in its path room's list
            // (`client_missiles::relink`).
            if let Some(r) = self.room_units.room_of(view_key(u.key)) {
                return Some(light_room(r));
            }
            // D2RUST's own, unverified: the model links the other set-C units
            // into no room list; their room is the active room of their
            // point (the room `objects::create_client_unit` placed them by).
            let (x, y) = u.cell();
            let rooms = self.active_rooms.as_deref()?;
            return room_of_point(rooms, i32::from(x), i32::from(y)).map(|r| light_room(r.room));
        }
        self.room_units.room_of(u.key).map(light_room)
    }

    fn cell_room(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        let start = self
            .active_rooms
            .as_deref()?
            .iter()
            .find(|r| light_room(r.room) == room)?;
        self.cell_lookup(start, x, y).map(|r| light_room(r.room))
    }
}

impl ClientWorld {
    /// A unit the view draws: set S, else a client-made monster or
    /// missile of set C under its view key ([`view_key`];: the
    /// critters draw like any monster, through their room's unit list).
    /// The two sets number their GUIDs apart (set C starts at 2, set S
    /// monsters at 1), so the view tells them apart by the key.
    pub fn view_unit(&self, key: &UnitKey) -> Option<std::borrow::Cow<'_, ClientUnit>> {
        if key.guid & VIEW_CLIENT_BIT == 0 {
            return self.units.get(key).map(std::borrow::Cow::Borrowed);
        }
        let real = UnitKey::new(key.unit_type, key.guid & !VIEW_CLIENT_BIT);
        let mut u = self
            .objclient
            .set_c
            .get(&real)
            .filter(|u| matches!(u.key.unit_type, MONSTER | OBJECT | MISSILE))?
            .clone();
        if u.key.unit_type == MISSILE {
            // A client missile draws at its path's precise position
            // (the path is the record the
            // bridge steps, `client_missiles::ClientMissile::pos`), or
            // where its pixel pair was last computed (`drawn_at`).
            let m = self.objclient.missiles.get(&real)?;
            u.position = Some(((m.pos.0 >> 16) as u16, (m.pos.1 >> 16) as u16));
            u.precise = Some(m.drawn_at.unwrap_or(m.pos));
        }
        u.key = *key;
        Some(std::borrow::Cow::Owned(u))
    }

    /// The path direction (+0x64) a view unit draws with, when the model
    /// holds its path: the C monsters' ([`super::critter_path`]) and the
    /// client missiles' (`ClientMissile::direction`).
    pub fn view_direction(&self, key: &UnitKey) -> Option<u8> {
        if key.guid & VIEW_CLIENT_BIT == 0 {
            return None;
        }
        let real = UnitKey::new(key.unit_type, key.guid & !VIEW_CLIENT_BIT);
        if key.unit_type == MISSILE {
            return self.objclient.missiles.get(&real).map(|m| m.direction);
        }
        super::critter_path::direction(self, real)
    }

    /// Every unit the view draws: set S in key order, then the
    /// client-made monsters, objects and missiles of set C
    /// ([`Self::view_unit`]).
    pub fn view_units(&self) -> Vec<std::borrow::Cow<'_, ClientUnit>> {
        let c = self
            .objclient
            .set_c
            .keys()
            .filter(|k| matches!(k.unit_type, MONSTER | OBJECT | MISSILE))
            .filter_map(|k| self.view_unit(&view_key(*k)));
        self.units
            .values()
            .map(std::borrow::Cow::Borrowed)
            .chain(c)
            .collect()
    }

    /// The 16.16 path position of a light owner that is a client missile
    /// (a dynamic path's precise position);
    /// `None` for any other owner.
    pub fn client_missile_pos(&self, owner: &Owner) -> Option<(u32, u32)> {
        if !owner.client_only || owner.unit_type != u32::from(MISSILE) {
            return None;
        }
        let key = UnitKey::new(MISSILE, owner.guid);
        self.objclient.missiles.get(&key).map(|m| m.pos)
    }

    /// The light owner lookup: set C when
    /// the record's lookup flag is set, else set S.
    pub fn light_owner(&self, owner: &Owner) -> Option<&ClientUnit> {
        let t = u8::try_from(owner.unit_type).ok()?;
        let key = UnitKey::new(t, owner.guid);
        if owner.client_only {
            return self.objclient.set_c.get(&key);
        }
        self.units.get(&key)
    }
}

/// The base-game order of the queue drains: missiles,
/// players, monsters, objects, items; then `GUID & 0x7F` ascending, then
/// GUID descending.
pub fn update_order(units: &BTreeMap<UnitKey, ClientUnit>) -> Vec<UnitKey> {
    const TYPES: [u8; 5] = [MISSILE, PLAYER, MONSTER, OBJECT, ITEM];
    let mut keys: Vec<UnitKey> = units
        .keys()
        .copied()
        .filter(|k| TYPES.contains(&k.unit_type))
        .collect();
    keys.sort_by_key(|k| {
        let t = TYPES.iter().position(|&t| t == k.unit_type);
        (t, k.guid & 0x7F, std::cmp::Reverse(k.guid))
    });
    keys
}

/// The unit an S→C message addresses through the receive table's unit
/// handler.
/// `None` for ids without a unit handler and for messages too short for
/// the lookup (open question 3).
pub fn addressed_unit(msg: &[u8]) -> Option<UnitKey> {
    let &id = msg.first()?;
    server_message(id)?.client_unit_handler?;
    let u32_at = |off: usize| -> Option<u32> {
        let b = msg.get(off..off + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if (0x67..=0x6D).contains(&id) {
        Some(UnitKey {
            unit_type: MONSTER,
            guid: u32_at(1)?,
        })
    } else {
        Some(UnitKey {
            unit_type: *msg.get(1)?,
            guid: u32_at(2)?,
        })
    }
}

/// Visibility of a unit's sprite at a client pixel point
/// (model): a render seam, taken as an
/// input. A closure so the render side can answer from its own
/// state (camera, COF and cel stores: `world_view::visibility`); the
/// bridge only calls it.
#[derive(Clone)]
pub struct VisibleFn(Arc<Visible>);

/// The closure behind a [`VisibleFn`].
type Visible = dyn Fn(&ClientUnit, i32, i32) -> bool + Send + Sync;

impl VisibleFn {
    pub fn new(f: impl Fn(&ClientUnit, i32, i32) -> bool + Send + Sync + 'static) -> Self {
        VisibleFn(Arc::new(f))
    }

    /// `visible(U, a, b)`, (a, b) in client pixel space.
    pub fn visible(&self, unit: &ClientUnit, a: i32, b: i32) -> bool {
        (self.0)(unit, a, b)
    }
}

impl std::fmt::Debug for VisibleFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VisibleFn(..)")
    }
}

/// The `xoff`, `yoff` of a unit's layer-`c` cel at its direction and
/// frame: a render seam
/// like [`VisibleFn`], answered by the view's unit art
/// (`world_view::unit_assets::cel_offset_fn`). `None`: the request fails
/// (no COF loaded for the draw mode, no such layer, no file or cel).
#[derive(Clone)]
pub struct CelOffsetFn(Arc<CelOffset>);

/// The closure behind a [`CelOffsetFn`].
type CelOffset = dyn Fn(&ClientUnit, u8) -> Option<(i32, i32)> + Send + Sync;

impl CelOffsetFn {
    pub fn new(f: impl Fn(&ClientUnit, u8) -> Option<(i32, i32)> + Send + Sync + 'static) -> Self {
        CelOffsetFn(Arc::new(f))
    }

    /// `unit`'s layer-`component` cel offset, `None` when the request fails.
    pub fn cel_offset(&self, unit: &ClientUnit, component: u8) -> Option<(i32, i32)> {
        (self.0)(unit, component)
    }
}

impl std::fmt::Debug for CelOffsetFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CelOffsetFn(..)")
    }
}

/// The tables the message rules read (Inputs). The
/// `monstats2` choice counts are record +0x15 + i
/// ([`MonsterClass::from_record`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientTables {
    /// One entry per `monstats` row: `None` = no `monstats2` row.
    pub monsters: Vec<Option<MonsterClass>>,
    /// The act level bonus of the monster skills by difficulty
    /// (the difficulty record +0x10, `difficultylevels`).
    pub monster_skill_bonus: [i32; 3],
    /// One entry per `itemstatcost` row.
    pub stats: Vec<StatSend>,
    /// One entry per `Levels.txt` row, by level id.
    pub levels: Vec<LevelRow>,
    /// One entry per `skills` row, by skill id (Inputs);
    /// the skill count is the row count.
    pub skills: Vec<SkillRow>,
    /// One entry per `skilldesc` row.
    pub skilldesc: Vec<SkillDescRow>,
    /// One entry per `objects.txt` row, by class.
    pub objects: Vec<ObjectRow>,
    /// One entry per `missiles` row, by class (the client create,
    ///–).
    pub missiles: Vec<super::client_missiles::ClientMissileRow>,
    /// The `Code` byte (+0) of each `shrines.txt` row, by index
    /// (table `[0x0096D468]`, count
    /// `[0x0096D46C]`).
    pub shrines: Vec<u8>,
    /// One entry per `states` row, by state id.
    pub states: Vec<StateRow>,
    /// The overlay count (data tables +0xBC0).
    pub overlay_count: u32,
    /// One entry per `overlay` row, by id: the fields of its light
    /// ([`super::overlay_lights`]).
    pub overlays: Vec<super::overlay_lights::OverlayLightRow>,
    /// One entry per `charstats` row, by class: its `Skill 1`…`Skill 10`
    /// (record +0xAE, i16 as u16).
    pub class_skills: Vec<[u16; 10]>,
    /// One entry per `charstats` row, by class: `WalkVelocity` (+0x40),
    /// `RunVelocity` (+0x41).
    pub class_velocities: Vec<[u8; 2]>,
    /// The monsters' skill sequences of the mode-14 load
    /// ([`super::monster_anim`]).
    pub monster_sequences: std::sync::Arc<MonsterSequences>,
    /// The `hireling` rows of the client hireling set-up.
    pub hirelings: std::sync::Arc<HirelingRows>,
}

/// Inputs of the message rules that are not model state.
#[derive(Clone, Debug, Default)]
pub struct ModelInputs {
    pub tables: ClientTables,
    /// `None`: the check refuses when it needs visibility.
    pub visible: Option<VisibleFn>,
    /// What the client DRLG of 0x03 is built from. `None`:
    /// no client DRLG is built (`ClientWorld::drlg` stays `None`).
    pub drlg: Option<DrlgSource>,
    /// The `d2exp.mpq` check.
    pub expansion_installed: bool,
    /// Wall-clock seconds (a `time` base plus elapsed `GetTickCount` /
    /// 1000; client-only, a
    /// host input). 0 when the host gives none.
    pub wall_seconds: Option<fn() -> i32>,
    /// `GetTickCount` of this update, wrapping milliseconds:
    /// the live client passes the host
    /// clock, tests and replays a scripted value.
    pub now: u32,
    /// What the client object functions read beside the model
    /// (Inputs).
    pub objclient: ObjClientInputs,
    /// The light quality ≠ 0 (high): missiles get lights
    /// (missile row).
    /// The play app sets it, as the preview draws at quality 2.
    pub high_light_quality: bool,
    /// The skills tables and formula buffers the passive refresh
    /// evaluates; `None`: a passive skill
    /// is a handler error.
    pub skill_tables: Option<std::sync::Arc<super::passive::Tables>>,
    /// The players' animation lookup of the client player update
    /// ([`super::player_anim`]); `None`: a player mode has no frames and
    /// an attack, cast or hit mode ends on the next update.
    pub player_anims: Option<std::sync::Arc<dyn super::player_anim::PlayerAnims>>,
    /// The view's cel offsets of a unit's layer (read by
    /// `cltdofunc` 74); `None`: every request fails.
    pub cel_offset: Option<CelOffsetFn>,
}

/// The unit update's first step with (U, 0):
/// each client-made NEWLENGTH list's expire counter − 1; at
/// ≤ 0 the list is freed and its removal callback clears its state bit.
pub fn expire_timed_lists(u: &mut ClientUnit) {
    let mut gone = Vec::new();
    for (&state, expire) in u.timed_lists.iter_mut() {
        *expire -= 1;
        if *expire <= 0 {
            gone.push(state);
        }
    }
    for state in gone {
        u.timed_lists.remove(&state);
        u.state_lists.remove(&state);
        u.states.remove(&state);
    }
}

/// MulDiv(a, b, 100) with the divisor
/// fixed at 100: a large first factor (> 0x100000) is divided first, then a
/// large second one (> 0x10000); otherwise the product is divided. All
/// i32, truncating toward zero.
fn percent_of(a: i32, b: i32) -> i32 {
    if a > 0x10_0000 {
        (a / 100).wrapping_mul(b)
    } else if b > 0x1_0000 {
        (b / 100).wrapping_mul(a)
    } else {
        a.wrapping_mul(b) / 100
    }
}
