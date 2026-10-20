//! An Act V-shaped variant of the synthetic set (task `q-a5-fields`), so
//! `Drlg::create(4, ..)` runs the real Act V placement (siege strip, the
//! B1 / B2 / BD linkers) and the Act V generator (barricade border walk,
//! ravine, entrances, caves, siege connection, type-12 border
//! substitution, prisons, special presets) on made-up data.
//!
//! Same method as [`crate::act3`]. Fixed by the game's rules: the level ids and
//! DrlgTypes (109 preset, 110 / 111 / 112 / 117 outdoor, 134 outdoor and
//! 136 preset for the A5U driver), LevelTypes 29 / 30 / 31, the sizes
//! (111 and 112 −1, 117 overridden), the
//! offsets, the lvlprest ids the generator stamps (865.. siege strip, the
//! barricade / ravine / snow rows, entrances 907..910, caves, 915.. prison
//! styles). Made up: names, every preset's size (2 × 2 cells, the siege
//! strip 2 × 6) and content (floor), the tiles, the monster columns, and
//! the type-12 substitution file (three groups giving the three prison
//! cells the siege generator needs).

use d2_formats::ds1::{Ds1, Ds1Group, Ds1Object};

use crate::act1::{clear, floor_preset, n};
use crate::content::{FLOOR_DT1, WALL_DT1};
use crate::drlg::archive_name;
use crate::dt1::{file, tile, write};
use crate::synth::{synthetic, Synthetic, TableSet};

pub const TOWN: u32 = 109;
pub const BLOODY_FOOTHILLS: u32 = 110;
pub const FRIGID_HIGHLANDS: u32 = 111;
pub const ARREAT_PLATEAU: u32 = 112;
pub const FROZEN_TUNDRA: u32 = 117;
/// Rows of `levels`: ids 0 ..= 136 (the A5U driver allocates 134 and 136).
pub const LEVEL_COUNT: u32 = 137;
/// LevelTypes.
pub const TOWN_TYPE: u32 = 29;
pub const FOOTHILLS_TYPE: u32 = 30;
pub const SNOW_TYPE: u32 = 31;
/// The outdoor levels the player can walk (111 and 112 meet; 117 joins
/// only through caves and warps).
pub const OUTDOOR: [u32; 4] = [
    BLOODY_FOOTHILLS,
    FRIGID_HIGHLANDS,
    ARREAT_PLATEAU,
    FROZEN_TUNDRA,
];

/// lvlprest: the town row, the Act V preset rows (865 ..= LAST_DEF).
pub const TOWN_DEF: u32 = 864;
pub const SIEGE_FIRST: u32 = 865;
pub const SIEGE_LAST: u32 = 879;
/// The preset level the A5U driver allocates.
pub const HALL: u32 = 136;
pub const HALL_DEF: u32 = 863;
pub const LAST_DEF: u32 = 990;
/// A level's preset size in tiles: 2 × 2 cells.
pub const PIECE: u32 = 16;
pub const TOWN_DS1: &str = "Synth\\Act5\\Town.ds1";
pub const PIECE_DS1: &str = "Synth\\Act5\\Piece.ds1";
pub const HALL_DS1: &str = "Synth\\Act5\\Hall.ds1";
pub const STRIP_DS1: &str = "Synth\\Act5\\Strip.ds1";
pub const BARRICADE_SUB_DS1: &str = "Synth\\Act5\\Barricade.ds1";
/// Levels-row `Waypoint` indices (made up; others 255).
pub const WAYPOINTS: [(u32, u32); 3] = [(TOWN, 27), (FRIGID_HIGHLANDS, 28), (ARREAT_PLATEAU, 29)];

/// The Act V-shaped synthetic set.
pub fn act5() -> Synthetic {
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
    format!("Synth A5 L{id}")
}

/// (id, DrlgType, LevelType, size, offset) of the placed levels. 111 and
/// 112 have size −1 (the linker sizes them), 117's size is overridden.
type LevelRow = (u32, u32, u32, (i32, i32), (i32, i32));

const PLACED_LEVELS: [LevelRow; 7] = [
    (109, 2, TOWN_TYPE, (40, 40), (1000, 1000)),
    (110, 3, FOOTHILLS_TYPE, (240, 48), (760, 1000)),
    (111, 3, SNOW_TYPE, (-1, -1), (-1, -1)),
    (112, 3, SNOW_TYPE, (-1, -1), (-1, -1)),
    (117, 3, SNOW_TYPE, (128, 80), (2000, 1896)),
    (134, 3, SNOW_TYPE, (40, 40), (8000, 8000)),
    (136, 2, SNOW_TYPE, (40, 40), (9000, 9000)),
];

fn levels(s: &mut Synthetic) {
    for id in 0..LEVEL_COUNT {
        let row = PLACED_LEVELS.iter().find(|r| r.0 == id);
        let name = level_name(id);
        s.strings.base.push((name.clone(), name.clone()));
        let (drlg, ltype, size, offset) = match row {
            Some(&(_, d, l, s, o)) => (d, l, s, o),
            None => (0, 0, (0, 0), (0, 0)),
        };
        let wp = WAYPOINTS.iter().find(|w| w.0 == id).map_or(255, |w| w.1);
        let act = match id {
            109.. => "4",
            75.. => "2",
            40.. => "1",
            _ => "0",
        };
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
        // No lvlsub on the Act V levels: the barricade substitution is
        // type 12, found by the generator, not by `SubType`.
        for c in ["SubType", "SubTheme", "SubWaypoint", "SubShrine"] {
            cells.push((c.into(), "-1".into()));
        }
        for k in 0..8 {
            cells.push((format!("Vis{k}"), "0".into()));
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

/// (size x, y, DS1) of lvlprest row `def`.
fn piece(def: u32) -> (u32, u32, &'static str) {
    match def {
        SIEGE_FIRST..=SIEGE_LAST => (PIECE, 3 * PIECE, STRIP_DS1),
        _ => (PIECE, PIECE, PIECE_DS1),
    }
}

fn tables(t: &mut TableSet) {
    // lvltypes: 0 none, 1..=31 the rest. Types 30 and 31 read File 1 and
    // File 5 (DT1 mask 0x11); 2 is the wall file.
    t.row("lvltypes", &[("Act", "0")]);
    for _ in 1..=SNOW_TYPE {
        t.row(
            "lvltypes",
            &[
                ("File 1", FLOOR_DT1[0]),
                ("File 2", WALL_DT1),
                ("File 3", FLOOR_DT1[1]),
                ("File 5", FLOOR_DT1[1]),
                ("Act", "4"),
            ],
        );
    }
    t.row("lvlprest", &[("Def", "0")]);
    let preset = |t: &mut TableSet, def: u32, level: u32, size: (u32, u32), ds1: &str| {
        // Frozen Tundra's special row 955 / 956 picks file 1 (F 1).
        let files = if matches!(def, 955 | 956) { "2" } else { "1" };
        let (def, level, sx, sy) = (n(def), n(level), n(size.0), n(size.1));
        t.row(
            "lvlprest",
            &[
                ("Def", def.as_str()),
                ("LevelId", level.as_str()),
                ("Populate", "1"),
                ("SizeX", sx.as_str()),
                ("SizeY", sy.as_str()),
                ("Files", files),
                ("Dt1Mask", "1"),
                ("File1", ds1),
                ("File2", ds1),
            ],
        );
    };
    for def in 1..HALL_DEF {
        preset(t, def, 0, (PIECE, PIECE), PIECE_DS1);
    }
    preset(t, HALL_DEF, HALL, (40, 40), HALL_DS1);
    t.row(
        "lvlprest",
        &[
            ("Def", &n(TOWN_DEF)),
            ("LevelId", &n(TOWN)),
            ("Populate", "1"),
            ("SizeX", "40"),
            ("SizeY", "40"),
            ("Files", "1"),
            ("Dt1Mask", "1"),
            ("File1", TOWN_DS1),
        ],
    );
    for def in TOWN_DEF + 1..=LAST_DEF {
        let (sx, sy, ds1) = piece(def);
        preset(t, def, 0, (sx, sy), ds1);
    }
    // Type 12: the barricade border substitution. One row; BordType 1 =
    // one replacement per group, and the file has three groups.
    t.row(
        "lvlsub",
        &[
            ("Type", "12"),
            ("File", BARRICADE_SUB_DS1),
            ("BordType", "1"),
            ("GridSize", "2"),
            ("Dt1Mask", "1"),
        ],
    );
}

/// Harrogath: 40 × 40 floor tiles and the town waypoint.
pub fn town() -> Ds1 {
    let mut d = floor_preset(40, 40);
    let (kind, id, x, y) = crate::act1::TOWN_WAYPOINT;
    d.objects.push(Ds1Object {
        kind,
        id,
        x,
        y,
        flags: 0,
    });
    d
}

/// Wall cell of the barricade style map:
/// style 48 value 31 matches any cell, style 49 value `v` (1..=16) is the
/// prison piece 914 + `v`.
const fn wall(style: u32, v: u32) -> u32 {
    (style << 20) | (v << 8) | 1
}

/// The type-12 substitution file: three 1 × 1 groups, each with a test
/// cell that matches anywhere (column 0) and one variant (column 2) that
/// stamps prison piece 915 + group.
pub fn barricade_sub() -> Ds1 {
    let mut d = crate::ds1::blank(9, 9, 0);
    d.tag_type = 2;
    d.tags = Some(vec![0; 81]);
    for k in 0..3u32 {
        d.walls[0][(k * 9) as usize] = wall(48, 31);
        d.walls[0][(k * 9 + 2) as usize] = wall(49, 1 + k);
        d.groups.push(Ds1Group {
            x: 0,
            y: k,
            width: 1,
            height: 1,
            // Variant count N (`sub_file`: the group's last value).
            unknown: 1,
        });
    }
    d
}

/// Every DRLG file of the Act V set.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let presets = [
        (TOWN_DS1, town()),
        (PIECE_DS1, floor_preset(PIECE, PIECE)),
        (HALL_DS1, floor_preset(40, 40)),
        (STRIP_DS1, floor_preset(PIECE, 3 * PIECE)),
        (BARRICADE_SUB_DS1, barricade_sub()),
    ];
    let mut out: Vec<(String, Vec<u8>)> = presets
        .iter()
        .map(|(p, d)| (archive_name(p), crate::ds1::write(d)))
        .collect();
    // Floor flags 0x600000 on 117 (main index 6) and 0 elsewhere.
    let floors = file(vec![tile(0, 0, 0, 1), tile(0, 1, 0, 1), tile(0, 6, 0, 1)]);
    out.extend(
        crate::drlg::files()
            .into_iter()
            .filter(|(name, _)| !name.ends_with(".ds1"))
            .map(|(name, bytes)| {
                if FLOOR_DT1.iter().any(|f| name == archive_name(f)) {
                    (name, write(&floors))
                } else {
                    (name, bytes)
                }
            }),
    );
    out
}
