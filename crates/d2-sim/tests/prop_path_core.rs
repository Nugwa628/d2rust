//! Property tests of the path core (`d2_sim::path`: coordinates,
//! collision queries, footprints) against a reference model:
//!
//! - §1 coordinates: sub-tile ↔ 16.16 centre round trip over the u16
//!   range the record holds, client coordinates of a centre, `dist_sq`.
//! - §4 rule 1 cell lookup through partial, ordered adjacency arrays.
//! - §4–§6: random sequences of footprint add / remove (per kind), try /
//!   forced / missile moves and box set / clear on a multi-room layout
//!   (gaps, a room without a grid, inactive and null room arguments)
//!   keep every collision grid equal to the model grid, return the
//!   model's results, and every point / plus / box / size / pattern query
//!   equals the model's.
//!
//! The adjacency of the footprint layout is complete, so every active
//! room finds every cell's room. These cases cannot distinguish alternate
//! room hints for footprint operations; that choice remains outside this check.

use std::collections::BTreeMap;

use d2_sim::drlg::{CollisionGrid, TileRect};
use d2_sim::path::collision::{
    box_apply, box_value, find_room, pattern_collides, pattern_value, plus_value, point_value,
    size_value, CollisionRooms,
};
use d2_sim::path::coords::{
    client_from_precise, client_from_subtile, dist_sq, subtile_of, tile_to_subtile, to_fp16_center,
    FP16_CENTER,
};
use d2_sim::path::footprint::{
    add_footprint, forced_move, missile_move, remove_footprint, try_move, FootShape, Footprint,
    RemoveRule,
};
use d2_sim::path::ObjectShape;
use d2_sim::units::RoomId;
use proptest::prelude::*;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

// ---- §1 coordinates --------------------------------------------------

proptest! {
    #![proptest_config(config(512))]

    /// §1 rule 2: the high 16 bits are the sub-tile, a resting unit's
    /// fraction is 0x8000; the record's u16 range round-trips.
    #[test]
    fn centre_round_trips(sub in 0i32..=0xFFFF) {
        let p = to_fp16_center(sub);
        prop_assert_eq!(subtile_of(p), sub);
        prop_assert_eq!(p & 0xFFFF, FP16_CENTER);
        prop_assert_eq!(p >> 16, sub as u32);
    }

    /// §1 rule 1: tile × 5; rule 3: client coordinates of both forms;
    /// rule 4: dx² + dy².
    #[test]
    fn client_and_distance(x in 0i32..0x8000, y in 0i32..0x8000, t in -0x1000i32..0x1000) {
        prop_assert_eq!(tile_to_subtile(t), t * 5);
        // Static: (x − y)·16, (x + y)·8.
        prop_assert_eq!(client_from_subtile(x, y), ((x - y) * 16, (x + y) * 8));
        // Dynamic at a centre: a = (x << 16 | 0x8000) >> 11 = 32x + 16.
        let (a, b) = (32 * x + 16, 32 * y + 16);
        prop_assert_eq!(
            client_from_precise(to_fp16_center(x), to_fp16_center(y)),
            ((a - b) >> 1, (a + b) >> 2)
        );
        let (dx, dy) = (x - y, t);
        prop_assert_eq!(dist_sq(dx, dy) as i64, (dx as i64).pow(2) + (dy as i64).pow(2));
        prop_assert_eq!(dist_sq(dx, dy), dist_sq(-dx, dy));
    }
}

// ---- layout and reference model ------------------------------------

/// Room slots: 3 × 3 squares of `S` sub-tiles from (BX, BY).
const S: i32 = 6;
const BX: i32 = 100;
const BY: i32 = 200;
/// A room id no slot uses (inactive).
const INACTIVE: RoomId = RoomId(77);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Absent,
    Grid,
    NoGrid,
}

/// The rooms of a layout; the fake `CollisionRooms` and the model read
/// the same rects and adjacency.
#[derive(Clone, Debug)]
struct Rooms {
    rects: BTreeMap<RoomId, TileRect>,
    adj: BTreeMap<RoomId, Vec<RoomId>>,
    grids: BTreeMap<RoomId, CollisionGrid>,
}

impl CollisionRooms for Rooms {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.rects.get(&room).copied()
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.adj.get(&room).map_or(0, Vec::len)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.adj.get(&room)?.get(i).copied()
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.grids.get(&room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.grids.get_mut(&room)
    }
}

fn slot_rect(i: usize) -> TileRect {
    TileRect::new(BX + (i % 3) as i32 * S, BY + (i / 3) as i32 * S, S, S)
}

/// Builds the rooms of `slots` with complete adjacency (each room lists
/// every other active room, in `order`) and the initial cell values.
fn build(slots: &[Slot], order: &[usize], init: &[u16]) -> Rooms {
    let mut rooms = Rooms {
        rects: BTreeMap::new(),
        adj: BTreeMap::new(),
        grids: BTreeMap::new(),
    };
    for (i, &s) in slots.iter().enumerate() {
        if s == Slot::Absent {
            continue;
        }
        let id = RoomId(i as u32);
        let rect = slot_rect(i);
        rooms.rects.insert(id, rect);
        if s == Slot::Grid {
            let mut g = CollisionGrid::new(rect);
            for (k, m) in g.masks.iter_mut().enumerate() {
                *m = init[(i * 36 + k) % init.len()];
            }
            rooms.grids.insert(id, g);
        }
    }
    let ids: Vec<RoomId> = order
        .iter()
        .map(|&i| RoomId(i as u32))
        .filter(|r| rooms.rects.contains_key(r))
        .collect();
    for &r in rooms.rects.keys() {
        rooms
            .adj
            .insert(r, ids.iter().copied().filter(|&o| o != r).collect());
    }
    rooms
}

/// The reference model: rects and adjacency as given, cell values in a
/// flat map (cells of rooms with a grid only).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Model {
    rects: BTreeMap<RoomId, TileRect>,
    adj: BTreeMap<RoomId, Vec<RoomId>>,
    gridded: Vec<RoomId>,
    cells: BTreeMap<(i32, i32), u16>,
}

const MISSING: u16 = 0x27;

type Cells = &'static [(i32, i32)];

fn inside(r: &TileRect, x: i32, y: i32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

const PLUS: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];
const POINT: [(i32, i32); 1] = [(0, 0)];
const BOX3: [(i32, i32); 9] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (0, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

impl Model {
    fn of(rooms: &Rooms) -> Model {
        let mut cells = BTreeMap::new();
        for g in rooms.grids.values() {
            let r = g.rect;
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    cells.insert((x, y), g.masks[((y - r.y) * r.w + (x - r.x)) as usize]);
                }
            }
        }
        Model {
            rects: rooms.rects.clone(),
            adj: rooms.adj.clone(),
            gridded: rooms.grids.keys().copied().collect(),
            cells,
        }
    }

    /// §4 rule 1: the room itself, else the first adjacent room that
    /// contains the cell, else none; a null or inactive room gives none.
    fn lookup(&self, room: Option<RoomId>, x: i32, y: i32) -> Option<RoomId> {
        let room = room?;
        let rect = self.rects.get(&room)?;
        if inside(rect, x, y) {
            return Some(room);
        }
        self.adj
            .get(&room)?
            .iter()
            .copied()
            .find(|r| self.rects.get(r).is_some_and(|rr| inside(rr, x, y)))
    }

    /// §4 rule 2: masked value, or 0x27 unmasked without a room / grid.
    fn value(&self, found: Option<RoomId>, x: i32, y: i32, mask: u16) -> u16 {
        match found {
            Some(r) if self.gridded.contains(&r) => self.cells[&(x, y)] & mask,
            _ => MISSING,
        }
    }

    fn point_q(&self, room: Option<RoomId>, x: i32, y: i32, mask: u16) -> u16 {
        self.value(self.lookup(room, x, y), x, y, mask)
    }

    /// §4 rule 3: the centre's room, neighbours looked up from it.
    fn plus_q(&self, room: Option<RoomId>, x: i32, y: i32, mask: u16) -> u16 {
        let Some(c) = self.lookup(room, x, y) else {
            return MISSING;
        };
        PLUS.iter().fold(0, |a, &(dx, dy)| {
            a | self.value(self.lookup(Some(c), x + dx, y + dy), x + dx, y + dy, mask)
        })
    }

    /// §4 rule 4 box walk: the room of (left, bottom); the box clipped at
    /// its right and top edges; the strip right of the room and the strip
    /// above it at the inside box's width walked again from that room.
    /// `inside` gets (room, l, b, r, t); `missing` a sub-box without room.
    fn walk_box(
        &self,
        room: Option<RoomId>,
        (l, b, r, t): (i32, i32, i32, i32),
        inside_fn: &mut dyn FnMut(RoomId, i32, i32, i32, i32),
        missing: &mut dyn FnMut(),
    ) {
        if l > r || b > t {
            return;
        }
        let Some(found) = self.lookup(room, l, b) else {
            missing();
            return;
        };
        let rect = self.rects[&found];
        let ir = r.min(rect.x + rect.w - 1);
        let it = t.min(rect.y + rect.h - 1);
        inside_fn(found, l, b, ir, it);
        if r > ir {
            self.walk_box(Some(found), (ir + 1, b, r, t), inside_fn, missing);
        }
        if t > it {
            self.walk_box(Some(found), (l, it + 1, ir, t), inside_fn, missing);
        }
    }

    fn corners(x: i32, y: i32, sx: u32, sy: u32) -> (i32, i32, i32, i32) {
        let l = x - (sx / 2) as i32;
        let b = y - (sy / 2) as i32;
        (l, b, l + sx as i32 - 1, b + sy as i32 - 1)
    }

    fn box_q(&self, room: Option<RoomId>, x: i32, y: i32, (sx, sy): (u32, u32), mask: u16) -> u16 {
        let mut acc = 0u16;
        let missing = std::cell::Cell::new(false);
        self.walk_box(
            room,
            Self::corners(x, y, sx, sy),
            &mut |rm, l, b, r, t| {
                for cy in b..=t {
                    for cx in l..=r {
                        acc |= self.value(Some(rm), cx, cy, mask);
                    }
                }
            },
            &mut || missing.set(true),
        );
        if missing.get() {
            acc |= MISSING;
        }
        acc
    }

    fn box_apply(
        &mut self,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        s: (u32, u32),
        mask: u16,
        set: bool,
    ) {
        let mut hits = Vec::new();
        self.walk_box(
            room,
            Self::corners(x, y, s.0, s.1),
            &mut |rm, l, b, r, t| {
                for cy in b..=t {
                    for cx in l..=r {
                        hits.push((rm, cx, cy));
                    }
                }
            },
            &mut || {},
        );
        for (rm, cx, cy) in hits {
            if self.gridded.contains(&rm) {
                let v = self.cells.get_mut(&(cx, cy)).unwrap();
                if set {
                    *v |= mask;
                } else {
                    *v &= !mask;
                }
            }
        }
    }

    /// §4 rule 5 size query.
    fn size_q(&self, room: Option<RoomId>, x: i32, y: i32, size: i32, mask: u16) -> u16 {
        match size {
            0 | 1 => self.point_q(room, x, y, mask),
            2 => self.plus_q(room, x, y, mask),
            3 => self.box_q(room, x, y, (3, 3), mask),
            _ => 0xFFFF,
        }
    }

    /// §4 rule 5 pattern query.
    fn pattern_q(&self, room: Option<RoomId>, x: i32, y: i32, pattern: u32, mask: u16) -> u16 {
        match pattern {
            0 => self.point_q(room, x, y, mask),
            1 | 3 | 5 => self.plus_q(room, x, y, mask),
            2 | 4 => self.box_q(room, x, y, (3, 3), mask),
            _ => 0xFFFF,
        }
    }

    /// §5.1: OR / AND-out `mask` on each cell that has a room (with a
    /// grid); a null room does nothing.
    fn apply(
        &mut self,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        cells: &[(i32, i32)],
        mask: u16,
        set: bool,
    ) {
        if room.is_none() {
            return;
        }
        for &(dx, dy) in cells {
            let (cx, cy) = (x + dx, y + dy);
            if let Some(r) = self.lookup(room, cx, cy) {
                if self.gridded.contains(&r) {
                    let v = self.cells.get_mut(&(cx, cy)).unwrap();
                    if set {
                        *v |= mask;
                    } else {
                        *v &= !mask;
                    }
                }
            }
        }
    }

    /// §5.1 pattern stamp / clear with the §3 cells and marker (marker
    /// only when the mask is not 0); pattern 0 and patterns above 5 do
    /// nothing.
    fn pattern(
        &mut self,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        pattern: u32,
        mask: u16,
        set: bool,
    ) {
        let (cells, marker): (Cells, Option<(u16, Cells)>) = match pattern {
            // Pattern 0 stamps and clears nothing (§5.1).
            0 => return,
            1 => (&PLUS, Some((0x1000, &POINT))),
            2 => (&BOX3, Some((0x1000, &PLUS))),
            3 => (&PLUS, Some((0x2000, &POINT))),
            4 => (&BOX3, Some((0x2000, &PLUS))),
            5 => (&PLUS, None),
            _ => return,
        };
        self.apply(room, x, y, cells, mask, set);
        if mask != 0 {
            if let Some((m, c)) = marker {
                self.apply(room, x, y, c, m, set);
            }
        }
    }

    /// §5.1 size stamp / clear: 1 the cell, 2 plus, 3 box, others nothing.
    fn size(&mut self, room: Option<RoomId>, x: i32, y: i32, size: i32, mask: u16, set: bool) {
        let cells: &[(i32, i32)] = match size {
            1 => &POINT,
            2 => &PLUS,
            3 => &BOX3,
            _ => return,
        };
        self.apply(room, x, y, cells, mask, set);
    }

    /// §5.2 per-kind shape.
    fn footprint(&mut self, fp: &Footprint, set: bool) {
        match fp.shape {
            FootShape::Pattern(p) => self.pattern(fp.room, fp.x, fp.y, p, fp.mask, set),
            FootShape::Box { size_x, size_y } => {
                self.box_apply(fp.room, fp.x, fp.y, (size_x, size_y), fp.mask, set)
            }
            FootShape::Size(s) => self.size(fp.room, fp.x, fp.y, s, fp.mask, set),
        }
    }
}

// ---- operations -----------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    Add(Footprint),
    Remove(Footprint, RemoveRule, bool),
    TryMove {
        room: Option<RoomId>,
        old: (i32, i32),
        new: (i32, i32),
        pattern: u32,
        foot: u16,
        test: u16,
    },
    Forced {
        room: Option<RoomId>,
        old: (i32, i32),
        new: (i32, i32),
        pattern: u32,
        foot: u16,
    },
    Missile {
        room: Option<RoomId>,
        old: (i32, i32),
        new: (i32, i32),
        size: i32,
        foot: u16,
        test: u16,
    },
    Box {
        room: Option<RoomId>,
        at: (i32, i32),
        size: (u32, u32),
        mask: u16,
        set: bool,
    },
}

fn room_arg() -> impl Strategy<Value = Option<RoomId>> {
    prop_oneof![
        6 => (0u32..9).prop_map(|i| Some(RoomId(i))),
        1 => Just(None),
        1 => Just(Some(INACTIVE)),
    ]
}

fn point() -> impl Strategy<Value = (i32, i32)> {
    (BX - 3..BX + 3 * S + 3, BY - 3..BY + 3 * S + 3)
}

fn mask() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(0u16),
        Just(0x1),
        Just(0x4),
        Just(0x80),
        Just(0x100),
        Just(0x200),
        Just(0x400),
        Just(0x804),
        Just(0x1000),
        Just(0x2000),
        Just(0x8000),
        Just(0x1C09),
        Just(0x3C01),
        any::<u16>(),
    ]
}

fn shape() -> impl Strategy<Value = FootShape> {
    prop_oneof![
        (0u32..7).prop_map(FootShape::Pattern),
        (0u32..5, 0u32..5).prop_map(|(size_x, size_y)| FootShape::Box { size_x, size_y }),
        (-1i32..5).prop_map(FootShape::Size),
    ]
}

fn footprint() -> impl Strategy<Value = Footprint> {
    (room_arg(), point(), shape(), mask()).prop_map(|(room, (x, y), shape, mask)| Footprint {
        room,
        x,
        y,
        shape,
        mask,
    })
}

fn rule() -> impl Strategy<Value = RemoveRule> {
    prop_oneof![
        (0u32..20).prop_map(|mode| RemoveRule::Player { mode }),
        (0u32..16).prop_map(|mode| RemoveRule::Monster { mode }),
        (0u32..10, any::<[bool; 8]>()).prop_map(|(mode, hc)| RemoveRule::Object {
            mode,
            shape: ObjectShape {
                has_collision: hc,
                ..ObjectShape::default()
            },
        }),
        Just(RemoveRule::Other),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => footprint().prop_map(Op::Add),
        2 => (footprint(), rule(), any::<bool>()).prop_map(|(f, r, force)| Op::Remove(f, r, force)),
        3 => (room_arg(), point(), point(), 0u32..7, mask(), mask()).prop_map(
            |(room, old, new, pattern, foot, test)| Op::TryMove { room, old, new, pattern, foot, test }
        ),
        1 => (room_arg(), point(), point(), 0u32..7, mask()).prop_map(
            |(room, old, new, pattern, foot)| Op::Forced { room, old, new, pattern, foot }
        ),
        2 => (room_arg(), point(), point(), -1i32..5, mask(), mask()).prop_map(
            |(room, old, new, size, foot, test)| Op::Missile { room, old, new, size, foot, test }
        ),
        1 => (room_arg(), point(), (0u32..6, 0u32..6), mask(), any::<bool>()).prop_map(
            |(room, at, size, mask, set)| Op::Box { room, at, size, mask, set }
        ),
    ]
}

fn slots() -> impl Strategy<Value = Vec<Slot>> {
    proptest::collection::vec(
        prop_oneof![1 => Just(Slot::Absent), 5 => Just(Slot::Grid), 1 => Just(Slot::NoGrid)],
        9,
    )
}

fn init_values() -> impl Strategy<Value = Vec<u16>> {
    proptest::collection::vec(
        prop_oneof![4 => Just(0u16), 1 => Just(0x1), 1 => Just(0x400), 1 => any::<u16>()],
        1..64,
    )
}

/// The rule of the §5.2 remove table.
fn clears(rule: RemoveRule, force: bool) -> bool {
    force
        || match rule {
            RemoveRule::Player { mode } => mode != 0 && mode != 17,
            RemoveRule::Monster { mode } => mode != 0 && mode != 12,
            RemoveRule::Object { mode, shape } => mode < 8 && shape.has_collision[mode as usize],
            RemoveRule::Other => true,
        }
}

fn check_queries(
    rooms: &Rooms,
    model: &Model,
    probes: &[(Option<RoomId>, (i32, i32), u16)],
) -> Result<(), TestCaseError> {
    for &(room, (x, y), m) in probes {
        prop_assert_eq!(
            point_value(rooms, room, x, y, m),
            model.point_q(room, x, y, m)
        );
        prop_assert_eq!(
            plus_value(rooms, room, x, y, m),
            model.plus_q(room, x, y, m)
        );
        for s in -1..5 {
            prop_assert_eq!(
                size_value(rooms, room, x, y, s, m),
                model.size_q(room, x, y, s, m),
                "size {}",
                s
            );
        }
        for p in 0..7u32 {
            let q = model.pattern_q(room, x, y, p, m);
            prop_assert_eq!(pattern_value(rooms, room, x, y, p, m), q, "pattern {}", p);
            // Any masked collision blocks; unsupported patterns also block.
            prop_assert_eq!(pattern_collides(rooms, room, x, y, p, m), p > 5 || q != 0);
        }
        for (sx, sy) in [(1u32, 1u32), (2, 3), (4, 1), (5, 5)] {
            prop_assert_eq!(
                box_value(rooms, room, x, y, (sx, sy), m),
                model.box_q(room, x, y, (sx, sy), m)
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config(256))]

    /// §4–§6: the collision grids follow the model through any sequence
    /// of footprint operations; results and queries agree.
    #[test]
    fn footprints_match_the_model(
        slots in slots(),
        order in Just((0..9).collect::<Vec<usize>>()).prop_shuffle(),
        init in init_values(),
        ops in proptest::collection::vec(op(), 1..40),
        probes in proptest::collection::vec((room_arg(), point(), mask()), 8),
    ) {
        let mut rooms = build(&slots, &order, &init);
        let mut model = Model::of(&rooms);
        check_queries(&rooms, &model, &probes)?;
        for op in &ops {
            match *op {
                Op::Add(fp) => {
                    add_footprint(&mut rooms, &fp);
                    model.footprint(&fp, true);
                }
                Op::Remove(fp, rule, force) => {
                    let got = remove_footprint(&mut rooms, &fp, rule, force);
                    let want = clears(rule, force);
                    prop_assert_eq!(got, want);
                    if want {
                        model.footprint(&fp, false);
                    }
                }
                Op::TryMove { room, old, new, pattern, foot, test } => {
                    let got = try_move(&mut rooms, room, old, new, pattern, foot, test);
                    // §6 rule 1.
                    model.pattern(room, old.0, old.1, pattern, foot, false);
                    let r = model.pattern_q(room, new.0, new.1, pattern, test);
                    let at = if r != 0 { old } else { new };
                    model.pattern(room, at.0, at.1, pattern, foot, true);
                    prop_assert_eq!(got, r);
                }
                Op::Forced { room, old, new, pattern, foot } => {
                    forced_move(&mut rooms, room, old, new, pattern, foot);
                    // §6 rule 2 (a null room does nothing).
                    if room.is_some() {
                        model.pattern(room, old.0, old.1, pattern, foot, false);
                        model.pattern(room, new.0, new.1, pattern, foot, true);
                    }
                }
                Op::Missile { room, old, new, size, foot, test } => {
                    let got = missile_move(&mut rooms, room, old, new, size, foot, test);
                    // §6 rule 3.
                    model.size(room, old.0, old.1, size, foot, false);
                    let r = model.size_q(room, new.0, new.1, size, test);
                    let at = if r & 0x5 == 0 { new } else { old };
                    model.size(room, at.0, at.1, size, foot, true);
                    prop_assert_eq!(got, r);
                }
                Op::Box { room, at, size, mask, set } => {
                    box_apply(&mut rooms, room, at.0, at.1, size, mask, set);
                    model.box_apply(room, at.0, at.1, size, mask, set);
                }
            }
            prop_assert_eq!(&Model::of(&rooms).cells, &model.cells, "after {:?}", op);
        }
        check_queries(&rooms, &model, &probes)?;
    }

    /// §4 rule 1 with partial, ordered adjacency arrays (rooms may
    /// overlap: the first match in array order wins).
    #[test]
    fn cell_lookup_follows_adjacency(
        rects in proptest::collection::vec((0i32..20, 0i32..20, 0i32..8, 0i32..8), 1..6),
        adj in proptest::collection::vec(proptest::collection::vec(0u32..7, 0..6), 6),
        probes in proptest::collection::vec((prop_oneof![(0u32..7).prop_map(Some), Just(None)], -2i32..30, -2i32..30), 32),
    ) {
        let mut rooms = Rooms { rects: BTreeMap::new(), adj: BTreeMap::new(), grids: BTreeMap::new() };
        for (i, &(x, y, w, h)) in rects.iter().enumerate() {
            rooms.rects.insert(RoomId(i as u32), TileRect::new(x, y, w, h));
            rooms.adj.insert(RoomId(i as u32), adj[i].iter().map(|&a| RoomId(a)).collect());
        }
        let model = Model::of(&rooms);
        for &(room, x, y) in &probes {
            let room = room.map(RoomId);
            prop_assert_eq!(find_room(&rooms, room, x, y), model.lookup(room, x, y));
        }
    }
}
