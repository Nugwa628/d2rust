//! Reading one item record, field by field, in stream order.
//!
//! Every record starts with a header: (save format only) the "JM" marker,
//! 32 flag bits, a 10-bit version and the location. What follows depends
//! on the flags:
//!
//! - compact ([`hflag::COMPACT`]): an ear, or a code with its gold amount
//!   and quest difficulty, then (save format) the trailer;
//! - alt-code ([`hflag::ALT_CODE`]): the base code, nothing else;
//! - full: code, socketed-child count, (save) the extra unit field, item
//!   level, quality, picture and automatic affix, the quality fields,
//!   runeword id, ear or personalized name, (save) the trailer, the fields
//!   of the item's type (defense, durability, gold, quantity, sockets), and
//!   for a shown item the set mask and the stat lists.

use super::reader::BitReader;
use super::{
    hflag, CodeFacts, Ear, ItemBits, ItemBitsError, ItemLookup, Location, QualityFields, Stat,
    SAVE_MARKER, TERMINATOR,
};

// itemstatcost ids the reader reads by name.
/// `armorclass`: an armor's defense.
const STAT_ARMORCLASS: u16 = 31;
/// `durability`: current durability.
const STAT_DURABILITY: u16 = 72;
/// `maxdurability`: maximum durability.
const STAT_MAXDURABILITY: u16 = 73;
/// `item_numsockets`: the socket count (its width is its `Save Bits`).
pub(super) const STAT_NUMSOCKETS: u16 = 194;
/// `poison_count`: in a stat list only its id is written (no param, no
/// value); it reads as 0.
const STAT_POISON_COUNT: u16 = 326;
/// `questitemdifficulty`: the difficulty a compact quest item belongs to.
const STAT_QUESTITEMDIFFICULTY: u16 = 356;

// Item qualities (`itemquality` numbering).
const QUALITY_LOW: u8 = 1;
const QUALITY_NORMAL: u8 = 2;
const QUALITY_SUPERIOR: u8 = 3;
const QUALITY_MAGIC: u8 = 4;
const QUALITY_SET: u8 = 5;
const QUALITY_RARE: u8 = 6;
const QUALITY_UNIQUE: u8 = 7;
const QUALITY_CRAFTED: u8 = 8;
const QUALITY_TEMPERED: u8 = 9;

// Item modes whose location is a ground position.
const MODE_ON_GROUND: u8 = 3;
const MODE_DROPPING: u8 = 5;

/// The partners of a grouped stat: the stats whose values follow it in the
/// same entry with no id of their own.
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

/// Reads one record: the network form, or with `save` the save form (the
/// "JM" marker, the extra unit field, the trailer, "shown" always).
pub(super) fn read_record(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    save: bool,
) -> Result<ItemBits, ItemBitsError> {
    let mut it = read_header(r, save)?;
    if it.flags & hflag::COMPACT != 0 {
        read_compact(r, t, &mut it)?;
    } else if it.flags & hflag::ALT_CODE != 0 {
        // Alt-code records carry no trailer, even in the save format.
        it.base_code = Some(read_code(r)?);
    } else {
        read_full(r, t, &mut it)?;
    }
    it.bits = r.pos();
    Ok(it)
}

/// Marker (save only), flags, version and location.
fn read_header(r: &mut BitReader<'_>, save: bool) -> Result<ItemBits, ItemBitsError> {
    if save {
        let marker = r.read(16)?;
        if marker != SAVE_MARKER {
            return Err(ItemBitsError::BadMarker(marker));
        }
    }
    let mut it = ItemBits {
        flags: r.read(32)?,
        save,
        ..ItemBits::default()
    };
    it.version = r.read(10)? as u16;
    read_location(r, &mut it)?;
    Ok(it)
}

/// A compact record: an ear, or a code with its gold and quest difficulty.
fn read_compact(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    it: &mut ItemBits,
) -> Result<(), ItemBitsError> {
    let ear = it.flags & hflag::EAR != 0;
    if ear {
        it.ear = Some(read_ear(r)?);
    } else {
        it.code = read_code(r)?;
        if facts(t, it.code)?.gold {
            it.gold = Some(read_gold(r)?);
        }
    }
    // The difficulty stat needs the code's facts; an ear record has no code.
    if !ear && t.code(it.code).is_some_and(|c| c.quest_diff) {
        it.quest_diff = Some(read_isc(r, t, STAT_QUESTITEMDIFFICULTY, 0)?);
    }
    if it.save {
        read_trailer(r, it)?;
    }
    Ok(())
}

/// A full record, from the code on.
fn read_full(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    it: &mut ItemBits,
) -> Result<(), ItemBitsError> {
    let f = it.flags;
    it.code = read_code(r)?;
    let facts = facts(t, it.code)?;
    it.filled = r.read(3)? as u8;
    if it.save {
        it.save_unit28 = Some(r.read(32)?);
    }
    it.ilvl = r.read(7)? as u8;
    it.quality = r.read(4)? as u8;
    if r.read(1)? == 1 {
        it.gfx = Some(r.read(3)? as u8);
    }
    if r.read(1)? == 1 {
        it.auto_affix = Some(r.read(11)? as u16);
    }
    read_quality_fields(r, t, &facts, it)?;
    if f & hflag::RUNEWORD != 0 {
        it.runeword = Some(r.read(16)? as u16);
    }
    if f & hflag::EAR != 0 {
        it.ear = Some(read_ear(r)?);
    } else if f & hflag::PERSONALIZED != 0 {
        it.name = Some(read_name(r)?);
    }
    if it.save {
        read_trailer(r, it)?;
    }
    read_type_fields(r, t, &facts, it)?;
    if it.shown() {
        read_stat_lists(r, t, it)?;
    }
    Ok(())
}

/// The fields that depend on the quality. Most are sent only for a shown
/// item; the low / superior file index and the rare / crafted affix slots
/// are always sent. A quality outside 1..=9 reads nothing here and fails
/// the record, which is still read on to its end.
fn read_quality_fields(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    facts: &CodeFacts,
    it: &mut ItemBits,
) -> Result<(), ItemBitsError> {
    let shown = it.shown();
    let q: &mut QualityFields = &mut it.quality_fields;
    match it.quality {
        QUALITY_LOW | QUALITY_SUPERIOR => q.file_index = Some(r.read(3)?),
        QUALITY_MAGIC => {
            if shown {
                q.magic = Some((r.read(11)? as u16, r.read(11)? as u16));
            }
        }
        QUALITY_SET | QUALITY_UNIQUE => {
            if shown {
                let n = r.read(12)?;
                q.file_index = Some(n);
                // A set record naming no `setitems` row fails.
                if it.quality == QUALITY_SET
                    && t.set_item_rows().is_some_and(|rows| n as usize >= rows)
                {
                    it.failed = true;
                }
            }
        }
        QUALITY_RARE | QUALITY_CRAFTED => {
            if shown {
                q.rare_names = Some((r.read(8)? as u8, r.read(8)? as u8));
            }
            // Three (prefix, suffix) slots, each a presence bit and an
            // 11-bit affix id when present.
            let mut slots = [(0u16, 0u16); 3];
            for s in slots.iter_mut() {
                let prefix = read_optional_affix(r)?;
                let suffix = read_optional_affix(r)?;
                *s = (prefix, suffix);
            }
            q.rare_slots = Some(slots);
        }
        QUALITY_TEMPERED => {
            if shown {
                q.rare_names = Some((r.read(8)? as u8, r.read(8)? as u8));
            }
        }
        QUALITY_NORMAL => {
            if facts.charm && shown {
                let is_prefix = r.read(1)? == 1;
                q.charm = Some((is_prefix, r.read(11)? as u16));
            }
            if facts.body_part {
                q.file_index = Some(r.read(10)?);
            }
            if facts.scroll_or_book {
                q.spell = Some(r.read(5)? as u8);
            }
        }
        // The server's writer never sends another quality.
        _ => it.failed = true,
    }
    Ok(())
}

/// One rare affix slot: a presence bit, then an 11-bit id (0 when absent).
fn read_optional_affix(r: &mut BitReader<'_>) -> Result<u16, ItemBitsError> {
    Ok(if r.read(1)? == 1 {
        r.read(11)? as u16
    } else {
        0
    })
}

/// Defense and durability (armor), durability (weapons) or the amount
/// (gold), then the stack size and the socket count when they apply.
fn read_type_fields(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    facts: &CodeFacts,
    it: &mut ItemBits,
) -> Result<(), ItemBitsError> {
    if facts.armor {
        it.defense = Some(read_isc(r, t, STAT_ARMORCLASS, 0)?);
        read_durability(r, t, it)?;
    } else if facts.weapon {
        read_durability(r, t, it)?;
    } else if facts.gold {
        it.gold = Some(read_gold(r)?);
    }
    if facts.stackable {
        it.quantity = Some(r.read(9)? as u16);
    }
    if it.flags & hflag::SOCKETED != 0 {
        let c = t
            .isc(STAT_NUMSOCKETS)
            .ok_or(ItemBitsError::UnknownStat(STAT_NUMSOCKETS))?;
        it.sockets = Some(r.read(u32::from(c.save_bits))? as u8);
    }
    Ok(())
}

/// The stat lists of a shown item: the main list, then for a set item one
/// list per bit of its 5-bit set mask up to the highest set bit (absent
/// where the bit is 0), and for a runeword one more list.
fn read_stat_lists(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    it: &mut ItemBits,
) -> Result<(), ItemBitsError> {
    let mut extra_lists = 0usize;
    let mut mask = 0u32;
    if it.quality == QUALITY_SET {
        mask = r.read(5)?;
        it.set_mask = Some(mask as u8);
        extra_lists = (32 - mask.leading_zeros()) as usize;
    }
    let runeword = it.flags & hflag::RUNEWORD != 0;
    if runeword {
        extra_lists += 1;
    }
    let mut failed = it.failed;
    it.lists.push(Some(read_list(r, t, &mut failed)?));
    for c in 0..extra_lists {
        let present = runeword || mask & (1 << c) != 0;
        it.lists.push(if present {
            Some(read_list(r, t, &mut failed)?)
        } else {
            None
        });
    }
    it.failed = failed;
    Ok(())
}

/// One stat list, up to its terminator. An id with no `itemstatcost` row
/// ends it with no failure (what follows is then misread); id 0 directly
/// after id 0 fails the record and ends it. A row with `Save Bits` 0 reads
/// 0 bits.
fn read_list(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    failed: &mut bool,
) -> Result<Vec<Stat>, ItemBitsError> {
    let mut out = Vec::new();
    let mut last = None;
    loop {
        let s = r.read(9)?;
        if s == TERMINATOR {
            return Ok(out);
        }
        let s = s as u16;
        let Some(c) = t.isc(s) else {
            return Ok(out);
        };
        if s == 0 && last == Some(0) {
            *failed = true;
            return Ok(out);
        }
        last = Some(s);
        if !partners(s).is_empty() {
            out.push(read_isc(r, t, s, 0)?);
            for &p in partners(s) {
                out.push(read_isc(r, t, p, 0)?);
            }
        } else if s == STAT_POISON_COUNT {
            out.push(Stat {
                stat: s,
                param: 0,
                raw: 0,
                save_add: 0,
            });
        } else {
            let param = if c.save_param_bits > 0 {
                r.read(c.save_param_bits)?
            } else {
                0
            };
            out.push(read_isc(r, t, s, param)?);
        }
    }
}

/// The code's facts; an unknown code is an error.
fn facts(t: &dyn ItemLookup, code: [u8; 4]) -> Result<CodeFacts, ItemBitsError> {
    t.code(code).ok_or(ItemBitsError::UnknownCode(code))
}

/// A stat value of `Save Bits` width; a stat without a row is an error.
fn read_isc(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    s: u16,
    param: u32,
) -> Result<Stat, ItemBitsError> {
    let c = t.isc(s).ok_or(ItemBitsError::UnknownStat(s))?;
    Ok(Stat {
        stat: s,
        param,
        raw: r.read(u32::from(c.save_bits))?,
        save_add: c.save_add,
    })
}

/// Maximum durability, then the current durability when the maximum is not
/// 0 (indestructible items have no current value).
fn read_durability(
    r: &mut BitReader<'_>,
    t: &dyn ItemLookup,
    it: &mut ItemBits,
) -> Result<(), ItemBitsError> {
    let max = read_isc(r, t, STAT_MAXDURABILITY, 0)?;
    it.max_durability = Some(max);
    if max.value() != 0 {
        it.durability = Some(read_isc(r, t, STAT_DURABILITY, 0)?);
    }
    Ok(())
}

/// Mode (3 bits), then a ground position (16 + 16 bits) or a slot (body
/// location, x, y: 4 bits each; page + 1: 3 bits).
fn read_location(r: &mut BitReader<'_>, it: &mut ItemBits) -> Result<(), ItemBitsError> {
    it.mode = r.read(3)? as u8;
    it.location = Some(if it.mode == MODE_ON_GROUND || it.mode == MODE_DROPPING {
        Location::Ground {
            x: r.read(16)? as u16,
            y: r.read(16)? as u16,
        }
    } else {
        Location::Slot {
            body: r.read(4)? as u8,
            x: r.read(4)? as u8,
            y: r.read(4)? as u8,
            page1: r.read(3)? as u8,
        }
    });
    Ok(())
}

/// A four-character item code (32 bits, first character in the low byte).
fn read_code(r: &mut BitReader<'_>) -> Result<[u8; 4], ItemBitsError> {
    Ok(r.read(32)?.to_le_bytes())
}

/// A gold amount: a width bit, then 32 bits when it is 1, else 12 bits.
fn read_gold(r: &mut BitReader<'_>) -> Result<u32, ItemBitsError> {
    if r.read(1)? == 1 {
        r.read(32)
    } else {
        r.read(12)
    }
}

/// A name, 7 bits per character, ended by a 0 character. No length bound:
/// the stream's length is the only limit.
fn read_name(r: &mut BitReader<'_>) -> Result<Vec<u8>, ItemBitsError> {
    let mut out = Vec::new();
    loop {
        let c = r.read(7)? as u8;
        if c == 0 {
            return Ok(out);
        }
        out.push(c);
    }
}

/// An ear: the class (3 bits) and level (7 bits) of the player it came
/// from, then their name.
fn read_ear(r: &mut BitReader<'_>) -> Result<Ear, ItemBitsError> {
    Ok(Ear {
        class: r.read(3)? as u8,
        level: r.read(7)? as u8,
        name: read_name(r)?,
    })
}

/// Save trailer, present when its first bit is 1: two 32-bit values that
/// are kept, then 32 bits read and dropped.
fn read_trailer(r: &mut BitReader<'_>, it: &mut ItemBits) -> Result<(), ItemBitsError> {
    if r.read(1)? == 1 {
        let a = r.read(32)?;
        let b = r.read(32)?;
        r.read(32)?;
        it.save_trailer = Some((a, b));
    }
    Ok(())
}
