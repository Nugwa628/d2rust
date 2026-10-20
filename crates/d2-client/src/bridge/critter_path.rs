//! The client path of a C monster (a critter of the room pass):
//! its dynamic path record, the walk request (code 0x01 of the
//! monster mode machine), and the path half of the
//! monster update (path step, the turn,
//! the mode end of a client-only unit).
//!
//! The path code is `d2-sim`'s (the base-game client calls the same
//! as the server) over the
//! client act's active rooms, with the unit footprints of the model on
//! the footprint grids (`client_missiles::stamp_unit_footprints`,
//! settled: a capture of the client grid of a walking Fallen has no
//! footprint difference over 12,329 unit records; idle units: a town
//! capture matches over 40 frames). The records live in `objclient.c_paths`, keyed
//! like set C.

use std::collections::BTreeMap;

use d2_sim::drlg::{CollisionGrid, Drlg, TileRect};
use d2_sim::path::coords::{to_fp16_center, Point};
use d2_sim::path::footprint::{
    add_footprint, remove_footprint, teleport_and_clear, FootShape, Footprint, PathMotion,
    RemoveRule,
};
use d2_sim::path::record::{
    alloc_dynamic_path, DynamicKind, DynamicPath, MonsterShape, TargetUnit,
};
use d2_sim::path::tables::PathTables;
use d2_sim::path::walk::find::compute;
use d2_sim::path::walk::velocity::{mode_velocity, set_velocity, STAT_VELOCITYPERCENT};
use d2_sim::path::walk::{PathWorld, Walk, WalkUnits};
use d2_sim::path::CollisionRooms;
use d2_sim::rng::Seed;
use d2_sim::units::{ClientId, RoomId, UnitId, UnitType};

use super::world::{view_key, ClientUnit, ClientWorld, ModelInputs, MonsterClass, UnitKey};

/// A monster the path code works on: a C critter, or a server (set S)
/// monster whose client path the single-player server copy keeps.
/// The records live in `objclient.c_paths` (C) and
/// `objclient.s_paths` (S).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Who {
    pub key: UnitKey,
    pub server: bool,
}

impl Who {
    pub fn server(key: UnitKey) -> Self {
        Who { key, server: true }
    }
}

impl From<UnitKey> for Who {
    fn from(key: UnitKey) -> Self {
        Who { key, server: false }
    }
}

fn unit_ref(w: &ClientWorld, k: Who) -> Option<&ClientUnit> {
    if k.server {
        w.units.get(&k.key)
    } else {
        w.objclient.set_c.get(&k.key)
    }
}

fn unit_mut(w: &mut ClientWorld, k: Who) -> Option<&mut ClientUnit> {
    if k.server {
        w.units.get_mut(&k.key)
    } else {
        w.objclient.set_c.get_mut(&k.key)
    }
}

fn path_ref(w: &ClientWorld, k: Who) -> Option<&DynamicPath> {
    if k.server {
        w.objclient.s_paths.get(&k.key)
    } else {
        w.objclient.c_paths.get(&k.key)
    }
}

fn path_mut(w: &mut ClientWorld, k: Who) -> Option<&mut DynamicPath> {
    if k.server {
        w.objclient.s_paths.get_mut(&k.key)
    } else {
        w.objclient.c_paths.get_mut(&k.key)
    }
}

fn path_take(w: &mut ClientWorld, k: Who) -> Option<DynamicPath> {
    if k.server {
        w.objclient.s_paths.remove(&k.key)
    } else {
        w.objclient.c_paths.remove(&k.key)
    }
}

fn path_put(w: &mut ClientWorld, k: Who, p: DynamicPath) {
    if k.server {
        w.objclient.s_paths.insert(k.key, p);
    } else {
        w.objclient.c_paths.insert(k.key, p);
    }
}

/// The C monster in its path context.
const ME: UnitId = UnitId(1);
/// The movement's base: is always 0, so 0x400.
const BASE: i32 = 0x400;
/// Unit flag-ex 0x2000, the walk flag W(f).
const WALK_FLAG: u32 = 0x2000;

/// The client act's active rooms over the footprint grids.
struct Rooms<'a> {
    drlg: &'a Drlg,
    grids: &'a mut BTreeMap<RoomId, CollisionGrid>,
}

impl CollisionRooms for Rooms<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        let r = self.drlg.drlg_room_of(room)?;
        self.drlg.active_room(r).map(|a| a.subtiles)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.drlg
            .drlg_room_of(room)
            .and_then(|r| self.drlg.active_room(r))
            .map_or(0, |a| a.adjacency.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        let r = self.drlg.drlg_room_of(room)?;
        let n = *self.drlg.active_room(r)?.adjacency.get(i)?;
        self.drlg.active_room(n).map(|a| a.id)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        if let Some(g) = self.grids.get(&room) {
            return Some(g);
        }
        let r = self.drlg.drlg_room_of(room)?;
        self.drlg.active_room(r).map(|a| &a.collision)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        if !self.grids.contains_key(&room) {
            let r = self.drlg.drlg_room_of(room)?;
            let g = self.drlg.active_room(r)?.collision.clone();
            self.grids.insert(room, g);
        }
        self.grids.get_mut(&room)
    }
}

/// The path context of one C monster: its rooms, its footprint as it
/// stands, and what the velocity and compute read of it.
struct Ctx<'a> {
    rooms: Rooms<'a>,
    /// The game's tick rate ([`ClientWorld::tick_rate`]).
    rate: d2_sim::tick::TickRate,
    frame: i32,
    foot: Footprint,
    class: u32,
    mode: u32,
    percent: i32,
    velocity: i32,
    npc: bool,
    in_town: bool,
    seed: Seed,
    /// The path's target unit (+0x58) as the client model holds it: its
    /// type, cell and unit size ([`TARGET`]).
    target: Option<(UnitType, Point, i32)>,
    /// The target unit's footprint as stamped on the client grids: the
    /// compute's step 6 lifts it while the path is built when the path
    /// has flag 0x800.
    target_foot: Option<Footprint>,
}

/// The id the path context gives the path's target unit.
const TARGET: UnitId = UnitId(2);

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
    fn load_path(&self, _: UnitId) -> Option<DynamicPath> {
        None
    }
    fn store_path(&mut self, _: UnitId, _: &DynamicPath) {}
    fn room_in_town(&self, room: RoomId) -> bool {
        let Some(r) = self.rooms.drlg.drlg_room_of(room) else {
            return false;
        };
        let d = self.rooms.drlg;
        d2_sim::drlg::is_town(d.level(d.room(r).level).id)
    }
    fn remove_footprint(&mut self, unit: UnitId, force: bool) -> bool {
        let fp = match unit {
            ME => self.foot,
            TARGET => match self.target_foot {
                Some(fp) => fp,
                None => return false,
            },
            _ => return false,
        };
        remove_footprint(&mut self.rooms, &fp, RemoveRule::Other, force)
    }
    fn add_footprint(&mut self, unit: UnitId) {
        let fp = match unit {
            ME => self.foot,
            TARGET => match self.target_foot {
                Some(fp) => fp,
                None => return,
            },
            _ => return,
        };
        add_footprint(&mut self.rooms, &fp);
    }
    fn room_list_remove(&mut self, _: UnitId, _: RoomId) {}
    fn room_list_insert(&mut self, _: UnitId, _: RoomId) {}
    fn queue_for_update(&mut self, _: UnitId) {}
    fn room_clients(&self, _: RoomId) -> Vec<ClientId> {
        Vec::new()
    }
}

impl WalkUnits for Ctx<'_> {
    /// The client's copy of a monster path steps once per server tick,
    /// at the game's rate.
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.rate
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        match self.target {
            Some((ty, _, _)) if unit == TARGET => ty,
            _ => UnitType::Monster,
        }
    }
    fn position(&self, unit: UnitId) -> Point {
        match self.target {
            Some((_, p, _)) if unit == TARGET => p,
            _ => Point::default(),
        }
    }
    /// The target's size: the arrival test measures the
    /// distance with it.
    fn unit_size(&self, unit: UnitId) -> i32 {
        match self.target {
            Some((_, _, size)) if unit == TARGET => size,
            _ => 0,
        }
    }
    fn class(&self, _: UnitId) -> u32 {
        self.class
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn mode(&self, _: UnitId) -> u32 {
        self.mode
    }
    fn stat(&self, _: UnitId, stat: u16) -> i32 {
        if stat == STAT_VELOCITYPERCENT {
            self.percent
        } else {
            0
        }
    }
    fn seed(&mut self, _: UnitId) -> &mut Seed {
        &mut self.seed
    }
    fn monstats_velocity(&self, _: UnitId) -> (i32, bool) {
        (self.velocity, self.npc)
    }
    fn monster_can_be_in_town(&self, _: UnitId) -> bool {
        self.npc || self.in_town
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

fn row(inputs: &ModelInputs, class: u32) -> Option<MonsterClass> {
    inputs
        .tables
        .monsters
        .get(class as usize)
        .copied()
        .flatten()
}

/// The unit size of a path's
/// target unit: player 2, monster `monstats2` `SizeX`, object `SizeX`,
/// missile `missiles` `Size` (row +0x18A), item 1, else 0.
fn target_size(inputs: &ModelInputs, u: &ClientUnit) -> i32 {
    match u.key.unit_type {
        super::world::MONSTER => row(inputs, u.class).map_or(0, |r| i32::from(r.size_x)),
        super::world::MISSILE => inputs
            .tables
            .missiles
            .get(u.class as usize)
            .map_or(0, |r| i32::from(r.size)),
        _ => super::objects::unit_size(u, &inputs.objclient.rows),
    }
}

/// The footprint of `path`'s target unit as the client grids hold it,
/// which the compute lifts in its step 6
/// (path flag 0x800): the unit layer's mark (a player, an object), else
/// the target's own client path footprint (a C monster); `None` without a
/// target or a footprint.
fn target_footprint(w: &ClientWorld, path: &DynamicPath) -> Option<Footprint> {
    let t = path.target_unit?;
    let k = UnitKey::new(t.ty as u8, t.guid);
    w.objclient
        .foot_marks
        .get(&k)
        .copied()
        .or_else(|| path_ref(w, Who::server(k)).map(footprint_of))
}

/// Runs `f` on the path of `key` in its context. `None`: no client DRLG,
/// no path, no unit or no class row.
fn with_ctx<R>(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: Who,
    f: impl FnOnce(&PathTables, &mut Ctx<'_>, &mut DynamicPath) -> R,
) -> Option<R> {
    let t = super::predict::path_tables()?;
    let u = unit_ref(w, key)?;
    let r = row(inputs, u.class)?;
    let (class, mode) = (u.class, u.mode);
    let percent = u.stats.get(&STAT_VELOCITYPERCENT).copied().unwrap_or(0);
    let mut path = path_take(w, key)?;
    // The target unit's type and cell, read before the rooms are lent.
    let target = path.target_unit.and_then(|t| {
        let k = UnitKey::new(t.ty as u8, t.guid);
        let u = w.units.get(&k).or_else(|| w.objclient.set_c.get(&k))?;
        let (x, y) = u.position?;
        Some((
            t.ty,
            Point::new(i32::from(x), i32::from(y)),
            target_size(inputs, u),
        ))
    });
    let target_foot = target_footprint(w, &path);
    let frame = if w.tick_rate.is_base() {
        0
    } else {
        w.share_frame()
    };
    let Some(d) = w.drlg.as_ref() else {
        path_put(w, key, path);
        return None;
    };
    let mut c = Ctx {
        rooms: Rooms {
            drlg: &d.drlg,
            grids: &mut w.objclient.unit_grids,
        },
        rate: w.tick_rate,
        frame,
        foot: footprint_of(&path),
        class,
        mode,
        percent,
        velocity: r.setup.map_or(0, |s| i32::from(s.velocity)),
        npc: r.npc,
        in_town: r.in_town,
        seed: Seed::default(),
        target,
        target_foot,
    };
    let out = f(t, &mut c, &mut path);
    path_put(w, key, path);
    Some(out)
}

/// The path inputs of a monster of class `class`:
/// the move mask is the server's, from `monstats` `flying` and
/// `opendoors`.
fn monster_shape(r: &MonsterClass, class: u32) -> MonsterShape {
    MonsterShape {
        size_x: i32::from(r.size_x),
        base_id: class,
        npc: r.npc,
        in_town: r.in_town,
        interact: r.interact,
        flying: r.flying,
        open_doors: r.open_doors,
        ..MonsterShape::default()
    }
}

/// The path of a monster at (x, y) in `room`, velocity := `monstats`
/// `Velocity` << 8 (set-up); its footprint is stamped.
fn alloc(w: &mut ClientWorld, key: Who, r: &MonsterClass, room: RoomId, x: i32, y: i32) -> bool {
    let Some(t) = super::predict::path_tables() else {
        return false;
    };
    let class = unit_ref(w, key).map_or(0, |u| u.class);
    let Some(d) = w.drlg.as_ref() else {
        return false;
    };
    let mut rooms = Rooms {
        drlg: &d.drlg,
        grids: &mut w.objclient.unit_grids,
    };
    let shape = monster_shape(r, class);
    let Ok(mut path) = alloc_dynamic_path(
        t,
        &mut rooms,
        DynamicKind::Monster(shape),
        ME,
        Some(room),
        x,
        y,
        true,
    ) else {
        return false;
    };
    set_velocity(&mut path, r.setup.map_or(0, |s| i32::from(s.velocity)) << 8);
    path_put(w, key, path);
    true
}

/// The set-up of a critter made by the room pass
/// (through the creator): the table stats, the path at (x, y) in `room` with
/// velocity := `monstats` `Velocity` << 8 (its footprint is
/// stamped), mode 1 NU (frame 0, the count and the rate), the
/// first frame from the unit seed, the flags, the initial
/// direction and the creator's stop distance (`monstats2` +0x0E).
pub fn setup(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: impl Into<Who>,
    room: RoomId,
    x: i32,
    y: i32,
) {
    let key = key.into();
    let Some(r) = unit_ref(w, key).and_then(|u| row(inputs, u.class)) else {
        return;
    };
    let (difficulty, expansion) = (w.difficulty, w.expansion != 0);
    if let (Some(u), Some(s)) = (unit_mut(w, key), &r.setup) {
        super::msg::units::setup_stats(u, s, difficulty, expansion);
    }
    if !alloc(w, key, &r, room, x, y) {
        return;
    }
    mode_set(w, inputs, key, 1, true);
    let Some(u) = unit_mut(w, key) else {
        return;
    };
    // The first frame.
    if let Some((lo, hi)) = u.seed {
        let mut s = Seed::new(lo, hi);
        u.frame = s.roll(u.frame_count) as i32;
        u.seed = Some((s.lo, s.hi));
    }
    if let Some(s) = &r.setup {
        super::msg::units::setup_flags(u, s);
    }
    // No direction draw for an `npc`; else one step of the unit
    // seed when the class has mode 2 WL, low 6 bits.
    // The base-class and class overrides name no critter.
    if r.npc {
        return;
    }
    let mut dir = 0;
    if r.modes & (1 << 2) != 0 {
        if let Some((lo, hi)) = u.seed {
            let mut s = Seed::new(lo, hi);
            dir = (s.step() & 0x3F) as u8;
            u.seed = Some((s.lo, s.hi));
        }
    }
    let stop = r.path_byte;
    if let Some(p) = path_mut(w, key) {
        snap(p, dir);
        // The stop distance set with (path, n): n − 1 for n in 1…19, else 0.
        p.stop_distance = if (1..=19).contains(&stop) {
            stop - 1
        } else {
            0
        };
    }
}

/// With (path, d): direction := new direction := d & 63.
fn snap(p: &mut DynamicPath, d: u8) {
    p.direction = d & 63;
    p.new_direction = d & 63;
}

/// The mode set of a C monster: the
/// same mode restarts nothing unless `restart`; a new one
/// sets frame 0, the mode's count and the rate with its
/// velocity half (a mode without the modifier
/// keeps the path's velocity).
fn mode_set(w: &mut ClientWorld, inputs: &ModelInputs, key: Who, mode: u32, restart: bool) {
    let Some(u) = unit_mut(w, key) else {
        return;
    };
    if u.mode == mode && !restart {
        return;
    }
    u.mode = mode;
    let r = row(inputs, u.class);
    let frames = r
        .and_then(|c| c.anims.get(mode as usize).copied().flatten())
        .map_or(0, |(f, _)| f as i32);
    u.frame = 0;
    u.frame_count = frames << 8;
    let speed = super::monster_anim::rate_of(u, inputs, |s| u.stats.get(&s).copied().unwrap_or(0));
    u.speed = Some(speed);
    if let Some(v) = with_ctx(w, inputs, key, |t, c, _| mode_velocity(t, c, ME, mode)).flatten() {
        if let Some(p) = path_mut(w, key) {
            set_velocity(p, v);
        }
    }
}

/// The neutral fallback F: a monster in mode 1…15
/// other than 12 → path stop and mode set 1; any other mode: nothing.
/// The path stop ends the path and puts
/// the precise position on the centre of the cell it is in
/// (`(p & 0xFFFF0000) + 0x8000` per axis).
fn neutral(w: &mut ClientWorld, inputs: &ModelInputs, key: Who) {
    let Some(u) = unit_ref(w, key) else {
        return;
    };
    if !(1..=15).contains(&u.mode) || u.mode == 12 {
        return;
    }
    if let Some(p) = path_mut(w, key) {
        p.point_count = 0;
        p.precise_x = (p.precise_x & 0xFFFF_0000) + 0x8000;
        p.precise_y = (p.precise_y & 0xFFFF_0000) + 0x8000;
        p.update_client();
    }
    sync_position(w, key);
    mode_set(w, inputs, key, 1, false);
}

/// W(f): flag-ex 0x2000 := f and path +0x38 := 0.
fn walk_flag(w: &mut ClientWorld, key: Who, f: bool) {
    if let Some(u) = unit_mut(w, key) {
        if f {
            u.flag_ex |= WALK_FLAG;
        } else {
            u.flag_ex &= !WALK_FLAG;
        }
    }
    if let Some(p) = path_mut(w, key) {
        p.field_38 = 0;
    }
}

/// The path cell of a C monster.
pub fn cell(w: &ClientWorld, key: impl Into<Who>) -> Option<(i32, i32)> {
    let key = key.into();
    path_ref(w, key).map(|p| (p.x(), p.y()))
}

/// The path velocity (+0x7C) of a monster's client path.
pub fn velocity(w: &ClientWorld, key: impl Into<Who>) -> Option<i32> {
    let key = key.into();
    path_ref(w, key).map(|p| p.velocity)
}

/// The path direction (+0x64) of a C monster: the direction it draws
/// with.
pub fn direction(w: &ClientWorld, key: impl Into<Who>) -> Option<u8> {
    let key = key.into();
    path_ref(w, key).map(|p| p.direction)
}

/// Writes the path direction (+0x64) and the new direction of a
/// monster's client path (the draw's snap write-back);
/// false when the unit has no client path.
pub fn set_direction(w: &mut ClientWorld, key: impl Into<Who>, dir: u8) -> bool {
    let key = key.into();
    let Some(p) = path_mut(w, key) else {
        return false;
    };
    p.direction = dir & 0x3F;
    p.new_direction = dir & 0x3F;
    true
}

/// The path's move mask (+0x50) of a C monster: "U's move mask" of the
/// critter AI's blocked-cell test.
pub fn move_mask(w: &ClientWorld, key: impl Into<Who>) -> Option<u16> {
    let key = key.into();
    path_ref(w, key).map(|p| p.move_mask)
}

/// The path target point (+0x10, +0x12) of a monster's client path.
pub fn target(w: &ClientWorld, key: impl Into<Who>) -> Option<(u16, u16)> {
    let key = key.into();
    path_ref(w, key).map(|p| (p.target_x, p.target_y))
}

/// The path target point (+0x10, +0x12) of a monster's client path :=
/// (x, y): the 0x15 skill start.
pub fn set_target(w: &mut ClientWorld, key: impl Into<Who>, x: u16, y: u16) {
    let key = key.into();
    if let Some(p) = path_mut(w, key) {
        p.target_x = x;
        p.target_y = y;
    }
}

/// The path's target unit (+0x58) of a monster's client path.
pub fn target_unit(w: &ClientWorld, key: impl Into<Who>) -> Option<UnitKey> {
    let key = key.into();
    path_ref(w, key)
        .and_then(|p| p.target_unit)
        .map(|t| UnitKey::new(t.ty as u8, t.guid))
}

/// Code 0x07 at the unit's own position: W(0);
/// |Δ| ≤ 1 on both axes → the neutral fallback.
pub fn idle(w: &mut ClientWorld, inputs: &ModelInputs, key: impl Into<Who>) {
    let key = key.into();
    walk_flag(w, key, false);
    neutral(w, inputs, key);
}

/// Code 0x01, walk to the point (x, y): W(1); target := the
/// point; compute with town access = unit
/// flag 0x200000 (set on every client-only unit); no point → the neutral
/// fallback, else mode set 2. Then the tail: ≠ 0 and the
/// stat-67 total ≠ → stat 67 :=, the rate again.
pub fn walk_to(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: impl Into<Who>,
    x: i32,
    y: i32,
    r4: i32,
) {
    let key = key.into();
    walk_flag(w, key, true);
    let n = with_ctx(w, inputs, key, |t, c, p| {
        p.set_target_point(x as u16, y as u16);
        let n = compute(t, c, p, ME, true).unwrap_or(0);
        c.foot = footprint_of(p);
        n
    });
    if n.unwrap_or(0) == 0 {
        neutral(w, inputs, key);
    } else {
        mode_set(w, inputs, key, 2, false);
    }
    tail(w, inputs, key, r4);
}

/// A point-group code setting `mode` at the point
/// (x, y): W(0); target := none; a point other than (0, 0) and the
/// unit's own cell → face it; mode set; the mode's row2
/// move bit → path to the point (as
/// [`walk_to`]; the result is ignored); then the tail.
#[allow(clippy::too_many_arguments)]
pub fn point_mode(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: impl Into<Who>,
    mode: u32,
    x: i32,
    y: i32,
    r4: i32,
) {
    let key = key.into();
    walk_flag(w, key, false);
    if let Some(p) = path_mut(w, key) {
        p.target_unit = None;
    }
    let (px, py) = ((x & 0xFFFF) as u16, (y & 0xFFFF) as u16);
    let own = unit_ref(w, key).and_then(|u| u.position);
    if (x != 0 || y != 0) && own != Some((px, py)) {
        face_c(w, key, px, py);
    }
    mode_set(w, inputs, key, mode, false);
    if moves(w, inputs, key, mode) {
        with_ctx(w, inputs, key, |t, c, p| {
            p.set_target_point(px, py);
            let _ = compute(t, c, p, ME, true);
            c.foot = footprint_of(p);
        });
    }
    tail(w, inputs, key, r4);
}

/// W(0), target := none, mode set `mode`, then the tail: what D2RUST runs
/// of a request outside the point group (the death code 8 of a C
/// monster, `critters::rat_ai`).
pub fn mode_only(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: impl Into<Who>,
    mode: u32,
    r4: i32,
) {
    let key = key.into();
    walk_flag(w, key, false);
    if let Some(p) = path_mut(w, key) {
        p.target_unit = None;
    }
    mode_set(w, inputs, key, mode, false);
    tail(w, inputs, key, r4);
}

/// The row2 move bit of `mode` (`monstats2` +0x104, bit
/// mode).
fn moves(w: &ClientWorld, inputs: &ModelInputs, key: Who, mode: u32) -> bool {
    unit_ref(w, key)
        .and_then(|u| row(inputs, u.class))
        .is_some_and(|r| mode < 16 && r.move_modes >> mode & 1 != 0)
}

/// The face with (U, x, y) of a C monster: the new direction toward
/// the centre of sub-tile (x, y) from the path's precise point, with the
/// turn step (flag 0); the update turns it.
fn face_c(w: &mut ClientWorld, key: Who, x: u16, y: u16) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    let Some(p) = path_mut(w, key) else {
        return;
    };
    let centre = |c: u16| (u32::from(c) << 16) | 0x8000;
    let from = (p.precise_x, p.precise_y);
    let (_, dir) = d2_sim::path::walk::geom::direction_vector(t, from, (centre(x), centre(y)));
    d2_sim::path::walk::geom::set_facing(t, p, UnitType::Monster, i32::from(dir & 63));
}

fn tail(w: &mut ClientWorld, inputs: &ModelInputs, key: Who, r4: i32) {
    if r4 == 0 {
        return;
    }
    let Some(u) = unit_mut(w, key) else {
        return;
    };
    if u.stats.get(&STAT_VELOCITYPERCENT).copied().unwrap_or(0) == r4 {
        return;
    }
    u.stats.insert(STAT_VELOCITYPERCENT, r4);
    let speed = super::monster_anim::rate_of(u, inputs, |s| u.stats.get(&s).copied().unwrap_or(0));
    u.speed = Some(speed);
    let mode = u.mode;
    if let Some(v) = with_ctx(w, inputs, key, |t, c, _| mode_velocity(t, c, ME, mode)).flatten() {
        if let Some(p) = path_mut(w, key) {
            set_velocity(p, v);
        }
    }
}

/// The monster update of a C monster: the mode
/// record ([`super::monster_anim::mode_record`]); path kind 1 → the
/// path step (P := the movement ended); the anim step
/// (kind 1 tests completion before it advances, 2 holds
/// the last frame once complete, 3 holds frame 1); the end test by end
/// kind (1 → P, 2 / 3 → A, 4 → P and the animation complete after
/// the advance, 0 → never); the turn; ended → the mode end
/// of a client-only unit (default: mode set 1 with restart).
pub fn update(w: &mut ClientWorld, inputs: &ModelInputs, key: impl Into<Who>) {
    let (rate, frame) = (w.tick_rate, w.share_frame());
    let complete = |u: &ClientUnit| super::monster_anim::complete_at(u, rate, frame);
    let advance = |u: &mut ClientUnit| super::monster_anim::advance_at(u, rate, frame);
    let key = key.into();
    let Some((class, mode)) = unit_ref(w, key).map(|u| (u.class, u.mode)) else {
        return;
    };
    let Some((path_kind, anim_kind, end_kind)) =
        super::monster_anim::mode_record(inputs, class, mode)
    else {
        return;
    };
    let mut p_done = false;
    if path_kind == 1 {
        p_done = with_ctx(w, inputs, key, |t, c, p| {
            let mut walk = Walk { t, c };
            !walk.movement(ME, p, BASE).unwrap_or(false)
        })
        .unwrap_or(false);
        sync_position(w, key);
    }
    let mut a_done = false;
    if let Some(u) = unit_mut(w, key) {
        match anim_kind {
            1 if complete(u) => a_done = true,
            2 if complete(u) => {
                u.frame = u.frame_count - 0x100;
                u.speed = Some(0);
            }
            0..=2 => advance(u),
            3 => {
                u.frame = 0x100;
                u.speed = Some(0);
            }
            _ => {}
        }
    }
    let ended = unit_ref(w, key).is_some_and(|u| {
        super::monster_anim::ended_at(u, end_kind, p_done, a_done, w.tick_rate, w.share_frame())
    });
    if let Some(p) = path_mut(w, key) {
        turn(p);
    }
    if ended {
        mode_set(w, inputs, key, 1, true);
    }
}

/// The turn: with direction ≠ new
/// direction and path flag 0x40 clear, direction := (direction + step)
/// & 63; once that passes the new direction (the distance left, mod 64,
/// on the far side of the step's sign), direction := new direction.
fn turn(p: &mut DynamicPath) {
    if p.direction == p.new_direction || p.flags & 0x40 != 0 {
        return;
    }
    let step = p.turn_step;
    let d = step.wrapping_add(p.direction) & 63;
    p.direction = d;
    let left = d.wrapping_sub(p.new_direction) & 63;
    // A step byte above 63 turns down (negative), else up.
    if (left > 31 && step > 63) || (left < 31 && step < 63) {
        p.direction = p.new_direction;
    }
}

/// Writes the path's cell and precise position to the model (the cell is
/// the unit's position; the draw uses the precise point) and files the
/// unit in the room of its cell.
fn sync_position(w: &mut ClientWorld, key: Who) {
    let Some(p) = path_ref(w, key) else {
        return;
    };
    let (cx, cy) = (p.x(), p.y());
    let precise = (p.precise_x, p.precise_y);
    let Some(u) = unit_mut(w, key) else {
        return;
    };
    let old = u.position;
    u.position = Some((cx as u16, cy as u16));
    u.precise = Some(precise);
    if old != u.position {
        let room = w.room_at(cx as u16, cy as u16).map(|r| r.room);
        let rk = if key.server {
            key.key
        } else {
            view_key(key.key)
        };
        // The recache: a room that
        // still holds the cell leaves the list order as it is.
        if room.is_some() && w.room_units.room_of(rk) == room {
            return;
        }
        w.room_units.place(rk, room);
    }
}

// ---- server (set S) monsters: the single-player server copy -------------

/// The path of a server monster made by the add: at (x, y) in the room
/// of the point, velocity from the table, the
/// initial direction the add drew (`monster_anim::first_direction`) and
/// the creator's stop distance. D2RUST: the base-game path allocation
/// of every monster add.
pub fn server_setup(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, x: i32, y: i32) {
    let who = Who::server(key);
    let Some(r) = unit_ref(w, who).and_then(|u| row(inputs, u.class)) else {
        return;
    };
    let Some(drlg_room) = w.room_at(x as u16, y as u16).map(|r| r.room) else {
        return;
    };
    let Some(room) = w
        .drlg
        .as_ref()
        .and_then(|d| d.drlg.active_room(drlg_room))
        .map(|a| a.id)
    else {
        return;
    };
    // The allocation stamps the footprint; D2RUST re-stamps every unit's
    // footprint once per update (`client_missiles::stamp_unit_footprints`,
    // measured equal on a walking Fallen's client grid),
    // so the grids stay as they were.
    let grids = w.objclient.unit_grids.clone();
    let made = alloc(w, who, &r, room, x, y);
    w.objclient.unit_grids = grids;
    if !made {
        return;
    }
    let dir = unit_ref(w, who).and_then(|u| u.path_dir).unwrap_or(0);
    let stop = r.path_byte;
    if let Some(p) = path_mut(w, who) {
        snap(p, dir);
        p.stop_distance = if (1..=19).contains(&stop) {
            stop - 1
        } else {
            0
        };
    }
}

/// The context of the teleport: the path context with the walk's set
/// position and movement reset.
struct Motion<'a, 'b> {
    t: &'a PathTables,
    c: &'a mut Ctx<'b>,
}

impl CollisionRooms for Motion<'_, '_> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.c.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.c.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.c.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.c.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.c.grid_mut(room)
    }
}

impl PathMotion for Motion<'_, '_> {
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, hint: Option<RoomId>) {
        let q = (to_fp16_center(x), to_fp16_center(y));
        Walk {
            t: self.t,
            c: &mut *self.c,
        }
        .set_position(ME, path, q, hint);
    }
    fn reset(&mut self, path: &mut DynamicPath) {
        Walk {
            t: self.t,
            c: &mut *self.c,
        }
        .reset(ME, path);
    }
}

/// The placement of a server unit by 0x15: its
/// client path is teleported to (x, y) in the room of (x, y)
/// (the cell centre, the movement reset, no points), so a
/// walk that follows starts from the placed cell. No direction is
/// written. No client path, room or DRLG: nothing.
pub fn server_place(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, x: i32, y: i32) {
    let who = Who::server(key);
    let Some(room) = w
        .room_at(x as u16, y as u16)
        .map(|r| r.room)
        .and_then(|r| w.drlg.as_ref()?.drlg.active_room(r).map(|a| a.id))
    else {
        return;
    };
    // As in [`server_setup`]: D2RUST re-stamps the footprints once per
    // update, so the grids stay as they were.
    let grids = w.objclient.unit_grids.clone();
    let placed = with_ctx(w, inputs, who, |t, c, p| {
        teleport_and_clear(&mut Motion { t, c }, p, false, Some(room), x, y).is_ok()
    });
    w.objclient.unit_grids = grids;
    if placed == Some(true) {
        sync_position(w, who);
    }
}

/// The path velocity of the unit's current mode, after
/// a mode set or a change of stat 67.
pub fn server_velocity(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    let who = Who::server(key);
    let Some(mode) = unit_ref(w, who).map(|u| u.mode) else {
        return;
    };
    if let Some(v) = with_ctx(w, inputs, who, |t, c, _| mode_velocity(t, c, ME, mode)).flatten() {
        if let Some(p) = path_mut(w, who) {
            set_velocity(p, v);
        }
    }
}

/// The path type set of a pathed request (a path type > 0 → path
/// type :=), on a server monster's
/// client path. A type the table does not hold, or a set the core
/// asserts on, leaves the path as it is (the base game stops on a fatal there).
pub fn server_path_type(w: &mut ClientWorld, key: UnitKey, t: u32) {
    let Some(tables) = super::predict::path_tables() else {
        return;
    };
    if let Some(p) = path_mut(w, Who::server(key)) {
        let _ = p.set_path_type(tables, false, t);
    }
}

/// Code 0x01 / 0x17 of a server monster: W(1), target := the
/// point, compute. True: a path was found, or there is no
/// client path context to ask (mode set 2 / 0xF follows); false: the
/// neutral fallback follows.
pub fn server_walk(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    x: i32,
    y: i32,
) -> bool {
    let who = Who::server(key);
    walk_flag(w, who, true);
    with_ctx(w, inputs, who, |t, c, p| {
        p.set_target_point(x as u16, y as u16);
        let n = compute(t, c, p, ME, true).unwrap_or(0);
        c.foot = footprint_of(p);
        n
    })
    .is_none_or(|n| n > 0)
}

/// The path's target unit (+0x58) := `target` (the unit
/// looked up in S: an absent one is none), or none.
pub fn set_target_unit(w: &mut ClientWorld, key: impl Into<Who>, target: Option<UnitKey>) {
    let key = key.into();
    let target = target.filter(|t| w.units.contains_key(t)).and_then(|t| {
        let ty = *UnitType::ALL.get(usize::from(t.unit_type))?;
        Some(TargetUnit {
            unit: TARGET,
            ty,
            guid: t.guid,
        })
    });
    if let Some(p) = path_mut(w, key) {
        p.target_unit = target;
    }
}

/// Code 0x00 / 0x18 of a server monster: W(1),
/// the path's target unit := (type, GUID), compute
/// toward that unit's cell. True: a path was found, or there
/// is no client path context to ask; false: the neutral fallback
/// follows.
pub fn server_walk_unit(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    target: UnitKey,
) -> bool {
    let who = Who::server(key);
    walk_flag(w, who, true);
    set_target_unit(w, who, Some(target));
    with_ctx(w, inputs, who, |t, c, p| {
        let n = compute(t, c, p, ME, true).unwrap_or(0);
        c.foot = footprint_of(p);
        n
    })
    .is_none_or(|n| n > 0)
}

/// The monster update of a server monster: the mode record; the skill
/// step; path kind 1 →
/// the path step (P); the NPC turn; the anim step by the anim kind
/// (A); the end test by the end kind and the turn;
/// ended → the mode end.
/// PROVISIONAL: the path step moves
/// the unit only in WL and RN (2, 15); in the other modes of path kind 1
/// the model holds no path set-up (KB's 0x14 path, the move bits' path to
/// the target), so P := 1 (the path done); the walk resume is not
/// run. Settled by a recording of a monster knocked back and of one
/// attacking with a move bit in sight.
pub fn server_update(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), super::dispatch::HandlerError> {
    let who = Who::server(key);
    let Some(u) = unit_ref(w, who) else {
        return Ok(());
    };
    // State 1 `freeze` on a live monster: no step this tick (head).
    if super::monster_anim::frozen(u) {
        return Ok(());
    }
    let mode = u.mode;
    let Some((path_kind, anim_kind, end_kind)) =
        super::monster_anim::mode_record(inputs, u.class, mode)
    else {
        return Ok(());
    };
    // The skill step, on the event byte of the
    // previous update's advance.
    super::monster_anim::skill_step(w, inputs, key)?;
    let p = path_kind == 1 && server_path_step(w, inputs, key);
    npc_turn(w, inputs, key);
    let a = super::monster_anim::anim_step(w, inputs, key, anim_kind);
    // The mode missile step, after the anim step.
    super::monster_anim::mode_missile(w, inputs, key)?;
    // The umod phase-2 hooks.
    super::monster_anim::umod_phase2(w, inputs, key)?;
    let ended = unit_ref(w, who).is_some_and(|u| {
        super::monster_anim::ended_at(u, end_kind, p, a, w.tick_rate, w.share_frame())
    });
    if let Some(p) = path_mut(w, who) {
        turn(p);
    }
    if ended {
        // The walk flag 0x2000 is cleared by every copy.
        let walk_flag = unit_ref(w, who).is_some_and(|u| u.flag_ex & WALK_FLAG != 0);
        super::monster_anim::mode_end(w, inputs, key, walk_flag);
    }
    Ok(())
}

/// The path step of a server monster:
/// whether its movement ended (P). In WL and RN (2, 15) the movement
/// runs, and in a sequence (14) while the used entry's E-flag bit 0 is
/// set (a moving skill's run, DiabRun); in the other modes P := 1
/// ([`server_update`]).
pub fn server_path_step(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) -> bool {
    let who = Who::server(key);
    let Some(mode) = unit_ref(w, who).map(|u| u.mode) else {
        return true;
    };
    let run = mode == 14 && super::skills::used_entry(w, key).is_some_and(|e| e.flags & 1 != 0);
    if !matches!(mode, 2 | 15) && !run {
        return true;
    }
    let p = with_ctx(w, inputs, who, |t, c, p| {
        let mut walk = Walk { t, c };
        !walk.movement(ME, p, BASE).unwrap_or(false)
    });
    // A sequence run without a path context keeps's P := 1.
    let p = p.unwrap_or(run);
    // The unit stands where its path stepped it: a later unit's
    // drain in this pass reads that cell (measured: a Fallen's code
    // 0x10 faces the walking hireling's new cell, dir64 15, not 23).
    sync_position(w, who);
    p
}

/// With (unit, x, y): the direction (0…63) from the unit's path
/// position toward the centre of sub-tile (x, y). The
/// local player's path position is its predicted point (B025).
pub fn direction_to(w: &ClientWorld, key: UnitKey, x: u16, y: u16) -> Option<u8> {
    let t = super::predict::path_tables()?;
    let u = w.units.get(&key)?;
    let predicted = w.local_predicted.filter(|_| w.local_player == Some(key));
    let from = predicted.or(u.precise).unwrap_or_else(|| {
        let (cx, cy) = u.cell();
        (
            (u32::from(cx) << 16) | 0x8000,
            (u32::from(cy) << 16) | 0x8000,
        )
    });
    let centre = |c: u16| (u32::from(c) << 16) | 0x8000;
    let (_, dir) = d2_sim::path::walk::geom::direction_vector(t, from, (centre(x), centre(y)));
    Some(dir & 63)
}

/// The face with (unit, x, y) of a server player or monster: the
/// new direction toward the centre of sub-tile (x, y) from the unit's
/// precise point and the turn step; the update turns the
/// direction by steps (measured on a Fire Bolt cast: 0, 60, 56, 54 over
/// four frames). The cast of code 0x15 / 0x16.
pub fn face_point(w: &mut ClientWorld, key: UnitKey, x: u16, y: u16) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    let Some(dir) = direction_to(w, key, x, y) else {
        return;
    };
    if let Some(p) = path_mut(w, Who::server(key)) {
        // A monster:, the new direction and the turn step; the
        // monster update turns it.
        d2_sim::path::walk::geom::set_facing(t, p, UnitType::Monster, i32::from(dir));
        return;
    }
    if let Some(u) = w.units.get_mut(&key) {
        let cur = u.path_dir.unwrap_or(0);
        if u.path_turn.map(|(d, _)| d) != Some(dir) {
            let step = t.dirdiff[((i32::from(dir) - i32::from(cur)) & 63) as usize] as u8;
            u.path_turn = Some((dir, step));
        }
        u.faces = u.faces.wrapping_add(1);
    }
}

/// The knockback face of a server monster's code 0x14: the KB path's
/// compute (type 11, one point:
/// the KB target) ends in the aim
/// with argument 1, which always faces; type 11's table
/// flags (0x1E604) hold 0x200, so the aim turns the direction to the
/// point by 32: the
/// knocked-back monster faces away from its KB point, toward the hitter
/// (measured on a bashed Fallen: KB point at 23, new direction 55).
pub fn face_knockback(w: &mut ClientWorld, key: UnitKey, x: u16, y: u16) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    let Some(dir) = direction_to(w, key, x, y) else {
        return;
    };
    if let Some(p) = path_mut(w, Who::server(key)) {
        let away = (i32::from(dir) - 32) & 63;
        d2_sim::path::walk::geom::set_facing(t, p, UnitType::Monster, away);
    }
}

/// The point-cast turn: a server
/// monster of `monstats` `Code` `PB  ` (the turrets, `firetower`) takes
/// its new direction + 0x38 (mod 64) through, which redraws
/// the turn step (measured: a face toward (0, 0) at
/// direction 24 becomes 16, turned by 4 a frame).
pub fn turret_turn(w: &mut ClientWorld, key: UnitKey) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    if let Some(p) = path_mut(w, Who::server(key)) {
        let d = (i32::from(p.new_direction) + 0x38) & 63;
        d2_sim::path::walk::geom::set_facing(t, p, UnitType::Monster, d);
    }
}

/// The face with (path, dir) of a server player or monster
/// toward direction `dir` (`setfunc` 14):
/// the new direction and the `dirdiff` turn step, turned
/// by the unit's update (measured on a werewolf: 11 → 7, 3, 0).
pub fn face_direction(w: &mut ClientWorld, key: UnitKey, dir: u8) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    let dir = dir & 63;
    if let Some(p) = path_mut(w, Who::server(key)) {
        d2_sim::path::walk::geom::set_facing(t, p, UnitType::Monster, i32::from(dir));
        return;
    }
    if let Some(u) = w.units.get_mut(&key) {
        if u.path_turn.map(|(d, _)| d) != Some(dir) {
            let cur = u.path_dir.unwrap_or(0);
            let step = t.dirdiff[((i32::from(dir) - i32::from(cur)) & 63) as usize] as u8;
            u.path_turn = Some((dir, step));
        }
    }
}

/// The aim of a player's walk to a point (code 0x01):
/// the path compute runs only for a target
/// other than the unit's sub-tile and at most 100 sub-tiles away on each
/// axis; its aim with (1) sets the new direction toward the path's
/// first point and the `dirdiff` turn step, which the
/// player update then turns by. The point is the unit's own
/// (after the 0x15 placement of an arrival), not the predicted one: a
/// waypoint arrival's 0x0D code 1 to x + 3, y + 3 turns the player one
/// step a tick though it never walks it (measured: 33 → 41, 49, 57, 0
/// over four ticks). PROVISIONAL (code 0x01; settled by a capture of
/// the client state at two walk-outs): the aim faces itself. The base
/// game aims at the compute's first point, which is one step
/// along a bent path (measured at a walk-to-unit compute) and the
/// target only for a one-point path, like
/// the arrival's free diagonal of 3.
pub fn aim_walk(w: &mut ClientWorld, key: UnitKey, x: u16, y: u16) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let (cx, cy) = u.cell();
    let (dx, dy) = (i32::from(x) - i32::from(cx), i32::from(y) - i32::from(cy));
    if (dx, dy) == (0, 0) || dx.abs() > 100 || dy.abs() > 100 {
        return;
    }
    let centre = |c: u16| (u32::from(c) << 16) | 0x8000;
    // The precise position only while it lies in the unit's cell (the
    // client track keeps it for walking units; a stale one is ignored).
    let from = u
        .precise
        .filter(|&(px, py)| (px >> 16, py >> 16) == (u32::from(cx), u32::from(cy)))
        .unwrap_or((centre(cx), centre(cy)));
    let (_, dir) = d2_sim::path::walk::geom::direction_vector(t, from, (centre(x), centre(y)));
    let dir = dir & 63;
    if u.path_turn.map(|(d, _)| d) != Some(dir) {
        let cur = u.path_dir.unwrap_or(0);
        let step = t.dirdiff[((i32::from(dir) - i32::from(cur)) & 63) as usize] as u8;
        u.path_turn = Some((dir, step));
    }
}

/// One turn step of a server monster's path now (the NPC dialog
/// branch of the UI layer runs after the frame's update pass in the headless
/// host, where the base game's runs before it: the first step lands in the same frame).
pub fn turn_now(w: &mut ClientWorld, key: UnitKey) {
    if let Some(p) = path_mut(w, Who::server(key)) {
        turn(p);
    }
}

/// The new direction the local player's client path step aims at this
/// update: a walking
/// player (server mode 2, 3, 6) takes `s.new_dir` as its pending turn,
/// the step from its current `path_dir` (the `dirdiff`
/// step, set only when the new direction changes), and [`player_turn`]
/// turns it once (measured: 0 → 60, 56, 52, 49 over four ticks). The
/// single-player copy leaves +0x64..+0x66
/// alone, so the facing is the client's own.
pub fn player_aim(w: &mut ClientWorld, s: &ServerPath) {
    let Some(new) = s.new_dir else {
        return;
    };
    // A knockback (server and client in mode 19): the client's own type
    // 0xB compute aims as the server's did (+0x65), and the player update
    // turns it from the next update on (measured: d 0, 60, 56, 55).
    let knockback = s.mode == super::modes::player_mode::SEQUENCE
        && w.units
            .get(&s.key)
            .is_some_and(|u| u.mode == super::modes::player_mode::SEQUENCE);
    if !matches!(s.mode, 2 | 3 | 6) && !path_skill(w, s.key) && !knockback {
        return;
    }
    if s.key.unit_type != 0 {
        return;
    }
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    let walking = matches!(s.mode, 2 | 3 | 6);
    let Some(u) = w.units.get_mut(&s.key) else {
        return;
    };
    let new = new & 63;
    if u.path_turn.map(|(d, _)| d) != Some(new) {
        let cur = u.path_dir.unwrap_or(0);
        let step = t.dirdiff[((i32::from(new) - i32::from(cur)) & 63) as usize] as u8;
        u.path_turn = Some((new, step));
    }
    if knockback {
        return;
    }
    // This tick's turn, in place of the update's (D2RUST reads the server
    // once per tick, after the update; drawn after both).
    player_turn(w, s.key);
    if let Some(u) = w.units.get_mut(&s.key) {
        u.copy_turned = walking;
    }
}

/// Whether a player's used skill has E-flag bit 0 (a moving skill whose
/// path the player update steps).
pub fn any_moving_skill(w: &ClientWorld) -> bool {
    w.units.values().any(|u| {
        u.key.unit_type == 0
            // A knockback (mode 19, path kind 1) steps its path too
            // (the KB end).
            && (u.mode == super::modes::player_mode::SEQUENCE
                || u.skills
                .as_ref()
                .and_then(|l| l.entries.get(l.current?))
                .is_some_and(|e| e.flags & 1 != 0))
    })
}

/// A player in mode 18 whose used skill entry has E-flag bit 0 (a moving
/// skill: Charge, Whirlwind): its update's
/// path step runs the same aim as a walk's.
pub fn path_skill(w: &ClientWorld, key: UnitKey) -> bool {
    w.units.get(&key).is_some_and(|u| {
        u.mode == 18
            && u.skills
                .as_ref()
                .and_then(|l| l.entries.get(l.current?))
                .is_some_and(|e| e.flags & 1 != 0)
    })
}

/// The result of an S player's own path step this update, read from the
/// server's
/// path, which ran the same step on the same words this tick:
/// still moving with points left, else ended.
pub fn player_step_result(w: &mut ClientWorld, s: &ServerPath) {
    if s.key.unit_type != 0 {
        return;
    }
    let Some(c) = &s.copy else {
        return;
    };
    if let Some(u) = w.units.get_mut(&s.key) {
        u.path_live = Some(c.moving && c.point_count > 0 && c.cur_point < c.point_count);
    }
}

/// A player in knockback (mode 19) whose server path this tick's read does
/// not hold: its knockback path counts as ended, so the player update ends
/// the knockback (mode 4, get-hit) as it does on the server copy's end.
/// A link with no server path copy (the game's own link, a network client)
/// never reports the end, and the player stayed in mode 19 at the
/// knockback's first cell until another mode message came.
pub fn knockback_without_server_path(w: &mut ClientWorld, steps: &[ServerPath]) {
    for u in w.units.values_mut() {
        if u.key.unit_type == 0
            && u.mode == super::modes::player_mode::SEQUENCE
            && u.path_live.is_none()
            && !steps.iter().any(|s| s.key == u.key && s.copy.is_some())
        {
            u.path_live = Some(false);
        }
    }
}

/// The swing helper of a unit in its sequence mode (player SQ 18,
/// monster 14) whose used skill's `srvdofunc` is 76 (Whirlwind): it runs twice a
/// frame from the sequence's event calls (`PlrSkills`),
/// sets the path direction to +0x15 / +0x2A for its point and back through,
/// which writes the direction and the new direction (+0x64, +0x65)
/// both. So the frame ends with the held new direction equal to the direction, and
/// the next path aim sees its direction as new and takes the turn step
/// from the current difference. Measured on a Whirlwind
/// (write watch on +0x64..+0x67, nine frames): the start aims 54 with step −4, the
/// first turn gives 60; each later frame the restore leaves new = d, the aim
/// re-takes 54 with step −1: 60, 59 … 54.
/// D2RUST holds (new, step) in `path_turn`; the copy's aim (`player_aim`) recomputes
/// the step when the held new direction differs.
pub fn swing_restore(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    const SRVDO_WHIRLWIND: i16 = 76;
    const PLAYER_SQ: u32 = 18;
    let whirl = super::skills::used_entry(w, key)
        .and_then(|e| inputs.tables.skills.get(usize::from(e.skill)))
        .is_some_and(|r| r.srvdofunc == SRVDO_WHIRLWIND);
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    if key.unit_type != 0 || u.mode != PLAYER_SQ {
        return;
    }
    if let (true, Some(d), Some((_, step))) = (whirl, u.path_dir, u.path_turn) {
        u.path_turn = Some((d, step));
    }
}

/// The turn of a server player's `path_dir` toward the
/// pending new direction, run by the player update.
pub fn player_turn(w: &mut ClientWorld, key: UnitKey) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let Some((new, step)) = u.path_turn else {
        return;
    };
    let mut p = DynamicPath {
        direction: u.path_dir.unwrap_or(0),
        new_direction: new,
        turn_step: step,
        ..DynamicPath::default()
    };
    turn(&mut p);
    u.path_dir = Some(p.direction);
}

/// The NPC turn: an
/// `interact` monster that is not a `critter`, in mode 1, unit flag
/// 0x200 clear, class outside 537–539, with the local player within
/// distance 7 rolls 100 on its own seed;
/// below 10 it faces the player.
fn npc_turn(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    let who = Who::server(key);
    let Some(u) = unit_ref(w, who) else {
        return;
    };
    let Some(r) = row(inputs, u.class) else {
        return;
    };
    if !r.interact || r.critter || u.mode != 1 || u.flag_200 || (537..=539).contains(&u.class) {
        return;
    }
    let Some(p) = w.local() else {
        return;
    };
    let rows = &inputs.objclient.rows;
    let d = super::objects::unit_distance_at(
        u.cell(),
        i32::from(r.size_x),
        p.cell(),
        super::objects::unit_size(p, rows),
    );
    if d >= 7 {
        return;
    }
    let target = p.cell();
    let Some((lo, hi)) = u.seed else {
        return;
    };
    let mut seed = Seed::new(lo, hi);
    let roll = seed.roll(100);
    if let Some(u) = unit_mut(w, who) {
        u.seed = Some((seed.lo, seed.hi));
    }
    if roll >= 10 {
        return;
    }
    face_cell(w, who, target);
}

/// The cell face with (path, x, y, 0) on a server monster's client path: the
/// turn toward the centre of cell (x, y) ([`face_cell`]).
pub fn face_toward(w: &mut ClientWorld, key: UnitKey, x: u16, y: u16) {
    face_cell(w, Who::server(key), (x, y));
}

/// With (path, x, y, 0): the unit's path takes the direction toward
/// the centre of cell `target` as its new direction, with the turn step of
/// (the unit then turns by that step in the monster update).
fn face_cell(w: &mut ClientWorld, who: Who, target: (u16, u16)) {
    let Some(t) = super::predict::path_tables() else {
        return;
    };
    if let Some(path) = path_mut(w, who) {
        let centre = |c: u16| (u32::from(c) << 16) | 0x8000;
        let (_, dir) = d2_sim::path::walk::geom::direction_vector(
            t,
            (path.precise_x, path.precise_y),
            (centre(target.0), centre(target.1)),
        );
        d2_sim::path::walk::geom::set_facing(t, path, UnitType::Monster, i32::from(dir) & 63);
    }
}

/// The target face with (U, 0) of a server monster: the path's target unit
/// (+0x58), dropped when it is no longer in S; a target
/// other than U itself toward the centre of the target's
/// cell (its path cell; the local player's
/// is where it walked to), with the path turn step.
pub fn face_target(w: &mut ClientWorld, key: UnitKey) {
    let who = Who::server(key);
    let Some(target) = target_unit(w, who) else {
        return;
    };
    if !w.units.contains_key(&target) {
        set_target_unit(w, who, None);
        return;
    }
    if target == key {
        return;
    }
    let Some(cell) = unit_cell(w, target) else {
        return;
    };
    face_cell(w, who, cell);
    if w.local_player == Some(target) {
        if let Some(new) = path_mut(w, who).map(|p| p.new_direction) {
            w.local_faces.retain(|&(k, _)| k != key);
            w.local_faces.push((key, new));
        }
    }
}

/// A unit's cell as the cell getters read it for a face: its
/// path cell; the local player's is where it walked to.
pub fn unit_cell(w: &ClientWorld, key: UnitKey) -> Option<(u16, u16)> {
    let t = w.units.get(&key)?;
    Some(
        match w.local_predicted.filter(|_| w.local_player == Some(key)) {
            Some((px, py)) => ((px >> 16) as u16, (py >> 16) as u16),
            None => t.cell(),
        },
    )
}

/// Redoes this frame's faces at the local player
/// ([`ClientWorld::local_faces`]) toward the cell of its fresh predicted
/// position: the base game steps the local player before the monsters, so a
/// monster facing it sees where it stands this tick (measured: a
/// Fallen's mode-end face takes cell
/// (5164, 4656), which the player entered that tick). A face is redone
/// only while the path still holds the direction it set.
pub fn refresh_local_faces(w: &mut ClientWorld) {
    let faces = std::mem::take(&mut w.local_faces);
    let Some((px, py)) = w.local_predicted else {
        return;
    };
    let cell = ((px >> 16) as u16, (py >> 16) as u16);
    for (key, new) in faces {
        let who = Who::server(key);
        if path_mut(w, who).is_some_and(|p| p.new_direction == new) {
            face_cell(w, who, cell);
        }
    }
}

/// The 0x28 turn: the server monster
/// `key` faces the local player's cell. The 0x28 packet is handled before
/// the frame's monster update in the base game, so the first turn step
/// lands in the same frame; the bridge applies the answer after the
/// update pass and takes that step here.
pub fn face_local_player(w: &mut ClientWorld, key: UnitKey) {
    let Some(target) = w.local().map(|p| p.cell()) else {
        return;
    };
    let who = Who::server(key);
    face_cell(w, who, target);
    if let Some(p) = path_mut(w, who) {
        turn(p);
    }
}

/// The server's unit as the copy reads it.
#[derive(Clone, Debug)]
pub struct ServerPath {
    pub key: UnitKey,
    pub mode: u32,
    pub x: u16,
    pub y: u16,
    pub xf: u16,
    pub yf: u16,
    pub tx: u16,
    pub ty: u16,
    /// Point index, count and list (+0x24, +0x28, +0x9C); `None`
    /// when the host does not hold the server's path words.
    pub copy: Option<d2_sim::debug::state::PathCopy>,
    /// A player's path new direction (+0x65), set by the path aim
    /// the client's own step runs on the copied path:
    /// read from the server's
    /// path, which ran the same aim on the same words this tick. `None`
    /// for monsters and when the link cannot read it.
    pub new_dir: Option<u8>,
}

/// The single-player server copy for one server monster:
/// the client unit takes the server unit's
/// position and target, the mode rule (monster rows), and the walk
/// flag cleared.
/// The point list, index, count, speed and limit words come from
/// the server's path too (`ServerPath::copy`; measured on a hireling
/// teleport).
pub fn server_copy(w: &mut ClientWorld, inputs: &ModelInputs, s: &ServerPath) {
    let who = Who::server(s.key);
    if unit_ref(w, who).is_none() {
        return;
    }
    if path_ref(w, who).is_none() {
        // An S player has no client path here: the cell and precise
        // position are copied to the unit (first words), so a
        // server-side move reaches it at the next update; the target and
        // the player mode rows are copied too. The path words are not,
        // as the prediction gives the same walk (measured on a stash run).
        if s.key.unit_type == 0 {
            copy_player_position(w, s);
            copy_player_mode(w, inputs, s);
        }
        return;
    }
    // A cell in no loaded client room → nothing at all. The room is
    // looked up from U's room (that room, then its
    // adjacency; no act-wide lookup), so a server warp out of sight (the
    // hireling following its player to another level) leaves the client
    // unit in its old room, which the room free then flags.
    let moved =
        path_ref(w, who).is_some_and(|p| (p.x(), p.y()) != (i32::from(s.x), i32::from(s.y)));
    // The room cell lookup from the unit's own room (its
    // adjacency array only, no act-wide lookup: a server move out of sight,
    // a warp to another level, copies nothing); a unit without a room
    // copies nothing.
    let room = match w.unit_room(s.key).copied() {
        Some(own) => w
            .cell_lookup(&own, i32::from(s.x), i32::from(s.y))
            .map(|r| r.room),
        None if w.active_rooms.is_some() => return,
        None => w.room_at(s.x, s.y).map(|r| r.room),
    };
    if moved && room.is_none() && w.active_rooms.is_some() {
        return;
    }
    // The forced placement moves the footprint with the
    // cell (clear at the old cell in the old room, stamp at the new in the
    // new room, no test) and the room recache sets the path's room, so the
    // client's own step looks its cells up from the right room.
    if moved {
        let new_room = room.and_then(|r| {
            w.drlg
                .as_ref()
                .and_then(|d| d.drlg.active_room(r).map(|a| a.id))
        });
        let old = path_ref(w, who).map(|p| (p.x(), p.y()));
        if let Some(old) = old {
            with_ctx(w, inputs, who, |_, c, p| {
                d2_sim::path::footprint::forced_move_rooms(
                    c,
                    p.room,
                    old,
                    new_room,
                    (i32::from(s.x), i32::from(s.y)),
                    p.pattern,
                    p.foot_mask,
                );
                if new_room.is_some() {
                    p.room = new_room;
                }
            });
        }
    }
    let precise = (
        (u32::from(s.x) << 16) | u32::from(s.xf),
        (u32::from(s.y) << 16) | u32::from(s.yf),
    );
    if let Some(p) = path_mut(w, who) {
        p.precise_x = precise.0;
        p.precise_y = precise.1;
        p.update_client();
        p.target_x = s.tx;
        p.target_y = s.ty;
        // The point index, count and list; the client's own path step
        // then turns toward the next point as the base-game client does.
        if let Some(c) = &s.copy {
            // The whole target block +0x10..+0x1B, so a client
            // re-path after a collision aims at the server's
            // final target, not the client request's point.
            p.put_prev_target(Point::new(
                i32::from(c.prev_target.0),
                i32::from(c.prev_target.1),
            ));
            p.put_final_target(Point::new(
                i32::from(c.final_target.0),
                i32::from(c.final_target.1),
            ));
            p.cur_point = c.cur_point;
            p.point_count = c.point_count;
            p.field_38 = c.field_38;
            p.target_lead = c.target_lead;
            (p.dir_vec_x, p.dir_vec_y) = c.dir_vec;
            [
                p.velocity,
                p.saved_velocity,
                p.max_velocity,
                p.acceleration,
                p.accel_counter,
            ] = c.velocity;
            [
                p.max_distance,
                p.ida_score,
                p.stop_distance,
                p.repath_budget,
            ] = c.limits;
            p.dir_offset = c.dir_offset;
            p.flags = (p.flags & !0x20) | if c.moving { 0x20 } else { 0 };
            for (slot, &(x, y)) in p.points.iter_mut().zip(&c.points) {
                *slot = d2_sim::path::record::PathPoint { x, y };
            }
        }
    }
    if let Some(u) = unit_mut(w, who) {
        u.position = Some((s.x, s.y));
        u.precise = Some(precise);
    }
    // The server's unit sits in the list of the room holding its cell. The
    // placement's recache changes the list only when the unit's room no longer
    // holds the cell: a move inside the room keeps the unit's place.
    let unlisted = room.is_some() && w.room_units.room_of(s.key) != room;
    if unlisted && w.active_rooms.is_some() {
        w.room_units.place(s.key, room);
    }
    // Monster rows: (server mode, client mode) → new mode.
    let c = unit_ref(w, who).map_or(0, |u| u.mode);
    if c != s.mode {
        let new = match (s.mode, c) {
            (1, 2 | 15) => Some(1),
            (2, 1) => Some(2),
            (15, 1) => Some(15),
            _ => None,
        };
        if let Some(m) = new {
            super::monster_anim::mode_set(w, inputs, s.key, m);
            server_velocity(w, inputs, s.key);
        }
    }
    walk_flag(w, who, false);
}

/// The player rows of the mode rule of the single-player copy:
/// (server mode, client mode)
/// → new mode. A client that walks while the server stands is pulled back
/// to neutral (5 in a town room, else 1), a standing client follows the
/// server into walk (6 / 2) or run (3). A server-side walk (the run to an
/// object) thus reaches the model, and ends in it.
fn copy_player_mode(w: &mut ClientWorld, inputs: &ModelInputs, s: &ServerPath) {
    let Some(c) = w.units.get(&s.key).map(|u| u.mode) else {
        return;
    };
    let (neutral, walk) = super::modes::neutral_walk(w, s.key);
    let new = match (s.mode, c) {
        (1 | 5, 2 | 3 | 6) => Some(neutral),
        (2 | 6, 1 | 5) => Some(walk),
        (3, 1 | 5) => Some(3),
        (_, 19) if s.mode != 19 => Some(s.mode),
        _ => None,
    };
    if let Some(m) = new {
        super::player_anim::mode_set(w, inputs, s.key, m);
    }
}

/// Whether the local player's update runs the path step of path kind 1
/// this pass (WL, RN, TW, KB).
pub fn local_walks(w: &ClientWorld) -> bool {
    w.local().is_some_and(|u| matches!(u.mode, 2 | 3 | 6 | 19))
}

/// A unit's cell and precise position, each unset or set.
type Placed = (Option<(u16, u16)>, Option<(u32, u32)>);

/// The local player's position before its path step this pass and the
/// position the step set ([`local_path_step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalStep {
    key: UnitKey,
    before: Placed,
    at: ((u16, u16), (u32, u32)),
}

/// The local player's path step in its update, before the S
/// monsters': their arrival test reads where it
/// stands this tick. The client steps its copy of the server's path, which
/// the server stepped the same way this tick, so the step's
/// result is the server's position. `None`: no step (not walking, or not
/// the local player).
pub fn local_path_step(w: &mut ClientWorld, s: &ServerPath) -> Option<LocalStep> {
    if w.local_player != Some(s.key) || !local_walks(w) {
        return None;
    }
    let u = w.units.get_mut(&s.key)?;
    let at = (
        (s.x, s.y),
        (
            (u32::from(s.x) << 16) | u32::from(s.xf),
            (u32::from(s.y) << 16) | u32::from(s.yf),
        ),
    );
    let before = (u.position, u.precise);
    u.position = Some(at.0);
    u.precise = Some(at.1);
    Some(LocalStep {
        key: s.key,
        before,
        at,
    })
}

/// The pass end: the stepped position goes back to the one the pass
/// started with, unless a message of the pass placed the player since.
/// D2RUST bookkeeping: D2RUST moves the local player in the server copy after
/// the pass (with its room list place) and the walk prediction
/// steps from the position before that copy, so the step's position
/// holds for the rest of this pass only.
pub fn local_path_step_end(w: &mut ClientWorld, step: LocalStep) {
    if let Some(u) = w.units.get_mut(&step.key) {
        if (u.position, u.precise) == (Some(step.at.0), Some(step.at.1)) {
            (u.position, u.precise) = step.before;
        }
    }
}

/// The position part of the single-player copy for an S player.
fn copy_player_position(w: &mut ClientWorld, s: &ServerPath) {
    // The unit's own room, else its adjacency array (no
    // act-wide lookup): a warp to a level out of sight finds none.
    let (x, y) = (i32::from(s.x), i32::from(s.y));
    let room = w
        .local_room()
        .and_then(|own| w.cell_lookup(own, x, y))
        .map(|r| r.room);
    let moved = w
        .units
        .get(&s.key)
        .is_some_and(|u| u.position != Some((s.x, s.y)));
    if moved && room.is_none() {
        return;
    }
    let precise = (
        (u32::from(s.x) << 16) | u32::from(s.xf),
        (u32::from(s.y) << 16) | u32::from(s.yf),
    );
    if let Some(u) = w.units.get_mut(&s.key) {
        u.position = Some((s.x, s.y));
        u.precise = Some(precise);
        // The raw copy carries the target (+0x10) too.
        u.path_target = Some((s.tx, s.ty));
    }
    // The placement's recache keeps the list place of a move inside the
    // room, as for a monster.
    let unlisted = room.is_some() && w.room_units.room_of(s.key) != room;
    if unlisted && w.active_rooms.is_some() {
        w.room_units.place(s.key, room);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(direction: u8, new: u8, step: i8) -> DynamicPath {
        DynamicPath {
            direction,
            new_direction: new,
            turn_step: step as u8,
            ..DynamicPath::default()
        }
    }

    /// Measured: a Fallen walking (path type
    /// 0xD, stop distance 0) to the player 2 cells across and 1 down is
    /// done on the client too, because the player counts size 2 in the
    /// unit distance (0); at size 0 the distance is 1 and the walk went on.
    #[test]
    fn the_target_player_counts_size_two_in_the_arrival_test() {
        let mut inputs = ModelInputs::default();
        inputs.tables.monsters = vec![Some(MonsterClass {
            size_x: 3,
            ..MonsterClass::default()
        })];
        let player = super::super::world::ClientUnit::new(UnitKey::new(0, 1));
        let monster = super::super::world::ClientUnit::new(UnitKey::new(1, 7));
        let item = super::super::world::ClientUnit::new(UnitKey::new(4, 9));
        assert_eq!(target_size(&inputs, &player), 2);
        assert_eq!(target_size(&inputs, &monster), 3);
        assert_eq!(target_size(&inputs, &item), 1);
        let Some(t) = super::super::predict::path_tables() else {
            return;
        };
        let (me, them) = (Point::new(5145, 4264), Point::new(5143, 4263));
        let d = |size| d2_sim::path::walk::geom::unit_distance(t, me, 2, them, size);
        assert_eq!((d(2), d(0)), (0, 1));
    }

    /// Measured on a Frozen Armor cast: Fallen 1:32's client step
    /// collides and re-paths toward the local
    /// player with path flag 0x800, so the compute lifts the player's
    /// footprint (step 6) and the walk goes on (dir 47 → 55, 63, 0). The
    /// context finds that footprint in the unit layer's marks, and a C
    /// monster target's in its own client path.
    #[test]
    fn the_compute_lifts_the_target_units_client_footprint() {
        use d2_sim::path::TargetUnit;
        let player = UnitKey::new(0, 1);
        let fallen = UnitKey::new(1, 27);
        let mut w = ClientWorld::default();
        let mark = Footprint {
            room: None,
            x: 5168,
            y: 4658,
            shape: FootShape::Pattern(1),
            mask: 0x80,
        };
        w.objclient.foot_marks.insert(player, mark);
        let other = DynamicPath {
            precise_x: (5166 << 16) | 0x8000,
            precise_y: (4659 << 16) | 0x8000,
            foot_mask: 0x100,
            ..DynamicPath::default()
        };
        w.objclient.s_paths.insert(fallen, other.clone());
        let to = |ty, guid| DynamicPath {
            target_unit: Some(TargetUnit {
                unit: UnitId(9),
                ty,
                guid,
            }),
            ..DynamicPath::default()
        };
        assert_eq!(target_footprint(&w, &to(UnitType::Player, 1)), Some(mark));
        assert_eq!(
            target_footprint(&w, &to(UnitType::Monster, 27)),
            Some(footprint_of(&other))
        );
        assert_eq!(target_footprint(&w, &to(UnitType::Monster, 28)), None);
        assert_eq!(target_footprint(&w, &DynamicPath::default()), None);
    }

    /// Measured (a write watch over nine frames): Whirlwind's
    /// swing helper leaves the held new direction equal to the direction,
    /// so the next aim re-takes the step from the current difference
    /// (60, 59 … 54); another skill or mode keeps the start's turn.
    #[test]
    fn the_whirlwind_swing_leaves_new_direction_equal_to_direction() {
        use super::super::skills::{SkillEntry, SkillList, NATIVE};
        use super::super::world::SkillRow;
        let p = UnitKey::new(0, 1);
        let mut inputs = ModelInputs::default();
        inputs.tables.skills = vec![SkillRow::default(); 152];
        inputs.tables.skills[151].srvdofunc = 76;
        let run = |mode: u32, skill: u16| {
            let mut w = ClientWorld::default();
            let mut u = ClientUnit::new(p);
            u.mode = mode;
            u.path_dir = Some(60);
            u.path_turn = Some((54, 252));
            u.skills = Some(SkillList {
                entries: vec![SkillEntry {
                    skill,
                    owner: NATIVE,
                    ..SkillEntry::default()
                }],
                current: Some(0),
                ..SkillList::default()
            });
            w.units.insert(p, u);
            swing_restore(&mut w, &inputs, p);
            w.units[&p].path_turn
        };
        assert_eq!(run(18, 151), Some((60, 252)));
        assert_eq!(run(10, 151), Some((54, 252)));
        assert_eq!(run(18, 150), Some((54, 252)));
    }

    /// Measured: a Fallen
    /// (`opendoors` 1) walks with move mask 0x3401, as on the server; with
    /// 0x3C01 its blocked step's re-path found no path and the walk ended.
    #[test]
    fn the_client_monster_path_takes_the_server_move_mask() {
        let mask = |flying, open_doors| {
            let r = MonsterClass {
                flying,
                open_doors,
                ..MonsterClass::default()
            };
            monster_shape(&r, 19).move_mask()
        };
        assert_eq!(mask(false, true), 0x3401);
        assert_eq!(mask(false, false), 0x3C01);
        assert_eq!(mask(true, true), 0x1804);
    }

    /// Measured: a walking player whose
    /// server path aims at 49 turns 0 → 60, 56, 52, 49, one step per copy,
    /// and the update after each copy does not turn it again.
    #[test]
    fn a_walking_player_turns_once_per_copy_toward_the_server_aim() {
        if super::super::predict::path_tables().is_none() {
            return;
        }
        let key = UnitKey::new(0, 1);
        let mut w = ClientWorld::default();
        w.units
            .insert(key, super::super::world::ClientUnit::new(key));
        let s = ServerPath {
            key,
            mode: 6,
            x: 0,
            y: 0,
            xf: 0,
            yf: 0,
            tx: 0,
            ty: 0,
            copy: None,
            new_dir: Some(49),
        };
        let mut seen = Vec::new();
        for _ in 0..5 {
            player_aim(&mut w, &s);
            seen.push(w.units[&key].path_dir);
            assert!(w.units[&key].copy_turned);
        }
        assert_eq!(seen, [Some(60), Some(56), Some(52), Some(49), Some(49)]);
        // Not walking on the server: no aim, no turn.
        let mut w2 = ClientWorld::default();
        w2.units
            .insert(key, super::super::world::ClientUnit::new(key));
        player_aim(&mut w2, &ServerPath { mode: 5, ..s });
        assert_eq!(w2.units[&key].path_dir, None);
    }

    /// Measured (codes 0x01 / 0x17): a
    /// waypoint arrival's walk request to x + 3, y + 3 aims the standing
    /// player there and the update turns it 33 → 41, 49, 57, 0; a target
    /// on the own sub-tile or over 100 sub-tiles away sets no aim.
    #[test]
    fn a_walk_request_aims_the_player_at_its_point() {
        if super::super::predict::path_tables().is_none() {
            return;
        }
        let key = UnitKey::new(0, 1);
        let mut w = ClientWorld::default();
        let mut u = super::super::world::ClientUnit::new(key);
        u.position = Some((5000, 6000));
        u.precise = Some(((5000 << 16) | 0x8000, (6000 << 16) | 0x8000));
        u.path_dir = Some(33);
        w.units.insert(key, u);
        aim_walk(&mut w, key, 5003, 6003);
        assert_eq!(w.units[&key].path_turn, Some((0, 8)));
        let mut seen = Vec::new();
        for _ in 0..5 {
            player_turn(&mut w, key);
            seen.push(w.units[&key].path_dir);
        }
        assert_eq!(seen, [Some(41), Some(49), Some(57), Some(0), Some(0)]);
        for (x, y) in [(5000, 6000), (5101, 6000), (5000, 5899)] {
            let mut w2 = w.clone();
            w2.units.get_mut(&key).expect("unit").path_turn = None;
            aim_walk(&mut w2, key, x, y);
            assert_eq!(w2.units[&key].path_turn, None, "{x}, {y}");
        }
        // A stale precise position (another level's) is not read: the aim
        // starts at the cell centre (a2 town: D2RUST panicked on
        // the out-of-range tangent index otherwise).
        let mut w3 = w.clone();
        let u3 = w3.units.get_mut(&key).expect("unit");
        u3.precise = Some((100 << 16, 200 << 16));
        u3.path_turn = None;
        u3.path_dir = Some(33);
        aim_walk(&mut w3, key, 5003, 6003);
        assert_eq!(w3.units[&key].path_turn, Some((0, 8)));
    }

    /// A player knocked back with no server path copy this tick (the
    /// game's own link has none) gets its knockback path ended, so the
    /// player update leaves mode 19; a player whose copy is read keeps the
    /// copy's answer, and a monster is left alone.
    #[test]
    fn a_knockback_with_no_server_path_ends() {
        let player = UnitKey::new(0, 1);
        let other = UnitKey::new(0, 2);
        let monster = UnitKey::new(1, 7);
        let mut w = ClientWorld::default();
        for key in [player, other, monster] {
            let mut u = super::super::world::ClientUnit::new(key);
            u.mode = super::super::modes::player_mode::SEQUENCE;
            w.units.insert(key, u);
        }
        let copied = ServerPath {
            key: other,
            mode: 19,
            x: 0,
            y: 0,
            xf: 0,
            yf: 0,
            tx: 0,
            ty: 0,
            copy: Some(d2_sim::debug::state::PathCopy::default()),
            new_dir: None,
        };
        knockback_without_server_path(&mut w, &[copied]);
        assert_eq!(w.units[&player].path_live, Some(false));
        assert_eq!(w.units[&other].path_live, None);
        assert_eq!(w.units[&monster].path_live, None);
        // Not in knockback: untouched.
        let mut w2 = ClientWorld::default();
        w2.units
            .insert(player, super::super::world::ClientUnit::new(player));
        knockback_without_server_path(&mut w2, &[]);
        assert_eq!(w2.units[&player].path_live, None);
    }

    /// Measured on a Molten Boulder hit (unit 1:23): the client walk
    /// request aimed the Fallen
    /// at (5127, 4654), the server's at (5125, 4654); after the copies the
    /// client path's final target +0x18 is the server's, so the re-path
    /// after the t340 collision aims at (5125, 4654), finds the point
    /// (5127, 4654) and the walk goes on (with the request's own point the
    /// re-path found only the unit's cell, the walk ended, mode set 1, and
    /// the walk frame restarted three frames behind the base game).
    #[test]
    fn the_server_copy_takes_the_whole_target_block() {
        let key = UnitKey::new(1, 23);
        let mut w = ClientWorld::default();
        w.units
            .insert(key, super::super::world::ClientUnit::new(key));
        let mut p = DynamicPath {
            precise_x: (5128 << 16) | 0x8000,
            precise_y: (4653 << 16) | 0x8000,
            ..DynamicPath::default()
        };
        p.set_target_point(5127, 4654);
        p.put_final_target(Point::new(5127, 4654));
        w.objclient.s_paths.insert(key, p);
        let copy = d2_sim::debug::state::PathCopy {
            point_count: 1,
            points: vec![(5127, 4654)],
            moving: true,
            prev_target: (5125, 4654),
            final_target: (5125, 4654),
            ..d2_sim::debug::state::PathCopy::default()
        };
        let s = ServerPath {
            key,
            mode: 2,
            x: 5128,
            y: 4653,
            xf: 188,
            yf: 65348,
            tx: 5127,
            ty: 4654,
            copy: Some(copy),
            new_dir: None,
        };
        server_copy(&mut w, &ModelInputs::default(), &s);
        let p = &w.objclient.s_paths[&key];
        assert_eq!(p.target(), Point::new(5127, 4654));
        assert_eq!(p.prev_target(), Point::new(5125, 4654));
        assert_eq!(p.final_target(), Point::new(5125, 4654));
    }

    /// The copy's placement recache: a move inside the unit's room leaves
    /// the room list as it is. Measured on a Fire Ball hit: the dying
    /// Fallen 1:24 stays behind
    /// the fire ball's client missiles made after it; a re-prepend per
    /// copied move put it at the head of its room run.
    #[test]
    fn a_server_copy_move_inside_the_room_keeps_the_list_place() {
        use crate::bridge::drlg::DrlgRoomId;
        use crate::bridge::world::ActiveRoom;
        let key = UnitKey::new(1, 24);
        let other = UnitKey::new(3, 288);
        let room = ActiveRoom {
            x0: 5120,
            y0: 4640,
            w: 40,
            h: 40,
            level: 1,
            room: DrlgRoomId(7),
        };
        let mut w = ClientWorld {
            active_rooms: Some(vec![room]),
            ..ClientWorld::default()
        };
        w.units
            .insert(key, super::super::world::ClientUnit::new(key));
        let p = DynamicPath {
            precise_x: (5128 << 16) | 0x8000,
            precise_y: (4653 << 16) | 0x8000,
            ..DynamicPath::default()
        };
        w.objclient.s_paths.insert(key, p);
        w.room_units.place(key, Some(DrlgRoomId(7)));
        w.room_units.place(other, Some(DrlgRoomId(7)));
        let s = ServerPath {
            key,
            mode: 2,
            x: 5129,
            y: 4653,
            xf: 0x8000,
            yf: 0x8000,
            tx: 5129,
            ty: 4653,
            copy: None,
            new_dir: None,
        };
        server_copy(&mut w, &ModelInputs::default(), &s);
        assert_eq!(w.objclient.s_paths[&key].x(), 5129);
        assert_eq!(w.room_units.list(DrlgRoomId(7)), [other, key]);
    }

    /// The S-player copy goes through the same placement recache: a copied
    /// move of the local player inside its room keeps its list place.
    /// Measured on a Fire Ball flight: Fallen 1:29 (y 78443) is
    /// drawn before Shaman 1:28 (y 78448); a re-prepend per copied player
    /// move made the player sink past both in the one-pass sort, so the
    /// two were never compared.
    #[test]
    fn a_server_copy_player_move_inside_the_room_keeps_the_list_place() {
        use crate::bridge::drlg::DrlgRoomId;
        use crate::bridge::world::ActiveRoom;
        let key = UnitKey::new(0, 1);
        let other = UnitKey::new(1, 28);
        let room = ActiveRoom {
            x0: 5120,
            y0: 4640,
            w: 40,
            h: 40,
            level: 1,
            room: DrlgRoomId(7),
        };
        let mut w = ClientWorld {
            active_rooms: Some(vec![room]),
            local_player: Some(key),
            ..ClientWorld::default()
        };
        let mut u = super::super::world::ClientUnit::new(key);
        u.position = Some((5150, 4656));
        w.units.insert(key, u);
        w.room_units.place(key, Some(DrlgRoomId(7)));
        w.room_units.place(other, Some(DrlgRoomId(7)));
        let s = ServerPath {
            key,
            mode: 2,
            x: 5151,
            y: 4656,
            xf: 0x8000,
            yf: 0x8000,
            tx: 5156,
            ty: 4656,
            copy: None,
            new_dir: None,
        };
        server_copy(&mut w, &ModelInputs::default(), &s);
        assert_eq!(w.units[&key].position, Some((5151, 4656)));
        assert_eq!(w.room_units.list(DrlgRoomId(7)), [other, key]);
    }

    /// Measured: the local
    /// player's walk steps in its own update, so a Fallen updated after it
    /// re-paths toward where it stands this tick; a standing player has no
    /// path step, and the pass end hands the copy back the old position.
    #[test]
    fn the_local_player_walk_steps_in_its_own_update() {
        let key = UnitKey::new(0, 1);
        let mut w = ClientWorld {
            local_player: Some(key),
            ..ClientWorld::default()
        };
        let mut u = super::super::world::ClientUnit::new(key);
        u.position = Some((5150, 4656));
        u.mode = 1;
        w.units.insert(key, u);
        let s = ServerPath {
            key,
            mode: 2,
            x: 5151,
            y: 4656,
            xf: 0x8000,
            yf: 0x4000,
            tx: 5156,
            ty: 4656,
            copy: None,
            new_dir: None,
        };
        assert_eq!(local_path_step(&mut w, &s), None, "NU: no step");
        w.units.get_mut(&key).unwrap().mode = 2;
        let step = local_path_step(&mut w, &s).expect("WL steps");
        assert_eq!(w.units[&key].position, Some((5151, 4656)));
        assert_eq!(
            w.units[&key].precise,
            Some(((5151 << 16) | 0x8000, (4656 << 16) | 0x4000))
        );
        local_path_step_end(&mut w, step);
        assert_eq!(w.units[&key].position, Some((5150, 4656)));
        assert_eq!(w.units[&key].precise, None);
        // A message of the pass that placed the player keeps its point.
        let step = local_path_step(&mut w, &s).expect("WL steps");
        w.units.get_mut(&key).unwrap().position = Some((5160, 4660));
        local_path_step_end(&mut w, step);
        assert_eq!(w.units[&key].position, Some((5160, 4660)));
    }

    /// Measured: Warriv's 0x67 carries path type 7, so his client path
    /// takes the straight type and its flags
    /// (0x1000: a blocked goal is prepared, not walked to).
    #[test]
    fn a_pathed_request_sets_the_path_type_of_r3() {
        let Some(t) = super::super::predict::path_tables() else {
            return;
        };
        let key = UnitKey::new(1, 7);
        let mut w = ClientWorld::default();
        w.objclient.s_paths.insert(
            key,
            DynamicPath {
                path_type: 2,
                ..DynamicPath::default()
            },
        );
        server_path_type(&mut w, key, 7);
        let p = &w.objclient.s_paths[&key];
        assert_eq!(p.path_type, 7);
        assert_eq!(p.flags & 0x1000, 0x1000);
        assert_eq!(p.flags & t.pathtype_flags[7], t.pathtype_flags[7]);
        // A type the table does not hold leaves the path as it is.
        server_path_type(&mut w, key, 99);
        assert_eq!(w.objclient.s_paths[&key].path_type, 7);
    }

    /// A recorded town chicken (93): 15 → 0 in
    /// steps of −4 (15, 11, 7, 3, then snapped to 0).
    #[test]
    fn a_turn_steps_toward_the_new_direction_and_snaps_when_it_passes() {
        let mut p = path(15, 0, -4);
        let mut seen = Vec::new();
        for _ in 0..5 {
            turn(&mut p);
            seen.push(p.direction);
        }
        assert_eq!(seen, [11, 7, 3, 0, 0]);
    }

    #[test]
    fn an_upward_turn_wraps_through_63() {
        let mut p = path(60, 2, 4);
        turn(&mut p);
        assert_eq!(p.direction, 0);
        turn(&mut p);
        assert_eq!(p.direction, 2);
    }

    #[test]
    fn path_flag_0x40_holds_the_direction() {
        let mut p = path(10, 20, 4);
        p.flags |= 0x40;
        turn(&mut p);
        assert_eq!(p.direction, 10);
    }
}
