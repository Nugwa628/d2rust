//! Item, inventory and cube intents of the server.
//!
//! Only ids whose behaviour D2RUST defines get a handler here; every
//! other item id stays the [`super::super::SimGame`] stub. The table
//! [`ITEM_IDS`] lists each item-related C→S id and the part of the game
//! that owns it.
//!
//! The handlers run the `d2-sim` modules on their real providers: the
//! cube (`d2_sim::world::cube`) through the economy wiring
//! (`d2_sim::wiring::economy::EconomyCube`: items, stats, unit records,
//! item creation) over the game's one unit world: the world host builds
//! the economy from the action wiring's unit records, stat lists and
//! hooks ([`super::world::WorldHost::cube`]). The player's inventory is
//! the game's one inventory model ([`moves::InvParts`],
//! `d2_sim::wiring::inventory`: the item lists, placement, removal, the
//! placement checks and the targeting reset), the same one the item
//! moves and the vendors ([`InvVendors`]) use. What the cube asks for
//! that no `d2-sim` module provides is either staged in the host's
//! [`CubeParts`] ([`Staged`]: the local date) or goes to [`ItemPending`],
//! whose provider is the owner still to be written (the inventory pass,
//! the item routines nothing else implements, quest hooks).

mod cube_world;
pub mod moves;
#[cfg(test)]
mod tests;
mod vendor_inv;

use std::collections::BTreeMap;

use d2_sim::game::Game;
use d2_sim::stats::StatHost;
use d2_sim::tick::EventDispatch;
use d2_sim::units::lifecycle::LifecycleHooks;
use d2_sim::units::RoomId;
use d2_sim::units::UnitId;
use d2_sim::wiring::action::{ActionHooks, Pending};
use d2_sim::wiring::economy::{Economy, EconomyCube, EconomyError};
use d2_sim::world::cube::CubeData;
use d2_sim::world::stash::{self, ButtonRoute};

use super::super::SimGame;
use super::world::WorldHost;
use crate::buffers::QueueError;
use crate::seams::{ClientId, MessageSink, ResultCode};

pub use cube_world::CreationInfo;
pub use moves::InvParts;
pub use vendor_inv::InvVendors;

/// C→S 0x2A ItemToCube: put the cursor item into the cube.
pub const ITEM_TO_CUBE: u8 = 0x2A;
/// C→S 0x4F ClickButton: the stash and cube buttons.
pub const CLICK_BUTTON: u8 = 0x4F;

/// Every item-related C→S id and the design note that owns its behaviour;
/// `None`: nothing defines it yet, the id stays a stub.
/// The inventory-move ids are handled in [`moves`] ([`moves::MOVE_IDS`]).
/// Vendor ids (0x32–0x38) belong to the world handlers, not here.
pub const ITEM_IDS: &[(u8, Option<&str>)] = &[
    (0x16, Some("specs/items/inventory-moves.md §7.1")), // PickItem
    (0x17, Some("specs/items/inventory-moves.md §7.2")), // DropItem
    (0x18, Some("specs/items/inventory-moves.md §7.3")), // InsertItemInBuffer
    (0x19, Some("specs/items/inventory-moves.md §7.4")), // RemoveItemFromBuffer
    (0x1A, Some("specs/items/inventory-moves.md §7.5")), // EquipItem
    (0x1B, Some("specs/items/inventory-moves.md §7.6")), // Swap2HandedItem
    (0x1C, Some("specs/items/inventory-moves.md §7.7")), // RemoveBodyItem
    (0x1D, Some("specs/items/inventory-moves.md §7.8")), // SwapCursorWithBody
    (0x1E, Some("specs/items/inventory-moves.md §7.9")), // Swap1HWith2H
    (0x1F, Some("specs/items/inventory-moves.md §7.10")), // SwapCursorBufferItem
    (0x20, Some("specs/items/inventory-moves.md §7.11")), // UseGridItem
    (0x21, Some("specs/items/inventory-moves.md §7.12")), // StackItems
    (0x22, Some("specs/items/inventory-moves.md §7.13")), // UnstackItems
    (0x23, Some("specs/items/inventory-moves.md §7.14")), // ItemToBelt
    (0x24, Some("specs/items/inventory-moves.md §7.15")), // ItemFromBelt
    (0x25, Some("specs/items/inventory-moves.md §7.16")), // SwitchBeltItem
    (0x26, Some("specs/items/inventory-moves.md §7.17")), // UseBeltItem
    (0x27, Some("specs/items/inventory-moves.md §7.18")), // UseItemAction
    (0x28, Some("specs/items/inventory-moves.md §7.19")), // SocketItem
    (0x29, Some("specs/items/inventory-moves.md §7.20")), // ScrollToBook
    (ITEM_TO_CUBE, Some("specs/world/cube.md §2")),
    // The cube routes 0x4C to the item-use routines, which are not written.
    (0x4C, None), // Transmogrify
    (CLICK_BUTTON, Some("specs/world/vendors-2.md §10.1")),
    (0x50, Some("specs/items/inventory-moves.md §7.22")), // DropGold
    (0x61, Some("specs/items/inventory-moves.md §7.23")), // MercItem
    (0x63, Some("specs/items/inventory-moves.md §7.24")), // ItemToBeltShift
];

/// Errors the handler result code cannot carry, in order.
#[derive(Debug, PartialEq, Eq)]
pub enum ItemError {
    /// A provider's error inside a seam call (`EconomyCube::errors`).
    Economy(EconomyError),
    /// Queueing a message for the acting client failed.
    Sink(QueueError),
    /// A module asked to send to a player other than the acting one.
    OtherPlayer(UnitId),
    /// A fatal inconsistency inside an inventory routine the cube calls
    /// (`items::moves::MoveFatal`).
    Move(d2_sim::items::moves::MoveFatal),
}

/// State no `d2-sim` module holds yet, staged by the caller until it
/// moves into `d2-sim`; and what the handlers record for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// `GetLocalTime` (day of month, day of week + 1): host input.
    pub local_date: (u8, u8),
    /// The cube's sound events, in order: a record of the calls. The event itself is queued on the
    /// game's sound slots (`d2_sim::units::sound`) and leaves as S→C 0x2C
    /// in the player's unit update.
    pub sounds: Vec<(UnitId, u8)>,
}

/// The cube's calls whose owner is not written yet; the provider is that
/// owner's code. Messages they queue go to `out` (the acting
/// client's, in call order). Placement, removal and the socketed items
/// are the inventory model's ([`moves::InvParts`]).
pub trait ItemPending {
    /// The inventory pass (inventory / UI owner).
    fn inventory_pass(&mut self, player: UnitId, out: &mut Vec<Vec<u8>>);
    /// Duplicate an item for a host without inventory parts (with them
    /// the cube copies on the inventory model).
    fn duplicate(&mut self, item: UnitId, fillers: bool) -> Option<UnitId>;
    /// The tempered affix for `item` (`prefix`: prefix slot, else suffix).
    fn tempered_affix(&mut self, item: UnitId, prefix: bool) -> u16;
    /// Drop the runeword stats of `item`.
    fn drop_runeword_stats(&mut self, item: UnitId);
    /// Repair `item`.
    fn repair(&mut self, item: UnitId);
    /// Recharge `item`.
    fn recharge(&mut self, item: UnitId);
    /// `hst ` / `qf2 ` hooks (Act II / III quests, not defined yet).
    fn quest_item_hook(&mut self, player: UnitId, item: UnitId, code: [u8; 4]);
    /// The `(player, code)` quest item hooks recorded since the last take
    /// (the host runs them on the quest control). Default: none.
    fn take_quest_items(&mut self) -> Vec<(UnitId, [u8; 4])> {
        Vec::new()
    }
    /// Kind 1: the Cow portal, through the game's quest control.
    fn cow_portal(&mut self, player: UnitId) -> bool;
}

/// The cube's part of a game's world host: the cube tables, the staged
/// state, the player data item creation reads, the pending provider and
/// the errors. The items, stats and unit records are the host's economy
/// (the action wiring's unit world); the inventories are the host's
/// inventory model ([`moves::InvParts`]).
pub struct CubeParts {
    /// The cube recipe tables and rules.
    pub cube: CubeData,
    /// State staged by the caller (see [`Staged`]).
    pub staged: Staged,
    /// Player data item creation reads, by player unit (player data is
    /// not in `d2-sim`).
    pub creation: BTreeMap<UnitId, CreationInfo>,
    /// Provider of the calls whose owner is not written yet.
    pub pending: Box<dyn ItemPending + Send + Sync>,
    /// Errors recorded by the handlers, in order.
    pub errors: Vec<ItemError>,
}

impl CubeParts {
    /// Cube parts with default staged state and no creation info or errors.
    pub fn new(cube: CubeData, pending: Box<dyn ItemPending + Send + Sync>) -> Self {
        Self {
            cube,
            staged: Staged::default(),
            creation: BTreeMap::new(),
            pending,
            errors: Vec::new(),
        }
    }
}

/// Whether an active room's level is a town (the stash buttons' common
/// check).
pub trait TownRooms {
    /// Whether `room` belongs to a town level.
    fn room_in_town(&self, game: &Game, room: RoomId) -> bool;
}

impl<X: Pending> TownRooms for ActionHooks<X> {
    fn room_in_town(&self, game: &Game, room: RoomId) -> bool {
        self.drlg.in_town(game, room)
    }
}

/// The hooks an economy needs for the cube and the stash: the unit
/// lifecycle hooks, the stat host and the town test (the action wiring's
/// `ActionHooks`).
pub trait CubeHooks: LifecycleHooks + StatHost + TownRooms {}

impl<H: LifecycleHooks + StatHost + TownRooms> CubeHooks for H {}

/// The quest calls of the cube on the host's quest control, lent for one
/// cube call: kind 1, the Cow portal
/// (`d2_sim::world::quests::cow_portal`) on the call's economy.
pub trait CubeQuests<H> {
    /// Run the Cow portal quest call; `true` when it opened.
    fn cow_portal(&mut self, econ: &mut Economy<'_, H>, player: UnitId) -> bool;
}

/// One call into the cube on the host's economy, its cube parts, the
/// host's inventory model (`None`: every player's inventory is empty)
/// and the host's quest control (`None`: [`ItemPending::cow_portal`]
/// answers). The player's interaction is the unit record's
/// (`d2_sim::units::record::InteractInfo`).
pub trait CubeCall {
    /// What the call returns to the handler.
    type Out;
    /// Run the call on the given economy and parts.
    fn call<H: CubeHooks>(
        self,
        econ: &mut Economy<'_, H>,
        parts: &mut CubeParts,
        inv: Option<&mut InvParts>,
        quests: Option<&mut dyn CubeQuests<H>>,
    ) -> Self::Out;
}

fn result_code(r: u32) -> ResultCode {
    match r {
        0 => ResultCode::Done,
        1 => ResultCode::Refused,
        2 => ResultCode::Invalid,
        _ => ResultCode::Malformed,
    }
}

/// The handler of an item id this module owns, after the dispatcher's
/// gate and size check. C→S 0x4F runs the button dispatch: the stash
/// buttons first, then the cube's. `None`: not an id with a handler
/// here, no player, a host without the cube, or a player-trade button
/// (nothing owns it yet), so the caller keeps its stub.
pub fn handle<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    let id = *msg.first()?;
    if id != ITEM_TO_CUBE && id != CLICK_BUTTON {
        return None;
    }
    let player = sim.player_of(client)?;
    let p = sim.parts();
    p.world.cube(
        p.game,
        p.events,
        CubeRun {
            player,
            client,
            msg,
            out,
        },
    )?
}

/// One cube message: the module call, then the messages for the acting
/// client in order.
struct CubeRun<'m> {
    player: UnitId,
    client: ClientId,
    msg: &'m [u8],
    out: &'m mut dyn MessageSink,
}

impl CubeCall for CubeRun<'_> {
    type Out = Option<ResultCode>;
    fn call<H: CubeHooks>(
        self,
        econ: &mut Economy<'_, H>,
        parts: &mut CubeParts,
        inv: Option<&mut InvParts>,
        quests: Option<&mut dyn CubeQuests<H>>,
    ) -> Option<ResultCode> {
        let CubeParts {
            cube,
            staged,
            creation,
            pending,
            errors,
        } = parts;
        let mut info = cube_world::InfoRest(creation);
        let mut w = cube_world::ServerCube::new(
            EconomyCube::new(econ, &mut info),
            staged,
            pending.as_mut(),
            inv,
            self.player,
        )
        .with_quests(quests.map(|q| q as &mut dyn CubeQuests<H>));
        let code = if self.msg[0] == ITEM_TO_CUBE {
            Some(cube.put_in(&mut w, self.player, self.msg))
        } else {
            // 0x4F: [button u16@1][p1 u16@3][p2 u16@5] (size 7 already
            // checked); the stash buttons are tried before the cube's.
            let le = |o: usize| u16::from_le_bytes([self.msg[o], self.msg[o + 1]]);
            let button = le(1);
            let v = stash::button_value(le(3), le(5));
            match stash::click_button(&mut w, self.player, button, v) {
                ButtonRoute::Done(r) => Some(r),
                ButtonRoute::Cube => cube.click_button(&mut w, self.player, button),
                // The player-trade switch has no owner yet.
                ButtonRoute::Trade => None,
            }
        };
        let (sent, errs) = w.finish();
        errors.extend(errs);
        for m in sent {
            if let Err(e) = self.out.queue(self.client, &m) {
                errors.push(ItemError::Sink(e));
            }
        }
        code.map(result_code)
    }
}
