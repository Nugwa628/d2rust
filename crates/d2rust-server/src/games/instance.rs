//! One game on its thread: the built game behind a hub, the seats, and
//! the frame loop's bookkeeping (leaves, player counts, saves, close).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use d2_formats::d2s::appearance::AppearanceTables;
use d2_formats::d2s::{self, D2s};
use d2_net::msg::ErrorCode;
use d2_net::wire::{Refused, Welcome};
use d2_net::{Packet, ResumeToken, Transport};
use d2_proto::dev::DevCommand;
use d2_server::adapters::session_flow::SessionFlow;
use d2_server::adapters::ProtoSizes;
use d2_server::dev::{DevCatalog, DevServer};
use d2_server::game::{character_save, loader, GameBuilder, GameData, Sim};
use d2_server::game_thread::Flow;
use d2_server::host::{Host, SystemClock};
use d2_server::hub::Hub;
use d2_server::seams::{ClientId, Tick};
use tokio::sync::mpsc;

use super::seat::{self, JoinSnapshot, SeatStream, Seats, ServerSession, ServerStore, Taken};
use super::{AttachedStream, GameEvent, GameSpec, GamesConfig, Seat};
use crate::log::ServerLog;

/// A save that takes longer than this is logged: it stalls the game's tick.
const SLOW_SAVE: Duration = Duration::from_millis(20);

/// A frame slower than this missed its tick.
const TICK: Duration = Duration::from_micros(16_667);

/// How long a seated player may take to send its join.
const JOIN_WAIT: Duration = Duration::from_secs(30);

type GameHub = Hub<Sim, ProtoSizes, ServerSession, SystemClock, Box<dyn Transport + Send>>;

/// What a game thread is built from.
pub(super) struct Parts {
    pub spec: GameSpec,
    pub cfg: GamesConfig,
    pub data: GameData,
    pub appearance: Option<Arc<AppearanceTables>>,
    pub log: Arc<ServerLog>,
    pub events: mpsc::Sender<GameEvent>,
    pub players: Arc<AtomicU8>,
    pub seated: Arc<Mutex<BTreeSet<String>>>,
    pub overruns: Arc<AtomicU64>,
    pub shutdown: Arc<AtomicBool>,
    pub closed: Arc<AtomicBool>,
    /// The dev command names, when the server runs `dev = test`.
    pub dev: Option<Arc<DevCatalog>>,
}

/// A running game. Lives on its game thread only.
pub(crate) struct Instance {
    pub(super) hub: GameHub,
    seats: Rc<RefCell<Seats>>,
    spec: GameSpec,
    cfg: GamesConfig,
    data: GameData,
    log: Arc<ServerLog>,
    events: mpsc::Sender<GameEvent>,
    players: Arc<AtomicU8>,
    players_sent: u8,
    seated_names: Arc<Mutex<BTreeSet<String>>>,
    overruns: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    next_client: ClientId,
    last_save: Instant,
    /// A seat was taken: from then on, no seats means the game is over.
    seated: bool,
    /// The dev command handlers (`dev = test` only).
    dev: Option<DevServer>,
    /// Test-only: the next resume panics after it resolves its session,
    /// to check that a resume panic drops only that session.
    #[cfg(test)]
    pub(super) panic_resume: bool,
}

impl Instance {
    /// Builds the game of `parts.spec` (on the game thread).
    pub(super) fn build(parts: Parts) -> Result<Instance, String> {
        let Parts {
            spec,
            cfg,
            data,
            appearance,
            log,
            events,
            players,
            seated,
            overruns,
            shutdown,
            closed,
            dev,
        } = parts;
        // The game's difficulty comes from its builder's character; the
        // players' characters come from the seats (the loader below).
        let character = d2_server::game::Character::New.with_difficulty(spec.difficulty);
        let mut sim = GameBuilder::new(data.clone())
            .seed(d2_server::game::clock_map_seed())
            .character(character)
            .build()
            .map_err(|e| e.to_string())?;
        if spec.hardcore {
            sim.events.action.hooks().x.hardcore = true;
        }
        let seats = Rc::new(RefCell::new(Seats::default()));
        sim.set_session(
            SessionFlow::new(loader(seat::characters(seats.clone(), spec.clone())))
                .with_max_players(usize::from(spec.max_players)),
        );
        let GameData::Live(live) = &data;
        sim.set_storage(Box::new(ServerStore {
            seats: seats.clone(),
            tables: Arc::new(live.save.clone()),
            appearance,
            events: events.clone(),
            game: spec.id,
        }));
        let host = Host::new(sim, ProtoSizes, ServerSession, SystemClock::default());
        let mut hub: GameHub = Hub::new(host);
        hub.set_grace(cfg.grace.as_millis().min(u128::from(u32::MAX)) as u32);
        Ok(Instance {
            hub,
            seats,
            spec,
            cfg,
            data,
            log,
            events,
            players,
            players_sent: 0,
            seated_names: seated,
            overruns,
            shutdown,
            closed,
            next_client: 0,
            last_save: Instant::now(),
            seated: false,
            dev: dev.map(|c| DevServer::new((*c).clone())),
            #[cfg(test)]
            panic_resume: false,
        })
    }

    /// Test-only: makes the next resume panic after it resolves its
    /// session (the panic guard's regression check).
    #[cfg(test)]
    pub(super) fn panic_next_resume(&mut self) {
        self.panic_resume = true;
    }

    fn line(&self, msg: &str) {
        self.log.line(&format!("game {}: {msg}", self.spec.id));
    }

    fn send(&self, ev: GameEvent) {
        // The lobby may be gone (server shutdown): nothing to tell then.
        let _ = self.events.blocking_send(ev);
    }

    /// Takes a seat: checks it, loads its character, answers the attach and
    /// connects the stream. A panic here closes the game.
    pub(super) fn admit(
        &mut self,
        seat: Seat,
        stream: Box<dyn AttachedStream>,
    ) -> Result<(), ErrorCode> {
        match catch_unwind(AssertUnwindSafe(|| self.admit_inner(seat, stream))) {
            Ok(r) => r,
            Err(_) => {
                self.line("panicked while admitting a player; closing");
                self.close_after_failure();
                Err(ErrorCode::GAME_CLOSED)
            }
        }
    }

    fn admit_inner(
        &mut self,
        seat: Seat,
        mut stream: Box<dyn AttachedStream>,
    ) -> Result<(), ErrorCode> {
        let checked = self.check_seat(&seat).and_then(|()| self.load_base(&seat));
        let base = match checked {
            Ok(base) => base,
            Err(code) => {
                let _ = stream.send_body(Refused::new(code).body());
                return Err(code);
            }
        };
        let client = self.next_client;
        self.next_client += 1;
        let token = ResumeToken::random();
        let welcome = Welcome {
            client,
            tick_rate: self.hub.host.game.tick_rate().hz(),
            max_players: u32::from(self.spec.max_players),
            resume: token,
        };
        stream.send_body(welcome.body()).map_err(|e| {
            self.line(&format!(
                "{}: answering the attach failed: {e}",
                seat.character
            ));
            ErrorCode::OTHER
        })?;
        let snapshot = Arc::new(Mutex::new(JoinSnapshot::default()));
        let filtered = SeatStream::capturing(stream, &seat, &self.spec, snapshot.clone());
        self.hub
            .connect_with_token(client, Box::new(filtered), token);
        self.line(&format!(
            "{} ({}) joins as client {client}{}",
            seat.character,
            seat.account,
            if seat.new.is_some() { ", new" } else { "" }
        ));
        self.seats.borrow_mut().insert(client, seat, base, snapshot);
        self.seated = true;
        self.update_players();
        Ok(())
    }

    fn check_seat(&self, seat: &Seat) -> Result<(), ErrorCode> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(ErrorCode::GAME_CLOSED);
        }
        match self
            .seats
            .borrow()
            .taken(seat, usize::from(self.spec.max_players))
        {
            Taken::Free => Ok(()),
            Taken::Full => Err(ErrorCode::GAME_FULL),
            Taken::Character => Err(ErrorCode::CHAR_IN_GAME),
        }
    }

    /// Takes `character` out of the game: saved, then gone as on a loss.
    pub(super) fn kick(&mut self, character: &str) -> bool {
        let Some(client) = self.seats.borrow().client_of(character) else {
            return false;
        };
        drop(self.hub.disconnect(client));
        self.note_leavers();
        self.update_players();
        true
    }

    /// Runs one dev command for `character` now, between two frames: the
    /// text of a success, or why it failed (contracts 10.1).
    pub(super) fn dev_command(
        &mut self,
        character: &str,
        cmd: &DevCommand,
    ) -> Result<String, String> {
        let Some(dev) = self.dev.as_mut() else {
            return Err("disabled (the server doesn't run dev = test)".into());
        };
        let client =
            self.seats.borrow().client_of(character).ok_or_else(|| {
                format!("no character named {character:?} in game {}", self.spec.id)
            })?;
        let player = self
            .hub
            .host
            .game
            .player_of(client)
            .ok_or_else(|| format!("{character} hasn't joined the game yet"))?;
        let r = dev.apply(&mut self.hub.host.game, Some(player), cmd);
        if r.ok {
            Ok(r.text)
        } else {
            Err(r.text)
        }
    }

    /// The save the seat's character is laid over: its file, read with the
    /// game's tables, or a new character's stub.
    fn load_base(&self, seat: &Seat) -> Result<D2s, ErrorCode> {
        if let Some(new) = &seat.new {
            let mut base =
                character_save::fresh(new.class, seat.character.as_bytes(), self.spec.difficulty);
            if new.hardcore {
                base.header.status |= d2s::status::HARDCORE;
            }
            // The progression set before the first game (--set-progression)
            // is part of the character from its first record: the game never
            // lowers it, so its saves keep it.
            base.header.status |= u16::from(new.progression & 0x1F) << 8;
            return Ok(base);
        }
        let file = std::fs::read(&seat.file).map_err(|e| {
            self.line(&format!(
                "{}: reading {}: {e}",
                seat.character,
                seat.file.display()
            ));
            if e.kind() == std::io::ErrorKind::NotFound {
                ErrorCode::UNKNOWN_CHAR
            } else {
                ErrorCode::OTHER
            }
        })?;
        let record = crate::characters::decode(&seat.character, &file).map_err(|e| {
            self.line(&e.to_string());
            ErrorCode::OTHER
        })?;
        let GameData::Live(live) = &self.data;
        live.read_save(&record, self.spec.difficulty).map_err(|e| {
            self.line(&format!(
                "{}: the character doesn't load: {e}",
                seat.character
            ));
            ErrorCode::OTHER
        })
    }

    /// Gives a held slot back to a returning stream as a re-entry (contracts
    /// 3.7): the same client, the player unit, its position and the game
    /// state stay, a new resume token is issued, any open window is closed,
    /// the parked backlog is dropped, and the client is sent the game's
    /// state again.
    ///
    /// A panic drops only this session (its stream and slot); the game and
    /// the other players keep running. The resume itself changes nothing
    /// the game trusts, so unlike a frame panic it does not close the game.
    pub(super) fn resume(
        &mut self,
        token: ResumeToken,
        stream: Box<dyn AttachedStream>,
    ) -> Result<(), ErrorCode> {
        let client = self.hub.resumable(&token);
        match catch_unwind(AssertUnwindSafe(|| self.resume_inner(token, stream))) {
            Ok(r) => r,
            Err(_) => {
                self.line("panicked in a resume; dropping that session");
                if let Some(client) = client {
                    self.drop_session(client);
                }
                Err(ErrorCode::OTHER)
            }
        }
    }

    /// Takes a client out of the game without closing it: the leave saves
    /// the character and the others keep playing.
    fn drop_session(&mut self, client: ClientId) {
        drop(self.hub.disconnect(client));
        self.note_leavers();
        self.update_players();
    }

    fn resume_inner(
        &mut self,
        token: ResumeToken,
        mut stream: Box<dyn AttachedStream>,
    ) -> Result<(), ErrorCode> {
        let Some(client) = self.hub.resumable(&token) else {
            let _ = stream.send_body(Refused::new(ErrorCode::UNKNOWN_RESUME).body());
            return Err(ErrorCode::UNKNOWN_RESUME);
        };
        let (seat, snapshot) = match self
            .seats
            .borrow()
            .get(client)
            .map(|s| (s.seat.clone(), s.snapshot.clone()))
        {
            Some(x) => x,
            None => {
                let _ = stream.send_body(Refused::new(ErrorCode::UNKNOWN_RESUME).body());
                return Err(ErrorCode::UNKNOWN_RESUME);
            }
        };
        #[cfg(test)]
        if std::mem::take(&mut self.panic_resume) {
            panic!("a resume fault (test)");
        }
        let new_token = ResumeToken::random();
        let welcome = Welcome {
            client,
            tick_rate: self.hub.host.game.tick_rate().hz(),
            max_players: u32::from(self.spec.max_players),
            resume: new_token,
        };
        if stream.send_body(welcome.body()).is_err() {
            return Err(ErrorCode::OTHER);
        }
        // A window the player left open must not survive the loss, and its
        // close must be told to the other player of a trade (contracts v12
        // 3.7). The close is done in the game before the replay; the
        // messages the resuming client expects follow the replay so they
        // end any window the replay opened.
        let closes = self.hub.host.game.close_client_interactions(client);
        // The client cleared its world: replay everything it was sent since
        // it joined, in order (the server-side journal, complete: the hub
        // backlog missed what a dying connection swallowed). That rebuilds
        // the character state (items, stats, quests, skills, waypoints,
        // mercenary, party) and the world as they are now.
        {
            let snapshot = snapshot.lock().unwrap_or_else(|e| e.into_inner());
            for m in snapshot.messages() {
                if stream.send(Packet::Deliver(m.clone())).is_err() {
                    return Err(ErrorCode::OTHER);
                }
            }
        }
        for m in closes {
            if stream.send(Packet::Deliver(m)).is_err() {
                return Err(ErrorCode::OTHER);
            }
        }
        let filtered = SeatStream::new(stream, &seat, &self.spec);
        match self.hub.reattach(&token, new_token, Box::new(filtered)) {
            Some(_) => {
                // What the journal cannot have: a character created in this
                // game never got the loaded character's 0x94, so its live
                // base skill levels go out now (contracts v12 3.7).
                self.hub.host.game.reenter_character_state(client);
                self.line(&format!(
                    "{} is back (client {client}), re-entered",
                    seat.character
                ));
                Ok(())
            }
            None => Err(ErrorCode::UNKNOWN_RESUME),
        }
    }

    /// One loop turn: a hub frame, then the bookkeeping. A panic closes the
    /// game.
    pub(super) fn frame(&mut self) -> Flow {
        match catch_unwind(AssertUnwindSafe(|| self.frame_inner())) {
            Ok(flow) => flow,
            Err(_) => {
                self.line("panicked in a frame; closing (characters keep their last save)");
                self.close_after_failure();
                Flow::Stop
            }
        }
    }

    fn frame_inner(&mut self) -> Flow {
        if self.shutdown.load(Ordering::SeqCst) {
            self.close(ErrorCode::GAME_CLOSED, "the server is stopping");
            return Flow::Stop;
        }
        // The dev switches act before the tick, as the client's do: god
        // heals, free mana refills, no-cooldown clears the skill delay.
        if self.dev.is_some() {
            let clients: Vec<ClientId> = self.hub.clients().collect();
            for c in clients {
                if let Some(p) = self.hub.host.game.player_of(c) {
                    d2_server::dev::upkeep(&mut self.hub.host.game, p);
                }
            }
        }
        let t = Instant::now();
        let served = self.hub.serve_frame();
        if t.elapsed() > TICK {
            self.overruns.fetch_add(1, Ordering::Relaxed);
        }
        if let Err(e) = served {
            self.line(&format!("frame failed: {e}"));
            self.close(ErrorCode::GAME_CLOSED, "the game failed");
            return Flow::Stop;
        }
        // A player that left cleanly (C→S 0x69) ends its stream; the hub
        // parks it like a lost one. Its record is gone, so it can't come
        // back: free the slot now.
        let gone: Vec<ClientId> = self
            .hub
            .parked()
            .filter(|&c| self.hub.host.game.sim_client(c).is_none())
            .collect();
        for c in gone {
            self.hub.disconnect(c);
        }
        // A connected player whose record went (its join was refused, or it
        // left and kept the stream open), or that never joined in time, has
        // no place in the game any more.
        let mut out = Vec::new();
        for (&c, s) in self.seats.borrow_mut().iter_mut() {
            if self.hub.host.game.sim_client(c).is_some() {
                s.entered = true;
            } else if s.entered || s.admitted.elapsed() >= JOIN_WAIT {
                out.push(c);
            }
        }
        for c in out {
            if self.hub.clients().any(|x| x == c) {
                self.hub.disconnect(c);
            }
        }
        for (client, e) in self.hub.take_dropped() {
            self.line(&format!("client {client} dropped: {e}"));
        }
        self.note_leavers();
        for (client, fault) in std::mem::take(&mut self.hub.host.game.save_faults) {
            self.line(&format!("client {client}: save failed: {fault}"));
        }
        self.update_players();
        if self.last_save.elapsed() >= self.cfg.save_interval {
            self.save_all();
        }
        if self.seated && self.seats.borrow().is_empty() {
            self.close(ErrorCode::GAME_CLOSED, "the last player left");
            return Flow::Stop;
        }
        Flow::Go
    }

    /// Seats whose client the hub no longer holds (neither connected nor
    /// parked) have left: their characters were saved by the leave.
    fn note_leavers(&mut self) {
        let held: Vec<ClientId> = self.hub.clients().chain(self.hub.parked()).collect();
        let left = self.seats.borrow_mut().remove_missing(&held);
        for s in left {
            self.line(&format!("{} left", s.seat.character));
            self.send(GameEvent::Left {
                game: self.spec.id,
                account: s.seat.account,
                character: s.seat.character,
            });
        }
    }

    fn update_players(&mut self) {
        let names = self.seats.borrow().name_keys();
        *self.seated_names.lock().unwrap_or_else(|e| e.into_inner()) = names;
        let n = self.seats.borrow().len().min(usize::from(u8::MAX)) as u8;
        self.players.store(n, Ordering::SeqCst);
        if n != self.players_sent {
            self.players_sent = n;
            self.send(GameEvent::Players {
                game: self.spec.id,
                players: n,
            });
        }
    }

    /// Saves every player's character now (between frames).
    fn save_all(&mut self) {
        self.last_save = Instant::now();
        if self.seats.borrow().is_empty() {
            return;
        }
        let t = Instant::now();
        let results = self.hub.host.game.save_characters();
        let took = t.elapsed();
        for (client, r) in results {
            if let Err(e) = r {
                self.line(&format!("client {client}: save failed: {e}"));
            }
        }
        if took >= SLOW_SAVE {
            self.line(&format!("saving took {} ms", took.as_millis()));
        }
    }

    /// Ends the game: every player is saved and taken out (the game's own
    /// leave path), then [`GameEvent::Closed`].
    fn close(&mut self, reason: ErrorCode, why: &str) {
        let clients: Vec<ClientId> = self.hub.clients().chain(self.hub.parked()).collect();
        for c in clients {
            // The leave saves every character still in the game.
            drop(self.hub.disconnect(c));
        }
        self.note_leavers();
        self.update_players();
        self.line(&format!("closed: {why}"));
        self.send_closed(reason);
    }

    /// A failure: nothing more runs on this game's state, so no save; the
    /// characters keep their last one.
    fn close_after_failure(&mut self) {
        self.send_closed(ErrorCode::GAME_CLOSED);
    }

    fn send_closed(&mut self, reason: ErrorCode) {
        self.players.store(0, Ordering::SeqCst);
        self.seated_names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.send(GameEvent::Closed {
                game: self.spec.id,
                reason,
            });
        }
    }
}

impl Drop for Instance {
    /// A game thread that ends any other way (a panic in a job, the handle
    /// dropped) still tells the lobby.
    fn drop(&mut self) {
        self.send_closed(ErrorCode::GAME_CLOSED);
    }
}
