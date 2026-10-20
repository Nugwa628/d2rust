//! Tests of `adapters::character`: the load-result to refusal-message table the
//! client shows when a character fails to load.

use super::*;

// Each load result maps to its (m, string id); 17 and 18 switch with `alt_bit`.
#[test]
fn refusal_table() {
    assert_eq!(refusal_message(1, false), (0, 5365));
    assert_eq!(refusal_message(6, false), (5, 5371));
    assert_eq!(refusal_message(7, false), (10, 5373));
    assert_eq!(refusal_message(14, false), (17, 5380));
    assert_eq!(refusal_message(17, false), (20, 5364));
    assert_eq!(refusal_message(17, true), (20, 0x5522));
    assert_eq!(refusal_message(18, false), (21, 5363));
    assert_eq!(refusal_message(18, true), (21, 0x5521));
    assert_eq!(refusal_message(22, false), (9, 5372));
    assert_eq!(refusal_message(23, false), (25, 10101));
    assert_eq!(refusal_message(24, false), (26, 10102));
    assert_eq!(refusal_message(25, false), (27, 5370));
    assert_eq!(refusal_message(26, false), (28, 5371));
    assert_eq!(refusal_message(99, false), (9, 5372));
}
