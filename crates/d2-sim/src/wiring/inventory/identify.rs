//! Identify from a scroll or tome (C→S 0x27 `UseItemAction`, the
//! `use_item` seam).
//!
//! The effect (a scroll and a tome on unidentified magic items, the tome
//! on an identified one): a scroll of
//! identify (`isc`) or a tome of identify (`ibk`) used on a stored or equipped
//! item that is not identified sets the identified flag (item flag 0x10)
//! and marks the item changed (item flag 0x1), so the next update pass
//! sends it again (0x9D action 0x15) with its properties shown. The
//! scroll's consumption and the tome's charge are the 0x27 handler's.
//!
//! Recorded: the target's next update is 0x9D action 0x15 with flags
//! 0x10 | 0x1; the used scroll leaves with the 0x20 removal; the tome
//! loses one charge (0x3E) and sends 0x22 (skill 218), 0x7C and the
//! targeting reset 0x3F; an identified target gets only the reset and
//! 0x7C, nothing is used. The two codes stay D2RUST's own (entry 1 of the
//! item-use table).

use super::{InvDesk, InvRest};
use crate::items::inventory::mode;
use crate::items::moves::{deferred, iflag, Guid, MoveUnits, Owner};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;
use crate::world::npc::{InvEntry, Place};

/// Items codes of the identify scroll and tome (D2RUST's own, unverified).
/// Sound event queued on the user when an identify succeeds.
const SOUND_IDENTIFY: u16 = 6;

/// Item codes of the identify scroll and tome (not checked against the game).
pub const IDENTIFY_CODES: [[u8; 4]; 2] = [*b"isc ", *b"ibk "];

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// `use_item` for an identify scroll or tome `item` on `target`:
    /// true = used. Any other item, or a target that is not an item or is
    /// already identified, gives false (the 0x27 handler then does nothing
    /// more).
    pub fn use_identify(&mut self, player: Owner, target: Owner, item: Guid) -> bool {
        if !IDENTIFY_CODES.contains(&self.code(item)) || target.ty != Owner::ITEM {
            return false;
        }
        let t = target.guid;
        if self.item_unit(t).is_none() {
            return false;
        }
        if !self.mark_identified(player, t) {
            return false;
        }
        // The used scroll or tome is consumed, which ends
        // with sound event 6 on the user, for the user's client only.
        if let Some(u) = self.unit_of(player) {
            let _ = crate::units::sound::queue_sound(self.econ.game, u, SOUND_IDENTIFY, Some(u));
        }
        true
    }

    /// Item flags 0x10 and 0x1 on `item` and the owner update queued;
    /// false when it is already identified.
    fn mark_identified(&mut self, player: Owner, item: Guid) -> bool {
        let flags = self.item_flags(item);
        if flags & iflag::IDENTIFIED != 0 {
            return false;
        }
        self.set_item_flags(item, flags | iflag::IDENTIFIED | iflag::CHANGED);
        deferred::mark(self, player, item, 0);
        // Identified, a charm or worn item may now be usable: the
        // inventory pass links its list.
        crate::items::moves::MovePending::inventory_pass(self, player);
        true
    }

    /// The player's items in link order with their places and item flags
    /// (what Cain's identify walks).
    pub fn npc_entries(&self, player: UnitId) -> Vec<InvEntry> {
        self.state
            .items_of(player)
            .into_iter()
            .filter_map(|u| {
                let d = self.state.items.get(&u)?;
                let place = match d.mode {
                    mode::STORED => Place::Grid(d.page),
                    mode::EQUIPPED => Place::Equipped,
                    mode::BELT => Place::Belt,
                    _ => Place::Other,
                };
                Some(InvEntry {
                    item: u,
                    place,
                    flags: d.flags,
                })
            })
            .collect()
    }

    /// Cain's identify (C→S 0x34, `AppRest::identify`'s call) of the
    /// item unit `item` held by `player` (D2RUST's own rule, the same effect
    /// as a scroll): flags set and the owner update queued.
    pub fn identify_unit(&mut self, player: UnitId, item: UnitId) -> bool {
        let (Some(o), Some(_)) = (self.owner_of(player), self.state.items.get(&item)) else {
            return false;
        };
        let g = self.guid_of(item);
        self.mark_identified(o, g)
    }
}
