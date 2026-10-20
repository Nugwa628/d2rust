//! The multi-room world fake shared by `prop_walk_rooms`,
//! `prop_walk_motion` and `prop_path_footprint_ops`.
//!
//! Rooms tile a grid of `rx` × `ry` rooms of `w` × `h` sub-tiles; a slot
//! can be a hole (no room). The adjacency arrays (`drlg/rooms.md` §6) are
//! either the 8-neighbour rooms or every room, rotated by a given amount
//! so that their order varies. With either, every cell within two cells
//! of a room's cell is found from that room (`path-placement.md` §4
//! rule 1), so the reference views below (one lookup over the union of
//! the rooms) equal the lookup from the room argument that
//! `path-placement.md` §5.1 states, as `docs/handoff/prop-walk.md` §4
//! notes for the one-room fake.
//!
//! The seams record what they are asked to do (`Ev`) so the properties
//! can check the room lists, the update queue, the unit flag, the AI
//! room memo and the add / removal messages against the spec.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::OnceLock;

use d2_sim::drlg::{CollisionGrid, TileRect};
use d2_sim::path::collision::CollisionRooms;
use d2_sim::path::coords::to_fp16_center;
use d2_sim::path::footprint::PathMotion;
use d2_sim::path::record::{DynamicPath, PathPoint};
use d2_sim::path::walk::seams::{PathWorld, Point, WalkUnits};
use d2_sim::path::walk::Walk;
use d2_sim::path::PathTables;
use d2_sim::rng::Seed;
use d2_sim::units::{ClientId, RoomId, UnitId, UnitType};
use proptest::prelude::*;

pub fn config(default: u32) -> ProptestConfig {
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

/// The spec's machine tables (`sim/path-tables.tsv`), parsed once.
pub fn tables() -> &'static PathTables {
    static T: OnceLock<PathTables> = OnceLock::new();
    T.get_or_init(|| PathTables::spec().unwrap())
}

pub const P: UnitId = UnitId(1);
pub const T: UnitId = UnitId(2);
pub const M: UnitId = UnitId(3);
/// Lower-left sub-tile of room (0, 0).
pub const OX: i32 = 40;
pub const OY: i32 = 60;
pub const WALL: u16 = 0x1;
/// Cell value without a room (`path-placement.md` §4 rule 2).
pub const MISSING: u16 = 0x27;

/// What a seam was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    ListRemove(UnitId, RoomId),
    ListInsert(UnitId, RoomId),
    Queue(UnitId),
    UnitFlag(UnitId, u32),
    AiMemo(UnitId),
    Removal(ClientId, UnitId),
    Add(ClientId, UnitId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Room {
    pub grid: CollisionGrid,
    pub adj: Vec<RoomId>,
    pub clients: Vec<ClientId>,
    pub town: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub ty: UnitType,
    pub guid: u32,
    pub mode: u32,
    /// Position of a unit without a path record.
    pub pos: Point,
    pub size: i32,
    pub stats: BTreeMap<u16, i32>,
    pub seed: Seed,
    pub run_bonus: i32,
}

impl Unit {
    pub fn new(ty: UnitType, guid: u32) -> Unit {
        Unit {
            ty,
            guid,
            mode: 1,
            pos: Point::default(),
            size: 2,
            stats: BTreeMap::from([(10, 100 << 8)]),
            seed: Seed::default(),
            run_bonus: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adjacency {
    /// The 8-neighbour rooms, each array rotated by the amount.
    Neighbours(usize),
    /// Every other room, rotated.
    Complete(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    pub rooms: Vec<Option<Room>>,
    pub paths: BTreeMap<UnitId, DynamicPath>,
    pub units: BTreeMap<UnitId, Unit>,
    pub client_players: BTreeMap<ClientId, UnitId>,
    pub frame: i32,
    pub events: Vec<Ev>,
}

impl World {
    /// `rx` × `ry` rooms of `w` × `h`; `holes[i]` leaves slot i empty;
    /// `masks` fills the cells (index over the union, cycled).
    pub fn layout(
        rx: i32,
        ry: i32,
        w: i32,
        h: i32,
        holes: &[bool],
        masks: &[u16],
        adj: Adjacency,
    ) -> World {
        let mut rooms = Vec::new();
        for j in 0..ry {
            for i in 0..rx {
                let id = (j * rx + i) as usize;
                if holes.get(id).copied().unwrap_or(false) {
                    rooms.push(None);
                    continue;
                }
                let mut grid = CollisionGrid::new(TileRect::new(OX + i * w, OY + j * h, w, h));
                for y in 0..h {
                    for x in 0..w {
                        let gx = i * w + x;
                        let gy = j * h + y;
                        let k = (gy * rx * w + gx) as usize;
                        grid.masks[(y * w + x) as usize] = masks[k % masks.len()];
                    }
                }
                rooms.push(Some(Room {
                    grid,
                    adj: Vec::new(),
                    clients: Vec::new(),
                    town: false,
                }));
            }
        }
        for j in 0..ry {
            for i in 0..rx {
                let id = (j * rx + i) as usize;
                if rooms[id].is_none() {
                    continue;
                }
                let mut list: Vec<RoomId> = Vec::new();
                for (k, r) in rooms.iter().enumerate() {
                    let (ki, kj) = (k as i32 % rx, k as i32 / rx);
                    if k == id || r.is_none() {
                        continue;
                    }
                    let near = (ki - i).abs() <= 1 && (kj - j).abs() <= 1;
                    if near || matches!(adj, Adjacency::Complete(_)) {
                        list.push(RoomId(k as u32));
                    }
                }
                let rot = match adj {
                    Adjacency::Neighbours(r) | Adjacency::Complete(r) => r,
                };
                if !list.is_empty() {
                    let n = rot % list.len();
                    list.rotate_left(n);
                }
                rooms[id].as_mut().unwrap().adj = list;
            }
        }
        World {
            rooms,
            paths: BTreeMap::new(),
            units: BTreeMap::new(),
            client_players: BTreeMap::new(),
            frame: 0,
            events: Vec::new(),
        }
    }

    pub fn room(&self, r: RoomId) -> Option<&Room> {
        self.rooms.get(r.0 as usize).and_then(|r| r.as_ref())
    }

    /// Whether the room's rect contains `p` (an inactive room: none).
    pub fn contains(&self, r: RoomId, p: Point) -> bool {
        self.room(r).is_some_and(|r| r.grid.rect.contains(p.x, p.y))
    }

    /// Reference cell lookup (`path-placement.md` §4 rule 1).
    pub fn lookup(&self, from: Option<RoomId>, p: Point) -> Option<RoomId> {
        let r = from?;
        let room = self.room(r)?;
        if room.grid.rect.contains(p.x, p.y) {
            return Some(r);
        }
        room.adj.iter().copied().find(|&a| self.contains(a, p))
    }

    /// The room of the union holding `p`.
    pub fn room_at(&self, p: Point) -> Option<RoomId> {
        (0..self.rooms.len() as u32)
            .map(RoomId)
            .find(|&r| self.contains(r, p))
    }

    /// Cell value (§4 rule 2): `MISSING` unmasked where no room is.
    pub fn value(&self, p: Point) -> Option<u16> {
        let r = self.room_at(p)?;
        self.room(r).unwrap().grid.get(p.x, p.y)
    }

    pub fn masked(&self, p: Point, mask: u16) -> u16 {
        self.value(p).map_or(MISSING, |v| v & mask)
    }

    /// Pattern / size query over the union (§4 rule 5).
    pub fn query(&self, at: Point, cells: Option<&[(i32, i32)]>, mask: u16) -> u16 {
        let Some(cs) = cells else { return 0xFFFF };
        cs.iter().fold(0, |a, &(dx, dy)| {
            a | self.masked(Point::new(at.x + dx, at.y + dy), mask)
        })
    }

    /// Free for a walker: the pattern meets no wall and lies in the union.
    pub fn free(&self, p: Point, pattern: u32) -> bool {
        self.query(p, pattern_cells(pattern), WALL) == 0
    }

    fn apply(&mut self, at: Point, cells: &[(i32, i32)], m: u16, set: bool) {
        for &(dx, dy) in cells {
            let c = Point::new(at.x + dx, at.y + dy);
            let Some(r) = self.room_at(c) else { continue };
            let g = &mut self.rooms[r.0 as usize].as_mut().unwrap().grid;
            let v = g.get_mut(c.x, c.y).unwrap();
            if set {
                *v |= m;
            } else {
                *v &= !m;
            }
        }
    }

    /// Pattern stamp / clear (§5.1) with the §3 marker when the mask is
    /// not 0.
    pub fn stamp(&mut self, at: Point, pattern: u32, mask: u16, set: bool) {
        // Pattern 0 stamps and clears nothing (§5.1).
        if pattern == 0 {
            return;
        }
        let Some(cs) = pattern_cells(pattern) else {
            return;
        };
        self.apply(at, cs, mask, set);
        if mask != 0 {
            if let Some((m, c)) = pattern_marker(pattern) {
                self.apply(at, c, m, set);
            }
        }
    }

    /// Size stamp / clear (§5.1): 1 the cell, 2 plus, 3 box, others
    /// nothing, no marker.
    pub fn stamp_size(&mut self, at: Point, size: i32, mask: u16, set: bool) {
        if let Some(cs) = size_stamp_cells(size) {
            self.apply(at, cs, mask, set);
        }
    }

    /// Every cell value of the union, in room order.
    pub fn grids(&self) -> Vec<Option<Vec<u16>>> {
        self.rooms
            .iter()
            .map(|r| r.as_ref().map(|r| r.grid.masks.clone()))
            .collect()
    }

    /// A player (`walker` true) or monster path at `p` (`path-placement.md`
    /// §2.4), stamped, in its room's list.
    pub fn add_walker(&mut self, unit: UnitId, ty: UnitType, p: Point) {
        let t = tables();
        let mut d = DynamicPath {
            owner: Some(unit),
            unit_size: 2,
            pattern: 1,
            precise_x: to_fp16_center(p.x),
            precise_y: to_fp16_center(p.y),
            velocity: 0x800,
            room: self.room_at(p),
            saved_count: 1,
            ..DynamicPath::default()
        };
        d.update_client();
        d.saved_steps[0] = PathPoint::from_point(p);
        if ty == UnitType::Player {
            d.foot_mask = 0x80;
            d.move_mask = 0x1C09;
            d.set_path_type(t, true, 7).unwrap();
            d.max_distance = 73;
            d.ida_score = 70;
        } else {
            d.foot_mask = 0x100;
            d.move_mask = 0x3C01;
            d.set_path_type(t, false, 2).unwrap();
            d.max_distance = 14;
        }
        self.stamp(p, d.pattern, d.foot_mask, true);
        self.paths.insert(unit, d);
        let mut u = Unit::new(ty, unit.0);
        u.pos = p;
        self.units.insert(unit, u);
    }

    /// Clears walls on the plus around `p` so a walker stands free.
    pub fn clear_plus(&mut self, p: Point) {
        self.apply(p, pattern_cells(1).unwrap(), WALL, false);
    }

    pub fn unit(&self, u: UnitId) -> &Unit {
        &self.units[&u]
    }

    pub fn unit_mut(&mut self, u: UnitId) -> &mut Unit {
        self.units.get_mut(&u).unwrap()
    }
}

pub fn pattern_cells(pattern: u32) -> Option<&'static [(i32, i32)]> {
    match pattern {
        0 => Some(&POINT),
        1 | 3 | 5 => Some(&PLUS),
        2 | 4 => Some(&BOX),
        _ => None,
    }
}

/// Size-query cells (§4 rule 5: 0, 1 point; 2 plus; 3 box).
pub fn size_query_cells(size: i32) -> Option<&'static [(i32, i32)]> {
    match size {
        0 | 1 => Some(&POINT),
        2 => Some(&PLUS),
        3 => Some(&BOX),
        _ => None,
    }
}

fn size_stamp_cells(size: i32) -> Option<&'static [(i32, i32)]> {
    match size {
        1 => Some(&POINT),
        2 => Some(&PLUS),
        3 => Some(&BOX),
        _ => None,
    }
}

pub fn pattern_marker(pattern: u32) -> Option<(u16, &'static [(i32, i32)])> {
    match pattern {
        1 => Some((0x1000, &POINT)),
        2 => Some((0x1000, &PLUS)),
        3 => Some((0x2000, &POINT)),
        4 => Some((0x2000, &PLUS)),
        _ => None,
    }
}

const POINT: [(i32, i32); 1] = [(0, 0)];
const PLUS: [(i32, i32); 5] = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];
const BOX: [(i32, i32); 9] = [
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

impl CollisionRooms for World {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.room(room).map(|r| r.grid.rect)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.room(room).map_or(0, |r| r.adj.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.room(room).and_then(|r| r.adj.get(i).copied())
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.room(room).map(|r| &r.grid)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.rooms
            .get_mut(room.0 as usize)
            .and_then(|r| r.as_mut())
            .map(|r| &mut r.grid)
    }
}

impl PathWorld for World {
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath> {
        self.paths.get(&unit).cloned()
    }
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath) {
        self.paths.insert(unit, path.clone());
    }
    fn room_in_town(&self, room: RoomId) -> bool {
        self.room(room).is_some_and(|r| r.town)
    }
    fn remove_footprint(&mut self, unit: UnitId, _force: bool) -> bool {
        let Some(p) = self.paths.get(&unit).cloned() else {
            return false;
        };
        self.stamp(p.cell(), p.pattern, p.foot_mask, false);
        true
    }
    fn add_footprint(&mut self, unit: UnitId) {
        if let Some(p) = self.paths.get(&unit).cloned() {
            self.stamp(p.cell(), p.pattern, p.foot_mask, true);
        }
    }
    fn room_list_remove(&mut self, unit: UnitId, room: RoomId) {
        self.events.push(Ev::ListRemove(unit, room));
    }
    fn room_list_insert(&mut self, unit: UnitId, room: RoomId) {
        self.events.push(Ev::ListInsert(unit, room));
    }
    fn queue_for_update(&mut self, unit: UnitId) {
        self.events.push(Ev::Queue(unit));
    }
    fn room_clients(&self, room: RoomId) -> Vec<ClientId> {
        self.room(room)
            .map(|r| r.clients.clone())
            .unwrap_or_default()
    }
}

impl WalkUnits for World {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        self.unit(unit).ty
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.unit(unit).guid
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.unit(unit).mode
    }
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.units
            .iter()
            .find(|(_, u)| u.ty == ty && u.guid == guid)
            .map(|(&id, _)| id)
    }
    /// `path-placement.md` §2.1: a path record's sub-tile, else the
    /// unit's own position.
    fn position(&self, unit: UnitId) -> Point {
        match self.paths.get(&unit) {
            Some(p) => p.cell(),
            None => self.unit(unit).pos,
        }
    }
    fn unit_size(&self, unit: UnitId) -> i32 {
        self.unit(unit).size
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        let u = self.unit(unit);
        let base = u.stats.get(&stat).copied().unwrap_or(0);
        if stat == 67 {
            base + u.run_bonus
        } else {
            base
        }
    }
    fn add_base_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        *self.unit_mut(unit).stats.entry(stat).or_insert(0) += delta;
    }
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.unit_mut(unit).stats.insert(stat, value);
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        &mut self.unit_mut(unit).seed
    }
    fn set_mode(&mut self, unit: UnitId, mode: u32) {
        let u = self.unit_mut(unit);
        u.mode = mode;
        u.run_bonus = 0;
    }
    fn attach_run_stats(&mut self, unit: UnitId, value: i32) {
        self.unit_mut(unit).run_bonus = value;
    }
    fn charstats_velocity(&self, _unit: UnitId) -> (i32, i32, i32) {
        // Fixed WalkVelocity, RunVelocity and RunDrain inputs for the fake.
        (6, 9, 20)
    }
    fn monstats_velocity(&self, _unit: UnitId) -> (i32, bool) {
        (6, false)
    }
    fn set_unit_flag(&mut self, unit: UnitId, bit: u32) {
        self.events.push(Ev::UnitFlag(unit, bit));
    }
    fn clear_ai_room_memo(&mut self, unit: UnitId) {
        self.events.push(Ev::AiMemo(unit));
    }
    fn client_player(&self, client: ClientId) -> Option<UnitId> {
        self.client_players.get(&client).copied()
    }
    fn send_unit_removal(&mut self, client: ClientId, unit: UnitId) {
        self.events.push(Ev::Removal(client, unit));
    }
    fn send_unit_add(&mut self, client: ClientId, unit: UnitId) {
        self.events.push(Ev::Add(client, unit));
    }
}

/// Set position and reset through the walk code (`pathing.md` §9.6 rule
/// 8, §9.7), as the wiring provides them.
impl PathMotion for World {
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, hint: Option<RoomId>) {
        let unit = path.owner.unwrap();
        Walk {
            t: tables(),
            c: self,
        }
        .set_position(unit, path, (to_fp16_center(x), to_fp16_center(y)), hint);
    }
    fn reset(&mut self, path: &mut DynamicPath) {
        let unit = path.owner.unwrap();
        Walk {
            t: tables(),
            c: self,
        }
        .reset(unit, path);
    }
}

// ---- reference models shared by the properties --------------------------

/// Room-change messages (`pathing.md` §9.8) as sets: clients only in the
/// old array (not the unit's own) get a removal, clients only in the new
/// array get the add messages, in ascending client order.
pub fn ref_messages(
    old: &[ClientId],
    new: &[ClientId],
    unit: UnitId,
    players: &BTreeMap<ClientId, UnitId>,
) -> Vec<Ev> {
    let mut all: Vec<ClientId> = old.iter().chain(new.iter()).copied().collect();
    all.sort();
    all.dedup();
    let mut out = Vec::new();
    for c in all {
        if players.get(&c) == Some(&unit) {
            continue;
        }
        match (old.contains(&c), new.contains(&c)) {
            (true, false) => out.push(Ev::Removal(c, unit)),
            (false, true) => out.push(Ev::Add(c, unit)),
            _ => {}
        }
    }
    out
}

/// Set position (`pathing.md` §9.6 rules 8–9) on a model record; the
/// seam calls go to `ev`.
pub fn ref_set_position(
    w: &World,
    path: &mut DynamicPath,
    unit: UnitId,
    q: (u32, u32),
    hint: Option<RoomId>,
    missile: bool,
    ev: &mut Vec<Ev>,
) {
    let cell = Point::new((q.0 >> 16) as i32, (q.1 >> 16) as i32);
    if missile && w.lookup(path.room, cell).is_none() {
        path.point_count = 0;
        return;
    }
    path.precise_x = q.0;
    path.precise_y = q.1;
    path.update_client();
    if path.flags & 0x1 == 0 {
        return;
    }
    if path.room.is_some_and(|r| w.contains(r, cell)) {
        return;
    }
    let new = w.lookup(path.room, cell).or_else(|| w.lookup(hint, cell));
    if new.is_none() && missile {
        path.point_count = 0;
        return;
    }
    path.prev_room = path.room;
    if let Some(old) = path.room {
        ev.push(Ev::ListRemove(unit, old));
    }
    path.flags |= 0x2;
    path.room = new;
    if let Some(n) = new {
        ev.push(Ev::ListInsert(unit, n));
        ev.push(Ev::Queue(unit));
    }
}

/// Reference unit distance using the `dist8_unit` table for nearby points
/// and the weighted distance formula for farther points.
pub fn ref_unit_distance(a: Point, sa: i32, b: Point, sb: i32) -> i32 {
    let dx = (a.x - b.x).abs();
    let dy = (a.y - b.y).abs();
    if dx < 8 && dy < 8 && sa < 4 && sb < 4 {
        let mut d = tables().dist8_unit[(dx + 8 * dy) as usize];
        // §9.5: a negative entry returns 0 at once (no size adjustment).
        if d < 0 {
            return 0;
        }
        if sa == 3 || sb == 3 {
            d = (d - 1).max(0);
        }
        if sa < 2 || sb < 2 {
            d + 1
        } else {
            d
        }
    } else {
        let ex = (dx - (sa / 2 + sb / 2)).max(0);
        let ey = (dy - (sa / 2 + sb / 2)).max(0);
        2 * ex.max(ey) + ex.min(ey)
    }
}

/// Sorted, distinct client arrays per room (`drlg/rooms.md` §7).
pub fn clients(max_rooms: usize) -> impl Strategy<Value = Vec<Vec<ClientId>>> {
    proptest::collection::vec(
        proptest::collection::btree_set(0u32..5, 0..4)
            .prop_map(|s| s.into_iter().map(ClientId).collect::<Vec<_>>()),
        max_rooms,
    )
}
