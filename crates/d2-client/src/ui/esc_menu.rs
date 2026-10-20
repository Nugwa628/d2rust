//! The in-game menu (ui state 9, the Esc menu) in play: a modal box with
//! "Options", "Save and Exit Game" and "Return to Game".
//!
//! Esc (command 56) closes the menu when it is open, else closes the open
//! panels, else opens it; state 9 has a gate (no player, no menu; a dead
//! player respawns instead); the world takes no click while it is open.
//!
//! The tree, rows, input and settings are `options_menu`; this panel
//! draws it and routes events. The DC6 art is `esc_art`; a row without
//! art (Window Mode) is Font16 English text with a thin bar and gold knob
//! stand-in. Esc closes the whole menu (`OriginalUi::game_menu_key`).

use super::hud::{FILL_FILE, FILL_H, FILL_W};
use super::options_menu::{value_x, Kind, MenuEvent, OptionsMenu, VALUE_BLOCK};
use super::{left, SharedRef};
use crate::ui::draw::{ImageRef, ImageRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::{utf16, PanelOutput};
use crate::ui::text::TextOpts;
use crate::ui::ui_ids::UiElement;

/// The panel's id: the UI state number.
pub const ESC_PANEL: PanelId = PanelId(9);

/// The fill file's frames (made by `hud::fill_frames`): gold, white, dark.
const GOLD: u32 = 1;
const WHITE: u32 = 3;
pub const DARK: u32 = 4;

/// Menu keys as UI `Char` events (the menu's key handlers, keyed by VK
/// code): private-use code units, D2RUST's own.
pub const CHAR_ENTER: u16 = 0x0D;
pub const CHAR_LEFT: u16 = 0xF025;
pub const CHAR_UP: u16 = 0xF026;
pub const CHAR_RIGHT: u16 = 0xF027;
pub const CHAR_DOWN: u16 = 0xF028;

/// Font16, and the colors of a row at rest, selected and disabled.
const FONT: u16 = 1;
const COLOR_REST: u16 = 0;
const COLOR_SELECTED: u16 = 3;
const COLOR_DISABLED: u16 = 5;

/// The menu's own state.
#[derive(Clone, Debug, Default)]
pub struct EscState {
    /// "Save and Exit Game" was chosen and the host has not read it yet.
    pub exit_requested: bool,
    /// "Configure Controls" was chosen and the host has not read it yet.
    pub controls_requested: bool,
    /// The Configure Controls screen, while it is open over the menu
    /// (`controls_host`).
    pub controls: Option<super::controls_host::ControlsHost>,
    /// Bindings accepted on that screen, until the host reads them.
    pub accepted: Option<crate::controls::Bindings>,
    /// The menu tree and the settings it edits (`options_menu`).
    pub menu: OptionsMenu,
}

impl EscState {
    /// Leave the Controls screen for the Options menu (Previous selected).
    pub fn close_controls(&mut self, f: super::controls_host::Finished) {
        use super::controls_host::Finished;
        self.controls = None;
        self.menu.return_from_controls();
        if let Finished::Accept(b) = f {
            self.accepted = Some(b);
        }
    }
}

/// The colour of the Video page's info lines (gold), set apart from the
/// rows' white.
const COLOR_INFO: u16 = 4;

/// The menu draws over the world with no backdrop,
/// so a dark strip behind the rows keeps the English stand-in text readable.
pub(crate) fn push_fill(out: &mut dyn UiDrawSink, file: u32, frame: u32, r: Rect) {
    let (w, h) = (FILL_W as i32, FILL_H as i32);
    let mut y = r.y;
    while y < r.y + i32::from(r.h) {
        let mut x = r.x;
        while x < r.x + i32::from(r.w) {
            let cw = (r.x + i32::from(r.w) - x).min(w);
            let ch = (r.y + i32::from(r.h) - y).min(h);
            out.push(UiDraw::Image(ImageRequest {
                image: ImageRef { file, frame },
                at: Point::new(x, y),
                clip: Rect::new(x, y, cw as u16, ch as u16),
                look: crate::ui::CelLook::PLAIN,
                call: crate::ui::draw::CelCall::Draw,
            }));
            x += w;
        }
        y += h;
    }
}

fn text(
    out: &mut dyn UiDrawSink,
    clip: Rect,
    s: &str,
    at: Point,
    color: u16,
    centered_in: Option<i32>,
) {
    out.push(UiDraw::Text(TextRequest {
        text: utf16(s),
        at,
        style: TextStyle { font: FONT, color },
        opts: TextOpts::Draw {
            centered: centered_in.is_some(),
            block_w: centered_in,
            mode: 5,
        },
        clip,
    }));
}

/// The adapter (installed last: top-most, so it takes every click).
pub struct EscMenuUi {
    pub(super) sh: SharedRef,
    /// The pentagram counter, advanced by the draws ([`PentClock`]).
    pub(super) pent: std::cell::Cell<super::esc_art::PentClock>,
}

impl Panel for EscMenuUi {
    fn id(&self) -> PanelId {
        ESC_PANEL
    }

    // Modal: the whole screen.
    fn rect(&self) -> Rect {
        self.sh.borrow().config.screen.frame_rect()
    }

    /// The UI pass draws the menu first, before the inventory family and
    /// the control panel (`options` … `pentspin` under `800ctrlpnl7`).
    fn draw_first(&self) -> bool {
        true
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        {
            let mut sh = self.sh.borrow_mut();
            let border = sh.tables.files.id(crate::ui::box_border::BOXPIECES_FILE);
            let slider = sh.tables.files.id(super::msg_log_ui::TEXTSLID_FILE);
            let sh = &mut *sh;
            if let Some(c) = sh.esc.controls.as_mut() {
                let fonts = sh
                    .fonts
                    .as_ref()
                    .map(|f| f as &dyn crate::ui::panels::TextMeasure);
                c.draw(ctx, (border, slider), fonts, out);
                return;
            }
        }
        let sh = self.sh.borrow();
        let m = &sh.esc.menu;
        let file = sh.tables.files.id(FILL_FILE);
        let (half, clip) = (m.half(), m.screen.rect());
        let width = |s: &str| {
            sh.fonts
                .as_ref()
                .and_then(|f| crate::ui::panels::TextMeasure::width(f, FONT, &utf16(s)))
        };
        // The info lines draw directly on the menu, like the other rows.
        let info = m.info_lines();
        if !info.is_empty() {
            for (k, line) in info.iter().enumerate() {
                let yb = m.info_baseline(k);
                text(
                    out,
                    clip,
                    line,
                    Point::new(0, yb - 12),
                    COLOR_INFO,
                    Some(m.screen.w),
                );
            }
        }
        for (i, def) in m.rows().iter().enumerate() {
            if super::esc_art::draw_row(&sh.tables.files, m, i, out) {
                continue;
            }
            let yb = m.baseline(i);
            let color = if !m.enabled(i) && def.kind != Kind::Title {
                COLOR_DISABLED
            } else if i == m.selected {
                COLOR_SELECTED
            } else {
                COLOR_REST
            };
            match def.kind {
                Kind::Title | Kind::Action => {
                    text(
                        out,
                        clip,
                        def.label,
                        Point::new(0, yb - 12),
                        color,
                        Some(m.screen.w),
                    );
                }
                Kind::Choice(_) => {
                    text(
                        out,
                        clip,
                        &m.label_text(i),
                        Point::new(half - 230, yb - 12),
                        color,
                        None,
                    );
                    let v = m.value_text(i);
                    // A value that has no effect (a frame limit above the
                    // refresh rate with VSync on) is shown greyed.
                    let value_color = if m.greyed(i, m.value(i)) {
                        COLOR_DISABLED
                    } else {
                        color
                    };
                    // Centred in the value column, or ending at its right
                    // edge when wider (`options_menu::value_x`).
                    let (x, block) = match width(&v) {
                        Some(w) => (value_x(half, w), None),
                        None => (half + 230 - VALUE_BLOCK, Some(VALUE_BLOCK)),
                    };
                    text(out, clip, &v, Point::new(x, yb - 12), value_color, block);
                }
                Kind::Slider { .. } => {
                    text(
                        out,
                        clip,
                        def.label,
                        Point::new(half - 230, yb - 12),
                        color,
                        None,
                    );
                    if let Some(file) = file {
                        let y = m.slider_y(i);
                        // Track from h − 60 to h + 230, skull at h − 60 + t.
                        push_fill(out, file, WHITE, Rect::new(half - 60, y - 14, 290, 2));
                        push_fill(
                            out,
                            file,
                            GOLD,
                            Rect::new(half - 60 + m.slider_t(i), y - 20, 12, 14),
                        );
                    }
                }
            }
        }
        let mut clock = self.pent.get();
        // Live play steps on the wall clock, which runs on through
        // the pause; a check run on the server tick (the pinned clock).
        // A fixed display clock (`play --display-clock`) stands in for the
        // wall clock, so shots of the open menu repeat.
        let now = sh
            .menu_wall_ms
            .get()
            .map(|wall| crate::world_view::display_clock::now_ms().unwrap_or(wall))
            .unwrap_or_else(|| {
                super::esc_art::PentClock::ms_of_tick(ctx.world.server_ticks, ctx.world.tick_rate)
            });
        let f = clock.draw(now);
        self.pent.set(clock);
        super::esc_art::draw_pents(&sh.tables.files, m, f, out);
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    /// The menu's rows, `options.<page>.<row>` as on the front end's
    /// options screen ([`OptionsMenu::ui_elements`]), or Configure Controls.
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<UiElement> {
        let sh = self.sh.borrow();
        if let Some(c) = &sh.esc.controls {
            return c.ui_elements();
        }
        sh.esc.menu.ui_elements()
    }

    fn event(&mut self, e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        let mut sh = self.sh.borrow_mut();
        if let Some(c) = sh.esc.controls.as_mut() {
            let done = match e {
                UiEvent::CursorMoved(p) => {
                    c.moved(p);
                    None
                }
                UiEvent::Wheel { steps, .. } => {
                    c.wheel(steps);
                    None
                }
                _ => match left(e) {
                    Some((true, at)) => c.press(at),
                    _ => None,
                },
            };
            if let Some(f) = done {
                sh.esc.close_controls(f);
            }
            return UiResponse::Consumed;
        }
        let m = &mut sh.esc.menu;
        let mut consumed = false;
        match e {
            UiEvent::CursorMoved(p) => m.moved(p),
            UiEvent::Char(c) => {
                consumed = true;
                match c {
                    CHAR_ENTER => m.key_enter(),
                    CHAR_LEFT => m.key_left(),
                    CHAR_UP => m.key_up(),
                    CHAR_RIGHT => m.key_right(),
                    CHAR_DOWN => m.key_down(),
                    _ => consumed = false,
                }
            }
            _ => {
                if let Some((down, at)) = left(e) {
                    if down {
                        m.press(at);
                    } else {
                        m.release(at);
                    }
                }
            }
        }
        for ev in m.take_events() {
            match ev {
                // Sound 1 at a choice / slider / arrow change, 2 at an
                // action entry.
                MenuEvent::CursorPass => sh.outputs.push(PanelOutput::Sound(1)),
                MenuEvent::CursorSelect => sh.outputs.push(PanelOutput::Sound(2)),
                MenuEvent::SaveAndExit => sh.esc.exit_requested = true,
                MenuEvent::Close => sh.outputs.push(PanelOutput::SetUi {
                    ui: 9,
                    mode: 1,
                    jump: false,
                }),
                MenuEvent::ConfigureControls => sh.esc.controls_requested = true,
            }
        }
        match e {
            UiEvent::Press { .. } | UiEvent::Release { .. } => UiResponse::Consumed,
            _ if consumed => UiResponse::Consumed,
            _ => UiResponse::Ignored,
        }
    }
}
