//! Typed S→C builders and the client-side parser: the wire layer between the
//! server's events and the client's decoder.
//!
//! There is one type per S→C message whose whole layout is known: every
//! byte after the id is a field, a constant, or a byte that carries no
//! value. The types come from two places: hand-written ones in [`messages`]
//! (NPC, quest, waypoint, cube and join messages) and the ones the
//! generator derives from the `server-messages.tsv` `layout` column (typed
//! in [`crate::generated::server`] and re-exported here). `d2-sim` has its
//! own byte builders and checks them against these types in its
//! `s2c_bytes` test; the client's bridge decodes delivered messages with
//! [`parse`].
//!
//! - [`ServerMsg`]: the trait of the hand-written messages, defined from a
//!   field list by `s2c_message!`.
//! - [`parse`]: one delivered message → [`Message`]; ids without a full
//!   layout are [`ParseError::Unbuilt`] with their [`audit()`] status.
//! - [`mod@audit`]: every id 0x00–0xB4 → built / generated / id only /
//!   partial / unspecified / never, with what is known of its layout.
//!
//! Bytes that carry no value (the server never fills them) are written as
//! 0, as `d2-sim`'s builders do, and listed in [`ServerMsg::UNWRITTEN`] so
//! a byte comparison can mask them; the parser ignores them.

pub mod audit;
mod field;
pub mod messages;
mod parse;

pub use audit::{audit, Audit, Status, AUDIT};
pub use messages::*;
pub use parse::{parse, Message, ParseError};

// Messages typed by the generator from the TSV `layout` column.
pub use crate::generated::server::{
    AddExpByte, AddExpDword, AddExpWord, ClearCursor, ConnectionTerminated, GameExit, GameFlags,
    GameLoading, LoadAct, LoadComplete, LoadSuccessful, MapReveal, MonsterHit, PortalFlags,
    Relator1, Relator2, SetItemState, SetStatByte, SetStatDword, SetStatWord, SmallGoldPickup,
    StartMercList, Unknown6E, Unknown6F, Unknown70, Unknown71, Unknown72, UnloadComplete,
    UseStackableItem, WeaponSwitch,
};

/// A fixed-size S→C message whose whole layout is known: fields,
/// constant bytes and bytes without a value cover every byte after the id
/// exactly once (the `layouts_cover_every_byte` test checks it).
pub trait ServerMsg: Sized {
    /// The message id (first byte).
    const ID: u8;
    /// Total size in bytes, id included.
    const SIZE: usize;
    /// Fields: name, offset, byte length.
    const FIELDS: &'static [(&'static str, usize, usize)];
    /// Bytes the sender always writes with this value (offset, value).
    const CONSTS: &'static [(usize, u8)];
    /// Bytes the server never writes (D2RUST writes them as 0).
    const UNWRITTEN: &'static [usize];
    /// Decodes exactly `SIZE` bytes starting with `ID`; checks
    /// [`Self::CONSTS`], ignores [`Self::UNWRITTEN`].
    fn decode(b: &[u8]) -> Result<Self, ParseError>;
    /// Writes the message into `out` (`SIZE` bytes). Panics on another
    /// length.
    fn write(&self, out: &mut [u8]);
}

/// Shared checks of [`ServerMsg::decode`].
fn check<M: ServerMsg>(b: &[u8]) -> Result<(), ParseError> {
    match b.first() {
        None => return Err(ParseError::Empty),
        Some(&f) if f != M::ID => {
            return Err(ParseError::WrongId {
                expected: M::ID,
                found: f,
            })
        }
        _ => {}
    }
    if b.len() != M::SIZE {
        return Err(ParseError::WrongSize {
            id: M::ID,
            expected: M::SIZE,
            found: b.len(),
        });
    }
    for &(offset, expected) in M::CONSTS {
        if b[offset] != expected {
            return Err(ParseError::Const {
                id: M::ID,
                offset,
                expected,
                found: b[offset],
            });
        }
    }
    Ok(())
}

/// Zeroes `out`, writes the id and the constants.
fn start<M: ServerMsg>(out: &mut [u8]) {
    assert_eq!(out.len(), M::SIZE, "output buffer size");
    out.fill(0);
    out[0] = M::ID;
    for &(offset, v) in M::CONSTS {
        out[offset] = v;
    }
}

/// Defines a [`ServerMsg`] struct from its layout: the id, name and size,
/// the fields (`name: type = offset`, read and written through
/// `field::WireField`), the constant bytes (`offset => value`) and the
/// bytes without a value. Also gives the struct an `encode` that returns
/// the message as a byte array.
macro_rules! s2c_message {
    (
        $(#[$doc:meta])*
        $id:literal $name:ident $size:literal {
            $( $(#[$fdoc:meta])* $field:ident : $ty:ty = $off:literal ),* $(,)?
        }
        consts [ $( $coff:literal => $cval:literal ),* $(,)? ]
        unwritten [ $( $uoff:literal ),* $(,)? ]
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name {
            $( $(#[$fdoc])* pub $field: $ty, )*
        }

        impl $crate::s2c::ServerMsg for $name {
            const ID: u8 = $id;
            const SIZE: usize = $size;
            const FIELDS: &'static [(&'static str, usize, usize)] = &[
                $( (stringify!($field), $off, <$ty as $crate::s2c::field::WireField>::LEN), )*
            ];
            const CONSTS: &'static [(usize, u8)] = &[ $( ($coff, $cval), )* ];
            const UNWRITTEN: &'static [usize] = &[ $( $uoff, )* ];
            #[allow(unused_variables)]
            fn decode(b: &[u8]) -> Result<Self, $crate::s2c::ParseError> {
                $crate::s2c::check::<Self>(b)?;
                Ok(Self {
                    $( $field: <$ty as $crate::s2c::field::WireField>::read(b, $off), )*
                })
            }
            #[allow(unused_variables)]
            fn write(&self, out: &mut [u8]) {
                $crate::s2c::start::<Self>(out);
                $( $crate::s2c::field::WireField::write(&self.$field, out, $off); )*
            }
        }

        impl $name {
            /// The message bytes (unwritten bytes 0).
            pub fn encode(&self) -> [u8; $size] {
                let mut b = [0; $size];
                $crate::s2c::ServerMsg::write(self, &mut b);
                b
            }
        }
    };
}
pub(crate) use s2c_message;

#[cfg(test)]
mod tests;
