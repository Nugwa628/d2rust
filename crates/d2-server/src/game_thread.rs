//! [`GameThread`]: a game that lives on its own thread.
//!
//! The wired game is not `Send`: its world source and level types are
//! boxed trait objects with no `Send` bound, and world generation shares
//! its level types as `Rc<RefCell<…>>`. So the game is built on a thread
//! of its own, from a `Send` builder, and never leaves that thread. Other
//! threads reach it only through jobs ([`GameThread::with`]).
//!
//! There are two ways to run it:
//! - **Served** ([`GameThread::spawn`]): the thread waits for jobs and runs
//!   each one as it arrives. The game moves only when a caller asks. Single
//!   player works this way: the client's link drives every frame.
//! - **Looped** ([`GameThread::spawn_looped`]): the thread owns the frame
//!   loop. It runs one frame, then the jobs that arrived meanwhile, then
//!   waits the frame pause. It repeats until the frame says
//!   [`Flow::Stop`] or the handle is dropped. A dedicated server works this
//!   way: its frame is [`crate::hub::Hub::serve_frame`], and jobs hand it new
//!   connections or read its state.
//!
//! Either way the game runs on one thread, so jobs and frames never
//! overlap, and jobs run in the order they were given.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

/// The game thread is gone: its builder failed or panicked, a frame or job
/// panicked, or a looped frame said [`Flow::Stop`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("game thread stopped: {0}")]
pub struct ThreadStopped(pub String);

/// What a looped frame tells the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Run the jobs, wait the pause, then run the next frame.
    Go,
    /// End the loop now. Jobs still waiting are not run.
    Stop,
}

type Job<G> = Box<dyn FnOnce(&mut G) + Send>;

/// Runs the thread's whole body (build, then serve or loop) on the new
/// thread: the caller sets up per-thread state around it (a log
/// dispatcher) or sees a panic before it unwinds. It must call the body.
pub type Around = Box<dyn FnOnce(Box<dyn FnOnce() + Send>) + Send>;

/// A game `G` that lives on its own thread. `GameThread<G>` is `Send +
/// Sync` whatever `G` is, because `G` is built on the thread and never
/// leaves it. Dropping the handle ends the thread and waits for it.
pub struct GameThread<G> {
    jobs: Option<Sender<Job<G>>>,
    thread: Option<JoinHandle<()>>,
}

impl<G: 'static> GameThread<G> {
    /// Starts a served game thread named `name` and builds the game there
    /// with `build`. Fails if `build` fails or panics.
    pub fn spawn<E, F>(name: &str, build: F) -> Result<Self, ThreadStopped>
    where
        F: FnOnce() -> Result<G, E> + Send + 'static,
        E: std::fmt::Display,
    {
        Self::start(name, None, build, serve)
    }

    /// [`GameThread::spawn`] with the thread's body run inside `around`.
    pub fn spawn_around<E, F>(name: &str, around: Around, build: F) -> Result<Self, ThreadStopped>
    where
        F: FnOnce() -> Result<G, E> + Send + 'static,
        E: std::fmt::Display,
    {
        Self::start(name, Some(around), build, serve)
    }

    /// Starts a looped game thread named `name`. It builds the game with
    /// `build`, then runs `frame`, the jobs that arrived meanwhile, and a
    /// sleep of `pause` (none when zero), over and over. Fails if `build`
    /// fails or panics.
    pub fn spawn_looped<E, F, R>(
        name: &str,
        build: F,
        pause: Duration,
        mut frame: R,
    ) -> Result<Self, ThreadStopped>
    where
        F: FnOnce() -> Result<G, E> + Send + 'static,
        E: std::fmt::Display,
        R: FnMut(&mut G) -> Flow + Send + 'static,
    {
        Self::start(name, None, build, move |game, jobs| loop {
            if frame(game) == Flow::Stop {
                return;
            }
            loop {
                match jobs.try_recv() {
                    Ok(job) => job(game),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if !pause.is_zero() {
                std::thread::sleep(pause);
            }
        })
    }

    fn start<E, F, Run>(
        name: &str,
        around: Option<Around>,
        build: F,
        run: Run,
    ) -> Result<Self, ThreadStopped>
    where
        F: FnOnce() -> Result<G, E> + Send + 'static,
        E: std::fmt::Display,
        Run: FnOnce(&mut G, Receiver<Job<G>>) + Send + 'static,
    {
        let (job_tx, job_rx) = mpsc::channel::<Job<G>>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let body: Box<dyn FnOnce() + Send> = Box::new(move || {
                    let mut game = match build() {
                        Ok(game) => game,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    if ready_tx.send(Ok(())).is_err() {
                        return;
                    }
                    run(&mut game, job_rx);
                });
                match around {
                    Some(around) => around(body),
                    None => body(),
                }
            })
            .map_err(|e| ThreadStopped(e.to_string()))?;
        let ready = ready_rx.recv();
        match ready {
            Ok(Ok(())) => Ok(GameThread {
                jobs: Some(job_tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(ThreadStopped(format!("building the game: {e}")))
            }
            Err(_) => {
                let _ = thread.join();
                Err(ThreadStopped("the game builder panicked".into()))
            }
        }
    }

    /// Runs `f` on the game inside the thread and returns its result. A
    /// served thread runs it next; a looped thread runs it after its
    /// current frame.
    pub fn with<R, F>(&self, f: F) -> Result<R, ThreadStopped>
    where
        R: Send + 'static,
        F: FnOnce(&mut G) -> R + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let jobs = self.jobs.as_ref().ok_or_else(|| stopped("closed"))?;
        jobs.send(Box::new(move |game| {
            let _ = tx.send(f(game));
        }))
        .map_err(|_| stopped("job"))?;
        rx.recv().map_err(|_| stopped("answer"))
    }

    /// True while the thread runs: false once a looped frame said
    /// [`Flow::Stop`], or a frame or job panicked.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

/// A served thread's run: each job as it arrives, until the handle drops.
fn serve<G>(game: &mut G, jobs: Receiver<Job<G>>) {
    for job in jobs {
        job(game);
    }
}

fn stopped(at: &str) -> ThreadStopped {
    ThreadStopped(format!("no {at} from the game thread"))
}

impl<G> Drop for GameThread<G> {
    fn drop(&mut self) {
        // Closing the job channel ends a served thread at once and a
        // looped one after its current frame.
        self.jobs = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
