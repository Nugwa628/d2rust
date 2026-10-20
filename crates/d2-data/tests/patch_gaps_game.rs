//! Patch-layer edge case on the player's own LoD tables: the column headers that repeat
//! a name. Needs the install in `D2_GAME_DIR`; the layer statements are our own.

use d2_data::bin::read_excel;
use d2_data::patch::{apply_stack, has_errors, parse_layer, rules, Code, PatchData};
use d2_formats::mpq::ArchiveSet;

/// `armor` repeats `mindam`/`maxdam` at columns 63/64 and 161/162,
/// `automap` repeats `Type2`, `chartemplate` repeats `SkillName` (no
/// list binds either); the leftmost copy binds, so a `set` on `name@2` is N02 (and on `name@1`
/// is not). `weapons` column 18 has an empty name.
/// Not yet run against an LoD install, so no claim is made.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn edge_duplicate_columns_in_game_headers() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let set = ArchiveSet::open_dir(dir).expect("archives open");
    let mut read = |f: &str| read_excel(&set, f).map_err(|e| e.to_string());
    let base = PatchData::from_base(&rules(), &mut read).expect("base tables read");

    let armor = base.table("armor").unwrap();
    for (c, name) in [
        (63, "mindam"),
        (64, "maxdam"),
        (161, "mindam"),
        (162, "maxdam"),
    ] {
        assert_eq!(armor.header[c], name.as_bytes(), "armor column {c}");
    }
    assert!(armor.bound[63] && armor.bound[64]);
    assert!(!armor.bound[161] && !armor.bound[162]);
    for (table, name) in [("automap", "Type2"), ("chartemplate", "SkillName")] {
        let t = base.table(table).unwrap();
        let cols: Vec<usize> = (0..t.header.len())
            .filter(|&c| t.header[c] == name.as_bytes())
            .collect();
        assert!(cols.len() >= 2, "{table} {name}: {cols:?}");
        // No field list names these columns: no copy binds.
        assert!(cols.iter().all(|&c| !t.bound[c]), "{table} {name}");
    }
    assert!(base.table("weapons").unwrap().header[18].is_empty());

    // N02 on the right copy, not on the left one (row 0 of armor).
    let key = String::from_utf8_lossy(armor.key(0)).into_owned();
    let (old1, old2) = (
        String::from_utf8_lossy(&armor.rows[0].cells[63]).into_owned(),
        String::from_utf8_lossy(&armor.rows[0].cells[161]).into_owned(),
    );
    let text = format!(
        "d2patch 1\ntable armor\nset #0 {key} mindam@2 [{old2}] -> [{old2}9]\n\
         set #0 {key} mindam@1 [{old1}] -> [{old1}9]\n"
    );
    let (layer, f) = parse_layer("t.d2patch", text.as_bytes(), 1);
    assert!(f.is_empty(), "{f:?}");
    let mut data = base.clone();
    let f = apply_stack(&mut data, &[layer], "t.d2stack");
    assert!(!has_errors(&f), "{f:?}");
    let got: Vec<(Code, usize)> = f.iter().map(|f| (f.code, f.line)).collect();
    assert_eq!(got, [(Code::N02, 3)]);
}
