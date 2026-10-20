//! Skill levels, special values, skill damage, mana cost, to-hit and
//! learning, and the skills and missiles formula contexts that read them.
//! Integer rules: signed 32-bit, wrapping, truncating division.

use super::calc::{self, CalcContext};
use super::special::{MissSpecial, SkillSpecial, MISS_SPECIALS, SKILL_SPECIALS};
use super::{SkillEntry, SkillTables, SkillUnits};
use crate::combat::pct;
use crate::rules::Ruleset;
use crate::units::UnitType;
use d2_data::calc::Family;
use d2_data::tables::Skills;

/// "No formula" (a calc field of 0xFFFFFFFF).
pub const NO_CALC: u32 = 0xFFFF_FFFF;

// Stat ids (itemstatcost).
const STR: u16 = 0;
const ENERGY: u16 = 1;
const DEX: u16 = 2;
const VIT: u16 = 3;
const LIFE: u16 = 6;
const MANA: u16 = 8;
const LEVEL: u16 = 12;
const MINDAMAGE: u16 = 21;
const DAMAGEPERCENT: u16 = 25;
const ADDCLASSSKILLS: u16 = 83;
const NONCLASSSKILL: u16 = 97;
const SINGLESKILL: u16 = 107;
const ELEMSKILL: u16 = 126;
const ALLSKILLS: u16 = 127;
const ADDSKILL_TAB: u16 = 188;
const MAXDAMAGE_PERCENT: u16 = 17;
const KICKDAMAGE: u16 = 137;

// States.
const STATE_CONCENTRATION: u16 = 42;
const STATE_BLOOD_MANA: u16 = 114;
const STATE_SHRINE_SKILL: u16 = 134;

/// i32 view of a u32 record field.
fn s32(v: u32) -> i32 {
    v as i32
}

/// i32 view of an i16 record field stored as u16.
fn s16(v: u16) -> i32 {
    i32::from(v as i16)
}

// ---------------------------------------------------------------

/// `skill_level(unit, entry, bonus)`.
pub fn skill_level<W: SkillUnits>(
    w: &W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    entry: Option<&SkillEntry>,
    bonus: bool,
) -> i32 {
    let (Some(u), Some(e)) = (unit, entry) else {
        return 0;
    };
    let player = (w.unit_type(u) == UnitType::Player).then(|| w.class_id(u));
    skill_level_of(
        t,
        e,
        bonus,
        player,
        w.has_state(u, STATE_SHRINE_SKILL),
        |stat, layer| w.stat(u, stat, layer),
    )
}

/// [`skill_level`] on the unit's facts (the arguments of
/// [`bonus_level_of`]): base + (with `bonus` and a native entry) the
/// bonus level, cut to `[0, level cap]`.
pub fn skill_level_of(
    t: &SkillTables,
    e: &SkillEntry,
    bonus: bool,
    player: Option<i32>,
    shrine: bool,
    stat: impl Fn(u16, u16) -> i32,
) -> i32 {
    let mut l = e.base;
    if bonus && e.is_native() {
        l = l.wrapping_add(bonus_level_of(
            t,
            (e.skill, e.base, e.level_bonus),
            player,
            shrine,
            stat,
        ));
    }
    l.clamp(0, t.level_cap.max(0))
}

/// `bonus_level(unit, skill)`, for the skill of
/// `entry` (whose +0x2C level bonus it adds).
///
/// Panics when a class skill of the player's class has no skilldesc
/// record: the game's own rules fault there.
pub fn bonus_level<W: SkillUnits>(w: &W, t: &SkillTables, u: W::Unit, entry: &SkillEntry) -> i32 {
    let player = (w.unit_type(u) == UnitType::Player).then(|| w.class_id(u));
    bonus_level_of(
        t,
        (entry.skill, entry.base, entry.level_bonus),
        player,
        w.has_state(u, STATE_SHRINE_SKILL),
        |stat, layer| w.stat(u, stat, layer),
    )
}

/// [`bonus_level`] on the unit's facts: the entry's (skill, base level,
/// +0x2C level bonus), `player` the class of a player unit (`None` for
/// another unit), `shrine` state 134 on, `stat` the unit's stat total
/// (stat, layer).
pub fn bonus_level_of(
    t: &SkillTables,
    (skill, base, level_bonus): (i32, i32, i32),
    player: Option<i32>,
    shrine: bool,
    stat: impl Fn(u16, u16) -> i32,
) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return 0;
    };
    let layer = skill as u16;
    let shrine = if shrine { 2 } else { 0 };
    let mut b = level_bonus
        .wrapping_add(shrine)
        .wrapping_add(stat(ALLSKILLS, 0));
    let charclass = i32::from(rec.charclass as i8);
    if let Some(class) = player {
        if class == charclass {
            b = b.wrapping_add(stat(ADDCLASSSKILLS, class as u16));
            let desc = t
                .skilldesc_of(rec)
                .expect("class skill without a skilldesc record (levels.md Edge case 4)");
            let page = i32::from(desc.skillpage as i8);
            if page != 0 {
                let tab = page.wrapping_add(8 * class).wrapping_sub(1);
                b = b.wrapping_add(stat(ADDSKILL_TAB, tab as u16));
            }
            b = b.wrapping_add(stat(NONCLASSSKILL, layer).min(3));
        } else {
            b = b.wrapping_add(stat(NONCLASSSKILL, layer));
        }
    } else if base <= 0 {
        b = b.wrapping_add(stat(NONCLASSSKILL, layer));
    } else {
        let n = stat(NONCLASSSKILL, layer);
        if n != 0 {
            b = b.wrapping_add(n.min(3));
        }
    }
    if rec.etype != 0 {
        b = b.wrapping_add(stat(ELEMSKILL, u16::from(rec.etype)));
    }
    b.wrapping_add(stat(SINGLESKILL, layer))
}

/// `highest_entry(unit, skill)`: the entry the
/// formula `skill` and `blvl` read.
pub fn highest_entry(list: &[SkillEntry], skill: i32) -> Option<SkillEntry> {
    let mut cur: Option<SkillEntry> = None;
    for e in list.iter().filter(|e| e.skill == skill && !e.has_charges) {
        cur = Some(match cur {
            None => *e,
            Some(_) if e.is_native() => *e,
            Some(c) if c.is_native() => c,
            Some(c) if e.base > c.base => *e,
            Some(c) => c,
        });
    }
    cur
}

// ---------------------------------------------------------------

/// `DM(lvl, a, b)`. Panics on `lvl = −6` (division by
/// zero; the game's own rules fault there).
pub fn dm(lvl: i32, a: i32, b: i32) -> i32 {
    let d = lvl.wrapping_add(6);
    assert!(
        d != 0,
        "DM at level -6 divides by zero (levels.md Edge case 3)"
    );
    let q = 110i32.wrapping_mul(lvl).wrapping_div(d);
    let r = q.wrapping_mul(b.wrapping_sub(a)).wrapping_div(100);
    let v = a.wrapping_add(r);
    if v > b {
        b
    } else {
        v
    }
}

/// `lnXY`: `p + (lvl − 1) × q`, wrapping.
pub fn ln(lvl: i32, p: i32, q: i32) -> i32 {
    p.wrapping_add(lvl.wrapping_sub(1).wrapping_mul(q))
}

/// The skills formula context: a unit (or none, for descriptions), a
/// skill and a level; then a mod game's skills functions from the game's
/// rules.
struct SkillCtx<'a, W: SkillUnits> {
    w: &'a mut W,
    t: &'a SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
}

impl<W: SkillUnits> CalcContext for SkillCtx<'_, W> {
    fn function_count(&self) -> u8 {
        7
    }
    fn arity(&self, index: u8) -> u8 {
        if index == 6 {
            3
        } else {
            2
        }
    }
    fn param(&mut self, c: i32) -> i32 {
        // Parameter callback: the context skill's special
        // value `c` (low 8 bits) at the context level.
        special(self.w, self.t, self.unit, c as u8, self.skill, self.lvl)
    }
    fn call(&mut self, index: u8, a: &[i32]) -> i32 {
        match index {
            0 => a[0].min(a[1]),
            1 => a[0].max(a[1]),
            2 => match self.unit {
                Some(u) => calc::rand(self.w.seed(u), a[0], a[1]),
                // TODO: `rand` with a context
                // but no unit (a skilldesc evaluation) has no seed rule;
                // narrowest reading: no draw, `a` when a ≥ b, else 0.
                None if a[0] >= a[1] => a[0],
                None => 0,
            },
            3 => formula_skill(self.w, self.t, self.unit, a[0], a[1]),
            4 => miss_special(self.w, self.t, None, self.unit, a[1] as u8, a[0], self.lvl),
            5 => match self.unit {
                Some(u) if a[0] >= 0 && a[0] < self.t.stat_count => {
                    self.w.formula_stat(u, a[0] as u16, a[1])
                }
                _ => 0,
            },
            6 => {
                let l = special(self.w, self.t, self.unit, a[1] as u8, self.skill, self.lvl);
                special(self.w, self.t, self.unit, a[2] as u8, a[0], l)
            }
            _ => 0,
        }
    }
    fn mod_calc(&self) -> Option<(Family, Ruleset)> {
        calc::mod_calc(self.w.rules(), Family::Skills)
    }
}

/// `skill(s, c)`: special value `c` of skill `s` at
/// the unit's level in it (with bonuses); level 0 with no unit or entry.
fn formula_skill<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    s: i32,
    c: i32,
) -> i32 {
    let l = match unit {
        Some(u) => {
            let e = highest_entry(&w.skill_list(u), s);
            skill_level(w, t, unit, e.as_ref(), true)
        }
        None => 0,
    };
    special(w, t, unit, c as u8, s, l)
}

/// `eval(unit, field, skill, lvl)`: a skills formula
/// field evaluated in the (unit, skill, level) context. Field −1 or an
/// offset past the buffer gives 0.
pub fn eval_skill<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    field: u32,
    skill: i32,
    lvl: i32,
) -> i32 {
    let code = &t.skills_code;
    let mut ctx = SkillCtx {
        w,
        t,
        unit,
        skill,
        lvl,
    };
    calc::eval(code, field, &mut ctx)
}

/// the evaluator of [`eval_skill`] run on the `skilldesccode` buffer
/// `code` (data tables +0x4C / +0x50) instead of `skillscode`; same
/// context and specials. Field past the buffer gives 0.
pub fn eval_desc<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    code: &[u8],
    unit: Option<W::Unit>,
    field: u32,
    skill: i32,
    lvl: i32,
) -> i32 {
    let mut ctx = SkillCtx {
        w,
        t,
        unit,
        skill,
        lvl,
    };
    calc::eval(code, field, &mut ctx)
}

/// `special(unit, c, skill, lvl)` (`skillcalc.tsv`).
pub fn special<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    c: u8,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return 0;
    };
    let Some(kind) = SKILL_SPECIALS.get(usize::from(c)) else {
        return 0;
    };
    use SkillSpecial as K;
    let guarded = |v: i32| if lvl > 0 { v } else { 0 };
    match *kind {
        K::Ln(p, q) => guarded(ln(lvl, skill_param(rec, p), skill_param(rec, q))),
        K::Dm(p, q, guard) => {
            if guard && lvl <= 0 {
                0
            } else {
                dm(lvl, skill_param(rec, p), skill_param(rec, q))
            }
        }
        K::Par(p) => skill_param(rec, p),
        K::Lvl => lvl,
        K::ElemMin { mastery, shift } => {
            let v = elem_min(w, t, unit, skill, lvl, mastery);
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::ElemMax { mastery, shift } => {
            let v = elem_max(w, t, unit, skill, lvl, mastery);
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::ElemLen => elem_len(w, t, unit, skill, lvl),
        K::ToHit => to_hit(w, t, unit, skill, lvl),
        K::Mana => guarded(mana_cost(rec, lvl) >> 8),
        K::Mps => {
            let n = s16(rec.mana).wrapping_add(s16(rec.lvlmana).wrapping_mul(lvl.wrapping_sub(1)));
            let v = n.wrapping_mul(25) / 2;
            guarded(v.wrapping_shl(u32::from(rec.manashift as u8)) >> 8)
        }
        K::Mastery(ty) => mastery(w, t, unit, rec, skill, lvl, ty),
        K::MissElem { slot, max, shift } => {
            if lvl <= 0 {
                return 0;
            }
            let Some(m) = desc_missile(t, rec, slot) else {
                return 0;
            };
            let v = if max {
                miss_elem_max(w, t, None, unit, m, lvl)
            } else {
                miss_elem_min(w, t, None, unit, m, lvl)
            };
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::MissElemLen { slot } => {
            if lvl <= 0 {
                return 0;
            }
            match desc_missile(t, rec, slot) {
                Some(m) => miss_elem_len(w, t, None, m, lvl),
                None => 0,
            }
        }
        K::MissRange { slot } => match desc_missile(t, rec, slot).and_then(|m| t.missile(m)) {
            Some(m) => s16(m.range).wrapping_add(lvl.wrapping_mul(s16(m.levrange))),
            None => 0,
        },
        K::ULvl => unit.map_or(0, |u| w.stat(u, LEVEL, 0)),
        K::BLvl => match unit {
            Some(u) => {
                let e = highest_entry(&w.skill_list(u), skill);
                skill_level(w, t, unit, e.as_ref(), false)
            }
            None => 0,
        },
        K::Usmc => guarded(mana_cost(rec, lvl)),
        K::Eval(f) => {
            let field = f.of(rec);
            eval_skill(w, t, unit, field, skill, lvl)
        }
    }
}

/// `ParamN` (1-based) of a skills record, as i32.
fn skill_param(rec: &Skills, n: u8) -> i32 {
    s32(match n {
        1 => rec.param1,
        2 => rec.param2,
        3 => rec.param3,
        4 => rec.param4,
        5 => rec.param5,
        6 => rec.param6,
        7 => rec.param7,
        _ => rec.param8,
    })
}

/// `descmissile{slot+1}` of the skill's skilldesc record; `None` when the
/// skilldesc record or the missile is missing.
fn desc_missile(t: &SkillTables, rec: &Skills, slot: u8) -> Option<i32> {
    let d = t.skilldesc_of(rec)?;
    let m = match slot {
        0 => d.descmissile1,
        1 => d.descmissile2,
        _ => d.descmissile3,
    };
    let m = i32::from(m as i16);
    t.missile(m).map(|_| m)
}

/// `mastery(type)`.
fn mastery<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    rec: &Skills,
    skill: i32,
    lvl: i32,
    ty: u8,
) -> i32 {
    if lvl <= 0 {
        return 0;
    }
    let (a, b) = match ty {
        0 => (342, 345),
        1 => (343, 346),
        _ => (344, 347),
    };
    let pairs = [
        (rec.passivestat1, rec.passivecalc1),
        (rec.passivestat2, rec.passivecalc2),
        (rec.passivestat3, rec.passivecalc3),
        (rec.passivestat4, rec.passivecalc4),
        (rec.passivestat5, rec.passivecalc5),
    ];
    for (stat, field) in pairs {
        let s = s16(stat);
        if s == a || s == b {
            return eval_skill(w, t, unit, field, skill, lvl);
        }
    }
    0
}

/// The missiles formula context: the missile unit, its owner, the
/// missile class and level; then a mod game's missiles functions from the
/// game's rules.
struct MissCtx<'a, W: SkillUnits> {
    w: &'a mut W,
    t: &'a SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    missile: i32,
    lvl: i32,
}

impl<W: SkillUnits> CalcContext for MissCtx<'_, W> {
    fn function_count(&self) -> u8 {
        4
    }
    fn arity(&self, _index: u8) -> u8 {
        2
    }
    fn param(&mut self, c: i32) -> i32 {
        miss_special(
            self.w,
            self.t,
            self.missile_unit,
            self.owner,
            c as u8,
            self.missile,
            self.lvl,
        )
    }
    fn call(&mut self, index: u8, a: &[i32]) -> i32 {
        match index {
            0 => a[0].min(a[1]),
            1 => a[0].max(a[1]),
            // TODO: missile `rand` has no settled
            // seed; validated buffers contain none (policy 6). No draw.
            2 => 0,
            _ => formula_skill(self.w, self.t, self.owner, a[0], a[1]),
        }
    }
    fn mod_calc(&self) -> Option<(Family, Ruleset)> {
        calc::mod_calc(self.w.rules(), Family::Missiles)
    }
}

/// Missiles evaluator.
pub fn eval_missile<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    field: u32,
    missile: i32,
    lvl: i32,
) -> i32 {
    // With a unit: not a missile → 0; class < 0 → the unit's class;
    // lvl < 0 → the missile's stored level (data +0x0C).
    let (missile, lvl) = match missile_unit {
        Some(m) if w.unit_type(m) != UnitType::Missile => return 0,
        Some(m) => (
            if missile < 0 { w.class_id(m) } else { missile },
            if lvl < 0 { w.missile_level(m) } else { lvl },
        ),
        None => (missile, lvl),
    };
    let code = &t.miss_code;
    let mut ctx = MissCtx {
        w,
        t,
        missile_unit,
        owner,
        missile,
        lvl,
    };
    calc::eval(code, field, &mut ctx)
}

/// `miss_special(missile_unit, owner, c, missile, lvl)`
/// (`misscalc.tsv`).
pub fn miss_special<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    c: u8,
    missile: i32,
    lvl: i32,
) -> i32 {
    if let Some(m) = missile_unit {
        if w.unit_type(m) != UnitType::Missile {
            return 0;
        }
    }
    let Some(rec) = t.missile(missile) else {
        return 0;
    };
    let Some(kind) = MISS_SPECIALS.get(usize::from(c)) else {
        return 0;
    };
    use MissSpecial as K;
    match *kind {
        K::Par(f) => f.of(rec),
        K::Lvl => lvl,
        K::ElemMin { shift } => {
            let v = miss_elem_min(w, t, missile_unit, owner, missile, lvl);
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::ElemMax { shift } => {
            let v = miss_elem_max(w, t, missile_unit, owner, missile, lvl);
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::ElemLen => miss_elem_len(w, t, missile_unit, missile, lvl),
        K::PhysMin { shift } => {
            let v = miss_phys_min(w, t, missile_unit, owner, missile, lvl);
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::PhysMax { shift } => {
            let v = miss_phys_max(w, t, missile_unit, owner, missile, lvl);
            if shift {
                v >> 8
            } else {
                v
            }
        }
        K::Range => s16(rec.range).wrapping_add(lvl.wrapping_mul(s16(rec.levrange))),
        K::Ln(p, q) => ln(lvl, p.of(rec), q.of(rec)),
        K::Dm(p, q) => dm(lvl, p.of(rec), q.of(rec)),
    }
}

// ---------------------------------------------------------------

/// `bracket(lvl, L1…L5)`.
pub fn bracket(lvl: i32, l: [i32; 5]) -> i32 {
    let m = |a: i32, b: i32| a.wrapping_mul(b);
    let s1 = m(7, l[0]);
    let s2 = s1.wrapping_add(m(8, l[1]));
    let s3 = s2.wrapping_add(m(6, l[2]));
    let s4 = s3.wrapping_add(m(6, l[3]));
    match lvl {
        i32::MIN..=1 => 0,
        2..=8 => m(lvl - 1, l[0]),
        9..=16 => s1.wrapping_add(m(lvl - 8, l[1])),
        17..=22 => s2.wrapping_add(m(lvl - 16, l[2])),
        23..=28 => s3.wrapping_add(m(lvl - 22, l[3])),
        _ => s4.wrapping_add(m(lvl - 28, l[4])),
    }
}

/// The 8/16 length brackets.
fn len_bracket(lvl: i32, l: [i32; 3]) -> i32 {
    let m = |a: i32, b: i32| a.wrapping_mul(b);
    if lvl <= 8 {
        m(lvl - 1, l[0])
    } else if lvl <= 16 {
        m(7, l[0]).wrapping_add(m(lvl - 8, l[1]))
    } else {
        m(7, l[0])
            .wrapping_add(m(8, l[1]))
            .wrapping_add(m(lvl.wrapping_sub(16), l[2]))
    }
}

/// Elemental mastery stat by `EType` (a byte table).
pub fn elem_mastery_stat(etype: u8) -> Option<u16> {
    match etype {
        1 => Some(329),
        2 => Some(330),
        4 | 12 => Some(331),
        5 => Some(332),
        _ => None,
    }
}

/// Shared body of `elem_min` / `elem_max`.
fn elem<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
    mastery: bool,
    max: bool,
) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return 0;
    };
    if lvl <= 0 {
        return 0;
    }
    let (base, levs) = if max {
        (
            rec.emax,
            [
                rec.emaxlev1,
                rec.emaxlev2,
                rec.emaxlev3,
                rec.emaxlev4,
                rec.emaxlev5,
            ],
        )
    } else {
        (
            rec.emin,
            [
                rec.eminlev1,
                rec.eminlev2,
                rec.eminlev3,
                rec.eminlev4,
                rec.eminlev5,
            ],
        )
    };
    let mut v = s32(base)
        .wrapping_add(bracket(lvl, levs.map(s32)))
        .wrapping_shl(u32::from(rec.hitshift));
    // The min's gate is tested before the formula: a gated-out min does
    // not evaluate it (no `rand` draw).
    if rec.edmgsympercalc != NO_CALC && (max || v > 256 || rec.eminlev1 != 0) {
        let p = eval_skill(w, t, unit, rec.edmgsympercalc, skill, lvl);
        if p != 0 {
            v = v.wrapping_add(pct(v, p, 100));
        }
    }
    if mastery {
        if let (Some(u), Some(stat)) = (unit, elem_mastery_stat(rec.etype)) {
            // Unit getter.
            v = v.wrapping_add(pct(v, w.stat(u, stat, 0), 100));
        }
    }
    v
}

/// `elem_min(unit, skill, lvl, mastery)`, 1/256
/// points.
pub fn elem_min<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
    mastery: bool,
) -> i32 {
    elem(w, t, unit, skill, lvl, mastery, false)
}

/// `elem_max(unit, skill, lvl, mastery)`.
pub fn elem_max<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
    mastery: bool,
) -> i32 {
    elem(w, t, unit, skill, lvl, mastery, true)
}

/// `elem_len(unit, skill, lvl)`, frames.
pub fn elem_len<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return 0;
    };
    if lvl <= 0 {
        return 0;
    }
    let l = len_bracket(lvl, [rec.elevlen1, rec.elevlen2, rec.elevlen3].map(s32));
    let mut v = s32(rec.elen).wrapping_add(l);
    if rec.elensympercalc != NO_CALC {
        let p = eval_skill(w, t, unit, rec.elensympercalc, skill, lvl);
        if p != 0 {
            v = v.wrapping_add(pct(v, p, 100));
        }
    }
    v
}

/// `phys_min` / `phys_max`.
fn phys<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
    use_srcdam: bool,
    max: bool,
) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return if max { 2 } else { 1 };
    };
    if rec.kick {
        let x = match unit {
            Some(u) if w.unit_type(u) == UnitType::Player => w
                .stat(u, DEX, 0)
                .wrapping_add(w.stat(u, STR, 0))
                .wrapping_sub(20),
            Some(u) => 3i32.wrapping_mul(w.stat(u, LEVEL, 0)),
            None => 0,
        }
        .max(1);
        return (if max { x / 3 } else { x / 4 }).wrapping_shl(8);
    }
    let mut s = 0i32;
    if rec.srcdam != 0 && use_srcdam {
        let wv = match unit {
            Some(u) => match w.weapon(u) {
                Some(item) if w.wield_type(item) == 2 => w.item_damage(item, max),
                _ => w.stat(u, if max { MINDAMAGE + 1 } else { MINDAMAGE }, 0),
            },
            None => 0,
        };
        s = i32::from(rec.srcdam).wrapping_mul(wv) / 128;
    }
    let (base, levs) = if max {
        (
            rec.maxdam,
            [
                rec.maxlevdam1,
                rec.maxlevdam2,
                rec.maxlevdam3,
                rec.maxlevdam4,
                rec.maxlevdam5,
            ],
        )
    } else {
        (
            rec.mindam,
            [
                rec.minlevdam1,
                rec.minlevdam2,
                rec.minlevdam3,
                rec.minlevdam4,
                rec.minlevdam5,
            ],
        )
    };
    let mut v = s
        .wrapping_add(s32(base))
        .wrapping_add(bracket(lvl, levs.map(s32)));
    if rec.dmgsympercalc != NO_CALC {
        let p = eval_skill(w, t, unit, rec.dmgsympercalc, skill, lvl);
        if p != 0 {
            v = v.wrapping_add(pct(v, p, 100));
        }
    }
    v.wrapping_shl(u32::from(rec.hitshift))
}

/// `phys_min(unit, skill, lvl, use_srcdam)`.
pub fn phys_min<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
    use_srcdam: bool,
) -> i32 {
    phys(w, t, unit, skill, lvl, use_srcdam, false)
}

/// `phys_max(unit, skill, lvl, use_srcdam)`.
pub fn phys_max<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
    use_srcdam: bool,
) -> i32 {
    phys(w, t, unit, skill, lvl, use_srcdam, true)
}

/// Missile id and level after the unit rules of the missile pick; `None` = result 0.
fn miss_args<W: SkillUnits>(
    w: &W,
    missile_unit: Option<W::Unit>,
    missile: i32,
    lvl: i32,
) -> Option<(i32, i32)> {
    match missile_unit {
        Some(m) if w.unit_type(m) != UnitType::Missile => None,
        Some(m) => {
            let id = if missile < 0 { w.class_id(m) } else { missile };
            let l = if lvl <= 0 { w.missile_level(m) } else { lvl };
            Some((id, l))
        }
        None => Some((missile, lvl)),
    }
}

/// Shared body of the missile damage functions.
#[allow(clippy::too_many_arguments)]
fn miss_dmg<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    missile: i32,
    lvl: i32,
    elemental: bool,
    max: bool,
) -> i32 {
    let Some((id, lvl)) = miss_args(w, missile_unit, missile, lvl) else {
        return 0;
    };
    let Some(rec) = t.missile(id) else {
        return 0;
    };
    let (base, levs, sym): (u32, [u32; 5], u32) = match (elemental, max) {
        (false, false) => (
            rec.mindamage,
            [
                rec.minlevdam1,
                rec.minlevdam2,
                rec.minlevdam3,
                rec.minlevdam4,
                rec.minlevdam5,
            ],
            rec.dmgsympercalc,
        ),
        (false, true) => (
            rec.maxdamage,
            [
                rec.maxlevdam1,
                rec.maxlevdam2,
                rec.maxlevdam3,
                rec.maxlevdam4,
                rec.maxlevdam5,
            ],
            rec.dmgsympercalc,
        ),
        (true, false) => (
            rec.emin,
            [
                rec.minelev1,
                rec.minelev2,
                rec.minelev3,
                rec.minelev4,
                rec.minelev5,
            ],
            rec.edmgsympercalc,
        ),
        (true, true) => (
            rec.emax,
            [
                rec.maxelev1,
                rec.maxelev2,
                rec.maxelev3,
                rec.maxelev4,
                rec.maxelev5,
            ],
            rec.edmgsympercalc,
        ),
    };
    let shift = u32::from(rec.hitshift);
    let mut v = s32(base).wrapping_add(bracket(lvl, levs.map(s32)));
    if sym != NO_CALC {
        let p = eval_missile(w, t, missile_unit, owner, sym, id, lvl);
        v = v.wrapping_add(pct(v, p, 100));
    }
    v.wrapping_shl(shift)
}

/// Missile physical min, 1/256 points.
pub fn miss_phys_min<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    missile: i32,
    lvl: i32,
) -> i32 {
    miss_dmg(w, t, missile_unit, owner, missile, lvl, false, false)
}

/// Missile physical max.
pub fn miss_phys_max<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    missile: i32,
    lvl: i32,
) -> i32 {
    miss_dmg(w, t, missile_unit, owner, missile, lvl, false, true)
}

/// Missile elemental min.
pub fn miss_elem_min<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    missile: i32,
    lvl: i32,
) -> i32 {
    miss_dmg(w, t, missile_unit, owner, missile, lvl, true, false)
}

/// Missile elemental max.
pub fn miss_elem_max<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    owner: Option<W::Unit>,
    missile: i32,
    lvl: i32,
) -> i32 {
    miss_dmg(w, t, missile_unit, owner, missile, lvl, true, true)
}

/// Missile elemental length, frames: `lvl ≤ 0` →
/// `ELen`; no synergy.
pub fn miss_elem_len<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    missile_unit: Option<W::Unit>,
    missile: i32,
    lvl: i32,
) -> i32 {
    let Some((id, lvl)) = miss_args(w, missile_unit, missile, lvl) else {
        return 0;
    };
    let Some(rec) = t.missile(id) else {
        return 0;
    };
    if lvl <= 0 {
        return s32(rec.elen);
    }
    s32(rec.elen).wrapping_add(len_bracket(
        lvl,
        [rec.elevlen1, rec.elevlen2, rec.elevlen3].map(s32),
    ))
}

/// `weapon_mastery(unit, item, skill, type)`: type
/// 0 to-hit, 1 damage, 2 critical strike. `skill = None` reads the used
/// skill.
pub fn weapon_mastery<W: SkillUnits>(
    w: &W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    item: Option<W::Item>,
    skill: Option<i32>,
    ty: u8,
) -> i32 {
    let (Some(u), Some(item)) = (unit, item) else {
        return 0;
    };
    let skill = skill.or_else(|| w.used_skill(u).map(|e| e.skill));
    let stat = if throw_gate(w, t, item, skill) {
        345
    } else {
        342
    } + u16::from(ty.min(2));
    mastery_of(w, u, item, stat)
}

/// The gate of the throw path (the throw mastery's first
/// test): the item is of type `throwable`, the skill
/// exists, its `itypea1` is-a 48 (`thro`) and its range is 2.
fn throw_gate<W: SkillUnits>(w: &W, t: &SkillTables, item: W::Item, skill: Option<i32>) -> bool {
    w.item_flag_throw(item)
        && skill.and_then(|s| t.skill(s)).is_some_and(|r| {
            r.range == 2 && r.itypea1 != 0xFFFF && w.itype_is(i32::from(r.itypea1), 48)
        })
}

/// Max(0, the values of up to 32 entries of `stat` whose layer is an item
/// type the item is).
fn mastery_of<W: SkillUnits>(w: &W, u: W::Unit, item: W::Item, stat: u16) -> i32 {
    w.stat_entries(u, stat, 32)
        .into_iter()
        .filter(|&(layer, _)| w.item_is(item, i32::from(layer)))
        .fold(0, |m, (_, v)| m.max(v))
}

/// Throw mastery `(unit, item, skill, type)` (Open
/// question 8): 0 unless the throw gate passes (the
/// item is `throwable`, the used skill has `itypea1` `thro` and range 2);
/// then stats 345 / 346 / 347 as in [`weapon_mastery`]. Null unit or
/// item → 0; null skill → the used skill.
pub fn throw_mastery<W: SkillUnits>(
    w: &W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    item: Option<W::Item>,
    skill: Option<i32>,
    ty: u8,
) -> i32 {
    let (Some(u), Some(item)) = (unit, item) else {
        return 0;
    };
    let skill = skill.or_else(|| w.used_skill(u).map(|e| e.skill));
    if !throw_gate(w, t, item, skill) {
        return 0;
    }
    mastery_of(w, u, item, 345 + u16::from(ty.min(2)))
}

/// Concentration: with state 42, `(damagepercent of
/// that state's list × Param1 of skill) / 8`; else 0.
pub fn concentration<W: SkillUnits>(w: &W, t: &SkillTables, unit: W::Unit, skill: i32) -> i32 {
    if !w.has_state(unit, STATE_CONCENTRATION) {
        return 0;
    }
    let dp = w
        .state_stat(unit, STATE_CONCENTRATION, DAMAGEPERCENT)
        .unwrap_or(0);
    let p1 = t.skill(skill).map_or(0, |r| s32(r.param1));
    dp.wrapping_mul(p1) / 8
}

/// The item reads of kick damage that belong to the items and
/// stat-lists specs (a seam). Provider: items / stat-lists implementation.
pub trait KickItems: SkillUnits {
    /// Toggle the weapons' stat lists off (`false`) or merge
    /// only the left weapon's list back (`true`, Edge case 6).
    fn toggle_weapon_lists(&mut self, u: Self::Unit, on: bool);
    /// Items record bytes +0xFE / +0xFF of the boots (kick min / max).
    fn boots_damage(&self, item: Self::Item) -> (i32, i32);
}

/// Kick damage.
pub fn kick_damage<W: KickItems>(
    w: &mut W,
    u: W::Unit,
    min: &mut i32,
    max: &mut i32,
    pc: &mut i32,
) {
    let k = w.item_stat(u, KICKDAMAGE, 0);
    *min = min.wrapping_add(k);
    *max = max.wrapping_add(k);
    w.toggle_weapon_lists(u, false);
    if let Some(boots) = w.item_at(u, 9) {
        let (bmin0, bmax0) = w.boots_damage(boots);
        let bmin = k.wrapping_add(bmin0);
        let mut bmax = k.wrapping_add(bmax0);
        let (sb, db) = w.str_dex_bonus(boots);
        let p = pct(w.stat(u, STR, 0), sb, 100)
            .wrapping_add(pct(w.stat(u, DEX, 0), db, 100))
            .wrapping_add(w.stat(u, DAMAGEPERCENT, 0))
            .max(-90);
        if bmin >= bmax {
            bmax = bmin;
        }
        *pc = pc
            .wrapping_add(w.stat(u, MAXDAMAGE_PERCENT, 0))
            .wrapping_add(p);
        *min = min.wrapping_add(bmin);
        *max = max.wrapping_add(bmax);
    }
    w.toggle_weapon_lists(u, true);
}

/// `roll_physical(unit, record, skill, lvl)`: adds
/// `a + roll(b − a)` to the record's physical damage (unit seed).
pub fn roll_physical<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: W::Unit,
    record: &mut crate::combat::DamageRecord,
    skill: i32,
    lvl: i32,
) {
    let a = phys_min(w, t, Some(unit), skill, lvl, false);
    let b = phys_max(w, t, Some(unit), skill, lvl, false);
    let r = w.seed(unit).roll(b.wrapping_sub(a)) as i32;
    record.physical = record.physical.wrapping_add(a.wrapping_add(r));
}

/// `roll_elemental(unit, record, skill, lvl)`:
/// evaluates `elem_len`, `elem_min`, `elem_max` (mastery on) in that
/// order, draws `roll(b − a)` and puts `v` and `len` into the record by
/// `EType` ([`add_element`]). Returns `v`.
pub fn roll_elemental<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: W::Unit,
    record: &mut crate::combat::DamageRecord,
    skill: i32,
    lvl: i32,
) -> i32 {
    let len = elem_len(w, t, Some(unit), skill, lvl);
    let a = elem_min(w, t, Some(unit), skill, lvl, true);
    let b = elem_max(w, t, Some(unit), skill, lvl, true);
    let v = a.wrapping_add(w.seed(unit).roll(b.wrapping_sub(a)) as i32);
    let etype = t.skill(skill).map_or(0, |r| i32::from(r.etype));
    add_element(w, unit, record, etype, v, len);
    v
}

/// What [`add_element`] reports through its two out arguments (`&res`,
/// `&e_out`), plus the element's hit class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementAdded {
    /// The element after the random remap (`e_out`).
    pub element: i32,
    /// The resist stat of the element, −1 for leech, stun, burn and
    /// physical (`res`).
    pub resist: i32,
    /// The hit class the element names (0x20 fire, 0x40 lightning, 0x30
    /// cold and freeze, 0x50 poison, 0x60 stun; 0 otherwise). A non-zero
    /// class is also stored in the record (a plain u32
    /// store to +0x60, replacing the old value); 0 leaves it unchanged.
    pub hit_class: u32,
}

/// `add_element(unit, record, e, v, len, &res, &e_out)`
/// (a jump table): adds `v` (and sets `len`) in the
/// record field of element `e`; `e = 10` is one random element of {1, 2,
/// 4, 5} (one step of the unit seed, `lo' & 3`) with `len ≤ 0` → 50.
pub fn add_element<W: SkillUnits>(
    w: &mut W,
    unit: W::Unit,
    record: &mut crate::combat::DamageRecord,
    e: i32,
    v: i32,
    len: i32,
) -> ElementAdded {
    let (mut e, mut len) = (e, len);
    if e == 10 {
        // Table.
        const RANDOM: [i32; 4] = [1, 2, 4, 5];
        e = RANDOM[(w.seed(unit).step() & 3) as usize];
        if len <= 0 {
            len = 50;
        }
    }
    let r = record;
    let (resist, hit_class) = match e {
        1 => {
            r.fire = r.fire.wrapping_add(v);
            (39, 0x20)
        }
        2 => {
            r.lightning = r.lightning.wrapping_add(v);
            (41, 0x40)
        }
        3 => {
            r.magic = r.magic.wrapping_add(v);
            (37, 0)
        }
        4 => {
            r.cold = r.cold.wrapping_add(v);
            r.cold_len = len;
            (43, 0x30)
        }
        5 => {
            r.poison = r.poison.wrapping_add(v);
            r.poison_len = len;
            (45, 0x50)
        }
        6 => {
            r.life_leech = r.life_leech.wrapping_add(v);
            (-1, 0)
        }
        7 => {
            r.mana_leech = r.mana_leech.wrapping_add(v);
            (-1, 0)
        }
        8 => {
            r.stamina_leech = r.stamina_leech.wrapping_add(v);
            (-1, 0)
        }
        9 => {
            r.stun_len = r.stun_len.wrapping_add(v.wrapping_add(len));
            (-1, 0x60)
        }
        11 => {
            r.burn = r.burn.wrapping_add(v);
            r.burn_len = len;
            (-1, 0)
        }
        12 => {
            r.cold = r.cold.wrapping_add(v);
            r.freeze_len = len;
            (43, 0x30)
        }
        _ => {
            r.physical = r.physical.wrapping_add(v);
            (-1, 0)
        }
    };
    // "hit class h" is a plain store (record +0x60 := h); the
    // elements without one leave +0x60 unchanged.
    if hit_class != 0 {
        r.hit_class = hit_class;
    }
    ElementAdded {
        element: e,
        resist,
        hit_class,
    }
}

// ---------------------------------------------------------------

/// Cost: `(mana + lvlmana × (lvl − 1)) <<
/// manashift`, 1/256 points; no level check, no `minmana`.
pub fn mana_cost(rec: &Skills, lvl: i32) -> i32 {
    s16(rec.mana)
        .wrapping_add(s16(rec.lvlmana).wrapping_mul(lvl.wrapping_sub(1)))
        .wrapping_shl(u32::from(rec.manashift as u8))
}

/// Shifted cost: `max(cost >> 8, 0)`. Panics on an
/// invalid skill (a fatal assertion in the game).
pub fn mana_cost_shifted(t: &SkillTables, skill: i32, lvl: i32) -> i32 {
    let rec = t
        .skill(skill)
        .expect("invalid skill in shifted mana cost (levels.md §4)");
    (mana_cost(rec, lvl) >> 8).max(0)
}

/// The mana effects that belong to other systems (a seam). Provider:
/// units/stats (stat writes) and the skills-use / items implementation.
pub trait ManaUnits: SkillUnits {
    /// Were-form test (for `srvdofunc` 116).
    fn shapeshifted(&self, u: Self::Unit) -> bool;
    /// Charges path; returns the consume result.
    fn consume_charges(&mut self, u: Self::Unit, entry: &SkillEntry) -> bool;
    /// Blood-mana payment.
    // TODO: the blood-mana rule is unspecified; the provider decides.
    fn pay_life(&mut self, u: Self::Unit, cost: i32) -> bool;
    /// Sets a stat's base value (here: mana, stat 8).
    fn set_stat(&mut self, u: Self::Unit, stat: u16, value: i32);
}

/// `can_afford(unit, skill)` for the skill of `entry`.
pub fn can_afford<W: ManaUnits>(w: &W, t: &SkillTables, u: W::Unit, entry: &SkillEntry) -> bool {
    if !entry.is_native() {
        return entry.charges > 0;
    }
    let Some(rec) = t.skill(entry.skill) else {
        return false;
    };
    let n = (skill_level(w, t, Some(u), Some(entry), true) - 1).max(0);
    let c = s16(rec.mana)
        .wrapping_add(s16(rec.lvlmana).wrapping_mul(n))
        .wrapping_shl(u32::from(rec.manashift as u8));
    if w.has_state(u, STATE_BLOOD_MANA) {
        w.stat(u, LIFE, 0) >= c
    } else if rec.srvdofunc == 116 && w.shapeshifted(u) {
        true
    } else {
        w.stat(u, MANA, 0) >= c
    }
}

/// `consume_mana(game, unit, skill, lvl)`. Returns
/// the game's result (`true` = 1).
pub fn consume_mana<W: ManaUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
) -> bool {
    let Some(u) = unit else {
        return true;
    };
    if w.unit_type(u) != UnitType::Player {
        return true;
    }
    if let Some(e) = w.used_skill(u) {
        if e.skill == skill && !e.is_native() {
            return w.consume_charges(u, &e);
        }
    }
    let Some(rec) = t.skill(skill) else {
        return false;
    };
    if rec.mana == 0 && rec.lvlmana == 0 {
        return false;
    }
    let c = s16(rec.mana)
        .wrapping_add(s16(rec.lvlmana).wrapping_mul(lvl.wrapping_sub(1).max(0)))
        .wrapping_shl(u32::from(rec.manashift as u8))
        .max(s16(rec.minmana).wrapping_shl(8));
    if w.has_state(u, STATE_BLOOD_MANA) {
        return w.pay_life(u, c);
    }
    let mana = w.stat(u, MANA, 0);
    if mana < c {
        return false;
    }
    w.set_stat(u, MANA, mana.wrapping_sub(c));
    true
}

// ---------------------------------------------------------------

/// `to_hit(unit, skill, lvl)`: the hit-test bonus.
pub fn to_hit<W: SkillUnits>(
    w: &mut W,
    t: &SkillTables,
    unit: Option<W::Unit>,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return 0;
    };
    if lvl <= 0 {
        return 0;
    }
    if rec.tohitcalc != NO_CALC {
        return eval_skill(w, t, unit, rec.tohitcalc, skill, lvl);
    }
    ln(lvl, s32(rec.tohit), s32(rec.levtohit))
}

// ---------------------------------------------------------------

/// `max_level(skill)`: `maxlvl` if > 0, else 20.
pub fn max_level(rec: &Skills) -> i32 {
    let m = s16(rec.maxlvl);
    if m > 0 {
        m
    } else {
        20
    }
}

/// The native entry of `skill` in a list (first in list order).
fn native_entry(list: &[SkillEntry], skill: i32) -> Option<SkillEntry> {
    list.iter()
        .find(|e| e.skill == skill && e.is_native())
        .copied()
}

/// `req_level(unit, skill)`: `reqlevel` + base of the
/// native entry; a bad id gives `i32::MAX`.
pub fn req_level<W: SkillUnits>(w: &W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    let Some(rec) = t.skill(skill) else {
        return i32::MAX;
    };
    let base = native_entry(&w.skill_list(u), skill).map_or(0, |e| e.base);
    s16(rec.reqlevel).wrapping_add(base)
}

/// Level and required skills.
pub fn meets_skill_reqs<W: SkillUnits>(w: &W, t: &SkillTables, u: W::Unit, skill: i32) -> bool {
    let Some(rec) = t.skill(skill) else {
        return false;
    };
    if w.stat(u, LEVEL, 0) < req_level(w, t, u, skill) {
        return false;
    }
    let list = w.skill_list(u);
    [rec.reqskill1, rec.reqskill2, rec.reqskill3]
        .into_iter()
        .map(s16)
        .filter(|&r| r >= 0)
        .all(|r| t.skill(r).is_some() && native_entry(&list, r).is_some_and(|e| e.base > 0))
}

/// `InGame`, level and attributes.
pub fn meets_attr_reqs<W: SkillUnits>(w: &W, t: &SkillTables, u: W::Unit, skill: i32) -> bool {
    let Some(rec) = t.skill(skill) else {
        return false;
    };
    rec.ingame
        && w.stat(u, LEVEL, 0) >= req_level(w, t, u, skill)
        && s16(rec.reqstr) <= w.stat(u, STR, 0)
        && s16(rec.reqdex) <= w.stat(u, DEX, 0)
        && s16(rec.reqint) <= w.stat(u, ENERGY, 0)
        && s16(rec.reqvit) <= w.stat(u, VIT, 0)
}

/// The skill-point effects that belong to other systems (a seam).
/// Provider: units and stat lists (skill list, stat 5) and the client messages.
pub trait LearnUnits: SkillUnits {
    /// `skill` is a class skill of the player.
    fn is_class_skill(&self, u: Self::Unit, skill: i32) -> bool;
    /// Spends `cost` from `newskills(5)`, adds a level,
    /// refreshes, toggles the passive state and calls
    /// the passive's stat refresh; refunds on failure.
    fn add_skill_level(&mut self, u: Self::Unit, skill: i32, cost: i32);
    /// Message 0x21 (skill 0, base level 1, remove 0) to
    /// the player's client: the client re-assigns Attack at level 1.
    // TODO(wiring): the d2-server handler (`handlers/skills/wired.rs`) does
    // not call `add_skill_point` yet; until it does these stay no-ops there.
    fn send_attack_reset(&mut self, _u: Self::Unit) {}
    /// The handler's tail: the skill and stat updates to the client.
    fn point_client_updates(&mut self, _u: Self::Unit) {}
}

/// Outcome of the 0x3B (add skill point) validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPointCheck {
    Ok,
    /// 2: id out of range, or already at the maximum level.
    Code2,
    /// 3: not a class skill, or a requirement fails.
    Code3,
}

/// Validate a skill-point request.
// TODO: what codes 2/3 send to the client is unspecified.
pub fn check_skill_point<W: LearnUnits>(
    w: &W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
) -> SkillPointCheck {
    let Some(rec) = t.skill(skill) else {
        return SkillPointCheck::Code2;
    };
    if !w.is_class_skill(u, skill)
        || !meets_skill_reqs(w, t, u, skill)
        || !meets_attr_reqs(w, t, u, skill)
    {
        return SkillPointCheck::Code3;
    }
    if let Some(e) = native_entry(&w.skill_list(u), skill) {
        if skill_level(w, t, Some(u), Some(&e), false) >= max_level(rec) {
            return SkillPointCheck::Code2;
        }
    }
    SkillPointCheck::Ok
}

/// Spend a point if `newskills(5)` (base
/// getter) covers the cost. Returns whether a level was added.
pub fn spend_skill_point<W: LearnUnits>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
) -> bool {
    let Some(rec) = t.skill(skill) else {
        return false;
    };
    let cost = if rec.skpoints == NO_CALC {
        1
    } else {
        let e = native_entry(&w.skill_list(u), skill);
        let l = e.map_or(0, |e| skill_level(w, t, Some(u), Some(&e), true));
        eval_skill(w, t, Some(u), rec.skpoints, skill, l)
    };
    if w.base_stat(u, 5, 0) < cost {
        return false;
    }
    w.add_skill_level(u, skill, cost);
    true
}

/// Message 0x3B AddSkillPoint handler: the result is
/// 0 (a level was added), 2 (bad id, at maximum level or a failed spend)
/// or 3 (not a class skill, or a requirement fails). Validator 2 and the
/// maximum-level case send message 0x21; 3 and a failed spend send
/// nothing.
// TODO: whether the tail updates also run after a
// failed spend is not stated; they run only after a successful one here.
pub fn add_skill_point<W: LearnUnits>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    match check_skill_point(w, t, u, skill) {
        SkillPointCheck::Code2 => {
            w.send_attack_reset(u);
            2
        }
        SkillPointCheck::Code3 => 3,
        SkillPointCheck::Ok => {
            if !spend_skill_point(w, t, u, skill) {
                return 2;
            }
            w.point_client_updates(u);
            0
        }
    }
}
