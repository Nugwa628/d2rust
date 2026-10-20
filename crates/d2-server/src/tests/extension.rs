//! Tests of the extension channels (`crate::extension`): a mod's messages
//! reach the extension that owns their channel at the start of the next
//! frame, in client and arrival order; answers go out before the frame
//! mark; unknown channels are dropped and listed; leaving clients are
//! announced. The transport is the in-memory pair.

use std::sync::{Arc, Mutex};

use d2_net::{pair, MemoryEnd, Packet, Transport};

use super::fakes::*;
use crate::extension::{ExtError, ExtOut, Extension, Extensions};
use crate::host::Host;
use crate::hub::Hub;
use crate::seams::ClientId;

type TestHub = Hub<FakeGame, TsvSizes, FakeSession, ManualClock, MemoryEnd>;

/// What the test extension saw, shared with the test.
#[derive(Default)]
struct Seen {
    msgs: Vec<(ClientId, u16, Vec<u8>, i32)>,
    ticks: u32,
    left: Vec<ClientId>,
}

/// Echoes each message back to its sender with the game frame, and
/// broadcasts a tick count after every tick.
struct Echo {
    channels: Vec<u16>,
    seen: Arc<Mutex<Seen>>,
}

impl Extension<FakeGame> for Echo {
    fn channels(&self) -> Vec<u16> {
        self.channels.clone()
    }
    fn handle(
        &mut self,
        game: &mut FakeGame,
        client: ClientId,
        channel: u16,
        msg: &[u8],
        out: &mut ExtOut,
    ) {
        self.seen
            .lock()
            .unwrap()
            .msgs
            .push((client, channel, msg.to_vec(), game.frame));
        out.send(client, channel, msg.iter().rev().copied().collect());
    }
    fn after_tick(&mut self, _: &mut FakeGame, out: &mut ExtOut) {
        let mut s = self.seen.lock().unwrap();
        s.ticks += 1;
        out.broadcast(self.channels[0], vec![s.ticks as u8]);
    }
    fn client_left(&mut self, _: &mut FakeGame, client: ClientId) {
        self.seen.lock().unwrap().left.push(client);
    }
}

fn echo(channels: &[u16]) -> (Box<Echo>, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    (
        Box::new(Echo {
            channels: channels.to_vec(),
            seen: seen.clone(),
        }),
        seen,
    )
}

fn hub() -> (TestHub, MemoryEnd, MemoryEnd) {
    let mut g = FakeGame::with_player(0, ALIVE);
    let p = g.players[&0].clone();
    g.players.insert(1, p);
    g.client_list = vec![0, 1];
    let mut hub = Hub::new(Host::new(
        g,
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    ));
    let (c0, s0) = pair();
    let (c1, s1) = pair();
    hub.connect(0, s0);
    hub.connect(1, s1);
    (hub, c0, c1)
}

fn ext(end: &mut MemoryEnd, channel: u16, msg: &[u8]) {
    end.send(Packet::Ext {
        channel,
        msg: msg.to_vec(),
    })
    .unwrap();
}

fn drain(end: &mut MemoryEnd) -> Vec<Packet> {
    std::iter::from_fn(|| end.recv().unwrap()).collect()
}

#[test]
fn a_channel_has_one_owner() {
    let mut e: Extensions<FakeGame> = Extensions::default();
    assert!(e.is_empty());
    e.register(echo(&[5, 6]).0).unwrap();
    assert_eq!(e.register(echo(&[7, 6]).0), Err(ExtError::ChannelTaken(6)));
    assert_eq!(e.register(echo(&[8, 8]).0), Err(ExtError::ChannelTaken(8)));
    // The failed registrations took nothing: 7 and 8 are still free.
    e.register(echo(&[7, 8]).0).unwrap();
}

#[test]
fn messages_reach_their_extension_in_client_then_arrival_order() {
    let (mut hub, mut c0, mut c1) = hub();
    let (e, seen) = echo(&[40]);
    hub.extensions.register(e).unwrap();
    // Client 1 sends first, but client 0 is read first.
    ext(&mut c1, 40, &[1, 1]);
    ext(&mut c0, 40, &[0, 1]);
    ext(&mut c0, 40, &[0, 2]);
    hub.poll().unwrap();
    // Nothing runs before the frame.
    assert!(seen.lock().unwrap().msgs.is_empty());
    hub.frame().unwrap();
    let got: Vec<(ClientId, Vec<u8>)> = seen
        .lock()
        .unwrap()
        .msgs
        .iter()
        .map(|(c, _, m, _)| (*c, m.clone()))
        .collect();
    assert_eq!(got, [(0, vec![0, 1]), (0, vec![0, 2]), (1, vec![1, 1])]);
    // Each answer goes to its sender, before the frame mark.
    assert_eq!(
        drain(&mut c0),
        [
            Packet::Ext {
                channel: 40,
                msg: vec![1, 0]
            },
            Packet::Ext {
                channel: 40,
                msg: vec![2, 0]
            },
            Packet::Frame { ticked: false },
        ]
    );
    assert_eq!(
        drain(&mut c1),
        [
            Packet::Ext {
                channel: 40,
                msg: vec![1, 1]
            },
            Packet::Frame { ticked: false },
        ]
    );
}

#[test]
fn a_tick_lets_the_extension_broadcast() {
    let (mut hub, mut c0, mut c1) = hub();
    let (e, seen) = echo(&[3]);
    hub.extensions.register(e).unwrap();
    // The first frame starts the clock and runs no tick.
    hub.frame().unwrap();
    assert_eq!(seen.lock().unwrap().ticks, 0);
    hub.host.clock.0 = 1040;
    hub.frame().unwrap();
    assert_eq!(seen.lock().unwrap().ticks, 1);
    for end in [&mut c0, &mut c1] {
        assert_eq!(
            drain(end),
            [
                Packet::Frame { ticked: false },
                Packet::Ext {
                    channel: 3,
                    msg: vec![1]
                },
                Packet::Frame { ticked: true },
            ]
        );
    }
}

#[test]
fn unknown_channels_are_dropped_and_listed() {
    // D2RUST's own game: no extension at all.
    let (mut hub, mut c0, _c1) = hub();
    ext(&mut c0, 9, &[1]);
    hub.poll().unwrap();
    hub.frame().unwrap();
    assert_eq!(hub.take_unrouted(), [(0, 9)]);
    assert_eq!(hub.take_unrouted(), []);
    assert_eq!(drain(&mut c0), [Packet::Frame { ticked: false }]);
    // The connection stays: extension messages are not server-only.
    assert_eq!(hub.clients().collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn a_leaving_client_is_announced_and_its_messages_dropped() {
    let (mut hub, mut c0, _c1) = hub();
    let (e, seen) = echo(&[40]);
    hub.extensions.register(e).unwrap();
    ext(&mut c0, 40, &[7]);
    hub.poll().unwrap();
    hub.disconnect(0);
    hub.frame().unwrap();
    let s = seen.lock().unwrap();
    assert_eq!(s.left, [0]);
    assert!(s.msgs.is_empty());
}
