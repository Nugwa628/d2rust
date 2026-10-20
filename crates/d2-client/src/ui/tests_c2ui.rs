//! Coverage tests (c2-ui-render session).
use super::inv_grid::*;

fn rec() -> GridRecord {
    GridRecord {
        grid_x: 10,
        grid_y: 4,
        left: 10,
        right: 300,
        top: 20,
        bottom: 200,
        cell_w: 29,
        cell_h: 29,
    }
}

#[test]
fn equipped_items_never_usable_tint() {
    for bits in 0u32..128 {
        let f = EquipItemFacts {
            hovered: false,
            cursor_state_8: false,
            transmogrify: false,
            ghost: bits & 1 != 0,
            shoots_or_quiver: bits & 2 != 0,
            grid: GridItemFacts {
                requirements_fail: bits & 4 != 0,
                flag_4: bits & 8 != 0,
                busy: bits & 16 != 0,
                not_usable: bits & 32 != 0,
                identified: bits & 64 != 0,
                quest_blocked: false,
            },
        };
        assert_ne!(equip_item_tint(&f), Some(Tint::Usable));
    }
    // a clean identified grid item is tint 2
    let g = GridItemFacts {
        identified: true,
        ..Default::default()
    };
    assert_eq!(grid_item_tint(&g), Tint::Usable);
    // red / unidentified win over blue
    let g = GridItemFacts {
        identified: false,
        ..Default::default()
    };
    assert_eq!(grid_item_tint(&g), Tint::Unidentified);
}

#[test]
fn partly_visible_footprint_partly_tinted() {
    let r = rec();
    // clip 640x480; cell column 20 starts at x = 10 + 29*20 = 590, col 22 at 648
    assert!(r.cell_tinted(20, 0, 640, 480));
    assert!(!r.cell_tinted(22, 0, 640, 480));
}
