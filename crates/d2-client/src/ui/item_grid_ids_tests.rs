//! Item grid ids use the production root and pointer handlers on synthetic models.
use super::*;
use crate::bridge::items::{self, mode, ItemArtRow};
use crate::bridge::world::KindData;
use crate::ui::panels::inv_items::tests::world as item_world;
use crate::ui::states::id;
use crate::ui::ui_ids::{find, UiElement};

fn setup(screen: Screen, panel: u8) -> Ui {
    let mut u = ui_at(screen, Some(areas()), true);
    let mut art = ItemArtRows::default();
    art.0.insert(
        *b"hp1 ",
        ItemArtRow {
            inv_w: 1,
            inv_h: 1,
            inv_file: "invhp1".into(),
            flippy_file: String::new(),
            beltable: false,
        },
    );
    u.ui.set_item_art(art);
    u.ui.set_ui(u32::from(panel), 0, false).unwrap();
    u.ui.sync_root(&mut u.root);
    u
}

fn elements(u: &Ui, w: &ClientWorld) -> Vec<UiElement> {
    u.root.ui_elements(&UiCtx {
        tick: 0,
        world: w,
        strings: &NoStrings,
    })
}

fn click_id(u: &mut Ui, w: &ClientWorld, id: &str) {
    let at = find(&elements(u, w), id).unwrap().at;
    u.send(w, UiEvent::CursorMoved(at));
    u.click(w, at);
}

fn grid_clicks(panel: u8, name: &str, page: u8) {
    for (screen, expansion) in [
        (Screen::R800, false),
        (Screen::R800, true),
        (Screen::WIDE, false),
        (Screen::WIDE, true),
    ] {
        let mut u = setup(screen, panel);
        let mut w = item_world(&[(7, mode::STORED, (0, 0, 0, page + 1), b"hp1 ")], None);
        w.expansion = u32::from(expansion);
        u.send(&w, UiEvent::CursorMoved(Point::new(0, 0)));
        let els = elements(&u, &w);
        let occupied = find(&els, &format!("{name}.slot(0,0)")).unwrap();
        let empty = find(&els, &format!("{name}.slot(1,0)")).unwrap();
        let sh = u.ui.shared.borrow();
        let g = match panel {
            id::STASH => sh.items.stash_grid(expansion, &screen),
            id::CUBE => sh.items.cube_grid(&screen),
            _ => sh.items.layout(Some(0), &screen).unwrap().grid,
        };
        for e in [occupied, empty] {
            assert!(g.contains_mouse(e.at));
            assert!(e.rect.contains(e.at));
        }
        let cells: Vec<_> = els
            .iter()
            .filter(|e| e.id.starts_with(&format!("{name}.slot(")))
            .collect();
        assert_eq!(cells.len(), usize::from(g.grid_x) * usize::from(g.grid_y));
        for e in cells {
            assert!(g.contains_mouse(e.at));
            assert!(g.contains_mouse(Point::new(e.rect.x, e.rect.y)));
            assert!(g.contains_mouse(Point::new(
                e.rect.x + i32::from(e.rect.w) - 1,
                e.rect.y + i32::from(e.rect.h) - 1
            )));
        }
        drop(sh);
        click_id(&mut u, &w, &format!("{name}.slot(0,0)"));
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(&items::remove(7))]
        );
        let mut cursor = item_world(&[(8, mode::CURSOR, (0, 0, 0, 0), b"hp1 ")], Some(8));
        cursor.expansion = u32::from(expansion);
        click_id(&mut u, &cursor, &format!("{name}.slot(1,0)"));
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(&items::insert(
                8,
                1,
                0,
                u32::from(page)
            ))]
        );
        let error = find(&els, "unknown.slot(99,99)").unwrap_err();
        assert!(error.contains(&format!("{name}.slot(0,0)")));
        assert!(find(&els, &format!("{name}.slot(99,99)")).is_err());
        u.ui.set_ui(u32::from(panel), 1, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with(&format!("{name}."))));
    }
}

#[test]
fn inventory_ids_reach_lift_and_place() {
    grid_clicks(id::INVENTORY, "inventory", 0);
}
#[test]
fn stash_ids_reach_lift_and_place() {
    grid_clicks(id::STASH, "stash", 4);
}
#[test]
fn cube_ids_reach_lift_and_place() {
    grid_clicks(id::CUBE, "cube", 3);
}

#[test]
fn stash_and_cube_button_ids_reach_existing_handlers() {
    for screen in [Screen::R800, Screen::WIDE] {
        let w = item_world(&[], None);
        for (panel, name) in [(id::STASH, "stash"), (id::CUBE, "cube")] {
            let mut u = setup(screen, panel);
            if panel == id::CUBE {
                click_id(&mut u, &w, "cube.transmute");
                assert!(u
                    .root
                    .take_intents()
                    .iter()
                    .any(|i| i.0.starts_with(&[0x4f, 0x18])));
            }
            click_id(&mut u, &w, &format!("{name}.close"));
            assert!(!u.ui.is_open(panel));
        }
    }
}

#[test]
fn shop_ids_use_visible_page_and_confirm_answers_cover_the_grids() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = setup(screen, id::NPC_SHOP);
        u.ui.set_ui(u32::from(id::NPC_MENU), 0, false).unwrap();
        u.ui.open_shop(99, 147);
        u.ui.sync_root(&mut u.root);
        let mut w = item_world(
            &[
                (7, mode::STORED, (0, 0, 0, 1), b"hp1 "),
                (8, mode::STORED, (0, 1, 0, 2), b"hp1 "),
            ],
            None,
        );
        for guid in [7, 8] {
            let KindData::Item(d) = &mut w
                .units
                .get_mut(&UnitKey::new(items::ITEM, guid))
                .unwrap()
                .kind
            else {
                panic!()
            };
            let record = d.last.as_mut().unwrap();
            record.action = 0x0b;
            record.seq = guid;
        }
        let els = elements(&u, &w);
        let g = super::super::shop_ui::grid_record(&u.ui.shared.borrow());
        let cells: Vec<_> = els
            .iter()
            .filter(|e| e.id.starts_with("shop.slot("))
            .collect();
        assert_eq!(cells.len(), usize::from(g.grid_x) * usize::from(g.grid_y));
        for e in cells {
            assert!(g.contains_mouse(e.at));
            assert!(g.contains_mouse(Point::new(e.rect.x, e.rect.y)));
            assert!(g.contains_mouse(Point::new(
                e.rect.x + i32::from(e.rect.w) - 1,
                e.rect.y + i32::from(e.rect.h) - 1
            )));
        }
        assert!(find(&elements(&u, &w), "shop.tab(2)").is_err());
        // Page 0 stocks one item at cell (0,0): `shop.item(0)` is its cell
        // and there is no second item.
        let els = elements(&u, &w);
        let (first, cell) = (
            find(&els, "shop.item(0)").unwrap(),
            find(&els, "shop.slot(0,0)").unwrap(),
        );
        assert_eq!((first.rect.x, first.rect.y), (cell.rect.x, cell.rect.y));
        assert!(find(&els, "shop.item(1)").is_err());
        click_id(&mut u, &w, "shop.tab(1)");
        assert!(u.ui.store_item_point(&w, 8).is_some());
        assert!(u.ui.store_item_point(&w, 7).is_none());
        // Page 1 stocks its item at (1,0): the id follows the item, not cell (0,0).
        let els = elements(&u, &w);
        let (second, cell) = (
            find(&els, "shop.item(0)").unwrap(),
            find(&els, "shop.slot(1,0)").unwrap(),
        );
        assert_eq!((second.rect.x, second.rect.y), (cell.rect.x, cell.rect.y));
        assert!(find(&els, "shop.item(1)").is_err());
        click_id(&mut u, &w, "shop.item(0)");
        assert!(
            u.ui.shop.borrow().confirm.is_some(),
            "the stocked item's id opens its confirm dialog"
        );
        click_id(&mut u, &w, "shop.confirm.no");
        click_id(&mut u, &w, "shop.slot(0,0)");
        assert!(
            u.ui.shop.borrow().confirm.is_none(),
            "page 0 item is not hit on page 1"
        );
        click_id(&mut u, &w, "shop.slot(1,0)");
        assert!(u.ui.shop.borrow().confirm.is_some());
        let els = elements(&u, &w);
        assert!(find(&els, "shop.confirm.yes").is_ok());
        assert!(!els.iter().any(|e| e.id.starts_with("shop.slot")
            || e.id.starts_with("shop.tab")
            || e.id.starts_with("inventory.")));
        click_id(&mut u, &w, "shop.confirm.no");
        assert!(u.ui.shop.borrow().confirm.is_none());
        click_id(&mut u, &w, "shop.slot(1,0)");
        click_id(&mut u, &w, "shop.confirm.yes");
        assert!(u
            .root
            .take_intents()
            .iter()
            .any(|i| i.0.first() == Some(&0x32)));
        u.ui.set_ui(u32::from(id::NPC_SHOP), 1, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert!(!elements(&u, &w).iter().any(|e| e.id.starts_with("shop.")));
    }
}

#[test]
fn equipment_ids_resolve_empty_and_occupied_boxes_and_reach_body_handler() {
    use crate::ui::inv_grid::EquipBox;
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = setup(screen, id::INVENTORY);
        {
            let mut sh = u.ui.shared.borrow_mut();
            let mut layout = sh.items.layout(Some(0), &screen).unwrap();
            layout.equip[3] = EquipBox {
                left: screen.right_start() + 100,
                right: screen.right_start() + 157,
                top: 150,
                bottom: 236,
                w: 58,
                h: 87,
            };
            sh.items.layouts = Some(vec![layout; 32]);
            sh.items.tips = Some(crate::ui::item_tip::tests::tips());
            sh.items.inv_tables = Some(std::sync::Arc::new(
                crate::ui::panels::inv_items::equip::tests::tables(),
            ));
        }
        let empty = item_world(&[], None);
        let worn = item_world(&[(7, mode::BODY, (3, 0, 0, 0), b"qui ")], None);
        u.send(&worn, UiEvent::CursorMoved(Point::new(0, 0)));
        assert_eq!(
            find(&elements(&u, &empty), "inventory.equip(3)").unwrap(),
            find(&elements(&u, &worn), "inventory.equip(3)").unwrap()
        );
        click_id(&mut u, &worn, "inventory.equip(3)");
        assert_eq!(u.root.take_intents(), vec![ClientIntent(vec![0x1c, 3, 0])]);
    }
}

#[test]
fn cube_animation_and_dead_player_hide_cells() {
    let u = setup(Screen::R800, id::CUBE);
    let mut w = item_world(&[], None);
    let mut anim = u.ui.shared.borrow().cube_anim.get();
    anim.start(0);
    u.ui.shared.borrow().cube_anim.set(anim);
    let els = elements(&u, &w);
    assert!(!els.iter().any(|e| e.id.starts_with("cube.slot")));
    assert!(find(&els, "cube.transmute").is_ok());
    w.units.get_mut(&w.local_player.unwrap()).unwrap().mode = 0x11;
    assert!(!elements(&u, &w).iter().any(|e| e.id.starts_with("cube.")));
}
