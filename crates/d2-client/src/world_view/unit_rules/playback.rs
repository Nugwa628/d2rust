//! Resident frame-pack playback. Selection is visual only: the pose retains
//! the starting source frame for COF order and simulation events.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::composite::ComponentFrame;
use crate::frames::{FramePart, FrameSet, FrameSetKey, FrameStore};
use crate::rules::unit_composite::CompositeKind;
use crate::world_view::unit_shadow;

use super::animation_deny::AnimationDeny;
use super::frame_pack::{FrameKey, FramePack, PackError};
use super::frame_source::{FrameRequest, FrameSource};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnimationMode {
    #[default]
    Original,
    Smooth,
}

impl AnimationMode {
    /// The Video row's name and the dev command's argument.
    pub fn name(self) -> &'static str {
        match self {
            AnimationMode::Original => "Original",
            AnimationMode::Smooth => "Smooth",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PackFrames {
    pub mode: AnimationMode,
    /// Animations that keep their source pictures whatever `mode` says.
    pub(crate) deny: AnimationDeny,
    frames: BTreeMap<FrameKey, ComponentFrame>,
}

impl PackFrames {
    /// Validate the entire pack and every key before touching either map, so
    /// a rejected or duplicate pack leaves all resident IDs intact. Keys are
    /// folded ([`FrameKey::folded`]). Only the pack's own pictures are added:
    /// the resident store is never copied, so an install costs the pack's
    /// size on the render thread, not the store's.
    pub fn install(&mut self, pack: &FramePack, store: &mut FrameStore) -> Result<(), PackError> {
        pack.validate()?;
        let digest: String = pack.input_hash.iter().map(|b| format!("{b:02x}")).collect();
        let mut planned = Vec::with_capacity(pack.frames.len());
        let mut keys = std::collections::BTreeSet::new();
        for (key, picture) in &pack.frames {
            let key = key.folded();
            if self.frames.contains_key(&key) || !keys.insert(key.clone()) {
                return Err(PackError::Invalid("resident duplicate key"));
            }
            let key_hash = format!("{:x}", Sha256::digest(format!("{key:?}").as_bytes()));
            let set = FrameSetKey::new(
                format!("generated/animation/{digest}/{key_hash}.dc6"),
                FramePart::Dir(key.direction),
            )
            .map_err(|_| PackError::Invalid("resident path"))?;
            let resident = match store.id(&set, 0) {
                Ok(id) if store.frame(id) != Some(picture) => {
                    return Err(PackError::Invalid("resident picture mismatch"))
                }
                Ok(_) => true,
                Err(_) => false,
            };
            // The derived shadow set, as for every resident source set: the
            // shadow pass looks up `<set>#shadow`, and a generated picture
            // without one would cast no shadow on its quartiles only.
            let shadow =
                unit_shadow::shadow_key(&set).map_err(|_| PackError::Invalid("resident path"))?;
            planned.push((key, set, shadow, picture, resident));
        }
        for (key, set, shadow, picture, resident) in planned {
            if !resident {
                let frames = FrameSet {
                    frames: vec![picture.clone()],
                };
                let shadows = unit_shadow::shadow_set(&frames);
                store
                    .insert(set.clone(), frames)
                    .map_err(|_| PackError::Invalid("resident frame"))?;
                store
                    .insert(shadow, shadows)
                    .map_err(|_| PackError::Invalid("resident frame"))?;
            }
            self.frames.insert(key, ComponentFrame { set, index: 0 });
        }
        Ok(())
    }

    /// Generated pictures resident for selection.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl FrameSource for PackFrames {
    fn frame(&self, req: &FrameRequest<'_>, source: ComponentFrame) -> ComponentFrame {
        if self.mode == AnimationMode::Original
            || req.sub_frame < 64
            || req.frame != source.index
            || self
                .deny
                .denies(req.codes.kind, &req.codes.unit_token, &req.codes.mode_token)
        {
            return source;
        }
        let Ok(frame) = u16::try_from(req.frame) else {
            return source;
        };
        let mut k = FrameKey {
            kind: match req.codes.kind {
                CompositeKind::Player => 0,
                CompositeKind::Monster => 1,
                CompositeKind::Object => 2,
            },
            token: req.codes.unit_token,
            mode: req.codes.mode_token,
            component: req.component,
            component_token: req.codes.component_token,
            armor_class: req.codes.armor_class,
            weapon_class: req.codes.weapon_class,
            direction: req.direction,
            frame,
            // Hold each generated quartile until the next one; never advance
            // the source frame or its COF layer order early.
            sub_frame: req.sub_frame / 64 * 64,
        }
        .folded();
        if let Some(frame) = self.frames.get(&k) {
            return frame.clone();
        }
        // A facing between the file's facings draws the source facing's
        // samples, matching the source cel drawn at the other quartiles.
        k.direction = req.source_direction;
        self.frames.get(&k).cloned().unwrap_or(source)
    }
}

/// The dev console's `animation` line: no argument toggles the running
/// client between Original and Smooth; `original` and `smooth` (also `off`
/// and `on`) pick one. The stored Video setting is left alone.
pub fn dev_animation(arg: &str, frames: &SharedPackFrames) -> Result<String, String> {
    let mode = match arg.trim().to_ascii_lowercase().as_str() {
        "" => match frames.mode() {
            AnimationMode::Original => AnimationMode::Smooth,
            AnimationMode::Smooth => AnimationMode::Original,
        },
        "original" | "off" => AnimationMode::Original,
        "smooth" | "on" => AnimationMode::Smooth,
        _ => return Err("usage: animation [original|smooth]".into()),
    };
    frames.set_mode(mode);
    Ok(format!("animation {}", mode.name()))
}

/// Shared selection state for the loader and world-view rules.
#[derive(Debug, Clone, Default)]
pub struct SharedPackFrames(Arc<RwLock<PackFrames>>);

impl SharedPackFrames {
    pub fn mode(&self) -> AnimationMode {
        self.0.read().unwrap_or_else(|e| e.into_inner()).mode
    }
    pub fn set_mode(&self, mode: AnimationMode) {
        self.0.write().unwrap_or_else(|e| e.into_inner()).mode = mode;
    }
    /// Generated pictures resident for selection.
    pub fn len(&self) -> usize {
        self.0.read().unwrap_or_else(|e| e.into_inner()).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Drops the selection map; resident pictures stay in the store and are
    /// reused by a later install of the same pack.
    pub fn clear(&self) {
        let mut frames = self.0.write().unwrap_or_else(|e| e.into_inner());
        frames.frames.clear();
    }
    /// Appearance jobs can share components; keep their existing resident IDs.
    pub fn install(&self, pack: &FramePack, store: &mut FrameStore) -> Result<(), PackError> {
        pack.validate()?;
        let mut frames = self.0.write().unwrap_or_else(|e| e.into_inner());
        let missing = FramePack {
            input_hash: pack.input_hash,
            frames: pack
                .frames
                .iter()
                .filter(|(key, _)| !frames.frames.contains_key(&key.folded()))
                .map(|(key, picture)| (key.clone(), picture.clone()))
                .collect(),
        };
        frames.install(&missing, store)
    }
}
impl FrameSource for SharedPackFrames {
    fn frame(&self, req: &FrameRequest<'_>, source: ComponentFrame) -> ComponentFrame {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .frame(req, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_command_toggles_or_picks_a_mode_without_saving() {
        let frames = SharedPackFrames::default();
        frames.set_mode(AnimationMode::Smooth);
        assert_eq!(dev_animation("", &frames).unwrap(), "animation Original");
        assert_eq!(frames.mode(), AnimationMode::Original);
        assert_eq!(dev_animation("", &frames).unwrap(), "animation Smooth");
        assert_eq!(frames.mode(), AnimationMode::Smooth);
        assert_eq!(
            dev_animation("original", &frames).unwrap(),
            "animation Original"
        );
        assert_eq!(dev_animation("off", &frames).unwrap(), "animation Original");
        assert_eq!(dev_animation("on", &frames).unwrap(), "animation Smooth");
        assert_eq!(
            dev_animation("smooth", &frames).unwrap(),
            "animation Smooth"
        );
        assert!(dev_animation("fast", &frames).is_err());
        assert_eq!(frames.mode(), AnimationMode::Smooth);
    }
}
