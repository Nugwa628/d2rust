//! Synthetic fixtures for `inv_items`: no game files.

use super::*;
use crate::bridge::items::ItemArtRow;
use crate::bridge::world::{ClientUnit, ItemData, ItemRecord, KindData, PlayerData, UnitKey};
use crate::ui::draw::UiDraw;

pub(crate) const PLAYER: UnitKey = UnitKey::new(0, 1);

/// An item stream head: flags, version, mode,
/// location (body, x, y, page + 1), code.
fn stream(m: u8, loc: (u8, u16, u16, u8), code: &[u8; 4]) -> Vec<u8> {
    let bits: Vec<(u32, u32)> = vec![
        (0x10, 32),
        (0x65, 10),
        (u32::from(m), 3),
        (u32::from(loc.0), 4),
        (u32::from(loc.1), 4),
        (u32::from(loc.2), 4),
        (u32::from(loc.3), 3),
        (u32::from_le_bytes(*code), 32),
    ];
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

/// A world with the local player (class 0) and items (guid, mode,
/// location, code); `cursor` names the cursor item.
/// guid, mode, location (body, x, y, page + 1), code.
pub(crate) type Fixture<'a> = (u32, u8, (u8, u16, u16, u8), &'a [u8; 4]);

pub(crate) fn world(items: &[Fixture], cursor: Option<u32>) -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut p = ClientUnit::new(PLAYER);
    p.kind = KindData::Player(PlayerData {
        cursor_item: cursor,
        ..PlayerData::default()
    });
    w.units.insert(PLAYER, p);
    w.local_player = Some(PLAYER);
    for &(guid, m, loc, code) in items {
        let k = UnitKey::new(items::ITEM, guid);
        let mut u = ClientUnit::new(k);
        u.kind = KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: 0x9C,
                action: 0x04,
                category: 0,
                owner: None,
                seq: 0,
                stream: stream(m, loc, code),
            }),
            ..ItemData::default()
        });
        w.units.insert(k, u);
    }
    w
}

fn layout() -> InvLayout {
    let mut equip = [NO_BOX; 11];
    // Torso (body location 3).
    equip[3] = EquipBox {
        left: 10,
        top: 20,
        w: 58,
        h: 87,
        right: 67,
        bottom: 106,
    };
    InvLayout {
        grid: GridRecord {
            grid_x: 10,
            grid_y: 4,
            left: 100,
            right: 390,
            top: 200,
            bottom: 316,
            cell_w: 29,
            cell_h: 29,
        },
        equip,
    }
}

fn ui() -> (ItemsUi, UiFiles) {
    let mut art = ItemArtRows::default();
    let row = |w, h, f: &str| ItemArtRow {
        inv_w: w,
        inv_h: h,
        inv_file: f.into(),
        flippy_file: String::new(),
        beltable: f == "invhp1",
    };
    art.0.insert(*b"hp1 ", row(1, 1, "invhp1"));
    art.0.insert(*b"qui ", row(2, 3, "invqlt"));
    let u = ItemsUi {
        art,
        layouts: Some(vec![layout()]),
        frame_sizes: BTreeMap::new(),
        shift: false,
        ..ItemsUi::default()
    };
    let mut files = UiFiles::new(&[]);
    u.register_files(&mut files);
    (u, files)
}

fn images(d: &[UiDraw]) -> Vec<(u32, u32, i32, i32)> {
    d.iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) => Some((i.image.file, i.image.frame, i.at.x, i.at.y)),
            _ => None,
        })
        .collect()
}

fn intents(o: &[PanelOutput]) -> Vec<Vec<u8>> {
    o.iter()
        .filter_map(|o| match o {
            PanelOutput::Intent(i) => Some(i.0.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_grid_item_draws_at_its_cell() {
    let (u, files) = ui();
    let w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"hp1 ")], None);
    let l = u.layout(Some(0), &Screen::R640).unwrap();
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_panel(&w, &files, &l, &mut out);
    let f = files.id("*items\\invhp1").unwrap();
    // Cell (2, 3): top-left (100 + 58, 200 + 87); cel at top + h (29).
    assert_eq!(images(&out), vec![(f, 0, 158, 316)]);
}

#[test]
fn an_ethereal_item_draws_in_mode_1_and_others_in_mode_5() {
    let (u, files) = ui();
    let mut w = world(
        &[
            (7, mode::STORED, (0, 2, 3, 1), b"hp1 "),
            (8, mode::STORED, (0, 4, 3, 1), b"hp1 "),
        ],
        None,
    );
    // Item flag 0x400000 (ethereal) on item 8: header byte 2, bit 0x40.
    if let KindData::Item(d) = &mut w.units.get_mut(&UnitKey::new(items::ITEM, 8)).unwrap().kind {
        d.last.as_mut().unwrap().stream[2] |= 0x40;
    }
    let l = u.layout(Some(0), &Screen::R640).unwrap();
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_panel(&w, &files, &l, &mut out);
    let looks: Vec<(i32, crate::ui::CelLook)> = out
        .iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) => Some((i.at.x, i.look)),
            _ => None,
        })
        .collect();
    // Cells (2, 3) and (4, 3): x 158 and 216; no tips, so no colour map.
    assert_eq!(
        looks,
        vec![
            (158, crate::ui::CelLook::PLAIN),
            (
                216,
                crate::ui::CelLook {
                    mode: 1,
                    remap: crate::ui::Remap::None
                }
            ),
        ]
    );
}

#[test]
fn an_equipped_item_draws_in_its_box() {
    let (u, files) = ui();
    let w = world(&[(8, mode::BODY, (3, 0, 0, 0), b"qui ")], None);
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_panel(&w, &files, &layout(), &mut out);
    let f = files.id("*items\\invqlt").unwrap();
    // x = 10 + (58 − 58) / 2 + 3 (torso), y = 20 + (87 − 87) / 2; + h 87.
    assert_eq!(images(&out), vec![(f, 0, 13, 107)]);
}

#[test]
fn empty_art_draws_nothing_and_item_files_map_to_items() {
    let w = world(&[(7, mode::STORED, (0, 0, 0, 1), b"hp1 ")], Some(7));
    let u = ItemsUi::default();
    let files = UiFiles::new(&[]);
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_panel(&w, &files, &layout(), &mut out);
    u.draw_cursor(&w, &files, (29, 29), Point::new(5, 5), &mut out);
    assert!(out.is_empty());
    assert_eq!(
        crate::world_view::panel_art::archive_name(&item_file_name("invHp1")),
        "data\\global\\items\\invhp1.dc6"
    );
    assert_eq!(
        crate::world_view::panel_art::archive_name("panel\\invchar6"),
        "data\\global\\ui\\panel\\invchar6.dc6"
    );
}

#[test]
fn the_cursor_item_draws_centred_on_the_mouse() {
    let (u, files) = ui();
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"qui ")], Some(9));
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_cursor(&w, &files, (29, 29), Point::new(300, 200), &mut out);
    let f = files.id("*items\\invqlt").unwrap();
    // 58 × 87: top-left (300 − 29, 200 − 43), cel at top + 87.
    assert_eq!(images(&out), vec![(f, 0, 271, 244)]);
}

#[test]
fn a_cursor_item_on_an_empty_cell_sends_0x18() {
    let (u, files) = ui();
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"hp1 ")], Some(9));
    // Mouse in cell (4, 1).
    let out = u.press(
        &w,
        &files,
        &layout(),
        Point::new(100 + 4 * 29 + 3, 200 + 29 + 3),
    );
    let want = ClientIntent::from_message(&items::insert(9, 4, 1, 0)).0;
    assert_eq!(want[0], 0x18);
    assert_eq!(intents(&out), vec![want]);
}

#[test]
fn a_grid_item_without_a_cursor_item_sends_0x19() {
    let (u, files) = ui();
    let w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"hp1 ")], None);
    let out = u.press(&w, &files, &layout(), Point::new(160, 290));
    assert_eq!(intents(&out), vec![vec![0x19, 7, 0, 0, 0]]);
    // An empty cell sends nothing.
    assert!(u
        .press(&w, &files, &layout(), Point::new(101, 201))
        .is_empty());
}

// The hover tint is the hovered item of the hover state, not the item under the
// mouse; a use press clears that state (measured: the right-clicked cube draws
// with its own tint) until the next move tracks it
// again.
#[test]
fn a_use_press_clears_the_hover_tint_until_the_mouse_moves() {
    let (mut u, files) = ui();
    u.tint_colors = Some(TINTS);
    let mut w = world(&[(7, mode::STORED, (0, 0, 0, 1), b"box ")], None);
    w.units.get_mut(&PLAYER).unwrap().position = Some((10, 20));
    let l = layout();
    let at = in_cell(0, 0);
    let cube = crate::bridge::items::local_items(&w)
        .into_iter()
        .find(|i| i.key.guid == 7)
        .unwrap();
    let hovered = |u: &ItemsUi| {
        let d = u.grid_item_tints(&w, &l.grid, at, &cube);
        !tiles(&d, TINTS[1]).is_empty()
    };
    // Under the mouse but not tracked: not hovered.
    assert!(!hovered(&u));
    u.track_hover(&w, &files, &l.grid, 0, at);
    assert!(hovered(&u));
    assert!(!u.right_press(&w, &files, &l, at).is_empty(), "0x20 sent");
    assert!(!hovered(&u), "the use press clears the hover");
    u.track_hover(&w, &files, &l.grid, 0, at);
    assert!(hovered(&u), "a move tracks it again");
}

#[test]
fn a_right_click_on_a_grid_item_sends_0x20_with_the_player_point() {
    let (u, _) = ui();
    let mut w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"hp1 ")], None);
    w.units.get_mut(&PLAYER).unwrap().position = Some((300, 400));
    let out = u.use_press(&w, &layout(), Point::new(160, 290));
    assert_eq!(
        intents(&out),
        vec![vec![0x20, 7, 0, 0, 0, 0x2C, 1, 0, 0, 0x90, 1, 0, 0]]
    );
    // An empty cell, or an item on the cursor: nothing.
    assert!(u.use_press(&w, &layout(), Point::new(101, 201)).is_empty());
    let c = world(&[(7, mode::STORED, (0, 2, 3, 1), b"hp1 ")], Some(9));
    assert!(u.use_press(&c, &layout(), Point::new(160, 290)).is_empty());
}

#[test]
fn the_cursor_cell_centres_an_even_item() {
    let g = layout().grid;
    // 2 × 3 item, graphic 58 × 87, mouse at (200, 250): c = (14 − 100 +
    // 200) / 29 = 3, minus 1 → 2; r = (250 − 200) / 29 = 1, minus 1 → 0.
    assert_eq!(
        grid_cursor_cell(&g, Point::new(200, 250), (2, 3), (58, 87)),
        Some((2, 0))
    );
    // Leaves the grid on the right: none.
    assert_eq!(
        grid_cursor_cell(&g, Point::new(385, 250), (2, 3), (58, 87)),
        None
    );
}

#[test]
fn an_overflowing_drop_cell_sends_nothing_after_a_valid_move() {
    let (u, files) = ui();
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"qui ")], Some(9));
    // A press inside the grid: drop cell (2, 0), 0x18 [item, 2, 0, page].
    let out = u.press(&w, &files, &layout(), Point::new(200, 250));
    let want = ClientIntent::from_message(&items::insert(9, 2, 0, 0)).0;
    assert_eq!(intents(&out), vec![want]);
    // The next press overhangs the right edge: the stored cell is stale
    // (2, 0), but the drop cell is (9, 0) and fails the placement test.
    let out = u.press(&w, &files, &layout(), Point::new(385, 250));
    assert!(intents(&out).is_empty());
}

#[test]
fn equipment_box_clicks_send_equip_swap_and_unequip() {
    let (mut u, files) = ui();
    u.tips = Some(crate::ui::item_tip::tests::tips());
    u.inv_tables = Some(std::sync::Arc::new(super::equip::tests::tables()));
    let at = Point::new(30, 50);
    // R3–r5: strength, dexterity and level of at least 1.
    let world = |i: &[Fixture], c| {
        let mut w = world(i, c);
        let p = w.units.get_mut(&PLAYER).unwrap();
        for s in [0, 2, 12] {
            p.stats.insert(s, 1);
        }
        w
    };
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"qui ")], Some(9));
    let want = ClientIntent::from_message(&items::equip(9, 3)).0;
    assert_eq!(want[0], 0x1A);
    assert_eq!(intents(&u.press(&w, &files, &layout(), at)), vec![want]);
    let w = world(
        &[
            (9, mode::CURSOR, (0, 0, 0, 0), b"qui "),
            (8, mode::BODY, (3, 0, 0, 0), b"qui "),
        ],
        Some(9),
    );
    assert_eq!(intents(&u.press(&w, &files, &layout(), at))[0][0], 0x1D);
    let w = world(&[(8, mode::BODY, (3, 0, 0, 0), b"qui ")], None);
    assert_eq!(
        intents(&u.press(&w, &files, &layout(), at)),
        vec![vec![0x1C, 3, 0]]
    );
    // Without the inventory tables the check cannot run: nothing is sent.
    u.inv_tables = None;
    assert!(u.press(&w, &files, &layout(), at).is_empty());
}

fn stash_grid() -> GridRecord {
    GridRecord {
        grid_x: 6,
        grid_y: 8,
        left: 154,
        right: 154 + 6 * 29,
        top: 142,
        bottom: 142 + 8 * 29,
        cell_w: 29,
        cell_h: 29,
    }
}

#[test]
fn the_stash_grid_is_record_12_or_8_plus_16_at_800() {
    use crate::ui::panels::stash_items::{fallback_stash_grid, stash_record};
    assert_eq!(stash_record(true, &Screen::R640), 12);
    assert_eq!(stash_record(true, &Screen::R800), 28);
    assert_eq!(stash_record(false, &Screen::R800), 24);
    let g = fallback_stash_grid(true, &Screen::R800);
    assert_eq!((g.left, g.top, g.grid_x, g.grid_y), (154, 142, 6, 8));
    // Layouts of the install win over the fallback.
    let mut u = ui().0;
    let mut l = vec![layout(); 29];
    l[28].grid = stash_grid();
    u.layouts = Some(l);
    assert_eq!(u.stash_grid(true, &Screen::R800), stash_grid());
}

#[test]
fn a_stash_item_draws_at_its_stash_cell_and_not_in_the_inventory() {
    let (u, files) = ui();
    let w = world(
        &[
            (7, mode::STORED, (0, 2, 3, 5), b"hp1 "),
            (8, mode::STORED, (0, 0, 0, 1), b"hp1 "),
        ],
        None,
    );
    let mut out = Vec::new();
    u.draw_stash(&w, &files, &stash_grid(), &mut out);
    let file = files.id(&item_file_name("invhp1")).unwrap();
    // Cell (2, 3): (154 + 58, 142 + 87), drawn at top + frame height 29.
    assert_eq!(images(&out), vec![(file, 0, 212, 258)]);
    let mut inv = Vec::new();
    u.draw_panel(&w, &files, &layout(), &mut inv);
    assert_eq!(images(&inv).len(), 1, "only the page-0 item");
}

#[test]
fn stash_grid_clicks_send_the_page_4_intents() {
    let (u, files) = ui();
    // Cursor item onto the empty stash cell (4, 1): 0x18 with page 4.
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"hp1 ")], Some(9));
    let at = Point::new(154 + 4 * 29 + 3, 142 + 29 + 3);
    let out = u.press_stash(&w, &files, &stash_grid(), at);
    let want = ClientIntent::from_message(&items::insert(9, 4, 1, 4)).0;
    assert_eq!(intents(&out), vec![want]);
    // A stash item without a cursor item lifts (0x19); a page-0 item at
    // the same cell is not under the stash mouse.
    let w = world(
        &[
            (7, mode::STORED, (0, 2, 3, 5), b"hp1 "),
            (8, mode::STORED, (0, 2, 3, 1), b"hp1 "),
        ],
        None,
    );
    let at = Point::new(154 + 2 * 29 + 3, 142 + 3 * 29 + 3);
    let out = u.press_stash(&w, &files, &stash_grid(), at);
    assert_eq!(intents(&out), vec![vec![0x19, 7, 0, 0, 0]]);
    // Outside the grid: nothing.
    assert!(u
        .press_stash(&w, &files, &stash_grid(), Point::new(5, 5))
        .is_empty());
}

mod belt {
    use super::*;
    use crate::ui::original::hud_belt::{BeltParts, HudBelt};
    use crate::ui::panels::control::belt::{BeltBox, BeltRecord};

    fn parts() -> BeltParts {
        // Four boxes in the strip (y 562..590, x 430 + 31 i), record
        // 0 · 7 + type 2.
        let boxes = (0..4)
            .map(|i| BeltBox {
                left: 430 + 31 * i,
                right: 458 + 31 * i,
                top: 562,
                bottom: 590,
            })
            .collect();
        let mut records = vec![BeltRecord { boxes: vec![] }; 2];
        records.push(BeltRecord { boxes });
        BeltParts {
            records,
            types: BTreeMap::new(),
            beltable: [*b"hp1 "].into(),
        }
    }

    /// The fixture items as compact records (the tip decodes those
    /// without the extended fields).
    fn compact(mut w: ClientWorld) -> ClientWorld {
        for u in w.units.values_mut() {
            if let KindData::Item(d) = &mut u.kind {
                if let Some(r) = d.last.as_mut() {
                    r.stream[2] |= (d2_proto::item_bits::hflag::COMPACT >> 16) as u8;
                }
            }
        }
        w
    }

    #[test]
    fn hovering_a_belt_item_yields_its_tip_at_the_box() {
        let tips = crate::ui::item_tip::tests::tips();
        let mut b = HudBelt {
            parts: parts(),
            ..Default::default()
        };
        let w = compact(world(&[(7, mode::BELT, (0, 1, 0, 0), b"cap ")], None));
        // Nothing hovered yet: no tip.
        assert!(b.hover_tip(&w, &tips).is_none());
        // Over box 1 (left 461, top 562): the pop-up's point is (left + 14,
        // top), colour 0, centred.
        b.draw_list(&w, (800, 600), false, (470, 570), true);
        let t = b
            .hover_tip(&w, &tips)
            .expect("the hovered potion has a tip");
        assert_eq!((t.x, t.y, t.color, t.centered), (475, 562, 0, true));
        // T ends with the name in colour 0 (`Prefix(N, 0)`).
        let name = &tips.lines(
            crate::bridge::items::stream(
                &w,
                w.units.keys().copied().find(|k| k.unit_type == 4).unwrap(),
            )
            .unwrap(),
        )[0]
        .text;
        let mut tail = vec![0xFF, u16::from(b'c'), u16::from(b'0')];
        tail.extend_from_slice(name);
        assert!(
            t.text.ends_with(&tail),
            "{:?}",
            String::from_utf16_lossy(&t.text)
        );
        // The short text, not the item tool tip: `Prefix(S, 3)`
        // then `Prefix(N, 0)` of the name and the property lines.
        let stream = crate::bridge::items::stream(
            &w,
            w.units.keys().copied().find(|k| k.unit_type == 4).unwrap(),
        )
        .unwrap();
        let full: Vec<String> = tips
            .lines(stream)
            .iter()
            .map(|l| String::from_utf16_lossy(&l.text))
            .collect();
        // The tool tip's requirement line is not part of it; a plain cap
        // has no property lines, so T is the name alone.
        assert_eq!(full, ["Cap", "Required Level: 3"]);
        assert_eq!(String::from_utf16_lossy(&t.text), "\u{ff}c0Cap");
        // An item on the cursor hides it.
        let w = compact(world(
            &[
                (7, mode::BELT, (0, 1, 0, 0), b"cap "),
                (9, mode::CURSOR, (0, 0, 0, 0), b"cap "),
            ],
            Some(9),
        ));
        assert!(b.hover_tip(&w, &tips).is_none());
        // Off the belt the hover ends.
        let w = compact(world(&[(7, mode::BELT, (0, 1, 0, 0), b"cap ")], None));
        b.draw_list(&w, (800, 600), false, (10, 10), true);
        assert!(b.hover_tip(&w, &tips).is_none());
    }

    #[test]
    fn a_belt_item_draws_in_its_box() {
        let (u, files) = ui();
        let w = world(&[(7, mode::BELT, (0, 1, 0, 0), b"hp1 ")], None);
        let mut b = HudBelt {
            parts: parts(),
            ..Default::default()
        };
        let mut out: Vec<UiDraw> = Vec::new();
        b.draw(&w, &u, &files, (800, 600), false, (0, 0), true, &mut out);
        let f = files.id("*items\\invhp1").unwrap();
        // Box 1: left 461, top 562; cel at top + 29.
        assert_eq!(images(&out), vec![(f, 0, 461, 591)]);
    }

    #[test]
    fn the_belt_draw_list_has_key_labels_and_the_hover_rect() {
        use crate::ui::panels::control::belt::{BeltColor, BeltDraw};
        let w = world(
            &[
                (7, mode::BELT, (0, 1, 0, 0), b"hp1 "),
                (8, mode::BELT, (0, 3, 0, 0), b"hp1 "),
            ],
            None,
        );
        let mut b = HudBelt {
            parts: parts(),
            ..Default::default()
        };
        // Off the belt: labels at (left + 2, bottom - 2), color 4, no rect.
        let d = b.draw_list(&w, (800, 600), false, (0, 0), true);
        let labels: Vec<_> = d
            .iter()
            .filter_map(|d| match d {
                BeltDraw::Label(l) => Some((l.text.clone(), l.x, l.y, l.color)),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            vec![
                (vec![u16::from(b'2')], 463, 588, 4),
                (vec![u16::from(b'4')], 525, 588, 4)
            ]
        );
        assert!(!d.iter().any(|d| matches!(d, BeltDraw::Box { .. })));
        // Over box 1: the item's green 29 x 29 hover rectangle at its box.
        let d = b.draw_list(&w, (800, 600), false, (470, 570), true);
        let boxes: Vec<_> = d
            .iter()
            .filter_map(|d| match d {
                BeltDraw::Box { rect, color } => Some((rect.x, rect.y, rect.w, rect.h, *color)),
                _ => None,
            })
            .collect();
        assert_eq!(boxes, vec![(461, 562, 29, 29, BeltColor::Green)]);
        assert_eq!(b.state.hover_item, Some(7));
    }

    // With a worn belt of type 0 and an item on the cursor, the popped
    // belt's hovered box is outlined: empty and the item fits a belt →
    // green; occupied (a swap possible) → yellow; drawn after the slots.
    #[test]
    fn the_cursor_item_highlights_the_hovered_belt_box() {
        use crate::ui::panels::control::belt::{BeltColor, BeltDraw};
        let boxes = parts().records[2].boxes.clone();
        let mut b = HudBelt {
            parts: BeltParts {
                records: vec![BeltRecord { boxes }; 14],
                types: BTreeMap::from([(*b"lbl ", 0)]),
                beltable: [*b"hp1 "].into(),
            },
            ..Default::default()
        };
        let w = world(
            &[
                (6, mode::BODY, (8, 0, 0, 0), b"lbl "),
                (7, mode::BELT, (0, 1, 0, 0), b"hp1 "),
                (9, mode::CURSOR, (0, 0, 0, 0), b"hp1 "),
            ],
            Some(9),
        );
        let last_box = |d: &[BeltDraw]| match d.last() {
            Some(BeltDraw::Box { rect, color }) => Some((rect.x, rect.y, rect.w, *color)),
            _ => None,
        };
        let d = b.draw_list(&w, (800, 600), false, (440, 570), true);
        assert_eq!(last_box(&d), Some((430, 562, 29, BeltColor::Green)));
        let d = b.draw_list(&w, (800, 600), false, (470, 570), true);
        assert_eq!(last_box(&d), Some((461, 562, 29, BeltColor::Yellow)));
        // Without a cursor item: no outline on the empty box.
        let w = world(&[(6, mode::BODY, (8, 0, 0, 0), b"lbl ")], None);
        let d = b.draw_list(&w, (800, 600), false, (440, 570), true);
        assert_eq!(last_box(&d), None);
    }

    #[test]
    fn a_hovered_belt_slot_paints_its_highlight_rect() {
        let (mut u, files) = ui();
        // The palette's tint colours: red 40, green 41, blue 42, yellow 43.
        u.tint_colors = Some([40, 41, 42, 43, 44]);
        let w = world(&[(7, mode::BELT, (0, 1, 0, 0), b"hp1 ")], None);
        let mut b = HudBelt {
            parts: parts(),
            ..Default::default()
        };
        // (x0, y0, x1, y1, colour, mode).
        let tiles = |out: &[UiDraw]| -> Vec<(i32, i32, i32, i32, u8, u8)> {
            out.iter()
                .filter_map(|d| match d {
                    UiDraw::Rect(r) => Some((r.x0, r.y0, r.x1, r.y1, r.color, r.mode)),
                    _ => None,
                })
                .collect()
        };
        let mut out: Vec<UiDraw> = Vec::new();
        b.draw(&w, &u, &files, (800, 600), false, (0, 0), true, &mut out);
        assert!(tiles(&out).is_empty(), "no hover, no rectangle");
        out.clear();
        b.draw(
            &w,
            &u,
            &files,
            (800, 600),
            false,
            (470, 570),
            true,
            &mut out,
        );
        // 29 x 29 at (461, 562), green, mode 0.
        assert_eq!(tiles(&out), vec![(461, 562, 490, 591, 41, 0)]);
    }

    #[test]
    fn rebinding_a_belt_key_changes_its_label() {
        use crate::controls::{Action, Key, Preset};
        let w = world(&[(7, mode::BELT, (0, 1, 0, 0), b"hp1 ")], None);
        let mut b = HudBelt {
            parts: parts(),
            ..Default::default()
        };
        let label = |b: &mut HudBelt| -> Vec<Vec<u16>> {
            b.draw_list(&w, (800, 600), false, (0, 0), true)
                .into_iter()
                .filter_map(|d| match d {
                    crate::ui::panels::control::belt::BeltDraw::Label(l) => Some(l.text),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(label(&mut b), vec![vec![u16::from(b'2')]]);
        let mut bind = Preset::Dev.bindings().unwrap();
        assert_eq!(bind.inputs(Action::BeltSlot2), &[Key::Digit2]);
        bind.set(Action::BeltSlot2, &[Key::F]);
        b.set_keys(&bind);
        assert_eq!(label(&mut b), vec!["F".encode_utf16().collect::<Vec<_>>()]);
        bind.set(Action::BeltSlot2, &[]);
        b.set_keys(&bind);
        assert!(label(&mut b).is_empty(), "unbound: no label");
    }

    #[test]
    fn clicking_the_belt_takes_and_puts_potions() {
        let b = HudBelt {
            parts: parts(),
            ..Default::default()
        };
        // No cursor item: 0x24 [guid] on the occupied box 1.
        let w = world(&[(7, mode::BELT, (0, 1, 0, 0), b"hp1 ")], None);
        let sent: Vec<Vec<u8>> = b
            .click(&w, false, (475, 570))
            .into_iter()
            .map(|i| i.0)
            .collect();
        assert_eq!(sent, vec![vec![0x24, 7, 0, 0, 0]]);
        // A potion on the cursor and an empty box 2: 0x23 [guid][slot].
        let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"hp1 ")], Some(9));
        let sent: Vec<Vec<u8>> = b
            .click(&w, false, (506, 570))
            .into_iter()
            .map(|i| i.0)
            .collect();
        assert_eq!(sent, vec![vec![0x23, 9, 0, 0, 0, 2, 0, 0, 0]]);
        // Outside the boxes: nothing.
        assert!(b.click(&w, false, (10, 10)).is_empty());
    }

    #[test]
    fn shift_click_sends_0x63() {
        let (mut u, files) = ui();
        u.shift = true;
        let w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"hp1 ")], None);
        let out = u.press(&w, &files, &layout(), Point::new(160, 290));
        assert_eq!(intents(&out), vec![vec![0x63, 7, 0, 0, 0]]);
    }

    #[test]
    fn a_ctrl_click_never_lifts_a_grid_item() {
        let (mut u, files) = ui();
        let w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"hp1 ")], None);
        // Without Ctrl the click lifts (0x19); with it, no store: nothing.
        let out = u.press(&w, &files, &layout(), Point::new(160, 290));
        assert_eq!(intents(&out), vec![vec![0x19, 7, 0, 0, 0]]);
        u.ctrl = true;
        let out = u.press(&w, &files, &layout(), Point::new(160, 290));
        assert!(intents(&out).is_empty());
    }

    #[test]
    fn the_belt_test_reads_the_tables_not_a_code_list() {
        let (mut u, files) = ui();
        u.shift = true;
        // A scroll: its type is beltable in the tables (not in the old list).
        u.art.0.insert(
            *b"isc ",
            ItemArtRow {
                inv_w: 1,
                inv_h: 1,
                inv_file: "invisc".into(),
                flippy_file: String::new(),
                beltable: true,
            },
        );
        assert!(fits_belt(&u.art, Some(*b"isc ")));
        assert!(!fits_belt(&u.art, Some(*b"qui ")));
        assert!(!fits_belt(&u.art, None));
        let w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"isc ")], None);
        let out = u.press(&w, &files, &layout(), Point::new(160, 290));
        assert_eq!(intents(&out), vec![vec![0x63, 7, 0, 0, 0]]);
        // The same item with a non-beltable type is picked up (0x19).
        u.art.0.get_mut(b"isc ").unwrap().beltable = false;
        let out = u.press(&w, &files, &layout(), Point::new(160, 290));
        assert_ne!(intents(&out), vec![vec![0x63, 7, 0, 0, 0]]);
    }
}

fn in_cell(c: i32, r: i32) -> Point {
    Point::new(100 + 29 * c + 5, 200 + 29 * r + 5)
}

#[test]
fn a_right_press_on_identify_then_a_grid_click_sends_0x27() {
    let (u, files) = ui();
    let w = world(
        &[
            (7, mode::STORED, (0, 0, 0, 1), b"isc "),
            (8, mode::STORED, (0, 2, 0, 1), b"hp1 "),
        ],
        None,
    );
    let l = layout();
    // A right press on a non-identify item does nothing.
    assert!(u.right_press(&w, &files, &l, in_cell(2, 0)).is_empty());
    assert!(!u.identify_pending());
    // The right press on the scroll uses it (0x20 arms it on the server).
    let out = u.right_press(&w, &files, &l, in_cell(0, 0));
    assert!(u.identify_pending());
    assert_eq!(intents(&out).len(), 1);
    assert_eq!(intents(&out)[0][0], 0x20);
    assert_eq!(intents(&out)[0][1..5], 7u32.to_le_bytes());
    // Another right press cancels.
    u.right_press(&w, &files, &l, in_cell(0, 0));
    assert!(!u.identify_pending());
    u.right_press(&w, &files, &l, in_cell(0, 0));
    let out = u.press(&w, &files, &l, in_cell(2, 0));
    // 0x27 target, used.
    assert_eq!(
        intents(&out),
        vec![[&[0x27u8][..], &8u32.to_le_bytes(), &7u32.to_le_bytes()].concat()]
    );
    assert!(!u.identify_pending(), "the state ends with the click");
    // Without the state a press lifts the item (0x19).
    let out = u.press(&w, &files, &l, in_cell(2, 0));
    assert_eq!(intents(&out)[0][0], 0x19);
}

#[test]
fn the_hovered_item_is_the_grid_or_equipped_item_under_the_mouse() {
    let (u, files) = ui();
    let w = world(
        &[
            (7, mode::STORED, (0, 0, 0, 1), b"hp1 "),
            (8, mode::BODY, (3, 0, 0, 0), b"qui "),
        ],
        None,
    );
    let l = layout();
    let guid = |p| u.item_at(&w, &files, &l, p).map(|i| i.key.guid);
    assert_eq!(guid(in_cell(0, 0)), Some(7));
    assert_eq!(guid(in_cell(1, 0)), None);
    assert_eq!(guid(Point::new(20, 50)), Some(8));
    // No tips data, or a cursor item: no lines.
    assert!(u
        .hover_lines(&w, &files, &Screen::R640, Some(0), in_cell(0, 0))
        .is_empty());
}

#[test]
fn a_right_press_on_the_cube_sends_0x20_with_the_player_position() {
    let (u, files) = ui();
    let mut w = world(
        &[
            (7, mode::STORED, (0, 0, 0, 1), b"box "),
            (8, mode::STORED, (0, 2, 0, 1), b"hp1 "),
        ],
        None,
    );
    w.units.get_mut(&PLAYER).unwrap().position = Some((10, 20));
    let l = layout();
    // A potion is not opened by a right press.
    assert!(u.right_press(&w, &files, &l, in_cell(2, 0)).is_empty());
    let out = u.right_press(&w, &files, &l, in_cell(0, 0));
    let want = [
        &[0x20u8][..],
        &7u32.to_le_bytes(),
        &10u32.to_le_bytes(),
        &20u32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(intents(&out), vec![want]);
    // With a cursor item nothing is used.
    let mut w = world(
        &[
            (7, mode::STORED, (0, 0, 0, 1), b"box "),
            (9, mode::CURSOR, (0, 0, 0, 0), b"hp1 "),
        ],
        Some(9),
    );
    w.units.get_mut(&PLAYER).unwrap().position = Some((10, 20));
    assert!(u.right_press(&w, &files, &l, in_cell(0, 0)).is_empty());
}

#[test]
fn a_right_press_on_a_town_portal_scroll_or_tome_sends_0x20() {
    let (u, files) = ui();
    let mut w = world(
        &[
            (7, mode::STORED, (0, 0, 0, 1), b"tsc "),
            (8, mode::STORED, (0, 2, 0, 1), b"tbk "),
        ],
        None,
    );
    w.units.get_mut(&PLAYER).unwrap().position = Some((10, 20));
    let l = layout();
    for (cell, guid) in [(0, 7u32), (2, 8)] {
        let out = u.right_press(&w, &files, &l, in_cell(cell, 0));
        let want = [
            &[0x20u8][..],
            &guid.to_le_bytes(),
            &10u32.to_le_bytes(),
            &20u32.to_le_bytes(),
        ]
        .concat();
        assert_eq!(intents(&out), vec![want]);
    }
}

#[test]
fn the_cube_grid_draws_page_3_and_sends_the_page_3_intents() {
    use crate::ui::panels::cube_items::{cube_present, cube_record, fallback_cube_grid};
    let (u, files) = ui();
    assert_eq!(cube_record(&Screen::R640), 9);
    assert_eq!(cube_record(&Screen::R800), 25);
    let g = fallback_cube_grid(&Screen::R800);
    assert_eq!(u.cube_grid(&Screen::R800), g);
    let w = world(
        &[
            (6, mode::STORED, (0, 0, 0, 1), b"box "),
            // The stream's page is the grid page + 1.
            (7, mode::STORED, (0, 1, 2, 4), b"hp1 "),
            (8, mode::STORED, (0, 0, 0, 1), b"hp1 "),
        ],
        None,
    );
    assert!(cube_present(&w));
    let mut out = Vec::new();
    u.draw_cube(&w, &files, &g, Point::new(0, 0), &mut out);
    let file = files.id(&item_file_name("invhp1")).unwrap();
    // Cell (1, 2): (left + 29, top + 58), drawn at top + frame height 29.
    assert_eq!(images(&out), vec![(file, 0, g.left + 29, g.top + 58 + 29)]);
    // Lift the cube item (0x19); a page-0 item is not under the cube mouse.
    let at = Point::new(g.left + 29 + 3, g.top + 58 + 3);
    assert_eq!(
        intents(&u.press_cube(&w, &files, &g, at)),
        vec![vec![0x19, 7, 0, 0, 0]]
    );
    // A cursor item onto the empty cell (0, 0): 0x18 with page 3.
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"hp1 ")], Some(9));
    assert!(!cube_present(&w));
    let at = Point::new(g.left + 3, g.top + 3);
    let want = ClientIntent::from_message(&items::insert(9, 0, 0, 3)).0;
    assert_eq!(intents(&u.press_cube(&w, &files, &g, at)), vec![want]);
    assert!(u.press_cube(&w, &files, &g, Point::new(5, 5)).is_empty());
}

/// The tint palette indices of the tests: refused, fits, usable, swap,
/// unidentified.
const TINTS: [u8; 5] = [40, 41, 42, 43, 44];

/// The (x, y, w, h) of the mode-0 rectangles of colour `color`.
fn tiles(d: &[UiDraw], color: u8) -> Vec<(i32, i32, i32, i32)> {
    d.iter()
        .filter_map(|d| match d {
            UiDraw::Rect(r) if r.color == color => {
                assert_eq!(r.mode, 0, "tints draw in mode 0 (§2 r2)");
                Some((r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0))
            }
            _ => None,
        })
        .collect()
}

/// A cap (requires level 3) worn in the head box, the player at `level`;
/// the mouse at `mouse`.
fn cap_tints(level: i32, mouse: Point) -> Vec<UiDraw> {
    let (mut u, mut files) = ui();
    u.art.0.insert(
        *b"cap ",
        ItemArtRow {
            inv_w: 2,
            inv_h: 2,
            inv_file: "invcap".into(),
            flippy_file: String::new(),
            beltable: false,
        },
    );
    u.tips = Some(crate::ui::item_tip::tests::tips());
    u.tint_colors = Some(TINTS);
    u.register_files(&mut files);
    let mut w = world(&[(8, mode::BODY, (1, 0, 0, 0), b"cap ")], None);
    w.units.get_mut(&PLAYER).unwrap().stats.insert(12, level);
    let mut l = layout();
    l.equip[1] = EquipBox {
        left: 30,
        top: 40,
        w: 58,
        h: 18,
        right: 87,
        bottom: 57,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_tints(&w, &l, mouse, &mut out);
    out
}

#[test]
fn an_equipped_item_over_the_players_level_paints_the_red_tint() {
    let out = cap_tints(1, Point::new(0, 0));
    assert_eq!(tiles(&out, TINTS[0]), vec![(30, 40, 58, 18)]);
}

#[test]
fn a_usable_identified_equipped_item_paints_no_tint() {
    let out = cap_tints(3, Point::new(0, 0));
    assert!(out.is_empty(), "{} fills", out.len());
}

#[test]
fn a_hovered_equipped_item_paints_the_green_tint() {
    let out = cap_tints(1, Point::new(40, 45));
    assert_eq!(tiles(&out, TINTS[1]), vec![(30, 40, 58, 18)]);
    assert!(tiles(&out, TINTS[0]).is_empty());
}

#[test]
fn grid_items_paint_blue_usable_and_red_refused_tints() {
    let (mut u, _) = ui();
    u.art.0.insert(
        *b"cap ",
        ItemArtRow {
            inv_w: 1,
            inv_h: 1,
            inv_file: "invcap".into(),
            flippy_file: String::new(),
            beltable: false,
        },
    );
    u.tips = Some(crate::ui::item_tip::tests::tips());
    u.tint_colors = Some(TINTS);
    let w = |level| {
        let mut w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"cap ")], None);
        w.units.get_mut(&PLAYER).unwrap().stats.insert(12, level);
        w
    };
    let l = layout();
    let (mut ok, mut bad) = (Vec::new(), Vec::new());
    u.draw_tints(&w(3), &l, Point::new(0, 0), &mut ok);
    u.draw_tints(&w(1), &l, Point::new(0, 0), &mut bad);
    // Cell (2, 3): one 29 × 29 rectangle at (158, 287).
    let cell = vec![(158, 287, 29, 29)];
    assert_eq!(tiles(&ok, TINTS[2]), cell);
    assert_eq!(tiles(&bad, TINTS[0]), cell);
    // No palette given: no tint (the colours are the palette's).
    u.tint_colors = None;
    let mut none = Vec::new();
    u.draw_tints(&w(1), &l, Point::new(0, 0), &mut none);
    assert!(none.is_empty());
}

#[test]
fn the_red_fill_frame_is_the_palette_entry_nearest_to_0x80_0_0() {
    use crate::ui::original::hud::{fill_frames, BELT_FILL_BASE};
    let mut colors = [d2_formats::palette::Rgb::default(); 256];
    colors[9] = d2_formats::palette::Rgb {
        r: 0x80,
        g: 0,
        b: 0,
    };
    let pal = d2_formats::palette::Palette { colors };
    let frames = fill_frames(&pal);
    let red = &frames[BELT_FILL_BASE as usize];
    assert!(red.pixels.iter().all(|&p| p == 9));
}

/// Per item, every cell of the footprint is tinted (one cell box each,
/// row by row, left to right), then the item is drawn; the next item
/// follows (`a1-panel-cube` rows 5–9 and 25–40).
#[test]
fn each_item_tints_its_cells_row_by_row_then_draws() {
    let (mut u, mut files) = ui();
    u.art.0.insert(
        *b"cap ",
        ItemArtRow {
            inv_w: 2,
            inv_h: 2,
            inv_file: "invcap".into(),
            flippy_file: String::new(),
            beltable: false,
        },
    );
    u.tips = Some(crate::ui::item_tip::tests::tips());
    u.tint_colors = Some(TINTS);
    u.register_files(&mut files);
    let mut w = world(
        &[
            (7, mode::STORED, (0, 2, 3, 1), b"cap "),
            (8, mode::STORED, (0, 0, 0, 1), b"hp1 "),
        ],
        None,
    );
    w.units.get_mut(&PLAYER).unwrap().stats.insert(12, 3);
    let l = layout();
    let mut out: Vec<UiDraw> = Vec::new();
    u.draw_items(&w, &files, &l, Point::new(0, 0), &mut out);
    let kinds: Vec<String> = out
        .iter()
        .map(|d| match d {
            UiDraw::Rect(r) => format!("box {} {}", r.x0, r.y0),
            UiDraw::Image(i) => format!("cel {}", i.at.x),
            UiDraw::Text(_) => "text".into(),
            UiDraw::Line(_) => "line".into(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "box 158 287",
            "box 187 287",
            "box 158 316",
            "box 187 316",
            "cel 158",
            "box 100 200",
            "cel 100",
        ]
    );
    // `draw_tints` alone paints the same cell boxes, without the cels.
    let mut old = Vec::new();
    u.draw_tints(&w, &l, Point::new(0, 0), &mut old);
    assert_eq!(tiles(&old, TINTS[2]).len(), 5);
}

#[test]
fn the_kept_cursor_cell_survives_an_overhanging_mouse_move() {
    // Record 16 of the spec vector: left 419, top 315, 29 x 29 cells,
    // 10 x 4; a 2 x 3 item with a 56 x 84 graphic on the cursor.
    let (mut u, files) = ui();
    u.frame_sizes.insert("invqlt".into(), (56, 84));
    let g = GridRecord {
        grid_x: 10,
        grid_y: 4,
        left: 419,
        right: 709,
        top: 315,
        bottom: 431,
        cell_w: 29,
        cell_h: 29,
    };
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"qui ")], Some(9));
    u.track_hover(&w, &files, &g, 0, Point::new(500, 340));
    assert_eq!(u.hover.get().cursor_cell, (2, 0));
    // c = (14 - 419 + 700) / 29 = 10 -> 9; 2 + 9 > 10: no change.
    u.track_hover(&w, &files, &g, 0, Point::new(700, 340));
    assert_eq!(u.hover.get().cursor_cell, (2, 0));
}

#[test]
fn the_placement_tint_covers_the_footprint_and_marks_a_blocked_one() {
    let (mut u, files) = ui();
    u.tint_colors = Some(TINTS);
    let g = layout().grid;
    // A 1 x 1 cursor item over the empty cell (4, 1): tint 1 (fits).
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"hp1 ")], Some(9));
    let at = in_cell(4, 1);
    u.track_hover(&w, &files, &g, 0, at);
    let mut out = Vec::new();
    u.draw_placement_tint(&w, &files, &g, 0, at, 600, &mut out);
    assert_eq!(
        tiles(&out, TINTS[1]),
        vec![(100 + 4 * 29, 200 + 29, 29, 29)]
    );
    // Over an item: a swap candidate, tint 3 over that item.
    let w = world(
        &[
            (9, mode::CURSOR, (0, 0, 0, 0), b"hp1 "),
            (7, mode::STORED, (0, 4, 1, 1), b"hp1 "),
        ],
        Some(9),
    );
    let mut out = Vec::new();
    u.draw_placement_tint(&w, &files, &g, 0, at, 600, &mut out);
    assert_eq!(
        tiles(&out, TINTS[3]),
        vec![(100 + 4 * 29, 200 + 29, 29, 29)]
    );
    // Not with the mouse in the bottom strip (screen_h - 0x27).
    let mut out = Vec::new();
    u.draw_placement_tint(&w, &files, &g, 0, at, 240, &mut out);
    assert!(out.is_empty());
}

#[test]
fn a_big_cursor_item_tints_one_box_per_cell() {
    let (mut u, files) = ui();
    u.tint_colors = Some(TINTS);
    u.frame_sizes.insert("invqlt".into(), (56, 84));
    let g = layout().grid;
    // A 2 x 3 item over free cells: six boxes, row by row.
    let w = world(&[(9, mode::CURSOR, (0, 0, 0, 0), b"qui ")], Some(9));
    let at = in_cell(2, 0);
    u.track_hover(&w, &files, &g, 0, at);
    let (c, r) = u.hover.get().cursor_cell;
    let mut out = Vec::new();
    u.draw_placement_tint(&w, &files, &g, 0, at, 600, &mut out);
    let (x0, y0) = (100 + 29 * c, 200 + 29 * r);
    let want: Vec<_> = (0..3)
        .flat_map(|row| (0..2).map(move |col| (x0 + 29 * col, y0 + 29 * row, 29, 29)))
        .collect();
    assert_eq!(tiles(&out, TINTS[1]), want);
}

#[test]
fn a_cursor_item_over_the_cube_with_room_sends_0x2a() {
    let (u, files) = ui();
    let w = world(
        &[
            (9, mode::CURSOR, (0, 0, 0, 0), b"hp1 "),
            (6, mode::STORED, (0, 4, 1, 1), b"box "),
        ],
        Some(9),
    );
    let out = u.press(&w, &files, &layout(), in_cell(4, 1));
    assert_eq!(
        intents(&out),
        vec![[&[0x2Au8][..], &9u32.to_le_bytes(), &6u32.to_le_bytes()].concat()]
    );
}

#[test]
fn a_ctrl_click_with_the_store_open_sells_with_0x33() {
    let (mut u, files) = ui();
    u.tips = Some(crate::ui::item_tip::tests::tips());
    u.ctrl = true;
    u.store_npc = Some(0x55);
    let w = world(&[(7, mode::STORED, (0, 2, 3, 1), b"cap ")], None);
    let out = u.press(&w, &files, &layout(), Point::new(160, 290));
    let got = intents(&out);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0][0], 0x33);
    assert_eq!(&got[0][1..5], &0x55u32.to_le_bytes());
    assert_eq!(&got[0][5..9], &7u32.to_le_bytes());
}

#[test]
fn an_equipment_box_includes_its_right_and_bottom_edges() {
    let (u, files) = ui();
    let w = world(&[(8, mode::BODY, (3, 0, 0, 0), b"qui ")], None);
    // The torso box: left 10, right 67, top 20, bottom 106, inclusive.
    for (p, hit) in [
        (Point::new(67, 106), true),
        (Point::new(10, 20), true),
        (Point::new(68, 106), false),
        (Point::new(67, 107), false),
    ] {
        assert_eq!(super::equip_loc(&layout(), p).is_some(), hit, "{p:?}");
    }
    let _ = (u, files, w);
}
