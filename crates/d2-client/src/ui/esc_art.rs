//! The DC6 art of the Esc menu: which image each row's label and value
//! is, and the draw list of a menu (labels, value images, slider bar and
//! skull, `pentspin` pentagrams). The files go through the same
//! [`UiFiles`] / `archive_name` path as every other panel image (no
//! second loader); label and value images live under
//! `data\local\ui\eng\` ([`LOCAL_PREFIX`]), the widgets under
//! `data\global\ui\`.
//!
//! The widths are fixed: the game reads them from the
//! DC6 headers; multi-frame images tile at 256 px.
//!
//! The disabled look (draw mode 1) and the dark slider rectangles ([`RectRequest::sized`]) go
//! through the sink's draw mode and rectangle. D2RUST's own, unverified: the `pentspin` frame
//! follows the client clock ([`PentClock`]: the game's > 50 ms rule); `textslid` is the
//! key-config screen's, not this menu's.

use d2_sim::tick::TickRate;

use super::options_menu::{Kind, MenuId, OptionsMenu, Row};
use crate::ui::draw::{CelLook, ImageRef, ImageRequest, RectRequest, Remap, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::panels::UiFiles;

/// Prefix of a `UiFiles` name under `data\local\ui\eng\`.
pub const LOCAL_PREFIX: &str = "*local\\";
const BAR: &str = "widgets\\optbar";
const BAR_C: &str = "widgets\\optbarc";
const SKULL: &str = "widgets\\optskull";
const PENTSPIN: &str = "cursor\\pentspin";

/// Widths of the `pentspin` frames.
pub const PENT_WIDTHS: [i32; 8] = [51, 43, 27, 9, 23, 40, 50, 52];

/// An image and its total width.
pub type Art = (&'static str, i32);

/// The label image of a row; `None`: no art.
pub fn label(menu: MenuId, row: Row) -> Option<Art> {
    Some(match row {
        Row::Options => ("options", 160),
        Row::ExitGame => ("exit", 434),
        Row::ReturnToGame => ("returntogame", 356),
        Row::SoundOptions => ("soundoptions", 309),
        Row::VideoOptions => ("videooptions", 295),
        Row::AutomapOptions => ("automapoptions", 378),
        Row::ConfigureControls => ("cfgoptions", 423),
        Row::Previous if menu == MenuId::Options => ("previous", 313),
        Row::Previous => ("sprevious", 231),
        Row::Title => match menu {
            MenuId::Sound => ("soundoptions", 309),
            MenuId::Video => ("videooptions", 295),
            _ => ("automapoptions", 378),
        },
        Row::Sound => ("sound", 95),
        Row::Music => ("music", 84),
        Row::Sound3d => ("3dsound", 144),
        Row::Eax => ("eax", 362),
        Row::Bias3d => ("3dbias", 108),
        Row::NpcSpeech => ("npcspeech", 157),
        Row::LightQuality => ("lightquality", 265),
        Row::BlendShadow => ("blendshadow", 266),
        Row::Gamma => ("gamma", 105),
        Row::Contrast => ("contrast", 151),
        Row::MapMode => ("automapmode", 213),
        Row::MapFade => ("automapfade", 70),
        Row::MapCenter => ("automapcenter", 336),
        Row::MapParty => ("automapparty", 194),
        Row::MapNames => ("automappartynames", 190),
        // D2RUST's own display rows, and Resolution (its 16:9 choice has no
        // art), have no art: the menu draws their text.
        Row::Resolution
        | Row::WindowMode
        | Row::WindowSize
        | Row::Scale
        | Row::VSync
        | Row::FrameLimit
        | Row::AnimationFrames
        | Row::MenuSides => return None,
    })
}

/// The value image of choice value `v` of a row.
pub fn value(row: Row, v: usize) -> Option<Art> {
    const ON_OFF: [Art; 2] = [("smalloff", 51), ("smallon", 40)];
    const NO_YES: [Art; 2] = [("smallno", 40), ("smallyes", 51)];
    let list: &[Art] = match row {
        Row::Sound3d | Row::Eax | Row::BlendShadow => &ON_OFF,
        Row::NpcSpeech => &[("audioonly", 184), ("textonly", 169), ("audiotext", 260)],
        Row::LightQuality => &[("low", 66), ("medium", 108), ("high", 57)],
        Row::MapMode => &[("full", 180), ("mini", 131)],
        Row::MapFade => &[
            ("smallno", 40),
            ("center", 104),
            ("everything", 173),
            ("auto", 86),
        ],
        Row::MapCenter | Row::MapParty | Row::MapNames => &NO_YES,
        _ => return None,
    };
    list.get(v).copied()
}

/// Every file the menu names (lowercase `UiFiles` names).
pub fn esc_files() -> Vec<String> {
    let mut v: Vec<String> = [BAR, BAR_C, SKULL, PENTSPIN]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for menu in [
        MenuId::Game,
        MenuId::Options,
        MenuId::Sound,
        MenuId::Video,
        MenuId::Automap,
    ] {
        for d in menu.rows() {
            let vals = (0..4).filter_map(|i| value(d.row, i));
            for (name, _) in label(menu, d.row).into_iter().chain(vals) {
                let n = format!("{LOCAL_PREFIX}{name}");
                if !v.contains(&n) {
                    v.push(n);
                }
            }
        }
    }
    v
}

/// Whether `name` is one of the menu's files (a missing one draws empty).
pub fn is_esc_file(name: &str) -> bool {
    esc_files().iter().any(|f| f == name)
}

/// Frames of an image `w` px wide (tiled at 256 px).
fn frames(w: i32) -> i32 {
    (w + 255) / 256
}

/// The draw mode of an enabled row (mode 5, normal).
const DRAW_MODE_ENABLED: u8 = 5;
/// The draw mode of a disabled row (mode 1, 50 % blend).
const DRAW_MODE_DISABLED: u8 = 1;

/// Every frame of `name` from cel position (x, y) with the row's draw
/// `mode`.
#[allow(clippy::too_many_arguments)]
fn art_mode(
    files: &UiFiles,
    out: &mut dyn UiDrawSink,
    clip: Rect,
    name: &str,
    w: i32,
    x: i32,
    y: i32,
    mode: u8,
) {
    let Some(file) = files.id(name) else { return };
    for k in 0..frames(w) {
        out.push(UiDraw::Image(ImageRequest {
            image: ImageRef {
                file,
                frame: k as u32,
            },
            at: Point::new(x + 256 * k, y),
            clip,
            look: CelLook {
                mode,
                remap: Remap::None,
            },
            call: crate::ui::draw::CelCall::Draw,
        }));
    }
}

fn local(name: &str) -> String {
    format!("{LOCAL_PREFIX}{name}")
}

/// The `pentspin` counter: it advances by one,
/// modulo 8, once per draw at which more than 50 ms have passed since the
/// last advance, after that draw's frames are drawn (the stamp starts at
/// zero, so the first draw advances at once but still draws frame 0). In
/// live play the clock is the wall clock; check runs use the server tick's
/// real time at the game's tick rate ([`Self::ms_of_tick`]). A draw every
/// 40 ms steps every 2nd draw, one every 16.67 ms every 4th.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PentClock {
    frame: u32,
    last_ms: Option<u64>,
}

impl PentClock {
    /// One draw at `now_ms`: returns the frame to draw, then advances when
    /// > 50 ms passed.
    pub fn draw(&mut self, now_ms: u64) -> u32 {
        let drawn = self.frame;
        if self
            .last_ms
            .is_none_or(|last| now_ms.saturating_sub(last) > 50)
        {
            self.frame = (self.frame + 1) % 8;
            self.last_ms = Some(now_ms);
        }
        drawn
    }

    /// The real time of server tick `tick` at the game's tick rate `rate`
    /// in milliseconds (rounded down): [`CLIENT_TICK_MS`] per base-rate
    /// tick, so 40 ms a tick at 25 Hz and 16.67 ms at 60 Hz.
    ///
    /// [`CLIENT_TICK_MS`]: crate::rules::camera::CLIENT_TICK_MS
    pub fn ms_of_tick(tick: u64, rate: TickRate) -> u64 {
        rate.progress(u64::from(crate::rules::camera::CLIENT_TICK_MS), tick)
    }
}

/// Draws the art of row `i` (label, value or slider); `false`: the row
/// has no art and the caller draws its text.
pub fn draw_row(files: &UiFiles, m: &OptionsMenu, i: usize, out: &mut dyn UiDrawSink) -> bool {
    let def = &m.rows()[i];
    let Some((name, w)) = label(m.menu, def.row) else {
        return false;
    };
    let name = local(name);
    if files.id(&name).is_none() {
        return false;
    }
    let (half, clip) = (m.half(), m.screen.rect());
    let yb = m.baseline(i);
    // Mode 5 if enabled else 1, for every cel of the row.
    let mode = if m.enabled(i) {
        DRAW_MODE_ENABLED
    } else {
        DRAW_MODE_DISABLED
    };
    match def.kind {
        Kind::Title | Kind::Action => {
            art_mode(files, out, clip, &name, w, half - 1 - (w >> 1), yb, mode)
        }
        Kind::Choice(_) => {
            let Some((vn, vw)) = value(def.row, m.value(i) as usize) else {
                return false;
            };
            art_mode(files, out, clip, &name, w, half - 230, yb, mode);
            art_mode(files, out, clip, &local(vn), vw, half + 230 - vw, yb, mode);
        }
        Kind::Slider { style, .. } => {
            art_mode(files, out, clip, &name, w, half - 230, yb, mode);
            let y = m.slider_y(i);
            // The two dark rectangles (colour 0), mode 1 / 2 left
            // of the knob's right edge and 1 / 0 right of it (style 1 / 0);
            // the row mode does not apply to them.
            let t = m.slider_t(i);
            let (left, right) = if style == 1 { (1, 1) } else { (2, 0) };
            let x0 = half - 59;
            out.push(UiDraw::Rect(RectRequest::sized(
                x0,
                y - 30,
                t + 12,
                30,
                0,
                left,
            )));
            out.push(UiDraw::Rect(RectRequest::sized(
                x0 + t + 12,
                y - 30,
                half + 230 - (x0 + t + 12),
                30,
                0,
                right,
            )));
            let bar = if style == 1 { BAR_C } else { BAR };
            art_mode(files, out, clip, bar, 290, half - 60, y, mode);
            // Skull at X0 + t, y − 1 (− 1 more for style 0).
            let sy = y - 1 - i32::from(style == 0);
            art_mode(files, out, clip, SKULL, 28, half - 60 + t, sy, mode);
        }
    }
    true
}

/// The pentagrams at the selected row: the left one spins the
/// other way, drawn right-aligned in its 52 px cell.
pub fn draw_pents(files: &UiFiles, m: &OptionsMenu, f: u32, out: &mut dyn UiDrawSink) {
    let Some(file) = files.id(PENTSPIN) else {
        return;
    };
    let (half, clip) = (m.half(), m.screen.rect());
    let left = if f == 0 { 0 } else { 8 - f };
    let y = m.pentagram_y();
    for (frame, x) in [(left, half - 52 - 249), (f, half + 249)] {
        out.push(UiDraw::Image(ImageRequest {
            image: ImageRef { file, frame },
            at: Point::new(x, y),
            clip,
            look: crate::ui::CelLook::PLAIN,
            call: crate::ui::draw::CelCall::Draw,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::super::options_menu::MenuId;
    use super::*;

    fn files() -> UiFiles {
        let mut f = UiFiles::new(&[]);
        f.extend(esc_files());
        f
    }

    /// (name, frame, x, y) of every image drawn.
    fn draws(f: &UiFiles, m: &OptionsMenu, frame: u32) -> Vec<(String, u32, i32, i32)> {
        let mut out: Vec<UiDraw> = Vec::new();
        for i in 0..m.rows().len() {
            // D2RUST's own display rows and Resolution have no art.
            let own = matches!(
                m.rows()[i].row,
                Row::Resolution
                    | Row::WindowMode
                    | Row::WindowSize
                    | Row::Scale
                    | Row::VSync
                    | Row::FrameLimit
                    | Row::AnimationFrames
                    | Row::MenuSides
            );
            assert_eq!(draw_row(f, m, i, &mut out), !own);
        }
        draw_pents(f, m, frame, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Image(r) => Some((
                    f.name(r.image.file).unwrap().to_string(),
                    r.image.frame,
                    r.at.x,
                    r.at.y,
                )),
                _ => None,
            })
            .collect()
    }

    // D2RUST's own, unverified: draw list against the spec's positions
    #[test]
    fn game_menu_draws_centred_labels_and_pentspin_at_the_selected_row() {
        let f = files();
        let mut m = OptionsMenu::default();
        m.open();
        let d = draws(&f, &m, 0);
        // 160 / 434 / 356 px labels centred at 400 − 1 − w/2, baselines 224 / 274 / 324.
        assert_eq!(d[0], ("*local\\options".into(), 0, 319, 224));
        assert_eq!(d[1], ("*local\\exit".into(), 0, 182, 274));
        assert_eq!(d[2], ("*local\\exit".into(), 1, 438, 274));
        assert_eq!(d[3], ("*local\\returntogame".into(), 0, 221, 324));
        assert_eq!(d[4], ("*local\\returntogame".into(), 1, 477, 324));
        // Return to Game selected: pentagrams at (99, 336), (649, 336).
        assert_eq!(d[5], ("cursor\\pentspin".into(), 0, 99, 336));
        assert_eq!(d[6], ("cursor\\pentspin".into(), 0, 649, 336));
        // Later frame: the left one spins the other way.
        let d = draws(&f, &m, 3);
        assert_eq!((d[5].1, d[6].1), (5, 3));
    }

    // D2RUST's own, unverified: slider art against
    #[test]
    fn slider_draws_the_bar_and_the_knob_at_the_position() {
        let f = files();
        let mut m = OptionsMenu::default();
        m.open();
        m.menu = MenuId::Sound;
        m.selected = 1;
        // Master volume 100 → position 20: knob at X0 + 265.
        let d = draws(&f, &m, 0);
        let y_top = m.y_top(1);
        let label = d.iter().position(|e| e.0 == "*local\\sound").unwrap();
        assert_eq!(d[label], ("*local\\sound".into(), 0, 170, y_top + 34));
        let bar = (&d[label + 1], &d[label + 2]);
        assert_eq!(bar.0 .0, "widgets\\optbar");
        assert_eq!((bar.0 .2, bar.1 .2), (340, 596));
        let knob = d.iter().find(|e| e.0 == "widgets\\optskull").unwrap();
        let t = m.slider_t(1);
        assert_eq!(knob.2, 340 + t);
        assert_eq!(knob.3, y_top + 36 - 2);
        // Pentagrams at the selected row.
        assert_eq!(d.last().unwrap().3, y_top + 49);
    }

    #[test]
    fn slider_rectangles_precede_the_bar_and_a_disabled_row_draws_mode_1() {
        let f = files();
        let mut m = OptionsMenu::default();
        m.open();
        m.menu = MenuId::Sound;
        let half = m.half();
        let mut out: Vec<UiDraw> = Vec::new();
        assert!(draw_row(&f, &m, 1, &mut out));
        let rects: Vec<&RectRequest> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 2);
        let (t, y) = (m.slider_t(1), m.slider_y(1));
        // x [h − 59, h − 59 + t + 12), y [Y − 30, Y); then to h + 230.
        assert_eq!((rects[0].x0, rects[0].y0), (half - 59, y - 30));
        assert_eq!((rects[0].x1, rects[0].y1), (half - 59 + t + 12, y));
        assert_eq!((rects[1].x0, rects[1].x1), (half - 59 + t + 12, half + 230));
        assert!(rects.iter().all(|r| r.color == 0));
        // Order: label, rectangles, bar, skull.
        let first_rect = out
            .iter()
            .position(|d| matches!(d, UiDraw::Rect(_)))
            .unwrap();
        let last_rect = out
            .iter()
            .rposition(|d| matches!(d, UiDraw::Rect(_)))
            .unwrap();
        assert!(out[..first_rect]
            .iter()
            .all(|d| matches!(d, UiDraw::Image(_))));
        assert!(!out[..first_rect].is_empty());
        assert!(out[last_rect + 1..].len() >= 2);
        assert!(out[last_rect + 1..]
            .iter()
            .all(|d| matches!(d, UiDraw::Image(_))));
        let enabled = m.enabled(1);
        let modes: Vec<u8> = out
            .iter()
            .filter_map(|d| match d {
                UiDraw::Image(r) => Some(r.look.mode),
                _ => None,
            })
            .collect();
        let want = if enabled { 5 } else { 1 };
        assert!(modes.iter().all(|&x| x == want), "{modes:?}");
        // A disabled row (Sound is disabled in D2RUST) draws mode 1.
        m.menu = MenuId::Sound;
        let dis = (0..m.rows().len()).find(|&i| !m.enabled(i) && m.rows()[i].kind != Kind::Title);
        if let Some(i) = dis {
            let mut out: Vec<UiDraw> = Vec::new();
            draw_row(&f, &m, i, &mut out);
            assert!(out
                .iter()
                .filter_map(|d| match d {
                    UiDraw::Image(r) => Some(r.look.mode),
                    _ => None,
                })
                .all(|x| x == 1));
        }
    }

    #[test]
    fn choice_values_are_right_aligned_images() {
        let f = files();
        let mut m = OptionsMenu::default();
        m.open();
        m.menu = MenuId::Video;
        m.selected = 3;
        let d = draws(&f, &m, 0);
        let v = d.iter().find(|e| e.0.starts_with("*local\\h")).unwrap();
        assert_eq!(v.2, 630 - 57);
    }

    #[test]
    fn pentagrams_and_clip_follow_the_screen() {
        use crate::ui::layout::Screen;
        let f = files();
        for (screen, h, left_y) in [(Screen::R800, 400, 336), (Screen::R640, 320, 276)] {
            let mut m = OptionsMenu::default();
            m.screen = screen;
            m.open();
            let mut out: Vec<UiDraw> = Vec::new();
            draw_pents(&f, &m, 0, &mut out);
            let at: Vec<(i32, i32)> = out
                .iter()
                .filter_map(|d| match d {
                    UiDraw::Image(r) => {
                        assert_eq!(r.clip, screen.rect());
                        Some((r.at.x, r.at.y))
                    }
                    _ => None,
                })
                .collect();
            // Pentagrams at h − 301 and h + 249; Game menu, Return to Game
            // selected: y_top + 51 (800: 99 / 649 at y 336, spec).
            let y = m.pentagram_y();
            assert_eq!(y, left_y);
            assert_eq!(at, [(h - 301, y), (h + 249, y)]);
        }
    }

    #[test]
    fn the_pentagram_steps_when_more_than_50_ms_passed() {
        // 25 Hz draws (40 ms): the first draw steps at once, then every 2nd.
        let mut c = PentClock::default();
        let frames: Vec<u32> = (0..7)
            .map(|i| c.draw(PentClock::ms_of_tick(i, TickRate::BASE)))
            .collect();
        assert_eq!(frames, [0, 1, 1, 2, 2, 3, 3]);
        // 60 Hz draws (16.67 ms): every 4th.
        let mut c = PentClock::default();
        let frames: Vec<u32> = (0..9).map(|i| c.draw(i * 50 / 3)).collect();
        assert_eq!(frames, [0, 1, 1, 1, 1, 2, 2, 2, 2]);
        // The counter wraps at 8.
        let mut c = PentClock::default();
        assert_eq!((0..40).map(|i| c.draw(i * 60)).last(), Some(39 % 8));
    }

    /// The server-tick clock runs in real time at the game's 60 Hz: a tick
    /// is 16.67 ms, not the base rate's 40 ms (which spun the pentagram
    /// 2.4× too fast), so a draw per tick steps every 4th tick.
    #[test]
    fn the_server_tick_clock_is_real_time_at_60_hz() {
        let hz60 = TickRate::new(60).unwrap();
        assert_eq!(PentClock::ms_of_tick(0, hz60), 0);
        assert_eq!(PentClock::ms_of_tick(3, hz60), 50);
        assert_eq!(PentClock::ms_of_tick(4, hz60), 66);
        assert_eq!(PentClock::ms_of_tick(60, hz60), 1_000);
        assert_eq!(PentClock::ms_of_tick(25, TickRate::BASE), 1_000);
        let mut c = PentClock::default();
        let frames: Vec<u32> = (0..9)
            .map(|i| c.draw(PentClock::ms_of_tick(i, hz60)))
            .collect();
        assert_eq!(frames, [0, 1, 1, 1, 1, 2, 2, 2, 2]);
        // One second of 60 Hz ticks: 15 steps (every 66.7 ms), not the 30
        // of a 40 ms tick.
        let mut c = PentClock::default();
        let mut steps = 0;
        let mut last = None;
        for t in 1..=60 {
            let f = c.draw(PentClock::ms_of_tick(t, hz60));
            if last.is_some_and(|l| l != f) {
                steps += 1;
            }
            last = Some(f);
        }
        assert_eq!(steps, 15, "steps seen over one second of draws");
    }
}
