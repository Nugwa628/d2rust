//! The forced target on the wired units: the monster-data override
//! (kind and GUID) that terror, attract and confuse set, and the check
//! that the main search and the secondary search run first
//! ([`crate::monsters::ai::forced::forced_target`] over [`Forced`]).

use crate::game::Game;
use crate::monsters::ai::forced::{forced_target, ForcedWorld};
use crate::monsters::ai::{AiStore, AiWorld};
use crate::units::{UnitId, UnitType};

use super::{Pending, View};

/// Override kinds the setter accepts (kind < 5).
const KIND_LIMIT: u32 = 5;

impl<X: Pending> View<'_, X> {
    /// Set the forced target: a monster whose monstats `switchai` is set
    /// takes `kind` (< 5) and `guid`; anything else is left alone.
    pub fn set_target_override(&mut self, unit: UnitId, kind: u32, guid: u32) {
        let Some(class) = self
            .units
            .get(unit)
            .filter(|r| r.ty == UnitType::Monster)
            .map(|r| r.class)
        else {
            return;
        };
        let switchai = self
            .h
            .tables
            .combat
            .monstats
            .get(class as usize)
            .is_some_and(|m| m.switchai);
        if !switchai || kind >= KIND_LIMIT {
            return;
        }
        if let Some(d) = self.h.monster_data_mut(unit) {
            d.override_kind = kind;
            d.override_guid = guid;
        }
    }

    /// Clear the forced target: kind and GUID back to 0.
    pub fn clear_target_override(&mut self, unit: UnitId) {
        if let Some(d) = self.h.monster_data_mut(unit) {
            d.override_kind = 0;
            d.override_guid = 0;
        }
    }

    /// Alignment: a player or monster's
    /// stat 172 in its state-105 list (0 without the list); 2 for any
    /// other unit type; 0 for no unit.
    pub fn alignment_stat(&self, unit: UnitId) -> u8 {
        match self.units.get(unit).map(|r| r.ty) {
            Some(UnitType::Player | UnitType::Monster) => self
                .state_list(unit, crate::combat::range::STATE_ALIGNMENT)
                .map_or(0, |l| {
                    self.stats
                        .base(l, crate::combat::range::STAT_ALIGNMENT, 0)
                        .clamp(0, 2) as u8
                }),
            Some(_) => 2,
            None => 0,
        }
    }

    /// The forced target of `unit` and its distance, or none.
    pub fn forced(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        a: bool,
        s: bool,
        ai: Option<&AiStore>,
    ) -> Option<(UnitId, i32)> {
        forced_target(&mut Forced { v: self, game, ai }, unit, a, s)
    }
}

/// [`ForcedWorld`] over the wired units and the game.
pub struct Forced<'v, 'a, 'g, X: Pending> {
    /// The view of the wired units.
    pub v: &'v mut View<'a, X>,
    /// The game.
    pub game: &'g mut Game,
    /// The AI store when lent out of the view (minion owners).
    pub ai: Option<&'g AiStore>,
}

impl<X: Pending> ForcedWorld for Forced<'_, '_, '_, X> {
    fn override_of(&self, unit: UnitId) -> Option<(u32, u32)> {
        if self.v.units.get(unit)?.ty != UnitType::Monster {
            return None;
        }
        let d = self.v.h.monster_data(unit)?;
        Some((d.override_kind, d.override_guid))
    }
    fn clear_override(&mut self, unit: UnitId) {
        self.v.clear_target_override(unit);
    }
    fn find_by_guid(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.game.lists.find_unit(ty, guid)
    }
    /// Distance from `from` to `to` with `from`'s size subtracted.
    fn full_distance(&self, from: UnitId, to: UnitId) -> i32 {
        crate::monsters::ai::distance_full_size(
            self.v.h.path_position(from),
            self.v.path_size(from),
            self.v.h.path_position(to),
        )
    }
    fn line_blocked(&self, a: UnitId, b: UnitId) -> bool {
        AiWorld::line_blocked(&*self.v, self.game, a, b)
    }
    fn alignment(&self, unit: UnitId) -> u8 {
        self.v.alignment_stat(unit)
    }
    fn set_alignment(&mut self, unit: UnitId, value: u8) {
        self.v.set_alignment(self.game, unit, value);
    }
    fn seed_bit(&mut self, unit: UnitId) -> u32 {
        crate::monsters::ai::AiUnits::seed(self.v, unit).step() & 1
    }
    /// Scan 5 (radius 35, LOS flag `a`) or scan 6: the main slot.
    fn scan(&mut self, unit: UnitId, scan: u8, a: bool) -> Option<(UnitId, i32)> {
        if scan == 6 {
            self.v.scan6(self.game, unit, self.ai).main
        } else {
            self.v.scan5(self.game, unit, a, self.ai).main
        }
    }
    fn unit_type(&self, unit: UnitId) -> Option<UnitType> {
        self.v.units.get(unit).map(|r| r.ty)
    }
    fn is_dead(&self, unit: UnitId) -> bool {
        self.v.units.is_dead(unit)
    }
}
