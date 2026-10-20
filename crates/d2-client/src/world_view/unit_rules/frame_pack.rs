//! Frame pack v1, shared by the user cache and mod build outputs.
//! All integers are little endian. Bounded decoding validates before allocation.

use std::collections::BTreeMap;

use crate::frames::{FrameAnchor, IndexFrame};

pub const VERSION: u16 = 1;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FRAMES: usize = 16_384;
pub const MAX_PIXELS: u64 = 4 * 1024 * 1024;
/// Bound placement arithmetic for packs supplied by external pipelines.
pub const MAX_OFFSET: i64 = 1024 * 1024;
const MAGIC: &[u8; 8] = b"D2FRAME\0";

/// Complete component identity, including equipment variant and source frame.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameKey {
    /// 0 player, 1 monster, 2 object, 3 missile, 4 overlay.
    pub kind: u8,
    pub token: [u8; 4],
    pub mode: [u8; 4],
    pub component: u8,
    pub component_token: [u8; 4],
    pub armor_class: [u8; 4],
    pub weapon_class: [u8; 4],
    /// Facing in the client's 64-direction space.
    pub direction: u8,
    pub frame: u16,
    /// Fraction of the source interval in 1/256 units (zero is source art).
    pub sub_frame: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePack {
    /// SHA-256 of source art, palette, ramps and generator settings. Used to
    /// invalidate user caches; mod pipelines compute it from their own inputs.
    pub input_hash: [u8; 32],
    pub frames: BTreeMap<FrameKey, IndexFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackError {
    #[error("invalid frame pack: {0}")]
    Invalid(&'static str),
    #[error("unsupported frame pack version {0}")]
    Version(u16),
}

/// A code's name part in lowercase, space padded: table codes, file names and
/// mod packs spell one token in different case and padding.
pub fn fold_code(code: &[u8; 4]) -> [u8; 4] {
    let mut out = *b"    ";
    for (o, &b) in out.iter_mut().zip(code.iter().take(3)) {
        if b == b' ' || b == 0 {
            break;
        }
        *o = b.to_ascii_lowercase();
    }
    out
}

impl FrameKey {
    /// The key with every code folded ([`fold_code`]); resident packs and
    /// playback lookups both use folded keys.
    pub fn folded(&self) -> FrameKey {
        FrameKey {
            token: fold_code(&self.token),
            mode: fold_code(&self.mode),
            component_token: fold_code(&self.component_token),
            armor_class: fold_code(&self.armor_class),
            weapon_class: fold_code(&self.weapon_class),
            ..self.clone()
        }
    }

    /// The key of a whole component file: facing, frame and fraction zero.
    pub fn file_key(&self) -> FrameKey {
        FrameKey {
            direction: 0,
            frame: 0,
            sub_frame: 0,
            ..self.folded()
        }
    }

    pub(crate) fn validate(&self) -> Result<(), PackError> {
        if self.kind > 4 || self.component >= 16 || self.direction >= 64 {
            return Err(PackError::Invalid("key range"));
        }
        for code in [
            &self.token,
            &self.mode,
            &self.component_token,
            &self.armor_class,
            &self.weapon_class,
        ] {
            if !code
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b' ' || *b == 0)
                || !code.iter().any(u8::is_ascii_alphanumeric)
            {
                return Err(PackError::Invalid("token"));
            }
        }
        Ok(())
    }
}

impl FramePack {
    pub fn validate(&self) -> Result<(), PackError> {
        if self.frames.len() > MAX_FRAMES {
            return Err(PackError::Invalid("frame limit"));
        }
        let mut bytes = 46usize;
        for (key, picture) in &self.frames {
            key.validate()?;
            let pixels = u64::from(picture.width) * u64::from(picture.height);
            if picture.width == 0
                || picture.height == 0
                || pixels > MAX_PIXELS
                || pixels != picture.pixels.len() as u64
            {
                return Err(PackError::Invalid("picture dimensions"));
            }
            if i64::from(picture.x_off).abs() > MAX_OFFSET
                || i64::from(picture.y_off).abs() > MAX_OFFSET
            {
                return Err(PackError::Invalid("picture offsets"));
            }
            bytes = bytes
                .checked_add(43 + picture.pixels.len())
                .ok_or(PackError::Invalid("byte limit"))?;
            if bytes > MAX_BYTES {
                return Err(PackError::Invalid("byte limit"));
            }
        }
        Ok(())
    }

    /// Canonical key order produces identical bytes in the cache and mod packs.
    pub fn encode(&self) -> Result<Vec<u8>, PackError> {
        self.validate()?;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.input_hash);
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for (k, f) in &self.frames {
            out.push(k.kind);
            out.extend_from_slice(&k.token);
            out.extend_from_slice(&k.mode);
            out.push(k.component);
            out.extend_from_slice(&k.component_token);
            out.extend_from_slice(&k.armor_class);
            out.extend_from_slice(&k.weapon_class);
            out.push(k.direction);
            out.extend_from_slice(&k.frame.to_le_bytes());
            out.push(k.sub_frame);
            out.extend_from_slice(&f.width.to_le_bytes());
            out.extend_from_slice(&f.height.to_le_bytes());
            out.extend_from_slice(&f.x_off.to_le_bytes());
            out.extend_from_slice(&f.y_off.to_le_bytes());
            out.push(match f.anchor {
                FrameAnchor::Top => 0,
                FrameAnchor::Bottom => 1,
                FrameAnchor::TopDown => 2,
            });
            out.extend_from_slice(&f.pixels);
        }
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
        let count = u32::from_le_bytes(r.take()?) as usize;
        if count > MAX_FRAMES {
            return Err(PackError::Invalid("frame limit"));
        }
        let mut frames = BTreeMap::new();
        for _ in 0..count {
            let key = FrameKey {
                kind: r.byte()?,
                token: r.take()?,
                mode: r.take()?,
                component: r.byte()?,
                component_token: r.take()?,
                armor_class: r.take()?,
                weapon_class: r.take()?,
                direction: r.byte()?,
                frame: u16::from_le_bytes(r.take()?),
                sub_frame: r.byte()?,
            };
            key.validate()?;
            if frames.contains_key(&key) {
                return Err(PackError::Invalid("duplicate key"));
            }
            let width = u32::from_le_bytes(r.take()?);
            let height = u32::from_le_bytes(r.take()?);
            let x_off = i32::from_le_bytes(r.take()?);
            let y_off = i32::from_le_bytes(r.take()?);
            if i64::from(x_off).abs() > MAX_OFFSET || i64::from(y_off).abs() > MAX_OFFSET {
                return Err(PackError::Invalid("picture offsets"));
            }
            let anchor = match r.byte()? {
                0 => FrameAnchor::Top,
                1 => FrameAnchor::Bottom,
                2 => FrameAnchor::TopDown,
                _ => return Err(PackError::Invalid("anchor")),
            };
            let pixels = u64::from(width) * u64::from(height);
            if width == 0 || height == 0 || pixels > MAX_PIXELS {
                return Err(PackError::Invalid("picture dimensions"));
            }
            let len = pixels as usize;
            let data =
                r.0.get(..len)
                    .ok_or(PackError::Invalid("truncated pixels"))?
                    .to_vec();
            r.0 = &r.0[len..];
            frames.insert(
                key,
                IndexFrame {
                    width,
                    height,
                    x_off,
                    y_off,
                    anchor,
                    pixels: data,
                },
            );
        }
        if !r.0.is_empty() {
            return Err(PackError::Invalid("trailing bytes"));
        }
        Ok(Self { input_hash, frames })
    }
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
#[path = "frame_pack_tests.rs"]
mod tests;
