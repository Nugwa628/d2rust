//! The end-to-end play smoke run (task `q-play-smoke`, moved onto the
//! user's install by q-fixture-migrate): one scripted run through the
//! real play path, headless (the bridge, the in-process server and the
//! sim, the play app's client as `play::add_live_client` wires it for
//! `d2-client play`, no window), on the user's game install
//! (`$D2_GAME_DIR`). Every table, level, tile and UI file is the
//! install's; nothing is invented. The tests are `#[ignore]`: the
//! real-data gate runs them (`tools/realdata-gate.sh`).
//!
//! After every step the run asserts that the server refused no intent
//! (every drained C→S message dispatched with result 0, `seams.rs`
//! `ResultCode::Done`), that the client model dropped, rejected,
//! discarded and left unhandled no S→C message, and no
//! system panicked (Bevy's error handler panics on a bridge error).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, GameData};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::local::{LocalLink, PendingSession};
use d2_client::bridge::world::UnitKey;
use d2_client::bridge::BridgeResource;
use d2_client::controls::Action;
use d2_client::ui::layout::OptionKind;
use d2_client::ui::panel::ActionId;
use d2_client::ui::{Point, PointerButton, UiEvent};
use d2_client::world_view::WorldViewUi;
use d2_server::adapters::ProtoSizes;
use d2_server::dispatch::Outcome;
use d2_server::host::Handled;
use d2_server::host::Host;
use d2_server::seams::{Clock, ResultCode};

use crate::app_support;
use app_support::{Server, SharedLink};

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// What crossed the link: the C→S messages sent and every drained
/// message the server did not accept.
#[derive(Default)]
struct Wire {
    sent: Vec<Vec<u8>>,
    refused: Vec<(u8, Handled)>,
    /// The ids of every S→C message received.
    received: Vec<u8>,
    /// The same chunks, whole (a chunk holds several messages).
    chunks: Vec<Vec<u8>>,
    /// Every drained C→S message's id and fate.
    drained: Vec<(u8, Handled)>,
}

/// The app's link: the shared server thread, recording both directions'
/// fates.
struct Probe {
    server: Server<StepClock>,
    wire: Arc<Mutex<Wire>>,
}

impl ServerLink for Probe {
    fn protocol_version(&self) -> u32 {
        SharedLink(self.server.clone()).protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        SharedLink(self.server.clone()).tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.wire.lock().unwrap().sent.push(msg.to_vec());
        SharedLink(self.server.clone()).send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        let p = SharedLink(self.server.clone()).pump()?;
        let drained = app_support::with(&self.server, |l| l.last_frame().messages.clone());
        let mut w = self.wire.lock().unwrap();
        for m in drained {
            w.drained.push((m.id, m.handled));
            let ok = matches!(
                m.handled,
                Handled::System | Handled::Game(Outcome::Dispatched(ResultCode::Done))
            );
            if !ok {
                w.refused.push((m.id, m.handled));
            }
        }
        Ok(p)
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        let got = SharedLink(self.server.clone()).receive();
        let mut w = self.wire.lock().unwrap();
        w.chunks.extend(got.iter().cloned());
        for c in &got {
            // Chunk ids only (the bridge splits them); enough for a trace.
            if let Some(&id) = c.first() {
                w.received.push(id);
            }
        }
        got
    }
}

/// The run: the app, its server thread, the host clock and the wire.
struct Run {
    app: App,
    server: Server<StepClock>,
    ms: Arc<AtomicU32>,
    wire: Arc<Mutex<Wire>>,
    frames: usize,
    character: single_player::Character,
    /// Divergences met on the way (a run leg drawn away from where the
    /// server's player stopped, a step the server did not carry out):
    /// asserted empty at the end of a run, so one divergence does not
    /// hide the steps after it. Each names its finding row.
    findings: Vec<String>,
}

/// A character's state for the save round trip: (stat, value), (skill,
/// base level), (item code, mode, body location).
/// The waypoint records and quest flag records of the three
/// difficulties.
type Snapshot = (
    Vec<(u16, i32)>,
    Vec<(u16, i32)>,
    Vec<([u8; 4], u8, u8)>,
    Option<[[u8; 16]; 3]>,
    Option<Vec<[u8; 96]>>,
);

impl Run {
    /// The play app on the user's install, as `d2-client play --new
    /// sorceress Smoke` wires it; returns once the join ran.
    fn start() -> Self {
        let character = single_player::new_character("sorceress", "Smoke").unwrap();
        Self::start_as(character)
    }

    /// [`Self::start`] with `character` (a new one, or a save).
    fn start_as(character: single_player::Character) -> Self {
        let data = app_support::game_data();
        let GameData::Live(live) = data.clone();
        let ms = Arc::new(AtomicU32::new(1000));
        let clock = StepClock(ms.clone());
        let built = character.clone();
        let game_data = data.clone();
        // The seed `d2-client play` builds with when no `--seed` is given:
        // a save's map seed, else the default (`game_seed`).
        let seed = single_player::game_seed(&character, None);
        let (tx, rx) = std::sync::mpsc::channel();
        let link = ThreadLink::spawn(move || {
            let g = single_player::build_with(&game_data, seed, built)?;
            let _ = tx.send(g.sim.world.rest.prices.clone());
            Ok::<_, single_player::BuildError>(LocalLink::new(Host::new(
                g.sim,
                ProtoSizes,
                PendingSession::default(),
                clock,
            )))
        })
        .unwrap();
        let prices = rx.recv().expect("the game built");
        let server = Arc::new(Mutex::new(link));
        let wire = Arc::new(Mutex::new(Wire::default()));
        let mut app = App::new();
        app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<ButtonInput<MouseButton>>();
        let probe = Probe {
            server: server.clone(),
            wire: wire.clone(),
        };
        let speeds = single_player::walk_speeds(&data, &character).unwrap();
        assert!(speeds.is_some(), "walk speeds from charstats");
        add_live_client(
            &mut app,
            Box::new(probe),
            LiveClient {
                data: &live,
                request: &character,
                start_flags: None,
                prices,
                speeds,
                hardcore: false,
                automap_files: None,
                gpu: false,
            },
        )
        .unwrap();
        let mut run = Run {
            app,
            server,
            ms,
            wire,
            frames: 0,
            character,
            findings: Vec::new(),
        };
        while app_support::local_player(&run.server).is_none() {
            run.step(1);
        }
        run.step(30);
        run.check("join");
        run
    }

    fn step(&mut self, n: usize) {
        for _ in 0..n {
            self.app.update();
            self.ms.fetch_add(40, Ordering::SeqCst);
            self.frames += 1;
            assert!(self.frames < 100_000, "the run finishes");
        }
    }

    /// Steps until `done` or `limit` frames.
    fn until(&mut self, what: &str, limit: usize, mut done: impl FnMut(&mut Run) -> bool) {
        for _ in 0..limit {
            if done(self) {
                return;
            }
            self.step(1);
        }
        assert!(done(self), "{what}: not reached in {limit} frames");
    }

    fn bridge(&mut self) -> &mut d2_client::bridge::Bridge<d2_client::bridge::mirror::DynLink> {
        &mut self
            .app
            .world_mut()
            .resource_mut::<BridgeResource>()
            .into_inner()
            .0
    }

    fn queue(&mut self, e: UiEvent) {
        self.app
            .world_mut()
            .non_send_mut::<WorldViewUi>()
            .queue
            .0
            .push(e);
    }

    fn click(&mut self, at: Point) {
        for press in [true, false] {
            let button = PointerButton::Left;
            self.queue(if press {
                UiEvent::Press { button, at }
            } else {
                UiEvent::Release { button, at }
            });
        }
        self.step(2);
    }

    /// The town waypoint to Cold Plains: interact, the menu opens, C→S
    /// 0x49, the server player is there.
    fn waypoint_to_cold_plains(&mut self) {
        // A waypoint: an `objects` row with operate function 23,
        // the nearest to the server player.
        let rows = app_support::live().waypoints.objects.clone();
        let (px, py) = self.pos();
        let wp = {
            let w = self.app.world().resource::<BridgeResource>().0.world();
            w.units
                .iter()
                .filter(|(k, u)| {
                    k.unit_type == 2
                        && u.position.is_some()
                        && rows
                            .get(u.class as usize)
                            .is_some_and(|r| r.operatefn == 23)
                })
                .min_by_key(|(_, u)| {
                    let (x, y) = u.position.unwrap();
                    (i32::from(x) - px).abs().max((i32::from(y) - py).abs())
                })
                .map(|(k, _)| *k)
                .expect("the waypoint in the model")
        };
        // Legs toward it first, as for an NPC (`approach`): a run to the
        // unit from afar stops at the first wall, the straight path's
        // A* covers 18 sub-tiles only.
        let at = {
            let w = self.app.world().resource::<BridgeResource>().0.world();
            let (x, y) = w.units[&wp].position.expect("the waypoint's position");
            (i32::from(x), i32::from(y))
        };
        // The torches east of the waypoint (objects class 37, 1 × 1 with
        // collision, on its row) stop a straight leg along it: the toward
        // walk finds no free step around one for the size-2 pattern
        // and A* is out of reach. Go round them 5 rows
        // south, then the legs toward it.
        let p = self.pos();
        let row = at.1 + 5;
        self.leg((p.0, row));
        while (self.pos().0 - at.0).abs() > 10 {
            let x = self.pos().0;
            let to = x + (at.0 - x).clamp(-test_fixtures::host::LEG, test_fixtures::host::LEG);
            self.leg((to, row));
            assert!(
                (self.pos().0 - x).abs() > 2,
                "the leg along y {row} stopped at {:?}",
                self.pos()
            );
        }
        let near = move |r: &mut Run| test_fixtures::host::cheb(r.pos(), at) <= 10;
        assert!(self.run_to(at, near), "the waypoint not reached");
        self.run_to_unit(wp);
        self.bridge().interact(wp).unwrap();
        self.until("the waypoint menu", 400, |r| r.ui_open(0x14));
        self.check("open the waypoint");
        let mut take = vec![0x49];
        take.extend_from_slice(&wp.guid.to_le_bytes());
        take.extend_from_slice(&single_player::COLD_PLAINS.to_le_bytes());
        self.bridge().send_bytes(&take).unwrap();
        self.until("Cold Plains", 400, |r| {
            r.server_level() == Some(single_player::COLD_PLAINS)
        });
        self.step(30);
        self.check("waypoint to Cold Plains");
    }

    fn ui_open(&self, ui: u8) -> bool {
        self.app
            .world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .unwrap()
            .is_open(ui)
    }

    /// The step's checks (module docs).
    fn check(&mut self, step: &str) {
        let w = self.wire.lock().unwrap();
        assert!(
            w.refused.is_empty(),
            "{step}: the server refused {:02X?}",
            w.refused
        );
        // No position resync asked: the server never walks the player
        // to a client guess (no rubber band).
        assert!(
            !w.sent.iter().any(|m| m.first() == Some(&0x5F)),
            "{step}: the client sent C→S 0x5F"
        );
        drop(w);
        let b = &self.app.world().resource::<BridgeResource>().0;
        let log = b.log();
        assert!(
            log.unowned.is_empty(),
            "{step}: unhandled {:02X?}",
            log.unowned
        );
        assert!(
            log.dropped.is_empty(),
            "{step}: dropped {:02X?}",
            log.dropped
        );
        assert!(
            log.rejected.is_empty(),
            "{step}: rejected {:?}",
            log.rejected
        );
        assert!(
            log.discarded.is_empty(),
            "{step}: discarded {:?}",
            log.discarded
        );
    }

    fn find(&self, ty: u8, class: u32) -> Option<UnitKey> {
        self.app
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .units
            .iter()
            .find(|(k, u)| k.unit_type == ty && u.class == class && u.position.is_some())
            .map(|(k, _)| *k)
    }

    /// The server's unit of type `ty` and class `class` and its sub-tile.
    fn server_unit(&self, ty: u8, class: u32) -> Option<(i32, i32)> {
        let st = match ty {
            1 => d2_sim::units::UnitType::Monster,
            _ => d2_sim::units::UnitType::Object,
        };
        app_support::with(&self.server, move |l| {
            let g = &mut l.host_mut().game;
            let u = g.game.lists.units_of_type(st).into_iter().find(|&u| {
                g.events
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == class)
            })?;
            Some(g.events.action.hooks().path_position(u))
        })
    }

    /// Run legs toward `goal` until `done`, within 10 sub-tiles of it, or
    /// stuck.
    fn run_to(&mut self, goal: (i32, i32), mut done: impl FnMut(&mut Run) -> bool) -> bool {
        use test_fixtures::host::{cheb, LEG};
        for _ in 0..30 {
            if done(self) {
                return true;
            }
            let p = self.pos();
            if cheb(p, goal) <= 10 {
                return done(self);
            }
            self.leg((
                p.0 + (goal.0 - p.0).clamp(-LEG, LEG),
                p.1 + (goal.1 - p.1).clamp(-LEG, LEG),
            ));
            if cheb(self.pos(), p) <= 2 && !self.sidestep() {
                break;
            }
        }
        done(self)
    }

    /// The town's DRLG room centres (sub-tiles), nearest to the player
    /// first.
    fn town_rooms(&self) -> Vec<(i32, i32)> {
        let p = self.pos();
        let mut v: Vec<(i32, i32)> = app_support::with(&self.server, |l| {
            let g = &mut l.host_mut().game;
            g.events
                .action
                .hooks()
                .drlg
                .with_act(0, &mut g.game.lists, |d, _| {
                    let l = d.find_level(single_player::ACT1_TOWN)?;
                    Some(
                        d.level_rooms(l)
                            .into_iter()
                            .map(|r| {
                                let t = d.room(r).rect;
                                ((t.x * 2 + t.w) * 5 / 2, (t.y * 2 + t.h) * 5 / 2)
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .flatten()
                .unwrap_or_default()
        });
        v.sort_by_key(|&c| test_fixtures::host::cheb(c, p));
        v
    }

    /// Walks the town until the client model holds the unit of type `ty`
    /// and class `class` within 10 sub-tiles of the player: toward it
    /// once the server has it, else through the town's rooms (their
    /// presets are placed when the player brings them into play).
    fn approach(&mut self, ty: u8, class: u32) -> UnitKey {
        let near = move |r: &mut Run| {
            let p = r.pos();
            r.find(ty, class).is_some()
                && r.server_unit(ty, class)
                    .is_some_and(|at| test_fixtures::host::cheb(p, at) <= 10)
        };
        let mut rooms = self.town_rooms().into_iter();
        for _ in 0..40 {
            if near(self) {
                return self.find(ty, class).unwrap();
            }
            match self.server_unit(ty, class) {
                Some(at) => {
                    self.run_to(at, near);
                }
                None => {
                    let Some(c) = rooms.next() else { break };
                    self.run_to(c, move |r| r.server_unit(ty, class).is_some());
                }
            }
        }
        self.dump("approach");
        panic!("unit {ty}/{class} not reached");
    }

    /// Runs to the unit `key` as the client does before an interact
    /// (C→S 0x04, run to a unit), until the player stops.
    fn run_to_unit(&mut self, key: UnitKey) {
        let mut m = vec![0x04];
        m.extend_from_slice(&u32::from(key.unit_type).to_le_bytes());
        m.extend_from_slice(&key.guid.to_le_bytes());
        self.bridge().send_bytes(&m).unwrap();
        self.step(2);
        for _ in 0..300 {
            if !test_fixtures::host::MOVING.contains(&self.mode()) {
                break;
            }
            self.step(1);
        }
        self.step(4);
        self.check("run to the unit");
    }

    /// No divergence was met ([`Run::findings`]).
    fn no_findings(&self) {
        assert!(
            self.findings.is_empty(),
            "{} divergences:\n{}",
            self.findings.len(),
            self.findings.join("\n")
        );
    }

    /// What the server and the client model hold (diagnostics).
    fn dump(&self, at: &str) {
        let server = app_support::with(&self.server, |l| {
            let g = &mut l.host_mut().game;
            let mut v = Vec::new();
            for ty in [
                d2_sim::units::UnitType::Monster,
                d2_sim::units::UnitType::Object,
            ] {
                for u in g.game.lists.units_of_type(ty) {
                    let class = g.events.action.sys.units.get(u).map(|r| r.class);
                    let p = g.events.action.hooks().path_position(u);
                    v.push((ty as u8, class, p));
                }
            }
            v
        });
        let w = self.app.world().resource::<BridgeResource>().0.world();
        let client: Vec<_> = w
            .units
            .iter()
            .map(|(k, u)| (k.unit_type, u.class, u.position))
            .collect();
        eprintln!(
            "{at}: player {:?}\n server {server:?}\n client {client:?}",
            self.pos()
        );
    }

    fn server_level(&self) -> Option<u32> {
        app_support::with(&self.server, |l| {
            let (p, _) = single_player::local_player(&l.host().game)?;
            let g = &mut l.host_mut().game;
            let room = g.game.lists.unit(p)?.room()?;
            g.events.action.hooks().drlg.level_id(&g.game, room)
        })
    }
}

/// The NPC menu row of `kind`: its index among the box's selectable rows.
fn menu_row_index(rows: &[Option<OptionKind>], kind: Option<OptionKind>) -> usize {
    rows.iter().position(|r| *r == kind).expect("the row")
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_scripted_play_run() {
    let mut run = Run::start();

    // 1. The new character: a player with a skill list (no injected
    // S→C 0x94 / 0x23).
    {
        let w = run.app.world().resource::<BridgeResource>().0.world();
        let p = w.local().expect("the local player");
        assert!(p.position.is_some(), "placed");
    }
    run.check("new character");

    // 2. Talk to Akara: run toward where the town's preset placed her
    // until the client holds her, then the interact sender (the preview's
    // arrival).
    let akara = run.approach(1, u32::from(d2_sim::world::npc::class::AKARA));
    run.run_to_unit(akara);
    run.bridge().interact(akara).unwrap();
    run.until("the NPC menu", 400, |r| {
        r.app
            .world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .unwrap()
            .npc_menu()
            .is_some()
    });
    run.check("talk to Akara");
    let rows: Vec<Option<OptionKind>> = run
        .app
        .world()
        .non_send::<WorldViewUi>()
        .original
        .as_ref()
        .unwrap()
        .npc_menu()
        .unwrap()
        .rows
        .iter()
        .map(|r| r.kind)
        .collect();
    eprintln!("Akara's menu: {rows:?}");
    // The box is the spec box above Akara, as ui 8.
    assert!(run.ui_open(8), "the menu is ui 8 (panels-2.md §14)");
    let p = app_support::npc_menu_row(&run.app, menu_row_index(&rows, Some(OptionKind::Trade)));
    run.click(p);
    run.step(10);
    run.check("trade with Akara");
    {
        let w = run.app.world().resource::<BridgeResource>().0.world();
        let skills = w
            .local()
            .and_then(|p| p.skills.as_ref())
            .map(|s| format!("{s:?}"));
        eprintln!("skills: {skills:?}");
        eprintln!(
            "store items: {}, local items: {}",
            d2_client::bridge::items::store_items(w).len(),
            d2_client::bridge::items::local_items(w).len()
        );
    }
    // The Trade choice opens the shop even with an empty store,
    // and its close ends the interaction with C→S 0x30.
    assert!(run.ui_open(0x0C), "the shop opened on the Trade choice");
    run.app
        .world_mut()
        .non_send_mut::<WorldViewUi>()
        .original
        .as_mut()
        .unwrap()
        .set_ui(0x0C, 1, false)
        .unwrap();
    run.step(10);
    // C→S 0x30: the server reads the NPC GUID at +5 only
    // (`client-messages.tsv`).
    let ended = run
        .wire
        .lock()
        .unwrap()
        .sent
        .iter()
        .any(|m| m.len() == 9 && m[0] == 0x30 && m[5..9] == akara.guid.to_le_bytes());
    assert!(ended, "closing the shop sent C→S 0x30 for Akara");
    run.check("close the shop");

    // 3. The waypoint to Cold Plains.
    run.waypoint_to_cold_plains();
    let units: Vec<(u8, u32)> = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .units
        .iter()
        .map(|(k, u)| (k.unit_type, u.class))
        .collect();
    eprintln!("units in the field: {units:?}");
    run.no_findings();
}

impl Run {
    fn with_ui<R>(&self, f: impl FnOnce(&d2_client::ui::original::OriginalUi) -> R) -> R {
        f(self
            .app
            .world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .unwrap())
    }

    /// Runs to the town NPC of `class`, interacts and waits for its built
    /// menu box (S→C 0x28, ui 8).
    fn open_menu(&mut self, class: u32) -> UnitKey {
        let key = self.approach(1, class);
        self.run_to_unit(key);
        self.bridge().interact(key).unwrap();
        self.until("the NPC menu box", 400, |r| {
            r.with_ui(|u| u.npc_menu().is_some_and(|m| !m.rows.is_empty()))
        });
        key
    }

    fn sent_any(&self, f: impl Fn(&[u8]) -> bool) -> bool {
        self.wire.lock().unwrap().sent.iter().any(|m| f(m))
    }
}

// The spec NPC UI on the user's install (q-fix-ui-play-wiring; the
// synthetic town smoke tests were removed by q-fixture-migrate): Akara's
// menu box carries the string table's captions above her (ui 8), Talk
// opens the topic box and its cancel builds the menu again (Akara's flag
// 1), a left click on a store item opens the confirm dialog and No sends
// nothing, the shop's close ends the interaction; Kashya's build sends the
// hire-list request C→S 0x38 [3][NPC][player].
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_spec_npc_ui_on_the_install() {
    let mut run = Run::start();
    let akara = run.open_menu(u32::from(d2_sim::world::npc::class::AKARA));
    assert!(run.ui_open(8), "ui 8");
    let (rect, items) = run.with_ui(|u| u.npc_menu_box()).expect("the box");
    let texts: Vec<String> = items
        .iter()
        .map(|(t, _)| String::from_utf16_lossy(t))
        .collect();
    eprintln!("Akara's box {rect:?}: {texts:?}");
    // The selectable captions come from the install's string table.
    assert!(
        texts[1..].iter().all(|t| !t.is_empty()),
        "captions: {texts:?}"
    );
    run.check("Akara's menu");

    // Talk: the topic box (caption, introduction?, gossip, …, cancel).
    let p = app_support::npc_menu_row(&run.app, 0);
    run.click(p);
    run.step(2);
    let topics = run.with_ui(|u| u.npc_topics()).expect("the topic box");
    let topics: Vec<String> = topics.iter().map(|t| String::from_utf16_lossy(t)).collect();
    eprintln!("Akara's topics: {topics:?}");
    assert!(topics.len() >= 3 && topics.iter().all(|t| !t.is_empty()));
    let p = app_support::npc_topic_cancel(&run.app);
    run.click(p);
    run.step(3);
    assert_eq!(app_support::npc_menu_len(&run.app), 3, "the menu again");
    run.check("talk");

    // Trade; a left click on a store item asks first; No sends nothing.
    let rows: Vec<Option<OptionKind>> = run
        .with_ui(|u| u.npc_menu())
        .unwrap()
        .rows
        .iter()
        .map(|r| r.kind)
        .collect();
    let p = app_support::npc_menu_row(&run.app, menu_row_index(&rows, Some(OptionKind::Trade)));
    run.click(p);
    run.step(10);
    assert!(run.ui_open(0x0C) && !run.ui_open(1) && run.ui_open(8));
    let w = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .clone();
    let (guid, it) = d2_client::bridge::items::store_items(&w)
        .into_iter()
        .find_map(|i| {
            run.with_ui(|u| u.store_item_point(&w, i.key.guid))
                .map(|p| (i.key.guid, p))
        })
        .expect("a store item on the shown page");
    // The store item's tip carries the store context:
    // its top line is `Cost: ` and the price.
    let tip = run.with_ui(|u| u.store_tip_lines(&w, guid));
    eprintln!("store tip: {tip:?}");
    assert!(
        tip.first().is_some_and(|l| l.starts_with("Cost: ")),
        "{tip:?}"
    );
    run.click(it);
    run.step(2);
    let (kind, confirm) = run
        .with_ui(|u| u.shop_state().confirm())
        .expect("the confirm dialog");
    let confirm: Vec<String> = confirm
        .iter()
        .map(|t| String::from_utf16_lossy(t))
        .collect();
    eprintln!("confirm {kind:?}: {confirm:?}");
    assert_eq!(kind, d2_client::ui::panels::shop::TxKind::Buy);
    let no = run.with_ui(|u| u.shop_confirm_point(false)).unwrap();
    run.click(no);
    run.step(4);
    assert!(!run.sent_any(|m| m[0] == 0x32), "No buys nothing");
    run.app
        .world_mut()
        .non_send_mut::<WorldViewUi>()
        .original
        .as_mut()
        .unwrap()
        .set_ui(0x0C, 1, false)
        .unwrap();
    run.step(10);
    assert!(run.sent_any(|m| m.len() == 9 && m[0] == 0x30 && m[5..9] == akara.guid.to_le_bytes()));
    assert!(!run.ui_open(8), "the shop's close ended the interaction");
    run.check("shop");

    // Kashya's build sends the hire-list request.
    let kashya = run.open_menu(u32::from(d2_sim::world::npc::class::KASHYA));
    let player = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .local()
        .unwrap()
        .key
        .guid;
    let mut want = vec![0x38, 3, 0, 0, 0];
    want.extend_from_slice(&kashya.guid.to_le_bytes());
    want.extend_from_slice(&player.to_le_bytes());
    assert!(run.sent_any(|m| m == want.as_slice()), "C→S 0x38 action 3");
    let n = app_support::npc_menu_len(&run.app);
    let p = app_support::npc_menu_row(&run.app, n - 1);
    run.click(p);
    run.step(4);
    assert!(!run.ui_open(8));
    run.check("Kashya");
    run.no_findings();
}

/// The play app's build (`GameData::select` on the install, every act
/// created, `LevelSource::live`) runs on the user's files.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_game_builds_on_the_install() {
    let data = app_support::game_data();
    let g = single_player::build(&data, single_player::DEFAULT_SEED);
    assert!(g.is_ok(), "{:?}", g.err());
}

impl Run {
    /// The server player's path position (sub-tiles).
    fn pos(&self) -> (i32, i32) {
        app_support::with(&self.server, |l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).unwrap();
            g.events.action.hooks().path_position(p)
        })
    }

    fn mode(&self) -> u32 {
        app_support::with(&self.server, |l| {
            let g = &l.host().game;
            let (p, _) = single_player::local_player(g).unwrap();
            g.events.action.sys.units.get(p).map_or(0, |u| u.mode)
        })
    }

    /// The tile rect of level `id` in the act of the player.
    fn level_rect(&self, id: u32) -> d2_sim::drlg::TileRect {
        app_support::with(&self.server, move |l| {
            let h = l.host_mut().game.events.action.hooks();
            h.drlg
                .dungeon
                .acts
                .iter()
                .flatten()
                .find_map(|d| d.find_level(id).map(|lv| d.level(lv).rect))
                .expect("level allocated")
        })
    }

    /// A run leg (C→S 0x03 through the bridge), then frames until the
    /// player stops.
    fn leg(&mut self, (x, y): (i32, i32)) {
        let mut m = vec![0x03];
        m.extend_from_slice(&(x as u16).to_le_bytes());
        m.extend_from_slice(&(y as u16).to_le_bytes());
        self.bridge().send_bytes(&m).unwrap();
        self.step(2);
        for _ in 0..400 {
            if !test_fixtures::host::MOVING.contains(&self.mode()) {
                break;
            }
            self.step(1);
        }
        // The walk prediction steps its last ticks (a frame behind).
        self.step(10);
        let (drawn, p) = (self.drawn(), self.pos());
        if test_fixtures::host::cheb(drawn, p) > 2 {
            eprintln!("run leg to ({x}, {y}): drawn {drawn:?}, server {p:?}");
            self.findings.push(format!(
                "q-fix-real-client-path: run leg to ({x}, {y}) drawn at {drawn:?}, server at {p:?}"
            ));
        }
        self.check("run leg");
    }

    /// The walk prediction's sub-tile: where the client draws its player.
    fn drawn(&self) -> (i32, i32) {
        self.app
            .world()
            .resource::<d2_client::world_view::walk::PreviewWalk>()
            .predict
            .cell()
            .map(|(x, y)| (i32::from(x), i32::from(y)))
            .expect("a prediction")
    }

    /// The shared collision route (`test_fixtures::host::route`) from the
    /// server player toward level `to`, inside levels `from` and `to`.
    fn route(&self, from: u32, to: u32) -> Vec<(i32, i32)> {
        let (fr, tr) = (self.level_rect(from), self.level_rect(to));
        let start = self.pos();
        app_support::with(&self.server, move |l| {
            let h = l.host_mut().game.events.action.hooks();
            let d = h.drlg.dungeon.acts[0].as_ref().expect("Act I");
            test_fixtures::host::route(d, start, fr, tr, 12)
        })
    }

    /// Run legs into level `to` (from `from`) along [`Self::route`] until
    /// the server player is there (the route is searched again as the
    /// rooms ahead become active).
    fn walk_into(&mut self, from: u32, to: u32) {
        for _ in 0..12 {
            if self.server_level() == Some(to) {
                return;
            }
            let route = self.route(from, to);
            for g in route {
                self.leg(g);
                if self.server_level() == Some(to) {
                    break;
                }
            }
        }
        assert_eq!(self.server_level(), Some(to), "walked into level {to}");
    }
}

/// The scripted run on the five-act install, loaded as `d2-client play`
/// loads the user's files: join, then out of the Rogue Encampment into
/// the Blood Moor on foot (run legs, C→S 0x03). It failed twice on the way:
/// the position checks of S→C 0x96 had no visibility predicate
/// (now the world view's), and a check correction sent
/// C→S 0x5F with a client position the server had not walked (the last
/// placement, or the straight-line prediction past an obstacle), so the
/// server walked the player there: the rubber band. While the preview
/// predicts, the check now follows the server; `Run::check`
/// asserts no 0x5F is ever sent.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_live_run() {
    let data = app_support::game_data();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("smoke-{}-live", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut run = Run::start();
    run.walk_into(1, 2);
    run.step(20);
    run.check("Blood Moor");
    assert_eq!(run.server_level(), Some(2), "out of town on foot");
    // Spawned monsters, killed with the left skill (Attack), until one
    // drops something.
    let mut ground = Vec::new();
    for n in 0..8 {
        let near = run.monsters_near();
        let target = *near.first().expect("a monster spawned in the Blood Moor");
        let killed = run.kill(target);
        assert!(
            killed,
            "monster {n} died: life {:?}",
            run.monster_life(target.guid)
        );
        run.step(30);
        run.check("kill");
        ground = run.ground_items();
        let xp = run.player_stat(13);
        eprintln!(
            "kill {n}: experience {xp}, ground {:?}",
            ground
                .iter()
                .map(|i| (i.key.guid, i.code))
                .collect::<Vec<_>>()
        );

        if !ground.is_empty() {
            break;
        }
    }
    assert!(!ground.is_empty(), "a kill dropped an item");

    // Pick up every drop (C→S 0x16, auto placement: the gold to the
    // player's gold, the items to the belt or the inventory), running to
    // it first as the client does (C→S 0x04, run to the unit).
    let gold0 = run.player_stat(14);
    for it in &ground {
        let p = run.pos();
        if (p.0 - i32::from(it.x))
            .abs()
            .max((p.1 - i32::from(it.y)).abs())
            > 2
        {
            let mut m = vec![0x04];
            m.extend_from_slice(&4u32.to_le_bytes());
            m.extend_from_slice(&it.key.guid.to_le_bytes());
            run.bridge().send_bytes(&m).unwrap();
            run.step(2);
            for _ in 0..300 {
                if !test_fixtures::host::MOVING.contains(&run.mode()) {
                    break;
                }
                run.step(1);
            }
            run.check("run to the item");
        }
        run.bridge()
            .send(&d2_client::bridge::items::pick(it.key.guid, false))
            .unwrap();
        run.step(6);
        run.check("pick up");
    }
    let left: Vec<_> = run
        .ground_items()
        .iter()
        .map(|i| (i.key.guid, i.code))
        .collect();
    assert!(left.is_empty(), "everything picked up: {left:?}");
    // The gold check holds for a drop with gold (a kill may drop items
    // only, e.g. a lone scroll).
    if ground.iter().any(|i| i.code == Some(*b"gld ")) {
        assert!(run.player_stat(14) > gold0, "the gold reached the player");
    }
    let mine = run.local_items();
    for it in ground.iter().filter(|i| i.code != Some(*b"gld ")) {
        assert!(
            mine.iter().any(|m| m.key == it.key),
            "{:?} is the player's: {mine:?}",
            it.code
        );
    }

    // Level up: more kills until level 2 (500 experience, the install's
    // `experience` row 2), searching deeper into the Blood Moor when none
    // is near.
    let rect = run.level_rect(2);
    let centre = (
        (rect.x + rect.w / 2) * test_fixtures::host::SUB,
        (rect.y + rect.h / 2) * test_fixtures::host::SUB,
    );
    let mut unreachable = Vec::new();
    for i in 0..120 {
        if run.player_stat(12) >= 2 {
            break;
        }
        let near = run.monsters_near();
        eprintln!(
            "level-up {i}: frame {}, experience {}, {} monsters near, unreachable {}",
            run.frames,
            run.player_stat(13),
            near.len(),
            unreachable.len()
        );
        match near.into_iter().find(|m| !unreachable.contains(&m.guid)) {
            Some(m) => {
                if !run.kill(m) {
                    unreachable.push(m.guid);
                    continue;
                }
                run.step(20);
                run.check("kill");
            }
            None => {
                // Toward the level's centre first, then the other
                // directions, until a leg moves (a blocked path stays).
                let p = run.pos();
                let toward = (
                    (centre.0 - p.0).signum() * 15,
                    (centre.1 - p.1).signum() * 15,
                );
                let dirs = [
                    toward,
                    (15, 0),
                    (0, 15),
                    (-15, 0),
                    (0, -15),
                    (15, 15),
                    (-15, 15),
                    (15, -15),
                    (-15, -15),
                ];
                for d in dirs {
                    run.leg((p.0 + d.0, p.1 + d.1));
                    if test_fixtures::host::cheb(run.pos(), p) > 3 {
                        break;
                    }
                }
            }
        }
    }
    assert_eq!(run.player_stat(12), 2, "level 2");
    // The level's stat points: the install's `charstats` StatPerLevel
    // of the sorceress (5); one skill point a level.
    let per_level = {
        let GameData::Live(d) = &data;
        let rows: Vec<d2_data::tables::Charstats> = d.tables.rows().unwrap();
        i32::from(rows[1].statperlevel)
    };
    let (points, skills) = (run.player_stat(4), run.player_stat(5));
    assert_eq!(
        (points, skills),
        (per_level, 1),
        "the level's stat and skill points"
    );
    // Spend: strength + 1 (C→S 0x3A stat 0, count − 1 = 0), Fire Bolt
    // (skill 36 of the install's `skills`, the sorceress' level-1 class
    // skill) + 1 (0x3B).
    let str0 = run.player_stat(0);
    run.bridge().send_bytes(&[0x3A, 0, 0]).unwrap();
    run.bridge().send_bytes(&[0x3B, 36, 0]).unwrap();
    run.step(6);
    run.check("spend points");
    assert_eq!(run.player_stat(0), str0 + 1, "strength");
    assert_eq!(run.player_stat(4), points - 1, "a stat point spent");
    assert_eq!(run.player_stat(5), skills - 1, "a skill point spent");
    let firebolt = {
        let w = run.app.world().resource::<BridgeResource>().0.world();
        w.local()
            .and_then(|p| p.skills.as_ref())
            .map(|l| format!("{l:?}"))
    };
    assert!(
        firebolt.is_some_and(|l| l.contains("skill: 36,")),
        "the client learned Firebolt (S→C 0x21)"
    );

    // Town Portal: the start scroll used (C→S 0x20 at the player's
    // position), the field portal reaches the client (S→C 0x51), a click
    // on it (0x13) takes the player to the Rogue Encampment.
    let scroll = run
        .local_items()
        .into_iter()
        .find(|i| i.code == Some(*b"tsc "))
        .expect("the start scroll")
        .key
        .guid;
    let p = run.pos();
    run.bridge()
        .send(&d2_client::bridge::items::use_grid(
            scroll, p.0 as u32, p.1 as u32,
        ))
        .unwrap();
    run.step(10);
    run.check("read the scroll");
    let portal = {
        let w = run.app.world().resource::<BridgeResource>().0.world();
        w.units
            .iter()
            .find(|(k, u)| k.unit_type == 2 && u.class == 59 && u.position.is_some())
            .map(|(k, _)| *k)
    };
    let portal = portal.expect("the field portal in the client model");
    // The portal's hostile delay has passed.
    run.ms.fetch_add(10_000, Ordering::SeqCst);
    run.bridge().interact(portal).unwrap();
    run.until("the town", 400, |r| r.server_level() == Some(1));
    run.step(20);
    run.check("through the portal");

    // Equip: the start weapon off the right arm to the cursor (C→S 0x1C)
    // and back on (0x1A).
    let weapon = run
        .local_items()
        .into_iter()
        .find(|i| i.mode == 1 && i.body == 4)
        .expect("the start weapon in the right arm")
        .key
        .guid;
    run.bridge()
        .send(&d2_proto::client::RemoveBodyItem { bodyloc: 4 })
        .unwrap();
    run.step(6);
    run.check("unequip");
    let cursor = {
        let w = run.app.world().resource::<BridgeResource>().0.world();
        d2_client::bridge::items::cursor_item(w).map(|i| i.key.guid)
    };
    assert_eq!(cursor, Some(weapon), "the weapon on the cursor");
    run.bridge()
        .send(&d2_client::bridge::items::equip(weapon, 4))
        .unwrap();
    run.step(6);
    run.check("equip");
    let worn = run
        .local_items()
        .into_iter()
        .find(|i| i.mode == 1 && i.body == 4)
        .map(|i| i.key.guid);
    if worn != Some(weapon) {
        // The staff stays on the cursor though 0x1A answers 0 (finding
        // q-fix-real-equip-2h): put it in the inventory (C→S 0x18) so the
        // run goes on.
        run.findings.push(format!(
            "q-fix-real-equip-2h: C→S 0x1A of the start staff (GUID {weapon}) to the right hand answered 0 and left it on the cursor"
        ));
        let mut m = vec![0x18];
        m.extend_from_slice(&weapon.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        run.bridge().send_bytes(&m).unwrap();
        run.step(6);
        run.check("to the inventory");
        let cursor = {
            let w = run.app.world().resource::<BridgeResource>().0.world();
            d2_client::bridge::items::cursor_item(w).map(|i| i.key.guid)
        };
        if cursor.is_some() {
            run.findings.push(format!(
                "q-fix-real-equip-2h: C→S 0x18 left the staff (GUID {weapon}) on the cursor too"
            ));
        }
    }

    // Save and exit on the server's path: the
    // server's character storage is the app's file writer (as
    // `play::run`'s `save::share` installs it, with the install's `.d2s`
    // tables); the Esc menu's Save and Exit sends C→S 0x69, the server's
    // leave writes the file before its 0x05, 0x06, and the app ends on
    // them. Then the file is loaded and joined again.
    let GameData::Live(live) = &data;
    let path = dir.join("Smoke.d2s");
    let before = run.snapshot();
    let layouts = |r: &Run| -> Vec<_> { [1u32, 2, 3].map(|l| r.level_rect(l)).to_vec() };
    let layout_before = layouts(&run);
    {
        use d2_client::app::save;
        let store = save::FileStore {
            path: path.clone(),
            base: save::base_save(&run.character),
            tables: Arc::new(live.save.clone()),
            appearance: Some(Arc::new(
                save::appearance_tables(&live.tables.fixed).expect("appearance tables"),
            )),
            codec: Arc::new(d2_client::app::single_player::D2sCodec),
        };
        app_support::with(&run.server, move |l| {
            l.host_mut().game.set_storage(Box::new(store))
        });
    }
    assert!(!path.exists(), "nothing written before the leave");
    run.queue(UiEvent::Action(ActionId(Action::GameMenu.index() as u16)));
    run.step(2);
    assert!(run.ui_open(9), "the Esc menu is open");
    // Game menu rows: tops 185 / 235 / 285; Save and Exit is row 1.
    run.click(Point::new(400, 185 + 50 + 20));
    run.until("the app ends on the server's answer", 50, |r| {
        r.app.should_exit().is_some()
    });
    assert!(
        run.wire.lock().unwrap().sent.iter().any(|m| m == &[0x69]),
        "Save and Exit sent C→S 0x69"
    );
    {
        let w = run.app.world().resource::<BridgeResource>().0.world();
        assert!(
            !w.in_game && w.unloaded && w.exit_requested,
            "0x05, 0x06 received"
        );
    }
    let (gone, faults) = app_support::with(&run.server, |l| {
        let g = &l.host().game;
        let gone = g.client_list().is_empty();
        let faults = g.session().map(|s| format!("{:?}", s.faults));
        (gone, faults.unwrap_or_default())
    });
    assert!(gone, "the leave removed the client");
    assert!(!faults.contains("Save"), "the leave saved: {faults}");
    assert!(path.exists(), "the server's leave wrote the save");
    let findings = std::mem::take(&mut run.findings);
    drop(run);
    // The map seed (save +0xAB = game +0x7C) is the one the game
    // was built with, and the town byte marks it for restoring.
    let character = single_player::load_character(&data, &path, 0).expect("the save loads");
    let single_player::Character::Save(saved, _) = &character else {
        unreachable!("a loaded save")
    };
    assert_eq!(
        saved.header.map_seed,
        single_player::DEFAULT_SEED,
        "the saved map seed"
    );
    assert_eq!(saved.header.towns[0] & 0x80, 0x80, "the Normal town byte");
    // The server's save rebuilt the appearance bytes from
    // the equipped items (a new character's base has none). With nothing
    // equipped they are 32 x 0xFF; a weapon in the right hand
    // is the weapon in use (inventory +0x1C, the body link),
    // so the right-hand owner,
    // and draws in RH as its own token (the item's
    // `alternategfx`, then `code`).
    {
        let a = d2_client::app::save::appearance_tables(&live.tables.fixed).unwrap();
        let c = saved.header.components;
        let worn: Vec<&([u8; 4], u8, u8)> = before.2.iter().filter(|i| i.1 == 1).collect();
        if worn.is_empty() {
            assert_eq!(c, [0xFF; 16], "nothing equipped: appearance components");
        }
        if let Some(&&(code, _, _)) = worn.iter().find(|i| i.2 == 4) {
            let g = a.items.iter().find(|g| g.code == code).expect("the record");
            let token = a.tokens.lookup(g.alternategfx, g.code);
            assert_eq!(
                c[d2_formats::d2s::appearance::part::RH],
                token,
                "the right-hand weapon's token in RH: {c:02X?}"
            );
        }
    }
    // The file alone gives the same bytes (`d2s-tool resave`'s path,
    // `equipment_of_save`: the load's reading of the items and its
    // weapon-in-use links) as the running game's rebuild.
    {
        let tables = live.tables.item_tables().unwrap();
        let a = d2_client::app::save::appearance_tables(&live.tables.fixed).unwrap();
        let items = &saved.body.as_ref().unwrap().items;
        let eq = d2_server::adapters::character::save::equipment_of_save(items, &tables, &a)
            .expect("the saved items read back");
        let mut again = (**saved).clone();
        again.header.components = [1; 16];
        again.header.rebuild_appearance(&eq, &a);
        assert_eq!(
            (again.header.components, again.header.colours),
            (saved.header.components, saved.header.colours),
            "the file's own rebuild"
        );
    }
    assert_eq!(saved.header.colours, [0xFF; 16], "appearance colours");
    assert_eq!(
        single_player::game_seed(&character, None),
        saved.header.map_seed
    );
    let mut run = Run::start_as(character);
    run.check("join the saved character");
    assert_eq!(layouts(&run), layout_before, "the same level layouts");
    let after = run.snapshot();
    assert_eq!(
        (&after.0, &after.1),
        (&before.0, &before.1),
        "the loaded character's stats and skills are the saved ones"
    );
    if after.2 != before.2 {
        // Finding q-fix-real-start-belt: the new sorceress's four `hp1`
        // (charstats item2, count 4) are not in the client model before
        // the save, and come back stored (mode 0), not in the belt.
        run.findings.push(format!(
            "q-fix-real-start-belt: items before the save {:?}, after the load {:?}",
            before.2, after.2
        ));
    }
    run.findings.extend(findings);
    run.no_findings();
}

impl Run {
    /// The hostile monsters (not allied, not dying or dead: the server's
    /// sides, `LocalSeams::sides`) the client model holds with a
    /// position, nearest to the server player first.
    fn monsters_near(&self) -> Vec<UnitKey> {
        let foes: Vec<(u32, (i32, i32))> = app_support::with(&self.server, |l| {
            let g = &mut l.host_mut().game;
            let lists = &g.game.lists;
            let x = &g.events.action.hooks().x;
            x.sides
                .iter()
                .filter(|(u, (ty, allied, _))| {
                    *ty == d2_sim::units::UnitType::Monster && !allied && !x.down.contains(u)
                })
                .filter_map(|(u, &(_, _, p))| Some((lists.unit(*u)?.guid, p)))
                .collect()
        });
        let w = self.app.world().resource::<BridgeResource>().0.world();
        let (px, py) = self.pos();
        let mut v: Vec<(i32, UnitKey)> = w
            .units
            .iter()
            .filter(|(k, u)| k.unit_type == 1 && u.position.is_some() && !u.is_dead())
            .filter_map(|(k, _)| {
                let &(_, (x, y)) = foes.iter().find(|(g, _)| *g == k.guid)?;
                Some(((x - px).abs().max((y - py).abs()), *k))
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, k)| k).collect()
    }

    /// The ground items of the model (`bridge::items::ground_items`).
    fn ground_items(&self) -> Vec<d2_client::bridge::items::ItemView> {
        let w = self.app.world().resource::<BridgeResource>().0.world();
        d2_client::bridge::items::ground_items(w)
    }

    /// What a save keeps of the server player: level, experience, the
    /// stats, gold, its class skills' levels, its items (code, mode,
    /// body location), sorted, its waypoints and its quest flags.
    fn snapshot(&self) -> Snapshot {
        let stats = [0u16, 1, 2, 3, 4, 5, 12, 13, 14]
            .iter()
            .map(|&s| (s, self.player_stat(s)))
            .collect();
        let skills = app_support::with(&self.server, |l| {
            let g = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(g).unwrap();
            let h = g.events.action.hooks();
            let mut v: Vec<(u16, i32)> = h
                .skill_lists
                .get(&p)
                .map(|l| l.entries.iter().map(|e| (e.skill, e.base)).collect())
                .unwrap_or_default();
            v.sort();
            v
        });
        let mut items: Vec<([u8; 4], u8, u8)> = self
            .local_items()
            .into_iter()
            .filter_map(|i| Some((i.code?, i.mode, i.body)))
            .collect();
        items.sort();
        let (waypoints, quests) = app_support::with(&self.server, |l| {
            let live = d2_client::app::save::read_live(&mut l.host_mut().game).unwrap();
            (live.extra.waypoints, live.quests.map(|q| q.to_vec()))
        });
        assert!(
            waypoints.is_some() && quests.is_some(),
            "the save reads both"
        );
        (stats, skills, items, waypoints, quests)
    }

    /// The local player's items (`bridge::items::local_items`).
    fn local_items(&self) -> Vec<d2_client::bridge::items::ItemView> {
        let w = self.app.world().resource::<BridgeResource>().0.world();
        d2_client::bridge::items::local_items(w)
    }

    /// The server player's base stat `s`.
    fn player_stat(&self, s: u16) -> i32 {
        app_support::with(&self.server, move |l| {
            let g = &l.host().game;
            let (p, _) = single_player::local_player(g).unwrap();
            g.events.action.sys.stats.unit_base(p, s, 0)
        })
    }

    /// The monster `guid` is dying or dead on the server (mode 0 or 12),
    /// or gone. Its life is not the test: a corpse's life
    /// still regenerates (the stat list has no dead-mode stop; an open
    /// question).
    fn monster_dead(&self, guid: u32) -> bool {
        app_support::with(&self.server, move |l| {
            let sim = &mut l.host_mut().game;
            let Some(m) = sim
                .game
                .lists
                .find_unit(d2_sim::units::UnitType::Monster, guid)
            else {
                return true;
            };
            sim.events
                .action
                .sys
                .units
                .get(m)
                .is_none_or(|r| matches!(r.mode, 0 | 12))
        })
    }

    /// The server's life of the monster `guid` (`None`: gone).
    fn monster_life(&self, guid: u32) -> Option<i32> {
        app_support::with(&self.server, move |l| {
            let sim = &mut l.host_mut().game;
            let m = sim
                .game
                .lists
                .find_unit(d2_sim::units::UnitType::Monster, guid)?;
            let a = &mut sim.events.action;
            Some(a.with(&mut sim.game, |_, v| {
                v.stat(m, d2_sim::stats::stat::HITPOINTS)
            }))
        })
    }

    /// A 15-sub-tile leg in the first of the eight directions that moves
    /// the player; `false` when none does.
    fn sidestep(&mut self) -> bool {
        let p = self.pos();
        for d in [
            (15, 0),
            (0, 15),
            (-15, 0),
            (0, -15),
            (15, 15),
            (-15, 15),
            (15, -15),
            (-15, -15),
        ] {
            self.leg((p.0 + d.0, p.1 + d.1));
            if test_fixtures::host::cheb(self.pos(), p) > 3 {
                return true;
            }
        }
        false
    }

    /// Left skill (Attack) on `key` (C→S 0x06) until the server says it
    /// is dead; one message each time the player is back in a neutral
    /// mode.
    /// `false`: out of reach (no leg toward it moves the player) or
    /// still alive.
    fn kill(&mut self, key: UnitKey) -> bool {
        for _ in 0..20 {
            if self.monster_dead(key.guid) {
                return true;
            }
            // A far monster: run toward it first (the unit target is
            // refused past its range).
            let mp = app_support::with(&self.server, move |l| {
                let g = &mut l.host_mut().game;
                let u = g
                    .game
                    .lists
                    .find_unit(d2_sim::units::UnitType::Monster, key.guid)?;
                Some(g.events.action.hooks().path_position(u))
            });
            if let Some(mp) = mp {
                let p = self.pos();
                if test_fixtures::host::cheb(p, mp) > 30 {
                    self.leg((
                        p.0 + (mp.0 - p.0).clamp(-25, 25),
                        p.1 + (mp.1 - p.1).clamp(-25, 25),
                    ));
                    // A blocked leg stays: step aside, else give the
                    // monster up (out of reach).
                    if test_fixtures::host::cheb(self.pos(), p) <= 3 && !self.sidestep() {
                        return false;
                    }
                    continue;
                }
            }
            let mut m = vec![0x06, 1, 0, 0, 0];
            m.extend_from_slice(&key.guid.to_le_bytes());
            self.bridge().send_bytes(&m).unwrap();
            let mut seen = vec![self.mode()];
            for _ in 0..200 {
                self.step(1);
                let m = self.mode();
                if seen.last() != Some(&m) {
                    seen.push(m);
                }
                if seen.len() > 1 && matches!(m, 1 | 5) {
                    break;
                }
            }
            self.check("attack");
            if std::env::var_os("SMOKE_DEBUG").is_some() {
                let (log, errs) = app_support::with(&self.server, |l| {
                    let h = l.host_mut().game.events.action.hooks();
                    (
                        h.x.skills
                            .log
                            .iter()
                            .rev()
                            .take(4)
                            .cloned()
                            .collect::<Vec<_>>(),
                        format!("{:?}", h.errors.iter().rev().take(3).collect::<Vec<_>>()),
                    )
                });
                let g = key.guid;
                let mmode = app_support::with(&self.server, move |l| {
                    let sim = &mut l.host_mut().game;
                    let m = sim
                        .game
                        .lists
                        .find_unit(d2_sim::units::UnitType::Monster, g)?;
                    let r = sim.events.action.sys.units.get(m)?;
                    Some((r.class, r.mode, r.flags))
                });
                eprintln!(
                    "attack {:?} {mmode:?}: modes {seen:?}, life {:?}, player {:?}, monster {mp:?}, log {log:?} errors {errs}",
                    key.guid,
                    self.monster_life(key.guid),
                    self.pos()
                );
            }
        }
        self.monster_dead(key.guid)
    }
}

/// A run into a wall: the drawn player stops where the server's player
/// stops. The fixture tiles have no walls, so the test stamps one (wall
/// bit 0x1) into the server's and the client's grids
/// alike, as the game builds both from the same tiles. The walk prediction
/// steps the player's own path over the client grids (`ClientPath`); a
/// straight line would run on through the wall.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_blocked_run_is_drawn_where_the_server_stops() {
    let mut run = Run::start();
    let p = run.pos();
    // 12 sub-tiles east, 61 long: past the A* radius,
    // so neither path goes round it.
    let wall: Vec<(i32, i32)> = (-30..=30).map(|k| (p.0 + 12, p.1 + k)).collect();
    let w = wall.clone();
    let server = app_support::with(&run.server, move |l| {
        let g = &mut l.host_mut().game;
        let d = g.events.action.hooks().drlg.dungeon.acts[0]
            .as_mut()
            .expect("Act I");
        w.iter()
            .filter_map(|&(x, y)| d.collision_at_mut(x, y).map(|v| *v |= 1))
            .count()
    });
    let client = {
        let mut b = run.app.world_mut().resource_mut::<BridgeResource>();
        let d = &mut b.0.drlg_mut().expect("the client DRLG").drlg;
        wall.iter()
            .filter_map(|&(x, y)| d.collision_at_mut(x, y).map(|v| *v |= 1))
            .count()
    };
    assert_eq!(
        (server, client),
        (wall.len(), wall.len()),
        "the wall in both grids"
    );
    run.leg((p.0 + 24, p.1));
    let s = run.pos();
    assert!(
        s.0 > p.0 && s.0 < p.0 + 12,
        "the server stopped at the wall: {p:?} → {s:?}"
    );
    let drawn = run.drawn();
    assert!(
        test_fixtures::host::cheb(drawn, s) <= 2,
        "the drawn player stopped there too: {drawn:?} / {s:?}"
    );
}

/// One request the sound layer took on the play path: the server tick it
/// was first seen at, its requested group base, units and start tick.
#[derive(Debug, Clone)]
struct Heard {
    server_tick: u64,
    base: i32,
    units: Vec<UnitKey>,
    start_tick: u32,
}

#[derive(Resource, Default)]
struct Listened {
    handles: std::collections::BTreeSet<u32>,
    heard: Vec<Heard>,
}

/// Records every new request of the audio driver after each frame.
fn listen(
    audio: Res<d2_client::app::sound::GameAudio>,
    bridge: Res<BridgeResource>,
    mut out: ResMut<Listened>,
) {
    let Some(driver) = audio.driver.as_ref() else {
        return;
    };
    let d = driver.lock().unwrap();
    let table = d.system().table();
    let tick = bridge.0.world().server_ticks;
    for r in d.system().requests() {
        if r.handle != 0 && out.handles.insert(r.handle) {
            out.heard.push(Heard {
                server_tick: tick,
                base: table.base(r.id),
                units: r.units.clone(),
                start_tick: r.start_tick,
            });
        }
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_play_path_steps_and_speaks() {
    let mut run = Run::start();
    run.app
        .init_resource::<Listened>()
        .add_systems(Last, listen);
    let me = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .local_player
        .expect("the local player");
    let class = run.app.world().resource::<BridgeResource>().0.world().units[&me].class;
    // Running to Akara: the sorceress's footsteps (class record 1:
    // `Footstep` base 2,720, +24 when running, +4 (k − 1) on a material).
    let akara = run.approach(1, u32::from(d2_sim::world::npc::class::AKARA));
    run.run_to_unit(akara);
    let steps: Vec<Heard> = run
        .app
        .world()
        .resource::<Listened>()
        .heard
        .iter()
        .filter(|h| h.units == [me])
        .cloned()
        .collect();
    let table_base = |run: &Run, id: i32| {
        let audio = run
            .app
            .world()
            .resource::<d2_client::app::sound::GameAudio>();
        let d = audio.driver.as_ref().unwrap().lock().unwrap();
        d.system().table().base(id)
    };
    assert_eq!(class, 1, "a sorceress");
    let bases: Vec<i32> = [
        2720, 2724, 2728, 2732, 2736, 2740, 2744, 2748, 2752, 2756, 2760, 2764,
    ]
    .iter()
    .map(|&id| table_base(&run, id))
    .collect();
    let footsteps: Vec<&Heard> = steps.iter().filter(|h| bases.contains(&h.base)).collect();
    assert!(
        footsteps.len() >= 4,
        "the player's footsteps on the way: {steps:?}"
    );
    // Interact: the NPC dialog branch speaks on the player, after the walk.
    let before = run.app.world().resource::<Listened>().heard.len();
    run.bridge().interact(akara).unwrap();
    run.until("the NPC menu", 400, |r| {
        r.app
            .world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .unwrap()
            .npc_menu()
            .is_some()
    });
    // Akara's text list names the dialog line m (the dialog's line
    // lookup for U, m); its speech is `npc-speech.tsv`'s sound of
    // that key, requested on the player with delay 5.
    let m = run
        .app
        .world()
        .non_send::<WorldViewUi>()
        .original
        .as_ref()
        .unwrap()
        .npc_text()
        .expect("the NPC text list")
        .m();
    let sound = d2_client::audio::triggers::tables::NpcSpeech::spec().sound(i32::from(m));
    assert!(sound > 0, "a speech line for text key {m}");
    let want = table_base(&run, sound);
    let said: Vec<Heard> = run.app.world().resource::<Listened>().heard[before..]
        .iter()
        .filter(|h| h.units == [me] && h.base == want)
        .cloned()
        .collect();
    assert_eq!(said.len(), 1, "Akara's dialog line once: {said:?}");
    assert!(
        said[0].start_tick > said[0].server_tick as u32
            && said[0].start_tick <= said[0].server_tick as u32 + 5,
        "delay 5 from the sound tick: {said:?}"
    );
    run.check("spoke");
}

/// The shop draws and hits each store item at the cells the server put
/// it in, so every cell of an item's
/// footprint finds its GUID and no two items share a cell. The server
/// places the store on the NPC's inventory (`InvDesk::store_place`), so
/// the 0x9C stream carries the
/// find-free cells (q-fix-server-store-fill).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_shop_finds_each_store_item_at_its_server_cells() {
    let mut run = Run::start();
    run.open_menu(u32::from(d2_sim::world::npc::class::AKARA));
    let rows: Vec<Option<OptionKind>> = run
        .with_ui(|u| u.npc_menu())
        .unwrap()
        .rows
        .iter()
        .map(|r| r.kind)
        .collect();
    let p = app_support::npc_menu_row(&run.app, menu_row_index(&rows, Some(OptionKind::Trade)));
    run.click(p);
    run.step(10);
    assert!(run.ui_open(0x0C));
    let w = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .clone();
    let cells = run.with_ui(|u| u.store_item_cells(&w));
    eprintln!("store cells: {cells:?}");
    assert!(cells.len() > 1, "{cells:?}");
    let mut taken = std::collections::BTreeSet::new();
    for &(g, x, y, cw, ch) in &cells {
        for cy in y..y + ch {
            for cx in x..x + cw {
                assert!(taken.insert((cx, cy)), "cell ({cx}, {cy}) twice: {cells:?}");
                assert_eq!(run.with_ui(|u| u.store_item_at_cell(&w, cx, cy)), Some(g));
            }
        }
    }
}

impl Run {
    /// Opens Akara's trade window (the shop, ui 0x0C).
    fn open_akara_shop(&mut self) -> UnitKey {
        let akara = self.open_menu(u32::from(d2_sim::world::npc::class::AKARA));
        let rows: Vec<Option<OptionKind>> = self
            .with_ui(|u| u.npc_menu())
            .unwrap()
            .rows
            .iter()
            .map(|r| r.kind)
            .collect();
        let p =
            app_support::npc_menu_row(&self.app, menu_row_index(&rows, Some(OptionKind::Trade)));
        self.click(p);
        self.step(10);
        assert!(self.ui_open(0x0C));
        akara
    }

    /// Ctrl held: the host's per-frame read of the keys needs the primary
    /// window, which the headless run lacks, so the flag is set directly.
    fn hold_ctrl(&mut self, down: bool) {
        self.app
            .world_mut()
            .non_send_mut::<WorldViewUi>()
            .original
            .as_mut()
            .unwrap()
            .set_ctrl(down);
    }
}

/// Ctrl-click on a backpack item with the shop open sells it (0x33 to the
/// open store's NPC): the install's server
/// answers, the item leaves the player's backpack and the gold rises.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn ctrl_click_sells_a_backpack_item_to_the_open_store() {
    let mut run = Run::start();
    let akara = run.open_akara_shop();
    let w = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .clone();
    let mine = d2_client::bridge::items::local_items(&w);
    eprintln!(
        "backpack: {:?}",
        mine.iter()
            .map(|i| (i.key.guid, i.code, i.mode, i.page))
            .collect::<Vec<_>>()
    );
    let it = mine
        .iter()
        .find(|i| i.mode == d2_client::bridge::items::mode::STORED && i.page == 0)
        .expect("a backpack item")
        .clone();
    let gold = run.player_stat(14);
    let at = run.with_ui(|u| u.inv_item_point(&w, it.key.guid)).unwrap();
    run.hold_ctrl(true);
    run.click(at);
    run.step(10);
    run.hold_ctrl(false);
    let sell = run.sent_any(|m| {
        m.len() == 17
            && m[0] == 0x33
            && m[1..5] == akara.guid.to_le_bytes()
            && m[5..9] == it.key.guid.to_le_bytes()
    });
    assert!(sell, "C→S 0x33 for the clicked item");
    assert!(!run.sent_any(|m| m[0] == 0x19), "Ctrl-click never lifts");
    run.until("the sold item leaves the backpack", 200, |r| {
        !r.local_items()
            .iter()
            .any(|i| i.key.guid == it.key.guid && i.mode == d2_client::bridge::items::mode::STORED)
    });
    assert!(run.player_stat(14) > gold, "the sale paid gold");
    run.check("sold");
    // No `no_findings`: the run leg's drawn / server offset (q-fix-real-client-path)
    // is the walk's, not this click's.
}

/// A Town Portal scroll used in the Rogue Encampment is refused and costs
/// nothing (as in the game, a known Cold Plains run, frame 362): the
/// server answers 0x3F and
/// 0x7C only, no portal object (0x51) and no skill count (0x22), and the
/// scroll stays in the backpack.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_town_portal_scroll_used_in_town_is_refused_without_cost() {
    let mut run = Run::start();
    let scroll = run
        .local_items()
        .into_iter()
        .find(|i| i.code == Some(*b"tsc ") && i.mode == d2_client::bridge::items::mode::STORED)
        .expect("a Town Portal scroll in the backpack")
        .key
        .guid;
    let before = run.wire.lock().unwrap().chunks.len();
    let (px, py) = run
        .app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .local()
        .and_then(|p| p.position)
        .expect("the player's position");
    run.bridge()
        .send(&d2_client::bridge::items::use_grid(
            scroll,
            u32::from(px),
            u32::from(py),
        ))
        .unwrap();
    run.step(10);
    let got: Vec<u8> = run.wire.lock().unwrap().chunks[before..].concat();
    let has = |needle: &[u8]| got.windows(needle.len()).any(|w| w == needle);
    let g = scroll.to_le_bytes();
    // `3F FF <item> FF FF` (the failure reset) and `7C 04 <item>`.
    assert!(
        has(&[0x3F, 0xFF, g[0], g[1], g[2], g[3], 0xFF, 0xFF]),
        "{got:02X?}"
    );
    assert!(has(&[0x7C, 0x04, g[0], g[1], g[2], g[3]]), "{got:02X?}");
    // No skill count (0x22) for the scroll and no portal object appears.
    assert!(!has(&[0x22, 0x04]), "{got:02X?}");
    assert!(
        run.local_items()
            .iter()
            .any(|i| i.key.guid == scroll && i.mode == d2_client::bridge::items::mode::STORED),
        "the scroll stays"
    );
}

/// A click while walking re-targets the walk from the precise position
/// ("the fraction is kept"): the predicted
/// position moves at most one walk step per server tick across the
/// re-target (`a1-walk-s`: re-placing the path at the cell centre jumped
/// it by up to half a sub-tile and drifted the camera 3 px).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_retarget_while_walking_keeps_the_precise_position() {
    let mut run = Run::start();
    let precise = |run: &Run| {
        run.app
            .world()
            .resource::<d2_client::world_view::walk::PreviewWalk>()
            .predict
            .position()
            .map(|(x, y)| (i64::from(x), i64::from(y)))
            .expect("a prediction")
    };
    // The clicks of `a1-walk-*`: through the UI, so the prediction walks.
    run.click(Point::new(600, 284));
    run.step(10);
    let last = precise(&run);
    // The click's own two frames (two ticks), then two more ticks.
    run.click(Point::new(400, 384));
    let p2 = precise(&run);
    run.step(1);
    let p3 = precise(&run);
    run.step(1);
    let p4 = precise(&run);
    let v = (p4.0 - p3.0, p4.1 - p3.1);
    assert!(v != (0, 0), "the re-targeted walk moves");
    assert!(p2 != last, "the click's frames walk");
    assert_eq!((p3.0 - v.0, p3.1 - v.1), p2, "one velocity from p2 on");
    // Where the re-targeted path's first step left from: the precise
    // position, not the centre of its sub-tile (the old re-placement
    // gave p2 = centre + v on both axes).
    let from = (p2.0 - v.0, p2.1 - v.1);
    let centre = |c: i64| (c & !0xFFFF) | 0x8000;
    assert_ne!(
        from,
        (centre(from.0), centre(from.1)),
        "the re-target restarted at a sub-tile centre"
    );
}

// The client path sees the
// model's living monsters as footprints (mask 0x100), so a walk onto a
// town NPC's sub-tile stops short of it, as the game's walks stop short
// of Warriv.
#[test]
#[ignore = "needs the D2 install (D2_GAME_DIR)"]
fn the_client_path_stops_short_of_a_monster_footprint() {
    use d2_client::bridge::client_path::{ClientPath, Own, PathTo};
    use d2_client::bridge::predict::{MoveStats, Speeds};
    let mut run = Run::start();
    run.step(4);
    let bridge = run.bridge();
    let world = bridge.world();
    let others = d2_client::world_view::walk::other_units(world, &bridge.inputs().tables.monsters);
    let me = world.local_cell().expect("the player's cell");
    let npc = *others
        .iter()
        .min_by_key(|u| {
            (i32::from(u.x) - i32::from(me.0)).abs() + (i32::from(u.y) - i32::from(me.1)).abs()
        })
        .expect("a town NPC in the model");
    let drlg = &world.drlg.as_ref().expect("the client DRLG").drlg;
    let t = d2_sim::path::tables::PathTables::spec().unwrap();
    let speeds = Speeds { walk: 6, run: 9 };
    let own = Own {
        stamina: 0x6400,
        moves: MoveStats::CREATION,
    };
    let end = |stamp: bool| {
        let mut p = ClientPath::default();
        assert!(p.place(&t, drlg, me.0, me.1), "the player's room is active");
        if stamp {
            p.stamp_others(&t, drlg, &others, &[]);
        }
        let to = PathTo::Point(npc.x, npc.y);
        assert!(
            p.request(&t, drlg, speeds, own, to, false),
            "the walk moves"
        );
        for _ in 0..400 {
            if stamp {
                p.stamp_others(&t, drlg, &others, &[]);
            }
            if !p.tick(&t, drlg, speeds, own, Some(to)) {
                break;
            }
        }
        let (x, y) = p.position().unwrap();
        ((x >> 16) as u16, (y >> 16) as u16)
    };
    assert_eq!(
        end(false),
        (npc.x, npc.y),
        "no footprint: the walk ends on the NPC"
    );
    assert_ne!(
        end(true),
        (npc.x, npc.y),
        "the NPC's footprint stops the walk"
    );
}

// The client path sees the footprint boxes the 0x51 object init stamped, so
// client path sees the footprint boxes the 0x51 object init stamped, so
// a walk onto such an object's sub-tile stops short of it, as the
// server's walk does (`cold-plains-2` soak: the server player stood
// at an object the client walked through).
#[test]
#[ignore = "needs the D2 install (D2_GAME_DIR)"]
fn the_client_path_stops_short_of_a_colliding_object() {
    use d2_client::bridge::client_path::{ClientPath, Own, PathTo};
    use d2_client::bridge::predict::{MoveStats, Speeds};
    let mut run = Run::start();
    run.step(4);
    let bridge = run.bridge();
    let world = bridge.world();
    let objects =
        d2_client::world_view::walk::other_objects(world, &bridge.inputs().objclient.rows);
    let me = world.local_cell().expect("the player's cell");
    let drlg = &world.drlg.as_ref().expect("the client DRLG").drlg;
    let t = d2_sim::path::tables::PathTables::spec().unwrap();
    let speeds = Speeds { walk: 6, run: 9 };
    let own = Own {
        stamina: 0x6400,
        moves: MoveStats::CREATION,
    };
    let end = |stamp: bool, to: (u16, u16)| {
        let mut p = ClientPath::default();
        assert!(p.place(&t, drlg, me.0, me.1), "the player's room is active");
        if stamp {
            p.stamp_others(&t, drlg, &[], &objects);
        }
        let to = PathTo::Point(to.0, to.1);
        if !p.request(&t, drlg, speeds, own, to, false) {
            return None;
        }
        for _ in 0..400 {
            if !p.tick(&t, drlg, speeds, own, Some(to)) {
                break;
            }
        }
        let (x, y) = p.position().unwrap();
        Some(((x >> 16) as u16, (y >> 16) as u16))
    };
    // The nearest colliding object the bare walk reaches.
    let mut near: Vec<_> = objects.iter().collect();
    near.sort_by_key(|o| {
        (i32::from(o.x) - i32::from(me.0)).abs() + (i32::from(o.y) - i32::from(me.1)).abs()
    });
    let o = near
        .into_iter()
        .find(|o| end(false, (o.x, o.y)) == Some((o.x, o.y)))
        .expect("a colliding object a walk without objects reaches");
    assert_ne!(
        end(true, (o.x, o.y)),
        Some((o.x, o.y)),
        "the object's footprint stops the walk"
    );
}
