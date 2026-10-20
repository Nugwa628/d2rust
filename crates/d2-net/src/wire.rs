//! The wire format every network transport shares: how packets become
//! frames, and the handshake that opens a connection.
//!
//! - **Frames.** Every unit on the wire is a frame: a 4-byte little-endian
//!   body length, then the body (a kind byte and its fields). A body is at
//!   most [`MAX_FRAME`] bytes; a longer length ends the connection.
//! - **Online sessions** (wire version 4, [`session`]): the control
//!   stream's handshake ([`ClientHello`], [`SessionWelcome`], [`Refused`])
//!   and the game stream's first frame ([`Attach`]).
//! - **Handshake (wire v2 layout).** The client opens with [`Hello`]: the wire magic and
//!   version, the `d2-proto` protocol version, the game build, the tick
//!   rate it expects (a value, so a new tick rate needs no new wire
//!   format), and the resume token of a slot it is coming back to, if any.
//!   The server answers with [`Welcome`] (the client's id, the server's
//!   tick rate, the game's player limit and the client's resume token) or
//!   `Refused` with a [`Refusal`]. In this version the client must match
//!   the server's protocol, build and tick rate exactly.
//! - **Extension frames.** A packet on an extension channel
//!   ([`Packet::Ext`]) is a frame of its own kind: the channel (u16) and
//!   the extension's bytes. The wire format does not read them.
//! - **Control frames.** A ping keeps a quiet connection alive; a goodbye
//!   ends it cleanly, so the other end reads everything sent before it,
//!   then [`NetError::Closed`].
//!
//! The QUIC transport (the `d2-quic` crate) carries it on its stream.

use std::io;
use std::time::Duration;

use crate::{ClientId, Lane, NetError, Packet};

pub mod session;
pub use session::{
    parse_attach_answer, parse_session_answer, Attach, ClientHello, Refused, ResumeState,
    ServerInfo, SessionWelcome,
};

/// The first bytes of every [`Hello`]: says "a D2RUST connection".
pub const MAGIC: [u8; 4] = *b"D2RN";

/// Version of this wire format (framing, handshake and control frames).
/// A change to any of them gets a new number. 4: the online session's
/// control and game streams ([`session`]).
pub const NET_VERSION: u16 = 4;

/// Largest frame body: the largest message plus its kind and lane bytes,
/// with room for the handshake strings.
pub const MAX_FRAME: usize = 0x1000;

/// Longest game build string a handshake carries.
pub const MAX_BUILD: usize = 64;

/// Frame kinds (the first body byte).
pub mod kind {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const REFUSED: u8 = 3;
    pub const SEND: u8 = 4;
    pub const DELIVER: u8 = 5;
    pub const FRAME: u8 = 6;
    pub const PING: u8 = 7;
    pub const BYE: u8 = 8;
    pub const EXT: u8 = 9;
    /// C→S, the first frame of a game stream ([`super::Attach`]).
    pub const ATTACH: u8 = 0x0A;
    /// S→C on a game stream: the server frame's packets are complete
    /// (written by the server end's `Transport::flush`).
    pub const FRAME_END: u8 = 0x0B;
}

/// A secret the server gives each client in its [`Welcome`]. A client
/// whose connection was lost presents it in its next [`Hello`] to get its
/// own player slot back while the server still holds it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResumeToken(pub [u8; 16]);

impl ResumeToken {
    /// A fresh token from the operating system's random source.
    pub fn random() -> Self {
        let mut b = [0; 16];
        getrandom::fill(&mut b).expect("the operating system has a random source");
        Self(b)
    }
}

impl std::fmt::Debug for ResumeToken {
    /// Shows only the first bytes: tokens are secrets, logs aren't.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ResumeToken({:02x}{:02x}..)", self.0[0], self.0[1])
    }
}

/// What a client says when it connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// `d2_proto::PROTOCOL_VERSION` of the client.
    pub protocol: u32,
    /// The client's game build (for example "0.1.1.0").
    pub build: String,
    /// The tick rate the client was built for, in ticks per second.
    pub tick_rate: u32,
    /// The token of the slot this client is coming back to, if it lost
    /// its connection; `None` for a new player.
    pub resume: Option<ResumeToken>,
}

/// The server's acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// The id the server gave this client (the old one on a resume).
    pub client: ClientId,
    /// The server's tick rate, in ticks per second.
    pub tick_rate: u32,
    /// How many players the game takes.
    pub max_players: u32,
    /// What to present to come back after a lost connection.
    pub resume: ResumeToken,
}

/// Why a server turned a client away.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Not a D2RUST connection, or another wire format version.
    #[error("not a D2RUST connection of wire version {NET_VERSION}")]
    WireVersion,
    /// Another `d2-proto` protocol version.
    #[error("protocol version {client}, server {server}")]
    Protocol {
        /// The client's.
        client: u32,
        /// The server's.
        server: u32,
    },
    /// Another game build.
    #[error("game build {client}, server {server}")]
    Build {
        /// The client's.
        client: String,
        /// The server's.
        server: String,
    },
    /// Another tick rate.
    #[error("tick rate {client} Hz, server {server} Hz")]
    TickRate {
        /// The client's.
        client: u32,
        /// The server's.
        server: u32,
    },
    /// The game has no free player slot.
    #[error("the game is full")]
    Full,
    /// The resume token names no slot the server still holds (the grace
    /// period ran out, or the server restarted).
    #[error("the server no longer holds that player")]
    UnknownResume,
    /// Any other reason, as the server words it.
    #[error("{0}")]
    Other(String),
}

/// A failed connect or accept.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The socket (or the QUIC connection) failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The other end broke the wire format.
    #[error("bad handshake: {0}")]
    Malformed(&'static str),
    /// The server refused this client.
    #[error("refused: {0}")]
    Refused(Refusal),
}

/// Keepalive and timeout lengths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Send a keepalive after this long without sending.
    pub keepalive: Duration,
    /// Close the connection after this long without hearing anything.
    pub timeout: Duration,
}

impl Default for Timing {
    /// A keepalive every 2 s, a timeout after 10 s of silence.
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(2),
            timeout: Duration::from_secs(10),
        }
    }
}

// ---------------------------------------------------------------- frames

/// Builds the frame of one body: the length, then the body.
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + body.len());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    v
}

/// The body length a frame header announces, if it's a legal one.
pub fn body_len(header: [u8; 4]) -> Result<usize, NetError> {
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(NetError::Malformed);
    }
    Ok(len)
}

/// Cuts the first whole frame off `buf`: its body, or `None` when the
/// frame has not fully arrived.
pub fn take_frame(buf: &mut Vec<u8>) -> Result<Option<Vec<u8>>, NetError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = body_len([buf[0], buf[1], buf[2], buf[3]])?;
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let body = buf[4..4 + len].to_vec();
    buf.drain(..4 + len);
    Ok(Some(body))
}

fn lane_byte(lane: Lane) -> u8 {
    match lane {
        Lane::Game => 0,
        Lane::System => 1,
    }
}

/// The body of a game packet.
pub fn packet_body(p: &Packet) -> Vec<u8> {
    match p {
        Packet::Send { lane, msg } => {
            let mut v = vec![kind::SEND, lane_byte(*lane)];
            v.extend_from_slice(msg);
            v
        }
        Packet::Deliver(msg) => {
            let mut v = vec![kind::DELIVER];
            v.extend_from_slice(msg);
            v
        }
        Packet::Frame { ticked } => vec![kind::FRAME, u8::from(*ticked)],
        Packet::Ext { channel, msg } => {
            let mut v = vec![kind::EXT];
            v.extend_from_slice(&channel.to_le_bytes());
            v.extend_from_slice(msg);
            v
        }
    }
}

/// A frame body after the handshake: a game packet, or a control frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Body {
    /// A game packet.
    Packet(Packet),
    /// A keepalive ping.
    Ping,
    /// A goodbye: the connection ends cleanly.
    Bye,
}

pub fn parse_body(b: &[u8]) -> Result<Body, NetError> {
    let (&k, rest) = b.split_first().ok_or(NetError::Malformed)?;
    Ok(match k {
        kind::SEND => {
            let (&lane, msg) = rest.split_first().ok_or(NetError::Malformed)?;
            let lane = match lane {
                0 => Lane::Game,
                1 => Lane::System,
                _ => return Err(NetError::Malformed),
            };
            Body::Packet(Packet::Send {
                lane,
                msg: msg.to_vec(),
            })
        }
        kind::DELIVER => Body::Packet(Packet::Deliver(rest.to_vec())),
        kind::FRAME => match rest {
            [t @ (0 | 1)] => Body::Packet(Packet::Frame { ticked: *t == 1 }),
            _ => return Err(NetError::Malformed),
        },
        kind::EXT => match rest {
            [lo, hi, msg @ ..] => Body::Packet(Packet::Ext {
                channel: u16::from_le_bytes([*lo, *hi]),
                msg: msg.to_vec(),
            }),
            _ => return Err(NetError::Malformed),
        },
        kind::PING if rest.is_empty() => Body::Ping,
        kind::BYE if rest.is_empty() => Body::Bye,
        _ => return Err(NetError::Malformed),
    })
}

// ------------------------------------------------------------- handshake

fn put_str(v: &mut Vec<u8>, s: &str) {
    let mut n = s.len().min(MAX_BUILD);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    v.push(n as u8);
    v.extend_from_slice(&s.as_bytes()[..n]);
}

/// Reads a little-endian field off the front of `b`.
fn take<const N: usize>(b: &mut &[u8]) -> Option<[u8; N]> {
    if b.len() < N {
        return None;
    }
    let (head, rest) = b.split_at(N);
    *b = rest;
    head.try_into().ok()
}

fn take_str(b: &mut &[u8]) -> Option<String> {
    let [n] = take::<1>(b)?;
    let n = n as usize;
    if b.len() < n {
        return None;
    }
    let (s, rest) = b.split_at(n);
    *b = rest;
    String::from_utf8(s.to_vec()).ok()
}

impl Hello {
    /// The frame body of this hello.
    pub fn body(&self) -> Vec<u8> {
        let mut v = vec![kind::HELLO];
        v.extend_from_slice(&MAGIC);
        v.extend_from_slice(&NET_VERSION.to_le_bytes());
        v.extend_from_slice(&self.protocol.to_le_bytes());
        v.extend_from_slice(&self.tick_rate.to_le_bytes());
        put_str(&mut v, &self.build);
        match self.resume {
            None => v.push(0),
            Some(t) => {
                v.push(1);
                v.extend_from_slice(&t.0);
            }
        }
        v
    }

    /// Parses a hello body; `Err(WireVersion)` when the magic or the wire
    /// version is not ours, or the body is not a whole hello.
    pub fn parse(b: &[u8]) -> Result<Self, Refusal> {
        let mut b = b;
        let bad = || Refusal::WireVersion;
        if take::<1>(&mut b) != Some([kind::HELLO]) || take::<4>(&mut b) != Some(MAGIC) {
            return Err(bad());
        }
        if take::<2>(&mut b).map(u16::from_le_bytes) != Some(NET_VERSION) {
            return Err(bad());
        }
        let protocol = take::<4>(&mut b).map(u32::from_le_bytes).ok_or_else(bad)?;
        let tick_rate = take::<4>(&mut b).map(u32::from_le_bytes).ok_or_else(bad)?;
        let build = take_str(&mut b).ok_or_else(bad)?;
        let resume = match take::<1>(&mut b) {
            Some([0]) => None,
            Some([1]) => Some(ResumeToken(take::<16>(&mut b).ok_or_else(bad)?)),
            _ => return Err(bad()),
        };
        if !b.is_empty() {
            return Err(bad());
        }
        Ok(Self {
            protocol,
            build,
            tick_rate,
            resume,
        })
    }

    /// The server's check: the client must match `server` exactly
    /// (protocol, build and tick rate; the resume token is not compared).
    pub fn check(&self, server: &Hello) -> Result<(), Refusal> {
        if self.protocol != server.protocol {
            return Err(Refusal::Protocol {
                client: self.protocol,
                server: server.protocol,
            });
        }
        if self.build != server.build {
            return Err(Refusal::Build {
                client: self.build.clone(),
                server: server.build.clone(),
            });
        }
        if self.tick_rate != server.tick_rate {
            return Err(Refusal::TickRate {
                client: self.tick_rate,
                server: server.tick_rate,
            });
        }
        Ok(())
    }
}

impl Welcome {
    /// The frame body of this welcome.
    pub fn body(&self) -> Vec<u8> {
        let mut v = vec![kind::WELCOME];
        v.extend_from_slice(&self.client.to_le_bytes());
        v.extend_from_slice(&self.tick_rate.to_le_bytes());
        v.extend_from_slice(&self.max_players.to_le_bytes());
        v.extend_from_slice(&self.resume.0);
        v
    }

    /// Parses the fields after the kind byte.
    fn parse_fields(mut b: &[u8]) -> Option<Self> {
        let client = take::<4>(&mut b).map(u32::from_le_bytes)?;
        let tick_rate = take::<4>(&mut b).map(u32::from_le_bytes)?;
        let max_players = take::<4>(&mut b).map(u32::from_le_bytes)?;
        let resume = ResumeToken(take::<16>(&mut b)?);
        b.is_empty().then_some(Self {
            client,
            tick_rate,
            max_players,
            resume,
        })
    }
}

/// The client's reading of the server's answer to its hello.
pub fn parse_answer(body: &[u8]) -> Result<Welcome, ConnectError> {
    match body.split_first() {
        Some((&kind::WELCOME, rest)) => {
            Welcome::parse_fields(rest).ok_or(ConnectError::Malformed("welcome"))
        }
        Some((&kind::REFUSED, rest)) => Err(ConnectError::Refused(parse_refused(rest))),
        _ => Err(ConnectError::Malformed("answer to hello")),
    }
}

/// The frame body that refuses a client.
pub fn refused_body(r: &Refusal) -> Vec<u8> {
    let mut v = vec![kind::REFUSED];
    let (code, text) = match r {
        Refusal::WireVersion => (1, String::new()),
        Refusal::Protocol { client, server } => (2, format!("{client} {server}")),
        Refusal::Build { client, server } => (3, format!("{client}\n{server}")),
        Refusal::TickRate { client, server } => (4, format!("{client} {server}")),
        Refusal::Full => (5, String::new()),
        Refusal::Other(s) => (6, s.clone()),
        Refusal::UnknownResume => (7, String::new()),
    };
    v.push(code);
    v.extend_from_slice(text.as_bytes());
    v
}

/// Parses the fields of a refusal (after the kind byte).
pub fn parse_refused(b: &[u8]) -> Refusal {
    let text = String::from_utf8_lossy(b.get(1..).unwrap_or_default()).into_owned();
    let pair = |sep: char| {
        let mut it = text.splitn(2, sep);
        (
            it.next().unwrap_or_default().to_string(),
            it.next().unwrap_or_default().to_string(),
        )
    };
    let nums = || {
        let (a, b) = pair(' ');
        (a.parse().unwrap_or(0), b.parse().unwrap_or(0))
    };
    match b.first() {
        Some(1) => Refusal::WireVersion,
        Some(2) => {
            let (client, server) = nums();
            Refusal::Protocol { client, server }
        }
        Some(3) => {
            let (client, server) = pair('\n');
            Refusal::Build { client, server }
        }
        Some(4) => {
            let (client, server) = nums();
            Refusal::TickRate { client, server }
        }
        Some(5) => Refusal::Full,
        Some(7) => Refusal::UnknownResume,
        _ => Refusal::Other(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Hello {
        Hello {
            protocol: 7,
            build: "0.1.1.0".into(),
            tick_rate: 25,
            resume: None,
        }
    }

    #[test]
    fn framing_rejects_bad_lengths_and_bodies() {
        let mut buf = vec![0, 0, 0, 0];
        assert_eq!(take_frame(&mut buf), Err(NetError::Malformed));
        let mut buf = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        assert_eq!(take_frame(&mut buf), Err(NetError::Malformed));
        let mut buf = frame(&[kind::FRAME, 1]);
        buf.truncate(5);
        assert_eq!(take_frame(&mut buf), Ok(None));
        assert_eq!(parse_body(&[kind::FRAME, 2]), Err(NetError::Malformed));
        assert_eq!(parse_body(&[kind::SEND, 9, 1]), Err(NetError::Malformed));
        assert_eq!(parse_body(&[0x55]), Err(NetError::Malformed));
        assert_eq!(parse_body(&[kind::PING]), Ok(Body::Ping));
        assert_eq!(parse_body(&[kind::BYE]), Ok(Body::Bye));
        assert_eq!(parse_body(&[kind::EXT, 7]), Err(NetError::Malformed));
        for p in [
            Packet::Send {
                lane: Lane::System,
                msg: vec![0x6B],
            },
            Packet::Deliver(vec![0x8F; 33]),
            Packet::Frame { ticked: false },
            Packet::Ext {
                channel: 0x1234,
                msg: vec![1, 2, 3],
            },
            Packet::Ext {
                channel: 0,
                msg: Vec::new(),
            },
        ] {
            assert_eq!(parse_body(&packet_body(&p)), Ok(Body::Packet(p)));
        }
    }

    #[test]
    fn hello_round_trips() {
        let h = hello();
        assert_eq!(Hello::parse(&h.body()), Ok(h.clone()));
        let back = Hello {
            resume: Some(ResumeToken([9; 16])),
            ..hello()
        };
        assert_eq!(Hello::parse(&back.body()), Ok(back.clone()));
        let mut wrong = h.body();
        wrong[5] ^= 0xFF; // the wire version
        assert_eq!(Hello::parse(&wrong), Err(Refusal::WireVersion));
        let mut cut = back.body();
        cut.pop();
        assert_eq!(Hello::parse(&cut), Err(Refusal::WireVersion));
        // A long build is cut on a character boundary.
        let long = Hello {
            build: "é".repeat(40),
            ..hello()
        };
        assert_eq!(Hello::parse(&long.body()).unwrap().build, "é".repeat(32));
    }

    #[test]
    fn welcome_and_refusals_round_trip() {
        let w = Welcome {
            client: 63,
            tick_rate: 25,
            max_players: 64,
            resume: ResumeToken([0xA5; 16]),
        };
        assert_eq!(parse_answer(&w.body()).unwrap(), w);
        assert!(matches!(
            parse_answer(&w.body()[..10]),
            Err(ConnectError::Malformed("welcome"))
        ));
        for r in [
            Refusal::WireVersion,
            Refusal::Full,
            Refusal::UnknownResume,
            Refusal::Other("maintenance".into()),
            Refusal::Protocol {
                client: 8,
                server: 7,
            },
            Refusal::TickRate {
                client: 60,
                server: 25,
            },
            Refusal::Build {
                client: "a".into(),
                server: "b".into(),
            },
        ] {
            assert_eq!(parse_refused(&refused_body(&r)[1..]), r);
            match parse_answer(&refused_body(&r)) {
                Err(ConnectError::Refused(got)) => assert_eq!(got, r),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn tokens_are_random_and_debug_hides_them() {
        let (a, b) = (ResumeToken::random(), ResumeToken::random());
        assert_ne!(a, b);
        assert_eq!(
            format!("{:?}", ResumeToken([0xAB; 16])),
            "ResumeToken(abab..)"
        );
    }
}
