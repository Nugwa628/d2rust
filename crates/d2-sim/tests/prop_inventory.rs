//! State-machine property test of the inventory model
//! (`d2_sim::items::inventory`) against a reference
//! model of grid placement, equipment checks and belt operations.
//!
//! Each case builds a random synthetic world (owner kind, page grid sizes,
//! item sizes and types, itemtypes body locations and flags, item and unit
//! stats; belt capacities come from the fixed `numboxes` array below)
//! and runs random placement, unlink, equip and belt operations, plus pure
//! eligibility checks on the state reached. After every step the inventory
//! must equal the model: items do not overlap or leave their grids; item
//! lists, cells, counts, cursor and updates agree; the free-position search
//! returns the same spot as the reference and finds one iff an item fits;
//! equipment checks enforce compatibility and requirements; belt slots stay
//! within the equipped belt's `numboxes`.
//!
//! The reference follows current choices for unresolved behavior (marked
//! `reading:`): a similar belt column without an empty slot tries the next
//! column; "type ≠ 38" tests the primary type; an equipped hand weapon can
//! be in either hand. The link check succeeds because the owner is not an
//! item, so these cases do not exercise socket-link failure.

use std::collections::BTreeMap;

use d2_data::fixup::maps::EquivMatrix;
use d2_sim::items::inventory::equip::res;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{
    auto_equip_location, belt_item_check, compact_belt, cursor_item_check, equip_check,
    equip_from_cursor, find_free_position, ground_or_owned_check, hands_compatible,
    owned_item_check, place_at_page, place_in_belt_slot, place_in_page, stack_test,
    stored_item_check, stored_or_equipped_check, InteractionTarget, InvItem, InvTables, InvWorld,
    Inventory, UnitKind, NO_GUID,
};
use d2_sim::items::inventory::{cmd, iflag, mode, node, page};
use d2_sim::units::UnitId;
use proptest::prelude::*;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

// ------------------------------------------------------------------ setup

/// Reads the random setup bytes; past the end every read is 0, so a
/// shrunk setup tends to the simplest world.
struct Dna<'a> {
    b: &'a [u8],
    i: usize,
}

impl Dna<'_> {
    fn u8(&mut self) -> u8 {
        let v = self.b.get(self.i).copied().unwrap_or(0);
        self.i += 1;
        v
    }
    fn below(&mut self, n: u8) -> u8 {
        self.u8() % n
    }
    fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[usize::from(self.u8()) % v.len()]
    }
}

// Itemtypes row IDs used by the synthetic tables and type checks.
const T_SHIE: i16 = 2;
const T_TORS: i16 = 3;
const T_BOWQ: i16 = 5;
const T_RING: i16 = 10;
const T_BOOK: i16 = 18;
const T_BELT: i16 = 19;
const T_BOW: i16 = 27;
const T_AXE: i16 = 28;
const T_SWOR: i16 = 30;
const T_HELM: i16 = 37;
const T_TPOT: i16 = 38;
const T_WEAP: i16 = 45;
const T_H2H: i16 = 67;
const T_HPOT: i16 = 76;
const N_TYPES: usize = 81;
const TYPE_POOL: [i16; 14] = [
    T_SHIE, T_TORS, T_BOWQ, T_RING, T_BOOK, T_BELT, T_BOW, T_AXE, T_SWOR, T_HELM, T_TPOT, T_WEAP,
    T_H2H, T_HPOT,
];
const CODES: [[u8; 4]; 8] = [
    *b"hp1 ", *b"hp3 ", *b"mp1 ", *b"mp5 ", *b"rvl ", *b"rvs ", *b"xyz ", *b"isc ",
];
/// Fixed `belts.bin` `numboxes` capacities for the 14 belt records.
const NUMBOXES: [u8; 14] = [12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16];

const N_REC: usize = 12;
const N_ITEMS: usize = 10;
const OWNER: UnitId = UnitId(1);
const OWNER_GUID: u32 = 1;
/// A GUID no unit has.
const MISSING_GUID: u32 = 9999;

fn uid(i: usize) -> UnitId {
    UnitId(100 + i as u32)
}
fn guid(i: usize) -> u32 {
    1000 + i as u32
}

/// Per-item answers of the fake seams.
#[derive(Clone, Debug, Default)]
struct Props {
    two_handed: bool,
    one_or_two: bool,
    ammo: Option<i16>,
    active: bool,
    contrib: [i32; 3],
    level_req: i32,
    req_percent: i32,
    quality: u8,
    file_index: i32,
    sockets: bool,
    damage: [i32; 6],
    quantity: i32,
    allowed_loc: bool,
    quiver_kind: bool,
    filled: bool,
}

#[derive(Debug)]
struct World {
    expansion: bool,
    owner_kind: UnitKind,
    unit_stats: [i32; 3],
    items: BTreeMap<UnitId, InvItem>,
    props: Vec<Props>,
    fits_free_page0: bool,
    same_act: bool,
    in_range: bool,
    // Logs of the seam calls the model predicts.
    untargets: Vec<u32>,
    room_removals: Vec<UnitId>,
    trade_hooks: Vec<UnitId>,
    foreign_unlinks: usize,
}

fn idx(u: UnitId) -> Option<usize> {
    (u.0 >= 100 && u.0 < 100 + N_ITEMS as u32).then(|| (u.0 - 100) as usize)
}

impl InvWorld for World {
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn item(&self, item: UnitId) -> Option<&InvItem> {
        self.items.get(&item)
    }
    fn item_mut(&mut self, item: UnitId) -> Option<&mut InvItem> {
        self.items.get_mut(&item)
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.items
            .iter()
            .find(|(_, d)| d.guid == guid)
            .map(|(&u, _)| u)
    }
    fn item_stat(&self, item: UnitId, stat: u16) -> i32 {
        let Some(p) = idx(item).map(|i| &self.props[i]) else {
            return 0;
        };
        match stat {
            70 => p.quantity,
            21 => p.damage[0],
            22 => p.damage[1],
            23 => p.damage[2],
            24 => p.damage[3],
            159 => p.damage[4],
            160 => p.damage[5],
            _ => 0,
        }
    }
    fn unlink_from(&mut self, _owner: UnitId, _item: UnitId) {
        self.foreign_unlinks += 1;
    }
    fn remove_from_room(&mut self, item: UnitId) {
        self.room_removals.push(item);
    }
    fn clear_targetable(&mut self, _item: UnitId) {}
    fn link_check(&mut self, _owner: UnitId, _item: UnitId, _kind: u8) -> bool {
        // The link check succeeds when the inventory does not belong to an item.
        true
    }
    fn charm_relink(&mut self, _owner: UnitId, _item: UnitId) {}
    fn active_item(&self, _owner: UnitId, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].active)
    }
    fn stat_refresh(&mut self, _owner: UnitId) {}
    fn socket_filled(&self, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].filled)
    }
    fn owner_refresh(&mut self, _owner: UnitId) {}
    fn inventory_pass(&mut self, _owner: UnitId) {}
    fn trade_hook(&mut self, _owner: UnitId, item: UnitId) {
        self.trade_hooks.push(item);
    }
    fn weapon_in_use_update(&mut self, _unit: UnitId) {}
    fn stat_link(&mut self, _unit: UnitId, _item: UnitId) {}
    fn weapon_bookkeeping(&mut self, _unit: UnitId, _item: UnitId) {}
    fn unit_kind(&self, unit: UnitId) -> Option<UnitKind> {
        (unit == OWNER).then_some(self.owner_kind)
    }
    fn unit_stat(&self, unit: UnitId, stat: u16) -> i32 {
        if unit != OWNER {
            return 0;
        }
        match stat {
            0 => self.unit_stats[0],
            2 => self.unit_stats[1],
            12 => self.unit_stats[2],
            _ => 0,
        }
    }
    fn req_percent(&self, item: UnitId) -> i32 {
        idx(item).map_or(0, |i| self.props[i].req_percent)
    }
    fn item_active_on(&self, item: UnitId, _unit: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].active)
    }
    fn own_contribution(&self, item: UnitId, _unit: UnitId, stat: u16) -> i32 {
        let Some(i) = idx(item) else { return 0 };
        match stat {
            0 => self.props[i].contrib[0],
            2 => self.props[i].contrib[1],
            _ => self.props[i].contrib[2],
        }
    }
    fn level_requirement(&self, item: UnitId, _unit: UnitId) -> i32 {
        idx(item).map_or(-1, |i| self.props[i].level_req)
    }
    fn two_handed(&self, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].two_handed)
    }
    fn one_or_two_handed(&self, _unit: UnitId, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].one_or_two)
    }
    fn ammo_type(&self, item: UnitId) -> Option<i16> {
        idx(item).and_then(|i| self.props[i].ammo)
    }
    fn fits_free_page0(&self, _inv: &Inventory, _item: UnitId) -> bool {
        self.fits_free_page0
    }
    fn quality(&self, item: UnitId) -> u8 {
        idx(item).map_or(0, |i| self.props[i].quality)
    }
    fn stack_file_index(&self, item: UnitId) -> i32 {
        idx(item).map_or(0, |i| self.props[i].file_index)
    }
    fn has_sockets(&self, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].sockets)
    }
    fn has_allowed_location(&self, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].allowed_loc)
    }
    fn quiver_kind(&self, item: UnitId) -> bool {
        idx(item).is_some_and(|i| self.props[i].quiver_kind)
    }
    /// Targeting probe: the owner's type (player 0, monster 1), 6 for any
    /// unit but the owner.
    fn targeting_probe(&self, unit: UnitId) -> u32 {
        match self.unit_kind(unit) {
            Some(UnitKind::Player { .. }) => 0,
            Some(_) => 1,
            None => 6,
        }
    }
    fn queue_untarget(&mut self, _player: UnitId, item_guid: u32) {
        self.untargets.push(item_guid);
    }
    fn interaction(&self, _player: UnitId) -> InteractionTarget {
        InteractionTarget::None
    }
    fn clear_interaction(&mut self, _player: UnitId) {}
    fn player_data_4c(&self, _player: UnitId) -> u32 {
        0
    }
    fn player_data_50(&self, _player: UnitId) -> u32 {
        0
    }
    fn npc_talking(&self, _npc: UnitId, _player: UnitId) -> bool {
        false
    }
    fn player_trade_gate(&self, _player: UnitId) -> Option<bool> {
        None
    }
    fn same_act(&self, _player: UnitId, _item: UnitId) -> bool {
        self.same_act
    }
    fn within_range(&self, _player: UnitId, _item: UnitId, _range: i32) -> bool {
        self.in_range
    }
}

fn build(dna: &[u8]) -> (InvTables, World) {
    let mut d = Dna { b: dna, i: 0 };
    let owner_kind = match d.below(10) {
        k @ 0..=6 => UnitKind::Player { class: k },
        k => UnitKind::Monster {
            class: [0x1A1, 0x21C, 0x230][usize::from(k - 7)],
        },
    };
    let expansion = d.bool();
    let grids = (0..32)
        .map(|_| GridRec {
            grid_x: d.pick(&[10, 6, 3, 1, 2, 4, 8, 0]),
            grid_y: d.pick(&[4, 1, 2, 3, 5, 8, 10, 0]),
        })
        .collect();
    let mut itemtypes = vec![
        InvTypeRec {
            class: 7,
            ..Default::default()
        };
        N_TYPES
    ];
    let words = N_TYPES.div_ceil(32);
    let mut equiv = EquivMatrix {
        n: N_TYPES,
        words,
        bits: vec![0; N_TYPES * words],
    };
    let mut eq = |i: i16, j: i16| {
        let (i, j) = (i as usize, j as usize);
        equiv.bits[i * words + j / 32] |= 1 << (j % 32);
    };
    for i in 0..N_TYPES as i16 {
        eq(i, i);
    }
    eq(T_H2H, T_WEAP);
    for t in TYPE_POOL {
        let r = &mut itemtypes[t as usize];
        // Hands (4, 5) dominate so hand compatibility checks are reached often.
        let locs = [4, 5, 4, 5, 1, 8, 6, 7, 3, 9, 10, 2, 11, 12, 0];
        r.bodyloc1 = d.pick(&locs);
        r.bodyloc2 = if d.below(3) == 2 {
            r.bodyloc1
        } else {
            d.pick(&locs)
        };
        r.body = u8::from(d.below(4) != 0);
        r.beltable = u8::from(d.bool());
        r.quiver = u16::from(d.below(5) == 4);
        r.class = if d.below(8) == 7 { d.below(7) } else { 7 };
        if d.bool() {
            eq(t, T_WEAP);
        }
        if d.below(4) == 3 {
            eq(t, T_H2H);
        }
    }
    let items = (0..N_REC)
        .map(|_| InvItemRec {
            code: d.pick(&CODES),
            type_: d.pick(&TYPE_POOL),
            type2: if d.below(4) == 3 {
                d.pick(&TYPE_POOL)
            } else {
                -1
            },
            invwidth: d.pick(&[1, 2, 1, 3, 0]),
            invheight: d.pick(&[1, 2, 3, 4, 1, 0]),
            reqstr: u16::from(d.below(4)) * 15,
            reqdex: u16::from(d.below(4)) * 10,
            belt: d.below(15),
            autobelt: d.below(2),
            stackable: d.below(2),
            ..Default::default()
        })
        .collect();
    let t = InvTables {
        grids,
        belts: NUMBOXES.to_vec(),
        items,
        itemtypes,
        equiv,
        books: Vec::new(),
        item_use: Default::default(),
    };
    let mut w = World {
        expansion,
        owner_kind,
        unit_stats: [0; 3],
        items: BTreeMap::new(),
        props: Vec::new(),
        fits_free_page0: false,
        same_act: true,
        in_range: true,
        untargets: Vec::new(),
        room_removals: Vec::new(),
        trade_hooks: Vec::new(),
        foreign_unlinks: 0,
    };
    for i in 0..N_ITEMS {
        let mut it = InvItem::new(guid(i), usize::from(d.below(N_REC as u8)));
        it.flags = if d.below(5) == 4 {
            0
        } else {
            iflag::IDENTIFIED
        };
        for (n, f) in [
            (6, iflag::ETHEREAL),
            (8, iflag::BROKEN),
            (8, iflag::F4000),
            (3, iflag::TARGETING),
        ] {
            if d.below(n) == n - 1 {
                it.flags |= f;
            }
        }
        w.items.insert(uid(i), it);
        w.props.push(Props {
            two_handed: d.bool(),
            one_or_two: d.bool(),
            ammo: (d.below(4) == 3).then(|| d.pick(&TYPE_POOL)),
            active: d.bool(),
            contrib: [i32::from(d.below(30)), i32::from(d.below(30)), 0],
            level_req: i32::from(d.below(20)) - 1,
            req_percent: d.pick(&[0, 0, 10, -20, 50]),
            quality: d.below(3),
            file_index: i32::from(d.below(2)),
            sockets: d.below(4) == 3,
            damage: [0; 6].map(|_: i32| i32::from(d.below(2))),
            quantity: i32::from(d.below(3)) - 1,
            allowed_loc: d.below(4) != 3,
            quiver_kind: d.below(4) == 3,
            filled: d.bool(),
        });
    }
    // Mostly strong enough, so equipment requirements often pass.
    w.unit_stats = [
        d.pick(&[100, 100, 60, 20, 0, 35]),
        d.pick(&[100, 100, 40, 10, 0, 25]),
        d.pick(&[30, 30, 5, 0]),
    ];
    w.fits_free_page0 = d.bool();
    w.same_act = d.below(4) != 3;
    w.in_range = d.below(4) != 3;
    (t, w)
}

// ------------------------------------------------------------------ model

/// Grid sizes of the fixed body and belt grids.
const BODY_GRID: (u8, u8) = (13, 1);
const BELT_GRID: (u8, u8) = (16, 1);

/// Reference inventory state tracked independently of the implementation.
#[derive(Clone, Debug, Default)]
struct Model {
    /// Item list in link order.
    list: Vec<usize>,
    /// Placement of each linked item: (grid, x, y).
    at: BTreeMap<usize, (usize, i32, i32)>,
    cursor: Option<usize>,
    count: u32,
    /// Update list GUIDs.
    updates: Vec<u32>,
    untargets: Vec<u32>,
    room_removals: Vec<UnitId>,
    trade_hooks: Vec<UnitId>,
}

/// Reference `inventory.bin` record selection for a page.
fn ref_record(owner: UnitKind, pg: u8, expansion: bool) -> Option<usize> {
    match owner {
        UnitKind::Player { class } => match pg {
            1 => Some(6),
            2 => Some(7),
            3 => Some(9),
            4 => Some(if expansion { 12 } else { 8 }),
            _ => match class {
                0..=4 => Some(usize::from(class)),
                5 => Some(14),
                6 => Some(15),
                _ => None,
            },
        },
        UnitKind::Monster { .. } => Some(5),
        UnitKind::Object { class } => match class {
            0x152 => Some(10),
            0x153 => Some(11),
            _ => None,
        },
        _ => None,
    }
}

struct Ctx<'a> {
    t: &'a InvTables,
    owner: UnitKind,
    expansion: bool,
}

impl Ctx<'_> {
    fn page_size(&self, pg: u8) -> Option<(u8, u8)> {
        let r = self.t.grids[ref_record(self.owner, pg, self.expansion)?];
        Some((r.grid_x, r.grid_y))
    }
    fn grid_size(&self, g: usize) -> Option<(u8, u8)> {
        match g {
            0 => Some(BODY_GRID),
            1 => Some(BELT_GRID),
            _ => self.page_size((g - 2) as u8),
        }
    }
    fn rec(&self, w: &World, i: usize) -> &InvItemRec {
        &self.t.items[w.items[&uid(i)].record]
    }
    /// Cells an item covers in grid g: invwidth × invheight on pages,
    /// 1 × 1 on the body and belt grids.
    fn cover(&self, w: &World, i: usize, g: usize) -> (u8, u8) {
        if g < 2 {
            (1, 1)
        } else {
            let r = self.rec(w, i);
            (r.invwidth, r.invheight)
        }
    }
    fn itype(&self, w: &World, i: usize) -> &InvTypeRec {
        &self.t.itemtypes[self.rec(w, i).type_ as usize]
    }
    fn is_type(&self, w: &World, i: usize, ty: i16) -> bool {
        let r = self.rec(w, i);
        let e = |a: i16| a >= 0 && self.t.equiv.get(a as usize, ty as usize);
        e(r.type_) || (r.type2 > 0 && e(r.type2))
    }
}

impl Model {
    fn occupant(&self, c: &Ctx, w: &World, g: usize, x: i32, y: i32) -> Option<usize> {
        self.at.iter().find_map(|(&i, &(gi, ix, iy))| {
            let (iw, ih) = c.cover(w, i, gi);
            (gi == g
                && x >= ix
                && x < ix.wrapping_add(i32::from(iw))
                && y >= iy
                && y < iy.wrapping_add(i32::from(ih)))
            .then_some(i)
        })
    }
    /// Bounds and fit (the moving item's own cells count as taken).
    #[allow(clippy::too_many_arguments)]
    fn fits(&self, c: &Ctx, w: &World, g: usize, x: i32, y: i32, iw: u8, ih: u8) -> bool {
        let Some((gw, gh)) = c.grid_size(g) else {
            return false;
        };
        // Use wrapping i32 bounds; grids 0 and 1 skip the bound test.
        if g >= 2
            && (x < 0
                || y < 0
                || x.wrapping_add(i32::from(iw)) > i32::from(gw)
                || y.wrapping_add(i32::from(ih)) > i32::from(gh))
        {
            return false;
        }
        (y..y.wrapping_add(i32::from(ih))).all(|yy| {
            (x..x.wrapping_add(i32::from(iw))).all(|xx| self.occupant(c, w, g, xx, yy).is_none())
        })
    }
    /// Unlink clears the cursor if it was the item, else
    /// the count drops by 1.
    fn unlink(&mut self, i: usize) -> bool {
        let Some(p) = self.list.iter().position(|&x| x == i) else {
            return false;
        };
        self.list.remove(p);
        self.at.remove(&i);
        if self.cursor == Some(i) {
            self.cursor = None;
        } else {
            self.count = self.count.wrapping_sub(1);
        }
        true
    }
    /// Reference effects of a successful placement.
    fn place(&mut self, w: &World, i: usize, g: usize, x: i32, y: i32) {
        if w.items[&uid(i)].mode == mode::GROUND {
            self.room_removals.push(uid(i));
        }
        self.unlink(i);
        // The cursor item belongs to this inventory: its unlink clears
        // the cursor.
        if self.cursor == Some(i) {
            self.cursor = None;
        }
        self.list.push(i);
        self.at.insert(i, (g, x, y));
        self.count = self.count.wrapping_add(1);
    }
    fn push_update(&mut self, g: u32) {
        if !self.updates.contains(&g) {
            self.updates.push(g);
        }
    }
    /// Targeting reset over the item list: queues the expected 0x3F
    /// GUIDs and returns the items whose flag 0x4 must be cleared.
    fn targeting_reset(&mut self, w: &World) -> Vec<usize> {
        let mut flagged = Vec::new();
        for &i in &self.list {
            if w.items[&uid(i)].flags & iflag::TARGETING != 0 {
                flagged.push(i);
                if matches!(w.owner_kind, UnitKind::Player { .. }) {
                    self.untargets.push(guid(i));
                }
            }
        }
        flagged
    }
}

/// Reference placement weight based on contact with grid edges and items.
fn ref_weight(
    occ: &dyn Fn(i32, i32) -> bool,
    gw: i32,
    gh: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> u32 {
    let mut total = 0;
    total += if x > 0 {
        (y..y + h).filter(|&yy| occ(x - 1, yy)).count() as i32
    } else {
        h
    };
    total += if x + w < gw {
        (y..y + h).filter(|&yy| occ(x + w, yy)).count() as i32
    } else {
        h
    };
    total += if y > 0 {
        (x..x + w).filter(|&xx| occ(xx, y - 1)).count() as i32
    } else {
        w
    };
    total += if y + h < gh {
        (x..x + w).filter(|&xx| occ(xx, y + h)).count() as i32
    } else {
        w
    };
    if total >= 2 * (w + h) {
        255
    } else {
        total as u32
    }
}

/// Brute-force placement search with order chosen by owner kind and item height.
fn ref_search(
    occ: &dyn Fn(i32, i32) -> bool,
    gw: u8,
    gh: u8,
    w: u8,
    h: u8,
    player: bool,
) -> Option<(i32, i32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let (gw, gh, w, h) = (i32::from(gw), i32::from(gh), i32::from(w), i32::from(h));
    let mut order = Vec::new();
    if h == 1 {
        for x in (0..gw).rev() {
            if player {
                order.extend((0..gh).rev().map(|y| (x, y)));
            } else {
                order.extend((0..gh).map(|y| (x, y)));
            }
        }
    } else if player {
        for y in 0..gh {
            order.extend((0..gw).map(|x| (x, y)));
        }
    } else {
        for x in 0..gw {
            order.extend((0..gh).map(|y| (x, y)));
        }
    }
    let fit = |x: i32, y: i32| {
        !occ(x, y)
            && x + w <= gw
            && y + h <= gh
            && (y..y + h).all(|yy| (x..x + w).all(|xx| !occ(xx, yy)))
    };
    let mut best = None;
    let mut best_w = 0;
    for (x, y) in order {
        if !fit(x, y) {
            continue;
        }
        if !player {
            return Some((x, y));
        }
        let wt = ref_weight(&occ, gw, gh, x, y, w, h);
        if wt > best_w {
            best_w = wt;
            best = Some((x, y));
            if wt == 255 {
                break;
            }
        }
    }
    best.filter(|_| best_w > 0)
}

/// Reference equipment requirements of item `i` for the owner.
fn ref_req(c: &Ctx, w: &World, i: usize, equipping: bool) -> bool {
    let d = w.items[&uid(i)];
    let r = c.rec(w, i);
    let p = &w.props[i];
    let (rs, rd) = (i32::from(r.reqstr), i32::from(r.reqdex));
    let (mut bs, mut bd) = (0, 0);
    if p.req_percent != 0 {
        bs = rs * p.req_percent / 100;
        bd = rd * p.req_percent / 100;
    }
    if d.flags & iflag::ETHEREAL != 0 {
        bs -= 10;
        bd -= 10;
    }
    // Rule 3 tests the stat-list link before subtracting the contribution;
    // rule 4 (dexterity) has no link test.
    let stat_ok = |v: i32, req: i32, own: i32, linked: bool| {
        if v < 1 || v < req {
            return false;
        }
        if equipping && linked {
            let v = v - own;
            if v < 1 || v < req {
                return false;
            }
        }
        true
    };
    if !stat_ok(w.unit_stats[0], rs + bs, p.contrib[0], p.active) {
        return false;
    }
    if !stat_ok(w.unit_stats[1], rd + bd, p.contrib[1], true) {
        return false;
    }
    if w.unit_stats[2] < p.level_req {
        return false;
    }
    if d.flags & iflag::IDENTIFIED == 0 {
        return false;
    }
    if r.type_ == T_BOOK && p.quantity <= 0 {
        return false;
    }
    let class = c.itype(w, i).class;
    if class == 7 {
        return true;
    }
    match c.owner {
        UnitKind::Player { class: pc } => pc == class,
        UnitKind::Monster { class: m } => (m == 0x230 || m == 0x231) && class == 4,
        _ => false,
    }
}

/// Reference allowed body-location check, including alternate hand slots.
fn ref_loc_allowed(c: &Ctx, w: &World, i: usize, loc: u8) -> bool {
    let ty = c.itype(w, i);
    let (a, b) = (ty.bodyloc1, ty.bodyloc2);
    loc == a || loc == b || ((loc == 11 || loc == 12) && [a, b].iter().any(|&l| l == 4 || l == 5))
}

/// Reference compatibility check for the two equipped hands.
fn ref_hands(c: &Ctx, w: &World, a: Option<usize>, b: Option<usize>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return true;
    };
    let ammo = |x: usize, y: usize| w.props[x].ammo.is_some_and(|t| c.is_type(w, y, t));
    if ammo(a, b) || ammo(b, a) {
        return true;
    }
    if c.itype(w, a).quiver != 0 || c.itype(w, b).quiver != 0 {
        return false;
    }
    if [a, b]
        .iter()
        .any(|&x| w.props[x].two_handed && !w.props[x].one_or_two)
    {
        return false;
    }
    let (wa, wb) = (c.is_type(w, a, T_WEAP), c.is_type(w, b, T_WEAP));
    if !(wa && wb) {
        return wa || wb;
    }
    let h2h = c.is_type(w, a, T_H2H) && c.is_type(w, b, T_H2H);
    match c.owner {
        UnitKind::Player { class: 4 } => true,
        UnitKind::Player { class: 6 } => h2h,
        UnitKind::Player { .. } => false,
        UnitKind::Monster { class } => match class {
            0x1A1 | 0x1A2 => h2h,
            0x21C..=0x21E => true,
            _ => false,
        },
        _ => false,
    }
}

/// Reference stack compatibility check.
fn ref_stack(c: &Ctx, w: &World, a: usize, b: usize) -> bool {
    let (pa, pb) = (&w.props[a], &w.props[b]);
    let ra = w.items[&uid(a)].record;
    ra == w.items[&uid(b)].record
        && pa.quality == pb.quality
        && pa.file_index == pb.file_index
        && c.t.items[ra].stackable != 0
        && (w.items[&uid(a)].flags & iflag::ETHEREAL) == (w.items[&uid(b)].flags & iflag::ETHEREAL)
        && (1..=3).contains(&pa.quality)
        && (1..=3).contains(&pb.quality)
        && pa.damage == pb.damage
        && !pa.sockets
        && !pb.sockets
}

/// Reference equip result, including hand displacement and compatibility.
fn ref_equip_check(m: &Model, c: &Ctx, w: &World, loc: u8, n: Option<usize>, skip: bool) -> u8 {
    if let Some(n) = n {
        if !ref_loc_allowed(c, w, n, loc) || (!skip && !ref_req(c, w, n, true)) {
            return res::NO;
        }
    }
    if loc == 0 {
        return res::NO;
    }
    let body = |l: u8| m.occupant(c, w, 0, i32::from(l), 0);
    let target = body(loc);
    let other = match loc {
        4 => 5,
        5 => 4,
        11 => 12,
        12 => 11,
        _ => {
            return match (n, target) {
                (Some(_), Some(_)) => res::SWAP,
                (Some(_), None) => res::FREE,
                (None, Some(_)) => res::REMOVABLE,
                (None, None) => res::NO,
            }
        }
    };
    let x = body(other);
    match (n, target, x) {
        (None, Some(_), _) => res::REMOVABLE,
        (None, None, Some(x)) if w.props[x].two_handed => res::OTHER_HAND_TWO_HANDED,
        (None, None, _) => res::NO,
        (Some(n), Some(tt), x) => {
            if ref_stack(c, w, tt, n) {
                res::STACK
            } else if ref_hands(c, w, Some(n), x) {
                res::SWAP
            } else if x.is_some() && w.fits_free_page0 {
                res::SWAP_OTHER_TO_PAGE
            } else {
                res::NO
            }
        }
        (Some(n), None, Some(x)) => {
            if ref_hands(c, w, Some(n), Some(x)) {
                res::FREE
            } else {
                res::OTHER_HAND_BLOCKS
            }
        }
        (Some(_), None, None) => res::FREE,
    }
}

/// Reference belt similarity by item record or potion-code group.
fn ref_similar(c: &Ctx, a: usize, b: usize) -> bool {
    const GROUPS: [&[&[u8; 4]]; 3] = [
        &[b"hp1 ", b"hp2 ", b"hp3 ", b"hp4 ", b"hp5 "],
        &[b"mp1 ", b"mp2 ", b"mp3 ", b"mp4 ", b"mp5 "],
        &[b"rvl ", b"rvs "],
    ];
    a == b || {
        let (ca, cb) = (&c.t.items[a].code, &c.t.items[b].code);
        GROUPS.iter().any(|g| g.contains(&ca) && g.contains(&cb))
    }
}

/// `numboxes` capacity of the belt in use.
fn ref_numboxes(m: &Model, c: &Ctx, w: &World) -> Option<u8> {
    let belt = m
        .occupant(c, w, 0, 8, 0)
        .map_or(2, |b| usize::from(c.rec(w, b).belt));
    NUMBOXES.get(belt).copied()
}

/// Reference belt search (reading: a full similar column tries the next).
fn ref_free_belt_slot(m: &Model, c: &Ctx, w: &World, i: usize) -> Option<u8> {
    let r = c.rec(w, i);
    if c.itype(w, i).beltable == 0 || (r.invwidth, r.invheight) != (1, 1) {
        return None;
    }
    let n = ref_numboxes(m, c, w)?;
    let rec = w.items[&uid(i)].record;
    let held = |s: u8| m.occupant(c, w, 1, i32::from(s), 0);
    for col in 0..4u8 {
        let similar = held(col).is_some_and(|h| ref_similar(c, w.items[&uid(h)].record, rec));
        if col < n && similar {
            let mut s = col;
            while s < n {
                if held(s).is_none() {
                    return Some(s);
                }
                s += 4;
            }
        }
    }
    if r.autobelt != 0 {
        return (0..4).find(|&s| held(s).is_none());
    }
    None
}

/// Reference auto-equip (readings: primary type for "type ≠ 38"; either hand
/// can supply the equipped weapon).
fn ref_auto_equip(m: &Model, c: &Ctx, w: &World, i: usize, skip: bool) -> Option<u8> {
    let d = w.items[&uid(i)];
    let ty = c.itype(w, i);
    if !skip && !ref_req(c, w, i, false) {
        return None;
    }
    if ty.body == 0 || !w.props[i].allowed_loc || d.flags & iflag::IDENTIFIED == 0 {
        return None;
    }
    if d.flags & (iflag::BROKEN | iflag::F4000) != 0 || c.rec(w, i).type_ == T_TPOT {
        return None;
    }
    let body = |l: u8| m.occupant(c, w, 0, i32::from(l), 0);
    if w.props[i].quiver_kind {
        let fed = [4, 5].iter().any(|&l| {
            body(l).is_some_and(|h| w.props[h].ammo.is_some_and(|am| c.is_type(w, i, am)))
        });
        if !fed {
            return None;
        }
    }
    let (l1, l2) = (ty.bodyloc1, ty.bodyloc2);
    if l1 == l2 {
        return body(l1).is_none().then_some(l1);
    }
    match (body(l1), body(l2)) {
        (None, None) => Some(l1),
        (None, Some(e)) => ref_compatible(c, w, i, e).then_some(l1),
        (Some(e), None) => ref_compatible(c, w, i, e).then_some(l2),
        (Some(_), Some(_)) => None,
    }
}

/// Compatibility of the new item `n` with the equipped item `e`,
/// based on item types, handedness and the owner's dual-wield capability.
fn ref_compatible(c: &Ctx, w: &World, n: usize, e: usize) -> bool {
    let prof = |i: usize| {
        let is = |t: i16| c.is_type(w, i, t);
        let dual = match c.owner {
            UnitKind::Player { class: 4 } => true,
            UnitKind::Player { class: 6 } => is(T_H2H),
            _ => false,
        };
        let one_hand = is(T_WEAP) && !w.props[i].two_handed;
        (is(27), is(35), is(5), is(6), is(51), one_hand, dual, is(10))
    };
    let (a, b) = (prof(n), prof(e));
    (a.0 && b.2)
        || (a.2 && b.0)
        || (a.1 && b.3)
        || (a.3 && b.1)
        || (a.5 && b.4)
        || (b.5 && a.4)
        || (a.5 && b.5 && a.6 && b.6)
        || (a.7 && b.7)
}

// ------------------------------------------------------------------ ops

#[derive(Clone, Debug)]
enum Op {
    /// Place at a position on a page.
    PlaceAt { i: usize, pg: u8, x: i32, y: i32 },
    /// Search for a free position, then place there.
    PlaceFree { i: usize, pg: u8 },
    /// Insert from the cursor.
    FromCursor {
        i: usize,
        pg: u8,
        x: i32,
        y: i32,
        find: bool,
        send: bool,
    },
    /// Unlink, then leave the item on the ground.
    Remove { i: usize },
    /// Equip from the cursor.
    Equip { i: usize, loc: u8, skip: bool },
    // (loc: see `Harness::loc`.)
    /// Place into a given belt slot.
    Belt { i: usize, slot: u8 },
    /// Search for a belt slot, then place there.
    BeltFree { i: usize },
    /// Compact the belt.
    Compact { slot: u8 },
    /// Check equip results, hand and stack compatibility, auto-equip and
    /// ground-or-owned status on the current state.
    Checks {
        a: Option<usize>,
        b: Option<usize>,
        loc: u8,
        skip: bool,
    },
    /// Flips item flags (identified, ethereal, broken, 0x4000, targeting).
    Flags { i: usize, bits: u8 },
    /// Set an item's mode for the ground-or-owned checks.
    SetMode { i: usize, m: u8 },
}

fn op() -> impl Strategy<Value = Op> {
    let i = 0..N_ITEMS;
    let pg = 0u8..6;
    // Mostly inside the grids; sometimes the extremes a u32 payload field
    // read as i32 gives (0x18 passes x, y through to cursor insertion).
    let xy = prop_oneof![
        12 => -2i32..12,
        1 => prop_oneof![
            Just(i32::MAX),
            Just(i32::MAX - 1),
            Just(i32::MAX - 3),
            Just(i32::MIN),
            Just(-1)
        ],
    ];
    prop_oneof![
        3 => (i.clone(), pg.clone(), xy.clone(), xy.clone())
            .prop_map(|(i, pg, x, y)| Op::PlaceAt { i, pg, x, y }),
        3 => (i.clone(), pg.clone()).prop_map(|(i, pg)| Op::PlaceFree { i, pg }),
        3 => (i.clone(), pg, xy.clone(), xy, any::<bool>(), any::<bool>()).prop_map(
            |(i, pg, x, y, find, send)| Op::FromCursor { i, pg, x, y, find, send }
        ),
        2 => i.clone().prop_map(|i| Op::Remove { i }),
        3 => (i.clone(), 0u8..20, any::<bool>()).prop_map(|(i, loc, skip)| Op::Equip { i, loc, skip }),
        2 => (i.clone(), 0u8..18).prop_map(|(i, slot)| Op::Belt { i, slot }),
        2 => i.clone().prop_map(|i| Op::BeltFree { i }),
        1 => (0u8..16).prop_map(|slot| Op::Compact { slot }),
        2 => (
            proptest::option::of(i.clone()),
            proptest::option::of(i.clone()),
            0u8..20,
            any::<bool>()
        )
            .prop_map(|(a, b, loc, skip)| Op::Checks { a, b, loc, skip }),
        1 => (i.clone(), any::<u8>()).prop_map(|(i, bits)| Op::Flags { i, bits }),
        1 => (i, 0u8..7).prop_map(|(i, m)| Op::SetMode { i, m }),
    ]
}

struct Harness<'a> {
    c: Ctx<'a>,
    w: World,
    inv: Inventory,
    m: Model,
}

impl Harness<'_> {
    fn d(&self, i: usize) -> InvItem {
        self.w.items[&uid(i)]
    }
    fn dm(&mut self, i: usize) -> &mut InvItem {
        self.w.items.get_mut(&uid(i)).unwrap()
    }
    fn player(&self) -> bool {
        matches!(self.c.owner, UnitKind::Player { .. })
    }

    /// Lift an item to the cursor as the intents do before insert / equip
    /// (unlink, mode 4, cursor := item).
    fn lift(&mut self, i: usize, pg: u8) {
        let linked = self.m.unlink(i);
        assert_eq!(
            self.inv.unlink(&mut self.w, uid(i)),
            linked,
            "unlink of {i}"
        );
        let d = self.dm(i);
        d.mode = mode::CURSOR;
        d.page = pg;
        self.inv.set_cursor(Some(uid(i)));
        self.m.cursor = Some(i);
    }

    /// Puts a refused cursor item back on the ground.
    fn drop_cursor(&mut self) {
        if let Some(i) = self.m.cursor.take() {
            if !self.m.list.contains(&i) {
                let d = self.dm(i);
                d.mode = mode::GROUND;
                d.page = page::NONE;
            }
        }
        self.inv.set_cursor(None);
    }

    /// The model's search for item i on page pg (None without a grid
    /// record or with a zero size).
    fn ref_find(&self, i: usize, pg: u8) -> Option<(i32, i32)> {
        let r = self.c.rec(&self.w, i);
        let (gw, gh) = self.c.page_size(pg)?;
        let g = 2 + usize::from(pg);
        let occ = |x: i32, y: i32| self.m.occupant(&self.c, &self.w, g, x, y).is_some();
        let found = ref_search(&occ, gw, gh, r.invwidth, r.invheight, self.player());
        // A spot iff one fits anywhere (a fitting candidate slid to an
        // edge or a neighbour has weight > 0).
        let exists = r.invwidth > 0
            && r.invheight > 0
            && (0..i32::from(gw)).any(|x| {
                (0..i32::from(gh)).any(|y| {
                    self.m
                        .fits(&self.c, &self.w, g, x, y, r.invwidth, r.invheight)
                })
            });
        assert_eq!(found.is_some(), exists, "search finds a spot iff one fits");
        if let Some((x, y)) = found {
            assert!(self
                .m
                .fits(&self.c, &self.w, g, x, y, r.invwidth, r.invheight));
        }
        found
    }

    fn run(&mut self, op: &Op) {
        let c_t = self.c.t;
        match *op {
            Op::PlaceAt { i, pg, x, y } => {
                let r = self.c.rec(&self.w, i);
                let (iw, ih) = (r.invwidth, r.invheight);
                let g = 2 + usize::from(pg);
                let want = iw > 0
                    && ih > 0
                    && self.c.page_size(pg).is_some()
                    && self.m.fits(&self.c, &self.w, g, x, y, iw, ih);
                let got = place_at_page(&mut self.inv, &mut self.w, c_t, uid(i), pg, x, y);
                assert_eq!(got, want, "place_at_page");
                if want {
                    self.m.place(&self.w, i, g, x, y);
                    // Mode 0 as the insertion callers set it.
                    self.dm(i).mode = mode::STORED;
                }
            }
            Op::PlaceFree { i, pg } => {
                let want = self.ref_find(i, pg);
                let got = find_free_position(&mut self.inv, &self.w, c_t, uid(i), pg);
                assert_eq!(got, want, "find_free_position");
                if let Some((x, y)) = got {
                    assert!(place_at_page(
                        &mut self.inv,
                        &mut self.w,
                        c_t,
                        uid(i),
                        pg,
                        x,
                        y
                    ));
                    self.m.place(&self.w, i, 2 + usize::from(pg), x, y);
                    self.dm(i).mode = mode::STORED;
                }
            }
            Op::FromCursor {
                i,
                pg,
                x,
                y,
                find,
                send,
            } => {
                self.lift(i, pg);
                let r = self.c.rec(&self.w, i);
                let (iw, ih) = (r.invwidth, r.invheight);
                let g = 2 + usize::from(pg);
                let flagged = self.m.targeting_reset(&self.w);
                let spot = if find {
                    self.ref_find(i, pg)
                } else {
                    let (x, y) = (x.max(0), y.max(0));
                    (iw > 0
                        && ih > 0
                        && self.c.page_size(pg).is_some()
                        && self.m.fits(&self.c, &self.w, g, x, y, iw, ih))
                    .then_some((x, y))
                };
                let got = place_in_page(
                    &mut self.inv,
                    &mut self.w,
                    c_t,
                    Some(uid(i)),
                    x,
                    y,
                    find,
                    send,
                );
                assert_eq!(got, spot.is_some(), "place_in_page");
                self.reset_done(&flagged);
                if let Some((x, y)) = spot {
                    self.m.place(&self.w, i, g, x, y);
                    if self.player() && pg == page::TRADE2 {
                        self.m.trade_hooks.push(uid(i));
                    }
                    // Step 7: the cursor stays for a player's page 1.
                    if !(self.player() && pg == page::TRADE1) {
                        self.m.cursor = None;
                    }
                    let d = self.d(i);
                    assert_eq!(d.mode, mode::STORED, "§2.4 step 7 mode");
                    if send {
                        assert_ne!(d.cmd_flags & cmd::PUT_IN_CONTAINER, 0);
                        assert_eq!(d.flags & iflag::F4000, 0);
                        if self.w.props[i].filled {
                            assert_ne!(d.flags & iflag::CHANGED, 0);
                        }
                        self.m.push_update(guid(i));
                    }
                } else {
                    assert_eq!(self.d(i).mode, mode::CURSOR, "refused: nothing changed");
                    self.drop_cursor();
                }
            }
            Op::Remove { i } => {
                let linked = self.m.unlink(i);
                assert_eq!(self.inv.unlink(&mut self.w, uid(i)), linked, "unlink");
                if linked {
                    let d = self.dm(i);
                    d.mode = mode::GROUND;
                    d.page = page::NONE;
                }
            }
            Op::Equip { i, loc, skip } => {
                let loc = self.loc(Some(i), loc);
                self.lift(i, page::NONE);
                let check = ref_equip_check(&self.m, &self.c, &self.w, loc, Some(i), skip);
                assert_eq!(
                    equip_check(&self.inv, &self.w, c_t, OWNER, loc, Some(uid(i)), skip),
                    check,
                    "equip_check before §4.6"
                );
                let flagged = self.m.targeting_reset(&self.w);
                // Equip requires an existing item in cursor mode 4.
                let (ok, out) = if check != res::FREE {
                    (false, false)
                } else if !skip && !ref_req(&self.c, &self.w, i, false) {
                    (false, true)
                } else {
                    (true, false)
                };
                let got =
                    equip_from_cursor(&mut self.inv, &mut self.w, c_t, Some(uid(i)), loc, skip);
                assert_eq!((got.ok, got.out), (ok, out), "equip_from_cursor");
                self.reset_done(&flagged);
                if ok {
                    // Never accepts equipment that violates requirements or compatibility.
                    assert!(ref_loc_allowed(&self.c, &self.w, i, loc));
                    assert!(self
                        .m
                        .occupant(&self.c, &self.w, 0, i32::from(loc), 0)
                        .is_none());
                    self.m.place(&self.w, i, 0, i32::from(loc), 0);
                    self.m.cursor = None;
                    self.m.push_update(guid(i));
                    let d = self.d(i);
                    assert_eq!(
                        (d.mode, d.page, d.body_loc),
                        (mode::EQUIPPED, page::NONE, loc)
                    );
                    assert_ne!(d.cmd_flags & cmd::EQUIP, 0);
                    assert_eq!(d.flags & (iflag::CHANGED | iflag::F4000), iflag::CHANGED);
                } else {
                    assert_eq!(self.d(i).mode, mode::CURSOR, "refused: nothing changed");
                    self.drop_cursor();
                }
            }
            Op::Belt { i, slot } => {
                let r = self.c.rec(&self.w, i);
                let want = self.c.itype(&self.w, i).beltable != 0
                    && (r.invwidth, r.invheight) == (1, 1)
                    && slot < 16
                    && self.m.fits(&self.c, &self.w, 1, i32::from(slot), 0, 1, 1);
                let got = place_in_belt_slot(&mut self.inv, &mut self.w, c_t, uid(i), slot);
                assert_eq!(got, want, "place_in_belt_slot");
                if want {
                    self.m.place(&self.w, i, 1, i32::from(slot), 0);
                    // Mode 2 as the belt-placement callers set it.
                    self.dm(i).mode = mode::BELT;
                }
            }
            Op::BeltFree { i } => {
                let want = ref_free_belt_slot(&self.m, &self.c, &self.w, i);
                let got = d2_sim::items::inventory::free_belt_slot(&self.inv, &self.w, c_t, uid(i));
                assert_eq!(got, want, "free_belt_slot");
                if let Some(s) = got {
                    // Belt capacity follows the belts table.
                    let n = ref_numboxes(&self.m, &self.c, &self.w).unwrap();
                    assert!(s < n, "slot {s} beyond numboxes {n}");
                    assert!(place_in_belt_slot(
                        &mut self.inv,
                        &mut self.w,
                        c_t,
                        uid(i),
                        s
                    ));
                    self.m.place(&self.w, i, 1, i32::from(s), 0);
                    self.dm(i).mode = mode::BELT;
                }
            }
            Op::Compact { slot } => {
                let col = slot & 3;
                let mut want = Vec::new();
                let mut flagged = Vec::new();
                for row in 0..4u8 {
                    let from = col + 4 * row;
                    let Some(it) = self.m.occupant(&self.c, &self.w, 1, i32::from(from), 0) else {
                        continue;
                    };
                    let Some(to_row) = (0..row).find(|&r| {
                        self.m
                            .occupant(&self.c, &self.w, 1, i32::from(col + 4 * r), 0)
                            .is_none()
                    }) else {
                        continue;
                    };
                    let to = col + 4 * to_row;
                    self.m.place(&self.w, it, 1, i32::from(to), 0);
                    self.m.push_update(guid(it));
                    want.push((from, to));
                    flagged.push(it);
                }
                let got = compact_belt(&mut self.inv, &mut self.w, c_t, slot);
                assert_eq!(got, want, "compact_belt moves");
                for it in flagged {
                    let f = self.d(it).flags;
                    assert_eq!(
                        f & (iflag::F400 | iflag::CHANGED | iflag::F4000),
                        iflag::F400 | iflag::CHANGED
                    );
                }
            }
            Op::Checks { a, b, loc, skip } => self.checks(a, b, loc, skip),
            Op::Flags { i, bits } => {
                let mut f = 0;
                for (k, flag) in [
                    iflag::IDENTIFIED,
                    iflag::ETHEREAL,
                    iflag::BROKEN,
                    iflag::F4000,
                    iflag::TARGETING,
                ]
                .into_iter()
                .enumerate()
                {
                    if bits & (1 << k) != 0 {
                        f |= flag;
                    }
                }
                self.dm(i).flags ^= f;
            }
            Op::SetMode { i, m } => {
                // Only items outside the inventory change mode freely; an
                // owned item keeps the mode its placement gave it.
                if !self.m.list.contains(&i) {
                    self.dm(i).mode = if m == mode::CURSOR { mode::GROUND } else { m };
                }
            }
        }
    }

    /// A body location selector: 0–12 as given, 13–19 one of the item's
    /// own allowed locations (so accepted equips are frequent).
    fn loc(&self, i: Option<usize>, sel: u8) -> u8 {
        match i {
            Some(i) if sel >= 13 => {
                let ty = self.c.itype(&self.w, i);
                if sel & 1 == 0 {
                    ty.bodyloc1
                } else {
                    ty.bodyloc2
                }
            }
            _ => sel % 13,
        }
    }

    /// Items found by targeting reset keep no flag 0x4.
    fn reset_done(&self, flagged: &[usize]) {
        for &i in flagged {
            assert_eq!(
                self.d(i).flags & iflag::TARGETING,
                0,
                "item {i} flag 0x4 cleared"
            );
        }
    }

    fn checks(&mut self, a: Option<usize>, b: Option<usize>, loc: u8, skip: bool) {
        let loc = self.loc(a, loc);
        let (m, c, w, t) = (&self.m, &self.c, &self.w, self.c.t);
        // Equip checks with an item and with none.
        for n in [a, None] {
            let got = equip_check(&self.inv, w, t, OWNER, loc, n.map(uid), skip);
            assert_eq!(
                got,
                ref_equip_check(m, c, w, loc, n, skip),
                "equip_check {n:?} at {loc}"
            );
        }
        // Compatibility between the two hands.
        assert_eq!(
            hands_compatible(w, t, OWNER, a.map(uid), b.map(uid)),
            ref_hands(c, w, a, b),
            "hands_compatible"
        );
        if let (Some(a), Some(b)) = (a, b) {
            assert_eq!(
                stack_test(w, t, uid(a), uid(b)),
                ref_stack(c, w, a, b),
                "stack_test"
            );
        }
        if let Some(a) = a {
            let got = auto_equip_location(&self.inv, w, t, uid(a), skip);
            assert_eq!(got, ref_auto_equip(m, c, w, a, skip), "auto_equip_location");
            if let Some(l) = got {
                // Never a forbidden or occupied body location.
                assert!(l == c.itype(w, a).bodyloc1 || l == c.itype(w, a).bodyloc2);
                assert!(m.occupant(c, w, 0, i32::from(l), 0).is_none());
            }
        }
        // Ground-or-owned check on an item (or a GUID nothing has).
        let g = a.map_or(MISSING_GUID, guid);
        let it = a.map(|a| self.d(a));
        let owned = a.is_some_and(|a| m.list.contains(&a));
        let cursor = a.is_some() && m.cursor == a;
        let md = it.map(|d| d.mode);
        let r = |ok: bool| u8::from(!ok);
        assert_eq!(
            cursor_item_check(&self.inv, w, g),
            r(md == Some(mode::CURSOR) && cursor),
            "cursor item check"
        );
        assert_eq!(
            stored_item_check(&self.inv, w, g),
            r(md == Some(mode::STORED) && owned),
            "stored item check"
        );
        assert_eq!(
            stored_or_equipped_check(&self.inv, w, g),
            u8::from(matches!(md, Some(0 | 1)) && !owned),
            "stored or equipped check"
        );
        let owned_r = r(it.is_some() && (owned || cursor));
        assert_eq!(
            owned_item_check(&self.inv, w, g),
            owned_r,
            "owned item check"
        );
        assert_eq!(
            belt_item_check(&self.inv, w, g),
            u8::from(md == Some(mode::BELT) && !owned),
            "belt item check"
        );
        let ground = match md {
            None => 1,
            Some(mode::GROUND) if !w.same_act => 2,
            Some(mode::GROUND) if !w.in_range => 1,
            Some(mode::GROUND) => 0,
            Some(m) if m > mode::CURSOR => 1,
            Some(_) => owned_r,
        };
        assert_eq!(
            ground_or_owned_check(&self.inv, w, g),
            ground,
            "ground or owned check"
        );
    }

    /// Inventory lists, counts, cells and placement invariants, then equality with the
    /// model.
    fn check(&self) {
        let inv = &self.inv;
        let w = &self.w;
        // Item list and cursor, count, update list.
        let list: Vec<UnitId> = self.m.list.iter().map(|&i| uid(i)).collect();
        assert_eq!(inv.items(), &list[..], "item list (link order)");
        assert_eq!(inv.cursor(), self.m.cursor.map(uid), "cursor");
        assert_eq!(inv.count, self.m.count, "count");
        assert_eq!(inv.update_list(), &self.m.updates[..], "update list");
        let mut seen = inv.update_list().to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            inv.update_list().len(),
            "update list without duplicates"
        );
        assert_eq!(w.untargets, self.m.untargets, "targeting reset 0x3F queue");
        assert_eq!(w.room_removals, self.m.room_removals, "room removals");
        assert_eq!(w.trade_hooks, self.m.trade_hooks, "page-2 trade hooks");
        assert_eq!(w.foreign_unlinks, 0);
        // Grids: each item covers exactly its rectangle, inside the grid;
        // no cell holds an item not in the grid's list (no overlap).
        let mut in_grid = BTreeMap::new();
        for g in 0..inv.grid_count() {
            let Some(grid) = inv.grid(g) else { continue };
            assert_eq!(
                Some((grid.width, grid.height)),
                self.c.grid_size(g),
                "grid {g} size"
            );
            let want: Vec<UnitId> = self
                .m
                .list
                .iter()
                .filter(|i| self.m.at[i].0 == g)
                .map(|&i| uid(i))
                .collect();
            assert_eq!(grid.items, want, "grid {g} list");
            for &item in &grid.items {
                assert!(in_grid.insert(item, g).is_none(), "{item:?} in two grids");
                let d = w.items[&item];
                let (iw, ih) = self.c.cover(w, idx(item).unwrap(), g);
                let (ex, ey) = (
                    d.x.wrapping_add(i32::from(iw)),
                    d.y.wrapping_add(i32::from(ih)),
                );
                // An end that wraps past 2^31 passes the bound
                // test and the item occupies no cell.
                let wrapped = ex < d.x || ey < d.y;
                assert!(
                    d.x >= 0
                        && d.y >= 0
                        && (wrapped
                            || (ex <= i32::from(grid.width) && ey <= i32::from(grid.height))),
                    "{item:?} outside grid {g}"
                );
                let cells = grid.cells.iter().filter(|&&c| c == Some(item)).count();
                let want = if wrapped {
                    0
                } else {
                    usize::from(iw) * usize::from(ih)
                };
                assert_eq!(cells, want, "{item:?} cell count");
                for yy in d.y..ey {
                    for xx in d.x..ex {
                        assert_eq!(grid.cell(xx, yy), Some(item), "{item:?} cell {xx},{yy}");
                    }
                }
            }
            for y in 0..i32::from(grid.height) {
                for x in 0..i32::from(grid.width) {
                    let want = self.m.occupant(&self.c, w, g, x, y).map(uid);
                    assert_eq!(grid.cell(x, y), want, "grid {g} cell {x},{y}");
                }
            }
        }
        // Item fields and the list ⇔ owning inventory agreement.
        let owner_guid = if matches!(self.c.owner, UnitKind::Player { .. }) {
            OWNER_GUID
        } else {
            NO_GUID
        };
        for i in 0..N_ITEMS {
            let d = w.items[&uid(i)];
            match self.m.at.get(&i) {
                Some(&(g, x, y)) => {
                    assert_eq!(d.inv, Some(OWNER), "item {i} owning inventory");
                    assert_eq!(in_grid.get(&uid(i)), Some(&g), "item {i} in its grid");
                    assert_eq!((d.x, d.y), (x, y), "item {i} position");
                    assert_eq!(usize::from(d.node_grid), g + 1, "item {i} node grid");
                    let kind = match g {
                        0 if x <= 10 => node::BODY,
                        0 => node::SWAP,
                        1 => node::BELT,
                        _ => node::PAGE,
                    };
                    assert_eq!(d.node_kind, kind, "item {i} node kind");
                    assert_eq!(d.owner_guid, owner_guid, "item {i} owner");
                    if g >= 2 {
                        assert_eq!(usize::from(d.page), g - 2, "item {i} page");
                    }
                }
                None => {
                    assert_eq!(d.inv, None, "item {i} unlinked");
                    assert_eq!((d.node_grid, d.node_kind), (0, node::NONE), "item {i} node");
                    assert!(!in_grid.contains_key(&uid(i)));
                }
            }
        }
    }
}

fn run_case(dna: &[u8], ops: &[Op]) {
    let (t, w) = build(dna);
    let c = Ctx {
        t: &t,
        owner: w.owner_kind,
        expansion: w.expansion,
    };
    let inv = Inventory::new(OWNER, w.owner_kind, OWNER_GUID);
    let mut h = Harness {
        c,
        w,
        inv,
        m: Model::default(),
    };
    h.check();
    for op in ops {
        h.run(op);
        h.check();
    }
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn inventory_matches_the_model(
        dna in proptest::collection::vec(any::<u8>(), 300..1200),
        ops in proptest::collection::vec(op(), 1..60),
    ) {
        run_case(&dna, &ops);
    }
}

/// Minimized: a 1 × 1 item placed at x or y = `i32::MAX` (0x18 passes a
/// u32 payload field as i32 to cursor insertion). The bound test is
/// signed 32-bit with wrap, so x + w wrapping past 2^31 passes, and the
/// fit and cell loops run zero times: the item is placed without a cell.
/// x + w = `i32::MAX` itself does not wrap and is refused.
#[test]
fn regress_place_near_i32_max() {
    let (t, mut w) = build(&[]);
    for (x, y, ok) in [
        (i32::MAX, 0, true),
        (0, i32::MAX, true),
        (i32::MAX - 1, i32::MAX, false),
    ] {
        let mut inv = Inventory::new(OWNER, w.owner_kind, OWNER_GUID);
        w.items.get_mut(&uid(0)).unwrap().mode = mode::CURSOR;
        w.items.get_mut(&uid(0)).unwrap().page = 0;
        inv.put_cursor(&mut w, Some(uid(0)));
        let got = place_in_page(&mut inv, &mut w, &t, Some(uid(0)), x, y, false, true);
        assert_eq!(got, ok, "({x}, {y})");
        if ok {
            assert_eq!(inv.items(), &[uid(0)]);
            assert_eq!(inv.cursor(), None);
            assert!(inv
                .grid(2)
                .is_some_and(|g| g.cells.iter().all(Option::is_none)));
            assert!(inv.unlink(&mut w, uid(0)));
        } else {
            assert_eq!(inv.cursor(), Some(uid(0)));
            assert!(inv.items().is_empty() && inv.update_list().is_empty());
        }
    }
}
