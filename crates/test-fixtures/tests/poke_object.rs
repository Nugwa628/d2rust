//! `poke object` on D2RUST creates the object the way the game's own
//! objects are created (`View::create_object`, the population's and the
//! quests' path): the allocator with the per-kind init
//! (the control record, the InitFn on the control seed) before the add,
//! so the poked object's state equals that of an object the game creates
//! at the same point from the same state; operating a poked chest runs
//! its operate function (it opens, drops through the chest drop).
//!
//! The synthetic install's chest is `objects` row 1 (OperateFn 4, no
//! InitFn); the live one (`#[ignore]`, `D2_GAME_DIR`) is row 5 (InitFn 3,
//! OperateFn 4).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use d2_data::bin;
use d2_formats::mpq::ArchiveSet;
use d2_server::world_data::tables::drop_tables;
use d2_sim::poke::{self, Env, PokeResult};
use d2_sim::rng::Seed;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::economy::{DeathDrops, GameFields};
use d2_sim::world::objects::{Dispatch, ObjectData};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup};
use test_fixtures::{install, synth};

fn synthetic() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("poke-object-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn setup(creation: ActCreation, init_seed: u32) -> Setup {
    Setup {
        creation,
        init_seed,
        town: 1,
        game_seed: 1234,
        class: 3,
        known_waypoints: Vec::new(),
    }
}

/// The object's control record, its unit seed and mode, and the control
/// seed after it.
type Made = (ObjectData, Seed, u32, Seed);

fn made(s: &mut Session, obj: UnitId) -> Made {
    let sim = s.sim();
    let st = sim
        .events
        .action
        .sys
        .hooks
        .objects
        .as_ref()
        .expect("object state");
    let u = sim.events.action.sys.units.get(obj).expect("unit");
    let d = *st.control.data.get(&obj).expect("the control record");
    (d, u.seed, u.mode, st.control.seed)
}

/// A session with the chest drop's state on the action wiring (as the
/// play game holds it).
fn session(d: &GameData, setup: &Setup) -> Session {
    let mut s = Session::new(d, setup);
    let t = drop_tables(&d.fixed).unwrap_or_else(|e| panic!("drop tables: {e}"));
    s.sim().events.action.hooks().object_drops = Some(Box::new(DeathDrops::new(
        Arc::new(t),
        GameFields::new(Seed::init_low(0), false),
    )));
    s
}

/// `poke object <class> @x+2 @y` in one session and the game's own
/// creation at the same point in a second, identical one: the same
/// control record, unit seed, mode and control seed. Then the poked
/// chest operated (the operate entry without an operator, so the
/// range seam is not involved): the dispatch runs operate 4, which opens
/// it. Returns the items in the game before and after the operate.
fn check(d: &GameData, setup: &Setup, class: u32) -> (usize, usize) {
    let mut a = session(d, setup);
    let mut b = session(d, setup);
    let (px, py) = a.pos();
    assert_eq!(b.pos(), (px, py));
    let player = a.player;
    let line = format!("object {class} @x+2 @y");
    let r = {
        let sim = a.sim();
        poke::apply_line(&mut sim.game, &mut sim.events, &Env::new(player), &line)
            .unwrap_or_else(|e| panic!("{line}: {e}"))
    };
    let PokeResult::Ok(Some(guid)) = r else {
        panic!("{line}: {r:?}");
    };
    let poked = a
        .sim()
        .game
        .lists
        .find_unit(UnitType::Object, guid)
        .expect("the poked object");
    let theirs = {
        let bp = b.player;
        let sim = b.sim();
        let room = sim
            .game
            .lists
            .unit(bp)
            .and_then(|e| e.room())
            .expect("the player's room");
        sim.events
            .action
            .with(&mut sim.game, |g, v| {
                v.create_object(g, room, class, px + 2, py, 0)
            })
            .expect("the game's own creation")
    };
    let mine = made(&mut a, poked);
    assert_eq!(mine.0.class, class as u16);
    assert_eq!(mine.0.guid, guid);
    // The owner is set at the end of creation.
    assert_eq!(mine.0.owner, Some(-1));
    assert_eq!(mine, made(&mut b, theirs));
    assert_eq!(
        a.sim().events.action.hooks().path_position(poked),
        (px + 2, py)
    );
    assert_eq!(a.sim().events.errors(), Vec::<String>::new());

    let items = |s: &mut Session| s.sim().game.lists.units_of_type(UnitType::Item).len();
    let before = items(&mut a);
    let r = {
        let sim = a.sim();
        sim.events
            .action
            .with(&mut sim.game, |g, v| v.operate_object(g, None, guid))
    };
    assert!(
        matches!(r, Some((1, Some(Dispatch::Done(_))))),
        "operate: {r:?}"
    );
    // The chest is open (mode 1, or 2 when it breaks).
    assert_ne!(made(&mut a, poked).2, 0, "the chest opened");
    assert_eq!(a.sim().events.errors(), Vec::<String>::new());
    (before, items(&mut a))
}

#[test]
fn a_poked_chest_is_created_and_operates_like_the_games_own() {
    let (before, after) = check(synthetic(), &setup(ActCreation::TownOnly, 0x1234_5678), 1);
    assert!(after >= before);
}

/// The live install: objects row 5 (chest, InitFn 3): the init's draws
/// on the control seed equal the game's own creation's, and opening it
/// drops. Game seed 1: the chest is unlocked and drops one item. (Game
/// seed 1234 gives an unlocked chest that drops nothing: the chest's
/// no-drop roll or the treasure walk's NoDrop, with no error: the same
/// pinned seed gives the same outcome every time.)
#[test]
#[ignore = "needs the user's install in D2_GAME_DIR"]
fn a_poked_chest_on_the_live_install() {
    static L: OnceLock<GameData> = OnceLock::new();
    let d = L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bins = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        GameData::load(bins, &set).unwrap_or_else(|e| panic!("game data: {e}"))
    });
    let mut st = setup(ActCreation::Full, 644_409_375);
    st.game_seed = 1;
    let (before, after) = check(d, &st, 5);
    eprintln!("items before {before}, after {after}");
    assert!(after > before, "the chest dropped");
}
