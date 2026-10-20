//! Tests for the Cold Plains cell grid, built synthetically: the Act I geometry (rect,
//! neighbours, vis) goes through the real level build, with lvlsub files built so that each
//! border substitution (Border - Middle island, Border - Corner at (6, 6) with its two
//! blank cells, the two Border - Border bumps) matches at exactly one place. The border
//! ring follows the generation rules; the special presets land where the synthetic draws
//! put them.
//!
//! The live check of the same table is
//! `d2-server` `world_data::tests::game::outdoor_levels_generate_through_the_dispatcher`.

use crate::drlg::outdoor::grid::cell;
use crate::drlg::outdoor::tests::{act1_data, od, one_cell_file, Presets, Rec};
use crate::drlg::outdoor::*;
use crate::drlg::tiles::CellGrid;
use crate::drlg::{Drlg, RoomKind};

/// The expected Cold Plains grid: grid 0 per cell after the Act I build, rows
/// y = 0..9; `o` outdoor room, `A` one of 48 / 29 / 30, `-` blank (no room).
pub(super) const COLD_PLAINS: [&str; 10] = [
    " 9  6  6  6  6  6  6  6  6 10",
    " 5  o  A  o  o  o  o  A 44  7",
    " 5  o  o  o 15  4  4 12  o  7",
    " 5  o  o  o 14  6 10  5  o  7",
    " 5  o 51  o  o  o  7  5  o  7",
    " 5  o  o  o  A  o 14 13  o  7",
    " 8  4 12  o  o  o  o  o  o  7",
    " 9  6 13  o 15 12  o  o  o  7",
    " 5  o  o  o  7  5  o 15  4 11",
    " 8  4  4  4 11  8  4 11  -  -",
];

/// One table cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cell {
    /// A lvlprest id the table names (border pieces, 44, 51).
    Preset(u32),
    /// `A`: 48, 29 or 30.
    Any,
    Outdoor,
    Blank,
}

pub(super) fn table() -> Vec<Vec<Cell>> {
    COLD_PLAINS
        .iter()
        .map(|r| {
            r.split_whitespace()
                .map(|t| match t {
                    "o" => Cell::Outdoor,
                    "A" => Cell::Any,
                    "-" => Cell::Blank,
                    n => Cell::Preset(n.parse().unwrap()),
                })
                .collect()
        })
        .collect()
}

/// Pattern / variant codes of a synthetic substitution file.
#[derive(Clone, Copy)]
enum C {
    /// Pattern: no wall bit, no floor bit (passes). Variant: blank cell.
    Free,
    /// Wall bit 1 with style P − 4 (base 4): pattern "c = P, not a
    /// link"; variant "stamp P".
    P(u32),
    /// Variant: style 62 = the skip style S (cell left as it is).
    Skip,
}

fn code(c: C) -> (u32, u32) {
    match c {
        C::Free => (0, 0),
        C::P(p) => (0, ((p - 4 + 1) << 8) | 1),
        C::Skip => (0, (63 << 8) | 1),
    }
}

/// Rows of pattern or variant codes.
type Rows<'a> = &'a [&'a [C]];

/// A group of `pattern` (rows) with one variant laid out to its right
/// at offset w + 1 (one variant, so v = 0 and the replace offset is (v + 1)(w + 1)).
fn group_file(groups: &[(Rows, Rows)]) -> SubFile {
    let width = groups
        .iter()
        .map(|(p, _)| 2 * (p[0].len() + 1))
        .max()
        .unwrap();
    let height: usize = groups.iter().map(|(p, _)| p.len()).sum();
    let mut floor = CellGrid::new(width, height);
    let mut wall = CellGrid::new(width, height);
    let mut out = SubFile {
        method: 1,
        floor: None,
        walls: Vec::new(),
        tile_types: vec![CellGrid::new(width, height)],
        ..SubFile::default()
    };
    let mut y0 = 0;
    for (pattern, variant) in groups {
        let w = pattern[0].len();
        for (j, (pr, vr)) in pattern.iter().zip(variant.iter()).enumerate() {
            for i in 0..w {
                for (x, c) in [(i, pr[i]), (i + w + 1, vr[i])] {
                    let (f, wl) = code(c);
                    floor.set(x, y0 + j, f);
                    wall.set(x, y0 + j, wl);
                }
            }
        }
        out.groups.push(SubGroup {
            x: 0,
            y: y0 as i32,
            w: w as i32,
            h: pattern.len() as i32,
            variants: 1,
        });
        y0 += pattern.len();
    }
    out.floor = Some(floor);
    out.walls = vec![wall];
    out
}

/// The Cold Plains build with the synthetic lvlsub rows (types 0..3,
/// `BordType` 1, `GridSize` 1).
struct Build {
    outdoor: Outdoor,
    drlg: Drlg,
    presets: Presets,
    level: crate::drlg::LevelIdx,
}

fn build() -> Build {
    use C::{Free as F, Skip as S, P};
    let mut data = act1_data();
    data.levels[3].level_type = 2;
    let mut od = od();
    let mut subs = SubFileMap::default();
    // Type 0: a style that never occurs (base 4 + 98): no replacement.
    let never = one_cell_file(0, (99 << 8) | 1, 1);
    // Type 1 (Border - Middle): the island of rows 2..5, keyed on the
    // whole top border row so that only (0, 0) matches.
    let top: &[C] = &[P(9), P(6), P(6), P(6), P(6), P(6), P(6), P(6), P(6), P(10)];
    let free10: &[C] = &[F; 10];
    let skip10: &[C] = &[S; 10];
    let island = group_file(&[(
        &[top, free10, free10, free10, free10, free10],
        &[
            skip10,
            skip10,
            &[S, S, S, S, P(15), P(4), P(4), P(12), S, S],
            &[S, S, S, S, P(14), P(6), P(10), P(5), S, S],
            &[S, S, S, S, S, S, P(7), P(5), S, S],
            &[S, S, S, S, S, S, P(14), P(13), S, S],
        ],
    )]);
    // Type 2 (Border - Corner): the bottom-right corner, two blank cells.
    let corner = group_file(&[(
        &[
            &[F, F, F, P(7)],
            &[F, F, F, P(7)],
            &[F, F, F, P(7)],
            &[P(4), P(4), P(4), P(11)],
        ],
        &[
            &[S, S, S, S],
            &[S, S, S, S],
            &[S, P(15), P(4), P(11)],
            &[P(4), P(11), F, F],
        ],
    )]);
    // Type 3 (Border - Border): the bump at (3, 6) and the one at (0, 5).
    let bumps = group_file(&[
        (
            &[
                &[F, F, F, F],
                &[F, F, F, F],
                &[F, F, F, F],
                &[P(4), P(4), P(4), P(4)],
            ],
            &[
                &[S, S, S, S],
                &[S, P(15), P(12), S],
                &[S, P(7), P(5), S],
                &[S, P(11), P(8), S],
            ],
        ),
        (
            &[
                &[P(5), F, F],
                &[P(5), F, F],
                &[P(5), F, F],
                &[P(5), F, F],
                &[P(8), F, F],
            ],
            &[
                &[S, S, S],
                &[P(8), P(4), P(12)],
                &[P(9), P(6), P(13)],
                &[S, S, S],
                &[S, S, S],
            ],
        ),
    ]);
    for (t, file) in [(0, never), (1, island), (2, corner), (3, bumps)] {
        let name = format!("t{t}").into_bytes();
        od.subs.push(SubRow {
            type_: t,
            file: name.clone(),
            bord_type: 1,
            grid_size: 1,
            ..SubRow::default()
        });
        subs.0.insert(name, file);
    }
    let mut outdoor = Outdoor::default();
    let mut rec = Rec::default();
    let mut presets = Presets::default();
    let mut types = OutdoorTypes {
        outdoor: &mut outdoor,
        od: &od,
        subs: &subs,
        presets: &mut presets,
        others: &mut rec,
        last_error: None,
    };
    let mut drlg = Drlg::create(0, 644409375, 0, 0, false, &data, &mut types).unwrap();
    let level = drlg.find_level(3).unwrap();
    drlg.generate_level(&data, &mut types, level).unwrap();
    assert_eq!(types.last_error, None);
    Build {
        outdoor,
        drlg,
        presets,
        level,
    }
}

#[test]
fn cold_plains_grid_reproduces_the_table() {
    let b = build();
    let info = b.outdoor.level(b.level).unwrap();
    assert_eq!((info.gw(), info.gh()), (10, 10));
    // Each substitution matched once, where its unique pattern fits.
    let hits: Vec<(i32, usize, i32, i32)> = info
        .sub_hits
        .iter()
        .map(|h| (h.t, h.group, h.x, h.y))
        .collect();
    assert_eq!(
        hits,
        [(1, 0, 0, 0), (2, 0, 6, 6), (3, 0, 3, 6), (3, 1, 0, 5)]
    );
    let mut blanks = 0;
    for (y, row) in table().iter().enumerate() {
        for (x, &want) in row.iter().enumerate() {
            let (x, y) = (x as i32, y as i32);
            let (g0, g2) = (info.grids[0].get(x, y), info.grids[2].get(x, y));
            match want {
                Cell::Preset(p) if (4..=15).contains(&p) => {
                    assert_eq!(g0, p, "({x}, {y})");
                    assert_ne!(g2 & cell::PRESET, 0, "({x}, {y})");
                }
                Cell::Blank => {
                    blanks += 1;
                    assert_eq!((g0, g2 & (cell::BLANK | cell::PRESET)), (0, cell::BLANK));
                }
                // Outdoor cells and the specials (44, 51, A): placed by
                // the synthetic draws, so only "a room, not a border
                // piece" is fixed.
                _ => {
                    assert_eq!(g2 & cell::BLANK, 0, "({x}, {y})");
                    assert!(
                        matches!(g0, 0 | 29 | 30 | 44 | 48 | 49 | 51),
                        "({x}, {y}): {g0}"
                    );
                }
            }
        }
    }
    assert_eq!(blanks, 2);
    // The link midpoints (0x400, file 3).
    for (x, y) in [(0, 1), (9, 5), (2, 9)] {
        assert_eq!(info.grids[2].get(x, y) & 0xF0400, 0x30400, "({x}, {y})");
    }
    // One room per non-blank cell, 98 = 100 − the two blanks of Border - Corner. The
    // special presets the synthetic draws placed decide the split between preset and
    // outdoor rooms.
    let rooms = b.drlg.level_rooms(b.level);
    let outdoor = rooms
        .iter()
        .filter(|&&r| b.drlg.room(r).kind == RoomKind::Outdoor)
        .count();
    assert_eq!(b.presets.calls.len() + outdoor, 98);
    assert_eq!(rooms.len(), outdoor);
}

/// Freeing the Cold Plains rooms resets the outdoor type data after the rooms are freed;
/// the reset drops their room records without reading a freed room.
#[test]
fn cold_plains_rooms_free_with_their_outdoor_records() {
    let mut b = build();
    let rooms = b.drlg.level_rooms(b.level);
    assert!(!rooms.is_empty());
    assert!(rooms.iter().all(|r| b.outdoor.room(*r).is_some()));
    let od = od();
    let subs = SubFileMap::default();
    let mut rec = Rec::default();
    let mut types = OutdoorTypes {
        outdoor: &mut b.outdoor,
        od: &od,
        subs: &subs,
        presets: &mut b.presets,
        others: &mut rec,
        last_error: None,
    };
    b.drlg.free_level_rooms(&mut types, b.level);
    assert!(rooms.iter().all(|&r| b.drlg.try_room(r).is_none()));
    assert!(b.outdoor.rooms.is_empty());
    assert!(b.drlg.level_rooms(b.level).is_empty());
}
