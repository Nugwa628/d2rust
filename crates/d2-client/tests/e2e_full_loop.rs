//! The full single-player loop, end to end, on the wired host with the
//! path provider on: the bridge (`d2_client::bridge`) on its local link
//! over the in-process `d2-server` host, whose game is `SimGame` on
//! `WorldSim` (worldgen around `ActionSim`, `ActionHooks::paths` on
//! before any allocation) with the `WiredWorld` host (waypoints, skills,
//! NPC / vendor / quest systems, the game's one item store and one
//! inventory model), from the synthetic tables and DS1 files of
//! `e2e_single_player.rs` (no game files).
//!
//! The loop (each step names its seams):
//!
//! 1. join: the first tick activates the rooms near the player and the
//!    room pass creates the DS1's monster on its path;
//! 2. walk (C→S 0x01) toward the monster: per-tick path steps, no S→C;
//! 3. attack: a right-skill cast (0x0C) creates the missile; its missile
//!    path flies into the monster's footprint: hit,
//!    kill, death mode, experience and level-up, the death drop (gold),
//!    picked up (0x16 cursor 0) into the player's gold. No S→C: the
//!    per-unit update that announces units is not wired (IS2);
//! 4. a cap the economy wiring makes on the ground (gold has no cursor
//!    form): run (0x03) to it, pick it up to the cursor (0x16) and place
//!    it (0x18): 0x9C / 0x47 / 0x48 bytes exact;
//! 5. run to Akara (0x04, unit form), talk (0x13: 0x27, 0x29, 0x28),
//!    trade (0x38), sell the cap (0x33: 0x2A kind 3), buy the store's
//!    cap (0x32: the copy into the backpack, 0x2A kind 4
//!    and its 0x9C action 4);
//! 6. walk to the waypoint (0x02, unit form) and travel to another level
//!    of the act (0x49): placed by the path code in the destination's
//!    spawn room, S→C 0x07 then 0x0D exact.
//!
//! Every frame's S→C bytes are asserted; frames that send nothing are
//! asserted empty. The run is repeated: same seed → the same transcript.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use d2_client::bridge::dispatch::Dispatch;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::local::{LocalLink, PendingSession};
use d2_client::bridge::world::MonsterClass;
use d2_client::bridge::{Bridge, FrameReport, UnitKey};
use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{
    Charstats, Difficultylevels, Experience, Itemstatcost, Levels, Missiles as MissileRow, Monlvl,
    Monstats, Monstats2, Objects, Record, Skilldesc, Skills,
};
use d2_formats::animdata::{self, AnimData, AnimRecord};
use d2_proto::client::{
    BuyItem, EntityAction, InsertItemInBuffer, InteractWithEntity, PickItem, RightSkill, Run,
    RunToUnit, SellItem, TakeOrCloseWp, Walk, WalkToUnit,
};
use d2_proto::server::{MapReveal, PlayerStop, ReassignPlayer};
use d2_server::adapters::handlers::skills::wired::WiredSkills;
use d2_server::adapters::handlers::skills::LearnRest;
use d2_server::adapters::handlers::walk::enable_paths;
use d2_server::adapters::handlers::world::{ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::dispatch::Outcome;
use d2_server::host::{Handled, Host};
use d2_server::seams::{Clock, PlayerGate, Pos, ResultCode};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::drlg::maze::{Maze, MazeData, MazeRow, Specials};
use d2_sim::drlg::outdoor::{OutdoorData, PresetDef as OutdoorPreset, SubDefs, SubFileMap};
use d2_sim::drlg::preset::{
    Ds1Input, Ds1ObjectInput, Ds1Source, PresetData, PresetDef, PresetTables,
};
use d2_sim::drlg::tiles::{cell, FIXED_LIBRARY};
use d2_sim::drlg::{Drlg, DrlgData, DrlgRoomId, Dungeon, LevelDef, TileInfo, TileSource};
use d2_sim::game::Game;
use d2_sim::items::inventory::InvItem;
use d2_sim::items::moves::Owner;
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{flag, ty, ItemRequest, ItemTables};
use d2_sim::missiles::unit_flag;
use d2_sim::monsters::init::{GameInfo, MonstatsExtra};
use d2_sim::monsters::population::PopTables;
use d2_sim::path::walk::geom::unit_distance;
use d2_sim::path::{CollisionRooms, PathTables, Point as PathPoint};
use d2_sim::rng::Seed;
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::{SkillEntry, SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::{StatData, StatTable};
use d2_sim::treasure::{ItemData, TcEntry, TreasureClass, TreasureClasses};
use d2_sim::units::hooks::{MonsterInfo, Sim as USim, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionTables, DrlgWorld, KillStep, Pending, SkillEvent};
use d2_sim::wiring::economy::{DeathDrops, DropSpot, DropTables, GameFields, ItemSpawn, ItemStore};
use d2_sim::wiring::interaction::{skill_events, UseRest};
use d2_sim::wiring::worldgen::{
    SharedTypes, WorldPending, WorldSim, WorldState, WorldTables, WorldTypes,
};
use d2_sim::world::npc::{class, NpcControl};
use d2_sim::world::quests::{PlayerQuests, QuestControl, QuestTables};
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};

use crate::e2e_support;
use e2e_support::{blank, item_tables, monstats as npc_monstats, tx, vendor_tables, Rest};
use e2e_support::{inv_parts, inv_tables, store, InvFx, BUC, CAP, N_MONSTATS};

// ---- constants -----------------------------------------------------------------------

/// The DRLG init seed of a known Act I placement
/// (`dwStartSeed` 4014346869): act 0's levels are allocated by the real
/// level types from it.
const DRLG_SEED: u32 = 644_409_375;
/// The game seed.
const GAME_SEED: u32 = 1234;
/// A preset level outside the Act I chain (40 × 18 tiles at (8000,
/// 8000)): the player's level, with a waypoint and the DS1 monster.
const ISLE: u32 = 30;
const ISLE_DEF: u32 = 1104;
/// The preset level the waypoint travels to: also outside the Act I
/// chain (40 × 18 at (9000, 9000)). A chain level (e.g. the Monastery
/// Gate) would generate its outdoor neighbours, which need `lvlsub` rows
/// this fixture does not have.
const GATE: u32 = 31;
const GATE_DEF: u32 = 1105;
/// Akara's position, beside the player.
const NPC_AT: (i32, i32) = (40_022, 40_022);
/// The player's gold before the trade.
const PLAYER_GOLD: i32 = 5000;
/// Stat ids (`itemstatcost`).
const DEXTERITY: u16 = 2;
const LEVEL: u16 = 12;
const GOLD: u16 = 14;
const ARMORCLASS: u16 = 31;
/// Waypoint indices (`levels.txt` `Waypoint`).
const ISLE_WP: u8 = 2;
const GATE_WP: u8 = 1;
/// The ISLE DS1's monster: DS1 sub-tile (12, 10), class 0.
const ISLE_MONSTER: (u32, u32) = (12, 10);
/// The player's position: room (8000, 8000)'s sub-tiles start at
/// (40000, 40000) (5 sub-tiles per tile).
const PLAYER_AT: (i32, i32) = (40_020, 40_020);
/// The ground cap's position (its item data).
const CAP_AT: (i32, i32) = (40_030, 40_026);
/// The waypoint object's position.
const WP_AT: (i32, i32) = (40_024, 40_020);
/// Skills of the synthetic table: attack, and a right skill (srvst 4,
/// mana 4 + 1 per level, shift 8; Multiple Shot's vector).
const ATTACK: i32 = 0;
const MULTI: i32 = 1;
/// Stat ids of the walk and the player's
/// stamina (synthetic, as `e2e_walk.rs`).
const STAT_STAMINA: u16 = 10;
const STAT_VELOCITY: u16 = 67;
const STAMINA: i32 = 0x6400;

// ---- seams without a provider --------------------------------------------------

/// The action and world-generation seams no written spec provides yet
/// (the path provider answers every position, path, placement and warp
/// seam, so none of those is here; left: the missile target distance,
/// transport, the DRLG population reads, the COF-name composer and the animation
/// rate, the monster death start's body), the skill use pipeline's rest
/// ([`UseRest`]: the player's skill state in [`Book`], for the message
/// and the timer paths alike), the drop state, and a log of the calls
/// that change something. The answers are the narrowest ones
/// (`Pending`'s defaults) except those the test stages (see each). The
/// player's interaction is the unit record's interact info.
#[derive(Default)]
struct TestPending {
    sent: Vec<(UnitId, Vec<u8>)>,
    log: Vec<String>,
    /// The point the cast aims its missile at (the skill missile
    /// helpers' record fill is not specified).
    aim_at: (i32, i32),
    book: Book,
}

/// The fixture's COF names (the composer for units with a
/// unit is an open question): the sorceress casting (SC)
/// and the monster dying (DT). Other modes get no name.
const PLAYER_SC: &[u8; 8] = b"SOSCHTH\0";
const MONSTER_DT: &[u8; 8] = b"M0DTHTH\0";

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
    /// The unit distance: not specified (stays `Pending` with the path
    /// provider on).
    fn target_distance(&self, _: UnitId) -> i32 {
        10
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
    /// The book is the player's: other units (the monster's mode
    /// message reads it) have no skill
    /// in use.
    fn used_skill(&self, u: UnitId) -> Option<SkillEntry> {
        let b = self.book.get();
        b.used.filter(|_| b.user == Some(u))
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
        d2_client::app::monster_drop::death_start(h, sim, unit, target)
    }
    fn set_entry_param_of(&mut self, _: UnitId, e: &SkillEntry, i: u8, v: i32) {
        self.book.param(e, i, v);
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
    fn set_used_skill(&mut self, u: UnitId, e: Option<SkillEntry>) {
        let mut b = self.book.get();
        b.used = e;
        b.user = Some(u);
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
struct Inner {
    list: Vec<SkillEntry>,
    right: Option<SkillEntry>,
    used: Option<SkillEntry>,
    /// The unit whose skill in use `used` is (the player).
    user: Option<UnitId>,
    log: Vec<String>,
}

#[derive(Clone, Default)]
struct Book(Arc<Mutex<Inner>>);

impl Book {
    fn get(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }
    fn find_entry(&self, _: UnitId, skill: i32) -> Option<SkillEntry> {
        self.get().list.iter().copied().find(|e| e.skill == skill)
    }
    fn entry_mode(&self, _: UnitId, e: &SkillEntry) -> u32 {
        // Skill entry +8: attack 7 (A1), the right skill
        // 10 (SC).
        if e.skill == MULTI {
            10
        } else {
            7
        }
    }
    /// The used entry's param `i` := v (srvst 53 writes param 1 := frame +
    /// level): logged.
    fn param(&self, e: &SkillEntry, i: u8, v: i32) {
        self.get().log.push(format!("param{i} {} {v}", e.skill));
    }
    fn srvst(&self, index: u16, _: UnitId, skill: i32, lvl: i32) -> i32 {
        self.get().log.push(format!("srvst {index} {skill} {lvl}"));
        1
    }
    /// The do bodies are catalogued only: logged, result
    /// 0 (the generic `srvmissile` creation still runs).
    #[allow(clippy::too_many_arguments)]
    fn srvdo(&self, i: u16, _: UnitId, s: i32, l: i32, c: bool, it: bool, a: bool) -> i32 {
        self.get()
            .log
            .push(format!("srvdo {i} {s} {l} {c} {it} {a}"));
        0
    }
}

// ---- DS1 / DT1 sources ---------------------------------------------------------------

#[derive(Default)]
struct Ds1s(BTreeMap<Vec<u8>, Ds1Input>);

impl Ds1Source for Ds1s {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input> {
        self.0.get(path)
    }
}

/// A v18 DS1 of `w × h` tiles: one layer of plain floors, an empty
/// shadow layer, monsters (id = class) at DS1 sub-tile positions.
fn ds1(w: u32, h: u32, monsters: &[(u32, u32, u32)]) -> Ds1Input {
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

fn ds1s() -> Ds1s {
    let (x, y) = ISLE_MONSTER;
    Ds1s(BTreeMap::from([
        (
            format!("def{ISLE_DEF}.ds1").into_bytes(),
            ds1(40, 18, &[(0, x, y)]),
        ),
        (format!("def{GATE_DEF}.ds1").into_bytes(), ds1(40, 18, &[])),
    ]))
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

struct Tiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tiles() -> Tiles {
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
fn drlg_data() -> DrlgData {
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
fn preset_data() -> PresetData {
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

fn outdoor_data(pd: &PresetData) -> OutdoorData {
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

fn maze_data() -> MazeData {
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

/// Monster class 0: killable, AI 1 (Idle), no skills, no minions; level
/// 1, 5 life, 100 experience (`noRatio`: the monstats values as they
/// are); treasure class 1 (Normal).
fn monster_class() -> Monstats {
    let mut m: Monstats = blank();
    m.killable = true;
    m.noratio = true;
    m.level = 1;
    (m.minhp, m.maxhp, m.exp) = (5, 5, 100);
    m.treasureclass1 = 1;
    m.velocity = 1;
    (m.drain, m.drain_n, m.drain_h) = (100, 100, 100);
    m.montype = 0xFFFF;
    m.ai = 1;
    (m.aidel, m.aidel_n, m.aidel_h) = (15, 15, 15);
    (m.skill1, m.skill2, m.skill3) = (0xFFFF, 0xFFFF, 0xFFFF);
    m.rarity = 1;
    (m.mingrp, m.maxgrp) = (1, 1);
    (m.minion1, m.minion2) = (0xFFFF, 0xFFFF);
    m.enabled = true;
    m.isspawn = true;
    m.monstatsex = 1;
    m
}

/// monstats2: row 0 blank (the NPC rows), row 1 the DS1 monster's shape:
/// `SizeX` 1, so its path has size 1, pattern 1 and a footprint the
/// missile can collide with (size 0 → pattern 0
/// stamps nothing).
fn monstats2() -> Vec<Monstats2> {
    let mut m: Monstats2 = blank();
    m.sizex = 1;
    vec![blank(), m]
}

/// levels.txt: no monsters, act by level id; waypoints at [`ISLE`] and
/// [`GATE`] only.
fn levels() -> Vec<Levels> {
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
fn stat_data() -> Arc<StatData> {
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

fn skill_rec() -> Skills {
    let mut s: Skills = blank();
    for f in [
        &mut s.auralencalc,
        &mut s.aurarangecalc,
        &mut s.aurastatcalc1,
        &mut s.calc1,
        &mut s.calc2,
        &mut s.calc3,
        &mut s.calc4,
        &mut s.passivecalc1,
        &mut s.passivecalc2,
        &mut s.passivecalc3,
        &mut s.passivecalc4,
        &mut s.passivecalc5,
        &mut s.petmax,
        &mut s.skpoints,
        &mut s.tohitcalc,
        &mut s.dmgsympercalc,
        &mut s.edmgsympercalc,
        &mut s.elensympercalc,
        &mut s.delay,
        &mut s.perdelay,
    ] {
        *f = 0xFFFF_FFFF;
    }
    s.skilldesc = 0xFFFF;
    s.charclass = 0xFF;
    (s.reqskill1, s.reqskill2, s.reqskill3) = (0xFFFF, 0xFFFF, 0xFFFF);
    s.itypea1 = 0xFFFF;
    s.srvmissile = 0xFFFF;
    s.intown = true;
    s.ingame = true;
    s
}

/// Missile 0: an arrow-like row (default flight, `Vel` 16: velocity
/// 3072 after the 75 % of the missile velocity rule, about half a
/// sub-tile per frame and axis on a diagonal; collide type 3,
/// kill on collision, to-hit), as the action wiring's tests use.
fn arrow() -> MissileRow {
    let mut r: MissileRow = blank();
    r.psrvdofunc = 1;
    r.vel = 16;
    r.maxvel = 16;
    r.range = 50;
    r.collidetype = 3;
    r.collidekill = 1;
    r.lastcollide = true;
    r.tohit = 1;
    r.size = 1;
    r
}

/// The right skill: start function 6 (a `mapped` slot standing in for
/// Multiple Shot's 4, whose body now runs on
/// the wired host), do function 8 (the Multiple Shot
/// slot, body catalogued only), `srvmissile` 0 (the generic missile of
/// the skill use).
fn skills() -> SkillTables {
    let mut v = vec![skill_rec(), skill_rec()];
    let m = &mut v[MULTI as usize];
    // Start 53 / do 53 as `d2_sim::bench_fixtures::combat::skills` (its
    // doc): `calc2` = formula 0 = `lvl`.
    (m.srvstfunc, m.mana, m.lvlmana, m.manashift) = (53, 4, 1, 8);
    (m.srvdofunc, m.srvmissile, m.calc2) = (53, 0, 0);
    SkillTables {
        skills: v,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![arrow()],
        skills_code: vec![0x04, 0x10, 0x00],
        miss_code: Vec::new(),
        level_cap: LEVEL_CAP_114D,
        stat_count: 359,
    }
}

fn combat_tables() -> CombatTables {
    let mut d: Difficultylevels = blank();
    (d.monsterfreezedivisor, d.monstercolddivisor) = (1, 1);
    (d.lifestealdivisor, d.manastealdivisor) = (1, 1);
    // The sorceress' walk / run columns: synthetic,
    // as `e2e_walk.rs`'s.
    let mut charstats = vec![blank::<Charstats>(); 7];
    charstats[1].walkvelocity = 6;
    charstats[1].runvelocity = 9;
    charstats[1].rundrain = 20;
    // Every monstats row the units read (the path shape of Akara,
    // the path shape): the NPC rows of `e2e_support`, row 0 the
    // DS1 monster.
    let mut monstats = npc_monstats();
    monstats[0] = monster_class();
    CombatTables {
        charstats,
        difficultylevels: vec![d; 3],
        monstats,
        monstats2: monstats2(),
        hitclass: vec![*b"none", *b"hth "],
    }
}

/// experience.txt: max level 3, thresholds 0, 100, 1500 for every class
/// (the kill's 100 experience reaches level 2);
/// charstats: the sorceress (class 1) gets 5 stat points per level (the
/// other per-level columns 0).
fn vitals() -> VitalsTables {
    let row = |v: u32| Experience {
        amazon: v,
        sorceress: v,
        necromancer: v,
        paladin: v,
        barbarian: v,
        druid: v,
        assassin: v,
        ..blank()
    };
    let mut charstats = vec![blank::<Charstats>(); 7];
    charstats[1].statperlevel = 5;
    VitalsTables {
        charstats,
        experience: vec![row(3), row(0), row(100), row(1500)],
    }
}

/// AnimData with the fixture's two names: the
/// sorceress' cast, 8 frames at speed 256 with a missile event (2) on
/// frame 4; the monster's death, 4 frames at speed 256, no events.
fn anim_data() -> AnimData {
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    let mut put = |name: &[u8; 8], frames, event: Option<usize>| {
        let mut events = [0u8; animdata::EVENTS];
        if let Some(i) = event {
            events[i] = 2;
        }
        let len = name.iter().position(|&b| b == 0).unwrap();
        a.buckets[animdata::hash(&name[..len])].push(AnimRecord {
            name: *name,
            frames,
            speed: 256,
            events,
        });
    };
    put(PLAYER_SC, 8, Some(4));
    put(MONSTER_DT, 4, None);
    a
}

/// Items: gold only (`ty::GOLD`, a child of `ty::MISC`); treasure class
/// 1: one pick of gold.
fn drop_tables() -> DropTables {
    // The game's one item table (gold among the vendor and cube items):
    // the drop creates into the game's one item store, which every item
    // system reads with these records.
    let items = game_item_tables();
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
        id: GOLD_REC as _,
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
fn waypoint_data() -> WaypointData {
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 15 << 8;
    WaypointData::new(&levels(), &[o])
}

// ---- the cube's item world --------------------------------------------------------------

/// Item types of the cube's items (the cube recipe's shape).
const T_RING: u16 = 10;
const T_BOX: u16 = 11;
const T_AMULET: u16 = 12;
/// The cube's item records (`box `, ring, amulet: 2–4) follow the vendor
/// items (`CAP`, `BUC`) in the game's one item table; this run uses none.
/// The gold pile's record (`gld `), the drop's item.
const GOLD_REC: usize = 5;

/// The game's item tables: the vendor items (`e2e_support::item_tables`:
/// cap, buckler) and the cube's items (`box `, a ring, an amulet; each
/// type is its own and type 0's).
fn game_item_tables() -> ItemTables {
    let mut t = item_tables();
    let rec = |ty: u16, code: &[u8; 4]| ItemRec {
        code: *code,
        type_: ty as i16,
        level: 1,
        ..ItemRec::default()
    };
    t.items.extend([
        rec(T_BOX, b"box "),
        rec(T_RING, b"rin "),
        rec(T_AMULET, b"amu "),
        rec(ty::GOLD, b"gld "),
    ]);
    // Gold is always normal quality (itemtypes `Normal`, the treasure
    // walk's first step) and misc.
    t.itemtypes[usize::from(ty::GOLD)].normal = 1;
    let (g, m, w) = (usize::from(ty::GOLD), usize::from(ty::MISC), t.equiv.words);
    t.equiv.bits[g * w + m / 32] |= 1 << (m % 32);
    t
}

// ---- the game --------------------------------------------------------------------------

type Sim = SimGame<WorldSim<TestPending>, WiredWorld<Rest, WiredSkills>>;

/// Manual host clock (ms), injected into the host.
struct Ms(u32);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type Link = LocalLink<Sim, ProtoSizes, PendingSession, Ms>;

/// The local link, recording every S→C chunk the bridge receives.
/// `inject` holds chunks delivered after the server's at the next
/// receive, not recorded (Akara's add, [`akara_add`]).
struct Tap {
    inner: Link,
    chunks: Vec<Vec<u8>>,
    inject: Vec<Vec<u8>>,
}

impl ServerLink for Tap {
    fn protocol_version(&self) -> u32 {
        self.inner.protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.inner.send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.inner.pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        let mut got = self.inner.receive();
        self.chunks.extend(got.iter().cloned());
        got.append(&mut self.inject);
        got
    }
}

/// Akara's monster add S→C 0xAC as the client reads it:
/// mode 1, no components, no type flags, no
/// source unit, no stat list. d2-sim does not build monster adds yet (the
/// server-side fields of a monster add are not specified, see the
/// `wiring::action::switch` module docs), so the harness delivers the
/// add of an NPC without any of those parts; the client needs her unit
/// for 0x28.
fn akara_add(guid: u32, class: u16, (x, y): (i32, i32)) -> Vec<u8> {
    let mut m = vec![0xAC];
    m.extend(guid.to_le_bytes());
    m.extend(class.to_le_bytes());
    m.extend((x as u16).to_le_bytes());
    m.extend((y as u16).to_le_bytes());
    m.push(128);
    m.push(14);
    // Bits, low first: mode 1 (4 bits), then five 0 presence bits.
    m.push(0x01);
    m
}

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

/// Steps 1–3 before the first host frame: game creation (the path
/// provider on before any allocation), the generated level, the player's
/// room streamed, the player, the waypoint object and Akara allocated
/// there (placed on their paths), the client joined.
struct Fx {
    bridge: Bridge<Tap>,
    player: UnitId,
    wp_unit: UnitId,
    wp: u32,
    book: Book,
    /// The ISLE level's DRLG rooms.
    level_rooms: Vec<DrlgRoomId>,
    /// Akara; the player's buckler (re-sellable) and cap (one of
    /// Akara's permanent codes).
    npc: UnitId,
    buckler: UnitId,
    cap: UnitId,
    /// The item-move seams' staged answers and log.
    inv: InvFx,
    /// An S→C message due in the tick of a given frame (the monster's
    /// death end), which [`walk`]'s
    /// frames expect then and only then; taken when it arrived.
    due: Option<(i32, Vec<u8>)>,
}

impl Fx {
    fn new(game_seed: u32) -> Self {
        // 1. Game creation: act 0's DRLG through the level-type
        // dispatcher on the recorded init seed; regions on the game seed.
        let data = Arc::new(drlg_data());
        let pd = preset_data();
        let types = SharedTypes::new(WorldTypes::new(
            data.clone(),
            Maze::new(maze_data()),
            pd.clone(),
            outdoor_data(&pd),
            Box::new(ds1s()),
            Box::new(SubFileMap::default()),
        ));
        let mut handle = types.clone();
        let drlg = Drlg::create(0, DRLG_SEED, 0, 0, false, &data, &mut handle).expect("act 0");
        let mut dungeon = Dungeon::default();
        dungeon.acts[0] = Some(drlg);
        let world = DrlgWorld {
            dungeon,
            data,
            tiles: Box::new(tiles()),
            types: Box::new(handle),
        };
        let tables = ActionTables {
            missiles: vec![arrow()],
            skills: skills(),
            combat: combat_tables(),
            levels: levels(),
            skill_modes: vec![[0; 8]],
            overlay_count: 0,
            monequip: Vec::new(),
            arena: Vec::new(),
        };
        let book = Book::default();
        let mut hooks = ActionHooks::new(
            Arc::new(tables),
            world,
            Seed::init_low(game_seed),
            TestPending {
                book: book.clone(),
                ..TestPending::default()
            },
        );
        hooks.object_drops = Some(Box::new(DeathDrops::new(
            Arc::new(drop_tables()),
            GameFields::new(Seed::init_low(game_seed), false),
        )));
        hooks.anim_data = Some(Arc::new(anim_data()));
        hooks.vitals = Some(Arc::new(vitals()));
        let wt = WorldTables {
            pop: PopTables::from_records(&levels(), &[monster_class()], &monstats2(), &[]),
            monstats: vec![monster_class()],
            monstats2: monstats2(),
            monlvl: vec![blank::<Monlvl>(); 10],
            levels: levels(),
            difficultylevels: vec![blank::<Difficultylevels>(); 3],
            monstats_extra: vec![MonstatsExtra::default()],
            components: vec![[0; 16]; 2],
            ..WorldTables::default()
        };
        let state = WorldState::new(types, Arc::new(wt), GameInfo::default());
        let unit_data = UnitData {
            // Class 0 (the DS1 monster) moves; the others (Akara) are
            // enabled and stand.
            monsters: (0..N_MONSTATS)
                .map(|c| MonsterInfo {
                    mode_chart: false,
                    enabled: true,
                    aidel: [15; 3],
                    moves: if c == 0 { 1 << 4 } else { 0 },
                })
                .collect(),
            ..UnitData::default()
        };
        let mut sim = WorldSim::new(stat_data(), unit_data, hooks, state);
        // The path provider (`d2_sim::wiring::path`), before any unit is
        // allocated: every position, step,
        // placement and warp below is the path code's, not a fixture's.
        enable_paths(&mut sim).expect("embedded path tables");
        sim.create_regions();
        // The world systems' game-creation seeds (fixture order).
        let mut seed = sim.action.hooks().game_seed;
        let ctl = NpcControl::new(&npc_monstats(), Vec::new(), false, 0, &mut seed).expect("npc");
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        sim.action.hooks().game_seed = seed;
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();

        // 2. The level, by the real preset generator; the player's room
        // streamed.
        let (level_rooms, room) = sim
            .action
            .hooks()
            .drlg
            .with_act(0, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, ISLE)?;
                d.generate_level(svc.data, svc.types, l)?;
                let rooms = d.level_rooms(l);
                let a = *rooms
                    .iter()
                    .find(|&&r| d.room(r).rect == d2_sim::drlg::TileRect::new(8000, 8000, 8, 8))
                    .expect("room at the level origin");
                let active = d.stream_room(svc, a)?.expect("active");
                Ok::<_, d2_sim::drlg::DrlgError>((rooms, active))
            })
            .expect("act 0")
            .expect("generated");

        // 3. The player (a sorceress, class 1), the waypoint object and
        // Akara, allocated on their paths.
        let mut alloc = |ty, class, (x, y): (i32, i32)| {
            let req = AllocRequest {
                ty,
                class,
                room: Some(room),
                add: true,
                fixed_guid: None,
                mode: 1,
                allied: ty == UnitType::Player,
            };
            sim.action
                .with(&mut game, |g, v| v.allocate(g, &req, x, y))
                .expect("allocated")
        };
        let object = alloc(UnitType::Object, 0, WP_AT);
        let player = alloc(UnitType::Player, 1, PLAYER_AT);
        // Akara (allocated as the DS1 monster is; her kind init is the
        // monster spec's, not run here).
        let npc = alloc(UnitType::Monster, u32::from(class::AKARA), NPC_AT);
        // Players are allocated in mode 0; neutral.
        sim.action.sys.units.get_mut(player).unwrap().mode = 1;
        // Both waypoints known.
        let rec = sim.action.hooks().waypoints.entry(player).or_default();
        rec.get_mut(0).set(ISLE_WP.into()).unwrap();
        rec.get_mut(0).set(GATE_WP.into()).unwrap();
        // Mana and max mana 4000 (1/256 units), gold, dexterity 25; the
        // velocity percentage 100, stamina and max stamina (the
        // level-up refills stamina to max;
        // the run's stamina drain;
        // the player's stat init is not written: synthetic, as
        // `e2e_walk.rs`).
        sim.action.with(&mut game, |_, v| {
            v.set_base(player, 8, 4000);
            v.set_base(player, 9, 4000);
            v.set_base(player, GOLD, PLAYER_GOLD);
            v.set_base(player, DEXTERITY, 25);
            v.set_base(player, STAT_VELOCITY, 100);
            v.set_base(player, STAT_STAMINA, STAMINA);
            v.set_base(player, STAT_STAMINA + 1, STAMINA);
        });
        let wp = game.lists.unit(object).unwrap().guid;
        let npc_guid = game.lists.unit(npc).unwrap().guid;

        // The world host: waypoints and skills (`ActionWorld`), the NPC /
        // vendor / quest systems on the same units (`WiredWorld`).
        let mut rest = Rest::default();
        rest.quests.insert(player, PlayerQuests::default());
        let action = ActionWorld {
            waypoints: Some(waypoint_data()),
            skills: WiredSkills::default(),
            ..ActionWorld::default()
        };
        let mut world = WiredWorld::new(
            action,
            game_item_tables(),
            quests,
            ctl,
            vendor_tables(),
            rest,
            1000,
        );
        // Monster init embeds the NPC's interaction list.
        world.state.add_npc(npc);
        // The game's one inventory model (`InvParts`) with the item-move
        // seams' staged answers (`InvFx`): the vendors read and write the
        // same inventory.
        let inv = InvFx::default();
        let pg = game.lists.unit(player).unwrap().guid;
        inv.with(|r| {
            r.distance = 3;
            r.room = Some(room);
            r.pos.insert(Owner::player(pg), PLAYER_AT);
        });
        // Synthetic sizes: cap, buckler, cube 2 × 2; ring, amulet, gold
        // 1 × 1.
        let sizes = [(2, 2), (2, 2), (2, 2), (1, 1), (1, 1), (1, 1)];
        let tables = inv_tables(&world.tables, &sizes);
        world.inventory = Some(inv_parts(tables, inv.clone(), player, 1, pg));
        // The player's buckler and cap, made by the economy wiring on the
        // game seed into the game's one item store and stored (mode 0,
        // page 0) in the player's inventory (as a loaded
        // character's).
        let (buckler, cap) = world.with_economy(&mut game, &mut sim, |econ, _| {
            let mut make = |record: usize| {
                let mut rq = ItemRequest {
                    item: record as i32,
                    ilvl: 1,
                    quality: 2,
                    format: 1,
                    ..ItemRequest::default()
                };
                let held = ItemSpawn {
                    room: None,
                    mode: 4,
                    init_flags: 1,
                };
                econ.create_item(&mut rq, false, held).expect("item")
            };
            (make(BUC), make(CAP))
        });
        for item in [buckler, cap] {
            store(&mut world, &mut game, &mut sim, (player, item), 0);
        }

        let mut s: Sim = SimGame::with_world(game, sim, world);
        s.join(LOCAL_CLIENT_ID, Some(player), None, client_state::IN_GAME)
            .unwrap();
        s.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        let multi = SkillEntry {
            skill: MULTI,
            base: 10,
            owner_guid: -1,
            ..SkillEntry::default()
        };
        {
            let mut b = book.get();
            b.list = vec![
                SkillEntry {
                    skill: ATTACK,
                    base: 1,
                    owner_guid: -1,
                    ..SkillEntry::default()
                },
                multi,
            ];
            b.right = Some(multi);
        }

        let link = LocalLink::new(Host::new(
            s,
            ProtoSizes,
            PendingSession::default(),
            Ms(1000),
        ));
        let tap = Tap {
            inner: link,
            chunks: Vec::new(),
            inject: vec![akara_add(npc_guid, class::AKARA, NPC_AT)],
        };
        let mut bridge = Bridge::with_dispatch(tap, Dispatch::from_spec().unwrap()).unwrap();
        // Akara's class row in the client tables (an `interact` NPC), so
        // her add creates the unit.
        let mut tables = bridge.inputs().tables.clone();
        tables.monsters = vec![None; usize::from(class::AKARA) + 1];
        tables.monsters[usize::from(class::AKARA)] = Some(MonsterClass {
            npc: true,
            interact: true,
            ..MonsterClass::default()
        });
        bridge.set_tables(tables);
        let mut fx = Fx {
            bridge,
            player,
            wp_unit: object,
            wp,
            book,
            level_rooms,
            npc,
            buckler,
            cap,
            inv,
            due: None,
        };
        fx.stage_facts();
        fx
    }

    /// The path position (sub-tiles).
    fn pos(&mut self, u: UnitId) -> (i32, i32) {
        self.sim().events.action.hooks().path_position(u)
    }

    /// The dispatcher's range checks (unit distance and
    /// point range) read `SimGame`'s staged unit facts, not the path record
    /// (a known gap): staged from the
    /// path positions before each request, for every unit the run
    /// targets.
    fn stage_facts(&mut self) {
        let mut units = vec![self.player, self.wp_unit, self.npc];
        units.extend(self.monsters());
        for u in units {
            let p = self.pos(u);
            self.sim().set_unit(u, facts(p));
        }
    }

    /// What the unwritten kind init and movement specs would set for the
    /// monster: its target flags. The player's
    /// to-hit inputs: attack rating 100, level 1. The
    /// cast's aim point is the monster's path position. (The run-time
    /// collision bit at the monster's position is the path code's
    /// footprint now.)
    fn stage_combat(&mut self, monster: UnitId) {
        let player = self.player;
        let at = self.pos(monster);
        let sim = self.sim();
        sim.events.action.sys.units.get_mut(monster).unwrap().flags |=
            unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
        sim.events.action.hooks().x.aim_at = at;
        sim.events.action.with(&mut sim.game, |_, v| {
            v.set_base(player, 19, 100);
            v.set_base(player, 12, 1);
        });
    }

    /// The skill damage setup is not written yet (`Pending`):
    /// the missile's damage stats (21, 22, 1/256 points) are set here.
    fn set_missile_damage(&mut self, m: UnitId, d: i32) {
        let sim = self.sim();
        sim.events.action.with(&mut sim.game, |_, v| {
            v.set_base(m, 21, d);
            v.set_base(m, 22, d);
        });
    }

    /// A copy of the unit's dynamic path record.
    fn path(&mut self, u: UnitId) -> d2_sim::path::DynamicPath {
        let h = self.sim().events.action.hooks();
        h.paths.as_ref().unwrap().dynamic(u).cloned().unwrap()
    }

    /// The missile units of the game, in id order.
    fn missiles(&self) -> Vec<UnitId> {
        let mut v = self.sim_ref().game.lists.units_of_type(UnitType::Missile);
        v.sort();
        v
    }

    /// The unit's timer events (type, expiry), sorted.
    fn timers(&self, u: UnitId) -> Vec<(u8, i32)> {
        let t = &self.sim_ref().game.timers;
        let mut v: Vec<_> = t
            .unit_timers(u)
            .into_iter()
            .filter_map(|i| Some((t.event(i)?.0, t.expire(i)?)))
            .collect();
        v.sort();
        v
    }

    /// The drops so far: item and spot.
    fn drops(&self) -> Vec<(UnitId, DropSpot)> {
        self.sim_ref()
            .events
            .action
            .sys
            .hooks
            .object_drops
            .as_ref()
            .unwrap()
            .placed
            .clone()
    }

    fn sim(&mut self) -> &mut Sim {
        &mut self.bridge.link_mut().inner.host_mut().game
    }

    fn sim_ref(&self) -> &Sim {
        &self.bridge.link().inner.host().game
    }

    fn pending(&mut self) -> &mut TestPending {
        &mut self.sim().events.action.hooks().x
    }

    /// The game's one item store.
    fn items(&mut self) -> &mut ItemStore {
        &mut self.sim().events.action.sys.hooks.items
    }

    /// The player's items in its inventory (link order), the game's one
    /// inventory model.
    fn inventory(&self) -> Vec<UnitId> {
        let inv = self.sim_ref().world.inventory.as_ref().unwrap();
        inv.state.items_of(self.player)
    }

    fn guid(&self, u: UnitId) -> u32 {
        self.sim_ref().game.lists.unit(u).unwrap().guid
    }

    fn mode(&self, u: UnitId) -> u32 {
        self.sim_ref().events.action.sys.units.get(u).unwrap().mode
    }

    fn room(&self, u: UnitId) -> Option<RoomId> {
        self.sim_ref().game.lists.unit(u).unwrap().room()
    }

    /// Stat `s` of unit `u` (unit getter).
    fn stat(&mut self, u: UnitId, s: u16) -> i32 {
        let sim = self.sim();
        sim.events.action.with(&mut sim.game, |_, v| v.stat(u, s))
    }

    /// The player's timer events (type, expiry).
    fn player_timers(&self) -> Vec<(u8, i32)> {
        self.timers(self.player)
    }

    /// The monsters other than Akara, in id order.
    fn monsters(&self) -> Vec<UnitId> {
        let mut v = self.sim_ref().game.lists.units_of_type(UnitType::Monster);
        v.retain(|&m| m != self.npc);
        v.sort();
        v
    }

    /// Every error so far: world adapters, level types, action adapters,
    /// the unit dispatch, the world handlers' faults, the stubs reached.
    fn errors(&self) -> Vec<String> {
        let s = self.sim_ref();
        let mut e = s.events.errors();
        e.extend(s.world.action.faults.iter().map(|f| format!("{f:?}")));
        e.extend(s.world.state.errors.iter().map(|f| format!("{f:?}")));
        if let Some(i) = &s.world.inventory {
            e.extend(i.state.errors.iter().map(|f| format!("{f:?}")));
        }
        e.extend(s.tick_faults.iter().map(|f| format!("{f:?}")));
        e.extend(s.unhandled.iter().map(|f| format!("unhandled {f:?}")));
        e
    }

    /// Sends `msgs` through the bridge, advances the host clock 40 ms
    /// (one tick) and runs one bridge frame. The frame's
    /// result codes, the S→C bytes the client received, the bridge
    /// report.
    fn step(&mut self, msgs: &[Vec<u8>]) -> Step {
        for m in msgs {
            assert_eq!(self.bridge.send_bytes(m).unwrap(), Sent::Queued);
        }
        self.bridge.link_mut().inner.host_mut().clock.0 += 40;
        let report = self.bridge.frame().unwrap();
        assert!(report.ticked);
        let host = self.bridge.link().inner.last_frame();
        let codes = host
            .messages
            .iter()
            .map(|h| match h.handled {
                Handled::Game(Outcome::Dispatched(c)) => (h.id, Some(c)),
                _ => (h.id, None),
            })
            .collect();
        Step {
            codes,
            received: self.bridge.log().unowned.clone(),
            report,
        }
    }
}

const LOCAL_CLIENT_ID: u32 = d2_client::bridge::LOCAL_CLIENT;

fn facts((x, y): (i32, i32)) -> UnitFacts {
    UnitFacts {
        act: 0,
        pos: Pos { x, y },
        owner: None,
    }
}

/// One bridge frame: (C→S id, result) per drained message, the
/// bridge's unowned S→C counts so far, the frame report.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    codes: Vec<(u8, Option<ResultCode>)>,
    received: BTreeMap<u8, u64>,
    report: FrameReport,
}

fn bytes<M: d2_proto::FixedMessage>(m: &M) -> Vec<u8> {
    d2_client::bridge::intent::encode(m)
}

/// The precise coordinate of a sub-tile's centre.
fn centre(x: i32) -> u32 {
    ((x as u32) << 16) | 0x8000
}

// ---- the run ----------------------------------------------------------------------------

/// One recorded bridge frame: the C→S bytes sent, the drained results,
/// the S→C chunks the client received.
type Frame = (Vec<Vec<u8>>, Step, Vec<Vec<u8>>);

/// Everything a run leaves behind, compared byte for byte between runs.
#[derive(Debug, PartialEq, Eq)]
struct Transcript {
    frames: Vec<Frame>,
    /// Per walk: the player's (precise x, precise y, mode) per frame.
    walks: Vec<Vec<(u32, u32, u32)>>,
    game_frame: i32,
    game_seed: Seed,
    /// Monsters: GUID, unit seed, path position, mode.
    monsters: Vec<(u32, Seed, (i32, i32), u32)>,
    active_rooms: Vec<RoomId>,
    /// The player's path position, room and mode at the end.
    player_end: ((i32, i32), Option<RoomId>, u32),
    player_exp: i32,
    /// Dropped items: GUID, unit seed, spot, mode.
    drops: Vec<(u32, Seed, i32, i32, u32)>,
    pending_log: Vec<String>,
    skill_log: Vec<String>,
    /// The player's level, stamina and gold at the end.
    player_stats: Vec<i32>,
    /// Akara's store: GUID, record, item seed, AC.
    store: Vec<(u32, usize, Seed, i32)>,
    rest_log: Vec<String>,
    inv_log: Vec<String>,
    inventory: Vec<UnitId>,
    client: (u64, u64, usize),
    errors: Vec<String>,
}

/// A recording bridge frame.
/// A frame's received messages with each 0x9C / 0x9D item bit stream
/// checked and cut off (size byte = header size),
/// so the steps state the item headers: the stream must decode to its
/// exact length with `d2-proto`'s reader on the game's item tables and
/// carry the item's code when the item is still in the game.
fn streams(fx: &Fx, msgs: &[Vec<u8>]) -> Vec<Vec<u8>> {
    use d2_server::adapters::item_bits::TablesLookup;
    let sim = fx.sim_ref();
    msgs.iter()
        .map(|m| {
            let head = match m[0] {
                0x9C => 8,
                0x9D => 13,
                _ => return m.clone(),
            };
            assert_eq!(usize::from(m[2]), m.len(), "size byte {m:?}");
            let bits = d2_proto::item_bits::decode(&m[head..], &TablesLookup(&sim.world.tables))
                .unwrap_or_else(|e| panic!("stream of {m:?}: {e}"));
            let guid = u32::from_le_bytes(m[4..8].try_into().unwrap());
            let unit = sim
                .game
                .lists
                .find_unit(d2_sim::units::UnitType::Item, guid);
            if let Some(it) = unit.and_then(|u| sim.events.action.sys.hooks.items.get(u)) {
                assert_eq!(
                    bits.code, sim.world.tables.items[it.record].code,
                    "code of {guid}"
                );
            }
            let mut h = m[..head].to_vec();
            h[2] = head as u8;
            h
        })
        .collect()
}

/// S→C 0x07 MapReveal of the room at tile (x, y) of `level`
/// (`server-messages.tsv`: x u16 @1, y u16 @3, level u8 @5).
/// S→C 0x51 AssignObject (the object add, part A).
fn assign_object(guid: u32, class: u16, (x, y): (i32, i32), mode: u8, interact: u8) -> Vec<u8> {
    d2_sim::units::messages::assign_object(guid, class, x as u16, y as u16, mode, interact).to_vec()
}

fn map_reveal(x: u16, y: u16, level: u32) -> Vec<u8> {
    let mut b = vec![0x07];
    b.extend(x.to_le_bytes());
    b.extend(y.to_le_bytes());
    b.push(level as u8);
    b
}

fn record(fx: &mut Fx, frames: &mut Vec<Frame>, msgs: Vec<Vec<u8>>) {
    let step = fx.step(&msgs);
    // Each received message is accounted once:
    // applied, queued on its unit, dropped
    // (unit-handler message for a unit the model does not hold), unowned
    // or rejected. No 0x04 arrives, so no update pass runs.
    let chunks = std::mem::take(&mut fx.bridge.link_mut().chunks);
    let r = &step.report;
    assert_eq!(
        r.handled + r.queued + r.dropped + r.unowned + r.rejected,
        r.messages
    );
    assert_eq!(r.drained, 0);
    frames.push((msgs, step, chunks));
}

/// Sends a walk / run request (unit facts staged from the paths first),
/// then records frames while the player keeps the mode the request
/// started (at most `max`). The first frame drains the one request with
/// result 0; every frame's S→C is empty (as in
/// the game, the walking player's own client receives no movement message).
/// Per frame: the player's precise position and mode.
fn walk(fx: &mut Fx, frames: &mut Vec<Frame>, msg: Vec<u8>, max: usize) -> Vec<(u32, u32, u32)> {
    fx.stage_facts();
    let id = msg[0];
    let player = fx.player;
    let mut out = Vec::new();
    let mut moving = None;
    let mut first = Some(msg);
    for i in 0..max {
        record(fx, frames, first.take().into_iter().collect());
        let f = frames.last().unwrap();
        if i == 0 {
            assert_eq!(f.1.codes, [(id, Some(ResultCode::Done))]);
            moving = Some(fx.mode(player));
        } else {
            assert!(f.1.codes.is_empty());
        }
        let now = fx.sim_ref().game.frame;
        match fx.due.take() {
            Some((at, m)) if at == now => assert_eq!(f.2, vec![m], "walk frame {i}"),
            due => {
                assert!(f.2.is_empty(), "walk frame {i}: {:?}", f.2);
                fx.due = due;
            }
        }
        let d = fx
            .sim()
            .events
            .action
            .hooks()
            .paths
            .as_ref()
            .unwrap()
            .dynamic(player)
            .unwrap()
            .clone();
        let m = fx.mode(player);
        out.push((d.precise_x, d.precise_y, m));
        if Some(m) != moving {
            break;
        }
    }
    fx.stage_facts();
    out
}

fn run() -> Transcript {
    run_with(GAME_SEED)
}

fn run_with(game_seed: u32) -> Transcript {
    let mut fx = Fx::new(game_seed);
    let mut frames = Vec::new();
    let mut walks: Vec<Vec<(u32, u32, u32)>> = Vec::new();
    let (player, wp) = (fx.player, fx.wp);
    let none: Vec<Vec<u8>> = Vec::new();
    let done = Some(ResultCode::Done);

    // Before any frame: one active room (the player's), no monster; the
    // allocations placed the player, the waypoint and Akara on their
    // paths at the requested sub-tiles.
    assert_eq!(fx.level_rooms.len(), 15);
    assert_eq!(fx.sim_ref().game.lists.active_rooms(0).len(), 1);
    assert!(fx.monsters().is_empty());
    assert_eq!(fx.pos(player), PLAYER_AT);
    assert_eq!(fx.pos(fx.wp_unit), WP_AT);
    let npc = fx.npc;
    assert_eq!(fx.pos(npc), NPC_AT);

    // Frame 1 at 1000 ms: the tick driver starts its clock; no tick.
    let r = fx.bridge.frame().unwrap();
    assert!(!r.ticked);

    // Frame 2 (tick 1), join: the client's room change activates the
    // rooms near the player's (4 rooms) and sends one
    // S→C 0x07 per room of the player's adjacency array, in its order
    // (the room's recipients: tile x, tile y, level); the
    // room pass creates the DS1's preset monster at its sub-tile,
    // placed on its path. Each joined room's
    // 0x07 is followed by the add messages of its units
    // (the unit adds): the waypoint's 0x51 (type 2,
    // class 0, its position, mode 1, interact 0: no object data in this
    // game) after its room's. Each monster (the preset monster, GUID 2,
    // then Akara, GUID 1, class 148) sends the add's part A: 0xAC
    // (life 128, a one-byte stream of mode 1 and
    // no optional blocks), 0xAA (its states: none), then part B's mode
    // message (mode 1: 0x6D). The client's own player is
    // skipped. This staged game sends no 0x03, so the client refuses
    // each 0x07 (fatal 0x58A).
    record(&mut fx, &mut frames, vec![]);
    let mut want: Vec<Vec<u8>> = [(8000, 8000), (8000, 8008), (8008, 8000), (8008, 8008)]
        .map(|(x, y)| map_reveal(x, y, ISLE))
        .to_vec();
    want.insert(1, assign_object(wp, 0, WP_AT, 1, 0));
    let monster_adds: [Vec<u8>; 6] = [
        vec![0xAC, 2, 0, 0, 0, 0, 0, 76, 156, 74, 156, 128, 14, 1],
        vec![0xAA, 1, 2, 0, 0, 0, 8, 0xFF],
        vec![0x6D, 2, 0, 0, 0, 76, 156, 74, 156, 128],
        vec![0xAC, 1, 0, 0, 0, 148, 0, 86, 156, 86, 156, 128, 14, 1],
        vec![0xAA, 1, 1, 0, 0, 0, 8, 0xFF],
        vec![0x6D, 1, 0, 0, 0, 86, 156, 86, 156, 128],
    ];
    for (i, m) in monster_adds.into_iter().enumerate() {
        want.insert(1 + i, m);
    }
    assert_eq!(frames[0].2, want);
    assert_eq!(fx.sim_ref().game.lists.active_rooms(0).len(), 4);
    let monsters = fx.monsters();
    assert_eq!(monsters.len(), 1, "the DS1 preset monster");
    let monster = monsters[0];
    let mpos = fx.pos(monster);
    assert_eq!(mpos, (40_012, 40_010));
    assert_eq!(
        fx.pending().log,
        [format!("preset {} class 0 at 12,10", monster.0)]
    );
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
    let start_room = fx.room(player);

    // 4. Walk toward the monster (C→S 0x01):
    // result 0, mode WK (2); the per-tick player step (event 0)
    // moves the path every frame toward (40016, 40014) and stops on the
    // target's centre in neutral. Nothing is sent to the
    // walking player's own client.
    let target = (40_016, 40_014);
    let w = walk(
        &mut fx,
        &mut frames,
        bytes(&Walk {
            x: target.0 as u16,
            y: target.1 as u16,
        }),
        80,
    );
    assert_eq!(w.len(), 20);
    assert!(w[..19].iter().all(|f| f.2 == 2));
    for pair in w.windows(2) {
        // Every frame moves toward the target (−x, −y), never past it.
        let (a, b) = (pair[0], pair[1]);
        assert!(b.0 <= a.0 && b.1 <= a.1 && (b.0, b.1) != (a.0, a.1));
        assert!(b.0 >= centre(target.0) && b.1 >= centre(target.1));
    }
    assert_eq!(w[19], (centre(target.0), centre(target.1), 1));
    assert_eq!(fx.pos(player), target);
    assert_eq!(fx.room(player), start_room);
    walks.push(w);

    // 5. Right skill at the monster (C→S 0x0C) from there:
    // accepted, mana charged at start (3,328 of 4,000), srvst 53, mode SC
    // (10); the action frame (event 0, 3 frames on) runs srvdo 53 and the
    // generic `srvmissile` 0 through the real missile creation
    // at the player, aimed at the monster.
    fx.stage_combat(monster);
    let cast = bytes(&RightSkill {
        x: mpos.0 as u16,
        y: mpos.1 as u16,
    });
    record(&mut fx, &mut frames, vec![cast]);
    let f0 = fx.sim_ref().game.frame;
    assert_eq!(frames.last().unwrap().1.codes, [(0x0C, done)]);
    assert_eq!(fx.mode(player), 10);
    assert_eq!(fx.stat(player, 8), 4000 - 3328);
    assert_eq!(fx.player_timers(), [(0, f0 + 3), (1, f0 + 7)]);
    for _ in 0..3 {
        record(&mut fx, &mut frames, vec![]);
    }
    assert_eq!(
        fx.book.get().log,
        [
            // srvst 53 ran at the dispatch frame f0 − 1 (before the tick)
            // with level 10: param 1 := f0 − 1 + 10.
            format!("param1 1 {}", f0 - 1 + 10),
            "srvdo 53 1 10 true false false".to_string()
        ]
    );
    let shot = fx.missiles();
    assert_eq!(shot.len(), 1);
    assert_eq!(fx.pos(shot[0]), target, "created at the player");
    // The skill damage setup is not written yet: 10 points.
    fx.set_missile_damage(shot[0], 2560);
    fx.pending().log.clear();
    // The missile creation builds the missile's path: a missile
    // path (flag 0x40000) of type 4,
    // velocity (Vel 16 · 256) · 75 / 100 = 3072
    // (3072 · 0x400 >> 6) · 2896 >> 12 per axis and
    // frame, one diagonal sub-tile every one or two frames.
    let d = fx.path(shot[0]);
    assert_eq!(d.flags & 0x60000, 0x60000);
    assert_eq!((d.path_type, d.velocity), (4, 3072));
    let mut flight = Vec::new();
    while fx.missiles() == shot {
        flight.push(fx.pos(shot[0]));
        record(&mut fx, &mut frames, vec![]);
    }
    flight.dedup();
    assert_eq!(
        flight,
        [
            (40_016, 40_014),
            (40_015, 40_013),
            (40_014, 40_012),
            (40_013, 40_011)
        ]
    );
    // Frame 32: the step enters the monster's sub-tile, whose footprint
    // (0x100, size 1: `monstats2()`) the missile move collides with
    // (move mask 0x184): hit (to-hit
    // on the player's seed), damage 10 points ≥ the monster's 5 → life
    // 0, result 3 (damage steps 11–15: events 10, 9), the
    // missile removed (collide-kill). The reaction kills the
    // monster: its seam steps in order, the death mode change with
    // the player as target (mode DT 0, the death start); the
    // player's experience (equal levels → 100) and its
    // level-up event come in the kill's step 2, before the arena event.
    let f_hit = fx.sim_ref().game.frame;
    assert_eq!(f_hit, f0 + 10);
    assert!(fx.missiles().is_empty());
    assert_eq!(fx.mode(player), 1, "event 1 at f0 + 7 → neutral");
    assert_eq!(fx.stat(monster, 6), 0);
    assert_eq!(fx.mode(monster), 0);
    let (p, m) = (player.0, monster.0);
    assert_eq!(
        fx.pending().log,
        [
            format!("event 0 Some({m})"),
            format!("event 11 Some({m})"),
            format!("event 2 Some({m})"),
            format!("event 10 Some({m})"),
            format!("event 9 Some({p})"),
            format!("reaction {p} {m} 0x3"),
            format!("kill PetCredit {m} {p}"),
            // Step 2: the experience distribution, then the arena
            // kill event.
            format!("level up {p}"),
            format!("kill AttackerBookkeeping {m} {p}"),
            format!("kill FaceAttacker {m} {p}"),
            format!("death start {m} target Some({p})"),
            // No QuestKill step: the client host lends its quest control, so the
            // kill parse runs inside the kill (#1052).
            format!("kill BarricadeDoors {m} {p}"),
        ]
    );
    assert_eq!(fx.stat(player, 13), 100);
    assert_eq!(fx.stat(player, LEVEL), 2);
    // The death animation: 4 frames → event 1 four frames on. Tick 1's
    // room switch woke the monster created by that tick's room pass
    // (the unit add's AI wake-up: think at frame
    // 1 + 2; Idle → the next think at 203); the death clean-up's
    // timer removal cancelled that think.
    assert_eq!(fx.timers(monster), [(1, f_hit + 4)]);
    // The drop: TC 1 picks gold on the monster's
    // seed; the item is created on the game seed,
    // placed at the start spot (x + 2, y + 3) in mode 3, its
    // amount in stat 14.
    let drops = fx.drops();
    assert_eq!(drops.len(), 1);
    let (gold, spot) = drops[0];
    let mroom = fx.room(monster);
    assert_eq!(
        spot,
        DropSpot {
            room: mroom,
            x: mpos.0 + 2,
            y: mpos.1 + 3
        }
    );
    assert_eq!(fx.room(gold), mroom);
    assert_eq!(fx.mode(gold), 3);
    let amount = fx.stat(gold, GOLD);
    assert!((1..=6).contains(&amount), "roll(5 · 1) + 1: {amount}");
    // For the transcript (the pile is freed by the pick-up below).
    let drops = {
        let r = fx.sim_ref().events.action.sys.units.get(gold).unwrap();
        vec![(fx.guid(gold), r.seed, spot.x, spot.y, r.mode)]
    };
    // The kill's mode set (DT, unit flag 0x1) goes out in the client
    // pass of the hit's tick (the mode
    // message): S→C 0x69 code 8 at the path END (
    // the last computed point, (0, 0) for none; as a known drop run shows,
    // frame 37), d = the path direction, e =
    // unit, the killing hit's class (the
    // missile record has no class, so it is 13 `over`). No other S→C so far
    // but the join's 0x07s (frame 2): the unit-add / ground messages of
    // the missile and the drop belong to the per-unit update
    // pass, which the tick wiring does not run for them yet
    // (a known gap).
    let md = fx.path(monster);
    let mguid = fx.guid(monster);
    let mut code8 = vec![0x69];
    code8.extend(mguid.to_le_bytes());
    code8.push(8);
    // No computed point: the path end is (0, 0).
    assert_eq!(md.point_count, 0);
    code8.extend([0, 0, 0, 0]);
    code8.extend([md.direction, 13]);
    assert_eq!(&code8[5..], [8, 0, 0, 0, 0, md.direction, 13]);
    let (hit, before) = frames[1..].split_last().unwrap();
    // The player's own skill message (S→C 0x4D while in its attack
    // mode); nothing else up
    // to the hit.
    for f in before {
        let rest: Vec<_> = f.2.iter().filter(|m| m[0] != 0x4D).cloned().collect();
        assert_eq!(rest, none, "no S→C up to the hit but the 0x4D echo");
    }
    // The kill's 100 experience is a level-up:
    // S→C 0x75 to the client first, then the
    // 0x69 code 8, and after the update the relators 0x47 / 0x48 and the
    // level-up sound 0x2C (sound 2).
    let pgd = fx.guid(player).to_le_bytes();
    let mut levelup = vec![0x75];
    levelup.extend(pgd);
    levelup.extend([0xFF, 0xFF, 2, 0, 0, 0, 1, 0]);
    let mut relay47 = vec![0x47, 0, 0];
    relay47.extend(pgd);
    relay47.extend([0; 4]);
    let mut relay48 = relay47.clone();
    relay48[0] = 0x48;
    let mut sound = vec![0x2C, 0];
    sound.extend(pgd);
    sound.extend([2, 0]);
    assert_eq!(
        hit.2,
        vec![levelup, code8, relay47, relay48, sound],
        "0x75 level-up, 0x69 code 8, relators, level-up sound in the hit's frame"
    );
    // The death end: event 1 of the 4-frame DT animation (f_hit + 4)
    // sets mode 12 (the death end), whose 0x69 code 9 at the
    // monster's cell with e = 0 goes out in that tick (during the run of
    // step 7).
    let mut code9 = vec![0x69];
    code9.extend(mguid.to_le_bytes());
    code9.push(9);
    code9.extend((md.x() as u16).to_le_bytes());
    code9.extend((md.y() as u16).to_le_bytes());
    code9.extend([md.direction, 0]);
    fx.due = Some((f_hit + 4, code9));
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());

    // 5b. Pick-up of the kill's gold (C→S 0x16 cursor 0):
    // the staged distance 3 (< 5, `InvRest::
    // distance`); gold: limit = level 2 × 10000, take = p: stat
    // 14 += take; the pile leaves its room and is freed. Result 0. The
    // freed pile's removal S→C 0x0A (type 4) leaves in the per-client
    // update; as in the game (a known far pick-up's
    // packets, frame 54): the pick-up frame carries exactly that 0x0A and
    // the 0x2C sound, and no stat 14 message (0x1D / 0x1E / 0x1F) follows
    // in the 86 frames to the end of that run, so inventory gold is
    // not sent with the pick-up (the vitals sync, not wired
    // here).
    let gold_guid = fx.guid(gold);
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&PickItem {
            type_: 4,
            id: gold_guid,
            cursor: 0,
        })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x16, done)]);
    let mut removal = vec![0x0A, 4];
    removal.extend_from_slice(&gold_guid.to_le_bytes());
    // Then the pick-up sound S→C 0x2C on the player (as in the game:
    // a known Amazon pick-up's packets match).
    let mut sound = vec![0x2C, 0];
    sound.extend_from_slice(&fx.guid(player).to_le_bytes());
    sound.extend_from_slice(&[1, 0]);
    assert_eq!(
        streams(&fx, &frames.last().unwrap().2),
        vec![removal, sound]
    );
    assert!(fx.sim_ref().game.lists.unit(gold).is_none(), "freed");
    let gold_picked = PLAYER_GOLD + amount;
    assert_eq!(fx.stat(player, GOLD), gold_picked);
    assert_eq!(
        fx.inv.with(|r| std::mem::take(&mut r.log)),
        [format!("pickup_sound {} {gold_guid}", fx.guid(player))]
    );
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());

    // 6. The cursor pick-up (C→S 0x16 cursor 1) and placement need an
    // item that is not gold: a ground item the economy wiring makes the
    // way the drop does (game seed, mode 3 in the
    // player's room): a cap, identified, its ground position in its item
    // data.
    let room = fx.room(player);
    let cap = {
        let sim = fx.sim();
        let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
        let cap = world.with_economy(game, events, |econ, _| {
            let mut rq = ItemRequest {
                item: CAP as i32,
                ilvl: 1,
                quality: 2,
                format: 1,
                ..ItemRequest::default()
            };
            let spawn = ItemSpawn {
                room,
                mode: 3,
                init_flags: 1,
            };
            econ.create_item(&mut rq, false, spawn).expect("cap")
        });
        events.action.sys.hooks.items.get_mut(cap).unwrap().flags |= flag::IDENTIFIED;
        let guid = game.lists.unit(cap).unwrap().guid;
        world.inventory.as_mut().unwrap().state.items.insert(
            cap,
            InvItem {
                x: CAP_AT.0,
                y: CAP_AT.1,
                ..InvItem::new(guid, CAP)
            },
        );
        cap
    };
    // The item's unit allocation gives it no path: the item units are
    // made without a position (`item_units.rs`: "positions belong to the
    // path spec") and the path code sets an item's static
    // path only when it is added in mode 3 at a position.
    assert!(!fx.sim().events.action.hooks().path_has(cap));

    // 7. Run to the cap (C→S 0x03): mode RN (3), stamina drained per
    // running tick; stops on the cap's sub-tile in neutral.
    let stamina = fx.stat(player, STAT_STAMINA);
    let w = walk(
        &mut fx,
        &mut frames,
        bytes(&Run {
            x: CAP_AT.0 as u16,
            y: CAP_AT.1 as u16,
        }),
        80,
    );
    let n = w.len();
    assert!(w[..n - 1].iter().all(|f| f.2 == 3));
    assert_eq!(w[n - 1], (centre(CAP_AT.0), centre(CAP_AT.1), 1));
    assert_eq!(fx.pos(player), CAP_AT);
    assert!(fx.stat(player, STAT_STAMINA) < stamina);
    walks.push(w);

    // 8. Pick-up to the cursor (C→S 0x16 cursor 1)
    // → 0x9C action 1 in the tick's update pass, then 0x47,
    // 0x48 (the item bit stream is an open question: empty). The distance
    // test reads the item-move seam `InvRest::distance`,
    // answered here from the path positions: the unit distance of
    // the path code between the player (size 2) and the item (size 1)
    // after the walk above put the player on the
    // cap's sub-tile.
    let paths = PathTables::spec().unwrap();
    let at = |p: (i32, i32)| PathPoint { x: p.0, y: p.1 };
    let d = unit_distance(&paths, at(fx.pos(player)), 2, at(CAP_AT), 1);
    fx.inv.with(|r| r.distance = d);
    let pg = fx.guid(player);
    let cg = fx.guid(cap);
    let x9c = |action: u8, g: u32| {
        let mut b = vec![0x9C, action, 8, 0];
        b.extend_from_slice(&g.to_le_bytes());
        b
    };
    let pass = |m: Vec<u8>| {
        let mut r = vec![0x47, 0, 0];
        r.extend_from_slice(&pg.to_le_bytes());
        r.extend_from_slice(&[0; 4]);
        let mut r2 = r.clone();
        r2[0] = 0x48;
        vec![m, r, r2]
    };
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&PickItem {
            type_: 4,
            id: cg,
            cursor: 1,
        })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x16, done)]);
    // The pick-up also sends the sound S→C 0x2C on the player (as in
    // the game: a known Amazon pick-up's packets match).
    let mut sound = vec![0x2C, 0];
    sound.extend_from_slice(&pg.to_le_bytes());
    sound.extend_from_slice(&[1, 0]);
    let mut picked = pass(x9c(0x01, cg));
    picked.push(sound);
    assert_eq!(streams(&fx, &frames.last().unwrap().2), picked);
    assert_eq!(fx.mode(cap), 4);
    assert_eq!(fx.room(cap), None);
    // Placed at (8, 0) of page 0 (C→S 0x18): 0x9C action 4.
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&InsertItemInBuffer {
            item: cg,
            x: 8,
            y: 0,
            page: 0,
        })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x18, done)]);
    assert_eq!(streams(&fx, &frames.last().unwrap().2), pass(x9c(0x04, cg)));
    // The client model: the two
    // 0x9C made one item unit, holding the last message with its item bit
    // stream (placement from the stream is not
    // wired in the model yet). 0x47 / 0x48 name
    // the player, which the model does not hold (no 0x59): no change. The
    // waypoint is there since the join's room switch sent its 0x51;
    // Akara from the harness's add (`akara_add`).
    {
        use d2_client::bridge::world::{ItemData, ItemRecord, KindData, ITEM, MONSTER, OBJECT};
        let w = fx.bridge.world();
        assert_eq!(
            w.units.keys().copied().collect::<Vec<_>>(),
            [
                UnitKey::new(MONSTER, fx.guid(fx.npc)),
                UnitKey::new(OBJECT, wp),
                UnitKey::new(ITEM, cg)
            ]
        );
        let item = &w.units[&UnitKey::new(ITEM, cg)];
        assert_eq!(item.position, None);
        assert_eq!(
            item.kind,
            KindData::Item(ItemData {
                last: Some(ItemRecord {
                    id: 0x9C,
                    action: 4,
                    category: 0,
                    owner: None,
                    seq: 0,
                    stream: frames
                        .last()
                        .unwrap()
                        .2
                        .iter()
                        .find(|m| m[0] == 0x9C)
                        .unwrap()[8..]
                        .to_vec(),
                }),
                flags4: false,
                // Item data := the stream's flag word
                // (the stream read). No 0x2000: the
                // cap sat in a room's update queue, whose clean-up cleared
                // it.
                flags: 0x0080_0010,
                ..ItemData::default()
            })
        );
    }
    assert_eq!(fx.mode(cap), 0);
    assert!(fx.inventory().contains(&cap));
    assert_eq!(
        fx.inv.with(|r| std::mem::take(&mut r.log)),
        [
            format!("quest_item_picked {cg}"),
            format!("pickup_sound {pg} {cg}"),
        ]
    );

    // 9. Run to Akara (C→S 0x04, unit form, type 1): the path targets
    // her unit. The player's stop distance (path) has no written
    // setter (0), so the run stops at the first
    // step where the unit distance is 0: a negative `dist8_unit` entry
    // one diagonal cell short of her sub-tile (the talk's approach:
    // short of the NPC's own sub-tile for any NPC size ≥ 2).
    let ng = fx.guid(npc);
    let w = walk(
        &mut fx,
        &mut frames,
        bytes(&RunToUnit { type_: 1, id: ng }),
        80,
    );
    let n = w.len();
    assert!(w[..n - 1].iter().all(|f| f.2 == 3));
    // The run now stops at distance 0 beside her (Δ=(1,1)), not on her
    // sub-tile (`unit_distance`).
    let stop = (NPC_AT.0 + 1, NPC_AT.1 + 1);
    assert_eq!(w[n - 1], (centre(stop.0), centre(stop.1), 1));
    assert_eq!(fx.pos(player), stop);
    walks.push(w);

    // 10. Talk (C→S 0x13): S→C 0x27, 0x29, 0x28 in order
    // (0x27 bytes 6–39 the unwritten text-list encoder's, zeros). The
    // talk range uses the vendor rest's staged distance 3 (`Rest`), not
    // the path (the same unrouted distance call).
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&InteractWithEntity { type_: 1, id: ng })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x13, done)]);
    let mut npc_info = vec![0x27, 1];
    npc_info.extend_from_slice(&ng.to_le_bytes());
    npc_info.extend_from_slice(&[0; 34]);
    let mut game_quests = vec![0x29];
    game_quests.extend_from_slice(&fx.sim_ref().world.quests.game.0);
    let mut quest_info = vec![0x28, 1];
    quest_info.extend_from_slice(&ng.to_le_bytes());
    quest_info.push(0);
    quest_info.extend_from_slice(&fx.sim_ref().world.rest.quests[&player].flags[0].0);
    assert_eq!(
        frames.last().unwrap().2,
        [npc_info, game_quests, quest_info]
    );
    // The client holds Akara (`akara_add`): its 0x28 handler sets her flag
    // 0x2 and queues C→S 0x2F, then the dialog branch's 0x31 slot
    // (an open question, decided as A). No UI answers the
    // branch here, so the next frame drops the slot and sends the 0x2F.
    assert_eq!(
        fx.bridge.world().units[&UnitKey::new(1, ng)].flag_2,
        Some(true)
    );

    // 11. Trade (C→S 0x38 action 1): the store
    // generated (1–3 bucklers, then the permanent cap). One 0x9C action 11
    // per store item.
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&EntityAction {
            action: 1,
            npc: ng,
            item: 0,
        })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x2F, done), (0x38, done)]);
    let trade_frame = frames.len() - 1;
    let store = {
        let w = &fx.sim_ref().world;
        let rec = &w.state.vendors[w.state.vendor_index(class::AKARA).unwrap()];
        assert!(rec.has_traded && rec.store_generated);
        rec.store.clone()
    };
    assert!((2..=4).contains(&store.len()), "{store:?}");
    let mut store_rows = Vec::new();
    for &item in &store {
        let guid = fx.guid(item);
        let ac = fx.stat(item, ARMORCLASS);
        let it = fx.items().get(item).unwrap().clone();
        store_rows.push((guid, it.record, it.item_seed, ac));
    }
    assert_eq!(store_rows.last().unwrap().1, CAP, "permanent codes last");
    // One 0x9C action 11 per store item, in store order. The
    // frame also carries the 0x2F's heal SetStat (Akara heals,
    // as the chat open's heal).
    let shown: Vec<(u8, u8, u32)> = frames[trade_frame]
        .2
        .iter()
        .filter(|m| m[0] == 0x9C)
        .map(|m| (m[0], m[1], u32::from_le_bytes(m[4..8].try_into().unwrap())))
        .collect();
    let want: Vec<(u8, u8, u32)> = store_rows.iter().map(|r| (0x9C, 11, r.0)).collect();
    assert_eq!(shown, want);

    // 12. Sell (C→S 0x33) the picked-up cap: a
    // permanent code, so no copy; S→C 0x9D action 5, removed from the
    // inventory and freed;
    // the price (B = 100·AC/5, B·512/1024) received: S→C
    // 0x2A kind 3, code 1, its GUID, the new gold.
    let sold = (100 * fx.stat(cap, ARMORCLASS) / 5) * 512 / 1024;
    assert!(sold > 0);
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&SellItem {
            npc: ng,
            item: cg,
            item_mode: 0,
            client_price: 0,
        })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x33, done)]);
    let gold_now = gold_picked + sold;
    // A stored cap (mode 0): S→C 0x9D action 5 (flags 0x20) before the
    // 0x2A (sale step 9, the message order).
    let sent = streams(&fx, &frames.last().unwrap().2);
    assert_eq!(sent.len(), 2, "{sent:02X?}");
    assert_eq!((sent[0][0], sent[0][1]), (0x9D, 0x05));
    assert_eq!(sent[0][4..8], cg.to_le_bytes());
    assert_eq!(sent[1], tx(3, 1, cg, gold_now));
    assert_eq!(fx.stat(player, GOLD), gold_now);
    assert!(!fx.inventory().contains(&cap));
    assert!(fx.sim_ref().game.lists.unit(cap).is_none(), "freed");

    // 13. Buy (C→S 0x32) the store's cap: steps 1–8 pass; the purchase
    // loop (step 9) copies the store cap (on the
    // inventory model), pays (by hand: 100·AC/5, sell mult 1024)
    // and auto-places the copy in the backpack: S→C 0x2A code 0, kind 4,
    // the copy's GUID, the new gold; its 0x9C action 4 in the update
    // pass. The permanent cap stays in the store (rule 12).
    let price = 100 * fx.stat(*store.last().unwrap(), ARMORCLASS) / 5;
    let store_cap = fx.guid(*store.last().unwrap());
    record(
        &mut fx,
        &mut frames,
        vec![bytes(&BuyItem {
            npc: ng,
            item: store_cap,
            transaction: 0,
            client_price: 0,
        })],
    );
    assert_eq!(frames.last().unwrap().1.codes, [(0x32, done)]);
    let bought = *fx.inventory().last().unwrap();
    assert!(!store.contains(&bought), "a new unit");
    let bg = fx.guid(bought);
    let gold_now = gold_now - price;
    let mut want = vec![tx(4, 0, bg, gold_now)];
    want.extend(pass(x9c(0x04, bg)));
    assert_eq!(streams(&fx, &frames.last().unwrap().2), want);
    assert_eq!(fx.stat(player, GOLD), gold_now);
    assert_eq!(fx.inventory(), [fx.buckler, fx.cap, bought]);

    // 14. Walk to the waypoint object (C→S 0x02, type 2): the walk ends
    // beside the object at distance 0 (Δ=(0,1) here), not on its
    // sub-tile (`unit_distance`).
    let og = fx.guid(fx.wp_unit);
    let w = walk(
        &mut fx,
        &mut frames,
        bytes(&WalkToUnit { type_: 2, id: og }),
        80,
    );
    let n = w.len();
    assert!(w[..n - 1].iter().all(|f| f.2 == 2));
    assert_eq!(w[n - 1], (centre(WP_AT.0), centre(WP_AT.1 + 1), 1));
    walks.push(w);

    // 15. Waypoint travel to the GATE level (C→S 0x49)
    // with the menu open (the operate path is the object spec's:
    // the interaction staged on the player's unit record). With the path
    // provider the same-act warp places the player in the destination's
    // spawn room, so rule 7 holds: in the
    // drain the client gets 0x07 MapReveal of the placement room, then
    // 0x0D at the player's position + 3.
    {
        let rec = fx.sim().events.action.sys.units.get_mut(player).unwrap();
        rec.interact.reset();
        rec.interact.set(2, wp);
    }
    let travel = bytes(&TakeOrCloseWp {
        wp,
        level: GATE as u16,
    });
    record(&mut fx, &mut frames, vec![travel]);
    assert_eq!(frames.last().unwrap().1.codes, [(0x49, done)]);
    let at = fx.pos(player);
    let room = fx.room(player).expect("placed in a room");
    let rect = {
        let h = fx.sim().events.action.hooks();
        h.drlg.subtile_rect(room).unwrap()
    };
    assert!(rect.contains(at.0, at.1));
    // In the GATE level: 40 × 18 tiles at tile (9000, 9000).
    assert!((45_000..45_200).contains(&at.0) && (45_000..45_090).contains(&at.1));
    let reveal = MapReveal {
        x: (rect.x / 5) as u16,
        y: (rect.y / 5) as u16,
        level: GATE as u8,
    };
    let stop = PlayerStop {
        type_: 0,
        guid: pg,
        a: 1,
        x: at.0 as u16 + 3,
        y: at.1 as u16 + 3,
        b: 0,
        life_pct: 0,
    };
    // Then the tick's room switch: one 0x07
    // per room of the placement room's adjacency array (none of them was
    // in the old array, which held the Isle's rooms), in its order, with
    // no unit in them; then the leave of each Isle room in the old array's
    // order: 0x0A for each of its units (list order; no missile is left),
    // 0x08, and after the client's old room the player update (rule 3.4:
    // 0x15 with flag 1 at the player's new cell, flags 2 bit 0x10000).
    let switch: Vec<Vec<u8>> = {
        let s = fx.sim_ref();
        let (d, r) = s
            .events
            .action
            .sys
            .hooks
            .drlg
            .drlg_room(&s.game, room)
            .unwrap();
        d.active_room(r)
            .unwrap()
            .adjacency
            .iter()
            .map(|&a| {
                let rr = d.room(a);
                map_reveal(rr.rect.x as u16, rr.rect.y as u16, GATE)
            })
            .collect()
    };
    assert_eq!(switch.len(), 6);
    let leave: Vec<Vec<u8>> = {
        let s = fx.sim_ref();
        let old = start_room.expect("the player's Isle room");
        let (d, r) = s
            .events
            .action
            .sys
            .hooks
            .drlg
            .drlg_room(&s.game, old)
            .unwrap();
        let mut v = Vec::new();
        for &a in &d.active_room(r).unwrap().adjacency {
            let rr = d.room(a);
            let id = rr.active().expect("still active").id;
            for u in s.game.lists.room_units(id) {
                let e = s.game.lists.unit(u).unwrap();
                assert_ne!(e.ty, UnitType::Missile);
                v.push(d2_sim::units::messages::remove_unit(e.ty as u8, e.guid).to_vec());
            }
            v.push(
                d2_sim::units::messages::map_hide(rr.rect.x as u16, rr.rect.y as u16, ISLE as u8)
                    .to_vec(),
            );
            if id == old {
                let p15 = ReassignPlayer {
                    type_: 0,
                    guid: pg,
                    x: at.0 as u16,
                    y: at.1 as u16,
                    flag: 1,
                };
                v.push(p15.encode().to_vec());
            }
        }
        v
    };
    // The Isle's four rooms; the waypoint, Akara and the dead monster in
    // the player's.
    assert_eq!(leave.iter().filter(|m| m[0] == 0x08).count(), 4);
    assert_eq!(leave.iter().filter(|m| m[0] == 0x0A).count(), 3);
    let mut want = vec![reveal.encode().to_vec(), stop.encode().to_vec()];
    want.extend(switch);
    want.extend(leave);
    assert_eq!(frames.last().unwrap().2, want);
    let rec = fx.sim_ref().events.action.sys.units.get(player).unwrap();
    assert_eq!(rec.interact.get(), None);
    // The next tick: nothing (the 0x15
    // came with the room switch's player update).
    record(&mut fx, &mut frames, vec![]);
    assert_eq!(streams(&fx, &frames.last().unwrap().2), none);

    // The run logs no error (the missile path of step 5 flies).
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
    let w = fx.bridge.world();
    let client = (w.frames, w.server_ticks, w.units.len());
    assert_eq!(client.0, frames.len() as u64 + 1);
    let log = fx.bridge.log();
    // The S→C stream drove the client model:
    // 0x9C ×3, 0x47 ×3, 0x48 ×3
    // (the bought cap's too),
    // the waypoint's 0x51, the leave's three 0x0A and its 0x15 applied;
    // 0x0D dropped (the player was never announced: the server sends no
    // 0x59 / 0x0B in this staged game) and the monster's two 0x69 (codes
    // 8 and 9) dropped too (no 0xAC announced it: the
    // unit is not in the model); every 0x07 and 0x08 rejected (no client
    // act: this staged game sends no 0x03, fatal 0x58A / 0x59E): the
    // join's four 0x07, the warp's one and the six of its room switch,
    // the switch's four 0x08. The NPC / quest / trade ids are handled
    // now (0x27, 0x29, 0x28, 0x2A
    // ×2), and Akara's harness add (0xAC) too: 14 + 6; plus the sell's
    // 0x9D action 5 (sale step 9): 21; the join's monster
    // adds add 4: two 0xAC (the client
    // creates nothing from them yet: no `ClientTables` monster rows,
    // so nothing is made) and two 0xAA. The preset monster's 0x6D
    // is dropped like its 0x69s (not in the model); Akara's is queued on
    // its unit and never drained in this staged game.
    assert!(log.unowned.is_empty(), "{:?}", log.unowned);
    // + the trade open's 0x9C action 11, one per store item.
    // + the picked gold pile's removal 0x0A.
    // + the 0x2F's heal at Akara: its sound 0x2C.
    // + the two pick-up sounds 0x2C (gold, cap).
    // + the level-up of the kill: 0x75, 0x47, 0x48 and the sound 0x2C.
    assert_eq!(log.handled, 33 + store.len() as u64);
    assert_eq!(
        log.dropped,
        // The player's own 0x4D echo is gone: the caster's own
        // client gets the skill-mode message only for a used skill with
        // E flag 0x4 (as in the game;
        // the local player's cast mode).
        BTreeMap::from([(0x0D, 1), (0x69, 2), (0x6D, 1)])
    );
    assert_eq!((log.queued, log.drained), (1, 0));
    assert_eq!(fx.due, None, "the death end's 0x69 code 9 arrived");
    let rejected: Vec<(u8, String)> = log
        .rejected
        .iter()
        .map(|r| (r.id, r.error.to_string()))
        .collect();
    let mut want = vec![(0x07, "fatal assert 0x58A".to_owned()); 11];
    want.extend(vec![(0x08, "fatal assert 0x59E".to_owned()); 4]);
    // The heal's SetStat 0x1E at Akara (the chat open's heal), after the
    // join's four 0x07: no local player in this staged game
    // (fatal 0x9AA).
    want.insert(4, (0x1E, "fatal assert 0x9AA".to_owned()));
    assert_eq!(rejected, want);
    assert!(log.discarded.is_empty());
    // No local player: the world view has no camera.
    assert_eq!(w.local_player, None);
    assert!(w.rooms_in_sight.is_empty() && w.act.is_none());

    let skill_log = fx.book.get().log.clone();
    let player_stats = [LEVEL, STAT_STAMINA, GOLD]
        .iter()
        .map(|&st| fx.stat(player, st))
        .collect();
    let player_exp = fx.stat(player, 13);
    let inv_log = fx.inv.with(|r| r.log.clone());
    let inventory = fx.inventory();
    let monsters = fx
        .monsters()
        .into_iter()
        .map(|m| (fx.guid(m), m))
        .collect::<Vec<_>>();
    let monsters = monsters
        .into_iter()
        .map(|(g, m)| {
            let p = fx.pos(m);
            let r = fx.sim_ref().events.action.sys.units.get(m).unwrap();
            (g, r.seed, p, r.mode)
        })
        .collect();
    let player_end = (fx.pos(player), fx.room(player), fx.mode(player));
    let errors = fx.errors();
    let s = fx.sim_ref();
    Transcript {
        frames,
        walks,
        game_frame: s.game.frame,
        game_seed: s.events.action.sys.hooks.game_seed,
        monsters,
        active_rooms: s.game.lists.active_rooms(0),
        player_end,
        player_exp,
        drops,
        pending_log: s.events.action.sys.hooks.x.log.clone(),
        skill_log,
        player_stats,
        store: store_rows,
        rest_log: s.world.rest.log.clone(),
        inv_log,
        inventory,
        client,
        errors,
    }
}

#[test]
fn full_single_player_loop() {
    let t = run();
    // Frames per walk / run: 20, 33 and 14 (the runs move at the run
    // velocity: the run list's stat 67 +50), 6; the
    // run to Akara and the walk to the waypoint each stop 2 frames short
    // of the target's sub-tile (rc-object-approach); 94 recorded frames,
    // one tick each.
    let lens: Vec<usize> = t.walks.iter().map(Vec::len).collect();
    assert_eq!(lens, [20, 33, 14, 6]);
    assert_eq!(t.frames.len(), 94);
    assert_eq!(t.frames.len() as i32, t.game_frame);
    // The kill: 100 experience, one drop (the gold, picked up).
    assert_eq!((t.player_exp, t.drops.len()), (100, 1));
    // The picked-up cap sold, a copy of the store's cap bought.
    assert_eq!(t.inventory.len(), 3);
    // Gold moved by the pickup, the sale and the buy (each exact in the
    // run's steps).
    assert_ne!(t.player_stats[2], PLAYER_GOLD);
}

/// Same seeds → the same run: every C→S byte, result code, S→C chunk,
/// path position, seed, unit, room and log, twice.
#[test]
fn same_seed_same_run() {
    assert_eq!(run(), run());
}

/// The comparison sees the seed (a mutation check): another game seed gives other
/// unit seeds and another store; the walks (path code, no randomness
/// in them) and every result code agree.
#[test]
fn other_seed_other_run() {
    let (a, b) = (run(), run_with(GAME_SEED + 1));
    assert_ne!(a.game_seed, b.game_seed);
    assert_ne!(a.monsters, b.monsters);
    assert_ne!(a.store, b.store);
    assert_eq!(a.walks, b.walks);
    let codes = |t: &Transcript| {
        t.frames
            .iter()
            .map(|f| f.1.codes.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(codes(&a), codes(&b));
}

#[path = "e2e_full_loop/equipment_swap.rs"]
mod equipment_swap;
