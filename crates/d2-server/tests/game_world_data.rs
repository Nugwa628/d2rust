//! Game-file integration tests of `d2_server::world_data` and of a fully
//! wired game on the game's live data (`#[ignore]`, `D2_GAME_DIR`):
//!
//! - every DS1 named by `lvlprest` parses through the provider, with the
//!   measurements stated at `every_lvlprest_ds1_parses`;
//! - every Act I level generates through `WorldTypes`, after the fixed
//!   Act I placement;
//! - a `SimGame` on `WorldSim` built from the live tables and files: game
//!   creation, the town generated and streamed, a player joins and 100
//!   ticks pass.
//!
//! Expected values are measurements of the game's data or fixed vectors;
//! unconfirmed until the first local run.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use d2_data::bin::{self, BinSet};
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{
    decode_all, Difficultylevels, Levels, Missiles, Monequip, Monlvl, Monprop, Monstats, Monstats2,
    Monumod, Record, Superuniques,
};
use d2_formats::animdata::AnimData;
use d2_formats::mpq::ArchiveSet;
use d2_server::adapters::handlers::world::NoWorld;
use d2_server::adapters::{PlayerData, PlayerFields, SimGame};
use d2_server::buffers::QueueError;
use d2_server::seams::{ClientId, MessageSink, PlayerGate, Tick};
use d2_server::world_data::tables::LevelTables;
use d2_server::world_data::{archive, names_file, WorldFiles};
use d2_sim::combat::vitals::{init_player_stats, VitalsTables};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::maze::Maze;
use d2_sim::drlg::preset::Ds1File;
use d2_sim::drlg::{Drlg, DrlgData, Dungeon, RoomKind, Services};
use d2_sim::game::Game;
use d2_sim::monsters::ai::skill_modes;
use d2_sim::monsters::init::{component_counts, monstats_extra, GameInfo, NamedIds};
use d2_sim::monsters::population::PopTables;
use d2_sim::rng::Seed;
use d2_sim::skills::{SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::lists::NoHost;
use d2_sim::stats::{StatData, StateTable};
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::interaction::{VitalsRest, VitalsView};
use d2_sim::wiring::worldgen::dispatch::WorldSim;
use d2_sim::wiring::worldgen::levels::{SharedTypes, WorldTypes};
use d2_sim::wiring::worldgen::{WorldPending, WorldState, WorldTables};

// ---- live data ------------------------------------------------------------------------

struct Live {
    bin: BinSet,
    anim: AnimData,
    fixed: FixedSet,
    tables: LevelTables,
    files: WorldFiles,
}

fn live() -> &'static Live {
    static L: OnceLock<Live> = OnceLock::new();
    L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bin = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        let anim = fixup::read_animdata(&set).expect("AnimData.d2 reads");
        let fixed = fixup::apply(&bin, &anim).expect("fix-ups apply");
        let tables = LevelTables::from_fixed(&fixed).expect("level tables");
        let files = WorldFiles::load(
            &tables.drlg,
            &tables.preset,
            &tables.outdoor,
            archive::reader(&set),
        )
        .expect("every DRLG file loads and parses");
        Live {
            bin,
            anim,
            fixed,
            tables,
            files,
        }
    })
}

fn table(name: &str) -> &'static d2_data::bin::BinTable {
    live()
        .fixed
        .table(name)
        .unwrap_or_else(|| panic!("table {name} in the live set"))
}

fn rows<T: Record>() -> Vec<T> {
    decode_all(table(T::TABLE)).unwrap_or_else(|e| panic!("{}: {e}", T::TABLE))
}

/// The Act I creation: DRLG init seed and `dwStartSeed`.
const INIT: u32 = 644_409_375;
const START: u32 = 4_014_346_869;
/// Rogue Encampment, the server's town level.
const TOWN: u32 = 1;

// ---- every lvlprest DS1 -----------------------------------------

/// Measurements over the DS1 files named by
/// lvlprest, through the provider (`archive::load` path, every
/// `File1`–`File6` string): 2,043 files, all v12–18; preset units of type
/// 1 (2,267) and 2 (14,105) only; one record with flags ≠ 0 (value 1);
/// object ids ≥ 573: 580 ×46, 581 ×135, 582 ×24; every file of a row
/// that sets `SizeX` / `SizeY` (1,054 rows) has that stored size.
// The parser rules themselves are tested in d2-formats / preset unit tests.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn every_lvlprest_ds1_parses() {
    let l = live();
    let ds1 = &l.files.ds1.0;
    // The provider also keeps files only lvlsub names apart (`subs`);
    // `ds1` holds exactly the lvlprest names.
    let named: std::collections::BTreeSet<&Vec<u8>> = l
        .tables
        .preset
        .defs
        .iter()
        .flat_map(|d| d.file.iter())
        .filter(|p| names_file(p))
        .collect();
    assert_eq!(ds1.len(), named.len(), "every named DS1 loaded");
    println!("{} distinct lvlprest DS1 names", named.len());
    assert_eq!(ds1.len(), 2_043, "DS1 files named by lvlprest");

    let mut versions: BTreeMap<u32, usize> = BTreeMap::new();
    let mut kinds: BTreeMap<u32, usize> = BTreeMap::new();
    let mut flagged: Vec<u32> = Vec::new();
    let mut high_ids: BTreeMap<u32, usize> = BTreeMap::new();
    for (path, f) in ds1 {
        *versions.entry(f.version).or_default() += 1;
        for o in &f.objects {
            *kinds.entry(o.kind).or_default() += 1;
            if o.flags != 0 {
                flagged.push(o.flags);
            }
        }
        // The ids ≥ 573 measured above are object
        // *class* ids after the object-preset table (`preset-tables.tsv`
        // `objpreset` holds 580–582), not the stored DS1 ids (< 150 for
        // v ≥ 6). Counted on the parser's output (`Ds1File::from_input`).
        let parsed = Ds1File::from_input(f, &l.tables.preset).unwrap_or_else(|e| {
            panic!("{}: {e:?}", String::from_utf8_lossy(path));
        });
        for u in &parsed.units {
            if u.unit_type == 2 && u.class >= 573 {
                *high_ids.entry(u.class as u32).or_default() += 1;
            }
        }
        assert!(
            (12..=18).contains(&f.version),
            "{}: version {}",
            String::from_utf8_lossy(path),
            f.version
        );
    }
    println!("versions {versions:?}; unit types {kinds:?}; object ids ≥ 573 {high_ids:?}");
    assert_eq!(
        kinds,
        BTreeMap::from([(1, 2_267), (2, 14_105)]),
        "preset unit records by type"
    );
    assert_eq!(flagged, [1], "records with flags ≠ 0");
    assert_eq!(
        high_ids,
        BTreeMap::from([(580, 46), (581, 135), (582, 24)]),
        "object ids ≥ 573"
    );

    let sized: Vec<_> = l
        .tables
        .preset
        .defs
        .iter()
        .filter(|d| d.size_x != 0 && d.size_y != 0)
        .collect();
    assert_eq!(sized.len(), 1_054, "rows that set SizeX / SizeY");
    for d in sized {
        for p in d.file.iter().filter(|p| names_file(p)) {
            let f = &ds1[p];
            assert_eq!(
                (f.width, f.height),
                (d.size_x, d.size_y),
                "def {} file {}",
                d.def,
                String::from_utf8_lossy(p)
            );
        }
    }
}

// ---- every Act I level ----------------------------------------------

fn level_types() -> (Arc<DrlgData>, SharedTypes) {
    let l = live();
    let data = Arc::new(l.tables.drlg.clone());
    let types = SharedTypes::new(WorldTypes::new(
        data.clone(),
        Maze::new(l.tables.maze.clone()),
        l.tables.preset.clone(),
        l.tables.outdoor.clone(),
        Box::new(l.files.ds1.clone()),
        Box::new(l.files.subs.clone()),
    ));
    (data, types)
}

fn type_errors(types: &SharedTypes) -> Vec<String> {
    types
        .borrow()
        .errors
        .iter()
        .map(|e| format!("{e:?}"))
        .collect()
}

/// Act 0 created on the fixed seed (`dwStartSeed`, DRLG
/// seed and level list), then
/// every level whose `levels` row is in act 0 generated through
/// `WorldTypes` (maze, preset, outdoor on the live tables and files) and
/// its first room streamed (its DT1 library loads): no generation or
/// level-type error, at least one room per level.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn every_act1_level_generates() {
    let (data, types) = level_types();
    let mut handle = types.clone();
    let mut drlg = Drlg::create(0, INIT, 0, TOWN, false, &data, &mut handle).expect("act 0");
    assert_eq!(type_errors(&types), Vec::<String>::new(), "creation");
    assert_eq!(drlg.start_seed, START);
    assert_eq!(drlg.seed, Seed::new(1_406_222_081, 1_674_353_446));
    let mut order: Vec<u32> = drlg
        .level_list()
        .into_iter()
        .map(|l| drlg.level(l).id)
        .collect();
    order.reverse();
    assert_eq!(
        order,
        // The whole list of generated levels, in order.
        [4, 3, 2, 1, 17, 39, 26, 7, 6, 27, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        "recorded list"
    );

    let act1: Vec<u32> = rows::<Levels>()
        .iter()
        .enumerate()
        .filter(|&(id, r)| id > 0 && r.act == 0)
        .map(|(id, _)| id as u32)
        .collect();
    assert!(!act1.is_empty());
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let mut counts = Vec::new();
    for &id in &act1 {
        let l = drlg
            .get_or_alloc_level(&data, &mut handle, id)
            .unwrap_or_else(|e| panic!("allocate level {id}: {e:?}"));
        if drlg.level_rooms(l).is_empty() {
            drlg.generate_level(&data, &mut handle, l)
                .unwrap_or_else(|e| panic!("generate level {id}: {e:?}"));
        }
        assert_eq!(type_errors(&types), Vec::<String>::new(), "level {id}");
        let rooms = drlg.level_rooms(l);
        assert!(!rooms.is_empty(), "level {id}: no room");
        let preset = rooms
            .iter()
            .filter(|&&r| drlg.room(r).kind == RoomKind::Preset)
            .count();
        counts.push((id, rooms.len(), preset));
        let mut svc = Services {
            data: &data,
            tiles: &live().files.dt1,
            types: &mut handle,
            rooms: &mut game.lists,
        };
        drlg.stream_room(&mut svc, rooms[0])
            .unwrap_or_else(|e| panic!("stream level {id}: {e:?}"));
        assert_eq!(type_errors(&types), Vec::<String>::new(), "stream {id}");
    }
    println!("Act I levels (id, rooms, preset rooms): {counts:?}");
}

// ---- a wired game on the live data ---------------------------------------------------

/// Seams with no provider: every default (`Pending`, `WorldPending`).
struct Unprovided;
impl Pending for Unprovided {}
impl WorldPending for Unprovided {}

struct NoRest;
impl VitalsRest for NoRest {
    fn refresh(&mut self, _: UnitId) {}
    fn level_up_notify(&mut self, _: UnitId) {}
    fn level_up_event(&mut self, _: UnitId) {}
}

#[derive(Default)]
struct Sink {
    queued: usize,
}

impl MessageSink for Sink {
    fn queue(&mut self, _: ClientId, _: &[u8]) -> Result<(), QueueError> {
        self.queued += 1;
        Ok(())
    }
}

const GAME_SEED: u32 = 1234;
const SORCERESS: u32 = 1;

/// Game creation on the live tables and files: `WorldSim` (the action
/// systems with the real stat data, unit data, skill / combat / vitals
/// tables and AnimData; the world state with the population and init
/// tables) behind `SimGame`; act 0 created through `WorldTypes` on the
/// fixed seed, the regions on the game seed, the town generated and
/// one room streamed; a Sorceress allocated there with her creation
/// stats joins; 100 ticks pass with no wiring error.
// An integration run: the only expected outcome is "no error".
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_game_on_live_tables_runs_100_ticks() {
    let l = live();
    let (data, types) = level_types();
    let mut handle = types.clone();
    let drlg = Drlg::create(0, INIT, 0, TOWN, false, &data, &mut handle).expect("act 0");
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(drlg);
    let world = DrlgWorld {
        dungeon,
        data,
        tiles: Box::new(l.files.dt1.clone()),
        types: Box::new(handle),
    };
    let tables = ActionTables {
        missiles: rows::<Missiles>(),
        skills: SkillTables::from_bin(&l.bin, LEVEL_CAP_114D).expect("skill tables"),
        combat: CombatTables::from_bin(&l.bin).expect("combat tables"),
        levels: rows::<Levels>(),
        skill_modes: skill_modes(table("monstats")),
        overlay_count: i32::try_from(table("overlay").count).unwrap_or(i32::MAX),
        monequip: Vec::new(),
        arena: Vec::new(),
    };
    let vitals = Arc::new(VitalsTables::from_bin(&l.bin).expect("vitals tables"));
    let mut hooks = ActionHooks::new(
        Arc::new(tables),
        world,
        Seed::init_low(GAME_SEED),
        Unprovided,
    );
    hooks.anim_data = Some(Arc::new(l.anim.clone()));
    hooks.vitals = Some(vitals.clone());

    let (monstats, monstats2) = (rows::<Monstats>(), rows::<Monstats2>());
    let (levels, superuniques) = (rows::<Levels>(), rows::<Superuniques>());
    let wt = WorldTables {
        pop: PopTables::from_records(&levels, &monstats, &monstats2, &superuniques)
            .with_bins(table("monstats"), table("monstats2")),
        monstats,
        monstats2,
        monlvl: rows::<Monlvl>(),
        levels,
        monprop: rows::<Monprop>(),
        monequip: rows::<Monequip>(),
        monumod: rows::<Monumod>(),
        superuniques,
        difficultylevels: rows::<Difficultylevels>(),
        monstats_extra: monstats_extra(table("monstats")),
        components: component_counts(table("monstats2")),
        ids: NamedIds::default(),
        montype_equiv: live().fixed.montype_equiv.clone(),
    };
    let info = GameInfo {
        expansion: true,
        ..GameInfo::default()
    };
    let state = WorldState::new(types, Arc::new(wt), info);
    let states = StateTable::new(table("states"), &l.fixed.states).expect("states");
    let stat_data = Arc::new(
        StatData::new(
            table("itemstatcost"),
            table("charstats"),
            states,
            table("monstats"),
            table("skills"),
        )
        .expect("stat data"),
    );
    let unit_data = UnitData {
        expansion: true,
        ..UnitData::new(table("monstats"), table("monstats2")).expect("unit data")
    };
    let mut sim = WorldSim::new(stat_data, unit_data, hooks, state);
    sim.create_regions();

    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let (room, rect) = sim
        .action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |d, svc| {
            let lv = d.get_or_alloc_level(svc.data, svc.types, TOWN)?;
            if d.level_rooms(lv).is_empty() {
                d.generate_level(svc.data, svc.types, lv)?;
            }
            let first = d.level_rooms(lv)[0];
            let rect = d.room(first).rect;
            Ok::<_, d2_sim::drlg::DrlgError>((d.stream_room(svc, first)?, rect))
        })
        .expect("act 0 has a DRLG")
        .expect("town generated and streamed");
    let room = room.expect("the streamed room is active");
    assert_eq!(sim.errors(), Vec::<String>::new(), "game creation");

    // The player: 20 sub-tiles inside the room's corner (5 sub-tiles per
    // tile).
    let (x, y) = (rect.x * 5 + 20, rect.y * 5 + 20);
    let req = AllocRequest {
        ty: UnitType::Player,
        class: SORCERESS,
        room: Some(room),
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let player = sim
        .action
        .with(&mut game, |g, v| v.allocate(g, &req, x, y))
        .expect("player allocated");
    sim.action.sys.units.get_mut(player).unwrap().mode = 1;
    {
        let s = &mut sim.action.sys;
        let mut v = VitalsView {
            units: &s.units,
            stats: &mut s.stats,
            hooks: &mut NoHost,
            rest: &mut NoRest,
        };
        init_player_stats(&mut v, &vitals, player, 0);
    }
    let life = sim.action.sys.stats.unit_total(player, 6, 0);
    assert_eq!(life, 10240, "Sorceress created (vitals.md Test vectors)");

    let frame0 = game.frame;
    let mut s: SimGame<_, NoWorld> = SimGame::with_events(game, sim);
    let client = 1;
    let sim_client = s
        .join(client, Some(player), None, client_state::IN_GAME)
        .expect("join");
    s.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode: 1,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    let mut sink = Sink::default();
    for _ in 0..100 {
        s.tick(&mut sink);
    }
    assert_eq!(s.game.frame, frame0 + 100, "100 ticks");
    assert_eq!(s.events.errors(), Vec::<String>::new(), "after 100 ticks");
    assert!(
        s.game.lists.unit(player).is_some(),
        "the player is still in the game"
    );
    assert!(
        s.game
            .lists
            .client(sim_client)
            .and_then(|c| c.room)
            .is_some(),
        "the first tick's room change gave the client a room (rooms.md §4.1)"
    );
    // Full life, no regeneration stat: life unchanged.
    assert_eq!(s.events.action.sys.stats.unit_total(player, 6, 0), life);
    println!("frame {}; messages queued: {}", s.game.frame, sink.queued);
}

// ---- the act edge floor records ------------------------

/// The act edge floor record: acts I–III find their key in the base
/// library the loader read (a miss would be a fatal load error); the
/// first entry in reverse file order is a floor (orientation 0).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn act_edge_floor_records_resolve_in_the_base_libraries() {
    use d2_sim::drlg::tiles::{first_entry, ACT_EDGE_TILE};
    use d2_sim::drlg::TileSource;
    for (act, e) in ACT_EDGE_TILE.iter().enumerate() {
        let Some((path, key)) = e else {
            assert!(act >= 3, "acts I–III have an edge record");
            continue;
        };
        let tiles = live()
            .files
            .dt1
            .dt1(path)
            .unwrap_or_else(|| panic!("act {act}: base library not loaded"));
        let i = first_entry(tiles, *key).unwrap_or_else(|| panic!("act {act}: no {key:?}"));
        assert_eq!(tiles[i as usize].orientation, 0, "act {act}");
        eprintln!("act {act}: tile {i} of {} ({:?})", tiles.len(), key);
    }
}
