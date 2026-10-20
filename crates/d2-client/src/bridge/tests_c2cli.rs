//! The multiplayer-only S→C ids are out of scope: each runs the shared
//! no-op handler and changes nothing.

use d2_proto::schema::SizeRule;

use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use super::world::ClientWorld;
use super::Bridge;

/// A link that sends nothing and delivers nothing.
struct Idle;

impl ServerLink for Idle {
    fn protocol_version(&self) -> u32 {
        d2_proto::PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
        unreachable!("no intent is sent")
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Ok(Pumped { ticked: false })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

#[test]
fn multiplayer_only_ids_are_no_ops() {
    for id in [0x7Fu8, 0x8B, 0x8C, 0x8D, 0x90] {
        let m = d2_proto::transport::server_message(id).unwrap();
        let SizeRule::Fixed(n) = m.size else {
            panic!("{id:#04X} is fixed size");
        };
        let mut msg = vec![0u8; n as usize];
        msg[0] = id;
        let mut b = Bridge::new(Idle).unwrap();
        let r = b.receive_chunk(&msg).unwrap();
        assert_eq!(
            (r.messages, r.unowned, r.handled),
            (1, 0, 1),
            "id {id:#04X}"
        );
        assert_eq!(b.world(), &ClientWorld::default());
    }
}

#[test]
fn download_save_is_a_no_op() {
    // 0xB3: size = u8@1 + 7; the save chunk is not written anywhere.
    let msg = [0xB3u8, 1, 0, 0, 0, 0, 0, 0];
    let mut b = Bridge::new(Idle).unwrap();
    let r = b.receive_chunk(&msg).unwrap();
    assert_eq!((r.messages, r.unowned, r.handled), (1, 0, 1));
    assert_eq!(b.world(), &ClientWorld::default());
}
