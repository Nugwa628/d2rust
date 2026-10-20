//! Mutation-testing gaps in `adapters::handlers::items::moves`: the id test that
//! routes a message to the item-move handlers.
//!
//! Integration test of `d2-server`; it pins `is_move_id` and `MOVE_IDS` against the
//! sim's `HANDLED` table, so the dispatcher never sends an item-move id elsewhere.

use d2_server::adapters::handlers::items::moves::{is_move_id, MOVE_IDS};
use d2_sim::items::moves::HANDLED;

/// `is_move_id` is true exactly for the 23 item-move ids
/// (`items::moves::HANDLED`, the `MOVE_IDS` rows) and false for every
/// other id, among them the item ids owned elsewhere (0x2A, 0x4C, 0x4F: the
/// cube and trade handlers).
#[test]
fn move_ids_are_exactly_the_handled_ids() {
    assert_eq!(HANDLED.len(), 23);
    assert_eq!(MOVE_IDS.len(), 23);
    for id in 0..=u8::MAX {
        let handled = HANDLED.iter().any(|&(i, _)| i == id);
        assert_eq!(is_move_id(id), handled, "{id:#04x}");
        assert_eq!(MOVE_IDS.iter().any(|r| r.0 == id), handled, "{id:#04x}");
    }
    for id in [0x2A, 0x4C, 0x4F, 0x15, 0x00] {
        assert!(!is_move_id(id), "{id:#04x}");
    }
}
