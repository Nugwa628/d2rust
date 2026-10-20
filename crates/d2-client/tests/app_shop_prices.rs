//! The shop prices an online client computes from its own game data
//! (`app::shop_prices`) against the prices the in-process server
//! publishes in single player (`ShopPrices`, `VendorRest::store_price`),
//! headless on the user's install: the player walks the Rogue
//! Encampment to the NPC (`app_support::approach`), talks (C→S 0x13,
//! the client's 0x2F) and opens the store (C→S 0x38); every item the
//! store shows must carry the same price on both boards, and Charsi's
//! repair-all total too (one worn item damaged first).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::shop_prices::{self, PriceTables, ShopView};
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::items;
use d2_client::bridge::world::{KindData, UnitKey};
use d2_client::bridge::BridgeResource;
use d2_client::ui::original::ShopPrices;
use d2_client::world_view::WorldViewUi;
use d2_server::rest::REPAIR_ALL_KEY;
use d2_server::seams::Clock;
use d2_sim::units::UnitType;
use d2_sim::world::npc::class;

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

/// The play app on the user's install, joined: the app, the server
/// thread and the board the server publishes its prices on.
fn play_app(ms: &Arc<AtomicU32>) -> (App, Server<StepClock>, ShopPrices) {
    let data = app_support::game_data();
    let GameData::Live(live) = data.clone();
    let character = single_player::new_character("sorceress", "Test").unwrap();
    let speeds = single_player::walk_speeds(&data, &character).unwrap();
    let (link, started) = single_player::start_with(
        data,
        single_player::DEFAULT_SEED,
        character.clone(),
        StepClock(ms.clone()),
    )
    .unwrap();
    let server: Server<StepClock> = Arc::new(std::sync::Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    let board = started.prices.clone();
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
    (app, server, board)
}

fn send(app: &mut App, m: &[u8]) {
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send_bytes(m)
        .unwrap();
}

fn store_len(app: &App) -> usize {
    items::store_items(app.world().resource::<BridgeResource>().0.world()).len()
}

/// The client's dialog with NPC `key` is open (its S→C 0x28 arrived; an
/// NPC with a speech to give first shows no menu yet, which the store
/// does not need).
fn talking(app: &App, key: UnitKey) -> bool {
    app.world()
        .resource::<BridgeResource>()
        .0
        .world()
        .interact_npc
        == Some(key)
}

/// Walks to the NPC of `npc`, talks (C→S 0x13: within 50 the NPC stops
/// and waits, within 6 the talk starts; the client answers 0x28 with
/// 0x2F) and asks for its store (0x38 `action`: 1 trade, 2 gamble) until
/// the client holds the store items.
fn open_store(app: &mut App, server: &Server<StepClock>, ms: &AtomicU32, npc: u16, action: u32) {
    let guid = app_support::approach(app, server, ms, 1, &[u32::from(npc)]);
    let key = UnitKey::new(1, guid);
    let mut talk = vec![0x13];
    talk.extend_from_slice(&1u32.to_le_bytes());
    talk.extend_from_slice(&guid.to_le_bytes());
    for _ in 0..10 {
        send(app, &talk);
        step(app, ms, 2);
        app_support::run_to_unit(app, server, ms, key);
        send(app, &talk);
        for _ in 0..25 {
            step(app, ms, 1);
            if talking(app, key) {
                break;
            }
        }
        if talking(app, key) {
            break;
        }
    }
    step(app, ms, 2);
    if !talking(app, key) {
        let at = app_support::server_pos(server);
        let unit = app_support::server_unit(server, 1, &[u32::from(npc)]);
        let log = app_support::with(server, |l| {
            let r = &l.host().game.world.rest.log;
            r[r.len().saturating_sub(30)..].to_vec()
        });
        let w = app.world().resource::<BridgeResource>().0.world();
        panic!(
            "no dialog with NPC {npc}: player {at:?}, npc {unit:?}, interact {:?}, in model {}, rest log {log:#?}",
            w.interact_npc,
            w.units.contains_key(&key)
        );
    }
    let mut m = vec![0x38];
    m.extend_from_slice(&action.to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&0u32.to_le_bytes());
    send(app, &m);
    for _ in 0..100 {
        step(app, ms, 1);
        if store_len(app) > 0 {
            break;
        }
    }
    step(app, ms, 5);
    assert!(store_len(app) > 0, "the NPC {npc} store reached the client");
}

/// The client's prices of the open store against the server's board:
/// every store item (and `repair_all`: the repair-all total) priced the
/// same. The number of items compared.
fn compare(app: &App, board: &ShopPrices, npc: u16, gamble: bool, repair_all: bool) -> usize {
    let tables = PriceTables::live(&app_support::live().tables).unwrap();
    let w = app.world().resource::<BridgeResource>().0.world();
    let quest = app
        .world()
        .non_send::<WorldViewUi>()
        .original
        .as_ref()
        .unwrap()
        .more()
        .client_quest;
    let shop = ShopView {
        npc_class: npc,
        gamble,
    };
    let ours = shop_prices::shop_prices(&tables, w, shop, Some(&quest));
    let mine = |g: u32| ours.iter().find(|&&(k, _)| k == g).map(|&(_, p)| p);
    let shown = items::store_items(w);
    let mut bad = Vec::new();
    for i in &shown {
        let (server, client) = (board.get(i.key.guid), mine(i.key.guid));
        if server.is_none() || server != client {
            let code = i.code.map(|c| String::from_utf8_lossy(&c).into_owned());
            let item = shop_prices::price_item(&tables, w, i.key);
            bad.push(format!(
                "{} {code:?}: server {server:?}, client {client:?}\n  {item:?}",
                i.key.guid
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} store items of NPC {npc} differ:\n{}",
        bad.len(),
        shown.len(),
        bad.join("\n")
    );
    // What the run compared: items by quality, and those whose price
    // reads bonus stats.
    let priced: Vec<_> = shown
        .iter()
        .filter_map(|i| shop_prices::price_item(&tables, w, i.key))
        .collect();
    eprintln!(
        "NPC {npc} gamble {gamble}: {} items match; qualities {:?}; {} with bonuses; repair-all {:?}",
        shown.len(),
        priced.iter().map(|p| p.quality).collect::<Vec<_>>(),
        priced.iter().filter(|p| !p.bonuses.is_empty()).count(),
        board.get(REPAIR_ALL_KEY).filter(|_| repair_all)
    );
    if repair_all {
        let server = board.get(REPAIR_ALL_KEY);
        assert!(server.is_some(), "the server published a repair-all total");
        assert_eq!(mine(REPAIR_ALL_KEY), server, "the repair-all total");
    }
    shown.len()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn akaras_store_prices_match_the_servers() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, server, board) = play_app(&ms);
    open_store(&mut app, &server, &ms, class::AKARA, 1);
    assert!(compare(&app, &board, class::AKARA, false, false) > 0);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn gheeds_store_and_gamble_prices_match_the_servers() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, server, board) = play_app(&ms);
    open_store(&mut app, &server, &ms, class::GHEED, 1);
    assert!(compare(&app, &board, class::GHEED, false, false) > 0);
    let (mut app, server, board) = play_app(&ms);
    open_store(&mut app, &server, &ms, class::GHEED, 2);
    assert!(compare(&app, &board, class::GHEED, true, false) > 0);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn charsis_store_and_repair_all_prices_match_the_servers() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, server, board) = play_app(&ms);
    // One worn item with durability loses half of it, on the server and
    // in the client's model (the server's S→C 0x3E).
    let worn = {
        let w = app.world().resource::<BridgeResource>().0.world();
        items::local_items(w)
            .into_iter()
            .filter(|i| i.mode == items::mode::BODY)
            .find_map(|i| match &w.units.get(&i.key)?.kind {
                KindData::Item(d) => {
                    let base = &d.fields.as_ref()?.base;
                    let max = base.iter().find(|p| p.stat == 73)?.value;
                    (max > 1).then_some((i.key.guid, max / 2))
                }
                _ => None,
            })
            .expect("a worn item with durability")
    };
    let (guid, dur) = worn;
    app_support::with(&server, move |l| {
        let sim = &mut l.host_mut().game;
        let item = sim
            .game
            .lists
            .units_of_type(UnitType::Item)
            .into_iter()
            .find(|&u| sim.game.lists.unit(u).is_some_and(|e| e.guid == guid))
            .expect("the worn item on the server");
        sim.events
            .action
            .with(&mut sim.game, |_, v| v.set_base(item, 72, dur));
    });
    let m = d2_sim::units::messages::update_item_stat(guid, 72, dur, 0);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .receive_chunk(&m[..usize::from(m[1])])
        .unwrap();
    open_store(&mut app, &server, &ms, class::CHARSI, 1);
    assert!(compare(&app, &board, class::CHARSI, false, true) > 0);
    assert!(
        board.get(REPAIR_ALL_KEY).is_some_and(|p| p > 0),
        "the damaged item costs a repair"
    );
}
