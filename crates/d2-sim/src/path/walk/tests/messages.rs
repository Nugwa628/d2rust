//! Builders checked against `sim/server-messages.tsv` (size and every
//! field offset of the rows 0x0D, 0x0F, 0x10, 0x15, 0x96).

use super::super::messages::*;
use super::super::seams::{Point, TargetUnit};
use crate::path::coords::to_fp16_center as centre;
use crate::path::record::DynamicPath;
use crate::units::{UnitId, UnitType};

const SERVER_TSV: &str = include_str!("../../../../../../tables/sim/server-messages.tsv");

/// (size, layout) of a row.
fn row(id: &str) -> (usize, String) {
    let line = SERVER_TSV
        .lines()
        .find(|l| l.split('\t').next() == Some(id))
        .unwrap_or_else(|| panic!("{id} missing"));
    let c: Vec<&str> = line.split('\t').collect();
    (c[2].parse().unwrap(), c[3].to_string())
}

/// Checks a byte layout `name:u8@1 …`: every byte is either the id or
/// inside exactly one field, and `fields[name]` reads back `value`.
fn check_layout(id: &str, msg: &[u8], expect: &[(&str, u64)]) {
    let (size, layout) = row(id);
    assert_eq!(msg.len(), size, "{id} size");
    assert_eq!(msg[0] as u64, u64::from_str_radix(&id[2..], 16).unwrap());
    let mut covered = vec![false; size];
    covered[0] = true;
    for f in layout.split_whitespace() {
        let (name, rest) = f.split_once(':').unwrap();
        let (ty, at) = rest.split_once('@').unwrap();
        let at: usize = at.parse().unwrap();
        let w = match ty {
            "u8" => 1,
            "u16" => 2,
            "u32" => 4,
            _ => panic!("{ty}"),
        };
        let mut v = 0u64;
        for k in 0..w {
            assert!(!covered[at + k], "{id} {name} overlaps");
            covered[at + k] = true;
            v |= (msg[at + k] as u64) << (8 * k);
        }
        let want = expect.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
        assert_eq!(Some(v), want, "{id} field {name}");
    }
    assert!(covered.iter().all(|c| *c), "{id} bytes not covered");
}

#[test]
fn byte_builders_match_tsv() {
    check_layout(
        "0x0D",
        &player_stop(0, 0x11223344, 5, 0x1234, 0x5678, 6, 99),
        &[
            ("type", 0),
            ("guid", 0x11223344),
            ("a", 5),
            ("x", 0x1234),
            ("y", 0x5678),
            ("b", 6),
            ("life_pct", 99),
        ],
    );
    check_layout(
        "0x0F",
        &player_move(0, 7, 0x17, 300, 301, 290, 291),
        &[
            ("type", 0),
            ("guid", 7),
            ("code", 0x17),
            ("target_x", 300),
            ("target_y", 301),
            ("zero", 0),
            ("x", 290),
            ("y", 291),
        ],
    );
    check_layout(
        "0x10",
        &player_to_target(0, 7, 0x18, 1, 0xAABBCCDD, 290, 291),
        &[
            ("type", 0),
            ("guid", 7),
            ("code", 0x18),
            ("target_type", 1),
            ("target_guid", 0xAABBCCDD),
            ("x", 290),
            ("y", 291),
        ],
    );
    check_layout(
        "0x15",
        &reassign_player(0, 7, 5000, 6000, 1),
        &[
            ("type", 0),
            ("guid", 7),
            ("x", 5000),
            ("y", 6000),
            ("flag", 1),
        ],
    );
}

#[test]
fn walk_verify_bits_match_tsv() {
    let (size, layout) = row("0x96");
    let msg = walk_verify(0x5A5A, 0xBEEF, 0x1234, -3, 7);
    assert_eq!(msg.len(), size);
    let fields = layout.strip_prefix("bits: ").unwrap();
    let mut at = 0usize;
    let read = |at: usize, w: usize| -> u64 {
        (0..w).fold(0u64, |v, i| {
            v | ((((msg[(at + i) / 8] >> ((at + i) % 8)) & 1) as u64) << i)
        })
    };
    let expect = [
        ("id", 0x96u64),
        ("stamina", 0x5A5A & 0x7FFF),
        ("x", 0xBEEF),
        ("y", 0x1234),
        ("dx", 0xFD),
        ("dy", 7),
    ];
    for f in fields.split_whitespace() {
        let (name, w) = f.split_once(':').unwrap();
        let w: usize = w.parse().unwrap();
        let want = expect.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(read(at, w), want, "0x96 {name}");
        at += w;
    }
    assert_eq!(at.div_ceil(8), size);
    // M08: a one-bit change of x moves exactly one bit of the message.
    let other = walk_verify(0x5A5A, 0xBEEF ^ 1, 0x1234, -3, 7);
    let diff: u32 = msg
        .iter()
        .zip(other.iter())
        .map(|(a, b)| (a ^ b).count_ones())
        .sum();
    assert_eq!(diff, 1);
}

fn moving_path(target_unit: Option<TargetUnit>) -> DynamicPath {
    let mut p = DynamicPath {
        owner: Some(UnitId(1)),
        ..DynamicPath::default()
    };
    p.precise_x = centre(290);
    p.precise_y = centre(291);
    p.put_target(Point::new(300, 301));
    p.target_unit = target_unit;
    p
}

#[test]
fn update_pass_message_choice() {
    let p = moving_path(None);
    assert_eq!(mode_update(2, 0, 7, &p, None, true), None);
    assert_eq!(
        mode_update(2, 0, 7, &p, None, false),
        Some(player_move(0, 7, 1, 300, 301, 290, 291))
    );
    assert_eq!(
        mode_update(6, 0, 7, &p, None, false),
        Some(player_move(0, 7, 1, 300, 301, 290, 291))
    );
    assert_eq!(
        mode_update(3, 0, 7, &p, None, false),
        Some(player_move(0, 7, 0x17, 300, 301, 290, 291))
    );
    let tu = TargetUnit {
        unit: UnitId(2),
        ty: UnitType::Monster,
        guid: 0x44,
    };
    let p = moving_path(Some(tu));
    assert_eq!(
        mode_update(2, 0, 7, &p, Some((1, 0x44)), false),
        Some(player_to_target(0, 7, 0, 1, 0x44, 290, 291))
    );
    assert_eq!(
        mode_update(3, 0, 7, &p, Some((1, 0x44)), false),
        Some(player_to_target(0, 7, 0x18, 1, 0x44, 290, 291))
    );
    assert_eq!(mode_update(1, 0, 7, &p, Some((1, 0x44)), false), None);
    // A path target the caller filtered out
    // gives the point message.
    assert_eq!(
        mode_update(2, 0, 7, &p, None, false),
        Some(player_move(0, 7, 1, 300, 301, 290, 291))
    );
    // 0x15: 0x10000 → flag 1 to every client; 0x800 only to others.
    assert_eq!(reassign_flag(0x10000, true), Some(1));
    assert_eq!(reassign_flag(0x800, true), None);
    assert_eq!(reassign_flag(0x800, false), Some(0));
    assert_eq!(reassign_flag(0, false), None);
}
