//! The local player's own path over the client DRLG: the walk
//! prediction's path step.
//!
//! The base-game client moves its own player between server messages along
//! its own path, stepped every client update (step 3). The path code is
//! the server's (the request, the compute, the per-tick step) over
//! the client's rooms, so the client's walk stops at the same wall and
//! goes round the same obstacle as the server's. This module runs that
//! code, `d2-sim`'s, over the client DRLG ([`ClientDrlg`]'s active rooms
//! and collision grids):
//!
//! - [`ClientPath::place`] allocates the player's path at a sub-tile in
//!   its client room (kind player);
//! - [`ClientPath::request`] is the player mode request (walk 2 / run 3
//!   to a point or a unit);
//! - [`ClientPath::tick`] is the player event 0 step of one tick.
//!
//! The player's footprint is stamped on a private copy of the grids it
//! touches (the model's DRLG is not written); so are the living
//! monsters' footprints (a client monster gets
//! footprint mask 0x100), at their model positions, before each request
//! and step ([`ClientPath::stamp_others`]).
//!
//! D2RUST's own, unverified. PROVISIONAL:
//! that the base-game client step runs these functions
//! with these inputs (the unit's stats read as: velocity percent and
//! stat 96 from the model ([`Own`]) plus the run bonus, stamina from the
//! model, no state, no used skill, no drain), and that a walking
//! monster's footprint stands at its model position (settled by
//! a capture of the base-game client grid under a walking NPC).
//!
//! Measured ((d), Rogue Encampment walk and run legs into the palisade):
//! the base-game client unit has its own path record (not the server's), and
//! its 16.16 position equals the server player's at every server tick and
//! every drawn frame (0 differences in 1,721 ticks / 1,923 frames; no lead,
//! no lag), including where a run stops at an obstacle on a sub-tile short
//! of the client path's own target. D2RUST allows 2 sub-tiles (`play_smoke`)
//! and drifts up to 6 (q-fix-real-client-path): q-fix-client-path-exact.

use std::collections::BTreeMap;

use d2_sim::drlg::{CollisionGrid, Drlg, TileRect};
use d2_sim::path::footprint::{add_footprint, remove_footprint, FootShape, Footprint, RemoveRule};
use d2_sim::path::record::{
    alloc_dynamic_path, pattern_of_size, DynamicKind, DynamicPath, MonsterShape, UnitShape,
};
use d2_sim::path::tables::PathTables;
use d2_sim::path::walk::request::mode;
use d2_sim::path::walk::velocity::{STAT_FASTERMOVE, STAT_VELOCITYPERCENT};
use d2_sim::path::walk::{request, PathWorld, Point, Step, Walk, WalkTarget, WalkUnits};
use d2_sim::path::CollisionRooms;
use d2_sim::rng::Seed;
use d2_sim::tick::TickRate;
use d2_sim::units::{ClientId, RoomId, UnitId, UnitType};

use super::predict::{MoveStats, Speeds};

/// The local player in the path context.
const ME: UnitId = UnitId(1);
/// The walk's target unit (a 0x02 / 0x04 walk).
const TARGET: UnitId = UnitId(2);
/// The stamina stat (8.8).
const STAT_STAMINA: u16 = 10;

/// The local player's own stats the path reads from the model: stamina
/// (stat 10) and the velocity stats ([`MoveStats`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Own {
    pub stamina: i32,
    pub moves: MoveStats,
}

/// What the walk goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathTo {
    Point(u16, u16),
    /// A unit of the model: its type, GUID and position.
    Unit(UnitType, u32, (u16, u16)),
}

/// The local player's client path (module doc).
///
/// A path that steps ([`ClientPath::tick`]) is made with
/// [`ClientPath::new`] at the game's tick rate; the default (base rate)
/// is for paths that only place, stamp or search and never step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientPath {
    /// The game's tick rate: each step moves this tick's share of the
    /// base-rate velocity, as the server's path does.
    rate: TickRate,
    path: Option<DynamicPath>,
    /// The grids the footprint wrote, copied from the client DRLG.
    grids: BTreeMap<RoomId, CollisionGrid>,
    mode: u32,
    run_bonus: i32,
    seed: Seed,
    frame: i32,
    /// The other units' footprints stamped on the private grids.
    others: Vec<Footprint>,
}

/// A monster of the model for [`ClientPath::stamp_others`]: its sub-tile
/// and the `monstats` / `monstats2` columns of its footprint pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtherUnit {
    pub x: u16,
    pub y: u16,
    pub size_x: i8,
    pub npc: bool,
    pub in_town: bool,
    pub interact: bool,
}

/// An object of the model whose footprint is on the client grid:
/// its sub-tile and footprint inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtherObject {
    pub x: u16,
    pub y: u16,
    pub shape: d2_sim::path::record::ObjectShape,
}

/// A client monster's footprint mask.
const MONSTER_FOOTPRINT: u16 = 0x100;

impl ClientPath {
    /// An empty path that steps at `rate` (the game's tick rate,
    /// [`super::world::ClientWorld::tick_rate`]).
    pub fn new(rate: TickRate) -> Self {
        ClientPath {
            rate,
            ..Default::default()
        }
    }

    /// Stamps `units` on the private grids in place of the last ones
    /// (pattern).
    pub fn stamp_others(
        &mut self,
        t: &PathTables,
        drlg: &Drlg,
        units: &[OtherUnit],
        objects: &[OtherObject],
    ) {
        let mut rooms = Rooms {
            drlg,
            grids: &mut self.grids,
        };
        for fp in self.others.drain(..) {
            remove_footprint(&mut rooms, &fp, RemoveRule::Other, true);
        }
        for u in units {
            let (x, y) = (i32::from(u.x), i32::from(u.y));
            let Some(room) = room_at(drlg, x, y) else {
                continue;
            };
            let shape = UnitShape::Monster(MonsterShape {
                size_x: i32::from(u.size_x),
                npc: u.npc,
                in_town: u.in_town,
                interact: u.interact,
                ..MonsterShape::default()
            });
            let fp = Footprint {
                room: Some(room),
                x,
                y,
                shape: FootShape::Pattern(pattern_of_size(t, i32::from(u.size_x), &shape)),
                mask: MONSTER_FOOTPRINT,
            };
            add_footprint(&mut rooms, &fp);
            self.others.push(fp);
        }
        // The 0x51 object init stamps the footprint with the
        // server's stamp (box `SizeX` × `SizeY`, the placement mask)
        // when `HasCollision[mode]` ≠ 0; frees it. The
        // callers pass the objects holding it (`world_view::walk::other_objects`).
        for o in objects {
            let (x, y) = (i32::from(o.x), i32::from(o.y));
            let Some(room) = room_at(drlg, x, y) else {
                continue;
            };
            let fp = Footprint {
                room: Some(room),
                x,
                y,
                shape: FootShape::Box {
                    size_x: o.shape.size_x,
                    size_y: o.shape.size_y,
                },
                mask: o.shape.foot_mask(),
            };
            add_footprint(&mut rooms, &fp);
            self.others.push(fp);
        }
    }

    /// Stamps the local player's own footprint at sub-tile (x, y) on the
    /// private grids with the others (size 2,
    /// pattern 1, mask 0x80, the NO_PATH marker 0x1000 on its cell). For
    /// the `fp` dump, whose grid has no
    /// path of the player's own.
    pub fn stamp_local(&mut self, t: &PathTables, drlg: &Drlg, x: u16, y: u16) {
        let (x, y) = (i32::from(x), i32::from(y));
        let Some(room) = room_at(drlg, x, y) else {
            return;
        };
        let mut rooms = Rooms {
            drlg,
            grids: &mut self.grids,
        };
        let fp = Footprint {
            room: Some(room),
            x,
            y,
            shape: FootShape::Pattern(pattern_of_size(t, 2, &UnitShape::Player)),
            mask: 0x80,
        };
        add_footprint(&mut rooms, &fp);
        self.others.push(fp);
    }

    /// The masks of the (2r+1)² sub-tiles around (x, y), row by row, as the
    /// path sees them (private grids in front of the DRLG's), read from the
    /// room holding (x, y) only: -1 for a cell outside it, and every cell
    /// -1 when no active room holds (x, y) (the state snapshot's key
    /// `fp`).
    pub fn window(&self, drlg: &Drlg, x: i32, y: i32, r: i32) -> Vec<i32> {
        let room = room_at(drlg, x, y);
        let grid = room.and_then(|id| {
            self.grids.get(&id).or_else(|| {
                drlg.active_room(drlg.drlg_room_of(id)?)
                    .map(|a| &a.collision)
            })
        });
        let mut out = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
        for cy in y - r..=y + r {
            for cx in x - r..=x + r {
                out.push(grid.and_then(|g| g.get(cx, cy)).map_or(-1, i32::from));
            }
        }
        out
    }

    /// The path's precise (16.16) position; `None` without a path.
    pub fn position(&self) -> Option<(u32, u32)> {
        self.path.as_ref().map(|p| (p.precise_x, p.precise_y))
    }

    /// The path's new direction (+0x65), re-aimed at its current point by
    /// each step; `None` without a path.
    pub fn new_direction(&self) -> Option<u8> {
        self.path.as_ref().map(|p| p.new_direction)
    }

    /// The path's last point; `None`
    /// without a path or with no points.
    pub fn path_end(&self) -> Option<(i32, i32)> {
        let p = self.path.as_ref()?;
        p.live_points().last().map(|q| (q.x, q.y))
    }

    /// The player's mode (2 walk, 3 run, 6 town walk while moving).
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// A path whose mode request gave `mode` (tests of the drawn mode).
    #[cfg(test)]
    pub(crate) fn with_mode(mode: u32) -> Self {
        ClientPath {
            mode,
            ..Default::default()
        }
    }

    /// Allocates the player's path at sub-tile (x, y) in the active room
    /// of `drlg` that contains it (kind player;
    /// footprint stamped on the private grids). `false`: no active room
    /// holds the point (no path).
    pub fn place(&mut self, t: &PathTables, drlg: &Drlg, x: u16, y: u16) -> bool {
        self.grids.clear();
        self.others.clear();
        self.path = None;
        self.mode = mode::NEUTRAL;
        let (x, y) = (i32::from(x), i32::from(y));
        let Some(room) = room_at(drlg, x, y) else {
            return false;
        };
        let mut rooms = Rooms {
            drlg,
            grids: &mut self.grids,
        };
        self.path = alloc_dynamic_path(
            t,
            &mut rooms,
            DynamicKind::Player,
            ME,
            Some(room),
            x,
            y,
            false,
        )
        .ok();
        self.path.is_some()
    }

    /// The player mode request of a walk (`run`: mode 3, else 2)
    /// to `to`. `true`: the path moves (a point count above 0).
    pub fn request(
        &mut self,
        t: &PathTables,
        drlg: &Drlg,
        speeds: Speeds,
        own: Own,
        to: PathTo,
        run: bool,
    ) -> bool {
        let m = if run { mode::RUN } else { mode::WALK };
        let target = match to {
            PathTo::Point(x, y) => WalkTarget::Point(Point::new(i32::from(x), i32::from(y))),
            PathTo::Unit(ty, guid, _) => WalkTarget::Unit { ty, guid },
        };
        let mut c = self.ctx(drlg, speeds, own, to);
        matches!(
            request(t, &mut c, ME, None, m, target, false),
            Ok(d2_sim::path::walk::Outcome::Moving(_))
        )
    }

    pub fn set_frame(&mut self, frame: i32) {
        self.frame = frame.wrapping_sub(1);
    }

    /// One tick of the player event 0 step. `true`: still moving.
    pub fn tick(
        &mut self,
        t: &PathTables,
        drlg: &Drlg,
        speeds: Speeds,
        own: Own,
        to: Option<PathTo>,
    ) -> bool {
        if !matches!(self.mode, mode::WALK | mode::RUN | mode::TOWN_WALK) {
            return false;
        }
        self.frame = self.frame.wrapping_add(1);
        let to = to.unwrap_or(PathTo::Point(0, 0));
        let mut c = self.ctx(drlg, speeds, own, to);
        let mut w = Walk { t, c: &mut c };
        matches!(w.player_event0(ME), Ok(Step::Moving))
    }

    fn ctx<'a>(&'a mut self, drlg: &'a Drlg, speeds: Speeds, own: Own, to: PathTo) -> Ctx<'a> {
        Ctx {
            rooms: Rooms {
                drlg,
                grids: &mut self.grids,
            },
            rate: self.rate,
            path: &mut self.path,
            mode: &mut self.mode,
            run_bonus: &mut self.run_bonus,
            seed: &mut self.seed,
            frame: self.frame,
            speeds,
            own,
            to,
        }
    }
}

/// The collision mask of the client's push-out.
pub const PUSH_OUT_MASK: u16 = 0xC01;

/// The point the base-game client puts a standing player at when its
/// footprint pattern collides with the client grid
/// (called at the end of
/// the client player update): with the unit in mode 1 (NU)
/// or 5 (TN), the pattern query of its sub-tile (x, y), the
/// unit's pattern, mask 0xC01, over the client grid of the active room
/// holding it; a non-zero value moves the unit to the free point
/// (its size, mask 0xC01, fallback on) through the client
/// teleport. The grid is the DRLG's with the footprints of
/// `objects`; a monster's footprint (0x100) is
/// outside the mask. `None`: no collision, or no room holds (x, y).
pub fn push_out(
    t: &PathTables,
    drlg: &Drlg,
    objects: &[OtherObject],
    (x, y): (u16, u16),
) -> Option<(i32, i32)> {
    use d2_sim::path::collision::pattern_value;
    use d2_sim::path::search::free_point;
    let (x, y) = (i32::from(x), i32::from(y));
    let room = room_at(drlg, x, y)?;
    let mut path = ClientPath::default();
    path.stamp_others(t, drlg, &[], objects);
    let rooms = Rooms {
        drlg,
        grids: &mut path.grids,
    };
    let size = UnitShape::Player.size();
    let pattern = pattern_of_size(t, size, &UnitShape::Player);
    if pattern_value(&rooms, Some(room), x, y, pattern, PUSH_OUT_MASK) == 0 {
        return None;
    }
    let view = d2_sim::wiring::path::place::Rooms(&rooms);
    let mut pt = Point::new(x, y);
    free_point(
        &view,
        Some(room),
        &mut pt,
        size,
        u32::from(PUSH_OUT_MASK),
        true,
    )
    .ok()?;
    Some((pt.x, pt.y))
}

/// The active room of `drlg` whose sub-tile rect holds (x, y).
fn room_at(drlg: &Drlg, x: i32, y: i32) -> Option<RoomId> {
    drlg.active_rooms()
        .into_iter()
        .find(|&(_, r)| {
            drlg.active_room(r)
                .is_some_and(|a| a.subtiles.contains(x, y))
        })
        .map(|(id, _)| id)
}

/// [`CollisionRooms`] over the client DRLG, with the private grids in
/// front of the DRLG's.
struct Rooms<'a> {
    drlg: &'a Drlg,
    grids: &'a mut BTreeMap<RoomId, CollisionGrid>,
}

impl Rooms<'_> {
    fn active(&self, room: RoomId) -> Option<&d2_sim::drlg::ActiveRoom> {
        self.drlg.active_room(self.drlg.drlg_room_of(room)?)
    }
}

impl CollisionRooms for Rooms<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.active(room).map(|a| a.subtiles)
    }

    fn adjacent_count(&self, room: RoomId) -> usize {
        self.active(room).map_or(0, |a| a.adjacency.len())
    }

    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        let n = *self.active(room)?.adjacency.get(i)?;
        self.drlg.active_room(n).map(|a| a.id)
    }

    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        match self.grids.get(&room) {
            Some(g) => Some(g),
            None => self.active(room).map(|a| &a.collision),
        }
    }

    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        if !self.grids.contains_key(&room) {
            let g = self.active(room)?.collision.clone();
            self.grids.insert(room, g);
        }
        self.grids.get_mut(&room)
    }
}

/// The walk context of the client path.
struct Ctx<'a> {
    rooms: Rooms<'a>,
    rate: TickRate,
    path: &'a mut Option<DynamicPath>,
    mode: &'a mut u32,
    run_bonus: &'a mut i32,
    seed: &'a mut Seed,
    frame: i32,
    speeds: Speeds,
    own: Own,
    to: PathTo,
}

impl CollisionRooms for Ctx<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.rooms.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.rooms.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.rooms.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.rooms.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.rooms.grid_mut(room)
    }
}

impl PathWorld for Ctx<'_> {
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath> {
        (unit == ME).then(|| self.path.clone()).flatten()
    }
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath) {
        if unit == ME {
            *self.path = Some(path.clone());
        }
    }
    fn room_in_town(&self, room: RoomId) -> bool {
        let Some(r) = self.rooms.drlg.drlg_room_of(room) else {
            return false;
        };
        let d = self.rooms.drlg;
        d2_sim::drlg::is_town(d.level(d.room(r).level).id)
    }
    fn remove_footprint(&mut self, unit: UnitId, force: bool) -> bool {
        let Some(fp) = self.path.as_ref().filter(|_| unit == ME).map(footprint_of) else {
            return false;
        };
        d2_sim::path::footprint::remove_footprint(
            &mut self.rooms,
            &fp,
            d2_sim::path::RemoveRule::Other,
            force,
        )
    }
    fn add_footprint(&mut self, unit: UnitId) {
        if let Some(fp) = self.path.as_ref().filter(|_| unit == ME).map(footprint_of) {
            d2_sim::path::footprint::add_footprint(&mut self.rooms, &fp);
        }
    }
    fn room_list_remove(&mut self, _: UnitId, _: RoomId) {}
    fn room_list_insert(&mut self, _: UnitId, _: RoomId) {}
    fn queue_for_update(&mut self, _: UnitId) {}
    fn room_clients(&self, _: RoomId) -> Vec<ClientId> {
        Vec::new()
    }
}

fn footprint_of(p: &DynamicPath) -> d2_sim::path::Footprint {
    d2_sim::path::Footprint {
        room: p.room,
        x: p.x(),
        y: p.y(),
        shape: d2_sim::path::FootShape::Pattern(p.pattern),
        mask: p.foot_mask,
    }
}

impl WalkUnits for Ctx<'_> {
    /// The client's path steps once per server tick, at the game's rate.
    fn tick_rate(&self) -> TickRate {
        self.rate
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        match (unit, self.to) {
            (TARGET, PathTo::Unit(ty, ..)) => ty,
            _ => UnitType::Player,
        }
    }
    fn guid(&self, unit: UnitId) -> u32 {
        match (unit, self.to) {
            (TARGET, PathTo::Unit(_, guid, _)) => guid,
            _ => unit.0,
        }
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn mode(&self, unit: UnitId) -> u32 {
        if unit == ME {
            *self.mode
        } else {
            mode::NEUTRAL
        }
    }
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        match self.to {
            PathTo::Unit(t, g, _) if t == ty && g == guid => Some(TARGET),
            _ => None,
        }
    }
    fn position(&self, unit: UnitId) -> Point {
        match (unit, self.to) {
            (ME, _) => self.path.as_ref().map(|p| p.cell()).unwrap_or_default(),
            (TARGET, PathTo::Unit(_, _, (x, y))) => Point::new(i32::from(x), i32::from(y)),
            _ => Point::default(),
        }
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        match (unit, stat) {
            (ME, STAT_VELOCITYPERCENT) => {
                self.own.moves.percent
                    + if *self.mode == mode::RUN {
                        *self.run_bonus
                    } else {
                        0
                    }
            }
            (ME, STAT_STAMINA) => self.own.stamina,
            _ => 0,
        }
    }
    fn item_stat(&self, unit: UnitId, stat: u16) -> i32 {
        match (unit, stat) {
            (ME, STAT_FASTERMOVE) => self.own.moves.faster,
            _ => 0,
        }
    }
    fn seed(&mut self, _: UnitId) -> &mut Seed {
        self.seed
    }
    fn set_mode(&mut self, unit: UnitId, m: u32) {
        if unit == ME {
            *self.mode = m;
        }
    }
    fn attach_run_stats(&mut self, _: UnitId, value: i32) {
        *self.run_bonus = value;
    }
    fn charstats_velocity(&self, _: UnitId) -> (i32, i32, i32) {
        (i32::from(self.speeds.walk), i32::from(self.speeds.run), 0)
    }
}
