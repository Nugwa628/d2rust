//! The item copy on the real item: the source written as a save-format
//! stream with children ([`InvDesk::save_view`], `items::bitstream`), the
//! first record read back (`items::bitstream::read`) and made an item
//! unit (`Economy::item_from_record`), then the follow-up steps of
//! [`InvDesk::copy_of`].
//! Callers: the vendors' buy and sell, the cube outputs, the hireling
//! take and the NPC socketing hand their source here (fillers as their
//! rule says); the server's cube copies here too; the interaction
//! wiring's NPC socketing and the economy cube's own `CubeRest` still
//! ask their rests.

use super::{InvDesk, InvError, InvRest};
use crate::items::bitstream::{self, read, BitWriter, StreamItem};
use crate::items::inventory::UnitKind;
use crate::items::moves::{
    add_iflags, clear_iflags, clear_uflags, deferred, iflag, mode, uflag, InventoryOps,
    MovePending, Owner,
};
use crate::items::{flag, stat};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

/// The copy's stream buffer: 1,024 bytes.
pub const COPY_BUFFER: usize = 0x400;
/// Item flag set on every copied source.
pub const COPIED: u32 = 0x800_0000;
/// Command flag 0x1, cleared on the copy.
const CMD_REMOVE: u32 = 0x1;
/// Item timer event 3: replenish.
const EVENT_REPLENISH: u8 = 3;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// The save-format view of `u`: the stream
    /// view at its own page with the unit record's init seed
    /// and the items of its own inventory as children, in
    /// list order. The trailer is the item's two realm data words.
    pub fn save_view(&self, u: UnitId) -> Option<StreamItem> {
        let page = self.state.items.get(&u)?.page;
        let mut v = self.stream_item(self.guid_of(u), 0, page)?;
        v.unit28 = self.econ.units.get(u)?.init_seed;
        v.children = self
            .state
            .inventories
            .get(&u)
            .map(|inv| {
                inv.items()
                    .iter()
                    .filter_map(|&c| self.save_view(c))
                    .collect()
            })
            .unwrap_or_default();
        Some(v)
    }

    /// The units of [`Self::save_view`] in its write order: `u`, then each
    /// child's own, depth first (the units that have a view).
    fn save_units(&self, u: UnitId) -> Vec<UnitId> {
        let has_view = self.state.items.get(&u).is_some_and(|d| {
            self.econ.units.get(u).is_some()
                && self.stream_item(self.guid_of(u), 0, d.page).is_some()
        });
        if !has_view {
            return Vec::new();
        }
        let mut out = vec![u];
        if let Some(inv) = self.state.inventories.get(&u) {
            for &c in inv.items() {
                out.extend(self.save_units(c));
            }
        }
        out
    }

    /// One child record of the copy: read like the first record but with no
    /// room (a failure gives none; the copy and the children read
    /// so far stay), mode 4, socketed into `parent`, flags 0x80000 /
    /// 0x2000, command flag 0x1 cleared. `parent` has an inventory. A
    /// failed record makes no item.
    pub(crate) fn insert_filler(&mut self, parent: UnitId, rec: &read::ReadItem) -> Option<UnitId> {
        if rec.failed {
            return None;
        }
        let child = match self.econ.item_from_record(rec, None) {
            Ok(c) => c,
            Err(e) => {
                self.state.errors.push(InvError::Economy(e));
                return None;
            }
        };
        self.link_filler(parent, child)?;
        Some(child)
    }

    /// The link part of [`Self::insert_filler`] for an item unit that
    /// already exists: `child` socketed into `parent`'s inventory (made
    /// when absent), as the inactive record restore puts a
    /// child back.
    pub fn link_filler(&mut self, parent: UnitId, child: UnitId) -> Option<()> {
        let cg = self.guid_of(parent);
        self.state.add_inventory(parent, UnitKind::Item, cg);
        self.sync_in();
        // Mode 4, then the child is linked into the copy's inventory.
        // The link has no target mode test: a stream written from a
        // socketed source holds only fillers that passed the socketing
        // checks, so the link cannot fail here.
        if let Some(d) = self.state.items.get_mut(&child) {
            d.mode = mode::CURSOR;
        }
        let fg = self.guid_of(child);
        let mut inv = self.state.inventories.remove(&parent)?;
        inv.link(self, child, None);
        // The copy's inventory cursor is cleared.
        inv.put_cursor(self, None);
        self.state.inventories.insert(parent, inv);
        clear_uflags(self, fg, uflag::TARGETABLE);
        // Filler properties and the owner link.
        self.filler_linked(fg, cg);
        if let Some(d) = self.state.items.get_mut(&child) {
            d.mode = mode::SOCKETED;
        }
        // A runeword match runs the runeword stats and the timers
        // without the recharge; no match returns here, before the tail.
        if self.activate_runeword_on(parent) {
            add_iflags(self, cg, iflag::CHANGED);
            clear_iflags(self, cg, iflag::NOEQUIP);
            deferred::owner_refresh(self, Owner::item(cg));
            self.update_list_add(Owner::item(cg), cg);
        }
        if let Some(d) = self.state.items.get_mut(&child) {
            d.flags = (d.flags | flag::INIT) & !flag::INSTORE;
            d.cmd_flags &= !CMD_REMOVE;
        }
        self.sync_out();
        Some(())
    }

    /// A copy of `src`, or none. `fillers`: read
    /// and socket the source's children into the copy.
    pub fn copy_of(&mut self, src: UnitId, fillers: bool) -> Option<UnitId> {
        // 1. The source's room (none off the ground). A ground
        // source would put the copy in that room, on the ground, and no caller
        // takes it out; every caller passes a held source, so a
        // ground one is a caller error (no copy).
        if self
            .econ
            .game
            .lists
            .unit(src)
            .and_then(|e| e.room())
            .is_some()
        {
            self.state.errors.push(InvError::GroundCopySource(src));
            return None;
        }
        let room = None;
        // 2. The save stream with children into 1,024 bytes; one that does
        // not fit has length 0 and step 3's read fails.
        let view = self.save_view(src)?;
        let mut w = BitWriter::new(COPY_BUFFER);
        let wbs = bitstream::write_save_into(&mut w, &view, &self.econ.tables.isc);
        let bytes = w.finish().unwrap_or_default();
        // The writer's changes (item level < 1 becomes 1, quality outside
        // 1–9 becomes 2) stay on the source and each child, in write order.
        for (u, wb) in self.save_units(src).into_iter().zip(wbs) {
            if let Some(it) = self.econ.items.get_mut(u) {
                it.ilvl = wb.ilvl;
                it.quality = wb.quality;
            }
        }
        // 3. The first record: class from the code, the unit allocated at
        // the record's mode and decoded; flags 0x80000 / 0x2000; timers.
        let mut r = read::BitReader::new(&bytes);
        let first = read::read_save_record(&mut r, self.econ.tables)
            .ok()
            .filter(|f| !f.failed)?;
        let copy = match self.econ.item_from_record(&first, room) {
            Ok(c) => c,
            Err(e) => {
                self.state.errors.push(InvError::Economy(e));
                return None;
            }
        };
        self.sync_in();
        if let Some(d) = self.state.items.get_mut(&copy) {
            d.x = first.item.x;
            d.y = first.item.y;
            d.body_loc = first.item.body_loc;
        }
        // 4. Flags again.
        if let Some(it) = self.econ.items.get_mut(copy) {
            it.flags = (it.flags | flag::INIT) & !flag::INSTORE;
        }
        // 5. The children.
        let n = if first.item.compact || first.item.alt {
            0
        } else {
            first.item.filled
        };
        if fillers && n != 0 {
            // Each child record in stream order (each starts at a byte):
            // read like step 3 with no room (a failure gives
            // none; the copy and the children read so far stay), mode 4,
            // socketed into the copy, flags 0x80000 / 0x2000, command
            // flag 0x1 cleared.
            let mut at = r.pos().div_ceil(8);
            let cg = self.guid_of(copy);
            self.state.add_inventory(copy, UnitKind::Item, cg);
            for _ in 0..n {
                let entry = read::read_save_entry(bytes.get(at..)?, self.econ.tables).ok()?;
                at += entry.len;
                self.insert_filler(copy, &entry.item)?;
            }
        }
        // 6. The source's flag.
        if let Some(it) = self.econ.items.get_mut(src) {
            it.flags |= COPIED;
        }
        // 7. Replenish: stat 252, then 253, a total ≠ 0 and no event 3 on
        // the copy (step 3 scheduled one when a total is set).
        let scheduled = self
            .econ
            .game
            .timers
            .unit_timers(copy)
            .into_iter()
            .any(|id| {
                self.econ
                    .game
                    .timers
                    .event(id)
                    .is_some_and(|(e, _, _)| e == EVENT_REPLENISH)
            });
        if !scheduled {
            let frame = self.econ.game.frame as u32;
            let v = [stat::REPLENISH_DURABILITY, stat::REPLENISH_QUANTITY]
                .into_iter()
                .map(|s| self.econ.stats.unit_total(copy, s, 0))
                .find(|&v| v != 0);
            if let Some(v) = v {
                let at = frame.wrapping_add(self.econ.game.ticks(2500 / v + 1) as u32);
                if let Err(e) = self.econ.game.schedule_event(
                    copy,
                    u32::from(EVENT_REPLENISH),
                    at as i32,
                    None,
                    0,
                    0,
                ) {
                    self.state.errors.push(InvError::Economy(e.into()));
                }
            }
        }
        self.sync_in();
        // 8. The per-item reset, then command flag 0x1 cleared.
        let g = self.guid_of(copy);
        deferred::item_reset(self, g);
        if let Some(d) = self.state.items.get_mut(&copy) {
            d.cmd_flags &= !CMD_REMOVE;
        }
        self.sync_out();
        Some(copy)
    }
}
