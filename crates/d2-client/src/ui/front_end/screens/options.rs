//! The main menu's Options screen: the same Options pages the game's Esc
//! menu shows (Sound, Video, Automap, Configure Controls), run by the same
//! engine ([`OptionsMenu`]) and editing the same settings. A change made
//! here is the change the game sees: the host hands the screen the
//! settings in force, saves each change and applies it at once
//! (`app::config::front_settings`).
//!
//! The screen opens on the Video page. "Previous Menu" climbs the same
//! tree as in game (a page, then the Options list); "Previous Menu" on the
//! Options list, and Esc on any page, go back to the main menu.
//!
//! What it looks like, built for reading the rows easily:
//! - the main menu's art is darkened by about three quarters (two
//!   half-black passes over the whole picture; the side fill a wide window
//!   shows is a blur of this picture, so it darkens with it);
//! - the page sits on a panel: a solid black backing behind the rows,
//!   framed by the game's box border (`menu\boxpieces`);
//! - each row is drawn by the in-game menu's own drawing ([`esc_art`]):
//!   label and value images, slider bars and skulls, and the two spinning
//!   pentagrams beside the selected row. D2RUST's own display rows, which
//!   have no art, are drawn as text in the in-game menu's colours.

use crate::ui::box_border::{border_pieces, BOXPIECES_FILE};
use crate::ui::draw::UiDraw;
use crate::ui::front_end::control::{vk, Control, ControlKind};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::{DrawItem, Registry, OPTIONS};
use crate::ui::geom::Point;
use crate::ui::original::esc_art::{self, PentClock};
use crate::ui::original::options_menu::{value_x, Kind, MenuEvent, MenuId, OptionsMenu};
use crate::ui::panels::UiFiles;
use crate::ui::ui_ids::UiElement;

use super::main_menu::{BACKGROUND, BACKGROUND_CLASSIC};

/// Arrow keys (virtual-key codes).
const VK_LEFT: u16 = 37;
const VK_UP: u16 = 38;
const VK_RIGHT: u16 = 39;
const VK_DOWN: u16 = 40;

/// Font16, and the colours of a row at rest, selected and disabled: the
/// Esc menu's.
const FONT: u16 = 1;
const COLOR_REST: i32 = 0;
const COLOR_SELECTED: i32 = 3;
const COLOR_DISABLED: i32 = 5;

/// The colour of the Video page's info lines (gold), set apart from the
/// rows' white.
const COLOR_INFO: i32 = 4;

/// The page the screen opens on.
pub const ENTRY_PAGE: MenuId = MenuId::Video;

/// The backdrop: black blended at half strength (draw mode 1), twice, so
/// about a quarter of the main menu's art shows through.
pub const BACKDROP: [DrawItem; 2] = [DrawItem::Dim { mode: 1 }, DrawItem::Dim { mode: 1 }];

/// The panel's backing: palette index 0 (black), written opaque (mode 5).
pub const PANEL_COLOR: u8 = 0;
const PANEL_MODE: u8 = 5;
/// The panel reaches a little beyond the pentagrams (which end 301 px
/// either side of the centre), and this far above and below the rows.
const PANEL_HALF_W: i32 = 312;
const PANEL_MARGIN_Y: i32 = 10;
/// What a page too tall for the picture keeps free above and below its
/// rows: the panel's margin and its border (whose bottom pieces reach 10
/// pixels under the panel), so the whole frame shows.
pub const PAGE_MARGIN: i32 = 25;

pub struct OptionsScreen {
    pub menu: OptionsMenu,
    /// Configure Controls was chosen: the next entry comes back from it.
    in_controls: bool,
    /// The in-game Options art, by the names the game's menu draws it with.
    files: UiFiles,
    /// The pentagrams' spin, stepped by the screen's draws.
    pent: PentClock,
}

impl Default for OptionsScreen {
    fn default() -> Self {
        let mut files = UiFiles::new(&[]);
        files.extend(esc_art::esc_files());
        let mut menu = OptionsMenu::default();
        menu.margin = PAGE_MARGIN;
        OptionsScreen {
            menu,
            in_controls: false,
            files,
            pent: PentClock::default(),
        }
    }
}

impl OptionsScreen {
    /// What the rows need to know about the machine, from the host.
    fn sync(&mut self, ctx: &FrontCtx) {
        self.menu.sound_device = ctx.settings.sound_device;
        self.menu.monitor = ctx.settings.monitor;
        self.menu.refresh_hz = ctx.settings.refresh_hz;
        self.menu.window = ctx.settings.window;
    }

    /// Copy a change into the shared settings and turn the menu's requests
    /// into a flow trigger.
    fn after_input(&mut self, ctx: &mut FrontCtx) -> Option<Trigger> {
        if let Some(s) = self.menu.take_changed() {
            ctx.settings.current = s;
            ctx.settings.changed = true;
        }
        let mut out = None;
        for ev in self.menu.take_events() {
            if ev == MenuEvent::ConfigureControls {
                self.in_controls = true;
                out = Some(Trigger::ConfigureControls);
            }
        }
        // "Previous Menu" of the Options list leads to the game's own page,
        // which the main menu has no use for: back to the main menu.
        if self.menu.menu == MenuId::Game {
            out = Some(Trigger::Exit);
        }
        out
    }

    /// The panel's rectangle (left, top, right, bottom, inclusive) around
    /// the current page's info lines, rows and pentagrams, kept on the
    /// picture.
    pub fn panel(&self) -> (i32, i32, i32, i32) {
        let m = &self.menu;
        let last = m.rows().len() - 1;
        let half = m.half();
        let top = (m.info_top() - PANEL_MARGIN_Y).max(0);
        let bottom = (m.y_top(last) + m.menu.header().2 + PANEL_MARGIN_Y).min(m.screen.h - 1);
        (half - PANEL_HALF_W, top, half + PANEL_HALF_W - 1, bottom)
    }

    /// The items of the current page at `now_ms`: the darkened backdrop,
    /// the panel, each row as the in-game Options menu draws it (its art,
    /// or text for a row without art), then the pentagrams.
    pub fn items(&mut self, now_ms: u64, adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        let mut out = BACKDROP.to_vec();
        let (l, t, r, b) = self.panel();
        out.push(DrawItem::Shade {
            at: Point::new(l, t),
            w: r - l + 1,
            h: b - t + 1,
            color: PANEL_COLOR,
            mode: PANEL_MODE,
        });
        for (frame, x, y) in border_pieces(l, t, r, b) {
            out.push(DrawItem::UiCel {
                file: BOXPIECES_FILE.to_owned(),
                frame,
                at: Point::new(x, y),
                mode: PANEL_MODE,
            });
        }
        let (m, files) = (&self.menu, &self.files);
        let half = m.half();
        let text = |s: &str, x: i32, y: i32, color: i32| DrawItem::Text {
            string_id: 0,
            text: s.to_string(),
            font: FONT,
            at: Point::new(x, y),
            label: None,
            color,
            boxed: None,
        };
        let width = |s: &str| adv(FONT, &s.encode_utf16().collect::<Vec<_>>());
        // The Video page's info lines, centred on the panel.
        for (k, line) in m.info_lines().iter().enumerate() {
            out.push(text(
                line,
                half - width(line) / 2,
                m.info_baseline(k),
                COLOR_INFO,
            ));
        }
        for (i, def) in m.rows().iter().enumerate() {
            let mut art = Vec::new();
            if esc_art::draw_row(files, m, i, &mut art) {
                out.extend(art.iter().filter_map(|d| ui_item(files, d)));
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
                    out.push(text(def.label, half - width(def.label) / 2, yb, color));
                }
                Kind::Choice(_) => {
                    out.push(text(&m.label_text(i), half - 230, yb, color));
                    let v = m.value(i);
                    let name = m.value_text(i);
                    let name = name.as_ref();
                    // A value that has no effect (a frame limit above the
                    // refresh rate with VSync on) is shown greyed.
                    let value_color = if m.greyed(i, v) {
                        COLOR_DISABLED
                    } else {
                        color
                    };
                    let x = value_x(half, width(name));
                    out.push(text(name, x, yb, value_color));
                }
                Kind::Slider { .. } => {
                    out.push(text(def.label, half - 230, yb, color));
                    let y = m.slider_y(i);
                    // Track from h − 60 to h + 230, knob at h − 60 + t.
                    out.push(DrawItem::Border {
                        at: Point::new(half - 60, y - 14),
                        w: 290,
                        h: 2,
                    });
                    out.push(DrawItem::Border {
                        at: Point::new(half - 60 + m.slider_t(i), y - 20),
                        w: 12,
                        h: 14,
                    });
                }
            }
        }
        let frame = self.pent.draw(now_ms);
        let mut pents = Vec::new();
        esc_art::draw_pents(&self.files, &self.menu, frame, &mut pents);
        out.extend(pents.iter().filter_map(|d| ui_item(&self.files, d)));
        out
    }
}

/// A draw of the in-game menu as a front-end item: an image becomes a
/// [`DrawItem::UiCel`] of its file, a rectangle a [`DrawItem::Shade`].
fn ui_item(files: &UiFiles, d: &UiDraw) -> Option<DrawItem> {
    match d {
        UiDraw::Image(r) => Some(DrawItem::UiCel {
            file: files.name(r.image.file)?.to_owned(),
            frame: r.image.frame,
            at: r.at,
            mode: r.look.mode,
        }),
        UiDraw::Rect(r) => Some(DrawItem::Shade {
            at: Point::new(r.x0, r.y0),
            w: r.x1 - r.x0,
            h: r.y1 - r.y0,
            color: r.color,
            mode: r.mode,
        }),
        UiDraw::Text(_) | UiDraw::Line(_) => None,
    }
}

impl Screen for OptionsScreen {
    fn canvas_changed(&mut self, canvas: super::super::canvas::FrontCanvas) {
        self.menu.screen = crate::ui::layout::Screen {
            h: canvas.height,
            ..crate::ui::layout::Screen::R800
        };
    }

    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        self.sync(ctx);
        self.menu.set_settings(ctx.settings.current);
        // Back from Configure Controls: the Options list with Previous Menu
        // selected, as in the game. A fresh visit opens the Video page.
        if std::mem::take(&mut self.in_controls) {
            self.menu.return_from_controls();
        } else {
            self.menu.open_page(ENTRY_PAGE);
        }
        self.menu.take_events();
        self.after_input(ctx);
        let bg = if ctx.expansion {
            BACKGROUND
        } else {
            BACKGROUND_CLASSIC
        };
        vec![Control::new(ControlKind::Image, 0, 599, 800, 600).with_art(bg)]
    }

    /// Each tick takes the window's facts again, so the info lines and the
    /// Window Size row follow a change and Alt+Enter as they happen.
    fn tick(&mut self, ctx: &mut FrontCtx) -> Option<Trigger> {
        self.sync(ctx);
        None
    }

    fn pointer(&mut self, ctx: &mut FrontCtx, p: Point) {
        self.sync(ctx);
        self.menu.moved(p);
        // A drag changes sliders; the flow never moves on a pointer move.
        let _ = self.after_input(ctx);
    }

    fn mouse(&mut self, ctx: &mut FrontCtx, down: bool, p: Point) -> Option<Trigger> {
        self.sync(ctx);
        if down {
            self.menu.press(p);
        } else {
            self.menu.release(p);
        }
        self.after_input(ctx)
    }

    fn key(&mut self, ctx: &mut FrontCtx, k: u16) -> Option<Trigger> {
        self.sync(ctx);
        match k {
            vk::ESC => return Some(Trigger::Exit),
            vk::ENTER => self.menu.key_enter(),
            VK_LEFT => self.menu.key_left(),
            VK_UP => self.menu.key_up(),
            VK_RIGHT => self.menu.key_right(),
            VK_DOWN => self.menu.key_down(),
            _ => return None,
        }
        self.after_input(ctx)
    }

    fn overlay(&mut self, now_ms: u64, adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        self.items(now_ms, adv)
    }

    /// The current page's rows a click picks, `options.<page>.<row>`
    /// ([`OptionsMenu::ui_elements`]).
    fn ui_elements(&self) -> Vec<UiElement> {
        self.menu.ui_elements()
    }
}

pub fn register(reg: &mut Registry) {
    reg.register(OPTIONS, Box::new(OptionsScreen::default()));
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
