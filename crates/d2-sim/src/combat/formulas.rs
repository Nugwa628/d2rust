//! Combat's side of a mod game's formula hooks (MS3,
//! [`crate::rules::FormulaHooks`]): the formula sites in [`super::hit`]
//! and [`super::damage`] ask the game's hooks first through [`ask`] /
//! [`ask_shared`], handing them the formula's units as a
//! [`FormulaHost`] over the combat world. Without hooks (every D2RUST
//! game) nothing is asked and D2RUST's formula runs.

use super::CombatWorld;
use crate::rng::Seed;
use crate::rules::{FormulaHooks, FormulaHost, Side};
use crate::units::UnitType;

/// The combat world, borrowed shared (formulas that never draw) or
/// mutably (formulas that may draw from a unit's seed).
enum World<'a, W> {
    Shared(&'a W),
    Mut(&'a mut W),
}

/// A formula's units on the combat world.
struct Host<'a, W: CombatWorld> {
    w: World<'a, W>,
    a: Option<W::Unit>,
    d: Option<W::Unit>,
}

impl<W: CombatWorld> Host<'_, W> {
    fn w(&self) -> &W {
        match &self.w {
            World::Shared(w) => w,
            World::Mut(w) => w,
        }
    }

    fn unit(&self, side: Side) -> Option<W::Unit> {
        match side {
            Side::Attacker => self.a,
            Side::Defender => self.d,
        }
    }
}

impl<W: CombatWorld> FormulaHost for Host<'_, W> {
    fn unit_type(&self, side: Side) -> Option<UnitType> {
        self.unit(side).map(|u| self.w().unit_type(u))
    }
    fn class_id(&self, side: Side) -> i32 {
        self.unit(side).map_or(0, |u| self.w().class_id(u))
    }
    fn stat(&self, side: Side, stat: u16, layer: u16) -> i32 {
        self.unit(side).map_or(0, |u| self.w().stat(u, stat, layer))
    }
    fn has_state(&self, side: Side, state: u16) -> bool {
        self.unit(side)
            .is_some_and(|u| self.w().has_state(u, state))
    }
    fn difficulty(&self) -> usize {
        self.w().difficulty()
    }
    fn expansion(&self) -> bool {
        self.w().expansion()
    }
    fn seed(&mut self, side: Side) -> Option<&mut Seed> {
        let u = self.unit(side)?;
        match &mut self.w {
            World::Shared(_) => None,
            World::Mut(w) => Some(w.seed(u)),
        }
    }
}

/// Asks the game's formula hooks with attacker `a` and defender `d`;
/// `None` when the game has no hooks or the hook keeps D2RUST's formula.
pub(crate) fn ask<W: CombatWorld, T>(
    w: &mut W,
    a: Option<W::Unit>,
    d: Option<W::Unit>,
    f: impl FnOnce(&dyn FormulaHooks, &mut dyn FormulaHost) -> Option<T>,
) -> Option<T> {
    w.rules().formulas()?;
    // The ruleset is cloned (one `Arc`) so the hook can borrow the world.
    let rules = w.rules().clone();
    let hooks = rules.formulas()?;
    f(
        hooks,
        &mut Host {
            w: World::Mut(w),
            a,
            d,
        },
    )
}

/// [`ask`] for formulas that never draw: the host hands out no seed.
pub(crate) fn ask_shared<W: CombatWorld, T>(
    w: &W,
    a: Option<W::Unit>,
    d: Option<W::Unit>,
    f: impl FnOnce(&dyn FormulaHooks, &mut dyn FormulaHost) -> Option<T>,
) -> Option<T> {
    let hooks = w.rules().formulas()?;
    f(
        hooks,
        &mut Host {
            w: World::Shared(w),
            a,
            d,
        },
    )
}
