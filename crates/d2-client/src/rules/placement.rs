//! What a draw at (X, Y) covers: cel placement for both orientations,
//! the rasterizer's row clipping, DT1 blocks, and the
//! D2RUST mapping onto `IndexFrame` offsets and the `DrawItem` top-left.
//! Integer math only.
//!
//! A cel is a picture with an offset (`xoff`, `yoff`) from the point it is drawn at.
//! Normally cels are stored bottom-up: `yoff` names the cel's bottom row and the picture
//! rises from there. DC6 frames with the orientation bit set are stored top-down instead:
//! `yoff` names their top row. The rasterizer clips rows against the bottom of the frame in
//! a way that differs between the two (see [`RowPlan`]); this module reproduces it, places
//! DT1 floor and wall blocks, and maps all of it onto the `IndexFrame` offsets and the
//! `DrawItem` top-left the compositor works with. Integer math only.
//!
//! Transparency is the frame decoders' job: palette index 0 of an `IndexFrame` is
//! transparent (DC6 runs and DT1 block pixels are taken never to store an opaque index 0).

use crate::frames::{FrameAnchor, IndexFrame};
use crate::scene::Rect;

/// The cel fields the row drawer reads: `w`, `h`, `xoff`, `yoff` and bit 0 of the
/// orientation word (`top_down`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cel {
    pub width: u32,
    pub height: u32,
    pub xoff: i32,
    pub yoff: i32,
    /// The orientation bit is set: the cel is stored top-down.
    pub top_down: bool,
}

impl Cel {
    /// Columns covered by a draw at `x`, inclusive: `X + xoff …
    /// X + xoff + w − 1`.
    pub fn columns(&self, x: i32) -> (i64, i64) {
        let first = i64::from(x) + i64::from(self.xoff);
        (first, first + i64::from(self.width) - 1)
    }

    /// Rows covered by a draw at `y`, inclusive, before clipping: `Y + yoff − h + 1 …
    /// Y + yoff` for a bottom-up cel, `Y + yoff … Y + yoff + h − 1` for a top-down one.
    pub fn rows(&self, y: i32) -> (i64, i64) {
        let anchor = i64::from(y) + i64::from(self.yoff);
        let h = i64::from(self.height);
        if self.top_down {
            (anchor, anchor + h - 1)
        } else {
            (anchor - h + 1, anchor)
        }
    }

    /// The rasterizer's row plan for a draw at `y` into a frame `height` rows high.
    pub fn row_plan(&self, y: i32, height: i32) -> RowPlan {
        RowPlan::new(self, y, height)
    }
}

/// Which encoded rows the rasterizer draws, and where.
///
/// The plan is computed as if every cel ran bottom-up from row `Y + yoff`, whatever its
/// orientation: rows below the frame's last row are skipped, and drawing starts at
/// `min(Y + yoff, H − 1)` for at most `Y + yoff + 1` rows. A top-down cel is then drawn
/// downwards from that start, which is why it can lose its top rows rather than its bottom
/// ones ([`place_cel`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowPlan {
    /// Encoded rows skipped (the clamp of the start to row `H − 1`).
    pub skip: u32,
    /// Screen row of the first drawn encoded row.
    pub start: i64,
    /// Encoded rows drawn (0: none).
    pub count: u32,
    /// The drawer's direction: down (top-down cel) or up (bottom-up).
    pub top_down: bool,
}

impl RowPlan {
    fn new(cel: &Cel, y: i32, height: i32) -> Self {
        let bottom = i64::from(y) + i64::from(cel.yoff);
        let last = i64::from(height) - 1;
        let skip = (bottom - last).max(0);
        let start = bottom.min(last);
        let left = (i64::from(cel.height) - skip).max(0);
        let count = left.min(start + 1).max(0);
        RowPlan {
            skip: skip.min(i64::from(cel.height)) as u32,
            start,
            count: count as u32,
            top_down: cel.top_down,
        }
    }

    /// Screen row of encoded row `k` (`skip ≤ k < skip + count`).
    pub fn screen_row(&self, k: u32) -> i64 {
        let i = i64::from(k) - i64::from(self.skip);
        if self.top_down {
            self.start + i
        } else {
            self.start - i
        }
    }
}

impl Cel {
    /// The cel the DCC decoder builds for a frame: `xoff`, `yoff` are
    /// the frame header's `x offset`, `y offset` unchanged, and an even
    /// `variable0` leaves it bottom-up (odd ones are refused when frames
    /// are built, `frames::FrameError::DccVariable0`).
    pub fn dcc(x_offset: i32, y_offset: i32, width: u32, height: u32) -> Cel {
        Cel {
            width,
            height,
            xoff: x_offset,
            yoff: y_offset,
            top_down: false,
        }
    }

    /// The cel of a DC6 frame: the frame header is the cel, `flip & 1`
    /// the orientation bit.
    pub fn dc6(flip: u32, width: u32, height: u32, offset_x: i32, offset_y: i32) -> Cel {
        Cel {
            width,
            height,
            xoff: offset_x,
            yoff: offset_y,
            top_down: flip & 1 == 1,
        }
    }
}

/// The `DrawItem` top-left of `frame` drawn at (X, Y):
/// `(X + x_off, Y + y_off − height + 1)` for a DC6 frame with
/// `flip & 1 = 0`, `(X + x_off, Y + y_off)` for every other frame.
pub fn draw_position(frame: &IndexFrame, x: i32, y: i32) -> (i32, i32) {
    let top = match frame.anchor {
        FrameAnchor::Top | FrameAnchor::TopDown => y + frame.y_off,
        FrameAnchor::Bottom => y + frame.y_off - frame.height as i32 + 1,
    };
    (x + frame.x_off, top)
}

/// A frame placed for the compositor: the `DrawItem` top-left and the clip that reproduces
/// the rasterizer's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Placed {
    pub x: i32,
    pub y: i32,
    /// `None`: no pixel is drawn.
    pub clip: Option<Rect>,
}

impl Placed {
    /// Whether `clip` draws the same pixels of a `width × height` image as the plain `frame`
    /// clip (so a draw's shared clip can stand for it).
    pub fn same_as_frame(&self, width: u32, height: u32, frame: Rect) -> bool {
        let image = Rect::new(self.x, self.y, width, height);
        let ours = self
            .clip
            .and_then(|c| c.intersect(&frame))
            .and_then(|c| c.intersect(&image));
        ours == frame.intersect(&image)
    }
}

/// Places `cel` drawn at (X, Y) into `frame` (`[0, W) × [0, H)` at the origin; the columns
/// are clipped to `[0, W)`).
///
/// A bottom-up cel draws exactly its image rows inside `[0, H)`: its clip is the frame. A
/// top-down cel starts at row `min(Y + yoff, H − 1)` after skipping `max(0, Y + yoff −
/// (H − 1))` of its top rows and draws at most `Y + yoff + 1` rows ([`RowPlan`]): its
/// top-left moves up by the skip and the clip keeps only the drawn rows. Rows that would
/// fall below the frame are not drawn.
pub fn place_cel(cel: &Cel, x: i32, y: i32, frame: Rect) -> Placed {
    let left = x + cel.xoff;
    if !cel.top_down {
        return Placed {
            x: left,
            y: y + cel.yoff - cel.height as i32 + 1,
            clip: Some(frame),
        };
    }
    let plan = cel.row_plan(y, frame.height as i32);
    let rows = Rect::new(frame.x, plan.start as i32, frame.width, plan.count);
    Placed {
        x: left,
        y: (plan.start - i64::from(plan.skip)) as i32,
        clip: if plan.count == 0 {
            None
        } else {
            rows.intersect(&frame)
        },
    }
}

/// [`place_cel`] for a D2RUST frame: a DC6 frame as its cel by
/// [`FrameAnchor`]; a DCC box or DT1 image at `(X + x_off, Y + y_off)`
/// clipped to the frame (a DCC frame is a bottom-up cel; DT1 blocks
/// clip per pixel).
pub fn place(frame: &IndexFrame, x: i32, y: i32, clip: Rect) -> Placed {
    let (w, h) = (frame.width, frame.height);
    match frame.anchor {
        FrameAnchor::Top => Placed {
            x: x + frame.x_off,
            y: y + frame.y_off,
            clip: Some(clip),
        },
        FrameAnchor::Bottom => place_cel(&Cel::dc6(0, w, h, frame.x_off, frame.y_off), x, y, clip),
        FrameAnchor::TopDown => place_cel(&Cel::dc6(1, w, h, frame.x_off, frame.y_off), x, y, clip),
    }
}

/// Screen position of DT1 block `(bx, by)`'s pixel `(px, py)` for a tile whose blocks start
/// at `origin` (the drawer's (X, Y), after the floor drawer's −80 and the panel shift,
/// `camera::Camera::block_origin`).
pub fn block_pixel(origin: (i32, i32), block: (i32, i32), pixel: (i32, i32)) -> (i32, i32) {
    (origin.0 + block.0 + pixel.0, origin.1 + block.1 + pixel.1)
}
