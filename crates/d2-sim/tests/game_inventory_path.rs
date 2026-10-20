//! Game-file tests for the inventory grids and belts and for the path
//! constant tables:
//!
//! - every `inventory.bin` record through D2RUST's `InvTables` projection
//!   and `grid_record` / `page_grid_size` for every owner and page, both
//!   resolutions (records 0–15 and their 800 × 600 copies 16–31);
//! - `belts.bin` capacities and the belt records of every item that
//!   equips at body location 8;
//! - every weapons / armor / misc item size against the free-position
//!   search on every server grid;
//! - `tables/sim/path-tables.tsv` against `Game.exe` in `D2_GAME_DIR`:
//!   rows are read at their `va` in the executable file image, and D2RUST's
//!   `FIELD_DX` and `FIELD_DY` are compared with those bytes too.
//!
//! Spawn-point placement is not tested here: the example points lack map
//! seeds, and this test has no live `CollisionView` / `LevelView` provider.
//!
//! `D2_GAME_DIR=<install> cargo test -p d2-sim --test game_inventory_path -- --ignored`

mod items_treasure_live;

use std::sync::OnceLock;

use d2_data::tables::{Belts, Inventory as InvBin};
use d2_sim::items::inventory::belt::{BELT_SLOTS, DEFAULT_BELT};
use d2_sim::items::inventory::grid::{
    fits, grid_record, in_bounds, page_grid_size, search, weight, CLASS_RECORDS,
};
use d2_sim::items::inventory::{
    beltable, body, body_location_allowed, page, similar, Grid, InvTables, UnitKind, BELT_GRID,
    BODY_GRID,
};
use d2_sim::path::search::{FIELD_DX, FIELD_DY};
use d2_sim::units::UnitId;
use items_treasure_live::{code, fixed, typed};

fn tables() -> &'static InvTables {
    static T: OnceLock<InvTables> = OnceLock::new();
    T.get_or_init(|| InvTables::from_fixed(fixed()).expect("inventory tables project"))
}

// ------------------------------------------------------------ inventory

/// The §1.3 measured table, records 0–15 (gridX × gridY).
fn spec_grid(record: usize) -> (u8, u8) {
    match record {
        5 => (10, 10),
        8 => (6, 4),
        9 => (3, 4),
        12 => (6, 8),
        13 => (0, 0),
        0..=15 => (10, 4),
        _ => unreachable!("server records are 0–15"),
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_inventory_records_both_resolutions() {
    // D1 through the projection the sim uses: 32 records.
    let t = tables();
    assert_eq!(t.grids.len(), 32);
    assert_eq!(typed::<InvBin>().len(), 32);
    for r in 0..16 {
        let g = t.grids[r];
        assert_eq!((g.grid_x, g.grid_y), spec_grid(r), "record {r}");
    }
    // Records 16–31: the 800 × 600 layouts of 0–15 with the grid sizes of
    // record r − 16, except 29 (Hireling2): 255 × 255, its `.txt` grid is
    // −1 stored as u8 (inventory.md §1.3, GX1 answered).
    for r in 16..32 {
        let g = t.grids[r];
        let want = if r == 29 {
            (255, 255)
        } else {
            spec_grid(r - 16)
        };
        assert_eq!(
            (g.grid_x, g.grid_y),
            want,
            "record {r} (grid of {})",
            r - 16
        );
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_page_grids_every_owner() {
    let t = tables();
    // Players, every class: page 0 → the class record (10 × 4 for all
    // seven), 1 / 2 trade 10 × 4, 3 cube 3 × 4, 4 stash 6 × 4 classic,
    // 6 × 8 expansion; any other page → the class record.
    for class in 0..7u8 {
        let owner = UnitKind::Player { class };
        let rec = CLASS_RECORDS.iter().find(|&&(c, _)| c == class).unwrap().1;
        for exp in [false, true] {
            assert_eq!(grid_record(owner, page::INVENTORY, exp), Some(rec));
            for (pg, want) in [
                (page::INVENTORY, (10, 4)),
                (page::TRADE1, (10, 4)),
                (page::TRADE2, (10, 4)),
                (page::CUBE, (3, 4)),
                (page::STASH, if exp { (6, 8) } else { (6, 4) }),
                (5, (10, 4)),
                (page::NONE, (10, 4)),
            ] {
                assert_eq!(
                    page_grid_size(t, owner, pg, exp),
                    Some(want),
                    "class {class} page {pg} expansion {exp}"
                );
            }
        }
    }
    // Class without a pair → −1: no grid.
    assert_eq!(
        page_grid_size(t, UnitKind::Player { class: 7 }, 0, true),
        None
    );
    // Monsters: record 5, 10 × 10, every page.
    for pg in 0..5 {
        assert_eq!(
            page_grid_size(t, UnitKind::Monster { class: 0 }, pg, true),
            Some((10, 10))
        );
    }
    // Objects 0x152 / 0x153: records 10 / 11, 10 × 4; others none.
    for class in [0x152, 0x153] {
        assert_eq!(
            page_grid_size(t, UnitKind::Object { class }, 0, true),
            Some((10, 4))
        );
    }
    assert_eq!(
        page_grid_size(t, UnitKind::Object { class: 0 }, 0, true),
        None
    );
    // Constant grids (§1.2).
    assert_eq!(BODY_GRID, (13, 1));
    assert_eq!(BELT_GRID, (16, 1));
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_belt_capacities() {
    // D2 through the projection, and the §3.1 names: records 0–6 = belt
    // 12, sash 8, default 4, girdle 16, light belt 8, heavy belt 12, uber
    // belt 16; 7–13 repeat them.
    let t = tables();
    assert_eq!(typed::<Belts>().len(), 14);
    assert_eq!(t.belts, [12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16]);
    for r in 0..7 {
        assert_eq!(t.numboxes(r), t.numboxes(r + 7), "record {r}");
    }
    assert_eq!(t.numboxes(DEFAULT_BELT), Some(4));
    assert!(t.belts.iter().all(|&n| n <= BELT_SLOTS));
    // Every item that equips at body location 8 names a belts record
    // (§3.1 rule 1 reads `belt` of that item).
    let mut belts = 0;
    for rec in 0..t.items.len() {
        if body_location_allowed(t, rec, body::BELT) {
            belts += 1;
            let b = t.items[rec].belt;
            assert!(
                t.numboxes(usize::from(b)).is_some(),
                "{} belt record {b}",
                code(t.items[rec].code)
            );
        }
    }
    assert!(belts > 0, "no item equips at body location 8");
    println!("{belts} items equip at body location 8");
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_potion_groups_similar() {
    // §3.4 on the live misc records: the group codes exist and are
    // similar inside their group, not across groups.
    let t = tables();
    let rec = |c: &[u8; 4]| {
        t.items
            .iter()
            .position(|r| &r.code == c)
            .unwrap_or_else(|| panic!("item {}", code(*c)))
    };
    let groups: [&[&[u8; 4]]; 3] = [
        &[b"hp1 ", b"hp2 ", b"hp3 ", b"hp4 ", b"hp5 "],
        &[b"mp1 ", b"mp2 ", b"mp3 ", b"mp4 ", b"mp5 "],
        &[b"rvl ", b"rvs "],
    ];
    for (gi, g) in groups.iter().enumerate() {
        for a in g.iter() {
            for (hi, h) in groups.iter().enumerate() {
                for b in h.iter() {
                    assert_eq!(
                        similar(t, rec(a), rec(b)),
                        gi == hi,
                        "{} / {}",
                        code(**a),
                        code(**b)
                    );
                }
            }
        }
    }
}

/// Every in-bounds fitting top-left cell, as §2.1 defines a fit.
fn any_fit(g: &Grid, w: u8, h: u8) -> bool {
    (0..i32::from(g.height)).any(|y| {
        (0..i32::from(g.width)).any(|x| {
            in_bounds(g, x, y, w, h)
                && (y..y + i32::from(h))
                    .all(|yy| (x..x + i32::from(w)).all(|xx| g.cell(xx, yy).is_none()))
        })
    })
}

fn mark(g: &mut Grid, x: i32, y: i32, w: u8, h: u8, id: u32) {
    for yy in y..y + i32::from(h) {
        for xx in x..x + i32::from(w) {
            let i = yy as usize * usize::from(g.width) + xx as usize;
            assert!(g.cells[i].is_none(), "cell ({xx}, {yy}) already taken");
            g.cells[i] = Some(UnitId(id));
        }
    }
}

/// Fills an empty `gw` × `gh` grid with copies of a `w` × `h` item through
/// the §2.3 search; pushes a line for every broken invariant (the sweep's
/// body).
fn fill_check(gw: u8, gh: u8, w: u8, h: u8, player: bool, failures: &mut Vec<String>) {
    let mut g = Grid::new(gw, gh);
    if w == 0 || h == 0 {
        if search(&g, w, h, player).is_some() {
            failures.push(format!("{w}x{h} in {gw}x{gh}: zero size placed"));
        }
        return;
    }
    let mut placed = 0u32;
    loop {
        let fit = any_fit(&g, w, h);
        match search(&g, w, h, player) {
            Some((x, y)) => {
                let ok = in_bounds(&g, x, y, w, h)
                    && fits(&g, x, y, w, h)
                    && (!player || weight(&g, x, y, w, h) > 0);
                if !ok || !fit {
                    failures.push(format!(
                        "{w}x{h} in {gw}x{gh} player {player}: bad spot ({x}, {y}) after {placed}"
                    ));
                    return;
                }
                placed += 1;
                mark(&mut g, x, y, w, h, placed);
            }
            None => {
                if fit {
                    failures.push(format!(
                        "{w}x{h} in {gw}x{gh} player {player}: none with a fit after {placed}"
                    ));
                }
                break;
            }
        }
    }
    if placed == 0 && w <= gw && h <= gh {
        failures.push(format!(
            "{w}x{h} in {gw}x{gh} player {player}: never placed"
        ));
    }
}

#[test]
fn fill_check_on_the_spec_grid_sizes() {
    // Runs in CI: the sweep's body on the §1.3 grid sizes and item sizes
    // up to 3 × 5 reports nothing, and reports an item that cannot fit.
    let mut failures = Vec::new();
    for (gw, gh) in [(10, 4), (10, 10), (6, 4), (3, 4), (6, 8)] {
        for w in 0..=3 {
            for h in 0..=5 {
                for player in [true, false] {
                    fill_check(gw, gh, w, h, player, &mut failures);
                }
            }
        }
    }
    assert_eq!(failures, Vec::<String>::new());
    let mut g = Grid::new(3, 4);
    mark(&mut g, 0, 0, 1, 1, 1);
    assert!(any_fit(&g, 2, 4));
    assert!(!any_fit(&g, 3, 4));
}

/// Every item size is checked against free-position search on server grids.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_every_item_size_places() {
    // Every weapons / armor / misc size (the search reads only w × h, so
    // each distinct size stands for its items) × every distinct server grid
    // (records 0–15 with a size) × player / other: filling an empty grid
    // with copies of the item, each search result is in bounds, free and
    // fits (§2.1); a player result has weight > 0; the search fails
    // exactly when no fitting cell is left (§2.3, edge case 9). Items with
    // a zero size never place (§2.2).
    let t = tables();
    let mut sizes: Vec<(u8, u8)> = (0..16)
        .map(|r| (t.grids[r].grid_x, t.grids[r].grid_y))
        .filter(|&(w, h)| w > 0 && h > 0)
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    let mut item_sizes: Vec<(u8, u8)> = t.items.iter().map(|r| (r.invwidth, r.invheight)).collect();
    let zero = item_sizes
        .iter()
        .filter(|&&(w, h)| w == 0 || h == 0)
        .count();
    let not_page0 = t
        .items
        .iter()
        .filter(|r| r.invwidth > 10 || r.invheight > 4)
        .map(|r| code(r.code))
        .collect::<Vec<_>>();
    item_sizes.sort_unstable();
    item_sizes.dedup();
    let mut failures = Vec::new();
    for &(gw, gh) in &sizes {
        for &(w, h) in &item_sizes {
            for player in [true, false] {
                fill_check(gw, gh, w, h, player, &mut failures);
            }
        }
    }
    println!(
        "{} items, {} distinct sizes {:?}, {zero} with a zero size, larger than 10 x 4: {:?}; grids {:?}",
        t.items.len(),
        item_sizes.len(),
        item_sizes,
        not_page0,
        sizes
    );
    let n = t.items.len();
    let belt_1x1 = (0..n)
        .filter(|&r| beltable(t, r) && t.size(r) == Some((1, 1)))
        .count();
    println!(
        "beltable items: {} ({} of them 1 x 1)",
        (0..n).filter(|&r| beltable(t, r)).count(),
        belt_1x1
    );
    failures.truncate(20);
    assert!(failures.is_empty(), "{failures:#?}");
}

// ------------------------------------------------------------ path tables

const PATH_TSV: &str = include_str!("../../../tables/sim/path-tables.tsv");
const TSV_HEADER: &str = "table\tindex\ta\tb\tc\td\te\tva";

/// One TSV row: table, index, the values a–e present, address.
struct TsvRow {
    line: usize,
    table: String,
    index: u32,
    vals: Vec<i64>,
    va: u32,
}

fn tsv_rows(tsv: &str) -> Vec<TsvRow> {
    let mut lines = tsv.lines();
    assert_eq!(lines.next(), Some(TSV_HEADER));
    lines
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            let c: Vec<&str> = l.split('\t').collect();
            assert_eq!(c.len(), 8, "line {}", i + 2);
            TsvRow {
                line: i + 2,
                table: c[0].to_string(),
                index: c[1].parse().unwrap(),
                vals: c[2..7]
                    .iter()
                    .filter(|v| !v.is_empty())
                    .map(|v| v.parse().unwrap_or_else(|e| panic!("line {}: {e}", i + 2)))
                    .collect(),
                va: u32::from_str_radix(c[7].trim_start_matches("0x"), 16).unwrap(),
            }
        })
        .collect()
}

/// Element size by table (`pathing.md` Constants: `altdir` bytes, the rest
/// 32-bit signed).
fn elem_size(table: &str) -> u32 {
    if table == "altdir" {
        1
    } else {
        4
    }
}

/// A PE file image: reads by virtual address, zero past a section's raw
/// data (as the loader maps it).
struct PeImage {
    data: Vec<u8>,
    base: u32,
    /// (rva, virtual span, raw offset, raw size)
    secs: Vec<(u32, u32, u32, u32)>,
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl PeImage {
    fn parse(data: Vec<u8>) -> Self {
        assert_eq!(&data[..2], b"MZ");
        let pe = u32_at(&data, 0x3C) as usize;
        assert_eq!(&data[pe..pe + 4], b"PE\0\0");
        let nsec = usize::from(u16_at(&data, pe + 6));
        let optsz = usize::from(u16_at(&data, pe + 20));
        let base = u32_at(&data, pe + 24 + 28);
        let off = pe + 24 + optsz;
        let secs = (0..nsec)
            .map(|i| {
                let s = off + 40 * i + 8;
                let (vsz, va, rsz, raw) = (
                    u32_at(&data, s),
                    u32_at(&data, s + 4),
                    u32_at(&data, s + 8),
                    u32_at(&data, s + 12),
                );
                (va, vsz.max(rsz), raw, rsz)
            })
            .collect();
        Self { data, base, secs }
    }

    fn byte(&self, va: u32) -> Option<u8> {
        let rva = va.checked_sub(self.base)?;
        let &(sva, _, raw, rsz) = self
            .secs
            .iter()
            .find(|&&(sva, span, _, _)| sva <= rva && rva < sva + span)?;
        let o = rva - sva;
        Some(if o < rsz {
            self.data[(raw + o) as usize]
        } else {
            0
        })
    }

    fn read(&self, va: u32, size: u32) -> Option<i64> {
        let mut b = [0u8; 4];
        for i in 0..size {
            b[i as usize] = self.byte(va + i)?;
        }
        Some(match size {
            1 => i64::from(b[0]),
            _ => i64::from(i32::from_le_bytes(b)),
        })
    }
}

/// Rows whose values differ from the image (or whose layout is off).
fn path_mismatches(img: &PeImage, rows: &[TsvRow]) -> Vec<String> {
    let mut bad = Vec::new();
    let mut base: Vec<(&str, u32, usize)> = Vec::new();
    for r in rows {
        let sz = elem_size(&r.table);
        let k = r.vals.len().max(1);
        // Rows of a table are contiguous: va = first va + index · k · size.
        match base.iter().find(|(t, _, _)| *t == r.table) {
            Some(&(_, va0, k0)) => {
                if k != k0 || r.va != va0 + r.index * k0 as u32 * sz {
                    bad.push(format!("line {}: {} layout", r.line, r.table));
                }
            }
            None => {
                if r.index != 0 {
                    bad.push(format!(
                        "line {}: {} starts at {}",
                        r.line, r.table, r.index
                    ));
                }
                base.push((&r.table, r.va, k));
            }
        }
        let got: Vec<Option<i64>> = (0..k as u32).map(|j| img.read(r.va + j * sz, sz)).collect();
        if got.iter().zip(&r.vals).any(|(g, v)| *g != Some(*v)) {
            bad.push(format!(
                "line {}: {} {} tsv {:?} != exe {:?} at 0x{:08X}",
                r.line, r.table, r.index, r.vals, got, r.va
            ));
        }
    }
    bad
}

#[allow(clippy::disallowed_methods)]
fn game_exe() -> PeImage {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let path = format!("{dir}/Game.exe");
    PeImage::parse(std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

/// The table names and row counts `pathing.md` Constants lists.
const PATH_TABLES: [(&str, usize); 19] = [
    ("pathtype_flags", 18),
    ("pathtype_diroff", 18),
    ("pattern_of_size", 4),
    ("dir8_toward", 8),
    ("dir8_target", 8),
    ("testdir", 25),
    ("altdir", 25),
    ("dist8_path", 64),
    ("dist8_unit", 64),
    ("snap9", 81),
    ("tan", 128),
    ("tan_before", 127),
    ("dirdiff", 64),
    ("field_dx", 9),
    ("field_dy", 9),
    ("velmod_player", 20),
    ("velmod_monster", 16),
    ("velmod_monster_x", 16),
    ("animstat", 5),
];

#[test]
fn path_tsv_shape_and_mismatch_report() {
    // Runs in CI (no game files): the TSV has the tables and row counts
    // the spec lists (709 rows), and the comparison reports exactly a
    // changed row on an image built from the TSV itself (M08).
    let rows = tsv_rows(PATH_TSV);
    assert_eq!(rows.len(), 709);
    for (name, n) in PATH_TABLES {
        assert_eq!(rows.iter().filter(|r| r.table == name).count(), n, "{name}");
    }
    // A one-section image at the base below holding every row's values.
    let base = 0x0040_0000u32;
    let lo = rows.iter().map(|r| r.va).min().unwrap() - base;
    let hi = rows.iter().map(|r| r.va + 20).max().unwrap() - base;
    let mut data = vec![0u8; 0x200];
    data[..2].copy_from_slice(b"MZ");
    data[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    data[0x40..0x44].copy_from_slice(b"PE\0\0");
    data[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
    data[0x54..0x56].copy_from_slice(&0xE0u16.to_le_bytes());
    data[0x58 + 28..0x58 + 32].copy_from_slice(&base.to_le_bytes());
    let s = 0x58 + 0xE0 + 8;
    let raw = 0x200u32;
    for (o, v) in [(0, hi - lo), (4, lo), (8, hi - lo), (12, raw)] {
        data[s + o..s + o + 4].copy_from_slice(&v.to_le_bytes());
    }
    data.resize((raw + hi - lo) as usize, 0);
    for r in &rows {
        let sz = elem_size(&r.table);
        for (j, v) in r.vals.iter().enumerate() {
            let at = (raw + r.va - base - lo + j as u32 * sz) as usize;
            let bytes = (*v as i32).to_le_bytes();
            data[at..at + sz as usize].copy_from_slice(&bytes[..sz as usize]);
        }
    }
    let img = PeImage::parse(data);
    assert_eq!(path_mismatches(&img, &rows), Vec::<String>::new());
    let pert = PATH_TSV.replacen("snap9\t40\t", "snap9\t40\t7", 1);
    assert_ne!(pert, PATH_TSV);
    let bad = path_mismatches(&img, &tsv_rows(&pert));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(
        bad[0].starts_with("line ") && bad[0].contains("snap9 40"),
        "{bad:?}"
    );
}

/// Compares path-table TSV rows and D2RUST field offsets with Game.exe.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_path_tables_equal_game_exe() {
    // Compare every TSV row with executable bytes at its virtual address,
    // reading zero past a section's raw data. This checks table contents;
    // it does not verify the executable's SHA-256.
    let img = game_exe();
    let rows = tsv_rows(PATH_TSV);
    let bad = path_mismatches(&img, &rows);
    assert!(
        bad.is_empty(),
        "{} mismatches: {:#?}",
        bad.len(),
        &bad[..bad.len().min(20)]
    );
    // D2RUST's copies of the field tables (`path-placement.md` Constants)
    // equal the same bytes.
    for (name, ours) in [("field_dx", FIELD_DX), ("field_dy", FIELD_DY)] {
        let va = rows.iter().find(|r| r.table == name).unwrap().va;
        let exe: Vec<i64> = (0..9).map(|i| img.read(va + 4 * i, 4).unwrap()).collect();
        let ours: Vec<i64> = ours.iter().map(|&v| i64::from(v)).collect();
        assert_eq!(ours, exe, "{name}");
    }
}
