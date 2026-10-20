//! Mutation-testing tests for `drlg::maze`: small, exact checks of the rect helpers,
//! shape tables, builders per level, special stamps, spiral, lava cross and build
//! rules, each written so that changing a single operator or constant fails it.

use d2_data::tables::{Lvlmaze, Lvlprest, Record};

use super::*;
use crate::drlg::{LevelDef, NoLevelTypes};
use crate::rng::Seed;

/// Start seed of the test character.
const START: u32 = 4014346869;

/// A preset seam that logs every map (def, file written by maze code)
/// and draws the default `roll(Files)` (Files = 1) on the level seed.
#[derive(Default)]
struct Log {
    direction: u32,
    maps: Vec<(u32, Option<i32>)>,
    small: Vec<bool>,
}

impl MazePresets for Log {
    fn level(&mut self, drlg: &mut Drlg, data: &DrlgData, id: u32) -> Result<LevelIdx, DrlgError> {
        drlg.get_or_alloc_level(data, &mut NoLevelTypes, id)
    }

    fn preset_direction(&self, _: &Drlg, _: LevelIdx) -> Result<u32, DrlgError> {
        Ok(self.direction)
    }

    fn alloc_map(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
        def: u32,
        _: TileRect,
    ) -> Result<MapId, DrlgError> {
        drlg.level_mut(level).seed.roll(1);
        self.maps.push((def, None));
        Ok(MapId(self.maps.len() as u32 - 1))
    }

    fn set_map_file(&mut self, map: MapId, file: i32) {
        self.maps[map.0 as usize].1 = Some(file);
    }

    fn build_map(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
        _: MapId,
        small: bool,
        _: &[MazeLink],
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        self.small.push(small);
        Ok(Some(super::tests::fake_room(
            drlg,
            level,
            TileRect::default(),
        )))
    }
}

/// Generates maze level `id` of `level_type` (act index `act`, level
/// rect 0, 0, 400, 400, cells `size` × `size`, `rooms` rooms; staff tomb
/// 66, boss tomb 67) and returns
/// the logged maps in build order, or the error.
fn run(
    act: u8,
    id: u32,
    level_type: u32,
    rooms: u32,
    size: i32,
) -> Result<Vec<(u32, Option<i32>)>, MazeError> {
    let data = DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    };
    let mut d = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    d.act = act;
    d.start_seed = START;
    (d.staff_tomb, d.boss_tomb) = (66, 67);
    let l = d.get_or_alloc_level(&data, &mut NoLevelTypes, id).unwrap();
    let lv = d.level_mut(l);
    lv.drlg_type = 1;
    lv.level_type = level_type;
    lv.rect = TileRect::new(0, 0, 400, 400);
    lv.seed = Seed::init_low(START.wrapping_add(id));
    let mut m = Maze::new(MazeData {
        rows: vec![MazeRow {
            level: id,
            rooms: [rooms; 3],
            size_x: size,
            size_y: size,
            merge: 0,
        }],
        prest_files: (0..1200).map(|d| (d, 1)).collect(),
        specials: Specials::shipped(),
    });
    m.init_level(&d, l)?;
    let mut log = Log::default();
    m.generate(&mut d, &data, &mut log, l)?;
    Ok(log.maps)
}

fn u32s(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// The table view takes the lvlmaze columns (`Level`, `Rooms[3]`, `SizeX`,
/// `SizeY`, `Merge`) in file order; lvlprest `Files` by `Def`, first row
/// of each def; the shipped special-room tables.
#[test]
fn table_view_from_records() {
    let mut m = vec![0u8; Lvlmaze::SIZE];
    for (i, v) in [8u32, 3, 4, 5, 10, 12, 100].into_iter().enumerate() {
        u32s(&mut m, 4 * i, v);
    }
    let mut p1 = vec![0u8; Lvlprest::SIZE];
    u32s(&mut p1, 0, 9);
    u32s(&mut p1, 64, 6);
    let mut p2 = p1.clone();
    u32s(&mut p2, 64, 2);
    let d = MazeData::from_tables(
        &[Lvlmaze::decode(&m)],
        &[Lvlprest::decode(&p1), Lvlprest::decode(&p2)],
    );
    assert_eq!(
        d.rows,
        [MazeRow {
            level: 8,
            rooms: [3, 4, 5],
            size_x: 10,
            size_y: 12,
            merge: 100,
        }]
    );
    assert_eq!(d.row_index(8), Ok(0));
    assert_eq!(d.files(9), Ok(6));
    assert_eq!(d.specials, Specials::shipped());
}

// ---- cells.rs ---------------------------------------------------------------

/// Gaps: dx uses `B.x − A.w − A.x` only when `A.x < B.x`,
/// else `A.x − B.w − B.x`; dy likewise.
#[test]
fn overlap_gaps() {
    use super::cells::gaps;
    let a = TileRect::new(0, 0, 8, 8);
    // Equal origins: the else branches, with B's size.
    assert_eq!(gaps(&a, &TileRect::new(0, 0, 4, 2)), (-4, -2));
    // A below and right of B.
    let a = TileRect::new(10, 10, 4, 4);
    assert_eq!(gaps(&a, &TileRect::new(3, 2, 5, 6)), (2, 2));
    // B below and right of A.
    assert_eq!(gaps(&a, &TileRect::new(20, 20, 5, 6)), (6, 6));
}

/// Direction: W needs B.x < A.x **and** A.x = B.x + B.w; a room
/// left of A that is not flush falls through to the y tests.
#[test]
fn direction_needs_both_conditions() {
    use super::cells::direction;
    let a = TileRect::new(8, 8, 8, 8);
    // Above A, overlapping in x: north.
    assert_eq!(direction(&a, &TileRect::new(4, 0, 8, 8)), 1);
    // Flush on the left: west.
    assert_eq!(direction(&a, &TileRect::new(0, 4, 8, 8)), 0);
    // Left, not flush, not touching in y: none.
    assert_eq!(direction(&a, &TileRect::new(0, 20, 4, 4)), -1);
}

/// Direction: N needs B.y < A.y **and** A.y = B.y + B.h.
#[test]
fn direction_north_needs_both_conditions() {
    use super::cells::direction;
    let a = TileRect::new(8, 8, 8, 8);
    assert_eq!(direction(&a, &TileRect::new(20, 0, 4, 4)), -1);
    assert_eq!(direction(&a, &TileRect::new(20, 4, 4, 4)), 1);
}

/// Shape def: the def by level type, every row of the base
/// + mask table and every cell of the two remap tables.
#[test]
fn shape_def_full_table() {
    use super::cells::shape_def;
    let bases = [
        (3, 52),
        (4, 108),
        (7, 167),
        (8, 205),
        (10, 257),
        (13, 301),
        (17, 413),
        (18, 481),
        (19, 509),
        (22, 753),
        (24, 664),
        (25, 704),
        (28, 836),
        (33, 1002),
        (34, 1058),
    ];
    for (t, base) in bases {
        for mask in [0, 3, 15] {
            assert_eq!(shape_def(t, mask, false), Ok(base + mask), "type {t}");
        }
    }
    let masks = [1, 2, 3, 4, 5, 6, 8, 9, 10, 12];
    let remaps: [(u32, [u32; 10]); 5] = [
        (14, [0, 0, 0, 0, 356, 355, 0, 357, 354, 0]),
        (15, [0, 0, 0, 0, 360, 359, 0, 361, 358, 0]),
        (23, [0, 0, 0, 0, 659, 660, 0, 661, 662, 0]),
        (32, [0, 0, 0, 0, 1045, 1044, 0, 1043, 1042, 0]),
        (35, [1056, 1055, 1057, 1054, 0, 0, 1053, 0, 0, 1058]),
    ];
    for (t, defs) in remaps {
        for (mask, def) in masks.into_iter().zip(defs) {
            assert_eq!(shape_def(t, mask, false), Ok(def), "type {t} mask {mask}");
        }
    }
}

// ---- layout.rs: builders by level ------------------------------------

/// Tomb: Claw Viper Temple 2 (61) is the single cell def 480 with file −1 (the
/// default file stays, since 480 is outside the rotation range); another tomb level
/// runs the hub builder.
#[test]
fn claw_viper_temple_2_is_one_fixed_cell() {
    assert_eq!(run(1, 61, 17, 10, 24), Ok(vec![(480, None)]));
    let other = run(1, 60, 17, 10, 24).unwrap();
    assert!(other.len() > 1);
    assert!(other.iter().all(|&(def, _)| def != 480));
}

/// Ice: Cellar of Pity (114) draws `roll(2)` (the
/// second level-seed step, after F's allocation): 1038 if non-zero, else
/// 1039; Echo Chamber (116) 1040; Glacial Caves 2 (119) 1041; file −1,
/// so the map keeps its default file.
#[test]
fn ice_single_cell_levels() {
    let mut s = Seed::init_low(START + 114);
    s.step();
    let def = if s.roll(2) != 0 { 1038 } else { 1039 };
    assert_eq!(run(4, 114, 33, 10, 24), Ok(vec![(def, None)]));
    assert_eq!(run(4, 116, 33, 10, 24), Ok(vec![(1040, None)]));
    assert_eq!(run(4, 119, 33, 10, 24), Ok(vec![(1041, None)]));
}

/// Sewers 1 (92) runs ring(5) and the four corner swaps
/// (709 → 735, 710 → 736, 713 → 737, 714 → 738); another Act 3 sewer
/// level does not.
#[test]
fn act3_sewers_1_corner_swaps() {
    let swapped = [735, 736, 737, 738];
    let maps = run(2, 92, 25, 30, 24).unwrap();
    for def in swapped {
        // File −1: the map keeps its default file.
        assert_eq!(maps.iter().filter(|m| m.0 == def).count(), 1, "def {def}");
        assert!(maps.contains(&(def, None)), "def {def}");
    }
    let maps = run(2, 93, 25, 30, 24).unwrap();
    assert!(maps.iter().all(|m| !swapped.contains(&m.0)));
}

// ---- cells.rs: Gen primitives ----------------------------------------------

use super::cells::{Extreme, Gen};
use super::layout;
use crate::drlg::DrlgRoomId;

/// A DRLG with maze level 8 (or `id`) of `level_type` at (0, 0, 1000,
/// 1000) and a second level 9 (for cross-level links).
fn env(level_type: u32) -> (Drlg, LevelIdx, LevelIdx) {
    env_id(level_type, 8)
}

fn env_id(level_type: u32, id: u32) -> (Drlg, LevelIdx, LevelIdx) {
    let data = DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    };
    let mut d = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    let l = d.get_or_alloc_level(&data, &mut NoLevelTypes, id).unwrap();
    d.level_mut(l).level_type = level_type;
    d.level_mut(l).rect = TileRect::new(0, 0, 1000, 1000);
    let l9 = d.get_or_alloc_level(&data, &mut NoLevelTypes, 9).unwrap();
    (d, l, l9)
}

fn mrow(rooms: u32, merge: i32) -> MazeRow {
    MazeRow {
        level: 8,
        rooms: [rooms; 3],
        size_x: 10,
        size_y: 10,
        merge,
    }
}

/// Cells at `at` (10 × 10), added so that the list order is `at`'s order.
fn cells(g: &mut Gen<'_>, at: &[(i32, i32)]) -> Vec<DrlgRoomId> {
    let mut out = Vec::new();
    for &(x, y) in at.iter().rev() {
        let c = g.alloc();
        g.drlg.room_mut(c).rect.x = x;
        g.drlg.room_mut(c).rect.y = y;
        g.add(c);
        out.insert(0, c);
    }
    assert_eq!(g.list(), out);
    out
}

/// Place test: the new cell may not collide with the box of
/// any of P's links, a cross-level link's level rect included.
#[test]
fn place_rejects_a_link_targets_box() {
    let md = MazeData::default();
    let (mut d, l, l9) = env(3);
    d.level_mut(l9).rect = TileRect::new(110, 100, 20, 20);
    let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
    let f = cells(&mut g, &[(100, 100)])[0];
    assert!(g.place(f, 2).is_some(), "free before the link");
    g.link_level(f, l9, 2);
    assert_eq!(g.place(f, 2), None);
    // Other directions stay free.
    assert!(g.place(f, 3).is_some());
}

/// Pick: the mask ORs the links' bits (two links west
/// count once); Ice with lvlmaze `Rooms[d]` = 1 takes the raw mask.
#[test]
fn pick_mask_and_ice_rooms_one() {
    let md = MazeData::default();
    let (mut d, l, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
    let c = cells(&mut g, &[(100, 100), (90, 0), (90, 200)]);
    g.link(c[0], c[1], 0);
    g.link(c[0], c[2], 0);
    g.pick(c[0]).unwrap();
    assert_eq!(g.cell(c[0]).def, 52 + 1);
    for (rooms, def) in [(1, 1), (2, 1003)] {
        let (mut d, l, _) = env(33);
        let mut g = Gen::new(&mut d, l, mrow(rooms, 0), &md).unwrap();
        let c = cells(&mut g, &[(100, 100), (90, 100)]);
        g.link(c[0], c[1], 0);
        g.pick(c[0]).unwrap();
        assert_eq!(g.cell(c[0]).def, def, "Rooms {rooms}");
    }
}

/// Merge: a room at gap 1 does not collide at margin 1
/// (no draw on its seed); `lo' mod 1000 < Merge` is strict; a merge to
/// the north (direction 1) links.
#[test]
fn merge_bounds_and_north_link() {
    let md = MazeData::default();
    // Gap 1 on x, then on y: R's seed untouched.
    for at in [(11, 0), (0, 11)] {
        let (mut d, l, _) = env(3);
        let mut g = Gen::new(&mut d, l, mrow(10, 1000), &md).unwrap();
        let r = cells(&mut g, &[(0, 0)])[0];
        let n = g.alloc();
        g.drlg.room_mut(n).rect.x = at.0;
        g.drlg.room_mut(n).rect.y = at.1;
        let before = g.drlg.room(r).seed;
        g.merge(n).unwrap();
        assert_eq!(g.drlg.room(r).seed, before, "{at:?}");
        assert!(g.cell(r).links.is_empty());
    }
    // Touching on the east: Merge = lo' mod 1000 does not link.
    let (mut d, l, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
    let r = cells(&mut g, &[(0, 0)])[0];
    let lo = g.drlg.room(r).seed.clone().step();
    g.row.merge = (lo % 1000) as i32;
    let n = g.alloc();
    g.drlg.room_mut(n).rect.x = 10;
    g.merge(n).unwrap();
    assert_eq!(g.drlg.room(r).seed.lo, lo);
    assert!(g.cell(r).links.is_empty());
    // N north of R, Merge 1000: R links N with direction 1.
    let (mut d, l, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(10, 1000), &md).unwrap();
    let r = cells(&mut g, &[(0, 10)])[0];
    let n = g.alloc();
    g.merge(n).unwrap();
    let links: Vec<_> = g.cell(r).links.iter().map(|k| (k.target, k.dir)).collect();
    assert_eq!(links, [(LinkTarget::Cell(n), 1)]);
}

/// "blank" attach: def, file −1, lock, no pick.
#[test]
fn blank_attach_sets_file_minus_one() {
    let md = MazeData::default();
    let (mut d, l, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
    let p = cells(&mut g, &[(100, 100)])[0];
    let n = g.blank(p, 2, 77).unwrap();
    let c = g.cell(n);
    assert_eq!((c.def, c.file, c.lock), (77, -1, true));
}

/// Extreme finders: the finders keep the first cell (list order)
/// at the extreme, probing only on a strict improvement, and skip a cell
/// whose probe fails; probe directions N, E, S, W.
#[test]
fn extreme_finders() {
    use Extreme::*;
    assert_eq!([MinY, MaxX, MaxY, MinX].map(Extreme::dir), [1, 2, 3, 0]);
    let md = MazeData::default();
    // (finder, a better value, the start value) on the compared axis; the
    // cells are 100 apart on the other axis.
    for (which, better, start) in [
        (MinY, 100, 300),
        (MaxX, 300, 100),
        (MaxY, 300, 100),
        (MinX, 100, 300),
    ] {
        let at = |v: i32, k: i32| match which {
            MinY | MaxY => (300 + 100 * k, v),
            MaxX | MinX => (v, 300 + 100 * k),
        };
        // A tie keeps the first cell.
        let (mut d, l, _) = env(3);
        let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
        let c = cells(&mut g, &[at(start, 0), at(start, 1)]);
        assert_eq!(g.extreme(which).unwrap(), Some(c[0]), "{which:?} tie");
        // A strictly better later cell wins.
        let (mut d, l, _) = env(3);
        let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
        let c = cells(&mut g, &[at(start, 0), at(better, 1)]);
        assert_eq!(g.extreme(which).unwrap(), Some(c[1]), "{which:?} better");
        // A locked cell fails its probe: no best.
        let (mut d, l, _) = env(3);
        let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
        let c = cells(&mut g, &[at(start, 0)]);
        g.cell_mut(c[0]).lock = true;
        assert_eq!(g.extreme(which).unwrap(), None, "{which:?} locked");
    }
}

/// Bounding box: it spans min x, min y to max x + w,
/// max y + h over the list, whatever the first cell is.
#[test]
fn bounding_box_of_the_list() {
    let md = MazeData::default();
    let (mut d, l, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
    cells(&mut g, &[(20, 30), (0, 0)]);
    assert_eq!(g.bounding_box(), TileRect::new(0, 0, 30, 40));
}

// ---- layout.rs: special stamps by level -----------------------------------

/// Each level stamps exactly its listed
/// special tables (counted by the specials they place) and no other table
/// of its family.
#[test]
fn special_stamps_by_level() {
    let specials = Specials::shipped();
    let count = |maps: &[(u32, Option<i32>)], kind: &str| {
        let defs: Vec<u32> = specials
            .table(kind)
            .unwrap()
            .iter()
            .map(|r| r.special)
            .collect();
        maps.iter().filter(|m| defs.contains(&m.0)).count()
    };
    let cases: &[(u32, u32, &str, &[&str])] = &[
        (8, 3, "cave_", &["cave_prev", "cave_doe"]),
        (9, 3, "cave_", &["cave_prev", "cave_down", "cave_coldcrow"]),
        (10, 3, "cave_", &["cave_prev", "cave_down", "cave_next"]),
        (11, 3, "cave_", &["cave_prev", "cave_down"]),
        (12, 3, "cave_", &["cave_prev", "cave_down"]),
        (18, 4, "crypt_", &["crypt_prev", "crypt_bonebreak"]),
        (19, 4, "crypt_", &["crypt_prev", "crypt_chest"]),
        (133, 4, "crypt_", &["crypt_prev", "crypt_chest"]),
        (21, 4, "crypt_", &["crypt_prev", "crypt_next"]),
        (24, 4, "crypt_", &["crypt_prev", "crypt_next"]),
        (29, 8, "jail_", &["jail_prev", "jail_waypoint", "jail_next"]),
        (30, 8, "jail_", &["jail_prev", "jail_pitspawn", "jail_next"]),
        (31, 8, "jail_", &["jail_prev", "jail_cath"]),
        (34, 10, "catacombs_", &["catacombs_next"]),
        (
            35,
            10,
            "catacombs_",
            &["catacombs_next", "catacombs_waypoint"],
        ),
        (36, 10, "catacombs_", &["catacombs_next"]),
        (74, 19, "arcane_", &["arcane_summoner"]),
        (86, 24, "dungeon_", &["dungeon_prev", "dungeon_next"]),
        (89, 24, "dungeon_", &["dungeon_prev", "dungeon_next"]),
        (92, 25, "a3sewer_", &["a3sewer_drain", "a3sewer_chest"]),
        // The builder stamps both unconditionally.
        (93, 25, "a3sewer_", &["a3sewer_drain", "a3sewer_chest"]),
        (13, 3, "cave_", &["cave_prev", "cave_down"]),
        (16, 3, "cave_", &["cave_prev", "cave_down"]),
        (25, 4, "crypt_", &["crypt_prev"]),
        (37, 10, "catacombs_", &["catacombs_next"]),
        (90, 24, "dungeon_", &["dungeon_prev", "dungeon_next"]),
        (91, 24, "dungeon_", &["dungeon_prev", "dungeon_next"]),
        (132, 34, "baal_", &["baal_next"]),
        (100, 22, "meph_", &["meph_prev", "meph_next"]),
        (
            101,
            22,
            "meph_",
            &["meph_prev", "meph_waypoint", "meph_next"],
        ),
        (128, 34, "baal_", &["baal_next"]),
        (129, 34, "baal_", &["baal_next", "baal_waypoint"]),
        (130, 34, "baal_", &["baal_next"]),
        (
            113,
            33,
            "ice_",
            &["ice_prev", "ice_next", "ice_down", "ice_waypoint"],
        ),
        (
            118,
            33,
            "ice_",
            &["ice_prev", "ice_next", "ice_down", "ice_waypoint"],
        ),
        (
            115,
            33,
            "ice_",
            &[
                "ice_prev",
                "ice_next",
                "ice_down",
                "ice_theme",
                "ice_waypoint",
            ],
        ),
        (62, 18, "lair_", &["lair_next", "lair_prev"]),
        (63, 18, "lair_", &["lair_next", "lair_prev"]),
        (
            64,
            18,
            "lair_",
            &["lair_tightspot", "lair_treasure", "lair_prev"],
        ),
        (
            48,
            13,
            "a2sewer_",
            &["a2sewer_prev", "a2sewer_waypoint", "a2sewer_next"],
        ),
        (49, 13, "a2sewer_", &["a2sewer_prev", "a2sewer_radament"]),
        (65, 13, "a2sewer_", &["a2sewer_prev", "a2sewer_chest"]),
        (55, 17, "tomb_", &["tomb_next"]),
        (57, 17, "tomb_", &["tomb_next", "tomb_waypoint"]),
        (58, 17, "tomb_", &["tomb_next"]),
        (
            59,
            17,
            "tomb_",
            &["tomb_chest", "tomb_leatherarm", "tomb_treasure"],
        ),
        (60, 17, "tomb_", &["tomb_cube"]),
        (66, 17, "tomb_", &["tomb_talrasha"]),
        (67, 17, "tomb_", &["tomb_chest", "tomb_kaa"]),
        (68, 17, "tomb_", &["tomb_chest"]),
        (122, 32, "temple_", &["temple_down"]),
        (123, 32, "temple_", &["temple_down", "temple_waypoint"]),
    ];
    for &(id, ty, family, want) in cases {
        let maps = run(0, id, ty, 12, 10).unwrap_or_else(|e| panic!("level {id}: {e:?}"));
        for kind in specials.kinds().filter(|k| k.starts_with(family)) {
            let n = want.iter().filter(|&&w| w == kind).count();
            assert_eq!(count(&maps, kind), n, "level {id} {kind}");
        }
    }
}

// ---- layout.rs: layout builders ---------------------------------------------

/// The cells linked from `c` as (direction, def, file, lock).
fn neighbours(g: &Gen<'_>, c: DrlgRoomId) -> Vec<(u8, u32, i32, bool)> {
    g.cell(c)
        .links
        .iter()
        .filter_map(|l| match l.target {
            LinkTarget::Cell(n) => {
                let x = g.cell(n);
                Some((l.dir, x.def, x.file, x.lock))
            }
            LinkTarget::Level(_) => None,
        })
        .collect()
}

/// Hub: r = level-seed step & 3; grow from F in
/// directions r, r+1, r+2 (mod 4); F := table[(r+3) mod 4], file −1,
/// locked.
#[test]
fn hub_grows_three_sides_and_sets_the_open_side_def() {
    let md = MazeData::default();
    let table = [447, 444, 446, 445];
    for seed in 0..8 {
        let (mut d, l, _) = env(17);
        d.level_mut(l).seed = Seed::init_low(seed);
        let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
        let f = cells(&mut g, &[(500, 500)])[0];
        let r = g.drlg.level(l).seed.clone().step() & 3;
        layout::hub(&mut g, f, &table).unwrap();
        let mut dirs: Vec<u8> = neighbours(&g, f).iter().map(|n| n.0).collect();
        dirs.sort();
        let mut want: Vec<u8> = (0..3).map(|i| ((r + i) % 4) as u8).collect();
        want.sort();
        assert_eq!(dirs, want, "seed {seed}");
        let c = g.cell(f);
        assert_eq!(
            (c.def, c.file, c.lock),
            (table[((r + 3) % 4) as usize], -1, true)
        );
    }
}

/// Lava cross: s = one step of F's room seed & 3; "fixed"
/// cells from rows 2s and 2s+1 (def, dir, file), then blanks of def 836
/// around every cell.
#[test]
fn lava_cross_rows_by_f_seed() {
    let md = MazeData::default();
    let rows: [(u32, u8, i32); 8] = [
        (1054, 1, 0),
        (1053, 3, 1),
        (1054, 1, 1),
        (1053, 3, 0),
        (1055, 0, 1),
        (1056, 2, 0),
        (1055, 0, 0),
        (1056, 2, 1),
    ];
    let mut seen = [false; 4];
    for k in 0..32u32 {
        let seed = START.wrapping_add(k.wrapping_mul(7919));
        let (mut d, l, _) = env(35);
        d.level_mut(l).seed = Seed::init_low(seed);
        let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
        let f = cells(&mut g, &[(500, 500)])[0];
        let s = (g.drlg.room(f).seed.clone().step() & 3) as usize;
        seen[s] = true;
        layout::lava_cross(&mut g, f).unwrap();
        let fixed: Vec<_> = neighbours(&g, f)
            .into_iter()
            .filter(|n| n.1 != 836)
            .map(|n| (n.1, n.0, n.2))
            .collect();
        let mut want = vec![rows[2 * s], rows[2 * s + 1]];
        let mut got = fixed;
        want.sort();
        got.sort();
        assert_eq!(got, want, "seed {seed}");
    }
    assert_eq!(seen, [true; 4]);
}

/// Spiral: cell k = 9 grows from cell 7 (8 is a dead end);
/// every kept cell but 8 and 12 gets file (r + b) mod 4, F file 4.
#[test]
fn arcane_spiral_dead_ends() {
    let md = MazeData::default();
    let (mut d, l, _) = env(19);
    let mut g = Gen::new(&mut d, l, mrow(61, 0), &md).unwrap();
    let f = cells(&mut g, &[(500, 500)])[0];
    layout::spiral(&mut g, f).unwrap();
    assert_eq!(g.count(), 61);
    assert_eq!(g.cell(f).file, 4);
    // Cells 8 and 12 of each branch keep file −1: 8 cells.
    let minus_one = g.list().iter().filter(|&&c| g.cell(c).file == -1).count();
    assert_eq!(minus_one, 8);
    // A dead end has one link; cell 9's parent (7) has three (6, 8, 9).
    let ends = g
        .list()
        .iter()
        .filter(|&&c| g.cell(c).file == -1)
        .all(|&c| g.cell(c).links.len() == 1);
    assert!(ends);
}

/// Act 2 Sewers 1 (47): a = lo₁, e = (a & 1)·2 + 1; C gets the
/// "fixed" 333 (file 0) to its west; G = "fixed" 336 (file 0) from E2 in
/// direction e; H next to G in direction e.
#[test]
fn act2_sewers_1_special_path() {
    let md = MazeData {
        specials: Specials::shipped(),
        ..MazeData::default()
    };
    let mut seen = [false; 2];
    for k in 0..12u32 {
        let (mut d, l, _) = env_id(13, 47);
        d.level_mut(l).seed = Seed::init_low(START.wrapping_add(k.wrapping_mul(7919)));
        let mut g = Gen::new(&mut d, l, mrow(12, 0), &md).unwrap();
        let f = cells(&mut g, &[(500, 500)])[0];
        layout::ring(&mut g, f, 2).unwrap();
        layout::grow_tree(&mut g).unwrap();
        let a = g.drlg.level(l).seed.clone().step();
        let e = ((a & 1) * 2 + 1) as u8;
        seen[(a & 1) as usize] = true;
        layout::act2_sewers(&mut g).unwrap();
        let with_def = |def| {
            g.list()
                .into_iter()
                .filter(|&c| g.cell(c).def == def)
                .collect::<Vec<_>>()
        };
        let c333 = with_def(333);
        assert_eq!(c333.len(), 1);
        let x = g.cell(c333[0]);
        assert_eq!((x.file, x.lock), (0, true));
        assert_eq!(x.links.len(), 1);
        assert_eq!(x.links[0].dir, 2, "C is east of the 333 cell");
        let gs = with_def(336);
        assert_eq!(gs.len(), 1);
        let gc = gs[0];
        assert_eq!((g.cell(gc).file, g.cell(gc).lock), (0, true));
        // H: G's neighbour allocated after G (higher slot).
        let h = g
            .cell(gc)
            .links
            .iter()
            .filter(|l| matches!(l.target, LinkTarget::Cell(n) if n > gc))
            .map(|l| l.dir)
            .collect::<Vec<_>>();
        assert_eq!(h, [e], "k {k}");
    }
    assert_eq!(seen, [true, true]);
}

// ---- layout.rs: neighbouring preset levels, themes ---------------

/// The theme base by level type; other types none.
#[test]
fn theme_base_table() {
    let want = [
        (3, 52),
        (4, 108),
        (7, 167),
        (8, 205),
        (10, 257),
        (13, 301),
        (17, 413),
        (22, 753),
        (24, 664),
        (25, 704),
    ];
    for t in 0..40 {
        let base = want.iter().find(|w| w.0 == t).map(|w| w.1);
        assert_eq!(layout::theme_base(t), base, "type {t}");
    }
}

/// River of Flame: the warp cell (852) and bridge 1 (855) are
/// "fixed" with file −1, so their maps keep the default file.
#[test]
fn river_of_flame_fixed_cells_keep_the_default_file() {
    let maps = run(3, 107, 28, 12, 10).unwrap();
    for def in [852, 855] {
        let m: Vec<_> = maps.iter().filter(|m| m.0 == def).collect();
        assert_eq!(m, [&(def, None)], "def {def}");
    }
}

/// Barracks: after the court cell, one level-seed step: odd →
/// barracks_next[q] then barracks_forge[q+1]; even → barracks_forge[q]
/// then barracks_next[q+1].
#[test]
fn barracks_stamp_order_by_parity() {
    let specials = Specials::shipped();
    let md = MazeData {
        specials: specials.clone(),
        prest_files: (0..1200).map(|d| (d, 1)).collect(),
        ..MazeData::default()
    };
    let data = DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    };
    let row = MazeRow {
        level: 28,
        rooms: [10; 3],
        size_x: 10,
        size_y: 14,
        merge: 0,
    };
    let fresh = |q: u32, k: u32| {
        let (mut d, l, _) = env_id(7, 28);
        d.level_mut(l).seed = Seed::init_low(START.wrapping_add(k.wrapping_mul(7919)));
        let l27 = d.get_or_alloc_level(&data, &mut NoLevelTypes, 27).unwrap();
        d.level_mut(l27).rect = TileRect::new(2000, 2000, 40, 40);
        (
            d,
            l,
            Log {
                direction: q,
                ..Log::default()
            },
        )
    };
    let mut seen = [false; 2];
    for q in 0..3u32 {
        for k in 0..6u32 {
            // The parity, from an identical run stopped after the court.
            let parity = {
                let (mut d, l, _) = fresh(q, k);
                let mut g = Gen::new(&mut d, l, row, &md).unwrap();
                let f = cells(&mut g, &[(500, 500)])[0];
                layout::builder(&mut g, f).unwrap();
                let (finder, dir) =
                    [(Extreme::MaxX, 2), (Extreme::MaxY, 3), (Extreme::MinX, 0)][q as usize];
                let p = g.extreme(finder).unwrap().unwrap();
                g.fixed(p, dir, 167, q as i32, true).unwrap().unwrap();
                g.level_step() & 1
            };
            seen[parity as usize] = true;
            let (mut d, l, mut log) = fresh(q, k);
            let mut g = Gen::new(&mut d, l, row, &md).unwrap();
            let f = cells(&mut g, &[(500, 500)])[0];
            layout::builder(&mut g, f).unwrap();
            layout::barracks(&mut g, &data, &mut log).unwrap();
            let q = q as usize;
            let special = |kind, r| specials.row(kind, r).unwrap().special;
            let want = if parity == 1 {
                [
                    special("barracks_next", q),
                    special("barracks_forge", q + 1),
                ]
            } else {
                [
                    special("barracks_forge", q),
                    special("barracks_next", q + 1),
                ]
            };
            let all: Vec<u32> = (0..4)
                .flat_map(|r| [special("barracks_next", r), special("barracks_forge", r)])
                .collect();
            let mut got: Vec<u32> = g
                .list()
                .into_iter()
                .map(|c| g.cell(c).def)
                .filter(|d| all.contains(d))
                .collect();
            got.sort();
            let mut want = want.to_vec();
            want.sort();
            assert_eq!(got, want, "q {q} k {k}");
        }
    }
    assert_eq!(seen, [true, true]);
}

/// A theme cell gets def target + 15 with file −1, so
/// its map keeps the default file (theme defs lie above the rotation
/// range).
#[test]
fn theme_cells_keep_the_default_file() {
    let maps = run(0, 9, 3, 12, 10).unwrap();
    let themed: Vec<_> = maps.iter().filter(|m| (68..=81).contains(&m.0)).collect();
    assert!(themed.len() >= 2, "need = max(2, count / 5 + 1)");
    assert!(themed.iter().all(|m| m.1.is_none()));
}

/// The rotation base B' by level type; other types none.
#[test]
fn rotation_base_table() {
    let want = [
        (3, 52),
        (4, 108),
        (7, 167),
        (8, 205),
        (10, 257),
        (13, 301),
        (17, 413),
        (18, 481),
        (22, 753),
        (24, 664),
        (25, 704),
        (28, 836),
        (33, 1002),
        (34, 1058),
        (35, 1052),
    ];
    for t in 0..40 {
        let base = want.iter().find(|w| w.0 == t).map(|w| w.1);
        assert_eq!(layout::rotation_base(t), base, "type {t}");
    }
}

/// "small" when the cell's w ≤ 12 **and** h ≤ 12.
#[test]
fn build_small_flag_needs_both_sides() {
    let md = MazeData {
        prest_files: (0..1200).map(|d| (d, 1)).collect(),
        ..MazeData::default()
    };
    let data = DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    };
    let (mut d, l, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(10, 0), &md).unwrap();
    let c = cells(&mut g, &[(0, 0), (100, 0), (200, 0)]);
    for (c, (w, h)) in c.into_iter().zip([(12, 12), (10, 14), (14, 10)]) {
        g.drlg.room_mut(c).rect.w = w;
        g.drlg.room_mut(c).rect.h = h;
        g.cell_mut(c).def = 999;
    }
    let mut log = Log::default();
    layout::build(&mut g, &data, &mut log, &mut Vec::new()).unwrap();
    assert_eq!(log.small, [true, false, false]);
}

/// `Specials::is_empty`: the shipped tables are not empty; no tables is.
#[test]
fn specials_is_empty() {
    assert!(!Specials::shipped().is_empty());
    assert!(Specials::default().is_empty());
}
