//! The quest path of the play client headless on the user's install,
//! wired as `d2-client play` wires it (`add_live_client`): the player
//! walks the Rogue Encampment until the town's preset places Akara
//! (`app_support::approach`), a left click on her walks there and sends
//! C→S 0x13; the server starts the interaction (S→C 0x27, 0x29, 0x28),
//! the client answers C→S 0x2F and the quest message 0x31, and the server
//! starts the Den of Evil (q-fixture-migrate).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::hover;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::mirror::DynLink;
use d2_client::bridge::BridgeResource;
use d2_client::controls::Action;
use d2_client::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
use d2_client::ui::quest_log::IconState;
use d2_client::ui::{ActionId, Point, PointerButton, UiEvent};
use d2_client::world_view::walk::PreviewWalk;
use d2_client::world_view::WorldViewUi;
use d2_server::seams::Clock;

use crate::app_support;

use app_support::{Server, SharedLink};

/// What crossed the link: the C→S messages.
#[derive(Default)]
struct Wire {
    sent: Vec<Vec<u8>>,
}

/// A link that records what crosses it.
struct Recorder {
    inner: DynLink,
    wire: Arc<Mutex<Wire>>,
}

impl ServerLink for Recorder {
    fn protocol_version(&self) -> u32 {
        self.inner.protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.wire.lock().unwrap().sent.push(msg.to_vec());
        self.inner.send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.inner.pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.inner.receive()
    }
}

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

fn queue(app: &mut App, e: UiEvent) {
    app.world_mut()
        .non_send_mut::<WorldViewUi>()
        .queue
        .0
        .push(e);
}

/// The play app on the user's install, joined, beside Akara: the app,
/// the server thread.
fn play_app(ms: &Arc<AtomicU32>, wire: &Arc<Mutex<Wire>>) -> (App, Server<StepClock>) {
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
    let server: Server<StepClock> = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    let link = Recorder {
        inner: Box::new(SharedLink(server.clone())),
        wire: wire.clone(),
    };
    add_live_client(
        &mut app,
        Box::new(link),
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
    let akara = u32::from(d2_sim::world::npc::class::AKARA);
    app_support::approach(&mut app, &server, ms, 1, &[akara]);
    (app, server)
}

/// The screen point of Akara in the model, as the
/// click's camera sees it.
fn akara_on_screen(app: &App) -> (u32, Point) {
    let w = app.world().resource::<BridgeResource>().0.world();
    let (key, u) = w
        .units
        .iter()
        .find(|(k, u)| {
            k.unit_type == 1
                && u.class == u32::from(d2_sim::world::npc::class::AKARA)
                && u.position.is_some()
        })
        .expect("Akara in the model");
    let at = app
        .world()
        .resource::<PreviewWalk>()
        .predict
        .position()
        .expect("predicted player");
    let cam = Camera::new(
        FrameSize::D2RUST,
        OpenMode::NONE,
        moving_to_client(at.0, at.1),
        (0, 0),
    );
    let (x, y) = hover::unit_feet(&cam, u.key.unit_type, u.position.unwrap());
    (key.guid, Point::new(x, y - 20))
}

fn click(app: &mut App, ms: &AtomicU32, at: Point) {
    for button in [true, false] {
        let e = if button {
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            }
        } else {
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            }
        };
        queue(app, e);
    }
    step(app, ms, 2);
}

fn ids(wire: &Arc<Mutex<Wire>>) -> Vec<u8> {
    wire.lock().unwrap().sent.iter().map(|m| m[0]).collect()
}

/// The player's quest record the client holds.
fn client_record(app: &App) -> [u8; 96] {
    let ui = app.world().non_send::<WorldViewUi>();
    ui.original.as_ref().unwrap().more().client_quest
}

/// Talks to Akara: click, walk, C→S 0x13, then the server's reply and the
/// client's 0x2F / 0x31.
fn talk_to_akara(app: &mut App, ms: &AtomicU32, wire: &Arc<Mutex<Wire>>) -> u32 {
    let guid = open_akara_menu(app, ms, wire);
    // Cancel (the last row of the NPC menu box) ends the chat: C→S 0x30.
    let p = app_support::npc_menu_row(app, 2);
    click(app, ms, p);
    step(app, ms, 3);
    guid
}

/// Clicks Akara and waits for her menu (S→C 0x28).
fn open_akara_menu(app: &mut App, ms: &AtomicU32, wire: &Arc<Mutex<Wire>>) -> u32 {
    // The client refuses a repeat interact on the same monster within
    // 200 ms: let an earlier talk's pass.
    step(app, ms, 10);
    let (guid, at) = akara_on_screen(app);
    assert!(
        (0..800).contains(&at.x) && (0..560).contains(&at.y),
        "Akara is on screen: {at:?}"
    );
    let mut want = vec![0x13, 1, 0, 0, 0];
    want.extend_from_slice(&guid.to_le_bytes());
    // This click's 0x13: a new one (an earlier talk's is in the log).
    let sent = |wire: &Arc<Mutex<Wire>>| {
        wire.lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| **m == want)
            .count()
    };
    let before = sent(wire);
    click(app, ms, at);
    for _ in 0..200 {
        step(app, ms, 1);
        if sent(wire) > before {
            break;
        }
    }
    assert!(sent(wire) > before, "C→S 0x13 on Akara: {:?}", ids(wire));
    // The 0x28 and the menu box (built at the next UI frame).
    for _ in 0..50 {
        step(app, ms, 1);
        if app_support::npc_menu_len(app) > 0 {
            break;
        }
    }
    step(app, ms, 2);
    guid
}

/// The C→S 0x31 messages sent so far, as (GUID, message).
fn quest_messages(wire: &Arc<Mutex<Wire>>) -> Vec<(u32, u32)> {
    wire.lock()
        .unwrap()
        .sent
        .iter()
        .filter(|m| m[0] == 0x31)
        .map(|m| {
            (
                u32::from_le_bytes(m[1..5].try_into().unwrap()),
                u32::from_le_bytes(m[5..9].try_into().unwrap()),
            )
        })
        .collect()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn talking_to_akara_twice_starts_the_den_of_evil() {
    let ms = Arc::new(AtomicU32::new(1000));
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut app, _server) = play_app(&ms, &wire);
    assert_eq!(client_record(&app)[2] & 0x04, 0, "Den not started yet");
    // First talk: the server starts the interaction (S→C 0x27 / 0x29 /
    // 0x28), the client answers C→S 0x2F and the quest message 0x31 of
    // the list (Akara's introduction, string 12), then closes the chat.
    let guid = talk_to_akara(&mut app, &ms, &wire);
    let mut chat = vec![0x2F];
    chat.extend_from_slice(&1u32.to_le_bytes());
    chat.extend_from_slice(&guid.to_le_bytes());
    assert!(
        wire.lock().unwrap().sent.contains(&chat),
        "C→S 0x2F: {:?}",
        ids(&wire)
    );
    assert_eq!(quest_messages(&wire), [(guid, 12)]);
    let mut end = vec![0x30];
    end.extend_from_slice(&1u32.to_le_bytes());
    end.extend_from_slice(&guid.to_le_bytes());
    assert!(wire.lock().unwrap().sent.contains(&end), "C→S 0x30");
    // Second talk: the list now carries the Den of Evil (message 64,
    // `quest-messages.tsv`); the server starts the quest on it:
    // slot 1 bit 2 in the record S→C 0x28 sends.
    talk_to_akara(&mut app, &ms, &wire);
    assert_eq!(quest_messages(&wire), [(guid, 12), (guid, 64)]);
}

fn press_q(app: &mut App, ms: &AtomicU32) {
    queue(
        app,
        UiEvent::Action(ActionId(Action::ToggleQuests.index() as u16)),
    );
    step(app, ms, 3);
}

fn log_rows(app: &App) -> Vec<(u8, IconState, u8)> {
    let ui = app.world().non_send::<WorldViewUi>();
    ui.original
        .as_ref()
        .unwrap()
        .quest_rows(0, true)
        .iter()
        .map(|r| (r.row.quest, r.row.icon, r.row.shown))
        .collect()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_quest_log_shows_the_started_den_of_evil() {
    let ms = Arc::new(AtomicU32::new(1000));
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut app, _server) = play_app(&ms, &wire);
    // Before Akara: the log opens, the server answers with every status 0.
    press_q(&mut app, &ms);
    assert!(wire.lock().unwrap().sent.contains(&vec![0x40]), "C→S 0x40");
    {
        let ui = app.world().non_send::<WorldViewUi>();
        assert!(
            ui.original.as_ref().unwrap().is_open(0x0F),
            "quest log open"
        );
    }
    let before = log_rows(&app);
    assert_eq!(before.len(), 6, "six Act I rows: {before:?}");
    assert!(
        before.iter().all(|r| r.2 == 0),
        "nothing started: {before:?}"
    );
    press_q(&mut app, &ms);
    // Akara twice (introduction, then the Den of Evil), then the log.
    talk_to_akara(&mut app, &ms, &wire);
    talk_to_akara(&mut app, &ms, &wire);
    press_q(&mut app, &ms);
    let rows = log_rows(&app);
    let den = rows.iter().find(|r| r.0 == 1).expect("the Den of Evil row");
    assert_eq!(den.2, 1, "status 1 (started): {rows:?}");
    assert_eq!(den.1, IconState::InProgress);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn akaras_menu_offers_talk_trade_and_cancel() {
    let ms = Arc::new(AtomicU32::new(1000));
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut app, _server) = play_app(&ms, &wire);
    let guid = open_akara_menu(&mut app, &ms, &wire);
    let menu = {
        let ui = app.world().non_send::<WorldViewUi>();
        ui.original.as_ref().unwrap().npc_menu()
    };
    let menu = menu.expect("the NPC menu is open after the 0x28");
    assert_eq!(menu.guid, guid);
    let kinds: Vec<_> = menu.rows.iter().map(|r| r.kind).collect();
    use d2_client::ui::layout::OptionKind::{Talk, Trade};
    assert_eq!(kinds, [Some(Talk), Some(Trade), None]);
    // The automatic chat close is gone: the chat stays open.
    assert!(
        !ids(&wire).contains(&0x30),
        "no C→S 0x30 yet: {:?}",
        ids(&wire)
    );
    // Talk closes the box and opens the topic box.
    let p = app_support::npc_menu_row(&app, 0);
    click(&mut app, &ms, p);
    step(&mut app, &ms, 2);
    let ui = app.world().non_send::<WorldViewUi>();
    assert!(ui.original.as_ref().unwrap().npc_menu().unwrap().talking);
    // The topic box's cancel ends the talk: Akara's menu is built again
    // (flag 1).
    let p = app_support::npc_topic_cancel(&app);
    click(&mut app, &ms, p);
    step(&mut app, &ms, 2);
    assert_eq!(app_support::npc_menu_len(&app), 3, "the menu is back");
    // Trade: C→S 0x38 action 1 [GUID] and the menu closes.
    let p = app_support::npc_menu_row(&app, 1);
    click(&mut app, &ms, p);
    step(&mut app, &ms, 3);
    let mut want = vec![0x38, 1, 0, 0, 0];
    want.extend_from_slice(&guid.to_le_bytes());
    want.extend_from_slice(&0u32.to_le_bytes());
    assert!(
        wire.lock().unwrap().sent.contains(&want),
        "C→S 0x38 trade: {:?}",
        ids(&wire)
    );
    let ui = app.world().non_send::<WorldViewUi>();
    assert!(ui.original.as_ref().unwrap().npc_menu().is_none());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn leaving_the_menu_sends_the_chat_end() {
    let ms = Arc::new(AtomicU32::new(1000));
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut app, _server) = play_app(&ms, &wire);
    let guid = talk_to_akara(&mut app, &ms, &wire);
    let mut want = vec![0x30, 1, 0, 0, 0];
    want.extend_from_slice(&guid.to_le_bytes());
    assert!(
        wire.lock().unwrap().sent.contains(&want),
        "C→S 0x30: {:?}",
        ids(&wire)
    );
}
