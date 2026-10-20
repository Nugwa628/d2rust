//! Tests for the action wiring: the real modules run together through the action
//! adapters, on synthetic tables and a synthetic DRLG level (two 8×8
//! preset rooms, streamed). Only the seams without a provider
//! ([`Pending`]) are faked: a straight-line path, positions, hostility.
//! Each submodule covers one area (combat, death, objects, rooms, ...); this one
//! holds the shared fixture `Fx`.

#[cfg(test)]
mod act_time;
#[cfg(test)]
mod ai;
#[cfg(test)]
mod ai_scan;
#[cfg(test)]
mod combat;
#[cfg(test)]
mod death;
#[cfg(test)]
mod e2e;
pub mod fight;
#[cfg(test)]
mod inactive;
#[cfg(test)]
mod inactive_sockets;
#[cfg(test)]
mod missiles;
#[cfg(test)]
mod objects;
#[cfg(test)]
mod player_death;
#[cfg(test)]
mod proto_msgs;
mod remove_callbacks;
#[cfg(test)]
mod rooms;
#[cfg(test)]
mod sound;
#[cfg(test)]
mod travel;
#[cfg(test)]
mod waypoints;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Levels, Missiles as MissileRow};

use super::*;
use crate::drlg::room::LinkAt;
use crate::drlg::tiles::{cell, FIXED_LIBRARY};
use crate::drlg::{
    CellGrid, Drlg, DrlgRoomId, GridPass, LevelDef, LevelIdx, RoomGrids, RoomKind, TileInfo,
    TileRect,
};
use crate::skills::fake::{blank, combat_tables, monster_rec, skill_rec, skill_tables};
use crate::units::hooks::MonsterInfo;
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};

/// The DRLG init seed of the fixture's act.
pub const INIT: u32 = 644_409_375;
/// The fixture's level (DRLG type 2, level type 1).
pub const LEVEL: u32 = 2;
/// Sub-tile column with a missile barrier (tile x = 4 of room A).
pub const BARRIER_X: i32 = 20;

// ---- seams without a provider -----------------------------------------------

/// A straight-line path: one sub-tile per step along x toward the target
/// point; positions, hostility (everyone but oneself) and a log of the
/// pending calls that matter.
#[derive(Default)]
pub struct TestPending {
    pub pos: BTreeMap<UnitId, (i32, i32)>,
    pub dir: BTreeMap<UnitId, i32>,
    pub crossed: BTreeMap<UnitId, Vec<(i32, i32)>>,
    pub velocity: BTreeMap<UnitId, i32>,
    /// COF names per (unit type, mode) for the AnimData lookup.
    pub names: BTreeMap<(UnitType, u32), [u8; 8]>,
    pub log: Vec<String>,
    /// Messages sent to players' clients, in order.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// (operator, object) of every interact-range test, in order.
    pub ranged: std::cell::RefCell<Vec<(UnitId, UnitId)>>,
    /// The corpse `create_corpse` answers and its owner GUID.
    pub corpse: Option<(UnitId, u32)>,
    /// Modes `class_has_mode` denies (every other mode exists).
    pub missing_modes: Vec<u8>,
    /// Used skills by unit.
    pub used: BTreeMap<UnitId, crate::skills::SkillEntry>,
    /// Used-skill entry flags by unit (default 0).
    pub entry_flags: BTreeMap<UnitId, u32>,
    /// What the monster skill start answers (logged).
    pub skill_start: i32,
    /// What `object_approach` answers (default: operate).
    pub reach: Option<crate::wiring::action::ObjectReach>,
    /// Defenders `may_attack` denies (default: everyone but oneself).
    pub peaceful: Vec<UnitId>,
    /// What `object_quest_record` answers (the portal's quest gate).
    pub quest_record: bool,
    /// Unit sizes (default 1).
    pub sizes: BTreeMap<UnitId, i32>,
    /// The weapon in use by unit (`current_weapon`).
    pub weapons: BTreeMap<UnitId, UnitId>,
    /// The weapon in use by unit (`weapon_in_use`).
    pub in_use: BTreeMap<UnitId, UnitId>,
    /// The item type of an item (`item_is`, exact type only).
    pub item_types: BTreeMap<UnitId, i32>,
    /// The damage weapon by (unit, weapsel, flag 0x2000); absent: not
    /// answered.
    pub damage_weapons: BTreeMap<(UnitId, u8, bool), Option<UnitId>>,
    /// Items whose items row passes the durability test (the row half of
    /// it).
    pub durable_rows: Vec<UnitId>,
    /// Socketed children by parent item (`Pending::socketed_items`).
    pub sockets: BTreeMap<UnitId, Vec<UnitId>>,
    /// (parent, child) of each `Pending::socket_restore`.
    pub socket_restored: Vec<(UnitId, UnitId)>,
}

impl crate::wiring::worldgen::WorldPending for TestPending {}

impl Pending for TestPending {
    fn socketed_items(&self, item: UnitId) -> Vec<UnitId> {
        self.sockets.get(&item).cloned().unwrap_or_default()
    }
    fn socket_restore(&mut self, _: &mut Game, parent: UnitId, child: UnitId) {
        self.socket_restored.push((parent, child));
    }
    fn item_has_durability(&self, item: UnitId) -> bool {
        self.durable_rows.contains(&item)
    }
    fn object_quest_record(&self, _: UnitId) -> bool {
        self.quest_record
    }
    fn anim_name(&self, _: UnitId, ty: UnitType, _: u32, mode: u32) -> Option<[u8; 8]> {
        self.names.get(&(ty, mode)).copied()
    }
    fn anim_rate(&self, _: UnitId, speed: Option<u32>) -> i16 {
        speed.map_or(0, |s| s as i16)
    }
    fn kill_step(&mut self, _: &mut Game, step: KillStep, d: UnitId, a: UnitId) {
        self.log.push(format!("kill {step:?} {} {}", d.0, a.0));
    }
    fn level_up_event(&mut self, unit: UnitId) {
        self.log.push(format!("level up {}", unit.0));
    }
    fn death_drop_gold(&mut self, _: &mut Game, unit: UnitId, amount: i32) {
        self.log.push(format!("drop gold {} {amount}", unit.0));
    }
    fn create_corpse(&mut self, _: &mut Game, _: UnitId) -> Option<UnitId> {
        self.corpse.map(|c| c.0)
    }
    fn corpse_owner_guid(&self, corpse: UnitId) -> Option<u32> {
        self.corpse.filter(|c| c.0 == corpse).map(|c| c.1)
    }
    /// The death start's body is not written: the fake sets mode DT, as
    /// a start function sets its mode.
    fn monster_death_start(
        h: &mut ActionHooks<Self>,
        sim: &mut crate::units::hooks::Sim<'_>,
        unit: UnitId,
        target: Option<UnitId>,
    ) -> bool {
        h.x.log
            .push(format!("death start {} {:?}", unit.0, target.map(|t| t.0)));
        crate::units::modes::set_mode(sim, h, unit, 0).expect("mode DT");
        true
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or((0, 0))
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn size(&self, u: UnitId) -> i32 {
        self.sizes.get(&u).copied().unwrap_or(1)
    }
    fn has_path(&self, _: UnitId) -> bool {
        true
    }
    fn set_velocity(&mut self, unit: UnitId, v: i32) {
        self.velocity.insert(unit, v);
    }
    fn velocity(&self, unit: UnitId) -> i32 {
        self.velocity.get(&unit).copied().unwrap_or(0)
    }
    fn set_target_point(&mut self, unit: UnitId, x: i32, _: i32) {
        let (ux, _) = self.position(unit);
        self.dir.insert(unit, (x - ux).signum());
    }
    fn target_distance(&self, _: UnitId) -> i32 {
        10
    }
    fn step(&mut self, _: &mut Game, unit: UnitId) -> bool {
        let d = self.dir.get(&unit).copied().unwrap_or(1);
        let (x, y) = self.position(unit);
        let p = (x + if d == 0 { 1 } else { d }, y);
        self.pos.insert(unit, p);
        self.crossed.insert(unit, vec![p]);
        true
    }
    fn crossed_subtiles(&self, unit: UnitId) -> Vec<(i32, i32)> {
        self.crossed.get(&unit).cloned().unwrap_or_default()
    }
    fn may_attack(&self, a: UnitId, d: UnitId) -> bool {
        a != d && !self.peaceful.contains(&d)
    }
    fn class_has_mode(&self, _: i32, mode: u8) -> bool {
        !self.missing_modes.contains(&mode)
    }
    fn used_skill(&self, unit: UnitId) -> Option<crate::skills::SkillEntry> {
        self.used.get(&unit).copied()
    }
    fn current_weapon(&self, unit: UnitId) -> Option<UnitId> {
        self.weapons.get(&unit).copied()
    }
    fn weapon_in_use(&self, unit: UnitId) -> Option<UnitId> {
        self.in_use.get(&unit).copied()
    }
    fn item_is(&self, item: UnitId, itype: i32) -> bool {
        self.item_types.get(&item) == Some(&itype)
    }
    fn damage_weapon(&self, unit: UnitId, weapsel: u8, flag: bool) -> Option<Option<UnitId>> {
        self.damage_weapons.get(&(unit, weapsel, flag)).copied()
    }
    fn entry_flags(&self, unit: UnitId, _: &crate::skills::SkillEntry) -> u32 {
        self.entry_flags.get(&unit).copied().unwrap_or(0)
    }
    fn monster_skill_start(
        h: &mut ActionHooks<Self>,
        _: &mut crate::units::hooks::Sim<'_>,
        unit: UnitId,
    ) -> i32 {
        h.x.log.push(format!("skill start {}", unit.0));
        h.x.skill_start
    }
    fn monster_sequence_frame(
        h: &mut ActionHooks<Self>,
        _: &mut crate::units::hooks::Sim<'_>,
        unit: UnitId,
    ) {
        h.x.log.push(format!("sequence frame {}", unit.0));
    }
    fn monster_attack_skill(
        h: &mut ActionHooks<Self>,
        _: &mut crate::units::hooks::Sim<'_>,
        unit: UnitId,
    ) {
        h.x.log.push(format!("attack skill {}", unit.0));
    }
    fn monster_attack_strike(
        h: &mut ActionHooks<Self>,
        _: &mut crate::units::hooks::Sim<'_>,
        unit: UnitId,
        moving: bool,
    ) {
        h.x.log.push(format!("attack strike {} {moving}", unit.0));
    }
    fn unit_event(
        &mut self,
        event: u8,
        unit: Option<UnitId>,
        _: Option<UnitId>,
        _: Option<&mut crate::combat::DamageRecord>,
    ) {
        self.log
            .push(format!("event {event} {:?}", unit.map(|u| u.0)));
    }
    fn reaction(&mut self, a: UnitId, d: UnitId, r: &mut crate::combat::DamageRecord) {
        self.log
            .push(format!("reaction {} {} {:#x}", a.0, d.0, r.result));
    }
    fn set_object_mode(&mut self, _: &mut Game, object: UnitId, mode: u8) {
        self.log.push(format!("object mode {} {mode}", object.0));
    }
    fn object_approach(
        &mut self,
        _: &mut Game,
        _: UnitId,
        _: UnitId,
    ) -> crate::wiring::action::ObjectReach {
        self.reach
            .unwrap_or(crate::wiring::action::ObjectReach::Operate)
    }
    /// Every operator is in interact range of every object (logged).
    fn object_in_range(&self, _: &Game, operator: UnitId, object: UnitId) -> bool {
        self.ranged.borrow_mut().push((operator, object));
        true
    }
    fn object_route(&mut self, _: &mut Game, route: crate::wiring::action::ObjectRoute) {
        self.log.push(format!("object route {route:?}"));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn death_end_action(&mut self, _: &mut Game, unit: UnitId, action: u8, minion: u16) {
        self.log
            .push(format!("death action {} {action} {minion}", unit.0));
    }
}

// ---- DRLG fixture -------------------------------------------------------------------

/// Rooms per level id; floor grids, with a missile-barrier tile column.
struct Types {
    rooms: BTreeMap<u32, Vec<TileRect>>,
    /// Preset units by the room's tile origin.
    presets: BTreeMap<(i32, i32), Vec<crate::drlg::seams::PresetUnit>>,
}

impl LevelTypes for Types {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = drlg.level(level).id;
        for rect in self.rooms.get(&id).cloned().unwrap_or_default() {
            let r = drlg.alloc_room(level, RoomKind::Preset, rect);
            drlg.room_mut(r).dt1_mask = 1;
            drlg.link_room(r, LinkAt::Tail);
        }
        Ok(())
    }

    fn preset_units(&self, drlg: &Drlg, room: DrlgRoomId) -> Vec<crate::drlg::seams::PresetUnit> {
        let r = drlg.room(room).rect;
        self.presets.get(&(r.x, r.y)).cloned().unwrap_or_default()
    }

    /// One floor pass: every cell a floor, edges linked; room-local tile
    /// column 4 of a room at x = 0 is "fill LOS" (record flag 0x80 →
    /// collision 0x4).
    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        let r = drlg.room(room).rect;
        let (w, h) = (r.w as usize + 1, r.h as usize + 1);
        let mut g = CellGrid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                let mut v = cell::FLOOR | if edge { cell::LINKED } else { 0 };
                if r.x == 0 && x == 4 {
                    v |= cell::FILL_LOS;
                }
                g.set(x, y, v);
            }
        }
        Ok(RoomGrids {
            passes: vec![GridPass {
                cells: g,
                orientation: None,
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        })
    }
}

fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation: o,
        main,
        sub,
        rarity,
        material: 0,
        subtile_flags: [0; 25],
        roof_height: 0,
        height: 0,
        light_direction: 0,
    }
}

/// DT1 files by path: open floors (0, 0, 0), walls/shadows of key (1, 0),
/// the fixed library files.
struct Tiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tiles() -> Tiles {
    let mut t = BTreeMap::new();
    let mut floor: Vec<TileInfo> = (0..4).map(|_| tile(0, 0, 0, 1)).collect();
    for o in [1, 2, 3, 4, 13] {
        floor.push(tile(o, 1, 0, 1));
    }
    t.insert(b"floor.dt1".to_vec(), floor);
    let blank_tile = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(
        FIXED_LIBRARY[0].to_vec(),
        vec![blank_tile(0), blank_tile(1)],
    );
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    Tiles(t)
}

fn drlg_data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
    }
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    d.lvltypes = vec![vec![Vec::new(); 32], files];
    for id in [1, LEVEL] {
        d.levels[id as usize].drlg_type = 2;
        d.levels[id as usize].level_type = 1;
    }
    d
}

// ---- tables -------------------------------------------------------------------------------

/// Missile 0: an arrow-like row (default flight, collide type 3, kill on
/// collision, to-hit).
pub fn arrow() -> MissileRow {
    let mut r: MissileRow = blank();
    r.psrvdofunc = 1;
    r.vel = 1;
    r.maxvel = 1;
    r.range = 50;
    r.collidetype = 3;
    r.collidekill = 1;
    r.lastcollide = true;
    r.tohit = 1;
    r.size = 1;
    r
}

/// Monster class 0: AI 1 (Idle), aidel 15, killable.
pub fn monster_class() -> d2_data::tables::Monstats {
    let mut m = monster_rec();
    m.ai = 1;
    m.aidel = 15;
    m.aidel_n = 15;
    m.aidel_h = 15;
    m.skill1 = 0xFFFF;
    m.skill2 = 0xFFFF;
    m.skill3 = 0xFFFF;
    m
}

fn tables() -> ActionTables {
    ActionTables {
        missiles: vec![arrow()],
        skills: skill_tables(vec![skill_rec()]),
        combat: combat_tables(vec![monster_class()]),
        levels: vec![blank::<Levels>(); 150],
        skill_modes: vec![[0; 8]],
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    }
}

fn unit_data() -> UnitData {
    UnitData {
        monsters: vec![MonsterInfo {
            enabled: true,
            aidel: [15, 15, 15],
            moves: 1 << 4,
            mode_chart: false,
        }],
        ..UnitData::default()
    }
}

// ---- the fixture --------------------------------------------------------------------------

/// A game with act 0's DRLG, level 2 generated, rooms A (tiles 0..8) and B
/// (8..16) streamed (active in the act room list), and the action systems.
pub struct Fx {
    pub game: Game,
    pub sim: ActionSim<TestPending>,
    /// Active rooms of A and B.
    pub a: RoomId,
    pub b: RoomId,
}

impl Fx {
    // Shared with the benches (`bench_fixtures::combat`), where clippy
    // sees it as a public type.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::with_rooms(&[
            (LEVEL, TileRect::new(0, 0, 8, 8)),
            (LEVEL, TileRect::new(8, 0, 8, 8)),
        ])
    }

    /// The rooms of `rooms` (level id, rect) streamed in order.
    /// A level of another act than act 0 gets its act's DRLG (same init
    /// seed, every level of `rooms` a preset level of type 1).
    pub fn with_rooms(rooms: &[(u32, TileRect)]) -> Self {
        Self::with_presets(rooms, BTreeMap::new())
    }

    /// [`Self::with_rooms`], the rooms' preset units by tile origin.
    pub fn with_presets(
        rooms: &[(u32, TileRect)],
        presets: BTreeMap<(i32, i32), Vec<crate::drlg::seams::PresetUnit>>,
    ) -> Self {
        let mut data = drlg_data();
        let mut by_level: BTreeMap<u32, Vec<TileRect>> = BTreeMap::new();
        for &(l, r) in rooms {
            by_level.entry(l).or_default().push(r);
            data.levels[l as usize].drlg_type = 2;
            data.levels[l as usize].level_type = 1;
        }
        let mut types = Types {
            rooms: by_level,
            presets,
        };
        let mut dungeon = crate::drlg::Dungeon::default();
        let mut acts = vec![0u8];
        acts.extend(rooms.iter().map(|r| crate::drlg::act_of_level(r.0)));
        acts.sort_unstable();
        acts.dedup();
        for &act in &acts {
            let drlg = Drlg::create(act, INIT, 0, 0, false, &data, &mut types).expect("drlg");
            dungeon.acts[usize::from(act)] = Some(drlg);
        }
        let world = DrlgWorld {
            dungeon,
            data: Arc::new(data),
            tiles: Box::new(tiles()),
            types: Box::new(types),
        };
        let hooks = ActionHooks::new(
            Arc::new(tables()),
            world,
            Seed::init_low(1234),
            TestPending::default(),
        );
        let mut sim = ActionSim::new(crate::stats::tests::data(), unit_data(), hooks);
        let mut game = Game::new();
        for &act in &acts {
            game.lists.ensure_act(act).unwrap();
        }
        let mut active = Vec::new();
        let levels: Vec<u32> = {
            let mut v: Vec<u32> = rooms.iter().map(|r| r.0).collect();
            v.dedup();
            v
        };
        for id in levels {
            let act = crate::drlg::act_of_level(id);
            let r = sim
                .hooks()
                .drlg
                .with_act(act, &mut game.lists, |d, svc| {
                    let l = d.get_or_alloc_level(svc.data, svc.types, id)?;
                    d.generate_level(svc.data, svc.types, l)?;
                    let mut out = Vec::new();
                    for r in d.level_rooms(l) {
                        out.push(d.stream_room(svc, r)?.expect("active"));
                    }
                    Ok::<_, DrlgError>(out)
                })
                .unwrap()
                .unwrap();
            active.extend(r);
        }
        Self {
            game,
            sim,
            a: active[0],
            b: active.get(1).copied().unwrap_or(active[0]),
        }
    }

    /// A new client record (no player) added to `room`'s client array: the
    /// room's client count becomes nonzero.
    pub fn add_room_client(&mut self, room: RoomId) {
        let c = self.game.lists.add_client(None, Some(room), 0);
        let act = self.game.lists.room(room).expect("room").act;
        let d = self.sim.hooks().drlg.dungeon.acts[usize::from(act)]
            .as_mut()
            .expect("act");
        let id = d.drlg_room_of(room).expect("active room");
        d.add_room_client(id, c);
    }

    /// Allocates a unit in `room` at (x, y).
    pub fn spawn(&mut self, ty: UnitType, class: u32, room: RoomId, x: i32, y: i32) -> UnitId {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        let u = self
            .sim
            .with(&mut self.game, |g, v| {
                if ty != UnitType::Player {
                    return v.allocate(g, &req, x, y);
                }
                // Players start in mode 0 (dead); the player mode starts
                // are not run here: neutral, set before the unit is added
                // so the dead-body path settings do not apply.
                let u = v.allocate_unadded(g, &req)?;
                v.units.get_mut(u).unwrap().mode = 1;
                v.add_allocated(g, u, &req, x, y).then_some(u)
            })
            .expect("allocated");
        u
    }

    /// Sets base stats of a unit.
    pub fn stats(&mut self, u: UnitId, values: &[(u16, i32)]) {
        self.sim.with(&mut self.game, |_, v| {
            for &(s, x) in values {
                v.set_base(u, s, x);
            }
        });
    }

    pub fn stat(&mut self, u: UnitId, s: u16) -> i32 {
        self.sim.with(&mut self.game, |_, v| v.stat(u, s))
    }

    /// Sets a unit's seed.
    pub fn seed(&mut self, u: UnitId, seed: Seed) {
        self.sim.sys.units.get_mut(u).unwrap().seed = seed;
    }

    /// Marks a unit's run-time collision bit at its position (what
    /// movement code will do).
    pub fn mark(&mut self, u: UnitId, bit: u16) {
        let (x, y) = self.sim.hooks().x.position(u);
        let room = self.game.lists.unit(u).unwrap().room().unwrap();
        let game = &self.game;
        *self
            .sim
            .sys
            .hooks
            .drlg
            .collision_mut(game, room, x, y)
            .expect("in a grid") |= bit;
    }

    /// One frame of the timer queue only (frame += 1).
    pub fn frame(&mut self) {
        self.game.frame += 1;
        crate::tick::run_timer_events(&mut self.game, &mut self.sim);
    }

    /// One full tick.
    pub fn tick(&mut self) {
        crate::tick::tick(&mut self.game, &mut self.sim);
    }

    /// The unit's pending timers as (type, expire), sorted.
    pub fn timers(&self, u: UnitId) -> Vec<(u8, i32)> {
        let mut v: Vec<_> = self
            .game
            .timers
            .unit_timers(u)
            .into_iter()
            .filter_map(|t| Some((self.game.timers.event(t)?.0, self.game.timers.expire(t)?)))
            .collect();
        v.sort();
        v
    }

    /// No adapter or unit-dispatch error so far.
    pub fn assert_clean(&self) {
        assert_eq!(self.sim.sys.hooks.errors, Vec::<WiringError>::new());
        assert!(self.sim.sys.errors.is_empty(), "{:?}", self.sim.sys.errors);
    }
}

/// A seed whose next step gives `lo′ = x` (lo 0, hi x).
pub fn seed_giving(x: u32) -> Seed {
    Seed::new(0, x)
}
