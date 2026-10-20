//! The game's panels: draw lists built from `panel-layout.tsv`
//! rows, hit tests and the C→S intents. Everything here is client
//! presentation; an outcome (stat point, buy, hire, travel) leaves only as
//! a [`super::ClientIntent`] that the root forwards through the bridge.
//!
//! Shared parts: the file registry that names [`super::ImageRef`] files,
//! text measuring (the sink owns the fonts), the row
//! emitter and the close button.

pub mod border;
pub mod char_details;
pub mod char_inputs;
pub mod character;
pub mod control;
pub mod cube_items;
pub mod inv_gold;
pub mod inv_items;
mod inv_items_tint;
pub mod inventory;
pub mod menu_box;
pub mod npc;
pub mod npc_menu;
pub mod scroll;
pub mod shop;
pub mod skill_inputs;
pub mod skilltree;
pub mod stash_cube;
pub mod stash_input;
pub mod stash_items;
pub mod waypoint;
pub mod waypoint_rows;

#[cfg(test)]
mod tests;

use super::draw::{ImageRef, ImageRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use super::geom::{Point, FRAME};
use super::layout::{panel_layout, CondEnv, LayoutRow, PanelKey, RowKind, Screen};
use super::text::TextOpts;

/// Class letters of the skill-tree art (table), in class order amazon,
/// sorceress, necromancer, paladin, barbarian, druid, assassin.
pub const CLASS_LETTERS: [char; 7] = ['a', 's', 'n', 'p', 'b', 'd', 'i'];

/// Panel art file names (paths under `data\global\ui\`, lower case, no
/// extension) and their [`ImageRef::file`] ids. Ids are the index in
/// [`UiFiles::names`]: every `draw` file of `panel-layout.tsv` in file order,
/// the `C` placeholder expanded to the seven class letters; the `CC` (class
/// icon prefix) files are not known yet.
#[derive(Clone, Debug)]
pub struct UiFiles {
    names: Vec<String>,
}

impl UiFiles {
    pub fn new(rows: &[LayoutRow]) -> Self {
        let mut names: Vec<String> = Vec::new();
        let mut add = |n: String| {
            if !names.contains(&n) {
                names.push(n);
            }
        };
        for f in rows.iter().filter_map(|r| r.file.as_deref()) {
            let f = f.to_ascii_lowercase();
            if f.contains("cc") && f.starts_with("spells\\cc") {
                continue;
            }
            if let Some(i) = f.find("_c_") {
                for c in CLASS_LETTERS {
                    add(format!("{}_{}_{}", &f[..i], c, &f[i + 3..]));
                }
            } else {
                add(f);
            }
        }
        Self { names }
    }

    /// Adds names not yet registered (lowercased), keeping the ids of the
    /// ones before.
    pub fn extend(&mut self, names: impl IntoIterator<Item = String>) {
        for n in names {
            let n = n.to_ascii_lowercase();
            if !self.names.contains(&n) {
                self.names.push(n);
            }
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The id of a file by name (case-insensitive).
    pub fn id(&self, name: &str) -> Option<u32> {
        let name = name.to_ascii_lowercase();
        self.names
            .iter()
            .position(|n| *n == name)
            .and_then(|i| u32::try_from(i).ok())
    }

    /// Adds `name` (lower case) when it is not a name yet; its id. For
    /// files outside `panel-layout.tsv` (item graphics,
    /// [`inv_items::ITEMS_PREFIX`]).
    pub fn add(&mut self, name: &str) -> u32 {
        if let Some(id) = self.id(name) {
            return id;
        }
        self.names.push(name.to_ascii_lowercase());
        (self.names.len() - 1) as u32
    }

    /// The name of an id.
    pub fn name(&self, id: u32) -> Option<&str> {
        self.names.get(id as usize).map(String::as_str)
    }

    /// The id of a row's file with the class letter `class` substituted
    /// for `C`.
    pub fn row_file(&self, row: &LayoutRow, class: Option<u8>) -> Option<u32> {
        let f = row.file.as_deref()?.to_ascii_lowercase();
        let f = match (f.find("_c_"), class) {
            (Some(i), Some(c)) => {
                let l = *CLASS_LETTERS.get(usize::from(c))?;
                format!("{}_{}_{}", &f[..i], l, &f[i + 3..])
            }
            (Some(_), None) => return None,
            (None, _) => f,
        };
        self.id(&f)
    }
}

/// Text width A in a font, measured where the fonts are
/// (the draw sink). `None` when the font or string is not available.
pub trait TextMeasure {
    fn width(&self, font: u16, text: &[u16]) -> Option<i32>;
}

/// The loaded tables every panel reads.
pub struct PanelTables {
    pub layout: Vec<LayoutRow>,
    pub files: UiFiles,
}

impl PanelTables {
    pub fn load() -> Result<Self, super::layout::LayoutError> {
        let layout = panel_layout()?;
        let files = UiFiles::new(&layout);
        Ok(Self { layout, files })
    }

    /// Rows of one panel, in file order.
    pub fn rows(&self, panel: PanelKey) -> impl Iterator<Item = &LayoutRow> {
        self.layout.iter().filter(move |r| r.panel == panel)
    }

    /// The rows of `panel` named `item` of `kind`.
    pub fn item<'a>(
        &'a self,
        panel: PanelKey,
        item: &'a str,
        kind: RowKind,
    ) -> impl Iterator<Item = &'a LayoutRow> {
        self.rows(panel)
            .filter(move |r| r.item == item && r.kind == kind)
    }
}

/// A cel draw at a draw position (the frame covers columns `X … X + w − 1`,
/// rows `Y − h + 1 … Y`).
pub fn cel(file: u32, frame: u32, x: i32, y: i32) -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef { file, frame },
        at: Point::new(x, y),
        clip: FRAME,
        look: crate::ui::CelLook::PLAIN,
        call: crate::ui::draw::CelCall::Draw,
    })
}

/// A `DrawText` call at pen (x, y), not centered.
pub fn text(s: Vec<u16>, x: i32, y: i32, font: u16, color: u16) -> UiDraw {
    UiDraw::Text(TextRequest {
        text: s,
        at: Point::new(x, y),
        style: TextStyle { font, color },
        opts: TextOpts::default(),
        clip: FRAME,
    })
}

/// "Centered in [a, b]": span `s = b − a + 1`; if width < s the pen
/// x is `a + ((s − width) >> 1)`, else `a`.
pub fn centered_in(a: i32, b: i32, width: i32) -> i32 {
    let s = b - a + 1;
    if width < s {
        a + ((s - width) >> 1)
    } else {
        a
    }
}

/// Emits every `draw` row of `panel` whose conditions hold and whose frame
/// is a plain index, in file order (the art quads, the close button, …).
/// Rows whose frame is a word are the panel's own to draw.
pub fn emit_static_draws(
    t: &PanelTables,
    panel: PanelKey,
    env: &CondEnv,
    class: Option<u8>,
    filter: &dyn Fn(&LayoutRow) -> bool,
    out: &mut dyn UiDrawSink,
) {
    for r in t.rows(panel) {
        if r.kind != RowKind::Draw || !filter(r) || !r.applies(env) {
            continue;
        }
        let super::layout::FrameSpec::Index(f) = r.frame else {
            continue;
        };
        if let Some(file) = t.files.row_file(r, class) {
            out.push(cel(file, f, r.x.eval(&env.screen), r.y.eval(&env.screen)));
        }
    }
}

/// The screen and game facts every panel reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelEnv {
    pub screen: Screen,
    /// Screen open mode 0–3.
    pub open_mode: u8,
    /// Expansion game with the expansion installed.
    pub exp: bool,
}

impl PanelEnv {
    /// A [`CondEnv`] with the pressed flag and the panel's own words.
    pub fn cond<'a>(
        &self,
        pressed: bool,
        extra: &'a dyn Fn(super::layout::Cond) -> bool,
    ) -> CondEnv<'a> {
        CondEnv {
            screen: self.screen,
            open_mode: self.open_mode,
            exp: self.exp,
            pressed,
            extra,
        }
    }
}

/// No panel-specific condition holds.
pub fn no_extra(_: super::layout::Cond) -> bool {
    false
}

/// What a panel's mouse handler asks for, in the game's order. A
/// panel never applies an outcome: intents go to the root's outbox, state
/// changes to [`super::states::UiStates::set`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelOutput {
    /// A C→S message.
    Intent(super::ClientIntent),
    /// `SetUIState(ui, mode, jump)` (mode 0 on; 1 off; 2 toggle).
    SetUi { ui: u8, mode: u8, jump: bool },
    /// A UI sound `(id, 0, 0, 0)` with the site's id (each site has its
    /// own id).
    Sound(i32),
    /// A player event (speech) on the local player (19 `impossible`; 20
    /// `cantuseyet`).
    PlayerEvent(u16),
    /// The NPC dialog text closed before its end (a skip, Esc, the
    /// interaction's end): its speech line stops.
    DialogSpeechEnd,
}

/// A string as UTF-16 code units.
pub fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests_c2ui;
