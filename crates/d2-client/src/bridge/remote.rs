//! The remote link: the bridge's server is a `d2rust-server` game on the
//! network (built with the `online` feature).
//!
//! The in-process link runs a server frame when the bridge pumps it. A
//! remote server runs on its own clock and sends its frames as they
//! happen, and the network delivers them unevenly. The link keeps a small
//! buffer of whole frames ([`d2_online::ServerFrame`]) and hands the
//! bridge one ticked frame per tick interval ([`Pacer`]), so the client's
//! own per-tick work and the unit smoothing see an even tick stream. The
//! buffer grows after the network starved it and shrinks again while it
//! stays full, so the delay is no longer than the network needs.
//!
//! A lost connection is not a bridge error: the link stops ticking, says
//! so in its [`RemoteStatus`] ("reconnecting"), and asks its reconnect hook
//! for a new game stream while the server holds the player's slot. When
//! that fails it says the connection is lost, with the reason; the app
//! shows it and leaves the game.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use d2_net::{Lane, ResumeToken};
use d2_online::{GameLink, OnlineError, ServerFrame};
use d2_proto::PROTOCOL_VERSION;
use d2_sim::tick::TickRate;

use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};

/// Where a [`RemoteLink`] gets its server frames from: the game stream, or
/// a test's script.
pub trait FrameSource: Send {
    /// Sends one C→S game message.
    fn send(&mut self, lane: Lane, msg: &[u8]) -> Result<(), OnlineError>;
    /// Sends one mod extension message.
    fn send_ext(&mut self, channel: u16, msg: &[u8]) -> Result<(), OnlineError>;
    /// The next whole server frame that has arrived, or `None`.
    fn poll_frame(&mut self) -> Result<Option<ServerFrame>, OnlineError>;
    /// The token that comes back to this source's game slot, when it has
    /// one (a game stream's `Welcome`). A new stream's welcome renews it.
    fn resume_token(&self) -> Option<ResumeToken> {
        None
    }
}

impl FrameSource for GameLink {
    fn send(&mut self, lane: Lane, msg: &[u8]) -> Result<(), OnlineError> {
        GameLink::send(self, lane, msg)
    }
    fn send_ext(&mut self, channel: u16, msg: &[u8]) -> Result<(), OnlineError> {
        GameLink::send_ext(self, channel, msg)
    }
    fn poll_frame(&mut self) -> Result<Option<ServerFrame>, OnlineError> {
        GameLink::poll_frame(self)
    }
    fn resume_token(&self) -> Option<ResumeToken> {
        Some(self.welcome().resume)
    }
}

/// The connection as the player sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    /// Frames are arriving.
    Playing,
    /// The connection broke; a new one is being made while the server holds
    /// the slot.
    Reconnecting {
        /// Attempts made so far.
        attempts: u32,
    },
    /// The game is over for this client: the text says why.
    Lost(String),
}

/// What a [`RemoteLink`] tells the rest of the app; shared, so the UI reads
/// it without reaching into the bridge.
#[derive(Clone, Debug)]
pub struct RemoteStatus(Arc<Mutex<StatusInner>>);

#[derive(Debug)]
struct StatusInner {
    connection: Connection,
    /// Counts the reconnects that succeeded.
    resumed: u32,
    /// Frames buffered and the pacer's target depth, for the dev overlay.
    buffered: usize,
    target: usize,
}

impl Default for RemoteStatus {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(StatusInner {
            connection: Connection::Playing,
            resumed: 0,
            buffered: 0,
            target: 0,
        })))
    }
}

impl RemoteStatus {
    fn with<T>(&self, f: impl FnOnce(&mut StatusInner) -> T) -> T {
        // A panic elsewhere cannot leave the plain fields inconsistent.
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g)
    }

    /// The connection now.
    pub fn connection(&self) -> Connection {
        self.with(|s| s.connection.clone())
    }

    /// How many times the connection came back.
    pub fn resumed(&self) -> u32 {
        self.with(|s| s.resumed)
    }

    /// (frames buffered, the buffer's target depth).
    pub fn buffer(&self) -> (usize, usize) {
        self.with(|s| (s.buffered, s.target))
    }

    fn set(&self, c: Connection) {
        self.with(|s| s.connection = c);
    }
}

/// Paces ticked server frames out of the buffer: one per tick interval,
/// once `target` of them are buffered.
#[derive(Debug)]
pub struct Pacer {
    interval_ms: f64,
    /// When the next ticked frame is due; `None` while the buffer refills.
    next_ms: Option<f64>,
    /// Ticked frames to hold before playing (and after a starve).
    target: usize,
    /// Since when the buffer has held more than the target, for shrinking it.
    full_since_ms: Option<f64>,
}

impl Pacer {
    /// The first target: two frames absorb most home-network jitter.
    pub const START_TARGET: usize = 2;
    /// Never hold more than this many frames.
    pub const MAX_TARGET: usize = 8;
    /// Frames past the target before the pacer catches up at once.
    const SLACK: usize = 2;
    /// How long the buffer must stay above target before the target drops.
    const SHRINK_AFTER_MS: f64 = 10_000.0;

    pub fn new(rate: TickRate) -> Self {
        Self {
            interval_ms: 1000.0 / f64::from(rate.hz()),
            next_ms: None,
            target: Self::START_TARGET,
            full_since_ms: None,
        }
    }

    pub fn target(&self) -> usize {
        self.target
    }

    /// Whether the next ticked frame is to be released at `now_ms` with
    /// `buffered` ticked frames waiting. Each `true` is one release.
    pub fn due(&mut self, now_ms: f64, buffered: usize) -> bool {
        if buffered > self.target {
            let since = *self.full_since_ms.get_or_insert(now_ms);
            if now_ms - since >= Self::SHRINK_AFTER_MS && self.target > 1 {
                self.target -= 1;
                self.full_since_ms = Some(now_ms);
            }
        } else {
            self.full_since_ms = None;
        }
        if buffered == 0 {
            if self.next_ms.is_some_and(|n| now_ms >= n + self.interval_ms) {
                // A whole interval with nothing to play: the network starved
                // the buffer. Hold more from now on, and refill first.
                self.target = (self.target + 1).min(Self::MAX_TARGET);
                self.next_ms = None;
            }
            return false;
        }
        let Some(next) = self.next_ms else {
            if buffered < self.target {
                return false;
            }
            self.next_ms = Some(now_ms + self.interval_ms);
            return true;
        };
        if buffered > self.target + Self::SLACK {
            // Far behind the server (after a stall): catch up at once,
            // keeping the beat.
            return true;
        }
        if now_ms < next {
            return false;
        }
        // Keep the beat; if the display fell more than a few intervals
        // behind, start the beat over instead of releasing a burst later.
        let mut n = next + self.interval_ms;
        if n < now_ms - 4.0 * self.interval_ms {
            n = now_ms + self.interval_ms;
        }
        self.next_ms = Some(n);
        true
    }
}

/// Makes a new frame source for the slot the server holds (the session's
/// resume). Runs on a worker thread; each call is one attempt. The game
/// token it takes is the latest one: a resumed stream's welcome carries a
/// new token, which the next attempt must present.
pub type Reconnect<S> = Box<dyn FnMut(Option<ResumeToken>) -> Result<S, OnlineError> + Send>;

/// How long the link keeps trying to reconnect (the server holds a lost
/// player's slot for 30 s).
pub const RECONNECT_FOR: Duration = Duration::from_secs(30);
/// The wait between reconnect attempts.
const RECONNECT_EVERY: Duration = Duration::from_secs(1);

/// What the reconnect worker sends back.
type Attempt<S> = (Result<S, OnlineError>, Reconnect<S>);

/// The bridge's link to a game on a remote server.
pub struct RemoteLink<S: FrameSource + 'static> {
    source: Option<S>,
    rate: TickRate,
    pacer: Pacer,
    /// Whole frames waiting to be released, oldest first.
    buffer: VecDeque<ServerFrame>,
    /// The released frames' deliveries the bridge has not received yet.
    ready: Vec<Vec<u8>>,
    ext_in: Vec<(u16, Vec<u8>)>,
    catch_up: bool,
    clock: Box<dyn FnMut() -> f64 + Send>,
    status: RemoteStatus,
    reconnect: Option<Reconnect<S>>,
    /// The token that comes back to the current source's game slot; a
    /// resumed stream's welcome replaces it.
    resume: Option<ResumeToken>,
    /// A reconnect succeeded since the bridge last took the resume: the
    /// next pump hands out nothing, so the bridge clears its world before
    /// the server's resend applies.
    resumed: bool,
    /// The running reconnect attempt and when trying started.
    retry: Option<(Receiver<Attempt<S>>, f64, u32)>,
}

impl<S: FrameSource + 'static> RemoteLink<S> {
    /// A link to the game `source` streams, at the server's tick `rate`
    /// (from its welcome).
    pub fn new(source: S, rate: TickRate) -> Self {
        let start = Instant::now();
        let resume = source.resume_token();
        Self {
            source: Some(source),
            rate,
            pacer: Pacer::new(rate),
            buffer: VecDeque::new(),
            ready: Vec::new(),
            ext_in: Vec::new(),
            catch_up: false,
            clock: Box::new(move || start.elapsed().as_secs_f64() * 1000.0),
            status: RemoteStatus::default(),
            reconnect: None,
            resume,
            resumed: false,
            retry: None,
        }
    }

    /// Reconnects through `hook` when the connection breaks.
    pub fn with_reconnect(mut self, hook: Reconnect<S>) -> Self {
        self.reconnect = Some(hook);
        self
    }

    /// Reads time from `clock` (milliseconds) instead of the wall clock.
    pub fn with_clock(mut self, clock: impl FnMut() -> f64 + Send + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// The status the UI reads.
    pub fn status(&self) -> RemoteStatus {
        self.status.clone()
    }

    /// Sends a mod extension's message on `channel`.
    pub fn send_ext(&mut self, channel: u16, msg: Vec<u8>) -> Result<(), LinkError> {
        if let Some(s) = &mut self.source {
            if let Err(e) = s.send_ext(channel, &msg) {
                self.broke(e);
            }
        }
        Ok(())
    }

    /// The extension messages the server sent since the last call.
    pub fn take_ext(&mut self) -> Vec<(u16, Vec<u8>)> {
        std::mem::take(&mut self.ext_in)
    }

    fn ticked_buffered(&self) -> usize {
        self.buffer.iter().filter(|f| f.ticked).count()
    }

    /// Moves every frame that has arrived into the buffer.
    fn fill(&mut self) {
        let Some(source) = &mut self.source else {
            return;
        };
        loop {
            match source.poll_frame() {
                Ok(Some(f)) => self.buffer.push_back(f),
                Ok(None) => return,
                Err(e) => return self.broke(e),
            }
        }
    }

    /// The connection broke: try to come back, or give up.
    fn broke(&mut self, e: OnlineError) {
        self.source = None;
        let Some(hook) = self.reconnect.take() else {
            self.status.set(Connection::Lost(e.message()));
            return;
        };
        let now = (self.clock)();
        self.status.set(Connection::Reconnecting { attempts: 0 });
        self.retry = Some((spawn_attempt(hook, self.resume, Duration::ZERO), now, 0));
    }

    /// Takes in a finished reconnect attempt.
    fn poll_retry(&mut self) {
        let Some((rx, since, attempts)) = &self.retry else {
            return;
        };
        let (since, attempts) = (*since, *attempts + 1);
        let (result, hook) = match rx.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.retry = None;
                self.status
                    .set(Connection::Lost("the reconnect attempt stopped".into()));
                return;
            }
        };
        self.retry = None;
        match result {
            Ok(source) => {
                // The server re-entered this client: the resend that follows
                // replaces everything, so nothing from before the loss may
                // reach the bridge (contracts 3.7).
                self.resume = source.resume_token();
                self.buffer.clear();
                self.ready.clear();
                self.ext_in.clear();
                self.source = Some(source);
                self.reconnect = Some(hook);
                self.pacer = Pacer::new(self.rate);
                self.resumed = true;
                self.status.set(Connection::Playing);
                self.status.with(|s| s.resumed += 1);
            }
            Err(e) => {
                let elapsed = Duration::from_secs_f64(((self.clock)() - since).max(0.0) / 1000.0);
                // The server refused the slot (gone, or another login): no
                // point trying again.
                let fatal = matches!(e, OnlineError::Refused { .. });
                if fatal || elapsed + RECONNECT_EVERY > RECONNECT_FOR {
                    self.status.set(Connection::Lost(e.message()));
                    return;
                }
                self.status.set(Connection::Reconnecting { attempts });
                self.retry = Some((
                    spawn_attempt(hook, self.resume, RECONNECT_EVERY),
                    since,
                    attempts,
                ));
            }
        }
    }

    /// Releases the frame at the buffer's head into `ready`.
    fn release(&mut self) -> bool {
        let Some(f) = self.buffer.pop_front() else {
            return false;
        };
        self.ready.extend(f.delivers);
        self.ext_in.extend(f.ext);
        f.ticked
    }

    fn publish(&self) {
        let (buffered, target) = (self.ticked_buffered(), self.pacer.target());
        self.status.with(|s| {
            s.buffered = buffered;
            s.target = target;
        });
    }
}

/// Runs one reconnect attempt on a worker thread after `delay`.
fn spawn_attempt<S: Send + 'static>(
    mut hook: Reconnect<S>,
    token: Option<ResumeToken>,
    delay: Duration,
) -> Receiver<Attempt<S>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("d2-online-reconnect".into())
        .spawn(move || {
            std::thread::sleep(delay);
            let result = hook(token);
            let _ = tx.send((result, hook));
        });
    // A thread that cannot start drops `tx`: the poll reads "stopped".
    drop(spawned);
    rx
}

impl<S: FrameSource + 'static> ServerLink for RemoteLink<S> {
    fn protocol_version(&self) -> u32 {
        // The handshake refused any other version.
        PROTOCOL_VERSION
    }

    fn tick_rate(&self) -> TickRate {
        self.rate
    }

    /// Goes to the server's sender for `queue`; whether its duplicate
    /// filter keeps it is the server's business, so a remote send is
    /// always [`Sent::Queued`]. A broken connection drops the message (the
    /// server's game is gone or will be resumed from its own state).
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        let lane = match queue {
            SendQueue::Game => Lane::Game,
            SendQueue::System => Lane::System,
        };
        if let Some(s) = &mut self.source {
            if let Err(e) = s.send(lane, msg) {
                self.broke(e);
            }
        }
        Ok(Sent::Queued)
    }

    /// Releases the frames that are due: every unticked frame at the head
    /// of the buffer, then at most one ticked frame.
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.poll_retry();
        // The frame a resume succeeded in: the bridge has not cleared its
        // world yet, so hand it nothing. `take_resume` tells it to clear;
        // the server's resend goes out from the pump after that.
        if self.resumed {
            return Ok(Pumped::default());
        }
        self.fill();
        while self.buffer.front().is_some_and(|f| !f.ticked) {
            self.release();
        }
        let now = (self.clock)();
        let ticked = self.pacer.due(now, self.ticked_buffered()) && self.release();
        self.publish();
        Ok(Pumped { ticked })
    }

    fn set_display_catch_up(&mut self, enabled: bool) {
        self.catch_up = enabled;
    }

    fn tick_pending(&mut self) -> bool {
        // The pacer decides inside the next pump; a buffered ticked frame
        // past its beat is due.
        self.catch_up
            && self.pacer.next_ms.is_some_and(|n| (self.clock)() >= n)
            && self.ticked_buffered() > 0
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.ready)
    }

    fn is_remote(&self) -> bool {
        true
    }

    fn take_resume(&mut self) -> bool {
        std::mem::take(&mut self.resumed)
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
