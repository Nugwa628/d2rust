//! A save item entry made an item unit of an owner's inventory:
//! [`InvDesk::load_entry`]. The record is decoded into a unit
//! (`Economy::item_from_record`), its socketed children are
//! inserted ([`InvDesk::insert_filler`]), then the item is placed
//! by the mode and position it was saved with.
//!
//! Placement: the unit is set to
//! cursor mode and goes through the same inventory calls the start items
//! use (`equip_from_cursor` at its body location; the belt at slot x; the
//! page its record names at the exact (x, y), no free-position search); an
//! item that cannot be placed there, or saved in mode 3, 5, 6 (top level)
//! or above 6, is freed.

use super::{InvDesk, InvError, InvRest};
use crate::items::bitstream::hflag;
use crate::items::bitstream::read::ReadEntry;
use crate::items::inventory::page;
use crate::items::inventory::UnitKind;
use crate::items::moves::{mode, InventoryOps, MoveUnits, Owner};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

/// Why a loaded item was not placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadFault {
    /// The unit could not be made from the record.
    NotCreated,
    /// The record was read but failed (an unreadable item): no item, the
    /// entry is skipped; its children are
    /// read and freed.
    RecordFailed,
    /// The owner has no inventory in the model.
    NoInventory,
    /// The saved place did not take it; the unit is freed.
    NoRoom,
    /// Rule 5: the item carries the runeword flag but matches no runeword
    /// row any more; after its placement it is removed by its mode
    /// (stored and cursor items freed, an equipped one detached).
    StaleRuneword,
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// Makes the item of `entry` (and its socketed children) and places it
    /// in `owner`'s inventory. Placements queue the owner's item messages
    /// (take them with the host's sent queue).
    pub fn load_entry(&mut self, owner: UnitId, entry: &ReadEntry) -> Result<UnitId, LoadFault> {
        self.load_entry_in(owner, entry, false)
    }

    /// [`Self::load_entry`] for a corpse's list (the
    /// corpse placement):
    /// a worn item is put straight at its saved body location (mode 1,
    /// page none; no equip from the cursor, no requirement test); a
    /// stored one at its saved cell, sending nothing; no free-position
    /// fallback (a failure frees the unit).
    pub fn load_corpse_entry(
        &mut self,
        corpse: UnitId,
        entry: &ReadEntry,
    ) -> Result<UnitId, LoadFault> {
        self.load_entry_in(corpse, entry, true)
    }

    fn load_entry_in(
        &mut self,
        owner: UnitId,
        entry: &ReadEntry,
        corpse: bool,
    ) -> Result<UnitId, LoadFault> {
        if !self.state.inventories.contains_key(&owner) {
            return Err(LoadFault::NoInventory);
        }
        let unit = self.make_entry(entry)?;
        self.place_made(owner, unit, entry, corpse)
    }

    /// The first half of the load: the unit made from the record
    /// with its socketed children, not placed anywhere.
    /// The Iron Golem item stops here at the `kf` read (the unit
    /// rule 1: the unit exists from the load; the join's re-cast places it).
    pub fn make_entry(&mut self, entry: &ReadEntry) -> Result<UnitId, LoadFault> {
        let rec = &entry.item;
        if rec.failed {
            return Err(LoadFault::RecordFailed);
        }
        let unit = match self.econ.item_from_record(rec, None) {
            Ok(u) => u,
            Err(e) => {
                self.state.errors.push(InvError::Economy(e));
                return Err(LoadFault::NotCreated);
            }
        };
        self.sync_in();
        if !entry.children.is_empty() {
            let g = self.guid_of(unit);
            self.state.add_inventory(unit, UnitKind::Item, g);
            for child in &entry.children {
                let _ = self.insert_filler(unit, &child.item);
            }
        }
        Ok(unit)
    }

    /// The Iron Golem cast's item step: the
    /// made `unit` is held (mode 4) and equipped on `owner` at its first
    /// allowed body location (`BodyLoc1`; equip without cursor with
    /// skip 1). False when it did not take.
    pub fn equip_first_loc(&mut self, owner: UnitId, unit: UnitId) -> bool {
        if !self.state.inventories.contains_key(&owner) {
            return false;
        }
        let loc = self
            .state
            .items
            .get(&unit)
            .and_then(|it| self.tables.itype_of(it.record))
            .map_or(0, |t| t.bodyloc1);
        if let Some(d) = self.state.items.get_mut(&unit) {
            d.mode = mode::CURSOR;
        }
        self.sync_out();
        let (o, g) = (
            self.owner_of(owner).unwrap_or(Owner::player(0)),
            self.guid_of(unit),
        );
        self.equip_from_cursor(o, g, loc, true).0
    }

    /// The second half: the made `unit` placed in `owner`'s inventory as
    /// saved.
    pub fn place_made(
        &mut self,
        owner: UnitId,
        unit: UnitId,
        entry: &ReadEntry,
        corpse: bool,
    ) -> Result<UnitId, LoadFault> {
        if !self.state.inventories.contains_key(&owner) {
            return Err(LoadFault::NoInventory);
        }
        let it = &entry.item.item;
        // Rule 5: a runeword flag the
        // sockets no longer back. The item is placed as saved first; then
        // the refresh removes it by its mode (below).
        let stale = self
            .econ
            .items
            .get(unit)
            .is_some_and(|i| i.flags & hflag::RUNEWORD != 0)
            && !self.runeword_matches(unit);
        let saved_mode = it.mode as u8;
        let page = if it.page == 0xFF { 0 } else { it.page };
        if let Some(d) = self.state.items.get_mut(&unit) {
            d.x = it.x;
            d.y = it.y;
            d.body_loc = it.body_loc;
            // A cursor item keeps the saved page as it is (0xFF stays 0xFF:
            // the next save writes it back).
            d.page = if saved_mode == mode::CURSOR {
                it.page
            } else {
                page
            };
            d.mode = mode::CURSOR;
        }
        self.sync_out();
        let (o, g) = (
            self.owner_of(owner).unwrap_or(Owner::player(0)),
            self.guid_of(unit),
        );
        if corpse {
            let placed = match saved_mode {
                mode::EQUIPPED if self.place_body(o, g, it.body_loc) => {
                    self.set_body_loc(g, it.body_loc);
                    self.set_mode(g, mode::EQUIPPED);
                    self.set_page(g, page::NONE);
                    true
                }
                mode::EQUIPPED => false,
                _ => self.place(owner, unit, (it.x, it.y), false, false),
            };
            if placed {
                return Ok(unit);
            }
            self.free(unit);
            return Err(LoadFault::NoRoom);
        }
        let at_saved_place = match saved_mode {
            mode::EQUIPPED => self.equip_from_cursor(o, g, it.body_loc, true).0,
            // The belt placement `(game, owner, GUID, belt slot, 0, &out)`:
            // the belt body at slot x (link, mode 2, page 0xFF,
            // command flag 0x400, the update
            // list), as the start items' belt placement.
            mode::BELT => {
                crate::items::moves::handlers::to_belt(self, o, g, it.x as u32)
                    == crate::items::moves::Outcome::DONE
            }
            // Mode 4: the owner's cursor; the
            // saved cell is not used.
            mode::CURSOR => {
                crate::items::moves::handlers::load_to_cursor(self, o, g);
                true
            }
            mode::STORED => self.place(owner, unit, (it.x, it.y), false, true),
            // Mode 3 (ground), 5, a top-level 6 (socketed, no parent) and
            // anything above 6 fail with 0xC: the unit is
            // freed below.
            _ => false,
        };
        if at_saved_place && stale {
            // The held-item removal by mode ([`InvDesk::delete_held_item`]): stored → 0x9D action 5
            // (flag 0x20, the stored page), unlinked and freed; equipped →
            // the detached take-off `(.., 1)` (not freed; the
            // next unit update sends its message); cursor → the cursor consume.
            self.delete_held_item(owner, unit);
            return Err(LoadFault::StaleRuneword);
        }
        if at_saved_place {
            return Ok(unit);
        }
        self.free(unit);
        Err(LoadFault::NoRoom)
    }
}
