//! The client session against a scripted server over loopback QUIC: the
//! handshake (welcome, refusals, the certificate pin), requests and their
//! errors, and joining a game whose frames arrive whole.

use std::time::Duration;

use d2_net::msg::{ErrorCode, ErrorMsg, Writer};
use d2_net::wire::{
    kind, Attach, ClientHello, Refused, ResumeToken, ServerInfo, SessionWelcome, Welcome,
};
use d2_net::{Packet, Timing, Transport};
use d2_online::{fingerprint_hex, ClientInfo, OnlineError, ServerSpec, ServerTrust, Session};
use d2_quic::{fingerprint, FramedStream, Identity, PacketStream, QuicConn, QuicServer};

fn me() -> ClientInfo {
    ClientInfo {
        protocol: 2,
        game: "d2rust".into(),
        build: "0.1.2.0".into(),
        tick_rate: 60,
    }
}

fn info() -> ServerInfo {
    ServerInfo {
        protocol: 2,
        game: "d2rust".into(),
        build: "0.1.2.0".into(),
        tick_rate: 60,
    }
}

fn session_welcome() -> SessionWelcome {
    SessionWelcome {
        tick_rate: 60,
        max_players: 8,
        session: ResumeToken([5; 16]),
        server_name: "Test realm".into(),
        registration_open: true,
        resume: None,
    }
}

fn game_welcome() -> Welcome {
    Welcome {
        client: 2,
        tick_rate: 60,
        max_players: 8,
        resume: ResumeToken([9; 16]),
    }
}

/// A scripted server: accepts connections and runs `script` on each.
struct Server {
    quic: std::sync::Arc<QuicServer>,
    cert: Vec<u8>,
    /// The server runs on it; dropped last.
    _rt: tokio::runtime::Runtime,
}

impl Server {
    fn start<F, Fut>(script: F) -> Self
    where
        F: Fn(QuicConn, FramedStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let id = Identity::self_signed().unwrap();
        let cert = id.cert.clone();
        let quic = std::sync::Arc::new(
            QuicServer::bind(
                "127.0.0.1:0".parse().unwrap(),
                id,
                Timing::default(),
                rt.handle().clone(),
            )
            .unwrap(),
        );
        let q = quic.clone();
        let script = std::sync::Arc::new(script);
        rt.spawn(async move {
            while let Some((conn, control)) = q.accept().await {
                tokio::spawn(script(conn, control));
            }
        });
        Self {
            quic,
            cert,
            _rt: rt,
        }
    }

    fn spec(&self) -> ServerSpec {
        ServerSpec {
            id: "t1".into(),
            name: "Test".into(),
            game: "d2rust".into(),
            address: self.quic.local_addr().unwrap().to_string(),
            trust: ServerTrust::Pinned(fingerprint(&self.cert)),
            version: "0.1.2.0".into(),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.quic.close();
    }
}

/// The server half of the handshake: welcome a matching hello, refuse
/// another; `true` when welcomed.
async fn handshake(control: &mut FramedStream) -> bool {
    let Ok(body) = control.recv_async().await else {
        return false;
    };
    let verdict = ClientHello::parse(&body).and_then(|h| h.check(&info()));
    match verdict {
        Ok(()) => control.send(session_welcome().body()).is_ok(),
        Err(r) => {
            let _ = control.send(r.body());
            control.close();
            false
        }
    }
}

#[test]
fn a_matching_client_is_welcomed() {
    let srv = Server::start(|_conn, mut control| async move {
        if handshake(&mut control).await {
            // Keep the session open until the client leaves.
            let _ = control.recv_async().await;
        }
    });
    let s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();
    assert_eq!(s.welcome().server_name, "Test realm");
    assert_eq!(s.token(), ResumeToken([5; 16]));
    s.close();
}

#[test]
fn another_version_is_refused_with_the_reason() {
    let srv = Server::start(|_conn, mut control| async move {
        handshake(&mut control).await;
    });
    let old = ClientInfo {
        build: "0.1.1.0".into(),
        ..me()
    };
    let e = Session::connect(&srv.spec(), &old, Timing::default()).unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::BUILD), "{e:?}");
    assert_eq!(e.message(), ErrorCode::BUILD.text());
}

#[test]
fn a_server_without_the_pinned_certificate_is_not_trusted() {
    let srv = Server::start(|_conn, mut control| async move {
        handshake(&mut control).await;
    });
    let mut spec = srv.spec();
    spec.trust = ServerTrust::Pinned(fingerprint(b"another certificate"));
    let e = Session::connect(&spec, &me(), Timing::default()).unwrap_err();
    assert!(matches!(e, OnlineError::Connect(_)), "{e:?}");
    assert_eq!(fingerprint_hex(&fingerprint(&srv.cert)).len(), 64);
}

#[test]
fn a_request_gets_its_answer_or_its_error_and_keeps_the_rest() {
    let srv = Server::start(|_conn, mut control| async move {
        if !handshake(&mut control).await {
            return;
        }
        while let Ok(b) = control.recv_async().await {
            match b[0] {
                // "Login": a list update arrives first, then the answer.
                0x21 => {
                    let _ = control.send(Writer::new(0x44).u32(7).finish());
                    let _ = control.send(Writer::new(0x22).str8("Mogger").u8(16).finish());
                }
                // "Register": refused.
                0x20 => {
                    let mut e = ErrorMsg::new(0x20, ErrorCode::RATE_LIMITED);
                    e.wait_s = 30;
                    let _ = control.send(e.body());
                }
                _ => {}
            }
        }
    });
    let mut s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();
    let wait = Duration::from_secs(5);
    let answer = s.request(vec![0x21, 1], &[0x22], wait).unwrap();
    assert_eq!(answer[0], 0x22);
    // The list update waits for poll.
    assert_eq!(s.poll().unwrap().map(|b| b[0]), Some(0x44));
    let e = s.request(vec![0x20, 1], &[0x22], wait).unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::RATE_LIMITED));
    assert!(
        e.message().ends_with("Try again in 30 seconds."),
        "{}",
        e.message()
    );
}

/// A game server: attaches the ticket [1; 16], sends one frame (a tick
/// and one delivery), and refuses any other ticket.
async fn serve_game(conn: QuicConn, mut control: FramedStream) {
    if !handshake(&mut control).await {
        return;
    }
    while let Some(mut game) = conn.accept_stream().await {
        let Ok(first) = game.recv_async().await else {
            continue;
        };
        match Attach::parse(&first) {
            Ok(Attach::Ticket(t)) if t == [1; 16] => {
                let _ = game.send(game_welcome().body());
                let mut down = PacketStream::server(game);
                let _ = down.send(Packet::Frame { ticked: true });
                let _ = down.send(Packet::Deliver(vec![0x95, 1, 2]));
                let _ = down.flush();
                // Hold the stream open until the client leaves.
                while down.stream_mut().recv_async().await.is_ok() {}
            }
            _ => {
                let _ = game.send(Refused::new(ErrorCode::TICKET_INVALID).body());
                game.close();
            }
        }
    }
}

#[test]
fn joining_a_game_hands_out_whole_frames() {
    let srv = Server::start(serve_game);
    let mut s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();
    let e = s.join([2; 16]).unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::TICKET_INVALID));
    let mut g = s.join([1; 16]).unwrap();
    assert_eq!(g.welcome().client, 2);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let frame = loop {
        if let Some(f) = g.poll_frame().unwrap() {
            break f;
        }
        assert!(std::time::Instant::now() < deadline, "no frame");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(frame.ticked);
    assert_eq!(frame.delivers, [vec![0x95, 1, 2]]);
    g.send(d2_net::Lane::Game, &[0x03, 1]).unwrap();
    g.leave();
    // The session stays open after leaving the game.
    s.send(Writer::new(0x48).finish()).unwrap();
    assert_eq!(kind::FRAME_END, 0x0B);
}

#[test]
fn a_bot_reads_the_game_as_packets() {
    let srv = Server::start(serve_game);
    let mut s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();
    let mut g = s.join([1; 16]).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while got.len() < 2 {
        assert!(std::time::Instant::now() < deadline, "no packets");
        while let Some(p) = Transport::recv(&mut g).unwrap() {
            got.push(p);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        got,
        [
            Packet::Frame { ticked: true },
            Packet::Deliver(vec![0x95, 1, 2])
        ]
    );
    assert_eq!(
        Transport::send(&mut g, Packet::Deliver(vec![1])),
        Err(d2_net::NetError::Malformed)
    );
}

#[test]
fn a_bot_can_say_another_hello_to_be_refused() {
    let srv = Server::start(|_conn, mut control| async move {
        handshake(&mut control).await;
    });
    let options = d2_online::ConnectOptions {
        hello: Some(ClientHello {
            protocol: 2,
            game: "d2rust".into(),
            build: "0.1.2.0".into(),
            tick_rate: 25,
            resume: None,
        }),
    };
    let e = Session::connect_with(&srv.spec(), &me(), Timing::default(), &options).unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::TICK_RATE));
    assert_eq!(e.code().map(ErrorCode::name), Some("TickRate"));
    assert!(!e.text().is_empty());
}
