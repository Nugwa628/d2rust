//! The save's item index of an item: its 1-based position in the
//! owner's inventory item list (link order),
//! and back. The writer and the post-load resolution
//! both read the inventory state; the header fields that
//! carry the indices are `d2-formats::d2s`'s.

use super::InvState;
use crate::units::UnitId;

/// The highest index the writer accepts (above → fatal).
pub const MAX_ITEM_INDEX: u32 = 0x7FFE;

/// The writer's fatal assert (a position above 0x7FFE).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("item index {0} above 0x7FFE")]
pub struct IndexTooLarge(pub u32);

impl InvState {
    /// The save index of an item: GUID −1 or an owner without an
    /// inventory → 0; else the 1-based position of the item with that
    /// GUID in the owner's item list; not found → 0.
    pub fn save_item_index(&self, owner: UnitId, guid: u32) -> Result<u16, IndexTooLarge> {
        if guid == u32::MAX {
            return Ok(0);
        }
        let Some(inv) = self.inventories.get(&owner) else {
            return Ok(0);
        };
        let pos = inv
            .items()
            .iter()
            .position(|u| self.items.get(u).is_some_and(|d| d.guid == guid));
        match pos {
            None => Ok(0),
            Some(p) if p as u32 + 1 > MAX_ITEM_INDEX => Err(IndexTooLarge(p as u32 + 1)),
            Some(p) => Ok(p as u16 + 1),
        }
    }

    /// The decode (index 0 → −1) and the post-load
    /// resolution: the GUID of the item at that
    /// 1-based position of the owner's item list; past the end (or no
    /// inventory) → −1.
    pub fn item_at_save_index(&self, owner: UnitId, index: u16) -> u32 {
        if index == 0 {
            return u32::MAX;
        }
        self.inventories
            .get(&owner)
            .and_then(|inv| inv.items().get(usize::from(index) - 1))
            .and_then(|u| self.items.get(u))
            .map_or(u32::MAX, |d| d.guid)
    }
}
