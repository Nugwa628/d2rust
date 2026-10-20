//! The dialog panel: open, text, scroll,
//! draw, wait for speech, the end of a pass, skip input and close.

use super::{atol, msg_u32s, LineDraw, RectDraw, FONT_FORMAL11};
use crate::ui::panel::ClientIntent;

/// Panel width.
pub const PANEL_W: i32 = 325;
/// The scroll speed when the first line has a unit ≥ 0x80.
pub const SPEED_DEFAULT: i32 = 8;
/// A text line is cut at 100 units.
pub const MAX_LINE: usize = 100;
/// Quest ids that send 0x31 at close: `A1Q5InitQuestTome`,
/// `A2Q4SuccessfulNarrator`, `AncientsAct5IntroGossip1`,
/// `A5Q3FoundAnyaAnya`, `A5Q6InitAncients`.
pub const CLOSE_MSG_IDS: [u32; 5] = [127, 396, 20002, 20131, 20169];
/// A skip within 100 ms of the open is only consumed.
pub const SKIP_GRACE_MS: u32 = 100;
/// 1/1024 pixel per pixel.
const ONE_LINE: i64 = 18 * 1024;

/// The text of a panel: its lines and the scroll speed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogText {
    pub lines: Vec<Vec<u16>>,
    pub speed: i32,
}

/// Lines split at LF, at most 100 units each (a longer line is cut and the rest
/// starts the next line). The first line is the scroll speed: when all its
/// units are < 0x80, speed:= `atol` of it, else 8; it is never shown. An empty
/// text gives no lines, and a text that ends with LF has no trailing empty line
/// (the end-of-text rule). An empty text leaves the speed unset; 8 stands in,
/// and no stock text is empty.
pub fn parse_text(text: &[u16]) -> DialogText {
    let mut all: Vec<Vec<u16>> = Vec::new();
    let mut cur: Vec<u16> = Vec::new();
    for &u in text {
        if u == 0x0A {
            // A line that reached exactly 100 units
            // leaves the LF unconsumed; it ends an empty next line.
            let cut = cur.len() >= MAX_LINE;
            all.push(std::mem::take(&mut cur));
            if cut {
                all.push(Vec::new());
            }
            continue;
        }
        if cur.len() == MAX_LINE {
            all.push(std::mem::take(&mut cur));
        }
        cur.push(u);
    }
    if !cur.is_empty() {
        all.push(cur);
    }
    if all.is_empty() {
        return DialogText {
            lines: Vec::new(),
            speed: SPEED_DEFAULT,
        };
    }
    let first = all.remove(0);
    let speed = if first.iter().all(|&u| u < 0x80) {
        let b: Vec<u8> = first.iter().map(|&u| u as u8).collect();
        atol(&b)
    } else {
        SPEED_DEFAULT
    };
    DialogText { lines: all, speed }
}

/// The scroll state of a panel: position p in 1/1024 pixel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scroll {
    pub p: i64,
    pub speed: i32,
    pub t_last: u32,
    pub t_start: u32,
    pub step: u32,
    pub acc: u32,
    pub same: u16,
}

impl Scroll {
    pub fn new(speed: i32) -> Self {
        Self {
            speed,
            ..Default::default()
        }
    }

    /// Per draw with `t` = `timeGetTime`.
    pub fn update(&mut self, t: u32) {
        if self.t_start == 0 {
            self.t_last = t;
            self.t_start = t;
            return;
        }
        let e = i64::from(t.wrapping_sub(self.t_start) >> 2);
        let mut a = 0i64;
        if t == self.t_last {
            if self.step == 0 {
                self.same = self.same.wrapping_add(1);
            } else {
                self.acc = self.acc.wrapping_add(self.step);
                a = i64::from(self.acc);
            }
        } else {
            self.acc = 0;
            if self.step == 0 && self.same != 0 {
                self.step = t.wrapping_sub(self.t_last) / u32::from(self.same);
            }
            self.t_last = t;
        }
        self.p = (a + e) * i64::from(self.speed);
    }

    /// The scroll is finished when (p >> 10) > 18 · max(count − 1, 1) +
    /// 112.
    pub fn finished(&self, count: usize) -> bool {
        (self.p >> 10) > 18 * (count.max(2) as i64 - 1) + 112
    }
}

/// One draw of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogDraw {
    /// (place 0 only).
    Backing(RectDraw),
    /// (`menu\boxpieces`) around (x − 1, y − 6, x + 326, y +
    /// 117).
    Border { l: i32, t: i32, r: i32, b: i32 },
    /// A whole line, `DrawText(line, x, y, 0, 0)`.
    Line(LineDraw, usize),
    /// A partial line with the vertical window:
    /// `skip` rows skipped, `lines` rows drawn.
    Window {
        line: usize,
        x: i32,
        y: i32,
        skip: i32,
        lines: i32,
    },
}

/// The panel draw at (x, y): `place0` is the top-centre place,
/// `font_h` the font height. The caller gates it on "Text
/// Display" option or the dialog's shown flag.
pub fn panel_draw(
    x: i32,
    y: i32,
    place0: bool,
    p: i64,
    lines: &[Vec<u16>],
    font_h: i32,
) -> Vec<DialogDraw> {
    let mut out = Vec::new();
    if place0 {
        out.push(DialogDraw::Backing(RectDraw {
            x,
            y: y - 5,
            w: PANEL_W,
            h: 122,
            color: 0,
            mode: 1,
        }));
        out.push(DialogDraw::Border {
            l: x - 1,
            t: y - 6,
            r: x + 326,
            b: y + 117,
        });
    }
    let n = lines.len();
    let q = |i: usize| p - ONE_LINE * i as i64;
    let whole: Vec<usize> = (0..n).filter(|&i| (0..96256).contains(&q(i))).collect();
    let (Some(&first), Some(&last)) = (whole.first(), whole.last()) else {
        // No whole line: the last line, q = p − 18432 (count − 1), when q
        // < 112 · 1024.
        if let Some(i) = n.checked_sub(1) {
            let qq = q(i);
            if qq < 112 * 1024 {
                let v = 112 - ((qq + 1024) >> 10);
                if v > 0 {
                    out.push(DialogDraw::Window {
                        line: i,
                        x: x + 16,
                        y: y + v as i32,
                        skip: 0,
                        lines: (v as i32).min(18),
                    });
                }
            }
        }
        return out;
    };
    // The line above the first whole one: the vertical window at
    // (x + 16, y + v), v = 94 − (q_first >> 10), skip 0, min(v, 18) lines.
    if first > 0 {
        let v = 94 - (q(first) >> 10) as i32;
        out.push(DialogDraw::Window {
            line: first - 1,
            x: x + 16,
            y: y + v,
            skip: 0,
            lines: v.min(18),
        });
    }
    for &i in &whole {
        out.push(DialogDraw::Line(
            LineDraw {
                text: lines[i].clone(),
                x: x + 16,
                y: y + 112 - (q(i) >> 10) as i32,
                color: 0,
            },
            i,
        ));
    }
    // The line below the last whole one: at (x + 16, y + 130 − (q_last
    // >> 10)) with skip s = 17 − (q_last >> 10) and lines h − s, when
    // s < h.
    if last + 1 < n {
        let ql = (q(last) >> 10) as i32;
        let s = 17 - ql;
        if s < font_h {
            out.push(DialogDraw::Window {
                line: last + 1,
                x: x + 16,
                y: y + 130 - ql,
                skip: s,
                lines: font_h - s,
            });
        }
    }
    out
}

/// While the speech handle is set, its request is
/// not yet playing and the master volume is not 0, the panel is neither
/// scrolled nor drawn.
pub fn waiting_for_speech(handle_set: bool, playing: bool, master_volume_zero: bool) -> bool {
    handle_set && !playing && !master_volume_zero
}

/// The place of an open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogOpen {
    /// 0x27 type 2 / the quest log.
    Quest { id: u32, place: u32, flag: bool },
    /// NPC talk and gossip; a unit GUID or none.
    Npc { unit: Option<u32>, id: u32 },
}

/// What the dialog asks of the rest of the UI (named by the game step they
/// stand for).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogEffect {
    /// The previous speech fades.
    FadeSpeech,
    /// Input reset (where the dialog's rule asks for one).
    InputReset,
    MouseWindow {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    },
    MouseWindowOff,
    /// The NPC's skill voices detach.
    DetachSkillVoices,
    /// The dialog speech for `id` on the local player, d = 5.
    RequestSpeech {
        id: u32,
        d: u8,
    },
    RegisterHandlers,
    UnregisterHandlers,
    Send(ClientIntent),
    /// After the C→S 0x30 of a unit dialog.
    UnitDialogCleanup,
    UnitDialogEnd(u32),
    /// The timed box is closed.
    CloseTimedBox,
    Restore,
    Run4a0880,
    /// The end callback (EDX 0 at the end of the text; 1 at a skip; the stack
    /// argument `mouse` is the dialog's mouse-down flag at a skip; 0 at the end of the text).
    EndCallback {
        skip: bool,
        mouse: bool,
    },
}

/// The panel object: lines, count, scroll and `+0x16` (0 and 1 run, 2
/// ends).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub lines: Vec<Vec<u16>>,
    pub scroll: Scroll,
    pub state: u8,
}

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogUi {
    /// Panel left edge.
    pub x: i32,
    /// Panel top edge.
    pub y: i32,
    /// The panel stands at place 1 (place ≠ 0).
    pub place1: bool,
    /// The dialog's text id.
    pub id: u32,
    /// The open dialog panel.
    pub panel: Option<Panel>,
    /// The text is shown.
    pub shown_flag: bool,
    /// The dialog is up.
    pub up: bool,
    /// A skip is in progress this pass.
    pub skip: bool,
    /// The speech handle is set.
    pub speech_pending: bool,
    /// The dialog's input handlers are registered.
    pub handlers: bool,
    /// The open time (`GetTickCount`).
    pub t_open: u32,
    /// The close sends C→S 0x31.
    pub close_flag: bool,
    /// The unit this dialog belongs to, by GUID.
    pub unit_dialog: Option<u32>,
    /// The timed box flag.
    pub timed_box_flag: bool,
    /// The end callback is set.
    pub end_callback: bool,
    /// Set by the button-down handler,
    /// cleared by every skip; the end callback's stack argument at a
    /// skip.
    pub mouse_down: bool,
    /// A flag cleared when the text finishes.
    pub flag_264: bool,
}

/// The inputs of the text pass step.
#[derive(Clone, Debug, Default)]
pub struct PassInput {
    /// `timeGetTime`.
    pub now: u32,
    /// [`waiting_for_speech`] result.
    pub speech_waiting: bool,
    /// The end callback's result when it runs at the end of the text.
    pub end_callback_result: u32,
    /// A timed box was drawn this pass (adds 1 to R).
    pub timed_box_drawn: bool,
    /// A dead or absent local player.
    pub local_dead_or_absent: bool,
    /// A unit dialog whose unit is present in mode 12.
    pub unit_in_mode12: bool,
}

/// The result of one pass: R, the effects, and whether r3–r4 ran (the
/// panel scrolled and is to be drawn).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PassOutput {
    pub r: u32,
    pub effects: Vec<DialogEffect>,
    pub drawn: bool,
}

/// Events of the skip handler table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipEvent {
    /// Right button up: consumed only.
    RightUp,
    /// Left and right button down.
    ButtonDown,
    /// Keys Esc and Space (kind 3).
    Key,
    /// WM_CHAR; `ch` and the key of binding 7.
    Char { ch: u32, automap_key: u32 },
    /// WM_SYSKEYDOWN; F4 passes.
    SysKeyDown { f4: bool },
}

/// The 7 registered handlers: kind, message, handler address.
pub const SKIP_TABLE: [(u8, &str, u32); 7] = [
    (1, "right button up", 0x0049_DA10),
    (1, "left button down", 0x004A_17D0),
    (1, "right button down", 0x004A_17D0),
    (3, "Esc", 0x004A_1770),
    (3, "Space", 0x004A_1770),
    (1, "WM_CHAR", 0x004A_1870),
    (1, "WM_SYSKEYDOWN", 0x004A_18C0),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkipOutcome {
    pub consumed: bool,
    pub effects: Vec<DialogEffect>,
}

impl DialogUi {
    /// Opens the dialog. `text` is the string of the dialog id; `w`, `h`, `sx`,
    /// `sy` are the screen size and scale; `now` is `GetTickCount`.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        o: DialogOpen,
        text: &[u16],
        w: i32,
        h: i32,
        sx: i32,
        sy: i32,
        now: u32,
    ) -> Vec<DialogEffect> {
        let mut eff = Vec::new();
        let (id, place0) = match o {
            DialogOpen::Quest { id, place, .. } => (id, place == 0),
            DialogOpen::Npc { id, .. } => (id, true),
        };
        if place0 {
            self.x = (w - PANEL_W) / 2;
            self.y = 12;
            self.place1 = false;
        } else {
            self.x = sx;
            self.y = 261 - sy;
            self.place1 = true;
        }
        eff.push(DialogEffect::FadeSpeech);
        self.speech_pending = false;
        eff.push(DialogEffect::InputReset);
        eff.push(DialogEffect::MouseWindow {
            l: self.x - 10,
            t: self.y,
            r: self.x + 335,
            b: (self.y + 182).min(h - 53),
        });
        self.id = id;
        if self.panel.is_none() {
            self.skip = false;
            let t = parse_text(text);
            self.panel = Some(Panel {
                lines: t.lines,
                scroll: Scroll::new(t.speed),
                state: 0,
            });
            self.up = true;
            if matches!(o, DialogOpen::Quest { .. }) {
                self.shown_flag = true;
            }
            if !self.handlers {
                eff.push(DialogEffect::RegisterHandlers);
                self.t_open = now;
                self.handlers = true;
            }
            if matches!(o, DialogOpen::Npc { .. }) {
                eff.push(DialogEffect::DetachSkillVoices);
            }
            eff.push(DialogEffect::RequestSpeech { id, d: 5 });
            self.speech_pending = true;
        }
        if let DialogOpen::Quest { id, flag: true, .. } = o {
            if CLOSE_MSG_IDS.contains(&id) {
                self.close_flag = true;
            }
        }
        if let DialogOpen::Npc { unit, .. } = o {
            match unit {
                Some(guid) => {
                    if let Some(old) = self.unit_dialog.filter(|&g| g != guid) {
                        eff.push(DialogEffect::Send(msg_u32s(0x30, &[1, old])));
                        eff.push(DialogEffect::UnitDialogCleanup);
                    }
                    self.unit_dialog = Some(guid);
                }
                None => {
                    eff.push(DialogEffect::CloseTimedBox);
                    self.timed_box_flag = false;
                    self.unit_dialog = None;
                }
            }
        }
        eff
    }

    /// The close: mouse window off, panel freed,
    /// speech faded (4), handlers unregistered; with the close flag set,
    /// C→S 0x31 [0xFFFFFFFF u32 @1][the id, zero-extended u32 @5].
    pub fn close(&mut self) -> Vec<DialogEffect> {
        let mut eff = vec![DialogEffect::MouseWindowOff];
        self.panel = None;
        eff.push(DialogEffect::FadeSpeech);
        if self.handlers {
            self.handlers = false;
        }
        eff.push(DialogEffect::UnregisterHandlers);
        if self.close_flag {
            eff.push(DialogEffect::Send(msg_u32s(0x31, &[0xFFFF_FFFF, self.id])));
            self.close_flag = false;
        }
        eff
    }

    /// The text pass step: R = 1 while the panel runs.
    pub fn pass(&mut self, inp: &PassInput) -> PassOutput {
        let mut out = PassOutput::default();
        let mut r = 0u32;
        let running = self.up && self.panel.as_ref().is_some_and(|p| p.state <= 1);
        if running {
            if self.skip {
                // A skip request: +0x16:= 2, R = 0.
                if let Some(p) = self.panel.as_mut() {
                    p.state = 2;
                }
            } else if inp.speech_waiting {
                // r5: neither scrolled nor drawn; it stays up.
                r = 1;
            } else {
                self.speech_pending = false;
                let (finished, count) = {
                    let p = self.panel.as_mut().expect("running");
                    p.scroll.update(inp.now);
                    (p.scroll.finished(p.lines.len()), p.lines.len())
                };
                let _ = count;
                out.drawn = true;
                if !finished {
                    r = 1;
                } else {
                    if self.flag_264 {
                        self.flag_264 = false;
                    }
                    self.skip = true;
                    if self.unit_dialog.is_none() {
                        self.panel = None;
                    }
                    out.effects.extend(self.close());
                    if self.end_callback {
                        out.effects.push(DialogEffect::EndCallback {
                            skip: false,
                            mouse: false,
                        });
                        r = inp.end_callback_result;
                    } else {
                        if self.timed_box_flag {
                            self.timed_box_flag = false;
                        } else if let Some(g) = self.unit_dialog {
                            out.effects.push(DialogEffect::UnitDialogEnd(g));
                        }
                        out.effects.push(DialogEffect::Restore);
                        self.unit_dialog = None;
                        out.effects.push(DialogEffect::MouseWindowOff);
                        out.effects.push(DialogEffect::UnregisterHandlers);
                        self.handlers = false;
                        r = 0;
                    }
                }
            }
            // Evaluated after the
            // running step, which may have cleared the unit dialog.
            if inp.local_dead_or_absent || (self.unit_dialog.is_some() && inp.unit_in_mode12) {
                if let Some(g) = self.unit_dialog {
                    out.effects
                        .push(DialogEffect::Send(msg_u32s(0x30, &[1, g])));
                    out.effects.push(DialogEffect::UnitDialogCleanup);
                }
                out.effects.push(DialogEffect::Run4a0880);
                // Destroys the dialog window: the next pass finds no running
                // panel.
                self.panel = None;
                self.unit_dialog = None;
                if self.handlers {
                    out.effects.push(DialogEffect::UnregisterHandlers);
                    self.handlers = false;
                }
            }
        }
        if inp.timed_box_drawn {
            r += 1;
        }
        if r == 0 && self.up {
            self.shown_flag = false;
            self.unit_dialog = None;
            self.skip = true;
            self.up = false;
            if self.handlers {
                self.handlers = false;
                out.effects.push(DialogEffect::UnregisterHandlers);
            }
        }
        // At the end of every pass the skip flag is cleared.
        self.skip = false;
        out.r = r;
        out
    }

    /// The skip handlers.
    /// `cb` is what the end callback returns with stack argument 0 and 1:
    /// D2RUST applies the callback's effects after the step, so its owner
    /// decides the return before.
    pub fn skip_event(&mut self, ev: SkipEvent, now: u32, cb: [u32; 2]) -> SkipOutcome {
        match ev {
            SkipEvent::RightUp => SkipOutcome {
                consumed: true,
                effects: Vec::new(),
            },
            SkipEvent::Char { ch, automap_key } if automap_key <= 0xDF && ch == automap_key => {
                SkipOutcome::default()
            }
            SkipEvent::SysKeyDown { f4: true } => SkipOutcome::default(),
            SkipEvent::ButtonDown => {
                self.mouse_down = true;
                self.do_skip(now, cb)
            }
            _ => self.do_skip(now, cb),
        }
    }

    /// The shown flag is cleared (the text hides at once unless the Text Display
    /// option is on); within 100 ms of the open the event is only consumed;
    /// else. Both clear.
    fn do_skip(&mut self, now: u32, cb: [u32; 2]) -> SkipOutcome {
        self.shown_flag = false;
        let mouse = std::mem::take(&mut self.mouse_down);
        let mut out = SkipOutcome {
            consumed: true,
            effects: Vec::new(),
        };
        if now.wrapping_sub(self.t_open) < SKIP_GRACE_MS {
            return out;
        }
        self.skip = true;
        if self.unit_dialog.is_none() && self.panel.is_none() {
            if self.timed_box_flag {
                out.effects.push(DialogEffect::CloseTimedBox);
                self.timed_box_flag = false;
            }
            return out;
        }
        if self.end_callback {
            // The input reset, then the callback (EDX 1, stack); a non-zero return
            // ends the skip there.
            out.effects.extend(self.close());
            out.effects.push(DialogEffect::Restore);
            out.effects
                .push(DialogEffect::EndCallback { skip: true, mouse });
            if cb[usize::from(mouse)] != 0 {
                return out;
            }
            if let Some(g) = self.unit_dialog {
                out.effects.push(DialogEffect::UnitDialogEnd(g));
            }
        } else {
            if let Some(g) = self.unit_dialog {
                out.effects.push(DialogEffect::UnitDialogEnd(g));
            } else {
                out.effects.extend(self.close());
            }
            out.effects.push(DialogEffect::Restore);
        }
        self.unit_dialog = None;
        out.effects.push(DialogEffect::UnregisterHandlers);
        self.handlers = false;
        out
    }
}

/// The font of the panel text: 8 (`FontFormal11`).
pub const PANEL_FONT: u16 = FONT_FORMAL11;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::messages::testutil::w;

    fn open_npc(d: &mut DialogUi, text: &str, unit: Option<u32>) -> Vec<DialogEffect> {
        d.open(
            DialogOpen::Npc { unit, id: 1000 },
            &w(text),
            800,
            600,
            80,
            60,
            5000,
        )
    }

    // Test vector "dialog text 67\nA\nB\nC\nD\nE".
    #[test]
    fn text_lines_and_speed() {
        let t = parse_text(&w("67\nA\nB\nC\nD\nE"));
        assert_eq!(t.speed, 67);
        assert_eq!(t.lines.len(), 5);
        assert_eq!(t.lines[0], w("A"));
        // The scroll finishes when (p >> 10) > 18 · 4 + 112 = 184.
        let mut s = Scroll::new(67);
        s.p = 184 << 10;
        assert!(!s.finished(5));
        s.p = 185 << 10;
        assert!(s.finished(5));
        // A first line with a unit ≥ 0x80: speed 8.
        assert_eq!(parse_text(&w("é\nA")).speed, 8);
        // Lines longer than 100 units are cut, the rest starts the next.
        let long = "x".repeat(250);
        let t = parse_text(&w(&format!("5\n{long}")));
        assert_eq!(
            t.lines.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![100, 100, 50]
        );
        // A line of exactly 100 units followed by LF leaves the LF
        // unconsumed; it becomes an empty next line.
        let t = parse_text(&w(&format!("8\n{}\nb", "a".repeat(100))));
        assert_eq!(t.lines, vec![w(&"a".repeat(100)), Vec::new(), w("b")]);
        // An empty text gives no lines; a speed line alone, none either.
        assert!(parse_text(&[]).lines.is_empty());
        assert!(parse_text(&w("7")).lines.is_empty());
        // The count floor of the finish test is 1 line (max(count − 1, 1)).
        let mut s = Scroll::new(1);
        s.p = 131 << 10;
        assert!(s.finished(0) && s.finished(1));
        s.p = 130 << 10;
        assert!(!s.finished(1));
    }

    #[test]
    fn scroll_per_draw() {
        let mut s = Scroll::new(8);
        // t_start = 0: t_last:= t_start:= t, p stays 0.
        s.update(1000);
        assert_eq!((s.t_start, s.t_last, s.p), (1000, 1000, 0));
        // Another draw in the same ms: step = 0 → same += 1; p = e · speed
        // with e = (t − t_start) >> 2 = 0.
        s.update(1000);
        assert_eq!((s.same, s.p), (1, 0));
        s.update(1000);
        assert_eq!(s.same, 2);
        // A new ms: acc:= 0, step:= (t − t_last) / same (once), t_last:= t.
        s.update(1008);
        assert_eq!((s.acc, s.step, s.t_last), (0, 4, 1008));
        assert_eq!(s.p, 2 * 8);
        // Same ms again: step ≠ 0 → acc += step and a = acc.
        s.update(1008);
        assert_eq!((s.acc, s.p), (4, (2 + 4) * 8));
        s.update(1008);
        assert_eq!((s.acc, s.p), (8, (2 + 8) * 8));
        // A later ms resets acc; step stays.
        s.update(1012);
        assert_eq!((s.acc, s.step, s.p), (0, 4, 3 * 8));
    }

    fn texts(d: &[DialogDraw]) -> Vec<(usize, i32)> {
        d.iter()
            .filter_map(|d| match d {
                DialogDraw::Line(l, i) => Some((*i, l.y)),
                _ => None,
            })
            .collect()
    }

    fn lines(n: usize) -> Vec<Vec<u16>> {
        (0..n).map(|i| w(&format!("L{i}"))).collect()
    }

    #[test]
    fn panel_draw_windows() {
        let ls = lines(5);
        // p = 0: line 0 is whole at y + 112; nothing above; the line
        // below (line 1) is a window: s = 17, h = 20 → skip 17 lines 3.
        let d = panel_draw(237, 12, true, 0, &ls, 20);
        assert_eq!(
            d[0],
            DialogDraw::Backing(RectDraw {
                x: 237,
                y: 7,
                w: 325,
                h: 122,
                color: 0,
                mode: 1
            })
        );
        assert_eq!(
            d[1],
            DialogDraw::Border {
                l: 236,
                t: 6,
                r: 563,
                b: 129
            }
        );
        assert_eq!(texts(&d), vec![(0, 12 + 112)]);
        assert_eq!(
            d[3],
            DialogDraw::Window {
                line: 1,
                x: 253,
                y: 12 + 130,
                skip: 17,
                lines: 3
            }
        );
        // Place ≠ 0: no backing or border.
        let d = panel_draw(80, 201, false, 0, &ls, 20);
        assert!(matches!(d[0], DialogDraw::Line(..)));
        // p = 30 px: line 0 at q = 30720 → y + 112 − 30; line 1 at q =
        // 12288 → y + 112 − 12; line 2 below: s = 17 − 12 = 5.
        let d = panel_draw(0, 0, false, 30 * 1024, &ls, 20);
        assert_eq!(texts(&d), vec![(0, 82), (1, 100)]);
        assert_eq!(
            d.last().unwrap(),
            &DialogDraw::Window {
                line: 2,
                x: 16,
                y: 118,
                skip: 5,
                lines: 15
            }
        );
        // p = 100 px: line 0 is above the window (q = 102400 ≥ 96256):
        // drawn as a window above line 1 (q = 83968 → 82 px up): v = 94 −
        // 82 = 12.
        let d = panel_draw(0, 0, false, 100 * 1024, &ls, 20);
        assert_eq!(
            d[0],
            DialogDraw::Window {
                line: 0,
                x: 16,
                y: 12,
                skip: 0,
                lines: 12
            }
        );
        assert_eq!(texts(&d)[0].0, 1);
        // No whole line: the last line, q = p − 18432 (count − 1).
        let one = lines(1);
        let p = 100 * 1024 + 512;
        let d = panel_draw(0, 0, false, p, &one, 20);
        // q = p ≥ 96256: v = 112 − ((q + 1024) >> 10) = 112 − 101 = 11.
        assert_eq!(
            d,
            vec![DialogDraw::Window {
                line: 0,
                x: 16,
                y: 11,
                skip: 0,
                lines: 11
            }]
        );
        // Past the end: nothing (v = 0).
        assert!(panel_draw(0, 0, false, 113 * 1024, &one, 20).is_empty());
        // A negative-s case does not occur; s ≥ h draws nothing: font
        // height 10, s = 17 → no window below.
        let d = panel_draw(0, 0, false, 0, &ls, 10);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn wait_for_speech() {
        assert!(waiting_for_speech(true, false, false));
        assert!(!waiting_for_speech(false, false, false));
        assert!(!waiting_for_speech(true, true, false));
        assert!(!waiting_for_speech(true, false, true));
    }

    #[test]
    fn open_places_and_state() {
        let mut d = DialogUi::default();
        let e = d.open(
            DialogOpen::Quest {
                id: 127,
                place: 0,
                flag: true,
            },
            &w("8\nhello"),
            800,
            600,
            80,
            60,
            5000,
        );
        // Place 0: x = (W − 325) / 2 (C division), y = 12.
        assert_eq!((d.x, d.y, d.place1), (237, 12, false));
        assert_eq!(
            e,
            vec![
                DialogEffect::FadeSpeech,
                DialogEffect::InputReset,
                DialogEffect::MouseWindow {
                    l: 227,
                    t: 12,
                    r: 572,
                    b: 194
                },
                DialogEffect::RegisterHandlers,
                DialogEffect::RequestSpeech { id: 127, d: 5 },
            ]
        );
        assert!(d.up && d.shown_flag && d.speech_pending && d.handlers);
        assert_eq!((d.id, d.t_open), (127, 5000));
        // The quest flag with one of the five ids → 0x31 at the close.
        assert!(d.close_flag);
        let p = d.panel.as_ref().unwrap();
        assert_eq!((p.lines.len(), p.scroll.speed, p.state), (1, 8, 0));
        // A second open while a panel is up changes only the id, the
        // position and the stopped speech.
        let e = d.open(
            DialogOpen::Quest {
                id: 5,
                place: 1,
                flag: false,
            },
            &w("9\nother"),
            800,
            600,
            80,
            60,
            9000,
        );
        assert_eq!((d.x, d.y, d.place1, d.id), (80, 201, true, 5));
        assert_eq!(d.t_open, 5000);
        assert_eq!(d.panel.as_ref().unwrap().lines[0], w("hello"));
        assert!(!e.iter().any(|e| matches!(
            e,
            DialogEffect::RegisterHandlers | DialogEffect::RequestSpeech { .. }
        )));
        assert!(!d.speech_pending);
        // The mouse window ends at min(y + 182, H − 53).
        assert!(e.contains(&DialogEffect::MouseWindow {
            l: 70,
            t: 201,
            r: 415,
            b: 383
        }));
        let mut d = DialogUi::default();
        let e = d.open(
            DialogOpen::Quest {
                id: 1,
                place: 1,
                flag: false,
            },
            &w("1\nx"),
            640,
            480,
            0,
            0,
            1,
        );
        assert!(e.contains(&DialogEffect::MouseWindow {
            l: -10,
            t: 261,
            r: 335,
            b: 427
        }));
        // A flag ≠ 0 with another id does not arm 0x31.
        assert!(!d.close_flag);
        // A unit dialog always opens at place 0, detaches the NPC's voices
        // and keeps the unit GUID.
        let mut d = DialogUi::default();
        let e = open_npc(&mut d, "3\nhi", Some(7));
        assert_eq!((d.x, d.y, d.unit_dialog), (237, 12, Some(7)));
        assert!(e.contains(&DialogEffect::DetachSkillVoices));
        assert!(!d.shown_flag);
        // A different unit already in a dialog gets C→S 0x30 [1][GUID].
        let e = open_npc(&mut d, "3\nhi", Some(9));
        assert!(e.contains(&DialogEffect::Send(ClientIntent(vec![
            0x30, 1, 0, 0, 0, 7, 0, 0, 0
        ]))));
        assert!(e.contains(&DialogEffect::UnitDialogCleanup));
        assert_eq!(d.unit_dialog, Some(9));
        // Without a unit: the timed box closes and the unit is cleared.
        d.timed_box_flag = true;
        let e = open_npc(&mut d, "3\nhi", None);
        assert!(e.contains(&DialogEffect::CloseTimedBox));
        assert_eq!((d.unit_dialog, d.timed_box_flag), (None, false));
    }

    #[test]
    fn close_sends_0x31_once() {
        let mut d = DialogUi::default();
        d.open(
            DialogOpen::Quest {
                id: 20131,
                place: 0,
                flag: true,
            },
            &w("1\nx"),
            800,
            600,
            0,
            0,
            1,
        );
        let e = d.close();
        assert_eq!(
            e,
            vec![
                DialogEffect::MouseWindowOff,
                DialogEffect::FadeSpeech,
                DialogEffect::UnregisterHandlers,
                DialogEffect::Send(ClientIntent(vec![
                    0x31, 0xFF, 0xFF, 0xFF, 0xFF, 0xA3, 0x4E, 0, 0
                ])),
            ]
        );
        assert!(d.panel.is_none() && !d.close_flag);
        assert_eq!(d.close().len(), 3);
    }

    fn running(speed: &str) -> DialogUi {
        let mut d = DialogUi::default();
        d.open(
            DialogOpen::Quest {
                id: 1,
                place: 0,
                flag: false,
            },
            &w(&format!("{speed}\nA\nB")),
            800,
            600,
            0,
            0,
            1,
        );
        d.speech_pending = false;
        d
    }

    #[test]
    fn pass_abort_tests_the_unit_dialog_after_the_step() {
        // A finishing step clears the unit dialog: mode 12 no longer aborts.
        let mut d = running("100");
        d.unit_dialog = Some(4);
        d.panel.as_mut().unwrap().scroll.t_start = 1;
        d.panel.as_mut().unwrap().scroll.p = 0;
        let o = d.pass(&PassInput {
            now: 1_000_000,
            unit_in_mode12: true,
            ..Default::default()
        });
        assert!(!o.effects.contains(&DialogEffect::Run4a0880));
        // A dead player with handlers registered: Run4a0880, then unregister.
        let mut d = running("8");
        d.handlers = true;
        let o = d.pass(&PassInput {
            now: 1000,
            local_dead_or_absent: true,
            ..Default::default()
        });
        let at = |e: DialogEffect| o.effects.iter().position(|x| *x == e);
        let (a, b) = (
            at(DialogEffect::Run4a0880).unwrap(),
            at(DialogEffect::UnregisterHandlers).unwrap(),
        );
        assert!(a < b);
        assert!(d.unit_dialog.is_none() && !d.handlers);
    }

    #[test]
    fn pass_runs_then_ends() {
        let mut d = running("8");
        let mut now = 1000;
        let pass = |d: &mut DialogUi, now: u32| {
            d.pass(&PassInput {
                now,
                ..Default::default()
            })
        };
        // Running: R = 1, drawn.
        let o = pass(&mut d, now);
        assert_eq!((o.r, o.drawn), (1, true));
        // Waiting for speech: R = 1, nothing scrolled.
        let o = d.pass(&PassInput {
            now,
            speech_waiting: true,
            ..Default::default()
        });
        assert_eq!((o.r, o.drawn), (1, false));
        // A skip request: +0x16:= 2, R = 0; the panel then ends (R = 0 and up:
        // flags cleared, handlers unregistered).
        d.skip = true;
        let o = pass(&mut d, now);
        assert_eq!(o.r, 0);
        assert!(!d.up && !d.shown_flag && !d.handlers && !d.skip);
        assert!(o.effects.contains(&DialogEffect::UnregisterHandlers));
        assert_eq!(d.panel.as_ref().unwrap().state, 2);
        // Finishing: scroll until (p >> 10) > 18 · 1 + 112.
        let mut d = running("100");
        let mut last = PassOutput::default();
        for _ in 0..2000 {
            now += 40;
            last = pass(&mut d, now);
            if last.r == 0 {
                break;
            }
        }
        assert_eq!(last.r, 0);
        assert!(last.effects.contains(&DialogEffect::Restore));
        assert!(d.panel.is_none() && !d.up);
        // A unit dialog keeps the panel and ends with the unit interaction's end (GUID).
        let mut d = running("100");
        d.unit_dialog = Some(4);
        d.panel.as_mut().unwrap().scroll.t_start = 1;
        d.panel.as_mut().unwrap().scroll.p = 0;
        let o = d.pass(&PassInput {
            now: 1_000_000,
            ..Default::default()
        });
        assert_eq!(o.r, 0);
        assert!(o.effects.contains(&DialogEffect::UnitDialogEnd(4)));
        assert!(d.unit_dialog.is_none());
        // An end callback: R is its result; the timed box adds 1.
        let mut d = running("100");
        d.end_callback = true;
        d.panel.as_mut().unwrap().scroll.t_start = 1;
        let o = d.pass(&PassInput {
            now: 1_000_000,
            end_callback_result: 1,
            ..Default::default()
        });
        assert_eq!(o.r, 1);
        assert!(o.effects.contains(&DialogEffect::EndCallback {
            skip: false,
            mouse: false
        }));
        let mut d = running("8");
        let o = d.pass(&PassInput {
            now: 10,
            timed_box_drawn: true,
            ..Default::default()
        });
        assert_eq!(o.r, 2);
        // A dead local player in a unit dialog: C→S 0x30 [1][GUID], then.
        let mut d = running("8");
        d.unit_dialog = Some(4);
        let o = d.pass(&PassInput {
            now: 10,
            local_dead_or_absent: true,
            ..Default::default()
        });
        assert!(o.effects.contains(&DialogEffect::Send(ClientIntent(vec![
            0x30, 1, 0, 0, 0, 4, 0, 0, 0
        ]))));
        assert!(o.effects.contains(&DialogEffect::Run4a0880));
        // Skip is cleared at the end of every pass.
        assert!(!d.skip);
    }

    #[test]
    fn skip_input() {
        assert_eq!(SKIP_TABLE.len(), 7);
        let mut d = running("8");
        d.t_open = 1000;
        d.shown_flag = true;
        // Right button up: consumed only.
        let o = d.skip_event(SkipEvent::RightUp, 5000, [0, 0]);
        assert!(o.consumed && o.effects.is_empty() && d.shown_flag);
        // WM_CHAR equal to the key of binding 7 (≤ 0xDF) does not skip;
        // another character does.
        let o = d.skip_event(
            SkipEvent::Char {
                ch: 0x4D,
                automap_key: 0x4D,
            },
            5000,
            [0, 0],
        );
        assert!(!o.consumed && d.panel.is_some());
        // F4 passes.
        assert!(
            !d.skip_event(SkipEvent::SysKeyDown { f4: true }, 5000, [0, 0])
                .consumed
        );
        // Within 100 ms of the open: the text hides, the event is only
        // consumed.
        let o = d.skip_event(SkipEvent::Key, 1099, [0, 0]);
        assert!(o.consumed && o.effects.is_empty());
        assert!(!d.shown_flag && !d.skip && d.panel.is_some());
        // Later: the skip flag is set, the panel closes, handlers go.
        let o = d.skip_event(SkipEvent::ButtonDown, 1100, [0, 0]);
        assert!(o.consumed && d.skip);
        assert!(o.effects.contains(&DialogEffect::Restore));
        assert!(d.panel.is_none() && !d.handlers);
        // With neither a unit dialog nor a panel, a shown timed box closes.
        let mut d = DialogUi {
            timed_box_flag: true,
            ..Default::default()
        };
        let o = d.skip_event(SkipEvent::Key, 5000, [0, 0]);
        assert_eq!(o.effects, vec![DialogEffect::CloseTimedBox]);
        assert!(!d.timed_box_flag);
        // With a unit dialog; with an end callback: EDX 1.
        let mut d = running("8");
        d.unit_dialog = Some(3);
        let o = d.skip_event(SkipEvent::Key, 5000, [0, 0]);
        assert!(o.effects.contains(&DialogEffect::UnitDialogEnd(3)));
        let mut d = running("8");
        d.end_callback = true;
        let o = d.skip_event(SkipEvent::Key, 5000, [0, 0]);
        assert!(o.effects.contains(&DialogEffect::EndCallback {
            skip: true,
            mouse: false
        }));
        // A button down: stack argument 1; a non-zero return keeps the
        // unit dialog and the handlers (the callback opened the next line).
        let mut d = running("8");
        d.end_callback = true;
        d.unit_dialog = Some(3);
        let o = d.skip_event(SkipEvent::ButtonDown, 5000, [0, 1]);
        assert!(o.effects.contains(&DialogEffect::EndCallback {
            skip: true,
            mouse: true
        }));
        assert!(!o.effects.contains(&DialogEffect::UnitDialogEnd(3)));
        assert_eq!(d.unit_dialog, Some(3));
        assert!(!d.mouse_down);
        // Return 0: the unit interaction ends and the unit dialog is cleared.
        let mut d = running("8");
        d.end_callback = true;
        d.unit_dialog = Some(3);
        let o = d.skip_event(SkipEvent::ButtonDown, 5000, [1, 0]);
        assert!(o.effects.contains(&DialogEffect::UnitDialogEnd(3)));
        assert_eq!(d.unit_dialog, None);
        // Within the grace a button down only clears.
        let mut d = running("8");
        d.t_open = 4990;
        d.skip_event(SkipEvent::ButtonDown, 5000, [0, 0]);
        assert!(!d.mouse_down && d.panel.is_some());
        // A character above 0xDF in binding 7 does not suppress.
        let mut d = running("8");
        let o = d.skip_event(
            SkipEvent::Char {
                ch: 0xE0,
                automap_key: 0xE0,
            },
            5000,
            [0, 0],
        );
        assert!(o.consumed);
    }
}
