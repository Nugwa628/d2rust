//! Plain draw requests a panel emits (spec "panels produce
//! `DrawItem`s").
//!
//! The scene's `DrawItem` belongs to
//! `d2-client::scene` (task C4). Until it lands, panels emit these neutral
//! requests through [`UiDrawSink`]; the scene side implements the sink and
//! turns each request into `DrawItem`s in pass `ui`, keeping the
//! emission order (draw order is list order; equal keys keep build order).

use super::geom::{Point, Rect};
use super::text::TextOpts;

/// A UI image: a frame of a file the panel registry names. Both are
/// opaque ids here; the panels' files and frames are
/// named by [`super::panels::UiFiles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageRef {
    pub file: u32,
    pub frame: u32,
}

/// Font id 0–13 and the caller's text color `k` (0 = no remap, else PL2
/// text-color map `k`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextStyle {
    pub font: u16,
    pub color: u16,
}

/// The palette argument of a cel draw: the plain cel draw has
/// none; the colored cel draw passes `k`, read as pointer `+0xD0 + 4k` of the
/// palette-table block; an item picture passes the item's colour map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Remap {
    /// No remap (`P` none).
    #[default]
    None,
    /// The colored cel draw's `k`: 0 = no remap, 1–12 = PL2 text-colour
    /// map `k`, negative = the text block entries.
    Palette(i32),
    /// Map `c` (0–20) of item palette file `t` (1–8), as
    /// the item colour rule selects it (`rules::shading::item_color`).
    ItemColor { t: u8, c: u8 },
}

/// How a cel is written: draw mode 0–7 (5 opaque; other values draw as 5) and
/// the palette argument. The UI's cel draws pass light 0xFF (no `L`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CelLook {
    pub mode: u8,
    pub remap: Remap,
}

impl CelLook {
    /// The plain panel cel draw: mode 5, no remap.
    pub const PLAIN: CelLook = CelLook {
        mode: DRAW_MODE_OPAQUE,
        remap: Remap::None,
    };
}

impl Default for CelLook {
    fn default() -> Self {
        CelLook::PLAIN
    }
}

/// Draw mode 5, the opaque copy.
pub const DRAW_MODE_OPAQUE: u8 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequest {
    pub image: ImageRef,
    /// The cel draw position (X, Y): the frame covers columns `X … X + w − 1`,
    /// rows `Y − h + 1 … Y`; the sink resolves it.
    pub at: Point,
    pub clip: Rect,
    /// Draw mode and remap ([`CelLook::PLAIN`] for the plain cel draw).
    pub look: CelLook,
    /// The cel wrapper the game calls for it (the rendering
    /// facts' `op`).
    pub call: CelCall,
}

/// Which cel draw wrapper a UI cel is: the plain draw, the
/// row-window draw of the globes, the colour (palette) draw of the skill
/// icons and font glyphs, the clipped draw of the automap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CelCall {
    #[default]
    Draw,
    Ex,
    Color,
    Clipped,
}

/// What the rendering facts print of a UI draw besides its wrapper: the draw
/// mode and the palette / colour argument, kept per UI draw in
/// `WorldFrame::ui_calls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiCelInfo {
    pub call: CelCall,
    pub mode: u8,
    /// The palette argument: `Some(0)` null, `Some(k)` the colour index of
    /// the colour draw; `None` not known (an item palette pointer).
    pub pal: Option<i32>,
    /// A text draw: each glyph's colour index is read from its shade map.
    pub text: bool,
}

impl CelCall {
    /// The `draws.tsv` op name.
    pub fn op(self) -> &'static str {
        match self {
            CelCall::Draw => "CelDraw",
            CelCall::Ex => "CelDrawEx",
            CelCall::Color => "CelDrawColor",
            CelCall::Clipped => "CelDrawClipped",
        }
    }
}

/// A filled rectangle: the arguments of `D2GFX_DrawRectangle` as the caller
/// passes them. Columns `x0 … x1 − 1`, rows `y0 … y1 − 1` after the clamp to
/// the surface; `color` is a palette index; `mode` picks the write (blend kind
/// 0: `color`, 1: `T[d]`, 2: `T[256·d + color]`). No clip: the game clamps
/// to the surface only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RectRequest {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub color: u8,
    pub mode: u8,
}

impl RectRequest {
    /// The UI's rectangle primitive
    /// = `DrawRectangle(x, y, x + w, y + h, color, mode)`.
    pub fn sized(x: i32, y: i32, w: i32, h: i32, color: u8, mode: u8) -> Self {
        RectRequest {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
            color,
            mode,
        }
    }
}

/// A UI line (`(x0, y0, x1, y1, color, alpha)` →
/// `D2GFX_DrawLine`): the GDI line's
/// pixels, written opaque with the palette index `color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineRequest {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub color: u8,
}

/// Text as UTF-16 code units, the way the string tables hold them (spec);
/// layout is done by the sink through [`super::text::layout_text`]. `at` is the
/// pen: the bottom row of the first-drawn line. `opts` is the text call and its
/// arguments. The game's text calls take no clip rectangle (decision CG2):
/// `clip` is the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRequest {
    pub text: Vec<u16>,
    pub at: Point,
    pub style: TextStyle,
    pub opts: TextOpts,
    pub clip: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiDraw {
    Image(ImageRequest),
    Text(TextRequest),
    Rect(RectRequest),
    Line(LineRequest),
}

impl UiDraw {
    /// Moves the draw `dx` pixels across, its clip with it: how the
    /// 800-wide UI box is placed in a wider frame ([`crate::rules::camera::FrameSize::ui_left`]).
    pub fn shift_x(&mut self, dx: i32) {
        match self {
            UiDraw::Image(r) => {
                r.at.x += dx;
                r.clip.x += dx;
            }
            UiDraw::Text(r) => {
                r.at.x += dx;
                r.clip.x += dx;
            }
            UiDraw::Rect(r) => {
                r.x0 += dx;
                r.x1 += dx;
            }
            UiDraw::Line(l) => {
                l.x0 += dx;
                l.x1 += dx;
            }
        }
    }

    /// An image or text clipped to exactly `from` is clipped to `to`
    /// instead (the UI box's whole-screen clip widened to a wider frame).
    pub fn widen_clip(&mut self, from: Rect, to: Rect) {
        let clip = match self {
            UiDraw::Image(r) => &mut r.clip,
            UiDraw::Text(r) => &mut r.clip,
            UiDraw::Rect(_) | UiDraw::Line(_) => return,
        };
        if *clip == from {
            *clip = to;
        }
    }
}

/// Receives a panel's draw requests in order.
pub trait UiDrawSink {
    fn push(&mut self, d: UiDraw);
}

/// A sink that moves every draw `dx` pixels across before passing it on.
/// A panel that places its draws in frame space (over world units) writes
/// through `ShiftSink(out, −ui_left)`, so the UI box's own shift brings
/// them back to where the world drew the unit.
pub struct ShiftSink<'a>(pub &'a mut dyn UiDrawSink, pub i32);

impl UiDrawSink for ShiftSink<'_> {
    fn push(&mut self, mut d: UiDraw) {
        d.shift_x(self.1);
        self.0.push(d);
    }
}

impl UiDrawSink for Vec<UiDraw> {
    fn push(&mut self, d: UiDraw) {
        Vec::push(self, d);
    }
}

/// Translate panel art while keeping its whole-canvas clip fixed.
pub struct OffsetSink<'a>(pub &'a mut dyn UiDrawSink, pub i32, pub i32);

impl UiDrawSink for OffsetSink<'_> {
    fn push(&mut self, mut d: UiDraw) {
        match &mut d {
            UiDraw::Image(r) => {
                r.at.x += self.1;
                r.at.y += self.2;
            }
            UiDraw::Text(r) => {
                r.at.x += self.1;
                r.at.y += self.2;
            }
            UiDraw::Rect(r) => {
                r.x0 += self.1;
                r.x1 += self.1;
                r.y0 += self.2;
                r.y1 += self.2;
            }
            UiDraw::Line(r) => {
                r.x0 += self.1;
                r.x1 += self.1;
                r.y0 += self.2;
                r.y1 += self.2;
            }
        }
        self.0.push(d);
    }
}

#[cfg(test)]
mod shift_tests {
    use super::*;
    use crate::ui::panel::{PointerButton, UiEvent};

    /// The UI box's shift moves a draw and its clip together, and a
    /// pointer event's point; frame space and UI space differ by it alone.
    #[test]
    fn the_ui_box_shift_moves_draws_and_events() {
        let mut d = UiDraw::Image(ImageRequest {
            image: ImageRef { file: 1, frame: 2 },
            at: Point::new(10, 20),
            clip: Rect::new(0, 0, 800, 600),
            look: CelLook::PLAIN,
            call: CelCall::Draw,
        });
        d.shift_x(134);
        let UiDraw::Image(i) = &d else { unreachable!() };
        assert_eq!(
            (i.at, i.clip),
            (Point::new(144, 20), Rect::new(134, 0, 800, 600))
        );
        let mut r = UiDraw::Rect(RectRequest::sized(5, 6, 10, 10, 0, 0));
        r.shift_x(134);
        assert_eq!(r, UiDraw::Rect(RectRequest::sized(139, 6, 10, 10, 0, 0)));

        let press = UiEvent::Press {
            button: PointerButton::Left,
            at: Point::new(534, 300),
        };
        let ui = press.shifted(-134);
        assert_eq!(ui.at(), Some(Point::new(400, 300)));
        assert_eq!(ui.shifted(134), press);
        assert_eq!(UiEvent::Char(65).shifted(-134), UiEvent::Char(65));
    }

    /// A draw clipped to the whole UI box gets the whole frame when the
    /// box sits in a wider frame (the side panels open past the box);
    /// any other clip stays.
    #[test]
    fn the_whole_box_clip_widens_to_the_frame() {
        let (box_clip, frame) = (Rect::new(0, 0, 800, 600), Rect::new(-134, 0, 1068, 600));
        let image = |clip| {
            UiDraw::Image(ImageRequest {
                image: ImageRef { file: 1, frame: 2 },
                at: Point::new(-100, 20),
                clip,
                look: CelLook::PLAIN,
                call: CelCall::Draw,
            })
        };
        let mut d = image(box_clip);
        d.widen_clip(box_clip, frame);
        d.shift_x(134);
        let UiDraw::Image(i) = &d else { unreachable!() };
        assert_eq!((i.at.x, i.clip), (34, Rect::new(0, 0, 1068, 600)));
        let own = Rect::new(10, 10, 50, 50);
        let mut d = image(own);
        d.widen_clip(box_clip, frame);
        assert_eq!(d, image(own));
    }
}
