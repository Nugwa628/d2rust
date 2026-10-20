//! Mutation-testing kills for `patch` and `patch/*`: tests that fail on mutants
//! `cargo mutants` reported as missed.
//
// Synthetic data only: the `items`/`gear`/`recipes` patch fixture
// (reimplemented here; the one in `patch::tests` is private),
// and an invented all-tables base for the compile checks.

use crate::bin::{BinSet, BinTable};
use crate::patch::*;
use crate::strings::StringTables;
use crate::txt::TxtTable;

fn v(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

// ------------------------------------------------------------- fixture

#[allow(clippy::too_many_arguments)]
fn rules_of(
    name: &str,
    key: Option<&str>,
    kind: KeyKind,
    unique: bool,
    fixed: bool,
    scope: &str,
    fields: &[&str],
) -> TableRules {
    TableRules {
        name: name.into(),
        txt_name: format!("{name}.txt"),
        key_field: key.map(v),
        kind,
        unique,
        fixed,
        scope: scope.into(),
        lists: vec![fields.iter().map(|f| v(f)).collect()],
    }
}

fn table(r: TableRules, header: &[&str], rows: &[&str]) -> PatchTable {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let cells: Vec<Vec<u8>> = line.split(';').map(v).collect();
            Row {
                origin: Origin::Base(i + 2),
                writers: vec![Writer::Base; cells.len()],
                cells,
            }
        })
        .collect();
    PatchTable::new(&r, "fixture", header.iter().map(|h| v(h)).collect(), rows).unwrap()
}

/// The patch fixture: `items`, `gear` (same scope, fixed), `recipes`.
fn fixture() -> PatchData {
    let items = table(
        rules_of(
            "items",
            Some("code"),
            KeyKind::Code,
            true,
            false,
            "items.code",
            &["name", "code", "lvl", "dam"],
        ),
        &["name", "code", "lvl", "dam", "dam", "*note"],
        &[
            "Axe;axe;1;3;0;",
            "Club;clb;1;2;0;old",
            "Axe;ax2;5;7;0;",
            "Big Club;clb;9;8;1;",
        ],
    );
    let gear = table(
        rules_of(
            "gear",
            Some("code"),
            KeyKind::Code,
            true,
            true,
            "items.code",
            &["name", "code"],
        ),
        &["name", "code"],
        &["Cap;cap", "Belt;blt"],
    );
    let recipes = table(
        rules_of(
            "recipes",
            None,
            KeyKind::Exact,
            false,
            false,
            "recipes",
            &["description", "enabled", "output"],
        ),
        &["description", "enabled", "output"],
        &["A;1;\"hp1,qty=3\"", "B;1;x", "C;0;y"],
    );
    PatchData::from_tables(vec![items, gear, recipes])
}

fn layer_of(path: &str, body: &str, pos: usize) -> Layer {
    let body = body.replace(" / ", "\n");
    let text = if body.starts_with("table ") {
        format!("d2patch 1\n{body}\n")
    } else {
        format!("d2patch 1\ntable items\n{body}\n")
    };
    let (l, f) = parse_layer(path, text.as_bytes(), pos);
    assert!(f.is_empty(), "{path}: {f:?}");
    l
}

/// One layer `a.d2patch` (statements after `table items` unless the body
/// starts with `table`; `/` separates lines).
fn one(body: &str) -> (PatchData, Vec<Finding>) {
    let mut d = fixture();
    let f = apply_stack(&mut d, &[layer_of("a.d2patch", body, 1)], "s.d2stack");
    (d, f)
}

fn codes(f: &[Finding]) -> Vec<String> {
    f.iter()
        .map(|f| format!("{:?} {}:{}:{}", f.code, f.file, f.line, f.col))
        .collect()
}

fn expect(body: &str, want: &[&str]) {
    let (_, f) = one(body);
    assert_eq!(codes(&f), want, "{body}");
}

// ----------------------------------------------------------- report

fn at(code: Code, file: &str, line: usize, col: usize) -> Finding {
    Finding::new(code, file, line, col)
}

/// Order: S; P; B; A and N; C.
#[test]
fn report_orders_classes() {
    let mut b = at(Code::B01, "data\\global\\excel\\items.txt", 0, 0);
    b.table = Some("items".into());
    let mut a = at(Code::A06, "a.d2patch", 3, 13);
    a.table = Some("items".into());
    let mut c = at(Code::C02, "base:x:items.txt", 2, 3);
    c.table = Some("items".into());
    let want = [
        at(Code::S05, "s.d2stack", 9, 7),
        at(Code::P05, "s.d2stack", 1, 1),
        b,
        a,
        c,
    ];
    // Every reversed or rotated input sorts to the same order.
    for k in 0..want.len() {
        let mut f = want.to_vec();
        f.rotate_left(k);
        f.reverse();
        sort_report(&mut f);
        let got: Vec<Code> = f.iter().map(|f| f.code).collect();
        assert_eq!(
            got,
            [Code::S05, Code::P05, Code::B01, Code::A06, Code::C02],
            "rotation {k}"
        );
    }
}

/// P findings by stack position, line, column.
#[test]
fn report_orders_p_by_line_and_column() {
    let mut f = vec![
        at(Code::P04, "a.d2patch", 9, 1),
        at(Code::P05, "a.d2patch", 3, 20),
        at(Code::P10, "a.d2patch", 3, 12),
    ];
    sort_report(&mut f);
    assert_eq!(
        codes(&f),
        [
            "P10 a.d2patch:3:12",
            "P05 a.d2patch:3:20",
            "P04 a.d2patch:9:1"
        ]
    );
}

/// B findings by table (never by code).
#[test]
fn report_orders_b_by_table() {
    let rules: Vec<TableRules> = ["zeta", "alpha", "mid"]
        .iter()
        .map(|n| rules_of(n, None, KeyKind::Exact, false, false, n, &["a"]))
        .collect();
    let mut read = |name: &str| -> Result<Option<(String, Vec<u8>)>, String> {
        match name {
            "zeta.txt" => Ok(None),                             // B01
            "alpha.txt" => Err("broken".to_owned()),            // B02
            _ => Ok(Some(("x.mpq".into(), b"a\r\n".to_vec()))), // B02 (E2)
        }
    };
    let errs = PatchData::from_base(&rules, &mut read).unwrap_err();
    let got: Vec<(Code, String)> = errs
        .iter()
        .map(|f| (f.code, f.table.clone().unwrap()))
        .collect();
    assert_eq!(
        got,
        [
            (Code::B02, "alpha".to_owned()),
            (Code::B02, "mid".to_owned()),
            (Code::B01, "zeta".to_owned())
        ]
    );
}

/// human form: `<file>:<line>:<col>: <table> …: <detail>`.
#[test]
fn human_form_colon_after_table_only() {
    let (_, f) = one("table gear / add #2 x");
    assert_eq!(codes(&f), ["A10 a.d2patch:3:1"]);
    assert!(f[0].row.is_none() && f[0].column.is_none());
    let text = f[0].to_string();
    assert!(
        text.starts_with("error[A10] a.d2patch:3:1: gear: "),
        "{text}"
    );
}

/// 85 patchable tables in the game's data set.
#[test]
fn rules_are_the_85_tables() {
    let r = rules();
    assert_eq!(r.len(), 85);
    assert!(r.iter().any(|t| t.name == "weapons"));
}

/// base `data\global\excel\<name>.txt`; the report file name is `base:<archive>:<path>`.
#[test]
fn base_file_names_archive_and_path() {
    assert_eq!(
        base_file("patch_d2.mpq", "weapons.txt"),
        "base:patch_d2.mpq:data\\global\\excel\\weapons.txt"
    );
}

// ------------------------------------------------------- selectors and apply

/// `k` is `[1-9][0-9]*`; `@01` is malformed (A02).
#[test]
fn column_index_with_leading_zero_is_a02() {
    expect("set axe dam@01 3 -> 4", &["A02 a.d2patch:3:9"]);
    expect("set axe dam@1 3 -> 4", &[]);
}

/// / open question 5(f): A02 related are the columns named exactly so.
#[test]
fn a02_related_are_the_same_named_columns() {
    let (_, f) = one("set axe dam 3 -> 4");
    assert_eq!(codes(&f), ["A02 a.d2patch:3:9"]);
    assert_eq!(f[0].related.len(), 2, "{:?}", f[0].related);
    assert!(f[0].related[0].ends_with('3') && f[0].related[1].ends_with('4'));
}

/// `add`: A11 only when the key column is column 0.
#[test]
fn add_expansion_is_a11_only_in_column_0() {
    expect("table recipes / add #3 Expansion", &["A11 a.d2patch:3:8"]);
    // `items` keys column 1: `Expansion` there is a plain key.
    expect("add #4 Expansion", &[]);
}

/// A and N by (stack position, line, column) inside one layer,
/// whatever check produced them.
#[test]
fn a_findings_of_one_layer_sort_by_line() {
    // End-of-layer A12 (line 3) before an A06 on line 4.
    expect(
        "set #2 ax2 code ax2 -> axe / set #1 clb lvl 9 -> 2",
        &["A12 a.d2patch:3:24", "A06 a.d2patch:4:16"],
    );
    // A06 on line 3 before the end-of-layer A12 on line 4.
    expect(
        "set #1 clb lvl 9 -> 2 / set #2 ax2 code ax2 -> axe",
        &["A06 a.d2patch:3:16", "A12 a.d2patch:4:24"],
    );
    // A06 on line 3 before A01 on line 4.
    expect(
        "set axe lvl 9 -> 2 / table nosuch",
        &["A06 a.d2patch:3:13", "A01 a.d2patch:4:7"],
    );
}

/// N04 names the layers not applied (not the failing one).
#[test]
fn n04_names_only_the_layers_not_applied() {
    let mut d = fixture();
    let layers = [
        layer_of("a.d2patch", "set axe lvl 9 -> 2", 1),
        layer_of("b.d2patch", "set axe lvl 1 -> 3", 2),
    ];
    let f = apply_stack(&mut d, &layers, "s.d2stack");
    assert_eq!(codes(&f), ["A06 a.d2patch:3:13", "N04 s.d2stack:0:0"]);
    assert!(f[1].detail.contains("b.d2patch"), "{}", f[1].detail);
    assert!(!f[1].detail.contains("a.d2patch"), "{}", f[1].detail);
}

// ------------------------------------------------------------ syntax

fn parse_codes(text: &[u8]) -> Vec<String> {
    let (_, f) = parse_layer("a.d2patch", text, 1);
    f.iter()
        .map(|f| format!("{:?} {}:{}", f.code, f.line, f.col))
        .collect()
}

fn stack_codes(text: &[u8]) -> Vec<String> {
    let (_, f) = parse_stack("s", text);
    f.iter()
        .map(|f| format!("{:?} {}:{}", f.code, f.line, f.col))
        .collect()
}

/// CR LF = LF on every line, also when the last line is unterminated.
#[test]
fn crlf_lines_before_an_unterminated_last_line() {
    assert!(parse_codes(b"d2patch 1\r\ntable items\r\nset axe lvl 1 -> 2").is_empty());
    assert!(stack_codes(b"d2stack 1\r\nlayer a.d2patch").is_empty());
}

/// a statement line over 4,096 bytes is P12; 4,096 is fine.
#[test]
fn statement_line_of_4096_bytes() {
    let stmt = "set axe lvl 1 -> 2";
    let line = format!("{stmt}{}", " ".repeat(4096 - stmt.len()));
    assert_eq!(line.len(), 4096);
    let text = format!("d2patch 1\ntable items\n{line}\n");
    assert!(parse_codes(text.as_bytes()).is_empty());
    let text = format!("d2patch 1\ntable items\n{line} \n");
    assert_eq!(parse_codes(text.as_bytes()), ["P12 3:4097"]);
}

/// A file of `len` bytes: line 1 `magic`, then one comment line.
fn file_of(magic: &str, len: usize) -> Vec<u8> {
    let mut out = format!("{magic}\n#").into_bytes();
    out.resize(len, b'x');
    out
}

/// a file over 16 MiB is P12 at 0:0; exactly 16 MiB is read.
#[test]
fn file_of_16_mib() {
    const MIB16: usize = 16 << 20;
    assert!(parse_codes(&file_of("d2patch 1", MIB16)).is_empty());
    assert_eq!(parse_codes(&file_of("d2patch 1", MIB16 + 1)), ["P12 0:0"]);
    assert!(stack_codes(&file_of("d2stack 1", MIB16)).is_empty());
    assert_eq!(stack_codes(&file_of("d2stack 1", MIB16 + 1)), ["P12 0:0"]);
}

/// the stack file's findings come before its layers' (stack position,
/// then line).
#[test]
fn stack_file_p_before_layer_p() {
    let stack = b"d2stack 1\nlayer a.d2patch\n#\t\n";
    let (layers, mut f) = load_stack("s.d2stack", stack, &mut |_| {
        Some(b"d2patch 1\nSet x\n".to_vec())
    });
    assert_eq!(layers.len(), 1);
    sort_report(&mut f);
    assert_eq!(codes(&f), ["P01 s.d2stack:3:2", "P04 a.d2patch:2:1"]);
}

// -------------------------------------------------------------- diff

fn edited(header: &str, rows: &[&str]) -> Vec<u8> {
    let mut out = header.replace(';', "\t").into_bytes();
    out.extend_from_slice(b"\r\n");
    for r in rows {
        out.extend_from_slice(r.replace(';', "\t").as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

const ITEMS_HEADER: &str = "name;code;lvl;dam;dam;*note";
const ITEMS: [&str; 4] = [
    "Axe;axe;1;3;0;",
    "Club;clb;1;2;0;old",
    "Axe;ax2;5;7;0;",
    "Big Club;clb;9;8;1;",
];

fn diff_of(t: &str, header: &str, rows: &[&str]) -> Result<String, DiffError> {
    let d = fixture();
    let e = edited(header, rows);
    diff_tables(&[(d.table(t).unwrap(), &e)], false).map(|o| s(&o))
}

fn items_with(extra: &[&str]) -> Result<String, DiffError> {
    let mut rows = ITEMS.to_vec();
    rows.extend_from_slice(extra);
    diff_of("items", ITEMS_HEADER, &rows)
}

fn err(r: Result<String, DiffError>) -> (Code, Option<usize>, usize) {
    let e = r.unwrap_err();
    (e.code, e.row.map(|r| r.0), e.line)
}

/// D03 and open question 5(h): D findings name the edited row, line
/// = row + 2.
#[test]
fn diff_d03_line_is_row_plus_2() {
    let mut rows = ITEMS;
    rows[0] = "A]x;axe;1;3;0;";
    assert_eq!(
        err(diff_of("items", ITEMS_HEADER, &rows)),
        (Code::D03, Some(0), 2)
    );
}

/// D03: a value over 1,024 bytes; 1,024 is expressible.
#[test]
fn diff_value_of_1024_bytes() {
    let ok = format!("{};new;;;;", "n".repeat(1024));
    let out = items_with(&[&ok]).unwrap();
    assert!(out.contains(&"n".repeat(1024)));
    let long = format!("{};new;;;;", "n".repeat(1025));
    assert_eq!(err(items_with(&[&long])).0, Code::D03);
}

/// D06 only for added rows of a `fixed` table; edits are fine.
#[test]
fn diff_fixed_table_rows() {
    let out = diff_of("gear", "name;code", &["Hat;cap", "Belt;blt"]).unwrap();
    assert_eq!(out, "d2patch 1\n\ntable gear\nset cap name Cap -> Hat\n");
    assert_eq!(
        err(diff_of(
            "gear",
            "name;code",
            &["Cap;cap", "Belt;blt", "X;x"]
        ))
        .0,
        Code::D06
    );
}

/// a `unique` key changed to another base row's key is D05.
#[test]
fn diff_key_moved_to_another_row_is_d05() {
    let mut rows = ITEMS;
    rows[2] = "Axe;axe;5;7;0;";
    assert_eq!(
        err(diff_of("items", ITEMS_HEADER, &rows)),
        (Code::D05, Some(2), 4)
    );
    // A key rewritten to the same normalized code is no move.
    let mut rows = ITEMS;
    rows[2] = "Axe;ax2 ;5;7;0;";
    let out = diff_of("items", ITEMS_HEADER, &rows).unwrap();
    assert!(out.contains("set #2 ax2 code ax2 -> [ax2 ]"), "{out}");
}

/// `X_i` = `B_(i−1)` or `B_(i+1)` is D05 (no key rule needed).
#[test]
fn diff_shifted_rows_are_d05() {
    let h = "description;enabled;output";
    let b = ["A;1;\"hp1,qty=3\"", "B;1;x", "C;0;y"];
    // Inserted at 1: row 2 equals base row 1.
    let ins = [b[0], "N;1;z", b[1], b[2]];
    assert_eq!(err(diff_of("recipes", h, &ins)).0, Code::D05);
    assert_eq!(err(diff_of("recipes", h, &ins)).1, Some(2));
    // Row 1 deleted (one appended): row 1 equals base row 2.
    let del = [b[0], b[2], "N;1;z"];
    assert_eq!(err(diff_of("recipes", h, &del)).0, Code::D05);
    assert_eq!(err(diff_of("recipes", h, &del)).1, Some(1));
}

/// the template shares ≥ 1 and ≥ half of the new row's
/// non-empty non-key cells; most, then lowest `t`.
#[test]
fn diff_template_choice() {
    // Ties (Axe at #0 and #2): the lowest.
    let out = items_with(&["Axe;new;;;;"]).unwrap();
    assert!(out.contains("add #4 new like axe sha:"), "{out}");
    // Most shares wins over a lower row.
    let out = items_with(&["Axe;new;5;;;"]).unwrap();
    assert!(out.contains("add #4 new like ax2 sha:"), "{out}");
    // 1 of 3 cells is under half: no template.
    let out = items_with(&["Axe;new;7;9;;"]).unwrap();
    assert!(out.contains("add #4 new\n"), "{out}");
    assert!(!out.contains("like"), "{out}");
}

/// `tsel` is the key only when unique in `X_0…X_(n−1)`.
#[test]
fn diff_template_selector_with_repeated_key() {
    let out = items_with(&["Club;new;;;;"]).unwrap();
    assert!(out.contains("add #4 new like #1 clb sha:"), "{out}");
}

// ---------------------------------------------- compile and checks

/// An invented base for every patchable table: one row (more where a
/// live-set check pins the count), almost all cells empty.
fn world() -> PatchData {
    fn cell(table: &str, col: &[u8], i: usize) -> Vec<u8> {
        match (table, col) {
            ("superuniques", b"hcIdx") => i.to_string().into_bytes(),
            ("gamble", b"code") => v("wpn"),
            ("weapons", b"code") => v(["wpn", "wp2"][i]),
            ("armor", b"code") => v("arm"),
            ("misc", b"code") => v("msc"),
            ("weapons", b"type") => v(["axe", "bad"][i]),
            ("itemtypes", b"code") => v("axe"),
            _ => Vec::new(),
        }
    }
    fn count(table: &str) -> usize {
        match table {
            "armtype" | "difficultylevels" => 3,
            "belts" => 14,
            "composit" => 16,
            "experience" => 101,
            "inventory" => 32,
            "superuniques" => 66,
            "weapons" => 2,
            _ => 1,
        }
    }
    let mut tables = Vec::new();
    for r in rules() {
        let mut header: Vec<Vec<u8>> = Vec::new();
        for c in r.lists.iter().flatten() {
            if !header.contains(c) {
                header.push(c.clone());
            }
        }
        let mut text = header.join(&b'\t');
        text.extend_from_slice(b"\r\n");
        for i in 0..count(&r.name) {
            let mut cells: Vec<Vec<u8>> = header.iter().map(|h| cell(&r.name, h, i)).collect();
            if cells.iter().all(Vec::is_empty) {
                *cells.last_mut().unwrap() = v("x");
            }
            text.extend_from_slice(&cells.join(&b'\t'));
            text.extend_from_slice(b"\r\n");
        }
        tables.push(PatchTable::from_base(&r, "t.mpq", &text).unwrap());
    }
    PatchData::from_tables(tables)
}

fn live() -> BinSet {
    let txt = || TxtTable {
        header: Vec::new(),
        records: Vec::new(),
        removed_lines: Vec::new(),
    };
    BinSet {
        lod: false,
        rules: Default::default(),
        strings: StringTables::default(),
        tables: Vec::new(),
        code: Default::default(),
        code_reports: Default::default(),
        hitclass: BinTable {
            name: "hitclass".into(),
            source: "t".into(),
            count: 0,
            record_size: 4,
            records: Vec::new(),
        },
        sounds: txt(),
        soundenviron: txt(),
    }
}

fn patched(body: &str) -> (PatchData, PatchData, Vec<Finding>) {
    let base = world();
    let mut data = base.clone();
    let f = apply_stack(&mut data, &[layer_of("l.d2patch", body, 1)], "s.d2stack");
    assert!(!has_errors(&f), "{f:?}");
    (base, data, f)
}

fn col1(d: &PatchData, t: &str, name: &str) -> usize {
    d.table(t)
        .unwrap()
        .header
        .iter()
        .position(|h| h == name.as_bytes())
        .unwrap()
        + 1
}

/// the unpatched base compiles clean and gives the live set.
#[test]
fn compile_world_clean() {
    let base = world();
    let r = compile_patched(&base, &base, &live());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert!(r.compiled.is_some() && r.live.is_some());
}

/// C02 at a `Base` cell: related are the key-column `set`s on the
/// tables filling the field's linker; old diagnostics are not reported.
#[test]
fn c02_base_cell_after_key_rename() {
    let (base, data, _) =
        patched("table itemtypes / set #0 axe code axe -> zz9 / set #0 zz9 repair [] -> 1");
    let r = compile_patched(&base, &data, &live());
    let c02: Vec<&Finding> = r.findings.iter().filter(|f| f.code == Code::C02).collect();
    assert_eq!(c02.len(), 1, "{:?}", r.findings);
    let f = c02[0];
    assert_eq!(f.file, "base:t.mpq:data\\global\\excel\\weapons.txt");
    assert_eq!((f.line, f.col), (2, col1(&data, "weapons", "type")));
    assert_eq!(f.table.as_deref(), Some("weapons"));
    assert_eq!(f.row, Some((0, v("wpn"))));
    assert_eq!(f.column, Some(v("type")));
    assert_eq!(f.related, ["l.d2patch:3"]);
    // An error: no live set.
    assert!(r.live.is_none());
}

/// An out-of-range integer is an error (C02) under `DataRules::Original`
/// and a note (C04) under `DataRules::Mod`, which clamps it.
#[test]
fn c04_out_of_range_int_under_mod_rules() {
    let (base, data, _) = patched("table weapons / set wpn durability [] -> 999");
    let r = compile_patched(&base, &data, &live());
    assert_eq!(
        r.findings.iter().map(|f| f.code).collect::<Vec<_>>(),
        [Code::C02],
        "{:?}",
        r.findings
    );
    assert!(r.live.is_none());
    let mut m = live();
    m.rules = crate::bin::DataRules::Mod;
    let r = compile_patched(&base, &data, &m);
    assert_eq!(
        r.findings.iter().map(|f| f.code).collect::<Vec<_>>(),
        [Code::C04],
        "{:?}",
        r.findings
    );
    assert!(
        r.findings[0].detail.starts_with("IntRange"),
        "{}",
        r.findings[0].detail
    );
    assert_eq!(r.live.map(|l| l.rules), Some(crate::bin::DataRules::Mod));
}

/// C02 StrMiss at a `set` cell: related is its writer.
#[test]
fn c02_strmiss_at_a_set_cell() {
    let (base, data, _) = patched("table weapons / set wpn namestr [] -> nosuchkey");
    let r = compile_patched(&base, &data, &live());
    assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
    let f = &r.findings[0];
    assert_eq!(f.code, Code::C02);
    assert!(f.detail.starts_with("StrMiss"), "{}", f.detail);
    assert_eq!((f.line, f.col), (2, col1(&data, "weapons", "namestr")));
    assert_eq!(f.related, ["l.d2patch:3"]);
}

/// only non-empty `strkey` cells are looked up; an added row with
/// empty cells is clean.
#[test]
fn added_row_with_empty_strkey_is_clean() {
    let (base, data, _) = patched("table weapons / add #2 wp3 like wpn");
    let r = compile_patched(&base, &data, &live());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert!(r.live.is_some());
}

/// C03: a live-set check fails; related are that table's `add`s.
#[test]
fn c03_related_are_the_tables_adds() {
    let (base, data, _) = patched(
        "table weapons / add #2 wp3 like wpn / table gamble / add #1 [] / table itemtypes / set axe repair [] -> 1",
    );
    let r = compile_patched(&base, &data, &live());
    let c03: Vec<&Finding> = r.findings.iter().filter(|f| f.code == Code::C03).collect();
    assert_eq!(c03.len(), 1, "{:?}", r.findings);
    assert_eq!(c03[0].table.as_deref(), Some("gamble"));
    assert_eq!(c03[0].related, ["l.d2patch:5"]);
    assert!(r.live.is_none());
}
