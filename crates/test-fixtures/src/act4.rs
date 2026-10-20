//! An Act IV-shaped variant of the synthetic set (task `q-a4`), so
//! `Drlg::create(3, ..)` runs the real act placer
//! (Pandemonium Fortress, Outer Steppes, Plains of Despair, City of the
//! Damned; A4C: Chaos Sanctuary) and the Act IV generator on
//! made-up data.
//!
//! Same method as [`crate::act2`]. Fixed by the game's rules: the level ids and
//! DrlgTypes (103 / 108 preset, 104..106 outdoor), the sizes and offsets
//! of the reference rects (the Act IV
//! placement vector), lvlprest `Def` = row for every id the generator stamps
//! (up to 862). Made up: names, the River of Flame (107, not placed by
//! the act placer), the LevelType, every preset's content (floor) and
//! size (one 8 × 8 cell, the Chaos Sanctuary's 24 × 24 pieces), `Files`
//! counts, the lvlsub row, the tiles, the monster columns.

use d2_formats::ds1::Ds1;

use crate::act1::{clear, floor_preset, n, CELL, CELL_DS1, CELL_FILES};
use crate::content::{FLOOR_DT1, SUB_DS1, WALL_DT1};
use crate::drlg::archive_name;
use crate::synth::{synthetic, Synthetic, TableSet};

/// The Pandemonium Fortress, the A4 chain, the River of Flame and the A4C level.
pub const TOWN: u32 = 103;
pub const OUTER_STEPPES: u32 = 104;
pub const PLAINS_OF_DESPAIR: u32 = 105;
pub const CITY_OF_THE_DAMNED: u32 = 106;
pub const RIVER_OF_FLAME: u32 = 107;
pub const CHAOS_SANCTUARY: u32 = 108;
/// Rows of `levels`: ids 0 ..= 108 (Act IV starts at 103).
pub const LEVEL_COUNT: u32 = 109;
/// The Act IV outdoor LevelType (made up; the Steppes' border style).
pub const STEPPES: u32 = 16;
/// The outdoor levels whose generator places a waypoint.
pub const WAYPOINT_LEVELS: [u32; 2] = [CITY_OF_THE_DAMNED, RIVER_OF_FLAME];

/// One placer level: (id, DrlgType, LevelType, size, offset, vis).
type LevelRow = (u32, u32, u32, (i32, i32), (i32, i32), &'static [u32]);

/// The levels of Act IV (sizes / absolute offsets of the reference rects;
/// vis: the chain as the placer links it).
pub const PLACER_LEVELS: [LevelRow; 5] = [
    (103, 2, 1, (32, 24), (1000, 1000), &[104]),
    (104, 3, STEPPES, (80, 64), (0, 0), &[103, 105]),
    (105, 3, STEPPES, (64, 80), (0, 0), &[104, 106]),
    (106, 3, STEPPES, (80, 64), (0, 0), &[105]),
    (108, 3, STEPPES, (120, 120), (1500, 1000), &[]),
];

/// lvlprest: Def 0 none, Def 1 the town, Defs 2..=[`LAST_CELL_DEF`] the
/// one-cell presets.
pub const TOWN_DEF: u32 = 1;
pub const LAST_CELL_DEF: u32 = 862;
/// The Chaos Sanctuary's pieces (3 × 3 cells each).
pub const CHAOS_DEFS: [u32; 7] = [836, 857, 858, 859, 860, 861, 862];
pub const CHAOS_DS1: &str = "Synth\\Act4\\Chaos.ds1";
pub const TOWN_DS1: &str = "Synth\\Act4\\Town.ds1";
/// Levels-row `Waypoint` indices (made up; others 255).
pub const WAYPOINTS: [(u32, u32); 3] = [(TOWN, 27), (CITY_OF_THE_DAMNED, 28), (RIVER_OF_FLAME, 29)];

/// The Act IV-shaped synthetic set.
pub fn act4() -> Synthetic {
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
    format!("Synth A4 L{id}")
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
        let act = if id >= TOWN { "3" } else { "0" };
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
    for _ in 1..=STEPPES {
        t.row(
            "lvltypes",
            &[("File 1", FLOOR_DT1[0]), ("File 2", WALL_DT1), ("Act", "3")],
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
    preset(t, TOWN_DEF, TOWN, (32, 24), 0, &[TOWN_DS1; 4]);
    for def in TOWN_DEF + 1..=LAST_CELL_DEF {
        if CHAOS_DEFS.contains(&def) {
            preset(t, def, 0, (24, 24), CELL_FILES, &[CHAOS_DS1; 6]);
        } else {
            preset(t, def, 0, (CELL, CELL), CELL_FILES, &[CELL_DS1; 6]);
        }
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

/// The Pandemonium Fortress: 32 × 24 floor tiles.
pub fn town() -> Ds1 {
    let mut d = floor_preset(32, 24);
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

/// Every DRLG file of the Act IV set.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let presets = [
        (TOWN_DS1, town()),
        (CELL_DS1, floor_preset(CELL, CELL)),
        (CHAOS_DS1, floor_preset(24, 24)),
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
