//! Combat: attack rating, defense, chance to hit, block and avoid ([`hit`]); damage
//! roll, resistances, application, leech and the death trigger ([`damage`]).
//!
//! The `events` module holds the unit event functions, `range` the hostility and melee
//! range rules, and `vitals` the life, mana and stamina handling.
//!
//! Unit and the effects other specs own (stat writes, states and
//! stat lists, timers, unit events, modes) go through the
//! [`CombatWorld`] seam, which extends [`crate::skills::SkillUnits`].
//! Table data comes from `d2_data` typed records ([`CombatTables`]).

/// Damage roll, resistances, application, leech and the death trigger.
pub mod damage;
/// The unit event functions (on-hit, on-kill and similar triggers).
pub mod events;
mod formulas;
#[cfg(test)]
mod formulas_tests;
/// Attack rating, defense, chance to hit, block and avoid.
pub mod hit;
/// Hostility and melee range rules.
pub mod range;
/// Life, mana and stamina handling.
pub mod vitals;

use crate::skills::SkillUnits;
use crate::units::UnitType;
use d2_data::bin::BinSet;
use d2_data::tables::{decode_all, Charstats, Difficultylevels, Monstats, Monstats2};

pub use damage::*;
pub use hit::*;

/// `pct(v, p, d)` (D2MOO `MONSTERUNIQUE_CalculatePercentage`): `v × p / d` with the
/// game's precision-losing shortcuts for large values. Signed 32-bit, truncating.
pub fn pct(v: i32, p: i32, d: i32) -> i32 {
    if d == 0 {
        return 0;
    }
    let wide = || (i64::from(v) * i64::from(p) / i64::from(d)) as i32;
    if v > 0x10_0000 {
        if d <= v >> 4 {
            v.wrapping_div(d).wrapping_mul(p)
        } else {
            wide()
        }
    } else if p > 0x1_0000 {
        if d <= p >> 4 {
            p.wrapping_div(d).wrapping_mul(v)
        } else {
            wide()
        }
    } else {
        v.wrapping_mul(p).wrapping_div(d)
    }
}

/// `(x × s) / 128` with a 32-bit product, truncating. Scales a damage value by the
/// skill's `SrcDam` (128 = 100%).
pub fn scale(x: i32, s: i32) -> i32 {
    x.wrapping_mul(s) / 128
}

/// Where a defender stands: no room, a town room, or a field room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomKind {
    /// No room.
    None,
    /// A town room.
    Town,
    /// Any room outside town.
    Field,
}

/// One entry of an attacker's combat list (D2MOO `pCombat`): a hit that has been rolled
/// and is waiting to be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatEntry {
    /// Attacker's unit type and GUID.
    pub attacker: (UnitType, u32),
    /// Defender's unit type and GUID.
    pub defender: (UnitType, u32),
    /// The rolled damage record.
    pub record: DamageRecord,
}

/// The unit queries and effects combat needs beyond [`SkillUnits`] (a seam).
///
/// Providers: the unit and stat layer (modes, ranges, stat writes, states, stat lists,
/// unit events, the combat list), the tick (timers), the monster layer (monster flags
/// and hooks) and items (durability, composits).
pub trait CombatWorld: SkillUnits {
    // ------------------------------------------------------------ game
    /// Current game frame.
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// True in an expansion (Lord of Destruction) game.
    fn expansion(&self) -> bool;
    /// Difficulty 0–2 (normal, nightmare, hell).
    fn difficulty(&self) -> usize;
    /// The element hit-class byte, shared by the whole server process (one counter, not
    /// one per game).
    fn hit_class_counter(&mut self) -> &mut u8;

    // ------------------------------------------------------- unit reads
    /// Unit type and GUID (combat-list identity).
    fn ident(&self, u: Self::Unit) -> (UnitType, u32);
    /// Unit mode.
    fn mode(&self, u: Self::Unit) -> i32;
    /// The unit is in a moving mode.
    fn moving_mode(&self, u: Self::Unit) -> bool;
    /// Monster type flags matching `mask` (2 superunique, 4 champion, 8 unique).
    fn monster_flag(&self, u: Self::Unit, mask: u32) -> bool;
    /// The unit is a boss.
    fn is_boss(&self, u: Self::Unit) -> bool;
    /// The unit is a hireling.
    fn is_hireling(&self, u: Self::Unit) -> bool;
    /// The unit is a demon.
    fn is_demon(&self, u: Self::Unit) -> bool;
    /// The unit is undead.
    fn is_undead(&self, u: Self::Unit) -> bool;
    /// The unit is a prime evil.
    fn is_prime_evil(&self, u: Self::Unit) -> bool;
    /// Unit flag 0x80000000 is set (revived).
    fn is_revived(&self, u: Self::Unit) -> bool;
    /// The unit's alignment (2 = good).
    fn alignment(&self, u: Self::Unit) -> i32;
    /// The attacker is hostile to the defender.
    fn hostile(&self, a: Self::Unit, d: Self::Unit) -> bool;
    /// Melee range of a unit.
    fn melee_range(&self, u: Self::Unit) -> i32;
    /// The defender is within `range` of the attacker.
    fn in_melee_range(&self, a: Self::Unit, d: Self::Unit, range: i32) -> bool;
    /// A shield is equipped.
    fn has_shield(&self, u: Self::Unit) -> bool;
    /// For monster classes other than 243/310/333/359: the composit item in slot 7
    /// (`SH`) is a code other than `tch ` whose items record has type 2.
    fn composit_shield(&self, u: Self::Unit) -> bool;
    /// The unit's weapon class (13 = `ht2`).
    fn weapon_class(&self, u: Self::Unit) -> i32;
    /// The weapon hit class of a unit.
    fn weapon_hit_class(&self, u: Self::Unit) -> u32;
    /// `layer` matches `montype` in the montype nest bitmap.
    fn montype_matches(&self, layer: u16, montype: i32) -> bool;
    /// The kind of room the unit is in.
    fn room(&self, u: Self::Unit) -> RoomKind;
    /// The unit is dead.
    fn is_dead(&self, u: Self::Unit) -> bool;
    /// The monster class has animation `mode`.
    fn monster_has_mode(&self, u: Self::Unit, mode: i32) -> bool;
    /// The unit type its mode converts to (D2MOO `D2COMMON_11013_ConvertMode`), for the
    /// leech rules.
    fn converted_type(&self, u: Self::Unit) -> i32;
    /// The item has durability.
    fn item_has_durability(&self, item: Self::Item) -> bool;
    /// The player-count life bonus for `players` (monster layer).
    fn player_count_bonus(&self, players: i32) -> i32;

    // --------------------------------------------------------- effects
    /// Sets a stat's base value (life 6, mana 8, stamina 10, armorclass 31).
    fn set_stat(&mut self, u: Self::Unit, stat: u16, value: i32);
    /// Runs the unit's event functions for `event` (an `events.txt` index) with `other`
    /// as the second party.
    fn unit_event(
        &mut self,
        event: u8,
        unit: Self::Unit,
        other: Self::Unit,
        record: &mut DamageRecord,
    );
    /// Switches `state` on or off.
    fn set_state(&mut self, u: Self::Unit, state: u16, on: bool);
    /// The curse helper (owned by the skills code): state `state` for `frames`, with
    /// stat `stat` = `value`, from `owner`.
    #[allow(clippy::too_many_arguments)]
    fn curse(
        &mut self,
        target: Self::Unit,
        owner: Self::Unit,
        state: u16,
        stat: u16,
        value: i32,
        frames: i32,
        skill: i32,
        level: i32,
    );
    /// Expiry frame of the unit's stat list for `state`, `None` without one.
    fn state_list_expiry(&self, u: Self::Unit, state: u16) -> Option<i32>;
    /// Sets that list's expiry.
    fn set_state_list_expiry(&mut self, u: Self::Unit, state: u16, expiry: i32);
    /// Creates and attaches a stat list for `state` (owner, expiry).
    fn create_state_list(&mut self, u: Self::Unit, state: u16, owner: Self::Unit, expiry: i32);
    /// Sets a stat in the list of `state`.
    fn set_state_list_stat(&mut self, u: Self::Unit, state: u16, stat: u16, value: i32);
    /// Schedules timer event `ty` for the unit at `frame`.
    fn schedule_timer(&mut self, u: Self::Unit, ty: u8, frame: i32);
    /// Cancels the unit's timers of event type `ty`.
    fn cancel_timers(&mut self, u: Self::Unit, ty: u8);
    /// Puts overlay `id` on the unit.
    fn overlay(&mut self, u: Self::Unit, id: i32);
    /// Refreshes the unit's animation rate.
    fn refresh_anim_rate(&mut self, u: Self::Unit);
    /// Remembers `a` as `d`'s last attacker.
    fn set_last_attacker(&mut self, d: Self::Unit, a: Self::Unit);
    /// Monster-layer hook run after a monster's hit.
    fn monster_hit_hook(&mut self, a: Self::Unit);
    /// Monster-layer hook run after a non-lethal hit on a monster.
    fn monster_damaged_hook(&mut self, d: Self::Unit);
    /// Switches the dual-wield weapon stat lists on or off around a roll.
    fn dual_wield_switch(&mut self, a: Self::Unit, offhand: bool, on: bool);
    /// The attacker's combat list (D2MOO `pCombat`), first = newest.
    fn combat_list(&mut self, u: Self::Unit) -> &mut Vec<CombatEntry>;
    /// The rest of `durability_hit` after a successful draw: durability − 1 and the
    /// item rules.
    fn durability_loss(&mut self, owner: Self::Unit, item: Self::Item);
    /// Thorns reflection (owned by the skills code).
    fn thorns(&mut self, a: Self::Unit, d: Self::Unit, record: &mut DamageRecord);
    /// The defender's hit reaction. Mode changes belong to the unit layer; combat only
    /// calls it.
    fn reaction(&mut self, a: Self::Unit, d: Self::Unit, record: &mut DamageRecord);
}

/// The tables combat reads, as `d2_data` typed records.
#[derive(Debug, Clone)]
pub struct CombatTables {
    /// `charstats` rows (player classes).
    pub charstats: Vec<Charstats>,
    /// `difficultylevels` rows.
    pub difficultylevels: Vec<Difficultylevels>,
    /// `monstats` rows (monster classes).
    pub monstats: Vec<Monstats>,
    /// `monstats2` rows.
    pub monstats2: Vec<Monstats2>,
    /// `hitclass` codes by row (the get-hit divisor).
    pub hitclass: Vec<[u8; 4]>,
}

impl CombatTables {
    /// From a loaded `.bin` set.
    pub fn from_bin(set: &BinSet) -> Result<Self, crate::skills::TablesError> {
        use crate::skills::TablesError;
        let table = |n: &'static str| set.table(n).ok_or(TablesError::Missing(n));
        Ok(Self {
            charstats: decode_all(table("charstats")?)?,
            difficultylevels: decode_all(table("difficultylevels")?)?,
            monstats: decode_all(table("monstats")?)?,
            monstats2: decode_all(table("monstats2")?)?,
            hitclass: set
                .hitclass
                .iter()
                .map(|r| [r[0], r[1], r[2], r[3]])
                .collect(),
        })
    }

    /// `charstats` row of a player class.
    pub fn charstats(&self, class: i32) -> Option<&Charstats> {
        usize::try_from(class)
            .ok()
            .and_then(|i| self.charstats.get(i))
    }

    /// `difficultylevels` row of a difficulty.
    pub fn difficulty(&self, d: usize) -> Option<&Difficultylevels> {
        self.difficultylevels.get(d)
    }

    /// `monstats` row of a monster class.
    pub fn monstats(&self, class: i32) -> Option<&Monstats> {
        usize::try_from(class)
            .ok()
            .and_then(|i| self.monstats.get(i))
    }

    /// `monstats2` row of a monster class (via `MonStatsEx`).
    pub fn monstats2(&self, class: i32) -> Option<&Monstats2> {
        self.monstats(class)
            .and_then(|m| self.monstats2.get(usize::from(m.monstatsex)))
    }
}

#[cfg(test)]
mod cov_tests;
#[cfg(test)]
mod damage_tests;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod hit_gap_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod tests;
