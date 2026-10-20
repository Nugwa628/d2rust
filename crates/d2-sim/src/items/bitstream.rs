//! The item bit stream of S→C 0x9C / 0x9D (network case:
//! save off, children off): the header, the compact record and
//! the full record written LSB first into a 0xF4-byte buffer.
//! The same writer produces the save format ([`write_save`]: save on,
//! children on).
//!
//! The writer reads one resolved view of the item ([`StreamItem`]: the
//! item data fields, the items / itemtypes facts and the stat values the
//! rules name) and the itemstatcost save columns ([`IscTable`]). Who fills
//! the view is the caller's (`wiring::inventory`: item store, tables, stat
//! lists, inventory state); no rule is decided there.
//!
//! Status: implemented, not yet checked against a live game; the ten
//! measured reference streams B1–B10 are re-encoded byte for byte
//! in the tests. The save format is tested on hand-computed
//! synthetic vectors only.

pub mod read;
#[cfg(test)]
mod read_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod tests;

/// Buffer size of the senders.
pub const BUFFER: usize = 0xF4;
/// Capacity of the save writer ([`write_save`]): the d2s writer's file
/// buffer of 0x2000 bytes. One item entry
/// (item plus children) never exceeds the whole file, so this is an upper
/// bound, not the space the d2s writer has left (owner:
/// Step 5).
pub const SAVE_BUFFER: usize = 0x2000;
/// Save-format marker "JM".
pub const SAVE_MARKER: u32 = 0x4D4A;
/// List terminator.
pub const TERMINATOR: u32 = 0x1FF;
/// Prefix part's first combined index in the base game.
pub const PREFIX_OFFSET: u16 = 747;
/// Automagic part's first combined index in the base game.
pub const AUTO_OFFSET: u16 = 1416;
/// Set-list states S_0..S_4 (table).
pub const SET_STATES: [u32; 5] = [165, 166, 167, 168, 169];
/// Runeword list state.
pub const RUNEWORD_STATE: u32 = 171;

/// Header flag bits the writer reads or forces.
pub mod hflag {
    pub const IDENTIFIED: u32 = 0x10;
    pub const SOCKETED: u32 = 0x800;
    pub const EAR: u32 = 0x10000;
    pub const INIT: u32 = 0x80000;
    pub const COMPACT: u32 = 0x200000;
    pub const ETHEREAL: u32 = 0x400000;
    pub const FORCED: u32 = 0x800000;
    pub const PERSONALIZED: u32 = 0x1000000;
    pub const ALT_CODE: u32 = 0x2000000;
    pub const RUNEWORD: u32 = 0x4000000;
}

/// The itemstatcost columns the stream reads (Constants: `ValShift`,
/// `Save Bits`, `Save Add`, `Save Param Bits`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Isc {
    pub valshift: u8,
    pub save_bits: u8,
    pub save_add: u32,
    pub save_param_bits: u32,
}

/// The itemstatcost rows by stat id. A stat without a row reads as all
/// zero (`Save Bits` 0: never written).
pub trait IscTable {
    fn isc(&self, stat: u16) -> Isc;
    /// The stat has an `itemstatcost` row (the reader ends a list at an id
    /// without one).
    fn has_row(&self, stat: u16) -> bool {
        self.isc(stat).save_bits != 0
    }
}

impl IscTable for [Isc] {
    fn isc(&self, stat: u16) -> Isc {
        self.get(usize::from(stat)).copied().unwrap_or_default()
    }
    fn has_row(&self, stat: u16) -> bool {
        usize::from(stat) < self.len()
    }
}

impl IscTable for Vec<Isc> {
    fn isc(&self, stat: u16) -> Isc {
        self.as_slice().isc(stat)
    }
    fn has_row(&self, stat: u16) -> bool {
        self.as_slice().has_row(stat)
    }
}

/// One stat-list entry in list order (param, id, value).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatEntry {
    pub stat: u16,
    pub param: u16,
    pub value: i32,
}

/// The type tests of the stream (with equivalence).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kind {
    /// Type 50 `armo`.
    pub armor: bool,
    /// Type 45 `weap`.
    pub weapon: bool,
    /// Type 4 `gold`.
    pub gold: bool,
    /// Type 13 `char`.
    pub charm: bool,
    /// Type 40 `body` and not type 7 `play`.
    pub body_part: bool,
    /// Type 22 `scro` or 18 `book`.
    pub scroll_or_book: bool,
}

/// The resolved fields of one item the writer reads (Inputs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamItem {
    /// Item flags (item data +0x18), already OR-ed with the sender's flag
    /// argument.
    pub flags: u32,
    /// The alt-code argument (no sender passes it).
    pub alt: bool,
    /// Items `compactsave` ≠ 0.
    pub compact: bool,
    /// Item data +0x30.
    pub version: u16,
    /// Unit mode (+0x10).
    pub mode: u32,
    /// Static path x / y: sub-tile position on the ground (modes 3, 5),
    /// grid / slot position otherwise.
    pub x: i32,
    pub y: i32,
    /// Body location (+0x44).
    pub body_loc: u8,
    /// Page (+0x45; 0xFF none).
    pub page: u8,
    /// Items `code` (first character in the low byte).
    pub code: [u8; 4],
    /// Items +0x84 (`code` when 0).
    pub base_code: [u8; 4],
    /// Ear class (file index) and ear level (+0x48).
    pub ear_class: i32,
    pub ear_level: i32,
    /// Name (+0x4A), NUL-ended.
    pub name: [u8; 16],
    /// Filled sockets: items in the item's own inventory when items
    /// `hasinv` ≠ 0, else 0.
    pub filled: u32,
    /// Item level (+0x2C).
    pub ilvl: i32,
    /// Quality (+0x00).
    pub quality: u8,
    /// File index (+0x28).
    pub file_index: i32,
    /// The primary type's `varinvgfx` ≠ 0, and the gfx variant (+0x49).
    pub varinvgfx: bool,
    pub gfx: i32,
    /// +0x36 (u16).
    pub auto_affix: u16,
    /// Magic prefix / suffix slots (+0x38.., +0x3E..).
    pub prefix: [u16; 3],
    pub suffix: [u16; 3],
    /// Rare names.
    pub rare_prefix: u16,
    pub rare_suffix: u16,
    /// The runeword record's +0x82 (0xFFFF without a record).
    pub runeword: u16,
    pub kind: Kind,
    /// Items `stackable` ≠ 0.
    pub stackable: bool,
    /// Items `quest` ≠ 0 and `questdiffcheck` ≠ 0.
    pub quest_diff: bool,
    /// Base values: 31 defense, 73 max durability, 194
    /// sockets.
    pub base_defense: i32,
    pub base_max_dur: i32,
    pub base_sockets: i32,
    /// Total values: 72 durability, 14 gold, 70 quantity,
    /// 356 quest difficulty.
    pub total_dur: i32,
    pub total_gold: i32,
    pub total_quantity: i32,
    pub total_quest_diff: i32,
    /// The main list (state 0, flag 0x40) in list order; `None` when the
    /// item has none.
    pub main: Option<Vec<StatEntry>>,
    /// The set lists of states 165–169 (flags 0x2040, else 0x40).
    pub sets: [Option<Vec<StatEntry>>; 5],
    /// The runeword list (state 171, flag 0x40).
    pub runeword_list: Option<Vec<StatEntry>>,
    /// Save format only: unit +0x28, written as is in 32
    /// bits. Its meaning is not known; it is carried through unchanged.
    pub unit28: u32,
    /// Save format only: `None` → 1 bit 0; `Some((a, b))` →
    /// 1 bit 1, 32 bits a, 32 bits b, 32 bits 0. What `a` and `b` mean is
    /// not known; they are carried through unchanged.
    pub save_trailer: Option<(u32, u32)>,
    /// Save format with children: the items of the item's own
    /// inventory in list order, each written as a complete stream after
    /// the item. Never on the wire.
    pub children: Vec<StreamItem>,
}

/// The item fields the writer changes while serializing (step 6 of the
/// write).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteBack {
    pub ilvl: i32,
    pub quality: u8,
}

/// The buffer filled up: the message carries no stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("item bit stream passes the end of its buffer")]
pub struct Overflow;

/// LSB-first bit writer.
#[derive(Clone, Debug)]
pub struct BitWriter {
    buf: Vec<u8>,
    bits: usize,
    cap: usize,
    overflow: bool,
}

impl BitWriter {
    /// A writer over `cap` bytes.
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            bits: 0,
            cap,
            overflow: false,
        }
    }

    /// Writes bits 0..n−1 of `v` raw (no clamp). A write that would pass
    /// the end sets the overflow flag and writes nothing further.
    pub fn raw(&mut self, n: u32, v: u32) {
        if self.overflow || self.bits + n as usize > self.cap * 8 {
            self.overflow = true;
            return;
        }
        for i in 0..n {
            let at = self.bits + i as usize;
            if at / 8 == self.buf.len() {
                self.buf.push(0);
            }
            if (v >> i) & 1 != 0 {
                self.buf[at / 8] |= 1 << (at % 8);
            }
        }
        self.bits += n as usize;
    }

    /// Writes `v` clamped to `n` bits.
    pub fn put(&mut self, n: u32, v: i32) {
        self.raw(n, clamp(n, v));
    }

    pub fn bit_len(&self) -> usize {
        self.bits
    }

    pub fn overflowed(&self) -> bool {
        self.overflow
    }

    /// Pads the stream to a whole byte with zero bits (a partial byte
    /// counts as written).
    pub fn pad_to_byte(&mut self) {
        self.bits = self.bits.div_ceil(8) * 8;
    }

    /// The whole bytes written (a partial byte counts).
    pub fn finish(self) -> Result<Vec<u8>, Overflow> {
        if self.overflow {
            Err(Overflow)
        } else {
            Ok(self.buf)
        }
    }
}

/// Clamp: n < 32 → min(v as u32, 2^n − 1); n = 32 → v.
pub fn clamp(n: u32, v: i32) -> u32 {
    let u = v as u32;
    if n >= 32 {
        u
    } else {
        u.min((1u32 << n) - 1)
    }
}

/// "ISC(s) value v".
fn isc_put(w: &mut BitWriter, t: &dyn IscTable, s: u16, v: i32) {
    let c = t.isc(s);
    w.put(u32::from(c.save_bits), v.wrapping_add(c.save_add as i32));
}

/// Affix id on the wire: a prefix p > P is sent as p − P.
pub fn prefix_id(p: u16) -> u16 {
    if p > PREFIX_OFFSET {
        p - PREFIX_OFFSET
    } else {
        p
    }
}

/// The header flags F (network).
pub fn header_flags(item: &StreamItem) -> u32 {
    header_flags_in(item, false)
}

/// The header flags F; the socketed bit is cleared for an
/// unidentified item on the network only.
fn header_flags_in(item: &StreamItem, save: bool) -> u32 {
    let mut f = (item.flags & !hflag::INIT) | hflag::FORCED;
    if item.compact {
        f |= hflag::COMPACT;
    }
    if item.alt {
        f = (f & !hflag::ETHEREAL) | hflag::ALT_CODE;
    }
    if !save && f & hflag::IDENTIFIED == 0 {
        f &= !hflag::SOCKETED;
    }
    f
}

fn code_u32(c: [u8; 4]) -> u32 {
    u32::from_le_bytes(c)
}

/// Name as 7-bit characters up to and including the terminating 0 (step 3
/// of the write).
fn put_name(w: &mut BitWriter, name: &[u8; 16]) {
    for &c in name.iter() {
        w.put(7, i32::from(c));
        if c == 0 {
            return;
        }
    }
    // A name filling all 16 bytes is followed by the 0 at +0x5A (rule
    // 3): the 16 characters, then 0.
    w.put(7, 0);
}

fn put_ear(w: &mut BitWriter, item: &StreamItem) {
    w.put(3, item.ear_class);
    w.put(7, item.ear_level);
    put_name(w, &item.name);
}

/// Gold: 1 bit (≥ 0x1000), then 32 or 12 bits.
fn put_gold(w: &mut BitWriter, gold: i32) {
    if gold >= 0x1000 {
        w.raw(1, 1);
        w.put(32, gold);
    } else {
        w.raw(1, 0);
        w.put(12, gold);
    }
}

/// Mode and location ((mode) – 3). The compact record
/// writes the same fields (vectors B1, B2).
fn put_location(w: &mut BitWriter, item: &StreamItem) {
    w.put(3, item.mode as i32);
    if item.mode == 3 || item.mode == 5 {
        w.put(16, item.x);
        w.put(16, item.y);
    } else {
        w.put(4, i32::from(item.body_loc));
        w.put(4, item.x);
        w.put(4, item.y);
        w.put(3, i32::from(item.page.wrapping_add(1)));
    }
}

/// Writes the network stream of `item`. The fields the writer
/// changes in the item are returned beside the bytes; the caller writes
/// them back.
pub fn write(item: &StreamItem, t: &dyn IscTable) -> Result<(Vec<u8>, WriteBack), Overflow> {
    let mut w = BitWriter::new(BUFFER);
    let wb = write_into(&mut w, item, t);
    Ok((w.finish()?, wb))
}

/// [`write`] on a caller's writer.
pub fn write_into(w: &mut BitWriter, item: &StreamItem, t: &dyn IscTable) -> WriteBack {
    write_mode(w, item, t, false)
}

/// Writes the save-format entry of `item` (save on, children on; rules
/// 2 and 5): the item's own stream
/// starting with "JM", padded to a whole byte, then each child of
/// [`StreamItem::children`] as a complete entry of its own (recursively),
/// each padded to a whole byte. The write-backs are every written item's
/// (a child goes through the same record writer), in write
/// order: the item, then each child's own, depth first. Capacity:
/// [`SAVE_BUFFER`].
pub fn write_save(
    item: &StreamItem,
    t: &dyn IscTable,
) -> Result<(Vec<u8>, Vec<WriteBack>), Overflow> {
    let mut w = BitWriter::new(SAVE_BUFFER);
    let wb = write_save_into(&mut w, item, t);
    Ok((w.finish()?, wb))
}

/// [`write_save`] on a caller's writer.
pub fn write_save_into(w: &mut BitWriter, item: &StreamItem, t: &dyn IscTable) -> Vec<WriteBack> {
    let mut out = vec![write_mode(w, item, t, true)];
    w.pad_to_byte();
    for child in &item.children {
        out.extend(write_save_into(w, child, t));
    }
    out
}

/// One item's own stream, network or save format.
fn write_mode(w: &mut BitWriter, item: &StreamItem, t: &dyn IscTable, save: bool) -> WriteBack {
    let mut wb = WriteBack {
        ilvl: item.ilvl,
        quality: item.quality,
    };
    let f = header_flags_in(item, save);
    if save {
        w.raw(16, SAVE_MARKER);
    }
    w.raw(32, f);
    if f & hflag::COMPACT != 0 {
        compact(w, item, f, t, save);
    } else {
        full(w, item, f, t, &mut wb, save);
    }
    wb
}

/// Trailer.
fn put_trailer(w: &mut BitWriter, item: &StreamItem) {
    match item.save_trailer {
        None => w.raw(1, 0),
        Some((a, b)) => {
            w.raw(1, 1);
            w.raw(32, a);
            w.raw(32, b);
            w.raw(32, 0);
        }
    }
}

fn compact(w: &mut BitWriter, item: &StreamItem, f: u32, t: &dyn IscTable, save: bool) {
    w.put(10, i32::from(item.version));
    put_location(w, item);
    if f & hflag::EAR != 0 {
        put_ear(w, item);
    } else {
        w.raw(32, code_u32(item.code));
        if item.kind.gold {
            put_gold(w, item.total_gold);
        }
    }
    if item.quest_diff {
        put_quest_diff(w, item, t);
    }
    if save {
        put_trailer(w, item);
    }
}

fn put_quest_diff(w: &mut BitWriter, item: &StreamItem, t: &dyn IscTable) {
    let s = crate::items::stat::QUESTITEMDIFFICULTY;
    isc_put(w, t, s, item.total_quest_diff >> t.isc(s).valshift);
}

fn full(
    w: &mut BitWriter,
    item: &StreamItem,
    f: u32,
    t: &dyn IscTable,
    wb: &mut WriteBack,
    save: bool,
) {
    use crate::items::stat;
    // 4.1 head
    w.put(10, i32::from(item.version));
    put_location(w, item);
    if f & hflag::ALT_CODE != 0 {
        let base = if item.base_code == [0; 4] {
            item.code
        } else {
            item.base_code
        };
        w.raw(32, code_u32(base));
        return;
    }
    w.raw(32, code_u32(item.code));
    w.put(3, item.filled as i32);
    if save {
        w.raw(32, item.unit28);
    }
    if wb.ilvl < 1 {
        wb.ilvl = 1;
    }
    w.put(7, wb.ilvl.min(99));
    w.put(4, i32::from(item.quality));
    w.raw(1, u32::from(item.varinvgfx));
    if item.varinvgfx {
        w.put(3, item.gfx);
    }
    let a = if item.auto_affix > AUTO_OFFSET {
        item.auto_affix - AUTO_OFFSET
    } else {
        item.auto_affix
    };
    w.raw(1, u32::from(a != 0));
    if a != 0 {
        w.put(11, i32::from(a));
    }

    // 4.3 by quality ("shown" = save format, or identified)
    let shown = save || f & hflag::IDENTIFIED != 0;
    let mut skip_lists = false;
    match item.quality {
        1 | 3 => w.put(3, item.file_index),
        4 => {
            if shown {
                w.put(11, i32::from(prefix_id(item.prefix[0])));
                w.put(11, i32::from(item.suffix[0]));
            }
        }
        5 | 7 => {
            if shown {
                w.put(12, item.file_index);
            }
        }
        6 | 8 => {
            if shown {
                w.put(8, i32::from(item.rare_prefix));
                w.put(8, i32::from(item.rare_suffix));
            }
            for i in 0..3 {
                for id in [prefix_id(item.prefix[i]), item.suffix[i]] {
                    w.raw(1, u32::from(id != 0));
                    if id != 0 {
                        w.put(11, i32::from(id));
                    }
                }
            }
        }
        9 => {
            if shown {
                w.put(8, i32::from(item.rare_prefix));
                w.put(8, i32::from(item.rare_suffix));
            }
        }
        q => {
            if q != 2 {
                wb.quality = 2;
                skip_lists = true;
            }
            if item.kind.charm && shown {
                let p = prefix_id(item.prefix[0]);
                w.raw(1, u32::from(p != 0));
                w.put(11, i32::from(if p != 0 { p } else { item.suffix[0] }));
            }
            if item.kind.body_part {
                w.put(10, item.file_index);
            }
            if item.kind.scroll_or_book {
                w.put(5, i32::from(item.suffix[0]));
            }
        }
    }

    // 4.4 runeword and names
    if f & hflag::RUNEWORD != 0 {
        w.raw(16, u32::from(item.runeword));
    }
    if f & hflag::EAR != 0 {
        put_ear(w, item);
    } else if f & hflag::PERSONALIZED != 0 {
        put_name(w, &item.name);
    }
    if save {
        put_trailer(w, item);
    }

    // 4.5 type-specific values
    if item.kind.armor {
        isc_put(w, t, stat::ARMORCLASS, item.base_defense);
        put_durability(w, item, t);
    } else if item.kind.weapon {
        put_durability(w, item, t);
    } else if item.kind.gold {
        put_gold(w, item.total_gold);
    }
    if item.stackable {
        w.put(9, item.total_quantity);
    }
    // The item's own flags, not F (an unidentified item's
    // header word has 0x800 cleared, but the count is sent).
    if item.flags & hflag::SOCKETED != 0 {
        w.put(
            u32::from(t.isc(stat::NUMSOCKETS).save_bits),
            item.base_sockets,
        );
    }
    if !shown {
        return;
    }

    // 4.6 property lists
    let mut l = 0usize;
    if item.quality == 5 {
        let mut m = 0u32;
        for (i, s) in item.sets.iter().enumerate() {
            if s.is_some() {
                m |= 1 << i;
            }
        }
        w.raw(5, m);
        l = (32 - m.leading_zeros()) as usize;
    }
    let runeword = f & hflag::RUNEWORD != 0;
    if runeword {
        l += 1;
    }
    let mut lists: Vec<Option<&Vec<StatEntry>>> = vec![item.main.as_ref()];
    for c in 0..l {
        if runeword && c == l - 1 {
            lists.push(item.runeword_list.as_ref());
        } else {
            lists.push(item.sets.get(c).and_then(|s| s.as_ref()));
        }
    }
    for (c, list) in lists.into_iter().enumerate() {
        if let Some(list) = list {
            if !skip_lists {
                put_list(w, list, t);
            }
        }
        if c == 0 || list.is_some() || runeword {
            w.raw(9, TERMINATOR);
        }
    }
}

fn put_durability(w: &mut BitWriter, item: &StreamItem, t: &dyn IscTable) {
    use crate::items::stat;
    isc_put(w, t, stat::MAXDURABILITY, item.base_max_dur);
    if item.base_max_dur != 0 {
        isc_put(w, t, stat::DURABILITY, item.total_dur);
    }
}

/// The partners of a grouped stat.
pub fn partners(s: u16) -> &'static [u16] {
    match s {
        17 => &[18],
        48 => &[49],
        50 => &[51],
        52 => &[53],
        54 => &[55, 56],
        57 => &[58, 59],
        _ => &[],
    }
}

/// One list's stats.
fn put_list(w: &mut BitWriter, list: &[StatEntry], t: &dyn IscTable) {
    // The record is per list (all slots zeroed before each
    // list).
    let mut recorded: Vec<(u16, i32)> = Vec::new();
    // A stat's value in the same list (layer 0).
    let value_of = |s: u16| {
        list.iter()
            .find(|e| e.stat == s && e.param == 0)
            .map_or(0, |e| e.value)
    };
    for e in list.iter().take(511) {
        let (s, v) = (e.stat, e.value);
        let c = t.isc(s);
        if c.save_bits == 0
            || v >> c.valshift == 0
            || recorded.iter().any(|&(rs, rv)| rs == s && rv == v)
        {
            continue;
        }
        w.raw(9, u32::from(s));
        match s {
            17 | 48 | 50 | 52 | 54 | 57 => {
                isc_put(w, t, s, v >> c.valshift);
                for &p in partners(s) {
                    let pv = value_of(p);
                    isc_put(w, t, p, pv);
                    recorded.push((p, pv));
                }
            }
            326 => {}
            _ => {
                if c.save_param_bits > 0 {
                    w.put(c.save_param_bits, i32::from(e.param));
                }
                isc_put(w, t, s, v >> c.valshift);
            }
        }
    }
}
