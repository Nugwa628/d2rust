//! An Act II-shaped variant of the synthetic set (task `q-a2-fields`), so
//! `Drlg::create(1, ..)` runs the real act placer
//! (Lut Gholein, Rocky Waste, Dry Hills, Far Oasis, Lost City, Valley of
//! Snakes; A2C: Canyon of the Magi) and the Act II outdoor generator
//! on made-up data.
//!
//! Same method as [`crate::act1`]. Fixed by the game's rules: the level ids and
//! their DrlgTypes (40 preset, 41..46 outdoor), the desert LevelType 16
//! (border style 2), the sizes and the absolute offsets of levels 40 and
//! 46 (the reference rects of the Act II
//! placement vector), lvlprest `Def` = row for every id the generator stamps
//! (362..413). Made up: names, every preset's content (one 8 × 8 floor
//! cell), `Files` counts, the lvlsub row, the tiles, the monster columns.

use d2_formats::ds1::Ds1;

use crate::act1::{clear, floor_preset, n, CELL, CELL_DS1, CELL_FILES};
use crate::content::{FLOOR_DT1, SUB_DS1, WALL_DT1};
use crate::drlg::archive_name;
use crate::synth::{synthetic, Synthetic, TableSet};

/// Lut Gholein, the A2 chain and the A2C level.
pub const TOWN: u32 = 40;
pub const ROCKY_WASTE: u32 = 41;
pub const DRY_HILLS: u32 = 42;
pub const FAR_OASIS: u32 = 43;
pub const LOST_CITY: u32 = 44;
pub const VALLEY_OF_SNAKES: u32 = 45;
pub const CANYON_OF_THE_MAGI: u32 = 46;
/// Rows of `levels`: ids 0 ..= 46 (Act II starts at 40).
pub const LEVEL_COUNT: u32 = 47;
/// The Act II desert's LevelType (16 → border style 2).
pub const DESERT: u32 = 16;
/// The outdoor levels whose generator places a waypoint.
pub const WAYPOINT_LEVELS: [u32; 3] = [DRY_HILLS, FAR_OASIS, LOST_CITY];

/// One placer level: (id, DrlgType, LevelType, size, offset, vis).
type LevelRow = (u32, u32, u32, (i32, i32), (i32, i32), &'static [u32]);

/// The levels of Act II (sizes / absolute offsets of the reference rects;
/// vis: the chain as the placer links it).
pub const PLACER_LEVELS: [LevelRow; 7] = [
    (40, 2, 1, (56, 56), (1000, 1000), &[41]),
    (41, 3, DESERT, (80, 80), (0, 0), &[40, 42]),
    (42, 3, DESERT, (80, 80), (0, 0), &[41, 43]),
    (43, 3, DESERT, (80, 80), (0, 0), &[42, 44]),
    (44, 3, DESERT, (80, 80), (0, 0), &[43, 45]),
    (45, 3, DESERT, (32, 32), (0, 0), &[44]),
    (46, 3, DESERT, (80, 80), (2500, 1000), &[]),
];

/// lvlprest: Def 0 none, Def 1 the town, Defs 2..=[`LAST_CELL_DEF`] the
/// one-cell presets.
pub const TOWN_DEF: u32 = 1;
pub const LAST_CELL_DEF: u32 = 413;
pub const TOWN_DS1: &str = "Synth\\Act2\\Town.ds1";
/// Levels-row `Waypoint` indices (made up; others 255).
pub const WAYPOINTS: [(u32, u32); 4] =
    [(TOWN, 9), (DRY_HILLS, 10), (FAR_OASIS, 11), (LOST_CITY, 12)];

/// The Act II-shaped synthetic set.
pub fn act2() -> Synthetic {
    let mut s = synthetic();
    for t in ["levels", "lvlprest", "lvltypes", "lvlsub", "lvlmaze"] {
        clear(&mut s.tables, t);
    }
    levels(&mut s);
    tables(&mut s.tables);
    s.files = files();
    s
}

/// The level name of id `id`.
pub fn level_name(id: u32) -> String {
    format!("Synth A2 L{id}")
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
        let wp = WAYPOINTS.iter().find(|w| w.0 == id).map_or(255, |w| w.1);
        let act = if id >= TOWN { "1" } else { "0" };
        let mut cells: Vec<(String, String)> = vec![
            ("Id".into(), n(id)),
            ("Act".into(), act.into()),
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
        for c in ["SubType", "SubTheme", "SubWaypoint", "SubShrine"] {
            cells.push((c.into(), "-1".into()));
        }
        for k in 0..8 {
            cells.push((format!("Vis{k}"), n(vis.get(k).copied().unwrap_or(0))));
            cells.push((format!("Warp{k}"), "-1".into()));
        }
        if drlg == 3 {
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
    // lvltypes: 0 none, 1 town and preset levels, 2.. the rest; the desert
    // type 16 needs its own row.
    t.row("lvltypes", &[("Act", "0")]);
    for _ in 1..=DESERT {
        t.row(
            "lvltypes",
            &[("File 1", FLOOR_DT1[0]), ("File 2", WALL_DT1), ("Act", "1")],
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
    preset(t, TOWN_DEF, TOWN, (56, 56), 0, &[TOWN_DS1; 4]);
    for def in TOWN_DEF + 1..=LAST_CELL_DEF {
        preset(t, def, 0, (CELL, CELL), CELL_FILES, &[CELL_DS1; 6]);
    }
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

/// Lut Gholein: 56 × 56 floor tiles.
pub fn town() -> Ds1 {
    let mut d = floor_preset(56, 56);
    let (kind, id, x, y) = crate::act1::TOWN_WAYPOINT;
    d.objects.push(d2_formats::ds1::Ds1Object {
        kind,
        id,
        x,
        y,
        flags: 0,
    });
    d
}

/// Every DRLG file of the Act II set.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let presets = [(TOWN_DS1, town()), (CELL_DS1, floor_preset(CELL, CELL))];
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
