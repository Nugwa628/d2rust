//! [`CubeWorld`] for the server: the cube's view of the game during one handler call.
//!
//! It combines the economy wiring's [`EconomyCube`] (items, stats, unit
//! records and creation, the player's interact info on the unit record
//! too); the game's one inventory model ([`InvParts`],
//! `d2_sim::wiring::inventory`) for the item list, the put-item and
//! cube checks, the targeting reset, placement, removal and the item
//! copy (`InvDesk::copy_of`); the staged date; the game's sound slots
//! (`d2_sim::units::sound`); and [`ItemPending`] for the calls nothing
//! else owns yet. [`super::handle`] builds one [`ServerCube`] per cube
//! or stash-button message; the stash buttons run on the same providers.

use std::cell::RefCell;
use std::collections::BTreeMap;

use d2_sim::game::GameError;
use d2_sim::items::moves::{iflag, page};
use d2_sim::rng::Seed;
use d2_sim::units::sound::queue_sound;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::economy::{CubeRest, EconomyCube, EconomyError};
use d2_sim::wiring::inventory::InvDesk;
use d2_sim::world::cube::{CraftProperty, CubeWorld, ItemRequest, StatRead};
use d2_sim::world::stash::StashWorld;

use super::moves::{take_sent, InvParts, MoveRest};
use super::{CubeHooks, CubeQuests, ItemError, ItemPending, Staged};

/// The player data item creation reads: the name and the client's
/// hardcore flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreationInfo {
    /// The player's name, zero padded.
    pub name: [u8; 16],
    /// The client's hardcore flag, `None` when unknown.
    pub hardcore: Option<bool>,
}

/// The rest [`EconomyCube`] calls itself: only `player_info` (from its
/// `create_item`). Every other [`CubeRest`] call is answered by
/// [`ServerCube`] before it reaches the economy.
pub(super) struct InfoRest<'r>(pub &'r BTreeMap<UnitId, CreationInfo>);

const OVERRIDDEN: &str = "ServerCube answers every CubeRest call but player_info";

impl CubeRest for InfoRest<'_> {
    fn player_info(&self, player: UnitId) -> Option<([u8; 16], Option<bool>)> {
        self.0.get(&player).map(|c| (c.name, c.hardcore))
    }
    fn local_date(&self) -> (u8, u8) {
        unreachable!("{OVERRIDDEN}")
    }
    fn attach_sound(&mut self, _: UnitId, _: u8) {
        unreachable!("{OVERRIDDEN}")
    }
    fn send(&mut self, _: UnitId, _: &[u8]) {
        unreachable!("{OVERRIDDEN}")
    }
    fn inventory_pass(&mut self, _: UnitId) {
        unreachable!("{OVERRIDDEN}")
    }
    fn inventory(&self, _: UnitId) -> Vec<UnitId> {
        unreachable!("{OVERRIDDEN}")
    }
    fn socketed(&self, _: UnitId) -> Vec<UnitId> {
        unreachable!("{OVERRIDDEN}")
    }
    fn place(&mut self, _: UnitId, _: UnitId) -> bool {
        unreachable!("{OVERRIDDEN}")
    }
    fn remove_cube_item(&mut self, _: UnitId, _: UnitId) {
        unreachable!("{OVERRIDDEN}")
    }
    fn targeting_reset(&mut self, _: UnitId) {
        unreachable!("{OVERRIDDEN}")
    }
    fn put_item_check(&self, _: UnitId, _: u32) -> u32 {
        unreachable!("{OVERRIDDEN}")
    }
    fn cube_check(&self, _: UnitId, _: u32) -> bool {
        unreachable!("{OVERRIDDEN}")
    }
    fn duplicate(&mut self, _: UnitId, _: bool) -> Option<UnitId> {
        unreachable!("{OVERRIDDEN}")
    }
    fn tempered_affix(&mut self, _: UnitId, _: bool) -> u16 {
        unreachable!("{OVERRIDDEN}")
    }
    fn drop_runeword_stats(&mut self, _: UnitId) {
        unreachable!("{OVERRIDDEN}")
    }
    fn repair(&mut self, _: UnitId) {
        unreachable!("{OVERRIDDEN}")
    }
    fn recharge(&mut self, _: UnitId) {
        unreachable!("{OVERRIDDEN}")
    }
    fn quest_item_hook(&mut self, _: UnitId, _: UnitId, _: [u8; 4]) {
        unreachable!("{OVERRIDDEN}")
    }
    fn cow_portal(&mut self, _: UnitId) -> bool {
        unreachable!("{OVERRIDDEN}")
    }
}

/// The cube's world on the server, for one handler call by `player`.
///
/// The economy and the inventory parts sit in cells: the item checks the
/// cube asks through `&self` run on an inventory desk, which borrows the
/// economy mutably (it fills its item data copies).
pub(super) struct ServerCube<'e, 'a, 'r, H> {
    econ: RefCell<EconomyCube<'e, 'a, H, InfoRest<'r>>>,
    inv: RefCell<Option<&'e mut InvParts>>,
    staged: &'e mut Staged,
    pending: &'e mut dyn ItemPending,
    quests: Option<&'e mut dyn CubeQuests<H>>,
    player: UnitId,
    sent: Vec<Vec<u8>>,
    errors: Vec<ItemError>,
}

/// The inventory desk of one cube call.
type CubeDesk<'d, 'a, H> = InvDesk<'d, 'a, H, dyn MoveRest + Send + Sync>;

impl<'e, 'a, 'r, H: CubeHooks> ServerCube<'e, 'a, 'r, H> {
    pub(super) fn new(
        econ: EconomyCube<'e, 'a, H, InfoRest<'r>>,
        staged: &'e mut Staged,
        pending: &'e mut dyn ItemPending,
        inv: Option<&'e mut InvParts>,
        player: UnitId,
    ) -> Self {
        Self {
            econ: RefCell::new(econ),
            inv: RefCell::new(inv),
            staged,
            pending,
            quests: None,
            player,
            sent: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// The host's quest control for the cube's quest calls
    /// ([`CubeWorld::cow_portal`]).
    pub(super) fn with_quests(mut self, quests: Option<&'e mut dyn CubeQuests<H>>) -> Self {
        self.quests = quests;
        self
    }

    /// The messages for the acting client, in order, and the errors.
    pub(super) fn finish(self) -> (Vec<Vec<u8>>, Vec<ItemError>) {
        let mut errors: Vec<_> = self
            .econ
            .into_inner()
            .errors
            .into_iter()
            .map(ItemError::Economy)
            .collect();
        errors.extend(self.errors);
        (self.sent, errors)
    }

    /// The economy cube (reads).
    fn ec(&self) -> std::cell::Ref<'_, EconomyCube<'e, 'a, H, InfoRest<'r>>> {
        self.econ.borrow()
    }

    /// The economy cube (writes).
    fn ec_mut(&mut self) -> &mut EconomyCube<'e, 'a, H, InfoRest<'r>> {
        self.econ.get_mut()
    }

    /// Runs `f` on the inventory desk over this call's economy (`None`:
    /// a host without inventory parts).
    fn read_desk<T>(&self, f: impl FnOnce(&CubeDesk<'_, 'a, H>) -> T) -> Option<T> {
        let mut inv = self.inv.borrow_mut();
        let parts = inv.as_deref_mut()?;
        let mut ec = self.econ.borrow_mut();
        let d = parts.desk(&mut *ec.econ);
        Some(f(&d))
    }

    /// Runs `f` on the inventory desk; the messages it queued go to the
    /// acting client in order (another receiver is an error, as for
    /// [`CubeWorld::send`]).
    fn with_desk<T>(&mut self, f: impl FnOnce(&mut CubeDesk<'_, 'a, H>) -> T) -> Option<T> {
        let parts = self.inv.get_mut().as_deref_mut()?;
        let mut d = parts.desk(&mut *self.econ.get_mut().econ);
        let out = f(&mut d);
        let sent = take_sent(&mut d);
        for (unit, bytes) in sent {
            match unit {
                Some(u) if u == self.player => self.sent.push(bytes),
                Some(u) => self.errors.push(ItemError::OtherPlayer(u)),
                None => {}
            }
        }
        Some(out)
    }
}

impl<H: CubeHooks> CubeWorld for ServerCube<'_, '_, '_, H> {
    fn expansion(&self) -> bool {
        self.ec().expansion()
    }
    fn game_type(&self) -> u8 {
        self.ec().game_type()
    }
    fn ladder(&self) -> bool {
        self.ec().ladder()
    }
    fn difficulty(&self) -> u8 {
        self.ec().difficulty()
    }
    fn item_format(&self) -> u16 {
        self.ec().item_format()
    }
    fn local_date(&self) -> (u8, u8) {
        self.staged.local_date
    }
    fn game_seed(&mut self) -> &mut Seed {
        self.ec_mut().game_seed()
    }

    fn player_class(&self, player: UnitId) -> u8 {
        self.ec().player_class(player)
    }
    fn stat(&self, unit: UnitId, read: StatRead, stat: u16) -> i32 {
        self.ec().stat(unit, read, stat)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.ec_mut().set_stat(unit, stat, value)
    }
    /// Queue the sound on the game's sound slots with the player as
    /// target, so only its own client hears the S→C 0x2C of its unit
    /// update.
    fn attach_sound(&mut self, player: UnitId, event: u8) {
        self.staged.sounds.push((player, event));
        let game = &mut *self.ec_mut().econ.game;
        if let Err(e) = queue_sound(game, player, u16::from(event), Some(player)) {
            self.errors
                .push(ItemError::Economy(EconomyError::Game(GameError::from(e))));
        }
    }
    /// Only the acting player's client receives (every cube path sends
    /// to the player it acts for).
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        if player == self.player {
            self.sent.push(msg.to_vec());
        } else {
            self.errors.push(ItemError::OtherPlayer(player));
        }
    }

    /// The interact info on the player's unit record (the economy
    /// cube's).
    fn interaction(&self, player: UnitId) -> Option<(u8, u32)> {
        self.ec().interaction(player)
    }
    fn set_interaction(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        self.ec_mut().set_interaction(player, unit_type, guid)
    }
    fn reset_interaction(&mut self, player: UnitId) {
        self.ec_mut().reset_interaction(player)
    }
    fn inventory_pass(&mut self, player: UnitId) {
        self.pending.inventory_pass(player, &mut self.sent);
    }
    fn interacting_with_stash(&self, player: UnitId) -> bool {
        self.ec().interacting_with_stash(player)
    }
    fn trading(&self, player: UnitId) -> bool {
        self.ec().trading(player)
    }

    /// The player's item list in link order.
    fn inventory(&self, player: UnitId) -> Vec<UnitId> {
        self.inv
            .borrow()
            .as_deref()
            .map_or_else(Vec::new, |p| p.state.items_of(player))
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.ec().item_by_guid(guid)
    }
    fn item_guid(&self, item: UnitId) -> u32 {
        self.ec().item_guid(item)
    }
    fn item_page(&self, item: UnitId) -> u8 {
        self.ec().item_page(item)
    }
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        self.ec_mut().set_item_page(item, page)
    }
    fn item_mode(&self, item: UnitId) -> u8 {
        self.ec().item_mode(item)
    }
    fn set_item_mode(&mut self, item: UnitId, mode: u8) {
        self.ec_mut().set_item_mode(item, mode)
    }
    fn item_class(&self, item: UnitId) -> Option<u32> {
        self.ec().item_class(item)
    }
    fn set_item_class(&mut self, item: UnitId, class: u32) {
        self.ec_mut().set_item_class(item, class)
    }
    fn class_is_type(&self, class: u32, ty: u16) -> bool {
        self.ec().class_is_type(class, ty)
    }
    fn item_quality(&self, item: UnitId) -> u8 {
        self.ec().item_quality(item)
    }
    fn item_file_index(&self, item: UnitId) -> u32 {
        self.ec().item_file_index(item)
    }
    fn item_level(&self, item: UnitId) -> i32 {
        self.ec().item_level(item)
    }
    fn set_item_level(&mut self, item: UnitId, level: i32) {
        self.ec_mut().set_item_level(item, level)
    }
    fn item_flags(&self, item: UnitId) -> u32 {
        self.ec().item_flags(item)
    }
    fn set_item_flag(&mut self, item: UnitId, flag: u32) {
        self.ec_mut().set_item_flag(item, flag)
    }
    fn item_sockets(&self, item: UnitId) -> i32 {
        self.ec().item_sockets(item)
    }
    fn max_sockets(&self, item: UnitId) -> i32 {
        self.ec().max_sockets(item)
    }
    fn add_sockets(&mut self, item: UnitId, n: i32) {
        self.ec_mut().add_sockets(item, n)
    }
    fn item_seed(&mut self, item: UnitId) -> &mut Seed {
        self.ec_mut().item_seed(item)
    }
    /// The item's own inventory, in link order (its socket fillers).
    fn socketed(&self, item: UnitId) -> Vec<UnitId> {
        self.inv
            .borrow()
            .as_deref()
            .map_or_else(Vec::new, |p| p.state.fillers(item))
    }
    /// Copy the item on the inventory model (`InvDesk::copy_of`), as the
    /// vendors' copy; without inventory parts: [`ItemPending::duplicate`].
    fn duplicate(&mut self, item: UnitId, fillers: bool) -> Option<UnitId> {
        if self.inv.get_mut().is_none() {
            return self.pending.duplicate(item, fillers);
        }
        self.with_desk(|d| d.copy_of(item, fillers)).flatten()
    }
    fn item_init(&mut self, item: UnitId) -> Option<UnitId> {
        self.ec_mut().item_init(item)
    }
    fn create_item(&mut self, request: &ItemRequest) -> Option<UnitId> {
        self.ec_mut().create_item(request)
    }
    fn tempered_affix(&mut self, item: UnitId, prefix: bool) -> u16 {
        self.pending.tempered_affix(item, prefix)
    }
    fn set_tempered(&mut self, item: UnitId, prefix: u16, suffix: u16) {
        self.ec_mut().set_tempered(item, prefix, suffix)
    }
    fn unique_found(&self, index: u32) -> bool {
        self.ec().unique_found(index)
    }
    fn set_unique_found(&mut self, index: u32, found: bool) {
        self.ec_mut().set_unique_found(index, found)
    }
    fn drop_runeword_stats(&mut self, item: UnitId) {
        self.pending.drop_runeword_stats(item)
    }
    fn add_craft_property(&mut self, item: UnitId, prop: &CraftProperty) {
        self.ec_mut().add_craft_property(item, prop)
    }
    fn repair(&mut self, item: UnitId) {
        self.pending.repair(item)
    }
    fn recharge(&mut self, item: UnitId) {
        self.pending.recharge(item)
    }
    /// Place the item in the player's inventory at a free position, and
    /// send it to the client.
    fn place(&mut self, player: UnitId, item: UnitId) -> bool {
        self.with_desk(|d| d.place(player, item, (0, 0), true, true))
            .unwrap_or(false)
    }
    fn free_item(&mut self, item: UnitId) {
        self.ec_mut().free_item(item)
    }
    /// Remove one consumed item: S→C 0x9D action 5 first (item flags
    /// | 0x20, the stored page set to 3 and shown), then the item is
    /// unlinked from the inventory and freed through the inventory
    /// wiring.
    ///
    /// TODO: an unlink failure has no defined outcome (the item is in
    /// the list, the caller walks it); it is ignored and the free runs.
    fn remove_cube_item(&mut self, player: UnitId, item: UnitId) {
        let fatal = self.with_desk(|d| {
            let r = d.send_item_page(player, item, iflag::COPIED, page::CUBE);
            d.remove(player, item);
            d.free(item);
            r
        });
        if let Some(Err(e)) = fatal {
            self.errors.push(ItemError::Move(e));
        }
    }
    /// Targeting reset on the player's item list (`InvDesk::reset_targeting`):
    /// item flag 0x4 cleared, S→C 0x3F queued per item where the reset asks.
    fn targeting_reset(&mut self, player: UnitId) {
        self.with_desk(|d| d.reset_targeting(player));
    }
    /// The put-item check (the item is on the ground or owned by the
    /// player) on the inventory model. Without inventory parts: 1.
    fn put_item_check(&self, player: UnitId, item: u32) -> u32 {
        self.read_desk(|d| u32::from(d.check_ground_or_owned(player, item)))
            .unwrap_or(1)
    }
    /// The cube check (the cube is a stored item of the player).
    fn cube_check(&self, player: UnitId, cube: u32) -> bool {
        self.read_desk(|d| d.check_stored(player, cube) == 0)
            .unwrap_or(false)
    }
    fn quest_item_hook(&mut self, player: UnitId, item: UnitId, code: [u8; 4]) {
        self.pending.quest_item_hook(player, item, code)
    }
    /// The Cow portal on the host's quest control:
    /// [`d2_sim::world::quests::cow_portal`]. A refusal queues sound 20,
    /// staged here as the cube's other sounds are. Without a quest
    /// control, [`ItemPending`] answers and a refusal plays the same
    /// sound.
    fn cow_portal(&mut self, player: UnitId) -> bool {
        const REFUSED: u8 = d2_sim::world::cube::SOUND_COW_REFUSED;
        if let Some(q) = self.quests.as_deref_mut() {
            let made = q.cow_portal(&mut *self.econ.get_mut().econ, player);
            if !made {
                self.staged.sounds.push((player, REFUSED));
            }
            return made;
        }
        let made = self.pending.cow_portal(player);
        if !made {
            CubeWorld::attach_sound(self, player, REFUSED);
        }
        made
    }
}

/// The stash buttons on the same providers: the
/// interact info on the player's unit record, the game's unit lists and
/// unit records for the stash object, the hooks' DRLG for the town test,
/// the stat lists and the game's sound slots.
impl<H: CubeHooks> StashWorld for ServerCube<'_, '_, '_, H> {
    fn interaction(&self, player: UnitId) -> Option<(u8, u32)> {
        CubeWorld::interaction(self, player)
    }
    fn reset_interaction(&mut self, player: UnitId) {
        CubeWorld::reset_interaction(self, player)
    }
    fn inventory_pass(&mut self, player: UnitId) {
        CubeWorld::inventory_pass(self, player)
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        CubeWorld::send(self, player, msg)
    }
    fn object_by_guid(&self, guid: u32) -> Option<(UnitId, u32)> {
        let ec = self.ec();
        let u = ec.econ.game.lists.find_unit(UnitType::Object, guid)?;
        Some((u, ec.econ.units.get(u)?.class))
    }
    fn in_town(&self, unit: UnitId) -> bool {
        let ec = self.ec();
        let game = &*ec.econ.game;
        game.lists
            .unit(unit)
            .and_then(|e| e.room())
            .is_some_and(|room| ec.econ.hooks.room_in_town(game, room))
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.ec().econ.stats.unit_total(unit, stat, 0)
    }
    fn set_base(&mut self, unit: UnitId, stat: u16, value: i32) {
        let e = &mut *self.ec_mut().econ;
        e.stats.unit_set(e.hooks, unit, stat, value, 0);
    }
    fn add_base(&mut self, unit: UnitId, stat: u16, d: i32) {
        let e = &mut *self.ec_mut().econ;
        let v = e.stats.unit_base(unit, stat, 0).wrapping_add(d);
        e.stats.unit_set(e.hooks, unit, stat, v, 0);
    }
    fn is_player(&self, unit: UnitId) -> bool {
        self.ec()
            .econ
            .game
            .lists
            .unit(unit)
            .is_some_and(|e| e.ty == UnitType::Player)
    }
    /// Overflow gold drop of the receive path: the withdraw checks the
    /// carried cap first, so the stash never reaches it.
    fn drop_gold(&mut self, _: UnitId, _: i32) {}
    fn sound(&mut self, unit: UnitId, event: u16, target: Option<UnitId>) {
        let game = &mut *self.ec_mut().econ.game;
        if let Err(e) = queue_sound(game, unit, event, target) {
            self.errors
                .push(ItemError::Economy(EconomyError::Game(GameError::from(e))));
        }
    }
}
