//! The small C→S handlers of the player: the messages that need no system
//! of their own and are routed here by the server's dispatcher:
//! 0x12 EndInferno, 0x14 OverheadChat, 0x3D HighlightDoor (message part),
//! 0x3F PlayAudio, 0x41 Resurrect, 0x44 StaffInOrifice (entry), 0x46
//! MercInteract, 0x47 MoveMerc, 0x48 TurnOffBusyState, 0x4B
//! RequestEntityUpdate, 0x4D PlayNpcMessage, 0x51 BindHotkey, 0x53
//! StaminaOn, 0x54 StaminaOff and 0x60 SwapWeapons (message part).
//!
//! The dispatcher (`crate::dispatch`) has run the gate and the exact
//! size check when [`handle`] runs. Each handler's own rules are [`run`],
//! on the game state the world host lends as a [`PlayerWorld`]
//! ([`WorldHost::player`]; the action wiring's provider is
//! [`action::ActionPlayer`]). A host without one leaves the ids to the
//! stub, as does a handler whose behaviour sits behind a call with no
//! provider (`Outcome::code` `None`: 0x44's staff-in-orifice call and
//! 0x60's weapon switch).
//!
//! The unit and point tests a handler makes with a range of its own
//! (0x3D range 10, 0x46 / 0x47 range 50) read the same staged facts as the
//! dispatcher's parsers ([`crate::seams::Intents::unit_target`]) and are
//! made here before the world call ([`Run::target`]).
//!
//! The hot-key slots 0x51 fills are client data:
//! [`SimGame::hotkeys`].

pub mod action;

#[cfg(test)]
mod tests;

use d2_sim::skills::SkillEntry;
use d2_sim::tick::EventDispatch;
use d2_sim::units::{UnitId, UnitType};

use super::super::SimGame;
use super::world::{WorldError, WorldFault, WorldHost};
use crate::seams::{ClientId, Intents, MessageSink, Pos, ResultCode, UnitTarget};

/// How a client id of this module is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Its rules are [`run`]'s.
    Handled,
    /// Belongs to another system; no `d2-sim` code to route it to yet: stub.
    Pending,
}

/// Every C→S id owned by this module and not routed by any other handler
/// module: (id, `client-messages.tsv` name, owner note, status). The order
/// is the id order.
pub const PLAYER_IDS: &[(u8, &str, &str, Status)] = &[
    (
        0x12,
        "EndInferno",
        "sim/intents-events.md §9 r2",
        Status::Handled,
    ),
    (
        0x14,
        "OverheadChat",
        "sim/intents-events.md §9 r3",
        Status::Handled,
    ),
    // Relay to the other clients; the checks are the dispatcher's.
    (
        0x15,
        "Chat",
        "sim/intents-events.md §9 r16 (checks: §2.4 r6)",
        Status::Handled,
    ),
    (
        0x3D,
        "HighlightDoor",
        "sim/intents-events.md §9 r4 (open question 15)",
        Status::Handled,
    ),
    (
        0x3F,
        "PlayAudio",
        "sim/intents-events.md §9 r5",
        Status::Handled,
    ),
    (
        0x41,
        "Resurrect",
        "sim/intents-events.md §9 r6",
        Status::Handled,
    ),
    (
        0x44,
        "StaffInOrifice",
        "world/quests-act2.md §8.6 (entry: sim/intents-events.md §9 r7)",
        Status::Handled,
    ),
    (
        0x46,
        "MercInteract",
        "sim/intents-events.md §9 r8",
        Status::Handled,
    ),
    (
        0x47,
        "MoveMerc",
        "sim/intents-events.md §9 r8",
        Status::Handled,
    ),
    (
        0x48,
        "TurnOffBusyState",
        "sim/intents-events.md §9 r9",
        Status::Handled,
    ),
    (
        0x4B,
        "RequestEntityUpdate",
        "sim/intents-events.md §9 r10",
        Status::Handled,
    ),
    (
        0x4D,
        "PlayNpcMessage",
        "sim/intents-events.md §9 r11",
        Status::Handled,
    ),
    (
        0x51,
        "BindHotkey",
        "sim/intents-events.md §9 r12",
        Status::Handled,
    ),
    (
        0x53,
        "StaminaOn",
        "sim/intents-events.md §9 r13",
        Status::Handled,
    ),
    (
        0x54,
        "StaminaOff",
        "sim/intents-events.md §9 r13",
        Status::Handled,
    ),
    // Routed to the NPC system (`handlers/world.rs`, `WORLD_IDS`).
    (
        0x59,
        "MakeEntityMove",
        "monsters/ai-bodies.md §9.9 (entry not written)",
        Status::Pending,
    ),
    (
        0x60,
        "SwapWeapons",
        "sim/intents-events.md §9 r14 (open question 16)",
        Status::Handled,
    ),
];

/// Whether `id` is routed to [`run`].
pub fn handled(id: u8) -> bool {
    PLAYER_IDS
        .iter()
        .any(|&(i, _, _, s)| i == id && s == Status::Handled)
}

/// State 12 `inferno`, cleared by 0x12.
pub const STATE_INFERNO: u16 = 12;
/// State 0x36 (54, uninterruptable), on then off in 0x41.
pub const STATE_UNINTERRUPTABLE: u16 = 0x36;
/// Player modes read and set here.
pub const MODE_NEUTRAL: u32 = 1;
pub const MODE_WALK: u32 = 2;
pub const MODE_RUN: u32 = 3;
pub const MODE_DEAD: u32 = 0x11;
/// Stats 6, 8, 10: life, mana, stamina (0x41 refills all three; 0x53 reads stamina).
pub const STAT_LIFE: u16 = 6;
pub const STAT_MANA: u16 = 8;
pub const STAT_STAMINA: u16 = 10;
/// Unit flags and flags-ex bits these handlers write or read.
pub const FLAG_RESURRECTED: u32 = 0x2;
pub const FLAG_OVERHEAD: u32 = 0x100;
pub const FLAG_NO_AUDIO: u32 = 0x400;
pub const FLAG_EX_RESEND: u32 = 0x1_0000;
/// Timer event 6 (free the overhead).
pub const EVENT_OVERHEAD: u32 = 6;
/// The overhead duration: 8 frames per character (at most 254), plus 0x7D.
pub const OVERHEAD_PER_CHAR: i32 = 8;
pub const OVERHEAD_MAX_CHARS: i32 = 254;
pub const OVERHEAD_BASE: i32 = 0x7D;
/// 0x3F's speech sounds.
pub const AUDIO_SOUNDS: std::ops::RangeInclusive<u16> = 25..=32;
/// The mercenary's acknowledgement sound toward the player (0x46 / 0x47).
pub const SOUND_MERC_ACK: u32 = 15;
/// The mercenary AI commands of 0x46 / 0x47.
pub const COMMAND_INTERACT: i32 = 0x0C;
pub const COMMAND_MOVE: i32 = 0x0D;
/// Ranges of the unit / point tests (0x3D door, 0x46 / 0x47 mercenary).
pub const DOOR_RANGE: u32 = 10;
pub const MERC_RANGE: u32 = 50;
/// The drop reason given to a hardcore player who asks to resurrect.
pub const DROP_HARDCORE: u32 = 3;
/// Hot-key slots of a client (8 bytes each).
pub const HOTKEY_SLOTS: usize = 16;

/// One hot-key slot of the client data: skill i16 (−1: unbound),
/// left-hand byte at +2, item u32 at +4 (set by 0x51; read by the join's
/// S→C 0x7B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotKey {
    pub skill: i16,
    pub left: bool,
    pub item: u32,
}

impl HotKey {
    /// An unbound slot.
    ///
    /// A brand-new character's client record holds skill −1 in all 16
    /// slots, so the join sends no 0x7B.
    pub const UNBOUND: HotKey = HotKey {
        skill: -1,
        left: false,
        item: 0,
    };
}

/// The game state these handlers read and write, as the world host lends
/// it. Each method says which game operation it stands for; its provider is
/// [`action::ActionPlayer`] (the action wiring), which answers what
/// `d2-sim` does not hold itself through `d2_sim::wiring::action::Pending`.
pub trait PlayerWorld {
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> d2_sim::tick::TickRate;
    /// The game is an expansion game.
    fn expansion(&self) -> bool;
    /// The game's difficulty.
    fn difficulty(&self) -> u8;
    /// The unit's mode.
    fn mode(&self, u: UnitId) -> u32;
    /// Set the unit's mode (as the unit-mode change does, without the
    /// combat-list drop of a full mode start).
    fn set_mode(&mut self, u: UnitId, mode: u32);
    /// The unit's flags.
    fn flags(&self, u: UnitId) -> u32;
    fn or_flags(&mut self, u: UnitId, bits: u32);
    /// ORs bits into the unit's flags-ex.
    fn or_flags_ex(&mut self, u: UnitId, bits: u32);
    /// The unit's stat value (layer 0).
    fn stat(&self, u: UnitId, stat: u16) -> i32;
    /// The state toggle (stat-list state on / off) with the update-queue
    /// insert.
    fn set_state(&mut self, u: UnitId, state: u16, on: bool);
    /// Queue the unit for the end-of-tick client update.
    fn queue_update(&mut self, u: UnitId);
    /// Schedule timer `event` for the unit, expiring at frame `expire`.
    fn schedule_event(&mut self, u: UnitId, event: u32, expire: i32);
    /// Play `sound` at the unit, heard by `to` (none: everyone near).
    fn sound(&mut self, u: UnitId, sound: u32, to: Option<UnitId>);
    /// Look a unit up by type and GUID.
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId>;
    /// The room test of 0x4B: the player's room is in the unit's room's
    /// room list.
    fn room_in_list_of(&self, player: UnitId, unit: UnitId) -> bool;
    /// Whether the unit is dead.
    fn is_dead(&self, u: UnitId) -> bool;
    /// Whether the unit has a skill in use.
    fn has_used_skill(&self, u: UnitId) -> bool;
    /// The skills count.
    fn skill_count(&self) -> u32;
    /// Whether the unit has `skill` (from `item`, for item skills).
    fn has_skill(&self, u: UnitId, skill: i32, item: u32) -> bool;
    /// The monstats row count.
    fn monstats_count(&self) -> u32;

    // ---- 0x14, 0x3D
    /// Whether the overhead / chat text is refused by the text filter.
    fn overhead_text_test(&self, text: &[u8]) -> bool;
    /// Free the unit's overhead record and make a new one ending at
    /// `end` with byte +8 = `byte8`, kept on the unit.
    fn replace_overhead(&mut self, u: UnitId, text: &[u8], byte8: u8, end: i32);
    /// Highlight the door `guid` for the player.
    fn highlight_door(&mut self, player: UnitId, guid: u32);

    // ---- 0x15
    /// Queue `bytes` for the client of `to` (a player without a client
    /// receives nothing).
    fn send(&mut self, to: UnitId, bytes: &[u8]);
    /// The party / relation test of `a` toward `b` for `flag`.
    fn relation(&self, a: UnitId, b: UnitId, flag: u32) -> bool;
    /// The player `u` is interacting with (an active interaction with a
    /// unit of type 0, resolved to that player by GUID); none otherwise.
    fn interacting_player(&self, u: UnitId) -> Option<UnitId>;
    /// A mod game's chat command hook
    /// (`d2_sim::rules::EventHooks::chat_command`): `true` takes the open
    /// chat line `text` of player `p`, which then goes to no one. Always
    /// `false` under D2RUST's own rules.
    fn chat_command(&mut self, p: UnitId, text: &[u8]) -> bool;

    // ---- 0x41
    /// Client flag 4: the player is hardcore.
    fn hardcore(&self, player: UnitId) -> bool;
    /// Drop the player's client from the game with `reason`.
    fn drop_client(&mut self, player: UnitId, reason: u32);
    /// The unit's skill list, in list order.
    fn skill_entries(&self, u: UnitId) -> Vec<SkillEntry>;
    /// skills.txt `passivestate` of `skill` (signed; ≤ 0: none).
    fn passive_state(&self, skill: i32) -> i32;
    /// Apply the passive state of skill entry `e` to the unit.
    fn passive_state_apply(&mut self, u: UnitId, e: &SkillEntry);
    /// The maximum of stats 6 / 8 / 10 (life / mana / stamina).
    fn stat_max(&self, u: UnitId, stat: u16) -> i32;
    /// Set the stat and send the stat message to the client.
    fn set_stat_send(&mut self, u: UnitId, stat: u16, value: i32);
    /// The level id of the unit's room (none: 0).
    fn current_level(&self, u: UnitId) -> u32;
    /// Warp the player to `level`.
    fn warp(&mut self, player: UnitId, level: u32, arg: u32);
    /// Start `mode` for the player with no target, skipping the first gate.
    fn start_mode_skip_gate(&mut self, player: UnitId, mode: u32);
    /// Re-select the left hand skill, then the right hand skill.
    fn reselect_hand_skills(&mut self, player: UnitId);

    // ---- 0x44
    /// Whether the player is busy.
    fn busy(&self, player: UnitId) -> bool;
    /// Whether the player is trading.
    fn trading(&self, player: UnitId) -> bool;
    /// The staff-in-orifice action on `object` with `item`; `None`: no
    /// provider.
    fn staff_in_orifice(
        &mut self,
        player: UnitId,
        object: u32,
        item: u32,
        action: u16,
    ) -> Option<u32>;

    // ---- 0x46, 0x47
    /// The player's hireling, if any.
    fn hireling(&self, player: UnitId) -> Option<UnitId>;
    /// Free the monster's AI commands, then add `command`.
    fn replace_ai_commands(&mut self, monster: UnitId, command: [i32; 5]);

    // ---- 0x48, 0x4D, 0x60
    /// The player data's busy byte is set.
    fn busy_flag(&self, player: UnitId) -> bool;
    /// Clear the busy flag: the player data's busy byte := 0.
    fn clear_busy_flag(&mut self, player: UnitId);
    /// Mark the NPC intro of `class` as played for the player.
    fn clear_npc_intro(&mut self, player: UnitId, difficulty: u8, class: u16);
    /// The weapon switch: (result, fail); `None`: no provider.
    fn weapon_switch(&mut self, player: UnitId) -> Option<(u32, bool)>;
}

/// One message routed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run<'m> {
    pub player: UnitId,
    /// The drained message (its exact size).
    pub msg: &'m [u8],
    /// The result of the handler's own unit / point test (0x3D, 0x46,
    /// 0x47; 0: passed), made before the world call ([`handle`]).
    pub target: u32,
    /// 0x15: every client of the game with a player, in visit order,
    /// with its client name (16 bytes).
    pub clients: &'m [(UnitId, [u8; 16])],
}

/// What the host answers about the player that the action wiring cannot
/// (the wired host's hireling list).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostFacts {
    /// `Some`: the host's answer to [`PlayerWorld::hireling`].
    pub hireling: Option<Option<UnitId>>,
}

/// What a handler did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The result code; `None`: the handler reached a call with no
    /// provider, the id stays a stub (nothing was changed).
    pub code: Option<u32>,
    /// 0x51: the slot and value to store.
    pub hotkey: Option<(usize, HotKey)>,
}

impl Outcome {
    fn code(c: u32) -> Self {
        Self {
            code: Some(c),
            hotkey: None,
        }
    }
    const STUB: Outcome = Outcome {
        code: None,
        hotkey: None,
    };
}

fn u16_at(m: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(m.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(m: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(m.get(at..at + 4)?.try_into().ok()?))
}

/// The exact size of each handled id (`client-messages.tsv`); 0x14 is
/// 4..=275.
fn size_ok(id: u8, len: usize) -> bool {
    match id {
        0x14 => (4..=275).contains(&len),
        0x15 => len >= 5,
        0x12 | 0x41 | 0x48 | 0x53 | 0x54 | 0x60 => len == 1,
        0x3F | 0x4D => len == 3,
        0x3D => len == 5,
        0x4B | 0x51 => len == 9,
        0x46 | 0x47 => len == 13,
        0x44 => len == 17,
        _ => false,
    }
}

/// The handler of `r.msg[0]` (a [`Status::Handled`] id) on `w`.
pub fn run<W: PlayerWorld>(w: &mut W, r: &Run<'_>) -> Outcome {
    let (p, m) = (r.player, r.msg);
    let Some(&id) = m.first() else {
        return Outcome::code(3);
    };
    if !size_ok(id, m.len()) {
        return Outcome::code(3);
    }
    match id {
        0x12 => Outcome::code(end_inferno(w, p)),
        0x14 => Outcome::code(overhead_chat(w, p, m)),
        0x15 => Outcome::code(chat(w, p, m, r.clients)),
        0x3D => Outcome::code(highlight_door(w, p, m, r.target)),
        0x3F => Outcome::code(play_audio(w, p, m)),
        0x41 => Outcome::code(resurrect(w, p)),
        0x44 => match staff_in_orifice(w, p, m) {
            Some(c) => Outcome::code(c),
            None => Outcome::STUB,
        },
        0x46 => Outcome::code(merc_interact(w, p, m, r.target)),
        0x47 => Outcome::code(move_merc(w, p, m, r.target)),
        0x48 => Outcome::code(turn_off_busy(w, p)),
        0x4B => Outcome::code(request_entity_update(w, p, m)),
        0x4D => Outcome::code(play_npc_message(w, p, m)),
        0x51 => bind_hotkey(w, p, m),
        0x53 => Outcome::code(stamina_on(w, p)),
        0x54 => Outcome::code(stamina_off(w, p)),
        0x60 => match swap_weapons(w, p) {
            Some(c) => Outcome::code(c),
            None => Outcome::STUB,
        },
        _ => Outcome::STUB,
    }
}

/// 0x12: state 12 off with the update-queue insert; 0.
pub fn end_inferno<W: PlayerWorld>(w: &mut W, p: UnitId) -> u32 {
    w.set_state(p, STATE_INFERNO, false);
    0
}

/// 0x14. The dispatcher's chat check already refused an empty or
/// unterminated text and one of 256 or more characters; the handler's own checks are kept in their order.
pub fn overhead_chat<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8]) -> u32 {
    let body = &m[3..];
    let n = body.iter().position(|&c| c == 0).unwrap_or(body.len());
    if n == 0 {
        return 0;
    }
    if n >= 256 {
        return 2;
    }
    let text = &body[..n];
    // The name cstr after it is copied (≤ 16 bytes) and not used.
    if w.overhead_text_test(text) {
        return 0;
    }
    let frame = w.frame();
    let d = OVERHEAD_PER_CHAR * (n as i32).min(OVERHEAD_MAX_CHARS) + OVERHEAD_BASE;
    // The text shows for `d` base ticks.
    let end = frame.wrapping_add(w.tick_rate().ticks(d));
    w.replace_overhead(p, text, m[2], end);
    w.queue_update(p);
    w.or_flags(p, FLAG_OVERHEAD);
    w.schedule_event(p, EVENT_OVERHEAD, frame.wrapping_add(1).max(end));
    0
}

/// Stat 12 (level), the sender's byte in the chat line.
const STAT_LEVEL: u16 = 12;
/// The chat line S→C 0x26: u8@1 `form`, u8@2 `lang`, u8@3
/// 2, u32@4 0, u8@8 0, u8@9 `level`, the name (at most 15 characters +
/// NUL) at 10, then the text and its NUL.
fn chat_line(form: u8, lang: u8, level: u8, name: &[u8], text: &[u8]) -> Vec<u8> {
    let name = &name[..name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(name.len())
        .min(15)];
    let mut m = vec![0x26, form, lang, 2, 0, 0, 0, 0, 0, level];
    m.extend_from_slice(name);
    m.push(0);
    m.extend_from_slice(text);
    m.push(0);
    m
}

/// Whether `x` is interacting with a player other than `y`.
fn busy_with_other<W: PlayerWorld>(w: &W, x: UnitId, y: UnitId) -> bool {
    w.interacting_player(x).is_some_and(|z| z != y)
}

/// 0x15 chat, after the dispatcher's checks: a broadcast skips a client
/// when the party test ([`PlayerWorld::relation`]) or the interaction test
/// (`busy_with_other`) holds either way round; a whisper skips it on the
/// party test only. The whisper name compare is ASCII case-insensitive.
pub fn chat<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8], clients: &[(UnitId, [u8; 16])]) -> u32 {
    let body = &m[3..];
    let n = body.iter().position(|&c| c == 0).unwrap_or(body.len());
    let text = &body[..n];
    if w.overhead_text_test(text) {
        return 0;
    }
    let rest = body.get(n + 1..).unwrap_or(&[]);
    let t = &rest[..rest
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(rest.len())
        .min(15)];
    let whisper = !t.is_empty();
    // An open line may be a mod game's command, which takes it.
    if !whisper && w.chat_command(p, text) {
        return 0;
    }
    let level = w.stat(p, STAT_LEVEL) as u8;
    let sender = clients
        .iter()
        .find(|&&(u, _)| u == p)
        .map_or(&[][..], |(_, name)| &name[..]);
    let line = chat_line(if whisper { 2 } else { 1 }, m[2], level, sender, text);
    let (mut blocked, mut sent) = (false, false);
    for &(c, ref name) in clients {
        let cname = &name[..name.iter().position(|&b| b == 0).unwrap_or(16)];
        let party = c != p && (w.relation(c, p, 4) || w.relation(p, c, 2));
        if whisper {
            if !cname.eq_ignore_ascii_case(t) {
                continue;
            }
            if party {
                blocked = true;
            } else {
                sent = true;
                w.send(c, &line);
            }
        } else if !(party || (c != p && (busy_with_other(w, p, c) || busy_with_other(w, c, p)))) {
            w.send(c, &line);
        }
    }
    if whisper {
        if blocked {
            w.send(p, &chat_notice(0x0D, t));
        } else if !sent {
            w.send(p, &chat_notice(4, t));
        } else {
            w.send(p, &chat_line(6, 0, level, t, text));
        }
    }
    0
}

/// S→C 0x5A (40 bytes): code u8@1, the name at 8.
fn chat_notice(code: u8, name: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; 40];
    m[0] = 0x5A;
    m[1] = code;
    m[8..8 + name.len()].copy_from_slice(name);
    m
}

/// 0x3D: the unit test (type 2, range 10, made by the caller: `target`);
/// else the door is highlighted; 0.
pub fn highlight_door<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8], target: u32) -> u32 {
    if target != 0 {
        return target;
    }
    let guid = u32_at(m, 1).unwrap_or(0);
    w.highlight_door(p, guid);
    0
}

/// 0x3F: play one of the speech sounds ([`AUDIO_SOUNDS`]) at the player.
pub fn play_audio<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8]) -> u32 {
    let sound = u16_at(m, 1).unwrap_or(0);
    if !AUDIO_SOUNDS.contains(&sound) {
        return 0;
    }
    if w.flags(p) & FLAG_NO_AUDIO != 0 {
        return 1;
    }
    w.sound(p, u32::from(sound), None);
    0
}

/// 0x41: resurrect in town, after the dispatcher's dead gate.
pub fn resurrect<W: PlayerWorld>(w: &mut W, p: UnitId) -> u32 {
    if w.mode(p) != MODE_DEAD {
        return 0;
    }
    if w.hardcore(p) {
        w.drop_client(p, DROP_HARDCORE);
        return 0;
    }
    // The warp's last argument is always 0.
    let warp_arg = 0;
    // The passive states back on.
    for e in w.skill_entries(p) {
        let s = w.passive_state(e.skill);
        if let Ok(s) = u16::try_from(s) {
            if s > 0 {
                w.set_state(p, s, true);
                w.passive_state_apply(p, &e);
            }
        }
    }
    for stat in [STAT_LIFE, STAT_MANA, STAT_STAMINA] {
        let max = w.stat_max(p, stat);
        w.set_stat_send(p, stat, max);
    }
    w.or_flags(p, FLAG_RESURRECTED);
    w.set_state(p, STATE_UNINTERRUPTABLE, true);
    w.set_state(p, STATE_UNINTERRUPTABLE, false);
    let act = d2_sim::drlg::act_of_level(w.current_level(p));
    let town = d2_sim::drlg::ACT_BOUNDARIES[usize::from(act)];
    w.warp(p, town, warp_arg);
    w.start_mode_skip_gate(p, MODE_NEUTRAL);
    w.reselect_hand_skills(p);
    0
}

/// 0x44: busy and trading → 3; else the staff-in-orifice call and its
/// result (`None`: no provider).
pub fn staff_in_orifice<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8]) -> Option<u32> {
    if w.busy(p) && w.trading(p) {
        return Some(3);
    }
    let object = u32_at(m, 5)?;
    let item = u32_at(m, 9)?;
    let action = u16_at(m, 13)?;
    w.staff_in_orifice(p, object, item, action)
}

/// Give the player's mercenary `merc` an AI command (`cmd`, `a`, `b`).
pub fn merc_command<W: PlayerWorld>(
    w: &mut W,
    p: UnitId,
    merc: u32,
    a: i32,
    b: i32,
    cmd: i32,
) -> u32 {
    let Some(m) = w.find_unit(UnitType::Monster, merc) else {
        return 1;
    };
    if w.hireling(p) != Some(m) {
        return 1;
    }
    // The command is five dwords; params 3 and 4 have no
    // defined value, and D2RUST stores 0, 0.
    w.replace_ai_commands(m, [cmd, a, b, 0, 0]);
    w.sound(m, SOUND_MERC_ACK, Some(p));
    0
}

/// 0x46: type ≥ 6 → 2; the unit test (range 50,
/// `target`); then [`merc_command`]'s result.
pub fn merc_interact<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8], target: u32) -> u32 {
    let (merc, guid, ty) = (
        u32_at(m, 1).unwrap_or(0),
        u32_at(m, 5).unwrap_or(0),
        u32_at(m, 9).unwrap_or(u32::MAX),
    );
    if ty >= 6 {
        return 2;
    }
    if target != 0 {
        return target;
    }
    merc_command(w, p, merc, guid as i32, ty as i32, COMMAND_INTERACT)
}

/// 0x47: the point test (range 50, `target`);
/// then [`merc_command`]; 0 whatever it returns.
pub fn move_merc<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8], target: u32) -> u32 {
    if target != 0 {
        return target;
    }
    let merc = u32_at(m, 1).unwrap_or(0);
    let (x, y) = (u16_at(m, 5).unwrap_or(0), u16_at(m, 9).unwrap_or(0));
    merc_command(w, p, merc, i32::from(x), i32::from(y), COMMAND_MOVE);
    0
}

/// 0x48: clear the busy flag; 2 when it was not set.
pub fn turn_off_busy<W: PlayerWorld>(w: &mut W, p: UnitId) -> u32 {
    if !w.busy_flag(p) {
        return 2;
    }
    w.clear_busy_flag(p);
    0
}

/// 0x4B: queue a unit in the player's room for a full resend.
pub fn request_entity_update<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8]) -> u32 {
    let ty = u32_at(m, 1).unwrap_or(u32::MAX);
    let Some(&uty) = UnitType::ALL.get(ty as usize).filter(|_| ty < 6) else {
        return 2;
    };
    let unit = w.find_unit(uty, u32_at(m, 5).unwrap_or(0));
    if ty == 0 && unit != Some(p) {
        return 3;
    }
    let Some(u) = unit else {
        return 1;
    };
    if !w.room_in_list_of(p, u) {
        return 1;
    }
    w.queue_update(u);
    w.or_flags_ex(u, FLAG_EX_RESEND);
    0
}

/// 0x4D: mark the NPC intro of a monster class as played.
pub fn play_npc_message<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8]) -> u32 {
    let class = u16_at(m, 1).unwrap_or(u16::MAX);
    if u32::from(class) >= w.monstats_count() {
        return 2;
    }
    let d = w.difficulty();
    w.clear_npc_intro(p, d, class);
    0
}

/// 0x51: the checks; the slot is stored by the caller
/// ([`SimGame::hotkeys`]); nothing is sent.
pub fn bind_hotkey<W: PlayerWorld>(w: &mut W, p: UnitId, m: &[u8]) -> Outcome {
    let Some(b) = crate::dispatch::bind_hotkey(m) else {
        return Outcome::code(3);
    };
    if usize::from(b.slot) >= HOTKEY_SLOTS {
        return Outcome::code(3);
    }
    let count = w.skill_count();
    let skill = u32::from(b.skill);
    let stored = if skill > count {
        -1
    } else if skill == count || !w.has_skill(p, i32::from(b.skill), b.item) {
        // skill = count, or 0 ≤ skill < count and the player lacks it.
        return Outcome::code(3);
    } else {
        b.skill as i16
    };
    Outcome {
        code: Some(0),
        hotkey: Some((
            usize::from(b.slot),
            HotKey {
                skill: stored,
                left: b.left,
                item: b.item,
            },
        )),
    }
}

/// 0x53: walking with stamina left becomes running.
pub fn stamina_on<W: PlayerWorld>(w: &mut W, p: UnitId) -> u32 {
    if w.mode(p) == MODE_WALK && w.stat(p, STAT_STAMINA) != 0 {
        w.set_mode(p, MODE_RUN);
    }
    0
}

/// 0x54: running becomes walking.
pub fn stamina_off<W: PlayerWorld>(w: &mut W, p: UnitId) -> u32 {
    if w.mode(p) == MODE_RUN {
        w.set_mode(p, MODE_WALK);
    }
    0
}

/// 0x60: swap weapons; `None` when the switch has no provider.
pub fn swap_weapons<W: PlayerWorld>(w: &mut W, p: UnitId) -> Option<u32> {
    if !w.expansion() {
        return Some(3);
    }
    if w.has_used_skill(p) || w.is_dead(p) {
        return Some(0);
    }
    let (r, fail) = w.weapon_switch(p)?;
    Some(if r == 0 && fail { 3 } else { 0 })
}

/// The unit test with range `range` on the staged facts: missing → 1; an
/// item the player owns → 0; another act → 2; else the Chebyshev test (→ 1).
pub fn unit_code(t: UnitTarget, range: u32) -> u32 {
    match t {
        UnitTarget::Missing => 1,
        UnitTarget::OwnedItem => 0,
        UnitTarget::OtherAct => 2,
        UnitTarget::At { player, target } => u32::from(!within(player, target, range)),
    }
}

/// The point test with range `range`: 1 out of range. A player without a staged position fails it.
pub fn point_code(player: Option<Pos>, target: Pos, range: u32) -> u32 {
    match player {
        Some(p) => u32::from(!within(p, target, range)),
        None => 1,
    }
}

fn within(a: Pos, b: Pos, r: u32) -> bool {
    a.x.abs_diff(b.x) <= r && a.y.abs_diff(b.y) <= r
}

/// The handler's own target test, from the staged facts.
fn target_code<D: EventDispatch, W: WorldHost<D>>(
    sim: &SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
) -> u32 {
    match msg[0] {
        0x3D => unit_code(
            sim.unit_target(client, 2, u32_at(msg, 1).unwrap_or(0)),
            DOOR_RANGE,
        ),
        0x46 => {
            let ty = u32_at(msg, 9).unwrap_or(u32::MAX);
            if ty >= 6 {
                // The handler refuses the type first (2).
                return 0;
            }
            unit_code(
                sim.unit_target(client, ty, u32_at(msg, 5).unwrap_or(0)),
                MERC_RANGE,
            )
        }
        0x47 => {
            let target = Pos {
                x: i32::from(u16_at(msg, 5).unwrap_or(0)),
                y: i32::from(u16_at(msg, 9).unwrap_or(0)),
            };
            point_code(sim.player_pos(client), target, MERC_RANGE)
        }
        _ => 0,
    }
}

/// The 0x15 resync of a client whose point target was out of range: the
/// player is queued for update with flag-ex bit 0x10000, which makes its
/// next update send S→C 0x15, exactly as C→S 0x4B for the player's own
/// unit does. A host without the player provider does nothing.
pub fn resync<D: EventDispatch, W: WorldHost<D>>(sim: &mut SimGame<D, W>, client: ClientId) {
    let Some(player) = sim.player_of(client) else {
        return;
    };
    let Some(guid) = sim.game.lists.unit(player).map(|e| e.guid) else {
        return;
    };
    let mut msg = [0u8; 9];
    msg[0] = 0x4B;
    msg[5..9].copy_from_slice(&guid.to_le_bytes());
    let run = Run {
        player,
        msg: &msg,
        target: 0,
        clients: &[],
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let _ = sim.world.player(game, events, run);
}

/// The handler for one dispatched message. `None`: not an id routed
/// here, no player, the host has no [`PlayerWorld`] provider, or the
/// handler reached a call with no provider: the caller keeps its stub.
pub fn handle<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    size: usize,
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    let id = *msg.first()?;
    if !handled(id) {
        return None;
    }
    let msg = &msg[..size.min(msg.len())];
    let player = sim.player_of(client)?;
    let target = if size_ok(id, msg.len()) {
        target_code(sim, client, msg)
    } else {
        0
    };
    let clients: Vec<(UnitId, [u8; 16])> = if id == 0x15 {
        let names = sim.session().map(|s| &s.requests);
        sim.clients()
            .into_iter()
            .filter_map(|c| {
                let unit = sim.player_of(c)?;
                let name = names?.get(&c)?.char_name;
                Some((unit, name))
            })
            .collect()
    } else {
        Vec::new()
    };
    let run = Run {
        player,
        msg,
        target,
        clients: &clients,
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let outcome = sim.world.player(game, events, run)?;
    let mut faults = Vec::new();
    for (unit, bytes) in sim.world.take_sent(&mut sim.events) {
        // A player without a client receives nothing.
        if let Some(c) = sim.client_of(unit) {
            if let Err(e) = out.queue(c, &bytes) {
                faults.push(WorldError::from(e));
            }
        }
    }
    let code = outcome.code?;
    let code = match outcome.hotkey {
        Some((slot, key)) => match sim.client_of(player) {
            Some(c) => {
                sim.set_hotkey(c, slot, key);
                code
            }
            // No client → 3.
            None => 3,
        },
        None => code,
    };
    if !faults.is_empty() {
        for error in faults {
            sim.world.fault(WorldFault { client, id, error });
        }
        return Some(ResultCode::Malformed);
    }
    Some(super::skills::code(code as i32))
}
