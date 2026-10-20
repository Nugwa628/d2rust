//! The loopback relay: it forwards both ways, adds its latency, drops
//! what its loss and cut say, and follows a server that moved.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use mp_test::net::{Conditions, Relay};

/// The address a relay normally uses toward the server.
fn local() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

/// A loopback echo server: returns every datagram to its sender.
fn echo() -> (UdpSocket, std::net::SocketAddr) {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = s.local_addr().unwrap();
    let t = s.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut b = [0u8; 2048];
        while let Ok((n, from)) = t.recv_from(&mut b) {
            if &b[..n] == b"stop" {
                break;
            }
            let _ = t.send_to(&b[..n], from);
        }
    });
    (s, addr)
}

fn client(timeout_ms: u64) -> UdpSocket {
    let c = UdpSocket::bind("127.0.0.1:0").unwrap();
    c.set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .unwrap();
    c
}

fn round_trip(c: &UdpSocket, relay: &Relay, msg: &[u8]) -> Option<Duration> {
    let t = Instant::now();
    c.send_to(msg, relay.addr()).unwrap();
    let mut b = [0u8; 2048];
    let (n, _) = c.recv_from(&mut b).ok()?;
    assert_eq!(&b[..n], msg);
    Some(t.elapsed())
}

#[test]
fn it_forwards_and_adds_its_latency() {
    let (_e, addr) = echo();
    let relay = Relay::start(addr, 1, local()).unwrap();
    let c = client(2000);
    assert!(round_trip(&c, &relay, b"hello").is_some());
    relay.set(Conditions {
        latency_ms: 100,
        ..Conditions::default()
    });
    let rtt = round_trip(&c, &relay, b"slow").unwrap();
    assert!(
        rtt >= Duration::from_millis(200),
        "100 ms each way: {rtt:?}"
    );
}

#[test]
fn a_cut_drops_everything_and_a_mend_restores_it() {
    let (_e, addr) = echo();
    let relay = Relay::start(addr, 1, local()).unwrap();
    let c = client(300);
    relay.set(Conditions {
        cut: true,
        ..Conditions::default()
    });
    assert!(round_trip(&c, &relay, b"lost").is_none());
    assert!(relay.counts().1 >= 1, "counted as dropped");
    relay.set(Conditions::default());
    assert!(round_trip(&c, &relay, b"back").is_some());
}

#[test]
fn loss_drops_about_its_share_the_same_way_for_a_seed() {
    let run = |seed| {
        let (_e, addr) = echo();
        let relay = Relay::start(addr, seed, local()).unwrap();
        relay.set(Conditions {
            loss: 0.3,
            ..Conditions::default()
        });
        let c = client(100);
        let got: Vec<bool> = (0..60u8)
            .map(|i| round_trip(&c, &relay, &[i]).is_some())
            .collect();
        got
    };
    let a = run(42);
    let ok = a.iter().filter(|x| **x).count();
    // Both directions lose 30%: about half the round trips survive.
    assert!((15..=50).contains(&ok), "{ok} of 60 round trips");
    assert_eq!(a, run(42), "same seed, same losses");
}

#[test]
fn a_relay_speaks_from_the_bots_own_address() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let relay = Relay::start(addr, 1, "127.0.0.9".parse().unwrap()).unwrap();
    let c = client(1000);
    c.send_to(b"who", relay.addr()).unwrap();
    let mut b = [0u8; 16];
    let (n, from) = server.recv_from(&mut b).unwrap();
    assert_eq!(&b[..n], b"who");
    assert_eq!(from.ip(), "127.0.0.9".parse::<IpAddr>().unwrap());
}

#[test]
fn it_follows_a_server_that_moved() {
    let (_a, first) = echo();
    let (_b, second) = echo();
    let relay = Relay::start(first, 1, local()).unwrap();
    let c = client(1000);
    assert!(round_trip(&c, &relay, b"one").is_some());
    relay.retarget(second);
    assert!(round_trip(&c, &relay, b"two").is_some());
}

#[test]
fn settings_parse() {
    let mut c = Conditions::default();
    let kv = |s: &[(&str, &str)]| -> Vec<(String, String)> {
        s.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    c.apply(&kv(&[
        ("latency", "80ms"),
        ("jitter", "20"),
        ("loss", "5%"),
        ("cut", "on"),
    ]))
    .unwrap();
    assert_eq!((c.latency_ms, c.jitter_ms, c.cut), (80, 20, true));
    assert!((c.loss - 0.05).abs() < 1e-9);
    assert!(c.apply(&kv(&[("loss", "150")])).is_err());
    assert!(c.apply(&kv(&[("speed", "1")])).is_err());
}
