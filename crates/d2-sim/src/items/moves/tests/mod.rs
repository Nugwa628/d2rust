//! Tests of `items::moves` on a fake world implementing the three seams.

mod answers;
mod deferred;
mod gaps;
mod ground;
mod handlers;
mod tsv;

#[path = "../mutant_tests.rs"]
mod mutant_tests;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use super::seams::{InventoryOps, MovePending, MoveUnits, Spot};
use super::{mode, page, Guid, Owner};
use crate::units::RoomId;

pub const P: Guid = 1;
pub fn me() -> Owner {
    Owner::player(P)
}

#[derive(Clone, Debug, Default)]
pub struct FItem {
    pub mode: u8,
    pub page: u8,
    pub stored_page: u8,
    pub body_loc: u8,
    pub cmd: u32,
    pub iflags: u32,
    pub owner: Option<Owner>,
    pub types: Vec<u16>,
    pub code: [u8; 4],
    pub quality: u8,
    pub file_index: i32,
    pub quest: u8,
    pub useable: bool,
    pub component: u8,
    pub max_stack: i32,
    pub filled: bool,
    pub filler: bool,
    pub sockets: i32,
    pub fillers: Vec<Guid>,
    pub spell: i32,
    pub expiry: i32,
    pub two_handed: bool,
    pub beltable: bool,
    pub carry_one: bool,
    pub stackable: bool,
    pub autostack: bool,
    pub quiver: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FUnit {
    pub class: u32,
    pub x: i32,
    pub y: i32,
    pub uflags: u32,
    pub c8: u32,
    pub mode: u32,
    pub stats: BTreeMap<u16, i32>,
}

#[derive(Clone, Debug, Default)]
pub struct FInv {
    pub list: Vec<Guid>,
    pub cursor: Option<Guid>,
    pub update: Vec<Guid>,
    pub body: BTreeMap<u8, Guid>,
    pub belt: BTreeMap<u8, Guid>,
    pub weapon: Option<Guid>,
}

/// Configurable results of the inventory operations and the pending seams.
#[derive(Clone, Debug)]
pub struct Knobs {
    pub busy: bool,
    pub trading: bool,
    pub gate: bool,
    pub in_town: bool,
    pub place_ok: bool,
    pub link_ok: bool,
    pub fail_body_place: bool,
    pub fail_link_kind: Option<u8>,
    pub fail_unlink: bool,
    pub free: Option<(i32, i32)>,
    pub belt_slot: Option<u8>,
    pub belt_gate: bool,
    pub equip_check: u8,
    pub requirements: bool,
    pub stack: bool,
    pub auto_equip: Option<u8>,
    pub equip_from_cursor: (bool, bool),
    pub to_remove: Option<Guid>,
    pub distance: i32,
    pub collides: bool,
    pub room_at: bool,
    pub spot: Option<Spot>,
    pub in_room: bool,
    pub quest_flags: BTreeSet<(u8, u8)>,
    pub pile_owner: Option<Owner>,
    pub merge: bool,
    pub consume: bool,
    pub use_ok: bool,
    pub equip_picked: bool,
    pub hireling: Option<Owner>,
    pub alive: bool,
    pub owns: bool,
    pub q44: bool,
    pub bits: Vec<u8>,
    /// `numboxes` of any belt.
    pub boxes: u8,
    /// Item skill of a scroll / tome.
    pub item_skill: i32,
    pub has_skill: bool,
    pub used_skill: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            busy: false,
            trading: false,
            gate: true,
            in_town: true,
            place_ok: true,
            link_ok: true,
            fail_body_place: false,
            fail_link_kind: None,
            fail_unlink: false,
            free: Some((0, 0)),
            belt_slot: None,
            belt_gate: true,
            equip_check: 1,
            requirements: true,
            stack: true,
            auto_equip: None,
            equip_from_cursor: (true, false),
            to_remove: None,
            distance: 1,
            collides: false,
            room_at: false,
            spot: Some(Spot {
                room: RoomId(7),
                x: 10,
                y: 20,
            }),
            in_room: true,
            quest_flags: BTreeSet::new(),
            pile_owner: None,
            merge: true,
            consume: false,
            use_ok: true,
            equip_picked: true,
            hireling: None,
            alive: true,
            owns: true,
            q44: false,
            bits: Vec::new(),
            boxes: 16,
            item_skill: -1,
            has_skill: false,
            used_skill: false,
        }
    }
}

/// A free-spot call: (start, origin, last).
pub type SpotCall = ((i32, i32), (i32, i32), u32);

#[derive(Clone, Debug, Default)]
pub struct Fake {
    pub items: BTreeMap<Guid, FItem>,
    pub units: BTreeMap<Owner, FUnit>,
    pub invs: BTreeMap<Owner, FInv>,
    pub expansion: bool,
    pub frame: i32,
    pub k: Knobs,
    /// Seam calls in order.
    pub log: Vec<String>,
    /// Messages queued "now" (direct sends).
    pub sent: Vec<Vec<u8>>,
    pub next_guid: Guid,
    /// Free-spot calls: (start, origin, last).
    pub spot_calls: RefCell<Vec<SpotCall>>,
}

impl Fake {
    /// A player (GUID 1, class 1, level 1) with an inventory.
    pub fn new() -> Self {
        let mut f = Fake {
            expansion: true,
            frame: 1000,
            next_guid: 500,
            ..Default::default()
        };
        let mut u = FUnit {
            class: 1,
            x: 100,
            y: 100,
            ..Default::default()
        };
        u.stats.insert(12, 1);
        f.units.insert(me(), u);
        f.invs.insert(me(), FInv::default());
        f
    }

    /// Adds an item in `m` mode, linked into the player's inventory unless
    /// on the ground; mode 4 also makes it the cursor item.
    pub fn item(&mut self, g: Guid, m: u8) -> &mut FItem {
        self.units.insert(Owner::item(g), FUnit::default());
        let it = FItem {
            mode: m,
            page: if m == mode::STORED { 0 } else { page::NONE },
            file_index: -1,
            max_stack: 1,
            code: *b"xxx ",
            ..Default::default()
        };
        if m != mode::GROUND {
            let inv = self.invs.get_mut(&me()).unwrap();
            if m == mode::CURSOR {
                inv.cursor = Some(g);
            } else {
                inv.list.push(g);
            }
        }
        self.items.insert(g, it);
        let it = self.items.get_mut(&g).unwrap();
        if m != mode::GROUND {
            it.owner = Some(me());
        }
        it
    }

    pub fn it(&self, g: Guid) -> &FItem {
        &self.items[&g]
    }
    pub fn inv(&self) -> &FInv {
        &self.invs[&me()]
    }
    pub fn inv_mut(&mut self) -> &mut FInv {
        self.invs.get_mut(&me()).unwrap()
    }
    pub fn unit(&mut self, u: Owner) -> &mut FUnit {
        self.units.entry(u).or_default()
    }
    pub fn logged(&self, s: &str) -> bool {
        self.log.iter().any(|l| l == s)
    }
    fn note(&mut self, s: String) {
        self.log.push(s);
    }
    fn owns(&self, p: Owner, g: Guid) -> bool {
        self.invs
            .get(&p)
            .is_some_and(|i| i.list.contains(&g) || i.cursor == Some(g))
    }
    fn exists(&self, g: Guid) -> bool {
        self.items.contains_key(&g)
    }
}

impl InventoryOps for Fake {
    type EquipmentCheckpoint = (
        BTreeMap<Guid, FItem>,
        BTreeMap<Owner, FUnit>,
        BTreeMap<Owner, FInv>,
    );
    fn equipment_checkpoint(&self, _owner: Owner) -> Self::EquipmentCheckpoint {
        (self.items.clone(), self.units.clone(), self.invs.clone())
    }
    fn restore_equipment(&mut self, _owner: Owner, saved: Self::EquipmentCheckpoint) {
        (self.items, self.units, self.invs) = saved;
    }

    fn has_inventory(&self, owner: Owner) -> bool {
        self.invs.contains_key(&owner)
    }
    fn cursor(&self, owner: Owner) -> Option<Guid> {
        self.invs.get(&owner).and_then(|i| i.cursor)
    }
    fn set_cursor(&mut self, owner: Owner, item: Option<Guid>) {
        self.invs.entry(owner).or_default().cursor = item;
    }
    fn items(&self, owner: Owner) -> Vec<Guid> {
        self.invs
            .get(&owner)
            .map(|i| i.list.clone())
            .unwrap_or_default()
    }
    fn unlink(&mut self, owner: Owner, item: Guid) -> bool {
        self.note(format!("unlink {item}"));
        if self.k.fail_unlink {
            return false;
        }
        let Some(inv) = self.invs.get_mut(&owner) else {
            return false;
        };
        let had = inv.list.contains(&item) || inv.cursor == Some(item);
        inv.list.retain(|&g| g != item);
        if inv.cursor == Some(item) {
            inv.cursor = None;
        }
        inv.body.retain(|_, g| *g != item);
        inv.belt.retain(|_, g| *g != item);
        had
    }
    fn update_list(&self, owner: Owner) -> Vec<Guid> {
        self.invs
            .get(&owner)
            .map(|i| i.update.clone())
            .unwrap_or_default()
    }
    fn update_list_add(&mut self, owner: Owner, item: Guid) {
        let inv = self.invs.entry(owner).or_default();
        if !inv.update.contains(&item) {
            inv.update.push(item);
        }
    }
    fn update_list_free(&mut self, owner: Owner) {
        if let Some(inv) = self.invs.get_mut(&owner) {
            inv.update.clear();
        }
    }
    fn weapon_in_use(&self, owner: Owner) -> Option<Guid> {
        self.invs.get(&owner).and_then(|i| i.weapon)
    }
    fn place_at(&mut self, owner: Owner, item: Guid, pg: u8, x: i32, y: i32) -> bool {
        self.note(format!("place_at {item} p{pg} {x},{y}"));
        if !self.k.place_ok {
            return false;
        }
        let inv = self.invs.entry(owner).or_default();
        inv.list.retain(|&g| g != item);
        inv.list.push(item);
        let it = self.items.get_mut(&item).unwrap();
        it.page = pg;
        it.owner = Some(owner);
        let u = self.units.entry(Owner::item(item)).or_default();
        u.x = x;
        u.y = y;
        true
    }
    fn find_free(&self, _owner: Owner, _item: Guid, _page: u8) -> Option<(i32, i32)> {
        self.k.free
    }
    fn place_in_page(
        &mut self,
        owner: Owner,
        item: Guid,
        x: i32,
        y: i32,
        find: bool,
        send: bool,
    ) -> bool {
        self.note(format!(
            "place_in_page {item} {x},{y} find={find} send={send}"
        ));
        if !self.k.place_ok {
            return false;
        }
        let inv = self.invs.entry(owner).or_default();
        inv.list.retain(|&g| g != item);
        inv.list.push(item);
        if inv.cursor == Some(item) {
            inv.cursor = None;
        }
        if send && !inv.update.contains(&item) {
            inv.update.push(item);
        }
        let it = self.items.get_mut(&item).unwrap();
        it.mode = mode::STORED;
        it.owner = Some(owner);
        if send {
            it.cmd |= 0x2;
        }
        true
    }
    fn link_check(&mut self, _owner: Owner, item: Guid, kind: u8) -> bool {
        self.note(format!("link {item} kind {kind}"));
        self.k.link_ok && self.k.fail_link_kind != Some(kind)
    }
    fn link_into_item(&mut self, target: Guid, filler: Guid) -> bool {
        if !self.k.link_ok {
            return false;
        }
        self.items.get_mut(&target).unwrap().fillers.push(filler);
        true
    }
    fn beltable(&self, item: Guid) -> bool {
        self.items.get(&item).is_some_and(|i| i.beltable)
    }
    fn auto_belt_gate(&self, _owner: Owner, _item: Guid) -> bool {
        self.k.belt_gate
    }
    fn belt_free_slot(&self, _owner: Owner, _item: Guid) -> Option<u8> {
        self.k.belt_slot
    }
    fn belt_place(&mut self, owner: Owner, item: Guid, slot: u32) -> bool {
        self.note(format!("belt_place {item} {slot}"));
        if !self.k.place_ok {
            return false;
        }
        let inv = self.invs.entry(owner).or_default();
        inv.list.retain(|&g| g != item);
        inv.list.push(item);
        inv.belt.insert(slot as u8, item);
        let u = self.units.entry(Owner::item(item)).or_default();
        u.x = slot as i32;
        u.y = 0;
        true
    }
    fn belt_item(&self, owner: Owner, slot: u8) -> Option<Guid> {
        self.invs
            .get(&owner)
            .and_then(|i| i.belt.get(&slot).copied())
    }
    fn belt_boxes(&self, _belt: Option<Guid>) -> u8 {
        self.k.boxes
    }
    fn page_items(&self, owner: Owner, pg: u8) -> Vec<Guid> {
        self.items(owner)
            .into_iter()
            .filter(|g| self.it(*g).page == pg && self.it(*g).mode == mode::STORED)
            .collect()
    }
    fn body_items(&self, owner: Owner) -> Vec<Guid> {
        self.invs
            .get(&owner)
            .map(|i| i.body.values().copied().collect())
            .unwrap_or_default()
    }
    fn belt_compact(&mut self, _owner: Owner, slot: u8) {
        self.note(format!("compact {slot}"));
    }
    fn body_item(&self, owner: Owner, loc: u8) -> Option<Guid> {
        self.invs
            .get(&owner)
            .and_then(|i| i.body.get(&loc).copied())
    }
    fn place_body(&mut self, owner: Owner, item: Guid, loc: u8) -> bool {
        self.note(format!("place_body {item} {loc}"));
        if !self.k.place_ok || self.k.fail_body_place {
            return false;
        }
        let inv = self.invs.entry(owner).or_default();
        inv.list.retain(|&g| g != item);
        inv.list.push(item);
        inv.body.insert(loc, item);
        true
    }
    fn clear_body_slot(&mut self, owner: Owner, loc: u8) {
        self.note(format!("clear_slot {loc}"));
        self.invs.entry(owner).or_default().body.remove(&loc);
    }
    fn item_to_remove(&self, owner: Owner, loc: u8) -> Option<Guid> {
        self.k.to_remove.or_else(|| self.body_item(owner, loc))
    }
    fn two_handed(&self, item: Guid) -> bool {
        self.items.get(&item).is_some_and(|i| i.two_handed)
    }
    fn requirements(&self, _item: Guid, _unit: Owner, _equipping: bool) -> bool {
        self.k.requirements
    }
    fn equip_check(&self, _unit: Owner, _loc: u8, _item: Option<Guid>, _skip: bool) -> u8 {
        self.k.equip_check
    }
    fn stack_test(&self, _a: Guid, _b: Guid) -> bool {
        self.k.stack
    }
    fn equip_from_cursor(&mut self, _p: Owner, item: Guid, loc: u8, skip: bool) -> (bool, bool) {
        self.note(format!("equip_from_cursor {item} {loc} {skip}"));
        self.k.equip_from_cursor
    }
    fn auto_equip(&self, _unit: Owner, _item: Guid, _skip: bool) -> Option<u8> {
        self.k.auto_equip
    }
    fn check_cursor_item(&self, p: Owner, item: Guid) -> u32 {
        let ok =
            self.exists(item) && self.it(item).mode == mode::CURSOR && self.cursor(p) == Some(item);
        u32::from(!ok)
    }
    fn check_stored(&self, p: Owner, item: Guid) -> u32 {
        let ok = self.exists(item) && self.it(item).mode == mode::STORED && self.owns(p, item);
        u32::from(!ok)
    }
    fn check_stored_or_equipped(&self, p: Owner, item: Guid) -> u32 {
        let bad = self.exists(item) && self.it(item).mode <= 1 && !self.owns(p, item);
        u32::from(bad)
    }
    fn check_owned(&self, p: Owner, item: Guid) -> u32 {
        u32::from(!(self.exists(item) && self.owns(p, item)))
    }
    fn check_belt(&self, p: Owner, item: Guid) -> u32 {
        let bad = self.exists(item) && self.it(item).mode == mode::BELT && !self.owns(p, item);
        u32::from(bad)
    }
    fn check_ground_or_owned(&self, p: Owner, item: Guid) -> u32 {
        if self.exists(item) && self.it(item).mode == mode::GROUND {
            0
        } else {
            self.check_owned(p, item)
        }
    }
    fn busy(&self, p: Owner) -> bool {
        self.k.busy || self.cursor(p).is_some()
    }
    fn trading(&self, _p: Owner) -> bool {
        self.k.trading
    }
    fn targeting_reset(&mut self, _p: Owner) {
        self.note("targeting_reset".into());
    }
    fn item_move_gate(&mut self, _p: Owner, _item: Option<Guid>) -> bool {
        self.k.gate
    }
}

impl MoveUnits for Fake {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn unit_exists(&self, u: Owner) -> bool {
        if u.ty == Owner::ITEM {
            self.exists(u.guid)
        } else {
            self.units.contains_key(&u)
        }
    }
    fn unit_class(&self, u: Owner) -> u32 {
        self.units.get(&u).map_or(0, |x| x.class)
    }
    fn unit_mode(&self, u: Owner) -> u32 {
        self.units.get(&u).map_or(0, |x| x.mode)
    }
    fn pos(&self, u: Owner) -> (i32, i32) {
        self.units.get(&u).map_or((0, 0), |x| (x.x, x.y))
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        let e = self.units.entry(u).or_default();
        e.x = x;
        e.y = y;
    }
    fn unit_flags(&self, u: Owner) -> u32 {
        self.units.get(&u).map_or(0, |x| x.uflags)
    }
    fn set_unit_flags(&mut self, u: Owner, v: u32) {
        self.units.entry(u).or_default().uflags = v;
    }
    fn update_bits(&self, u: Owner) -> u32 {
        self.units.get(&u).map_or(0, |x| x.c8)
    }
    fn set_update_bits(&mut self, u: Owner, v: u32) {
        self.units.entry(u).or_default().c8 = v;
    }
    fn stat(&self, u: Owner, id: u16) -> i32 {
        self.units
            .get(&u)
            .and_then(|x| x.stats.get(&id).copied())
            .unwrap_or(0)
    }
    fn set_stat(&mut self, u: Owner, id: u16, v: i32) {
        self.units.entry(u).or_default().stats.insert(id, v);
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn mode(&self, item: Guid) -> u8 {
        self.it(item).mode
    }
    fn set_mode(&mut self, item: Guid, m: u8) {
        self.items.get_mut(&item).unwrap().mode = m;
    }
    fn page(&self, item: Guid) -> u8 {
        self.it(item).page
    }
    fn set_page(&mut self, item: Guid, p: u8) {
        self.items.get_mut(&item).unwrap().page = p;
    }
    fn stored_page(&self, item: Guid) -> u8 {
        self.it(item).stored_page
    }
    fn set_stored_page(&mut self, item: Guid, p: u8) {
        self.items.get_mut(&item).unwrap().stored_page = p;
    }
    fn body_loc(&self, item: Guid) -> u8 {
        self.it(item).body_loc
    }
    fn set_body_loc(&mut self, item: Guid, loc: u8) {
        self.items.get_mut(&item).unwrap().body_loc = loc;
    }
    fn cmd_flags(&self, item: Guid) -> u32 {
        self.it(item).cmd
    }
    fn set_cmd_flags(&mut self, item: Guid, v: u32) {
        self.items.get_mut(&item).unwrap().cmd = v;
    }
    fn item_flags(&self, item: Guid) -> u32 {
        self.it(item).iflags
    }
    fn set_item_flags(&mut self, item: Guid, v: u32) {
        self.items.get_mut(&item).unwrap().iflags = v;
    }
    fn set_expiry(&mut self, item: Guid, frame: i32) {
        self.items.get_mut(&item).unwrap().expiry = frame;
    }
    fn item_owner(&self, item: Guid) -> Option<Owner> {
        self.it(item).owner
    }
    fn is_type(&self, item: Guid, ty: u16) -> bool {
        self.it(item).types.contains(&ty)
    }
    /// The first listed type is the primary one.
    fn primary_type(&self, item: Guid) -> u16 {
        self.it(item).types.first().copied().unwrap_or(0)
    }
    fn stackable(&self, item: Guid) -> bool {
        self.it(item).stackable
    }
    fn autostack(&self, item: Guid) -> bool {
        self.it(item).autostack
    }
    fn quiver(&self, item: Guid) -> bool {
        self.it(item).quiver
    }
    fn code(&self, item: Guid) -> [u8; 4] {
        self.it(item).code
    }
    fn quality(&self, item: Guid) -> u8 {
        self.it(item).quality
    }
    fn file_index(&self, item: Guid) -> i32 {
        self.it(item).file_index
    }
    fn quest(&self, item: Guid) -> u8 {
        self.it(item).quest
    }
    fn useable(&self, item: Guid) -> bool {
        self.it(item).useable
    }
    fn component(&self, item: Guid) -> u8 {
        self.it(item).component
    }
    fn max_stack(&self, item: Guid) -> i32 {
        self.it(item).max_stack
    }
    fn socket_filled(&self, item: Guid) -> bool {
        self.it(item).filled
    }
    fn socket_filler(&self, item: Guid) -> bool {
        self.it(item).filler
    }
    fn sockets(&self, item: Guid) -> i32 {
        self.it(item).sockets
    }
    fn fillers(&self, item: Guid) -> Vec<Guid> {
        self.it(item).fillers.clone()
    }
    fn spell(&self, item: Guid) -> i32 {
        self.it(item).spell
    }
}

impl MovePending for Fake {
    fn distance(&self, _a: Owner, _b: Owner) -> i32 {
        self.k.distance
    }
    fn collides(&self, _a: Owner, _b: Owner, mask: u32) -> bool {
        assert_eq!(mask, 0x804);
        self.k.collides
    }
    fn walk_to_item(&mut self, _p: Owner, item: Guid, cursor: bool) {
        self.note(format!("walk {item} {cursor}"));
    }
    fn room_at(&self, _x: i32, _y: i32) -> bool {
        self.k.room_at
    }
    fn free_spot(
        &self,
        start: (i32, i32),
        origin: (i32, i32),
        size: u32,
        m: u32,
        m2: u32,
        last: u32,
    ) -> Option<Spot> {
        assert_eq!((size, m, m2), (1, 0x3E01, 0x801));
        self.spot_calls.borrow_mut().push((start, origin, last));
        self.k.spot
    }
    fn in_town(&self, _p: Owner) -> bool {
        self.k.in_town
    }
    fn room_delete_notice(&mut self, item: Guid) {
        self.note(format!("room_delete {item}"));
    }
    fn free_collision(&mut self, item: Guid) {
        self.note(format!("free_collision {item}"));
    }
    fn remove_from_room(&mut self, item: Guid) {
        self.note(format!("remove_from_room {item}"));
    }
    fn add_to_room(&mut self, item: Guid, spot: Spot) {
        self.note(format!("add_to_room {item} {}", spot.room.0));
    }
    fn in_room(&self, _item: Guid) -> bool {
        self.k.in_room
    }
    fn room_change_notice(&mut self, item: Guid, x: i32, y: i32) {
        self.note(format!("room_change {item} {x},{y}"));
    }
    fn queue_update(&mut self, u: Owner) {
        self.note(format!("queue_update {}:{}", u.ty, u.guid));
    }
    fn stat_refresh(&mut self, _u: Owner) {
        self.note("stat_refresh".into());
    }
    fn stat_refresh_unlink(&mut self, u: Owner, b: u32) {
        self.note(format!("stat_refresh_unlink {}:{} {b}", u.ty, u.guid));
    }
    fn stat_link(&mut self, _owner: Owner, item: Guid) {
        self.note(format!("stat_link {item}"));
    }
    fn charm_relink(&mut self, _owner: Owner, item: Guid) {
        self.note(format!("charm_relink {item}"));
    }
    fn charm_unlink(&mut self, _owner: Owner, item: Guid) {
        self.note(format!("charm_unlink {item}"));
    }
    fn inventory_pass(&mut self, _owner: Owner) {
        self.note("inventory_pass".into());
    }
    fn sound(&mut self, _u: Owner, id: u32) {
        self.note(format!("sound {id:#x}"));
    }
    fn pickup_sound(&mut self, _p: Owner, _item: Guid) {
        self.note("pickup_sound".into());
    }
    fn requirement_sound(&mut self, _p: Owner) {
        self.note("requirement_sound".into());
    }
    fn merc_sound(&mut self, _p: Owner) {
        self.note("merc_sound".into());
    }
    fn quest_flag(&self, _p: Owner, quest: u8, flag: u8) -> bool {
        self.k.quest_flags.contains(&(quest, flag))
    }
    fn quest_item_picked(&mut self, _p: Owner, item: Guid) {
        self.note(format!("quest_picked {item}"));
    }
    fn quest_item_dropped(&mut self, item: Guid) {
        self.note(format!("quest_dropped {item}"));
    }
    fn carry_one(&self, item: Guid) -> bool {
        self.it(item).carry_one
    }
    fn create_gold(&mut self, _unit: Owner, _spot: Spot) -> Option<Guid> {
        let g = self.next_guid;
        self.next_guid += 1;
        self.note(format!("create_gold {g}"));
        self.units.insert(Owner::item(g), FUnit::default());
        self.items.insert(
            g,
            FItem {
                mode: mode::GROUND,
                types: vec![4],
                code: *b"gld ",
                quality: 2,
                file_index: -1,
                ..Default::default()
            },
        );
        Some(g)
    }
    fn free_item(&mut self, item: Guid) {
        self.note(format!("free {item}"));
    }
    fn copy_item(&mut self, item: Guid) -> Option<Guid> {
        let g = self.next_guid;
        self.next_guid += 1;
        let mut c = self.it(item).clone();
        c.iflags = 0;
        self.items.insert(g, c);
        self.note(format!("copy {item} -> {g}"));
        Some(g)
    }
    fn give_cursor_item(&mut self, p: Owner, item: Guid) {
        self.set_cursor(p, Some(item));
    }
    fn consume_one(&mut self, item: Guid) -> bool {
        self.note(format!("consume {item}"));
        self.k.consume
    }
    fn set_owner(&mut self, item: Guid, _owner: Owner) {
        self.note(format!("set_owner {item}"));
    }
    fn pile_owner(&self, _item: Guid) -> Option<Owner> {
        self.k.pile_owner
    }
    fn gold_drop_has_no_owner(&self) -> bool {
        self.k.q44
    }
    fn rest_pile(&mut self, _p: Owner, rest: i32) {
        self.note(format!("rest_pile {rest}"));
    }
    fn merge_allowed(&self, _src: Guid) -> bool {
        self.k.merge
    }
    fn book_count_changed(&mut self, _p: Owner, _book: Guid, n: i32) {
        self.note(format!("book {n}"));
    }
    fn use_item(&mut self, _p: Owner, target: Owner, item: Guid) -> bool {
        self.note(format!("use {item} on {}:{}", target.ty, target.guid));
        self.k.use_ok
    }
    fn remove_used(&mut self, _p: Owner, item: Guid) {
        self.note(format!("remove_used {item}"));
    }
    fn equip_picked(&mut self, _p: Owner, item: Guid) -> bool {
        self.note(format!("equip_picked {item}"));
        self.k.equip_picked
    }
    fn runeword(&mut self, _p: Owner, target: Guid) -> bool {
        self.note(format!("runeword {target}"));
        false
    }
    fn hireling(&self, _p: Owner) -> Option<Owner> {
        self.k.hireling
    }
    fn alive(&self, _u: Owner) -> bool {
        self.k.alive
    }
    fn owns_hireling(&self, _p: Owner, _m: Owner) -> bool {
        self.k.owns
    }
    fn equip_on_merc(&mut self, _m: Owner, item: Guid) {
        self.note(format!("equip_on_merc {item}"));
    }
    fn send(&mut self, _p: Owner, bytes: Vec<u8>) {
        self.sent.push(bytes);
    }
    fn walk_to_unit(&mut self, _p: Owner, t: Owner, cursor: bool) {
        self.note(format!("walk_unit {}:{} {cursor}", t.ty, t.guid));
    }
    fn tile_warp(&mut self, _p: Owner, t: Owner) {
        self.note(format!("warp {}", t.guid));
    }
    fn corpse_pickup(&mut self, _p: Owner, c: Owner) -> bool {
        self.note(format!("corpse {}", c.guid));
        false
    }
    fn player_interact(&mut self, _p: Owner, o: Owner) {
        self.note(format!("interact {}", o.guid));
    }
    fn use_item_at(&mut self, _p: Owner, item: Guid, x: i32, y: i32) -> bool {
        self.note(format!("use_at {item} {x},{y}"));
        self.k.use_ok
    }
    fn consume_item(&mut self, _p: Owner, item: Guid) {
        self.note(format!("consume_item {item}"));
    }
    fn item_skill(&self, _item: Guid) -> i32 {
        self.k.item_skill
    }
    fn has_skill(&self, _p: Owner, _s: i32) -> bool {
        self.k.has_skill
    }
    fn skill_decrement(&mut self, _p: Owner, s: i32) {
        self.note(format!("skill_dec {s}"));
    }
    fn set_quest_flag(&mut self, _p: Owner, q: u8, f: u8, on: bool) {
        self.note(format!("quest_flag {q} {f} {on}"));
        if on {
            self.k.quest_flags.insert((q, f));
        } else {
            self.k.quest_flags.remove(&(q, f));
        }
    }
    fn quest_item_used(&mut self, _p: Owner, chain: u8) {
        self.note(format!("quest_used {chain}"));
    }
    fn quest_tr2_used(&mut self, _p: Owner) {
        self.note("tr2".into());
    }
    fn reset_skills_stats(&mut self, _p: Owner) {
        self.note("reset_skills_stats".into());
    }
    fn has_used_skill(&self, _p: Owner) -> bool {
        self.k.used_skill
    }
    fn send_item_stat(&mut self, _p: Owner, item: Guid, stat: u16) {
        self.note(format!("3E {item} {stat}"));
    }
    fn item_bits(&self, _item: Guid, flags: u32, page: u8) -> Vec<u8> {
        let mut b = self.k.bits.clone();
        if !b.is_empty() {
            // Echo the flag argument and the page shown.
            b.push(flags as u8);
            b.push(page);
        }
        b
    }
}
