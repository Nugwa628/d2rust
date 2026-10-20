//! `world::npc` ↔ `world::vendors`, both directions:
//!
//! - [`NpcVendors`] on [`Desk`]: trade / gamble open ([`store::open`]) on
//!   the NPC record's [`VendorRecord`] and the NPC-control seed, the
//!   gamble-list drop, pay and item repair.
//! - [`NpcLink`] on [`VendorDesk`]: NPC lookups on the unit lists and the
//!   interaction lists, the hire-list flag and the hire list of
//!   [`NpcControl`].
//! - The vendor message handlers that need both (C→S 0x32, 0x33, 0x35,
//!   0x37) and the refresh hooks as [`VendorDesk`] entry points.

use super::NpcRest;
use super::{Desk, InteractionError, PlayerQuestsRef, VendorRest};
use crate::rng::Seed;
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};
use crate::wiring::economy::QuestRest;
use crate::world::npc::{NpcControl, NpcError, NpcVendors, NpcWorld, UNIT_MONSTER};
use crate::world::vendors::price::PriceFatal;
use crate::world::vendors::store::{self, StoreCtx};
use crate::world::vendors::trade::{self, BuyMsg, RepairMsg, SellMsg};
use crate::world::vendors::{gamble, NpcLink, VendorRecord};

/// The vendors' world: the desk plus the NPC control block, when the
/// call has one (trade open, the vendor handlers; not pay / repair from
/// an NPC service).
pub struct VendorDesk<'v, 'd, 'a, H, R> {
    pub desk: &'v mut Desk<'d, 'a, H, R>,
    pub ctl: Option<&'v mut NpcControl>,
    /// The NPC unit of a trade open (the owner of the items it creates).
    pub npc: Option<UnitId>,
}

impl<'d, 'a, H, R> Desk<'d, 'a, H, R> {
    /// The vendors' view of this desk.
    pub fn vendors<'v>(
        &'v mut self,
        ctl: Option<&'v mut NpcControl>,
    ) -> VendorDesk<'v, 'd, 'a, H, R> {
        VendorDesk {
            desk: self,
            ctl,
            npc: None,
        }
    }
}

impl<'a, H, R> Desk<'_, 'a, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    /// Runs `f` on the vendor record of `class`, lent out of the state
    /// for the call (the record is not reachable through the world while
    /// a vendor function holds it).
    fn with_record<T>(
        &mut self,
        class: u16,
        f: impl FnOnce(&mut VendorRecord, &mut Self) -> T,
    ) -> Option<T> {
        let Some(i) = self.state.vendor_index(class) else {
            self.state.errors.push(InteractionError::NoRecord(class));
            return None;
        };
        let mut rec = std::mem::take(&mut self.state.vendors[i]);
        let out = f(&mut rec, self);
        self.state.vendors[i] = rec;
        Some(out)
    }

    /// The vendor record class of the interact NPC with GUID `guid`.
    fn npc_class_of(&self, guid: u32) -> Option<u16> {
        let npc = NpcWorld::monster_by_guid(self, guid)?;
        NpcWorld::monster_class(self, npc)
    }
}

impl<'a, H, R> NpcVendors for Desk<'_, 'a, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    /// Opens a trade (or gamble) with `npc`, on the record of the NPC's
    /// class. Store generation and the hire list draw from the NPC-control
    /// seed; the seed is lent to the store code and back to
    /// [`NpcControl::make_hire_list`] through
    /// [`NpcLink::make_hire_list`].
    fn open_trade(
        &mut self,
        ctl: &mut NpcControl,
        player: UnitId,
        npc: UnitId,
        single: bool,
        gamble: bool,
    ) -> Result<(), NpcError> {
        let class = NpcWorld::monster_class(self, npc).unwrap_or(0);
        let tables = self.vendor_tables;
        let now = self.now;
        let mut seed = ctl.seed;
        self.state.shown_player = Some(player);
        self.state.shown_npc = Some(npc);
        self.state.shown_class = class;
        self.state.shown_gamble = gamble;
        let r = self.with_record(class, |rec, desk| {
            let mut c = StoreCtx {
                tables,
                seed: &mut seed,
            };
            let mut v = desk.vendors(Some(&mut *ctl));
            v.npc = Some(npc);
            match store::open(&mut c, rec, &mut v, npc, player, single, gamble, now) {
                Ok(()) => v.take_npc_error(),
                Err(e) => Some(e.into()),
            }
        });
        ctl.seed = seed;
        match r {
            Some(Some(e)) => Err(e),
            _ => Ok(()),
        }
    }

    /// Drops the player's gamble list on the record of the NPC's class.
    fn drop_gamble_list(&mut self, player: UnitId, npc: UnitId) {
        let class = NpcWorld::monster_class(self, npc).unwrap_or(0);
        let pg = NpcWorld::guid(self, player);
        self.with_record(class, |rec, desk| {
            gamble::drop_list(rec, &mut desk.vendors(None), pg);
        });
    }

    /// Charges `cost` to the player.
    fn pay(&mut self, player: UnitId, cost: u32) -> bool {
        trade::pay(&mut self.vendors(None), player, cost as i32)
    }

    /// Repairs `item`.
    ///
    /// TODO: the service passes the item only; the player argument of the
    /// repair routine is read as none (no 0x3E item-stat messages).
    fn repair(&mut self, item: UnitId) {
        let t = self.vendor_tables;
        trade::repair_item(t, &mut self.vendors(None), item, None);
    }
}

impl<H, R> VendorDesk<'_, '_, '_, H, R> {
    /// An NPC error a hire-list call raised inside a vendor function.
    fn take_npc_error(&mut self) -> Option<NpcError> {
        let i = self
            .desk
            .state
            .errors
            .iter()
            .rposition(|e| matches!(e, InteractionError::Npc(_)))?;
        match self.desk.state.errors.remove(i) {
            InteractionError::Npc(e) => Some(e),
            _ => None,
        }
    }

    fn no_ctl(&mut self) {
        self.desk
            .state
            .errors
            .push(InteractionError::Npc(NpcError::Table(
                "hire list without the NPC control block".into(),
            )));
    }
}

impl<H, R> NpcLink for VendorDesk<'_, '_, '_, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    /// A monster with an interaction list.
    fn npc_by_guid(&self, guid: u32) -> Option<UnitId> {
        let npc = self
            .desk
            .econ
            .game
            .lists
            .find_unit(UnitType::Monster, guid)?;
        self.desk.state.lists.contains_key(&npc).then_some(npc)
    }
    fn npc_class(&self, npc: UnitId) -> u16 {
        self.desk.econ.units.get(npc).map_or(0, |r| r.class as u16)
    }
    /// Starting an NPC interaction needs the player's interact unit to be
    /// (1, GUID).
    fn is_interact_unit(&self, player: UnitId, npc: UnitId) -> bool {
        let guid = NpcWorld::guid(&*self.desk, npc);
        NpcWorld::interact_unit(&*self.desk, player) == Some((UNIT_MONSTER, guid))
    }
    fn interaction_empty(&self, npc: UnitId) -> bool {
        self.desk
            .state
            .lists
            .get(&npc)
            .is_none_or(|l| l.nodes.is_empty())
    }
    /// Whether the hire list of the class's record has been made.
    fn hire_list_made(&self, class: u16) -> bool {
        self.ctl
            .as_ref()
            .and_then(|c| c.record(class))
            .is_some_and(|r| r.hire_made)
    }
    fn set_hire_list_made(&mut self, class: u16) {
        let Some(ctl) = self.ctl.as_mut() else {
            self.no_ctl();
            return;
        };
        if let Some(r) = ctl.record_mut(class) {
            r.hire_made = true;
        }
    }
    /// Makes the hire list of the class's record on the lent NPC-control
    /// seed.
    fn make_hire_list(&mut self, class: u16, seed: &mut Seed) {
        let Some(ctl) = self.ctl.as_mut() else {
            self.no_ctl();
            return;
        };
        std::mem::swap(&mut ctl.seed, seed);
        let r = ctl.make_hire_list(class);
        std::mem::swap(&mut ctl.seed, seed);
        if let Err(e) = r {
            self.desk.state.errors.push(InteractionError::Npc(e));
        }
    }
}

impl<H, R> VendorDesk<'_, '_, '_, H, R>
where
    H: LifecycleHooks,
    R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef,
{
    /// The class of the NPC a message names, for its vendor record: the
    /// record of the message NPC's class.
    fn message_class(&self, npc_guid: u32) -> Option<u16> {
        self.desk.npc_class_of(npc_guid)
    }

    /// Runs a vendor handler on the record of `class`; without a class
    /// (no NPC with the message's GUID: the handler refuses before it
    /// reads the record) on an empty record.
    fn on_record(
        &mut self,
        class: Option<u16>,
        f: impl FnOnce(&mut VendorRecord, &mut Self) -> Result<u32, PriceFatal>,
    ) -> u32 {
        let i = match class {
            Some(c) => match self.desk.state.vendor_index(c) {
                Some(i) => Some(i),
                None => {
                    self.desk.state.errors.push(InteractionError::NoRecord(c));
                    return 0;
                }
            },
            None => None,
        };
        let mut rec = match i {
            Some(i) => std::mem::take(&mut self.desk.state.vendors[i]),
            None => VendorRecord::default(),
        };
        let r = f(&mut rec, self);
        if let Some(i) = i {
            self.desk.state.vendors[i] = rec;
        }
        match r {
            Ok(v) => v,
            Err(e) => {
                self.desk.state.errors.push(InteractionError::Price(e));
                0
            }
        }
    }

    /// C→S 0x32 BuyItem. `None`: a bad message.
    pub fn buy(&mut self, player: UnitId, msg: &[u8]) -> Option<u32> {
        let m = BuyMsg::parse(msg)?;
        let class = self.message_class(m.npc);
        let t = self.desk.vendor_tables;
        Some(self.on_record(class, |rec, v| trade::buy(t, rec, v, player, &m)))
    }

    /// C→S 0x33 SellItem.
    pub fn sell(&mut self, player: UnitId, msg: &[u8]) -> Option<u32> {
        let m = SellMsg::parse(msg)?;
        let class = self.message_class(m.npc);
        let t = self.desk.vendor_tables;
        Some(self.on_record(class, |rec, v| trade::sell(t, rec, v, player, &m)))
    }

    /// C→S 0x35 Repair: the handler drops the routine's result, so the
    /// answer is 0 for every 17-byte message.
    pub fn repair(&mut self, player: UnitId, msg: &[u8]) -> Option<u32> {
        let m = RepairMsg::parse(msg)?;
        let t = self.desk.vendor_tables;
        match trade::repair(t, self, player, &m) {
            Ok(_) => Some(0),
            Err(e) => {
                self.desk.state.errors.push(InteractionError::Price(e));
                Some(0)
            }
        }
    }

    /// C→S 0x37 IdentifyGamble.
    pub fn identify_gamble(&mut self, player: UnitId, msg: &[u8]) -> u32 {
        gamble::identify_gamble(self, player, msg)
    }

    /// A player leaves level `from` for level `to`: the vendor refresh.
    pub fn level_changed(&mut self, player: UnitId, from: u16, to: u16) {
        let now = self.desk.now;
        let mut records = std::mem::take(&mut self.desk.state.vendors);
        store::level_changed(&mut records, self, player, from, to, now);
        self.desk.state.vendors = records;
    }

    /// A client leaves the game: the vendor refresh.
    pub fn client_left(&mut self, player: UnitId) {
        let now = self.desk.now;
        let mut records = std::mem::take(&mut self.desk.state.vendors);
        store::client_left(&mut records, self, player, now);
        self.desk.state.vendors = records;
    }
}
