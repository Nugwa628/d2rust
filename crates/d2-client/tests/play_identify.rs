//! Identify end to end on the user's install: the play app's client as
//! `d2-client play` wires it (`play::add_live_client`, the install's
//! tables, UI layouts and item tips, no window) over the app's own
//! single-player game, a new sorceress. Every press goes through the UI
//! event queue, every intent over the link, and the test reads both the
//! server's inventory and the client model.
//!
//! - The starting Scroll of Identify: a right press takes it as the used
//!   item (the identify cursor), a left press on an unidentified magic ring
//!   sends C→S 0x27; the ring is identified on the server and in the model,
//!   its tip shows its properties, the scroll is used up, the cursor ends.
//! - A Tome of Identify the same way: one charge goes, the tome stays.
//! - The starting scroll lifted and dropped on a Tome of Identify (C→S
//!   0x29) goes in: the tome gains one charge.
//! - The identify cursor with nothing to identify, cancelled by a right
//!   press: nothing is sent, nothing is lost.
//!
//! Staging (test inputs, not game rules): the tome comes from the dev
//! console's `item` command (the dev tools ship with the game); the ring
//! is made the same way on the server but left unidentified (the dev
//! command identifies what it gives).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::dev::server::DevChannel;
use d2_client::app::play::{add_live_client, LiveClient};
use d2_client::app::single_player::{self, GameData};
use d2_client::app::strings::{TableStrings, LANG};
use d2_client::bridge::items::{self, mode, ItemView};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::BridgeResource;
use d2_client::controls::Action;
use d2_client::ui::panel::ActionId;
use d2_client::ui::{Point, PointerButton, UiEvent};
use d2_client::world_view::WorldViewUi;
use d2_proto::dev::{parse, DevCommand};
use d2_proto::item_bits::hflag;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::{Server, SharedLink};

/// Stat 70, the quantity (a tome's charges).
const QUANTITY: u16 = 70;
/// Item flag 0x10, identified (server item flags and stream header alike).
const IDENTIFIED: u32 = 0x10;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The app's link: the shared server thread, keeping every C→S message.
struct Probe {
    server: Server<StepClock>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerLink for Probe {
    fn protocol_version(&self) -> u32 {
        SharedLink(self.server.clone()).protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        SharedLink(self.server.clone()).tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.lock().unwrap().push(msg.to_vec());
        SharedLink(self.server.clone()).send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        SharedLink(self.server.clone()).pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        SharedLink(self.server.clone()).receive()
    }
}

/// A server item: its item flags and quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ServerItem {
    flags: u32,
    quantity: i32,
}

struct Run {
    app: App,
    server: Server<StepClock>,
    chan: DevChannel,
    ms: Arc<AtomicU32>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Run {
    /// A new sorceress in the Rogue Encampment, the inventory open.
    fn start() -> Self {
        let data = app_support::game_data();
        let GameData::Live(live) = data.clone();
        let character = single_player::new_character("sorceress", "Ident").unwrap();
        let ms = Arc::new(AtomicU32::new(1000));
        let seed = single_player::game_seed(&character, None);
        let (mut link, started) =
            single_player::start_with(data.clone(), seed, character.clone(), StepClock(ms.clone()))
                .unwrap();
        let strings = TableStrings::load(live.archives.as_ref(), LANG).unwrap();
        let catalog = d2_client::app::dev::catalog(&live, &strings);
        let chan = d2_client::app::dev::install_server(&mut link, catalog, None).unwrap();
        let server = Arc::new(Mutex::new(link));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<ButtonInput<MouseButton>>();
        let speeds = single_player::walk_speeds(&data, &character).unwrap();
        add_live_client(
            &mut app,
            Box::new(Probe {
                server: server.clone(),
                sent: sent.clone(),
            }),
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
        let mut run = Run {
            app,
            server,
            chan,
            ms,
            sent,
        };
        let mut n = 0;
        while app_support::local_player(&run.server).is_none() {
            run.step(1);
            n += 1;
            assert!(n < 2000, "the join runs");
        }
        run.step(30);
        run.queue(UiEvent::Action(ActionId(
            Action::ToggleInventory.index() as u16
        )));
        run.step(3);
        assert!(run.ui_open(1), "the inventory is open");
        run
    }

    fn step(&mut self, n: usize) {
        for _ in 0..n {
            self.app.update();
            self.ms.fetch_add(40, Ordering::SeqCst);
        }
    }

    fn queue(&mut self, e: UiEvent) {
        self.app
            .world_mut()
            .non_send_mut::<WorldViewUi>()
            .queue
            .0
            .push(e);
    }

    /// The mouse moved to `at`, then a press and release of `button`.
    fn click(&mut self, button: PointerButton, at: Point) {
        self.hover(at);
        self.queue(UiEvent::Press { button, at });
        self.queue(UiEvent::Release { button, at });
        self.step(2);
    }

    fn hover(&mut self, at: Point) {
        self.queue(UiEvent::CursorMoved(at));
        self.step(2);
    }

    fn ui_open(&self, ui: u8) -> bool {
        self.with_ui(|u| u.is_open(ui))
    }

    fn with_ui<R>(&self, f: impl FnOnce(&d2_client::ui::original::OriginalUi) -> R) -> R {
        f(self
            .app
            .world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .unwrap())
    }

    fn world(&self) -> d2_client::bridge::world::ClientWorld {
        self.app
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .clone()
    }

    /// The local player's items in the client model.
    fn local_items(&self) -> Vec<ItemView> {
        items::local_items(&self.world())
    }

    fn client_item(&self, guid: u32) -> Option<ItemView> {
        self.local_items().into_iter().find(|i| i.key.guid == guid)
    }

    /// The backpack item (page 0) of `code` the client model holds.
    fn backpack(&self, code: &[u8; 4]) -> Vec<ItemView> {
        self.local_items()
            .into_iter()
            .filter(|i| i.code == Some(*code) && i.mode == mode::STORED && i.page == 0)
            .collect()
    }

    /// The centre of the backpack cell item `guid` starts at.
    fn point_of(&self, guid: u32) -> Point {
        let w = self.world();
        self.with_ui(|u| u.inv_item_point(&w, guid))
            .unwrap_or_else(|| panic!("item {guid} is in the drawn backpack"))
    }

    /// The tip the inventory draws over item `guid`.
    fn tip(&mut self, guid: u32) -> Vec<String> {
        let at = self.point_of(guid);
        self.hover(at);
        let w = self.world();
        self.with_ui(|u| u.inv_hover_tip(&w))
    }

    /// The player's item `guid` on the server: flags and quantity.
    fn server_item(&self, guid: u32) -> Option<ServerItem> {
        app_support::with(&self.server, move |l| {
            let sim = &l.host().game;
            let (p, _) = single_player::local_player(sim)?;
            let inv = sim.world.inventory.as_ref()?;
            let u = inv
                .state
                .items_of(p)
                .into_iter()
                .find(|u| inv.state.items.get(u).is_some_and(|d| d.guid == guid))?;
            Some(ServerItem {
                flags: inv.state.items[&u].flags,
                quantity: sim.events.action.sys.stats.unit_base(u, QUANTITY, 0),
            })
        })
    }

    /// One dev console line, run on the server thread between frames.
    fn dev(&mut self, line: &str) -> String {
        let cmd: DevCommand = parse(line).unwrap().unwrap();
        self.chan.lock().unwrap().commands.push(cmd);
        for _ in 0..10 {
            self.step(1);
            let reply = self.chan.lock().unwrap().replies.pop();
            if let Some(r) = reply {
                assert!(r.ok, "{line:?}: {}", r.text);
                self.step(5);
                return r.text;
            }
        }
        panic!("no reply to {line:?}");
    }

    /// A new item from the dev console `line` that reached the backpack
    /// of the client model: its GUID.
    fn give(&mut self, line: &str, code: &[u8; 4]) -> u32 {
        let before: Vec<u32> = self.backpack(code).iter().map(|i| i.key.guid).collect();
        let t = self.dev(line);
        assert!(t.contains("in the inventory"), "{line:?}: {t}");
        self.backpack(code)
            .into_iter()
            .map(|i| i.key.guid)
            .find(|g| !before.contains(g))
            .unwrap_or_else(|| panic!("{line:?}: the new item reached the client"))
    }

    /// The C→S messages sent since `from` with id `id`.
    fn sent_since(&self, from: usize, id: u8) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap()[from..]
            .iter()
            .filter(|m| m.first() == Some(&id))
            .cloned()
            .collect()
    }

    fn sent_len(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    /// The client model took every S→C message.
    fn clean(&self, step: &str) {
        let b = &self.app.world().resource::<BridgeResource>().0;
        let log = b.log();
        assert!(
            log.rejected.is_empty(),
            "{step}: rejected {:?}",
            log.rejected
        );
        assert!(
            log.dropped.is_empty(),
            "{step}: dropped {:02X?}",
            log.dropped
        );
        assert!(
            log.unowned.is_empty(),
            "{step}: unhandled {:02X?}",
            log.unowned
        );
    }

    /// The starting Scroll of Identify (the compact record of a new
    /// character) in the backpack.
    fn starting_scroll(&self) -> u32 {
        let s = self.backpack(b"isc ");
        assert_eq!(s.len(), 1, "a new character has one Scroll of Identify");
        assert!(
            s[0].flags & hflag::COMPACT != 0,
            "the starting scroll is a compact record"
        );
        s[0].key.guid
    }

    /// A magic ring in the backpack, unidentified on both sides. Staged on
    /// the server as the dev `item` command makes one (`create_reward`, the
    /// inventory placement), without the reward's identify: a ring found
    /// on the ground is unidentified.
    fn unidentified_ring(&mut self) -> u32 {
        use d2_sim::wiring::economy::quest_reward::{create_reward, QuestInv, QuestInventory};
        let before: Vec<u32> = self.backpack(b"rin ").iter().map(|i| i.key.guid).collect();
        app_support::with(&self.server, |l| {
            let s = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(s).unwrap();
            s.world
                .with_economy(&mut s.game, &mut s.events, |econ, parts| {
                    let inv = parts.inventory.as_deref_mut().expect("the inventory model");
                    let item = create_reward(econ, p, *b"rin ", 30, 4)
                        .unwrap()
                        .expect("a magic ring");
                    econ.items.get_mut(item).unwrap().flags &= !IDENTIFIED;
                    let mut q = QuestInv::new(&inv.tables, &mut inv.state, inv.rest.as_mut());
                    assert!(q.place(econ, p, item), "the ring fits the backpack");
                });
            let sent = s.world.take_inventory_sent(&mut s.game, &mut s.events);
            s.world.rest.sent.extend(sent);
        });
        self.step(5);
        let ring = self
            .backpack(b"rin ")
            .into_iter()
            .map(|i| i.key.guid)
            .find(|g| !before.contains(g))
            .expect("the ring reached the client");
        let s = self.server_item(ring).expect("the ring on the server");
        assert_eq!(s.flags & IDENTIFIED, 0, "the server ring is unidentified");
        let c = self.client_item(ring).unwrap();
        assert_eq!(c.flags & IDENTIFIED, 0, "the client ring is unidentified");
        ring
    }

    /// A right press on `used` (the identify cursor), then a left press on
    /// `target`: the 0x27 the client sent.
    fn identify(&mut self, used: u32, target: u32) -> Vec<Vec<u8>> {
        let from = self.sent_len();
        let at = self.point_of(used);
        self.click(PointerButton::Right, at);
        assert!(
            self.with_ui(|u| u.identify_pending()),
            "the identify cursor is on"
        );
        let at = self.point_of(target);
        self.click(PointerButton::Left, at);
        self.step(10);
        self.sent_since(from, 0x27)
    }
}

/// 0x27 (target, used).
fn use_item_action(target: u32, used: u32) -> Vec<u8> {
    let mut m = vec![0x27];
    m.extend_from_slice(&target.to_le_bytes());
    m.extend_from_slice(&used.to_le_bytes());
    m
}

/// The ring's tip, unidentified then identified: the unidentified line
/// goes and property lines come.
fn assert_tip_identified(before: &[String], after: &[String]) {
    let unid = |t: &[String]| t.iter().any(|l| l.contains("Unidentified"));
    assert!(
        unid(before),
        "the unidentified ring's tip says so: {before:?}"
    );
    assert!(!unid(after), "the identified ring's tip: {after:?}");
    assert!(
        after.len() > before.len(),
        "the identified tip shows its properties: {before:?} -> {after:?}"
    );
}

// (a) The starting Scroll of Identify on an unidentified magic ring.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_starting_scroll_of_identify_identifies_a_magic_ring() {
    let mut run = Run::start();
    let scroll = run.starting_scroll();
    let name = run.tip(scroll);
    assert!(
        name.iter().any(|l| l.contains("Scroll of Identify")),
        "the starting scroll's tip: {name:?}"
    );
    let ring = run.unidentified_ring();
    let before = run.tip(ring);

    let sent = run.identify(scroll, ring);
    assert_eq!(sent, vec![use_item_action(ring, scroll)], "one 0x27");
    assert!(
        !run.with_ui(|u| u.identify_pending()),
        "the identify cursor ended"
    );
    let s = run.server_item(ring).expect("the ring stays");
    assert_ne!(s.flags & IDENTIFIED, 0, "the server ring is identified");
    let c = run.client_item(ring).expect("the ring in the model");
    assert_ne!(c.flags & IDENTIFIED, 0, "the client ring is identified");
    assert_eq!((c.mode, c.page), (mode::STORED, 0), "the ring stays put");
    let after = run.tip(ring);
    assert_tip_identified(&before, &after);
    assert!(run.server_item(scroll).is_none(), "the scroll is used up");
    assert!(run.client_item(scroll).is_none(), "the client scroll went");
    run.clean("identify with the scroll");
}

// (b) A Tome of Identify on an unidentified magic ring.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_tome_of_identify_uses_one_charge_and_stays() {
    let mut run = Run::start();
    let tome = run.give("item ibk", b"ibk ");
    let q0 = run.server_item(tome).unwrap().quantity;
    assert!(q0 >= 1, "the new tome has charges ({q0})");
    let ring = run.unidentified_ring();
    let before = run.tip(ring);

    let sent = run.identify(tome, ring);
    assert_eq!(sent, vec![use_item_action(ring, tome)], "one 0x27");
    assert!(
        !run.with_ui(|u| u.identify_pending()),
        "the identify cursor ended"
    );
    let s = run.server_item(ring).unwrap();
    assert_ne!(s.flags & IDENTIFIED, 0, "the server ring is identified");
    let c = run.client_item(ring).unwrap();
    assert_ne!(c.flags & IDENTIFIED, 0, "the client ring is identified");
    let after = run.tip(ring);
    assert_tip_identified(&before, &after);
    let t = run.server_item(tome).expect("the tome stays");
    assert_eq!(t.quantity, q0 - 1, "one charge used");
    let ct = run.client_item(tome).expect("the client tome stays");
    assert_eq!((ct.mode, ct.page), (mode::STORED, 0));
    let tip = run.tip(tome);
    assert!(
        tip.iter().any(|l| l.ends_with(&format!(" {}", q0 - 1))),
        "the tome's tip shows {} charges: {tip:?}",
        q0 - 1
    );
    // The starting scroll was not touched.
    let scroll = run.starting_scroll();
    assert!(run.server_item(scroll).is_some());
    run.clean("identify with the tome");
}

// (c) The starting scroll dropped on a Tome of Identify goes in.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_starting_scroll_goes_into_a_tome_of_identify() {
    let mut run = Run::start();
    let scroll = run.starting_scroll();
    let tome = run.give("item ibk", b"ibk ");
    let q0 = run.server_item(tome).unwrap().quantity;
    let from = run.sent_len();
    // Lift the scroll, drop it on the tome.
    let at = run.point_of(scroll);
    run.click(PointerButton::Left, at);
    run.step(5);
    let held = items::cursor_item(&run.world()).map(|i| i.key.guid);
    assert_eq!(held, Some(scroll), "the scroll is on the cursor");
    let at = run.point_of(tome);
    run.click(PointerButton::Left, at);
    run.step(10);
    let books = run.sent_since(from, 0x29);
    assert_eq!(books.len(), 1, "one C→S 0x29: {books:02X?}");
    assert!(run.server_item(scroll).is_none(), "the scroll went in");
    assert!(run.client_item(scroll).is_none(), "the client scroll went");
    assert!(
        items::cursor_item(&run.world()).is_none(),
        "the cursor is empty"
    );
    assert_eq!(
        run.server_item(tome).unwrap().quantity,
        q0 + 1,
        "the tome gained a charge"
    );
    let tip = run.tip(tome);
    assert!(
        tip.iter().any(|l| l.ends_with(&format!(" {}", q0 + 1))),
        "the tome's tip shows {} charges: {tip:?}",
        q0 + 1
    );
    run.clean("scroll into the tome");
}

// (d) The identify cursor with nothing to identify, then cancelled.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_cancelled_identify_cursor_loses_nothing() {
    let mut run = Run::start();
    let scroll = run.starting_scroll();
    let held_before: Vec<(u32, u32)> = run
        .local_items()
        .iter()
        .map(|i| (i.key.guid, i.flags))
        .collect();
    let from = run.sent_len();
    let at = run.point_of(scroll);
    run.click(PointerButton::Right, at);
    assert!(
        run.with_ui(|u| u.identify_pending()),
        "the identify cursor is on"
    );
    // A right press cancels.
    run.click(PointerButton::Right, at);
    assert!(!run.with_ui(|u| u.identify_pending()), "cancelled");
    run.step(10);
    assert!(run.sent_since(from, 0x27).is_empty(), "no 0x27 sent");
    assert!(run.server_item(scroll).is_some(), "the server scroll stays");
    let c = run.client_item(scroll).expect("the client scroll stays");
    assert_eq!((c.mode, c.page), (mode::STORED, 0));
    let held_after: Vec<(u32, u32)> = run
        .local_items()
        .iter()
        .map(|i| (i.key.guid, i.flags))
        .collect();
    assert_eq!(held_before, held_after, "nothing changed");
    // The scroll still identifies afterwards.
    let ring = run.unidentified_ring();
    let sent = run.identify(scroll, ring);
    assert_eq!(sent, vec![use_item_action(ring, scroll)]);
    assert_ne!(run.server_item(ring).unwrap().flags & IDENTIFIED, 0);
    run.clean("cancelled identify");
}
