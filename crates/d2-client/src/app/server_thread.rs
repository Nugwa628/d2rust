//! [`ThreadLink`]: a [`ServerLink`] whose server lives on its own thread.
//!
//! The Bevy app holds its link as `Box<dyn ServerLink + Send + Sync>`
//! ([`crate::bridge::BridgeResource`]). The wired single-player game is
//! not `Send`, so the link and its game live on a served
//! [`d2_server::game_thread::GameThread`]: built there, never moved. The
//! link forwards each call to that thread and waits for the answer.
//!
//! Every call is synchronous (request, then the answer), so `send`,
//! `pump` and `receive` run exactly in the caller's order. The thread adds
//! no concurrency to the game and no clock of its own: the link inside
//! owns the host clock. The server is still in-process.

pub use d2_server::game_thread::ThreadStopped;
use d2_server::game_thread::{Around, GameThread};

use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};

impl From<ThreadStopped> for LinkError {
    fn from(e: ThreadStopped) -> Self {
        LinkError::Server(Box::new(e))
    }
}

/// A hook the thread runs on the link before each pump.
type Hook<L> = Box<dyn FnMut(&mut L) + Send>;

/// What lives on the thread: the link and its before-pump hook.
struct Served<L> {
    link: L,
    before_pump: Option<Hook<L>>,
}

/// A link `L` running on a dedicated thread. `ThreadLink<L>` is `Send +
/// Sync` whatever `L` is: `L` is built on the thread and never leaves it.
pub struct ThreadLink<L> {
    game: GameThread<Served<L>>,
    version: u32,
    /// The game's tick rate, read once when the link is built.
    rate: d2_sim::tick::TickRate,
    /// The read behind [`ServerLink::step_paths`], run on the thread.
    step_paths: Option<StepPaths<L>>,
}

/// A read of the server's paths on the link (see [`ThreadLink::set_step_paths`]).
pub type StepPaths<L> = fn(&mut L) -> Vec<crate::bridge::critter_path::ServerPath>;

impl<L: ServerLink + 'static> ThreadLink<L> {
    /// Starts the thread and builds the link there with `build`. Fails if
    /// `build` fails or panics.
    pub fn spawn<E, F>(build: F) -> Result<Self, ThreadStopped>
    where
        F: FnOnce() -> Result<L, E> + Send + 'static,
        E: std::fmt::Display,
    {
        // The thread logs through the app's tracing dispatcher, and a panic
        // on it (building or serving) is logged for the repro checks before
        // it unwinds.
        let dispatch = bevy::log::tracing::dispatcher::get_default(Clone::clone);
        let around: Around = Box::new(move |body| {
            let _dispatch = bevy::log::tracing::dispatcher::set_default(&dispatch);
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
                let detail = crate::app::checks::capture::panic_text(payload.as_ref());
                bevy::log::tracing::error!(target: "d2_repro_panic", "{detail}");
                std::panic::resume_unwind(payload);
            }
        });
        let game = GameThread::spawn_around("d2-server", around, move || {
            build().map(|link| Served {
                link,
                before_pump: None,
            })
        })?;
        let (version, rate) = game.with(|s| (s.link.protocol_version(), s.link.tick_rate()))?;
        Ok(ThreadLink {
            game,
            version,
            rate,
            step_paths: None,
        })
    }

    /// The read [`ServerLink::step_paths`] runs on the thread. A host that
    /// copies the server's paths itself after the update answers the
    /// update's own path step here.
    pub fn set_step_paths(&mut self, f: StepPaths<L>) {
        self.step_paths = Some(f);
    }

    /// Runs `f` on the link inside the thread and returns its result:
    /// for set-up and inspection (tests, diagnostics), between frames.
    pub fn with<R, F>(&mut self, f: F) -> Result<R, ThreadStopped>
    where
        R: Send + 'static,
        F: FnOnce(&mut L) -> R + Send + 'static,
    {
        self.game.with(move |s| f(&mut s.link))
    }

    /// Installs `f`, run on the link inside the thread before every pump
    /// (replacing an earlier one): between server frames, before the
    /// next frame's drain and tick (`play --poke`).
    pub fn set_before_pump<F>(&mut self, f: F) -> Result<(), ThreadStopped>
    where
        F: FnMut(&mut L) + Send + 'static,
    {
        self.game.with(move |s| s.before_pump = Some(Box::new(f)))
    }
}

impl<L: ServerLink + 'static> ServerLink for ThreadLink<L> {
    fn protocol_version(&self) -> u32 {
        self.version
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.rate
    }

    fn server_frame(&mut self) -> Option<i32> {
        self.with(|l| l.server_frame()).ok().flatten()
    }

    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        let msg = msg.to_vec();
        self.game.with(move |s| s.link.send(queue, &msg))?
    }

    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.game.with(|s| {
            if let Some(h) = s.before_pump.as_mut() {
                h(&mut s.link);
            }
            s.link.pump()
        })?
    }

    fn set_display_catch_up(&mut self, enabled: bool) {
        let _ = self.with(move |l| l.set_display_catch_up(enabled));
    }
    fn tick_pending(&mut self) -> bool {
        self.with(|l| l.tick_pending()).unwrap_or(false)
    }

    /// A stopped thread delivers nothing; the next `pump` reports it.
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.game.with(|s| s.link.receive()).unwrap_or_default()
    }

    /// The read set by [`ThreadLink::set_step_paths`], else none; a
    /// stopped thread has none.
    fn step_paths(&mut self) -> Vec<crate::bridge::critter_path::ServerPath> {
        match self.step_paths {
            Some(f) => self.with(f).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Run on the thread; a stopped thread ignores it (the next `pump`
    /// reports the stop).
    fn set_send_now(&mut self, now: Option<u32>) {
        let _ = self.game.with(move |s| s.link.set_send_now(now));
    }
}
