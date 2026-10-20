//! Contract checks of the item-grid seam: the sim's item place (page,
//! cell, body location, belt slot) as the server writes it, the wire
//! carries it and the client model and UI read it. Each test feeds one
//! value through both sides' own code and compares. No game files.

use d2_client::bridge::items::{self, mode};
use d2_client::bridge::world::{
    ClientUnit, ClientWorld, ItemData, ItemRecord, KindData, PlayerData, UnitKey,
};
use d2_client::ui::geom::Point;
use d2_client::ui::inv_grid::GridRecord;
use d2_client::ui::layout::Screen;
use d2_client::ui::original::inventory_record;
use d2_client::ui::panels::cube_items::cube_record;
use d2_client::ui::panels::shop::shop_tabs;
use d2_client::ui::panels::stash_items::{fallback_stash_grid, stash_record};
use d2_proto::item_bits::{self, CodeFacts, IscSave, ItemLookup, Location};
use d2_sim::items::bitstream::{self, Isc, StreamItem};
use d2_sim::items::inventory::{grid_record, UnitKind};

/// A compact record (no property lists) at the given place.
fn stream(mode: u32, body: u8, x: i32, y: i32, page: u8) -> Vec<u8> {
    let item = StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        mode,
        x,
        y,
        body_loc: body,
        page,
        code: *b"hp1 ",
        ..Default::default()
    };
    let isc: Vec<Isc> = Vec::new();
    bitstream::write(&item, &isc).expect("stream fits").0
}

struct AnyCode;
impl ItemLookup for AnyCode {
    fn code(&self, _: [u8; 4]) -> Option<CodeFacts> {
        Some(CodeFacts::default())
    }
    fn isc(&self, _: u16) -> Option<IscSave> {
        None
    }
}

/// A world with the local player and one item whose last record is
/// `stream` (0x9C, `action`).
fn world_with(stream: Vec<u8>, action: u8) -> (ClientWorld, UnitKey) {
    let mut w = ClientWorld::default();
    let p = UnitKey::new(0, 1);
    let mut u = ClientUnit::new(p);
    u.kind = KindData::Player(PlayerData::default());
    w.units.insert(p, u);
    w.local_player = Some(p);
    let k = UnitKey::new(4, 7);
    let mut u = ClientUnit::new(k);
    u.kind = KindData::Item(ItemData {
        last: Some(ItemRecord {
            id: 0x9C,
            action,
            category: 0,
            owner: None,
            seq: 0,
            stream,
        }),
        ..ItemData::default()
    });
    w.units.insert(k, u);
    (w, k)
}

#[test]
fn wire_place_round_trips_to_the_client_model() {
    // Stored items on every page, body items, belt items, the cursor.
    let mut cases: Vec<(u32, u8, i32, i32, u8)> = Vec::new();
    for page in [0u8, 1, 2, 3, 4] {
        for (x, y) in [(0, 0), (9, 3), (5, 7), (2, 9)] {
            cases.push((0, 0, x, y, page));
        }
    }
    for body in 1u8..=12 {
        cases.push((1, body, i32::from(body), 0, 0xFF));
    }
    for slot in 0..16 {
        cases.push((2, 0, slot, 0, 0xFF));
    }
    cases.push((4, 0, 0, 0, 0xFF));
    for (m, body, x, y, page) in cases {
        let s = stream(m, body, x, y, page);
        // Client model head.
        let (_, cm, cbody, cpage, cx, cy, code) = items::peek(&s).expect("client head");
        assert_eq!(
            (u32::from(cm), cbody, cpage, i32::from(cx), i32::from(cy)),
            (m, body, page, x, y),
            "client head of mode {m} body {body} ({x}, {y}) page {page:#x}"
        );
        assert_eq!(code, Some(*b"hp1 "));
        // Proto reader: page + 1 raw.
        let d = item_bits::decode(&s, &AnyCode).expect("proto decode");
        assert_eq!(
            d.location,
            Some(Location::Slot {
                body,
                x: x as u8,
                y: y as u8,
                page1: page.wrapping_add(1),
            })
        );
    }
    // Ground: 16-bit sub-tile, no page.
    let s = stream(3, 0, 0x1234, 0x0F0F, 0xFF);
    let (_, cm, _, cpage, cx, cy, _) = items::peek(&s).unwrap();
    assert_eq!((cm, cpage, cx, cy), (mode::GROUND, 0xFF, 0x1234, 0x0F0F));
}

#[test]
fn grid_record_by_page_agrees() {
    for res in [Screen::R640, Screen::R800] {
        let add = if res.res2() { 16 } else { 0 };
        for class in 0u8..=6 {
            let sim = grid_record(UnitKind::Player { class }, 0, true).map(|r| r + add);
            assert_eq!(
                sim,
                inventory_record(u32::from(class), &res),
                "class {class}"
            );
        }
        for exp in [false, true] {
            let p = UnitKind::Player { class: 0 };
            assert_eq!(
                grid_record(p, 4, exp).map(|r| r + add),
                Some(stash_record(exp, &res)),
                "stash, expansion {exp}"
            );
            assert_eq!(
                grid_record(p, 3, exp).map(|r| r + add),
                Some(cube_record(&res)),
                "cube"
            );
        }
    }
}

/// The measured inventory records (known test vectors).
fn measured() -> Vec<(&'static str, GridRecord)> {
    let g = |gx, gy, left, right, top, bottom| GridRecord {
        grid_x: gx,
        grid_y: gy,
        left,
        right,
        top,
        bottom,
        cell_w: 29,
        cell_h: 29,
    };
    vec![
        ("record 0", g(10, 4, 339, 626, 255, 368)),
        ("record 16", g(10, 4, 419, 706, 315, 428)),
        ("record 12", g(6, 8, 74, 244, 82, 313)),
        ("record 28", g(6, 8, 154, 324, 142, 373)),
    ]
}

#[test]
fn every_accepted_pixel_names_an_in_grid_cell() {
    let mut grids = measured();
    for (name, expansion, screen) in [
        ("fallback 12", true, Screen::R640),
        ("fallback 28", true, Screen::R800),
        ("fallback 8", false, Screen::R640),
        ("fallback 24", false, Screen::R800),
    ] {
        grids.push((name, fallback_stash_grid(expansion, &screen)));
    }
    for (name, g) in grids {
        for y in g.top - 2..=g.bottom + 2 {
            for x in g.left - 2..=g.right + 2 {
                let p = Point::new(x, y);
                if !g.contains_mouse(p) {
                    continue;
                }
                let (c, r) = g.mouse_cell(p);
                assert!(
                    c < u32::from(g.grid_x) && r < u32::from(g.grid_y),
                    "{name}: pixel ({x}, {y}) → cell ({c}, {r}) outside the grid"
                );
                // The cell's rectangle holds the pixel.
                let (cx, cy, cw, ch) = g.cell(c as i32, r as i32);
                assert!((cx..cx + cw).contains(&x) && (cy..cy + ch).contains(&y));
            }
        }
    }
}

#[test]
fn the_stash_fallback_is_the_measured_record() {
    let m = measured();
    assert_eq!(fallback_stash_grid(true, &Screen::R640), m[2].1);
    assert_eq!(fallback_stash_grid(true, &Screen::R800), m[3].1);
}

#[test]
fn belt_slot_is_the_stream_x() {
    for slot in 0..16 {
        let (w, k) = world_with(stream(2, 0, slot, 0, 0xFF), 0x0E);
        let belt = items::belt(&w);
        assert_eq!(belt.len(), 1);
        assert_eq!(
            belt.get(&(slot as u16)).map(|i| i.key),
            Some(k),
            "slot {slot}"
        );
    }
}

#[test]
fn store_page_is_the_shop_tab() {
    // `storepage` 0 armor, 1 weapons, 2 magic, 3 misc.
    for store_page in 0u8..4 {
        let (w, _) = world_with(stream(0, 0, 0, 0, store_page), 0x0B);
        let shown = items::store_items(&w);
        assert_eq!(shown.len(), 1);
        let mut counts = [0u32; 5];
        counts[usize::from(shown[0].page)] += 1;
        let (tabs, cur) = shop_tabs(0, counts, 3);
        assert_eq!(cur, store_page);
        assert!(tabs[usize::from(store_page)].1, "tab {store_page} shown");
    }
}
