//! An Act I-shaped variant of the synthetic set, so `Drlg::create` runs
//! the real act placer (A1W, A1M, neighbour
//! entries 1..17) and the Act I outdoor generator on made-up data.
//!
//! [`act1`] is [`crate::synth::synthetic`] with the level-type tables
//! replaced (`levels`, `lvlprest`, `lvltypes`, `lvlsub`, `lvlmaze`) and
//! the DRLG files rewritten ([`files`]). Every other table is the base
//! set's. What the game's rules fix, and is therefore not made up:
//!
//! - **Level ids and types.** The placer links levels by their D2RUST ids
//!   (1, 2, 3, 4, 17 on A1W; 39, 26, 7, 6, 5 on A1M; 27 at the
//!   Black Marsh row) and branches on DrlgType (2 preset, 3 outdoor);
//!   the generator picks the Act I border style and DT1 mask for LevelType 2.
//!   So levels 1..39 are act 0, the placer's levels have those types, and
//!   every outdoor level is LevelType 2.
//! - **Sizes and offsets of the placer's levels**: the reference
//!   placement vector (the derived rects), so the
//!   synthetic act creation can be compared with it. The town's 56 × 40
//!   also matches the dirt-path start offsets of the town exits.
//! - **lvlprest Def = row** for the ids the generator
//!   stamps (borders 4..15, cliffs 16..23, transitions 2 / 3, river 26 /
//!   27, bridge 28, caves 24 / 25 / 51 / 52, the special cells), all up
//!   to 163; the preset levels 1, 26, 27 found by `LevelId`.
//!
//! Made up: the names, every preset's content (one 8 × 8-tile floor
//! grid, [`CELL_DS1`]), `Files` counts other than the town's 0, the
//! vis links beyond the chain, the lvlsub row, the tiles.
//!
//! Levels 8..38 outside the chain are placeholders (DrlgType 0,
//! never generated); no vis or warp names them.

use d2_formats::ds1::{Ds1, Ds1Object};

use crate::content::{FLOOR_DT1, SUB_DS1, WALL_DT1};
use crate::drlg::{archive_name, FLOOR_CELL, WAYPOINT_DS1_ID};
use crate::synth::{synthetic, Synthetic, TableSet};

/// Rogue Encampment, Blood Moor, Cold Plains, Stony Field (the A1W chain).
pub const TOWN: u32 = 1;
pub const BLOOD_MOOR: u32 = 2;
pub const COLD_PLAINS: u32 = 3;
/// Levels of act 0: ids 0 ("Null") ..= 39 (Act II starts at 40).
pub const LEVEL_COUNT: u32 = 40;

/// One placer level: (id, DrlgType, LevelType, size, offset, vis).
type LevelRow = (u32, u32, u32, (i32, i32), (i32, i32), &'static [u32]);

/// The levels the Act I placer allocates
/// with the sizes and offsets of the reference placement vector. Vis links
/// (all warp −1, i.e. outdoor adjacency): the A1W chain as the
/// vector's neighbours (Blood Moor ↔ town, Cold Plains; Cold Plains ↔
/// Stony Field, Burial Grounds) and the A1M chain (Dark Wood ↔ Black
/// Marsh ↔ Tamoe Highland → Monastery Gate). Made-up choice: no cave,
/// tower or Tristram links.
pub const PLACER_LEVELS: [LevelRow; 11] = [
    (1, 2, 1, (56, 40), (0, 0), &[2]),
    (2, 3, 2, (56, 96), (0, 0), &[1, 3]),
    (3, 3, 2, (80, 80), (0, 0), &[2, 4, 17]),
    (4, 3, 2, (80, 80), (1000, 1000), &[3]),
    (5, 3, 2, (80, 80), (0, 0), &[6]),
    (6, 3, 2, (80, 80), (0, 0), &[5, 7]),
    (7, 3, 2, (80, 80), (0, 0), &[6, 26]),
    (17, 3, 2, (40, 48), (0, 0), &[3]),
    (26, 2, 1, (40, 18), (3000, 1000), &[7]),
    (27, 2, 1, (40, 40), (0, 0), &[]),
    (39, 3, 2, (64, 64), (5000, 1148), &[]),
];

/// The town DS1 (lvlprest row [`TOWN_DEF`], `File1`..`File4`: one file
/// per direction; the same content).
pub const TOWN_DS1: &str = "Synth\\Act1\\Town.ds1";
/// The one-cell preset every stamped lvlprest id uses.
pub const CELL_DS1: &str = "Synth\\Act1\\Cell.ds1";
/// The preset levels 26 and 27.
pub const GATE_DS1: &str = "Synth\\Act1\\Gate.ds1";
pub const CLOISTER_DS1: &str = "Synth\\Act1\\Cloister.ds1";

/// lvlprest rows: Def 0 none, Def 1 the town, Defs 2..=163 one-cell
/// presets, then the preset levels 26 and 27.
pub const TOWN_DEF: u32 = 1;
pub const LAST_CELL_DEF: u32 = 163;
pub const GATE_DEF: u32 = 164;
pub const CLOISTER_DEF: u32 = 165;
/// Tiles per outdoor cell, the one-cell preset's size.
pub const CELL: u32 = 8;
/// `Files` of the one-cell presets; `File1`..`File6` all name
/// [`CELL_DS1`], so every file index the generator
/// can pick names a file.
pub const CELL_FILES: u32 = 4;

/// The town waypoint record (type 2, id, sub-tile x, y): the middle of
/// tile (28, 20).
pub const TOWN_WAYPOINT: (u32, u32, u32, u32) = (2, WAYPOINT_DS1_ID, 142, 102);
/// The levels-row `Waypoint` index of the town and of the Cold Plains
/// (made up; the others 255).
pub const WAYPOINTS: [(u32, u32); 2] = [(TOWN, 0), (COLD_PLAINS, 1)];

pub(crate) fn n(v: impl ToString) -> String {
    v.to_string()
}

/// The Act I-shaped synthetic set.
pub fn act1() -> Synthetic {
    let mut s = synthetic();
    for t in ["levels", "lvlprest", "lvltypes", "lvlsub", "lvlmaze"] {
        clear(&mut s.tables, t);
    }
    levels(&mut s);
    tables(&mut s.tables);
    s.files = files();
    s
}

pub(crate) fn clear(t: &mut TableSet, txt: &str) {
    t.files
        .get_mut(&format!("{txt}.txt"))
        .unwrap_or_else(|| panic!("{txt}.txt is compiled"))
        .rows
        .clear();
}

/// The level name of id `id`.
pub fn level_name(id: u32) -> String {
    format!("Synth A1 L{id}")
}

fn levels(s: &mut Synthetic) {
    for id in 0..LEVEL_COUNT {
        let row = PLACER_LEVELS.iter().find(|r| r.0 == id);
        let name = level_name(id);
        s.strings.base.push((name.clone(), name.clone()));
        let (drlg, ltype, size, offset, vis) = match row {
            Some(&(_, d, l, s, o, v)) => (d, l, s, o, v),
            None => (0, 0, (0, 0), (0, 0), &[][..]),
        };
        let outdoor = drlg == 3;
        let wp = WAYPOINTS.iter().find(|w| w.0 == id).map_or(255, |w| w.1);
        let mut cells: Vec<(String, String)> = vec![
            ("Id".into(), n(id)),
            ("Act".into(), "0".into()),
            ("LevelName".into(), name.clone()),
            ("LevelWarp".into(), name.clone()),
            ("EntryFile".into(), name),
            ("Waypoint".into(), n(wp)),
            ("IsInside".into(), "0".into()),
            ("DrlgType".into(), n(drlg)),
            ("LevelType".into(), n(ltype)),
            ("OffsetX".into(), n(offset.0)),
            ("OffsetY".into(), n(offset.1)),
            ("Layer".into(), n(id)),
        ];
        for sfx in ["", "(N)", "(H)"] {
            cells.push((format!("SizeX{sfx}"), n(size.0)));
            cells.push((format!("SizeY{sfx}"), n(size.1)));
        }
        // No sub-theme, waypoint or shrine substitution rows:
        // −1.
        for c in ["SubType", "SubTheme", "SubWaypoint", "SubShrine"] {
            cells.push((c.into(), "-1".into()));
        }
        for k in 0..8 {
            cells.push((format!("Vis{k}"), n(vis.get(k).copied().unwrap_or(0))));
            cells.push((format!("Warp{k}"), "-1".into()));
        }
        if outdoor {
            for (c, v) in [
                ("MonLvl1", "2"),
                ("MonDen", "500"),
                ("NumMon", "2"),
                ("mon1", "beast1"),
                ("mon2", "ghoul1"),
                ("nmon1", "beast1"),
                ("umon1", "ghoul1"),
            ] {
                cells.push((c.into(), v.into()));
            }
        }
        let refs: Vec<(&str, &str)> = cells
            .iter()
            .map(|(c, v)| (c.as_str(), v.as_str()))
            .collect();
        s.tables.row("levels", &refs);
    }
}

fn tables(t: &mut TableSet) {
    // lvltypes: 0 none, 1 town and preset levels, 2 Act I wild.
    t.row("lvltypes", &[("Act", "0")]);
    for file in FLOOR_DT1 {
        t.row(
            "lvltypes",
            &[("File 1", file), ("File 2", WALL_DT1), ("Act", "0")],
        );
    }
    // lvlprest, Def = row.
    t.row("lvlprest", &[("Def", "0")]);
    let preset =
        |t: &mut TableSet, def: u32, level: u32, size: (u32, u32), files: u32, f: &[&str]| {
            let (def, level, sx, sy, files) = (n(def), n(level), n(size.0), n(size.1), n(files));
            let mut cells = vec![
                ("Def", def.as_str()),
                ("LevelId", level.as_str()),
                ("Populate", "1"),
                ("SizeX", sx.as_str()),
                ("SizeY", sy.as_str()),
                ("Files", files.as_str()),
                ("Dt1Mask", "1"),
            ];
            let names = ["File1", "File2", "File3", "File4", "File5", "File6"];
            cells.extend(names.iter().copied().zip(f.iter().copied()));
            t.row("lvlprest", &cells);
        };
    // The town: `Files` 0, the layout's direction picks File1..File4.
    preset(t, TOWN_DEF, TOWN, (56, 40), 0, &[TOWN_DS1; 4]);
    for def in TOWN_DEF + 1..=LAST_CELL_DEF {
        preset(t, def, 0, (CELL, CELL), CELL_FILES, &[CELL_DS1; 6]);
    }
    preset(t, GATE_DEF, 26, (40, 18), 1, &[GATE_DS1]);
    // Outer Cloister: three files (the Black Marsh row may replace the
    // direction).
    preset(t, CLOISTER_DEF, 27, (40, 40), 3, &[CLOISTER_DS1; 3]);
    // One lvlsub row per type the Act I generator substitutes with
    // (types 0..3 on levels 2..7 and 39).
    for ty in ["0", "1", "2", "3"] {
        t.row(
            "lvlsub",
            &[
                ("Type", ty),
                ("File", SUB_DS1),
                ("BordType", "1"),
                ("GridSize", "2"),
                ("Dt1Mask", "1"),
                ("Prob0", "50"),
                ("Trials0", "1"),
                ("Max0", "2"),
            ],
        );
    }
}

/// A floor-only preset of `w × h` tiles.
pub(crate) fn floor_preset(w: u32, h: u32) -> Ds1 {
    let mut d = crate::ds1::blank(w + 1, h + 1, 0);
    d.files = FLOOR_DT1
        .iter()
        .chain([&WALL_DT1])
        .map(|f| f.as_bytes().to_vec())
        .collect();
    d.floors[0].fill(FLOOR_CELL);
    d
}

/// The town: 56 × 40 floor tiles and the waypoint.
pub fn town() -> Ds1 {
    let mut d = floor_preset(56, 40);
    let (kind, id, x, y) = TOWN_WAYPOINT;
    d.objects.push(Ds1Object {
        kind,
        id,
        x,
        y,
        flags: 0,
    });
    d
}

/// Every DRLG file of the Act I set: the base set's lvlsub file, tiles
/// and fixed library, and the Act I presets.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let presets = [
        (TOWN_DS1, town()),
        (CELL_DS1, floor_preset(CELL, CELL)),
        (GATE_DS1, floor_preset(40, 18)),
        (CLOISTER_DS1, floor_preset(40, 40)),
    ];
    let mut out: Vec<(String, Vec<u8>)> = presets
        .iter()
        .map(|(p, d)| (archive_name(p), crate::ds1::write(d)))
        .collect();
    out.extend(
        crate::drlg::files()
            .into_iter()
            .filter(|(name, _)| !name.ends_with(".ds1") || *name == archive_name(SUB_DS1)),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_parse_and_sizes_match_the_rows() {
        let files = files();
        let get = |p: &str| {
            let name = archive_name(p);
            let (_, b) = files.iter().find(|f| f.0 == name).expect(p);
            Ds1::parse(b).unwrap()
        };
        // Stored size = lvlprest SizeX / SizeY.
        for (p, w, h) in [
            (TOWN_DS1, 56, 40),
            (CELL_DS1, CELL, CELL),
            (GATE_DS1, 40, 18),
            (CLOISTER_DS1, 40, 40),
        ] {
            let d = get(p);
            assert_eq!((d.width - 1, d.height - 1), (w, h), "{p}");
        }
        assert_eq!(get(TOWN_DS1), town());
        assert!(files.iter().any(|f| f.0 == archive_name(SUB_DS1)));
    }

    #[test]
    fn rows_cover_the_placer() {
        let s = act1();
        let levels = s.tables.file("levels.txt");
        assert_eq!(levels.rows.len(), LEVEL_COUNT as usize);
        let lp = s.tables.file("lvlprest.txt");
        assert_eq!(lp.rows.len(), CLOISTER_DEF as usize + 1);
        let def = lp.columns.iter().position(|c| c == "Def").unwrap();
        for (i, r) in lp.rows.iter().enumerate() {
            assert_eq!(r[def], i.to_string(), "Def = row");
        }
    }
}
