//! Tests from the spec's rules and synthetic test vectors.

use std::collections::BTreeMap;

use super::cells::{group, GROUP_PAIRS};
use super::draw::{blocks, clip_rect, draw_mode, pass_fade, walk, FadeFacts};
use super::header::{header, HeaderFacts};
use super::markers::{
    color, cross, roster_markers, unit_markers, MarkerColor, MarkerCtx, MarkerPalette,
    MarkerSubject, MarkerUnit, RosterEntry, SHAPE,
};
use super::options::{cel_paths, MemoryStore, FADE, LEFT};
use super::persist::{open_index, IndexOpen, MAP_BYTES, RECORD_HEADER, TABLE_BYTES};
use super::picker::{PickKey, PickRecord};
use super::place::{
    add_tile, add_units, tile_cell, unit_cel, unit_cell, UnitLevel, REC_AUTOMAP, UNIT_AUTOMAP,
    UNIT_DRAWN,
};
use super::town::town_cells;
use super::*;
use crate::rules::draw_order::{Dt1Facts, Fade, LevelFacts, TileRecord, TileRect, MONSTER, OBJECT};

fn pos(x: i32, y: i32) -> ClientPos {
    ClientPos { x, y }
}

fn record(tile: (i32, i32), ty: u32, orientation: u32, flags: u32) -> TileRecord {
    TileRecord {
        tile,
        flags,
        ty,
        dt1: Dt1Facts {
            orientation,
            main: 0,
            sub: 5,
            ..Dt1Facts::default()
        },
        fade: Fade::OPAQUE,
        logical: None,
    }
}

fn rec(level: i32, tile: i32, style: u8, seq: (u8, u8), cels: &[i32]) -> PickRecord {
    let mut c = [-1; 4];
    c[..cels.len()].copy_from_slice(cels);
    PickRecord {
        level,
        tile,
        style,
        start: seq.0,
        end: seq.1,
        cels: c,
        count: cels.len() as i32,
    }
}

/// LevelType 1: records 0–2; LevelType 2: record 3; LevelType 3 unused;
/// LevelType 4: broken range.
fn picker() -> CelPicker {
    let mut ranges = vec![(-1, -1); 36];
    ranges[1] = (0, 3);
    ranges[2] = (3, 4);
    ranges[4] = (4, -1);
    CelPicker {
        records: vec![
            rec(1, 0, 0, (1, 46), &[0, 1, 2, 3]),
            rec(1, 0, 0xFF, (0xFF, 0), &[40]),
            rec(1, 1, 0xFF, (0xFF, 0), &[7]),
            rec(2, 0, 0xFF, (0xFF, 0), &[60, 70]),
        ],
        ranges,
    }
}

struct Levels;

impl AutomapLevels for Levels {
    fn layer(&self, level: u32) -> Option<u32> {
        // Levels 1 and 2 share layer 1; level 3 is layer 3; 40 is 40.
        match level {
            1 | 2 => Some(1),
            3 => Some(3),
            40 | 103 | 109 => Some(level),
            _ => None,
        }
    }

    fn level_type(&self, level: u32) -> Option<u32> {
        match level {
            1 | 2 | 40 => Some(1),
            3 => Some(2),
            _ => None,
        }
    }

    fn act(&self, level: u32) -> Option<u8> {
        Some(if level == 3 { 2 } else { 0 })
    }
}

struct Cels;

impl UnitCels for Cels {
    fn monster_cel(&self, class: u32) -> Option<u32> {
        match class {
            0 => None,
            1 => Some(0),
            c => Some(300 + c),
        }
    }

    fn object_cel(&self, class: u32) -> Option<u32> {
        match class {
            0 => Some(0),
            c => Some(400 + c % 100),
        }
    }
}

fn tables<'a>(p: &'a CelPicker) -> AutomapTables<'a> {
    AutomapTables {
        picker: p,
        levels: &Levels,
        cels: &Cels,
    }
}

fn automap() -> Automap {
    Automap::new(&MemoryStore::default(), SEED)
}

fn cells_of(a: &Automap, k: TreeKind) -> Vec<(i16, i16, i16)> {
    a.cells
        .tree(k)
        .in_order()
        .iter()
        .map(|c| (c.cel, c.x, c.y))
        .collect()
}

fn frame() -> FrameFacts {
    FrameFacts {
        width: 800,
        height: 600,
        open_mode: 0,
        mini_down: false,
        unit_origin: pos(0, 0),
    }
}

// -----------------------------------------------------------------

#[test]
fn groups_are_the_49_pairs_and_minus_one_elsewhere() {
    assert_eq!(GROUP_PAIRS.len(), 49);
    for (c, g) in [
        (0, 0),
        (3, 0),
        (8, 1),
        (12, 2),
        (14, 3),
        (38, 4),
        (39, 5),
        (49, 6),
    ] {
        assert_eq!(group(c), g, "cel {c}");
    }
    for (c, g) in [(54, 7), (70, 8), (71, 9), (171, 10), (172, 11), (259, 12)] {
        assert_eq!(group(c), g, "cel {c}");
    }
    for (c, g) in [
        (267, 13),
        (338, 14),
        (472, 15),
        (475, 15),
        (522, 16),
        (534, 17),
    ] {
        assert_eq!(group(c), g, "cel {c}");
    }
    for c in [4, 5, 9, 10, 15, 476, 535, 2047, 2048, -1] {
        assert_eq!(group(c), -1, "cel {c}");
    }
}

#[test]
fn duplicate_cells_follow_the_groups() {
    // existing (y 10, x 5, cel 0) + new cel 2: refused (group 0 = 0).
    let mut t = CellTree::default();
    assert!(t.insert(Cell::new(0, 5, 10)));
    assert!(!t.insert(Cell::new(2, 5, 10)));
    // + new cel 5: refused (g(5) = −1).
    assert!(!t.insert(Cell::new(5, 5, 10)));
    assert_eq!((t.len(), t.allocated()), (1, 3));
    // existing cel 5 + new cel 6: inserted after it (g = 1, 6 > 5).
    let mut t = CellTree::default();
    assert!(t.insert(Cell::new(5, 5, 10)));
    assert!(t.insert(Cell::new(6, 5, 10)));
    let order: Vec<i16> = t.in_order().iter().map(|c| c.cel).collect();
    assert_eq!(order, [5, 6]);
    // A same-group cell is refused by the node on its descent path.
    let mut t = CellTree::default();
    t.insert(Cell::new(0, 5, 10));
    assert!(t.insert(Cell::new(6, 5, 10)));
    assert!(!t.insert(Cell::new(7, 5, 10)));
    // An ungrouped old cell orders a grouped new one by cel number.
    let mut t = CellTree::default();
    t.insert(Cell::new(9, 5, 10));
    assert!(t.insert(Cell::new(3, 5, 10)));
    assert_eq!(
        t.in_order().iter().map(|c| c.cel).collect::<Vec<_>>(),
        [3, 9]
    );
}

#[test]
fn tree_is_an_avl_in_y_x_order() {
    let mut t = CellTree::default();
    let mut s = Seed::new(7, 666);
    let mut want = Vec::new();
    for _ in 0..500 {
        let c = Cell::new(9, s.roll(40) as i16 - 20, s.roll(40) as i16 - 20);
        if t.insert(c) {
            want.push((c.y, c.x));
        }
        assert!(t.balanced());
    }
    want.sort();
    let got: Vec<(i16, i16)> = t.in_order().iter().map(|c| (c.y, c.x)).collect();
    assert_eq!(got, want);
    // Ascending inserts stay logarithmic.
    let mut t = CellTree::default();
    for i in 0..1023 {
        t.insert(Cell::new(9, 0, i));
    }
    assert_eq!(t.height(), 10);
}

#[test]
fn switch_keeps_only_the_current_layer_in_memory() {
    let mut a = automap();
    a.attach_file(MaFile::default(), 7);
    a.limits = CelLimits {
        maxi: 10,
        town: [10; 4],
    };
    a.switch(1).unwrap();
    a.cells.tree_mut(TreeKind::Floor).insert(Cell::new(0, 1, 2));
    a.switch(3).unwrap();
    // Prepend: the newest layer heads the list.
    assert_eq!(a.layers().iter().map(|l| l.id).collect::<Vec<_>>(), [3, 1]);
    assert_eq!(a.current(), Some(3));
    assert!(a.cells.tree(TreeKind::Floor).is_empty());
    // Back to layer 1: its cells come from the file, saved flag 1.
    a.switch(1).unwrap();
    let c = a.cells.tree(TreeKind::Floor).in_order();
    assert_eq!(
        c,
        [Cell {
            saved: true,
            cel: 0,
            x: 1,
            y: 2
        }]
    );
    // Switching to the current layer changes nothing.
    let before = a.file().unwrap().bytes.len();
    a.switch(1).unwrap();
    assert_eq!(a.file().unwrap().bytes.len(), before);
}

// -----------------------------------------------------------------

#[test]
fn picker_without_a_range_gives_minus_one_without_a_draw() {
    let p = picker();
    let mut s = SEED;
    let k = |level_type| PickKey {
        level_type,
        ..PickKey::default()
    };
    assert_eq!(p.pick(k(0), &mut s), Ok(-1));
    assert_eq!(p.pick(k(3), &mut s), Ok(-1));
    assert_eq!(s, SEED);
    assert!(matches!(
        p.pick(k(4), &mut s),
        Err(AutomapError::Fatal { code: 0x53B, .. })
    ));
}

#[test]
fn picker_takes_the_first_match_and_rolls_its_cels() {
    let p = picker();
    // Record 0 matches (style 0, sub 5 in 1–46): roll(4) on {0, 666} =
    // 666 & 3 = 2 → Cel3 = 2; seed {666, 0}.
    let mut s = SEED;
    let k = PickKey {
        level_type: 1,
        orientation: 0,
        main: 0,
        sub: 5,
    };
    assert_eq!(p.pick(k, &mut s), Ok(2));
    assert_eq!(s, Seed::new(666, 0));
    // Main 3 fails record 0's style; record 1 (wildcards) wins; count 1
    // still draws.
    let mut s = SEED;
    assert_eq!(p.pick(PickKey { main: 3, ..k }, &mut s), Ok(40));
    assert_ne!(s, SEED);
    // Sub 50 fails record 0's sequences.
    let mut s = SEED;
    assert_eq!(p.pick(PickKey { sub: 50, ..k }, &mut s), Ok(40));
    // Orientation 1 → record 2; orientation 5 → no winner, no draw.
    let mut s = SEED;
    assert_eq!(
        p.pick(
            PickKey {
                orientation: 1,
                ..k
            },
            &mut s
        ),
        Ok(7)
    );
    let mut s = SEED;
    assert_eq!(
        p.pick(
            PickKey {
                orientation: 5,
                ..k
            },
            &mut s
        ),
        Ok(-1)
    );
    assert_eq!(s, SEED);
}

// -----------------------------------------------------------------

#[test]
fn tile_cell_vectors() {
    let c = |wx, wy, ty| {
        let c = tile_cell(wx, wy, ty, 1).unwrap();
        (c.x, c.y)
    };
    assert_eq!(c(100, 50, 1), (400, 600));
    assert_eq!(c(100, 50, 17), (400, 624));
    assert_eq!(c(50, 100, 0), (-400, 600));
    assert_eq!(c(100, 50, 16), (400, 624));
    assert_eq!(c(100, 50, 15), (400, 600));
}

#[test]
fn tile_cell_range_is_fatal() {
    let f = |r: Result<Cell, AutomapError>| match r {
        Err(AutomapError::Fatal { code, .. }) => code,
        o => panic!("{o:?}"),
    };
    assert_eq!(f(tile_cell(5000, 0, 0, 1)), 0x391);
    assert_eq!(f(tile_cell(5000, 5000, 0, 1)), 0x392);
    assert_eq!(f(tile_cell(0, 0, 0, 40000)), 0x393);
}

#[test]
fn a_record_is_offered_once() {
    let p = picker();
    let mut s = SEED;
    let mut t = CellTree::default();
    let mut r = record((2, 3), 0, 0, 0);
    // Origin (98, 47) + (2, 3) = (100, 50).
    assert_eq!(
        add_tile(&mut r, (98, 47), 1, &p, &mut s, &mut t),
        Ok(Some(true))
    );
    assert_eq!(r.flags & REC_AUTOMAP, REC_AUTOMAP);
    assert_eq!(
        t.in_order(),
        [Cell::new(2, 400, 600)],
        "cel from the vector roll, cell from §3 r3"
    );
    let seed = s;
    assert_eq!(add_tile(&mut r, (98, 47), 1, &p, &mut s, &mut t), Ok(None));
    assert_eq!(s, seed);
    // A record the picker rejects is flagged too and never retried.
    let mut miss = record((0, 0), 0, 5, 0);
    assert_eq!(add_tile(&mut miss, (0, 0), 1, &p, &mut s, &mut t), Ok(None));
    assert_eq!(miss.flags & REC_AUTOMAP, REC_AUTOMAP);
}

// -----------------------------------------------------------------

#[test]
fn unit_cell_uses_c_division() {
    let c = unit_cell(pos(-25, 47), 3).unwrap();
    assert_eq!((c.cel, c.x, c.y), (3, -1, 1));
    assert!(matches!(
        unit_cell(pos(400_000, 0), 1),
        Err(AutomapError::Fatal { code: 0x3C5, .. })
    ));
    assert!(matches!(
        unit_cell(pos(0, 400_000), 1),
        Err(AutomapError::Fatal { code: 0x3C6, .. })
    ));
    assert!(matches!(
        unit_cell(pos(0, 0), 40_000),
        Err(AutomapError::Fatal { code: 0x3C7, .. })
    ));
}

#[test]
fn unit_cels_and_object_exceptions() {
    let l = UnitLevel { id: 1, act: 0 };
    let u = |unit_type, class, mode| AutomapUnit {
        unit_type,
        class,
        mode,
        ..AutomapUnit::default()
    };
    // Monsters: no chain → none; cel 0 → none.
    assert_eq!(unit_cel(&u(MONSTER, 0, 0), l, &Cels), None);
    assert_eq!(unit_cel(&u(MONSTER, 1, 0), l, &Cels), None);
    assert_eq!(unit_cel(&u(MONSTER, 5, 0), l, &Cels), Some(305));
    // Objects: AutoMap 0 → none.
    assert_eq!(unit_cel(&u(OBJECT, 0, 0), l, &Cels), None);
    assert_eq!(unit_cel(&u(OBJECT, 3, 0), l, &Cels), Some(403));
    // 267 only in acts 2, 3; 366 only in mode 2; 402 only in level 74.
    assert_eq!(unit_cel(&u(OBJECT, 267, 0), l, &Cels), None);
    assert_eq!(
        unit_cel(&u(OBJECT, 267, 0), UnitLevel { id: 1, act: 2 }, &Cels),
        Some(467)
    );
    assert_eq!(
        unit_cel(&u(OBJECT, 267, 0), UnitLevel { id: 1, act: 3 }, &Cels),
        Some(467)
    );
    assert_eq!(unit_cel(&u(OBJECT, 366, 1), l, &Cels), None);
    assert_eq!(unit_cel(&u(OBJECT, 366, 2), l, &Cels), Some(466));
    assert_eq!(unit_cel(&u(OBJECT, 402, 0), l, &Cels), None);
    assert_eq!(
        unit_cel(&u(OBJECT, 402, 0), UnitLevel { id: 74, act: 0 }, &Cels),
        Some(402)
    );
    // Players, missiles, items: nothing.
    assert_eq!(unit_cel(&u(0, 5, 0), l, &Cels), None);
    assert_eq!(unit_cel(&u(4, 5, 0), l, &Cels), None);
}

#[test]
fn units_are_added_once_after_they_were_drawn() {
    let mut units = vec![
        AutomapUnit {
            unit_type: MONSTER,
            class: 5,
            flags: UNIT_DRAWN,
            pos: pos(100, 200),
            ..AutomapUnit::default()
        },
        // Not drawn yet.
        AutomapUnit {
            unit_type: MONSTER,
            class: 6,
            flags: 0,
            ..AutomapUnit::default()
        },
        // Drawn, no cel: still marked.
        AutomapUnit {
            unit_type: OBJECT,
            class: 0,
            flags: UNIT_DRAWN,
            ..AutomapUnit::default()
        },
    ];
    let mut t = CellTree::default();
    add_units(&mut units, UnitLevel::default(), &Cels, &mut t).unwrap();
    assert_eq!(t.in_order(), [Cell::new(305, 11, 17)]);
    assert_eq!(units[0].flags & UNIT_AUTOMAP, UNIT_AUTOMAP);
    assert_eq!(units[1].flags & UNIT_AUTOMAP, 0);
    assert_eq!(units[2].flags & UNIT_AUTOMAP, UNIT_AUTOMAP);
    // A second walk adds nothing.
    add_units(&mut units, UnitLevel::default(), &Cels, &mut t).unwrap();
    assert_eq!(t.allocated(), 1);
}

// B030: a used shrine icon stays on the automap.
#[test]
fn a_unit_cell_outlives_its_unit() {
    let mut units = vec![AutomapUnit {
        unit_type: OBJECT,
        class: 5,
        flags: UNIT_DRAWN,
        pos: pos(100, 200),
        ..AutomapUnit::default()
    }];
    let mut t = CellTree::default();
    add_units(&mut units, UnitLevel::default(), &Cels, &mut t).unwrap();
    // The unit is gone (a used shrine); the cell add has no removal path,
    // so a later walk over the room's remaining units leaves the cell.
    units.clear();
    add_units(&mut units, UnitLevel::default(), &Cels, &mut t).unwrap();
    assert_eq!(t.in_order().len(), 1);
}

// -----------------------------------------------------------------

#[test]
fn reveal_distance_vectors() {
    let moved = |dx: i32, dy: i32| {
        let mut r = RevealState::default();
        r.step(pos(dx, dy))
    };
    // (60, 40): (120 + 40) / 2 = 80 ≥ 0x50.
    assert!(moved(60, 40));
    // (50, 59): 84; (50, 55): 80; (40, 59): 79.
    assert!(moved(50, 59));
    assert!(moved(50, 55));
    assert!(!moved(40, 59));
    assert!(moved(-60, -40));
    let mut r = RevealState::default();
    assert!(!r.step(pos(40, 59)));
    assert_eq!(r.last, (0, 0));
    assert!(r.step(pos(60, 40)));
    assert_eq!(r.last, (60, 40));
}

fn room(level: u32, origin: (i32, i32), recs: Vec<TileRecord>) -> Room {
    Room {
        level,
        tiles: TileRect {
            x: origin.0,
            y: origin.1,
            w: 8,
            h: 8,
        },
        floors: recs.clone(),
        walls: recs.iter().map(|r| TileRecord { ty: 17, ..*r }).collect(),
        ..Room::default()
    }
}

fn near(rooms: Vec<Room>, level: u32) -> NearRooms {
    NearRooms {
        rooms,
        player_tile: (0, 0),
        player_logical: 0,
        player_subtile: (0, 0),
        edge: None,
        level: LevelFacts {
            id: level,
            ..LevelFacts::default()
        },
    }
}

#[test]
fn reveal_adds_drawn_records_of_the_players_layer() {
    let p = picker();
    let t = tables(&p);
    let drawn = record((0, 0), 0, 1, REC_DRAWN);
    let hidden = record((1, 0), 0, 1, REC_DRAWN | REC_HIDDEN);
    let undrawn = record((2, 0), 0, 1, 0);
    // Level 2 shares layer 1 with level 1 (the player's level): its room
    // is revealed too; level 3 (layer 3) is not.
    let mut n = near(
        vec![
            room(1, (10, 0), vec![drawn, hidden, undrawn]),
            room(2, (20, 0), vec![drawn]),
            room(3, (30, 0), vec![drawn]),
        ],
        1,
    );
    let mut units = vec![Vec::new(), Vec::new(), Vec::new()];
    let mut a = automap();
    a.reveal_frame(
        Some(pos(100, 0)),
        Some(RevealRooms {
            near: &mut n,
            units: &mut units,
        }),
        &t,
    )
    .unwrap();
    assert_eq!(a.current(), Some(1));
    // Orientation 1 → cel 7; floors at y = 4(wx + wy), walls (type 17)
    // 24 lower.
    assert_eq!(cells_of(&a, TreeKind::Floor), [(7, 80, 40), (7, 160, 80)]);
    assert_eq!(cells_of(&a, TreeKind::Wall), [(7, 80, 64), (7, 160, 104)]);
    assert_eq!(n.rooms[0].floors[0].flags & REC_AUTOMAP, REC_AUTOMAP);
    assert_eq!(n.rooms[0].floors[1].flags & REC_AUTOMAP, 0);
    assert_eq!(n.rooms[0].floors[2].flags & REC_AUTOMAP, 0);
    assert_eq!(n.rooms[2].floors[0].flags & REC_AUTOMAP, 0);
    // Not far enough since the reveal: nothing more.
    n.rooms[0].floors[2].flags |= REC_DRAWN;
    a.reveal_frame(
        Some(pos(120, 0)),
        Some(RevealRooms {
            near: &mut n,
            units: &mut units,
        }),
        &t,
    )
    .unwrap();
    assert_eq!(a.cells.tree(TreeKind::Floor).len(), 2);
    // No room: the position is stored, nothing added.
    a.reveal_frame(Some(pos(400, 0)), None, &t).unwrap();
    assert_eq!(a.reveal.last, (400, 0));
    // No local player: fatal 0x696.
    assert!(matches!(
        a.reveal_frame(None, None, &t),
        Err(AutomapError::Fatal { code: 0x696, .. })
    ));
}

#[test]
fn placement_skips_two_frames() {
    let p = picker();
    let t = tables(&p);
    let mut a = automap();
    a.placed();
    // No player in either frame: the countdown runs before the check.
    a.reveal_frame(None, None, &t).unwrap();
    a.reveal_frame(None, None, &t).unwrap();
    assert_eq!(a.reveal.countdown, 0);
    assert!(a.reveal_frame(None, None, &t).is_err());
}

#[test]
fn preset_rooms_add_every_record_and_switch_back() {
    let p = picker();
    let t = tables(&p);
    let mut a = automap();
    a.switch(3).unwrap();
    let mut r = room(
        1,
        (0, 0),
        vec![record((1, 1), 0, 1, 0), record((2, 2), 0, 1, REC_HIDDEN)],
    );
    a.preset_room(&mut r, &mut [], &t).unwrap();
    assert_eq!(a.current(), Some(3));
    // All = 1 still skips a record with flag 0x8.
    assert_ne!(r.floors[0].flags & REC_AUTOMAP, 0);
    assert_eq!(r.floors[1].flags & REC_AUTOMAP, 0);
    // Without a file the layer's cells are gone after the switch back;
    // with no previous layer the preset layer stays current.
    let mut b = automap();
    b.preset_room(&mut r.clone(), &mut [], &t).unwrap();
    assert_eq!(b.current(), Some(1));
    let mut r2 = room(1, (0, 0), vec![record((1, 1), 0, 1, 0)]);
    b.preset_room(&mut r2, &mut [], &t).unwrap();
    assert_eq!(cells_of(&b, TreeKind::Floor), [(7, 0, 8)]);
}

// -----------------------------------------------------------------

#[test]
fn town_art_grids() {
    let (k, c) = town_cells(40, 1, 10, 4).unwrap();
    let (x, y) = (48, 56);
    assert_eq!(k, TownKind::LutGholein);
    let n: Vec<i16> = c.iter().map(|c| c.cel).collect();
    assert_eq!(n, [1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 17, 18]);
    assert_eq!((c[0].x, c[0].y), ((x - 453 + 160) as i16, (y - 119) as i16));
    // n = 18: col 3, row 3.
    assert_eq!(
        (c[14].x, c[14].y),
        ((x - 453 + 480) as i16, (y - 119 + 300) as i16)
    );
    assert!(c.iter().all(|c| !c.saved));
    let (_, c) = town_cells(40, 2, 10, 4).unwrap();
    let n: Vec<i16> = c.iter().map(|c| c.cel).collect();
    assert_eq!(n, [21, 22, 23, 24, 26, 27, 28, 29, 31, 32, 33, 34, 37, 38]);
    let (k, c) = town_cells(103, 1, 0, 0).unwrap();
    assert_eq!(k, TownKind::Pandemonium);
    let at: Vec<_> = c.iter().map(|c| (c.cel, c.x, c.y)).collect();
    assert_eq!(at, [(0, -133, -40), (1, 3, -40), (2, -133, 50), (3, 3, 50)]);
    let (k, c) = town_cells(109, 1, 0, 0).unwrap();
    assert_eq!((k, c.len()), (TownKind::Harrogath, 6));
    assert_eq!((c[5].cel, c[5].x, c[5].y), (5, -250 + 360, -15 + 170));
    assert!(matches!(
        town_cells(1, 1, 0, 0),
        Err(AutomapError::Fatal { code: 0x751, .. })
    ));
    // The callback sets the layer's kind and switches back.
    let p = picker();
    let mut a = automap();
    a.switch(1).unwrap();
    a.town_art(40, 1, (10, 4), &tables(&p)).unwrap();
    assert_eq!(a.current(), Some(1));
    assert_eq!(
        a.layers().iter().find(|l| l.id == 40).map(|l| l.town),
        Some(TownKind::LutGholein)
    );
    let mut b = automap();
    b.town_art(103, 1, (0, 0), &tables(&p)).unwrap();
    assert_eq!(
        (b.town(), b.cells.tree(TreeKind::Town).len()),
        (TownKind::Pandemonium, 4)
    );
}

// -----------------------------------------------------------------

fn u32s(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[test]
fn index_slots() {
    // No file: version 0xC, next 1, slot 0 = key, all deleted.
    let o = open_index(None, 77).unwrap();
    assert_eq!((o.slot, o.delete.clone()), (0, vec![0, 1, 2, 3]));
    let h = o.rewrite.unwrap();
    assert_eq!(u32s(&h), [0xC, 1, 77, 0, 0, 0]);
    // A known key opens its slot, no rewrite.
    assert_eq!(
        open_index(Some(&h), 77).unwrap(),
        IndexOpen {
            slot: 0,
            rewrite: None,
            delete: vec![]
        }
    );
    // A new key takes `next`, which advances mod 4.
    let o = open_index(Some(&h), 88).unwrap();
    assert_eq!((o.slot, o.delete.clone()), (1, vec![1]));
    assert_eq!(u32s(&o.rewrite.unwrap()), [0xC, 2, 77, 88, 0, 0]);
    let mut full = [0u8; MAP_BYTES];
    for (i, v) in [0xC, 3, 1, 2, 3, 4].into_iter().enumerate() {
        full[4 * i..4 * i + 4].copy_from_slice(&u32::to_le_bytes(v));
    }
    let o = open_index(Some(&full), 9).unwrap();
    assert_eq!(o.slot, 3);
    assert_eq!(u32s(&o.rewrite.unwrap()), [0xC, 0, 1, 2, 3, 9]);
    // Wrong version or short file: invalid.
    let mut old = full;
    old[0] = 0xB;
    assert_eq!(open_index(Some(&old), 1).unwrap().delete.len(), 4);
    assert_eq!(open_index(Some(&full[..20]), 1).unwrap().delete.len(), 4);
}

#[test]
fn records_append_and_chain() {
    let mut f = MaFile::default();
    let r = |town: u32, n: i16| MaRecord {
        layer: 5,
        town_kind: town,
        act2: 9,
        blobs: [
            vec![Cell::new(n, 1, 2)],
            vec![],
            vec![Cell::new(3, -4, 5), Cell::new(4, 6, 7)],
            vec![],
        ],
    };
    assert_eq!(f.append(&r(0, 1)).unwrap(), TABLE_BYTES as u32);
    assert_eq!(u32s(&f.bytes[20..24]), [TABLE_BYTES as u32]);
    let h = u32s(&f.bytes[TABLE_BYTES..TABLE_BYTES + RECORD_HEADER]);
    assert_eq!(h, [0, 5, 0, 9, 6, 0, 12, 0]);
    let body = &f.bytes[TABLE_BYTES + RECORD_HEADER..];
    assert_eq!(body.len(), 18);
    assert_eq!(&body[..6], &[1, 0, 1, 0, 2, 0]);
    assert_eq!(&body[6..8], &[3, 0]);
    assert_eq!(&body[8..10], &(-4i16).to_le_bytes());
    // The second record is linked from the first's first u32.
    let second = f.append(&r(0, 2)).unwrap();
    assert_eq!(u32s(&f.bytes[TABLE_BYTES..TABLE_BYTES + 4]), [second]);
    let l = f
        .load(
            5,
            9,
            CelLimits {
                maxi: 100,
                town: [100; 4],
            },
        )
        .unwrap();
    assert!(!l.cut);
    assert_eq!(l.trees[0].iter().map(|c| c.cel).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(l.trees[2].len(), 4);
    assert!(l.trees.iter().flatten().all(|c| c.saved));
}

#[test]
fn load_cuts_bad_chains_and_skips_other_acts_units() {
    let lim = CelLimits {
        maxi: 10,
        town: [10, 1, 1, 1],
    };
    let r = |layer, cel| MaRecord {
        layer,
        town_kind: 1,
        act2: 9,
        blobs: [
            vec![Cell::new(cel, 0, 0), Cell::new(1, 0, 1)],
            vec![Cell::new(2, 0, 0)],
            vec![Cell::new(3, 0, 0)],
            vec![Cell::new(0, 0, 0)],
        ],
    };
    // Unit blob of another act record: skipped.
    let mut f = MaFile::default();
    f.append(&r(5, 0)).unwrap();
    let l = f.load(5, 8, lim).unwrap();
    assert_eq!(
        l.trees.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 1, 0, 1]
    );
    // A cel past the file's count stops that blob and cuts the chain.
    let mut f = MaFile::default();
    f.append(&r(5, 0)).unwrap();
    f.append(&r(5, 10)).unwrap();
    f.append(&r(5, 0)).unwrap();
    let l = f.load(5, 9, lim).unwrap();
    assert!(l.cut);
    assert_eq!(l.trees[0].len(), 2);
    assert_eq!(l.trees[1].len(), 2);
    let again = f.load(5, 9, lim).unwrap();
    assert_eq!(again.trees[0].len(), 2);
    assert!(!again.cut);
    // A record of another layer stops the load and clears the link.
    let mut f = MaFile::default();
    let first = f.append(&r(5, 0)).unwrap() as usize;
    let second = f.append(&r(5, 0)).unwrap();
    f.bytes[second as usize + 4..second as usize + 8].copy_from_slice(&6u32.to_le_bytes());
    let l = f.load(5, 9, lim).unwrap();
    assert!(l.cut);
    assert_eq!(u32s(&f.bytes[first..first + 4]), [0]);
    // The town blob is checked against the town file's count.
    let mut f = MaFile::default();
    let mut big = r(5, 0);
    big.blobs[3] = vec![Cell::new(1, 0, 0)];
    f.append(&big).unwrap();
    assert!(f.load(5, 9, lim).unwrap().cut);
}

#[test]
fn only_unsaved_cells_are_written_and_teardown_saves() {
    let mut a = automap();
    a.limits = CelLimits {
        maxi: 100,
        town: [100; 4],
    };
    a.attach_file(MaFile::default(), 9);
    a.switch(1).unwrap();
    a.cells.tree_mut(TreeKind::Wall).insert(Cell::new(1, 0, 0));
    a.switch(3).unwrap();
    a.switch(1).unwrap();
    a.cells.tree_mut(TreeKind::Wall).insert(Cell::new(2, 0, 4));
    let seed = a.seed;
    let f = a.teardown().unwrap().unwrap();
    assert_eq!((a.current(), a.layers().len(), a.seed), (None, 0, seed));
    assert!(a.cells.tree(TreeKind::Wall).is_empty());
    // Layer 1 has two records: cel 1, then only the new cel 2.
    let mut f2 = f.clone();
    let l = f2.load(1, 9, a.limits).unwrap();
    assert_eq!(l.trees[1].iter().map(|c| c.cel).collect::<Vec<_>>(), [1, 2]);
}

// -----------------------------------------------------------------

#[test]
fn options_init_and_setters() {
    let d = Options::init(&MemoryStore::default());
    assert_eq!(
        (
            d.fade,
            d.fade_latch,
            d.centers,
            d.party,
            d.party_names,
            d.left
        ),
        (0, 0, true, true, true, true)
    );
    let mut store = MemoryStore(BTreeMap::from([(FADE.to_owned(), 1), (LEFT.to_owned(), 0)]));
    let mut o = Options::init(&store);
    assert_eq!((o.fade, o.fade_latch, o.left), (1, 0, false));
    // F10 cycles mod 4 and writes the value and both globals.
    for want in [2, 3, 0, 1] {
        o.cycle_fade(&mut store);
        assert_eq!(
            (o.fade, o.fade_latch, store.read(FADE)),
            (want, want, Some(want))
        );
    }
    o.toggle_party(&mut store);
    o.toggle_party_names(&mut store);
    o.toggle_left(&mut store);
    assert_eq!((o.party, o.party_names, o.left), (false, false, true));
    assert_eq!(store.read(LEFT), Some(1));
}

#[test]
fn size_and_recentre() {
    let f = frame();
    let mut a = automap();
    assert_eq!((a.view.mini, a.view.div), (false, 10));
    assert!(a.set_size(true, &f));
    assert!(!a.set_size(true, &f));
    // Mini, Left = 1, no down shift: (mx, my) = (0, 0); offset (W/3 − 16,
    // H/3 − 16).
    assert_eq!((a.view.div, a.view.offset), (20, (250, 184)));
    a.view.offset = (1, 1);
    // Centers off: only a forced re-centre runs.
    a.options.centers = false;
    a.toggled(false, &f);
    a.cleared(&f);
    assert_eq!(a.view.offset, (1, 1));
    a.toggled(true, &f);
    a.centre(&f);
    assert_eq!(a.view.offset, (250, 184));
    a.options.centers = true;
    a.set_size(false, &f);
    assert_eq!((a.view.div, a.view.offset), (10, (0, 0)));
}

#[test]
fn cel_file_names() {
    let p = |m, e| cel_paths(m, e).map(|p| p.map(|s| s.rsplit('\\').next().unwrap().to_owned()));
    let s = |v: &str| Some(v.to_owned());
    assert_eq!(
        p(false, true),
        [
            s("MaxiMap.dc6"),
            s("Act2Map.dc6"),
            s("Act4Map.dc6"),
            s("ExTnMap.dc6")
        ]
    );
    assert_eq!(p(false, false)[3], None);
    assert_eq!(
        p(true, false),
        [
            s("MaxiMapS.dc6"),
            s("Act2MapS.dc6"),
            s("Act4MapS.dc6"),
            s("ExTnMapS.dc6")
        ]
    );
    assert!(cel_paths(true, true)[0]
        .as_deref()
        .unwrap()
        .starts_with("DATA\\GLOBAL\\UI\\AutoMap\\"));
}

// -----------------------------------------------------------------

#[test]
fn marker_rectangle_and_mini_origin() {
    let mut f = frame();
    let mut o = Options::default();
    let mut v = View::new(&o);
    v.compute(&o, &f);
    assert_eq!(
        v.marker,
        Bounds {
            left: -16,
            top: -16,
            right: 800,
            bottom: 600
        }
    );
    v.mini = true;
    o.left = false;
    v.compute(&o, &f);
    assert_eq!(v.mini_origin, (533, 78));
    assert_eq!(
        v.marker,
        Bounds {
            left: 525,
            top: 78,
            right: 799,
            bottom: 278
        }
    );
    o.left = true;
    f.mini_down = true;
    v.compute(&o, &f);
    assert_eq!(v.mini_origin, (0, 96));
    // A change of Left (or of the down shift, mini) re-centres.
    let mut v = View::new(&o);
    v.mini = true;
    v.frame(&o, &f);
    assert_eq!(v.offset, (250, 88));
    v.offset = (0, 0);
    v.frame(&o, &f);
    assert_eq!(v.offset, (0, 0));
    o.left = false;
    v.frame(&o, &f);
    assert_eq!(v.offset, (266 - 533 - 16, 200 - 78 - 16));
}

#[test]
fn panel_side_forces_left_and_shifts_full_mode() {
    let mut f = frame();
    let mut o = Options {
        left: false,
        ..Options::default()
    };
    let mut v = View::new(&o);
    f.open_mode = 1;
    assert_eq!(v.panel_side(&mut o, &f), 200);
    assert!(o.left);
    f.open_mode = 2;
    assert_eq!(v.panel_side(&mut o, &f), -200);
    assert!(!o.left);
    f.open_mode = 1;
    v.panel_side(&mut o, &f);
    f.open_mode = 0;
    assert_eq!(v.panel_side(&mut o, &f), 0);
    assert!(!o.left, "restored");
    v.mini = true;
    f.open_mode = 1;
    assert_eq!(v.panel_side(&mut o, &f), 0);
}

#[test]
fn origin_and_cell_position() {
    let mut f = frame();
    f.unit_origin = pos(1000, 2000);
    let mut v = View::new(&Options::default());
    v.offset = (3, 4);
    // Ax = 3 + 40 + (100 − 400 + 200), Ay = 4 + 15 + (200 − 280).
    let a = v.origin(200, &f);
    assert_eq!(a, (-57, -61));
    assert_eq!(v.cell_screen(400, 600, a), (457, 661));
    v.div = 20;
    assert_eq!(v.cell_screen(-5, 7, (0, 0)), (-2, 3));
}

// ------------------------------------------------------------------

#[test]
fn blocks_and_windows() {
    let f = frame();
    let o = Options::default();
    let mut v = View::new(&o);
    let b = blocks(&v, TownKind::None, &f);
    assert_eq!(b.map(|b| b.tree), TreeKind::ALL);
    assert!(b
        .iter()
        .all(|b| b.file == CelFile::MaxiMap && b.d == (8, 16)));
    assert_eq!(
        b[0].window,
        Bounds {
            left: -16,
            top: -32,
            right: 816,
            bottom: 592
        }
    );
    let t = blocks(&v, TownKind::LutGholein, &f)[3];
    assert_eq!((t.file, t.d), (CelFile::Act2Map, (80, 50)));
    assert_eq!(
        t.window,
        Bounds {
            left: -160,
            top: -100,
            right: 960,
            bottom: 660
        }
    );
    assert_eq!(blocks(&v, TownKind::Pandemonium, &f)[3].d, (68, 45));
    assert_eq!(
        blocks(&v, TownKind::Harrogath, &f)[3].file,
        CelFile::ExTnMap
    );
    v.mini = true;
    v.compute(&o, &f);
    let b = blocks(&v, TownKind::Harrogath, &f);
    assert_eq!(b[0].d, (4, 8));
    assert_eq!(
        b[0].window,
        Bounds {
            left: -8,
            top: -16,
            right: 274,
            bottom: 202
        }
    );
    assert_eq!(
        b[3].window,
        Bounds {
            left: -180,
            top: -170,
            right: 446,
            bottom: 356
        }
    );
}

#[test]
fn walk_draws_ascending_inside_the_window() {
    let v = View::new(&Options::default());
    let mut t = CellTree::default();
    for (cel, x, y) in [
        (1, 0, 50),
        (2, 5, 10),
        (3, 900, 10),
        (6, 5, 10),
        (4, 0, -40),
        (5, 0, 700),
    ] {
        t.insert(Cell::new(cel, x, y));
    }
    let w = Bounds {
        left: -16,
        top: -32,
        right: 816,
        bottom: 592,
    };
    // Cel 6 at the same place as 2 is a duplicate (g(6) = 1 vs −1? no:
    // g(2) = 0, g(6) = 1: ordered after it).
    assert_eq!(
        walk(&t, &v, (0, 0), &w),
        [(2, 5, 10), (6, 5, 10), (1, 0, 50)]
    );
}

#[test]
fn fade_vectors() {
    let f = frame();
    let v = View::new(&Options::default());
    let b = blocks(&v, TownKind::LutGholein, &f);
    let fade = FadeFacts {
        v: 1,
        open_mode: 0,
        player_byte_18: 0,
    };
    // (sx, sy) = (X + 8, Y + 16).
    let m = |sx: i32, sy: i32| draw_mode(&v, &b[0], (sx - 8, sy - 16), fade, &f);
    assert_eq!(m(400, 270), 0);
    assert_eq!(m(470, 270), 1);
    assert_eq!(m(530, 270), 2);
    assert_eq!(m(541, 270), 5);
    assert_eq!(m(400, 270 - 151), 5);
    assert_eq!(m(400, 270 + 141), 5);
    // Open mode 1: the box moves by −W/4.
    let shifted = FadeFacts {
        open_mode: 1,
        ..fade
    };
    assert_eq!(draw_mode(&v, &b[0], (200 - 8, 270 - 16), shifted, &f), 0);
    // v = 2, 3, others.
    let with = |v2, byte| FadeFacts {
        v: v2,
        open_mode: 0,
        player_byte_18: byte,
    };
    assert_eq!(draw_mode(&v, &b[0], (0, 0), with(2, 0), &f), 1);
    assert_eq!(draw_mode(&v, &b[0], (0, 0), with(3, 0), &f), 2);
    assert_eq!(draw_mode(&v, &b[0], (0, 0), with(3, 2), &f), 2);
    assert_eq!(draw_mode(&v, &b[0], (0, 0), with(3, 1), &f), 5);
    assert_eq!(draw_mode(&v, &b[0], (0, 0), with(0, 0), &f), 5);
    let mut mini = v;
    mini.mini = true;
    // Mini honours the act byte too (was 1 for every byte).
    assert_eq!(draw_mode(&mini, &b[0], (0, 0), with(3, 1), &f), 5);
    assert_eq!(draw_mode(&mini, &b[0], (0, 0), with(3, 2), &f), 1);
    assert_eq!(draw_mode(&mini, &b[0], (0, 0), with(3, 0), &f), 1);
    // Clip rectangles.
    let mut o = Options::default();
    assert_eq!(
        clip_rect(&v, &o, &f),
        Bounds {
            left: 0,
            top: 0,
            right: 799,
            bottom: 599
        }
    );
    o.left = false;
    assert_eq!(
        clip_rect(&mini, &o, &f),
        Bounds {
            left: 519,
            top: 57,
            right: 798,
            bottom: 282
        }
    );
    o.left = true;
    assert_eq!(clip_rect(&mini, &o, &f).top, -21);
}

#[test]
fn town_art_never_fades() {
    let f = frame();
    let v = View::new(&Options::default());
    let b = blocks(&v, TownKind::LutGholein, &f);
    let fade = FadeFacts {
        v: 1,
        open_mode: 0,
        player_byte_18: 0,
    };
    assert_eq!(draw_mode(&v, &b[3], (400 - 80, 270 - 50), fade, &f), 5);
    // Kind 0 draws its town tree with MaxiMap, which fades.
    let b0 = blocks(&v, TownKind::None, &f);
    assert_eq!(draw_mode(&v, &b0[3], (400 - 8, 270 - 16), fade, &f), 0);
}

#[test]
fn mini_fade_one_becomes_two_in_a_fresh_session() {
    let mut store = MemoryStore(BTreeMap::from([(FADE.to_owned(), 1)]));
    let mut o = Options::init(&store);
    let mut v = View::new(&o);
    v.mini = true;
    assert_eq!(pass_fade(&v, &mut o, &mut store), 2);
    assert_eq!(store.read(FADE), Some(2));
    // After a write the latch is set: v = 1 becomes 0.
    o.set_fade(1, &mut store);
    assert_eq!(pass_fade(&v, &mut o, &mut store), 0);
    // Full mode keeps 1.
    o.set_fade(1, &mut store);
    v.mini = false;
    assert_eq!(pass_fade(&v, &mut o, &mut store), 1);
}

fn input<'a>(
    f: FrameFacts,
    markers: &'a [MarkerUnit],
    roster: &'a [RosterEntry],
    h: &'a HeaderFacts,
    strings: &'a dyn Fn(u16) -> Option<Vec<u16>>,
) -> PassInput<'a> {
    PassInput {
        frame: f,
        open: true,
        ready: true,
        player_byte_18: 0,
        markers,
        local_party: -1,
        player_gate: false,
        palette: MarkerPalette {
            index: [10, 11, 12, 13, 14, 15, 16, 18, 0],
        },
        roster,
        local_act: 0,
        header: h,
        strings,
    }
}

fn strings(id: u16) -> Option<Vec<u16>> {
    Some(format!("<{id}>").encode_utf16().collect())
}

#[test]
fn pass_order_and_gates() {
    let mut a = automap();
    a.switch(1).unwrap();
    a.cells.tree_mut(TreeKind::Floor).insert(Cell::new(0, 0, 0));
    let mut f = frame();
    f.unit_origin = pos(0, 0);
    let h = HeaderFacts::default();
    let m = [MarkerUnit {
        subject: MarkerSubject::Object {
            class: 59,
            target_level: None,
        },
        pos: pos(0, 0),
    }];
    let mut store = MemoryStore::default();
    let out = a
        .draw_pass(&input(f, &m, &[], &h, &strings), &mut store)
        .unwrap();
    // Cels, then the marker's 12 lines, then the header (version line).
    assert!(matches!(out[0], AutomapDraw::Cel { cel: 0, .. }));
    assert!(out[1..13]
        .iter()
        .all(|d| matches!(d, AutomapDraw::Line { .. })));
    assert!(matches!(out[13], AutomapDraw::Text { .. }));
    assert_eq!(out.len(), 14);
    // Closed, open mode 3, or no player/room: nothing.
    let mut i = input(f, &m, &[], &h, &strings);
    i.open = false;
    assert!(a.draw_pass(&i, &mut store).unwrap().is_empty());
    let mut i = input(f, &m, &[], &h, &strings);
    i.ready = false;
    assert!(a.draw_pass(&i, &mut store).unwrap().is_empty());
    f.open_mode = 3;
    assert!(a
        .draw_pass(&input(f, &m, &[], &h, &strings), &mut store)
        .unwrap()
        .is_empty());
}

// ------------------------------------------------------------------

fn ctx() -> MarkerCtx {
    MarkerCtx {
        local_party: 3,
        party: true,
        names: true,
        player_gate: false,
        mini: false,
        div: 10,
        a: (0, 0),
        rect: Bounds {
            left: -16,
            top: -16,
            right: 800,
            bottom: 600,
        },
        palette: MarkerPalette {
            index: [10, 11, 12, 13, 14, 15, 16, 18, 0],
        },
    }
}

fn player(local: bool, mode: u32, party: i16) -> MarkerSubject {
    MarkerSubject::Player {
        local,
        mode,
        party,
        own_inventory: false,
        state_7: true,
        name: vec![u16::from(b'P')],
    }
}

fn monster(class: u32, interact: bool, disguised: Option<bool>, relation: u8) -> MarkerSubject {
    MarkerSubject::Monster {
        class,
        mode: 1,
        flags: 0,
        interact,
        disguised,
        relation,
        name: vec![u16::from(b'M')],
        owner_name: vec![u16::from(b'O')],
    }
}

#[test]
fn marker_colours() {
    let c = ctx();
    use MarkerColor as K;
    assert_eq!(color(&player(true, 1, 3), &c), Some(K::B0));
    assert_eq!(color(&player(false, 1, 3), &c), Some(K::B3));
    assert_eq!(color(&player(false, 1, 4), &c), Some(K::B1));
    let mut nobody = c;
    nobody.local_party = -1;
    assert_eq!(color(&player(false, 1, -1), &nobody), Some(K::B1));
    assert_eq!(color(&player(false, 17, 3), &c), None);
    let corpse = MarkerSubject::Player {
        local: false,
        mode: 17,
        party: -1,
        own_inventory: true,
        state_7: true,
        name: vec![],
    };
    assert_eq!(color(&corpse, &c), Some(K::B2));
    assert_eq!(color(&monster(5, false, Some(true), 0), &c), Some(K::B4));
    assert_eq!(color(&monster(5, false, Some(false), 0), &c), Some(K::B1));
    assert_eq!(color(&monster(5, false, None, 1), &c), Some(K::B4));
    assert_eq!(color(&monster(5, false, None, 2), &c), Some(K::B5));
    let mut off = c;
    off.party = false;
    assert_eq!(color(&monster(5, false, None, 2), &off), None);
    assert_eq!(color(&monster(5, false, None, 0), &c), None);
    assert_eq!(color(&monster(5, false, None, 3), &c), None);
    assert_eq!(color(&monster(5, true, None, 0), &c), Some(K::B6));
    assert_eq!(color(&monster(538, true, None, 1), &c), None);
    let dead = MarkerSubject::Monster {
        class: 5,
        mode: 12,
        flags: 0,
        interact: true,
        disguised: None,
        relation: 1,
        name: vec![],
        owner_name: vec![],
    };
    assert_eq!(color(&dead, &c), None);
    let bit21 = MarkerSubject::Monster {
        class: 5,
        mode: 1,
        flags: 1 << 21,
        interact: true,
        disguised: None,
        relation: 1,
        name: vec![],
        owner_name: vec![],
    };
    assert_eq!(color(&bit21, &c), None);
    let obj = |class, target_level| MarkerSubject::Object {
        class,
        target_level,
    };
    assert_eq!(color(&obj(59, None), &c), Some(K::B8));
    assert_eq!(color(&obj(60, Some(1)), &c), Some(K::B8));
    for t in [111, 112, 117, 125, 126, 127] {
        assert_eq!(color(&obj(60, Some(t)), &c), None);
    }
    assert_eq!(color(&obj(267, None), &c), Some(K::Index0));
    assert_eq!(color(&obj(1, None), &c), None);
    assert_eq!(color(&MarkerSubject::Other, &c), None);
    assert_eq!(K::B4.rgb(), Some((0x44, 0x70, 0x74)));
    let p = MarkerPalette::new(|r, g, b| r ^ g ^ b);
    assert_eq!((p.get(K::B0), p.get(K::B8), p.get(K::Index0)), (255, 0, 0));
}

#[test]
fn cross_shape() {
    let mut out = Vec::new();
    cross(100, 50, false, 7, &mut out);
    assert_eq!(out.len(), 12);
    assert_eq!(
        out[0],
        AutomapDraw::Line {
            from: (100, 48),
            to: (104, 46),
            color: 7
        }
    );
    assert_eq!(
        out[11],
        AutomapDraw::Line {
            from: (96, 46),
            to: (100, 48),
            color: 7
        }
    );
    assert_eq!((SHAPE[0], SHAPE[12]), ((0, -1), (0, -1)));
    let mut mini = Vec::new();
    cross(100, 50, true, 7, &mut mini);
    assert_eq!(
        mini[0],
        AutomapDraw::Line {
            from: (99, 53),
            to: (103, 51),
            color: 7
        }
    );
}

fn lines(out: &[AutomapDraw]) -> usize {
    out.iter()
        .filter(|d| matches!(d, AutomapDraw::Line { .. }))
        .count()
}

fn texts(out: &[AutomapDraw]) -> Vec<(Label, u16, i32, i32)> {
    out.iter()
        .filter_map(|d| match d {
            AutomapDraw::Text {
                text, color, x, y, ..
            } => Some((text.clone(), *color, *x, *y)),
            _ => None,
        })
        .collect()
}

#[test]
fn marker_gate_and_rectangle() {
    let mut c = ctx();
    let at = |x, y| MarkerUnit {
        subject: player(true, 1, -1),
        pos: pos(x, y),
    };
    let mut out = Vec::new();
    // X = 1000 / 10 + 8 = 108, Y = 500 / 10 − 8 = 42.
    unit_markers(&[at(1000, 500)], &c, &mut out);
    assert_eq!(lines(&out), 12);
    assert!(matches!(
        out[0],
        AutomapDraw::Line {
            from: (108, 40),
            ..
        }
    ));
    // Outside the rectangle: nothing.
    out.clear();
    unit_markers(&[at(8000, 500), at(-300, 0)], &c, &mut out);
    assert!(out.is_empty());
    // The gate skips players without state 7.
    c.player_gate = true;
    let ghost = MarkerUnit {
        subject: MarkerSubject::Player {
            local: true,
            mode: 1,
            party: -1,
            own_inventory: false,
            state_7: false,
            name: vec![],
        },
        pos: pos(0, 0),
    };
    unit_markers(&[ghost], &c, &mut out);
    assert!(out.is_empty());
}

#[test]
fn marker_names() {
    let mut c = ctx();
    let u = |subject| MarkerUnit {
        subject,
        pos: pos(1000, 500),
    };
    let p = u16::from(b'P');
    let mut out = Vec::new();
    // A party member: cross and name in colour 2 (an immediate, not the
    // marker byte), top at Y − 10.
    unit_markers(&[u(player(false, 1, 3))], &c, &mut out);
    assert_eq!(lines(&out), 12);
    assert_eq!(texts(&out), [(Label::Text(vec![p]), 2, 108, 32)]);
    // A hostile player (B1) is named too, colour 1.
    out.clear();
    unit_markers(&[u(player(false, 1, 4))], &c, &mut out);
    assert_eq!(texts(&out), [(Label::Text(vec![p]), 1, 108, 32)]);
    // Names off: no name.
    let mut no_names = c;
    no_names.names = false;
    out.clear();
    unit_markers(&[u(player(false, 1, 4))], &no_names, &mut out);
    assert!(texts(&out).is_empty());
    // A disguised monster: the owner's name in colour 1 unless the owner
    // is in the local party.
    let o = u16::from(b'O');
    out.clear();
    unit_markers(&[u(monster(5, false, Some(false), 0))], &c, &mut out);
    assert_eq!(texts(&out), [(Label::Text(vec![o]), 1, 108, 32)]);
    out.clear();
    unit_markers(&[u(monster(5, false, Some(true), 0))], &c, &mut out);
    assert!(texts(&out).is_empty());
    // Independently of the interact name.
    out.clear();
    unit_markers(&[u(monster(5, true, Some(false), 0))], &c, &mut out);
    assert_eq!(texts(&out).iter().map(|t| t.1).collect::<Vec<_>>(), [4, 1]);
    // The local player: no name.
    out.clear();
    unit_markers(&[u(player(true, 1, 3))], &c, &mut out);
    assert!(texts(&out).is_empty());
    // Party off: no cross, no name for a party member.
    c.party = false;
    out.clear();
    unit_markers(&[u(player(false, 1, 3))], &c, &mut out);
    assert!(out.is_empty());
    c.party = true;
    // Interact monster: colour 4.
    out.clear();
    unit_markers(&[u(monster(5, true, None, 0))], &c, &mut out);
    assert_eq!(
        texts(&out),
        [(Label::Text(vec![u16::from(b'M')]), 4, 108, 32)]
    );
    // Object 267 with names: the string instead of a cross.
    out.clear();
    let o267 = MarkerSubject::Object {
        class: 267,
        target_level: None,
    };
    unit_markers(&[u(o267.clone())], &c, &mut out);
    assert_eq!(lines(&out), 0);
    assert_eq!(texts(&out), [(Label::String(0xCF3), 0, 108, 32)]);
    c.names = false;
    out.clear();
    unit_markers(&[u(o267), u(monster(5, true, None, 0))], &c, &mut out);
    assert_eq!((lines(&out), texts(&out).len()), (24, 0));
    // Empty names draw nothing; colours ≥ 13 become 0.
    out.clear();
    super::markers::name(&[], 0, 0, 1, &mut out);
    assert!(out.is_empty());
    super::markers::name(&[1], 0, 0, 12, &mut out);
    super::markers::name(&[1], 0, 0, 13, &mut out);
    assert_eq!(texts(&out).iter().map(|t| t.1).collect::<Vec<_>>(), [12, 0]);
}

// ------------------------------------------------------------------

#[test]
fn roster_crosses_for_party_members_without_units() {
    let c = ctx();
    let e = |party, act, has_unit| RosterEntry {
        party,
        act,
        // Subtiles (10, 5) → client (80, 120).
        x: 10,
        y: 5,
        has_unit,
        name: vec![u16::from(b'R')],
    };
    let mut out = Vec::new();
    roster_markers(&[e(3, 0, true), e(3, 0, false)], 0, &c, &mut out);
    assert_eq!(lines(&out), 12);
    assert!(matches!(
        out[0],
        AutomapDraw::Line {
            from: (16, 2),
            color: 13,
            ..
        }
    ));
    assert_eq!(texts(&out).len(), 1);
    // Other act, own unit, other party: nothing.
    out.clear();
    roster_markers(
        &[e(3, 0, true), e(3, 1, false), e(4, 0, false)],
        0,
        &c,
        &mut out,
    );
    assert!(out.is_empty());
    // A party id held by one roster entry only is no party.
    roster_markers(&[e(3, 0, false)], 0, &c, &mut out);
    assert!(out.is_empty());
    let mut off = c;
    off.party = false;
    roster_markers(&[e(3, 0, true), e(3, 0, false)], 0, &off, &mut out);
    assert!(out.is_empty());
}

// ------------------------------------------------------------------

#[test]
fn header_lines() {
    let s = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
    let f = HeaderFacts {
        game_name: s("g"),
        password: s("p"),
        level_name: s("Blood Moor"),
        difficulty: 2,
        game_type: 6,
        game_type_text: s("T"),
        expansion: true,
    };
    let mut out = Vec::new();
    header(&f, 800, &strings, &mut out).unwrap();
    let got: Vec<(String, i32, i32)> = out
        .iter()
        .map(|d| match d {
            AutomapDraw::Text {
                text: Label::Text(t),
                x,
                y,
                font: 1,
                color: 4,
                align: TextAlign::Right,
            } => (String::from_utf16(t).unwrap(), *x, *y),
            o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(
        got,
        [
            ("<4181>g".to_owned(), 784, 24),
            ("<4182>p".to_owned(), 784, 40),
            ("Blood Moor".to_owned(), 784, 56),
            (format!("D2RUST {}", crate::launch::VERSION), 784, 72),
            ("<4183><5155>".to_owned(), 784, 88),
            ("T".to_owned(), 784, 104),
            ("<22730>".to_owned(), 784, 120),
        ]
    );
    // Normal, classic, no game name: only the version line.
    let mut out = Vec::new();
    header(&HeaderFacts::default(), 800, &strings, &mut out).unwrap();
    assert_eq!(out.len(), 1);
    // Nightmare uses 5154; a line over 299 units is fatal 0xB2A.
    let nm = HeaderFacts {
        difficulty: 1,
        ..HeaderFacts::default()
    };
    let mut out = Vec::new();
    header(&nm, 800, &strings, &mut out).unwrap();
    assert!(
        matches!(&out[1], AutomapDraw::Text { text: Label::Text(t), .. } if String::from_utf16(t).unwrap() == "<4183><5154>")
    );
    let long = |id: u16| Some(vec![u16::from(b'x'); if id == 5154 { 296 } else { 4 }]);
    assert!(matches!(
        header(&nm, 800, &long, &mut Vec::new()),
        Err(AutomapError::Fatal { code: 0xB2A, .. })
    ));
}

// ------------------------------------------------------------------

#[test]
fn init_state() {
    let a = automap();
    assert_eq!((a.current(), a.seed, a.view.div), (None, SEED, 10));
    assert_eq!(a.reveal, RevealState::default());
    // Act load makes the colour indices from the act palette.
    let p = MarkerPalette::new(|r, _, _| r);
    assert_eq!(p.get(MarkerColor::B1), 255);
}

#[test]
fn dead_gate_and_mini_down_follow_their_answers() {
    use super::markers::{unit_dead, DeadKind};
    use super::view::mini_down;
    assert!(unit_dead(DeadKind::Player, 17, 0));
    assert!(unit_dead(DeadKind::Player, 0, 0));
    assert!(!unit_dead(DeadKind::Player, 12, 0));
    assert!(unit_dead(DeadKind::Monster, 12, 0));
    assert!(unit_dead(DeadKind::Monster, 0, 0));
    assert!(!unit_dead(DeadKind::Monster, 17, 0));
    // Other types: only the flag (bit 16).
    assert!(!unit_dead(DeadKind::Other, 0, 0));
    assert!(unit_dead(DeadKind::Other, 1, 0x1_0000));
    assert!(unit_dead(DeadKind::Player, 1, 0x1_0000));
    assert!(!unit_dead(DeadKind::Player, 1, 0x2_0000));
    assert!(mini_down(0) && mini_down(1) && !mini_down(2));
}

#[test]
fn two_records_of_layer_2_link_from_the_first_u32() {
    let mut f = MaFile::default();
    let r = |n: i16| MaRecord {
        layer: 2,
        town_kind: 0,
        act2: 9,
        blobs: [vec![Cell::new(n, 0, 0)], vec![], vec![], vec![]],
    };
    f.append(&r(1)).unwrap();
    f.append(&r(2)).unwrap();
    assert_eq!(u32s(&f.bytes[8..12]), [400]);
    assert_eq!(u32s(&f.bytes[400..404]), [400 + 32 + 6]);
    assert_eq!(u32s(&f.bytes[404..408]), [2]);
    let lim = CelLimits {
        maxi: 100,
        town: [100; 4],
    };
    let l = f.load(2, 9, lim).unwrap();
    assert_eq!(l.trees[0].iter().map(|c| c.cel).collect::<Vec<_>>(), [1, 2]);
}

// ------------------------------------------------------------------ 16:9

/// The 16:9 frame (1068 × 600) keeps the world between two open side
/// panels, so the automap shows in every open mode there; the mini map
/// then sits centred in the gap. At 4:3 both panels hide it as before.
#[test]
fn the_wide_frame_shows_the_automap_with_both_panels_open() {
    let wide = |open_mode| FrameFacts {
        width: 1068,
        open_mode,
        ..frame()
    };
    let narrow = |open_mode| FrameFacts {
        open_mode,
        ..frame()
    };
    for m in 0..=3 {
        assert!(wide(m).draws_world(), "mode {m}");
        assert_eq!(narrow(m).draws_world(), m != 3);
        assert_eq!(wide(m).mini_centred(), m == 3);
        assert!(!narrow(m).mini_centred());
    }
    // An open side panel moves the full map by half its 320 columns.
    assert_eq!(wide(1).panel_shift(), 160);
    assert_eq!(narrow(1).panel_shift(), 200);
    // Mode 3 at 16:9: the mini map (W / 3 wide) and its clip centred.
    let o = Options::default();
    let mut v = View::new(&o);
    v.mini = true;
    v.compute(&o, &wide(3));
    assert_eq!(v.mini_origin, ((1068 - 356) / 2, 78));
    let c = clip_rect(&v, &o, &wide(3));
    assert_eq!(
        (c.left, c.right),
        ((1068 - 279) / 2, (1068 - 279) / 2 + 279)
    );
    // Entering it re-centres the mini map on its new origin.
    let mut v = View::new(&o);
    assert!(v.set_size(true, &o, &wide(0)));
    v.frame(&o, &wide(3));
    let (mx, my) = v.mini_origin;
    assert_eq!(v.offset, (1068 / 3 - mx - 16, 600 / 3 - my - 16));
}
