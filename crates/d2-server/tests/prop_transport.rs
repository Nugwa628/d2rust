//! Robustness properties (METHODS M07, `CLAUDE.md` hard rule 7): any
//! bytes a client sends go through the local transport (duplicate
//! filter, classifier, queues, drain), the game-message entry and the
//! dispatcher with its gate, size check and parsers, without a panic, and
//! with the result code the dispatch rules give. Any S→C buffer goes
//! through local delivery. The game behind the seams is a fake whose
//! answers are drawn by the test; the wired sim is `prop_handle.rs`.
//! These tests cover the server's message transport (`d2_server::transport`
//! and `d2_server::dispatch`), the path every client message takes before
//! a game handler sees it.
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder (it overrides every default here).

use std::collections::BTreeMap;

use d2_proto::envelope;
use d2_proto::schema::{Gate as TsvGate, HandlerSize, Kind as TsvKind};
use d2_proto::transport::{classify_client, Classified as ProtoClassified, ClientQueue};
use d2_proto::CLIENT_MESSAGES;
use d2_server::adapters::ProtoSizes;
use d2_server::buffers::{queued_form, server_ids, ClientBuffers, Inbox, QueueError, BUFFER_SIZE};
use d2_server::dispatch::{dispatch, process_game_message, ClientRecord, Outcome};
use d2_server::host::{Handled, Host};
use d2_server::seams::{
    ClientId, Clock, Intents, MessageSink, PlayerGate, PlayerLookup, PointState, Pos, ResultCode,
    SessionHandler, Tick, UnitTarget,
};
use d2_server::transport::{
    classify, Classified, DuplicateFilter, Queue, SendError, ServerQueues, DRAIN_COPY,
    MAX_GAME_SEND, MAX_MESSAGE,
};
use proptest::prelude::*;
use proptest::test_runner::Config;

/// Proptest config with `default` cases, or `PROPTEST_CASES` when set.
fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

/// Bytes biased to the values size fields, strings and unit types react
/// to.
fn byte() -> impl Strategy<Value = u8> {
    prop_oneof![
        4 => any::<u8>(),
        2 => Just(0u8),
        1 => Just(0xFFu8),
        1 => Just(0x80u8),
        1 => 0u8..8,
    ]
}

/// A C→S message: any id (often a game id), then `0..max` bytes, or a
/// message of exactly its id's fixed size.
fn message(max: usize) -> impl Strategy<Value = Vec<u8>> {
    let id = prop_oneof![3 => 0u8..0x71, 1 => any::<u8>()];
    (id, prop::collection::vec(byte(), 0..max), any::<bool>()).prop_map(|(id, rest, exact)| {
        let mut m = vec![id];
        m.extend(rest);
        if exact {
            if let Some(n) =
                d2_proto::transport::client_message(id).and_then(|r| r.transport_size.fixed())
            {
                m.resize(n, 0);
            }
        }
        m
    })
}

// ---- the fake game ---------------------------------------------------------------------

/// What the fake game answers; drawn per case.
#[derive(Clone, Copy, Debug)]
struct Answers {
    lookup: u8,
    gate: PlayerGate,
    point: Option<PointState>,
    frame: i32,
    target: u8,
    player: Pos,
    other: Pos,
}

fn pos() -> impl Strategy<Value = Pos> {
    // Subtile coordinates: what a u16 target field can name, and a margin.
    (-0x100i32..0x10100, -0x100i32..0x10100).prop_map(|(x, y)| Pos { x, y })
}

fn answers() -> impl Strategy<Value = Answers> {
    (
        0u8..8,
        (
            prop_oneof![Just(0u32), Just(1), Just(0x11), any::<u32>()],
            any::<bool>(),
        ),
        prop::option::weighted(0.9, (pos(), any::<i32>())),
        any::<i32>(),
        0u8..4,
        pos(),
        pos(),
    )
        .prop_map(
            |(lookup, (mode, un), point, frame, target, player, other)| Answers {
                lookup,
                gate: PlayerGate {
                    mode,
                    uninterruptable: un,
                },
                point: point.map(|(player, last_accept)| PointState {
                    player,
                    last_accept,
                }),
                frame,
                target,
                player,
                other,
            },
        )
}

/// A game behind the seams that answers from [`Answers`] and logs every
/// call that changes something.
#[derive(Debug)]
struct Fake {
    a: Answers,
    handled: Vec<(ClientId, Vec<u8>, usize)>,
    accepts: Vec<i32>,
    resyncs: usize,
    ticks: usize,
}

impl Fake {
    fn new(a: Answers) -> Self {
        Self {
            a,
            handled: Vec::new(),
            accepts: Vec::new(),
            resyncs: 0,
            ticks: 0,
        }
    }
}

impl Intents for Fake {
    fn player(&self, _: ClientId) -> PlayerLookup {
        match self.a.lookup {
            0 => PlayerLookup::NotInGame,
            1 => PlayerLookup::NoPlayer,
            _ => PlayerLookup::Player(self.a.gate),
        }
    }
    fn frame(&self) -> i32 {
        self.a.frame
    }
    fn point_state(&self, _: ClientId) -> Option<PointState> {
        self.a.point
    }
    fn set_point_accept(&mut self, _: ClientId, frame: i32) {
        self.accepts.push(frame);
    }
    fn queue_resync(&mut self, _: ClientId, _: &mut dyn MessageSink) {
        self.resyncs += 1;
    }
    fn unit_target(&self, _: ClientId, unit_type: u32, _: u32) -> UnitTarget {
        assert!(unit_type < 6, "unit type {unit_type} reached the lookup");
        match self.a.target {
            0 => UnitTarget::Missing,
            1 => UnitTarget::OwnedItem,
            2 => UnitTarget::OtherAct,
            _ => UnitTarget::At {
                player: self.a.player,
                target: self.a.other,
            },
        }
    }
    fn handle(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        _: &mut dyn MessageSink,
    ) -> ResultCode {
        self.handled.push((client, msg.to_vec(), size));
        ResultCode::Done
    }
    fn clients(&self) -> Vec<ClientId> {
        vec![0]
    }
}

impl Tick for Fake {
    fn tick(&mut self, _: &mut dyn MessageSink) {
        self.ticks += 1;
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
}

/// The expected dispatch result from the TSV columns, independent of `d2_server::dispatch`'s own tables; `None`
/// when the message reaches the handler.
fn expected(a: &Answers, msg: &[u8], size: usize) -> Option<ResultCode> {
    use ResultCode::*;
    let id = msg[0];
    if id == 0 || id >= 0x67 {
        return Some(Malformed);
    }
    let row = &CLIENT_MESSAGES[id as usize];
    if row.kind == TsvKind::None {
        return Some(Malformed);
    }
    let alive = !(a.gate.uninterruptable || a.gate.mode == 0 || a.gate.mode == 0x11);
    let open = match row.gate {
        TsvGate::None => true,
        TsvGate::Dead => a.gate.mode == 0x11,
        _ => alive,
    };
    if !open {
        return Some(Done);
    }
    match row.kind {
        TsvKind::Stub0 => return Some(Done),
        TsvKind::Stub3 => return Some(Malformed),
        _ => {}
    }
    let size_ok = match row.handler_size {
        HandlerSize::Exact(n) => size == n as usize,
        HandlerSize::Range(lo, hi) => (lo as usize..=hi as usize).contains(&size),
        HandlerSize::Chat | HandlerSize::Any | HandlerSize::None => true,
    };
    if !size_ok {
        return Some(Malformed);
    }
    let u16_at = |o: usize| u16::from_le_bytes([msg[o], msg[o + 1]]) as i32;
    let u32_at = |o: usize| u32::from_le_bytes([msg[o], msg[o + 1], msg[o + 2], msg[o + 3]]);
    let near = |p: Pos, t: Pos| (t.x - p.x).abs() <= 50 && (t.y - p.y).abs() <= 50;
    match id {
        0x01 | 0x03 | 0x05 | 0x08 | 0x0C | 0x0F => {
            let Some(p) = a.point else {
                return Some(Invalid);
            };
            if !near(
                p.player,
                Pos {
                    x: u16_at(1),
                    y: u16_at(3),
                },
            ) {
                return Some(Refused);
            }
        }
        0x02 | 0x04 | 0x06 | 0x07 | 0x09 | 0x0A | 0x0D | 0x0E | 0x10 | 0x11 => {
            if u32_at(1) >= 6 {
                return Some(Invalid);
            }
            match a.target {
                0 => return Some(Refused),
                1 => {}
                2 => return Some(Invalid),
                _ if !near(a.player, a.other) => return Some(Refused),
                _ => {}
            }
        }
        // Chat 0x14: strlen 0 is done with no effect; strlen ≥ 256 → 2.
        0x14 => match msg.get(3..).unwrap_or(&[]).iter().position(|&c| c == 0) {
            Some(0) => return Some(Done),
            Some(n) if n < 256 => {}
            _ => return Some(Invalid),
        },
        // Chat 0x15: strlen < 256 and strlen + 4 < size, else rejected
        // with 2.
        // A message shorter than its string's offset has no NUL: rejected.
        0x15 => match msg.get(3..).unwrap_or(&[]).iter().position(|&c| c == 0) {
            Some(n) if n < 256 && n + 4 < size => {}
            _ => return Some(Invalid),
        },
        _ => {}
    }
    None
}

/// The sender's duplicate-filter state, independent of
/// `d2_server::transport`: a 0x200-byte store (zeroed, overwritten only
/// over each sent message's size) and the time of the last send.
struct FilterModel {
    store: Vec<u8>,
    at: u32,
}

impl Default for FilterModel {
    fn default() -> Self {
        Self {
            store: vec![0; 0x200],
            at: 0,
        }
    }
}

impl FilterModel {
    /// True when `m` (shorter than 0x200 bytes) is sent at `now`.
    fn pass(&mut self, m: &[u8], now: u32) -> bool {
        let window = match m.first() {
            Some(0x05..=0x0A | 0x0C..=0x11) => Some(50),
            Some(0x3A) | None => None,
            Some(_) => Some(200),
        };
        if let Some(w) = window {
            if self.store[..m.len()] == *m && now.wrapping_sub(self.at) < w {
                return false;
            }
        }
        self.store[..m.len()].copy_from_slice(m);
        self.at = now;
        true
    }
}

/// Regression (`duplicate_filter_any`, seen once under nextest): the
/// store keeps bytes past a shorter message, so a longer message whose
/// tail matches them is a repeat although no sent message starts with
/// it. The old oracle (`last.starts_with(m)`) called this drop wrong.
#[test]
fn duplicate_filter_store_tail() {
    let mut f = DuplicateFilter::default();
    assert_eq!(f.pass(&[0x20, 1, 2, 3], 0), Ok(true));
    assert_eq!(f.pass(&[0x21], 300), Ok(true));
    // Store is now 21 01 02 03: within 200 ms, 21 01 02 repeats it.
    assert_eq!(f.pass(&[0x21, 1, 2], 350), Ok(false));
    // The store and time stay as they were after a drop.
    assert_eq!(f.pass(&[0x21, 1, 2, 3], 499), Ok(false));
    assert_eq!(f.pass(&[0x21, 1, 2, 3], 500), Ok(true));
}

/// Regression: a 0x15 shorter than its string's offset (+3) has no NUL,
/// so both the oracle and the dispatcher reject it with 2 (the old oracle sliced `msg[3..]` and panicked on the random case
/// `[0x15]`).
#[test]
fn chat_0x15_shorter_than_its_string() {
    let a = Answers {
        lookup: 0,
        gate: PlayerGate {
            mode: 1,
            uninterruptable: false,
        },
        point: None,
        frame: 0,
        target: 1,
        player: Pos { x: 0, y: 0 },
        other: Pos { x: 0, y: 0 },
    };
    for m in [&[0x15][..], &[0x15, 0], &[0x15, 0, 0]] {
        let mut game = Fake::new(a);
        let mut out = ClientBuffers::new();
        let code = dispatch(&mut game, &ProtoSizes, &mut out, 0, a.gate, m, m.len());
        assert_eq!(expected(&a, m, m.len()), Some(ResultCode::Invalid), "{m:?}");
        assert_eq!(code, ResultCode::Invalid, "{m:?}");
        assert!(game.handled.is_empty());
    }
}

/// Regression: the store and time start zeroed, so an all-zero message
/// inside the first 200 ms is dropped before anything was sent, and a
/// short message followed by itself plus zero bytes repeats the zeroed
/// tail. The first assert is the shrunk failing input of the old oracle
/// (`sends = [([0x00], 0)]`; PROPTEST_RNG_SEED 1, 2 and 3 all reach it,
/// after about 23k–60k cases).
#[test]
fn duplicate_filter_zeroed_store() {
    let mut f = DuplicateFilter::default();
    assert_eq!(f.pass(&[0x00], 0), Ok(false));
    assert_eq!(f.pass(&[0x00, 0x00], 199), Ok(false));
    assert_eq!(f.pass(&[0x00, 0x00], 200), Ok(true));
    let mut f = DuplicateFilter::default();
    assert_eq!(f.pass(&[0x20], 1000), Ok(true));
    assert_eq!(f.pass(&[0x20, 0x00], 1100), Ok(false));
    // A 50 ms id outside its window passes; 0x3A never filters.
    assert_eq!(f.pass(&[0x05], 2000), Ok(true));
    assert_eq!(f.pass(&[0x05], 2050), Ok(true));
    assert_eq!(f.pass(&[0x3A], 2051), Ok(true));
    assert_eq!(f.pass(&[0x3A], 2051), Ok(true));
}

proptest! {
    #![proptest_config(config(256))]

    /// The server classifier on `d2-proto`'s sizes agrees with
    /// `d2-proto`'s classifier (the queue rule), and 0xFF with the gate
    /// closed is invalid.
    #[test]
    fn classifiers_agree(m in message(0x240), gate in any::<bool>()) {
        for k in 0..=m.len() {
            let b = &m[..k];
            let ours = classify(&ProtoSizes, b, gate);
            let theirs = classify_client(b);
            let want = match theirs {
                ProtoClassified::Queue(ClientQueue::Game) => Classified::Queued(Queue::Game),
                ProtoClassified::Queue(ClientQueue::System) => Classified::Queued(Queue::System),
                ProtoClassified::Queue(ClientQueue::Admin) if gate => Classified::Queued(Queue::Admin),
                ProtoClassified::Queue(ClientQueue::Admin) => Classified::Invalid,
                ProtoClassified::Incomplete => Classified::Incomplete,
                ProtoClassified::Invalid => Classified::Invalid,
                ProtoClassified::NegativeSize(n) => Classified::NegativeSize(n),
            };
            prop_assert_eq!(ours, want, "{:02X?}", b);
        }
    }

    /// Sends and drains: over 0x204 bytes is the sender's
    /// assert; queued messages come back system queue first,
    /// FIFO within a queue, truncated to 0x1FC bytes with their full size.
    #[test]
    fn queues_send_and_drain(msgs in prop::collection::vec(message(0x240), 0..24)) {
        let mut q = ServerQueues::new();
        let mut want: [Vec<(u32, Vec<u8>)>; 3] = Default::default();
        for (i, m) in msgs.iter().enumerate() {
            let client = i as u32 % 3;
            match q.send(&ProtoSizes, client, m) {
                Err(e) => {
                    prop_assert!(m.len() > MAX_MESSAGE);
                    prop_assert_eq!(e, SendError::TooLarge(m.len()));
                }
                Ok(c) => {
                    prop_assert!(m.len() <= MAX_MESSAGE);
                    prop_assert_eq!(c, classify(&ProtoSizes, m, true));
                    if let Classified::Queued(queue) = c {
                        want[queue as usize].push((client, m.clone()));
                    }
                }
            }
        }
        let drained = q.drain();
        prop_assert!(q.is_empty());
        let mut it = drained.iter();
        for queue in [Queue::System, Queue::Game, Queue::Admin] {
            for (client, m) in &want[queue as usize] {
                let d = it.next().expect("drained");
                prop_assert_eq!(d.queue, queue);
                prop_assert_eq!(d.client, *client);
                prop_assert_eq!(d.size, m.len());
                prop_assert_eq!(&d.msg[..], &m[..m.len().min(DRAIN_COPY)]);
            }
        }
        prop_assert!(it.next().is_none());
    }

    /// The client's duplicate filter on any message and
    /// time: 0x200 bytes or more is the sender's assert; a message is
    /// dropped exactly when its window applies, it equals the store over
    /// its own size and less than the window has passed. The store is the
    /// model's own 0x200 bytes (zeroed at start, each send overwrites its
    /// length only), not just the last message: see `filter_model`.
    #[test]
    fn duplicate_filter_any(sends in prop::collection::vec((message(0x220), 0u32..400), 0..24)) {
        let mut f = DuplicateFilter::default();
        let mut model = FilterModel::default();
        let mut now = 0u32;
        for (m, dt) in sends {
            now = now.wrapping_add(dt);
            match f.pass(&m, now) {
                Err(e) => {
                    prop_assert!(m.len() >= MAX_GAME_SEND);
                    prop_assert_eq!(e, SendError::GameTooLarge(m.len()));
                }
                Ok(sent) => {
                    prop_assert!(m.len() < MAX_GAME_SEND);
                    prop_assert_eq!(sent, model.pass(&m, now), "{:02X?} at {}", m, now);
                }
            }
        }
    }

    /// Per-client buffers: messages over 0x200 bytes are
    /// refused, buffers hold whole messages, never more than 0x200 bytes,
    /// and concatenate back to the queued messages in order (a 0x3E in its
    /// 0x22-byte queued form); unknown
    /// clients are ignored.
    #[test]
    fn client_buffers_any(msgs in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..0x240), 0..24)) {
        let mut b = ClientBuffers::new();
        b.add_client(7);
        let mut want = Vec::new();
        for m in &msgs {
            prop_assert_eq!(b.queue(9, m), Ok(()));
            match b.queue(7, m) {
                Ok(()) => want.extend_from_slice(&queued_form(m)),
                Err(e) => {
                    prop_assert!(m.len() > BUFFER_SIZE);
                    prop_assert_eq!(e, QueueError::TooLarge(m.len()));
                }
            }
        }
        let mut got = Vec::new();
        for buf in b.buffers(7).unwrap() {
            prop_assert!(buf.len() <= BUFFER_SIZE);
            got.extend_from_slice(buf);
        }
        prop_assert_eq!(got, want);
        prop_assert!(b.buffers(9).is_none());
    }

    /// Local delivery of any buffer and any direct send:
    /// what lands in the lists plus what is discarded is the buffer; the
    /// lists hold ids by range; a bad id or size is an error, never a
    /// panic.
    #[test]
    fn inbox_any(buf in prop::collection::vec(any::<u8>(), 0..0x240), direct in prop::collection::vec(any::<u8>(), 0..0x240)) {
        let mut inbox = Inbox::default();
        match inbox.deliver(&ProtoSizes, &buf) {
            Ok(discarded) => {
                prop_assert!(discarded <= buf.len());
                let got: Vec<u8> = inbox.game.iter().chain(inbox.system.iter()).flatten().copied().collect();
                prop_assert_eq!(got.len() + discarded, buf.len());
            }
            Err(QueueError::BadId(id)) => prop_assert!(id >= server_ids() || id == envelope::SERVER_ID),
            // A size rule above 0x204 is a fatal assert in the delivery
            // code (found on a CI-only seed: id 0x94 sized 537).
            Err(QueueError::BadSize(n)) => prop_assert!(n > MAX_MESSAGE),
            Err(e) => return Err(TestCaseError::fail(format!("{e:?}"))),
        }
        for m in inbox.game.iter() {
            prop_assert!(m[0] < 0xAF);
        }
        for m in inbox.system.iter() {
            prop_assert!((0xAF..server_ids()).contains(&m[0]));
        }
        let before = inbox.game.len() + inbox.system.len();
        match inbox.push(&direct) {
            Ok(()) => prop_assert_eq!(inbox.game.len() + inbox.system.len(), before + 1),
            Err(QueueError::BadSize(n)) => prop_assert!(n == 0 || n > MAX_MESSAGE),
            Err(QueueError::BadId(id)) => prop_assert!(id >= server_ids() || id == envelope::SERVER_ID),
            Err(e) => return Err(TestCaseError::fail(format!("{e:?}"))),
        }
    }

    /// The dispatcher on any drained message and any game
    /// answers: the result code is the expected one, the handler runs exactly
    /// when everything before it passed and sees the drained bytes, a
    /// point target is recorded only when accepted.
    #[test]
    fn dispatch_any(m in message(0x240), a in answers()) {
        let mut game = Fake::new(a);
        let mut out = ClientBuffers::new();
        let size = m.len();
        let drained = &m[..size.min(DRAIN_COPY)];
        let code = dispatch(&mut game, &ProtoSizes, &mut out, 0, a.gate, drained, size);
        let want = expected(&a, drained, size);
        match want {
            Some(w) => {
                prop_assert_eq!(code, w);
                prop_assert!(game.handled.is_empty());
            }
            None => {
                prop_assert_eq!(code, ResultCode::Done);
                prop_assert_eq!(&game.handled, &vec![(0, drained.to_vec(), size)]);
            }
        }
        let point = matches!(m[0], 0x01 | 0x03 | 0x05 | 0x08 | 0x0C | 0x0F);
        if point && want.is_none() {
            prop_assert_eq!(&game.accepts, &vec![a.frame]);
        } else {
            prop_assert!(game.accepts.is_empty());
        }
        prop_assert!(game.resyncs <= 1);
    }

    /// Game-message entry: not in game and no player drop the
    /// message before the dispatcher; a missing client record is a
    /// fatal assert (an error); otherwise the record's time is
    /// set and the dispatcher runs.
    #[test]
    fn process_any(m in message(0x40), a in answers(), has_record in any::<bool>(), now in any::<u32>()) {
        let mut game = Fake::new(a);
        let mut out = ClientBuffers::new();
        let mut records = BTreeMap::new();
        if has_record {
            records.insert(0, ClientRecord::default());
        }
        let r = process_game_message(&mut game, &ProtoSizes, &mut records, &mut out, 0, &m, m.len(), now);
        match (a.lookup, has_record) {
            (0, _) => prop_assert_eq!(r, Ok(Outcome::NotInGame)),
            (_, false) => prop_assert!(r.is_err()),
            (1, true) => prop_assert_eq!(r, Ok(Outcome::NoPlayer)),
            _ => prop_assert!(matches!(r, Ok(Outcome::Dispatched(_)))),
        }
        if a.lookup != 0 && has_record {
            prop_assert_eq!(records[&0].last_message_ms, now);
        }
    }
}

// ---- the host loop ---------------------------------------------------------------------

#[derive(Default)]
struct Session(Vec<(Vec<u8>, usize)>);

impl SessionHandler for Session {
    fn system_message(&mut self, _: ClientId, msg: &[u8], size: usize, _: &mut dyn MessageSink) {
        self.0.push((msg.to_vec(), size));
    }
}

struct Ms(u32);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

proptest! {
    #![proptest_config(config(64))]

    /// The host frame over any client sends on both
    /// senders and any S→C direct sends: frames never fail for a
    /// connected client, every queued message is handled once in drain
    /// order, and what the client receives is well-formed.
    #[test]
    fn host_frames_any(
        a in answers(),
        frames in prop::collection::vec(
            (prop::collection::vec((any::<bool>(), message(0x240)), 0..6), 0u32..100,
             prop::option::of(prop::collection::vec(any::<u8>(), 0..0x220))),
            1..8,
        ),
    ) {
        let mut a = a;
        a.lookup = a.lookup.max(1);
        let mut host = Host::new(Fake::new(a), ProtoSizes, Session::default(), Ms(1000));
        host.connect(0);
        for (sends, dt, direct) in frames {
            let mut queued = 0;
            for (system, m) in sends {
                let r = if system {
                    host.send_system(0, &m).map(Some)
                } else {
                    host.send_game(0, &m)
                };
                match r {
                    Ok(Some(Classified::Queued(_))) => queued += 1,
                    Ok(_) => {}
                    Err(SendError::TooLarge(n)) => prop_assert!(n > MAX_MESSAGE),
                    Err(SendError::GameTooLarge(n)) => prop_assert!(n >= MAX_GAME_SEND),
                }
            }
            if let Some(d) = direct {
                let _ = host.send_direct(0, &d);
            }
            host.clock.0 = host.clock.0.wrapping_add(dt);
            let report = host.frame().expect("a connected client's frame");
            prop_assert_eq!(report.messages.len(), queued);
            for h in &report.messages {
                if let Handled::Game(Outcome::Dispatched(_)) = h.handled {
                    prop_assert!(h.id < 0x67);
                }
            }
            for msg in host.receive(0) {
                prop_assert!(!msg.is_empty() && msg.len() <= MAX_MESSAGE);
                prop_assert!(msg[0] < server_ids() || msg[0] == envelope::SERVER_ID);
            }
        }
    }
}
