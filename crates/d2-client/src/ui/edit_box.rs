//! The D2Win edit box (control record E): selection fill, key handler
//! and scroll window.
//! Plain state and text arithmetic; widths come from the caller's
//! [`EditMetrics`] (width of a span of units in the box's font).

pub const BACKSPACE: u16 = 0x08;
pub const TAB: u16 = 0x09;
pub const ENTER: u16 = 0x0D;
pub const ESCAPE: u16 = 0x1B;
pub const END: u16 = 0x23;
pub const HOME: u16 = 0x24;
pub const LEFT: u16 = 0x25;
pub const UP: u16 = 0x26;
pub const RIGHT: u16 = 0x27;
pub const DOWN: u16 = 0x28;
pub const DELETE: u16 = 0x2E;
pub const F1: u16 = 0x70;

const CR: u16 = 0x0D;
const LF: u16 = 0x0A;
const UNDERSCORE: u16 = 0x5F;
const STAR: u16 = 0x2A;

/// Flag bits of E +0x260.
pub const FLAG_PASSWORD: u32 = 1;
/// With this bit clear an Enter ends the handler right after the callback.
pub const FLAG_ENTER_CONTINUES: u32 = 2;
pub const FLAG_MULTILINE: u32 = 8;

/// Measuring the box's font.
pub trait EditMetrics {
    /// Width of the units (width B/A).
    fn width(&self, units: &[u16]) -> i32;
    /// Multi-line Up/Down: the caret index after moving the
    /// caret's point one line height up (`dir` < 0) or down.
    fn move_line(&self, text: &[u16], caret: usize, dir: i32) -> usize;
}

/// What a key asks the owner to do (the callbacks of E +0x264/+0x26C, the
/// focus chain of E +0x278/+0x27C).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyOutcome {
    /// The handler's return value.
    pub handled: bool,
    /// Enter callback with the text (E +0x264).
    pub enter: Option<Vec<u16>>,
    /// Escape callback with 0.
    pub escape: bool,
    /// Tab: focus the next (`Some(false)`) or previous (`Some(true)`)
    /// control of the chain.
    pub tab: Option<bool>,
    /// The change callback E +0x26C(0) is due.
    pub changed: bool,
}

/// A selection fill rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionFill {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// Palette nearest of this RGB.
    pub rgb: (u8, u8, u8),
    /// Draw mode.
    pub mode: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditBox {
    /// E +0x5C.
    pub text: Vec<u16>,
    /// E +0x25C as a unit index.
    pub caret: usize,
    /// E +0x54 / +0x58 (−1 = none).
    pub sel_a: i32,
    pub sel_b: i32,
    /// E +0x4C / +0x50: first / last visible unit.
    pub first: i32,
    pub last: i32,
    /// E +0x14 and E +0x40.
    pub width: i32,
    pub border: i32,
    /// E +0x260.
    pub flags: u32,
    /// E +0x264 set.
    pub has_enter_cb: bool,
    /// E +0x26C set.
    pub has_change_cb: bool,
    /// E +0x08 passes a global mask (otherwise not handled).
    pub passes_masks: bool,
    /// Tab chain links E +0x278 / +0x27C are set.
    pub has_next: bool,
    pub has_prev: bool,
}

impl EditBox {
    pub fn new(width: i32, border: i32, flags: u32) -> Self {
        Self {
            text: Vec::new(),
            caret: 0,
            sel_a: -1,
            sel_b: -1,
            first: 0,
            last: -1,
            width,
            border,
            flags,
            has_enter_cb: false,
            has_change_cb: false,
            passes_masks: true,
            has_next: true,
            has_prev: true,
        }
    }

    /// The selection exists: +0x54 ≠ −1 and ≠ +0x58.
    pub fn has_selection(&self) -> bool {
        self.sel_a != -1 && self.sel_a != self.sel_b
    }

    fn clear_selection(&mut self) {
        self.sel_a = -1;
        self.sel_b = 0;
    }

    /// The selection ends ordered low / high.
    pub fn selection(&self) -> Option<(usize, usize)> {
        if !self.has_selection() || self.sel_b < 0 {
            return None;
        }
        let (a, b) = (self.sel_a as usize, self.sel_b as usize);
        Some((a.min(b), a.max(b)))
    }

    /// The fill for the part of the line `line` (units `start..end` of the
    /// text) inside the selection, drawn before the text at (`x`, `y`);
    /// `line_h` is the line height.
    pub fn selection_fill(
        &self,
        m: &dyn EditMetrics,
        start: usize,
        end: usize,
        x: i32,
        y: i32,
        line_h: i32,
    ) -> Option<SelectionFill> {
        let (lo, hi) = self.selection()?;
        let (a, b) = (lo.max(start), hi.min(end));
        if a >= b {
            return None;
        }
        Some(SelectionFill {
            x0: x + m.width(&self.text[start..a]),
            y0: y - line_h,
            x1: x + m.width(&self.text[start..b]),
            y1: y,
            rgb: (64, 64, 64),
            mode: 5,
        })
    }

    /// Deletes the selection; the caret goes to its low end.
    fn delete_selection(&mut self) {
        if let Some((lo, hi)) = self.selection() {
            let hi = hi.min(self.text.len());
            let lo = lo.min(hi);
            self.text.drain(lo..hi);
            self.caret = lo;
        }
        self.clear_selection();
    }

    /// The key handler for key `vk`; `shift` is the
    /// Shift key state. Typed characters are another handler.
    pub fn key(&mut self, vk: u16, shift: bool, m: &dyn EditMetrics) -> KeyOutcome {
        let mut out = KeyOutcome::default();
        if !self.passes_masks {
            return out;
        }
        // r5.1 pre-step
        if matches!(vk, BACKSPACE | DELETE) && self.has_selection() {
            self.delete_selection();
            return self.finish(m, out);
        }
        if (END..=DOWN).contains(&vk) {
            if !shift {
                self.clear_selection();
            } else if self.sel_a == -1 {
                self.sel_a = self.caret as i32;
                self.sel_b = self.caret as i32;
            }
        }
        let n = self.text.len();
        match vk {
            // r5.2
            BACKSPACE => {
                if self.caret > 0 {
                    self.caret -= 1;
                    self.text.remove(self.caret);
                    if self.caret > 0 && self.text[self.caret - 1] == CR {
                        self.caret -= 1;
                        self.text.remove(self.caret);
                    }
                }
            }
            DELETE => {
                if self.caret < n {
                    self.text.remove(self.caret);
                    if self.text.get(self.caret) == Some(&LF) {
                        self.text.remove(self.caret);
                    }
                }
            }
            // r5.3
            HOME => {
                self.caret = 0;
                self.extend_selection(shift, 0);
            }
            END => {
                self.caret = n;
                self.extend_selection(shift, n);
            }
            LEFT => {
                let back = if self.caret > 0 && self.text[self.caret - 1] == LF {
                    2
                } else {
                    1
                };
                self.caret = self.caret.saturating_sub(back);
                self.extend_selection(shift, self.caret);
            }
            RIGHT => {
                if self.caret < n {
                    let fwd = if self.text.get(self.caret + 1) == Some(&LF) {
                        2
                    } else {
                        1
                    };
                    self.caret = (self.caret + fwd).min(n);
                }
                self.extend_selection(shift, self.caret);
            }
            UP | DOWN => {
                if self.flags & FLAG_MULTILINE != 0 {
                    let dir = if vk == UP { -1 } else { 1 };
                    self.caret = m.move_line(&self.text, self.caret, dir).min(n);
                } else {
                    self.caret = if vk == UP { 0 } else { n };
                }
                self.extend_selection(shift, self.caret);
            }
            // r5.4
            TAB => {
                let link = if shift { self.has_prev } else { self.has_next };
                if link {
                    out.tab = Some(shift);
                }
                out.handled = true;
                return out;
            }
            // r5.5
            ENTER => {
                if self.has_enter_cb {
                    out.enter = Some(self.text.clone());
                    if self.flags & FLAG_ENTER_CONTINUES == 0 {
                        out.handled = true;
                        return out;
                    }
                }
            }
            ESCAPE => {
                out.escape = self.has_enter_cb;
                out.handled = true;
                return out;
            }
            // r5.6
            F1 => return out,
            _ => {}
        }
        self.finish(m, out)
    }

    /// With Shift and an anchor, +0x58 follows the caret.
    fn extend_selection(&mut self, shift: bool, to: usize) {
        if shift && self.sel_a != -1 {
            self.sel_b = to as i32;
        }
    }

    /// r5.7: the scroll window refit, then the change callback.
    fn finish(&mut self, m: &dyn EditMetrics, mut out: KeyOutcome) -> KeyOutcome {
        self.refit(m);
        out.changed = self.has_change_cb;
        out.handled = true;
        out
    }

    /// The scroll window.
    pub fn refit(&mut self, m: &dyn EditMetrics) {
        let n = self.text.len() as i32;
        let w = self.width - 2 * self.border;
        let stars: Vec<u16>;
        let shown: &[u16] = if self.flags & FLAG_PASSWORD != 0 {
            stars = vec![STAR; n as usize];
            &stars
        } else {
            &self.text
        };
        let wt = m.width(shown);
        let wc = m.width(&[UNDERSCORE]);
        let caret = self.caret as i32;
        let at_end = caret == n;
        if (if at_end { wt + wc } else { wt }) <= w {
            self.first = 0;
            self.last = n - 1;
            return;
        }
        let w2 = if at_end { w - wc } else { w };
        // span of units first..=last (clipped to the text)
        let span = |a: i32, b: i32| -> i32 {
            if a > b || a >= n {
                return 0;
            }
            m.width(&shown[a as usize..(b.min(n - 1) + 1) as usize])
        };
        // Walks `first` down from one before the right end
        // while the span stays under `w2`, stopping at unit 0 untested,
        // then steps back one: `first` never ends at 0 here. (`f > 0`
        // stands for the game's `f != 0`; a negative start is
        // unreachable.)
        let walk_down = |start: i32, width: &dyn Fn(i32) -> i32| -> i32 {
            let mut f = start;
            while f > 0 && width(f) < w2 {
                f -= 1;
            }
            f + 1
        };
        let walk_up = |first: i32| -> i32 {
            let mut l = first + 1;
            while l < n && span(first, l) < w2 {
                l += 1;
            }
            l - 1
        };
        if caret > self.last {
            self.last = caret;
            let last = self.last;
            self.first = walk_down(caret - 1, &|f| span(f, last));
        } else if caret <= self.first + 1 {
            self.first = (caret - 1).max(0);
            self.last = walk_up(self.first);
        } else if span(self.first, n - 1) < w2 {
            // The caret is inside the window and the text from `first`
            // to the end fits: refit from the end of the text.
            self.last = n - 1;
            self.first = walk_down(n - 2, &|f| span(f, n - 1));
        } else {
            self.last = walk_up(self.first);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every unit 5 wide.
    struct Fixed;
    impl EditMetrics for Fixed {
        fn width(&self, u: &[u16]) -> i32 {
            5 * u.len() as i32
        }
        fn move_line(&self, _: &[u16], caret: usize, dir: i32) -> usize {
            if dir < 0 {
                caret.saturating_sub(10)
            } else {
                caret + 10
            }
        }
    }

    fn eb(s: &str) -> EditBox {
        let mut e = EditBox::new(100, 3, 0);
        e.text = s.encode_utf16().collect();
        e.caret = e.text.len();
        e
    }
    fn txt(e: &EditBox) -> String {
        String::from_utf16(&e.text).unwrap()
    }

    #[test]
    fn selection_fill_per_line() {
        let mut e = eb("abcdefgh");
        e.sel_a = 6;
        e.sel_b = 2; // ends are ordered low / high
        let f = e.selection_fill(&Fixed, 0, 8, 10, 50, 16).unwrap();
        assert_eq!(
            f,
            SelectionFill {
                x0: 20,
                y0: 34,
                x1: 40,
                y1: 50,
                rgb: (64, 64, 64),
                mode: 5
            }
        );
        // a line holding units 4.. only sees the part inside the selection
        let f = e.selection_fill(&Fixed, 4, 8, 0, 20, 10).unwrap();
        assert_eq!((f.x0, f.x1), (0, 10));
        // outside the line: nothing; equal ends or −1: nothing
        assert!(e.selection_fill(&Fixed, 6, 8, 0, 0, 10).is_none());
        e.sel_b = 6;
        assert!(e.selection_fill(&Fixed, 0, 8, 0, 0, 10).is_none());
        e.sel_a = -1;
        assert!(e.selection_fill(&Fixed, 0, 8, 0, 0, 10).is_none());
    }

    #[test]
    fn key_handler() {
        let m = &Fixed;
        // Backspace removes the unit before the caret
        let mut e = eb("abc");
        assert!(e.key(BACKSPACE, false, m).handled);
        assert_eq!((txt(&e).as_str(), e.caret), ("ab", 2));
        // ... and a CR before the removed unit goes too (CR LF pair)
        let mut e = eb("a\r\n");
        e.key(BACKSPACE, false, m);
        assert_eq!((txt(&e).as_str(), e.caret), ("a", 1));
        // Delete removes the unit at the caret; an LF now there goes too
        let mut e = eb("a\r\nb");
        e.caret = 1;
        e.key(DELETE, false, m);
        assert_eq!(txt(&e), "ab");
        // Delete at the end: nothing
        let mut e = eb("ab");
        e.key(DELETE, false, m);
        assert_eq!(txt(&e), "ab");
        // Home / End / Left / Right
        let mut e = eb("abcd");
        e.key(HOME, false, m);
        assert_eq!(e.caret, 0);
        e.key(LEFT, false, m);
        assert_eq!(e.caret, 0);
        e.key(RIGHT, false, m);
        assert_eq!(e.caret, 1);
        e.key(END, false, m);
        assert_eq!(e.caret, 4);
        e.key(RIGHT, false, m);
        assert_eq!(e.caret, 4);
        // Left over a LF steps two
        let mut e = eb("a\r\nb");
        e.caret = 3;
        e.key(LEFT, false, m);
        assert_eq!(e.caret, 1);
        // Right before CR LF steps two
        e.key(RIGHT, false, m);
        assert_eq!(e.caret, 3);
        // Up / Down without the multi-line bit act as Home / End
        let mut e = eb("abcd");
        e.caret = 2;
        e.key(UP, false, m);
        assert_eq!(e.caret, 0);
        e.key(DOWN, false, m);
        assert_eq!(e.caret, 4);
        // Shift: anchor at the caret, +0x58 follows
        let mut e = eb("abcd");
        e.caret = 2;
        e.key(RIGHT, true, m);
        assert_eq!((e.sel_a, e.sel_b, e.caret), (2, 3, 3));
        e.key(HOME, true, m);
        assert_eq!((e.sel_a, e.sel_b), (2, 0));
        // no Shift clears the selection
        e.key(LEFT, false, m);
        assert_eq!((e.sel_a, e.sel_b), (-1, 0));
        // Backspace with a selection deletes it and does nothing else
        let mut e = eb("abcdef");
        e.sel_a = 1;
        e.sel_b = 4;
        e.caret = 4;
        e.key(BACKSPACE, false, m);
        assert_eq!((txt(&e).as_str(), e.caret, e.sel_a), ("aef", 1, -1));
        // Tab: next / previous; an empty link does nothing; returns 1
        let mut e = eb("a");
        let o = e.key(TAB, false, m);
        assert_eq!((o.handled, o.tab), (true, Some(false)));
        assert_eq!(e.key(TAB, true, m).tab, Some(true));
        e.has_next = false;
        let o = e.key(TAB, false, m);
        assert_eq!((o.handled, o.tab), (true, None));
        // Enter / Escape callbacks
        let mut e = eb("hi");
        e.has_enter_cb = true;
        let o = e.key(ENTER, false, m);
        assert_eq!((o.handled, o.enter), (true, Some(vec![0x68, 0x69])));
        assert!(e.key(ESCAPE, false, m).escape);
        let mut e = eb("hi");
        let o = e.key(ESCAPE, false, m);
        assert_eq!((o.handled, o.escape), (true, false));
        // bit 1 set: Enter goes on to the common ending
        let mut e = eb("hi");
        e.has_enter_cb = true;
        e.has_change_cb = true;
        e.flags = FLAG_ENTER_CONTINUES;
        let o = e.key(ENTER, false, m);
        assert!(o.enter.is_some() && o.changed);
        // F1 not handled; a key failing the masks is not handled
        assert!(!eb("a").key(F1, false, m).handled);
        let mut e = eb("a");
        e.passes_masks = false;
        assert!(!e.key(HOME, false, m).handled);
        // multi-line Up moves by the line seam
        let mut e = eb("abcdefghijklmnopqrstuvwxyz");
        e.flags = FLAG_MULTILINE;
        e.key(UP, false, m);
        assert_eq!(e.caret, 16);
    }

    #[test]
    fn scroll_window() {
        let m = &Fixed;
        // W = 100 − 6 = 94: 18 units + caret glyph = 95 does not fit at
        // the end; 18 units alone fit when the caret is inside.
        let mut e = eb("abcdefghijklmnopqr");
        e.caret = 0;
        e.refit(m);
        assert_eq!((e.first, e.last), (0, 17));
        // the caret at the end: 18 × 5 + 5 = 95 > 94 -> window moves
        e.caret = 18;
        e.refit(m);
        // caret > last? last = 17 -> caret 18 > 17: last = 18, first walks
        // down while the span < W' = 94 − 5 = 89: units f..=18 clipped to
        // the text; 17 units = 85 < 89, 18 units = 90 not
        assert_eq!((e.first, e.last), (1, 18));
        // password: widths measured on stars (same width here)
        let mut p = eb("ab");
        p.flags = FLAG_PASSWORD;
        p.refit(m);
        assert_eq!((p.first, p.last), (0, 1));
        // empty text
        let mut z = eb("");
        z.refit(m);
        assert_eq!((z.first, z.last), (0, -1));
        // the caret moves back before the window: first:= caret − 1
        let mut e = eb("abcdefghijklmnopqrst");
        e.first = 6;
        e.last = 19;
        e.caret = 3;
        e.refit(m);
        assert_eq!(e.first, 2);
        // last walks up from first + 1 while the span < W: 18 units fit
        assert_eq!(e.last, 19);
    }

    #[test]
    fn scroll_window_case1_skips_unit_0() {
        let mut e = eb("abcdefghijklmnopqrst");
        e.first = 0;
        e.last = 0;
        e.caret = 1;
        e.refit(&Fixed);
        // unit 0 would fit, but the walk stops at 0 untested, then + 1
        assert_eq!((e.first, e.last), (1, 1));
    }

    #[test]
    fn scroll_window_case3() {
        // W = 101 − 6 = 95; 20 units = 100 do not fit.
        let text: Vec<u16> = "abcdefghijklmnopqrst".encode_utf16().collect();
        let mut e = EditBox::new(101, 3, 0);
        e.text = text.clone();
        // tail(1) = 19 × 5 = 95, not strictly under 95: keep first,
        // last walks up while span(1, l) < 95 -> l = 18
        e.first = 1;
        e.last = 15;
        e.caret = 10;
        e.refit(&Fixed);
        assert_eq!((e.first, e.last), (1, 18));
        // tail(5) = 75 fits: refit from the end, first from n − 2 down
        // while tail(f) < 95 -> stops at f = 1 (95), first = 2
        let mut e = EditBox::new(101, 3, 0);
        e.text = text;
        e.first = 5;
        e.last = 15;
        e.caret = 10;
        e.refit(&Fixed);
        assert_eq!((e.first, e.last), (2, 19));
    }
}
