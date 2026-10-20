//! Tests for the table-specific field callbacks in `compile::callbacks`: cube inputs and
//! outputs, skill modes, component choices, preset placement and the text helpers.
//!
//! The linkers are synthetic, so no game files are needed; they are filled so that the
//! item, item type, unique and set indices match the values the vectors use.

use super::*;
use crate::compile::{compile_table, Linker, StdCallbacks};
use crate::schema::{FieldDef, FieldType, Link};
use crate::strings::StringTables;
use crate::txt::{ErrorCode, TxtTable};

/// A code linker with `codes[i]` at the given indices, filler elsewhere.
fn code_linker(n: u32, codes: &[(&[u8], u32)]) -> Linker {
    let mut l = CodeLinker::default();
    for i in 0..n {
        let code = codes
            .iter()
            .find(|(_, at)| *at == i)
            .map(|(c, _)| code_u32(c))
            .unwrap_or(0x8000_0000 + i);
        l.add(code);
    }
    Linker::Code(l)
}

fn name_linker(n: u32, names: &[(&[u8], u32)]) -> Linker {
    let mut l = NameLinker::default();
    for i in 0..n {
        let key = names
            .iter()
            .find(|(_, at)| *at == i)
            .map(|(k, _)| name_key(k).unwrap())
            .unwrap_or_else(|| format!("\x01filler{i}").into_bytes());
        l.add_always(&key);
    }
    Linker::Name(l)
}

/// Indices used by the vectors: items `hax` 0, `axe` 1, `lrg` 330, `amu` 520, `rin`
/// 522; item types row 0 `"    "`, `axe` 28, `pole` 34, `gem2` 93; unique
/// 122 `The Stone of Jordan` (`rin`, lvl 39); set item 0 `Civerb's Ward`
/// (`lrg`, lvl 13).
fn setup() -> (Linkers, SpecialItems) {
    let mut l = Linkers::default();
    l.insert(
        "items.code",
        code_linker(
            600,
            &[
                (b"hax", 0),
                (b"axe", 1),
                (b"lrg", 330),
                (b"amu", 520),
                (b"rin", 522),
            ],
        ),
    );
    l.insert(
        "itemtypes.code",
        code_linker(100, &[(b"", 0), (b"axe", 28), (b"pole", 34), (b"gem2", 93)]),
    );
    l.insert(
        UNIQUES_LINKER,
        name_linker(130, &[(b"The Stone of Jordan", 122)]),
    );
    l.insert(SETS_LINKER, name_linker(5, &[(b"Civerb's Ward", 0)]));
    let sp = SpecialItems {
        uniques: (0..130)
            .map(|i| SpecialItem {
                code: if i == 122 { code_u32(b"rin") } else { 0 },
                lvl: if i == 122 { 39 } else { 0 },
            })
            .collect(),
        sets: vec![
            SpecialItem {
                code: code_u32(b"lrg"),
                lvl: 13,
            };
            5
        ],
    };
    let modes = [
        "DT", "NU", "WL", "GH", "A1", "A2", "BL", "SC", "S1", "S2", "S3", "S4", "DD", "KB", "xx",
        "RN",
    ];
    let mut mm = CodeLinker::default();
    for m in modes {
        mm.add(code_u32(m.as_bytes()));
    }
    l.insert("monmode_lookup.code", Linker::Code(mm));
    l.insert(
        "monseq.sequence",
        name_linker(10, &[(b"", 0), (b"seq_skeletonraise", 8)]),
    );
    l.insert(
        "compcode.code",
        code_linker(10, &[(b"nil", 0), (b"lit", 1), (b"med", 2), (b"hvy", 4)]),
    );
    l.insert(
        "superuniques.Superunique",
        name_linker(10, &[(b"Griswold", 5), (b"The Countess", 6)]),
    );
    l.insert(
        "monstats.Id",
        name_linker(400, &[(b"gheed", 147), (b"griswold", 365)]),
    );
    l.insert("monplace.code", name_linker(5, &[]));
    (l, sp)
}

/// Runs `cb(<name>)` with field-list slot `slot` on `record`.
fn call(
    name: &str,
    slot: u32,
    text: Option<&[u8]>,
    record: &mut [u8],
) -> Result<Vec<DiagKind>, CallbackError> {
    let (l, sp) = setup();
    let f = FieldDef::new(
        "x",
        FieldType::CustomLink,
        0,
        slot,
        Link::Table(name.to_owned()),
    );
    let mut diags = Vec::new();
    let known = run(
        name,
        FieldCall {
            field: &f,
            text,
            record,
            record_index: 0,
            slot: 0,
            linkers: &l,
            diagnostics: &mut diags,
        },
        &sp,
    )?;
    assert!(known);
    Ok(diags)
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

fn input(text: &str) -> Result<Vec<u8>, ErrorCode> {
    let mut r = vec![0u8; 328];
    call("cubemain.input", 1, Some(text.as_bytes()), &mut r).map_err(|e| e.code)?;
    Ok(r[28..36].to_vec())
}

#[test]
fn cube_inputs() {
    for (text, bytes) in [
        ("any", "01 00 ff ff 00 00 00 00"),
        ("\"ANY,nos\"", "05 00 ff ff 00 00 00 00"),
        ("\"gem2,qty=3\"", "02 00 5d 00 00 00 00 03"),
        ("axe", "02 00 1c 00 00 00 00 00"),
        ("The Stone of Jordan", "41 00 0a 02 7b 00 07 00"),
        ("\"Civerb's Ward,mag\"", "41 00 4a 01 01 00 04 00"),
        ("\"hax,sock=2,eth\"", "09 00 00 00 00 00 00 00"),
        ("\",qty=2\"", "02 00 00 00 00 00 00 02"),
        ("\"rin,qty=300\"", "01 00 0a 02 00 00 00 2c"),
        ("\"rin,qty= 7x\"", "01 00 0a 02 00 00 00 07"),
        ("\"rin,qty=-1\"", "01 00 0a 02 00 00 00 ff"),
        ("\"rin,qty=99999999999\"", "01 00 0a 02 00 00 00 ff"),
        ("rin\",mag\"", "01 00 0a 02 00 00 00 00"),
        ("\"qqq,mag\"", "00 00 00 00 00 00 00 00"),
        ("", "00 00 00 00 00 00 00 00"),
        ("\"\"", "00 00 00 00 00 00 00 00"),
    ] {
        assert_eq!(input(text), Ok(hex(bytes)), "{text}");
    }
    assert_eq!(input("\"rin,qty\""), Err(ErrorCode::E15));
}

fn output(text: &str, slot: u32) -> (Vec<u8>, Vec<DiagKind>) {
    let mut r = vec![0u8; 328];
    let d = call("cubemain.output", slot, Some(text.as_bytes()), &mut r).unwrap();
    let o = 76 + 84 * slot as usize;
    (r[o..o + 24].to_vec(), d)
}

#[test]
fn cube_outputs() {
    let z = |n: usize| vec!["00"; n].join(" ");
    for (text, bytes) in [
        ("Cow Portal", format!("{} 01 {}", z(8), z(15))),
        ("cow portal", format!("{} 01 {}", z(8), z(15))),
        (
            "Pandemonium Finale Portal",
            format!("{} 03 {}", z(8), z(15)),
        ),
        (
            "\"usetype,mag,suf=162\"",
            "00 00 00 00 00 00 04 00 ff 00 00 00 00 00 00 00 00 00 a2 00 00 00 00 00".into(),
        ),
        (
            "\"useitem,sock=1\"",
            format!("02 00 00 00 00 00 00 01 fe {}", z(15)),
        ),
        (
            "\"useitem,reg\"",
            format!("40 00 00 00 00 00 00 00 ff {}", z(15)),
        ),
        (
            "\"amu,mag,pre=331\"",
            format!("00 00 08 02 00 00 04 00 fc 00 00 00 4b 01 {}", z(10)),
        ),
        (
            "\"pole,mag,pre=191\"",
            format!("00 00 22 00 00 00 04 00 fd 00 00 00 bf 00 {}", z(10)),
        ),
        (
            "\"rin,pre=1,pre=2,pre=3,pre=4\"",
            "00 00 0a 02 00 00 00 00 fc 00 00 00 01 00 02 00 03 00 04 00 00 00 00 00".into(),
        ),
        (
            "The Stone of Jordan",
            format!("08 00 0a 02 7b 00 07 00 fc 00 00 27 {}", z(12)),
        ),
        (
            "\"Civerb's Ward,qty=2\"",
            format!("08 00 4a 01 01 00 05 00 fc 00 00 0d {}", z(12)),
        ),
        ("\"usetype,foo,mag\"", format!("{} ff {}", z(8), z(15))),
        ("\"Useitem\"", z(24)),
        ("\"\"", format!("{} fd {}", z(8), z(15))),
        ("axe", format!("00 00 01 00 {} fc {}", z(4), z(15))),
    ] {
        assert_eq!(output(text, 0).0, hex(&bytes), "{text}");
    }
    assert_eq!(output("\"usetype,foo,mag\"", 0).1, [DiagKind::CbStop]);
    assert_eq!(output("\"Civerb's Ward,qty=2\"", 0).1, [DiagKind::CbStop]);
    assert_eq!(output("\"Useitem\"", 0).1, [DiagKind::CbMiss]);
    assert_eq!(output("\"rin,mag,\"", 0).1, []);
}

#[test]
fn cube_output_overflow() {
    let mut r = vec![0u8; 328];
    let pres = |n: usize| format!("\"rin{}\"", ",pre=1".repeat(n));
    let t = pres(36);
    assert!(call("cubemain.output", 2, Some(t.as_bytes()), &mut r).is_ok());
    let t = pres(37);
    let e = call("cubemain.output", 2, Some(t.as_bytes()), &mut r).unwrap_err();
    assert_eq!(e.code, ErrorCode::E15);
    let t = format!("\"rin{}\"", ",suf=1".repeat(34));
    let e = call("cubemain.output", 2, Some(t.as_bytes()), &mut r).unwrap_err();
    assert_eq!(e.code, ErrorCode::E15);
    let e = call("cubemain.output", 0, Some(b"\"rin,sock\""), &mut r).unwrap_err();
    assert_eq!(e.code, ErrorCode::E15);
}

fn skillmode(text: Option<&str>, skill: i16) -> (u8, u16) {
    let mut r = vec![0xAAu8; 424];
    r[368 + 4..370 + 4].copy_from_slice(&skill.to_le_bytes());
    call("monstats.skillmode", 2, text.map(str::as_bytes), &mut r).unwrap();
    (r[386], u16::from_le_bytes([r[396], r[397]]))
}

#[test]
fn skill_modes() {
    assert_eq!(skillmode(Some("A1"), 321), (4, 0xFFFF));
    assert_eq!(skillmode(Some("KB"), 5), (13, 0xFFFF));
    assert_eq!(skillmode(Some("a1"), 5), (14, 0xFFFF));
    assert_eq!(skillmode(Some("xx"), 5), (14, 0xFFFF));
    assert_eq!(skillmode(Some("seq_skeletonraise"), 158), (14, 8));
    assert_eq!(skillmode(Some("SEQ_SKELETONRAISE"), 5), (14, 8));
    assert_eq!(skillmode(Some(""), 5), (14, 0));
    assert_eq!(skillmode(Some("A1"), -1), (0, 0xFFFF));
    assert_eq!(skillmode(None, 5), (0, 0xFFFF));
}

fn composit(text: Option<&str>) -> (u8, Vec<u8>, Vec<DiagKind>) {
    let mut r = vec![0u8; 308];
    let d = call("monstats2.composit", 1, text.map(str::as_bytes), &mut r).unwrap();
    (r[22], r[50..62].to_vec(), d)
}

#[test]
fn composits() {
    let ff = |n: usize| vec![0xFF; n];
    let cat = |a: &[u8], n| [a, &ff(n)].concat();
    assert_eq!(
        composit(Some("\"lit,med,hvy\"")),
        (3, cat(&[1, 2, 4], 9), vec![])
    );
    assert_eq!(
        composit(Some("\"lit,,med\"")),
        (1, cat(&[1], 11), vec![DiagKind::CbStop])
    );
    assert_eq!(
        composit(Some("\"lit,toolong,med\"")),
        (1, cat(&[1], 11), vec![DiagKind::CbStop])
    );
    assert_eq!(composit(Some("LIT")), (1, ff(12), vec![DiagKind::CbMiss]));
    assert_eq!(
        composit(Some("\"lit,med,\"")),
        (2, cat(&[1, 2], 10), vec![])
    );
    assert_eq!(composit(Some("")), (0, ff(12), vec![]));
    assert_eq!(composit(None), (0, ff(12), vec![]));
    let t = format!("\"{}\"", vec!["lit"; 13].join(","));
    assert_eq!(
        composit(Some(&t)),
        (12, vec![1; 12], vec![DiagKind::CbStop])
    );
}

#[test]
fn composit_total() {
    // `skeleton1`: total 49, written by the S8v call only.
    let mut r = vec![0u8; 308];
    let counts = [7, 3, 3, 3, 3, 10, 0, 5, 12, 12, 0, 0, 0, 0, 0];
    r[21..36].copy_from_slice(&counts);
    call("monstats2.composit", 15, Some(b""), &mut r).unwrap();
    assert_eq!(r[37], 49);
    r[21..36].fill(255);
    call("monstats2.composit", 15, Some(b"lit"), &mut r).unwrap();
    assert_eq!(r[37], 254);
    r[37] = 0;
    call("monstats2.composit", 15, None, &mut r).unwrap();
    assert_eq!(r[37], 0);
}

fn place(text: &str) -> Vec<u8> {
    let mut r = vec![9u8; 4];
    call("monpreset.place", 0, Some(text.as_bytes()), &mut r).unwrap();
    r[1..].to_vec()
}

#[test]
fn places() {
    assert_eq!(place("gheed"), [1, 0x93, 0]);
    assert_eq!(place("GHEED"), [1, 0x93, 0]);
    assert_eq!(place("The Countess"), [2, 6, 0]);
    assert_eq!(place("Griswold"), [2, 5, 0]);
    assert_eq!(place("nonexistent"), [0, 0, 0]);
    assert_eq!(place(""), [0, 0, 0]);
}

#[test]
fn helpers() {
    assert_eq!(unquote(b"\"lit,med\""), b"lit,med");
    assert_eq!(unquote(b"rin\",mag\""), b"rin");
    assert_eq!(split(b"a,", b","), (&b"a"[..], Some(&b""[..])));
    assert_eq!(split(b"a", b","), (&b"a"[..], None));
    assert_eq!(num(b" 7x"), 7);
    assert_eq!(num(b"+5"), 5);
    assert_eq!(num(b"-1"), -1);
    assert_eq!(num(b"-99999999999"), i32::MIN);
    assert_eq!(num(b"x"), 0);
}

/// Through the compiler: diagnostics carry line and column, and E15
/// rejects the file at the cell.
#[test]
fn compiled_with_diagnostics() {
    let (mut l, sp) = setup();
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    cb.special = sp;
    let f = [FieldDef::new(
        "input 1",
        FieldType::CustomLink,
        0,
        0,
        Link::Table("cubemain.input".into()),
    )];
    let txt = TxtTable::parse("t.txt", b"input 1\r\nany\r\nqqq\r\n").unwrap();
    let c = compile_table("t.txt", &txt, &f, 328, &mut l, &mut cb).unwrap();
    assert_eq!(c.diagnostics.len(), 1);
    assert_eq!(c.diagnostics[0].kind, DiagKind::CbMiss);
    assert_eq!(
        (c.diagnostics[0].line, c.diagnostics[0].column),
        (3, Some(0))
    );
    assert!(cb.unspecified.is_empty());
    let txt = TxtTable::parse("t.txt", b"input 1\r\nany\r\n\"rin,qty\"\r\n").unwrap();
    let e = compile_table("t.txt", &txt, &f, 328, &mut l, &mut cb).unwrap_err();
    assert_eq!(e.code, ErrorCode::E15);
}
