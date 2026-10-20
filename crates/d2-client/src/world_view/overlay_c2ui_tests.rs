//! Coverage tests (c2-ui-render session).
use super::*;

#[test]
fn overlay_light_rules() {
    let mut row = OverlayRow::default();
    assert_eq!(overlay_light(&row), None);
    row.radius = 5;
    row.init_radius = 5;
    assert_eq!(
        overlay_light(&row),
        Some(OverlayLight {
            init_radius: 5,
            target_radius: None
        })
    );
    row.init_radius = 2;
    assert_eq!(overlay_light(&row).unwrap().target_radius, Some(5));
}

#[test]
fn kind6_clock_is_real_time_per_update() {
    assert_eq!(kind6_clock_ms(0, TickRate::BASE), 0);
    assert_eq!(kind6_clock_ms(25, TickRate::BASE), 1000);
    // The same second at the game's 60 Hz.
    let r = TickRate::new(60).unwrap();
    assert_eq!(kind6_clock_ms(60, r), 1000);
    assert_eq!(kind6_clock_ms(90, r), 1500);
}

#[test]
fn overlay_draw_filter() {
    let mut o = Overlay {
        kind: 2,
        frame: 3 * 256,
        frames: 10 * 256,
        pre_draw: true,
        ..Overlay::default()
    };
    assert!(overlay_draws(&o, true));
    assert!(!overlay_draws(&o, false));
    o.pre_draw = false;
    assert!(overlay_draws(&o, false));
    o.frame = 10 * 256;
    assert!(!overlay_draws(&o, false));
    o.frame = 0;
    o.kind = 6;
    o.a = 0;
    assert!(!overlay_draws(&o, false));
    o.kind = 8;
    o.active = false;
    assert!(!overlay_draws(&o, false));
    o.active = true;
    assert!(overlay_draws(&o, false));
}
