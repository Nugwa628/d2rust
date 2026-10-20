//! The panel model of the in-game UI: ids, events, responses, the context
//! a panel reads, and the `Panel` trait every panel implements.

use d2_proto::FixedMessage;

use super::draw::UiDrawSink;
use super::geom::{Point, Rect};
use super::ui_ids::UiElement;
use crate::bridge::intent;
use crate::bridge::world::ClientWorld;

/// A panel's id, unique within one [`super::UiRoot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PanelId(pub u16);

/// A widget's id, unique within its panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WidgetId(pub u16);

/// An action of the controls' closed `Action` enum, by index. The enum is
/// owned by `d2-client::controls`; the UI only routes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId(pub u16);

/// Pointer buttons the UI routes: Left and Right are the fixed world
/// buttons (never rebindable); Middle (and the X buttons) run the command
/// bound to keys 0x100–0x102 (by default middle = command 7, automap).
/// Shift / Ctrl / Alt meanings come only from the bindings of commands
/// 36 / 34 / 37, except right up, which reads the event's MK_SHIFT /
/// MK_CONTROL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// One UI event. Positions are in the 800×600 frame: the input layer
/// drops pointer input that maps to [`super::FramePos::Outside`] and sends
/// `CursorLeft` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent {
    CursorMoved(Point),
    CursorLeft,
    Press {
        button: PointerButton,
        at: Point,
    },
    Release {
        button: PointerButton,
        at: Point,
    },
    /// Wheel steps, positive away from the user.
    Wheel {
        steps: i32,
        at: Point,
    },
    /// One UTF-16 code unit of typed text.
    Char(u16),
    Action(ActionId),
}

impl UiEvent {
    /// The frame point of a pointer event; `None` for the others.
    pub fn at(&self) -> Option<Point> {
        match *self {
            UiEvent::CursorMoved(p)
            | UiEvent::Press { at: p, .. }
            | UiEvent::Release { at: p, .. }
            | UiEvent::Wheel { at: p, .. } => Some(p),
            UiEvent::CursorLeft | UiEvent::Char(_) | UiEvent::Action(_) => None,
        }
    }

    /// The event moved `dx` pixels across: the UI box of a frame wider
    /// than the 800-wide layout sits `ui_left` pixels in, so a frame point
    /// is a UI point after `shifted(−ui_left)`. Non-pointer events stay.
    pub fn shifted(self, dx: i32) -> UiEvent {
        let mv = |p: Point| Point::new(p.x + dx, p.y);
        match self {
            UiEvent::CursorMoved(p) => UiEvent::CursorMoved(mv(p)),
            UiEvent::Press { button, at } => UiEvent::Press { button, at: mv(at) },
            UiEvent::Release { button, at } => UiEvent::Release { button, at: mv(at) },
            UiEvent::Wheel { steps, at } => UiEvent::Wheel { steps, at: mv(at) },
            other => other,
        }
    }
}

/// The input layer's side of the UI: drains this frame's input as UI
/// events, in order. Implemented by the controls/input module (task C9:
/// key and button bindings → actions; [`super::edge`] for the cursor).
pub trait UiInput {
    fn drain(&mut self, out: &mut Vec<UiEvent>);
}

/// An encoded client-to-server message (what `bridge::intent` sends).
/// Only the root forwards it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIntent(pub Vec<u8>);

impl ClientIntent {
    pub fn from_message<M: FixedMessage>(msg: &M) -> Self {
        Self(intent::encode(msg))
    }
}

/// A panel's answer to an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiResponse {
    Consumed,
    Ignored,
    Intent(ClientIntent),
}

/// String lookup by table key, UTF-16 as stored.
pub trait StringLookup {
    fn get(&self, key: &str) -> Option<&[u16]>;

    /// Lookup by string-table id (panels name some strings by id). None
    /// until a table by id is wired (`d2-data::strings`).
    fn get_id(&self, _id: u16) -> Option<&[u16]> {
        None
    }
}

/// A lookup with no strings (tests, panels that need none).
pub struct NoStrings;

impl StringLookup for NoStrings {
    fn get(&self, _key: &str) -> Option<&[u16]> {
        None
    }
}

/// Read-only data for drawing and events. Fonts are not here: text
/// layout runs on the draw-sink side, which owns them.
pub struct UiCtx<'a> {
    /// Client frame tick.
    pub tick: u64,
    pub world: &'a ClientWorld,
    pub strings: &'a dyn StringLookup,
}

/// A panel. `rect` and `hit` are integer, in the 800×600 frame.
pub trait Panel {
    fn id(&self) -> PanelId;
    fn rect(&self) -> Rect;
    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink);
    /// A panel drawn right before another open panel rather than at its
    /// own place in the root (the routing order is unchanged): the
    /// inventory family's mode panels before the inventory.
    fn draw_before(&self) -> Option<PanelId> {
        None
    }
    /// Drawn before every other open panel (the routing order is
    /// unchanged): the Esc game menu, the first thing the UI pass draws.
    fn draw_first(&self) -> bool {
        false
    }
    fn hit(&self, p: Point) -> Option<WidgetId>;
    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse;
    /// The clickable elements the panel shows now, with their ids and
    /// rectangles in the frame ([`super::ui_ids`]), from the same layout
    /// its event handling hit-tests. Only asked of open panels; none by
    /// default.
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<UiElement> {
        Vec::new()
    }
}
