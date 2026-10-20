//! The set-item state update: the partial lists of a
//! set item parked or unparked by the owner's set mask
//! and the owner's per-set list tagged with stat 71, and
//! the set branch of the stat refresh.

use crate::units::UnitId;

/// The states of the partial and owner lists (table).
pub const SET_STATES: [u32; 6] = [165, 166, 167, 168, 169, 170];
/// The dword before the table, read for mask 0 by
/// `add func` 2: a state no list has.
pub const STATE_BEFORE_TABLE: u32 = 3375;
/// Stat 71 (`item_setid`, the owner list's tag).
pub const STAT_SET_ID: u16 = 71;
/// Quality 5.
pub const QUALITY_SET: u8 = 5;

/// A set item's setitems row fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetItemRow {
    /// `add func` (+0x87).
    pub add_func: u8,
    /// The item's set slot (+0x2E).
    pub slot: i16,
    /// The set (+0x2C).
    pub set: i16,
}

/// What reads and writes.
pub trait SetWorld {
    /// The unit is an item (type 4).
    fn is_item(&self, u: UnitId) -> bool;
    /// The unit has an inventory (+0x60).
    fn has_inventory(&self, u: UnitId) -> bool;
    /// I is in O's inventory with node page 3 (body).
    fn in_body_of(&self, o: UnitId, i: UnitId) -> bool;
    /// The set mask with I included.
    fn set_mask(&self, o: UnitId, i: UnitId) -> u32;
    fn quality(&self, i: UnitId) -> u8;
    /// I's setitems row (file index below the count).
    fn setitem_row(&self, i: UnitId) -> Option<SetItemRow>;
    /// The sets count (table +0xC10).
    fn sets_count(&self) -> usize;
    /// Park / unpark I's list of `state`; a
    /// state without a list does nothing.
    fn park(&mut self, i: UnitId, state: u32, park: bool);
    /// O's list of `state`: `Some(stat 71, layer 0)`, or
    /// `None` without a list.
    fn owner_list_tag(&self, o: UnitId, state: u32) -> Option<i32>;
    /// Remove all of the list's stats, then stat 71 :=
    /// `set`.
    fn reset_owner_list(&mut self, o: UnitId, state: u32, set: i32);
    /// Detach the list from O and free it.
    fn free_owner_list(&mut self, o: UnitId, state: u32);
    /// A new list (`(O's pool, flags 0, expiry 0, I's type,
    /// I's GUID)) attached to O with reset 1, state `state`.
    fn new_owner_list(&mut self, o: UnitId, i: UnitId, state: u32);
    /// Stat 71 := `set` on O's list of `state`.
    fn tag_owner_list(&mut self, o: UnitId, state: u32, set: i32);
    /// The set bonuses`(O, I, state).
    fn set_bonuses(&mut self, o: UnitId, i: UnitId, state: u32);
}

/// Popcount table (masks ≥ 64 → 0).
fn popcount(mask: u32) -> i32 {
    if mask >= 64 {
        0
    } else {
        mask.count_ones() as i32
    }
}

///`(I, mask).
pub fn partial_lists<W: SetWorld + ?Sized>(w: &mut W, i: UnitId, mask: u32) {
    if w.quality(i) != QUALITY_SET {
        return;
    }
    let Some(row) = w.setitem_row(i) else {
        return;
    };
    match row.add_func {
        1 => {
            let n = i32::from(row.slot);
            for b in 0..6i32 {
                if b == n {
                    continue;
                }
                let k = if b > n { b - 1 } else { b };
                let unpark = mask & (1 << b) != 0;
                // k is 0..=5 (b ≤ 5, minus 1 above n).
                w.park(i, SET_STATES[k as usize], !unpark);
            }
        }
        2 => {
            let k = popcount(mask) - 1;
            for j in 0..k.max(0) {
                w.park(i, SET_STATES[j as usize], false);
            }
            if k < 5 {
                for j in k..5 {
                    let state = if j < 0 {
                        STATE_BEFORE_TABLE
                    } else {
                        SET_STATES[j as usize]
                    };
                    w.park(i, state, true);
                }
            }
        }
        _ => {}
    }
}

///`(O, I, r).
pub fn owner_list<W: SetWorld + ?Sized>(w: &mut W, o: UnitId, i: UnitId, r: i32) -> bool {
    if r > 1 || !w.is_item(i) || w.quality(i) != QUALITY_SET {
        return false;
    }
    if w.is_item(o) && (4..=9).contains(&w.quality(o)) {
        return false;
    }
    let Some(row) = w.setitem_row(i) else {
        return false;
    };
    let set = i32::from(row.set);
    if set < 0 || set as usize >= w.sets_count() {
        return false;
    }
    let mut free: i32 = -1;
    for (j, &s) in SET_STATES.iter().enumerate() {
        match w.owner_list_tag(o, s) {
            None => {
                if free < 0 {
                    free = j as i32;
                }
            }
            Some(tag) if tag != set => {}
            Some(_) => {
                match r {
                    0 => {
                        w.reset_owner_list(o, s, set);
                        w.set_bonuses(o, i, s);
                    }
                    1 => w.free_owner_list(o, s),
                    _ => {}
                }
                return true;
            }
        }
    }
    if r == 1 || free < 0 {
        return false;
    }
    let s = SET_STATES[free as usize];
    w.new_owner_list(o, i, s);
    if r == 0 {
        w.tag_owner_list(o, s, set);
        w.set_bonuses(o, i, s);
    }
    true
}

///`(O, I, r, p): `None` owner or item → 0.
pub fn set_item_update<W: SetWorld + ?Sized>(
    w: &mut W,
    o: Option<UnitId>,
    i: Option<UnitId>,
    r: i32,
    p: i32,
) -> bool {
    // Step 1.
    let Some(o) = o else {
        return false;
    };
    let Some(i) = i.filter(|&i| w.is_item(i)) else {
        return false;
    };
    if !w.has_inventory(o) {
        return false;
    }
    // Step 2.
    let mask = if w.in_body_of(o, i) {
        w.set_mask(o, i)
    } else {
        0
    };
    // Steps 3, 5.
    if p != 0 || r == 0 {
        partial_lists(w, i, mask);
    }
    // Steps 4, 6.
    owner_list(w, o, i, r);
    true
}

/// The set branch of the stat refresh: a
/// quality-5 item that is neither a gem nor a rune filler gets
/// the set refresh `(owner, item, 0, 0)`. Returns whether the branch ran.
pub fn stat_refresh_set<W: SetWorld + ?Sized>(
    w: &mut W,
    owner: UnitId,
    item: UnitId,
    gem_or_rune: bool,
) -> bool {
    if gem_or_rune || w.quality(item) != QUALITY_SET {
        return false;
    }
    set_item_update(w, Some(owner), Some(item), 0, 0);
    true
}

#[cfg(test)]
mod tests;
