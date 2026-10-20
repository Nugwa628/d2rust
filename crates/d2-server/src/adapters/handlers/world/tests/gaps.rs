//! The quest data request (C→S 0x40) through the host frame: its framed
//! size and the order of its reply. Part of the world handler tests.

use super::fake::*;
use super::*;
use crate::adapters::sizes::ProtoSizes;
use crate::seams::MessageSizes;

#[test]
fn quest_data_request_is_one_byte() {
    // The handler's size is 1: the framing takes the id byte alone.
    assert_eq!(ProtoSizes.client_size(&[0x40]), Ok(1));
    assert_eq!(ProtoSizes.client_size(&[0x40, 0x7F, 0x7F]), Ok(1));
    // The reply is 0x28 first, 0x52 last.
    let (s, _) = sim();
    let mut h = host(s);
    let (code, got) = send(&mut h, &[0x40]);
    assert_eq!(code, ResultCode::Done);
    let ids: Vec<u8> = got.iter().map(|m| m[0]).collect();
    assert_eq!(ids, [0x28, 0x52]);
    assert!(h.game.world.faults.is_empty());
}
