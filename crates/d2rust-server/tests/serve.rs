//! The running server over real QUIC on loopback: the handshake and its
//! refusals, accounts and characters, the lobby seam, and two players
//! attaching their game streams to one game and seeing each other.
//!
//! The lobby here is a stand-in ([`TicketLobby`]): any ticket seats the
//! session's selected character in game 1. The real lobby's rules are its
//! own tests.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use d2_net::msg::account::{AccountEvent, AccountRequest};
use d2_net::msg::{ErrorCode, ErrorMsg};
use d2_net::wire::{self, Attach, ClientHello, ServerInfo, Timing};
use d2_net::{Lane, Packet, Transport};
use d2_quic::{
    fingerprint, FramedStream, Identity, PacketStream, QuicClient, QuicServer, ServerTrust,
};
use d2_server::game::{create_request_for, new_character, GameData};
use d2rust_server::accounts::{Accounts, NewChar};
use d2rust_server::config::ServerConfig;
use d2rust_server::denied::Denied;
use d2rust_server::games::{GameEvent, GameHandle, GameSpec, Games, GamesConfig, Seat};
use d2rust_server::log::ServerLog;
use d2rust_server::net::{Net, NetConfig};
use d2rust_server::session::{LobbyApi, SessionCtx, Sessions};

mod support;

const BUILD: &str = "9.9.9.9";
const WAIT: Duration = Duration::from_secs(20);

/// Any ticket seats the session's selected character in game 1.
struct TicketLobby {
    games: Games,
    accounts: Accounts,
    game: Mutex<Option<GameHandle>>,
}

impl LobbyApi for TicketLobby {
    async fn handle(&self, _s: &mut SessionCtx, _body: &[u8]) -> Result<(), Denied> {
        Err(ErrorCode::NOT_SUPPORTED.into())
    }

    fn redeem(&self, s: &SessionCtx, _ticket: [u8; 16]) -> Result<(GameHandle, Seat), Denied> {
        let (account, _) = s
            .account
            .clone()
            .ok_or_else(|| Denied::from(ErrorCode::NOT_LOGGED_IN))?;
        let c = s
            .character
            .clone()
            .ok_or_else(|| Denied::from(ErrorCode::NO_CHAR_SELECTED))?;
        let mut game = self.game.lock().unwrap();
        if game.as_ref().is_none_or(|g| g.is_closed()) {
            let spec = GameSpec {
                id: 1,
                name: "together".into(),
                difficulty: 0,
                hardcore: false,
                expansion: true,
                max_players: 0,
            };
            *game = Some(
                self.games
                    .start(spec, self.games.events().unwrap())
                    .map_err(Denied::from)?,
            );
        }
        let file = self.accounts.character_file(account, &c.name)?;
        let new = (!file.exists()).then(|| NewChar {
            name: c.name.clone(),
            class: c.class,
            hardcore: false,
            expansion: true,
            progression: 0,
        });
        let seat = Seat {
            account,
            character: c.name,
            file,
            new,
        };
        Ok((game.clone().unwrap(), seat))
    }

    fn on_game_event(&self, _ev: GameEvent) {}

    fn session_ended(&self, _s: &SessionCtx) {}

    fn character_in_game(&self, _name_key: &str) -> bool {
        false
    }
}

/// A server on loopback.
struct Server {
    /// Runs the network side; dropped last.
    _rt: tokio::runtime::Runtime,
    addr: SocketAddr,
    cert: [u8; 32],
    games: Games,
    accounts: Accounts,
    _quic: Arc<QuicServer>,
    dir: PathBuf,
}

impl Server {
    fn start(name: &str) -> Server {
        Self::start_with(name, false)
    }

    /// A server on loopback; `dev_test` writes `dev = test` in its config.
    fn start_with(name: &str, dev_test: bool) -> Server {
        let dir = support::temp_dir(name);
        let game = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
        let dev = if dev_test { "dev = test\n" } else { "" };
        let cfg = ServerConfig::parse(
            &format!("game_data = {game}\nargon2 = test\nlisten = 127.0.0.1:0\ngrace_s = 2\n{dev}"),
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
        let lobby = Arc::new(TicketLobby {
            games: games.clone(),
            accounts: accounts.clone(),
            game: Mutex::new(None),
        });
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
            accounts.clone(),
            lobby,
            games.clone(),
            Sessions::default(),
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
            accounts,
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

fn hello(build: &str) -> ClientHello {
    ClientHello {
        protocol: d2_proto::PROTOCOL_VERSION,
        game: "d2rust".into(),
        build: build.into(),
        tick_rate: 60,
        resume: None,
    }
}

/// A client's session: the connection and its control stream.
struct Client {
    quic: QuicClient,
    control: FramedStream,
}

impl Client {
    fn connect(s: &Server, build: &str) -> Result<Client, wire::Refused> {
        let (quic, mut control) =
            QuicClient::connect(s.addr, ServerTrust::Pinned(s.cert), Timing::default()).unwrap();
        control.send(hello(build).body()).unwrap();
        let answer = control
            .recv_timeout(WAIT)
            .unwrap()
            .expect("an answer to the hello");
        wire::parse_session_answer(&answer)?;
        Ok(Client { quic, control })
    }

    /// Sends a request and reads the answer (an event, or an error).
    fn ask(&mut self, req: AccountRequest) -> Result<AccountEvent, ErrorMsg> {
        self.control.send(req.body()).unwrap();
        let body = self.control.recv_timeout(WAIT).unwrap().expect("an answer");
        match ErrorMsg::parse(&body) {
            Ok(e) => Err(e),
            Err(_) => Ok(AccountEvent::parse(&body).unwrap()),
        }
    }

    fn sign_up(&mut self, account: &str, character: &str, class: u8) {
        let ev = self.ask(AccountRequest::Register {
            account: account.into(),
            password: "a fine password".into(),
        });
        assert!(matches!(ev, Ok(AccountEvent::LoggedIn { .. })), "{ev:?}");
        let ev = self.ask(AccountRequest::CharCreate {
            name: character.into(),
            class,
            hardcore: false,
            expansion: true,
        });
        assert!(matches!(ev, Ok(AccountEvent::CharCreated(_))), "{ev:?}");
        let ev = self.ask(AccountRequest::CharSelect {
            name: character.into(),
        });
        assert!(matches!(ev, Ok(AccountEvent::CharSelected(_))), "{ev:?}");
    }

    /// Opens the game stream with a ticket; the game's welcome comes back.
    fn attach(&mut self) -> PacketStream {
        let mut stream = self.quic.open_stream().unwrap();
        stream.send(Attach::Ticket([7; 16]).body()).unwrap();
        let answer = stream
            .recv_timeout(WAIT)
            .unwrap()
            .expect("an attach answer");
        wire::parse_attach_answer(&answer).expect("the game's welcome");
        PacketStream::client(stream)
    }
}

/// Plays the start of a game as the game client does (0x67, then 0x6B
/// after the server's 0x02) and collects what arrives until `done` says so.
fn play(game: &mut PacketStream, name: &str, done: impl Fn(&[Vec<u8>]) -> bool) -> Vec<Vec<u8>> {
    let req = create_request_for(&new_character("barbarian", name).unwrap(), 800, 600);
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

/// S→C 0x5B (a player joined) naming `name`.
fn sees(got: &[Vec<u8>], name: &str) -> bool {
    got.iter()
        .any(|m| m.first() == Some(&0x5B) && m.len() >= 24 && m[8..24].starts_with(name.as_bytes()))
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_client_of_another_version_is_refused_with_words() {
    let s = Server::start("serve-refuse");
    let r = Client::connect(&s, "0.0.0.1").err().expect("refused");
    assert_eq!(r.code, ErrorCode::BUILD);
    assert_eq!(
        r.text,
        format!("The server runs version {BUILD}; you have 0.0.0.1. Update through the launcher.")
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn requests_need_a_login_and_a_character_and_a_new_login_takes_over() {
    let s = Server::start("serve-session");
    let mut a = Client::connect(&s, BUILD).unwrap();
    let e = a.ask(AccountRequest::CharList).unwrap_err();
    assert_eq!(e.code, ErrorCode::NOT_LOGGED_IN);
    a.sign_up("Owner", "Kashya", 0);
    // The lobby's kinds go to the lobby once a character is selected.
    a.control.send(vec![0x40]).unwrap();
    let e = ErrorMsg::parse(&a.control.recv_timeout(WAIT).unwrap().unwrap()).unwrap();
    assert_eq!((e.request_kind, e.code), (0x40, ErrorCode::NOT_SUPPORTED));
    // The same account logs in elsewhere: the first session is taken over.
    let mut b = Client::connect(&s, BUILD).unwrap();
    let ev = b.ask(AccountRequest::Login {
        account: "owner".into(),
        password: "a fine password".into(),
    });
    assert!(matches!(ev, Ok(AccountEvent::LoggedIn { .. })), "{ev:?}");
    let e = ErrorMsg::parse(&a.control.recv_timeout(WAIT).unwrap().unwrap()).unwrap();
    assert_eq!(e.code, ErrorCode::SESSION_TAKEN);
    assert!(
        a.control.recv_timeout(WAIT).is_err(),
        "the old session is closed"
    );
    let ev = b.ask(AccountRequest::CharList);
    assert!(
        matches!(ev, Ok(AccountEvent::CharList(ref l)) if l.len() == 1),
        "{ev:?}"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn two_logins_at_once_do_not_deadlock() {
    let s = Server::start("serve-login-race");
    let mut owner = Client::connect(&s, BUILD).unwrap();
    let ev = owner
        .ask(AccountRequest::Register {
            account: "Racer".into(),
            password: "a fine password".into(),
        })
        .unwrap();
    assert!(matches!(ev, AccountEvent::LoggedIn { .. }), "{ev:?}");
    // The account is made; both sessions start logged out.
    owner.control.send(AccountRequest::Logout.body()).unwrap();
    let _ = owner.control.recv_timeout(WAIT);
    drop(owner);
    let mut a = Client::connect(&s, BUILD).unwrap();
    let mut b = Client::connect(&s, BUILD).unwrap();
    let login = AccountRequest::Login {
        account: "Racer".into(),
        password: "a fine password".into(),
    };
    // Both requests are in flight before either session reads its answer:
    // each login locks the other session to take it over. Holding its own
    // session lock across that locked the two tasks against each other and
    // neither ever answered.
    a.control.send(login.body()).unwrap();
    b.control.send(login.body()).unwrap();
    let ra = a
        .control
        .recv_timeout(WAIT)
        .expect("session a answered or closed: the logins did not deadlock");
    let rb = b
        .control
        .recv_timeout(WAIT)
        .expect("session b answered or closed: the logins did not deadlock");
    // Whatever a session got, it is a login or the takeover; the deadlock
    // produced neither (both reads timed out).
    let mut logged_in = 0;
    for body in [ra.as_ref(), rb.as_ref()].into_iter().flatten() {
        if matches!(AccountEvent::parse(body), Ok(AccountEvent::LoggedIn { .. })) {
            logged_in += 1;
        } else {
            let e = ErrorMsg::parse(body).expect("an event or an error");
            assert_eq!(e.code, ErrorCode::SESSION_TAKEN, "{e:?}");
        }
    }
    assert!(logged_in >= 1, "a login was accepted");
    // Only one session keeps the account: the other was taken over.
    let mut holder = 0;
    for c in [&mut a, &mut b] {
        let _ = c.control.send(AccountRequest::CharList.body());
        if matches!(
            c.control.recv_timeout(WAIT),
            Ok(Some(body)) if matches!(AccountEvent::parse(&body), Ok(AccountEvent::CharList(_)))
        ) {
            holder += 1;
        }
    }
    assert_eq!(holder, 1, "exactly one session holds the account");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn two_players_attach_to_one_game_and_see_each_other() {
    let s = Server::start("serve-together");
    let mut a = Client::connect(&s, BUILD).unwrap();
    let mut b = Client::connect(&s, BUILD).unwrap();
    a.sign_up("Alpha", "Amazona", 0);
    b.sign_up("Bravo", "Sorcerina", 1);
    let mut ga = a.attach();
    let mut gb = b.attach();
    let (ta, tb) = (
        std::thread::spawn(move || {
            let got = play(&mut ga, "Amazona", |g| sees(g, "Sorcerina"));
            (got, ga)
        }),
        std::thread::spawn(move || {
            let got = play(&mut gb, "Sorcerina", |g| sees(g, "Amazona"));
            (got, gb)
        }),
    );
    let (got_a, _ga) = ta.join().unwrap();
    let (got_b, _gb) = tb.join().unwrap();
    assert!(sees(&got_a, "Sorcerina"), "Alpha sees Bravo's player");
    assert!(sees(&got_b, "Amazona"), "Bravo sees Alpha's player");
    let game = s.games.get(1).expect("game 1 runs");
    assert_eq!(game.players(), 2);
}

/// Joins `name` in `game` (0x67, then 0x6B after the game's 0x02) and
/// returns once the game holds its player: `heal` answers then.
fn seat(s: &Server, game: &mut PacketStream, class: &str, name: &str) {
    let req = create_request_for(&new_character(class, name).unwrap(), 800, 600);
    game.send(Packet::Send {
        lane: Lane::System,
        msg: req.encode().to_vec(),
    })
    .unwrap();
    let heal = d2_proto::dev::parse("heal").unwrap().unwrap();
    let until = Instant::now() + WAIT;
    let mut joined = false;
    loop {
        while let Some(p) = game.recv().unwrap() {
            if let Packet::Deliver(m) = p {
                if m.first() == Some(&0x02) && !joined {
                    joined = true;
                    game.send(Packet::Send {
                        lane: Lane::System,
                        msg: vec![0x6B],
                    })
                    .unwrap();
                }
            }
        }
        if s.games.dev_command(name, &heal).is_ok() {
            break;
        }
        assert!(
            Instant::now() < until,
            "the character never joined the game"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dev_commands_run_for_a_seated_character_and_name_unknown_ones() {
    let s = Server::start_with("serve-dev", true);
    let mut a = Client::connect(&s, BUILD).unwrap();
    a.sign_up("Developer", "Devvy", 1);
    let mut game = a.attach();
    seat(&s, &mut game, "sorceress", "Devvy");
    let gold = d2_proto::dev::parse("gold 250").unwrap().unwrap();
    let text = s
        .games
        .dev_command("devvy", &gold)
        .expect("the gold command runs");
    assert!(text.contains("gold ") && text.contains("->"), "{text}");
    let e = s
        .games
        .dev_command("Nobody", &gold)
        .expect_err("no such character");
    assert!(e.contains("Nobody"), "{e}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dev_commands_are_refused_when_the_server_is_not_dev_test() {
    let s = Server::start("serve-nodev");
    let cmd = d2_proto::dev::parse("gold 5").unwrap().unwrap();
    let e = s.games.dev_command("Anyone", &cmd).unwrap_err();
    assert!(e.contains("disabled"), "{e}");
}

/// `set-progression` on stdin (contracts 10.1): the accounts row and its
/// file, while the character is out of a game; `dev CHARACTER
/// set-progression N` is the same line; an unknown character is named;
/// a seated one is refused.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dev_set_progression_needs_the_character_out_of_a_game() {
    use d2rust_server::devcmd;
    let s = Server::start_with("serve-dev-progression", true);
    let mut a = Client::connect(&s, BUILD).unwrap();
    a.sign_up("Tester", "Progressy", 0);
    let answer = |line: &str| {
        devcmd::answer(line, true, |req| {
            devcmd::run_command(&s.games, &s.accounts, req)
        })
        .expect("an answer")
    };
    assert_eq!(
        answer("set-progression Progressy 5"),
        "dev ok Progressy progression set to 5"
    );
    assert_eq!(
        answer("dev Progressy set-progression 6"),
        "dev ok Progressy progression set to 6"
    );
    let ev = a.ask(AccountRequest::CharList).expect("a character list");
    let AccountEvent::CharList(chars) = ev else {
        panic!("{ev:?}")
    };
    let c = chars
        .iter()
        .find(|c| c.name == "Progressy")
        .expect("Progressy");
    assert_eq!(c.difficulties, 3, "progression 6 opens Nightmare and Hell");
    assert_eq!(
        answer("set-progression Nobody 5"),
        "dev err no character named `Nobody`"
    );
    // In a game the running server refuses: the character's save owns the
    // record while it plays.
    let mut game = a.attach();
    seat(&s, &mut game, "sorceress", "Progressy");
    assert_eq!(
        answer("set-progression Progressy 7"),
        "dev err Progressy is in a game; the progression is set while it is out"
    );
}
