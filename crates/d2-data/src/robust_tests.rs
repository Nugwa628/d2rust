//! Robustness properties of the `.txt` reader and the `.bin`
//! container and load checks: malformed input is an error, never a panic,
//! a hang or an unbounded allocation. Calc and patch-layer properties live
//! next to their modules (`calc/robust_tests.rs`, `patch/robust_tests.rs`).

use proptest::prelude::*;

use crate::bin::{post_load_check, BinTable};
use crate::robust::{bounded, bytes, config, mutated, text_of};
use crate::schema::schema;
use crate::strings::StringTables;
use crate::txt::{bind, TxtTable};

/// Pieces of `.txt` files: separators, line ends, the removed-row marker,
/// BOM starts, NUL.
const TXT_PARTS: &[&[u8]] = &[
    b"\t",
    b"\t",
    b"\r\n",
    b"\r\n",
    b"\r",
    b"\n",
    b"a",
    b"code",
    b"name",
    b"1",
    b"-7",
    b"",
    b"Expansion",
    b"expansion",
    b" ",
    b"\"x\"",
    b"\x00",
    b"\xEF\xBB\xBF",
    b"\xFF\xFE",
    b"\x85",
];

fn valid_txt() -> Vec<u8> {
    b"name\tcode\tlvl\r\nAxe\taxe\t1\r\nExpansion\r\nClub\tclb\t\r\n".to_vec()
}

fn parse_txt(data: Vec<u8>) {
    bounded(move || {
        if let Ok(t) = TxtTable::parse("t.txt", &data) {
            // Every record has the header's cell count.
            assert!(t.records.iter().all(|r| r.cells.len() == t.columns()));
            let _ = bind(&t.header, &["name", "code", "lvl"]);
        }
    });
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn txt_arbitrary_bytes(data in bytes(512)) {
        parse_txt(data);
    }

    #[test]
    fn txt_alphabet(data in text_of(TXT_PARTS, 64)) {
        parse_txt(data);
    }

    #[test]
    fn txt_mutated(data in mutated(valid_txt())) {
        parse_txt(data);
    }
}

#[test]
fn txt_valid_input_parses() {
    let t = TxtTable::parse("t.txt", &valid_txt()).unwrap();
    assert_eq!((t.columns(), t.records.len()), (3, 2));
}

/// A `.bin` of `count` zero records of `size` bytes.
fn bin(count: u32, size: usize) -> Vec<u8> {
    let mut d = count.to_le_bytes().to_vec();
    d.resize(4 + count as usize * size, 0);
    d
}

/// Every runtime table, parsed from `data` with its schema record size,
/// then run through the post-load checks against one zero record of each earlier
/// table.
fn parse_and_check(data: Vec<u8>, table: usize) {
    bounded(move || {
        let defs: Vec<_> = schema().runtime().collect();
        let def = defs[table % defs.len()];
        let earlier: Vec<BinTable> = defs
            .iter()
            .map(|d| {
                BinTable::parse(
                    &d.name,
                    "p",
                    &d.bin_name,
                    &bin(1, d.record_size),
                    d.record_size,
                )
                .unwrap()
            })
            .collect();
        if let Ok(t) = BinTable::parse(&def.name, "p", &def.bin_name, &data, def.record_size) {
            assert_eq!(t.records.len(), t.count * t.record_size);
            let _ = post_load_check(&t, &earlier, &StringTables::default(), true);
            let _ = post_load_check(&t, &[], &StringTables::default(), false);
        }
    });
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn bin_arbitrary_bytes(data in bytes(64), size in 0usize..16) {
        bounded(move || {
            let _ = BinTable::parse("x", "p", "x.bin", &data, size);
        });
    }

    #[test]
    fn bin_mutated_tables(table in any::<usize>(), count in 0u32..4, seed in any::<u64>()) {
        let defs: Vec<_> = schema().runtime().collect();
        let def = defs[table % defs.len()];
        // Records of pseudo-random bytes so the checks see varied content.
        let mut data = bin(count, def.record_size);
        let mut x = seed | 1;
        for b in &mut data[4..] {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        parse_and_check(data, table);
    }
}

proptest! {
    #![proptest_config(config(64))]

    #[test]
    fn bin_mutated_container(
        (table, data) in (0..schema().runtime().count()).prop_flat_map(|t| {
            let size = schema().runtime().nth(t).unwrap().record_size;
            (Just(t), mutated(bin(2, size)))
        })
    ) {
        // Count, length and record-byte mutations of a valid table.
        parse_and_check(data, table);
    }
}

#[test]
fn bin_valid_input_parses() {
    for def in schema().runtime() {
        let t = BinTable::parse(
            &def.name,
            "p",
            &def.bin_name,
            &bin(2, def.record_size),
            def.record_size,
        )
        .unwrap();
        assert_eq!(t.count, 2);
    }
}

#[test]
fn regress_bin_huge_record_size() {
    // count × record_size must not overflow (debug panic) for any size.
    let e = BinTable::parse("x", "p", "x.bin", &[0xFF; 4], usize::MAX);
    assert!(e.is_err());
}
