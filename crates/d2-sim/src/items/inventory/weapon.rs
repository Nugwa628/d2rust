//! The weapon in use (inventory +0x1C, read): the body
//! link and unlink are its only writers besides
//! the inventory unlink. Every place that
//! puts an item on a body slot or takes one off calls them, so the weapon
//! in use always matches what the body holds.

use super::{body, iflag, ty, InvTables, InvWorld, Inventory, NO_GUID};
use crate::units::UnitId;

/// Identified, not broken, no item flag 0x4000.
fn usable_flags(flags: u32) -> bool {
    flags & iflag::IDENTIFIED != 0 && flags & (iflag::BROKEN | iflag::F4000) == 0
}

/// The item's primary type is `tpot` (38).
fn is_tpot<W: InvWorld + ?Sized>(w: &W, t: &InvTables, item: UnitId) -> bool {
    w.item(item)
        .and_then(|d| t.item(d.record))
        .is_some_and(|r| r.type_ == ty::TPOT)
}

fn is_weap<W: InvWorld + ?Sized>(w: &W, t: &InvTables, item: UnitId) -> bool {
    w.item(item).is_some_and(|d| t.is_type(d.record, ty::WEAP))
}

/// Body link: only a `weap`
/// at body location 4 or 5. Not usable → +0x1C := −1 when it held the
/// item. Usable, W := the weapon in use: W none, not `weap` or a `tpot`
/// → +0x1C := the item; W the item itself → −1; else unchanged (the first
/// wielded weapon stays in use).
pub fn weapon_link<W: InvWorld + ?Sized>(inv: &mut Inventory, w: &W, t: &InvTables, item: UnitId) {
    let Some(d) = w.item(item) else {
        return;
    };
    let (guid, loc, flags) = (d.guid, d.body_loc, d.flags);
    if !matches!(loc, body::RIGHT_HAND | body::LEFT_HAND) || !is_weap(w, t, item) {
        return;
    }
    if !usable_flags(flags) {
        if inv.weapon_guid == guid {
            inv.weapon_guid = NO_GUID;
        }
        return;
    }
    let current = (inv.weapon_guid != NO_GUID)
        .then(|| w.item_by_guid(inv.weapon_guid))
        .flatten();
    match current {
        None => inv.weapon_guid = guid,
        Some(c) if !is_weap(w, t, c) || is_tpot(w, t, c) => inv.weapon_guid = guid,
        Some(c) if c == item => inv.weapon_guid = NO_GUID,
        Some(_) => {}
    }
}

/// Body unlink: only a
/// `weap`: +0x1C = its GUID → −1; then, when it is at location 4 the
/// item at 5 (and the reverse), when present and usable, becomes +0x1C
/// (its type is not tested).
pub fn weapon_unlink<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &W,
    t: &InvTables,
    item: UnitId,
) {
    let Some(d) = w.item(item) else {
        return;
    };
    let (guid, loc) = (d.guid, d.body_loc);
    if !is_weap(w, t, item) {
        return;
    }
    if inv.weapon_guid == guid {
        inv.weapon_guid = NO_GUID;
    }
    let other = match loc {
        body::RIGHT_HAND => body::LEFT_HAND,
        body::LEFT_HAND => body::RIGHT_HAND,
        _ => return,
    };
    if let Some(o) = inv.body_item(other).filter(|&o| o != item) {
        if let Some(od) = w.item(o).filter(|od| usable_flags(od.flags)) {
            inv.weapon_guid = od.guid;
        }
    }
}
