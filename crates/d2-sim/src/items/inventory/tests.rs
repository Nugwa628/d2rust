//! Synthetic tests: T1–T9, B1–B5, E1–E4 and the inventory rules on a fake
//! world.

use std::collections::BTreeMap;

use d2_data::fixup::maps::EquivMatrix;

use super::equip::{equip_put, other_hand, res};
use super::grid::{fits, in_bounds};
use super::tables::{GridRec, InvItemRec, InvTypeRec};
use super::*;
use crate::units::UnitId;

// Itemtypes rows of the synthetic tables (D3 numbers where named).
const T_SHIE: i16 = 2;
const T_HELM: i16 = 37;
const T_BOOK: i16 = 18;
const T_BELT: i16 = 19;
const T_AXE: i16 = 28;
const T_SWOR: i16 = 30;
const T_BOW: i16 = 27;
const T_TPOT: i16 = 38;
const T_WEAP: i16 = 45;
const T_H2H: i16 = 67;
const T_HPOT: i16 = 76;
const T_RING: i16 = 10;
const T_BOWQ: i16 = 5;
const T_MISC: i16 = 52;
const T_SHLD: i16 = 51;
const N_TYPES: usize = 81;

// Item records of the synthetic tables.
const R_HP1: usize = 0;
const R_HP2: usize = 1;
const R_HP4: usize = 2;
const R_MP1: usize = 3;
const R_MP3: usize = 4;
const R_RVS: usize = 5;
const R_RING: usize = 6;
const R_SWORD: usize = 7;
const R_2HSWORD: usize = 8;
const R_AXE: usize = 9;
const R_SHIELD: usize = 10;
const R_BOW: usize = 11;
const R_HELM: usize = 12;
const R_SASH: usize = 13;
const R_GIRDLE: usize = 14;
const R_BOOK: usize = 15;
const R_CLAW: usize = 16;
const R_ARROWS: usize = 17;
const R_TPOT: usize = 19;
const R_ZERO: usize = 20;

fn item_rec(code: &[u8; 4], type_: i16, w: u8, h: u8) -> InvItemRec {
    InvItemRec {
        code: *code,
        type_,
        type2: -1,
        invwidth: w,
        invheight: h,
        ..Default::default()
    }
}

fn tables() -> InvTables {
    let mut grids = vec![GridRec::default(); 32];
    for (r, (x, y)) in [
        (0, (10, 4)),
        (1, (10, 4)),
        (2, (10, 4)),
        (3, (10, 4)),
        (4, (10, 4)),
        (5, (10, 10)),
        (6, (10, 4)),
        (7, (10, 4)),
        (8, (6, 4)),
        (9, (3, 4)),
        (10, (10, 4)),
        (11, (10, 4)),
        (12, (6, 8)),
        (13, (0, 0)),
        (14, (10, 4)),
        (15, (10, 4)),
    ] {
        grids[r] = GridRec {
            grid_x: x,
            grid_y: y,
        };
    }
    let belts = vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16];
    let mut items = vec![
        item_rec(b"hp1 ", T_HPOT, 1, 1),
        item_rec(b"hp2 ", T_HPOT, 1, 1),
        item_rec(b"hp4 ", T_HPOT, 1, 1),
        item_rec(b"mp1 ", T_HPOT, 1, 1),
        item_rec(b"mp3 ", T_HPOT, 1, 1),
        item_rec(b"rvs ", T_HPOT, 1, 1),
        item_rec(b"rin ", T_RING, 1, 1),
        item_rec(b"ssd ", T_SWOR, 1, 3),
        item_rec(b"2hs ", T_SWOR, 1, 4),
        item_rec(b"hax ", T_AXE, 2, 3),
        item_rec(b"buc ", T_SHIE, 2, 2),
        item_rec(b"sbw ", T_BOW, 2, 3),
        item_rec(b"cap ", T_HELM, 2, 2),
        item_rec(b"vbl ", T_BELT, 2, 1),
        item_rec(b"hbl ", T_BELT, 2, 1),
        item_rec(b"tbk ", T_BOOK, 1, 2),
        item_rec(b"ktr ", T_H2H, 1, 3),
        item_rec(b"aqv ", T_BOWQ, 1, 3),
        item_rec(b"qui ", T_MISC, 2, 3), // unused row
        item_rec(b"tpot", T_TPOT, 1, 1),
        item_rec(b"zer ", T_MISC, 0, 1),
    ];
    items[R_SASH].belt = 1;
    items[R_GIRDLE].belt = 3;
    items[R_RVS].autobelt = 1;
    items[R_SWORD].reqstr = 25;
    items[R_SWORD].reqdex = 10;
    items[R_ARROWS].stackable = 1;
    let mut itemtypes = vec![
        InvTypeRec {
            class: 7,
            ..Default::default()
        };
        N_TYPES
    ];
    let set = |v: &mut Vec<InvTypeRec>, t: i16, l1: u8, l2: u8, body: u8| {
        let r = &mut v[t as usize];
        r.bodyloc1 = l1;
        r.bodyloc2 = l2;
        r.body = body;
    };
    set(&mut itemtypes, T_SHIE, 5, 4, 1);
    set(&mut itemtypes, T_HELM, 1, 1, 1);
    set(&mut itemtypes, T_SWOR, 4, 5, 1);
    set(&mut itemtypes, T_AXE, 4, 5, 1);
    set(&mut itemtypes, T_BOW, 4, 5, 1);
    set(&mut itemtypes, T_H2H, 4, 5, 1);
    set(&mut itemtypes, T_BELT, 8, 8, 1);
    set(&mut itemtypes, T_RING, 6, 7, 1);
    set(&mut itemtypes, T_BOWQ, 5, 4, 1);
    set(&mut itemtypes, T_TPOT, 1, 1, 1);
    itemtypes[T_HPOT as usize].beltable = 1;
    itemtypes[T_BOWQ as usize].quiver = 1;
    // Equivalence: every row to itself; weapon rows to `weap`.
    let words = N_TYPES.div_ceil(32);
    let mut equiv = EquivMatrix {
        n: N_TYPES,
        words,
        bits: vec![0; N_TYPES * words],
    };
    let mut eq = |i: usize, j: usize| equiv.bits[i * words + j / 32] |= 1 << (j % 32);
    for i in 0..N_TYPES {
        eq(i, i);
    }
    for t in [T_SWOR, T_AXE, T_BOW, T_H2H] {
        eq(t as usize, T_WEAP as usize);
    }
    // `shie` is a `shld` (itemtypes 51; profile).
    eq(T_SHIE as usize, T_SHLD as usize);
    InvTables {
        grids,
        belts,
        items,
        itemtypes,
        equiv,
        books: Vec::new(),
        item_use: Default::default(),
    }
}

/// Per-item properties the fake answers for the seam.
#[derive(Clone, Debug, Default)]
struct Props {
    stats: BTreeMap<u16, i32>,
    two_handed: bool,
    one_or_two: bool,
    ammo: Option<i16>,
    active: bool,
    contribution: BTreeMap<u16, i32>,
    level_req: i32,
    req_percent: i32,
    quality: u8,
    sockets: bool,
}

#[derive(Default)]
struct Fake {
    expansion: bool,
    items: BTreeMap<UnitId, InvItem>,
    props: BTreeMap<UnitId, Props>,
    kinds: BTreeMap<UnitId, UnitKind>,
    unit_stats: BTreeMap<(UnitId, u16), i32>,
    interaction: Option<InteractionTarget>,
    pd4c: u32,
    pd50: u32,
    talking: bool,
    same_act: bool,
    in_range: bool,
    link_ok: bool,
    /// Answers "no free position on page 0".
    no_free_page0: bool,
    log: Vec<String>,
}

impl Fake {
    fn new() -> Self {
        let mut f = Fake {
            same_act: true,
            in_range: true,
            link_ok: true,
            ..Default::default()
        };
        f.kinds.insert(PLAYER, UnitKind::Player { class: 1 });
        f.unit_stats.insert((PLAYER, stat::STRENGTH), 100);
        f.unit_stats.insert((PLAYER, stat::DEXTERITY), 100);
        f.unit_stats.insert((PLAYER, stat::LEVEL), 10);
        f
    }
    fn add(&mut self, id: u32, record: usize, mode: u8) -> UnitId {
        let u = UnitId(id);
        let mut d = InvItem::new(1000 + id, record);
        d.mode = mode;
        d.flags = iflag::IDENTIFIED;
        self.items.insert(u, d);
        self.props.insert(
            u,
            Props {
                level_req: -1,
                // Normal quality (stacks only qualities 1–3).
                quality: 2,
                ..Default::default()
            },
        );
        u
    }
    fn p(&mut self, u: UnitId) -> &mut Props {
        self.props.get_mut(&u).unwrap()
    }
    fn d(&self, u: UnitId) -> InvItem {
        self.items[&u]
    }
}

const PLAYER: UnitId = UnitId(1);

impl InvWorld for Fake {
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
        self.props[&item].stats.get(&stat).copied().unwrap_or(0)
    }
    fn unlink_from(&mut self, owner: UnitId, item: UnitId) {
        self.log.push(format!("unlink_from {} {}", owner.0, item.0));
        if let Some(d) = self.items.get_mut(&item) {
            d.inv = None;
        }
    }
    fn remove_from_room(&mut self, item: UnitId) {
        self.log.push(format!("room {}", item.0));
    }
    fn clear_targetable(&mut self, item: UnitId) {
        self.log.push(format!("untargetable {}", item.0));
    }
    fn link_check(&mut self, _owner: UnitId, item: UnitId, kind: u8) -> bool {
        self.log.push(format!("link_check {} {kind}", item.0));
        self.link_ok
    }
    fn charm_relink(&mut self, _owner: UnitId, item: UnitId) {
        self.log.push(format!("charm {}", item.0));
    }
    fn active_item(&self, _owner: UnitId, item: UnitId) -> bool {
        self.props[&item].active
    }
    fn stat_refresh(&mut self, _owner: UnitId) {
        self.log.push("stat_refresh".into());
    }
    fn socket_filled(&self, item: UnitId) -> bool {
        self.props[&item].sockets
    }
    fn owner_refresh(&mut self, _owner: UnitId) {
        self.log.push("owner_refresh".into());
    }
    fn inventory_pass(&mut self, _owner: UnitId) {
        self.log.push("inventory_pass".into());
    }
    fn trade_hook(&mut self, _owner: UnitId, item: UnitId) {
        self.log.push(format!("trade {}", item.0));
    }
    fn weapon_in_use_update(&mut self, _unit: UnitId) {
        self.log.push("weapon_in_use".into());
    }
    fn stat_link(&mut self, _unit: UnitId, item: UnitId) {
        self.log.push(format!("stat_link {}", item.0));
    }
    fn weapon_bookkeeping(&mut self, _unit: UnitId, item: UnitId) {
        self.log.push(format!("weapon_bookkeeping {}", item.0));
    }
    fn unit_kind(&self, unit: UnitId) -> Option<UnitKind> {
        self.kinds.get(&unit).copied()
    }
    fn unit_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.unit_stats.get(&(unit, stat)).copied().unwrap_or(0)
    }
    fn req_percent(&self, item: UnitId) -> i32 {
        self.props[&item].req_percent
    }
    fn item_active_on(&self, item: UnitId, _unit: UnitId) -> bool {
        self.props[&item].active
    }
    fn own_contribution(&self, item: UnitId, _unit: UnitId, stat: u16) -> i32 {
        self.props[&item]
            .contribution
            .get(&stat)
            .copied()
            .unwrap_or(0)
    }
    fn level_requirement(&self, item: UnitId, _unit: UnitId) -> i32 {
        self.props[&item].level_req
    }
    fn two_handed(&self, item: UnitId) -> bool {
        self.props[&item].two_handed
    }
    fn one_or_two_handed(&self, _unit: UnitId, item: UnitId) -> bool {
        self.props[&item].one_or_two
    }
    fn ammo_type(&self, item: UnitId) -> Option<i16> {
        self.props[&item].ammo
    }
    fn fits_free_page0(&self, _inv: &Inventory, _item: UnitId) -> bool {
        !self.no_free_page0
    }
    fn quality(&self, item: UnitId) -> u8 {
        self.props[&item].quality
    }
    fn stack_file_index(&self, _item: UnitId) -> i32 {
        -1
    }
    fn has_sockets(&self, item: UnitId) -> bool {
        self.props[&item].sockets
    }
    fn has_allowed_location(&self, _item: UnitId) -> bool {
        true
    }
    fn quiver_kind(&self, item: UnitId) -> bool {
        self.items[&item].record == R_ARROWS
    }
    /// The unit's type (player 0, monster 1, object 2,
    /// item 4), 6 for a unit with no kind.
    fn targeting_probe(&self, unit: UnitId) -> u32 {
        match self.kinds.get(&unit) {
            Some(UnitKind::Player { .. }) => 0,
            Some(UnitKind::Monster { .. }) => 1,
            Some(UnitKind::Object { .. }) => 2,
            Some(UnitKind::Item) => 4,
            Some(UnitKind::Other) => 3,
            None => 6,
        }
    }
    fn queue_untarget(&mut self, _player: UnitId, item_guid: u32) {
        self.log.push(format!("0x3F {item_guid}"));
    }
    fn interaction(&self, _player: UnitId) -> InteractionTarget {
        self.interaction.unwrap_or(InteractionTarget::None)
    }
    fn clear_interaction(&mut self, _player: UnitId) {
        self.interaction = None;
        self.log.push("clear_interaction".into());
    }
    fn player_data_4c(&self, _player: UnitId) -> u32 {
        self.pd4c
    }
    fn player_data_50(&self, _player: UnitId) -> u32 {
        self.pd50
    }
    fn npc_talking(&self, _npc: UnitId, _player: UnitId) -> bool {
        self.talking
    }
    fn player_trade_gate(&self, _player: UnitId) -> Option<bool> {
        None
    }
    fn same_act(&self, _player: UnitId, _item: UnitId) -> bool {
        self.same_act
    }
    fn within_range(&self, _player: UnitId, _item: UnitId, range: i32) -> bool {
        assert_eq!(range, 10);
        self.in_range
    }
}

fn player_inv() -> Inventory {
    Inventory::new(PLAYER, UnitKind::Player { class: 1 }, 77)
}

/// A grid with occupied rectangles (x, y, w, h).
fn grid_with(w: u8, h: u8, rects: &[(i32, i32, u8, u8)]) -> Grid {
    let mut g = Grid::new(w, h);
    for (i, &(x, y, rw, rh)) in rects.iter().enumerate() {
        g.set_rect(x, y, rw, rh, Some(UnitId(900 + i as u32)));
    }
    g
}

fn full_except(w: u8, h: u8, free: Option<(i32, i32)>) -> Grid {
    let mut g = grid_with(w, h, &[(0, 0, w, h)]);
    if let Some((x, y)) = free {
        g.cells[y as usize * usize::from(w) + x as usize] = None;
    }
    g
}

// ---------------------------------------------------------------

#[test]
fn grid_record_by_page() {
    let p = |c| UnitKind::Player { class: c };
    assert_eq!(grid_record(p(1), 1, false), Some(6));
    assert_eq!(grid_record(p(1), 2, false), Some(7));
    assert_eq!(grid_record(p(1), 3, false), Some(9));
    assert_eq!(grid_record(p(1), 4, false), Some(8));
    assert_eq!(grid_record(p(1), 4, true), Some(12));
    for c in 0..5 {
        assert_eq!(grid_record(p(c), 0, false), Some(usize::from(c)));
    }
    assert_eq!(grid_record(p(5), 0, false), Some(14));
    assert_eq!(grid_record(p(6), 0, true), Some(15));
    assert_eq!(grid_record(p(7), 0, true), None);
    assert_eq!(
        grid_record(UnitKind::Monster { class: 3 }, 9, false),
        Some(5)
    );
    assert_eq!(
        grid_record(UnitKind::Object { class: 0x152 }, 0, false),
        Some(10)
    );
    assert_eq!(
        grid_record(UnitKind::Object { class: 0x153 }, 0, false),
        Some(11)
    );
    assert_eq!(
        grid_record(UnitKind::Object { class: 0x154 }, 0, false),
        None
    );
    let t = tables();
    assert_eq!(page_grid_size(&t, p(1), 0, false), Some((10, 4)));
    assert_eq!(page_grid_size(&t, p(1), 3, false), Some((3, 4)));
    assert_eq!(page_grid_size(&t, p(1), 4, false), Some((6, 4)));
    assert_eq!(page_grid_size(&t, p(1), 4, true), Some((6, 8)));
}

#[test]
fn grids_created_on_first_use_with_fixed_size() {
    let mut inv = player_inv();
    assert!(inv.grid(0).is_none());
    assert!(inv.grid_or_create(2, 10, 4).is_some());
    assert_eq!(inv.grid_count(), 3);
    assert!(inv.grid(2).unwrap().cells.iter().all(Option::is_none));
    assert!(inv.grid_or_create(2, 10, 4).is_some());
    assert!(inv.grid_or_create(2, 6, 4).is_none());
    assert_eq!(BODY_GRID, (13, 1));
    assert_eq!(BELT_GRID, (16, 1));
}

#[test]
fn link_order_and_unlink() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    let b = w.add(11, R_RING, mode::CURSOR);
    let c = w.add(12, R_RING, mode::CURSOR);
    for (i, x) in [(b, 0), (a, 1), (c, 2)] {
        assert!(place_at_page(&mut inv, &mut w, &t, i, 0, x, 0));
    }
    assert_eq!(inv.items(), &[b, a, c]);
    assert_eq!(inv.grid(2).unwrap().items, vec![b, a, c]);
    assert_eq!(inv.count, 3);
    // Unlink clears cells, drops the count, clears a matching weapon GUID
    // and zeroes the node fields.
    inv.weapon_guid = w.d(a).guid;
    assert!(inv.unlink(&mut w, a));
    assert_eq!(inv.item_at(2, 1, 0), None);
    assert_eq!(inv.count, 2);
    assert_eq!(inv.weapon_guid, NO_GUID);
    assert_eq!(
        (w.d(a).node_grid, w.d(a).node_kind, w.d(a).inv),
        (0, 0, None)
    );
    assert_eq!(inv.items(), &[b, c]);
    // Re-linking appends at the tail.
    assert!(place_at_page(&mut inv, &mut w, &t, a, 0, 5, 0));
    assert_eq!(inv.items(), &[b, c, a]);
    // The cursor item: unlink clears the cursor, the count is kept.
    inv.set_cursor(Some(c));
    let n = inv.count;
    assert!(inv.unlink(&mut w, c));
    assert_eq!(inv.cursor(), None);
    assert_eq!(inv.count, n);
    assert!(!inv.unlink(&mut w, c));
}

#[test]
fn update_list_appends_once() {
    let mut inv = player_inv();
    inv.push_update(5);
    inv.push_update(3);
    inv.push_update(5);
    assert_eq!(inv.update_list(), &[5, 3]);
    assert!(inv.is_listed(3));
    assert_eq!(inv.take_updates(), vec![5, 3]);
    assert!(inv.update_list().is_empty());
}

// ---------------------------------------------------------------

#[test]
fn fit_test_and_bounds() {
    let g = grid_with(10, 4, &[(0, 0, 2, 3)]);
    assert!(!fits(&g, 1, 0, 2, 2));
    assert!(fits(&g, 2, 0, 2, 2));
    assert!(fits(&g, 0, 3, 2, 1));
    assert!(in_bounds(&g, 8, 2, 2, 2));
    assert!(!in_bounds(&g, 9, 2, 2, 2));
    assert!(!in_bounds(&g, -1, 0, 1, 1));
    assert!(!in_bounds(&g, 0, 3, 1, 2));
}

#[test]
fn search_vectors_t1_to_t9() {
    let e = Grid::new(10, 4);
    let s = |g: &Grid, w, h| (search(g, w, h, true), search(g, w, h, false));
    // T1.
    assert_eq!(s(&e, 1, 1), (Some((9, 3)), Some((9, 0))));
    // T2.
    assert_eq!(s(&e, 2, 3), (Some((0, 0)), Some((0, 0))));
    assert_eq!(s(&e, 2, 2), (Some((0, 0)), Some((0, 0))));
    // T3.
    let g3 = grid_with(10, 4, &[(0, 0, 2, 3)]);
    assert_eq!(s(&g3, 1, 1), (Some((0, 3)), Some((9, 0))));
    assert_eq!(weight(&g3, 0, 3, 1, 1), 3);
    // T4.
    assert_eq!(s(&g3, 2, 2), (Some((2, 0)), Some((2, 0))));
    // T5.
    let g5 = grid_with(10, 4, &[(9, 3, 1, 1)]);
    assert_eq!(s(&g5, 1, 1), (Some((9, 2)), Some((9, 0))));
    assert_eq!(s(&g5, 1, 2), (Some((0, 0)), Some((0, 0))));
    // T6.
    let cube = Grid::new(3, 4);
    assert_eq!(s(&cube, 1, 1), (Some((2, 3)), Some((2, 0))));
    assert_eq!(s(&cube, 2, 4), (Some((0, 0)), Some((0, 0))));
    // T7.
    let g7 = grid_with(6, 8, &[(0, 0, 2, 4)]);
    assert_eq!(s(&g7, 2, 4), (Some((0, 4)), Some((0, 4))));
    assert_eq!(weight(&g7, 0, 4, 2, 4), 8);
    // T8.
    let g8 = full_except(10, 4, Some((4, 1)));
    assert_eq!(s(&g8, 1, 1), (Some((4, 1)), Some((4, 1))));
    assert_eq!(weight(&g8, 4, 1, 1, 1), 255);
    let full = full_except(10, 4, None);
    assert_eq!(s(&full, 1, 1), (None, None));
    // T9.
    assert_eq!(weight(&e, 5, 1, 1, 1), 0);
    assert_eq!(weight(&e, 0, 0, 1, 1), 2);
    // Zero size never fits.
    assert_eq!(s(&e, 0, 1), (None, None));
}

// Weighted searches keep the first strictly greater weight: an empty
// grid's four corners all weigh 2, so the first corner in search order
// wins (h = 1: x descending, then y descending).
#[test]
fn weighted_search_keeps_first_best() {
    let e = Grid::new(10, 4);
    for (x, y) in [(0, 0), (9, 0), (0, 3), (9, 3)] {
        assert_eq!(weight(&e, x, y, 1, 1), 2);
    }
    assert_eq!(search(&e, 1, 1, true), Some((9, 3)));
    // h ≥ 2: y ascending, then x ascending.
    assert_eq!(search(&e, 1, 2, true), Some((0, 0)));
}

#[test]
fn place_at_position() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_AXE, mode::GROUND);
    // Out of bounds and overlap fail without change.
    assert!(!place_at_page(&mut inv, &mut w, &t, a, 0, 9, 0));
    assert!(!place_at_page(&mut inv, &mut w, &t, a, 0, -1, 0));
    assert!(w.log.is_empty());
    assert!(place_at_page(&mut inv, &mut w, &t, a, 3, 1, 1));
    let d = w.d(a);
    assert_eq!(
        (d.x, d.y, d.page, d.node_grid, d.node_kind),
        (1, 1, 3, 6, node::PAGE)
    );
    assert_eq!(d.owner_guid, 77);
    assert_eq!(d.inv, Some(PLAYER));
    assert_eq!(w.log, vec!["room 10".to_string()]);
    for (x, y) in [(1, 1), (2, 1), (1, 3), (2, 3)] {
        assert_eq!(inv.item_at(5, x, y), Some(a));
    }
    let b = w.add(11, R_RING, mode::CURSOR);
    assert!(!place_at_page(&mut inv, &mut w, &t, b, 3, 2, 2));
    // Leaves the mode to its callers; moving inside the same
    // inventory unlinks first.
    assert_eq!(w.d(a).mode, mode::GROUND);
    w.items.get_mut(&a).unwrap().mode = mode::STORED;
    w.log.clear();
    assert!(place_at_page(&mut inv, &mut w, &t, a, 0, 0, 0));
    assert_eq!(inv.item_at(5, 1, 1), None);
    assert_eq!(inv.items(), &[a]);
    assert_eq!(inv.count, 1);
    assert_eq!(w.d(a).page, 0);
    // An item of another inventory is unlinked there.
    w.items.get_mut(&b).unwrap().inv = Some(UnitId(50));
    assert!(place_at_page(&mut inv, &mut w, &t, b, 0, 9, 3));
    assert_eq!(w.log, vec!["unlink_from 50 11".to_string()]);
    // Zero size never places.
    let z = w.add(12, R_ZERO, mode::CURSOR);
    assert!(!place_at_page(&mut inv, &mut w, &t, z, 0, 5, 0));
    // Body and belt grids: node kinds 3 / 4 and 2, page untouched.
    let h = w.add(13, R_HELM, mode::CURSOR);
    assert!(place_at_body(&mut inv, &mut w, h, body::HEAD));
    assert_eq!((w.d(h).node_kind, w.d(h).page), (node::BODY, page::NONE));
    let s = w.add(14, R_SWORD, mode::CURSOR);
    assert!(place_at_body(&mut inv, &mut w, s, body::SWAP_LEFT));
    assert_eq!(w.d(s).node_kind, node::SWAP);
    let p = w.add(15, R_HP1, mode::CURSOR);
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, p, 3));
    assert_eq!((w.d(p).node_kind, w.d(p).node_grid), (node::BELT, 2));
    // A non-player owner writes −1 as the item owner.
    let mut minv = Inventory::new(UnitId(60), UnitKind::Monster { class: 1 }, 5);
    let r = w.add(16, R_RING, mode::CURSOR);
    assert!(place_at_page(&mut minv, &mut w, &t, r, 0, 9, 9));
    assert_eq!(w.d(r).owner_guid, NO_GUID);
}

#[test]
fn place_in_page_steps() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    // Step 2: missing or not on the cursor.
    assert!(!place_in_page(
        &mut inv, &mut w, &t, None, 0, 0, false, true
    ));
    let a = w.add(10, R_RING, mode::STORED);
    assert!(!place_in_page(
        &mut inv,
        &mut w,
        &t,
        Some(a),
        0,
        0,
        false,
        true
    ));
    // Steps 4, 7, 8: negative coordinates become 0; cursor cleared;
    // mode 0; command flag 0x2, item flag 0x1 when socket-filled, 0x4000
    // cleared; update list.
    w.items.get_mut(&a).unwrap().mode = mode::CURSOR;
    w.items.get_mut(&a).unwrap().page = 0;
    w.items.get_mut(&a).unwrap().flags |= iflag::F4000;
    w.p(a).sockets = true;
    inv.set_cursor(Some(a));
    assert!(place_in_page(
        &mut inv,
        &mut w,
        &t,
        Some(a),
        -3,
        -1,
        false,
        true
    ));
    let d = w.d(a);
    assert_eq!((d.x, d.y, d.mode), (0, 0, mode::STORED));
    assert_eq!(inv.cursor(), None);
    assert_eq!(d.cmd_flags, cmd::PUT_IN_CONTAINER);
    assert_eq!(d.flags & (iflag::CHANGED | iflag::F4000), iflag::CHANGED);
    assert_eq!(inv.update_list(), &[d.guid]);
    // Without "send": no flags, no update list entry.
    let b = w.add(11, R_RING, mode::CURSOR);
    w.items.get_mut(&b).unwrap().page = 0;
    assert!(place_in_page(
        &mut inv,
        &mut w,
        &t,
        Some(b),
        0,
        0,
        true,
        false
    ));
    assert_eq!((w.d(b).x, w.d(b).y), (9, 3));
    assert_eq!(w.d(b).cmd_flags, 0);
    assert_eq!(inv.update_list().len(), 1);
    // Failure leaves the item unchanged.
    let c = w.add(12, R_RING, mode::CURSOR);
    w.items.get_mut(&c).unwrap().page = 0;
    assert!(!place_in_page(
        &mut inv,
        &mut w,
        &t,
        Some(c),
        9,
        3,
        false,
        true
    ));
    assert_eq!(w.d(c).mode, mode::CURSOR);
}

#[test]
fn place_in_page_cursor_charm_and_passes() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    // Page 1: step 7 keeps the cursor, but the placement's unlink of the
    // cursor item (+0x5C is this inventory) already
    // cleared it.
    let a = w.add(10, R_RING, mode::CURSOR);
    w.items.get_mut(&a).unwrap().page = page::TRADE1;
    inv.set_cursor(Some(a));
    assert!(place_in_page_from_cursor(
        &mut inv,
        &mut w,
        &t,
        Some(a),
        0,
        0,
        false,
        false
    ));
    assert_eq!(inv.cursor(), None);
    // Page 4: no charm re-link; an active item refreshes stats and runs
    // the inventory pass with an owner refresh.
    let b = w.add(11, R_RING, mode::CURSOR);
    w.items.get_mut(&b).unwrap().page = page::STASH;
    w.p(b).active = true;
    w.log.clear();
    assert!(place_in_page_from_cursor(
        &mut inv,
        &mut w,
        &t,
        Some(b),
        0,
        0,
        false,
        false
    ));
    assert_eq!(
        w.log,
        vec![
            "link_check 11 1",
            "untargetable 11",
            "stat_refresh",
            "inventory_pass",
            "owner_refresh"
        ]
    );
    // Page 0: charm re-link before the targetable flag.
    let c = w.add(12, R_RING, mode::CURSOR);
    w.items.get_mut(&c).unwrap().page = 0;
    w.log.clear();
    assert!(place_in_page_from_cursor(
        &mut inv,
        &mut w,
        &t,
        Some(c),
        0,
        0,
        false,
        false
    ));
    assert_eq!(
        w.log,
        vec!["link_check 12 1", "charm 12", "untargetable 12"]
    );
    // Page 2: trade hook after step 8, before step 9.
    let e = w.add(13, R_RING, mode::CURSOR);
    w.items.get_mut(&e).unwrap().page = page::TRADE2;
    w.log.clear();
    assert!(place_in_page_from_cursor(
        &mut inv,
        &mut w,
        &t,
        Some(e),
        0,
        0,
        false,
        false
    ));
    assert_eq!(
        w.log,
        vec!["link_check 13 1", "charm 13", "untargetable 13", "trade 13"]
    );
}

#[test]
fn place_in_page_resets_targeting_first() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    w.items.get_mut(&a).unwrap().page = 0;
    assert!(place_at_page(&mut inv, &mut w, &t, a, 0, 0, 0));
    w.items.get_mut(&a).unwrap().flags |= iflag::TARGETING;
    w.log.clear();
    assert!(!place_in_page(
        &mut inv, &mut w, &t, None, 0, 0, false, true
    ));
    assert_eq!(w.log, vec![format!("0x3F {}", w.d(a).guid)]);
}

// ---------------------------------------------------------------

fn belt_with(w: &mut Fake, inv: &mut Inventory, belt: Option<usize>) {
    if let Some(r) = belt {
        let b = w.add(500, r, mode::CURSOR);
        assert!(place_at_body(inv, w, b, body::BELT));
    }
}

fn put(w: &mut Fake, inv: &mut Inventory, t: &InvTables, id: u32, r: usize, slot: u8) -> UnitId {
    let u = w.add(id, r, mode::CURSOR);
    assert!(place_in_belt_slot(inv, w, t, u, slot));
    u
}

#[test]
fn belt_type_and_boxes() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    assert_eq!(belt_type(&inv, &w, &t), 2);
    assert_eq!(belt_numboxes(&inv, &w, &t), Some(4));
    belt_with(&mut w, &mut inv, Some(R_GIRDLE));
    assert_eq!(belt_type(&inv, &w, &t), 3);
    assert_eq!(belt_numboxes(&inv, &w, &t), Some(16));
}

#[test]
fn belt_vectors_b1_to_b4() {
    let t = tables();
    // B1.
    let mut w = Fake::new();
    let mut inv = player_inv();
    put(&mut w, &mut inv, &t, 10, R_HP1, 0);
    let it = w.add(11, R_HP2, mode::CURSOR);
    assert_eq!(free_belt_slot(&inv, &w, &t, it), None);
    w.items.get_mut(&it).unwrap().record = R_RVS; // autobelt ≠ 0
                                                  // rvs is not similar to hp1; with autobelt the first empty bottom slot.
    assert_eq!(free_belt_slot(&inv, &w, &t, it), Some(1));
    // B2.
    let mut w = Fake::new();
    let mut inv = player_inv();
    belt_with(&mut w, &mut inv, Some(R_SASH));
    put(&mut w, &mut inv, &t, 10, R_MP3, 1);
    let it = w.add(11, R_MP1, mode::CURSOR);
    assert_eq!(free_belt_slot(&inv, &w, &t, it), Some(5));
    // B3.
    let mut w = Fake::new();
    let mut inv = player_inv();
    belt_with(&mut w, &mut inv, Some(R_GIRDLE));
    for (i, s) in [0u8, 4, 8].iter().enumerate() {
        put(&mut w, &mut inv, &t, 10 + i as u32, R_HP1, *s);
    }
    let it = w.add(20, R_HP4, mode::CURSOR);
    assert_eq!(free_belt_slot(&inv, &w, &t, it), Some(12));
    // B4.
    let mut w = Fake::new();
    let inv = player_inv();
    let it = w.add(11, R_RVS, mode::CURSOR);
    assert_eq!(free_belt_slot(&inv, &w, &t, it), Some(0));
    // Not beltable / not 1 × 1 → none.
    let r = w.add(12, R_RING, mode::CURSOR);
    assert_eq!(free_belt_slot(&inv, &w, &t, r), None);
}

#[test]
fn belt_slots_and_similar() {
    let t = tables();
    assert!(similar(&t, R_HP1, R_HP4));
    assert!(similar(&t, R_MP1, R_MP3));
    assert!(similar(&t, R_RVS, R_RVS));
    assert!(!similar(&t, R_HP1, R_MP1));
    assert!(!similar(&t, R_HP1, R_RVS));
    assert!(similar(&t, R_RING, R_RING));
    // Slot s = column (s & 3) + 4 × row; a 4-box belt uses slots 0..3:
    // a similar item in column 0 cannot stack upward.
    let mut w = Fake::new();
    let mut inv = player_inv();
    put(&mut w, &mut inv, &t, 10, R_HP1, 0);
    put(&mut w, &mut inv, &t, 11, R_MP1, 1);
    let it = w.add(12, R_MP3, mode::CURSOR);
    assert_eq!(free_belt_slot(&inv, &w, &t, it), None);
    belt_with(&mut w, &mut inv, Some(R_SASH));
    assert_eq!(free_belt_slot(&inv, &w, &t, it), Some(5));
}

#[test]
fn beltable_gate_and_slot_placement() {
    let t = tables();
    assert!(beltable(&t, R_HP1));
    assert!(!beltable(&t, R_RING));
    let mut w = Fake::new();
    let mut inv = player_inv();
    let r = w.add(10, R_RING, mode::CURSOR);
    assert!(auto_belt_gate(&inv, &w, &t, r));
    assert!(!place_in_belt_slot(&mut inv, &mut w, &t, r, 0));
    let p = w.add(11, R_HP1, mode::CURSOR);
    assert!(!place_in_belt_slot(&mut inv, &mut w, &t, p, 16));
    // No numboxes check: slot 15 with the default 4-box belt.
    assert!(place_in_belt_slot(&mut inv, &mut w, &t, p, 15));
    assert_eq!(inv.belt_item(15), Some(p));
    let q = w.add(12, R_HP2, mode::CURSOR);
    assert!(!place_in_belt_slot(&mut inv, &mut w, &t, q, 15));
}

#[test]
fn belt_compaction_b5() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = put(&mut w, &mut inv, &t, 10, R_HP1, 1);
    let b = put(&mut w, &mut inv, &t, 11, R_HP1, 9);
    let c = put(&mut w, &mut inv, &t, 12, R_HP1, 13);
    w.items.get_mut(&b).unwrap().flags |= iflag::F4000;
    assert_eq!(compact_belt(&mut inv, &mut w, &t, 5), vec![(9, 5), (13, 9)]);
    assert_eq!(inv.belt_item(1), Some(a));
    assert_eq!(inv.belt_item(5), Some(b));
    assert_eq!(inv.belt_item(9), Some(c));
    assert_eq!(inv.belt_item(13), None);
    for u in [b, c] {
        let f = w.d(u).flags;
        assert_eq!(
            f & (iflag::F400 | iflag::CHANGED | iflag::F4000),
            iflag::F400 | iflag::CHANGED
        );
    }
    assert_eq!(w.d(a).flags & iflag::F400, 0);
    assert_eq!(inv.update_list(), &[w.d(b).guid, w.d(c).guid]);
    // Re-placement re-links: moved items go to the tail of the item list.
    assert_eq!(inv.items(), &[a, b, c]);
}

// ---------------------------------------------------------------

#[test]
fn body_location_compatibility() {
    let t = tables();
    assert!(body_location_allowed(&t, R_SWORD, 4));
    assert!(body_location_allowed(&t, R_SWORD, 5));
    assert!(body_location_allowed(&t, R_SWORD, 11));
    assert!(body_location_allowed(&t, R_SHIELD, 12));
    assert!(!body_location_allowed(&t, R_SWORD, 1));
    assert!(body_location_allowed(&t, R_HELM, 1));
    assert!(!body_location_allowed(&t, R_HELM, 11));
    assert!(!body_location_allowed(&t, R_RING, 12));
}

fn req_setup() -> (InvTables, Fake, UnitId) {
    let t = tables();
    let mut w = Fake::new();
    let s = w.add(10, R_SWORD, mode::CURSOR); // reqstr 25, reqdex 10
    (t, w, s)
}

#[test]
fn requirements_stats() {
    let (t, mut w, s) = req_setup();
    assert!(!requirements_met(&w, &t, None, PLAYER, false));
    let bad = w.add(11, 999, mode::CURSOR);
    assert!(!requirements_met(&w, &t, Some(bad), PLAYER, false));
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    // Strength below the requirement, and below 1.
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 24);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 25);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    // Requirement percent: +50 % of 25 = 12 → 37.
    w.p(s).req_percent = 50;
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 37);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    // Ethereal: both bonuses −10.
    w.p(s).req_percent = 0;
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 15);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    w.items.get_mut(&s).unwrap().flags |= iflag::ETHEREAL;
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 0);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 1);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    // Equipping an active item: its own contribution is subtracted.
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 40);
    w.p(s).active = true;
    w.p(s).contribution.insert(stat::STRENGTH, 26);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, true));
    w.p(s).contribution.insert(stat::STRENGTH, 20);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, true));
    w.p(s).contribution.insert(stat::DEXTERITY, 1);
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, true));
    w.p(s).contribution.clear();
    // Level.
    w.p(s).level_req = 11;
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    w.p(s).level_req = 10;
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
}

#[test]
fn requirements_q1_dexterity_has_no_link_test() {
    let (mut t, mut w, s) = req_setup();
    // Q1: reqdex 50, dex 55, item not linked, a socketed +10 dex filler.
    t.items[R_SWORD].reqdex = 50;
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 100);
    w.unit_stats.insert((PLAYER, stat::DEXTERITY), 55);
    w.p(s).active = false;
    w.p(s).contribution.insert(stat::DEXTERITY, 10);
    // Dexterity: no link test, 55 − 10 < 50 → fail (equipping only).
    assert!(requirements_met(&w, &t, Some(s), PLAYER, false));
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, true));
    // Strength (reqstr 50, str 55, filler str 10): unlinked → no
    // subtraction → passes.
    t.items[R_SWORD].reqdex = 0;
    t.items[R_SWORD].reqstr = 50;
    w.unit_stats.insert((PLAYER, stat::STRENGTH), 55);
    w.p(s).contribution.clear();
    w.p(s).contribution.insert(stat::STRENGTH, 10);
    assert!(requirements_met(&w, &t, Some(s), PLAYER, true));
}

#[test]
fn requirements_identified_book_class() {
    let (mut t, mut w, s) = req_setup();
    w.items.get_mut(&s).unwrap().flags = 0;
    assert!(!requirements_met(&w, &t, Some(s), PLAYER, false));
    let b = w.add(11, R_BOOK, mode::CURSOR);
    assert!(!requirements_met(&w, &t, Some(b), PLAYER, false));
    w.p(b).stats.insert(stat::QUANTITY, 1);
    assert!(requirements_met(&w, &t, Some(b), PLAYER, false));
    // Class-restricted type (class 4, barbarian).
    t.itemtypes[T_AXE as usize].class = 4;
    let a = w.add(12, R_AXE, mode::CURSOR);
    assert!(!requirements_met(&w, &t, Some(a), PLAYER, false));
    w.kinds.insert(PLAYER, UnitKind::Player { class: 4 });
    assert!(requirements_met(&w, &t, Some(a), PLAYER, false));
    let merc = UnitId(2);
    for (class, ok) in [(0x230, true), (0x231, true), (0x232, false)] {
        w.kinds.insert(merc, UnitKind::Monster { class });
        w.unit_stats.insert((merc, stat::STRENGTH), 100);
        w.unit_stats.insert((merc, stat::DEXTERITY), 100);
        assert_eq!(requirements_met(&w, &t, Some(a), merc, false), ok);
    }
    t.itemtypes[T_AXE as usize].class = 1;
    assert!(!requirements_met(&w, &t, Some(a), merc, false));
}

struct Hands {
    t: InvTables,
    w: Fake,
    inv: Inventory,
}

fn hands() -> Hands {
    Hands {
        t: tables(),
        w: Fake::new(),
        inv: player_inv(),
    }
}

impl Hands {
    fn equip(&mut self, id: u32, r: usize, loc: u8) -> UnitId {
        let u = self.w.add(id, r, mode::EQUIPPED);
        assert!(place_at_body(&mut self.inv, &mut self.w, u, loc));
        u
    }
    fn check(&self, loc: u8, n: Option<UnitId>) -> u8 {
        equip_check(&self.inv, &self.w, &self.t, PLAYER, loc, n, false)
    }
}

#[test]
fn equip_check_simple_locations_e1() {
    let mut h = hands();
    let n = h.w.add(10, R_HELM, mode::CURSOR);
    // E1.
    assert_eq!(h.check(1, Some(n)), res::FREE);
    assert_eq!(h.check(1, None), res::NO);
    h.equip(11, R_HELM, 1);
    assert_eq!(h.check(1, Some(n)), res::SWAP);
    assert_eq!(h.check(1, None), res::REMOVABLE);
    // Wrong location; failing requirements unless skipped; location 0.
    assert_eq!(h.check(2, Some(n)), res::NO);
    h.w.items.get_mut(&n).unwrap().flags = 0;
    assert_eq!(h.check(1, Some(n)), res::NO);
    assert_eq!(
        equip_check(&h.inv, &h.w, &h.t, PLAYER, 1, Some(n), true),
        res::SWAP
    );
    assert_eq!(h.check(0, None), res::NO);
}

#[test]
fn equip_check_hands_e2_e3_e4() {
    // E2: sorceress, two-handed sword (not 1-or-2) over a shield.
    let mut h = hands();
    h.equip(10, R_SHIELD, 5);
    let n = h.w.add(11, R_2HSWORD, mode::CURSOR);
    h.w.p(n).two_handed = true;
    assert_eq!(h.check(4, Some(n)), res::OTHER_HAND_BLOCKS);
    // E3: barbarian, one-handed sword, an axe in the left hand.
    let mut h = hands();
    h.w.kinds.insert(PLAYER, UnitKind::Player { class: 4 });
    h.equip(10, R_AXE, 5);
    let n = h.w.add(11, R_SWORD, mode::CURSOR);
    assert_eq!(h.check(4, Some(n)), res::FREE);
    // Other players cannot dual-wield.
    h.w.kinds.insert(PLAYER, UnitKind::Player { class: 1 });
    assert_eq!(h.check(4, Some(n)), res::OTHER_HAND_BLOCKS);
    // E4: no N, right empty, left holds a two-handed bow.
    let mut h = hands();
    let b = h.equip(10, R_BOW, 5);
    h.w.p(b).two_handed = true;
    assert_eq!(h.check(4, None), res::OTHER_HAND_TWO_HANDED);
    h.w.p(b).two_handed = false;
    assert_eq!(h.check(4, None), res::NO);
    // Occupied target: stack 6, swap 5, other to page 7, else 0.
    let mut h = hands();
    let q = h.equip(10, R_ARROWS, 4);
    let n = h.w.add(11, R_ARROWS, mode::CURSOR);
    assert_eq!(h.check(4, Some(n)), res::STACK);
    let mut h = hands();
    h.equip(10, R_SWORD, 4);
    let n = h.w.add(11, R_SWORD, mode::CURSOR);
    assert_eq!(h.check(4, Some(n)), res::SWAP);
    let x = h.equip(12, R_SHIELD, 5);
    assert_eq!(h.check(4, Some(n)), res::SWAP);
    h.w.p(n).two_handed = true;
    assert_eq!(h.check(4, Some(n)), res::SWAP_OTHER_TO_PAGE);
    let _ = (q, x);
    // Swap locations pair 11 / 12; no N, target present → 3.
    let mut h = hands();
    h.equip(10, R_SWORD, 11);
    assert_eq!(h.check(11, None), res::REMOVABLE);
    assert_eq!(other_hand(12), Some(11));
    let n = h.w.add(11, R_SHIELD, mode::CURSOR);
    assert_eq!(h.check(12, Some(n)), res::FREE);
}

#[test]
fn hands_compatible_rules() {
    let t = tables();
    let mut w = Fake::new();
    let sword = w.add(10, R_SWORD, mode::CURSOR);
    let sword2 = w.add(11, R_SWORD, mode::CURSOR);
    let shield = w.add(12, R_SHIELD, mode::CURSOR);
    let bow = w.add(13, R_BOW, mode::CURSOR);
    let quiver = w.add(14, R_ARROWS, mode::CURSOR);
    let claw = w.add(15, R_CLAW, mode::CURSOR);
    let claw2 = w.add(16, R_CLAW, mode::CURSOR);
    let helm = w.add(17, R_HELM, mode::CURSOR);
    let hc = |w: &Fake, u, a, b| hands_compatible(w, &t, u, Some(a), Some(b));
    // 1.
    assert!(hands_compatible(&w, &t, PLAYER, None, Some(sword)));
    // 2: bow and its ammo type, either order.
    w.p(bow).ammo = Some(T_BOWQ);
    w.p(bow).two_handed = true;
    assert!(hc(&w, PLAYER, bow, quiver));
    assert!(hc(&w, PLAYER, quiver, bow));
    // 3: a quiver type with anything else.
    assert!(!hc(&w, PLAYER, sword, quiver));
    // 4: two-handed and not one-or-two-handed.
    assert!(!hc(&w, PLAYER, bow, shield));
    w.p(bow).one_or_two = true;
    assert!(hc(&w, PLAYER, bow, shield));
    // 5: one weapon → yes; neither → no.
    assert!(hc(&w, PLAYER, sword, shield));
    assert!(!hc(&w, PLAYER, helm, shield));
    // 2: an ammo type of 0 never matches (skips the test).
    w.p(helm).ammo = Some(0);
    w.p(shield).ammo = Some(0);
    assert!(!hc(&w, PLAYER, helm, shield));
    w.p(helm).ammo = None;
    w.p(shield).ammo = None;
    // 6: both weapons.
    assert!(!hc(&w, UnitId(99), sword, sword2));
    assert!(!hc(&w, PLAYER, sword, sword2));
    w.kinds.insert(PLAYER, UnitKind::Player { class: 4 });
    assert!(hc(&w, PLAYER, sword, sword2));
    w.kinds.insert(PLAYER, UnitKind::Player { class: 6 });
    assert!(!hc(&w, PLAYER, sword, claw));
    assert!(hc(&w, PLAYER, claw, claw2));
    let m = UnitId(3);
    for (class, swords, claws) in [
        (0x1A1, false, true),
        (0x1A2, false, true),
        (0x21C, true, true),
        (0x21D, true, true),
        (0x21E, true, true),
        (0x21F, false, false),
    ] {
        w.kinds.insert(m, UnitKind::Monster { class });
        assert_eq!(hc(&w, m, sword, sword2), swords, "{class:#x}");
        assert_eq!(hc(&w, m, claw, claw2), claws, "{class:#x}");
    }
}

#[test]
fn stack_test_fields() {
    let t = tables();
    let mut w = Fake::new();
    let a = w.add(10, R_ARROWS, mode::STORED);
    let b = w.add(11, R_ARROWS, mode::STORED);
    assert!(stack_test(&w, &t, a, b));
    let s = w.add(12, R_SWORD, mode::STORED);
    let s2 = w.add(13, R_SWORD, mode::STORED);
    assert!(!stack_test(&w, &t, s, s2), "not stackable");
    assert!(!stack_test(&w, &t, a, s), "different class");
    w.p(b).quality = 4;
    assert!(!stack_test(&w, &t, a, b));
    w.p(b).quality = 0;
    w.p(b).stats.insert(stat::THROW_MAXDAMAGE, 3);
    assert!(!stack_test(&w, &t, a, b));
    w.p(b).stats.clear();
    w.p(a).sockets = true;
    assert!(!stack_test(&w, &t, a, b));
}

#[test]
fn equip_from_cursor_steps() {
    let mut h = hands();
    let o = |ok, out| EquipOutcome { ok, out };
    // Step 1.
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, None, 1, false),
        o(false, true)
    );
    let n = h.w.add(10, R_HELM, mode::STORED);
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(n), 1, false),
        o(false, true)
    );
    // Step 2: the check must give 1.
    h.w.items.get_mut(&n).unwrap().mode = mode::CURSOR;
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(n), 2, false),
        o(false, false)
    );
    // (Step 3 repeats not equipping, which a pass of step 2's
    // equipping test implies; not separately observable here.)
    // Steps 4–5.
    h.inv.set_cursor(Some(n));
    h.inv.weapon_guid = 5;
    h.w.items.get_mut(&n).unwrap().flags |= iflag::F4000;
    h.w.log.clear();
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(n), 1, false),
        o(true, false)
    );
    let d = h.w.d(n);
    assert_eq!(
        (d.mode, d.page, d.body_loc, d.node_kind),
        (mode::EQUIPPED, page::NONE, 1, node::BODY)
    );
    assert_eq!(d.cmd_flags, cmd::EQUIP);
    assert_eq!(d.flags & (iflag::CHANGED | iflag::F4000), iflag::CHANGED);
    assert_eq!(h.inv.cursor(), None);
    assert_eq!(h.inv.body_item(1), Some(n));
    assert_eq!(h.inv.update_list(), &[d.guid]);
    assert_eq!(
        h.w.log,
        vec![
            "weapon_in_use",
            "link_check 10 3",
            "stat_link 10",
            "stat_refresh",
            "untargetable 10",
            "owner_refresh",
            "weapon_bookkeeping 10",
            "inventory_pass"
        ]
    );
    // Swap locations: link kind 4, no stat link, no weapon-in-use update.
    let s = h.w.add(11, R_SWORD, mode::CURSOR);
    h.w.log.clear();
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(s), 11, false),
        o(true, false)
    );
    assert_eq!(h.w.log[0], "link_check 11 4");
    assert!(!h
        .w
        .log
        .iter()
        .any(|l| l.starts_with("stat_link") || l == "weapon_in_use"));
    assert_eq!(h.w.d(s).node_kind, node::SWAP);
    // Failed link check → out 1.
    let r = h.w.add(12, R_RING, mode::CURSOR);
    h.w.link_ok = false;
    assert_eq!(
        equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(r), 6, false),
        o(false, true)
    );
    // Variant: command flag 0x10000.
    h.w.link_ok = true;
    let r2 = h.w.add(13, R_RING, mode::CURSOR);
    assert!(equip_put(
        &mut h.inv,
        &mut h.w,
        &h.t,
        r2,
        7,
        cmd::INDIRECT_SWAP_BODY
    ));
    assert_eq!(h.w.d(r2).cmd_flags, cmd::INDIRECT_SWAP_BODY);
}

#[test]
fn auto_equip_rules() {
    let mut h = hands();
    let helm = h.w.add(10, R_HELM, mode::GROUND);
    assert_eq!(
        auto_equip_location(&h.inv, &h.w, &h.t, helm, false),
        Some(1)
    );
    // Not identified / broken / 0x4000 / tpot / failing requirements.
    for f in [
        0,
        iflag::IDENTIFIED | iflag::BROKEN,
        iflag::IDENTIFIED | iflag::F4000,
    ] {
        h.w.items.get_mut(&helm).unwrap().flags = f;
        assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, helm, true), None);
    }
    h.w.items.get_mut(&helm).unwrap().flags = iflag::IDENTIFIED;
    h.w.p(helm).level_req = 50;
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, helm, false), None);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, helm, true), Some(1));
    let tp = h.w.add(11, R_TPOT, mode::GROUND);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, tp, false), None);
    // Same location occupied → no.
    h.equip(12, R_HELM, 1);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, helm, true), None);
    // Two locations (sword 4 / 5).
    let s = h.w.add(13, R_SWORD, mode::GROUND);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, s, false), Some(4));
    // A one-hand weapon and a shield are compatible (profile).
    h.equip(14, R_SHIELD, 5);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, s, false), Some(4));
    // Two one-hand weapons need two dual-wielders: a sorceress (class 1)
    // gets no; a barbarian (class 4) gets the free hand.
    let mut h2 = hands();
    h2.equip(14, R_SWORD, 4);
    let s2 = h2.w.add(13, R_SWORD, mode::GROUND);
    assert_eq!(auto_equip_location(&h2.inv, &h2.w, &h2.t, s2, false), None);
    h2.w.kinds.insert(PLAYER, UnitKind::Player { class: 4 });
    assert_eq!(
        auto_equip_location(&h2.inv, &h2.w, &h2.t, s2, false),
        Some(5)
    );
    h2.equip(15, R_SHIELD, 5);
    assert_eq!(auto_equip_location(&h2.inv, &h2.w, &h2.t, s2, false), None);
}

#[test]
fn auto_equip_quiver_needs_fed_weapon() {
    let mut h = hands();
    let q = h.w.add(10, R_ARROWS, mode::GROUND);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, q, false), None);
    let b = h.equip(11, R_BOW, 4);
    h.w.p(b).ammo = Some(T_BOWQ);
    assert_eq!(auto_equip_location(&h.inv, &h.w, &h.t, q, false), Some(5));
}

// ---------------------------------------------------------------

#[test]
fn item_checks() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let g = |w: &Fake, u: UnitId| w.d(u).guid;
    let cur = w.add(10, R_RING, mode::CURSOR);
    inv.set_cursor(Some(cur));
    let st = w.add(11, R_RING, mode::CURSOR);
    assert!(place_at_page(&mut inv, &mut w, &t, st, 0, 0, 0));
    w.items.get_mut(&st).unwrap().mode = mode::STORED;
    let other = w.add(12, R_RING, mode::STORED);
    w.items.get_mut(&other).unwrap().inv = Some(UnitId(40));
    let belt_other = w.add(13, R_HP1, mode::BELT);
    w.items.get_mut(&belt_other).unwrap().inv = Some(UnitId(40));
    let ground = w.add(14, R_RING, mode::GROUND);
    let missing = 4242;
    // Cursor.
    assert_eq!(cursor_item_check(&inv, &w, g(&w, cur)), 0);
    assert_eq!(cursor_item_check(&inv, &w, g(&w, st)), 1);
    assert_eq!(cursor_item_check(&inv, &w, missing), 1);
    // Stored.
    assert_eq!(stored_item_check(&inv, &w, g(&w, st)), 0);
    assert_eq!(stored_item_check(&inv, &w, g(&w, other)), 1);
    assert_eq!(stored_item_check(&inv, &w, missing), 1);
    // Stored or equipped: inverted, a missing item passes.
    assert_eq!(stored_or_equipped_check(&inv, &w, missing), 0);
    assert_eq!(stored_or_equipped_check(&inv, &w, g(&w, ground)), 0);
    assert_eq!(stored_or_equipped_check(&inv, &w, g(&w, st)), 0);
    assert_eq!(stored_or_equipped_check(&inv, &w, g(&w, other)), 1);
    // Owned.
    assert_eq!(owned_item_check(&inv, &w, g(&w, st)), 0);
    assert_eq!(owned_item_check(&inv, &w, g(&w, cur)), 0);
    assert_eq!(owned_item_check(&inv, &w, g(&w, other)), 1);
    assert_eq!(owned_item_check(&inv, &w, missing), 1);
    // Belt: inverted.
    assert_eq!(belt_item_check(&inv, &w, missing), 0);
    assert_eq!(belt_item_check(&inv, &w, g(&w, st)), 0);
    assert_eq!(belt_item_check(&inv, &w, g(&w, belt_other)), 1);
    // Ground or owned.
    assert_eq!(ground_or_owned_check(&inv, &w, missing), 1);
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, ground)), 0);
    w.in_range = false;
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, ground)), 1);
    w.same_act = false;
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, ground)), 2);
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, st)), 0);
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, other)), 1);
    w.items.get_mut(&st).unwrap().mode = 5;
    assert_eq!(ground_or_owned_check(&inv, &w, g(&w, st)), 1);
}

#[test]
fn busy_and_trading() {
    let mut w = Fake::new();
    let mut inv = player_inv();
    assert!(!busy(&inv, &w));
    w.pd4c = 1;
    assert!(busy(&inv, &w));
    w.pd4c = 0;
    inv.set_cursor(Some(UnitId(9)));
    assert!(busy(&inv, &w));
    inv.set_cursor(None);
    w.interaction = Some(InteractionTarget::Unit {
        ty: 1,
        unit: UnitId(5),
    });
    assert!(busy(&inv, &w));
    assert!(!trading(&inv, &w));
    w.interaction = Some(InteractionTarget::Unit {
        ty: 0,
        unit: UnitId(5),
    });
    assert!(trading(&inv, &w));
    w.interaction = Some(InteractionTarget::Missing);
    assert!(!trading(&inv, &w));
}

#[test]
fn targeting_reset_in_list_order() {
    let t = tables();
    let mut w = Fake::new();
    let mut inv = player_inv();
    let a = w.add(10, R_RING, mode::CURSOR);
    let b = w.add(11, R_RING, mode::CURSOR);
    let c = w.add(12, R_RING, mode::CURSOR);
    for (u, x) in [(c, 0), (a, 1), (b, 2)] {
        assert!(place_at_page(&mut inv, &mut w, &t, u, 0, x, 0));
    }
    for u in [a, c] {
        w.items.get_mut(&u).unwrap().flags |= iflag::TARGETING;
    }
    // The items are no player (probe 6 for them): the probe is asked
    // about the owner, so every flagged item of a player sends a 0x3F.
    w.log.clear();
    targeting_reset(&inv, &mut w);
    assert_eq!(
        w.log,
        vec![
            format!("0x3F {}", w.d(c).guid),
            format!("0x3F {}", w.d(a).guid)
        ]
    );
    for u in [a, b, c] {
        assert_eq!(w.d(u).flags & iflag::TARGETING, 0);
    }
    w.log.clear();
    targeting_reset(&inv, &mut w);
    assert!(w.log.is_empty());

    // A mercenary owner: the flag is cleared, no 0x3F, even for an item
    // whose own would answer 0.
    let merc = UnitId(60);
    let class = 0x230;
    w.kinds.insert(merc, UnitKind::Monster { class });
    let mut minv = Inventory::new(merc, UnitKind::Monster { class }, 5);
    let m = w.add(13, R_RING, mode::CURSOR);
    assert!(place_at_page(&mut minv, &mut w, &t, m, 0, 0, 0));
    w.kinds.insert(m, UnitKind::Player { class: 0 });
    w.items.get_mut(&m).unwrap().flags |= iflag::TARGETING;
    w.log.clear();
    targeting_reset(&minv, &mut w);
    assert_eq!(w.d(m).flags & iflag::TARGETING, 0);
    assert!(w.log.is_empty());
}

#[test]
fn item_move_gate_rules() {
    let mut w = Fake::new();
    let inv = player_inv();
    assert!(item_move_gate(&inv, &mut w));
    w.pd50 = 5;
    assert!(!item_move_gate(&inv, &mut w));
    w.pd50 = 4;
    w.pd4c = 1;
    assert!(!item_move_gate(&inv, &mut w));
    w.pd4c = 0;
    w.interaction = Some(InteractionTarget::Missing);
    assert!(!item_move_gate(&inv, &mut w));
    assert_eq!(w.interaction, None);
    assert_eq!(w.log, vec!["clear_interaction"]);
    let npc = UnitId(7);
    w.interaction = Some(InteractionTarget::Unit { ty: 1, unit: npc });
    w.pd50 = 9;
    assert!(item_move_gate(&inv, &mut w));
    w.talking = true;
    assert!(!item_move_gate(&inv, &mut w));
    for ty in [0, 2] {
        w.interaction = Some(InteractionTarget::Unit { ty, unit: npc });
        assert!(!item_move_gate(&inv, &mut w));
        w.pd50 = 0;
        assert!(item_move_gate(&inv, &mut w));
        w.pd50 = 9;
    }
}

// ------------------------------------------------------------ game files

/// Reads one base-game `.bin` table from the MPQs in `D2_GAME_DIR` (tests
/// only; conventions), through `d2_data::bin::read_excel`, the
/// game-data archive lookup order. Not a fixed
/// `patch_d2` path: `tables/data/tables.tsv` gives `inventory` and `belts`
/// as `d2exp/*.bin` (only `itemtypes` comes from `patch_d2`), and
/// `d2-data`'s `excel_lookup_order` / `live_bin_set_loads` read
/// `inventory.bin` from `d2exp.mpq`. The old extracted-`patch_d2` path made
/// the first local run fail with "inventory.bin not found" (2026-10-06).
#[allow(clippy::disallowed_methods)]
fn game_table(name: &str, size: usize) -> d2_data::bin::BinTable {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let set = d2_formats::mpq::ArchiveSet::open_dir(dir).expect("archives open");
    let file = format!("{name}.bin");
    let (source, bytes) = d2_data::bin::read_excel(&set, &file)
        .unwrap_or_else(|e| panic!("{file}: {e}"))
        .unwrap_or_else(|| panic!("{file}: in no archive"));
    d2_data::bin::BinTable::parse(name, &source, &file, &bytes, size).expect("table parses")
}

/// D1–D3 (expected values measured on the base-game install; they run
/// only where the game files are present).
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_grid_belt_and_type_tables() {
    use d2_data::tables::{decode_all, Belts, Inventory as InvBin, Itemtypes, Record};
    // D1.
    let inv: Vec<InvBin> = decode_all(&game_table("inventory", InvBin::SIZE)).unwrap();
    assert_eq!(inv.len(), 32);
    let sizes: Vec<(u8, u8)> = inv[..16].iter().map(|r| (r.gridx, r.gridy)).collect();
    let mut want = vec![(10, 4); 16];
    want[5] = (10, 10);
    want[8] = (6, 4);
    want[9] = (3, 4);
    want[12] = (6, 8);
    want[13] = (0, 0);
    assert_eq!(sizes, want);
    // D2.
    let belts: Vec<Belts> = decode_all(&game_table("belts", Belts::SIZE)).unwrap();
    let n: Vec<u8> = belts.iter().map(|b| b.numboxes).collect();
    assert_eq!(n, [12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16]);
    // D3.
    let types: Vec<Itemtypes> = decode_all(&game_table("itemtypes", Itemtypes::SIZE)).unwrap();
    for (i, code) in [
        (2, b"shie"),
        (3, b"tors"),
        (4, b"gold"),
        (18, b"book"),
        (22, b"scro"),
        (27, b"bow "),
        (28, b"axe "),
        (30, b"swor"),
        (33, b"spea"),
        (34, b"pole"),
        (37, b"helm"),
        (45, b"weap"),
        (67, b"h2h "),
        (71, b"phlm"),
        (76, b"hpot"),
        (80, b"apot"),
        (81, b"wpot"),
    ] {
        assert_eq!(&types[i].code, code, "itemtypes row {i}");
    }
}

#[path = "answer_tests.rs"]
mod answer_tests;
#[path = "gap_tests.rs"]
mod gap_tests;
#[path = "mutant_tests.rs"]
mod mutant_tests;

#[test]
fn weapon_in_use_link_and_unlink() {
    use super::weapon::{weapon_link, weapon_unlink};
    let mut h = hands();
    let at = |h: &mut Hands, id, r, loc| {
        let u = h.equip(id, r, loc);
        h.w.items.get_mut(&u).unwrap().body_loc = loc;
        u
    };
    // The first usable weapon in a hand becomes the weapon in use.
    let a = at(&mut h, 20, R_SWORD, body::RIGHT_HAND);
    weapon_link(&mut h.inv, &h.w, &h.t, a);
    assert_eq!(h.inv.weapon_guid, h.w.d(a).guid);
    // A second one leaves it (the first wielded weapon stays).
    let b = at(&mut h, 21, R_SWORD, body::LEFT_HAND);
    weapon_link(&mut h.inv, &h.w, &h.t, b);
    assert_eq!(h.inv.weapon_guid, h.w.d(a).guid);
    // Not a weapon: nothing.
    let helm = at(&mut h, 22, R_HELM, body::HEAD);
    weapon_link(&mut h.inv, &h.w, &h.t, helm);
    assert_eq!(h.inv.weapon_guid, h.w.d(a).guid);
    // Taking the weapon in use off hands +0x1C to the other hand.
    weapon_unlink(&mut h.inv, &h.w, &h.t, a);
    assert_eq!(h.inv.weapon_guid, h.w.d(b).guid);
    // The weapon in use linked again → −1 (W is the item itself).
    weapon_link(&mut h.inv, &h.w, &h.t, b);
    assert_eq!(h.inv.weapon_guid, NO_GUID);
    // A weapon that is not usable (unidentified) clears +0x1C when
    // it held it, and never becomes the weapon in use.
    weapon_link(&mut h.inv, &h.w, &h.t, b);
    assert_eq!(h.inv.weapon_guid, h.w.d(b).guid);
    h.w.items.get_mut(&b).unwrap().flags = 0;
    weapon_link(&mut h.inv, &h.w, &h.t, b);
    assert_eq!(h.inv.weapon_guid, NO_GUID);
    weapon_link(&mut h.inv, &h.w, &h.t, b);
    assert_eq!(h.inv.weapon_guid, NO_GUID);
}

#[test]
fn failed_equip_keeps_cursor_and_ground_items_in_place() {
    for from_ground in [false, true] {
        for fail_placement in [false, true] {
            let mut h = hands();
            let mode = if from_ground {
                mode::GROUND
            } else {
                mode::CURSOR
            };
            let item = h.w.add(10, R_HELM, mode);
            if !from_ground {
                h.inv.set_cursor(Some(item));
            }
            let before = h.w.d(item);
            h.w.link_ok = fail_placement;
            if fail_placement {
                h.inv
                    .grid_or_create(grid_id::BODY, BODY_GRID.0 + 1, BODY_GRID.1)
                    .unwrap();
            }
            let placed = if from_ground {
                equip::equip_without_cursor(&mut h.inv, &mut h.w, &h.t, item, false)
            } else {
                equip_from_cursor(&mut h.inv, &mut h.w, &h.t, Some(item), 1, false).ok
            };
            assert!(!placed);
            assert_eq!(h.w.d(item), before);
            assert_eq!(h.inv.body_item(1), None);
            assert_eq!(h.inv.cursor(), (!from_ground).then_some(item));
            assert!(h.inv.items().is_empty());
            assert!(h.inv.update_list().is_empty());
        }
    }
}
