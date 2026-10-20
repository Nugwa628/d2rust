//! Extra tests for `compile` and `compile::callbacks`, each aimed at a mutant that
//! `cargo mutants` reported as missed (a code change the other tests did not notice).
//!
//! They cover linker counters, calc formula name links, field-list checks, text and
//! integer limits, and the table-specific callbacks. All use synthetic linkers and txt
//! text, so no game files are needed.

use crate::calc::CalcDiag;
use crate::compile::{
    check_field_list, code4, compile_table, name_key, special_linker, CallbackError, Callbacks,
    CodeLinker, DiagKind, FieldCall, Linker, Linkers, NameLinker, StdCallbacks,
};
use crate::schema::{CalcBuffer, FieldDef, FieldType as T, Link};
use crate::strings::StringTables;
use crate::txt::{ErrorCode, TxtTable};

fn code(s: &[u8]) -> u32 {
    u32::from_le_bytes(code4(s))
}

/// A code linker holding `codes` in order (index = position).
fn codes(codes: &[&str]) -> Linker {
    let mut l = CodeLinker::default();
    for c in codes {
        l.add(code(c.as_bytes()));
    }
    Linker::Code(l)
}

/// A name linker of `n` keys with `names` at their indices, filler elsewhere.
fn names(n: u32, names: &[(&str, u32)]) -> Linker {
    let mut l = NameLinker::default();
    for i in 0..n {
        let key = names
            .iter()
            .find(|(_, at)| *at == i)
            .map(|(k, _)| name_key(k.as_bytes()).unwrap())
            .unwrap_or_else(|| format!("\x01filler{i}").into_bytes());
        l.add_always(&key);
    }
    Linker::Name(l)
}

/// Runs one field callback through [`StdCallbacks`].
fn std_call(
    cb: &mut StdCallbacks<'_>,
    field: &FieldDef,
    text: Option<&str>,
    record: &mut [u8],
    linkers: &Linkers,
) -> (Result<(), CallbackError>, Vec<DiagKind>) {
    let mut diags = Vec::new();
    let r = cb.field(FieldCall {
        field,
        text: text.map(str::as_bytes),
        record,
        record_index: 0,
        slot: 0,
        linkers,
        diagnostics: &mut diags,
    });
    (r, diags)
}

// ------------------------------------------------------------ linkers

/// A linker's counter `n` starts at 0 and every add
/// takes the next index.
#[test]
fn linker_counters_start_empty() {
    let mut c = CodeLinker::default();
    assert!(c.is_empty());
    assert_eq!(c.len(), 0);
    c.add(code(b"abc"));
    assert!(!c.is_empty());
    assert_eq!(c.len(), 1);

    let mut n = NameLinker::default();
    assert!(n.is_empty());
    assert_eq!(n.len(), 0);
    n.find_or_add(b"x");
    assert!(!n.is_empty());
    assert_eq!(n.len(), 1);
    let mut a = NameLinker::default();
    a.add_always(b"y");
    assert!(!a.is_empty());
}

// ------------------------------------------------------- calc formulas

/// The first 42 skillcalc codes a calc formula can use.
const SKILLCALC: [&str; 42] = [
    "ln12", "dm12", "ln34", "dm34", "ln56", "dm56", "ln78", "dm78", "par1", "par2", "par3", "par4",
    "par5", "par6", "par7", "par8", "lvl", "edmn", "edmx", "edln", "toht", "mana", "mps", "math",
    "madm", "macr", "m1en", "m1ex", "m1el", "m2en", "m2ex", "m2el", "m3en", "m3ex", "m3el", "m1rn",
    "m2rn", "m3rn", "edns", "edxs", "ulvl", "blvl",
];

/// The misscalc codes a calc formula can use.
const MISSCALC: [&str; 43] = [
    "par1", "par2", "par3", "par4", "par5", "cpa1", "cpa2", "cpa3", "cpa4", "cpa5", "hpa1", "hpa2",
    "hpa3", "chp1", "chp2", "chp3", "dpa1", "dpa2", "lvl", "edmn", "edmx", "edln", "edns", "edxs",
    "damn", "damx", "dmns", "dmxs", "rang", "sl12", "sd12", "sl34", "sd34", "cl12", "cd12", "cl34",
    "cd34", "shl1", "shd1", "chl1", "chd1", "dl12", "dd12",
];

/// The links of the synthetic compiler vectors: `Fire Bolt` = skill 36, `firebolt` = missile 58,
/// `strength` = stat 0. The missile family's `skill(` uses the
/// compile-only `skills_lookup.skill`; here only that
/// link holds `Fire Bolt`.
fn calc_linkers(skills_in_lookup_only: bool) -> Linkers {
    let mut l = Linkers::default();
    let fire_bolt = names(40, &[("Fire Bolt", 36)]);
    if skills_in_lookup_only {
        l.insert("skills.skill", names(40, &[]));
    } else {
        l.insert("skills.skill", fire_bolt.clone());
    }
    l.insert("skills_lookup.skill", fire_bolt);
    l.insert("missiles.Missile", names(60, &[("firebolt", 58)]));
    l.insert("itemstatcost.stat", names(5, &[("strength", 0)]));
    l.insert("skillcalc.code", codes(&SKILLCALC));
    l.insert("misscalc.code", codes(&MISSCALC));
    l
}

/// Compiles `text` as a `calc(buffer)` cell; the bytes appended to the
/// buffer (the field holds 0, the start of an empty buffer).
fn calc_bytes(buffer: CalcBuffer, text: &str, linkers: &Linkers) -> Vec<u8> {
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let f = FieldDef::new("calc", T::CalcToDword, 0, 0, Link::Calc(buffer));
    let mut rec = [0xAAu8; 4];
    let (r, d) = std_call(&mut cb, &f, Some(text), &mut rec, linkers);
    r.unwrap();
    assert!(d.is_empty());
    assert_eq!(rec, [0; 4], "{text}: field = buffer offset 0");
    cb.buffers[&buffer].clone()
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

/// Calc formula name resolution through the compiler's links, with synthetic vectors.
#[test]
fn calc_formula_links() {
    let l = calc_linkers(false);
    for (text, bytes) in [
        ("skill('Fire Bolt'.lvl)", "07 24 07 10 01 03 00"),
        ("miss('firebolt'.edmn)", "07 3A 07 13 01 04 00"),
        ("stat('strength'.base)", "07 00 07 01 01 05 00"),
        ("stat('nosuchstat'.base)", "04 00 07 01 01 05 00"),
        ("lvl", "04 10 00"),
        ("ln12", "04 00 00"),
        ("par34", "04 0A 00"),
    ] {
        assert_eq!(
            calc_bytes(CalcBuffer::SkillsCode, text, &l),
            hex(bytes),
            "{text}"
        );
    }
    let l = calc_linkers(true);
    for (text, bytes) in [
        ("dl12", "04 29 00"),
        ("lvl", "04 12 00"),
        ("miss('firebolt'.dl12)", "07 3A 07 29 01 04 00"),
        ("skill('Fire Bolt'.sl12)", "07 24 00"),
    ] {
        assert_eq!(
            calc_bytes(CalcBuffer::MissCode, text, &l),
            hex(bytes),
            "{text}"
        );
    }
}

/// calc D2RUST policy 4: each formula cell reports its outcomes; two cells
/// with an unknown name report it twice.
#[test]
fn calc_diagnostics_counted_per_cell() {
    let l = calc_linkers(false);
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let f = FieldDef::new(
        "calc",
        T::CalcToDword,
        0,
        0,
        Link::Calc(CalcBuffer::SkillsCode),
    );
    let mut rec = [0u8; 4];
    std_call(&mut cb, &f, Some("foo"), &mut rec, &l).0.unwrap();
    assert_eq!(
        cb.calc_diagnostics
            .get(&(CalcBuffer::SkillsCode, CalcDiag::UnknownName)),
        Some(&1)
    );
    std_call(&mut cb, &f, Some("foo"), &mut rec, &l).0.unwrap();
    assert_eq!(
        cb.calc_diagnostics
            .get(&(CalcBuffer::SkillsCode, CalcDiag::UnknownName)),
        Some(&2)
    );
}

// --------------------------------------------------------- field lists

/// A callback's footprint (a calc or param field is a u32 at its offset) must lie
/// inside the record (E13).
#[test]
fn calc_and_param_footprints_checked() {
    for link in [Link::Calc(CalcBuffer::SkillsCode), Link::Param] {
        let at = |o| [FieldDef::new("c", T::CalcToDword, 0, o, link.clone())];
        assert!(check_field_list("t.txt", &at(4), 8).is_ok());
        let e = check_field_list("t.txt", &at(5), 8).unwrap_err();
        assert_eq!(e.code, ErrorCode::E13);
    }
}

/// Type-5 runs: a `u8?` with len > 1 at the end of
/// the list has no following entry: E13.
#[test]
fn type5_run_broken_at_list_end() {
    let f = [
        FieldDef::new("a", T::Dword, 0, 0, Link::None),
        FieldDef::new("b", T::Unknown1, 2, 4, Link::None),
    ];
    let e = check_field_list("t.txt", &f, 8).unwrap_err();
    assert_eq!(e.code, ErrorCode::E13);
}

// ---------------------------------------------------------- diagnostics

fn diag_kinds(fields: &[FieldDef], size: usize, text: &[u8]) -> Vec<DiagKind> {
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let mut l = Linkers::default();
    let txt = TxtTable::parse("t.txt", text).unwrap();
    compile_table("t.txt", &txt, fields, size, &mut l, &mut cb)
        .unwrap()
        .diagnostics
        .iter()
        .map(|d| d.kind)
        .collect()
}

/// TextCut: ASCII / BYTE2 cut at `len`, UNKNOWN6 at
/// min(max(len, 1) − 1, 31).
#[test]
fn text_cut_limits() {
    for t in [T::Ascii, T::Byte2] {
        let f = [FieldDef::new("s", t, 2, 0, Link::None)];
        assert_eq!(diag_kinds(&f, 8, b"s\r\nNU\r\n"), []);
        assert_eq!(diag_kinds(&f, 8, b"s\r\nNU0\r\n"), [DiagKind::TextCut]);
    }
    let f = [FieldDef::new(
        "k",
        T::Unknown6,
        4,
        0,
        Link::Linker("t.k".into()),
    )];
    assert_eq!(diag_kinds(&f, 8, b"k\r\nabc\r\n"), []);
    assert_eq!(diag_kinds(&f, 8, b"k\r\nabcd\r\n"), [DiagKind::TextCut]);
}

/// IntRange: WORD −32,768..=65,535; DWORD −2³¹..=2³²−1.
#[test]
fn int_range_bounds() {
    let word = [FieldDef::new("w", T::Word, 0, 0, Link::None)];
    for (cell, bad) in [
        ("-1", false),
        ("-32768", false),
        ("65535", false),
        ("-32769", true),
        ("65536", true),
    ] {
        let d = diag_kinds(&word, 4, format!("w\r\n{cell}\r\n").as_bytes());
        assert_eq!(d == [DiagKind::IntRange], bad, "word {cell}");
        assert!(bad || d.is_empty(), "word {cell}");
    }
    let dword = [FieldDef::new("d", T::Dword, 0, 0, Link::None)];
    for (cell, bad) in [
        ("5", false),
        ("-1", false),
        ("-2147483648", false),
        ("4294967295", false),
        ("-2147483649", true),
        ("4294967296", true),
    ] {
        let d = diag_kinds(&dword, 4, format!("d\r\n{cell}\r\n").as_bytes());
        assert_eq!(d == [DiagKind::IntRange], bad, "dword {cell}");
        assert!(bad || d.is_empty(), "dword {cell}");
    }
}

/// Reports a CbMiss for every missing-field call and fails on field `z`.
struct MissingProbe;

impl Callbacks for MissingProbe {
    fn key(&mut self, _f: &FieldDef, _t: &[u8]) -> u16 {
        0
    }
    fn field(&mut self, call: FieldCall<'_>) -> Result<(), CallbackError> {
        if call.text.is_none() {
            if call.field.column == b"z" {
                return Err(CallbackError {
                    code: ErrorCode::E15,
                    detail: String::new(),
                });
            }
            call.diagnostics.push(DiagKind::CbMiss);
        }
        Ok(())
    }
}

/// A missing field's callback runs with column
/// index k = header column count + its position among the missing
/// fields; its diagnostics and errors are reported at that column.
#[test]
fn missing_field_callback_columns() {
    let cb = |name: &str| FieldDef::new(name, T::CustomLink, 0, 0, Link::Table("x.y".into()));
    let fields = [
        FieldDef::new("a", T::Byte, 0, 0, Link::None),
        FieldDef::new("b", T::Byte, 0, 1, Link::None),
        cb("m0"),
        cb("m1"),
    ];
    let txt = TxtTable::parse("t.txt", b"a\tb\r\n1\t2\r\n").unwrap();
    let c = compile_table(
        "t.txt",
        &txt,
        &fields,
        4,
        &mut Linkers::default(),
        &mut MissingProbe,
    )
    .unwrap();
    let at: Vec<_> = c.diagnostics.iter().map(|d| (d.line, d.column)).collect();
    assert_eq!(at, [(2, Some(2)), (2, Some(3))]);

    let fields = [
        FieldDef::new("a", T::Byte, 0, 0, Link::None),
        FieldDef::new("b", T::Byte, 0, 1, Link::None),
        cb("m0"),
        cb("z"),
    ];
    let e = compile_table(
        "t.txt",
        &txt,
        &fields,
        4,
        &mut Linkers::default(),
        &mut MissingProbe,
    )
    .unwrap_err();
    assert_eq!(
        (e.code, e.line, e.column),
        (ErrorCode::E15, Some(2), Some(3))
    );
}

// -------------------------------------------------- table callbacks

/// `@uniques` / `@sets` from compiled records: the
/// `index` text at offset 2, add-always in record order; base code and
/// level read at the given offsets (uniques: code +40, lvl +52).
#[test]
fn special_linker_reads_records() {
    let rec = |name: &[u8], c: &[u8], lvl: u16| {
        let mut r = vec![0u8; 0x14C];
        r[2..2 + name.len()].copy_from_slice(name);
        r[40..44].copy_from_slice(&code4(c));
        r[52..54].copy_from_slice(&lvl.to_le_bytes());
        r
    };
    let recs = [
        rec(b"Foo", b"rin", 39),
        rec(b"Bar", b"amu", 300),
        rec(b"foo", b"hax", 1),
    ];
    let (l, items) = special_linker(recs.iter().map(Vec::as_slice), 40, 52).unwrap();
    assert_eq!(l.len(), 3);
    assert_eq!(l.find(b"foo"), Some(0));
    assert_eq!(l.find(b"bar"), Some(1));
    let got: Vec<_> = items.iter().map(|s| (s.code, s.lvl)).collect();
    assert_eq!(
        got,
        [(code(b"rin"), 39), (code(b"amu"), 300), (code(b"hax"), 1)]
    );
}

/// items `hax` 0, `rin` 1; item types `"    "` 0, `gem2` 1; compcode `nil`
/// 0, `lit` 1, `abcd` 2; the skill mode codes; monseq `""` 0, `seq_a` 1.
fn cb_linkers() -> Linkers {
    let mut l = Linkers::default();
    l.insert("items.code", codes(&["hax", "rin"]));
    l.insert("itemtypes.code", codes(&["", "gem2"]));
    l.insert("compcode.code", codes(&["nil", "lit", "abcd"]));
    l.insert(
        "monmode_lookup.code",
        codes(&[
            "DT", "NU", "WL", "GH", "A1", "A2", "BL", "SC", "S1", "S2", "S3", "S4", "DD", "KB",
            "xx", "RN",
        ]),
    );
    l.insert("monseq.sequence", names(2, &[("", 0), ("seq_a", 1)]));
    l
}

/// Runs `cb(name)` with slot `slot` on a fresh `size`-byte record.
fn table_cb(
    name: &str,
    slot: u32,
    text: Option<&str>,
    record: &mut [u8],
) -> (Result<(), CallbackError>, Vec<DiagKind>) {
    let l = cb_linkers();
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let f = FieldDef::new("x", T::CustomLink, 0, slot, Link::Table(name.into()));
    std_call(&mut cb, &f, text, record, &l)
}

fn input(slot: u32, text: &str) -> Vec<u8> {
    let mut r = vec![0u8; 328];
    let (res, _) = table_cb("cubemain.input", slot, Some(text), &mut r);
    res.unwrap();
    let o = 20 + 8 * slot as usize;
    let mut expect_rest = r.clone();
    expect_rest[o..o + 8].fill(0);
    assert!(expect_rest.iter().all(|&b| b == 0), "{text}: slot only");
    r[o..o + 8].to_vec()
}

/// Cube inputs: slot k is 8 bytes at 20 + 8k (k = 0–6); quality
/// words, flag words, flags OR-ed, `qty` read with num (no digits
/// → 0).
#[test]
fn cube_input_words() {
    assert_eq!(input(6, "rin"), [1, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(input(0, "rin"), [1, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(input(1, "\"rin,qty= \""), [1, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(input(1, "\"rin,qty=  \""), [1, 0, 1, 0, 0, 0, 0, 0]);
    for (w, q) in [
        ("low", 1),
        ("nor", 2),
        ("hiq", 3),
        ("mag", 4),
        ("set", 5),
        ("rar", 6),
        ("uni", 7),
        ("crf", 8),
        ("tmp", 9),
    ] {
        assert_eq!(
            input(1, &format!("\"rin,{w},qty=2\"")),
            [1, 0, 1, 0, 0, 0, q, 2],
            "{w}"
        );
    }
    for (w, f) in [
        ("nos", 0x0004u16),
        ("sock", 0x0008),
        ("eth", 0x0010),
        ("noe", 0x0020),
        ("upg", 0x0080),
        ("bas", 0x0100),
        ("exc", 0x0200),
        ("eli", 0x0400),
        ("nru", 0x0800),
    ] {
        let flags = (0x0001 | f).to_le_bytes();
        assert_eq!(
            input(1, &format!("\"rin,{w},qty=2\"")),
            [flags[0], flags[1], 1, 0, 0, 0, 0, 2],
            "{w}"
        );
    }
    // OR-ed: a repeated flag keeps its bit.
    assert_eq!(input(1, "\"rin,eth,eth\""), [0x11, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(input(1, "\"gem2,eth,eth\""), [0x12, 0, 1, 0, 0, 0, 0, 0]);
}

fn output(slot: u32, text: &str) -> Vec<u8> {
    let mut r = vec![0u8; 328];
    let (res, _) = table_cb("cubemain.output", slot, Some(text), &mut r);
    res.unwrap();
    let o = 76 + 84 * slot as usize;
    let mut expect_rest = r.clone();
    expect_rest[o..o + 24].fill(0);
    assert!(expect_rest.iter().all(|&b| b == 0), "{text}: slot only");
    r[o..o + 12].to_vec()
}

/// Cube outputs: `qty` sets the quantity (+7); quality and flag
/// words; flags OR-ed.
#[test]
fn cube_output_words() {
    assert_eq!(
        output(0, "\"rin,qty=3\""),
        [0, 0, 1, 0, 0, 0, 0, 3, 0xFC, 0, 0, 0]
    );
    assert_eq!(
        output(1, "\"rin,qty=3\""),
        [0, 0, 1, 0, 0, 0, 0, 3, 0xFC, 0, 0, 0]
    );
    for (w, q) in [
        ("low", 1),
        ("nor", 2),
        ("hiq", 3),
        ("mag", 4),
        ("set", 5),
        ("rar", 6),
        ("uni", 7),
        ("crf", 8),
        ("tmp", 9),
    ] {
        assert_eq!(
            output(0, &format!("\"useitem,{w},qty=2\"")),
            [0, 0, 0, 0, 0, 0, q, 2, 0xFE, 0, 0, 0],
            "{w}"
        );
    }
    for (w, f) in [
        ("mod", 0x0001u16),
        ("eth", 0x0004),
        ("uns", 0x0010),
        ("rem", 0x0020),
        ("exc", 0x0080),
        ("eli", 0x0100),
        ("rep", 0x0200),
        ("rch", 0x0400),
    ] {
        let flags = f.to_le_bytes();
        assert_eq!(
            output(0, &format!("\"useitem,{w},{w},qty=2\"")),
            [flags[0], flags[1], 0, 0, 0, 0, 0, 2, 0xFE, 0, 0, 0],
            "{w}"
        );
    }
}

fn skillmode(text: Option<&str>) -> (u8, u16, Vec<DiagKind>) {
    let mut r = vec![0xAAu8; 424];
    r[368 + 2..370 + 2].copy_from_slice(&5i16.to_le_bytes());
    let (res, d) = table_cb("monstats.skillmode", 1, text, &mut r);
    res.unwrap();
    (r[385], u16::from_le_bytes([r[394], r[395]]), d)
}

/// Skill modes: mode 14 and the sequence lookup; a miss
/// gives 0xFFFF and CbMiss (non-empty text only).
#[test]
fn skillmode_sequence_lookup() {
    assert_eq!(skillmode(Some("seq_a")), (14, 1, vec![]));
    assert_eq!(skillmode(Some("SEQ_A")), (14, 1, vec![]));
    assert_eq!(
        skillmode(Some("nosuchseq")),
        (14, 0xFFFF, vec![DiagKind::CbMiss])
    );
    assert_eq!(skillmode(Some("a1")), (14, 0xFFFF, vec![DiagKind::CbMiss]));
    assert_eq!(skillmode(Some("")), (14, 0, vec![]));
    assert_eq!(skillmode(Some("A1")), (4, 0xFFFF, vec![]));
}

fn composit(slot: usize, text: Option<&str>) -> Vec<u8> {
    let mut r = vec![0xAAu8; 308];
    let (res, _) = table_cb("monstats2.composit", slot as u32, text, &mut r);
    res.unwrap();
    r
}

/// Component choices: count = 0 and the 12 choices = 0xFF, even
/// with no text; nothing else is written.
#[test]
fn composit_reset() {
    for slot in [0, 1, 7] {
        let r = composit(slot, None);
        let mut expect = vec![0xAAu8; 308];
        expect[21 + slot] = 0;
        expect[38 + 12 * slot..50 + 12 * slot].fill(0xFF);
        assert_eq!(r, expect, "slot {slot}");
    }
}

/// Component choices: a token of exactly 4 bytes is read; only
/// longer ones stop.
#[test]
fn composit_four_byte_token() {
    let r = composit(1, Some("\"lit,abcd\""));
    assert_eq!(r[22], 2);
    assert_eq!(r[50..53], [1, 2, 0xFF]);
}
