//! The in-memory transport: two connected ends in one process, for single
//! player and for in-process tests with several clients.
//!
//! Each direction is a channel, so a packet is readable on the other end
//! the moment it is sent, in send order. The ends are `Send`: the server
//! may live on its own thread (the client's server thread does).

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crate::{NetError, Packet, Transport};

/// One end of an in-memory connection (see [`pair`]).
#[derive(Debug)]
pub struct MemoryEnd {
    tx: Sender<Packet>,
    rx: Receiver<Packet>,
}

/// Two connected ends: what one sends, the other receives. By convention
/// the first is the client's end and the second the server's.
pub fn pair() -> (MemoryEnd, MemoryEnd) {
    let (a_tx, b_rx) = mpsc::channel();
    let (b_tx, a_rx) = mpsc::channel();
    (
        MemoryEnd { tx: a_tx, rx: a_rx },
        MemoryEnd { tx: b_tx, rx: b_rx },
    )
}

impl Transport for MemoryEnd {
    fn send(&mut self, packet: Packet) -> Result<(), NetError> {
        self.tx.send(packet).map_err(|_| NetError::Closed)
    }

    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        match self.rx.try_recv() {
            Ok(p) => Ok(Some(p)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(NetError::Closed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lane;

    /// Packets cross in send order, each direction on its own.
    #[test]
    fn ends_carry_packets_in_order() {
        let (mut client, mut server) = pair();
        let a = Packet::Send {
            lane: Lane::Game,
            msg: vec![0x01, 2, 0, 3, 0],
        };
        let b = Packet::Send {
            lane: Lane::System,
            msg: vec![0x6B],
        };
        client.send(a.clone()).unwrap();
        client.send(b.clone()).unwrap();
        server.send(Packet::Frame { ticked: true }).unwrap();
        assert_eq!(server.recv(), Ok(Some(a)));
        assert_eq!(server.recv(), Ok(Some(b)));
        assert_eq!(server.recv(), Ok(None));
        assert_eq!(client.recv(), Ok(Some(Packet::Frame { ticked: true })));
        assert_eq!(client.recv(), Ok(None));
    }

    /// A dropped end closes the connection: sends fail, and the other end
    /// reads what was already sent before it sees the close.
    #[test]
    fn dropped_end_closes() {
        let (mut client, server) = pair();
        let mut server = Some(server);
        server
            .as_mut()
            .unwrap()
            .send(Packet::Deliver(vec![0xAF, 0]))
            .unwrap();
        drop(server.take());
        assert_eq!(client.recv(), Ok(Some(Packet::Deliver(vec![0xAF, 0]))));
        assert_eq!(client.recv(), Err(NetError::Closed));
        assert_eq!(
            client.send(Packet::Frame { ticked: false }),
            Err(NetError::Closed)
        );
    }
}
