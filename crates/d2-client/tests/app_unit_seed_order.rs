//! The game-seed order of a single-player join on the user's install,
//! against a reference run of the game (seed 1234, a 335-byte
//! Amazon stub, `d2s-tool new-stub --class ama --expansion`). After the
//! four game-creation steps the load draws the player's unit seed, then
//! each start item a unit seed and an item seed, then (after the act's
//! DRLG) the town's objects and monsters one unit seed each.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::save;
use d2_client::app::single_player::{self, Character, GameData};
use d2_server::adapters::character::LoadContext;
use d2_server::seams::Clock;
use d2_sim::rng::Seed;
use d2_sim::units::UnitType;

use crate::app_support;

use app_support::{Server, SharedLink};

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

fn step(app: &mut App, ms: &AtomicU32, n: usize) {
    for _ in 0..n {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
}

/// `d2-client play --seed 1234` with `character`, without a window:
/// joined, with the town's first ticks run.
fn play_app(ms: &Arc<AtomicU32>, character: Character) -> (App, Server<StepClock>) {
    let data = app_support::game_data();
    let GameData::Live(live) = data.clone();
    let speeds = single_player::walk_speeds(&data, &character).unwrap();
    let (link, started) =
        single_player::start_with(data, 1234, character.clone(), StepClock(ms.clone())).unwrap();
    let server: Server<StepClock> = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_live_client(
        &mut app,
        Box::new(SharedLink(server.clone())),
        LiveClient {
            data: &live,
            request: &character,
            start_flags: None,
            prices: started.prices,
            speeds,
            hardcore: false,
            automap_files: None,
            gpu: false,
        },
    )
    .unwrap();
    while app_support::local_player(&server).is_none() {
        step(&mut app, ms, 1);
    }
    step(&mut app, ms, 10);
    (app, server)
}

/// One game-seed consumer: the unit's type, class, and what it drew
/// (`dwInitSeed`; an item's start seed second).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Drawn {
    ty: UnitType,
    class: u32,
    unit: u32,
    item: Option<u32>,
}

/// The player's unit seed: the first game-seed step after the four of
/// game creation (seq 5–2346 of both reference runs: 2972047412,
/// 1542758918, 1961566614, 2016663226), at seq 2349.
const PLAYER: Drawn = Drawn {
    ty: UnitType::Player,
    class: 0,
    unit: 4048349444,
    item: None,
};

/// `--auto ScnAma` (an 848-byte full save with no items, `d2s-tool new
/// --class ama --expansion`): the player, then after the act's DRLG the
/// town's objects and monsters, seq 7270–7408. A monster's seed is drawn
/// by its init, an object's
/// by none or the animation roll; the classes are the
/// install's rows D2RUST gives those seeds (the reference run names none).
fn recorded_full_save() -> Vec<Drawn> {
    use UnitType::{Monster, Object};
    let town = [
        (Object, 37, 108806926),
        (Object, 385, 4040195123),
        (Object, 39, 3329408713),
        (Object, 35, 1870986632),
        (Object, 37, 1508444576),
        (Object, 78, 637663495),
        (Monster, 152, 42126709),
        (Object, 37, 3969005355),
        (Object, 37, 3319447487),
        (Object, 37, 328665009),
        (Object, 119, 2532869550),
        (Monster, 152, 1899002361),
        (Monster, 150, 242740622),
        (Object, 37, 1324584008),
        (Object, 37, 369906869),
        (Monster, 147, 2114917475),
        (Monster, 152, 1222022467),
        (Object, 37, 628920819),
        (Monster, 154, 2303219957),
        (Object, 36, 2782325891),
        (Object, 37, 3892453691),
        (Object, 37, 589455960),
        (Object, 267, 1100743764),
        (Monster, 155, 1864241619),
    ];
    let mut v = vec![PLAYER];
    v.extend(town.iter().map(|&(ty, class, unit)| Drawn {
        ty,
        class,
        unit,
        item: None,
    }));
    v
}

/// `--auto StubAma` (the 335-byte stub, `d2s-tool new-stub --class ama
/// --expansion`): the player, then the eight start items, each a unit
/// seed and an item-seed step, seq 2349–2402, then after
/// the act's DRLG the town's objects and monsters, seq 7323–7440: the
/// same classes as [`recorded_full_save`], each 16 steps later. (Items
/// are compared without their class.)
fn recorded_stub() -> Vec<Drawn> {
    let items = [
        (108806926, 4040195123),
        (3329408713, 1870986632),
        (1508444576, 637663495),
        (42126709, 3969005355),
        (3319447487, 328665009),
        (2532869550, 1899002361),
        (242740622, 1324584008),
        (369906869, 2114917475),
    ];
    let town = [
        1222022467, 628920819, 2303219957, 2782325891, 3892453691, 589455960, 1100743764,
        1864241619, 2931418351, 990265188, 2813298517, 3163509470, 42754933, 2602431189, 980535581,
        3385903807, 3184650063, 367684955, 1282899071,
    ];
    let mut v = vec![PLAYER];
    v.extend(items.iter().map(|&(unit, item)| Drawn {
        ty: UnitType::Item,
        class: 0,
        unit,
        item: Some(item),
    }));
    v.extend(
        recorded_full_save()[1..]
            .iter()
            .zip(town)
            .map(|(d, unit)| Drawn { unit, ..*d }),
    );
    v
}

/// The game's units in the order they took their game-seed steps: each
/// unit's `dwInitSeed` placed by its index in the game seed's chain.
fn consumers(server: &Server<StepClock>) -> Vec<Drawn> {
    app_support::with(server, |l| {
        let s = &l.host().game;
        let mut chain = Seed::init_low(1234);
        let index: std::collections::HashMap<u32, usize> =
            (0..4096).map(|i| (chain.step(), i)).collect();
        let mut out = Vec::new();
        for ty in [
            UnitType::Player,
            UnitType::Monster,
            UnitType::Object,
            UnitType::Item,
        ] {
            for u in s.game.lists.units_of_type(ty) {
                let Some(r) = s.events.action.sys.units.get(u) else {
                    continue;
                };
                let Some(&i) = index.get(&r.init_seed) else {
                    continue;
                };
                let class = if ty == UnitType::Item { 0 } else { r.class };
                let item = r.item_seed.map(|(_, start)| start);
                out.push((
                    i,
                    Drawn {
                        ty,
                        class,
                        unit: r.init_seed,
                        item,
                    },
                ));
            }
        }
        out.sort_by_key(|&(i, _)| i);
        out.into_iter().map(|(_, d)| d).collect::<Vec<_>>()
    })
}

fn assert_prefix(got: &[Drawn], want: &[Drawn]) {
    assert!(got.len() >= want.len(), "{got:#?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(g, w, "game-seed consumer {i}");
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_full_save_join_takes_the_recorded_game_seed_steps_in_order() {
    let ms = Arc::new(AtomicU32::new(1000));
    let new = single_player::new_character("amazon", "ScnAma").unwrap();
    let full = Character::Save(
        Box::new(save::base_save(&new)),
        LoadContext {
            difficulty: 0,
            map_seed_applies: false,
        },
    );
    let (_app, server) = play_app(&ms, full);
    assert_prefix(&consumers(&server), &recorded_full_save());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_new_character_draws_its_seed_before_its_start_items_and_the_town() {
    let ms = Arc::new(AtomicU32::new(1000));
    let new = single_player::new_character("amazon", "StubAma").unwrap();
    let (_app, server) = play_app(&ms, new);
    // No extra start item (this reference run settles it): the town's
    // first object takes the step after the eighth item.
    assert_prefix(&consumers(&server), &recorded_stub());
}
