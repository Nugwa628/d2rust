//! The app's single-player game and its server thread: the link is
//! `Send + Sync` (the game is not, and never leaves its thread), builder
//! failures are reported, the build is deterministic, and the live tables
//! (with `D2_GAME_DIR`, ignored by default) give a waypoint object.

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{
    self, GameData, Link, WaypointTables, ACT2_TOWN, COLD_PLAINS, DEFAULT_SEED,
};
use d2_client::bridge::link::ServerLink;
use d2_client::bridge::local::{LocalLink, PendingSession};
use d2_proto::server::LoadAct;
use d2_proto::{FixedMessage, PROTOCOL_VERSION};
use d2_server::adapters::ProtoSizes;
use d2_server::host::{Host, SystemClock};
use d2_sim::rng::Seed;

use crate::app_support;

fn send_sync<T: Send + Sync>() {}

#[test]
fn the_link_is_send_and_sync() {
    send_sync::<ThreadLink<Link<SystemClock>>>();
    send_sync::<Box<dyn ServerLink + Send + Sync>>();
}

#[test]
fn a_failing_builder_is_an_error() {
    let r = ThreadLink::<Link<SystemClock>>::spawn(|| Err::<Link<SystemClock>, _>("no tables"));
    let e = r.err().expect("refused");
    assert!(e.to_string().contains("no tables"), "{e}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_build_is_deterministic_and_runs_on_its_thread() {
    let seeds = |seed| {
        let mut g = single_player::build(&app_support::game_data(), seed).unwrap();
        let h = g.sim.events.action.hooks();
        (h.game_seed, h.objects.as_ref().map(|o| o.obj_seed))
    };
    assert_eq!(seeds(DEFAULT_SEED), seeds(DEFAULT_SEED));

    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        SystemClock::default(),
    )
    .unwrap();
    assert_eq!(link.protocol_version(), PROTOCOL_VERSION);
    // The game behind the link is the one built: the session flow is set,
    // and no client record or player exists before the client's C→S 0x67
    // / 0x6B.
    let seen = link
        .with(move |l| {
            let sim = &l.host().game;
            (
                sim.session().is_some(),
                sim.sim_client(d2_client::bridge::LOCAL_CLIENT).is_some(),
                single_player::local_player(sim),
            )
        })
        .unwrap();
    assert_eq!(seen, (true, false, None));
    // A pump on the server thread: the first frame starts the host clock.
    assert!(!link.pump().unwrap().ticked);
}

/// A mod game's rules, for the builder test: a name and nothing else.
#[derive(Debug)]
struct TestMod;

impl d2_sim::rules::ModRules for TestMod {
    fn name(&self) -> &str {
        "test-mod"
    }
}

/// A mod game builds through the public builder with its own rules and
/// save folder (a mod seam); the D2RUST defaults
/// build the same game `build` does.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_builder_takes_a_games_rules_and_saves() {
    use single_player::{D2sCodec, GameBuilder, Saves};
    use std::sync::Arc;

    let seeds = |g: &mut single_player::LocalGame| {
        let h = g.sim.events.action.hooks();
        (h.game_seed, h.objects.as_ref().map(|o| o.obj_seed))
    };
    let mut plain = single_player::build(&app_support::game_data(), DEFAULT_SEED).unwrap();
    let mut d2rust = GameBuilder::new(app_support::game_data()).build().unwrap();
    assert!(d2rust.sim.events.action.hooks().rules.is_original());
    assert_eq!(d2rust.saves.folder, Saves::d2rust().folder);
    assert_eq!(seeds(&mut d2rust), seeds(&mut plain));

    let folder = std::env::temp_dir().join(format!("d2rust-ms2-game-{}", std::process::id()));
    let mut modded = GameBuilder::new(app_support::game_data())
        .seed(DEFAULT_SEED)
        .rules(d2_sim::rules::Ruleset::Mod(Arc::new(TestMod)))
        .saves(Saves::new(&folder, Arc::new(D2sCodec)))
        .build()
        .unwrap();
    let name = modded
        .sim
        .events
        .action
        .hooks()
        .rules
        .mod_rules()
        .map(|r| r.name().to_owned());
    assert_eq!(name.as_deref(), Some("test-mod"));
    assert_eq!(modded.saves.file("Hero"), folder.join("Hero.d2s"));
    // No seam reads these rules yet: the game is D2RUST's.
    assert_eq!(seeds(&mut modded), seeds(&mut plain));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_link_built_elsewhere_runs_unchanged() {
    // Any `ServerLink` can live on the thread: here the bare local link.
    let mut link = ThreadLink::spawn(|| {
        let g = single_player::build(&app_support::game_data(), DEFAULT_SEED)?;
        Ok::<_, single_player::BuildError>(LocalLink::new(Host::new(
            g.sim,
            ProtoSizes,
            PendingSession::default(),
            SystemClock::default(),
        )))
    })
    .unwrap();
    assert!(link.receive().is_empty());
}

/// The host clock, advanced by the test.
struct StepClock(std::sync::Arc<std::sync::atomic::AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The app's session sequence on the link: the
/// client's 0x67 (`single_player::create_request`) is drained in the
/// first frame and answered with 0x01, 0x00, 0x02 at tick 1's flush; the
/// loader runs only at the 0x6B: it creates a player of the request's
/// class (sorceress), knowing only the town waypoint (B034), with its player
/// fields; the join's 0x59 … 0x7E and 0x04 follow with tick 2's flush.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_session_flow_creates_the_game_then_loads_the_character_at_the_join() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let ids = |chunks: Vec<Vec<u8>>| -> Vec<u8> { chunks.iter().map(|c| c[0]).collect() };
    let req = single_player::create_request();
    assert_eq!(req.encode()[0], 0x67);
    link.send(SendQueue::System, &req.encode()).unwrap();
    assert!(!link.pump().unwrap().ticked);
    assert!(link.receive().is_empty());
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    assert_eq!(ids(link.receive()), [0x01, 0x00, 0x02]);
    let before = link
        .with(|l| {
            let sim = &l.host().game;
            (
                sim.sim_client(d2_client::bridge::LOCAL_CLIENT).is_some(),
                single_player::local_player(sim),
            )
        })
        .unwrap();
    assert_eq!(before, (true, None), "the character loads at the join");

    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    let chunks = link.receive();
    // S→C 0x03 carries game +0x80, the object control's `dwObjSeed`.
    let load = chunks
        .iter()
        .find(|c| c[0] == LoadAct::ID)
        .map(|c| LoadAct::decode(c).unwrap())
        .expect("0x03 at the join");
    let obj_seed = link
        .with(|l| {
            l.host_mut()
                .game
                .events
                .action
                .hooks()
                .objects
                .as_ref()
                .map(|o| o.obj_seed)
        })
        .unwrap();
    assert_eq!(Some(load.f8), obj_seed);
    assert_ne!(load.f8, 0);
    let got = ids(chunks);
    assert_eq!(got.first(), Some(&0x59), "{got:02X?}");
    // 0x04, then the state-3 inventory refresh and the join sequence
    // (as a known join shows at frame 2: "0x04, 0x48, 0x5B, 0x65, 0x8D,
    // 0x5A").
    assert_eq!(
        got[got.len() - 6..],
        [0x04, 0x48, 0x5B, 0x65, 0x8D, 0x5A],
        "{got:02X?}"
    );
    // A new sorceress has its player record: 0x5F after 0x0B and three
    // 0x23, as a known join of a character made in the create screen
    // shows: the load's own
    // `StartSkill` select before 0x0B (the install has the vitals tables),
    // then the two hands after 0x5F.
    assert!(got.windows(2).any(|w| w == [0x0B, 0x5F]), "{got:02X?}");
    assert_eq!(got.iter().filter(|&&i| i == 0x23).count(), 3, "{got:02X?}");
    let pos = |id: u8| got.iter().position(|&i| i == id).unwrap();
    assert!(pos(0x23) < pos(0x0B), "the select's 0x23 comes first");
    let (class, fields, knows, faults, log) = link
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(sim).expect("joined");
            let class = sim.events.action.sys.units.get(p).map(|u| u.class);
            let fields = sim.player_fields(p).is_some();
            let faults = sim.session().map(|f| f.faults.len());
            let h = sim.events.action.hooks();
            // B034: the town's bit 0 only, not Cold Plains' (index 1).
            let knows = h.waypoints.get_mut(&p).map(|r| {
                (
                    r.get_mut(0).test(0).unwrap_or(false),
                    r.get_mut(0).test(1).unwrap_or(false),
                )
            });
            (class, fields, knows, faults, h.x.log.clone())
        })
        .unwrap();
    assert_eq!(class, Some(single_player::PLAYER_CLASS));
    assert!(fields);
    assert_eq!(
        knows,
        Some((true, false)),
        "a new character knows the town waypoint, not Cold Plains (index 1)"
    );
    assert_eq!(faults, Some(0));
    // The new character is the stub load: its steps without a provider in the
    // synthetic game are named, nothing else is logged.
    let steps: Vec<&str> = log
        .iter()
        .map(|l| {
            l.strip_prefix("join: new character: Unapplied { step: \"")
                .and_then(|r| r.split('"').next())
                .unwrap_or(l)
        })
        .collect();
    assert_eq!(
        steps,
        [
            // On the install the `skills` rows exist, so the player
            // skills are applied; `StartSkill` comes with the start items'
            // stat 107, which has no provider ("has skill", as
            // `synthetic_game.rs`). The one-rows 0x23 reading is the
            // recorded join's (above).
            "new character set-up",
            "has skill",
            "mouse skills",
            "quest entry"
        ]
    );
}

/// The mutation check for the test above: the same 0x67 without flag bits 1 and 2 is
/// refused by the server's checks: no client record, nothing
/// sent, a 0x6B after it finds no record and no player is loaded.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_refused_create_request_starts_nothing() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let mut req = single_player::create_request();
    req.flags &= !0x6;
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    assert!(link.receive().is_empty());
    let (client, player, faults) = link
        .with(|l| {
            let sim = &l.host().game;
            (
                sim.sim_client(d2_client::bridge::LOCAL_CLIENT).is_some(),
                single_player::local_player(sim),
                sim.session().map(|f| f.faults.len()),
            )
        })
        .unwrap();
    assert_eq!((client, player, faults), (false, None, Some(2)));
}

/// The waypoint object rule on the user's own tables: the chosen
/// waypoint object has operate function 23 and init function 17, and no
/// earlier row has both.
#[test]
#[ignore = "needs the game files in D2_GAME_DIR"]
fn live_tables_give_a_waypoint_object() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let archives = d2_formats::mpq::ArchiveSet::open_dir(&dir).unwrap();
    let t = WaypointTables::live(&archives).unwrap();
    let class = t.object_class as usize;
    assert_eq!(
        (t.objects[class].operatefn, t.objects[class].initfn),
        (23, 17)
    );
    assert!(t.objects[..class]
        .iter()
        .all(|o| !(o.operatefn == 23 && o.initfn == 17)));
}

/// The client skill rows from the user's
/// `skills` table: one per row, in row order, with `maxlvl`, `anim`,
/// `monanim`, `passivestate` read as the typed record has them.
/// `D2_GAME_DIR=<install> cargo test -p d2-client --test app_single_player -- --ignored client_skill_rows`
#[test]
#[ignore = "needs the game files in D2_GAME_DIR"]
fn client_skill_rows_from_the_install() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let archives = d2_formats::mpq::ArchiveSet::open_dir(&dir).unwrap();
    let rows = single_player::client_skill_rows(&archives).unwrap();
    let set = d2_data::bin::load(&archives, "eng").unwrap();
    let skills: Vec<d2_data::tables::Skills> =
        d2_data::tables::decode_all(set.table("skills").unwrap()).unwrap();
    assert_eq!(rows.len(), skills.len());
    for (r, s) in rows.iter().zip(&skills) {
        assert_eq!(
            (r.anim, r.monanim, r.passivestate, r.maxlvl),
            (s.anim, s.monanim, s.passivestate, s.maxlvl)
        );
    }
    let passive = rows.iter().filter(|r| r.passivestate as i16 > 0).count();
    eprintln!("{} skills rows, {passive} with a passive state", rows.len());
}

/// The game on the user's files (`GameData::select`): levels generated
/// through drlg-data's providers and the level-type dispatcher. Act 0
/// holds the town (level 1, generated at act creation) and Cold Plains,
/// act 1 Lut Gholein (its town), each with rooms; the waypoint world is
/// set.
#[test]
#[ignore = "needs the game files in D2_GAME_DIR"]
fn live_data_generates_the_levels_from_the_users_files() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let data = GameData::select(Some(dir.as_ref())).unwrap();
    assert!(matches!(data, GameData::Live(_)));
    let mut g = single_player::build(&data, DEFAULT_SEED).unwrap();
    assert!(g.sim.world.action.waypoints.is_some());
    let dungeon = &g.sim.events.action.hooks().drlg.dungeon;
    for (act, level) in [(0, 1), (0, COLD_PLAINS), (1, ACT2_TOWN)] {
        let d = dungeon.acts[act].as_ref().expect("act created");
        let l = d.find_level(level).expect("level allocated");
        let rooms = d.level_rooms(l).len();
        assert!(rooms > 0, "level {level}: no rooms");
        eprintln!(
            "act {act} level {level}: {rooms} rooms, rect {:?}",
            d.level(l).rect
        );
    }
}

/// Without a game directory there is no game (no invented fallback,
/// ever); a directory that does not load is an error naming it.
#[test]
fn data_selection_needs_the_game_files() {
    let e = GameData::select(None).unwrap_err();
    assert!(e.to_string().contains("D2_GAME_DIR"), "{e}");
    let missing = std::path::Path::new("/nonexistent/d2rust-no-game-dir");
    let e = GameData::select(Some(missing)).unwrap_err();
    assert!(e.to_string().contains("d2rust-no-game-dir"), "{e}");
}

/// Game creation at build: on the `--seed` game seed
/// `{N, 666}` (unstepped, the fixed-seed branch), the regions, the object
/// control (`dwObjSeed` = the second step's lo'),
/// the NPC control and the quest control each take one step, before any
/// unit. The build allocates no unit itself: the towns' units are their
/// presets, placed by the room population after the join. The controls
/// are the wired host's; the user's files give the drop state.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn game_creation_derives_the_four_controls_in_order_before_the_first_unit() {
    let mut g = single_player::build(&app_support::game_data(), DEFAULT_SEED).unwrap();
    let mut want = Seed::init_low(DEFAULT_SEED);
    want.step();
    let obj_seed = want.step();
    want.step();
    want.step();
    let h = g.sim.events.action.hooks();
    assert_eq!(h.game_seed, want);
    assert_eq!(h.objects.as_ref().map(|o| o.obj_seed), Some(obj_seed));
    assert!(h.object_drops.is_some());
    assert_eq!(h.uniques, Default::default());
    let w = &g.sim.world;
    assert!(w.quests.record(1).is_some(), "the quest control's records");
    assert!(w.state.hireling_tables.is_some());
    // Another seed gives another object seed.
    let mut other = single_player::build(&app_support::game_data(), DEFAULT_SEED + 1).unwrap();
    let o = other
        .sim
        .events
        .action
        .hooks()
        .objects
        .as_ref()
        .unwrap()
        .obj_seed;
    assert_ne!(o, obj_seed);
}

/// `--save` needs the user's tables: the synthetic data refuses it
/// (no fallback), and the character name of a save is its header's
/// (+0x14, up to the NUL; +0x23 not read).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_save_needs_the_users_tables_and_names_its_character() {
    let e =
        single_player::load_character(&app_support::game_data(), "x.d2s".as_ref(), 0).unwrap_err();
    assert!(e.to_string().contains("x.d2s"), "{e}");
    let mut b = vec![0u8; 0x14F];
    b[0x14..0x18].copy_from_slice(b"Kara");
    assert_eq!(single_player::save_name(&b), b"Kara");
    b[0x14..0x23].fill(b'a');
    b[0x23] = b'z';
    assert_eq!(single_player::save_name(&b), &[b'a'; 15][..]);
    assert_eq!(single_player::save_name(&b[..0x20]), b"");
}

/// The game on the user's files: game creation's NPC control holds the
/// `interact` NPCs' records, the chest drop's state and the hireling
/// tables are installed, and the join runs.
#[test]
#[ignore = "needs the game files in D2_GAME_DIR"]
fn live_game_creation_installs_the_drops_and_the_hireling_tables() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let data = GameData::select(Some(dir.as_ref())).unwrap();
    let mut g = single_player::build(&data, DEFAULT_SEED).unwrap();
    let h = g.sim.events.action.hooks();
    assert!(h.object_drops.is_some());
    assert!(h.objects.is_some());
    let w = &g.sim.world;
    assert!(w.state.hireling_tables.is_some());
    assert!(!w.npc.records.is_empty(), "the interact NPCs' records");
    assert_eq!(w.state.vendors.len(), w.npc.records.len());
}

/// `--save`: a save read with the user's tables joins (the class and
/// name of its header in the 0x67, the load at the 0x6B). The save is
/// `$D2_SAVE` (an expansion, softcore character saved on Normal).
#[test]
#[ignore = "needs the game files in D2_GAME_DIR and a save in D2_SAVE"]
fn a_save_from_the_command_line_joins() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let save = std::env::var("D2_SAVE").expect("D2_SAVE");
    let data = GameData::select(Some(dir.as_ref())).unwrap();
    let character = single_player::load_character(&data, save.as_ref(), 0).unwrap();
    let req = single_player::create_request_for(&character);
    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) =
        single_player::start_with(data, DEFAULT_SEED, character, StepClock(ms.clone())).unwrap();
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    let (player, faults, log) = link
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let p = single_player::local_player(sim);
            let faults = sim.session().map(|f| format!("{:?}", f.faults));
            (p, faults, sim.events.action.hooks().x.log.clone())
        })
        .unwrap();
    eprintln!("load log: {log:#?}");
    assert!(player.is_some(), "joined; faults {faults:?}");
}

/// The app's C→S 0x67 bytes: the known single-player layout for an
/// expansion character; a classic save sends bit 2 alone (as the game
/// does).
#[test]
fn the_create_request_has_the_builder_layout() {
    use d2_client::app::single_player::{Character, CREATE_FLAGS_CLASSIC};
    use d2_server::adapters::character::LoadContext;
    let b = single_player::create_request().encode();
    assert_eq!(b.len(), 46);
    assert_eq!(b[0], 0x67);
    assert_eq!(b[1..0x11], [0; 16], "empty game name");
    assert_eq!(b[0x11], 3, "game type 3");
    assert_eq!(b[0x12], single_player::PLAYER_CLASS as u8);
    assert_eq!((b[0x13], b[0x14]), (0, 0), "template, Normal");
    let mut name = [0u8; 16];
    name[..single_player::PLAYER_NAME.len()].copy_from_slice(single_player::PLAYER_NAME);
    assert_eq!(b[0x15..0x25], name);
    assert_eq!(b[0x25..0x27], [0, 0]);
    assert_eq!(b[0x27..0x2B], 0x0010_0004u32.to_le_bytes());
    assert_eq!(b[0x2B..0x2E], [0, 0, 0]);
    // A save's name, class byte 0 (the game leaves the builder's
    // class at 0 for an existing character; the server reads the save's
    // class); its status bit 5 picks the flags.
    for (status, flags) in [(0x20u16, 0x0010_0004u32), (0, CREATE_FLAGS_CLASSIC)] {
        let save = d2_formats::d2s::D2s::new_stub(b"Necro", 2, status, 1).unwrap();
        let r = single_player::create_request_for(&Character::Save(
            Box::new(save),
            LoadContext::default(),
        ));
        assert_eq!((r.class, &r.char_name[..6]), (0, &b"Necro\0"[..]));
        assert_eq!(r.flags, flags, "status {status:#x}");
    }
}

/// The join on the play path, up to the first tick's flush: the C→S 0x67
/// and 0x6B of the app, two ticks; returns the second flush's message
/// ids, in order.
fn join_ids() -> Vec<Vec<u8>> {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let req = single_player::create_request();
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    link.receive()
}

/// The first tick after the join: the
/// per-client update sends the player's stat messages (the changed-stat
/// array, still holding the join's stats) after the game entry's 0x7E
/// and before the 0x04, not after the tick; then the flag-ex bit 21
/// inventory refresh.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_first_tick_sends_the_stats_in_the_client_pass_before_0x04() {
    let got = join_ids();
    let ids: Vec<u8> = got.iter().map(|m| m[0]).collect();
    let entry = ids.iter().position(|&i| i == 0x7E).expect("0x7E");
    let done = ids.iter().position(|&i| i == 0x04).expect("0x04");
    let stats = ids[entry..done]
        .iter()
        .filter(|&&i| matches!(i, 0x1D..=0x1F))
        .count();
    assert!(stats > 0, "{:02X?}", &ids[entry..]);
    // The join's item messages set flag-ex bit 21: the inventory refresh's
    // 0x48 follows the stats, right before 0x04 (recorded `-022633`
    // frame 2: units, 0x1D / 0x1E, 0x48, 0x04).
    assert_eq!(ids[done - 1], 0x48, "{:02X?}", &ids[entry..]);
    assert!(
        matches!(ids[done - 2], 0x1D..=0x1F),
        "{:02X?}",
        &ids[entry..]
    );
    assert!(
        !ids[done..].iter().any(|&i| matches!(i, 0x1D..=0x1F)),
        "{:02X?}",
        &ids[done..]
    );
}

/// A death's messages reach the same tick's client pass
/// (unit work runs inside step 4, before step 5): in the tick a player's
/// death starts, its S→C 0x0D code 8 (DT) comes before that tick's per-client
/// update messages (here the stat flush of a level change made in the
/// same tick), not after the whole tick.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_death_reaches_the_client_pass_of_its_tick() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    link.send(SendQueue::System, &single_player::create_request().encode())
        .unwrap();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    for _ in 0..4 {
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        link.receive();
    }
    let guid = link
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(sim).expect("joined");
            sim.events.action.start_death(&mut sim.game, p);
            let s = &mut sim.events.action.sys;
            let l = s.stats.unit_list(p).unwrap();
            s.stats.set(&mut s.hooks, l, 12, 7, 0, Some(p));
            s.units.get(p).unwrap().guid
        })
        .unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    let got = link.receive();
    let g = guid.to_le_bytes();
    let dt = got
        .iter()
        .position(|m| m.len() > 6 && m[0] == 0x0D && m[2..6] == g && m[6] == 8)
        .unwrap_or_else(|| panic!("0x0D code 8 in {got:02X?}"));
    let stat = got
        .iter()
        .position(|m| m[..] == [0x1D, 12, 7])
        .unwrap_or_else(|| panic!("the level's 0x1D in {got:02X?}"));
    assert!(dt < stat, "{got:02X?}");
}

/// The act change on the play path: NPC travel to Lut Gholein (level 40, tile
/// 0) runs inside the tick's step-4 work; that tick's flush has 0x05,
/// then 0x03, then 0x53, the new rooms (0x07) and the player's 0x15,
/// no 0x04, no re-add (0x59 / 0x0B), and the client is in state 5. A
/// later tick's client pass, the new room ready, sends 0x04 then the
/// inventory refresh's 0x48 (no join sequence) and the state is 4.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn an_act_change_goes_through_state_5_and_the_client_pass_sends_0x04() {
    use d2_client::bridge::link::SendQueue;
    use d2_sim::units::lists::client_state;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    link.send(SendQueue::System, &single_player::create_request().encode())
        .unwrap();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    for _ in 0..4 {
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        link.receive();
    }
    let guid = link
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, g) = single_player::local_player(sim).expect("joined");
            sim.events.action.hooks().act_changes.push((p, 40, 0));
            g
        })
        .unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    let got = link.receive();
    let ids: Vec<u8> = got.iter().map(|m| m[0]).collect();
    let at = |id: u8| ids.iter().position(|&i| i == id);
    let (unload, load, env) = (at(0x05), at(0x03), at(0x53));
    assert!(
        unload.is_some() && load.is_some() && env.is_some(),
        "{ids:02X?}"
    );
    assert!(unload < load && load < env, "0x05, 0x03, 0x53: {ids:02X?}");
    assert!(at(0x07) > env, "the new rooms after 0x53: {ids:02X?}");
    let g = guid.to_le_bytes();
    let placed = got
        .iter()
        .position(|m| m[0] == 0x15 && m[2..6] == g)
        .expect("the player's 0x15");
    assert!(Some(placed) > env, "{ids:02X?}");
    assert_eq!(at(0x04), None, "no 0x04 from the act change: {ids:02X?}");
    assert!(
        at(0x59).is_none() && at(0x0B).is_none(),
        "no re-add: {ids:02X?}"
    );
    assert_eq!(
        link.with(client_state_of).unwrap(),
        client_state::CHANGING_ACT
    );
    let mut done = None;
    for tick in 0..10 {
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        let ids: Vec<u8> = link.receive().iter().map(|m| m[0]).collect();
        if let Some(i) = ids.iter().position(|&i| i == 0x04) {
            assert_eq!(ids.get(i + 1), Some(&0x48), "{ids:02X?}");
            assert!(!ids.contains(&0x5B), "no join sequence: {ids:02X?}");
            done = Some(tick);
            break;
        }
    }
    assert!(done.is_some(), "0x04 from a later client pass");
    assert_eq!(link.with(client_state_of).unwrap(), client_state::IN_GAME);
}

/// The local client's state (client +0x04).
fn client_state_of(l: &mut single_player::Link<StepClock>) -> u32 {
    let sim = &l.host().game;
    let c = sim.sim_client(d2_client::bridge::LOCAL_CLIENT).unwrap();
    sim.game.lists.client(c).unwrap().state
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_join_sends_the_quest_entry_messages_before_the_player_record() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    link.send(SendQueue::System, &single_player::create_request().encode())
        .unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    let chunks = link.receive();
    let got: Vec<u8> = chunks.iter().map(|c| c[0]).collect();
    // A new character: the stub load's mode 1 and the caller's mode 0
    // each send 0x5E, 0x28 (type 6, GUID 0) and 0x29.
    let first = |id: u8| got.iter().position(|&i| i == id);
    let (q5e, q28, q29, h0b) = (first(0x5E), first(0x28), first(0x29), first(0x0B));
    assert!(
        q5e.is_some() && q28.is_some() && q29.is_some(),
        "{got:02X?}"
    );
    assert!(q5e < q28 && q28 < q29 && q29 < h0b, "{got:02X?}");
    for id in [0x5E, 0x28, 0x29] {
        assert_eq!(got.iter().filter(|&&i| i == id).count(), 2, "{id:#X}");
    }
    let q28 = chunks.iter().find(|c| c[0] == 0x28).unwrap();
    assert_eq!((q28[1], &q28[2..6]), (6, &[0u8, 0, 0, 0][..]));
    assert_eq!(q28.len(), 103);
}

/// A save standing in the town of Act III, IV or V joins: the server
/// builds the act's slot at the join (only acts I and II exist from the
/// game's creation), so the join runs
/// on to the player's placement instead of stopping after S→C 0x03
/// (the `a3`..`a5-town-*` scenes drew no world before).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_save_in_act_three_to_five_is_placed_at_the_join() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    for act in 2u8..=4 {
        // The save of `prepare_scene_chars.sh`'s SceAct3..5: the act
        // transitions done, standing in the act's town.
        let dir = std::env::temp_dir().join(format!("d2rust-far-act-{}-{act}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Far.d2s");
        let args: Vec<String> = [
            "new",
            "--name",
            "Far",
            "--class",
            "sor",
            "--expansion",
            "--map-seed",
            "1",
            "--time",
            "1700000000",
            "--waypoints",
            "all",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([
            "--quests".into(),
            format!("acts={act}"),
            "--act".into(),
            act.to_string(),
            "-o".into(),
            path.display().to_string(),
        ])
        .collect();
        assert_eq!(d2s_tool::cli::run(&args, &mut std::io::sink()).unwrap(), 0);
        let data = app_support::game_data();
        let character = single_player::load_character(&data, &path, 0).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let ms = std::sync::Arc::new(AtomicU32::new(1000));
        let (mut link, _) =
            single_player::start_with(data, DEFAULT_SEED, character.clone(), StepClock(ms.clone()))
                .unwrap();
        let req = single_player::create_request_for(&character);
        link.send(SendQueue::System, &req.encode()).unwrap();
        link.pump().unwrap();
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        link.receive();
        link.send(SendQueue::System, &[0x6B]).unwrap();
        ms.fetch_add(40, Ordering::SeqCst);
        assert!(link.pump().unwrap().ticked);
        let chunks = link.receive();
        let load = chunks
            .iter()
            .find(|c| c[0] == LoadAct::ID)
            .map(|c| LoadAct::decode(c).unwrap())
            .expect("0x03 at the join");
        assert_eq!(load.act, act, "act {act}");
        let ids: Vec<u8> = chunks.iter().map(|c| c[0]).collect();
        assert!(ids.contains(&0x04), "act {act}: {ids:02X?}");
        let placed = link
            .with(|l| single_player::local_player(&l.host().game).is_some())
            .unwrap();
        assert!(placed, "act {act}: no player after the join");
    }
}

/// A saved character's hitpoints and mana are the stored values after the
/// items are placed: the item bonuses raise the maximum, not the current
/// value (as the game does with the same save, frame 2: mana 14984 of
/// 25856, not the max-rescaled 25856).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_loaded_characters_mana_ignores_the_items_bonuses() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let dir = std::env::temp_dir().join(format!("d2rust-load-mana-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Mana.d2s");
    let args: Vec<String> = [
        "new",
        "--name",
        "Mana",
        "--class",
        "ama",
        "--expansion",
        "--level",
        "30",
        "--item",
        "cap/body=1/q=magic",
        "--item",
        "amu/body=2/q=rare",
        "--item",
        "rin/body=6/q=magic",
        "--item",
        "rin/body=7/q=rare",
        "-o",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([path.display().to_string()])
    .collect();
    assert_eq!(d2s_tool::cli::run(&args, &mut std::io::sink()).unwrap(), 0);
    let data = app_support::game_data();
    let character = single_player::load_character(&data, &path, 0).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) =
        single_player::start_with(data, DEFAULT_SEED, character.clone(), StepClock(ms.clone()))
            .unwrap();
    let req = single_player::create_request_for(&character);
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    for _ in 0..4 {
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        link.receive();
    }
    let (mana, max) = link
        .with(|l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).expect("joined");
            let s = &g.events.action.sys.stats;
            (s.unit_total(p, 8, 0), s.unit_total(p, 9, 0))
        })
        .unwrap();
    assert!(max > 14976 + 256, "the items raise the maximum: {max}");
    // The stored 14976 plus a few ticks of regeneration (the game: 14984 at
    // frame 2), far below the maximum.
    assert!(
        (14976..14976 + 64).contains(&mana),
        "mana {mana} of {max} after the load"
    );
}

/// A saved character's items reach the client as the join's item
/// messages (the update-list pass): after the first stat messages and
/// before the 0x23 pair, in the save's order, and not again after the
/// join sequence (which resets the list). Bytes and order as the game's
/// join of the same save (a reference run, frame
/// 2: the equipped short sword's 0x9D, the two belt potions' 0x9C).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_saved_characters_items_are_the_joins_item_messages() {
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let dir = std::env::temp_dir().join(format!("d2rust-join-items-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("CmbPot.d2s");
    let args: Vec<String> = [
        "new",
        "--name",
        "CmbPot",
        "--class",
        "bar",
        "--expansion",
        "--level",
        "3",
        "--item",
        "ssd/body=4",
        "--item",
        "hp1/belt=0",
        "--item",
        "hp1/belt=4",
        "-o",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([path.display().to_string()])
    .collect();
    assert_eq!(d2s_tool::cli::run(&args, &mut std::io::sink()).unwrap(), 0);
    let data = app_support::game_data();
    let character = single_player::load_character(&data, &path, 0).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) =
        single_player::start_with(data, DEFAULT_SEED, character.clone(), StepClock(ms.clone()))
            .unwrap();
    let req = single_player::create_request_for(&character);
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    let mut got = Vec::new();
    for _ in 0..3 {
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        got.extend(link.receive());
    }
    let hex = |s: &str| -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    };
    let want: Vec<Vec<u8>> = [
        "9d061e05010000000001000000110080006584083037470682818081e13f",
        "9c0e1410020000001000a0006508008006170302",
        "9c0e1410030000001000a0006508088006170302",
    ]
    .iter()
    .map(|s| hex(s))
    .collect();
    let items: Vec<&Vec<u8>> = got.iter().filter(|m| matches!(m[0], 0x9C | 0x9D)).collect();
    assert_eq!(items.len(), want.len(), "item messages: {items:02x?}");
    for (g, w) in items.iter().zip(&want) {
        assert_eq!(&g[..], &w[..]);
    }
    let at = |id: u8| got.iter().position(|m| m[0] == id).unwrap();
    let first_item = got.iter().position(|m| m[0] == 0x9D).unwrap();
    assert!(at(0x5F) < first_item, "after 0x5F and the stats");
    assert!(first_item < at(0x23), "before the 0x23 pair");
    assert!(at(0x23) < at(0x04), "the join's own, before 0x04");
}

/// The in-process game's links answer the server path read the client's
/// single-player copy and its own path step use (a knockback ends when the
/// server's path does): the bare thread link has none, the shared one the
/// joined player's. `autoplay-host`, scenarios and `play --control` answer
/// through the same `save::server_paths_on`.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_shared_links_read_the_server_paths() {
    use d2_client::app::save::{server_paths_on, SharedLink};
    use d2_client::bridge::link::SendQueue;
    use std::sync::atomic::{AtomicU32, Ordering};

    let ms = std::sync::Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    link.send(SendQueue::System, &single_player::create_request().encode())
        .unwrap();
    for _ in 0..2 {
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        link.receive();
    }
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    link.receive();
    assert!(
        link.server_paths().is_empty(),
        "the bare thread link reads none"
    );
    let paths = server_paths_on(&mut link);
    let player = paths.iter().find(|p| p.key.unit_type == 0);
    assert!(player.is_some_and(|p| p.copy.is_some()), "{paths:?}");
    let mut shared = SharedLink::new(link);
    assert_eq!(shared.server_paths().len(), paths.len());
}
