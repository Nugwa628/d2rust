//! The game stream: one game's packets between a client and the server.
//!
//! A client opens it after the lobby let it join (`JoinOk` and its
//! ticket), attaches with the ticket, and from the server's `Welcome` on it
//! carries the game exactly as the in-process link does: the client's
//! intents up, the server's deliveries and frame marks down. Leaving the
//! game ends the stream; the session stays in the lobby.
//!
//! The server writes each of its frames as one piece: the frame mark, the
//! frame's deliveries, then `FRAME_END`. [`GameLink::poll_frame`] hands out
//! whole frames only, so the client applies a server frame as a unit, as it
//! does in single player.

use std::collections::VecDeque;

use d2_net::wire::{kind, packet_body, parse_body, Body};
use d2_net::{Lane, NetError, Packet, Transport, Welcome};
use d2_quic::FramedStream;

use crate::OnlineError;

/// One server frame as the client applies it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerFrame {
    /// A game tick ran in this frame (the client's update pass runs only
    /// after a tick).
    pub ticked: bool,
    /// The S→C messages of the frame, in delivery order.
    pub delivers: Vec<Vec<u8>>,
    /// Mod extension messages of the frame: (channel, bytes).
    pub ext: Vec<(u16, Vec<u8>)>,
}

/// Builds whole server frames out of game-stream bodies.
///
/// A frame opens with its `FRAME` mark and closes at `FRAME_END`. A
/// resumed connection's replay can carry several marks before one end:
/// each new mark closes the frame before it. Deliveries with no open frame
/// (before the first mark) form an unticked frame of their own.
#[derive(Debug, Default)]
pub struct FrameAssembler {
    open: Option<ServerFrame>,
}

impl FrameAssembler {
    /// Takes one body; frames it completes go to `done`, in order.
    pub fn push(&mut self, body: &[u8], done: &mut VecDeque<ServerFrame>) -> Result<(), NetError> {
        if body == [kind::FRAME_END] {
            if let Some(f) = self.open.take() {
                done.push_back(f);
            }
            return Ok(());
        }
        match parse_body(body)? {
            Body::Packet(Packet::Frame { ticked }) => {
                if let Some(f) = self.open.take() {
                    done.push_back(f);
                }
                self.open = Some(ServerFrame {
                    ticked,
                    ..ServerFrame::default()
                });
            }
            Body::Packet(Packet::Deliver(msg)) => {
                self.open
                    .get_or_insert_with(ServerFrame::default)
                    .delivers
                    .push(msg);
            }
            Body::Packet(Packet::Ext { channel, msg }) => {
                self.open
                    .get_or_insert_with(ServerFrame::default)
                    .ext
                    .push((channel, msg));
            }
            // The server never sends a client message; pings and goodbyes
            // never reach here (the stream eats them).
            Body::Packet(Packet::Send { .. }) | Body::Ping | Body::Bye => {
                return Err(NetError::Malformed)
            }
        }
        Ok(())
    }
}

/// A client's game stream after the server welcomed it into the game.
///
/// The game reads whole frames ([`GameLink::poll_frame`]); a test bot can
/// read it as a [`Transport`] instead: each frame's mark, then its
/// deliveries and extension messages, in order. Use one way per link.
#[derive(Debug)]
pub struct GameLink {
    stream: FramedStream,
    welcome: Welcome,
    frames: FrameAssembler,
    ready: VecDeque<ServerFrame>,
    /// The packets of frames already taken by [`Transport::recv`].
    packets: VecDeque<Packet>,
}

impl GameLink {
    /// A game stream the server has welcomed with `welcome`.
    pub fn attached(stream: FramedStream, welcome: Welcome) -> Self {
        Self {
            stream,
            welcome,
            frames: FrameAssembler::default(),
            ready: VecDeque::new(),
            packets: VecDeque::new(),
        }
    }

    /// The server's welcome into the game: the client's id, the tick
    /// rate, and the token that resumes this game slot.
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// Sends one C→S game message for the server's `lane` sender.
    pub fn send(&mut self, lane: Lane, msg: &[u8]) -> Result<(), OnlineError> {
        let body = packet_body(&Packet::Send {
            lane,
            msg: msg.to_vec(),
        });
        self.stream.send(body).map_err(OnlineError::Lost)
    }

    /// Sends one mod extension message on `channel`.
    pub fn send_ext(&mut self, channel: u16, msg: &[u8]) -> Result<(), OnlineError> {
        let body = packet_body(&Packet::Ext {
            channel,
            msg: msg.to_vec(),
        });
        self.stream.send(body).map_err(OnlineError::Lost)
    }

    /// The next whole server frame that has arrived, or `None`.
    pub fn poll_frame(&mut self) -> Result<Option<ServerFrame>, OnlineError> {
        if self.ready.is_empty() {
            while let Some(body) = self.stream.recv().map_err(OnlineError::Lost)? {
                self.frames
                    .push(&body, &mut self.ready)
                    .map_err(|_| OnlineError::Malformed)?;
            }
        }
        Ok(self.ready.pop_front())
    }

    /// Leaves the game: ends the game stream. The session stays open.
    pub fn leave(mut self) {
        self.stream.close();
    }
}

impl Transport for GameLink {
    /// Sends a client packet ([`Packet::Send`] or [`Packet::Ext`]); the
    /// server's packet kinds are refused as [`NetError::Malformed`].
    fn send(&mut self, packet: Packet) -> Result<(), NetError> {
        match packet {
            Packet::Send { .. } | Packet::Ext { .. } => self.stream.send(packet_body(&packet)),
            Packet::Deliver(_) | Packet::Frame { .. } => Err(NetError::Malformed),
        }
    }

    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        if self.packets.is_empty() {
            let frame = match self.poll_frame() {
                Ok(f) => f,
                Err(OnlineError::Lost(e)) => return Err(e),
                Err(_) => return Err(NetError::Malformed),
            };
            if let Some(f) = frame {
                self.packets.push_back(Packet::Frame { ticked: f.ticked });
                self.packets
                    .extend(f.delivers.into_iter().map(Packet::Deliver));
                self.packets.extend(
                    f.ext
                        .into_iter()
                        .map(|(channel, msg)| Packet::Ext { channel, msg }),
                );
            }
        }
        Ok(self.packets.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(p: Packet) -> Vec<u8> {
        packet_body(&p)
    }

    fn feed(a: &mut FrameAssembler, bodies: &[Vec<u8>]) -> Vec<ServerFrame> {
        let mut done = VecDeque::new();
        for b in bodies {
            a.push(b, &mut done).unwrap();
        }
        done.into()
    }

    #[test]
    fn a_frame_is_handed_out_at_its_end() {
        let mut a = FrameAssembler::default();
        let open = feed(
            &mut a,
            &[
                body(Packet::Frame { ticked: true }),
                body(Packet::Deliver(vec![0x95, 1])),
                body(Packet::Ext {
                    channel: 3,
                    msg: vec![7],
                }),
                body(Packet::Deliver(vec![0x0C, 2])),
            ],
        );
        assert!(open.is_empty(), "nothing before FRAME_END");
        let done = feed(&mut a, &[vec![kind::FRAME_END]]);
        assert_eq!(
            done,
            [ServerFrame {
                ticked: true,
                delivers: vec![vec![0x95, 1], vec![0x0C, 2]],
                ext: vec![(3, vec![7])],
            }]
        );
    }

    #[test]
    fn a_replay_with_several_marks_makes_one_frame_per_mark() {
        let mut a = FrameAssembler::default();
        let done = feed(
            &mut a,
            &[
                body(Packet::Deliver(vec![1])),
                body(Packet::Frame { ticked: false }),
                body(Packet::Frame { ticked: true }),
                body(Packet::Deliver(vec![2])),
                vec![kind::FRAME_END],
                vec![kind::FRAME_END],
            ],
        );
        assert_eq!(
            done,
            [
                ServerFrame {
                    ticked: false,
                    delivers: vec![vec![1]],
                    ext: vec![],
                },
                ServerFrame::default(),
                ServerFrame {
                    ticked: true,
                    delivers: vec![vec![2]],
                    ext: vec![],
                },
            ]
        );
    }

    #[test]
    fn a_client_message_from_the_server_is_malformed() {
        let mut a = FrameAssembler::default();
        let mut done = VecDeque::new();
        let up = body(Packet::Send {
            lane: Lane::Game,
            msg: vec![1],
        });
        assert_eq!(a.push(&up, &mut done), Err(NetError::Malformed));
        assert_eq!(a.push(&[0xEE], &mut done), Err(NetError::Malformed));
    }
}
