use super::*;

fn layout(expansion: bool) -> GoldLayout {
    GoldLayout {
        sx: 10,
        sy: -5,
        w: 800,
        h: 600,
        expansion,
    }
}

#[test]
fn gold_line_positions() {
    let e = layout(true);
    let c = layout(false);
    assert_eq!(GoldLine::Stash.stat(), 15);
    assert_eq!(GoldLine::Inventory.stat(), 14);
    assert_eq!(GoldLine::Shop.stat(), 15);
    assert_eq!(
        GoldLine::Stash.value_pen(&e, 0),
        Point::new(175, 600 - 5 - 440)
    );
    assert_eq!(
        GoldLine::Stash.value_pen(&c, 0),
        Point::new(175, 600 - 5 - 244)
    );
    assert_eq!(
        GoldLine::Inventory.value_pen(&e, 0),
        Point::new(800 - 10 - 212, 600 - 5 - 72)
    );
    // shop: right-aligned by width A
    assert_eq!(
        GoldLine::Shop.value_pen(&e, 33),
        Point::new(10 + 198 - 33, 600 - 5 - 106)
    );
    assert_eq!(shop_label_pos(&e), Point::new(31, 600 - 5 - 106));
    // buttons: pressed draws one row lower; the shop has none
    assert_eq!(GoldLine::Stash.button(&e, false), Some(Point::new(85, 155)));
    assert_eq!(
        GoldLine::Stash.button(&c, true),
        Some(Point::new(85, 600 - 5 - 244 + 1))
    );
    assert_eq!(
        GoldLine::Inventory.button(&e, true),
        Some(Point::new(800 - 10 - 236, 600 - 5 - 71 + 1))
    );
    assert_eq!(GoldLine::Shop.button(&e, false), None);
    assert_eq!(gold_text(1234), "1234");
    assert_eq!((gold_button_frame(false), gold_button_frame(true)), (0, 1));
    // stash hover: x in [sx + 73, sx + 93], y in [Y − 18, Y] (inclusive)
    let y = 600 - 5 - 438;
    assert_eq!(
        stash_hover(&e, Point::new(83, y)),
        Some((4124, Point::new(63, 600 - 5 - 458)))
    );
    assert!(stash_hover(&e, Point::new(103, y)).is_some());
    assert!(stash_hover(&e, Point::new(104, y)).is_none());
    assert!(stash_hover(&e, Point::new(83, y - 18)).is_some());
    assert!(stash_hover(&e, Point::new(83, y - 19)).is_none());
    assert!(stash_hover(&e, Point::new(83, y + 1)).is_none());
    let yc = 600 - 5 - 242;
    assert_eq!(
        stash_hover(&c, Point::new(83, yc)),
        Some((4124, Point::new(63, 600 - 5 - 262)))
    );
    // inventory gold rectangle, inclusive, none with a cursor item
    let l = &e;
    let (x0, y0) = (800 - 10 - 237, 600 - 5 - 87);
    assert!(inventory_gold_hit(l, Point::new(x0, y0), false));
    assert!(inventory_gold_hit(l, Point::new(x0 + 20, y0 + 18), false));
    assert!(!inventory_gold_hit(l, Point::new(x0 + 21, y0), false));
    assert!(!inventory_gold_hit(l, Point::new(x0, y0 + 19), false));
    assert!(!inventory_gold_hit(l, Point::new(x0, y0), true));
    assert!(stash_gold_hit(l, Point::new(83, y), false));
    assert!(!stash_gold_hit(l, Point::new(83, y), true));
}

#[test]
fn press_and_release() {
    let mut b = GoldButtons::default();
    assert_eq!(
        b.press_inventory(false),
        GoldPress {
            consumed: false,
            sound: None
        }
    );
    assert!(!b.inv_pressed);
    assert_eq!(
        b.press_inventory(true),
        GoldPress {
            consumed: true,
            sound: Some(4)
        }
    );
    assert!(b.inv_pressed);
    // release in the rectangle with the dialog flag clear: kind 1
    assert_eq!(b.release_inventory(true), Some(GoldKind::Drop));
    assert!(!b.inv_pressed);
    // no pressed flag: nothing
    assert_eq!(b.release_inventory(true), None);
    // a dialog already open: no new one, the flag is still cleared
    b.inv_pressed = true;
    b.dialog_flag = true;
    assert_eq!(b.release_inventory(true), None);
    assert!(!b.inv_pressed);
    // release outside: cleared, no dialog
    b.dialog_flag = false;
    b.inv_pressed = true;
    assert_eq!(b.release_inventory(false), None);
    // stash modes: deposit first, then withdraw; no pressed check
    let mut b = GoldButtons::default();
    assert_eq!(b.release_stash(true, false, false), vec![GoldKind::Deposit]);
    assert_eq!(
        b.release_stash(false, true, false),
        vec![GoldKind::Withdraw]
    );
    assert_eq!(
        b.release_stash(true, true, false),
        vec![GoldKind::Deposit, GoldKind::Withdraw]
    );
    b.dialog_flag = true;
    assert!(b.release_stash(true, true, false).is_empty());
    // the pressed flags clear unless the mouse is over the belt
    b.inv_pressed = true;
    b.stash_pressed = true;
    b.release_stash(false, false, true);
    assert!(b.inv_pressed && b.stash_pressed);
    b.release_stash(false, false, false);
    assert!(!b.inv_pressed && !b.stash_pressed);
}

#[test]
fn dialog_kinds_open_close_ok() {
    // openers
    assert_eq!(
        opener_kind(GoldOpener::InventoryButton {
            stash_open: false,
            cube_open: false
        }),
        GoldKind::Drop
    );
    assert_eq!(
        opener_kind(GoldOpener::InventoryButton {
            stash_open: true,
            cube_open: false
        }),
        GoldKind::Deposit
    );
    assert_eq!(
        opener_kind(GoldOpener::InventoryButton {
            stash_open: false,
            cube_open: true
        }),
        GoldKind::Drop
    );
    assert_eq!(opener_kind(GoldOpener::StashButton), GoldKind::Withdraw);
    assert_eq!(opener_kind(GoldOpener::PlayerTrade), GoldKind::Trade);
    assert_eq!(opener_kind(GoldOpener::Other), GoldKind::Plain);
    assert!(can_open(true, false, false));
    assert!(!can_open(false, false, false));
    assert!(!can_open(true, true, false));
    assert!(!can_open(true, false, true));
    // kinds ≥ 5 close at once
    assert_eq!(GoldKind::from_u8(4), Some(GoldKind::Withdraw));
    assert_eq!(GoldKind::from_u8(5), None);
    // prompts and maxima
    let prompts: Vec<u32> = (0..5)
        .map(|k| GoldKind::from_u8(k).unwrap().prompt_id())
        .collect();
    assert_eq!(prompts, [4033, 4033, 4046, 4049, 4050]);
    assert_eq!(GoldKind::Withdraw.max_stat(), 15);
    assert_eq!(GoldKind::Drop.max_stat(), 14);
    assert!(GoldKind::Deposit.prefills() && !GoldKind::Drop.prefills());
    assert!(GoldKind::Withdraw.sets_dialog_flag() && !GoldKind::Plain.sets_dialog_flag());
    // positions as passed (640 × 480)
    assert_eq!(
        (pos::SPINNER, pos::EDIT, pos::OK, pos::CANCEL),
        (
            Point::new(0xDF, 0xDB),
            Point::new(0x102, 0xE4),
            Point::new(0xFA, 0x11F),
            Point::new(0x163, 0x11F)
        )
    );
    assert_eq!((pos::EDIT_WIDTH, pos::EDIT_CAP), (100, 10));
    // open: key mode 0 only when the latch was clear; amount 0; kind 3 pre-fills
    let (d, fx) = GoldDialog::open(GoldKind::Deposit, 4500, false);
    assert_eq!(
        fx,
        OpenEffects {
            key_mode_0: true,
            set_dialog_flag: true
        }
    );
    assert_eq!(d.edit.value(), 4500);
    assert!(d.latch);
    let (d, fx) = GoldDialog::open(GoldKind::Plain, 100, true);
    assert_eq!(
        fx,
        OpenEffects {
            key_mode_0: false,
            set_dialog_flag: false
        }
    );
    assert_eq!((d.amount, d.edit.value()), (0, 0));
    // close takes the edit box's value and clears the latch
    let (mut d, _) = GoldDialog::open(GoldKind::Drop, 500, false);
    d.edit.char(b'1' as u32, false, &|_| false);
    d.edit.char(b'2' as u32, false, &|_| false);
    d.edit.char(b'3' as u32, false, &|_| false);
    assert_eq!(d.close(), 123);
    assert!(!d.latch);
    // OK: kind 1, typed 123 → 0x50 [player GUID, 123]
    assert_eq!(
        ok_action(GoldKind::Drop, 123, Some(9)),
        GoldSend::DropGold {
            guid: 9,
            amount: 123
        }
    );
    assert_eq!(
        ok_action(GoldKind::Plain, 5, None),
        GoldSend::DropGold {
            guid: u32::MAX,
            amount: 5
        }
    );
    // kind 4, 70000 → 4F 13, p1 = 1, p2 = 0x1170, sound 0xDD
    assert_eq!(
        ok_action(GoldKind::Withdraw, 70000, Some(1)),
        GoldSend::StashButton {
            button: 0x13,
            p1: 1,
            p2: 0x1170,
            sound: 0xDD
        }
    );
    assert_eq!(
        ok_action(GoldKind::Deposit, 70000, Some(1)),
        GoldSend::StashButton {
            button: 0x14,
            p1: 1,
            p2: 0x1170,
            sound: 0xDD
        }
    );
    // amount 0: nothing, except kind 2 (trade) which still calls the trade code
    assert_eq!(ok_action(GoldKind::Drop, 0, Some(1)), GoldSend::Nothing);
    assert_eq!(ok_action(GoldKind::Withdraw, 0, Some(1)), GoldSend::Nothing);
    assert_eq!(
        ok_action(GoldKind::Trade, 0, Some(1)),
        GoldSend::TradeOffer { sound: None }
    );
    assert_eq!(
        ok_action(GoldKind::Trade, 10, Some(1)),
        GoldSend::TradeOffer { sound: Some(0xDD) }
    );
    // spinner down with step 1000 at amount 300 → 0; up clamps at the max
    let (mut d, _) = GoldDialog::open(GoldKind::Drop, 1000, false);
    d.edit.set_value(300);
    d.spinner_apply(SpinDir::Down, 1000);
    assert_eq!((d.amount, d.edit.value()), (0, 0));
    d.edit.set_value(300);
    d.spinner_apply(SpinDir::Up, 900);
    assert_eq!(d.amount, 1000);
    d.spinner_apply(SpinDir::Down, 1);
    assert_eq!(d.amount, 999);
    d.spinner_apply(SpinDir::None, 5);
    assert_eq!(d.amount, 999);
}

fn wraps(text: &[u8]) -> bool {
    // 10 units wide per digit, 100 wide box: 11 digits wrap
    text.len() * 10 > 100
}

#[test]
fn digit_edit_box() {
    let mut e = DigitEdit::new(5000);
    let type_all = |e: &mut DigitEdit, s: &str| {
        for c in s.bytes() {
            e.char(u32::from(c), false, &wraps);
        }
    };
    type_all(&mut e, "12");
    assert_eq!(
        (e.text.clone(), e.caret, e.value()),
        (b"12".to_vec(), 2, 12)
    );
    // clamped to the max as typed
    type_all(&mut e, "345");
    assert_eq!(e.text, b"5000");
    assert_eq!((e.caret, e.value()), (4, 5000));
    // Backspace: removes before the caret; at caret 0 it removes the last unit
    let mut e = DigitEdit::new(u32::MAX);
    type_all(&mut e, "1234");
    e.caret = 2;
    e.char(8, false, &wraps);
    assert_eq!((e.text.clone(), e.caret), (b"134".to_vec(), 1));
    e.caret = 0;
    e.char(8, false, &wraps);
    assert_eq!((e.text.clone(), e.caret), (b"13".to_vec(), 0));
    // `.` acts as Delete at the caret, never typed
    e.caret = 0;
    assert_eq!(e.char(u32::from(b'.'), false, &wraps), CharResult::Consumed);
    assert_eq!(e.text, b"3");
    e.caret = 1;
    e.char(u32::from(b'.'), false, &wraps);
    assert_eq!(e.text, b"3");
    // insertion in the middle: the caret does not advance
    let mut e = DigitEdit::new(u32::MAX);
    type_all(&mut e, "13");
    e.caret = 1;
    type_all(&mut e, "2");
    assert_eq!((e.text.clone(), e.caret), (b"123".to_vec(), 1));
    // other characters: space, Esc, CR are left to the box; others consumed
    for c in [0x20u32, 0x1B, 0x0D] {
        assert_eq!(e.char(c, false, &wraps), CharResult::LeftToBox);
    }
    assert_eq!(e.char(u32::from(b'a'), false, &wraps), CharResult::Consumed);
    assert_eq!(e.char(0x100, false, &wraps), CharResult::Consumed);
    assert_eq!(e.text, b"123");
    // at most 9 digits
    let mut e = DigitEdit::new(u32::MAX);
    type_all(&mut e, "12345678901");
    assert_eq!(e.text, b"123456789");
    // wrapping to more lines removes the typed unit again
    let mut e = DigitEdit::new(u32::MAX);
    e.text = b"1234567890".to_vec();
    e.caret = 10;
    // length 10 ≥ cap − 1: nothing typed
    e.char(u32::from(b'5'), false, &wraps);
    assert_eq!(e.text.len(), 10);
    let narrow = |t: &[u8]| t.len() > 3;
    let mut e = DigitEdit::new(u32::MAX);
    type_all_with(&mut e, "12345", &narrow);
    assert_eq!(e.text, b"123");
    // key repeat off: a repeat is swallowed
    let mut e = DigitEdit::new(100);
    e.key_repeat = false;
    e.char(u32::from(b'1'), true, &wraps);
    assert!(e.text.is_empty());
    e.char(u32::from(b'1'), false, &wraps);
    assert_eq!(e.text, b"1");
    // value: empty 0; over the max 0; set value cuts to 10 units
    let mut e = DigitEdit::new(100);
    assert_eq!(e.value(), 0);
    e.text = b"101".to_vec();
    assert_eq!(e.value(), 0);
    e.set_value(4_000_000_000);
    assert_eq!((e.text.len(), e.caret), (10, 10));
    // arrows move the caret within [0, length]
    e.set_value(42);
    e.arrow(false);
    assert_eq!(e.caret, 1);
    e.arrow(true);
    e.arrow(true);
    assert_eq!(e.caret, 2);
    for _ in 0..4 {
        e.arrow(false);
    }
    assert_eq!(e.caret, 0);
    // blink: shown for the first 19 draws, then 10 off / 10 on
    let mut e = DigitEdit::new(1);
    let mut seq = Vec::new();
    for _ in 0..50 {
        e.blink();
        seq.push(e.visible);
    }
    assert!(seq[..19].iter().all(|&v| v));
    assert!(!seq[19]);
    assert!(seq[19..28].iter().all(|&v| !v));
    assert!(seq[29]);
    assert!(e.caret_drawn() == e.visible);
    // mouse: down sets pressed (not taken); up with pressed resets the blink
    e.mouse_down();
    assert!(e.pressed);
    e.mouse_up();
    assert_eq!((e.pressed, e.counter), (false, 0));
}

fn type_all_with(e: &mut DigitEdit, s: &str, w: &dyn Fn(&[u8]) -> bool) {
    for c in s.bytes() {
        e.char(u32::from(c), false, w);
    }
}

#[test]
fn spinner() {
    let o = pos::SPINNER;
    let (up, down) = Spinner::arrow_points(o);
    assert_eq!((up, down), (Point::new(224, 219), Point::new(224, 232)));
    let mut s = Spinner::default();
    // mouse down on the up arrow: x in (ux, ux + w), y in (uy − h, uy]
    assert!(!s.mouse_down(o, Point::new(224, 219), 0)); // x = ux exclusive
    assert!(s.held);
    let mut s = Spinner::default();
    assert!(s.mouse_down(o, Point::new(230, 219), 1000));
    assert!(s.up && !s.down);
    assert_eq!(s.direction(), SpinDir::Up);
    // first step: 1 and t1:= now; within 70 ms: 0
    assert_eq!(s.step(1000), 1);
    assert_eq!(s.step(1050), 0);
    // d ≤ 1000: 1
    assert_eq!(s.step(1100), 1);
    s.n = 64;
    assert_eq!(s.step(1000 + 1500), 64 >> 5);
    assert_eq!(s.step(1000 + 2500), 64 >> 4);
    assert_eq!(s.step(1000 + 3500), 64 >> 2);
    assert_eq!(s.step(1000 + 5000), (5000 >> 11) * 64);
    // draw: n += 1 while pressed; frames 2 × up, 2 × down + 1
    let mut s2 = Spinner {
        up: true,
        ..Default::default()
    };
    assert_eq!(s2.draw_frames(), (2, 1, true));
    assert_eq!(s2.n, 1);
    let mut s3 = Spinner {
        down: true,
        ..Default::default()
    };
    assert_eq!(s3.draw_frames(), (0, 3, true));
    assert_eq!(Spinner::default().draw_frames(), (0, 1, false));
    // down rectangle exclusive at the bottom
    let mut s = Spinner::default();
    assert!(!s.mouse_down(o, Point::new(230, 232), 0));
    assert!(s.mouse_down(o, Point::new(230, 231), 0));
    assert!(s.down);
    // mouse up clears one and returns 1
    assert!(s.mouse_up());
    assert!(!s.down && !s.held && s.n == 0);
    assert!(!s.mouse_up());
    // wheel: pulse; step clears and returns 1
    let mut s = Spinner::default();
    assert!(s.wheel(120));
    assert_eq!(s.direction(), SpinDir::Up);
    assert_eq!(s.step(5), 1);
    assert_eq!(s.direction(), SpinDir::None);
    assert!(s.wheel(-120));
    assert_eq!(s.direction(), SpinDir::Down);
    assert_eq!(Spinner::default().step(5), 0);
}

#[test]
fn dialog_buttons() {
    let mut ok = DialogButton::ok();
    let mut cancel = DialogButton::cancel();
    assert_eq!((ok.frame(), cancel.frame()), (16, 10));
    assert_eq!((ok.string_id(), cancel.string_id()), (3401, 4142));
    // rectangle x in [x, x + w], y in [y − h, y]
    assert!(ok.contains(Point::new(250, 287)));
    assert!(ok.contains(Point::new(282, 255)));
    assert!(!ok.contains(Point::new(283, 270)));
    assert!(!ok.contains(Point::new(260, 254)));
    assert_eq!(ok.caption_pos(12), Point::new(250 + 10, 287 - 32 - 5));
    // mouse down inside: pressed, sound 4; up inside: callback
    assert_eq!(ok.mouse_down(Point::new(260, 270)), Some(4));
    assert_eq!(ok.frame(), 17);
    assert!(ok.mouse_up(Point::new(260, 270)));
    assert!(!ok.pressed);
    // up outside: no callback, pressed cleared; not pressed: nothing
    ok.mouse_down(Point::new(260, 270));
    assert!(!ok.mouse_up(Point::new(0, 0)));
    assert!(!ok.pressed);
    assert!(!ok.mouse_up(Point::new(260, 270)));
    assert_eq!(cancel.mouse_down(Point::new(0, 0)), None);
    // Enter = OK: enabled, not a repeat, CR
    assert!(ok.char(0x0D, false));
    assert!(!ok.char(0x0D, true));
    assert!(!ok.char(b'a' as u32, false));
    assert!(!cancel.char(0x0D, false));
}

#[test]
fn box_input() {
    assert_eq!(pos::BOX, Point::new(215, 140));
    assert_eq!(pos::BOX_SIZE, (210, 158));
    // list order: the add order is prepended to
    assert_eq!(CONTROL_ORDER, ["cancel", "ok", "edit", "spinner"]);
    // 80 ms grace
    assert!(in_grace(1079, 1000));
    assert!(!in_grace(1080, 1000));
    // mouse down
    assert_eq!(box_mouse_down(true, true, false), BoxDown::Control);
    assert_eq!(box_mouse_down(false, false, false), BoxDown::Consumed);
    assert_eq!(box_mouse_down(false, true, false), BoxDown::Close);
    assert_eq!(box_mouse_down(false, true, true), BoxDown::PassOn);
    // the mini panel strip
    assert!(in_mini_strip(640, 480, Point::new(320, 420)));
    assert!(!in_mini_strip(640, 480, Point::new(246, 420)));
    assert!(in_mini_strip(640, 480, Point::new(247, 420)));
    assert!(!in_mini_strip(640, 480, Point::new(391, 420)));
    assert!(!in_mini_strip(640, 480, Point::new(320, 411)));
    assert!(!in_mini_strip(640, 480, Point::new(320, 430)));
    // Esc: nothing with the chat open; closes after more than 4 draws
    assert_eq!(box_esc(true, 10), (false, false));
    assert_eq!(box_esc(false, 4), (true, false));
    assert_eq!(box_esc(false, 5), (true, true));
    assert!(wheel_offered(120) && wheel_offered(-240) && !wheel_offered(119));
}
