//! A mod game's NPC interaction (`docs/plans/mods/seams.md` MS8c): which
//! NPC classes trade, gamble, heal, identify, sell hirelings and
//! resurrect, and the mod's own menu actions.
//!
//! The mod answers first and D2RUST's code runs when it doesn't:
//! - **Roles.** [`NpcProvider::roles`] gives a class's roles; `None` keeps
//!   D2RUST's lists ([`NpcRoles::d2rust`]). A mod's own NPC classes (any
//!   monstats row with `interact`) get roles this way, and a mod may change
//!   a D2RUST NPC's roles.
//! - **Menu actions.** [`NpcProvider::menu_action`] runs a C→S 0x38 menu
//!   action before D2RUST's (trade, gamble, hire list, services). `None`
//!   runs D2RUST's action. The message checks (size, unit check, the
//!   `interact` flag, an interaction list) stay D2RUST's and run first.
//!
//! Action numbers are message fields and classes are monstats rows, so
//! there is no slot count to extend. Providers get the NPC world as
//! `dyn NpcHost` and must stay deterministic: no I/O, no clock, no floats
//! in game state, draws only from the game's seeds (the NPC-control seed
//! in [`NpcControl`], item seeds through [`NpcWorld::item_seed`]).

use std::fmt::Debug;

use super::{NpcControl, NpcError, NpcVendors, NpcWorld};
use super::{GAMBLERS, HEALERS, IDENTIFIERS, RESURRECTORS, SELLERS, TRADERS};
use crate::rules::Ruleset;
use crate::units::UnitId;

/// Every seam an NPC interaction needs, as one object-safe trait.
pub trait NpcHost: NpcWorld + NpcVendors {}
impl<T: NpcWorld + NpcVendors> NpcHost for T {}

/// What an NPC class does at its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NpcRoles {
    /// Menu action 1 opens its store.
    pub trades: bool,
    /// Menu action 2 opens its gamble list.
    pub gambles: bool,
    /// A chat open heals the player and the player's pets.
    pub heals: bool,
    /// C→S 0x34 identifies the player's items.
    pub identifies: bool,
    /// It sells mercenaries (hire list).
    pub hires: bool,
    /// It resurrects the player's mercenary.
    pub resurrects: bool,
}

impl NpcRoles {
    /// D2RUST's roles of `class`.
    pub fn d2rust(class: u16) -> Self {
        Self {
            trades: TRADERS.contains(&class),
            gambles: GAMBLERS.contains(&class),
            heals: HEALERS.contains(&class),
            identifies: IDENTIFIERS.contains(&class),
            hires: SELLERS.contains(&class),
            resurrects: RESURRECTORS.contains(&class),
        }
    }

    /// The roles of `class` under `rules`: the mod's, else D2RUST's.
    pub fn of(rules: &Ruleset, class: u16) -> Self {
        rules
            .npcs()
            .and_then(|p| p.roles(class))
            .unwrap_or_else(|| Self::d2rust(class))
    }
}

/// A mod game's NPC roles and menu actions. Every method defaults to "not
/// the mod's", so a provider only writes what its game adds.
pub trait NpcProvider: Send + Sync + Debug {
    /// The roles of NPC `class`; `None`: D2RUST's.
    fn roles(&self, class: u16) -> Option<NpcRoles> {
        let _ = class;
        None
    }

    /// Run menu `action` (with the message's `item` GUID) of NPC `npc` of
    /// `class` for `player`. `Some`: the message result (or a fatal
    /// error); `None`: D2RUST's action runs.
    #[allow(clippy::too_many_arguments)]
    fn menu_action(
        &self,
        ctl: &mut NpcControl,
        w: &mut dyn NpcHost,
        player: UnitId,
        npc: UnitId,
        class: u16,
        action: u32,
        item: u32,
    ) -> Option<Result<u32, NpcError>> {
        let _ = (ctl, w, player, npc, class, action, item);
        None
    }
}
