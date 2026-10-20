//! The client-side overrides a mod game installs (`docs/plans/mods/seams.md`
//! MS14): [`ClientMods`].
//!
//! Each mod is its own game, built on the D2RUST crates (see
//! `d2_sim::rules`). The simulation's overrides travel in the game's
//! `Ruleset`; what a mod game changes in how the client *shows* the game
//! (extra HUD text, extra lines on an item's hover tip, …) is registered
//! here instead. The registry is a Bevy resource the mod game's binary
//! inserts into its app before the UI is added (`app::ui::add_original_ui`
//! reads it); the D2RUST exe never inserts one, and an empty registry is
//! the same as none.
//!
//! Rules every hook follows:
//! - **Presentation only.** A hook reads the client's model of the game
//!   ([`ClientWorld`], an item's decoded bits) and changes what is drawn.
//!   It never decides an outcome and never sends anything to the server;
//!   game rules stay in the simulation (`ModRules`).
//! - **No change without a hook.** With no hook registered the client
//!   draws exactly what D2RUST draws: no extra panel is added and no extra
//!   work is done, so D2RUST's frames and frame hashes are untouched.
//! - **Order.** Hooks of one kind run in the order they were registered,
//!   each seeing the output of the ones before it.
//!
//! Hooks so far: [`HudLines`] (text lines drawn with the HUD) and
//! [`ItemLines`] (lines of an item's hover tip), MS14a; panels added or
//! replaced ([`PanelFactory`], [`PanelPlace`]) and the ground item label
//! filter ([`ItemNameFilter`]), MS14b; per-category audio gains
//! ([`AudioGains`], [`SoundCategory`]), MS14c.

use std::fmt;
use std::sync::Arc;

use bevy::prelude::Resource;
use d2_proto::item_bits::ItemBits;

use crate::bridge::world::ClientWorld;
use crate::ui::geom::Point;
use crate::ui::item_tip::TipLine;
use crate::ui::panel::Panel;
use crate::ui::panels::UiFiles;

/// The client-side overrides of a mod game. Built with the `with_…`
/// methods and inserted as a resource; cheap to clone (the hooks are
/// shared).
#[derive(Resource, Clone, Default)]
pub struct ClientMods {
    hud_lines: Vec<Arc<dyn HudLines>>,
    item_lines: Vec<Arc<dyn ItemLines>>,
    panels: Vec<(PanelPlace, Arc<dyn PanelFactory>)>,
    name_filters: Vec<Arc<dyn ItemNameFilter>>,
    audio_gains: Vec<Arc<dyn AudioGains>>,
}

impl fmt::Debug for ClientMods {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientMods")
            .field("hud_lines", &self.hud_lines.len())
            .field("item_lines", &self.item_lines.len())
            .field("panels", &self.panels.len())
            .field("name_filters", &self.name_filters.len())
            .field("audio_gains", &self.audio_gains.len())
            .finish()
    }
}

impl ClientMods {
    /// A registry with no hooks: the client as D2RUST draws it.
    pub fn new() -> ClientMods {
        ClientMods::default()
    }

    /// No hook of any kind is registered.
    pub fn is_empty(&self) -> bool {
        self.hud_lines.is_empty()
            && self.item_lines.is_empty()
            && self.panels.is_empty()
            && self.name_filters.is_empty()
            && self.audio_gains.is_empty()
    }

    /// Adds a source of HUD text lines.
    pub fn with_hud_lines(mut self, hook: impl HudLines + 'static) -> ClientMods {
        self.hud_lines.push(Arc::new(hook));
        self
    }

    /// Adds a hook over the lines of an item's hover tip.
    pub fn with_item_lines(mut self, hook: impl ItemLines + 'static) -> ClientMods {
        self.item_lines.push(Arc::new(hook));
        self
    }

    /// Adds a panel, or replaces one of the game's ([`PanelPlace`]).
    pub fn with_panel(
        mut self,
        place: PanelPlace,
        factory: impl PanelFactory + 'static,
    ) -> ClientMods {
        self.panels.push((place, Arc::new(factory)));
        self
    }

    /// Adds a filter over ground item labels.
    pub fn with_item_name_filter(mut self, filter: impl ItemNameFilter + 'static) -> ClientMods {
        self.name_filters.push(Arc::new(filter));
        self
    }

    /// The registered panels, in registration order.
    pub fn panels(&self) -> impl Iterator<Item = (PanelPlace, &dyn PanelFactory)> {
        self.panels.iter().map(|(p, f)| (*p, f.as_ref()))
    }

    /// Some ground item label filter is registered.
    pub fn has_item_name_filter(&self) -> bool {
        !self.name_filters.is_empty()
    }

    /// Runs every label filter over `label`, the label of `item` as D2RUST
    /// made it. A filter that hides the label ends the run.
    pub fn filter_item_name(&self, item: &ItemBits, label: &mut ItemLabel) {
        for f in &self.name_filters {
            if !label.shown {
                break;
            }
            f.filter(item, label);
        }
    }

    /// Adds per-category audio gains.
    pub fn with_audio_gains(mut self, gains: impl AudioGains + 'static) -> ClientMods {
        self.audio_gains.push(Arc::new(gains));
        self
    }

    /// Some audio gain hook is registered.
    pub fn has_audio_gains(&self) -> bool {
        !self.audio_gains.is_empty()
    }

    /// Volume `v` of sound `id` in `category` after every gain hook, each
    /// a percentage applied in integer math (truncating, as the player's
    /// own volume settings are).
    pub fn sound_gain(&self, category: SoundCategory, id: i32, v: i32) -> i32 {
        self.audio_gains.iter().fold(v, |v, g| {
            g.percent(category, id).clamp(0, 1_000).wrapping_mul(v) / 100
        })
    }

    /// Some HUD line source is registered (the UI adds the HUD lines panel
    /// only then).
    pub fn has_hud_lines(&self) -> bool {
        !self.hud_lines.is_empty()
    }

    /// Some item tip hook is registered.
    pub fn has_item_lines(&self) -> bool {
        !self.item_lines.is_empty()
    }

    /// This frame's HUD lines of every source, in registration order.
    pub fn hud_lines(&self, world: &ClientWorld) -> Vec<HudLine> {
        let mut out = Vec::new();
        for hook in &self.hud_lines {
            hook.lines(world, &mut out);
        }
        out
    }

    /// Runs every item tip hook over `lines`, the tip of `item` as D2RUST
    /// built it (top line first).
    pub fn item_lines(&self, item: &ItemBits, lines: &mut Vec<TipLine>) {
        for hook in &self.item_lines {
            hook.lines(item, lines);
        }
    }
}

/// One line of mod text drawn with the HUD, in the 800 × 600 UI box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudLine {
    /// UTF-16 text; colour codes (`ÿc`) work as in any UI text.
    pub text: Vec<u16>,
    /// Where the text is drawn (a text draw's position).
    pub at: Point,
    /// Font index (`ui::text::FONTS`).
    pub font: u16,
    /// Text colour index (`ui::item_tip::color`).
    pub color: u16,
}

impl HudLine {
    /// White text in the 16-pixel font at `at`.
    pub fn new(text: impl AsRef<str>, at: Point) -> HudLine {
        HudLine {
            text: text.as_ref().encode_utf16().collect(),
            at,
            font: 1,
            color: crate::ui::item_tip::color::WHITE,
        }
    }
}

/// A source of HUD text lines (MS14a): extra read-outs a mod game shows
/// over the play screen. Called once per drawn frame while the HUD is up;
/// lines are drawn with the control panel's overlays, below the game's
/// boxes and menus.
pub trait HudLines: Send + Sync {
    /// Appends this frame's lines, read from the client's model of the
    /// game.
    fn lines(&self, world: &ClientWorld, out: &mut Vec<HudLine>);
}

/// A hook over an item's hover tip (MS14a): the box drawn when the
/// pointer is over an item in a panel (inventory, stash, cube, store).
/// Called with the item's decoded bits and the tip's lines as D2RUST
/// built them, top line first; the hook may add, change or remove lines.
pub trait ItemLines: Send + Sync {
    fn lines(&self, item: &ItemBits, lines: &mut Vec<TipLine>);
}

/// Where a mod game's panel goes in the play UI (MS14b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelPlace {
    /// A new panel, above the game's panels and below the Esc menu and
    /// the cursor; `open`: open from the start (a panel that is not a UI
    /// state is never opened or closed by the game, so an always-open
    /// panel decides itself when it draws). Its id must not be one the
    /// game uses; ids from [`MOD_PANEL_IDS`] are kept free for mods.
    Add { open: bool },
    /// Takes the place of the game's panel with the same id: same draw and
    /// input order, same open state (a UI state panel keeps opening and
    /// closing with its UI flag).
    Replace,
}

/// Panel ids the game never uses: the first id free for mod panels.
pub const MOD_PANEL_IDS: u16 = 0x1000;

/// Makes a mod game's panel (MS14b). Panels hold UI state that is not
/// shared between threads, so the registry holds the factory and the UI
/// makes the panel when it is installed.
pub trait PanelFactory: Send + Sync {
    /// The art files the panel draws, added to the UI's file list before
    /// the panel is made (`ImageRef::file` ids come from `files`).
    fn files(&self) -> Vec<String> {
        Vec::new()
    }
    /// Makes the panel. `files` gives the ids of the art files.
    fn make(&self, files: &UiFiles) -> Box<dyn Panel>;
}

/// A ground item's name label as the Show Items labels draw it (MS14b).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemLabel {
    /// UTF-16 text, as D2RUST names the item.
    pub text: Vec<u16>,
    /// Text colour index (`ui::item_tip::color`).
    pub color: u16,
    /// Drawn at all; `false` leaves the item without a label.
    pub shown: bool,
}

/// A filter over ground item labels (MS14b): renames, recolours or hides
/// an item's label. Called with the item's decoded bits and the label as
/// D2RUST made it.
pub trait ItemNameFilter: Send + Sync {
    fn filter(&self, item: &ItemBits, label: &mut ItemLabel);
}

/// The category a sound's volume is scaled by (MS14c).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundCategory {
    /// Songs and the sounds that follow the music volume setting.
    Music,
    /// Spoken lines: NPC speech, the player's and hirelings' voices.
    Speech,
    /// Every other sound: units, skills, items, the world, the UI.
    Effects,
}

/// Per-category audio gains (MS14c): a mod game's own volume sliders, or
/// a fixed mix. Asked each time a playing sound's volume is computed,
/// after the player's master and music volume and the ducks, before
/// distance falloff.
pub trait AudioGains: Send + Sync {
    /// The gain of sound `id` in `category`, in percent: 100 leaves it, 0
    /// silences it (clamped to 0–1000).
    fn percent(&self, category: SoundCategory, id: i32) -> i32;
}

#[cfg(test)]
mod tests;
