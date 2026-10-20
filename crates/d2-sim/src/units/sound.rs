//! The per-unit sound-event slot and its flush as S→C 0x2C.
//!
//! Stores the event at unit u16 +0x6E,
//! the target at +0x70, sets unit flag 0x400 and queues the unit for
//! update; a second call before the flush overwrites both.
//! The per-client unit update sends it as
//! S→C 0x2C through when the target is none
//! or that client's player; the room clean-up clears flag
//! 0x400.
//!
//! Here the slot is [`SoundEvents`] on the [`Game`]: one entry per unit,
//! in a `BTreeMap` (unit order, never iterated for an outcome). An entry
//! present is unit flag 0x400 set; the entry holds +0x6E and +0x70.

use std::collections::BTreeMap;

use crate::game::Game;
use crate::units::{ListError, UnitId};

/// S→C 0x2C PlaySound (`server-messages.tsv`: 8 bytes).
pub const PLAY_SOUND: u8 = 0x2C;

/// One unit's queued sound event: unit u16 +0x6E and its target +0x70
/// (`None`: 0, every client).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundSlot {
    pub event: u16,
    pub target: Option<UnitId>,
}

/// The units with unit flag 0x400 (a queued sound event), with their
/// slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoundEvents {
    slots: BTreeMap<UnitId, SoundSlot>,
}

impl SoundEvents {
    /// The unit's slot while its flag 0x400 is set.
    pub fn get(&self, unit: UnitId) -> Option<SoundSlot> {
        self.slots.get(&unit).copied()
    }

    /// Stores the event and target and sets flag 0x400; the last call
    /// before the flush wins.
    pub fn set(&mut self, unit: UnitId, event: u16, target: Option<UnitId>) {
        self.slots.insert(unit, SoundSlot { event, target });
    }

    /// Clears flag 0x400 (the room clean-up; unit removal).
    pub fn clear(&mut self, unit: UnitId) {
        self.slots.remove(&unit);
    }

    /// Every queued slot in unit order (the sound-flag units).
    pub fn queued(&self) -> impl Iterator<Item = (UnitId, SoundSlot)> + '_ {
        self.slots.iter().map(|(&u, &s)| (u, s))
    }

    /// No unit has a queued event.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// The slot, then the
/// unit queued for update. A unit not in the lists keeps
/// its slot and the list error is returned.
pub fn queue_sound(
    game: &mut Game,
    unit: UnitId,
    event: u16,
    target: Option<UnitId>,
) -> Result<(), ListError> {
    game.sounds.set(unit, event, target);
    game.lists.queue_update(unit)
}

/// The S→C 0x2C bytes: `2C`, unit type u8 @1, GUID u32
/// @2, event u16 @6.
pub fn play_sound_message(unit_type: u8, guid: u32, event: u16) -> [u8; 8] {
    let mut b = [0; 8];
    b[0] = PLAY_SOUND;
    b[1] = unit_type;
    b[2..6].copy_from_slice(&guid.to_le_bytes());
    b[6..8].copy_from_slice(&event.to_le_bytes());
    b
}

/// With flag 0x400 set, the
/// 0x2C for the client whose player is `receiver`, only when the
/// target is none or `receiver`. `None`: nothing to send (no slot, the
/// target is another player, or the unit is not in the lists).
pub fn sound_message(game: &Game, unit: UnitId, receiver: UnitId) -> Option<[u8; 8]> {
    let slot = game.sounds.get(unit)?;
    if slot.target.is_some_and(|t| t != receiver) {
        return None;
    }
    let e = game.lists.unit(unit)?;
    Some(play_sound_message(e.ty as u8, e.guid, slot.event))
}

#[cfg(test)]
mod tests;
