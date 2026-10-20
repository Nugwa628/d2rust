//! The Esc menu engine: the menu tree (Game → Options → Sound / Video /
//! Automap), the selected row, the input rules (wrap past disabled rows,
//! slider clamp, click on release, drag) and the mapping of each row onto
//! [`Settings`]. No drawing here: [`super::esc_menu`] draws and routes
//! events; this module only decides.
//!
//! The Video page opens with two read-only info lines that say what the
//! player is really looking at, read from the system every frame
//! ([`display_line`], [`picture_line`]): the monitor (or the window on it)
//! and the game picture with the scale it is shown at. They sit above the
//! rows, in a band the row layout leaves free ([`INFO_H`]).
//!
//! The Video page also holds D2RUST's own display rows, which apply as soon
//! as they're picked:
//! - View: the game picture's shape: 16:9 Widescreen (1068 x 600, the
//!   default), 4:3 Original (800 x 600) or 4:3 Classic (640 x 480). It takes
//!   effect at the next start.
//! - Window: Windowed (the default), Windowed Borderless or Fullscreen
//!   (exclusive, at the monitor's own resolution). Alt+Enter switches between
//!   the mode in use and Windowed Borderless (`app::config::alt_enter`).
//! - Window Size: the Windowed window's size: Maximized (the default) or a
//!   size in physical pixels from a list filtered to what fits the monitor.
//!   Maximized shows the window's real size; in the other modes the row is
//!   greyed and shows the monitor's own size ("Native (3840x2160)").
//! - Scale: fit (sharp at any scale) or integer.
//! - VSync: on or off.
//! - Frame Limit: Off or 30 to 360 frames per second. With VSync on, the
//!   limits above the monitor's refresh rate are greyed and skipped.
//! - Animation frames: Original or Smooth generated in-between pictures.
//!   The choice applies at once; no animation the deny list names changes.
//! - Menu Sides (after Contrast): what fills a wide window beside the main
//!   menu's 4:3 picture: black or a blur of it.
//!
//! Text has no row: it is drawn by the display upscaler like the rest of the
//! picture (`world_view::native_ui`), at every View, Window and Scale choice.
//!
//! The page has no Perspective row: D2RUST draws in one mode only, so the
//! row could never change, and its height goes to the info lines.
//!
//! An expansion install is assumed (expansion-only rows are present).

use std::borrow::Cow;

use crate::app::config::{
    AnimationFrameMode, MenuSides, ScaleMode, Settings, WindowMode, WindowSize,
    ANIMATION_FRAME_NAMES, FRAME_LIMITS, WINDOW_MODE_NAMES, WINDOW_SIZES, WINDOW_SIZE_NAMES,
};
use crate::app::frame_limit::limit_greyed;
use crate::ui::geom::{Point, Rect};

use crate::ui::layout::Screen;
use crate::ui::ui_ids::UiElement;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuId {
    #[default]
    Game,
    Options,
    Sound,
    Video,
    Automap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Title,
    Action,
    /// Number of values.
    Choice(u8),
    /// Positions (`n`) and style (0 plain, 1 centre-marked).
    Slider {
        n: u8,
        style: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Title,
    Options,
    ExitGame,
    ReturnToGame,
    SoundOptions,
    VideoOptions,
    AutomapOptions,
    ConfigureControls,
    Previous,
    Sound,
    Music,
    Sound3d,
    Eax,
    Bias3d,
    NpcSpeech,
    Resolution,
    WindowMode,
    WindowSize,
    Scale,
    VSync,
    FrameLimit,
    AnimationFrames,
    LightQuality,
    BlendShadow,
    Gamma,
    Contrast,
    MapMode,
    MapFade,
    MapCenter,
    MapParty,
    MapNames,
    MenuSides,
}

/// One row of a menu record: the label image name (drawn as
/// English text here, the DC6 labels are not available to this module)
/// and the value names of a choice.
#[derive(Clone, Copy, Debug)]
pub struct RowDef {
    pub row: Row,
    pub kind: Kind,
    pub label: &'static str,
    pub values: &'static [&'static str],
}

const fn def(row: Row, kind: Kind, label: &'static str, values: &'static [&'static str]) -> RowDef {
    RowDef {
        row,
        kind,
        label,
        values,
    }
}

const OFF_ON: &[&str] = &["Off", "On"];
const NO_YES: &[&str] = &["No", "Yes"];
/// The Frame Limit row's values, one per [`FRAME_LIMITS`] entry.
const FRAME_LIMIT_NAMES: [&str; FRAME_LIMITS.len()] =
    ["Off", "30", "60", "120", "144", "165", "240", "360"];
/// The View row's values, by the stored `resolution` value: 0 the
/// 640 x 480 frame, 1 the 800 x 600 frame, 2 the 1068 x 600 frame.
pub const VIEW_NAMES: [&str; 3] = ["4:3 Classic (640)", "4:3 Original", "16:9 Widescreen"];
const S21: Kind = Kind::Slider { n: 21, style: 0 };
const S21C: Kind = Kind::Slider { n: 21, style: 1 };

const GAME: &[RowDef] = &[
    def(Row::Options, Kind::Action, "Options", &[]),
    def(Row::ExitGame, Kind::Action, "Save and Exit Game", &[]),
    def(Row::ReturnToGame, Kind::Action, "Return to Game", &[]),
];
const OPTIONS: &[RowDef] = &[
    def(Row::SoundOptions, Kind::Action, "Sound Options", &[]),
    def(Row::VideoOptions, Kind::Action, "Video Options", &[]),
    def(Row::AutomapOptions, Kind::Action, "Automap Options", &[]),
    def(
        Row::ConfigureControls,
        Kind::Action,
        "Configure Controls",
        &[],
    ),
    def(Row::Previous, Kind::Action, "Previous Menu", &[]),
];
const SOUND: &[RowDef] = &[
    def(Row::Title, Kind::Title, "Sound Options", &[]),
    def(Row::Sound, S21, "Sound", &[]),
    def(Row::Music, S21, "Music", &[]),
    def(Row::Sound3d, Kind::Choice(2), "3D Sound", OFF_ON),
    def(Row::Eax, Kind::Choice(2), "EAX", OFF_ON),
    def(Row::Bias3d, S21C, "3D Bias", &[]),
    def(
        Row::NpcSpeech,
        Kind::Choice(3),
        "NPC Speech",
        &["Audio Only", "Text Only", "Audio and Text"],
    ),
    def(Row::Previous, Kind::Action, "Previous Menu", &[]),
];
const VIDEO: &[RowDef] = &[
    def(Row::Title, Kind::Title, "Video Options", &[]),
    def(
        Row::Resolution,
        Kind::Choice(3),
        "View (next start)",
        &VIEW_NAMES,
    ),
    def(
        Row::WindowMode,
        Kind::Choice(3),
        "Window",
        &WINDOW_MODE_NAMES,
    ),
    def(
        Row::WindowSize,
        Kind::Choice(WINDOW_SIZES.len() as u8),
        "Window Size",
        &WINDOW_SIZE_NAMES,
    ),
    def(Row::Scale, Kind::Choice(2), "Scale", &["Fit", "Integer"]),
    def(Row::VSync, Kind::Choice(2), "VSync", OFF_ON),
    def(
        Row::FrameLimit,
        Kind::Choice(FRAME_LIMITS.len() as u8),
        "Frame Limit",
        &FRAME_LIMIT_NAMES,
    ),
    def(
        Row::AnimationFrames,
        Kind::Choice(ANIMATION_FRAME_NAMES.len() as u8),
        "Animation frames",
        &ANIMATION_FRAME_NAMES,
    ),
    def(
        Row::LightQuality,
        Kind::Choice(3),
        "Light Quality",
        &["Low", "Medium", "High"],
    ),
    def(Row::BlendShadow, Kind::Choice(2), "Blended Shadows", OFF_ON),
    def(Row::Gamma, S21C, "Gamma", &[]),
    def(
        Row::Contrast,
        Kind::Slider { n: 100, style: 1 },
        "Contrast",
        &[],
    ),
    def(
        Row::MenuSides,
        Kind::Choice(2),
        "Menu Sides",
        &["Black", "Blur"],
    ),
    def(Row::Previous, Kind::Action, "Previous Menu", &[]),
];
const AUTOMAP: &[RowDef] = &[
    def(Row::Title, Kind::Title, "Automap Options", &[]),
    def(
        Row::MapMode,
        Kind::Choice(2),
        "Automap Size",
        &["Full Screen", "Mini"],
    ),
    def(
        Row::MapFade,
        Kind::Choice(4),
        "Automap Fade",
        &["No", "Center", "Everything", "Auto"],
    ),
    def(
        Row::MapCenter,
        Kind::Choice(2),
        "Center when Cleared",
        NO_YES,
    ),
    def(Row::MapParty, Kind::Choice(2), "Show Party", NO_YES),
    def(Row::MapNames, Kind::Choice(2), "Show Party Names", NO_YES),
    def(Row::Previous, Kind::Action, "Previous Menu", &[]),
];

impl MenuId {
    pub fn rows(self) -> &'static [RowDef] {
        match self {
            MenuId::Game => GAME,
            MenuId::Options => OPTIONS,
            MenuId::Sound => SOUND,
            MenuId::Video => VIDEO,
            MenuId::Automap => AUTOMAP,
        }
    }

    /// Header: row pitch, label baseline, pentagram and slider
    /// offsets.
    pub fn header(self) -> (i32, i32, i32, i32) {
        match self {
            MenuId::Game | MenuId::Options => (50, 39, 51, 0),
            _ => (45, 34, 49, 36),
        }
    }
}

/// What the menu asks of its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    /// UI sound `cursor_pass` (1).
    CursorPass,
    /// UI sound `cursor_select` (2).
    CursorSelect,
    /// Save and Exit Game.
    SaveAndExit,
    /// Return to Game: close the whole menu.
    Close,
    /// Configure Controls.
    ConfigureControls,
}

/// The slider value mapping: `(min, max, n)` of a row.
fn slider_range(row: Row) -> (i64, i64, i64) {
    match row {
        Row::Gamma => (55, 255, 21),
        Row::Contrast => (0, 100, 100),
        _ => (0, 100, 21),
    }
}

/// Position from value: `trunc((v − min + 1)(n − 1) / (max − min))`.
pub fn pos_from_value(row: Row, v: i64) -> i64 {
    let (lo, hi, n) = slider_range(row);
    ((v - lo + 1) * (n - 1) / (hi - lo)).clamp(0, n - 1)
}

/// Value from position: `trunc(min + (max − min) p / (n − 1))`.
pub fn value_from_pos(row: Row, p: i64) -> i64 {
    let (lo, hi, n) = slider_range(row);
    lo + (hi - lo) * p / (n - 1)
}

/// What the platform reports about the game's window right now, for the
/// Video page's info lines and the Window Size row. The host fills it
/// every frame, so it follows each setting change and Alt+Enter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowFacts {
    /// The mode the window is in now (Alt+Enter included).
    pub mode: Option<WindowMode>,
    /// The window's size in physical pixels.
    pub size: Option<(u32, u32)>,
    /// The game frame in force. The View row changes it at the next start,
    /// so it can differ from the row's value.
    pub frame: Option<(u32, u32)>,
}

/// The height of the Video page's info band above its rows: two lines of
/// Font16 ([`INFO_LINE`] apart) and a little space under them.
pub const INFO_H: i32 = 2 * INFO_LINE + 2;
/// The distance between the info lines' baselines.
pub const INFO_LINE: i32 = 14;

/// The least space from a slider row's top to the next row's top: the
/// slider's box needs it.
pub const SLIDER_PITCH: i32 = 33;
/// The baseline of the first info line below the band's top.
pub const INFO_BASELINE: i32 = 13;

/// The value column of a choice row: a value up to this wide is centred in
/// it; a wider one ends at its right edge, `half + 230`.
pub const VALUE_BLOCK: i32 = 130;

/// The left x of a choice value `w` pixels wide (see [`VALUE_BLOCK`]).
pub fn value_x(half: i32, w: i32) -> i32 {
    let right = half + 230;
    if w <= VALUE_BLOCK {
        right - VALUE_BLOCK + (VALUE_BLOCK - w) / 2
    } else {
        right - w
    }
}

fn size_text((w, h): (u32, u32)) -> String {
    format!("{w}x{h}")
}

/// The monitor as "3840x2160 at 144 Hz" (the rate left out when unknown).
fn monitor_text(monitor: Option<(u32, u32)>, refresh_hz: Option<u32>) -> Option<String> {
    let m = size_text(monitor?);
    Some(match refresh_hz {
        Some(hz) => format!("{m} at {hz} Hz"),
        None => m,
    })
}

/// The first info line: the monitor and how the window sits on it.
/// - Windowed: "Display: Window 1920x1080 on 3840x2160 at 144 Hz", the
///   window's real size in physical pixels;
/// - Windowed Borderless and Fullscreen, which cover the monitor:
///   "Display: 3840x2160 at 144 Hz, Fullscreen".
///
/// `mode` is the mode the window is in; a part the platform hasn't
/// reported is left out.
pub fn display_line(
    monitor: Option<(u32, u32)>,
    refresh_hz: Option<u32>,
    mode: WindowMode,
    window: Option<(u32, u32)>,
) -> String {
    let screen = monitor_text(monitor, refresh_hz);
    match (mode, window, screen) {
        (WindowMode::Windowed, Some(w), Some(s)) => {
            format!("Display: Window {} on {s}", size_text(w))
        }
        (WindowMode::Windowed, Some(w), None) => format!("Display: Window {}", size_text(w)),
        (_, w, s) => {
            let s = s.or_else(|| w.map(size_text)).unwrap_or_else(|| "?".into());
            let name = WindowMode::ALL
                .iter()
                .position(|&m| m == mode)
                .map_or("", |i| WINDOW_MODE_NAMES[i]);
            format!("Display: {s}, {name}")
        }
    }
}

/// A scale factor as the info line shows it: "2x" when whole, else up to
/// two decimals ("1.8x", "1.28x").
pub fn scale_text(scale: f64) -> String {
    let hundredths = (scale * 100.0).round() as i64;
    let (whole, frac) = (hundredths / 100, hundredths % 100);
    match frac {
        0 => format!("{whole}x"),
        f if f % 10 == 0 => format!("{whole}.{}x", f / 10),
        f => format!("{whole}.{f:02}x"),
    }
}

/// The second info line: the game frame, the scale it is shown at in the
/// window and how ("Sharp" for Fit, whose sharp scaler keeps every game
/// pixel a solid block; "Integer"), e.g. "Picture: 1068x600 shown at 3.6x
/// (Sharp)". A window too small for Integer shows the picture as Fit does,
/// and the line says so. When the View row asks for another frame than the
/// one in force, the line ends with what the next start shows.
pub fn picture_line(
    frame: (u32, u32),
    window: Option<(u32, u32)>,
    mode: ScaleMode,
    next: Option<&str>,
) -> String {
    let mut out = format!("Picture: {}", size_text(frame));
    if let Some(p) =
        window.and_then(|(w, h)| crate::ui::edge::present_in(w, h, frame.0, frame.1, mode).ok())
    {
        let scale = f64::from(p.image_w) / f64::from(p.frame_w);
        let scale = scale.min(f64::from(p.image_h) / f64::from(p.frame_h));
        let how = match p.mode {
            ScaleMode::Fit => "Sharp",
            ScaleMode::Integer => "Integer",
        };
        out.push_str(&format!(" shown at {} ({how})", scale_text(scale)));
    }
    if let Some(next) = next {
        out.push_str(&format!(", next start: {next}"));
    }
    out
}

/// The menu's state.
#[derive(Clone, Debug)]
pub struct OptionsMenu {
    /// W and H: the play frame (800 × 600 or 640 × 480).
    pub screen: Screen,
    pub menu: MenuId,
    pub selected: usize,
    pub settings: Settings,
    /// The sound device is up; down under `--no-sound`.
    pub sound_device: bool,
    /// The monitor's physical size, when known: the Window Size row offers
    /// only the sizes that fit on it.
    pub monitor: Option<(u32, u32)>,
    /// The refresh rate of the monitor the window is on, in Hz, when known:
    /// with VSync on, the Frame Limit row greys the limits above it.
    pub refresh_hz: Option<u32>,
    /// The window as the platform reports it now: the Video page's info
    /// lines and the Window Size row's Maximized size read it.
    pub window: WindowFacts,
    /// Space kept free above and below the rows when a page is too tall for
    /// the frame (the main menu's panel border needs it); 0 in game.
    pub margin: i32,
    /// A setting changed and the host has not written it yet.
    pub changed: bool,
    dragging: bool,
    latch: bool,
    last_ptr: Option<Point>,
    pub events: Vec<MenuEvent>,
}

impl Default for OptionsMenu {
    fn default() -> Self {
        Self {
            screen: Screen::R800,
            menu: MenuId::default(),
            selected: 0,
            settings: Settings::default(),
            sound_device: true,
            monitor: None,
            refresh_hz: None,
            window: WindowFacts::default(),
            margin: 0,
            changed: false,
            dragging: false,
            latch: false,
            last_ptr: None,
            events: Vec::new(),
        }
    }
}

impl OptionsMenu {
    /// `h = W / 2` (400 at 800 × 600, 320 at 640 × 480).
    pub fn half(&self) -> i32 {
        self.screen.w / 2
    }

    /// Open on the Game menu, Return to Game selected.
    pub fn open(&mut self) {
        self.go(MenuId::Game);
        self.dragging = false;
        self.latch = false;
    }

    /// Take the settings in force; Gamma and Contrast snap to their grid
    /// as at every game join (and written back).
    pub fn set_settings(&mut self, s: Settings) {
        self.settings = s;
        for row in [Row::Gamma, Row::Contrast] {
            let v = self.raw(row);
            let snapped = value_from_pos(row, pos_from_value(row, v));
            if snapped != v {
                self.store(row, snapped);
                self.changed = true;
            }
        }
    }

    /// Back from Configure Controls: the Options menu with Previous Menu
    /// selected (the same entry rule as `go`).
    pub fn return_from_controls(&mut self) {
        self.go(MenuId::Options);
    }

    /// Open straight on page `m` (the main menu's Options screen opens on
    /// the Video page) with its first row that can be picked selected.
    /// Previous Menu still leads up the same tree.
    pub fn open_page(&mut self, m: MenuId) {
        self.go(m);
        if let Some(i) = (0..self.rows().len()).find(|&i| self.enabled(i)) {
            self.selected = i;
        }
        self.dragging = false;
        self.latch = false;
    }

    fn go(&mut self, m: MenuId) {
        self.menu = m;
        self.selected = m.rows().len() - 1;
    }

    pub fn rows(&self) -> &'static [RowDef] {
        self.menu.rows()
    }

    /// Whether row `i` can be picked: titles never; 3D Sound, EAX and 3D
    /// Bias are always off in D2RUST; Sound Options, Sound, Music and NPC
    /// Speech only while the sound device is up; Party Names only while the
    /// party is shown.
    pub fn enabled(&self, i: usize) -> bool {
        match self.rows()[i].row {
            Row::Title | Row::Sound3d | Row::Eax | Row::Bias3d => false,
            // Windowed Borderless and Fullscreen always cover the monitor at
            // its own size: the Window Size row is greyed.
            Row::WindowSize => self.settings.window_mode == WindowMode::Windowed,
            Row::SoundOptions | Row::Sound | Row::Music | Row::NpcSpeech => self.sound_device,
            Row::MapNames => self.settings.automap_party != 0,
            _ => true,
        }
    }

    /// The row's label.
    pub fn label_text(&self, i: usize) -> Cow<'static, str> {
        Cow::Borrowed(self.rows()[i].label)
    }

    /// The stored integer of a setting row (registry-style value).
    fn raw(&self, row: Row) -> i64 {
        let s = &self.settings;
        match row {
            Row::Sound => s.master_volume.into(),
            Row::Music => s.music_volume.into(),
            Row::Bias3d => s.positional_bias.into(),
            Row::Gamma => s.gamma.into(),
            Row::Contrast => s.contrast.into(),
            _ => 0,
        }
    }

    fn store(&mut self, row: Row, v: i64) {
        let s = &mut self.settings;
        match row {
            Row::Sound => s.master_volume = v as u8,
            Row::Music => s.music_volume = v as u8,
            Row::Bias3d => s.positional_bias = v as u8,
            Row::Gamma => s.gamma = v as u16,
            Row::Contrast => s.contrast = v as u8,
            _ => {}
        }
    }

    /// The choice index or slider position of row `i`.
    pub fn value(&self, i: usize) -> i64 {
        let row = self.rows()[i].row;
        let s = &self.settings;
        match row {
            Row::Sound | Row::Music | Row::Bias3d | Row::Gamma | Row::Contrast => {
                pos_from_value(row, self.raw(row))
            }
            Row::Sound3d => i64::from(matches!(s.mixer, 1 | 2)),
            Row::Eax => i64::from(s.mixer == 2),
            Row::NpcSpeech => s.npc_speech.into(),
            Row::Resolution => s.resolution.into(),
            // Indices into `WindowMode::ALL`, `WINDOW_SIZES` and `ScaleMode::ALL`.
            Row::WindowMode => WindowMode::ALL
                .iter()
                .position(|&m| m == s.window_mode)
                .unwrap_or(0) as i64,
            Row::WindowSize => s.window_size.index() as i64,
            Row::Scale => ScaleMode::ALL
                .iter()
                .position(|&m| m == s.scale_mode)
                .unwrap_or(0) as i64,
            Row::VSync => s.vsync.into(),
            Row::FrameLimit => FRAME_LIMITS
                .iter()
                .position(|&l| l == s.frame_limit)
                .unwrap_or(0) as i64,
            Row::AnimationFrames => AnimationFrameMode::ALL
                .iter()
                .position(|&m| m == s.animation_frames)
                .unwrap_or(0) as i64,
            Row::LightQuality => s.light_quality.into(),
            Row::BlendShadow => s.blended_shadows.into(),
            Row::MapMode => s.automap_mode.into(),
            Row::MapFade => s.automap_fade.into(),
            Row::MapCenter => s.automap_centers.into(),
            Row::MapParty => s.automap_party.into(),
            // Party and names.
            Row::MapNames => i64::from(s.automap_party != 0 && s.automap_party_names != 0),
            // Index into `MenuSides::ALL`.
            Row::MenuSides => MenuSides::ALL
                .iter()
                .position(|&m| m == s.menu_sides)
                .unwrap_or(0) as i64,
            _ => 0,
        }
    }

    /// The text row `i`'s value shows: its value name, except on the
    /// Window Size row, which shows what the window really is:
    /// - in Windowed Borderless and Fullscreen (row greyed), the monitor's
    ///   own size: "Native (3840x2160)";
    /// - Maximized in a Windowed window, the window's size now:
    ///   "Maximized (2560x1377)".
    pub fn value_text(&self, i: usize) -> Cow<'static, str> {
        let def = &self.rows()[i];
        let name = def
            .values
            .get(self.value(i) as usize)
            .copied()
            .unwrap_or("");
        if def.row != Row::WindowSize {
            return Cow::Borrowed(name);
        }
        let s = &self.settings;
        if s.window_mode != WindowMode::Windowed {
            return match self.monitor.or(self.window.size) {
                Some(m) => Cow::Owned(format!("Native ({})", size_text(m))),
                None => Cow::Borrowed("Native"),
            };
        }
        match (s.window_size, self.window.mode, self.window.size) {
            (WindowSize::Maximized, Some(WindowMode::Windowed), Some(w)) => {
                Cow::Owned(format!("{name} ({})", size_text(w)))
            }
            _ => Cow::Borrowed(name),
        }
    }

    /// The Video page's info lines ([`display_line`], [`picture_line`]);
    /// none on the other pages.
    pub fn info_lines(&self) -> Vec<String> {
        if self.menu != MenuId::Video {
            return Vec::new();
        }
        let s = &self.settings;
        let mode = self.window.mode.unwrap_or(s.window_mode);
        let view = s.frame_size();
        let view = (view.width as u32, view.height as u32);
        let frame = self.window.frame.unwrap_or(view);
        let next = (frame != view).then(|| VIEW_NAMES[usize::from(s.resolution.min(2))]);
        vec![
            display_line(self.monitor, self.refresh_hz, mode, self.window.size),
            picture_line(frame, self.window.size, s.scale_mode, next),
        ]
    }

    /// The top of the info band: [`INFO_H`] above the first row.
    pub fn info_top(&self) -> i32 {
        self.y0() - self.info_h()
    }

    /// The baseline of info line `k`.
    pub fn info_baseline(&self, k: usize) -> i32 {
        self.info_top() + INFO_BASELINE + INFO_LINE * k as i32
    }

    fn info_h(&self) -> i32 {
        if self.menu == MenuId::Video {
            INFO_H
        } else {
            0
        }
    }

    /// The apply callback: store the new value, mark it for the
    /// host to write.
    fn apply(&mut self, i: usize, v: i64) {
        let row = self.rows()[i].row;
        match row {
            Row::Sound | Row::Music | Row::Bias3d | Row::Gamma | Row::Contrast => {
                self.store(row, value_from_pos(row, v));
            }
            Row::NpcSpeech => self.settings.npc_speech = v as u8,
            Row::Resolution => self.settings.resolution = v as u8,
            Row::WindowMode => self.settings.set_window_mode(WindowMode::ALL[v as usize]),
            Row::WindowSize => self.settings.window_size = WINDOW_SIZES[v as usize],
            Row::Scale => self.settings.scale_mode = ScaleMode::ALL[v as usize],
            Row::VSync => self.settings.vsync = v != 0,
            Row::FrameLimit => self.settings.frame_limit = FRAME_LIMITS[v as usize],
            Row::AnimationFrames => {
                self.settings.animation_frames = AnimationFrameMode::ALL[v as usize]
            }
            Row::LightQuality => self.settings.light_quality = v as u8,
            Row::BlendShadow => self.settings.blended_shadows = v as u8,
            Row::MapMode => self.settings.automap_mode = v as u8,
            Row::MapFade => self.settings.automap_fade = v as u8,
            Row::MapCenter => self.settings.automap_centers = v as u8,
            Row::MapParty => self.settings.automap_party = v as u8,
            Row::MapNames => self.settings.automap_party_names = v as u8,
            Row::MenuSides => self.settings.menu_sides = MenuSides::ALL[v as usize],
            _ => return,
        }
        self.changed = true;
    }

    // ---- geometry ----

    /// The height the rows may use: the frame less the info band and the
    /// margins.
    fn room(&self) -> i32 {
        self.screen.h - 2 * self.margin - self.info_h()
    }

    /// The row pitch: the page's own, or less when its rows wouldn't fit
    /// the room they have (the Video page): then the rows share the height
    /// left above the last row's full box, so every row stays on screen
    /// whatever the page's row count. A slider row keeps at least
    /// [`SLIDER_PITCH`] to the next row for its box ([`Self::gap`]); the
    /// other rows share what is left.
    fn pitch(&self) -> i32 {
        let p = self.menu.header().0;
        let n = self.n();
        if p * n <= self.room() || n < 2 {
            return p;
        }
        let even = (self.room() - p) / (n - 1);
        let sliders = self.slider_gaps();
        if even >= SLIDER_PITCH.min(p) || sliders == 0 || sliders >= n - 1 {
            return even;
        }
        (self.room() - self.last_row_height() - SLIDER_PITCH * sliders) / (n - 1 - sliders)
    }

    /// What a compressed page leaves for its last row. The Video page's
    /// last row needs less than its normal pitch at 640 x 480, so its
    /// labels keep 30 pixels of pitch while still fitting the frame.
    fn last_row_height(&self) -> i32 {
        if self.menu == MenuId::Video {
            self.menu.header().0.min(22)
        } else {
            self.menu.header().0
        }
    }

    /// The label offset of the last row: its normal offset unless the
    /// compressed Video page has less room.
    fn last_row_offset(&self, normal: i32) -> i32 {
        if self.menu != MenuId::Video {
            return normal;
        }
        let last = self.rows().len() - 1;
        let room = self.screen.h - self.margin - self.y_top(last);
        normal.min((room - 2).max(12))
    }

    /// How many rows other than the last are sliders: the gaps below them
    /// keep [`SLIDER_PITCH`].
    fn slider_gaps(&self) -> i32 {
        let rows = self.rows();
        rows[..rows.len().saturating_sub(1)]
            .iter()
            .filter(|d| matches!(d.kind, Kind::Slider { .. }))
            .count() as i32
    }

    /// The space from the top of row `i` to the top of row `i + 1`:
    /// [`Self::pitch`], and at least [`SLIDER_PITCH`] below a slider row.
    fn gap(&self, i: usize) -> i32 {
        let pitch = self.pitch();
        match self.rows().get(i).map(|d| d.kind) {
            Some(Kind::Slider { .. }) => pitch.max(SLIDER_PITCH.min(self.menu.header().0)),
            _ => pitch,
        }
    }

    /// The rows' height: from the first row's top to where a row after the
    /// last would start.
    fn span(&self) -> i32 {
        (0..self.rows().len()).map(|i| self.gap(i)).sum()
    }

    fn n(&self) -> i32 {
        self.rows().len() as i32
    }

    /// y of the first row's top: the rows are centred above the bottom 80
    /// pixels of the frame. A page too tall for that (the Video page)
    /// starts right under the margin and its info band instead, so no row
    /// is cut off.
    pub fn y0(&self) -> i32 {
        ((self.screen.h - 80) / 2 - self.span() / 2).max(self.margin + self.info_h())
    }

    pub fn y_top(&self, i: usize) -> i32 {
        self.y0() + (0..i).map(|k| self.gap(k)).sum::<i32>()
    }

    /// Label baseline of row `i`.
    pub fn baseline(&self, i: usize) -> i32 {
        let offset = if i + 1 == self.rows().len() {
            self.last_row_offset(self.menu.header().1)
        } else {
            self.menu.header().1
        };
        self.y_top(i) + offset
    }

    /// Pentagram y of the selected row.
    pub fn pentagram_y(&self) -> i32 {
        let offset = if self.selected + 1 == self.rows().len() {
            self.last_row_offset(self.menu.header().2)
        } else {
            self.menu.header().2
        };
        self.y_top(self.selected) + offset
    }

    /// Slider base Y of row `i`.
    pub fn slider_y(&self, i: usize) -> i32 {
        self.y_top(i) + self.menu.header().3
    }

    /// Slider knob offset `t = trunc(265 p / (n − 1))` of row `i`.
    pub fn slider_t(&self, i: usize) -> i32 {
        let Kind::Slider { n, .. } = self.rows()[i].kind else {
            return 0;
        };
        (265 * self.value(i) / (i64::from(n) - 1)) as i32
    }

    /// The row under the pointer, by y only: none outside the rows' span,
    /// on a title or on a disabled row.
    pub fn row_at(&self, y: i32) -> Option<usize> {
        // The rows' span, from the same top as the drawing.
        let top = self.y0();
        let bottom = top + 2 * (self.span() / 2);
        if !(top < y && y < bottom) {
            return None;
        }
        let n = self.n();
        let i = (0..self.rows().len())
            .rev()
            .find(|&i| y >= self.y_top(i) + n)?;
        (self.rows()[i].kind != Kind::Title && self.enabled(i)).then_some(i)
    }

    // ---- input ----

    fn push(&mut self, e: MenuEvent) {
        self.events.push(e);
    }

    /// Down (`step` = 1) or Up (`step` = −1): next enabled row, wrapping.
    fn step(&mut self, step: i32) {
        let n = self.rows().len() as i32;
        let mut i = self.selected as i32;
        loop {
            i = (i + step).rem_euclid(n);
            if self.enabled(i as usize) || i as usize == self.selected {
                break;
            }
        }
        self.selected = i as usize;
        self.push(MenuEvent::CursorPass);
    }

    pub fn key_down(&mut self) {
        self.step(1);
    }

    pub fn key_up(&mut self) {
        self.step(-1);
    }

    /// Whether choice `v` of row `i` can be picked: a window size must fit
    /// on the monitor, and a frame limit must not be greyed ([`Self::greyed`]).
    fn allowed(&self, i: usize, v: i64) -> bool {
        match self.rows()[i].row {
            Row::WindowSize => WINDOW_SIZES
                .get(v as usize)
                .is_some_and(|s: &WindowSize| s.fits(self.monitor)),
            Row::FrameLimit => !self.greyed(i, v),
            _ => true,
        }
    }

    /// Whether choice `v` of row `i` is shown greyed: with VSync on, a frame
    /// limit above the monitor's refresh rate has no effect. A limit picked
    /// with VSync off stays stored when VSync goes on, shown greyed.
    pub fn greyed(&self, i: usize, v: i64) -> bool {
        match self.rows()[i].row {
            Row::FrameLimit => FRAME_LIMITS
                .get(v as usize)
                .is_some_and(|&l| limit_greyed(self.settings.vsync, l, self.refresh_hz)),
            _ => false,
        }
    }

    /// The next choice of row `i` (`n` values) after `old` in the `delta`
    /// direction that can be picked, wrapping; `old` when no other can.
    fn next_choice(&self, i: usize, n: i64, old: i64, delta: i64) -> i64 {
        let mut v = old;
        for _ in 0..n {
            v = (v + delta).rem_euclid(n);
            if self.allowed(i, v) {
                return v;
            }
        }
        old
    }

    fn change(&mut self, delta: i64) {
        let i = self.selected;
        if !self.enabled(i) {
            return;
        }
        let old = self.value(i);
        let new = match self.rows()[i].kind {
            Kind::Choice(n) => self.next_choice(i, i64::from(n), old, delta),
            Kind::Slider { n, .. } => (old + delta).clamp(0, i64::from(n) - 1),
            _ => return,
        };
        if new != old {
            self.apply(i, new);
            self.push(MenuEvent::CursorPass);
        }
    }

    pub fn key_left(&mut self) {
        self.change(-1);
    }

    pub fn key_right(&mut self) {
        self.change(1);
    }

    pub fn key_enter(&mut self) {
        self.activate();
    }

    /// Disabled rows are ignored.
    fn activate(&mut self) {
        let i = self.selected;
        if !self.enabled(i) {
            return;
        }
        match self.rows()[i].kind {
            Kind::Action => {
                self.push(MenuEvent::CursorSelect);
                match self.rows()[i].row {
                    Row::Options => self.go(MenuId::Options),
                    Row::ExitGame => self.push(MenuEvent::SaveAndExit),
                    Row::ReturnToGame => self.push(MenuEvent::Close),
                    Row::SoundOptions => self.go(MenuId::Sound),
                    Row::VideoOptions => self.go(MenuId::Video),
                    Row::AutomapOptions => self.go(MenuId::Automap),
                    Row::ConfigureControls => self.push(MenuEvent::ConfigureControls),
                    // Previous Menu of Options → Game; of a sub-menu → Options.
                    _ => self.go(if self.menu == MenuId::Options {
                        MenuId::Game
                    } else {
                        MenuId::Options
                    }),
                }
            }
            Kind::Choice(n) => {
                let new = self.next_choice(i, i64::from(n), self.value(i), 1);
                self.apply(i, new);
                self.push(MenuEvent::CursorPass);
            }
            _ => {}
        }
    }

    /// Slider drag with the pointer at `p`.
    fn drag(&mut self, p: Point) {
        let i = self.selected;
        if !(self.latch || self.row_at(p.y) == Some(i)) || !self.enabled(i) {
            return;
        }
        let Kind::Slider { n, .. } = self.rows()[i].kind else {
            return;
        };
        if !self.latch && !(self.half() - 59 < p.x && p.x < self.half() + 230) {
            return;
        }
        let n = i64::from(n);
        let x0 = self.half() - 48;
        let pos = if p.x < x0 {
            0
        } else if p.x > x0 + 265 {
            n - 1
        } else {
            let f = (265.0f64 / (n - 1) as f64 * 0.5) as f32;
            let a = ((p.x - x0) as f32 / f + 1.0) as i64;
            a / 2
        };
        self.latch = true;
        if pos != self.value(i) {
            self.apply(i, pos);
            self.push(MenuEvent::CursorPass);
        }
    }

    /// Left button down.
    pub fn press(&mut self, p: Point) {
        self.last_ptr = Some(p);
        if let Some(i) = self.row_at(p.y) {
            self.selected = i;
        }
        self.dragging = true;
        self.drag(p);
    }

    /// Left button up: activates when the pointer is still on the selected row.
    pub fn release(&mut self, p: Point) {
        self.last_ptr = Some(p);
        if self.dragging && self.row_at(p.y) == Some(self.selected) {
            self.activate();
        }
        self.dragging = false;
        self.latch = false;
    }

    /// Pointer moved (checked at each draw): drag or hover.
    pub fn moved(&mut self, p: Point) {
        if self.last_ptr == Some(p) {
            return;
        }
        self.last_ptr = Some(p);
        if self.dragging {
            self.drag(p);
        } else if let Some(i) = self.row_at(p.y) {
            self.selected = i;
        }
    }

    pub fn take_events(&mut self) -> Vec<MenuEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn take_changed(&mut self) -> Option<Settings> {
        std::mem::take(&mut self.changed).then_some(self.settings)
    }
}

impl OptionsMenu {
    /// The clickable elements of the open page ([`crate::ui::ui_ids`]): its
    /// rows, `options.<page>.<row>` ([`page_id`], [`row_id`]). Each is the
    /// row's band of the frame, as the menu's own hit test
    /// ([`OptionsMenu::row_at`]) finds it, in a strip
    /// left of the slider track, so a click on a slider row selects it
    /// without moving the slider. A click on a choice row steps its value;
    /// on an action row it opens the page.
    pub fn ui_elements(&self) -> Vec<UiElement> {
        let menu = self;
        let page = page_id(menu.menu);
        let x = menu.half() - ROW_ID_LEFT;
        let mut out: Vec<UiElement> = Vec::new();
        let mut y = 0;
        while y < menu.screen.h {
            let Some(i) = menu.row_at(y) else {
                y += 1;
                continue;
            };
            let top = y;
            while y < menu.screen.h && menu.row_at(y) == Some(i) {
                y += 1;
            }
            let h = u16::try_from(y - top).unwrap_or(u16::MAX);
            let id = format!("options.{page}.{}", row_id(menu.rows()[i].row));
            if !out.iter().any(|e| e.id == id) {
                let mut element = UiElement::new(id, Rect::new(x, top, ROW_ID_W, h));
                if matches!(menu.rows()[i].kind, Kind::Slider { .. }) {
                    element.ends = Some([
                        Point::new(menu.half() - 48, menu.slider_y(i)),
                        Point::new(menu.half() + 218, menu.slider_y(i)),
                    ]);
                }
                out.push(element);
            }
        }
        out
    }
}

/// The rows' click strip ([`OptionsScreen::ui_elements`]): from this far
/// left of the centre, this wide. It ends 60 pixels left of the centre,
/// where a slider row's drag area begins.
const ROW_ID_LEFT: i32 = 250;
const ROW_ID_W: u16 = 190;

/// A page's name in ui ids.
pub fn page_id(m: MenuId) -> &'static str {
    match m {
        MenuId::Game => "game",
        MenuId::Options => "options",
        MenuId::Sound => "sound",
        MenuId::Video => "video",
        MenuId::Automap => "automap",
    }
}

/// A row's name in ui ids.
pub fn row_id(r: Row) -> &'static str {
    match r {
        Row::Title => "title",
        Row::Options => "options",
        Row::ExitGame => "exit_game",
        Row::ReturnToGame => "return_to_game",
        Row::SoundOptions => "sound_options",
        Row::VideoOptions => "video_options",
        Row::AutomapOptions => "automap_options",
        Row::ConfigureControls => "configure_controls",
        Row::Previous => "previous",
        Row::Sound => "sound",
        Row::Music => "music",
        Row::Sound3d => "sound_3d",
        Row::Eax => "eax",
        Row::Bias3d => "bias_3d",
        Row::NpcSpeech => "npc_speech",
        Row::Resolution => "resolution",
        Row::WindowMode => "window_mode",
        Row::WindowSize => "window_size",
        Row::Scale => "scale",
        Row::VSync => "vsync",
        Row::FrameLimit => "frame_limit",
        Row::AnimationFrames => "animation_frames",
        Row::LightQuality => "light_quality",
        Row::BlendShadow => "blend_shadow",
        Row::Gamma => "gamma",
        Row::Contrast => "contrast",
        Row::MapMode => "map_mode",
        Row::MapFade => "map_fade",
        Row::MapCenter => "map_center",
        Row::MapParty => "map_party",
        Row::MapNames => "map_names",
        Row::MenuSides => "menu_sides",
    }
}

#[cfg(test)]
#[path = "options_menu_tests.rs"]
mod tests;
