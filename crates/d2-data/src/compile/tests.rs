//! Tests for the txt → record compiler in `compile`: cell conversions by field type,
//! linkers, diagnostics, field-list checks, callbacks and pass ordering.
//!
//! They use small synthetic field lists and txt text, so no game files are needed.

use super::*;
use crate::schema::FieldType as T;

fn field(name: &str, t: T, len: u32, offset: u32, link: &str) -> FieldDef {
    let link = match link {
        "" => Link::None,
        l if l.starts_with("cb:") => Link::Table(l[3..].to_owned()),
        l => Link::Linker(l.to_owned()),
    };
    FieldDef::new(name, t, len, offset, link)
}

/// A recorded field-callback call: text, offset, len, record, slot.
type Call = (Option<Vec<u8>>, u32, u32, u32, u32);

/// Callbacks that record their calls; the key callback returns 0x100 +
/// the text length.
#[derive(Default)]
struct Mock {
    keys: Vec<Vec<u8>>,
    calls: Vec<Call>,
}

impl Callbacks for Mock {
    fn key(&mut self, _f: &FieldDef, text: &[u8]) -> u16 {
        self.keys.push(text.to_vec());
        0x100 + text.len() as u16
    }
    fn field(&mut self, call: FieldCall<'_>) -> Result<(), CallbackError> {
        self.calls.push((
            call.text.map(<[u8]>::to_vec),
            call.field.offset,
            call.field.length,
            call.record_index,
            call.slot,
        ));
        Ok(())
    }
}

fn compile_with(
    fields: &[FieldDef],
    size: usize,
    text: &[u8],
    linkers: &mut Linkers,
    cb: &mut dyn Callbacks,
) -> Result<Compiled, TxtError> {
    let txt = TxtTable::parse("t.txt", text).expect("test text parses");
    compile_table("t.txt", &txt, fields, size, linkers, cb)
}

fn compile(fields: &[FieldDef], size: usize, text: &[u8], linkers: &mut Linkers) -> Compiled {
    compile_with(fields, size, text, linkers, &mut Mock::default()).expect("compiles")
}

fn code(s: &[u8]) -> u32 {
    u32::from_le_bytes(code4(s))
}

fn diag_kinds(c: &Compiled) -> Vec<DiagKind> {
    c.diagnostics.iter().map(|d| d.kind).collect()
}

#[test]
fn integer_vectors() {
    // (cell, u32) pairs.
    let cases: [(&[u8], u32); 35] = [
        (b"", 0),
        (b"5", 5),
        (b"12", 12),
        (b"-1", 0xFFFF_FFFF),
        (b"-12", 0xFFFF_FFF4),
        (b"300", 0x12C),
        (b"256", 0x100),
        (b"999", 0x3E7),
        (b" ", 0xFFFF_FFF0),
        (b" 5", 0xFFFF_FF65),
        (b" 1", 0xFFFF_FF61),
        (b"1 ", 0xFFFF_FFFA),
        (b"5 ", 0x22),
        (b"+5", 0xFFFF_FFD3),
        (b"12abc", 0x442B),
        (b"abc", 0x154B),
        (b"0x10", 0x1C2A),
        (b"1.5", 0x55),
        (b"1e3", 0x279),
        (b"-", 0),
        (b"-0", 0),
        (b"--5", 0x19),
        (b"--1", 0x1D),
        (b"5-", 0x2F),
        (b"007", 7),
        (b"*16", 0xFFFF_FDB8),
        (b"*12", 0xFFFF_FDB4),
        (b"65535", 0xFFFF),
        (b"65536", 0x10000),
        (b"-32768", 0xFFFF_8000),
        (b"2147483648", 0x8000_0000),
        (b"4294967295", 0xFFFF_FFFF),
        (b"4294967296", 0),
        (b"99999999999", 0x4876_E7FF),
        (b"\xE9", 0xFFFF_FFB9),
    ];
    for (cell, v) in cases {
        assert_eq!(parse_int(cell), v, "{cell:?}");
    }
    assert_eq!(parse_int(b"\x85"), 0xFFFF_FF55);
}

#[test]
fn bit_and_optional_byte() {
    let f = [field("b", T::Bit, 10, 16, "")];
    let mut l = Linkers::default();
    for (cell, set) in [
        ("1", true),
        ("2", true),
        ("-1", true),
        (" ", true),
        ("0", false),
        ("", false),
        ("-", false),
    ] {
        let text = format!("b\r\n{cell}\r\n");
        let c = compile(&f, 20, text.as_bytes(), &mut l);
        assert_eq!(c.records[17], if set { 0x04 } else { 0 }, "{cell:?}");
    }
    // u8?: empty writes nothing; "-" writes 0.
    let f = [
        field("b", T::Byte, 0, 0, ""),
        field("u", T::Unknown1, 1, 0, ""),
    ];
    let c = compile(&f, 1, b"b\tu\r\n7\t\r\n7\t-\r\n", &mut l);
    assert_eq!(c.records, [0x07, 0x00]);
}

#[test]
fn text_and_code_vectors() {
    let mut l = Linkers::default();
    let one = |t: T, len: u32, cell: &[u8], size: usize, l: &mut Linkers| {
        let mut text = b"a\r\n".to_vec();
        text.extend_from_slice(cell);
        text.extend_from_slice(b"\r\n");
        compile(&[field("a", t, len, 0, "")], size, &text, l).records
    };
    assert_eq!(one(T::Ascii, 4, b"abc", 5, &mut l), b"abc\0\0");
    assert_eq!(one(T::Ascii, 4, b"abcdef", 5, &mut l), b"abcd\0");
    assert_eq!(one(T::Ascii, 4, b"", 5, &mut l), [0; 5]);
    assert_eq!(one(T::Ascii, 2, b"NU0", 3, &mut l), b"NU\0");
    assert_eq!(one(T::Raw, 0, b"", 4, &mut l), b"    ");
    assert_eq!(one(T::Raw, 0, b"DT", 4, &mut l), b"DT  ");
    assert_eq!(one(T::Raw, 0, b"ab", 4, &mut l), b"ab  ");
    assert_eq!(one(T::Raw, 0, b"staff", 4, &mut l), b"staf");
    assert_eq!(one(T::Raw, 0, b"ring ", 4, &mut l), b"ring");
    assert_eq!(one(T::Raw, 0, b" a", 4, &mut l), b" a  ");
    // Missing RAW column: zeros, not spaces.
    let c = compile(
        &[field("a", T::Byte, 0, 4, ""), field("r", T::Raw, 0, 0, "")],
        5,
        b"a\r\n1\r\n",
        &mut l,
    );
    assert_eq!(c.records, [0, 0, 0, 0, 1]);
}

#[test]
fn own_key_text_types() {
    let mut l = Linkers::default();
    // UNKNOWN6 (key(str)), len 4: stores "abc" + NUL, registers "abc".
    let f = [field("k", T::Unknown6, 4, 0, "N")];
    let c = compile(&f, 4, b"k\r\nabcdef\r\n", &mut l);
    assert_eq!(c.records, b"abc\0");
    assert_eq!(l.name("N").unwrap().find(b"abc"), Some(0));
    assert_eq!(diag_kinds(&c), [DiagKind::TextCut]);
    // len 0: stores NUL, registers "".
    let f = [field("k", T::Unknown6, 0, 0, "N0")];
    let c = compile(&f, 1, b"k\r\nabc\r\n", &mut l);
    assert_eq!(c.records, [0]);
    assert_eq!(l.name("N0").unwrap().find(b""), Some(0));
    // UNKNOWN4 / UNKNOWN5 store code bytes, not the index.
    let c = compile(
        &[field("k", T::Unknown4, 0, 0, "C4")],
        1,
        b"k\r\nabc\r\n",
        &mut l,
    );
    assert_eq!(c.records, b"a");
    assert_eq!(l.code("C4").unwrap().find(code(b"abc")), Some(0));
    let c = compile(
        &[field("k", T::Unknown5, 0, 0, "C5")],
        2,
        b"k\r\nabc\r\n",
        &mut l,
    );
    assert_eq!(c.records, b"ab");
}

#[test]
fn code_linker_vectors() {
    let mut l = CodeLinker::default();
    assert_eq!(l.add(code(b"abc")), (0, false));
    assert_eq!(l.add(code(b"xyz")), (1, false));
    assert_eq!(l.add(code(b"abc")), (2, true));
    assert_eq!(l.find(0x2063_6262), Some(2)); // "bbc "
    for (s, i) in [
        (&b"abc"[..], Some(0)),
        (b"xyz", Some(1)),
        (b"bbc", Some(2)),
        (b"abd", None),
        (b"", None),
        (b"ABC", None),
    ] {
        assert_eq!(l.find(code(s)), i, "{s:?}");
    }

    let mut l = CodeLinker::default();
    l.add(code(b"abc"));
    l.add(code(b"abc"));
    assert_eq!(l.add(code(b"bbc")), (2, true));
    assert_eq!(
        (l.find(code(b"bbc")), l.find(code(b"cbc"))),
        (Some(1), Some(2))
    );

    let mut l = CodeLinker::default();
    for i in 0..5 {
        assert_eq!(l.add(code(b"")).0, i);
        assert_eq!(l.find(0x2020_2020 + i), Some(i));
    }
    assert_eq!(l.find(code(b"")), Some(0));
}

#[test]
fn name_linker_vectors() {
    let mut linkers = Linkers::default();
    let f = [field("n", T::NameToIndex, 0, 0, "N")];
    let c = compile(
        &f,
        2,
        b"n\r\nFire Bolt\r\nfire bolt\r\nIce\r\n\r\n",
        &mut linkers,
    );
    assert_eq!(c.records, [0, 0, 0, 0, 1, 0, 2, 0]);
    let n = linkers.name("N").unwrap();
    assert_eq!(n.len(), 3);
    assert_eq!(n.find(&name_key(b"FIRE BOLT").unwrap()), Some(0));
    assert_eq!(n.find(b""), Some(2));
    assert_eq!(n.find(b"fire"), None);

    let f = [field("n", T::Unknown6, 8, 0, "A")];
    let c = compile(&f, 8, b"n\r\na\r\nA\r\nb\r\n", &mut linkers);
    let a = linkers.name("A").unwrap();
    assert_eq!((a.find(b"a"), a.find(b"b"), a.len()), (Some(0), Some(2), 3));
    assert_eq!(diag_kinds(&c), [DiagKind::DupName]);

    let mut n = NameLinker::default();
    let long_a = name_key(&[b'q'; 40]).unwrap();
    let mut other = vec![b'q'; 31];
    other.extend_from_slice(b"zzzzzzzzz");
    assert_eq!(
        n.find_or_add(&long_a),
        n.find_or_add(&name_key(&other).unwrap())
    );

    // Non-ASCII in the first 31 bytes: E11; past them: accepted.
    let e = compile_with(
        &[field("n", T::NameToIndex, 0, 0, "E")],
        2,
        b"n\r\nCaf\xE9\r\n",
        &mut linkers,
        &mut Mock::default(),
    )
    .unwrap_err();
    assert_eq!(
        (e.code, e.line, e.column),
        (ErrorCode::E11, Some(2), Some(0))
    );
    let mut text = b"n\r\n".to_vec();
    text.extend_from_slice(&[b'a'; 35]);
    text.push(0xE9);
    text.extend_from_slice(b"aaaa\r\n");
    let c = compile(
        &[field("n", T::NameToIndex, 0, 0, "F")],
        2,
        &text,
        &mut linkers,
    );
    assert_eq!(diag_kinds(&c), [DiagKind::TextCut]);
}

#[test]
fn whole_record_vectors() {
    // k ASCIITOCODE@0 → L, n CODETOBYTE@4 → L, m CODETOWORD@6 → L2 (no
    // column), r RAW@8 (no column); 12.
    let mut l = Linkers::default();
    l.ensure("L2", LinkerKind::Code).unwrap();
    let f = [
        field("k", T::AsciiToCode, 0, 0, "L"),
        field("n", T::CodeToByte, 0, 4, "L"),
        field("m", T::CodeToWord, 0, 6, "L2"),
        field("r", T::Raw, 0, 8, ""),
    ];
    let c = compile(&f, 12, b"k\tn\r\na\tb\r\nb\ta\r\n", &mut l);
    assert_eq!(
        c.record(0),
        [0x61, 0x20, 0x20, 0x20, 0x01, 0, 0xFF, 0xFF, 0, 0, 0, 0]
    );
    assert_eq!(
        c.record(1),
        [0x62, 0x20, 0x20, 0x20, 0x00, 0, 0xFF, 0xFF, 0, 0, 0, 0]
    );
    assert!(l.code("L2").unwrap().is_empty());

    // Duplicate code: the record keeps the cell's code.
    let mut l = Linkers::default();
    let c = compile(
        &[field("k", T::AsciiToCode, 0, 0, "L")],
        4,
        b"k\r\nabc\r\nabc\r\n",
        &mut l,
    );
    assert_eq!(c.records, b"abc abc ");
    assert_eq!(l.code("L").unwrap().find(0x2063_6262), Some(1));
    assert_eq!(c.diagnostics[0].kind, DiagKind::DupCode);
    assert_eq!(c.diagnostics[0].line, 3);

    // Missing key column: no key.
    let mut l = Linkers::default();
    let f = [
        field("a", T::Byte, 0, 4, ""),
        field("k", T::AsciiToCode, 0, 0, "L"),
    ];
    let c = compile(&f, 5, b"a\r\n1\r\n", &mut l);
    assert_eq!(c.records, [0, 0, 0, 0, 1]);
    assert!(l.code("L").unwrap().is_empty());

    // Missing value is written last.
    let mut l = Linkers::default();
    l.ensure("N", LinkerKind::Name).unwrap();
    let f = [
        field("a", T::Word, 0, 0, ""),
        field("z", T::NameToWord, 0, 0, "N"),
    ];
    assert_eq!(compile(&f, 2, b"a\r\n5\r\n", &mut l).records, [0xFF, 0xFF]);

    // NAMETOWORD2 stores one byte.
    let c = compile(
        &[field("s", T::NameToWord2, 0, 0, "N")],
        2,
        b"s\r\nx\r\n",
        &mut l,
    );
    assert_eq!(c.records, [0xFF, 0x00]);
    assert_eq!(diag_kinds(&c), [DiagKind::LinkMiss]);

    // NAMETODWORD with N = {x → 0}.
    let mut l = Linkers::default();
    let Linker::Name(n) = l.ensure("N", LinkerKind::Name).unwrap() else {
        unreachable!()
    };
    n.find_or_add(b"x");
    let f = [
        field("d", T::NameToDword, 0, 0, "N"),
        field("e", T::NameToDword, 0, 4, "N"),
    ];
    let c = compile(&f, 8, b"d\r\ny\r\nX\r\n", &mut l);
    assert_eq!(c.record(0), [0xFF; 8]);
    assert_eq!(c.record(1), [0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);

    // A shared code linker across tables.
    let mut l = Linkers::default();
    compile(
        &[field("c", T::AsciiToCode, 0, 0, "I")],
        4,
        b"c\r\nx\r\ny\r\n",
        &mut l,
    );
    compile(
        &[field("c", T::AsciiToCode, 0, 0, "I")],
        4,
        b"c\r\nz\r\n",
        &mut l,
    );
    let c = compile(
        &[field("i", T::Unknown3, 0, 0, "I")],
        4,
        b"i\r\nz\r\nx\r\n",
        &mut l,
    );
    assert_eq!(c.records, [2, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn callback_vectors() {
    let mut l = Linkers::default();
    let f = [
        field("a", T::Dword, 0, 0, ""),
        field("f", T::CustomLink, 7, 4, "cb:M"),
        field("g", T::CustomLink, 9, 8, "cb:M"),
    ];
    let mut m = Mock::default();
    let c = compile_with(&f, 12, b"f\ta\r\nxy\t5\r\n\t6\r\n", &mut l, &mut m).unwrap();
    assert_eq!(
        m.calls,
        [
            (Some(b"xy".to_vec()), 4, 7, 0, 0),
            (None, 8, 9, 0, 2),
            (Some(Vec::new()), 4, 7, 1, 0),
            (None, 8, 9, 1, 2),
        ]
    );
    assert_eq!(c.record(0), [5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(c.record(1), [6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let f = [
        field("k", T::KeyToWord, 0, 0, ""),
        field("m", T::KeyToWord, 0, 2, ""),
    ];
    let mut m = Mock::default();
    let c = compile_with(&f, 4, b"k\r\nabc\r\n", &mut l, &mut m).unwrap();
    assert_eq!(m.keys, [b"abc".to_vec()]);
    assert_eq!(c.records, [3, 1, 0, 0]);

    let mut text = b"k\r\n".to_vec();
    text.extend_from_slice(&[b'x'; 300]);
    text.extend_from_slice(b"\r\n");
    let mut m = Mock::default();
    let c = compile_with(&f[..1], 2, &text, &mut l, &mut m).unwrap();
    assert_eq!(m.keys[0].len(), 256);
    assert_eq!(diag_kinds(&c), [DiagKind::TextCut]);
}

#[test]
fn mod_rules_clamp_out_of_range_ints() {
    let mut l = Linkers::default();
    let mut run = |ty: T, size: usize, cell: &str, rules: DataRules| {
        let text = format!("a\r\n{cell}\r\n");
        let txt = TxtTable::parse("t.txt", text.as_bytes()).expect("test text parses");
        let fields = [field("a", ty, 0, 0, "")];
        let mut cb = Mock::default();
        let c = compile_table_with("t.txt", &txt, &fields, size, &mut l, &mut cb, rules)
            .expect("compiles");
        (c.records, diag_kinds_of(&c.diagnostics))
    };
    let m = DataRules::Mod;
    let range = vec![DiagKind::IntRange];
    // Clamped into the field's range, still reported.
    assert_eq!(run(T::Byte, 1, "999", m), (vec![0xFF], range.clone()));
    assert_eq!(run(T::Byte, 1, "-129", m), (vec![0x80], range.clone()));
    assert_eq!(
        run(T::Word, 2, "65536", m),
        (vec![0xFF, 0xFF], range.clone())
    );
    assert_eq!(
        run(T::Word, 2, "-40000", m),
        (vec![0x00, 0x80], range.clone())
    );
    assert_eq!(
        run(T::Dword, 4, "4294967296", m),
        (vec![0xFF; 4], range.clone())
    );
    // In range and syntax errors compile as under Original.
    assert_eq!(run(T::Byte, 1, "-128", m), (vec![0x80], vec![]));
    assert_eq!(
        run(T::Byte, 1, " ", m),
        (vec![0xF0], vec![DiagKind::IntSyntax])
    );
    // Original wraps.
    assert_eq!(
        run(T::Byte, 1, "999", DataRules::Original),
        (vec![0xE7], range)
    );
}

fn diag_kinds_of(d: &[Diagnostic]) -> Vec<DiagKind> {
    d.iter().map(|d| d.kind).collect()
}

#[test]
fn diagnostics_vectors() {
    let mut l = Linkers::default();
    let byte = |cell: &str, l: &mut Linkers| {
        let c = compile(
            &[field("a", T::Byte, 0, 0, "")],
            1,
            format!("a\r\n{cell}\r\n").as_bytes(),
            l,
        );
        (c.records[0], diag_kinds(&c))
    };
    assert_eq!(byte("999", &mut l), (0xE7, vec![DiagKind::IntRange]));
    assert_eq!(byte(" ", &mut l), (0xF0, vec![DiagKind::IntSyntax]));
    assert_eq!(byte("-", &mut l), (0x00, vec![DiagKind::IntSyntax]));
    assert_eq!(byte("", &mut l), (0x00, vec![]));
    assert_eq!(byte("-128", &mut l), (0x80, vec![]));
    assert_eq!(byte("255", &mut l), (0xFF, vec![]));
    assert_eq!(byte("-129", &mut l), (0x7F, vec![DiagKind::IntRange]));
    let word = compile(
        &[field("a", T::Word, 0, 0, "")],
        2,
        b"a\r\n65535\r\n65536\r\n",
        &mut l,
    );
    assert_eq!(word.records, [0xFF, 0xFF, 0, 0]);
    assert_eq!(diag_kinds(&word), [DiagKind::IntRange]);
    let dword = compile(
        &[field("a", T::Dword, 0, 0, "")],
        4,
        b"a\r\n-1\r\n4294967296\r\n",
        &mut l,
    );
    assert_eq!(dword.records, [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]);
    assert_eq!(diag_kinds(&dword), [DiagKind::IntRange]);
    let bit = compile(&[field("a", T::Bit, 0, 0, "")], 1, b"a\r\n2\r\n", &mut l);
    assert_eq!(
        (bit.records[0], diag_kinds(&bit)),
        (1, vec![DiagKind::IntRange])
    );
    // CODETOWORD `staff` looks up `staf`: TextCut.
    let c = compile(
        &[field("k", T::AsciiToCode, 0, 0, "S")],
        4,
        b"k\r\nstaf\r\n",
        &mut l,
    );
    assert!(c.diagnostics.is_empty());
    let c = compile(
        &[field("a", T::CodeToWord, 0, 0, "S")],
        2,
        b"a\r\nstaff\r\n",
        &mut l,
    );
    assert_eq!(
        (c.records.clone(), diag_kinds(&c)),
        (vec![0, 0], vec![DiagKind::TextCut])
    );
    // Empty lookup that misses: no LinkMiss.
    l.ensure("E", LinkerKind::Name).unwrap();
    let c = compile(
        &[field("a", T::NameToWord, 0, 0, "E")],
        2,
        b"a\r\nzzz\r\n\r\n",
        &mut l,
    );
    assert_eq!(c.records, [0xFF; 4]);
    assert_eq!(diag_kinds(&c), [DiagKind::LinkMiss]);
}

#[test]
fn binding_diagnostics() {
    let mut l = Linkers::default();
    let f = [
        field("name", T::Byte, 0, 0, ""),
        field("level", T::Byte, 0, 1, ""),
    ];
    let c = compile(&f, 2, b"Name\tLEVEL\tname\r\n1\t2\t3\r\n", &mut l);
    assert_eq!(c.records, [1, 2]);
    assert_eq!(c.diagnostics[0].kind, DiagKind::DupColumn);
    assert_eq!(
        (c.diagnostics[0].line, c.diagnostics[0].column),
        (1, Some(2))
    );
}

fn e13(fields: &[FieldDef], size: usize, text: &[u8]) -> ErrorCode {
    let mut l = Linkers::default();
    l.ensure("N", LinkerKind::Name).unwrap();
    compile_with(fields, size, text, &mut l, &mut Mock::default())
        .map(|_| ErrorCode::E1)
        .unwrap_or_else(|e| e.code)
}

#[test]
fn field_list_checks() {
    const OK: ErrorCode = ErrorCode::E1; // stands for "compiled"
    let t = b"q\r\n1\r\n";
    assert_eq!(
        e13(
            &[field("x", T::Byte, 0, 0, ""), field("X", T::Byte, 0, 1, "")],
            2,
            t
        ),
        ErrorCode::E13
    );
    assert_eq!(e13(&[field("", T::Byte, 0, 0, "")], 1, t), ErrorCode::E13);
    let many = |n: usize| {
        (0..n)
            .map(|i| field(&format!("f{i}"), T::Byte, 0, 0, ""))
            .collect::<Vec<_>>()
    };
    assert_eq!(e13(&many(281), 1, t), ErrorCode::E13);
    assert_eq!(e13(&many(279), 1, t), OK);
    assert_eq!(e13(&[field("a", T::Byte, 0, 0, "")], 0, t), ErrorCode::E13);
    assert_eq!(
        e13(
            &[
                field("a", T::Unknown1, 2, 0, ""),
                field("b", T::Byte, 0, 1, "")
            ],
            2,
            t
        ),
        ErrorCode::E13
    );
    assert_eq!(
        e13(
            &[
                field("a", T::Unknown1, 2, 0, ""),
                field("b", T::Unknown1, 2, 1, "")
            ],
            2,
            t
        ),
        OK
    );
    assert_eq!(
        e13(&[field("a", T::Word, 0, 11, "")], 12, t),
        ErrorCode::E13
    );
    assert_eq!(e13(&[field("a", T::Word, 0, 10, "")], 12, t), OK);
    assert_eq!(
        e13(&[field("a", T::Ascii, 4, 8, "")], 12, t),
        ErrorCode::E13
    );
    assert_eq!(e13(&[field("a", T::Ascii, 3, 8, "")], 12, t), OK);
    assert_eq!(e13(&[field("a", T::Bit, 95, 0, "")], 12, t), OK);
    assert_eq!(e13(&[field("a", T::Bit, 96, 0, "")], 12, t), ErrorCode::E13);
    assert_eq!(
        e13(&[field("a", T::CodeToWord, 0, 0, "N")], 2, t),
        ErrorCode::E13
    );
    assert_eq!(
        e13(&[field("a", T::CodeToWord, 0, 0, "absent")], 2, t),
        ErrorCode::E13
    );
}

#[test]
fn slot_limit() {
    let f = [field("zz", T::Byte, 0, 0, "")];
    let text = |cols: usize| {
        let mut t = vec![b'c'];
        for _ in 1..cols {
            t.extend_from_slice(b"\tc");
        }
        t.extend_from_slice(b"\r\n");
        t.extend(std::iter::repeat_n(b'\t', cols - 1));
        t.extend_from_slice(b"\r\n");
        t
    };
    assert_eq!(e13(&f, 1, &text(280)), ErrorCode::E14);
    assert_eq!(e13(&f, 1, &text(279)), ErrorCode::E1);
}

#[test]
fn range_linker_codes() {
    let Linker::Code(r) = range_linker() else {
        unreachable!()
    };
    for (i, c) in [&b"none"[..], b"h2h", b"rng", b"both", b"loc"]
        .iter()
        .enumerate()
    {
        assert_eq!(r.find(code(c)), Some(i as u32));
    }
}

#[test]
fn param_callback() {
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let mut l = Linkers::default();
    let Linker::Name(s) = l.ensure("skills.skill", LinkerKind::Name).unwrap() else {
        unreachable!()
    };
    s.find_or_add(b"attack");
    s.find_or_add(b"battle command");
    let f = [FieldDef::new("p", T::CalcToDword, 0, 0, Link::Param)];
    let c = compile_with(
        &f,
        4,
        b"p\r\nBattle Command\r\n41\r\n12abc\r\n-5\r\nnothing\r\n\r\n",
        &mut l,
        &mut cb,
    )
    .unwrap();
    let values: Vec<u32> = c
        .records
        .chunks(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(values, [1, 41, 12, (-5i32) as u32, 0, 0]);
}

#[test]
fn calc_callback_appends() {
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let mut l = Linkers::default();
    let f = [
        FieldDef::new(
            "calc1",
            T::CalcToDword,
            0,
            0,
            Link::Calc(CalcBuffer::ItemsCode),
        ),
        FieldDef::new(
            "len",
            T::CalcToDword,
            0,
            4,
            Link::Calc(CalcBuffer::ItemsCode),
        ),
    ];
    let c = compile_with(&f, 8, b"calc1\r\n5\r\n\r\n750\r\n", &mut l, &mut cb).unwrap();
    let values: Vec<u32> = c
        .records
        .chunks(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    // `len` is missing: 0xFFFFFFFF.
    assert_eq!(values, [0, u32::MAX, u32::MAX, u32::MAX, 3, u32::MAX]);
    assert_eq!(
        cb.buffers[&CalcBuffer::ItemsCode],
        [0x07, 0x05, 0x00, 0x08, 0xEE, 0x02, 0x00]
    );
}

// ------------------------------------------------- coverage-gap tests

const COMPILED: ErrorCode = ErrorCode::E1; // stands for "compiled"

fn outcome(fields: &[FieldDef], size: usize, text: &[u8], linkers: &mut Linkers) -> ErrorCode {
    compile_with(fields, size, text, linkers, &mut Mock::default())
        .map(|_| COMPILED)
        .unwrap_or_else(|e| e.code)
}

/// `n` Byte fields `f0`, `f1`, … at offset 0.
fn byte_fields(n: usize) -> Vec<FieldDef> {
    (0..n)
        .map(|i| field(&format!("f{i}"), T::Byte, 0, 0, ""))
        .collect()
}

/// A one-column table: header `column`, one record per cell.
fn column_text(column: &str, cells: &[&[u8]]) -> Vec<u8> {
    let mut t = column.as_bytes().to_vec();
    t.extend_from_slice(b"\r\n");
    for c in cells {
        t.extend_from_slice(c);
        t.extend_from_slice(b"\r\n");
    }
    t
}

#[test]
fn field_list_rules() {
    let mut l = Linkers::default();
    // The list is a slice: there is no type-0 entry to end it.
    assert_eq!(FieldType::from_id(0), None);
    // At most 280 fields (1 column + 279 missing = 280 slots).
    assert_eq!(
        outcome(&byte_fields(280), 1, b"f0\r\n1\r\n", &mut l),
        COMPILED
    );
    assert_eq!(
        outcome(&byte_fields(281), 1, b"f0\r\n1\r\n", &mut l),
        ErrorCode::E13
    );
    // Duplicate names (case-insensitive) are rejected.
    let dup = [
        field("ab", T::Byte, 0, 0, ""),
        field("AB", T::Byte, 0, 1, ""),
    ];
    assert_eq!(outcome(&dup, 2, b"ab\r\n1\r\n", &mut l), ErrorCode::E13);
    // A field bound to no column is missing.
    let txt = TxtTable::parse("t.txt", b"f1\r\n1\r\n").unwrap();
    let b = bind(&txt.header, &["f0", "f1"]);
    assert_eq!(b.missing_fields().collect::<Vec<_>>(), [0]);
    // Header columns + missing fields ≤ 280: 2 columns + 279 missing is E14.
    assert_eq!(
        outcome(&byte_fields(280), 1, b"f0\tzz\r\n1\t2\r\n", &mut l),
        ErrorCode::E14
    );
    // No alignment required: a WORD at offset 1, a DWORD at offset 3.
    let f = [
        field("w", T::Word, 0, 1, ""),
        field("d", T::Dword, 0, 3, ""),
    ];
    let c = compile(&f, 7, b"w\td\r\n258\t-2\r\n", &mut l);
    assert_eq!(c.records, [0, 0x02, 0x01, 0xFE, 0xFF, 0xFF, 0xFF]);
    // Error codes: E13 list (callback type without a callback), E3 zero
    // records, E11 name-key byte, E12 key(code1) / key(code2) overflow.
    assert_eq!(
        outcome(
            &[field("c", T::CustomLink, 0, 0, "")],
            4,
            b"c\r\n1\r\n",
            &mut l
        ),
        ErrorCode::E13
    );
    assert_eq!(
        TxtTable::parse("t.txt", b"f0\r\nExpansion\r\n")
            .unwrap_err()
            .code,
        ErrorCode::E3
    );
    assert_eq!(
        outcome(
            &[field("n", T::NameToIndex, 0, 0, "N11")],
            2,
            b"n\r\n\x85\r\n",
            &mut l
        ),
        ErrorCode::E11
    );
    let codes = |n: usize| {
        let mut t = b"k\r\n".to_vec();
        for i in 0..n {
            t.extend_from_slice(format!("{i}\r\n").as_bytes());
        }
        t
    };
    let mut l = Linkers::default();
    let k1 = [field("k", T::Unknown4, 0, 0, "K1")];
    assert_eq!(outcome(&k1, 1, &codes(256), &mut l), COMPILED);
    let mut l = Linkers::default();
    let e = compile_with(&k1, 1, &codes(257), &mut l, &mut Mock::default()).unwrap_err();
    assert_eq!((e.code, e.line), (ErrorCode::E12, Some(258)));
    let mut l = Linkers::default();
    let k2 = [field("k", T::Unknown5, 0, 0, "K2")];
    assert_eq!(outcome(&k2, 2, &codes(65_536), &mut l), COMPILED);
    let mut l = Linkers::default();
    assert_eq!(outcome(&k2, 2, &codes(65_537), &mut l), ErrorCode::E12);
}

#[test]
fn columns_left_to_right() {
    let mut l = Linkers::default();
    // Two bound fields on one byte: the later column writes last,
    // whatever the field-list order.
    let f = [field("a", T::Byte, 0, 0, ""), field("b", T::Byte, 0, 0, "")];
    assert_eq!(compile(&f, 1, b"a\tb\r\n1\t2\r\n", &mut l).records, [2]);
    assert_eq!(compile(&f, 1, b"b\ta\r\n2\t1\r\n", &mut l).records, [1]);
    // Registrations follow column order too.
    let f = [
        field("k1", T::NameToIndex, 0, 0, "N"),
        field("k2", T::NameToIndex, 0, 2, "N"),
    ];
    let c = compile(&f, 4, b"k2\tk1\r\nx\ty\r\n", &mut l);
    assert_eq!(c.records, [1, 0, 0, 0]);
}

/// Record 0 of a table whose only header column is `fill` (a DWORD at 0
/// holding 0x11111111) plus a missing field `x`.
fn missing_over_fill(x: FieldDef, l: &mut Linkers, m: &mut Mock) -> Vec<u8> {
    let f = [field("fill", T::Dword, 0, 0, ""), x];
    compile_with(&f, 4, b"fill\r\n286331153\r\n", l, m)
        .unwrap()
        .records
}

#[test]
fn cell_conversion_table() {
    let mut l = Linkers::default();
    {
        let Linker::Code(c) = l.ensure("C", LinkerKind::Code).unwrap() else {
            unreachable!()
        };
        c.add(code(b"aa"));
        c.add(code(b"bb"));
    }
    {
        let Linker::Name(n) = l.ensure("N", LinkerKind::Name).unwrap() else {
            unreachable!()
        };
        for i in 0..300 {
            n.find_or_add(format!("k{i}").as_bytes());
        }
    }
    l.ensure("M", LinkerKind::Name).unwrap();
    let mut l2 = l.clone();

    // Every record starts as zero bytes; each type's bound value.
    let one = |t: T, len: u32, link: &str, cell: &[u8], l: &mut Linkers| {
        let mut m = Mock::default();
        let c = compile_with(
            &[field("x", t, len, 0, link)],
            8,
            &column_text("x", &[cell]),
            l,
            &mut m,
        )
        .unwrap();
        (c.records, m)
    };
    let r = |t: T, len: u32, link: &str, cell: &[u8], l: &mut Linkers| one(t, len, link, cell, l).0;
    assert_eq!(r(T::Ascii, 3, "", b"abcdef", &mut l), b"abc\0\0\0\0\0");
    assert_eq!(r(T::Ascii, 3, "", b"ab", &mut l), b"ab\0\0\0\0\0\0");
    assert_eq!(
        r(T::Dword, 0, "", b"-2", &mut l),
        [0xFE, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::Word, 0, "", b"65537", &mut l),
        [1, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(r(T::Byte, 0, "", b"257", &mut l), [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        r(T::Unknown1, 0, "", b"7", &mut l),
        [7, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::Unknown2, 0, "", b"-1", &mut l),
        [0xFF, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(r(T::Byte2, 2, "", b"xyz", &mut l), b"xy\0\0\0\0\0\0");
    assert_eq!(r(T::Dword2, 0, "", b"5", &mut l), [5, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(r(T::Raw, 0, "", b"abcdef", &mut l), b"abcd\0\0\0\0");
    assert_eq!(r(T::Raw, 0, "", b"", &mut l), b"    \0\0\0\0");
    assert_eq!(r(T::AsciiToCode, 0, "C", b"ab", &mut l), b"ab  \0\0\0\0");
    assert_eq!(l.code("C").unwrap().find(code(b"ab")), Some(2));
    assert_eq!(
        r(T::Unknown3, 0, "C", b"bb", &mut l),
        [1, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::Unknown3, 0, "C", b"zz", &mut l),
        [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]
    );
    assert_eq!(r(T::Unknown4, 0, "C", b"cd", &mut l), b"c\0\0\0\0\0\0\0");
    assert_eq!(
        r(T::CodeToByte, 0, "C", b"cd", &mut l),
        [3, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::CodeToByte, 0, "C", b"zz", &mut l),
        [0xFF, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(r(T::Unknown5, 0, "C", b"ef", &mut l), b"ef\0\0\0\0\0\0");
    assert_eq!(
        r(T::CodeToWord, 0, "C", b"ef", &mut l),
        [4, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::CodeToWord, 0, "C", b"zz", &mut l),
        [0xFF, 0xFF, 0, 0, 0, 0, 0, 0]
    );
    // UNKNOWN6: first min(L, max(len, 1) − 1) bytes + NUL; key from them.
    assert_eq!(r(T::Unknown6, 4, "M", b"ABCDEF", &mut l), b"ABC\0\0\0\0\0");
    assert_eq!(l.name("M").unwrap().find(b"abc"), Some(0));
    assert_eq!(
        r(T::NameToIndex, 0, "M", b"ABC", &mut l),
        [0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToIndex, 0, "M", b"New", &mut l),
        [1, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToIndex2, 0, "M", b"two", &mut l),
        [2, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToDword, 0, "N", b"K257", &mut l),
        [1, 1, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToDword, 0, "N", b"zz", &mut l),
        [0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToWord, 0, "N", b"k257", &mut l),
        [1, 1, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToWord, 0, "N", b"zz", &mut l),
        [0xFF, 0xFF, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToWord2, 0, "N", b"k257", &mut l),
        [1, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        r(T::NameToWord2, 0, "N", b"zz", &mut l),
        [0xFF, 0, 0, 0, 0, 0, 0, 0]
    );
    // KEYTOWORD: the key callback's u16 (Mock: 0x100 + text length).
    let (rec, m) = one(T::KeyToWord, 0, "", b"abc", &mut l);
    assert_eq!(
        (rec, m.keys),
        (vec![3, 1, 0, 0, 0, 0, 0, 0], vec![b"abc".to_vec()])
    );
    let long = [b'q'; 300];
    let (_, m) = one(T::KeyToWord, 0, "", &long, &mut l);
    assert_eq!(m.keys[0].len(), 256);
    // Types 23–25: the field callback gets the first 256 bytes and the
    // column; the compiler writes nothing.
    for t in [T::CustomLink, T::Unknown7, T::CalcToDword] {
        let (rec, m) = one(t, 5, "cb:M", &long, &mut l);
        assert_eq!(rec, [0; 8]);
        assert_eq!(m.calls, [(Some(long[..256].to_vec()), 0, 5, 0, 0)]);
    }
    // BIT n = 9: byte 1, mask 0x02.
    assert_eq!(r(T::Bit, 9, "", b"1", &mut l), [0, 2, 0, 0, 0, 0, 0, 0]);
    assert_eq!(r(T::Bit, 9, "", b" ", &mut l), [0, 2, 0, 0, 0, 0, 0, 0]);
    let f = [
        field("fill", T::Dword, 0, 0, ""),
        field("x", T::Bit, 9, 0, ""),
    ];
    let c = compile(&f, 4, b"fill\tx\r\n-1\t0\r\n", &mut l);
    assert_eq!(c.records, [0xFF, 0xFD, 0xFF, 0xFF]);

    // Missing-column values, written after the bound columns, so they win
    // over the overlapping `fill`. A missing own key never registers.
    let x = |t: T, len: u32, link: &str| field("x", t, len, 0, link);
    let l = &mut l2;
    let (c_len, m_len) = (l.code("C").unwrap().len(), l.name("M").unwrap().len());
    let cases: [(FieldDef, [u8; 4]); 22] = [
        (x(T::Ascii, 3, ""), [0, 0x11, 0x11, 0x11]),
        (x(T::Dword, 0, ""), [0; 4]),
        (x(T::Word, 0, ""), [0, 0, 0x11, 0x11]),
        (x(T::Byte, 0, ""), [0, 0x11, 0x11, 0x11]),
        (x(T::Unknown1, 0, ""), [0, 0x11, 0x11, 0x11]),
        (x(T::Unknown2, 0, ""), [0, 0x11, 0x11, 0x11]),
        (x(T::Byte2, 3, ""), [0, 0x11, 0x11, 0x11]),
        (x(T::Dword2, 0, ""), [0; 4]),
        (x(T::Raw, 0, ""), [0; 4]),
        (x(T::AsciiToCode, 0, "C"), [0; 4]),
        (x(T::Unknown3, 0, "C"), [0xFF; 4]),
        (x(T::Unknown4, 0, "C"), [0, 0x11, 0x11, 0x11]),
        (x(T::CodeToByte, 0, "C"), [0xFF, 0x11, 0x11, 0x11]),
        (x(T::Unknown5, 0, "C"), [0, 0, 0x11, 0x11]),
        (x(T::CodeToWord, 0, "C"), [0xFF, 0xFF, 0x11, 0x11]),
        (x(T::Unknown6, 4, "M"), [0, 0x11, 0x11, 0x11]),
        (x(T::NameToIndex, 0, "M"), [0, 0, 0x11, 0x11]),
        (x(T::NameToIndex2, 0, "M"), [0; 4]),
        (x(T::NameToDword, 0, "N"), [0xFF; 4]),
        (x(T::NameToWord, 0, "N"), [0xFF, 0xFF, 0x11, 0x11]),
        (x(T::NameToWord2, 0, "N"), [0xFF, 0x11, 0x11, 0x11]),
        (x(T::Bit, 9, ""), [0x11; 4]),
    ];
    for (f, want) in cases {
        let t = f.field_type;
        assert_eq!(missing_over_fill(f, l, &mut Mock::default()), want, "{t:?}");
    }
    assert_eq!(l.code("C").unwrap().len(), c_len);
    assert_eq!(l.name("M").unwrap().len(), m_len);
    // KEYTOWORD: u16 0, no call. Types 23–25: the callback, without text.
    let mut m = Mock::default();
    assert_eq!(
        missing_over_fill(x(T::KeyToWord, 0, ""), l, &mut m),
        [0, 0, 0x11, 0x11]
    );
    assert!(m.keys.is_empty());
    for t in [T::CustomLink, T::Unknown7, T::CalcToDword] {
        let mut m = Mock::default();
        assert_eq!(missing_over_fill(x(t, 5, "cb:M"), l, &mut m), [0x11; 4]);
        assert_eq!(m.calls, [(None, 0, 5, 0, 1)]);
    }
}

#[test]
fn pass_one_registers_first() {
    let mut l = Linkers::default();
    // A record links to a later record of its own table.
    let f = [
        field("id", T::NameToIndex, 0, 0, "T"),
        field("next", T::NameToWord, 0, 2, "T"),
    ];
    let c = compile(&f, 4, b"id\tnext\r\na\tb\r\nb\ta\r\n", &mut l);
    assert_eq!(c.records, [0, 0, 1, 0, 1, 0, 0, 0]);
    // Record by record, columns left to right.
    let f = [
        field("k1", T::NameToIndex, 0, 0, "U"),
        field("k2", T::NameToIndex, 0, 2, "U"),
    ];
    let c = compile(&f, 4, b"k1\tk2\r\nx\ty\r\nz\tw\r\n", &mut l);
    assert_eq!(c.records, [0, 0, 1, 0, 2, 0, 3, 0]);
    // Code registrations too: a CODETOBYTE in an earlier column sees the
    // key of a later record.
    let f = [
        field("k", T::AsciiToCode, 0, 0, "V"),
        field("p", T::CodeToByte, 0, 4, "V"),
    ];
    let c = compile(&f, 5, b"p\tk\r\nb\ta\r\na\tb\r\n", &mut l);
    assert_eq!(c.records, *b"a   \x01b   \x00");
    assert_eq!(l.code("V").unwrap().len(), 2);
}

#[test]
fn pass_two_order() {
    let mut l = Linkers::default();
    let f = [
        field("m1", T::CustomLink, 1, 0, "cb:M"),
        field("b1", T::CustomLink, 2, 0, "cb:M"),
        field("b0", T::Unknown7, 3, 0, "cb:M"),
        field("m2", T::CalcToDword, 4, 0, "cb:M"),
        field("k", T::AsciiToCode, 0, 0, "K"),
    ];
    let mut m = Mock::default();
    compile_with(
        &f,
        4,
        b"b0\tk\tb1\r\nx\tc\ty\r\nz\tc\tw\r\n",
        &mut l,
        &mut m,
    )
    .unwrap();
    let order: Vec<(u32, u32, u32)> = m.calls.iter().map(|c| (c.3, c.2, c.4)).collect();
    // (record, len = field, slot): bound columns left to right, then the
    // missing fields in list order at slots C + 0, C + 1.
    assert_eq!(
        order,
        [
            (0, 3, 0),
            (0, 2, 2),
            (0, 1, 3),
            (0, 4, 4),
            (1, 3, 0),
            (1, 2, 2),
            (1, 1, 3),
            (1, 4, 4)
        ]
    );
    // Own keys are not converted again in pass 2: two adds, one bump.
    assert_eq!(l.code("K").unwrap().len(), 2);
}

#[test]
fn linker_rules() {
    // A linker lives across tables: n is never reset.
    let mut l = Linkers::default();
    let key = [field("code", T::AsciiToCode, 0, 0, "items.code")];
    compile(&key, 4, b"code\r\naa\r\nbb\r\n", &mut l);
    compile(&key, 4, b"code\r\ncc\r\n", &mut l);
    let look = [field("r", T::Unknown3, 0, 0, "items.code")];
    let c = compile(&look, 4, b"r\r\ncc\r\naa\r\n", &mut l);
    assert_eq!(c.records, [2, 0, 0, 0, 0, 0, 0, 0]);

    // Code register: bump while present (wrapping); every cell takes an
    // index; lookups are exact and case-sensitive.
    let mut c = CodeLinker::default();
    assert_eq!(c.add(code(b"abc")), (0, false));
    assert_eq!(c.add(code(b"abc")), (1, true));
    assert_eq!(c.find(code(b"bbc")), Some(1));
    assert_eq!(c.add(code(b"bbc")), (2, true));
    assert_eq!(c.find(code(b"cbc")), Some(2));
    assert_eq!(c.find(code(b"ABC")), None);
    assert_eq!(c.add(code(b"")), (3, false));
    assert_eq!(c.find(0x2020_2020), Some(3));
    assert_eq!(c.add(u32::MAX), (4, false));
    assert_eq!(c.add(u32::MAX), (5, true));
    assert_eq!(c.find(0), Some(5));

    // Name register-always (type 16): a duplicate takes an index nothing
    // maps to.
    let mut n = NameLinker::default();
    assert_eq!(n.add_always(b"x"), (0, true));
    assert_eq!(n.add_always(b"x"), (1, false));
    assert_eq!(n.add_always(b"y"), (2, true));
    assert_eq!((n.find(b"x"), n.find(b"y"), n.len()), (Some(0), Some(2), 3));
    // Find-or-register (17, 18): a duplicate gets the first index.
    let mut l = Linkers::default();
    let f = [field("id", T::NameToIndex2, 0, 0, "S")];
    let c = compile(&f, 4, b"id\r\na\r\nb\r\nA\r\nc\r\n", &mut l);
    assert_eq!(c.records, [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(l.name("S").unwrap().len(), 3);
    // Lookups (19–21) normalize: case-insensitive, first 31 bytes.
    let mut long = vec![b'p'; 31];
    long.extend_from_slice(b"one");
    let mut other = vec![b'P'; 31];
    other.extend_from_slice(b"two");
    let f = [field("id", T::NameToIndex, 0, 0, "L31")];
    compile(&f, 2, &column_text("id", &[b"x", &long]), &mut l);
    let f = [field("r", T::NameToWord, 0, 0, "L31")];
    let c = compile(&f, 2, &column_text("r", &[b"X", &other]), &mut l);
    assert_eq!(c.records, [0, 0, 1, 0]);

    // Empty keys: an empty cell registers the empty key; an empty lookup
    // hits it, or misses (−1) without it; a missing lookup field is −1
    // even when the empty key exists.
    let mut l = Linkers::default();
    compile(
        &[field("id", T::NameToIndex, 0, 0, "E")],
        2,
        b"id\r\n\r\nq\r\n",
        &mut l,
    );
    compile(
        &[field("c", T::AsciiToCode, 0, 0, "EC")],
        4,
        b"c\r\n\r\nq\r\n",
        &mut l,
    );
    l.ensure("NE", LinkerKind::Name).unwrap();
    let f = [
        field("a", T::NameToWord, 0, 0, "E"),
        field("b", T::CodeToWord, 0, 2, "EC"),
        field("c", T::NameToWord, 0, 4, "NE"),
        field("m1", T::NameToWord, 0, 6, "E"),
        field("m2", T::CodeToWord, 0, 8, "EC"),
    ];
    let c = compile(&f, 10, b"a\tb\tc\r\n\t\t\r\n", &mut l);
    assert_eq!(c.records, [0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
}

/// A one-slot string table holding `key` as element `element`.
fn one_key_table(key: &[u8], element: u16) -> d2_formats::tbl::StringTable {
    use d2_formats::tbl::{key_hash, StringTable, TblEntry, TblHeader};
    StringTable {
        header: TblHeader {
            crc: 0,
            num_elements: element + 1,
            hash_table_size: 1,
            version: 0,
            data_start: 0,
            max_tries: 1,
            file_size: 0,
        },
        indices: vec![0; usize::from(element) + 1],
        entries: vec![TblEntry {
            used: 1,
            index: element,
            hash: key_hash(key),
            key: key.to_vec(),
            value: b"v".to_vec(),
        }],
    }
}

#[test]
fn field_type_edge_cases() {
    let mut l = Linkers::default();
    // Missing vs empty code4 / key(code4): zeros vs spaces.
    let f = [
        field("b", T::Byte, 0, 8, ""),
        field("r", T::Raw, 0, 0, ""),
        field("k", T::AsciiToCode, 0, 4, "K"),
    ];
    assert_eq!(
        compile(&f, 9, b"b\r\n1\r\n", &mut l).records,
        [0, 0, 0, 0, 0, 0, 0, 0, 1]
    );
    assert_eq!(
        compile(&f, 9, b"b\tr\tk\r\n1\t\t\r\n", &mut l).records,
        *b"        \x01"
    );
    // strkey: missing → 0 without a lookup; empty and unknown → 5382;
    // string.tbl element 0 → 5382 (unreachable); patch element 3 → 10003.
    let strings = StringTables {
        base: Some(one_key_table(b"zero", 0)),
        patch: Some(one_key_table(b"p", 3)),
        expansion: None,
        layer: Default::default(),
    };
    let mut cb = StdCallbacks::new(&strings);
    let f = [
        field("s", T::KeyToWord, 0, 0, ""),
        field("m", T::KeyToWord, 0, 2, ""),
    ];
    let c = compile_with(&f, 4, b"s\r\n\r\nzz\r\nzero\r\np\r\n", &mut l, &mut cb).unwrap();
    let ids: Vec<(u16, u16)> = c
        .records
        .chunks(4)
        .map(|r| {
            (
                u16::from_le_bytes([r[0], r[1]]),
                u16::from_le_bytes([r[2], r[3]]),
            )
        })
        .collect();
    assert_eq!(ids, [(5382, 0), (5382, 0), (5382, 0), (10_003, 0)]);
    // Link cells: an empty cell hits the empty key; a missing column is −1.
    let mut l = Linkers::default();
    compile(
        &[field("c", T::AsciiToCode, 0, 0, "E")],
        4,
        b"c\r\n\r\n",
        &mut l,
    );
    let f = [
        field("a", T::CodeToByte, 0, 0, "E"),
        field("m", T::CodeToByte, 0, 1, "E"),
    ];
    assert_eq!(compile(&f, 2, b"a\r\n\r\n", &mut l).records, [0, 0xFF]);
    // str(N) NUL spill onto the next field: the later column wins.
    let f = [
        field("s", T::Ascii, 2, 0, ""),
        field("b", T::Byte, 0, 2, ""),
    ];
    assert_eq!(
        compile(&f, 3, b"b\ts\r\n7\txy\r\n", &mut l).records,
        *b"xy\0"
    );
    assert_eq!(
        compile(&f, 3, b"b\ts\r\n7\tx\r\n", &mut l).records,
        *b"x\0\x07"
    );
    assert_eq!(
        compile(&f, 3, b"s\tb\r\nxy\t7\r\n", &mut l).records,
        *b"xy\x07"
    );
    // Bumped duplicate codes take a later real code's key.
    let mut l = Linkers::default();
    let f = [field("k", T::AsciiToCode, 0, 0, "D")];
    compile(&f, 4, b"k\r\nabc\r\nabc\r\nbbc\r\n", &mut l);
    let d = l.code("D").unwrap();
    assert_eq!(
        (
            d.find(code(b"abc")),
            d.find(code(b"bbc")),
            d.find(code(b"cbc"))
        ),
        (Some(0), Some(1), Some(2))
    );
    // link8 keeps the low byte of an index ≥ 256.
    let mut l = Linkers::default();
    {
        let Linker::Code(c) = l.ensure("B", LinkerKind::Code).unwrap() else {
            unreachable!()
        };
        for i in 0..=257u32 {
            assert_eq!(c.add(code(format!("{i:03}").as_bytes())), (i, false));
        }
    }
    let f = [field("b", T::CodeToByte, 0, 0, "B")];
    assert_eq!(compile(&f, 1, b"b\r\n257\r\n", &mut l).records, [0x01]);
    // 31-byte name keys.
    let mut long = vec![b'n'; 31];
    long.extend_from_slice(b"a");
    let mut other = vec![b'N'; 31];
    other.extend_from_slice(b"b");
    let f = [field("id", T::NameToIndex, 0, 0, "L")];
    let c = compile(&f, 2, &column_text("id", &[&long, &other]), &mut l);
    assert_eq!(c.records, [0, 0, 0, 0]);
    // Not reproduced: a name-key byte ≥ 0x80 is E11, also in `param`.
    assert_eq!(outcome(&f, 2, b"id\r\nx\xC0\r\n", &mut l), ErrorCode::E11);
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let f = [FieldDef::new("p", T::CalcToDword, 0, 0, Link::Param)];
    let e = compile_with(&f, 4, b"p\r\nx\xC0\r\n", &mut l, &mut cb).unwrap_err();
    assert_eq!(e.code, ErrorCode::E11);
}

#[test]
fn txt_edge_cases() {
    let mut l = Linkers::default();
    // Missing RAW (9) / ASCIITOCODE (10): u32 0; empty bound: "    ".
    let f = [
        field("b", T::Byte, 0, 8, ""),
        field("r", T::Raw, 0, 0, ""),
        field("k", T::AsciiToCode, 0, 4, "K"),
    ];
    assert_eq!(
        compile(&f, 9, b"b\r\n1\r\n", &mut l).records,
        [0, 0, 0, 0, 0, 0, 0, 0, 1]
    );
    assert_eq!(
        compile(&f, 9, b"r\tk\tb\r\n\t\t1\r\n", &mut l).records,
        *b"        \x01"
    );
    // ASCII writes its NUL at offset + min(L, len): len + 1 bytes.
    let f = [field("s", T::Ascii, 3, 1, "")];
    assert_eq!(compile(&f, 5, b"s\r\nab\r\n", &mut l).records, *b"\0ab\0\0");
    assert_eq!(
        compile(&f, 5, b"s\r\nabcd\r\n", &mut l).records,
        *b"\0abc\0"
    );
    // NAMETOWORD2 stores one byte; the next field starts one byte later.
    let mut l = Linkers::default();
    compile(
        &[field("id", T::NameToIndex, 0, 0, "N")],
        2,
        b"id\r\nx\r\n",
        &mut l,
    );
    let f = [
        field("p", T::NameToWord2, 0, 0, "N"),
        field("q", T::Byte, 0, 1, ""),
    ];
    assert_eq!(compile(&f, 2, b"q\tp\r\n9\tx\r\n", &mut l).records, [0, 9]);
    // Types 12 and 14 store code bytes, not the index.
    let mut l = Linkers::default();
    compile(
        &[field("k", T::AsciiToCode, 0, 0, "C")],
        4,
        b"k\r\nzz\r\n",
        &mut l,
    );
    let f = [
        field("a", T::Unknown4, 0, 0, "C"),
        field("b", T::Unknown5, 0, 1, "C"),
    ];
    assert_eq!(
        compile(&f, 3, b"a\tb\r\nqr\tst\r\n", &mut l).records,
        *b"qst"
    );
    // Type 10 stores the cell's code; a bumped key exists only in the
    // linker (the itemtypes pattern).
    let mut l = Linkers::default();
    let f = [field("k", T::AsciiToCode, 0, 0, "I")];
    let c = compile(&f, 4, b"k\r\n\r\n\r\n\r\n", &mut l);
    assert_eq!(c.records, *b"            ");
    let i = l.code("I").unwrap();
    assert_eq!(
        (
            i.find(0x2020_2020),
            i.find(0x2020_2021),
            i.find(0x2020_2022)
        ),
        (Some(0), Some(1), Some(2))
    );
    // A bumped duplicate takes a later real code's key; that code is bumped.
    let mut l = Linkers::default();
    let c = compile(&f, 4, b"k\r\nabc\r\nabc\r\nbbc\r\n", &mut l);
    assert_eq!(c.records, *b"abc abc bbc ");
    let i = l.code("I").unwrap();
    assert_eq!(
        (i.find(code(b"bbc")), i.find(code(b"cbc"))),
        (Some(1), Some(2))
    );
    // An empty lookup cell hits an empty key; a missing column is −1.
    let f = [
        field("e", T::NameToWord, 0, 0, "E"),
        field("m", T::NameToWord, 0, 2, "E"),
    ];
    compile(
        &[field("id", T::NameToIndex, 0, 0, "E")],
        2,
        b"id\r\n\r\n",
        &mut l,
    );
    assert_eq!(
        compile(&f, 4, b"e\r\n\r\n", &mut l).records,
        [0, 0, 0xFF, 0xFF]
    );
    // Column map: more than 280 slots is E14 (the column map holds that many).
    let mut text = b"zz".to_vec();
    for i in 1..280 {
        text.extend_from_slice(format!("\tc{i}").as_bytes());
    }
    text.extend_from_slice(b"\r\n");
    text.extend(std::iter::repeat_n(b'\t', 279));
    text.extend_from_slice(b"\r\n");
    let f = [
        field("zz", T::Byte, 0, 0, ""),
        field("miss", T::Byte, 0, 0, ""),
    ];
    assert_eq!(outcome(&f, 1, &text, &mut l), ErrorCode::E14);
    assert_eq!(outcome(&f[..1], 1, &text, &mut l), COMPILED);
}

#[test]
fn hratli_magic_lvl_header_does_not_bind() {
    // `fields.tsv` names Hratli's weapons / armor column `HraltiMagicLvl`
    // at offset 400; the game's `weapons.txt` / `armor.txt` headers spell
    // it `HratliMagicLvl`, so it does not bind and offset 400 keeps the
    // missing-column value 0 whatever the cells hold. `misc.txt` spells
    // it as the field does, and its cells load.
    for table in ["weapons", "armor", "misc"] {
        let def = crate::schema::schema().table(table).unwrap();
        let f = def.field("HraltiMagicLvl").unwrap().clone();
        assert_eq!(f.offset, 400, "{table}");
        let size = 401;
        let mut l = Linkers::default();
        let c = compile(
            std::slice::from_ref(&f),
            size,
            b"HratliMagicLvl\r\n20\r\n",
            &mut l,
        );
        assert_eq!(c.record(0)[400], 0, "{table}: typo header binds nothing");
        let c = compile(&[f], size, b"HraltiMagicLvl\r\n255\r\n", &mut l);
        assert_eq!(c.record(0)[400], 255, "{table}: field spelling binds");
    }
}
