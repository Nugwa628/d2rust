//! The interface a front-end screen implements, and the registry that maps
//! a [`ScreenId`] to its implementation. A screen module registers itself
//! from its own `register` function (see `screens/mod.rs`); no shared code
//! changes when one is added.

use std::collections::BTreeMap;

use super::control::{vk, Action, Control};
use super::flow::{FlowCtx, Trigger};
use super::{DrawItem, ScreenId};
use crate::ui::geom::Point;
use crate::ui::ui_ids::UiElement;

/// What a screen may read and change while it runs.
pub struct FrontCtx<'a> {
    /// The player's install has the expansion.
    pub expansion: bool,
    /// The flow facts; a screen sets `difficulties_open` before it fires
    /// [`Trigger::Ok`].
    pub flow: &'a mut FlowCtx,
    /// Milliseconds since the front end started (tick × 40, C0).
    pub now_ms: u64,
    /// The player's settings, shared with the game's Options pages: the
    /// host fills them in and saves them when a screen changes them.
    pub settings: &'a mut SettingsSlot,
}

/// The settings the front end shows, whether a screen changed them since
/// the host last saved them, and what the Options rows need to know about
/// the machine: whether the sound device is up (the sound rows are off
/// without it), the monitor's physical size (the Window Size row offers
/// what fits) and its refresh rate (with VSync on, the Frame Limit row
/// greys the limits above it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsSlot {
    pub current: crate::app::config::Settings,
    pub changed: bool,
    pub sound_device: bool,
    pub monitor: Option<(u32, u32)>,
    pub refresh_hz: Option<u32>,
    /// The window as the platform reports it now (the Video page's info
    /// lines and the Window Size row read it).
    pub window: crate::ui::original::options_menu::WindowFacts,
}

impl Default for SettingsSlot {
    fn default() -> Self {
        SettingsSlot {
            current: Default::default(),
            changed: false,
            sound_device: true,
            monitor: None,
            refresh_hz: None,
            window: Default::default(),
        }
    }
}

/// One screen. All methods have defaults, so a placeholder is empty.
pub trait Screen {
    /// New committed canvas; this does not re-enter or reset the screen.
    fn canvas_changed(&mut self, _canvas: super::canvas::FrontCanvas) {}

    /// A screen whose list topology changes can rebuild from retained state.
    fn relayout(&mut self, _ctx: &mut FrontCtx) -> Option<Vec<Control>> {
        None
    }

    /// The controls, in creation (= draw) order. Called on every (re)entry:
    /// re-entering rebuilds the controls, so a screen always opens fresh.
    fn build(&mut self, _ctx: &mut FrontCtx) -> Vec<Control> {
        Vec::new()
    }

    /// A control with [`Action::Custom`] fired. May return a flow trigger.
    fn action(&mut self, _ctx: &mut FrontCtx, _id: u32) -> Option<Trigger> {
        None
    }

    /// Called once per 40 ms tick while the screen is current.
    fn tick(&mut self, _ctx: &mut FrontCtx) -> Option<Trigger> {
        None
    }

    /// A character typed (UTF-16 unit) while the screen is current.
    fn char(&mut self, _ctx: &mut FrontCtx, _unit: u16) {}

    /// The pointer moved in the canvas's central 800-column layout.
    fn pointer(&mut self, _ctx: &mut FrontCtx, _p: Point) {}

    /// Mouse wheel, `delta` in units of 120 per notch.
    fn wheel(&mut self, _ctx: &mut FrontCtx, _delta: i32) -> Option<Trigger> {
        None
    }

    /// The left button went down (`down`) or up at `p`, after the
    /// controls had it. Screens that lay out their own rows use this.
    fn mouse(&mut self, _ctx: &mut FrontCtx, _down: bool, _p: Point) -> Option<Trigger> {
        None
    }

    /// A key went down that no control holds (virtual-key code).
    fn key(&mut self, _ctx: &mut FrontCtx, _vk: u16) -> Option<Trigger> {
        None
    }

    /// A key went up (virtual-key code).
    fn key_up(&mut self, _ctx: &mut FrontCtx, _vk: u16) {}

    /// The middle mouse button went down.
    fn middle_down(&mut self, _ctx: &mut FrontCtx) {}

    /// Items drawn after the controls: per-tick content the control list
    /// cannot hold (scrolling text, animated heroes). `adv` = text width in
    /// a font.
    fn overlay(&mut self, _now_ms: u64, _adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        Vec::new()
    }

    /// Adjust the controls to the screen's state (after build and every
    /// tick): show / hide / enable.
    fn sync(&mut self, _controls: &mut [Control]) {}

    /// A run of control indices walked in creation order on a mouse-down:
    /// the first one under the pointer that takes the click wins, not the
    /// topmost (the create screen's heroes: a selected hero's larger cel
    /// overlaps its neighbours). Everything else is "last created wins"
    /// (pop-ups come last). `None` for most screens.
    fn first_wins(&self) -> Option<std::ops::Range<usize>> {
        None
    }

    /// The clickable elements the screen lays out itself (rows it
    /// hit-tests in [`Screen::mouse`] rather than controls), with their ids
    /// and rectangles on the current frame ([`crate::ui::ui_ids`]). The
    /// controls' ids come from the front end
    /// ([`super::FrontEnd::ui_elements`]).
    fn ui_elements(&self) -> Vec<UiElement> {
        Vec::new()
    }

    /// Whether entering the screen loads the sky palette (the menu screens
    /// drawn over the sky art do). The character screens draw
    /// their own background and leave the palette as it was.
    fn loads_sky_palette(&self) -> bool {
        true
    }

    /// The palette files entering the screen loads (`None`: keep the
    /// current one): the sky palette when [`Screen::loads_sky_palette`].
    fn palette(&self) -> Option<[&'static str; 2]> {
        self.loads_sky_palette().then_some(super::SKY_PALETTE)
    }
}

/// A screen nobody implemented yet: Esc → [`Trigger::Exit`], Enter →
/// [`Trigger::Ok`] (the menus' usual hotkeys), no art.
#[derive(Debug, Default)]
pub struct Placeholder;

impl Screen for Placeholder {
    fn build(&mut self, _ctx: &mut FrontCtx) -> Vec<Control> {
        vec![
            Control::key_only(vk::ESC, Action::Trigger(Trigger::Exit)),
            Control::key_only(vk::ENTER, Action::Trigger(Trigger::Ok)),
        ]
    }
}

/// `ScreenId` → implementation. A missing id runs as a [`Placeholder`].
#[derive(Default)]
pub struct Registry {
    screens: BTreeMap<&'static str, Box<dyn Screen>>,
}

impl Registry {
    pub fn register(&mut self, id: ScreenId, screen: Box<dyn Screen>) {
        self.screens.insert(id.0, screen);
    }

    pub fn is_registered(&self, id: ScreenId) -> bool {
        self.screens.contains_key(id.0)
    }

    pub(super) fn first_wins(&self, id: ScreenId) -> Option<std::ops::Range<usize>> {
        self.screens.get(id.0)?.first_wins()
    }

    pub(super) fn ui_elements(&self, id: ScreenId) -> Vec<UiElement> {
        self.screens
            .get(id.0)
            .map_or_else(Vec::new, |s| s.ui_elements())
    }

    pub(super) fn get_mut(&mut self, id: ScreenId) -> &mut dyn Screen {
        self.screens
            .entry(id.0)
            .or_insert_with(|| Box::new(Placeholder))
            .as_mut()
    }
}
