//! [`ItemStats`] on the real stat lists: a unit's stats are its extended
//! list; a [`ListKey`] list is a plain list with that state and flags
//! attached to the unit.
//!
//! Handles share the lists through a [`RefCell`] so that two units can
//! be written in one call (set bonuses write the owner while reading the
//! item). Host callbacks receive the lists directly
//! and never reach a handle, so a borrow never nests.

use std::cell::RefCell;

use crate::items::{ItemStats, ListKey};
use crate::stats::lists::flag;
use crate::stats::{ListId, StatHost, StatLists};
use crate::units::UnitId;

/// The stat lists and the host their writes notify.
pub struct StatCtx<'a> {
    /// The stat lists.
    pub lists: &'a mut StatLists,
    /// The host notified of stat writes.
    pub host: &'a mut dyn StatHost,
}

impl<'a> StatCtx<'a> {
    /// A context over the lists and the host.
    pub fn new(lists: &'a mut StatLists, host: &'a mut dyn StatHost) -> Self {
        Self { lists, host }
    }
}

/// One unit's stats through shared stat lists.
#[derive(Clone, Copy)]
pub struct UnitStats<'r, 'a> {
    ctx: &'r RefCell<StatCtx<'a>>,
    unit: UnitId,
}

impl<'r, 'a> UnitStats<'r, 'a> {
    /// Stats of `unit` through the shared context.
    pub fn new(ctx: &'r RefCell<StatCtx<'a>>, unit: UnitId) -> Self {
        Self { ctx, unit }
    }

    /// The unit these stats belong to.
    pub fn unit(&self) -> UnitId {
        self.unit
    }
}

/// The unit's list with `key`'s state and flags.
///
/// Searches the parked chain when flag 0x2000 is asked, else the active
/// chain, and takes the first list whose state equals `key.state` and whose
/// flags hold every asked bit. TODO: the exact lookup rule is not settled;
/// this is a reading of it.
pub fn find_list(lists: &StatLists, unit: UnitId, key: ListKey) -> Option<ListId> {
    let r = lists.unit_list(unit)?;
    let chain = if key.flags & flag::SET != 0 {
        lists.parked_chain(r)
    } else {
        lists.active_chain(r)
    };
    chain.into_iter().find(|&l| {
        lists.state(l) == u32::from(key.state) && lists.flags(l) & key.flags == key.flags
    })
}

/// [`find_list`], or a new plain list attached to the unit if missing.
/// `None` when the unit has no stat list.
///
/// The new list is allocated with the key's flags, no expiry and the unit
/// list's owner type and GUID; its state is `key.state`; it is attached with
/// reset 1 (every stat counts in the unit). TODO: these creation arguments
/// are a reading, not confirmed.
fn list_for(ctx: &mut StatCtx<'_>, unit: UnitId, key: ListKey) -> Option<ListId> {
    if let Some(l) = find_list(ctx.lists, unit, key) {
        return Some(l);
    }
    let r = ctx.lists.unit_list(unit)?;
    let (owner_type, owner_guid) = (ctx.lists.owner_type(r), ctx.lists.owner_guid(r));
    let l = ctx.lists.alloc(key.flags, 0, owner_type, owner_guid);
    ctx.lists.set_state(l, u32::from(key.state));
    ctx.lists.attach(ctx.host, unit, l, true);
    Some(l)
}

impl ItemStats for UnitStats<'_, '_> {
    fn has_stats(&self) -> bool {
        self.ctx.borrow().lists.unit_list(self.unit).is_some()
    }
    fn stat(&self, id: u16, layer: u16) -> i32 {
        self.ctx.borrow().lists.unit_total(self.unit, id, layer)
    }
    fn base(&self, id: u16, layer: u16) -> i32 {
        self.ctx.borrow().lists.unit_base(self.unit, id, layer)
    }
    /// Sets the unit's base stat.
    fn set_base(&mut self, id: u16, layer: u16, value: i32) {
        let mut c = self.ctx.borrow_mut();
        let StatCtx { lists, host } = &mut *c;
        lists.unit_set(*host, self.unit, id, value, layer);
    }
    fn has_list(&self, key: ListKey) -> bool {
        find_list(self.ctx.borrow().lists, self.unit, key).is_some()
    }
    fn list_set(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        let mut c = self.ctx.borrow_mut();
        if let Some(l) = list_for(&mut c, self.unit, key) {
            let StatCtx { lists, host } = &mut *c;
            lists.set(*host, l, id, value, layer, None);
        }
    }
    fn list_add(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        let mut c = self.ctx.borrow_mut();
        if let Some(l) = list_for(&mut c, self.unit, key) {
            let StatCtx { lists, host } = &mut *c;
            lists.add(*host, l, id, value, layer);
        }
    }
    fn list_get(&self, key: ListKey, id: u16, layer: u16) -> i32 {
        let c = self.ctx.borrow();
        find_list(c.lists, self.unit, key).map_or(0, |l| c.lists.base(l, id, layer))
    }
}
