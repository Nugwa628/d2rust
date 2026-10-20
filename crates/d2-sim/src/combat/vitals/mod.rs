//! Vitals: player creation values, spending stat points and the stat reset, level-up,
//! the experience table lookups, the experience-on-kill level factor, the gain,
//! distribution, add, death penalties and corpse experience ([`experience`]) and the
//! client vitals sync ([`sync`]).
//!
//! Nothing here is random.
//!
//! Stat reads and writes go through the [`VitalsUnits`] seam (provider: the unit stat
//! getters and setters); table data comes from `d2_data` typed records
//! ([`VitalsTables`]).

/// Experience on a kill and on death: gain, distribution, penalties, corpse experience.
pub mod experience;
#[cfg(test)]
mod experience_tests;
/// The client vitals sync: life / mana / experience packets and their prediction.
pub mod sync;
#[cfg(test)]
mod tests;

use super::pct;
use crate::rules::Ruleset;
use crate::units::UnitType;
use d2_data::bin::BinSet;
use d2_data::tables::{decode_all, Charstats, Experience};

/// Stat ids of the vitals (rows of the `itemstatcost` table).
pub mod stat {
    pub use crate::stats::stat::{
        DEXTERITY, ENERGY, HITPOINTS, LEVEL, MANA, MAXHP, MAXMANA, MAXSTAMINA, STAMINA, STRENGTH,
        VITALITY,
    };
    /// Unspent stat points.
    pub const STATPTS: u16 = 4;
    /// Unspent skill points.
    pub const NEWSKILLS: u16 = 5;
    /// Experience.
    pub const EXPERIENCE: u16 = 13;
    /// Attack rating.
    pub const TOHIT: u16 = 19;
    /// Block chance.
    pub const TOBLOCK: u16 = 20;
    /// The experience at the previous level threshold shown in the UI (`lastexp`).
    pub const LASTEXP: u16 = 29;
    /// The experience needed for the next level (`nextexp`).
    pub const NEXTEXP: u16 = 30;
    /// Velocity percent (movement speed).
    pub const VELOCITYPERCENT: u16 = 67;
    /// Attack rate (animation speed of attacks).
    pub const ATTACKRATE: u16 = 68;
    /// Animation rate of other modes.
    pub const OTHER_ANIMRATE: u16 = 69;
}

/// The stat reads, writes and notifications the vitals need (a seam).
/// Provider: the unit stat getters and setters, and `d2-server` for the client
/// notifications.
pub trait VitalsUnits {
    /// Handle for a unit in the host's world.
    type Unit: Copy + Eq + std::fmt::Debug;
    /// The unit's type.
    fn unit_type(&self, u: Self::Unit) -> UnitType;
    /// The unit's class id.
    fn class_id(&self, u: Self::Unit) -> i32;
    /// A stat's base value.
    fn base_stat(&self, u: Self::Unit, stat: u16) -> i32;
    /// A stat's full value (base plus modifiers).
    fn stat(&self, u: Self::Unit, stat: u16) -> i32;
    /// Sets a stat's base value.
    fn set_base_stat(&mut self, u: Self::Unit, stat: u16, v: i32);
    /// Adds `v` to a stat's base value.
    fn add_base_stat(&mut self, u: Self::Unit, stat: u16, v: i32);
    /// Maximum life.
    fn max_life(&self, u: Self::Unit) -> i32;
    /// Maximum mana.
    fn max_mana(&self, u: Self::Unit) -> i32;
    /// Maximum stamina.
    fn max_stamina(&self, u: Self::Unit) -> i32;
    /// The refresh after a strength / dexterity change.
    fn refresh(&mut self, u: Self::Unit);
    /// The notifications of a level-up, in this fixed order: the party roster, the
    /// level-up sound, further client updates, then the host callback.
    fn level_up_notify(&mut self, u: Self::Unit);
    /// Unit event 12 (`levelup`).
    fn level_up_event(&mut self, u: Self::Unit);
    /// The rules the game runs under: a mod game's experience table
    /// length comes from them. Default: D2RUST's own.
    fn rules(&self) -> &Ruleset {
        &crate::rules::ORIGINAL
    }
}

/// The tables the vitals read.
#[derive(Debug, Clone)]
pub struct VitalsTables {
    /// The `charstats` rows, one per player class.
    pub charstats: Vec<Charstats>,
    /// `experience.txt` rows: row 0 `MaxLvl`, row `L + 1` level `L`.
    pub experience: Vec<Experience>,
}

impl VitalsTables {
    /// From a loaded `.bin` set.
    pub fn from_bin(set: &BinSet) -> Result<Self, crate::skills::TablesError> {
        use crate::skills::TablesError;
        let table = |n: &'static str| set.table(n).ok_or(TablesError::Missing(n));
        Ok(Self {
            charstats: decode_all(table("charstats")?)?,
            experience: decode_all(table("experience")?)?,
        })
    }

    /// `charstats` row of a player class.
    pub fn charstats(&self, class: i32) -> Option<&Charstats> {
        usize::try_from(class)
            .ok()
            .and_then(|i| self.charstats.get(i))
    }

    /// Column `class` of experience row `row`. Classes outside 0–6 use class 0. Panics
    /// past the table.
    fn exp(&self, class: i32, row: usize) -> u32 {
        let r = self
            .experience
            .get(row)
            .expect("experience row past the table");
        match class {
            1 => r.sorceress,
            2 => r.necromancer,
            3 => r.paladin,
            4 => r.barbarian,
            5 => r.druid,
            6 => r.assassin,
            _ => r.amazon,
        }
    }

    /// The maximum level of `class`: row 0.
    pub fn max_level(&self, class: i32) -> u32 {
        self.exp(class, 0)
    }

    /// The experience threshold of level `L` for `class`: row `L + 1`, the experience
    /// needed to reach level `L + 1`.
    pub fn threshold(&self, class: i32, level: u32) -> u32 {
        self.exp(class, level as usize + 1)
    }

    /// The level that `exp` (unsigned) reaches for `class`.
    pub fn level_from_exp(&self, class: i32, exp: u32) -> u32 {
        let max = self.max_level(class);
        let mut i = 0;
        while i < max && exp >= self.exp(class, i as usize + 1) {
            i += 1;
        }
        i
    }

    /// The highest level of `class` under rules `r`: a mod game's
    /// [`crate::rules::FormulaHooks::max_level`] first, else the table's.
    pub fn max_level_in(&self, r: &Ruleset, class: i32) -> u32 {
        r.formulas()
            .and_then(|f| f.max_level(class))
            .unwrap_or_else(|| self.max_level(class))
    }

    /// The experience to reach level `level + 1` under rules `r`: a mod
    /// game's [`crate::rules::FormulaHooks::exp_threshold`] first, else
    /// the table's.
    pub fn threshold_in(&self, r: &Ruleset, class: i32, level: u32) -> u32 {
        r.formulas()
            .and_then(|f| f.exp_threshold(class, level))
            .unwrap_or_else(|| self.threshold(class, level))
    }

    /// The level `exp` reaches under rules `r` (as
    /// [`VitalsTables::level_from_exp`], with the mod game's table length
    /// and thresholds).
    pub fn level_from_exp_in(&self, r: &Ruleset, class: i32, exp: u32) -> u32 {
        if r.formulas().is_none() {
            return self.level_from_exp(class, exp);
        }
        let max = self.max_level_in(r, class);
        let mut i = 0;
        while i < max && exp >= self.threshold_in(r, class, i) {
            i += 1;
        }
        i
    }
}

/// A charstats quarter-point column as 1/256 units: `v << 6`.
fn q(v: i32) -> i32 {
    v.wrapping_shl(6)
}

// ---------------------------------------------------------------- creation

/// The target level of each act, used to start a character in a later act.
pub const ACT_TARGET_LEVEL: [u32; 5] = [1, 15, 20, 26, 32];

/// Sets the starting stats of a new player from its `charstats` row, then raises the
/// experience to the target level of `act`.
pub fn init_player_stats<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit, act: u32) {
    let class = w.class_id(u);
    let Some(c) = t.charstats(class) else {
        return;
    };
    let act = act.min(4);
    let (str_, int, dex, vit) = (
        i32::from(c.str),
        i32::from(c.int),
        i32::from(c.dex),
        i32::from(c.vit),
    );
    let life = (vit + i32::from(c.hpadd)) << 8;
    let stamina = i32::from(c.stamina) << 8;
    for (s, v) in [
        (stat::STRENGTH, str_),
        (stat::ENERGY, int),
        (stat::DEXTERITY, dex),
        (stat::VITALITY, vit),
        (stat::TOHIT, 0),
        (stat::TOBLOCK, 0),
        (stat::HITPOINTS, life),
        (stat::MAXHP, life),
        (stat::MANA, int << 8),
        (stat::MAXMANA, int << 8),
        (stat::STAMINA, stamina),
        (stat::MAXSTAMINA, stamina),
        (stat::LEVEL, 1),
        (stat::NEXTEXP, t.threshold_in(w.rules(), class, 1) as i32),
        (stat::ATTACKRATE, 100),
        (stat::VELOCITYPERCENT, 100),
        (stat::OTHER_ANIMRATE, 100),
    ] {
        // The table's row order, left column first.
        w.set_base_stat(u, s, v);
    }
    if act > 0 {
        let idx = if (act as usize) < ACT_TARGET_LEVEL.len() {
            act as usize
        } else {
            0
        };
        set_experience_for_target_level(w, t, u, ACT_TARGET_LEVEL[idx]);
    }
}

/// Raises the experience to the target level (D2MOO
/// `SUNITDMG_SetExperienceForTargetLevel`): only when `target` > the base level, adds
/// `threshold(class, target) − experience` if positive, through `add_experience`.
/// `threshold(class, target)` is row target + 1: the player ends at level target + 1.
pub fn set_experience_for_target_level<W: VitalsUnits>(
    w: &mut W,
    t: &VitalsTables,
    u: W::Unit,
    target: u32,
) {
    if w.base_stat(u, stat::LEVEL) >= target as i32 {
        return;
    }
    let class = w.class_id(u);
    let need = t.threshold_in(w.rules(), class, target);
    let have = w.base_stat(u, stat::EXPERIENCE) as u32;
    if need > have {
        add_experience(w, t, u, need - have);
    }
}

// ---------------------------------------------------------------- stat points

/// Handler of client message 0x3A (spend stat points): returns 0, 2 (a spend failed)
/// or 3 (bad message). Byte 1 is the stat, byte 2 is `count − 1`.
pub fn handle_add_stat_point<W: VitalsUnits>(
    w: &mut W,
    t: &VitalsTables,
    u: W::Unit,
    m: &[u8],
) -> i32 {
    if m.len() != 3 {
        return 3;
    }
    let (s, n1) = (m[1], m[2]);
    if s > 15 || n1 > 99 {
        return 3;
    }
    for _ in 0..=n1 {
        if !spend(w, t, u, s) {
            return 2;
        }
    }
    0
}

/// Spends one stat point on stat `s`; false = fail.
pub fn spend<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit, s: u8) -> bool {
    if w.stat(u, stat::STATPTS) == 0 {
        return false;
    }
    match u16::from(s) {
        s @ (stat::STRENGTH | stat::DEXTERITY) => {
            w.add_base_stat(u, stat::STATPTS, -1);
            w.add_base_stat(u, s, 1);
            w.refresh(u);
            true
        }
        stat::ENERGY => {
            gain_energy(w, t, u, 1);
            true
        }
        stat::VITALITY => {
            gain_vitality(w, t, u, 1);
            true
        }
        _ => false,
    }
}

/// Adds `n` points of energy, paid from the stat points, and the mana they give.
// The `n > 0` test guards the current-mana add only.
pub fn gain_energy<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit, n: i32) {
    let per = t
        .charstats(w.class_id(u))
        .map_or(0, |c| i32::from(c.manapermagic));
    w.add_base_stat(u, stat::STATPTS, n.wrapping_neg());
    w.add_base_stat(u, stat::ENERGY, n);
    let d = q(per.wrapping_mul(n));
    if n > 0 {
        w.add_base_stat(u, stat::MANA, d);
    }
    w.add_base_stat(u, stat::MAXMANA, d);
    if w.stat(u, stat::MANA) > w.stat(u, stat::MAXMANA) {
        let m = w.stat(u, stat::MAXMANA);
        w.set_base_stat(u, stat::MANA, m);
    }
}

/// Adds `n` points of vitality, paid from the stat points, and the life and stamina they
/// give.
pub fn gain_vitality<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit, n: i32) {
    let (life, stam) = t.charstats(w.class_id(u)).map_or((0, 0), |c| {
        (
            i32::from(c.lifepervitality),
            i32::from(c.staminapervitality),
        )
    });
    w.add_base_stat(u, stat::STATPTS, n.wrapping_neg());
    w.add_base_stat(u, stat::VITALITY, n);
    let dl = q(life.wrapping_mul(n));
    w.add_base_stat(u, stat::MAXHP, dl);
    if n > 0 {
        w.add_base_stat(u, stat::HITPOINTS, dl);
    }
    if w.stat(u, stat::HITPOINTS) > w.stat(u, stat::MAXHP) {
        let m = w.stat(u, stat::MAXHP);
        w.set_base_stat(u, stat::HITPOINTS, m);
    }
    let ds = q(stam.wrapping_mul(n));
    w.add_base_stat(u, stat::MAXSTAMINA, ds);
    if n > 0 {
        w.add_base_stat(u, stat::STAMINA, ds);
    }
    if w.stat(u, stat::STAMINA) > w.stat(u, stat::MAXSTAMINA) {
        let m = w.stat(u, stat::MAXSTAMINA);
        w.set_base_stat(u, stat::STAMINA, m);
    }
}

/// Resets strength, energy, dexterity and vitality to the class start values, returning
/// the points to the stat points. Players only.
pub fn reset_stats<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit) {
    if w.unit_type(u) != UnitType::Player {
        return;
    }
    let Some(c) = t.charstats(w.class_id(u)) else {
        return;
    };
    let starts = [
        (stat::STRENGTH, i32::from(c.str)),
        (stat::ENERGY, i32::from(c.int)),
        (stat::DEXTERITY, i32::from(c.dex)),
        (stat::VITALITY, i32::from(c.vit)),
    ];
    for (s, start) in starts {
        let d = start.wrapping_sub(w.base_stat(u, s));
        match s {
            stat::ENERGY => gain_energy(w, t, u, d),
            stat::VITALITY => gain_vitality(w, t, u, d),
            _ => {
                if d != 0 {
                    w.add_base_stat(u, stat::STATPTS, d.wrapping_neg());
                    w.add_base_stat(u, s, d);
                    w.refresh(u);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- level-up

/// Recomputes the level from the experience and grants the per-level gains. Returns
/// `d = new − old`.
pub fn level_up<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit) -> i32 {
    let class = w.class_id(u);
    let old = w.base_stat(u, stat::LEVEL);
    let new = t.level_from_exp_in(w.rules(), class, w.base_stat(u, stat::EXPERIENCE) as u32);
    w.set_base_stat(u, stat::LEVEL, new as i32);
    let next = t.threshold_in(w.rules(), class, new);
    w.set_base_stat(u, stat::NEXTEXP, next as i32);
    let d = (new as i32).wrapping_sub(old);
    if d <= 0 {
        return d;
    }
    let (lpl, mpl, spl, statpl) = t.charstats(class).map_or((0, 0, 0, 0), |c| {
        (
            i32::from(c.lifeperlevel),
            i32::from(c.manaperlevel),
            i32::from(c.staminaperlevel),
            i32::from(c.statperlevel as i8),
        )
    });
    let maxhp = w
        .base_stat(u, stat::MAXHP)
        .wrapping_add(q(lpl.wrapping_mul(d)));
    w.set_base_stat(u, stat::MAXHP, maxhp);
    if w.stat(u, stat::HITPOINTS) > 0 {
        let m = w.max_life(u);
        w.set_base_stat(u, stat::HITPOINTS, m);
    }
    let maxmana = w
        .base_stat(u, stat::MAXMANA)
        .wrapping_add(q(mpl.wrapping_mul(d)));
    w.set_base_stat(u, stat::MAXMANA, maxmana);
    let m = w.max_mana(u);
    w.set_base_stat(u, stat::MANA, m);
    let maxst = w
        .base_stat(u, stat::MAXSTAMINA)
        .wrapping_add(q(spl.wrapping_mul(d)));
    w.set_base_stat(u, stat::MAXSTAMINA, maxst);
    let m = w.max_stamina(u);
    w.set_base_stat(u, stat::STAMINA, m);
    w.add_base_stat(u, stat::STATPTS, statpl.wrapping_mul(d));
    w.add_base_stat(u, stat::NEWSKILLS, d);
    w.level_up_notify(u);
    d
}

// ---------------------------------------------------------------- experience

/// Level factor table `T1` (256ths): defender at or below the attacker's level.
pub const LEVEL_FACTOR_LOWER: [i32; 11] = [256, 256, 256, 256, 256, 256, 207, 159, 110, 61, 13];
/// Level factor table `T2` (256ths): defender above the attacker's level.
pub const LEVEL_FACTOR_HIGHER: [i32; 11] = [256, 256, 256, 256, 256, 256, 225, 174, 92, 38, 5];

/// Scales kill experience `exp` by the level difference between attacker `alvl` and
/// defender `dlvl`; signed.
// The operand roles of `pct(exp, alvl, dlvl)` in the `dlvl > alvl >= 25` branch are
// unconfirmed.
pub fn level_factor(exp: i32, alvl: i32, dlvl: i32) -> i32 {
    let f = if dlvl <= alvl {
        LEVEL_FACTOR_LOWER[alvl.wrapping_sub(dlvl).min(10) as usize]
    } else {
        if alvl >= 25 && dlvl > 0 {
            return pct(exp, alvl, dlvl);
        }
        LEVEL_FACTOR_HIGHER[dlvl.wrapping_sub(alvl).min(10) as usize]
    };
    if f == 256 {
        exp
    } else {
        pct(exp, f, 256)
    }
}

/// `add_experience_at` at the gainer's base level, for callers without their own level
/// argument.
pub fn add_experience<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit, gain: u32) {
    let l0 = w.base_stat(u, stat::LEVEL);
    add_experience_at(w, t, u, l0, gain as i32);
}

/// Adds `g` experience to a player whose level was `l0`: new:= old + g (32-bit), capped
/// at `threshold(class, max_level − 1)` (unsigned); `lastexp` (29):= new − old;
/// experience (13):= new; a level from the new experience ≠ `l0` → `level_up` and
/// unit event 12. Other unit types are ignored.
pub fn add_experience_at<W: VitalsUnits>(w: &mut W, t: &VitalsTables, u: W::Unit, l0: i32, g: i32) {
    if w.unit_type(u) != UnitType::Player {
        return;
    }
    let class = w.class_id(u);
    let old = w.base_stat(u, stat::EXPERIENCE) as u32;
    let r = w.rules();
    let cap = t.threshold_in(r, class, t.max_level_in(r, class).saturating_sub(1));
    let new = old.wrapping_add(g as u32).min(cap);
    w.set_base_stat(u, stat::LASTEXP, new.wrapping_sub(old) as i32);
    w.set_base_stat(u, stat::EXPERIENCE, new as i32);
    if t.level_from_exp_in(w.rules(), class, new) as i32 != l0 {
        level_up(w, t, u);
        w.level_up_event(u);
    }
}
