//! Body-box presses over synthetic tables (no game files): the itemtypes
//! ids of the inventory body table (2 `shie`, 3 `tors`, 27 `bow`, 28 `axe`,
//! 30 `swor`, 37 `helm`, 45 `weap`, 71 `phlm`), the rest invented.

use super::*;
use crate::bridge::items::mode;
use crate::bridge::world::ClientWorld;
use crate::ui::panels::inv_items::tests::{world, Fixture, PLAYER as P};
use d2_data::fixup::maps::EquivMatrix;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};

const SHIE: u8 = 2;
const TORS: u8 = 3;
const AMMO: u8 = 5;
const BOW: u8 = 27;
const AXE: u8 = 28;
const SWOR: u8 = 30;
const HELM: u8 = 37;
const WEAP: u8 = 45;
const PHLM: u8 = 71;
const N_TYPES: usize = 80;

/// Player classes (`playerclass`): 1 sorceress, 4 barbarian.
const SORCERESS: u32 = 1;
const BARBARIAN: u32 = 4;

fn equiv() -> EquivMatrix {
    let n = N_TYPES;
    let words = n.div_ceil(32);
    let mut m = EquivMatrix {
        n,
        words,
        bits: vec![0; n * words],
    };
    let mut set = |i: usize, j: usize| m.bits[i * words + j / 32] |= 1 << (j % 32);
    for i in 0..n {
        set(i, i);
    }
    for c in [BOW, AXE, SWOR] {
        set(usize::from(c), usize::from(WEAP));
    }
    m
}

pub(crate) fn tables() -> InvTables {
    let mut itemtypes = vec![
        InvTypeRec {
            class: CLASS_NONE,
            ..InvTypeRec::default()
        };
        N_TYPES
    ];
    let mut at = |t: u8, a: u8, b: u8| {
        let r = &mut itemtypes[usize::from(t)];
        r.body = 1;
        r.bodyloc1 = a;
        r.bodyloc2 = b;
    };
    for t in [SHIE, BOW, AXE, SWOR, AMMO] {
        at(t, 4, 5);
    }
    at(HELM, 1, 1);
    at(PHLM, 1, 1);
    at(TORS, 3, 3);
    itemtypes[usize::from(PHLM)].class = BARBARIAN as u8;
    itemtypes[usize::from(AMMO)].quiver = 1;
    let rec = |code: &[u8; 4], t: u8, w, h| InvItemRec {
        code: *code,
        type_: i16::from(t),
        invwidth: w,
        invheight: h,
        ..InvItemRec::default()
    };
    let mut items = vec![
        rec(b"ssd ", SWOR, 1, 3),
        rec(b"2hs ", SWOR, 2, 4),
        rec(b"buc ", SHIE, 2, 2),
        rec(b"cap ", HELM, 2, 2),
        rec(b"qui ", TORS, 2, 3),
        rec(b"ba1 ", PHLM, 2, 2),
        rec(b"hax ", AXE, 1, 3),
        rec(b"sbw ", BOW, 2, 3),
        rec(b"aqv ", AMMO, 2, 3),
    ];
    items[1].twohanded = 1;
    items[7].twohanded = 1;
    items[3].reqstr = 50;
    items[8].stackable = 1;
    let g = |x, y| GridRec {
        grid_x: x,
        grid_y: y,
    };
    let mut grids = vec![g(10, 4); 16];
    grids[5] = g(10, 10);
    InvTables {
        grids,
        belts: vec![12; 14],
        items,
        itemtypes,
        equiv: equiv(),
        books: Vec::new(),
        item_use: Default::default(),
    }
}

/// Every stream decodes as a normal-quality (2) record with no lists.
fn decode(_: &[u8]) -> Option<ItemBits> {
    Some(ItemBits {
        quality: 2,
        ..ItemBits::default()
    })
}

fn player(w: &mut ClientWorld, class: u32, strength: i32) {
    let p = w.units.get_mut(&P).unwrap();
    p.class = class;
    p.stats.insert(0, strength);
    p.stats.insert(2, 100);
    p.stats.insert(12, 50);
}

fn press(items: &[Fixture], cursor: Option<u32>, class: u32, loc: u8, c: Cursor) -> BodyPress {
    let mut w = world(items, cursor);
    player(&mut w, class, 100);
    press_in(&w, loc, c)
}

fn press_in(w: &ClientWorld, loc: u8, c: Cursor) -> BodyPress {
    let t = tables();
    let it = ItemTables::default();
    let (view, inv, _) = ClientInv::build(w, &t, &it, &decode).unwrap();
    body_press(&view, &inv, loc, c)
}

fn bytes(p: &BodyPress) -> Vec<Vec<u8>> {
    p.out
        .iter()
        .filter_map(|o| match o {
            PanelOutput::Intent(i) => Some(i.0.clone()),
            _ => None,
        })
        .collect()
}

fn events(p: &BodyPress) -> Vec<u16> {
    p.out
        .iter()
        .filter_map(|o| match o {
            PanelOutput::PlayerEvent(e) => Some(*e),
            _ => None,
        })
        .collect()
}

const CUR: (u8, u16, u16, u8) = (0, 0, 0, 0);
fn worn(l: u8) -> (u8, u16, u16, u8) {
    (l, 0, 0, 0)
}

#[test]
fn e1_head_free_equips_and_occupied_swaps() {
    let p = press(
        &[(9, mode::CURSOR, CUR, b"ba1 ")],
        Some(9),
        BARBARIAN,
        1,
        Cursor::Item,
    );
    assert_eq!(bytes(&p), vec![vec![0x1A, 9, 0, 0, 0, 1, 0, 0, 0]]);
    let p = press(
        &[
            (9, mode::CURSOR, CUR, b"ba1 "),
            (8, mode::BODY, worn(1), b"ba1 "),
        ],
        Some(9),
        BARBARIAN,
        1,
        Cursor::Item,
    );
    assert_eq!(bytes(&p), vec![vec![0x1D, 9, 0, 0, 0, 1, 0, 0, 0]]);
}

#[test]
fn e2_two_hander_onto_a_free_hand_beside_a_shield_sends_0x1b() {
    let p = press(
        &[
            (9, mode::CURSOR, CUR, b"2hs "),
            (8, mode::BODY, worn(5), b"buc "),
        ],
        Some(9),
        SORCERESS,
        4,
        Cursor::Item,
    );
    assert_eq!(bytes(&p), vec![vec![0x1B, 9, 0, 0, 0, 4, 0, 0, 0]]);
}

#[test]
fn e3_barbarian_one_hander_beside_an_axe_equips() {
    let items = [
        (9, mode::CURSOR, CUR, b"ssd "),
        (8, mode::BODY, worn(5), b"hax "),
    ];
    let p = press(&items, Some(9), BARBARIAN, 4, Cursor::Item);
    assert_eq!(bytes(&p), vec![vec![0x1A, 9, 0, 0, 0, 4, 0, 0, 0]]);
    // A sorceress cannot pair two weapons: e 2 → 0x1B.
    let p = press(&items, Some(9), SORCERESS, 4, Cursor::Item);
    assert_eq!(bytes(&p), vec![vec![0x1B, 9, 0, 0, 0, 4, 0, 0, 0]]);
}

#[test]
fn e4_empty_hand_beside_a_bow_lifts_the_other_hand() {
    let p = press(
        &[(8, mode::BODY, worn(5), b"sbw ")],
        None,
        SORCERESS,
        4,
        Cursor::Plain,
    );
    assert_eq!(bytes(&p), vec![vec![0x1C, 5, 0]]);
}

#[test]
fn an_occupied_box_without_a_cursor_item_lifts_it() {
    let p = press(
        &[(8, mode::BODY, worn(3), b"qui ")],
        None,
        SORCERESS,
        3,
        Cursor::Plain,
    );
    assert_eq!(bytes(&p), vec![vec![0x1C, 3, 0]]);
    // An empty box with nothing on the cursor: e 0, nothing.
    let p = press(&[], None, SORCERESS, 3, Cursor::Plain);
    assert!(p.out.is_empty());
}

#[test]
fn a_two_hander_over_sword_and_shield_sends_0x1e() {
    let p = press(
        &[
            (9, mode::CURSOR, CUR, b"2hs "),
            (8, mode::BODY, worn(4), b"ssd "),
            (7, mode::BODY, worn(5), b"buc "),
        ],
        Some(9),
        SORCERESS,
        4,
        Cursor::Item,
    );
    assert_eq!(bytes(&p), vec![vec![0x1E, 9, 0, 0, 0, 4, 0, 0, 0]]);
}

#[test]
fn ammo_onto_a_worn_stack_of_the_same_kind_sends_0x21() {
    let p = press(
        &[
            (9, mode::CURSOR, CUR, b"aqv "),
            (8, mode::BODY, worn(5), b"aqv "),
        ],
        Some(9),
        SORCERESS,
        5,
        Cursor::Item,
    );
    assert_eq!(bytes(&p), vec![vec![0x21, 9, 0, 0, 0, 8, 0, 0, 0]]);
}

#[test]
fn q4_a_failed_strength_requirement_says_cant_use_yet_and_sends_nothing() {
    let mut w = world(&[(9, mode::CURSOR, CUR, b"cap ")], Some(9));
    player(&mut w, SORCERESS, 10);
    let p = press_in(&w, 1, Cursor::Item);
    assert!(bytes(&p).is_empty());
    assert_eq!(events(&p), vec![event::CANT_USE_YET]);
}

#[test]
fn q5_another_class_item_says_impossible_and_sends_nothing() {
    let p = press(
        &[(9, mode::CURSOR, CUR, b"ba1 ")],
        Some(9),
        SORCERESS,
        1,
        Cursor::Item,
    );
    assert!(bytes(&p).is_empty());
    assert_eq!(events(&p), vec![event::IMPOSSIBLE]);
    // The location refuses the item outright: `impossible` too.
    let p = press(
        &[(9, mode::CURSOR, CUR, b"buc ")],
        Some(9),
        SORCERESS,
        1,
        Cursor::Item,
    );
    assert_eq!(events(&p), vec![event::IMPOSSIBLE]);
}

#[test]
fn the_use_cursor_on_a_worn_item_sends_0x27_and_ends() {
    let p = press(
        &[(8, mode::BODY, worn(3), b"qui ")],
        None,
        SORCERESS,
        3,
        Cursor::Use(5),
    );
    assert_eq!(bytes(&p), vec![vec![0x27, 8, 0, 0, 0, 5, 0, 0, 0]]);
    assert!(p.end_use);
    // A hand: rule 2 sends, rule 3 (e 3) finds T marked and sends nothing.
    let p = press(
        &[(8, mode::BODY, worn(4), b"ssd ")],
        None,
        SORCERESS,
        4,
        Cursor::Use(5),
    );
    assert_eq!(bytes(&p), vec![vec![0x27, 8, 0, 0, 0, 5, 0, 0, 0]]);
    // An empty box: r = 0, the use is cancelled.
    let p = press(&[], None, SORCERESS, 3, Cursor::Use(5));
    assert!(p.out.is_empty());
    assert!(p.end_use);
}
