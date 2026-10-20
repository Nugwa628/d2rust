//! Quest ids through the host frame: the real `QuestControl` on the seam
//! fake; the chat end goes through the NPC module's 0x30.
//!
//! Part of the world handler's test suite (see also `quests_act1` and
//! `trade_quests` for the wired host).

use super::fake::*;
use super::*;

// Akara's start message (0x31) opens the talk; the chat end sets the quest slot.
#[test]
fn akara_start_message_then_chat_end() {
    // 0x31 (Akara, message 64) → 0x27, 0x29; the chat end → `5d 01 00 01 0000`;
    // slot 1 = `04 00`.
    let (s, p) = sim();
    let mut h = host(s);
    let (code, got) = send(&mut h, &hex("31 10000000 4000 0000"));
    assert_eq!(code, ResultCode::Done);
    let ids: Vec<u8> = got.iter().map(|m| m[0]).collect();
    assert_eq!(ids, [0x27, 0x29]);
    let (code, got) = send(&mut h, &hex("30 01000000 10000000"));
    assert_eq!(
        (code, got),
        (ResultCode::Done, vec![hex("5d 01 00 01 0000")])
    );
    assert_eq!(h.game.world.w.quests[&p].flags[0].0[2..4], [0x04, 0x00]);
    assert!(h.game.world.faults.is_empty());
}

// Requesting quest data (0x40) sends the quest record (0x28) and the 0x52 block.
#[test]
fn request_quest_data() {
    let (s, p) = sim();
    let mut h = host(s);
    let (code, got) = send(&mut h, &[0x40]);
    assert_eq!(code, ResultCode::Done);
    let record = h.game.world.w.quests[&p].flags[0].0;
    assert_eq!(
        got,
        vec![
            [&hex("28 06 00000000 00")[..], &record[..]].concat(),
            [&[0x52u8][..], &[0u8; 41][..]].concat(),
        ]
    );
    assert!(h.game.world.faults.is_empty());
}

// Quest completed (0x58) sets the log bit; an out-of-range quest id is invalid.
#[test]
fn quest_completed_sets_the_log_bit() {
    let (s, p) = sim();
    let mut h = host(s);
    let (code, got) = send(&mut h, &hex("58 0500"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert!(h.game.world.w.quests[&p].flags[0].get(5, 12));
    let (code, got) = send(&mut h, &hex("58 2a00"));
    assert_eq!((code, got), (ResultCode::Invalid, vec![]));
    assert!(h.game.world.faults.is_empty());
}
