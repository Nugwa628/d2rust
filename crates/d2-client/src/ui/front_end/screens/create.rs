//! The character-create screen: the campfire line-up, the per-class animation
//! state machine, hover text, the name box, the Hardcore / Expansion check
//! boxes and OK / Exit.
//!
//! The rules live in [`CreateState`] (plain data, tested on synthetic
//! timing); [`CreateScreen`] is the [`Screen`] glue. OK hands the choice to
//! the host through a shared [`NewCharacterSink`]: writing the 335-byte stub
//! save is the host's job, as the screen sees no file system.
//!
//! Preview fills are `// D2RUST's own, unverified`. The name filter and the click
//! during a walk are measured. Hover and click both use the DC6 frame-0 box of the
//! current state's cel ([`HERO_BOXES`]; the hover box of the Barbarian at
//! (400, 330) is x 361..446, y 198..380). PROVISIONAL (the click only): a click
//! on the box edge is not yet measured.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::ui::front_end::control::{vk, Action, Control, ControlKind};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::{DrawItem, Registry, CHAR_CREATE};
use crate::ui::geom::Point;

/// Creation flag bits `[+0x1EF]`.
pub const FLAG_HARDCORE: u8 = 0x04;
pub const FLAG_EXPANSION: u8 = 0x20;

/// Name box limits: at most 15 characters.
pub const NAME_MAX: usize = 15;
pub const NAME_MIN: usize = 2;

/// `Action::Custom` ids of this screen.
pub mod act {
    /// `CLASS + class id`: a click on that hero.
    pub const CLASS: u32 = 100;
    pub const OK: u32 = 20;
    pub const HARDCORE: u32 = 21;
    pub const EXPANSION: u32 = 22;
    pub const BACKSPACE: u32 = 23;
    pub const WARN_OK: u32 = 24;
    pub const WARN_CANCEL: u32 = 25;
    /// The message pop-up's CANCEL.
    pub const MESSAGE_CLOSE: u32 = 26;
}

/// Class ids (save +0x28).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Amazon = 0,
    Sorceress = 1,
    Necromancer = 2,
    Paladin = 3,
    Barbarian = 4,
    Druid = 5,
    Assassin = 6,
}

impl Class {
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> Option<Class> {
        use Class::*;
        [
            Amazon,
            Sorceress,
            Necromancer,
            Paladin,
            Barbarian,
            Druid,
            Assassin,
        ]
        .get(usize::from(id))
        .copied()
    }

    /// Assassin and Druid appear only with the expansion.
    pub fn is_expansion(self) -> bool {
        matches!(self, Class::Druid | Class::Assassin)
    }

    /// (name string id, description string id).
    pub fn strings(self) -> (u32, u32) {
        match self {
            Class::Amazon => (4011, 5128),
            Class::Sorceress => (4010, 5131),
            Class::Necromancer => (4009, 5129),
            Class::Paladin => (4008, 5132),
            Class::Barbarian => (4007, 5130),
            Class::Druid => (10097, 22518),
            Class::Assassin => (10098, 22519),
        }
    }
}

/// One hero of the line-up: class and cel position (x, bottom y), in
/// creation (= draw) order.
pub type Slot = (Class, i32, i32);

pub const CLASSIC: [Slot; 5] = [
    (Class::Barbarian, 400, 330),
    (Class::Necromancer, 301, 333),
    (Class::Sorceress, 521, 344),
    (Class::Amazon, 195, 341),
    (Class::Paladin, 610, 359),
];

pub const EXPANSION: [Slot; 7] = [
    (Class::Barbarian, 400, 330),
    (Class::Sorceress, 626, 353),
    (Class::Paladin, 521, 339),
    (Class::Necromancer, 301, 333),
    (Class::Assassin, 232, 364),
    (Class::Amazon, 100, 337),
    (Class::Druid, 720, 370),
];

/// The campfire: descriptor 178 (expansion) / 179 (classic).
pub const FIRE_EXPANSION: (i32, i32) = (345, 470);
pub const FIRE_CLASSIC: (i32, i32) = (345, 454);

/// Hero control size. DC6 frame-0 box (w, h, ox, oy) of each class's five state
/// cels, indexed by class id then state (made by
/// `tools/facts/create_hero_boxes.py`).
pub const HERO_BOXES: [[(i32, i32, i32, i32); 5]; 7] = [
    [
        (118, 198, -75, 80),
        (118, 198, -75, 80),
        (118, 198, -75, 80),
        (121, 234, -68, 107),
        (121, 233, -67, 106),
    ],
    [
        (89, 166, -61, 53),
        (89, 166, -61, 53),
        (89, 166, -61, 53),
        (107, 209, -52, 74),
        (107, 209, -52, 74),
    ],
    [
        (58, 183, -31, 63),
        (58, 183, -31, 63),
        (58, 183, -31, 63),
        (129, 233, -51, 102),
        (129, 233, -51, 102),
    ],
    [
        (82, 179, -35, 52),
        (82, 179, -35, 52),
        (84, 179, -34, 52),
        (131, 173, -117, 98),
        (132, 173, -117, 98),
    ],
    [
        (86, 183, -39, 51),
        (86, 183, -39, 51),
        (86, 183, -39, 51),
        (95, 201, -50, 67),
        (95, 201, -50, 67),
    ],
    [
        (87, 199, -40, 48),
        (87, 199, -40, 48),
        (87, 199, -40, 48),
        (112, 241, -75, 78),
        (112, 241, -75, 78),
    ],
    [
        (51, 189, -56, 54),
        (51, 189, -56, 54),
        (51, 189, -56, 54),
        (86, 187, -41, 67),
        (86, 187, -41, 67),
    ],
];

pub const HERO_W: u16 = 88;
pub const HERO_H: u16 = 184;

/// The hover and click box of each hero of `slots` (x0, y0, x1, y1, ends
/// exclusive), the hero in `states[i]`: the DC6 frame-0 box of its current
/// cel ([`HERO_BOXES`]), cut at the side where a hero created before it
/// covers it. Neighbours' sprite boxes overlap by a few columns (a selected
/// hero's larger cel by more); the click goes to the first created hero
/// under the pointer (B003, the screen's `first_wins`), so that hero
/// keeps the shared columns. Cut this way no two boxes share a pixel, and
/// hover and click pick the same hero.
pub fn hero_boxes(slots: &[Slot], states: &[u8]) -> Vec<(i32, i32, i32, i32)> {
    let full: Vec<(i32, i32, i32, i32)> = slots
        .iter()
        .zip(states)
        .map(|(&(class, x, y), &state)| {
            let (w, h, ox, oy) = HERO_BOXES[usize::from(class.id())][usize::from(state.min(4))];
            (x + ox, y + oy - h, x + ox + w, y + oy)
        })
        .collect();
    let mut out = full.clone();
    for (j, b) in out.iter_mut().enumerate() {
        for a in &full[..j] {
            let meet = a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3;
            if !meet {
                continue;
            }
            if a.0 > b.0 {
                b.2 = b.2.min(a.0);
            } else if a.2 < b.2 {
                b.0 = b.0.max(a.2);
            }
        }
    }
    out
}

/// Animation states, the index into the anim table.
pub const IDLE: u8 = 0;
pub const HOVER: u8 = 1;
pub const FORWARD: u8 = 2;
pub const SELECTED: u8 = 3;
pub const BACK: u8 = 4;

/// (frames, speed units of 40 ms) per state for a class.
pub fn anim_entry(class: Class, state: u8) -> (u32, u32) {
    // nu1, nu2, fw, nu3, bw
    let t: [(u32, u32); 5] = match class {
        Class::Assassin => [(31, 3), (31, 3), (91, 1), (31, 2), (51, 1)],
        Class::Druid => [(21, 2), (21, 2), (121, 1), (21, 2), (40, 1)],
        Class::Amazon => [(26, 3), (26, 3), (54, 1), (18, 2), (30, 1)],
        Class::Necromancer => [(12, 3), (12, 3), (38, 1), (12, 2), (28, 1)],
        Class::Barbarian => [(16, 2), (16, 2), (64, 1), (26, 1), (19, 1)],
        Class::Sorceress => [(32, 2), (32, 2), (52, 1), (12, 1), (30, 1)],
        Class::Paladin => [(26, 2), (26, 2), (80, 1), (9, 2), (40, 1)],
    };
    t[usize::from(state.min(4))]
}

/// The cel file of a class state, relative to `data\global\ui\` (all under
/// `FrontEnd\<class>\`).
pub fn anim_file(class: Class, state: u8) -> &'static str {
    const F: [[&str; 5]; 7] = [
        [
            "FrontEnd\\amazon\\amnu1",
            "FrontEnd\\amazon\\amnu2",
            "FrontEnd\\amazon\\amfw",
            "FrontEnd\\amazon\\amnu3",
            "FrontEnd\\amazon\\ambw",
        ],
        [
            "FrontEnd\\sorceress\\sonu1",
            "FrontEnd\\sorceress\\sonu2",
            "FrontEnd\\sorceress\\sofw",
            "FrontEnd\\sorceress\\sonu3",
            "FrontEnd\\sorceress\\sobw",
        ],
        [
            "FrontEnd\\necromancer\\nenu1",
            "FrontEnd\\necromancer\\nenu2",
            "FrontEnd\\necromancer\\nefw",
            "FrontEnd\\necromancer\\nenu3",
            "FrontEnd\\necromancer\\nebw",
        ],
        [
            "FrontEnd\\paladin\\panu1",
            "FrontEnd\\paladin\\panu2",
            "FrontEnd\\paladin\\pafw",
            "FrontEnd\\paladin\\panu3",
            "FrontEnd\\paladin\\pabw",
        ],
        [
            "FrontEnd\\barbarian\\banu1",
            "FrontEnd\\barbarian\\banu2",
            "FrontEnd\\barbarian\\bafw",
            "FrontEnd\\barbarian\\banu3",
            "FrontEnd\\barbarian\\babw",
        ],
        [
            "FrontEnd\\druid\\dznu1",
            "FrontEnd\\druid\\dznu2",
            "FrontEnd\\druid\\dzfw",
            "FrontEnd\\druid\\dznu3",
            "FrontEnd\\druid\\dzbw",
        ],
        [
            "FrontEnd\\assassin\\asnu1",
            "FrontEnd\\assassin\\asnu2",
            "FrontEnd\\assassin\\asfw",
            "FrontEnd\\assassin\\asnu3",
            "FrontEnd\\assassin\\asbw",
        ],
    ];
    F[usize::from(class.id())][usize::from(state.min(4))]
}

/// The overlay cel of a class state and its draw mode (the "+file
/// mode"), drawn after the base cel with the same frame.
pub fn anim_overlay(class: Class, state: u8) -> Option<(&'static str, u8)> {
    Some(match (class, state) {
        (Class::Necromancer, FORWARD) => ("FrontEnd\\necromancer\\nefws", 3),
        (Class::Necromancer, SELECTED) => ("FrontEnd\\necromancer\\nenu3s", 3),
        (Class::Necromancer, BACK) => ("FrontEnd\\necromancer\\nebws", 3),
        (Class::Barbarian, FORWARD) => ("FrontEnd\\barbarian\\bafws", 5),
        (Class::Sorceress, FORWARD) => ("FrontEnd\\sorceress\\sofws", 3),
        (Class::Sorceress, SELECTED) => ("FrontEnd\\sorceress\\sonu3s", 3),
        (Class::Sorceress, BACK) => ("FrontEnd\\sorceress\\sobws", 3),
        (Class::Paladin, FORWARD) => ("FrontEnd\\paladin\\pafws", 5),
        _ => return None,
    })
}

/// One hero's animation control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeroAnim {
    pub class: Class,
    pub state: u8,
    /// `[+0x48]`, ms.
    pub t0: u64,
    /// The frame stored by the last pass: the update reads it,
    /// so a walk ends one pass after its frame reaches `frames − 2`.
    pub stored: u32,
}

impl HeroAnim {
    pub fn new(class: Class, now: u64) -> Self {
        Self {
            class,
            state: IDLE,
            t0: now,
            stored: 0,
        }
    }

    /// Frame count and frame duration (ms) of the current state.
    fn timing(&self) -> (u32, u64) {
        let (frames, speed) = anim_entry(self.class, self.state);
        (frames, u64::from(speed) * 1000 / 25)
    }

    /// Frame shown at `now`: states 0, 1, 3 loop over
    /// `frames − 1` (the last frame never shows); 2 and 4 play once.
    pub fn frame(&self, now: u64) -> u32 {
        let (frames, dur) = self.timing();
        if frames <= 1 || dur == 0 {
            return 0;
        }
        let n = (now.saturating_sub(self.t0) / dur) as u32;
        match self.state {
            FORWARD | BACK => n.min(frames - 1),
            _ => n % (frames - 1),
        }
    }

    /// Update (before every draw): idle/hover follow the pointer (t0 kept); a
    /// finished walk moves on and restarts.
    pub fn update(&mut self, inside: bool, now: u64) {
        match self.state {
            IDLE | HOVER => self.state = if inside { HOVER } else { IDLE },
            FORWARD | BACK => {
                let (frames, _) = self.timing();
                if self.stored + 2 >= frames {
                    self.state = if self.state == FORWARD {
                        SELECTED
                    } else {
                        HOVER
                    };
                    self.t0 = now;
                }
            }
            _ => {}
        }
        self.stored = self.frame(now);
    }

    /// Restart the current state (a click or a walk's end): frame 0, t0:= now.
    fn restart(&mut self, state: u8, now: u64) {
        self.state = state;
        self.t0 = now;
        self.stored = 0;
    }
}

/// Whether `c` may be typed into the name box at unit `caret`.
/// Measured: letters
/// only, at most 15; a `-` / `_` is refused at caret 0 even when text follows
/// (Home, then `-` on "ab" leaves "ab"), accepted inside, and refused when the
/// text already holds one; digits, space, 0xC0 / 0xE9 / 0xFF, `.` and `'` are
/// refused. The screen's caret is always at the end (it has no Home / Left /
/// Right: `q-fix-create-name-caret`).
pub fn accepts_char(text: &str, caret: usize, c: char) -> bool {
    if text.chars().count() >= NAME_MAX {
        return false;
    }
    match c {
        'A'..='Z' | 'a'..='z' => true,
        '-' | '_' => caret != 0 && !text.contains(['-', '_']),
        _ => false,
    }
}

/// OK enable rule.
pub fn name_valid(name: &str) -> bool {
    let n = name.chars().count();
    let sep = |c: char| c == '-' || c == '_';
    (NAME_MIN..=NAME_MAX).contains(&n)
        && !name.starts_with(sep)
        && !name.ends_with(sep)
        && name.chars().filter(|&c| sep(c)).count() < 2
}

/// Whether `<dir>/<name>.d2s` exists, ignoring case (the Windows file system is
/// case-insensitive).
pub fn name_taken_in(dir: &Path, name: &str) -> bool {
    let want = format!("{name}.d2s").to_ascii_lowercase();
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == want)
        })
        .unwrap_or(false)
}

/// What OK produced: the host writes the stub save from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCharacter {
    pub name: String,
    pub class: Class,
    pub hardcore: bool,
    pub expansion: bool,
}

/// Why OK did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OkResult {
    /// OK disabled (no class or invalid name): the click is ignored.
    Disabled,
    /// Popup 5165 "That character name is already taken."
    NameTaken,
    /// Hardcore popup 5303 up; answer with [`CreateState::warning_ok`] /
    /// [`CreateState::warning_cancel`].
    Warning,
    Created(NewCharacter),
}

pub const STR_NAME_TAKEN: u32 = 5165;
pub const STR_HARDCORE_WARNING: u32 = 5303;

/// The screen's data.
pub struct CreateState {
    expansion_installed: bool,
    pub heroes: Vec<HeroAnim>,
    pub selected: Option<Class>,
    /// Hovered class for the texts.
    pub hovered: Option<Class>,
    pub name: String,
    pub flags: u8,
    pub warning: bool,
    pub name_taken: bool,
    /// OK turned off by a taken name until the next edit.
    pub ok_off: bool,
}

impl CreateState {
    /// Entry: nothing selected, flags 0.
    pub fn new(expansion_installed: bool, now: u64) -> Self {
        let slots: &[Slot] = if expansion_installed {
            &EXPANSION
        } else {
            &CLASSIC
        };
        Self {
            expansion_installed,
            heroes: slots.iter().map(|s| HeroAnim::new(s.0, now)).collect(),
            selected: None,
            hovered: None,
            name: String::new(),
            flags: 0,
            warning: false,
            name_taken: false,
            ok_off: false,
        }
    }

    fn hero(&self, c: Class) -> Option<&HeroAnim> {
        self.heroes.iter().find(|h| h.class == c)
    }

    /// Per-frame update with the class under the pointer (if any).
    pub fn update(&mut self, under_pointer: Option<Class>, now: u64) {
        for h in &mut self.heroes {
            h.update(under_pointer == Some(h.class), now);
        }
    }

    /// Hover callbacks: texts follow the pointer only while
    /// nothing is selected.
    pub fn hover(&mut self, class: Option<Class>) {
        if self.selected.is_some() {
            return;
        }
        self.hovered = class;
    }

    /// Texts 197 / 198 (name id, description id), if any.
    pub fn texts(&self) -> Option<((u32, u32), Class)> {
        let c = self.selected.or(self.hovered)?;
        Some((c.strings(), c))
    }

    /// A click on a hero. Returns whether the
    /// click was accepted.
    pub fn click(&mut self, class: Class, now: u64) -> bool {
        let Some(i) = self.heroes.iter().position(|h| h.class == class) else {
            return false;
        };
        // The state is the one the last update pass left: a
        // hero the pointer was not over at that pass is idle, the click
        // callback still runs but the walk does not start (the walk
        // moves state 1 → 2 and 3 → 4 only).
        let s = self.heroes[i].state;
        // r6.2, in creation order: a selected hero walks back; a hero that
        // is already walking ends the click (heroes visited stay switched).
        for h in &mut self.heroes {
            match h.state {
                SELECTED => h.restart(BACK, now),
                FORWARD | BACK => return false,
                _ => {}
            }
        }
        let prev = self.selected.take();
        // r6.5
        match s {
            HOVER if prev != Some(class) => self.selected = Some(class),
            FORWARD | SELECTED | BACK => self.selected = None,
            _ => self.selected = Some(class),
        }
        // click callback result: hover → forward (state 3 already went back).
        if s == HOVER {
            self.heroes[i].restart(FORWARD, now);
        }
        self.hovered = None;
        // r6.6 / r6.7
        if self.selected.is_some() && self.expansion_installed {
            // every new selection re-checks Expansion
            self.flags |= FLAG_EXPANSION;
        }
        true
    }

    /// Name box and OK are shown only with a class selected.
    pub fn name_box_visible(&self) -> bool {
        self.selected.is_some()
    }

    /// The hardcore pair is offered with a class selected.
    pub fn hardcore_visible(&self) -> bool {
        self.selected.is_some()
    }

    /// Expansion check box visible; the grey box replaces it for
    /// Assassin / Druid.
    pub fn expansion_box_visible(&self) -> bool {
        self.expansion_installed && self.selected.is_some_and(|c| !c.is_expansion())
    }

    pub fn expansion_grey_visible(&self) -> bool {
        self.expansion_installed && self.selected.is_some_and(Class::is_expansion)
    }

    /// OK enabled: a class and a valid name.
    pub fn ok_enabled(&self) -> bool {
        self.selected.is_some() && name_valid(&self.name) && !self.warning && !self.ok_off
    }

    /// A typed character; returns whether it was accepted. Only while the
    /// name box is shown.
    pub fn type_char(&mut self, c: char) -> bool {
        let caret = self.name.chars().count();
        if !self.name_box_visible() || self.warning || !accepts_char(&self.name, caret, c) {
            return false;
        }
        self.name.push(c);
        self.name_taken = false;
        self.ok_off = false;
        true
    }

    pub fn backspace(&mut self) {
        if self.name_box_visible() && !self.warning {
            self.name.pop();
            self.name_taken = false;
            self.ok_off = false;
        }
    }

    /// Hardcore box click.
    pub fn toggle_hardcore(&mut self) {
        if self.hardcore_visible() && !self.warning {
            self.flags ^= FLAG_HARDCORE;
        }
    }

    /// Expansion box click: no-op without the expansion, or
    /// for the classes that show the grey box.
    pub fn toggle_expansion(&mut self) {
        if self.expansion_box_visible() && !self.warning {
            self.flags ^= FLAG_EXPANSION;
        }
    }

    /// OK / Enter. `taken` is the duplicate-name
    /// check against the save folder.
    pub fn ok(&mut self, taken: &dyn Fn(&str) -> bool) -> OkResult {
        if !self.ok_enabled() {
            return OkResult::Disabled;
        }
        if taken(&self.name) {
            self.name_taken = true;
            self.ok_off = true;
            return OkResult::NameTaken;
        }
        if self.flags & FLAG_HARDCORE != 0 {
            self.warning = true;
            return OkResult::Warning;
        }
        OkResult::Created(self.finish())
    }

    fn finish(&mut self) -> NewCharacter {
        let class = self.selected.unwrap_or(Class::Amazon);
        if class.is_expansion() {
            self.flags |= FLAG_EXPANSION;
        }
        NewCharacter {
            name: self.name.clone(),
            class,
            hardcore: self.flags & FLAG_HARDCORE != 0,
            expansion: self.flags & FLAG_EXPANSION != 0,
        }
    }

    /// The warning's OK.
    pub fn warning_ok(&mut self) -> Option<NewCharacter> {
        if !self.warning {
            return None;
        }
        self.warning = false;
        Some(self.finish())
    }

    /// The warning's CANCEL: the whole screen is rebuilt, so
    /// selection, name and flags are lost.
    pub fn warning_cancel(&mut self, now: u64) {
        *self = Self::new(self.expansion_installed, now);
    }

    /// Frame of a hero for drawing.
    pub fn hero_frame(&self, class: Class, now: u64) -> Option<(u8, u32)> {
        self.hero(class).map(|h| (h.state, h.frame(now)))
    }
}

/// Where OK leaves the choice for the host.
pub type NewCharacterSink = Rc<RefCell<Option<NewCharacter>>>;

type TakenFn = Box<dyn Fn(&str) -> bool>;

/// The [`Screen`] glue.
pub struct CreateScreen {
    state: Option<CreateState>,
    taken: TakenFn,
    sink: NewCharacterSink,
    /// Indices into the built control list, for [`Screen::sync`].
    idx: Idx,
    /// Pointer seen: the heroes follow it (else the legacy click rule).
    under: Option<Option<Class>>,
}

/// Control positions in the built list that change with the state.
#[derive(Default)]
struct Idx {
    heroes: Vec<usize>,
    exp_label: Vec<usize>,
    name_box: Vec<usize>,
    hardcore: Vec<usize>,
    expansion: Vec<usize>,
    grey: Vec<usize>,
    ok: Option<usize>,
    warn: Vec<usize>,
    taken: Vec<usize>,
}

impl CreateScreen {
    pub fn new(sink: NewCharacterSink, taken: TakenFn) -> Self {
        Self {
            state: None,
            taken,
            sink,
            idx: Idx::default(),
            under: None,
        }
    }

    pub fn state(&self) -> Option<&CreateState> {
        self.state.as_ref()
    }
}

const BG: &str = "FrontEnd\\CharacterCreate";
/// Fonts of the text descriptors: Font30,
/// Font16.
const FONT30: u16 = 2;
const FONT16: u16 = 1;
/// Font24: the message pop-up text 249.
const FONT24: u16 = 7;
/// Positions of texts 197 and 198 in the built list.
const TEXT_NAME: usize = 1;
const TEXT_DESC: usize = 2;
const BG_EXP: &str = "FrontEnd\\charactercreationscreenEXP";
/// Pop-up art: hardcore warning `PopUpOKCancel`,
/// message `PopUpOK2`, buttons `OkCancelButtonBlank`.
const POPUP_OK_CANCEL: &str = "FrontEnd\\PopUpOKCancel";
const POPUP_OK: &str = "FrontEnd\\PopUpOk2";
const POPUP_BUTTON: &str = "FrontEnd\\OkCancelButtonBlank";
/// 5103 CANCEL, 5102 OK.
const STR_CANCEL: u32 = 5103;
const STR_OK: u32 = 5102;

/// A pop-up text (208 / 249): (268, 320) 264 × 120, margins 10 / 8, flag 2 (centred, wrapped),
/// colour 0. The pop-up layer: the dimming of the whole frame, then the art as two cels with
/// their last row at 349 (one row above the control box), as measured on the character select's
/// pop-ups (`ui-frontend-charsel-popup`, `ui-frontend-menus-b`).
fn push_popup_art(v: &mut Vec<Control>, art: &'static str) {
    v.push(
        Control::new(ControlKind::Image, 0, 600, 800, 600)
            .with_art(crate::ui::front_end::DIM_LAYER),
    );
    for (frame, x) in [(0, 268), (1, 524)] {
        let mut c = Control::new(ControlKind::Image, x, 350, 264, 176).with_art(art);
        c.cel = Some(frame);
        v.push(c);
    }
}

fn popup_text(string_id: u32, font: u16) -> Control {
    let mut c = Control::new(ControlKind::Text, 268, 320, 264, 120)
        .with_string(string_id)
        .with_font(font, 2);
    c.margin = (10, 8);
    c
}

impl Screen for CreateScreen {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        let st = CreateState::new(ctx.expansion, ctx.now_ms);
        self.under = None;
        self.idx = Idx::default();
        let mut v = vec![
            Control::new(ControlKind::Image, 0, 599, 800, 600).with_art(if ctx.expansion {
                BG_EXP
            } else {
                BG
            }),
            // 197 class name, 198 description, 174 title: the creation order
            // of the screen.
            Control::new(ControlKind::Text, 0, 180, 800, 100).with_font(FONT30, 2),
            Control::new(ControlKind::Text, 250, 210, 300, 100).with_font(FONT16, 2),
            Control::new(ControlKind::Text, 0, 80, 800, 50)
                .with_string(5127)
                .with_font(FONT30, 2),
            Control::new(ControlKind::Button, 33, 572, 128, 35)
                .with_art("FrontEnd\\MediumSelButtonBlank")
                .with_string(5101)
                .with_hotkey(vk::ESC)
                .with_action(Action::Trigger(Trigger::Exit)),
            // Descriptor 200 at its own y: a one-row image's cel bottom is the
            // control's y.
            Control::new(ControlKind::Image, 319, 519, 169, 26).with_art("FrontEnd\\textbox"),
            Control::new(ControlKind::Text, 321, 512, 200, 32)
                .with_string(5125)
                .with_font(FONT16, 0)
                .with_color(4),
            {
                let mut e = Control::new(ControlKind::EditBox, 318, 510, 157, 16);
                e.font = 5;
                e
            },
        ];
        self.idx.name_box = vec![v.len() - 3, v.len() - 2, v.len() - 1];
        let (hx, hy, lx, ly) = if ctx.expansion {
            (319, 560, 339, 581)
        } else {
            (319, 540, 339, 561)
        };
        // Creation order:
        // expansion label 211, box 212, grey box 213, then hardcore label
        // 189 and box 190.
        if ctx.expansion {
            self.idx.exp_label = vec![v.len()];
            self.idx.expansion = vec![v.len() + 1];
            self.idx.grey = vec![v.len() + 2];
            // 211: size 200 x 32 as label 201, so "EXPANSION CHARACTER" stays
            // on one row (a 100 px box would wrap it onto the Hardcore
            // label); drawn on one row.
            v.push(
                Control::new(ControlKind::Text, 339, 561, 200, 32)
                    .with_string(22731)
                    .with_font(FONT16, 0)
                    .with_color(4),
            );
            v.push(
                Control::new(ControlKind::Button, 319, 540, 15, 16)
                    .with_art("FrontEnd\\clickbox")
                    .with_action(Action::Custom(act::EXPANSION))
                    .with_on_press(),
            );
            v.push(
                Control::new(ControlKind::Image, 319, 540, 15, 16)
                    .with_art("FrontEnd\\joingameclickboxgrey"),
            );
        }
        self.idx.hardcore = vec![v.len() + 1, v.len()];
        v.push(
            Control::new(ControlKind::Text, lx, ly, 100, 32)
                .with_string(5126)
                .with_font(FONT16, 0)
                .with_color(4),
        );
        v.push(
            Control::new(ControlKind::Button, hx, hy, 15, 16)
                .with_art("FrontEnd\\clickbox")
                .with_action(Action::Custom(act::HARDCORE))
                .with_on_press(),
        );
        self.idx.ok = Some(v.len());
        v.push(
            Control::new(ControlKind::Button, 627, 572, 128, 35)
                .with_art("FrontEnd\\MediumSelButtonBlank")
                .with_string(5102)
                .with_hotkey(vk::ENTER)
                .with_action(Action::Custom(act::OK)),
        );
        let (fx, fy) = if ctx.expansion {
            FIRE_EXPANSION
        } else {
            FIRE_CLASSIC
        };
        let slots: &[Slot] = if ctx.expansion { &EXPANSION } else { &CLASSIC };
        self.idx.heroes.clear();
        for &(class, x, y) in slots {
            self.idx.heroes.push(v.len());
            // Drawn by `overlay` (state-dependent file and frame); the
            // control only takes the click.
            v.push(
                Control::new(ControlKind::Image, x, y, HERO_W, HERO_H)
                    .with_action(Action::Custom(act::CLASS + u32::from(class.id()))),
            );
        }
        let mut fire =
            Control::new(ControlKind::AnimImage, fx, fy, 110, 127).with_art("FrontEnd\\fire");
        fire.late = true;
        v.push(fire);
        v.push(Control::key_only(
            vk::BACKSPACE,
            Action::Custom(act::BACKSPACE),
        ));
        // Hardcore warning: 205 image, 206
        // CANCEL, 207 OK, 208 text 5303; shown while it is up.
        self.idx.warn = (v.len()..v.len() + 6).collect();
        push_popup_art(&mut v, POPUP_OK_CANCEL);
        v.push(
            Control::new(ControlKind::Button, 281, 337, 96, 32)
                .with_art(POPUP_BUTTON)
                .with_string(STR_CANCEL)
                .with_action(Action::Custom(act::WARN_CANCEL))
                .with_on_press(),
        );
        v.push(
            Control::new(ControlKind::Button, 421, 337, 96, 32)
                .with_art(POPUP_BUTTON)
                .with_string(STR_OK)
                .with_action(Action::Custom(act::WARN_OK))
                .with_on_press(),
        );
        v.push(popup_text(STR_HARDCORE_WARNING, FONT16));
        // Message pop-up with 5165: 247 image,
        // 250 CANCEL (Esc), 249 text (Font24).
        self.idx.taken = (v.len()..v.len() + 5).collect();
        push_popup_art(&mut v, POPUP_OK);
        v.push(
            Control::new(ControlKind::Button, 351, 337, 96, 32)
                .with_art(POPUP_BUTTON)
                .with_string(STR_CANCEL)
                .with_hotkey(vk::ESC)
                .with_action(Action::Custom(act::MESSAGE_CLOSE))
                .with_on_press(),
        );
        v.push(popup_text(STR_NAME_TAKEN, FONT24));
        self.state = Some(st);
        v
    }

    fn palette(&self) -> Option<[&'static str; 2]> {
        Some(crate::ui::front_end::FECHAR_PALETTE)
    }

    fn action(&mut self, ctx: &mut FrontCtx, id: u32) -> Option<Trigger> {
        let st = self.state.as_mut()?;
        let done = match id {
            act::HARDCORE => {
                st.toggle_hardcore();
                None
            }
            act::EXPANSION => {
                st.toggle_expansion();
                None
            }
            act::BACKSPACE => {
                st.backspace();
                None
            }
            act::OK => match st.ok(&*self.taken) {
                OkResult::Created(c) => Some(c),
                _ => None,
            },
            act::WARN_OK => st.warning_ok(),
            // The warning's CANCEL rebuilds the whole screen: every control
            // restarts, the fire included.
            act::WARN_CANCEL => return Some(Trigger::Rebuild),
            act::MESSAGE_CLOSE => {
                st.name_taken = false;
                None
            }
            n if n >= act::CLASS => {
                if let Some(c) = Class::from_id((n - act::CLASS) as u8) {
                    st.click(c, ctx.now_ms);
                }
                None
            }
            _ => None,
        };
        done.map(|c| {
            *self.sink.borrow_mut() = Some(c);
            Trigger::Ok
        })
    }

    fn tick(&mut self, ctx: &mut FrontCtx) -> Option<Trigger> {
        let st = self.state.as_mut()?;
        match self.under {
            // The pointer is known: `CreateState::update`.
            Some(under) => st.update(under, ctx.now_ms),
            // No pointer yet: heroes keep their state and only the walks
            // advance.
            None => {
                for h in &mut st.heroes {
                    let inside = h.state == HOVER;
                    h.update(inside, ctx.now_ms);
                }
            }
        }
        None
    }

    fn pointer(&mut self, ctx: &mut FrontCtx, p: Point) {
        let slots: &[Slot] = if ctx.expansion { &EXPANSION } else { &CLASSIC };
        // Hit box: frame 0 of the current state's cel, cut by the heroes on top.
        let under = self.state.as_ref().and_then(|st| {
            let states: Vec<u8> = st.heroes.iter().map(|h| h.state).collect();
            slots.iter().zip(hero_boxes(slots, &states)).rev().find_map(
                |(&(class, _, _), (x0, y0, x1, y1))| {
                    (p.x >= x0 && p.x < x1 && p.y >= y0 && p.y < y1).then_some(class)
                },
            )
        });
        self.under = Some(under);
        if let Some(st) = self.state.as_mut() {
            st.hover(under);
        }
    }

    fn overlay(&mut self, now_ms: u64, _adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        let Some(st) = self.state.as_ref() else {
            return Vec::new();
        };
        let slots: &[Slot] = if st.expansion_installed {
            &EXPANSION
        } else {
            &CLASSIC
        };
        let mut out = Vec::new();
        for &(class, x, y) in slots {
            if let Some((state, frame)) = st.hero_frame(class, now_ms) {
                let at = Point::new(x, y);
                out.push(DrawItem::Art {
                    file: anim_file(class, state),
                    frame,
                    at,
                });
                if let Some((file, mode)) = anim_overlay(class, state) {
                    out.push(DrawItem::Blend {
                        file,
                        frame,
                        at,
                        mode,
                        boxed: false,
                    });
                }
            }
        }
        if st.name_box_visible() {
            // Caret: a trailing `_` shown while `GetTickCount / 250` is odd
            // (edit-box draw).
            out.push(DrawItem::Text {
                label: None,
                string_id: 0,
                text: format!(
                    "{}{}",
                    st.name,
                    if (now_ms / 250) % 2 == 1 { "_" } else { "" }
                ),
                font: 5,
                at: Point::new(326, 514),
                color: 0,
                boxed: None,
            });
        }
        out
    }

    fn sync(&mut self, controls: &mut [Control]) {
        let Some(st) = self.state.as_ref() else {
            return;
        };
        let show = |controls: &mut [Control], ix: &[usize], on: bool| {
            for &i in ix {
                if let Some(c) = controls.get_mut(i) {
                    c.visible = on;
                }
            }
        };
        // Hero click boxes follow the current state's cel.
        let slots: &[Slot] = if st.expansion_installed {
            &EXPANSION
        } else {
            &CLASSIC
        };
        let states: Vec<u8> = st.heroes.iter().map(|h| h.state).collect();
        for (&ix, (x0, y0, x1, y1)) in self.idx.heroes.iter().zip(hero_boxes(slots, &states)) {
            if let Some(c) = controls.get_mut(ix) {
                c.x = x0;
                c.y = y1;
                c.w = (x1 - x0).max(0) as u16;
                c.h = (y1 - y0).max(0) as u16;
            }
        }
        show(controls, &self.idx.name_box, st.name_box_visible());
        show(controls, &self.idx.hardcore, st.hardcore_visible());
        show(controls, &self.idx.expansion, st.expansion_box_visible());
        // The label shows for every class; only the box differs.
        show(
            controls,
            &self.idx.exp_label,
            st.expansion_installed && st.selected.is_some(),
        );
        show(controls, &self.idx.grey, st.expansion_grey_visible());
        // The pop-ups (list A): shown and live only while up.
        for (ix, on) in [
            (&self.idx.warn, st.warning),
            (&self.idx.taken, st.name_taken),
        ] {
            for &i in ix {
                if let Some(c) = controls.get_mut(i) {
                    c.visible = on;
                    c.enabled = on;
                    c.top = true;
                }
            }
        }
        // Behind a pop-up the heroes take no clicks.
        let modal = st.warning || st.name_taken;
        for &i in &self.idx.heroes {
            if let Some(c) = controls.get_mut(i) {
                c.enabled = !modal;
            }
        }
        // The check marks: frame 1 of the box art.
        for (ix, on) in [
            (&self.idx.hardcore, st.flags & FLAG_HARDCORE != 0),
            (&self.idx.expansion, st.flags & FLAG_EXPANSION != 0),
        ] {
            if let Some(c) = ix.first().and_then(|&i| controls.get_mut(i)) {
                c.checked = on;
            }
        }
        // Texts 197 / 198: the hovered or selected class.
        let (name, desc) = st.texts().map_or((0, 0), |(ids, _)| ids);
        for (i, id) in [(TEXT_NAME, name), (TEXT_DESC, desc)] {
            if let Some(c) = controls.get_mut(i) {
                c.string_id = id;
            }
        }
        if let Some(c) = self.idx.ok.and_then(|i| controls.get_mut(i)) {
            c.visible = st.name_box_visible();
            c.enabled = st.ok_enabled();
        }
    }

    fn char(&mut self, _ctx: &mut FrontCtx, unit: u16) {
        if let (Some(st), Some(c)) = (self.state.as_mut(), char::from_u32(u32::from(unit))) {
            st.type_char(c);
        }
    }

    /// The heroes are controls in the screen's list, walked from the
    /// first created on a mouse-down (a selected hero's larger cel
    /// overlaps its neighbours).
    fn first_wins(&self) -> Option<std::ops::Range<usize>> {
        let h = &self.idx.heroes;
        Some(*h.first()?..h.last()? + 1)
    }

    fn loads_sky_palette(&self) -> bool {
        false
    }
}

/// Register with a host-supplied sink and duplicate-name check.
pub fn register_with(reg: &mut Registry, sink: NewCharacterSink, taken: TakenFn) {
    reg.register(CHAR_CREATE, Box::new(CreateScreen::new(sink, taken)));
}

/// Default registration: no save folder known, nothing is taken, and the
/// choice goes to a sink nobody reads (the host uses [`register_with`]).
pub fn register(reg: &mut Registry) {
    register_with(reg, Rc::default(), Box::new(|_| false));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two heroes' boxes overlap in any mix of animation states, none is
    /// empty, and the columns two idle neighbours share go to the one
    /// created first, which takes the click there (the Sorceress and
    /// Paladin of the expansion line-up share x 565..567).
    #[test]
    fn hero_boxes_never_overlap() {
        for slots in [&CLASSIC[..], &EXPANSION[..]] {
            let n = slots.len();
            for code in 0..5usize.pow(n as u32) {
                let states: Vec<u8> = (0..n)
                    .map(|i| (code / 5usize.pow(i as u32) % 5) as u8)
                    .collect();
                let boxes = hero_boxes(slots, &states);
                for (i, a) in boxes.iter().enumerate() {
                    assert!(a.0 < a.2 && a.1 < a.3, "{states:?}: {:?} empty", slots[i].0);
                    for (j, b) in boxes.iter().enumerate().skip(i + 1) {
                        let meet = a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3;
                        assert!(
                            !meet,
                            "{states:?}: {:?} overlaps {:?}",
                            slots[i].0, slots[j].0
                        );
                    }
                }
            }
        }
        let boxes = hero_boxes(&EXPANSION, &[IDLE; 7]);
        let (sorc, pal) = (boxes[1], boxes[2]);
        assert_eq!((sorc.0, sorc.2), (565, 654));
        assert_eq!((pal.0, pal.2), (486, 565));
    }

    #[test]
    fn name_filter_vectors() {
        assert!(accepts_char("", 0, 'Z'));
        assert!(!accepts_char("", 0, '7'));
        assert!(!accepts_char("Zz", 2, ' '));
        assert!(!accepts_char("Zz", 2, 'é'));
        assert!(!accepts_char("", 0, '-'));
        assert!(accepts_char("Zz", 2, '-'));
        assert!(!accepts_char("a-b", 3, '_'));
        assert!(!accepts_char(&"a".repeat(15), 15, 'b'));
        assert!(accepts_char(&"a".repeat(14), 14, 'b'));
    }

    /// The measured name-box steps.
    #[test]
    fn name_filter_matches_the_recording() {
        // "-" then "_" into the empty box: both refused.
        assert!(!accepts_char("", 0, '-') && !accepts_char("", 0, '_'));
        // "a1 #b-c_d" → "ab-cd".
        let mut t = String::new();
        for c in "a1 #b-c_d".chars() {
            if accepts_char(&t, t.len(), c) {
                t.push(c);
            }
        }
        assert_eq!(t, "ab-cd");
        // "ab", Home, "-": refused at caret 0 although text follows.
        assert!(!accepts_char("ab", 0, '-'));
        // caret 1: "_" accepted ("a_b"); then "-" refused.
        assert!(accepts_char("ab", 1, '_'));
        assert!(!accepts_char("a_xb", 3, '-'));
        // 0xE9, 0xC0, 0xFF, space, 0, 9, '.', '\'' refused; "-" then "_"
        // after "a": "a-", the "_" refused.
        for c in ['\u{e9}', '\u{c0}', '\u{ff}', ' ', '0', '9', '.', '\''] {
            assert!(!accepts_char("a", 1, c), "{c:?}");
        }
        assert!(accepts_char("a", 1, '-') && !accepts_char("a-", 2, '_'));
        // 17 letters typed: 15 kept.
        let mut t = String::new();
        for c in "QRSTUVWXYZabcdefg".chars() {
            if accepts_char(&t, t.len(), c) {
                t.push(c);
            }
        }
        assert_eq!(t, "QRSTUVWXYZabcde");
    }

    #[test]
    fn ok_name_rule() {
        assert!(name_valid("Zz"));
        assert!(!name_valid("Z"));
        assert!(!name_valid("Zz-"));
        assert!(!name_valid("_Zz"));
        assert!(!name_valid("a-b_c"));
        assert!(name_valid("a-bc"));
        assert!(!name_valid(&"a".repeat(16)));
    }

    #[test]
    fn loop_never_shows_last_frame() {
        let a = HeroAnim::new(Class::Paladin, 0);
        // panu1: 26 frames, speed 2 = 80 ms.
        assert_eq!(a.frame(0), 0);
        assert_eq!(a.frame(79), 0);
        assert_eq!(a.frame(80), 1);
        assert_eq!(a.frame(80 * 25), 0);
    }
}

#[cfg(test)]
mod adaptive_canvas_tests {
    use super::*;

    #[test]
    fn canvas_changes_keep_the_selected_class_and_typed_name() {
        let mut screen = CreateScreen::new(Rc::default(), Box::new(|_| false));
        let mut flow = crate::ui::front_end::flow::FlowCtx::default();
        let mut settings = crate::ui::front_end::screen::SettingsSlot::default();
        let mut ctx = FrontCtx {
            expansion: true,
            flow: &mut flow,
            now_ms: 0,
            settings: &mut settings,
        };
        screen.build(&mut ctx);
        let st = screen.state.as_mut().unwrap();
        assert!(st.click(Class::Amazon, 0));
        for c in "Hero".chars() {
            assert!(st.type_char(c), "{c}");
        }
        st.toggle_hardcore();
        let name = st.name.clone();
        let selected = st.selected;
        let hardcore = st.flags & FLAG_HARDCORE != 0;
        for frame in [
            crate::rules::camera::FrameSize {
                width: 960,
                height: 540,
            },
            crate::rules::camera::FrameSize {
                width: 801,
                height: 541,
            },
            crate::rules::camera::FrameSize {
                width: 800,
                height: 1600,
            },
            crate::rules::camera::FrameSize::D2RUST,
        ] {
            screen.canvas_changed(crate::ui::front_end::canvas::FrontCanvas::new(frame));
            let _ = screen.relayout(&mut ctx);
            let st = screen.state.as_ref().unwrap();
            assert_eq!(st.name, name, "{frame:?}");
            assert_eq!(st.selected, selected, "{frame:?}");
            assert_eq!(st.flags & FLAG_HARDCORE != 0, hardcore, "{frame:?}");
        }
    }
}
