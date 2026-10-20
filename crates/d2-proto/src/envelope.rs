//! The mod message envelope: how a mod game sends its own messages over
//! D2RUST's protocol without changing it (`docs/plans/mods/seams.md` MS13).
//!
//! A mod game is its own game built on D2RUST's crates. When it needs a
//! message D2RUST does not have, it wraps its payload in one envelope id
//! per direction instead of adding ids or forking the netcode:
//!
//! - C→S [`CLIENT_ID`] (0x71) and S→C [`SERVER_ID`] (0xB5), the first id
//!   past each direction's own messages;
//! - layout `[id][u16 length, little-endian][payload]`: the length counts
//!   the payload only;
//! - the payload is opaque to D2RUST. The mod game owns its layout and
//!   versions it with a version byte of its own.
//!
//! Envelopes exist only on a mod game's wire ([`Wire::Mod`]). On D2RUST's
//! own wire ([`Wire::Original`]) both ids stay invalid and are refused
//! exactly like any other unknown id: the plain size and split functions in
//! [`crate::transport`] never accept them.
//!
//! A whole envelope is at most [`MAX_ENVELOPE`] bytes (0x1FC): the most the
//! server's queue drain copies of one C→S message, under the client's
//! game-message limit (0x200) and within one S→C buffer.

use crate::schema::Size;

/// C→S envelope id: one past the last system id (0x70).
pub const CLIENT_ID: u8 = 0x71;
/// S→C envelope id: one past the last S→C id (0xB4).
pub const SERVER_ID: u8 = 0xB5;
/// Bytes before the payload: the id and the u16 length.
pub const HEADER: usize = 3;
/// Largest whole envelope: what the server's queue drain copies of one
/// C→S message (0x200 less the 4-byte client id), so a payload is never
/// cut short. It is also under the client's 0x200 game-message limit and
/// within one 0x200-byte S→C buffer.
pub const MAX_ENVELOPE: usize = 0x1FC;
/// Largest payload an envelope carries.
pub const MAX_PAYLOAD: usize = MAX_ENVELOPE - HEADER;

/// The wire a game speaks: D2RUST's own, or a mod game's, which adds the
/// two envelope ids. Set when the game is built, from its ruleset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wire {
    /// D2RUST's own messages only; the envelope ids are invalid.
    #[default]
    Original,
    /// D2RUST's messages plus the envelopes of a mod game.
    Mod,
}

impl Wire {
    /// `true` when the envelope ids are valid on this wire.
    pub fn envelopes(self) -> bool {
        self == Wire::Mod
    }
}

/// A direction of the wire: which envelope id is meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Client to server ([`CLIENT_ID`]).
    ToServer,
    /// Server to client ([`SERVER_ID`]).
    ToClient,
}

impl Direction {
    /// The envelope id of this direction.
    pub fn id(self) -> u8 {
        match self {
            Direction::ToServer => CLIENT_ID,
            Direction::ToClient => SERVER_ID,
        }
    }
}

/// A payload too large for one envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("mod message payload of {0} bytes (at most {MAX_PAYLOAD})")]
pub struct TooLarge(pub usize);

/// The whole envelope of `payload` in direction `dir`.
pub fn encode(dir: Direction, payload: &[u8]) -> Result<Vec<u8>, TooLarge> {
    if payload.len() > MAX_PAYLOAD {
        return Err(TooLarge(payload.len()));
    }
    let mut b = Vec::with_capacity(HEADER + payload.len());
    b.push(dir.id());
    b.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    b.extend_from_slice(payload);
    Ok(b)
}

/// Size of the envelope at the start of `b` (its first byte must be an
/// envelope id): the header plus the length field. A length past
/// [`MAX_PAYLOAD`] is [`Size::Invalid`]; fewer than [`HEADER`] bytes is
/// [`Size::Incomplete`].
pub fn size(b: &[u8]) -> Size {
    if b.len() < HEADER {
        return Size::Incomplete;
    }
    let n = usize::from(u16::from_le_bytes([b[1], b[2]]));
    if n > MAX_PAYLOAD {
        Size::Invalid
    } else {
        Size::Bytes(HEADER + n)
    }
}

/// The payload of the whole envelope `msg`: `None` unless `msg` starts
/// with `dir`'s id and is exactly as long as its length field says.
pub fn payload(dir: Direction, msg: &[u8]) -> Option<&[u8]> {
    if msg.first() != Some(&dir.id()) {
        return None;
    }
    match size(msg) {
        Size::Bytes(n) if n == msg.len() => Some(&msg[HEADER..]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_writes_the_id_the_length_and_the_payload() {
        let b = encode(Direction::ToServer, &[7, 8, 9]).unwrap();
        assert_eq!(b, [0x71, 3, 0, 7, 8, 9]);
        let b = encode(Direction::ToClient, &[]).unwrap();
        assert_eq!(b, [0xB5, 0, 0]);
    }

    #[test]
    fn the_largest_envelope_passes_both_limits() {
        let p = vec![0xAA; MAX_PAYLOAD];
        let b = encode(Direction::ToServer, &p).unwrap();
        assert_eq!(b.len(), MAX_ENVELOPE);
        assert!(b.len() <= 0x200 - 4, "the server drain's copy");
        assert!(b.len() < 0x200, "the client's game-message limit");
        assert!(b.len() <= crate::transport::BUFFER_SIZE, "one S→C buffer");
        assert_eq!(size(&b), Size::Bytes(MAX_ENVELOPE));
        assert_eq!(
            encode(Direction::ToServer, &[0; MAX_PAYLOAD + 1]),
            Err(TooLarge(MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn size_reads_the_length_field() {
        assert_eq!(size(&[0x71]), Size::Incomplete);
        assert_eq!(size(&[0x71, 2]), Size::Incomplete);
        assert_eq!(size(&[0x71, 2, 0]), Size::Bytes(5));
        assert_eq!(size(&[0xB5, 0x00, 0x01]), Size::Bytes(HEADER + 0x100));
        // A length past the largest payload is never a message.
        assert_eq!(size(&[0x71, 0xF9, 0x01]), Size::Bytes(MAX_ENVELOPE));
        assert_eq!(size(&[0x71, 0xFA, 0x01]), Size::Invalid);
        assert_eq!(size(&[0x71, 0xFF, 0xFF]), Size::Invalid);
    }

    #[test]
    fn payload_needs_the_right_id_and_the_exact_length() {
        let b = encode(Direction::ToClient, b"hi").unwrap();
        assert_eq!(payload(Direction::ToClient, &b), Some(&b"hi"[..]));
        assert_eq!(payload(Direction::ToServer, &b), None);
        assert_eq!(payload(Direction::ToClient, &b[..4]), None);
        let mut long = b.clone();
        long.push(0);
        assert_eq!(payload(Direction::ToClient, &long), None);
    }

    #[test]
    fn only_the_mod_wire_has_envelopes() {
        assert!(!Wire::Original.envelopes());
        assert!(!Wire::default().envelopes());
        assert!(Wire::Mod.envelopes());
    }
}
