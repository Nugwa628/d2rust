//! The frame store: the resident frame sets under their [`FrameSetKey`]s,
//! each frame numbered with a scene [`FrameId`].
//!
//! The store is the one place frame ids come from. It
//! - answers the question the composite asks, `(FrameSetKey, index)` →
//!   `FrameId` ([`crate::composite::FrameIds`]);
//! - is the CPU compositor's frame source ([`scene::FrameSource`]);
//! - builds the GPU atlas whose slot `n` holds `FrameId(n)`
//!   ([`FrameStore::atlas`], the `SlotSource` numbering of
//!   [`mod@crate::gpu_compositor::pack`]).
//!
//! Plain Rust, no Bevy types. Ids are dense and follow insertion order
//! (set by set, frames in file order), so the same insertions always give
//! the same ids and therefore the same draw lists. Lookups are strict: a
//! key inserted twice, an unknown key or a frame index past the set's end
//! is an error, never a default frame.
//!
//! Not decided here: which sets are resident and when they are evicted
//! (residency, see `crate::assets::cache`). A store is built for what the
//! caller made resident; eviction rebuilds it.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use super::{Atlas, FrameSet, FrameSetKey, IndexFrame};
use crate::gpu_compositor::{AtlasFrames, GpuError};
use crate::scene::{self, FrameId, FrameView, SceneError};

/// Errors of the frame store.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// A set is already stored under this key.
    #[error("frame set {} {} is already in the store", .0.path(), .0.part())]
    Duplicate(FrameSetKey),
    /// No set is stored under this key.
    #[error("frame set {} {} is not resident", .0.path(), .0.part())]
    NotResident(FrameSetKey),
    /// The set has fewer frames than the index asked for.
    #[error("frame set {} {} has {len} frames, frame {index} asked", .key.path(), .key.part())]
    Index {
        key: FrameSetKey,
        index: usize,
        len: usize,
    },
    /// Adding the set would overflow the `FrameId` range.
    #[error("the store holds {held} frames; {adding} more exceed the FrameId range")]
    Full { held: usize, adding: usize },
}

/// Where a set's frames sit in the id space: `FrameId(first + i)` is frame
/// `i` of the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    first: u32,
    len: u32,
}

/// Resident frame sets and their scene ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameStore {
    sets: BTreeMap<FrameSetKey, Range>,
    /// `frames[n]` is `FrameId(n)`; `owners[n]` its set and index.
    frames: Vec<IndexFrame>,
    /// `owners[n]` is the set and index `FrameId(n)` was given for.
    owners: Vec<(FrameSetKey, usize)>,
}

impl FrameStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `set` under `key`; its frames get the next ids in file order.
    /// Returns the id of frame 0 (the next free id when the set is empty).
    /// On error the store is unchanged.
    pub fn insert(&mut self, key: FrameSetKey, set: FrameSet) -> Result<FrameId, StoreError> {
        if self.sets.contains_key(&key) {
            return Err(StoreError::Duplicate(key));
        }
        let full = StoreError::Full {
            held: self.frames.len(),
            adding: set.frames.len(),
        };
        let first = u32::try_from(self.frames.len()).map_err(|_| full.clone())?;
        let len = u32::try_from(set.frames.len()).map_err(|_| full.clone())?;
        first.checked_add(len).ok_or(full)?;
        for index in 0..set.frames.len() {
            self.owners.push((key.clone(), index));
        }
        self.frames.extend(set.frames);
        self.sets.insert(key, Range { first, len });
        Ok(FrameId(first))
    }

    /// The scene id of frame `index` of the resident set `key`.
    pub fn id(&self, key: &FrameSetKey, index: usize) -> Result<FrameId, StoreError> {
        let range = self
            .sets
            .get(key)
            .ok_or_else(|| StoreError::NotResident(key.clone()))?;
        if index >= range.len as usize {
            return Err(StoreError::Index {
                key: key.clone(),
                index,
                len: range.len as usize,
            });
        }
        Ok(FrameId(range.first + index as u32))
    }

    /// The frame with scene id `id`.
    pub fn frame(&self, id: FrameId) -> Option<&IndexFrame> {
        self.frames.get(id.0 as usize)
    }

    /// The set and index `id` was given for.
    pub fn owner(&self, id: FrameId) -> Option<(&FrameSetKey, usize)> {
        self.owners.get(id.0 as usize).map(|(k, i)| (k, *i))
    }

    /// Whether `key` is in the store.
    pub fn contains(&self, key: &FrameSetKey) -> bool {
        self.sets.contains_key(key)
    }

    /// Frames held (the id range is `0..len`).
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether the store holds no frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Every frame in id order.
    pub fn frames(&self) -> &[IndexFrame] {
        &self.frames
    }

    /// An atlas of every frame, packed as one set in id order, so
    /// `slots[n]` is the slot of `FrameId(n)`. The atlas holds pixels
    /// only; the format offsets and anchors stay with the store.
    pub fn atlas(&self, max_pages: u32) -> Result<AtlasFrames, GpuError> {
        let mut atlas = Atlas::new(max_pages)?;
        let slots = atlas.insert_set(&self.frames)?;
        Ok(AtlasFrames { atlas, slots })
    }
}

/// The store as the CPU compositor's frame source: `FrameId(n)` is frame
/// `n`; an id past the end is [`SceneError::FrameMissing`].
impl scene::FrameSource for FrameStore {
    fn frame(&self, id: FrameId) -> Result<FrameView<'_>, SceneError> {
        let f = self
            .frames
            .get(id.0 as usize)
            .ok_or(SceneError::FrameMissing(id))?;
        FrameView::new(f.width, f.height, &f.pixels)
    }
}
