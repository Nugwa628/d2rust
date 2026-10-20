//! The front end: a state machine from the trademark screen through the main
//! menu to the game load. Plain Rust; the host feeds [`FrontInput`]s, calls
//! [`FrontEnd::tick`] every 40 ms (C0) and draws [`FrontEnd::draw`].
//!
//! Adding a screen: implement [`Screen`] in `screens/<name>.rs` and register
//! it in that module's `register`. Controls fire [`Action::Trigger`]s;
//! [`flow::next`] holds the transitions, so a screen names no other screen.
//!
//! Preview fills are `// D2RUST's own, unverified`. Mouse buttons follow the
//! front-end dispatcher's button rules.

pub mod canvas;
pub mod control;
pub mod doll;
pub mod flow;
pub mod glyphs;
#[cfg(test)]
mod input_tests;
pub mod screen;
pub mod screens;

use crate::ui::geom::Point;

pub use control::{Action, Control, ControlKind, TextRow};
pub use flow::{FlowCtx, GameLoad, Next, Trigger};
pub use screen::{FrontCtx, Placeholder, Registry, Screen, SettingsSlot};
pub use screens::ids::*;

/// A screen's name. An open set: a new screen adds a const in its module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenId(pub &'static str);

/// One front-end tick is 40 ms (C0 r1).
pub const TICK_MS: u64 = crate::rules::camera::CLIENT_TICK_MS as u64;

/// The sky palette every menu screen loads, under
/// `data\global\palette\sky\`.
pub const SKY_PALETTE: [&str; 2] = [
    r"data\global\palette\sky\pal.dat",
    r"data\global\palette\sky\pal.pl2",
];

/// Logo frame: `((now − created)/40) mod 29`; frame 29 is never shown.
pub fn logo_frame(now_ms: u64, created_ms: u64) -> u32 {
    (now_ms.saturating_sub(created_ms) / 40 % 29) as u32
}

/// The palette of the character-create screen (`fechar`): the builder
/// passes `palette\fechar\pal.dat` / `.pl2` to the loader.
pub const FECHAR_PALETTE: [&str; 2] = [
    r"data\global\palette\fechar\pal.dat",
    r"data\global\palette\fechar\pal.pl2",
];

/// The create screen's fire cel: itself the additive layer.
/// The front-end pointer art.
pub const CURSOR: &str = r"CURSOR\ohand";
/// Pseudo art of an image control that draws [`DrawItem::Dim`] (the pop-up dimming layer).
pub const DIM_MODE: u8 = 1;
/// Draw mode of the pop-up art.
pub const POPUP_MODE: u8 = 2;
pub const DIM_LAYER: &str = "::dim";
pub const FIRE: &str = r"FrontEnd\fire";
/// Draw mode of every fire overlay: additive.
pub const FIRE_MODE: u8 = 3;

/// The fire cel drawn additively over a logo half's black base.
/// File names: all four exist in `d2data.mpq`.
pub fn fire_overlay(base: &str) -> Option<&'static str> {
    match base {
        r"FrontEnd\D2logoBlackLeft" => Some(r"FrontEnd\D2logoFireLeft"),
        r"FrontEnd\D2logoBlackRight" => Some(r"FrontEnd\D2logoFireRight"),
        _ => None,
    }
}

/// How the front end ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Program exit.
    Exit,
    /// Start the game.
    GameLoad(GameLoad),
}

/// Input, in 800×600 frame coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontInput {
    Move(Point),
    /// Left button down / up.
    Down(Point),
    Up(Point),
    /// Key-down, a virtual-key code.
    Key(u16),
    /// A typed UTF-16 unit.
    Char(u16),
    /// Key-up, a virtual-key code.
    KeyUp(u16),
    /// Mouse wheel (120 per notch, positive away from the user).
    Wheel(i32),
    /// Middle button down.
    Middle,
}

/// Whether the Save folder holds a character.
pub trait SaveFolder {
    fn has_saves(&self) -> bool;
}

/// A fixed answer (tests, `--new`).
impl SaveFolder for bool {
    fn has_saves(&self) -> bool {
        *self
    }
}

/// One drawn thing, in creation order. The host maps it to its draw list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawItem {
    /// Frame `frame` of `file` (under `data\global\ui\`) at the cel position
    /// (x, bottom y).
    Art {
        file: &'static str,
        frame: u32,
        at: Point,
    },
    /// A label or text: a string id (0: `text` is literal).
    Text {
        string_id: u32,
        text: String,
        font: u16,
        at: Point,
        /// A button label: centered in the button, baseline
        /// from its height; `None` for a plain text control.
        label: Option<Label>,
        /// Text colour `k` (0 none).
        color: i32,
        /// A text control's row: laid out in the control box
        /// (`at` = the control's x, bottom y); `None`: a plain `DrawText`
        /// pen at `at`.
        boxed: Option<TextBox>,
    },
    /// A blended layer: frame `frame` of `file` drawn with draw mode `mode` (3
    /// = additive) at (x, bottom y) plus the DC6 frame offsets: the logo and
    /// title fire.
    Blend {
        file: &'static str,
        frame: u32,
        at: Point,
        mode: u8,
        /// Drawn with the frame's bottom-left at `at`, its offsets not
        /// applied (the create fire, [`FIRE_AT_BOX`]).
        boxed: bool,
    },
    /// A character-select paper doll (`doll`): the figure of class'
    /// `class`, mode `mode` built from the entry's component and colour
    /// bytes, its anchor at `at`, drawn with draw mode `draw_mode` (5; 1 for
    /// a dead hardcore character) at animation step `ticks` (40 ms ticks
    /// since the screen was built) under `epoch`.
    Doll {
        class: u8,
        mode: u8,
        components: [u8; 16],
        colours: [u8; 16],
        at: Point,
        draw_mode: u8,
        key: u32,
        epoch: u64,
        ticks: u32,
    },
    /// A cel of the in-game UI art, named as the game's panels name it
    /// ([`crate::ui::panels::UiFiles`]: `widgets\optbar`, `cursor\pentspin`,
    /// or a `*local\` label image), drawn at the cel position (x, bottom y)
    /// with draw mode `mode` (5 normal, 1 half-blended). The main menu's
    /// Options screen draws the in-game Options art through it.
    UiCel {
        file: String,
        frame: u32,
        at: Point,
        mode: u8,
    },
    /// A rectangle as the in-game UI draws it: columns `at.x.. at.x + w`, rows
    /// `at.y.. at.y + h`, palette index `color` written with draw mode `mode`
    /// (0–2 blend it with the picture under it, others are opaque). The Options
    /// sliders' dark backing.
    Shade {
        at: Point,
        w: i32,
        h: i32,
        color: u8,
        mode: u8,
    },
    /// A dark filled box (menu panels).
    Rect { at: Point, w: i32, h: i32 },
    /// A box outline.
    Border { at: Point, w: i32, h: i32 },
    /// The pop-up layer's dimming of the whole frame (`DrawBoxAlpha(0, 0, W, H,
    /// 0, 0x7F)` once a pop-up set the modal control): black through the alpha blend table of
    /// draw mode `mode` (1), every pixel but the last column and row.
    Dim { mode: u8 },
}

/// Where a button label sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub w: u16,
    pub h: u16,
    pub pressed: bool,
    /// The second label string (flag 0x40); 0 none.
    pub second: u32,
}

/// A text control's box and the row this item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextBox {
    pub w: u16,
    pub h: u16,
    /// Margins mx, my (descriptor [5], [6]).
    pub mx: i32,
    pub my: i32,
    /// Descriptor [11]: 2 centred, 0x10 right aligned, 0x20 / 1 no wrap.
    pub flags: u16,
    /// Rows added before this one (its first row's index).
    pub row: u16,
}

/// The create fire (descriptor 178 / 179, a type-3 control without an
/// anim table) is drawn with each frame's bottom-left at the control's
/// (x, bottom y), the frame offsets not applied: the create screens show the
/// fire at columns 345–454, bottom row 470; with the offsets (−34, 132) of
/// `fire.dc6` it would sit at 311–420, rows 476–602.
pub const FIRE_AT_BOX: bool = true;

pub struct FrontEnd {
    canvas: canvas::FrontCanvas,
    registry: Registry,
    saves: Box<dyn SaveFolder>,
    expansion: bool,
    current: ScreenId,
    controls: Vec<Control>,
    /// A pop-up screen's backdrop: the screen under it (its controls are inert and
    /// draw first; the difficulty box opens over the character list).
    under: Option<(ScreenId, Vec<Control>)>,
    /// Build time (ms) of each control, for timers and animation.
    built_ms: u64,
    pressed: Option<usize>,
    /// The held control had the pointer inside at the last draw (`[+0x48]`,
    /// 4a/4b): a release fires it only then.
    held_inside: bool,
    /// The point of the last mouse message (move, press or release): every
    /// one sets the cursor, which the draw tests.
    mouse: Option<Point>,
    pointer: Option<Point>,
    pending: Vec<FrontInput>,
    flow: FlowCtx,
    tick: u64,
    outcome: Option<Outcome>,
    palette: Option<[&'static str; 2]>,
    /// The game kind: 0 single player.
    pub game_kind: u8,
    /// Screens entered, in order (the log of the flow).
    pub entered: Vec<ScreenId>,
    /// Input flushes performed (C5 r4).
    pub flushes: u32,
    /// The player's settings, as the Options screen shows and edits them.
    settings: SettingsSlot,
}

impl FrontEnd {
    pub fn new(expansion: bool, saves: Box<dyn SaveFolder>, registry: Registry) -> Self {
        Self {
            canvas: Default::default(),
            registry,
            saves,
            expansion,
            current: TRADEMARK,
            controls: Vec::new(),
            under: None,
            built_ms: 0,
            pressed: None,
            held_inside: false,
            mouse: None,
            pointer: None,
            pending: Vec::new(),
            flow: FlowCtx::default(),
            tick: 0,
            outcome: None,
            palette: None,
            game_kind: 0,
            entered: Vec::new(),
            flushes: 0,
            settings: SettingsSlot::default(),
        }
    }

    pub fn canvas(&self) -> canvas::FrontCanvas {
        self.canvas
    }

    /// Reflow a live screen while retaining edits, selection and popups.
    pub fn set_canvas(&mut self, frame: crate::rules::camera::FrameSize) {
        let next = canvas::FrontCanvas::new(frame);
        if next == self.canvas {
            return;
        }
        self.canvas = next;
        self.pressed = None;
        self.held_inside = false;
        self.pending.clear();
        for p in [&mut self.mouse, &mut self.pointer].into_iter().flatten() {
            p.x = p.x.clamp(0, next.width - 1);
            p.y = p.y.clamp(0, next.height - 1);
        }
        let mut ctx = FrontCtx {
            expansion: self.expansion,
            flow: &mut self.flow,
            now_ms: self.tick * TICK_MS,
            settings: &mut self.settings,
        };
        let screen = self.registry.get_mut(self.current);
        screen.canvas_changed(next);
        if let Some(controls) = screen.relayout(&mut ctx) {
            self.controls = controls;
        }
        screen.sync(&mut self.controls);
        if let Some((id, controls)) = &mut self.under {
            let screen = self.registry.get_mut(*id);
            screen.canvas_changed(next);
            if let Some(next) = screen.relayout(&mut ctx) {
                *controls = next;
            }
            screen.sync(controls);
        }
    }

    /// Hand the front end the settings in force (the host does this when
    /// the front end starts, so the Options screen shows what the game uses).
    pub fn set_settings(&mut self, s: crate::app::config::Settings) {
        self.settings.current = s;
        self.settings.changed = false;
    }

    /// Whether the sound device is up (false under `--no-sound`): the
    /// Options screen's sound rows are off without it.
    pub fn set_sound_device(&mut self, up: bool) {
        self.settings.sound_device = up;
    }

    /// The monitor the window is on: its physical size and refresh rate,
    /// when the platform reports them (the Window Size and Frame Limit
    /// rows depend on them).
    pub fn set_display(&mut self, monitor: Option<(u32, u32)>, refresh_hz: Option<u32>) {
        self.settings.monitor = monitor;
        self.settings.refresh_hz = refresh_hz;
    }

    /// The window as the platform reports it now, for the Options screen's
    /// Video page.
    pub fn set_window(&mut self, window: crate::ui::original::options_menu::WindowFacts) {
        self.settings.window = window;
    }

    /// The settings in force (the Options screen's values).
    pub fn settings(&self) -> crate::app::config::Settings {
        self.settings.current
    }

    /// The settings after the Options screen changed them, once per change.
    pub fn take_settings_change(&mut self) -> Option<crate::app::config::Settings> {
        std::mem::take(&mut self.settings.changed).then_some(self.settings.current)
    }

    /// A front end with every screen module registered.
    pub fn with_screens(expansion: bool, saves: Box<dyn SaveFolder>) -> Self {
        let mut reg = Registry::default();
        screens::register_all(&mut reg);
        Self::new(expansion, saves, reg)
    }

    /// Enter the front end. `first_entry`: the first in this process: flush
    /// the input and open the trademark screen; later entries go straight to
    /// the main menu.
    pub fn start(&mut self, first_entry: bool) {
        if !first_entry {
            self.enter(MAIN_MENU);
            return;
        }
        self.flush_input();
        self.enter(TRADEMARK);
    }

    /// Remove every pending input unprocessed (C5 r4).
    pub fn flush_input(&mut self) {
        self.pending.clear();
        self.flushes += 1;
    }

    pub fn input(&mut self, ev: FrontInput) {
        self.pending.push(ev);
    }

    pub fn current(&self) -> ScreenId {
        self.current
    }

    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    /// The palette files in use; `None` until a screen loaded one.
    pub fn palette(&self) -> Option<[&'static str; 2]> {
        self.palette
    }

    pub fn now_ms(&self) -> u64 {
        self.tick * TICK_MS
    }

    /// Facts the flow table reads; a test or host sets `difficulties_open`.
    pub fn flow_mut(&mut self) -> &mut FlowCtx {
        &mut self.flow
    }

    /// One 40 ms tick (C0): process the pending input, then timers and the
    /// screen's own update.
    pub fn tick(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        self.tick += 1;
        for ev in std::mem::take(&mut self.pending) {
            if self.outcome.is_some() {
                break;
            }
            let flushes = self.flushes;
            self.handle(ev);
            if self.flushes != flushes {
                // C5 r4: a flush drops the rest of this tick's input.
                self.registry.get_mut(self.current).sync(&mut self.controls);
                break;
            }
            self.registry.get_mut(self.current).sync(&mut self.controls);
        }
        self.run_timers();
        if self.outcome.is_none() {
            let mut ctx = FrontCtx {
                expansion: self.expansion,
                flow: &mut self.flow,
                now_ms: self.tick * TICK_MS,
                settings: &mut self.settings,
            };
            if let Some(t) = self.registry.get_mut(self.current).tick(&mut ctx) {
                self.trigger(t);
            }
            self.registry.get_mut(self.current).sync(&mut self.controls);
        }
        // The draw after this tick re-tests the held button against the pointer.
        if let (Some(i), Some(p)) = (self.pressed, self.mouse) {
            self.held_inside = self
                .controls
                .get(i)
                .is_some_and(|c| self.canvas.control(self.current, c).contains(p));
        }
    }

    /// Fire a flow trigger as the current screen would.
    pub fn trigger(&mut self, t: Trigger) {
        if self.outcome.is_some() {
            return;
        }
        if t == Trigger::SinglePlayer {
            // Game kind:= single player; then the save scan.
            self.game_kind = 0;
            self.flow.saves_found = self.saves.has_saves();
        }
        if t == Trigger::Rebuild {
            self.enter(self.current);
            return;
        }
        match flow::next(self.current, t, self.flow) {
            Next::Screen(s) => self.enter(s),
            Next::Exit => self.outcome = Some(Outcome::Exit),
            Next::GameLoad(g) => self.outcome = Some(Outcome::GameLoad(g)),
            Next::Stay => {}
        }
    }

    /// Jump to a screen without the flow (hosts and tests; screens the flow
    /// table does not reach yet, such as Configure Controls).
    pub fn goto(&mut self, id: ScreenId) {
        self.enter(id);
    }

    fn enter(&mut self, id: ScreenId) {
        let popup = self.current == screens::ids::CHAR_SELECT && id == screens::ids::DIFFICULTY;
        self.under = popup.then(|| (self.current, std::mem::take(&mut self.controls)));
        self.current = id;
        self.pressed = None;
        self.built_ms = self.tick * TICK_MS;
        let screen = self.registry.get_mut(id);
        screen.canvas_changed(self.canvas);
        let mut ctx = FrontCtx {
            expansion: self.expansion,
            flow: &mut self.flow,
            now_ms: self.built_ms,
            settings: &mut self.settings,
        };
        self.controls = screen.build(&mut ctx);
        screen.sync(&mut self.controls);
        if popup {
            for c in &mut self.controls {
                c.top = true;
            }
        }
        if let Some(p) = screen.palette() {
            self.palette = Some(p);
        }
        self.entered.push(id);
    }

    fn fire(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Trigger(t) => self.trigger(t),
            Action::Custom(id) => {
                let mut ctx = FrontCtx {
                    expansion: self.expansion,
                    flow: &mut self.flow,
                    now_ms: self.tick * TICK_MS,
                    settings: &mut self.settings,
                };
                if let Some(t) = self.registry.get_mut(self.current).action(&mut ctx, id) {
                    self.trigger(t);
                }
            }
        }
    }

    /// The pointer read at draw time (the draw reads the cursor every
    /// frame, not only on a tick): the cel shows before the first tick.
    pub fn seen_pointer(&mut self, p: Point) {
        self.pointer = Some(p);
    }

    /// The pointer's last position (800×600), if it was seen.
    pub fn pointer(&self) -> Option<Point> {
        self.pointer
    }

    /// The clickable control under `p`: the topmost (last created), except
    /// within a screen's [`Screen::first_wins`] run, where the first
    /// created one under `p` takes the click (B003).
    fn hit(&self, p: Point) -> Option<usize> {
        let clickable = |c: &Control| {
            c.enabled
                && !c.inert
                && c.visible
                && matches!(
                    c.kind,
                    ControlKind::Image | ControlKind::Button | ControlKind::AnimImage
                )
                && c.action != Action::None
                && self.canvas.control(self.current, c).contains(p)
        };
        let top = self.controls.iter().rposition(clickable)?;
        match self.registry.first_wins(self.current) {
            Some(run) if run.contains(&top) => run
                .clone()
                .find(|&i| self.controls.get(i).is_some_and(clickable))
                .or(Some(top)),
            _ => Some(top),
        }
    }

    fn handle(&mut self, ev: FrontInput) {
        match ev {
            FrontInput::Move(p) => {
                self.pointer = Some(p);
                self.mouse = Some(p);
                let mut ctx = FrontCtx {
                    expansion: self.expansion,
                    flow: &mut self.flow,
                    now_ms: self.tick * TICK_MS,
                    settings: &mut self.settings,
                };
                self.registry
                    .get_mut(self.current)
                    .pointer(&mut ctx, self.canvas.local(p));
            }
            FrontInput::Wheel(d) => {
                let mut ctx = FrontCtx {
                    expansion: self.expansion,
                    flow: &mut self.flow,
                    now_ms: self.tick * TICK_MS,
                    settings: &mut self.settings,
                };
                if let Some(t) = self.registry.get_mut(self.current).wheel(&mut ctx, d) {
                    self.trigger(t);
                }
            }
            FrontInput::KeyUp(k) => {
                let mut ctx = FrontCtx {
                    expansion: self.expansion,
                    flow: &mut self.flow,
                    now_ms: self.tick * TICK_MS,
                    settings: &mut self.settings,
                };
                self.registry.get_mut(self.current).key_up(&mut ctx, k);
            }
            FrontInput::Middle => {
                let mut ctx = FrontCtx {
                    expansion: self.expansion,
                    flow: &mut self.flow,
                    now_ms: self.tick * TICK_MS,
                    settings: &mut self.settings,
                };
                self.registry.get_mut(self.current).middle_down(&mut ctx);
            }
            // 4b: the press goes to the control under
            // the pointer; a toggle fires now (4a), a plain button is held.
            FrontInput::Down(p) => {
                self.mouse = Some(p);
                match self.hit(p) {
                    Some(i) if self.controls[i].on_press => {
                        self.pressed = None;
                        let a = self.controls[i].action;
                        self.fire(a);
                    }
                    down => {
                        self.pressed = down;
                        self.held_inside = down.is_some();
                    }
                }
                self.screen_mouse(true, p);
            }
            // The release fires the held button when the pointer was inside
            // it at the last draw, not tested at the release point.
            FrontInput::Up(p) => {
                self.mouse = Some(p);
                if let Some(i) = self.pressed.take() {
                    if std::mem::take(&mut self.held_inside) {
                        let a = self.controls[i].action;
                        self.fire(a);
                    }
                }
                self.screen_mouse(false, p);
            }
            // The topmost (last created) control holding the key: a pop-up
            // (list A) is asked before the screen under it (list B).
            FrontInput::Key(k) => {
                let hit = self
                    .controls
                    .iter()
                    .rev()
                    .find(|c| c.enabled && !c.inert && c.action != Action::None && c.takes_key(k))
                    .map(|c| c.action);
                match hit {
                    Some(a) => self.fire(a),
                    None => {
                        let mut ctx = FrontCtx {
                            expansion: self.expansion,
                            flow: &mut self.flow,
                            now_ms: self.tick * TICK_MS,
                            settings: &mut self.settings,
                        };
                        if let Some(t) = self.registry.get_mut(self.current).key(&mut ctx, k) {
                            self.trigger(t);
                        }
                    }
                }
            }
            FrontInput::Char(u) => {
                let mut ctx = FrontCtx {
                    expansion: self.expansion,
                    flow: &mut self.flow,
                    now_ms: self.tick * TICK_MS,
                    settings: &mut self.settings,
                };
                self.registry.get_mut(self.current).char(&mut ctx, u);
            }
        }
    }

    /// A mouse button event for screens that lay out their own rows.
    fn screen_mouse(&mut self, down: bool, p: Point) {
        if self.outcome.is_some() {
            return;
        }
        let mut ctx = FrontCtx {
            expansion: self.expansion,
            flow: &mut self.flow,
            now_ms: self.tick * TICK_MS,
            settings: &mut self.settings,
        };
        if let Some(t) =
            self.registry
                .get_mut(self.current)
                .mouse(&mut ctx, down, self.canvas.local(p))
        {
            self.trigger(t);
        }
    }

    /// Once `⌊now/1000⌋ − ⌊start/1000⌋ > seconds`, the timer
    /// fires on every update until the screen is torn down.
    fn run_timers(&mut self) {
        let now = self.tick * TICK_MS / 1000;
        let start = self.built_ms / 1000;
        let due = self
            .controls
            .iter()
            .find(|c| c.kind == ControlKind::Timer && now - start > u64::from(c.seconds))
            .map(|c| c.action);
        if let Some(a) = due {
            self.fire(a);
        }
    }

    /// The current screen's per-tick items (credits rows, hero frames,...),
    /// drawn after [`FrontEnd::draw`]. `adv`: text width in a font.
    pub fn overlay(&mut self, adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        let now = self.now_ms();
        let mut out = Vec::new();
        if let Some((id, _)) = &self.under {
            out = self.registry.get_mut(*id).overlay(now, adv);
        }
        out.extend(self.registry.get_mut(self.current).overlay(now, adv));
        for item in &mut out {
            self.canvas.shift_item(item);
        }
        for c in self
            .controls
            .iter()
            .filter(|c| c.visible && c.late && !c.top)
        {
            let c = self.canvas.control(self.current, c);
            if let Some(file) = c.art {
                out.push(DrawItem::Blend {
                    file,
                    frame: logo_frame(now, self.built_ms),
                    at: Point::new(c.x, c.y),
                    mode: FIRE_MODE,
                    boxed: FIRE_AT_BOX,
                });
            }
        }
        // Pop-up controls (list A) draw above the screen's own items, dimmed
        // once: a pop-up that carries its own dim layer is not dimmed again.
        let top = || self.controls.iter().filter(|c| c.visible && c.top);
        if top().next().is_some() && !top().any(|c| c.art == Some(DIM_LAYER)) {
            out.push(DrawItem::Dim { mode: DIM_MODE });
        }
        out.extend(self.draw_controls(|c| c.top));
        out
    }

    /// The pointer cel: frame 0 of
    /// `CURSOR\ohand` at the pointer clamped to >= 0, one row up; `None`
    /// until the pointer was seen.
    pub fn cursor_item(&self) -> Option<DrawItem> {
        let p = self.pointer?;
        Some(DrawItem::Art {
            file: CURSOR,
            frame: 0,
            at: Point::new(p.x.max(0), p.y.max(0) - 1),
        })
    }

    /// The frame's draw items, in creation order.
    pub fn draw(&self) -> Vec<DrawItem> {
        let mut out = match &self.under {
            Some((id, v)) => self.draw_list(*id, v, None, |c| !c.late),
            None => Vec::new(),
        };
        out.extend(self.draw_controls(|c| !c.late && !c.top));
        out
    }

    fn draw_controls(&self, pick: impl Fn(&Control) -> bool) -> Vec<DrawItem> {
        // Pressed frames only while the pointer is inside.
        let held = self.pressed.filter(|_| self.held_inside);
        self.draw_list(self.current, &self.controls, held, pick)
    }

    fn draw_list(
        &self,
        screen: ScreenId,
        list: &[Control],
        pressed_ix: Option<usize>,
        pick: impl Fn(&Control) -> bool,
    ) -> Vec<DrawItem> {
        let mut out = Vec::new();
        let now = self.now_ms();
        for (i, c) in list
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible && pick(c))
        {
            let mapped = self.canvas.control(screen, c);
            let c = &mapped;
            let pressed = pressed_ix == Some(i) && c.enabled;
            let at = Point::new(c.x, c.y);
            match c.kind {
                ControlKind::Image => {
                    if c.art == Some(DIM_LAYER) {
                        out.push(DrawItem::Dim { mode: DIM_MODE });
                    } else if let (Some(file), Some(frame)) = (c.art, c.cel) {
                        out.push(DrawItem::Art { file, frame, at });
                    } else if let Some(file) = c.art {
                        let tiles = image_tiles(file, c.x, c.y, c.w, c.h);
                        match c.blend {
                            None => out.extend(tiles),
                            Some(mode) => out.extend(tiles.into_iter().map(|t| match t {
                                DrawItem::Art { file, frame, at } => DrawItem::Blend {
                                    file,
                                    frame,
                                    at,
                                    mode,
                                    boxed: false,
                                },
                                other => other,
                            })),
                        }
                    }
                }
                ControlKind::AnimImage => {
                    if let Some(file) = c.art {
                        let frame = logo_frame(now, self.built_ms);
                        if file == FIRE {
                            out.push(DrawItem::Blend {
                                file,
                                frame,
                                at,
                                mode: FIRE_MODE,
                                boxed: FIRE_AT_BOX,
                            });
                        } else {
                            out.push(DrawItem::Art { file, frame, at });
                            if let Some(fire) = fire_overlay(file) {
                                out.push(DrawItem::Blend {
                                    file: fire,
                                    frame,
                                    at,
                                    mode: FIRE_MODE,
                                    boxed: false,
                                });
                            }
                        }
                    }
                }
                ControlKind::Button => {
                    if let Some(file) = c.art {
                        let tiles = control::button_tiles(c.w, c.h);
                        // A wide button's art comes in 256-px tiles, drawn
                        // left to right.
                        for t in 0..tiles {
                            let frame = if c.checked {
                                1
                            } else {
                                control::button_frame(t, tiles, pressed, c.enabled, false)
                            };
                            let at = Point::new(c.x + 256 * t as i32, c.y);
                            if c.enabled {
                                out.push(DrawItem::Art { file, frame, at });
                            } else {
                                // A disabled button draws its up frames
                                // darkened (draw mode 1).
                                out.push(DrawItem::Blend {
                                    file,
                                    frame,
                                    at,
                                    mode: 1,
                                    boxed: false,
                                });
                            }
                        }
                    }
                    // A label is a string id, or literal text on D2RUST's own
                    // buttons (string id 0).
                    if c.string_id != 0 || c.text.is_some() {
                        out.push(DrawItem::Text {
                            string_id: c.string_id,
                            // Only the gateway label carries an argument.
                            text: c.text.clone().unwrap_or_default(),
                            font: control::label_font(c.h),
                            at,
                            label: Some(Label {
                                w: c.w,
                                h: c.h,
                                pressed,
                                second: c.second_label,
                            }),
                            color: 0,
                            boxed: None,
                        });
                    }
                }
                ControlKind::Text => out.extend(text_rows(c)),
                ControlKind::EditBox | ControlKind::Timer => {}
            }
        }
        out
    }
}

/// The rows of a text control, one item each: a control nothing
/// was added to draws nothing. An empty first row followed by others still
/// takes its row (it is added, and draws nothing).
fn text_rows(c: &Control) -> Vec<DrawItem> {
    let first = (c.has_text() || !c.more_rows.is_empty()).then(|| TextRow {
        string_id: c.string_id,
        text: c.text.clone().unwrap_or_default(),
        color: c.color,
    });
    first
        .into_iter()
        .chain(c.more_rows.iter().cloned())
        .enumerate()
        .map(|(i, r)| DrawItem::Text {
            string_id: r.string_id,
            text: r.text,
            font: c.font,
            at: Point::new(c.x, c.y),
            label: None,
            color: r.color,
            boxed: Some(TextBox {
                w: c.w,
                h: c.h,
                mx: c.margin.0,
                my: c.margin.1,
                flags: c.flags,
                // D2RUST's own: rows a wrap adds before this one are not
                // counted (every multi-row control here has short rows).
                row: i as u16,
            }),
        })
        .collect()
}

/// An image control's cel cut in 256 × 256 tiles (e.g. an 800 ×
/// 600 background is 4 × 3 frames, left to right, top to bottom): one draw per
/// tile. Tile row r of the full rows is drawn at cel Y = (y − h + 1) + 256 (r +
/// 1); the last row (partial, or the only one) is drawn at the control's bottom
/// edge y. A full row therefore covers rows y − h +
/// 2 … (one below the control box top: screen row 0 stays
/// black), and the last row of an 800 × 600 image (88 rows) ends on row y =
/// 599.
fn image_tiles(file: &'static str, x: i32, y: i32, w: u16, h: u16) -> Vec<DrawItem> {
    let (w, h) = (i32::from(w.max(1)), i32::from(h.max(1)));
    let cols = (w + 255) / 256;
    let rows = (h + 255) / 256;
    let top = y - h + 1;
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for r in 0..rows {
        let row_h = (h - 256 * r).min(256);
        let mut bottom = top + 256 * r + row_h;
        if r == rows - 1 {
            bottom = y;
        }
        for c in 0..cols {
            out.push(DrawItem::Art {
                file,
                frame: (r * cols + c) as u32,
                at: Point::new(x + 256 * c, bottom),
            });
        }
    }
    out
}

#[cfg(test)]
mod canvas_tests;
