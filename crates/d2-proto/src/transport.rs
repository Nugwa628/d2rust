//! Size lookup per direction and the local transport's message
//! classification (C→S) and buffer split (S→C).
//!
//! Messages carry no length prefix: the length of each one follows from its
//! id's size rule (a fixed count, a length field, or the chat strings) in
//! the tables of [`crate::generated`].
//!
//! - C→S: the server runs [`classify_client`] on each incoming message to
//!   pick its queue. Game ids (below 0x67) go to the game queue, system ids
//!   0x67..=0x72 to the system queue, 0xFF (a 16-byte admin message) to the
//!   admin queue; other ids in 0x73..=0xFE are invalid.
//! - S→C: the server packs messages into a per-client buffer and flushes
//!   it; the client cuts the flushed buffer apart with
//!   [`split_server_buffer`] and delivers ids below 0xAF to its game list,
//!   the rest to its system list ([`server_list`]).
//!
//! Each function has an `_on` form that takes the game's [`Wire`]. On
//! [`Wire::Original`] (the plain forms) the mod envelope ids are invalid
//! like any unknown id; on [`Wire::Mod`] they are sized by their length
//! field ([`crate::envelope`]), a C→S envelope goes to the game queue and
//! an S→C envelope to the game list.

use crate::envelope::{self, Wire};
use crate::generated::{CLIENT_MESSAGES, SERVER_MESSAGES};
use crate::schema::{ClientMessage, ServerMessage, Size};

/// C→S ids no client message uses (past the system ids, before admin).
const INVALID_CLIENT_IDS: std::ops::RangeInclusive<u8> = 0x73..=0xFE;
/// C→S id of the admin message (queue 2, [`ADMIN_SIZE`] bytes).
const ADMIN_ID: u8 = 0xFF;

/// Largest message either direction carries (0x204).
pub const MAX_MESSAGE: usize = 0x204;
/// Size of a per-client S→C buffer.
pub const BUFFER_SIZE: usize = 0x200;
/// Size of a queue-2 (id 0xFF) admin message.
pub const ADMIN_SIZE: usize = 16;
/// First C→S system id; lower ids are game messages.
pub const FIRST_SYSTEM_ID: u8 = 0x67;
/// First S→C id delivered to the client's system list.
pub const FIRST_SERVER_SYSTEM_ID: u8 = 0xAF;

/// The C→S descriptor of `id`, if the size table has an entry for it.
pub fn client_message(id: u8) -> Option<&'static ClientMessage> {
    CLIENT_MESSAGES.get(id as usize)
}

/// The S→C descriptor of `id`, if the size table has an entry for it.
pub fn server_message(id: u8) -> Option<&'static ServerMessage> {
    SERVER_MESSAGES.get(id as usize)
}

/// C→S size of the message at the start of `b`, by the transport size rule
/// of its id. Ids past the table (0x71..) are [`Size::Invalid`].
pub fn client_size(b: &[u8]) -> Size {
    match b.first() {
        None => Size::Incomplete,
        Some(&id) => match client_message(id) {
            Some(_) if id == envelope::CLIENT_ID => Size::Invalid,
            Some(m) => m.transport_size.eval(b),
            None => Size::Invalid,
        },
    }
}

/// S→C size of the message at the start of `b`, by the size rule of its
/// id. Ids past the table (0xB5..) are [`Size::Invalid`].
pub fn server_size(b: &[u8]) -> Size {
    match b.first() {
        None => Size::Incomplete,
        Some(&id) => match server_message(id) {
            Some(_) if id == envelope::SERVER_ID => Size::Invalid,
            Some(m) => m.size.eval(b),
            None => Size::Invalid,
        },
    }
}

/// [`client_size`] on `wire`: a C→S envelope on [`Wire::Mod`] is sized by
/// its length field.
pub fn client_size_on(wire: Wire, b: &[u8]) -> Size {
    match b.first() {
        Some(&envelope::CLIENT_ID) if wire.envelopes() => envelope::size(b),
        _ => client_size(b),
    }
}

/// [`server_size`] on `wire`: an S→C envelope on [`Wire::Mod`] is sized by
/// its length field.
pub fn server_size_on(wire: Wire, b: &[u8]) -> Size {
    match b.first() {
        Some(&envelope::SERVER_ID) if wire.envelopes() => envelope::size(b),
        _ => server_size(b),
    }
}

/// Server queue a C→S message goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientQueue {
    /// Queue 0: system ids 0x67..0x72.
    System,
    /// Queue 1: game ids below 0x67.
    Game,
    /// Queue 2: id 0xFF, subject to the net object's gate, which the
    /// caller applies; failing it is [`Classified::Invalid`].
    Admin,
}

/// Classifier result. In local mode only `Queue` enqueues; the others drop
/// the message silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classified {
    /// The message is accepted into this queue.
    Queue(ClientQueue),
    /// Too short, size rule 0, over 0x204, or over the given size.
    Incomplete,
    /// An unassigned id in 0x73..0xFE.
    Invalid,
    /// The chat size rule came out negative (its last byte is signed).
    /// What the server does with such a message is its own decision, so
    /// the classifier reports it apart instead of calling it incomplete.
    NegativeSize(i32),
}

/// Classifies one C→S message of exactly `b.len()` bytes: invalid ids
/// first, then the size check (the rule's size must be at most
/// [`MAX_MESSAGE`] and fit in `b`), then the queue by id.
pub fn classify_client(b: &[u8]) -> Classified {
    classify_client_on(Wire::Original, b)
}

/// [`classify_client`] on `wire`: on [`Wire::Mod`] a C→S envelope whose
/// length field fits `b` goes to the game queue, in order with the game's
/// own messages.
pub fn classify_client_on(wire: Wire, b: &[u8]) -> Classified {
    if wire.envelopes() && b.first() == Some(&envelope::CLIENT_ID) {
        return match envelope::size(b) {
            Size::Bytes(n) if n <= b.len() => Classified::Queue(ClientQueue::Game),
            _ => Classified::Incomplete,
        };
    }
    let Some(&id) = b.first() else {
        return Classified::Incomplete;
    };
    if INVALID_CLIENT_IDS.contains(&id) || id == envelope::CLIENT_ID {
        return Classified::Invalid;
    }
    let size = if id == ADMIN_ID {
        Size::Bytes(ADMIN_SIZE)
    } else {
        client_size(b)
    };
    match size {
        Size::Bytes(n) if n <= MAX_MESSAGE && n <= b.len() => {}
        Size::Negative(n) => return Classified::NegativeSize(n),
        _ => return Classified::Incomplete,
    }
    Classified::Queue(match id {
        0..FIRST_SYSTEM_ID => ClientQueue::Game,
        ADMIN_ID => ClientQueue::Admin,
        _ => ClientQueue::System,
    })
}

/// Client list an S→C message is delivered to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerList {
    /// Ids below 0xAF, and the S→C envelope 0xB5 on a mod game's wire.
    Game,
    /// System ids from 0xAF, including ViewReady (0xB6).
    System,
}

/// The list an S→C message with this id is delivered to. An S→C envelope
/// (only ever split out on [`Wire::Mod`]) goes to the game list.
pub fn server_list(id: u8) -> ServerList {
    if id < FIRST_SERVER_SYSTEM_ID || id == envelope::SERVER_ID {
        ServerList::Game
    } else {
        ServerList::System
    }
}

/// A flushed S→C buffer split into messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split<'a> {
    /// The whole messages, in buffer order.
    pub messages: Vec<&'a [u8]>,
    /// Bytes after a message whose size rule gave 0 or could not be
    /// evaluated; they are dropped, not delivered.
    pub discarded: &'a [u8],
}

/// A buffer the split cannot take: a message over [`MAX_MESSAGE`], or a
/// message running past the buffer's end (buffers hold whole messages).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SplitError {
    /// The message at byte `at` is `size` bytes, over [`MAX_MESSAGE`].
    #[error("message at byte {at} is {size} bytes, over 0x204")]
    TooLarge { at: usize, size: usize },
    /// The message at byte `at` is `size` bytes, past the buffer's end.
    #[error("message at byte {at} is {size} bytes, past the buffer's end")]
    Truncated { at: usize, size: usize },
}

/// Splits a flushed S→C buffer into messages with the size rule. The split
/// stops at the first id whose rule gives no size (an invalid id, size 0,
/// or too few bytes to evaluate): the rest of the buffer is
/// [`Split::discarded`].
pub fn split_server_buffer(buf: &[u8]) -> Result<Split<'_>, SplitError> {
    split_server_buffer_on(Wire::Original, buf)
}

/// [`split_server_buffer`] on `wire`: on [`Wire::Mod`] S→C envelopes are
/// whole messages too.
pub fn split_server_buffer_on(wire: Wire, buf: &[u8]) -> Result<Split<'_>, SplitError> {
    let mut messages = Vec::new();
    let mut at = 0;
    while at < buf.len() {
        let rest = &buf[at..];
        let size = match server_size_on(wire, rest) {
            Size::Bytes(n) => n,
            // S→C rules never come out negative.
            Size::Invalid | Size::Incomplete | Size::Negative(_) => break,
        };
        if size > MAX_MESSAGE {
            return Err(SplitError::TooLarge { at, size });
        }
        if size > rest.len() {
            return Err(SplitError::Truncated { at, size });
        }
        messages.push(&rest[..size]);
        at += size;
    }
    Ok(Split {
        messages,
        discarded: &buf[at..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// C→S sizes: fixed, field-driven, chat and invalid ids.
    #[test]
    fn client_size_vectors() {
        let chat = |last: u8| bytes(&[&[0x15, 0x01, 0x00], b"hi\0bob\0", &[last]]);
        assert_eq!(client_size(&chat(0x00)), Size::Bytes(11));
        assert_eq!(client_size(&chat(0x05)), Size::Bytes(16));
        assert_eq!(client_size(&chat(0xFF)), Size::Bytes(10));
        assert_eq!(client_size(&[0x66, 0x05, 0x00]), Size::Bytes(8));
        assert_eq!(client_size(&[0x66, 0xFD, 0x01]), Size::Bytes(512));
        assert_eq!(client_size(&[0x66, 0xFE, 0x01]), Size::Bytes(3));
        assert_eq!(client_size(&[0x6C, 0x10, 0, 0, 0, 0]), Size::Bytes(23));
        assert_eq!(client_size(&[0x6C, 0x10]), Size::Incomplete);
        for id in [0x2C, 0x4A, 0x64] {
            assert_eq!(client_size(&[id; 16]), Size::Invalid);
        }
        // Chat without its strings' NULs, or without the trailing byte.
        assert_eq!(client_size(&[0x15, 0x01, 0x00, b'h']), Size::Incomplete);
        assert_eq!(client_size(b"\x15\x01\x00hi\0bob\0"), Size::Incomplete);
        // A large negative trailing byte.
        assert_eq!(client_size(&chat(0x80)), Size::Negative(11 - 128));
    }

    /// Classifier: queues, invalid ids, short and oversized input.
    #[test]
    fn classifier_vectors() {
        assert_eq!(classify_client(&[0x80; 5]), Classified::Invalid);
        assert_eq!(
            classify_client(&[0xFF; 16]),
            Classified::Queue(ClientQueue::Admin)
        );
        assert_eq!(
            classify_client(&[0x6B]),
            Classified::Queue(ClientQueue::System)
        );
        assert_eq!(
            classify_client(&[0x01, 0x10, 0x00, 0x20, 0x00]),
            Classified::Queue(ClientQueue::Game)
        );
        assert_eq!(
            classify_client(&[0x01, 0x10, 0x00, 0x20]),
            Classified::Incomplete
        );
        assert_eq!(classify_client(&[]), Classified::Incomplete);
        // Size 0 in the table: never queued.
        assert_eq!(classify_client(&[0x2C; 8]), Classified::Incomplete);
        // Over 0x204 is never accepted, longer buffers are (whole copy).
        assert_eq!(
            classify_client(&[0x01; 6]),
            Classified::Queue(ClientQueue::Game)
        );
        let mut chat = bytes(&[&[0x15, 0x01, 0x00], b"hi\0bob\0", &[0x05]]);
        assert_eq!(classify_client(&chat), Classified::Incomplete);
        chat.resize(16, 0);
        assert_eq!(classify_client(&chat), Classified::Queue(ClientQueue::Game));
    }

    /// S→C sizes: fixed, field-driven, chat and invalid ids.
    #[test]
    fn server_size_vectors() {
        let mut m94 = vec![0x94, 0x03];
        m94.extend([0; 7]);
        assert_eq!(server_size(&m94), Size::Bytes(15));
        assert_eq!(server_size(&[0xAF, 0x00]), Size::Bytes(2));
        assert_eq!(server_size(&[0xAF, 0x05]), Size::Bytes(6));
        assert_eq!(server_size(&[0xAE, 0x10, 0x00]), Size::Bytes(19));
        let m26 = bytes(&[&[0x26], &[0; 9], b"a\0bc\0"]);
        assert_eq!(server_size(&m26), Size::Bytes(15));
        assert_eq!(server_size(&[0x16, 0x20, 0x00]), Size::Incomplete);
        let mut m16 = vec![0x16, 0x20, 0x00];
        m16.resize(13, 0);
        assert_eq!(server_size(&m16), Size::Bytes(32));
        assert_eq!(server_size(&[0x80]), Size::Invalid);
        assert_eq!(server_size(&[0xB5]), Size::Invalid);
        assert_eq!(server_size(&[0x1A, 0x07]), Size::Bytes(2));
    }

    /// Buffer split: whole messages, the discarded tail, truncation and
    /// oversize errors, and the list an id goes to.
    #[test]
    fn split() {
        // 0x1A (2 bytes), 0x5F (5 bytes), then an id with size 0 ends it.
        let buf = [0x1A, 0x07, 0x5F, 1, 2, 3, 4, 0x80, 0x1A, 0x07];
        let s = split_server_buffer(&buf).unwrap();
        assert_eq!(s.messages, vec![&buf[0..2], &buf[2..7]]);
        assert_eq!(s.discarded, &buf[7..]);
        assert_eq!(
            split_server_buffer(&[0x1A, 0x07, 0x5F, 1]),
            Err(SplitError::Truncated { at: 2, size: 5 })
        );
        let mut big = vec![0x16, 0x05, 0x02];
        big.resize(13, 0);
        assert_eq!(
            split_server_buffer(&big),
            Err(SplitError::TooLarge { at: 0, size: 0x205 })
        );
        assert_eq!(server_list(0xAE), ServerList::Game);
        assert_eq!(server_list(0xAF), ServerList::System);
    }

    #[test]
    fn the_original_wire_refuses_the_envelope_ids_as_before() {
        let c = crate::envelope::encode(crate::envelope::Direction::ToServer, &[1, 2]).unwrap();
        let sv = crate::envelope::encode(crate::envelope::Direction::ToClient, &[1, 2]).unwrap();
        assert_eq!(classify_client(&c), Classified::Invalid);
        assert_eq!(classify_client_on(Wire::Original, &c), Classified::Invalid);
        assert_eq!(client_size_on(Wire::Original, &c), Size::Invalid);
        assert_eq!(server_size_on(Wire::Original, &sv), Size::Invalid);
        let s = split_server_buffer_on(Wire::Original, &sv).unwrap();
        assert!(s.messages.is_empty());
        assert_eq!(s.discarded, &sv[..]);
    }

    #[test]
    fn the_mod_wire_sizes_queues_and_splits_envelopes() {
        use crate::envelope::{encode, Direction};
        let c = encode(Direction::ToServer, &[1, 2, 3]).unwrap();
        assert_eq!(client_size_on(Wire::Mod, &c), Size::Bytes(6));
        assert_eq!(
            classify_client_on(Wire::Mod, &c),
            Classified::Queue(ClientQueue::Game)
        );
        // Shorter than its length field, or a length past the largest
        // payload: not sendable.
        assert_eq!(
            classify_client_on(Wire::Mod, &c[..5]),
            Classified::Incomplete
        );
        assert_eq!(
            classify_client_on(Wire::Mod, &[0x71, 0xFF, 0xFF]),
            Classified::Incomplete
        );
        // The other ids keep their own rules on the mod wire.
        assert_eq!(
            classify_client_on(Wire::Mod, &[0x72; 13]),
            Classified::Queue(ClientQueue::System)
        );
        for id in 0x73..=0xFE {
            assert_eq!(
                classify_client_on(Wire::Mod, &[id; 16]),
                Classified::Invalid
            );
        }
        assert_eq!(server_size_on(Wire::Mod, &[0xB6]), Size::Bytes(13));
        assert_eq!(server_size_on(Wire::Mod, &[0xB6; 13]), Size::Bytes(13));

        let a = encode(Direction::ToClient, b"mod").unwrap();
        let b = encode(Direction::ToClient, &[]).unwrap();
        let buf = bytes(&[&a, &[0x1A, 0x07], &b]);
        let s = split_server_buffer_on(Wire::Mod, &buf).unwrap();
        assert_eq!(s.messages, [&a[..], &[0x1A, 0x07][..], &b[..]]);
        assert!(s.discarded.is_empty());
        assert_eq!(server_list(0xB5), ServerList::Game);
        assert_eq!(server_list(0xB4), ServerList::System);
        assert_eq!(server_list(0xB6), ServerList::System);
    }
}
