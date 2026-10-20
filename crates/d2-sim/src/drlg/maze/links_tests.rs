//! Tests for maze link lists: prepend order, the room free's link removal, the
//! cross-level sorted insert, the spiral's file pass and the build's link
//! hand-over to the built rooms.

use super::cells::{Gen, LinkTarget, MazeLink};
use super::layout;
use super::*;
use crate::drlg::room::LinkAt;
use crate::drlg::{DrlgRoomId, LevelDef, NoLevelTypes, RoomKind};

/// A DRLG with maze level 8 of `level_type` at (0, 0, 1000, 1000) and a
/// second level 9 (cross-level link target).
fn env(level_type: u32) -> (Drlg, LevelIdx, LevelIdx, DrlgData) {
    let data = DrlgData {
        levels: vec![LevelDef::default(); 140],
        ..DrlgData::default()
    };
    let mut d = Drlg::create(0, 1, 0, 0, false, &data, &mut NoLevelTypes).unwrap();
    let l = d.get_or_alloc_level(&data, &mut NoLevelTypes, 8).unwrap();
    d.level_mut(l).level_type = level_type;
    d.level_mut(l).rect = TileRect::new(0, 0, 1000, 1000);
    let l9 = d.get_or_alloc_level(&data, &mut NoLevelTypes, 9).unwrap();
    (d, l, l9, data)
}

fn mrow(merge: i32) -> MazeRow {
    MazeRow {
        level: 8,
        rooms: [61; 3],
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
    out
}

fn targets(g: &Gen<'_>, c: DrlgRoomId) -> Vec<(LinkTarget, u8)> {
    g.cell(c).links.iter().map(|l| (l.target, l.dir)).collect()
}

use LinkTarget::{Cell as C, Level as L};

// New links go to the head of the list, each side once.
#[test]
fn links_are_prepended() {
    let md = MazeData::default();
    let (mut d, l, _, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(0), &md).unwrap();
    let v = cells(&mut g, &[(10, 10), (20, 10), (10, 0)]);
    let (p, n, m) = (v[0], v[1], v[2]);
    g.link(p, n, 2);
    g.link(p, m, 1);
    // Newest first on P; each neighbour got its reverse link.
    assert_eq!(targets(&g, p), [(C(m), 1), (C(n), 2)]);
    assert_eq!(targets(&g, n), [(C(p), 0)]);
    assert_eq!(targets(&g, m), [(C(p), 3)]);
    // A second link between the same rooms adds nothing.
    g.link(n, p, 0);
    assert_eq!(targets(&g, p), [(C(m), 1), (C(n), 2)]);
    assert!(g.cell(p).links.iter().all(|k| k.init));
}

// Freeing a cell removes the neighbours' links back to it.
#[test]
fn free_removes_the_neighbours_links_back() {
    let md = MazeData::default();
    let (mut d, l, l9, _) = env(3);
    let mut g = Gen::new(&mut d, l, mrow(0), &md).unwrap();
    let v = cells(&mut g, &[(10, 10), (20, 10), (0, 10), (10, 0)]);
    let (n, p, q, r) = (v[0], v[1], v[2], v[3]);
    g.link(p, n, 0);
    g.link(q, n, 2);
    g.link(r, n, 3);
    g.link(r, p, 3);
    g.link_level(p, l9, 2);
    g.free_listed(n);
    // P keeps its other links (init and cross-level), Q has none left,
    // R keeps its link to P.
    assert_eq!(targets(&g, p), [(C(r), 1), (L(l9), 2)]);
    assert!(g.cell(q).links.is_empty());
    assert_eq!(targets(&g, r), [(C(p), 3)]);
    assert_eq!(g.list(), [p, q, r]);

    // Probe: P gains a link to the probe cell, which the free removes.
    let before = g.cell(p).links.clone();
    let count = g.count();
    assert!(g.probe(p, 0).unwrap());
    assert_eq!(g.cell(p).links, before);
    assert_eq!(g.count(), count);
}

// A cross-level link is inserted in order, never displacing the head.
#[test]
fn cross_level_link_sorted_insert() {
    let md = MazeData::default();
    let (mut d, l, l9, _) = env(3);
    d.level_mut(l9).rect = TileRect::new(100, 100, 50, 50);
    let mut g = Gen::new(&mut d, l, mrow(0), &md).unwrap();
    let v = cells(&mut g, &[(10, 10), (20, 10), (0, 10), (10, 0), (10, 20)]);
    let (c, e, w, nn, s) = (v[0], v[1], v[2], v[3], v[4]);

    // Empty list: head.
    g.link_level(c, l9, 2);
    assert_eq!(targets(&g, c), [(L(l9), 2)]);
    g.cell_mut(c).links.clear();

    // One record: before it when it precedes (dir 1 < 2) ...
    g.link(c, e, 2);
    g.link_level(c, l9, 1);
    assert_eq!(targets(&g, c), [(L(l9), 1), (C(e), 2)]);
    g.cell_mut(c).links.remove(0);
    // ... after it when not (dir 3 > 2).
    g.link_level(c, l9, 3);
    assert_eq!(targets(&g, c), [(C(e), 2), (L(l9), 3)]);
    g.cell_mut(c).links.pop();
    // Same direction, by box: dir 2 a.y > b.y (100 > 10) → before.
    g.link_level(c, l9, 2);
    assert_eq!(targets(&g, c), [(L(l9), 2), (C(e), 2)]);
    g.cell_mut(c).links.clear();
    g.cell_mut(e).links.clear();

    // More records: the head is never displaced; it goes before the
    // first record from the second on that it precedes.
    g.link(c, w, 0); // list: [W0]
    g.link(c, nn, 1); // [N1, W0]
    g.link(c, s, 3); // [S3, N1, W0]
    g.link_level(c, l9, 0);
    // Precedes S3 (head, skipped) and N1 (0 < 1): before N1.
    assert_eq!(
        targets(&g, c),
        [(C(s), 3), (L(l9), 0), (C(nn), 1), (C(w), 0)]
    );
    g.cell_mut(c).links.remove(1);
    // Dir 3: it precedes neither N1 nor W0 → tail.
    g.link_level(c, l9, 3);
    assert_eq!(
        targets(&g, c),
        [(C(s), 3), (C(nn), 1), (C(w), 0), (L(l9), 3)]
    );
    assert!(!g.cell(c).links[3].init);
}

// The spiral sets files only after all four branches are grown.
#[test]
fn arcane_spiral_files_after_all_branches() {
    // Merge 1000: merges re-pick (file −1) earlier cells; the file pass
    // after all four branches overwrites that for every recorded cell.
    let md = MazeData::default();
    let (mut d, l, _, _) = env(19);
    let mut g = Gen::new(&mut d, l, mrow(1000), &md).unwrap();
    let f = cells(&mut g, &[(500, 500)])[0];
    let r = g.drlg.level(l).seed.clone().step() & 3;
    layout::spiral(&mut g, f).unwrap();
    assert_eq!(g.count(), 61);
    assert_eq!(g.cell(f).file, 4);
    let list: Vec<DrlgRoomId> = g.list().into_iter().rev().collect();
    // Merges happened (cell 13 of a branch links back to its cell 5,
    // whose re-pick set file −1); every recorded cell still ends with
    // (r + b) mod 4. Branches never share an edge, so no cross-branch
    // merge occurs with this geometry.
    let merged = list[1..]
        .iter()
        .filter(|&&c| g.cell(c).links.len() == 3)
        .count();
    assert!(merged > 8);
    for b in 0..4 {
        for k in 0..15 {
            let c = list[1 + 15 * b + k];
            if k != 8 && k != 12 {
                assert_eq!(
                    g.cell(c).file,
                    ((r + b as u32) % 4) as i32,
                    "branch {b} cell {k}"
                );
            }
        }
    }
}

/// A preset seam whose build allocates one room over the map (head
/// insert) and returns it; logs the links it got.
#[derive(Default)]
struct Builder {
    rects: Vec<TileRect>,
    links: Vec<Vec<MazeLink>>,
    built: Vec<DrlgRoomId>,
    none: bool,
}

impl MazePresets for Builder {
    fn level(&mut self, drlg: &mut Drlg, data: &DrlgData, id: u32) -> Result<LevelIdx, DrlgError> {
        drlg.get_or_alloc_level(data, &mut NoLevelTypes, id)
    }

    fn preset_direction(&self, _: &Drlg, _: LevelIdx) -> Result<u32, DrlgError> {
        Ok(0)
    }

    fn alloc_map(
        &mut self,
        _: &mut Drlg,
        _: &DrlgData,
        _: LevelIdx,
        _: u32,
        rect: TileRect,
    ) -> Result<MapId, DrlgError> {
        self.rects.push(rect);
        Ok(MapId(self.rects.len() as u32 - 1))
    }

    fn set_map_file(&mut self, _: MapId, _: i32) {}

    fn build_map(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
        map: MapId,
        _: bool,
        links: &[MazeLink],
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        self.links.push(links.to_vec());
        if self.none {
            return Ok(None);
        }
        let r = drlg.alloc_room(level, RoomKind::Preset, self.rects[map.0 as usize]);
        drlg.link_room(r, LinkAt::Head);
        self.built.push(r);
        Ok(Some(r))
    }
}

// The build links each built room to its cell's neighbours and frees the cells.
#[test]
fn build_moves_links_to_the_built_rooms() {
    let md = MazeData::default();
    let (mut d, l, _, data) = env(3);
    let mut p = Builder::default();
    let mut g = Gen::new(&mut d, l, mrow(0), &md).unwrap();
    // A — B — C in a row; list C, B, A (build order).
    let v = cells(&mut g, &[(20, 0), (10, 0), (0, 0)]);
    let (c, b, a) = (v[0], v[1], v[2]);
    g.link(a, b, 2);
    g.link(b, c, 2);
    layout::build(&mut g, &data, &mut p, &mut Vec::new()).unwrap();
    let (rc, rb, ra) = (p.built[0], p.built[1], p.built[2]);
    // The links each build got (init, list order): C's to B; B's to C's
    // room (prepended when C's room was linked) then A; A's to B's room.
    let got: Vec<Vec<(LinkTarget, u8)>> = p
        .links
        .iter()
        .map(|ls| ls.iter().map(|k| (k.target, k.dir)).collect())
        .collect();
    assert_eq!(
        got,
        [
            vec![(C(b), 0)],
            vec![(C(rc), 2), (C(a), 0)],
            vec![(C(rb), 2)],
        ]
    );
    // The built rooms carry the links among themselves; the cells are
    // gone.
    assert_eq!(targets(&g, ra), [(C(rb), 2)]);
    assert_eq!(targets(&g, rb), [(C(ra), 0), (C(rc), 2)]);
    assert_eq!(targets(&g, rc), [(C(rb), 0)]);
    assert!([a, b, c].iter().all(|x| !g.cells.contains_key(x)));
    assert_eq!(g.list(), [ra, rb, rc]);
    assert_eq!(g.count(), 3);
}

// A build that returns no room for a cell with links is an error.
#[test]
fn build_without_a_room_and_links_is_the_null_crash() {
    let md = MazeData::default();
    let (mut d, l, _, data) = env(3);
    let mut p = Builder {
        none: true,
        ..Builder::default()
    };
    let mut g = Gen::new(&mut d, l, mrow(0), &md).unwrap();
    let v = cells(&mut g, &[(10, 0), (0, 0)]);
    g.link(v[1], v[0], 2);
    assert_eq!(
        layout::build(&mut g, &data, &mut p, &mut Vec::new()),
        Err(MazeError::NullCell("built room"))
    );
}
