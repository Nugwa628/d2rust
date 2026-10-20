//! Reader of the item bit stream carried by S→C 0x9C (from byte 8) and
//! 0x9D (from byte 13): the client's side of `d2_sim::items::bitstream`
//! (the server's writer).
//!
//! The stream is LSB-first ([`BitReader`]) and does not say which type
//! tests an item code passes or how wide a stat is: the reader asks the
//! tables through [`ItemLookup`] (items / itemtypes facts by code,
//! itemstatcost save columns by stat). Values are returned as written,
//! `Save Add` removed ([`Stat::value`]); `ValShift`'s low bits are not on
//! the wire. A network stream holds one record padded with zero bits to a
//! whole byte ([`decode`]).
//!
//! The same reader decodes the save format (`.d2s` item entries):
//! [`decode_save_record`] and [`save_entry_len`]. A save record starts with
//! the "JM" marker, keeps the header flags as written, carries an extra
//! 32-bit unit field and an optional trailer, and is always "shown". A
//! save entry is the record padded to a whole byte, followed by the
//! entries of its socketed children.
//!
//! A record can fail without stopping the reader: it is read on to where
//! the reader stops and flagged [`ItemBits::failed`] (a quality outside
//! 1..=9, a set item with no `setitems` row, an item stat id 0 directly
//! after id 0). Such a record makes no item, and what follows it is not
//! checked.
//!
//! - `types`: [`ItemLookup`] and its facts, the errors, the decoded record.
//! - `reader`: [`BitReader`].
//! - `record`: the record layout, field by field.

mod reader;
mod record;
mod types;

pub use reader::BitReader;
pub use record::partners;
pub use types::{
    CodeFacts, Ear, IscSave, ItemBits, ItemBitsError, ItemLookup, Location, QualityFields,
    SaveEntry, Stat,
};

use reader::bit_set;
use record::{read_record, STAT_NUMSOCKETS};

/// Header flag bits the reader branches on.
pub mod hflag {
    /// The item is identified (its quality fields and stat lists are sent).
    pub const IDENTIFIED: u32 = 0x10;
    /// The item has sockets (a socket count follows).
    pub const SOCKETED: u32 = 0x800;
    /// The record is an ear.
    pub const EAR: u32 = 0x10000;
    /// Compact record: no quality fields and no stat lists.
    pub const COMPACT: u32 = 0x200000;
    /// The item carries a personalized name.
    pub const PERSONALIZED: u32 = 0x1000000;
    /// Alt-code record: the base code only, and nothing after it.
    pub const ALT_CODE: u32 = 0x2000000;
    /// The item is a runeword (a runeword id and an extra stat list follow).
    pub const RUNEWORD: u32 = 0x4000000;
}

/// Save-format marker "JM", read as 16 bits.
pub const SAVE_MARKER: u32 = 0x4D4A;

/// Terminator of a stat list (a 9-bit stat id of all ones).
pub const TERMINATOR: u32 = 0x1FF;

/// Decodes one network stream. The stream must end in its last byte with
/// zero padding; anything else is an error.
pub fn decode(stream: &[u8], t: &dyn ItemLookup) -> Result<ItemBits, ItemBitsError> {
    let mut r = BitReader::new(stream);
    let mut it = decode_record(&mut r, t)?;
    let mut used = r.pos();
    let total = stream.len() * 8;
    // The server's writer sends the socket count of an unidentified full
    // record although its header lacks the socketed flag, and the reader
    // stops before it. When bits remain that are not plain padding, those
    // bits are skipped here (not read) so the padding check sees what
    // follows.
    let unflagged_full_record =
        it.flags & (hflag::IDENTIFIED | hflag::COMPACT | hflag::ALT_CODE | hflag::SOCKETED) == 0;
    if !it.failed
        && !it.save
        && unflagged_full_record
        && (total - used >= 8 || (used..total).any(|at| bit_set(stream, at)))
    {
        if let Some(c) = t.isc(STAT_NUMSOCKETS) {
            let n = usize::from(c.save_bits);
            if total - used >= n {
                used += n;
                it.bits = used;
            }
        }
    }
    // A failed record stopped inside its data: what follows is not checked.
    if it.failed {
        return Ok(it);
    }
    if total - used >= 8 {
        return Err(ItemBitsError::Trailing(total - used));
    }
    check_padding(stream, used..total)?;
    Ok(it)
}

/// Decodes one network record from `r` (no padding check).
pub fn decode_record(r: &mut BitReader<'_>, t: &dyn ItemLookup) -> Result<ItemBits, ItemBitsError> {
    read_record(r, t, false)
}

/// Decodes one save-format record from `r`: the "JM" marker, the flags as
/// written, the extra unit field and the trailer, "shown" always. No
/// padding, no children.
pub fn decode_save_record(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
) -> Result<ItemBits, ItemBitsError> {
    read_record(r, t, true)
}

/// Decodes one save item entry at `buf[0..]`: the item's save record, zero
/// padding to a whole byte, then `filled` children (0 for compact and
/// alt-code records), each an entry of its own read the same way. The
/// padding of a failed record is not checked.
pub fn save_entry_len(buf: &[u8], t: &dyn ItemLookup) -> Result<SaveEntry, ItemBitsError> {
    let mut r = BitReader::new(buf);
    let item = decode_save_record(&mut r, t)?;
    let used = r.pos();
    let len = used.div_ceil(8);
    let checked = if item.failed { used } else { len * 8 };
    check_padding(buf, used..checked)?;
    let mut entry = SaveEntry {
        len,
        children: Vec::with_capacity(usize::from(item.filled)),
        item,
    };
    for _ in 0..entry.item.filled {
        let child = save_entry_len(&buf[entry.len..], t).map_err(|e| shift(e, entry.len))?;
        entry.len += child.len;
        entry.children.push(child);
    }
    Ok(entry)
}

/// Every bit of `bits` is 0; the first one set is [`ItemBitsError::Padding`].
fn check_padding(buf: &[u8], bits: std::ops::Range<usize>) -> Result<(), ItemBitsError> {
    match bits.into_iter().find(|&at| bit_set(buf, at)) {
        Some(at) => Err(ItemBitsError::Padding(at)),
        None => Ok(()),
    }
}

/// Bit positions in a child's error are made relative to the parent's start.
fn shift(e: ItemBitsError, bytes: usize) -> ItemBitsError {
    match e {
        ItemBitsError::Short(b) => ItemBitsError::Short(b + bytes * 8),
        ItemBitsError::Padding(b) => ItemBitsError::Padding(b + bytes * 8),
        e => e,
    }
}

#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod tests;
