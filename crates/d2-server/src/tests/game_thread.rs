//! Tests of [`crate::game_thread`]: a game built on its own thread (served
//! or looped) and [`crate::hub::Hub::serve_frame`], the looped server's
//! frame. The hub tests use the fakes from `fakes.rs` and `d2-net`'s
//! in-memory transport.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use d2_net::{pair, Lane, MemoryEnd, Packet, Transport};

use super::fakes::*;
use crate::game_thread::{Flow, GameThread, ThreadStopped};
use crate::host::Host;
use crate::hub::Hub;

fn send_sync<T: Send + Sync>() {}

/// A game that can't leave its thread (`Rc`), with a frame counter.
struct Counter {
    frames: Rc<Cell<u32>>,
    log: Vec<&'static str>,
}

fn counter() -> Result<Counter, &'static str> {
    Ok(Counter {
        frames: Rc::new(Cell::new(0)),
        log: Vec::new(),
    })
}

/// Waits up to 5 s for `done`.
fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    done()
}

#[test]
fn the_handle_is_send_and_sync_for_a_game_that_is_not() {
    send_sync::<GameThread<Counter>>();
}

#[test]
fn a_served_game_runs_jobs_in_order_on_its_named_thread() {
    let t = GameThread::spawn("game-test", counter).unwrap();
    for step in ["a", "b", "c"] {
        t.with(move |g| g.log.push(step)).unwrap();
    }
    let name = t
        .with(|_| std::thread::current().name().map(String::from))
        .unwrap();
    assert_eq!(name.as_deref(), Some("game-test"));
    assert_eq!(t.with(|g| g.log.clone()).unwrap(), ["a", "b", "c"]);
    // A served game moves only when asked: no frames ran.
    assert_eq!(t.with(|g| g.frames.get()).unwrap(), 0);
}

#[test]
fn a_failing_builder_is_an_error() {
    let e = GameThread::<Counter>::spawn("game-test", || Err("no tables"))
        .err()
        .expect("refused");
    assert!(e.to_string().contains("no tables"), "{e}");
}

#[test]
fn a_panicking_builder_is_an_error() {
    let e = GameThread::<Counter>::spawn("game-test", || -> Result<Counter, &str> {
        panic!("builder")
    })
    .err()
    .expect("refused");
    assert_eq!(e, ThreadStopped("the game builder panicked".into()));
}

#[test]
fn a_panicking_job_stops_the_thread() {
    let t = GameThread::spawn("game-test", counter).unwrap();
    assert!(t.with(|_| -> u32 { panic!("job") }).is_err());
    assert!(t.with(|g| g.frames.get()).is_err());
    assert!(wait_for(|| !t.is_running()));
}

#[test]
fn a_looped_game_runs_frames_and_jobs_between_them() {
    let t = GameThread::spawn_looped("game-test", counter, Duration::ZERO, |g| {
        g.frames.set(g.frames.get() + 1);
        Flow::Go
    })
    .unwrap();
    assert!(wait_for(|| t.with(|g| g.frames.get()).unwrap() >= 3));
    // A job runs after a frame, never during one.
    let a = t.with(|g| g.frames.get()).unwrap();
    let b = t.with(|g| g.frames.get()).unwrap();
    assert!(a >= 1 && b >= a, "{a} {b}");
    assert!(t.is_running());
}

#[test]
fn a_looped_game_stops_when_its_frame_says_so() {
    let t = GameThread::spawn_looped("game-test", counter, Duration::ZERO, |g| {
        g.frames.set(g.frames.get() + 1);
        if g.frames.get() == 5 {
            Flow::Stop
        } else {
            Flow::Go
        }
    })
    .unwrap();
    assert!(wait_for(|| !t.is_running()));
    assert!(t.with(|g| g.frames.get()).is_err());
}

#[test]
fn dropping_the_handle_ends_a_looped_game() {
    struct Flagged(Arc<AtomicBool>);
    impl Drop for Flagged {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let gone = Arc::new(AtomicBool::new(false));
    let flag = gone.clone();
    let t = GameThread::spawn_looped(
        "game-test",
        move || Ok::<_, &str>(Flagged(flag)),
        Duration::from_millis(1),
        |_| Flow::Go,
    )
    .unwrap();
    assert!(!gone.load(Ordering::SeqCst));
    drop(t);
    // The drop waits for the thread, so the game is gone when it returns.
    assert!(gone.load(Ordering::SeqCst));
}

type TestHub = Hub<FakeGame, TsvSizes, FakeSession, ManualClock, MemoryEnd>;

const RUN: [u8; 5] = [0x01, 100, 0, 100, 0];

fn test_hub() -> TestHub {
    let mut g = FakeGame::with_player(0, ALIVE);
    g.tick_out = vec![(0, vec![0x01; 8])];
    g.handler_out = vec![(0, vec![0x0C; 9])];
    Hub::new(Host::new(
        g,
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    ))
}

fn drain(end: &mut MemoryEnd) -> Vec<Packet> {
    std::iter::from_fn(|| end.recv().unwrap()).collect()
}

/// `serve_frame` is `poll`, `frame`, `deliver_all`: the same game and the
/// same packets on the client's end.
#[test]
fn serve_frame_is_poll_frame_then_deliver() {
    let (mut a, mut b) = (test_hub(), test_hub());
    let (mut ca, sa) = pair();
    let (mut cb, sb) = pair();
    a.connect(0, sa);
    b.connect(0, sb);
    let mut got = (Vec::new(), Vec::new());
    for step in 0..5u32 {
        a.host.clock.0 = 1000 + 20 * step;
        b.host.clock.0 = 1000 + 20 * step;
        for end in [&mut ca, &mut cb] {
            end.send(Packet::Send {
                lane: Lane::Game,
                msg: RUN.to_vec(),
            })
            .unwrap();
        }
        let ra = a.serve_frame().unwrap();
        b.poll().unwrap();
        let rb = b.frame().unwrap();
        b.deliver_all();
        assert_eq!(ra, rb);
        got.0.push(drain(&mut ca));
        got.1.push(drain(&mut cb));
    }
    assert_eq!(got.0, got.1);
    assert_eq!(a.host.game.handled, b.host.game.handled);
    assert!(got
        .0
        .iter()
        .flatten()
        .any(|p| matches!(p, Packet::Deliver(_))));
}

/// A hub on a looped game thread serves a client that connects through a
/// job: the client's sends reach the game and the game's ticks reach the
/// client.
#[test]
fn a_looped_hub_serves_a_client_handed_in_by_a_job() {
    let t = GameThread::spawn_looped(
        "game-test",
        || Ok::<_, &str>(test_hub()),
        Duration::ZERO,
        |hub| {
            // A clock that moves 10 ms a frame: a tick every 4 frames.
            hub.host.clock.0 += 10;
            hub.serve_frame().expect("in-memory clients");
            Flow::Go
        },
    )
    .unwrap();
    let (mut client, server) = pair();
    t.with(move |hub| hub.connect(0, server)).unwrap();
    client
        .send(Packet::Send {
            lane: Lane::Game,
            msg: RUN.to_vec(),
        })
        .unwrap();
    let mut seen = Vec::new();
    assert!(wait_for(|| {
        seen.extend(drain(&mut client));
        seen.contains(&Packet::Frame { ticked: true })
            && seen.contains(&Packet::Deliver(vec![0x01; 8]))
    }));
    let handled = t.with(|hub| hub.host.game.handled.len()).unwrap();
    assert!(handled >= 1, "the client's message reached the game");
}
