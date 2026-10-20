//! A bot: one headless client. It connects over QUIC through its own
//! relay ([`crate::net::Relay`]), sends what a player's client sends, and
//! keeps a small view of the game from what the server delivers (the
//! players it was told about and where they stand). It never draws and
//! never runs the game's rules: what it knows, it knows from the server.
//!
//! It speaks wire version 4 (contracts 3.1): the session's handshake on
//! the control stream ([`ClientHello`], [`SessionWelcome`], [`Refused`]),
//! then the account and lobby messages on that stream and the game stream
//! a game is entered on, which starts with an [`Attach`] (the lobby's
//! ticket, or the game's resume token) and carries the game's packets
//! after the server's welcome.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use d2_net::msg::account::{AccountEvent, AccountRequest, CharSummary};
use d2_net::msg::lobby::{self, GameInfo, LobbyEvent, LobbyRequest};
use d2_net::msg::{kind as msg_kind, ErrorCode, ErrorMsg};
use d2_net::wire::{
    parse_attach_answer, parse_session_answer, Attach, ClientHello, Refused, SessionWelcome,
    Timing, Welcome,
};
use d2_net::{ClientId, Lane, NetError, Packet, ResumeToken, Transport};
use d2_proto::s2c::{self, Message, ParseError};
use d2_quic::{fingerprint, FramedStream, PacketStream, QuicClient, ServerTrust};

use crate::net::Relay;
use crate::play::Play;

/// Keepalive and timeout of every bot connection: short, so a cut link
/// is noticed within a scenario's wait.
pub const BOT_TIMING: Timing = Timing {
    keepalive: Duration::from_millis(500),
    timeout: Duration::from_secs(3),
};

/// How long a game stream may take to answer its attach: the game loads
/// the character (and its save) before it welcomes the player.
const ATTACH_WAIT: Duration = Duration::from_secs(10);

/// How often a driver reads a stream while it waits.
const POLL: Duration = Duration::from_millis(2);

/// How a step went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// It worked.
    Ok,
    /// The server said no: a stable code and its words.
    Refused {
        /// The contracts' `ErrorCode` name (`Build`, `WrongLogin`, ...),
        /// or `no-player` when a game gave no player and said nothing.
        code: String,
        /// What the server said.
        text: String,
    },
    /// It failed for another reason.
    Error(String),
    /// The bot's connection is closed.
    Closed(String),
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Ok => write!(f, "ok"),
            Status::Refused { code, text } => write!(f, "refused={code} ({text})"),
            Status::Error(e) => write!(f, "error ({e})"),
            Status::Closed(why) => write!(f, "closed ({why})"),
        }
    }
}

/// A step's result: its status and the facts it reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// How it went.
    pub status: Status,
    /// What it found (`players=2`, `client=3`, ...).
    pub facts: BTreeMap<String, String>,
}

impl Outcome {
    /// A plain success.
    pub fn ok() -> Self {
        Self {
            status: Status::Ok,
            facts: BTreeMap::new(),
        }
    }

    /// A failure that isn't a refusal.
    pub fn error(e: impl Into<String>) -> Self {
        Self {
            status: Status::Error(e.into()),
            facts: BTreeMap::new(),
        }
    }

    /// Adds a fact.
    pub fn fact(mut self, k: impl Into<String>, v: impl ToString) -> Self {
        self.facts.insert(k.into(), v.to_string());
        self
    }
}

/// A server's refusal as the step's status: its `ErrorCode` name in the
/// multiplayer contracts (section 3.8), so scenarios read the same for
/// every message.
fn refused(r: &Refused) -> Status {
    Status::Refused {
        code: r.code.name().into(),
        text: r.text.clone(),
    }
}

/// A server's answer to a request that failed.
fn error_status(e: &ErrorMsg) -> Status {
    Status::Refused {
        code: e.code.name().into(),
        text: e.text.clone(),
    }
}

/// Why a request failed.
enum Failed {
    /// The server answered `ERROR`.
    Server(ErrorMsg),
    /// The request couldn't be made or answered.
    Local(Status),
}

impl Failed {
    /// The step's outcome, with how long the server took.
    fn outcome(self, ms: u128) -> Outcome {
        match self {
            Failed::Server(e) => Outcome {
                status: error_status(&e),
                facts: BTreeMap::from([
                    ("ms".into(), ms.to_string()),
                    ("wait_s".into(), e.wait_s.to_string()),
                ]),
            },
            Failed::Local(status) => Outcome {
                status,
                facts: BTreeMap::new(),
            },
        }
    }
}

impl From<Status> for Failed {
    fn from(s: Status) -> Self {
        Failed::Local(s)
    }
}

/// The certificate a server named in its ready line, for pinning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerCert {
    /// The DER certificate itself, when the server wrote it to a file.
    pub der: Option<Vec<u8>>,
    /// Its SHA-256, when the server printed `cert_sha256=`.
    pub sha256: Option<[u8; 32]>,
}

impl ServerCert {
    /// The fingerprint a bot pins.
    pub fn pin(&self) -> Option<[u8; 32]> {
        self.sha256.or_else(|| self.der.as_deref().map(fingerprint))
    }
}

/// A player a bot was told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// The character name.
    pub name: String,
    /// The class.
    pub class: u8,
    /// Where it stands (or walks to), as last told.
    pub x: u16,
    /// Where it stands (or walks to), as last told.
    pub y: u16,
    /// Where it was first seen.
    pub first: (u16, u16),
}

/// What a bot knows about the game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The players it was told about, by unit id.
    pub players: BTreeMap<u32, Seen>,
    /// Its own player, once assigned.
    pub own: Option<u32>,
    /// Frame marks received, and how many had a tick.
    pub frames: u64,
    /// Frame marks with a tick.
    pub ticks: u64,
    /// Messages delivered.
    pub delivered: u64,
    /// Delivered messages that broke their size rule or layout.
    pub bad: Vec<String>,
    /// Delivered messages by id.
    pub ids: BTreeMap<u8, u64>,
    /// Mod extension messages received.
    pub ext: u64,
    /// The game answered the join (`0x02`): its 0x6B may go now.
    pub loaded: bool,
    /// The game ended the session's play (`0x06`), as a leave does.
    pub exiting: bool,
    /// Monsters, items, objects, act and area.
    pub play: Play,
}

impl View {
    /// Takes in one delivered message.
    fn apply(&mut self, msg: &[u8], own_name: &str) {
        self.delivered += 1;
        if let Some(&id) = msg.first() {
            *self.ids.entry(id).or_default() += 1;
        }
        let parsed = s2c::parse(msg);
        self.play.apply(msg, parsed.as_ref().ok(), self.own);
        match parsed {
            Ok(Message::AssignPlayer(a)) => {
                let name = cstr(&a.name);
                if name == own_name {
                    self.own = Some(a.guid);
                }
                self.players.insert(
                    a.guid,
                    Seen {
                        name,
                        class: a.class,
                        x: a.x,
                        y: a.y,
                        first: (a.x, a.y),
                    },
                );
            }
            Ok(Message::PlayerMove(m)) if m.type_ == 0 => {
                if let Some(p) = self.players.get_mut(&m.guid) {
                    (p.x, p.y) = (m.target_x, m.target_y);
                }
            }
            Ok(Message::ReassignPlayer(m)) if m.type_ == 0 => {
                if let Some(p) = self.players.get_mut(&m.guid) {
                    (p.x, p.y) = (m.x, m.y);
                }
            }
            Ok(Message::PlayerLeft(m)) => {
                self.players.remove(&m.guid);
            }
            Ok(Message::RemoveUnit(m)) if m.type_ == 0 => {
                self.players.remove(&m.guid);
            }
            Ok(Message::LoadSuccessful(_)) => self.loaded = true,
            Ok(Message::GameExit(_)) => self.exiting = true,
            Ok(_) | Err(ParseError::Unbuilt { .. }) => {}
            Err(e) => self.bad.push(e.to_string()),
        }
    }

    /// The player named `name`, if known.
    pub fn named(&self, name: &str) -> Option<&Seen> {
        self.players.values().find(|p| p.name == name)
    }
}

/// A NUL-padded name as text.
fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// What a bot says in its hello unless a step says otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelloDefaults {
    /// `d2-proto` protocol version.
    pub protocol: u32,
    /// The game the server runs (`d2rust`).
    pub game: String,
    /// Game build.
    pub build: String,
    /// Tick rate.
    pub tick_rate: u32,
}

/// Why opening the session's handshake failed.
enum Open {
    /// The server refused the hello.
    Refused(Refused),
    /// The connection or its answer failed.
    Error(String),
}

/// A bot's open connection: the QUIC connection, its control stream
/// (the session handshake, then account and lobby messages) and the game
/// stream once a game is entered.
#[derive(Debug)]
struct Conn {
    /// Dropped after the streams: closing it ends the connection.
    client: QuicClient,
    /// The session's control stream.
    control: FramedStream,
    /// Bodies that arrived on the control stream and no driver has read.
    inbox: VecDeque<Vec<u8>>,
    /// The game stream, once attached.
    game: Option<PacketStream>,
}

impl Conn {
    /// Connects, sends `hello` on the control stream and waits (at most
    /// the timing's timeout) for the session's answer.
    fn open(
        addr: std::net::SocketAddr,
        hello: &ClientHello,
        trust: ServerTrust,
        timing: Timing,
    ) -> Result<(Self, SessionWelcome), Open> {
        let (client, mut control) =
            QuicClient::connect(addr, trust, timing).map_err(|e| Open::Error(e.to_string()))?;
        let answer = control
            .send(hello.body())
            .and_then(|()| control.recv_timeout(timing.timeout))
            .map_err(|e| Open::Error(format!("handshake: {e}")))?;
        let Some(body) = answer else {
            return Err(Open::Error("the server did not answer in time".into()));
        };
        let welcome = parse_session_answer(&body).map_err(Open::Refused)?;
        Ok((
            Self {
                client,
                control,
                inbox: VecDeque::new(),
                game: None,
            },
            welcome,
        ))
    }

    /// Ends the connection with a goodbye on both streams.
    fn close(mut self) {
        if let Some(game) = self.game.as_mut() {
            game.close();
        }
        self.control.close();
        self.client.close();
    }
}

/// One bot client.
#[derive(Debug)]
pub struct Bot {
    /// Its index in the scenario (`b0`, `b1`, ...).
    pub index: usize,
    /// Its relay to the server.
    pub relay: Relay,
    conn: Option<Conn>,
    /// Why its last connection ended, once it has.
    pub closed: Option<String>,
    /// What it knows.
    pub view: View,
    /// It was in a game (sent 0x67) since it entered.
    pub entered: bool,
    /// The session token to come back with after a lost connection.
    session: Option<ResumeToken>,
    /// The game's resume token, while it holds a game slot.
    game_token: Option<ResumeToken>,
    /// The game's client id, as last welcomed.
    game_client: Option<ClientId>,
    /// The account it is logged in to, as stored.
    logged_in: Option<String>,
    /// The character the lobby plays with.
    selected: Option<CharSummary>,
    /// Character names this bot has seen or asked about.
    known_chars: BTreeSet<String>,
    /// The lobby's games, as last listed or updated.
    games: BTreeMap<u32, GameInfo>,
    /// Game names this bot has seen or asked about.
    known_games: BTreeSet<String>,
    /// Complete game lists received.
    lists: u64,
    /// The lobby is sending this session the game list.
    listening: bool,
    /// The game the session is in, as the lobby said.
    game_id: Option<u32>,
    /// Its difficulty, as the lobby gave it.
    game_difficulty: Option<u8>,
    /// The last chat line heard: who and what.
    last_chat: Option<(String, String)>,
    /// The last session-level error the server sent.
    session_error: Option<ErrorMsg>,
    /// The server took this session over: its close is the contract's.
    taken: bool,
    /// A close a driver caused that `pump` should report.
    close_event: Option<String>,
}

impl Bot {
    /// A bot with no connection.
    pub fn new(index: usize, relay: Relay) -> Self {
        Self {
            index,
            relay,
            conn: None,
            closed: None,
            view: View::default(),
            entered: false,
            session: None,
            game_token: None,
            game_client: None,
            logged_in: None,
            selected: None,
            known_chars: BTreeSet::new(),
            games: BTreeMap::new(),
            known_games: BTreeSet::new(),
            lists: 0,
            listening: false,
            game_id: None,
            game_difficulty: None,
            last_chat: None,
            session_error: None,
            taken: false,
            close_event: None,
        }
    }

    /// Its character name: `bot<index>`.
    pub fn char_name(&self) -> String {
        format!("bot{}", self.index)
    }

    /// The name it plays with: its selected character's, or its own.
    pub fn play_name(&self) -> String {
        self.selected
            .as_ref()
            .map_or_else(|| self.char_name(), |c| c.name.clone())
    }

    /// It has an open connection.
    pub fn connected(&self) -> bool {
        self.conn.is_some()
    }

    /// It holds a game stream.
    pub fn in_game(&self) -> bool {
        self.conn.as_ref().is_some_and(|conn| conn.game.is_some())
    }

    /// The server took its session over (a new login): its close is
    /// expected, not a broken connection.
    pub fn taken(&self) -> bool {
        self.taken
    }

    /// The account it is logged in to, if it is.
    pub fn account(&self) -> &str {
        self.logged_in.as_deref().unwrap_or("")
    }

    /// The last session-level error's code, if the server sent one.
    pub fn session_error(&self) -> &str {
        self.session_error.as_ref().map_or("", |e| e.code.name())
    }

    /// The game the lobby put it in, as text (`-` when none).
    pub fn game(&self) -> std::borrow::Cow<'_, str> {
        match self.game_id {
            Some(id) => std::borrow::Cow::Owned(id.to_string()),
            None => std::borrow::Cow::Borrowed("-"),
        }
    }

    /// Its game's difficulty, if the lobby gave it one.
    pub fn difficulty(&self) -> &'static str {
        self.game_difficulty.map_or("", difficulty_name)
    }

    /// Reads everything that has arrived. Returns why the connection
    /// ended, when it ended now.
    pub fn pump(&mut self) -> Option<String> {
        if let Some(why) = self.close_event.take() {
            return Some(why);
        }
        let name = self.play_name();
        let mut control_end = None;
        if let Some(conn) = self.conn.as_mut() {
            loop {
                match conn.control.recv() {
                    Ok(Some(b)) => conn.inbox.push_back(b),
                    Ok(None) => break,
                    Err(e) => {
                        control_end = Some(e.to_string());
                        break;
                    }
                }
            }
        }
        self.note_async_errors();
        if let Some(why) = control_end {
            let why = format!("the session's control stream ended: {why}");
            self.closed = Some(why.clone());
            self.conn = None;
            self.entered = false;
            return Some(why);
        }
        let mut game_end = None;
        if let Some(conn) = self.conn.as_mut() {
            if let Some(game) = conn.game.as_mut() {
                loop {
                    match game.recv() {
                        Ok(Some(Packet::Deliver(m))) => self.view.apply(&m, &name),
                        Ok(Some(Packet::Frame { ticked })) => {
                            self.view.frames += 1;
                            self.view.ticks += u64::from(ticked);
                        }
                        // Mod extension messages: D2RUST's own game sends none,
                        // and bots of a mod game don't read them (yet).
                        Ok(Some(Packet::Ext { .. })) => self.view.ext += 1,
                        Ok(Some(Packet::Send { .. })) => {
                            self.view
                                .bad
                                .push("the server sent a client-only packet".into());
                        }
                        Ok(None) => break,
                        Err(e) => {
                            game_end = Some(e);
                            break;
                        }
                    }
                }
            }
        }
        let e = game_end?;
        if let Some(conn) = self.conn.as_mut() {
            conn.game = None;
        }
        self.entered = false;
        // The game stream ends when a player leaves and the server closes
        // it; the session lives on. A lost connection ends both streams.
        let lost = match e {
            NetError::TimedOut | NetError::Malformed => true,
            NetError::Closed => self
                .conn
                .as_ref()
                .is_some_and(|conn| conn.client.closed_reason().is_some()),
        };
        if lost {
            let why = e.to_string();
            self.conn = None;
            self.closed = Some(why.clone());
            return Some(why);
        }
        None
    }

    /// Reads the session-level errors already queued: a takeover is the
    /// server ending this session on purpose.
    fn note_async_errors(&mut self) {
        let Some(mut inbox) = self.conn.as_mut().map(|c| std::mem::take(&mut c.inbox)) else {
            return;
        };
        let mut kept = VecDeque::new();
        let mut errors = Vec::new();
        while let Some(b) = inbox.pop_front() {
            if b.first() == Some(&msg_kind::ERROR) {
                if let Ok(e) = ErrorMsg::parse(&b) {
                    errors.push(e);
                    continue;
                }
            }
            kept.push_back(b);
        }
        if let Some(conn) = self.conn.as_mut() {
            conn.inbox = kept;
        }
        for e in errors {
            self.note_session_error(e);
        }
    }

    fn note_session_error(&mut self, e: ErrorMsg) {
        if e.code == ErrorCode::SESSION_TAKEN {
            self.taken = true;
        }
        self.session_error = Some(e);
    }

    /// Marks the connection closed for `pump` to report once.
    fn close_now(&mut self, why: String) -> Status {
        self.closed = Some(why.clone());
        self.conn = None;
        self.entered = false;
        self.close_event = Some(why.clone());
        Status::Closed(why)
    }

    /// Connects (or comes back with its session token when `resume`).
    /// `args`: `build=`, `protocol=`, `tick=`, `game=` override the hello;
    /// `trust=pinned|any|wrong` picks the certificate check.
    pub fn connect(
        &mut self,
        args: &[(String, String)],
        defaults: &HelloDefaults,
        cert: &ServerCert,
        resume: bool,
    ) -> Outcome {
        if self.conn.is_some() {
            if !resume {
                return Outcome::error("already connected");
            }
            // The old connection is the one this token comes back for; a
            // server that restarted no longer holds it. Let it go and
            // present the token on a new one.
            let _ = self.pump();
            if let Some(conn) = self.conn.take() {
                conn.close();
            }
            self.closed = None;
            self.close_event = None;
        }
        let mut hello = ClientHello {
            protocol: defaults.protocol,
            game: defaults.game.clone(),
            build: defaults.build.clone(),
            tick_rate: defaults.tick_rate,
            resume: None,
        };
        let mut trust = match cert.pin() {
            Some(f) => ServerTrust::Pinned(f),
            None => ServerTrust::Any,
        };
        for (k, v) in args {
            let bad = || Outcome::error(format!("connect {k}={v}: not a valid value"));
            match k.as_str() {
                "build" => hello.build = v.clone(),
                "game" => hello.game = v.clone(),
                "protocol" => match v.parse() {
                    Ok(p) => hello.protocol = p,
                    Err(_) => return bad(),
                },
                "tick" => match v.parse() {
                    Ok(t) => hello.tick_rate = t,
                    Err(_) => return bad(),
                },
                "trust" => {
                    trust = match v.as_str() {
                        "any" => ServerTrust::Any,
                        "pinned" => match cert.pin() {
                            Some(f) => ServerTrust::Pinned(f),
                            None => return Outcome::error("the server named no certificate"),
                        },
                        // A certificate the server doesn't hold.
                        "wrong" => {
                            ServerTrust::Pinned(fingerprint(b"not the server's certificate"))
                        }
                        _ => return bad(),
                    }
                }
                _ => return Outcome::error(format!("connect: unknown setting {k}")),
            }
        }
        if resume {
            match self.session {
                Some(t) => hello.resume = Some(t),
                None => return Outcome::error("no earlier session to come back to"),
            }
        }
        let previous = self.game_client;
        match Conn::open(self.relay.addr(), &hello, trust, BOT_TIMING) {
            Ok((conn, welcome)) => {
                self.conn = Some(conn);
                self.closed = None;
                self.session_error = None;
                self.taken = false;
                self.close_event = None;
                self.session = Some(welcome.session);
                let mut o = Outcome::ok()
                    .fact("max_players", welcome.max_players)
                    .fact("tick", welcome.tick_rate)
                    .fact("session", "yes")
                    .fact("server", &welcome.server_name);
                if !resume {
                    self.view = View::default();
                    self.entered = false;
                    self.game_token = None;
                    self.game_client = None;
                    self.logged_in = None;
                    self.selected = None;
                    self.known_chars.clear();
                    self.games.clear();
                    self.known_games.clear();
                    self.lists = 0;
                    self.listening = false;
                    self.game_id = None;
                    self.game_difficulty = None;
                    self.last_chat = None;
                    return o;
                }
                // A held session comes back with its game; a token the
                // server doesn't know still tries the old game token, so
                // the server's refusal names the rule.
                let kept = welcome.resume.is_some();
                let held = welcome.resume.as_ref().is_some_and(|s| s.game_id.is_some());
                if let Some(token) = self.game_token.filter(|_| held || !kept) {
                    match self.attach(Attach::Resume(token)) {
                        Ok(game) => {
                            let same = previous == Some(game.client);
                            o = o
                                .fact("client", game.client)
                                .fact("same", if same { "yes" } else { "no" });
                        }
                        Err(status) => {
                            // The old game is gone: start over cleanly.
                            if let Some(conn) = self.conn.take() {
                                conn.close();
                            }
                            self.session = None;
                            self.game_token = None;
                            self.game_client = None;
                            return Outcome {
                                status,
                                facts: BTreeMap::new(),
                            };
                        }
                    }
                }
                if !kept {
                    // Nothing was kept: a fresh session, with no account,
                    // character or game.
                    self.view = View::default();
                    self.entered = false;
                    self.game_token = None;
                    self.game_client = None;
                    self.logged_in = None;
                    self.selected = None;
                    self.known_chars.clear();
                    self.games.clear();
                    self.known_games.clear();
                    self.lists = 0;
                    self.listening = false;
                    self.game_id = None;
                    self.last_chat = None;
                    o = o.fact("same", "no");
                }
                o
            }
            Err(Open::Refused(r)) => Outcome {
                status: refused(&r),
                facts: BTreeMap::new(),
            },
            Err(Open::Error(e)) => Outcome::error(e),
        }
    }

    /// Opens the game stream and attaches it with `mode`, then keeps it
    /// for the game's packets. `Err` is the refusal or failure.
    fn attach(&mut self, mode: Attach) -> Result<Welcome, Status> {
        let Some(conn) = self.conn.as_mut() else {
            return Err(Status::Closed(
                self.closed
                    .clone()
                    .unwrap_or_else(|| "not connected".into()),
            ));
        };
        if conn.game.is_some() {
            return Err(Status::Error("already in a game".into()));
        }
        let mut stream = conn
            .client
            .open_stream()
            .map_err(|e| Status::Error(format!("opening the game stream: {e}")))?;
        stream
            .send(mode.body())
            .map_err(|e| Status::Error(format!("attach: {e}")))?;
        let answer = stream
            .recv_timeout(ATTACH_WAIT)
            .map_err(|e| Status::Error(format!("attach: {e}")))?;
        let Some(body) = answer else {
            return Err(Status::Error(
                "the server did not answer the attach in time".into(),
            ));
        };
        match parse_attach_answer(&body) {
            Ok(welcome) => {
                conn.game = Some(PacketStream::client(stream));
                Ok(welcome)
            }
            Err(r) => {
                stream.close();
                Err(refused(&r))
            }
        }
    }

    /// Enters a game directly: opens the game stream and attaches it with
    /// the stand-in's ticket unless one is already open (the stand-in
    /// keeps the stream for the next enter after a leave). The real
    /// server's games are entered through the lobby instead.
    pub fn open_game(&mut self) -> Outcome {
        if self.in_game() {
            return Outcome::ok();
        }
        match self.attach(Attach::Ticket([0; 16])) {
            Ok(welcome) => {
                self.view = View::default();
                self.game_token = Some(welcome.resume);
                self.game_client = Some(welcome.client);
                Outcome::ok()
                    .fact("client", welcome.client)
                    .fact("max_players", welcome.max_players)
                    .fact("tick", welcome.tick_rate)
            }
            Err(status) => Outcome {
                status,
                facts: BTreeMap::new(),
            },
        }
    }

    /// Ends the connection with a goodbye.
    pub fn disconnect(&mut self) -> Outcome {
        match self.conn.take() {
            Some(c) => {
                c.close();
                self.closed = Some("disconnected".into());
                self.session = None;
                self.game_token = None;
                self.game_client = None;
                self.selected = None;
                self.game_id = None;
                Outcome::ok()
            }
            None => Outcome::error("not connected"),
        }
    }

    /// Sends one C→S message on the game stream.
    pub fn send(&mut self, lane: Lane, msg: &[u8]) -> Outcome {
        let failed = {
            let Some(conn) = self.conn.as_mut() else {
                return self.closed_outcome();
            };
            let Some(game) = conn.game.as_mut() else {
                return Outcome::error("not in a game (enter first)");
            };
            game.send(Packet::Send {
                lane,
                msg: msg.to_vec(),
            })
            .err()
        };
        let Some(e) = failed else {
            return Outcome::ok();
        };
        Outcome {
            status: self.close_now(e.to_string()),
            facts: BTreeMap::new(),
        }
    }

    /// Sends one body on the session's control stream.
    fn send_control(&mut self, body: Vec<u8>) -> Result<(), Status> {
        if self.conn.is_none() {
            return Err(self.closed_outcome().status);
        }
        let failed = {
            let conn = self.conn.as_mut().expect("checked");
            conn.control.send(body).err()
        };
        match failed {
            None => Ok(()),
            Some(e) => Err(self.close_now(format!("the control stream: {e}"))),
        }
    }

    /// The outcome of a step on a bot without a connection.
    pub fn closed_outcome(&self) -> Outcome {
        match &self.closed {
            Some(why) => Outcome {
                status: Status::Closed(why.clone()),
                facts: BTreeMap::new(),
            },
            None => Outcome::error("not connected"),
        }
    }

    /// The next body from the control stream, waiting at most `wait`.
    fn read_body(&mut self, wait: Duration) -> Result<Vec<u8>, Status> {
        enum Got {
            Body(Vec<u8>),
            Empty,
            End(String),
        }
        let deadline = Instant::now() + wait;
        loop {
            if self.conn.is_none() {
                return Err(self.closed_outcome().status);
            }
            let got = {
                let conn = self.conn.as_mut().expect("checked");
                match conn.inbox.pop_front() {
                    Some(b) => Got::Body(b),
                    None => match conn.control.recv() {
                        Ok(Some(b)) => Got::Body(b),
                        Ok(None) => Got::Empty,
                        Err(e) => Got::End(e.to_string()),
                    },
                }
            };
            match got {
                Got::Body(b) => return Ok(b),
                Got::End(e) => {
                    return Err(self.close_now(format!("the session's control stream ended: {e}")))
                }
                Got::Empty => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Status::Error("the server did not answer in time".into()));
            }
            std::thread::sleep(left.min(POLL));
        }
    }

    /// Takes in a body a driver doesn't answer: a game list update, a
    /// chat line, or a session-level error. True when it was one.
    fn handle_event(&mut self, body: &[u8]) -> bool {
        let Some(&kind) = body.first() else {
            return false;
        };
        match kind {
            msg_kind::ERROR => match ErrorMsg::parse(body) {
                Ok(e) => {
                    self.note_session_error(e);
                    true
                }
                Err(_) => false,
            },
            k if k == lobby::kind::GAME_LIST_BEGIN => {
                if matches!(
                    LobbyEvent::parse(body),
                    Ok(LobbyEvent::GameListBegin { .. })
                ) {
                    self.games.clear();
                    return true;
                }
                false
            }
            k if k == lobby::kind::GAME_ENTRY => match LobbyEvent::parse(body) {
                Ok(LobbyEvent::GameEntry(g)) => {
                    self.known_games.insert(g.name.clone());
                    self.games.insert(g.game_id, g);
                    true
                }
                _ => false,
            },
            k if k == lobby::kind::GAME_LIST_END => {
                if matches!(LobbyEvent::parse(body), Ok(LobbyEvent::GameListEnd)) {
                    self.lists += 1;
                    self.listening = true;
                    return true;
                }
                false
            }
            k if k == lobby::kind::GAME_GONE => match LobbyEvent::parse(body) {
                Ok(LobbyEvent::GameGone { game_id }) => {
                    self.games.remove(&game_id);
                    true
                }
                _ => false,
            },
            k if k == lobby::kind::CHAT_LINE => match LobbyEvent::parse(body) {
                Ok(LobbyEvent::ChatLine { from, text, .. }) => {
                    self.last_chat = Some((from, text));
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Reads bodies until one of the `answers` kinds arrives (an `ERROR`
    /// for this request's kind is the failure); events in between are
    /// taken in.
    fn request(
        &mut self,
        req: &dyn RequestBody,
        answers: &[u8],
        wait: Duration,
    ) -> Result<Vec<u8>, Failed> {
        let kind = req.kind();
        let body = req.body();
        self.send_control(body).map_err(Failed::Local)?;
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let b = self.read_body(left).map_err(Failed::Local)?;
            match b.first() {
                Some(&msg_kind::ERROR) => {
                    let e = ErrorMsg::parse(&b)
                        .map_err(|_| Failed::Local(Status::Error("a malformed error".into())))?;
                    if e.request_kind == kind {
                        return Err(Failed::Server(e));
                    }
                    self.note_session_error(e);
                }
                Some(k) if answers.contains(k) => return Ok(b),
                Some(_) => {
                    if !self.handle_event(&b) {
                        return Err(Failed::Local(Status::Error(format!(
                            "unexpected answer 0x{:02X} to 0x{kind:02X}",
                            b[0]
                        ))));
                    }
                }
                None => {
                    return Err(Failed::Local(Status::Error(
                        "the server sent an empty answer".into(),
                    )))
                }
            }
        }
    }

    /// `register ACCOUNT PASSWORD`: a new account, logged in at once.
    pub fn register(&mut self, account: &str, password: &str, wait: Duration) -> Outcome {
        let req = AccountRequest::Register {
            account: account.into(),
            password: password.into(),
        };
        self.login_step(&req, wait)
    }

    /// `login ACCOUNT PASSWORD`.
    pub fn login(&mut self, account: &str, password: &str, wait: Duration) -> Outcome {
        let req = AccountRequest::Login {
            account: account.into(),
            password: password.into(),
        };
        self.login_step(&req, wait)
    }

    /// A register or login, with the answer's time and a refusal's wait.
    fn login_step(&mut self, req: &AccountRequest, wait: Duration) -> Outcome {
        let started = Instant::now();
        let answer = self.request(req, &[lobby_answer::LOGGED_IN], wait);
        let ms = started.elapsed().as_millis();
        match answer {
            Ok(body) => match AccountEvent::parse(&body) {
                Ok(AccountEvent::LoggedIn {
                    account,
                    max_characters,
                }) => {
                    self.logged_in = Some(account.clone());
                    Outcome::ok()
                        .fact("account", account)
                        .fact("max_characters", max_characters)
                        .fact("ms", ms)
                        .fact("wait_s", 0)
                }
                Ok(_) => Outcome::error("the server answered something else"),
                Err(e) => Outcome::error(format!("a malformed login answer: {e}")),
            },
            Err(f) => f.outcome(ms),
        }
    }

    /// `logout`: the server ends the session and closes the connection.
    pub fn logout(&mut self, wait: Duration) -> Outcome {
        if let Err(status) = self.send_control(AccountRequest::Logout.body()) {
            return Outcome {
                status,
                facts: BTreeMap::new(),
            };
        }
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.read_body(left) {
                Ok(_) => {}
                Err(Status::Closed(_)) => {
                    if let Some(conn) = self.conn.take() {
                        conn.close();
                    }
                    return Outcome::ok();
                }
                Err(e) => {
                    return Outcome {
                        status: e,
                        facts: BTreeMap::new(),
                    }
                }
            }
            if Instant::now() >= deadline {
                return Outcome::error("the server did not close the session");
            }
        }
    }

    /// `char-create NAME CLASS [hardcore] [classic]`.
    pub fn char_create(
        &mut self,
        name: &str,
        class: u8,
        hardcore: bool,
        classic: bool,
        wait: Duration,
    ) -> Outcome {
        self.known_chars.insert(name.to_string());
        let req = AccountRequest::CharCreate {
            name: name.into(),
            class,
            hardcore,
            expansion: !classic,
        };
        let started = Instant::now();
        match self.request(&req, &[lobby_answer::CHAR_CREATED], wait) {
            Ok(body) => match AccountEvent::parse(&body) {
                Ok(AccountEvent::CharCreated(c)) => {
                    self.known_chars.insert(c.name.clone());
                    Outcome::ok()
                        .fact("name", c.name)
                        .fact("class", c.class)
                        .fact("difficulties", c.difficulties)
                }
                Ok(_) => Outcome::error("the server answered something else"),
                Err(e) => Outcome::error(format!("a malformed character answer: {e}")),
            },
            Err(f) => f.outcome(started.elapsed().as_millis()),
        }
    }

    /// `char-list`: the account's characters.
    pub fn char_list(&mut self, wait: Duration) -> Outcome {
        let started = Instant::now();
        match self.request(&AccountRequest::CharList, &[lobby_answer::CHAR_LIST], wait) {
            Ok(body) => match AccountEvent::parse(&body) {
                Ok(AccountEvent::CharList(chars)) => {
                    for c in &chars {
                        self.known_chars.insert(c.name.clone());
                    }
                    let mut o = Outcome::ok().fact("count", chars.len());
                    for name in &self.known_chars {
                        let there = chars.iter().any(|c| c.name.eq_ignore_ascii_case(name));
                        o = o.fact(format!("has.{name}"), if there { "yes" } else { "no" });
                    }
                    for c in &chars {
                        o = o
                            .fact(format!("{}.level", c.name), c.level)
                            .fact(format!("{}.class", c.name), c.class)
                            .fact(format!("{}.difficulties", c.name), c.difficulties);
                    }
                    o
                }
                Ok(_) => Outcome::error("the server answered something else"),
                Err(e) => Outcome::error(format!("a malformed character list: {e}")),
            },
            Err(f) => f.outcome(started.elapsed().as_millis()),
        }
    }

    /// `char-select NAME`: the lobby plays with this character.
    pub fn char_select(&mut self, name: &str, wait: Duration) -> Outcome {
        self.known_chars.insert(name.to_string());
        let req = AccountRequest::CharSelect { name: name.into() };
        let started = Instant::now();
        match self.request(&req, &[lobby_answer::CHAR_SELECTED], wait) {
            Ok(body) => match AccountEvent::parse(&body) {
                Ok(AccountEvent::CharSelected(c)) => {
                    let o = Outcome::ok()
                        .fact("name", &c.name)
                        .fact("difficulties", c.difficulties);
                    self.selected = Some(c);
                    // The lobby screen lists the games: subscribing is what
                    // lets this player hear lobby chat.
                    let left = wait.saturating_sub(started.elapsed());
                    let _ = self.list_games(left);
                    o
                }
                Ok(_) => Outcome::error("the server answered something else"),
                Err(e) => Outcome::error(format!("a malformed character answer: {e}")),
            },
            Err(f) => f.outcome(started.elapsed().as_millis()),
        }
    }

    /// `char-delete NAME PASSWORD`: the account's password confirms it.
    pub fn char_delete(&mut self, name: &str, password: &str, wait: Duration) -> Outcome {
        self.known_chars.insert(name.to_string());
        let req = AccountRequest::CharDelete {
            name: name.into(),
            password: password.into(),
        };
        let started = Instant::now();
        match self.request(&req, &[lobby_answer::CHAR_DELETED], wait) {
            Ok(body) => match AccountEvent::parse(&body) {
                Ok(AccountEvent::CharDeleted { name }) => {
                    if self
                        .selected
                        .as_ref()
                        .is_some_and(|c| c.name.eq_ignore_ascii_case(&name))
                    {
                        self.selected = None;
                    }
                    Outcome::ok().fact("name", name)
                }
                Ok(_) => Outcome::error("the server answered something else"),
                Err(e) => Outcome::error(format!("a malformed character answer: {e}")),
            },
            Err(f) => f.outcome(started.elapsed().as_millis()),
        }
    }

    /// Listens from now on and replaces the game list with a snapshot.
    fn list_games(&mut self, wait: Duration) -> Result<(), Failed> {
        let before = self.lists;
        let body = LobbyRequest::ListGames.body();
        self.send_control(body).map_err(Failed::Local)?;
        let deadline = Instant::now() + wait;
        while self.lists == before {
            let left = deadline.saturating_duration_since(Instant::now());
            let b = self.read_body(left).map_err(Failed::Local)?;
            if b.first() == Some(&msg_kind::ERROR) {
                let e = ErrorMsg::parse(&b)
                    .map_err(|_| Failed::Local(Status::Error("a malformed error".into())))?;
                return Err(Failed::Server(e));
            }
            self.handle_event(&b);
        }
        Ok(())
    }

    /// `games`: the open games with their counts, kind, lock and whether
    /// the selected character may join.
    pub fn games(&mut self, wait: Duration) -> Outcome {
        let started = Instant::now();
        match self.list_games(wait) {
            Ok(()) => {
                let mut o = Outcome::ok().fact("count", self.games.len());
                for name in &self.known_games {
                    let there = self
                        .games
                        .values()
                        .any(|g| g.name.eq_ignore_ascii_case(name));
                    o = o.fact(format!("has.{name}"), if there { "yes" } else { "no" });
                }
                for g in self.games.values() {
                    o = o
                        .fact(format!("{}.players", g.name), g.players)
                        .fact(format!("{}.max", g.name), g.max_players)
                        .fact(
                            format!("{}.difficulty", g.name),
                            difficulty_name(g.difficulty),
                        )
                        .fact(
                            format!("{}.locked", g.name),
                            if g.locked { "yes" } else { "no" },
                        )
                        .fact(format!("{}.joinable", g.name), g.joinable);
                }
                o
            }
            Err(f) => f.outcome(started.elapsed().as_millis()),
        }
    }

    /// `create-game NAME [password=P] [difficulty=...] [max=N]`: makes the
    /// game, takes its seat and enters it.
    pub fn create_game(
        &mut self,
        name: &str,
        password: &str,
        difficulty: u8,
        max_players: u8,
        wait: Duration,
        until: Instant,
    ) -> Outcome {
        self.known_games.insert(name.to_string());
        let req = LobbyRequest::CreateGame {
            name: name.into(),
            password: password.into(),
            difficulty,
            max_players,
        };
        let started = Instant::now();
        match self.request(&req, &[lobby_answer::JOIN_OK], wait) {
            Ok(body) => self.joined(&body, name, until, started),
            Err(f) => f.outcome(started.elapsed().as_millis()),
        }
    }

    /// `join-game NAME [password=P]`: finds the game in the list, takes a
    /// seat and enters it.
    pub fn join_game(
        &mut self,
        name: &str,
        password: &str,
        wait: Duration,
        until: Instant,
    ) -> Outcome {
        self.known_games.insert(name.to_string());
        let started = Instant::now();
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if let Err(f) = self.list_games(left.max(POLL)) {
                return f.outcome(started.elapsed().as_millis());
            }
            let found = self
                .games
                .values()
                .find(|g| g.name.eq_ignore_ascii_case(name))
                .map(|g| (g.game_id, g.difficulty));
            let Some((game_id, difficulty)) = found else {
                if Instant::now() >= deadline {
                    return Outcome::error(format!("no game {name} in the list"));
                }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            };
            let req = LobbyRequest::JoinGame {
                game_id,
                password: password.into(),
            };
            let left = deadline.saturating_duration_since(Instant::now());
            match self.request(&req, &[lobby_answer::JOIN_OK], left.max(POLL)) {
                Ok(body) => {
                    let o = self.joined(&body, name, until, started);
                    return o.fact("difficulty", difficulty_name(difficulty));
                }
                Err(Failed::Server(e))
                    if e.code == ErrorCode::CHAR_IN_GAME
                        || e.code == ErrorCode::ALREADY_IN_GAME =>
                {
                    // The character leaves a game a moment before the
                    // lobby hears of it: joining again right after a leave
                    // waits for that.
                    if Instant::now() >= until {
                        return Failed::Server(e).outcome(started.elapsed().as_millis());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(f) => return f.outcome(started.elapsed().as_millis()),
            }
        }
    }

    /// The seat is held (`JoinOk`): attach the game stream and enter.
    fn joined(&mut self, body: &[u8], name: &str, until: Instant, started: Instant) -> Outcome {
        let join = match LobbyEvent::parse(body) {
            Ok(LobbyEvent::JoinOk {
                game_id,
                ticket,
                difficulty,
            }) => (game_id, ticket, difficulty),
            _ => return Outcome::error("the server answered something else to the join"),
        };
        let (game_id, ticket, difficulty) = join;
        self.game_id = Some(game_id);
        self.game_difficulty = Some(difficulty);
        self.listening = false;
        let welcome = match self.attach(Attach::Ticket(ticket)) {
            Ok(w) => w,
            Err(status) => {
                return Outcome {
                    status,
                    facts: BTreeMap::from([("game_id".into(), game_id.to_string())]),
                }
            }
        };
        self.view = View::default();
        self.game_token = Some(welcome.resume);
        self.game_client = Some(welcome.client);
        let class = self.selected.as_ref().map_or(0, |c| c.class);
        let mut o = self.enter_current(name, class, difficulty, until);
        o = o
            .fact("game_id", game_id)
            .fact("difficulty", difficulty_name(difficulty))
            .fact("ms", started.elapsed().as_millis());
        o
    }

    /// Sends the 0x67 join for the game just attached, waits for the
    /// game's `0x02`, then sends 0x6B and waits for its own player.
    pub fn enter_current(
        &mut self,
        game: &str,
        class: u8,
        difficulty: u8,
        until: Instant,
    ) -> Outcome {
        if !self.in_game() {
            return Outcome::error("not in a game (create or join one first)");
        }
        let mut game_name = [0u8; 16];
        let n = game.len().min(15);
        game_name[..n].copy_from_slice(&game.as_bytes()[..n]);
        let mut char_name = [0u8; 16];
        let name = self.play_name();
        let n = name.len().min(15);
        char_name[..n].copy_from_slice(&name.as_bytes()[..n]);
        let req = d2_proto::client::CreateGame {
            game_name,
            class,
            difficulty,
            char_name,
            flags: (1 << 20) | 0x4,
            ..d2_proto::client::CreateGame::default()
        };
        self.view.loaded = false;
        self.view.exiting = false;
        let o = self.send(Lane::System, &req.encode());
        if o.status != Status::Ok {
            return o;
        }
        self.entered = true;
        if let Err(e) = self.wait_view(until, |v| v.loaded) {
            return e;
        }
        let o = self.send(Lane::System, &[0x6B]);
        if o.status != Status::Ok {
            return o;
        }
        if let Err(e) = self.wait_view(until, |v| v.own.is_some()) {
            return e;
        }
        match self
            .view
            .own
            .and_then(|g| self.view.players.get(&g).map(|p| (g, p)))
        {
            Some((g, p)) => Outcome::ok().fact("guid", g).fact("x", p.x).fact("y", p.y),
            None => Outcome {
                status: Status::Refused {
                    code: "no-player".into(),
                    text: "no player assigned in time".into(),
                },
                facts: BTreeMap::new(),
            },
        }
    }

    /// Pumps until the view holds `done`; a closed connection or game is
    /// the failure.
    fn wait_view(&mut self, until: Instant, done: impl Fn(&View) -> bool) -> Result<(), Outcome> {
        loop {
            if let Some(why) = self.pump() {
                return Err(Outcome {
                    status: Status::Closed(why),
                    facts: BTreeMap::new(),
                });
            }
            if done(&self.view) {
                return Ok(());
            }
            if !self.in_game() {
                return Err(Outcome::error("the game stream ended"));
            }
            if Instant::now() >= until {
                return Err(Outcome::error("the game did not answer in time"));
            }
            std::thread::sleep(POLL);
        }
    }

    /// `leave-game`: leaves the game (0x69), ends its stream and is back
    /// in the lobby.
    pub fn leave_game(&mut self, wait: Duration) -> Outcome {
        if !self.in_game() {
            return Outcome::error("not in a game");
        }
        self.view.exiting = false;
        let o = self.send(Lane::System, &[0x69]);
        if o.status != Status::Ok {
            return o;
        }
        let deadline = Instant::now() + wait;
        loop {
            self.pump();
            if self.view.exiting || !self.in_game() || !self.connected() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(POLL);
        }
        if let Some(conn) = self.conn.as_mut() {
            if let Some(mut game) = conn.game.take() {
                game.close();
            }
        }
        self.entered = false;
        self.game_token = None;
        self.game_id = None;
        self.game_difficulty = None;
        self.view = View::default();
        Outcome::ok()
    }

    /// `chat TEXT`: says a line in the lobby, heard by every listener.
    pub fn chat(&mut self, text: &str, wait: Duration) -> Outcome {
        if !self.listening {
            if let Err(f) = self.list_games(wait) {
                return f.outcome(0);
            }
        }
        let body = LobbyRequest::Chat { text: text.into() }.body();
        if let Err(status) = self.send_control(body) {
            return Outcome {
                status,
                facts: BTreeMap::new(),
            };
        }
        let me = self.play_name();
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let b = match self.read_body(left) {
                Ok(b) => b,
                Err(e) => {
                    return Outcome {
                        status: e,
                        facts: BTreeMap::new(),
                    }
                }
            };
            if b.first() == Some(&msg_kind::ERROR) {
                if let Ok(e) = ErrorMsg::parse(&b) {
                    if e.request_kind == lobby::kind::CHAT {
                        return Failed::Server(e).outcome(0);
                    }
                    self.note_session_error(e);
                }
                continue;
            }
            let Ok(LobbyEvent::ChatLine { from, text, .. }) = LobbyEvent::parse(&b) else {
                self.handle_event(&b);
                continue;
            };
            self.last_chat = Some((from.clone(), text.clone()));
            if from.eq_ignore_ascii_case(&me) {
                return Outcome::ok();
            }
        }
    }

    /// `heard`: the last chat line and who said it.
    pub fn heard(&mut self) -> Outcome {
        self.drain_events();
        match &self.last_chat {
            Some((from, text)) => Outcome::ok().fact("last", text).fact("from", from),
            None => Outcome::ok(),
        }
    }

    /// Takes in every queued event, leaving request answers alone.
    fn drain_events(&mut self) {
        let Some(mut inbox) = self.conn.as_mut().map(|c| std::mem::take(&mut c.inbox)) else {
            return;
        };
        let mut kept = VecDeque::new();
        while let Some(b) = inbox.pop_front() {
            if !self.handle_event(&b) {
                kept.push_back(b);
            }
        }
        if let Some(conn) = self.conn.as_mut() {
            conn.inbox = kept;
        }
    }
}

/// A request a [`Bot::request`] can send: its kind and its body.
trait RequestBody {
    fn kind(&self) -> u8;
    fn body(&self) -> Vec<u8>;
}

impl RequestBody for AccountRequest {
    fn kind(&self) -> u8 {
        AccountRequest::kind(self)
    }

    fn body(&self) -> Vec<u8> {
        AccountRequest::body(self)
    }
}

impl RequestBody for LobbyRequest {
    fn kind(&self) -> u8 {
        LobbyRequest::kind(self)
    }

    fn body(&self) -> Vec<u8> {
        LobbyRequest::body(self)
    }
}

/// The S→C kinds the account and lobby drivers wait for.
mod lobby_answer {
    use d2_net::msg::account::kind as account;
    use d2_net::msg::lobby::kind as lobby;

    pub const LOGGED_IN: u8 = account::LOGGED_IN;
    pub const CHAR_LIST: u8 = account::CHAR_LIST_ANSWER;
    pub const CHAR_CREATED: u8 = account::CHAR_CREATED;
    pub const CHAR_DELETED: u8 = account::CHAR_DELETED;
    pub const CHAR_SELECTED: u8 = account::CHAR_SELECTED;
    pub const JOIN_OK: u8 = lobby::JOIN_OK;
}

/// A difficulty's word, as scenarios say it.
pub fn difficulty_name(difficulty: u8) -> &'static str {
    match difficulty {
        1 => "nightmare",
        2 => "hell",
        _ => "normal",
    }
}

/// A difficulty as its number: `normal` (0), `nightmare` (1), `hell` (2).
pub fn difficulty_number(word: &str) -> Option<u8> {
    match word {
        "normal" => Some(0),
        "nightmare" => Some(1),
        "hell" => Some(2),
        _ => word.parse().ok().filter(|d| *d < 3),
    }
}
