//! Unit shadows of the play preview: the sheared half-height shadow of
//! every COF layer whose shadow byte is set, drawn in the shadow pass with
//! the darkening blend (`rules::blend::unit_shadow_ops`).
//!
//! The shadow of a cel depends only on the cel (the shape,
//! `rules::blend::shadow_image`), so each component frame set made
//! resident also gets a derived `#shadow` set of the same frames; the
//! draw then needs only the position (the unit's draw position).
//!
//! D2RUST's own, unverified (the preview): the motion-record height `oz` is zero (the
//! preview states no unit offset), so the shadow sits at the unit's feet;
//! the no-shadow unit flag and state 146 are not in the model (facts are
//! zero); the COF box pre-test is not applied. The shadow slot's own
//! per-cel pre-test is.

use d2_formats::cof::Cof;

use crate::composite::ComponentDraw;
use crate::frames::{FrameAnchor, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::blend::{shadow_cel_drawn, shadow_image, unit_shadow_ops};
use crate::rules::draw_order::OrderKey;
use crate::rules::placement::draw_position;
use crate::scene::{DrawItem, DrawKey, FrameView, Rect};

use super::{ViewAssets, ViewError};

/// The pixel value of an opaque shadow pixel (the shadow ops ignore it).
const OPAQUE: u8 = 1;

/// The key of the derived shadow set of `set`.
pub fn shadow_key(set: &FrameSetKey) -> Result<FrameSetKey, String> {
    FrameSetKey::new(format!("{}#shadow", set.path()), set.part()).map_err(|e| e.to_string())
}

/// The cel `yoff` of a frame: the bottom row,
/// `None` for a top-down cel (no shadow shape is specified for it).
fn cel_yoff(frame: &IndexFrame) -> Option<i32> {
    match frame.anchor {
        FrameAnchor::Top => Some(frame.y_off + frame.height as i32 - 1),
        FrameAnchor::Bottom => Some(frame.y_off),
        FrameAnchor::TopDown => None,
    }
}

/// The shadow of one frame as a frame whose
/// offsets are relative to the unit's shadow origin (X, Y); a frame with
/// no shadow rows is a 1×1 transparent frame.
pub fn shadow_frame(frame: &IndexFrame) -> IndexFrame {
    let empty = IndexFrame {
        width: 1,
        height: 1,
        x_off: 0,
        y_off: 0,
        anchor: FrameAnchor::Top,
        pixels: vec![0],
    };
    let (Some(yoff), Ok(view)) = (
        cel_yoff(frame),
        FrameView::new(frame.width, frame.height, &frame.pixels),
    ) else {
        return empty;
    };
    let s = shadow_image(&view, 0, 0, frame.x_off, yoff);
    if s.image.width == 0 || s.image.height == 0 {
        return empty;
    }
    IndexFrame {
        width: s.image.width,
        height: s.image.height,
        x_off: s.x,
        y_off: s.y,
        anchor: FrameAnchor::Top,
        pixels: s
            .image
            .pixels
            .iter()
            .map(|&p| if p == 0 { 0 } else { OPAQUE })
            .collect(),
    }
}

/// The shadow set of `set`, frame for frame.
pub fn shadow_set(set: &FrameSet) -> FrameSet {
    FrameSet {
        frames: set.frames.iter().map(shadow_frame).collect(),
    }
}

/// The shadow draws of one unit's component draws `draws`, keyed at the
/// shadow pass slot `at`. A layer without the shadow byte, or whose
/// shadow set is not resident, draws no shadow. Without the act's shade
/// tables (`assets.shades`) the shadow cannot be blended: none is drawn.
pub fn draws(
    cof: &Cof,
    unit_guid: u32,
    at: OrderKey,
    draws: &[ComponentDraw],
    assets: &ViewAssets,
) -> Result<Vec<DrawItem>, ViewError> {
    let Some(tables) = assets.shades.as_ref() else {
        return Ok(Vec::new());
    };
    let (chain, blend) = unit_shadow_ops(tables, crate::world_view::sharp::blended_shadows());
    let mut out = Vec::new();
    for d in draws {
        if cof.layers[d.slot.layer].shadow == 0 {
            continue;
        }
        let Ok(key) = shadow_key(&d.frame.set) else {
            continue;
        };
        let Ok(id) = assets.frames.id(&key, d.frame.index) else {
            continue;
        };
        let (Ok(cel), Ok(shadow)) = (
            assets.frame(&d.frame.set, d.frame.index),
            assets.frame(&key, d.frame.index),
        ) else {
            continue;
        };
        // The unit's (X, Y): the inverse of the cel's placement. The draw row's
        // X - 2 is cancelled by the shear: the
        // measured image is the cel shadow placed at the unit's own X (r2a).
        let (left, top) = draw_position(cel, 0, 0);
        let (x, y) = (d.item.x - left, d.item.y - top);
        // The slot's own pre-test (r2b) on the passed position (X - 2, Y): a
        // cel it rejects is still a shadow call (the draw row) that paints
        // nothing.
        let view = super::play_view();
        let drawn = cel_yoff(cel).is_some_and(|yoff| {
            shadow_cel_drawn(
                x - 2,
                y,
                cel.width as i32,
                cel.height as i32,
                cel.x_off,
                yoff,
                view.width as i32,
                view.height as i32,
            )
        });
        let (sx, sy) = draw_position(shadow, x, y);
        let mut item = DrawItem::new(id, sx, sy);
        // The shadow rows are clipped by the cel clip [0, W) and the frame
        // rows (->), not by the body cel's own rows.
        item.clip = if drawn {
            view
        } else {
            Rect {
                width: 0,
                height: 0,
                ..view
            }
        };
        item.shade = chain;
        item.blend = blend;
        item.key =
            DrawKey::new(at.pass, at.major, at.minor, d.slot.slot).map_err(ViewError::Scene)?;
        item.tag = crate::scene::ItemTag::Unit(unit_guid);
        out.push(item);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
