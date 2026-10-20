//! CONFIGURE CONTROLS over the game: the Esc menu's Options → Configure
//! Controls row opens the [`ConfigureControls`] model (the one the front
//! end hosts, `q-menu-controls`) while ui 9 stays open. The Esc menu panel
//! draws it instead of the menu, keys reach it raw
//! ([`super::OriginalUi::controls_key`]) and pointer events by the
//! configure-controls geometry. Cancel and Accept return to the Options menu with Previous
//! Menu selected; Accept writes `controls.toml` and hands the new play
//! bindings to the host ([`super::OriginalUi::take_accepted_bindings`]).
//!
//! The panel draws as the game does: DrawBox fills, the
//! boxpieces borders, textslid scroll art and texts at the model's pen y.
//!
//! D2RUST's own, unverified: English stand-ins for strings the table lookup
//! does not give, clicks act on press. The error line and the edit cell's
//! blink run on the UI clock ([`super::ui_clock`]): the wall clock in live
//! play, 40 ms per bridge frame in check runs; keys are stamped with the
//! same clock.

use std::path::PathBuf;

use crate::controls::keymap::action_of_cmd;
use crate::controls::Bindings;
use crate::ui::box_border::push_border;
use crate::ui::draw::{
    CelCall, ImageRef, ImageRequest, RectRequest, TextRequest, TextStyle, UiDraw, UiDrawSink,
};
use crate::ui::front_end::screens::controls::{
    load_table, save_table, table_to_bindings, CfgDraw, ConfigureControls, Done, BTN_H, BTN_HALF,
    BTN_TEXT_BLOCK, C, FONT, M, T, VISIBLE, W,
};
use crate::ui::geom::Point;
use crate::ui::layout::Screen;
use crate::ui::panel::UiCtx;
use crate::ui::panels::{utf16, TextMeasure};
use crate::ui::text::TextOpts;
use crate::ui::CelLook;

/// The open screen.
#[derive(Clone, Debug)]
pub struct ControlsHost {
    model: ConfigureControls,
    path: Option<PathBuf>,
    pointer: Option<Point>,
    /// The UI's live wall clock ([`super::OriginalUi::set_menu_wall_ms`]).
    wall_ms: Option<u64>,
    /// The bridge frame of the last draw (the clock without a wall clock).
    frames: u64,
}

/// What a key or click finished.
#[derive(Debug)]
pub enum Finished {
    Cancel,
    Accept(Bindings),
}

fn cell_rect(r: i32, col: i32) -> crate::ui::geom::Rect {
    let (x, w) = if col == 1 {
        (M + 18, 2 * C)
    } else {
        (M + 18 + 2 * C, C)
    };
    crate::ui::geom::Rect::new(x, T + 41 + 18 * r, w as u16, 18)
}
fn button_rect(i: usize) -> crate::ui::geom::Rect {
    let c = M + 206 * i as i32 + 103;
    crate::ui::geom::Rect::new(
        c - BTN_HALF[i],
        T + 329,
        (2 * BTN_HALF[i] + 1) as u16,
        BTN_H as u16,
    )
}

fn contains(x: i32, y: i32, w: i32, h: i32, p: Point) -> bool {
    (x..x + w).contains(&p.x) && (y..y + h).contains(&p.y)
}

fn stand_in(id: u32) -> &'static str {
    match id {
        3921 => "Function",
        3922 => "Key / Button One",
        3923 => "Key / Button Two",
        3762 => "None",
        3972 => "Default",
        3973 => "Accept",
        3974 => "Cancel",
        3978 => "Can't assign the mouse wheel here",
        3979 => "Can't assign this key",
        _ => "",
    }
}

impl ControlsHost {
    pub fn open(expansion: bool, path: Option<PathBuf>) -> Self {
        let table = load_table(path.as_deref());
        Self {
            model: ConfigureControls::open(expansion, table),
            path,
            pointer: None,
            wall_ms: None,
            frames: 0,
        }
    }

    /// The UI's live wall clock, `None` in check runs.
    pub fn set_wall_ms(&mut self, wall: Option<u64>) {
        self.wall_ms = wall;
    }

    /// The screen's clock: the UI clock at the last draw's frame.
    pub fn now_ms(&self) -> u64 {
        crate::ui::ui_clock::ui_ms(self.wall_ms, self.frames)
    }

    pub fn model(&self) -> &ConfigureControls {
        &self.model
    }

    fn finish(&mut self, d: Done) -> Option<Finished> {
        match d {
            Done::Stay => None,
            Done::Cancel => Some(Finished::Cancel),
            Done::Accept => {
                if let Some(p) = &self.path {
                    // D2RUST's own: a failed write is logged, the screen closes.
                    if let Err(e) = save_table(self.model.table(), p) {
                        eprintln!("controls: {e}");
                    }
                }
                Some(Finished::Accept(table_to_bindings(self.model.table())))
            }
        }
    }

    /// A key went down (`vk`: Windows virtual key; Esc is 27).
    pub fn key(&mut self, vk: u16, now_ms: u64) -> Option<Finished> {
        let was = self.model.editing();
        let d = self.model.key_down(vk, false, now_ms);
        // No key-up event reaches here: the release of the Enter that began
        // editing is swallowed, as the front-end screen does.
        if !was && self.model.editing() {
            self.model.key_up(vk, now_ms);
        }
        self.finish(d)
    }

    pub fn moved(&mut self, p: Point) {
        self.pointer = Some(p);
    }

    pub fn wheel(&mut self, steps: i32) {
        self.model.wheel(steps * 120);
    }

    pub fn ui_elements(&self) -> Vec<crate::ui::ui_ids::UiElement> {
        use crate::ui::ui_ids::UiElement;
        let mut out = Vec::new();
        for r in 0..VISIBLE {
            let Some(row) = self.model.rows().get(self.model.top() + r) else {
                break;
            };
            if row.cmd == crate::controls::original::SEPARATOR {
                continue;
            }
            for col in [1, 0] {
                out.push(UiElement::new(
                    format!("controls.binding({},{col})", row.cmd),
                    cell_rect(r as i32, col),
                ));
            }
        }
        for (i, name) in ["cancel", "default", "accept"].into_iter().enumerate() {
            let mut e = UiElement::new(format!("controls.{name}"), button_rect(i));
            e.idle = self.model.idle(name);
            out.push(e);
        }
        for (name, y) in [("scroll_up", T + 47), ("scroll_down", T + 305)] {
            let mut e = UiElement::new(
                format!("controls.{name}"),
                crate::ui::geom::Rect::new(W - M - 31, y, 12, 13),
            );
            e.idle = self.model.idle(name);
            out.push(e);
        }
        out
    }

    /// A left press at `p`: key cell, button or scroll arrow.
    pub fn press(&mut self, p: Point) -> Option<Finished> {
        self.pointer = Some(p);
        for r in 0..VISIBLE as i32 {
            for col in [1, 0] {
                if cell_rect(r, col).contains(p) {
                    self.model.click_cell(r as usize, col);
                    return None;
                }
            }
        }
        for i in 0..3 {
            if button_rect(i as usize).contains(p) {
                let d = match i {
                    0 => self.model.cancel(),
                    1 => {
                        self.model.default_all();
                        Done::Stay
                    }
                    _ => self.model.accept(),
                };
                return self.finish(d);
            }
        }
        for (down, y) in [(false, T + 47), (true, T + 305)] {
            if contains(W - M - 31, y, 12, 13, p) {
                self.model.scroll(down);
            }
        }
        None
    }

    /// The configure-controls draw list. `border` / `slider`: the `menu\boxpieces` /
    /// `menu\textslid` file ids (`None`: not loaded, those cels are left
    /// out). `fonts` measures the button labels (`None`: they are
    /// centred in a fixed block instead).
    pub fn draw(
        &mut self,
        ctx: &UiCtx,
        art: (Option<u32>, Option<u32>),
        fonts: Option<&dyn TextMeasure>,
        out: &mut dyn UiDrawSink,
    ) {
        let (border, slider) = art;
        self.frames = ctx.tick;
        let now = self.now_ms();
        let pointer = self.pointer.map(|p| (p.x, p.y));
        let rows = self.model.rows().to_vec();
        let draws = self.model.draw_list(now, pointer);
        let clip = Screen::play().rect();
        let label = |id: u32, lit: &str| -> String {
            if id == 0 {
                return lit.to_string();
            }
            if let Some(t) = u16::try_from(id).ok().and_then(|i| ctx.strings.get_id(i)) {
                return String::from_utf16_lossy(t);
            }
            if let Some(r) = rows.iter().find(|r| r.string_id == id) {
                if let Some(a) = action_of_cmd(r.cmd) {
                    return a.name().replace('_', " ");
                }
            }
            stand_in(id).to_string()
        };
        let mut texts = Vec::new();
        for d in draws {
            match d {
                // (`DrawBox`, the game's list's
                // box call).
                CfgDraw::Rect { x, y, w, h } => {
                    flush_texts(&mut texts, fonts, out);
                    out.push(UiDraw::Rect(RectRequest::sized(x, y, w, h, 0, 1)));
                }
                // Over (l, t, r, b), both ends in.
                CfgDraw::Border { x, y, w, h } => {
                    if let Some(file) = border {
                        push_border(file, (x, y, x + w, y + h), clip, out);
                    }
                }
                CfgDraw::Slider { frame, x, y } => {
                    flush_texts(&mut texts, fonts, out);
                    if let Some(file) = slider {
                        out.push(UiDraw::Image(ImageRequest {
                            image: ImageRef { file, frame },
                            at: Point::new(x, y),
                            clip,
                            look: CelLook::PLAIN,
                            call: CelCall::Draw,
                        }));
                    }
                }
                CfgDraw::Text {
                    string_id,
                    text,
                    x,
                    y,
                    color,
                } => texts.push((label(string_id, &text), x, y, color)),
            }
        }
        flush_texts(&mut texts, fonts, out);
    }
}

/// Draws the queued texts in order (the list's texts stay between its
/// rectangles and cels, as the game draws them). A button label is drawn
/// from x = c − w/2 (c its centre, w its width A, halved toward zero;
/// the button keeps c − w/2 − 10 as its left and
/// draws the text 10 to its right).
fn flush_texts(
    texts: &mut Vec<(String, i32, i32, u8)>,
    fonts: Option<&dyn TextMeasure>,
    out: &mut dyn UiDrawSink,
) {
    for (t, x, y, color) in texts.drain(..) {
        let text = utf16(&t);
        let button = y == T + 351;
        let width = fonts.filter(|_| button).and_then(|f| f.width(FONT, &text));
        // The message passes its centre as x (as do the buttons when no
        // font is bound).
        let centred = width.is_none() && (button || y == T + 399);
        let (at_x, block) = match width {
            Some(w) => (x - w / 2, None),
            None if centred => (x - BTN_TEXT_BLOCK / 2, Some(BTN_TEXT_BLOCK)),
            None => (x, None),
        };
        out.push(UiDraw::Text(TextRequest {
            text,
            at: Point::new(at_x, y),
            style: TextStyle {
                font: FONT,
                color: u16::from(color),
            },
            opts: TextOpts::Draw {
                centered: centred,
                block_w: block,
                mode: 5,
            },
            clip: Screen::play().rect(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::ClientWorld;
    use crate::ui::panel::NoStrings;
    use crate::ui::ui_clock::{draw_ms, period_ms, RATES};

    fn texts(c: &mut ControlsHost, frames: u64) -> Vec<String> {
        let w = ClientWorld::default();
        let ctx = UiCtx {
            tick: frames,
            world: &w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        c.draw(&ctx, (None, None), None, &mut out);
        out.into_iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
                _ => None,
            })
            .collect()
    }

    // The edit cell blinks on 40 ms steps of wall time (hidden while
    // step & 15 <= 4: 200 ms of each 640 ms) and a refused key's line
    // shows 2 s, at any draw rate. A per-draw counter blinked 5.8 times
    // too fast at 144 draws a second; the line was timed by the frame
    // count against a key stamped by another clock.
    #[test]
    fn the_blink_and_the_error_line_keep_wall_time_at_25_60_and_144_draws_a_second() {
        for fps in RATES {
            let mut c = ControlsHost::open(true, None);
            c.set_wall_ms(Some(draw_ms(fps, 0)));
            let shown = texts(&mut c, 0).len();
            let now = c.now_ms();
            assert!(c.key(13, now).is_none());
            assert!(c.model().editing());
            let mut i = 1;
            while draw_ms(fps, i) - draw_ms(fps, 0) < 1280 {
                c.set_wall_ms(Some(draw_ms(fps, i)));
                let hidden = texts(&mut c, i).len() == shown - 1;
                let step = (draw_ms(fps, i) - draw_ms(fps, 0)) / 40;
                assert_eq!(hidden, step & 15 <= 4, "{fps}/s draw {i}");
                i += 1;
            }
            // Up is not a key a command takes: the error line.
            let line = "Can't assign this key".to_string();
            let start = draw_ms(fps, i);
            c.set_wall_ms(Some(start));
            texts(&mut c, i);
            let now = c.now_ms();
            c.key(0x26, now);
            loop {
                i += 1;
                c.set_wall_ms(Some(draw_ms(fps, i)));
                if !texts(&mut c, i).contains(&line) {
                    break;
                }
                assert!(i < 100_000);
            }
            let ms = draw_ms(fps, i) - start;
            assert!(
                (2_000..=2_000 + period_ms(fps)).contains(&ms),
                "{fps}/s: {ms} ms"
            );
        }
    }
}
