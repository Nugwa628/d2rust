//! The main menu on synthetic input, no game files.

use d2_client::ui::front_end::screens::main_menu::*;
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;

fn menu(expansion: bool, saves: bool) -> FrontEnd {
    let mut f = FrontEnd::with_screens(expansion, Box::new(saves));
    f.start(false);
    assert_eq!(f.current(), MAIN_MENU);
    f
}

fn click(f: &mut FrontEnd, x: i32, y: i32) {
    let p = Point::new(x, y);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    f.tick();
}

/// (string, x, y, w, h) of each button, top to bottom (the creation order
/// of the expansion menu is not the screen order).
fn buttons(f: &FrontEnd) -> Vec<(u32, i32, i32, u16, u16, bool)> {
    let mut v: Vec<_> = f
        .controls()
        .iter()
        .filter(|c| c.kind == ControlKind::Button)
        .map(|c| (c.string_id, c.x, c.y, c.w, c.h, c.enabled))
        .collect();
    v.sort_by_key(|b| (b.2, b.1));
    v
}

#[test]
fn expansion_layout_matches_table() {
    let f = menu(true, true);
    assert_eq!(
        buttons(&f),
        [
            // The second button (ONLINE with the `online` feature, a literal
            // label), gateway, Other Multiplayer: enabled (the game's
            // screenshot draws them enabled); the last two have no action.
            (5106, 264, 324, 272, 35, true),
            (
                if cfg!(feature = "online") { 0 } else { 5107 },
                264,
                366,
                272,
                35,
                true
            ),
            (11049, 264, 391, 272, 25, true),
            (5108, 264, 433, 272, 35, true),
            // D2RUST's Display Options button (a wide button, literal label).
            (0, 264, OPTIONS_Y, 272, 35, true),
            (5110, 332, 528, 135, 25, true),
            (5109, 264, 568, 272, 35, true),
        ]
    );
}

#[test]
fn classic_layout_is_100_higher_for_first_four() {
    let f = menu(false, true);
    let ys: Vec<i32> = buttons(&f).iter().map(|b| b.2).collect();
    assert_eq!(ys, [224, 266, 291, 333, OPTIONS_Y, 528, 568]);
    let first = &f.draw()[0];
    assert!(matches!(first, DrawItem::Art { file, .. } if *file == BACKGROUND_CLASSIC));
}

/// A text control's first-row baseline, as the box layout places it.
fn baseline(c: &Control) -> i32 {
    let height = i32::from(d2_client::ui::text::font_info(c.font).unwrap().height);
    c.y - i32::from(c.h) + height + c.margin.1
}

#[test]
fn version_text_and_draw_order() {
    let f = menu(true, true);
    let d = f.draw();
    // The background: 12 tiles of 256 × 256 (4 × 3), frames 0–11.
    for (t, it) in d[..12].iter().enumerate() {
        assert!(
            matches!(it, DrawItem::Art { file, frame, .. } if *file == BACKGROUND && *frame == t as u32),
            "{it:?}"
        );
    }
    assert!(matches!(&d[12], DrawItem::Art { file, .. } if *file == LOGO_LEFT));
    // Per logo half, the black base and then its fire overlay.
    assert!(matches!(&d[13], DrawItem::Blend { mode: 3, .. }));
    assert!(matches!(&d[14], DrawItem::Art { file, .. } if *file == LOGO_RIGHT));
    assert!(matches!(&d[15], DrawItem::Blend { mode: 3, .. }));
    // The last two draws are the version lines: the D2RUST build version,
    // then the game data version below it.
    let n = d.len();
    match &d[n - 2] {
        DrawItem::Text { text, .. } => assert_eq!(text, &version_text()),
        other => panic!("{other:?}"),
    }
    match &d[n - 1] {
        DrawItem::Text { text, .. } => assert_eq!(text, GAME_DATA_VERSION),
        other => panic!("{other:?}"),
    }
}

/// The two version lines: the D2RUST build version in the old version
/// line's box and font, the game data version in the smaller font, on a
/// lower baseline, both inside the 800 x 600 menu and clear of the buttons.
#[test]
fn version_lines_show_both_versions() {
    let f = menu(true, true);
    let texts: Vec<&Control> = f
        .controls()
        .iter()
        .filter(|c| c.kind == ControlKind::Text)
        .collect();
    assert_eq!(texts.len(), 2);
    let top = texts
        .iter()
        .find(|c| c.text.as_deref() == Some(version_text().as_str()))
        .expect("the D2RUST version line");
    let small = texts
        .iter()
        .find(|c| c.text.as_deref() == Some(GAME_DATA_VERSION))
        .expect("the game data version line");
    assert_eq!(
        (top.x, top.y, top.w, top.h),
        (VERSION_X, VERSION_Y, VERSION_W, VERSION_H)
    );
    assert_eq!(top.font, VERSION_FONT);
    assert_eq!(small.font, GAME_DATA_VERSION_FONT);
    // The smaller font's glyph cels are shorter than Font16's.
    let font = |id: u16| d2_client::ui::text::font_info(id).unwrap();
    assert!(font(small.font).frame_h < font(top.font).frame_h);
    // The game data line sits below the D2RUST line, and both baselines
    // are inside the frame.
    assert!(baseline(small) > baseline(top));
    assert!(baseline(top) > 0 && baseline(top) < 600);
    assert!(baseline(small) > 0 && baseline(small) < 600);
    // Both boxes are on the left, away from the buttons (x >= 264).
    for c in [top, small] {
        assert!(c.x + i32::from(c.w) <= 264);
        assert!(c.y <= 599);
    }
}

#[test]
fn clicks_route_and_disabled_buttons_ignore() {
    // No action: gateway, Other Multiplayer, and the second button without
    // the `online` feature (with it, ONLINE opens the online login).
    for y in [350, 380, 420] {
        let mut f = menu(true, true);
        click(&mut f, 300, y);
        let want = if y == 350 && cfg!(feature = "online") {
            d2_client::ui::front_end::screens::online::ONLINE_LOGIN
        } else {
            MAIN_MENU
        };
        assert_eq!(f.current(), want, "y {y}");
    }
    let mut f = menu(true, true);
    click(&mut f, 300, 310);
    assert_eq!(f.current(), CHAR_SELECT);
    let mut f = menu(true, false);
    click(&mut f, 300, 310);
    assert_eq!(f.current(), CHAR_CREATE);
    let mut f = menu(true, true);
    click(&mut f, 400, 515);
    assert_eq!(f.current(), CREDITS);
    let mut f = menu(true, true);
    click(&mut f, 500, 515);
    assert_eq!(f.current(), MAIN_MENU, "the freed row is not a button");
    let mut f = menu(true, true);
    click(&mut f, 300, OPTIONS_Y - 10);
    assert_eq!(f.current(), OPTIONS);
    let mut f = menu(true, true);
    click(&mut f, 300, 550);
    assert_eq!(f.outcome(), Some(Outcome::Exit));
    let mut f = menu(true, true);
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.outcome(), Some(Outcome::Exit));
}

#[test]
fn button_art_states() {
    let f = menu(true, true);
    let c = &f.controls()[6];
    assert_eq!(
        (button_frame(c, 0, false), button_frame(c, 1, false)),
        (0, 1)
    );
    assert_eq!((button_frame(c, 0, true), button_frame(c, 1, true)), (2, 3));
    // A disabled button never shows its pressed frames.
    let mut dis = f.controls()[4].clone();
    dis.enabled = false;
    assert_eq!(button_frame(&dis, 0, true), 0);
}

#[test]
fn logo_fire_frames_loop_0_to_28() {
    let mut f = menu(true, true);
    let mut seen = Vec::new();
    for _ in 0..60 {
        f.tick();
        // d[12]: the left logo half, after the 12 background tiles.
        if let DrawItem::Art { frame, .. } = f.draw()[12] {
            seen.push(frame);
        }
    }
    // Built at tick 0; frame = tick mod 29, never 29.
    assert_eq!(seen[0], 1);
    assert_eq!(seen[27], 28);
    assert_eq!(seen[28], 0);
    assert!(seen.iter().all(|&x| x < 29));
}

/// Display Options looks like the main menu's other wide buttons (the same
/// art, size and label colour) and sits centred in the gap between Other
/// Multiplayer and Credits, overlapping neither.
#[test]
fn display_options_is_a_wide_button_centred_in_its_gap() {
    let f = menu(true, true);
    let by_string = |s: u32| {
        f.controls()
            .iter()
            .find(|c| c.kind == ControlKind::Button && c.string_id == s)
            .cloned()
            .unwrap()
    };
    let opts = f
        .controls()
        .iter()
        .find(|c| c.text.as_deref() == Some(OPTIONS_LABEL))
        .cloned()
        .expect("the Display Options button");
    let sp = by_string(STR_SINGLE_PLAYER);
    assert_eq!(opts.art, sp.art);
    assert_eq!((opts.x, opts.w, opts.h), (sp.x, sp.w, sp.h));
    assert_eq!(opts.color, sp.color);
    let om = by_string(STR_OTHER_MULTIPLAYER);
    let credits = by_string(STR_CREDITS);
    let top = opts.y - i32::from(opts.h);
    let credits_top = credits.y - i32::from(credits.h);
    assert!(om.y < top && opts.y < credits_top);
    // 70 px of gap around it: 18 above, 17 below.
    assert!(
        ((top - om.y) - (credits_top - opts.y)).abs() <= 1,
        "even gaps"
    );
    assert_eq!(OPTIONS_LABEL, "DISPLAY OPTIONS");
}
