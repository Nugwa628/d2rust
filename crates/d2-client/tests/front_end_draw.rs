//! The front end's shared draw on synthetic data, no game files: the
//! pressed button frame, the fire overlay and the text glyph path.

use d2_client::ui::front_end::glyphs::{text_quads, GlyphQuad};
use d2_client::ui::front_end::screens::main_menu::*;
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use d2_formats::font::{FontTable, Glyph};

fn menu() -> FrontEnd {
    let mut f = FrontEnd::with_screens(true, Box::new(true));
    f.start(false);
    f
}

/// 256 records, glyph `i` has frame `i`; advance 10 except `a` (7) and `b` (12).
fn font(height: u8) -> FontTable {
    let glyphs = (0..256u16)
        .map(|i| Glyph {
            code: 255 - i,
            unknown1: 0,
            width: match i {
                0x61 => 7,
                0x62 => 12,
                _ => 10,
            },
            height,
            unknown2: 1,
            unknown3: 0,
            frame: i,
            unknown5: 0,
        })
        .collect();
    FontTable {
        version: 1,
        unknown: 0,
        count: 256,
        height,
        width: 0,
        glyphs,
    }
}

fn single_player_frames(f: &FrontEnd) -> Vec<u32> {
    f.draw()
        .iter()
        .filter_map(|d| match d {
            DrawItem::Art { file, frame, at } if *file == WIDE && at.y == 324 => Some(*frame),
            _ => None,
        })
        .collect()
}

#[test]
fn pressed_button_draws_its_pressed_frame() {
    let mut f = menu();
    assert_eq!(single_player_frames(&f), [0, 1]);
    f.input(FrontInput::Down(Point::new(300, 310)));
    f.tick();
    // 272×35: two tiles, pressed = tiles .. 2·tiles−1.
    assert_eq!(single_player_frames(&f), [2, 3]);
    // Dragged off before the release: no click.
    f.input(FrontInput::Move(Point::new(500, 100)));
    f.tick();
    f.input(FrontInput::Up(Point::new(500, 100)));
    f.tick();
    assert_eq!(single_player_frames(&f), [0, 1]);
}

#[test]
fn disabled_button_never_draws_pressed() {
    let mut f = menu();
    f.input(FrontInput::Down(Point::new(300, 350)));
    f.tick();
    let frames: Vec<u32> = f
        .draw()
        .iter()
        .filter_map(|d| match d {
            DrawItem::Art { file, frame, at } if *file == WIDE2 && at.y == 366 => Some(*frame),
            _ => None,
        })
        .collect();
    // With the `online` feature that button is ONLINE, which has an action,
    // so it draws pressed.
    let want: [u32; 2] = if cfg!(feature = "online") {
        [2, 3]
    } else {
        [0, 1]
    };
    assert_eq!(frames, want);
}

#[test]
fn fire_overlay_follows_the_logo_frame_each_tick() {
    let mut f = menu();
    let mut seen = Vec::new();
    for _ in 0..60 {
        f.tick();
        let d = f.draw();
        // The background is 12 tiles; the left logo half follows it.
        let i = d
            .iter()
            .position(|it| matches!(it, DrawItem::Art { file, .. } if *file == LOGO_LEFT))
            .expect("left logo half");
        let DrawItem::Art { frame: base, .. } = d[i] else {
            panic!()
        };
        match d[i + 1] {
            DrawItem::Blend {
                file, frame, mode, ..
            } => {
                // The left logo fire: `D2logoFireLeft`.
                assert_eq!((file, mode), (r"FrontEnd\D2logoFireLeft", 3));
                assert_eq!(frame, base, "same frame index as the base");
                seen.push(frame);
            }
            ref o => panic!("{o:?}"),
        }
    }
    // 40 ms per tick: one frame per tick, 0–28 looping.
    assert_eq!((seen[0], seen[1], seen[28], seen[29]), (1, 2, 0, 1));
}

fn text(s: &str, font: u16, label: Option<Label>) -> DrawItem {
    DrawItem::Text {
        string_id: 0,
        text: s.into(),
        font,
        at: Point::new(100, 200),
        label,
        color: 0,
        boxed: None,
    }
}

#[test]
fn text_item_yields_glyph_quads_with_the_font_advances() {
    let item = text("aba", 1, None);
    let q = text_quads(&item, &|_| Vec::new(), &|_| Some(font(10)));
    let at = |x| Point::new(x, 200);
    let g = |frame, x| GlyphQuad {
        font: 1,
        frame,
        at: at(x),
        color: 0,
    };
    assert_eq!(q, [g(0x61, 100), g(0x62, 107), g(0x61, 119)]);
}

#[test]
fn string_id_text_resolves_through_the_table_lookup() {
    let item = DrawItem::Text {
        string_id: 5106,
        text: String::new(),
        font: 1,
        at: Point::new(0, 0),
        label: None,
        color: 0,
        boxed: None,
    };
    let q = text_quads(
        &item,
        &|id| if id == 5106 { vec![0x62] } else { vec![] },
        &|_| Some(font(10)),
    );
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].frame, 0x62);
    assert!(text_quads(&item, &|_| Vec::new(), &|_| Some(font(10))).is_empty());
    assert!(text_quads(&item, &|_| vec![0x61], &|_| None).is_empty());
}

#[test]
fn button_label_is_centered_with_the_pressed_offset() {
    // 272×35 button (font 9, k = 4): the label uses the font height (the
    // header byte, 10 here), not the text height 16.
    let label = |pressed| Label {
        w: 272,
        h: 35,
        pressed,
        second: 0,
    };
    let item = |p| text("ab", 9, Some(label(p)));
    let up = text_quads(&item(false), &|_| Vec::new(), &|_| Some(font(10)));
    // width 7 + 12 = 19 → x = 100 + (272 − 19)/2; y = 200 − (35 − 10)/2 + 4.
    assert_eq!(up[0].at, Point::new(100 + (272 - 19) / 2, 200 - 12 + 4));
    assert_eq!(up[1].at.x, up[0].at.x + 7);
    // Pressed: x − 2, y + 2.
    let down = text_quads(&item(true), &|_| Vec::new(), &|_| Some(font(10)));
    assert_eq!(down[0].at, Point::new(up[0].at.x - 2, up[0].at.y + 2));
}
