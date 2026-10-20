//! Vendor ids through the host frame: the real trade functions on the
//! seam fake. Only the paths that end before a price is computed run
//! here; the priced paths (buy 56, sell 500) are the vendors module's own
//! vectors (`d2_sim::world::vendors` tests), since their tables have no
//! provider outside `d2-sim`'s test fixtures.
//!
//! Part of the world handler's test suite.

use d2_sim::units::UnitId;
use d2_sim::world::npc;

use crate::adapters::handlers::world::{WorldError, WorldFault};

use super::fake::*;
use super::*;

fn tx(code: u8, guid: u32, gold: u32) -> Vec<u8> {
    npc::transaction(0, code, guid, gold).to_vec()
}

/// A world with an item (GUID 7, mode 4 on the cursor) the player owns,
/// and one (GUID 8) it does not; Charsi is the interact unit when `talk`.
fn setup(talk: bool) -> (TestHost<FakeSim>, UnitId) {
    let (mut s, p) = sim();
    let w = &mut s.world.w;
    for (u, guid, owner) in [(UnitId(2007), 7, Some(p)), (UnitId(2008), 8, None)] {
        w.add(
            u,
            FUnit {
                guid,
                ty: 4,
                mode: 4,
                owner,
                ..FUnit::default()
            },
        );
    }
    if talk {
        w.interact.insert(p, (1, CHARSI));
    }
    (host(s), p)
}

fn assert_clean(h: &TestHost<FakeSim>) {
    assert!(h.game.world.faults.is_empty(), "{:?}", h.game.world.faults);
    assert!(h.game.unhandled.is_empty());
}

// Buy (0x32) refusals that happen before a price is computed.
#[test]
fn buy_refusals_before_the_price() {
    let (mut h, p) = setup(false);
    // NPC not the interact unit → code 9, result 1.
    let (code, got) = send(&mut h, &hex("32 06000000 07000000 00000000 38000000"));
    assert_eq!(
        (code, got),
        (ResultCode::Refused, vec![tx(9, u32::MAX, 500)])
    );
    // Item missing → code 7 with the requested GUID, result 1.
    h.game.world.w.interact.insert(p, (1, CHARSI));
    let (code, got) = send(&mut h, &hex("32 06000000 99000000 00000000 38000000"));
    assert_eq!((code, got), (ResultCode::Refused, vec![tx(7, 0x99, 500)]));
    // Not in the store (t = 0) → code 7 with the requested GUID.
    let (code, got) = send(&mut h, &hex("32 06000000 08000000 00000000 38000000"));
    assert_eq!((code, got), (ResultCode::Refused, vec![tx(7, 8, 500)]));
    assert_clean(&h);
}

// Sell (0x33) refusals that happen before a price is computed.
#[test]
fn sell_refusals_before_the_price() {
    let (mut h, p) = setup(false);
    // Item missing → result 1, no message.
    let (code, got) = send(&mut h, &hex("33 06000000 99000000 0400 0000 f4010000"));
    assert_eq!((code, got), (ResultCode::Refused, vec![]));
    // NPC not the interact unit → code 9, result 1.
    let (code, got) = send(&mut h, &hex("33 06000000 07000000 0400 0000 f4010000"));
    assert_eq!(
        (code, got),
        (ResultCode::Refused, vec![tx(9, u32::MAX, 500)])
    );
    h.game.world.w.interact.insert(p, (1, CHARSI));
    // Not the player's item → code 11, result 3.
    let (code, got) = send(&mut h, &hex("33 06000000 08000000 0400 0000 f4010000"));
    assert_eq!(
        (code, got),
        (ResultCode::Malformed, vec![tx(11, u32::MAX, 500)])
    );
    // Item mode ≠ u16 @9 → code 9, result 3.
    let (code, got) = send(&mut h, &hex("33 06000000 07000000 0000 0000 f4010000"));
    assert_eq!(
        (code, got),
        (ResultCode::Malformed, vec![tx(9, u32::MAX, 500)])
    );
    assert_clean(&h);
}

// Repair (0x35) refusals: not the interact unit, and a vendor that does not repair.
#[test]
fn repair_refusals() {
    let (mut h, p) = setup(false);
    // NPC not the interact unit → code 9 (handler result 0).
    let (code, got) = send(&mut h, &hex("35 06000000 07000000 0000 0000 00000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![tx(9, u32::MAX, 500)]));
    // Akara does not repair → code 9.
    h.game.world.w.interact.insert(p, (1, AKARA));
    let (code, got) = send(&mut h, &hex("35 10000000 07000000 0000 0000 00000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![tx(9, u32::MAX, 500)]));
    assert_clean(&h);
}

// Identify at the gambler (0x37): unknown item, not the last bought item, and success.
#[test]
fn identify_gamble() {
    let (mut h, _) = setup(true);
    // Unknown item → 2; not the last bought item → 3.
    let (code, _) = send(&mut h, &hex("37 99000000"));
    assert_eq!(code, ResultCode::Invalid);
    let (code, _) = send(&mut h, &hex("37 07000000"));
    assert_eq!(code, ResultCode::Malformed);
    // The last bought item: identified, result 0, no message (a fresh
    // client: the transport filters a repeated message).
    let (mut h, p) = setup(true);
    h.game.world.w.last_bought.insert(p, 7);
    let (code, got) = send(&mut h, &hex("37 07000000"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(h.game.world.w.units[&UnitId(2007)].flags & 0x10, 0x10);
    assert_clean(&h);
}

/// A fault a call met inside the host but could not return (the store
/// item send of a trade open: fatal 0x77B with no client player, 0x78C
/// when the item plus 8 reaches 0xFD bytes; it never skips the item) is
/// the handler's own: recorded with the message's client and id, result 3.
#[test]
fn a_store_item_send_fault_is_the_handlers_fault() {
    let (mut h, _) = setup(false);
    let e = d2_sim::items::moves::MoveFatal::MessageSize(0xFD);
    h.game.world.late.push(WorldError::Move(e));
    let (code, _) = send(&mut h, &hex("32 06000000 07000000 00000000 38000000"));
    assert_eq!(code, ResultCode::Malformed);
    assert_eq!(
        h.game.world.faults,
        [WorldFault {
            client: 0,
            id: 0x32,
            error: WorldError::Move(e),
        }]
    );
    assert!(h.game.world.late.is_empty());
}
