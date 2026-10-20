//! Property tests of the fully wired game over time: `SimGame` on
//! `WorldSim` (the world-generation dispatch around `ActionSim`), built
//! as `d2-client`'s single-player e2e builds it (act 0's DRLG through the
//! level-type dispatcher from a known Act I placement seed, a
//! generated preset level with a DS1 monster, the population room pass,
//! the regions on the game seed, a sorceress with the wired skill
//! handlers, a waypoint object), driven by random sequences of valid and
//! invalid C→S game messages interleaved with hundreds of ticks.
//!
//! Properties:
//! 1. no panic, debug overflow or fatal path (`sys.errors`, the world
//!    adapters' errors, the world handlers' faults), and no unbounded
//!    growth: unit counts, the timer queue, the act's room list and the
//!    active rooms, the transport outbox stay under fixed bounds;
//! 2. determinism (hard rule 6): the same game seed and message sequence
//!    run twice give byte-identical S→C output and an identical state
//!    digest after every message and every tick;
//! 3. another game seed changes the digest (it is not trivially
//!    constant);
//! 4. the dispatcher's own rejections (closed gate, wrong size, unit
//!    type ≥ 6, point out of range) return
//!    before any handler runs and leave the digest unchanged.
//!
//! The fixture (tables, DS1 / DT1 sources, the seams without a provider)
//! is `e2e_single_player.rs`'s (`e2e_support/world.rs`), used without the
//! bridge and the cube's item world (a second unit world); the NPC /
//! vendor rests and tables are `e2e_support`'s.
//! The world host is `WiredWorld<_, WiredSkills>` (waypoints, the skill
//! handlers in its skill slot, Akara beside the player, the player's
//! buckler and cap). The seam
//! answers are the e2e's (see each); none is behaviour. The messages go
//! straight to the server's dispatcher (`d2_server::dispatch`), as the
//! host's drain hands them over, and the S→C output is what the
//! handlers and the tick queue in the client's buffers.
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use d2_data::tables::{Difficultylevels, Monlvl};
use d2_proto::client::{
    AddStatPoint, BuyItem, EntityAction, InitEntityChat, InteractWithEntity, RightSkill,
    RightSkillOnUnit, SellItem, TakeOrCloseWp,
};
use d2_proto::schema::FieldType;
use d2_proto::{FixedMessage, CLIENT_MESSAGES};
use d2_server::adapters::handlers::skills::wired::WiredSkills;
use d2_server::adapters::handlers::world::{ActionWorld, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::buffers::ClientBuffers;
use d2_server::dispatch::{dispatch, gate, is_point, is_unit, kind, Gate, Kind};
use d2_server::seams::{Intents, PlayerGate, Pos, ResultCode, Tick};
use d2_sim::drlg::collision::bits;
use d2_sim::drlg::maze::Maze;
use d2_sim::drlg::outdoor::SubFileMap;
use d2_sim::drlg::{Drlg, Dungeon};
use d2_sim::game::Game;
use d2_sim::items::ItemRequest;
use d2_sim::missiles::unit_flag;
use d2_sim::monsters::init::{GameInfo, MonstatsExtra};
use d2_sim::monsters::population::PopTables;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillEntry;
use d2_sim::tick::timer::TimerClass;
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::economy::{DeathDrops, DropTables, GameFields, ItemSpawn};
use d2_sim::wiring::worldgen::{SharedTypes, WorldSim, WorldState, WorldTables, WorldTypes};
use d2_sim::world::npc::{class, NpcControl};
use d2_sim::world::quests::{PlayerQuests, QuestControl, QuestTables};
use proptest::prelude::*;
use proptest::test_runner::Config;

use crate::e2e_support;
#[path = "e2e_support/world.rs"]
mod e2e_world;
use e2e_support::{blank, item_tables, monstats as npc_monstats, vendor_tables, Rest};
use e2e_support::{inv_parts, inv_tables, store, InvFx, BUC, CAP, N_MONSTATS};
use e2e_world::*;

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

/// The drop tables over the gold-only item table.
fn drop_tables() -> DropTables {
    drop_tables_from(gold_item_tables(), 0)
}

// ---- the wired game ---------------------------------------------------------------------

type Sim = SimGame<WorldSim<TestPending>, WiredWorld<Rest, WiredSkills>>;

/// The transport client id of the local player.
const CLIENT: u32 = 0;

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

fn facts((x, y): (i32, i32)) -> UnitFacts {
    UnitFacts {
        act: 0,
        pos: Pos { x, y },
        owner: None,
    }
}

/// Bounds of property 1. Generous, fixed, independent of the run's
/// length: a leak grows past them over hundreds of ticks.
const MAX_UNITS: usize = 128;
const MAX_TIMERS_PER_UNIT: usize = 8;
const MAX_ROOMS: usize = 64;
const MAX_OUTBOX: usize = 64;

/// The game of `e2e_single_player.rs`'s steps 1–3 on `game_seed`: the
/// generated level, the player's room streamed, the player and the
/// waypoint object allocated there, the client joined, the skills and
/// waypoint handlers wired.
struct Fx {
    sim: Sim,
    out: ClientBuffers,
    player: UnitId,
    /// Akara.
    npc: UnitId,
    /// The ISLE level's DRLG room count.
    level_rooms: usize,
    /// Monsters whose combat inputs were staged (`stage_combat`).
    staged: Vec<UnitId>,
    /// Missiles whose damage was staged.
    armed: Vec<UnitId>,
}

impl Fx {
    fn new(game_seed: u32) -> Self {
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
                drops: Some(DeathDrops::new(
                    Arc::new(drop_tables()),
                    GameFields::new(Seed::init_low(game_seed), false),
                )),
                ..TestPending::default()
            },
        );
        hooks.anim_data = Some(Arc::new(anim_data()));
        hooks.vitals = Some(Arc::new(vitals()));
        let wt = WorldTables {
            pop: PopTables::from_records(&levels(), &[monster_class()], &[blank()], &[]),
            monstats: vec![monster_class()],
            monstats2: vec![blank()],
            monlvl: vec![blank::<Monlvl>(); 10],
            levels: levels(),
            difficultylevels: vec![blank::<Difficultylevels>(); 3],
            monstats_extra: vec![MonstatsExtra::default()],
            components: vec![[0; 16]],
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
        sim.create_regions();
        // The world systems' creation seeds after the regions (a fixture order).
        let mut seed = sim.action.hooks().game_seed;
        let ctl = NpcControl::new(&npc_monstats(), Vec::new(), false, 0, &mut seed).expect("npc");
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        sim.action.hooks().game_seed = seed;
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();

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
                Ok::<_, d2_sim::drlg::DrlgError>((rooms.len(), active))
            })
            .expect("act 0")
            .expect("generated");

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
        // Akara beside the player (her kind init is the monster spec's).
        let npc = alloc(UnitType::Monster, u32::from(class::AKARA), NPC_AT);
        sim.action.sys.units.get_mut(player).unwrap().mode = 1;
        let rec = sim.action.hooks().waypoints.entry(player).or_default();
        rec.get_mut(0).set(ISLE_WP.into()).unwrap();
        rec.get_mut(0).set(GATE_WP.into()).unwrap();
        // Mana and max mana 4000 (1/256 units); the to-hit inputs of
        // `stage_combat` (attack rating 100, level 1).
        sim.action.with(&mut game, |_, v| {
            v.set_base(player, 8, 4000);
            v.set_base(player, 9, 4000);
            v.set_base(player, 19, 100);
            v.set_base(player, 12, 1);
            v.set_base(player, GOLD, 5000);
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
            ctl,
            vendor_tables(),
            rest,
            1000,
        );
        world.state.add_npc(npc);
        // The game's one inventory model (the item moves', the vendors'
        // and the cube's) with the player's inventory; its item-move
        // seams answer as `InvFx` stages them.
        let pg = game.lists.unit(player).unwrap().guid;
        let inv_t = inv_tables(&world.tables, &[(2, 2), (2, 2)]);
        world.inventory = Some(inv_parts(inv_t, InvFx::default(), player, 1, pg));
        // The player's buckler and cap, made by the economy wiring on the
        // game seed and stored (mode 0) in its inventory.
        let (buckler, cap) = world.with_economy(&mut game, &mut sim, |econ, _| {
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
        for item in [buckler, cap] {
            store(&mut world, &mut game, &mut sim, (player, item), 0);
        }
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
        s.set_unit(player, facts(PLAYER_AT));
        s.set_unit(object, facts(WP_AT));
        s.set_unit(npc, facts(NPC_AT));
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
        let mut out = ClientBuffers::new();
        out.add_client(CLIENT);
        Fx {
            sim: s,
            out,
            player,
            npc,
            level_rooms,
            staged: Vec::new(),
            armed: Vec::new(),
        }
    }

    /// A listed unit's GUID (−1 once it is gone).
    fn guid(&self, u: UnitId) -> u32 {
        self.sim.game.lists.unit(u).map_or(u32::MAX, |e| e.guid)
    }

    fn pending(&mut self) -> &mut TestPending {
        &mut self.sim.events.action.hooks().x
    }

    /// Stages what the unwritten specs would hold for each unit: its act
    /// and position for the dispatcher's target checks (`UnitFacts`, as
    /// the e2e stages them), and for a new monster the kind init's target
    /// flags and its run-time collision bit,
    /// as the e2e's `stage_combat`. The cast aims at
    /// the first monster.
    fn stage(&mut self) {
        let mut all = Vec::new();
        for ty in UnitType::ALL {
            all.extend(self.sim.game.lists.units_of_type(ty));
        }
        all.sort();
        for u in all {
            let pos = self.pending().position(u);
            self.sim.set_unit(u, facts(pos));
        }
        let mut monsters = self.sim.game.lists.units_of_type(UnitType::Monster);
        monsters.sort();
        for m in monsters {
            if m == self.npc || self.staged.contains(&m) {
                continue;
            }
            self.staged.push(m);
            let sim = &mut self.sim;
            sim.events.action.sys.units.get_mut(m).unwrap().flags |=
                unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
            let (x, y) = sim.events.action.hooks().x.position(m);
            let Some(room) = sim.game.lists.unit(m).and_then(|u| u.room()) else {
                continue;
            };
            let game = &sim.game;
            if let Some(c) = sim
                .events
                .action
                .sys
                .hooks
                .drlg
                .collision_mut(game, room, x, y)
            {
                *c |= bits::MONSTER;
            }
            if self.staged.len() == 1 {
                self.pending().aim_at = (x, y);
            }
        }
    }

    /// One message through the dispatcher: its result code.
    fn send(&mut self, m: &[u8]) -> ResultCode {
        dispatch(
            &mut self.sim,
            &ProtoSizes,
            &mut self.out,
            CLIENT,
            ALIVE,
            m,
            m.len(),
        )
    }

    /// One tick. New missiles get the e2e's damage (10 points, stats 21
    /// and 22 in 1/256): the skill damage setup is not written yet.
    fn tick(&mut self) {
        self.sim.tick(&mut self.out);
        let mut missiles = self.sim.game.lists.units_of_type(UnitType::Missile);
        missiles.sort();
        let sim = &mut self.sim;
        for m in missiles {
            if self.armed.contains(&m) {
                continue;
            }
            self.armed.push(m);
            sim.events.action.with(&mut sim.game, |_, v| {
                v.set_base(m, 21, 2560);
                v.set_base(m, 22, 2560);
            });
        }
    }

    /// Every S→C buffer queued for the client since the last call (the
    /// handlers' and the tick's), in order.
    fn drain(&mut self) -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        while let Some(b) = self.out.pop(CLIENT) {
            v.push(b);
        }
        v
    }

    /// Every error so far: world adapters, level types, action adapters,
    /// the unit dispatch, the world handlers' faults, the tick's queueing
    /// faults.
    fn errors(&self) -> Vec<String> {
        let mut e = self.sim.events.errors();
        e.extend(
            self.sim
                .world
                .action
                .faults
                .iter()
                .map(|f| format!("{f:?}")),
        );
        e.extend(self.sim.world.state.errors.iter().map(|f| format!("{f:?}")));
        e.extend(self.sim.tick_faults.iter().map(|f| format!("{f:?}")));
        e
    }

    /// Live timers in the queue: every bucket and every-tick list.
    fn timer_count(&self) -> usize {
        let t = &self.sim.game.timers;
        TimerClass::RUN_ORDER
            .iter()
            .map(|&c| (0..64).map(|b| t.bucket(c, b).len()).sum::<usize>() + t.every_tick(c).len())
            .sum()
    }

    /// Every unit in the lists, by type then id.
    fn units(&self) -> Vec<UnitId> {
        let mut all = Vec::new();
        for ty in UnitType::ALL {
            let mut v = self.sim.game.lists.units_of_type(ty);
            v.sort();
            all.extend(v);
        }
        all
    }

    /// Property 1's bounds.
    fn check_bounds(&self) -> Result<(), String> {
        let units = self.units().len();
        let timers = self.timer_count();
        let lists = &self.sim.game.lists;
        let mut rooms = 0;
        let mut r = lists.room_first(0);
        while let Some(id) = r {
            rooms += 1;
            r = lists.room_next(id);
        }
        let active = lists.active_rooms(0).len();
        let outbox = self.sim.events.action.sys.hooks.x.sent.len();
        if units > MAX_UNITS
            || timers > MAX_TIMERS_PER_UNIT * units.max(1)
            || rooms > MAX_ROOMS
            || active > rooms
            || outbox > MAX_OUTBOX
        {
            return Err(format!(
                "bounds: units {units}, timers {timers}, rooms {rooms}, active {active}, outbox {outbox}"
            ));
        }
        Ok(())
    }

    /// The state digest (property 2): built from public state only. Per
    /// unit (type, id order): GUID, class, mode, flags, act, room, the
    /// fixture position, the unit seed, the item seed, the animation
    /// fields, its stat list (base and full entries), its timers (event,
    /// args, expire); then the game frame, the game seed, the drop
    /// state's seed, the act's active rooms, the timer queue's current
    /// bucket. Readable lines, so a diff names the first unit that moved.
    fn digest(&mut self) -> String {
        let mut d = String::new();
        let units = self.units();
        let sim = &mut self.sim;
        let game = &sim.game;
        let a = &mut sim.events.action;
        let _ = writeln!(
            d,
            "frame {} bucket {}",
            game.frame,
            game.timers.current_bucket()
        );
        let _ = writeln!(d, "game seed {:?}", a.sys.hooks.game_seed);
        if let Some(drops) = &a.sys.hooks.x.drops {
            let _ = writeln!(d, "drop seed {:?}", drops.fields.seed);
        }
        let _ = writeln!(d, "active {:?}", game.lists.active_rooms(0));
        for u in units {
            let e = game.lists.unit(u).expect("listed");
            let _ = write!(d, "{:?} {:?} guid {} room {:?}", e.ty, u, e.guid, e.room());
            if let Some(r) = a.sys.units.get(u) {
                let _ = write!(
                    d,
                    " class {} mode {} flags {:#x}/{:#x} act {} seed {:?} init {} item {:?} anim {:?} {} {} {}",
                    r.class,
                    r.mode,
                    r.flags,
                    r.flags2,
                    r.act,
                    r.seed,
                    r.init_seed,
                    r.item_seed,
                    r.anim.record,
                    r.anim.frame,
                    r.anim.frame_count,
                    r.anim.speed,
                );
            }
            let _ = write!(d, " pos {:?}", a.sys.hooks.x.pos.get(&u));
            if let Some(l) = a.sys.stats.unit_list(u) {
                let _ = write!(
                    d,
                    " stats {:?} {:?}",
                    a.sys.stats.base_entries(l),
                    a.sys.stats.full_entries(l)
                );
            }
            let t = &game.timers;
            let timers: Vec<_> = t
                .unit_timers(u)
                .into_iter()
                .map(|i| (t.event(i), t.expire(i)))
                .collect();
            let _ = writeln!(d, " timers {timers:?}");
        }
        // The game's one item store (records, flags, seeds); the trade
        // world: the vendor records (stores), the NPC control, the quest state, the
        // world's interaction lists.
        let _ = writeln!(d, "items {:?}", sim.events.action.sys.hooks.items);
        let w = &sim.world;
        let _ = writeln!(d, "vendors {:?}", w.state.vendors);
        let _ = writeln!(d, "interactions {:?}", w.state.lists);
        let _ = writeln!(d, "npc {:?}", w.npc);
        let _ = writeln!(d, "quests {:?}", w.quests);
        d
    }
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ---- messages ---------------------------------------------------------------------------

/// One generated C→S game message (as `prop_handle.rs`): any id, layout
/// fields from a pool built on the game's GUIDs.
#[derive(Clone, Debug)]
struct Gen {
    id: u8,
    picks: Vec<(u8, u32)>,
    raw: Vec<u8>,
    /// 0: exact size; 1: one byte short; 2: one byte long.
    len: u8,
}

fn gen_any() -> impl Strategy<Value = Gen> {
    (
        1u8..0x67,
        prop::collection::vec((any::<u8>(), any::<u32>()), 8),
        prop::collection::vec(any::<u8>(), 0..64),
        prop_oneof![8 => Just(0u8), 1 => Just(1u8), 1 => Just(2u8)],
    )
        .prop_map(|(id, picks, raw, len)| Gen {
            id,
            picks,
            raw,
            len,
        })
}

/// The ids with a handler on this host (skills 0x05–0x11, 0x3A–0x3C;
/// waypoints 0x49) get most of the generated messages.
fn gen() -> impl Strategy<Value = Gen> {
    let owned = prop::sample::select(vec![
        0x05u8, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x3A, 0x3B,
        0x3C, 0x49,
    ]);
    prop_oneof![
        1 => gen_any(),
        2 => (owned, gen_any()).prop_map(|(id, g)| Gen { id, ..g }),
    ]
}

fn pick(name: &str, guids: &[u32], (sel, rnd): (u8, u32)) -> u32 {
    if sel & 0x80 != 0 {
        let guid = match rnd as usize % (guids.len() + 1) {
            i if i < guids.len() => guids[i],
            _ => u32::MAX,
        };
        return match name {
            "x" => PLAYER_AT.0 as u32 + rnd % 101 - 50,
            "y" => PLAYER_AT.1 as u32 + rnd % 101 - 50,
            "type" => rnd % 6,
            "id" | "item" | "unit" | "wp" | "target" | "player" => guid,
            "skill" => rnd % 3,
            "level" => [ISLE, GATE, rnd % 150][rnd as usize % 3],
            "stat" => rnd % 8,
            "left" => rnd % 2,
            _ => rnd % 4,
        };
    }
    const POOL: &[u32] = &[
        0,
        1,
        2,
        5,
        6,
        7,
        0xFF,
        0xFFFF,
        0x7FFF,
        0x8000,
        0x7FFF_FFFF,
        0x8000_0000,
        u32::MAX,
        40_020,
        40_070,
        40_071,
        39_969,
        39_970,
        358,
        359,
    ];
    let n = POOL.len() + guids.len() + 1;
    match sel as usize % n {
        i if i < POOL.len() => POOL[i],
        i if i < POOL.len() + guids.len() => guids[i - POOL.len()],
        _ => rnd,
    }
}

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

fn bytes<M: FixedMessage>(m: &M) -> Vec<u8> {
    let mut b = vec![0; M::SIZE];
    m.write(&mut b);
    b
}

/// One step of a run: a message (or none), then `ticks` ticks.
#[derive(Clone, Debug)]
enum Msg {
    None,
    /// Right skill at the `n`-th monster's position (or a point near the
    /// player when there is none): the e2e's cast.
    CastAt(u8),
    /// Right skill on the `n`-th unit of the game.
    CastOn(u8),
    /// A stat point on stat `n % 8`.
    Stat(u8),
    /// Waypoint travel to ISLE or GATE.
    Travel(bool),
    /// Talk to Akara, chat, trade.
    Talk,
    Chat,
    Trade,
    /// Buy the `n`-th store item.
    Buy(u8),
    /// Sell the `n`-th item of the player's inventory.
    Sell(u8),
    /// Anything, valid or not.
    Any(Gen),
}

#[derive(Clone, Debug)]
struct Op {
    msg: Msg,
    ticks: u8,
}

fn op() -> impl Strategy<Value = Op> {
    let msg = prop_oneof![
        2 => Just(Msg::None),
        2 => any::<u8>().prop_map(Msg::CastAt),
        1 => any::<u8>().prop_map(Msg::CastOn),
        1 => any::<u8>().prop_map(Msg::Stat),
        1 => any::<bool>().prop_map(Msg::Travel),
        1 => Just(Msg::Talk),
        1 => Just(Msg::Chat),
        1 => Just(Msg::Trade),
        1 => any::<u8>().prop_map(Msg::Buy),
        1 => any::<u8>().prop_map(Msg::Sell),
        6 => gen().prop_map(Msg::Any),
    ];
    (msg, 0u8..8).prop_map(|(msg, ticks)| Op { msg, ticks })
}

fn message(fx: &mut Fx, msg: &Msg) -> Option<Vec<u8>> {
    let units = fx.units();
    let nth = |v: &[UnitId], n: u8| (!v.is_empty()).then(|| v[n as usize % v.len()]);
    Some(match msg {
        Msg::None => return None,
        Msg::CastAt(n) => {
            let mut monsters = fx.sim.game.lists.units_of_type(UnitType::Monster);
            monsters.sort();
            let (x, y) = match nth(&monsters, *n) {
                Some(m) => fx.pending().position(m),
                None => (PLAYER_AT.0 - 8, PLAYER_AT.1 - 10),
            };
            bytes(&RightSkill {
                x: x as u16,
                y: y as u16,
            })
        }
        Msg::CastOn(n) => {
            let u = nth(&units, *n)?;
            let e = fx.sim.game.lists.unit(u)?;
            bytes(&RightSkillOnUnit {
                type_: e.ty as u32,
                id: e.guid,
            })
        }
        Msg::Stat(n) => bytes(&AddStatPoint {
            stat: *n % 8,
            repeat: 0,
        }),
        Msg::Travel(isle) => {
            let wp = fx.units().into_iter().find_map(|u| {
                let e = fx.sim.game.lists.unit(u)?;
                (e.ty == UnitType::Object).then_some(e.guid)
            })?;
            bytes(&TakeOrCloseWp {
                wp,
                level: if *isle { ISLE } else { GATE } as u16,
            })
        }
        Msg::Talk => bytes(&InteractWithEntity {
            type_: 1,
            id: fx.guid(fx.npc),
        }),
        Msg::Chat => bytes(&InitEntityChat {
            id: fx.guid(fx.npc),
        }),
        Msg::Trade => bytes(&EntityAction {
            action: 1,
            npc: fx.guid(fx.npc),
            item: 0,
        }),
        Msg::Buy(n) => {
            let w = &fx.sim.world;
            let store = w.state.vendors[w.state.vendor_index(class::AKARA)?]
                .store
                .clone();
            let item = nth(&store, *n)?;
            bytes(&BuyItem {
                npc: fx.guid(fx.npc),
                item: fx.guid(item),
                transaction: 0,
                client_price: 0,
            })
        }
        Msg::Sell(n) => {
            let state = &fx.sim.world.inventory.as_ref()?.state;
            let inv = state.items_of(fx.player);
            let item = nth(&inv, *n)?;
            bytes(&SellItem {
                npc: fx.guid(fx.npc),
                item: fx.guid(item),
                item_mode: 0,
                client_price: 0,
            })
        }
        Msg::Any(g) => {
            let guids: Vec<u32> = units
                .iter()
                .filter_map(|&u| Some(fx.sim.game.lists.unit(u)?.guid))
                .collect();
            build(g, &guids)
        }
    })
}

/// Whether the dispatcher itself rejects `m` before any handler
/// (closed gate, wrong size, unit type, point out of range), given the
/// player's staged position; checked against the result code.
fn player_flags2(fx: &Fx) -> u32 {
    fx.sim
        .events
        .action
        .sys
        .units
        .get(fx.player)
        .map_or(0, |r| r.flags2)
}

fn set_player_flags2(fx: &mut Fx, v: u32) {
    if let Some(r) = fx.sim.events.action.sys.units.get_mut(fx.player) {
        r.flags2 = v;
    }
}

fn dispatcher_rejects(fx: &Fx, m: &[u8], code: ResultCode) -> Result<bool, String> {
    let (id, size) = (m[0], m.len());
    let row = &CLIENT_MESSAGES[id as usize];
    if gate(id) == Gate::Dead {
        // The player is alive: the dead gate is closed.
        return if code == ResultCode::Done {
            Ok(true)
        } else {
            Err(format!("{m:02X?}: closed gate gave {code:?}"))
        };
    }
    let expect = if kind(id) != Kind::Handler {
        return Ok(code == ResultCode::Malformed);
    } else if row.transport_size.fixed().is_some_and(|n| n != size) {
        ResultCode::Malformed
    } else if is_unit(id) && u32::from_le_bytes([m[1], m[2], m[3], m[4]]) >= 6 {
        ResultCode::Invalid
    } else if is_point(id) {
        let p = fx.sim.point_state(CLIENT).expect("staged").player;
        let at = |o: usize| i32::from(u16::from_le_bytes([m[o], m[o + 1]]));
        if (at(1) - p.x).abs() > 50 || (at(3) - p.y).abs() > 50 {
            ResultCode::Refused
        } else {
            return Ok(false);
        }
    } else {
        return Ok(false);
    };
    if code == expect {
        Ok(true)
    } else {
        Err(format!("{m:02X?}: expected {expect:?}, got {code:?}"))
    }
}

/// What a run leaves behind, per message and per tick: the step, the
/// result code, the S→C buffers, the digest.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    /// The message's id, `None` for a tick.
    id: Option<u8>,
    what: String,
    code: Option<ResultCode>,
    sent: Vec<Vec<u8>>,
    digest: String,
}

/// Runs `ops` on a fresh game: before each message the fixture stages
/// unit facts; after each message and each tick the bounds and the
/// error lists are checked (property 1), the dispatcher's rejections
/// compared (property 4), and a record kept (property 2). After the ops,
/// `quiet` ticks without messages.
fn run(game_seed: u32, ops: &[Op], quiet: u32) -> Result<Vec<Entry>, String> {
    let mut fx = Fx::new(game_seed);
    let mut log = Vec::new();
    let tick = |fx: &mut Fx, log: &mut Vec<Entry>| -> Result<(), String> {
        fx.tick();
        let sent = fx.drain();
        let e = fx.errors();
        if !e.is_empty() {
            return Err(format!("tick {}: {e:?}", fx.sim.game.frame));
        }
        fx.check_bounds()?;
        log.push(Entry {
            id: None,
            what: "tick".into(),
            code: None,
            sent,
            digest: fx.digest(),
        });
        Ok(())
    };
    for op in ops {
        fx.stage();
        if let Some(m) = message(&mut fx, &op.msg) {
            let before = fx.digest();
            let resyncs = fx.sim.resyncs.len();
            let flags2_before = player_flags2(&fx);
            let code = fx.send(&m);
            let rejected = dispatcher_rejects(&fx, &m, code)?;
            // A refused target more than 25 frames after the last accepted
            // one queues the player for S→C 0x15: the unit's flag-ex (flags2) bit
            // 0x10000 is set and the unit joins the update queue. The digest
            // holds the flag word, not the queue, so that bit is the one
            // field that may differ; exactly it, and only on a resync.
            let resynced = fx.sim.resyncs.len() != resyncs;
            let flags2_after = player_flags2(&fx);
            if resynced {
                if !rejected {
                    return Err(format!("{m:02X?}: resync without a refusal ({code:?})"));
                }
                if flags2_after != flags2_before | 0x10000 {
                    return Err(format!(
                        "{m:02X?}: resync set flags2 {flags2_before:#x} -> {flags2_after:#x}, not just 0x10000"
                    ));
                }
                set_player_flags2(&mut fx, flags2_before);
            } else if flags2_after != flags2_before && rejected {
                return Err(format!(
                    "{m:02X?}: rejected without a resync, flags2 changed"
                ));
            }
            if rejected {
                let after = fx.digest();
                if resynced {
                    set_player_flags2(&mut fx, flags2_after);
                }
                if after != before {
                    return Err(format!(
                        "{m:02X?} was rejected ({code:?}) and changed the digest:\n{before}\n---\n{after}"
                    ));
                }
            }
            let e = fx.errors();
            if !e.is_empty() {
                return Err(format!("{m:02X?}: {e:?}"));
            }
            fx.check_bounds()?;
            log.push(Entry {
                id: Some(m[0]),
                what: format!("{m:02X?}"),
                code: Some(code),
                sent: fx.drain(),
                digest: fx.digest(),
            });
        }
        for _ in 0..op.ticks {
            tick(&mut fx, &mut log)?;
        }
    }
    for _ in 0..quiet {
        tick(&mut fx, &mut log)?;
    }
    Ok(log)
}

/// The first record where two runs differ, for the failure message.
fn first_diff(a: &[Entry], b: &[Entry]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!("record {i}:\n{:#?}\n---\n{:#?}", a[i], b[i]),
        None => format!("lengths {} vs {}", a.len(), b.len()),
    }
}

proptest! {
    #![proptest_config(config(8))]

    /// Properties 1, 2 and 4: hundreds of ticks with valid and invalid
    /// messages run without a panic, an error or growth past the bounds;
    /// a second run of the same seed and sequence is byte-identical.
    #[test]
    fn same_seed_same_messages_same_game(
        seed in any::<u32>(),
        ops in prop::collection::vec(op(), 40..80),
    ) {
        let a = run(seed, &ops, 100).map_err(TestCaseError::fail)?;
        let b = run(seed, &ops, 100).map_err(TestCaseError::fail)?;
        let ticks = a.iter().filter(|r| r.code.is_none()).count();
        prop_assert!(ticks >= 100);
        prop_assert!(a == b, "{}", first_diff(&a, &b));
    }

    /// Property 3: another game seed gives another digest sequence.
    #[test]
    fn other_seed_other_digest(
        seed in any::<u32>(),
        other in any::<u32>(),
        ops in prop::collection::vec(op(), 0..10),
    ) {
        prop_assume!(seed != other);
        let a = run(seed, &ops, 5).map_err(TestCaseError::fail)?;
        let b = run(other, &ops, 5).map_err(TestCaseError::fail)?;
        let (da, db): (Vec<u64>, Vec<u64>) = (
            a.iter().map(|r| hash(&r.digest)).collect(),
            b.iter().map(|r| hash(&r.digest)).collect(),
        );
        prop_assert_ne!(da, db);
    }
}

/// The fixture is live: the first tick populates the level (the DS1's
/// preset monster), the e2e's cast at it is
/// accepted, the missile flies and the monster dies; the digest moves
/// with the game.
#[test]
fn fixture_reaches_the_wired_paths() {
    let mut fx = Fx::new(GAME_SEED);
    assert_eq!(fx.level_rooms, 15);
    let d0 = fx.digest();
    fx.tick();
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
    let mut monsters = fx.sim.game.lists.units_of_type(UnitType::Monster);
    monsters.retain(|&m| m != fx.npc);
    assert_eq!(monsters.len(), 1, "the DS1 preset monster");
    assert_ne!(fx.digest(), d0);
    fx.stage();
    let (x, y) = fx.pending().position(monsters[0]);
    let code = fx.send(&bytes(&RightSkill {
        x: x as u16,
        y: y as u16,
    }));
    assert_eq!(code, ResultCode::Done);
    let mode_of = |fx: &Fx| {
        fx.sim
            .events
            .action
            .sys
            .units
            .get(monsters[0])
            .map(|r| r.mode)
    };
    // Killed (DT), then the end of the death animation sets DD.
    let mut killed = false;
    for _ in 0..40 {
        fx.tick();
        killed |= mode_of(&fx) == Some(0);
    }
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
    assert!(killed, "killed (DT)");
    assert_eq!(mode_of(&fx), Some(12), "dead (DD)");
    assert_eq!(
        fx.sim
            .events
            .action
            .sys
            .units
            .get(fx.player)
            .map(|r| r.mode),
        Some(1)
    );
}

/// The generated runs reach the wired paths (a spot check of the
/// generator, not a property): over a fixed sample of runs, the handled
/// ids answer 0 and refusals, the dispatcher rejects, missiles fly, and
/// the trade handlers send S→C messages.
#[test]
fn runs_reach_the_handlers() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let ops = prop::collection::vec(op(), 60);
    let mut codes: BTreeMap<(u8, String), u32> = BTreeMap::new();
    let (mut sent, mut missiles, mut ticks) = (BTreeMap::<u8, u32>::new(), 0, 0);
    for seed in 0..4 {
        let ops = ops.new_tree(&mut runner).unwrap().current();
        for r in run(seed, &ops, 50).unwrap() {
            match r.id {
                Some(id) => *codes.entry((id, format!("{:?}", r.code))).or_default() += 1,
                None => ticks += 1,
            }
            for b in r.sent {
                *sent.entry(b[0]).or_default() += 1;
            }
            missiles += r
                .digest
                .lines()
                .filter(|l| l.starts_with("Missile"))
                .count();
        }
    }
    eprintln!("ticks {ticks}, missile-ticks {missiles}\ncodes {codes:?}\nS→C buffers by first id {sent:?}");
    let done = |id: u8| codes.contains_key(&(id, "Some(Done)".to_string()));
    for id in [0x0C, 0x0D, 0x13, 0x2F, 0x38, 0x3A, 0x49] {
        assert!(done(id), "{id:#04X} never accepted");
    }
    for c in ["Some(Malformed)", "Some(Invalid)", "Some(Refused)"] {
        assert!(codes.keys().any(|k| k.1 == c), "no {c}");
    }
    assert!(ticks >= 400 && missiles > 0);
    assert!(
        sent.contains_key(&0x27) && sent.contains_key(&0x2A),
        "{sent:?}"
    );
}
