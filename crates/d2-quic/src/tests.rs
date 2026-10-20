//! Tests of the QUIC stream layer over loopback: bodies in order both
//! ways on the control stream and on a second stream, certificate
//! pinning by fingerprint, clean and dropped ends, keepalive, timeouts,
//! connection migration, and the game stream's frame batching.

use std::thread;
use std::time::{Duration, Instant};

use super::*;
use d2_net::Lane;

fn quick() -> Timing {
    Timing {
        keepalive: Duration::from_millis(50),
        timeout: Duration::from_millis(600),
    }
}

/// A server on loopback with the runtime it runs on (dropped after it).
struct Server {
    quic: QuicServer,
    rt: Runtime,
}

fn server(timing: Timing) -> Server {
    server_with(Identity::self_signed().unwrap(), timing)
}

fn server_with(identity: Identity, timing: Timing) -> Server {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let quic = QuicServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        identity,
        timing,
        rt.handle().clone(),
    )
    .unwrap();
    Server { quic, rt }
}

impl Server {
    fn addr(&self) -> SocketAddr {
        self.quic.local_addr().unwrap()
    }

    /// Waits for the next connection whose control stream has opened.
    fn accept(&self) -> (QuicConn, FramedStream) {
        self.rt
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), self.quic.accept()).await
            })
            .expect("no client arrived")
            .expect("the server is open")
    }

    fn accept_stream(&self, conn: &QuicConn) -> FramedStream {
        self.rt
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), conn.accept_stream()).await
            })
            .expect("no stream arrived")
            .expect("the connection is open")
    }
}

/// A connected client and the server's ends, after one body went up on
/// the control stream (the server only sees a stream once it carries data).
fn connected(timing: Timing) -> (QuicClient, FramedStream, QuicConn, FramedStream, Server) {
    let srv = server(timing);
    let (client, mut c) = QuicClient::connect(srv.addr(), ServerTrust::Any, timing).unwrap();
    c.send(b"hello".to_vec()).unwrap();
    let (conn, mut s) = srv.accept();
    assert_eq!(next(&mut s), Ok(b"hello".to_vec()));
    (client, c, conn, s, srv)
}

/// Polls `end` until a body arrives (or the stream ends).
fn next(end: &mut FramedStream) -> Result<Vec<u8>, NetError> {
    end.recv_timeout(Duration::from_secs(5))
        .map(|b| b.expect("nothing arrived"))
}

fn next_packet(end: &mut PacketStream) -> Result<Packet, NetError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(p) = end.recv()? {
            return Ok(p);
        }
        assert!(Instant::now() < deadline, "nothing arrived");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn bodies_cross_in_order_both_ways() {
    let (_client, mut c, _conn, mut s, _srv) = connected(Timing::default());
    for i in 0..50u8 {
        c.send(vec![0x20, i]).unwrap();
    }
    for i in 0..50u8 {
        assert_eq!(next(&mut s), Ok(vec![0x20, i]));
        s.send(vec![0x22, i]).unwrap();
    }
    for i in 0..50u8 {
        assert_eq!(next(&mut c), Ok(vec![0x22, i]));
    }
    assert_eq!(c.recv(), Ok(None));
}

#[test]
fn bodies_outside_the_frame_limits_are_refused() {
    let (_client, mut c, _conn, _s, _srv) = connected(Timing::default());
    assert_eq!(c.send(Vec::new()), Err(NetError::Malformed));
    assert_eq!(c.send(vec![1; MAX_FRAME + 1]), Err(NetError::Malformed));
    assert_eq!(c.send(vec![1; MAX_FRAME]), Ok(()));
}

/// The game stream is a second stream on the same connection: it carries
/// its own bodies, and ending it leaves the control stream open.
#[test]
fn a_second_stream_is_independent_of_the_control_stream() {
    let (client, mut c, conn, mut s, srv) = connected(Timing::default());
    let mut cg = client.open_stream().unwrap();
    cg.send(vec![9, 1]).unwrap();
    let mut sg = srv.accept_stream(&conn);
    assert_eq!(next(&mut sg), Ok(vec![9, 1]));
    sg.send(vec![5, 5]).unwrap();
    s.send(vec![0x41]).unwrap();
    assert_eq!(next(&mut cg), Ok(vec![5, 5]));
    assert_eq!(next(&mut c), Ok(vec![0x41]));
    // Leaving the game ends the game stream only.
    cg.close();
    assert_eq!(next(&mut sg), Err(NetError::Closed));
    c.send(vec![0x24]).unwrap();
    assert_eq!(next(&mut s), Ok(vec![0x24]));
    // A new game stream on the same connection works.
    let mut cg = client.open_stream().unwrap();
    cg.send(vec![9, 2]).unwrap();
    let mut sg = srv.accept_stream(&conn);
    assert_eq!(next(&mut sg), Ok(vec![9, 2]));
}

#[test]
fn pinned_trust_takes_only_that_fingerprint() {
    let id = Identity::self_signed().unwrap();
    let srv = server_with(id.clone(), Timing::default());
    assert_eq!(srv.quic.cert(), id.cert.as_slice());
    let right = ServerTrust::Pinned(fingerprint(&id.cert));
    let (_client, mut c) = QuicClient::connect(srv.addr(), right, Timing::default()).unwrap();
    c.send(vec![1]).unwrap();
    let (_conn, mut s) = srv.accept();
    assert_eq!(next(&mut s), Ok(vec![1]));

    let other = Identity::self_signed().unwrap();
    let wrong = ServerTrust::Pinned(fingerprint(&other.cert));
    assert!(QuicClient::connect(srv.addr(), wrong, Timing::default()).is_err());
}

#[test]
fn fingerprints_read_and_write_as_hex() {
    let fp = fingerprint(b"a certificate");
    let hex = fingerprint_hex(&fp);
    assert_eq!(hex.len(), 64);
    assert!(hex
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(parse_fingerprint(&hex), Some(fp));
    assert_eq!(parse_fingerprint(&hex.to_uppercase()), Some(fp));
    assert_eq!(parse_fingerprint(&hex[..62]), None);
    assert_eq!(parse_fingerprint(&format!("{}zz", &hex[..62])), None);
}

#[test]
fn clean_close_is_closed_after_the_last_body() {
    let (_client, mut c, _conn, mut s, _srv) = connected(Timing::default());
    c.send(vec![1, 2, 3]).unwrap();
    c.close();
    assert_eq!(next(&mut s), Ok(vec![1, 2, 3]));
    assert_eq!(next(&mut s), Err(NetError::Closed));
    assert_eq!(s.recv(), Err(NetError::Closed));
    assert_eq!(c.send(vec![4]), Err(NetError::Closed));
}

#[test]
fn a_closed_client_connection_closes_the_server_streams() {
    let (client, _c, _conn, mut s, _srv) = connected(Timing::default());
    drop(client);
    assert_eq!(next(&mut s), Err(NetError::Closed));

    // And the other way: the server closes a client's connection.
    let (_client, mut c, conn, _s, _srv) = connected(Timing::default());
    conn.close(7, "kicked");
    assert_eq!(next(&mut c), Err(NetError::Closed));
}

#[test]
fn keepalive_holds_an_idle_connection_open() {
    let (_client, mut c, _conn, mut s, _srv) = connected(quick());
    // Three timeouts of silence. QUIC keeps the connection alive by itself,
    // so neither end has to poll.
    thread::sleep(quick().timeout * 3);
    assert_eq!(c.recv(), Ok(None));
    assert_eq!(s.recv(), Ok(None));
    s.send(vec![6, 1]).unwrap();
    assert_eq!(next(&mut c), Ok(vec![6, 1]));
}

#[test]
fn a_vanished_peer_times_out() {
    let (client, _c, _conn, mut s, _srv) = connected(quick());
    // The client vanishes (its machine loses the network): no goodbye and
    // no close leave, and the server hears nothing more.
    client.vanish();
    let start = Instant::now();
    let r = loop {
        match s.recv() {
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            other => break other,
        }
    };
    assert_eq!(r, Err(NetError::TimedOut));
    assert!(start.elapsed() >= quick().timeout / 2);
}

/// A client that changes network keeps its connection: the server follows
/// it to the new address and the game sees nothing.
#[test]
fn a_client_that_moves_keeps_its_connection() {
    let (client, mut c, conn, mut s, _srv) = connected(Timing::default());
    let before = conn.peer();
    client.rebind("127.0.0.1:0".parse().unwrap()).unwrap();
    for i in 0..20u8 {
        c.send(vec![0x20, i]).unwrap();
        assert_eq!(next(&mut s), Ok(vec![0x20, i]));
        s.send(vec![0x22, i]).unwrap();
        assert_eq!(next(&mut c), Ok(vec![0x22, i]));
    }
    assert_ne!(
        conn.peer().port(),
        before.port(),
        "the server follows the client"
    );
}

/// The server's game stream holds a frame's packets until the frame ends,
/// then writes them with `FRAME_END`; the client's sends go out at once.
#[test]
fn the_game_stream_writes_a_frame_at_its_end() {
    let (client, _c, conn, _s, srv) = connected(Timing::default());
    let mut up = PacketStream::client(client.open_stream().unwrap());
    up.send(Packet::Send {
        lane: Lane::Game,
        msg: vec![0x01, 2],
    })
    .unwrap();
    let mut down = PacketStream::server(srv.accept_stream(&conn));
    assert_eq!(
        next_packet(&mut down),
        Ok(Packet::Send {
            lane: Lane::Game,
            msg: vec![0x01, 2]
        })
    );
    down.send(Packet::Frame { ticked: true }).unwrap();
    down.send(Packet::Deliver(vec![0x95, 1])).unwrap();
    // Nothing leaves before the frame ends.
    thread::sleep(Duration::from_millis(100));
    assert_eq!(up.stream_mut().recv(), Ok(None));
    down.flush().unwrap();
    let raw = up.stream_mut();
    assert_eq!(next(raw), Ok(vec![kind::FRAME, 1]));
    assert_eq!(next(raw), Ok(vec![kind::DELIVER, 0x95, 1]));
    assert_eq!(next(raw), Ok(vec![kind::FRAME_END]));
    // A packet reader skips the frame's end.
    down.send(Packet::Frame { ticked: false }).unwrap();
    down.flush().unwrap();
    down.send(Packet::Deliver(vec![7])).unwrap();
    down.flush().unwrap();
    assert_eq!(next_packet(&mut up), Ok(Packet::Frame { ticked: false }));
    assert_eq!(next_packet(&mut up), Ok(Packet::Deliver(vec![7])));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(up.recv(), Ok(None));
}
