//! Tests for maze generation on a fake preset seam: fixed-seed vectors for whole
//! levels, the rules of the cell primitives and layout builders, and the checks of
//! the special-room TSV against the shape tables.

use std::collections::BTreeMap;

use super::cells::{adjacent, collide, direction, shape_def, shape_override, Extreme, Gen};
use super::layout::{self, grow_target, rotation_base, theme_base, theme_budget};
use super::*;
use crate::drlg::{DrlgRoomId, LevelDef, NoLevelTypes};
use crate::rng::Seed;

/// Start seed of the test character.
const START: u32 = 4014346869;

/// One allocated map as the fake preset seam saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct MapLog {
    def: u32,
    rect: TileRect,
    /// Value of the default `roll(Files)` draw.
    default: u32,
    /// Level seed low word right after the default draw.
    seed_after: u32,
    /// File written by maze code, if any.
    file: Option<i32>,
    small: Option<bool>,
    links: Vec<MazeLink>,
}

/// A fake `drlg::preset`: `Files` per def (default 1), a direction for
/// every preset level, a log of maps. At the first map it snapshots the
/// level's cells (rect, init seed, seed).
#[derive(Default)]
struct FakePresets {
    files: BTreeMap<u32, u32>,
    direction: u32,
    maps: Vec<MapLog>,
    cells: Vec<(TileRect, u32, Seed)>,
}

impl MazePresets for FakePresets {
    fn level(&mut self, drlg: &mut Drlg, data: &DrlgData, id: u32) -> Result<LevelIdx, DrlgError> {
        drlg.get_or_alloc_level(data, &mut NoLevelTypes, id)
    }

    fn preset_direction(&self, _drlg: &Drlg, _level: LevelIdx) -> Result<u32, DrlgError> {
        Ok(self.direction)
    }

    fn alloc_map(
        &mut self,
        drlg: &mut Drlg,
        _data: &DrlgData,
        level: LevelIdx,
        def: u32,
        rect: TileRect,
    ) -> Result<MapId, DrlgError> {
        if self.maps.is_empty() {
            self.cells = drlg
                .level_rooms(level)
                .into_iter()
                .map(|r| (drlg.room(r).rect, drlg.room(r).init_seed, drlg.room(r).seed))
                .collect();
        }
        let n = self.files.get(&def).copied().unwrap_or(1);
        let default = drlg.level_mut(level).seed.roll(n as i32);
        self.maps.push(MapLog {
            def,
            rect,
            default,
            seed_after: drlg.level(level).seed.lo,
            file: None,
            small: None,
            links: Vec::new(),
        });
        Ok(MapId(self.maps.len() as u32 - 1))
    }

    fn set_map_file(&mut self, map: MapId, file: i32) {
        self.maps[map.0 as usize].file = Some(file);
    }

    fn build_map(
        &mut self,
        drlg: &mut Drlg,
        _data: &DrlgData,
        level: LevelIdx,
        map: MapId,
        small: bool,
        links: &[MazeLink],
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        let m = &mut self.maps[map.0 as usize];
        m.small = Some(small);
        m.links = links.to_vec();
        Ok(Some(fake_room(drlg, level, m.rect)))
    }
}

/// The room a fake build returns: a real DRLG room of `level`, not in
/// the level list, allocated without spending level-seed draws (the fakes
/// model no rooms).
pub(super) fn fake_room(drlg: &mut Drlg, level: LevelIdx, rect: TileRect) -> DrlgRoomId {
    let seed = drlg.level(level).seed;
    let r = drlg.alloc_room(level, crate::drlg::RoomKind::Preset, rect);
    drlg.level_mut(level).seed = seed;
    r
}

fn leveldata() -> DrlgData {
    DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    }
}

/// A DRLG (act index `act`) with `dwStartSeed` = [`START`] and one
/// maze level `id` of `level_type` at `rect`, seeded `{START + id, 666}`.
fn world(act: u8, id: u32, level_type: u32, rect: TileRect) -> (Drlg, LevelIdx, DrlgData) {
    let data = leveldata();
    let mut d = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    d.act = act;
    d.start_seed = START;
    let l = d.get_or_alloc_level(&data, &mut NoLevelTypes, id).unwrap();
    let lv = d.level_mut(l);
    lv.drlg_type = 1;
    lv.level_type = level_type;
    lv.rect = rect;
    lv.seed = Seed::init_low(START.wrapping_add(id));
    (d, l, data)
}

fn row(level: u32, rooms: u32, size: i32, merge: i32) -> MazeRow {
    MazeRow {
        level,
        rooms: [rooms; 3],
        size_x: size,
        size_y: size,
        merge,
    }
}

/// Maze data with `Files` = 1 for every def.
fn maze(rows: Vec<MazeRow>) -> Maze {
    Maze::new(MazeData {
        rows,
        prest_files: (0..1200).map(|d| (d, 1)).collect(),
        specials: Specials::shipped(),
    })
}

/// Steps of a fresh seed `{x, 666}`.
fn steps(x: u32, n: usize) -> Vec<u32> {
    let mut s = Seed::init_low(x);
    (0..n).map(|_| s.step()).collect()
}

/// A Gen with the first cell F added at (x, y) (no draws counted by the
/// caller beyond F's allocation).
fn gen_with_f<'a>(
    d: &'a mut Drlg,
    l: LevelIdx,
    r: MazeRow,
    md: &'a MazeData,
    x: i32,
    y: i32,
) -> (Gen<'a>, DrlgRoomId) {
    let mut g = Gen::new(d, l, r, md).unwrap();
    let f = g.alloc();
    g.drlg.room_mut(f).rect.x = x;
    g.drlg.room_mut(f).rect.y = y;
    g.add(f);
    (g, f)
}

fn mdata() -> MazeData {
    MazeData {
        specials: Specials::shipped(),
        ..MazeData::default()
    }
}

// ---- Test vectors --------------------------------------------------------------

// Den of Evil from a fixed seed: cell positions, seeds, build order, files, links.
#[test]
fn den_of_evil_vector() {
    let (mut d, l, data) = world(0, 8, 3, TileRect::new(1500, 1000, 200, 200));
    let mut m = maze(vec![row(8, 1, 24, 0)]);
    m.init_level(&d, l).unwrap();
    let mut p = FakePresets::default();
    m.generate(&mut d, &data, &mut p, l).unwrap();

    let ls = steps(START + 8, 5);
    assert_eq!(&ls[..4], &[2583727307, 678241120, 3697141424, 3271692635]);
    assert_eq!(ls[1] & 3, 0);
    // F's init seed, P's and D's allocations (before normalization the
    // cells sit at (1588,1088), (1588,1112), (1564,1088)).
    assert_eq!(Seed::init_low(ls[0]).step(), 3298855633);
    let cells: Vec<(TileRect, u32)> = p.cells.iter().map(|c| (c.0, c.1)).collect();
    assert_eq!(
        cells,
        vec![
            (TileRect::new(1500, 1000, 24, 24), steps(ls[3], 1)[0]),
            (TileRect::new(1524, 1024, 24, 24), steps(ls[2], 1)[0]),
            (TileRect::new(1524, 1000, 24, 24), 3298855633),
        ]
    );
    // Build order D, P, F; defs 96, 86, 57. D and P are outside the
    // rotation range 52 < def < 68 and keep the default file.
    let defs: Vec<u32> = p.maps.iter().map(|m| m.def).collect();
    assert_eq!(defs, vec![96, 86, 57]);
    // D's map alloc roll(1) is the fifth level-seed step.
    assert_eq!(p.maps[0].default, 0);
    assert_eq!(p.maps[0].seed_after, ls[4]);
    assert_eq!(p.maps[0].file, None);
    assert_eq!(p.maps[1].file, None);
    // F (Cave SW, 57) rotates: roll(1) default, then the rotation roll(1),
    // v = (0 + 1) mod 1 = 0.
    assert_eq!(p.maps[2].file, Some(0));
    assert_eq!(
        m.level_data(&d, l).unwrap().rotation,
        vec![Rotation {
            def: 57,
            n: 1,
            v: 0
        }]
    );
    // Every cell was freed by the build.
    assert_eq!(d.room_count(l), 0);
    assert!(p.maps.iter().all(|m| m.small == Some(false)));
    // F's links: P (S, dir 3) and D (W, dir 0), both init.
    let f_links: Vec<u8> = p.maps[2].links.iter().map(|l| l.dir).collect();
    assert_eq!(f_links, vec![3, 0]);
}

// Generating Spider Cavern from a fixed seed: merge draw, link order and seeds.
#[test]
fn spider_cavern_vector() {
    let (mut d, l, data) = world(2, 85, 23, TileRect::new(0, 0, 64, 64));
    let mut m = maze(vec![row(85, 4, 16, 500)]);
    m.init_level(&d, l).unwrap();
    let mut p = FakePresets::default();
    p.files.insert(662, 3);
    m.generate(&mut d, &data, &mut p, l).unwrap();

    let ls = steps(START + 85, 5);
    assert_eq!(&ls[..4], &[3082426380, 80731269, 2373915171, 3826282939]);
    // F's room seed: init step, then the one merge draw.
    let f_steps = steps(ls[0], 2);
    assert_eq!(f_steps[1], 500245367);
    assert!(f_steps[1] % 1000 < 500);
    let f = p.cells.last().unwrap();
    assert_eq!(f.1, f_steps[0]);
    assert_eq!(f.2.lo, f_steps[1]);
    // The other cells drew only their init steps.
    for (i, c) in p.cells[..3].iter().enumerate() {
        assert_eq!(c.2.lo, steps(ls[3 - i], 1)[0]);
    }
    let defs: Vec<u32> = p.maps.iter().map(|m| m.def).collect();
    assert_eq!(defs, vec![662, 660, 659, 663]);
    assert!(p.maps.iter().all(|m| m.file.is_none()));
    assert_eq!(ls[4], 1615270581);
    assert_eq!(p.maps[0].seed_after, 1615270581);
    assert_eq!(p.maps[0].default, 0);
    // F's links: A (N) then C (W, from the merge, R→N gives F dir 0).
    let f_links: Vec<u8> = p.maps[3].links.iter().map(|l| l.dir).collect();
    assert_eq!(f_links, vec![1, 0]);
}

// The grow-tree target room count per level and difficulty.
#[test]
fn grow_tree_target_vector() {
    assert_eq!(grow_target(6, 66, 66, 67), 18);
    assert_eq!(grow_target(6, 67, 66, 67), 12);
    assert_eq!(grow_target(6, 68, 66, 67), 6);
}

// A ring(2) on four cells: shapes and the draws it makes.
#[test]
fn ring2_vector() {
    for merge in [0, 1000] {
        let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
        let md = mdata();
        let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, merge), &md, 50, 50);
        let f_init = g.drlg.room(f).seed;
        layout::ring(&mut g, f, 2).unwrap();
        let list = g.list();
        assert_eq!(list.len(), 4);
        let (c, b, a) = (list[0], list[1], list[2]);
        assert_eq!(g.rect(a), TileRect::new(50, 40, 10, 10));
        assert_eq!(g.rect(b), TileRect::new(40, 40, 10, 10));
        assert_eq!(g.rect(c), TileRect::new(40, 50, 10, 10));
        // F NW (61), A SW (57), B SE (58), C NE (62).
        let defs: Vec<u32> = list.iter().map(|&x| g.cell(x).def).collect();
        assert_eq!(defs, vec![62, 58, 57, 61]);
        // Exactly one merge draw, on F.
        let mut s = f_init;
        s.step();
        assert_eq!(g.drlg.room(f).seed, s);
        for x in [a, b] {
            assert_eq!(g.drlg.room(x).seed.lo, g.drlg.room(x).init_seed);
        }
    }
}

// The theme pass budget (cells needed and tries) for a room count.
#[test]
fn theme_need_vector() {
    assert_eq!(theme_budget(9), (2, 18));
    assert_eq!(theme_budget(16), (4, 32));
    assert_eq!(theme_budget(1), (2, 2));
}

// ---- Rules ----------------------------------------------------------------------

// Cell geometry: adjacent origins, gaps, collisions and directions.
#[test]
fn geometry_rules() {
    let p = TileRect::new(100, 200, 10, 20);
    let want = [
        (90, 200),
        (100, 180),
        (110, 200),
        (100, 220),
        (90, 180),
        (110, 180),
        (110, 220),
        (90, 220),
    ];
    for (d, w) in want.iter().enumerate() {
        assert_eq!(adjacent(&p, d as u8), *w);
    }
    let a = TileRect::new(0, 0, 10, 10);
    // Touching edge: gaps (0, −10): collide at margin 1, not at 0.
    let b = TileRect::new(10, 0, 10, 10);
    assert!(collide(&a, &b, 1) && !collide(&a, &b, 0));
    assert!(collide(&a, &TileRect::new(5, 5, 10, 10), 0));
    assert_eq!(direction(&a, &b), 2);
    assert_eq!(direction(&b, &a), 0);
    assert_eq!(direction(&a, &TileRect::new(0, -10, 10, 10)), 1);
    assert_eq!(direction(&a, &TileRect::new(0, 10, 10, 10)), 3);
    assert_eq!(direction(&a, &TileRect::new(10, 10, 10, 10)), 2);
    assert_eq!(direction(&a, &TileRect::new(30, 30, 10, 10)), -1);
}

#[test]
fn shape_table() {
    assert_eq!(shape_def(3, 5, false), Ok(57));
    assert_eq!(shape_def(34, 15, false), Ok(1073));
    assert_eq!(shape_def(33, 6, true), Ok(6));
    assert_eq!(shape_def(33, 6, false), Ok(1008));
    assert_eq!(shape_def(14, 10, false), Ok(354));
    assert_eq!(shape_def(14, 3, false), Ok(0));
    assert_eq!(shape_def(35, 12, false), Ok(1058));
    assert_eq!(shape_def(35, 5, false), Ok(0));
    assert_eq!(shape_def(32, 9, false), Ok(1043));
    assert_eq!(shape_def(9, 1, false), Err(MazeError::BadLevelType(9)));
    assert_eq!(shape_override(52, 361), (361, Some(2)));
    assert_eq!(shape_override(54, 359), (359, Some(3)));
    assert_eq!(shape_override(84, 662), (664, None));
    assert_eq!(shape_override(85, 661), (663, None));
    assert_eq!(shape_override(85, 662), (662, None));
}

// A mask with no shape def leaves the cell unchanged.
#[test]
fn pick_without_shape_leaves_cell() {
    let (mut d, l, _) = world(2, 85, 23, TileRect::new(0, 0, 100, 100));
    let md = mdata();
    let (mut g, f) = gen_with_f(&mut d, l, row(85, 4, 10, 0), &md, 50, 50);
    // One W link: mask 1 has no Spider shape.
    let n = g.place(f, 0).unwrap();
    g.link(f, n, 0);
    g.pick(f).unwrap();
    assert_eq!(
        (g.cell(f).def, g.cell(f).file, g.cell(f).lock),
        (0, 0, false)
    );
    g.cell_mut(f).lock = true;
    let s = g.place(f, 3).unwrap();
    g.link(f, s, 3);
    g.pick(f).unwrap(); // W|S = 5: 659, lock cleared
    assert_eq!(
        (g.cell(f).def, g.cell(f).file, g.cell(f).lock),
        (659, -1, false)
    );
}

// A rejected placement still spends its seed draws and is freed.
#[test]
fn rejected_place_still_draws() {
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
    let md = mdata();
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 50, 50);
    // A blocker north of F.
    let x = g.alloc();
    g.drlg.room_mut(x).rect = TileRect::new(50, 40, 10, 10);
    g.add(x);
    let before = g.drlg.level(l).seed;
    assert!(g.place(f, 1).is_none());
    let mut s = before;
    s.step();
    assert_eq!(g.drlg.level(l).seed, s);
    assert_eq!(g.count(), 2);
    // West is free.
    assert!(g.place(f, 0).is_some());
}

#[test]
fn merge_draws_on_edge_neighbours() {
    for (merge, linked) in [(0, false), (1000, true)] {
        let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
        let md = mdata();
        let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, merge), &md, 50, 50);
        // E of F, not linked to F: one draw on F, link iff merge passes.
        let e = g.place(f, 2).unwrap();
        let s0 = g.drlg.room(f).seed;
        g.merge(e).unwrap();
        let mut s = s0;
        s.step();
        assert_eq!(g.drlg.room(f).seed, s);
        assert_eq!(g.cell(e).links.len(), usize::from(linked));
        if linked {
            assert_eq!(g.cell(f).def, 54); // Cave E
            assert_eq!(g.cell(f).links[0].dir, 2);
        }
        // A locked new cell skips the merge entirely.
        let w = g.place(f, 0).unwrap();
        g.cell_mut(w).lock = true;
        let s1 = g.drlg.room(f).seed;
        g.merge(w).unwrap();
        assert_eq!(g.drlg.room(f).seed, s1);
    }
}

// Special-room stamps: found cell, no cell, and the fallback attach.
#[test]
fn stamp_rules() {
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
    let md = mdata();
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 50, 50);
    // Found: F has def 60 (Cave N) → 86, locked, no allocation.
    g.cell_mut(f).def = 60;
    let seed = g.drlg.level(l).seed;
    let mut r = 0;
    g.stamp_next("cave_prev", &mut r).unwrap();
    assert_eq!(
        (g.cell(f).def, g.cell(f).file, g.cell(f).lock),
        (86, -1, true)
    );
    assert_eq!(g.drlg.level(l).seed, seed);
    assert_eq!(r, 1);
    // Not found and every cell locked: nothing placed, counter advances.
    r = 3;
    g.stamp_next("cave_doe", &mut r).unwrap();
    assert_eq!((r, g.count()), (0, 1));
    assert_eq!(g.drlg.level(l).seed, seed);
    // Not found, unlocked parent: fallback dir 2 (row 3, W) from F.
    g.cell_mut(f).lock = false;
    r = 3;
    g.stamp_next("cave_down", &mut r).unwrap();
    let n = g.list()[0];
    assert_eq!(g.rect(n), TileRect::new(60, 50, 10, 10));
    assert_eq!((g.cell(n).def, g.cell(n).lock), (91, true));
    assert_eq!(g.cell(f).def, 54); // picked: one E link
}

#[test]
fn probe_and_extremes() {
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
    let md = mdata();
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 50, 50);
    let n = g.grow(f, 1).unwrap().unwrap(); // N of F
    let links = g.cell(f).links.clone();
    let seed = g.drlg.level(l).seed;
    assert!(g.probe(f, 2).unwrap());
    assert_eq!(g.count(), 2);
    assert_eq!(g.cell(f).links, links);
    let mut s = seed;
    s.step();
    assert_eq!(g.drlg.level(l).seed, s);
    // A link in the probed direction: false, no draw.
    assert!(!g.probe(f, 1).unwrap());
    assert_eq!(g.drlg.level(l).seed, s);
    // Smallest y is N; largest y is F.
    assert_eq!(g.extreme(Extreme::MinY).unwrap(), Some(n));
    assert_eq!(g.extreme(Extreme::MaxY).unwrap(), Some(f));
    assert_eq!(g.count(), 2);
}

// The grow tree draws a direction even for a locked cell, and grows nothing there.
#[test]
fn grow_tree_draws_on_locked_cells() {
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 1000, 1000));
    let md = mdata();
    let r = row(9, 3, 10, 0);
    let (mut g, f) = gen_with_f(&mut d, l, r, &md, 500, 500);
    let x = g.alloc();
    g.drlg.room_mut(x).rect = TileRect::new(100, 100, 10, 10);
    g.add(x);
    g.cell_mut(x).lock = true;
    let (mut ls, mut xs, mut fs) = (
        g.drlg.level(l).seed,
        g.drlg.room(x).seed,
        g.drlg.room(f).seed,
    );
    layout::grow_tree(&mut g).unwrap();
    // Replay: X (head) picked → its draw only; F picked → draw + grow.
    let mut x_picks = 0;
    loop {
        if ls.roll(2) == 0 {
            xs.step();
            x_picks += 1;
        } else {
            fs.step();
            break;
        }
    }
    assert!(x_picks > 0, "seed choice exercises a locked pick");
    assert_eq!(g.drlg.room(x).seed, xs);
    assert_eq!(g.drlg.room(f).seed, fs);
    assert_eq!(g.count(), 3);
    // Random cell with count 0 draws nothing (an empty roll spends no step).
    let (mut d2, l2, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
    let md2 = mdata();
    let mut g2 = Gen::new(&mut d2, l2, row(9, 1, 10, 0), &md2).unwrap();
    let s = g2.drlg.level(l2).seed;
    assert_eq!(g2.random_cell(), None);
    assert_eq!(g2.drlg.level(l2).seed, s);
}

// A rejected ring grow is an error; ring(5) leaves the expected shapes.
#[test]
fn ring_rejection_is_fatal_and_ring5_shape() {
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
    let md = mdata();
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 50, 50);
    let x = g.alloc();
    g.drlg.room_mut(x).rect = TileRect::new(50, 40, 10, 10);
    g.add(x);
    assert_eq!(layout::ring(&mut g, f, 2), Err(MazeError::NullCell("ring")));

    let (mut d, l, _) = world(0, 92, 25, TileRect::new(0, 0, 100, 100));
    let (mut g, f) = gen_with_f(&mut d, l, row(92, 1, 10, 0), &md, 50, 50);
    layout::ring(&mut g, f, 5).unwrap();
    assert_eq!(g.count(), 16);
    let bb = g.bounding_box();
    assert_eq!(bb, TileRect::new(10, 10, 50, 50));
    // F is the SE corner: NW shape (704 + 9).
    assert_eq!(g.cell(f).def, 713);
    layout::sewer_corner_swaps(&mut g).unwrap();
    let defs: Vec<u32> = g.list().iter().map(|&c| g.cell(c).def).collect();
    for d in [735, 736, 737, 738] {
        assert_eq!(defs.iter().filter(|&&x| x == d).count(), 1);
    }
    assert_eq!(
        layout::sewer_corner_swaps(&mut g),
        Err(MazeError::MissingSwap(709))
    );
}

#[test]
fn catacomb_start_and_hub() {
    let md = mdata();
    let (mut d, l, _) = world(0, 34, 10, TileRect::new(0, 0, 100, 100));
    let (mut g, f) = gen_with_f(&mut d, l, row(34, 1, 10, 0), &md, 50, 50);
    let s = g.drlg.level(l).seed;
    layout::catacomb_start(&mut g, f).unwrap();
    assert_eq!(g.count(), 5);
    assert_eq!(
        (g.cell(f).def, g.cell(f).file, g.cell(f).lock),
        (290, -1, true)
    );
    let mut e = s;
    for _ in 0..4 {
        e.step();
    }
    assert_eq!(g.drlg.level(l).seed, e);

    let (mut d, l, _) = world(0, 35, 10, TileRect::new(0, 0, 100, 100));
    let (mut g, f) = gen_with_f(&mut d, l, row(35, 1, 10, 0), &md, 50, 50);
    let lo = g.drlg.level(l).seed.clone().step();
    layout::catacomb_start(&mut g, f).unwrap();
    let want = if lo & 1 == 0 { 289 } else { 288 };
    assert_eq!(g.cell(f).def, want);
    assert_eq!(g.count(), 3);

    let (mut d, l, _) = world(1, 56, 17, TileRect::new(0, 0, 100, 100));
    let (mut g, f) = gen_with_f(&mut d, l, row(56, 1, 10, 0), &md, 50, 50);
    let r = g.drlg.level(l).seed.clone().step() & 3;
    layout::hub(&mut g, f, &[447, 444, 446, 445]).unwrap();
    let open = (r + 3) % 4;
    assert_eq!(g.cell(f).def, [447, 444, 446, 445][open as usize]);
    assert!(g.cell(f).links.iter().all(|k| u32::from(k.dir) != open));
    assert_eq!(g.count(), 4);
    // Tombs read r back from the hub def.
    let hub_r = match g.cell(f).def {
        446 => 0,
        445 => 1,
        447 => 2,
        _ => 3,
    };
    assert_eq!(hub_r, (open + 2) % 4);
}

#[test]
fn arcane_spiral() {
    let md = mdata();
    let (mut d, l, _) = world(1, 74, 19, TileRect::new(0, 0, 400, 400));
    let (mut g, f) = gen_with_f(&mut d, l, row(74, 61, 10, 0), &md, 200, 200);
    let r = g.drlg.level(l).seed.clone().step() & 3;
    layout::spiral(&mut g, f).unwrap();
    assert_eq!(g.count(), 61);
    assert_eq!(g.cell(f).file, 4);
    // Branch b's cells are the 15 added after the previous branch.
    let list: Vec<DrlgRoomId> = g.list().into_iter().rev().collect();
    for b in 0..4 {
        for k in 0..15 {
            let c = list[1 + 15 * b + k];
            let want = if k == 8 || k == 12 {
                -1
            } else {
                ((r + b as u32) % 4) as i32
            };
            assert_eq!(g.cell(c).file, want, "branch {b} cell {k}");
        }
    }
}

#[test]
fn lava_cross_and_fill_blanks() {
    let md = mdata();
    let (mut d, l, _) = world(4, 125, 35, TileRect::new(0, 0, 400, 400));
    let (mut g, f) = gen_with_f(&mut d, l, row(125, 1, 10, 0), &md, 200, 200);
    let mut fs = g.drlg.room(f).seed;
    let s = (fs.step() & 3) as usize;
    let ls0 = g.drlg.level(l).seed;
    layout::lava_cross(&mut g, f).unwrap();
    // 2 fixed placements, then 3 cells × 8 blank tries: one level-seed
    // step per allocation, kept or not.
    let mut e = ls0;
    for _ in 0..(2 + 3 * 8) {
        e.step();
    }
    assert_eq!(g.drlg.level(l).seed, e);
    let defs: Vec<u32> = g.list().iter().map(|&c| g.cell(c).def).collect();
    assert_eq!(
        defs.iter().filter(|&&d| d == 836).count(),
        g.count() as usize - 3
    );
    let picked = [(1054, 1053), (1054, 1053), (1055, 1056), (1055, 1056)][s];
    assert!(defs.contains(&picked.0) && defs.contains(&picked.1));
    // The fill visits the copied list (B, A, F) only: B keeps 7 (F
    // blocks N), A keeps 5 (F and B's NW/NE blanks block S, SW, SE), F
    // keeps 0 (every side is taken by then).
    assert_eq!(g.count(), 3 + 7 + 5);
}

// Normalizing moves the cells to the level origin, or fails when they do not fit.
#[test]
fn normalize_rules() {
    let md = mdata();
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(1000, 2000, 20, 10));
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 5, 7);
    g.grow(f, 0).unwrap();
    layout::normalize(&mut g).unwrap();
    assert_eq!(g.bounding_box(), TileRect::new(1000, 2000, 20, 10));
    g.grow(f, 3).unwrap();
    assert_eq!(layout::normalize(&mut g), Err(MazeError::DoesNotFit));
}

// The theme pass: which cells convert, in what order, and which levels skip it.
#[test]
fn theme_pass() {
    let md = mdata();
    // Only 4-way cells: nothing changes, 31 draws.
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 100, 100));
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 50, 50);
    g.cell_mut(f).def = 67;
    let s0 = g.drlg.level(l).seed;
    layout::theme(&mut g).unwrap();
    let mut e = s0;
    for _ in 0..31 {
        e.step();
    }
    assert_eq!(g.drlg.level(l).seed, e);
    assert_eq!((g.cell(f).def, g.cell(f).lock), (67, false));
    // One cell of each shape 53..66: need 3.
    let (mut d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 1000, 1000));
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 10, 0), &md, 0, 0);
    g.cell_mut(f).def = 53;
    for def in 54..=66 {
        let c = g.alloc();
        g.drlg.room_mut(c).rect.x = 20 * (def as i32 - 53);
        g.add(c);
        g.cell_mut(c).def = def;
    }
    layout::theme(&mut g).unwrap();
    let themed: Vec<u32> = g
        .list()
        .iter()
        .map(|&c| g.cell(c))
        .filter(|c| c.lock)
        .map(|c| c.def)
        .collect();
    assert_eq!(themed.len(), 3);
    assert!(themed.iter().all(|d| (68..=81).contains(d)));
    // Den of Evil and non-theme types skip with no draw.
    assert_eq!(theme_base(19), None);
    let (mut d, l, _) = world(0, 8, 3, TileRect::new(0, 0, 100, 100));
    let (mut g, _) = gen_with_f(&mut d, l, row(8, 1, 10, 0), &md, 50, 50);
    let s = g.drlg.level(l).seed;
    layout::theme(&mut g).unwrap();
    assert_eq!(g.drlg.level(l).seed, s);
}

// The build: file rotation per def, files left at the default, and cell frees.
#[test]
fn build_rotation() {
    let (mut d, l, data) = world(0, 9, 3, TileRect::new(0, 0, 1000, 1000));
    let mut md = mdata();
    md.prest_files.insert(56, 3);
    let (mut g, f) = gen_with_f(&mut d, l, row(9, 1, 12, 0), &md, 0, 0);
    g.cell_mut(f).def = 56;
    g.cell_mut(f).file = -1;
    for (i, (def, file)) in [(56, -1), (86, -1), (56, -1), (56, 2)]
        .into_iter()
        .enumerate()
    {
        let c = g.alloc();
        g.drlg.room_mut(c).rect.x = 20 * (i as i32 + 1);
        g.add(c);
        g.cell_mut(c).def = def;
        g.cell_mut(c).file = file;
    }
    let mut p = FakePresets::default();
    p.files.insert(56, 3);
    let mut seed = g.drlg.level(l).seed;
    let mut rot = Vec::new();
    layout::build(&mut g, &data, &mut p, &mut rot).unwrap();
    // List order: (56,2), (56,−1), (86,−1), (56,−1), F.
    let d0 = seed.roll(3); // first map default
    assert_eq!(p.maps[0].default, d0);
    assert_eq!(p.maps[0].file, Some(2));
    let d1 = seed.roll(3);
    let v = seed.roll(3) as i32; // rotation record
    assert_eq!(p.maps[1].default, d1);
    assert_eq!(p.maps[1].file, Some((v + 1) % 3));
    seed.roll(1);
    assert_eq!(p.maps[2].file, None); // 86: outside 52 < def < 68
    seed.roll(3);
    assert_eq!(p.maps[3].file, Some((v + 2) % 3));
    seed.roll(3);
    assert_eq!(p.maps[4].file, Some(v % 3));
    assert_eq!(g.drlg.level(l).seed, seed);
    assert!(p.maps.iter().all(|m| m.small == Some(true)));
    assert_eq!(g.count(), 0);
    assert_eq!(
        rot,
        vec![Rotation {
            def: 56,
            n: 3,
            v: v % 3
        }]
    );
    assert_eq!(rotation_base(35), Some(1052));
    assert_eq!(rotation_base(19), None);
}

#[test]
fn tomb_stamps_and_errors() {
    let md = mdata();
    let (mut d, l, _) = world(1, 66, 17, TileRect::new(0, 0, 400, 400));
    d.staff_tomb = 66;
    d.boss_tomb = 67;
    let (mut g, f) = gen_with_f(&mut d, l, row(66, 1, 10, 0), &md, 200, 200);
    assert_eq!(layout::tombs(&mut g), Err(MazeError::TombHub(None)));
    g.set(f, 446, -1); // r = 0: Talrasha row 0 (Tomb N), fallback dir 3
    let s = g.drlg.level(l).seed;
    layout::tombs(&mut g).unwrap();
    // No draw of its own; F, the only cell, is locked, so every stamp's
    // fallback places nothing and draws nothing.
    assert_eq!(g.drlg.level(l).seed, s);
    assert_eq!(g.count(), 1);
}

// Every maze level generates, fits its rect and gets its expected specials.
#[test]
fn every_level_generates() {
    // (id, type, rooms, size, act); merge 500; big level rects.
    let levels: &[(u32, u32, u32, i32, u8)] = &[
        (8, 3, 1, 24, 0),
        (9, 3, 10, 24, 0),
        (10, 3, 12, 24, 0),
        (11, 3, 10, 24, 0),
        (18, 4, 6, 24, 0),
        (19, 4, 6, 24, 0),
        (21, 4, 6, 24, 0),
        (29, 8, 12, 24, 0),
        (30, 8, 12, 24, 0),
        (31, 8, 12, 24, 0),
        (34, 10, 8, 24, 0),
        (35, 10, 12, 24, 0),
        (47, 13, 12, 24, 1),
        (48, 13, 12, 24, 1),
        (49, 13, 12, 24, 1),
        (52, 15, 4, 24, 1),
        (55, 17, 8, 24, 1),
        (57, 17, 8, 24, 1),
        (59, 17, 8, 24, 1),
        (60, 17, 8, 24, 1),
        (61, 17, 1, 24, 1),
        (62, 18, 8, 24, 1),
        (64, 18, 8, 24, 1),
        (65, 13, 8, 24, 1),
        (66, 17, 6, 24, 1),
        (74, 19, 61, 16, 1),
        (84, 23, 4, 16, 2),
        (86, 24, 8, 24, 2),
        (92, 25, 20, 24, 2),
        (100, 22, 8, 24, 2),
        (101, 22, 8, 24, 2),
        (113, 33, 8, 24, 4),
        (114, 33, 1, 24, 4),
        (115, 33, 8, 24, 4),
        (122, 32, 4, 24, 4),
        (123, 32, 4, 24, 4),
        (125, 35, 1, 24, 4),
        (128, 34, 8, 24, 4),
        (129, 34, 8, 24, 4),
        (88, 28, 8, 24, 3),
    ];
    for &(id, ty, rooms, size, act) in levels {
        let (mut d, l, data) = world(act, id, ty, TileRect::new(0, 0, 400, 400));
        d.staff_tomb = 66;
        d.boss_tomb = 67;
        let mut m = maze(vec![row(id, rooms, size, 500)]);
        m.init_level(&d, l).unwrap();
        let mut p = FakePresets::default();
        m.generate(&mut d, &data, &mut p, l)
            .unwrap_or_else(|e| panic!("level {id}: {e}"));
        assert!(!p.maps.is_empty(), "level {id}");
        assert_eq!(d.room_count(l), 0, "level {id}");
        // Every cell inside the level rect.
        let lr = d.level(l).rect;
        for mlog in &p.maps {
            let r = mlog.rect;
            assert!(r.x >= lr.x && r.y >= lr.y, "level {id}");
            assert!(
                r.x + r.w <= lr.x + lr.w && r.y + r.h <= lr.y + lr.h,
                "level {id}"
            );
        }
        // Expected specials present.
        let defs: Vec<u32> = p.maps.iter().map(|m| m.def).collect();
        let want: &[&[u32]] = match id {
            8 => &[&[83, 84, 85, 86], &[95, 96, 97, 98]],
            61 => &[&[480]],
            114 => &[&[1038, 1039]],
            74 => &[&[525, 526, 527, 528]],
            129 => &[&[1078, 1079, 1080, 1081], &[1082, 1083, 1084, 1085]],
            _ => &[],
        };
        for set in want {
            assert!(defs.iter().any(|d| set.contains(d)), "level {id}: {defs:?}");
        }
    }
}

// Barracks and River of Flame: cross-level links, shifts and special cells.
#[test]
fn barracks_and_river() {
    for q in 0..3u32 {
        let (mut d, l, data) = world(0, 28, 7, TileRect::new(0, 0, 400, 400));
        let lc = d.get_or_alloc_level(&data, &mut NoLevelTypes, 27).unwrap();
        d.level_mut(lc).rect = TileRect::new(5000, 6000, 80, 80);
        let mut m = maze(vec![MazeRow {
            level: 28,
            rooms: [12; 3],
            size_x: 10,
            size_y: 14,
            merge: 500,
        }]);
        m.init_level(&d, l).unwrap();
        let mut p = FakePresets {
            direction: q,
            ..FakePresets::default()
        };
        m.generate(&mut d, &data, &mut p, l).unwrap();
        let court = p.maps.iter().find(|m| m.def == 167).unwrap();
        assert_eq!(court.file, Some(q as i32));
        let c = court.rect;
        let want = match q {
            0 => (5000 - 10, 6000 + 40),
            1 => (5000 + 40 - 6, 6000 - 14),
            _ => (5080, 6041),
        };
        assert_eq!((c.x, c.y), want, "q {q}");
        // The cross-level link has no init flag: not handed to the build.
        assert!(court.links.iter().all(|k| k.init));
        assert_eq!(court.links.len(), 1);
        // Level rect = bounding box of the cells.
        let lr = d.level(l).rect;
        let x0 = p.maps.iter().map(|m| m.rect.x).min().unwrap();
        let y1 = p.maps.iter().map(|m| m.rect.y + m.rect.h).max().unwrap();
        assert_eq!((lr.x, lr.y + lr.h), (x0, y1));
        let defs: Vec<u32> = p.maps.iter().map(|m| m.def).collect();
        assert!(defs.iter().any(|d| (198..=201).contains(d)), "{defs:?}");
        assert!(defs.iter().any(|d| (202..=205).contains(d)), "{defs:?}");
    }
    let (mut d, l, data) = world(0, 28, 7, TileRect::new(0, 0, 400, 400));
    let mut m = maze(vec![row(28, 12, 10, 500)]);
    m.init_level(&d, l).unwrap();
    let mut p = FakePresets {
        direction: 3,
        ..FakePresets::default()
    };
    assert_eq!(
        m.generate(&mut d, &data, &mut p, l),
        Err(MazeError::BarracksDirection(3))
    );

    let (mut d, l, data) = world(3, 107, 28, TileRect::new(0, 0, 400, 400));
    let s = d.get_or_alloc_level(&data, &mut NoLevelTypes, 108).unwrap();
    d.level_mut(s).rect = TileRect::new(3000, 4000, 100, 120);
    let mut m = maze(vec![row(107, 10, 16, 500)]);
    m.init_level(&d, l).unwrap();
    let mut p = FakePresets::default();
    m.generate(&mut d, &data, &mut p, l).unwrap();
    let defs: Vec<u32> = p.maps.iter().map(|m| m.def).collect();
    for want in [852, 855] {
        assert_eq!(defs.iter().filter(|&&x| x == want).count(), 1, "{defs:?}");
    }
    assert_eq!(defs.iter().filter(|&&x| x == 856).count(), 2);
    assert!(defs.iter().any(|d| *d == 853 || *d == 854), "{defs:?}");
    // B3 (the second 856, northmost) moved to (S.x + 2w, S.y + S.h).
    let b3 = p
        .maps
        .iter()
        .filter(|m| m.def == 856)
        .min_by_key(|m| m.rect.y)
        .unwrap();
    assert_eq!((b3.rect.x, b3.rect.y), (3000 + 32, 4120));
    assert!(defs.contains(&836));
}

// The lvlmaze lookup, level init, reset and free.
#[test]
fn lvlmaze_lookup() {
    let md = MazeData {
        rows: vec![row(9, 1, 1, 0), row(8, 2, 1, 0), row(8, 3, 1, 0)],
        ..MazeData::default()
    };
    assert_eq!(md.row_index(8), Ok(1));
    assert_eq!(md.row_index(7), Err(MazeError::NoMazeRow(7)));
    let (d, l, _) = world(0, 7, 3, TileRect::new(0, 0, 10, 10));
    let mut m = Maze::new(md);
    assert_eq!(m.init_level(&d, l), Err(MazeError::NoMazeRow(7)));
    // reset keeps the record, clears the rotation list; free drops both.
    let (d, l, _) = world(0, 9, 3, TileRect::new(0, 0, 10, 10));
    m.init_level(&d, l).unwrap();
    m.levels
        .get_mut(&(0, l))
        .unwrap()
        .rotation
        .push(Rotation { def: 1, n: 1, v: 0 });
    m.reset_level(&d, l);
    assert_eq!(
        m.level_data(&d, l),
        Some(&MazeLevel {
            row: Some(0),
            rotation: vec![]
        })
    );
    m.free_level(&d, l);
    assert_eq!(m.level_data(&d, l), None);
}

// ---- maze-specials.tsv ------------------------------------------------------

/// Pick-shape base def of each standard special table's type, for the
/// find-def cross-check: row r (N, E, S, W) finds base + (8, 2, 4, 1).
const TABLE_BASES: &[(&str, u32)] = &[
    ("cave_", 52),
    ("crypt_", 108),
    ("jail_", 205),
    ("catacombs_", 257),
    ("a2sewer_", 301),
    ("tomb_", 413),
    ("lair_", 481),
    ("arcane_", 509),
    ("dungeon_", 664),
    ("a3sewer_", 704),
    ("meph_", 753),
    ("baal_", 1058),
    ("barracks_", 167),
    ("ice_", 1002),
];

/// Every TSV row against the code's shape tables; returns mismatches.
fn check_specials(tsv: &str) -> Result<Vec<String>, String> {
    let s = Specials::parse(tsv)?;
    let mut bad = Vec::new();
    for kind in s.kinds() {
        let rows = s.table(kind).unwrap();
        let base = TABLE_BASES.iter().find(|(p, _)| kind.starts_with(p));
        match (kind, base) {
            ("lair_tightspot", _) => {
                if rows
                    != [SpecialRow {
                        find: 485,
                        special: 509,
                        file: -1,
                        dir: 1,
                    }]
                {
                    bad.push(format!("{kind}: {rows:?}"));
                }
            }
            ("a4lava_forge", _) => {
                let w = shape_def(28, 1, false).unwrap();
                let e = shape_def(28, 2, false).unwrap();
                if rows.len() != 2
                    || (rows[0].find, rows[0].dir, rows[1].find, rows[1].dir) != (w, 2, e, 0)
                {
                    bad.push(format!("{kind}: {rows:?}"));
                }
            }
            (k, _) if k.starts_with("temple_") => {
                // Rows find the Temple corner shapes NE, NW, SW, SE.
                for (r, row) in rows.iter().enumerate() {
                    let mask = [10, 9, 5, 6][r];
                    let dir = [1, 0, 3, 2][r];
                    if (row.find, row.dir) != (shape_def(32, mask, false).unwrap(), dir) {
                        bad.push(format!("{kind} row {r}: {row:?}"));
                    }
                }
            }
            (_, Some(&(_, b))) => {
                if rows.len() != 4 {
                    bad.push(format!("{kind}: {} rows", rows.len()));
                }
                for (r, row) in rows.iter().enumerate() {
                    let want = (b + [8, 2, 4, 1][r % 4], [3, 0, 1, 2][r % 4], -1);
                    if (row.find, row.dir, row.file) != want {
                        bad.push(format!("{kind} row {r}: {row:?}, expected {want:?}"));
                    }
                }
            }
            _ => bad.push(format!("{kind}: no check")),
        }
    }
    if s.len() != 210 {
        bad.push(format!("{} rows, expected 210", s.len()));
    }
    Ok(bad)
}

#[test]
fn specials_tsv_parses() {
    let s = Specials::parse(SPECIALS_TSV).unwrap();
    assert_eq!(s.len(), 210);
    assert_eq!(check_specials(SPECIALS_TSV).unwrap(), Vec::<String>::new());
    // Every table the code stamps exists with the rows it uses.
    let used: &[(&str, usize)] = &[
        ("cave_prev", 4),
        ("cave_doe", 4),
        ("cave_down", 4),
        ("cave_coldcrow", 4),
        ("cave_next", 4),
        ("crypt_prev", 4),
        ("crypt_bonebreak", 4),
        ("crypt_chest", 4),
        ("crypt_next", 4),
        ("jail_prev", 4),
        ("jail_waypoint", 4),
        ("jail_next", 4),
        ("jail_pitspawn", 4),
        ("jail_cath", 4),
        ("catacombs_next", 4),
        ("catacombs_waypoint", 4),
        ("a2sewer_prev", 4),
        ("a2sewer_next", 4),
        ("a2sewer_waypoint", 4),
        ("a2sewer_radament", 4),
        ("a2sewer_chest", 4),
        ("tomb_next", 4),
        ("tomb_waypoint", 4),
        ("tomb_chest", 4),
        ("tomb_leatherarm", 4),
        ("tomb_cube", 4),
        ("tomb_treasure", 4),
        ("tomb_talrasha", 4),
        ("tomb_kaa", 4),
        ("lair_prev", 4),
        ("lair_next", 4),
        ("lair_treasure", 4),
        ("lair_tightspot", 1),
        ("arcane_summoner", 4),
        ("dungeon_prev", 4),
        ("dungeon_next", 4),
        ("a3sewer_drain", 4),
        ("a3sewer_chest", 4),
        ("meph_prev", 4),
        ("meph_waypoint", 4),
        ("meph_next", 4),
        ("temple_down", 3),
        ("temple_waypoint", 4),
        ("baal_next", 4),
        ("baal_waypoint", 4),
        ("barracks_next", 4),
        ("barracks_forge", 4),
        ("a4lava_forge", 2),
        ("ice_prev", 4),
        ("ice_next", 4),
        ("ice_down", 4),
        ("ice_theme", 4),
        ("ice_waypoint", 4),
    ];
    for &(k, n) in used {
        assert_eq!(s.table(k).map(<[_]>::len), Some(n), "{k}");
    }
}

#[test]
fn specials_check_catches_perturbations() {
    let perturb = |line: usize, col: usize, to: &str| -> String {
        SPECIALS_TSV
            .lines()
            .enumerate()
            .map(|(i, l)| {
                if i == line {
                    let mut c: Vec<&str> = l.split('\t').collect();
                    c[col] = to;
                    c.join("\t")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Line 1 = cave_prev row 0. A wrong find def is reported for that row.
    let bad = check_specials(&perturb(1, 2, "61")).unwrap();
    assert_eq!(bad, vec!["cave_prev row 0: SpecialRow { find: 61, special: 86, file: -1, dir: 3 }, expected (60, 3, -1)".to_string()]);
    // A wrong fallback direction.
    let bad = check_specials(&perturb(2, 5, "2")).unwrap();
    assert_eq!(bad.len(), 1);
    assert!(bad[0].starts_with("cave_prev row 1"));
    // Strict parse errors name the line.
    assert_eq!(
        Specials::parse(&perturb(3, 1, "5")).unwrap_err(),
        "line 4: cave_prev row 5, expected 2"
    );
    assert_eq!(
        Specials::parse(&perturb(3, 5, "4")).unwrap_err(),
        "line 4: fallback_dir 4 outside 0..3"
    );
    assert!(Specials::parse(&perturb(3, 3, "x"))
        .unwrap_err()
        .starts_with("line 4: replace_def"));
    assert!(Specials::parse(&perturb(0, 0, "Kind"))
        .unwrap_err()
        .starts_with("line 1: header"));
    assert!(Specials::parse(&perturb(3, 6, "6EF8C8"))
        .unwrap_err()
        .starts_with("line 4: va"));
}

/// The shipped specials restricted to `kinds`.
fn specials_only(kinds: &[&str]) -> Specials {
    let text: Vec<&str> = SPECIALS_TSV
        .lines()
        .enumerate()
        .filter(|(i, l)| *i == 0 || kinds.contains(&l.split('\t').next().unwrap_or("")))
        .map(|(_, l)| l)
        .collect();
    Specials::parse(&text.join("\n")).unwrap()
}

#[test]
fn builders_stamp_the_unlisted_levels_of_their_type() {
    type Builder = fn(&mut Gen<'_>) -> Result<(), MazeError>;
    let cases: &[(u32, Builder, &[&str])] = &[
        (13, layout::cave, &["cave_prev", "cave_down"]),
        (14, layout::cave, &["cave_prev", "cave_down"]),
        (15, layout::cave, &["cave_prev", "cave_down"]),
        (16, layout::cave, &["cave_prev", "cave_down"]),
        (25, layout::crypt, &["crypt_prev"]),
        (37, layout::catacombs, &["catacombs_next"]),
        (90, layout::dungeon, &["dungeon_prev", "dungeon_next"]),
        (91, layout::dungeon, &["dungeon_prev", "dungeon_next"]),
        (93, layout::act3_sewers, &["a3sewer_drain", "a3sewer_chest"]),
        (132, layout::baal, &["baal_next"]),
    ];
    for &(id, build, kinds) in cases {
        // With only the first i tables present, the builder fails on
        // stamp i (row r + i mod 4); with all of them it succeeds.
        for i in 0..=kinds.len() {
            let md = MazeData {
                specials: specials_only(&kinds[..i]),
                ..MazeData::default()
            };
            let (mut d, l, _) = world(1, id, 3, TileRect::new(0, 0, 400, 400));
            let (mut g, f) = gen_with_f(&mut d, l, row(id, 1, 10, 0), &md, 200, 200);
            g.cell_mut(f).lock = true;
            let mut s = g.drlg.level(l).seed;
            let r = (s.step() & 3) as usize;
            let got = build(&mut g);
            if i < kinds.len() {
                assert_eq!(
                    got,
                    Err(MazeError::NoSpecialRow(kinds[i], (r + i) % 4)),
                    "level {id} stamp {i}"
                );
            } else {
                assert_eq!(got, Ok(()), "level {id}");
            }
        }
    }
}
