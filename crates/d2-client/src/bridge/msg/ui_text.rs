//! Text and small UI messages: 0x26 chat and overhead text, 0x27 NPC
//! text, 0x78 trade partner, 0x5A event text, 0x61 act video, 0x76
//! overhead clear and 0x7B skill hotkey. Each emits one output (with the
//! model facts its base-game handler reads, captured at receive); only 0x5A
//! code 0x12 writes the model (the act environment).

use super::super::dispatch::{HandlerError, Message};
use super::super::output::Output;
use super::super::world::{ClientWorld, KindData, UnitKey, MONSTER, OBJECT, PLAYER};
use super::lighting::event_eclipse;
use super::Bytes;

fn len(msg: &Message<'_>, n: usize, what: &'static str) -> Result<(), HandlerError> {
    if msg.bytes.len() == n {
        Ok(())
    } else {
        Err(HandlerError::Invalid(what))
    }
}

/// The bytes of the C string at `off` (up to its NUL or the message
/// end), cut to `max` bytes; and the offset after its NUL.
fn cstr(b: &[u8], off: usize, max: usize) -> (Vec<u8>, usize) {
    let s = b.get(off..).unwrap_or(&[]);
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    (s[..n.min(max)].to_vec(), off + n + 1)
}

fn player_name(w: &ClientWorld, key: UnitKey) -> Option<[u8; 16]> {
    match &w.units.get(&key)?.kind {
        KindData::Player(p) if key.unit_type == PLAYER => Some(p.name),
        _ => None,
    }
}

/// 0x26 Chat: type u8@1, lang u8@2, unit type u8@3, GUID
/// u32@4, u8@8, u8@9, name cstr @10 (≤ 16 bytes copied), text cstr after
/// the name's NUL (≤ 256 bytes copied).
pub fn chat(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let unit = UnitKey::new(b.u8(3)?, b.u32(4)?);
    let (name, next) = cstr(msg.bytes, 10, 16);
    let (text, _) = cstr(msg.bytes, next, 256);
    msg.out.push(Output::ChatLine {
        kind: b.u8(1)?,
        lang: b.u8(2)?,
        unit,
        b8: b.u8(8)?,
        b9: b.u8(9)?,
        name,
        text,
        present: w.units.contains_key(&unit),
        player_name: player_name(w, unit),
    });
    Ok(())
}

/// 0x27 NpcInfo: the 40 bytes; whether (1, GUID) or (2, GUID) is
/// in S by the type u8@1, and a type-2 object's class.
pub fn npc_text(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 40, "0x27 is 40 bytes")?;
    let b = Bytes(msg.bytes);
    let ty = b.u8(1)?;
    let key = UnitKey::new(ty, b.u32(2)?);
    let unit = (ty == MONSTER || ty == OBJECT)
        .then(|| w.units.get(&key))
        .flatten();
    let mut bytes = [0u8; 40];
    bytes.copy_from_slice(msg.bytes);
    msg.out.push(Output::NpcText {
        bytes,
        present: unit.is_some(),
        object_class: if ty == OBJECT {
            unit.map_or(0, |u| u.class)
        } else {
            0
        },
    });
    Ok(())
}

/// 0x78 TradeAccepted: name 16 bytes @1, GUID u32@17.
pub fn trade_partner(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 21, "0x78 is 21 bytes")?;
    let b = Bytes(msg.bytes);
    let mut name = [0u8; 16];
    name.copy_from_slice(b.slice(1, 16)?);
    msg.out.push(Output::TradePartner {
        name,
        guid: b.u32(17)?,
    });
    Ok(())
}

/// 0x5A EventMessage: the 40 bytes with the name @8 cut to 15
/// characters (byte 0x17 := 0 when it is longer); code 0x12 also sets
/// the act environment. One `EventText` with the local player's
/// name.
pub fn event_text(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 40, "0x5A is 40 bytes")?;
    let mut bytes = [0u8; 40];
    bytes.copy_from_slice(msg.bytes);
    if !bytes[8..0x17].contains(&0) {
        bytes[0x17] = 0;
    }
    if bytes[1] == 0x12 {
        event_eclipse(w)?;
        // The UI layer's camera call with (6, 4000, 10000, 4000)
        // (the camera's first row), kept in the model so the
        // frame's camera reads it.
        w.start_shake(6, 4000, 10000, 4000);
    }
    let local_name = w.local_player.and_then(|k| player_name(w, k));
    msg.out.push(Output::EventText { bytes, local_name });
    Ok(())
}

/// 0x61 CanGoToAct: video u8@1.
pub fn act_video(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 2, "0x61 is 2 bytes")?;
    let video = Bytes(msg.bytes).u8(1)?;
    w.mark_session(crate::bridge::world::SessionMark::Video(video));
    msg.out.push(Output::ActVideo { video });
    // The video handler ends with the 1-byte send:
    // c2s 0x48 TurnOffBusyState (sent after every 0x61 in the base
    // game).
    w.outgoing.push(vec![0x48]);
    Ok(())
}

/// 0x76 PlayerInProximity: type u8@1, GUID u32@2.
pub fn overhead_clear(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 6, "0x76 is 6 bytes")?;
    let b = Bytes(msg.bytes);
    msg.out.push(Output::OverheadClear {
        unit: UnitKey::new(b.u8(1)?, b.u32(2)?),
    });
    Ok(())
}

/// 0x7B AssignHotkey: slot u8@1, skill u16@2 (bits 0–11 skill, −1
/// when greater than the skill count; bit 15 left), item GUID u32@4.
pub fn hotkey(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 8, "0x7B is 8 bytes")?;
    let b = Bytes(msg.bytes);
    let word = b.u16(2)?;
    let skill = i32::from(word & 0xFFF);
    let count = msg.inputs.tables.skills.len() as i32;
    msg.out.push(Output::HotkeyAssign {
        slot: b.u8(1)?,
        skill: if skill > count { -1 } else { skill },
        left: word & 0x8000 != 0,
        item: b.u32(4)?,
    });
    Ok(())
}
