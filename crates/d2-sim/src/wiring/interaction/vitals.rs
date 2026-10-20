//! [`VitalsUnits`] on the real providers: unit records (type, class),
//! the stat lists (base and unit getters, base set and add, and the life,
//! mana and stamina maxima); the refresh and the level-up notifications
//! and event stay a seam ([`VitalsRest`]).
//!
//! Regeneration (timer event 3) already runs on
//! the same stat lists in the unit dispatch
//! ([`crate::units::dispatch::player_regen`]); its maxima are the ones
//! [`VitalsView`] reads, so a level-up's new maximum is what the next
//! regeneration tick fills up to.
//!
//! Experience on a kill: [`kill_experience`] is the distribution's solo
//! path; the kill on the action wiring runs the
//! whole distribution (`wiring::action::reaction::kill`).

use crate::combat::vitals::experience::player_gain;
use crate::combat::vitals::{add_experience_at, stat, VitalsTables, VitalsUnits};
use crate::stats::{StatHost, StatLists};
use crate::units::record::Units;
use crate::units::{UnitId, UnitType};

/// The vitals calls that are not answered from the real providers.
pub trait VitalsRest {
    /// Refreshes the unit's derived data after a strength / dexterity change.
    fn refresh(&mut self, u: UnitId);
    /// Level-up notification: party roster, level-up sound, the level
    /// broadcast, the item and host callbacks (transport, party, items).
    fn level_up_notify(&mut self, u: UnitId);
    /// Unit event 12 `levelup` (the event registry is not in d2-sim).
    fn level_up_event(&mut self, u: UnitId);
}

/// The vitals' view: unit records, stat lists, the stat host and the
/// rest.
pub struct VitalsView<'a, H, R> {
    pub units: &'a Units,
    pub stats: &'a mut StatLists,
    pub hooks: &'a mut H,
    pub rest: &'a mut R,
}

impl<H: StatHost, R: VitalsRest> VitalsUnits for VitalsView<'_, H, R> {
    type Unit = UnitId;

    fn unit_type(&self, u: UnitId) -> UnitType {
        self.units.get(u).map_or(UnitType::Tile, |r| r.ty)
    }
    fn class_id(&self, u: UnitId) -> i32 {
        self.units.get(u).map_or(0, |r| r.class as i32)
    }
    /// The unit's base value of the stat (layer 0).
    fn base_stat(&self, u: UnitId, s: u16) -> i32 {
        self.stats.unit_base(u, s, 0)
    }
    /// The unit's total value of the stat (layer 0).
    fn stat(&self, u: UnitId, s: u16) -> i32 {
        self.stats.unit_total(u, s, 0)
    }
    /// Sets the unit's base value of the stat (layer 0).
    fn set_base_stat(&mut self, u: UnitId, s: u16, v: i32) {
        self.stats.unit_set(&mut *self.hooks, u, s, v, 0);
    }
    /// Adds to the unit's base value of the stat (layer 0).
    fn add_base_stat(&mut self, u: UnitId, s: u16, v: i32) {
        self.stats.unit_add(&mut *self.hooks, u, s, v, 0);
    }
    fn max_life(&self, u: UnitId) -> i32 {
        self.stats.max_life(u)
    }
    fn max_mana(&self, u: UnitId) -> i32 {
        self.stats.max_mana(u)
    }
    fn max_stamina(&self, u: UnitId) -> i32 {
        self.stats.max_stamina(u)
    }
    fn refresh(&mut self, u: UnitId) {
        self.rest.refresh(u);
    }
    fn level_up_notify(&mut self, u: UnitId) {
        self.rest.level_up_notify(u);
    }
    fn level_up_event(&mut self, u: UnitId) {
        self.rest.level_up_event(u);
    }
}

/// Experience for a player `attacker` killing `defender`: the solo
/// path of the experience distribution: the gain from the defender's
/// base experience at both base levels, then the add at the player's base
/// level. Returns the gain added
/// (0 for a non-player attacker). The full distribution (credited player,
/// hireling and party shares) is [`crate::combat::vitals::experience::distribute`].
pub fn kill_experience<W: VitalsUnits>(
    w: &mut W,
    t: &VitalsTables,
    attacker: W::Unit,
    defender: W::Unit,
) -> u32 {
    if w.unit_type(attacker) != UnitType::Player {
        return 0;
    }
    let e = w.base_stat(defender, stat::EXPERIENCE);
    if e <= 0 {
        return 0;
    }
    let dl = w.base_stat(defender, stat::LEVEL);
    let pl = w.base_stat(attacker, stat::LEVEL);
    let gain = player_gain(w, t, e, attacker, pl, dl);
    add_experience_at(w, t, attacker, pl, gain);
    gain as u32
}
