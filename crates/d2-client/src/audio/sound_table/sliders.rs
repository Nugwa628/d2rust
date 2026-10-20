//! How the three audio sliders of the in-game options menu turn into the
//! sound settings. Pure integer helpers: the menu layout and drawing are
//! the UI's; this module
//! owns the position / value mapping, the enabled tests, the input rules
//! and which sound each input requests.

use crate::audio::triggers::ui::id::{CURSOR_PASS, CURSOR_SELECT};

/// Positions of an audio slider (n = 21, positions 0–20).
pub const SLIDER_N: u32 = 21;

/// The audio sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSlider {
    /// `Sound` (Master Volume).
    Sound,
    /// `Music` (Music Volume).
    Music,
    /// `3DBias` (Positional Bias).
    Bias,
}

impl AudioSlider {
    /// The enabled test (`+0x110`): `Sound` and `Music` while the sound
    /// device is up; `3DBias` only when, in addition, the
    /// mixer mode is 1 or 2. A disabled entry ignores every input.
    pub fn enabled(self, device_up: bool, mixer_mode: u8) -> bool {
        match self {
            AudioSlider::Sound | AudioSlider::Music => device_up,
            AudioSlider::Bias => device_up && matches!(mixer_mode, 1 | 2),
        }
    }
}

/// Init: p := trunc((n − 1) × (v − 0 + 1) / (100 − 0)) = ⌊(v + 1) /
/// 5⌋ for the stored value v 0–100.
pub fn position_of(v: i32) -> u32 {
    let p = (SLIDER_N - 1) as i32 * (v + 1) / 100;
    p.max(0) as u32
}

/// Apply: v := trunc(0 + (100 − 0) / (n − 1) × p) = 5 × p.
pub fn value_of(p: u32) -> i32 {
    5 * p as i32
}

/// Left arrow: p − 1, clamped at 0 (the unsigned test against n −
/// 1 makes the clamp exact). Returns the new position.
pub fn left(p: u32) -> u32 {
    p.saturating_sub(1)
}

/// Right arrow: p + 1, clamped at n − 1.
pub fn right(p: u32) -> u32 {
    if p >= SLIDER_N - 1 {
        SLIDER_N - 1
    } else {
        p + 1
    }
}

/// The layout flag (+0x53C ≠ 0 shifts x0 and the drag window).
fn x0(width: i32, other_layout: bool) -> i32 {
    let h = width / 2;
    if other_layout {
        h - 48
    } else {
        h - 133
    }
}

/// A drag starts only with h − 144 < x < h + 145 (h − 59 … h + 230
/// for the other layout).
pub fn drag_starts(x: i32, width: i32, other_layout: bool) -> bool {
    let h = width / 2;
    let (lo, hi) = if other_layout {
        (h - 59, h + 230)
    } else {
        (h - 144, h + 145)
    };
    lo < x && x < hi
}

/// Drag: W = screen width, h = W / 2, x0 = h − 133 (h − 48 for the
/// other layout); x < x0 → 0; x > x0 + 265 → n − 1; else p =
/// trunc(trunc((x − x0) / f + 1.0) / 2) with f = 6.625.
pub fn drag_position(x: i32, width: i32, other_layout: bool) -> u32 {
    let x0 = x0(width, other_layout);
    if x < x0 {
        0
    } else if x > x0 + 265 {
        SLIDER_N - 1
    } else {
        let f = 265.0_f32 / (SLIDER_N - 1) as f32 * 0.5;
        let q = ((x - x0) as f32 / f + 1.0) as i32;
        (q / 2) as u32
    }
}

/// Outcome of an input: the new position, whether the apply runs
/// and the sound requested (id; no unit, delay 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub p: u32,
    pub apply: bool,
    pub sound: Option<i32>,
}

/// An arrow or drag moved p from `before` to `after`: changed →
/// apply and 1 `cursor_pass`; unchanged → no apply, no sound.
pub fn after_move(before: u32, after: u32) -> Outcome {
    if before != after {
        Outcome {
            p: after,
            apply: true,
            sound: Some(CURSOR_PASS),
        }
    } else {
        Outcome {
            p: before,
            apply: false,
            sound: None,
        }
    }
}

/// Up / down arrow (next / previous enabled entry) request 1.
pub const ARROW_UP_DOWN_SOUND: i32 = CURSOR_PASS;

/// Entry kinds (+0x00).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Action,
    Choice,
    Slider,
}

/// Enter, or a click released on the selected entry. Kind 1:
/// p + 1 wrapping to 0 past n − 1, apply, request 1; kind 0: apply,
/// request 2 `cursor_select`; kind 2: nothing.
pub fn activate(kind: EntryKind, p: u32, n: u32) -> Outcome {
    match kind {
        EntryKind::Choice => Outcome {
            p: if p + 1 >= n { 0 } else { p + 1 },
            apply: true,
            sound: Some(CURSOR_PASS),
        },
        EntryKind::Action => Outcome {
            p,
            apply: true,
            sound: Some(CURSOR_SELECT),
        },
        EntryKind::Slider => Outcome {
            p,
            apply: false,
            sound: None,
        },
    }
}
