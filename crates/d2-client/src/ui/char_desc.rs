//! The character panel's skill damage and attack-rating values: the `skilldesc`
//! `descdam` functions (table, entries 1–24) and `descatt` functions (entries
//! 1–5), evaluated on the client model of the local player. The skills formulas
//! are the `d2-sim` evaluators (`skills::levels`) on a [`SkillUnits`] view of
//! the model whose item reads answer from the body items and the item tables
//! ([`DescUnits`]). Nothing here decides an outcome: it reads and draws.
//!
//! The progressive charges (the charge bonus and the charge damage) are read
//! from the `pgsv` states' lists of the model (three Tiger Strike charges on
//! Dragon Talon 83-100 give all 42 text lines of the panel). An item's own stat
//! list (`item_normaldamage` F1, `item_throw_*`) is read from the unit totals,
//! as the model attaches item stats to the player, and a dual-wielder's ranges
//! toggle the other hand's list off (an active Firelizard's Talons, two Short
//! Swords).

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_sim::rng::Seed;
use d2_sim::skills::levels::{
    concentration, elem_len, elem_max, elem_min, eval_desc, eval_skill, highest_entry,
    miss_elem_len, miss_elem_max, miss_elem_min, miss_phys_max, miss_phys_min, phys_max, phys_min,
    skill_level, to_hit, weapon_mastery,
};
use d2_sim::skills::{SkillEntry as SimEntry, SkillTables, SkillUnits};
use d2_sim::units::UnitType;

use super::skill_desc::{etype_color, range_text, range_text_two_line};
use super::skill_desc_more::pct;
use crate::bridge::passive::ClientSkills;
use crate::bridge::world::{ClientWorld, UnitKey};

/// The item fields the functions read, by item code (`weapons.txt`,
/// `armor.txt`, `misc.txt`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemRow {
    /// `type` / `type2` (`itemtypes` rows; 0xFFFF none).
    pub type1: u16,
    pub type2: u16,
    /// Weapon row (`weapons.txt`).
    pub weapon: bool,
    /// `2handed`.
    pub two_handed: bool,
    /// `1or2handed` (record +0x13D).
    pub one_or_two: bool,
    /// `mindam` / `maxdam` (record +0xFE / +0xFF; a shield's smite damage).
    pub mindam: u8,
    pub maxdam: u8,
    /// `StrBonus` / `DexBonus` (i16).
    pub str_bonus: i32,
    pub dex_bonus: i32,
    /// `missiletype` (weapons +0xFA; −1 none): the missile a thrown potion
    /// makes.
    pub missile_type: i16,
}

/// One `itemtypes` row: nesting and the `throwable` flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItypeRow {
    pub equiv1: u16,
    pub equiv2: u16,
    pub throwable: bool,
}

/// `states.txt` flags of the value colours: bit 0 `damblue`, 1 `damred`,
/// 2 `attblue`, 3 `attred`.
pub const DAMBLUE: u8 = 1;
pub const DAMRED: u8 = 2;
pub const ATTBLUE: u8 = 4;
pub const ATTRED: u8 = 8;

/// The tables of the damage block (empty without game files).
#[derive(Clone, Debug, Default)]
pub struct DescTables {
    pub skills: Option<Arc<SkillTables>>,
    /// `skilldesccode` buffer.
    pub desc_code: Vec<u8>,
    pub items: BTreeMap<[u8; 4], ItemRow>,
    pub itypes: Vec<ItypeRow>,
    /// Per `states` row: [`DAMBLUE`] … [`ATTRED`].
    pub state_colors: Vec<u8>,
    /// `states` rows with the `restrict` flag (the shapeshift forms).
    pub form_states: Vec<u8>,
    /// `states` rows with the `pgsv` flag, in row order: the progressive
    /// charge states of data tables +0x16C.
    pub pgsv_states: Vec<u8>,
}

impl DescTables {
    /// `itype` is-a `parent` (over the `equiv` links).
    pub fn itype_is(&self, itype: i32, parent: i32) -> bool {
        // Each row once: a blank link reads 0 here, and row 0's own links
        // would otherwise refill the stack before the real chain is walked.
        let mut todo = vec![itype];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(t) = todo.pop() {
            if t == parent {
                return true;
            }
            if !seen.insert(t) {
                continue;
            }
            if let Some(r) = usize::try_from(t).ok().and_then(|i| self.itypes.get(i)) {
                for e in [r.equiv1, r.equiv2] {
                    if e != 0xFFFF {
                        todo.push(i32::from(e));
                    }
                }
            }
        }
        false
    }

    fn throwable(&self, itype: u16) -> bool {
        self.itypes
            .get(usize::from(itype))
            .is_some_and(|r| r.throwable)
    }
}

/// A body item of the local player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BodyItem {
    key: UnitKey,
    body: u8,
    row: ItemRow,
    /// Item flag 0x4000 clear (the record's flag word and the bits the
    /// model writes): an item the inventory pass switched off has its
    /// weapon link (+0x1C) undone.
    linked: bool,
}

/// The client model with its body items, as the skills formulas read it.
pub struct DescUnits<'a> {
    inner: ClientSkills<'a>,
    t: &'a DescTables,
    owner: UnitKey,
    class: u32,
    body: Vec<BodyItem>,
    seed: Seed,
}

impl<'a> DescUnits<'a> {
    pub fn new(world: &'a ClientWorld, key: UnitKey, t: &'a DescTables) -> Self {
        use crate::bridge::items::{items, mode};
        let body = items(world)
            .into_iter()
            .filter(|i| i.mode == mode::BODY && i.owner == Some(key))
            .filter_map(|i| {
                let row = t.items.get(&i.code?).copied()?;
                let model = match world.units.get(&i.key).map(|u| &u.kind) {
                    Some(crate::bridge::world::KindData::Item(d)) => d.flags,
                    _ => 0,
                };
                Some(BodyItem {
                    key: i.key,
                    body: i.body,
                    row,
                    linked: (i.flags | model) & 0x4000 == 0,
                })
            })
            .collect();
        DescUnits {
            inner: ClientSkills::new(world, key),
            t,
            owner: key,
            class: world.units.get(&key).map_or(0, |u| u.class),
            body,
            seed: Seed::new(0, 0),
        }
    }

    fn row(&self, item: UnitKey) -> Option<ItemRow> {
        self.body.iter().find(|b| b.key == item).map(|b| b.row)
    }

    fn at(&self, loc: u8) -> Option<BodyItem> {
        self.body.iter().find(|b| b.body == loc).copied()
    }

    /// The weapon in use: the weapon at body location 4 (right
    /// arm), else 5. D2RUST's own, unverified: the weapon-switch slot is not
    /// told apart.
    pub fn weapon_in_use(&self) -> Option<UnitKey> {
        [4u8, 5]
            .iter()
            .filter_map(|&l| self.at(l))
            .find(|b| b.row.weapon && b.linked)
            .map(|b| b.key)
    }

    /// The item at the other hand of `w`.
    fn other_hand(&self, w: UnitKey) -> Option<UnitKey> {
        let at = self.body.iter().find(|b| b.key == w)?.body;
        let other = if at == 4 { 5 } else { 4 };
        self.at(other).map(|b| b.key)
    }

    /// A shield at a hand (item type `shld` 2).
    fn shield(&self) -> Option<ItemRow> {
        [4u8, 5]
            .iter()
            .filter_map(|&l| self.at(l))
            .find(|b| self.item_is_row(b.row, 2))
            .map(|b| b.row)
    }

    fn item_is_row(&self, r: ItemRow, itype: i32) -> bool {
        [r.type1, r.type2]
            .iter()
            .any(|&t| t != 0xFFFF && self.t.itype_is(i32::from(t), itype))
    }

    /// The grip of the player's hands.
    pub fn grip(&self) -> i32 {
        let (r, l) = (self.at(4).map(|b| b.row), self.at(5).map(|b| b.row));
        let barb = |r: ItemRow| self.class == 4 && r.one_or_two;
        match (r, l) {
            (Some(a), Some(b)) => {
                if barb(a) || barb(b) {
                    1
                } else if a.two_handed || b.two_handed {
                    2
                } else {
                    -1
                }
            }
            (Some(a), None) | (None, Some(a)) => {
                if a.two_handed || barb(a) {
                    2
                } else {
                    -1
                }
            }
            (None, None) => -1,
        }
    }

    /// Dual-wield unit: player class 4 or 6.
    pub fn dual_wield_class(&self) -> bool {
        matches!(self.class, 4 | 6)
    }

    fn total(&self, stat: u16) -> i32 {
        self.inner.world.total(self.owner, stat, 0)
    }

    fn has_flag_state(&self, flag: u8) -> bool {
        self.inner.world.units.get(&self.owner).is_some_and(|u| {
            u.states.iter().any(|&s| {
                self.t
                    .state_colors
                    .get(usize::from(s))
                    .is_some_and(|f| f & flag != 0)
            })
        })
    }
}

impl SkillUnits for DescUnits<'_> {
    type Unit = UnitKey;
    type Item = UnitKey;

    fn unit_type(&self, u: UnitKey) -> UnitType {
        self.inner.unit_type(u)
    }
    fn class_id(&self, u: UnitKey) -> i32 {
        self.inner.class_id(u)
    }
    fn stat(&self, u: UnitKey, stat: u16, layer: u16) -> i32 {
        self.inner.stat(u, stat, layer)
    }
    fn item_stat(&self, u: UnitKey, stat: u16, layer: u16) -> i32 {
        self.inner.item_stat(u, stat, layer)
    }
    fn base_stat(&self, u: UnitKey, stat: u16, layer: u16) -> i32 {
        self.inner.base_stat(u, stat, layer)
    }
    fn formula_stat(&self, u: UnitKey, stat: u16, mode: i32) -> i32 {
        self.inner.formula_stat(u, stat, mode)
    }
    fn stat_entries(&self, u: UnitKey, stat: u16, max: usize) -> Vec<(u16, i32)> {
        self.inner.stat_entries(u, stat, max)
    }
    fn has_state(&self, u: UnitKey, state: u16) -> bool {
        self.inner.has_state(u, state)
    }
    fn state_stat(&self, u: UnitKey, state: u16, stat: u16) -> Option<i32> {
        self.inner.state_stat(u, state, stat)
    }
    fn seed(&mut self, _u: UnitKey) -> &mut Seed {
        &mut self.seed
    }
    fn skill_list(&self, u: UnitKey) -> Vec<SimEntry> {
        self.inner.skill_list(u)
    }
    fn used_skill(&self, u: UnitKey) -> Option<SimEntry> {
        self.inner.used_skill(u)
    }
    fn current_weapon(&self, u: UnitKey) -> Option<UnitKey> {
        (u == self.owner).then(|| self.weapon_in_use()).flatten()
    }
    fn weapon(&self, u: UnitKey) -> Option<UnitKey> {
        self.current_weapon(u)
    }
    fn item_at(&self, u: UnitKey, loc: u8) -> Option<UnitKey> {
        (u == self.owner)
            .then(|| self.at(loc).map(|b| b.key))
            .flatten()
    }
    fn item_is(&self, item: UnitKey, itype: i32) -> bool {
        self.row(item).is_some_and(|r| self.item_is_row(r, itype))
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        self.t.itype_is(itype, parent)
    }
    fn wield_type(&self, item: UnitKey) -> i32 {
        if self.row(item).is_some() {
            self.grip()
        } else {
            -1
        }
    }
    fn item_damage(&self, _item: UnitKey, max: bool) -> i32 {
        // The weapon's secondary damage is in the unit totals (23 / 24).
        self.total(if max { 24 } else { 23 })
    }
    fn str_dex_bonus(&self, item: UnitKey) -> (i32, i32) {
        self.row(item)
            .map_or((0, 0), |r| (r.str_bonus, r.dex_bonus))
    }
    fn item_flag_throw(&self, item: UnitKey) -> bool {
        self.row(item).is_some_and(|r| self.t.throwable(r.type1))
    }
    fn missile_level(&self, _u: UnitKey) -> i32 {
        0
    }
}

/// Stats read here.
const STRENGTH: u16 = 0;
const DEX: u16 = 2;
const MAXDAMAGE_PERCENT: u16 = 17;
const MINDAMAGE_PERCENT: u16 = 18;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;
const SECONDARY_MINDAMAGE: u16 = 23;
const SECONDARY_MAXDAMAGE: u16 = 24;
const DAMAGEPERCENT: u16 = 25;
const ITEM_NORMALDAMAGE: u16 = 111;
const TOHIT_PERCENT: u16 = 119;
const KICKDAMAGE: u16 = 137;
const THROW_MIN: u16 = 159;
const THROW_MAX: u16 = 160;
const PROGRESSIVE_TOHIT: u16 = 325;
/// `throwingmastery`.
const STATE_THROWING_MASTERY: u16 = 78;
/// `holyshield`.
const STATE_HOLYSHIELD: u16 = 101;
/// Item types: `tpot` 38, `weap` 45.
const ITYPE_TPOT: i32 = 38;

/// What a `descdam` function draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DamageDraw {
    /// Nothing (a null entry, or the function returned early).
    None,
    /// One range in a colour.
    One { min: i32, max: i32, color: i32 },
    /// Two ranges, A on top.
    Two {
        a: (i32, i32),
        ca: i32,
        b: (i32, i32),
        cb: i32,
    },
}

impl DamageDraw {
    /// The lines (text, colour, y offset from the entry's y, two-line
    /// font) to draw, by the entry's damage line rules.
    pub fn lines(&self, top: i32) -> Vec<(String, i32, i32, bool)> {
        match *self {
            DamageDraw::None => Vec::new(),
            DamageDraw::One { min, max, color } => vec![(range_text(min, max), color, 0, false)],
            DamageDraw::Two { a, ca, b, cb } => {
                if a == (0, 0) {
                    vec![(range_text(b.0, b.1), cb, 0, false)]
                } else if b == (0, 0) {
                    vec![(range_text(a.0, a.1), ca, 0, false)]
                } else {
                    vec![
                        (range_text_two_line(a.0, a.1), ca, top, true),
                        (range_text_two_line(b.0, b.1), cb, 2, true),
                    ]
                }
            }
        }
    }
}

/// The evaluation context of one hand: unit, skill, level.
pub struct Hand<'a, 'b> {
    pub u: &'b mut DescUnits<'a>,
    pub t: &'b SkillTables,
    pub code: &'b [u8],
    pub key: UnitKey,
    pub skill: i32,
    pub lvl: i32,
}

impl Hand<'_, '_> {
    fn stat(&self, s: u16) -> i32 {
        self.u.total(s)
    }
    fn rec(&self) -> Option<&d2_data::tables::Skills> {
        self.t.skill(self.skill)
    }
    fn desc(&self) -> Option<&d2_data::tables::Skilldesc> {
        self.rec().and_then(|r| self.t.skilldesc_of(r))
    }
    fn src_dam(&self) -> i32 {
        self.rec().map_or(0, |r| i32::from(r.srcdam))
    }
    fn etype(&self) -> i32 {
        self.rec().map_or(0, |r| i32::from(r.etype))
    }
    fn finishing(&self) -> bool {
        self.rec().is_some_and(|r| r.finishing)
    }
    /// The progressive charges: for each `pgsv` state the unit
    /// has with a stat list, (k, v, n): k = the list's stat 350
    /// (`modifierlist_skill`), v = stat 351 (`modifierlist_level`), both
    /// positive and k a skills row; n = the list's value of k's
    /// `aurastat1` (≥ 0), positive.
    fn charges(&self) -> Vec<(i32, i32, i32)> {
        let Some(u) = self.u.inner.world.units.get(&self.key) else {
            return Vec::new();
        };
        self.u
            .t
            .pgsv_states
            .iter()
            .filter(|s| u.states.contains(s))
            .filter_map(|s| u.state_lists.get(s))
            .filter_map(|l| {
                let at = |stat: u16| l.get(&(stat, 0)).copied().unwrap_or(0);
                let (k, v) = (at(350), at(351));
                if k <= 0 || v <= 0 {
                    return None;
                }
                let a = self.t.skill(k)?.aurastat1 as i16;
                let n = u16::try_from(a).map_or(0, at);
                (n > 0).then_some((k, v, n))
            })
            .collect()
    }
    /// Charge bonus: Σ over the charges whose
    /// skill has `prgdam` 1 of (`Param1` + (v − 1) × `Param2` of k) × n.
    fn charge_bonus(&self) -> i32 {
        self.charges()
            .into_iter()
            .filter_map(|(k, v, n)| {
                let r = self.t.skill(k)?;
                (r.prgdam == 1).then(|| {
                    (r.param1 as i32)
                        .wrapping_add((v - 1).wrapping_mul(r.param2 as i32))
                        .wrapping_mul(n)
                })
            })
            .fold(0i32, i32::wrapping_add)
    }
    /// Charge damage into (min, max, c): per
    /// charge (any `prgdam`), k without a skilldesc row → `elem_min` /
    /// `elem_max` (k, v) >> 8; else j = clamp(n, 1, 3) and the skilldesc's
    /// `p{j}dmmin` / `p{j}dmmax` (context k, v), `p{j}dmelem` 1–12 → its
    /// EType colour.
    fn charge_damage(&mut self, min: &mut i32, max: &mut i32, c: &mut i32) {
        for (k, v, n) in self.charges() {
            let Some(r) = self.t.skill(k) else { continue };
            let Some(d) = self.t.skilldesc_of(r) else {
                *min += elem_min(self.u, self.t, Some(self.key), k, v, true) >> 8;
                *max += elem_max(self.u, self.t, Some(self.key), k, v, true) >> 8;
                continue;
            };
            let (lo, hi, el) = match n.clamp(1, 3) {
                1 => (d.p1dmmin, d.p1dmmax, d.p1dmelem),
                2 => (d.p2dmmin, d.p2dmmax, d.p2dmelem),
                _ => (d.p3dmmin, d.p3dmmax, d.p3dmelem),
            };
            *min += eval_desc(self.u, self.t, self.code, Some(self.key), lo, k, v);
            *max += eval_desc(self.u, self.t, self.code, Some(self.key), hi, k, v);
            if (1..=12).contains(&el) {
                if let Some(col) = etype_color(i32::from(el)) {
                    *c = col;
                }
            }
        }
    }
    fn param(&self, n: u8) -> i32 {
        self.rec().map_or(0, |r| {
            (match n {
                1 => r.param1,
                2 => r.param2,
                3 => r.param3,
                4 => r.param4,
                _ => r.param5,
            }) as i32
        })
    }
    fn eval_d(&mut self, field: u32) -> i32 {
        eval_desc(
            self.u,
            self.t,
            self.code,
            Some(self.key),
            field,
            self.skill,
            self.lvl,
        )
    }
    fn eval_s(&mut self, field: u32) -> i32 {
        eval_skill(self.u, self.t, Some(self.key), field, self.skill, self.lvl)
    }
    fn ddam(&mut self) -> Option<(i32, i32)> {
        let d = self.desc()?;
        let (c1, c2) = (d.ddam_calc1, d.ddam_calc2);
        Some((self.eval_d(c1), self.eval_d(c2)))
    }
    fn phys(&mut self, srcdam: bool) -> (i32, i32) {
        (
            phys_min(self.u, self.t, Some(self.key), self.skill, self.lvl, srcdam) >> 8,
            phys_max(self.u, self.t, Some(self.key), self.skill, self.lvl, srcdam) >> 8,
        )
    }
    fn elem_raw(&mut self) -> (i32, i32) {
        (
            elem_min(self.u, self.t, Some(self.key), self.skill, self.lvl, true),
            elem_max(self.u, self.t, Some(self.key), self.skill, self.lvl, true),
        )
    }
    /// Skill elements: (a, b) and the colour, if any.
    fn skill_elements(&mut self) -> (i32, i32, Option<i32>) {
        let (mut a, mut b) = self.elem_raw();
        if self.etype() == 5 {
            let n = elem_len(self.u, self.t, Some(self.key), self.skill, self.lvl);
            a = a.wrapping_mul(n);
            b = b.wrapping_mul(n);
        }
        (a >> 8, b >> 8, etype_color(self.etype()))
    }
    fn mastery(&self, w: Option<UnitKey>, ty: u8) -> i32 {
        weapon_mastery(self.u, self.t, Some(self.key), w, Some(self.skill), ty)
    }

    /// `damblue` / `damred` colour override.
    fn dam_color(&self, c: &mut i32) {
        if self.u.has_flag_state(DAMBLUE) {
            *c = 3;
        }
        if self.u.has_flag_state(DAMRED) {
            *c = 1;
        }
    }

    /// `weapon_phys`.
    #[allow(clippy::too_many_arguments)]
    fn weapon_phys(
        &mut self,
        e: i32,
        min: &mut i32,
        max: &mut i32,
        c: &mut i32,
        f0: i32,
        s: i32,
        w: Option<UnitKey>,
    ) {
        if self.rec().is_none() {
            return;
        }
        let w = w.or_else(|| self.u.weapon_in_use());
        let (mut b, mut big) = match w {
            None => (self.stat(MINDAMAGE) + 1, self.stat(MAXDAMAGE) + 2),
            Some(_) if self.u.grip() == 2 => (
                self.stat(SECONDARY_MINDAMAGE),
                self.stat(SECONDARY_MAXDAMAGE),
            ),
            Some(_) => (self.stat(MINDAMAGE), self.stat(MAXDAMAGE)),
        };
        let s = if s == 0 { self.src_dam() } else { s };
        if s != 128 {
            b = b.wrapping_mul(s) / 128;
            big = big.wrapping_mul(s) / 128;
        }
        if s != 0 {
            *min += b.max(1);
            *max += big.max(2);
        }
        let mut p = e + self.stat(DAMAGEPERCENT) + self.mastery(w, 1);
        // Rule 4: a `finishing` skill with a charge bonus.
        if self.finishing() {
            let bonus = self.charge_bonus();
            if bonus != 0 {
                p += bonus;
                *c = 3;
            }
        }
        let mut f1 = 0;
        match w.and_then(|w| self.u.row(w)) {
            None => p += self.stat(STRENGTH),
            Some(r) => {
                if r.str_bonus != 0 {
                    p += self.stat(STRENGTH) * r.str_bonus / 100;
                }
                if r.dex_bonus != 0 {
                    p += self.stat(DEX) * r.dex_bonus / 100;
                }
                f1 = self.stat(ITEM_NORMALDAMAGE);
            }
        }
        self.dam_color(c);
        if p < -89 {
            p = -90;
        }
        *min = (self.stat(MINDAMAGE_PERCENT) + 100 + p).wrapping_mul(*min) / 100 + f1 + f0;
        *max = (self.stat(MAXDAMAGE_PERCENT) + 100 + p).wrapping_mul(*max) / 100 + f1 + f0;
    }

    /// Stat elements.
    fn stat_elements(&mut self, min: &mut i32, max: &mut i32, c: &mut i32) {
        const EL: [(u16, u16, u16, i32); 4] = [
            (48, 49, 329, 1),
            (50, 51, 330, 9),
            (54, 55, 331, 3),
            (52, 53, 0, 3),
        ];
        for (lo, hi, m, col) in EL {
            let (mut mn, mut mx) = (self.stat(lo), self.stat(hi));
            let m = if m == 0 { 0 } else { self.stat(m) };
            if m != 0 {
                mn += pct(mn, m, 100);
                mx += pct(mx, m, 100);
            }
            mn = mn.min(mx);
            *min += mn;
            *max += mx;
            if mn != 0 || mx != 0 {
                *c = col;
            }
        }
        let (mut mn, mut mx) = (self.stat(57), self.stat(58));
        if mx != 0 {
            let m = self.stat(332);
            if m != 0 {
                mn += pct(mn, m, 100);
                mx += pct(mx, m, 100);
            }
            let o = self.stat(101);
            let n = if o > 0 {
                o
            } else {
                let q = self.stat(326);
                self.stat(59) / if q < 2 { 1 } else { q }
            };
            *c = 2;
            *min += n.wrapping_mul(mn) >> 8;
            *max += n.wrapping_mul(mx) >> 8;
        }
        if *max > 0 {
            *min = (*min).max(1);
            *max = (*max).max(*min + 1);
        }
        // Rule 4: a `finishing` skill adds the charge damage.
        if self.finishing() {
            self.charge_damage(min, max, c);
        }
        self.dam_color(c);
    }

    /// Weapon part with s = `SrcDam`, W = in use.
    fn weapon_part(&mut self, min: &mut i32, max: &mut i32, c: &mut i32) {
        let s = self.src_dam();
        if s == 0 {
            return;
        }
        let (mut a, mut b) = (0, 0);
        self.weapon_phys(0, &mut a, &mut b, c, 0, 128, None);
        self.stat_elements(&mut a, &mut b, c);
        *min += a.wrapping_mul(s) / 128;
        *max += b.wrapping_mul(s) / 128;
    }

    /// Stat damage of the skill's `EType`.
    fn etype_stat_damage(&self) -> (i32, i32) {
        match self.etype() {
            1 => (self.stat(48), self.stat(49)),
            2 => (self.stat(50), self.stat(51)),
            3 => (self.stat(52), self.stat(53)),
            4 | 12 => (self.stat(54), self.stat(55)),
            5 => {
                let o = self.stat(101);
                let n = if o > 0 {
                    o
                } else {
                    let q = self.stat(326);
                    self.stat(59) / if q < 2 { 1 } else { q }
                };
                (
                    n.wrapping_mul(self.stat(57)) >> 8,
                    n.wrapping_mul(self.stat(58)) >> 8,
                )
            }
            _ => (0, 0),
        }
    }

    /// Rule 2, a missile potion with `missiletype` m: the missile's physical +
    /// elemental min / max at level 1 for U, its `EType` giving the colour (1 →
    /// 1, 2 → 4, 4 → 3, 5 → 2 with a, b / max(1, length / 25)); min = a >> 8,
    /// max = max(b >> 8, min).
    fn potion_damage(&mut self, m: i32) -> (i32, i32, i32) {
        let k = Some(self.key);
        let mut a = miss_phys_min(self.u, self.t, None, k, m, 1)
            .wrapping_add(miss_elem_min(self.u, self.t, None, k, m, 1));
        let mut b = miss_phys_max(self.u, self.t, None, k, m, 1)
            .wrapping_add(miss_elem_max(self.u, self.t, None, k, m, 1));
        let mut c = 0;
        match self.t.missile(m).map_or(0, |r| r.etype) {
            1 => c = 1,
            2 => c = 4,
            4 => c = 3,
            5 => {
                c = 2;
                let n = (miss_elem_len(self.u, self.t, None, m, 1) / 25).max(1);
                a /= n;
                b /= n;
            }
            _ => {}
        }
        let min = a >> 8;
        (min, (b >> 8).max(min), c)
    }

    /// Throw damage with W = in use or `w`.
    fn throw_damage(&mut self, e: i32, w: Option<UnitKey>) -> (i32, i32, i32) {
        let mut c = 0;
        let Some(w) = w.or_else(|| self.u.weapon_in_use()) else {
            return (0, 0, c);
        };
        let Some(r) = self.u.row(w) else {
            return (0, 0, c);
        };
        if !self.u.t.throwable(r.type1) {
            return (0, 0, c);
        }
        if self.u.item_is_row(r, ITYPE_TPOT) {
            return self.potion_damage(i32::from(r.missile_type));
        }
        let mut p = 0;
        if r.str_bonus != 0 {
            p += self.stat(STRENGTH) * r.str_bonus / 100;
        }
        if r.dex_bonus != 0 {
            p += self.stat(DEX) * r.dex_bonus / 100;
        }
        p += self.stat(DAMAGEPERCENT);
        if p < -89 {
            p = -90;
        }
        let t = self.stat(THROW_MIN);
        let mut min = t + (self.stat(MINDAMAGE_PERCENT) + p).wrapping_mul(t) / 100;
        let t = self.stat(THROW_MAX);
        let mut max = t + (self.stat(MAXDAMAGE_PERCENT) + p).wrapping_mul(t) / 100;
        let mut q = e;
        if self.u.has_state(self.key, STATE_THROWING_MASTERY) {
            q += self.mastery(Some(w), 1);
        }
        min += pct(min, q, 100);
        max += pct(max, q, 100);
        max = max.max(min);
        let (mut a, mut b) = (0, 0);
        self.stat_elements(&mut a, &mut b, &mut c);
        b = b.max(a);
        (min + a, max + b, c)
    }

    /// `AR` with W = in use or `w`.
    fn ar(&mut self, w: Option<UnitKey>, tohit_factor: i32, throw: bool) -> (i32, i32) {
        let mut c = 0;
        let mut v = attack_rating(self, tohit_factor);
        let w = w.or_else(|| self.u.weapon_in_use());
        if w.and_then(|w| self.u.row(w))
            .is_some_and(|r| r.type1 == ITYPE_TPOT as u16)
        {
            v = 0;
        }
        let m = if !throw || self.u.has_state(self.key, STATE_THROWING_MASTERY) {
            self.mastery(w, 0)
        } else {
            0
        };
        let mut p = m
            + self.stat(TOHIT_PERCENT)
            + to_hit(self.u, self.t, Some(self.key), self.skill, self.lvl);
        if !throw && self.finishing() {
            let pt = self.stat(PROGRESSIVE_TOHIT);
            if pt != 0 {
                p += pt;
                c = 3;
            }
        }
        v += v.wrapping_mul(p) / 100;
        if throw {
            c = 0;
        }
        if self.u.has_flag_state(ATTBLUE) {
            c = 3;
        }
        if self.u.has_flag_state(ATTRED) {
            c = 1;
        }
        (v, c)
    }
}

/// `attack_rating(U)`: `tohit(19)` +
/// 5 × (dex − 7) + the class `ToHitFactor`.
fn attack_rating(h: &Hand, tohit_factor: i32) -> i32 {
    h.stat(19) + 5 * (h.stat(DEX) - 7) + tohit_factor
}

/// A `descdam` function for one hand.
pub fn descdam(h: &mut Hand, n: u16) -> DamageDraw {
    use DamageDraw as D;
    if h.rec().is_none() {
        return D::None;
    }
    match n {
        1 | 18 | 7 => {
            let Some((a, b)) = h.ddam() else {
                return D::None;
            };
            if n == 1 && h.u.dual_wield_class() {
                return driver_weapon(h, a, b, true);
            }
            entry7(h, a, b)
        }
        19 | 20 => {
            let Some((a, b)) = h.ddam() else {
                return D::None;
            };
            if h.u.dual_wield_class() {
                return driver_weapon(h, a, b, n == 19);
            }
            if n == 20 {
                return entry7(h, a, b);
            }
            // The weapon damage with the weapon in use, E = A, F0 = B, s = SrcDam.
            let (mut mn, mut mx, mut c) = (0, 0, 0);
            h.weapon_phys(a, &mut mn, &mut mx, &mut c, b, 0, None);
            h.stat_elements(&mut mn, &mut mx, &mut c);
            D::One {
                min: mn,
                max: mx,
                color: c,
            }
        }
        2 => {
            let Some(r) = h.rec() else { return D::None };
            let v = (r.mindam as i32).wrapping_shl(((i32::from(r.hitshift) - 8) & 31) as u32)
                + h.stat(KICKDAMAGE);
            D::One {
                min: v,
                max: v,
                color: 0,
            }
        }
        3 => {
            let (a, b, c) = h.throw_damage(0, None);
            let s = h.src_dam();
            let (e, big, ec) = h.skill_elements();
            D::One {
                min: a.wrapping_mul(s) / 128 + e,
                max: b.wrapping_mul(s) / 128 + big,
                color: ec.unwrap_or(c),
            }
        }
        5 | 24 => {
            let mut c = etype_color(h.etype()).unwrap_or(0);
            let (mut a, mut b) = (0, 0);
            if n == 24 {
                let (p, q) = h.phys(false);
                a += p;
                b += q;
                h.weapon_part(&mut a, &mut b, &mut c);
            } else {
                h.weapon_part(&mut a, &mut b, &mut c);
                let (p, q) = h.phys(false);
                a += p;
                b += q;
            }
            let (e, big, _) = h.skill_elements();
            D::One {
                min: a + e,
                max: b + big,
                color: c,
            }
        }
        6 => {
            let mut ca = 0;
            let (mut a0, mut a1) = (0, 0);
            h.weapon_part(&mut a0, &mut a1, &mut ca);
            let (p, q) = h.phys(false);
            let (em, ex) = h.elem_raw();
            let (sm, sx) = h.etype_stat_damage();
            D::Two {
                a: (a0, a1),
                ca,
                b: (p + (em >> 8) + sm, q + (ex >> 8) + sx),
                cb: etype_color(h.etype()).unwrap_or(0),
            }
        }
        8 => {
            let Some((a, b)) = h.ddam() else {
                return D::None;
            };
            let (m, big) = h.elem_raw();
            let a = if a == 0 { 1 } else { a };
            let b = if b == 0 { 1 } else { b };
            D::One {
                min: (a.wrapping_mul(m).wrapping_mul(25) / b) >> 8,
                max: (a.wrapping_mul(big).wrapping_mul(25) / b) >> 8,
                color: etype_color(h.etype()).unwrap_or(0),
            }
        }
        9 => {
            let Some(r) = h.rec() else { return D::None };
            let sh = (u32::from(r.hitshift)) & 31;
            let (lo, hi) = ((r.mindam as i32) << sh, (r.maxdam as i32) << sh);
            let (m, big) = h.elem_raw();
            let mut mn = (lo + m).wrapping_mul(75) >> 8;
            let mut mx = (hi + big).wrapping_mul(75) >> 8;
            let mut c = etype_color(h.etype()).unwrap_or(0);
            h.weapon_part(&mut mn, &mut mx, &mut c);
            D::One {
                min: mn,
                max: mx,
                color: c,
            }
        }
        10 => entry10(h),
        11 => {
            let mut c = etype_color(h.etype()).unwrap_or(0);
            let (mut a, mut b) = (0, 0);
            h.weapon_part(&mut a, &mut b, &mut c);
            let (mut b0, mut big0) = if h.u.weapon_in_use().is_some() && h.u.grip() == 2 {
                (h.stat(SECONDARY_MINDAMAGE), h.stat(SECONDARY_MAXDAMAGE))
            } else {
                (h.stat(MINDAMAGE), h.stat(MAXDAMAGE))
            };
            b0 = b0.max(1);
            big0 = big0.max(2).max(b0);
            let Some(r) = h.rec() else { return D::None };
            let calcs = [(r.calc1, 329u16), (r.calc2, 331), (r.calc3, 330)];
            for (f, m) in calcs {
                let mut p = h.eval_s(f);
                let mv = h.stat(m);
                if p != 0 && mv != 0 {
                    p += pct(p, mv, 100);
                }
                a += pct(b0, p, 100);
                b += pct(big0, p, 100);
            }
            if c == 0 {
                c = 3;
            }
            D::One {
                min: a,
                max: b,
                color: c,
            }
        }
        12 => {
            let (mut a, mut b, c) = h.skill_elements();
            let p = if h.u.inner.world.expansion != 0 {
                concentration(h.u, h.t, h.key, h.skill)
            } else {
                h.stat(DAMAGEPERCENT).max(-90)
            };
            a += p.wrapping_mul(a) / 100;
            b += p.wrapping_mul(b) / 100;
            D::One {
                min: a,
                max: b,
                color: c.unwrap_or(0),
            }
        }
        13 => {
            let Some(r) = h.rec() else { return D::None };
            let a = h.eval_s(r.calc1);
            driver_throw(h, a)
        }
        14 => {
            let (mut a, mut b, mut c) = (0, 0, 0);
            h.weapon_phys(0, &mut a, &mut b, &mut c, 0, 0, None);
            let q = h.param(5);
            a += q.wrapping_mul(a) / 100;
            b += q.wrapping_mul(b) / 100;
            h.stat_elements(&mut a, &mut b, &mut c);
            D::One {
                min: a,
                max: b,
                color: c,
            }
        }
        15 | 16 => entry15_16(h, n),
        17 => {
            let Some((a, b)) = h.ddam() else {
                return D::None;
            };
            let (mut a0, mut a1, mut ca) = (0, 0, 0);
            h.weapon_phys(a, &mut a0, &mut a1, &mut ca, b, 128, None);
            h.stat_elements(&mut a0, &mut a1, &mut ca);
            let (e, big, cb) = h.skill_elements();
            D::Two {
                a: (a0, a1),
                ca,
                b: (e, big),
                cb: cb.unwrap_or(0),
            }
        }
        21 => entry21(h),
        22 => {
            let (a, b, ca) = h.throw_damage(0, None);
            let s = h.src_dam();
            let (e, big, cb) = h.skill_elements();
            D::Two {
                a: (pct(a, s, 128), pct(b, s, 128)),
                ca,
                b: (e, big),
                cb: cb.unwrap_or(0),
            }
        }
        23 => {
            let (mut a0, mut a1, mut ca) = (0, 0, 0);
            h.weapon_phys(0, &mut a0, &mut a1, &mut ca, 0, 128, None);
            h.stat_elements(&mut a0, &mut a1, &mut ca);
            let (e, big, cb) = h.skill_elements();
            D::Two {
                a: (a0, a1),
                ca,
                b: (e, big),
                cb: cb.unwrap_or(0),
            }
        }
        // Left hand throw draws no line here (the panel's lines
        // stay as they are).
        4 => D::None,
        _ => D::None,
    }
}

/// Entry 7 (and 1 / 18 / 20 without dual weapons).
fn entry7(h: &mut Hand, a: i32, b: i32) -> DamageDraw {
    let (mut mn, mut mx, mut c) = (0, 0, 0);
    h.weapon_phys(a, &mut mn, &mut mx, &mut c, b, 128, None);
    h.stat_elements(&mut mn, &mut mx, &mut c);
    let s = h.src_dam();
    if s != 128 {
        mn = pct(mn, s, 128);
        mx = pct(mx, s, 128);
    }
    let (p, q) = h.phys(false);
    let (em, ex) = h.elem_raw();
    mn += p + (em >> 8);
    mx += q + (ex >> 8);
    DamageDraw::One {
        min: mn,
        max: mx,
        color: c,
    }
}

/// Entry 10 (smite).
fn entry10(h: &mut Hand) -> DamageDraw {
    let Some(sh) = h.u.shield() else {
        return DamageDraw::None;
    };
    let (mut b, mut big) = (i32::from(sh.mindam), i32::from(sh.maxdam));
    let list = h.u.skill_list(h.key);
    if h.u.has_state(h.key, STATE_HOLYSHIELD) {
        if let Some(e) = highest_entry(&list, 117) {
            let l = skill_level(h.u, h.t, Some(h.key), Some(&e), true);
            b += phys_min(h.u, h.t, Some(h.key), 117, l, true) >> 8;
            big += phys_max(h.u, h.t, Some(h.key), 117, l, true) >> 8;
        }
    }
    let mut p = h.param(3) + (h.lvl - 1) * h.param(4);
    if sh.str_bonus != 0 {
        p += h.stat(STRENGTH) * sh.str_bonus / 100;
    }
    if sh.dex_bonus != 0 {
        p += h.stat(DEX) * sh.dex_bonus / 100;
    }
    p += h.stat(DAMAGEPERCENT);
    if p < -89 {
        p = -90;
    }
    let mut c = 0;
    h.dam_color(&mut c);
    DamageDraw::One {
        min: (p.wrapping_mul(b) + h.stat(MINDAMAGE_PERCENT)) / 100 + b,
        max: (p.wrapping_mul(big) + h.stat(MAXDAMAGE_PERCENT)) / 100 + big,
        color: c,
    }
}

/// Entries 15 (dragon talon, flight) and 16 (dragon tail).
fn entry15_16(h: &mut Hand, n: u16) -> DamageDraw {
    let (s, s2) = h.phys(n == 15);
    // The charge bonus.
    let t = h.charge_bonus();
    let p = if n == 15 {
        t + h.param(1) + (h.lvl - 1) * h.param(2)
    } else {
        t
    };
    let mut c = if n == 16 {
        etype_color(h.etype()).unwrap_or(0)
    } else {
        0
    };
    if n == 15 && p != 0 {
        c = 3;
    }
    let (k, big_k, kp) = kick(h, p);
    let mut mn = k + pct(k, kp, 100) + s + pct(s, p, 100);
    let mut mx = big_k + pct(big_k, kp, 100) + s2 + pct(s2, p, 100);
    if n == 16 {
        let Some(r) = h.rec() else {
            return DamageDraw::None;
        };
        let q = h.eval_s(r.calc1);
        mn += pct(mn, q, 100);
        mx += pct(mx, q, 100);
        // The stat elements read from the totals agree
        // with the weapon toggles (an active weapon).
        h.stat_elements(&mut mn, &mut mx, &mut c);
    } else {
        // Entry 15: the charge damage into a fresh colour.
        let (mut a, mut b, mut c2) = (0, 0, 0);
        h.charge_damage(&mut a, &mut b, &mut c2);
        mn += a;
        mx += b;
    }
    DamageDraw::One {
        min: mn,
        max: mx,
        color: c,
    }
}

/// Kick from k = K = 0, kp = `p`:
/// `item_kickdamage(137)` to both; with boots (body location 9) their
/// `mindam` / `maxdam` (record +0xFE / +0xFF) plus 137 again, and kp +=
/// `maxdamage_percent(17)` + P, P = str × `StrBonus` / 100 + dex ×
/// `DexBonus` / 100 (the boots') + `damagepercent(25)`, at least −90.
/// The game reads these with the hand weapons' stat lists detached;
/// the model reads the totals, equal for weapons without those stats
/// (a claw with fire damage agrees).
fn kick(h: &Hand, p: i32) -> (i32, i32, i32) {
    let ik = h.stat(KICKDAMAGE);
    let (mut k, mut big, mut kp) = (ik, ik, p);
    if let Some(b) = h.u.at(9) {
        let r = b.row;
        let lo = i32::from(r.mindam) + ik;
        let hi = (i32::from(r.maxdam) + ik).max(lo);
        let mut q = 0;
        if r.str_bonus != 0 {
            q += pct(h.stat(STRENGTH), r.str_bonus, 100);
        }
        if r.dex_bonus != 0 {
            q += pct(h.stat(DEX), r.dex_bonus, 100);
        }
        q += h.stat(DAMAGEPERCENT);
        if q < -89 {
            q = -90;
        }
        kp += h.stat(MAXDAMAGE_PERCENT) + q;
        k += lo;
        big += hi;
    }
    (k, big, kp)
}

/// Entry 21 (lightning bolt).
fn entry21(h: &mut Hand) -> DamageDraw {
    let Some(w) = h.u.weapon_in_use() else {
        return DamageDraw::None;
    };
    let Some(r) = h.u.row(w) else {
        return DamageDraw::None;
    };
    if !h.u.t.throwable(r.type1) {
        return DamageDraw::None;
    }
    let mut p = 0;
    if r.str_bonus != 0 {
        p += h.stat(STRENGTH) * r.str_bonus / 100;
    }
    if r.dex_bonus != 0 {
        p += h.stat(DEX) * r.dex_bonus / 100;
    }
    p += h.stat(DAMAGEPERCENT);
    if p < -89 {
        p = -90;
    }
    let t = h.stat(THROW_MIN);
    let mut a = t + (h.stat(MINDAMAGE_PERCENT) + p).wrapping_mul(t) / 100;
    let t = h.stat(THROW_MAX);
    let mut b = t + (h.stat(MAXDAMAGE_PERCENT) + p).wrapping_mul(t) / 100;
    let q = if h.u.has_state(h.key, STATE_THROWING_MASTERY) {
        h.mastery(Some(w), 1)
    } else {
        0
    };
    a += pct(a, q, 100);
    b += pct(b, q, 100);
    let (sm, sx) = h.etype_stat_damage();
    a += sm;
    b += sx;
    b = b.max(a);
    let s = h.src_dam();
    a = pct(a, s, 128);
    b = pct(b, s, 128);
    let (e, big, c) = h.skill_elements();
    DamageDraw::One {
        min: a + e,
        max: b + big,
        color: c.unwrap_or(0),
    }
}

/// The hand swap around `f`: on
/// a copy of the model, `off`'s list leaves U's totals and `on`'s is
/// attached (the item's `stats_off`), so
/// each hand's range and attack rating read only its own weapon.
fn with_hands<R>(
    h: &mut Hand,
    on: Option<UnitKey>,
    off: Option<UnitKey>,
    f: impl FnOnce(&mut Hand) -> R,
) -> R {
    use crate::bridge::world::KindData;
    let mut w = h.u.inner.world.clone();
    for (k, is_off) in [(off, true), (on, false)] {
        if let Some(KindData::Item(d)) = k.and_then(|k| w.units.get_mut(&k)).map(|u| &mut u.kind) {
            d.stats_off = is_off;
        }
    }
    let mut u = DescUnits::new(&w, h.key, h.u.t);
    let mut h2 = Hand {
        u: &mut u,
        t: h.t,
        code: h.code,
        key: h.key,
        skill: h.skill,
        lvl: h.lvl,
    };
    f(&mut h2)
}

/// Both hands hold a usable weapon that is not a missile potion.
fn dual_pair(u: &DescUnits) -> bool {
    let ok = |l: u8| {
        u.at(l)
            .is_some_and(|b| b.row.weapon && !u.item_is_row(b.row, ITYPE_TPOT))
    };
    ok(4) && ok(5)
}

/// The weapon damage driver with f = weapon_phys + stat elements,
/// `k` the skill-elements switch. Single weapon: range 1 only.
fn driver_weapon(h: &mut Hand, a: i32, b: i32, k: bool) -> DamageDraw {
    let (e, big, c) = if k {
        let (e, big, c) = h.skill_elements();
        (e, big, c.unwrap_or(0))
    } else {
        (0, 0, 0)
    };
    let w = h.u.weapon_in_use();
    let o = if dual_pair(h.u) {
        w.and_then(|w| h.u.other_hand(w))
    } else {
        None
    };
    let range = |h: &mut Hand, w: Option<UnitKey>| {
        let (mut mn, mut mx, mut cc) = (0, 0, c);
        h.weapon_phys(a, &mut mn, &mut mx, &mut cc, b, 0, w);
        h.stat_elements(&mut mn, &mut mx, &mut cc);
        let s = h.src_dam();
        if mn != 0 && mx != 0 {
            mn = mn.wrapping_mul(s) / 128 + e;
            mx = mx.wrapping_mul(s) / 128 + big;
        }
        ((mn, mx), cc)
    };
    // The other hand's range with W's list off, then W's
    // with O's off; rule 4: W alone, attached.
    let (r1, c1) = with_hands(h, w, o, |h| range(h, w));
    let (r2, c2) = if o.is_some() {
        with_hands(h, o, w, |h| range(h, o))
    } else {
        ((0, 0), c)
    };
    DamageDraw::Two {
        a: r1,
        ca: c1,
        b: r2,
        cb: c2,
    }
}

/// The damage driver with f = throw damage (entry 13).
fn driver_throw(h: &mut Hand, a: i32) -> DamageDraw {
    let (e, big, c) = h.skill_elements();
    let c = c.unwrap_or(0);
    let w = h.u.weapon_in_use();
    let o = if h.u.dual_wield_class() && dual_pair(h.u) {
        w.and_then(|w| h.u.other_hand(w))
    } else {
        None
    };
    let s = h.src_dam();
    let range = |h: &mut Hand, w: Option<UnitKey>| {
        let (mut mn, mut mx, cc) = h.throw_damage(a, w);
        if mn != 0 && mx != 0 {
            mn = mn.wrapping_mul(s) / 128 + e;
            mx = mx.wrapping_mul(s) / 128 + big;
        }
        ((mn, mx), if cc == 0 { c } else { cc })
    };
    // The other hand's range with W's list off, then W's
    // with O's off; rule 4: W alone, attached.
    let (r1, c1) = with_hands(h, w, o, |h| range(h, w));
    let (r2, c2) = if o.is_some() {
        with_hands(h, o, w, |h| range(h, o))
    } else {
        ((0, 0), c)
    };
    DamageDraw::Two {
        a: r1,
        ca: c1,
        b: r2,
        cb: c2,
    }
}

/// A `descatt` function: (v1, c1, v2, c2).
pub fn descatt(h: &mut Hand, n: u16, tohit_factor: i32) -> (i32, i32, i32, i32) {
    if h.rec().is_none() {
        return (0, 0, 0, 0);
    }
    match n {
        1 => {
            let (v, c) = h.ar(None, tohit_factor, false);
            (v, c, 0, 0)
        }
        2 => {
            let mut c = 0;
            let mut v = attack_rating(h, tohit_factor);
            let mut p = h.stat(TOHIT_PERCENT) + to_hit(h.u, h.t, Some(h.key), h.skill, h.lvl);
            if h.finishing() {
                let pt = h.stat(PROGRESSIVE_TOHIT);
                if pt != 0 {
                    p += pt;
                    c = 3;
                }
            }
            v += v.wrapping_mul(p) / 100;
            if h.u.has_flag_state(ATTBLUE) {
                c = 3;
            }
            if h.u.has_flag_state(ATTRED) {
                c = 1;
            }
            (v, c, 0, 0)
        }
        3 | 5 => {
            let w = h.u.weapon_in_use();
            let pair = h.u.dual_wield_class() && dual_pair(h.u);
            let o = if pair {
                w.and_then(|w| h.u.other_hand(w))
            } else {
                None
            };
            let (v1, c1) = with_hands(h, w, o, |h| h.ar(w, tohit_factor, false));
            let (v2, c2) = if pair {
                with_hands(h, o, w, |h| h.ar(o, tohit_factor, false))
            } else {
                (0, 0)
            };
            if n == 5 {
                // Measured (`ui-draws-char-desc-ass-255`: a katar alone draws
                // its AR 628): without a weapon pair the driver's one call
                // lands in the slot that v1:= v2 then reads; a non-dual class
                // draws nothing (Edge case 6).
                if !h.u.dual_wield_class() {
                    (0, 0, 0, 0)
                } else if pair {
                    (v2, c2, 0, 0)
                } else {
                    (v1, c1, 0, 0)
                }
            } else {
                (v1, c1, v2, c2)
            }
        }
        4 => {
            let (v, c) = h.ar(None, tohit_factor, true);
            (v, c, 0, 0)
        }
        _ => (0, 0, 0, 0),
    }
}

/// The use state the damage block gates on (`d` / `a`): the skill use
/// tests over the model, as
/// `bridge::use_state` reads them ([`hand_use_code`]). Returns whether the
/// state is 0, 1, 6 or 8 (the codes that draw).
pub fn hand_state_draws(h: &mut Hand) -> bool {
    matches!(hand_use_code(h), 0 | 1 | 6 | 8)
}

/// The use state code of [`hand_state_draws`]: tests 1–8 and 10 (cooldown) over
/// the model; test 5 is the item type test with the matched item's
/// rules (the `shoots` ammo rule, as the client's own test: a bow without
/// quiver draws the same panel); test 8 the start stat.
/// PROVISIONAL: a hand's item skill reads as a native one (the charge branch of
/// test 6 and test 9 need the entry's owner GUID and charges, which [`Hand`]
/// does not carry); settled by modelling the charge and owner tests
/// on the hand's entry.
pub fn hand_use_code(h: &mut Hand) -> u32 {
    let Some(r) = h.rec() else {
        return 3;
    };
    if !r.ingame {
        return 3;
    }
    if h.lvl == 0 {
        return 7;
    }
    if r.aura {
        return 6;
    }
    if r.passive {
        return 5;
    }
    if !item_type_test(h, r) {
        return 2;
    }
    if !can_afford(h, r) {
        return 1;
    }
    let restrict = r.restrict;
    let states = [r.state1, r.state2, r.state3];
    let in_form =
        h.u.inner
            .world
            .units
            .get(&h.key)
            .is_some_and(|u| u.states.iter().any(|&s| h.u.t.form_states.contains(&s)));
    let shape = match restrict {
        0 => !in_form,
        2 => {
            in_form
                && states
                    .iter()
                    .take_while(|&&s| (s as i16) >= 0)
                    .any(|&s| h.u.has_state(h.key, s))
        }
        _ => true,
    };
    if !shape {
        return 4;
    }
    // Test 8: a player below `startmana` << 8 mana and not
    // in state 12 (`inferno`) → 1.
    let start = i32::from(r.startmana as i16);
    if h.key.unit_type == crate::bridge::world::PLAYER
        && start > 0
        && h.stat(8) < start << 8
        && !h.u.has_state(h.key, 12)
    {
        return 1;
    }
    // Test 10: a player with state 121 (`skilldelay`) and a
    // `delay` calc → 8. Any `delay` calc reads as > 0 (no skill calc
    // evaluator on this seam), as `bridge::use_state`: exact for the stock
    // tables, whose 23 non-empty `skills.txt` `delay` cells are all integers > 0.
    if h.key.unit_type == crate::bridge::world::PLAYER
        && h.u.has_state(h.key, 121)
        && r.delay != u32::MAX
    {
        return 8;
    }
    0
}

/// `can_afford` for a native entry.
fn can_afford(h: &Hand, r: &d2_data::tables::Skills) -> bool {
    let n = (h.lvl - 1).max(0);
    let c = i32::from(r.mana as i16)
        .wrapping_add(i32::from(r.lvlmana as i16).wrapping_mul(n))
        .wrapping_shl(u32::from(r.manashift as u8));
    if h.u.has_state(h.key, 114) {
        h.stat(6) >= c
    } else {
        h.stat(8) >= c
    }
}

/// The item type test on the body items.
fn item_type_test(h: &Hand, r: &d2_data::tables::Skills) -> bool {
    let ia = [r.itypea1, r.itypea2, r.itypea3].map(|v| v as i16);
    let ib = [r.itypeb1, r.itypeb2, r.itypeb3].map(|v| v as i16);
    let ea = [r.etypea1, r.etypea2].map(|v| v as i16);
    let eb = [r.etypeb1, r.etypeb2].map(|v| v as i16);
    if ea[0] <= 0 && ia[0] <= 0 {
        return true;
    }
    let (mut a, mut b) = (h.u.at(4), h.u.at(5));
    if matches!(h.skill, 4 | 5) {
        let w = h.u.weapon_in_use();
        if w.is_some() && a.map(|x| x.key) == w {
            a = None;
        } else if w.is_some() && b.map(|x| x.key) == w {
            b = None;
        }
    }
    let is = |x: ItemRow, t: i16| h.u.item_is_row(x, i32::from(t));
    let is_key = |k: UnitKey, t: i16| h.u.row(k).is_some_and(|x| is(x, t));
    let listed = |t: &[i16]| t.iter().copied().take_while(|&t| t > 0).collect::<Vec<_>>();
    let skill = u16::try_from(h.skill).unwrap_or(u16::MAX);
    let hand = |it: &[i16], et: &[i16], x: Option<BodyItem>, y: Option<BodyItem>| match x {
        None => {
            if et[0] <= 0 && it[0] <= 0 {
                return true;
            }
            [45, 46, 67].contains(&ia[0]) && ib[0] <= 0 && !y.is_some_and(|y| is(y.row, 45))
        }
        Some(x) => {
            if listed(et).into_iter().any(|t| is(x.row, t)) {
                return false;
            }
            // No type listed passes without the matched item's rules.
            let want = listed(it);
            if want.is_empty() {
                return true;
            }
            want.into_iter().any(|t| is(x.row, t))
                && crate::bridge::use_state::matched_item_ok(
                    h.u.inner.world,
                    skill,
                    r.noammo,
                    x.key,
                    y.map(|y| y.key),
                    is_key,
                )
        }
    };
    if hand(&ia, &ea, a, b) {
        hand(&ib, &eb, b, a)
    } else {
        hand(&ib, &eb, a, b) && hand(&ia, &ea, b, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> DescTables {
        // itemtypes: 0 none, 1 `weap` parent, 2 `shld`, 3 child of 1 (throwable).
        DescTables {
            itypes: vec![
                ItypeRow {
                    equiv1: 0xFFFF,
                    equiv2: 0xFFFF,
                    throwable: false,
                },
                ItypeRow {
                    equiv1: 0xFFFF,
                    equiv2: 0xFFFF,
                    throwable: false,
                },
                ItypeRow {
                    equiv1: 0xFFFF,
                    equiv2: 0xFFFF,
                    throwable: false,
                },
                ItypeRow {
                    equiv1: 1,
                    equiv2: 0xFFFF,
                    throwable: true,
                },
            ],
            ..DescTables::default()
        }
    }

    #[test]
    fn itype_nesting_follows_equiv_links() {
        let t = tables();
        assert!(t.itype_is(3, 1));
        assert!(t.itype_is(3, 3));
        assert!(!t.itype_is(1, 3));
        assert!(!t.itype_is(2, 1));
        assert!(t.throwable(3) && !t.throwable(1));
    }

    /// w1-w1-char-desc-throw-potion: a blank link reads 0 and row 0 links
    /// to itself; `tpot` (38) → `thro` (48) → `weap` (45) is still found.
    #[test]
    fn a_self_linked_row_0_does_not_hide_the_chain() {
        let row = |e1, e2| ItypeRow {
            equiv1: e1,
            equiv2: e2,
            throwable: false,
        };
        let mut itypes = vec![row(0, 0); 50];
        itypes[38] = row(48, 0);
        itypes[48] = row(45, 0);
        let t = DescTables {
            itypes,
            ..DescTables::default()
        };
        assert!(t.itype_is(38, 45));
        assert!(!t.itype_is(38, 46));
    }

    /// Test 10 of the use state: the
    /// player under state 121 with a `delay` skill is 8, else 0.
    #[test]
    fn cooldown_state_121_with_delay_is_8() {
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let mut row: Skills = blank();
        row.ingame = true;
        row.delay = u32::MAX;
        let mut delayed = row.clone();
        delayed.delay = 0;
        let t = SkillTables {
            skills: vec![blank(), row, delayed],
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, 1, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        };
        let mut w = ClientWorld::default();
        let key = UnitKey::new(crate::bridge::world::PLAYER, 1);
        w.units
            .insert(key, crate::bridge::world::ClientUnit::new(key));
        let desc = DescTables::default();
        let code = |w: &ClientWorld, skill: i32| {
            let mut units = DescUnits::new(w, key, &desc);
            let mut h = Hand {
                u: &mut units,
                t: &t,
                code: &[],
                key,
                skill,
                lvl: 1,
            };
            hand_use_code(&mut h)
        };
        assert_eq!((code(&w, 1), code(&w, 2)), (0, 0));
        w.units.get_mut(&key).unwrap().states.insert(121);
        assert_eq!((code(&w, 1), code(&w, 2)), (0, 8));
    }

    #[test]
    fn two_ranges_pick_lines_as_2_2() {
        let one = DamageDraw::Two {
            a: (0, 0),
            ca: 1,
            b: (3, 3),
            cb: 9,
        };
        // A both zero: B alone (max:= min + 1).
        assert_eq!(one.lines(-6), vec![("3-4".to_string(), 9, 0, false)]);
        let both = DamageDraw::Two {
            a: (5, 5),
            ca: 0,
            b: (7, 9),
            cb: 3,
        };
        // Without rule 1: min = max prints alone.
        assert_eq!(
            both.lines(-6),
            vec![
                ("5".to_string(), 0, -6, true),
                ("7-9".to_string(), 3, 2, true)
            ]
        );
    }

    /// Three Tiger
    /// Strike charges (state 122 `progressive_damage`: stat 318 = 3,
    /// 350 = 254, 351 = 20) give the charge bonus 3 × (100 + 19 × 20) =
    /// 1440; a state not in the `pgsv` list, or n = 0, adds nothing.
    #[test]
    fn three_tiger_strike_charges_bonus() {
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let mut skills: Vec<Skills> = (0..256).map(|_| blank()).collect();
        skills[254].prgdam = 1;
        skills[254].param1 = 100;
        skills[254].param2 = 20;
        skills[254].aurastat1 = 318;
        let t = SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, 1, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        };
        let mut w = ClientWorld::default();
        let key = UnitKey::new(crate::bridge::world::PLAYER, 1);
        let mut u = crate::bridge::world::ClientUnit::new(key);
        u.states.insert(122);
        u.state_lists.insert(
            122,
            [
                ((318, 0), 3),
                ((325, 0), 150),
                ((350, 0), 254),
                ((351, 0), 20),
            ]
            .into_iter()
            .collect(),
        );
        w.units.insert(key, u);
        let bonus = |pgsv: Vec<u8>| {
            let desc = DescTables {
                pgsv_states: pgsv,
                ..DescTables::default()
            };
            let mut units = DescUnits::new(&w, key, &desc);
            let h = Hand {
                u: &mut units,
                t: &t,
                code: &[],
                key,
                skill: 255,
                lvl: 20,
            };
            h.charge_bonus()
        };
        assert_eq!(bonus(vec![122]), 1440);
        assert_eq!(bonus(vec![123]), 0);
    }
}
