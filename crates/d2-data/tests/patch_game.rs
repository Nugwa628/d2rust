//! Patch layers on the player's own LoD tables: they need the install in
//! `D2_GAME_DIR`. The layers are our own statements; no game data is
//! written anywhere.

use std::sync::OnceLock;

use d2_data::bin::{self, read_excel, BinSet};
use d2_data::fixup;
use d2_data::patch::{
    apply_stack, compile_patched, diff_tables, has_errors, load_stack, parse_layer, rules, Code,
    Finding, PatchData,
};
use d2_formats::mpq::ArchiveSet;

fn set() -> &'static ArchiveSet {
    static SET: OnceLock<ArchiveSet> = OnceLock::new();
    SET.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        ArchiveSet::open_dir(dir).expect("archives open")
    })
}

/// `AnimData.d2` for the monstats speed fix-ups (`d2_data::fixup`).
fn anim() -> &'static d2_formats::animdata::AnimData {
    static A: OnceLock<d2_formats::animdata::AnimData> = OnceLock::new();
    A.get_or_init(|| fixup::read_animdata(set()).expect("AnimData.d2"))
}

fn live() -> &'static BinSet {
    static L: OnceLock<BinSet> = OnceLock::new();
    L.get_or_init(|| bin::load(set(), bin::DEFAULT_LANGUAGE).expect("live set loads"))
}

fn base() -> &'static PatchData {
    static B: OnceLock<PatchData> = OnceLock::new();
    B.get_or_init(|| {
        let mut read = |f: &str| read_excel(set(), f).map_err(|e| e.to_string());
        PatchData::from_base(&rules(), &mut read).expect("base tables read")
    })
}

/// Applies one layer of `body` (after `d2patch 1`) to the base.
fn apply(body: &str) -> (PatchData, Vec<Finding>) {
    let text = format!("d2patch 1\n{body}\n");
    let (layer, f) = parse_layer("t.d2patch", text.as_bytes(), 1);
    assert!(f.is_empty(), "{f:?}");
    let mut data = base().clone();
    let f = apply_stack(&mut data, &[layer], "t.d2stack");
    (data, f)
}

fn errors(f: &[Finding]) -> Vec<&Finding> {
    f.iter().filter(|f| f.code.is_error()).collect()
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g1_rows_render_identity_and_empty_diff() {
    let b = base();
    assert_eq!(b.tables.len(), 85);
    let rows: usize = b.tables.iter().map(|t| t.rows.len()).sum();
    assert_eq!(rows, 21_632);
    for t in &b.tables {
        let (_, bytes) = read_excel(set(), &t.rules.txt_name).unwrap().unwrap();
        // render(read(f)) = f minus its Expansion lines.
        let expected: Vec<u8> = bytes
            .split_inclusive(|&b| b == b'\n')
            .enumerate()
            .filter(|(i, l)| {
                *i == 0 || l.split(|&b| b == b'\t' || b == b'\r').next() != Some(b"Expansion")
            })
            .flat_map(|(_, l)| l.iter().copied())
            .collect();
        assert_eq!(t.render(), expected, "{}", t.name());
        let out = diff_tables(&[(t, &t.render())], false).unwrap();
        assert_eq!(out, b"d2patch 1\n", "{}", t.name());
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g2_empty_stack_compiles_clean() {
    let r = compile_patched(base(), base(), live());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    let patched = r.live.expect("no C error");
    fixup::apply(&patched, anim()).expect("fix-ups apply");
}

const ITEMS: &str = "d2patch 1
# Stronger hand axes, a new axe and a new magic prefix.

table weapons
set hax mindam 3 -> 4
set hax maxdam 6 -> 8
# The new axe copies the patched Hand Axe row.
add #306 ov1 like hax sha:527afb9693739d22
set ov1 level 3 -> 9
set ov1 mindam 4 -> 6
set ov1 maxdam 8 -> 11

table armor
# Two mindam columns; the compiler binds the first.
set buc mindam@1 1 -> 2

table magicprefix
set #1 Sturdy levelreq 3 -> 2
add #669 Keen like #12 Jagged sha:31284bff98e59f80
set Keen mod1min 10 -> 12
set Keen mod1max 20 -> 25

table treasureclassex
check [Act 1 Good] group 5
set [Act 1 Good] Picks 1 -> 2
";

const BALANCE: &str = "d2patch 1
# Written against base + 10-items.
table weapons
set hax mindam 4 -> 5
";

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g3_example_stack() {
    let stack = b"d2stack 1\nlayer core/10-items.d2patch\nlayer core/20-balance.d2patch\n";
    let (layers, f) = load_stack("overhaul.d2stack", stack, &mut |p| match p {
        "core/10-items.d2patch" => Some(ITEMS.as_bytes().to_vec()),
        "core/20-balance.d2patch" => Some(BALANCE.as_bytes().to_vec()),
        _ => None,
    });
    assert!(f.is_empty(), "{f:?}");
    let mut data = base().clone();
    let f = apply_stack(&mut data, &layers, "overhaul.d2stack");
    let shown: Vec<String> = f.iter().map(Finding::to_string).collect();
    assert_eq!(
        shown,
        [
            "note[N01] core/20-balance.d2patch:4:16: weapons hax (#0) mindam: replaces [4] \
          written by core/10-items.d2patch:5; writer core/10-items.d2patch:5"
        ]
    );
    let digest = |t: &str| data.table(t).unwrap().digest()[..16].to_owned();
    assert_eq!(digest("weapons"), "ef2ca1073f89cf91");
    assert_eq!(digest("armor"), "a3bd13b18286ee12");
    assert_eq!(digest("magicprefix"), "63b952e7aa3c56f0");
    assert_eq!(digest("treasureclassex"), "0a0a4c940f8f3fb9");
    let r = compile_patched(base(), &data, live());
    assert!(!has_errors(&r.findings), "{:?}", r.findings);
    let patched = r.live.expect("no C error");
    assert_eq!(
        patched.table("weapons").unwrap().count,
        live().table("weapons").unwrap().count + 1
    );
    fixup::apply(&patched, anim()).expect("fix-ups apply");
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g4_g5_selector_and_key_errors() {
    let (_, f) = apply("table monstats\nset cr_lancer8 Level 5 -> 6");
    assert_eq!(errors(&f)[0].code, Code::A04);
    assert_eq!(errors(&f)[0].related, ["monstats 617", "monstats 723"]);
    let (_, f) = apply("table armor\nset buc mindam 1 -> 2");
    assert_eq!(errors(&f)[0].code, Code::A02);
    assert_eq!(errors(&f)[0].related, ["column 63", "column 161"]);
    let (_, f) = apply("table armor\nadd #202 hax");
    assert_eq!(errors(&f)[0].code, Code::A12);
    let (_, f) = apply("table treasureclassex\nadd #853 [act 1 good]");
    assert_eq!(errors(&f)[0].code, Code::A12);
    assert_eq!(
        errors(&f)[0].related,
        ["treasureclassex 209", "treasureclassex 853"]
    );
    let (_, f) = apply("table skills\nset [fire bolt] reqlevel 1 -> 2");
    assert_eq!(errors(&f)[0].code, Code::A03);
}

fn c02(body: &str) -> Vec<Finding> {
    let (data, f) = apply(body);
    assert!(!has_errors(&f), "{f:?}");
    compile_patched(base(), &data, live()).findings
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g6_g7_g8_compile_checks() {
    let f = c02("table weapons\nset hax type axe -> zzz");
    assert!(
        f.iter()
            .any(|f| f.code == Code::C02 && f.detail.starts_with("LinkMiss")),
        "{f:?}"
    );
    let f = c02("table weapons\nset hax namestr hax -> nosuchkey");
    assert!(
        f.iter()
            .any(|f| f.code == Code::C02 && f.detail.starts_with("StrMiss")),
        "{f:?}"
    );
    let f = c02("table itemtypes\nset #28 axe Code axe -> zz9");
    let at_weapons: Vec<&Finding> = f
        .iter()
        .filter(|f| f.code == Code::C02 && f.table.as_deref() == Some("weapons"))
        .collect();
    assert!(!at_weapons.is_empty(), "{f:?}");
    assert!(at_weapons.iter().all(|f| f.related == ["t.d2patch:3"]));
    let f = c02("table gamble\nadd #125 X");
    assert!(f.iter().any(|f| f.code == Code::C03), "{f:?}");
}

// The fidelity bar of the patch machinery: the empty stack compiles to the LoD tables.
// Every record table and code buffer of the empty-stack compile equals the one loaded from
// the shipped `.bin` files, byte for byte.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g9_empty_stack_equals_live_bins() {
    let r = compile_patched(base(), base(), live());
    let patched = r.live.expect("no C error");
    let l = live();
    assert_eq!(patched.tables.len(), l.tables.len());
    let mut differ = Vec::new();
    for (p, o) in patched.tables.iter().zip(&l.tables) {
        assert_eq!(p.name, o.name);
        if (p.count, p.record_size) != (o.count, o.record_size) || p.records != o.records {
            let first = p.records.iter().zip(&o.records).position(|(a, b)| a != b);
            differ.push(format!(
                "{} (count {} vs {}, first byte {first:?})",
                p.name, p.count, o.count
            ));
        }
    }
    assert_eq!(patched.code.len(), l.code.len());
    for (k, c) in &patched.code {
        if c.bytes != l.code[k].bytes {
            differ.push(format!("code {k:?}"));
        }
    }
    assert!(differ.is_empty(), "{differ:?}");
}

/// `addcol` extension columns on base tables: the patched live set is
/// byte-identical to the empty stack's (records, sizes, code buffers), and
/// mod code reads the values through the typed accessor.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn g10_addcol_keeps_records() {
    use d2_data::schema::ExtValue;
    let (data, f) = apply(
        "table weapons
addcol xbonus int 3
addcol [x note] text
set hax xbonus 3 -> -7
set hax [x note] [] -> [mod axe]
table itemstatcost
addcol xsynergy int
table levels
addcol xweather text clear",
    );
    assert!(f.is_empty(), "{f:?}");
    let r = compile_patched(base(), &data, live());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    let plain = compile_patched(base(), base(), live());
    let (p, o) = (r.live.expect("no C error"), plain.live.expect("no C error"));
    for (a, b) in p.tables.iter().zip(&o.tables) {
        assert_eq!(
            (a.count, a.record_size, &a.records),
            (b.count, b.record_size, &b.records),
            "{}",
            a.name
        );
    }
    for (k, c) in &p.code {
        assert_eq!(c.bytes, o.code[k].bytes, "code {k:?}");
    }
    let hax = data.table("weapons").unwrap().rows_keyed(b"hax")[0];
    assert_eq!(r.ext.ext("weapons", hax, "xbonus"), Some(ExtValue::Int(-7)));
    assert_eq!(
        r.ext.ext("weapons", hax + 1, "XBONUS"),
        Some(ExtValue::Int(3))
    );
    assert_eq!(
        r.ext.ext("weapons", hax, "x note"),
        Some(ExtValue::Text(b"mod axe".to_vec()))
    );
    assert_eq!(
        r.ext.ext("itemstatcost", 0, "xsynergy"),
        Some(ExtValue::Int(0))
    );
    assert_eq!(
        r.ext.ext("levels", 1, "xweather"),
        Some(ExtValue::Text(b"clear".to_vec()))
    );
    assert_eq!(r.ext.ext("levels", 1, "xbonus"), None);
    assert!(plain.ext.is_empty());
}
