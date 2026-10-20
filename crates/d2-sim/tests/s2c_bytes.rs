// Spec: specs/world/npc.md §7.3, §8.1, §9; specs/world/waypoints.md §5.3, §7 r7; specs/world/cube.md §1; specs/sim/pathing.md §10
//! Pins the exact bytes of `d2-sim`'s S→C byte builders (HANDOFF §2 step
//! 7k): the layouts the specs give, for a few inputs each, including the
//! extremes of every field. `conformance/tests/s2c_builders.rs` checks
//! the same builders against `d2_proto::s2c`.

use d2_sim::path::walk::messages::player_stop;
use d2_sim::world::cube::trade_action;
use d2_sim::world::npc::{resurrect_message, service_result, transaction};
use d2_sim::world::waypoints::{arrival_message, menu_message};

#[test]
fn npc_transaction_0x2a() {
    assert_eq!(
        transaction(3, 1, 7, 500),
        [0x2A, 3, 1, 0, 0, 0, 0, 7, 0, 0, 0, 0xF4, 1, 0, 0]
    );
    assert_eq!(
        transaction(0, 12, u32::MAX, 0x0102_0304),
        [0x2A, 0, 12, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 4, 3, 2, 1]
    );
    assert_eq!(
        transaction(0xFF, 0xFF, 0x1122_3344, u32::MAX),
        [0x2A, 0xFF, 0xFF, 0, 0, 0, 0, 0x44, 0x33, 0x22, 0x11, 0xFF, 0xFF, 0xFF, 0xFF]
    );
}

#[test]
fn service_result_0x58() {
    assert_eq!(service_result(0x41, 6), [0x58, 0x41, 0, 0, 0, 6, 0]);
    assert_eq!(service_result(0, 0), [0x58, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        service_result(0xA1B2_C3D4, 0xFF),
        [0x58, 0xD4, 0xC3, 0xB2, 0xA1, 0xFF, 0]
    );
}

#[test]
fn resurrect_0x9b() {
    assert_eq!(resurrect_message(), [0x9B, 0xFF, 0xFF, 0, 0, 0, 0]);
}

#[test]
fn waypoint_menu_0x63() {
    let mut rec = [0u8; 16];
    rec[..8].copy_from_slice(&[0x02, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 0x00]);
    assert_eq!(
        menu_message(0x33, &rec),
        [
            0x63, 0x33, 0, 0, 0, 0x02, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 0, 0, 0, 0, 0, 0, 0, 0,
            0
        ]
    );
    let rec: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
    let m = menu_message(u32::MAX, &rec);
    assert_eq!(m[..5], [0x63, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(m[5..], rec);
}

#[test]
fn waypoint_arrival_0x0d() {
    assert_eq!(
        arrival_message(1, 0x1320 - 3, 0x1384 - 3),
        [0x0D, 0, 1, 0, 0, 0, 1, 0x20, 0x13, 0x84, 0x13, 0, 0]
    );
    // Coordinates travel as u16: x + 3 is truncated.
    assert_eq!(
        arrival_message(0xDEAD_BEEF, -3, 0xFFFF - 3 + 0x10000),
        [0x0D, 0, 0xEF, 0xBE, 0xAD, 0xDE, 1, 0, 0, 0xFF, 0xFF, 0, 0]
    );
    assert_eq!(
        arrival_message(0, -4, 0x10000 - 3),
        [0x0D, 0, 0, 0, 0, 0, 1, 0xFF, 0xFF, 0, 0, 0, 0]
    );
}

#[test]
fn player_stop_0x0d() {
    assert_eq!(
        player_stop(0, 0x1122_3344, 5, 0x1234, 0x5678, 6, 99),
        [0x0D, 0, 0x44, 0x33, 0x22, 0x11, 5, 0x34, 0x12, 0x78, 0x56, 6, 99]
    );
    assert_eq!(
        player_stop(0xFF, u32::MAX, 0xFF, u16::MAX, u16::MAX, 0xFF, 0xFF),
        [0x0D; 1]
            .into_iter()
            .chain([0xFF; 12])
            .collect::<Vec<_>>()
            .as_slice()
    );
}

#[test]
fn cube_trade_action_0x77() {
    assert_eq!(trade_action(0x0C), [0x77, 0x0C]);
    assert_eq!(trade_action(0x11), [0x77, 0x11]);
    assert_eq!(trade_action(0x15), [0x77, 0x15]);
}
