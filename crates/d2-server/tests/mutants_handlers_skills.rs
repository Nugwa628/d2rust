//! Mutation-testing gaps of the skill handlers on the merged host
//! (`SimGame<ActionSim<Book>, ActionWorld<WiredSkills>>`): the server's
//! `skills::world::World` over the skill use pipeline's provider. Each
//! test sends a skill message through `SimGame::handle` (the handler
//! API, after the dispatcher's gate and parse) and asserts the outcome
//! the skill-use rules give: result code, mode, mana, used / left / right
//! skill, timers, states, the provider calls the rule names (run to the
//! target, consume charges, decrement quantity).
//!
//! The units stand in a real field room (the waypoint tests' DRLG).
//! What the host does not answer by itself is staged on [`Book`], the action
//! wiring's `Pending` value (`UseRest`, `LearnRest`).
//!
//! Integration test of `d2-server`; it guards the server's skill handlers and
//! shares the waypoint DRLG fixture style of the other `mutants_handlers_*` tests.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use d2_data::bin::BinTable;
use d2_data::fixup::maps::states as state_maps;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemstatcost, Record, Skilldesc, Skills, States};
use d2_server::adapters::handlers::skills::wired::WiredSkills;
use d2_server::adapters::handlers::skills::LearnRest;
use d2_server::adapters::handlers::world::{ActionWorld, Outbox};
use d2_server::adapters::{PlayerData, PlayerFields, SimGame, UnitFacts};
use d2_server::buffers::ClientBuffers;
use d2_server::seams::{Intents, PlayerGate, Pos, ResultCode};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind as DrlgRoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::skills::use_::{mode, state, ModeTarget, ServerMsg, UseState};
use d2_sim::skills::{SkillEntry, SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::{ClassStats, StatData, StatTable, StateTable};
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::interaction::UseRest;

use ResultCode::*;

// ---- tables ------------------------------------------------------------------------------

const N_STATS: usize = 359;
const N_STATES: usize = 200;

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost (no ops, no shifts) and 200 blank states.
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
        count: N_STATES,
        record_size: States::SIZE,
        records: vec![0u8; N_STATES * States::SIZE],
    };
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        states: StateTable::new(&states, &state_maps(&states)).expect("states"),
        damage_regen: vec![0; 8],
        aurastate: vec![0; 8],
        rescale_precision: d2_sim::stats::DEFAULT_RESCALE_PRECISION,
    })
}

/// A skills record with no formulas, no required skills, usable in town.
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

/// Attack.
const ATTACK: i32 = 0;
/// Multiple Shot: srvst [`START`]; mana 4, +1/level, shift 8.
const MULTI: i32 = 1;
/// An immediate aura with a do function (srvdo [`DO`]), mana 2 (shift 8),
/// `decquant`, delay = formula 0, aura state 33.
const MIGHT: i32 = 2;
/// A non-immediate aura, aura state 40.
const AURA: i32 = 3;
/// A start skill (srvst [`START`]) with `srvdofunc` 116 (free while shapeshifted).
const WERE: i32 = 4;
/// Left Hand Swing, the second hand of a dual wield.
const LHS: i32 = 5;
/// A melee (h2h) skill with a start function.
const MELEE: i32 = 6;
/// A "both" range skill.
const BOTH: i32 = 7;
/// A ranged skill.
const RANGED: i32 = 8;
/// `TargetableOnly`, srvst [`START`].
const TARGETED: i32 = 9;
/// `lineofsight` 4, srvst [`START`].
const LOS: i32 = 10;
/// A skill with `interrupt`.
const INTERRUPT: i32 = 11;

/// Formula 0: `stat(0, 0)`, CALL 5 (the unit's
/// stat 0, total) = strength.
const DELAY_STRENGTH: [u8; 6] = [0x07, 0, 0x07, 0, 0x01, 0x05];

/// Start slot of the synthetic start skills: srvst 53 (the Inferno monster
/// skill's start) stands in for Multiple Shot's srvst 4, whose body
/// (ammunition) needs items. Every filled start slot has a body, so no seam
/// call shows the start; srvst 53 sets the used entry's param 1 :=
/// frame + max(`calc2`, 1), and with `calc2` = `lvl` ([`LVL_CALC`]) the fake's
/// param log ([`started`]) shows the skill and the level the start ran with,
/// as a seam log would. The do slot 53 (filled, `unreferenced`, no body)
/// stands in for Might's do slot 65, so the fake's seam answers.
const START: u16 = 53;
/// The do stand-in (see [`START`]).
const DO: u16 = 53;
/// Formula offset of `lvl` (bytes `04 10 00`), after
/// [`DELAY_STRENGTH`] and its end byte.
const LVL_CALC: u32 = 7;

/// The fake's log line for a start of `skill` at level `lvl` at `frame`
/// (srvst 53's param 1 = frame + max(lvl, 1)).
fn started(frame: i32, skill: i32, lvl: i32) -> String {
    format!("param1 {skill} {}", frame + lvl.max(1))
}

fn skills() -> SkillTables {
    let mut v: Vec<Skills> = (0..12).map(|_| skill_rec()).collect();
    let m = &mut v[MULTI as usize];
    (m.srvstfunc, m.mana, m.lvlmana, m.manashift) = (START, 4, 1, 8);
    let m = &mut v[MIGHT as usize];
    (m.aura, m.immediate, m.srvdofunc, m.aurastate) = (true, true, DO, 33);
    (m.mana, m.manashift, m.decquant, m.delay) = (2, 8, true, 0);
    let m = &mut v[AURA as usize];
    (m.aura, m.aurastate) = (true, 40);
    let m = &mut v[WERE as usize];
    (m.srvstfunc, m.srvdofunc, m.mana, m.manashift) = (START, 116, 4, 8);
    for s in [MELEE, BOTH, RANGED] {
        v[s as usize].srvstfunc = START;
    }
    v[MELEE as usize].range = 1;
    v[BOTH as usize].range = 3;
    v[RANGED as usize].range = 2;
    let m = &mut v[TARGETED as usize];
    (m.srvstfunc, m.targetableonly) = (START, true);
    let m = &mut v[LOS as usize];
    (m.srvstfunc, m.lineofsight) = (START, 4);
    v[INTERRUPT as usize].interrupt = true;
    for s in [MULTI, WERE, MELEE, BOTH, RANGED, TARGETED, LOS] {
        v[s as usize].calc2 = LVL_CALC;
    }
    let mut code = DELAY_STRENGTH.to_vec();
    code.push(0x00);
    code.extend([0x04, 0x10, 0x00]);
    SkillTables {
        skills: v,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: Vec::new(),
        skills_code: code,
        miss_code: Vec::new(),
        level_cap: LEVEL_CAP_114D,
        stat_count: N_STATS as _,
    }
}

// ---- the field room ----------------------------------------------------------------------

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

/// Cold Plains, a field level.
const COLD_PLAINS: u32 = 3;

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
    data.levels[COLD_PLAINS as usize].drlg_type = 2;
    data.levels[COLD_PLAINS as usize].level_type = 1;
    let mut types = FieldTypes([(COLD_PLAINS, TileRect::new(0, 0, 8, 8))].into());
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
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

// ---- the staged provider -----------------------------------------------------------------

/// What the seams without a written provider answer, staged per test,
/// and a log of the calls that act.
#[derive(Default)]
struct Inner {
    list: Vec<SkillEntry>,
    left: Option<SkillEntry>,
    right: Option<SkillEntry>,
    used: Option<SkillEntry>,
    /// Entry mode by skill (skill entry +8); default SC.
    modes: BTreeMap<i32, u32>,
    /// `use_state` by skill (default usable).
    states: BTreeMap<i32, UseState>,
    pos: BTreeMap<UnitId, (i32, i32)>,
    cursor: bool,
    dual: bool,
    /// Items at body locations.
    body: BTreeMap<u8, UnitId>,
    equippable: BTreeSet<UnitId>,
    /// Item types each item is.
    types: BTreeMap<UnitId, Vec<i32>>,
    param4: i32,
    shapeshifted: bool,
    target: Option<UnitId>,
    hostile: bool,
    pet: bool,
    ally: bool,
    melee: bool,
    bow: bool,
    mask: bool,
    owner: BTreeMap<UnitId, UnitId>,
    target_pos: Option<(i32, i32)>,
    /// `line_clear` is true only towards this point with this mask.
    clear_line: Option<((i32, i32), u32)>,
    event_arg: i32,
    log: Vec<String>,
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
    fn skill_list(&self, _: UnitId) -> Vec<SkillEntry> {
        self.get().list.clone()
    }
    fn used_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.get().used
    }
    fn item_at(&self, _: UnitId, loc: u8) -> Option<UnitId> {
        self.get().body.get(&loc).copied()
    }
    fn item_is(&self, item: UnitId, itype: i32) -> bool {
        self.get()
            .types
            .get(&item)
            .is_some_and(|t| t.contains(&itype))
    }
    fn may_attack(&self, _: UnitId, _: UnitId) -> bool {
        self.get().hostile
    }
    fn in_melee_range(&self, _: UnitId, _: UnitId, _: i32) -> bool {
        self.get().melee
    }
    /// srvst 53 ([`START`]) writes param 1 of the used entry: logged.
    fn set_entry_param_of(&mut self, _: UnitId, e: &SkillEntry, i: u8, v: i32) {
        self.get().log.push(format!("param{i} {} {v}", e.skill));
    }
}

impl Outbox for Book {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        Vec::new()
    }
}

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
        self.get().cursor
    }
    fn in_own_inventory(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn within_reach(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn owner(&self, u: UnitId) -> Option<UnitId> {
        self.get().owner.get(&u).copied()
    }
    fn is_pet(&self, _: UnitId, _: UnitId) -> bool {
        self.get().pet
    }
    fn is_ally(&self, _: UnitId, _: UnitId) -> bool {
        self.get().ally
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
    fn entry_mode(&self, _: UnitId, e: &SkillEntry) -> u32 {
        self.get().modes.get(&e.skill).copied().unwrap_or(mode::SC)
    }
    fn attack_param4(&self, _: UnitId) -> i32 {
        self.get().param4
    }
    fn set_attack_param4(&mut self, _: UnitId, v: i32) {
        self.get().param4 = v;
    }
    fn use_state(&mut self, _: UnitId, e: &SkillEntry) -> UseState {
        self.get()
            .states
            .get(&e.skill)
            .copied()
            .unwrap_or(UseState::Usable)
    }
    fn shapeshifted(&self, _: UnitId) -> bool {
        self.get().shapeshifted
    }
    fn consume_charges(&mut self, _: UnitId, e: &SkillEntry) -> bool {
        self.get().log.push(format!("charges {}", e.skill));
        true
    }
    fn pay_life(&mut self, _: UnitId, cost: i32) -> bool {
        self.get().log.push(format!("life {cost}"));
        true
    }
    fn can_dual_wield(&self, _: UnitId) -> bool {
        self.get().dual
    }
    fn equippable(&self, item: UnitId) -> bool {
        self.get().equippable.contains(&item)
    }
    fn bow_equipped(&self, _: UnitId) -> bool {
        self.get().bow
    }
    fn state_mask(&self, _: UnitId, mask: u32) -> bool {
        self.get().mask && mask == 0x26
    }
    fn start_mode(&mut self, _: &mut Game, _: UnitId, _: u32, _: ModeTarget<UnitId>) {}
    fn run_to(&mut self, _: UnitId, target: UnitId, e: SkillEntry) {
        self.get().log.push(format!("run {} {}", target.0, e.skill));
    }
    fn target(&self, _: UnitId) -> Option<UnitId> {
        self.get().target
    }
    fn clear_target(&mut self, _: UnitId) {
        self.get().target = None;
    }
    fn event_arg(&self, _: UnitId) -> i32 {
        self.get().event_arg
    }
    fn set_event_arg(&mut self, _: UnitId, _: i32) {}
    fn step_path(&mut self, _: UnitId) -> i32 {
        0
    }
    fn target_position(&self, _: UnitId) -> Option<(i32, i32)> {
        self.get().target_pos
    }
    fn line_clear(&self, _: UnitId, to: (i32, i32), mask: u32) -> bool {
        self.get().clear_line == Some((to, mask))
    }
    fn set_aura_state(&mut self, _: UnitId, state: u16, skill: i32, lvl: i32) {
        self.get()
            .log
            .push(format!("aura on {state} {skill} {lvl}"));
    }
    fn srvst(&mut self, index: u16, _: UnitId, skill: i32, lvl: i32) -> i32 {
        self.get().log.push(format!("srvst {index} {skill} {lvl}"));
        1
    }
    fn srvdo(&mut self, i: u16, _: UnitId, s: i32, l: i32, _: bool, _: bool, _: bool) -> i32 {
        self.get().log.push(format!("srvdo {i} {s} {l}"));
        1
    }
}

impl LearnRest for Book {
    fn is_class_skill(&self, _: UnitId, _: i32) -> bool {
        false
    }
    fn add_skill_level(&mut self, _: UnitId, _: i32, _: i32) {}
    fn after_skill_point(&mut self, _: UnitId) {}
}

// ---- the host ----------------------------------------------------------------------------

type Wired = SimGame<ActionSim<Book>, ActionWorld<WiredSkills>>;

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

/// A player (class `class`, mode NU) for client 0 at (100, 100), a
/// monster at (120, 100) and two items, in one field room, at frame
/// 1000; the provider's positions agree with the staged facts.
struct Fx {
    sim: Wired,
    player: UnitId,
    monster: UnitId,
    item: UnitId,
    item2: UnitId,
    book: Book,
}

fn entry(skill: i32, base: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

impl Fx {
    fn new(class: u32) -> Self {
        let book = Book::default();
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
        let hooks = ActionHooks::new(
            Arc::new(tables),
            field_drlg(),
            Seed::init_low(1234),
            book.clone(),
        );
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
        let item2 = alloc(UnitType::Item, 0);
        events.sys.units.get_mut(player).unwrap().mode = mode::NU;
        game.frame = 1000;
        let world = ActionWorld {
            skills: WiredSkills::default(),
            ..ActionWorld::default()
        };
        let mut sim: Wired = SimGame::with_world(game, events, world);
        sim.join(0, Some(player), Some(room), client_state::IN_GAME)
            .unwrap();
        sim.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        let mut fx = Fx {
            sim,
            player,
            monster,
            item,
            item2,
            book,
        };
        fx.place(player, 0, 100, 100, None);
        fx.place(monster, 0, 120, 100, None);
        fx.place(item, 0, 101, 100, None);
        fx.place(item2, 0, 102, 100, None);
        // Mana for every start (stat 8).
        fx.set_stats(player, &[(8, 100_000)]);
        fx
    }

    /// Stages a unit's facts (act, position, owner) and the provider's
    /// position.
    fn place(&mut self, u: UnitId, act: u8, x: i32, y: i32, owner: Option<UnitId>) {
        self.sim.set_unit(
            u,
            UnitFacts {
                act,
                pos: Pos { x, y },
                owner,
            },
        );
        self.book.get().pos.insert(u, (x, y));
    }

    fn book(&self) -> MutexGuard<'_, Inner> {
        self.book.get()
    }

    fn guid(&self, u: UnitId) -> u32 {
        self.sim.game.lists.unit(u).unwrap().guid
    }

    /// `SimGame::handle` for client 0 (the dispatcher's parse not run).
    fn handle(&mut self, msg: &[u8]) -> ResultCode {
        let mut out = ClientBuffers::new();
        out.add_client(0);
        Intents::handle(&mut self.sim, 0, msg, msg.len(), &mut out)
    }

    fn stat(&mut self, s: u16) -> i32 {
        let p = self.player;
        let g = &mut self.sim;
        g.events.with(&mut g.game, |_, v| v.stat(p, s))
    }

    fn set_stats(&mut self, u: UnitId, values: &[(u16, i32)]) {
        let g = &mut self.sim;
        g.events.with(&mut g.game, |_, v| {
            for &(s, x) in values {
                v.set_base(u, s, x);
            }
        });
    }

    fn set_state(&mut self, u: UnitId, s: u16) {
        let g = &mut self.sim;
        g.events.with(&mut g.game, |_, v| v.set_state(u, s, true));
    }

    fn has_state(&mut self, u: UnitId, s: u16) -> bool {
        let g = &mut self.sim;
        g.events
            .with(&mut g.game, |_, v| v.stats.has_state(u, u32::from(s)))
    }

    fn mode(&self) -> u32 {
        self.sim.events.sys.units.get(self.player).unwrap().mode
    }

    fn set_mode(&mut self, u: UnitId, m: u32) {
        self.sim.events.sys.units.get_mut(u).unwrap().mode = m;
    }

    fn last_accept(&self) -> i32 {
        let data = self.sim.player_fields(self.player).unwrap().data;
        data.unwrap().last_accept
    }

    fn log(&self) -> Vec<String> {
        std::mem::take(&mut self.book().log)
    }

    /// The player's timers: (event, expire).
    fn timers(&self) -> Vec<((u8, u32, u32), i32)> {
        let g = &self.sim.game;
        g.timers
            .unit_timers(self.player)
            .into_iter()
            .filter_map(|t| Some((g.timers.event(t)?, g.timers.expire(t)?)))
            .collect()
    }

    /// Stages `skill` as the left and right skill (entry level 10).
    fn with_skill(&mut self, skill: i32) {
        let e = entry(skill, 10);
        let mut b = self.book();
        b.list = vec![entry(ATTACK, 1), e];
        b.left = Some(e);
        b.right = Some(e);
    }
}

fn point(id: u8, x: u16, y: u16) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(x.to_le_bytes());
    m.extend(y.to_le_bytes());
    m
}

fn unit_msg(id: u8, ty: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(ty.to_le_bytes());
    m.extend(guid.to_le_bytes());
    m
}

fn select(skill: i32, left: bool) -> Vec<u8> {
    let mut m = vec![0x3C];
    m.extend((skill as u32 | if left { 0x8000_0000 } else { 0 }).to_le_bytes());
    m.extend((-1i32).to_le_bytes());
    m
}

// ---- the handler's own validators ----------------------------------------------------------

/// The point validator in the handler: no player
/// data → 2; out of range → 1, with a resync only when more than 25
/// frames passed since the last accepted frame; in range → it := frame.
#[test]
fn point_validator_in_the_handler() {
    let mut fx = Fx::new(0);
    fx.with_skill(MULTI);
    let p = fx.player;
    fx.sim.set_player(
        p,
        PlayerFields {
            gate: ALIVE,
            data: None,
        },
    );
    assert_eq!(fx.handle(&point(0x0C, 110, 90)), Invalid);
    assert!(fx.sim.resyncs.is_empty());

    let data = |last_accept| PlayerFields {
        gate: ALIVE,
        data: Some(PlayerData { last_accept }),
    };
    fx.sim.set_player(p, data(975));
    assert_eq!(fx.handle(&point(0x0C, 151, 100)), Refused);
    assert!(fx.sim.resyncs.is_empty(), "25 frames: no resync");
    fx.sim.set_player(p, data(974));
    assert_eq!(fx.handle(&point(0x0C, 100, 151)), Refused);
    assert_eq!(fx.sim.resyncs, vec![0], "26 frames: resync");
    assert_eq!(fx.last_accept(), 974);

    fx.sim.game.frame = 1234;
    assert_eq!(fx.handle(&point(0x0C, 110, 90)), Done);
    assert_eq!(fx.last_accept(), 1234);
}

/// The unit validator in the handler: an owned
/// item passes whatever its distance; another act → 2; out of reach → 1.
#[test]
fn unit_validator_in_the_handler() {
    let mut fx = Fx::new(0);
    fx.with_skill(MULTI);
    let (p, m, i) = (fx.player, fx.monster, fx.item);
    // Owned, and far on the provider's path.
    fx.place(i, 0, 101, 100, Some(p));
    fx.book().pos.insert(i, (1000, 1000));
    assert_eq!(fx.handle(&unit_msg(0x0E, 4, fx.guid(i))), Done);
    // Not owned, in another act.
    fx.place(i, 1, 101, 100, None);
    assert_eq!(fx.handle(&unit_msg(0x0E, 4, fx.guid(i))), Invalid);
    fx.place(m, 1, 120, 100, None);
    assert_eq!(fx.handle(&unit_msg(0x0E, 1, fx.guid(m))), Invalid);
    // Same act, 51 subtiles away.
    fx.place(m, 0, 151, 100, None);
    assert_eq!(fx.handle(&unit_msg(0x0E, 1, fx.guid(m))), Refused);
    assert!(fx.log().is_empty(), "no use attempt");
    fx.place(m, 0, 150, 100, None);
    assert_eq!(fx.handle(&unit_msg(0x0E, 1, fx.guid(m))), Done);
    assert_eq!(fx.log(), [started(fx.sim.game.frame, 1, 10)]);
}

// ---- dual wield ---------------------------------------------------------------------------

/// Attack with two weapons (type 45, not 38) in bodylocs 4 and 5 and a
/// unit that can dual-wield: Param4 0 → 5, Left Hand Swing; Param4 5 →
/// 0, Attack. Any condition missing → Attack, Param4 kept.
#[test]
fn dual_wield_alternates_hands() {
    let mut fx = Fx::new(6);
    let (a, b) = (fx.item, fx.item2);
    {
        let mut bk = fx.book();
        bk.list = vec![entry(ATTACK, 1), entry(LHS, 1)];
        bk.left = Some(entry(ATTACK, 1));
        bk.dual = true;
        bk.body = [(4, a), (5, b)].into();
        bk.equippable = [a, b].into();
        bk.types = [(a, vec![45]), (b, vec![45])].into();
    }
    let used = |fx: &mut Fx| {
        let r = fx.handle(&point(0x05, 110, 90));
        assert_eq!(r, Done);
        fx.set_mode(fx.player, mode::NU);
        let b = fx.book();
        (b.used.map(|e| e.skill), b.param4)
    };
    assert_eq!(used(&mut fx), (Some(LHS), 5));
    assert_eq!(used(&mut fx), (Some(ATTACK), 0));
    assert_eq!(used(&mut fx), (Some(LHS), 5));
    fx.book().param4 = 0;

    // Each missing condition: Attack, Param4 untouched.
    let cases: [fn(&mut Inner, UnitId); 5] = [
        |b, _| b.dual = false,
        |b, _| {
            b.body.remove(&5);
        },
        |b, i| {
            b.equippable.remove(&i);
        },
        |b, i| {
            b.types.insert(i, vec![45, 38]);
        },
        |b, i| {
            b.types.insert(i, vec![]);
        },
    ];
    for (n, f) in cases.iter().enumerate() {
        let saved = {
            let bk = fx.book();
            (
                bk.dual,
                bk.body.clone(),
                bk.equippable.clone(),
                bk.types.clone(),
            )
        };
        f(&mut fx.book(), b);
        assert_eq!(used(&mut fx), (Some(ATTACK), 0), "case {n}");
        let mut bk = fx.book();
        (bk.dual, bk.body, bk.equippable, bk.types) = saved;
    }
}

// ---- use on a unit --------------------------------------------------------------------------

/// With run allowed (0x0D): h2h in melee range → use now; out
/// of range → run to the target (no mode change); "both" is rng with a
/// bow, else h2h; rng with state mask 0x26 is h2h.
#[test]
fn run_or_use_by_range() {
    let mut fx = Fx::new(0);
    let m = fx.monster;
    let g = fx.guid(m);
    let run = format!("run {} ", m.0);
    let go = |fx: &mut Fx, skill: i32, setup: fn(&mut Inner)| {
        fx.with_skill(skill);
        setup(&mut fx.book());
        fx.set_mode(fx.player, mode::NU);
        assert_eq!(fx.handle(&unit_msg(0x0D, 1, g)), Done);
        let log = fx.log();
        let mut b = fx.book();
        (b.melee, b.bow, b.mask) = (false, false, false);
        log
    };
    assert_eq!(
        go(&mut fx, MELEE, |b| b.melee = true),
        [started(fx.sim.game.frame, 6, 10)]
    );
    assert_eq!(go(&mut fx, MELEE, |_| {}), [format!("{run}{MELEE}")]);
    assert_eq!(
        go(&mut fx, BOTH, |b| b.bow = true),
        [started(fx.sim.game.frame, 7, 10)]
    );
    assert_eq!(go(&mut fx, BOTH, |_| {}), [format!("{run}{BOTH}")]);
    assert_eq!(
        go(&mut fx, RANGED, |_| {}),
        [started(fx.sim.game.frame, 8, 10)]
    );
    assert_eq!(
        go(&mut fx, RANGED, |b| b.mask = true),
        [format!("{run}{RANGED}")]
    );
    // Shift (0x0E): no run, whatever the range.
    fx.with_skill(MELEE);
    assert_eq!(fx.handle(&unit_msg(0x0E, 1, g)), Done);
    assert_eq!(fx.log(), [started(fx.sim.game.frame, 6, 10)]);
}

/// A target with state 143 (`attached`) and an owner → the
/// owner is the target (here: the one the unit runs to).
#[test]
fn attached_target_is_its_owner() {
    let mut fx = Fx::new(0);
    fx.with_skill(MELEE);
    let (m, owner) = (fx.monster, fx.item2);
    fx.book().owner.insert(m, owner);
    let g = fx.guid(m);
    assert_eq!(fx.handle(&unit_msg(0x0D, 1, g)), Done);
    assert_eq!(fx.log(), [format!("run {} {MELEE}", m.0)]);
    fx.set_state(m, state::ATTACHED);
    assert_eq!(fx.handle(&unit_msg(0x0D, 1, g)), Done);
    assert_eq!(fx.log(), [format!("run {} {MELEE}", owner.0)]);
}

// ---- mode change gates --------------------------------------------------------------------

/// `can_change_mode`: a cursor item blocks; A1 only up to `E + 5` (E =
/// the smallest positive type-1 expire); S1 not for an Amazon.
#[test]
fn can_change_mode_gates() {
    let mut fx = Fx::new(0);
    fx.with_skill(MULTI);
    let p = fx.player;
    fx.book().cursor = true;
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!((fx.mode(), fx.log().len()), (mode::NU, 0), "cursor");
    fx.book().cursor = false;

    // A1 with ENDANIM timers expiring at 990 and 996: E = 990.
    fx.set_mode(p, mode::A1);
    fx.sim.game.frame = 900;
    for f in [996, 990] {
        fx.sim.game.schedule_event(p, 1, f, None, 0, 0).unwrap();
    }
    fx.sim.game.frame = 996;
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!((fx.mode(), fx.log().len()), (mode::A1, 0), "996 > 995");
    fx.sim.game.frame = 995;
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!(fx.mode(), mode::SC, "995 ≤ 990 + 5");
    assert_eq!(fx.log(), [started(fx.sim.game.frame, 1, 10)]);

    // S1: an Amazon (class 0) cannot leave it, a Paladin (3) can (no
    // used skill: the interrupt gate passes).
    fx.book().used = None;
    fx.set_mode(p, mode::S1);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!((fx.mode(), fx.log().len()), (mode::S1, 0));
    let mut pal = Fx::new(3);
    pal.with_skill(MULTI);
    pal.set_mode(pal.player, mode::S1);
    pal.handle(&point(0x0C, 110, 90));
    assert_eq!(pal.mode(), mode::SC);
}

/// `interrupt_gate` step 4: with state 42 on, `roll(100)` is drawn from
/// the unit's seed and compared with stat 164 of state 42's list (no
/// list: not blocked); state 15 blocks; a blocked unit not in NU keeps
/// its mode.
#[test]
fn interrupt_gate_draws_and_blocks() {
    let mut fx = Fx::new(3);
    fx.with_skill(MULTI);
    let p = fx.player;
    fx.book().used = Some(entry(INTERRUPT, 1));
    fx.set_state(p, state::CONCENTRATION);
    let mut expect = fx.sim.events.sys.units.get(p).unwrap().seed;
    expect.roll(100);
    fx.set_mode(p, mode::S1);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!(fx.mode(), mode::SC);
    assert_eq!(fx.sim.events.sys.units.get(p).unwrap().seed, expect);

    // State 42's list holds stat 164 = 100: every roll is below it.
    let mut fx = Fx::new(3);
    fx.with_skill(MULTI);
    let p = fx.player;
    fx.book().used = Some(entry(INTERRUPT, 1));
    {
        let g = &mut fx.sim;
        let guid = g.game.lists.unit(p).unwrap().guid;
        g.events.with(&mut g.game, |_, v| {
            let l = v.stats.alloc(2, 0, 0, guid);
            v.stats.set_state(l, u32::from(state::CONCENTRATION));
            v.stats.attach(&mut *v.h, p, l, true);
            v.set_list_stat(l, 164, 100);
            v.set_state(p, state::CONCENTRATION, true);
        });
    }
    fx.set_mode(p, mode::S1);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!((fx.mode(), fx.log().len()), (mode::S1, 0), "blocked");
    // Stat 164 = 1: blocked only by a roll of 0; this seed's roll is not.
    let mut fx2 = Fx::new(3);
    fx2.with_skill(MULTI);
    let p2 = fx2.player;
    fx2.book().used = Some(entry(INTERRUPT, 1));
    let mut probe = fx2.sim.events.sys.units.get(p2).unwrap().seed;
    assert!(probe.roll(100) > 0);
    {
        let g = &mut fx2.sim;
        let guid = g.game.lists.unit(p2).unwrap().guid;
        g.events.with(&mut g.game, |_, v| {
            let l = v.stats.alloc(2, 0, 0, guid);
            v.stats.set_state(l, u32::from(state::CONCENTRATION));
            v.stats.attach(&mut *v.h, p2, l, true);
            v.set_list_stat(l, 164, 1);
            v.set_state(p2, state::CONCENTRATION, true);
        });
    }
    fx2.set_mode(p2, mode::S1);
    fx2.handle(&point(0x0C, 110, 90));
    assert_eq!(fx2.mode(), mode::SC, "not blocked");

    let mut fx = Fx::new(3);
    fx.with_skill(MULTI);
    let p = fx.player;
    fx.book().used = Some(entry(INTERRUPT, 1));
    fx.set_state(p, state::CONCENTRATE);
    fx.set_mode(p, mode::S1);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!((fx.mode(), fx.log().len()), (mode::S1, 0));
}

/// The mode start clears unit flag 0x40 and keeps the
/// others.
#[test]
fn mode_start_clears_flag_0x40() {
    let flags_after = |before: u32| {
        let mut fx = Fx::new(0);
        fx.with_skill(MULTI);
        let p = fx.player;
        fx.sim.events.sys.units.get_mut(p).unwrap().flags = before;
        fx.handle(&point(0x0C, 110, 90));
        assert_eq!(fx.mode(), mode::SC);
        fx.sim.events.sys.units.get(p).unwrap().flags
    };
    let (with, without) = (flags_after(0x42), flags_after(0x02));
    assert_eq!(with & 0x42, 0x02);
    assert_eq!(with, without);
}

// ---- start ---------------------------------------------------------------------------------

/// `lineofsight` 4 tests the line to the target position with
/// mask 0x804; a blocked line → 0 (neutral); no position skips the test
/// (passes).
#[test]
fn start_line_of_sight() {
    let mut fx = Fx::new(0);
    let go = |fx: &mut Fx, pos: Option<(i32, i32)>, clear: Option<((i32, i32), u32)>| {
        fx.with_skill(LOS);
        {
            let mut b = fx.book();
            (b.target_pos, b.clear_line) = (pos, clear);
        }
        fx.set_mode(fx.player, mode::NU);
        fx.handle(&point(0x0C, 110, 90));
        (fx.mode(), fx.log().len())
    };
    let at = (37, 41);
    assert_eq!(go(&mut fx, Some(at), Some((at, 0x804))), (mode::SC, 1));
    assert_eq!(go(&mut fx, None, Some((at, 0x804))), (mode::SC, 1));
    // A failed start outside town: NU (REC-1644).
    assert_eq!(go(&mut fx, Some(at), Some((at, 0x805))), (mode::NU, 0));
}

/// The mana check and the charge at start.
/// `srvdofunc` 116 is free while shapeshifted; the shrine state (134)
/// adds 2 levels to the cost; an item skill pays with charges; blood
/// mana (state 114) pays with life.
#[test]
fn start_mana() {
    let mut fx = Fx::new(0);
    let p = fx.player;
    fx.with_skill(WERE);
    fx.set_stats(p, &[(8, 0)]);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!((fx.mode(), fx.log().len()), (mode::NU, 0), "no mana");
    fx.book().shapeshifted = true;
    fx.set_mode(p, mode::NU);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!(fx.mode(), mode::SC, "free while shapeshifted");
    assert_eq!(fx.log(), [started(fx.sim.game.frame, 4, 10)]);

    // Multiple Shot L10: (4 + 9) << 8 = 3,328; with the shrine L12: 3,840.
    let mut fx = Fx::new(0);
    let p = fx.player;
    fx.with_skill(MULTI);
    fx.set_stats(p, &[(8, 4000)]);
    fx.set_state(p, 134);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!(fx.stat(8), 4000 - 3840);
    assert_eq!(fx.log(), [started(fx.sim.game.frame, 1, 12)]);

    // Blood mana: the cost is paid with life.
    let mut fx = Fx::new(0);
    let p = fx.player;
    fx.with_skill(MULTI);
    fx.set_stats(p, &[(8, 4000), (6, 4000)]);
    fx.set_state(p, 114);
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!(
        fx.log(),
        [started(fx.sim.game.frame, 1, 10), "life 3328".to_string()]
    );

    // An item skill (owner GUID ≠ −1) with charges.
    let mut fx = Fx::new(0);
    let e = SkillEntry {
        owner_guid: 77,
        charges: 3,
        ..entry(MULTI, 10)
    };
    {
        let mut b = fx.book();
        b.list = vec![e];
        b.right = Some(e);
    }
    fx.handle(&point(0x0C, 110, 90));
    assert_eq!(
        fx.log(),
        [started(fx.sim.game.frame, 1, 10), "charges 1".to_string()]
    );
}

// ---- do, cooldown, auras (through 0x3C) ------------------------------------------------------

/// 0x3C on an immediate aura runs the do core once: the do
/// function; mana at do (no start function); `decquant`; the delay
/// (formula: strength) → state 121 with a list and a type-12 timer at
/// frame + delay; the aura-form type-8 timer replaces the old one.
#[test]
fn immediate_aura_do_and_cooldown() {
    let mut fx = Fx::new(0);
    let p = fx.player;
    fx.book().list = vec![entry(MIGHT, 1)];
    fx.set_stats(p, &[(0, 30), (8, 1000)]);
    assert_eq!(fx.handle(&select(MIGHT, false)), Done);
    // `decquant`: the quantity body runs on the
    // wired host; with no stack in hand it changes nothing.
    assert_eq!(fx.log(), [format!("srvdo {DO} 2 1")]);
    assert_eq!(fx.stat(8), 1000 - 512);
    assert!(fx.has_state(p, state::SKILL_DELAY));
    let mut t = fx.timers();
    t.sort();
    assert_eq!(t, [((8, u32::MAX, 0), 1001), ((12, 0, 0), 1030)]);

    // Again 10 frames later: the list's expiry moves, one more type-12
    // timer, one type-8 timer.
    fx.sim.game.frame = 1010;
    assert_eq!(fx.handle(&select(MIGHT, false)), Done);
    let mut t = fx.timers();
    t.sort();
    assert_eq!(
        t,
        [
            ((8, u32::MAX, 0), 1011),
            ((12, 0, 0), 1030),
            ((12, 0, 0), 1040)
        ]
    );
    let expiry = {
        let g = &mut fx.sim;
        g.events.with(&mut g.game, |_, v| {
            let l = v.state_list(p, state::SKILL_DELAY).unwrap();
            v.stats.expire(l)
        })
    };
    assert_eq!(expiry, 1040);
}

/// In mode SQ the delay is set only when the frame-event word's bits 8+
/// (the frame event argument) are 0.
#[test]
fn sequence_mode_delay_needs_event_arg_0() {
    for (arg, delayed) in [(1, false), (0, true)] {
        let mut fx = Fx::new(0);
        let p = fx.player;
        fx.book().list = vec![entry(MIGHT, 1)];
        fx.book().event_arg = arg;
        fx.set_stats(p, &[(0, 30), (8, 1000)]);
        fx.set_mode(p, mode::SQ);
        assert_eq!(fx.handle(&select(MIGHT, false)), Done);
        assert_eq!(fx.has_state(p, state::SKILL_DELAY), delayed, "arg {arg}");
    }
}

/// A dead unit's do charges nothing (only a living unit is charged).
#[test]
fn dead_unit_do_charges_nothing() {
    let mut fx = Fx::new(0);
    let p = fx.player;
    fx.book().list = vec![entry(MIGHT, 1)];
    fx.set_stats(p, &[(0, 30), (8, 1000)]);
    fx.set_mode(p, mode::DD);
    assert_eq!(fx.handle(&select(MIGHT, false)), Done);
    assert_eq!(fx.log(), [format!("srvdo {DO} 2 1")]);
    assert_eq!(fx.stat(8), 1000);
}

/// A non-immediate aura switches its aura state on; replacing an
/// aura right skill frees its state (off) and its type-8 (−1) timers.
#[test]
fn replacing_an_aura() {
    let mut fx = Fx::new(0);
    let p = fx.player;
    fx.book().list = vec![entry(AURA, 1), entry(MULTI, 1)];
    assert_eq!(fx.handle(&select(AURA, false)), Done);
    assert_eq!(fx.log(), ["aura on 40 3 1"]);
    assert_eq!(fx.timers(), [((8, u32::MAX, 0), 1001)]);
    fx.set_state(p, 40);
    assert_eq!(fx.handle(&select(MULTI, false)), Done);
    assert!(!fx.has_state(p, 40));
    assert_eq!(fx.timers(), []);
    assert_eq!(fx.book().right.map(|e| e.skill), Some(MULTI));
}

// ---- the skill slot ----------------------------------------------------------------------

/// Without the vitals tables only 0x3A stays a stub; the other skill ids
/// still run (`pierce_idx` + 1).
#[test]
fn no_vitals_stubs_only_add_stat_point() {
    let mut fx = Fx::new(0);
    fx.with_skill(MULTI);
    assert!(fx.sim.events.sys.hooks.vitals.is_none());
    assert_eq!(fx.handle(&[0x3A, 0, 0]), Done);
    assert_eq!(fx.sim.unhandled, vec![(0, 0x3A, 3)]);
    assert_eq!(fx.handle(&point(0x0C, 110, 90)), Done);
    assert_eq!(fx.stat(328), 1);
    assert_eq!(fx.sim.unhandled.len(), 1);
}

/// A left-hand message without a left skill → 3, no use, no
/// `pierce_idx`.
#[test]
fn left_message_without_left_skill() {
    let mut fx = Fx::new(0);
    fx.with_skill(MULTI);
    fx.book().left = None;
    assert_eq!(fx.handle(&point(0x05, 110, 90)), Malformed);
    assert_eq!((fx.stat(328), fx.mode()), (0, mode::NU));
    assert!(fx.log().is_empty());
}
