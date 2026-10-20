//! The online smoke on a real `d2rust-server` (tools/mp-test/online-smoke.sh
//! starts one on loopback and passes its address and certificate): the
//! game's own screens and link, as a player uses them. A new account
//! registers, creates a character, selects it, creates a game in the lobby
//! and joins it; the game's remote link then sends the character's join
//! (C→S 0x67, then 0x6B after S→C 0x02, as the bridge does) and the server
//! puts the player into the world (S→C 0x59 with the character's name).

#![cfg(feature = "online")]

use std::time::{Duration, Instant};

use d2_client::app::online::{self as app_online, JoinTask, OnlineJoin, ServerList};
use d2_client::app::single_player;
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_client::bridge::remote::RemoteLink;
use d2_client::ui::front_end::screens::online::net_link::{entries, NetLink};
use d2_client::ui::front_end::screens::online::{self, *};
use d2_client::ui::front_end::*;
use d2_online::{parse_fingerprint, ServerTrust};
use d2_sim::tick::TickRate;

const TAB: u16 = 9;
const ENTER: u16 = 13;

struct Smoke {
    f: FrontEnd,
}

impl Smoke {
    fn ids(&self) -> Vec<String> {
        self.f.ui_elements().into_iter().map(|e| e.id).collect()
    }

    fn texts(&mut self) -> Vec<String> {
        let mut items = self.f.draw();
        items.extend(self.f.overlay(&|_, t| t.len() as i32 * 8));
        items
            .into_iter()
            .filter_map(|i| match i {
                DrawItem::Text { text, .. } if !text.is_empty() => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Clicks `id` once it is shown (the server's answers take real time).
    fn click(&mut self, id: &str) {
        let end = Instant::now() + Duration::from_secs(30);
        let e = loop {
            if let Some(e) = self.f.ui_elements().into_iter().find(|e| e.id == id) {
                break e;
            }
            assert!(
                Instant::now() < end,
                "no {id} on {:?}: {:?}",
                self.f.current(),
                self.ids()
            );
            std::thread::sleep(Duration::from_millis(10));
            self.f.tick();
        };
        self.f.input(FrontInput::Down(e.at));
        self.f.input(FrontInput::Up(e.at));
        self.f.tick();
        self.f.tick();
    }

    fn key(&mut self, vk: u16) {
        self.f.input(FrontInput::Key(vk));
        self.f.tick();
    }

    fn typed(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.f.input(FrontInput::Char(u));
        }
        self.f.tick();
    }

    /// Ticks in real time until `done` (at most 30 s).
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let end = Instant::now() + Duration::from_secs(30);
        while !done(self) {
            let shown = self.texts();
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}: {shown:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
            self.f.tick();
        }
        eprintln!("smoke: {what}");
    }
}

#[test]
#[ignore = "needs a running d2rust-server: tools/mp-test/online-smoke.sh"]
fn online_smoke_register_create_join_and_enter_the_world() {
    let address = std::env::var("D2RUST_SMOKE_ADDR").expect("D2RUST_SMOKE_ADDR");
    let cert = std::env::var("D2RUST_SMOKE_CERT").expect("D2RUST_SMOKE_CERT");
    let t0 = Instant::now();
    let list = ServerList {
        servers: vec![app_online::ServerEntry {
            id: "smoke".into(),
            name: "Smoke".into(),
            game: "d2rust".into(),
            address,
            trust: ServerTrust::Pinned(parse_fingerprint(&cert).expect("the certificate")),
            version: d2_client::launch::VERSION.into(),
        }],
    };
    let session = app_online::OnlineSession::default();
    let servers = entries(&list);
    let online = Online::new(Box::new(NetLink::new(session.clone(), list)), servers);
    let mut reg = Registry::default();
    screens::register_all(&mut reg);
    online::register_with(&mut reg, online.clone());
    let mut f = FrontEnd::new(true, Box::new(false), reg);
    f.start(false);
    let mut s = Smoke { f };
    s.f.trigger(Trigger::Online);
    s.f.tick();
    assert_eq!(s.f.current(), ONLINE_LOGIN);

    // A new account (a fresh name per run: the server keeps accounts).
    let account = format!("smoke{}", std::process::id() % 100_000);
    s.click("online_login.new_account");
    s.typed(&account);
    s.key(TAB);
    s.typed("password1");
    s.key(TAB);
    s.typed("password1");
    s.key(ENTER);
    s.until("registered and logged in", |s| {
        s.f.current() == ONLINE_CHARS
    });

    // A new sorceress.
    s.click("online_chars.create_new");
    assert_eq!(s.f.current(), ONLINE_CREATE);
    s.click("online_create.class.sorceress");
    for _ in 0..30 {
        s.f.tick();
    }
    s.typed("SmokeSorc");
    s.key(ENTER);
    s.until("the character created", |s| {
        s.f.current() == ONLINE_CHARS && s.ids().contains(&"online_chars.char(SmokeSorc)".into())
    });
    s.click("online_chars.char(SmokeSorc)");
    s.click("online_chars.ok");
    s.until("the lobby", |s| s.f.current() == ONLINE_LOBBY);

    // A new game, joined at once.
    s.click("online_lobby.tab_create");
    s.click("online_lobby.field(0)");
    s.typed("smoke game");
    s.click("online_lobby.primary");
    s.until(
        "the game created and joined",
        |s| matches!(s.f.outcome(), Some(Outcome::GameLoad(g)) if g.online),
    );

    // The game: the lobby's ticket on the game stream, the remote link.
    let join = OnlineJoin::take(&mut online.borrow_mut()).expect("the lobby join");
    assert_eq!((join.name.as_str(), join.class), ("SmokeSorc", 1));
    let task = JoinTask::start(&session, join.ticket).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    let game = loop {
        if let Some(a) = task.poll() {
            break a.unwrap_or_else(|e| panic!("the join: {}", e.message()));
        }
        assert!(Instant::now() < end, "no answer to the join");
        std::thread::sleep(Duration::from_millis(5));
    };
    eprintln!("smoke: the game stream attached");
    let rate = TickRate::new(game.welcome().tick_rate).expect("the server's tick rate");
    let mut link = RemoteLink::new(game, rate);
    let character = single_player::new_character("sorceress", "SmokeSorc").unwrap();
    let mut request = single_player::create_request_for(&character);
    request.difficulty = join.difficulty;
    link.send(SendQueue::System, &request.encode()).unwrap();
    let mut answered = false;
    let end = Instant::now() + Duration::from_secs(60);
    let assigned = 'world: loop {
        assert!(Instant::now() < end, "the player never entered the world");
        link.pump().expect("the link");
        for m in link.receive() {
            if m.first() == Some(&0x02) && !answered {
                answered = true;
                link.send(SendQueue::System, &[0x6B]).unwrap();
            }
            if m.first() == Some(&0x59) && m.len() >= 22 && &m[6..15] == b"SmokeSorc" {
                break 'world m;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(answered, "S→C 0x02 came before the player");
    assert_eq!(assigned[5], 1, "a sorceress");
    eprintln!(
        "smoke: SmokeSorc entered the world (S→C 0x59) {:.1} s after the start",
        t0.elapsed().as_secs_f64()
    );
}
