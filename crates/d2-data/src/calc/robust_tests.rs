//! Robustness properties (METHODS M07) of the formula compiler, the constant evaluator
//! and the code-buffer validator in `calc`: any input returns, never panics or hangs.

use proptest::prelude::*;

use super::tests::Links;
use super::*;
use crate::robust::{bounded, bytes, config, mutated, text_of};

const FAMILIES: [Family; 3] = [Family::Skills, Family::Missiles, Family::Items];

/// Pieces of formula text: numbers, operators, keywords, names, quotes.
const FORMULA_PARTS: &[&[u8]] = &[
    b"0",
    b"1",
    b"7",
    b"300",
    b"99999",
    b"2147483648",
    b"4294967295",
    b"(",
    b")",
    b"(",
    b")",
    b",",
    b"+",
    b"-",
    b"*",
    b"/",
    b"^",
    b"?",
    b":",
    b"<",
    b">",
    b"=",
    b"!",
    b"<=",
    b">=",
    b"==",
    b"!=",
    b".",
    b"'",
    b"\"",
    b" ",
    b"\t",
    b"min",
    b"max",
    b"rand",
    b"skill",
    b"miss",
    b"stat",
    b"sklvl",
    b"ln12",
    b"par1",
    b"lvl",
    b"base",
    b"mod",
    b"strength",
    b"'Fire Bolt'",
    b"firebolt",
    b".ln12",
    b".blvl",
    b"zzzz",
    b"#",
    b"\x7f",
];

fn compile_all(text: Vec<u8>) {
    bounded(move || {
        for family in FAMILIES {
            if let Ok(c) = compile(family, &Links, &text) {
                // A compiled expression is a valid one-expression buffer.
                if !c.code.is_empty() {
                    let r = validate_buffer(family, &c.code, [0]).unwrap();
                    assert_eq!(r.starts, [0]);
                }
                let _ = eval_const(&c.code);
            }
        }
    });
}

/// A small valid buffer: three compiled expressions back to back.
fn valid_buffer() -> Vec<u8> {
    let mut out = Vec::new();
    for t in ["ln12*2+min(lvl,5)", "7", "skill('Fire Bolt'.blvl)"] {
        out.extend(compile(Family::Skills, &Links, t.as_bytes()).unwrap().code);
    }
    out
}

fn decode_all(buf: Vec<u8>, fields: Vec<u32>) {
    bounded(move || {
        let _ = eval_const(&buf);
        for family in FAMILIES {
            if let Ok(r) = validate_buffer(family, &buf, fields.iter().copied()) {
                assert!(r.starts.iter().all(|&s| s < buf.len()));
                for &s in &r.starts {
                    let _ = eval_const(&buf[s..]);
                }
            }
        }
    });
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn compile_arbitrary_bytes(text in bytes(300)) {
        compile_all(text);
    }

    #[test]
    fn compile_alphabet(text in text_of(FORMULA_PARTS, 80)) {
        compile_all(text);
    }

    #[test]
    fn compile_mutated(text in mutated(b"min(ln12*2,skill('Fire Bolt'.blvl))+(lvl>3?1:par1)".to_vec())) {
        compile_all(text);
    }

    #[test]
    fn buffer_arbitrary_bytes(buf in bytes(128), fields in prop::collection::vec(any::<u32>(), 0..4)) {
        decode_all(buf, fields);
    }

    #[test]
    fn buffer_mutated(buf in mutated(valid_buffer()), fields in prop::collection::vec(0u32..40, 0..4)) {
        decode_all(buf, fields);
    }
}

#[test]
fn valid_inputs_compile_and_validate() {
    let buf = valid_buffer();
    let r = validate_buffer(Family::Skills, &buf, [0]).unwrap();
    assert_eq!(r.starts.len(), 3);
    let c = compile(
        Family::Skills,
        &Links,
        b"min(ln12*2,skill('Fire Bolt'.blvl))+(lvl>3?1:par1)",
    )
    .unwrap();
    assert!(!c.code.is_empty());
}

#[test]
fn deep_nesting_is_bounded() {
    // The parser keeps explicit stacks (64 entries, 1024 output bytes):
    // deep nesting fails the formula, it never recurses.
    for open in ["(", "min(", "-", "1+(", "1?("] {
        let mut text = open.repeat(100_000).into_bytes();
        text.push(b'1');
        text.extend(std::iter::repeat_n(b')', 100_000));
        compile_all(text);
    }
    compile_all(b"1+".repeat(100_000));
    // eval_const on a long buffer of pushes keeps its 64-entry stack.
    let buf = [0x07, 0x01].repeat(100_000);
    assert_eq!(eval_const(&buf), 0);
}
