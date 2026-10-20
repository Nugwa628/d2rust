//! Baked animation pack v2: one motion vector and one accepted-fraction mask
//! per source frame pair. Pixels are not stored; the loader rebuilds each
//! accepted in-between from the source cel and the bake folder's `blend.lut`.
//! See `publish/FORMAT.md`.
//!
//! The file on disk is a raw-deflate stream of [`BakedPack::encode`]'s bytes;
//! [`super::animation_baked::read_baked_pack`] owns the inflate.
//!
//! All integers are little endian. Decoding validates before allocating.

use std::collections::BTreeMap;

use super::frame_pack::{FrameKey, FramePack, PackError, MAX_BYTES, MAX_FRAMES};
use super::generation_job::{AnimationInput, PackedPair};
use super::generator::{compose_frame, Motion, FRACTIONS};

pub const VERSION: u16 = 2;
pub const MAGIC: &[u8; 8] = b"D2FRAME\0";
/// The bake folder's shared blend table: three fractions x 256 x 256.
pub const LUT_FILE: &str = "blend.lut";
pub const LUT_BYTES: usize = FRACTIONS.len() * 256 * 256;
const PAIRS: usize = 256 * 256;
const MAX_PAIRS: usize = MAX_FRAMES;

/// One source frame pair of a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub dx: i8,
    pub dy: i8,
    /// Bit 0 fraction 64, bit 1 128, bit 2 192; set means accepted.
    pub mask: u8,
}

/// A component file's baked animation data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakedPack {
    pub input_hash: [u8; 32],
    pub looping: bool,
    /// Direction -> one pair per source frame, frames contiguous from zero.
    pub directions: BTreeMap<u8, Vec<Pair>>,
}

impl BakedPack {
    /// Groups a bake's pairs (ascending direction and frame) into pack order.
    pub fn from_pairs(
        input_hash: [u8; 32],
        looping: bool,
        pairs: &[PackedPair],
    ) -> Result<Self, PackError> {
        let mut directions: BTreeMap<u8, Vec<Pair>> = BTreeMap::new();
        for pair in pairs {
            let frames = directions.entry(pair.direction).or_default();
            if frames.len() != usize::from(pair.frame) {
                return Err(PackError::Invalid("frame order"));
            }
            frames.push(Pair {
                dx: pair.dx,
                dy: pair.dy,
                mask: pair.mask,
            });
        }
        let pack = Self {
            input_hash,
            looping,
            directions,
        };
        pack.validate()?;
        Ok(pack)
    }

    pub fn accepted(&self) -> u32 {
        self.directions
            .values()
            .flatten()
            .map(|p| p.mask.count_ones())
            .sum()
    }

    fn validate(&self) -> Result<(), PackError> {
        if self.directions.len() > 64 {
            return Err(PackError::Invalid("direction limit"));
        }
        let mut total = 0usize;
        for (&direction, pairs) in &self.directions {
            if direction >= 64 || pairs.is_empty() {
                return Err(PackError::Invalid("direction"));
            }
            total = total
                .checked_add(pairs.len())
                .ok_or(PackError::Invalid("pair limit"))?;
            if total > MAX_PAIRS {
                return Err(PackError::Invalid("pair limit"));
            }
            for pair in pairs {
                if pair.mask & !0x07 != 0 {
                    return Err(PackError::Invalid("fraction mask"));
                }
            }
        }
        if self.accepted() > MAX_FRAMES as u32 {
            return Err(PackError::Invalid("frame limit"));
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, PackError> {
        self.validate()?;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.input_hash);
        out.push(u8::from(self.looping));
        out.push(self.directions.len() as u8);
        for (&direction, pairs) in &self.directions {
            out.push(direction);
            out.extend_from_slice(&(pairs.len() as u16).to_le_bytes());
            for pair in pairs {
                out.push(pair.dx as u8);
                out.push(pair.dy as u8);
                out.push(pair.mask);
            }
        }
        out.extend_from_slice(&self.accepted().to_le_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PackError> {
        if bytes.len() > MAX_BYTES {
            return Err(PackError::Invalid("byte limit"));
        }
        let mut r = Reader(bytes);
        if &r.take::<8>()? != MAGIC {
            return Err(PackError::Invalid("magic"));
        }
        let version = u16::from_le_bytes(r.take()?);
        if version != VERSION {
            return Err(PackError::Version(version));
        }
        let input_hash = r.take()?;
        let looping = match r.byte()? {
            0 => false,
            1 => true,
            _ => return Err(PackError::Invalid("looping")),
        };
        let count = usize::from(r.byte()?);
        if count > 64 {
            return Err(PackError::Invalid("direction limit"));
        }
        let mut directions: BTreeMap<u8, Vec<Pair>> = BTreeMap::new();
        let mut total = 0usize;
        for _ in 0..count {
            let direction = r.byte()?;
            if direction >= 64 || directions.contains_key(&direction) {
                return Err(PackError::Invalid("direction"));
            }
            let frames = usize::from(u16::from_le_bytes(r.take()?));
            if frames == 0 {
                return Err(PackError::Invalid("frame count"));
            }
            total = total
                .checked_add(frames)
                .ok_or(PackError::Invalid("pair limit"))?;
            if total > MAX_PAIRS {
                return Err(PackError::Invalid("pair limit"));
            }
            let mut pairs = Vec::with_capacity(frames);
            for _ in 0..frames {
                let pair = Pair {
                    dx: r.byte()? as i8,
                    dy: r.byte()? as i8,
                    mask: r.byte()?,
                };
                if pair.mask & !0x07 != 0 {
                    return Err(PackError::Invalid("fraction mask"));
                }
                pairs.push(pair);
            }
            directions.insert(direction, pairs);
        }
        let accepted = u32::from_le_bytes(r.take()?);
        if !r.0.is_empty() {
            return Err(PackError::Invalid("trailing bytes"));
        }
        let pack = Self {
            input_hash,
            looping,
            directions,
        };
        pack.validate()?;
        if pack.accepted() != accepted {
            return Err(PackError::Invalid("accepted count"));
        }
        Ok(pack)
    }

    /// Whether this pack was made from `input`'s source frames.
    pub fn matches(&self, input: &AnimationInput) -> Result<(), String> {
        if self.looping != input.looping {
            return Err("looping differs".into());
        }
        if self.directions.len() != input.directions.len() {
            return Err("direction count differs".into());
        }
        for (&direction, pairs) in &self.directions {
            let Some(frames) = input.directions.get(&direction) else {
                return Err("direction differs".into());
            };
            if pairs.len() != frames.len() {
                return Err("frame count differs".into());
            }
        }
        Ok(())
    }

    /// Rebuilds every accepted in-between from the source frames and the
    /// bake folder's `blend.lut` bytes. This is the loader's decode path.
    pub fn decode_to_frames(
        &self,
        input: &AnimationInput,
        lut: &[u8],
    ) -> Result<FramePack, String> {
        if lut.len() != LUT_BYTES {
            return Err(format!("blend.lut must be {LUT_BYTES} bytes"));
        }
        self.matches(input)?;
        let mut frames = BTreeMap::new();
        for (&direction, pairs) in &self.directions {
            let sources = &input.directions[&direction];
            for (frame, pair) in pairs.iter().enumerate() {
                if pair.mask == 0 {
                    continue;
                }
                let a = &sources[frame];
                let b = if !self.looping && frame + 1 == sources.len() {
                    a
                } else {
                    &sources[(frame + 1) % sources.len()]
                };
                let motion = Motion {
                    dx: pair.dx,
                    dy: pair.dy,
                };
                for (slot, &fraction) in FRACTIONS.iter().enumerate() {
                    if pair.mask & (1 << slot) == 0 {
                        continue;
                    }
                    let blend = &lut[slot * PAIRS..(slot + 1) * PAIRS];
                    let picture = compose_frame(a, b, motion, fraction, blend)
                        .map_err(|e| format!("{e:?}"))?;
                    let key = frame_key(
                        input,
                        direction,
                        u16::try_from(frame).map_err(|_| "too many source frames")?,
                        fraction,
                    );
                    if frames.insert(key, picture).is_some() {
                        return Err("duplicate pack frame".into());
                    }
                }
            }
        }
        if frames.len() != self.accepted() as usize {
            return Err("accepted count".into());
        }
        Ok(FramePack {
            input_hash: self.input_hash,
            frames,
        })
    }
}

/// The key a decoded frame gets; exposed for tests and callers that build
/// packs themselves.
pub fn frame_key(input: &AnimationInput, direction: u8, frame: u16, fraction: u8) -> FrameKey {
    let mut key = input.key.clone();
    key.direction = direction;
    key.frame = frame;
    key.sub_frame = fraction;
    key
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], PackError> {
        let bytes = self
            .0
            .get(..N)
            .ok_or(PackError::Invalid("truncated header"))?;
        let mut out = [0; N];
        out.copy_from_slice(bytes);
        self.0 = &self.0[N..];
        Ok(out)
    }
    fn byte(&mut self) -> Result<u8, PackError> {
        Ok(self.take::<1>()?[0])
    }
}

#[cfg(test)]
#[path = "animation_pack_tests.rs"]
mod tests;
