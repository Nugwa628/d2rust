//! Game message entry, dispatcher and gate, and the server side of the
//! handler contract: stubs, the exact-size check, the shared point and unit
//! parsers and the chat checks, all before the sim's handler runs.
//!
//! [`crate::host::Host`] feeds drained client messages into
//! [`process_game_message`]; the sim sits behind the [`Intents`] trait.

use std::collections::BTreeMap;

use crate::seams::{
    ClientId, Intents, MessageSink, MessageSizes, PlayerGate, PlayerLookup, PointState, Pos,
    ResultCode, UnitTarget,
};

/// Ids with a handler-table entry (0x67 entries).
pub const GAME_IDS: u8 = 0x67;

/// Player mode "death".
pub const MODE_DEATH: u32 = 0;
/// Player mode "dead".
pub const MODE_DEAD: u32 = 0x11;

/// Point / unit target range per axis, in subtiles.
pub const TARGET_RANGE: i32 = 50;

/// Frames after the last accepted point target before an out-of-range
/// target resyncs the client (more than 25 frames).
pub const RESYNC_FRAMES: i32 = 25;

/// What the handler-table entry of an id does (the `kind` column of
/// `client-messages.tsv`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Null handler: return 3.
    None,
    /// Stub returning 0.
    Stub0,
    /// Stub returning 3.
    Stub3,
    /// A real handler (the sim's).
    Handler,
}

/// Dispatch gate (the `gate` column of `client-messages.tsv`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Always open.
    None,
    /// Open only while the player is dead.
    Dead,
    /// Open while the player can act: not dead, dying or uninterruptable.
    Alive,
}

/// Handler-table kind of a game id (< 0x67).
pub fn kind(id: u8) -> Kind {
    match id {
        0x00 | 0x2B | 0x4A | 0x4E | 0x55..=0x57 | 0x5A..=0x5C | 0x64 | 0x65 => Kind::None,
        0x2C | 0x2D | 0x39 | 0x45 | 0x52 => Kind::Stub3,
        0x2E | 0x42 | 0x43 | 0x66 => Kind::Stub0,
        _ => Kind::Handler,
    }
}

/// Gate of a game id (ids below 0x14 take the default, alive).
pub fn gate(id: u8) -> Gate {
    match id {
        0x14 | 0x15 | 0x3C | 0x43 | 0x66 => Gate::None,
        0x41 => Gate::Dead,
        _ => Gate::Alive,
    }
}

/// The alive gate: the player is neither uninterruptable nor in death or
/// dead mode.
pub fn gate_alive(p: PlayerGate) -> bool {
    !(p.uninterruptable || p.mode == MODE_DEATH || p.mode == MODE_DEAD)
}

/// Point messages (walk and cast-at-location ids), parsed by `parse`.
pub fn is_point(id: u8) -> bool {
    matches!(id, 0x01 | 0x03 | 0x05 | 0x08 | 0x0C | 0x0F)
}

/// Unit messages (target a unit by type and id), parsed by `parse`.
pub fn is_unit(id: u8) -> bool {
    matches!(
        id,
        0x02 | 0x04 | 0x06 | 0x07 | 0x09 | 0x0A | 0x0D | 0x0E | 0x10 | 0x11
    )
}

/// The 50-subtile Chebyshev range test: the
/// exact |dx| and |dy| for any two positions (no `i32` overflow; the same
/// answer wherever the `i32` difference fits).
pub fn in_range(player: Pos, target: Pos) -> bool {
    let r = TARGET_RANGE.unsigned_abs();
    target.x.abs_diff(player.x) <= r && target.y.abs_diff(player.y) <= r
}

/// Out-of-range point target: does it queue S→C 0x15 (reassign player)?
pub fn needs_resync(frame: i32, last_accept: i32) -> bool {
    frame.wrapping_sub(last_accept) > RESYNC_FRAMES
}

/// 0x3C fields: skill (bits 0–30), left hand (bit 31),
/// item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectSkill {
    /// Skill id.
    pub skill: u32,
    /// Selected for the left hand.
    pub left: bool,
    /// Item id the skill comes from.
    pub item: u32,
}

/// 0x51 fields: skill (bits 0–14), left (bit 15), slot
/// (bits 16–31), item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindHotkey {
    /// Skill id.
    pub skill: u16,
    /// Bound to the left-hand skill.
    pub left: bool,
    /// Hotkey slot.
    pub slot: u16,
    /// Item id the skill comes from.
    pub item: u32,
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// Decodes 0x3C (9 bytes).
pub fn select_skill(b: &[u8]) -> Option<SelectSkill> {
    let w = u32_at(b, 1)?;
    Some(SelectSkill {
        skill: w & 0x7FFF_FFFF,
        left: w >> 31 != 0,
        item: u32_at(b, 5)?,
    })
}

/// Decodes 0x51 (9 bytes).
pub fn bind_hotkey(b: &[u8]) -> Option<BindHotkey> {
    let w = u32_at(b, 1)?;
    Some(BindHotkey {
        skill: (w & 0x7FFF) as u16,
        left: w & 0x8000 != 0,
        slot: (w >> 16) as u16,
        item: u32_at(b, 5)?,
    })
}

fn strlen(b: &[u8]) -> Option<usize> {
    b.iter().position(|&c| c == 0)
}

/// The server's record of a client (host-only fields).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientRecord {
    /// The host clock (ms) of the last game message.
    pub last_message_ms: u32,
}

/// What became of one drained game message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The client is in no game; dropped.
    NotInGame,
    /// The client's game has no player unit for it; dropped.
    NoPlayer,
    /// Dispatched; the result code (the host ignores it).
    Dispatched(ResultCode),
}

/// A state that cannot occur in a consistent host.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    /// The game knows the client but the host has no record for it.
    #[error("client {0} is in a game but has no client record")]
    NoClientRecord(ClientId),
    /// The message sink refused a queue.
    #[error("sink: {0}")]
    Sink(#[from] crate::buffers::QueueError),
}

/// Game message entry for one drained queue-1 message: finds the client's
/// game and player, stamps the client record, then dispatches.
#[allow(clippy::too_many_arguments)]
pub fn process_game_message(
    game: &mut impl Intents,
    sizes: &impl MessageSizes,
    records: &mut BTreeMap<ClientId, ClientRecord>,
    out: &mut dyn MessageSink,
    client: ClientId,
    msg: &[u8],
    size: usize,
    now_ms: u32,
) -> Result<Outcome, DispatchError> {
    let lookup = game.player(client);
    if lookup == PlayerLookup::NotInGame {
        return Ok(Outcome::NotInGame);
    }
    let record = records
        .get_mut(&client)
        .ok_or(DispatchError::NoClientRecord(client))?;
    record.last_message_ms = now_ms;
    let PlayerLookup::Player(player) = lookup else {
        return Ok(Outcome::NoPlayer);
    };
    Ok(Outcome::Dispatched(dispatch(
        game, sizes, out, client, player, msg, size,
    )))
}

/// Dispatcher: id check, handler kind, gate, size check, target parse,
/// then the sim's handler.
pub fn dispatch(
    game: &mut impl Intents,
    sizes: &impl MessageSizes,
    out: &mut dyn MessageSink,
    client: ClientId,
    player: PlayerGate,
    msg: &[u8],
    size: usize,
) -> ResultCode {
    let id = match msg.first() {
        Some(&id) if id != 0 && id < GAME_IDS => id,
        _ => return ResultCode::Malformed,
    };
    let kind = kind(id);
    if kind == Kind::None {
        return ResultCode::Malformed;
    }
    let open = match gate(id) {
        Gate::None => true,
        Gate::Dead => player.mode == MODE_DEAD,
        Gate::Alive => gate_alive(player),
    };
    if !open {
        return ResultCode::Done;
    }
    // The game's sync timer is host-only and never read.
    match kind {
        Kind::Stub0 => return ResultCode::Done,
        Kind::Stub3 => return ResultCode::Malformed,
        _ => {}
    }
    if let Err(code) = check_size(sizes, id, msg, size) {
        return code;
    }
    if is_point(id) || is_unit(id) {
        game.refresh_targets(client, msg);
    }
    if let Err(code) = parse(game, out, client, id, msg) {
        return code;
    }
    game.handle(client, msg, size, out)
}

/// Every handler checks its exact size first (transport and handler sizes
/// agree, so the fixed C→S size is the handler's), except 0x14 (4..=275)
/// and 0x15, whose handler checks its strings (the refusal code for that
/// is not pinned down, so it stays in the handler).
fn check_size(
    sizes: &impl MessageSizes,
    id: u8,
    msg: &[u8],
    size: usize,
) -> Result<(), ResultCode> {
    let ok = match id {
        0x14 => (4..=275).contains(&size),
        0x15 => true,
        _ => sizes.client_size(msg) == Ok(size),
    };
    if ok {
        Ok(())
    } else {
        Err(ResultCode::Malformed)
    }
}

/// The server-side parse: point and unit targets and the chat strings of
/// 0x14 and 0x15. Other field checks belong to the handlers.
fn parse(
    game: &mut impl Intents,
    out: &mut dyn MessageSink,
    client: ClientId,
    id: u8,
    msg: &[u8],
) -> Result<(), ResultCode> {
    if is_point(id) {
        let Some(PointState {
            player,
            last_accept,
        }) = game.point_state(client)
        else {
            return Err(ResultCode::Invalid);
        };
        let target = Pos {
            x: i32::from(u16_at(msg, 1).unwrap_or(0)),
            y: i32::from(u16_at(msg, 3).unwrap_or(0)),
        };
        let frame = game.frame();
        if !in_range(player, target) {
            if needs_resync(frame, last_accept) {
                game.queue_resync(client, out);
            }
            return Err(ResultCode::Refused);
        }
        game.set_point_accept(client, frame);
    } else if is_unit(id) {
        let unit_type = u32_at(msg, 1).unwrap_or(u32::MAX);
        if unit_type >= 6 {
            return Err(ResultCode::Invalid);
        }
        let unit_id = u32_at(msg, 5).unwrap_or(0);
        match game.unit_target(client, unit_type, unit_id) {
            UnitTarget::Missing => return Err(ResultCode::Refused),
            UnitTarget::OwnedItem => {}
            UnitTarget::OtherAct => return Err(ResultCode::Invalid),
            UnitTarget::At { player, target } => {
                if !in_range(player, target) {
                    return Err(ResultCode::Refused);
                }
            }
        }
    } else if id == 0x14 {
        // msg = cstr at +3, strlen < 256 (a missing NUL fails too); strlen
        // 0 is done with no effect.
        match msg.get(3..).and_then(strlen) {
            Some(0) => return Err(ResultCode::Done),
            Some(n) if n < 256 => {}
            _ => return Err(ResultCode::Invalid),
        }
    } else if id == 0x15 {
        // msg = cstr at +3 with strlen < 256 and strlen + 4 < size (a
        // missing NUL fails too), else rejected as invalid (code 2): both
        // tests share one rejection.
        match msg.get(3..).and_then(strlen) {
            Some(n) if n < 256 && n + 4 < msg.len() => {}
            _ => return Err(ResultCode::Invalid),
        }
    }
    Ok(())
}
