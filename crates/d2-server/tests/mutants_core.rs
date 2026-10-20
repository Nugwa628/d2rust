//! Integration tests that close mutation-testing gaps in the transport,
//! buffers, dispatcher and host loop of `d2-server`: each test pins a
//! behaviour of the message pipeline that a mutant of
//! `cargo mutants -p d2-server` survived before the test existed.

use std::time::Duration;

use d2_server::adapters::ProtoSizes;
use d2_server::buffers::{ClientBuffers, Inbox, QueueError, BUFFER_SIZE};
use d2_server::dispatch::{bind_hotkey, dispatch, select_skill, BindHotkey, SelectSkill};
use d2_server::host::{Host, SystemClock};
use d2_server::seams::*;
use d2_server::transport::{Queue, ServerQueues, MAX_MESSAGE};

/// A player that passes every gate; the handler records what reaches it.
#[derive(Default)]
struct Game {
    handled: Vec<(u8, usize)>,
    /// Messages the tick queues for client 0.
    tick_out: Vec<Vec<u8>>,
}

impl Intents for Game {
    fn player(&self, _: ClientId) -> PlayerLookup {
        PlayerLookup::Player(PlayerGate {
            mode: 1,
            uninterruptable: false,
        })
    }
    fn frame(&self) -> i32 {
        0
    }
    fn point_state(&self, _: ClientId) -> Option<PointState> {
        None
    }
    fn set_point_accept(&mut self, _: ClientId, _: i32) {}
    fn queue_resync(&mut self, _: ClientId, _: &mut dyn MessageSink) {}
    fn unit_target(&self, _: ClientId, _: u32, _: u32) -> UnitTarget {
        UnitTarget::Missing
    }
    fn handle(
        &mut self,
        _: ClientId,
        msg: &[u8],
        size: usize,
        _: &mut dyn MessageSink,
    ) -> ResultCode {
        self.handled.push((msg[0], size));
        ResultCode::Done
    }
    fn clients(&self) -> Vec<ClientId> {
        vec![0]
    }
}

impl Tick for Game {
    fn tick(&mut self, out: &mut dyn MessageSink) {
        for m in &self.tick_out {
            out.queue(0, m).unwrap();
        }
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
}

struct NoSession;

impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

struct ManualClock(u32);

impl Clock for ManualClock {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

fn run(game: &mut Game, msg: &[u8], size: usize) -> ResultCode {
    let mut out = ClientBuffers::new();
    dispatch(game, &ProtoSizes, &mut out, 0, ALIVE, msg, size)
}

// ---- dispatcher

/// Id ≥ 0x67 returns 3 even when its transport size is
/// right; the handler never runs.
#[test]
fn system_ids_never_reach_a_game_handler() {
    let mut g = Game::default();
    let create = [0x67; 46];
    assert_eq!(ProtoSizes.client_size(&create), Ok(46));
    assert_eq!(run(&mut g, &create, 46), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x69], 1), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x70], 1), ResultCode::Malformed);
    assert_eq!(g.handled, vec![]);
    // The last game id still dispatches (gate none, stub 0).
    assert_eq!(run(&mut g, &[0x66, 0, 0], 3), ResultCode::Done);
}

/// 0x15 has no exact-size check; its handler checks its
/// strings. A chat message with a byte past its size rule
/// reaches the handler with the full size.
#[test]
fn chat_skips_the_exact_size_check() {
    let mut g = Game::default();
    let mut chat = vec![0x15, 0x01, 0x00, b'h', b'i', 0, b'b', b'o', b'b', 0, 0];
    assert_eq!(ProtoSizes.client_size(&chat), Ok(11));
    chat.push(0);
    assert_eq!(run(&mut g, &chat, 12), ResultCode::Done);
    assert_eq!(g.handled, vec![(0x15, 12)]);
    // Any other id is held to its rule size (0x01: 5).
    assert_eq!(run(&mut g, &[1, 0, 0, 0, 0, 0], 6), ResultCode::Malformed);
}

/// Skill fields with the hand bit clear (right hand).
#[test]
fn skill_fields_right_hand() {
    assert_eq!(
        select_skill(&[0x3C, 0x05, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]),
        Some(SelectSkill {
            skill: 5,
            left: false,
            item: u32::MAX,
        })
    );
    assert_eq!(
        bind_hotkey(&[0x51, 0x06, 0x00, 0x03, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]),
        Some(BindHotkey {
            skill: 6,
            left: false,
            slot: 3,
            item: u32::MAX,
        })
    );
}

// ---- transport

/// A queued message waits until the drain.
#[test]
fn queues_report_waiting_messages() {
    let mut q = ServerQueues::new();
    assert!(q.is_empty());
    q.send(&ProtoSizes, 0, &[0x69]).unwrap();
    assert!(!q.is_empty());
    assert_eq!(q.len(Queue::System), 1);
    q.drain();
    assert!(q.is_empty());
}

// ---- buffers and local delivery

/// A buffer is full at exactly 0x200 bytes, so a 0x200-byte
/// message fits one buffer.
#[test]
fn message_of_a_whole_buffer_fits() {
    let mut b = ClientBuffers::new();
    b.add_client(0);
    b.queue(0, &[0x00; BUFFER_SIZE]).unwrap();
    assert_eq!(
        b.queue(0, &[0x00; BUFFER_SIZE + 1]),
        Err(QueueError::TooLarge(BUFFER_SIZE + 1))
    );
    let sizes: Vec<_> = b.buffers(0).unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![BUFFER_SIZE]);
}

/// Client null → nothing; a removed client keeps nothing.
#[test]
fn removed_client_has_no_buffers() {
    let mut b = ClientBuffers::new();
    b.add_client(0);
    b.queue(0, &[0x00]).unwrap();
    b.remove_client(0);
    assert!(b.buffers(0).is_none());
    assert_eq!(b.pop(0), None);
    b.queue(0, &[0x00]).unwrap();
    assert!(b.buffers(0).is_none());
}

/// Sizes up to 0x204 are delivered; above, fatal.
#[test]
fn delivery_accepts_the_largest_message() {
    let mut inbox = Inbox::default();
    inbox.push(&[0x01; MAX_MESSAGE]).unwrap();
    assert_eq!(
        inbox.push(&[0x01; MAX_MESSAGE + 1]),
        Err(QueueError::BadSize(MAX_MESSAGE + 1))
    );
    assert_eq!(inbox.game.len(), 1);
}

/// A size rule the buffer cannot satisfy (0x0A needs 6
/// bytes, 3 are left) or a size-0 id (0x80) ends the split; the rest is
/// discarded.
#[test]
fn split_ends_on_a_short_or_size_zero_message() {
    let remove = [0x0A, 1, 2, 3, 4, 5];
    let mut inbox = Inbox::default();
    let mut buf = remove.to_vec();
    buf.extend_from_slice(&[0x0A, 1, 2]);
    assert_eq!(inbox.deliver(&ProtoSizes, &buf), Ok(3));
    assert_eq!(inbox.receive(), vec![remove.to_vec()]);
    let mut buf = remove.to_vec();
    buf.extend_from_slice(&[0x80, 0, 0, 0]);
    assert_eq!(inbox.deliver(&ProtoSizes, &buf), Ok(4));
    assert_eq!(inbox.receive(), vec![remove.to_vec()]);
}

// ---- host

type TestHost = Host<Game, ProtoSizes, NoSession, ManualClock>;

fn host(game: Game) -> TestHost {
    let mut h = Host::new(game, ProtoSizes, NoSession, ManualClock(1000));
    h.connect(0);
    h
}

/// The flush after a tick reports the bytes a
/// split lost.
#[test]
fn frame_reports_discarded_bytes() {
    let game = Game {
        tick_out: vec![vec![0x0A, 1, 2, 3, 4, 5], vec![0x80, 0, 0]],
        ..Game::default()
    };
    let mut h = host(game);
    assert!(!h.frame().unwrap().ticked);
    h.clock.0 += 40;
    let r = h.frame().unwrap();
    assert!(r.ticked);
    assert_eq!(r.flushed_buffers, 1);
    assert_eq!(r.discarded_bytes, 3);
    assert_eq!(h.receive(0), vec![vec![0x0A, 1, 2, 3, 4, 5]]);
}

/// A disconnected client has no record, no buffers and nothing to
/// receive (a client in a game needs its record).
#[test]
fn disconnect_forgets_the_client() {
    let mut h = host(Game::default());
    h.send_direct(0, &[0x00]).unwrap();
    h.buffers.queue(0, &[0x00]).unwrap();
    assert!(h.record(0).is_some());
    h.disconnect(0);
    assert!(h.record(0).is_none());
    assert!(h.buffers.buffers(0).is_none());
    assert_eq!(h.receive(0), Vec::<Vec<u8>>::new());
}

/// The host clock is wall-clock milliseconds.
#[test]
fn system_clock_advances() {
    let mut c = SystemClock::default();
    let start = c.now_ms();
    std::thread::sleep(Duration::from_millis(30));
    assert!(c.now_ms().wrapping_sub(start) >= 30);
}
