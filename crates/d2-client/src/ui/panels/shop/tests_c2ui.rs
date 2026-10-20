//! Shop buttons and shop tabs.
use super::*;

#[test]
fn shop_buttons_by_npc() {
    let a = shop_button_records(148);
    assert_eq!(a.len(), 4);
    assert_eq!(
        (a[0].string, a[0].base_frame, a[0].enabled),
        (3335, 2, true)
    );
    assert_eq!((a[1].string, a[1].base_frame), (3336, 4));
    assert!(!a[2].enabled);
    assert_eq!((a[3].string, a[3].base_frame), (4144, 10));
    let r = shop_button_records(154);
    assert_eq!((r[2].string, r[2].base_frame), (3338, 6));
    assert_eq!((r[3].string, r[3].base_frame), (10095, 18));
    assert!(shop_button_records(1).is_empty());
}

#[test]
fn shop_tabs_and_start_page() {
    assert_eq!(shop_start_page(147), (0, false));
    assert_eq!(shop_start_page(154), (1, false));
    assert_eq!(shop_start_page(148), (3, true));
    assert_eq!(shop_start_page(7), (0, false));
    // page 3 empty: steps to 0 (above max 3), which has items
    let (t, cur) = shop_tabs(3, [2, 0, 0, 0, 0], 3);
    assert_eq!(cur, 0);
    assert_eq!(t[0], (true, true));
    assert_eq!(t[3], (false, false));
    // pages > 4 read as 0
    let (_, cur) = shop_tabs(9, [1, 1, 0, 0, 0], 3);
    assert_eq!(cur, 0);
    // steps +1
    let (t, cur) = shop_tabs(1, [0, 0, 5, 0, 0], 3);
    assert_eq!(cur, 2);
    assert_eq!(t[2], (true, true));
}
