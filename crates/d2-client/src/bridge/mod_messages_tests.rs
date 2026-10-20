//! Mod messages through the bridge: a mod game's wire sends C→S envelopes
//! and hands S→C envelopes to its hook; D2RUST's own wire refuses both.

use std::sync::{Arc, Mutex};

use d2_proto::envelope::{Wire, CLIENT_ID, SERVER_ID};
use d2_proto::PROTOCOL_VERSION;

use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use super::world::ClientWorld;
use super::{Bridge, ModMessages};

/// The sends a [`Link`] saw: queue and bytes.
type Sends = Arc<Mutex<Vec<(SendQueue, Vec<u8>)>>>;

/// A link that records sends and delivers one chunk list per pump.
#[derive(Clone, Default)]
struct Link {
    sent: Sends,
    chunks: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerLink for Link {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.lock().unwrap().push((queue, msg.to_vec()));
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Ok(Pumped { ticked: false })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.chunks.lock().unwrap())
    }
}

/// A mod hook that keeps the payloads it is handed.
struct Seen(Arc<Mutex<Vec<Vec<u8>>>>);

impl ModMessages for Seen {
    fn message(&mut self, _world: &mut ClientWorld, payload: &[u8]) {
        self.0.lock().unwrap().push(payload.to_vec());
    }
}

#[test]
fn a_mod_hook_sends_and_receives_envelopes() {
    let link = Link::default();
    let mut b = Bridge::new(link.clone()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    b.set_mod_messages(Box::new(Seen(seen.clone())));
    assert_eq!(b.wire(), Wire::Mod);

    b.send_mod(&[1, 2, 3]).unwrap();
    assert_eq!(
        *link.sent.lock().unwrap(),
        [(SendQueue::Game, vec![CLIENT_ID, 3, 0, 1, 2, 3])]
    );

    // A chunk with a D2RUST message between two envelopes: the envelopes'
    // payloads reach the hook in order, the D2RUST message its handler.
    let chunk = [SERVER_ID, 1, 0, 9, 0x1A, 0x07, SERVER_ID, 0, 0];
    let report = b.receive_chunk(&chunk).unwrap();
    assert_eq!(report.messages, 3);
    assert_eq!(*seen.lock().unwrap(), [vec![9], vec![]]);

    // The frame loop hands them over the same way.
    link.chunks
        .lock()
        .unwrap()
        .push(vec![SERVER_ID, 2, 0, 4, 5]);
    b.frame().unwrap();
    assert_eq!(seen.lock().unwrap().last(), Some(&vec![4, 5]));
}

#[test]
fn d2rusts_own_wire_refuses_envelopes_both_ways() {
    let link = Link::default();
    let mut b = Bridge::new(link.clone()).unwrap();
    assert_eq!(b.wire(), Wire::Original);
    assert!(b.send_mod(&[1]).is_err());
    assert!(link.sent.lock().unwrap().is_empty());
    // S→C: the size rule refuses the id, so the split discards the
    // envelope's bytes and nothing in it is dispatched.
    let report = b.receive_chunk(&[SERVER_ID, 1, 0, 9]).unwrap();
    assert_eq!(report.messages, 0);
    assert_eq!(report.discarded_bytes, 4);
}
