//! Sound triggers: which code path requests which sound.
//! One plain-Rust function per cause class, each
//! with explicit inputs, the per-unit sound fields in
//! [`UnitSound`] / [`Globals`], and every call to the sound layer through
//! [`SoundCalls`] (plus the read-only row/request queries of
//! [`TriggerSound`]). No Bevy, no float, no I/O; all draws on the client
//! RNG through [`SoundCalls::roll`].
//!
//! - server sound events, player event sounds: [`events`]
//! - mode sounds: [`modes`]
//! - footsteps, monster idle voices: [`movement`]
//! - object mode sounds: [`objects`] (table [`tables::ObjectSounds`])
//! - skills/missiles/states, items: [`skills`]
//! - NPC speech: [`npc`] (table [`tables::NpcSpeech`])
//! - UI sounds, other fixed requests: [`ui`]

pub mod events;
pub mod hooks;
pub mod identity;
pub mod modes;
pub mod movement;
pub mod npc;
pub mod objects;
pub mod skills;
pub mod tables;
pub mod ui;

#[cfg(test)]
mod tests;

use d2_data::tables::Monsounds;

use super::calls::{Handle, SoundCalls};
use crate::bridge::world::UnitKey;

pub use crate::bridge::world::{MONSTER, OBJECT, PLAYER};

/// Read-only queries the rules need beyond the [`SoundCalls`] surface:
/// `sounds.txt` row data and the request/unit lists the sound layer owns.
/// Implemented by the sound table; tests use a fake that logs the calls.
pub trait TriggerSound: SoundCalls {
    /// Sound system on. The monster idle voice
    /// reads it (its group test is false while the system is off).
    fn sound_on(&self) -> bool;
    /// Group base of a sound id.
    fn group_base(&self, id: i32) -> i32;
    /// `Group Size` of row `base` after the group pass.
    fn group_size(&self, base: i32) -> i32;
    /// The sound row's `Loop` column.
    fn looping(&self, id: i32) -> bool;
    /// U+0x78: U's request list, newest
    /// first, as (handle, current id): waiting, playing and
    /// ended-not-yet-freed requests; a freed handle reads id 0.
    fn unit_requests(&self, unit: UnitKey) -> Vec<(Handle, i32)>;
    /// Number of units in a request's unit list.
    fn unit_count(&self, h: Handle) -> usize;
    /// The variant pick of `id` with its draws, used by the NPC greeting.
    fn variant(&mut self, id: i32) -> i32;
    /// The volume of the request with this handle, `None` when it is
    /// gone.
    fn request_volume(&self, h: Handle) -> Option<i32>;
}

/// A fatal path of the sound rules (where the game asserts or crashes).
/// Returned, never swallowed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TriggerError {
    #[error("player class {0} >= 7 (class record table, §3 r1)")]
    PlayerClass(i32),
    #[error("swing index {0} >= 14 (§1 r9)")]
    SwingIndex(u8),
    #[error("object class {0} > 572 (§7 r1)")]
    ObjectClass(i32),
    #[error("object mode {0} >= 8 (§7 r7)")]
    ObjectMode(u8),
    #[error("client object function {0} >= 19 (§20 r1, fatal 0x546)")]
    ClientFn(u8),
    #[error("footstep divisor n × s = 0 (F = {frames}, n = {count}; §5 r9 division by zero)")]
    FootstepStep { frames: u32, count: u32 },
}

/// The client unit as the rules read it.
/// Fields a rule does not use may stay at their defaults.
#[derive(Clone, Copy, Debug)]
pub struct Unit<'a> {
    pub key: UnitKey,
    /// The sound identity's type: the raw type,
    /// except a player transformed into a monster form reads 1.
    pub identity_type: u8,
    /// Player class, monstats row, object class or item row (+0x04).
    pub class: i32,
    /// Monster `BaseId`.
    pub base_class: i32,
    /// Current mode (+0x10).
    pub mode: u8,
    /// +0x44 frame (fixed point, as stored).
    pub frame: u32,
    /// +0x48 frame count.
    pub frame_count: u32,
    /// +0x4C animation speed.
    pub speed: i32,
    /// U is the local player P.
    pub is_local: bool,
    /// Monster's `monsounds` record, if any.
    pub monsounds: Option<&'a Monsounds>,
    /// `monstats2.critter`.
    pub critter: bool,
    /// Has state 146.
    pub state_146: bool,
    /// Has state 1 `freeze`.
    pub frozen: bool,
    /// Dying or dead.
    pub dying: bool,
    /// Weapon hit class.
    pub weapon_hit_class: u8,
    /// U has a room and its level is a town.
    pub in_town: bool,
    /// Within 700 of P (see [`within_700`]).
    pub near_local: bool,
    /// Distance to P.
    pub local_dist: i32,
}

impl<'a> Unit<'a> {
    pub fn new(key: UnitKey, class: i32) -> Self {
        Unit {
            key,
            identity_type: key.unit_type,
            class,
            base_class: class,
            mode: 0,
            frame: 0,
            frame_count: 0,
            speed: 0,
            is_local: false,
            monsounds: None,
            critter: false,
            state_146: false,
            frozen: false,
            dying: false,
            weapon_hit_class: 0,
            in_town: false,
            near_local: false,
            local_dist: 0,
        }
    }

    /// The raw unit type (+0x00).
    pub fn unit_type(&self) -> u8 {
        self.key.unit_type
    }

    /// The sound identity's type: what the mode, footstep
    /// and idle rules mean by "player" and "monster".
    pub fn identity(&self) -> u8 {
        self.identity_type
    }
}

/// U has a request in `s`'s group. B = s's
/// group base, n = B's `Group Size`; true for the first handle (newest
/// first) whose request's current id lies in B … B + n − 1 (signed
/// compares; a freed handle reads 0). False while the channels are not
/// set up (the sound system off).
pub fn request_in_group(s: &dyn TriggerSound, unit: UnitKey, id: i32) -> bool {
    if !s.sound_on() {
        return false;
    }
    let b = s.group_base(id);
    let n = s.group_size(b);
    s.unit_requests(unit)
        .iter()
        .any(|&(_, cur)| cur >= b && cur < b.wrapping_add(n))
}

/// The handle [`request_in_group`] finds: U's first request
/// (newest first) whose current id lies in `s`'s group; none while the
/// sound system is off.
pub fn first_in_group(s: &dyn TriggerSound, unit: UnitKey, id: i32) -> Option<Handle> {
    if !s.sound_on() {
        return None;
    }
    let b = s.group_base(id);
    let n = s.group_size(b);
    s.unit_requests(unit)
        .into_iter()
        .find(|&(_, cur)| cur >= b && cur < b.wrapping_add(n))
        .map(|(h, _)| h)
}

/// Detach every handle of U's list (`force` ends them too);
/// the unit free runs it without force first, so loops
/// lose U and stop when U was their last unit and one-shots keep playing.
pub fn detach_all(s: &mut dyn TriggerSound, unit: UnitKey, force: bool) {
    for (h, _) in s.unit_requests(unit) {
        s.detach(h, unit, force);
    }
}

/// x² + (2y)² < 490,000 for the client position deltas of U relative to
/// P.
pub fn within_700(dx: i64, dy: i64) -> bool {
    dx * dx + (2 * dy) * (2 * dy) < 490_000
}

/// Per-unit sound fields, kept by the client for each unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitSound {
    /// +0x70 object mode seen.
    pub obj_seen: bool,
    /// +0x74 object previous mode.
    pub obj_prev_mode: u8,
    /// +0x7C last voice (C).
    pub last_voice: u32,
    /// +0x80 last idle voice (C).
    pub last_idle: u32,
    /// +0x84 last footstep (C).
    pub last_footstep: u32,
    /// +0xB0 last hit class taken (its writer is not known).
    pub hit_class: u8,
}

/// Global sound timers. All start 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Globals {
    /// Last idle voice of any monster (C).
    pub last_idle_any: u32,
    /// Last voice of any unit (C).
    pub last_voice_any: u32,
    /// Idle gap (C).
    pub idle_gap: u32,
    /// Time (C) of the last player speech line.
    pub speech_time: u32,
    /// Id of the last player speech line.
    pub speech_id: i32,
}

impl Globals {
    /// The state sound init leaves at game start: idle gap 90.
    pub fn sound_init() -> Self {
        Self {
            idle_gap: 90,
            ..Self::default()
        }
    }
}

/// What a rule call works with: the sound layer, the globals and the
/// client update counter C.
pub struct Ctx<'a> {
    pub s: &'a mut dyn TriggerSound,
    pub g: &'a mut Globals,
    /// C, the client tick count.
    pub c: u32,
}

impl<'a> Ctx<'a> {
    pub fn new(s: &'a mut dyn TriggerSound, g: &'a mut Globals, c: u32) -> Self {
        Ctx { s, g, c }
    }

    /// request(id, U, d, f, o).
    fn req(&mut self, id: i32, unit: Option<UnitKey>, delay: u32) -> Handle {
        self.s.request(id, unit, delay, 0, 0)
    }

    /// request(id, U, 0, 0, 0) on behalf of a model rule outside
    /// the trigger functions (the client object code and the shrine
    /// sound).
    pub fn unit_request(&mut self, id: i32, unit: UnitKey) -> Handle {
        self.req(id, Some(unit), 0)
    }

    /// The unit's last voice := C, the last voice of any unit := C.
    fn voiced(&mut self, us: &mut UnitSound) {
        us.last_voice = self.c;
        self.g.last_voice_any = self.c;
    }
}

/// The swing table: (id, frames) per index `h & 0xF`, id 0 = none.
/// Indices 14, 15 are outside the table (fatal).
pub const SWING: [(i32, u32); 14] = [
    (0, 0),
    (251, 6),
    (262, 6),
    (268, 6),
    (274, 8),
    (280, 8),
    (286, 6),
    (292, 8),
    (268, 6),
    (298, 6),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
];

/// Swing table entry for hit class `h`.
pub fn swing_entry(h: u8) -> Result<(i32, u32), TriggerError> {
    SWING
        .get((h & 0xF) as usize)
        .copied()
        .ok_or(TriggerError::SwingIndex(h & 0xF))
}

/// `swing(U, h)`: (F × 256 + 128) / max(speed, 1),
/// unsigned, truncating.
pub fn swing(speed: i32, h: u8) -> Result<u32, TriggerError> {
    let (_, frames) = swing_entry(h)?;
    Ok(delay_ticks(frames, speed))
}

/// (F × 256 + 128) / max(speed, 1).
pub fn delay_ticks(frames: u32, speed: i32) -> u32 {
    frames.wrapping_mul(256).wrapping_add(128) / speed.max(1) as u32
}

/// Player class sound record, in field order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassRecord {
    pub hit: i32,
    pub death: i32,
    pub impossible: i32,
    pub needmana: i32,
    pub needkey: i32,
    pub cantcarry: i32,
    pub cantuseyet: i32,
    pub notintown: i32,
    pub chat_base: i32,
    pub quest_base: i32,
    pub footstep_base: i32,
}

const fn rec(v: [i32; 11]) -> ClassRecord {
    ClassRecord {
        hit: v[0],
        death: v[1],
        impossible: v[2],
        needmana: v[3],
        needkey: v[4],
        cantcarry: v[5],
        cantuseyet: v[6],
        notintown: v[7],
        chat_base: v[8],
        quest_base: v[9],
        footstep_base: v[10],
    }
}

/// The player class sound records, one per class.
pub const CLASS_RECORDS: [ClassRecord; 7] = [
    rec([
        2878, 2883, 2953, 2957, 2955, 2934, 2936, 2959, 2937, 2961, 2768,
    ]),
    rec([
        2910, 2915, 3265, 3269, 3267, 3246, 3248, 3271, 3249, 3273, 2720,
    ]),
    rec([
        2894, 2899, 3109, 3113, 3111, 3090, 3092, 3115, 3093, 3117, 2720,
    ]),
    rec([
        2886, 2891, 3187, 3191, 3189, 3168, 3170, 3193, 3171, 3195, 2768,
    ]),
    rec([
        2902, 2907, 3031, 3035, 3033, 3012, 3014, 3037, 3015, 3039, 2816,
    ]),
    rec([
        2926, 2931, 3421, 3425, 3423, 3402, 3404, 3427, 3405, 3429, 2720,
    ]),
    rec([
        2918, 2923, 3343, 3347, 3345, 3324, 3326, 3349, 3327, 3351, 2720,
    ]),
];

/// Class record of a player class; class ≥ 7 (or negative) is fatal.
pub fn class_record(class: i32) -> Result<&'static ClassRecord, TriggerError> {
    usize::try_from(class)
        .ok()
        .and_then(|c| CLASS_RECORDS.get(c))
        .ok_or(TriggerError::PlayerClass(class))
}

/// A `monsounds` sound column as the signed id the rules compare.
pub(crate) fn sid(v: u32) -> i32 {
    v as i32
}

/// Detach with force every request of U whose group base is one of
/// `ids`' group bases.
fn detach_groups(s: &mut dyn TriggerSound, unit: UnitKey, ids: &[i32]) {
    let bases: Vec<i32> = ids.iter().map(|&id| s.group_base(id)).collect();
    for (h, id) in s.unit_requests(unit) {
        if bases.contains(&s.group_base(id)) {
            s.detach(h, unit, true);
        }
    }
}

/// The skill voices of a monster record (`Skill1..4`, +0x44 … +0x50).
fn skill_voices(r: &Monsounds) -> [i32; 4] {
    [sid(r.skill1), sid(r.skill2), sid(r.skill3), sid(r.skill4)]
}

/// Detach N's skill voices with force. Only when N's identity type is 1
/// and N has a
/// record: every handle of N's list whose request's current id has a group
/// base **equal to** one of the raw `Skill1` … `Skill4` cells (not reduced
/// to their group bases); a freed handle has base 0 and matches a 0 cell.
pub fn detach_skill_voices(s: &mut dyn TriggerSound, npc: &Unit) {
    let Some(r) = npc.monsounds else { return };
    if npc.identity_type != MONSTER {
        return;
    }
    let cells = skill_voices(r);
    for (h, id) in s.unit_requests(npc.key) {
        if cells.contains(&s.group_base(id)) {
            s.detach(h, npc.key, true);
        }
    }
}

/// The idle gap := uniform(30, 90).
pub const IDLE_GAP_RANGE: (i32, i32) = (30, 90);

fn uniform_gap(cx: &mut Ctx) -> u32 {
    super::calls::uniform(&mut *cx.s, IDLE_GAP_RANGE.0, IDLE_GAP_RANGE.1) as u32
}
