//! Shared generation job of the install-time bake and mod builds. This module
//! has no network path. Cache hits are verified against source, palette, ramps
//! and settings.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use d2_formats::palette::Palette;
use sha2::{Digest, Sha256};

use super::frame_pack::{FrameKey, FramePack};
use super::generator::{
    compose_frame, BlendTables, Generator, Motion, RampLabels, RejectReason, FRACTIONS,
};
use crate::frames::IndexFrame;

#[derive(Debug, Clone)]
pub struct AnimationInput {
    pub key: FrameKey,
    pub directions: BTreeMap<u8, Vec<IndexFrame>>,
    pub looping: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedFrame {
    pub key: FrameKey,
    pub reason: RejectReason,
}

#[derive(Debug)]
pub struct GenerationResult {
    pub pack: FramePack,
    pub rejected: Vec<RejectedFrame>,
    pub elapsed: Duration,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
    pub accepted: usize,
    pub rejected: usize,
}

/// One source frame pair's stored result: the motion vector and which of the
/// three fractions the quality gates accepted (bit 0 = 64, bit 1 = 128,
/// bit 2 = 192).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedPair {
    pub direction: u8,
    pub frame: u16,
    pub dx: i8,
    pub dy: i8,
    pub mask: u8,
}

/// A component file reduced to what the bake stores (pack v2).
#[derive(Debug)]
pub struct PackedGeneration {
    pub input_hash: [u8; 32],
    pub pairs: Vec<PackedPair>,
    pub rejected: Vec<RejectedFrame>,
    pub accepted: u32,
    pub elapsed: Duration,
}

/// The bake's generation step: one motion search per source pair, then the
/// three fractions through the shared pair tables. Pixels are not kept; the
/// pack stores vectors and masks, and the loader rebuilds them from the
/// source cel and `blend.lut`.
pub fn generate_packed(
    input: &AnimationInput,
    generator: &Generator<'_>,
    tables: &BlendTables,
    mut candidate: impl FnMut(bool),
) -> Result<PackedGeneration, String> {
    let start = Instant::now();
    validate_input(input)?;
    let input_hash = hash_inputs(
        std::slice::from_ref(input),
        generator.palette(),
        generator.ramps(),
    );
    let mut pairs = Vec::new();
    let mut rejected = Vec::new();
    let mut accepted = 0u32;
    for (&direction, frames) in &input.directions {
        for (frame, a) in frames.iter().enumerate() {
            let b = if !input.looping && frame + 1 == frames.len() {
                a
            } else {
                &frames[(frame + 1) % frames.len()]
            };
            let frame = u16::try_from(frame).map_err(|_| "too many source frames")?;
            let motion = generator.motion(a, b);
            let motion = match motion {
                Ok(motion) => motion,
                Err(reason) => {
                    for &fraction in &FRACTIONS {
                        let mut key = input.key.clone();
                        key.direction = direction;
                        key.frame = frame;
                        key.sub_frame = fraction;
                        rejected.push(RejectedFrame { key, reason });
                        candidate(false);
                    }
                    pairs.push(PackedPair {
                        direction,
                        frame,
                        dx: 0,
                        dy: 0,
                        mask: 0,
                    });
                    continue;
                }
            };
            let mut mask = 0u8;
            for (slot, &fraction) in FRACTIONS.iter().enumerate() {
                match generator.compose_gates(a, b, motion, fraction, tables) {
                    Ok(_) => {
                        mask |= 1 << slot;
                        accepted += 1;
                        candidate(true);
                    }
                    Err(reason) => {
                        let mut key = input.key.clone();
                        key.direction = direction;
                        key.frame = frame;
                        key.sub_frame = fraction;
                        rejected.push(RejectedFrame { key, reason });
                        candidate(false);
                    }
                }
            }
            pairs.push(PackedPair {
                direction,
                frame,
                dx: motion.dx,
                dy: motion.dy,
                mask,
            });
        }
    }
    Ok(PackedGeneration {
        input_hash,
        pairs,
        rejected,
        accepted,
        elapsed: start.elapsed(),
    })
}

/// This is the same entry point for the install-time bake and a mod's
/// own build pipeline. The bake passes no `cache` (it writes its own packs);
/// callers of the build tool choose their mod output directory outside this repo.
pub fn generate(
    inputs: &[AnimationInput],
    palette: &Palette,
    ramps: &RampLabels,
    cache: Option<&Path>,
    mut progress: impl FnMut(Progress),
) -> Result<GenerationResult, String> {
    let start = Instant::now();
    let total = validate_inputs(inputs)?;
    let input_hash = hash_inputs(inputs, palette, ramps);
    let name: String = input_hash.iter().map(|v| format!("{v:02x}")).collect();
    let file = cache.map(|root| root.join(format!("{name}.d2frames")));
    // Cache and review form one validated result. A missing, stale or corrupt
    // half causes regeneration, so rejected reasons survive a cache hit.
    if let Some(file) = &file {
        let cached = || -> Option<(FramePack, Vec<RejectedFrame>)> {
            let bytes = read_bounded(file, super::frame_pack::MAX_BYTES).ok()?;
            let pack = FramePack::decode(&bytes).ok()?;
            let review_bytes =
                read_bounded(&file.with_extension("review.json"), 8 * 1024 * 1024).ok()?;
            let review: serde_json::Value = serde_json::from_slice(&review_bytes).ok()?;
            let expected = expected_keys(inputs);
            let candidates: Vec<_> = expected.iter().collect();
            let entries = review["rejected"].as_array()?;
            if entries.len() > total {
                return None;
            }
            let mut rejected = Vec::new();
            for entry in entries {
                let index = usize::try_from(entry["candidate"].as_u64()?).ok()?;
                let reason = match entry["reason"].as_str()? {
                    "EmptySource" => RejectReason::EmptySource,
                    "MotionMismatch" => RejectReason::MotionMismatch,
                    "ShapeChange" => RejectReason::ShapeChange,
                    "RampConflict" => RejectReason::RampConflict,
                    "PaletteError" => RejectReason::PaletteError,
                    _ => return None,
                };
                rejected.push(RejectedFrame {
                    key: (*candidates.get(index)?).clone(),
                    reason,
                });
            }
            if review["pack_hash"] != serde_json::json!(format!("{:x}", Sha256::digest(&bytes)))
                || pack.input_hash != input_hash
                || review["input_hash"] != serde_json::json!(input_hash)
                || review["version"].as_u64() != Some(1)
                || review["total"].as_u64() != Some(total as u64)
                || pack.frames.len() + rejected.len() != total
            {
                return None;
            }
            let mut observed = std::collections::BTreeSet::new();
            for key in pack.frames.keys().chain(rejected.iter().map(|r| &r.key)) {
                if !observed.insert(key.clone()) {
                    return None;
                }
            }
            if observed != expected {
                return None;
            }
            Some((pack, rejected))
        };
        if let Some((pack, rejected)) = cached() {
            progress(Progress {
                completed: total,
                total,
                accepted: pack.frames.len(),
                rejected: rejected.len(),
            });
            return Ok(GenerationResult {
                pack,
                rejected,
                elapsed: start.elapsed(),
                cache_hit: true,
            });
        }
    }
    let generator = Generator::new(palette, ramps);
    let tables = generator.tables();
    let mut status = Progress {
        completed: 0,
        total,
        accepted: 0,
        rejected: 0,
    };
    progress(status);
    let mut pack = FramePack {
        input_hash,
        frames: BTreeMap::new(),
    };
    let mut rejected = Vec::new();
    for input in inputs {
        let packed = generate_packed(input, &generator, &tables, |accepted| {
            if accepted {
                status.accepted += 1;
            } else {
                status.rejected += 1;
            }
            status.completed += 1;
            progress(status);
        })?;
        for pair in &packed.pairs {
            let frames = &input.directions[&pair.direction];
            let a = &frames[usize::from(pair.frame)];
            let b = if !input.looping && usize::from(pair.frame) + 1 == frames.len() {
                a
            } else {
                &frames[(usize::from(pair.frame) + 1) % frames.len()]
            };
            for (slot, &fraction) in FRACTIONS.iter().enumerate() {
                if pair.mask & (1 << slot) == 0 {
                    continue;
                }
                let picture = compose_frame(
                    a,
                    b,
                    Motion {
                        dx: pair.dx,
                        dy: pair.dy,
                    },
                    fraction,
                    tables.blend(slot),
                )
                .map_err(|e| format!("{e:?}"))?;
                let mut key = input.key.clone();
                key.direction = pair.direction;
                key.frame = pair.frame;
                key.sub_frame = fraction;
                if pack.frames.insert(key, picture).is_some() {
                    return Err("duplicate generation input".into());
                }
            }
        }
        rejected.extend(packed.rejected);
    }
    if pack.frames.len() + rejected.len() != total {
        return Err("generation count mismatch".into());
    }
    let encoded = pack.encode().map_err(|e| e.to_string())?;
    if let Some(file) = file {
        let parent = file.parent().ok_or("cache directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        publish_file(&file, &encoded)?;
        let review = rejected
            .iter()
            .map(|r| format!("{:?}\t{:?}\n", r.key, r.reason))
            .collect::<String>();
        std::fs::write(file.with_extension("review.txt"), review).map_err(|e| e.to_string())?;
        let candidates: BTreeMap<_, _> = expected_keys(inputs)
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index))
            .collect();
        let review = serde_json::json!({"version":1,"input_hash":input_hash,"total":total,
            "pack_hash":format!("{:x}", Sha256::digest(&encoded)),
            "rejected":rejected.iter().map(|r|serde_json::json!({
                "candidate":candidates[&r.key],"reason":format!("{:?}",r.reason)
            })).collect::<Vec<_>>()});
        let bytes = serde_json::to_vec(&review).map_err(|e| e.to_string())?;
        publish_file(&file.with_extension("review.json"), &bytes)?;
    }
    Ok(GenerationResult {
        pack,
        rejected,
        elapsed: start.elapsed(),
        cache_hit: false,
    })
}

/// The generator and its settings; part of every pack identity, so a changed
/// generator makes every baked pack stale.
pub const GENERATOR_SETTINGS: &str = "D2RUST-animation-generator-v2:packv2:linear-srgb:motion8-coarse4+2-sample1024-mse.35-distance.00001:shape.45:ramp.02:palette.04:ramps-signatures:64,128,192";

/// The pack identity of `inputs`: source pictures, palette, ramps and
/// [`GENERATOR_SETTINGS`]. A pack file is named by it.
///
/// The component key (archive file name) is deliberately not hashed: the pack
/// stores no keys, so byte-identical component files under different names
/// share one content-addressed pack file.
pub fn hash_inputs(inputs: &[AnimationInput], palette: &Palette, ramps: &RampLabels) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(GENERATOR_SETTINGS.as_bytes());
    for c in palette.colors {
        h.update([c.r, c.g, c.b]);
    }
    h.update(ramps);
    for input in inputs {
        h.update([u8::from(input.looping)]);
        for (&direction, frames) in &input.directions {
            h.update([direction]);
            h.update((frames.len() as u64).to_le_bytes());
            for f in frames {
                h.update(f.width.to_le_bytes());
                h.update(f.height.to_le_bytes());
                h.update(f.x_off.to_le_bytes());
                h.update(f.y_off.to_le_bytes());
                h.update(format!("{:?}", f.anchor).as_bytes());
                h.update(&f.pixels);
            }
        }
    }
    h.finalize().into()
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("file exceeds byte limit".into());
    }
    Ok(bytes)
}

fn publish_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}.{nonce}.partial", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    // Concurrent jobs for this content identity produce identical bytes. Keep
    // a complete existing file; replace corrupt data through a temporary file.
    if read_bounded(path, super::frame_pack::MAX_BYTES).is_ok_and(|b| b == bytes) {
        let _ = std::fs::remove_file(&temporary);
        return Ok(());
    }
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let same = read_bounded(path, super::frame_pack::MAX_BYTES).is_ok_and(|b| b == bytes);
        let _ = std::fs::remove_file(&temporary);
        if !same {
            return Err(error.to_string());
        }
    }
    Ok(())
}

fn expected_keys(inputs: &[AnimationInput]) -> std::collections::BTreeSet<FrameKey> {
    let mut keys = std::collections::BTreeSet::new();
    for input in inputs {
        for (&direction, frames) in &input.directions {
            for frame in 0..frames.len() {
                for sub_frame in [64, 128, 192] {
                    let mut key = input.key.clone();
                    key.direction = direction;
                    key.frame = frame as u16;
                    key.sub_frame = sub_frame;
                    keys.insert(key);
                }
            }
        }
    }
    keys
}

fn validate_inputs(inputs: &[AnimationInput]) -> Result<usize, String> {
    let mut total = 0usize;
    for input in inputs {
        total = total
            .checked_add(validate_input(input)?)
            .ok_or("source frame limit")?;
        if total > super::frame_pack::MAX_FRAMES {
            return Err("source frame limit".into());
        }
    }
    if expected_keys(inputs).len() != total {
        return Err("duplicate generation input".into());
    }
    Ok(total)
}

fn validate_input(input: &AnimationInput) -> Result<usize, String> {
    input.key.validate().map_err(|e| e.to_string())?;
    let mut total = 0usize;
    let mut pixels = 0usize;
    for (&direction, frames) in &input.directions {
        if direction >= 64 || frames.is_empty() || frames.len() > u16::MAX as usize {
            return Err("invalid source direction or frame count".into());
        }
        total = total
            .checked_add(frames.len() * 3)
            .ok_or("source frame limit")?;
        if total > super::frame_pack::MAX_FRAMES {
            return Err("source frame limit".into());
        }
        for frame in frames {
            let count = u64::from(frame.width) * u64::from(frame.height);
            if frame.width == 0
                || frame.height == 0
                || count > super::frame_pack::MAX_PIXELS
                || count != frame.pixels.len() as u64
                || i64::from(frame.x_off).abs() > super::frame_pack::MAX_OFFSET
                || i64::from(frame.y_off).abs() > super::frame_pack::MAX_OFFSET
            {
                return Err("invalid source dimensions".into());
            }
            pixels = pixels
                .checked_add(frame.pixels.len())
                .ok_or("source byte limit")?;
            if pixels > super::frame_pack::MAX_BYTES {
                return Err("source byte limit".into());
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
#[path = "generation_job_tests.rs"]
mod tests;
