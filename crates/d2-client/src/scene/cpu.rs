//! CPU reference compositor: the only definition of the D2RUST image. The GPU compositor must
//! match it byte for byte. Straight loops, integers only.
//!
//! A frame is composed onto a base (the previous frame, because the framebuffer persists between
//! frames) with the clears of a [`FramePlan`]; the plain [`compose`] is the single-frame case: an
//! all-0 base and no clear. The result is an index framebuffer; `to_rgba` turns it into the
//! presented image through the frame's palette.

use d2_formats::palette::Palette;

use super::bins::Bins;
use super::frame::FramePlan;
use super::item::{DrawItem, FrameSource, FrameView, MapTable};
use super::{Rect, SceneError};

/// Composes `items` in list order into an index framebuffer of `view`
/// (row-major, `view.width × view.height`, pixel 0 = screen `(view.x,
/// view.y)`), starting from all index 0 with no clear: a single frame
/// without a recorded previous frame. Every item is
/// validated before any pixel is drawn.
pub fn compose<F: FrameSource + ?Sized>(
    items: &[DrawItem],
    frames: &F,
    maps: &MapTable,
    view: Rect,
) -> Result<Vec<u8>, SceneError> {
    compose_frame(items, frames, maps, view, &clear(&view), FramePlan::NONE)
}

/// One frame of the frame cycle: `base` (the
/// previous frame, `view` sized) with rows `0..plan.clear_rows` set to 0,
/// then `items` in list order, then, if `plan.clear_after`, every pixel 0.
/// Returns the presented index frame; inputs are validated first.
pub fn compose_frame<F: FrameSource + ?Sized>(
    items: &[DrawItem],
    frames: &F,
    maps: &MapTable,
    view: Rect,
    base: &[u8],
    plan: FramePlan,
) -> Result<Vec<u8>, SceneError> {
    check_base(&view, base, plan)?;
    let resolved = resolve_all(items, frames, maps, &view)?;
    let stride = view.width as usize;
    let mut out = base.to_vec();
    out[..plan.clear_rows as usize * stride].fill(0);
    for (item, (frame, area)) in items.iter().zip(&resolved) {
        let Some(area) = area else { continue };
        for sy in i64::from(area.y)..i64::from(area.y) + i64::from(area.height) {
            let row = (sy - i64::from(view.y)) as usize * stride;
            for sx in i64::from(area.x)..i64::from(area.x) + i64::from(area.width) {
                let px = &mut out[row + (sx - i64::from(view.x)) as usize];
                *px = item.pixel(frame, maps, sx, sy, area, *px);
            }
        }
    }
    if plan.clear_after {
        out.fill(0);
    }
    Ok(out)
}

/// Composes through `bins` the way the GPU does: per pixel, walk the
/// pixel's bin list in order. Equal to [`compose`] for the same input.
pub fn compose_binned<F: FrameSource + ?Sized>(
    items: &[DrawItem],
    bins: &Bins,
    frames: &F,
    maps: &MapTable,
    view: Rect,
) -> Result<Vec<u8>, SceneError> {
    compose_binned_frame(
        items,
        bins,
        frames,
        maps,
        view,
        &clear(&view),
        FramePlan::NONE,
    )
}

/// [`compose_frame`] the way the GPU does it: per pixel, the start value
/// (0 in the cleared rows, else the base), the pixel's bin list in order,
/// then the post-draw clear. Equal to [`compose_frame`].
pub fn compose_binned_frame<F: FrameSource + ?Sized>(
    items: &[DrawItem],
    bins: &Bins,
    frames: &F,
    maps: &MapTable,
    view: Rect,
    base: &[u8],
    plan: FramePlan,
) -> Result<Vec<u8>, SceneError> {
    if bins.view() != view {
        return Err(SceneError::BinsView {
            built: bins.view(),
            view,
        });
    }
    if bins.item_count() != items.len() {
        return Err(SceneError::BinsItems {
            built: bins.item_count(),
            items: items.len(),
        });
    }
    check_base(&view, base, plan)?;
    let resolved = resolve_all(items, frames, maps, &view)?;
    let mut out = clear(&view);
    let stride = view.width as usize;
    for row in 0..bins.rows() {
        for col in 0..bins.cols() {
            let list = bins.list(col, row);
            let rect = bins.rect(col, row);
            for sy in i64::from(rect.y)..i64::from(rect.y) + i64::from(rect.height) {
                let y = (sy - i64::from(view.y)) as usize;
                for sx in i64::from(rect.x)..i64::from(rect.x) + i64::from(rect.width) {
                    let at = y * stride + (sx - i64::from(view.x)) as usize;
                    let mut value = if y < plan.clear_rows as usize {
                        0
                    } else {
                        base[at]
                    };
                    for &index in list {
                        let index = index as usize;
                        let (frame, area) = &resolved[index];
                        if let Some(a) = area.filter(|a| a.contains(sx, sy)) {
                            value = items[index].pixel(frame, maps, sx, sy, &a, value);
                        }
                    }
                    out[at] = if plan.clear_after { 0 } else { value };
                }
            }
        }
    }
    Ok(out)
}

/// [`compose`] then [`to_rgba`]: the reference image.
pub fn compose_rgba<F: FrameSource + ?Sized>(
    items: &[DrawItem],
    frames: &F,
    maps: &MapTable,
    palette: &Palette,
    view: Rect,
) -> Result<Vec<u8>, SceneError> {
    Ok(to_rgba(&compose(items, frames, maps, view)?, palette))
}

/// Maps an index framebuffer to RGBA8 through the frame palette, alpha 255:
/// the presented image. Every index, 0 included, is a plain palette lookup; a frame has exactly
/// one palette (no per-region palettes).
pub fn to_rgba(indexed: &[u8], palette: &Palette) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(indexed.len() * 4);
    for &i in indexed {
        let c = palette.colors[usize::from(i)];
        rgba.extend_from_slice(&[c.r, c.g, c.b, 255]);
    }
    rgba
}

type Resolved<'f> = Vec<(FrameView<'f>, Option<Rect>)>;

fn resolve_all<'f, F: FrameSource + ?Sized>(
    items: &[DrawItem],
    frames: &'f F,
    maps: &MapTable,
    view: &Rect,
) -> Result<Resolved<'f>, SceneError> {
    view.check_view()?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| item.resolve(frames, maps, view).map_err(|e| e.at(index)))
        .collect()
}

/// The base must cover the view and the cleared rows must lie in it.
fn check_base(view: &Rect, base: &[u8], plan: FramePlan) -> Result<(), SceneError> {
    let pixels = u64::from(view.width) * u64::from(view.height);
    if base.len() as u64 != pixels {
        return Err(SceneError::BaseSize {
            len: base.len(),
            pixels,
        });
    }
    if plan.clear_rows > view.height {
        return Err(SceneError::FramePlan(plan));
    }
    Ok(())
}

fn clear(view: &Rect) -> Vec<u8> {
    vec![0u8; view.width as usize * view.height as usize]
}
