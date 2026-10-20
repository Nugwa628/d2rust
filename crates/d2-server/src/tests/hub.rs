//! Tests of the client connections (`crate::hub`): messages that come in over
//! a transport reach the host exactly as direct host calls do, clients are read
//! in id order, deliveries keep the receive order, and every frame is marked
//! on every connection. The game, session and clock are the fakes from
//! `fakes.rs`; the transport is `d2-net`'s in-memory pair, and QUIC over
//! loopback for the network tests. Lost clients are dropped, or held
//! for a grace period and resumed with their token.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use d2_net::wire::{parse_answer, refused_body};
use d2_net::{
    pair, ConnectError, Hello, Lane, MemoryEnd, NetError, Packet, Refusal, ResumeToken, Timing,
    Transport, Welcome,
};
use d2_quic::{Identity, PacketStream, QuicClient, QuicConn, QuicServer, ServerTrust};

use super::fakes::*;
use crate::host::Host;
use crate::hub::{Accepted, Hub, HubError};
use crate::seams::ClientId;
use crate::transport::{Classified, Queue};

type TestHost = Host<FakeGame, TsvSizes, FakeSession, ManualClock>;
type TestHub = Hub<FakeGame, TsvSizes, FakeSession, ManualClock, MemoryEnd>;

fn host() -> TestHost {
    let mut g = FakeGame::with_player(0, ALIVE);
    let p = g.players[&0].clone();
    g.players.insert(1, p);
    g.client_list = vec![1, 0];
    Host::new(
        g,
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    )
}

/// A hub with clients 0 and 1 connected; returns their client ends.
fn hub() -> (TestHub, MemoryEnd, MemoryEnd) {
    let mut hub = Hub::new(host());
    let (c0, s0) = pair();
    let (c1, s1) = pair();
    hub.connect(0, s0);
    hub.connect(1, s1);
    (hub, c0, c1)
}

fn send(end: &mut MemoryEnd, lane: Lane, msg: &[u8]) {
    end.send(Packet::Send {
        lane,
        msg: msg.to_vec(),
    })
    .unwrap();
}

/// Everything waiting on a client end.
fn drain(end: &mut MemoryEnd) -> Vec<Packet> {
    std::iter::from_fn(|| end.recv().unwrap()).collect()
}

const WALK: [u8; 5] = [0x03, 0x10, 0x00, 0x20, 0x00];
const RUN: [u8; 5] = [0x01, 100, 0, 100, 0];

/// The same sends, frames and receives give the same game whether they go
/// through the hub or straight to the host.
#[test]
fn hub_matches_direct_host_calls() {
    let mut direct = host();
    direct.connect(0);
    direct.connect(1);
    let (mut hub, mut c0, mut c1) = hub();
    for h in [&mut direct.game, &mut hub.host.game] {
        h.handler_out = vec![(0, vec![0x0C; 9])];
        h.tick_out = vec![(0, vec![0x01; 8]), (1, vec![0x01; 8])];
    }
    let mut got_direct = Vec::new();
    let mut got_hub = Vec::new();
    for step in 0..4u32 {
        direct.clock.0 = 1000 + 20 * step;
        hub.host.clock.0 = 1000 + 20 * step;
        // Client 0 then 1, as the hub reads them.
        direct.send_game(0, &RUN).unwrap();
        direct.send_system(0, &[0x6B]).unwrap();
        direct.send_game(1, &WALK).unwrap();
        send(&mut c1, Lane::Game, &WALK);
        send(&mut c0, Lane::Game, &RUN);
        send(&mut c0, Lane::System, &[0x6B]);
        hub.poll().unwrap();
        let a = direct.frame().unwrap();
        let b = hub.frame().unwrap();
        assert_eq!(a, b);
        for c in [0, 1] {
            got_direct.push(direct.receive(c));
            hub.deliver(c).unwrap();
        }
        for end in [&mut c0, &mut c1] {
            got_hub.push(
                drain(end)
                    .into_iter()
                    .filter_map(|p| match p {
                        Packet::Deliver(m) => Some(m),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            );
        }
    }
    assert_eq!(got_direct, got_hub);
    assert_eq!(direct.game.handled, hub.host.game.handled);
    assert_eq!(direct.session.seen, hub.host.session.seen);
}

/// A poll reads clients in id order, each client's packets in arrival
/// order, and reports what each sender did.
#[test]
fn poll_orders_by_client_then_arrival() {
    let (mut hub, mut c0, mut c1) = hub();
    send(&mut c1, Lane::Game, &WALK);
    send(&mut c0, Lane::Game, &RUN);
    send(&mut c0, Lane::Game, &RUN);
    send(&mut c1, Lane::System, &[0x6B]);
    let polled = hub.poll().unwrap();
    let order: Vec<(ClientId, u8)> = polled.iter().map(|p| (p.client, p.id)).collect();
    assert_eq!(order, [(0, 0x01), (0, 0x01), (1, 0x03), (1, 0x6B)]);
    let queued = Ok(Accepted::Classified(Classified::Queued(Queue::Game)));
    assert_eq!(polled[0].result, queued);
    // The repeat inside the duplicate filter's window.
    assert_eq!(polled[1].result, Ok(Accepted::Filtered));
    assert_eq!(polled[2].result, queued);
    assert_eq!(
        polled[3].result,
        Ok(Accepted::Classified(Classified::Queued(Queue::System)))
    );
    let drained: Vec<ClientId> = hub
        .host
        .frame()
        .unwrap()
        .messages
        .iter()
        .map(|m| m.client)
        .collect();
    // The system queue drains first, then the game queue in poll order.
    assert_eq!(drained, [1, 0, 1]);
}

/// Every frame is marked on every connection, with whether it ticked.
#[test]
fn frames_are_marked_on_every_connection() {
    let (mut hub, mut c0, mut c1) = hub();
    hub.frame().unwrap();
    hub.host.clock.0 = 1040;
    hub.frame().unwrap();
    for end in [&mut c0, &mut c1] {
        let marks = drain(end);
        assert_eq!(
            marks,
            [
                Packet::Frame { ticked: false },
                Packet::Frame { ticked: true }
            ]
        );
    }
}

/// A delivery keeps the host's receive order: the system list first.
#[test]
fn deliver_keeps_receive_order() {
    let (mut hub, mut c0, _c1) = hub();
    hub.host.send_direct(0, &[0x06]).unwrap();
    hub.host.send_direct(0, &[0xB0]).unwrap();
    assert_eq!(hub.deliver(0), Ok(2));
    assert_eq!(
        drain(&mut c0),
        [Packet::Deliver(vec![0xB0]), Packet::Deliver(vec![0x06])]
    );
    assert_eq!(hub.deliver(0), Ok(0));
}

/// Server-only packets from a client, unknown clients and closed
/// connections are errors; a disconnect forgets the client.
#[test]
fn connection_errors() {
    let (mut hub, mut c0, c1) = hub();
    c0.send(Packet::Frame { ticked: true }).unwrap();
    assert_eq!(hub.poll_client(0), Err(HubError::Unexpected { client: 0 }));
    assert_eq!(hub.deliver(7), Err(HubError::UnknownClient(7)));
    drop(c1);
    assert_eq!(
        hub.poll_client(1),
        Err(HubError::Net {
            client: 1,
            source: NetError::Closed
        })
    );
    assert!(hub.disconnect(1).is_some());
    assert_eq!(hub.clients().collect::<Vec<_>>(), [0]);
    assert!(hub.host.record(1).is_none());
}

/// Serving all clients drops the ones whose connection failed (after
/// taking what they sent before) and the others play on.
#[test]
fn failed_connections_are_dropped() {
    let (mut hub, mut c0, mut c1) = hub();
    send(&mut c1, Lane::Game, &WALK);
    drop(c1);
    c0.send(Packet::Frame { ticked: true }).unwrap();
    let polled = hub.poll().unwrap();
    assert_eq!(polled.len(), 1);
    assert_eq!(polled[0].client, 1);
    assert_eq!(
        hub.take_dropped(),
        [(0, NetError::Malformed), (1, NetError::Closed)]
    );
    assert_eq!(hub.clients().count(), 0);
    assert!(hub.take_dropped().is_empty());

    // A client that is gone when the frame marks go out.
    let (mut hub, c0, mut c1) = self::hub();
    drop(c0);
    hub.frame().unwrap();
    hub.deliver_all();
    assert_eq!(hub.take_dropped(), [(0, NetError::Closed)]);
    assert_eq!(drain(&mut c1), [Packet::Frame { ticked: false }]);
}

/// A network transport as the loopback hub test drives it: a listening
/// server that accepts with a decision, and clients that connect.
trait Net {
    type End: Transport + Send + 'static;
    type Server;
    fn bind() -> Self::Server;
    fn addr(server: &Self::Server) -> SocketAddr;
    fn connect(addr: SocketAddr, hello: &Hello) -> Result<(Self::End, Welcome), ConnectError>;
    fn accept(
        server: &Self::Server,
        decide: impl FnOnce(&Hello) -> Result<Welcome, Refusal>,
    ) -> Result<Option<(Self::End, Hello)>, ConnectError>;
    fn close(end: &mut Self::End);
}

/// A QUIC server on loopback with the runtime it runs on.
struct QuicLoopback {
    server: QuicServer,
    _rt: tokio::runtime::Runtime,
}

/// One end of a game connection over QUIC: the wire v2 handshake runs on
/// the connection's first stream, which then carries the game packets.
struct QuicEnd {
    stream: PacketStream,
    _conn: Option<QuicConn>,
    client: Option<QuicClient>,
}

impl Transport for QuicEnd {
    fn send(&mut self, packet: Packet) -> Result<(), NetError> {
        self.stream.send(packet)
    }
    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        self.stream.recv()
    }
    fn flush(&mut self) -> Result<(), NetError> {
        self.stream.flush()
    }
}

struct Quic;
impl Net for Quic {
    type End = QuicEnd;
    type Server = QuicLoopback;
    fn bind() -> QuicLoopback {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let server = QuicServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            Identity::self_signed().unwrap(),
            Timing::default(),
            rt.handle().clone(),
        )
        .unwrap();
        QuicLoopback { server, _rt: rt }
    }
    fn addr(server: &QuicLoopback) -> SocketAddr {
        server.server.local_addr().unwrap()
    }
    fn connect(addr: SocketAddr, hello: &Hello) -> Result<(QuicEnd, Welcome), ConnectError> {
        let timing = Timing::default();
        let (client, mut control) = QuicClient::connect(addr, ServerTrust::Any, timing)?;
        control
            .send(hello.body())
            .map_err(|_| ConnectError::Malformed("send hello"))?;
        let answer = control
            .recv_timeout(timing.timeout)
            .ok()
            .flatten()
            .ok_or(ConnectError::Malformed("no answer to hello"))?;
        let welcome = parse_answer(&answer)?;
        let end = QuicEnd {
            stream: PacketStream::client(control),
            _conn: None,
            client: Some(client),
        };
        Ok((end, welcome))
    }
    fn accept(
        server: &QuicLoopback,
        decide: impl FnOnce(&Hello) -> Result<Welcome, Refusal>,
    ) -> Result<Option<(QuicEnd, Hello)>, ConnectError> {
        let Some((conn, mut control)) = server.server.try_accept() else {
            return Ok(None);
        };
        let body = control
            .recv_timeout(Duration::from_secs(5))
            .ok()
            .flatten()
            .ok_or(ConnectError::Malformed("hello"))?;
        let hello = Hello::parse(&body).map_err(ConnectError::Refused)?;
        match decide(&hello) {
            Ok(w) => {
                control
                    .send(w.body())
                    .map_err(|_| ConnectError::Malformed("send welcome"))?;
                let end = QuicEnd {
                    stream: PacketStream::server(control),
                    _conn: Some(conn),
                    client: None,
                };
                Ok(Some((end, hello)))
            }
            Err(r) => {
                let _ = control.send(refused_body(&r));
                control.close();
                Err(ConnectError::Refused(r))
            }
        }
    }
    fn close(end: &mut QuicEnd) {
        end.stream.close();
        if let Some(c) = &mut end.client {
            c.close();
        }
    }
}

fn net_hello(resume: Option<ResumeToken>) -> Hello {
    Hello {
        protocol: 1,
        build: "test".into(),
        tick_rate: d2_sim::tick::TickRate::BASE.hz(),
        resume,
    }
}

/// Connects one client to `server` and waits until the server accepts it
/// with `decide`; returns the server's end, the client's, and the welcome.
fn join<N: Net>(
    server: &N::Server,
    hello: Hello,
    mut decide: impl FnMut(&Hello) -> Result<Welcome, Refusal>,
) -> (N::End, N::End, Welcome) {
    let addr = N::addr(server);
    let client = std::thread::spawn(move || N::connect(addr, &hello));
    let deadline = Instant::now() + Duration::from_secs(5);
    let (end, _) = loop {
        match N::accept(server, &mut decide).unwrap() {
            Some(x) => break x,
            None => {
                assert!(Instant::now() < deadline, "no client arrived");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    };
    let (c, w) = client.join().unwrap().unwrap();
    (end, c, w)
}

/// Polls `end` until `n` packets have arrived.
fn wait_for<T: Transport>(end: &mut T, n: usize) -> Vec<Packet> {
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while got.len() < n {
        assert!(Instant::now() < deadline, "packets did not arrive");
        while let Some(p) = end.recv().unwrap() {
            got.push(p);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    got
}

/// The same game over a loopback network: two clients connect with the
/// handshake, their messages reach the host in id-then-arrival order, the
/// deliveries and frame marks come back, and a client that hangs up is
/// dropped while the other plays on.
fn hub_over_loopback<N: Net>() {
    let server = N::bind();
    let mut hub: Hub<FakeGame, TsvSizes, FakeSession, ManualClock, N::End> = Hub::new(host());
    hub.host.game.handler_out = vec![(0, vec![0x0C; 9])];
    let mut ends = Vec::new();
    for id in [0, 1] {
        let (end, c, w) = join::<N>(&server, net_hello(None), |h| {
            h.check(&net_hello(None))?;
            Ok(Welcome {
                client: id,
                tick_rate: d2_sim::tick::TickRate::BASE.hz(),
                max_players: 8,
                resume: ResumeToken([id as u8; 16]),
            })
        });
        hub.connect(id, end);
        assert_eq!(w.client, id);
        ends.push(c);
    }
    let mut ends = ends.into_iter();
    let (mut c0, mut c1) = (ends.next().unwrap(), ends.next().unwrap());
    let send_net = |end: &mut N::End, lane, msg: &[u8]| {
        end.send(Packet::Send {
            lane,
            msg: msg.to_vec(),
        })
        .unwrap()
    };
    send_net(&mut c1, Lane::Game, &RUN);
    send_net(&mut c0, Lane::Game, &RUN);
    send_net(&mut c0, Lane::System, &[0x6B]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut polled = Vec::new();
    while polled.len() < 3 {
        assert!(Instant::now() < deadline, "sends did not arrive");
        polled.extend(hub.poll().unwrap());
        std::thread::sleep(Duration::from_millis(1));
    }
    // The first frame starts the tick driver; the second ticks and flushes.
    hub.frame().unwrap();
    hub.host.clock.0 += 40;
    hub.frame().unwrap();
    hub.deliver_all();
    assert_eq!(hub.host.game.handled.len(), 2);
    let want = [
        Packet::Frame { ticked: false },
        Packet::Frame { ticked: true },
        Packet::Deliver(vec![0x0C; 9]),
        Packet::Deliver(vec![0x0C; 9]),
    ];
    // Both handler messages (one per game message) went to client 0.
    assert_eq!(wait_for(&mut c0, want.len()), want);
    // Client 1 hangs up; the hub drops it and keeps client 0.
    N::close(&mut c1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while hub.clients().count() > 1 {
        assert!(Instant::now() < deadline, "the hang-up was not seen");
        hub.poll().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(hub.take_dropped(), [(1, NetError::Closed)]);
    assert_eq!(hub.clients().collect::<Vec<_>>(), [0]);
    assert!(hub.host.record(1).is_none());
}

#[test]
fn hub_over_loopback_quic() {
    hub_over_loopback::<Quic>();
}

/// With a grace period, a client whose connection is lost keeps its slot:
/// it comes back over a new connection with its token, gets what it
/// missed in order, and plays on. One that doesn't come back in time is
/// dropped when the period ends.
#[test]
fn a_lost_client_resumes_within_the_grace_period() {
    let mut hub: TestHub = Hub::new(host());
    hub.set_grace(30_000);
    hub.host.game.tick_out = vec![(0, vec![0x01; 8]), (1, vec![0x01; 8])];
    let (mut c0, s0) = pair();
    let (c1, s1) = pair();
    hub.connect_with_token(0, s0, ResumeToken([0; 16]));
    hub.connect_with_token(1, s1, ResumeToken([1; 16]));
    hub.frame().unwrap();
    // Client 1's connection drops.
    drop(c1);
    hub.poll().unwrap();
    assert_eq!(hub.clients().collect::<Vec<_>>(), [0]);
    assert_eq!(hub.parked().collect::<Vec<_>>(), [1]);
    assert!(hub.take_dropped().is_empty());
    assert!(hub.host.record(1).is_some(), "still in the game");
    // The game goes on: a tick with a message for each client.
    hub.host.clock.0 += 40;
    hub.frame().unwrap();
    hub.deliver_all();
    assert_eq!(
        drain(&mut c0),
        [
            Packet::Frame { ticked: false },
            Packet::Frame { ticked: true },
            Packet::Deliver(vec![0x01; 8]),
        ]
    );
    // A wrong token gets nothing; the right one gets the slot and the
    // missed frame mark and message.
    let (_stranger, s) = pair();
    assert_eq!(hub.resume(&ResumeToken([9; 16]), s), None);
    assert_eq!(hub.resumable(&ResumeToken([1; 16])), Some(1));
    let (mut c1, s1) = pair();
    assert_eq!(hub.resume(&ResumeToken([1; 16]), s1), Some(1));
    assert_eq!(
        drain(&mut c1),
        [
            Packet::Frame { ticked: true },
            Packet::Deliver(vec![0x01; 8])
        ]
    );
    assert_eq!(hub.clients().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(hub.parked().count(), 0);
    send(&mut c1, Lane::Game, &WALK);
    assert_eq!(hub.poll().unwrap()[0].client, 1);

    // Lost again, and this time it doesn't come back in time.
    drop(c1);
    hub.poll().unwrap();
    hub.host.clock.0 += 29_999;
    hub.frame().unwrap();
    assert_eq!(hub.parked().collect::<Vec<_>>(), [1]);
    hub.host.clock.0 += 1;
    hub.frame().unwrap();
    assert_eq!(hub.parked().count(), 0);
    assert_eq!(hub.take_dropped(), [(1, NetError::Closed)]);
    assert!(hub.host.record(1).is_none());
    let (_late, s1) = pair();
    assert_eq!(hub.resume(&ResumeToken([1; 16]), s1), None);
}

/// A re-entry (a resume whose game sends its state again, contracts 3.7)
/// takes the slot over a new connection with a new token and does not
/// replay the backlog parked while it was away.
#[test]
fn a_reattached_client_gets_no_parked_backlog_and_a_new_token() {
    let mut hub: TestHub = Hub::new(host());
    hub.set_grace(30_000);
    let (_c0, s0) = pair();
    let (c1, s1) = pair();
    hub.connect_with_token(0, s0, ResumeToken([0; 16]));
    hub.connect_with_token(1, s1, ResumeToken([1; 16]));
    hub.frame().unwrap();
    hub.deliver_all();
    // Client 1 drops; the game keeps sending to it while parked.
    drop(c1);
    hub.poll().unwrap();
    assert_eq!(hub.parked().collect::<Vec<_>>(), [1]);
    hub.host.game.tick_out = vec![(1, vec![0x42; 8])];
    hub.host.clock.0 += 40;
    hub.frame().unwrap();
    hub.deliver_all();

    let (mut c1, s1) = pair();
    let new = ResumeToken([7; 16]);
    assert_eq!(hub.reattach(&ResumeToken([1; 16]), new, s1), Some(1));
    assert_eq!(drain(&mut c1), [], "the parked backlog is not replayed");
    assert_eq!(hub.resumable(&ResumeToken([1; 16])), None);
    assert_eq!(hub.resumable(&new), Some(1));
    assert_eq!(hub.clients().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(hub.parked().count(), 0);

    // The client plays on: the next frame reaches it.
    hub.host.game.tick_out = vec![(1, vec![0x42; 8])];
    hub.host.clock.0 += 40;
    hub.frame().unwrap();
    hub.deliver_all();
    assert!(
        drain(&mut c1)
            .iter()
            .any(|p| matches!(p, Packet::Deliver(_))),
        "a reattached client gets the game's next deliveries"
    );
}

/// Without a grace period, or without a token, or when the client broke
/// the wire format, a failed client is dropped at once.
#[test]
fn some_losses_are_not_held() {
    let mut hub: TestHub = Hub::new(host());
    let (c0, s0) = pair();
    hub.connect_with_token(0, s0, ResumeToken([0; 16]));
    drop(c0);
    hub.poll().unwrap();
    assert_eq!(
        hub.take_dropped(),
        [(0, NetError::Closed)],
        "no grace period"
    );

    hub.set_grace(30_000);
    let (c0, s0) = pair();
    hub.connect(0, s0);
    drop(c0);
    hub.poll().unwrap();
    assert_eq!(hub.take_dropped(), [(0, NetError::Closed)], "no token");

    let (mut c0, s0) = pair();
    hub.connect_with_token(0, s0, ResumeToken([0; 16]));
    c0.send(Packet::Frame { ticked: true }).unwrap();
    hub.poll().unwrap();
    assert_eq!(
        hub.take_dropped(),
        [(0, NetError::Malformed)],
        "broke the wire"
    );
    assert_eq!(hub.parked().count(), 0);
}

/// A client that comes back over QUIC before the server noticed the loss
/// takes over its slot from the stale connection.
#[test]
fn resume_over_quic_replaces_a_stale_connection() {
    let server = Quic::bind();
    let mut hub: Hub<FakeGame, TsvSizes, FakeSession, ManualClock, QuicEnd> = Hub::new(host());
    hub.set_grace(30_000);
    let token = ResumeToken::random();
    let welcome = |client| Welcome {
        client,
        tick_rate: d2_sim::tick::TickRate::BASE.hz(),
        max_players: 8,
        resume: token,
    };
    let (end, _old, w) = join::<Quic>(&server, net_hello(None), |_| Ok(welcome(0)));
    hub.connect_with_token(w.client, end, w.resume);
    // The client lost its network and dials again with its token.
    let (end, mut c, w) = join::<Quic>(&server, net_hello(Some(token)), |h| {
        let back = h
            .resume
            .and_then(|t| hub.resumable(&t))
            .ok_or(Refusal::UnknownResume)?;
        Ok(welcome(back))
    });
    assert_eq!(w.client, 0);
    assert_eq!(hub.resume(&token, end), Some(0));
    hub.frame().unwrap();
    hub.deliver_all();
    assert_eq!(wait_for(&mut c, 1), [Packet::Frame { ticked: false }]);
}
