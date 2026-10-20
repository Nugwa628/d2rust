//! Tests for cross-reference validation in `links`: valid, out-of-range and miss
//! cells, linker size rules, and (ignored) the live set from the player's game files.

use super::*;

/// A table of `count` zeroed records of the schema's size.
fn table(name: &str, count: usize) -> BinTable {
    let size = schema().table(name).unwrap().record_size;
    BinTable {
        name: name.to_owned(),
        source: "test".to_owned(),
        count,
        record_size: size,
        records: vec![0; count * size],
    }
}

fn set(t: &mut BinTable, row: usize, offset: usize, bytes: &[u8]) {
    let o = row * t.record_size + offset;
    t.records[o..o + bytes.len()].copy_from_slice(bytes);
}

const ITEM1LOC: usize = 96; // charstats item1loc: link8(bodylocs.code)
const ITEM2LOC: usize = 104;

#[test]
fn valid_out_of_range_and_miss() {
    let bodylocs = table("bodylocs", 3);
    let mut charstats = table("charstats", 3);
    set(&mut charstats, 0, ITEM1LOC, &[2]); // valid: last index
    set(&mut charstats, 1, ITEM1LOC, &[3]); // out of range
    set(&mut charstats, 2, ITEM2LOC, &[0xFF]); // legal miss
    let sizes = LinkerSizes::from_tables([&bodylocs, &charstats]);
    assert_eq!(sizes.get("bodylocs.code"), Some(3));
    let report = validate([&charstats], &sizes);
    assert_eq!(
        report.broken,
        vec![BrokenLink {
            table: "charstats".into(),
            row: 1,
            column: "item1loc".into(),
            offset: ITEM1LOC as u32,
            linker: "bodylocs.code".into(),
            value: 3,
            size: 3,
        }]
    );
    assert_eq!(report.misses, 1);
    assert!(!report.is_clean());
    assert_eq!(
        report.broken[0].to_string(),
        "charstats row 1 column `item1loc` (+96): 3 is not an index of bodylocs.code (3 keys)"
    );
}

#[test]
fn unknown_linker_is_unchecked() {
    let mut charstats = table("charstats", 1);
    set(&mut charstats, 0, ITEM1LOC, &[200]);
    let report = validate([&charstats], &LinkerSizes::default());
    assert!(report.broken.is_empty());
    assert_eq!(report.valid + report.misses, 0);
    assert!(report
        .unchecked
        .iter()
        .any(|f| f.column == "item1loc" && f.linker == "bodylocs.code"));
}

#[test]
fn name_linker_size_is_largest_index_plus_one() {
    // monseq: key(name16) `sequence` at 0; duplicates share an index
    // so 3 records with indices 0, 1, 1 give 2 keys.
    let mut monseq = table("monseq", 3);
    set(&mut monseq, 1, 0, &[1, 0]);
    set(&mut monseq, 2, 0, &[1, 0]);
    let sizes = LinkerSizes::from_tables([&monseq]);
    assert_eq!(sizes.get("monseq.sequence"), Some(2));
}

#[test]
fn lookup_linker_takes_the_runtime_table_size() {
    // monseq `mode` is link8(monmode_lookup.code); monmode_lookup compiles
    // the same monmode.txt as the runtime monmode table.
    let monmode = table("monmode", 4);
    let mut monseq = table("monseq", 3);
    set(&mut monseq, 0, 2, &[3]);
    set(&mut monseq, 1, 2, &[4]);
    set(&mut monseq, 2, 2, &[0xFF]);
    let sizes = LinkerSizes::from_tables([&monmode, &monseq]);
    assert_eq!(sizes.get("monmode_lookup.code"), Some(4));
    let report = validate([&monseq], &sizes);
    assert_eq!(report.broken.len(), 1);
    assert_eq!((report.broken[0].row, report.broken[0].value), (1, 4));
    assert_eq!(report.misses, 1);
}

#[test]
fn shared_and_hand_built_linkers() {
    let weapons = table("weapons", 2);
    let armor = table("armor", 3);
    let misc = table("misc", 4);
    let sizes = LinkerSizes::from_tables([&weapons, &armor, &misc, &weapons]);
    assert_eq!(sizes.get("items.code"), Some(9)); // weapons counted once
    assert_eq!(sizes.get("@range"), Some(5));
    assert_eq!(sizes.get("@treasureclass"), None);

    // gems `code` is link32(items.code): 8 valid, 9 broken, −1 a miss.
    let mut gems = table("gems", 3);
    set(&mut gems, 0, 40, &8u32.to_le_bytes());
    set(&mut gems, 1, 40, &9u32.to_le_bytes());
    set(&mut gems, 2, 40, &u32::MAX.to_le_bytes());
    let report = validate([&gems], &sizes);
    assert_eq!(report.broken.len(), 1);
    assert_eq!((report.broken[0].row, report.broken[0].value), (1, 9));
    assert_eq!((report.valid, report.misses), (1, 1));
}

#[test]
fn treasure_class_linker() {
    let mut itemtypes = table("itemtypes", 2);
    set(&mut itemtypes, 1, 0x1D, &[1]);
    let mut tcx = table("treasureclassex", 3);
    set(&mut tcx, 0, 0, b"a");
    set(&mut tcx, 1, 0, b"b");
    let sizes = LinkerSizes::from_tables([&itemtypes, &tcx]);
    assert_eq!(sizes.get("@treasureclass"), Some(1 + 32 + 2));
}

/// The player's own LoD table set has no broken links.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_set_has_no_broken_links() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let archives = ArchiveSet::open_dir(dir).expect("archives open");
    let data = crate::bin::load(&archives, crate::bin::DEFAULT_LANGUAGE).expect("live set");
    let lookups = load_lookups(&archives).expect("lookup by-products");
    let (sizes, report) = validate_set(&data, &lookups);
    assert!(report.unchecked.is_empty(), "{:?}", report.unchecked);
    for b in &report.broken {
        eprintln!("{b}");
    }
    assert!(report.is_clean(), "{} broken links", report.broken.len());
    assert_eq!(sizes.get("items.code"), Some(659));
    assert_eq!(sizes.get("@treasureclass"), Some(1_013));
    assert_eq!(sizes.get("sounds.Sound"), Some(4_699));
    assert_eq!(sizes.get("monseq.sequence"), Some(60));
}
