//! The UI consumers of the S→C messages whose model half is
//! the client message model (chat lines, overhead text, NPC dialog,
//! hire popup, item-socket dialog, NPC intros). Plain Rust state and
//! pure functions: the Bevy layer and [`super::msg_ui`] call these; no
//! game logic lives in a Bevy system. Text measuring is a [`Metrics`]
//! the caller supplies (the draw sink owns the fonts).
//!
//! | Module | Rules |
//! |---|---|
//! | [`chat`] | screen message list, chat formats, recipe scroll |
//! | [`overhead`] | overhead text, timed text box |
//! | [`npc_text`] | NPC text list and the talk topic box |
//! | [`dialog`] | dialog panel |
//! | [`hire`] | hire popup, Inifuss scroll |
//! | [`socket`] | item-socket dialog |
//! | [`intro`] | NPC intro table, interact NPC |

pub mod chat;
pub mod dialog;
pub mod hire;
pub mod intro;
pub mod npc_facts;
pub mod npc_text;
pub mod overhead;
pub mod socket;

use super::panel::ClientIntent;

/// A rectangle as the draw / `IntersectRect` see it: left, top,
/// right, bottom, right and bottom exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ltrb {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Ltrb {
    pub const fn new(l: i32, t: i32, r: i32, b: i32) -> Self {
        Self { l, t, r, b }
    }

    /// `DrawRectangle(x, y, x + w, y + h)` from `(x, y, w, h)`.
    pub const fn xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self::new(x, y, x + w, y + h)
    }

    /// Win32 `IntersectRect` returns non-zero: the intersection is not
    /// empty (touching edges do not intersect).
    pub fn intersects(&self, o: &Ltrb) -> bool {
        self.l.max(o.l) < self.r.min(o.r) && self.t.max(o.t) < self.b.min(o.b)
    }
}

/// `DrawRectangle(x, y, w, h, color, mode)` request: color 0
/// with mode 1 is the dark translucent text backing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectDraw {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub color: u8,
    pub mode: u8,
}

/// A `DrawText(text, x, y, color, 0)` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineDraw {
    pub text: Vec<u16>,
    pub x: i32,
    pub y: i32,
    pub color: u32,
}

/// Text measuring in one font, supplied by the
/// caller. Widths in pixels.
pub trait Metrics {
    /// `Wrap(text, max)`: the lines in order.
    fn wrap(&self, font: u16, text: &[u16], max: i32) -> Vec<Vec<u16>>;
    /// Width A.
    fn width_a(&self, font: u16, text: &[u16]) -> i32;
    /// Width C.
    fn width_c(&self, font: u16, text: &[u16]) -> i32;
    /// Font height.
    fn font_height(&self, font: u16) -> i32;
}

/// Font 13, `FontInGameChat`.
pub const FONT_CHAT: u16 = 13;
/// Font 8, `FontFormal11`.
pub const FONT_FORMAL11: u16 = 8;
/// Font 4, `FontFormal10`.
pub const FONT_FORMAL10: u16 = 4;
/// Font 1, `Font16`.
pub const FONT_16: u16 = 1;
/// Font 5.
pub const FONT_5: u16 = 5;

/// One row of the entry-point table: the S→C message, and the
/// sections of this spec whose consumers it feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryPoint {
    pub message: u8,
    pub sections: &'static [u8],
}

/// The entry points (the client message model is the other half).
pub const ENTRY_POINTS: [EntryPoint; 7] = [
    EntryPoint {
        message: 0x26,
        sections: &[2, 3, 4, 5],
    },
    EntryPoint {
        message: 0x27,
        sections: &[5, 6, 7, 8],
    },
    // 0x4E and 0x4F share.
    EntryPoint {
        message: 0x4E,
        sections: &[9],
    },
    EntryPoint {
        message: 0x50,
        sections: &[9, 10],
    },
    EntryPoint {
        message: 0x58,
        sections: &[11],
    },
    EntryPoint {
        message: 0x8A,
        sections: &[12],
    },
    EntryPoint {
        message: 0x91,
        sections: &[13],
    },
];

/// The sections a message's UI entry runs. 0x4F routes like 0x4E;
/// 0x89 is the lighting's and 0x78 (trade) is out of
/// scope: `None`.
pub fn entry_sections(message: u8) -> Option<&'static [u8]> {
    let m = if message == 0x4F { 0x4E } else { message };
    ENTRY_POINTS
        .iter()
        .find(|e| e.message == m)
        .map(|e| e.sections)
}

/// Overlay 72 is `overlay.txt` row 72, `npcalert` (`patch_d2`;
/// `npc hail` in the base files), file `NPCSpeechBalloon`, 16 frames.
pub const NPC_ALERT_OVERLAY: u32 = 72;
/// The overlay's file (`NPCSpeechBalloon`).
pub const NPC_ALERT_FILE: &str = "NPCSpeechBalloon";
pub const NPC_ALERT_FRAMES: u32 = 16;
/// `wussie_cheer_1`, `wussie_help_me`, `guard_halt` (sound ids).
pub const NPC_ALERT_SOUNDS: [i32; 3] = [4603, 4607, 3983];

/// `atol` on a byte string (C runtime): optional white space, a sign,
/// decimal digits, wrapping to 32 bits.
pub fn atol(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | 9..=13) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut v: i32 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add(i32::from(s[i] - b'0'));
        i += 1;
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

/// A C→S message of `len` bytes: `id`, then `u32`s from byte 1.
pub fn msg_u32s(id: u8, words: &[u32]) -> ClientIntent {
    let mut b = vec![id];
    for w in words {
        b.extend_from_slice(&w.to_le_bytes());
    }
    ClientIntent(b)
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::Metrics;

    /// Width 7 per code unit in every font; height 16; a wrap that
    /// breaks at white space.
    pub struct Fixed;

    impl Metrics for Fixed {
        fn wrap(&self, _font: u16, text: &[u16], max: i32) -> Vec<Vec<u16>> {
            let per = (max / 7).max(1) as usize;
            if text.len() <= per {
                return vec![text.to_vec()];
            }
            text.chunks(per).map(|c| c.to_vec()).collect()
        }
        fn width_a(&self, _font: u16, text: &[u16]) -> i32 {
            7 * text.len() as i32
        }
        fn width_c(&self, _font: u16, text: &[u16]) -> i32 {
            7 * text.len() as i32
        }
        fn font_height(&self, _font: u16) -> i32 {
            16
        }
    }

    pub fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_point_table() {
        assert_eq!(entry_sections(0x26), Some(&[2u8, 3, 4, 5][..]));
        assert_eq!(entry_sections(0x27), Some(&[5u8, 6, 7, 8][..]));
        assert_eq!(entry_sections(0x4E), Some(&[9u8][..]));
        assert_eq!(entry_sections(0x4F), Some(&[9u8][..]));
        assert_eq!(entry_sections(0x50), Some(&[9u8, 10][..]));
        assert_eq!(entry_sections(0x58), Some(&[11u8][..]));
        assert_eq!(entry_sections(0x8A), Some(&[12u8][..]));
        assert_eq!(entry_sections(0x91), Some(&[13u8][..]));
        // 0x89 (lighting) and 0x78 (trade) are not consumers here.
        assert_eq!(entry_sections(0x89), None);
        assert_eq!(entry_sections(0x78), None);
    }

    #[test]
    fn npc_alert_constants() {
        assert_eq!(NPC_ALERT_OVERLAY, 72);
        assert_eq!(NPC_ALERT_FILE, "NPCSpeechBalloon");
        assert_eq!(NPC_ALERT_FRAMES, 16);
        assert_eq!(NPC_ALERT_SOUNDS, [4603, 4607, 3983]);
    }

    #[test]
    fn atol_like_c() {
        assert_eq!(atol(b"3983"), 3983);
        assert_eq!(atol(b"  -12x"), -12);
        assert_eq!(atol(b"x"), 0);
        assert_eq!(atol(b""), 0);
    }

    #[test]
    fn intersect_rect_touching_edges() {
        let a = Ltrb::new(0, 0, 10, 10);
        assert!(!a.intersects(&Ltrb::new(10, 0, 20, 10)));
        assert!(a.intersects(&Ltrb::new(9, 9, 20, 20)));
    }
}
