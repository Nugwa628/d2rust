//! Picture rules for three classes of bug that a single frame can't show:
//!
//! - **Flicker** ([`flicker_rules`]): in consecutive frames of a scene where
//!   nothing should blink, pixels that go A -> B -> A (a shadow drawn on one
//!   frame and not the next, two render paths taking turns).
//! - **Hover highlight shape** ([`highlight_rules`]): the brightened pixels
//!   between a shot without and with the pointer on a unit or entrance must
//!   follow that thing's own graphics, not fill a box or a floor tile.
//! - **Upscale factor** ([`upscale_factor`], [`text_scale_rules`]): text goes
//!   through the same display upscaler as the world, so in an enlarged
//!   picture both are made of the same size blocks.
//!
//! Every rule reads pictures only and reports the measured values.
use anyhow::{bail, Result};
use image::RgbaImage;
use serde_json::{json, Value};

use crate::check::Frame;
use crate::Rect;

fn max_channel(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> u8 {
    a.0[..3]
        .iter()
        .zip(&b.0[..3])
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// Pixels bounding box as `[x, y, w, h]`.
#[derive(Clone, Copy, Debug, Default)]
struct Bbox {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    n: usize,
}

impl Bbox {
    fn add(&mut self, x: u32, y: u32) {
        if self.n == 0 {
            (self.x0, self.y0, self.x1, self.y1) = (x, y, x, y);
        } else {
            self.x0 = self.x0.min(x);
            self.y0 = self.y0.min(y);
            self.x1 = self.x1.max(x);
            self.y1 = self.y1.max(y);
        }
        self.n += 1;
    }
    fn merge(&mut self, o: &Bbox) {
        if o.n == 0 {
            return;
        }
        if self.n == 0 {
            *self = *o;
            return;
        }
        self.x0 = self.x0.min(o.x0);
        self.y0 = self.y0.min(o.y0);
        self.x1 = self.x1.max(o.x1);
        self.y1 = self.y1.max(o.y1);
        self.n += o.n;
    }
    fn w(&self) -> u32 {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> u32 {
        self.y1 - self.y0 + 1
    }
    fn json(&self) -> Value {
        json!([self.x0, self.y0, self.w(), self.h()])
    }
}

/// Flicker over `frames` in the given (display) order: a pixel flickers in
/// a triple when the first and third frames agree within `tol` and the
/// middle one differs from both by more than `delta`. A triple with at
/// least `min_pixels` such pixels is a flicker; all of them are reported
/// as one `flicker` violation with the count, the worst triple and the
/// union of their boxes.
pub fn flicker_rules(
    frames: &[Frame],
    tol: u8,
    delta: u8,
    min_pixels: usize,
) -> Result<Vec<Value>> {
    if let Some(first) = frames.first() {
        if frames
            .iter()
            .any(|f| f.image.dimensions() != first.image.dimensions())
        {
            bail!("flicker frames must have the same dimensions");
        }
    }
    let (mut triples, mut union, mut worst) = (0usize, Bbox::default(), None::<(usize, Value)>);
    for w in frames.windows(3) {
        let [a, b, c] = [&w[0].image, &w[1].image, &w[2].image];
        let mut bb = Bbox::default();
        for (x, y, pb) in b.enumerate_pixels() {
            let (pa, pc) = (a.get_pixel(x, y), c.get_pixel(x, y));
            if max_channel(pa, pc) <= tol
                && max_channel(pa, pb) > delta
                && max_channel(pb, pc) > delta
            {
                bb.add(x, y);
            }
        }
        if bb.n >= min_pixels {
            triples += 1;
            union.merge(&bb);
            if worst.as_ref().is_none_or(|(n, _)| bb.n > *n) {
                worst = Some((
                    bb.n,
                    json!({"frames":[w[0].name, w[1].name, w[2].name], "pixels": bb.n, "bbox": bb.json()}),
                ));
            }
        }
    }
    Ok(match worst {
        Some((_, worst)) => vec![json!({"rule":"flicker", "triples": triples,
            "of": frames.len().saturating_sub(2), "bbox": union.json(), "worst": worst})],
        None => vec![],
    })
}

fn luma(p: &image::Rgba<u8>) -> f64 {
    0.299 * p.0[0] as f64 + 0.587 * p.0[1] as f64 + 0.114 * p.0[2] as f64
}

/// Pixels set in the `(2r+1)`-square window around each pixel (pixels
/// outside the picture count as unset).
fn window_counts(mask: &[bool], w: u32, h: u32, r: u32) -> Vec<u32> {
    let (w, h, r) = (w as usize, h as usize, r as usize);
    // Integral image, one row and column larger.
    let mut sum = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0;
        for x in 0..w {
            row += u32::from(mask[y * w + x]);
            sum[(y + 1) * (w + 1) + x + 1] = sum[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            out[y * w + x] = sum[y1 * (w + 1) + x1] + sum[y0 * (w + 1) + x0]
                - sum[y0 * (w + 1) + x1]
                - sum[y1 * (w + 1) + x0];
        }
    }
    out
}

fn dilate(mask: &[bool], w: u32, h: u32, r: u32) -> Vec<bool> {
    window_counts(mask, w, h, r)
        .into_iter()
        .map(|n| n > 0)
        .collect()
}

fn erode(mask: &[bool], w: u32, h: u32, r: u32) -> Vec<bool> {
    let full = (2 * r + 1) * (2 * r + 1);
    window_counts(mask, w, h, r)
        .into_iter()
        .map(|n| n == full)
        .collect()
}

/// The pixels (as indices) of the biggest 8-connected group of `mask`.
fn biggest_group(mask: &[bool], w: u32, h: u32) -> Vec<u32> {
    let mut seen = vec![false; mask.len()];
    let mut best: Vec<u32> = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut group = vec![start as u32];
        seen[start] = true;
        let mut i = 0;
        while i < group.len() {
            let p = group[i];
            let (x, y) = (p % w, p / w);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let q = (ny as u32 * w + nx as u32) as usize;
                    if mask[q] && !seen[q] {
                        seen[q] = true;
                        group.push(q as u32);
                    }
                }
            }
            i += 1;
        }
        if group.len() > best.len() {
            best = group;
        }
    }
    best
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The hover highlight brightens what is hovered. `bases` are shots with
/// the pointer away (before and after, so light that changed between the
/// ticks is not taken for the highlight), `hover` with it on the target.
/// Highlighted pixels are brighter than every base by at least 20 % (and
/// 12 levels); pixels in
/// `ignore` (the pointer) never count. Two views of them are tested:
///
/// - the biggest 8-connected group as lit: `highlight-rect` when it fills
///   90 % of its box, or covers 80 % of the box's outline with an empty
///   inside (a filled or outlined rectangle);
/// - its solid part: holes up to 4 px closed (black art does not
///   brighten), then strokes under 7 px wide opened away (the name label's
///   text, rain, a weapon), and the biggest group left: `highlight-tile`
///   when half of its rows' side edges run at the 2:1 slope
///   of floor cells (an isometric cell or a block of them, not the shape
///   of the art; a tip hidden under the label does not matter);
/// - `highlight-none`: fewer than `min_pixels` lit pixels in the group.
///
/// A highlight that follows sprite art is ragged and passes. The metrics
/// are returned with the violations.
pub fn highlight_rules(
    bases: &[RgbaImage],
    hover: &RgbaImage,
    min_pixels: usize,
    ignore: &[Rect],
) -> Result<(Value, Vec<Value>)> {
    if bases.is_empty()
        || hover.is_empty()
        || bases.iter().any(|b| b.dimensions() != hover.dimensions())
    {
        bail!("highlight shots must have the same nonempty dimensions");
    }
    let (w, h) = hover.dimensions();
    let mut lit = vec![false; (w * h) as usize];
    for (x, y, ph) in hover.enumerate_pixels() {
        let lh = luma(ph);
        let brighter = bases.iter().all(|b| {
            let lb = luma(b.get_pixel(x, y));
            lh >= lb + 12.0 && lh >= lb * 1.2
        });
        if brighter && !ignore.iter().any(|r| r.contains(x, y)) {
            lit[(y * w + x) as usize] = true;
        }
    }
    let total = lit.iter().filter(|l| **l).count();
    let raw = biggest_group(&lit, w, h);
    let mut metrics = json!({"lit_pixels": total, "group_pixels": raw.len()});
    let mut v = Vec::new();
    if raw.len() < min_pixels {
        v.push(json!({"rule":"highlight-none", "pixels": raw.len(), "min": min_pixels}));
        return Ok((metrics, v));
    }
    let mut bb = Bbox::default();
    for p in &raw {
        bb.add(p % w, p / w);
    }
    let area = (bb.w() * bb.h()) as f64;
    let fill = raw.len() as f64 / area;
    // Outline: pixels within 2 px of the box edge, against the edge length.
    let band =
        |x: u32, y: u32| x <= bb.x0 + 1 || x + 1 >= bb.x1 || y <= bb.y0 + 1 || y + 1 >= bb.y1;
    let on_band = raw.iter().filter(|p| band(*p % w, *p / w)).count() as f64;
    let band_len = (2 * (bb.w() + bb.h())) as f64 * 2.0;
    let inner = raw.len() as f64 - on_band;
    let inner_area = ((bb.w().saturating_sub(4)) * (bb.h().saturating_sub(4))).max(1) as f64;
    metrics["bbox"] = bb.json();
    metrics["fill"] = json!(r3(fill));
    metrics["outline"] = json!(r3(on_band / band_len));
    if fill >= 0.9 || (on_band / band_len >= 0.8 && inner / inner_area < 0.3) {
        v.push(json!({"rule":"highlight-rect", "bbox": bb.json(), "fill": r3(fill), "outline": r3(on_band / band_len)}));
        return Ok((metrics, v));
    }
    let closed = erode(&dilate(&lit, w, h, 2), w, h, 2);
    let solid = dilate(&erode(&closed, w, h, 3), w, h, 3);
    let core = biggest_group(&solid, w, h);
    metrics["solid_pixels"] = json!(core.len());
    if core.len() < min_pixels {
        return Ok((metrics, v));
    }
    let mut cb = Bbox::default();
    for p in &core {
        cb.add(p % w, p / w);
    }
    // Isometric edges: a floor cell's sides run two pixels across per row
    // down. Each row's leftmost and rightmost pixel of the solid part,
    // compared with the row `STEP` below: an edge counts when it moved
    // 2 * STEP (within 3 px), as along a diamond's sides; the share is of
    // both edges of every row. A diamond whose tip is hidden (under the name
    // label) still has its sides.
    const STEP: u32 = 8;
    let mut rows = vec![(u32::MAX, 0u32); cb.h() as usize];
    for p in &core {
        let (x, y) = (p % w, p / w);
        let r = &mut rows[(y - cb.y0) as usize];
        r.0 = r.0.min(x);
        r.1 = r.1.max(x);
    }
    let iso = |a: u32, b: u32| (a.abs_diff(b) as i64 - 2 * STEP as i64).abs() <= 3;
    let (mut on, mut of) = (0usize, 0usize);
    for y in 0..rows.len().saturating_sub(STEP as usize) {
        let (a, b) = (rows[y], rows[y + STEP as usize]);
        if a.0 == u32::MAX || b.0 == u32::MAX {
            continue;
        }
        of += 2;
        on += usize::from(iso(a.0, b.0)) + usize::from(iso(a.1, b.1));
    }
    let iso_rows = if of == 0 { 0.0 } else { on as f64 / of as f64 };
    metrics["solid"] = json!({"bbox": cb.json(), "iso_edge_rows": r3(iso_rows)});
    if iso_rows >= 0.5 && cb.w() >= 32 {
        v.push(json!({"rule":"highlight-tile", "bbox": cb.json(), "iso_edge_rows": r3(iso_rows)}));
    }
    Ok((metrics, v))
}

/// The block size a region was enlarged by: for each factor k (1..=6) and
/// block offset, the share of neighbouring pixel pairs that differ (by more
/// than 4 levels) inside a block against across a block edge. An image
/// enlarged k times has nearly equal pixels inside its blocks. The factor
/// is the k whose inside share is the smallest part of its edge share, at
/// most a quarter (the largest such k when several are as clean: a 6x
/// picture splits as cleanly into 2x or 3x blocks); 1 when none is. Only pairs with a pixel selected by `mask` count
/// (all pixels when `None`). Returns (k, inside share, edge share, pairs).
pub fn upscale_factor(
    img: &RgbaImage,
    rect: Rect,
    mask: Option<&dyn Fn(u32, u32) -> bool>,
) -> (u32, f64, f64, usize) {
    let (w, h) = img.dimensions();
    let r = Rect {
        x: rect.x.min(w),
        y: rect.y.min(h),
        w: rect.w.min(w.saturating_sub(rect.x)),
        h: rect.h.min(h.saturating_sub(rect.y)),
    };
    let take = |x: u32, y: u32| mask.is_none_or(|m| m(x, y));
    // Differences of right and down neighbours, with their coordinate along
    // the pair's axis.
    let mut pairs: Vec<(u32, bool)> = Vec::new();
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            let p = img.get_pixel(x, y);
            if x + 1 < r.x + r.w && (take(x, y) || take(x + 1, y)) {
                pairs.push((
                    (x + 1 - r.x) * 2,
                    max_channel(p, img.get_pixel(x + 1, y)) > 4,
                ));
            }
            if y + 1 < r.y + r.h && (take(x, y) || take(x, y + 1)) {
                pairs.push((
                    (y + 1 - r.y) * 2 + 1,
                    max_channel(p, img.get_pixel(x, y + 1)) > 4,
                ));
            }
        }
    }
    let n = pairs.len();
    let mut best = (1, 0.0, 0.0);
    for k in 2..=6u32 {
        // Each axis gets its own best offset.
        let mut inside = (0usize, 0usize);
        let mut edge = (0usize, 0usize);
        for axis in 0..2u32 {
            let mut top = None::<(f64, (usize, usize), (usize, usize))>;
            for o in 0..k {
                let (mut ii, mut ie) = ((0usize, 0usize), (0usize, 0usize));
                for &(pos, diff) in pairs.iter().filter(|(p, _)| p % 2 == axis) {
                    let at = pos / 2;
                    let slot = if (at + k - o) % k == 0 {
                        &mut ie
                    } else {
                        &mut ii
                    };
                    slot.0 += usize::from(diff);
                    slot.1 += 1;
                }
                let rate = |s: (usize, usize)| {
                    if s.1 == 0 {
                        0.0
                    } else {
                        s.0 as f64 / s.1 as f64
                    }
                };
                let score = rate(ie) - rate(ii);
                if top.as_ref().is_none_or(|t| score > t.0) {
                    top = Some((score, ii, ie));
                }
            }
            if let Some((_, ii, ie)) = top {
                inside = (inside.0 + ii.0, inside.1 + ii.1);
                edge = (edge.0 + ie.0, edge.1 + ie.1);
            }
        }
        let rate = |s: (usize, usize)| {
            if s.1 == 0 {
                0.0
            } else {
                s.0 as f64 / s.1 as f64
            }
        };
        let (ri, re) = (rate(inside), rate(edge));
        // The cleanest split wins; a multiple of the true factor also
        // passes but with block edges inside its blocks, so a larger k
        // replaces a smaller one only when it is as clean.
        if re > 0.05 && ri <= 0.25 * re && (best.0 == 1 || ri / re <= best.1 / best.2 + 0.03) {
            best = (k, ri, re);
        }
    }
    (best.0, best.1, best.2, n)
}

/// Text and world enlarged alike. `base` is the picture without the text
/// (a menu closed), `text` with it, both of one held frame: pixels that
/// changed are the text; pixels that did not, inside `world`, are the
/// world. `text-scale-mismatch` when their [`upscale_factor`]s differ.
pub fn text_scale_rules(
    base: &RgbaImage,
    text: &RgbaImage,
    world: Rect,
) -> Result<(Value, Vec<Value>)> {
    if base.dimensions() != text.dimensions() || base.is_empty() {
        bail!("text-scale shots must have the same nonempty dimensions");
    }
    let (w, h) = base.dimensions();
    let changed = |x: u32, y: u32| max_channel(base.get_pixel(x, y), text.get_pixel(x, y)) > 24;
    let mut bb = Bbox::default();
    for y in 0..h {
        for x in 0..w {
            if changed(x, y) {
                bb.add(x, y);
            }
        }
    }
    if bb.n < 64 {
        return Ok((
            json!({"text_pixels": bb.n}),
            vec![json!({"rule":"text-none", "pixels": bb.n})],
        ));
    }
    let text_rect = Rect {
        x: bb.x0,
        y: bb.y0,
        w: bb.w(),
        h: bb.h(),
    };
    let (tk, ti, te, tn) = upscale_factor(text, text_rect, Some(&changed));
    let unchanged = |x: u32, y: u32| !changed(x, y);
    let (wk, wi, we, wn) = upscale_factor(text, world, Some(&unchanged));
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    let metrics = json!({"text": {"factor": tk, "inside": r3(ti), "edge": r3(te), "pairs": tn, "bbox": bb.json()},
                         "world": {"factor": wk, "inside": r3(wi), "edge": r3(we), "pairs": wn,
                                   "rect": [world.x, world.y, world.w, world.h]}});
    let mut v = Vec::new();
    if tk != wk {
        v.push(json!({"rule":"text-scale-mismatch", "text_factor": tk, "world_factor": wk}));
    }
    Ok((metrics, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn frame(name: &str, image: RgbaImage) -> Frame {
        Frame {
            name: name.into(),
            tick: 0,
            phase: 0,
            image,
        }
    }
    fn noise(w: u32, h: u32, seed: u32) -> RgbaImage {
        let mut s = seed.wrapping_mul(2654435761).max(1);
        RgbaImage::from_fn(w, h, |_, _| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let v = (s % 200) as u8 + 20;
            Rgba([v, v / 2, v / 3, 255])
        })
    }
    fn enlarge(img: &RgbaImage, k: u32) -> RgbaImage {
        RgbaImage::from_fn(img.width() * k, img.height() * k, |x, y| {
            *img.get_pixel(x / k, y / k)
        })
    }

    #[test]
    fn a_shadow_drawn_every_other_frame_flickers() {
        let base = noise(32, 32, 1);
        let mut dark = base.clone();
        for y in 10..16 {
            for x in 8..20 {
                dark.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let frames: Vec<Frame> = (0..6)
            .map(|i| {
                frame(
                    &format!("f{i}"),
                    if i % 2 == 0 {
                        base.clone()
                    } else {
                        dark.clone()
                    },
                )
            })
            .collect();
        let v = flicker_rules(&frames, 0, 16, 16).unwrap();
        assert_eq!(v[0]["rule"], "flicker");
        assert_eq!(v[0]["triples"], 4);
        assert_eq!(v[0]["bbox"], json!([8, 10, 12, 6]));
    }

    #[test]
    fn steady_change_and_still_frames_do_not_flicker() {
        // A moving block (A -> B -> C) and identical frames.
        let frames: Vec<Frame> = (0..5)
            .map(|i| {
                let mut img = noise(32, 32, 1);
                for y in 4..10 {
                    for x in (i * 4)..(i * 4 + 6) {
                        img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
                    }
                }
                frame(&format!("m{i}"), img)
            })
            .collect();
        assert!(flicker_rules(&frames, 0, 16, 16).unwrap().is_empty());
        let still: Vec<Frame> = (0..4)
            .map(|i| frame(&format!("s{i}"), noise(16, 16, 3)))
            .collect();
        assert!(flicker_rules(&still, 0, 16, 16).unwrap().is_empty());
    }

    fn brighten(img: &mut RgbaImage, f: impl Fn(u32, u32) -> bool) {
        for (x, y, p) in img.enumerate_pixels_mut() {
            if f(x, y) {
                for c in &mut p.0[..3] {
                    *c = (*c as u32 * 17 / 10).min(255) as u8;
                }
            }
        }
    }

    #[test]
    fn a_box_highlight_is_a_rectangle() {
        let base = RgbaImage::from_pixel(80, 60, Rgba([60, 50, 40, 255]));
        let mut hover = base.clone();
        brighten(&mut hover, |x, y| {
            (20..60).contains(&x) && (10..40).contains(&y)
        });
        let (_, v) = highlight_rules(std::slice::from_ref(&base), &hover, 50, &[]).unwrap();
        assert_eq!(v[0]["rule"], "highlight-rect");
        // An outlined box.
        let mut outline = base.clone();
        brighten(&mut outline, |x, y| {
            (20..60).contains(&x)
                && (10..40).contains(&y)
                && !((22..58).contains(&x) && (12..38).contains(&y))
        });
        assert_eq!(
            highlight_rules(std::slice::from_ref(&base), &outline, 50, &[])
                .unwrap()
                .1[0]["rule"],
            "highlight-rect"
        );
    }

    #[test]
    fn a_floor_cell_highlight_is_a_tile() {
        let base = RgbaImage::from_pixel(200, 100, Rgba([60, 50, 40, 255]));
        let mut hover = base.clone();
        // One floor cell at 1x: a 160 x 80 diamond.
        brighten(&mut hover, |x, y| {
            let (dx, dy) = (
                (x as f64 - 100.0).abs() / 80.0,
                (y as f64 - 50.0).abs() / 40.0,
            );
            dx + dy <= 1.0
        });
        let (m, v) = highlight_rules(std::slice::from_ref(&base), &hover, 50, &[]).unwrap();
        assert_eq!(v[0]["rule"], "highlight-tile", "{m}");
    }

    #[test]
    fn a_floor_block_with_dark_holes_a_label_and_a_spear_is_a_tile() {
        // Two floor cells by two at 2x (a 320 x 160 diamond) over dim
        // ground with black specks, the name label's text touching its top
        // and a thin spear crossing its lower edge.
        let mut base = RgbaImage::from_pixel(420, 260, Rgba([50, 45, 35, 255]));
        let mut s = 7u32;
        for (_, _, p) in base.enumerate_pixels_mut() {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            if s >> 28 == 0 {
                *p = Rgba([0, 0, 0, 255]);
            }
        }
        let mut hover = base.clone();
        brighten(&mut hover, |x, y| {
            (x as f64 - 210.0).abs() / 160.0 + (y as f64 - 130.0).abs() / 80.0 <= 1.0
        });
        for x in 150..270 {
            for y in 40..56 {
                if (x / 3 + y) % 4 == 0 {
                    hover.put_pixel(x, y, Rgba([240, 240, 240, 255]));
                }
            }
        }
        for t in 0..90 {
            hover.put_pixel(240 + t, 190 + t / 3, Rgba([230, 230, 230, 255]));
        }
        let (m, v) = highlight_rules(std::slice::from_ref(&base), &hover, 50, &[]).unwrap();
        assert_eq!(v[0]["rule"], "highlight-tile", "{m}");
    }

    #[test]
    fn a_sprite_shaped_highlight_passes_and_none_is_reported() {
        let base = noise(100, 80, 9);
        let mut hover = base.clone();
        // A ragged shape: an arch with a dark opening and uneven sides.
        brighten(&mut hover, |x, y| {
            let (fx, fy) = (x as f64 - 50.0, y as f64 - 60.0);
            let r = (fx * fx + fy * fy).sqrt();
            let wobble = 4.0 * ((x as f64) * 0.7).sin() + 3.0 * ((y as f64) * 1.3).cos();
            (18.0 + wobble..34.0 + wobble).contains(&r) && fy < 5.0
        });
        let (m, v) = highlight_rules(std::slice::from_ref(&base), &hover, 50, &[]).unwrap();
        assert!(v.is_empty(), "{v:?} {m}");
        assert_eq!(
            highlight_rules(std::slice::from_ref(&base), &base, 50, &[])
                .unwrap()
                .1[0]["rule"],
            "highlight-none"
        );
    }

    #[test]
    fn light_that_grows_between_the_shots_is_not_a_highlight() {
        // A square of light fading in: the hover shot is brighter than the
        // shot before it, but the shot after is brighter still.
        let lit = |level: u8| {
            let mut i = RgbaImage::from_pixel(120, 120, Rgba([40, 40, 40, 255]));
            for y in 20..100 {
                for x in 20..100 {
                    i.put_pixel(x, y, Rgba([level, level, level, 255]));
                }
            }
            i
        };
        let (before, hover, after) = (lit(40), lit(80), lit(120));
        let one = highlight_rules(std::slice::from_ref(&before), &hover, 50, &[]).unwrap();
        assert_eq!(one.1[0]["rule"], "highlight-rect");
        let both = highlight_rules(&[before, after], &hover, 50, &[]).unwrap();
        assert_eq!(both.1[0]["rule"], "highlight-none", "{}", both.0);
    }

    #[test]
    fn the_upscale_factor_of_enlarged_pictures() {
        let small = noise(40, 30, 5);
        let all = |img: &RgbaImage| Rect {
            x: 0,
            y: 0,
            w: img.width(),
            h: img.height(),
        };
        assert_eq!(upscale_factor(&small, all(&small), None).0, 1);
        for k in [2, 3] {
            let big = enlarge(&small, k);
            assert_eq!(upscale_factor(&big, all(&big), None).0, k);
        }
    }

    #[test]
    fn text_drawn_at_a_different_scale_than_the_world_mismatches() {
        let world = enlarge(&noise(60, 40, 7), 2);
        // Text 2x like the world: passes.
        let mut same = world.clone();
        let glyphs = enlarge(&noise(20, 6, 11), 2);
        let mut fine = world.clone();
        let glyphs_fine = noise(40, 12, 13);
        for (x, y, p) in glyphs.enumerate_pixels() {
            same.put_pixel(20 + x, 10 + y, Rgba([255, p.0[1], 0, 255]));
        }
        for (x, y, p) in glyphs_fine.enumerate_pixels() {
            fine.put_pixel(20 + x, 10 + y, Rgba([255, p.0[1], 0, 255]));
        }
        let rect = Rect {
            x: 0,
            y: 40,
            w: 120,
            h: 40,
        };
        let (m, v) = text_scale_rules(&world, &same, rect).unwrap();
        assert!(v.is_empty(), "{m}");
        let (m, v) = text_scale_rules(&world, &fine, rect).unwrap();
        assert_eq!(v[0]["rule"], "text-scale-mismatch", "{m}");
        assert_eq!(v[0]["text_factor"], 1);
        assert_eq!(v[0]["world_factor"], 2);
        assert_eq!(
            text_scale_rules(&world, &world, rect).unwrap().1[0]["rule"],
            "text-none"
        );
    }
}
