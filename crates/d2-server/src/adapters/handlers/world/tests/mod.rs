//! The world handlers through the real host frame (drain → tick →
//! flush) on the real `d2-proto` sizes.
//! Waypoints run on the wired `ActionSim` ([`waypoints`]); the quests
//! also run on the wired `WiredWorld` ([`trade_quests`]). The NPC,
//! vendor and quest tests in [`npc`], [`vendors`], [`quests`] run the
//! real `d2-sim` modules on a seam fake ([`fake`]).

mod fake;
mod gaps;
mod hirelings;
mod ids;
mod npc;
mod objects;
mod quest_objects;
mod quests;
mod quests_act1;
mod quests_act2;
mod quests_act5;
pub(crate) mod trade_quests;
mod vendors;
pub(crate) mod waypoints;

use crate::adapters::ProtoSizes;
use crate::dispatch::Outcome;
use crate::host::{Handled, Host};
use crate::seams::{ClientId, Clock, Intents, MessageSink, ResultCode, SessionHandler, Tick};

/// The C→S message table, `tables/sim/client-messages.tsv`.
pub const CLIENT_TSV: &str = include_str!("../../../../../../../tables/sim/client-messages.tsv");

/// Hex bytes, whitespace ignored.
pub fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// System messages are not part of these tests.
#[derive(Default)]
pub struct NoSession;

impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

/// A manual clock in milliseconds.
pub struct Ms(pub u32);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

/// The host the world tests drive.
pub type TestHost<G> = Host<G, ProtoSizes, NoSession, Ms>;

/// A host for `game` with client 0 connected; the first frame (no tick)
/// has run.
pub fn host<G: Intents + Tick>(game: G) -> TestHost<G> {
    let mut h = Host::new(game, ProtoSizes, NoSession, Ms(1000));
    h.connect(0);
    h.frame().unwrap();
    h
}

/// One message from client 0 through a full host frame: drained and
/// handled, one tick, flushed. Returns the result code and what client 0
/// received, in order.
pub fn send<G: Intents + Tick>(h: &mut TestHost<G>, msg: &[u8]) -> (ResultCode, Vec<Vec<u8>>) {
    let sent = h.send_game(0, msg).unwrap();
    assert!(sent.is_some(), "dropped by the duplicate filter (§2.1)");
    h.clock.0 += 40;
    let r = h.frame().unwrap();
    assert!(r.ticked);
    assert_eq!(r.messages.len(), 1, "{:?}", r.messages);
    let Handled::Game(Outcome::Dispatched(code)) = r.messages[0].handled else {
        panic!("not dispatched: {:?}", r.messages[0]);
    };
    (code, h.receive(0))
}
