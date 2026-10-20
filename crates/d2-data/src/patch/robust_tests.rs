//! Robustness properties of the layer and stack parsers,
//! applying parsed layers, base tables and the diff: any input gives
//! findings, never a panic or a hang.

use proptest::prelude::*;

use super::tests::fixture;
use super::*;
use crate::robust::{bounded, bytes, config, mutated, text_of};

/// Pieces of layer and stack files.
const LAYER_PARTS: &[&[u8]] = &[
    b"d2patch",
    b"d2stack",
    b" 1",
    b" 2",
    b"\n",
    b"\n",
    b"\r\n",
    b"\r",
    b" ",
    b" ",
    b"#",
    b"table",
    b"items",
    b"gear",
    b"recipes",
    b"set",
    b"check",
    b"add",
    b"like",
    b"->",
    b"#0",
    b"#3",
    b"#4",
    b"#01",
    b"#4294967296",
    b"[",
    b"]",
    b"[]",
    b"axe",
    b"clb",
    b"lvl",
    b"dam",
    b"dam@2",
    b"dam@0",
    b"dam@99999999999999999999",
    b"code",
    b"name",
    b"Expansion",
    b"sha:84ec3f726bd38615",
    b"sha:0123",
    b"layer",
    b"a.d2patch",
    b"../x.d2patch",
    b"\t",
    b"\x00",
    b"\x80",
];

const VALID_LAYER: &[u8] = b"d2patch 1\n# c\ntable items\nset axe lvl 1 -> 4\n\
check #1 clb dam@2 0\nadd #4 spr like axe sha:da7b748ddf3353a9\nset #4 spr name Axe -> [Spear x]\n\
table recipes\nset B output x -> y\n";

const VALID_STACK: &[u8] = b"d2stack 1\n\nlayer a.d2patch\n# c\nlayer sub/b-1.d2patch\n";

/// Parses `data` as a layer and a stack, then applies the layer (with
/// whatever statements parsed) to the fixture.
fn parse_and_apply(data: Vec<u8>) {
    bounded(move || {
        let (layer, _) = parse_layer("a.d2patch", &data, 1);
        let _ = parse_stack("s.d2stack", &data);
        let _ = load_stack("s.d2stack", &data, &mut |_| Some(data.clone()));
        let mut d = fixture();
        let _ = apply_stack(&mut d, &[layer.clone(), layer], "s.d2stack");
    });
}

/// Rebuilds the fixture's `items` table from `data` as its base file and
/// diffs `data` against the fixture's `items`.
fn base_and_diff(data: Vec<u8>) {
    bounded(move || {
        let d = fixture();
        let items = d.table("items").unwrap();
        let _ = PatchTable::from_base(&items.rules, "fixture", &data);
        let _ = diff_tables(&[(items, &data)], false);
        let _ = diff_tables(&[(items, &data)], true);
    });
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn layer_arbitrary_bytes(data in bytes(256)) {
        parse_and_apply(data);
    }

    #[test]
    fn layer_alphabet(body in text_of(LAYER_PARTS, 64)) {
        let mut data = b"d2patch 1\ntable items\n".to_vec();
        data.extend(body);
        parse_and_apply(data);
    }

    #[test]
    fn layer_mutated(data in mutated(VALID_LAYER.to_vec())) {
        parse_and_apply(data);
    }

    #[test]
    fn stack_mutated(data in mutated(VALID_STACK.to_vec())) {
        parse_and_apply(data);
    }

    #[test]
    fn diff_mutated(data in mutated(fixture().table("items").unwrap().render())) {
        base_and_diff(data);
    }
}

#[test]
fn valid_inputs_parse() {
    let (layer, f) = parse_layer("a.d2patch", VALID_LAYER, 1);
    assert!(f.is_empty(), "{f:?}");
    let mut d = fixture();
    let f = apply_stack(&mut d, &[layer], "s.d2stack");
    assert!(!has_errors(&f), "{f:?}");
    let (entries, f) = parse_stack("s.d2stack", VALID_STACK);
    assert!(f.is_empty() && entries.len() == 2, "{f:?}");
    let items = fixture().table("items").unwrap().clone();
    assert!(PatchTable::from_base(&items.rules, "fixture", &items.render()).is_ok());
    assert_eq!(
        diff_tables(&[(&items, &items.render())], false).unwrap(),
        b"d2patch 1\n"
    );
}

#[test]
fn regress_apply_statement_after_failed_table_line() {
    // `table` with a bad name is P08 but still opens the table zone, so
    // the `set` parses with no `table` statement before it. Applying that
    // layer gave a panic ("P09 guarantees a table"); it is now P09.
    let (layer, f) = parse_layer(
        "a.d2patch",
        b"d2patch 1\ntable Items\nset axe lvl 1 -> 4\n",
        1,
    );
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].code, Code::P08);
    assert_eq!(layer.statements.len(), 1);
    let mut d = fixture();
    let f = apply_stack(&mut d, &[layer], "s.d2stack");
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].code, f[0].line, f[0].col), (Code::P09, 3, 1));
    assert!(d.changes.is_empty());
}
