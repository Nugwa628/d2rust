//! `mp-test-synth`: a stand-in server for the tool. It runs the one-room
//! town of the synthetic game data (no game files) behind a real QUIC
//! endpoint and the server's hub, the way the M1d multi-client harness
//! runs it in process: a session flow whose loader gives each client a
//! fresh character of the class and name it asked for.
//!
//! It speaks wire version 4 (contracts 3.1): a client opens the control
//! stream and says [`ClientHello`]; the stand-in answers [`SessionWelcome`]
//! (or [`Refused`]). The client then opens the game stream and attaches
//! it ([`Attach`]): its first game is entered with any ticket (the
//! stand-in has no lobby), and a game slot held after a lost connection
//! comes back with the game's resume token.
//!
//! It is what the tool tests itself against, and what the game-level
//! scenarios run on while `d2rust-server` can't run games yet. It has no
//! accounts, lobby or saved characters, and in its one-room town a walk is
//! queued but nothing comes of it (no move reaches anyone): its ready line
//! announces only `connect` and `game`.
//!
//! ```text
//! mp-test-synth --listen 127.0.0.1:0 --data DIR [--players N] [--grace MS] [--build S]
//! ```
//!
//! It stops at the end of its standard input.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use d2_net::msg::ErrorCode;
use d2_net::wire::{
    Attach, ClientHello, Refused, ResumeState, ServerInfo, SessionWelcome, Timing, Welcome,
};
use d2_net::{ClientId, NetError, ResumeToken};
use d2_quic::{FramedStream, Identity, PacketStream, QuicConn, QuicServer};
use d2_server::adapters::handlers::world::ActionWorld;
use d2_server::adapters::session::Entry;
use d2_server::adapters::session_flow::{CharacterLoader, CreateGame, Loaded, SessionFlow};
use d2_server::adapters::{ProtoSizes, SimGame};
use d2_server::host::Host;
use d2_server::hub::{Accepted, Hub};
use d2_server::seams::{Clock, MessageSink, SessionHandler};
use d2_server::transport::Classified;
use d2_sim::game::Game;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::worldgen::WorldSim;
use test_fixtures::game::{ActCreation, GameData, Seams, Sim};
use test_fixtures::{install, synth as fixture};

/// The town level of act 1.
const TOWN: u32 = 1;
/// The game's init seed and its seed (any fixed values).
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
/// The game id this stand-in serves.
const GAME: &str = "d2rust";

/// The server's connection timing: short, like the bots'.
pub const SYNTH_TIMING: Timing = Timing {
    keepalive: Duration::from_millis(500),
    timeout: Duration::from_secs(3),
};

/// The most game slots it holds at once.
const MAX_CONNECTIONS: usize = 128;

/// The pause between two server frames (the host ticks when a tick is
/// due).
const FRAME_PAUSE: Duration = Duration::from_millis(5);

struct NoSession;
impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

/// Milliseconds since the server started (from 1000, like the tests).
struct Wall(Instant);
impl Clock for Wall {
    fn now_ms(&mut self) -> u32 {
        1000 + self.0.elapsed().as_millis() as u32
    }
}

type SynthHost = Host<Sim, ProtoSizes, NoSession, Wall>;
type SynthHub = Hub<Sim, ProtoSizes, NoSession, Wall>;

/// A session above the hub: the game slot it holds and the token it comes
/// back with.
#[derive(Debug)]
struct Session {
    /// The token a client presents in its hello to take the session back.
    token: ResumeToken,
    /// The connection its control stream is on.
    conn: u64,
    /// Its control stream, while the connection is up.
    control: Option<FramedStream>,
    /// The hub client of its game slot, once attached.
    client: Option<ClientId>,
    /// Its game's resume token, once attached.
    game: Option<ResumeToken>,
}

/// Every open session, by number, and the lookups the protocol needs.
#[derive(Debug, Default)]
struct Sessions {
    map: BTreeMap<u64, Session>,
    /// Session token (as bytes) to session number.
    tokens: BTreeMap<[u8; 16], u64>,
    /// Connection number to the session whose control stream it carries.
    by_conn: BTreeMap<u64, u64>,
    /// The next session number.
    next: u64,
}

impl Sessions {
    /// A new session for `conn`, or the one `resume` names with a game it
    /// still holds. `Err` when the stand-in is full.
    fn open(
        &mut self,
        hub: &SynthHub,
        conn: u64,
        resume: Option<ResumeToken>,
    ) -> Result<(u64, Option<ResumeState>, ResumeToken), Refused> {
        if let Some(sid) = resume.and_then(|t| self.tokens.get(&t.0).copied()) {
            if self.map.contains_key(&sid) {
                let s = self.map.get_mut(&sid).expect("checked");
                // The game is gone once the hub no longer holds it (its
                // grace ran out): the session comes back without it.
                if s.game.is_some_and(|t| hub.resumable(&t).is_none()) {
                    s.game = None;
                    s.client = None;
                }
                let state = ResumeState {
                    account: "mp-test".into(),
                    character: None,
                    game_id: s.game.map(|_| 0),
                };
                self.tokens.remove(&s.token.0);
                let token = ResumeToken::random();
                s.token = token;
                s.conn = conn;
                self.by_conn.insert(conn, sid);
                self.tokens.insert(token.0, sid);
                return Ok((sid, Some(state), token));
            }
        }
        if hub.clients().count() + hub.parked().count() >= MAX_CONNECTIONS {
            return Err(Refused::new(ErrorCode::SERVER_FULL));
        }
        let sid = self.next;
        self.next += 1;
        let token = ResumeToken::random();
        self.map.insert(
            sid,
            Session {
                token,
                conn,
                control: None,
                client: None,
                game: None,
            },
        );
        self.tokens.insert(token.0, sid);
        self.by_conn.insert(conn, sid);
        Ok((sid, None, token))
    }

    /// Gives the session its (new) control stream.
    fn set_control(&mut self, sid: u64, control: FramedStream) {
        if let Some(s) = self.map.get_mut(&sid) {
            s.control = Some(control);
        }
    }

    /// Ends a session: its token is no longer valid.
    fn remove(&mut self, sid: u64) {
        if let Some(s) = self.map.remove(&sid) {
            self.tokens.remove(&s.token.0);
            self.by_conn.retain(|_, v| *v != sid);
        }
    }

    /// Reads every control stream once: a goodbye ends the session and
    /// its game; a failed link holds the session (and the hub holds the
    /// game for its grace).
    fn read_controls(&mut self, hub: &mut SynthHub) {
        let mut ended = Vec::new();
        for (&sid, s) in self.map.iter_mut() {
            let Some(control) = s.control.as_mut() else {
                continue;
            };
            match control.recv() {
                Ok(None) => {}
                // The stand-in has no account or lobby messages.
                Ok(Some(_)) => ended.push((sid, true)),
                Err(e) => ended.push((sid, matches!(e, NetError::Closed))),
            }
        }
        for (sid, clean) in ended {
            if clean {
                let client = self.map.get(&sid).and_then(|s| s.client);
                self.remove(sid);
                if let Some(c) = client {
                    hub.disconnect(c);
                }
            } else if let Some(s) = self.map.get_mut(&sid) {
                s.control = None;
            }
        }
    }

    /// Forgets sessions whose game the hub dropped after its grace: their
    /// tokens are no longer resumable.
    fn expire(&mut self, hub: &SynthHub) {
        let gone: Vec<u64> = self
            .map
            .iter()
            .filter(|(_, s)| {
                s.control.is_none()
                    && s.client.is_some_and(|c| {
                        !hub.clients().any(|x| x == c) && !hub.parked().any(|x| x == c)
                    })
            })
            .map(|(&sid, _)| sid)
            .collect();
        for sid in gone {
            self.remove(sid);
        }
    }
}

/// The player as the save loader leaves it: no room, at (0, 0), mode 1.
fn alloc_player(s: &mut Sim, class: u32) -> UnitId {
    let req = AllocRequest {
        ty: UnitType::Player,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = s
        .events
        .action
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .expect("allocated");
    s.events
        .action
        .sys
        .units
        .get_mut(p)
        .expect("just made")
        .mode = 1;
    p
}

/// The synthetic town behind a host with a session flow of player limit
/// `max_players`.
fn host(d: &GameData, max_players: usize) -> Result<SynthHost, String> {
    let (mut sim, _) = d
        .world_sim(
            ActCreation::TownOnly,
            INIT,
            TOWN,
            GAME_SEED,
            Seams::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut game = Game::new();
    game.lists.ensure_act(0).map_err(|e| format!("{e:?}"))?;
    sim.action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |dr, svc| {
            let lv = dr.get_or_alloc_level(svc.data, svc.types, TOWN)?;
            let r = dr.level_rooms(lv)[0];
            dr.stream_room(svc, r)
        })
        .ok_or("act 0 has no DRLG")?
        .map_err(|e| format!("town: {e:?}"))?
        .ok_or("the town room isn't active")?;
    let world = ActionWorld {
        waypoints: Some(d.waypoints().map_err(|e| e.to_string())?),
        ..ActionWorld::default()
    };
    let mut s: Sim = SimGame::with_world(game, sim, world);
    let loader: CharacterLoader<WorldSim<Seams>, ActionWorld> =
        Box::new(|s: &mut Sim, _: ClientId, r: &CreateGame| {
            let player = alloc_player(s, u32::from(r.class));
            Ok(Loaded {
                player,
                entry: Entry::new(0, r.char_name),
            })
        });
    s.set_session(SessionFlow::new(loader).with_max_players(max_players));
    Ok(Host::new(s, ProtoSizes, NoSession, Wall(Instant::now())))
}

/// The command line.
struct Args {
    listen: SocketAddr,
    data: PathBuf,
    players: usize,
    grace_ms: u32,
    build: String,
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        data: PathBuf::from("mp-test-synth"),
        players: d2_server::adapters::session_flow::DEFAULT_MAX_PLAYERS,
        grace_ms: 0,
        build: "0.0.0".into(),
    };
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("{k} takes a value"))?;
        let bad = || format!("{k} {v}: not a valid value");
        match k.as_str() {
            "--listen" => a.listen = v.parse().map_err(|_| bad())?,
            "--data" => a.data = PathBuf::from(v),
            "--players" => a.players = v.parse().map_err(|_| bad())?,
            "--grace" => a.grace_ms = v.parse().map_err(|_| bad())?,
            "--build" => a.build = v,
            _ => return Err(format!("unknown argument {k}")),
        }
    }
    Ok(a)
}

/// The program.
pub fn main() -> ExitCode {
    let a = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("mp-test-synth: {e}\nusage: mp-test-synth --listen ADDR --data DIR [--players N] [--grace MS] [--build S]");
            return ExitCode::from(2);
        }
    };
    match serve(&a) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mp-test-synth: {e}");
            ExitCode::FAILURE
        }
    }
}

fn serve(a: &Args) -> Result<(), String> {
    std::fs::create_dir_all(&a.data).map_err(|e| e.to_string())?;
    let inst = install::build(&a.data.join("install"), &fixture::synthetic())
        .map_err(|e| e.to_string())?;
    let data = GameData::from_install(&inst).map_err(|e| e.to_string())?;
    let mut hub: SynthHub = Hub::new(host(&data, a.players)?);
    hub.set_grace(a.grace_ms);
    let identity = Identity::self_signed().map_err(|e| e.to_string())?;
    let cert = a.data.join("cert.der");
    std::fs::write(&cert, &identity.cert).map_err(|e| e.to_string())?;
    // The QUIC endpoint runs on a small runtime of its own; the hub and the
    // sessions stay on this thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let server = QuicServer::bind(a.listen, identity, SYNTH_TIMING, rt.handle().clone())
        .map_err(|e| e.to_string())?;
    let info = ServerInfo {
        protocol: d2_proto::PROTOCOL_VERSION,
        game: GAME.into(),
        build: a.build.clone(),
        tick_rate: d2_sim::tick::TickRate::BASE.hz(),
    };
    println!(
        "ready listen={} cert={} game={} caps=connect,game build={} protocol={} tick={}",
        server.local_addr().map_err(|e| e.to_string())?,
        cert.display(),
        info.game,
        info.build,
        info.protocol,
        info.tick_rate
    );

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                match line {
                    Ok(l) if l.trim() == "quit" => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            stop.store(true, Ordering::Relaxed);
        });
    }

    let mut next: ClientId = 0;
    let mut next_conn: u64 = 0;
    // Connections whose hello hasn't arrived yet, with when they came.
    let mut waiting: Vec<(u64, QuicConn, FramedStream, Instant)> = Vec::new();
    // The game streams the connections' tasks have taken, with their
    // connection.
    let (game_tx, game_rx) = mpsc::channel::<(u64, FramedStream)>();
    let mut sessions = Sessions::default();
    while !stop.load(Ordering::Relaxed) {
        while let Some((conn, control)) = server.try_accept() {
            let id = next_conn;
            next_conn += 1;
            waiting.push((id, conn, control, Instant::now()));
        }
        let mut still = Vec::new();
        for (conn_id, conn, mut control, since) in waiting.drain(..) {
            let body = match control.recv() {
                Ok(Some(b)) => b,
                Ok(None) if since.elapsed() < SYNTH_TIMING.timeout => {
                    still.push((conn_id, conn, control, since));
                    continue;
                }
                Ok(None) => {
                    eprintln!("handshake: no hello in time");
                    control.close();
                    continue;
                }
                Err(e) => {
                    eprintln!("handshake: {e}");
                    continue;
                }
            };
            let hello = match ClientHello::parse(&body).and_then(|h| h.check(&info).map(|()| h)) {
                Ok(h) => h,
                Err(r) => {
                    let _ = control.send(r.body());
                    control.close();
                    eprintln!("handshake: refused: {r}");
                    continue;
                }
            };
            let opened = sessions.open(&hub, conn_id, hello.resume);
            let (sid, state, token) = match opened {
                Ok(x) => x,
                Err(r) => {
                    let _ = control.send(r.body());
                    control.close();
                    eprintln!("handshake: refused: {r}");
                    continue;
                }
            };
            let welcome = SessionWelcome {
                tick_rate: info.tick_rate,
                max_players: a.players as u32,
                session: token,
                server_name: "mp-test-synth".into(),
                registration_open: false,
                resume: state,
            };
            if let Err(e) = control.send(welcome.body()) {
                eprintln!("handshake: {e}");
                sessions.remove(sid);
                continue;
            }
            sessions.set_control(sid, control);
            // The client's game stream comes over this connection.
            let tx = game_tx.clone();
            let c = conn.clone();
            rt.spawn(async move {
                while let Some(stream) = c.accept_stream().await {
                    if tx.send((conn_id, stream)).is_err() {
                        break;
                    }
                }
            });
        }
        waiting = still;
        while let Ok((conn_id, stream)) = game_rx.try_recv() {
            let Some(sid) = sessions.by_conn.get(&conn_id).copied() else {
                continue;
            };
            if let Err(e) = attach(
                &mut hub,
                &mut sessions,
                sid,
                stream,
                a.players as u32,
                info.tick_rate,
                &mut next,
            ) {
                eprintln!("client of connection {conn_id}: {e}");
            }
        }
        sessions.read_controls(&mut hub);
        // As `Hub::serve_frame`, with every message the host's senders
        // didn't queue written to stderr (it lands in the report).
        for p in hub.poll().map_err(|e| format!("poll: {e}"))? {
            if !matches!(p.result, Ok(Accepted::Classified(Classified::Queued(_)))) {
                eprintln!(
                    "client {} {:?} 0x{:02X}: {:?}",
                    p.client, p.lane, p.id, p.result
                );
            }
        }
        hub.frame().map_err(|e| format!("frame: {e}"))?;
        hub.deliver_all();
        for (client, e) in hub.take_dropped() {
            eprintln!("client {client} dropped: {e}");
        }
        sessions.expire(&hub);
        std::thread::sleep(FRAME_PAUSE);
    }
    Ok(())
}

/// A game stream's `Attach`: the first game enters with a ticket (the
/// stand-in has no lobby, so it takes any); a held game comes back with
/// its resume token.
fn attach(
    hub: &mut SynthHub,
    sessions: &mut Sessions,
    sid: u64,
    mut stream: FramedStream,
    max_players: u32,
    tick_rate: u32,
    next: &mut ClientId,
) -> Result<(), String> {
    let body = match stream.recv_timeout(SYNTH_TIMING.timeout) {
        Ok(Some(b)) => b,
        Ok(None) => {
            stream.close();
            return Err("attach: no frame in time".into());
        }
        Err(e) => {
            stream.close();
            return Err(format!("attach: {e}"));
        }
    };
    let refuse = |stream: &mut FramedStream, code: ErrorCode| {
        let _ = stream.send(Refused::new(code).body());
        stream.close();
    };
    let attach = match Attach::parse(&body) {
        Ok(a) => a,
        Err(_) => {
            refuse(&mut stream, ErrorCode::MALFORMED);
            return Ok(());
        }
    };
    let in_game = sessions.map.get(&sid).is_some_and(|s| s.game.is_some());
    match attach {
        Attach::Ticket(_) => {
            if in_game {
                refuse(&mut stream, ErrorCode::ALREADY_IN_GAME);
                return Ok(());
            }
            if hub.clients().count() + hub.parked().count() >= MAX_CONNECTIONS {
                refuse(&mut stream, ErrorCode::SERVER_FULL);
                return Ok(());
            }
            let client = *next;
            *next += 1;
            let token = ResumeToken::random();
            let welcome = Welcome {
                client,
                tick_rate,
                max_players,
                resume: token,
            };
            stream
                .send(welcome.body())
                .map_err(|e| format!("attach: {e}"))?;
            hub.connect_with_token(client, Box::new(PacketStream::server(stream)), token);
            if let Some(s) = sessions.map.get_mut(&sid) {
                s.client = Some(client);
                s.game = Some(token);
            }
        }
        Attach::Resume(token) => {
            let Some(client) = hub.resumable(&token) else {
                refuse(&mut stream, ErrorCode::UNKNOWN_RESUME);
                return Ok(());
            };
            let welcome = Welcome {
                client,
                tick_rate,
                max_players,
                resume: token,
            };
            stream
                .send(welcome.body())
                .map_err(|e| format!("attach: {e}"))?;
            if hub
                .resume(&token, Box::new(PacketStream::server(stream)))
                .is_none()
            {
                return Err("attach: the resume did not find its slot".into());
            }
            if let Some(s) = sessions.map.get_mut(&sid) {
                s.client = Some(client);
                s.game = Some(token);
            }
        }
    }
    Ok(())
}
