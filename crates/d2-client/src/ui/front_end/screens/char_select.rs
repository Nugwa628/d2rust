//! Character select. `Model` is the list, selection and button logic (plain
//! Rust, tested on synthetic save folders); `CharSelect` is the [`Screen`]
//! that draws it and fires the flow triggers.
//!
//! A screen's controls are built only on entry, so every change the player
//! sees (selection, scrolling, a pop-up) re-enters the screen through
//! `Trigger::Rebuild` with `refresh` set; `build` then keeps the model
//! (selection, scroll, pop-up, doll phases) instead of rescanning.
//!
//! Preview fills are `// D2RUST's own, unverified`; each open point carries its
//! own marker and oracle below (METHODS M22).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use d2_formats::d2s::{checksum, status, HEADER_SIZE, MAGIC, MAX_FILE, VERSION, VERSION_MIN};

use crate::ui::front_end::control::{vk, Action, Control, ControlKind, TextRow};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::{DrawItem, Registry, SaveFolder, CHAR_SELECT};
use crate::ui::geom::Point;

/// Visible slots: 2 columns × 4 rows.
pub const SLOTS: usize = 8;
/// Double-click window.
pub const DOUBLE_CLICK_MS: u64 = 500;
/// String ids used by the screen.
pub mod strings {
    pub const EXIT: u32 = 5101;
    pub const OK: u32 = 5102;
    pub const DELETE: u32 = 5272;
    pub const CREATE_NEW: u32 = 10832;
    pub const CHARACTER: u32 = 21796;
    pub const CONVERT_TO: u32 = 22732;
    pub const EXPANSION_LABEL: u32 = 22730;
    pub const EXPANSION_CHARACTER: u32 = 22731;
    pub const DEAD_HARDCORE: u32 = 5304;
    pub const DELETE_QUESTION: u32 = 5163;
    pub const NO: u32 = 5167;
    pub const YES: u32 = 5166;
    pub const CONVERT_WARNING: u32 = 22734;
    pub const CONVERT_FAILED: u32 = 21872;
    /// The convert pop-up's question row (0xDD; 22735 is empty).
    pub const CONVERT_QUESTION: u32 = 22736;
}
/// `Action::Custom` ids (the screen's descriptor ids where they exist).
pub mod ids {
    pub const SLOT_TEXT: u32 = 0x84; // 0x84..=0x8B
    pub const SLOT_FIGURE: u32 = 0x8C; // 0x8C..=0x93
    pub const OK: u32 = 0xA2;
    pub const CONVERT: u32 = 0xA5;
    pub const DELETE: u32 = 0xA6;
    pub const POPUP_NO: u32 = 0xD7;
    pub const POPUP_YES: u32 = 0xD8;
    pub const KEY_HOME: u32 = 0x100;
    pub const KEY_END: u32 = 0x101;
    pub const KEY_LEFT: u32 = 0x102;
    pub const KEY_RIGHT: u32 = 0x103;
    pub const KEY_UP: u32 = 0x104;
    pub const KEY_DOWN: u32 = 0x105;
}
/// Virtual keys of the selection box's handler.
mod key {
    pub const END: u16 = 35;
    pub const HOME: u16 = 36;
    pub const LEFT: u16 = 37;
    pub const UP: u16 = 38;
    pub const RIGHT: u16 = 39;
    pub const DOWN: u16 = 40;
}

/// One save in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file name up to its first `.` (not the header's name field).
    pub name: String,
    pub class: u8,
    pub level: u8,
    pub status: u16,
    pub components: [u8; 11],
    pub colours: [u8; 11],
    /// File last-write time, nanoseconds since the epoch.
    pub mtime: u128,
}

impl Entry {
    pub fn hardcore(&self) -> bool {
        self.status & status::HARDCORE != 0
    }
    pub fn dead(&self) -> bool {
        self.status & (status::HARDCORE | status::DEAD) == (status::HARDCORE | status::DEAD)
    }
    pub fn expansion(&self) -> bool {
        self.status & status::EXPANSION != 0
    }
    /// Progression p, status bits 8–12.
    pub fn progression(&self) -> u8 {
        status::progression(self.status)
    }
    /// Difficulties open: 1 Normal, 2 + Nightmare, 3 + Hell (the thresholds of
    /// the save's progression byte, [`d2_formats::d2s::status::difficulties`]).
    pub fn difficulties_open(&self) -> u8 {
        d2_formats::d2s::status::difficulties(self.status).count_ones() as u8
    }
    /// Paper doll `(class', mode)`: softcore mode 5; hardcore
    /// alive mode 1 (expansion front end); hardcore dead class' 8 / 9, mode 5.
    pub fn paper_doll(&self, expansion_front_end: bool) -> (u8, u8) {
        if self.dead() {
            (
                if matches!(self.class, 0 | 1 | 6) {
                    8
                } else {
                    9
                },
                5,
            )
        } else if self.hardcore() && expansion_front_end {
            (self.class, 1)
        } else {
            (self.class, 5)
        }
    }
}

/// The class name of the level line: string 4011 Amazon,
/// 4010 Sorceress, 4009 Necromancer, 4008 Paladin, 4007 Barbarian, 10097
/// Druid, 10098 Assassin for classes 0–6, any other class 5383 ("an evil
/// force"); the English texts of the stock tables.
pub fn class_name(class: u8) -> &'static str {
    const NAMES: [&str; 7] = [
        "Amazon",
        "Sorceress",
        "Necromancer",
        "Paladin",
        "Barbarian",
        "Druid",
        "Assassin",
    ];
    NAMES
        .get(usize::from(class))
        .copied()
        .unwrap_or("an evil force")
}

/// The title prefix (the character title of the quest progress).
pub fn title_prefix(class: u8, p: u8, hardcore: bool, expansion: bool) -> &'static str {
    let mut t = if expansion {
        match p {
            0..=4 => 0,
            5..=9 => 1,
            10..=14 => 2,
            _ => 3,
        }
    } else {
        match p {
            0..=3 => 0,
            4..=7 => 1,
            8..=11 => 2,
            _ => 3,
        }
    };
    if hardcore && t != 0 {
        t += 3;
    }
    let female = matches!(class, 0 | 1 | 6 | 9);
    match (expansion, female, t) {
        (false, true, 1) => "Dame ",
        (false, false, 1) => "Sir ",
        (false, true, 2) => "Lady ",
        (false, false, 2) => "Lord ",
        (false, true, 3) => "Baroness ",
        (false, false, 3) => "Baron ",
        (false, true, 4) => "Countess ",
        (false, false, 4) => "Count ",
        (false, true, 5) => "Duchess ",
        (false, false, 5) => "Duke ",
        (false, true, 6) => "Queen ",
        (false, false, 6) => "King ",
        (true, _, 1) => "Slayer ",
        (true, _, 2) => "Champion ",
        (true, _, 3) => "Matriarch/Patriarch ",
        (true, _, 4) => "Destroyer ",
        (true, _, 5) => "Conqueror ",
        (true, _, 6) => "Guardian ",
        _ => "",
    }
}

/// The expansion tier 3 title differs by sex.
fn title(class: u8, p: u8, hardcore: bool, expansion: bool) -> String {
    let s = title_prefix(class, p, hardcore, expansion);
    if s.starts_with("Matriarch") {
        let f = if matches!(class, 0 | 1 | 6 | 9) {
            "Matriarch "
        } else {
            "Patriarch "
        };
        return f.to_string();
    }
    s.to_string()
}

/// The save header read; `None` = rejected.
pub fn read_header(b: &[u8], name: &str, mtime: u128) -> Option<Entry> {
    if b.len() < 8 || u32::from_le_bytes(b[0..4].try_into().ok()?) != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(b[4..8].try_into().ok()?);
    if version >= VERSION_MIN {
        if b.len() < HEADER_SIZE || version > VERSION {
            return None;
        }
        let mut components = [0u8; 11];
        let mut colours = [0u8; 11];
        components.copy_from_slice(&b[0x88..0x88 + 11]);
        colours.copy_from_slice(&b[0x98..0x98 + 11]);
        if components[0] == 0 {
            components = DEFAULT_COMPONENTS;
        }
        Some(Entry {
            name: name.to_string(),
            class: b[0x28],
            level: b[0x2B],
            status: u16::from_le_bytes([b[0x24], b[0x25]]),
            components,
            colours,
            mtime,
        })
    } else {
        if b.len() < 0x82 || version < 0x47 {
            return None;
        }
        // D2RUST's own, unverified: the legacy level offset is not in
        // the legacy save layout; level 1 is shown.
        Some(Entry {
            name: name.to_string(),
            class: b[0x22],
            level: 1,
            status: u16::from_le_bytes([b[0x18], b[0x19]]),
            components: DEFAULT_COMPONENTS,
            colours: [0xFF; 11],
            mtime,
        })
    }
}

/// The default component set: D2RUST's own, unverified;
/// the standard unequipped set.
const DEFAULT_COMPONENTS: [u8; 11] = [0xFF; 11];

fn mtime_of(p: &Path) -> u128 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Scan `dir`: `*.d2s`, name up to the first `.`, filtered,
/// header-checked, sorted newest first (expansion) or in scan order
/// (classic). Scan order is the sorted file-name order (D2RUST: the file
/// system's order is not stable).
pub fn scan(dir: &Path, expansion_front_end: bool) -> Vec<Entry> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let f = e.file_name().to_string_lossy().into_owned();
            f.to_ascii_lowercase()
                .ends_with(".d2s")
                .then(|| (f, e.path()))
        })
        .collect();
    files.sort();
    let mut out = Vec::new();
    for (file, path) in files {
        let stem = file.split('.').next().unwrap_or("").to_string();
        let n = stem.chars().count();
        if !(2..=15).contains(&n) || stem.ends_with('-') || stem.starts_with('_') {
            continue;
        }
        // The read goes to `<folder><stem>.d2s`, not the listed file.
        let read_path = dir.join(format!("{stem}.d2s"));
        let Ok(mut bytes) = fs::read(&read_path) else {
            continue;
        };
        bytes.truncate(MAX_FILE);
        if let Some(e) = read_header(&bytes, &stem, mtime_of(&path)) {
            out.push(e);
        }
    }
    if expansion_front_end {
        out.sort_by_key(|e| std::cmp::Reverse(e.mtime)); // stable: ties keep scan order
    }
    out
}

/// `<folder>*.d2s` holds a usable character.
#[derive(Clone, Debug)]
pub struct DirSaves {
    pub dir: PathBuf,
    pub expansion: bool,
}

impl SaveFolder for DirSaves {
    fn has_saves(&self) -> bool {
        !scan(&self.dir, self.expansion).is_empty()
    }
}

/// A pop-up over the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    None,
    Delete,
    Convert,
    /// A message box with OK (string id).
    Message(u32),
    /// The difficulty box over the list; `hell`: the Hell button is on.
    Difficulty {
        hell: bool,
    },
}

/// What OK did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OkResult {
    Nothing,
    /// Message box 5304 up.
    Message(u32),
    /// Difficulty box: `difficulties_open` > 1.
    Difficulty(Entry),
    /// The game starts at once at Normal.
    Start(Entry),
}

/// Which buttons are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub ok: bool,
    pub delete: bool,
    pub convert: bool,
    pub create: bool,
}

/// The screen state: list, selection, scrolling, pop-up.
#[derive(Clone, Debug)]
pub struct Model {
    pub dir: PathBuf,
    pub expansion: bool,
    pub entries: Vec<Entry>,
    /// The selected entry; −1 none.
    pub sel: i32,
    /// The first entry shown.
    pub first: usize,
    /// Scroll bar thumb position (rows of 2).
    pub scroll: i32,
    /// The bar position at the last callback (the scroll callback gets the change from it).
    pub cb_pos: i32,
    /// The bar's arrows drawn pressed: the wheel sets one, a click clears both.
    pub up_pressed: bool,
    pub down_pressed: bool,
    /// `now_ms` when a message box (5304) opened: it ignores OK for `MESSAGE_GUARD_MS` (an
    /// Enter 0.3 s after the click that opened it leaves it up; a
    /// Return 12 ticks (0.48 s) after it opened closes it;
    /// the guard lies in (0.3 s, 0.48 s], mechanism not found yet).
    pub msg_ms: u64,
    pub popup: Popup,
    last_click: Option<(u64, i32)>,
    columns: usize,
    /// A list given by the host instead of the folder's saves (the online
    /// characters): rescans read it, Delete YES leaves the name in
    /// [`Model::delete_request`] instead of deleting files, and Convert is
    /// off.
    pub fixed: Option<Vec<Entry>>,
    /// The character whose Delete was confirmed on a fixed list.
    pub delete_request: Option<String>,
}

impl Model {
    pub fn new(dir: PathBuf, expansion: bool) -> Self {
        let mut m = Self {
            dir,
            expansion,
            entries: Vec::new(),
            sel: -1,
            first: 0,
            scroll: 0,
            cb_pos: 0,
            up_pressed: false,
            down_pressed: false,
            msg_ms: 0,
            popup: Popup::None,
            last_click: None,
            columns: 2,
            fixed: None,
            delete_request: None,
        };
        m.rescan();
        m
    }

    /// A model on a list the host gives (in the order to show).
    pub fn with_entries(entries: Vec<Entry>, expansion: bool) -> Self {
        let mut m = Self::new(PathBuf::new(), expansion);
        m.fixed = Some(entries);
        m.rescan();
        m
    }

    /// Scan again; `sel` = `first` = 0.
    pub fn rescan(&mut self) {
        self.entries = match &self.fixed {
            Some(list) => list.clone(),
            None => scan(&self.dir, self.expansion),
        };
        self.sel = if self.entries.is_empty() { -1 } else { 0 };
        self.first = 0;
        self.scroll = 0;
        self.cb_pos = 0;
        self.up_pressed = false;
        self.down_pressed = false;
        self.popup = Popup::None;
        self.last_click = None;
    }

    fn n(&self) -> i32 {
        self.entries.len() as i32
    }

    pub fn selected(&self) -> Option<&Entry> {
        usize::try_from(self.sel)
            .ok()
            .and_then(|i| self.entries.get(i))
    }

    pub fn buttons(&self) -> Buttons {
        let e = self.selected();
        Buttons {
            ok: e.is_some(),
            delete: e.is_some(),
            convert: self.expansion && self.fixed.is_none() && e.is_some_and(|e| !e.expansion()),
            create: true,
        }
    }

    /// Scroll bar range ⌈(n − 8) / 2⌉ when n > 8.
    pub fn scroll_range(&self) -> Option<i32> {
        (self.n() > SLOTS as i32)
            .then(|| (self.n() - SLOTS as i32 + self.columns as i32 - 1) / self.columns as i32)
    }

    /// The entry in visible slot `i`.
    pub fn slot(&self, i: usize) -> Option<&Entry> {
        if i >= SLOTS {
            return None;
        }
        self.entries.get(self.first + i)
    }

    fn keep_visible(&mut self) {
        if self.columns == 3 {
            let selected = self.sel.max(0) as usize;
            self.first = self.first / 3 * 3;
            if selected < self.first {
                self.first = selected / 3 * 3;
            } else if selected >= self.first + SLOTS {
                self.first = (selected + 1 - SLOTS).div_ceil(3) * 3;
            }
            self.scroll = (self.first / 3) as i32;
            self.cb_pos = self.scroll;
            return;
        }

        let sel = self.sel.max(0) as usize;
        if sel < self.first {
            self.first = sel;
        } else if sel >= self.first + SLOTS {
            self.first = sel + 1 - SLOTS;
        }
    }

    fn set_columns(&mut self, columns: usize) {
        if self.columns == columns {
            return;
        }
        self.columns = columns;
        self.keep_visible();
        self.scroll = (self.first / self.columns) as i32;
        self.cb_pos = self.scroll;
        self.last_click = None;
    }

    fn compact_key(&mut self, vk: u16) {
        let n = self.n();
        if n == 0 || self.popup != Popup::None {
            return;
        }
        let columns = self.columns as i32;
        match vk {
            key::HOME => self.sel = 0,
            key::END => self.sel = n - 1,
            key::LEFT if self.sel % columns > 0 => self.sel -= 1,
            key::RIGHT if self.sel % columns < columns - 1 && self.sel + 1 < n => self.sel += 1,
            key::UP if self.sel >= columns => self.sel -= columns,
            key::DOWN if self.sel + columns < n => self.sel += columns,
            _ => {}
        }
        self.keep_visible();
    }

    /// A key of the selection box.
    pub fn key(&mut self, vk: u16) {
        if self.columns != 2 {
            self.compact_key(vk);
            return;
        }
        let n = self.n();
        if n == 0 || self.popup != Popup::None {
            return;
        }
        let eight = SLOTS as i32;
        match vk {
            key::HOME => {
                self.sel = 0;
                self.first = 0;
            }
            key::END => {
                self.sel = n - 1;
                if n > eight {
                    self.first = (n - eight) as usize;
                }
            }
            key::LEFT => {
                if self.sel & 1 == 1 {
                    self.sel -= 1;
                    self.keep_visible();
                }
            }
            key::RIGHT => {
                if self.sel & 1 == 0 && self.sel < n - 1 {
                    self.sel += 1;
                    self.keep_visible();
                }
            }
            key::UP => {
                if self.sel >= 2 {
                    self.sel -= 2;
                    if (self.sel as usize) < self.first {
                        self.first = (self.sel & !1) as usize;
                        self.scroll -= 1;
                    }
                }
            }
            key::DOWN if self.sel + 2 < n => {
                self.sel += 2;
                if self.sel as usize >= self.first + SLOTS {
                    let f = if self.sel & 1 == 1 {
                        self.sel - 7
                    } else {
                        self.sel - 6
                    };
                    self.first = f.max(0) as usize;
                    self.scroll += 1;
                }
            }
            _ => {}
        }
    }

    /// The scroll bar moved to position `pos`: the callback takes the change from the
    /// position of its last call, `first += 2d`, clamped to `[0, n − 1]`; `sel` unchanged. The
    /// Up / Down keys move the bar without a callback, so the next callback can carry several
    /// rows (first 4, bar 2 after Down ×5 / Up, one wheel step
    /// gives first 10).
    pub fn scroll_to(&mut self, pos: i32) {
        let range = self.scroll_range().unwrap_or(0);
        self.scroll = pos.clamp(0, range);
        let d = self.scroll - self.cb_pos;
        self.cb_pos = self.scroll;
        let f = (self.first as i32 + self.columns as i32 * d).clamp(0, (self.n() - 1).max(0));
        self.first = f as usize;
    }

    /// The scroll bar moved by `d` (arrows, thumb).
    pub fn scroll_by(&mut self, d: i32) {
        self.scroll_to(self.scroll + d);
    }

    /// The wheel: k = delta / 120 (toward 0); up (k > 0) steps the bar −1, down +1;
    /// nothing with n ≤ 8 or a pop-up.
    pub fn wheel(&mut self, delta: i32) {
        let k = delta / 120;
        if k == 0 || self.scroll_range().is_none() || self.popup != Popup::None {
            return;
        }
        if k > 0 {
            self.up_pressed = true;
        } else {
            self.down_pressed = true;
        }
        self.scroll_to(self.scroll - k.signum());
    }

    /// A click on visible slot `i` at `now_ms`. Returns `true`
    /// for a double click (the caller runs OK).
    pub fn click(&mut self, i: usize, now_ms: u64) -> bool {
        self.up_pressed = false;
        self.down_pressed = false;
        if self.popup != Popup::None {
            return false;
        }
        let k = (self.first + i) as i32;
        // A click on the slot right after the last entry
        // (k = n) changes nothing; the selection box stays on the entry.
        if k >= self.n() {
            return false;
        }
        self.sel = if self.n() < 1 { -1 } else { k };
        // The click that selects a dead hardcore slot shows 5304.
        if self.selected().is_some_and(Entry::dead) {
            self.popup = Popup::Message(strings::DEAD_HARDCORE);
            self.msg_ms = now_ms;
        }
        let double = self
            .last_click
            .is_some_and(|(t, s)| s == self.sel && now_ms.saturating_sub(t) <= DOUBLE_CLICK_MS);
        self.last_click = Some((now_ms, self.sel));
        double
    }

    /// OK / Enter.
    pub fn ok(&mut self) -> OkResult {
        if self.popup != Popup::None {
            return OkResult::Nothing;
        }
        let Some(e) = self.selected().cloned() else {
            return OkResult::Nothing;
        };
        if e.expansion() && !self.expansion {
            return OkResult::Nothing;
        }
        if e.dead() {
            self.popup = Popup::Message(strings::DEAD_HARDCORE);
            return OkResult::Message(strings::DEAD_HARDCORE);
        }
        if e.difficulties_open() > 1 {
            OkResult::Difficulty(e)
        } else {
            OkResult::Start(e)
        }
    }

    /// Delete YES: every non-directory `<folder><name>.*`,
    /// stopping at the first failed delete; then rescan.
    pub fn delete_selected(&mut self) {
        if self.fixed.is_some() {
            self.delete_request = self.selected().map(|e| e.name.clone());
            self.popup = Popup::None;
            return;
        }
        if let Some(name) = self.selected().map(|e| e.name.clone()) {
            let prefix = format!("{}.", name.to_ascii_lowercase());
            loop {
                let Ok(rd) = fs::read_dir(&self.dir) else {
                    break;
                };
                let hit = rd.filter_map(Result::ok).find(|e| {
                    e.file_type().is_ok_and(|t| t.is_file())
                        && e.file_name()
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .starts_with(&prefix)
                });
                match hit {
                    Some(f) if fs::remove_file(f.path()).is_ok() => {}
                    _ => break,
                }
            }
        }
        self.rescan();
    }

    /// Convert YES. A failure shows
    /// message 21872 after a rescan.
    pub fn convert_selected(&mut self) {
        let Some(e) = self.selected().cloned() else {
            self.popup = Popup::None;
            return;
        };
        if e.expansion() {
            self.popup = Popup::None;
            return;
        }
        let path = self.dir.join(format!("{}.d2s", e.name));
        let done = (|| -> Option<()> {
            let mut b = fs::read(&path).ok()?;
            if b.len() > MAX_FILE || b.len() < HEADER_SIZE {
                return None;
            }
            if u32::from_le_bytes(b[4..8].try_into().ok()?) < VERSION_MIN {
                return None;
            }
            let s = u16::from_le_bytes([b[0x24], b[0x25]]) | status::EXPANSION;
            b[0x24..0x26].copy_from_slice(&s.to_le_bytes());
            b[0x0C..0x10].fill(0);
            let c = checksum(&b);
            b[0x0C..0x10].copy_from_slice(&c.to_le_bytes());
            fs::write(&path, b).ok()
        })();
        if done.is_some() {
            let i = self.sel as usize;
            self.entries[i].status |= status::EXPANSION;
            self.popup = Popup::None;
        } else {
            self.rescan();
            self.popup = Popup::Message(strings::CONVERT_FAILED);
        }
    }

    /// The slot's text lines: title + name, level line, and the
    /// expansion string id when status & 0x20. Colour: 1 (red) hardcore,
    /// else 4 (gold) for the name line.
    pub fn slot_lines(&self, i: usize) -> Option<SlotLines> {
        let e = self.slot(i)?;
        Some(SlotLines {
            // The title takes the first row (the realm's, empty in single player), the name the
            // next one.
            title: title(e.class, e.progression(), e.hardcore(), e.expansion())
                .trim()
                .to_string(),
            name: e.name.clone(),
            name_colour: if e.hardcore() { 1 } else { 4 },
            level: format!("Level {} {}", e.level, class_name(e.class)),
            expansion: e.expansion().then_some(strings::EXPANSION_CHARACTER),
        })
    }
}

/// What a slot shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotLines {
    /// The title prefix, its own row above the name; empty when the
    /// character has none (the two are separate rows).
    pub title: String,
    pub name: String,
    pub name_colour: u8,
    pub level: String,
    /// String id 22731 in colour 2 (green).
    pub expansion: Option<u32>,
}

/// The character the player chose, for the host and the difficulty box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub entry: Entry,
    /// Difficulties open (1 Normal, 2 + Nightmare, 3 + Hell).
    pub difficulties_open: u8,
}

/// Where the screen leaves the chosen character.
pub type SelectionHandle = Arc<Mutex<Option<Selection>>>;

/// The screen.
pub struct CharSelect {
    /// `ctx.now_ms` when the controls were last built (the dolls start
    /// their animation then).
    built_ms: u64,
    /// The list was scanned again (Delete, a failed Convert YES): the dolls start over.
    restart_dolls: bool,
    /// Per entry: the milliseconds its doll was drawn since the scan (the phase advances
    /// per draw, a doll outside the 8 slots is not drawn and keeps its phase).
    doll_ms: std::collections::BTreeMap<usize, u64>,
    doll_last_ms: u64,
    /// The entries whose dolls the last pass drew.
    doll_drawn: Vec<usize>,
    expansion: bool,
    dir: Option<PathBuf>,
    model: Option<Model>,
    compact: bool,
    refresh: bool,
    handle: SelectionHandle,
    /// The host's list in place of the save folder (the online
    /// characters); OK never opens the difficulty box on it (the online
    /// lobby picks the difficulty).
    list: Option<Vec<Entry>>,
}

impl CharSelect {
    pub fn new(dir: Option<PathBuf>, handle: SelectionHandle) -> Self {
        Self {
            built_ms: 0,
            restart_dolls: false,
            doll_ms: std::collections::BTreeMap::new(),
            doll_last_ms: 0,
            doll_drawn: Vec::new(),
            expansion: false,
            dir,
            model: None,
            compact: false,
            refresh: false,
            handle,
            list: None,
        }
    }

    /// The screen on a list the host gives instead of a save folder.
    pub fn with_list(list: Vec<Entry>, handle: SelectionHandle) -> Self {
        Self {
            list: Some(list),
            ..Self::new(None, handle)
        }
    }

    /// A new list: the next build starts over on it (selection, scroll,
    /// dolls).
    pub fn set_list(&mut self, list: Vec<Entry>) {
        self.list = Some(list);
        self.refresh = false;
    }

    /// The next build keeps the list, selection and scroll (a host that
    /// wraps the screen rebuilds it for its own pop-ups).
    pub fn keep(&mut self) {
        self.refresh = true;
    }

    /// The name whose Delete the player confirmed on a host list, once.
    pub fn take_delete_request(&mut self) -> Option<String> {
        self.model.as_mut()?.delete_request.take()
    }

    pub fn model(&self) -> Option<&Model> {
        self.model.as_ref()
    }

    fn again(&mut self) -> Option<Trigger> {
        self.refresh = true;
        Some(Trigger::Rebuild)
    }

    fn run_ok(&mut self, ctx: &mut FrontCtx) -> Option<Trigger> {
        let m = self.model.as_mut()?;
        match m.ok() {
            OkResult::Nothing => None,
            OkResult::Message(_) => {
                m.msg_ms = ctx.now_ms;
                self.again()
            }
            OkResult::Difficulty(e) | OkResult::Start(e) => {
                // The box pushes over the list (list A stays, F2.8 Esc pops it):
                // coming back keeps the selection instead of rescanning.
                // On a host list the lobby picks the difficulty.
                let open = if self.list.is_some() {
                    1
                } else {
                    e.difficulties_open()
                };
                self.refresh = open > 1;
                ctx.flow.difficulties_open = open;
                *self.handle.lock().ok()? = Some(Selection {
                    difficulties_open: open,
                    entry: e,
                });
                if open > 1 {
                    // The difficulty box opens over the list.
                    self.model.as_mut()?.popup = Popup::Difficulty { hell: open >= 3 };
                    return self.again();
                }
                Some(Trigger::Ok)
            }
        }
    }
}

const BG: &str = r"CharSelect\characterselectscreenEXP";
const MEDIUM: &str = r"FrontEnd\MediumButtonBlank";
const TALL: &str = r"CharSelect\TallButtonBlank";
const SEL_BOX: &str = r"CharSelect\charselectbox";
const POPUP: &str = r"FrontEnd\PopUpOKCancel2";
const POPUP_OK: &str = r"FrontEnd\PopUpOk";
const CANCEL: &str = r"FrontEnd\CancelButtonBlank";
const SCROLL: &str = r"FrontEnd\joingamescrollbars";

fn slot_x(i: usize) -> i32 {
    if i & 1 == 0 {
        37
    } else {
        309
    }
}

fn slot_y(i: usize) -> i32 {
    178 + 93 * (i / 2) as i32
}

fn canvas_slot(compact: bool, i: usize) -> (i32, i32) {
    if compact {
        (4 + 260 * (i % 3) as i32, 178 + 93 * (i / 3) as i32)
    } else {
        (slot_x(i), slot_y(i))
    }
}

/// Fonts of the text descriptors: Font42,
/// Font16.
const FONT42: u16 = 3;
/// A message box ignores its OK for this long after it opened.
const MESSAGE_GUARD_MS: u64 = 400;
const FONT8: u16 = 0;
const FONT16: u16 = 1;
/// Font24 (the difficulty title; the message box text).
const FONT24: u16 = 7;

/// A pop-up text (0xD9 / 0xDD, 268, 320, 264 × 120): Font16, margins 10 / 8,
/// flag 2 (centred, wrapped), as the create pop-up 208 (check
/// `ui-frontend-menus-b`; shots delete-pop / convert-pop).
fn popup_text(string_id: u32) -> Control {
    let mut c = Control::new(ControlKind::Text, 268, 320, 264, 120)
        .with_string(string_id)
        .with_font(FONT16, 2);
    c.margin = (10, 8);
    c
}

fn button(x: i32, y: i32, w: u16, h: u16, art: &'static str, label: u32) -> Control {
    Control::new(ControlKind::Button, x, y, w, h)
        .with_art(art)
        .with_string(label)
}

impl Screen for CharSelect {
    fn canvas_changed(&mut self, canvas: super::super::canvas::FrontCanvas) {
        self.compact = canvas.height < 600;
        if let Some(model) = self.model.as_mut() {
            model.set_columns(if self.compact { 3 } else { 2 });
        }
    }

    fn relayout(&mut self, ctx: &mut FrontCtx) -> Option<Vec<Control>> {
        self.refresh = true;
        Some(self.build(ctx))
    }

    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        let fresh = !self.refresh || self.model.is_none();
        if fresh {
            // D2RUST's own, unverified: no folder = an empty list.
            self.model = Some(match &self.list {
                Some(list) => Model::with_entries(list.clone(), ctx.expansion),
                None => Model::new(self.dir.clone().unwrap_or_default(), ctx.expansion),
            });
        }
        self.model
            .as_mut()
            .expect("model set above")
            .set_columns(if self.compact { 3 } else { 2 });
        self.refresh = false;
        // The dolls are built when the list is scanned and keep their phase across selection
        // changes and pop-ups (`ui-frontend-charlist`): restart only on entry / rescan.
        if fresh || self.restart_dolls {
            self.built_ms = ctx.now_ms;
            self.doll_last_ms = ctx.now_ms;
            self.doll_ms.clear();
            self.doll_drawn.clear();
            self.restart_dolls = false;
        }
        self.expansion = ctx.expansion;
        let m = self.model.as_ref().expect("model set above");
        let b = m.buttons();
        let slot = |i| canvas_slot(self.compact, i);
        let text_w = if self.compact { 188 } else { 200 };
        let mut v = Vec::new();

        // Background (0xA1).
        v.push(Control::new(ControlKind::Image, 0, 599, 800, 600).with_art(BG));
        // Slot labels (0x84+i) and click controls (0x84+i / 0x8C+i).
        for i in 0..SLOTS {
            if let Some(l) = m.slot_lines(i) {
                // 0x84 + i: 200 × 92, a = 76, b = 3, Font16; rows
                // name (gold / red), level (colour 0), expansion (green).
                // No wrap (flag 0x20): EXPANSION CHARACTER stays on
                // one row although it is wider than w − 2·mx = 48.
                // The first row is the title (empty without one), the name
                // sits one row pitch (14 px) below it.
                let mut c = Control::new(ControlKind::Text, slot(i).0, slot(i).1, text_w, 92)
                    .with_font(if self.compact { FONT8 } else { FONT16 }, 0x20);
                c.margin = (76, 3);
                if !l.title.is_empty() {
                    c = c.with_text(l.title.clone(), i32::from(l.name_colour));
                }
                c.more_rows.push(TextRow {
                    string_id: 0,
                    text: l.name.clone(),
                    color: i32::from(l.name_colour),
                });
                c.more_rows.push(TextRow {
                    string_id: 0,
                    text: l.level.clone(),
                    color: 0,
                });
                if let Some(id) = l.expansion {
                    c.more_rows.push(TextRow {
                        string_id: id,
                        text: String::new(),
                        color: 2,
                    });
                }
                v.push(c);
            }
        }
        for i in 0..SLOTS {
            let (x, y) = slot(i);
            v.push(
                Control::new(ControlKind::Button, x, y, text_w, 92)
                    .with_action(Action::Custom(ids::SLOT_TEXT + i as u32)),
            );
            v.push(
                Control::new(ControlKind::Button, x + i32::from(text_w), y, 72, 93)
                    .with_action(Action::Custom(ids::SLOT_FIGURE + i as u32)),
            );
        }
        // Selection box (0x96) when the selection is visible.
        if let Ok(s) = usize::try_from(m.sel) {
            if !self.compact && s >= m.first && s < m.first + SLOTS && s < m.entries.len() + 1 {
                let i = s - m.first;
                // `charselectbox` is two frames, 256 + 16 px wide: both are
                // drawn side by side: 272 × 93 (37–308), the bottom row on the
                // slot row y (a one-row image's cel bottom is the control's y).
                v.push(
                    Control::new(ControlKind::Image, slot_x(i), slot_y(i), 272, 93)
                        .with_art(SEL_BOX),
                );
            }
        }
        // Top label (0x9C): the selected name.
        // Centred (flags 2): the name centred in 85–550.
        if let Some(e) = m.selected() {
            v.push(
                Control::new(ControlKind::Text, 85, 78, 466, 42)
                    .with_font(FONT42, 2)
                    .with_text(e.name.clone(), 0),
            );
        }
        // Scroll bar (0xA7), n > 8.
        if let Some(range) = m.scroll_range() {
            // The widget's cels (`joingamescrollbars`, 12 x 14; the track is the background's),
            // placed as measured: arrows at x 585, up top 89,
            // down top 442 (frames 2 / 3 while pressed by the wheel), thumb x 585, top 99 + 333 pos / range.
            let cel = |frame: u32, top: i32| {
                let mut c = Control::new(
                    ControlKind::Image,
                    if self.compact { 788 } else { 585 },
                    top + 14,
                    12,
                    14,
                )
                .with_art(SCROLL);
                c.cel = Some(frame);
                c
            };
            v.push(cel(if m.up_pressed { 2 } else { 0 }, 89));
            v.push(cel(
                if m.down_pressed { 3 } else { 1 },
                if self.compact { 380 } else { 442 },
            ));
            v.push(cel(
                4,
                99 + (if self.compact { 271 } else { 333 }) * m.scroll / range,
            ));
        }
        // Buttons (0xA2–0xA6).
        let mut ok = button(627, 572, 128, 35, MEDIUM, strings::OK)
            .with_hotkey(vk::ENTER)
            .with_action(Action::Custom(ids::OK));
        ok.enabled = b.ok;
        v.push(ok);
        v.push(
            button(33, 572, 128, 35, MEDIUM, strings::EXIT)
                .with_hotkey(vk::ESC)
                .with_action(Action::Trigger(Trigger::Exit)),
        );
        // Second label lines: CHARACTER, EXPANSION.
        let mut create = button(33, 528, 168, 60, TALL, strings::CREATE_NEW)
            .with_action(Action::Trigger(Trigger::CreateNew));
        create.second_label = strings::CHARACTER;
        v.push(create);
        let mut conv = button(233, 528, 168, 60, TALL, strings::CONVERT_TO)
            .with_action(Action::Custom(ids::CONVERT));
        conv.second_label = strings::EXPANSION_LABEL;
        conv.enabled = b.convert;
        // Not on a host list (the online characters convert on the server, if ever).
        conv.visible = ctx.expansion && self.list.is_none();
        v.push(conv);
        let mut del = button(433, 528, 168, 60, TALL, strings::DELETE)
            .with_action(Action::Custom(ids::DELETE));
        del.second_label = strings::CHARACTER;
        del.enabled = b.delete;
        v.push(del);
        // Keys (the selection box's handler).
        for (k, id) in [
            (key::HOME, ids::KEY_HOME),
            (key::END, ids::KEY_END),
            (key::LEFT, ids::KEY_LEFT),
            (key::RIGHT, ids::KEY_RIGHT),
            (key::UP, ids::KEY_UP),
            (key::DOWN, ids::KEY_DOWN),
        ] {
            v.push(Control::key_only(k, Action::Custom(id)));
        }
        // Pop-up, on top of everything.
        if m.popup != Popup::None {
            v.push(
                Control::new(ControlKind::Image, 0, 600, 800, 600)
                    .with_art(crate::ui::front_end::DIM_LAYER),
            );
        }
        match m.popup {
            Popup::None => {}
            Popup::Delete | Popup::Convert | Popup::Message(_) => {
                // The pop-up art sits one row higher than the control box (rows 174-349):
                // its two cels placed directly.
                let art = if matches!(m.popup, Popup::Message(_)) {
                    POPUP_OK
                } else {
                    POPUP
                };
                for (frame, x) in [(0, 268), (1, 524)] {
                    let mut c = Control::new(ControlKind::Image, x, 350, 264, 176).with_art(art);
                    c.cel = Some(frame);
                    v.push(c);
                }
                if let Popup::Message(id) = m.popup {
                    let mut t = popup_text(id).with_font(FONT24, 2);
                    t.color = 0;
                    v.push(t);
                    v.push(
                        button(351, 337, 96, 32, CANCEL, strings::OK)
                            .with_hotkey(vk::ENTER)
                            .with_action(Action::Custom(ids::POPUP_NO)),
                    );
                } else {
                    let q = if m.popup == Popup::Delete {
                        strings::DELETE_QUESTION
                    } else {
                        strings::CONVERT_WARNING
                    };
                    if m.popup == Popup::Convert {
                        // 0xDD (268, 310, 264 x 120): the warning wraps to three rows, a blank
                        // row, then the question (rows 18 px apart in Font8, `ui-frontend-menus-b`).
                        let mut a = popup_text(q).with_font(FONT8, 2);
                        a.y = 302;
                        let mut b = popup_text(strings::CONVERT_QUESTION).with_font(FONT8, 2);
                        b.y = 302 + 4 * 18;
                        v.push(a);
                        v.push(b);
                    } else {
                        v.push(popup_text(q));
                    }
                    v.push(
                        button(281, 337, 96, 32, CANCEL, strings::NO)
                            .with_hotkey(vk::ESC)
                            .with_action(Action::Custom(ids::POPUP_NO)),
                    );
                    v.push(
                        button(421, 337, 96, 32, CANCEL, strings::YES)
                            .with_action(Action::Custom(ids::POPUP_YES)),
                    );
                }
            }
            Popup::Difficulty { hell } => {
                // Backdrop, title and the three buttons (R, N, H); Esc closes.
                for (frame, x) in [(0, 237), (1, 493)] {
                    let mut bd = Control::new(ControlKind::Image, x, 400, 256, 200)
                        .with_art(r"CharSelect\DifficultyLevels");
                    bd.cel = Some(frame);
                    v.push(bd);
                }
                let mut t = Control::new(ControlKind::Text, 264, 260, 272, 35)
                    .with_string(10019)
                    .with_font(FONT24, 2);
                t.color = 7;
                v.push(t);
                for (y, s, k, d) in [
                    (297, 10018, b'R', 0u8),
                    (340, 10017, b'N', 1),
                    (383, 10016, b'H', 2),
                ] {
                    let mut b = button(264, y, 272, 35, r"FrontEnd\WideButtonBlank", s)
                        .with_hotkey(u16::from(k))
                        .with_action(Action::Trigger(Trigger::Difficulty(d)));
                    b.enabled = d < 2 || hell;
                    v.push(b);
                }
                let mut esc = Control::new(ControlKind::Button, 900, 900, 10, 10)
                    .with_hotkey(vk::ESC)
                    .with_action(Action::Custom(ids::POPUP_NO));
                esc.visible = false;
                v.push(esc);
            }
        }
        if m.popup != Popup::None {
            // The pop-up layer owns the input: the list's controls go dead.
            // D2RUST's own, unverified: Esc/Enter belong to the pop-up only.
            let n = v.len();
            let popup_start = v
                .iter()
                .position(|c| c.art == Some(crate::ui::front_end::DIM_LAYER))
                .unwrap_or(n);
            for c in &mut v[..popup_start] {
                c.inert = true;
            }
        }
        v
    }

    /// The paper dolls: entry k = `first` + i in slot i,
    /// anchored at (column x + 30, row bottom − 13).
    fn overlay(&mut self, now_ms: u64, _adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        let Some(m) = self.model.as_ref() else {
            return Vec::new();
        };
        let dt = now_ms.saturating_sub(self.doll_last_ms);
        self.doll_last_ms = self.doll_last_ms.max(now_ms);
        let first = m.first;
        // The add follows each draw, so the time since the last pass goes to the
        // dolls drawn in it; a doll outside the 8 slots is not drawn and keeps its phase, and one
        // that scrolls in shows that phase first (`ui-frontend-charlist`, a05end). The dolls on
        // screen when the list is built count as drawn at the build (`list`, a00down).
        if self.doll_ms.is_empty() {
            self.doll_drawn = (0..SLOTS)
                .filter(|i| m.slot(*i).is_some())
                .map(|i| first + i)
                .collect();
        }
        for k in &self.doll_drawn {
            *self.doll_ms.entry(*k).or_insert(0) += dt;
        }
        self.doll_drawn.clear();
        for i in 0..SLOTS {
            if m.slot(i).is_some() {
                self.doll_ms.entry(first + i).or_insert(0);
                self.doll_drawn.push(first + i);
            }
        }
        let doll_ms = &self.doll_ms;
        let mut out: Vec<DrawItem> = (0..SLOTS)
            .filter_map(|i| {
                let e = m.slot(i)?;
                let (class, mode) = e.paper_doll(self.expansion);
                let mut components = [0xFFu8; 16];
                let mut colours = [0xFFu8; 16];
                components[..11].copy_from_slice(&e.components);
                colours[..11].copy_from_slice(&e.colours);
                Some(DrawItem::Doll {
                    class,
                    mode,
                    components,
                    colours,
                    at: Point::new(
                        canvas_slot(self.compact, i).0 + 30,
                        canvas_slot(self.compact, i).1 - 13,
                    ),
                    draw_mode: if e.dead() { 1 } else { 5 },
                    key: (m.first + i) as u32,
                    epoch: self.built_ms,
                    ticks: (doll_ms.get(&(first + i)).copied().unwrap_or(0) / 40) as u32,
                })
            })
            .collect();
        if self.compact {
            if let Ok(selected) = usize::try_from(m.sel) {
                if selected >= m.first && selected < m.first + SLOTS {
                    let (x, y) = canvas_slot(true, selected - m.first);
                    out.push(DrawItem::Border {
                        at: Point::new(x, y - 92),
                        w: 260,
                        h: 93,
                    });
                }
            }
        }
        out
    }

    fn wheel(&mut self, _ctx: &mut FrontCtx, delta: i32) -> Option<Trigger> {
        self.model.as_mut()?.wheel(delta);
        self.again()
    }

    fn action(&mut self, ctx: &mut FrontCtx, id: u32) -> Option<Trigger> {
        let now = ctx.now_ms;
        match id {
            ids::OK => return self.run_ok(ctx),
            ids::CONVERT => {
                let m = self.model.as_mut()?;
                if m.buttons().convert && m.popup == Popup::None {
                    m.popup = Popup::Convert;
                    return self.again();
                }
            }
            ids::DELETE => {
                let m = self.model.as_mut()?;
                if m.buttons().delete && m.popup == Popup::None {
                    m.popup = Popup::Delete;
                    return self.again();
                }
            }
            ids::POPUP_NO => {
                let m = self.model.as_mut()?;
                if matches!(m.popup, Popup::Message(_))
                    && now.saturating_sub(m.msg_ms) < MESSAGE_GUARD_MS
                {
                    return None;
                }
                m.popup = Popup::None;
                return self.again();
            }
            ids::POPUP_YES => {
                let m = self.model.as_mut()?;
                // Delete and a failed convert rebuild the screen (rescan): the dolls start over. A
                // convert that succeeds only sets the entry's status bit: the dolls keep
                // their phase (`ui-frontend-flow-list`, l10convyes).
                let rescan = match m.popup {
                    Popup::Delete => {
                        m.delete_selected();
                        true
                    }
                    Popup::Convert => {
                        m.convert_selected();
                        m.popup != Popup::None
                    }
                    _ => false,
                };
                self.restart_dolls |= rescan;
                return self.again();
            }
            ids::KEY_HOME..=ids::KEY_DOWN => {
                let k = [
                    key::HOME,
                    key::END,
                    key::LEFT,
                    key::RIGHT,
                    key::UP,
                    key::DOWN,
                ][(id - ids::KEY_HOME) as usize];
                self.model.as_mut()?.key(k);
                return self.again();
            }
            _ => {
                let i = if (ids::SLOT_TEXT..ids::SLOT_TEXT + 8).contains(&id) {
                    id - ids::SLOT_TEXT
                } else if (ids::SLOT_FIGURE..ids::SLOT_FIGURE + 8).contains(&id) {
                    id - ids::SLOT_FIGURE
                } else {
                    return None;
                };
                let double = self.model.as_mut()?.click(i as usize, now);
                if double {
                    if let Some(t) = self.run_ok(ctx) {
                        return Some(t);
                    }
                }
                return self.again();
            }
        }
        None
    }

    // Ends with the palette loader on the two sky paths:
    // re-entering after the create screen (fechar) restores
    // them.
}

/// Register the screen with no save folder (an empty list). The host
/// replaces it with [`register_with`].
pub fn register(reg: &mut Registry) {
    register_with(reg, None, SelectionHandle::default());
}

/// Register the screen on `dir`; the chosen character lands in `handle`.
pub fn register_with(reg: &mut Registry, dir: Option<PathBuf>, handle: SelectionHandle) {
    reg.register(CHAR_SELECT, Box::new(CharSelect::new(dir, handle)));
}

#[cfg(test)]
mod adaptive_navigation_tests {
    use super::*;
    #[test]
    fn compact_rows_remain_aligned_through_keys_scroll_and_layout_switches() {
        for count in [1, 8, 9, 10, 17, 30] {
            let mut model = Model {
                dir: PathBuf::new(),
                expansion: true,
                entries: (0..count)
                    .map(|i| Entry {
                        name: format!("Hero{i}"),
                        class: 0,
                        level: 1,
                        status: 0,
                        components: [0; 11],
                        colours: [0; 11],
                        mtime: 0,
                    })
                    .collect(),
                sel: 0,
                first: 0,
                scroll: 0,
                cb_pos: 0,
                up_pressed: false,
                down_pressed: false,
                msg_ms: 0,
                popup: Popup::None,
                last_click: None,
                columns: 2,
                fixed: None,
                delete_request: None,
            };
            model.set_columns(3);
            for code in [
                key::END,
                key::UP,
                key::LEFT,
                key::RIGHT,
                key::HOME,
                key::DOWN,
                key::DOWN,
                key::END,
            ] {
                model.key(code);
                assert_eq!(model.first % 3, 0);
                let selected = model.sel as usize;
                assert!(selected >= model.first && selected < model.first + SLOTS);
                let slot = selected - model.first;
                assert_eq!(model.slot(slot), model.selected());
                let (x, y) = canvas_slot(true, slot);
                assert!(x >= 0 && x + 260 <= 800 && y - 93 >= 0 && y < 408);
            }
            let selection = model.selected().cloned();
            model.set_columns(2);
            assert_eq!(model.selected(), selection.as_ref());
            model.set_columns(3);
            assert_eq!(model.selected(), selection.as_ref());
            model.scroll_to(model.scroll_range().unwrap_or(0));
            assert_eq!(model.first % 3, 0);
            assert!(model.first < count);
        }
    }
}
