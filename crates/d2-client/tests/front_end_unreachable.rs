//! The online screens on the real link ([`NetLink`]) when the server can't
//! be reached: a firewall that drops the packets (a silent UDP port) and a
//! closed port. Shipped builds have online play on while the server may be
//! down, so: nothing connects before LOG IN, the front end never waits on
//! the network, the login screen says plainly that the server can't be
//! reached, and LOG IN again tries again.

#![cfg(feature = "online")]

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use d2_client::app::online::{OnlineSession, ServerList};
use d2_client::ui::front_end::screens::online::net_link::{entries, NetLink};
use d2_client::ui::front_end::screens::online::{self, *};
use d2_client::ui::front_end::*;

const TAB: u16 = 9;
const ENTER: u16 = 13;
const REACH: &str = "Could not reach the server.";
/// `Session::connect` gives up after about two 10 s timeouts.
const DEADLINE: Duration = Duration::from_secs(40);

/// The built-in list (the D2RUST server, its pin) moved to `address`.
fn front_end(address: String) -> FrontEnd {
    let mut list = ServerList::builtin();
    list.servers[0].address = address;
    let servers = entries(&list);
    let online = Online::new(
        Box::new(NetLink::new(OnlineSession::default(), list)),
        servers,
    );
    let mut reg = Registry::default();
    screens::register_all(&mut reg);
    online::register_with(&mut reg, online);
    let mut f = FrontEnd::new(true, Box::new(false), reg);
    f.start(false);
    f
}

fn texts(f: &mut FrontEnd) -> Vec<String> {
    let mut items = f.draw();
    items.extend(f.overlay(&|_, t| t.len() as i32 * 8));
    items
        .into_iter()
        .filter_map(|i| match i {
            DrawItem::Text { text, .. } if !text.is_empty() => Some(text),
            _ => None,
        })
        .collect()
}

fn shows(f: &mut FrontEnd, s: &str) -> bool {
    texts(f).iter().any(|t| t.contains(s))
}

fn key(f: &mut FrontEnd, vk: u16) {
    f.input(FrontInput::Key(vk));
    f.tick();
}

fn typed(f: &mut FrontEnd, s: &str) {
    for u in s.encode_utf16() {
        f.input(FrontInput::Char(u));
    }
    f.tick();
}

/// Ticks the front end until the login screen shows the failure; returns
/// how long that took and the longest single tick.
fn until_failure(f: &mut FrontEnd) -> (Duration, Duration) {
    let t0 = Instant::now();
    let mut longest = Duration::ZERO;
    while !shows(f, REACH) {
        assert!(
            t0.elapsed() < DEADLINE,
            "no failure after {DEADLINE:?}: {:?}",
            texts(f)
        );
        let t = Instant::now();
        f.tick();
        longest = longest.max(t.elapsed());
        std::thread::sleep(Duration::from_millis(20));
    }
    (t0.elapsed(), longest)
}

fn unreachable_server_says_so_and_retries(address: String) {
    let t0 = Instant::now();
    let mut f = front_end(address);
    f.trigger(Trigger::Online);
    f.tick();
    assert_eq!(f.current(), ONLINE_LOGIN);
    // Opening the screens touches no network.
    assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
    assert!(
        shows(&mut f, "SERVER: D2RUST EUROPE"),
        "{:?}",
        texts(&mut f)
    );
    assert!(!shows(&mut f, REACH));

    typed(&mut f, "mogger");
    key(&mut f, TAB);
    typed(&mut f, "password1");
    key(&mut f, ENTER);
    assert!(
        shows(&mut f, "Connecting to D2RUST Europe..."),
        "{:?}",
        texts(&mut f)
    );
    let (took, longest) = until_failure(&mut f);
    assert!(
        longest < Duration::from_millis(250),
        "a tick waited {longest:?}"
    );
    assert_eq!(f.current(), ONLINE_LOGIN);
    eprintln!("unreachable: failure shown after {took:?}, longest tick {longest:?}");

    // OK, then LOG IN again: a new attempt, failing the same way.
    let ok = f
        .ui_elements()
        .into_iter()
        .find(|e| e.id == "online_login.message_ok")
        .expect("the message's OK");
    f.input(FrontInput::Down(ok.at));
    f.input(FrontInput::Up(ok.at));
    f.tick();
    assert!(!shows(&mut f, REACH));
    key(&mut f, ENTER);
    assert!(
        shows(&mut f, "Connecting to D2RUST Europe..."),
        "{:?}",
        texts(&mut f)
    );
    let (again, _) = until_failure(&mut f);
    eprintln!("unreachable: retry failed after {again:?}");
}

/// A firewall that drops the packets: a UDP port that never answers.
#[test]
fn a_silent_server_port_says_it_cannot_be_reached_and_retries() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = silent.local_addr().unwrap().to_string();
    unreachable_server_says_so_and_retries(address);
    drop(silent);
}

/// Nothing listens on the port.
#[test]
fn a_closed_server_port_says_it_cannot_be_reached_and_retries() {
    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    unreachable_server_says_so_and_retries(format!("127.0.0.1:{port}"));
}
