//! The stash (ui 0x19) and the Horadric Cube (ui 0x1A): panel
//! art, the stash gold-cap line, close / transmute buttons, their C→S
//! messages, the cube-gone rule and the transmute animation step.
//!
//! The grids (stash page 4, cube page 3) and the right-half inventory are
//! the inventory grid code, not here.
//!
//! Not implemented yet / inputs:
//! - GoldMax is drawn in the "current font": the font id is
//!   not known, so it is an input of [`StashPanel::draw`].
//! - The press / release hit rectangle of the stash close
//!   button is not known (only the hover rectangle); the caller decides
//!   a release on the button and calls [`stash_close`].
//! - Whether a close-button release sends 0x4F 0x12 once (the
//!   hook's send) or also from the handler: [`stash_close`] sends it once.
//! - Cube button hit rectangles and hover rectangles
//!   (only the tool-tip y `H + sy − 100` / `H + sy − 223` is given), and
//!   whether the cube close button also calls `SetUIState(0x1A, off)`:
//!   [`cube_close`] sends 0x4F 0x17 only.
//! - the frame is exposed as a query ([`HoradricAnim::frame`],
//!   [`horadric_pos`]); the cube adapter draws it in mode 3. Whether
//!   frame 30 is drawn once before the stop, and how the "last step" tick
//!   is initialised when the animation starts, are not stated: here the
//!   caller passes the start tick, and the step that reaches 30 stops the
//!   animation (frames 0–29 shown).

use d2_proto::client::ClickButton;

use super::{emit_static_draws, no_extra, text, PanelEnv, PanelOutput, PanelTables};
use crate::ui::draw::UiDrawSink;
use crate::ui::geom::Point;
use crate::ui::layout::{PanelKey, RowKind, Screen};
use crate::ui::panel::{ClientIntent, StringLookup};

/// Stash ui id.
pub const UI_STASH: u8 = 0x19;
/// Cube ui id.
pub const UI_CUBE: u8 = 0x1A;
/// "Gold Max: %d".
pub const STR_GOLD_MAX: u16 = 4051;
/// "Transmute" tool tip.
pub const STR_TRANSMUTE: u16 = 3341;
/// Inventory mode while the stash is open.
pub const INV_MODE_STASH: u8 = 0x0C;
/// Inventory mode while the cube is open.
pub const INV_MODE_CUBE: u8 = 0x0E;
/// Transmute animation length: it stops at frame 30.
pub const HORADRIC_END: u32 = 30;
/// Step period: a step when more than 70 ms passed.
pub const HORADRIC_STEP_MS: u32 = 70;

fn click_button(button: u16) -> PanelOutput {
    PanelOutput::Intent(ClientIntent::from_message(&ClickButton {
        button,
        p1: 0,
        p2: 0,
    }))
}

/// Replaces the first `%d` of `fmt` with `v` in decimal.
fn format_d(fmt: &[u16], v: i32) -> Vec<u16> {
    let pct = u16::from(b'%');
    let d = u16::from(b'd');
    match fmt.windows(2).position(|w| w == [pct, d]) {
        Some(i) => {
            let mut out = fmt[..i].to_vec();
            out.extend(v.to_string().encode_utf16());
            out.extend_from_slice(&fmt[i + 2..]);
            out
        }
        None => fmt.to_vec(),
    }
}

// ---------------------------------------------------------------------
// Stash
// ---------------------------------------------------------------------

/// The stash panel's inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StashPanel {
    /// Close button pressed.
    pub close_pressed: bool,
}

impl StashPanel {
    /// Art quads (`TradeStash` in an expansion game, `bank` otherwise), the
    /// GoldMax line with `gold_cap` in color 0 and `font` (the current font,
    /// module Open), then the close button, in `panel-layout.tsv` order.
    pub fn draw(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        strings: &dyn StringLookup,
        gold_cap: i32,
        font: u16,
        out: &mut dyn UiDrawSink,
    ) {
        let cond = env.cond(self.close_pressed, &no_extra);
        let key = PanelKey::Ui(UI_STASH);
        emit_static_draws(t, key, &cond, None, &|r| r.item.starts_with("art"), out);
        if let Some(r) = t
            .item(key, "goldmax", RowKind::Text)
            .find(|r| r.applies(&cond))
        {
            if let Some(fmt) = strings.get_id(STR_GOLD_MAX) {
                out.push(text(
                    format_d(fmt, gold_cap),
                    r.x.eval(&env.screen),
                    r.y.eval(&env.screen),
                    font,
                    0,
                ));
            }
        }
        emit_static_draws(t, key, &cond, None, &|r| r.item == "close", out);
    }
}

/// Stash close button draw position (X, Y): (`sx + 272`,
/// `H + sy − 64`) expansion, (`sx + 275`, `H + sy − 65`) classic.
pub fn stash_close_pos(s: &Screen, exp: bool) -> (i32, i32) {
    if exp {
        (s.sx() + 272, s.h + s.sy() - 64)
    } else {
        (s.sx() + 275, s.h + s.sy() - 65)
    }
}

/// The `strClose` hover test: x in [X, X + 40], y in [Y − 35,
/// Y + 5], inclusive.
pub fn stash_close_hover(s: &Screen, exp: bool, p: Point) -> bool {
    let (x, y) = stash_close_pos(s, exp);
    (x..=x + 40).contains(&p.x) && (y - 35..=y + 5).contains(&p.y)
}

/// What closing the stash does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashClose {
    /// `SetUIState(0x19, off, 0)` then C→S 0x4F button 0x12, p1 = p2 = 0.
    pub outputs: Vec<PanelOutput>,
    /// The inventory mode set between the two (0).
    pub inventory_mode: u8,
}

/// Closing the stash (close button release or the
/// close hook).
pub fn stash_close() -> StashClose {
    StashClose {
        outputs: vec![
            PanelOutput::SetUi {
                ui: UI_STASH,
                mode: 1,
                jump: false,
            },
            click_button(0x12),
        ],
        inventory_mode: 0,
    }
}

// ---------------------------------------------------------------------
// Horadric Cube
// ---------------------------------------------------------------------

/// The cube panel's inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CubePanel {
    /// Close button pressed (frame 10 + flag).
    pub close_pressed: bool,
    /// Transmute button frame 0 / 1.
    pub transmute_pressed: bool,
}

impl CubePanel {
    /// When the local player is missing or dead
    /// (`player_ok` false), returns `SetUIState(0x1A,
    /// off)` and C→S 0x4F 0x17 and draws nothing; else draws the art
    /// quads, the close button and the transmute button and returns
    /// nothing. (No `jump`: the cube is a full-kind state, for
    /// which the cursor never jumps, so `false` is exact.)
    pub fn draw(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        player_ok: bool,
        out: &mut dyn UiDrawSink,
    ) -> Vec<PanelOutput> {
        if !player_ok {
            // The close hook sends the latched 0x17,
            // then the close button's release sends it again: two messages.
            return vec![
                PanelOutput::SetUi {
                    ui: UI_CUBE,
                    mode: 1,
                    jump: false,
                },
                click_button(0x17),
                click_button(0x17),
            ];
        }
        self.draw_art(t, env, out);
        self.draw_buttons(t, env, out);
        Vec::new()
    }

    /// The art quads alone, before the cube grid.
    pub fn draw_art(&self, t: &PanelTables, env: &PanelEnv, out: &mut dyn UiDrawSink) {
        let c = env.cond(false, &no_extra);
        emit_static_draws(
            t,
            PanelKey::Ui(UI_CUBE),
            &c,
            None,
            &|r| r.item.starts_with("art"),
            out,
        );
    }

    /// After the cube grid, the close button then the transmute
    /// button (`a1-panel-cube` rows 10–11).
    pub fn draw_buttons(&self, t: &PanelTables, env: &PanelEnv, out: &mut dyn UiDrawSink) {
        let key = PanelKey::Ui(UI_CUBE);
        let c = env.cond(self.close_pressed, &no_extra);
        emit_static_draws(t, key, &c, None, &|r| r.item == "close", out);
        let c = env.cond(self.transmute_pressed, &no_extra);
        emit_static_draws(t, key, &c, None, &|r| r.item == "transmute", out);
    }
}

/// Transmute button release: C→S 0x4F button 0x18.
pub fn cube_transmute() -> Vec<PanelOutput> {
    vec![click_button(0x18)]
}

/// Cube close: C→S 0x4F button 0x17.
pub fn cube_close() -> Vec<PanelOutput> {
    vec![click_button(0x17)]
}

/// The transmute animation, stepped by the caller with a
/// millisecond tick (`GetTickCount`); wall-clock, not game time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HoradricAnim {
    running: bool,
    n: u32,
    last: u32,
}

impl HoradricAnim {
    /// Starts at frame 0 with `now` as the last step tick (module Open).
    pub fn start(&mut self, now: u32) {
        *self = Self {
            running: true,
            n: 0,
            last: now,
        };
    }

    /// `n` += 1 when more than 70 ms passed since the last step; at
    /// `n` = 30 the animation stops.
    pub fn step(&mut self, now: u32) {
        if !self.running {
            return;
        }
        if now.wrapping_sub(self.last) > HORADRIC_STEP_MS {
            self.n += 1;
            self.last = now;
            if self.n >= HORADRIC_END {
                self.running = false;
            }
        }
    }

    /// The `menu\horadric` frame to draw (draw mode 3), or `None`.
    pub fn frame(&self) -> Option<u32> {
        self.running.then_some(self.n)
    }

    /// The cube grid (page 3) is not drawn while the animation runs with
    /// `n` < 14; from `n` = 14 on, and
    /// when no animation runs, it is drawn.
    pub fn grid_visible(&self) -> bool {
        !self.running || self.n >= 14
    }
}

/// Where the animation frame is drawn: (W / 2, H / 2 − 1) at 4:3, the
/// cube side's right edge (`Screen::left_end`), so in a wider frame it stays
/// beside the cube panel.
pub fn horadric_pos(s: &Screen) -> (i32, i32) {
    (s.left_end(), s.h / 2 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::{ImageRef, UiDraw};

    struct Strs;
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            const G: &[u16] = &[71, 111, 108, 100, 32, 77, 97, 120, 58, 32, 37, 100];
            (id == STR_GOLD_MAX).then_some(G)
        }
    }

    #[derive(Debug, PartialEq)]
    enum D {
        I(String, u32, i32, i32),
        T(String, i32, i32, u16, u16),
    }

    fn conv(t: &PanelTables, out: Vec<UiDraw>) -> Vec<D> {
        out.into_iter()
            .map(|d| match d {
                UiDraw::Image(i) => {
                    let ImageRef { file, frame } = i.image;
                    D::I(t.files.name(file).unwrap().into(), frame, i.at.x, i.at.y)
                }
                UiDraw::Text(x) => D::T(
                    String::from_utf16(&x.text).unwrap(),
                    x.at.x,
                    x.at.y,
                    x.style.font,
                    x.style.color,
                ),
                UiDraw::Rect(r) => panic!("rectangle {r:?}"),
                UiDraw::Line(_) => panic!("line"),
            })
            .collect()
    }

    fn env(screen: Screen, exp: bool) -> PanelEnv {
        PanelEnv {
            screen,
            open_mode: 3,
            exp,
        }
    }

    fn i(f: &str, fr: u32, x: i32, y: i32) -> D {
        D::I(f.into(), fr, x, y)
    }

    // Partial: the stash art; the right-half inventory is the grid code's.
    #[test]
    fn stash_draw_exp_and_classic() {
        let t = PanelTables::load().unwrap();
        let mut out = Vec::new();
        StashPanel {
            close_pressed: false,
        }
        .draw(&t, &env(Screen::R800, true), &Strs, 2_500_000, 6, &mut out);
        assert_eq!(
            conv(&t, out),
            vec![
                i("panel\\tradestash", 0, 80, 316),
                i("panel\\tradestash", 1, 336, 316),
                i("panel\\tradestash", 2, 80, 492),
                i("panel\\tradestash", 3, 336, 492),
                D::T("Gold Max: 2500000".into(), 158, 125, 6, 0),
                i("panel\\buysellbtn", 10, 352, 476),
            ]
        );
        let mut out = Vec::new();
        StashPanel {
            close_pressed: true,
        }
        .draw(&t, &env(Screen::R640, false), &Strs, 50_000, 3, &mut out);
        assert_eq!(
            conv(&t, out),
            vec![
                i("panel\\bank", 0, 0, 256),
                i("panel\\bank", 1, 256, 256),
                i("panel\\bank", 2, 0, 432),
                i("panel\\bank", 3, 256, 432),
                D::T("Gold Max: 50000".into(), 78, 261, 3, 0),
                i("panel\\buysellbtn", 11, 275, 415),
            ]
        );
    }

    #[test]
    fn stash_close_button_pos_and_hover() {
        assert_eq!(stash_close_pos(&Screen::R800, true), (352, 476));
        assert_eq!(stash_close_pos(&Screen::R640, false), (275, 415));
        let s = Screen::R640;
        // Expansion at 640: X 272, Y 416 → x 272–312, y 381–421.
        for (p, inside) in [
            ((272, 381), true),
            ((312, 421), true),
            ((271, 400), false),
            ((313, 400), false),
            ((290, 380), false),
            ((290, 422), false),
        ] {
            assert_eq!(
                stash_close_hover(&s, true, Point::new(p.0, p.1)),
                inside,
                "{p:?}"
            );
        }
    }

    #[test]
    fn stash_close_outputs() {
        let c = stash_close();
        assert_eq!(c.inventory_mode, 0);
        assert_eq!(
            c.outputs,
            vec![
                PanelOutput::SetUi {
                    ui: 0x19,
                    mode: 1,
                    jump: false
                },
                PanelOutput::Intent(ClientIntent(vec![0x4F, 0x12, 0, 0, 0, 0, 0])),
            ]
        );
    }

    #[test]
    fn dead_or_absent_player_closes_and_draws_nothing() {
        let t = PanelTables::load().unwrap();
        let mut out: Vec<UiDraw> = Vec::new();
        let o = CubePanel::default().draw(&t, &env(Screen::R800, true), false, &mut out);
        assert!(out.is_empty());
        assert_eq!(
            o,
            vec![
                PanelOutput::SetUi {
                    ui: 0x1A,
                    mode: 1,
                    jump: false
                },
                PanelOutput::Intent(ClientIntent(vec![0x4F, 0x17, 0, 0, 0, 0, 0])),
                PanelOutput::Intent(ClientIntent(vec![0x4F, 0x17, 0, 0, 0, 0, 0])),
            ]
        );
    }

    // (art and buttons; the grid is the inventory grid's).
    #[test]
    fn cube_draw_buttons() {
        let t = PanelTables::load().unwrap();
        let mut out = Vec::new();
        let o = CubePanel {
            close_pressed: true,
            transmute_pressed: false,
        }
        .draw(&t, &env(Screen::R800, true), true, &mut out);
        assert!(o.is_empty());
        assert_eq!(
            conv(&t, out),
            vec![
                i("panel\\supertransmogrifier", 0, 80, 316),
                i("panel\\supertransmogrifier", 1, 336, 316),
                i("panel\\supertransmogrifier", 2, 80, 492),
                i("panel\\supertransmogrifier", 3, 336, 492),
                i("panel\\buysellbtn", 11, 355, 475),
                i("panel\\miniconvert", 0, 224, 352),
            ]
        );
        let mut out = Vec::new();
        CubePanel {
            close_pressed: false,
            transmute_pressed: true,
        }
        .draw(&t, &env(Screen::R640, false), true, &mut out);
        let d = conv(&t, out);
        assert_eq!(d[4], i("panel\\buysellbtn", 10, 275, 415));
        assert_eq!(d[5], i("panel\\miniconvert", 1, 144, 292));
    }

    #[test]
    fn cube_messages() {
        assert_eq!(
            cube_transmute(),
            vec![PanelOutput::Intent(ClientIntent(vec![
                0x4F, 0x18, 0, 0, 0, 0, 0
            ]))]
        );
        assert_eq!(
            cube_close(),
            vec![PanelOutput::Intent(ClientIntent(vec![
                0x4F, 0x17, 0, 0, 0, 0, 0
            ]))]
        );
    }

    // Partial: step rule and position; frame-30 draw and start tick open.
    #[test]
    fn horadric_steps_after_more_than_70ms() {
        assert_eq!(horadric_pos(&Screen::R800), (400, 299));
        assert_eq!(horadric_pos(&Screen::R640), (320, 239));
        let mut a = HoradricAnim::default();
        assert_eq!(a.frame(), None);
        a.start(1000);
        assert_eq!(a.frame(), Some(0));
        a.step(1070);
        assert_eq!(a.frame(), Some(0), "exactly 70 ms: no step");
        a.step(1071);
        assert_eq!(a.frame(), Some(1));
        let mut now = 1071;
        for n in 2..HORADRIC_END {
            now += 71;
            a.step(now);
            assert_eq!(a.frame(), Some(n));
        }
        a.step(now + 71);
        assert_eq!(a.frame(), None);
        a.step(now + 1000);
        assert_eq!(a.frame(), None);
    }

    #[test]
    fn format_d_replaces_first() {
        assert_eq!(
            String::from_utf16(&format_d(
                &"a %d b %d".encode_utf16().collect::<Vec<_>>(),
                -5
            ))
            .unwrap(),
            "a -5 b %d"
        );
    }
}
