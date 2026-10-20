//! Integration tests for the production loaders of `d2_server::world_data` a game is built
//! from (`game::GameTables`, `tables::drop_tables`, `tables::SaveData`)
//! on the synthetic install (`test_fixtures::install`, no game files),
//! and the same loaders on the user's install (`#[ignore]`,
//! `D2_GAME_DIR`). `world_data` feeds the server's game build, the drop tables
//! and the character save/load adapters.

use std::path::PathBuf;

use d2_formats::d2s::{Hireling, SaveTables};
use d2_formats::mpq::ArchiveSet;
use d2_server::world_data::game::GameTables;
use d2_server::world_data::tables::{drop_tables, SaveData};
use d2_sim::items::bitstream::{write_save, StreamItem};
use test_fixtures::{content, install, synth};

fn synthetic() -> (install::Install, GameTables) {
    // One directory per call: nextest runs each test in its own process,
    // `cargo test` runs them as threads of one.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("synthetic-world-data-{}-{n}", std::process::id()));
    let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
    let t = GameTables::load(&i.archives).expect("the synthetic install loads");
    (i, t)
}

// Every view of the loaded tables builds from the synthetic install.
#[test]
fn every_game_view_builds_from_the_synthetic_install() {
    let (_, t) = synthetic();
    t.action_tables().unwrap();
    t.object_tables().unwrap();
    t.world_tables().unwrap();
    t.vitals().unwrap();
    t.stat_data().unwrap();
    assert!(t.unit_data(true).unwrap().expansion);
    assert!(!t.unit_data(false).unwrap().expansion);
    t.hire_rows().unwrap();
    assert_eq!(t.item_tables().unwrap().items.len(), 6);
    assert_eq!(t.vendor_tables().unwrap().items.len(), 6);
    let p = t.class_picks().unwrap();
    assert_eq!((p.rows.len(), p.weapons, p.armor), (6, 2, 2));
}

// The drop tables hold the item list, treasure classes and super uniques.
#[test]
fn drop_tables_hold_the_items_the_treasure_classes_and_the_superuniques() {
    let (_, t) = synthetic();
    let d = drop_tables(&t.fixed).unwrap();
    assert_eq!(d.items.items.len(), 6, "2 weapons + 2 armor + 2 misc");
    assert_eq!(d.treasure_items.len(), 6, "the item list in index order");
    // TC 0, 32 automatic TCs for each of `weap` and `armo`, 3 rows
    // (`test-fixtures` `server_tables::treasure_classes`).
    assert_eq!(d.tcs.len(), 1 + 64 + 3);
    assert_eq!(d.superuniques.len(), content::SUPERUNIQUES);
}

// The save tables read the save columns and measure item entries.
#[test]
fn save_data_reads_the_save_columns_and_measures_item_entries() {
    let (_, t) = synthetic();
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    assert_eq!(s.stats.len(), content::STATS.len());
    // `CSvBits` of the synthetic rows: 32 for experience, else 10.
    let exp = content::STATS
        .iter()
        .position(|n| *n == "experience")
        .unwrap();
    assert_eq!(s.stat_save(exp as u16).map(|c| c.bits), Some(32));
    assert_eq!(s.stat_save(0).map(|c| c.bits), Some(10));
    assert_eq!(s.stat_save(content::STATS.len() as u16), None);
    // A compact record of the last misc code: its stream is the entry;
    // the bytes after it are not read.
    let code = s.items.items.last().unwrap().code;
    let item = StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code,
        page: 0xFF,
        ..StreamItem::default()
    };
    let (bytes, _) = write_save(&item, &s.items.isc).unwrap();
    let mut buf = bytes.clone();
    buf.extend_from_slice(&[0xFF; 4]);
    assert_eq!(s.item_entry_len(&buf), Ok(bytes.len()));
    // A code outside the items table does not decode.
    let bad = StreamItem {
        code: *b"zzz ",
        ..item
    };
    let (bytes, _) = write_save(&bad, &s.items.isc).unwrap();
    assert!(s.item_entry_len(&bytes).is_err());
}

// A saved hireling is restored only when present and its row exists.
#[test]
fn a_saved_hireling_is_restored_when_present_and_its_row_is_found() {
    let (_, t) = synthetic();
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    let rows = &s.hirelings.rows;
    assert!(!rows.is_empty(), "the synthetic install has hireling rows");
    let id = rows
        .iter()
        .map(|r| r.id)
        .find(|&id| s.hirelings.row_at(true, id, 1).is_some())
        .expect("an expansion row at level 1");
    let id = u16::try_from(id).unwrap();
    let present = Hireling {
        flags: 0,
        seed: 7,
        name_index: 0,
        id,
        experience: 0,
        rest: [0; 16],
    };
    assert!(s.hireling_restored(&present));
    // Rule 1: seed, name index and experience all 0 → none.
    let absent = Hireling { seed: 0, ..present };
    assert!(!s.hireling_restored(&absent));
    // Rule 2: no row for the `Id` → not restored (M08: the same block
    // with an id no row has).
    let unknown = (0..=255u16)
        .find(|&i| s.hirelings.row_at(true, u32::from(i), 1).is_none())
        .expect("an id without rows");
    let no_row = Hireling {
        id: unknown,
        ..present
    };
    assert!(!s.hireling_restored(&no_row));
}

/// The user's install (`$D2_GAME_DIR`).
fn live() -> GameTables {
    let dir = std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR is set");
    let archives = ArchiveSet::open_dir(PathBuf::from(dir)).expect("archives open");
    GameTables::load(&archives).expect("the live set loads")
}

// Every loader builds from the user's install.
#[test]
#[ignore = "needs the game files (D2_GAME_DIR)"]
fn every_loader_builds_from_the_users_install() {
    let t = live();
    t.action_tables().unwrap();
    t.object_tables().unwrap();
    t.world_tables().unwrap();
    t.vitals().unwrap();
    t.stat_data().unwrap();
    t.unit_data(true).unwrap();
    t.hire_rows().unwrap();
    t.vendor_tables().unwrap();
    let d = drop_tables(&t.fixed).unwrap();
    assert!(d.tcs.notes.is_empty(), "{:?}", d.tcs.notes);
    // The game's own tables hold 66 super uniques.
    assert_eq!(d.superuniques.len(), 66);
    assert_eq!(
        d.treasure_items.len(),
        d.items.items.len(),
        "one item list entry per items row"
    );
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    assert_eq!(s.stats.len(), t.table("itemstatcost").unwrap().count);
}

// The object tables load objgroup, leveldefs and the drop-pick columns.
#[test]
#[ignore = "needs the game files (D2_GAME_DIR)"]
fn object_tables_load_objgroup_leveldefs_and_the_pick_columns() {
    let t = live();
    let o = t.object_tables().unwrap();
    // `objgroup.bin` (d2exp): one record per objgroup.txt row, the
    // `EXPANSION` separator (record 97) included (`d2-data` game_data).
    assert_eq!(o.objgroup.len(), 133);
    let g = &o.objgroup[97];
    assert_eq!([g.id0, g.id1, g.id2, u32::from(g.prob0)], [0; 4]);
    // `leveldefs.bin` is built from levels.txt: one record per levels row.
    assert_eq!(o.leveldefs.len(), o.levels.len());
    assert!(o.objects.len() > 500, "objects rows: {}", o.objects.len());
    // The drop helpers' picks: "200 of the 306 weapon rows have bit 1".
    let p = t.class_picks().unwrap();
    assert_eq!(p.weapons, 306);
    let w = p.part(d2_sim::treasure::class_pick::Part::Weapons);
    let keep = p.rows[w].iter().filter(|r| r.bitfield1 & 2 != 0).count();
    assert_eq!(keep, 200);
    assert_eq!(p.rows.len(), t.item_tables().unwrap().items.len());
}
