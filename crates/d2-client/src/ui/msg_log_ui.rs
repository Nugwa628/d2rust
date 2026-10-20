//! The message log (ui 0x18, key M): a menu box with the title `Message Log`
//! and a `Close` item and, inside it, a list box with the messages of
//! the message log, newest first.
//!
//! The box is the menu box `(5, 30, handler, p6 = W − 20, p7 = H − 130)`, style 1,
//! with the items `strMsgLog` (4035; height 21, colour 4, not selectable) and
//! `strClose` (4144; height H − 165, selectable; its handler is
//! `SetUIState(0x18, off)`); the list box (`(5, 65, W − 20, H − 200,
//! spacing 2, …, font 13)`) is its child. Draw order (then the child): the box
//! rectangle and border, the list's border, its scroll arrows and body
//! (`menu\textslid` frames 9 / 11, 13, 8 / 10), the lines of the entries from
//! the first visible one, the slider thumb, then the box items.
//!
//! The lines of an entry (up to six, wrapped at `W − 70` in font 13) are
//! drawn at x = list x + 0x20, y = list y + 0x19 + 14 per line + 2 per
//! entry, in the entry's colour; a line wider than the list minus 0x32 loses
//! trailing units first.
//!
//! The scroll arrows: a left press on an arrow's cell marks it pressed and
//! zeroes the repeat counter; every 40 ms of the UI clock (`super::ui_clock`,
//! the first draw at once) then counts a frame while the pointer stays in
//! the pressed cell, at any frame rate, and steps on each count that is not a
//! multiple of 10 (the list box's arrow step); the release steps once more over the cell and clears
//! both marks. The wheel steps once per event, the Up / Down keys step, Left /
//! Right are only consumed. A step plays the silent request 0.

use std::cell::{Cell, RefCell};

use super::game_messages::Measure;
use super::npc_box::{item_at, push_menu_draws};
use super::{esc_menu, SharedRef};
use crate::ui::draw::{CelCall, ImageRef, ImageRequest, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::messages::chat::ScreenMessage;
use crate::ui::messages::{Metrics, FONT_CHAT};
use crate::ui::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::menu_box::{MenuBox, MenuDraw, MenuParams};
use crate::ui::panels::{text, PanelOutput};
use crate::ui::PointerButton;

/// The state UI id of the message log.
pub const UI_MESSAGE_LOG: u8 = 0x18;
/// `menu\textslid` (17 frames of 12 × 13): the list's scroll art.
pub const TEXTSLID_FILE: &str = "menu\\textslid";
/// `strMsgLog` and `strClose`.
const STR_TITLE: u16 = 4035;
const STR_CLOSE: u16 = 4144;
/// The box and list placement.
const BOX_X: i32 = 5;
const BOX_Y: i32 = 30;
const LIST_X: i32 = 5;
const LIST_Y: i32 = 65;
/// Spacing per entry (`param_6`), per line, the first line's offset and the
/// text's indent.
const ENTRY_GAP: i32 = 2;
const LINE_STEP: i32 = 14;
const FIRST_LINE: i32 = 0x19;
const TEXT_INDENT: i32 = 0x20;
/// The line clip is the list width minus this.
const CLIP_MARGIN: i32 = 0x32;
/// Cel pieces are 12 high.
const PIECE: i32 = 12;
/// `textslid` frames.
const F_DOWN: u32 = 8;
const F_UP: u32 = 9;
const F_BODY: u32 = 0xd;
const F_THUMB_TOP: u32 = 0x10;
const F_THUMB_MID: u32 = 0xe;
const F_THUMB_BOTTOM: u32 = 0xf;
/// The thumb track lengths: 18, or 28 at 800 × 600
/// for the message log's list.
const TRACK_800: i32 = 28;
const TRACK_640: i32 = 18;

/// The files the log draws, for the UI file table.
pub(super) fn msg_log_files() -> [String; 1] {
    [TEXTSLID_FILE.to_string()]
}

/// The box handler: `SetUIState(0x18, off)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CloseLog;

/// The geometry of the log for a frame of `w` × `h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogGeom {
    /// The list box: x, y, width, height.
    pub list: (i32, i32, i32, i32),
    /// The scroll column x and the first piece's y.
    pub scroll: (i32, i32),
    /// The body pieces between the arrows.
    pub body: i32,
    /// The thumb track length.
    pub track: i32,
}

impl LogGeom {
    pub fn new(w: i32, h: i32, res2: bool) -> Self {
        let (lw, lh) = (w - 20, h - 200);
        let body = if lh < 0x20 {
            1
        } else {
            (lh - 0x10) / PIECE - 1
        };
        LogGeom {
            list: (LIST_X, LIST_Y, lw, lh),
            scroll: (LIST_X - 0x12 + lw, LIST_Y + 0x10),
            body,
            track: if res2 { TRACK_800 } else { TRACK_640 },
        }
    }
}

/// How many entries fit from `first`: the entries are
/// counted while the heights of the entries from `first` stay within the
/// list height minus 20; the result is at least 1 once one does not.
/// `lines[i]` is the line count of entry `i`.
pub fn visible_count(lines: &[usize], first: usize, list_h: i32) -> usize {
    let mut acc = 0;
    for (i, &n) in lines.iter().enumerate() {
        if i >= first {
            acc += LINE_STEP * n as i32 + ENTRY_GAP;
        }
        if list_h - 0x14 < acc {
            return (i.saturating_sub(first)).max(1);
        }
    }
    lines.len()
}

/// The entries counted by the thumb: from `first` while the
/// running height stays below the list height minus 20.
fn thumb_count(lines: &[usize], first: usize, list_h: i32) -> usize {
    let mut acc = 0;
    let mut n = 0;
    for &l in lines.iter().skip(first) {
        acc += LINE_STEP * l as i32 + ENTRY_GAP;
        if acc < list_h - 0x14 {
            n += 1;
        }
    }
    n
}

/// The slider thumb pieces: `(frame, y)` from the scroll
/// column's x − 1, top piece, middle pieces, bottom piece; `None` when all
/// entries are visible.
pub fn thumb(lines: &[usize], first: usize, g: &LogGeom) -> Option<Vec<(u32, i32)>> {
    let total = lines.len() as i32;
    let v = thumb_count(lines, first, g.list.3) as i32;
    if v >= total {
        return None;
    }
    let ratio = (v << 7) / total;
    let first = first as i32;
    let (top, len);
    if first == 0 {
        top = 0;
        len = (g.track * ratio) >> 7;
    } else if first < total - v {
        let t = ((g.track * first) / total).max(0);
        top = t;
        let l = ((g.track * ratio) >> 7) + t;
        len = if l <= g.track { l } else { g.track };
    } else {
        len = g.track;
        top = (g.track - ((g.track * ratio) >> 7)).min(g.track);
    }
    let base = g.scroll.1 + 0xd;
    let y0 = base + top * PIECE;
    let mut out = vec![(F_THUMB_TOP, y0)];
    let mut bottom = top;
    if top < len {
        let mut y = y0 + PIECE;
        for _ in 0..(len - top) {
            out.push((F_THUMB_MID, y));
            y += PIECE;
        }
        bottom = len;
    }
    out.push((F_THUMB_BOTTOM, base + (bottom * 3 + 3) * 4));
    Some(out)
}

/// A line cut to the list width minus 0x32: units come off
/// the end while it is wider.
pub fn clip_line(line: &[u16], width_c: &dyn Fn(&[u16]) -> i32, list_w: i32) -> Vec<u16> {
    let max = list_w - CLIP_MARGIN;
    let mut t = line.to_vec();
    while max < width_c(&t) && !t.is_empty() {
        t.pop();
    }
    t
}

/// The log panel (ui 0x18).
pub(super) struct MsgLogUi {
    pub(super) sh: SharedRef,
    /// The first visible entry (`[0x601CC]`).
    first: Cell<usize>,
    /// The item and arrow a left press went down on.
    pressed: RefCell<Pressed>,
    /// The hovered item (`+0x44`); `None` = the box's own choice.
    selected: Cell<Option<i32>>,
    /// The held-arrow repeat counter.
    repeat: Cell<u32>,
    /// The repeat's 40 ms clock.
    repeat_clock: Cell<crate::ui::ui_clock::StepCounter>,
    /// The pointer as of the last pointer event (the draw reads the mouse,
    /// not only at a pointer event).
    pointer: Cell<Option<Point>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pressed {
    item: Option<usize>,
    up: bool,
    down: bool,
}

impl MsgLogUi {
    pub(super) fn new(sh: SharedRef) -> Self {
        MsgLogUi {
            sh,
            first: Cell::new(0),
            pressed: RefCell::default(),
            selected: Cell::new(None),
            repeat: Cell::new(0),
            repeat_clock: Cell::new(Default::default()),
            pointer: Cell::new(None),
        }
    }

    /// The box with its two items; `None` without the
    /// strings.
    fn build(&self, ctx: &UiCtx, m: &Measure, w: i32, h: i32) -> Option<MenuBox<CloseLog>> {
        let title = ctx.strings.get_id(STR_TITLE)?;
        let close = ctx.strings.get_id(STR_CLOSE)?;
        let mut bx = MenuBox::new(
            (BOX_X, BOX_Y),
            MenuParams {
                p1: Some(CloseLog),
                p6: w - 20,
                p7: h - 130,
                ..Default::default()
            },
        )?;
        bx.set_style(1);
        bx.add_item(title, 21, 0, 4, 1, None, false, m).ok()?;
        bx.add_item(close, h - 165, 0, 0, 1, Some(CloseLog), true, m)
            .ok()?;
        // The open ends with no selection: nothing
        // is selected until the pointer moves onto an item.
        bx.selected = self.selected.get().unwrap_or(-1);
        Some(bx)
    }

    fn entries(&self) -> Vec<ScreenMessage> {
        self.sh
            .borrow()
            .messages
            .list()
            .log()
            .iter()
            .cloned()
            .collect()
    }

    /// The arrow cells (the same test as the arrow step): x in (column
    /// x, column x + 16), y in (bottom − 12, bottom), both exclusive; the up
    /// arrow's bottom is the first piece's y, the down arrow's that plus 12 per
    /// body piece.
    fn arrows(g: &LogGeom) -> (Rect, Rect) {
        let (sx, sy) = g.scroll;
        let r = |y: i32| Rect::new(sx + 1, y - PIECE + 1, 15, (PIECE - 1) as u16);
        (r(sy), r(sy + g.body * PIECE))
    }

    /// The scroll column from the up arrow's top to the down arrow's
    /// bottom. A click there scrolls (on an arrow) or does nothing; it
    /// never closes the log, though the log's item spans the whole box.
    fn scroll_column(g: &LogGeom) -> Rect {
        let (up, down) = Self::arrows(g);
        let h = u16::try_from(down.bottom() - i64::from(up.y)).unwrap_or(0);
        Rect::new(up.x, up.y, up.w, h)
    }

    /// The line counts of the log's entries.
    fn lines(&self) -> Vec<usize> {
        self.sh
            .borrow()
            .messages
            .list()
            .log()
            .iter()
            .map(line_count)
            .collect()
    }

    /// One step up (no callback): the first visible entry goes back by one;
    /// with no entry counted visible (`[+0x18074E]` = 0) the step is undone.
    /// `true` when the step ran (and its sound).
    fn step_up(&self, lines: &[usize], list_h: i32) -> bool {
        let first = self.first.get();
        if first == 0 {
            return false;
        }
        if thumb_count(lines, first, list_h) > 0 {
            self.first.set(first - 1);
        }
        true
    }

    /// One step down (no callback): the first visible entry moves on by one
    /// while the visible ones (`[+0x18074E]`, the thumb's count) do not reach
    /// the last entry.
    fn step_down(&self, lines: &[usize], list_h: i32) -> bool {
        let first = self.first.get();
        if first + thumb_count(lines, first, list_h) < lines.len() {
            self.first.set(first + 1);
            true
        } else {
            false
        }
    }

    /// Runs a step and queues its click.
    fn step(&self, down: bool, g: &LogGeom) {
        let lines = self.lines();
        let ran = if down {
            self.step_down(&lines, g.list.3)
        } else {
            self.step_up(&lines, g.list.3)
        };
        if ran {
            self.sh
                .borrow_mut()
                .outputs
                .push(PanelOutput::Sound(super::CLICK_SOUND_ID));
        }
    }

    /// The end of the list's draw at UI clock `now_ms`: a held arrow with
    /// the pointer in its cell counts a frame per 40 ms passed (module doc)
    /// and steps on each count that is not 0 or a multiple of 10. Time with
    /// the pointer outside the cell does not count.
    fn held_repeat(&self, g: &LogGeom, now_ms: u64) {
        let mut clock = self.repeat_clock.get();
        let held = self.pointer.get().and_then(|p| {
            let pressed = *self.pressed.borrow();
            let (up, down) = Self::arrows(g);
            if up.contains(p) && pressed.up {
                Some(false)
            } else if down.contains(p) && pressed.down {
                Some(true)
            } else {
                None
            }
        });
        let Some(down_held) = held else {
            clock.reset();
            self.repeat_clock.set(clock);
            return;
        };
        let frames = clock.steps(now_ms, 25);
        self.repeat_clock.set(clock);
        for _ in 0..frames {
            let n = self.repeat.get().wrapping_add(1);
            self.repeat.set(n);
            if n != 0 && !n.is_multiple_of(10) {
                self.step(down_held, g);
            }
        }
    }
}

/// The lines an entry counts for: the
/// non-empty ones of its six slots.
fn line_count(e: &ScreenMessage) -> usize {
    e.lines.iter().filter(|l| !l.is_empty()).count()
}

fn cel(file: u32, frame: u32, x: i32, y: i32) -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef { file, frame },
        at: Point::new(x, y),
        clip: crate::ui::FRAME,
        look: Default::default(),
        call: CelCall::Draw,
    })
}

/// The log's draws, laid out across the frame, into the UI box's space:
/// a whole-screen clip becomes the whole frame, then the draw moves `ox`
/// left.
struct LogSink<'a> {
    out: &'a mut dyn UiDrawSink,
    ox: i32,
    clip: Rect,
}

impl UiDrawSink for LogSink<'_> {
    fn push(&mut self, mut d: UiDraw) {
        d.widen_clip(crate::ui::FRAME, self.clip);
        d.shift_x(-self.ox);
        self.out.push(d);
    }
}

/// The message log covers the whole frame: `(ox, w)` are the columns of
/// frame left of the UI box and the frame's width. The log is laid out for
/// a screen `w` wide and moved `ox` left into the box's space (0 and `W` at
/// 4:3).
fn log_frame(s: &crate::ui::layout::Screen) -> (i32, i32) {
    (s.left, s.w + 2 * s.left)
}

impl Panel for MsgLogUi {
    /// The state flag mirrors the panel (the NPC menu's way).
    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_MESSAGE_LOG))
    }

    fn rect(&self) -> Rect {
        let s = self.sh.borrow().config.screen;
        let (ox, w) = log_frame(&s);
        Rect::new(BOX_X - ox, BOX_Y, (w - 20) as u16, (s.h - 130) as u16)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let s = sh.config.screen;
        // The log is laid out across the whole frame and moved into the
        // UI box's space (`log_frame`).
        let (ox, w) = log_frame(&s);
        let s = crate::ui::layout::Screen { w, ..s };
        let mut out = LogSink {
            out,
            ox,
            clip: crate::ui::geom::screen_clip((s.w, s.h)),
        };
        let out = &mut out;
        let m = Measure(sh.fonts.as_ref());
        let Some(bx) = self.build(ctx, &m, s.w, s.h) else {
            return;
        };
        let border = sh.tables.files.id(crate::ui::box_border::BOXPIECES_FILE);
        let clip = crate::ui::geom::screen_clip((s.w, s.h));
        let mut spin = 0;
        let mut parts = bx.draw(&mut spin, &m).into_iter();
        // The box rectangle and its border, then the child list, then the
        // items.
        let head: Vec<MenuDraw> = parts.by_ref().take(2).collect();
        push_menu_draws(head, clip, border, out);
        let g = LogGeom::new(s.w, s.h, s.res2());
        let (lx, ly, lw, lh) = g.list;
        if let Some(b) = border {
            crate::ui::box_border::push_border(b, (lx, ly, lx + lw - 1, ly + lh - 1), clip, out);
        }
        let entries = self.entries();
        let lines: Vec<usize> = entries.iter().map(line_count).collect();
        let first = self.first.get().min(entries.len().saturating_sub(1));
        if let Some(f) = sh.tables.files.id(TEXTSLID_FILE) {
            let p = *self.pressed.borrow();
            let (sx, sy) = g.scroll;
            out.push(cel(f, F_UP + 2 * u32::from(p.up), sx, sy));
            for i in 0..g.body {
                out.push(cel(f, F_BODY, sx, sy + PIECE * (i + 1)));
            }
            out.push(cel(
                f,
                F_DOWN + 2 * u32::from(p.down),
                sx,
                sy + PIECE * g.body,
            ));
            if let Some(pieces) = thumb(&lines, first, &g) {
                for (frame, y) in pieces {
                    out.push(cel(f, frame, sx - 1, y));
                }
            }
        }
        // The lines of the visible entries.
        let shown = visible_count(&lines, first, lh);
        let width_c = |t: &[u16]| m.width_c(FONT_CHAT, t);
        let mut line_no = 0;
        for (k, e) in entries.iter().enumerate().skip(first).take(shown) {
            for l in &e.lines {
                if l.is_empty() {
                    continue;
                }
                let t = clip_line(l, &width_c, lw);
                out.push(text(
                    t,
                    lx + TEXT_INDENT,
                    ENTRY_GAP * (k - first) as i32 + LINE_STEP * line_no + FIRST_LINE + ly,
                    FONT_CHAT,
                    u16::try_from(e.color).unwrap_or(0),
                ));
                line_no += 1;
            }
        }
        push_menu_draws(parts.collect(), clip, border, out);
        let now = sh.ui_ms(ctx.tick);
        drop(sh);
        self.held_repeat(&g, now);
    }

    fn ui_elements(&self, ctx: &UiCtx) -> Vec<crate::ui::ui_ids::UiElement> {
        use crate::ui::ui_ids::UiElement;
        let sh = self.sh.borrow();
        let s = sh.config.screen;
        let (ox, w) = log_frame(&s);
        let m = Measure(sh.fonts.as_ref());
        let Some(bx) = self.build(ctx, &m, w, s.h) else {
            return Vec::new();
        };
        let mut out = super::npc_box::menu_elements(&bx, "message_log");
        let g = LogGeom::new(w, s.h, s.res2());
        let column = Self::scroll_column(&g);
        for e in &mut out {
            e.id = "message_log.close".into();
            // The close strip ends left of the scroll column, which does
            // not close the log.
            let right = e.rect.right().min(i64::from(column.x));
            if right < e.rect.right() {
                e.rect.w = u16::try_from(right - i64::from(e.rect.x)).unwrap_or(0);
                e.at = crate::ui::ui_ids::centre(e.rect);
            }
        }
        let (up, down) = Self::arrows(&g);
        // An arrow at its end of the list steps nothing (`step_up` /
        // `step_down`); its pressed art clears on the release.
        let lines = self.lines();
        let first = self.first.get();
        out.extend([
            UiElement::new("message_log.scroll_up", up)
                .idle_if(first == 0, "the log already shows its newest entry"),
            UiElement::new("message_log.scroll_down", down).idle_if(
                first + thumb_count(&lines, first, g.list.3) >= lines.len(),
                "the log already shows its oldest entry",
            ),
        ]);
        for e in &mut out {
            e.rect.x -= ox;
            e.at.x -= ox;
        }
        out
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let s = self.sh.borrow().config.screen;
        let (ox, w) = log_frame(&s);
        let (e, s) = (e.shifted(ox), crate::ui::layout::Screen { w, ..s });
        let g = LogGeom::new(s.w, s.h, s.res2());
        match e {
            UiEvent::CursorLeft => {
                self.pointer.set(None);
                return UiResponse::Ignored;
            }
            // The box offers the arrow keys to the list: Up / Down step, Left /
            // Right return 0 and the box's item step has fewer than two
            // selectable items: consumed either way.
            UiEvent::Char(c) => {
                return match c {
                    esc_menu::CHAR_UP | esc_menu::CHAR_DOWN => {
                        self.step(c == esc_menu::CHAR_DOWN, &g);
                        UiResponse::Consumed
                    }
                    esc_menu::CHAR_LEFT | esc_menu::CHAR_RIGHT => UiResponse::Consumed,
                    _ => UiResponse::Ignored,
                };
            }
            _ => {}
        }
        let Some(p) = e.at() else {
            return UiResponse::Ignored;
        };
        self.pointer.set(Some(p));
        let fonts = self.sh.borrow().fonts.clone();
        let m = Measure(fonts.as_ref());
        let Some(bx) = self.build(ctx, &m, s.w, s.h) else {
            return UiResponse::Ignored;
        };
        let r = bx.rect();
        let inside = p.x >= r.l && p.x < r.r && p.y >= r.t && p.y < r.b;
        let (up, down) = Self::arrows(&g);
        let item = item_at(&bx, p).filter(|_| !Self::scroll_column(&g).contains(p));
        match e {
            UiEvent::CursorMoved(_) => {
                if let Some(i) = item {
                    self.selected.set(Some(i as i32));
                }
                UiResponse::Ignored
            }
            // One step per wheel event, up away from the
            // user; the box consumes it wherever the pointer is.
            UiEvent::Wheel { steps, .. } => {
                if steps != 0 {
                    self.step(steps < 0, &g);
                }
                UiResponse::Consumed
            }
            UiEvent::Press {
                button: PointerButton::Left,
                ..
            } if inside => {
                let pressed = Pressed {
                    item,
                    up: up.contains(p),
                    down: down.contains(p),
                };
                if pressed.up || pressed.down {
                    self.repeat.set(0);
                    self.repeat_clock.set(Default::default());
                }
                *self.pressed.borrow_mut() = pressed;
                UiResponse::Consumed
            }
            UiEvent::Release {
                button: PointerButton::Left,
                ..
            } => {
                let was = std::mem::take(&mut *self.pressed.borrow_mut());
                if !inside {
                    return UiResponse::Ignored;
                }
                if was.up && up.contains(p) {
                    self.step(false, &g);
                } else if was.down && down.contains(p) {
                    self.step(true, &g);
                } else if item.is_some() && was.item == item {
                    // SetUIState(0x18, off).
                    self.sh.borrow_mut().outputs.push(PanelOutput::SetUi {
                        ui: UI_MESSAGE_LOG,
                        mode: 1,
                        jump: false,
                    });
                }
                UiResponse::Consumed
            }
            _ if inside => UiResponse::Consumed,
            _ => UiResponse::Ignored,
        }
    }
}
