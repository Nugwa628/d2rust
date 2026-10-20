//! Horadric Cube world: [`CubeWorld`] on the real providers: game fields
//! (`GameFields`), unit records and stat lists, item creation and init,
//! socket rules, craft property lists and the item data
//! ([`super::ItemStore`]). [`EconomyCube`] is what the cube module runs
//! against; whatever the economy cannot supply stays a seam: [`CubeRest`].

use super::{Economy, EconomyError, ItemSpawn};
use crate::items::tables::PropRec;
use crate::items::{create, props};
use crate::rng::Seed;
use crate::units::lifecycle::LifecycleHooks;
use crate::units::{UnitId, UnitType};
use crate::world::cube::{CraftProperty, CubeWorld, ItemRequest, StatRead};

/// Stat 194 (`item_numsockets`).
const STAT_SOCKETS: u16 = 194;
/// The stash object class (0x10B).
const STASH_CLASS: u32 = 0x10B;

/// The cube's calls the economy cannot answer itself, each with its expected
/// provider noted on each method.
pub trait CubeRest {
    /// The player data item creation reads: name and the client's hardcore
    /// flag; `None` without player data (player data / client records: not in
    /// d2-sim).
    fn player_info(&self, player: UnitId) -> Option<([u8; 16], Option<bool>)>;
    /// The local calendar date (month, day): host input.
    fn local_date(&self) -> (u8, u8);
    /// Attaches a sound event to the player's unit.
    fn attach_sound(&mut self, player: UnitId, event: u8);
    /// Sends a message to the player's client (`d2-server` transport).
    fn send(&mut self, player: UnitId, msg: &[u8]);
    // Interaction (interaction / UI owner).
    fn inventory_pass(&mut self, player: UnitId);
    // Inventory.
    fn inventory(&self, player: UnitId) -> Vec<UnitId>;
    fn socketed(&self, item: UnitId) -> Vec<UnitId>;
    fn place(&mut self, player: UnitId, item: UnitId) -> bool;
    fn remove_cube_item(&mut self, player: UnitId, item: UnitId);
    fn targeting_reset(&mut self, player: UnitId);
    fn put_item_check(&self, player: UnitId, item: u32) -> u32;
    fn cube_check(&self, player: UnitId, cube: u32) -> bool;
    // Item routines the economy does not implement.
    /// Duplicates the item (with its fillers when `fillers`).
    fn duplicate(&mut self, item: UnitId, fillers: bool) -> Option<UnitId>;
    /// Picks the tempered affix (prefix when `prefix`, else suffix) for the
    /// item. TODO: the cube calls this once per side; what the routine does
    /// beyond that is not settled, so it is not wired.
    fn tempered_affix(&mut self, item: UnitId, prefix: bool) -> u16;
    /// Drops the item's runeword stats.
    fn drop_runeword_stats(&mut self, item: UnitId);
    /// Repairs the item.
    fn repair(&mut self, item: UnitId);
    /// Recharges the item.
    fn recharge(&mut self, item: UnitId);
    // Quests.
    fn quest_item_hook(&mut self, player: UnitId, item: UnitId, code: [u8; 4]);
    fn cow_portal(&mut self, player: UnitId) -> bool;
}

/// The cube's world: the economy plus the rest.
pub struct EconomyCube<'e, 'a, H, R> {
    /// The economy the cube runs on.
    pub econ: &'e mut Economy<'a, H>,
    /// The seams the economy cannot answer.
    pub rest: &'e mut R,
    /// Errors the [`CubeWorld`] signatures cannot return (fatal item
    /// errors, misuse), in order.
    pub errors: Vec<EconomyError>,
}

impl<'e, 'a, H, R> EconomyCube<'e, 'a, H, R> {
    /// A cube world over the economy and the rest.
    pub fn new(econ: &'e mut Economy<'a, H>, rest: &'e mut R) -> Self {
        Self {
            econ,
            rest,
            errors: Vec::new(),
        }
    }
}

impl<H: LifecycleHooks, R: CubeRest> EconomyCube<'_, '_, H, R> {
    fn item_field<T>(
        &self,
        item: UnitId,
        f: impl FnOnce(&crate::items::Item<()>) -> T,
    ) -> Option<T> {
        self.econ.items.get(item).map(f)
    }

    fn edit_item(&mut self, item: UnitId, f: impl FnOnce(&mut crate::items::Item<()>)) {
        match self.econ.items.get_mut(item) {
            Some(i) => f(i),
            None => self.errors.push(EconomyError::NotAnItem(item)),
        }
    }

    fn note<T>(&mut self, r: Result<T, EconomyError>) -> Option<T> {
        r.map_err(|e| self.errors.push(e)).ok()
    }
}

impl<H: LifecycleHooks, R: CubeRest> CubeWorld for EconomyCube<'_, '_, H, R> {
    /// The game's rules, from the unit hooks.
    fn rules(&self) -> &crate::rules::Ruleset {
        self.econ.hooks.rules()
    }
    fn expansion(&self) -> bool {
        self.econ.fields.expansion
    }
    fn game_type(&self) -> u8 {
        self.econ.fields.game_type
    }
    fn ladder(&self) -> bool {
        self.econ.fields.ladder
    }
    fn difficulty(&self) -> u8 {
        self.econ.fields.difficulty
    }
    fn item_format(&self) -> u16 {
        crate::items::ItemGame::item_format(&*self.econ.fields)
    }
    fn local_date(&self) -> (u8, u8) {
        self.rest.local_date()
    }
    fn game_seed(&mut self) -> &mut Seed {
        &mut self.econ.fields.seed
    }

    /// The unit's class; a missing unit reads class 0xFF, which
    /// no recipe class test accepts but "any".
    fn player_class(&self, player: UnitId) -> u8 {
        self.econ.units.get(player).map_or(0xFF, |r| r.class as u8)
    }
    fn stat(&self, unit: UnitId, read: StatRead, stat: u16) -> i32 {
        let s = &self.econ.stats;
        match read {
            StatRead::Value => s.unit_total(unit, stat, 0),
            StatRead::Base => s.unit_base(unit, stat, 0),
            StatRead::Bonus => s.unit_bonus(unit, stat, 0),
        }
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        let e = &mut *self.econ;
        e.stats.unit_set(e.hooks, unit, stat, value, 0);
    }
    fn attach_sound(&mut self, player: UnitId, event: u8) {
        self.rest.attach_sound(player, event)
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.rest.send(player, msg)
    }

    /// The player's interaction, from the unit record's
    /// [`crate::units::record::InteractInfo`]; no record gives none.
    fn interaction(&self, player: UnitId) -> Option<(u8, u32)> {
        self.econ.units.get(player)?.interact.get()
    }
    /// Sets the interaction; only takes effect while none is active.
    fn set_interaction(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        if let Some(r) = self.econ.units.get_mut(player) {
            r.interact.set(unit_type, guid);
        }
    }
    /// Resets the interaction: GUID −1, type 6, inactive.
    fn reset_interaction(&mut self, player: UnitId) {
        if let Some(r) = self.econ.units.get_mut(player) {
            r.interact.reset();
        }
    }
    fn inventory_pass(&mut self, player: UnitId) {
        self.rest.inventory_pass(player)
    }
    /// Whether the interaction is the stash object (type 2, object class
    /// 0x10B).
    fn interacting_with_stash(&self, player: UnitId) -> bool {
        self.interaction(player).is_some_and(|(ty, guid)| {
            ty == UnitType::Object as u8
                && self
                    .econ
                    .game
                    .lists
                    .find_unit(UnitType::Object, guid)
                    .and_then(|u| self.econ.units.get(u))
                    .is_some_and(|r| r.class == STASH_CLASS)
        })
    }
    /// Whether the interaction is with a player unit that exists.
    fn trading(&self, player: UnitId) -> bool {
        self.interaction(player).is_some_and(|(ty, guid)| {
            ty == UnitType::Player as u8
                && self
                    .econ
                    .game
                    .lists
                    .find_unit(UnitType::Player, guid)
                    .is_some()
        })
    }

    fn inventory(&self, player: UnitId) -> Vec<UnitId> {
        self.rest.inventory(player)
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.econ
            .game
            .lists
            .find_unit(UnitType::Item, guid)
            .filter(|&u| self.econ.items.contains(u))
    }
    fn item_guid(&self, item: UnitId) -> u32 {
        self.econ.units.get(item).map_or(0, |r| r.guid)
    }
    fn item_page(&self, item: UnitId) -> u8 {
        self.item_field(item, |i| i.inv_page).unwrap_or(0xFF)
    }
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        self.edit_item(item, |i| i.inv_page = page);
    }
    /// The unit mode.
    fn item_mode(&self, item: UnitId) -> u8 {
        self.econ.units.get(item).map_or(0, |r| r.mode as u8)
    }
    fn set_item_mode(&mut self, item: UnitId, mode: u8) {
        match self.econ.units.get_mut(item) {
            Some(r) => r.mode = u32::from(mode),
            None => self.errors.push(EconomyError::NoRecord(item)),
        }
    }
    fn item_class(&self, item: UnitId) -> Option<u32> {
        self.item_field(item, |i| i.record)
            .filter(|&r| self.econ.tables.item(r).is_some())
            .map(|r| r as u32)
    }
    /// The class lives in the item data (record) and the unit record
    /// field; both are written.
    fn set_item_class(&mut self, item: UnitId, class: u32) {
        self.edit_item(item, |i| i.record = class as usize);
        if let Some(r) = self.econ.units.get_mut(item) {
            r.class = class;
        }
    }
    fn class_is_type(&self, class: u32, ty: u16) -> bool {
        self.econ.tables.is_type(class as usize, ty as i16)
    }
    fn item_quality(&self, item: UnitId) -> u8 {
        self.item_field(item, |i| i.quality).unwrap_or(2)
    }
    fn item_file_index(&self, item: UnitId) -> u32 {
        self.item_field(item, |i| i.file_index as u32)
            .unwrap_or(u32::MAX)
    }
    fn item_level(&self, item: UnitId) -> i32 {
        self.item_field(item, |i| i.ilvl).unwrap_or(0)
    }
    fn set_item_level(&mut self, item: UnitId, level: i32) {
        self.edit_item(item, |i| i.ilvl = level);
    }
    fn item_flags(&self, item: UnitId) -> u32 {
        self.item_field(item, |i| i.flags).unwrap_or(0)
    }
    fn set_item_flag(&mut self, item: UnitId, flag: u32) {
        self.edit_item(item, |i| i.flags |= flag);
    }
    fn item_sockets(&self, item: UnitId) -> i32 {
        self.econ.stats.unit_total(item, STAT_SOCKETS, 0)
    }
    /// The most sockets the item can have, from its record and item level.
    fn max_sockets(&self, item: UnitId) -> i32 {
        let Some(i) = self.econ.items.get(item) else {
            return 0;
        };
        // Only the record and the item level matter here, so no stats.
        let (probe, ()) = super::item_units::swap_stats(i.clone(), NoStats);
        create::max_sockets(self.econ.tables, &probe)
    }
    /// Adds `n` sockets to the item.
    fn add_sockets(&mut self, item: UnitId, n: i32) {
        let r = self
            .econ
            .with_item(item, |s| create::socket_count(s.tables, s.item, n));
        self.note(r);
    }
    /// The item's unit seed in its unit record.
    ///
    /// Panics when the unit has no record: the cube asks only for items
    /// it holds.
    fn item_seed(&mut self, item: UnitId) -> &mut Seed {
        &mut self
            .econ
            .units
            .get_mut(item)
            .expect("cube item without a unit record")
            .seed
    }
    fn socketed(&self, item: UnitId) -> Vec<UnitId> {
        self.rest.socketed(item)
    }
    fn duplicate(&mut self, item: UnitId, fillers: bool) -> Option<UnitId> {
        self.rest.duplicate(item, fillers)
    }
    /// Runs item stat initialisation with no request and not as a quest
    /// item: the item when it succeeds.
    fn item_init(&mut self, item: UnitId) -> Option<UnitId> {
        let r = self.econ.with_item(item, |s| {
            create::init_item_stats(s.tables, s.game, s.item, None, false)
        });
        match self.note(r)? {
            Ok(true) => Some(item),
            Ok(false) => None,
            Err(f) => {
                self.errors.push(f.into());
                None
            }
        }
    }
    /// Creates an item from the cube's request, converted to an items
    /// request. The source unit is the player: class and stat 12 from d2-sim,
    /// player data from the rest.
    fn create_item(&mut self, request: &ItemRequest) -> Option<UnitId> {
        let unit = request.player.and_then(|p| {
            let info = self.rest.player_info(p);
            self.econ.request_unit(p, info)
        });
        let widen = |a: [u16; 3]| a.map(i32::from);
        let mut rq = crate::items::ItemRequest {
            unit,
            ilvl: request.level,
            item: request.class as i32,
            format: request.item_format,
            quality: request.quality,
            index: i32::from(request.item_index),
            prefix: widen(request.prefix),
            suffix: widen(request.suffix),
            flags2: request.flags2,
            ..Default::default()
        };
        let spawn = ItemSpawn {
            room: None,
            mode: u32::from(request.spawn_type),
            init_flags: u32::from(request.init_flags),
        };
        match self.econ.create_item(&mut rq, false, spawn) {
            Ok(u) => Some(u),
            Err(EconomyError::Create(create::CreateError::Fatal(f))) => {
                self.errors.push(f.into());
                None
            }
            Err(EconomyError::Create(_)) => None,
            Err(e) => {
                self.errors.push(e);
                None
            }
        }
    }
    fn tempered_affix(&mut self, item: UnitId, prefix: bool) -> u16 {
        self.rest.tempered_affix(item, prefix)
    }
    /// Quality 9 (tempered) with the rare prefix and suffix slots set.
    fn set_tempered(&mut self, item: UnitId, prefix: u16, suffix: u16) {
        self.edit_item(item, |i| {
            i.quality = crate::items::q::TEMPERED;
            i.rare_prefix = prefix;
            i.rare_suffix = suffix;
        });
    }
    fn unique_found(&self, index: u32) -> bool {
        self.econ.fields.uniques.get(index)
    }
    /// Indices above 4096 have no bit: nothing happens.
    fn set_unique_found(&mut self, index: u32, found: bool) {
        if index > crate::items::UniqueBits::MAX {
            return;
        }
        let w = &mut self.econ.fields.uniques.0[(index >> 5) as usize];
        if found {
            *w |= 1 << (index & 31);
        } else {
            *w &= !(1 << (index & 31));
        }
    }
    fn drop_runeword_stats(&mut self, item: UnitId) {
        self.rest.drop_runeword_stats(item)
    }
    /// Applies the craft property list (here one record) to the item.
    ///
    /// TODO: the craft-list routine also takes an expansion argument whose
    /// meaning is not settled; it is not passed.
    fn add_craft_property(&mut self, item: UnitId, prop: &CraftProperty) {
        let rec = PropRec {
            code: prop.property,
            param: prop.param,
            min: prop.min,
            max: prop.max,
        };
        let r = self
            .econ
            .with_item(item, |s| props::apply_craft_list(s.tables, s.item, &[rec]));
        self.note(r);
    }
    fn repair(&mut self, item: UnitId) {
        self.rest.repair(item)
    }
    fn recharge(&mut self, item: UnitId) {
        self.rest.recharge(item)
    }
    fn place(&mut self, player: UnitId, item: UnitId) -> bool {
        self.rest.place(player, item)
    }
    /// Frees the item unit and its item data.
    fn free_item(&mut self, item: UnitId) {
        let r = self.econ.free_item(item);
        self.note(r);
    }
    fn remove_cube_item(&mut self, player: UnitId, item: UnitId) {
        self.rest.remove_cube_item(player, item)
    }
    fn targeting_reset(&mut self, player: UnitId) {
        self.rest.targeting_reset(player)
    }
    fn put_item_check(&self, player: UnitId, item: u32) -> u32 {
        self.rest.put_item_check(player, item)
    }
    fn cube_check(&self, player: UnitId, cube: u32) -> bool {
        self.rest.cube_check(player, cube)
    }
    fn quest_item_hook(&mut self, player: UnitId, item: UnitId, code: [u8; 4]) {
        self.rest.quest_item_hook(player, item, code)
    }
    fn cow_portal(&mut self, player: UnitId) -> bool {
        self.rest.cow_portal(player)
    }
}

/// A stats holder for item rules that read no stats.
struct NoStats;

impl crate::items::ItemStats for NoStats {
    fn has_stats(&self) -> bool {
        false
    }
    fn stat(&self, _: u16, _: u16) -> i32 {
        0
    }
    fn base(&self, _: u16, _: u16) -> i32 {
        0
    }
    fn set_base(&mut self, _: u16, _: u16, _: i32) {}
    fn has_list(&self, _: crate::items::ListKey) -> bool {
        false
    }
    fn list_set(&mut self, _: crate::items::ListKey, _: u16, _: u16, _: i32) {}
    fn list_add(&mut self, _: crate::items::ListKey, _: u16, _: u16, _: i32) {}
    fn list_get(&self, _: crate::items::ListKey, _: u16, _: u16) -> i32 {
        0
    }
}
