//! The wired single-player world's fixture, shared by
//! `e2e_single_player.rs` (through the bridge) and `prop_worldsim.rs`
//! (straight to the dispatcher): the seams no written spec provides
//! ([`TestPending`], [`Book`]: staged answers and call logs, never
//! behaviour), the DS1 / DT1 sources, and the synthetic tables of act
//! 0's DRLG, the ISLE / GATE preset levels, the sorceress' skills, the
//! drop and waypoint tables. The positions and seeds below are the
//! run's; each test builds its own host from them. The combat / missile
//! fixtures (the sorceress' skills and arrow, the monster class, the
//! combat, vitals and AnimData tables, the COF names) are
//! `d2_sim::bench_fixtures::combat`'s, re-exported here.
//!
//! Declared by path (`#[path = "e2e_support/world.rs"] mod e2e_world;`)
//! by the tests that use it, beside `e2e_support`: it needs `d2-sim`'s
//! `bench-fixtures` feature, which `d2-server`'s tests (which include
//! `e2e_support` too) do not enable.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use d2_data::bin::BinTable;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Levels, Objects, Record};
use d2_server::adapters::handlers::skills::LearnRest;
use d2_server::adapters::handlers::world::Outbox;
use d2_sim::drlg::maze::{MazeData, MazeRow, Specials};
use d2_sim::drlg::outdoor::{OutdoorData, PresetDef as OutdoorPreset, SubDefs};
use d2_sim::drlg::preset::{
    Ds1Input, Ds1ObjectInput, Ds1Source, PresetData, PresetDef, PresetTables,
};
use d2_sim::drlg::tiles::{cell, FIXED_LIBRARY};
use d2_sim::drlg::{DrlgData, LevelDef, TileInfo, TileSource};
use d2_sim::game::Game;
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{ty, ItemTables};
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::SkillEntry;
use d2_sim::stats::{StatData, StatTable};
use d2_sim::treasure::{ItemData, TcEntry, TreasureClass, TreasureClasses};
use d2_sim::units::hooks::Sim as USim;
use d2_sim::units::modes;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, KillStep, Pending, SkillEvent};
use d2_sim::wiring::economy::{monster_death_drop, DeathDrops, DropSpot, DropTables, FreeSpot};
use d2_sim::wiring::interaction::{skill_events, UseRest};
use d2_sim::wiring::worldgen::WorldPending;
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};

pub use d2_sim::bench_fixtures::combat::{
    anim_data, arrow, combat_tables, monster_class, skills, vitals, MONSTER_DT, MULTI, PLAYER_SC,
};

use crate::e2e_support::blank;

// ---- constants -----------------------------------------------------------------------

/// The DRLG init seed of a known Act I placement
/// (`dwStartSeed` 4014346869): act 0's levels are allocated by the real
/// level types from it.
pub const DRLG_SEED: u32 = 644_409_375;
/// The game seed.
pub const GAME_SEED: u32 = 1234;
/// A preset level outside the Act I chain (40 × 18 tiles at (8000,
/// 8000)): the player's level, with a waypoint and the DS1 monster.
pub const ISLE: u32 = 30;
pub const ISLE_DEF: u32 = 1104;
/// The preset level the waypoint travels to: also outside the Act I
/// chain (40 × 18 at (9000, 9000)). A chain level (e.g. the Monastery
/// Gate) would generate its outdoor neighbours, which need `lvlsub` rows
/// this fixture does not have.
pub const GATE: u32 = 31;
pub const GATE_DEF: u32 = 1105;
/// Akara's position, beside the player.
pub const NPC_AT: (i32, i32) = (40_022, 40_022);
/// Stat ids (`itemstatcost`).
pub const GOLD: u16 = 14;
/// Waypoint indices (`levels.txt` `Waypoint`).
pub const ISLE_WP: u8 = 2;
pub const GATE_WP: u8 = 1;
/// The ISLE DS1's monster: DS1 sub-tile (12, 10), class 0.
pub const ISLE_MONSTER: (u32, u32) = (12, 10);
/// The player's position: room (8000, 8000)'s sub-tiles start at
/// (40000, 40000) (5 sub-tiles per tile).
pub const PLAYER_AT: (i32, i32) = (40_020, 40_020);
/// The waypoint object's position.
pub const WP_AT: (i32, i32) = (40_024, 40_020);
/// Skills of the synthetic table: attack, and a right skill (srvst 53
/// standing in for Multiple Shot's 4, mana 4 + 1 per level, shift 8;
/// Multiple Shot's vector).
pub const ATTACK: i32 = 0;

// ---- seams without a provider --------------------------------------------------

/// The action and world-generation seams no written spec provides yet
/// (positions and a straight-line path, warp, arrival mode, transport,
/// the DRLG population reads, the COF-name composer and the animation
/// rate, the monster death start's body), the skill use pipeline's rest
/// ([`UseRest`]: the player's skill state in [`Book`], for the message
/// and the timer paths alike), the drop state, and a log of the calls
/// that change something. The answers are the narrowest ones
/// (`Pending`'s defaults) except those the test stages (see each). The
/// player's interaction is the unit record's interact info.
#[derive(Default)]
pub struct TestPending {
    pub pos: BTreeMap<UnitId, (i32, i32)>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
    pub log: Vec<String>,
    /// Missile target points and the sub-tiles each step
    /// crossed.
    pub aim: BTreeMap<UnitId, (i32, i32)>,
    pub crossed: BTreeMap<UnitId, Vec<(i32, i32)>>,
    pub velocity: BTreeMap<UnitId, i32>,
    /// The point the cast aims its missile at (the skill missile
    /// helpers' record fill is not specified).
    pub aim_at: (i32, i32),
    pub book: Book,
    /// The game's drop state, lent out during a drop.
    pub drops: Option<DeathDrops>,
}

impl Pending for TestPending {
    fn anim_name(&self, _: UnitId, ty: UnitType, _: u32, mode: u32) -> Option<[u8; 8]> {
        match (ty, mode) {
            (UnitType::Player, 10) => Some(*PLAYER_SC),
            (UnitType::Monster, 0) => Some(*MONSTER_DT),
            _ => None,
        }
    }
    /// The rate formula is not written: the AnimData speed
    /// as is (no rate stats in this game).
    fn anim_rate(&self, _: UnitId, speed: Option<u32>) -> i16 {
        speed.map_or(0, |s| s as i16)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or_default()
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn size(&self, _: UnitId) -> i32 {
        1
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
    fn set_target_point(&mut self, unit: UnitId, x: i32, y: i32) {
        self.aim.insert(unit, (x, y));
    }
    fn target_distance(&self, _: UnitId) -> i32 {
        10
    }
    /// One sub-tile toward the target point on each axis.
    fn step(&mut self, _: &mut Game, unit: UnitId) -> bool {
        let (x, y) = self.position(unit);
        let (tx, ty) = self.aim.get(&unit).copied().unwrap_or((x + 1, y));
        let p = (x + (tx - x).signum(), y + (ty - y).signum());
        self.pos.insert(unit, p);
        self.crossed.insert(unit, vec![p]);
        true
    }
    fn crossed_subtiles(&self, unit: UnitId) -> Vec<(i32, i32)> {
        self.crossed.get(&unit).cloned().unwrap_or_default()
    }
    fn may_attack(&self, a: UnitId, d: UnitId) -> bool {
        a != d
    }
    fn class_has_mode(&self, _: i32, _: u8) -> bool {
        true
    }
    fn skill_list(&self, _: UnitId) -> Vec<SkillEntry> {
        self.book.get().list.clone()
    }
    fn used_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.book.get().used
    }
    fn unit_event(
        &mut self,
        event: u8,
        unit: Option<UnitId>,
        _: Option<UnitId>,
        _: Option<&mut d2_sim::combat::DamageRecord>,
    ) {
        self.log
            .push(format!("event {event} {:?}", unit.map(|u| u.0)));
    }
    fn reaction(&mut self, a: UnitId, d: UnitId, r: &mut d2_sim::combat::DamageRecord) {
        self.log
            .push(format!("reaction {} {} {:#x}", a.0, d.0, r.result));
    }
    fn kill_step(&mut self, _: &mut Game, step: KillStep, d: UnitId, a: UnitId) {
        self.log.push(format!("kill {step:?} {} {}", d.0, a.0));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn warp(&mut self, _: &mut Game, player: UnitId, level: u32, tile_code: u8) {
        self.log
            .push(format!("warp {} {level} {tile_code}", player.0));
    }
    fn set_player_mode_arrival(&mut self, _: &mut Game, player: UnitId) {
        self.log.push(format!("arrival mode {}", player.0));
    }
    fn level_up_event(&mut self, unit: UnitId) {
        self.log.push(format!("level up {}", unit.0));
    }
    fn skill_event(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, ev: SkillEvent) {
        skill_events::route(h, sim, ev);
    }
    fn action_frame(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        u: UnitId,
        a1: u32,
        a2: u32,
    ) -> u32 {
        h.x.log.push(format!("action frame {} {a1} {a2}", u.0));
        skill_events::action_frame(h, sim, u, a1, a2)
    }
    /// The death start's body is not written: the fixture sets mode DT
    /// (a start function sets its mode, monster spec) and runs the drop
    /// gate and drop it is known to call.
    fn monster_death_start(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        target: Option<UnitId>,
    ) -> bool {
        h.x.log.push(format!(
            "death start {} target {:?}",
            unit.0,
            target.map(|t| t.0)
        ));
        modes::set_mode(sim, h, unit, 0).expect("mode DT");
        if let Some(mut d) = h.x.drops.take() {
            monster_death_drop(h, sim, &mut d, &mut Spot, unit, target);
            h.x.drops = Some(d);
        }
        true
    }
    fn set_entry_param_of(&mut self, _: UnitId, e: &SkillEntry, i: u8, v: i32) {
        self.book.param(e, i, v);
    }
}

/// The free-spot search (not written): the
/// start spot as is.
pub struct Spot;

impl FreeSpot for Spot {
    fn free_spot(
        &mut self,
        room: Option<RoomId>,
        start: (i32, i32),
        _: (i32, i32),
    ) -> Option<DropSpot> {
        Some(DropSpot {
            room,
            x: start.0,
            y: start.1,
        })
    }
}

impl WorldPending for TestPending {
    fn preset_created(&mut self, unit: UnitId, p: &d2_sim::monsters::population::PresetUnit) {
        self.log.push(format!(
            "preset {} class {} at {},{}",
            unit.0, p.class, p.x, p.y
        ));
    }
}

impl Outbox for TestPending {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// The skill use pipeline's rest (the message path through the server's
/// skill handlers, the action frame on the timer path): the player's
/// skill state from [`Book`], the rest narrowest.
impl UseRest for TestPending {
    fn send(&mut self, u: UnitId, msg: ServerMsg) {
        self.log.push(format!("send {} {msg:?}", u.0));
    }
    fn has_player_data(&self, _: UnitId) -> bool {
        true
    }
    fn last_point_frame(&self, _: UnitId) -> i32 {
        0
    }
    fn set_last_point_frame(&mut self, _: UnitId, _: i32) {}
    fn cursor_item(&self, _: UnitId) -> bool {
        false
    }
    fn in_own_inventory(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn within_reach(&self, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn owner(&self, _: UnitId) -> Option<UnitId> {
        None
    }
    fn is_pet(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn is_ally(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn left_skill(&self, _: UnitId) -> Option<SkillEntry> {
        None
    }
    fn right_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.book.get().right
    }
    fn set_left_skill(&mut self, _: UnitId, _: SkillEntry) {}
    fn set_right_skill(&mut self, _: UnitId, e: SkillEntry) {
        self.book.get().right = Some(e);
    }
    fn find_entry(&self, u: UnitId, skill: i32) -> Option<SkillEntry> {
        self.book.find_entry(u, skill)
    }
    fn find_entry_owned(&self, _: UnitId, _: i32, _: i32) -> Option<SkillEntry> {
        None
    }
    fn owns_skill(&self, u: UnitId, skill: i32) -> bool {
        self.book.find_entry(u, skill).is_some()
    }
    fn set_used_skill(&mut self, _: UnitId, e: Option<SkillEntry>) {
        self.book.get().used = e;
    }
    fn used_skill_flags(&self, _: UnitId) -> u32 {
        0
    }
    fn set_used_skill_flags(&mut self, _: UnitId, _: u32) {}
    fn entry_mode(&self, u: UnitId, e: &SkillEntry) -> u32 {
        self.book.entry_mode(u, e)
    }
    fn attack_param4(&self, _: UnitId) -> i32 {
        0
    }
    fn set_attack_param4(&mut self, _: UnitId, _: i32) {}
    fn use_state(&mut self, _: UnitId, _: &SkillEntry) -> UseState {
        UseState::Usable
    }
    fn shapeshifted(&self, _: UnitId) -> bool {
        false
    }
    fn consume_charges(&mut self, _: UnitId, _: &SkillEntry) -> bool {
        true
    }
    fn pay_life(&mut self, _: UnitId, _: i32) -> bool {
        true
    }
    fn can_dual_wield(&self, _: UnitId) -> bool {
        false
    }
    fn equippable(&self, _: UnitId) -> bool {
        false
    }
    fn bow_equipped(&self, _: UnitId) -> bool {
        false
    }
    fn state_mask(&self, _: UnitId, _: u32) -> bool {
        false
    }
    fn start_mode(&mut self, _: &mut Game, _: UnitId, _: u32, _: ModeTarget<UnitId>) {}
    fn run_to(&mut self, _: UnitId, _: UnitId, _: SkillEntry) {}
    fn target(&self, _: UnitId) -> Option<UnitId> {
        None
    }
    fn clear_target(&mut self, _: UnitId) {}
    fn event_arg(&self, _: UnitId) -> i32 {
        0
    }
    fn set_event_arg(&mut self, _: UnitId, _: i32) {}
    fn step_path(&mut self, _: UnitId) -> i32 {
        0
    }
    fn target_position(&self, _: UnitId) -> Option<(i32, i32)> {
        Some(self.aim_at)
    }
    fn line_clear(&self, _: UnitId, _: (i32, i32), _: u32) -> bool {
        true
    }
    fn set_aura_state(&mut self, _: UnitId, _: u16, _: i32, _: i32) {}
    fn srvst(&mut self, index: u16, u: UnitId, skill: i32, lvl: i32) -> i32 {
        self.book.srvst(index, u, skill, lvl)
    }
    fn srvdo(&mut self, i: u16, u: UnitId, s: i32, l: i32, c: bool, it: bool, a: bool) -> i32 {
        self.book.srvdo(i, u, s, l, c, it, a)
    }
}

/// The skill-point calls: not reached in this run.
impl LearnRest for TestPending {
    fn is_class_skill(&self, _: UnitId, _: i32) -> bool {
        false
    }
    fn add_skill_level(&mut self, _: UnitId, _: i32, _: i32) {}
    fn after_skill_point(&mut self, _: UnitId) {}
}

/// The skill pipeline's state without a provider (skill list, skill
/// bodies are not provided): the player's skill list and a call log,
/// shared with the test.
#[derive(Default)]
pub struct Inner {
    pub list: Vec<SkillEntry>,
    pub right: Option<SkillEntry>,
    pub used: Option<SkillEntry>,
    pub log: Vec<String>,
}

#[derive(Clone, Default)]
pub struct Book(pub Arc<Mutex<Inner>>);

impl Book {
    pub fn get(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }
    pub fn find_entry(&self, _: UnitId, skill: i32) -> Option<SkillEntry> {
        self.get().list.iter().copied().find(|e| e.skill == skill)
    }
    pub fn entry_mode(&self, _: UnitId, e: &SkillEntry) -> u32 {
        // Skill entry +8: attack 7 (A1), the right skill
        // 10 (SC).
        if e.skill == MULTI {
            10
        } else {
            7
        }
    }
    /// The used entry's param `i` := v (srvst 53, the fixture's start,
    /// writes param 1 := frame + level): logged.
    pub fn param(&self, e: &SkillEntry, i: u8, v: i32) {
        self.get().log.push(format!("param{i} {} {v}", e.skill));
    }
    pub fn srvst(&self, index: u16, _: UnitId, skill: i32, lvl: i32) -> i32 {
        self.get().log.push(format!("srvst {index} {skill} {lvl}"));
        1
    }
    /// The do bodies are catalogued only: logged, result
    /// 0 (the generic `srvmissile` creation still runs).
    #[allow(clippy::too_many_arguments)]
    pub fn srvdo(&self, i: u16, _: UnitId, s: i32, l: i32, c: bool, it: bool, a: bool) -> i32 {
        self.get()
            .log
            .push(format!("srvdo {i} {s} {l} {c} {it} {a}"));
        0
    }
}

// ---- DS1 / DT1 sources ---------------------------------------------------------------

#[derive(Default)]
pub struct Ds1s(pub BTreeMap<Vec<u8>, Ds1Input>);

impl Ds1Source for Ds1s {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input> {
        self.0.get(path)
    }
}

/// A v18 DS1 of `w × h` tiles: one layer of plain floors, an empty
/// shadow layer, monsters (id = class) at DS1 sub-tile positions.
pub fn ds1(w: u32, h: u32, monsters: &[(u32, u32, u32)]) -> Ds1Input {
    let cells = ((w + 1) * (h + 1)) as usize;
    Ds1Input {
        version: 18,
        width: w,
        height: h,
        act: 0,
        tag_type: 0,
        walls: Vec::new(),
        orientations: Vec::new(),
        floors: vec![vec![cell::FLOOR; cells]],
        shadow: vec![0; cells],
        objects: monsters
            .iter()
            .map(|&(id, x, y)| Ds1ObjectInput {
                kind: 1,
                id,
                x,
                y,
                flags: 0,
            })
            .collect(),
        paths: Vec::new(),
    }
}

pub fn ds1s() -> Ds1s {
    let (x, y) = ISLE_MONSTER;
    Ds1s(BTreeMap::from([
        (
            format!("def{ISLE_DEF}.ds1").into_bytes(),
            ds1(40, 18, &[(0, x, y)]),
        ),
        (format!("def{GATE_DEF}.ds1").into_bytes(), ds1(40, 18, &[])),
    ]))
}

pub fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
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

pub struct Tiles(pub BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

pub fn tiles() -> Tiles {
    let mut t = BTreeMap::new();
    t.insert(b"floor.dt1".to_vec(), vec![tile(0, 0, 0, 1); 4]);
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

// ---- tables -----------------------------------------------------------------------------

/// Act I sizes, offsets and Vis of a known placement,
/// plus [`ISLE`] and the maze level 8; level types 1 and 3 with one DT1.
pub fn drlg_data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
        l.level_type = 1;
    }
    let set = |d: &mut DrlgData, id: usize, ty: u32, size: (i32, i32), off: (i32, i32)| {
        d.levels[id].drlg_type = ty;
        d.levels[id].size = [size; 3];
        d.levels[id].offset = off;
    };
    set(&mut d, 1, 2, (56, 40), (0, 0));
    set(&mut d, 2, 3, (56, 96), (0, 0));
    set(&mut d, 3, 3, (80, 80), (0, 0));
    set(&mut d, 4, 3, (80, 80), (1000, 1000));
    set(&mut d, 5, 3, (80, 80), (0, 0));
    set(&mut d, 6, 3, (80, 80), (0, 0));
    set(&mut d, 7, 3, (80, 80), (0, 0));
    set(&mut d, 8, 1, (200, 200), (1500, 1000));
    set(&mut d, 17, 3, (40, 48), (0, 0));
    set(&mut d, 26, 2, (40, 18), (3000, 1000));
    set(&mut d, 27, 2, (40, 40), (0, 0));
    set(&mut d, 39, 3, (64, 64), (5000, 1148));
    set(&mut d, ISLE as usize, 2, (40, 18), (8000, 8000));
    set(&mut d, GATE as usize, 2, (40, 18), (9000, 9000));
    d.levels[8].level_type = 3;
    d.levels[2].vis = [1, 3, 0, 0, 0, 0, 0, 0];
    d.levels[3].vis = [2, 4, 17, 0, 0, 0, 0, 0];
    d.levels[1].vis = [2, 0, 0, 0, 0, 0, 0, 0];
    d.levels[4].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    d.levels[17].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    d.lvltypes = vec![
        vec![Vec::new(); 32],
        files.clone(),
        vec![Vec::new(); 32],
        files,
    ];
    d
}

/// lvlprest rows 0..1199 (`Files` 1, DT1 mask 1), the preset levels'
/// rows (town Files 0, Monastery Gate, Outer Cloister Files 3, ISLE, the
/// travel level) and
/// the outdoor cell row 1103 (16 × 16, Files 2).
pub fn preset_data() -> PresetData {
    let mut defs: Vec<PresetDef> = (0..1200)
        .map(|i| PresetDef {
            def: i,
            files: 1,
            dt1_mask: 1,
            populate: 1,
            file: Default::default(),
            ..PresetDef::default()
        })
        .collect();
    for d in &mut defs {
        d.file[0] = format!("def{}.ds1", d.def).into_bytes();
    }
    for (i, level, files) in [
        (1100, 1, 0),
        (1101, 26, 1),
        (GATE_DEF, GATE, 1),
        (1102, 27, 3),
        (ISLE_DEF, ISLE, 1),
        (1103, 0, 2),
    ] {
        defs[i as usize].level_id = level;
        defs[i as usize].files = files;
    }
    defs[1103].size_x = 16;
    defs[1103].size_y = 16;
    PresetData {
        defs,
        monpreset_acts: Default::default(),
        monpreset: Vec::new(),
        monstats_count: 1,
        superuniques_count: 0,
        hdm_item: -1,
        tables: PresetTables::spec().expect("preset-tables.tsv"),
    }
}

pub fn outdoor_data(pd: &PresetData) -> OutdoorData {
    OutdoorData {
        levels: vec![
            SubDefs {
                sub_type: -1,
                sub_theme: -1,
                sub_waypoint: -1,
                sub_shrine: -1,
            };
            150
        ],
        presets: pd
            .defs
            .iter()
            .map(|d| OutdoorPreset {
                size_x: d.size_x as i32,
                size_y: d.size_y as i32,
                files: d.files,
            })
            .collect(),
        subs: Vec::new(),
    }
}

pub fn maze_data() -> MazeData {
    MazeData {
        rows: vec![MazeRow {
            level: 8,
            rooms: [1; 3],
            size_x: 24,
            size_y: 24,
            merge: 0,
        }],
        prest_files: (0..1200).map(|d| (d, 1)).collect(),
        specials: Specials::shipped(),
    }
}

/// levels.txt: no monsters, act by level id; waypoints at [`ISLE`] and
/// [`GATE`] only.
pub fn levels() -> Vec<Levels> {
    let mut v: Vec<Levels> = vec![blank(); 150];
    for (i, l) in v.iter_mut().enumerate() {
        for m in [
            &mut l.mon1,
            &mut l.mon2,
            &mut l.mon3,
            &mut l.mon4,
            &mut l.mon5,
            &mut l.mon6,
            &mut l.mon7,
            &mut l.mon8,
            &mut l.mon9,
            &mut l.mon10,
            &mut l.mon11,
            &mut l.mon12,
            &mut l.mon13,
            &mut l.mon14,
            &mut l.mon15,
            &mut l.mon16,
            &mut l.mon17,
            &mut l.mon18,
            &mut l.mon19,
            &mut l.mon20,
            &mut l.mon21,
            &mut l.mon22,
            &mut l.mon23,
            &mut l.mon24,
            &mut l.mon25,
            &mut l.nmon1,
            &mut l.umon1,
        ] {
            *m = 0xFFFF;
        }
        l.act = d2_sim::drlg::act_of_level(i as u32);
        l.waypoint = NO_WAYPOINT;
    }
    v[ISLE as usize].waypoint = ISLE_WP;
    v[GATE as usize].waypoint = GATE_WP;
    v
}

/// A synthetic itemstatcost: 359 stats, no ops, fixed up.
pub fn stat_data() -> Arc<StatData> {
    let (n, size) = (359, Itemstatcost::SIZE);
    let mut records = vec![0u8; n * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: n,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        ..StatData::default()
    })
}

/// The drop tables over `items`, whose record `gold` is gold; treasure
/// class 1: one pick of gold. `e2e_single_player.rs` passes the game's
/// one item table (`game_item_tables`, `GOLD_REC`), `prop_worldsim.rs`
/// the gold-only table ([`gold_item_tables`], 0).
pub fn drop_tables_from(items: ItemTables, gold: usize) -> DropTables {
    let treasure_items = items
        .items
        .iter()
        .map(|r| ItemData {
            code: r.code,
            ubercode: r.ubercode,
            ultracode: r.ultracode,
            version: r.version,
            level: r.level,
            type_: r.type_ as u16,
            type2: r.type2 as u16,
            unique: r.unique,
            quest: r.quest,
            spawnable: 1,
        })
        .collect();
    let tc = |name: &[u8], entries: Vec<TcEntry>, total| TreasureClass {
        name: name.to_vec(),
        group: 0,
        level: 0,
        total_classic: total,
        total_expansion: total,
        picks: 1,
        nodrop: 0,
        mods: [0; 6],
        entries,
    };
    let gold_entry = TcEntry {
        start_classic: 0,
        start_expansion: 0,
        id: gold as _,
        row: 0,
        flags: 0,
        mods: [0; 6],
    };
    DropTables {
        items,
        tcs: TreasureClasses {
            tcs: vec![tc(b"none", Vec::new(), 0), tc(b"gold", vec![gold_entry], 1)],
            group_offset: 0,
            chest: [None; 45],
            notes: Vec::new(),
        },
        treasure_items,
        superuniques: Vec::new(),
    }
}

/// One waypoint object class (0): operate 23, init 17.
pub fn waypoint_data() -> WaypointData {
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 15 << 8;
    WaypointData::new(&levels(), &[o])
}

/// Items: gold only (`ty::GOLD`, a child of `ty::MISC`), with the item
/// types and equivalences the treasure walk reads: `prop_worldsim.rs`'s
/// drop table (no vendor or cube items on its host).
pub fn gold_item_tables() -> ItemTables {
    let n: usize = 40;
    let words = n.div_ceil(32);
    let mut equiv = EquivMatrix {
        n,
        words,
        bits: vec![0; n * words],
    };
    for i in 1..n {
        equiv.bits[i * words] |= 1;
        equiv.bits[i * words + i / 32] |= 1 << (i % 32);
    }
    let (g, m) = (usize::from(ty::GOLD), usize::from(ty::MISC));
    equiv.bits[g * words + m / 32] |= 1 << (m % 32);
    let mut itemtypes: Vec<Itemtypes> = (0..n)
        .map(|_| {
            let mut t: Itemtypes = blank();
            (t.class, t.staffmods, t.rare) = (0xFF, 0xFF, 1);
            // Empty `shoots`: the link miss (link16 −1).
            t.shoots = 0xFFFF;
            t
        })
        .collect();
    // Gold is always normal quality (itemtypes `Normal`).
    itemtypes[g].normal = 1;
    let mut ratio: Itemratio = blank();
    ratio.version = 1;
    let gold = ItemRec {
        code: *b"gld ",
        type_: ty::GOLD as i16,
        level: 1,
        ..ItemRec::default()
    };
    ItemTables {
        items: vec![gold],
        itemtypes,
        equiv,
        itemratio: vec![ratio],
        valshift: vec![0; 359],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}
