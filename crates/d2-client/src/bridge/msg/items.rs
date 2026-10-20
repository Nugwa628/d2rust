//! Hireling stats (0x9E–0xA2) and the item state messages (0x3E,
//! 0x40, 0x7C, 0x7D, 0x92, 0x97, 0xA6).
//!
//! The requirement refresh (open question 4),
//! the gfx refreshes and the stat-list links of items to their owners
//! (the item stream, open question 3) set no
//! model field yet.

use super::super::bits::BitReader;
use super::super::dispatch::{HandlerError, Message};
use super::super::world::{ClientWorld, KindData, UnitKey, ITEM, MONSTER, PLAYER};
use super::Bytes;

/// 0x9E–0xA2: stat u8@1, GUID u32@2, value @6 (u8, u16, u32 set;
/// u8, u16 add) on the monster (1, GUID).
pub fn merc_stat(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let (expected, value, add) = match msg.id {
        0x9E => (7, u32::from(b.u8(6)?), false),
        0x9F => (8, u32::from(b.u16(6)?), false),
        0xA0 => (10, b.u32(6)?, false),
        0xA1 => (7, u32::from(b.u8(6)?), true),
        0xA2 => (8, u32::from(b.u16(6)?), true),
        _ => return Err(HandlerError::Invalid("not 0x9E..=0xA2")),
    };
    if msg.bytes.len() != expected {
        return Err(HandlerError::Invalid("hireling stat message size"));
    }
    let stat = u16::from(b.u8(1)?);
    let key = UnitKey::new(MONSTER, b.u32(2)?);
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    // Step 3: stat 12 first runs the requirement refresh of 0x47 (no
    // model field, module doc).
    let v = if add {
        u.stat(stat).wrapping_add(value as i32)
    } else {
        value as i32
    };
    u.stats.insert(stat, v);
    Ok(())
}

/// The bit-width choice of 0x3E: 1 bit a; 0 → 8 bits, else 1
/// bit b: 16 (b = 0) or 32.
fn sized(r: &mut BitReader<'_>) -> u32 {
    if r.read(1) == 0 {
        r.read(8)
    } else if r.read(1) == 0 {
        r.read(16)
    } else {
        r.read(32)
    }
}

/// 0x3E UpdateItemStats: size u8@1, the bit fields from @2.
pub fn update_item_stats(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let size = usize::from(b.u8(1)?);
    if size != msg.bytes.len() || size < 2 {
        return Err(HandlerError::Invalid("0x3E size byte"));
    }
    let mut r = BitReader::new(&msg.bytes[2..]);
    let guid = sized(&mut r);
    let set = r.read(1) == 1;
    let stat = r.read(9) as u16;
    let value = sized(&mut r) as i32;
    let param = if r.read(1) == 0 {
        r.read(8)
    } else {
        r.read(16)
    };
    let key = UnitKey::new(ITEM, guid);
    let local = w.local().is_some();
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    if stat == 204 {
        // Step 1.1:
        // only with a local player, the
        // item's first stat-204 entry of its flag-0x40 lists whose layer is
        // the key (skill and level, which the param already is) takes
        // max << 8 | min(charges, max), max = the entry >> 8 in 1…255;
        // max 0 or past 255 writes nothing. The search of
        // the item's own inventory (sockets) is not modelled: the model
        // holds no inventory nodes, so an item without the entry stops.
        if local {
            if let KindData::Item(d) = &mut u.kind {
                let entry = d
                    .props
                    .iter_mut()
                    .find(|p| p.stat == 204 && u32::from(p.layer) == param);
                if let Some(p) = entry {
                    let max = p.value >> 8;
                    if (1..=255).contains(&max) {
                        p.value = max << 8 | (value & 0xFF).min(max);
                    }
                }
            }
        }
        return Ok(());
    }
    // Step 1.2.
    if set {
        u.stats.insert(stat, value);
        // The set writes the item's base array, layer 0.
        if let KindData::Item(d) = &mut u.kind {
            if let Some(f) = d.fields.as_mut() {
                set_base(&mut f.base, stat, value);
            }
        }
    }
    if local && stat == 70 && value > 0 {
        if let KindData::Item(d) = &mut u.kind {
            d.set_flags(4 | 0x4000, false);
        }
    }
    Ok(())
}

/// Sets `stat` layer 0 in a base array kept in (stat, layer) order.
fn set_base(base: &mut Vec<super::super::item_lists::ItemProp>, stat: u16, value: i32) {
    match base.binary_search_by_key(&(stat, 0), |p| (p.stat, p.layer)) {
        Ok(i) => base[i].value = value,
        Err(i) => base.insert(
            i,
            super::super::item_lists::ItemProp {
                stat,
                layer: 0,
                value,
            },
        ),
    }
}

/// 0x40 ItemFlags: GUID u32@1, mask u32@5, value u32@9.
pub fn item_flags(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 13 {
        return Err(HandlerError::Invalid("0x40 is 13 bytes"));
    }
    let key = UnitKey::new(ITEM, b.u32(1)?);
    let (mask, value) = (b.u32(5)?, b.u32(9)?);
    if let Some(KindData::Item(d)) = w.units.get_mut(&key).map(|u| &mut u.kind) {
        d.set_flags(mask, value != 0);
    }
    Ok(())
}

/// The state-54 clear of a unit (also 0x3F step 2.1): a state-54 list →
/// state 54 off, the list freed.
pub fn clear_scroll_state(w: &mut ClientWorld, key: UnitKey) {
    if w.units
        .get(&key)
        .is_some_and(|u| u.state_lists.contains_key(&54))
    {
        // The bit and the
        // list, no colour call (that is 0xA9's).
        if let Some(u) = w.units.get_mut(&key) {
            u.states.remove(&54);
            u.state_lists.remove(&54);
        }
    }
}

/// 0x7C UseScroll: type u8@1, GUID u32@2.
pub fn use_scroll(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 6 {
        return Err(HandlerError::Invalid("0x7C is 6 bytes"));
    }
    clear_scroll_state(w, UnitKey::new(b.u8(1)?, b.u32(2)?));
    Ok(())
}

/// 0x7D SetItemState: owner type u8@1, owner GUID u32@2, item
/// GUID u32@6, code u32@10, value u32@14.
pub fn set_item_state(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 18 {
        return Err(HandlerError::Invalid("0x7D is 18 bytes"));
    }
    let owner = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let item = UnitKey::new(ITEM, b.u32(6)?);
    let (code, value) = (b.u32(10)?, b.u32(14)?);
    if !w.units.contains_key(&owner) {
        return Ok(());
    }
    let Some(KindData::Item(d)) = w.units.get_mut(&item).map(|u| &mut u.kind) else {
        return Ok(());
    };
    match code {
        0x100 => d.set_flags(0x100, value != 0),
        0x200 => d.set_flags(0x100, false),
        _ => {}
    }
    Ok(())
}

/// 0x92 RemoveItemsDisplay (settled): type u8@1,
/// GUID u32@2. Only a unit with an inventory changes: then
/// remove every item of the inventory (any node kind: body,
/// grid, belt; the item units stay in S) and the item's stat list is
/// detached unless item flag 0x100 is set (already detached). The nodes
/// are the item units whose last record names U as owner (a 0x9C item
/// belongs to the local player), `store` items excluded. In the model
/// that is
/// [`ItemData::unlinked`](super::super::world::ItemData::unlinked): the
/// item's properties stop counting until its next record re-adds it.
/// The gfx refreshes, the requirement refresh
/// and the final with (inventory) write no model field.
pub fn remove_items_display(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    if msg.bytes.len() != 6 {
        return Err(HandlerError::Invalid("0x92 is 6 bytes"));
    }
    let b = Bytes(msg.bytes);
    let unit = UnitKey::new(b.u8(1)?, b.u32(2)?);
    if !matches!(unit.unit_type, PLAYER | MONSTER) || !w.units.contains_key(&unit) {
        return Ok(());
    }
    let nodes: Vec<UnitKey> = w
        .units
        .keys()
        .copied()
        .filter(|&k| k.unit_type == ITEM)
        .filter(|&k| {
            super::super::items::item(w, k).is_some_and(|v| !v.store && v.owner == Some(unit))
        })
        .collect();
    for k in nodes {
        if let Some(KindData::Item(d)) = w.units.get_mut(&k).map(|u| &mut u.kind) {
            d.unlinked = true;
        }
    }
    Ok(())
}

/// 0x97 WeaponSwitch: with `d2exp.mpq` and an expansion game,
/// `weapon_set` := 1 − itself.
pub fn weapon_switch(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    if msg.bytes.len() != 1 {
        return Err(HandlerError::Invalid("0x97 is 1 byte"));
    }
    if msg.inputs.expansion_installed && w.expansion != 0 {
        w.weapon_set = 1 - w.weapon_set.min(1);
    }
    Ok(())
}

/// The runtime item table entry size.
pub const ITEM_ENTRY: usize = 0x120;

/// 0xA6: code u8@1, size u16@2, index u16@4, record @6 (0x120
/// bytes always copied; a shorter message is a handler error).
pub fn item_table_entry(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let size = usize::from(b.u16(2)?);
    if size != msg.bytes.len() || size < 4 {
        return Err(HandlerError::Invalid("0xA6 size word"));
    }
    if b.u8(1)? != 0 {
        return Ok(());
    }
    let index = usize::from(b.u16(4)?);
    let record = b
        .slice(6, ITEM_ENTRY)
        .map_err(|_| HandlerError::Invalid("0xA6 shorter than 0x126 bytes"))?
        .to_vec();
    let t = &mut w.item_table_ext;
    if index >= t.len() {
        t.resize(index + 1, vec![0; ITEM_ENTRY]);
    }
    t[index] = record;
    Ok(())
}
