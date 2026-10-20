//! Scene: the draw list and the CPU reference compositor.
//!
//! Everything the client draws is first described as a list of `DrawItem`s (a sprite frame, a
//! position, a clip, a shade chain, a blend op and a sort key); this module sorts that list and
//! composes it into an index framebuffer. Plain Rust, no Bevy types, integer math only: the GPU
//! compositor (`render`) and the verify harness consume exactly these types.
//!
//! Flow: build `Vec<DrawItem>` → [`order`] (stable sort by [`DrawKey`]) →
//! [`cpu::compose`] (or [`bins::bin`] + [`cpu::compose_binned`], the GPU's model) → index
//! framebuffer → [`cpu::to_rgba`].
//!
//! Game rules that decide what to draw, where, and with which shade and blend are not decided
//! here. They come in from the world builder and the unit composite as already-built items.
//!
//! Submodules: `item` (the draw item, shade chains, blend ops), `order` (draw keys and passes),
//! `cpu` (reference compositor), `bins` (screen bins for the GPU), `frame` (persistent
//! framebuffer and frame cycle).

pub mod bins;
pub mod cpu;
pub mod frame;
pub mod item;
pub mod order;

#[cfg(test)]
mod frame_rules_tests;
#[cfg(test)]
mod tests;

pub use bins::{bin, Bins};
pub use cpu::{
    compose, compose_binned, compose_binned_frame, compose_frame, compose_rgba, to_rgba,
};
pub use frame::{present_palette, FrameCycle, FramePlan, PL2_PALETTE_BYTES, UNCLEARED_ROWS};
pub use item::{
    gradient, BlendOp, CelArgs, CelPal, DrawItem, FrameId, FrameImage, FrameSource, FrameView,
    GradientKind, ItemTag, LightGradient, MapId, MapTable, PixelTables, ShadeChain,
};
pub use order::{order, DrawKey};

/// Width of the composed frame.
pub const FRAME_WIDTH: u32 = 800;
/// Height of the composed frame.
pub const FRAME_HEIGHT: u32 = 600;
/// Side of a square bin in pixels.
pub const BIN_SIZE: u32 = 32;
/// Longest shade chain.
pub const MAX_SHADE: usize = 4;

/// An integer screen rectangle, y down. Edges are computed in `i64`, so no
/// position or size overflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Rect {
    /// The full 800×600 frame at the origin.
    pub const FRAME: Rect = Rect {
        x: 0,
        y: 0,
        width: FRAME_WIDTH,
        height: FRAME_HEIGHT,
    };

    /// The default clip of a world draw: the 800 × 600 rectangle, widened
    /// to the play frame when that is wider (the 16:9 frame). At 4:3 it is
    /// exactly [`Rect::FRAME`], so 4:3 frames compose as they always have.
    pub fn world_clip() -> Rect {
        let play = crate::rules::camera::FrameSize::play();
        Rect {
            x: 0,
            y: 0,
            width: FRAME_WIDTH.max(play.width as u32),
            height: FRAME_HEIGHT.max(play.height as u32),
        }
    }

    /// A rectangle from its top-left corner and size.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// The overlap of two rectangles, or `None` when it is empty.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= i64::from(x0) || y1 <= i64::from(y0) {
            return None;
        }
        // Width and height are bounded by one input's u32 size.
        Some(Rect {
            x: x0,
            y: y0,
            width: (x1 - i64::from(x0)) as u32,
            height: (y1 - i64::from(y0)) as u32,
        })
    }

    /// Whether the screen pixel `(x, y)` lies inside.
    pub fn contains(&self, x: i64, y: i64) -> bool {
        x >= i64::from(self.x) && x < self.right() && y >= i64::from(self.y) && y < self.bottom()
    }

    /// A view's pixels must be screen points (i32): items are placed in
    /// i32, and bin rectangles ([`Bins::rect`]) are `Rect`s.
    pub(crate) fn check_view(&self) -> Result<(), SceneError> {
        let end = i64::from(i32::MAX) + 1;
        if self.right() > end || self.bottom() > end {
            return Err(SceneError::View(*self));
        }
        Ok(())
    }
}

/// Errors of draw-list construction and composition. Inputs are strict: anything the compositor
/// cannot draw exactly is an error, never a skipped or defaulted draw.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SceneError {
    /// A shade chain has more maps than [`MAX_SHADE`].
    #[error("shade chain of {len} maps (at most {MAX_SHADE})")]
    ShadeChainTooLong { len: usize },
    /// A draw key field does not fit its bits.
    #[error("draw key field {field} = {value} exceeds {max}")]
    KeyField {
        field: &'static str,
        value: u32,
        max: u32,
    },
    /// A draw item names a frame the frame source does not hold.
    #[error("frame {0:?} is not resident")]
    FrameMissing(FrameId),
    /// A frame's pixel count does not match its size.
    #[error("frame {width}x{height} has {len} pixels")]
    FrameSize { width: u32, height: u32, len: usize },
    /// A shade map id is not in the map table.
    #[error("map {0:?} is not in the map table")]
    MapMissing(MapId),
    /// A blend table does not have its 256 rows in the map table.
    #[error("blend table at {0:?} needs 256 rows in the map table")]
    BlendTable(MapId),
    /// A light gradient does not have its 32 light maps in the map table.
    #[error("light gradient at {0:?} needs 32 light maps in the map table")]
    LightMaps(MapId),
    /// A gradient-lit item draws outside its gradient block.
    #[error("drawn area {area:?} leaves the gradient block {block:?}")]
    GradientArea { area: Rect, block: Rect },
    /// An item asked for horizontal mirroring, which no draw supports.
    #[error("flip_x is always false: no draw mirrors its frame")]
    FlipX,
    /// A view reaches past the 32-bit screen range.
    #[error("view {0:?} has pixels past the i32 screen range")]
    View(Rect),
    /// The bins were built for a different view than the one being composed.
    #[error("bins were built for {built:?}, composing {view:?}")]
    BinsView { built: Rect, view: Rect },
    /// The bins were built for a different number of items.
    #[error("bins were built for {built} items, composing {items}")]
    BinsItems { built: usize, items: usize },
    /// The base framebuffer is not the size of the view.
    #[error("base framebuffer has {len} bytes, the view {pixels} pixels")]
    BaseSize { len: usize, pixels: u64 },
    /// The framebuffer is too short for the start-of-frame clear.
    #[error(
        "framebuffer height {height} leaves no rows for the frame clear (more than 47 needed)"
    )]
    FramebufferHeight { height: u32 },
    /// The frame plan does not fit the framebuffer or the frame cycle.
    #[error("frame plan {0:?} does not fit the framebuffer or the frame cycle")]
    FramePlan(frame::FramePlan),
    /// The PL2 data is too short to hold a palette.
    #[error("PL2 data of {len} bytes has no 1,024-byte palette")]
    Pl2Size { len: usize },
    /// An error in one draw item, with its position in the list.
    #[error("draw item {index}: {error}")]
    Item {
        index: usize,
        error: Box<SceneError>,
    },
}

impl SceneError {
    fn at(self, index: usize) -> SceneError {
        SceneError::Item {
            index,
            error: Box::new(self),
        }
    }
}
