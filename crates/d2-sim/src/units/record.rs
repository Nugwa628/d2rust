//! The unit record fields the simulation owns, beside the list
//! fields of [`super::lists`]: one [`UnitRecord`] per live unit in
//! [`Units`], keyed by [`UnitId`]. Fields whose owner module is not
//! written yet (path, inventory, per-kind data, combat list) are not
//! here; they stay with their systems.

use std::collections::BTreeMap;

use crate::rng::Seed;
use crate::stats::ListId;

use super::{UnitId, UnitType};

/// Unit flags (+0xC4).
pub mod flags {
    /// Changed: set by every mode set.
    pub const CHANGED: u32 = 0x1;
    /// Tile.
    pub const TILE: u32 = 0x2;
    /// Seed set (every allocation).
    pub const SEED_SET: u32 = 0x10;
    /// Cleared by attack-mode starts.
    pub const ATTACK_PENDING: u32 = 0x40;
    /// Hover freed.
    pub const HOVER_FREED: u32 = 0x100;
    /// Dead.
    pub const DEAD: u32 = 0x10000;
    /// Monster mode changing.
    pub const MODE_CHANGING: u32 = 0x80000;
}

/// Unit flags 2 (+0xC8).
pub mod flags2 {
    /// Disguised.
    pub const DISGUISE: u32 = 0x8;
    /// Expansion game.
    pub const EXPANSION: u32 = 0x0200_0000;
    /// Server unit (every allocation).
    pub const SERVER: u32 = 0x0400_0000;
}

/// Node index at allocation (+0xD0).
pub const INITIAL_NODE_INDEX: u32 = 11;

/// Event bytes per AnimData record (main-form bound).
pub const ANIM_EVENTS: usize = 144;

/// The AnimData record fields reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimRecord {
    /// Frames per direction.
    pub frames: u32,
    /// Byte +0x0F, the speed's high byte: event index −1 of the schedule
    /// variants (edge case 1).
    pub byte_0f: u8,
    /// Event bytes, +0x10 + i.
    pub events: [u8; ANIM_EVENTS],
}

/// A sequence's frame count, speed and event bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// +0x34, 8.8.
    pub frame_count: i32,
    /// +0x3C.
    pub speed: i32,
    /// +0x38, the position in the list, 8.8 (0 at the load).
    pub pos: i32,
    /// Event byte per frame index.
    pub events: Vec<u8>,
    /// Drawn frame per frame index (record +3): the frame setup
    /// Stores it ·256 in +0x44. Empty: +0x44 is left alone.
    pub drawn: Vec<u8>,
}

/// Animation fields (+0x30 … +0x50).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Anim {
    /// +0x30 (with +0x34, +0x3C).
    pub sequence: Option<Sequence>,
    /// +0x44, current frame, 8.8.
    pub frame: i32,
    /// +0x48, frame count, 8.8.
    pub frame_count: i32,
    /// +0x4C, i16, 1/256 frame per tick.
    pub speed: i16,
    /// +0x4E.
    pub action_frame: u8,
    /// +0x50.
    pub record: Option<AnimRecord>,
}

/// The interact info of a player unit: +0x64 GUID, +0x68 unit type,
/// +0x6C active (Inputs, state
/// table). One home for every system that reads or sets it (NPCs, the
/// cube, waypoints, objects, quests, the inventory's busy test; handoff
/// HM3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractInfo {
    /// +0x64.
    pub guid: u32,
    /// +0x68.
    pub ty: u32,
    /// +0x6C.
    pub active: bool,
}

/// The type the reset writes (type 6).
pub const INTERACT_RESET_TYPE: u32 = 6;

impl InteractInfo {
    /// The state [`InteractInfo::reset`] leaves.
    pub const INACTIVE: InteractInfo = InteractInfo {
        guid: u32::MAX,
        ty: INTERACT_RESET_TYPE,
        active: false,
    };

    /// (unit type, GUID) while active, else none.
    pub fn get(&self) -> Option<(u8, u32)> {
        self.active.then_some((self.ty as u8, self.guid))
    }

    /// (type, GUID) and active, ignored while already active.
    pub fn set(&mut self, ty: u8, guid: u32) {
        if !self.active {
            self.guid = guid;
            self.ty = u32::from(ty);
            self.active = true;
        }
    }

    /// GUID −1, type 6, active 0.
    pub fn reset(&mut self) {
        self.guid = u32::MAX;
        self.ty = INTERACT_RESET_TYPE;
        self.active = false;
    }
}

/// The simulation's unit record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitRecord {
    /// +0x00.
    pub ty: UnitType,
    /// +0x04.
    pub class: u32,
    /// +0x0C (also in the lists).
    pub guid: u32,
    /// +0x10.
    pub mode: u32,
    /// +0x18; +0x1C is the act record of the game.
    pub act: u8,
    /// +0x20, +0x28.
    pub seed: Seed,
    pub init_seed: u32,
    /// Item seed and start seed (item data +0x04, +0x10).
    pub item_seed: Option<(Seed, u32)>,
    pub anim: Anim,
    /// +0x5C.
    pub stats: Option<ListId>,
    /// +0xA4: the hover's timeout frame, when a hover is
    /// set.
    pub hover: Option<i32>,
    /// +0xC4.
    pub flags: u32,
    /// +0xC8.
    pub flags2: u32,
    /// +0x94 / +0x98: the source-unit link's owner type and GUID, valid
    /// while +0xC8 bit 0x400 is set.
    pub source: (u32, u32),
    /// +0xD0.
    pub node_index: u32,
    /// +0x64 / +0x68 / +0x6C ([`InteractInfo`]).
    pub interact: InteractInfo,
    /// +0xB0: the hit class of the last hit taken, read by the hit and
    /// mode messages.
    pub hit_class: u32,
}

impl UnitRecord {
    /// A record with every field at its allocation default.
    pub fn new(ty: UnitType, class: u32, guid: u32) -> Self {
        Self {
            ty,
            class,
            guid,
            mode: 0,
            act: 0,
            seed: Seed::init(),
            init_seed: 0,
            item_seed: None,
            anim: Anim::default(),
            stats: None,
            hover: None,
            flags: 0,
            flags2: 0,
            source: (0, 0),
            node_index: INITIAL_NODE_INDEX,
            // The zeroed allocation leaves
            // GUID 0, type 0, inactive; nothing in the unit creation writes them.
            interact: InteractInfo::default(),
            hit_class: 0,
        }
    }

    /// "Dead": flag 0x10000, a player in mode 0 or 17,
    /// a monster in mode 0 or 12, and every other unit type.
    pub fn is_dead(&self) -> bool {
        if self.flags & flags::DEAD != 0 {
            return true;
        }
        match self.ty {
            UnitType::Player => matches!(self.mode, 0 | 17),
            UnitType::Monster => matches!(self.mode, 0 | 12),
            _ => true,
        }
    }
}

/// The unit records of one game.
#[derive(Clone, Debug, Default)]
pub struct Units {
    records: BTreeMap<UnitId, UnitRecord>,
}

impl Units {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: UnitId) -> Option<&UnitRecord> {
        self.records.get(&id)
    }

    pub fn get_mut(&mut self, id: UnitId) -> Option<&mut UnitRecord> {
        self.records.get_mut(&id)
    }

    pub fn insert(&mut self, id: UnitId, record: UnitRecord) {
        self.records.insert(id, record);
    }

    pub fn remove(&mut self, id: UnitId) -> Option<UnitRecord> {
        self.records.remove(&id)
    }

    /// A missing unit counts as dead.
    pub fn is_dead(&self, id: UnitId) -> bool {
        self.get(id).is_none_or(UnitRecord::is_dead)
    }

    /// The records in id order.
    pub fn iter(&self) -> impl Iterator<Item = (UnitId, &UnitRecord)> {
        self.records.iter().map(|(&id, r)| (id, r))
    }
}

#[cfg(test)]
mod interact_tests {
    use super::*;

    /// The interact info: set while inactive, ignored while active; the
    /// reset writes GUID −1, type 6, inactive; the getter answers only
    /// while active.
    #[test]
    fn set_get_reset() {
        let mut r = UnitRecord::new(UnitType::Player, 1, 7);
        assert_eq!(r.interact.get(), None);
        r.interact.set(1, 0x10);
        assert_eq!(r.interact.get(), Some((1, 0x10)));
        r.interact.set(2, 0x20);
        assert_eq!(r.interact.get(), Some((1, 0x10)), "ignored while active");
        r.interact.reset();
        assert_eq!(
            r.interact,
            InteractInfo {
                guid: u32::MAX,
                ty: INTERACT_RESET_TYPE,
                active: false
            }
        );
        assert_eq!(r.interact.get(), None);
        r.interact.set(4, 0x30);
        assert_eq!(r.interact.get(), Some((4, 0x30)));
    }
}
