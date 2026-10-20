//! The control-descriptor model every front-end screen is built from.
//!
//! A [`Control`] is one control descriptor: type, position, size,
//! hotkey, string id, art, action. **y is the bottom edge**: a hit box is `x ≤
//! mx < x+w`, `y−h ≤ my < y`.

use crate::ui::geom::{Point, Rect};

use super::flow::Trigger;

/// Descriptor field [0], the control types the front end uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// 1 edit box.
    EditBox,
    /// 2 image.
    Image,
    /// 3 animated image (the logo halves).
    AnimImage,
    /// 4 text.
    Text,
    /// 6 button.
    Button,
    /// 8 timer (invisible).
    Timer,
}

/// What a control does when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing (a disabled or out-of-scope control).
    None,
    /// A screen-flow trigger, resolved by [`super::flow::next`].
    Trigger(Trigger),
    /// A screen-private action: handed to [`super::Screen::action`].
    Custom(u32),
}

/// One control descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub kind: ControlKind,
    /// Frame x of the left edge.
    pub x: i32,
    /// Frame y of the **bottom** edge.
    pub y: i32,
    pub w: u16,
    pub h: u16,
    /// Hotkey virtual key (27 Esc, 13 Enter, 0 none), descriptor [5].
    pub hotkey: u16,
    /// Further virtual keys that fire the action (the key handler of the
    /// trademark image).
    pub keys: &'static [u16],
    /// String id of the label, descriptor [6]; 0 none.
    pub string_id: u32,
    /// Fixed text (the version line) when there is no string id.
    pub text: Option<String>,
    /// Font of a text control, descriptor [10].
    pub font: u16,
    /// Text control flags, descriptor [11] (2 centred, 0x10 right aligned, 0x20
    /// no wrap, 1 sets 0x20).
    pub flags: u16,
    /// Text control margins mx = [5], my = [6].
    pub margin: (i32, i32),
    /// Colour `k` of the text control's row (r8 a).
    pub color: i32,
    /// Further rows added to a text control after the first (r8 a), each
    /// with its own colour.
    pub more_rows: Vec<TextRow>,
    /// A button's second label string (flag 0x40, `[+0x270]`); 0 none.
    pub second_label: u32,
    /// The cel file, relative to `data\global\ui\`; `None`: no art.
    pub art: Option<&'static str>,
    pub action: Action,
    /// Timer: whole seconds before it fires, descriptor [1].
    pub seconds: u32,
    pub enabled: bool,
    /// Takes no click or key but is drawn as it is (the controls under a pop-up layer:
    /// the game keeps their enabled look).
    pub inert: bool,
    /// An image drawn through draw mode `m` (alpha blend tables) instead of opaque.
    pub blend: Option<u8>,
    /// An image that is one cel: frame `n` of the art placed at (x, y), y one below the last row (the Art convention).
    pub cel: Option<u32>,
    pub visible: bool,
    /// A check box (art `clickbox`) that is on: draws frame 1.
    pub checked: bool,
    /// Drawn after the screen's overlay (the create fire is created last
    /// but drawn after the heroes).
    pub late: bool,
    /// Drawn after the screen's overlay and the late controls: a pop-up
    /// (the pop-up list is above the screen's list).
    pub top: bool,
    /// A toggle / radio button, descriptor [11] & 3 ≠ 0: its
    /// action runs on the press, the release does nothing.
    pub on_press: bool,
}

impl Control {
    pub fn new(kind: ControlKind, x: i32, y: i32, w: u16, h: u16) -> Self {
        Self {
            kind,
            x,
            y,
            w,
            h,
            hotkey: 0,
            keys: &[],
            string_id: 0,
            text: None,
            font: 0,
            flags: 0,
            margin: (0, 0),
            color: 0,
            more_rows: Vec::new(),
            second_label: 0,
            art: None,
            action: Action::None,
            seconds: 0,
            enabled: true,
            inert: false,
            blend: None,
            cel: None,
            visible: true,
            checked: false,
            late: false,
            top: false,
            on_press: false,
        }
    }

    /// The pointer hit box in frame pixels: top edge `y − h`.
    pub fn hit_box(&self) -> Rect {
        Rect::new(self.x, self.y - i32::from(self.h), self.w, self.h)
    }

    pub fn contains(&self, p: Point) -> bool {
        let r = self.hit_box();
        i64::from(p.x) >= i64::from(r.x)
            && i64::from(p.x) < r.right()
            && i64::from(p.y) >= i64::from(r.y)
            && i64::from(p.y) < r.bottom()
    }

    /// Whether the virtual key `vk` fires this control.
    pub fn takes_key(&self, vk: u16) -> bool {
        vk != 0 && (self.hotkey == vk || self.keys.contains(&vk))
    }

    pub fn with_action(mut self, a: Action) -> Self {
        self.action = a;
        self
    }

    pub fn with_art(mut self, art: &'static str) -> Self {
        self.art = Some(art);
        self
    }

    /// Fires on the press (descriptor [11] & 3 ≠ 0).
    pub fn with_on_press(mut self) -> Self {
        self.on_press = true;
        self
    }

    pub fn with_hotkey(mut self, vk: u16) -> Self {
        self.hotkey = vk;
        self
    }

    pub fn with_keys(mut self, keys: &'static [u16]) -> Self {
        self.keys = keys;
        self
    }

    pub fn with_string(mut self, id: u32) -> Self {
        self.string_id = id;
        self
    }

    /// A text control's font and flags.
    pub fn with_font(mut self, font: u16, flags: u16) -> Self {
        self.font = font;
        self.flags = flags;
        self
    }

    /// The text of a text control's first row, colour `k` (r8 a).
    pub fn with_text(mut self, text: impl Into<String>, k: i32) -> Self {
        self.text = Some(text.into());
        self.color = k;
        self
    }

    /// A text control's colour (string-id controls; r8 a).
    pub fn with_color(mut self, k: i32) -> Self {
        self.color = k;
        self
    }

    /// Whether a text control holds a row to draw: a string id or a text (a
    /// control nothing was added to draws nothing).
    pub fn has_text(&self) -> bool {
        self.string_id != 0 || self.text.as_deref().is_some_and(|t| !t.is_empty())
    }

    /// A timer descriptor (type 8): fires `action` after `seconds`.
    pub fn timer(seconds: u32, action: Action) -> Self {
        let mut c = Self::new(ControlKind::Timer, 0, 0, 0, 0);
        c.seconds = seconds;
        c.action = action;
        c.visible = false;
        c
    }

    /// An invisible key-only control (hotkey fires `action`).
    pub fn key_only(vk: u16, action: Action) -> Self {
        let mut c = Self::new(ControlKind::Button, 0, 0, 0, 0)
            .with_hotkey(vk)
            .with_action(action);
        c.visible = false;
        c
    }
}

/// One row added to a text control (r8 a): a string id (0: `text`) and its
/// colour `k`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRow {
    pub string_id: u32,
    pub text: String,
    pub color: i32,
}

/// Virtual-key codes the front end names.
pub mod vk {
    pub const BACKSPACE: u16 = 8;
    pub const TAB: u16 = 9;
    pub const ENTER: u16 = 13;
    pub const ESC: u16 = 27;
    pub const SPACE: u16 = 32;
}

/// Frames per state of a button cel cut in 256×256 tiles:
/// `ceil(w/256)·ceil(h/256)`. Up = `0 … tiles−1`, pressed = `tiles … 2·tiles−1`.
pub fn button_tiles(w: u16, h: u16) -> u32 {
    u32::from(w).div_ceil(256) * u32::from(h).div_ceil(256)
}

/// Label font: 9 (FontExocet10) if `h ≥ 35`, else 10 (FontRidiculous).
pub fn label_font(h: u16) -> u16 {
    if h >= 35 {
        9
    } else {
        10
    }
}

/// The baseline constant k of the label (the draw): 4 (h ≥ 35), 3 (h = 32
/// exactly), else 2 (h > 20, so 33 and 34 too) or 1 (h ≤ 20).
pub fn label_k(h: u16) -> i32 {
    match h {
        35.. => 4,
        32 => 3,
        21.. => 2,
        _ => 1,
    }
}

/// The frame index a pressed or resting button draws: `Some(frame)` for tile
/// `tile`; a disabled button draws from frame `tiles·2` if `flag_0x20` (3-state
/// art), else the up frames.
pub fn button_frame(tile: u32, tiles: u32, pressed: bool, enabled: bool, flag_0x20: bool) -> u32 {
    if !enabled && flag_0x20 {
        tiles * 2 + tile
    } else if pressed && enabled {
        tiles + tile
    } else {
        tile
    }
}
