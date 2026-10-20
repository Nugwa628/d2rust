//! The mode set without the combat-list drop
//! that ([`super::modes::set_mode`]) runs first. C→S 0x53 /
//! 0x54 call it directly.
//!
//! TODO: [`super::modes::set_mode`] repeats this body
//! after its drop; fold it onto [`set_mode_only`] when `modes.rs` is
//! next edited.

use crate::game::GameError;

use super::hooks::{Sim, UnitHooks};
use super::modes::{monster_mode, UnitError};
use super::record::flags;
use super::{UnitId, UnitType};

/// A tile does nothing; otherwise the mode is
/// written to +0x10, flags |= 1 and the unit is queued for update; a new
/// mode also re-initialises the animation fields (`UnitHooks::reinit_anim`).
/// A monster staying in mode 1 is left alone.
pub fn set_mode_only<H: UnitHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    mode: u32,
) -> Result<(), UnitError> {
    let rec = sim
        .units
        .get_mut(unit)
        .ok_or(UnitError::UnknownUnit(unit))?;
    if rec.ty == UnitType::Tile {
        return Ok(());
    }
    let changed = rec.mode != mode;
    if !changed && rec.ty == UnitType::Monster && mode == monster_mode::NU {
        return Ok(());
    }
    rec.mode = mode;
    rec.flags |= flags::CHANGED;
    sim.game.lists.queue_update(unit).map_err(GameError::from)?;
    if changed {
        hooks.reinit_anim(sim, unit);
    }
    Ok(())
}
