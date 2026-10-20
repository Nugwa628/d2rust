//! Path records: which unit types have a static or a dynamic path,
//! the static path (0x20 bytes) and the dynamic path (0x200 bytes) with
//! the fields the path and walk code use, the dynamic path
//! allocation, and unit size, collision pattern and footprint
//! mask.

use crate::units::{RoomId, UnitId, UnitType};

use super::collision::{masks, CollisionRooms};
use super::coords::{client_from_precise, client_from_subtile, subtile_of, to_fp16_center, Point};
use super::footprint::stamp_pattern;
use super::tables::PathTables;
use super::PathError;

/// Dynamic path flags (+0x34).
pub mod flags {
    /// A path point (or a move's destination) lies outside the current room.
    pub const OUTSIDE_ROOM: u32 = 0x1;
    /// Room changed since the room-change messages.
    pub const ROOM_CHANGED: u32 = 0x2;
    /// Move footprints without testing.
    pub const NO_TEST: u32 = 0x4;
    /// The last step crossed at least one cell.
    pub const MOVED: u32 = 0x8;
    /// Set by the allocation argument; keeps the target.
    pub const KEEP_TARGET: u32 = 0x10;
    /// A path is active.
    pub const ACTIVE: u32 = 0x20;
    /// Face away (−32 on the computed direction).
    pub const FACE_AWAY: u32 = 0x200;
    /// Remove the target unit's footprint while computing (path compute
    /// Step 6).
    pub const REMOVE_TARGET_FOOTPRINT: u32 = 0x800;
    /// Prepare a blocked target.
    pub const PREPARE_TARGET: u32 = 0x1000;
    /// A type set stores the previous type.
    pub const SAVE_PREV_TYPE: u32 = 0x2000;
    /// The previous type is already stored.
    pub const PREV_TYPE_KEPT: u32 = 0x4000;
    /// A type set stores the velocity.
    pub const SAVE_VELOCITY: u32 = 0x8000;
    /// The velocity is already stored.
    pub const VELOCITY_KEPT: u32 = 0x10000;
    /// Store saved steps.
    pub const SAVE_STEPS: u32 = 0x20000;
    /// Path-type flags 0x800..0x20000, cleared by a type set.
    pub const PATH_TYPE_BITS: u32 = 0x7FF00;
    /// Missile path.
    pub const MISSILE: u32 = 0x40000;
}

/// Path type numbers used here.
pub mod path_types {
    pub const ASTAR: u32 = 1;
    pub const TOWARD: u32 = 2;
    pub const MISSILE: u32 = 4;
    pub const STRAIGHT: u32 = 7;
    pub const KNOCKBACK_SERVER: u32 = 8;
    pub const KNOCKBACK_CLIENT: u32 = 11;
    /// Toward, finishing on a target unit.
    pub const TOWARD_FINISH: u32 = 13;
    /// Wall follow.
    pub const WALL_FOLLOW: u32 = 15;
}

/// Path slots (+0x9C: 78 × {x, y}).
pub const PATH_POINTS: usize = 78;
/// Saved steps (+0x1D8: 10 × {x, y}).
pub const SAVED_STEPS: usize = 10;
/// Default velocity at allocation (+0x7C).
pub const DEFAULT_VELOCITY: i32 = 0x800;
/// Player max path distance / IDA* start score.
pub const PLAYER_MAX_DISTANCE: u8 = 73;
pub const PLAYER_IDA_SCORE: u8 = 70;
/// Monster max path distance.
pub const MONSTER_MAX_DISTANCE: u8 = 14;
/// monstats `BaseId` of wraith1.
pub const WRAITH_BASE_ID: u32 = 38;

/// A path point or saved step, in sub-tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PathPoint {
    pub x: u16,
    pub y: u16,
}

impl PathPoint {
    /// Store a point: each coordinate cut to its u16 word.
    pub fn from_point(p: Point) -> Self {
        Self {
            x: p.x as u16,
            y: p.y as u16,
        }
    }

    /// Read a point: each word zero-extended.
    pub fn point(self) -> Point {
        Point::new(i32::from(self.x), i32::from(self.y))
    }
}

/// The target unit of a dynamic path (+0x58 unit, +0x5C type, +0x60
/// GUID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetUnit {
    pub unit: UnitId,
    pub ty: UnitType,
    pub guid: u32,
}

/// Which path record a unit type has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Static,
    Dynamic,
}

impl PathKind {
    /// Types 0 player, 1 monster, 3 missile: dynamic; 2, 4, 5: static.
    pub fn of(t: UnitType) -> Self {
        match t {
            UnitType::Player | UnitType::Monster | UnitType::Missile => Self::Dynamic,
            UnitType::Object | UnitType::Item | UnitType::Tile => Self::Static,
        }
    }
}

/// Static path (0x20 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticPath {
    /// +0x00.
    pub room: Option<RoomId>,
    /// +0x04, +0x08.
    pub client_x: i32,
    pub client_y: i32,
    /// +0x0C, +0x10.
    pub x: i32,
    pub y: i32,
    /// +0x1C.
    pub direction: u8,
    /// +0x1D, cleared by [`StaticPath::set`].
    pub room_changed: u8,
}

impl StaticPath {
    /// Room, position, client
    /// coordinates; room-changed flag := 0.
    pub fn set(&mut self, room: Option<RoomId>, x: i32, y: i32) {
        self.room = room;
        self.x = x;
        self.y = y;
        (self.client_x, self.client_y) = client_from_subtile(x, y);
        self.room_changed = 0;
    }
}

/// Dynamic movement state: positions, rooms, target, collision masks, velocity
/// and bounded point buffers. Defaults to zero values and absent references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicPath {
    /// +0x00, +0x04: 16.16 position.
    pub precise_x: u32,
    pub precise_y: u32,
    /// +0x08, +0x0C.
    pub client_x: i32,
    pub client_y: i32,
    /// +0x10, +0x12.
    pub target_x: u16,
    pub target_y: u16,
    /// +0x14, +0x16.
    pub prev_target_x: u16,
    pub prev_target_y: u16,
    /// +0x18, +0x1A.
    pub final_target_x: u16,
    pub final_target_y: u16,
    /// +0x1C.
    pub room: Option<RoomId>,
    /// +0x20.
    pub prev_room: Option<RoomId>,
    /// +0x24, +0x28.
    pub cur_point: u32,
    pub point_count: u32,
    /// +0x30.
    pub owner: Option<UnitId>,
    /// +0x34 ([`flags`]).
    pub flags: u32,
    /// +0x38: 15 after a velocity change, 0 after a new path.
    pub field_38: u32,
    /// +0x3C, +0x40.
    pub path_type: u32,
    pub prev_path_type: u32,
    /// +0x44.
    pub unit_size: i32,
    /// +0x48.
    pub pattern: u32,
    /// +0x4C.
    pub foot_mask: u16,
    /// +0x50.
    pub move_mask: u16,
    /// +0x54.
    pub collided_mask: u16,
    /// +0x58 unit, +0x5C type, +0x60 GUID.
    pub target_unit: Option<TargetUnit>,
    /// +0x64, +0x65, +0x66.
    pub direction: u8,
    pub new_direction: u8,
    pub turn_step: u8,
    /// +0x68.
    pub target_lead: u8,
    /// +0x6A, +0x6E (length 4096).
    pub dir_vec_x: i32,
    pub dir_vec_y: i32,
    /// +0x72, +0x76 (16.16 per tick).
    pub vel_vec_x: i32,
    pub vel_vec_y: i32,
    /// +0x7C, +0x80, +0x84, +0x88, +0x8C.
    pub velocity: i32,
    pub saved_velocity: i32,
    pub max_velocity: i32,
    pub acceleration: i32,
    pub accel_counter: i32,
    /// +0x90, +0x91, +0x92, +0x93.
    pub dist_budget: u8,
    pub max_distance: u8,
    pub ida_score: u8,
    pub stop_distance: u8,
    /// +0x94: monster re-path budget (read by the walk code;
    /// adjusted by the budget step, set by the budget setter).
    pub repath_budget: u8,
    /// +0x98.
    pub dir_offset: i32,
    /// +0x9C.
    pub points: [PathPoint; PATH_POINTS],
    /// +0x1D4, +0x1D8.
    pub saved_count: u32,
    pub saved_steps: [PathPoint; SAVED_STEPS],
}

impl Default for DynamicPath {
    /// The zeroed record.
    fn default() -> Self {
        Self {
            precise_x: 0,
            precise_y: 0,
            client_x: 0,
            client_y: 0,
            target_x: 0,
            target_y: 0,
            prev_target_x: 0,
            prev_target_y: 0,
            final_target_x: 0,
            final_target_y: 0,
            room: None,
            prev_room: None,
            cur_point: 0,
            point_count: 0,
            owner: None,
            flags: 0,
            field_38: 0,
            path_type: 0,
            prev_path_type: 0,
            unit_size: 0,
            pattern: 0,
            foot_mask: 0,
            move_mask: 0,
            collided_mask: 0,
            target_unit: None,
            direction: 0,
            new_direction: 0,
            turn_step: 0,
            target_lead: 0,
            dir_vec_x: 0,
            dir_vec_y: 0,
            vel_vec_x: 0,
            vel_vec_y: 0,
            velocity: 0,
            saved_velocity: 0,
            max_velocity: 0,
            acceleration: 0,
            accel_counter: 0,
            dist_budget: 0,
            max_distance: 0,
            ida_score: 0,
            stop_distance: 0,
            repath_budget: 0,
            dir_offset: 0,
            points: [PathPoint::default(); PATH_POINTS],
            saved_count: 0,
            saved_steps: [PathPoint::default(); SAVED_STEPS],
        }
    }
}

impl DynamicPath {
    /// Sub-tile x: the high word of precise x.
    pub fn x(&self) -> i32 {
        subtile_of(self.precise_x)
    }

    /// Sub-tile y.
    pub fn y(&self) -> i32 {
        subtile_of(self.precise_y)
    }

    /// The current sub-tile (high words of the precise position).
    pub fn cell(&self) -> Point {
        Point::new(self.x(), self.y())
    }

    /// Target point (+0x10, +0x12), zero-extended.
    pub fn target(&self) -> Point {
        PathPoint {
            x: self.target_x,
            y: self.target_y,
        }
        .point()
    }

    /// Writes +0x10/+0x12 only (cut to u16); the target unit is kept
    /// (unlike [`DynamicPath::set_target_point`]).
    pub fn put_target(&mut self, p: Point) {
        let q = PathPoint::from_point(p);
        (self.target_x, self.target_y) = (q.x, q.y);
    }

    /// Previous target (+0x14, +0x16).
    pub fn prev_target(&self) -> Point {
        PathPoint {
            x: self.prev_target_x,
            y: self.prev_target_y,
        }
        .point()
    }

    /// Writes +0x14/+0x16 (cut to u16).
    pub fn put_prev_target(&mut self, p: Point) {
        let q = PathPoint::from_point(p);
        (self.prev_target_x, self.prev_target_y) = (q.x, q.y);
    }

    /// Final target (+0x18, +0x1A).
    pub fn final_target(&self) -> Point {
        PathPoint {
            x: self.final_target_x,
            y: self.final_target_y,
        }
        .point()
    }

    /// Writes +0x18/+0x1A (cut to u16).
    pub fn put_final_target(&mut self, p: Point) {
        let q = PathPoint::from_point(p);
        (self.final_target_x, self.final_target_y) = (q.x, q.y);
    }

    /// Path point `i` (+0x9C + 4·i), zero-extended.
    pub fn point(&self, i: usize) -> Point {
        self.points[i].point()
    }

    /// The live points `points[0..point_count]`, the count read signed
    /// and clamped to 0..=78.
    pub fn live_points(&self) -> Vec<Point> {
        let n = (self.point_count as i32).clamp(0, PATH_POINTS as i32) as usize;
        self.points[..n].iter().map(|p| p.point()).collect()
    }

    /// Client coordinates from the precise position.
    pub fn update_client(&mut self) {
        (self.client_x, self.client_y) = client_from_precise(self.precise_x, self.precise_y);
    }

    /// Set the target point: +0x10/+0x12 and clear the
    /// target unit (+0x58).
    pub fn set_target_point(&mut self, x: u16, y: u16) {
        self.target_x = x;
        self.target_y = y;
        self.target_unit = None;
    }

    /// Re-path budget setter: a value
    /// above 255 is a fatal error.
    pub fn set_repath_budget(&mut self, value: u32) -> Result<(), PathError> {
        self.repath_budget = u8::try_from(value).map_err(|_| PathError::RepathBudget(value))?;
        Ok(())
    }

    /// Re-path budget adjust: budget +=
    /// `delta`, clamped to 0..=255.
    pub fn add_repath_budget(&mut self, delta: i32) {
        self.repath_budget = (i32::from(self.repath_budget) + delta).clamp(0, 255) as u8;
    }

    /// Set the path type. Fatal asserts
    /// are errors: a player taking type 2; a previous type of 11 or 8;
    /// type 4 with max distance ≥ 78.
    pub fn set_path_type(
        &mut self,
        tables: &PathTables,
        is_player: bool,
        t: u32,
    ) -> Result<(), PathError> {
        let i = t as usize;
        let (Some(&tf), Some(&off)) = (tables.pathtype_flags.get(i), tables.pathtype_diroff.get(i))
        else {
            return Err(PathError::PathType(t));
        };
        if is_player && t == path_types::TOWARD {
            return Err(PathError::PathType(t));
        }
        if tf & flags::SAVE_PREV_TYPE != 0 && self.flags & flags::PREV_TYPE_KEPT == 0 {
            self.prev_path_type = self.path_type;
        }
        if tf & flags::SAVE_VELOCITY != 0 && self.flags & flags::VELOCITY_KEPT == 0 {
            self.saved_velocity = self.velocity;
        }
        self.flags = (self.flags & !flags::PATH_TYPE_BITS) | tf;
        self.path_type = t;
        self.dir_offset = off;
        if matches!(
            self.prev_path_type,
            path_types::KNOCKBACK_SERVER | path_types::KNOCKBACK_CLIENT
        ) || (t == path_types::MISSILE && usize::from(self.max_distance) >= PATH_POINTS)
        {
            return Err(PathError::PathType(t));
        }
        Ok(())
    }
}

/// A unit's path record, if any (unit +0x2C).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitPath {
    Static(StaticPath),
    Dynamic(Box<DynamicPath>),
}

impl UnitPath {
    /// Position: static x/y at +0x0C/+0x10, dynamic the sub-tile
    /// words.
    pub fn position(&self) -> (i32, i32) {
        match self {
            Self::Static(p) => (p.x, p.y),
            Self::Dynamic(p) => (p.x(), p.y()),
        }
    }

    /// Room of the unit: static +0x00, dynamic +0x1C.
    pub fn room(&self) -> Option<RoomId> {
        match self {
            Self::Static(p) => p.room,
            Self::Dynamic(p) => p.room,
        }
    }
}

/// Unit position getter: a unit without a path reads (0, 0).
pub fn unit_position(path: Option<&UnitPath>) -> (i32, i32) {
    path.map_or((0, 0), UnitPath::position)
}

/// Monster inputs to size, pattern and move mask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonsterShape {
    /// monstats2 `SizeX` (+0x08, signed).
    pub size_x: i32,
    /// monstats `BaseId`.
    pub base_id: u32,
    /// monstats `flying`, `opendoors`.
    pub flying: bool,
    pub open_doors: bool,
    /// monstats `npc`, `inTown`, and unit flags bit 31.
    pub npc: bool,
    pub in_town: bool,
    pub unit_flag_31: bool,
    /// monstats `interact`.
    pub interact: bool,
}

impl MonsterShape {
    /// Can be in town: `npc` or `inTown` or flag bit 31.
    pub fn can_be_in_town(&self) -> bool {
        self.npc || self.in_town || self.unit_flag_31
    }

    /// Move mask: `flying` → 0x1804, else `opendoors` →
    /// 0x3401, else 0x3C01.
    pub fn move_mask(&self) -> u16 {
        if self.flying {
            0x1804
        } else if self.open_doors {
            0x3401
        } else {
            masks::MONSTER_MOVE
        }
    }
}

/// Object inputs to size and footprint mask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectShape {
    /// objects `SizeX` (+0xD0), `SizeY` (+0xD4).
    pub size_x: u32,
    pub size_y: u32,
    pub is_door: bool,
    pub blocks_vis: bool,
    pub block_missile: bool,
    /// objects `SubClass`.
    pub sub_class: u32,
    /// objects `HasCollision0..7` (+0x120 + mode).
    pub has_collision: [bool; 8],
}

impl ObjectShape {
    /// `HasCollision[mode]` ≠ 0 (objects byte +0x120 +
    /// mode, no bound check). `ObjMode.txt` has modes 0–7 only, so
    /// a mode above 7 does not occur in the base game; it would read the
    /// next objects.txt fields (`IsAttackable0`, `Start0`, …), which this
    /// shape does not hold: such a mode reads as 0 here.
    pub fn collides_in(&self, mode: u32) -> bool {
        self.has_collision
            .get(mode as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Footprint mask: `SubClass` bit 4 returns
    /// 0x8000 at once, without the `BlockMissile` bit.
    pub fn foot_mask(&self) -> u16 {
        if !self.is_door {
            if self.sub_class & 4 != 0 {
                0x8000
            } else if self.block_missile {
                0x404
            } else {
                0x400
            }
        } else if self.blocks_vis {
            0x806
        } else if self.block_missile {
            0x804
        } else {
            0x400
        }
    }
}

/// Per-type inputs to size and pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitShape {
    Player,
    Monster(MonsterShape),
    Object(ObjectShape),
    /// missiles `Size` (+0x18A).
    Missile {
        size: i32,
    },
    Item,
    Tile,
}

impl UnitShape {
    /// Unit size: player 2, monster monstats2 `SizeX`,
    /// object `SizeX`, missile `Size`, item 1, tile 0.
    pub fn size(&self) -> i32 {
        match self {
            Self::Player => 2,
            Self::Monster(m) => m.size_x,
            Self::Object(o) => o.size_x as i32,
            Self::Missile { size } => *size,
            Self::Item => 1,
            Self::Tile => 0,
        }
    }

    /// Footprint mask: players, monsters and missiles
    /// read their path (+0x4C); objects from the table; item 0x200; tile
    /// 0x1.
    pub fn foot_mask(&self, path: Option<&DynamicPath>) -> u16 {
        match self {
            Self::Player | Self::Monster(_) | Self::Missile { .. } => {
                path.map_or(0, |p| p.foot_mask)
            }
            Self::Object(o) => o.foot_mask(),
            Self::Item => 0x200,
            Self::Tile => 0x1,
        }
    }
}

/// Pattern from size: `pattern_of_size` for sizes
/// 0..3, any other (unsigned compare, so negative too) → 1; a monster
/// that can be in town with `interact` clear gets 1 → 3 and 2 → 4.
pub fn pattern_of_size(tables: &PathTables, size: i32, shape: &UnitShape) -> u32 {
    let p = tables
        .pattern_of_size
        .get(size as u32 as usize)
        .copied()
        .unwrap_or(1);
    match shape {
        UnitShape::Monster(m) if m.can_be_in_town() && !m.interact => match p {
            1 => 3,
            2 => 4,
            other => other,
        },
        _ => p,
    }
}

/// The unit of a dynamic path allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicKind {
    Player,
    Monster(MonsterShape),
    /// missiles `Size`.
    Missile {
        size: i32,
    },
}

impl DynamicKind {
    fn shape(&self) -> UnitShape {
        match *self {
            Self::Player => UnitShape::Player,
            Self::Monster(m) => UnitShape::Monster(m),
            Self::Missile { size } => UnitShape::Missile { size },
        }
    }
}

/// Dynamic path allocation
/// Stamps the footprint when a room is given; the
/// caller then puts the unit in the room's unit list
/// (unit-list order), which happens before the client coordinates
/// and flag 0x10 of rule 6 (neither reads the list).
#[allow(clippy::too_many_arguments)]
pub fn alloc_dynamic_path<R: CollisionRooms + ?Sized>(
    tables: &PathTables,
    rooms: &mut R,
    kind: DynamicKind,
    owner: UnitId,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    set_0x10: bool,
) -> Result<DynamicPath, PathError> {
    // Rule 1.
    let mut p = DynamicPath {
        owner: Some(owner),
        ..DynamicPath::default()
    };
    // Rule 2.
    let shape = kind.shape();
    p.unit_size = shape.size();
    p.pattern = pattern_of_size(tables, p.unit_size, &shape);
    // Rule 3.
    p.precise_x = to_fp16_center(x);
    p.precise_y = to_fp16_center(y);
    p.velocity = DEFAULT_VELOCITY;
    p.room = room;
    p.saved_count = 1;
    p.saved_steps[0] = PathPoint {
        x: x as u16,
        y: y as u16,
    };
    // Rule 4.
    match kind {
        DynamicKind::Player => {
            p.foot_mask = 0x80;
            p.move_mask = masks::PLAYER_MOVE;
            p.set_path_type(tables, true, path_types::STRAIGHT)?;
            p.max_distance = PLAYER_MAX_DISTANCE;
            p.ida_score = PLAYER_IDA_SCORE;
        }
        DynamicKind::Monster(m) => {
            p.foot_mask = 0x100;
            p.flags &= !flags::PATH_TYPE_BITS;
            p.path_type = path_types::TOWARD;
            p.dir_offset = 0;
            if m.base_id == WRAITH_BASE_ID {
                p.pattern = 5;
                p.move_mask = 0x804;
            } else {
                p.move_mask = m.move_mask();
            }
            p.max_distance = MONSTER_MAX_DISTANCE;
        }
        DynamicKind::Missile { .. } => {
            p.foot_mask = 0;
            p.move_mask = 0;
            // Type 4 through set type: flags get the table's
            // 0x60000 (missile path, saved steps), direction offset 0.
            p.set_path_type(tables, false, path_types::MISSILE)?;
        }
    }
    // Rule 5: footprint (players and monsters by pattern, missiles
    // by size).
    if room.is_some() {
        match kind {
            DynamicKind::Missile { .. } => {
                super::footprint::stamp_size(rooms, room, x, y, p.unit_size, p.foot_mask)
            }
            _ => stamp_pattern(rooms, room, x, y, p.pattern, p.foot_mask),
        }
    }
    // Rule 6.
    p.update_client();
    if set_0x10 {
        p.flags |= flags::KEEP_TARGET;
    }
    Ok(p)
}
