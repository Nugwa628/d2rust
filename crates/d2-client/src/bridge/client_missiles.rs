//! Client missile creation: the client create fills a
//! client-only type-3 unit in set C from a
//! 0x5C-byte create record ([`CreateRecord`]) and its `missiles` row
//! ([`ClientMissileRow`]), and gives it its light. The missile's
//! own fields (frames, animation, velocity, target, skill, owner, pierce
//! count) are kept in [`ClientMissile`] beside the set-C unit.
//!
//! Each client update runs [`update`] on every set-C missile: the
//! light flicker, the row's client function (here and in `bodies`), the
//! end ([`end`]) and the default removal.
//!
//! The path is the straight missile path:
//! built at the create, stepped by the default step through the
//! path movement and cell walk over the client DRLG (the missile move),
//! which ends the missile on a missile wall.
//!
//! The motion record is made by the
//! create, updated before each dispatch and ends a landed
//! arc. The town tests read the active room of
//! the missile's sub-tile.
//!
//! Unit hits search the saved steps of the walk with the
//! collide tests and end the missile on the unit;
//! the client hit functions are in `hits`.
//!
//! Sounds are [`MissileSound`] calls handed to
//! the audio layer; the owner's umod callbacks run at the create (,
//! umod 29's hook).
//!
//! Not modelled (each named where it would run): the init callbacks of
//! the callers outside the model (function 8's is run), the
//! client event hooks, the second pass, the hit
//! functions not in `hits` (PROVISIONAL for a few bodies, see
//! `hits::call`), and the client functions not listed in [`update_with`]'s
//! dispatch (the missile is then left as it is). The aim nudge
//! reads the owner's direction from the record
//! ([`CreateRecord::owner_dir64`]; none given is a handler error).

pub use crate::app::single_player::rows::ClientMissileRow;

use std::collections::{BTreeMap, BTreeSet};

use d2_sim::rng::Seed;

use super::dispatch::HandlerError;
use super::world::{ClientWorld, UnitKey, MISSILE, MONSTER, PLAYER};
use crate::rules::lighting::records::{unit_light_pos, LightKind, Owner};
use crate::rules::unit_composite::MotionRecord;
use d2_sim::path::collision::{find_room, size_value};
use d2_sim::path::footprint::missile_move;
use d2_sim::path::CollisionRooms;
use d2_sim::units::RoomId;

#[path = "client_missiles_bodies.rs"]
mod bodies;
#[path = "client_missiles_hits.rs"]
mod hits;
pub(crate) use hits::{
    count as search_count, in_melee_range, melee_reach, pick as search_pick, ring,
};

/// The create record (0x5C bytes) as the client
/// create reads it (last paragraph: flags 1, 2, 4, 8, 0x10, 0x20,
/// 0x100, 0x200, 0x400, 0x800, 0x2000, 0x4000, 0x8000).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreateRecord {
    pub flags: u32,
    /// +0x04 owner, +0x08 origin, +0x0C target unit.
    pub owner: Option<UnitKey>,
    pub origin: Option<UnitKey>,
    pub target: Option<UnitKey>,
    /// +0x10.
    pub class: u32,
    /// +0x14, +0x18 (flag 1).
    pub x: i32,
    pub y: i32,
    /// +0x1C, +0x20 (flag 2 relative, flag 0x20 absolute).
    pub tx: i32,
    pub ty: i32,
    /// +0x28 velocity (flag 4).
    pub velocity: i32,
    /// +0x2C skill, +0x30 level.
    pub skill: i32,
    pub level: i32,
    /// +0x34 loops (flag 8).
    pub loops: i32,
    /// +0x40 start frame (flag 0x200), +0x44 activate frames (flag
    /// 0x800), +0x4C range (flag 0x8000).
    pub start_frame: i32,
    pub activate: i32,
    pub range: i32,
    /// +0x50 light byte.
    pub light: u8,
    /// The owner's direction (0…63) for the aim nudge,
    /// as the caller reads it from the unit's drawn pose
    /// (`world_view::UnitPose::dir64`); the model holds no client path.
    pub owner_dir64: Option<u8>,
    /// +0x24 arc height (flag 0x100).
    pub arc_height: i32,
}

/// Record flags.
pub mod flag {
    pub const POSITION: u32 = 0x1;
    pub const TARGET_RELATIVE: u32 = 0x2;
    pub const VELOCITY: u32 = 0x4;
    pub const LOOPS: u32 = 0x8;
    pub const VELOCITY_FIXED: u32 = 0x10;
    pub const TARGET_ABSOLUTE: u32 = 0x20;
    pub const ARC: u32 = 0x100;
    pub const START_FRAME: u32 = 0x200;
    pub const FRAMES_FROM_DISTANCE: u32 = 0x400;
    pub const ACTIVATE: u32 = 0x800;
    pub const RANDOM_DIRECTION: u32 = 0x2000;
    pub const NO_LIGHT: u32 = 0x4000;
    pub const RANGE: u32 = 0x8000;
}

/// A client missile's own fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientMissile {
    /// Missile data: activate frame (+0x08), skill (+0x0A), level
    /// (+0x0C), total (+0x0E), current = frames left (+0x10).
    pub activate: i32,
    pub skill: i32,
    pub level: i32,
    pub total: i32,
    pub current: i32,
    /// Animation frame, length, speed (+0x44, +0x48, +0x4C; 8.8).
    pub frame: i32,
    pub anim_len: i32,
    pub anim_speed: i32,
    /// Path: velocity (8.8 sub-tiles), target point, target unit,
    /// direction (0…63), acceleration, max velocity.
    pub velocity: i32,
    pub target_point: (i32, i32),
    pub target_unit: Option<UnitKey>,
    pub direction: u8,
    pub accel: i16,
    pub max_vel: i32,
    /// The last-collided unit (with `LastCollide`).
    pub last_collided: Option<UnitKey>,
    /// Owner (+0x94 / +0x98, flag-ex 0x400).
    pub owner: Option<UnitKey>,
    /// Stat 328, the pierce count.
    pub pierce: u32,
    /// Unit flag 0x10000 (set by functions 2 and 11).
    pub flat: bool,
    /// The path's precise position (16.16 sub-tiles), direction vector
    /// (path +0x6A, +0x6E; 0 when the path has no point) and
    /// acceleration counter (+0x8C).
    pub pos: (u32, u32),
    /// The point the path's client pixel pair (+0x08 / +0x0C, which the
    /// draw reads) was last computed from, while it differs from `pos`:
    /// the path init and set-position compute
    /// the pair, the raw setters (functions 6
    /// and 18) write +0x00 / +0x04 only. `None`: the
    /// pair follows `pos`.
    pub drawn_at: Option<(u32, u32)>,
    pub dir_vec: (i32, i32),
    pub accel_counter: i32,
    /// Missile data +0x28 / +0x2C: set
    /// by the creator unless a body writes them.
    pub d28: i32,
    pub d2c: i32,
    /// Missile data +0x04 / +0x06 (i16):
    /// written only by client skill functions, 0 otherwise.
    pub d04: i16,
    pub d06: i16,
    /// The path new-step flag (path +0x34 bit 3):
    /// the last path step entered a new sub-tile.
    pub new_step: bool,
    /// Path flag 0x40 (path +0x34, set by the path re-aim setter): a
    /// missile with it keeps its facing when the path re-aims
    /// (does nothing).
    pub keep_facing: bool,
    /// The client motion record (gfx +0x30):
    /// created and restarted by the create, a timed arc
    /// with flag 0x100, updated before each dispatch.
    pub motion: MotionRecord,
    /// The path's room (path +0x1C), its
    /// collided mask (+0x54, the OR of the last step's missile moves),
    /// the saved steps of the last cell walk (+0x1D4 / +0x1D8, at most 10)
    /// and a point count of 0 (`stopped`: the step entered no room).
    pub room: Option<RoomId>,
    pub collided: u16,
    pub steps: [(i32, i32); 10],
    pub step_count: u8,
    pub stopped: bool,
    /// The room-exit flag (path +0x34 bit 0x1):
    /// a type-4 compute aimed outside the path room, or with no
    /// path room. Never cleared. Only with it does set position recache
    /// the room, so a missile without it keeps the room (and
    /// the room list) it was created in.
    pub room_exit: bool,
    /// The target of the last type-4 compute whose room-exit test
    /// is still to run against the path room: the computes
    /// that re-aim a flying missile have no DRLG at hand, so the test
    /// runs at the next path step, before the room can change.
    pub exit_test: Option<(i32, i32)>,
    /// A point path: charged bolt (path type 10, [`bolt_init`]) or
    /// blessed hammer (type 14, [`hammer_init`]); none for the straight
    /// type 4.
    pub bolt: Option<BoltPath>,
}

/// The point slots a charged-bolt path uses: at most 77 >> 1 = 38
/// jitter points and the end point.
pub const BOLT_POINTS: usize = 39;

/// The point slots of a point path: the 77 of the blessed-hammer spiral,
/// more than a bolt's [`BOLT_POINTS`].
pub const POINT_SLOTS: usize = 77;

/// The points, count and index of a path of type 10 or 14 (path +0x9C,
/// +0x28, +0x24), cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoltPath {
    pub points: [(i32, i32); POINT_SLOTS],
    pub count: u8,
    pub index: u8,
}

/// m's sub-tile (the path position, (x, y)).
fn cell_of(m: &ClientMissile) -> (i32, i32) {
    ((m.pos.0 >> 16) as i32, (m.pos.1 >> 16) as i32)
}

/// Re-path with (path, m, 0) toward m's target point (a target
/// unit is dropped by the point setter): the straight path
/// path; no point → the direction vector is 0 (no step).
fn repath(m: &mut ClientMissile) {
    repath_to(m, m.target_point);
}

/// Re-path toward `t` (the target unit's position or the target point).
fn repath_to(m: &mut ClientMissile, t: (i32, i32)) {
    match straight_path(m.pos, t) {
        Some((vec, dir)) if t != cell_of(m) => {
            m.dir_vec = vec;
            face(m, dir);
            m.exit_test = Some(t);
        }
        Some(_) => {
            m.dir_vec = (0, 0);
            m.exit_test = Some(t);
        }
        None => m.dir_vec = (0, 0),
    }
}

/// The room-exit test of the type-4 compute: no path room, or the target outside
/// the room's sub-tile rect (half-open) → path flag 0x1; a target
/// inside leaves the flag as it is.
fn room_exit_test(m: &mut ClientMissile, rooms: &impl CollisionRooms, t: (i32, i32)) {
    let inside = m.room.is_some_and(|r| {
        rooms
            .subtile_rect(r)
            .is_some_and(|rect| rect.contains(t.0, t.1))
    });
    if !inside {
        m.room_exit = true;
    }
}

/// The path tables (the direction-vector `tan` table).
fn path_tables() -> Option<&'static d2_sim::path::tables::PathTables> {
    static T: std::sync::OnceLock<Option<d2_sim::path::tables::PathTables>> =
        std::sync::OnceLock::new();
    T.get_or_init(|| d2_sim::path::tables::PathTables::spec().ok())
        .as_ref()
}

/// The straight missile path compute (type 4)
/// from the precise start `pos` toward the target point
/// (cell centre): no point when the target is 100 or more away on an
/// axis or has a 0 coordinate; else the direction vector and direction
/// of the straight path.
fn straight_path(pos: (u32, u32), target: (i32, i32)) -> Option<((i32, i32), u8)> {
    let (sx, sy) = ((pos.0 >> 16) as i32, (pos.1 >> 16) as i32);
    if (target.0 - sx).abs() > 99 || (target.1 - sy).abs() > 99 || target.0 == 0 || target.1 == 0 {
        return None;
    }
    let centre = |c: i32| ((c as u32) << 16) | 0x8000;
    let t = path_tables()?;
    Some(d2_sim::path::walk::geom::direction_vector(
        t,
        pos,
        (centre(target.0), centre(target.1)),
    ))
}

/// The charged-bolt create callback with (m, i) (record +0x54):
/// total frames n ≥ 78 → total and
/// frames left := 77; m's seed := `init_low`(u16 path target x + i);
/// path type 10 (its flags equal type 4's);
/// max distance := n capped at 77; the compute
/// → type 10: n >> 1
/// draws on m's seed, the jitter points from m's cell. The index (0) and
/// the direction vector the create set stay.
pub fn bolt_init(w: &mut ClientWorld, key: UnitKey, i: i32) {
    bolt_path(w, key, i, false);
}

/// The DiabWall create callback with (m, i):
/// [`bolt_init`] with a gate after the seed's initialisation:
/// one step of m's seed, low % 100 < 20 → done (the create's path stays).
pub fn wall_init(w: &mut ClientWorld, key: UnitKey, i: i32) {
    bolt_path(w, key, i, true);
}

fn bolt_path(w: &mut ClientWorld, key: UnitKey, i: i32, gate: bool) {
    let rate = w.tick_rate;
    let Some(t) = path_tables() else {
        return;
    };
    let Some(m) = w.objclient.missiles.get_mut(&key) else {
        return;
    };
    let Some(unit) = w.objclient.set_c.get_mut(&key) else {
        return;
    };
    let mut nf = rate.base_ticks(m.total);
    if nf >= 78 {
        nf = 77;
        m.total = rate.ticks(77);
        m.current = m.total;
    }
    let tx = m.target_point.0 as u16;
    let mut seed = Seed::init_low(u32::from(tx).wrapping_add(i as u32));
    if gate && seed.step() % 100 < 20 {
        unit.seed = Some((seed.lo, seed.hi));
        return;
    }
    let max_distance = (nf as u8).min(77);
    let n = usize::from(max_distance >> 1);
    use d2_sim::path::walk::{geom, missile::BOLT_OFFSETS, seams::Point};
    let start = cell_of(m);
    let target = Point::new(m.target_point.0, m.target_point.1);
    let d0 = t.testdir[geom::octant(Point::new(start.0, start.1), target)][0];
    let mut bolt = BoltPath {
        points: [(0, 0); POINT_SLOTS],
        count: (n + 1) as u8,
        index: 0,
    };
    let mut cur = start;
    for p in bolt.points.iter_mut().take(n) {
        let lo = seed.step();
        let d = (BOLT_OFFSETS[(lo & 31) as usize] + d0) & 7;
        *p = cur;
        let s = geom::step(t, d as u8);
        cur = (cur.0 + 2 * s.x, cur.1 + 2 * s.y);
    }
    bolt.points[n] = cur;
    m.bolt = Some(bolt);
    unit.seed = Some((seed.lo, seed.hi));
}

/// Path type 14 and its compute after the blessed-hammer create:
/// the path type set to 14, then the path compute with (m, 0) → type 14:
/// the 77 spiral cells from m's precise position, count 77. No
/// draw; the index (0) and the direction vector the create set stay.
pub fn hammer_init(w: &mut ClientWorld, key: UnitKey) {
    let Some(m) = w.objclient.missiles.get_mut(&key) else {
        return;
    };
    let Ok(cells) = d2_sim::path::walk::missile::hammer_cells(m.pos) else {
        return;
    };
    let mut path = BoltPath {
        points: [(0, 0); POINT_SLOTS],
        count: POINT_SLOTS as u8,
        index: 0,
    };
    for (slot, &(x, y)) in path.points.iter_mut().zip(cells.iter()) {
        *slot = (x as i32, y as i32);
    }
    m.bolt = Some(path);
}

/// for a point path (type 10 or 14): blocked →
/// index := count (the movement ends); else a reached point → index += 1.
fn bolt_reached(m: &mut ClientMissile, reach: bool, blocked: bool) {
    if let Some(b) = m.bolt.as_mut() {
        if blocked {
            b.index = b.count;
        } else if reach {
            b.index += 1;
        }
    }
}

/// for a point path (type 10 or 14) after a step:
/// while m's precise position is the point's centre (exact)
/// the point is passed (the last one: direction vector and velocity 0,
/// index := count); then the direction vector toward the point's
/// centre and the facing (no path flag 0x40).
fn bolt_next_point(m: &mut ClientMissile) {
    let Some(mut b) = m.bolt else {
        return;
    };
    if b.index >= b.count {
        return;
    }
    let centre = |c: i32| ((c as u32) << 16) | 0x8000;
    let at = |b: &BoltPath| {
        let p = b.points[usize::from(b.index)];
        (centre(p.0), centre(p.1))
    };
    while at(&b) == m.pos {
        if b.index + 1 >= b.count {
            m.dir_vec = (0, 0);
            m.velocity = 0;
            b.index = b.count;
            m.bolt = Some(b);
            return;
        }
        b.index += 1;
    }
    m.bolt = Some(b);
    if let Some(t) = path_tables() {
        let (vec, dir) = d2_sim::path::walk::geom::direction_vector(t, m.pos, at(&b));
        m.dir_vec = vec;
        face(m, dir);
    }
}

/// for a missile: direction := d & 63, unless the
/// path has flag 0x40 (the facing stays).
fn face(m: &mut ClientMissile, dir: u8) {
    if !m.keep_facing {
        m.direction = dir & 63;
    }
}

/// A client missile's sound call, handed
/// to the audio layer as [`super::output::Output::MissileSound`] in
/// update order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissileSound {
    /// A request of `id` on the missile: `TravelSound` at
    /// the create, `HitSound` at the end.
    Request { id: i32, missile: UnitKey },
    /// The create's sound stop with (owner, 314): the owner's
    /// first request in sound 314's group is stopped.
    StopOwnerGroup { owner: UnitKey, id: i32 },
    /// The end's detach with (m, TravelSound): m's first
    /// request in the travel sound's group is detached from m.
    DetachTravel { missile: UnitKey, id: i32 },
    /// Function 9's detach with (m): m's first
    /// attached request (its list head) is detached with force.
    DetachHead { missile: UnitKey },
}

/// The sound 314 whose group the create stops on the owner.
const OWNER_STOP_SOUND: i32 = 314;

/// The client missiles of set C, by key.
pub type ClientMissiles = BTreeMap<UnitKey, ClientMissile>;

/// Flag-ex 0x40000: not drawn while in `InitSteps`.
pub const FLAG_EX_NOT_DRAWN: u32 = 0x40000;

/// The client create. `Ok(None)`: the create
/// returned none. `lights` is the light-quality flag ≠ 0
/// (high quality).
pub fn create(
    w: &mut ClientWorld,
    rows: &[ClientMissileRow],
    rec: &CreateRecord,
    lights: bool,
) -> Result<Option<UnitKey>, HandlerError> {
    let rate = w.tick_rate;
    let frame = w.share_frame();
    let Some(row) = rows.get(rec.class as usize).copied() else {
        return Ok(None);
    };
    let (x, y) = if rec.flags & flag::POSITION != 0 {
        (rec.x, rec.y)
    } else {
        let Some(o) = rec
            .origin
            .and_then(|k| w.units.get(&k).or_else(|| w.objclient.set_c.get(&k)))
        else {
            return Ok(None);
        };
        let (x, y) = o.cell();
        (i32::from(x), i32::from(y))
    };
    // The room containing (x, y) searched from the owner's room (no
    // owner: the local player's), else the client DRLG lookup.
    let from = match rec.owner {
        Some(o) => Some(o),
        None => w.local_player,
    };
    if let Some(from) = from {
        if w.active_rooms.is_some() {
            let start = w.unit_room(from).copied();
            let found = start
                .and_then(|r| w.cell_lookup(&r, x, y))
                .or_else(|| point_room(w, x, y));
            if found.is_none() {
                return Ok(None);
            }
        }
    }
    let (tx, ty) = if rec.flags & flag::TARGET_RELATIVE != 0 {
        (x + rec.tx, y + rec.ty)
    } else if rec.flags & flag::TARGET_ABSOLUTE != 0 {
        (rec.tx, rec.ty)
    } else {
        (x, y)
    };
    let mut v = if rec.flags & flag::VELOCITY == 0 {
        row.vel
            .wrapping_add(row.vel_lev.wrapping_mul(rec.level) / 8)
            .wrapping_shl(8)
    } else if rec.flags & flag::VELOCITY_FIXED != 0 {
        rec.velocity
    } else {
        rec.velocity.wrapping_shl(8)
    };
    let mut s = 100;
    if let Some(o) = rec.owner.and_then(|k| w.units.get(&k)) {
        if row.can_slow && o.states.contains(&87) {
            if let Some(list) = o.state_lists.get(&87) {
                s = list.get(&(161, 0)).copied().unwrap_or(0);
                v = d2_sim::combat::pct(v, s, 100);
            }
        }
    }
    if v != 0 {
        v = if v <= 0x100000 {
            (i64::from(v) * 75 / 100) as i32
        } else {
            (v / 100) * 75
        };
    }
    // The aim. Owner none: nothing.
    let mut target = rec.target;
    let (mut tx, mut ty) = (tx, ty);
    if v != 0 {
        if let Some(owner) = rec.owner {
            let o_cell = w.units.get(&owner).map(|u| u.cell());
            let nudge = match target {
                None => (tx, ty) == (x, y),
                Some(t) if t != owner => {
                    let t_cell = w.units.get(&t).map(|u| u.cell());
                    let same = t_cell.is_some() && t_cell == o_cell;
                    if same {
                        target = None;
                    }
                    same
                }
                Some(_) => false,
            };
            if nudge {
                // D := owner direction >> 3; (tx, ty) :=
                // owner position + (DX[d], DY[d]).
                const DX: [i32; 8] = [0, -1, -2, -1, 0, 1, 2, 1];
                const DY: [i32; 8] = [2, 1, 0, -1, -2, -1, 0, 1];
                let dir = rec.owner_dir64.ok_or(HandlerError::Invalid(
                    "missiles/client.md §C2 r8: the aim nudge needs the owner's direction",
                ))?;
                let d = usize::from((dir & 63) >> 3);
                let (ox, oy) = o_cell.unwrap_or_default();
                (tx, ty) = (i32::from(ox) + DX[d], i32::from(oy) + DY[d]);
            }
            let aim = match target.and_then(|t| w.units.get(&t)) {
                Some(t) => {
                    let (ax, ay) = t.cell();
                    (i32::from(ax), i32::from(ay))
                }
                None => (tx, ty),
            };
            if (aim.0 - x).abs() >= 100 || (aim.1 - y).abs() >= 100 {
                return Ok(None);
            }
        }
    }
    // Allocate in set C (GUID counter, client room seed step).
    let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
        return Ok(None);
    };
    let Some(key) = super::objects::create_client_unit(w, MISSILE, rec.class, ux, uy) else {
        return Ok(None);
    };
    let mut m = ClientMissile {
        anim_len: i32::from(row.anim_len) << 8,
        anim_speed: (i32::from(row.anim_speed) << 4).clamp(-0x8000, 0x7FFF),
        skill: rec.skill,
        level: rec.level,
        accel: row.accel,
        max_vel: row.max_vel << 8,
        ..ClientMissile::default()
    };
    let mut f = if rec.flags & flag::RANGE != 0 {
        rec.range
    } else {
        let mut f = i32::from(row.range) + i32::from(row.lev_range) * rec.level;
        if rec.flags & flag::LOOPS != 0 && row.sub_loop != 0 {
            f += (i32::from(row.sub_stop) - i32::from(row.sub_start)) * rec.loops;
        }
        f
    };
    if s != 0 && s != 100 {
        f += d2_sim::combat::pct(f, 100, s);
    }
    if rec.flags & flag::START_FRAME != 0 {
        f -= rec.start_frame;
        m.frame = rec.start_frame << 8;
    }
    let f = f.clamp(-0x8000, 0x7FFF);
    m.total = rate.ticks(f);
    m.current = m.total;
    m.activate = m.total
        - rate.ticks(if rec.flags & flag::ACTIVATE != 0 {
            rec.activate
        } else {
            row.activate
        });
    // Velocity, target; v ≠ 0 → the path built toward the target
    // (a target unit's position, else the point).
    m.target_unit = target;
    m.target_point = (tx, ty);
    m.velocity = v;
    m.pos = (((x as u32) << 16) | 0x8000, ((y as u32) << 16) | 0x8000);
    m.room = w.drlg.as_ref().and_then(|d| drlg_room_at(&d.drlg, x, y));
    if v != 0 {
        let aim = match target.and_then(|t| w.units.get(&t)) {
            Some(t) => {
                let (ax, ay) = t.cell();
                (i32::from(ax), i32::from(ay))
            }
            None => (tx, ty),
        };
        if aim == (x, y) {
            // The point is the position (and the
            // last): vectors and velocity 0.
            m.velocity = 0;
            m.exit_test = straight_path(m.pos, aim).map(|_| aim);
        } else if let Some((vec, dir)) = straight_path(m.pos, aim) {
            m.dir_vec = vec;
            // A missile without path flag 0x40 faces d.
            m.direction = dir & 63;
            m.exit_test = Some(aim);
        }
        // against the creation room.
        if let (Some(t), Some(d)) = (m.exit_test.take(), w.drlg.as_ref()) {
            let empty = Default::default();
            room_exit_test(&mut m, &MissileRooms(&d.drlg, &empty), t);
        }
    }
    if row.last_collide {
        m.last_collided = rec.owner;
    }
    let unit = w.objclient.set_c.get_mut(&key).expect("just created");
    // Type init: mode := `CollideType`.
    unit.mode = u32::from(row.collide_type);
    if row.init_steps != 0 {
        unit.flag_ex |= FLAG_EX_NOT_DRAWN;
    }
    // Direction := rnd(64) on the missile's seed.
    if rec.flags & flag::RANDOM_DIRECTION != 0 {
        if let Some((lo, hi)) = unit.seed {
            let mut seed = Seed::new(lo, hi);
            m.direction = seed.roll(64) as u8;
            unit.seed = Some((seed.lo, seed.hi));
        }
    }
    // Frames from the distance (max + ⌊min / 2⌋).
    if rec.flags & flag::FRAMES_FROM_DISTANCE != 0 {
        let (dx, dy) = ((tx - x).abs(), (ty - y).abs());
        let d = (dx.max(dy) + dx.min(dy) / 2).max(1);
        let vv = (v as u32).wrapping_shl(4);
        if let Some(n) = ((d as u32) << 16).checked_div(vv) {
            m.total = rate.ticks(n as i32);
            m.current = m.total;
        }
    }
    // The motion record created (zeroed) and restarted.
    m.motion.restart();
    // The timed arc (height +0x24, n = total frames), one motion
    // update, direction := (tx + ty) & 63 (path flag 0x40: the facing
    // stays).
    if rec.flags & flag::ARC != 0 {
        m.motion.timed_arc(rec.arc_height, rate.base_ticks(m.total));
        motion_update(&mut m.motion, rate, frame)?;
        m.direction = ((tx + ty) & 63) as u8;
    }
    if let Some(owner) = rec.owner {
        m.owner = Some(owner);
        unit.flag_ex |= 0x400;
        m.pierce = pierce_count(w, &row, owner);
    }
    w.objclient.missiles.insert(key, m);
    // The path init files the unit in its room's list.
    relink(w, key);
    // The light (the missile row of the light rules).
    missile_light(w, key, &row, rec, lights);
    // The init callback is the caller's (function 8's draws):
    // `TravelSound` ≠ 0 → a request on m; with an owner, its first
    // request in sound 314's group stops.
    if row.travel_sound != 0 {
        w.objclient.missile_sounds.push(MissileSound::Request {
            id: i32::from(row.travel_sound),
            missile: key,
        });
    }
    if let Some(owner) = rec.owner {
        w.objclient
            .missile_sounds
            .push(MissileSound::StopOwnerGroup {
                owner,
                id: OWNER_STOP_SOUND,
            });
    }
    // The owner's umod callbacks of phase 4.
    if let Some(owner) = rec.owner {
        umod_missile(w, rows, owner, key, lights)?;
    }
    Ok(Some(key))
}

/// Umod 29 `multishot`'s client hook.
const UMOD_MULTISHOT: u8 = 29;

/// The umod dispatcher's phase 4 with (owner, missile):
/// an owner of type 1 with
/// monster data runs, for each of its 9 umod bytes in order (bytes after
/// a 0 included), the byte's phase-4 hook; only umod 29 has one:
/// unique (type flag 8); m = the missile's class,
/// in the table and without `NoMultiShot`; O = its owner without the
/// guard flag (+0x16 0x80); target (tx, ty) = O's target unit's position,
/// else the missile's path target point; (sx, sy) = the signs of O − (tx,
/// ty) (0 for m in 63 … 66); with the guard set, two client missiles of
/// m (flags 0x20, owner and origin O) at (tx − sy, ty + sx)
/// and (tx + sy, ty − sx), the missile's skill and level.
///
/// PROVISIONAL: the model holds no monster target unit
/// (the base game reads the path's target unit, path +0x58), so the
/// path target point is used; settled by a capture of a multishot
/// monster's client state.
fn umod_missile(
    w: &mut ClientWorld,
    rows: &[ClientMissileRow],
    owner: UnitKey,
    key: UnitKey,
    lights: bool,
) -> Result<(), HandlerError> {
    if owner.unit_type != MONSTER {
        return Ok(());
    }
    let Some(data) = w.units.get(&owner).and_then(|u| match &u.kind {
        super::world::KindData::Monster(d) => Some((d.umods, d.flags)),
        _ => None,
    }) else {
        return Ok(());
    };
    let (umods, flags) = data;
    for umod in umods {
        if umod != UMOD_MULTISHOT || flags & 8 == 0 {
            continue;
        }
        let Some(class) = w.objclient.set_c.get(&key).map(|u| u.class) else {
            return Ok(());
        };
        if rows.get(class as usize).is_none_or(|r| r.no_multishot) {
            continue;
        }
        if w.objclient.multishot_guard.contains(&owner) {
            continue;
        }
        let Some(m) = w.objclient.missiles.get(&key).copied() else {
            return Ok(());
        };
        let Some((ox, oy)) = w.units.get(&owner).map(|u| u.cell()) else {
            continue;
        };
        let (tx, ty) = m.target_point;
        let (mut sx, mut sy) = ((i32::from(ox) - tx).signum(), (i32::from(oy) - ty).signum());
        if (63..=66).contains(&class) {
            (sx, sy) = (0, 0);
        }
        w.objclient.multishot_guard.insert(owner);
        for (cx, cy) in [(tx - sy, ty + sx), (tx + sy, ty - sx)] {
            let rec = CreateRecord {
                flags: flag::TARGET_ABSOLUTE,
                owner: Some(owner),
                origin: Some(owner),
                class,
                tx: cx,
                ty: cy,
                skill: m.skill,
                level: m.level,
                owner_dir64: Some(m.direction),
                ..CreateRecord::default()
            };
            let made = create(w, rows, &rec, lights);
            if made.is_err() {
                w.objclient.multishot_guard.remove(&owner);
            }
            made?;
        }
        w.objclient.multishot_guard.remove(&owner);
    }
    Ok(())
}

/// A create made by the client missile `parent` (its bodies):
/// [`create`] with the record's owner direction, when the record names
/// none, read as the parent missile's direction.
///
/// PROVISIONAL: the aim nudge reads the owner's
/// direction, which the model does not hold (unit
/// directions live in the view's poses); a child created by a missile
/// takes its parent's direction instead. Found on the user's rows (a
/// child with velocity aimed at its own start, `app_client_drlg`
/// `the_users_client_missiles_run_their_functions`). Oracle (data):
/// `missiles.txt` `Vel` of the children each caller creates; function
/// 27's two stock children (`mephistofirewall`, `andyfirewall`) have no
/// `Vel`, so the stock rows of that body never nudge; the other callers
/// of this function are settled when their children's `Vel` is checked
/// the same way.
fn create_from(
    w: &mut ClientWorld,
    env: &Env,
    parent: UnitKey,
    rec: &CreateRecord,
) -> Result<Option<UnitKey>, HandlerError> {
    let mut rec = *rec;
    if rec.owner_dir64.is_none() {
        rec.owner_dir64 = w.objclient.missiles.get(&parent).map(|m| m.direction);
    }
    create(w, env.rows, &rec, env.lights)
}

/// The room of a point through the client DRLG.
fn point_room(w: &ClientWorld, x: i32, y: i32) -> Option<super::world::ActiveRoom> {
    let rooms = w.active_rooms.as_deref()?;
    super::world::room_of_point(rooms, x, y).copied()
}

/// The pierce count of a missile owned by `o`.
fn pierce_count(w: &ClientWorld, row: &ClientMissileRow, o: UnitKey) -> u32 {
    if !row.pierce || !matches!(o.unit_type, PLAYER | MONSTER) {
        return 0;
    }
    let p = w.total(o, 166, 0).wrapping_add(w.total(o, 156, 0));
    if p == 0 {
        return 0;
    }
    let (p, c) = if o.unit_type == PLAYER && w.local_player != Some(o) {
        (100, 0)
    } else {
        (p, w.units.get(&o).map_or(0, |u| u.stat(328)))
    };
    let mut seed = Seed::init_low(c as u32);
    let mut n = 0;
    for _ in 0..4 {
        let lo = seed.step();
        if ((lo % 100) as i32) < p {
            n += 1;
        } else {
            break;
        }
    }
    n
}

/// Low quality or flag 0x4000 → the unit's light removed; else
/// `Light` ≠ 0 or record byte +0x50 ≠ 0 → removed and created
/// (missile light: kind 1, radius `Light`, the row's colour; radius 0
/// creates nothing).
fn missile_light(
    w: &mut ClientWorld,
    key: UnitKey,
    row: &ClientMissileRow,
    rec: &CreateRecord,
    lights: bool,
) {
    let owner = Owner {
        unit_type: u32::from(MISSILE),
        guid: key.guid,
        client_only: true,
    };
    let old = w
        .lights
        .iter()
        .find(|(_, r)| r.owner() == Some(owner))
        .map(|(id, _)| id);
    if !lights || rec.flags & flag::NO_LIGHT != 0 {
        if let Some(id) = old {
            let _ = w.lights.remove(id);
        }
        return;
    }
    if row.light == 0 && rec.light == 0 {
        return;
    }
    if let Some(id) = old {
        let _ = w.lights.remove(id);
    }
    let Some(u) = w.objclient.set_c.get(&key) else {
        return;
    };
    let (x, y) = u.cell();
    let (r, g, b) = row.rgb;
    w.lights.create(
        Some(owner),
        (
            unit_light_pos(i32::from(x) << 16),
            unit_light_pos(i32::from(y) << 16),
        ),
        LightKind::Plain,
        i32::from(row.light),
        crate::rules::lighting::sources::SOURCE_INTENSITY,
        r,
        g,
        b,
    );
}

/// A client missile's own light (missile +0x64), as the client skill do
/// functions set it: the old one removed, a new one created
/// at the missile's cell with
/// kind, radius, intensity and colour, then its target radius
/// when one is given.
#[allow(clippy::too_many_arguments)]
pub fn replace_missile_light(
    w: &mut ClientWorld,
    key: UnitKey,
    kind: LightKind,
    radius: i32,
    i: u8,
    (r, g, b): (u8, u8, u8),
    target: Option<i32>,
) {
    let owner = Owner {
        unit_type: u32::from(MISSILE),
        guid: key.guid,
        client_only: true,
    };
    let old = w
        .lights
        .iter()
        .find(|(_, rec)| rec.owner() == Some(owner))
        .map(|(id, _)| id);
    if let Some(id) = old {
        let _ = w.lights.remove(id);
    }
    let Some(u) = w.objclient.set_c.get(&key) else {
        return;
    };
    let (x, y) = u.cell();
    let id = w.lights.create(
        Some(owner),
        (
            unit_light_pos(i32::from(x) << 16),
            unit_light_pos(i32::from(y) << 16),
        ),
        kind,
        radius,
        i,
        r,
        g,
        b,
    );
    if let (Some(id), Some(t)) = (id, target) {
        w.lights.set_target(id, t);
    }
}

/// The client functions the model runs: the default step (1),
/// the client bodies named here.
pub const FN_DEFAULT_STEP: u16 = 1;
pub const FN_FLAT_AT_END: u16 = 11;
pub const FN_DEN_LIGHT: u16 = 23;
pub const FN_SUB_LOOP_FIRE: u16 = 5;
pub const FN_FOLLOW_OWNER: u16 = 43;
pub const FN_ORBIT_EVEN: u16 = 60;
pub const FN_ORBIT: u16 = 63;
pub const FN_TRAIL: u16 = 8;
pub const FN_SCATTER: u16 = 4;
pub const FN_MIST: u16 = 25;
pub const FN_SPAWN_FACING: u16 = 49;
pub const FN_WALL_MAKER: u16 = 6;
pub const FN_BLOOD: u16 = 2;
pub const FN_SUB_AT_STEP: u16 = 3;
pub const FN_HEIGHT_WINDOW: u16 = 59;
pub const FN_SPIRIT: u16 = 65;
pub const FN_GUIDED: u16 = 7;
pub const FN_METEOR: u16 = 9;
pub const FN_CURSE_CENTRE: u16 = 17;
pub const FN_SPEAR_TRAIL: u16 = 18;
pub const FN_WANDER_MAKER: u16 = 27;
pub const FN_DIABLO_WALL_MAKER: u16 = 16;
pub const FN_DIABLO_APPEARS: u16 = 37;
pub const FN_FRAME_ZERO: u16 = 39;
pub const FN_JAVELIN_TRAIL: u16 = 46;
pub const FN_RECYCLER: u16 = 51;
pub const FN_WAKE_MAKER: u16 = 52;
pub const FN_MON_BLIZZARD: u16 = 10;
pub const FN_BLIZZARD: u16 = 13;
pub const FN_FROZEN_ORB: u16 = 19;
pub const FN_ORB_NOVA: u16 = 20;
pub const FN_FINGER_MAGE_SPIDER: u16 = 15;
pub const FN_DISTRACTION: u16 = 44;
pub const FN_DISTRACTION_FOG: u16 = 45;
pub const FN_MOLTEN_BOULDER: u16 = 47;
pub const FN_ERUPTION: u16 = 48;
pub const FN_TIGER_FURY: u16 = 53;
pub const FN_CHAOS_ICE: u16 = 58;
pub const FN_SUC_FIREBALL: u16 = 68;
pub const FN_QUEEN_DEATH_CENTRE: u16 = 35;

/// The next-GUID pick with (C, x, y, r, mask, g)
/// for callers outside the missile code (the
/// client skill do).
#[allow(clippy::too_many_arguments)]
pub fn next_guid(
    w: &ClientWorld,
    env: &Env,
    c: UnitKey,
    at: (i32, i32),
    r: i32,
    mask: u32,
    g: u32,
) -> Result<Option<UnitKey>, HandlerError> {
    hits::next_guid(w, env, c, at, r, mask, g)
}

/// The per-update dispatch of the set-C missile `key`.
pub fn update(
    w: &mut ClientWorld,
    rows: &[ClientMissileRow],
    key: UnitKey,
    lights: bool,
) -> Result<(), HandlerError> {
    update_with(
        w,
        &Env {
            rows,
            lights,
            skills: None,
            monsters: &[],
            model: None,
        },
        key,
    )
}

/// What the client missile code reads beside the model: the `missiles`
/// rows, the light quality ≠ 0 and the skills
/// tables with the formula buffers (`ModelInputs::skill_tables`; the
/// missiles evaluator and the skill evaluator
/// of the bodies; `None`: a body that evaluates is a handler error).
#[derive(Clone, Copy)]
pub struct Env<'a> {
    pub rows: &'a [ClientMissileRow],
    pub lights: bool,
    pub skills: Option<&'a super::passive::Tables>,
    /// The client `monstats` rows (`ClientTables::monsters`): monster
    /// sizes and set-ups for the distance and hostility tests.
    pub monsters: &'a [Option<super::world::MonsterClass>],
    /// The model inputs (the `overlay` rows and the overlay create's
    /// tests, hit 10); `None`: hit 10 makes no
    /// overlay.
    pub model: Option<&'a super::world::ModelInputs>,
}

/// [`update`] with the skills tables ([`Env`]). After the dispatch the
/// missile's room list follows its path room ([`relink`]).
pub fn update_with(w: &mut ClientWorld, env: &Env, key: UnitKey) -> Result<(), HandlerError> {
    let r = dispatch(w, env, key);
    relink(w, key);
    r
}

/// The room recache of a client missile:
/// the unit
/// sits in the unit list of its path room (head insert when the room
/// changes), which is how the draw finds it. Set C keys are filed by
/// their view key (`ClientWorld::view_key`). A missile no longer in
/// set C, or whose path has no room, is in no list.
pub(crate) fn relink(w: &mut ClientWorld, key: UnitKey) {
    let vk = super::world::view_key(key);
    let room = w
        .objclient
        .missiles
        .get(&key)
        .filter(|_| w.objclient.set_c.contains_key(&key))
        .and_then(|m| m.room)
        .and_then(|r| w.drlg.as_ref()?.drlg.drlg_room_of(r));
    if room.is_some() && w.room_units.room_of(vk) == room {
        return;
    }
    w.room_units.place(vk, room);
}

fn dispatch(w: &mut ClientWorld, env: &Env, key: UnitKey) -> Result<(), HandlerError> {
    let rate = w.tick_rate;
    let frame = w.share_frame();
    let Some(class) = w.objclient.set_c.get(&key).map(|u| u.class) else {
        return Ok(());
    };
    // The motion update runs before the dispatch.
    if let Some(m) = w.objclient.missiles.get_mut(&key) {
        motion_update(&mut m.motion, rate, frame)?;
    }
    let Some(row) = env.rows.get(class as usize).copied() else {
        remove(w, key);
        return Ok(());
    };
    if row.flicker != 0 && rate.share(1, frame) != 0 {
        flicker(w, key, &row);
    }
    // A row without `Town` in a town room → default removal.
    if !row.town && in_town(w, key) {
        remove(w, key);
        return Ok(());
    }
    match row.clt_do_func {
        FN_DEFAULT_STEP => default_step(w, env, key, &row),
        FN_FLAT_AT_END => {
            // 11: at the animation end flag 0x10000 and the light
            // removed; else step. No countdown at the end.
            let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
            if m.frame + rate.share(m.anim_speed, frame) >= m.anim_len {
                if let Some(m) = w.objclient.missiles.get_mut(&key) {
                    m.flat = true;
                }
                remove_light(w, key);
                Ok(())
            } else {
                default_step(w, env, key, &row)
            }
        }
        FN_DEN_LIGHT => {
            // 23: frames left < 100 → 500; step (never expires).
            if let Some(m) = w.objclient.missiles.get_mut(&key) {
                if m.current < rate.ticks(100) {
                    m.current = rate.ticks(500);
                }
            }
            default_step(w, env, key, &row)
        }
        FN_TRAIL => trail(w, env, key, &row),
        FN_SCATTER => {
            // 4: scatter(m, P1, P2, P3, S1), then step.
            let [p1, p2, p3, ..] = row.clt_param;
            scatter(w, env, key, p1, p2, p3, i32::from(row.clt_sub[0]))?;
            default_step(w, env, key, &row)
        }
        FN_SPAWN_FACING => {
            // 49: no row or S1 < 0 → remove; elapsed mod max(P1, 1)
            // = 0 → spawn facing(m, S1); step.
            let s1 = row.clt_sub[0];
            if s1 < 0 {
                remove(w, key);
                return Ok(());
            }
            let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
            if (m.total - m.current) % rate.ticks(row.clt_param[0].max(1)).max(1) == 0 {
                if let Some(x) = spawn(w, env, key, s1 as u32)? {
                    if let Some(xm) = w.objclient.missiles.get_mut(&x) {
                        xm.direction = m.direction;
                    }
                }
            }
            default_step(w, env, key, &row)
        }
        FN_MIST => mist(w, env, key, &row),
        FN_WALL_MAKER => wall_maker(w, env, key, &row),
        FN_SUB_LOOP_FIRE => {
            if rate.share(1, frame) != 0 {
                fire_frames(w, key, &row);
            }
            default_step(w, env, key, &row)
        }
        FN_FOLLOW_OWNER => follow_owner(w, env, key),
        FN_ORBIT_EVEN | FN_ORBIT => {
            let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
            if (row.clt_do_func == FN_ORBIT && rate.share(1, frame) != 0)
                || (row.clt_do_func == FN_ORBIT_EVEN
                    && (m.total - m.current) % rate.ticks(2).max(1) == 0)
            {
                orbit(w, key);
            }
            default_step(w, env, key, &row)
        }
        FN_BLOOD => bodies::blood(w, env, key, &row),
        FN_SUB_AT_STEP => bodies::sub_at_new_step(w, env, key, &row),
        FN_HEIGHT_WINDOW => bodies::height_window(w, env, key, &row),
        FN_SPIRIT => bodies::spirit(w, env, key, &row),
        FN_GUIDED => bodies::guided(w, env, key, &row),
        FN_METEOR => bodies::meteor(w, env, key, &row),
        FN_CURSE_CENTRE => bodies::curse_centre(w, env, key, &row),
        FN_SPEAR_TRAIL => bodies::spear_trail(w, env, key, &row),
        FN_WANDER_MAKER => bodies::wander_maker(w, env, key, &row),
        FN_DIABLO_WALL_MAKER => bodies::wall_maker_16(w, env, key, &row),
        FN_FRAME_ZERO => bodies::frame_zero(w, env, key, &row),
        FN_JAVELIN_TRAIL | FN_WAKE_MAKER => bodies::pair_trail(w, env, key, &row),
        FN_RECYCLER => bodies::recycler(w, env, key, &row),
        FN_MON_BLIZZARD => bodies::mon_blizzard(w, env, key, &row),
        FN_BLIZZARD => bodies::blizzard(w, env, key, &row),
        FN_FROZEN_ORB => bodies::frozen_orb(w, env, key, &row),
        FN_ORB_NOVA => bodies::orb_nova(w, env, key, &row),
        FN_DISTRACTION => bodies::distraction(w, env, key, &row),
        FN_FINGER_MAGE_SPIDER => bodies::finger_mage_spider(w, env, key, &row),
        FN_DISTRACTION_FOG => {
            // 45: S1 < 0 → remove; scatter(m,
            // P1, P2, P3, S1); step.
            let s1 = row.clt_sub[0];
            if s1 < 0 {
                remove(w, key);
                return Ok(());
            }
            let [p1, p2, p3, ..] = row.clt_param;
            scatter(w, env, key, p1, p2, p3, i32::from(s1))?;
            default_step(w, env, key, &row)
        }
        FN_MOLTEN_BOULDER => bodies::molten_boulder(w, env, key, &row),
        FN_ERUPTION => bodies::eruption(w, env, key, &row),
        FN_TIGER_FURY => bodies::tiger_fury(w, env, key, &row),
        FN_CHAOS_ICE => bodies::chaos_ice(w, env, key, &row),
        FN_SUC_FIREBALL => bodies::suc_fireball(w, env, key, &row),
        FN_QUEEN_DEATH_CENTRE => bodies::queen_death_centre(w, env, key, &row),
        // The camera shake starts, then the step.
        12 | 29 | 31 | 36 | FN_DIABLO_APPEARS | 38 | 54 => bodies::shaker(w, env, key, &row),
        // Rule W.
        66 => bodies::worldstone_shake(w, key),
        // f ≤ 0: never stepped; other functions: not modelled.
        _ => Ok(()),
    }
}

/// Function 8 (trails): S1 < 0 → remove.
/// When elapsed ≥ `InitSteps` and the last path step entered a new
/// sub-tile: S1 created with flags 1 at m's position, m's owner,
/// skill and level, then its init callback (m): the child's
/// P2 > 0 → rnd(P2) on **m's** seed (the child's motion z, − ⌊P2 / 2⌋;
/// the motion record is not modelled, the draw is made); the child's
/// direction := m's; its frame := rnd(`AnimLen`) on the child's seed <<
/// 8. Then step.
fn trail(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    row: &ClientMissileRow,
) -> Result<(), HandlerError> {
    let rate = w.tick_rate;
    let s1 = row.clt_sub[0];
    if s1 < 0 {
        remove(w, key);
        return Ok(());
    }
    let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
    if m.total - m.current >= rate.ticks(i32::from(row.init_steps)) && m.new_step {
        let rec = CreateRecord {
            flags: flag::POSITION,
            owner: m.owner,
            class: s1 as u32,
            x: (m.pos.0 >> 16) as i32,
            y: (m.pos.1 >> 16) as i32,
            skill: m.skill,
            level: m.level,
            ..CreateRecord::default()
        };
        if let Some(child) = create_from(w, env, key, &rec)? {
            let crow = env.rows.get(s1 as usize).copied().unwrap_or_default();
            // The init callback: the child's height z :=
            // rnd(P2) on m's seed − ⌊P2 / 2⌋ (P2 > 0, else 0), its motion
            // position (0, 0, z) through the setter (`<< 11`),
            // so the draw lifts it by z.
            let p2 = crow.clt_param[1];
            let z = if p2 > 0 { rnd(w, key, p2) - p2 / 2 } else { 0 };
            let frame = rnd(w, child, i32::from(crow.anim_len)) << 8;
            if let Some(c) = w.objclient.missiles.get_mut(&child) {
                c.direction = m.direction;
                c.frame = frame;
                c.motion.pos = [0, 0, z.wrapping_shl(11)];
            }
        }
    }
    default_step(w, env, key, row)
}

/// Scatter with (m, chance, count, spread, c):
/// c out of range → nothing; frames left ≠ 0 and rnd(chance) ≠
/// 0 → nothing (left 0: no draw); else `count` creates (flags 0x20,
/// owner O, origin m, class c, m's skill and level), each aimed at (x +
/// dx + σ(dx)·spread, y + dy + σ(dy)·spread), dx, dy := rnd(2·spread) −
/// spread, σ(v) = −1 for v < 0, else +1.
#[allow(clippy::too_many_arguments)]
fn scatter(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    chance: i32,
    count: i32,
    spread: i32,
    c: i32,
) -> Result<(), HandlerError> {
    if c < 0 || c as usize >= env.rows.len() {
        return Ok(());
    }
    let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
    if m.current != 0 && (w.tick_rate.share(1, w.share_frame()) == 0 || rnd(w, key, chance) != 0) {
        return Ok(());
    }
    let (x, y) = ((m.pos.0 >> 16) as i32, (m.pos.1 >> 16) as i32);
    let sigma = |v: i32| if v < 0 { -1 } else { 1 };
    for _ in 0..count {
        let dx = rnd(w, key, 2 * spread) - spread;
        let dy = rnd(w, key, 2 * spread) - spread;
        let rec = CreateRecord {
            flags: flag::TARGET_ABSOLUTE,
            owner: m.owner,
            origin: Some(key),
            class: c as u32,
            tx: x + dx + sigma(dx) * spread,
            ty: y + dy + sigma(dy) * spread,
            skill: m.skill,
            level: m.level,
            ..CreateRecord::default()
        };
        create_from(w, env, key, &rec)?;
    }
    Ok(())
}

/// spawn(c), the client missile create with (m, c, 0, 0, skill, level):
/// flags 0x20, origin m, owner m's owner (none →
/// none), target (0, 0) absolute.
fn spawn(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    c: u32,
) -> Result<Option<UnitKey>, HandlerError> {
    let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
    let Some(owner) = m.owner else {
        return Ok(None);
    };
    let rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE,
        owner: Some(owner),
        origin: Some(key),
        class: c,
        skill: m.skill,
        level: m.level,
        ..CreateRecord::default()
    };
    create_from(w, env, key, &rec)
}

/// Function 25 (`towermist`): S1 <
/// 0 → remove; elapsed mod max(P3, 1) = 0 → u, v := rnd(2·P1 + 1) − P1,
/// sx, sy := x / y + rnd(2·P2 + 1) − P2 (in this order); a create with
/// flags 0x21, owner O, start (sx, sy), target (sx + u, sy + v), class
/// S1, skill, level. Step.
fn mist(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    row: &ClientMissileRow,
) -> Result<(), HandlerError> {
    let rate = w.tick_rate;
    let s1 = row.clt_sub[0];
    if s1 < 0 {
        remove(w, key);
        return Ok(());
    }
    let [p1, p2, p3, ..] = row.clt_param;
    let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
    if (m.total - m.current) % rate.ticks(p3.max(1)).max(1) == 0 {
        let (x, y) = ((m.pos.0 >> 16) as i32, (m.pos.1 >> 16) as i32);
        let u = rnd(w, key, 2 * p1 + 1) - p1;
        let v = rnd(w, key, 2 * p1 + 1) - p1;
        let sx = x + rnd(w, key, 2 * p2 + 1) - p2;
        let sy = y + rnd(w, key, 2 * p2 + 1) - p2;
        let rec = CreateRecord {
            flags: flag::POSITION | flag::TARGET_ABSOLUTE,
            owner: m.owner,
            class: s1 as u32,
            x: sx,
            y: sy,
            tx: sx + u,
            ty: sy + v,
            skill: m.skill,
            level: m.level,
            ..CreateRecord::default()
        };
        create_from(w, env, key, &rec)?;
    }
    default_step(w, env, key, row)
}

/// Function 6 (`firewallmaker`):
/// the last path step entered no new sub-tile → step. S1 < 0 or out of
/// range → remove. Else a create with flags 0x21 (0x8021 with frames :=
/// S1's `Range` when that is ≠ 0), owner O, start = target = (x, y), m's
/// skill and level, of a class picked on m's seed (S2 and S3 ≥ 0: rnd(3)
/// 0 → S2, 1 → S3, 2 → S1; only S2: rnd(2) 0 → S2, else S1; else S1),
/// without a light (flag 0x4000) when P1 = 0 or rnd(P1) ≠ 0; the child
/// takes m's precise position. Step.
fn wall_maker(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    row: &ClientMissileRow,
) -> Result<(), HandlerError> {
    let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
    if !m.new_step {
        return default_step(w, env, key, row);
    }
    let [s1, s2, s3] = row.clt_sub;
    if s1 < 0 || s1 as usize >= env.rows.len() {
        remove(w, key);
        return Ok(());
    }
    let (x, y) = ((m.pos.0 >> 16) as i32, (m.pos.1 >> 16) as i32);
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: m.owner,
        x,
        y,
        tx: x,
        ty: y,
        skill: m.skill,
        level: m.level,
        ..CreateRecord::default()
    };
    let range = i32::from(env.rows[s1 as usize].range);
    if range != 0 {
        rec.flags |= flag::RANGE;
        rec.range = range;
    }
    let c = if s2 >= 0 && s3 >= 0 {
        match rnd(w, key, 3) {
            0 => s2,
            1 => s3,
            _ => s1,
        }
    } else if s2 >= 0 {
        if rnd(w, key, 2) == 0 {
            s2
        } else {
            s1
        }
    } else {
        s1
    };
    rec.class = c as u32;
    let p1 = row.clt_param[0];
    if p1 == 0 || rnd(w, key, p1) != 0 {
        rec.flags |= flag::NO_LIGHT;
    }
    if let Some(child) = create_from(w, env, key, &rec)? {
        if let Some(cm) = w.objclient.missiles.get_mut(&child) {
            cm.drawn_at = Some(cm.pos);
            cm.pos = m.pos;
        }
    }
    default_step(w, env, key, row)
}

/// Function 5 before its step:
/// f := frame >> 8, A := `SubStart`, Z := `SubStop`; f = A − 1 → frame
/// = (f + rnd(Z − A)) << 8; else left = A → frame := max(A − 3, 0) <<
/// 8; else left < A → frame := max(f − 2, 0) << 8.
fn fire_frames(w: &mut ClientWorld, key: UnitKey, row: &ClientMissileRow) {
    let Some(m) = w.objclient.missiles.get(&key).copied() else {
        return;
    };
    let (f, a, z) = (
        m.frame >> 8,
        i32::from(row.sub_start),
        i32::from(row.sub_stop),
    );
    let frame = if f == a - 1 {
        let r = rnd(w, key, z - a);
        Some((f + r) << 8)
    } else if m.current == w.tick_rate.ticks(a) {
        Some((a - 3).max(0) << 8)
    } else if m.current < w.tick_rate.ticks(a) {
        Some((f - 2).max(0) << 8)
    } else {
        None
    };
    if let (Some(frame), Some(m)) = (frame, w.objclient.missiles.get_mut(&key)) {
        m.frame = frame;
    }
}

/// `rnd(n)` on the missile's unit seed (n < 1 →
/// 0, no draw).
fn rnd(w: &mut ClientWorld, key: UnitKey, n: i32) -> i32 {
    if n < 1 {
        return 0;
    }
    let Some(u) = w.objclient.set_c.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut seed = Seed::new(lo, hi);
    let r = seed.roll(n) as i32;
    u.seed = Some((seed.lo, seed.hi));
    r
}

/// Function 43: no owner → remove; an
/// owner of type ≤ 1 that is dead → end(none, 0); else m takes the
/// owner's position and advances its animation with wrap; no step, no
/// countdown.
fn follow_owner(w: &mut ClientWorld, env: &Env, key: UnitKey) -> Result<(), HandlerError> {
    let rate = w.tick_rate;
    let frame = w.share_frame();
    let owner = w
        .objclient
        .missiles
        .get(&key)
        .and_then(|m| m.owner)
        .and_then(|o| w.units.get(&o));
    let Some(o) = owner else {
        remove(w, key);
        return Ok(());
    };
    if o.key.unit_type <= MONSTER && o.is_dead() {
        end_with(w, env, key, None, false)?;
        return Ok(());
    }
    let (ox, oy) = o.cell();
    let room = w
        .drlg
        .as_ref()
        .and_then(|d| drlg_room_at(&d.drlg, i32::from(ox), i32::from(oy)));
    if let Some(u) = w.objclient.set_c.get_mut(&key) {
        u.position = Some((ox, oy));
    }
    if let Some(m) = w.objclient.missiles.get_mut(&key) {
        m.room = room;
        m.pos = (
            (u32::from(ox) << 16) | 0x8000,
            (u32::from(oy) << 16) | 0x8000,
        );
        m.frame += rate.share(m.anim_speed, frame);
        if m.anim_len > 0 && m.frame >= m.anim_len {
            m.frame -= m.anim_len;
        }
    }
    Ok(())
}

/// Functions 60 / 63, the orbit: target
/// point := (x + (y − d2C), y − (x − d28)), a quarter turn around (d28,
/// d2C), and the path re-built toward it.
fn orbit(w: &mut ClientWorld, key: UnitKey) {
    let Some(m) = w.objclient.missiles.get_mut(&key) else {
        return;
    };
    let (x, y) = cell_of(m);
    m.target_point = (x + (y - m.d2c), y - (x - m.d28));
    repath(m);
}

/// The light flicker: when the
/// frame has bits 0x300 clear, the missile has a light and its radius ≥
/// `Light`: target := `Light` + rnd(`Flicker`) on the missile's seed.
fn flicker(w: &mut ClientWorld, key: UnitKey, row: &ClientMissileRow) {
    let frame = w.objclient.missiles.get(&key).map_or(0, |m| m.frame);
    if frame & 0x300 != 0 {
        return;
    }
    let Some(id) = light_of(w, key) else {
        return;
    };
    if w.lights.radius(id).unwrap_or(0) < i32::from(row.light) {
        return;
    }
    let Some(u) = w.objclient.set_c.get_mut(&key) else {
        return;
    };
    let Some((lo, hi)) = u.seed else {
        return;
    };
    let mut seed = Seed::new(lo, hi);
    let r = seed.roll(i32::from(row.flicker)) as i32;
    u.seed = Some((seed.lo, seed.hi));
    w.lights.set_target(id, i32::from(row.light) + r);
}

/// The default step without the path (module doc).
fn default_pass(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    row: &ClientMissileRow,
) -> Result<bool, HandlerError> {
    let rate = w.tick_rate;
    let frame = w.share_frame();
    let Some(m) = w.objclient.missiles.get_mut(&key) else {
        return Ok(false);
    };
    let active = m.current <= m.activate;
    if active && m.total - m.current > rate.ticks(i32::from(row.init_steps)) {
        if let Some(u) = w.objclient.set_c.get_mut(&key) {
            u.flag_ex &= !FLAG_EX_NOT_DRAWN;
        }
    }
    let m = w.objclient.missiles.get_mut(&key).expect("checked above");
    // The motion record reports "not moving" (a flag-0x100 arc that
    // landed) → end(none, 1).
    if !m.motion.moving() {
        end_with(w, env, key, None, true)?;
        return Ok(false);
    }
    let m = w.objclient.missiles.get_mut(&key).expect("checked above");
    if active {
        if !row.loop_anim {
            if m.frame + rate.share(m.anim_speed, frame) < m.anim_len {
                m.frame += rate.share(m.anim_speed, frame);
            }
        } else if row.sub_loop != 0
            && m.current > rate.ticks(i32::from(row.anim_len) - i32::from(row.sub_stop) + 1)
        {
            m.frame += rate.share(m.anim_speed, frame);
            if m.frame >= i32::from(row.sub_stop) << 8 {
                m.frame += (i32::from(row.sub_start) - i32::from(row.sub_stop)) << 8;
            }
        } else {
            m.frame += rate.share(m.anim_speed, frame);
            if m.frame >= m.anim_len {
                m.frame -= m.anim_len;
            }
        }
    }
    // One path step.
    if m.velocity != 0 {
        path_step(w, env, key);
        let m = w.objclient.missiles.get(&key).expect("checked above");
        let cell = (m.pos.0 >> 16, m.pos.1 >> 16);
        let pos = m.pos;
        if let (Some(u), Ok(cx), Ok(cy)) = (
            w.objclient.set_c.get_mut(&key),
            u16::try_from(cell.0),
            u16::try_from(cell.1),
        ) {
            u.position = Some((cx, cy));
            // The path's precise point: the sound position reads the
            // dynamic path +0x08 / +0x0C.
            u.precise = Some(pos);
        }
    }
    let m = w.objclient.missiles.get_mut(&key).expect("checked above");
    m.current -= 1;
    if m.current < 1 {
        end_with(w, env, key, None, false)?;
        return Ok(false);
    }
    if !active {
        return Ok(false);
    }
    // `CltSrcTown` T ≠ 0 and (no owner, or the owner's room is a
    // town room): frames left ≥ T → T, and without `LoopAnim` with
    // `AnimLen` − T > 0, frame := (`AnimLen` − T) << 8.
    let t = rate.ticks(i32::from(row.clt_src_town));
    if t != 0 {
        let owner = w.objclient.missiles.get(&key).and_then(|m| m.owner);
        if owner.is_none_or(|o| super::modes::in_town(w, o)) {
            let m = w.objclient.missiles.get_mut(&key).expect("checked above");
            if m.current >= t {
                m.current = t;
                let rest = i32::from(row.anim_len) - rate.base_ticks(t);
                if !row.loop_anim && rest > 0 {
                    m.frame = rest << 8;
                }
            }
        }
    }
    if row.collide_type == 0 {
        return Ok(false);
    }
    // The collision word under the missile; & 5 → end(none, 1).
    let word = collision_under(w, env, key);
    if word & 5 != 0 {
        end_with(w, env, key, None, true)?;
        return Ok(false);
    }
    if row.collide_type == 6 {
        return Ok(false);
    }
    // `ClientCol` and a unit bit in the word: the unit search on
    // each saved step of the walk, in order; a living unit found ends
    // the missile on it, a dead one moves the search to the next point.
    if row.client_col && word & 0xFFFF != 0 {
        let m = w
            .objclient
            .missiles
            .get(&key)
            .copied()
            .expect("checked above");
        for &(x, y) in &m.steps[..usize::from(m.step_count)] {
            let found = unit_at_point(w, env, key, *row, m.room, x, y)?;
            if let Some(u) = found.filter(|&u| !is_dead(w, u)) {
                end_with(w, env, key, Some(u), false)?;
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// The default step: one pass, and the second pass of the client
/// update: the
/// first pass ran through, the owner is the local player (the game types
/// 0, 1, 6, 8 are every local and online game this build plays), the
/// elapsed frames are 1, frames are left and the path has a velocity.
fn default_step(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    row: &ClientMissileRow,
) -> Result<(), HandlerError> {
    let rate = w.tick_rate;
    if !default_pass(w, env, key, row)? {
        return Ok(());
    }
    let again = w.objclient.missiles.get(&key).is_some_and(|m| {
        m.owner.is_some()
            && m.owner == w.local_player
            && m.total - m.current == rate.ticks(1)
            && m.current > 0
            && m.velocity != 0
    });
    if again {
        default_pass(w, env, key, row)?;
    }
    Ok(())
}

/// The client act's active rooms as read-only [`CollisionRooms`]:
/// a missile's footprint mask is 0
/// (table), so its moves stamp nothing and `grid_mut` answers none.
/// A room's grid is its copy with the unit footprints
/// ([`stamp_unit_footprints`]) when it has one.
struct MissileRooms<'a>(
    &'a d2_sim::drlg::Drlg,
    &'a BTreeMap<RoomId, d2_sim::drlg::CollisionGrid>,
);

impl CollisionRooms for MissileRooms<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<d2_sim::drlg::TileRect> {
        let r = self.0.drlg_room_of(room)?;
        self.0.active_room(r).map(|a| a.subtiles)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.0
            .drlg_room_of(room)
            .and_then(|r| self.0.active_room(r))
            .map_or(0, |a| a.adjacency.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        let r = self.0.drlg_room_of(room)?;
        let n = *self.0.active_room(r)?.adjacency.get(i)?;
        self.0.active_room(n).map(|a| a.id)
    }
    fn grid(&self, room: RoomId) -> Option<&d2_sim::drlg::CollisionGrid> {
        if let Some(g) = self.1.get(&room) {
            return Some(g);
        }
        let r = self.0.drlg_room_of(room)?;
        self.0.active_room(r).map(|a| &a.collision)
    }
    fn grid_mut(&mut self, _: RoomId) -> Option<&mut d2_sim::drlg::CollisionGrid> {
        None
    }
}

/// [`CollisionRooms`] writing into copies of the client act's grids.
struct FootRooms<'a> {
    drlg: &'a d2_sim::drlg::Drlg,
    grids: BTreeMap<RoomId, d2_sim::drlg::CollisionGrid>,
}

impl CollisionRooms for FootRooms<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<d2_sim::drlg::TileRect> {
        MissileRooms(self.drlg, &self.grids).subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        MissileRooms(self.drlg, &self.grids).adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        MissileRooms(self.drlg, &self.grids).adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&d2_sim::drlg::CollisionGrid> {
        if let Some(g) = self.grids.get(&room) {
            return Some(g);
        }
        let r = self.drlg.drlg_room_of(room)?;
        self.drlg.active_room(r).map(|a| &a.collision)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut d2_sim::drlg::CollisionGrid> {
        if !self.grids.contains_key(&room) {
            let r = self.drlg.drlg_room_of(room)?;
            let g = self.drlg.active_room(r)?.collision.clone();
            self.grids.insert(room, g);
        }
        self.grids.get_mut(&room)
    }
}

/// [`CollisionRooms`] over the footprint layer: zeroed grids holding
/// only the unit and object bits, made on first touch.
struct LayerRooms<'a> {
    drlg: &'a d2_sim::drlg::Drlg,
    grids: &'a mut BTreeMap<RoomId, d2_sim::drlg::CollisionGrid>,
}

impl CollisionRooms for LayerRooms<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<d2_sim::drlg::TileRect> {
        MissileRooms(self.drlg, &*self.grids).subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        MissileRooms(self.drlg, &*self.grids).adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        MissileRooms(self.drlg, &*self.grids).adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&d2_sim::drlg::CollisionGrid> {
        self.grids.get(&room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut d2_sim::drlg::CollisionGrid> {
        if !self.grids.contains_key(&room) {
            let rect = self.subtile_rect(room)?;
            self.grids
                .insert(room, d2_sim::drlg::CollisionGrid::new(rect));
        }
        self.grids.get_mut(&room)
    }
}

/// Brings the footprint layer up to date for the units of `kinds`:
/// drops the rooms no longer active, clears the footprint of each unit
/// gone (or now without one), then, in `wanted` order, clears each moved
/// unit's old footprint and stamps its new one (/:
/// a clear ANDs the mask out, whoever else stamped the cell), and
/// rebuilds `unit_grids` as each layer room's grid ORed with the layer.
fn update_footprints(
    w: &mut ClientWorld,
    kinds: &[u8],
    wanted: Vec<(UnitKey, d2_sim::path::Footprint)>,
    restamp: &BTreeSet<UnitKey>,
) {
    let Some(d) = w.drlg.as_ref() else {
        w.objclient.unit_grids.clear();
        w.objclient.foot_layer.clear();
        w.objclient.foot_marks.clear();
        return;
    };
    let o = &mut w.objclient;
    let active = |room: RoomId| {
        d.drlg
            .drlg_room_of(room)
            .and_then(|r| d.drlg.active_room(r))
            .map(|a| a.subtiles)
    };
    o.foot_layer
        .retain(|&room, g| active(room).is_some_and(|rect| rect == g.rect));
    o.foot_marks
        .retain(|_, f| f.room.is_none_or(|room| active(room).is_some()));
    let mut rooms = LayerRooms {
        drlg: &d.drlg,
        grids: &mut o.foot_layer,
    };
    let want: BTreeMap<UnitKey, d2_sim::path::Footprint> = wanted.iter().copied().collect();
    let gone: Vec<UnitKey> = o
        .foot_marks
        .keys()
        .filter(|k| kinds.contains(&k.unit_type) && !want.contains_key(k))
        .copied()
        .collect();
    for k in gone {
        if let Some(f) = o.foot_marks.remove(&k) {
            d2_sim::path::footprint::remove_footprint(
                &mut rooms,
                &f,
                d2_sim::path::RemoveRule::Other,
                true,
            );
        }
    }
    for (k, f) in wanted {
        if o.foot_marks.get(&k) == Some(&f) {
            // A mode request on a living unit stamps its footprint again
            // without clearing it first: the cells a neighbour's clear
            // took come back.
            if restamp.contains(&k) {
                d2_sim::path::footprint::add_footprint(&mut rooms, &f);
            }
            continue;
        }
        if let Some(old) = o.foot_marks.insert(k, f) {
            d2_sim::path::footprint::remove_footprint(
                &mut rooms,
                &old,
                d2_sim::path::RemoveRule::Other,
                true,
            );
        }
        d2_sim::path::footprint::add_footprint(&mut rooms, &f);
    }
    o.unit_grids = o
        .foot_layer
        .iter()
        .filter_map(|(&room, layer)| {
            let base = d
                .drlg
                .drlg_room_of(room)
                .and_then(|r| d.drlg.active_room(r))?;
            let mut g = base.collision.clone();
            for (m, l) in g.masks.iter_mut().zip(&layer.masks) {
                *m |= *l;
            }
            Some((room, g))
        })
        .collect();
}

/// The unit footprints of the client grids as the client missiles read
/// them, brought up to date once per client update before the set-C
/// missile walk: every living player and monster of the model at its
/// sub-tile, with its pattern (size 0 → 0,
/// 1 and 2 → 1, 3 → 2, others → 1; a monster that can be in town (`npc`
/// or `inTown`) without `interact` 1 → 3, 2 → 4; players size 2,
/// monsters `monstats2` `SizeX`) and footprint mask (player 0x80,
/// monster 0x100; a dying or dead monster none), set S then the C
/// monsters, stamped on
/// a layer kept between updates: a unit that moved or died has its old
/// footprint cleared, which also clears the cells it
/// shared with others (2026-10-14, L08).
///
/// The base-game client stamps and clears these on its grids at each
/// sub-tile step of its units; the model's
/// grids carry none (the local player's walk prediction reads them
/// without other units, (d)), so the missiles
/// read copies with the layer, which moves each unit once per update from
/// its last model position to its new one. Settled by a capture of the
/// client state (equal over 180 frames) of a necromage missile, where a dying
/// Fallen's cleared plus lets the missile fly on.
pub fn stamp_unit_footprints(w: &mut ClientWorld, monsters: &[Option<super::world::MonsterClass>]) {
    let mut wanted = Vec::new();
    if let Some(d) = w.drlg.as_ref() {
        // The C monsters stand on the same grids as set S (the 0xAC
        // create's path and footprint).
        let set_c = w
            .objclient
            .set_c
            .iter()
            .filter(|(k, _)| k.unit_type == MONSTER);
        for (k, u) in w.units.iter().chain(set_c) {
            if u.is_dead() {
                continue;
            }
            let (size, town_npc, mask) = match k.unit_type {
                PLAYER => (2, false, 0x80),
                MONSTER => {
                    let c = monsters.get(u.class as usize).and_then(|c| c.as_ref());
                    let size = c.map_or(0, |c| i32::from(c.size_x));
                    let town = c.is_some_and(|c| (c.npc || c.in_town) && !c.interact);
                    (size, town, 0x100)
                }
                _ => continue,
            };
            let Some((x, y)) = u.position.map(|(x, y)| (i32::from(x), i32::from(y))) else {
                continue;
            };
            wanted.push((
                *k,
                d2_sim::path::Footprint {
                    room: drlg_room_at(&d.drlg, x, y),
                    x,
                    y,
                    shape: d2_sim::path::FootShape::Pattern(footprint_pattern(size, town_npc)),
                    mask,
                },
            ));
        }
    }
    let mut restamp = BTreeSet::new();
    let requests: BTreeMap<UnitKey, u32> = w
        .units
        .iter()
        .chain(&w.objclient.set_c)
        .filter(|(k, _)| k.unit_type == MONSTER && wanted.iter().any(|(j, _)| j == *k))
        .map(|(k, u)| (*k, u.mode_requests))
        .collect();
    for (k, n) in &requests {
        if w.objclient.foot_requests.get(k).is_some_and(|m| m != n) {
            restamp.insert(*k);
        }
    }
    w.objclient.foot_requests = requests;
    update_footprints(w, &[PLAYER, MONSTER], wanted, &restamp);
}

/// Stamps the footprints of the objects whose init stamped one
/// (`HasCollision[mode]`) on the
/// layer of [`stamp_unit_footprints`]: the objects.txt `SizeX` x
/// `SizeY` box with the object mask of
/// (`ObjectShape::foot_mask`). PROVISIONAL
/// (object part): stamped at the model position and mode of the
/// pass, as the units' footprints are; the base game stamps at the init and mode
/// change. Measured on a capture of the client grid (a Barracks door
/// opened at frame 40): equal through frame 40, then the capture's
/// click walk diverges (player `ty`, frame 41) before the door opens.
pub fn stamp_object_footprints(w: &mut ClientWorld, rows: &[super::objects::ObjClientRow]) {
    use super::world::{KindData, OBJECT};
    let mut wanted = Vec::new();
    if let Some(d) = w.drlg.as_ref() {
        let set_c = w
            .objclient
            .set_c
            .iter()
            .filter(|(k, _)| k.unit_type == OBJECT);
        for (k, u) in w
            .units
            .iter()
            .filter(|(k, _)| k.unit_type == OBJECT)
            .chain(set_c)
        {
            let KindData::Object(data) = &u.kind else {
                continue;
            };
            if !data.footprint {
                continue;
            }
            let Some(row) = rows.get(u.class as usize) else {
                continue;
            };
            let Some((x, y)) = u.position.map(|(x, y)| (i32::from(x), i32::from(y))) else {
                continue;
            };
            wanted.push((
                *k,
                d2_sim::path::Footprint {
                    room: drlg_room_at(&d.drlg, x, y),
                    x,
                    y,
                    shape: d2_sim::path::FootShape::Box {
                        size_x: row.shape.size_x,
                        size_y: row.shape.size_y,
                    },
                    mask: row.shape.foot_mask(),
                },
            ));
        }
    }
    update_footprints(w, &[OBJECT], wanted, &BTreeSet::new());
}

/// The footprint pattern of a unit size:
/// size 0 → 0, 1 and 2 → 1, 3 → 2, others → 1; a monster that can be
/// in town without `interact`: 3 → 4, others but 0 → 3).
pub fn footprint_pattern(size: i32, town_npc: bool) -> u32 {
    match (size, town_npc) {
        (0, _) => 0,
        (3, false) => 2,
        (3, true) => 4,
        (_, true) => 3,
        _ => 1,
    }
}

/// The footprint collide test with (room, x, y, size, mask) over the client DRLG with the
/// unit footprints ([`stamp_unit_footprints`]): `room` is the active
/// room record the query starts from. No client DRLG → 0.
pub fn size_query(w: &ClientWorld, room: RoomId, x: i32, y: i32, size: i32, mask: u16) -> u16 {
    w.drlg.as_ref().map_or(0, |d| {
        size_value(
            &MissileRooms(&d.drlg, &w.objclient.unit_grids),
            Some(room),
            x,
            y,
            size,
            mask,
        )
    })
}

/// The area collide test with (room, x, y, sx, sy, mask), sx = sy = `size`, from
/// U's room: both sizes < 2 → the point query,
/// else the box query.
/// U in no room → none; no client DRLG → Some(0).
pub fn unit_room_query(
    w: &ClientWorld,
    key: UnitKey,
    x: i32,
    y: i32,
    size: u32,
    mask: u16,
) -> Option<u16> {
    let (ux, uy) = w.units.get(&key)?.cell();
    let Some(d) = w.drlg.as_ref() else {
        return Some(0);
    };
    let room = drlg_room_at(&d.drlg, i32::from(ux), i32::from(uy))?;
    let rooms = MissileRooms(&d.drlg, &w.objclient.unit_grids);
    Some(if size < 2 {
        d2_sim::path::collision::point_value(&rooms, Some(room), x, y, mask)
    } else {
        d2_sim::path::collision::box_value(&rooms, Some(room), x, y, (size, size), mask)
    })
}

/// The free point with (room, &(x, y), size, mask, fallback)
/// on the client grids the missiles read,
/// from the active room holding (x, y): the point found, `None` when
/// no room holds (x, y) or the search finds none. Without a client DRLG
/// (the model's unit tests) (x, y) reads free.
pub fn free_point(
    w: &ClientWorld,
    (x, y): (i32, i32),
    size: i32,
    mask: u16,
    fallback: bool,
) -> Option<(i32, i32)> {
    let Some(d) = w.drlg.as_ref() else {
        return Some((x, y));
    };
    let room = drlg_room_at(&d.drlg, x, y)?;
    let rooms = MissileRooms(&d.drlg, &w.objclient.unit_grids);
    let view = d2_sim::wiring::path::place::Rooms(&rooms);
    let mut pt = d2_sim::path::coords::Point::new(x, y);
    d2_sim::path::search::free_point(&view, Some(room), &mut pt, size, u32::from(mask), fallback)
        .ok()
        .flatten()
        .map(|_| (pt.x, pt.y))
}

/// Stamps one unit footprint (size pattern,
/// footprint mask) at (x, y) on the footprint grids, as a new unit's
/// placement does (settled, as [`stamp_unit_footprints`]).
pub fn stamp_footprint(w: &mut ClientWorld, x: i32, y: i32, pattern: u32, mask: u16) {
    let Some(d) = w.drlg.as_ref() else {
        return;
    };
    let mut rooms = FootRooms {
        drlg: &d.drlg,
        grids: std::mem::take(&mut w.objclient.unit_grids),
    };
    let room = drlg_room_at(&d.drlg, x, y);
    d2_sim::path::footprint::stamp_pattern(&mut rooms, room, x, y, pattern, mask);
    w.objclient.unit_grids = rooms.grids;
}

/// The active room of the client act whose sub-tile rect holds (x, y)
/// (the path's room at the create).
fn drlg_room_at(d: &d2_sim::drlg::Drlg, x: i32, y: i32) -> Option<RoomId> {
    d.active_rooms()
        .into_iter()
        .find(|&(_, r)| d.active_room(r).is_some_and(|a| a.subtiles.contains(x, y)))
        .map(|(id, _)| id)
}

fn cell16(p: u32) -> i32 {
    (p >> 16) as i32
}

fn centre16(c: i32) -> u32 {
    ((c as u32) << 16) | 0x8000
}

/// One path step of a missile, the movement with (m, 0x400)
/// with the one step and cell walk:
///
/// 1. The new-step flag and the collided mask cleared.
/// 2. A stopped path (point count 0, step 9.6.8) or a zero direction
///    vector: the movement reset (position := its cell centre).
/// 3. The acceleration counter; velocity vector := ((0x400 ×
///    velocity) >> 6) × direction vector >> 12.
/// 4. In another cell: the cell walk in steps halved to at
///    most 0x10000 per axis; each cell change is the missile move
///    (the size query at the new cell with
///    the move mask, ORed into the collided mask), refused when the
///    result has 0x1 or 0x4 → Q := the centre of the last free cell; each
///    entered cell is a saved step (at most 10, then the walk stops); the
///    new-step flag is set when the walk ends with a saved step and was
///    not refused.
/// 5. Set position: a new cell in no room → point count 0,
///    position unchanged; else, only with the room-exit flag,
///    the room recache from the path's room.
///
/// With no client DRLG the step moves without the walk.
fn path_step(w: &mut ClientWorld, env: &Env, key: UnitKey) {
    let rate = w.tick_rate;
    let frame = w.share_frame();
    // The step places the path through: the pixel pair
    // follows the position again.
    if let Some(m) = w.objclient.missiles.get_mut(&key) {
        m.drawn_at = None;
    }
    let size = w
        .objclient
        .set_c
        .get(&key)
        .and_then(|u| env.rows.get(u.class as usize))
        .map_or(0, |r| i32::from(r.size));
    let move_mask = w
        .objclient
        .set_c
        .get(&key)
        .and_then(|u| env.rows.get(u.class as usize))
        .map_or(0, |r| move_mask(r.collide_type));
    let drlg = w.drlg.as_ref().map(|d| &d.drlg);
    let unit_grids = &w.objclient.unit_grids;
    let Some(m) = w.objclient.missiles.get_mut(&key) else {
        return;
    };
    // 1.
    m.new_step = false;
    m.collided = 0;
    m.step_count = 0;
    // 2.
    let bolt_ended = m.bolt.is_some_and(|b| b.index >= b.count);
    if m.stopped || m.dir_vec == (0, 0) || bolt_ended {
        m.pos = (centre16(cell16(m.pos.0)), centre16(cell16(m.pos.1)));
        return;
    }
    // 3.
    if m.accel != 0 {
        m.accel_counter += 1;
        if m.accel_counter >= rate.ticks(5) {
            m.velocity = (m.velocity + i32::from(m.accel)).clamp(0, m.max_vel);
            if m.velocity == m.max_vel {
                m.accel = 0;
            }
            m.accel_counter = 0;
        }
    }
    let k = 0x400i32.wrapping_mul(m.velocity) >> 6;
    let mut d = (
        k.wrapping_mul(m.dir_vec.0) >> 12,
        k.wrapping_mul(m.dir_vec.1) >> 12,
    );
    if d == (0, 0) {
        m.pos = (centre16(cell16(m.pos.0)), centre16(cell16(m.pos.1)));
        return;
    }
    d = (rate.share(d.0, frame), rate.share(d.1, frame));
    if d == (0, 0) {
        return;
    }
    // 3a. The snap (a point path, type 10 or 14, only here): the point
    // within the step's larger axis → Δ := the way to its centre.
    let mut reach = false;
    if let Some(b) = m.bolt {
        let p = b.points[usize::from(b.index)];
        let r = (
            centre16(p.0).wrapping_sub(m.pos.0) as i32,
            centre16(p.1).wrapping_sub(m.pos.1) as i32,
        );
        let most = d.0.abs().max(d.1.abs());
        if r.0.abs() <= most && r.1.abs() <= most {
            d = r;
            reach = true;
        }
    }
    let end = (
        m.pos.0.wrapping_add_signed(d.0),
        m.pos.1.wrapping_add_signed(d.1),
    );
    let old = (cell16(m.pos.0), cell16(m.pos.1));
    let end_cell = (cell16(end.0), cell16(end.1));
    let Some(drlg) = drlg else {
        m.pos = end;
        m.new_step = end_cell != old;
        bolt_reached(m, reach, false);
        bolt_next_point(m);
        return;
    };
    let mut rooms = MissileRooms(drlg, unit_grids);
    // 4.
    let mut q = end;
    if let Some(t) = m.exit_test.take() {
        room_exit_test(m, &rooms, t);
    }
    let mut blocked = false;
    if end_cell != old {
        let (mut sx, mut sy) = d;
        while sx.abs() > 0x10000 || sy.abs() > 0x10000 {
            sx >>= 1;
            sy >>= 1;
        }
        let mut c = m.pos;
        while (cell16(c.0), cell16(c.1)) != end_cell {
            let n = (c.0.wrapping_add_signed(sx), c.1.wrapping_add_signed(sy));
            let (cc, nc) = ((cell16(c.0), cell16(c.1)), (cell16(n.0), cell16(n.1)));
            if cc != nc {
                let r = missile_move(&mut rooms, m.room, cc, nc, size, 0, move_mask);
                m.collided |= r;
                if r & 0x5 != 0 {
                    blocked = true;
                    q = (centre16(cc.0), centre16(cc.1));
                    break;
                }
                m.steps[usize::from(m.step_count)] = nc;
                m.step_count += 1;
                if m.step_count >= 10 {
                    break;
                }
            }
            c = n;
        }
        m.new_step = !blocked && m.step_count > 0;
    }
    bolt_reached(m, reach, blocked);
    // 5.
    let qc = (cell16(q.0), cell16(q.1));
    let room = if m.room.is_some_and(|r| {
        rooms
            .subtile_rect(r)
            .is_some_and(|rect| rect.contains(qc.0, qc.1))
    }) {
        m.room
    } else {
        find_room(&rooms, m.room, qc.0, qc.1)
    };
    match room {
        None => m.stopped = true,
        Some(r) => {
            // The recache only with the room-exit flag.
            if m.room_exit {
                m.room = Some(r);
            }
            m.pos = q;
            bolt_next_point(m);
        }
    }
}

/// The move-test masks of the client collide table, by
/// `CollideType`.
const MOVE_MASKS: [u16; 9] = [0, 0x84, 0x104, 0x184, 0, 0x104, 4, 0x40, 0x185];

fn move_mask(collide_type: u8) -> u16 {
    MOVE_MASKS
        .get(usize::from(collide_type))
        .copied()
        .unwrap_or(0)
}

/// The point test with (room, x, y, mask) from `room`
/// (no room → 0x27 unmasked) over the
/// client DRLG; no client DRLG → 0.
fn point_value_from(w: &ClientWorld, room: Option<RoomId>, x: i32, y: i32, mask: u16) -> u32 {
    w.drlg.as_ref().map_or(0, |d| {
        u32::from(d2_sim::path::collision::point_value(
            &MissileRooms(&d.drlg, &w.objclient.unit_grids),
            room,
            x,
            y,
            mask,
        ))
    })
}

/// The collision word under the missile:
/// with path velocity 0 the size query (`Size`) at the path
/// position with all bits, from the path's room (no room → 0x27);
/// otherwise the collided mask the last step cached. No client DRLG →
/// 0.
fn collision_under(w: &ClientWorld, env: &Env, key: UnitKey) -> u32 {
    let (Some(d), Some(m)) = (w.drlg.as_ref(), w.objclient.missiles.get(&key)) else {
        return 0;
    };
    if m.velocity != 0 {
        return u32::from(m.collided);
    }
    let size = w
        .objclient
        .set_c
        .get(&key)
        .and_then(|u| env.rows.get(u.class as usize))
        .map_or(0, |r| i32::from(r.size));
    let (x, y) = cell_of(m);
    u32::from(size_value(
        &MissileRooms(&d.drlg, &w.objclient.unit_grids),
        m.room,
        x,
        y,
        size,
        0xFFFF,
    ))
}

/// The end with (m, none, forced) of a missile that hit no
/// unit. Returns the explosion missile made, if any.
pub fn end(
    w: &mut ClientWorld,
    rows: &[ClientMissileRow],
    key: UnitKey,
    forced: bool,
    lights: bool,
) -> Result<Option<UnitKey>, HandlerError> {
    let env = Env {
        rows,
        lights,
        skills: None,
        monsters: &[],
        model: None,
    };
    end_with(w, &env, key, None, forced)
}

/// [`end`] with the skills tables ([`Env`]).
pub fn end_with(
    w: &mut ClientWorld,
    env: &Env,
    key: UnitKey,
    unit: Option<UnitKey>,
    forced: bool,
) -> Result<Option<UnitKey>, HandlerError> {
    let rate = w.tick_rate;
    let Some(class) = w.objclient.set_c.get(&key).map(|u| u.class) else {
        return Ok(None);
    };
    let row = env.rows.get(class as usize).copied().unwrap_or_default();
    let m = w.objclient.missiles.get(&key).copied().unwrap_or_default();
    let owner = m.owner;
    let mut r = 3;
    // The hit tests.
    if let Some(u) = unit {
        if row.next_hit && just_hit(w, u) {
            return Ok(None);
        }
        if row.last_collide {
            if m.last_collided == Some(u) {
                return Ok(None);
            }
            if let Some(m) = w.objclient.missiles.get_mut(&key) {
                m.last_collided = Some(u);
            }
        }
        if !flags_4_8(w, u) {
            return Ok(None);
        }
        if let Some(o) = owner {
            if !super::combat::hostile_between(w, env.monsters, o, u) && !row.collide_friend {
                return Ok(None);
            }
            let piercing = w
                .units
                .get(&o)
                .is_some_and(|ou| ou.states.contains(&69) || w.total(o, 156, 0) != 0);
            if row.pierce && o.unit_type <= MONSTER && piercing {
                let mm = w.objclient.missiles.get_mut(&key).expect("checked above");
                if mm.pierce != 0 {
                    mm.pierce -= 1;
                    r = 2;
                }
            }
        }
    }
    let mut x = None;
    // No unit, not forced, no `AlwaysExplode` → the end below.
    if unit.is_some() || forced || row.always_explode {
        // U gets state 86 for `NextDelay` frames.
        if let (true, Some(u)) = (row.next_hit, unit) {
            w.objclient
                .just_hit
                .insert(u, rate.ticks(i32::from(row.next_delay)));
        }
        // U's client event hooks of kind 0: not
        // modelled (no client event hook list in the model):
        // the client hit function; 0 → the missile stays.
        match hits::call(w, env, key, unit, row.clt_hit_func) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(e) => {
                // A body the model cannot run: the error is reported and
                // the end goes on as for a non-zero result without the
                // explosion, so the missile does not stay to
                // fail again on every update (PROVISIONAL; settled by the
                // `monstats2.txt` `large` column for hit 52 and a capture
                // of a Molten Boulder hitting a Fallen).
                if let Some(id) = light_of(w, key) {
                    let _ = w.lights.die(id);
                }
                super::objects::remove_client_unit(w, key);
                return Err(e);
            }
        }
        // `HitSound` ≥ 0 → a request on m.
        if row.hit_sound >= 0 {
            w.objclient.missile_sounds.push(MissileSound::Request {
                id: i32::from(row.hit_sound),
                missile: key,
            });
        }
        // The client missile create with (m, E, 0, 0, skill, level): flags 0x20, the
        // owner m's owner (none → none), origin m.
        if row.explosion_missile >= 0 {
            if let Some(owner) = owner {
                let rec = CreateRecord {
                    flags: flag::TARGET_ABSOLUTE,
                    owner: Some(owner),
                    origin: Some(key),
                    class: row.explosion_missile as u32,
                    skill: m.skill,
                    level: m.level,
                    ..CreateRecord::default()
                };
                x = create_from(w, env, key, &rec)?;
                if let Some(xk) = x {
                    // X's motion position := m's (x, y, z) read through
                    // the getters
                    // (stored >> 11), so with the low 11 bits cleared;
                    // done, restart.
                    if let Some(xm) = w.objclient.missiles.get_mut(&xk) {
                        xm.motion.pos = m.motion.pos.map(|v| (v >> 11) << 11);
                        xm.motion.done();
                        xm.motion.restart();
                    }
                    // X's direction: rnd(64) on X's seed for 146
                    // `spidergoo`, else m's.
                    let dir = if row.explosion_missile == 146 {
                        let u = w.objclient.set_c.get_mut(&xk).expect("just made");
                        u.seed.map(|(lo, hi)| {
                            let mut s = Seed::new(lo, hi);
                            let d = s.roll(64) as u8;
                            u.seed = Some((s.lo, s.hi));
                            d
                        })
                    } else {
                        Some(m.direction)
                    };
                    if let (Some(d), Some(xm)) = (dir, w.objclient.missiles.get_mut(&xk)) {
                        xm.direction = d;
                    }
                }
            }
        }
    }
    // A unit hit without `CollideKill`: the missile flies on.
    if unit.is_some() && !row.collide_kill {
        return Ok(x);
    }
    // `TravelSound` ≥ 0 → its request on m detached.
    if row.travel_sound >= 0 {
        w.objclient.missile_sounds.push(MissileSound::DetachTravel {
            missile: key,
            id: i32::from(row.travel_sound),
        });
    }
    // M's light dies.
    if let Some(id) = light_of(w, key) {
        let _ = w.lights.die(id);
    }
    // R & 1 (r = 3) → m removed from set C; a piercing hit (r = 2)
    // keeps it (Edge case 2).
    if r & 1 != 0 {
        super::objects::remove_client_unit(w, key);
    }
    Ok(x)
}

/// U is in state 86 `justhit`: the server's state, or the
/// client's own state list while its frames run
/// ([`super::objects::ClientObjects::just_hit`]).
fn just_hit(w: &ClientWorld, u: UnitKey) -> bool {
    w.units.get(&u).is_some_and(|x| x.states.contains(&86)) || w.objclient.just_hit.contains_key(&u)
}

/// U's unit flags +0xC4 0x4 and 0x8, read by the collide test and
/// the hit tests. A player has both while not dead; a monster
/// has 0x8 and 0x4 := `monstats2.txt` `isAtt` (set up by
/// `msg/units.rs` `setup_flags`; cleared by the death); objects, items,
/// missiles neither.
/// Settled by the `monstats2.txt` `isAtt` column. Not modelled:
/// the vulture motion code 0x0C, after which the monster has neither.
fn flags_4_8(w: &ClientWorld, u: UnitKey) -> bool {
    match u.unit_type {
        PLAYER => w.units.get(&u).is_some_and(|x| !x.is_dead()),
        MONSTER => w.units.get(&u).is_some_and(|x| x.flag_4 && !x.is_dead()),
        _ => false,
    }
}

/// The dead test of a model
/// unit; units outside the model (set C) are not dead.
fn is_dead(w: &ClientWorld, u: UnitKey) -> bool {
    w.units.get(&u).is_some_and(|x| x.is_dead())
}

/// The unit at a point with (room, x, y, accept, m, size)
/// of a client missile, with the collide
/// test of its `CollideType`: the rooms of the missile's room's
/// adjacency array in order, each room's unit list from its head; players
/// in mode 0 / 17, monsters in mode 0 / 12, objects, items and tiles
/// skipped; a candidate of size s (≤ 0 skipped, above 3 read as 3) whose
/// shape meets the query shape of the missile's `Size`
/// (`shapes_overlap`) and that the test accepts is returned. `Size`
/// outside 1…3 or no room → none.
fn unit_at_point(
    w: &ClientWorld,
    env: &Env,
    key: UnitKey,
    row: ClientMissileRow,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<Option<UnitKey>, HandlerError> {
    let r = i32::from(row.size);
    let (Some(d), Some(room)) = (w.drlg.as_ref(), room) else {
        return Ok(None);
    };
    if !(1..=3).contains(&r) {
        return Ok(None);
    }
    if row.collide_type == 4 {
        return Err(HandlerError::Invalid(
            "missiles/client.md §C8: CollideType 4 passes a null test to the unit search",
        ));
    }
    let Some(adjacency) = d
        .drlg
        .drlg_room_of(room)
        .and_then(|dr| d.drlg.active_room(dr))
        .map(|a| a.adjacency.clone())
    else {
        return Ok(None);
    };
    for dr in adjacency {
        for &u in w.room_units.list(dr) {
            let mode = w.units.get(&u).map_or(0, |x| x.mode);
            match u.unit_type {
                PLAYER if mode == 0 || mode == 17 => continue,
                MONSTER if mode == 0 || mode == 12 => continue,
                PLAYER | MONSTER | MISSILE => {}
                _ => continue,
            }
            let s = unit_size(env, w, u);
            if s <= 0 {
                continue;
            }
            let Some((ux, uy)) = w.units.get(&u).map(|x| x.cell()) else {
                continue;
            };
            let (dx, dy) = ((x - i32::from(ux)).abs(), (y - i32::from(uy)).abs());
            if d2_sim::path::collision::shapes_overlap(r, s.min(3), dx, dy)
                && collide_test(w, env, key, &row, u)
            {
                return Ok(Some(u));
            }
        }
    }
    Ok(None)
}

/// The collide test of the missile's `CollideType` (table): the
/// common test (U flags 0x8 and 0x4; not (`NextHit` and U in
/// state 86); not (`LastCollide` and U the last-collided); no owner, or
/// the owner hostile to U, or `CollideFriend`) for players (modes 1, 3,
/// 8) and monsters (2, 3, 5, 8); mode 1 accepts monsters whose alignment
///    (state 105's list, stat 172) is 2 without it; mode 7 accepts missiles
///    whose row has `CanDestroy`.
fn collide_test(
    w: &ClientWorld,
    env: &Env,
    key: UnitKey,
    row: &ClientMissileRow,
    u: UnitKey,
) -> bool {
    let common = || {
        let Some(m) = w.objclient.missiles.get(&key) else {
            return false;
        };
        flags_4_8(w, u)
            && !(row.next_hit && just_hit(w, u))
            && !(row.last_collide && m.last_collided == Some(u))
            && (m
                .owner
                .is_none_or(|o| super::combat::hostile_between(w, env.monsters, o, u))
                || row.collide_friend)
    };
    match (row.collide_type, u.unit_type) {
        (1 | 3 | 8, PLAYER) => common(),
        (2 | 3 | 5 | 8, MONSTER) => common(),
        (1, MONSTER) => {
            w.units
                .get(&u)
                .and_then(|x| x.state_lists.get(&105))
                .and_then(|l| l.get(&(172, 0)))
                == Some(&2)
        }
        (7, MISSILE) => w
            .units
            .get(&u)
            .and_then(|x| env.rows.get(x.class as usize))
            .is_some_and(|r| r.can_destroy),
        _ => false,
    }
}

/// The client state lists count their frames down, one per client
/// update, before the set-C missile walk (`NextDelay` counts U's own
/// client updates; an open question): an entry at 0 is removed.
pub fn tick_just_hit(w: &mut ClientWorld) {
    w.objclient.just_hit.retain(|_, n| {
        *n -= 1;
        *n > 0
    });
}

/// The town test of a missile's room:
/// the active room containing its sub-tile (set-C units are in
/// no room list), its level a town level;
/// no client DRLG or no room → not in town.
fn in_town(w: &ClientWorld, key: UnitKey) -> bool {
    let Some(m) = w.objclient.missiles.get(&key) else {
        return false;
    };
    let (x, y) = cell_of(m);
    let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
        return false;
    };
    w.room_at(x, y)
        .is_some_and(|r| d2_sim::drlg::is_town(u32::from(r.level)))
}

/// The unit size of a client
/// unit the missile code measures: player 2, monster `monstats2`
/// `SizeX`, missile `Size`, item 1; objects and tiles 0 (the object rows
/// are not in [`Env`]; no modelled body measures one).
fn unit_size(env: &Env, w: &ClientWorld, k: UnitKey) -> i32 {
    use super::world::ITEM;
    match k.unit_type {
        PLAYER => 2,
        MONSTER => w
            .units
            .get(&k)
            .and_then(|u| env.monsters.get(u.class as usize))
            .and_then(|c| c.as_ref())
            .map_or(0, |c| i32::from(c.size_x)),
        MISSILE => w
            .objclient
            .set_c
            .get(&k)
            .or_else(|| w.units.get(&k))
            .and_then(|u| env.rows.get(u.class as usize))
            .map_or(0, |r| i32::from(r.size)),
        ITEM => 1,
        _ => 0,
    }
}

/// The motion update of a missile's record;
/// no follow record is made by
/// the modelled creators, so the linked unit is none.
fn motion_update(
    r: &mut MotionRecord,
    rate: d2_sim::tick::TickRate,
    frame: i32,
) -> Result<(), HandlerError> {
    r.update_at(true, None, rate, frame).map_err(|_| {
        HandlerError::Invalid("render/unit-composite.md §8 r4: a missile follows a non-monster")
    })
}

/// One advance of the missile's seed (the seed step): the new low
/// word lo'.
fn seed_step(w: &mut ClientWorld, key: UnitKey) -> u32 {
    let Some(u) = w.objclient.set_c.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut seed = Seed::new(lo, hi);
    let lo = seed.step();
    u.seed = Some((seed.lo, seed.hi));
    lo
}

/// The default removal: the light removed, the
/// unit removed.
pub fn remove(w: &mut ClientWorld, key: UnitKey) {
    remove_light(w, key);
    super::objects::remove_client_unit(w, key);
}

fn light_owner(key: UnitKey) -> Owner {
    Owner {
        unit_type: u32::from(MISSILE),
        guid: key.guid,
        client_only: true,
    }
}

fn light_of(w: &ClientWorld, key: UnitKey) -> Option<crate::rules::lighting::records::LightId> {
    let owner = light_owner(key);
    w.lights
        .iter()
        .find(|(_, r)| r.owner() == Some(owner))
        .map(|(id, _)| id)
}

fn remove_light(w: &mut ClientWorld, key: UnitKey) {
    if let Some(id) = light_of(w, key) {
        let _ = w.lights.remove(id);
    }
}

#[cfg(test)]
#[path = "client_missiles_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "client_missiles_bodies_tests.rs"]
mod bodies_tests;

#[cfg(test)]
#[path = "client_missiles_hits_tests.rs"]
mod hits_tests;
