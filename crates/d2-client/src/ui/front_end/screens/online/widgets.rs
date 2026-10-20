//! The pieces every online screen is built from, in the menus' own art:
//! text fields drawn in the `textbox` cel with the create screen's caret,
//! the message box (`PopUpOk2`, one OK) that shows an error's text, and the
//! wait box (`PopUpOk2` with CANCEL) shown while a request is out.

use crate::ui::front_end::control::vk;
use crate::ui::front_end::{Action, Control, ControlKind, DIM_LAYER};

/// Font8, Font16, Font30, Font24, and the name box's font (the create
/// screen's typed name).
pub const FONT8: u16 = 0;
pub const FONT16: u16 = 1;
pub const FONT30: u16 = 2;
pub const FONT24: u16 = 7;
pub const FONT_FIELD: u16 = 5;

/// Text colours (`k`): white, red, green, gold, grey.
pub const WHITE: i32 = 0;
pub const RED: i32 = 1;
pub const GREEN: i32 = 2;
pub const GOLD: i32 = 4;
pub const GREY: i32 = 5;

pub const BG_MENU: &str = r"FrontEnd\gameselectscreenEXP";
pub const BG_MENU_CLASSIC: &str = r"FrontEnd\TitleScreen";
pub const TEXTBOX: &str = r"FrontEnd\textbox";
pub const MEDIUM: &str = r"FrontEnd\MediumButtonBlank";
pub const WIDE: &str = r"FrontEnd\WideButtonBlank";
pub const SHORT: &str = r"CharSelect\ShortButtonBlank";
/// A pop-up's box and button art, by the palette the screen shows: the
/// menus' sky palette (character select's `PopUpOk` and
/// `CancelButtonBlank`) or the create screen's (`PopUpOk2`,
/// `OkCancelButtonBlank`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupArt {
    pub frame: &'static str,
    pub button: &'static str,
}

pub const SKY_POPUP: PopupArt = PopupArt {
    frame: r"FrontEnd\PopUpOk",
    button: r"FrontEnd\CancelButtonBlank",
};
pub const CREATE_POPUP: PopupArt = PopupArt {
    frame: r"FrontEnd\PopUpOk2",
    button: r"FrontEnd\OkCancelButtonBlank",
};
pub const RADIO: &str = r"FrontEnd\radiobutton";
pub const SCROLL: &str = r"FrontEnd\joingamescrollbars";

/// The `textbox` cel's size, and where the typed text sits in it: the
/// create screen's name box puts the pen 7 px in and 5 px above the cel
/// bottom; in a text control's box (FontFormal12, 20 px high) that is
/// margins 7 / 1.
pub const FIELD_W: u16 = 169;
pub const FIELD_H: u16 = 26;
const FIELD_MARGIN: (i32, i32) = (7, 1);

/// One line of typed text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    /// Most characters it takes.
    pub max: usize,
    /// Drawn as `*`s.
    pub password: bool,
}

impl Field {
    pub fn new(max: usize) -> Self {
        Field {
            text: String::new(),
            max,
            password: false,
        }
    }

    pub fn password(max: usize) -> Self {
        Field {
            password: true,
            ..Field::new(max)
        }
    }

    /// Adds a printable character if there is room; `ok` filters what the
    /// field takes.
    pub fn type_char(&mut self, c: char, ok: impl Fn(char) -> bool) -> bool {
        if c.is_control() || self.text.chars().count() >= self.max || !ok(c) {
            return false;
        }
        self.text.push(c);
        true
    }

    pub fn backspace(&mut self) {
        self.text.pop();
    }

    /// What the box shows.
    pub fn shown(&self) -> String {
        if self.password {
            "*".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }
}

/// The controls of a field at (x, cel bottom y): the box art, a click
/// area that fires `focus`, and last the text control [`show_field`] fills.
pub fn field_controls(x: i32, y: i32, focus: u32) -> Vec<Control> {
    vec![
        Control::new(ControlKind::Image, x, y, FIELD_W, FIELD_H).with_art(TEXTBOX),
        Control::new(ControlKind::Image, x, y, FIELD_W, FIELD_H).with_action(Action::Custom(focus)),
        field_text(x, y),
    ]
}

/// A field's text control without the box (a box the background draws).
pub fn field_text(x: i32, y: i32) -> Control {
    let mut c = Control::new(ControlKind::Text, x, y, FIELD_W, FIELD_H)
        .with_font(FONT_FIELD, 0x20)
        .with_text(String::new(), WHITE);
    c.margin = FIELD_MARGIN;
    c
}

/// The text a field shows now: its text, and the caret when focused (a
/// trailing `_` while `now / 250` is odd, the edit box's blink). `keep`:
/// the most characters shown (the end of a longer text).
pub fn show_field(c: &mut Control, f: &Field, focused: bool, now_ms: u64, keep: usize) {
    let shown = f.shown();
    let n = shown.chars().count();
    let tail: String = shown.chars().skip(n.saturating_sub(keep)).collect();
    let caret = focused && (now_ms / 250) % 2 == 1;
    c.text = Some(format!("{tail}{}", if caret { "_" } else { "" }));
}

/// A label: literal text in Font16 at (x, bottom y), one row.
pub fn label(x: i32, y: i32, w: u16, text: impl Into<String>, color: i32) -> Control {
    Control::new(ControlKind::Text, x, y, w, 20)
        .with_font(FONT16, 0x20)
        .with_text(text, color)
}

/// Centred text in a box.
pub fn centred(x: i32, y: i32, w: u16, h: u16, font: u16, text: impl Into<String>) -> Control {
    Control::new(ControlKind::Text, x, y, w, h)
        .with_font(font, 2)
        .with_text(text, WHITE)
}

/// A button with a literal label (D2RUST's own buttons write their
/// labels in capitals, as the stock ones are).
pub fn button(x: i32, y: i32, w: u16, h: u16, art: &'static str, text: &str, id: u32) -> Control {
    Control::new(ControlKind::Button, x, y, w, h)
        .with_art(art)
        .with_text(text.to_uppercase(), 0)
        .with_action(Action::Custom(id))
}

/// The pop-up layer: the dimming of the whole frame, then the box art as
/// its two cels, last row 349 (the create screen's message pop-up).
pub fn push_popup_art(v: &mut Vec<Control>, art: PopupArt) {
    v.push(Control::new(ControlKind::Image, 0, 600, 800, 600).with_art(DIM_LAYER));
    for (frame, x) in [(0, 268), (1, 524)] {
        let mut c = Control::new(ControlKind::Image, x, 350, 264, 176).with_art(art.frame);
        c.cel = Some(frame);
        v.push(c);
    }
}

/// The pop-up's text: (268, 320) 264 × 120, margins 10 / 8, centred and
/// wrapped, Font16.
pub fn popup_text(text: impl Into<String>) -> Control {
    let mut c = Control::new(ControlKind::Text, 268, 320, 264, 120)
        .with_font(FONT16, 2)
        .with_text(text, WHITE);
    c.margin = (10, 8);
    c
}

/// A message box: `text` and one button (`OK` on Enter and Esc) that fires
/// `close`.
pub fn push_message(v: &mut Vec<Control>, art: PopupArt, text: &str, close: u32) {
    push_popup_art(v, art);
    v.push(popup_text(text));
    v.push(
        button(351, 337, 96, 32, art.button, "OK", close)
            .with_hotkey(vk::ENTER)
            .with_keys(&[vk::ESC])
            .with_on_press(),
    );
}

/// The wait box while a request is out: `text` and CANCEL (Esc).
pub fn push_wait(v: &mut Vec<Control>, art: PopupArt, text: &str, cancel: u32) {
    push_popup_art(v, art);
    v.push(popup_text(text));
    v.push(
        button(351, 337, 96, 32, art.button, "CANCEL", cancel)
            .with_hotkey(vk::ESC)
            .with_on_press(),
    );
}

/// Everything before the pop-up layer goes dead (drawn as it is): the
/// pop-up owns the input, and it draws above the screen's own items (the
/// chat lines, the paper dolls).
pub fn modal(v: &mut [Control]) {
    if let Some(start) = v.iter().position(|c| c.art == Some(DIM_LAYER)) {
        for c in &mut v[..start] {
            c.inert = true;
        }
        for c in &mut v[start..] {
            c.top = true;
        }
    }
}
