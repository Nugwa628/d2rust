//! The account side of a session against a scripted server over loopback
//! QUIC: register, login (and a wrong password), the character list,
//! create, select and delete, and that a refusal is the server's error
//! while unrelated bodies wait for `poll`.

use std::time::{Duration, Instant};

use d2_net::msg::account::{kind, AccountEvent, AccountRequest, CharSummary};
use d2_net::msg::{self, ErrorCode, ErrorMsg};
use d2_net::wire::{ClientHello, ResumeToken, ServerInfo, SessionWelcome};
use d2_net::Timing;
use d2_online::{ClientInfo, OnlineError, ServerSpec, ServerTrust, Session, SessionEvent};
use d2_quic::{fingerprint, FramedStream, Identity, QuicConn, QuicServer};

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

fn character(name: &str, class: u8) -> CharSummary {
    CharSummary {
        name: name.into(),
        class,
        level: 42,
        status: 0,
        difficulties: 0b001,
        look: vec![1, 2, 3],
        last_played: 1_700_000_000,
    }
}

/// A scripted server: accepts connections and runs `script` on each.
struct Server {
    quic: std::sync::Arc<QuicServer>,
    cert: Vec<u8>,
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
/// another.
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

/// The account server: answers the account requests the way the contract
/// says, with a wrong-password login and a classic Druid refused.
async fn serve_accounts(_conn: QuicConn, mut control: FramedStream) {
    if !handshake(&mut control).await {
        return;
    }
    while let Ok(b) = control.recv_async().await {
        let Ok(req) = AccountRequest::parse(&b) else {
            continue;
        };
        match req {
            AccountRequest::Register { account, .. } => {
                let answer = AccountEvent::LoggedIn {
                    account,
                    max_characters: 8,
                };
                let _ = control.send(answer.body());
            }
            AccountRequest::Login { password, .. } => {
                if password == "open sesame" {
                    let answer = AccountEvent::LoggedIn {
                        account: "Mogger".into(),
                        max_characters: 8,
                    };
                    let _ = control.send(answer.body());
                } else {
                    let _ = control.send(ErrorMsg::new(kind::LOGIN, ErrorCode::WRONG_LOGIN).body());
                }
            }
            AccountRequest::Logout => return,
            AccountRequest::CharList => {
                // Two bodies that are not this request's answer arrive
                // first: they stay for `poll`.
                let _ = control.send(d2_net::msg::Writer::new(0x44).u32(7).finish());
                let _ = control.send(ErrorMsg::new(kind::LOGIN, ErrorCode::WRONG_LOGIN).body());
                let answer =
                    AccountEvent::CharList(vec![character("Mogger", 1), character("Sorc", 1)]);
                let _ = control.send(answer.body());
            }
            AccountRequest::CharCreate {
                name,
                class,
                expansion,
                ..
            } => {
                if class == 5 && !expansion {
                    let e = ErrorMsg::new(kind::CHAR_CREATE, ErrorCode::EXPANSION_CLASS);
                    let _ = control.send(e.body());
                } else {
                    let _ = control.send(AccountEvent::CharCreated(character(&name, class)).body());
                }
            }
            AccountRequest::CharDelete { name, password } => {
                if password == "open sesame" {
                    let _ = control.send(AccountEvent::CharDeleted { name }.body());
                } else {
                    let _ = control
                        .send(ErrorMsg::new(kind::CHAR_DELETE, ErrorCode::WRONG_LOGIN).body());
                }
            }
            AccountRequest::CharSelect { name } => {
                let _ = control.send(AccountEvent::CharSelected(character(&name, 1)).body());
            }
        }
    }
}

#[test]
fn account_requests_get_their_answers_and_errors() {
    let srv = Server::start(serve_accounts);
    let mut s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();

    let logged_in = s.register("Mogger", "open sesame").unwrap();
    assert_eq!(logged_in.account, "Mogger");
    assert_eq!(logged_in.max_characters, 8);

    let wrong = s.login("Mogger", "wrong password").unwrap_err();
    match &wrong {
        OnlineError::Server {
            request_kind,
            code,
            wait_s,
            text,
        } => {
            assert_eq!(*request_kind, kind::LOGIN);
            assert_eq!(*code, ErrorCode::WRONG_LOGIN);
            assert_eq!(*wait_s, 0);
            assert_eq!(text, ErrorCode::WRONG_LOGIN.text());
        }
        other => panic!("expected the server's error, got {other:?}"),
    }
    assert_eq!(wrong.message(), ErrorCode::WRONG_LOGIN.text());

    let chars = s.characters().unwrap();
    assert_eq!(chars.len(), 2);
    assert_eq!(chars[0].name, "Mogger");
    assert_eq!(chars[1].name, "Sorc");
    assert_eq!(chars[0].level, 42);
    assert_eq!(chars[0].difficulties, 0b001);
    // The two bodies that arrived before the answer are still there.
    let first = s.poll().unwrap().unwrap();
    assert_eq!(first[0], 0x44);
    let second = s.poll().unwrap().unwrap();
    assert_eq!(second[0], msg::kind::ERROR);
    assert_eq!(ErrorMsg::parse(&second).unwrap().request_kind, kind::LOGIN);

    let refused = s.create_character("Druid", 5, false, false).unwrap_err();
    match refused {
        OnlineError::Server {
            request_kind,
            code,
            wait_s,
            ..
        } => {
            assert_eq!(request_kind, kind::CHAR_CREATE);
            assert_eq!(code, ErrorCode::EXPANSION_CLASS);
            assert_eq!(wait_s, 0);
        }
        other => panic!("expected ExpansionClass, got {other:?}"),
    }

    let made = s.create_character("Mogger", 0, true, false).unwrap();
    assert_eq!(made.name, "Mogger");
    assert_eq!(made.class, 0);

    let picked = s.select_character("Mogger").unwrap();
    assert_eq!(picked.name, "Mogger");
    assert_eq!(picked.difficulties, 0b001);

    s.delete_character("Mogger", "open sesame").unwrap();

    s.logout();
}

/// Waits (on this thread) for the next account event, so the test reads
/// like a bot's loop without a blocking call.
fn next_event(s: &mut Session) -> SessionEvent {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(ev) = s.poll_event() {
            return ev;
        }
        assert!(Instant::now() < until, "no event");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_non_blocking_pair_returns_at_once_and_decodes() {
    let srv = Server::start(serve_accounts);
    let mut s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();

    let t = Instant::now();
    s.send_request(AccountRequest::Register {
        account: "Mogger".into(),
        password: "open sesame".into(),
    })
    .unwrap();
    assert!(
        t.elapsed() < Duration::from_millis(100),
        "send returns at once"
    );
    assert_eq!(
        next_event(&mut s),
        SessionEvent::LoggedIn {
            account: "Mogger".into(),
            max_characters: 8,
        }
    );

    s.send_request(AccountRequest::Login {
        account: "Mogger".into(),
        password: "wrong password".into(),
    })
    .unwrap();
    match next_event(&mut s) {
        SessionEvent::Error {
            request_kind,
            code,
            wait_s,
            text,
        } => {
            assert_eq!(request_kind, kind::LOGIN);
            assert_eq!(code, ErrorCode::WRONG_LOGIN);
            assert_eq!(wait_s, 0);
            assert_eq!(text, ErrorCode::WRONG_LOGIN.text());
        }
        other => panic!("expected the server's error, got {other:?}"),
    }

    s.send_request(AccountRequest::CharList).unwrap();
    // A lobby body (0x44) is not an account event: it stays for poll(),
    // and the account answers after it are decoded in order.
    let until = Instant::now() + Duration::from_secs(5);
    let lobby = loop {
        assert!(s.poll_event().is_none(), "0x44 is not an account event");
        if let Some(b) = s.poll().unwrap() {
            break b;
        }
        assert!(Instant::now() < until, "no lobby body");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(lobby[0], 0x44);
    match next_event(&mut s) {
        SessionEvent::Error { request_kind, .. } => assert_eq!(request_kind, kind::LOGIN),
        other => panic!("expected the queued error, got {other:?}"),
    }
    match next_event(&mut s) {
        SessionEvent::Chars(chars) => assert_eq!(chars.len(), 2),
        other => panic!("expected the character list, got {other:?}"),
    }

    s.logout();
}

#[test]
fn a_rate_limited_register_carries_the_servers_wait() {
    let srv = Server::start(|_conn, mut control| async move {
        if !handshake(&mut control).await {
            return;
        }
        while let Ok(b) = control.recv_async().await {
            if b[0] == kind::REGISTER {
                let mut e = ErrorMsg::new(kind::REGISTER, ErrorCode::RATE_LIMITED);
                e.wait_s = 30;
                let _ = control.send(e.body());
            }
        }
    });
    let mut s = Session::connect(&srv.spec(), &me(), Timing::default()).unwrap();
    let e = s.register("Mogger", "open sesame").unwrap_err();
    match e {
        OnlineError::Server {
            request_kind,
            code,
            wait_s,
            ..
        } => {
            assert_eq!(request_kind, kind::REGISTER);
            assert_eq!(code, ErrorCode::RATE_LIMITED);
            assert_eq!(wait_s, 30);
            assert!(
                e.message().ends_with("Try again in 30 seconds."),
                "{}",
                e.message()
            );
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
    // The session is still open after a refusal.
    s.send(d2_net::msg::Writer::new(kind::LOGOUT).finish())
        .unwrap();
    s.close();
}
