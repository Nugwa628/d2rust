//! Coverage tests (c2-ui-render session) for panel rules.
use super::waypoint::close_hover;
use crate::ui::geom::Point;
use crate::ui::layout::Screen;

#[test]
fn waypoint_close_hover_area_and_tip() {
    let s = Screen::R640;
    let (sx, sy) = (s.sx(), s.sy());
    let h = close_hover(&s, Point::new(sx + 273, 387 - sy)).unwrap();
    assert_eq!(h.tooltip, 4130);
    assert_eq!((h.tooltip_cx, h.tooltip_y), (sx + 291, 385 - sy));
    assert!(close_hover(&s, Point::new(sx + 272, 400 - sy)).is_none());
    assert!(close_hover(&s, Point::new(sx + 290, 386 - sy)).is_none());
}

#[test]
fn panel_area_bottom_inclusive() {
    use super::inventory::in_panel_area;
    // record 0 at 640 x 480: x 320-639, y 0-441
    assert!(in_panel_area(320, 640, 0, 441, 320, 0));
    assert!(in_panel_area(320, 640, 0, 441, 639, 441));
    assert!(!in_panel_area(320, 640, 0, 441, 640, 100));
    assert!(!in_panel_area(320, 640, 0, 441, 400, 442));
    assert!(!in_panel_area(320, 640, 0, 441, 319, 100));
}
