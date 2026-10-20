//! Component-local motion interpolation in linear colour, followed by palette
//! quantization. Rejected pictures keep the starting source picture.
//!
//! The install-time bake searches a frame pair's motion once and stores only
//! the vector and the accepted fractions; the pair tables below also produce
//! the shared `blend.lut` the loader remaps source pixels through (see
//! `publish/FORMAT.md`).

use d2_formats::palette::Palette;

use crate::frames::{FrameAnchor, IndexFrame};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RejectReason {
    EmptySource,
    MotionMismatch,
    ShapeChange,
    RampConflict,
    PaletteError,
}

#[derive(Debug, Clone)]
pub struct GeneratedFrame {
    pub picture: IndexFrame,
    pub rejected: Option<RejectReason>,
}

/// The three in-between fractions of a source interval, in pack order (bit
/// 0, 1, 2 of a pack frame's accepted-fraction mask).
pub const FRACTIONS: [u8; 3] = [64, 128, 192];

/// Pair-table slot of `fraction`, if it is one of [`FRACTIONS`].
pub fn fraction_slot(fraction: u8) -> Option<usize> {
    FRACTIONS.iter().position(|&f| f == fraction)
}

const PAIRS: usize = 256 * 256;

/// Equal nonzero labels identify indices belonging to the same colour-map
/// ramp. Label zero is ordinary, unmapped colour. Never merge separate ramps.
pub type RampLabels = [u8; 256];

/// Two colours this close in linear RGB are the same shade for ramp purposes.
/// Different item-colour families overlap at their dark and grey ends, so a
/// label difference alone does not make a blend unsafe.
const SHADE_OVERLAP: f64 = 0.02;

/// A source frame pair's displacement: one search shared by every fraction,
/// layer and equipment variant of the same pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Motion {
    pub dx: i8,
    pub dy: i8,
}

/// One palette-index pair's rule at one fraction.
#[derive(Debug, Clone, Copy)]
struct PairValue {
    index: u8,
    error: f64,
    conflict: bool,
}

/// The colour decisions of every palette-index pair, computed once per bake.
/// `blend` is exactly the bytes of `blend.lut`.
#[derive(Debug, Clone)]
pub struct BlendTables {
    blend: Vec<u8>,
    error: Vec<f64>,
    conflict: Vec<bool>,
}

impl BlendTables {
    pub fn new(generator: &Generator<'_>) -> Self {
        let mut blend = vec![0u8; FRACTIONS.len() * PAIRS];
        let mut error = vec![0.0f64; FRACTIONS.len() * PAIRS];
        let mut conflict = vec![false; PAIRS];
        for pa in 0..256usize {
            for pb in 0..256usize {
                let pair = pa * 256 + pb;
                for (slot, &fraction) in FRACTIONS.iter().enumerate() {
                    let value = generator.pair_value(pa as u8, pb as u8, fraction);
                    blend[slot * PAIRS + pair] = value.index;
                    error[slot * PAIRS + pair] = value.error;
                    conflict[pair] = value.conflict;
                }
            }
        }
        Self {
            blend,
            error,
            conflict,
        }
    }

    /// The bytes of the bake folder's `blend.lut`.
    pub fn bytes(&self) -> &[u8] {
        &self.blend
    }

    /// The table of one [`FRACTIONS`] slot (a `blend.lut` third).
    pub fn blend(&self, slot: usize) -> &[u8] {
        &self.blend[slot * PAIRS..(slot + 1) * PAIRS]
    }
}

pub struct Generator<'a> {
    palette: &'a Palette,
    ramps: &'a RampLabels,
    linear: [[f64; 3]; 256],
    /// Ascending palette indices of each ramp label.
    ramp_indices: Vec<Vec<u8>>,
    /// Squared linear-RGB distance by palette index pair.
    distances: Vec<f64>,
}

impl<'a> Generator<'a> {
    pub fn new(palette: &'a Palette, ramps: &'a RampLabels) -> Self {
        let convert = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        let linear = palette
            .colors
            .map(|c| [convert(c.r), convert(c.g), convert(c.b)]);
        let mut ramp_indices = vec![Vec::new(); 256];
        for index in 1..palette.colors.len() {
            ramp_indices[usize::from(ramps[index])].push(index as u8);
        }
        let mut distances = vec![0.0; PAIRS];
        for (pa, ca) in linear.iter().enumerate() {
            for (pb, cb) in linear.iter().enumerate() {
                distances[pa * 256 + pb] = distance(*ca, *cb);
            }
        }
        Self {
            palette,
            ramps,
            linear,
            ramp_indices,
            distances,
        }
    }

    pub fn palette(&self) -> &'a Palette {
        self.palette
    }

    pub fn ramps(&self) -> &'a RampLabels {
        self.ramps
    }

    /// Builds the pair tables once per bake; every job shares them.
    pub fn tables(&self) -> BlendTables {
        BlendTables::new(self)
    }

    /// Rejects a source pair, or finds its displacement once for all three
    /// fractions. The 17x17 search samples a bounded regular set of the
    /// earlier cel; shape, ramp and palette gates still inspect every output
    /// pixel in [`Generator::compose`].
    pub fn motion(&self, a: &IndexFrame, b: &IndexFrame) -> Result<Motion, RejectReason> {
        if [a, b].iter().any(|f| {
            let pixels = u64::from(f.width) * u64::from(f.height);
            f.width == 0
                || f.height == 0
                || pixels > 4 * 1024 * 1024
                || pixels != f.pixels.len() as u64
        }) {
            return Err(RejectReason::ShapeChange);
        }
        let occupied = |f: &IndexFrame| f.pixels.iter().filter(|&&p| p != 0).count();
        if occupied(a) == 0 || occupied(b) == 0 {
            return Err(RejectReason::EmptySource);
        }
        // Identical cels: the zero displacement is the unique minimum (every
        // other offset pays the positive distance penalty on top of zero
        // errors), so the search can be skipped exactly.
        if a == b {
            return Ok(Motion { dx: 0, dy: 0 });
        }
        let samples = motion_samples(a);
        if samples.is_empty() {
            // The sampled subset saw no lit pixel: no displacement fits.
            return Err(RejectReason::MotionMismatch);
        }
        let count = samples.len() as f64;
        let (ax, ay) = (i64::from(a.x_off), i64::from(top_offset(a)));
        // Two-stage search: a 5x5 grid with step 4, then a 5x5 refinement
        // around its winner. Every true offset is within 2 of a grid point, so
        // the refinement contains the true best whenever the coarse winner is
        // its nearest grid point; the result can still differ from the
        // exhaustive 17x17 search (measured diffs are in the PR). About 50
        // offsets are evaluated instead of 289.
        let grid = [-8i32, -4, 0, 4, 8];
        let mut best = (f64::INFINITY, 0i32, 0i32);
        for &dy in &grid {
            for &dx in &grid {
                if let Some(error) = self.motion_error(b, &samples, count, ax, ay, dx, dy, best.0) {
                    if error < best.0 {
                        best = (error, dx, dy);
                    }
                }
            }
        }
        let (cx, cy) = (best.1, best.2);
        for dy in (cy - 2).max(-8)..=(cy + 2).min(8) {
            for dx in (cx - 2).max(-8)..=(cx + 2).min(8) {
                if (dx, dy) == (cx, cy) {
                    continue;
                }
                if let Some(error) = self.motion_error(b, &samples, count, ax, ay, dx, dy, best.0) {
                    if error < best.0 {
                        best = (error, dx, dy);
                    }
                }
            }
        }
        if best.0 > 0.35 {
            return Err(RejectReason::MotionMismatch);
        }
        Ok(Motion {
            dx: best.1 as i8,
            dy: best.2 as i8,
        })
    }

    /// The mean sample error of one offset, or `None` once a prefix makes any
    /// further work pointless (all remaining terms are non-negative).
    #[allow(clippy::too_many_arguments)]
    fn motion_error(
        &self,
        b: &IndexFrame,
        samples: &[MotionSample],
        count: f64,
        ax: i64,
        ay: i64,
        dx: i32,
        dy: i32,
        threshold: f64,
    ) -> Option<f64> {
        let penalty = f64::from(dx * dx + dy * dy) * 0.00001;
        let mut sum = 0.0;
        for (i, s) in samples.iter().enumerate() {
            let pb = sample(b, ax + s.x + i64::from(dx), ay + s.y + i64::from(dy));
            sum += if pb == 0 {
                1.0
            } else {
                self.distances[usize::from(s.pa) * 256 + usize::from(pb)]
            };
            if i % 8 == 7 && sum / count + penalty > threshold {
                return None;
            }
        }
        Some(sum / count + penalty)
    }

    /// One accepted in-between, or the gate that rejected it. `motion` comes
    /// from [`Generator::motion`]; `tables` holds this generator's palette.
    pub fn compose(
        &self,
        a: &IndexFrame,
        b: &IndexFrame,
        motion: Motion,
        fraction: u8,
        tables: &BlendTables,
    ) -> Result<IndexFrame, RejectReason> {
        let Some(slot) = fraction_slot(fraction) else {
            return Err(RejectReason::ShapeChange);
        };
        let bounds = compose_bounds(a, b, motion, fraction)?;
        let base = slot * PAIRS;
        let mut pixels = Vec::with_capacity(bounds.width as usize * bounds.height as usize);
        self.compose_with(
            a,
            b,
            &bounds,
            |pa, pb| {
                let pair = usize::from(pa) * 256 + usize::from(pb);
                PairValue {
                    index: tables.blend[base + pair],
                    error: tables.error[base + pair],
                    conflict: tables.conflict[pair],
                }
            },
            |index| pixels.push(index),
        )?;
        Ok(IndexFrame {
            width: bounds.width,
            height: bounds.height,
            x_off: bounds.left,
            y_off: bounds.top,
            anchor: FrameAnchor::Top,
            pixels,
        })
    }

    /// The same gates as [`Generator::compose`] without building the picture:
    /// the bake only stores vectors and masks.
    pub fn compose_gates(
        &self,
        a: &IndexFrame,
        b: &IndexFrame,
        motion: Motion,
        fraction: u8,
        tables: &BlendTables,
    ) -> Result<(), RejectReason> {
        let Some(slot) = fraction_slot(fraction) else {
            return Err(RejectReason::ShapeChange);
        };
        let bounds = compose_bounds(a, b, motion, fraction)?;
        let base = slot * PAIRS;
        self.compose_with(
            a,
            b,
            &bounds,
            |pa, pb| {
                let pair = usize::from(pa) * 256 + usize::from(pb);
                PairValue {
                    index: tables.blend[base + pair],
                    error: tables.error[base + pair],
                    conflict: tables.conflict[pair],
                }
            },
            |_| {},
        )
    }

    fn compose_with(
        &self,
        a: &IndexFrame,
        b: &IndexFrame,
        bounds: &Bounds,
        mut pair_value: impl FnMut(u8, u8) -> PairValue,
        mut emit: impl FnMut(u8),
    ) -> Result<(), RejectReason> {
        let mut union = 0usize;
        let mut changed = 0usize;
        let mut conflicts = 0usize;
        let mut colour_error = 0.0;
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let px = i64::from(bounds.left) + i64::from(x);
                let py = i64::from(bounds.top) + i64::from(y);
                let pa = sample(a, px - i64::from(bounds.sx), py - i64::from(bounds.sy));
                let pb = sample(
                    b,
                    px + i64::from(bounds.dx - bounds.sx),
                    py + i64::from(bounds.dy - bounds.sy),
                );
                let value = pair_value(pa, pb);
                emit(value.index);
                if pa == 0 && pb == 0 {
                    continue;
                }
                union += 1;
                if pa == 0 || pb == 0 {
                    changed += 1;
                } else if value.conflict {
                    conflicts += 1;
                } else {
                    colour_error += value.error;
                }
            }
        }
        let union = union.max(1) as f64;
        if changed as f64 / union > 0.45 {
            return Err(RejectReason::ShapeChange);
        }
        if conflicts as f64 / union > 0.02 {
            return Err(RejectReason::RampConflict);
        }
        if colour_error / union > 0.04 {
            return Err(RejectReason::PaletteError);
        }
        Ok(())
    }

    /// The single-call form of the old generator API: searches motion and
    /// composes one fraction, quantizing each distinct index pair once (the
    /// bake precomputes every pair instead).
    pub fn interpolate(&self, a: &IndexFrame, b: &IndexFrame, fraction: u8) -> GeneratedFrame {
        let keep = |reason| GeneratedFrame {
            picture: a.clone(),
            rejected: Some(reason),
        };
        if [a, b].iter().any(|f| {
            let pixels = u64::from(f.width) * u64::from(f.height);
            f.width == 0
                || f.height == 0
                || pixels > 4 * 1024 * 1024
                || pixels != f.pixels.len() as u64
        }) {
            return keep(RejectReason::ShapeChange);
        }
        if fraction == 0 {
            return GeneratedFrame {
                picture: a.clone(),
                rejected: None,
            };
        }
        let motion = match self.motion(a, b) {
            Ok(motion) => motion,
            Err(reason) => return keep(reason),
        };
        let Ok(bounds) = compose_bounds(a, b, motion, fraction) else {
            return keep(RejectReason::ShapeChange);
        };
        let mut cache = std::collections::BTreeMap::new();
        let mut pixels = Vec::with_capacity(bounds.width as usize * bounds.height as usize);
        match self.compose_with(
            a,
            b,
            &bounds,
            |pa, pb| {
                *cache
                    .entry((pa, pb))
                    .or_insert_with(|| self.pair_value(pa, pb, fraction))
            },
            |index| pixels.push(index),
        ) {
            Ok(()) => GeneratedFrame {
                picture: IndexFrame {
                    width: bounds.width,
                    height: bounds.height,
                    x_off: bounds.left,
                    y_off: bounds.top,
                    anchor: FrameAnchor::Top,
                    pixels,
                },
                rejected: None,
            },
            Err(reason) => keep(reason),
        }
    }

    /// The generator's rule for one palette-index pair at one fraction.
    fn pair_value(&self, pa: u8, pb: u8, fraction: u8) -> PairValue {
        let pick = PairValue {
            index: if fraction < 128 { pa } else { pb },
            error: 0.0,
            conflict: false,
        };
        if pa == 0 || pb == 0 {
            return pick;
        }
        let (ramp_a, ramp_b) = (self.ramps[usize::from(pa)], self.ramps[usize::from(pb)]);
        let ca = self.linear[usize::from(pa)];
        let cb = self.linear[usize::from(pb)];
        if ramp_a != ramp_b && distance(ca, cb) > SHADE_OVERLAP {
            return PairValue {
                conflict: true,
                ..pick
            };
        }
        // Stay on the starting ramp when it is mapped; a compatible pair from
        // two families may fall back to the other one.
        let ramp = if ramp_a != 0 { ramp_a } else { ramp_b };
        let t = f64::from(fraction) / 256.0;
        let mixed = std::array::from_fn(|i| ca[i] * (1.0 - t) + cb[i] * t);
        let (index, error) = self.nearest(mixed, ramp);
        PairValue {
            index,
            error,
            conflict: false,
        }
    }

    fn nearest(&self, colour: [f64; 3], ramp: u8) -> (u8, f64) {
        let mut best = (1u8, f64::INFINITY);
        for &index in &self.ramp_indices[usize::from(ramp)] {
            let error = distance(colour, self.linear[usize::from(index)]);
            if error < best.1 {
                best = (index, error);
            }
        }
        best
    }
}

/// One sampled pixel of the earlier cel's motion-estimation subset.
struct MotionSample {
    x: i64,
    y: i64,
    pa: u8,
}

fn motion_samples(a: &IndexFrame) -> Vec<MotionSample> {
    let stride = a.pixels.len().div_ceil(1024).max(1);
    let width = a.width as usize;
    let mut samples = Vec::new();
    for index in (0..a.pixels.len()).step_by(stride) {
        let pa = a.pixels[index];
        if pa == 0 {
            continue;
        }
        samples.push(MotionSample {
            x: (index % width) as i64,
            y: (index / width) as i64,
            pa,
        });
    }
    samples
}

/// The output box of one in-between, shared by the bake and the loader.
struct Bounds {
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    dx: i32,
    dy: i32,
    sx: i32,
    sy: i32,
}

fn compose_bounds(
    a: &IndexFrame,
    b: &IndexFrame,
    motion: Motion,
    fraction: u8,
) -> Result<Bounds, RejectReason> {
    let (dx, dy) = (i32::from(motion.dx), i32::from(motion.dy));
    let shift = |d: i32| (d * i32::from(fraction) + 128).div_euclid(256);
    let (sx, sy) = (shift(dx), shift(dy));
    let left = a
        .x_off
        .saturating_add(sx)
        .min(b.x_off.saturating_sub(dx - sx));
    let top = top_offset(a)
        .saturating_add(sy)
        .min(top_offset(b).saturating_sub(dy - sy));
    let right = (i64::from(a.x_off) + i64::from(a.width) + i64::from(sx))
        .max(i64::from(b.x_off) + i64::from(b.width) - i64::from(dx - sx));
    let bottom = (i64::from(top_offset(a)) + i64::from(a.height) + i64::from(sy))
        .max(i64::from(top_offset(b)) + i64::from(b.height) - i64::from(dy - sy));
    let Ok(width) = u32::try_from(right - i64::from(left)) else {
        return Err(RejectReason::ShapeChange);
    };
    let Ok(height) = u32::try_from(bottom - i64::from(top)) else {
        return Err(RejectReason::ShapeChange);
    };
    if u64::from(width) * u64::from(height) > super::frame_pack::MAX_PIXELS
        || i64::from(left).abs() > super::frame_pack::MAX_OFFSET
        || i64::from(top).abs() > super::frame_pack::MAX_OFFSET
    {
        return Err(RejectReason::ShapeChange);
    }
    Ok(Bounds {
        width,
        height,
        left,
        top,
        dx,
        dy,
        sx,
        sy,
    })
}

/// Rebuilds one accepted in-between from its stored vector and one
/// `blend.lut` table (`blend` is 65,536 bytes, this fraction's third).
pub fn compose_frame(
    a: &IndexFrame,
    b: &IndexFrame,
    motion: Motion,
    fraction: u8,
    blend: &[u8],
) -> Result<IndexFrame, RejectReason> {
    let bounds = compose_bounds(a, b, motion, fraction)?;
    let mut pixels = Vec::with_capacity(bounds.width as usize * bounds.height as usize);
    for y in 0..bounds.height {
        for x in 0..bounds.width {
            let px = i64::from(bounds.left) + i64::from(x);
            let py = i64::from(bounds.top) + i64::from(y);
            let pa = sample(a, px - i64::from(bounds.sx), py - i64::from(bounds.sy));
            let pb = sample(
                b,
                px + i64::from(bounds.dx - bounds.sx),
                py + i64::from(bounds.dy - bounds.sy),
            );
            pixels.push(blend[usize::from(pa) * 256 + usize::from(pb)]);
        }
    }
    Ok(IndexFrame {
        width: bounds.width,
        height: bounds.height,
        x_off: bounds.left,
        y_off: bounds.top,
        anchor: FrameAnchor::Top,
        pixels,
    })
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        / 3.0
}

fn top_offset(f: &IndexFrame) -> i32 {
    match f.anchor {
        FrameAnchor::Top | FrameAnchor::TopDown => f.y_off,
        FrameAnchor::Bottom => (i64::from(f.y_off) + 1 - i64::from(f.height))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    }
}

fn sample(f: &IndexFrame, x: i64, y: i64) -> u8 {
    let x = x - i64::from(f.x_off);
    let y = y - i64::from(top_offset(f));
    if x < 0 || y < 0 || x >= i64::from(f.width) || y >= i64::from(f.height) {
        return 0;
    }
    f.pixels[y as usize * f.width as usize + x as usize]
}

/// Partition mapped indices by which item-palette file recolours them. A file
/// is one colour family; every index a file can transform is on that family's
/// shades, so map-level signatures must not split it into single indices.
/// Unmapped indices share label zero. Mod pipelines supply their own labels.
pub fn ramps_from_files(files: &[&[[u8; 256]]]) -> RampLabels {
    let signatures: Vec<Vec<bool>> = (0..256)
        .map(|index| {
            files
                .iter()
                .map(|maps| maps.iter().any(|map| usize::from(map[index]) != index))
                .collect()
        })
        .collect();
    let mut labels = [0; 256];
    let mut assigned: std::collections::BTreeMap<Vec<bool>, u8> = Default::default();
    let mut label = 0u8;
    for index in 1..256 {
        if signatures[index].iter().any(|&changed| changed) {
            let next = *assigned
                .entry(signatures[index].clone())
                .or_insert_with(|| {
                    label = label.saturating_add(1);
                    label
                });
            labels[index] = next;
        }
    }
    labels
}

pub fn ramps_from_maps(maps: &[[u8; 256]]) -> RampLabels {
    ramps_from_files(&[maps])
}

pub fn ramps_from_source(
    source: &dyn crate::assets::path::FileSource,
) -> Result<RampLabels, String> {
    use crate::rules::shading::{ITEM_PALETTE_FILES, ITEM_PALETTE_MAPS};
    let mut files = Vec::new();
    for name in ITEM_PALETTE_FILES {
        let path = crate::world_view::panel_art::item_palette_name(name);
        let bytes = source
            .read_file(&path)
            .ok_or_else(|| format!("missing ramp maps: {path}"))??;
        let need = usize::from(ITEM_PALETTE_MAPS) * 256;
        if bytes.len() < need {
            return Err(format!("short ramp maps: {path}"));
        }
        files.push(bytes[..need].to_vec());
    }
    let groups: Vec<&[[u8; 256]]> = files
        .iter()
        .map(|bytes| bytes.as_chunks::<256>().0)
        .collect();
    Ok(ramps_from_files(&groups))
}

#[cfg(test)]
#[path = "generator_tests.rs"]
mod tests;
