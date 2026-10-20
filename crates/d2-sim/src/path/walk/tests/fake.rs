//! Seam fakes: rooms over one collision grid [0, W) × [0, H) (every room
//! reads and writes the same grid; the core's collision and footprint
//! path-core functions do the grid work), a unit
//! table, and [`Ctx`]: the one context object the walk code takes.

use std::collections::BTreeMap;

use crate::drlg::{CollisionGrid, TileRect};
use crate::game::Game;
use crate::path::collision::CollisionRooms;
use crate::path::footprint::{self, FootShape, Footprint, RemoveRule};
use crate::path::history::PositionHistory;
use crate::path::record::{alloc_dynamic_path, DynamicKind, DynamicPath};
use crate::path::tables::PathTables;
use crate::path::walk::resync::ResyncRing;
use crate::path::walk::seams::{PathWorld, Point, StartTarget, UsedSkill, WalkUnits};
use crate::rng::Seed;
use crate::tick::TickRate;
use crate::units::{ClientId, RoomId, UnitId, UnitType};

pub const WALL: u16 = 0x1;
pub const NO_ROOM: u16 = 0x27;
pub const ROOM: RoomId = RoomId(0);

pub struct FakeWorld {
    /// The one grid every room reads and writes.
    pub grid: CollisionGrid,
    pub town: bool,
    /// Rect of [`ROOM`]; the grid may be larger (cells of other rooms).
    pub room0: TileRect,
    pub paths: BTreeMap<UnitId, DynamicPath>,
    pub clients: BTreeMap<RoomId, Vec<ClientId>>,
    /// Rooms that are no longer rooms of the unit's act.
    pub other_act: Vec<RoomId>,
    /// Extra rooms: id → rect; cells outside the main room resolve here.
    pub rooms: BTreeMap<RoomId, TileRect>,
    pub log: Vec<String>,
}

impl FakeWorld {
    pub fn new(w: i32, h: i32) -> FakeWorld {
        FakeWorld {
            grid: CollisionGrid::new(TileRect::new(0, 0, w, h)),
            town: false,
            room0: TileRect::new(0, 0, w, h),
            paths: BTreeMap::new(),
            clients: BTreeMap::new(),
            other_act: Vec::new(),
            rooms: BTreeMap::new(),
            log: Vec::new(),
        }
    }

    pub fn wall(&mut self, x: i32, y: i32) {
        *self.grid.get_mut(x, y).expect("wall inside the grid") |= WALL;
    }

    /// The grid value of a cell (outside the grid: 0x27).
    pub fn value(&self, p: Point) -> u16 {
        self.grid.get(p.x, p.y).unwrap_or(NO_ROOM)
    }

    /// A player path as the path allocation makes it, footprint
    /// stamped.
    pub fn add_player(&mut self, t: &PathTables, unit: UnitId, x: i32, y: i32) {
        let p = alloc_dynamic_path(t, self, DynamicKind::Player, unit, Some(ROOM), x, y, false)
            .unwrap();
        self.paths.insert(unit, p);
    }
}

/// Every room's adjacency array: [`ROOM`] then the extra rooms in id
/// order (any cell of any room is reached from any room).
impl CollisionRooms for FakeWorld {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        if room == ROOM {
            Some(self.room0)
        } else {
            self.rooms.get(&room).copied()
        }
    }
    fn adjacent_count(&self, _room: RoomId) -> usize {
        1 + self.rooms.len()
    }
    fn adjacent(&self, _room: RoomId, i: usize) -> Option<RoomId> {
        if i == 0 {
            Some(ROOM)
        } else {
            self.rooms.keys().nth(i - 1).copied()
        }
    }
    fn grid(&self, _room: RoomId) -> Option<&CollisionGrid> {
        Some(&self.grid)
    }
    fn grid_mut(&mut self, _room: RoomId) -> Option<&mut CollisionGrid> {
        Some(&mut self.grid)
    }
}

fn footprint_of(p: &DynamicPath) -> Footprint {
    Footprint {
        room: p.room,
        x: p.x(),
        y: p.y(),
        shape: FootShape::Pattern(p.pattern),
        mask: p.foot_mask,
    }
}

/// The walk context: world, units and game in one object.
pub struct Ctx {
    pub w: FakeWorld,
    pub u: FakeUnits,
    pub g: Game,
}

impl Ctx {
    pub fn new(w: FakeWorld, u: FakeUnits) -> Ctx {
        Ctx {
            w,
            u,
            g: Game::new(),
        }
    }
}

impl CollisionRooms for Ctx {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.w.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.w.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.w.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.w.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.w.grid_mut(room)
    }
}

impl PathWorld for Ctx {
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath> {
        self.w.paths.get(&unit).cloned()
    }
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath) {
        self.w.paths.insert(unit, path.clone());
    }
    fn room_in_town(&self, _room: RoomId) -> bool {
        self.w.town
    }
    fn remove_footprint(&mut self, unit: UnitId, force: bool) -> bool {
        let Some(fp) = self.w.paths.get(&unit).map(footprint_of) else {
            return false;
        };
        footprint::remove_footprint(&mut self.w, &fp, RemoveRule::Other, force)
    }
    fn add_footprint(&mut self, unit: UnitId) {
        if let Some(fp) = self.w.paths.get(&unit).map(footprint_of) {
            footprint::add_footprint(&mut self.w, &fp);
        }
    }
    fn room_list_remove(&mut self, unit: UnitId, room: RoomId) {
        self.w.log.push(format!("leave {} {}", unit.0, room.0));
    }
    fn room_list_insert(&mut self, unit: UnitId, room: RoomId) {
        self.w.log.push(format!("insert {} {}", unit.0, room.0));
    }
    fn queue_for_update(&mut self, unit: UnitId) {
        self.w.log.push(format!("queue {}", unit.0));
    }
    fn room_clients(&self, room: RoomId) -> Vec<ClientId> {
        self.w.clients.get(&room).cloned().unwrap_or_default()
    }
    fn room_in_unit_act(&self, _: UnitId, room: RoomId) -> bool {
        !self.w.other_act.contains(&room)
    }
}

#[derive(Clone)]
pub struct FakeUnit {
    pub ty: UnitType,
    pub class: u32,
    pub guid: u32,
    pub mode: u32,
    pub pos: Point,
    pub size: i32,
    pub stats: BTreeMap<u16, i32>,
    pub item_stats: BTreeMap<u16, i32>,
    pub states: Vec<u16>,
    pub state_stats: BTreeMap<(u16, u16), i32>,
    pub used_skill: Option<UsedSkill>,
    pub seed: Seed,
    pub cursor: bool,
    pub torso_speed: Option<i32>,
    pub run_bonus: i32,
}

impl FakeUnit {
    pub fn player() -> FakeUnit {
        FakeUnit {
            ty: UnitType::Player,
            class: 0,
            guid: 1,
            mode: 1,
            pos: Point::default(),
            size: 2,
            stats: BTreeMap::from([(10, 100 << 8), (67, 100)]),
            item_stats: BTreeMap::new(),
            states: Vec::new(),
            state_stats: BTreeMap::new(),
            used_skill: None,
            seed: Seed::default(),
            cursor: false,
            torso_speed: None,
            run_bonus: 0,
        }
    }
}

#[derive(Default)]
pub struct FakeUnits {
    pub units: BTreeMap<UnitId, FakeUnit>,
    pub walk_velocity: (i32, i32, i32),
    pub type1_expire: i32,
    pub log: Vec<String>,
    pub client_players: BTreeMap<ClientId, UnitId>,
    /// Position histories kept per unit (none: not kept).
    pub history: BTreeMap<UnitId, PositionHistory>,
    /// 0x5F seams.
    pub has_client: bool,
    pub dead: bool,
    pub place_ok: bool,
    pub ring: ResyncRing,
    pub game_type: u8,
    /// Door orientation of every unit (`None`: not a door).
    pub door: Option<bool>,
    /// Monsters may not be in town (town test false).
    pub no_town: bool,
}

impl FakeUnits {
    pub fn with_player(unit: UnitId) -> FakeUnits {
        FakeUnits {
            units: BTreeMap::from([(unit, FakeUnit::player())]),
            walk_velocity: (6, 9, 20),
            has_client: true,
            ..FakeUnits::default()
        }
    }
    pub fn unit(&mut self, u: UnitId) -> &mut FakeUnit {
        self.units.get_mut(&u).unwrap()
    }
}

impl WalkUnits for Ctx {
    fn frame(&self) -> i32 {
        self.g.frame
    }
    fn tick_rate(&self) -> TickRate {
        self.g.tick_rate
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        self.u.units[&unit].ty
    }
    fn class(&self, unit: UnitId) -> u32 {
        self.u.units[&unit].class
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.u.units[&unit].guid
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.u.units[&unit].mode
    }
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.u
            .units
            .iter()
            .find(|(_, u)| u.ty == ty && u.guid == guid)
            .map(|(id, _)| *id)
    }
    fn position(&self, unit: UnitId) -> Point {
        self.u.units[&unit].pos
    }
    fn unit_size(&self, unit: UnitId) -> i32 {
        self.u.units[&unit].size
    }
    fn first_type1_expire(&self, _unit: UnitId) -> i32 {
        self.u.type1_expire
    }
    fn has_cursor_item(&self, unit: UnitId) -> bool {
        self.u.units[&unit].cursor
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.u.units[&unit].states.contains(&state)
    }
    fn state_stat(&self, unit: UnitId, state: u16, stat: u16) -> i32 {
        self.u.units[&unit]
            .state_stats
            .get(&(state, stat))
            .copied()
            .unwrap_or(0)
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        let u = &self.u.units[&unit];
        let base = u.stats.get(&stat).copied().unwrap_or(0);
        if stat == 67 {
            base + u.run_bonus
        } else {
            base
        }
    }
    fn item_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.u.units[&unit]
            .item_stats
            .get(&stat)
            .copied()
            .unwrap_or(0)
    }
    fn add_base_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        *self.u.unit(unit).stats.entry(stat).or_insert(0) += delta;
    }
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.u.unit(unit).stats.insert(stat, value);
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        &mut self.u.unit(unit).seed
    }
    fn used_skill(&self, unit: UnitId) -> Option<UsedSkill> {
        self.u.units[&unit].used_skill
    }
    fn set_used_skill(&mut self, unit: UnitId, skill: Option<u16>) {
        self.u.log.push(format!("skill {skill:?}"));
        if skill.is_none() {
            self.u.unit(unit).used_skill = None;
        }
    }
    fn clear_queued_action(&mut self, _unit: UnitId) {
        self.u.log.push("clear queued".into());
    }
    fn set_mode(&mut self, unit: UnitId, mode: u32) {
        self.u.log.push(format!("mode {mode}"));
        let u = self.u.unit(unit);
        u.mode = mode;
        // The run stat list is freed at the next mode set.
        u.run_bonus = 0;
    }
    fn cancel_events(&mut self, _unit: UnitId, ty: u8) {
        self.u.log.push(format!("cancel {ty}"));
    }
    fn schedule_event0(&mut self, _unit: UnitId) {
        self.u.log.push("event0".into());
    }
    fn start_other_mode(&mut self, unit: UnitId, mode: u32, _target: StartTarget) {
        self.u.log.push(format!("start {mode}"));
        self.u.unit(unit).mode = mode;
    }
    fn attach_run_stats(&mut self, unit: UnitId, value: i32) {
        self.u.log.push(format!("run list {value}"));
        self.u.unit(unit).run_bonus = value;
    }
    fn charstats_velocity(&self, _unit: UnitId) -> (i32, i32, i32) {
        self.u.walk_velocity
    }
    fn torso_speed(&self, unit: UnitId) -> Option<i32> {
        self.u.units[&unit].torso_speed
    }
    fn door_orientation(&self, _unit: UnitId) -> Option<bool> {
        self.u.door
    }
    fn monster_can_be_in_town(&self, _unit: UnitId) -> bool {
        !self.u.no_town
    }
    fn client_player(&self, client: ClientId) -> Option<UnitId> {
        self.u.client_players.get(&client).copied()
    }
    fn send_unit_removal(&mut self, client: ClientId, unit: UnitId) {
        self.u
            .log
            .push(format!("remove {} to {}", unit.0, client.0));
    }
    fn send_unit_add(&mut self, client: ClientId, unit: UnitId) {
        self.u.log.push(format!("add {} to {}", unit.0, client.0));
    }
    fn clear_ai_room_memo(&mut self, unit: UnitId) {
        self.u.log.push(format!("memo {}", unit.0));
    }
    fn state13_step(&mut self, _unit: UnitId) {
        self.u.log.push("state13".into());
    }
    fn position_history(&mut self, unit: UnitId) -> Option<&mut PositionHistory> {
        self.u.history.get_mut(&unit)
    }
    fn has_client(&self, _unit: UnitId) -> bool {
        self.u.has_client
    }
    fn is_dead(&self, _unit: UnitId) -> bool {
        self.u.dead
    }
    fn place_resync(&mut self, unit: UnitId, x: i32, y: i32) -> bool {
        self.u.log.push(format!("place {} ({x},{y})", unit.0));
        self.u.place_ok
    }
    fn resync_ring(&mut self, _unit: UnitId) -> Option<&mut ResyncRing> {
        Some(&mut self.u.ring)
    }
    fn game_type(&self) -> u8 {
        self.u.game_type
    }
    fn resync_lock(&mut self, _unit: UnitId, expire: i32) {
        self.u.log.push(format!("lock {expire}"));
    }
    fn send_to_client(&mut self, _unit: UnitId, bytes: &[u8]) {
        self.u.log.push(format!("send {bytes:02x?}"));
    }
}
