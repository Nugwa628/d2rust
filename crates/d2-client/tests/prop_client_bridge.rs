//! Property tests on the bridge's untrusted paths, beside the no-panic
//! and delivery properties of `prop_bridge.rs` (fuzz-server): arbitrary S→C chunks
//! through the receive path and dispatch (split, unowned ids, handler
//! rejections, discarded bytes, refused chunks), the frame loop over a
//! link that returns arbitrary chunks, the protocol version check, and
//! arbitrary C→S bytes through the send path on the in-process host
//! (classifier and the link's duplicate filter).
//!
//! The expected results come from the spec's rules applied to
//! `d2_proto::transport::split_server_buffer` / `classify_client` (the
//! size rules the spec names), never from the bridge's own code.

use crate::prop_support;

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::bridge::dispatch::{Dispatch, HandlerError, Message, IDS};
use d2_client::bridge::intent::{self, IntentError};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::local::{LocalLink, PendingSession};
use d2_client::bridge::world::{addressed_unit, ClientWorld};
use d2_client::bridge::{Bridge, BridgeError, ClientUnit, LOCAL_CLIENT};
use d2_proto::schema::Size;
use d2_proto::transport::{client_size, server_size, split_server_buffer};
use d2_proto::PROTOCOL_VERSION;
use d2_server::adapters::ProtoSizes;
use d2_server::host::Host;
use d2_server::seams::{
    ClientId, Clock, Intents, MessageSink, PlayerLookup, PointState, ResultCode, Tick, UnitTarget,
};
use d2_server::transport::duplicate_window;
use proptest::prelude::*;

use prop_support::{bounded, config};

// ---------------------------------------------------------------------------
// Receive path

/// Test handler: checks what the bridge hands it (the message's own id
/// and bytes, and the unit it addresses), then rejects messages whose
/// length is a multiple of 3 (a handler rejection) and otherwise toggles
/// the addressed unit in the model.
fn track(world: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    if msg.bytes.first() != Some(&msg.id) {
        return Err(HandlerError::Invalid("id is not the first byte"));
    }
    if msg.unit != addressed_unit(msg.bytes) {
        return Err(HandlerError::Invalid("unit is not the addressed unit"));
    }
    if msg.bytes.len().is_multiple_of(3) {
        return Err(HandlerError::Invalid("rejected by the fixture"));
    }
    if let Some(key) = msg.unit {
        if world.units.remove(&key).is_none() {
            world.units.insert(key, ClientUnit::new(key));
        }
    }
    Ok(())
}

fn dispatch(owned: &[u8]) -> Dispatch {
    let mut d = Dispatch::empty();
    for &id in owned {
        if usize::from(id) < IDS {
            d.set(id, "specs/client/bridge.md", track);
        }
    }
    d
}

/// What the receive path must do with `chunks`, computed from
/// the split alone: the model, unowned counts per id, handled, rejected,
/// discarded (first byte, count) per chunk, and the index of the first
/// refused chunk.
#[derive(Debug, Default, PartialEq, Eq)]
struct Expected {
    world: ClientWorld,
    handled: u64,
    unowned: BTreeMap<u8, u64>,
    rejected: Vec<u8>,
    discarded: Vec<(u8, usize)>,
}

fn expect(owned: &[u8], chunk: &[u8], e: &mut Expected) -> bool {
    let Ok(split) = split_server_buffer(chunk) else {
        return false;
    };
    for m in &split.messages {
        let id = m[0];
        if owned.contains(&id) && usize::from(id) < IDS {
            if m.len().is_multiple_of(3) {
                e.rejected.push(id);
            } else {
                e.handled += 1;
                if let Some(key) = addressed_unit(m) {
                    if e.world.units.remove(&key).is_none() {
                        e.world.units.insert(key, ClientUnit::new(key));
                    }
                }
            }
        } else {
            *e.unowned.entry(id).or_default() += 1;
        }
    }
    if let Some(&first) = split.discarded.first() {
        e.discarded.push((first, split.discarded.len()));
    }
    true
}

/// One S→C message the size rule accepts: id, then `fill` cut to the
/// rule's size (the size may depend on the bytes, so it is evaluated on
/// the filled buffer).
fn server_message_bytes(id: u8, fill: &[u8]) -> Option<Vec<u8>> {
    let mut b = vec![id];
    b.extend_from_slice(fill);
    match server_size(&b) {
        Size::Bytes(n) if n >= 1 && n <= b.len() => {
            b.truncate(n);
            Some(b)
        }
        _ => None,
    }
}

/// Chunks: raw bytes (mostly refused or discarded), or a run of messages
/// the size rule accepts with an optional raw tail (mostly dispatched).
fn chunk() -> impl Strategy<Value = Vec<u8>> {
    let raw = proptest::collection::vec(any::<u8>(), 0..64);
    let wellformed = (
        proptest::collection::vec(
            (0u8..0xB7, proptest::collection::vec(any::<u8>(), 0..0x30)),
            0..8,
        ),
        proptest::collection::vec(any::<u8>(), 0..4),
    )
        .prop_map(|(msgs, tail)| {
            let mut out = Vec::new();
            for (id, fill) in msgs {
                if let Some(m) = server_message_bytes(id, &fill) {
                    out.extend_from_slice(&m);
                }
            }
            out.extend_from_slice(&tail);
            out
        });
    prop_oneof![1 => raw, 3 => wellformed]
}

/// Owned ids: a few random ids plus some unit-handler ids, so handlers,
/// unit lookups and rejections all run.
fn owned_ids() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(prop_oneof![any::<u8>(), 0x0Eu8..0x20, 0x67u8..0x6E], 0..24)
}

/// A test link: delivers queued chunk lists one frame at a time.
struct Script {
    version: u32,
    frames: VecDeque<Vec<Vec<u8>>>,
    pending: Vec<Vec<u8>>,
}

impl ServerLink for Script {
    fn protocol_version(&self) -> u32 {
        self.version
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.pending = self.frames.pop_front().unwrap_or_default();
        Ok(Pumped { ticked: true })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending)
    }
}

fn script(frames: Vec<Vec<Vec<u8>>>) -> Script {
    Script {
        version: PROTOCOL_VERSION,
        frames: frames.into(),
        pending: Vec::new(),
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// Arbitrary chunks into `receive_chunk`: a refused chunk changes
    /// nothing; otherwise every split message is handled,
    /// unowned or rejected, in order, and the discarded tail is recorded.
    #[test]
    fn receive_matches_split(owned in owned_ids(), chunks in proptest::collection::vec(chunk(), 0..8)) {
        bounded(move || {
            let mut bridge = Bridge::with_dispatch(script(Vec::new()), dispatch(&owned)).unwrap();
            let mut e = Expected::default();
            for c in &chunks {
                let before_world = bridge.world().clone();
                let before_rejected = bridge.log().rejected.len();
                let before_discarded = bridge.log().discarded.len();
                let accepted = expect(&owned, c, &mut e);
                match bridge.receive_chunk(c) {
                    Ok(r) => {
                        assert!(accepted, "chunk the split refuses was accepted: {c:02X?}");
                        assert_eq!(r.messages, r.handled + r.unowned + r.rejected);
                        assert_eq!(r.discarded_bytes, split_server_buffer(c).unwrap().discarded.len());
                    }
                    Err(_) => {
                        assert!(!accepted, "chunk the split accepts was refused: {c:02X?}");
                        assert_eq!(bridge.world(), &before_world);
                        assert_eq!(bridge.log().rejected.len(), before_rejected);
                        assert_eq!(bridge.log().discarded.len(), before_discarded);
                    }
                }
            }
            let log = bridge.log();
            assert_eq!(bridge.world(), &e.world);
            assert_eq!(log.handled, e.handled);
            assert_eq!(log.unowned, e.unowned);
            assert_eq!(log.rejected.iter().map(|r| r.id).collect::<Vec<_>>(), e.rejected);
            assert_eq!(
                log.discarded.iter().map(|d| (d.first, d.bytes)).collect::<Vec<_>>(),
                e.discarded
            );
        });
    }

    /// The frame loop over a link returning arbitrary chunks: counters
    /// advance once per frame, the report sums its chunks,
    /// and a refused chunk ends the frame with an error after the chunks
    /// before it were applied.
    #[test]
    fn frames_over_arbitrary_chunks(
        owned in owned_ids(),
        frames in proptest::collection::vec(proptest::collection::vec(chunk(), 0..4), 0..6),
    ) {
        bounded(move || {
            let mut bridge = Bridge::with_dispatch(script(frames.clone()), dispatch(&owned)).unwrap();
            let mut e = Expected::default();
            for (n, chunks) in frames.iter().enumerate() {
                let result = bridge.frame();
                let mut refused = false;
                let (mut messages, mut discarded) = (0, 0);
                for c in chunks {
                    if !expect(&owned, c, &mut e) {
                        refused = true;
                        break;
                    }
                    let s = split_server_buffer(c).unwrap();
                    messages += s.messages.len();
                    discarded += s.discarded.len();
                }
                match result {
                    Ok(r) => {
                        assert!(!refused);
                        assert!(r.ticked);
                        assert_eq!(r.chunks, chunks.len());
                        assert_eq!(r.messages, messages);
                        assert_eq!(r.messages, r.handled + r.unowned + r.rejected);
                        assert_eq!(r.discarded_bytes, discarded);
                    }
                    Err(BridgeError::Split(_)) => assert!(refused),
                    Err(other) => panic!("unexpected frame error {other}"),
                }
                assert_eq!(bridge.world().frames, n as u64 + 1);
                assert_eq!(bridge.world().server_ticks, n as u64 + 1);
                assert_eq!(bridge.world().units, e.world.units);
            }
        });
    }

    /// Any protocol version but the client's is refused.
    #[test]
    fn version_check(version in prop_oneof![Just(PROTOCOL_VERSION), any::<u32>()]) {
        let mut link = script(Vec::new());
        link.version = version;
        match Bridge::with_dispatch(link, Dispatch::empty()) {
            Ok(_) => prop_assert_eq!(version, PROTOCOL_VERSION),
            Err(BridgeError::Version { client, server }) => {
                prop_assert_ne!(version, PROTOCOL_VERSION);
                prop_assert_eq!((client, server), (PROTOCOL_VERSION, version));
            }
            Err(other) => panic!("unexpected error {other}"),
        }
    }

    /// `addressed_unit` on arbitrary bytes never panics and follows the
    /// addressing rule (monster ids 0x67–0x6D read a u32 at +1, others type at +1
    /// and u32 at +2; too short → none).
    #[test]
    fn addressed_unit_total(msg in proptest::collection::vec(any::<u8>(), 0..12)) {
        let unit = addressed_unit(&msg);
        let Some(&id) = msg.first() else {
            prop_assert_eq!(unit, None);
            return Ok(());
        };
        let has_handler = d2_proto::transport::server_message(id)
            .is_some_and(|m| m.client_unit_handler.is_some());
        let need = if (0x67..=0x6D).contains(&id) { 5 } else { 6 };
        prop_assert_eq!(unit.is_some(), has_handler && msg.len() >= need);
    }
}

// ---------------------------------------------------------------------------
// Send path on the in-process host

/// A game with no players: every intent is refused by the gate and a tick
/// sends nothing. The send path and the duplicate filter run before it.
struct NoGame;

impl Intents for NoGame {
    fn player(&self, _: ClientId) -> PlayerLookup {
        PlayerLookup::NotInGame
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
    fn handle(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) -> ResultCode {
        ResultCode::Refused
    }
    fn clients(&self) -> Vec<ClientId> {
        vec![LOCAL_CLIENT]
    }
}

impl Tick for NoGame {
    fn tick(&mut self, _: &mut dyn MessageSink) {}
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
}

/// A manual host clock shared with the test.
#[derive(Clone, Default)]
struct Ms(Arc<AtomicU32>);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

type Local = LocalLink<NoGame, ProtoSizes, PendingSession, Ms>;

fn local() -> (Bridge<Local>, Ms) {
    let clock = Ms::default();
    let host = Host::new(NoGame, ProtoSizes, PendingSession::default(), clock.clone());
    (Bridge::new(LocalLink::new(host)).unwrap(), clock)
}

/// A C→S message the classifier accepts: id, then `fill` cut to the
/// size rule's size.
fn client_message_bytes(id: u8, fill: &[u8]) -> Option<Vec<u8>> {
    let mut b = vec![id];
    b.extend_from_slice(fill);
    match client_size(&b) {
        Size::Bytes(n) if n >= 1 && n <= b.len() => {
            b.truncate(n);
            Some(b)
        }
        _ => None,
    }
}

/// One send: either arbitrary bytes or one of a small pool of valid
/// messages (so repeats happen), after `dt` ms.
#[derive(Debug, Clone)]
enum SendOp {
    Raw(Vec<u8>),
    Pool(usize),
}

/// A pool of valid messages and the sends: (op, ms before it, pump after).
type SendPlan = (Vec<Vec<u8>>, Vec<(SendOp, u32, bool)>);

fn send_ops() -> impl Strategy<Value = SendPlan> {
    let pool = proptest::collection::vec(
        (
            prop_oneof![0u8..0x67, 0x67u8..0x71, Just(0xFFu8)],
            proptest::collection::vec(any::<u8>(), 0..0x20),
        ),
        1..4,
    )
    .prop_map(|v| {
        v.into_iter()
            .filter_map(|(id, fill)| client_message_bytes(id, &fill))
            .collect::<Vec<_>>()
    });
    let op = prop_oneof![
        1 => proptest::collection::vec(any::<u8>(), 0..0x220).prop_map(SendOp::Raw),
        4 => (0usize..4).prop_map(SendOp::Pool),
    ];
    let dt = prop_oneof![Just(0u32), 0u32..60, 0u32..400, any::<u32>()];
    (
        pool,
        proptest::collection::vec((op, dt, proptest::bool::weighted(0.2)), 0..24),
    )
}

proptest! {
    #![proptest_config(config(128))]

    /// Arbitrary C→S bytes: refused exactly when the classifier refuses
    /// them, the admin queue or a game message of 0x200+ bytes; a
    /// sendable message is queued or filtered, and filtered exactly when
    /// the game's sender's duplicate rule says so (same bytes over the new length
    /// of a store each send overwrites for its own length, within the
    /// id's window). Pumping between sends never fails.
    #[test]
    fn send_path_and_duplicate_filter((pool, ops) in send_ops()) {
        bounded(move || {
            let (mut bridge, clock) = local();
            let mut stored = [0u8; 0x200];
            let mut at = 0u32;
            for (op, dt, pump) in ops {
                let now = clock.0.load(Ordering::Relaxed).wrapping_add(dt);
                clock.0.store(now, Ordering::Relaxed);
                let msg = match op {
                    SendOp::Raw(b) => b,
                    SendOp::Pool(i) if !pool.is_empty() => pool[i % pool.len()].clone(),
                    SendOp::Pool(_) => Vec::new(),
                };
                let routed = intent::route(&msg);
                // Nothing the net send asserts on (> 0x204) is routed.
                if msg.len() > d2_proto::transport::MAX_MESSAGE {
                    assert!(routed.is_err());
                }
                let got = bridge.send_bytes(&msg);
                match (routed, got) {
                    (Err(want), Err(BridgeError::Intent(e))) => assert_eq!(e, want),
                    (Err(want), other) => panic!("route refused {msg:02X?} ({want}) but send gave {other:?}"),
                    (Ok(SendQueue::System), Ok(s)) => assert_eq!(s, Sent::Queued),
                    (Ok(SendQueue::Game), Ok(s)) => {
                        let filtered = duplicate_window(msg[0]).is_some_and(|w| {
                            stored[..msg.len()] == msg[..] && now.wrapping_sub(at) < w
                        });
                        if filtered {
                            assert_eq!(s, Sent::Filtered, "{msg:02X?} at {now}");
                        } else {
                            assert_eq!(s, Sent::Queued, "{msg:02X?} at {now}");
                            stored[..msg.len()].copy_from_slice(&msg);
                            at = now;
                        }
                    }
                    (Ok(q), Err(e)) => panic!("routable {msg:02X?} ({q:?}) failed: {e}"),
                }
                if let Err(IntentError::GameTooLarge(n)) = intent::route(&msg) {
                    assert!(n >= intent::MAX_GAME_SEND);
                }
                if pump {
                    bridge.frame().expect("pump over a game with no players");
                }
            }
        });
    }
}
