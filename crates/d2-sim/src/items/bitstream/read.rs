//! The save-format record reader on the server side: the inverse of
//! [`super::write_save`] into the writer's own view ([`StreamItem`]), so a
//! record read back writes the same bytes. The decoder (full and compact
//! records) is read as the exact inverse of the writer; this module is
//! that reading. The client's reader of the network
//! stream is `d2_proto::item_bits`.
//!
//! What the stream does not carry, the reader takes from the tables (the
//! class from the code, the type tests); what the writer derives (the
//! runeword name id, the filled count) is returned as read.

use super::{
    hflag, Isc, IscTable, Kind, StatEntry, StreamItem, AUTO_OFFSET, PREFIX_OFFSET, SAVE_MARKER,
    TERMINATOR,
};
use crate::items::{stat, ty, ItemTables};

/// Why a record did not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("record ends at bit {0}")]
    Short(usize),
    #[error("save marker is {0:#06x}, not 0x4D4A (\"JM\")")]
    BadMarker(u32),
    /// The class lookup of the header peek: a code outside
    /// the items table (none).
    #[error("unknown item code {0:?}")]
    UnknownCode([u8; 4]),
    #[error("padding bit {0} is set")]
    Padding(usize),
}

/// LSB-first bit reader.
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// The next `n` bits (n ≤ 32), bit 0 first.
    pub fn read(&mut self, n: u32) -> Result<u32, ReadError> {
        let n = n as usize;
        if self.pos + n > self.buf.len() * 8 {
            return Err(ReadError::Short(self.pos));
        }
        let mut v = 0u32;
        for i in 0..n {
            let at = self.pos + i;
            v |= u32::from((self.buf[at / 8] >> (at % 8)) & 1) << i;
        }
        self.pos += n;
        Ok(v)
    }

    pub fn pos(&self) -> usize {
        self.pos
    }
}

/// One record read back: the items class its code names and the view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadItem {
    /// Combined items index of the code (`ear ` for an ear record).
    pub record: usize,
    pub item: StreamItem,
    /// The record was read to its end but failed (a bad code, quality or
    /// stat list): no item is made from it, its
    /// bytes are used (the entry is skipped).
    pub failed: bool,
}

/// One save item entry: the record, its byte
/// length (padded), and its children (`filled` of them, 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadEntry {
    pub len: usize,
    pub item: ReadItem,
    pub children: Vec<ReadEntry>,
}

/// The combined items index of an item code (the
/// first row with that code).
pub fn record_of(t: &ItemTables, code: [u8; 4]) -> Option<usize> {
    t.items.iter().position(|r| r.code == code)
}

/// The writer's type tests of a record.
pub fn kind_of(t: &ItemTables, record: usize) -> Kind {
    let is = |k: u16| t.is_type(record, k as i16);
    Kind {
        armor: is(ty::ARMO),
        weapon: is(ty::WEAP),
        gold: is(ty::GOLD),
        charm: is(ty::CHAR),
        body_part: is(ty::BODY) && !is(ty::PLAY),
        scroll_or_book: is(ty::SCRO) || is(ty::BOOK),
    }
}

/// "ISC(s)" read back: the written value minus `Save Add`.
fn isc_get(r: &mut BitReader<'_>, c: Isc) -> Result<i32, ReadError> {
    Ok(r.read(u32::from(c.save_bits))?.wrapping_sub(c.save_add) as i32)
}

fn read_name(r: &mut BitReader<'_>) -> Result<[u8; 16], ReadError> {
    let mut name = [0u8; 16];
    for i in 0.. {
        let c = r.read(7)? as u8;
        if c == 0 {
            break;
        }
        // The reader reads to its 0 with no bound; D2RUST keeps
        // the first 16 bytes (+0x4A–+0x59; the setter keeps at most 15,
        // edge case 7, so no longer name arises from D2RUST).
        if let Some(b) = name.get_mut(i) {
            *b = c;
        }
    }
    Ok(name)
}

fn read_gold(r: &mut BitReader<'_>) -> Result<i32, ReadError> {
    Ok(if r.read(1)? == 1 {
        r.read(32)? as i32
    } else {
        r.read(12)? as i32
    })
}

/// The trailer (legacy save format at the
/// current version): 1 bit; when 1, a (+0x1C), b (+0x20) and 32 bits
/// that are read and dropped.
fn read_trailer(r: &mut BitReader<'_>, it: &mut StreamItem) -> Result<(), ReadError> {
    if r.read(1)? == 1 {
        let a = r.read(32)?;
        let b = r.read(32)?;
        r.read(32)?;
        it.save_trailer = Some((a, b));
    }
    Ok(())
}

/// The inverse of a prefix id on the wire (reader):
/// p′ ≠ 0 → p′ + P, 0 → 0 (every base-game prefix id is > P).
fn prefix_from(p: u32) -> u16 {
    if p == 0 {
        0
    } else {
        p as u16 + PREFIX_OFFSET
    }
}

/// One list's stats up to its terminator (inverted): values
/// are shifted back by `ValShift` (the low bits are not on the wire);
/// grouped partners are written without the shift and read the same way.
/// An id with no `itemstatcost` row ends
/// the list with no failure (edge case 1: what follows is then misread);
/// id 0 directly after id 0 fails the record (`failed`) and ends the list.
fn read_list(
    r: &mut BitReader<'_>,
    t: &dyn IscTable,
    failed: &mut bool,
) -> Result<Vec<StatEntry>, ReadError> {
    let mut out = Vec::new();
    let mut last = None;
    loop {
        let s = r.read(9)?;
        if s == TERMINATOR {
            return Ok(out);
        }
        let s = s as u16;
        if !t.has_row(s) {
            return Ok(out);
        }
        if s == 0 && last == Some(0) {
            *failed = true;
            return Ok(out);
        }
        last = Some(s);
        let c = t.isc(s);
        match s {
            17 | 48 | 50 | 52 | 54 | 57 => {
                let v = isc_get(r, c)?;
                out.push(StatEntry {
                    stat: s,
                    param: 0,
                    value: v << c.valshift,
                });
                for &p in super::partners(s) {
                    let v = isc_get(r, t.isc(p))?;
                    out.push(StatEntry {
                        stat: p,
                        param: 0,
                        value: v,
                    });
                }
            }
            // The id only (no value on the wire).
            326 => {}
            _ => {
                let param = if c.save_param_bits > 0 {
                    r.read(c.save_param_bits)? as u16
                } else {
                    0
                };
                let v = isc_get(r, c)?;
                out.push(StatEntry {
                    stat: s,
                    param,
                    value: v << c.valshift,
                });
            }
        }
    }
}

fn read_location(r: &mut BitReader<'_>, it: &mut StreamItem) -> Result<(), ReadError> {
    it.mode = r.read(3)?;
    if it.mode == 3 || it.mode == 5 {
        it.x = r.read(16)? as i32;
        it.y = r.read(16)? as i32;
    } else {
        it.body_loc = r.read(4)? as u8;
        it.x = r.read(4)? as i32;
        it.y = r.read(4)? as i32;
        // Page + 1 in 3 bits; page 0xFF is written as 0.
        it.page = match r.read(3)? {
            0 => 0xFF,
            v => v as u8 - 1,
        };
    }
    Ok(())
}

/// Reads one save-format record (marker, the trailer of the legacy
/// layout): no padding, no children. The flags are kept as stored
/// except 0x80000 and the alt-code bit, which the decoder drops;
/// `compact` and `alt` come from the header.
pub fn read_save_record(r: &mut BitReader<'_>, t: &ItemTables) -> Result<ReadItem, ReadError> {
    let isc: &dyn IscTable = &t.isc;
    let m = r.read(16)?;
    if m != SAVE_MARKER {
        return Err(ReadError::BadMarker(m));
    }
    let f = r.read(32)?;
    let mut it = StreamItem {
        flags: f & !(hflag::INIT | hflag::ALT_CODE),
        compact: f & hflag::COMPACT != 0,
        alt: f & hflag::ALT_CODE != 0,
        version: r.read(10)? as u16,
        ..StreamItem::default()
    };
    read_location(r, &mut it)?;
    let facts = |it: &mut StreamItem, record: usize| {
        it.kind = kind_of(t, record);
        if let Some(rec) = t.item(record) {
            it.stackable = rec.stackable != 0;
            it.quest_diff = rec.quest != 0 && rec.questdiffcheck != 0;
        }
    };
    if it.compact {
        let record = if f & hflag::EAR != 0 {
            it.ear_class = r.read(3)? as i32;
            it.ear_level = r.read(7)? as i32;
            it.name = read_name(r)?;
            it.code = *b"ear ";
            record_of(t, it.code).ok_or(ReadError::UnknownCode(it.code))?
        } else {
            it.code = r.read(32)?.to_le_bytes();
            let record = record_of(t, it.code).ok_or(ReadError::UnknownCode(it.code))?;
            facts(&mut it, record);
            if it.kind.gold {
                it.total_gold = read_gold(r)?;
            }
            record
        };
        facts(&mut it, record);
        if it.quest_diff {
            let c = isc.isc(stat::QUESTITEMDIFFICULTY);
            it.total_quest_diff = isc_get(r, c)? << c.valshift;
        }
        read_trailer(r, &mut it)?;
        // sets item level 1, quality 2 and suffix slot 0
        // (0 for `tsc `, 1 for `isc `); the seed field is 0 (unit28).
        it.ilvl = 1;
        it.quality = 2;
        it.unit28 = 0;
        it.suffix[0] = match &it.code {
            b"tsc " => 0,
            b"isc " => 1,
            _ => it.suffix[0],
        };
        return Ok(ReadItem {
            record,
            item: it,
            failed: false,
        });
    }
    let code = r.read(32)?.to_le_bytes();
    if it.alt {
        // The record ends after the base code; the reader
        // sets item level 1 and quality 1.
        it.code = code;
        it.ilvl = 1;
        it.quality = 1;
        it.base_code = code;
        let record = record_of(t, code).ok_or(ReadError::UnknownCode(code))?;
        facts(&mut it, record);
        return Ok(ReadItem {
            record,
            item: it,
            failed: false,
        });
    }
    it.code = code;
    let record = record_of(t, code).ok_or(ReadError::UnknownCode(code))?;
    facts(&mut it, record);
    let mut failed = false;
    it.filled = r.read(3)?;
    it.unit28 = r.read(32)?;
    it.ilvl = (r.read(7)? as i32).max(1);
    it.quality = r.read(4)? as u8;
    it.varinvgfx = r.read(1)? == 1;
    if it.varinvgfx {
        it.gfx = r.read(3)? as i32;
    }
    if r.read(1)? == 1 {
        // Reader: a′ ≠ 0 → a′ + A, 0 → 0.
        let a = r.read(11)? as u16;
        it.auto_affix = if a == 0 { 0 } else { a + AUTO_OFFSET };
    }
    // "shown" always in the save format.
    match it.quality {
        1 | 3 => it.file_index = r.read(3)? as i32,
        4 => {
            it.prefix[0] = prefix_from(r.read(11)?);
            it.suffix[0] = r.read(11)? as u16;
        }
        // (v ≥ 0x5D): the `setitems`
        // row n gives the file index; no row fails the record.
        5 => {
            let v = r.read(12)?;
            if (v as usize) < t.setitems.len() {
                it.file_index = v as i32;
            } else {
                failed = true;
            }
        }
        7 => {
            let v = r.read(12)?;
            // A unique's index at or above the
            // uniqueitems count is −1 (a negative index is written as
            // 0xFFF).
            it.file_index = if (v as usize) < t.uniques.len() {
                v as i32
            } else {
                -1
            };
        }
        6 | 8 => {
            it.rare_prefix = r.read(8)? as u16;
            it.rare_suffix = r.read(8)? as u16;
            for i in 0..3 {
                if r.read(1)? == 1 {
                    it.prefix[i] = prefix_from(r.read(11)?);
                }
                if r.read(1)? == 1 {
                    it.suffix[i] = r.read(11)? as u16;
                }
            }
        }
        9 => {
            it.rare_prefix = r.read(8)? as u16;
            it.rare_suffix = r.read(8)? as u16;
        }
        2 => {
            if it.kind.charm {
                let is_prefix = r.read(1)? == 1;
                let v = r.read(11)?;
                if is_prefix {
                    it.prefix[0] = prefix_from(v);
                } else {
                    it.suffix[0] = v as u16;
                }
            }
            if it.kind.body_part {
                it.file_index = r.read(10)? as i32;
            }
            if it.kind.scroll_or_book {
                it.suffix[0] = r.read(5)? as u16;
            }
        }
        // Edge case 6: any other
        // quality reads nothing here, fails, and is read on to the end.
        _ => failed = true,
    }
    it.runeword = if f & hflag::RUNEWORD != 0 {
        r.read(16)? as u16
    } else {
        0xFFFF
    };
    if f & hflag::EAR != 0 {
        it.ear_class = r.read(3)? as i32;
        it.ear_level = r.read(7)? as i32;
        it.name = read_name(r)?;
    } else if f & hflag::PERSONALIZED != 0 {
        it.name = read_name(r)?;
    }
    read_trailer(r, &mut it)?;
    let durability = |r: &mut BitReader<'_>, it: &mut StreamItem| -> Result<(), ReadError> {
        it.base_max_dur = isc_get(r, isc.isc(stat::MAXDURABILITY))?;
        if it.base_max_dur != 0 {
            it.total_dur = isc_get(r, isc.isc(stat::DURABILITY))?;
        }
        Ok(())
    };
    if it.kind.armor {
        it.base_defense = isc_get(r, isc.isc(stat::ARMORCLASS))?;
        durability(r, &mut it)?;
    } else if it.kind.weapon {
        durability(r, &mut it)?;
    } else if it.kind.gold {
        it.total_gold = read_gold(r)?;
    }
    if it.stackable {
        it.total_quantity = r.read(9)? as i32;
    }
    if f & hflag::SOCKETED != 0 {
        it.base_sockets = r.read(u32::from(isc.isc(stat::NUMSOCKETS).save_bits))? as i32;
    }
    // The set mask, then the lists c = −1 … L − 1.
    let mut mask = 0u32;
    let mut l = 0usize;
    if it.quality == 5 {
        mask = r.read(5)?;
        l = (32 - mask.leading_zeros()) as usize;
    }
    let runeword = f & hflag::RUNEWORD != 0;
    if runeword {
        l += 1;
    }
    it.main = Some(read_list(r, isc, &mut failed)?);
    for c in 0..l {
        if runeword && c == l - 1 {
            it.runeword_list = Some(read_list(r, isc, &mut failed)?);
        } else if mask & (1 << c) != 0 || runeword {
            let list = read_list(r, isc, &mut failed)?;
            if mask & (1 << c) != 0 {
                it.sets[c] = Some(list);
            }
        }
    }
    Ok(ReadItem {
        record,
        item: it,
        failed,
    })
}

/// Reads one save item entry at `buf[0..]`: the
/// record, zero padding to a whole byte, then its children (the record's
/// `filled` count, 0 for compact and alt-code records:),
/// each an entry of its own.
pub fn read_save_entry(buf: &[u8], t: &ItemTables) -> Result<ReadEntry, ReadError> {
    let mut r = BitReader::new(buf);
    let item = read_save_record(&mut r, t)?;
    let used = r.pos();
    let len = used.div_ceil(8);
    // A failed record stopped inside its data (legacy layout, first
    // step): its length is the bytes read, whatever follows.
    let checked = if item.failed { used } else { len * 8 };
    for at in used..checked {
        if (buf[at / 8] >> (at % 8)) & 1 != 0 {
            return Err(ReadError::Padding(at));
        }
    }
    let n = if item.item.compact || item.item.alt {
        0
    } else {
        item.item.filled
    };
    let mut entry = ReadEntry {
        len,
        item,
        children: Vec::new(),
    };
    for _ in 0..n {
        let rest = buf.get(entry.len..).unwrap_or_default();
        let child = read_save_entry(rest, t).map_err(|e| match e {
            ReadError::Short(b) => ReadError::Short(b + entry.len * 8),
            ReadError::Padding(b) => ReadError::Padding(b + entry.len * 8),
            e => e,
        })?;
        entry.len += child.len;
        entry.children.push(child);
    }
    Ok(entry)
}
