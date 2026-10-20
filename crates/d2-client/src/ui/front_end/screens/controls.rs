//! CONFIGURE CONTROLS (`UI_CONFIG`, ui 11).
//!
//! [`ConfigureControls`] is the whole screen as plain state: 15 visible
//! rows of the key-config table, the selected row and column, the blinking
//! edit cell, the one-press latch, the two-key rule, Default / Accept /
//! Cancel. [`ConfigureControls::draw_list`] gives the screen layout with
//! colours. [`ControlsScreen`] adapts it to the front-end shell: a control
//! per key (capture), per visible key cell (click-to-edit) and per button.
//!
//! Persistence is D2RUST's own: Accept writes `controls.toml` over the `dev`
//! preset; [`saved_bindings`] is what play input loads. Everything not in the
//! specs is `D2RUST's own, unverified`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::controls::keymap::{action_of_cmd, key_to_vk, vk_to_key};
use crate::controls::original::{
    assign_key, menu_table, AssignError, BindingTable, MenuRow, SEPARATOR, UNBOUND,
};
use crate::controls::{self, Action as Act, Bindings, Context, ControlsFile, Key, Preset};
use crate::ui::front_end::control::{Action, Control, ControlKind};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::{DrawItem, Registry};
use crate::ui::geom::Point;

use super::ids::CONTROLS;

// ---- layout constants, 800 × 600 frame -----------------------

pub(crate) const W: i32 = 800;
pub(crate) const H: i32 = 600;
pub(crate) const M: i32 = (W - 620) / 2;
pub(crate) const T: i32 = (H - 40 - 420) / 2;
pub(crate) const C: i32 = (620 - 49) / 3;
pub(crate) const VISIBLE: usize = 15;
/// FontInGameChat.
pub const FONT: u16 = 13;
/// The error message stays for 2,000 ms.
const MESSAGE_MS: u64 = 2000;

/// Text colours.
pub mod color {
    pub const WHITE: u8 = 0;
    pub const RED: u8 = 1;
    pub const GREEN: u8 = 2;
    pub const BLUE: u8 = 3;
    pub const GOLD: u8 = 4;
    pub const GREY: u8 = 5;
}

/// A sound request (named only: sound is deferred).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    CursorPass,
    CursorSelect,
}

/// What a key or button did to the screen as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Stay,
    /// Accept: bindings kept (the host persists with [`ConfigureControls::save`]).
    Accept,
    /// Cancel: the snapshot was restored.
    Cancel,
}

/// One drawn thing of the screen, in draw order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgDraw {
    /// Filled rectangle (colour 0, mode 1: the dark box / selection).
    Rect { x: i32, y: i32, w: i32, h: i32 },
    /// `menu\boxpieces` border around the rectangle.
    Border { x: i32, y: i32, w: i32, h: i32 },
    /// A frame of `data\global\ui\MENU\textslid`.
    Slider { frame: u32, x: i32, y: i32 },
    /// Text; `string_id` 0 means literal `text`.
    Text {
        string_id: u32,
        text: String,
        x: i32,
        y: i32,
        color: u8,
    },
}

/// The screen state.
#[derive(Clone, Debug)]
pub struct ConfigureControls {
    expansion: bool,
    rows: Vec<MenuRow>,
    table: BindingTable,
    snapshot: BindingTable,
    top: usize,
    selected: usize,
    /// 1 = slot 1 "Key/Button One" (initial), 0 = slot 0.
    column: i32,
    latch: u8,
    editing: bool,
    message: Option<(AssignError, u64)>,
    /// The blink counter: one step per 40 ms of the draw clock.
    blink: crate::ui::ui_clock::StepCounter,
    /// Sounds requested since the last [`ConfigureControls::take_sounds`].
    sounds: Vec<Sound>,
}

impl ConfigureControls {
    /// Open: snapshot the table, top row and selected row 0.
    pub fn open(expansion: bool, table: BindingTable) -> Self {
        Self {
            expansion,
            rows: menu_table(expansion),
            snapshot: table.clone(),
            table,
            top: 0,
            selected: 0,
            column: 1,
            latch: 1,
            editing: false,
            message: None,
            blink: Default::default(),
            sounds: Vec::new(),
        }
    }

    pub fn table(&self) -> &BindingTable {
        &self.table
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn top(&self) -> usize {
        self.top
    }
    pub fn column(&self) -> i32 {
        self.column
    }
    pub fn editing(&self) -> bool {
        self.editing
    }
    pub fn latch(&self) -> u8 {
        self.latch
    }
    pub fn rows(&self) -> &[MenuRow] {
        &self.rows
    }
    pub fn take_sounds(&mut self) -> Vec<Sound> {
        std::mem::take(&mut self.sounds)
    }
    pub fn message(&self, now_ms: u64) -> Option<AssignError> {
        self.message
            .filter(|&(_, at)| now_ms.saturating_sub(at) < MESSAGE_MS)
            .map(|(e, _)| e)
    }

    fn max_top(&self) -> usize {
        self.rows.len().saturating_sub(VISIBLE)
    }

    /// Why a click on button `name` (`default`, `scroll_up`,
    /// `scroll_down`) changes nothing now, or `None`: the table is
    /// already the defaults (only the select sound plays) or the list is
    /// already at that end. While a key is being edited every button
    /// ends the edit.
    pub fn idle(&self, name: &str) -> Option<&'static str> {
        if self.editing {
            return None;
        }
        match name {
            "default" if self.table == BindingTable::defaults() => {
                Some("the bindings are already the defaults")
            }
            "scroll_up" if self.top == 0 => Some("the list already shows its first row"),
            "scroll_down" if self.top >= self.max_top() => {
                Some("the list already shows its last row")
            }
            _ => None,
        }
    }

    fn is_sep(&self, row: usize) -> bool {
        self.rows[row].cmd == SEPARATOR
    }

    fn cmd(&self) -> i32 {
        self.rows[self.selected].cmd
    }

    /// Down / Up: ±1 skipping separators, no wrap, scrolling to keep the
    /// selection among the 15 visible rows.
    fn step(&mut self, down: bool) {
        let mut r = self.selected;
        loop {
            if down {
                if r + 1 >= self.rows.len() {
                    return;
                }
                r += 1;
            } else {
                if r == 0 {
                    return;
                }
                r -= 1;
            }
            if !self.is_sep(r) {
                break;
            }
        }
        self.selected = r;
        if r < self.top {
            self.top = r;
        } else if r >= self.top + VISIBLE {
            self.top = r + 1 - VISIBLE;
        }
        self.sounds.push(Sound::CursorPass);
    }

    fn start_edit(&mut self) {
        self.editing = true;
        self.sounds.push(Sound::CursorSelect);
        self.latch = 0;
    }

    /// A key went down. `repeat`: auto-repeat. `now_ms` stamps an error.
    pub fn key_down(&mut self, vk: u16, repeat: bool, now_ms: u64) -> Done {
        if self.editing {
            return self.edit_key(vk, repeat, now_ms);
        }
        match vk {
            0x28 => self.step(true),
            0x26 => self.step(false),
            0x25 if self.column == 0 => {
                self.column = 1;
                self.sounds.push(Sound::CursorPass);
            }
            0x27 if self.column == 1 => {
                self.column = 0;
                self.sounds.push(Sound::CursorPass);
            }
            // Enter, Delete, Backspace, Esc, Space share the latch.
            13 | 0x2E | 8 | 27 | 32 => {
                if self.latch == 0 {
                    self.latch = 1;
                } else if !repeat {
                    match vk {
                        13 => {
                            if !self.is_sep(self.selected) {
                                self.start_edit();
                            }
                        }
                        0x2E | 8 => {
                            if !self.is_sep(self.selected) {
                                let c = self.cmd();
                                self.table.unbind(c, self.column);
                                self.sounds.push(Sound::CursorSelect);
                            }
                        }
                        _ => return self.cancel(),
                    }
                }
            }
            _ => {}
        }
        Done::Stay
    }

    /// A key went up (only the editing state reacts).
    pub fn key_up(&mut self, vk: u16, now_ms: u64) {
        if !self.editing {
            return;
        }
        if self.latch == 0 {
            self.latch = 1;
        } else {
            self.assign(vk, now_ms);
        }
    }

    fn edit_key(&mut self, vk: u16, repeat: bool, now_ms: u64) -> Done {
        if repeat {
            return Done::Stay;
        }
        if vk == 27 {
            self.editing = false;
            self.sounds.push(Sound::CursorPass);
            self.latch = 0;
        } else {
            self.assign(vk, now_ms);
        }
        Done::Stay
    }

    /// Assign K to (selected command, column) with the two-key rule.
    fn assign(&mut self, k: u16, now_ms: u64) {
        let (c, s) = (self.cmd(), self.column);
        match assign_key(&mut self.table, c, s, k) {
            Ok(()) => {
                self.editing = false;
                self.sounds.push(Sound::CursorSelect);
                self.latch = if matches!(k, 13 | 0x2E | 8 | 32) {
                    0
                } else {
                    1
                };
            }
            Err(e) => {
                self.latch = 1;
                self.message = Some((e, now_ms));
            }
        }
    }

    /// Wheel (not editing): n = delta / 120 scrolls the view by −2n rows.
    pub fn wheel(&mut self, delta: i32) {
        let n = delta / 120;
        if self.editing || n == 0 {
            return;
        }
        let top = (self.top as i32 - 2 * n).clamp(0, self.max_top() as i32) as usize;
        self.top = top;
        self.sounds.push(Sound::CursorPass);
    }

    /// A click on visible row `vis` (0..15), column `col`: stops an edit in
    /// progress, selects the cell and (release on the same cell) edits it.
    pub fn click_cell(&mut self, vis: usize, col: i32) {
        self.editing = false;
        let row = self.top + vis;
        if vis >= VISIBLE || row >= self.rows.len() || self.is_sep(row) {
            return;
        }
        self.selected = row;
        self.column = col;
        self.start_edit();
    }

    /// Scroll arrows: top ∓ 1.
    pub fn scroll(&mut self, down: bool) {
        self.editing = false;
        self.top = if down {
            (self.top + 1).min(self.max_top())
        } else {
            self.top.saturating_sub(1)
        };
    }

    /// Default: the compiled defaults; nothing is written.
    pub fn default_all(&mut self) {
        self.editing = false;
        self.table = BindingTable::defaults();
        self.sounds.push(Sound::CursorSelect);
    }

    /// Cancel (also Esc / Space): restore the snapshot.
    pub fn cancel(&mut self) -> Done {
        self.editing = false;
        self.table = self.snapshot.clone();
        Done::Cancel
    }

    /// Accept: keep the table (the host saves it).
    pub fn accept(&mut self) -> Done {
        self.editing = false;
        self.sounds.push(Sound::CursorSelect);
        Done::Accept
    }

    /// Name of the key held by (cmd, slot): "None" when unbound.
    pub fn key_name(&self, cmd: i32, slot: i32) -> String {
        match self.table.key_of(cmd, slot) {
            UNBOUND => "None".into(),
            vk => vk_name(vk),
        }
    }

    /// Key colour: the selected row is blue in both slots
    /// while a key is awaited, else in the selected slot; a command with
    /// no key is red; otherwise a bound slot is gold and an empty one grey.
    fn key_color(&self, row: usize, slot: i32) -> u8 {
        let cmd = self.rows[row].cmd;
        if row == self.selected && (self.editing || slot == self.column) {
            return color::BLUE;
        }
        if !self.table.is_bound(cmd, 1) && !self.table.is_bound(cmd, 0) {
            color::RED
        } else if self.table.is_bound(cmd, slot) {
            color::GOLD
        } else {
            color::GREY
        }
    }

    /// The screen layout at 800 × 600. The edit cell blinks on a counter
    /// that steps every 40 ms of `now_ms` from the first draw (a 640 ms
    /// period, hidden while counter & 15 ≤ 4: 200 ms), at any frame rate.
    pub fn draw_list(&mut self, now_ms: u64, pointer: Option<(i32, i32)>) -> Vec<CfgDraw> {
        let mut out = Vec::new();
        let text = |id: u32, t: &str, x: i32, y: i32, c: u8| CfgDraw::Text {
            string_id: id,
            text: t.to_string(),
            x,
            y,
            color: c,
        };
        out.push(CfgDraw::Rect {
            x: M,
            y: T,
            w: W - 2 * M,
            h: 369,
        });
        out.push(CfgDraw::Border {
            x: M,
            y: T + 3,
            w: W - 2 * M - 3,
            h: 365,
        });
        out.push(CfgDraw::Border {
            x: M,
            y: T + 323,
            w: W - 2 * M - 3,
            h: 45,
        });
        for (i, id) in [3921u32, 3922, 3923].into_iter().enumerate() {
            out.push(text(id, "", M + 18 + C * i as i32, T + 30, color::GOLD));
        }
        let blink_hidden = self.blink.count(now_ms) & 15 <= 4;
        for r in 0..VISIBLE {
            let row = self.top + r;
            let Some(mr) = self.rows.get(row).copied() else {
                break;
            };
            let y = T + 52 + 18 * r as i32;
            if row == self.selected {
                out.push(CfgDraw::Rect {
                    x: M + 18,
                    y: T + 35 + 18 * r as i32,
                    w: 3 * C,
                    h: 18,
                });
            }
            if mr.cmd == SEPARATOR {
                continue;
            }
            let lc = if row == self.selected && self.editing {
                color::BLUE
            } else {
                color::WHITE
            };
            out.push(text(mr.string_id, "", M + 30, y, lc));
            for (slot, x) in [(1, M + 30 + C), (0, M + 30 + 2 * C)] {
                if self.editing && row == self.selected && slot == self.column && blink_hidden {
                    continue;
                }
                // The long name:
                // a string id the host resolves, or the key's low byte.
                let (id, name) =
                    match crate::controls::key_names::long_name(self.table.key_of(mr.cmd, slot)) {
                        crate::controls::key_names::KeyName::Id(id) => {
                            (u32::from(id), String::new())
                        }
                        crate::controls::key_names::KeyName::Unit(u) => {
                            (0, String::from_utf16_lossy(&[u]))
                        }
                    };
                out.push(text(id, &name, x, y, self.key_color(row, slot)));
            }
        }
        self.scroll_bar(&mut out);
        let labels = [
            (3974u32, color::RED),
            (3972, color::BLUE),
            (3973, color::GREEN),
        ];
        for (i, (id, hot)) in labels.into_iter().enumerate() {
            let c = M + 206 * i as i32 + 103;
            let over = pointer.is_some_and(|(px, py)| {
                (c - BTN_HALF[i]..=c + BTN_HALF[i]).contains(&px)
                    && (T + 329..T + 329 + BTN_H).contains(&py)
            });
            // Text x = centre − w/2; the width is the host's font measure: the
            // centre is passed as x and the host centres.
            out.push(text(
                id,
                "",
                c,
                T + 351,
                if over { hot } else { color::GOLD },
            ));
        }
        if let Some(e) = self.message(now_ms) {
            out.push(text(e.string_id(), "", W / 2, T + 399, color::RED));
        }
        out
    }

    fn scroll_bar(&self, out: &mut Vec<CfgDraw>) {
        let x = W - M - 31;
        for k in 0..TRACK_FRAMES {
            out.push(CfgDraw::Slider {
                frame: 13,
                x,
                y: T + 59 + 12 * k,
            });
        }
        // Track, thumb, then the arrows: the game's draw
        // order.
        let s: i32 = if self.expansion { 5 } else { 6 };
        let max_top = self.max_top().max(1) as i32;
        let ty = (246 - 12 * s) * self.top as i32 / max_top + 47 + T;
        for j in 0..s {
            out.push(CfgDraw::Slider {
                frame: 14,
                x: W - M - 32,
                y: ty + 12 + 12 * j,
            });
        }
        out.push(CfgDraw::Slider {
            frame: 11,
            x,
            y: T + 47,
        });
        out.push(CfgDraw::Slider {
            frame: 10,
            x,
            y: T + 305,
        });
    }
}

/// Track frames drawn (bottoms T + 59 + 12k). Measured
/// (`traces/frontend/frontend-options/frontend-0006.json`): the track shows
/// down to y T + 293 (just above the down arrow at T + 294..T + 305) and
/// nothing below the arrow, so k = 0..20: 21 frames (20 would leave T + 288..T +
/// 293 bare).
const TRACK_FRAMES: i32 = 21;
/// Half width of a button hit box, c ± (w/2 + 10) with w the label's width,
/// both ends in. Measured on the English labels: Cancel x 161..=225
/// (c 193), Default x 364..=434 (c 399), Accept x 573..=637 (c 605), so the
/// half widths are 32, 35 and 32 for the English install;
/// deriving them from the label width is
/// a D2RUST refactor that must give the same numbers.
pub(crate) const BTN_HALF: [i32; 3] = [32, 35, 32];
/// Rows of a button hit box: T + 329..= T + 367 (measured, both ends in: y 399
/// and 437 hover Cancel, 398 and 438 do not).
pub(crate) const BTN_H: i32 = 39;
/// Width of the block a button label is centred in (the host's text
/// placement; wider than any label).
pub(crate) const BTN_TEXT_BLOCK: i32 = 240;

/// Display name of a key (D2RUST's own: the game's names are localised).
pub fn vk_name(vk: u16) -> String {
    match vk_to_key(vk) {
        Some(k) => k.name().to_string(),
        None => format!("0x{vk:02X}"),
    }
}

// ---- controls.toml bridge (D2RUST's own) ----------------------------------

/// The table → effective play bindings: the `original` preset with every
/// command replaced by its (slot 1, slot 0) keys; the actions that are
/// not commands lose an input a command in the same context now holds.
pub fn table_to_bindings(t: &BindingTable) -> Bindings {
    let mut b = Preset::Original.bindings().expect("original preset");
    let mut mapped = Vec::new();
    let mut held: Vec<(Context, Key)> = Vec::new();
    for cmd in 0..57 {
        let Some(a) = action_of_cmd(cmd) else {
            continue;
        };
        let keys: Vec<Key> = [1, 0]
            .into_iter()
            .map(|s| t.key_of(cmd, s))
            .filter(|&vk| vk != UNBOUND)
            .filter_map(vk_to_key)
            .collect();
        for &k in &keys {
            held.push((a.context(), k));
        }
        b.set(a, &keys);
        mapped.push(a);
    }
    for &a in Act::ALL.iter().filter(|a| !mapped.contains(a)) {
        let keep: Vec<Key> = b
            .inputs(a)
            .iter()
            .copied()
            .filter(|k| !held.contains(&(a.context(), *k)))
            .collect();
        if keep.len() != b.inputs(a).len() {
            b.set(a, &keep);
        }
    }
    b
}

/// Overlay saved play bindings on `t`: each mapped command takes its
/// action's inputs as (slot 1, slot 0).
pub fn overlay_bindings(t: &mut BindingTable, b: &Bindings) {
    for cmd in 0..57 {
        let Some(a) = action_of_cmd(cmd) else {
            continue;
        };
        let inputs = b.inputs(a);
        for (i, slot) in [1, 0].into_iter().enumerate() {
            let vk = inputs.get(i).and_then(|&k| key_to_vk(k)).unwrap_or(UNBOUND);
            for e in t.0.iter_mut().filter(|e| e.cmd == cmd && e.slot == slot) {
                e.key = vk;
            }
        }
    }
}

/// `controls.toml`: in the player's D2RUST folder, next to `settings.toml`
/// ([`crate::user_dir::controls_file`]). This screen saves it and play
/// loads it, so a rebind is what the next game uses. `D2RUST_CONFIG_DIR`
/// names another folder for it (tests).
pub fn config_path() -> Option<PathBuf> {
    match crate::launch::env_os("D2RUST_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join("controls.toml")),
        None => Some(crate::user_dir::controls_file()),
    }
}

/// The saved play bindings, when a valid `controls.toml` exists.
pub fn saved_bindings() -> Option<Bindings> {
    let path = config_path().filter(|p| p.is_file())?;
    controls::load(&path).ok().map(|(_, b)| b)
}

/// The key-config table to open with: defaults, then `path` when it holds
/// a valid file.
pub fn load_table(path: Option<&Path>) -> BindingTable {
    let mut t = BindingTable::defaults();
    if let Some((_, b)) = path
        .filter(|p| p.is_file())
        .and_then(|p| controls::load(p).ok())
    {
        overlay_bindings(&mut t, &b);
    }
    t
}

/// Write `t` to `path` as `controls.toml` over the `original` preset.
pub fn save_table(t: &BindingTable, path: &Path) -> anyhow::Result<()> {
    let eff = table_to_bindings(t);
    let file = ControlsFile::from_effective(Preset::Original, &eff)
        .ok_or_else(|| anyhow::anyhow!("original preset unavailable"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, controls::write(&file))?;
    Ok(())
}

// ---- the front-end screen ---------------------------------------------

const KEY_BASE: u32 = 0x1000;
const CELL_BASE: u32 = 0x2000;
const BTN_BASE: u32 = 0x3000;
const ARROW_UP: u32 = 0x4000;
const ARROW_DOWN: u32 = 0x4001;

/// The front-end screen. The model is shared so the host can draw it
/// ([`ControlsScreen::model`] + [`ConfigureControls::draw_list`]).
pub struct ControlsScreen {
    model: Rc<RefCell<ConfigureControls>>,
    path: Option<PathBuf>,
    pointer: Option<(i32, i32)>,
    /// Key whose release was already counted by the `action` swallow; the
    /// host's real key-up for it is then ignored once.
    swallowed: Option<u16>,
    click_controls: Vec<Control>,
}

impl ControlsScreen {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            model: Rc::new(RefCell::new(ConfigureControls::open(
                false,
                BindingTable::defaults(),
            ))),
            path,
            pointer: None,
            swallowed: None,
            click_controls: Vec::new(),
        }
    }

    pub fn model(&self) -> Rc<RefCell<ConfigureControls>> {
        self.model.clone()
    }

    fn finish(&self, d: Done) -> Option<Trigger> {
        match d {
            Done::Stay => None,
            // Cancel and Accept both return to the Options menu (the
            // front end has no game to go back to).
            Done::Cancel => Some(Trigger::Exit),
            Done::Accept => {
                if let Some(p) = &self.path {
                    // D2RUST's own: a failed write is logged, the screen closes.
                    if let Err(e) = save_table(self.model.borrow().table(), p) {
                        eprintln!("controls: {e}");
                    }
                }
                Some(Trigger::Ok)
            }
        }
    }
}

impl Screen for ControlsScreen {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        *self.model.borrow_mut() =
            ConfigureControls::open(ctx.expansion, load_table(self.path.as_deref()));
        let mut v = Vec::new();
        // Keys (capture): one invisible control per virtual key.
        for vk in 1..=0xFEu16 {
            v.push(Control::key_only(
                vk,
                Action::Custom(KEY_BASE + u32::from(vk)),
            ));
        }
        // Key cells: rows are hit by `y ≤ T + 58 + 18r`.
        for r in 0..VISIBLE as i32 {
            let (bottom, h) = (T + 59 + 18 * r, if r == 0 { 19 } else { 18 });
            for (col, x, w) in [(1, M + 18, 2 * C), (0, M + 18 + 2 * C, C)] {
                let mut c = Control::new(ControlKind::Button, x, bottom, w as u16, h)
                    .with_action(Action::Custom(CELL_BASE + (r as u32) * 2 + col as u32));
                // The overlay draws the cell; the descriptor handles its click.
                c.visible = true;
                v.push(c);
            }
        }
        for (i, &half) in BTN_HALF.iter().enumerate() {
            let c = M + 206 * i as i32 + 103;
            let mut b = Control::new(
                ControlKind::Button,
                c - half,
                T + 329 + BTN_H,
                (2 * half + 1) as u16,
                BTN_H as u16,
            )
            .with_action(Action::Custom(BTN_BASE + i as u32));
            b.visible = true;
            v.push(b);
        }
        for (id, y) in [(ARROW_UP, T + 47 + 13), (ARROW_DOWN, T + 305 + 13)] {
            let mut a = Control::new(ControlKind::Button, W - M - 31, y, 12, 13)
                .with_action(Action::Custom(id));
            a.visible = true;
            v.push(a);
        }
        self.click_controls = v.clone();
        v
    }

    fn ui_elements(&self) -> Vec<crate::ui::ui_ids::UiElement> {
        let m = self.model.borrow();
        self.click_controls
            .iter()
            .filter_map(|c| {
                let Action::Custom(a) = c.action else {
                    return None;
                };
                let name = match a {
                    a if (CELL_BASE..CELL_BASE + 2 * VISIBLE as u32).contains(&a) => {
                        let n = a - CELL_BASE;
                        let row = m.rows.get(m.top + (n / 2) as usize)?;
                        if row.cmd == SEPARATOR {
                            return None;
                        }
                        format!("binding({},{})", row.cmd, n % 2)
                    }
                    BTN_BASE => "cancel".into(),
                    a if a == BTN_BASE + 1 => "default".into(),
                    a if a == BTN_BASE + 2 => "accept".into(),
                    ARROW_UP => "scroll_up".into(),
                    ARROW_DOWN => "scroll_down".into(),
                    _ => return None,
                };
                let idle = m.idle(&name);
                let mut e =
                    crate::ui::ui_ids::UiElement::new(format!("controls.{name}"), c.hit_box());
                e.idle = idle;
                Some(e)
            })
            .collect()
    }

    fn pointer(&mut self, _ctx: &mut FrontCtx, p: Point) {
        self.pointer = Some((p.x, p.y));
    }

    fn wheel(&mut self, _ctx: &mut FrontCtx, delta: i32) -> Option<Trigger> {
        self.model.borrow_mut().wheel(delta);
        None
    }

    fn key_up(&mut self, ctx: &mut FrontCtx, vk: u16) {
        if self.swallowed == Some(vk) {
            self.swallowed = None;
            return;
        }
        self.model.borrow_mut().key_up(vk, ctx.now_ms);
    }

    fn middle_down(&mut self, ctx: &mut FrontCtx) {
        // The middle button is the pseudo key 0x100 (`vk_to_key`); its release
        // assigns it, as for a keyboard key.
        let mut m = self.model.borrow_mut();
        m.key_down(0x100, false, ctx.now_ms);
        m.key_up(0x100, ctx.now_ms);
    }

    fn overlay(&mut self, now_ms: u64, _adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        // D2RUST's own, unverified: text colour and the `boxpieces` border art
        // are not carried by `DrawItem`.
        let list = self.model.borrow_mut().draw_list(now_ms, self.pointer);
        list.into_iter()
            .map(|d| match d {
                CfgDraw::Rect { x, y, w, h } => DrawItem::Rect {
                    at: Point::new(x, y),
                    w,
                    h,
                },
                CfgDraw::Border { x, y, w, h } => DrawItem::Border {
                    at: Point::new(x, y),
                    w,
                    h,
                },
                CfgDraw::Slider { frame, x, y } => DrawItem::Art {
                    file: r"MENU\textslid",
                    frame,
                    at: Point::new(x, y),
                },
                CfgDraw::Text {
                    string_id,
                    text,
                    x,
                    y,
                    color,
                } => DrawItem::Text {
                    label: None,
                    string_id,
                    text,
                    font: FONT,
                    at: Point::new(x, y),
                    color: i32::from(color),
                    boxed: None,
                },
            })
            .collect()
    }

    fn action(&mut self, ctx: &mut FrontCtx, id: u32) -> Option<Trigger> {
        let done = {
            let mut m = self.model.borrow_mut();
            match id {
                k if (KEY_BASE..KEY_BASE + 0x100).contains(&k) => {
                    let vk = (k - KEY_BASE) as u16;
                    let was = m.editing();
                    let d = m.key_down(vk, false, ctx.now_ms);
                    // The host has no key-up event: the release of the Enter
                    // that began editing is swallowed here.
                    if !was && m.editing() {
                        m.key_up(vk, ctx.now_ms);
                        self.swallowed = Some(vk);
                    }
                    d
                }
                c if (CELL_BASE..CELL_BASE + 0x40).contains(&c) => {
                    let n = c - CELL_BASE;
                    m.click_cell((n / 2) as usize, (n % 2) as i32);
                    Done::Stay
                }
                BTN_BASE => m.cancel(),
                x if x == BTN_BASE + 1 => {
                    m.default_all();
                    Done::Stay
                }
                x if x == BTN_BASE + 2 => m.accept(),
                ARROW_UP => {
                    m.scroll(false);
                    Done::Stay
                }
                ARROW_DOWN => {
                    m.scroll(true);
                    Done::Stay
                }
                _ => Done::Stay,
            }
        };
        self.finish(done)
    }
}

pub fn register(reg: &mut Registry) {
    reg.register(CONTROLS, Box::new(ControlsScreen::new(config_path())));
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    /// Default and the scroll arrows are idle only while their click
    /// changes nothing; an edit in progress makes every button act.
    #[test]
    fn buttons_are_idle_only_while_a_click_changes_nothing() {
        let mut m = ConfigureControls::open(true, BindingTable::defaults());
        assert!(m.idle("default").is_some());
        assert!(m.idle("scroll_up").is_some());
        assert!(m.idle("scroll_down").is_none());
        assert!(m.idle("accept").is_none());
        m.scroll(true);
        assert!(m.idle("scroll_up").is_none());
        while m.top() < m.max_top() {
            m.scroll(true);
        }
        assert!(m.idle("scroll_down").is_some());
        m.click_cell(0, 1);
        assert!(m.editing());
        assert!(m.idle("scroll_down").is_none());
        assert!(m.idle("default").is_none());
    }
}
