//! The lobby on the running server over real QUIC on loopback: the game
//! list, a password game created and joined (blank, wrong and right
//! passwords), the difficulty rule on create, chat, and the join tickets
//! seating both players in one game where they see each other.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use d2_net::msg::account::{AccountEvent, AccountRequest};
use d2_net::msg::lobby::{LobbyEvent, LobbyRequest};
use d2_net::msg::{ErrorCode, ErrorMsg};
use d2_net::wire::{self, Attach, ClientHello, ServerInfo, Timing};
use d2_net::{Lane, Packet, Transport};
use d2_quic::{
    fingerprint, FramedStream, Identity, PacketStream, QuicClient, QuicServer, ServerTrust,
};
use d2_server::game::{create_request_for, new_character, GameData};
use d2rust_server::accounts::Accounts;
use d2rust_server::config::ServerConfig;
use d2rust_server::games::{Games, GamesConfig};
use d2rust_server::lobby::{self, Lobby};
use d2rust_server::log::ServerLog;
use d2rust_server::net::{Net, NetConfig};
use d2rust_server::session::Sessions;

mod support;

const BUILD: &str = "9.9.9.9";
const WAIT: Duration = Duration::from_secs(20);

struct Server {
    _rt: tokio::runtime::Runtime,
    addr: SocketAddr,
    cert: [u8; 32],
    games: Games,
    _quic: Arc<QuicServer>,
    dir: PathBuf,
}

impl Server {
    fn start(name: &str) -> Server {
        let dir = support::temp_dir(name);
        let game = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
        let cfg = ServerConfig::parse(
            &format!("game_data = {game}\nargon2 = test\nlisten = 127.0.0.1:0\ngrace_s = 2\n"),
            &dir,
        )
        .unwrap();
        let log = Arc::new(ServerLog::open(&cfg.logs).unwrap());
        let data = GameData::select(Some(Path::new(&game))).unwrap();
        let accounts = Accounts::open(&cfg).unwrap();
        let games = Games::new(GamesConfig::from_server(&cfg), data, log.clone());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let sessions = Sessions::default();
        let lobby = Arc::new(Lobby::new(
            lobby::limits(&cfg),
            games.clone(),
            sessions.clone(),
            accounts.clone(),
        ));
        let net = Arc::new(Net::new(
            NetConfig {
                server: ServerInfo {
                    protocol: d2_proto::PROTOCOL_VERSION,
                    game: "d2rust".into(),
                    build: BUILD.into(),
                    tick_rate: 60,
                },
                server_name: "test".into(),
                max_players: 8,
                max_sessions: 16,
                grace: cfg.grace,
            },
            accounts,
            lobby,
            games.clone(),
            sessions,
            log,
        ));
        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        games.set_events(tx);
        let identity = Identity::self_signed().unwrap();
        let cert = fingerprint(&identity.cert);
        let quic = rt.block_on(async {
            QuicServer::bind(
                "127.0.0.1:0".parse().unwrap(),
                identity,
                Timing::default(),
                tokio::runtime::Handle::current(),
            )
            .unwrap()
        });
        let quic = Arc::new(quic);
        let addr = quic.local_addr().unwrap();
        rt.spawn(net.clone().run(quic.clone()));
        rt.spawn(net.run_events(rx));
        Server {
            _rt: rt,
            addr,
            cert,
            games,
            _quic: quic,
            dir,
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("logs").join(d2rust_server::log::FILE_NAME))
            .unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("server log:\n{}", self.log());
        }
        self.games.shutdown_all(Duration::from_secs(10));
    }
}

struct Client {
    quic: QuicClient,
    control: FramedStream,
}

/// What came back on the control stream.
#[derive(Debug)]
enum In {
    Account(AccountEvent),
    Lobby(LobbyEvent),
    Error(ErrorMsg),
}

impl Client {
    fn connect(s: &Server) -> Client {
        let (quic, mut control) =
            QuicClient::connect(s.addr, ServerTrust::Pinned(s.cert), Timing::default()).unwrap();
        control
            .send(
                ClientHello {
                    protocol: d2_proto::PROTOCOL_VERSION,
                    game: "d2rust".into(),
                    build: BUILD.into(),
                    tick_rate: 60,
                    resume: None,
                }
                .body(),
            )
            .unwrap();
        let answer = control.recv_timeout(WAIT).unwrap().expect("a welcome");
        wire::parse_session_answer(&answer).unwrap();
        Client { quic, control }
    }

    fn next(&mut self) -> In {
        let body = self.control.recv_timeout(WAIT).unwrap().expect("a message");
        if let Ok(e) = ErrorMsg::parse(&body) {
            return In::Error(e);
        }
        if let Ok(l) = LobbyEvent::parse(&body) {
            return In::Lobby(l);
        }
        In::Account(AccountEvent::parse(&body).unwrap())
    }

    fn account(&mut self, req: AccountRequest) -> AccountEvent {
        self.control.send(req.body()).unwrap();
        match self.next() {
            In::Account(ev) => ev,
            other => panic!("{other:?}"),
        }
    }

    fn sign_up(&mut self, account: &str, character: &str, class: u8) {
        let ev = self.account(AccountRequest::Register {
            account: account.into(),
            password: "a fine password".into(),
        });
        assert!(matches!(ev, AccountEvent::LoggedIn { .. }), "{ev:?}");
        let ev = self.account(AccountRequest::CharCreate {
            name: character.into(),
            class,
            hardcore: false,
            expansion: true,
        });
        assert!(matches!(ev, AccountEvent::CharCreated(_)), "{ev:?}");
        let ev = self.account(AccountRequest::CharSelect {
            name: character.into(),
        });
        assert!(matches!(ev, AccountEvent::CharSelected(_)), "{ev:?}");
    }

    fn send(&mut self, r: LobbyRequest) {
        self.control.send(r.body()).unwrap();
    }

    /// Messages until one matches `want` (others are dropped).
    fn until(&mut self, want: impl Fn(&In) -> bool) -> In {
        let until = Instant::now() + WAIT;
        while Instant::now() < until {
            let m = self.next();
            if want(&m) {
                return m;
            }
        }
        panic!("nothing matched");
    }

    fn error(&mut self) -> ErrorMsg {
        match self.until(|m| matches!(m, In::Error(_))) {
            In::Error(e) => e,
            _ => unreachable!(),
        }
    }

    fn join_ok(&mut self) -> ([u8; 16], u32) {
        match self.until(|m| matches!(m, In::Lobby(LobbyEvent::JoinOk { .. }) | In::Error(_))) {
            In::Lobby(LobbyEvent::JoinOk {
                ticket, game_id, ..
            }) => (ticket, game_id),
            other => panic!("{other:?}"),
        }
    }

    /// The list snapshot after `ListGames`.
    fn list(&mut self) -> Vec<d2_net::msg::lobby::GameInfo> {
        self.send(LobbyRequest::ListGames);
        let mut games = Vec::new();
        let _ = self.until(|m| matches!(m, In::Lobby(LobbyEvent::GameListBegin { .. })));
        loop {
            match self.next() {
                In::Lobby(LobbyEvent::GameEntry(g)) => games.push(g),
                In::Lobby(LobbyEvent::GameListEnd) => return games,
                other => panic!("{other:?}"),
            }
        }
    }

    fn attach(&mut self, ticket: [u8; 16]) -> Result<PacketStream, wire::Refused> {
        let mut stream = self.quic.open_stream().unwrap();
        stream.send(Attach::Ticket(ticket).body()).unwrap();
        let answer = stream
            .recv_timeout(WAIT)
            .unwrap()
            .expect("an attach answer");
        wire::parse_attach_answer(&answer)?;
        Ok(PacketStream::client(stream))
    }
}

/// Plays the start of a game as the game client does and collects what
/// arrives until `done` says so.
fn play(
    game: &mut PacketStream,
    class: &str,
    name: &str,
    done: impl Fn(&[Vec<u8>]) -> bool,
) -> Vec<Vec<u8>> {
    let req = create_request_for(&new_character(class, name).unwrap(), 800, 600);
    game.send(Packet::Send {
        lane: Lane::System,
        msg: req.encode().to_vec(),
    })
    .unwrap();
    let mut got = Vec::new();
    let mut joined = false;
    let until = Instant::now() + WAIT;
    while Instant::now() < until && !done(&got) {
        match game.recv().unwrap() {
            Some(Packet::Deliver(m)) => {
                if m.first() == Some(&0x02) && !joined {
                    joined = true;
                    game.send(Packet::Send {
                        lane: Lane::System,
                        msg: vec![0x6B],
                    })
                    .unwrap();
                }
                got.push(m);
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    got
}

fn sees(got: &[Vec<u8>], name: &str) -> bool {
    got.iter()
        .any(|m| m.first() == Some(&0x5B) && m.len() >= 24 && m[8..24].starts_with(name.as_bytes()))
}

/// Rules (contracts 3.6, 6; owner decisions 3, 4): a password game is
/// listed locked; blank and wrong passwords are refused, the right one
/// joins; both tickets seat their players in the one game.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_password_game_is_created_listed_and_joined() {
    let s = Server::start("lobby-password");
    let mut a = Client::connect(&s);
    let mut b = Client::connect(&s);
    a.sign_up("Alpha", "Amazona", 0);
    b.sign_up("Bravo", "Sorcerina", 1);
    assert!(b.list().is_empty());
    a.send(LobbyRequest::CreateGame {
        name: "cows".into(),
        password: "moo".into(),
        difficulty: 0,
        max_players: 0,
    });
    let (ta, id) = a.join_ok();
    // B listens: the new game arrives as an update, locked, joinable.
    let entry = match b.until(|m| matches!(m, In::Lobby(LobbyEvent::GameEntry(_)))) {
        In::Lobby(LobbyEvent::GameEntry(g)) => g,
        _ => unreachable!(),
    };
    assert_eq!((entry.game_id, entry.name.as_str()), (id, "cows"));
    assert!(entry.locked);
    assert_eq!(entry.joinable, 0);
    for (pw, code) in [
        ("", ErrorCode::PASSWORD_NEEDED),
        ("oink", ErrorCode::WRONG_PASSWORD),
    ] {
        b.send(LobbyRequest::JoinGame {
            game_id: id,
            password: pw.into(),
        });
        let e = b.error();
        assert_eq!((e.request_kind, e.code), (0x46, code), "password {pw:?}");
    }
    b.send(LobbyRequest::JoinGame {
        game_id: id,
        password: "moo".into(),
    });
    let (tb, id_b) = b.join_ok();
    assert_eq!(id_b, id);
    // A ticket is the session's own: B can't use A's.
    assert_eq!(
        b.attach(ta).err().map(|r| r.code),
        Some(ErrorCode::TICKET_INVALID)
    );
    let mut ga = a.attach(ta).expect("A's ticket seats A");
    let mut gb = b.attach(tb).expect("B's ticket seats B");
    let (ja, jb) = (
        std::thread::spawn(move || {
            let got = play(&mut ga, "amazon", "Amazona", |g| sees(g, "Sorcerina"));
            (got, ga)
        }),
        std::thread::spawn(move || {
            let got = play(&mut gb, "sorceress", "Sorcerina", |g| sees(g, "Amazona"));
            (got, gb)
        }),
    );
    let (got_a, _ga) = ja.join().unwrap();
    let (got_b, _gb) = jb.join().unwrap();
    assert!(sees(&got_a, "Sorcerina"));
    assert!(sees(&got_b, "Amazona"));
    assert_eq!(s.games.get(id).expect("the game runs").players(), 2);
}

/// Rule (owner decision 4, contracts 6): a fresh character creates only at
/// Normal; a taken name (any case) and a bad name are refused.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn create_follows_the_difficulty_and_name_rules() {
    let s = Server::start("lobby-create");
    let mut a = Client::connect(&s);
    a.sign_up("Alpha", "Amazona", 0);
    for (name, d, code) in [
        ("nm", 1, ErrorCode::DIFFICULTY_LOCKED),
        ("hell", 2, ErrorCode::DIFFICULTY_LOCKED),
        ("bad!", 0, ErrorCode::BAD_GAME_NAME),
    ] {
        a.send(LobbyRequest::CreateGame {
            name: name.into(),
            password: String::new(),
            difficulty: d,
            max_players: 0,
        });
        let e = a.error();
        assert_eq!((e.request_kind, e.code), (0x45, code), "{name}");
    }
    a.send(LobbyRequest::CreateGame {
        name: "Normal Run".into(),
        password: String::new(),
        difficulty: 0,
        max_players: 0,
    });
    let _ = a.join_ok();
    let mut b = Client::connect(&s);
    b.sign_up("Bravo", "Sorcerina", 1);
    let list = b.list();
    assert_eq!(list.len(), 1);
    assert!(!list[0].locked);
    b.send(LobbyRequest::CreateGame {
        name: "normal run".into(),
        password: String::new(),
        difficulty: 0,
        max_players: 0,
    });
    assert_eq!(b.error().code, ErrorCode::GAME_NAME_TAKEN);
}

/// Rule (contracts 3.6): chat goes to every session in the lobby, named
/// by the sender's character.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn chat_reaches_the_lobby() {
    let s = Server::start("lobby-chat");
    let mut a = Client::connect(&s);
    let mut b = Client::connect(&s);
    a.sign_up("Alpha", "Amazona", 0);
    b.sign_up("Bravo", "Sorcerina", 1);
    a.list();
    b.list();
    a.send(LobbyRequest::Chat {
        text: "anyone for cows?".into(),
    });
    for c in [&mut a, &mut b] {
        match c.until(|m| matches!(m, In::Lobby(LobbyEvent::ChatLine { .. }))) {
            In::Lobby(LobbyEvent::ChatLine { from, text, .. }) => {
                assert_eq!(
                    (from.as_str(), text.as_str()),
                    ("Amazona", "anyone for cows?")
                );
            }
            _ => unreachable!(),
        }
    }
}
