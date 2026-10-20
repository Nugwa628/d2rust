//! Robustness properties of the
//! bridge on its in-process server link ([`LocalLink`]): any C→S bytes go
//! through the bridge's classifier check, the link and the host without
//! a panic, and a message the bridge sends arrives at the server's
//! drain byte for byte; any S→C bytes the server delivers (buffered or
//! direct) go through the receive path and the handlers without a panic.
//!
//! The game behind the host is `SimGame` with a joined player and no
//! wiring: every intent ends in a stub, which is all the transport round
//! trip needs (the wired handlers are `d2-server`'s `prop_handle.rs`).
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder (it overrides every default here).

use d2_client::bridge::dispatch::Dispatch;
use d2_client::bridge::intent::{route, IntentError, MAX_GAME_SEND};
use d2_client::bridge::link::{LinkError, SendQueue, Sent, ServerLink};
use d2_client::bridge::local::{LocalError, LocalLink, PendingSession};
use d2_client::bridge::{Bridge, BridgeError, LOCAL_CLIENT};
use d2_proto::envelope;
use d2_proto::transport::{
    classify_client, split_server_buffer, Classified, ClientQueue, SplitError, MAX_MESSAGE,
};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::buffers::server_ids;
use d2_server::host::{Handled, Host};
use d2_server::seams::{Clock, PlayerGate, Pos};
use d2_server::transport::DRAIN_COPY;
use d2_sim::game::Game;
use d2_sim::units::lists::client_state;
use d2_sim::units::UnitType;
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

struct Ms(u32);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type Link = LocalLink<SimGame, ProtoSizes, PendingSession, Ms>;

/// A host with the local client's player joined, alive, at (100, 100).
fn link() -> Link {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let player = game.spawn_unit(UnitType::Player, None, true).unwrap();
    let mut sim = SimGame::new(game);
    sim.join(LOCAL_CLIENT, Some(player), None, client_state::IN_GAME)
        .unwrap();
    sim.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode: 1,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    sim.set_unit(
        player,
        UnitFacts {
            act: 0,
            pos: Pos { x: 100, y: 100 },
            owner: None,
        },
    );
    LocalLink::new(Host::new(
        sim,
        ProtoSizes,
        PendingSession::default(),
        Ms(1000),
    ))
}

/// A C→S-shaped input: an id (mostly sendable), then `0..max` bytes, or
/// exactly its fixed size.
fn message(max: usize) -> impl Strategy<Value = Vec<u8>> {
    let id = prop_oneof![3 => 0u8..0x71, 1 => any::<u8>()];
    let byte = prop_oneof![4 => any::<u8>(), 1 => Just(0u8), 1 => Just(0xFFu8)];
    (id, prop::collection::vec(byte, 0..max), any::<bool>()).prop_map(|(id, rest, exact)| {
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

/// The link's error as the local host's.
fn local(e: &LinkError) -> Option<&LocalError> {
    let LinkError::Server(e) = e;
    e.downcast_ref::<LocalError>()
}

proptest! {
    #![proptest_config(config(128))]

    /// The bridge's send path on any bytes: the bridge
    /// refuses what the classifier refuses, the admin queue and game
    /// messages of 0x200 bytes or more; what it routes, the link queues
    /// or filters, never errs; a frame then succeeds and the server
    /// drains every queued message with its bytes and size.
    #[test]
    fn bridge_send_any(msgs in prop::collection::vec(message(0x240), 1..12)) {
        let mut bridge = Bridge::with_dispatch(link(), Dispatch::from_spec().unwrap()).unwrap();
        let mut queued = Vec::new();
        for m in &msgs {
            let class = classify_client(m);
            match bridge.send_bytes(m) {
                Ok(Sent::Queued) => queued.push(m.clone()),
                Ok(Sent::Filtered) => prop_assert!(m[0] < 0x67, "{:02X?}", m),
                Err(BridgeError::Intent(IntentError::NotSendable(c))) => {
                    prop_assert_eq!(c, class);
                    prop_assert!(!matches!(c, Classified::Queue(_)));
                }
                Err(BridgeError::Intent(IntentError::AdminQueue)) => {
                    prop_assert_eq!(class, Classified::Queue(ClientQueue::Admin));
                }
                Err(BridgeError::Intent(IntentError::GameTooLarge(n))) => {
                    prop_assert!(n >= MAX_GAME_SEND);
                    prop_assert_eq!(class, Classified::Queue(ClientQueue::Game));
                }
                // Found by this test: a system message over 0x204 bytes
                // passed the classifier and failed in the transport.
                Err(BridgeError::Intent(IntentError::TooLarge(n))) => {
                    prop_assert!(n > MAX_MESSAGE);
                    prop_assert_eq!(class, Classified::Queue(ClientQueue::System));
                }
                Err(e) => return Err(TestCaseError::fail(format!("{:02X?}: {e}", m))),
            }
        }
        bridge.link_mut().host_mut().clock.0 += 40;
        // The first frame only starts the tick driver.
        bridge.frame().expect("frame");
        let drained = &bridge.link().last_frame().messages;
        prop_assert_eq!(drained.len(), queued.len());
        // System messages drain before game messages.
        let (sys, game): (Vec<_>, Vec<_>) = queued.iter().partition(|m| m[0] >= 0x67);
        for (h, m) in drained.iter().zip(sys.iter().chain(game.iter())) {
            prop_assert_eq!(h.id, m[0]);
            prop_assert_eq!(h.size, m.len());
            prop_assert_eq!(matches!(h.handled, Handled::System), m[0] >= 0x67);
        }
        let session = &bridge.link().host().session.received;
        for ((_, bytes, size), m) in session.iter().zip(&sys) {
            // The drain copy stops at 0x1FC bytes.
            prop_assert_eq!(&bytes[..], &m[..m.len().min(DRAIN_COPY)]);
            prop_assert_eq!(*size, m.len());
        }
    }

    /// The link itself on any bytes and either queue, past
    /// the bridge's check: a send is queued, filtered, or refused with
    /// the transport's assert or the classifier's drop; never a panic.
    #[test]
    fn link_send_any(msgs in prop::collection::vec((any::<bool>(), message(0x240)), 1..12)) {
        let mut l = link();
        for (system, m) in &msgs {
            let queue = if *system { SendQueue::System } else { SendQueue::Game };
            match l.send(queue, m) {
                Ok(_) => {
                    prop_assert!(m.len() <= MAX_MESSAGE);
                }
                Err(e) => match local(&e) {
                    Some(LocalError::Send(_)) => prop_assert!(m.len() >= MAX_GAME_SEND),
                    Some(LocalError::Dropped { id, class }) => {
                        prop_assert_eq!(*id, m[0]);
                        prop_assert!(!matches!(class, d2_server::transport::Classified::Queued(_)));
                    }
                    other => return Err(TestCaseError::fail(format!("{other:?}"))),
                },
            }
        }
        l.host_mut().clock.0 += 40;
        prop_assert!(l.pump().is_ok());
        let _ = l.receive();
    }

    /// The receive path on any S→C chunk: split and dispatched, or
    /// refused whole with the split's error; the handlers
    /// never panic on any bytes.
    #[test]
    fn receive_any_chunk(chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..0x240), 1..6)) {
        let mut bridge = Bridge::with_dispatch(link(), Dispatch::from_spec().unwrap()).unwrap();
        for c in &chunks {
            match (bridge.receive_chunk(c), split_server_buffer(c)) {
                (Ok(r), Ok(s)) => {
                    prop_assert_eq!(r.messages, s.messages.len());
                    prop_assert_eq!(r.handled + r.queued + r.dropped + r.unowned + r.rejected, r.messages);
                    prop_assert_eq!(r.discarded_bytes, s.discarded.len());
                }
                (Err(BridgeError::Split(e)), Err(w)) => prop_assert_eq!(e, w),
                (r, s) => return Err(TestCaseError::fail(format!("{r:?} vs {s:?}"))),
            }
        }
    }

    /// Well-formed S→C messages (whole, by the size rule) for every id,
    /// so the handlers see their own ids with arbitrary fields.
    #[test]
    fn receive_any_message(id in 0u8..0xB7, body in prop::collection::vec(any::<u8>(), 0x204)) {
        let mut bridge = Bridge::with_dispatch(link(), Dispatch::from_spec().unwrap()).unwrap();
        let mut m = body;
        m[0] = id;
        if let d2_proto::schema::Size::Bytes(n) = d2_proto::transport::server_size(&m) {
            // Some rules read past the size they give (0x16 needs 13
            // bytes for any size): only a message that sizes itself.
            let whole = |m: &[u8]| d2_proto::transport::server_size(m) == d2_proto::schema::Size::Bytes(m.len());
            if n <= MAX_MESSAGE && whole(&m[..n]) {
                m.truncate(n);
                let r = bridge.receive_chunk(&m).expect("a whole message");
                prop_assert_eq!(r.messages, 1);
            }
        }
    }

    /// Direct sends of any bytes reach the client through
    /// the frame: the host refuses what local delivery asserts on, and
    /// the bridge refuses whole a chunk it cannot split.
    #[test]
    fn direct_any(direct in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..0x240), 1..6)) {
        let mut bridge = Bridge::with_dispatch(link(), Dispatch::from_spec().unwrap()).unwrap();
        let mut pushed = Vec::new();
        for d in &direct {
            if bridge.link_mut().host_mut().send_direct(LOCAL_CLIENT, d).is_ok() {
                prop_assert!(
                    !d.is_empty()
                        && d.len() <= MAX_MESSAGE
                        && (d[0] < server_ids() || d[0] == envelope::SERVER_ID)
                );
                pushed.push(d.clone());
            }
        }
        bridge.link_mut().host_mut().clock.0 += 40;
        match bridge.frame() {
            Ok(r) => prop_assert_eq!(r.chunks, pushed.len()),
            Err(BridgeError::Split(SplitError::Truncated { .. } | SplitError::TooLarge { .. })) => {}
            Err(e) => return Err(TestCaseError::fail(format!("{e}"))),
        }
    }
}

/// `route` agrees with the classifier on every id at every length up to
/// one past the transport limit, and refuses what the sender or the
/// transport asserts on.
#[test]
fn route_every_id_and_length() {
    for id in 0..=255u8 {
        for len in 1..=MAX_MESSAGE + 1 {
            let mut m = vec![0u8; len];
            m[0] = id;
            let r = route(&m);
            match classify_client(&m) {
                Classified::Queue(ClientQueue::Admin) => {
                    assert_eq!(r, Err(IntentError::AdminQueue))
                }
                Classified::Queue(ClientQueue::Game) if len >= MAX_GAME_SEND => {
                    assert_eq!(r, Err(IntentError::GameTooLarge(len)))
                }
                Classified::Queue(ClientQueue::Game) => assert_eq!(r, Ok(SendQueue::Game)),
                Classified::Queue(ClientQueue::System) if len > MAX_MESSAGE => {
                    assert_eq!(r, Err(IntentError::TooLarge(len)))
                }
                Classified::Queue(ClientQueue::System) => assert_eq!(r, Ok(SendQueue::System)),
                c => assert_eq!(r, Err(IntentError::NotSendable(c))),
            }
        }
    }
}
