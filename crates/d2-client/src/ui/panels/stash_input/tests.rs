use super::*;
use crate::ui::panel::ClientIntent;

const S: Screen = Screen::R800; // sx 80, sy −60, W 800, H 600

fn ptr(x: i32, y: i32) -> Pointer {
    Pointer {
        at: Point::new(x, y),
        in_inv_close: false,
        cursor_item: false,
    }
}

fn msg(b: u16) -> PanelOutput {
    PanelOutput::Intent(ClientIntent(vec![0x4F, b as u8, 0, 0, 0, 0, 0]))
}

fn set_ui(ui: u8) -> PanelOutput {
    PanelOutput::SetUi {
        ui,
        mode: 1,
        jump: false,
    }
}

#[test]
fn stash_press_rectangles() {
    // stash gold button, expansion: x [153, 230], y [85, 102] (H + sy = 540)
    assert!(stash_gold_rect_hit(&S, true, Point::new(153, 85)));
    assert!(stash_gold_rect_hit(&S, true, Point::new(230, 102)));
    assert!(!stash_gold_rect_hit(&S, true, Point::new(231, 100)));
    assert!(!stash_gold_rect_hit(&S, true, Point::new(200, 103)));
    // classic: y [281, 298]
    assert!(stash_gold_rect_hit(&S, false, Point::new(200, 281)));
    assert!(!stash_gold_rect_hit(&S, false, Point::new(200, 280)));
    // stash close, strict: expansion X = 352, Y = 476
    for (p, inside) in [
        ((353, 442), true),
        ((391, 480), true),
        ((352, 460), false),
        ((392, 460), false),
        ((370, 441), false),
        ((370, 481), false),
    ] {
        assert_eq!(
            stash_close_hit(&S, true, Point::new(p.0, p.1)),
            inside,
            "{p:?}"
        );
    }
    // the hover (inclusive) is wider by one pixel; tool tip at (X + 12 − w/2, Y − 35)
    assert_eq!(
        stash_close_tooltip(&S, true, 30),
        Point::new(352 + 12 - 15, 476 - 35)
    );
    // mouse down: inventory close first, then gold, then close; each sound 4
    let mut f = StashCubeInput::default();
    let e = f.stash_down(
        &S,
        true,
        &Pointer {
            in_inv_close: true,
            ..ptr(0, 0)
        },
    );
    assert!(e.consumed && e.sound4 && f.inv_close);
    let mut f = StashCubeInput::default();
    let e = f.stash_down(&S, true, &ptr(160, 90));
    assert!(e.consumed && e.sound4 && f.stash_gold && !f.stash_close_flag);
    let mut f = StashCubeInput::default();
    let e = f.stash_down(&S, true, &ptr(370, 460));
    assert!(e.consumed && e.sound4);
    assert!(f.stash_close_flag && f.stash_close_pressed && f.latch_ce9c);
    // elsewhere: not consumed, no sound
    let mut f = StashCubeInput::default();
    assert_eq!(
        f.stash_down(&S, true, &ptr(5, 5)),
        StashCubeEffect::default()
    );
    assert_eq!(f, StashCubeInput::default());
}

#[test]
fn stash_release_message_counts() {
    // inventory close rectangle, no cursor item: one 0x12 (the hook's)
    let mut f = StashCubeInput::default();
    let e = f.stash_up(
        &S,
        true,
        &Pointer {
            in_inv_close: true,
            ..ptr(0, 0)
        },
    );
    assert_eq!(e.outputs, vec![set_ui(0x19), msg(0x12)]);
    // with a cursor item: nothing
    let e = f.stash_up(
        &S,
        true,
        &Pointer {
            in_inv_close: true,
            cursor_item: true,
            ..ptr(0, 0)
        },
    );
    assert!(e.outputs.is_empty() && !e.consumed);
    // stash close rectangle with the button pressed: pressed:= 0, two 0x12
    let mut f = StashCubeInput {
        stash_close_pressed: true,
        ..Default::default()
    };
    let e = f.stash_up(&S, true, &ptr(370, 460));
    assert_eq!(e.outputs, vec![set_ui(0x19), msg(0x12), msg(0x12)]);
    assert!(!f.stash_close_pressed);
    // not pressed: nothing
    let e = f.stash_up(&S, true, &ptr(370, 460));
    assert!(e.outputs.is_empty());
}

#[test]
fn cube_press_rectangles() {
    // cube close, strict: sx + 275 < x < sx + 315, H + sy − 100 < y < H + sy − 60
    assert!(cube_close_hit(&S, Point::new(356, 441)));
    assert!(cube_close_hit(&S, Point::new(394, 479)));
    assert!(!cube_close_hit(&S, Point::new(355, 460)));
    assert!(!cube_close_hit(&S, Point::new(395, 460)));
    assert!(!cube_close_hit(&S, Point::new(370, 440)));
    assert!(!cube_close_hit(&S, Point::new(370, 480)));
    // transmute: sx + 144 < x < sx + 184, H + sy − 223 < y < H + sy − 183
    assert!(transmute_hit(&S, Point::new(225, 318)));
    assert!(transmute_hit(&S, Point::new(263, 356)));
    assert!(!transmute_hit(&S, Point::new(224, 330)));
    assert!(!transmute_hit(&S, Point::new(264, 330)));
    assert!(!transmute_hit(&S, Point::new(240, 317)));
    assert!(!transmute_hit(&S, Point::new(240, 357)));
    // inventory close
    let mut f = StashCubeInput::default();
    let e = f.cube_down(
        &S,
        &Pointer {
            in_inv_close: true,
            ..ptr(0, 0)
        },
    );
    assert!(e.consumed && e.sound4 && f.inv_close);
    // cube close: flags, transmute cleared
    let mut f = StashCubeInput {
        transmute_flag: true,
        transmute_pressed: true,
        ..Default::default()
    };
    let e = f.cube_down(&S, &ptr(370, 460));
    assert!(e.consumed && e.sound4);
    assert!(f.cube_close_flag && f.cube_close_pressed && f.latch_ce9c);
    assert!(!f.transmute_flag && !f.transmute_pressed);
    // transmute: close flags cleared; with an empty cursor the flags set
    let mut f = StashCubeInput {
        cube_close_flag: true,
        cube_close_pressed: true,
        ..Default::default()
    };
    let e = f.cube_down(&S, &ptr(240, 330));
    assert!(e.consumed && e.sound4);
    assert!(f.transmute_flag && f.transmute_pressed && f.latch_ce9c);
    assert!(!f.cube_close_flag && !f.cube_close_pressed);
    // ... holding an item: consumed, nothing set, no sound
    let mut f = StashCubeInput::default();
    let e = f.cube_down(
        &S,
        &Pointer {
            cursor_item: true,
            ..ptr(240, 330)
        },
    );
    assert!(e.consumed && !e.sound4 && !f.transmute_pressed);
    // elsewhere: not consumed
    let mut f = StashCubeInput::default();
    assert!(!f.cube_down(&S, &ptr(5, 5)).consumed);
}

#[test]
fn cube_release_and_close_latch() {
    // transmute release with pressed → 0x4F 0x18
    let mut f = StashCubeInput {
        transmute_pressed: true,
        inv_close: true,
        ..Default::default()
    };
    let e = f.cube_up(&S, &ptr(240, 330));
    assert_eq!(e.outputs, vec![msg(0x18)]);
    assert!(!f.transmute_pressed && !f.inv_close);
    // not pressed: nothing
    assert!(f.cube_up(&S, &ptr(240, 330)).outputs.is_empty());
    // cube close release with pressed: SetUIState + one 0x17
    let mut f = StashCubeInput {
        cube_close_pressed: true,
        ..Default::default()
    };
    let e = f.cube_up(&S, &ptr(370, 460));
    assert_eq!(e.outputs, vec![set_ui(0x1A), msg(0x17)]);
    // the latch lets only the first close of an open send: one 0x17
    assert_eq!(f.cube_close_path(), vec![set_ui(0x1A)]);
    // a new open clears the latch
    f.cube_opened();
    assert_eq!(f.cube_close_path(), vec![set_ui(0x1A), msg(0x17)]);
    // inventory close rectangle, no cursor item, via the hook: one 0x17
    let mut f = StashCubeInput::default();
    let e = f.cube_up(
        &S,
        &Pointer {
            in_inv_close: true,
            ..ptr(0, 0)
        },
    );
    assert_eq!(e.outputs, vec![set_ui(0x1A), msg(0x17)]);
    // with a cursor item: nothing
    let mut f = StashCubeInput::default();
    let e = f.cube_up(
        &S,
        &Pointer {
            in_inv_close: true,
            cursor_item: true,
            ..ptr(0, 0)
        },
    );
    assert!(e.outputs.is_empty());
    // another inventory mode: only the stash latch:= 0
    f.latch_ce9c = true;
    f.cube_up_other_mode();
    assert!(!f.latch_ce9c);
    // tool tips
    let (close, trans) = cube_tooltips(&S, 20, 40);
    assert_eq!(close, Point::new(80 + 289 - 10, 540 - 100));
    assert_eq!(trans, Point::new(80 + 158 - 20, 540 - 223));
}

#[test]
fn cube_gone_sends_two() {
    let mut f = StashCubeInput::default();
    assert_eq!(f.cube_gone(true), vec![set_ui(0x1A), msg(0x17), msg(0x17)]);
    // ui already closed: one message
    let mut f = StashCubeInput::default();
    assert_eq!(f.cube_gone(false), vec![msg(0x17)]);
}

#[test]
fn gold_max_text_and_font() {
    // 4051 "Gold Max: %d" with the cap; drawn in font 1 (Font16), color 0
    assert_eq!(GOLD_MAX_FONT, 1);
    assert_eq!(
        crate::ui::text::font_info(GOLD_MAX_FONT).unwrap().name,
        "Font16"
    );
    assert_eq!(super::super::stash_cube::STR_GOLD_MAX, 4051);
}

#[test]
fn horadric_grid_and_frames() {
    use super::super::stash_cube::{horadric_pos, HoradricAnim, HORADRIC_END};
    assert_eq!(horadric_pos(&S), (400, 299));
    let mut a = HoradricAnim::default();
    assert!(a.grid_visible(), "no animation: the grid is drawn");
    a.start(0);
    assert_eq!(a.frame(), Some(0));
    assert!(!a.grid_visible());
    let mut now = 0;
    for n in 1..HORADRIC_END {
        now += 71;
        a.step(now);
        assert_eq!(a.frame(), Some(n));
        assert_eq!(a.grid_visible(), n >= 14, "n = {n}");
    }
    // frame 30 is never drawn: the step to 30 stops and draws nothing
    now += 71;
    a.step(now);
    assert_eq!(a.frame(), None);
    assert!(a.grid_visible());
    // exactly 70 ms is not a step
    let mut a = HoradricAnim::default();
    a.start(1000);
    a.step(1070);
    assert_eq!(a.frame(), Some(0));
}
