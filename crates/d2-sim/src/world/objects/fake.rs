//! A recording fake of [`ObjectWorld`] for the object tests. The
//! extension traits are implemented for [`Fake`] in each submodule's test
//! file (`chests/tests.rs`, `shrines/tests.rs`, `misc/tests.rs`).

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use d2_data::tables::{Levels, Objects, Record, Shrines};

use super::*;

/// A blank `objects.txt` row.
pub fn blank_object() -> Objects {
    Objects::decode(&[0; Objects::SIZE])
}
/// A blank `shrines.txt` row.
pub fn blank_shrine() -> Shrines {
    Shrines::decode(&[0; Shrines::SIZE])
}
/// A blank `levels.txt` row.
pub fn blank_level() -> Levels {
    Levels::decode(&[0; Levels::SIZE])
}

/// A recorded box query: (x, y, sx, sy, mask).
pub type BoxQuery = (i32, i32, u32, u32, u32);
/// Predicate over a box query: `true` = blocked.
pub type BlockFn = fn(i32, i32, u32, u32, u32) -> bool;

/// One recorded seam call, in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Mode(UnitId, u8, bool),
    Anim(UnitId, i32, i32, i16),
    Schedule(UnitId, u8, i32),
    CancelTimers(UnitId),
    Stamp(UnitId),
    Free(UnitId),
    Sound(UnitId, u8, Option<UnitId>, bool),
    KeyTest(UnitId),
    Allocate(RoomId, u16, i32, i32, u8),
    Queue(UnitId),
    /// Free-form record for extension-trait fakes.
    Other(String),
}

#[derive(Debug, Clone, Default)]
pub struct Fake {
    pub frame: i32,
    pub tick: u32,
    pub guids: BTreeMap<UnitId, u32>,
    pub operators: BTreeMap<UnitId, Operator>,
    pub modes: BTreeMap<UnitId, u8>,
    pub seeds: BTreeMap<UnitId, Seed>,
    pub flags: BTreeMap<UnitId, u32>,
    pub rooms: BTreeMap<UnitId, RoomId>,
    pub levels: BTreeMap<UnitId, u32>,
    /// Level id of a room (`room_level`).
    pub room_levels: BTreeMap<RoomId, u32>,
    pub positions: BTreeMap<UnitId, (i32, i32)>,
    /// Players that pass the key test.
    pub keys: BTreeSet<UnitId>,
    /// Operators out of interact range.
    pub out_of_range: BTreeSet<UnitId>,
    pub interact_active: BTreeSet<UnitId>,
    pub busy: BTreeSet<UnitId>,
    pub cursor: BTreeSet<UnitId>,
    /// The next unit id `allocate_object` returns (incremented per call);
    /// `None`: allocation fails.
    pub next_alloc: Option<u32>,
    pub staff_tomb: u32,
    /// Stats per unit (for extension fakes).
    pub stats: BTreeMap<(UnitId, u16), i32>,
    pub calls: Vec<Call>,
    /// Units of a room in room-list order (mech / chest fakes).
    pub room_unit_lists: BTreeMap<RoomId, Vec<UnitId>>,
    /// Unit type / class of a unit (mech / chest fakes).
    pub unit_types: BTreeMap<UnitId, u8>,
    pub unit_classes: BTreeMap<UnitId, u32>,
    /// Adjacency arrays of a room.
    pub adjacent: BTreeMap<RoomId, Vec<RoomId>>,
    /// Gold placeholder fakes: a point is inside the init room when this
    /// returns true; every point is free when `free_all`.
    pub room_at_fn: Option<fn(i32, i32) -> bool>,
    pub free_all: bool,
    /// Box queries seen by the population fake: (x, y, sx, sy, mask).
    pub box_log: std::cell::RefCell<Vec<BoxQuery>>,
    /// Predicate for the population fake's box query: `true` = blocked.
    pub block: Option<BlockFn>,
    /// The game's rules (default `Original`; a mod's in the MS8b tests).
    pub rules: crate::rules::Ruleset,
}

impl Fake {
    pub fn calls_of(&self, f: impl Fn(&Call) -> bool) -> Vec<Call> {
        self.calls.iter().filter(|c| f(c)).cloned().collect()
    }
    pub fn schedules(&self) -> Vec<(UnitId, u8, i32)> {
        self.calls
            .iter()
            .filter_map(|c| match c {
                Call::Schedule(u, e, f) => Some((*u, *e, *f)),
                _ => None,
            })
            .collect()
    }
}

impl ObjectWorld for Fake {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn host_tick(&self) -> u32 {
        self.tick
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.guids.get(&unit).copied().unwrap_or(unit.0)
    }
    fn find_object(&self, guid: u32) -> Option<UnitId> {
        self.guids
            .iter()
            .find(|(u, g)| **g == guid && !self.operators.contains_key(u))
            .map(|(u, _)| *u)
    }
    fn operator(&self, unit: UnitId) -> Operator {
        self.operators
            .get(&unit)
            .copied()
            .unwrap_or(Operator::Other)
    }
    fn mode(&self, unit: UnitId) -> u8 {
        self.modes.get(&unit).copied().unwrap_or(0)
    }
    fn write_mode(&mut self, unit: UnitId, mode: u8, queue: bool) {
        self.modes.insert(unit, mode);
        *self.flags.entry(unit).or_default() |= oflags::CHANGED;
        self.calls.push(Call::Mode(unit, mode, queue));
    }
    fn set_anim(&mut self, unit: UnitId, frame_count: i32, frame: i32, speed: i16) {
        self.calls.push(Call::Anim(unit, frame_count, frame, speed));
    }
    fn unit_seed(&mut self, unit: UnitId) -> Option<&mut Seed> {
        self.seeds.get_mut(&unit)
    }
    fn flags(&self, unit: UnitId) -> u32 {
        self.flags.get(&unit).copied().unwrap_or(0)
    }
    fn set_flags(&mut self, unit: UnitId, flags: u32) {
        self.flags.insert(unit, flags);
    }
    fn queue_update(&mut self, unit: UnitId) {
        self.calls.push(Call::Queue(unit));
    }
    fn room(&self, unit: UnitId) -> Option<RoomId> {
        self.rooms.get(&unit).copied()
    }
    fn level(&self, unit: UnitId) -> Option<u32> {
        self.levels.get(&unit).copied()
    }
    fn room_level(&self, room: RoomId) -> Option<u32> {
        self.room_levels.get(&room).copied()
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.positions.get(&unit).copied().unwrap_or((0, 0))
    }
    fn schedule(&mut self, unit: UnitId, ev: u8, frame: i32) {
        self.calls.push(Call::Schedule(unit, ev, frame));
    }
    fn cancel_timers(&mut self, unit: UnitId) {
        self.calls.push(Call::CancelTimers(unit));
    }
    fn stamp_footprint(&mut self, unit: UnitId, _: Option<RoomId>, _: i32, _: i32) {
        self.calls.push(Call::Stamp(unit));
    }
    fn free_footprint(&mut self, unit: UnitId) {
        self.calls.push(Call::Free(unit));
    }
    fn sound(&mut self, unit: UnitId, id: u8, to: Option<UnitId>, now: bool) {
        self.calls.push(Call::Sound(unit, id, to, now));
    }
    fn key_test(&mut self, player: UnitId) -> bool {
        self.calls.push(Call::KeyTest(player));
        self.keys.contains(&player)
    }
    fn in_interact_range(&self, operator: UnitId, _object: UnitId) -> bool {
        !self.out_of_range.contains(&operator)
    }
    fn interact_active(&self, player: UnitId) -> bool {
        self.interact_active.contains(&player)
    }
    fn player_busy(&self, player: UnitId) -> bool {
        self.busy.contains(&player)
    }
    fn cursor_item(&self, player: UnitId) -> bool {
        self.cursor.contains(&player)
    }
    fn allocate_object(
        &mut self,
        room: RoomId,
        class: u16,
        x: i32,
        y: i32,
        mode: u8,
    ) -> Option<UnitId> {
        self.calls.push(Call::Allocate(room, class, x, y, mode));
        let id = self.next_alloc?;
        self.next_alloc = Some(id + 1);
        let u = UnitId(id);
        self.modes.insert(u, mode);
        self.rooms.insert(u, room);
        self.positions.insert(u, (x, y));
        Some(u)
    }
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.room_at_fn.and_then(|f| f(x, y).then_some(room))
    }
    fn gold_drop(&mut self, _room: RoomId, x: i32, y: i32) {
        self.calls.push(Call::Other(format!("gold {x} {y}")));
    }
    fn point_free(&self, _room: RoomId, _x: i32, _y: i32, _mask: u32) -> bool {
        self.free_all
    }
    fn staff_tomb_level(&self) -> u32 {
        self.staff_tomb
    }
    fn store_mode(&mut self, unit: UnitId, mode: u8) {
        self.modes.insert(unit, mode);
        self.calls
            .push(Call::Other(format!("store {} {mode}", unit.0)));
    }
}
