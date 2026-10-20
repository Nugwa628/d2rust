//! Test vectors for the formula compiler, the constant evaluator and the code-buffer
//! validator in `calc`: synthetic formulas with fixed links, plus `#[ignore]`d checks
//! against the player's own game files (`D2_GAME_DIR`).

use super::*;

/// Parameter codes of the skill and missile formula tables.
const SKILLCALC: [&str; 73] = [
    "ln12", "dm12", "ln34", "dm34", "ln56", "dm56", "ln78", "dm78", "par1", "par2", "par3", "par4",
    "par5", "par6", "par7", "par8", "lvl", "edmn", "edmx", "edln", "toht", "mana", "mps", "math",
    "madm", "macr", "m1en", "m1ex", "m1el", "m2en", "m2ex", "m2el", "m3en", "m3ex", "m3el", "m1rn",
    "m2rn", "m3rn", "edns", "edxs", "ulvl", "blvl", "usmc", "m1eo", "m1ey", "m2eo", "m2ey", "me3o",
    "me3y", "enma", "exma", "edma", "enms", "exms", "len", "clc1", "clc2", "clc3", "clc4", "rng",
    "ast1", "ast2", "ast3", "ast4", "ast5", "ast6", "pst1", "pst2", "pst3", "pst4", "pst5", "pets",
    "skpt",
];
const MISSCALC: [&str; 43] = [
    "par1", "par2", "par3", "par4", "par5", "cpa1", "cpa2", "cpa3", "cpa4", "cpa5", "hpa1", "hpa2",
    "hpa3", "chp1", "chp2", "chp3", "dpa1", "dpa2", "lvl", "edmn", "edmx", "edln", "edns", "edxs",
    "damn", "damx", "dmns", "dmxs", "rang", "sl12", "sd12", "sl34", "sd34", "cl12", "cd12", "cl34",
    "cd34", "shl1", "shd1", "chl1", "chd1", "dl12", "dd12",
];

/// The links of the synthetic vectors: `Fire Bolt` = skill 36,
/// `firebolt` = missile 58, `strength` = stat 0, `Holy Fire` = skill 102.
pub(super) struct Links;

impl CalcLinks for Links {
    fn skill(&self, key: &[u8]) -> Option<u32> {
        match key {
            b"fire bolt" => Some(36),
            b"holy fire" => Some(102),
            _ => None,
        }
    }
    fn missile(&self, key: &[u8]) -> Option<u32> {
        (key == b"firebolt").then_some(58)
    }
    fn stat(&self, key: &[u8]) -> Option<u32> {
        (key == b"strength").then_some(0)
    }
    fn skillcalc(&self, code: u32) -> Option<u32> {
        SKILLCALC
            .iter()
            .position(|c| code_of(c.as_bytes()) == code)
            .map(|i| i as u32)
    }
    fn misscalc(&self, code: u32) -> Option<u32> {
        MISSCALC
            .iter()
            .position(|c| code_of(c.as_bytes()) == code)
            .map(|i| i as u32)
    }
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

fn check(family: Family, text: &str, expected: &str) {
    let got = compile(family, &Links, text.as_bytes()).unwrap().code;
    let want = if expected == "fail" {
        Vec::new()
    } else {
        hex(expected)
    };
    assert_eq!(got, want, "{family:?} `{text}`");
}

#[test]
fn skills_synthetic() {
    let s = |t, e| check(Family::Skills, t, e);
    for t in ["", "   ", "\"\"", "()", ",", "min("] {
        s(t, "fail");
    }
    s("7", "07 07 00");
    s("-7", "07 F9 00");
    s("- 5", "07 FB 00");
    s("300", "08 2C 01 00");
    s("-300", "08 D4 FE 00");
    s("100000", "09 A0 86 01 00 00");
    s("2147483648", "09 00 00 00 80 00");
    s("4294967297", "07 01 00");
    s("2+3*4", "07 0E 00");
    s("(2+3)*4", "07 14 00");
    s("10-4-3", "07 03 00");
    s("2^3^2", "07 40 00");
    s("-2^2", "07 04 00");
    s("2^-1", "07 01 00");
    s("0^0", "07 01 00");
    s("2*-3", "07 FA 00");
    s("7/2", "07 03 00");
    s("-7/2", "07 FD 00");
    s("7/-2", "07 FD 00");
    s("5/0", "07 00 00");
    s("1<2", "07 01 00");
    s("2>=2", "07 01 00");
    s("2<=1", "07 00 00");
    s("3==3", "07 01 00");
    s("3!=3", "07 00 00");
    s("3>2>1", "07 00 00");
    s("1?2:3", "07 02 00");
    s("0?2:3", "07 03 00");
    s("1 < 2 ? 5 : 6", "07 01 00");
    s("(1<2)?5:6", "07 05 00");
    s("1?2:3+10", "07 0C 00");
    s("-1?2:3", "07 FE 00");
    s("(1<2)?(3+4):5*2", "07 0E 00");
    s("1?2 3", "07 02 00");
    s("0?2 3", "07 03 00");
    s("(lvl<4)?lvl 3", "04 10 07 04 0A 04 10 07 03 16 00");
    for t in ["1?2+1:3", "1?-2:3", "1?2", "--5", "+5", "5*", "lvl+", "1)"] {
        s(t, "fail");
    }
    s("1=1", "07 01 00");
    s("1.5", "07 01 00");
    s("5 $ 3", "07 05 00");
    s("5 3", "07 03 00");
    s("1:2", "07 02 00");
    s("1,2", "07 02 00");
    s("(", "07 00 00");
    s("ln12", "04 00 00");
    s("lvl", "04 10 00");
    s("'lvl'", "04 10 00");
    s("\"ln12\"", "04 00 00");
    s("LVL", "07 00 00");
    s("foo", "07 00 00");
    s("a.b", "07 00 00");
    s("par34", "04 0A 00");
    s("1+ln12", "07 01 04 00 10 00");
    s("ln12\"+1\"", "04 00 07 01 10 00");
    for t in ["min(3,5)", "MIN(3,5)", "min (3,5)"] {
        s(t, "07 03 07 05 01 00 00");
    }
    for t in [
        "min(-1,2)",
        "min((-1),2)",
        "skill(ln12)",
        "stat('strength')",
    ] {
        s(t, "fail");
    }
    s("min(0-1,2)", "07 00 07 01 11 07 02 01 00 00");
    s("max(2,0-1)", "07 02 07 00 07 01 11 01 01 00");
    s("min(1,2,3)", "07 01 07 02 07 03 01 00 00");
    s("min(1,2)min(3,4)", "07 01 07 02 01 00 07 03 07 04 01 00 00");
    s("max(1,2", "07 01 07 02 00");
    s("lvl(2)", "04 10 07 02 00");
    s("lvl*(2", "04 10 07 02 02 12 00");
    s("rand(1,6)", "07 01 07 06 01 02 00");
    s("skill('Fire Bolt'.lvl)", "07 24 07 10 01 03 00");
    s("skill('Fire Bolt'.blvl", "07 24 07 29 00");
    s("skill('fire bolt'.LVL)", "07 24 00");
    s("skill('No Such Skill'.lvl)", "07 00 07 10 01 03 00");
    s("stat('strength'.base)", "07 00 07 01 01 05 00");
    s("stat('strength'.mod)", "07 00 07 02 01 05 00");
    s("stat('strength'.accr)", "07 00 07 00 01 05 00");
    s("stat('strength'.base)+1", "07 00 07 01 01 05 07 01 10 00");
    s("stat('nosuchstat'.base)", "04 00 07 01 01 05 00");
    s("miss('firebolt'.edmn)", "07 3A 07 13 01 04 00");
    s("sklvl('Fire Bolt'.ln12.lvl)", "07 24 07 00 07 10 01 06 00");
    // Formulas taken from the game's skill tables (the links they need are in `Links`).
    s("\"-min(ln34,150)\"", "04 02 08 96 00 01 00 15 00");
    s("-par3 * lvl", "04 0A 15 04 10 12 00");
    s(
        "(lvl < 4) ?lvl:(2+lvl/3)",
        "04 10 07 04 0A 04 10 07 02 04 10 07 03 13 10 16 00",
    );
    s(
        "\"(lvl < 5) ? lvl : min(12,5+(lvl-5)/3)\"",
        "04 10 07 05 0A 04 10 07 0C 07 05 04 10 07 05 11 07 03 13 10 01 00 16 00",
    );
    s("sklvl('Holy Fire'.ln56.edmn)", "07 66 07 04 07 11 01 06 00");
    s(" ", "fail");
}

#[test]
fn other_families() {
    let m = |t, e| check(Family::Missiles, t, e);
    m("dl12", "04 29 00");
    m("lvl", "04 12 00");
    m("miss('firebolt'.dl12)", "07 3A 07 29 01 04 00");
    m("skill('Fire Bolt'.sl12)", "07 24 00");
    m("stat('strength'.base)", "07 00 00");
    m("sklvl('Fire Bolt'.ln12.lvl)", "07 00 00");
    let i = |t, e| check(Family::Items, t, e);
    i("5", "07 05 00");
    i("750", "08 EE 02 00");
    i("lvl", "04 00 00");
    i("lvl+1", "04 00 07 01 10 00");
    i("stat('strength'.base)", "07 00 07 01 01 03 00");
    i(
        "min(stat('strength'.base),10)",
        "07 00 07 01 01 03 07 0A 01 00 00",
    );
    i("skill(1,2)", "04 00 07 01 07 02 00");
    i("rand(1,3)", "07 01 07 03 01 02 00");
}

#[test]
fn refusals() {
    assert_eq!(
        compile(Family::Skills, &Links, b"lvl\xE9").unwrap_err(),
        CalcError::NonAscii
    );
    assert_eq!(
        compile(Family::Missiles, &Links, b"rand(1,3)").unwrap_err(),
        CalcError::MissileRand
    );
}

#[test]
fn diagnostics() {
    let d = |t: &str| {
        compile(Family::Skills, &Links, t.as_bytes())
            .unwrap()
            .diagnostics
    };
    assert_eq!(d("1?2"), [CalcDiag::Fail]);
    assert_eq!(d("5 $ 3"), [CalcDiag::Stop]);
    assert_eq!(d("foo"), [CalcDiag::UnknownName]);
    assert_eq!(d("lvl*(2"), [CalcDiag::OpenParen]);
    assert_eq!(d("max(1,2"), [CalcDiag::OpenFunction]);
    assert!(d("lvl+1").is_empty());
}

#[test]
fn constant_evaluator() {
    let e = |s: &str| eval_const(&hex(s));
    assert_eq!(e("07 05 07 00 13 00"), 0);
    assert_eq!(e("07 F9 07 02 13 00"), -3);
    assert_eq!(e("07 07 07 FE 13 00"), -3);
    assert_eq!(e("07 FE 07 03 14 00"), -8);
    assert_eq!(e("07 02 07 00 14 00"), 1);
    assert_eq!(e("07 02 07 FF 14 00"), 1);
    assert_eq!(e("09 00 00 00 80 15 00"), i32::MIN);
    assert_eq!(e("10 00"), 0);
    assert_eq!(e("07 05 07 06 16 00"), 6);
    assert_eq!(e("07 00 07 01 16 07 05 00"), 5);
    assert_eq!(e("07 FF 00"), -1);
    assert_eq!(e("07 05 03 07 06 00"), 5);
    assert_eq!(e("07 05 17 07 06 00"), 5);
    assert_eq!(e("07 05 FF 07 06 00"), 5);
    assert_eq!(e("07 05 02 07 06 10 00"), 5);
    assert_eq!(e("07"), 0);
    assert_eq!(e("07 05"), 0);
    let mut many = hex(&"07 01 ".repeat(64));
    many.extend(hex("07 02 00"));
    assert_eq!(eval_const(&many), 1);
}

#[test]
fn buffer_validation() {
    let v = |buf: &str, fields: &[u32]| {
        validate_buffer(Family::Skills, &hex(buf), fields.iter().copied())
    };
    assert!(matches!(
        v("07 05 00 07", &[]),
        Err(BufferError::Truncated { .. })
    ));
    assert!(matches!(
        v("08 05", &[]),
        Err(BufferError::Truncated { .. })
    ));
    assert!(matches!(
        v("07 05 03 00", &[]),
        Err(BufferError::BadOpcode { .. })
    ));
    let r = v("04 29 00 07 00 00", &[0, 3]).unwrap();
    assert_eq!(
        (r.starts.clone(), r.shared, r.unreferenced),
        (vec![0, 3], 0, 0)
    );
    assert!(matches!(
        v("04 29 00 07 00 00", &[0, 1]),
        Err(BufferError::BadField { value: 1 })
    ));
    assert_eq!(v("04 29 00 07 00 00", &[0, 3, 3]).unwrap().shared, 1);
    assert_eq!(v("04 29 00 07 00 00", &[u32::MAX]).unwrap().unreferenced, 2);
}

/// Links with large code indices and a key-length skill lookup, for the
/// push-value widths and the 31-byte name key.
struct BigLinks;

impl CalcLinks for BigLinks {
    fn skill(&self, key: &[u8]) -> Option<u32> {
        if key.starts_with(b"abcdefghij") {
            Some(key.len() as u32)
        } else {
            Links.skill(key)
        }
    }
    fn missile(&self, key: &[u8]) -> Option<u32> {
        Links.missile(key)
    }
    fn stat(&self, key: &[u8]) -> Option<u32> {
        Links.stat(key)
    }
    fn skillcalc(&self, code: u32) -> Option<u32> {
        if code == code_of(b"big1") {
            Some(200)
        } else if code == code_of(b"big2") {
            Some(40_000)
        } else {
            Links.skillcalc(code)
        }
    }
    fn misscalc(&self, code: u32) -> Option<u32> {
        Links.misscalc(code)
    }
}

fn check_big(text: &str, expected: &str) {
    let got = compile(Family::Skills, &BigLinks, text.as_bytes())
        .unwrap()
        .code;
    let want = if expected == "fail" {
        Vec::new()
    } else {
        hex(expected)
    };
    assert_eq!(got, want, "`{text}`");
}

fn diags(family: Family, text: &str) -> Vec<CalcDiag> {
    compile(family, &Links, text.as_bytes())
        .unwrap()
        .diagnostics
}

/// Integers are little-endian; i8/i16/i32 operands are two's complement.
#[test]
fn little_endian_signed_operands() {
    let s = |t, e| check(Family::Skills, t, e);
    s("300", "08 2C 01 00");
    s("-300", "08 D4 FE 00");
    s("100000", "09 A0 86 01 00 00");
    s("0-100000", "09 60 79 FE FF 00");
    let e = |s: &str| eval_const(&hex(s));
    assert_eq!(e("08 2C 01 00"), 300);
    assert_eq!(e("08 D4 FE 00"), -300);
    assert_eq!(e("09 A0 86 01 00 00"), 100_000);
    assert_eq!(e("09 FF FF FF FF 00"), -1);
    assert_eq!(e("07 80 00"), -128);
}

/// Calc columns, offsets and buffers of the four families, from the
/// embedded `fields.tsv` (no game files).
#[test]
fn calc_field_offsets() {
    use crate::schema::{schema, Link};
    let mut want: Vec<(&str, String, u32)> = Vec::new();
    for (c, o) in [
        ("SrvCalc1", 128),
        ("CltCalc1", 132),
        ("SHitCalc1", 136),
        ("CHitCalc1", 140),
        ("DmgCalc1", 144),
        ("DmgSymPerCalc", 224),
        ("EDmgSymPerCalc", 280),
    ] {
        want.push(("missiles", c.into(), o));
    }
    let mut skills: Vec<(String, u32)> = vec![
        ("prgcalc1".into(), 56),
        ("prgcalc2".into(), 60),
        ("prgcalc3".into(), 64),
        ("auralencalc".into(), 96),
        ("aurarangecalc".into(), 100),
        ("petmax".into(), 192),
        ("perdelay".into(), 296),
        ("skpoints".into(), 368),
        ("delay".into(), 400),
        ("ToHitCalc".into(), 416),
        ("DmgSymPerCalc".into(), 472),
        ("EDmgSymPerCalc".into(), 528),
        ("ELenSymPerCalc".into(), 548),
    ];
    let run = |v: &mut Vec<(String, u32)>, name: &dyn Fn(u32) -> String, n: u32, base: u32| {
        for k in 0..n {
            v.push((name(k + 1), base + 4 * k));
        }
    };
    run(&mut skills, &|k| format!("aurastatcalc{k}"), 6, 104);
    run(&mut skills, &|k| format!("passivecalc{k}"), 5, 164);
    run(&mut skills, &|k| format!("sumsk{k}calc"), 5, 208);
    run(&mut skills, &|k| format!("cltcalc{k}"), 3, 276);
    run(&mut skills, &|k| format!("calc{k}"), 4, 312);
    want.extend(skills.into_iter().map(|(c, o)| ("skills", c, o)));
    let mut desc: Vec<(String, u32)> = vec![
        ("ddam calc1".into(), 24),
        ("ddam calc2".into(), 28),
        ("p1dmmin".into(), 36),
        ("p2dmmin".into(), 40),
        ("p3dmmin".into(), 44),
        ("p1dmmax".into(), 48),
        ("p2dmmax".into(), 52),
        ("p3dmmax".into(), 56),
    ];
    run(&mut desc, &|k| format!("desccalca{k}"), 6, 152);
    run(&mut desc, &|k| format!("dsc2calca{k}"), 4, 176);
    run(&mut desc, &|k| format!("dsc3calca{k}"), 7, 192);
    run(&mut desc, &|k| format!("desccalcb{k}"), 6, 220);
    run(&mut desc, &|k| format!("dsc2calcb{k}"), 4, 244);
    run(&mut desc, &|k| format!("dsc3calcb{k}"), 7, 260);
    want.extend(desc.into_iter().map(|(c, o)| ("skilldesc", c, o)));
    for table in ["weapons", "armor", "misc"] {
        for (c, o) in [
            ("calc1", 164),
            ("calc2", 168),
            ("calc3", 172),
            ("len", 176),
            ("spelldesccalc", 184),
        ] {
            want.push((table, c.into(), o));
        }
    }
    let buffer = |t: &str| match t {
        "missiles" => CalcBuffer::MissCode,
        "skills" => CalcBuffer::SkillsCode,
        "skilldesc" => CalcBuffer::SkillDescCode,
        _ => CalcBuffer::ItemsCode,
    };
    // Every calc field of the schema, and only those, in the six tables.
    let mut got: Vec<(String, Vec<u8>, u32)> = Vec::new();
    for def in &schema().tables {
        for f in &def.fields {
            if let Link::Calc(b) = f.link {
                assert_eq!(f.field_type, crate::schema::FieldType::CalcToDword);
                assert_eq!(b, buffer(&def.name), "{} {}", def.name, f.name());
                got.push((def.name.clone(), f.column.to_ascii_lowercase(), f.offset));
            }
        }
    }
    let mut want: Vec<(String, Vec<u8>, u32)> = want
        .into_iter()
        .map(|(t, c, o)| (t.to_owned(), c.to_ascii_lowercase().into_bytes(), o))
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got.len(), 100);
    assert_eq!(got, want);
    // Record sizes of the four tables.
    for (t, size) in [
        ("missiles", 420),
        ("skills", 572),
        ("skilldesc", 288),
        ("weapons", 424),
        ("armor", 424),
        ("misc", 424),
    ] {
        assert_eq!(schema().table(t).unwrap().record_size, size, "{t}");
    }
}

/// Compiles `text` as one table with calc fields `fields` (name, offset)
/// into a fresh `StdCallbacks`; returns the u32 fields per record and the
/// buffer.
fn table_compile(
    buffer: CalcBuffer,
    fields: &[(&str, u32)],
    size: usize,
    text: &[u8],
) -> (Vec<Vec<u32>>, Vec<u8>) {
    use crate::compile::{compile_table, Linkers, StdCallbacks};
    use crate::schema::{FieldDef, FieldType, Link};
    use crate::strings::StringTables;
    use crate::txt::TxtTable;
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    let mut linkers = Linkers::default();
    let defs: Vec<FieldDef> = fields
        .iter()
        .map(|&(c, o)| FieldDef::new(c, FieldType::CalcToDword, 0, o, Link::Calc(buffer)))
        .collect();
    let txt = TxtTable::parse("t.txt", text).unwrap();
    let c = compile_table("t.txt", &txt, &defs, size, &mut linkers, &mut cb).unwrap();
    let records = (0..c.count)
        .map(|r| {
            fields
                .iter()
                .map(|&(_, o)| crate::bin::u32_at(c.record(r), o as usize))
                .collect()
        })
        .collect();
    (records, cb.buffers[&buffer].clone())
}

/// Cell to field: missing column, empty / nothing / failed → 0xFFFFFFFF
/// with nothing appended; else the buffer length before the append. The
/// buffer is the plain concatenation, in record order and `.txt` column
/// order (header `CALC2` before `calc1`, bound case-insensitively), with
/// no sharing of equal expressions; 0 is a valid offset.
#[test]
fn cells_to_fields() {
    let (fields, buf) = table_compile(
        CalcBuffer::ItemsCode,
        &[("calc1", 0), ("calc2", 4), ("calc3", 8)],
        12,
        b"CALC2\tcalc1\r\n5\t7\r\n\t5\r\n  \t1?2\r\n300\tlvl*\r\n",
    );
    const N: u32 = u32::MAX;
    assert_eq!(
        fields,
        [vec![3, 0, N], vec![6, N, N], vec![N, N, N], vec![N, 9, N]]
    );
    assert_eq!(buf, hex("07 05 00 07 07 00 07 05 00 08 2C 01 00"));
}

/// The callback text is the first min(L, 256) bytes of the cell.
#[test]
fn cell_text_cut_at_256() {
    let mut cell = vec![b' '; 255];
    cell.extend_from_slice(b"7");
    cell.extend_from_slice(&[b'8'; 44]);
    let mut text = b"calc1\r\n".to_vec();
    text.extend_from_slice(&cell);
    text.extend_from_slice(b"\r\n");
    let (fields, buf) = table_compile(CalcBuffer::ItemsCode, &[("calc1", 0)], 4, &text);
    assert_eq!(fields, [vec![0]]);
    assert_eq!(buf, hex("07 07 00"));
    // Unlimited text through `compile` reads the whole number.
    let all = compile(Family::Items, &Links, &cell).unwrap().code;
    assert_ne!(all, hex("07 07 00"));
}

/// Opcode table: what the compiler emits for each opcode, operand sizes
/// and extension, and the opcode set the validator accepts.
#[test]
fn opcode_table() {
    let s = |t, e| check(Family::Skills, t, e);
    s("min(lvl,1)", "04 10 07 01 01 00 00");
    s("lvl*(2", "04 10 07 02 02 12 00");
    s("lvl", "04 10 00");
    check_big("big1", "05 C8 00 00");
    check_big("big2", "06 40 9C 00 00 00");
    s("lvl+1", "04 10 07 01 10 00");
    s("lvl+300", "04 10 08 2C 01 10 00");
    s("lvl+100000", "04 10 09 A0 86 01 00 10 00");
    for (op, code) in [
        ("<", "0A"),
        (">", "0B"),
        ("<=", "0C"),
        (">=", "0D"),
        ("==", "0E"),
        ("!=", "0F"),
        ("+", "10"),
        ("-", "11"),
        ("*", "12"),
        ("/", "13"),
        ("^", "14"),
    ] {
        check(
            Family::Skills,
            &format!("lvl{op}ln12"),
            &format!("04 10 04 00 {code} 00"),
        );
    }
    s("-lvl", "04 10 15 00");
    s("lvl?1:2", "04 10 07 01 07 02 16 00");
    // Operand extension: INT8/INT16 sign-extended.
    let e = |s: &str| eval_const(&hex(s));
    assert_eq!(e("07 FF 00"), -1);
    assert_eq!(e("08 FF FF 00"), -1);
    assert_eq!(e("09 FE FF FF FF 00"), -2);
    // The validator accepts exactly the opcodes the compiler can emit.
    for op in 0u8..=0xFF {
        let mut buf = vec![op];
        buf.extend(std::iter::repeat_n(0x01, operand_len(op)));
        if op != 0 {
            buf.push(0);
        }
        let ok = validate_buffer(Family::Skills, &buf, []).is_ok();
        assert_eq!(
            ok,
            matches!(op, 0x00 | 0x01 | 0x02 | 0x04..=0x16),
            "{op:#04x}"
        );
    }
}

/// Expression boundaries: the first 0x00 opcode; operand bytes skipped.
#[test]
fn expression_boundaries() {
    let buf = hex(
        "07 00 00 04 00 00 08 00 00 00 09 00 00 00 00 00 01 00 00 05 00 00 00 06 00 00 00 00 00",
    );
    let r = validate_buffer(Family::Skills, &buf, []).unwrap();
    assert_eq!(r.starts, [0, 3, 6, 10, 16, 19, 23]);
    // 0x02 is no delimiter for layout, but stops evaluation.
    let one = hex("07 05 02 07 06 10 00");
    assert_eq!(
        validate_buffer(Family::Skills, &one, [0]).unwrap().starts,
        [0]
    );
    assert_eq!(eval_const(&one), 5);
}

/// Keywords, function tables, arities and missile `miss`.
#[test]
fn functions_and_keywords() {
    use Keyword::*;
    assert_eq!(
        Family::Skills.keywords(),
        [Min, Max, Rand, Skill, Miss, Stat, Sklvl]
    );
    assert_eq!(Family::Missiles.keywords(), [Min, Max, Rand, Skill, Miss]);
    assert_eq!(Family::Items.keywords(), [Min, Max, Rand, Stat]);
    assert_eq!(
        [Min, Max, Rand, Skill, Miss, Stat, Sklvl].map(Keyword::text),
        ["min", "max", "rand", "skill", "miss", "stat", "sklvl"]
    );
    let counts = [Family::Skills, Family::Missiles, Family::Items].map(Family::function_count);
    assert_eq!(counts, [7, 4, 4]);
    for i in 0..7 {
        assert_eq!(Family::Skills.arity(i), if i == 6 { 3 } else { 2 });
    }
    for i in 0..5 {
        assert_eq!(Family::Missiles.arity(i), 2);
        assert_eq!(Family::Items.arity(i), 2);
    }
    let s = |t, e| check(Family::Skills, t, e);
    s("RaNd(1,6)", "07 01 07 06 01 02 00");
    s("SKLVL(1,2,3)", "07 01 07 02 07 03 01 06 00");
    s("sklvl(1,2)", "fail");
    s("Stat(1,2)", "07 01 07 02 01 05 00");
    s("MISS(1,2)", "07 01 07 02 01 04 00");
    let m = |t, e| check(Family::Missiles, t, e);
    // Missile `miss` compiles to CALL 4 (no table entry).
    m("Miss(1,2)", "07 01 07 02 01 04 00");
    m("Skill(1,2)", "07 01 07 02 01 03 00");
    m("stat(lvl,2)", "07 00 04 12 07 02 00");
    let i = |t, e| check(Family::Items, t, e);
    i("STAT(1,2)", "07 01 07 02 01 03 00");
    i("miss(1,2)", "04 00 07 01 07 02 00");
    i("sklvl(1,2,3)", "04 00 07 01 07 02 07 03 00");
    // CALL 4 has an evaluator entry only in skills.
    let call4 = hex("07 01 07 02 01 04 00");
    let unknown = |f| validate_buffer(f, &call4, [0]).unwrap().unknown_call;
    assert_eq!(
        [Family::Skills, Family::Missiles, Family::Items].map(unknown),
        [0, 1, 1]
    );
    // A CALL without an entry pushes 0 and pops nothing.
    assert_eq!(eval_const(&hex("07 01 07 02 01 04 10 00")), 2);
}

/// Tokens: skipped bytes, each token kind, stop.
#[test]
fn tokens() {
    let s = |t, e| check(Family::Skills, t, e);
    // Spaces 0x09–0x0D, 0x20 and `"` are skipped.
    s("\t\n\x0b\x0c\r 7", "07 07 00");
    s("\"7\"", "07 07 00");
    s("\"min(24,ln12)\"", "07 18 04 00 01 00 00");
    // Numbers: decimal, wrapping, no sign or hex.
    s("007", "07 07 00");
    s("2147483648", "09 00 00 00 80 00");
    s("4294967297", "07 01 00");
    s("5x", "07 00 00");
    s("+5", "fail");
    // Quoted names: up to the next `'` or the end; `'` consumed.
    s("'lvl", "04 10 00");
    s("'lvl'5", "04 10 07 05 00");
    s("'lvl'+1", "04 10 07 01 10 00");
    s("'l v l'", "07 00 00");
    // Words: alnum run (`_` is not alnum); a non-keyword before `(` is a
    // name and the `(` stays.
    s("lvl_", "04 10 00");
    assert_eq!(diags(Family::Skills, "lvl_"), [CalcDiag::Stop]);
    s("lvl2", "07 00 00");
    s("lvl (2)", "04 10 07 02 00");
    s("foo(1)", "07 01 00");
    // Suffix: index ≥ 0 → constant, whatever the kind; −1 → stop.
    s("lvl+.lvl", "04 10 07 10 10 00");
    s(".", "fail");
    s(".x", "fail");
    assert_eq!(
        diags(Family::Skills, ".x"),
        [CalcDiag::Stop, CalcDiag::Fail]
    );
    s("lvl.foo", "04 10 00");
    // Symbols and two-byte comparisons; lone `=` / `!` and others stop.
    s("lvl<1", "04 10 07 01 0A 00");
    s("lvl>1", "04 10 07 01 0B 00");
    s("lvl<=1", "04 10 07 01 0C 00");
    s("lvl>=1", "04 10 07 01 0D 00");
    s("lvl==1", "04 10 07 01 0E 00");
    s("lvl!=1", "04 10 07 01 0F 00");
    for t in ["1=2", "1!2", "1 # 2", "1;2", "1&2", "1[2"] {
        s(t, "07 01 00");
    }
    // A stop ignores the rest; compilation ends normally.
    s("2+3 # ) ) (", "07 05 00");
    assert_eq!(diags(Family::Skills, "2+3 # ) ) ("), [CalcDiag::Stop]);
    // End of text is no stop diagnostic.
    assert!(diags(Family::Skills, "2+3").is_empty());
    // Resolution: constant kind → INT, parameter kind → PARAM, −1 → 0.
    s("skill('Fire Bolt',lvl)", "07 24 04 10 01 03 00");
    s("nope", "07 00 00");
    assert_eq!(diags(Family::Skills, "nope"), [CalcDiag::UnknownName]);
}

/// Name resolution per family and context (top pending entry only).
#[test]
fn name_resolution() {
    let s = |t, e| check(Family::Skills, t, e);
    // skill( / sklvl(: skills name lookup, else skillcalc code.
    s("skill('Fire Bolt',1)", "07 24 07 01 01 03 00");
    s("skill('FIRE BOLT',1)", "07 24 07 01 01 03 00");
    s("skill(lvl,1)", "04 10 07 01 01 03 00");
    s("sklvl(lvl,1,2)", "04 10 07 01 07 02 01 06 00");
    // Only the top entry counts.
    s("skill(('Fire Bolt'),1)", "07 00 07 01 01 03 00");
    s("skill(1+'Fire Bolt',2)", "07 01 07 00 10 07 02 01 03 00");
    // miss(: missiles name lookup, else misscalc code.
    s("miss('FireBolt',1)", "07 3A 07 01 01 04 00");
    s("miss(dl12,1)", "04 29 07 01 01 04 00");
    s("dl12", "07 00 00");
    // stat(: itemstatcost name lookup, else stat mode (whole name).
    s("stat('Strength',1)", "07 00 07 01 01 05 00");
    s("stat(base,1)", "04 01 07 01 01 05 00");
    s("stat(BASE,1)", "04 01 07 01 01 05 00");
    s("stat(Mod,1)", "04 02 07 01 01 05 00");
    s("stat(accr,1)", "04 00 07 01 01 05 00");
    s("stat(based,1)", "04 00 07 01 01 05 00");
    // none, min(, max(, rand(: skillcalc code.
    s("min(lvl,1)", "04 10 07 01 01 00 00");
    s("max('Fire Bolt',1)", "07 00 07 01 01 01 00");
    s("rand('Fire Bolt',9)", "07 00 07 09 01 02 00");
    // Name key: first 31 bytes, lowercased.
    check_big(
        "skill('Abcdefghijklmnopqrstuvwxyzabcdefghijklmn',1)",
        "07 1F 07 01 01 03 00",
    );
    // Code: first 4 bytes, space-padded, case-sensitive.
    s("ln123", "04 00 00");
    s("LVL", "07 00 00");
    s("min(lv,1)", "07 00 07 01 01 00 00");
    let m = |t, e| check(Family::Missiles, t, e);
    m("skill('Fire Bolt',1)", "07 24 07 01 01 03 00");
    m("skill(lvl,1)", "04 10 07 01 01 03 00");
    m("miss('firebolt',1)", "07 3A 07 01 01 04 00");
    m("miss(lvl,1)", "04 12 07 01 01 04 00");
    m("lvl", "04 12 00");
    m("min(lvl,1)", "04 12 07 01 01 00 00");
    m("min('Fire Bolt',1)", "07 00 07 01 01 00 00");
    let i = |t, e| check(Family::Items, t, e);
    i("stat('strength',1)", "07 00 07 01 01 03 00");
    i("stat(mod,1)", "04 02 07 01 01 03 00");
    i("stat(xyz,1)", "04 00 07 01 01 03 00");
    i("min('strength',1)", "04 00 07 01 01 00 00");
    i("'Fire Bolt'", "04 00 00");
    i("ln12", "04 00 00");
}

/// Parser state: strengths, arities, Emit, push value, folding flag, the
/// op-stack limit and Operator(N).
#[test]
fn parser_state() {
    let s = |t, e| check(Family::Skills, t, e);
    // Strengths: pop while strength ≥ N, so equal strengths pop (left-associative).
    s("lvl<ln12+par1", "04 10 04 00 04 08 10 0A 00");
    s("lvl+ln12<par1", "04 10 04 00 10 04 08 0A 00");
    s("lvl+ln12*par1", "04 10 04 00 04 08 12 10 00");
    s("lvl*ln12^par1", "04 10 04 00 04 08 14 12 00");
    s("lvl^ln12*par1", "04 10 04 00 14 04 08 12 00");
    s("-lvl^ln12", "04 10 15 04 00 14 00");
    s("lvl^-ln12", "04 10 04 00 15 14 00");
    s("lvl-ln12-par1", "04 10 04 00 11 04 08 11 00");
    s("lvl+ln12?par1:par2", "04 10 04 00 04 08 04 09 16 10 00");
    s("lvl?ln12:par1+par2", "04 10 04 00 04 08 16 04 09 10 00");
    // A function is never popped by an operator; `,` pops above it.
    s("min(lvl+1,2)", "04 10 07 01 10 07 02 01 00 00");
    // `,` (3) does not pop `(` (2) and pushes nothing.
    s("(1,2)", "07 02 00");
    // Emit fails when `count` < arity.
    for t in ["lvl*", "*lvl", "-", "sklvl(1,2)"] {
        s(t, "fail");
    }
    // Push value widths, constants and parameters.
    s("127", "07 7F 00");
    s("128", "08 80 00 00");
    s("0-128", "07 80 00");
    s("0-129", "08 7F FF 00");
    s("32767", "08 FF 7F 00");
    s("32768", "09 00 80 00 00 00");
    s("0-32768", "08 00 80 00");
    s("0-32769", "09 FF 7F FF FF 00");
    check_big("big1", "05 C8 00 00");
    check_big("big2", "06 40 9C 00 00 00");
    // `called`: set by parameters and functions only.
    s("1+.lvl", "07 11 00");
    s("lvl-lvl", "04 10 04 10 11 00");
    s("min(1,2)", "07 01 07 02 01 00 00");
    // At most 64 pending entries.
    check(Family::Skills, &format!("{}1", "(".repeat(64)), "07 01 00");
    check(Family::Skills, &format!("{}1", "(".repeat(65)), "fail");
    check(
        Family::Skills,
        &format!("{}1+2", "(".repeat(63)),
        "07 03 00",
    );
    check(Family::Skills, &format!("{}1+2", "(".repeat(64)), "fail");
    check(
        Family::Skills,
        &format!("{}1", "min(".repeat(64)),
        "07 01 00",
    );
    check(Family::Skills, &format!("{}1", "min(".repeat(65)), "fail");
    // End with empty `out` fails; else 0x00 is appended.
    for t in ["", "()", ","] {
        s(t, "fail");
    }
}

/// Token actions, close, end and the `out` limits.
#[test]
fn token_actions() {
    let s = |t, e| check(Family::Skills, t, e);
    // `pending`: `(` / `)` / `,` / `?` / `:` leave it; operators clear it.
    s("lvl (-1)", "04 10 07 01 11 00");
    s("(lvl)-1", "04 10 07 01 11 00");
    s("()-1", "07 FF 00");
    s("lvl+-1", "04 10 07 01 15 10 00");
    s("lvl<-1", "04 10 07 01 15 0A 00");
    s("lvl*-1", "04 10 07 01 15 12 00");
    s("1:-2", "07 FF 00");
    for t in ["min(-1,2)", "1?-2:3"] {
        s(t, "fail");
    }
    // Close: empty ops fails; `(` is popped; a function is emitted with its index;
    // an operator is emitted, failing if ops is then empty.
    s("1)", "fail");
    s("1+2)", "fail");
    s("(1+2)*3", "07 09 00");
    s("sklvl(1,2,3)", "07 01 07 02 07 03 01 06 00");
    s("max(lvl+1,2*3)", "04 10 07 01 10 07 02 07 03 12 01 01 00");
    // End: entries above the topmost function are emitted (an open
    // `(` as 0x02); the function and anything below it are dropped.
    s("min(1,2+lvl", "07 01 07 02 04 10 10 00");
    s("lvl+min(1,2", "04 10 07 01 07 02 00");
    s("lvl*(2", "04 10 07 02 02 12 00");
    s("(lvl", "04 10 02 00");
    // End: no name, parameter or function → one constant.
    s("2*(3+4)", "07 0E 00");
    s("foo+1", "07 01 00");
    s("(1+2", "07 03 00");
    // End: otherwise the result is `out` as built.
    s("lvl+0", "04 10 07 00 10 00");
    // Fail: empty code and a Fail diagnostic.
    let c = compile(Family::Skills, &Links, b"1+").unwrap();
    assert!(c.code.is_empty());
    assert_eq!(c.diagnostics, [CalcDiag::Fail]);
    // Limits: `out` at most 1,024 bytes (only reachable without the
    // 256-byte cell cut).
    let len = |t: String| {
        compile(Family::Skills, &Links, t.as_bytes())
            .unwrap()
            .code
            .len()
    };
    assert_eq!(len("lvl ".repeat(511)), 1023);
    assert_eq!(len("lvl ".repeat(512)), 0);
    assert_eq!(len(format!("300 {}", "lvl ".repeat(510))), 1024);
    assert_eq!(len(format!("{}300", "lvl ".repeat(510))), 1024);
    assert_eq!(len(format!("300 {}300", "lvl ".repeat(509))), 0);
    assert_eq!(len(format!("{}100000", "lvl ".repeat(509))), 1024);
    assert_eq!(len(format!("300 {}100000", "lvl ".repeat(508))), 0);
}

/// Compiler quirks that are kept on purpose, and the cases D2RUST rejects.
#[test]
fn kept_quirks_and_rejected_cases() {
    let e = |s: &str| eval_const(&hex(s));
    // Evaluation continues after COND.
    assert_eq!(e("07 01 07 05 07 06 16 07 02 10 00"), 7);
    // Missile miss() → CALL 4.
    check(
        Family::Missiles,
        "miss('firebolt'.dl12)",
        "07 3A 07 29 01 04 00",
    );
    // Stops, open parens, argument counts.
    check(Family::Skills, "5 $ 3", "07 05 00");
    check(Family::Skills, "lvl*(2", "04 10 07 02 02 12 00");
    check(Family::Skills, "min(1,2,3)", "07 01 07 02 07 03 01 00 00");
    check(Family::Skills, "min(1)", "fail");
    // Name contexts and 4-byte codes.
    check(
        Family::Skills,
        "skill(('Fire Bolt').lvl)",
        "07 00 07 10 01 03 00",
    );
    check(Family::Skills, "par34", "04 0A 00");
    // −2³¹ / −1 = −2³¹, at run time and when folding.
    assert_eq!(e("09 00 00 00 80 07 FF 13 00"), i32::MIN);
    check(Family::Skills, "(0-2147483648)/(0-1)", "09 00 00 00 80 00");
    // A byte ≥ 0x80 is an error, even after a stop.
    assert_eq!(
        compile(Family::Skills, &Links, b"5 $ \x80").unwrap_err(),
        CalcError::NonAscii
    );
    // A missile formula with CALL 2 is rejected.
    assert_eq!(
        compile(Family::Missiles, &Links, b"rand(1,2)").unwrap_err(),
        CalcError::MissileRand
    );
}

/// Per-cell diagnostics, and the non-ASCII error.
#[test]
fn strictness_reports() {
    let d = |t| diags(Family::Skills, t);
    assert_eq!(d("1?2"), [CalcDiag::Fail]);
    assert_eq!(d(" "), [CalcDiag::Fail]);
    assert_eq!(d("5 $ 3"), [CalcDiag::Stop]);
    assert_eq!(d("'nope'"), [CalcDiag::UnknownName]);
    assert_eq!(d("nope"), [CalcDiag::UnknownName]);
    assert_eq!(d("lvl*(2"), [CalcDiag::OpenParen]);
    assert_eq!(d("max(1,2"), [CalcDiag::OpenFunction]);
    for t in ["lvl", "2+3", "min(1,2)", "skill('Fire Bolt'.lvl)"] {
        assert!(d(t).is_empty(), "{t}");
    }
    // The code is still emitted as before; only a diagnostic is added.
    check(Family::Skills, "max(1,2", "07 01 07 02 00");
    assert_eq!(
        compile(Family::Items, &Links, b"\xFF").unwrap_err(),
        CalcError::NonAscii
    );
}

/// A missile formula calling `rand` is an error in the compiler
/// and the validator; other families accept it.
#[test]
fn missile_rand_refused() {
    for t in ["rand(1,2)", "min(rand(1,2),3)", "RAND(lvl,5)"] {
        assert_eq!(
            compile(Family::Missiles, &Links, t.as_bytes()).unwrap_err(),
            CalcError::MissileRand,
            "{t}"
        );
    }
    let buf = hex("07 00 00 07 01 07 03 01 02 00");
    assert_eq!(
        validate_buffer(Family::Missiles, &buf, [0, 3]),
        Err(BufferError::MissileRand { offset: 7 })
    );
    assert!(validate_buffer(Family::Skills, &buf, [0, 3]).is_ok());
    assert!(validate_buffer(Family::Items, &buf, [0, 3]).is_ok());
    // An operand byte 2 after CALL 1 is no rand call.
    assert!(validate_buffer(Family::Missiles, &hex("07 02 07 02 01 01 00"), [0]).is_ok());
}

/// Checks against the player's own game files. Run with:
/// `D2_GAME_DIR=<install> cargo test -p d2-data calc::tests::game -- --ignored`
mod game {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use d2_formats::mpq::{archive_file_name, ArchiveSet};

    use super::super::*;
    use crate::bin::{
        self, buffer_tables, excel_path, formula_fields, read_excel, BinSet, DEFAULT_LANGUAGE,
    };
    use crate::compile_set::{compile_all, CompiledSet};
    use crate::schema::{schema, FieldDef, Link};
    use crate::strings::StringTables;
    use crate::txt::{bind, TxtTable};

    fn set() -> &'static ArchiveSet {
        static SET: OnceLock<ArchiveSet> = OnceLock::new();
        SET.get_or_init(|| {
            let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
            ArchiveSet::open_dir(dir).expect("archives open")
        })
    }

    fn bins() -> &'static BinSet {
        static B: OnceLock<BinSet> = OnceLock::new();
        B.get_or_init(|| bin::load(set(), DEFAULT_LANGUAGE).expect("live .bin set loads"))
    }

    fn compiled() -> &'static CompiledSet {
        static C: OnceLock<CompiledSet> = OnceLock::new();
        C.get_or_init(|| {
            let strings = StringTables::load(set(), "eng", true).expect("string tables");
            let mut read = |f: &str| read_excel(set(), f).map_err(|e| e.to_string());
            compile_all(&mut read, &strings).expect("text set compiles")
        })
    }

    fn calc_fields(table: &str, buffer: CalcBuffer) -> Vec<&'static FieldDef> {
        schema()
            .table(table)
            .unwrap()
            .fields
            .iter()
            .filter(|f| f.link == Link::Calc(buffer))
            .collect()
    }

    fn starts(buffer: &[u8]) -> Vec<usize> {
        let mut out = vec![0];
        let mut i = 0;
        while i < buffer.len() {
            let op = buffer[i];
            i += 1 + operand_len(op);
            if op == 0 && i < buffer.len() {
                out.push(i);
            }
        }
        out
    }

    /// Families: code file sizes and expression counts, calc columns per
    /// table, record files without a trailing section, and no code files
    /// or calc tables in `d2exp.mpq` / `d2data.mpq`.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn families() {
        let data = bins();
        for (buffer, size, exprs) in [
            (CalcBuffer::MissCode, 196, 25),
            (CalcBuffer::SkillsCode, 5_891, 853),
            (CalcBuffer::SkillDescCode, 4_252, 867),
            (CalcBuffer::ItemsCode, 158, 45),
        ] {
            assert_eq!(data.code[&buffer].bytes.len(), size, "{buffer:?}");
            assert_eq!(data.code_reports[&buffer].starts.len(), exprs, "{buffer:?}");
        }
        for (table, buffer, columns) in [
            ("missiles", CalcBuffer::MissCode, 7),
            ("skills", CalcBuffer::SkillsCode, 36),
            ("skilldesc", CalcBuffer::SkillDescCode, 42),
            ("weapons", CalcBuffer::ItemsCode, 5),
            ("armor", CalcBuffer::ItemsCode, 5),
            ("misc", CalcBuffer::ItemsCode, 5),
        ] {
            assert_eq!(calc_fields(table, buffer).len(), columns, "{table}");
        }
        assert_eq!(
            buffer_tables(CalcBuffer::ItemsCode),
            ["weapons", "armor", "misc"]
        );
        for (file, len, count, size) in [
            ("skills.bin", 204_208, 357, 572),
            ("skilldesc.bin", 63_652, 221, 288),
            ("missiles.bin", 287_284, 684, 420),
            ("misc.bin", 64_028, 151, 424),
        ] {
            let (_, bytes) = read_excel(set(), file).unwrap().unwrap();
            assert_eq!(bytes.len(), len, "{file}");
            assert_eq!(len, 4 + count * size, "{file}");
        }
        for archive in ["d2exp.mpq", "d2data.mpq"] {
            let a = set()
                .archives()
                .iter()
                .find(|a| archive_file_name(a).eq_ignore_ascii_case(archive))
                .unwrap();
            for file in [
                "misscode.bin",
                "skillscode.bin",
                "skilldesccode.bin",
                "itemscode.bin",
                "skillcalc.txt",
                "skillcalc.bin",
                "misscalc.txt",
                "misscalc.bin",
            ] {
                assert!(!a.contains(&excel_path(file)), "{archive} {file}");
            }
        }
    }

    /// Non-empty formula cells per table; weapons and armor have no calc
    /// columns, so all their formula fields are 0xFFFFFFFF.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn calc_cells() {
        for (table, buffer, cells, spaces) in [
            ("missiles", CalcBuffer::MissCode, 25, 0),
            ("skills", CalcBuffer::SkillsCode, 853, 0),
            ("skilldesc", CalcBuffer::SkillDescCode, 868, 1),
            ("weapons", CalcBuffer::ItemsCode, 0, 0),
            ("armor", CalcBuffer::ItemsCode, 0, 0),
            ("misc", CalcBuffer::ItemsCode, 45, 0),
        ] {
            let def = schema().table(table).unwrap();
            let (_, bytes) = read_excel(set(), &def.txt_name).unwrap().unwrap();
            let txt = TxtTable::parse(&def.txt_name, &bytes).unwrap();
            let fields = calc_fields(table, buffer);
            let names: Vec<&Vec<u8>> = fields.iter().map(|f| &f.column).collect();
            let binding = bind(&txt.header, &names);
            let bound: Vec<usize> = binding.field_column.iter().flatten().copied().collect();
            if cells == 0 {
                assert!(bound.is_empty(), "{table} has calc columns");
                let c = &compiled().table(table).unwrap().compiled;
                for r in 0..c.count {
                    for f in &fields {
                        assert_eq!(bin::u32_at(c.record(r), f.offset as usize), u32::MAX);
                    }
                }
                continue;
            }
            assert_eq!(bound.len(), fields.len(), "{table}");
            let cell = |rec: &crate::txt::TxtRecord, c: usize| rec.cells.get(c).cloned();
            let non_empty: Vec<Vec<u8>> = txt
                .records
                .iter()
                .flat_map(|rec| bound.iter().filter_map(move |&c| cell(rec, c)))
                .filter(|t| !t.is_empty())
                .collect();
            assert_eq!(non_empty.len(), cells, "{table}");
            let blank = non_empty.iter().filter(|t| t.as_slice() == b" ").count();
            assert_eq!(blank, spaces, "{table}");
        }
    }

    /// The text compile reproduces the four code files and every formula
    /// field; the sorted field values are exactly the expression starts,
    /// each referenced once.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn text_compile_reproduces_code_files() {
        let (data, text) = (bins(), compiled());
        let mut total = 0;
        for buffer in CalcBuffer::ALL {
            let shipped = &data.code[&buffer].bytes;
            assert_eq!(&text.buffers[&buffer], shipped, "{buffer:?}");
            let mut values = Vec::new();
            for table in buffer_tables(buffer) {
                let (b, c) = (
                    data.table(table).unwrap(),
                    &text.table(table).unwrap().compiled,
                );
                assert_eq!(b.count, c.count, "{table}");
                // Record-major, as `bin::formula_fields` collects them.
                let fields = calc_fields(table, buffer);
                for r in 0..b.count {
                    for f in &fields {
                        let o = f.offset as usize;
                        let v = bin::u32_at(b.record(r), o);
                        assert_eq!(bin::u32_at(c.record(r), o), v, "{table} {r} {}", f.name());
                        values.push(v);
                    }
                }
            }
            total += values.len();
            assert_eq!(values, formula_fields(buffer, &data.tables), "{buffer:?}");
            let mut set: Vec<usize> = values
                .into_iter()
                .filter(|&v| v != u32::MAX)
                .map(|v| v as usize)
                .collect();
            set.sort_unstable();
            assert_eq!(set, starts(shipped), "{buffer:?}");
        }
        assert_eq!(total, 30_217);
    }

    /// Opcode occurrences over the 1,790 expressions of the shipped tables.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn opcode_counts() {
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for file in bins().code.values() {
            let b = &file.bytes;
            let mut i = 0;
            while i < b.len() {
                *counts.entry(b[i]).or_default() += 1;
                i += 1 + operand_len(b[i]);
            }
        }
        let want: BTreeMap<u8, usize> = [
            (0x00, 1_790),
            (0x01, 531),
            (0x02, 1),
            (0x04, 1_606),
            (0x07, 1_403),
            (0x08, 234),
            (0x0A, 14),
            (0x0B, 1),
            (0x10, 321),
            (0x11, 79),
            (0x12, 363),
            (0x13, 108),
            (0x15, 23),
            (0x16, 15),
        ]
        .into_iter()
        .collect();
        assert_eq!(counts, want);
    }

    /// The loader keeps the code files as raw bytes, validates
    /// them with their fields, and every field is none or an offset.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn loader_validates_code_files() {
        let data = bins();
        assert_eq!(data.code.len(), 4);
        for buffer in CalcBuffer::ALL {
            let file = &data.code[&buffer];
            let (_, raw) = read_excel(set(), &format!("{}.bin", buffer.name()))
                .unwrap()
                .unwrap();
            assert_eq!(file.bytes, raw, "{buffer:?}");
            let report = &data.code_reports[&buffer];
            let again = validate_buffer(
                Family::of(buffer),
                &raw,
                formula_fields(buffer, &data.tables),
            )
            .unwrap();
            assert_eq!(report, &again);
            assert_eq!((report.unreferenced, report.shared), (0, 0), "{buffer:?}");
            for v in formula_fields(buffer, &data.tables) {
                assert!(v == u32::MAX || report.starts.contains(&(v as usize)));
            }
        }
    }

    /// Diagnostic counts on the shipped tables: one OpenParen (skills) and one Fail (skilldesc).
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn strictness_counts() {
        let want: BTreeMap<(CalcBuffer, CalcDiag), usize> = [
            ((CalcBuffer::SkillsCode, CalcDiag::OpenParen), 1),
            ((CalcBuffer::SkillDescCode, CalcDiag::Fail), 1),
        ]
        .into_iter()
        .collect();
        assert_eq!(compiled().calc_diagnostics, want);
    }
}
