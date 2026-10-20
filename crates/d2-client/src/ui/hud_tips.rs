//! The control panel tool tips (`Tip` of `ui::panels::control`) bound to
//! the string tables: the run, menu, new-stats / new-skills, experience
//! and mini-panel hovers resolve their string ids through
//! [`StringLookup`] and leave as centred text draws.
// D2RUST's own, unverified: the tip font (1, the font of the globe numbers)
// is not named by the spec. Without the font measure the globe numbers
// are centred on width 0. The run and mini-panel key names are the play
// bindings' names ([`TipIn::keys`]), not the key name strings.

use crate::ui::draw::{RectRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::geom::Point;
use crate::ui::panel::StringLookup;
use crate::ui::panels::control::buttons::{menu_tip, run_tip, tip_800, BtnEnv, NewBtn};
use crate::ui::panels::control::globes::{
    exp_tip, globe_numbers, stamina_tip, ExpIn, NumbersIn, StaminaIn, Tip,
};
use crate::ui::panels::control::minipanel::{Layout, MiniPanel, FUNCTION_BINDINGS};
use crate::ui::text::TextOpts;
use crate::ui::FRAME;

/// Tip font (D2RUST's own, unverified).
pub(crate) const TIP_FONT: u16 = 1;

/// What the tips read.
pub struct TipIn<'a> {
    pub w: i32,
    pub h: i32,
    pub mouse: (i32, i32),
    /// Resolution mode 2 (800 × 600) of the screen.
    pub res2: bool,
    pub mini_open: bool,
    /// State 9 (the new-stats / skills tip is hidden while it is open).
    pub state9_open: bool,
    pub exp: ExpIn,
    pub strings: &'a dyn StringLookup,
    /// The tip font's measure (the pop-up placement); `None`: the tips are
    /// drawn at their call point.
    pub fonts: Option<&'a super::FontMeasure>,
    /// The primary and secondary key name of a command (by command
    /// number).
    pub keys: &'a dyn Fn(i32) -> [Option<Vec<u16>>; 2],
    /// The drawn mini panel and its layout, while state 0x15 is
    /// open and a side is free.
    pub mini: Option<(Layout, &'a MiniPanel)>,
}

/// The run / walk command (cmd 0x23).
const CMD_RUN: i32 = 0x23;

/// The tips under the mouse, in the order run, menu, new stats, new
/// skills, experience, mini panel.
pub fn hud_tips(i: &TipIn<'_>) -> Vec<Tip> {
    let s = |id: u16| {
        i.strings
            .get_id(id)
            .map(<[u16]>::to_vec)
            .unwrap_or_default()
    };
    let env = BtnEnv {
        w: i.w,
        h: i.h,
        res2: i.res2,
        open_mode: 0,
    };
    [
        run_tip(i.w, i.h, i.mouse, (i.keys)(CMD_RUN), &s),
        menu_tip(i.w, i.h, i.mini_open, i.mouse, &s),
        // The tool tips are 800 × 600 only.
        i.res2
            .then(|| tip_800(&env, NewBtn::Stats, i.mouse, i.state9_open, &s))
            .flatten(),
        i.res2
            .then(|| tip_800(&env, NewBtn::Skills, i.mouse, i.state9_open, &s))
            .flatten(),
        exp_tip(&i.exp, i.w, i.h, i.mouse, &s),
        // The button's string and its binding's keys.
        i.mini.and_then(|(l, m)| {
            let keys = |f: usize| match FUNCTION_BINDINGS[f] {
                Some(cmd) => (i.keys)(i32::from(cmd)),
                None => [None, None],
            };
            m.tip(l, i.w, i.h, i.mouse, &keys, &s)
        }),
    ]
    .into_iter()
    .flatten()
    .filter(|t| !t.text.is_empty())
    .collect()
}

/// What the globe numbers and the stamina tip read.
pub struct GlobeTextIn<'a> {
    pub w: i32,
    pub h: i32,
    pub numbers: NumbersIn,
    pub stamina: StaminaIn,
    pub strings: &'a dyn StringLookup,
    /// Width A of a string in the tip font; none without the font.
    pub width_a: &'a dyn Fn(&[u16]) -> i32,
    /// As [`TipIn::fonts`].
    pub fonts: Option<&'a super::FontMeasure>,
}

/// A tip as its pop-up draw
/// ([`push_popup`]).
fn push_tip(t: Tip, w: i32, h: i32, fonts: Option<&super::FontMeasure>, out: &mut dyn UiDrawSink) {
    push_popup(
        t.text,
        Point::new(t.x, t.y),
        u16::from(t.color),
        t.centered,
        (w, h),
        fonts,
        out,
    );
}

/// The colour and mode of the pop-up's backing box
/// (`DrawRectangle(…, colour 0, mode 2)`, blend kind 2).
pub const POPUP_BOX_COLOR: u8 = 0;
pub const POPUP_BOX_MODE: u8 = 2;
/// The mode of the health bar's filled part.
pub const POPUP_BAR_MODE: u8 = 1;

/// The pop-up draw of a call `(text, at, color, centre)`
/// in the tip font: the backing box (x', b − Ht)–(x' + W, b) in colour 0, mode
/// 2, then the text centred in a block of max width + 8 whose centre is x
/// (centre 1), bottom y + 2, at the bottom − 3 for Font16. Without the font's
/// measure: the text at the call point, no box. The too-tall font swap and the
/// one slot per frame are not applied.
pub(crate) fn push_popup(
    text: Vec<u16>,
    at: Point,
    color: u16,
    centered: bool,
    screen: (i32, i32),
    fonts: Option<&super::FontMeasure>,
    out: &mut dyn UiDrawSink,
) {
    push_popup_bar(text, at, color, centered, screen, fonts, None, out);
}

/// The health-bar backing of the pop-up (with the health-bar flag
/// ≠ 0): `value` (1–128) of 128 of the box is filled with `color` (the
/// palette's nearest to RGB (0x50, 0, 0)), in draw mode 1;
/// the rest is the usual translucent black.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupBar {
    pub value: i32,
    pub color: u8,
}

/// [`push_popup`] with the health-bar backing of the monster pop-up
/// (`bar`: `None` and a value of 0 give the plain box).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_popup_bar(
    text: Vec<u16>,
    at: Point,
    color: u16,
    centered: bool,
    (w, h): (i32, i32),
    fonts: Option<&super::FontMeasure>,
    bar: Option<PopupBar>,
    out: &mut dyn UiDrawSink,
) {
    // A pop-up of the 800-wide UI box in a wider frame (the tips of the
    // side panels, which open at the frame's edges) keeps to the whole
    // frame: it is placed and clamped in frame space and moved back.
    let frame = crate::rules::camera::FrameSize::play();
    let ox = if w == crate::rules::camera::UI_BOX_WIDTH && frame.width > w {
        frame.ui_left()
    } else {
        0
    };
    if ox != 0 {
        let mut out = crate::ui::draw::ShiftSink(out, -ox);
        let at = Point::new(at.x + ox, at.y);
        let screen = (w + 2 * ox, h);
        push_popup_bar(text, at, color, centered, screen, fonts, bar, &mut out);
        return;
    }
    let style = TextStyle {
        font: TIP_FONT,
        color,
    };
    if let Some(fr) = fonts.and_then(|f| f.popup(TIP_FONT, &text, at, centered, (w, h))) {
        let (p0, p1) = fr.rect;
        let rect = |x0: i32, x1: i32, color: u8, mode: u8| {
            UiDraw::Rect(RectRequest {
                x0,
                y0: p0.y,
                x1,
                y1: p1.y,
                color,
                mode,
            })
        };
        match bar.filter(|b| b.value != 0) {
            None => out.push(rect(p0.x, p1.x, POPUP_BOX_COLOR, POPUP_BOX_MODE)),
            Some(b) => {
                // `iVar9 = value * width >> 7`; the remainder only when
                // it is not empty.
                let width = p1.x - p0.x;
                let filled = (b.value * width) >> 7;
                out.push(rect(p0.x, p0.x + filled, b.color, POPUP_BAR_MODE));
                if width - filled != 0 {
                    out.push(rect(
                        p0.x + filled,
                        p0.x + filled + (width - filled),
                        POPUP_BOX_COLOR,
                        POPUP_BOX_MODE,
                    ));
                }
            }
        }
        out.push(UiDraw::Text(TextRequest {
            text,
            at: fr.pen,
            style,
            opts: fr.opts,
            clip: crate::ui::geom::screen_clip((w, h)),
        }));
        return;
    }
    out.push(UiDraw::Text(TextRequest {
        text,
        at,
        style,
        opts: if centered {
            TextOpts::centered()
        } else {
            TextOpts::default()
        },
        clip: crate::ui::geom::screen_clip((w, h)),
    }));
}

/// Pushes the life / mana numbers and the stamina tip.
pub fn draw_globe_text(i: &GlobeTextIn<'_>, out: &mut dyn UiDrawSink) {
    draw_globe_numbers(i, out);
    draw_stamina_tip(i, out);
}

/// Pushes the life / mana numbers: plain
/// `DrawText` of UI pass step 8, before the mini panel.
pub fn draw_globe_numbers(i: &GlobeTextIn<'_>, out: &mut dyn UiDrawSink) {
    let s = |id: u16| {
        i.strings
            .get_id(id)
            .map(<[u16]>::to_vec)
            .unwrap_or_default()
    };
    let style = |color: u16| TextStyle {
        font: TIP_FONT,
        color,
    };
    for l in globe_numbers(&i.numbers, i.w, i.h, &s, i.width_a) {
        if l.text.is_empty() {
            continue;
        }
        out.push(UiDraw::Text(TextRequest {
            text: l.text,
            at: Point::new(l.x, l.y),
            style: style(l.color as u16),
            opts: TextOpts::default(),
            clip: FRAME,
        }));
    }
}

/// Pushes the stamina tip: pop-up text, drawn at UI pass step
/// 10, after the mini panel.
pub fn draw_stamina_tip(i: &GlobeTextIn<'_>, out: &mut dyn UiDrawSink) {
    let s = |id: u16| {
        i.strings
            .get_id(id)
            .map(<[u16]>::to_vec)
            .unwrap_or_default()
    };
    let st = &i.stamina;
    if let Some(t) = stamina_tip(st.shown, st.max, st.shrine, i.w, i.h, i.numbers.mouse, &s) {
        if !t.text.is_empty() {
            push_tip(t, i.w, i.h, i.fonts, out);
        }
    }
}

/// Pushes the tips as text draws.
pub fn draw_tips(i: &TipIn<'_>, out: &mut dyn UiDrawSink) {
    for t in hud_tips(i) {
        push_tip(t, i.w, i.h, i.fonts, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Strs(HashMap<u16, Vec<u16>>);
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            self.0.get(&id).map(Vec::as_slice)
        }
    }

    fn strs() -> Strs {
        Strs(
            [
                (4179, "Run"),
                (4167, "Open Mini Panel"),
                (3986, "New Stats"),
            ]
            .into_iter()
            .map(|(k, v)| (k, v.encode_utf16().collect()))
            .collect(),
        )
    }

    fn tips(mouse: (i32, i32), strings: &Strs) -> Vec<String> {
        hud_tips(&TipIn {
            w: 800,
            h: 600,
            mouse,
            res2: true,
            mini_open: false,
            state9_open: false,
            exp: ExpIn::default(),
            strings,
            fonts: None,
            keys: &|_| [None, None],
            mini: None,
        })
        .iter()
        .map(|t| String::from_utf16_lossy(&t.text))
        .collect()
    }

    fn tips_at(w: i32, h: i32, mouse: (i32, i32), strings: &Strs) -> Vec<String> {
        hud_tips(&TipIn {
            w,
            h,
            mouse,
            res2: w == 800,
            mini_open: false,
            state9_open: false,
            exp: ExpIn::default(),
            strings,
            fonts: None,
            keys: &|_| [None, None],
            mini: None,
        })
        .iter()
        .map(|t| String::from_utf16_lossy(&t.text))
        .collect()
    }

    #[test]
    fn the_new_stats_tip_is_800_only() {
        let s = strs();
        // Hover W/2 − 194 < x < W/2 − 160, H − 42 < y < H − 8 gives
        // the tip at 800 × 600 (206 < x < 240, 558 < y < 592).
        assert_eq!(tips_at(800, 600, (220, 570), &s), ["New Stats"]);
        assert!(tips_at(800, 600, (206, 570), &s).is_empty());
        // At 640 × 480 the button is drawn with a caption and has no
        // tool tip: the 800 rectangle at 640 (126 < x < 160, 438 < y < 472)
        // gives none.
        assert!(!tips_at(640, 480, (140, 450), &s).contains(&"New Stats".to_string()));
    }

    #[test]
    fn tips_resolve_their_string_ids() {
        let s = strs();
        // Run rectangle, menu rectangle, new-stats button.
        assert_eq!(tips((255, 580), &s), ["Run"]);
        assert_eq!(tips((400, 570), &s), ["Open Mini Panel"]);
        assert_eq!(tips((220, 570), &s), ["New Stats"]);
        assert!(tips((10, 10), &s).is_empty());
        let mut out: Vec<UiDraw> = Vec::new();
        draw_tips(
            &TipIn {
                w: 800,
                h: 600,
                mouse: (400, 570),
                res2: true,
                mini_open: false,
                state9_open: false,
                exp: ExpIn::default(),
                strings: &s,
                fonts: None,
                keys: &|_| [None, None],
                mini: None,
            },
            &mut out,
        );
        assert_eq!(out.len(), 1);
    }

    fn globe_strs() -> Strs {
        Strs(
            [
                (4165, "Life: %d / %d"),
                (4166, "Mana: %d / %d"),
                (4164, "Stamina: %d / %d"),
            ]
            .into_iter()
            .map(|(k, v)| (k, v.encode_utf16().collect()))
            .collect(),
        )
    }

    fn globe_text(mouse: (i32, i32), show_hp: bool) -> Vec<(String, i32, i32, u16)> {
        globe_text_in(mouse, show_hp, None)
    }

    fn globe_text_in(
        mouse: (i32, i32),
        show_hp: bool,
        fonts: Option<&crate::ui::original::FontMeasure>,
    ) -> Vec<(String, i32, i32, u16)> {
        let s = globe_strs();
        let mut out: Vec<UiDraw> = Vec::new();
        draw_globe_text(
            &GlobeTextIn {
                w: 800,
                h: 600,
                numbers: NumbersIn {
                    show_hp,
                    mouse,
                    life_shown: 50 << 8,
                    life_max: 100 << 8,
                    mana_shown: 30 << 8,
                    mana_max: 40 << 8,
                    living_player: true,
                    ..NumbersIn::default()
                },
                stamina: StaminaIn {
                    shown: 20 << 8,
                    max: 25 << 8,
                    shrine: false,
                },
                strings: &s,
                width_a: &|t| 6 * t.len() as i32,
                fonts,
            },
            &mut out,
        );
        out.into_iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some((
                    String::from_utf16_lossy(&t.text),
                    t.at.x,
                    t.at.y,
                    t.style.color,
                )),
                // The pop-up's backing box (checked by its own test).
                UiDraw::Rect(_) => None,
                _ => panic!("text and pop-up boxes only"),
            })
            .collect()
    }

    #[test]
    fn hovering_the_life_globe_draws_the_numbers() {
        // "Life: 50 / 100" is 14 units, width 84: x = 65 - 42.
        assert_eq!(
            globe_text((60, 550), false),
            [("Life: 50 / 100".to_string(), 23, 505, 0)]
        );
        // Mana: "Mana: 30 / 40" is 13 units, width 78: x = 720 - 39.
        assert_eq!(
            globe_text((700, 550), false),
            [("Mana: 30 / 40".to_string(), 681, 505, 0)]
        );
        // The toggle shows life without hover; elsewhere nothing.
        assert_eq!(globe_text((400, 300), true).len(), 1);
        assert!(globe_text((400, 300), false).is_empty());
    }

    #[test]
    fn hovering_stamina_draws_its_tip() {
        // Stamina rectangle x 273…375, y 573…591; centred at (324, 548).
        assert_eq!(
            globe_text((300, 580), false),
            [("Stamina: 20 / 25".to_string(), 324, 548, 0)]
        );
    }

    #[test]
    fn the_stamina_tip_is_a_popup_centred_on_its_point() {
        use d2_formats::font::{FontTable, Glyph};
        // Font16 stand-in: every advance 6, height byte 10.
        let glyphs = (0..256u16)
            .map(|i| Glyph {
                code: i,
                unknown1: 0,
                width: 6,
                height: 10,
                unknown2: 1,
                unknown3: 0,
                frame: i,
                unknown5: 0,
            })
            .collect();
        let mut m = crate::ui::original::FontMeasure::default();
        m.insert(
            TIP_FONT,
            FontTable {
                version: 1,
                unknown: 0,
                count: 256,
                height: 10,
                width: 0,
                glyphs,
            },
        );
        // 16 units, max width 96, W = 104: the block starts at 324 − 52;
        // bottom 548 + 2, text row 550 − 3.
        assert_eq!(
            globe_text_in((300, 580), false, Some(&m)),
            [("Stamina: 20 / 25".to_string(), 272, 547, 0)]
        );
    }

    #[test]
    fn a_popup_draws_its_backing_box_before_its_text() {
        use d2_formats::font::{FontTable, Glyph};
        // The spec vector: `Run` at (255, 577), centre 1, 800 × 600, max
        // width 26, Ht 16 → W = 34, box (238, 563)–(272, 579) in colour
        // 0, mode 2; the text block at x 238, y 576.
        let widths = |c: u16| match c {
            0x52 => 10,
            0x75 | 0x6E => 8,
            _ => 6,
        };
        let glyphs = (0..256u16)
            .map(|i| Glyph {
                code: i,
                unknown1: 0,
                width: widths(i),
                height: 16,
                unknown2: 1,
                unknown3: 0,
                frame: i,
                unknown5: 0,
            })
            .collect();
        let mut m = crate::ui::original::FontMeasure::default();
        m.insert(
            TIP_FONT,
            FontTable {
                version: 1,
                unknown: 0,
                count: 256,
                height: 10,
                width: 0,
                glyphs,
            },
        );
        let mut out: Vec<UiDraw> = Vec::new();
        push_popup(
            "Run".encode_utf16().collect(),
            Point::new(255, 577),
            0,
            true,
            (800, 600),
            Some(&m),
            &mut out,
        );
        let [UiDraw::Rect(r), UiDraw::Text(t)] = &out[..] else {
            panic!("box then text: {out:?}");
        };
        assert_eq!(
            (r.x0, r.y0, r.x1, r.y1, r.color, r.mode),
            (238, 563, 272, 579, 0, 2)
        );
        assert_eq!((t.at.x, t.at.y), (238, 576));
    }

    #[test]
    fn no_strings_draws_nothing() {
        assert!(tips((400, 570), &Strs(HashMap::new())).is_empty());
    }
}
