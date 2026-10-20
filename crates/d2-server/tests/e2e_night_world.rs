//! Integration test of the night's world features on the wired single-player
//! host, end to end
//! through `d2_server::host::Host` (no bridge): `SimGame` on `WorldSim`
//! (worldgen around `ActionSim`, the path provider on before any unit,
//! live room population) with the `WiredWorld` host (waypoints, the
//! skill handlers, the NPC / vendor / quest systems), on the synthetic
//! act-0 world the e2e tests share (`d2-client/tests/e2e_support`,
//! declared here by path): the ISLE preset level (40 × 18 tiles at
//! (8000, 8000)) with a waypoint, the GATE level it travels to.
//!
//! Flows, intents in and S→C bytes and sim state out:
//!
//! 1. hiring a mercenary at Greiz (C→S 0x13 to the NPC, then 0x36);
//! 2. the hireling following the player through a waypoint teleport
//!    (C→S 0x49);
//! 3. a monster placed by live room population, walked to and killed with the right skill's
//!    missile; its death drop; the 0x69 death messages.
//!
//! The seams the game does not implement yet are the shared fixture's
//! (`e2e_support::Rest`, `e2e_world::TestPending`: staged answers and
//! call logs, never behaviour); where a flow reaches one, the test says
//! so.

use std::sync::{Arc, Mutex, MutexGuard};

use d2_data::tables::{Charstats, Difficultylevels, Levels, Monlvl, Monstats, Monstats2};
use d2_proto::client::{RightSkill, TakeOrCloseWp, Walk};
use d2_proto::FixedMessage;
use d2_server::adapters::handlers::skills::wired::WiredSkills;
use d2_server::adapters::handlers::skills::LearnRest;
use d2_server::adapters::handlers::walk::enable_paths;
use d2_server::adapters::handlers::world::Outbox;
use d2_server::adapters::handlers::world::{ActionWorld, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::dispatch::Outcome;
use d2_server::host::{Handled, Host};
use d2_server::seams::{ClientId, Clock, MessageSink, PlayerGate, Pos, ResultCode, SessionHandler};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::maze::Maze;
use d2_sim::drlg::outdoor::SubFileMap;
use d2_sim::drlg::{Drlg, Dungeon};
use d2_sim::game::Game;
use d2_sim::missiles::unit_flag;
use d2_sim::monsters::init::{GameInfo, MonstatsExtra};
use d2_sim::monsters::population::PopTables;
use d2_sim::path::CollisionRooms;
use d2_sim::rng::Seed;
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::{SkillEntry, SkillTables};
use d2_sim::units::hooks::Sim as USim;
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::modes;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionTables, DrlgWorld, KillStep, Pending, SkillEvent};
use d2_sim::wiring::economy::{monster_death_drop, DeathDrops, DropSpot, FreeSpot, GameFields};
use d2_sim::wiring::interaction::{skill_events, UseRest};
use d2_sim::wiring::worldgen::WorldPending;
use d2_sim::wiring::worldgen::{SharedTypes, WorldSim, WorldState, WorldTables, WorldTypes};
use d2_sim::world::hirelings::{flags as hflags, HirelingTables, PetNode};
use d2_sim::world::npc::hire::hire_init;
use d2_sim::world::npc::{class, code, HireRow, NpcControl};
use d2_sim::world::quests::{PlayerQuests, QuestControl, QuestTables};

#[path = "../../d2-client/tests/e2e_support/mod.rs"]
mod e2e_support;
#[path = "../../d2-client/tests/e2e_support/world.rs"]
mod e2e_world;
use e2e_support::{blank, item_tables, vendor_tables, Rest, N_MONSTATS};
use e2e_world::*;

// ---- constants ------------------------------------------------------------------------

const CLIENT: ClientId = 0;
/// Stat ids (`itemstatcost`).
const LEVEL: u16 = 12;
const EXPERIENCE: u16 = 13;
const LIFE: u16 = 6;
const PLAYER_GOLD: i32 = 5000;
const PLAYER_LEVEL: i32 = 10;
/// The mercenary's `hireling` row (fixture values):
/// expansion version 100, Act II (1-based 2), Normal column 1, seller
/// Greiz, monstats class 1 (the fixture's monstats2 row), 200 gold,
/// level 9, name ids 100–104.
const MERC_CLASS: u32 = 1;
const NAME_FIRST: u16 = 100;
const NAME_LAST: u16 = 104;
/// monstats `AI` Hireable and its think.
const HIREABLE_AI: u16 = 61;
const HIREABLE_FN: u32 = 0x005E_52D0;

fn hire_rows() -> Vec<HireRow> {
    vec![HireRow {
        version: 100,
        class: MERC_CLASS,
        act: 2,
        difficulty: 1,
        seller: u32::from(class::GREIZ),
        gold: 200,
        level: 9,
        name_first: NAME_FIRST,
        name_last: NAME_LAST,
    }]
}

/// Monstats of the NPC control: Greiz is `npc` and `interact`.
fn npc_monstats() -> Vec<Monstats> {
    let mut v = e2e_support::monstats();
    v[usize::from(class::GREIZ)].npc = true;
    v[usize::from(class::GREIZ)].interact = true;
    v
}

/// Monster class 0 (the bench fixture's: killable, level 1, 5 life, 100
/// experience, treasure class 1, `isSpawn`, groups of 1), with its
/// `MonStatsEx` row 1.
fn monster_class() -> Monstats {
    let mut m = e2e_world::monster_class();
    m.monstatsex = 1;
    m
}

/// monstats2: row 0 blank, row 1 `SizeX` 1 (a footprint the missile can
/// collide with).
fn monstats2() -> Vec<Monstats2> {
    let mut m: Monstats2 = blank();
    m.sizex = 1;
    vec![blank(), m]
}

/// The shared levels with live room population on ISLE: class 0,
/// `NumMon` 1, `MonDen` 10000 in every difficulty.
fn levels() -> Vec<Levels> {
    let mut v = e2e_world::levels();
    let g = &mut v[ISLE as usize];
    g.mon1 = 0;
    g.nummon = 1;
    g.monden = 10_000;
    g.monden_n = 10_000;
    g.monden_h = 10_000;
    v
}

/// The bench arrow at `Vel` 16 (velocity 3072 after the 75 % missile
/// speed factor), as `e2e_full_loop.rs`.
fn arrow() -> d2_data::tables::Missiles {
    let mut r = e2e_world::arrow();
    (r.vel, r.maxvel) = (16, 16);
    r
}

/// The combat tables of `e2e_full_loop.rs`: the sorceress' walk / run
/// columns (synthetic), every monstats row the units
/// read (the NPC rows, row 0 the monster class, the merc row on the
/// Hireable AI), the monstats2 shapes.
fn combat() -> CombatTables {
    let mut t = combat_tables();
    let mut charstats = vec![blank::<Charstats>(); 7];
    charstats[1].walkvelocity = 6;
    charstats[1].runvelocity = 9;
    charstats[1].rundrain = 20;
    let mut monstats = npc_monstats();
    monstats[0] = monster_class();
    monstats[MERC_CLASS as usize].ai = HIREABLE_AI;
    (t.charstats, t.monstats, t.monstats2) = (charstats, monstats, monstats2());
    t
}

fn skill_tables() -> SkillTables {
    let mut t = e2e_world::skills();
    t.missiles = vec![arrow()];
    t
}

// ---- seams without a provider --------------------------------------------------
//
// The path-provider configuration of `d2-client/tests/e2e_full_loop.rs`
// (copied: its seams are that test's own, not `e2e_support/world.rs`'s,
// whose `TestPending` answers positions, steps and crossed sub-tiles
// itself for runs without the path provider, so a path-moved missile
// never collides there).

/// The action and world-generation seams no written spec provides yet
/// (the path provider answers every position, path, placement and warp
/// seam, so none of those is here; left: the missile target distance,
/// transport, the DRLG population reads, the COF-name composer and the animation
/// rate, the monster death start's body), the skill use pipeline's rest
/// ([`UseRest`]: the player's skill state in [`Book`], for the message
/// and the timer paths alike), the drop state, and a log of the calls
/// that change something. The answers are the narrowest ones
/// (`Pending`'s defaults) except those the test stages (see each). The
/// player's interaction is the server host's (the NPC rest, `Rest`).
#[derive(Default)]
struct TestPending {
    sent: Vec<(UnitId, Vec<u8>)>,
    log: Vec<String>,
    /// The point the cast aims its missile at (the skill missile
    /// helpers' record fill is staged here).
    aim_at: (i32, i32),
    book: Book,
    /// The game's drop state, lent out during a drop.
    drops: Option<DeathDrops>,
}

impl Pending for TestPending {
    fn anim_name(&self, _: UnitId, ty: UnitType, _: u32, mode: u32) -> Option<[u8; 8]> {
        match (ty, mode) {
            (UnitType::Player, 10) => Some(*PLAYER_SC),
            (UnitType::Monster, 0) => Some(*MONSTER_DT),
            (UnitType::Monster, 2) => Some(*b"M0WLHTH\0"),
            _ => None,
        }
    }
    /// The animation rate formula is not implemented here: the AnimData speed
    /// as is (no rate stats in this game).
    fn anim_rate(&self, _: UnitId, speed: Option<u32>) -> i16 {
        speed.map_or(0, |s| s as i16)
    }
    /// The target distance is not implemented (stays `Pending` with the path
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
    /// The death start's body is not implemented: the fixture sets mode DT
    /// (a start function sets its mode) and runs the drop
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

/// The free-spot search is not implemented here: the start spot as is.
struct Spot;

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
/// bodies): the player's skill list and a call log,
/// shared with the test.
#[derive(Default)]
struct Inner {
    list: Vec<SkillEntry>,
    right: Option<SkillEntry>,
    used: Option<SkillEntry>,
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
    /// The do bodies are not implemented: logged, result
    /// 0 (the generic `srvmissile` creation still runs).
    #[allow(clippy::too_many_arguments)]
    fn srvdo(&self, i: u16, _: UnitId, s: i32, l: i32, c: bool, it: bool, a: bool) -> i32 {
        self.get()
            .log
            .push(format!("srvdo {i} {s} {l} {c} {it} {a}"));
        0
    }
}

// ---- the host ------------------------------------------------------------------------

struct NoSession;
impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

struct Ms(u32);
impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type World = WiredWorld<Rest, WiredSkills>;
type Sim = SimGame<WorldSim<TestPending>, World>;
type TestHost = Host<Sim, ProtoSizes, NoSession, Ms>;

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

struct Fx {
    host: TestHost,
    player: UnitId,
    wp: UnitId,
    npc: UnitId,
}

impl Fx {
    /// Act 0 through the level-type dispatcher on the fixed init seed;
    /// the action hooks with the drop state; the path provider; the
    /// population regions, the NPC control (with the hireling rows) and
    /// the quest control on the game seed (fixture order as in the shared e2e
    /// fixture: no object control); the
    /// ISLE level generated and its origin room streamed; the waypoint,
    /// the player (a sorceress, class 1) and Greiz allocated there; the
    /// client joined (its room set by the first tick's room change).
    fn new() -> Self {
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
        let drlg = Drlg::create(0, DRLG_SEED, 0, 0, false, &data, &mut handle).unwrap();
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
            skills: skill_tables(),
            combat: combat(),
            levels: levels(),
            skill_modes: vec![[0; 8]],
            overlay_count: 0,
            monequip: Vec::new(),
            arena: Vec::new(),
        };
        let book = Book::default();
        let gold_tables = gold_item_tables();
        let mut hooks = ActionHooks::new(
            Arc::new(tables),
            world,
            Seed::init_low(GAME_SEED),
            TestPending {
                book: book.clone(),
                drops: Some(DeathDrops::new(
                    Arc::new(drop_tables_from(gold_tables, 0)),
                    GameFields::new(Seed::init_low(GAME_SEED), false),
                )),
                ..TestPending::default()
            },
        );
        hooks.anim_data = Some(Arc::new({
            // The fixture's two names plus a monster walk (6 frames).
            use d2_formats::animdata::{self, AnimRecord};
            let mut a = anim_data();
            let name = *b"M0WLHTH\0";
            a.buckets[animdata::hash(&name[..7])].push(AnimRecord {
                name,
                frames: 6,
                speed: 256,
                events: [0; animdata::EVENTS],
            });
            a
        }));
        hooks.vitals = Some(Arc::new(vitals()));
        let wt = WorldTables {
            // Class 1 (the mercenary) too: its creation goes through the
            // population placement, which needs both rows.
            pop: PopTables::from_records(
                &levels(),
                &[monster_class(), monster_class()],
                &monstats2(),
                &[],
            ),
            monstats: vec![monster_class()],
            monstats2: monstats2(),
            monlvl: vec![blank::<Monlvl>(); 10],
            levels: levels(),
            difficultylevels: vec![blank::<Difficultylevels>(); 3],
            monstats_extra: vec![MonstatsExtra::default(); 2],
            components: vec![[0; 16]; 2],
            ..WorldTables::default()
        };
        let state = WorldState::new(types, Arc::new(wt), GameInfo::default());
        // Class 0 (the population and DS1 monster) stands (no movement
        // bits): the kill below aims at where it was placed. Every other
        // class (the NPC, the mercenary) is enabled and stands.
        let unit_data = UnitData {
            monsters: (0..N_MONSTATS)
                .map(|_| MonsterInfo {
                    mode_chart: false,
                    enabled: true,
                    aidel: [15; 3],
                    moves: 0,
                })
                .collect(),
            ..UnitData::default()
        };
        let mut sim = WorldSim::new(stat_data(), unit_data, hooks, state);
        enable_paths(&mut sim).unwrap();
        sim.create_regions();
        let mut seed = sim.action.hooks().game_seed;
        let npc_ctl = NpcControl::new(&npc_monstats(), hire_rows(), true, 0, &mut seed).unwrap();
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        sim.action.hooks().game_seed = seed;
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = sim
            .action
            .hooks()
            .drlg
            .with_act(0, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, ISLE)?;
                d.generate_level(svc.data, svc.types, l)?;
                let a = *d
                    .level_rooms(l)
                    .iter()
                    .find(|&&r| d.room(r).rect == d2_sim::drlg::TileRect::new(8000, 8000, 8, 8))
                    .expect("room at the level origin");
                Ok::<_, d2_sim::drlg::DrlgError>(d.stream_room(svc, a)?.expect("active"))
            })
            .unwrap()
            .unwrap();
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
                .unwrap()
        };
        let wp = alloc(UnitType::Object, 0, WP_AT);
        let player = alloc(UnitType::Player, 1, PLAYER_AT);
        let npc = alloc(UnitType::Monster, u32::from(class::GREIZ), NPC_AT);
        sim.action.sys.units.get_mut(player).unwrap().mode = 1;
        let rec = sim.action.hooks().waypoints.entry(player).or_default();
        rec.get_mut(0).set(ISLE_WP.into()).unwrap();
        rec.get_mut(0).set(GATE_WP.into()).unwrap();
        // The player's stat init is not written (synthetic, as the other
        // e2e runs): mana 4000 and max, gold, level, velocity 100 %,
        // stamina and max stamina.
        sim.action.with(&mut game, |_, v| {
            v.set_base(player, 8, 4000);
            v.set_base(player, 9, 4000);
            v.set_base(player, GOLD, PLAYER_GOLD);
            v.set_base(player, LEVEL, PLAYER_LEVEL);
            v.set_base(player, 67, 100);
            v.set_base(player, 10, 0x6400);
            v.set_base(player, 11, 0x6400);
        });

        let mut rest = Rest::default();
        rest.quests.insert(player, PlayerQuests::default());
        let action = ActionWorld {
            waypoints: Some(waypoint_data()),
            skills: WiredSkills::default(),
            ..ActionWorld::default()
        };
        let mut world = WiredWorld::new(
            action,
            item_tables(),
            quests,
            npc_ctl,
            vendor_tables(),
            rest,
            1000,
        );
        // Monster init embeds the NPC's interaction list.
        world.state.add_npc(npc);
        let mut s: Sim = SimGame::with_world(game, sim, world);
        s.join(CLIENT, Some(player), None, client_state::IN_GAME)
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
        let mut host: TestHost = Host::new(s, ProtoSizes, NoSession, Ms(1000));
        host.connect(CLIENT);
        let r = host.frame().unwrap();
        assert!(!r.ticked);
        let mut fx = Self {
            host,
            player,
            wp,
            npc,
        };
        // Frame 2 (tick 1): the client's room change activates the rooms
        // near the player and the room pass populates them.
        fx.step(&[]);
        fx.assert_clean();
        fx
    }

    fn sim(&mut self) -> &mut Sim {
        &mut self.host.game
    }
    fn guid(&self, u: UnitId) -> u32 {
        self.host.game.game.lists.unit(u).unwrap().guid
    }
    fn mode(&mut self, u: UnitId) -> u32 {
        self.sim().events.action.sys.units.get(u).unwrap().mode
    }
    fn room(&self, u: UnitId) -> Option<RoomId> {
        self.host.game.game.lists.unit(u).and_then(|e| e.room())
    }
    fn pos(&mut self, u: UnitId) -> (i32, i32) {
        self.sim().events.action.hooks().path_position(u)
    }
    fn stat(&mut self, u: UnitId, s: u16) -> i32 {
        self.sim().events.action.sys.stats.unit_total(u, s, 0)
    }
    fn pending(&mut self) -> &mut TestPending {
        &mut self.sim().events.action.hooks().x
    }
    fn monsters(&self) -> Vec<UnitId> {
        let mut v = self.host.game.game.lists.units_of_type(UnitType::Monster);
        v.retain(|&u| u != self.npc);
        v.sort();
        v
    }
    fn missiles(&self) -> Vec<UnitId> {
        let mut v = self.host.game.game.lists.units_of_type(UnitType::Missile);
        v.sort();
        v
    }
    /// The unit's timer events (type, expiry), sorted.
    fn timers(&self, u: UnitId) -> Vec<(u8, i32)> {
        let t = &self.host.game.game.timers;
        let mut v: Vec<_> = t
            .unit_timers(u)
            .into_iter()
            .filter_map(|i| Some((t.event(i)?.0, t.expire(i)?)))
            .collect();
        v.sort();
        v
    }
    fn errors(&self) -> Vec<String> {
        let s = &self.host.game;
        let mut e = s.events.errors();
        e.extend(s.world.action.faults.iter().map(|f| format!("{f:?}")));
        e.extend(s.world.state.errors.iter().map(|f| format!("{f:?}")));
        e.extend(s.tick_faults.iter().map(|f| format!("{f:?}")));
        e.extend(s.unhandled.iter().map(|f| format!("unhandled {f:?}")));
        e
    }
    fn assert_clean(&self) {
        assert_eq!(self.errors(), Vec::<String>::new());
    }

    /// The dispatcher's range checks read the
    /// staged unit facts: staged from the path positions.
    fn stage_facts(&mut self) {
        let mut units = vec![self.player, self.wp, self.npc];
        units.extend(self.monsters());
        for u in units {
            let (x, y) = self.pos(u);
            self.sim().set_unit(
                u,
                UnitFacts {
                    act: 0,
                    pos: Pos { x, y },
                    owner: None,
                },
            );
        }
    }

    /// One host frame of 40 ms with `msgs` from the client: the codes,
    /// the S→C the client received.
    fn step(&mut self, msgs: &[Vec<u8>]) -> (Vec<ResultCode>, Vec<Vec<u8>>) {
        self.stage_facts();
        for m in msgs {
            let sent = self.host.send_game(CLIENT, m).unwrap();
            assert!(sent.is_some(), "dropped by the duplicate filter");
        }
        self.host.clock.0 += 40;
        let r = self.host.frame().unwrap();
        assert!(r.ticked);
        let codes = r
            .messages
            .iter()
            .map(|m| match m.handled {
                Handled::Game(Outcome::Dispatched(c)) => c,
                ref h => panic!("not dispatched: {h:?}"),
            })
            .collect();
        (codes, self.host.receive(CLIENT))
    }
}

fn bytes<M: FixedMessage>(m: &M) -> Vec<u8> {
    let mut b = vec![0; M::SIZE];
    m.write(&mut b);
    b
}

/// C→S 0x13 InteractWithEntity (unit type u32@1, GUID u32@5).
fn interact(unit_type: u8, guid: u32) -> Vec<u8> {
    let mut m = vec![0x13, unit_type, 0, 0, 0];
    m.extend(guid.to_le_bytes());
    m
}

// ---- 1. hiring ------------------------------------------------------------------------

// Talking to Greiz offers names; hiring one takes the gold and spawns the mercenary.
#[test]
fn hiring_at_greiz_spawns_the_mercenary() {
    let mut fx = Fx::new();
    let (p, ng) = (fx.player, fx.guid(fx.npc));
    // The name offer worked out by hand on a copy of the NPC-control seed: one step per slot
    // (n = 104 − 100 + 1 = 5), then up to 10 offers.
    let mut seed = fx.sim().world.npc.seed;
    let n = usize::from(NAME_LAST - NAME_FIRST + 1);
    let seeds: Vec<u32> = (0..n).map(|_| seed.step()).collect();
    let mut offered = vec![false; n];
    for _ in 0..10 {
        let s = seed.roll(n as i32) as usize;
        let mut q = s;
        while offered[q] {
            q = (q + 1) % n;
            if q == s {
                break;
            }
        }
        if offered[q] {
            break;
        }
        offered[q] = true;
    }
    // The talk (C→S 0x13, unit type 1): the server sends 0x4F, then one 0x4E
    // per offered slot in slot order (name id u16, seed u32).
    let (codes, got) = fx.step(&[interact(1, ng)]);
    assert_eq!(codes, [ResultCode::Done]);
    let mut list = vec![vec![0x4F]];
    for i in (0..n).filter(|&i| offered[i]) {
        let mut m = vec![0x4E];
        m.extend((NAME_FIRST + i as u16).to_le_bytes());
        m.extend(seeds[i].to_le_bytes());
        list.push(m);
    }
    assert_eq!(got[..list.len()].to_vec(), list, "{got:02x?}");
    assert_eq!(fx.sim().world.npc.seed, seed);
    // The hire (C→S 0x36: NPC GUID u32@1, name u16@5) of the first
    // offered name: the offer is rolled on the slot seed, the price is
    // taken, and the unit creation (near the NPC, then the player) is the
    // rest's `spawn_mercenary`. The gold is already taken when the hire
    // is reported.
    let k = offered.iter().position(|&o| o).unwrap();
    let name = NAME_FIRST + k as u16;
    let want = hire_init(&hire_rows(), 100, seeds[k], 1, 0, PLAYER_LEVEL as u32).unwrap();
    let mut m = vec![0x36];
    m.extend(ng.to_le_bytes());
    m.extend(name.to_le_bytes());
    m.extend([0, 0]);
    let (codes, got) = fx.step(&[m]);
    assert_eq!(codes, [ResultCode::Done]);
    let gold = PLAYER_GOLD - want.price as i32;
    assert_eq!(fx.stat(p, GOLD), gold);
    // The unit spawn is `LifecycleHooks::spawn_near` (the
    // client is told of the new monster (S→C 0xAC) and of the hire
    // (0x2A code 5 with the mercenary's GUID).
    assert!(got.iter().any(|m| m[0] == 0xAC), "{got:02x?}");
    assert!(
        got.iter().any(|m| m[0] == 0x2A && m[2] == code::MERC),
        "{got:02x?}"
    );
    assert!(!got.iter().any(|m| m[0] == 0x2A && m[2] == code::NOT_PLACED));
    let slots = &fx
        .sim()
        .world
        .npc
        .record(class::GREIZ)
        .unwrap()
        .hire
        .as_ref()
        .unwrap()
        .slots;
    assert!(slots[k].hired);
    // The fixture holds no hireling tables, so the merc init reports
    // `NoHirelingTables` (the live host loads them): not `assert_clean`.
}

// ---- 2. the hireling follows a waypoint teleport ---------------------------------------

// A same-act waypoint warp brings the hireling to the player's point and room.
#[test]
fn the_hireling_follows_a_waypoint_teleport() {
    let mut fx = Fx::new();
    let p = fx.player;
    // The hireling the hire would have made (its unit spawn is Pending,
    // flow 1): a mercenary-class monster beside the player, and the pet
    // node the hire adds for it, staged.
    let merc = {
        let room = fx.room(p);
        let req = AllocRequest {
            ty: UnitType::Monster,
            class: MERC_CLASS,
            room,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: false,
        };
        let s = fx.sim();
        s.events
            .action
            .with(&mut s.game, |g, v| {
                v.allocate(g, &req, PLAYER_AT.0, PLAYER_AT.1 + 3)
            })
            .unwrap()
    };
    let mg = fx.guid(merc);
    let merc_at = fx.pos(merc);
    // `pettype` row 7 of the game's data: warp 1, range 0, basemax 1.
    let rows = d2_sim::world::hirelings::HirelingRows::default();
    let w = &mut fx.sim().world;
    w.state.hireling_tables = Some(HirelingTables {
        rows,
        exp_ratios: Default::default(),
        max_level: 99,
        pet_flags: HirelingTables::WARP,
        pet_basemax: 1,
    });
    w.state.hirelings.list_mut(p).nodes = vec![PetNode {
        guid: mg,
        ..PetNode::default()
    }];
    // The waypoint menu open (its operate path is staged on the player's
    // interact info, as `e2e_full_loop.rs`), then C→S 0x49 to GATE:
    // the same-act warp places the player in GATE's spawn room
    // which queues the pet follow; the host runs it after the handler.
    let wg = fx.guid(fx.wp);
    fx.sim()
        .events
        .action
        .sys
        .units
        .get_mut(p)
        .unwrap()
        .interact
        .set(2, wg);
    let travel = bytes(&TakeOrCloseWp {
        wp: wg,
        level: GATE as u16,
    });
    let (codes, got) = fx.step(&[travel]);
    assert_eq!(codes, [ResultCode::Done]);
    let at = fx.pos(p);
    let room = fx.room(p).expect("placed");
    let rect = fx
        .sim()
        .events
        .action
        .hooks()
        .drlg
        .subtile_rect(room)
        .unwrap();
    assert!(rect.contains(at.0, at.1), "in a GATE room");
    assert!((45_000..45_200).contains(&at.0) && (45_000..45_090).contains(&at.1));
    // 0x07 of the placement room, then 0x0D at the position + 3.
    assert_eq!(got[0][0], 0x07, "{got:02x?}");
    assert_eq!(got[1][0], 0x0D, "{got:02x?}");
    // Warp: the living node's unit moves to the player (placed at the
    // player's room and point by the path teleport, flags 2 bit 0x10000
    // set). The bit is cleared again by the end-of-tick room clean-up
    // (flag-ex 0x10000 := 0), which the tick runs; after the step it reads 0.
    let f2 = fx.sim().events.action.sys.units.get(merc).unwrap().flags2;
    assert_eq!(f2 & hflags::WARP2, 0);
    assert_ne!(fx.pos(merc), merc_at, "the hireling moved");
    assert_eq!(fx.pos(merc), at, "at the player's point");
    assert_eq!(fx.room(merc), Some(room), "in the player's room");
    assert_eq!(fx.sim().events.action.hooks().pet_follows, Some(vec![]));
    fx.assert_clean();
}

// ---- 2b. the restored hireling thinks by its Hireable AI ------------------------------

/// The save restore runs on the lent monster world: the unit gets its AI
/// control at the type init and the owner link at the hireling init, so
/// the Hireable think (`HIREABLE_FN`) drives it; no host stand-in does.
#[test]
fn the_restored_hireling_takes_the_hireable_think() {
    use d2_sim::monsters::ai::UnitRef;
    use d2_sim::wiring::action::HirelingCall;
    use d2_sim::world::hirelings::life::{Loader, SavedHireling};
    use d2_sim::world::hirelings::{HirelingRow, HirelingRows};
    let mut fx = Fx::new();
    let p = fx.player;
    let pg = fx.guid(p);
    let row = HirelingRow {
        id: 1,
        class: MERC_CLASS,
        act: 1,
        difficulty: 1,
        seller: u32::from(class::GREIZ),
        level: 1,
        exp_lvl: 100,
        name_first: NAME_FIRST,
        name_last: NAME_LAST,
        ..HirelingRow::default()
    };
    fx.sim().world.state.hireling_tables = Some(HirelingTables {
        rows: HirelingRows::new(vec![row]),
        exp_ratios: Default::default(),
        max_level: 99,
        pet_flags: HirelingTables::WARP,
        pet_basemax: 1,
    });
    fx.sim()
        .events
        .action
        .hooks()
        .hireling_calls
        .get_or_insert_with(Vec::new)
        .extend([
            HirelingCall::Restore {
                player: p,
                saved: SavedHireling {
                    dead: false,
                    seed: 5,
                    name_index: 0,
                    id: 1,
                    experience: 0,
                },
                loader: Loader::Current,
            },
            HirelingCall::JoinFollow(p),
        ]);
    fx.step(&[]);
    let merc: Vec<UnitId> = fx
        .monsters()
        .into_iter()
        .filter(|&u| fx.host.game.events.action.sys.units.get(u).unwrap().class == MERC_CLASS)
        .collect();
    assert_eq!(merc.len(), 1, "{:?}", fx.errors());
    let merc = merc[0];
    let control = |fx: &mut Fx| {
        fx.sim()
            .events
            .action
            .sys
            .hooks
            .ai
            .as_ref()
            .and_then(|s| s.control(merc))
            .cloned()
            .expect("the type init's AI control")
    };
    let c = control(&mut fx);
    assert_eq!(
        c.minion_owner,
        Some(UnitRef {
            ty: UnitType::Player,
            guid: pg,
        })
    );
    assert_eq!(c.function, HIREABLE_FN);
    // It stays on the Hireable think while its owner is there.
    for _ in 0..10 {
        fx.step(&[]);
    }
    assert_eq!(control(&mut fx).function, HIREABLE_FN);
    fx.assert_clean();
}

// ---- 3. a room-population monster killed with a missile --------------------------------

// A room-population monster is walked to, killed by the right skill's
// missile, drops gold, and the client sees the death messages.
#[test]
fn a_population_monster_killed_with_a_missile() {
    let mut fx = Fx::new();
    let p = fx.player;
    // The room pass of tick 1 populated the activated rooms: the DS1's
    // preset monster (logged as `preset`) and room population's.
    let preset: Vec<String> = fx
        .pending()
        .log
        .iter()
        .filter(|l| l.starts_with("preset"))
        .cloned()
        .collect();
    assert_eq!(preset.len(), 1, "{:?}", fx.pending().log);
    let presets: Vec<u32> = preset
        .iter()
        .map(|l| l.split(' ').nth(1).unwrap().parse().unwrap())
        .collect();
    let mut populated: Vec<UnitId> = fx
        .monsters()
        .into_iter()
        .filter(|u| !presets.contains(&u.0))
        .collect();
    assert!(!populated.is_empty(), "room population placed none");
    // The nearest one to the player.
    let me = fx.pos(p);
    populated.sort_by_key(|&u| {
        let q = fx.host.game.events.action.sys.hooks.path_position(u);
        ((q.0 - me.0).pow(2) + (q.1 - me.1).pow(2), u.0)
    });
    let monster = populated[0];
    let mpos = fx.pos(monster);
    let mroom = fx.room(monster);
    fx.pending().log.clear();
    // Walk (C→S 0x01) to 4 sub-tiles from it on the x axis, toward the
    // player; per-tick path steps until the player stops (mode 1).
    let dx = if me.0 < mpos.0 { -4 } else { 4 };
    let target = (mpos.0 + dx, mpos.1);
    let (codes, _) = fx.step(&[bytes(&Walk {
        x: target.0 as u16,
        y: target.1 as u16,
    })]);
    assert_eq!(codes, [ResultCode::Done]);
    let mut n = 0;
    while fx.mode(p) != 1 {
        let (_, got) = fx.step(&[]);
        assert!(got.is_empty(), "the walking player's client gets no S→C");
        n += 1;
        assert!(n < 200, "the walk did not end");
    }
    // What the kind init (not implemented) would set for the monster (its
    // target flags), the player's to-hit inputs, and the cast's aim point
    // (the skill missile helpers' record fill is staged), as `e2e_full_loop.rs`.
    let at = fx.pos(monster);
    {
        let s = fx.sim();
        s.events.action.sys.units.get_mut(monster).unwrap().flags |=
            unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
        s.events.action.hooks().x.aim_at = at;
        s.events.action.with(&mut s.game, |_, v| {
            v.set_base(p, 19, 100);
            v.set_base(p, 12, 1);
        });
    }
    // The right skill at the monster (C→S 0x0C): accepted, the missile
    // created at the action frame.
    let (codes, _) = fx.step(&[bytes(&RightSkill {
        x: at.0 as u16,
        y: at.1 as u16,
    })]);
    assert_eq!(codes, [ResultCode::Done]);
    let mut shot = Vec::new();
    let mut transcript = Vec::new();
    for _ in 0..6 {
        transcript.extend(fx.step(&[]).1);
        shot = fx.missiles();
        if !shot.is_empty() {
            break;
        }
    }
    assert_eq!(shot.len(), 1, "one missile");
    // The skill damage setup is Pending: 10
    // points on the missile, as `e2e_full_loop.rs`.
    {
        let s = fx.sim();
        s.events.action.with(&mut s.game, |_, v| {
            v.set_base(shot[0], 21, 2560);
            v.set_base(shot[0], 22, 2560);
        });
    }
    let mut n = 0;
    while fx.missiles() == shot {
        transcript.extend(fx.step(&[]).1);
        n += 1;
        assert!(n < 60, "the missile did not hit");
    }
    // The hit kills it: life 0, death mode 0, the
    // kill's experience to the player (equal levels: 100).
    assert_eq!(fx.stat(monster, LIFE), 0);
    assert_eq!(fx.mode(monster), 0);
    assert_eq!(fx.stat(p, EXPERIENCE), 100);
    let log = fx.pending().log.clone();
    let death = format!("death start {} target Some({})", monster.0, p.0);
    assert!(log.contains(&death), "{log:?}");
    // The drop: treasure class 1 picks gold; placed
    // at the start spot (x + 2, y + 3) in the monster's room, mode 3.
    // (The death start's body is not implemented: the
    // shared fixture runs the drop it is known to call.)
    let drops = fx.pending().drops.as_ref().unwrap().placed.clone();
    assert_eq!(drops.len(), 1);
    let (gold, spot) = drops[0];
    assert_eq!(
        spot,
        DropSpot {
            room: mroom,
            x: at.0 + 2,
            y: at.1 + 3
        }
    );
    assert_eq!(fx.mode(gold), 3);
    assert!((1..=6).contains(&fx.stat(gold, GOLD)));
    // The death mode's animation schedule: one event 1 when it ends. The
    // AI think the client's room join scheduled (Idle → 203) was cancelled
    // by the death clean-up, so only the animation event is left.
    let timers = fx.timers(monster);
    assert_eq!(timers.len(), 1, "{timers:?}");
    let (ev, end) = timers[0];
    assert_eq!(ev, 1);
    while fx.host.game.game.frame < end + 2 {
        transcript.extend(fx.step(&[]).1);
    }
    // Nothing is left pending on the dead monster.
    assert_eq!(fx.timers(monster), []);
    // The DT-end event (event 1) sets mode 12 (DD).
    assert_eq!(fx.mode(monster), 12, "the DT end sets DD");
    // The death messages (0x69 code 8 at the kill, code 9 when the death
    // animation ends) and the drop's 0x9C come from the client pass's
    // per-unit update, which the tick wiring does not run here, so none of
    // them is in the transcript. The monster's two mode changes while
    // its skill is in use send the skill message instead of a mode
    // message: 0x4D type 1, GUID 3, skill 1 (u32), level 10 (the low
    // byte of `skill_level(unit, E, 1)`), the path target (the spawn point
    // the creation's mode request wrote: (40013, 40022)), w 0.
    let skill_4d = vec![0x4D, 1, 3, 0, 0, 0, 1, 0, 0, 0, 10, 77, 156, 86, 156, 0, 0];
    // The kill's experience levels the player up: the party info 0x75 to the
    // clients at once, the level-up sound 0x2C with the player's update.
    let party_75 = vec![0x75, 1, 0, 0, 0, 255, 255, 2, 0, 0, 0, 1, 0];
    let sound_2c = vec![0x2C, 0, 1, 0, 0, 0, 2, 0];
    assert_eq!(
        transcript,
        vec![party_75, skill_4d.clone(), sound_2c, skill_4d]
    );
    // Nothing is logged.
    assert_eq!(fx.errors(), Vec::<String>::new());
}
