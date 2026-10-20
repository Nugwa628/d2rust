//! The in-process server host of single player and its schedule: one client
//! frame drains the server queues, runs at most one tick when the next one
//! is due at the game's tick rate, and flushes every client after a tick.
//! Wall-clock time comes from an injected [`Clock`] and never reaches the
//! sim. The host extends it to a 1 ms base clock ([`BaseClock`]) that never
//! wraps: ticks are scheduled on it ("next tick at ms N"), and backend tasks
//! read it from [`Host::base_ms`].
//!
//! [`Host`] ties [`crate::transport`] (C→S queues), [`crate::dispatch`]
//! (game messages), [`crate::buffers`] (S→C buffers and receive lists) and
//! the sim behind the [`crate::seams`] traits. `d2-client`'s bridge and the
//! scenario tools drive it frame by frame.

use std::collections::BTreeMap;
use std::time::Instant;

use d2_proto::envelope::{self, Wire};
use d2_proto::schema::Size;

use crate::buffers::{ClientBuffers, Inbox, QueueError, Tapped};
use crate::dispatch::{dispatch, process_game_message, ClientRecord, DispatchError, Outcome};
use crate::packets::{PacketEvent, PacketObserver};
use crate::seams::{
    ClientId, Clock, Intents, MessageSink, MessageSizes, PlayerLookup, ResultCode, SessionHandler,
    Tick,
};
use crate::transport::{Classified, DuplicateFilter, Queue, SendError, ServerQueues};
use d2_sim::tick::TickRate;

/// The real clock: milliseconds since the clock was made.
#[derive(Debug)]
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now_ms(&mut self) -> u32 {
        // Truncated to 32 bits: the clock wraps after about 49.7 days.
        self.0.elapsed().as_millis() as u32
    }
}

/// The host's 1 ms base clock: the injected [`Clock`]'s 32-bit millisecond
/// reading extended to a 64-bit count that never wraps.
///
/// It starts at the first reading and then moves forward by the difference
/// between readings, across the 32-bit wrap (about 49.7 days). A reading
/// behind the latest one (the clock stepped back) holds the base clock until
/// a reading passes the latest one again; a forward step of 2^31 ms or more
/// in one reading counts as a step back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaseClock {
    /// The latest reading the clock moved to; `None` before the first.
    raw: Option<u32>,
    /// Milliseconds on the base clock.
    ms: u64,
}

impl BaseClock {
    /// Moves the clock to the reading `raw` and returns its time in ms.
    pub fn advance(&mut self, raw: u32) -> u64 {
        match self.raw {
            None => {
                self.raw = Some(raw);
                self.ms = u64::from(raw);
            }
            Some(latest) => {
                let step = raw.wrapping_sub(latest);
                if (step as i32) > 0 {
                    self.raw = Some(raw);
                    self.ms += u64::from(step);
                }
            }
        }
        self.ms
    }

    /// The time in ms at the latest [`Self::advance`] (0 before the first).
    pub fn ms(&self) -> u64 {
        self.ms
    }
}

/// Tick driver: decides each frame whether a tick is due, at the game's
/// [`TickRate`], on the host's 1 ms [`BaseClock`].
///
/// Tick *n* after the start is due at `⌊n · 1000 / hz⌋` ms, so the tick
/// lengths are whole milliseconds that add up to exactly one second per
/// `hz` ticks: 40 ms each at 25 Hz, 16, 17, 17 ms at 60 Hz. The schedule
/// does not drift, and a late tick does not move the ones after it. The
/// next tick's time is [`Self::next_tick_ms`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickDriver {
    /// Base-clock time (ms) of the last tick's schedule; 0 means "not
    /// started" (the test is `last == 0`, there is no separate flag): the
    /// next poll sets it to its `now`.
    pub last: u64,
    /// The game's tick rate.
    rate: TickRate,
    /// Position of the next tick in the one-second cycle of `hz` ticks
    /// (`0..hz`): its length is [`Self::step_ms`] of it.
    phase: u32,
}

impl Default for TickDriver {
    fn default() -> Self {
        Self::new(TickRate::BASE)
    }
}

impl TickDriver {
    /// A driver at `rate` that has not ticked yet.
    pub fn new(rate: TickRate) -> Self {
        Self::started(rate, 0)
    }

    /// A driver at `rate` whose last tick was at `last` (0: not started),
    /// at the start of its one-second cycle.
    pub fn started(rate: TickRate, last: u64) -> Self {
        Self {
            last,
            rate,
            phase: 0,
        }
    }

    /// The rate the driver schedules ticks at.
    pub fn rate(&self) -> TickRate {
        self.rate
    }

    /// The shortest tick length in ms, `⌊1000 / hz⌋` (40 at 25 Hz, 16 at
    /// 60 Hz): the flush throttle.
    pub fn tick_ms(&self) -> u32 {
        1000 / self.rate.hz()
    }

    /// Length in ms of the tick at `phase` of the one-second cycle: the
    /// gap between `⌊phase · 1000 / hz⌋` and `⌊(phase + 1) · 1000 / hz⌋`.
    fn step_ms(&self, phase: u32) -> u32 {
        let hz = self.rate.hz();
        (phase + 1) * 1000 / hz - phase * 1000 / hz
    }

    /// Base-clock time (ms) the next tick is due at; `None` before the
    /// driver started.
    pub fn next_tick_ms(&self) -> Option<u64> {
        (self.last != 0).then(|| self.last + u64::from(self.step_ms(self.phase)))
    }

    /// True when a tick is due at `now` (ms on the [`BaseClock`]), updating
    /// `last` to the tick's scheduled time. With `catch_up` the lag carried
    /// forward is at most one tick: when the next tick is already due as
    /// well, it runs on the next call and any older lag is dropped. A `now`
    /// before `last` runs no tick.
    pub fn poll(&mut self, now: u64, catch_up: bool) -> bool {
        let lag = catch_up.then(|| u64::from(self.step_ms((self.phase + 1) % self.rate.hz())));
        self.poll_with_lag(now, lag)
    }

    /// Polls with an optional maximum backlog in milliseconds. Display frames
    /// retain several ticks of debt; a long stall drops debt beyond this cap.
    pub fn poll_with_lag(&mut self, now: u64, max_lag_ms: Option<u64>) -> bool {
        // A clock of exactly 0 on first use leaves `last` at 0, so the next
        // call starts the schedule instead of ticking.
        if self.last == 0 {
            self.last = now;
        }
        let Some(elapsed) = now.checked_sub(self.last) else {
            return false;
        };
        let step = u64::from(self.step_ms(self.phase));
        if elapsed < step {
            return false;
        }
        self.phase = (self.phase + 1) % self.rate.hz();
        let mut excess = elapsed - step;
        if let Some(limit) = max_lag_ms {
            excess = excess.min(limit);
        }
        self.last = now - excess;
        true
    }
}

/// What the host did with one drained message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// Queue 0, passed to the session handler.
    System,
    /// Queue 1, run through the game message entry.
    Game(Outcome),
    /// Queue 2: runs only with host callbacks (realm/admin), which D2RUST
    /// does not have; dropped.
    AdminIgnored,
    /// Queue 1, a mod message on a mod game's wire, handed to the game's
    /// mod rules ([`Intents::mod_message`]); `true` when they took it.
    Mod(bool),
}

/// One drained message and its fate, in drain order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandledMessage {
    /// The sending client.
    pub client: ClientId,
    /// The message id (first byte).
    pub id: u8,
    /// The message size as queued.
    pub size: usize,
    /// What the host did with it.
    pub handled: Handled,
}

/// One client frame's server part.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameReport {
    /// The messages drained this frame, in drain order.
    pub messages: Vec<HandledMessage>,
    /// Whether a tick ran this frame.
    pub ticked: bool,
    /// Buffers handed to local delivery by the flush.
    pub flushed_buffers: usize,
    /// Bytes lost to a split that ended early (a size-rule failure ends the
    /// split of a buffer).
    pub discarded_bytes: usize,
}

/// A state that cannot occur in a consistent host.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// A game message could not be dispatched.
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    /// A buffer queue or local delivery failed.
    #[error(transparent)]
    Queue(#[from] QueueError),
}

/// The in-process server host of single player.
pub struct Host<G, S, H, C> {
    /// The sim side: intents in, ticks out.
    pub game: G,
    /// Diagnostic packet counts; excluded from game state and dispatch decisions.
    pub coverage: d2_sim::debug::coverage::Counters,
    /// The per-direction message size rules.
    pub sizes: S,
    /// Handler for system (queue 0) messages.
    pub session: H,
    /// The wall clock the schedule reads, through [`Self::base_ms`].
    pub clock: C,
    /// The 1 ms base clock the clock's readings extend.
    base: BaseClock,
    /// The three C→S server queues.
    pub queues: ServerQueues,
    /// The S→C buffers of every client.
    pub buffers: ClientBuffers,
    /// Decides when a tick runs.
    pub driver: TickDriver,
    /// `catch_up` argument of the driver (true in single player).
    pub catch_up: bool,
    /// Display-frame catch-up debt cap, in ms. `None` keeps the single-step
    /// harness policy; interactive play retains debt across slow frames.
    pub display_lag_ms: Option<u64>,
    records: BTreeMap<ClientId, ClientRecord>,
    inboxes: BTreeMap<ClientId, Inbox>,
    filters: BTreeMap<ClientId, DuplicateFilter>,
    /// The client sender's own clock (a millisecond tick count) when a
    /// check run replays it ([`Host::set_send_now`]);
    /// `None`: the host clock.
    send_now: Option<u32>,
    /// The client sender's clock per frame when a check run replays the
    /// one recorded with the same run's packets ([`Host::set_send_clock`]).
    send_clock: BTreeMap<i32, u32>,
    last_flush: Option<u32>,
    /// The packet recorder; `None` by
    /// default. It only reads: the game runs the same with or without it.
    packets: Option<Box<dyn PacketObserver + Send>>,
    /// Direct sends made before a recorder was installed (the server-start 0xAF),
    /// replayed to the recorder when it is.
    early_direct: Vec<(ClientId, Vec<u8>)>,
}

impl<G, S, H, C> Host<G, S, H, C>
where
    G: Intents + Tick,
    S: MessageSizes,
    H: SessionHandler,
    C: Clock,
{
    /// A host with no clients, the tick driver unstarted at the game's
    /// tick rate ([`Tick::tick_rate`]), catch-up on.
    pub fn new(game: G, sizes: S, session: H, clock: C) -> Self {
        let driver = TickDriver::new(game.tick_rate());
        Self {
            game,
            coverage: Default::default(),
            sizes,
            session,
            clock,
            base: BaseClock::default(),
            queues: ServerQueues::new(),
            buffers: ClientBuffers::new(),
            driver,
            catch_up: true,
            display_lag_ms: None,
            records: BTreeMap::new(),
            inboxes: BTreeMap::new(),
            filters: BTreeMap::new(),
            send_now: None,
            send_clock: BTreeMap::new(),
            last_flush: None,
            packets: None,
            early_direct: Vec::new(),
        }
    }

    /// Installs (or with `None` removes) the packet recorder: from now on
    /// every message the host moves is also reported to it, in order. The
    /// game is not changed.
    pub fn set_packet_observer(&mut self, observer: Option<Box<dyn PacketObserver + Send>>) {
        self.buffers.set_tap(observer.is_some());
        let had = self.packets.is_some();
        self.packets = observer;
        if !had && self.packets.is_some() {
            for (client, msg) in std::mem::take(&mut self.early_direct) {
                self.buffers.note(|| Tapped::Direct(client, msg.clone()));
            }
            self.note_tap();
        }
    }

    /// Reports one event to the recorder, if any.
    fn note(&mut self, ev: PacketEvent<'_>) {
        if self.coverage.enabled {
            if let PacketEvent::C2s { msg, .. } = ev {
                if let Some(id) = msg.first() {
                    self.coverage
                        .hit("server_packets_received", || u32::from(*id));
                }
            }
        }
        if let Some(p) = self.packets.as_mut() {
            p.packet(ev);
        }
    }

    /// Reports what the buffers' tap saw since the last call (queued
    /// messages, direct sends, flushed buffers), in order.
    fn note_tap(&mut self) {
        let Some(p) = self.packets.as_mut() else {
            return;
        };
        for t in self.buffers.take_tap() {
            p.packet(match &t {
                Tapped::Queued(client, msg) => PacketEvent::S2c {
                    client: *client,
                    msg,
                },
                Tapped::Direct(client, msg) => PacketEvent::Net {
                    client: *client,
                    msg,
                    direct: true,
                },
                Tapped::Flushed(client, msg) => PacketEvent::Net {
                    client: *client,
                    msg,
                    direct: false,
                },
            });
        }
    }

    /// Registers a client: its record, buffers, receive lists and sender.
    pub fn connect(&mut self, client: ClientId) {
        self.records.entry(client).or_default();
        self.buffers.add_client(client);
        self.inboxes.entry(client).or_default();
        self.filters.entry(client).or_default();
    }

    /// Forgets a client and everything queued for it. The game is told
    /// first ([`Intents::client_gone`]), so a player whose client is gone
    /// leaves the game and the other clients hear of it.
    pub fn disconnect(&mut self, client: ClientId) {
        self.game.client_gone(client, &mut self.buffers);
        self.records.remove(&client);
        self.buffers.remove_client(client);
        self.inboxes.remove(&client);
        self.filters.remove(&client);
    }

    /// The wire the host speaks: [`Wire::Mod`] admits mod messages both
    /// ways (`d2_proto::envelope`). A mod game's host sets it before any
    /// client sends; D2RUST's own host keeps [`Wire::Original`].
    pub fn set_wire(&mut self, wire: Wire) {
        self.queues.wire = wire;
    }

    /// The wire the host speaks ([`Host::set_wire`]).
    pub fn wire(&self) -> Wire {
        self.queues.wire
    }

    /// The server's record of `client`.
    pub fn record(&self, client: ClientId) -> Option<&ClientRecord> {
        self.records.get(&client)
    }

    /// The 1 ms base clock at the latest frame (0 before the first): the
    /// time ticks are scheduled on ([`TickDriver::next_tick_ms`]), for
    /// backend tasks that need a host time finer than a tick.
    pub fn base_ms(&self) -> u64 {
        self.base.ms()
    }

    /// Sets the time the client's game-message sender reads for its
    /// duplicate filter (a host input, a millisecond tick count). A check
    /// run replays the recorded host clock here; `None` goes back to the
    /// host clock.
    pub fn set_send_now(&mut self, now: Option<u32>) {
        self.send_now = now;
    }

    /// Sets the sender's recorded clock per frame: a send while the game
    /// frame is `f` reads the value of the
    /// greatest recorded frame ≤ `f` (`f` is the packet records' `frame`,
    /// so both packet logs key it alike); before the first, the host clock.
    /// [`Host::set_send_now`], when set, comes first.
    pub fn set_send_clock(&mut self, clock: BTreeMap<i32, u32>) {
        self.send_clock = clock;
    }

    /// The client's game-message sender:
    /// `None` when the duplicate filter drops it.
    pub fn send_game(
        &mut self,
        client: ClientId,
        msg: &[u8],
    ) -> Result<Option<Classified>, SendError> {
        let frame = self.game.frame();
        let recorded = self.send_clock.range(..=frame).next_back();
        let now = match (self.send_now, recorded) {
            (Some(t), _) | (None, Some((_, &t))) => t,
            (None, None) => self.clock.now_ms(),
        };
        self.note(PacketEvent::ClientSend { client, msg });
        let filter = self.filters.entry(client).or_default();
        if !filter.pass(msg, now)? {
            return Ok(None);
        }
        self.note(PacketEvent::ClientOut { client, msg });
        self.queues.send(&self.sizes, client, msg).map(Some)
    }

    /// The dispatcher called now for `client`'s player with `msg`, outside
    /// the queues: no sender, no duplicate filter, no client record update
    /// (the debugger call of the poke tool). What the handler sends goes
    /// to the client's buffers as in a drain. `None`: the client is in no
    /// game or has no player (the dispatcher is not reached). The packet
    /// recorder sees a `dispatch` and the S→C messages, but no `c2s` and
    /// no `result`: those belong to the queue entry, which is not run.
    pub fn dispatch_now(&mut self, client: ClientId, msg: &[u8]) -> Option<ResultCode> {
        let PlayerLookup::Player(player) = self.game.player(client) else {
            return None;
        };
        if self.packets.is_some() && !msg.is_empty() {
            let game_frame = self.game.frame();
            self.note(PacketEvent::Dispatch {
                client,
                id: msg[0],
                size: msg.len(),
                game_frame,
            });
        }
        let code = dispatch(
            &mut self.game,
            &self.sizes,
            &mut self.buffers,
            client,
            player,
            msg,
            msg.len(),
        );
        self.note_tap();
        Some(code)
    }

    /// Queues `msg` for `client` now, outside the tick (a poke at the
    /// tick-end point): the bytes join the client's buffer before this
    /// frame's flush, as a send from the poked function would.
    pub fn queue_now(&mut self, client: ClientId, msg: &[u8]) -> Result<(), QueueError> {
        let r = self.buffers.queue(client, msg);
        self.note_tap();
        r
    }

    /// System-message sender: no duplicate filter.
    pub fn send_system(&mut self, client: ClientId, msg: &[u8]) -> Result<Classified, SendError> {
        self.note(PacketEvent::ClientOut { client, msg });
        self.queues.send(&self.sizes, client, msg)
    }

    /// A direct send the local client never reads (the server-start 0xAF:
    /// the client only sets a connection flag from it): it reaches the
    /// packet recorder, not the receive lists.
    pub fn announce_direct(&mut self, client: ClientId, msg: &[u8]) {
        if self.packets.is_some() {
            self.buffers.note(|| Tapped::Direct(client, msg.to_vec()));
            self.note_tap();
        } else {
            self.early_direct.push((client, msg.to_vec()));
        }
    }

    /// Direct send: straight to the client's receive
    /// lists, ahead of anything still buffered.
    pub fn send_direct(&mut self, client: ClientId, msg: &[u8]) -> Result<(), QueueError> {
        self.buffers.note(|| Tapped::Direct(client, msg.to_vec()));
        self.note_tap();
        let wire = self.queues.wire;
        self.inboxes.entry(client).or_default().push_on(wire, msg)
    }

    /// One client frame's server part: drain → tick driver → flush if a
    /// tick ran. The clock is read once.
    pub fn frame(&mut self) -> Result<FrameReport, HostError> {
        self.frame_with(|_| {})
    }

    /// [`Self::frame`] with `at_tick_end` run after the tick and before
    /// its flush, when a tick ran: the point where pokes run. Not run in a
    /// frame without a tick.
    pub fn frame_with(
        &mut self,
        at_tick_end: impl FnOnce(&mut Self),
    ) -> Result<FrameReport, HostError> {
        let now = self.clock.now_ms();
        // The game's tick-count input: the clock plus the host's
        // uptime origin (`seams::HOST_CLOCK_ORIGIN_MS`); the tick driver
        // and the drain keep the clock itself.
        self.game
            .set_host_tick(now.wrapping_add(crate::seams::HOST_CLOCK_ORIGIN_MS));
        // tools/perf: wall-clock timing of the parts, off unless enabled.
        let timed = crate::perf::enabled();
        let t0 = Instant::now();
        let mut report = FrameReport {
            messages: self.drain(now)?,
            ..FrameReport::default()
        };
        let base = self.base.advance(now);
        let ticked = match self.display_lag_ms {
            Some(limit) => self.driver.poll_with_lag(base, Some(limit)),
            None => self.driver.poll(base, self.catch_up),
        };
        if ticked {
            report.ticked = true;
            let drain_us = crate::perf::us_since(t0);
            let t1 = Instant::now();
            if self.packets.is_some() {
                // Sends made before the tick (the harness pokes run before
                // the host frame) belong to the previous frame: the
                // recorder logs a send where it happens.
                self.note_tap();
                let frame = self.game.frame().wrapping_add(1);
                self.note(PacketEvent::Tick { frame });
            }
            self.game.tick(&mut self.buffers);
            if self.packets.is_some() {
                self.note_tap();
            }
            // The `tick_end` record is written after the pokes run.
            at_tick_end(self);
            if self.packets.is_some() {
                // What the pokes queued is the tick's (the send is recorded
                // where it happens, before the `tick_end` record).
                self.note_tap();
                let frame = self.game.frame();
                self.note(PacketEvent::TickEnd { frame });
            }
            let tick_us = crate::perf::us_since(t1);
            let t2 = Instant::now();
            let (buffers, discarded) = self.flush(true, now)?;
            report.flushed_buffers = buffers;
            report.discarded_bytes = discarded;
            if timed {
                crate::perf::record(crate::perf::TickTime {
                    frame: self.game.frame() as u32,
                    drain_us,
                    tick_us,
                    flush_us: crate::perf::us_since(t2),
                    at_us: crate::perf::at_us(t0),
                });
            }
        }
        Ok(report)
    }

    /// Drains the server queues and runs the per-queue handlers.
    fn drain(&mut self, now: u32) -> Result<Vec<HandledMessage>, HostError> {
        let mut out = Vec::new();
        self.note(PacketEvent::Drain);
        for d in self.queues.drain() {
            if d.queue != Queue::Admin {
                self.note(PacketEvent::C2s {
                    system: d.queue == Queue::System,
                    client: d.client,
                    size: d.size,
                    msg: &d.msg,
                });
            }
            // A mod message (only queued on a mod game's wire) goes to the
            // game's mod rules, not to the dispatcher.
            let mod_message = d.queue == Queue::Game
                && self.queues.wire.envelopes()
                && d.msg[0] == envelope::CLIENT_ID;
            // The dispatcher runs exactly when the player lookup finds a
            // player (`process_game_message`); read before, for the record.
            if self.packets.is_some()
                && !mod_message
                && d.queue == Queue::Game
                && matches!(self.game.player(d.client), PlayerLookup::Player(_))
                && self.records.contains_key(&d.client)
            {
                let game_frame = self.game.frame();
                self.note(PacketEvent::Dispatch {
                    client: d.client,
                    id: d.msg[0],
                    size: d.size,
                    game_frame,
                });
            }
            let handled = match d.queue {
                Queue::System => {
                    // The game's session part first (`Intents::session_message`),
                    // with the direct send and the client flush of the leave.
                    let mut sink = SystemSink {
                        buffers: &mut self.buffers,
                        inboxes: &mut self.inboxes,
                        sizes: &self.sizes,
                        wire: self.queues.wire,
                    };
                    if !self
                        .game
                        .session_message(d.client, &d.msg, d.size, &mut sink)
                    {
                        self.session
                            .system_message(d.client, &d.msg, d.size, &mut self.buffers);
                    }
                    Handled::System
                }
                Queue::Game if mod_message => {
                    // The whole envelope as the classifier sized it.
                    let n = match envelope::size(&d.msg) {
                        Size::Bytes(n) => n.min(d.msg.len()),
                        _ => d.msg.len(),
                    };
                    let payload = &d.msg[envelope::HEADER.min(n)..n];
                    Handled::Mod(self.game.mod_message(d.client, payload, &mut self.buffers))
                }
                Queue::Game => Handled::Game(process_game_message(
                    &mut self.game,
                    &self.sizes,
                    &mut self.records,
                    &mut self.buffers,
                    d.client,
                    &d.msg,
                    d.size,
                    now,
                )?),
                Queue::Admin => Handled::AdminIgnored,
            };
            self.note_tap();
            if let Handled::Game(Outcome::Dispatched(code)) = handled {
                self.note(PacketEvent::Result {
                    client: d.client,
                    code,
                });
            }
            out.push(HandledMessage {
                client: d.client,
                id: d.msg[0],
                size: d.size,
                handled,
            });
        }
        Ok(out)
    }

    /// Flush: unless forced, only when at least the shortest tick length
    /// ([`TickDriver::tick_ms`]) passed since the last flush; then every client in the
    /// game's client list, buffers head first, each split into the
    /// client's receive lists. Returns (buffers sent, bytes discarded).
    /// Game types 1 and 2 (three buffers per flush, message 0xB3) are not
    /// single player and not implemented; the empty-game timeout belongs
    /// to the session code.
    pub fn flush(&mut self, force: bool, now: u32) -> Result<(usize, usize), HostError> {
        self.note(PacketEvent::Flush);
        let r = self.flush_inner(force, now);
        self.note_tap();
        r
    }

    fn flush_inner(&mut self, force: bool, now: u32) -> Result<(usize, usize), HostError> {
        if !force {
            if let Some(last) = self.last_flush {
                if now.wrapping_sub(last) < self.driver.tick_ms() {
                    return Ok((0, 0));
                }
            }
        }
        self.last_flush = Some(now);
        // The vitals sync ends the tick's batch: queued inside the flush,
        // before any buffer is sent.
        self.game.flush_sync(&mut self.buffers);
        let (mut sent, mut discarded) = (0, 0);
        for client in self.game.clients() {
            while let Some(buf) = self.buffers.pop(client) {
                let inbox = self.inboxes.entry(client).or_default();
                discarded += inbox.deliver_on(self.queues.wire, &self.sizes, &buf)?;
                sent += 1;
            }
        }
        Ok((sent, discarded))
    }

    /// Client receive: the client's system list, then its
    /// game list, in delivery order.
    pub fn receive(&mut self, client: ClientId) -> Vec<Vec<u8>> {
        let messages = self
            .inboxes
            .get_mut(&client)
            .map(Inbox::receive)
            .unwrap_or_default();
        if self.coverage.enabled {
            for message in &messages {
                if let Some(id) = message.first() {
                    self.coverage.hit("server_packets_sent", || u32::from(*id));
                }
            }
        }
        messages
    }
}

/// The sink of the system-queue drain: queued sends go to the client's
/// buffers; direct sends and a client flush go to its receive lists.
struct SystemSink<'a, S> {
    buffers: &'a mut ClientBuffers,
    inboxes: &'a mut BTreeMap<ClientId, Inbox>,
    sizes: &'a S,
    wire: Wire,
}

impl<S: MessageSizes> MessageSink for SystemSink<'_, S> {
    fn queue(&mut self, client: ClientId, msg: &[u8]) -> Result<(), QueueError> {
        self.buffers.queue(client, msg)
    }
    fn has_queued(&self, client: ClientId) -> bool {
        self.buffers.has_queued(client)
    }
    fn send_direct(&mut self, client: ClientId, msg: &[u8]) -> Result<(), QueueError> {
        self.buffers.note(|| Tapped::Direct(client, msg.to_vec()));
        self.inboxes
            .entry(client)
            .or_default()
            .push_on(self.wire, msg)
    }
    fn flush_client(&mut self, client: ClientId) -> Result<(), QueueError> {
        while let Some(buf) = self.buffers.pop(client) {
            self.inboxes
                .entry(client)
                .or_default()
                .deliver_on(self.wire, self.sizes, &buf)?;
        }
        Ok(())
    }
}
