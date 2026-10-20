//! Skills: level, special values, skill damage, mana cost, to-hit and
//! learning ([`levels`]); the special-value tables ([`special`], from
//! `skillcalc.tsv` / `misscalc.tsv`); the formula evaluator ([`calc`]).
//!
//! Status: implemented, unverified (the rules are a draft; their
//! confirmations are still queued).
//!
//! Unit state (stats, states, skill lists, items, seeds) is read through
//! the [`SkillUnits`] seam, provided by the units/stats implementation.
//! Table data
//! comes from `d2_data` typed records ([`SkillTables`]).

pub mod calc;
pub mod levels;
pub mod list;
pub mod sequences;
pub mod special;
pub mod stat_cb;
pub mod use_;

use crate::rng::Seed;
use crate::units::UnitType;
use d2_data::bin::BinSet;
use d2_data::schema::CalcBuffer;
use d2_data::tables::{decode_all, Missiles, Skilldesc, Skills, WrongTable};

pub use levels::*;

/// The skill-level cap of the base game: entry 0 of the max-level table
/// `[0x0096C8A8]`.
// TODO: confirm the table is filled from `experience.txt`
// `MaxLvl`; until then the base-game value is passed in by the caller.
pub const LEVEL_CAP_114D: i32 = 99;

/// One skill-list entry (D2MOO `D2SkillStrc`, the base game offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SkillEntry {
    /// Skill id.
    pub skill: i32,
    /// +0x28 base level (hard points).
    pub base: i32,
    /// +0x2C level bonus.
    pub level_bonus: i32,
    /// +0x34 owner GUID; −1 = native skill, else the granting item.
    pub owner_guid: i32,
    /// +0x38 charges.
    pub charges: i32,
    /// +0x3C has-charges flag.
    pub has_charges: bool,
}

impl SkillEntry {
    /// Owner GUID −1: the unit's own skill.
    pub fn is_native(&self) -> bool {
        self.owner_guid == -1
    }
}

/// The unit reads skills and combat need (a seam). Provider: the
/// units/stats implementation in `d2-sim` (stat getters,
/// unit fields, the items for item queries). Every
/// method is a plain read of unit state.
pub trait SkillUnits {
    /// A unit handle.
    type Unit: Copy + Eq + std::fmt::Debug;
    /// An item handle.
    type Item: Copy + Eq + std::fmt::Debug;

    /// Unit type (unit +0x00).
    fn unit_type(&self, u: Self::Unit) -> UnitType;
    /// Class id (unit +0x04): player class, monster class (`monstats`
    /// row), missile id.
    fn class_id(&self, u: Self::Unit) -> i32;
    /// Unit getter.
    fn stat(&self, u: Self::Unit, stat: u16, layer: u16) -> i32;
    /// Item/skill getter.
    fn item_stat(&self, u: Self::Unit, stat: u16, layer: u16) -> i32;
    /// Base getter.
    fn base_stat(&self, u: Self::Unit, stat: u16, layer: u16) -> i32;
    /// The formula function `stat(s, mode)`;
    /// `s` is already in range.
    fn formula_stat(&self, u: Self::Unit, stat: u16, mode: i32) -> i32;
    /// The unit's entries of `stat` as (layer, value), in
    /// list order, at most `max`.
    fn stat_entries(&self, u: Self::Unit, stat: u16, max: usize) -> Vec<(u16, i32)>;
    /// Whether the unit has `state` on.
    fn has_state(&self, u: Self::Unit, state: u16) -> bool;
    /// Value of `stat` in the stat list of `state` (then a
    /// list getter); `None` without that list.
    fn state_stat(&self, u: Self::Unit, state: u16, stat: u16) -> Option<i32>;
    /// The unit's RNG seed (unit +0x20).
    fn seed(&mut self, u: Self::Unit) -> &mut Seed;
    /// The unit's skill list (unit +0xA8), in list order.
    fn skill_list(&self, u: Self::Unit) -> Vec<SkillEntry>;
    /// The entry of the skill in use.
    fn used_skill(&self, u: Self::Unit) -> Option<SkillEntry>;
    /// The current weapon (combat).
    fn current_weapon(&self, u: Self::Unit) -> Option<Self::Item>;
    /// The weapon used for skill `SrcDam`.
    fn weapon(&self, u: Self::Unit) -> Option<Self::Item>;
    /// The item in body location `loc` (1 head … 10 gloves), if any.
    fn item_at(&self, u: Self::Unit, loc: u8) -> Option<Self::Item>;
    /// The item is of item type `itype`
    /// (nesting included).
    fn item_is(&self, item: Self::Item, itype: i32) -> bool;
    /// Item type `itype` is-a `parent` (`itemtypes` nesting).
    fn itype_is(&self, itype: i32, parent: i32) -> bool;
    /// Wield type (2 = two-handed with secondary damage).
    fn wield_type(&self, item: Self::Item) -> i32;
    /// The item damage read: the weapon's min /
    /// max damage for skill `SrcDam`.
    fn item_damage(&self, item: Self::Item, max: bool) -> i32;
    /// Weapon `StrBonus` / `DexBonus` (i16).
    fn str_dex_bonus(&self, item: Self::Item) -> (i32, i32);
    /// (throwing predicate; items).
    fn item_flag_throw(&self, item: Self::Item) -> bool;
    /// Missile unit data +0x0C (u16): its stored level.
    fn missile_level(&self, u: Self::Unit) -> i32;
    /// The rules the game runs under; the
    /// skills and missiles formulas read a mod game's functions from
    /// them. Default: D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }
}

/// The tables skills read, as `d2_data` typed records, plus the two code
/// buffers the special values evaluate.
#[derive(Debug, Clone)]
pub struct SkillTables {
    pub skills: Vec<Skills>,
    pub skilldesc: Vec<Skilldesc>,
    pub missiles: Vec<Missiles>,
    /// `skillscode` buffer (skills formulas).
    pub skills_code: Vec<u8>,
    /// `misscode` buffer (missiles formulas).
    pub miss_code: Vec<u8>,
    /// Cap ([`LEVEL_CAP_114D`]).
    pub level_cap: i32,
    /// `itemstatcost` record count (bound of the formula `stat`).
    pub stat_count: i32,
}

/// A table the skills tables need is missing or malformed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TablesError {
    #[error("table {0} missing")]
    Missing(&'static str),
    #[error("code buffer {0} missing")]
    MissingCode(&'static str),
    #[error(transparent)]
    Wrong(#[from] WrongTable),
}

impl SkillTables {
    /// From a loaded `.bin` set.
    pub fn from_bin(set: &BinSet, level_cap: i32) -> Result<Self, TablesError> {
        let table = |n: &'static str| set.table(n).ok_or(TablesError::Missing(n));
        let code = |b: CalcBuffer, n: &'static str| {
            set.code
                .get(&b)
                .map(|c| c.bytes.clone())
                .ok_or(TablesError::MissingCode(n))
        };
        Ok(Self {
            skills: decode_all(table("skills")?)?,
            skilldesc: decode_all(table("skilldesc")?)?,
            missiles: decode_all(table("missiles")?)?,
            skills_code: code(CalcBuffer::SkillsCode, "skillscode")?,
            miss_code: code(CalcBuffer::MissCode, "misscode")?,
            level_cap,
            stat_count: i32::try_from(table("itemstatcost")?.count).unwrap_or(i32::MAX),
        })
    }

    /// The skills record of `skill`, `None` when out of range.
    pub fn skill(&self, skill: i32) -> Option<&Skills> {
        usize::try_from(skill).ok().and_then(|i| self.skills.get(i))
    }

    /// The missiles record of `missile`, `None` when out of range.
    pub fn missile(&self, missile: i32) -> Option<&Missiles> {
        usize::try_from(missile)
            .ok()
            .and_then(|i| self.missiles.get(i))
    }

    /// The skilldesc record of a skill, `None` when missing.
    pub fn skilldesc_of(&self, s: &Skills) -> Option<&Skilldesc> {
        self.skilldesc.get(usize::from(s.skilldesc))
    }
}

#[cfg(any(test, feature = "bench-fixtures"))]
#[cfg_attr(not(test), allow(unused, dead_code))]
pub(crate) mod fake;
#[cfg(test)]
mod levels_gap_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod stat_cb_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::game_loader as tests_game;
