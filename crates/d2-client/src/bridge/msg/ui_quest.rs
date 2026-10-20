//! Quest and dialog messages: 0x29 game quest flags, 0x50 quest special,
//! 0x52 quest log status, 0x58 UI open, 0x5E game quest availability.
//! 0x50 code 23 and 0x58 code 5 write the model; the rest is one output
//! each for the UI layer.

use super::super::dispatch::{HandlerError, Message};
use super::super::output::Output;
use super::super::world::{ClientWorld, KindData};
use super::Bytes;

fn len(msg: &Message<'_>, n: usize, what: &'static str) -> Result<(), HandlerError> {
    if msg.bytes.len() == n {
        Ok(())
    } else {
        Err(HandlerError::Invalid(what))
    }
}

/// 0x29 GameQuestInfo: the 96-byte game quest record @1.
pub fn game_quest_flags(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 97, "0x29 is 97 bytes")?;
    let mut record = [0u8; 96];
    record.copy_from_slice(&msg.bytes[1..]);
    w.quest_game = Some(record);
    msg.out.push(Output::GameQuestFlags { record });
    Ok(())
}

/// 0x52 QuestLogInfo: 41 status bytes @1.
pub fn quest_log(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 42, "0x52 is 42 bytes")?;
    let mut status = [0u8; 41];
    status.copy_from_slice(&msg.bytes[1..]);
    w.quest_status = Some(status);
    msg.out.push(Output::QuestLog { status });
    Ok(())
}

/// 0x5E GameQuestAvailability: 37 bytes @1. The client keeps
/// them for client quest byte reads
/// ([`ClientWorld::client_quest_byte`]).
pub fn quest_availability(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 38, "0x5E is 38 bytes")?;
    let mut bytes = [0u8; 37];
    bytes.copy_from_slice(&msg.bytes[1..]);
    w.quest_availability = Some(bytes);
    msg.out.push(Output::QuestAvailability { bytes });
    Ok(())
}

/// 0x50 QuestSpecial: code u16@1, six u16 words @3…@13. Code 23
/// sends C→S 0x69 and sets `exit_requested` (model); codes 1–4, 23, 36
/// emit one `QuestSpecial`.
pub fn quest_special(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 15, "0x50 is 15 bytes")?;
    let b = Bytes(msg.bytes);
    let code = b.u16(1)?;
    let mut words = [0u16; 6];
    for (k, v) in words.iter_mut().enumerate() {
        *v = b.u16(3 + 2 * k)?;
    }
    if code == 23 {
        // (single player: nothing else), then;
        // the two flags are the output's.
        w.outgoing.push(vec![0x69]);
        w.exit_requested = true;
    }
    if code == 1 {
        w.quest_counters = [
            i32::from(words[0]),
            i32::from(words[1]),
            i32::from(words[2]),
        ];
    }
    if code == 36 {
        // The zoo latch of the client chickens
        // (client session
        // state, so the model holds it).
        let l = &mut w.objclient.latches;
        l.zoo = true;
        l.zoo_word = words[0];
    }
    if matches!(code, 1..=4 | 23 | 36) {
        msg.out.push(Output::QuestSpecial { code, words });
    }
    Ok(())
}

/// 0x58 OpenUi: GUID u32@1, code u8@5, arg u8@6. Codes 2, 3 and
/// those past 7 are fatal 0x354; code 5 takes the local player's cursor
/// item out of its inventory (`cursor_item` := none, the item stays in
/// S).
pub fn open_ui(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0x58 is 7 bytes")?;
    let b = Bytes(msg.bytes);
    let code = b.u8(5)?;
    if matches!(code, 2 | 3 | 8..) {
        return Err(HandlerError::Fatal(0x354));
    }
    if code == 5 {
        let local = w.local_player;
        if let Some(KindData::Player(p)) =
            local.and_then(|k| w.units.get_mut(&k)).map(|u| &mut u.kind)
        {
            p.cursor_item = None;
        }
    }
    msg.out.push(Output::OpenUi {
        guid: b.u32(1)?,
        code,
        arg: b.u8(6)?,
    });
    Ok(())
}
