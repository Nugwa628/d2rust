//! Robustness properties (`CLAUDE.md` hard rule 7) of the intent handlers on the
//! wired host: every C→S game id, with payload fields drawn from the real unit
//! GUIDs and from out-of-range values (unit types, GUIDs, skill and stat ids,
//! coordinates, item positions), goes through the dispatcher into
//! `SimGame::handle` without a panic and with a frame that does not fail.
//!
//! Three hosts, as the server's handler tests build them:
//! - the action host: `SimGame<ActionSim<Book>, ActionWorld<WiredSkills>>`
//!   (the skill handlers in the host's skill slot, the vitals tables on
//!   the action wiring) with waypoint tables: a player, a monster, a
//!   ground item and a waypoint object in one field room;
//! - the item host: `SimGame<ActionSim<_>, WiredWorld<_>>` with the
//!   cube's parts (`CubeParts`) and the inventory model (`InvParts`): a
//!   player, a cube in its inventory and a ring, units of the action sim,
//!   items in the game's one store;
//! - the trade host (below): the same `WiredWorld` with Akara, vendor
//!   tables and the player's items in its inventory.
//!
//! The dispatcher's own rejections (null handler, stub, wrong size, unit type,
//! point range) run before any handler, so the game state is compared before and
//! after them. A handler's refusal is not checked that way: a handler may change
//! state before it refuses (the cube's 0x2A resets the targeting flag first).
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder (it overrides every default here).
//!
//! Integration test of `d2-server`; it reuses the end-to-end fixtures of
//! `d2-client/tests` and guards the dispatcher and all intent handlers together.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use d2_data::bin::BinTable;
use d2_data::fixup::maps::{EquivMatrix, StateMaps};
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{
    Charstats, Experience, Itemratio, Itemstatcost, Itemtypes, Levels, Objects, Record, Skilldesc,
    Skills, States,
};
use d2_proto::schema::FieldType;
use d2_proto::CLIENT_MESSAGES;
use d2_server::adapters::handlers::items::moves::{InvParts, MoveRest};
use d2_server::adapters::handlers::items::{CubeParts, ItemPending};
use d2_server::adapters::handlers::skills::wired::WiredSkills;
use d2_server::adapters::handlers::skills::LearnRest;
use d2_server::adapters::handlers::world::{ActionEvents, ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::buffers::ClientBuffers;
use d2_server::dispatch::{dispatch, gate, is_point, is_unit, kind, Gate, Kind};
use d2_server::seams::{PlayerGate, Pos, ResultCode, Tick};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind as DrlgRoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{InvTables, UnitKind as InvKind};
use d2_sim::items::moves::{Guid, MovePending, Owner};
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{q, ty, ItemRequest, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::{SkillEntry, SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::{ClassStats, StatData, StatTable, StateTable};
use d2_sim::tick::EventDispatch;
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::economy::{GameFields, ItemSpawn};
use d2_sim::wiring::interaction::UseRest;
use d2_sim::wiring::inventory::InvRest;
use d2_sim::world::cube::{
    input_flags, kind as cube_kind, CraftMod, CubeData, InputSlot, ItemRecord, OutputSlot, Recipe,
};
use d2_sim::world::npc::NpcControl;
use d2_sim::world::quests::{QuestControl, QuestTables};
use d2_sim::world::vendors::VendorTables;
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};
use proptest::prelude::*;
use proptest::test_runner::Config;

/// Proptest config with `default` cases, or `PROPTEST_CASES` when set.
fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

const N_STATS: usize = 359;

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost (359 stats, no ops, no shifts) through the
/// d2-data fix-up, with an empty states table.
fn stat_data() -> Arc<StatData> {
    let size = Itemstatcost::SIZE;
    let mut records = vec![0u8; N_STATS * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            set_u16(r, o, 0xFFFF);
        }
        set_u16(r, 0, s as u16);
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: N_STATS,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    let states = BinTable {
        name: "states".into(),
        source: "synthetic".into(),
        count: 0,
        record_size: States::SIZE,
        records: Vec::new(),
    };
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        states: StateTable::new(&states, &StateMaps::default()).expect("states"),
        damage_regen: vec![0; 8],
        aurastate: vec![0; 8],
        rescale_precision: d2_sim::stats::DEFAULT_RESCALE_PRECISION,
    })
}

// ---- the action host -------------------------------------------------------------------

pub struct NoTiles;
impl TileSource for NoTiles {
    fn dt1(&self, _: &[u8]) -> Option<&[TileInfo]> {
        None
    }
}

/// A skills record with no formulas and no required skills.
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
    s.reqskill1 = 0xFFFF;
    s.reqskill2 = 0xFFFF;
    s.reqskill3 = 0xFFFF;
    s.itypea1 = 0xFFFF;
    s.srvmissile = 0xFFFF;
    s.intown = true;
    s.ingame = true;
    s
}

/// Attack (0), Multiple Shot (1: srvst 52, mana), Might (2: aura), a
/// learnable skill (3: max level 3).
/// Start slot of the synthetic start skills: srvst 52 (Emerge,
/// the Emerge skill's start: flags |= 0xE, returns 1) stands in for Multiple
/// Shot's srvst 4, whose body (ammunition) needs items; every filled start slot
/// has a body. The do slot 53 (filled, `unreferenced`, no body) stands in for
/// Might's do slot 65, so the fake's seam answers.
fn skills() -> SkillTables {
    let mut v: Vec<Skills> = (0..4).map(|_| skill_rec()).collect();
    let m = &mut v[1];
    (m.srvstfunc, m.mana, m.lvlmana, m.manashift) = (52, 4, 1, 8);
    let m = &mut v[2];
    (m.aura, m.immediate, m.perdelay, m.srvdofunc, m.aurastate) = (true, true, 0, 53, 33);
    v[3].maxlvl = 3;
    SkillTables {
        skills: v,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: Vec::new(),
        skills_code: vec![0x07, 50, 0x00],
        miss_code: Vec::new(),
        level_cap: LEVEL_CAP_114D,
        stat_count: N_STATS as _,
    }
}

/// The class `charstats` rows and `experience` with MaxLvl 99.
fn vitals() -> VitalsTables {
    let rows: [[u8; 13]; 7] = [
        [20, 25, 15, 20, 84, 30, 8, 4, 6, 12, 4, 6, 5],
        [10, 25, 35, 10, 74, 30, 4, 4, 8, 8, 4, 8, 5],
        [15, 25, 25, 15, 79, 30, 6, 4, 8, 8, 4, 8, 5],
        [25, 20, 15, 25, 89, 30, 8, 4, 6, 12, 4, 6, 5],
        [30, 20, 10, 25, 92, 30, 8, 4, 4, 16, 4, 4, 5],
        [15, 20, 20, 25, 84, 30, 6, 4, 8, 8, 4, 8, 5],
        [20, 20, 25, 20, 95, 30, 8, 5, 6, 12, 5, 7, 5],
    ];
    let charstats = rows
        .iter()
        .map(|r| {
            let mut c: Charstats = blank();
            (c.str, c.dex, c.int, c.vit, c.stamina, c.hpadd) = (r[0], r[1], r[2], r[3], r[4], r[5]);
            (c.lifeperlevel, c.staminaperlevel, c.manaperlevel) = (r[6], r[7], r[8]);
            (c.lifepervitality, c.staminapervitality, c.manapermagic) = (r[9], r[10], r[11]);
            c.statperlevel = r[12];
            c
        })
        .collect();
    let mut row: Experience = blank();
    row.amazon = 99;
    VitalsTables {
        charstats,
        experience: vec![row],
    }
}

/// The action wiring's `Pending` value: the seams without a provider
/// (positions, interaction, warp, arrival mode, transport, as the
/// bridge's local tests give them), the skill use pipeline's `UseRest`
/// and the skill-point calls (`LearnRest`): a fixed skill list (every
/// skill of the table), the right hand on Multiple Shot; the test keeps
/// a handle.
#[derive(Default)]
struct Inner {
    list: Vec<SkillEntry>,
    left: Option<SkillEntry>,
    right: Option<SkillEntry>,
    used: Option<SkillEntry>,
    pos: BTreeMap<UnitId, (i32, i32)>,
    sent: Vec<(UnitId, Vec<u8>)>,
}

#[derive(Clone, Default)]
struct Book(Arc<Mutex<Inner>>);

impl Book {
    fn get(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }
}

impl Pending for Book {
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.get().pos.get(&unit).copied().unwrap_or_default()
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.get().pos.insert(unit, (x, y));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.get().sent.push((player, msg.to_vec()));
    }
    fn warp(&mut self, _: &mut Game, _: UnitId, _: u32, _: u8) {}
    fn set_player_mode_arrival(&mut self, _: &mut Game, _: UnitId) {}
    fn skill_list(&self, _: UnitId) -> Vec<SkillEntry> {
        self.get().list.clone()
    }
    fn used_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.get().used
    }
}

impl Outbox for Book {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.get().sent)
    }
}

/// The narrowest answer wherever nothing is staged (as the server's
/// skill handler tests give it). The message path's player data,
/// positions, reach and sends are the server's (`skills::world::World`).
impl UseRest for Book {
    fn send(&mut self, _: UnitId, _: ServerMsg) {}
    fn has_player_data(&self, _: UnitId) -> bool {
        false
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
        false
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
        self.get().left
    }
    fn right_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.get().right
    }
    fn set_left_skill(&mut self, _: UnitId, e: SkillEntry) {
        self.get().left = Some(e);
    }
    fn set_right_skill(&mut self, _: UnitId, e: SkillEntry) {
        self.get().right = Some(e);
    }
    fn find_entry(&self, _: UnitId, skill: i32) -> Option<SkillEntry> {
        self.get().list.iter().copied().find(|e| e.skill == skill)
    }
    fn find_entry_owned(&self, _: UnitId, skill: i32, owner: i32) -> Option<SkillEntry> {
        self.get()
            .list
            .iter()
            .copied()
            .find(|e| e.skill == skill && e.owner_guid == owner)
    }
    fn owns_skill(&self, _: UnitId, _: i32) -> bool {
        false
    }
    fn set_used_skill(&mut self, _: UnitId, e: Option<SkillEntry>) {
        self.get().used = e;
    }
    fn used_skill_flags(&self, _: UnitId) -> u32 {
        0
    }
    fn set_used_skill_flags(&mut self, _: UnitId, _: u32) {}
    fn entry_mode(&self, _: UnitId, _: &SkillEntry) -> u32 {
        0
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
        false
    }
    fn pay_life(&mut self, _: UnitId, _: i32) -> bool {
        false
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
        None
    }
    fn line_clear(&self, _: UnitId, _: (i32, i32), _: u32) -> bool {
        false
    }
    fn set_aura_state(&mut self, _: UnitId, _: u16, _: i32, _: i32) {}
    fn srvst(&mut self, _: u16, _: UnitId, _: i32, _: i32) -> i32 {
        1
    }
    fn srvdo(&mut self, _: u16, _: UnitId, _: i32, _: i32, _: bool, _: bool, _: bool) -> i32 {
        1
    }
}

impl LearnRest for Book {
    fn is_class_skill(&self, _: UnitId, skill: i32) -> bool {
        skill == 3
    }
    fn add_skill_level(&mut self, _: UnitId, _: i32, _: i32) {}
    fn after_skill_point(&mut self, _: UnitId) {}
}

/// One 8×8-tile room per listed level (the server's waypoint tests'
/// DRLG types).
struct FieldTypes(BTreeMap<u32, TileRect>);

impl LevelTypes for FieldTypes {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = drlg.level(level).id;
        if let Some(&rect) = self.0.get(&id) {
            let r = drlg.alloc_room(level, DrlgRoomKind::Preset, rect);
            drlg.room_mut(r).dt1_mask = 1;
            drlg.link_room(r, LinkAt::Tail);
        }
        Ok(())
    }
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
                g.set(x, y, d2_sim::drlg::tiles::cell::FLOOR);
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

struct FieldTiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for FieldTiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tile(orientation: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation,
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

/// Cold Plains, a field level (`levels` row 3, waypoint index 1).
const COLD_PLAINS: u32 = 3;
/// The other waypoint levels of [`waypoint_data`]: 1 (act 0, index 0)
/// and 40 (act 1, index 9).
const WAYPOINT_LEVELS: [u32; 3] = [1, COLD_PLAINS, 40];

/// A DRLG world for acts 0 and 1 where each waypoint level has one
/// 8×8-tile room, as the server's waypoint and skill handler tests build
/// it: the skill use pipeline reads the room kind from the unit's real
/// room (`UseView`), so the units stand in Cold Plains (a field room),
/// and a waypoint travel finds a spawn room on every level the waypoint
/// table lists.
fn field_drlg() -> DrlgWorld {
    use d2_sim::drlg::tiles::FIXED_LIBRARY;
    let mut data = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut data.levels {
        l.warp = [-1; 8];
    }
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    data.lvltypes = vec![vec![Vec::new(); 32], files];
    for id in WAYPOINT_LEVELS {
        data.levels[id as usize].drlg_type = 2;
        data.levels[id as usize].level_type = 1;
    }
    let mut types = FieldTypes(
        WAYPOINT_LEVELS
            .iter()
            .map(|&id| (id, TileRect::new(0, 0, 8, 8)))
            .collect(),
    );
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    dungeon.acts[1] = Some(Drlg::create(1, 2, 0, 0, false, &data, &mut types).unwrap());
    let mut t = BTreeMap::new();
    t.insert(b"floor.dt1".to_vec(), vec![tile(0, 0, 0, 1)]);
    let blank = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(FIXED_LIBRARY[0].to_vec(), vec![blank(0), blank(1)]);
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(FieldTiles(t)),
        types: Box::new(types),
    }
}

/// [`field_drlg`]'s Cold Plains room, generated and streamed on `sim`
/// (act 0 is made in `game`'s lists).
fn field_room<X: Pending>(sim: &mut ActionSim<X>, game: &mut Game) -> RoomId {
    game.lists.ensure_act(0).unwrap();
    sim.hooks()
        .drlg
        .with_act(0, &mut game.lists, |d, svc| {
            let l = d.get_or_alloc_level(svc.data, svc.types, COLD_PLAINS)?;
            d.generate_level(svc.data, svc.types, l)?;
            let r = d.level_rooms(l)[0];
            d.stream_room(svc, r)
        })
        .unwrap()
        .unwrap()
        .unwrap()
}

/// `levels` (150 rows, acts by id; waypoints on levels 1 and 3, and 40 in
/// act 1) and one waypoint object class 0.
fn waypoint_data() -> WaypointData {
    let mut levels = vec![blank::<Levels>(); 150];
    for (i, l) in levels.iter_mut().enumerate() {
        l.waypoint = NO_WAYPOINT;
        l.act = if i >= 40 { 1 } else { 0 };
    }
    levels[1].waypoint = 0;
    levels[3].waypoint = 1;
    levels[40].waypoint = 9;
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 15 << 8;
    WaypointData::new(&levels, &[o])
}

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

type ActionGame = SimGame<ActionSim<Book>, ActionWorld<WiredSkills>>;

/// The player (class `class`, mode NU, at (100, 100)) for client 0, a
/// monster at (120, 100), an item on the ground at (101, 100) and a
/// waypoint object at (102, 100), all in one field room; the vitals
/// tables on the action wiring (0x3A), the skill handlers in the host's
/// skill slot.
fn action_host(class: u32) -> (ActionGame, Vec<u32>) {
    let book = Book::default();
    {
        let mut b = book.get();
        b.list = (0..4)
            .map(|skill| SkillEntry {
                skill,
                base: 1,
                owner_guid: -1,
                ..SkillEntry::default()
            })
            .collect();
        b.right = Some(b.list[1]);
        b.left = Some(b.list[0]);
    }
    let tables = ActionTables {
        missiles: Vec::new(),
        skills: skills(),
        combat: CombatTables {
            charstats: Vec::new(),
            difficultylevels: Vec::new(),
            monstats: Vec::new(),
            monstats2: Vec::new(),
            hitclass: Vec::new(),
        },
        levels: Vec::new(),
        skill_modes: Vec::new(),
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    };
    let mut hooks = ActionHooks::new(
        Arc::new(tables),
        field_drlg(),
        Seed::init_low(1234),
        book.clone(),
    );
    hooks.vitals = Some(Arc::new(vitals()));
    let data = UnitData {
        monsters: vec![MonsterInfo {
            mode_chart: false,
            enabled: true,
            aidel: [15; 3],
            moves: 0,
        }],
        ..UnitData::default()
    };
    let mut events = ActionSim::new(stat_data(), data, hooks);
    let mut game = Game::new();
    let room = field_room(&mut events, &mut game);
    let mut alloc = |ty, class| {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        events
            .with(&mut game, |g, v| v.allocate(g, &req, 20, 20))
            .expect("allocated")
    };
    let player = alloc(UnitType::Player, class);
    let monster = alloc(UnitType::Monster, 0);
    let item = alloc(UnitType::Item, 0);
    let wp = alloc(UnitType::Object, 0);
    events.sys.units.get_mut(player).unwrap().mode = 1;
    // Stat points (4) and skill points (5) to spend, so 0x3A and 0x3B get
    // past their first check; level
    // (12) 1, the learnable skill's required level (reqlevel 0 + base 1),
    // so 0x3B reaches the spend.
    events.with(&mut game, |_, v| {
        v.set_base(player, 12, 1);
        v.set_base(player, 4, 5);
        v.set_base(player, 5, 5);
    });
    let world = ActionWorld {
        waypoints: Some(waypoint_data()),
        skills: WiredSkills::default(),
        ..ActionWorld::default()
    };
    let mut sim: ActionGame = SimGame::with_world(game, events, world);
    sim.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    sim.set_player(
        player,
        PlayerFields {
            gate: ALIVE,
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    let at = |x, y| UnitFacts {
        act: 0,
        pos: Pos { x, y },
        owner: None,
    };
    sim.set_unit(player, at(100, 100));
    sim.set_unit(monster, at(120, 100));
    sim.set_unit(item, at(101, 100));
    sim.set_unit(wp, at(102, 100));
    for (u, x) in [(player, 100), (monster, 120), (item, 101), (wp, 102)] {
        book.get().pos.insert(u, (x, 100));
    }
    let guids = [player, monster, item, wp]
        .iter()
        .map(|&u| sim.game.lists.unit(u).unwrap().guid)
        .collect();
    (sim, guids)
}

// ---- the item host ---------------------------------------------------------------------

const N_TYPES: usize = 80;
const T_RING: u16 = 10;
const T_BOX: u16 = 11;
const T_AMULET: u16 = 12;
const CUBE: usize = 0;
const RING: usize = 1;
const AMULET: usize = 2;

fn equiv() -> EquivMatrix {
    let n = N_TYPES;
    let words = n.div_ceil(32);
    let mut m = EquivMatrix {
        n,
        words,
        bits: vec![0; n * words],
    };
    let mut set = |i: usize, j: usize| m.bits[i * words + j / 32] |= 1 << (j % 32);
    for i in 1..n {
        set(i, 0);
        set(i, i);
    }
    for c in [T_RING, T_BOX, T_AMULET] {
        set(usize::from(c), usize::from(ty::MISC));
    }
    m
}

fn item_rec(t: u16, code: &[u8; 4]) -> ItemRec {
    ItemRec {
        code: *code,
        type_: t as i16,
        level: 1,
        ..ItemRec::default()
    }
}

/// Items: the cube (`box `), a ring, an amulet.
fn item_tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    let itemtypes = (0..N_TYPES)
        .map(|_| {
            let mut t = Itemtypes::decode(&[0u8; Itemtypes::SIZE]);
            t.class = 0xFF;
            t.staffmods = 0xFF;
            // Empty `shoots`: the link miss (link16 −1).
            t.shoots = 0xFFFF;
            t.rare = 1;
            t
        })
        .collect();
    ItemTables {
        items: vec![
            item_rec(T_BOX, b"box "),
            item_rec(T_RING, b"rin "),
            item_rec(T_AMULET, b"amu "),
        ],
        itemtypes,
        equiv: equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// A cube recipe shape without mods: one ring → a normal amulet.
fn cube_data(t: &ItemTables) -> CubeData {
    let mut inputs = [InputSlot::default(); 7];
    inputs[0] = InputSlot {
        flags: input_flags::USEANY,
        item: RING as u16,
        ..InputSlot::default()
    };
    let out = OutputSlot {
        kind: cube_kind::ITEMCODE,
        item: AMULET as u16,
        quality: q::NORMAL,
        mods: [CraftMod {
            property: -1,
            ..CraftMod::default()
        }; 5],
        ..OutputSlot::default()
    };
    let recipe = Recipe {
        enabled: 1,
        class: 0xFF,
        numinputs: 1,
        inputs,
        outputs: [out, OutputSlot::default(), OutputSlot::default()],
        ..Recipe::default()
    };
    CubeData {
        recipes: vec![recipe],
        items: t
            .items
            .iter()
            .map(|r| ItemRecord {
                code: r.code,
                level: r.level,
                spawnable: 1,
                ..ItemRecord::default()
            })
            .collect(),
        valshift: vec![0; N_STATS],
        max_level: 99,
    }
}

/// [`ItemPending`] stand-in: nothing happens.
struct ItemRest;

impl ItemPending for ItemRest {
    fn inventory_pass(&mut self, _: UnitId, _: &mut Vec<Vec<u8>>) {}
    fn duplicate(&mut self, _: UnitId, _: bool) -> Option<UnitId> {
        None
    }
    fn tempered_affix(&mut self, _: UnitId, _: bool) -> u16 {
        0
    }
    fn drop_runeword_stats(&mut self, _: UnitId) {}
    fn repair(&mut self, _: UnitId) {}
    fn recharge(&mut self, _: UnitId) {}
    fn quest_item_hook(&mut self, _: UnitId, _: UnitId, _: [u8; 4]) {}
    fn cow_portal(&mut self, _: UnitId) -> bool {
        false
    }
}

/// The item-move seams no d2-sim module provides, answered narrowest
/// (`InvRest` / `MovePending` defaults: distance 0, no free spot, no
/// interaction); the sends are collected for the host.
#[derive(Default)]
struct InvStandIn {
    sent: Vec<(Owner, Vec<u8>)>,
}

impl MovePending for InvStandIn {
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.sent.push((player, bytes));
    }
}

impl InvRest for InvStandIn {
    fn item_active_on(&self, _: Guid, _: Owner) -> bool {
        false
    }
    fn own_contribution(&self, _: Guid, _: Owner, _: u16) -> i32 {
        0
    }
    fn level_requirement(&self, _: Guid, _: Owner) -> i32 {
        -1
    }
    fn two_handed(&self, _: Guid) -> bool {
        false
    }
    fn one_or_two_handed(&self, _: Owner, _: Guid) -> bool {
        false
    }
    fn ammo_type(&self, _: Guid) -> Option<i16> {
        None
    }
    fn has_allowed_location(&self, _: Guid) -> bool {
        true
    }
    fn quiver_kind(&self, _: Guid) -> bool {
        false
    }
    fn player_data_4c(&self, _: Owner) -> u32 {
        0
    }
    fn player_data_50(&self, _: Owner) -> u32 {
        0
    }
    fn npc_talking(&self, _: Owner, _: Owner) -> bool {
        false
    }
    fn player_trade_gate(&self, _: Owner) -> Option<bool> {
        None
    }
}

impl MoveRest for InvStandIn {
    fn take_sent(&mut self) -> Vec<(Owner, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// Inventory tables (grid records) over items of
/// (code, type, invwidth, invheight), in record order.
fn inv_tables(items: &[([u8; 4], u16, u8, u8)], n_types: usize, equiv: EquivMatrix) -> InvTables {
    let g = |x, y| GridRec {
        grid_x: x,
        grid_y: y,
    };
    let mut grids = vec![g(10, 4); 16];
    grids[5] = g(10, 10);
    grids[8] = g(6, 4);
    grids[9] = g(3, 4);
    grids[12] = g(6, 8);
    grids[13] = g(0, 0);
    InvTables {
        grids,
        belts: vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16],
        items: items
            .iter()
            .map(|&(code, t, w, h)| InvItemRec {
                code,
                type_: t as i16,
                invwidth: w,
                invheight: h,
                ..InvItemRec::default()
            })
            .collect(),
        itemtypes: vec![
            InvTypeRec {
                class: 7,
                ..InvTypeRec::default()
            };
            n_types
        ],
        equiv,
        books: Vec::new(),
        item_use: Default::default(),
    }
}

/// The inventory parts of a wired host, with the player's inventory
/// (added at player creation).
fn inv_parts(tables: InvTables, player: UnitId, class: u8, guid: u32) -> InvParts {
    let mut parts = InvParts::new(tables, Box::new(InvStandIn::default()));
    parts
        .state
        .add_inventory(player, InvKind::Player { class }, guid);
    parts
}

/// Places `item` (put on the cursor, page `page`) into the player's
/// inventory through the inventory model's placement (free position, no
/// "send"): a fixture's stored item.
fn store_item(sim: &mut trade::TradeGame, player: UnitId, item: UnitId, page: u8) {
    sim.events.sys.units.get_mut(item).unwrap().mode = 4;
    sim.events.sys.hooks.items.get_mut(item).unwrap().inv_page = page;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    let placed = world.with_economy(game, events, |econ, p| {
        let inv = p.inventory.as_deref_mut().expect("inventory parts");
        inv.desk(econ).place(player, item, (0, 0), true, false)
    });
    assert!(placed);
}

type ItemGame = trade::TradeGame;

fn new_item(sim: &mut ItemGame, class: usize, mode: u32) -> UnitId {
    let mut rq = ItemRequest {
        item: class as i32,
        format: 101,
        ilvl: 5,
        quality: q::NORMAL,
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: None,
        mode,
        init_flags: 1,
    };
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    world
        .with_economy(game, events, |econ, _| {
            econ.create_item(&mut rq, false, spawn)
        })
        .unwrap()
}

/// The wired host as the server's cube tests build it: game creation on
/// the action sim (expansion, the game seed), the NPC control and the
/// quests on the game seed, the cube's parts on the host; a player
/// (class 2) for client 0 with the cube stored in its inventory and a
/// ring in mode `ring_mode`, real units of the action sim.
fn item_host(ring_mode: u32) -> (ItemGame, Vec<u32>) {
    let tables = item_tables();
    let hooks = ActionHooks::new(
        Arc::new(empty_action_tables()),
        field_drlg(),
        Seed::init(),
        trade::ActionRest::default(),
    );
    let mut events = ActionSim::new(stat_data(), UnitData::default(), hooks);
    events.create_game(&GameFields::new(Seed::init_low(0x5EED), true));
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let mut seed = events.hooks().game_seed;
    let npc = NpcControl::new(&[], Vec::new(), true, 0, &mut seed).unwrap();
    let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
    events.hooks().game_seed = seed;
    let mut world = WiredWorld::new(
        ActionWorld::default(),
        tables,
        quests,
        npc,
        VendorTables::default(),
        trade::Rest::default(),
        1000,
    );
    let mut parts = CubeParts::new(cube_data(&world.tables), Box::new(ItemRest));
    parts.staged.local_date = (15, 3);
    world.cube = Some(parts);
    let req = AllocRequest {
        ty: UnitType::Player,
        class: 2,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let player = events
        .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    let pg = events.sys.units.get(player).unwrap().guid;
    let items = [
        (*b"box ", T_BOX, 2, 2),
        (*b"rin ", T_RING, 1, 1),
        (*b"amu ", T_AMULET, 1, 1),
    ];
    world.inventory = Some(inv_parts(
        inv_tables(&items, N_TYPES, equiv()),
        player,
        2,
        pg,
    ));
    let mut sim: ItemGame = SimGame::with_world(game, events, world);
    let cube = new_item(&mut sim, CUBE, 4);
    store_item(&mut sim, player, cube, 0);
    let ring = new_item(&mut sim, RING, ring_mode);
    sim.events.sys.hooks.items.get_mut(ring).unwrap().inv_page = 0;
    if ring_mode == 4 {
        let inv = sim.world.inventory.as_mut().unwrap();
        inv.state
            .inventories
            .get_mut(&player)
            .unwrap()
            .set_cursor(Some(ring));
    }
    sim.join(0, Some(player), None, client_state::IN_GAME)
        .unwrap();
    sim.set_player(
        player,
        PlayerFields {
            gate: ALIVE,
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    let at = |x, y| UnitFacts {
        act: 0,
        pos: Pos { x, y },
        owner: None,
    };
    sim.set_unit(player, at(100, 100));
    sim.set_unit(ring, at(100, 100));
    let guids = [player, cube, ring]
        .iter()
        .map(|&u| sim.events.sys.units.get(u).unwrap().guid)
        .collect();
    (sim, guids)
}

/// Action tables with no skills (the economy hosts).
fn empty_action_tables() -> ActionTables {
    ActionTables {
        missiles: Vec::new(),
        skills: SkillTables {
            skills: Vec::new(),
            skilldesc: Vec::new(),
            missiles: Vec::new(),
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 0,
            stat_count: 0,
        },
        combat: CombatTables {
            charstats: Vec::new(),
            difficultylevels: Vec::new(),
            monstats: Vec::new(),
            monstats2: Vec::new(),
            hitclass: Vec::new(),
        },
        levels: Vec::new(),
        skill_modes: Vec::new(),
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    }
}

// ---- messages --------------------------------------------------------------------------

/// One generated C→S game message, built against the host's GUIDs.
#[derive(Clone, Debug)]
struct Gen {
    id: u8,
    /// Per layout field: a pool index and a random value.
    picks: Vec<(u8, u32)>,
    raw: Vec<u8>,
    /// 0: exact size; 1: one byte short; 2: one byte long.
    len: u8,
}

fn gen() -> impl Strategy<Value = Gen> {
    (
        1u8..0x67,
        prop::collection::vec((any::<u8>(), any::<u32>()), 8),
        prop::collection::vec(any::<u8>(), 0..300),
        prop_oneof![8 => Just(0u8), 1 => Just(1u8), 1 => Just(2u8)],
    )
        .prop_map(|(id, picks, raw, len)| Gen {
            id,
            picks,
            raw,
            len,
        })
}

/// Field values: from the field's domain (by layout name), or the edges
/// of every field's range, unit types 0..=7,
/// skill and stat ids around the tables' counts, coordinates near and
/// far from the player at (100, 100), item positions, the host's GUIDs.
fn pick(name: &str, guids: &[u32], (sel, rnd): (u8, u32)) -> u32 {
    // Half the time, a value from the field's own domain, so the
    // handlers get past the dispatcher's parse and their first checks.
    if sel & 0x80 != 0 {
        // A host GUID, or -1 ("no unit", e.g. 0x3C's item).
        let guid = match rnd as usize % (guids.len() + 1) {
            i if i < guids.len() => guids[i],
            _ => u32::MAX,
        };
        return match name {
            "x" | "y" => 100 + rnd % 101 - 50,
            "type" => rnd % 6,
            "id" | "item" | "cube" | "unit" | "npc" | "wp" | "target" | "merc" | "book"
            | "scroll" | "socketable" | "cursor" | "player" => guid,
            "skill" => rnd % 5,
            "cost" => rnd % 6000,
            "action" | "msg" | "quest" => rnd % 8,
            "stat" => rnd % 8,
            "left" => rnd % 2,
            "button" | "page" | "bodyloc" | "slot" | "level" | "tab" => rnd % 20,
            _ => rnd % 4,
        };
    }
    const POOL: &[u32] = &[
        0,
        1,
        2,
        3,
        4,
        5,
        6,
        7,
        15,
        16,
        0xFF,
        0xFFFF,
        0x7FFF,
        0x8000,
        0x7FFF_FFFF,
        0x8000_0000,
        u32::MAX,
        u32::MAX - 1,
        49,
        50,
        51,
        100,
        101,
        120,
        150,
        151,
        358,
        359,
        360,
    ];
    let n = POOL.len() + guids.len() + 1;
    match sel as usize % n {
        i if i < POOL.len() => POOL[i],
        i if i < POOL.len() + guids.len() => guids[i - POOL.len()],
        _ => rnd,
    }
}

/// The message bytes for `g`: its id's fixed size (0x14: 4..=275; 0x15:
/// its raw bytes) with each layout field set from the pool.
fn build(g: &Gen, guids: &[u32]) -> Vec<u8> {
    let row = &CLIENT_MESSAGES[g.id as usize];
    let mut m = g.raw.clone();
    match row.transport_size.fixed() {
        Some(n) => m.resize(n, 0),
        None if g.id == 0x14 => m.resize(m.len().clamp(4, 275), 0),
        None => m.truncate(0x200 - 1),
    }
    if m.is_empty() {
        m.push(0);
    }
    m[0] = g.id;
    for (f, &p) in row.layout.iter().zip(&g.picks) {
        let Some(off) = f.offset else { continue };
        let off = off as usize;
        let v = pick(f.name, guids, p);
        let (w, mask) = match f.ty {
            FieldType::U8 => (1, 0xFF),
            FieldType::U16 => (2, 0xFFFF),
            FieldType::U32 => (4, u32::MAX),
            FieldType::Bits(n) => (4, (1u32 << n) - 1),
            FieldType::Bit(n) => (4, 1u32 << n),
            _ => continue,
        };
        if off + w > m.len() {
            continue;
        }
        let mut word = [0u8; 4];
        word[..w].copy_from_slice(&m[off..off + w]);
        let old = u32::from_le_bytes(word);
        let v = match f.ty {
            FieldType::Bit(n) => (v & 1) << n,
            _ => v,
        };
        let new = (old & !mask) | (v & mask);
        m[off..off + w].copy_from_slice(&new.to_le_bytes()[..w]);
    }
    match g.len {
        1 if m.len() > 1 => {
            m.pop();
        }
        2 => m.push(0),
        _ => {}
    }
    m
}

/// The game state a rejected intent must leave alone, and the fatal
/// paths (asserts in the game rules) the handlers record instead of panicking.
trait Snapshot {
    fn snapshot(&self) -> String;
    fn faults(&self) -> Vec<String>;
}

impl Snapshot for ActionGame {
    fn snapshot(&self) -> String {
        let s = &self.events.sys;
        format!("{:?}{:?}{:?}", self.game, s.units, s.stats)
    }
    fn faults(&self) -> Vec<String> {
        let s = &self.events.sys;
        let mut v: Vec<String> = self.world.faults.iter().map(|f| format!("{f:?}")).collect();
        v.extend(s.errors.iter().map(|e| format!("{e:?}")));
        v.extend(s.hooks.errors.iter().map(|e| format!("{e:?}")));
        v.extend(self.tick_faults.iter().map(|f| format!("{f:?}")));
        v
    }
}

/// Runs `msgs` one per frame on `sim`: dispatch, then one tick. Every
/// dispatch returns; a stub returns 0; a wrong size is 3, a unit type ≥ 6 is 2
/// and a point target out of range is 1; the dispatcher's rejections leave the
/// state as it was; no handler or tick meets a fatal path (a game-rule assert).
fn run<D, W>(sim: &mut SimGame<D, W>, guids: &[u32], msgs: &[Gen]) -> Result<(), TestCaseError>
where
    D: EventDispatch + d2_sim::tick::TickHooks,
    W: d2_server::adapters::handlers::world::WorldHost<D>,
    SimGame<D, W>: Snapshot,
{
    let mut out = ClientBuffers::new();
    out.add_client(0);
    for g in msgs {
        let m = build(g, guids);
        let (id, size) = (m[0], m.len());
        let before = sim.snapshot();
        let stubs = sim.unhandled.len();
        let code = dispatch(sim, &ProtoSizes, &mut out, 0, ALIVE, &m, size);
        if sim.unhandled.len() > stubs {
            prop_assert_eq!(code, ResultCode::Done, "{:02X?}", m);
            prop_assert_eq!(kind(id), Kind::Handler);
        }
        let row = &CLIENT_MESSAGES[id as usize];
        // The player is alive: only the dead gate (0x41) is closed, and
        // a closed gate returns 0 before the size check.
        let open = gate(id) != Gate::Dead;
        if !open {
            prop_assert_eq!(code, ResultCode::Done, "{:02X?}", m);
        }
        // The dispatcher's own rejections, before any handler: they must
        // change nothing. (A handler's refusal may follow state changes it
        // makes first, e.g. the cube's targeting reset before a 3.)
        let mut rejected = open && kind(id) != Kind::Handler && code == ResultCode::Malformed;
        let handler = open && kind(id) == Kind::Handler;
        if handler && row.transport_size.fixed().is_some_and(|n| n != size) {
            prop_assert_eq!(code, ResultCode::Malformed, "{:02X?}", m);
            rejected = true;
        } else if handler && is_unit(id) {
            if u32::from_le_bytes([m[1], m[2], m[3], m[4]]) >= 6 {
                prop_assert_eq!(code, ResultCode::Invalid, "{:02X?}", m);
                rejected = true;
            }
        } else if handler && is_point(id) {
            // The player is staged at (100, 100) and never moved.
            let at = |o: usize| i32::from(u16::from_le_bytes([m[o], m[o + 1]]));
            if (at(1) - 100).abs() > 50 || (at(3) - 100).abs() > 50 {
                prop_assert_eq!(code, ResultCode::Refused, "{:02X?}", m);
                rejected = true;
            }
        }
        if rejected {
            prop_assert!(
                sim.snapshot() == before,
                "{:02X?} was rejected and changed the state",
                m
            );
        }
        sim.tick(&mut out);
        let faults = sim.faults();
        prop_assert!(faults.is_empty(), "{:02X?}: {:?}", m, faults);
    }
    Ok(())
}

proptest! {
    #![proptest_config(config(48))]

    /// Every game id with arbitrary fields on the action host.
    #[test]
    fn action_host_any_intents(class in 0u32..7, msgs in prop::collection::vec(gen(), 1..16)) {
        let (mut sim, guids) = action_host(class);
        run(&mut sim, &guids, &msgs)?;
    }

    /// Every game id with arbitrary fields on the item host.
    #[test]
    fn item_host_any_intents(ring_mode in 0u32..6, msgs in prop::collection::vec(gen(), 1..16)) {
        let (mut sim, guids) = item_host(ring_mode);
        run(&mut sim, &guids, &msgs)?;
    }
}

/// Each handled id alone, many times, on fresh hosts: the ids the
/// handler modules own (skills 0x05–0x11, 0x3A–0x3C; waypoints 0x49;
/// cube 0x2A, 0x4F; quests 0x31, 0x40, 0x58, which the item host's
/// `WiredWorld` handles) get most of the cases.
fn owned_id() -> impl Strategy<Value = u8> {
    prop::sample::select(vec![
        0x05u8, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x2A, 0x31,
        0x3A, 0x3B, 0x3C, 0x40, 0x49, 0x4F, 0x58,
    ])
}

proptest! {
    #![proptest_config(config(96))]

    #[test]
    fn handled_ids_any_fields(
        ids in prop::collection::vec(owned_id(), 1..8),
        gens in prop::collection::vec(gen(), 8),
        class in 0u32..7,
    ) {
        let msgs: Vec<Gen> = ids.iter().zip(gens).map(|(&id, g)| Gen { id, ..g }).collect();
        let (mut sim, guids) = action_host(class);
        run(&mut sim, &guids, &msgs)?;
        let (mut sim, guids) = item_host(class % 6);
        run(&mut sim, &guids, &msgs)?;
    }
}

/// The fixture is live: the hosts' handlers answer (a known message
/// reaches each handler module), so the properties exercise them.
#[test]
fn hosts_reach_the_handlers() {
    let (mut sim, guids) = action_host(1);
    let mut out = ClientBuffers::new();
    out.add_client(0);
    // 0x3C select skill 1 on the right hand: the skill handler.
    let mut m = vec![0x3C, 1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    let code = dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &m, 9);
    assert_eq!(code, ResultCode::Done);
    assert!(sim.unhandled.is_empty(), "{:?}", sim.unhandled);
    // 0x49 at the waypoint object: the waypoint handler.
    m = vec![0x49];
    m.extend_from_slice(&guids[3].to_le_bytes());
    m.extend_from_slice(&1u32.to_le_bytes());
    dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &m, 9);
    assert!(sim.unhandled.is_empty(), "{:?}", sim.unhandled);
    let (mut sim, guids) = item_host(0);
    // 0x2A with the ring and the cube: the cube handler.
    m = vec![0x2A];
    m.extend_from_slice(&guids[2].to_le_bytes());
    m.extend_from_slice(&guids[1].to_le_bytes());
    dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &m, 9);
    assert!(sim.unhandled.is_empty(), "{:?}", sim.unhandled);
    // 0x31, 0x40, 0x58 on the wired hosts: the quest handlers.
    let (mut trade, _) = trade::trade_host(1);
    for m in [
        vec![0x31, 1, 0, 0, 0, 0, 0, 0, 0],
        vec![0x40],
        vec![0x58, 5, 0],
    ] {
        for sim in [&mut sim, &mut trade] {
            dispatch(sim, &ProtoSizes, &mut out, 0, ALIVE, &m, m.len());
            assert!(sim.unhandled.is_empty(), "{:02X?}: {:?}", m, sim.unhandled);
        }
    }
}

// ---- the trade host --------------------------------------------------------------------
//
// `SimGame<ActionSim<_>, WiredWorld<_>>` as `d2-client`'s
// `e2e_vendor.rs` builds it: the NPC, vendor and quest handlers on
// `wiring::interaction` over the action sim's own units. Akara (class
// 148) next to the player; the player owns a buckler and a cap and
// carries 5000 gold. The interaction rest (`Rest`) and the item, vendor
// and NPC tables are the e2e fixtures' (`e2e_support`, shared with
// `d2-client/tests`).

/// The end-to-end fixtures of `d2-client/tests` (one copy for both
/// crates' tests).
#[path = "../../d2-client/tests/e2e_support/mod.rs"]
mod e2e_support;

mod trade {

    use std::sync::Arc;

    use d2_server::adapters::handlers::world::{ActionWorld, Outbox, WiredWorld};
    use d2_server::adapters::{PlayerData, PlayerFields, SimGame, UnitFacts};
    use d2_server::seams::Pos;
    use d2_sim::combat::CombatTables;
    use d2_sim::drlg::data::DrlgData;
    use d2_sim::drlg::{Dungeon, NoLevelTypes};
    use d2_sim::game::Game;

    use d2_sim::items::{ty, ItemRequest};
    use d2_sim::rng::Seed;
    use d2_sim::skills::SkillTables;
    use d2_sim::units::hooks::{MonsterInfo, UnitData};
    use d2_sim::units::lifecycle::AllocRequest;
    use d2_sim::units::lists::client_state;
    use d2_sim::units::{UnitId, UnitType};
    use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
    use d2_sim::wiring::economy::ItemSpawn;

    use d2_sim::world::npc::{class, NpcControl};
    use d2_sim::world::quests::{PlayerQuests, QuestControl, QuestTables};

    pub use super::e2e_support::Rest;
    use super::e2e_support::{
        equiv, item_tables, monstats, vendor_tables, BUC, CAP, N_MONSTATS, N_TYPES,
    };
    use super::{stat_data, NoTiles, Snapshot, ALIVE};

    /// The action wiring's seams: `Pending`'s defaults; sends kept.
    #[derive(Default)]
    pub struct ActionRest {
        pub sent: Vec<(UnitId, Vec<u8>)>,
    }

    impl Pending for ActionRest {
        fn send(&mut self, player: UnitId, msg: &[u8]) {
            self.sent.push((player, msg.to_vec()));
        }
    }

    impl Outbox for ActionRest {
        fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
            std::mem::take(&mut self.sent)
        }
    }

    pub type TradeGame = SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>;

    impl Snapshot for TradeGame {
        fn snapshot(&self) -> String {
            let s = &self.events.sys;
            let staged = self.world.cube.as_ref().map(|c| &c.staged);
            let inv = self.world.inventory.as_ref().map(|i| &i.state);
            format!(
                "{:?}{:?}{:?}{:?}{:?}{:?}",
                self.game, s.units, s.stats, s.hooks.items, staged, inv
            )
        }
        fn faults(&self) -> Vec<String> {
            let s = &self.events.sys;
            let mut e: Vec<String> = s.hooks.errors.iter().map(|e| format!("{e:?}")).collect();
            e.extend(s.errors.iter().map(|e| format!("{e:?}")));
            e.extend(self.world.state.errors.iter().map(|e| format!("{e:?}")));
            e.extend(self.world.action.faults.iter().map(|f| format!("{f:?}")));
            if let Some(c) = &self.world.cube {
                e.extend(c.errors.iter().map(|x| format!("{x:?}")));
            }
            if let Some(i) = &self.world.inventory {
                e.extend(i.state.errors.iter().map(|x| format!("{x:?}")));
            }
            e.extend(self.tick_faults.iter().map(|f| format!("{f:?}")));
            e
        }
    }

    /// The host and its GUIDs: player, Akara, buckler, cap.
    pub fn trade_host(game_seed: u32) -> (TradeGame, Vec<u32>) {
        let tables = ActionTables {
            missiles: Vec::new(),
            skills: SkillTables {
                skills: Vec::new(),
                skilldesc: Vec::new(),
                missiles: Vec::new(),
                skills_code: Vec::new(),
                miss_code: Vec::new(),
                level_cap: 0,
                stat_count: 0,
            },
            combat: CombatTables {
                charstats: Vec::new(),
                difficultylevels: Vec::new(),
                monstats: Vec::new(),
                monstats2: Vec::new(),
                hitclass: Vec::new(),
            },
            levels: Vec::new(),
            skill_modes: Vec::new(),
            overlay_count: 0,
            monequip: Vec::new(),
            arena: Vec::new(),
        };
        let drlg = DrlgWorld {
            dungeon: Dungeon::default(),
            data: Arc::new(DrlgData::default()),
            tiles: Box::new(NoTiles),
            types: Box::new(NoLevelTypes),
        };
        let hooks = ActionHooks::new(
            Arc::new(tables),
            drlg,
            Seed::init_low(game_seed),
            ActionRest::default(),
        );
        let data = UnitData {
            monsters: vec![
                MonsterInfo {
                    mode_chart: false,
                    enabled: true,
                    aidel: [15; 3],
                    moves: 0,
                };
                N_MONSTATS
            ],
            ..UnitData::default()
        };
        let mut events = ActionSim::new(stat_data(), data, hooks);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let mut seed = events.hooks().game_seed;
        let ctl = NpcControl::new(&monstats(), Vec::new(), false, 0, &mut seed).expect("npc");
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        events.hooks().game_seed = seed;
        let mut alloc = |ty, class| {
            let req = AllocRequest {
                ty,
                class,
                room: None,
                add: true,
                fixed_guid: None,
                mode: 1,
                allied: ty == UnitType::Player,
            };
            events
                .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
                .expect("allocated")
        };
        let npc = alloc(UnitType::Monster, u32::from(class::AKARA));
        let player = alloc(UnitType::Player, 1);
        events.sys.units.get_mut(player).unwrap().mode = 1;
        events.with(&mut game, |_, v| {
            v.set_base(player, 12, 1);
            v.set_base(player, 14, 5000);
        });
        let mut rest = Rest::default();
        rest.quests.insert(player, PlayerQuests::default());
        let mut world = WiredWorld::new(
            ActionWorld::default(),
            item_tables(),
            quests,
            ctl,
            vendor_tables(),
            rest,
            1000,
        );
        world.state.add_npc(npc);
        let (buckler, cap) = world.with_economy(&mut game, &mut events, |econ, _| {
            let mut make = |record: usize| {
                let mut rq = ItemRequest {
                    item: record as i32,
                    ilvl: 1,
                    quality: 2,
                    format: 1,
                    ..ItemRequest::default()
                };
                let spawn = ItemSpawn {
                    room: None,
                    mode: 4,
                    init_flags: 1,
                };
                econ.create_item(&mut rq, false, spawn).expect("item")
            };
            (make(BUC), make(CAP))
        });
        let pg = events.sys.units.get(player).unwrap().guid;
        let items = [(*b"cap ", ty::HELM, 2, 2), (*b"buc ", ty::SHIE, 2, 2)];
        let tables = super::inv_tables(&items, N_TYPES, equiv());
        world.inventory = Some(super::inv_parts(tables, player, 1, pg));
        let mut s: TradeGame = SimGame::with_world(game, events, world);
        // The player's buckler and cap, stored (mode 0) in its inventory.
        super::store_item(&mut s, player, buckler, 0);
        super::store_item(&mut s, player, cap, 0);
        s.join(0, Some(player), None, client_state::IN_GAME)
            .unwrap();
        s.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        let at = UnitFacts {
            act: 0,
            pos: Pos { x: 100, y: 100 },
            owner: None,
        };
        s.set_unit(player, at);
        s.set_unit(npc, at);
        let guids = [player, npc, buckler, cap]
            .iter()
            .map(|&u| s.game.lists.unit(u).unwrap().guid)
            .collect();
        (s, guids)
    }
}

/// The NPC, vendor and quest ids `WiredWorld` handles.
fn trade_id() -> impl Strategy<Value = u8> {
    prop::sample::select(vec![
        0x13u8, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x40, 0x58, 0x62,
    ])
}

/// Talk (0x13), chat (0x2F) and trade (0x38 action 1) with Akara, so the
/// vendor paths past "no interaction" are reached.
fn open_trade(npc: u32) -> Vec<Vec<u8>> {
    let mut talk = vec![0x13, 1, 0, 0, 0];
    talk.extend_from_slice(&npc.to_le_bytes());
    let mut chat = vec![0x2F, 1, 0, 0, 0];
    chat.extend_from_slice(&npc.to_le_bytes());
    let mut trade = vec![0x38, 1, 0, 0, 0];
    trade.extend_from_slice(&npc.to_le_bytes());
    trade.extend_from_slice(&[0; 4]);
    vec![talk, chat, trade]
}

proptest! {
    #![proptest_config(config(64))]

    /// Every game id on the trade host, optionally after opening a trade
    /// with Akara; the NPC / vendor / quest ids get most of the cases.
    #[test]
    fn trade_host_any_intents(
        seed in any::<u32>(),
        open in any::<bool>(),
        ids in prop::collection::vec(prop_oneof![3 => trade_id(), 1 => 1u8..0x67], 1..12),
        gens in prop::collection::vec(gen(), 12),
    ) {
        let (mut sim, mut guids) = trade::trade_host(seed);
        if open {
            let mut out = ClientBuffers::new();
            out.add_client(0);
            for m in open_trade(guids[1]) {
                let code = dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &m, m.len());
                prop_assert_eq!(code, ResultCode::Done, "{:02X?}", m);
            }
            // The store items the trade generated join the GUID pool.
            let mut items = sim.game.lists.units_of_type(d2_sim::units::UnitType::Item);
            items.sort();
            for u in items {
                let g = sim.game.lists.unit(u).unwrap().guid;
                if !guids.contains(&g) {
                    guids.push(g);
                }
            }
        }
        let msgs: Vec<Gen> = ids.iter().zip(gens).map(|(&id, g)| Gen { id, ..g }).collect();
        run(&mut sim, &guids, &msgs)?;
    }
}

// ---- handlers' early refusals ----------------------------------------------------------
//
// The refusals each handler makes before any effect (its checks come before
// the first write): the result code is the handler's, and the game, the
// host's world state and the outgoing messages (the client's buffers, the
// seams' outboxes, the rests' call logs) are unchanged. Refusals that come
// after a write are left out (the cube's targeting reset before its later
// refusals; the NPC handlers' path and AI steps before the start check; the
// waypoint refusals with the waypoint as the interaction, which reset it; the
// point forms, whose validator stores the frame first).
// The item moves and walk are not on these hosts: their refusals are
// tested with the handler fixtures (`handlers/items/moves/tests.rs`,
// `handlers/walk/tests.rs`).

/// Everything a refusal could touch on a host, as text, and its
/// outgoing messages.
trait Digest {
    fn digest(&mut self) -> String;
    /// The seams' outboxes (messages queued for the next flush).
    fn queued(&mut self) -> Vec<(UnitId, Vec<u8>)>;
}

fn player_digest<D: EventDispatch, W>(sim: &SimGame<D, W>) -> String {
    let p = sim.player_of(0);
    format!(
        "{:?}{:?}{:?}{:?}",
        p.and_then(|p| sim.player_fields(p)),
        sim.resyncs,
        sim.unhandled,
        sim.tick_faults
    )
}

impl Digest for ActionGame {
    fn digest(&mut self) -> String {
        let h = &self.events.sys.hooks;
        let b = h.x.get();
        let book = format!(
            "{:?}{:?}{:?}{:?}{:?}",
            b.list, b.left, b.right, b.used, b.pos
        );
        drop(b);
        format!(
            "{}|{}|{:?}{:?}{:?}|{:?}{:?}|{}",
            self.snapshot(),
            book,
            h.game_seed,
            h.waypoints,
            h.mode_target,
            h.arrivals,
            self.world.faults,
            player_digest(self)
        )
    }
    fn queued(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        self.events.sys.hooks.x.get().sent.clone()
    }
}

impl Digest for ItemGame {
    fn digest(&mut self) -> String {
        let w = &self.world;
        let cube = w
            .cube
            .as_ref()
            .map(|c| format!("{:?}{:?}{:?}", c.staged, c.creation, c.errors));
        format!(
            "{}|{:?}{:?}{:?}{:?}{:?}{:?}|{:?}{:?}|{}",
            self.snapshot(),
            w.state,
            w.npc,
            w.quests,
            self.events.sys.hooks.uniques,
            w.rest,
            cube,
            self.events.sys.hooks.game_seed,
            self.world.action.faults,
            player_digest(self)
        )
    }
    fn queued(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        let mut q = self.events.sys.hooks.x.sent.clone();
        q.extend(self.world.rest.sent.iter().cloned());
        q
    }
}

/// Sends `m` through the dispatcher and checks the refusal: result
/// `code`, nothing changed, nothing sent or queued. `what` names the
/// refusal rule.
fn refused<D, W>(sim: &mut SimGame<D, W>, m: &[u8], code: ResultCode, what: &str)
where
    D: EventDispatch,
    W: d2_server::adapters::handlers::world::WorldHost<D>,
    SimGame<D, W>: Digest,
{
    let mut out = ClientBuffers::new();
    out.add_client(0);
    let before = (sim.digest(), sim.queued());
    let got = dispatch(sim, &ProtoSizes, &mut out, 0, ALIVE, m, m.len());
    assert_eq!(got, code, "{what}: {m:02X?}");
    assert_eq!(out.pop(0), None, "{what}: {m:02X?} sent a message");
    let after = (sim.digest(), sim.queued());
    assert!(
        before == after,
        "{what}: {m:02X?} changed the host:\n{before:?}\n{after:?}"
    );
}

fn msg(id: u8, fields: &[u32]) -> Vec<u8> {
    let mut m = vec![id];
    for f in fields {
        m.extend_from_slice(&f.to_le_bytes());
    }
    m
}

const NO_GUID: u32 = 0xDEAD;

/// Skills: the use, stat-point and skill-point refusals.
#[test]
fn skill_refusals_change_nothing() {
    let (mut sim, guids) = action_host(1);
    // 0x3C: no skill entry (99) → 3, before any write.
    refused(
        &mut sim,
        &msg(0x3C, &[99, u32::MAX]),
        ResultCode::Malformed,
        "use §7 entry",
    );
    // 0x3A: stat id > 15 → 3; a stat with no spend (4..15) → 2.
    refused(
        &mut sim,
        &[0x3A, 16, 0],
        ResultCode::Malformed,
        "vitals §2 stat id",
    );
    refused(
        &mut sim,
        &[0x3A, 4, 0],
        ResultCode::Invalid,
        "vitals §2 first spend",
    );
    // 0x3B: a skill id past the table → 2.
    refused(
        &mut sim,
        &[0x3B, 0xFF, 0x7F],
        ResultCode::Invalid,
        "levels §6.4 id",
    );
    // 0x06 / 0x0D (unit forms): no left / right skill → 3, before the
    // pierce index.
    {
        let mut b = sim.events.sys.hooks.x.get();
        b.left = None;
        b.right = None;
    }
    for id in [0x06, 0x07, 0x0D, 0x0E] {
        refused(
            &mut sim,
            &msg(id, &[1, guids[1]]),
            ResultCode::Malformed,
            "use §1 no skill",
        );
    }
}

/// Waypoint 0x49 with no interaction: the refusal's interaction reset does not
/// apply, so each refusal changes nothing.
#[test]
fn waypoint_refusals_change_nothing() {
    let (mut sim, guids) = action_host(1);
    refused(
        &mut sim,
        &msg(0x49, &[NO_GUID, 1]),
        ResultCode::Refused,
        "§6.2 step 1 missing",
    );
    refused(
        &mut sim,
        &msg(0x49, &[guids[3], 0xFFFF]),
        ResultCode::Malformed,
        "§6.2 step 5 level",
    );
}

/// The item unit of GUID `g`.
fn item_unit(sim: &ItemGame, g: u32) -> UnitId {
    sim.game.lists.find_unit(UnitType::Item, g).unwrap()
}

/// Opens the inventory desk once with no call, so its item data copies
/// (`InvDesk::sync_in`) hold the fixture's direct edits of the units and
/// the item store before a digest is taken: any later handler's desk
/// would copy them, which is not the handler's effect.
fn settle(sim: &mut ItemGame) {
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    world.with_economy(game, events, |econ, p| {
        let inv = p.inventory.as_deref_mut().expect("inventory parts");
        let _ = inv.desk(econ);
    });
}

/// The cube's 0x2A first two checks, before the targeting reset (the cube
/// carries flag 0x4, so a reset would show); 0x4F 0x17 / 0x18 with another
/// interaction.
#[test]
fn cube_refusals_change_nothing() {
    // The ring (mode 4) is the cursor item.
    let (mut sim, guids) = item_host(4);
    let (cube, ring) = (guids[1], guids[2]);
    let (cube_u, ring_u) = (item_unit(&sim, cube), item_unit(&sim, ring));
    sim.events.sys.hooks.items.get_mut(cube_u).unwrap().flags |= 0x4;
    settle(&mut sim);
    refused(
        &mut sim,
        &msg(0x2A, &[NO_GUID, cube]),
        ResultCode::Refused,
        "§2 step 1 missing",
    );
    refused(
        &mut sim,
        &msg(0x2A, &[ring, NO_GUID]),
        ResultCode::Refused,
        "§2 step 2 missing",
    );
    // A cursor-mode ring that is not the cursor item.
    let player = sim.player_of(0).unwrap();
    let set_cursor = |sim: &mut ItemGame, c| {
        let parts = sim.world.inventory.as_mut().unwrap();
        parts
            .state
            .inventories
            .get_mut(&player)
            .unwrap()
            .set_cursor(c);
    };
    set_cursor(&mut sim, None);
    refused(
        &mut sim,
        &msg(0x2A, &[ring, cube]),
        ResultCode::Refused,
        "§2 step 1 cursor",
    );
    set_cursor(&mut sim, Some(ring_u));
    {
        let r = sim.events.sys.units.get_mut(player).unwrap();
        r.interact.reset();
        r.interact.set(2, 77);
    }
    for b in [0x17u8, 0x18] {
        refused(
            &mut sim,
            &[0x4F, b, 0, 0, 0, 0, 0],
            ResultCode::Malformed,
            "§1 interaction",
        );
    }
}

/// NPC ids on the trade host.
#[test]
fn npc_refusals_change_nothing() {
    let (mut sim, guids) = trade::trade_host(1);
    let akara = guids[1];
    refused(
        &mut sim,
        &msg(0x13, &[1, NO_GUID]),
        ResultCode::Refused,
        "§2 missing",
    );
    refused(
        &mut sim,
        &msg(0x2F, &[1, NO_GUID]),
        ResultCode::Refused,
        "§3 missing",
    );
    refused(
        &mut sim,
        &msg(0x30, &[1, NO_GUID]),
        ResultCode::Refused,
        "§3 missing",
    );
    // 0x34 at the interact unit (after talk) whose class is not Cain:
    // 0, no message.
    let mut out = ClientBuffers::new();
    out.add_client(0);
    let talk = &open_trade(akara)[0];
    dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, talk, talk.len());
    sim.world.rest.sent.clear();
    sim.events.sys.hooks.x.sent.clear();
    refused(
        &mut sim,
        &msg(0x34, &[akara]),
        ResultCode::Done,
        "§6 step 2 not Cain",
    );
}

/// Vendor ids on the trade host after talk, chat and trade
/// (the vendor handlers' early refusals).
#[test]
fn vendor_refusals_change_nothing() {
    let (mut sim, guids) = trade::trade_host(1);
    let (akara, buckler) = (guids[1], guids[2]);
    let mut out = ClientBuffers::new();
    out.add_client(0);
    for m in open_trade(akara) {
        dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &m, m.len());
    }
    sim.world.rest.sent.clear();
    sim.events.sys.hooks.x.sent.clear();
    refused(
        &mut sim,
        &msg(0x33, &[akara, NO_GUID, 0, 1]),
        ResultCode::Refused,
        "§7.2 r1",
    );
    refused(
        &mut sim,
        &msg(0x37, &[NO_GUID]),
        ResultCode::Invalid,
        "§5.5 missing",
    );
    refused(
        &mut sim,
        &msg(0x37, &[buckler]),
        ResultCode::Malformed,
        "§5.5 not bought",
    );
}

/// Quest 0x58 with a quest id ≥ 0x2A → 2.
#[test]
fn quest_refusal_changes_nothing() {
    let (mut sim, _) = trade::trade_host(1);
    refused(
        &mut sim,
        &[0x58, 0x2A, 0],
        ResultCode::Invalid,
        "§1.7 quest id",
    );
}

/// Mutation guard: the check sees a refusal that changes something: the cube's
/// stored-item refusal (→ 3) comes after the targeting reset (flag 0x4
/// cleared), so `refused` must fail on it.
#[test]
fn refusal_check_sees_a_late_refusal() {
    let (mut sim, guids) = item_host(0);
    let player = sim.player_of(0).unwrap();
    let (cube_u, ring_u) = (item_unit(&sim, guids[1]), item_unit(&sim, guids[2]));
    store_item(&mut sim, player, ring_u, 0);
    sim.events.sys.units.get_mut(ring_u).unwrap().mode = 0;
    sim.events.sys.hooks.items.get_mut(cube_u).unwrap().flags |= 0x4;
    settle(&mut sim);
    let m = msg(0x2A, &[guids[2], guids[1]]);
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        refused(&mut sim, &m, ResultCode::Malformed, "§2 step 3.4")
    }));
    assert!(late.is_err(), "the targeting reset went unseen");
    let flags = sim.events.sys.hooks.items.get(cube_u).unwrap().flags;
    assert_eq!(flags & 0x4, 0, "the reset ran");
}
