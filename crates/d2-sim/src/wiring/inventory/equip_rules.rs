//! The equipment bookkeeping (`items::inventory::bookkeeping`) and the
//! set-item state update (`items::set_state`) on [`InvDesk`]: the
//! inventories, the item data copies, the item store, the item tables
//! and the stat lists (links of the set lists, park / unpark) and the
//! books table's skill columns. The
//! skill list, the player data mouse slots, the stat link of an item and the
//! messages have no d2-sim owner: [`InvRest`]'s equipment seams.
//!
//! The rules run in place of the [`InvRest`] calls of the same names
//! (`MovePending::charm_relink`, `charm_unlink`, `inventory_pass`,
//! `weapon_bookkeeping`) when [`InvState::equip_rules`] is on.

use super::{InvDesk, InvError, InvRest};
use crate::items::inventory::bookkeeping::{self as bk, EquipWorld, SkillRef};
use crate::items::inventory::checks::{active_inventory_item, usable};
use crate::items::inventory::{body, iflag, node, InvWorld};
use crate::items::set_state::{self, SetItemRow, SetWorld, QUALITY_SET, STAT_SET_ID};
use crate::items::ty;
use crate::skills::use_::item_type::{item_type_test, ItemTypeView};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};

/// An equipment-rule call asked by an inventory function while the
/// owner's inventory was lent out (`InvState::equip_queue`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipCall {
    /// Item-skill link (owner, item): scroll and tome skill quantities.
    SkillLink(UnitId, UnitId),
    /// Inventory pass with send 0.
    Pass(UnitId),
    /// Weapon bookkeeping: the mouse skills against the equipped weapons.
    Weapons(UnitId),
    /// The set-item update after an item was linked (owner, item;
    /// [`item_link`](super::item_link)).
    SetLink(UnitId, UnitId),
    /// The set-item update after an item left the body.
    SetUnlink(UnitId, UnitId),
    /// The charm links of the inventory pass.
    Charms(UnitId),
}

/// Unit type 4 (item), the owner type of a new set list.
const OWNER_TYPE_ITEM: u32 = 4;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// Logs a fatal assert of the rules.
    fn note_equip<T>(&mut self, r: Result<T, bk::EquipFatal>) {
        if let Err(e) = r {
            self.state.errors.push(InvError::Equip(e));
        }
    }

    /// Queues a call for the end of the inventory call (the owner's
    /// inventory is lent to the inventory function now).
    pub(super) fn queue_equip(&mut self, c: EquipCall) {
        self.state.equip_queue.push(c);
    }

    /// Runs the queued calls, in order. D2RUST order: the calls are queued
    /// rather than run inline at their call sites (the grid placement and
    /// the equip); the later steps of those functions read no skill
    /// quantity, mouse skill, stat link or 0x4000 flag they change.
    pub(super) fn run_equip_queue(&mut self) {
        for c in std::mem::take(&mut self.state.equip_queue) {
            match c {
                EquipCall::SkillLink(o, i) => self.run_item_skill_link(o, i, true),
                EquipCall::Pass(o) => self.run_inventory_pass(o, false),
                EquipCall::Weapons(o) => self.run_weapon_bookkeeping(o),
                EquipCall::SetLink(o, i) => self.run_set_link(o, i),
                EquipCall::SetUnlink(o, i) => self.run_set_unlink(o, i),
                EquipCall::Charms(o) => self.run_link_charms(o),
            }
        }
    }

    /// Item-skill link (`add`) on the rules.
    pub fn run_item_skill_link(&mut self, owner: UnitId, item: UnitId, add: bool) {
        let r = bk::item_skill_link(self, owner, item, add);
        self.note_equip(r);
    }

    /// Book count change by `n` on the rules
    /// ([`bk::item_skill_add`]).
    pub fn run_item_skill_add(&mut self, owner: UnitId, book: UnitId, n: i32) {
        let r = bk::item_skill_add(self, owner, book, n);
        self.note_equip(r);
    }

    /// Skill decrement of `skill` on the rules ([`bk::skill_decrement`]).
    pub fn run_skill_decrement(&mut self, owner: UnitId, skill: i32) {
        let r = bk::skill_decrement(self, owner, skill);
        self.note_equip(r);
    }

    /// Whether the player has `skill`: its quantity slot exists.
    pub fn owns_skill(&self, owner: UnitId, skill: i32) -> bool {
        bk::EquipWorld::skill_quantity(self, owner, skill).is_some()
    }

    /// Removal of `n` from an item's skill quantity on the rules
    /// ([`bk::item_skill_remove`]).
    pub fn run_item_skill_remove(&mut self, owner: UnitId, item: UnitId, n: i32) {
        bk::item_skill_remove(self, owner, item, n);
    }

    /// Cube recount on the rules.
    pub fn run_cube_recount(&mut self, owner: UnitId) {
        let r = bk::cube_recount(self, owner);
        self.note_equip(r);
    }

    /// Inventory pass (`send` as given) on the rules.
    pub fn run_inventory_pass(&mut self, owner: UnitId, send: bool) {
        bk::inventory_pass(self, owner, send);
        self.sync_out();
    }

    /// Weapon bookkeeping on the rules.
    pub fn run_weapon_bookkeeping(&mut self, owner: UnitId) {
        let r = bk::weapon_bookkeeping(self, owner);
        self.note_equip(r);
    }

    /// The set-item update for `owner` and `item` with arguments `r`
    /// and `p` (see `items::set_state`).
    pub fn run_set_item_update(&mut self, owner: UnitId, item: UnitId, r: i32, p: i32) -> bool {
        set_state::set_item_update(self, Some(owner), Some(item), r, p)
    }

    fn inv_items(&self, u: UnitId) -> Vec<UnitId> {
        self.state
            .inventories
            .get(&u)
            .map(|i| i.items().to_vec())
            .unwrap_or_default()
    }

    fn record_type(&self, i: UnitId) -> i16 {
        self.state
            .items
            .get(&i)
            .and_then(|d| self.econ.tables.item(d.record))
            .map_or(-1, |r| r.type_)
    }

    fn set_row_of(&self, i: UnitId) -> Option<SetItemRow> {
        let it = self.econ.items.get(i)?;
        let si = usize::try_from(it.file_index)
            .ok()
            .and_then(|f| self.econ.tables.setitems.get(f))?;
        Some(SetItemRow {
            add_func: si.add_func,
            slot: si.slot as i16,
            set: si.set,
        })
    }

    fn owner_set_list(&self, o: UnitId, state: u32) -> Option<crate::stats::lists::ListId> {
        let r = self.econ.stats.unit_list(o)?;
        self.econ.stats.list_of_state(r, state)
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> EquipWorld for InvDesk<'_, '_, H, R> {
    fn unit_type(&self, u: UnitId) -> Option<u8> {
        self.econ.units.get(u).map(|r| r.ty as u8)
    }
    fn unit_flags(&self, u: UnitId) -> u32 {
        self.econ.units.get(u).map_or(0, |r| r.flags)
    }
    fn has_inventory(&self, u: UnitId) -> bool {
        self.state.inventories.contains_key(&u)
    }
    fn item_list(&self, u: UnitId) -> Vec<UnitId> {
        self.inv_items(u)
    }
    fn body_item(&self, u: UnitId, loc: u8) -> Option<UnitId> {
        self.state.inventories.get(&u)?.body_item(loc)
    }
    fn weapon_in_use(&self, u: UnitId) -> Option<UnitId> {
        // The weapon in use, written by the body link / unlink
        // (`items::inventory::weapon`).
        let inv = self.state.inventories.get(&u)?;
        self.item_unit(inv.weapon_guid)
    }
    fn add_unit_stat(&mut self, u: UnitId, stat: u16, d: i32) {
        self.econ
            .stats
            .unit_add(&mut *self.econ.hooks, u, stat, d, 0);
    }
    fn item_type(&self, i: UnitId) -> i16 {
        self.record_type(i)
    }
    fn item_is_type(&self, i: UnitId, t: i16) -> bool {
        self.state
            .items
            .get(&i)
            .is_some_and(|d| self.tables.is_type(d.record, t))
    }
    fn item_flags(&self, i: UnitId) -> u32 {
        self.state.items.get(&i).map_or(0, |d| d.flags)
    }
    fn set_item_flag(&mut self, i: UnitId, bits: u32, on: bool) {
        if let Some(d) = self.state.items.get_mut(&i) {
            if on {
                d.flags |= bits;
            } else {
                d.flags &= !bits;
            }
        }
    }
    fn item_mode(&self, i: UnitId) -> u8 {
        self.state.items.get(&i).map_or(0xFF, |d| d.mode)
    }
    fn item_node(&self, i: UnitId) -> u8 {
        self.state.items.get(&i).map_or(node::NONE, |d| d.node_kind)
    }
    fn item_page(&self, i: UnitId) -> u8 {
        self.state.items.get(&i).map_or(0xFF, |d| d.page)
    }
    fn item_quality(&self, i: UnitId) -> u8 {
        InvWorld::quality(self, i)
    }
    fn item_stat(&self, i: UnitId, stat: u16) -> i32 {
        InvWorld::item_stat(self, i, stat)
    }
    /// The `scrollskill` / `bookskill` column of the books row for the
    /// item's spell index ([`InvDesk::spell_of`]), from the desk's books
    /// table; no row gives none. A desk without the books table (tables
    /// built without it) asks the rest.
    fn book_skill(&self, i: UnitId, scroll: bool) -> Option<i32> {
        let spell = self.spell_of(i);
        if self.tables.books.is_empty() {
            return self.rest.book_skill(spell, scroll);
        }
        let b = usize::try_from(spell)
            .ok()
            .and_then(|k| self.tables.books.get(k))?;
        Some(if scroll { b.scrollskill } else { b.bookskill })
    }
    fn active_inventory_item(&self, u: UnitId, i: UnitId) -> bool {
        active_inventory_item(self, self.tables, i, u)
    }
    fn usable(&self, u: UnitId, i: UnitId) -> bool {
        self.state
            .inventories
            .get(&u)
            .is_some_and(|inv| usable(inv, self, self.tables, i))
    }
    fn stat_linked(&self, u: UnitId, i: UnitId) -> bool {
        InvWorld::item_active_on(self, i, u)
    }
    /// The weapon-in-use link, then the stat
    /// link.
    fn stat_link(&mut self, u: UnitId, i: UnitId) {
        self.weapon_link_on(u, i, true);
        InvWorld::stat_link(self, u, i)
    }
    /// The body unlink (weapon in use), then the stat
    /// unlink.
    fn stat_unlink(&mut self, u: UnitId, i: UnitId) {
        self.weapon_link_on(u, i, false);
        if self.unlink_item_stats(u, i) {
            return;
        }
        let (o, g) = (self.owner_or_none(u), self.guid_of(i));
        self.rest.stat_unlink(o, g)
    }
    /// The set update with r = 1, p = 0 for set items, then the rest's
    /// part.
    fn deactivate(&mut self, i: UnitId, u: UnitId) {
        if InvWorld::quality(self, i) == QUALITY_SET {
            set_state::set_item_update(self, Some(u), Some(i), 1, 0);
        }
        let o = self.owner_or_none(u);
        self.rest.stat_refresh_unlink(o, 1);
    }
    /// The set branch of the stat refresh
    /// on the rules; gems and runes (an item owner only) stay the rest's.
    fn stat_refresh(&mut self, i: UnitId, u: UnitId) {
        let gem_or_rune =
            self.item_is_type(i, ty::GEM as i16) || self.item_is_type(i, ty::RUNE as i16);
        if !set_state::stat_refresh_set(self, u, i, gem_or_rune) {
            let o = self.owner_or_none(u);
            self.rest.stat_refresh(o);
        }
    }
    fn owner_refresh(&mut self, u: UnitId) {
        InvWorld::owner_refresh(self, u)
    }
    fn send_unit_refresh(&mut self, u: UnitId) {
        let o = self.owner_or_none(u);
        self.rest.send_unit_refresh(o)
    }
    fn skill_quantity(&self, u: UnitId, skill: i32) -> Option<i32> {
        self.rest.skill_quantity(self.owner_or_none(u), skill)
    }
    fn set_skill_quantity(&mut self, u: UnitId, skill: i32, q: i32) {
        let o = self.owner_or_none(u);
        self.rest.set_skill_quantity(o, skill, q)
    }
    /// The skill-point spend: the cost is 1 (the `skpoints` column is
    /// empty in every row); base `newskills` (stat 5) below the cost does
    /// nothing; else stat 5 drops by 1 and the
    /// level is added (the rest's skill list). The item-skill link adds the
    /// quantity to stat 5 first, so a learned scroll skill nets 0.
    fn learn_skill(&mut self, u: UnitId, skill: i32) {
        const NEWSKILLS: u16 = 5;
        if self.econ.stats.unit_base(u, NEWSKILLS, 0) < 1 {
            return;
        }
        self.add_unit_stat(u, NEWSKILLS, -1);
        let o = self.owner_or_none(u);
        self.rest.learn_skill(o, skill)
    }
    /// S→C 0x22 to the unit's
    /// client through the rest's transport: the unit's type and GUID, the
    /// skill, the quantity's low byte, flag 1 when the unit has state 7 at
    /// send. A unit without a record sends nothing.
    fn send_skill_quantity(&mut self, u: UnitId, skill: i32, q: i32) {
        let Some(o) = self.owner_of(u) else {
            return;
        };
        let state7 = self.econ.stats.has_state(u, 7);
        let msg =
            crate::units::messages::update_item_skill(o.ty, o.guid, skill as u16, q as u8, state7);
        self.rest.send(o, msg.to_vec())
    }
    fn mouse_skill(&self, u: UnitId, left: bool) -> Option<SkillRef> {
        self.rest.mouse_skill(self.owner_or_none(u), left)
    }
    fn select_skill(&mut self, u: UnitId, left: bool, s: SkillRef) {
        let o = self.owner_or_none(u);
        self.rest.select_skill(o, left, s)
    }
    fn has_skill(&self, u: UnitId, s: SkillRef) -> bool {
        self.rest.has_skill_owned(self.owner_or_none(u), s)
    }
    /// The skill use-state check: the first four tests come from the rest,
    /// then the fifth on the desk's items: the skill item test
    /// ([`InvDesk::skill_item_ok`]) and the item type test
    /// (failure gives 2). Provisional: the scroll / book part of the
    /// skill item test and the later checks (mana, shape, start stat,
    /// charges, cooldown) are not run here; the mouse-skill bookkeeping
    /// only reads the codes 2 and 7.
    fn use_state(&mut self, u: UnitId, s: SkillRef) -> u8 {
        let o = self.owner_or_none(u);
        let code = self.rest.skill_use_state(o, s);
        if code != 0 {
            return code;
        }
        if !self.skill_item_ok(u, s.0) {
            return bk::USE_NO_QUANTITY;
        }
        match self.rest.skill_item_types(s.0) {
            Some(k) if !item_type_test(&DeskHands { d: self, u }, &k) => bk::USE_NO_QUANTITY,
            _ => 0,
        }
    }
    fn throw_skill_row(&self, skill: i32) -> bool {
        self.rest.throw_skill_row(skill)
    }
    fn saved_mouse_skill(&self, u: UnitId, left: bool) -> SkillRef {
        self.rest.saved_mouse_skill(self.owner_or_none(u), left)
    }
    fn set_saved_mouse_skill(&mut self, u: UnitId, left: bool, s: SkillRef) {
        let o = self.owner_or_none(u);
        self.rest.set_saved_mouse_skill(o, left, s)
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> SetWorld for InvDesk<'_, '_, H, R> {
    fn is_item(&self, u: UnitId) -> bool {
        self.econ
            .units
            .get(u)
            .is_some_and(|r| r.ty == UnitType::Item)
    }
    fn has_inventory(&self, u: UnitId) -> bool {
        self.state.inventories.contains_key(&u)
    }
    fn in_body_of(&self, o: UnitId, i: UnitId) -> bool {
        self.state
            .items
            .get(&i)
            .is_some_and(|d| d.inv == Some(o) && d.node_kind == node::BODY)
    }
    /// The set slots of `o`'s equipped (body node) set items of `i`'s
    /// set, `i` included, without the no-equip (0x4000) or broken (0x100)
    /// ones.
    fn set_mask(&self, o: UnitId, i: UnitId) -> u32 {
        let Some(set) = self.set_row_of(i).map(|r| r.set) else {
            return 0;
        };
        let mut mask = 0u32;
        let mut add = |x: UnitId| {
            if let Some(r) = self.set_row_of(x).filter(|r| r.set == set) {
                if let Some(bit) = 1u32.checked_shl(r.slot as u32) {
                    mask |= bit;
                }
            }
        };
        add(i);
        for x in self.inv_items(o) {
            if x == i {
                continue;
            }
            let Some(d) = self.state.items.get(&x) else {
                continue;
            };
            if d.node_kind == node::BODY
                && d.flags & (iflag::F4000 | iflag::BROKEN) == 0
                && InvWorld::quality(self, x) == QUALITY_SET
                && (body::HEAD..=body::GLOVES).contains(&d.body_loc)
            {
                add(x);
            }
        }
        mask
    }
    fn quality(&self, i: UnitId) -> u8 {
        InvWorld::quality(self, i)
    }
    fn setitem_row(&self, i: UnitId) -> Option<SetItemRow> {
        self.set_row_of(i)
    }
    fn sets_count(&self) -> usize {
        self.econ.tables.sets.len()
    }
    fn park(&mut self, i: UnitId, state: u32, park: bool) {
        self.econ.stats.park(&mut *self.econ.hooks, i, state, park);
    }
    fn owner_list_tag(&self, o: UnitId, state: u32) -> Option<i32> {
        let l = self.owner_set_list(o, state)?;
        Some(self.econ.stats.base(l, STAT_SET_ID, 0))
    }
    fn reset_owner_list(&mut self, o: UnitId, state: u32, set: i32) {
        let Some(l) = self.owner_set_list(o, state) else {
            return;
        };
        let st = &mut *self.econ.stats;
        st.remove_all(&mut *self.econ.hooks, l);
        st.set(&mut *self.econ.hooks, l, STAT_SET_ID, set, 0, None);
    }
    fn free_owner_list(&mut self, o: UnitId, state: u32) {
        let Some(l) = self.owner_set_list(o, state) else {
            return;
        };
        let st = &mut *self.econ.stats;
        st.unit_detach(&mut *self.econ.hooks, l);
        st.free_plain(&mut *self.econ.hooks, l);
    }
    fn new_owner_list(&mut self, o: UnitId, i: UnitId, state: u32) {
        let guid = self.econ.units.get(i).map_or(0, |r| r.guid);
        let st = &mut *self.econ.stats;
        let l = st.alloc(0, 0, OWNER_TYPE_ITEM, guid);
        st.attach(&mut *self.econ.hooks, o, l, true);
        st.set_state(l, state);
    }
    fn tag_owner_list(&mut self, o: UnitId, state: u32, set: i32) {
        if let Some(l) = self.owner_set_list(o, state) {
            self.econ
                .stats
                .set(&mut *self.econ.hooks, l, STAT_SET_ID, set, 0, None);
        }
    }
    /// The set bonuses: on the desk when item stats are linked, else the rest's.
    fn set_bonuses(&mut self, o: UnitId, i: UnitId, state: u32) {
        if self.state.link_item_stats {
            return self.apply_set_bonuses(o, i, state);
        }
        let (ow, g) = (self.owner_or_none(o), self.guid_of(i));
        self.rest.set_bonuses(ow, g, state)
    }
}

/// The inventory desk's answers to the item type test
/// of the skill use checks: the hands of `u` (body locations 4 / 5), the
/// item flags, the primary type's `shoots` and the
/// item stats.
struct DeskHands<'v, 'd, 'a, H, R: ?Sized> {
    d: &'v InvDesk<'d, 'a, H, R>,
    u: UnitId,
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> ItemTypeView for DeskHands<'_, '_, '_, H, R> {
    type Item = UnitId;
    fn hand_item(&self, loc: u8) -> Option<UnitId> {
        self.d.body_item(self.u, loc)
    }
    fn has_inventory(&self) -> bool {
        EquipWorld::has_inventory(self.d, self.u)
    }
    fn weapon_in_use(&self) -> Option<UnitId> {
        self.d.weapon_in_use(self.u)
    }
    fn is_a(&self, item: UnitId, itype: i32) -> bool {
        i16::try_from(itype).is_ok_and(|t| self.d.item_is_type(item, t))
    }
    fn ammo_free(&self, item: UnitId) -> bool {
        self.d.item_flags(item) & (iflag::F4000 | iflag::BROKEN) != 0
    }
    fn shoots(&self, item: UnitId) -> i32 {
        self.d.ammo_of(item).map_or(0, i32::from)
    }
    fn stat(&self, item: UnitId, stat: u16) -> i32 {
        InvWorld::item_stat(self.d, item, stat)
    }
    fn stackable(&self, item: UnitId) -> bool {
        self.d
            .state
            .items
            .get(&item)
            .and_then(|i| self.d.econ.tables.item(i.record))
            .is_some_and(|r| r.stackable != 0)
    }
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// Neither item flag 0x100 (broken) nor 0x4000 set; the identified
    /// flag is not tested.
    fn item_ok(&self, i: UnitId) -> bool {
        EquipWorld::item_flags(self, i) & (iflag::BROKEN | iflag::F4000) == 0
    }
    /// The weapon pick: the item
    /// at body location 4, else 5, that passes [`Self::item_ok`] and is of type
    /// 45 `weap`; with its location and whether it is the weapon in use.
    fn weapon_pick(&self, u: UnitId) -> Option<(UnitId, u8, bool)> {
        let in_use = EquipWorld::weapon_in_use(self, u);
        [4u8, 5].into_iter().find_map(|loc| {
            let i = EquipWorld::body_item(self, u, loc)?;
            (self.item_ok(i) && self.item_is_type(i, ty::WEAP as i16)).then_some((
                i,
                loc,
                Some(i) == in_use,
            ))
        })
    }
    /// Whether the item's own type has itemtypes `Throwable`.
    fn throwable(&self, i: UnitId) -> bool {
        self.state
            .items
            .get(&i)
            .and_then(|d| self.tables.itype_of(d.record))
            .is_some_and(|t| t.throwable != 0)
    }
    /// A throwable item, or else one with stat 0x7D.
    fn throw_item_ok(&self, i: UnitId) -> bool {
        self.throwable(i) || InvWorld::item_stat(self, i, 0x7D) != 0
    }
    /// The skill item test (fifth check of the skill use state): skill 2
    /// (Throw) needs an inventory; a unit that
    /// cannot dual-wield needs the weapon pick to be the weapon in use,
    /// one that can takes the pick or, when the pick is not the weapon in
    /// use, the item at the other location; that item
    /// throwable or with stat 0x7D. Skill 4 (Left Hand Throw): the pick
    /// when it is not the weapon in use, else the other location's item,
    /// throwable or with stat 0x7D. Skill 5 (Left Hand Swing): a
    /// dual-wielding unit whose pick (not in use) or other item is of type
    /// 45 `weap`. Every other skill passes.
    pub(super) fn skill_item_ok(&self, u: UnitId, skill: i32) -> bool {
        if !matches!(skill, 2 | 4 | 5) {
            return true;
        }
        if !EquipWorld::has_inventory(self, u) {
            return false;
        }
        // Dual wielding: a player of class 4 or 6, or monster class 417–418.
        let dual = self.econ.units.get(u).is_some_and(|r| match r.ty {
            UnitType::Player => matches!(r.class, 4 | 6),
            UnitType::Monster => matches!(r.class, 417 | 418),
            _ => false,
        });
        let other = |loc: u8| EquipWorld::body_item(self, u, if loc == 5 { 4 } else { 5 });
        let Some((pick, loc, in_use)) = self.weapon_pick(u) else {
            return false;
        };
        match skill {
            2 if !dual => in_use && self.throw_item_ok(pick),
            2 => {
                let i = if in_use { Some(pick) } else { other(loc) };
                i.is_some_and(|i| self.throw_item_ok(i))
            }
            4 => {
                let i = if in_use { other(loc) } else { Some(pick) };
                i.is_some_and(|i| self.throw_item_ok(i))
            }
            _ => {
                if !dual {
                    return false;
                }
                let i = if in_use { other(loc) } else { Some(pick) };
                i.is_some_and(|i| self.item_is_type(i, ty::WEAP as i16))
            }
        }
    }
}
