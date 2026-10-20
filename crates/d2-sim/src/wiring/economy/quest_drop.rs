//! The quest drop helper on the action wiring's units, stats and DRLG, as the
//! chest drop of [`super::object_chest_drop`]: the unit is the dropper (its
//! unit seed draws the class pick; item level from its kind), the item a real
//! item unit at the floor drop of the unit's room and position
//! ([`super::ItemDrops`]). The game's drop state ([`DeathDrops`]) holds the
//! item tables; the `levels` rows are the action tables'.
//!
//! The drop itself is [`drop_helpers::source_drop`] (one implementation for
//! the objects, quests and unique-monster-modifier callbacks).
//!
//! Callers: the quests' `drop_item_at` / `quest_drop`
//! ([`super::HostQuests`]) and the unique-monster-modifier callback's drop
//! (the world host).

use std::sync::Arc;

use super::{drop_helpers, DeathDrops, FreeSpot};
use crate::treasure::class_pick::ClassPicks;
use crate::units::hooks::Sim;
use crate::units::UnitId;
use crate::wiring::action::{ActionHooks, Pending};

/// Drops from `unit` with its drop code `drop_code`: the created item, or
/// none. One implementation: [`drop_helpers::source_drop`], with the action
/// tables' `levels`. The item is also appended to [`DeathDrops::placed`]; a
/// pick fatal goes to [`DeathDrops::pick_errors`]. A drop state without pick
/// rows ([`DeathDrops::picks`] empty) picks from its item tables
/// ([`ClassPicks::of_items`], the same combined array).
#[allow(clippy::too_many_arguments)]
pub fn unit_quest_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    spots: &mut F,
    unit: UnitId,
    drop_code: Option<[u8; 4]>,
    quality: u8,
    p6: i32,
    p7: i32,
) -> Option<UnitId> {
    sim.units.get(unit)?;
    let saved = d.picks.rows.is_empty().then(|| {
        let t = &d.tables.items;
        std::mem::replace(
            &mut d.picks,
            Arc::new(ClassPicks::of_items(&t.items, t.parts)),
        )
    });
    let levels = h.tables.clone();
    let code = drop_code.map_or(0, u32::from_le_bytes);
    let (item, _) = drop_helpers::source_drop(
        h,
        sim,
        d,
        &levels.levels,
        spots,
        unit,
        code,
        quality,
        p6,
        p7 != 0,
    );
    if let Some(p) = saved {
        d.picks = p;
    }
    item
}
