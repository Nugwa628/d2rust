//! The belt: belt type and boxes, slots, beltable,
//! similar items, the free slot for an item, the auto-belt
//! gate, placing in a slot and compaction.

use super::{
    body, grid::place_in_grid, grid_id, iflag, node, InteractionTarget, InvTables, InvWorld,
    Inventory, BELT_GRID,
};
use crate::units::UnitId;

/// Belt record used without a belt: `belts` record 2, 4 boxes.
pub const DEFAULT_BELT: usize = 2;
/// Slot count of the belt grid.
pub const BELT_SLOTS: u8 = 16;

/// Similar-potion groups.
pub const POTION_GROUPS: [&[[u8; 4]]; 3] = [
    &[*b"hp1 ", *b"hp2 ", *b"hp3 ", *b"hp4 ", *b"hp5 "],
    &[*b"mp1 ", *b"mp2 ", *b"mp3 ", *b"mp4 ", *b"mp5 "],
    &[*b"rvl ", *b"rvs "],
];

/// Belt type: items `belt` of the item at body
/// location 8, with no item-type test; no item → record 2.
pub fn belt_type<W: InvWorld + ?Sized>(inv: &Inventory, w: &W, t: &InvTables) -> usize {
    inv.body_item(body::BELT)
        .and_then(|b| w.item(b))
        .and_then(|d| t.item(d.record))
        .map_or(DEFAULT_BELT, |r| usize::from(r.belt))
}

/// `numboxes` of the belt in use; none when the record is missing.
pub fn belt_numboxes<W: InvWorld + ?Sized>(inv: &Inventory, w: &W, t: &InvTables) -> Option<u8> {
    t.numboxes(belt_type(inv, w, t))
}

/// Beltable: itemtypes `beltable` of the item's type.
pub fn beltable(t: &InvTables, record: usize) -> bool {
    t.itype_of(record).is_some_and(|r| r.beltable != 0)
}

/// Similar: same item class, or both codes in one
/// potion group.
pub fn similar(t: &InvTables, a: usize, b: usize) -> bool {
    if a == b {
        return true;
    }
    let (Some(ra), Some(rb)) = (t.item(a), t.item(b)) else {
        return false;
    };
    POTION_GROUPS
        .iter()
        .any(|g| g.contains(&ra.code) && g.contains(&rb.code))
}

fn is_1x1(t: &InvTables, record: usize) -> bool {
    t.size(record) == Some((1, 1))
}

/// Free slot for an item.
pub fn free_belt_slot<W: InvWorld + ?Sized>(
    inv: &Inventory,
    w: &W,
    t: &InvTables,
    item: UnitId,
) -> Option<u8> {
    let rec = w.item(item)?.record;
    if !beltable(t, rec) || !is_1x1(t, rec) {
        return None;
    }
    let n = belt_numboxes(inv, w, t)?;
    for c in 0..4u8 {
        let held = inv.belt_item(c).and_then(|i| w.item(i)).map(|d| d.record);
        if c < n && held.is_some_and(|h| similar(t, h, rec)) {
            let slot = (c..n).step_by(4).find(|&s| inv.belt_item(s).is_none());
            // A column with no empty slot falls through to the next one.
            if slot.is_some() {
                return slot;
            }
        }
    }
    if t.item(rec).is_some_and(|r| r.autobelt != 0) {
        return (0..4u8).find(|&s| inv.belt_item(s).is_none());
    }
    None
}

/// Auto-belt gate: true for every item. Its call to
/// the bottom-row test can only reject on −1, which that test
/// never returns (a base-game bug, kept on purpose).
pub fn auto_belt_gate<W: InvWorld + ?Sized>(
    _inv: &Inventory,
    _w: &W,
    _t: &InvTables,
    _item: UnitId,
) -> bool {
    true
}

/// Place in a slot: beltable, 1 × 1, slot ≤ 15
/// (unsigned) → [`super::grid::place_in_grid`] on grid 1 at (slot, 0). No `numboxes` check here nor
/// in 0x23 / 0x25: a crafted message fills any empty slot 0–15 whatever
/// the belt (edge case 6).
pub fn place_in_belt_slot<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: UnitId,
    slot: u8,
) -> bool {
    let Some(rec) = w.item(item).map(|d| d.record) else {
        return false;
    };
    if !beltable(t, rec) || !is_1x1(t, rec) || slot >= BELT_SLOTS {
        return false;
    }
    place_in_grid(
        inv,
        w,
        item,
        grid_id::BELT,
        i32::from(slot),
        0,
        (1, 1),
        BELT_GRID,
    )
}

/// Compaction after slot `s` is emptied: column
/// c = min(s & 3, 3); walking rows 0..3, each item found moves down to the
/// lowest free row of the column below it, gets item flags 0x400
/// and 0x1 (item flags +0x18, not command flags), loses 0x4000,
/// refreshes its owner and joins the update list.
/// Returns the moves (from slot, to slot) in order.
pub fn compact_belt<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    s: u8,
) -> Vec<(u8, u8)> {
    let c = (s & 3).min(3);
    let mut moves = Vec::new();
    for row in 0..4u8 {
        let from = c + 4 * row;
        let Some(item) = inv.belt_item(from) else {
            continue;
        };
        // Only items whose row changes are flagged and listed.
        let Some(to_row) = (0..row).find(|&r| inv.belt_item(c + 4 * r).is_none()) else {
            continue;
        };
        let to = c + 4 * to_row;
        if !place_in_belt_slot(inv, w, t, item, to) {
            continue;
        }
        moves.push((from, to));
        let mut guid = None;
        if let Some(d) = w.item_mut(item) {
            d.flags |= iflag::F400 | iflag::CHANGED;
            d.flags &= !iflag::F4000;
            guid = Some(d.guid);
        }
        w.owner_refresh(inv.owner);
        if let Some(g) = guid {
            inv.push_update(g);
        }
    }
    moves
}

/// `numboxes` of the belt type of item `belt` (on the item),
/// or of record 2 when there is none.
pub fn belt_boxes_of<W: InvWorld + ?Sized>(w: &W, t: &InvTables, belt: Option<UnitId>) -> u8 {
    let ty = belt
        .and_then(|b| w.item(b))
        .and_then(|d| t.item(d.record))
        .map_or(DEFAULT_BELT, |r| usize::from(r.belt));
    t.numboxes(ty).unwrap_or(0)
}

/// Belt removal gate: refused only when the
/// owner has an interaction with a unit of type 0 (another player: trade)
/// and some item of the inventory is in the belt (node kind 2).
pub fn belt_removal_allowed<W: InvWorld + ?Sized>(inv: &Inventory, w: &W) -> bool {
    let trading = matches!(
        w.interaction(inv.owner),
        InteractionTarget::Unit { ty: 0, .. }
    );
    !(trading
        && inv
            .items()
            .iter()
            .any(|&i| w.item(i).is_some_and(|d| d.node_kind == node::BELT)))
}
