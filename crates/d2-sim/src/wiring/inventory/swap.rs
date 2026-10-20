//! The weapon switch (C→S 0x60): the items on the
//! weapon-swap body locations 11 / 12 trade places with the ones in the
//! hands (4 / 5) and the mouse-skill pairs of the two sets trade with
//! them.
//!
//! The desk first swaps the sets ([`InvDesk::swap_weapon_sets`]) and then
//! decides which saved skill each side selects
//! ([`InvDesk::weapon_switch_sides`]). The host sends S→C 0x97 after the
//! pass's messages, then the selects' direct 0x23 and the queued 0x23
//! records (they need the server skill lists, which the host returns to
//! the action hooks after each call).

use super::{InvDesk, InvRest};
use crate::items::inventory::body;
use crate::items::inventory::bookkeeping::{self as bk};
use crate::items::moves::{
    add_cmd, add_iflags, clear_iflags, clear_uflags, iflag, mode, owner_refresh, page, uflag,
    InventoryOps, MovePending, MoveUnits, Owner,
};
use crate::units::lifecycle::LifecycleHooks;

/// Command flag 0x200000 of a moved item: it makes the dispatcher send
/// S→C 0x9D action 0x17 to every client.
const SWITCHED: u32 = crate::items::inventory::cmd::WEAPON_SWITCH;

/// The move table: from, to, link kind.
const TABLE: [(u8, u8, u8); 4] = [
    (body::RIGHT_HAND, body::SWAP_RIGHT, 4),
    (body::LEFT_HAND, body::SWAP_LEFT, 4),
    (body::SWAP_RIGHT, body::RIGHT_HAND, 3),
    (body::SWAP_LEFT, body::LEFT_HAND, 3),
];

/// `use_state` codes that keep the saved skill unselected:
/// 2 no quantity / item type, 7 no level.
const NOT_SELECTED: [u8; 2] = [bk::USE_NO_QUANTITY, 7];

/// What the set swap left: `ok` false is out 1 with result 0 (the handler
/// answers 3); `saved` are the left and right mouse-skill pairs taken in
/// step 2 of the swap, as (skill id, owner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchedSets {
    pub ok: bool,
    pub saved: [(i32, i32); 2],
}

/// The outcome for one hand side after the swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchSide {
    /// The saved pair is selected (its 0x23 is sent at once) and is the
    /// queued record's skill.
    Select((i32, i32)),
    /// Nothing is selected; the record carries the side's current skill
    /// (none → (0, −1)).
    Keep((i32, i32)),
}

impl SwitchSide {
    /// The queued record's (skill id, owner).
    pub fn record(self) -> (i32, i32) {
        match self {
            Self::Select(s) | Self::Keep(s) => s,
        }
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// Swaps the weapon sets of `owner` (steps 1–7 below). `None`: the
    /// owner has no inventory.
    pub fn swap_weapon_sets(&mut self, owner: Owner) -> Option<SwitchedSets> {
        if !self.has_inventory(owner) {
            return None;
        }
        // 1. Targeting reset.
        self.targeting_reset(owner);
        // 2. The mouse-skill sets trade; the throw-restore slots reset
        //    (+0x90 / +0x94 have no D2RUST state).
        let saved = self.rest.trade_swap_skills(owner);
        for left in [true, false] {
            self.rest.set_saved_mouse_skill(owner, left, (0, -1));
        }
        let mut out = SwitchedSets { ok: true, saved };
        // 3. Every item in the table must be in mode 1.
        let held: Vec<_> = TABLE
            .iter()
            .map(|&(from, _, _)| self.body_item(owner, from))
            .collect();
        if held
            .iter()
            .flatten()
            .any(|&i| self.mode(i) != mode::EQUIPPED)
        {
            out.ok = false;
            return Some(out);
        }
        // 4. Deactivation of the hand items, right then left
        //    through `EquipWorld::deactivate`.
        if let Some(u) = self.unit_of(owner) {
            for i in held[..2].iter().flatten() {
                if let Some(iu) = self.item_unit(*i) {
                    bk::EquipWorld::deactivate(self, iu, u);
                }
            }
        }
        // 5. All four off the body before any is placed.
        for (k, (&(from, _, _), item)) in TABLE.iter().zip(&held).enumerate() {
            let Some(i) = *item else { continue };
            if k < 2 {
                self.body_leave_effects(owner, i);
            }
            if !self.unlink(owner, i) {
                out.ok = false;
                return Some(out);
            }
            self.clear_body_slot(owner, from);
            self.set_page(i, page::NONE);
            add_cmd(self, i, SWITCHED);
            add_iflags(self, i, iflag::CHANGED);
            clear_iflags(self, i, iflag::NOEQUIP);
            self.update_list_add(owner, i);
        }
        // 6. Placement in table order.
        for (k, (&(_, to, kind), item)) in TABLE.iter().zip(&held).enumerate() {
            let Some(i) = *item else { continue };
            if !self.place_body(owner, i, to) || !self.link_check(owner, i, kind) {
                out.ok = false;
                return Some(out);
            }
            self.set_body_loc(i, to);
            if to == body::RIGHT_HAND || to == body::LEFT_HAND {
                self.stat_link(owner, i);
                self.stat_refresh(owner);
                add_iflags(self, i, iflag::SWAP_IN);
            } else {
                add_iflags(self, i, iflag::SWAP_OUT);
            }
            clear_uflags(self, i, uflag::TARGETABLE);
            self.set_mode(i, mode::EQUIPPED);
            add_iflags(self, i, iflag::CHANGED);
            add_cmd(self, i, SWITCHED);
            if k < 2 {
                self.weapon_bookkeeping(owner);
            }
        }
        // 7. Inventory pass, send 0, then the owner refresh.
        self.inventory_pass(owner);
        owner_refresh(self, owner);
        Some(out)
    }

    /// Decides each side's skill on the lists after the pass (item-granted
    /// skills of the new hands exist): left, then right. A saved pair the
    /// player has whose `use_state` is neither 2 nor 7 is selected;
    /// otherwise the side keeps its skill.
    pub fn weapon_switch_sides(&mut self, owner: Owner, saved: [(i32, i32); 2]) -> [SwitchSide; 2] {
        let unit = self.unit_of(owner);
        let side = |d: &mut Self, left: bool, s: (i32, i32)| {
            let usable = d.rest.has_skill_owned(owner, s)
                && unit
                    .is_some_and(|u| !NOT_SELECTED.contains(&bk::EquipWorld::use_state(d, u, s)));
            if usable {
                SwitchSide::Select(s)
            } else {
                SwitchSide::Keep(d.rest.mouse_skill(owner, left).unwrap_or((0, -1)))
            }
        };
        let l = side(self, true, saved[0]);
        let r = side(self, false, saved[1]);
        [l, r]
    }
}
