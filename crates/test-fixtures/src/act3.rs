//! An Act III-shaped variant of the synthetic set (task `q-a3-fields`), so
//! `Drlg::create(2, ..)` runs the real jungle placer and Kurast chain and
//! the Act III generator (jungle stamping, Kurast border rows, Travincal)
//! on made-up data. Task `q-a3-dungeons` adds levels 84..=102: the maze
//! dungeons (Spider, Swampy Pit, Flayer Dungeon, sewers, Durance 1 / 2)
//! and the one-cell preset temples and Durance 3.
//!
//! Same method as [`crate::act2`]. Fixed by the game's rules: the level ids and
//! DrlgTypes (75 preset, 76..83 outdoor), LevelTypes 20 / 21 / 22, the
//! sizes, the docks offset (1000, 1000),
//! lvlprest `Def` = row for 529..658 with the jungle generator's sizes and `Files`
//! of the jungle pieces (32 × 32 blocks, 64 × 32 head and tail). Made up:
//! names, the Kurast pieces' sizes (one 8 × 8 cell, except the causeway
//! 48 × 16 and the Travincal pieces 16 / 32 / 16 × 32), every preset's
//! content (floor), the tiles, the monster columns.

use d2_formats::ds1::{Ds1, Ds1Object};

use crate::act1::{clear, floor_preset, n, CELL, CELL_DS1, CELL_FILES};
use crate::content::{FLOOR_DT1, SUB_DS1, WALL_DT1};
use crate::drlg::archive_name;
use crate::dt1::{file, tile, write};
use crate::synth::{synthetic, Synthetic, TableSet};

/// Kurast Docks, the jungles, the Kurast chain.
pub const TOWN: u32 = 75;
pub const SPIDER_FOREST: u32 = 76;
pub const GREAT_MARSH: u32 = 77;
pub const FLAYER_JUNGLE: u32 = 78;
pub const LOWER_KURAST: u32 = 79;
pub const KURAST_BAZAAR: u32 = 80;
pub const UPPER_KURAST: u32 = 81;
pub const KURAST_CAUSEWAY: u32 = 82;
pub const TRAVINCAL: u32 = 83;
/// Rows of `levels`: ids 0 ..= 102 (83 is Travincal, 84.. the dungeons).
pub const LEVEL_COUNT: u32 = 103;
/// LevelTypes.
pub const DOCKS_TYPE: u32 = 20;
pub const JUNGLE_TYPE: u32 = 21;
pub const KURAST_TYPE: u32 = 22;
/// The dungeons (task `q-a3-dungeons`, REC-139).
pub const SPIDER_CAVE: u32 = 84;
pub const SPIDER_CAVERN: u32 = 85;
pub const SEWERS_1: u32 = 92;
pub const SEWERS_2: u32 = 93;
pub const DURANCE_1: u32 = 100;
pub const DURANCE_2: u32 = 101;
pub const DURANCE_3: u32 = 102;
/// The maze levels with their lvltypes `Id`: Spider 23,
/// Dungeon 24 (Swampy Pit 86 / 87 / 89, Flayer Dungeon 88 / 90 / 91),
/// Act 3 Sewer 25, Kurast 22.
pub const MAZE_LEVELS: [(u32, u32); 12] = [
    (84, 23),
    (85, 23),
    (86, 24),
    (87, 24),
    (88, 24),
    (89, 24),
    (90, 24),
    (91, 24),
    (92, 25),
    (93, 25),
    (100, 22),
    (101, 22),
];
/// The temples and Fanes (94..=99) and Durance of Hate 3, one-cell preset
/// levels (their layouts are fixed DS1s in the game data).
pub const PRESET_LEVELS: [u32; 7] = [94, 95, 96, 97, 98, 99, 102];
/// The vis chains of the dungeons (made up: in the game data the entrances are
/// DS1 warp units of the outdoor levels).
const DUNGEON_CHAINS: [&[u32]; 4] = [&[86, 87, 89], &[88, 90, 91], &[92, 93], &[100, 101, 102]];
/// The 24-tile maze cell and the level rect side of a maze level (made
/// up; the shipped rows are about 200).
pub const MAZE_CELL: u32 = 24;
const MAZE_SIZE: u32 = 200;
/// lvlprest: maze defs (659..=1100) and the preset-level rows after them.
pub const LAST_MAZE_DEF: u32 = 1100;
pub const MAZE_DS1: &str = "Synth\\Act3\\Maze.ds1";

/// The outdoor levels of Act III.
pub const OUTDOOR: [u32; 8] = [
    SPIDER_FOREST,
    GREAT_MARSH,
    FLAYER_JUNGLE,
    LOWER_KURAST,
    KURAST_BAZAAR,
    UPPER_KURAST,
    KURAST_CAUSEWAY,
    TRAVINCAL,
];

/// lvlprest: the town row, the jungle pieces and the Kurast pieces.
pub const TOWN_DEF: u32 = 529;
pub const JUNGLE_HEAD: u32 = 573;
pub const JUNGLE_TAIL: u32 = 574;
pub const CAUSEWAY_DEF: u32 = 652;
pub const LAST_DEF: u32 = 658;
pub const TOWN_DS1: &str = "Synth\\Act3\\Town.ds1";
pub const BLOCK_DS1: &str = "Synth\\Act3\\Block.ds1";
pub const HEAD_DS1: &str = "Synth\\Act3\\Head.ds1";
pub const CAUSEWAY_DS1: &str = "Synth\\Act3\\Causeway.ds1";
pub const TRAV_NARROW_DS1: &str = "Synth\\Act3\\TravNarrow.ds1";
pub const TRAV_WIDE_DS1: &str = "Synth\\Act3\\TravWide.ds1";
/// Levels-row `Waypoint` indices (made up; others 255).
pub const WAYPOINTS: [(u32, u32); 7] = [
    (TOWN, 13),
    (SPIDER_FOREST, 14),
    (GREAT_MARSH, 15),
    (FLAYER_JUNGLE, 16),
    (LOWER_KURAST, 17),
    (KURAST_BAZAAR, 18),
    (TRAVINCAL, 19),
];

/// The Act III-shaped synthetic set.
pub fn act3() -> Synthetic {
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
    format!("Synth A3 L{id}")
}

/// (id, DrlgType, LevelType, size, offset, vis) of the placer levels.
type LevelRow = (u32, u32, u32, (i32, i32), (i32, i32), &'static [u32]);

const PLACER_LEVELS: [LevelRow; 9] = [
    (75, 2, DOCKS_TYPE, (64, 48), (1000, 1000), &[76]),
    (76, 3, JUNGLE_TYPE, (64, 192), (-1, -1), &[75, 77]),
    (77, 3, JUNGLE_TYPE, (64, 192), (-1, -1), &[76, 78]),
    (78, 3, JUNGLE_TYPE, (64, 192), (-1, -1), &[77, 79]),
    (79, 3, KURAST_TYPE, (80, 64), (0, 0), &[78, 80]),
    (80, 3, KURAST_TYPE, (80, 64), (0, 0), &[79, 81]),
    (81, 3, KURAST_TYPE, (80, 64), (0, 0), &[80, 82]),
    (82, 3, KURAST_TYPE, (48, 16), (0, 0), &[81, 83]),
    (83, 3, KURAST_TYPE, (64, 64), (0, 0), &[82]),
];

fn levels(s: &mut Synthetic) {
    for id in 0..LEVEL_COUNT {
        let row = PLACER_LEVELS.iter().find(|r| r.0 == id);
        let name = level_name(id);
        s.strings.base.push((name.clone(), name.clone()));
        let maze = MAZE_LEVELS.iter().find(|m| m.0 == id);
        let dungeon_vis: Vec<u32> = DUNGEON_CHAINS
            .iter()
            .find_map(|c| {
                let i = c.iter().position(|&l| l == id)?;
                Some(c.get(i + 1).copied().into_iter().collect())
            })
            .unwrap_or_default();
        let (drlg, ltype, size, offset, vis) = match row {
            Some(&(_, d, l, s, o, v)) => (d, l, s, o, v),
            None if maze.is_some() => (
                1,
                maze.map_or(0, |m| m.1),
                (MAZE_SIZE as i32, MAZE_SIZE as i32),
                (0, 0),
                &dungeon_vis[..],
            ),
            None if PRESET_LEVELS.contains(&id) => (
                2,
                1,
                (MAZE_CELL as i32, MAZE_CELL as i32),
                (0, 0),
                &dungeon_vis[..],
            ),
            None => (0, 0, (0, 0), (0, 0), &[][..]),
        };
        let wp = WAYPOINTS.iter().find(|w| w.0 == id).map_or(255, |w| w.1);
        let act = match id {
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
            ("IsInside".into(), n(u32::from(maze.is_some()))),
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
        if drlg == 3 || maze.is_some() {
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

/// (size x, y, DS1) of lvlprest row `def` in 530..=LAST_DEF.
fn piece(def: u32) -> (u32, u32, &'static str) {
    match def {
        530..=572 | 575..=604 => (32, 32, BLOCK_DS1),
        JUNGLE_HEAD | JUNGLE_TAIL => (64, 32, HEAD_DS1),
        CAUSEWAY_DEF => (48, 16, CAUSEWAY_DS1),
        653 | 655 | 656 | 658 => (16, 32, TRAV_NARROW_DS1),
        654 | 657 => (32, 32, TRAV_WIDE_DS1),
        _ => (CELL, CELL, CELL_DS1),
    }
}

fn tables(t: &mut TableSet) {
    // lvltypes: 0 none, 1 town and preset levels, 2..=25 the rest.
    t.row("lvltypes", &[("Act", "0")]);
    for _ in 1..=25 {
        t.row(
            "lvltypes",
            &[
                ("File 1", FLOOR_DT1[0]),
                ("File 2", WALL_DT1),
                ("File 3", FLOOR_DT1[1]),
                ("Act", "2"),
            ],
        );
    }
    // lvlmaze: one row per maze level, 24-tile cells; `Merge` 500 so the
    // Spider Cavern vector's draw (367) links.
    for &(id, _) in &MAZE_LEVELS {
        t.row(
            "lvlmaze",
            &[
                ("Level", &n(id)),
                ("Rooms", "6"),
                ("Rooms(N)", "8"),
                ("Rooms(H)", "10"),
                ("SizeX", &n(MAZE_CELL)),
                ("SizeY", &n(MAZE_CELL)),
                ("Merge", "500"),
            ],
        );
    }
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
    for def in 1..TOWN_DEF {
        preset(t, def, 0, (CELL, CELL), CELL_FILES, &[CELL_DS1; 6]);
    }
    preset(t, TOWN_DEF, TOWN, (64, 48), 0, &[TOWN_DS1; 4]);
    for def in TOWN_DEF + 1..=LAST_DEF {
        let (sx, sy, ds1) = piece(def);
        let files = match def {
            JUNGLE_HEAD | JUNGLE_TAIL => 0,
            541 => 5,
            530..=544 => 3,
            545..=572 | 575..=604 => 1,
            _ => CELL_FILES,
        };
        preset(t, def, 0, (sx, sy), files, &[ds1; 6]);
    }
    // The maze defs, then one row per preset dungeon level (`LevelId`).
    for def in LAST_DEF + 1..=LAST_MAZE_DEF {
        preset(t, def, 0, (MAZE_CELL, MAZE_CELL), 6, &[MAZE_DS1; 6]);
    }
    for (i, &level) in PRESET_LEVELS.iter().enumerate() {
        let def = LAST_MAZE_DEF + 1 + i as u32;
        preset(t, def, level, (MAZE_CELL, MAZE_CELL), 1, &[MAZE_DS1]);
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

/// Kurast Docks: 64 × 48 floor tiles and the town waypoint.
pub fn town() -> Ds1 {
    let mut d = floor_preset(64, 48);
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

/// Every DRLG file of the Act III set.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let presets = [
        (TOWN_DS1, town()),
        (CELL_DS1, floor_preset(CELL, CELL)),
        (BLOCK_DS1, floor_preset(32, 32)),
        (HEAD_DS1, floor_preset(64, 32)),
        (CAUSEWAY_DS1, floor_preset(48, 16)),
        (TRAV_NARROW_DS1, floor_preset(16, 32)),
        (TRAV_WIDE_DS1, floor_preset(32, 32)),
        (MAZE_DS1, floor_preset(MAZE_CELL, MAZE_CELL)),
    ];
    let mut out: Vec<(String, Vec<u8>)> = presets
        .iter()
        .map(|(p, d)| (archive_name(p), crate::ds1::write(d)))
        .collect();
    // The jungle and Kurast floor cells carry main index 1 (flags 0x120000 /
    // 0x100000; main = bits 20..25), so the floor files
    // hold a tile for it too.
    let floors = file(vec![tile(0, 0, 0, 1), tile(0, 1, 0, 1)]);
    out.extend(
        crate::drlg::files()
            .into_iter()
            .filter(|(name, _)| !name.ends_with(".ds1") || *name == archive_name(SUB_DS1))
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
