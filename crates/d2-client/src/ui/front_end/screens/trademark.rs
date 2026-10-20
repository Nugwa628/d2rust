//! The trademark screen: image (descriptor 2), 9 s timer
//! (3), logo halves (6, 7) and the legal text (4). Click, a key of the C2
//! list, or the timer → main menu.

use crate::ui::front_end::control::{Action, Control, ControlKind};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::screens::main_menu::{LOGO_LEFT, LOGO_RIGHT};
use crate::ui::front_end::{Registry, TRADEMARK};

/// Timer seconds (descriptor 3).
pub const TIMER_SECONDS: u32 = 9;

/// Keys whose key-down goes to the main menu (C2 r1): Backspace, Tab,
/// Enter, Space, Page Up/Down, End, Home, arrows, Delete, H, N, R, F1, Esc.
pub const KEYS: [u16; 18] = [
    8,
    9,
    13,
    32,
    33,
    34,
    35,
    36,
    37,
    38,
    39,
    40,
    46,
    b'H' as u16,
    b'N' as u16,
    b'R' as u16,
    112,
    27,
];

pub const BACKGROUND: &str = "FrontEnd\\trademarkscreenEXP";
pub const BACKGROUND_CLASSIC: &str = "FrontEnd\\trademark";

/// Text: descriptor 4, FontFormal12, (100, 580) 600×80, centred. The
/// builder adds the copyright string (expansion 21882, classic
/// 5104; its year "2001" / "2000" is replaced by "2000 - 2016") and string
/// 5105 "All Rights Reserved." (both rows
/// drawn, glyph for glyph).
const TEXT_FONT: u16 = 5;
/// Text colour `k` of both rows: 4 (the one that matches the shipped
/// screen; colours 0-12 tried).
pub const TEXT_COLOR: i32 = 4;
pub const STR_COPYRIGHT_EXP: u32 = 21882;
pub const STR_COPYRIGHT: u32 = 5104;
pub const STR_RESERVED: u32 = 5105;

struct Trademark;

impl Screen for Trademark {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        let go = Action::Trigger(Trigger::Continue);
        let art = if ctx.expansion {
            BACKGROUND
        } else {
            BACKGROUND_CLASSIC
        };
        let (id, year) = if ctx.expansion {
            (STR_COPYRIGHT_EXP, "2001")
        } else {
            (STR_COPYRIGHT, "2000")
        };
        let mut text = Control::new(ControlKind::Text, 100, 580, 600, 80)
            .with_font(TEXT_FONT, 2)
            .with_string(id)
            .with_text(
                format!("{year}{}2000 - 2016", crate::ui::front_end::glyphs::SUBST),
                TEXT_COLOR,
            );
        text.more_rows.push(crate::ui::front_end::control::TextRow {
            string_id: STR_RESERVED,
            text: String::new(),
            color: TEXT_COLOR,
        });
        vec![
            Control::new(ControlKind::Image, 0, 599, 800, 600)
                .with_art(art)
                .with_keys(&KEYS)
                .with_action(go),
            Control::timer(TIMER_SECONDS, go),
            // Logo halves, descriptors 6 / 7.
            Control::new(ControlKind::AnimImage, 400, 120, 181, 170).with_art(LOGO_LEFT),
            Control::new(ControlKind::AnimImage, 400, 120, 188, 177).with_art(LOGO_RIGHT),
            text,
        ]
    }
}

pub fn register(reg: &mut Registry) {
    reg.register(TRADEMARK, Box::new(Trademark));
}
