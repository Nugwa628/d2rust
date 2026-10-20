//! The screen message list, the chat line formats and the recipe scroll text
//! (0x26 type 7).

use std::collections::VecDeque;

use super::{LineDraw, Metrics, RectDraw, FONT_CHAT, FONT_FORMAL10};
use crate::ui::panels::PanelOutput;

/// A record lives 10 s on the wall clock.
pub const LIFETIME_MS: u32 = 10_000;
/// Lines kept per record.
pub const MAX_RECORD_LINES: usize = 6;
/// Total lines above which the oldest record goes.
pub const MAX_TOTAL_LINES: usize = 18;
/// Message log capacity.
pub const LOG_MAX: usize = 128;
/// UI sound 6 `cursor_switch`.
pub const ADD_SOUND: i32 = 6;
/// Line step.
pub const LINE_STEP: i32 = 15;

/// A screen message record: up to 6 wrapped lines, a color and
/// the expiry tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenMessage {
    pub lines: Vec<Vec<u16>>,
    pub color: u32,
    pub expiry: u32,
}

/// What a new message asks of the rest of the UI besides the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddOutcome {
    /// The 8-bit text handed to the text filter: color 4 and
    /// exactly one line.
    pub filter: Option<Vec<u8>>,
    /// UI sound requested with no unit.
    pub sound: i32,
    /// The message log (state 0x18) is open with its object: its method
    /// refresh hook runs.
    pub refresh_log: bool,
}

/// The wide → 8-bit conversion of the filter text: UTF-8
/// per u16 unit (1 byte < 0x80, 2 < 0x800, else 3; surrogates are plain 3-byte
/// units), while fewer than 255 bytes are written (a unit that starts at byte
/// 253 or 254 may run past 255).
pub fn to_8bit(line: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    for &u in line {
        if out.len() >= 255 {
            break;
        }
        if u < 0x80 {
            out.push(u as u8);
        } else if u < 0x800 {
            out.push(0xC0 | (u >> 6) as u8);
            out.push(0x80 | (u & 0x3F) as u8);
        } else {
            out.push(0xE0 | (u >> 12) as u8);
            out.push(0x80 | ((u >> 6) & 0x3F) as u8);
            out.push(0x80 | (u & 0x3F) as u8);
        }
    }
    out
}

/// The screen message list (head = oldest) and the message
/// log (newest first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenMessages {
    list: VecDeque<ScreenMessage>,
    log: VecDeque<ScreenMessage>,
}

impl ScreenMessages {
    pub fn new() -> Self {
        Self::default()
    }

    /// The records, oldest first.
    pub fn records(&self) -> &VecDeque<ScreenMessage> {
        &self.list
    }

    /// The message log, newest first.
    pub fn log(&self) -> &VecDeque<ScreenMessage> {
        &self.log
    }

    /// Total lines held by the list.
    pub fn total_lines(&self) -> usize {
        self.list.iter().map(|r| r.lines.len()).sum()
    }

    /// Adds a message: wrapped, kept and aged. `w` is the frame width W; `now` is `GetTickCount`;
    /// `log_open` is "state 0x18 is open and its object exists".
    pub fn add(
        &mut self,
        text: &[u16],
        color: u32,
        now: u32,
        w: i32,
        log_open: bool,
        m: &dyn Metrics,
    ) -> AddOutcome {
        // r1: wrapped to W − 70 in font 13, the first 6 lines kept, the
        // record appended at the tail.
        let mut lines = m.wrap(FONT_CHAT, text, w - 70);
        lines.truncate(MAX_RECORD_LINES);
        let rec = ScreenMessage {
            lines,
            color,
            expiry: now.wrapping_add(LIFETIME_MS),
        };
        // r2: a copy to the log (newest first, at most 128).
        if self.log.len() >= LOG_MAX {
            self.log.pop_back();
        }
        self.log.push_front(rec.clone());
        let filter = (color == 4 && rec.lines.len() == 1).then(|| to_8bit(&rec.lines[0]));
        self.list.push_back(rec);
        // r3: more than 18 lines in total → the head (oldest) goes, once.
        if self.total_lines() > MAX_TOTAL_LINES {
            self.list.pop_front();
        }
        AddOutcome {
            filter,
            sound: ADD_SOUND,
            refresh_log: log_open,
        }
    }

    /// The draw: the rectangles and lines in order.
    /// `open_mode` is 0–3, `state_13_open` the state 0x13 flag. State 0x18
    /// closed is the caller's gate.
    pub fn draw(
        &self,
        w: i32,
        open_mode: u8,
        state_13_open: bool,
        m: &dyn Metrics,
    ) -> Vec<(RectDraw, LineDraw)> {
        let start = if state_13_open { 95 } else { 20 };
        let (x, rewrap) = match open_mode {
            1 => (15, true),
            2 => (w / 2 + 15, true),
            _ => (15, false),
        };
        let mut out = Vec::new();
        let mut k = 0;
        for rec in &self.list {
            for line in &rec.lines {
                let pieces: Vec<Vec<u16>> = if rewrap {
                    m.wrap(FONT_CHAT, line, 300)
                } else {
                    vec![line.clone()]
                };
                for piece in pieces {
                    let lw = m.width_c(FONT_CHAT, &piece);
                    if lw == 0 {
                        continue;
                    }
                    let y = start + LINE_STEP * k;
                    let pad = if rewrap { 5 } else { 4 };
                    out.push((
                        RectDraw {
                            x: x - pad,
                            y: y - 14,
                            w: lw + 2 * pad,
                            h: 16,
                            color: 0,
                            mode: 1,
                        },
                        LineDraw {
                            text: piece,
                            x,
                            y,
                            color: rec.color,
                        },
                    ));
                    k += 1;
                }
            }
        }
        out
    }

    /// R5, end of the draw: every record whose expiry is below `now`
    /// is unlinked and freed. Returns how many.
    pub fn expire(&mut self, now: u32) -> usize {
        let n = self.list.len();
        self.list.retain(|r| r.expiry >= now);
        n - self.list.len()
    }
}

/// The chat line strings (English): 3654 `chatmsg1`, 3658
/// `strwhisperworked`, 3994 `colorcode`, 4048 `SysmsgPlayer1`.
#[derive(Clone, Debug)]
pub struct ChatStrings {
    pub whispers: Vec<u16>,
    pub whispered: Vec<u16>,
    pub colorcode: Vec<u16>,
    pub sep: Vec<u16>,
}

impl Default for ChatStrings {
    fn default() -> Self {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        Self {
            whispers: w(" whispers: "),
            whispered: w("\u{ff}c0You whispered to \u{ff}c1%s\u{ff}c0: %s"),
            colorcode: w("\u{ff}c"),
            sep: w(": "),
        }
    }
}

/// What a 0x26 message does at the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatAction {
    /// A line added to the screen message list.
    Line {
        text: Vec<u16>,
        color: u32,
    },
    /// Type 5 with the unit present: overhead text.
    Overhead,
    /// Type 7: the recipe scroll.
    Recipe,
    Nothing,
}

/// Failure of a chat line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    /// `Strip` of a string of 500 units or more is fatal 0x37F.
    #[error("strip of {0} units (fatal 0x37F)")]
    StripTooLong(usize),
}

/// A 0x26 message as the type switch sees it (after the squelch, filter and
/// conversion steps).
#[derive(Clone, Debug)]
pub struct ChatMessage<'a> {
    pub kind: u8,
    /// u8@3.
    pub b3: u8,
    /// c8 = u8@8.
    pub c8: u8,
    /// N, the wide copy of the message name (≤ 16 units).
    pub name: &'a [u16],
    /// n, the byte length of the name.
    pub name_bytes: usize,
    /// T, the converted wide text (≤ 256 units).
    pub text: &'a [u16],
    /// t, the byte length of the text.
    pub text_bytes: usize,
    /// For type 6: the text as the format's `%s` argument.
    pub raw_text: &'a [u16],
    /// Type 5: the unit exists.
    pub unit_present: bool,
}

const COLOR_LEAD: u16 = 0xFF;
const C: u16 = b'c' as u16;

/// `Prefix(s, k)`: when `s` is not empty, `"ÿc"` + the
/// digit `'0' + k` + `s`.
pub fn prefix(s: &[u16], k: u8) -> Vec<u16> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut v = vec![COLOR_LEAD, C, u16::from(b'0' + k)];
    v.extend_from_slice(s);
    v
}

/// `Strip(s)`: removes every `ÿc` and the unit after it,
/// scanning from the start again after each removal; stops at a `ÿc`
/// that ends the string. `s` of 500 units or more is fatal 0x37F.
pub fn strip(s: &[u16]) -> Result<Vec<u16>, ChatError> {
    if s.len() >= 500 {
        return Err(ChatError::StripTooLong(s.len()));
    }
    let mut v = s.to_vec();
    loop {
        let Some(i) = v.windows(2).position(|p| p[0] == COLOR_LEAD && p[1] == C) else {
            return Ok(v);
        };
        if i + 2 >= v.len() {
            return Ok(v);
        }
        v.drain(i..i + 3);
    }
}

/// `swprintf` of the two `%s` of string 3658 into a 0x264-unit buffer.
fn format_whispered(fmt: &[u16], a: &[u16], b: &[u16]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut args = [a, b].into_iter();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] == u16::from(b'%') && fmt.get(i + 1) == Some(&u16::from(b's')) {
            if let Some(arg) = args.next() {
                out.extend_from_slice(arg);
            }
            i += 2;
        } else {
            out.push(fmt[i]);
            i += 1;
        }
    }
    out.truncate(0x263);
    out
}

/// The 0x26 type switch.
pub fn chat_action(msg: &ChatMessage<'_>, s: &ChatStrings) -> Result<ChatAction, ChatError> {
    // r1: the test is `n − 1 > 14` unsigned: n = 0 and n ≥ 16 take the
    // "else" rows.
    let name_ok = (1..=15).contains(&msg.name_bytes);
    let text_ok = msg.text_bytes <= 305;
    let line = |text: Vec<u16>, color: u32| ChatAction::Line { text, color };
    Ok(match msg.kind {
        4 => line(msg.text.to_vec(), u32::from(msg.c8)),
        1 => {
            if msg.b3 == 0 || msg.b3 == 1 {
                line(msg.text.to_vec(), u32::from(msg.c8))
            } else if name_ok {
                // Prefix(N, 4) + Prefix(": " + T, 0); r3: T is not stripped.
                let mut t = s.sep.clone();
                t.extend_from_slice(msg.text);
                let mut v = prefix(msg.name, 4);
                v.extend(prefix(&t, 0));
                line(v, u32::from(msg.c8))
            } else if text_ok {
                line(msg.text.to_vec(), u32::from(msg.c8))
            } else {
                ChatAction::Nothing
            }
        }
        2 => {
            if name_ok {
                let mut v = msg.name.to_vec();
                v.extend_from_slice(&s.whispers);
                v.extend_from_slice(msg.text);
                line(v, 2)
            } else if text_ok {
                line(msg.text.to_vec(), 2)
            } else {
                ChatAction::Nothing
            }
        }
        // r2: the format's colors and any `ÿc` the player typed are
        // stripped, so the echo is one color (2).
        6 => line(
            strip(&format_whispered(&s.whispered, msg.name, msg.raw_text))?,
            2,
        ),
        5 if msg.unit_present => ChatAction::Overhead,
        7 => ChatAction::Recipe,
        _ => ChatAction::Nothing,
    })
}

/// State of the recipe scroll: the text and its color.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipeScroll {
    pub text: Vec<u16>,
    pub color: u8,
}

/// UI state id of the recipe scroll.
pub const UI_RECIPE_SCROLL: u8 = 0x25;

/// The draw requests of the recipe scroll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeDraw {
    /// `menu\recipescroll` frames 0–3: (frame, x, y), light 0xFF, mode 5.
    pub cels: [(u32, i32, i32); 4],
    /// Font 4 text at (sx + 80, 130 − sy).
    pub text: LineDraw,
    pub font: u16,
}

impl RecipeScroll {
    /// The text converted with `MultiByteToWideChar(CP_ACP, MB_PRECOMPOSED, …,
    /// 256)`, `[…CEA4]`:= c8, then `SetUIState(0x25, on, 0)`. The code page is
    /// the machine's (measured): the reference PC's ACP is 65001, so the bytes
    /// decode as UTF-8 and each invalid byte gives U+FFFD; bytes below 0x80
    /// convert 1:1 under any code page.
    pub fn set(&mut self, text: &[u8], c8: u8) -> PanelOutput {
        let bytes: Vec<u8> = text
            .iter()
            .take(255)
            .copied()
            .take_while(|&b| b != 0)
            .collect();
        self.text = String::from_utf8_lossy(&bytes).encode_utf16().collect();
        self.color = c8;
        PanelOutput::SetUi {
            ui: UI_RECIPE_SCROLL,
            mode: 0,
            jump: false,
        }
    }

    /// Only in an expansion game with state 0x25
    /// open.
    pub fn draw(
        &self,
        sx: i32,
        sy: i32,
        h: i32,
        expansion_game: bool,
        state_open: bool,
    ) -> Option<RecipeDraw> {
        if !expansion_game || !state_open {
            return None;
        }
        Some(RecipeDraw {
            cels: [
                (0, sx, h + sy - 224),
                (1, sx + 256, h + sy - 224),
                (2, sx, h + sy - 48),
                (3, sx + 256, h + sy - 48),
            ],
            text: LineDraw {
                text: self.text.clone(),
                x: sx + 80,
                y: 130 - sy,
                color: u32::from(self.color),
            },
            font: FONT_FORMAL10,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_filter_text_is_utf8() {
        assert_eq!(
            super::to_8bit(&[0x41, 0xE9, 0x20AC]),
            vec![0x41, 0xC3, 0xA9, 0xE2, 0x82, 0xAC]
        );
        assert_eq!(super::to_8bit(&[b'a' as u16; 300]).len(), 255);
    }

    use super::super::testutil::{w, Fixed};
    use super::*;

    fn msg<'a>(kind: u8, name: &'a [u16], text: &'a [u16]) -> ChatMessage<'a> {
        ChatMessage {
            kind,
            b3: 0,
            c8: 0,
            name,
            name_bytes: name.len(),
            text,
            text_bytes: text.len(),
            raw_text: text,
            unit_present: false,
        }
    }

    fn line(a: ChatAction) -> (String, u32) {
        match a {
            ChatAction::Line { text, color } => (String::from_utf16(&text).unwrap(), color),
            other => panic!("{other:?}"),
        }
    }

    // Test vectors "0x26 type 2/6/1" and the type table rows.
    #[test]
    fn chat_formats() {
        let s = ChatStrings::default();
        let (bob, hi) = (w("Bob"), w("hi"));
        assert_eq!(
            line(chat_action(&msg(2, &bob, &hi), &s).unwrap()),
            ("Bob whispers: hi".into(), 2)
        );
        // Type 6: the player's own ÿc is stripped as well as the format's.
        let typed = w("\u{ff}c1hi");
        let mut m = msg(6, &bob, &typed);
        m.raw_text = &typed;
        assert_eq!(
            line(chat_action(&m, &s).unwrap()),
            ("You whispered to Bob: hi".into(), 2)
        );
        // Type 1, u8@3 = 2, named: Prefix(N, 4) + Prefix(": hi", 0).
        let mut m = msg(1, &bob, &hi);
        m.b3 = 2;
        assert_eq!(
            line(chat_action(&m, &s).unwrap()),
            ("\u{ff}c4Bob\u{ff}c0: hi".into(), 0)
        );
        // Type 1, u8@3 = 1: the text as is with c8.
        let mut m = msg(1, &[], &hi);
        m.b3 = 1;
        m.c8 = 4;
        assert_eq!(line(chat_action(&m, &s).unwrap()), ("hi".into(), 4));
        // Type 4: T with c8.
        let mut m = msg(4, &bob, &hi);
        m.c8 = 9;
        assert_eq!(line(chat_action(&m, &s).unwrap()), ("hi".into(), 9));
        // r3: types 1 and 2 do not strip a player's colors.
        let col = w("\u{ff}c1hi");
        let mut m = msg(1, &bob, &col);
        m.b3 = 2;
        assert_eq!(
            line(chat_action(&m, &s).unwrap()).0,
            "\u{ff}c4Bob\u{ff}c0: \u{ff}c1hi"
        );
        let m = msg(2, &bob, &col);
        assert_eq!(
            line(chat_action(&m, &s).unwrap()).0,
            "Bob whispers: \u{ff}c1hi"
        );
        // Type 5 with the unit, type 7, others.
        let mut m = msg(5, &bob, &hi);
        assert_eq!(chat_action(&m, &s).unwrap(), ChatAction::Nothing);
        m.unit_present = true;
        assert_eq!(chat_action(&m, &s).unwrap(), ChatAction::Overhead);
        assert_eq!(
            chat_action(&msg(7, &[], &hi), &s).unwrap(),
            ChatAction::Recipe
        );
        assert_eq!(
            chat_action(&msg(3, &[], &hi), &s).unwrap(),
            ChatAction::Nothing
        );
    }

    #[test]
    fn empty_or_long_name_takes_the_else_rows() {
        let s = ChatStrings::default();
        let long = w("ABCDEFGHIJKLMNOP");
        let hi = w("hi");
        // Test vector: empty name, text of 306 bytes → nothing.
        let mut m = msg(2, &[], &hi);
        m.text_bytes = 306;
        assert_eq!(chat_action(&m, &s).unwrap(), ChatAction::Nothing);
        // 305 bytes: the text alone, color 2.
        m.text_bytes = 305;
        assert_eq!(line(chat_action(&m, &s).unwrap()), ("hi".into(), 2));
        // n = 16 behaves like n = 0 for types 1 and 2.
        let m = msg(2, &long, &hi);
        assert_eq!(line(chat_action(&m, &s).unwrap()), ("hi".into(), 2));
        let mut m = msg(1, &long, &hi);
        m.b3 = 2;
        assert_eq!(line(chat_action(&m, &s).unwrap()), ("hi".into(), 0));
        m.text_bytes = 306;
        assert_eq!(chat_action(&m, &s).unwrap(), ChatAction::Nothing);
        // n = 15 is a name.
        let n15 = w("ABCDEFGHIJKLMNO");
        let m = msg(2, &n15, &hi);
        assert_eq!(
            line(chat_action(&m, &s).unwrap()).0,
            "ABCDEFGHIJKLMNO whispers: hi"
        );
    }

    #[test]
    fn prefix_and_strip_edges() {
        assert!(prefix(&[], 4).is_empty());
        assert_eq!(prefix(&w("x"), 4), w("\u{ff}c4x"));
        // Rescan after a removal; a ÿc at the end stays.
        assert_eq!(strip(&w("\u{ff}\u{ff}c1ca")).unwrap(), w(""));
        assert_eq!(strip(&w("ab\u{ff}c")).unwrap(), w("ab\u{ff}c"));
        assert!(strip(&vec![65; 500]).is_err());
    }

    #[test]
    fn add_wraps_appends_and_caps_at_18_lines() {
        let m = Fixed;
        let mut l = ScreenMessages::new();
        // W = 800: wrap width 730 = 104 units of 7 px.
        for i in 0..13 {
            l.add(&w(&format!("m{i}")), 0, 1000, 800, false, &m);
        }
        assert_eq!((l.records().len(), l.total_lines()), (13, 13));
        // The head is the oldest, a new record goes to the tail.
        assert_eq!(l.records()[0].lines[0], w("m0"));
        assert_eq!(l.records()[12].lines[0], w("m12"));
        assert_eq!(l.records()[0].expiry, 11_000);
        // A 14th message wrapping to 13 lines keeps 6: 19 lines > 18, so
        // the oldest record is removed (13 records, 18 lines).
        let long = vec![65u16; 104 * 13];
        l.add(&long, 0, 2000, 800, false, &m);
        assert_eq!(l.records().len(), 13);
        assert_eq!(l.total_lines(), 18);
        assert_eq!(l.records()[0].lines[0], w("m1"));
        assert_eq!(l.records().back().unwrap().lines.len(), 6);
    }

    #[test]
    fn add_log_filter_sound_and_refresh() {
        let m = Fixed;
        let mut l = ScreenMessages::new();
        let o = l.add(&w("hi"), 4, 0, 800, true, &m);
        assert_eq!(o.filter, Some(b"hi".to_vec()));
        assert_eq!((o.sound, o.refresh_log), (6, true));
        // Color 4 with two lines: no filter. Other colors: none.
        let o = l.add(&vec![65u16; 200], 4, 0, 800, false, &m);
        assert_eq!((o.filter, o.refresh_log), (None, false));
        assert_eq!(l.add(&w("x"), 1, 0, 800, false, &m).filter, None);
        // The log is newest first and capped at 128 (the oldest freed).
        let mut l = ScreenMessages::new();
        for i in 0..130 {
            l.add(&w(&format!("n{i}")), 0, 0, 800, false, &m);
        }
        assert_eq!(l.log().len(), 128);
        assert_eq!(l.log()[0].lines[0], w("n129"));
        assert_eq!(l.log()[127].lines[0], w("n2"));
    }

    #[test]
    fn draw_positions_by_open_mode() {
        let m = Fixed;
        let mut l = ScreenMessages::new();
        l.add(&w("hi"), 3, 0, 800, false, &m);
        l.add(&w("yo"), 5, 0, 800, false, &m);
        // Mode 0: x = 15, y = 20 + 15 k, backing (x − 4, y − 14, w + 8, 16).
        let d = l.draw(800, 0, false, &m);
        assert_eq!(d.len(), 2);
        assert_eq!(
            d[0].0,
            RectDraw {
                x: 11,
                y: 6,
                w: 22,
                h: 16,
                color: 0,
                mode: 1
            }
        );
        assert_eq!((d[0].1.x, d[0].1.y, d[0].1.color), (15, 20, 3));
        assert_eq!((d[1].1.x, d[1].1.y, d[1].1.color), (15, 35, 5));
        // State 0x13 open: start y 95.
        assert_eq!(l.draw(800, 0, true, &m)[0].1.y, 95);
        // Mode 2 at 800 × 600: x = 415, re-wrapped, box (x − 5, …, w + 10).
        let d = l.draw(800, 2, false, &m);
        assert_eq!(d[0].1.x, 415);
        assert_eq!((d[0].0.x, d[0].0.w), (410, 24));
        // Mode 1: x = 15 re-wrapped; mode 3: x = 15, not re-wrapped.
        assert_eq!(l.draw(800, 1, false, &m)[0].0.x, 10);
        assert_eq!(l.draw(800, 3, false, &m)[0].0.x, 11);
        // A re-wrapped line (300 px = 42 units) shows as pieces with k
        // counting each piece.
        let mut l = ScreenMessages::new();
        l.add(&[65u16; 90], 0, 0, 800, false, &m);
        let d = l.draw(800, 2, false, &m);
        assert_eq!(d.len(), 3);
        assert_eq!(d[2].1.y, 20 + 30);
        // A line of width 0 is skipped and k stays.
        let mut l = ScreenMessages::new();
        l.add(&[], 0, 0, 800, false, &m);
        l.add(&w("a"), 0, 0, 800, false, &m);
        let d = l.draw(800, 0, false, &m);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].1.y, 20);
    }

    #[test]
    fn expiry_frees_records_below_now() {
        let m = Fixed;
        let mut l = ScreenMessages::new();
        l.add(&w("a"), 0, 0, 800, false, &m);
        l.add(&w("b"), 0, 5000, 800, false, &m);
        assert_eq!(l.expire(10_000), 0);
        assert_eq!(l.expire(10_001), 1);
        assert_eq!(l.records().len(), 1);
        assert_eq!(l.expire(15_001), 1);
        assert!(l.records().is_empty());
        // The log keeps its copy.
        assert_eq!(l.log().len(), 2);
    }

    // The reference PC's ANSI code page is 65001.
    #[test]
    fn recipe_scroll_bytes_above_0x7f_decode_as_utf8() {
        let mut r = RecipeScroll::default();
        r.set(b"caf\xc3\xa9", 0);
        assert_eq!(r.text, [0x63, 0x61, 0x66, 0xE9]);
        r.set(b"caf\xe9", 0);
        assert_eq!(r.text, [0x63, 0x61, 0x66, 0xFFFD]);
    }

    #[test]
    fn recipe_scroll_text_and_draw() {
        let mut r = RecipeScroll::default();
        let out = r.set(b"Cube: 3 Rune", 4);
        assert_eq!(
            out,
            PanelOutput::SetUi {
                ui: 0x25,
                mode: 0,
                jump: false
            }
        );
        assert_eq!((r.text.len(), r.color), (12, 4));
        // The text is capped at 255 units plus the terminator.
        r.set(&[b'a'; 400], 0);
        assert_eq!(r.text.len(), 255);
        // Drawn only in an expansion game with state 0x25 open.
        assert!(r.draw(80, 60, 600, false, true).is_none());
        assert!(r.draw(80, 60, 600, true, false).is_none());
        let d = r.draw(80, 60, 600, true, true).unwrap();
        assert_eq!(
            d.cels,
            [(0, 80, 436), (1, 336, 436), (2, 80, 612), (3, 336, 612)]
        );
        assert_eq!((d.text.x, d.text.y, d.font), (160, 70, 4));
    }
}
