//! The play client's unit interaction headless on the user's install,
//! wired as `d2-client play --new` wires it (`add_live_client`): the
//! player walks the Rogue Encampment until the town's preset places its
//! waypoint (`app_support::approach`); a left click on the waypoint
//! object picks it as the hover target (D2RUST's own preview pick), walks
//! toward it with the interaction pending and sends C→S 0x13 on arrival;
//! the server answers S→C 0x63, the waypoint menu (UI 0x14) opens with
//! the act's waypoint levels and a row click sends C→S 0x49
//! (q-fixture-migrate).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::hover;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::mirror::DynLink;
use d2_client::bridge::BridgeResource;
use d2_client::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
use d2_client::ui::{Point, PointerButton, UiEvent};
use d2_client::world_view::walk::PreviewWalk;
use d2_client::world_view::{WorldViewState, WorldViewUi};
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

/// The play app on the user's install, joined, beside the town's
/// waypoint: the app, the server thread.
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
    app_support::approach(&mut app, &server, ms, 2, &waypoint_classes());
    (app, server)
}

/// The install's waypoint object classes (`objects` operate function 23).
fn waypoint_classes() -> Vec<u32> {
    app_support::waypoints()
        .waypoint_classes()
        .into_iter()
        .map(u32::from)
        .collect()
}

/// The screen point of the waypoint object's feet, as the click's
/// camera sees it (centred on the predicted player).
fn waypoint_on_screen(app: &App) -> (u32, Point) {
    let w = app.world().resource::<BridgeResource>().0.world();
    let classes = waypoint_classes();
    let (key, u) = w
        .units
        .iter()
        .find(|(k, u)| k.unit_type == 2 && u.position.is_some() && classes.contains(&u.class))
        .expect("the waypoint object in the model");
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

fn waypoint_open(app: &App) -> Option<u32> {
    let ui = app.world().non_send::<WorldViewUi>();
    let o = ui.original.as_ref().unwrap();
    o.msg_state().waypoint.as_ref().map(|w| w.guid)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn clicking_the_waypoint_walks_there_and_interacts() {
    let ms = Arc::new(AtomicU32::new(1000));
    let wire = Arc::new(Mutex::new(Wire::default()));
    let (mut app, server) = play_app(&ms, &wire);
    let (guid, at) = waypoint_on_screen(&app);
    assert!(
        (0..800).contains(&at.x) && (0..560).contains(&at.y),
        "the waypoint is on screen: {at:?}"
    );
    assert_eq!(waypoint_open(&app), None);
    queue(
        &mut app,
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
    );
    queue(
        &mut app,
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    );
    step(&mut app, &ms, 2);
    assert!(
        app.world()
            .resource::<WorldViewState>()
            .interact
            .pending
            .is_some_and(|p| p.target.guid == guid),
        "the click on the waypoint left the interaction pending"
    );
    let mut want = vec![0x13, 2, 0, 0, 0];
    want.extend_from_slice(&guid.to_le_bytes());
    for _ in 0..200 {
        step(&mut app, &ms, 1);
        if wire.lock().unwrap().sent.contains(&want) {
            break;
        }
    }
    let w = wire.lock().unwrap();
    assert!(
        w.sent.contains(&want),
        "C→S 0x13 on the waypoint: {:?}",
        w.sent.iter().map(|m| m[0]).collect::<Vec<_>>()
    );
    // The interaction ran once: the pending record is gone.
    assert!(app
        .world()
        .resource::<WorldViewState>()
        .interact
        .pending
        .is_none());
    drop(w);
    // The server operates the waypoint (the interact range is in range on
    // arrival, as measured) and answers S→C 0x63: the menu
    // (UI 0x14) opens on the waypoint's GUID.
    step(&mut app, &ms, 4);
    assert_eq!(waypoint_open(&app), Some(guid));
    let o = app.world().non_send::<WorldViewUi>();
    let o = o.original.as_ref().unwrap();
    assert!(o.is_open(0x14), "the waypoint menu is open");
    // The rows: the Act I waypoint levels in index order (`levels`
    // `Waypoint`), known as the server's record holds them, the town
    // current.
    let map = app_support::waypoints().map;
    let known = app_support::with(&server, |l| {
        let (p, _) = single_player::local_player(&l.host().game).unwrap();
        let g = &mut l.host_mut().game;
        let wp = g.events.action.hooks().waypoints.entry(p).or_default();
        (0..64u32)
            .map(|i| wp.get_mut(0).test(i).unwrap_or(false))
            .collect::<Vec<bool>>()
    });
    let mut want: Vec<(u8, u16)> = (0..map.level_count())
        .filter(|&lv| map.act(lv) == Some(0))
        .filter_map(|lv| Some((map.index_of_level(lv)?, lv as u16)))
        .collect();
    want.sort();
    let want: Vec<(u16, bool, bool)> = want
        .into_iter()
        .map(|(i, lv)| {
            (
                lv,
                known[usize::from(i)],
                u32::from(lv) == single_player::ACT1_TOWN,
            )
        })
        .collect();
    let rows: Vec<(u16, bool, bool)> = o
        .waypoint_rows()
        .iter()
        .map(|r| (r.level, r.known, r.current))
        .collect();
    assert_eq!(rows, want);
    assert!(
        rows[1].1,
        "row 1 is known (the loader's staging, q-fix-real-known-wp)"
    );
    // A click on row 1 (the menu hit at 800 wide: x' = x − 80,
    // y' = y − 60) sends C→S 0x49 [GUID][level 3] and closes the menu.
    let at = Point::new(80 + 150, 60 + 110);
    for e in [
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    ] {
        queue(&mut app, e);
    }
    step(&mut app, &ms, 2);
    let mut take = vec![0x49];
    take.extend_from_slice(&guid.to_le_bytes());
    take.extend_from_slice(&u32::from(rows[1].0).to_le_bytes());
    assert!(
        wire.lock().unwrap().sent.contains(&take),
        "C→S 0x49 to row 1's level: {:?}",
        wire.lock()
            .unwrap()
            .sent
            .iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
    );
    let ui = app.world().non_send::<WorldViewUi>();
    assert!(!ui.original.as_ref().unwrap().is_open(0x14), "menu closed");
}
