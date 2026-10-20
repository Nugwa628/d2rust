//! A scripted host for population tests: one or more rooms, units with
//! seeds, a call log.

use std::collections::{BTreeMap, BTreeSet};

use crate::monsters::population::seams::{Alloc, MonsterInit, OwnerKey, PopWorld};
use crate::monsters::population::{
    CoordRect, Ctx, GameInfo, LevelPop, Mon2Pop, MonPop, PopState, PopTables, PresetUnit, RoomBox,
    TileRec,
};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

pub const R0: RoomId = RoomId(0);

#[derive(Clone, Debug)]
pub struct FakeUnit {
    pub class: i32,
    pub x: i32,
    pub y: i32,
    pub room: RoomId,
    pub mode: u8,
    pub seed: Seed,
    pub type_flags: u16,
    pub align: u8,
    pub unit_flags: u32,
    pub monster_flags: u32,
}

#[derive(Default)]
pub struct Fake {
    pub game_seed: Seed,
    pub room_seeds: BTreeMap<RoomId, Seed>,
    pub level: i32,
    pub populated_level: Option<i32>,
    pub room_count: i32,
    pub coords: Vec<CoordRect>,
    pub default_index: i32,
    pub index_map: BTreeMap<(i32, i32), i32>,
    pub boxes: BTreeMap<RoomId, RoomBox>,
    pub warps: Vec<(i32, i32)>,
    pub spawn_loc: Option<(i32, i32)>,
    /// Points that collide.
    pub blocked: BTreeSet<(i32, i32)>,
    /// Points outside every room.
    pub void: BTreeSet<(i32, i32)>,
    pub masked: BTreeSet<(i32, i32)>,
    pub tiles: Vec<TileRec>,
    pub presets: Vec<PresetUnit>,
    pub clients: u32,
    pub quest_flags: BTreeSet<u8>,
    pub chaos: bool,
    pub units: Vec<FakeUnit>,
    /// `boss_modifiers` makes the boss a champion.
    pub champion: bool,
    pub alloc_fail: bool,
    /// Points where allocation fails.
    pub alloc_fail_at: BTreeSet<(i32, i32)>,
    pub log: Vec<String>,
    /// Every collision test (x, y, size, mask).
    pub collide_calls: std::cell::RefCell<Vec<(i32, i32, i32, u16)>>,
    /// The nearest free point, if set; else the asked point.
    pub nearest: Option<(i32, i32)>,
    /// The room of the nearest free point, if set; else the asked room.
    pub nearest_room: Option<RoomId>,
    /// Every point the nearest free point was searched from.
    pub nearest_asked: std::cell::RefCell<Vec<(i32, i32)>>,
}

impl Fake {
    pub fn new() -> Self {
        let mut f = Self {
            level: 2,
            room_count: 10,
            default_index: 1,
            ..Self::default()
        };
        f.room_seeds.insert(R0, Seed::init());
        f.boxes.insert(
            R0,
            RoomBox {
                x: 0,
                y: 0,
                width: 1000,
                height: 1000,
            },
        );
        f
    }

    /// A unit placed directly (not through population).
    pub fn add_unit(&mut self, class: i32, x: i32, y: i32, seed: Seed) -> UnitId {
        self.units.push(FakeUnit {
            class,
            x,
            y,
            room: R0,
            mode: 1,
            seed,
            type_flags: 0,
            align: 0,
            unit_flags: 0,
            monster_flags: 0,
        });
        UnitId(self.units.len() as u32 - 1)
    }

    pub fn unit(&self, u: UnitId) -> &FakeUnit {
        &self.units[u.0 as usize]
    }

    pub fn allocs(&self) -> Vec<i32> {
        self.units.iter().map(|u| u.class).collect()
    }

    pub fn calls(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(prefix)).count()
    }
}

impl PopWorld for Fake {
    fn game_seed(&mut self) -> &mut Seed {
        &mut self.game_seed
    }
    fn room_seed(&mut self, room: RoomId) -> &mut Seed {
        self.room_seeds.entry(room).or_default()
    }
    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed {
        &mut self.units[unit.0 as usize].seed
    }
    fn room_level(&self, _room: RoomId) -> i32 {
        self.level
    }
    fn populated_level(&self, _room: RoomId) -> i32 {
        self.populated_level.unwrap_or(self.level)
    }
    fn populated_room_count(&mut self, _act: u8, _level: i32) -> i32 {
        self.room_count
    }
    fn coord_list(&self, _room: RoomId) -> Vec<CoordRect> {
        self.coords.clone()
    }
    fn coord_at(&self, _room: RoomId, _x: i32, _y: i32) -> Option<CoordRect> {
        self.coords.first().copied()
    }
    fn coord_index_at(&self, _room: RoomId, x: i32, y: i32) -> i32 {
        *self.index_map.get(&(x, y)).unwrap_or(&self.default_index)
    }
    fn room_box(&self, room: RoomId) -> RoomBox {
        self.boxes.get(&room).copied().unwrap_or_default()
    }
    fn warp_points(&self, _room: RoomId) -> Vec<(i32, i32)> {
        self.warps.clone()
    }
    fn spawn_location(&mut self, _room: RoomId, kind: u8) -> Option<(i32, i32)> {
        assert_eq!(kind, 11);
        self.spawn_loc
    }
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        (!self.void.contains(&(x, y))).then_some(room)
    }
    fn collides(&self, _room: RoomId, x: i32, y: i32, size: i32, mask: u16) -> bool {
        self.collide_calls.borrow_mut().push((x, y, size, mask));
        self.blocked.contains(&(x, y))
    }
    fn mask_at(&self, _room: RoomId, x: i32, y: i32, mask: u16) -> bool {
        assert_eq!(mask, 0x100);
        self.masked.contains(&(x, y))
    }
    fn tile_records(&self, _room: RoomId) -> Vec<TileRec> {
        self.tiles.clone()
    }
    fn preset_units(&self, _room: RoomId) -> Vec<PresetUnit> {
        self.presets.clone()
    }
    fn client_count(&self, _room: RoomId) -> u32 {
        self.clients
    }
    fn unit_room(&self, unit: UnitId) -> Option<RoomId> {
        Some(self.unit(unit).room)
    }
    fn unit_position(&self, unit: UnitId) -> (i32, i32) {
        let u = self.unit(unit);
        (u.x, u.y)
    }
    fn quest_flag(&self, flag: u8) -> bool {
        self.quest_flags.contains(&flag)
    }
    fn chaos_blocks_population(&self) -> bool {
        self.chaos
    }
    fn nearest_free_point(&self, room: RoomId, x: i32, y: i32) -> Option<(RoomId, i32, i32)> {
        self.nearest_asked.borrow_mut().push((x, y));
        let (x, y) = self.nearest.unwrap_or((x, y));
        Some((self.nearest_room.unwrap_or(room), x, y))
    }
}

impl MonsterInit for Fake {
    fn allocate_monster(&mut self, a: Alloc, _: &mut PopState) -> Option<UnitId> {
        if self.alloc_fail || self.alloc_fail_at.contains(&(a.x, a.y)) {
            return None;
        }
        // One game-seed step for the unit seed.
        let seed = self.game_seed.derive();
        self.log.push(format!(
            "alloc {} {} {} m{}{}",
            a.class,
            a.x,
            a.y,
            a.mode,
            a.guid.map_or(String::new(), |g| format!(" g{g}"))
        ));
        let u = self.add_unit(a.class, a.x, a.y, seed);
        self.units[u.0 as usize].room = a.room;
        self.units[u.0 as usize].mode = a.mode;
        Some(u)
    }
    fn set_monster_flag(&mut self, unit: UnitId, flag: u32) {
        self.units[unit.0 as usize].monster_flags |= flag;
    }
    fn set_coord_record(
        &mut self,
        unit: UnitId,
        rect: Option<CoordRect>,
        _: RoomId,
        _: i32,
        _: i32,
    ) {
        self.log
            .push(format!("coord {} {}", unit.0, rect.is_some()));
    }
    fn set_alignment(&mut self, unit: UnitId, align: u8) {
        self.units[unit.0 as usize].align = align;
    }
    fn set_unit_flags(&mut self, unit: UnitId, flags: u32) {
        self.units[unit.0 as usize].unit_flags |= flags;
    }
    fn class_extras(&mut self, unit: UnitId, _: &mut PopState) {
        self.log.push(format!("extras {}", unit.0));
    }
    fn init_monster(&mut self, unit: UnitId, _: &mut PopState) {
        self.log.push(format!("init {}", unit.0));
    }
    fn unit_class(&self, unit: UnitId) -> i32 {
        self.unit(unit).class
    }
    fn unit_level(&self, _unit: UnitId) -> i32 {
        self.level
    }
    fn type_flags(&self, unit: UnitId) -> u16 {
        self.unit(unit).type_flags
    }
    fn set_type_flags(&mut self, unit: UnitId, flags: u16) {
        self.units[unit.0 as usize].type_flags |= flags;
    }
    fn boss_modifiers(&mut self, boss: UnitId, champion_allowed: bool) {
        self.log
            .push(format!("bossmods {} {champion_allowed}", boss.0));
        if self.champion && champion_allowed {
            self.units[boss.0 as usize].type_flags |= 4;
        }
    }
    fn boss_modifier_init(&mut self, boss: UnitId) {
        self.log.push(format!("modinit {}", boss.0));
    }
    fn add_modifier(&mut self, unit: UnitId, m: u8, _: &mut PopState) {
        self.log.push(format!("mod {} {m}", unit.0));
        if m == 16 {
            self.units[unit.0 as usize].type_flags |= 4;
        }
    }
    fn transfer_modifiers(&mut self, boss: UnitId, minion: UnitId) {
        self.log.push(format!("xfer {} {}", boss.0, minion.0));
    }
    fn set_owner_data(&mut self, unit: UnitId, owner: OwnerKey, a: i32, b: i32, c: i32) {
        self.log
            .push(format!("owner {} {owner:?} {a} {b} {c}", unit.0));
    }

    fn add_minion(&mut self, leader: UnitId, minion: UnitId) {
        self.log.push(format!("minion {} {}", leader.0, minion.0));
    }
    fn set_owner(&mut self, minion: UnitId, owner: UnitId) {
        self.log.push(format!("setowner {} {}", minion.0, owner.0));
    }
    fn boss_quest_hook(&mut self, boss: UnitId) {
        self.log.push(format!("quest {}", boss.0));
    }
    fn superunique_init(&mut self, boss: UnitId, su: i32) {
        self.log.push(format!("suinit {} {su}", boss.0));
    }

    fn create_object(&mut self, _room: RoomId, class: i32, x: i32, y: i32) {
        self.log.push(format!("object {class} {x} {y}"));
    }

    fn move_preset_path(&mut self, unit: UnitId, _room: RoomId, index: usize) {
        self.log.push(format!("path {} {index}", unit.0));
    }
    fn preset_created(&mut self, unit: UnitId, preset: &PresetUnit) {
        self.log
            .push(format!("preset {} {}", unit.0, preset.has_data));
    }
    fn schedule_monumod(&mut self, unit: UnitId) {
        self.log.push(format!("monumod {}", unit.0));
    }
    fn change_alignment(&mut self, unit: UnitId, a: i32, b: i32) {
        self.log.push(format!("realign {} {a} {b}", unit.0));
    }
    fn restore_inactive_units(&mut self, room: RoomId) {
        self.log.push(format!("restore {}", room.0));
    }
    fn populate_objects(&mut self, room: RoomId) {
        self.log.push(format!("objects {}", room.0));
    }
}

/// Tables: 600 monstats rows (BaseId = own row, no chain, no minions,
/// monstats2 row 0), one monstats2 row, 140 levels, 10 superuniques.
pub fn tables() -> PopTables {
    let mon = |i: usize| MonPop {
        base_id: i as i16,
        next_in_class: -1,
        mon_stats_ex: 0,
        spawn: -1,
        minion1: -1,
        minion2: -1,
        is_spawn: true,
        ..MonPop::default()
    };
    PopTables {
        levels: vec![LevelPop::default(); 140],
        monstats: (0..600).map(mon).collect(),
        monstats2: vec![Mon2Pop::default()],
        superuniques: vec![Default::default(); 10],
    }
}

/// A state with one region for level `level` (others empty).
pub fn state_with(t: &PopTables, level: i32) -> PopState {
    let mut s = PopState::default();
    s.regions.slots = vec![None; t.levels.len()];
    let r = crate::monsters::population::Region {
        level_id: level,
        room_count: 10,
        mon_den: 520,
        ai_field: -1,
        ..Default::default()
    };
    s.regions.slots[level as usize] = Some(r);
    s
}

pub fn ctx<'a>(t: &'a PopTables, s: &'a mut PopState, f: &'a mut Fake) -> Ctx<'a, Fake> {
    Ctx {
        tables: t,
        info: GameInfo::default(),
        state: s,
        host: f,
    }
}

/// The first seed `{lo, 666}` (lo ≥ 1) whose next draws mod `m` are `want`.
pub fn seed_for(m: u32, want: &[u32]) -> Seed {
    for lo in 1.. {
        let mut s = Seed::init_low(lo);
        if want.iter().all(|&w| s.step() % m == w) {
            return Seed::init_low(lo);
        }
    }
    unreachable!()
}
