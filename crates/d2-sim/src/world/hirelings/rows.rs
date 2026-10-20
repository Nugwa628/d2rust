//! The `hireling.txt` row model, the lookups, the offer
//! values and price, the experience threshold and the
//! resurrect cost.

use d2_data::bin::BinTable;
use d2_data::tables::{Hireling, Record};

use super::HirelingError;
use crate::rng::Seed;

/// Expansion rows carry `Version` 100, classic rows 0.
pub const VERSION_EXPANSION: u16 = 100;
/// The resurrect cost cap.
pub const RESURRECT_CAP: u32 = 50_000;
/// Skill slots per row.
pub const SKILL_SLOTS: usize = 6;

/// One skill slot of a row (+0x78 skill, +0xC0 mode, +0xC6 level, +0xCC
/// level per level; bytes read signed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowSkill {
    pub skill: i32,
    pub mode: i8,
    pub level: i8,
    pub lvl_per_lvl: i8,
}

/// The `hireling` record fields the hireling rules read. Numeric
/// columns are held as the i32 the base-game code reads them as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HirelingRow {
    /// +0x00.
    pub version: u16,
    /// +0x04 (subtype).
    pub id: u32,
    /// +0x08 (monstats class).
    pub class: u32,
    /// +0x0C (1-based).
    pub act: u32,
    /// +0x10 (1-based).
    pub difficulty: u32,
    /// +0x14 (NPC monstats class).
    pub seller: u32,
    pub gold: i32,
    /// +0x1C (bracket start).
    pub level: i32,
    pub exp_lvl: i32,
    pub hp: i32,
    pub hp_lvl: i32,
    pub defense: i32,
    pub def_lvl: i32,
    pub str_: i32,
    pub str_lvl: i32,
    pub dex: i32,
    pub dex_lvl: i32,
    pub ar: i32,
    pub ar_lvl: i32,
    pub share: i32,
    pub dmg_min: i32,
    pub dmg_max: i32,
    pub dmg_lvl: i32,
    pub resist: i32,
    pub resist_lvl: i32,
    pub skills: [RowSkill; SKILL_SLOTS],
    /// `DefaultChance` (+0x64), `Chance1`–`6` (+0x90) and
    /// `ChancePerLvl1`–`6` (+0xA8): the Hireable AI's skill pick.
    pub default_chance: i32,
    pub chance: [i32; SKILL_SLOTS],
    pub chance_per_lvl: [i32; SKILL_SLOTS],
    /// u8 +0xD2.
    pub hire_desc: u8,
    /// +0x114 / +0x116 (string ids).
    pub name_first: u16,
    pub name_last: u16,
}

impl HirelingRow {
    /// The columns the Hireable AI's skill pick reads.
    pub fn ai_row(&self) -> crate::monsters::ai::HireRow {
        crate::monsters::ai::HireRow {
            level: self.level,
            default_chance: self.default_chance,
            skill: self.skills.map(|s| s.skill),
            chance: self.chance,
            chance_per_lvl: self.chance_per_lvl,
            mode: self.skills.map(|s| s.mode as u8),
        }
    }
}

/// The hireling tables of a game: the rows in table order and the two
/// first-row-of-`Id` tables.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HirelingRows {
    pub rows: Vec<HirelingRow>,
    /// [classic, expansion]: 256 entries each, −1 for none.
    first: [Vec<i32>; 2],
}

/// The offer of a hire slot (output words 0–14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    /// Index of the candidate row in [`HirelingRows::rows`].
    pub row: usize,
    /// Word 0.
    pub id: u32,
    /// Word 1: L.
    pub level: i32,
    pub life: i32,
    pub strength: i32,
    pub dexterity: i32,
    pub price: i32,
    pub experience: i32,
    pub defense: i32,
    pub min_damage: i32,
    pub max_damage: i32,
    pub share: i32,
    pub resist: i32,
    pub hire_desc: u8,
}

impl HirelingRows {
    /// Builds the first-row tables: for each row in
    /// order with `Id` < 256, the table of its version keeps the first row.
    pub fn new(rows: Vec<HirelingRow>) -> Self {
        let mut first = [vec![-1i32; 256], vec![-1i32; 256]];
        for (r, row) in rows.iter().enumerate() {
            if row.id < 256 {
                let t = &mut first[usize::from(row.version >= VERSION_EXPANSION)];
                if t[row.id as usize] < 0 {
                    t[row.id as usize] = r as i32;
                }
            }
        }
        Self { rows, first }
    }

    /// Decodes the `hireling` table (fixed up: name ids at +0x114 /
    /// +0x116).
    pub fn from_table(t: &BinTable) -> Result<Self, HirelingError> {
        if t.name != Hireling::TABLE || t.record_size != Hireling::SIZE {
            return Err(HirelingError::Table(format!(
                "{} ({}-byte records) is not hireling",
                t.name, t.record_size
            )));
        }
        let rows = t
            .iter()
            .map(|r| {
                let h = Hireling::decode(r);
                let i8_at = |o: usize| r[o] as i8;
                let i32_at = |o: usize| i32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]]);
                let skill = |k: usize| RowSkill {
                    skill: i32::from_le_bytes([
                        r[0x78 + 4 * k],
                        r[0x79 + 4 * k],
                        r[0x7A + 4 * k],
                        r[0x7B + 4 * k],
                    ]),
                    mode: i8_at(0xC0 + k),
                    level: i8_at(0xC6 + k),
                    lvl_per_lvl: i8_at(0xCC + k),
                };
                HirelingRow {
                    version: h.version,
                    id: h.id,
                    class: h.class,
                    act: h.act,
                    difficulty: h.difficulty,
                    seller: h.seller,
                    gold: h.gold as i32,
                    level: h.level as i32,
                    exp_lvl: h.exp_lvl as i32,
                    hp: h.hp as i32,
                    hp_lvl: h.hp_lvl as i32,
                    defense: h.defense as i32,
                    def_lvl: h.def_lvl as i32,
                    str_: h.str as i32,
                    str_lvl: h.str_lvl as i32,
                    dex: h.dex as i32,
                    dex_lvl: h.dex_lvl as i32,
                    ar: h.ar as i32,
                    ar_lvl: h.ar_lvl as i32,
                    share: h.share as i32,
                    dmg_min: h.dmg_min as i32,
                    dmg_max: h.dmg_max as i32,
                    dmg_lvl: h.dmg_lvl as i32,
                    resist: h.resist as i32,
                    resist_lvl: h.resist_lvl as i32,
                    skills: std::array::from_fn(skill),
                    default_chance: i32_at(0x64),
                    chance: std::array::from_fn(|k| i32_at(0x90 + 4 * k)),
                    chance_per_lvl: std::array::from_fn(|k| i32_at(0xA8 + 4 * k)),
                    hire_desc: h.hiredesc,
                    name_first: u16::from_le_bytes([r[0x114], r[0x115]]),
                    name_last: u16::from_le_bytes([r[0x116], r[0x117]]),
                }
            })
            .collect();
        Ok(Self::new(rows))
    }

    /// The candidate rows of an act and
    /// difficulty (0-based) for the game's version: the first match and
    /// every later match with the same `Level`.
    pub fn candidates(&self, expansion: bool, act0: u32, diff0: u32) -> Vec<usize> {
        let version = if expansion { VERSION_EXPANSION } else { 0 };
        let (act, diff) = (act0.wrapping_add(1), diff0.wrapping_add(1));
        let same = |r: &HirelingRow| r.act == act && r.difficulty == diff && r.version == version;
        let Some(first) = self.rows.iter().position(same) else {
            return Vec::new();
        };
        let level = self.rows[first].level;
        std::iter::once(first)
            .chain(
                (first + 1..self.rows.len())
                    .filter(|&i| same(&self.rows[i]) && self.rows[i].level == level),
            )
            .collect()
    }

    /// The row of `id` at level `l`: the last
    /// bracket with `Level` ≤ l, the first bracket when l is below all.
    pub fn row_at(&self, expansion: bool, id: u32, l: i32) -> Option<usize> {
        if id > 255 {
            return None;
        }
        let version = if expansion { VERSION_EXPANSION } else { 0 };
        let start = self.first[usize::from(expansion)][id as usize];
        if start < 0 {
            return None;
        }
        let mut result: Option<usize> = None;
        for (i, r) in self.rows.iter().enumerate().skip(start as usize) {
            if r.id > id {
                break;
            }
            if r.id == id && r.version == version {
                if result.is_some() && l < r.level {
                    return result;
                }
                result = Some(i);
            }
        }
        result
    }

    /// The act of a hireling (expansion, class, name): `Act − 1` of
    /// the first row of the game's version (100 expansion, else 0) whose
    /// `Class` equals `class`, else of the first such row
    /// whose `NameFirst` ≤ `name` ≤ `NameLast` (unsigned);
    /// neither → 0. Every base-game caller passes class 0 ([`Self::act_of_name`]).
    pub fn act_of(&self, expansion: bool, class: u32, name: u16) -> u32 {
        let version = if expansion { 100 } else { 0 };
        let same = |r: &&HirelingRow| r.version == version;
        self.rows
            .iter()
            .filter(same)
            .find(|r| r.class == class)
            .or_else(|| {
                self.rows
                    .iter()
                    .filter(same)
                    .find(|r| r.name_first <= name && name <= r.name_last)
            })
            .map_or(0, |r| r.act.wrapping_sub(1))
    }

    /// [`Self::act_of`] with class 0, as every base-game caller calls it.
    pub fn act_of_name(&self, expansion: bool, name: u16) -> u32 {
        self.act_of(expansion, 0, name)
    }

    /// The offer of a slot seed. `None`: no candidate.
    pub fn offer(
        &self,
        expansion: bool,
        player_level: i32,
        seed: u32,
        act0: u32,
        diff0: u32,
    ) -> Option<Offer> {
        let mut local = Seed::init_low(seed);
        let candidates = self.candidates(expansion, act0, diff0);
        if candidates.is_empty() {
            return None;
        }
        let idx = candidates[local.roll(candidates.len() as i32) as usize];
        let lo = local.step();
        let l = ((lo % 5) as i32)
            .wrapping_add(player_level)
            .wrapping_sub(5)
            .max(2);
        let r = &self.rows[idx];
        let d = l.wrapping_sub(r.level);
        let per = |base: i32, lvl: i32| base.wrapping_add(lvl.wrapping_mul(d) >> 3);
        let price = r
            .gold
            .wrapping_mul(100i32.wrapping_add(15i32.wrapping_mul(d)))
            / 100;
        Some(Offer {
            row: idx,
            id: r.id,
            level: l,
            life: r.hp.wrapping_add(r.hp_lvl.wrapping_mul(d)).max(40),
            strength: per(r.str_, r.str_lvl).max(10),
            dexterity: per(r.dex, r.dex_lvl).max(10),
            price: if price < r.gold { r.gold } else { price },
            experience: threshold(r.exp_lvl, l).max(0),
            defense: r.defense.wrapping_add(r.def_lvl.wrapping_mul(d)).max(0),
            min_damage: per(r.dmg_min, r.dmg_lvl).max(0),
            max_damage: per(r.dmg_max, r.dmg_lvl).max(1),
            share: r.share.max(0),
            resist: r.resist.max(0),
            hire_desc: r.hire_desc,
        })
    }
}

/// The experience threshold: threshold(n) = (n + 1)·n·n·Exp/Lvl, 32-bit
/// wrap (offer word 6).
pub fn threshold(exp_lvl: i32, n: i32) -> i32 {
    n.wrapping_add(1)
        .wrapping_mul(exp_lvl)
        .wrapping_mul(n)
        .wrapping_mul(n)
}

/// The resurrect cost at level L: ((L·L)/2)·15, signed division; above 50000
/// (unsigned) → 50000.
pub fn resurrect_cost(level: i32) -> u32 {
    let cost = (level.wrapping_mul(level) / 2).wrapping_mul(15) as u32;
    cost.min(RESURRECT_CAP)
}
