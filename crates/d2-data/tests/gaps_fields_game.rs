//! Game-file tests for `d2-data` field types and txt linking: they compare the compiled
//! tables with the `.bin` files shipped in the player's LoD install, found through
//! `D2_GAME_DIR`.
//!
//! Run: `D2_GAME_DIR=<install> cargo test -p d2-data --test gaps_fields_game -- --ignored`

use std::sync::OnceLock;

use d2_data::bin::{self, read_excel, BinSet};
use d2_data::compile::{check_field_list, Linkers};
use d2_data::compile_set::{compile_all, CompiledSet};
use d2_data::crosscheck::{self, Role, REASON_NAMESTR_707};
use d2_data::schema::{schema, Link};
use d2_data::strings::StringTables;
use d2_data::txt::{bind, TxtTable};
use d2_formats::mpq::ArchiveSet;

fn set() -> &'static ArchiveSet {
    static SET: OnceLock<ArchiveSet> = OnceLock::new();
    SET.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        ArchiveSet::open_dir(dir).expect("archives open")
    })
}

fn live() -> &'static BinSet {
    static B: OnceLock<BinSet> = OnceLock::new();
    B.get_or_init(|| bin::load(set(), bin::DEFAULT_LANGUAGE).expect("live set loads"))
}

fn compile() -> CompiledSet {
    let strings = StringTables::load(set(), "eng", true).expect("string tables");
    let mut read = |f: &str| read_excel(set(), f).map_err(|e| e.to_string());
    compile_all(&mut read, &strings).expect("text set compiles")
}

fn compiled() -> &'static CompiledSet {
    static C: OnceLock<CompiledSet> = OnceLock::new();
    C.get_or_init(compile)
}

fn record(table: &str, i: usize) -> &'static [u8] {
    compiled().table(table).unwrap().compiled.record(i)
}

fn txt(file: &str) -> TxtTable {
    let (_, bytes) = read_excel(set(), file).unwrap().expect("txt present");
    TxtTable::parse(file, &bytes).unwrap()
}

fn code(s: &[u8]) -> u32 {
    u32::from_le_bytes(d2_data::compile::code4(s))
}

/// The acceptance test for the compiler: 86 compared tables (73
/// runtime, `hitclass`, 12 by-products), counts equal, every byte equal
/// except monstats record 707 `NameStr`; code buffers and callback bytes
/// equal.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn compiled_tables_equal_shipped_bins() {
    let report = crosscheck::run(set()).expect("cross-check runs");
    let compared: Vec<_> = report.tables.iter().filter(|t| t.compared()).collect();
    let count = |r: Role| compared.iter().filter(|t| t.role == r).count();
    assert_eq!(
        (
            count(Role::Runtime),
            count(Role::ClientOnly),
            count(Role::ByProduct)
        ),
        (73, 1, 12)
    );
    // The cross-archive pairs and levels.txt → leveldefs.bin.
    let t = |n: &str| report.tables.iter().find(|t| t.name == n).unwrap();
    for (name, txt_src, bin_src) in [
        ("automagic", "d2exp.mpq", "patch_d2.mpq"),
        ("rareprefix", "d2exp.mpq", "patch_d2.mpq"),
        ("raresuffix", "d2exp.mpq", "patch_d2.mpq"),
        ("inventory", "patch_d2.mpq", "d2exp.mpq"),
        ("plrmode", "patch_d2.mpq", "d2exp.mpq"),
    ] {
        let r = t(name);
        assert_eq!(
            (r.txt_source.as_str(), r.bin_source.as_deref()),
            (txt_src, Some(bin_src)),
            "{name}"
        );
    }
    let leveldefs = schema().table("leveldefs").unwrap();
    assert_eq!(leveldefs.txt_name, "levels.txt");
    assert!(t("leveldefs").compared());
    // Counts equal; 85 identical; monstats differs only in
    // record 707 NameStr (2 bytes, 1 record).
    for r in &compared {
        assert!(r.counts_equal(), "{}", r.name);
        assert!(r.matches(), "{}: {:?}", r.name, r.mismatches);
        if r.name == "monstats" {
            assert_eq!(
                r.explained.iter().collect::<Vec<_>>(),
                [(&REASON_NAMESTR_707, &(2, 1))]
            );
        } else {
            assert!(r.identical(), "{}", r.name);
        }
    }
    assert_eq!(compared.iter().filter(|r| r.identical()).count(), 85);
    // The code buffers.
    assert_eq!(report.buffers.len(), 4);
    for b in &report.buffers {
        assert!(b.identical(), "{b:?}");
    }
    // Every table-specific callback has a known meaning, and their tables match
    // (checked above).
    assert!(report.unspecified_callbacks.is_empty());
    for name in ["monstats", "monstats2", "monpreset", "cubemain"] {
        assert!(t(name).matches(), "{name}");
    }
}

/// Field-type checks on the LoD set.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn records_layout_on_live_set() {
    // Every footprint lies inside its record (E13 otherwise).
    for def in &schema().tables {
        if def.record_size > 0 && !def.fields.is_empty() {
            check_field_list(&def.name, &def.fields, def.record_size)
                .unwrap_or_else(|e| panic!("{}: {e}", def.name));
        }
    }
    // The .bin container: u32 count, then count × size bytes.
    for t in &live().tables {
        let (_, bytes) = read_excel(set(), &schema().table(&t.name).unwrap().bin_name)
            .unwrap()
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            t.count
        );
        assert_eq!(bytes.len(), 4 + t.count * t.record_size, "{}", t.name);
    }
    // Bytes outside every field footprint are 0 in the live bins (tables
    // without table-specific callbacks, which write outside them).
    let mut checked = 0;
    for t in &live().tables {
        let def = schema().table(&t.name).unwrap();
        if def.fields.iter().any(|f| matches!(f.link, Link::Table(_))) {
            continue;
        }
        let mut covered = vec![false; def.record_size];
        for f in &def.fields {
            for o in f.footprint() {
                covered[o] = true;
            }
        }
        for (r, rec) in t.iter().enumerate() {
            for (o, &b) in rec.iter().enumerate() {
                assert!(covered[o] || b == 0, "{} record {r} byte {o}", t.name);
            }
        }
        checked += 1;
    }
    assert!(checked >= 69, "{checked}");
    // leveldefs records come from levels.txt: same rows.
    let levels = txt("levels.txt");
    assert_eq!(compiled().table("leveldefs").unwrap().compiled.count, 137);
    assert_eq!(levels.records.len(), 137);
    // Compilation is a pure function of its inputs.
    let again = compile();
    for (a, b) in compiled().tables.iter().zip(&again.tables) {
        assert_eq!(a.compiled, b.compiled, "{}", a.name);
    }
    assert_eq!(compiled().buffers, again.buffers);
}

/// Linker checks on the LoD data: shared item-code linker, find-or-register
/// duplicates and the empty keys.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn linker_data_on_live_set() {
    let l: &Linkers = &compiled().linkers;
    let items = l.code("items.code").unwrap();
    assert_eq!(items.len(), 659);
    assert_eq!(items.find(code(b"r08")), Some(617));
    assert_eq!(
        u32::from_le_bytes(record("runes", 0)[152..156].try_into().unwrap()),
        617
    );
    let it = l.code("itemtypes.code").unwrap();
    for (k, i) in [(0u32, 0u32), (1, 1), (2, 14), (3, 17), (4, 23)] {
        assert_eq!(it.find(0x2020_2020 + k), Some(i));
    }
    for name in [
        "bodylocs.code",
        "elemtypes.code",
        "hitclass.code",
        "hiredesc.code",
    ] {
        assert_eq!(l.code(name).unwrap().find(0x2020_2020), Some(0), "{name}");
    }
    for name in [
        "sounds.Sound",
        "montype.type",
        "monsounds.Id",
        "monseq.sequence",
    ] {
        assert_eq!(l.name(name).unwrap().find(b""), Some(0), "{name}");
    }
    let u16_at = |r: &[u8]| u16::from_le_bytes([r[0], r[1]]);
    for (r, v) in [(617, 617), (723, 617), (724, 723), (733, 732)] {
        assert_eq!(u16_at(record("monstats", r)), v, "monstats {r}");
    }
    assert_eq!(compiled().table("monseq").unwrap().compiled.count, 1010);
    assert_eq!(l.name("monseq.sequence").unwrap().len(), 60);
    assert_eq!(u16_at(record("monseq", 1009)), 59);
}

/// Txt edge cases on the LoD set.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn txt_edge_case_data() {
    // weapons.txt lacks BetterGem (RAW): 0 in weapons.bin.
    let weapons = txt("weapons.txt");
    assert!(!weapons
        .header
        .iter()
        .any(|h| h.eq_ignore_ascii_case(b"BetterGem")));
    for r in live().table("weapons").unwrap().iter() {
        assert_eq!(&r[188..192], [0; 4]);
    }
    // itemtypes records 1, 14, 17, 23 hold "    "; keys are bumped.
    let it = live().table("itemtypes").unwrap();
    for i in [0, 1, 14, 17, 23] {
        assert_eq!(&it.record(i)[..4], b"    ");
    }
    // NAMETOWORD2: skills pettype at 190, the next field at 191.
    let skills = schema().table("skills").unwrap();
    assert_eq!(skills.field("pettype").unwrap().offset, 190);
    assert_eq!(skills.field("summode").unwrap().offset, 191);
    // Column map slots of the largest lists.
    for (table, columns, missing) in [("skills", 256, 0), ("armor", 164, 39), ("levels", 140, 45)] {
        let def = schema().table(table).unwrap();
        let t = txt(&def.txt_name);
        let names: Vec<&[u8]> = def.fields.iter().map(|f| f.column.as_slice()).collect();
        let b = bind(&t.header, &names);
        assert_eq!(
            (t.columns(), b.missing_fields().count()),
            (columns, missing),
            "{table}"
        );
    }
}
