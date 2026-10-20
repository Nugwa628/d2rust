//! Item-move intents and deferred item messages:
//! the per-client item message dispatcher ([`deferred`]), every intent
//! handler ([`handlers`]), pickup from and drop to the ground and
//! gold ([`ground`]) and the S→C byte layouts ([`layouts`]).
//!
//! Status: implemented, not yet checked against a live game (the
//! reference cases R1–R6 are not measured yet).
//!
//! The inventory model, placement, belt, equip and shared checks
//! belong to `items::inventory`; this module reaches
//! them only through the seam [`InventoryOps`]. Unit record fields go
//! through [`MoveUnits`]; every call into a system that is not
//! wired yet (path and free spot, rooms, stat lists, item
//! creation and use, quests, hirelings, sounds, transport) goes through
//! [`MovePending`], whose defaults are the narrowest reading (nothing
//! happens, or the value that makes the caller do nothing).

pub mod deferred;
pub mod ground;
pub mod handlers;
pub mod layouts;
pub mod seams;

#[cfg(test)]
mod prop_tests;
#[cfg(test)]
mod tests;

pub use deferred::{
    announce_item, announce_item_as, category, dispatch, ground_update, ground_update_as,
    item_reset, item_unit_update, mark, merc_update_pass, owner_refresh, player_update,
    room_cleanup, update_list_pass, update_list_reset, Cond, ItemAction, Test, To, ITEM_ACTIONS,
};
pub use handlers::{handle, unequip_detached, HANDLED};
pub use seams::{InventoryOps, MovePending, MoveUnits, MoveWorld, Spot};

/// An item or unit GUID.
pub type Guid = u32;
/// "No unit" GUID (−1).
pub const NONE_GUID: Guid = u32::MAX;

/// A unit that owns an inventory (or any unit by type and GUID): players
/// (type 0), monsters / hirelings (type 1), items (type 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Owner {
    pub ty: u8,
    pub guid: Guid,
}

impl Owner {
    pub const PLAYER: u8 = 0;
    pub const MONSTER: u8 = 1;
    pub const ITEM: u8 = 4;
    /// Owner type "none" in messages.
    pub const NONE: u8 = 6;

    pub fn player(guid: Guid) -> Self {
        Self {
            ty: Self::PLAYER,
            guid,
        }
    }
    pub fn monster(guid: Guid) -> Self {
        Self {
            ty: Self::MONSTER,
            guid,
        }
    }
    pub fn item(guid: Guid) -> Self {
        Self {
            ty: Self::ITEM,
            guid,
        }
    }
    pub fn is_player(&self) -> bool {
        self.ty == Self::PLAYER
    }
}

/// Item modes (Outputs).
pub mod mode {
    pub const STORED: u8 = 0;
    pub const EQUIPPED: u8 = 1;
    pub const BELT: u8 = 2;
    pub const GROUND: u8 = 3;
    pub const CURSOR: u8 = 4;
    pub const SOCKETED: u8 = 6;
}

/// Pages.
pub mod page {
    pub const INVENTORY: u8 = 0;
    pub const TRADE1: u8 = 1;
    pub const TRADE2: u8 = 2;
    pub const CUBE: u8 = 3;
    pub const STASH: u8 = 4;
    pub const NONE: u8 = 0xFF;
}

/// Command flags (item data +0x14, `item-actions.tsv`).
pub mod cmd {
    pub const PUT_IN_PAGE: u32 = 0x2;
    pub const FROM_PAGE: u32 = 0x4;
    pub const EQUIP: u32 = 0x8;
    pub const UNEQUIP: u32 = 0x10;
    pub const SWAP_BODY: u32 = 0x20;
    pub const GROUND_TO_CURSOR: u32 = 0x40;
    pub const PICKED_TO_PAGE: u32 = 0x80;
    pub const ADD_QUANTITY: u32 = 0x100;
    pub const TO_BELT: u32 = 0x400;
    pub const FROM_BELT: u32 = 0x800;
    pub const SWAP_BELT: u32 = 0x1000;
    pub const PICKED_TO_BELT: u32 = 0x2000;
    pub const AUTO_UNEQUIP: u32 = 0x4000;
    pub const INDIRECT_SWAP: u32 = 0x10000;
    pub const SWAP_IN_PAGE: u32 = 0x40000;
    pub const TO_CURSOR: u32 = 0x100000;
}

/// Item flags (item data +0x18) used here.
pub mod iflag {
    pub const CHANGED: u32 = 0x1;
    pub const TARGETING: u32 = 0x4;
    pub const STACK_FULL: u32 = 0x8;
    pub const IDENTIFIED: u32 = 0x10;
    pub const COPIED: u32 = 0x20;
    pub const SWAP_IN: u32 = 0x40;
    pub const SWAP_OUT: u32 = 0x80;
    pub const BROKEN: u32 = 0x100;
    pub const REPAIRED: u32 = 0x200;
    pub const SOCKETED: u32 = 0x800;
    pub const NOEQUIP: u32 = 0x4000;
    pub const NO_UPDATE: u32 = 0x40000;
}

/// Unit flags (unit +0xC4) used here.
pub mod uflag {
    /// Changed this tick (set by every mode set).
    pub const CHANGED: u32 = 0x1;
    pub const TARGETABLE: u32 = 0x2;
    /// New, not yet announced to clients (cleared with 0x1 by the
    /// room clean-up).
    pub const NOT_ANNOUNCED: u32 = 0x10;
    pub const DROPPED: u32 = 0x1000;
    pub const GROUND_PLACED: u32 = 0x1002;
    pub const ON_GROUND: u32 = 0x200_0000;
}

/// Handler result codes.
pub mod res {
    pub const OK: u32 = 0;
    pub const RANGE: u32 = 1;
    pub const BAD: u32 = 2;
    pub const REFUSED: u32 = 3;
}

/// Item type numbers used here (Test vectors D3).
pub mod ty {
    pub const SHIE: u16 = 2;
    pub const TORS: u16 = 3;
    pub const GOLD: u16 = 4;
    pub const BOOK: u16 = 18;
    pub const BELT: u16 = 19;
    pub const SCRO: u16 = 22;
    pub const BOW: u16 = 27;
    pub const AXE: u16 = 28;
    pub const SWOR: u16 = 30;
    pub const SPEA: u16 = 33;
    pub const POLE: u16 = 34;
    pub const HELM: u16 = 37;
    pub const PHLM: u16 = 71;
    pub const HPOT: u16 = 76;
    pub const APOT: u16 = 80;
    pub const WPOT: u16 = 81;
}

/// Stat ids used here.
pub mod stat {
    pub const LEVEL: u16 = 12;
    pub const GOLD: u16 = 14;
    pub const QUANTITY: u16 = 70;
    pub const DURABILITY: u16 = 72;
}

/// Sound events the item moves play.
pub mod sound {
    pub const REFUSED_PICKUP: u32 = 0x13;
    pub const NO_ROOM: u32 = 0x17;
    /// The `toa ` use.
    pub const TOA_USED: u32 = 2;
}

/// The cube's item code.
pub const CUBE_CODE: [u8; 4] = *b"box ";
/// Gold limit per character level.
pub const GOLD_PER_LEVEL: i32 = 10_000;
/// Gold pile cap and piles per drop.
pub const PILE_CAP: i32 = 2_000_000_000;
pub const MAX_PILES: usize = 32;
/// Ground pickup range and walk range.
pub const PICK_RANGE: i32 = 50;
pub const WALK_RANGE: i32 = 5;
/// Use range of 0x20 / 0x26.
pub const USE_RANGE: i32 = 50;
/// Collision mask of the 0x16 walk test.
pub const PICK_COLLISION_MASK: u32 = 0x804;
/// Free-spot masks of a drop.
pub const DROP_MASK: u32 = 0x3E01;
pub const DROP_MASK2: u32 = 0x801;
/// Total size limit of 0x9C / 0x9D (≥ 0xFD is fatal).
pub const MAX_ITEM_MSG: usize = 0xFD;

/// Fatal asserts: the base game exits; D2RUST returns them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MoveFatal {
    /// An unlink found the item missing or in another inventory.
    #[error("unlink: item missing or not in the inventory")]
    Unlink,
    /// 0x25: the cursor item does not fit the belt item's slot (line 0x12D8).
    #[error("belt switch placement failed (line 0x12D8)")]
    BeltSwitch,
    /// 0x29: book and scroll spells differ (line 0x149C).
    #[error("scroll and book spells differ (line 0x149C)")]
    SpellMismatch,
    /// 0x63: the player has no inventory.
    #[error("player without an inventory")]
    NoInventory,
    /// A link whose failure is fatal failed (0x28 filler, pickup belt).
    #[error("item link failed")]
    Link,
    /// An item the move asserts on is missing (no item at L).
    #[error("item missing")]
    Missing,
    /// A book pickup met a negative quantity or max stack.
    #[error("negative book quantity")]
    NegativeQuantity,
    /// An item message reached 0xFD bytes.
    #[error("item message of {0} bytes")]
    MessageSize(usize),
    /// An item copy or creation whose failure is fatal returned none
    /// (take from the hireling).
    #[error("item copy or creation failed")]
    Create,
    /// Phase 1 step 5: the target body location is held (line
    /// 0x1775).
    #[error("corpse take-back: body location held (line 0x1775)")]
    SlotHeld,
}

/// Helper outcome: `ok` = result 1; `out` = the refusal flag (result
/// 0 with out ≠ 0 → handler 3, with out = 0 → handler 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Outcome {
    pub ok: bool,
    pub out: bool,
}

impl Outcome {
    pub const DONE: Outcome = Outcome {
        ok: true,
        out: false,
    };
    pub const NOTHING: Outcome = Outcome {
        ok: false,
        out: false,
    };
    pub const REFUSED: Outcome = Outcome {
        ok: false,
        out: true,
    };
    /// The handler result (text).
    pub fn result(self) -> u32 {
        if !self.ok && self.out {
            res::REFUSED
        } else {
            res::OK
        }
    }
}

// ---- small field helpers shared by the handlers ---------------------------

pub(crate) fn add_iflags<W: MoveUnits>(w: &mut W, item: Guid, f: u32) {
    let v = w.item_flags(item);
    w.set_item_flags(item, v | f);
}

pub(crate) fn clear_iflags<W: MoveUnits>(w: &mut W, item: Guid, f: u32) {
    let v = w.item_flags(item);
    w.set_item_flags(item, v & !f);
}

pub(crate) fn add_cmd<W: MoveUnits>(w: &mut W, item: Guid, f: u32) {
    let v = w.cmd_flags(item);
    w.set_cmd_flags(item, v | f);
}

pub(crate) fn add_uflags<W: MoveUnits>(w: &mut W, item: Guid, f: u32) {
    let u = Owner::item(item);
    let v = w.unit_flags(u);
    w.set_unit_flags(u, v | f);
}

pub(crate) fn clear_uflags<W: MoveUnits>(w: &mut W, item: Guid, f: u32) {
    let u = Owner::item(item);
    let v = w.unit_flags(u);
    w.set_unit_flags(u, v & !f);
}

/// Item flag 0x1 when the item is socketed with fillers.
pub(crate) fn changed_if_filled<W: MoveUnits>(w: &mut W, item: Guid) {
    if w.socket_filled(item) {
        add_iflags(w, item, iflag::CHANGED);
    }
}

/// Whether an item unit exists (lookup type 4).
pub(crate) fn exists<W: MoveUnits>(w: &W, item: Guid) -> bool {
    w.unit_exists(Owner::item(item))
}
