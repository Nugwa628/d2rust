//! Running games (contracts section 6): every game the lobby creates runs
//! on its own looped [`GameThread`] at the server tick, with a hub of the
//! players' game streams.
//!
//! - **Lifecycle.** The lobby calls [`Games::start`] on `CreateGame`. The
//!   game is built when its first seat is admitted ([`GameHandle::admit`]),
//!   at the spec's difficulty. It closes when it has no players and no held
//!   slots (a lost player's slot is held for the grace period), when it is
//!   never entered ([`GamesConfig::unused_close`]), on
//!   [`GameHandle::shutdown`], or when it fails. Every close ends with
//!   [`GameEvent::Closed`], once.
//! - **Seats.** A seat is the ticket's account and character. The game
//!   loads the character from its file (or makes a new one), and the
//!   player's 0x67 join must name that character: the game stream's seat
//!   filter ([`seat::SeatStream`]) refuses another name, and writes the
//!   game's own difficulty and kind into the join, so a client can't choose
//!   them. Character uploads (0x6C) never reach the game.
//! - **Saves.** On leave and on loss (the game's own leave path), every
//!   [`GamesConfig::save_interval`] between frames, and on close; also the
//!   game's own 8192-frame save. Each write goes to a temporary file that is
//!   flushed and renamed over the character's file, so a crash leaves the
//!   last good one. After each save the game sends [`GameEvent::Saved`].
//! - **Failures.** A panic in a frame or a job is caught: the game closes
//!   with [`ErrorCode::GAME_CLOSED`] and its characters keep their last
//!   save. The other games and the lobby play on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use d2_formats::d2s::appearance::AppearanceTables;
use d2_net::msg::account::CharSummary;
use d2_net::msg::ErrorCode;
use d2_net::wire::Refused;
use d2_net::{NetError, Transport};
use d2_proto::dev::DevCommand;
use d2_server::dev::DevCatalog;
use d2_server::game::GameData;
use d2_server::game_thread::GameThread;
use tokio::sync::mpsc;

use crate::accounts::{AccountId, NewChar};
use crate::config::ServerConfig;
use crate::log::ServerLog;

mod instance;
pub mod probe;
pub mod seat;
#[cfg(test)]
mod tests;

use instance::Instance;

/// What the lobby creates a game with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSpec {
    pub id: u32,
    pub name: String,
    /// 0 Normal, 1 Nightmare, 2 Hell.
    pub difficulty: u8,
    pub hardcore: bool,
    pub expansion: bool,
    /// The most players; 0 is the server's default.
    pub max_players: u8,
}

/// One player's place in a game: whose character, and where it is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub account: AccountId,
    pub character: String,
    /// The character's file (`Accounts::character_file`).
    pub file: PathBuf,
    /// Set for a character that has never been played: the game makes it.
    pub new: Option<NewChar>,
}

/// What a game tells the lobby.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameEvent {
    /// The number of players (in the game or holding a slot) changed.
    Players { game: u32, players: u8 },
    /// A character was saved; the summary is its file's new state.
    Saved {
        game: u32,
        account: AccountId,
        summary: CharSummary,
    },
    /// A player left the game (cleanly, lost past the grace period, or
    /// dropped), after its character was saved.
    Left {
        game: u32,
        account: AccountId,
        character: String,
    },
    /// The game is over: empty, never entered, failed or shut down.
    Closed { game: u32, reason: ErrorCode },
}

/// A game stream the game can take a seat on: a transport of game packets
/// that can also carry the game's answer to the attach (a raw frame body)
/// before the first packet.
pub trait AttachedStream: Transport + Send + 'static {
    /// Sends one raw frame body (the game's `Welcome`, or `Refused`).
    fn send_body(&mut self, body: Vec<u8>) -> Result<(), NetError>;
}

/// The server's end of a QUIC game stream.
impl AttachedStream for d2_quic::PacketStream {
    fn send_body(&mut self, body: Vec<u8>) -> Result<(), NetError> {
        self.stream_mut().send(body)
    }
}

/// The settings every game runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamesConfig {
    /// The most games at once.
    pub max_games: usize,
    /// The most players of a game whose spec says 0.
    pub max_players: u8,
    /// How long a lost player's slot is held.
    pub grace: Duration,
    /// The pause between two frames of a game's loop.
    pub frame_pause: Duration,
    /// How often the players' characters are saved.
    pub save_interval: Duration,
    /// How long a created game waits for its first player.
    pub unused_close: Duration,
    /// `dev = test`: the games take dev commands on the server's standard
    /// input (contracts 10.1).
    pub dev_test: bool,
}

impl GamesConfig {
    /// The games' part of the server config.
    pub fn from_server(cfg: &ServerConfig) -> Self {
        Self {
            max_games: cfg.max_games,
            max_players: cfg.max_players.min(usize::from(u8::MAX)) as u8,
            grace: cfg.grace,
            frame_pause: cfg.frame_pause,
            save_interval: cfg.save_interval,
            unused_close: Duration::from_secs(60),
            dev_test: cfg.dev_test,
        }
    }
}

/// Every running game.
#[derive(Clone)]
pub struct Games {
    inner: Arc<GamesInner>,
}

struct GamesInner {
    shared: Arc<Shared>,
    games: Mutex<BTreeMap<u32, GameHandle>>,
}

/// What every game is built with.
struct Shared {
    cfg: GamesConfig,
    data: GameData,
    /// The save's appearance tables (the character-select picture),
    /// made once from the game data.
    appearance: Option<Arc<AppearanceTables>>,
    log: Arc<ServerLog>,
    /// Frames of any game that took longer than a tick (status.json).
    overruns: Arc<AtomicU64>,
    /// The dev command names, when the server runs `dev = test`.
    dev_catalog: Option<Arc<DevCatalog>>,
    /// Where the games' events go ([`Games::set_events`]).
    events: OnceLock<mpsc::Sender<GameEvent>>,
}

impl Games {
    /// The games of a server on `data`.
    pub fn new(cfg: GamesConfig, data: GameData, log: Arc<ServerLog>) -> Games {
        let GameData::Live(live) = &data;
        let appearance = d2_server::game::character_save::appearance_tables(&live.tables.fixed)
            .map_err(|e| {
                log.line(&format!(
                    "games: no appearance tables ({e}); saves keep the loaded look"
                ))
            })
            .ok()
            .map(Arc::new);
        let dev_catalog = cfg.dev_test.then(|| {
            let catalog = DevCatalog::load(live).unwrap_or_else(|e| {
                log.line(&format!(
                    "dev: no name catalog ({e}); commands by id still work"
                ));
                DevCatalog::default()
            });
            Arc::new(catalog)
        });
        Games {
            inner: Arc::new(GamesInner {
                shared: Arc::new(Shared {
                    cfg,
                    data,
                    appearance,
                    log,
                    overruns: Arc::new(AtomicU64::new(0)),
                    dev_catalog,
                    events: OnceLock::new(),
                }),
                games: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Sets where the games' events go: the server's event pump, which
    /// updates the sessions and hands each event on to the lobby. Set once.
    pub fn set_events(&self, tx: mpsc::Sender<GameEvent>) {
        let _ = self.inner.shared.events.set(tx);
    }

    /// The sender [`Games::set_events`] set: what the lobby passes to
    /// [`Games::start`].
    pub fn events(&self) -> Option<mpsc::Sender<GameEvent>> {
        self.inner.shared.events.get().cloned()
    }

    /// Opens a game for `spec`; it is built when its first seat is
    /// admitted. Refused when the server runs `max_games`, when the id is
    /// taken, or when the spec asks for a kind of game this server can't
    /// run.
    pub fn start(
        &self,
        spec: GameSpec,
        events: mpsc::Sender<GameEvent>,
    ) -> Result<GameHandle, ErrorCode> {
        let cfg = &self.inner.shared.cfg;
        if spec.difficulty > 2 {
            return Err(ErrorCode::OTHER);
        }
        // The game is always the expansion game.
        if !spec.expansion {
            return Err(ErrorCode::NOT_SUPPORTED);
        }
        let mut games = self.lock();
        games.retain(|_, g| !g.is_closed());
        if games.len() >= cfg.max_games {
            return Err(ErrorCode::TOO_MANY_GAMES);
        }
        if games.contains_key(&spec.id) {
            return Err(ErrorCode::GAME_NAME_TAKEN);
        }
        let max_players = match spec.max_players {
            0 => cfg.max_players,
            n => n.min(cfg.max_players),
        };
        let handle = GameHandle {
            inner: Arc::new(HandleInner {
                spec: GameSpec {
                    max_players,
                    ..spec
                },
                shared: self.inner.shared.clone(),
                events,
                created: Instant::now(),
                players: Arc::new(AtomicU8::new(0)),
                shutdown: Arc::new(AtomicBool::new(false)),
                closed: Arc::new(AtomicBool::new(false)),
                seated: Arc::new(Mutex::new(BTreeSet::new())),
                thread: Mutex::new(None),
            }),
        };
        games.insert(handle.id(), handle.clone());
        self.inner.shared.log.line(&format!(
            "game {}: opened \"{}\", difficulty {}, {} players at most",
            handle.id(),
            handle.inner.spec.name,
            handle.inner.spec.difficulty,
            max_players
        ));
        Ok(handle)
    }

    /// The open game with this id.
    pub fn get(&self, id: u32) -> Option<GameHandle> {
        self.lock().get(&id).filter(|g| !g.is_closed()).cloned()
    }

    /// The open games, in id order.
    pub fn list(&self) -> Vec<GameHandle> {
        self.lock()
            .values()
            .filter(|g| !g.is_closed())
            .cloned()
            .collect()
    }

    /// Closes the games that were never entered in time, and forgets the
    /// closed ones. The server calls it every few seconds.
    pub fn reap(&self) {
        let unused = self.inner.shared.cfg.unused_close;
        let games: Vec<GameHandle> = self.lock().values().cloned().collect();
        for g in games {
            g.close_if_unused(unused);
        }
        self.lock().retain(|_, g| !g.is_closed());
    }

    /// Whether a character (its lowercase name) is seated in an open game.
    pub fn character_in_game(&self, name_key: &str) -> bool {
        self.list().iter().any(|g| g.has_character(name_key))
    }

    /// Runs one dev command for `character` in its game (contracts 10.1):
    /// the command runs on the game's thread, between two frames. Refused
    /// when the server doesn't run `dev = test`.
    pub fn dev_command(&self, character: &str, cmd: &DevCommand) -> Result<String, String> {
        if !self.inner.shared.cfg.dev_test {
            return Err("disabled (the server doesn't run dev = test)".into());
        }
        let key = character.to_ascii_lowercase();
        let game = self
            .list()
            .into_iter()
            .find(|g| g.has_character(&key))
            .ok_or_else(|| format!("no character named {character:?} in a game"))?;
        game.dev_command(character, cmd)
    }

    /// Players in every open game.
    pub fn players(&self) -> usize {
        self.list().iter().map(|g| usize::from(g.players())).sum()
    }

    /// Frames, over every game so far, that took longer than one tick.
    pub fn tick_overruns(&self) -> u64 {
        self.inner.shared.overruns.load(Ordering::Relaxed)
    }

    /// Server shutdown: every game saves its players and closes. Waits up
    /// to `wait` for them all.
    pub fn shutdown_all(&self, wait: Duration) {
        let games: Vec<GameHandle> = self.lock().values().cloned().collect();
        for g in &games {
            g.shutdown();
        }
        let until = Instant::now() + wait;
        while games.iter().any(|g| !g.is_closed()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        // Join the game threads: their last saves are done when this returns.
        for g in &games {
            g.inner
                .thread
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u32, GameHandle>> {
        self.inner.games.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One open game. Cloning shares it.
#[derive(Clone)]
pub struct GameHandle {
    inner: Arc<HandleInner>,
}

struct HandleInner {
    spec: GameSpec,
    shared: Arc<Shared>,
    events: mpsc::Sender<GameEvent>,
    created: Instant,
    /// Kept by the game thread after every frame.
    players: Arc<AtomicU8>,
    /// Asks the game thread to save everyone and close.
    shutdown: Arc<AtomicBool>,
    /// Set once [`GameEvent::Closed`] went out.
    closed: Arc<AtomicBool>,
    /// The seated characters' lowercase names, kept by the game thread.
    seated: Arc<Mutex<BTreeSet<String>>>,
    /// The game's thread, from its first admitted seat on.
    thread: Mutex<Option<GameThread<Instance>>>,
}

impl GameHandle {
    pub fn id(&self) -> u32 {
        self.inner.spec.id
    }

    /// The game's spec, with its player limit resolved.
    pub fn spec(&self) -> &GameSpec {
        &self.inner.spec
    }

    /// Players in the game or holding a slot.
    pub fn players(&self) -> u8 {
        self.inner.players.load(Ordering::SeqCst)
    }

    /// Whether the game is over.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Whether a character (its lowercase name) is seated in this game.
    pub fn has_character(&self, name_key: &str) -> bool {
        self.inner
            .seated
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(name_key)
    }

    /// Takes a character out of the game (its account logged in elsewhere):
    /// it is saved and leaves as on a lost connection. False when it isn't
    /// seated here.
    pub fn kick(&self, character: &str) -> bool {
        let name = character.to_owned();
        let thread = self.inner.thread.lock().unwrap_or_else(|e| e.into_inner());
        match thread.as_ref() {
            Some(t) => t.with(move |inst| inst.kick(&name)).unwrap_or(false),
            None => false,
        }
    }

    /// Runs one dev command for `character` on the game's thread
    /// (contracts 10.1): the text of a success, or why it failed.
    pub fn dev_command(&self, character: &str, cmd: &DevCommand) -> Result<String, String> {
        let (name, cmd) = (character.to_owned(), cmd.clone());
        self.with_thread(move |inst| inst.dev_command(&name, &cmd))
            .map_err(|_| format!("game {} is closed", self.id()))?
    }

    /// Hands an attached game stream and its seat to the game: the game
    /// loads the seat's character (or makes the new one), answers the
    /// attach with its `Welcome` on `stream` and takes the player's packets
    /// from then on. The first seat builds the game, which takes a moment:
    /// call it off the network's async threads.
    /// A refused seat is told why on `stream` before this returns.
    pub fn admit(&self, seat: Seat, stream: impl AttachedStream) -> Result<(), ErrorCode> {
        let mut stream: Box<dyn AttachedStream> = Box::new(stream);
        if self.is_closed() || self.inner.shutdown.load(Ordering::SeqCst) {
            let _ = stream.send_body(Refused::new(ErrorCode::GAME_CLOSED).body());
            return Err(ErrorCode::GAME_CLOSED);
        }
        self.with_thread(move |inst| inst.admit(seat, stream))?
    }

    /// Hands a game stream that attached with a game resume token back to
    /// the slot held for it. A refused stream is told why.
    pub fn resume(
        &self,
        token: d2_net::ResumeToken,
        stream: impl AttachedStream,
    ) -> Result<(), ErrorCode> {
        let mut stream: Box<dyn AttachedStream> = Box::new(stream);
        let thread = self.inner.thread.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = thread.as_ref() else {
            let _ = stream.send_body(Refused::new(ErrorCode::UNKNOWN_RESUME).body());
            return Err(ErrorCode::UNKNOWN_RESUME);
        };
        t.with(move |inst| inst.resume(token, stream))
            .map_err(|_| ErrorCode::GAME_CLOSED)?
    }

    /// Saves every player and closes the game ([`GameEvent::Closed`]).
    pub fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        // A game that never started has no thread to see the flag.
        if self
            .inner
            .thread
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            self.close_now(ErrorCode::GAME_CLOSED);
        }
    }

    /// Runs `f` on the game thread, starting the thread first if this is
    /// the game's first seat.
    fn with_thread<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Instance) -> R + Send + 'static,
    ) -> Result<R, ErrorCode> {
        let mut thread = self.inner.thread.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_closed() || self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(ErrorCode::GAME_CLOSED);
        }
        if thread.is_none() {
            *thread = Some(self.spawn()?);
        }
        let t = thread.as_ref().expect("started above");
        t.with(f).map_err(|_| ErrorCode::GAME_CLOSED)
    }

    /// Builds the game on its own thread.
    fn spawn(&self) -> Result<GameThread<Instance>, ErrorCode> {
        let games = &self.inner.shared;
        let started = Instant::now();
        let parts = instance::Parts {
            spec: self.inner.spec.clone(),
            cfg: games.cfg.clone(),
            data: games.data.clone(),
            appearance: games.appearance.clone(),
            log: games.log.clone(),
            events: self.inner.events.clone(),
            players: self.inner.players.clone(),
            seated: self.inner.seated.clone(),
            overruns: games.overruns.clone(),
            shutdown: self.inner.shutdown.clone(),
            closed: self.inner.closed.clone(),
            dev: games.dev_catalog.clone(),
        };
        let pause = games.cfg.frame_pause;
        let name = format!("game-{}", self.id());
        let built =
            GameThread::spawn_looped(&name, move || Instance::build(parts), pause, |g| g.frame());
        match built {
            Ok(t) => {
                games.log.line(&format!(
                    "game {}: built in {} ms",
                    self.id(),
                    started.elapsed().as_millis()
                ));
                Ok(t)
            }
            Err(e) => {
                games
                    .log
                    .line(&format!("game {}: building failed: {e}", self.id()));
                self.close_now(ErrorCode::GAME_CLOSED);
                Err(ErrorCode::GAME_CLOSED)
            }
        }
    }

    /// Closes a game that has no thread: no player was ever admitted.
    fn close_if_unused(&self, after: Duration) {
        let thread = self.inner.thread.lock().unwrap_or_else(|e| e.into_inner());
        if thread.is_none() && self.inner.created.elapsed() >= after {
            drop(thread);
            self.inner
                .shared
                .log
                .line(&format!("game {}: nobody came; closing", self.id()));
            self.close_now(ErrorCode::GAME_CLOSED);
        }
    }

    /// Sends [`GameEvent::Closed`] for a game without a running thread
    /// (once).
    fn close_now(&self, reason: ErrorCode) {
        if !self.inner.closed.swap(true, Ordering::SeqCst) {
            let _ = self.inner.events.try_send(GameEvent::Closed {
                game: self.id(),
                reason,
            });
        }
    }
}

impl std::fmt::Debug for GameHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameHandle")
            .field("spec", &self.inner.spec)
            .field("players", &self.players())
            .field("closed", &self.is_closed())
            .finish()
    }
}
