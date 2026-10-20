//! The hit test: attack rating, defense, chance to hit, melee result flags, block
//! chance, and block / weapon block / dodge / avoid / evade.
//!
//! Damage itself is rolled and applied in `damage`. Each random draw comes from the
//! attacker's or defender's own seed, in the order the functions below make them.

use super::{formulas, pct, CombatTables, CombatWorld};
use crate::skills::{eval_skill, weapon_mastery, SkillTables};
use crate::units::UnitType;

/// Result flags of a damage record.
pub mod result {
    /// The attack hit.
    pub const HIT: u16 = 0x0001;
    /// The hit kills the defender.
    pub const WILL_DIE: u16 = 0x0002;
    /// The defender plays its get-hit reaction.
    pub const GET_HIT: u16 = 0x0004;
    /// The hit knocks the defender back.
    pub const KNOCKBACK: u16 = 0x0008;
    /// The defender blocked.
    pub const BLOCK: u16 = 0x0010;
    /// Skip the hit's unit events.
    pub const NO_EVENTS: u16 = 0x0020;
    /// The defender dodged.
    pub const DODGE: u16 = 0x0080;
    /// The defender avoided the attack.
    pub const AVOID: u16 = 0x0100;
    /// The defender evaded.
    pub const EVADE: u16 = 0x0200;
    /// Critical strike.
    pub const CRITICAL: u16 = 0x2000;
    /// Soft hit.
    pub const SOFT_HIT: u16 = 0x4000;
    /// The defender blocked with its weapon.
    pub const WEAPON_BLOCK: u16 = 0x8000;
}

// Stats.
const DEX: u16 = 2;
const LEVEL: u16 = 12;
const ITEM_ARMOR_PERCENT: u16 = 16;
const TOHIT: u16 = 19;
const TOBLOCK: u16 = 20;
const ARMORCLASS: u16 = 31;
const AC_VS_MISSILE: u16 = 32;
const AC_VS_HTH: u16 = 33;
const IGNORETARGETAC: u16 = 115;
const FRACTIONALTARGETAC: u16 = 116;
const PREVENTHEAL: u16 = 117;
const TOHIT_PERCENT: u16 = 119;
const DEMON_TOHIT: u16 = 123;
const UNDEAD_TOHIT: u16 = 124;
const SKILL_ARMOR_PERCENT: u16 = 171;
const ATTACK_VS_MONTYPE: u16 = 179;
const ARMOR_OVERRIDE_PERCENT: u16 = 182;
const PASSIVE_DODGE: u16 = 338;
const PASSIVE_AVOID: u16 = 339;
const PASSIVE_EVADE: u16 = 340;
const PASSIVE_WEAPONBLOCK: u16 = 348;
const MODIFIERLIST_SKILL: u16 = 350;
const MODIFIERLIST_LEVEL: u16 = 351;

// States.
const STATE_PREVENTHEAL: u16 = 52;
const STATE_UNINTERRUPTABLE: u16 = 54;
const STATE_HOLYSHIELD: u16 = 101;

/// Length in frames of the prevent-heal curse a hit applies.
pub const PREVENT_HEAL_FRAMES: i32 = 120_000;
/// Cap on block chance, in percent.
pub const BLOCK_CAP: i32 = 75;
/// Weapon class that can weapon-block (`ht2`).
pub const WEAPON_CLASS_HT2: i32 = 13;

/// Player modes.
const PLAYER_WALK: i32 = 2;
const PLAYER_RUN: i32 = 3;
/// Monster modes.
const MONSTER_WALK: i32 = 2;
const MONSTER_RUN: i32 = 15;

/// Monster type flags, as taken by `CombatWorld::monster_flag`.
const MON_SUPERUNIQUE: u32 = 2;
const MON_UNIQUE_OR_SUPER: u32 = 0x0A;

/// A player's attack rating: `tohit` + 5 × (dexterity − 7) + the class's `tohitfactor`.
/// Panics for a non-player.
pub fn attack_rating<W: CombatWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    assert_eq!(
        w.unit_type(u),
        UnitType::Player,
        "attack_rating of a non-player"
    );
    let f = ct
        .charstats(w.class_id(u))
        .map_or(0, |c| c.tohitfactor as i32);
    w.stat(u, TOHIT, 0)
        .wrapping_add(5i32.wrapping_mul(w.stat(u, DEX, 0).wrapping_sub(7)))
        .wrapping_add(f)
}

/// A unit's defense: armor class plus dexterity / 4, scaled by the armor percent bonuses
/// (including Holy Shield while a shield is worn) and the armor override percent.
/// Takes the world mutably because the Holy Shield formula is evaluated, and a `rand` in
/// a formula would draw from a seed.
pub fn defense<W: CombatWorld>(w: &mut W, st: &SkillTables, u: W::Unit) -> i32 {
    let base = w.stat(u, ARMORCLASS, 0).wrapping_add(w.stat(u, DEX, 0) / 4);
    let mut p = w
        .item_stat(u, SKILL_ARMOR_PERCENT, 0)
        .wrapping_add(w.item_stat(u, ITEM_ARMOR_PERCENT, 0));
    if w.has_state(u, STATE_HOLYSHIELD) {
        let skill = w
            .state_stat(u, STATE_HOLYSHIELD, MODIFIERLIST_SKILL)
            .unwrap_or(0);
        let level = w
            .state_stat(u, STATE_HOLYSHIELD, MODIFIERLIST_LEVEL)
            .unwrap_or(0);
        if skill > 0 && level > 0 && w.has_shield(u) {
            if let Some(rec) = st.skill(skill) {
                let calc1 = rec.calc1;
                p = p.wrapping_add(eval_skill(w, st, Some(u), calc1, skill, level));
            }
        }
    }
    let bonus = if base > 0 {
        p.wrapping_mul(base) / 100
    } else {
        p.wrapping_mul(base) / -100
    };
    let mut total = base.wrapping_add(bonus);
    let ovr = w.stat(u, ARMOR_OVERRIDE_PERCENT, 0);
    if ovr != 0 {
        total = total.wrapping_add(pct(total, ovr, 100));
    }
    total
}

/// The terms of the hit chance formula, after attack rating and defense adjustments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitTerms {
    /// Attacker's attack rating.
    pub ar: i32,
    /// Percent bonus to attack rating.
    pub pct_ar: i32,
    /// Defender's defense.
    pub def: i32,
    /// Attacker's level.
    pub alvl: i32,
    /// Defender's level.
    pub dlvl: i32,
}

/// The hit chance clamped to 5–95 percent. Panics when `alvl + dlvl = 0` (division by
/// zero).
pub fn hit_chance(t: HitTerms) -> i32 {
    let mut to_hit = t.ar.wrapping_add(pct(t.ar, t.pct_ar, 100));
    let mut def = t.def;
    if def < 0 {
        to_hit = to_hit.wrapping_sub(def);
        def = 0;
    }
    if to_hit < 0 {
        def = def.wrapping_sub(to_hit);
        to_hit = 0;
    }
    if def < 0 {
        def = 0;
    }
    let sum = to_hit.wrapping_add(def);
    let factor = if sum != 0 {
        100i32.wrapping_mul(to_hit).wrapping_div(sum)
    } else {
        100
    };
    let lv = t.alvl.wrapping_add(t.dlvl);
    assert!(lv != 0, "hit test with alvl + dlvl = 0");
    let chance = 2i32
        .wrapping_mul(factor)
        .wrapping_mul(t.alvl)
        .wrapping_div(lv);
    chance.clamp(5, 95)
}

/// Monster's `MonType`, or 0.
fn montype<W: CombatWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    ct.monstats(w.class_id(u))
        .map_or(0, |m| i32::from(m.montype as i16))
}

/// Sum of the attacker's entries of `stat` (at most 128) whose layer matches the
/// defender's montype. Used for the to-hit and damage bonuses against a monster type.
pub(crate) fn montype_bonus<W: CombatWorld>(
    w: &W,
    ct: &CombatTables,
    a: W::Unit,
    d: W::Unit,
    stat: u16,
) -> i32 {
    if w.unit_type(d) != UnitType::Monster {
        return 0;
    }
    let mt = montype(w, ct, d);
    if mt <= 0 {
        return 0;
    }
    w.stat_entries(a, stat, 128)
        .into_iter()
        .filter(|&(layer, _)| w.montype_matches(layer, mt))
        .fold(0i32, |s, (_, v)| s.wrapping_add(v))
}

/// Builds the terms of the hit test for an attack: attack rating and its percent
/// bonus, the defender's defense after the target-defense reductions, and both levels.
pub fn hit_terms<W: CombatWorld>(
    w: &mut W,
    st: &SkillTables,
    ct: &CombatTables,
    a: W::Unit,
    d: W::Unit,
    bonus: i32,
    missile: bool,
) -> HitTerms {
    let dlvl = w.stat(d, LEVEL, 0);
    let alvl = w.stat(a, LEVEL, 0);
    let extra = w.stat(d, if missile { AC_VS_MISSILE } else { AC_VS_HTH }, 0);
    let mut def = defense(w, st, d).wrapping_add(extra);
    let (ar, pct_ar);
    if w.unit_type(a) == UnitType::Player {
        let mut r = attack_rating(w, ct, a);
        // Defense reductions, then attack rating bonuses against demons and undead.
        let d_mon = w.unit_type(d) == UnitType::Monster;
        if w.item_stat(a, IGNORETARGETAC, 0) != 0
            && d_mon
            && !w.monster_flag(d, MON_UNIQUE_OR_SUPER)
            && !w.is_boss(d)
            && !w.is_hireling(d)
        {
            def = 0;
        }
        let mut f = w.item_stat(a, FRACTIONALTARGETAC, 0);
        if f > 0 {
            let strong =
                !d_mon || w.monster_flag(d, MON_SUPERUNIQUE) || w.is_boss(d) || w.is_hireling(d);
            if strong {
                f /= 2;
            }
            f = f.clamp(0, 100);
            def = def.wrapping_sub(pct(def, f, 100));
        }
        let demon = w.item_stat(a, DEMON_TOHIT, 0);
        if demon != 0 && w.is_demon(d) {
            r = r.wrapping_add(demon);
        }
        let undead = w.item_stat(a, UNDEAD_TOHIT, 0);
        if undead != 0 && w.is_undead(d) {
            r = r.wrapping_add(undead);
        }
        let mut p = 0i32;
        if !missile {
            if let Some(weapon) = w.current_weapon(a) {
                p = weapon_mastery(w, st, Some(a), Some(weapon), None, 0);
            }
        }
        p = p
            .wrapping_add(bonus)
            .wrapping_add(w.item_stat(a, TOHIT_PERCENT, 0));
        p = p.wrapping_add(montype_bonus(w, ct, a, d, ATTACK_VS_MONTYPE));
        ar = r;
        pct_ar = p;
    } else {
        ar = w
            .stat(a, TOHIT, 0)
            .wrapping_add(bonus)
            .wrapping_add(5i32.wrapping_mul(w.stat(a, DEX, 0)));
        pct_ar = w.item_stat(a, TOHIT_PERCENT, 0);
    }
    HitTerms {
        ar,
        pct_ar,
        def,
        alvl,
        dlvl,
    }
}

/// The hit test: one draw from the attacker's seed (`lo′ mod 100 < chance`). On a hit by a
/// player with prevent heal against a monster, the prevent-heal curse is applied.
pub fn hit_test<W: CombatWorld>(
    w: &mut W,
    st: &SkillTables,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: Option<W::Unit>,
    bonus: i32,
    missile: bool,
) -> bool {
    let (Some(a), Some(d)) = (a, d) else {
        return false;
    };
    let terms = hit_terms(w, st, ct, a, d, bonus, missile);
    let chance = formulas::ask(w, Some(a), Some(d), |f, h| f.hit_chance(h, terms))
        .unwrap_or_else(|| hit_chance(terms));
    let r = (w.seed(a).step() % 100) as i32;
    if r >= chance {
        return false;
    }
    // Prevent-heal curse.
    if w.unit_type(a) == UnitType::Player
        && w.unit_type(d) == UnitType::Monster
        && w.item_stat(a, PREVENTHEAL, 0) != 0
    {
        w.curse(
            d,
            a,
            STATE_PREVENTHEAL,
            ARMORCLASS,
            0,
            PREVENT_HEAL_FRAMES,
            0,
            1,
        );
    }
    true
}

/// Result flags of a melee attack: the range check, the hit test, then block / dodge /
/// avoid / weapon block, and `GET_HIT` unless the defender is uninterruptable.
pub fn melee_result<W: CombatWorld>(
    w: &mut W,
    st: &SkillTables,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: Option<W::Unit>,
    bonus: i32,
    range_offset: i32,
) -> u16 {
    let (Some(a), Some(d)) = (a, d) else {
        return 0;
    };
    let pm = |t| matches!(t, UnitType::Player | UnitType::Monster);
    if !pm(w.unit_type(a)) || !pm(w.unit_type(d)) || !w.hostile(a, d) {
        return 0;
    }
    // The monster-side check is true for every monster, so it is not tested.
    let mut flags = 0u16;
    if w.unit_type(d) == UnitType::Player && w.mode(d) == PLAYER_RUN {
        flags = result::HIT;
    }
    let mut range = 1 + 2 * i32::from(w.unit_type(a) == UnitType::Monster);
    if range_offset != 0 {
        range = range.wrapping_sub(range_offset.wrapping_add(w.melee_range(a)));
    }
    if !w.in_melee_range(a, d, range) {
        return 0;
    }
    if flags & result::HIT == 0 && hit_test(w, st, ct, Some(a), Some(d), bonus, false) {
        flags |= result::HIT;
    }
    if flags & result::HIT != 0 {
        let b = block_or_dodge(w, ct, a, d, false, true);
        flags |= match b {
            BlockResult::Avoid => result::AVOID,
            BlockResult::Dodge => result::DODGE,
            BlockResult::WeaponBlock => result::WEAPON_BLOCK,
            BlockResult::Block => result::BLOCK,
            BlockResult::None | BlockResult::Evade => 0,
        };
        if b != BlockResult::None {
            flags &= !result::HIT;
        }
    }
    if flags & result::BLOCK != 0 {
        flags &= !result::HIT;
    }
    if flags & result::HIT != 0 && !w.has_state(d, STATE_UNINTERRUPTABLE) {
        flags |= result::GET_HIT;
    }
    flags
}

/// Block chance of a unit in percent, capped at `BLOCK_CAP`. Players need a shield;
/// monsters need the `noshldblock` flag or a shield composit.
pub fn block_chance<W: CombatWorld>(w: &W, ct: &CombatTables, u: W::Unit, expansion: bool) -> i32 {
    if let Some(c) = formulas::ask_shared(w, None, Some(u), |f, h| f.block_chance(h)) {
        return c;
    }
    match w.unit_type(u) {
        UnitType::Player => {
            if !w.has_shield(u) {
                return 0;
            }
            let bf = ct
                .charstats(w.class_id(u))
                .map_or(0, |c| i32::from(c.blockfactor));
            let mut b = w.stat(u, TOBLOCK, 0).wrapping_add(bf);
            if expansion {
                let lvl = w.stat(u, LEVEL, 0).max(1);
                b = w
                    .stat(u, DEX, 0)
                    .wrapping_sub(15)
                    .wrapping_mul(b)
                    .wrapping_div(2i32.wrapping_mul(lvl));
            }
            b.min(BLOCK_CAP)
        }
        UnitType::Monster => {
            let class = w.class_id(u);
            let no_shld = ct.monstats(class).is_some_and(|m| m.noshldblock);
            let shield = no_shld
                || match class {
                    243 | 310 | 333 => true,
                    359 => false,
                    _ => w.composit_shield(u),
                };
            if shield {
                w.stat(u, TOBLOCK, 0).min(BLOCK_CAP)
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Outcome of `block_or_dodge` / `dodge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockResult {
    /// Nothing was blocked or dodged.
    None,
    /// Blocked with a shield.
    Block,
    /// Avoided.
    Avoid,
    /// Dodged.
    Dodge,
    /// Evaded while moving.
    Evade,
    /// Blocked with a weapon.
    WeaponBlock,
}

/// Tries the defender's shield block first (when `block` is set and the block chance is
/// positive), then falls back to `dodge`. A running player's block chance is divided
/// by 3.
pub fn block_or_dodge<W: CombatWorld>(
    w: &mut W,
    ct: &CombatTables,
    a: W::Unit,
    d: W::Unit,
    avoid: bool,
    block: bool,
) -> BlockResult {
    if !block {
        return dodge(w, a, d, avoid);
    }
    let mut c = block_chance(w, ct, d, w.expansion());
    if c <= 0 {
        return dodge(w, a, d, avoid);
    }
    if w.unit_type(d) == UnitType::Player && w.moving_mode(d) && w.mode(d) != PLAYER_WALK {
        c /= 3;
    }
    let r = (w.seed(d).step() % 100) as i32;
    if r < c {
        BlockResult::Block
    } else {
        dodge(w, a, d, avoid)
    }
}

/// The non-shield defenses. A moving defender can only evade; a standing one tries weapon
/// block, then avoid (when `avoid` is set) or dodge.
pub fn dodge<W: CombatWorld>(w: &mut W, _a: W::Unit, d: W::Unit, avoid: bool) -> BlockResult {
    let mode = w.mode(d);
    let moving = match w.unit_type(d) {
        UnitType::Player => mode == PLAYER_WALK || mode == PLAYER_RUN,
        UnitType::Monster => mode == MONSTER_WALK || mode == MONSTER_RUN,
        _ => false,
    };
    if moving {
        let e = avoid_chance(w, d, BlockResult::Evade, |w| w.stat(d, PASSIVE_EVADE, 0));
        if e > 0 && ((w.seed(d).step() % 100) as i32) < e {
            return BlockResult::Evade;
        }
        return BlockResult::None;
    }
    let wb = avoid_chance(w, d, BlockResult::WeaponBlock, |w| weapon_block(w, Some(d)));
    if wb > 0 && w.weapon_class(d) == WEAPON_CLASS_HT2 && (w.seed(d).roll(100) as i32) < wb {
        return BlockResult::WeaponBlock;
    }
    let (stat, res) = if avoid {
        (PASSIVE_AVOID, BlockResult::Avoid)
    } else {
        (PASSIVE_DODGE, BlockResult::Dodge)
    };
    let v = avoid_chance(w, d, res, |w| w.stat(d, stat, 0));
    if v > 0 && (w.seed(d).roll(100) as i32) < v {
        return res;
    }
    BlockResult::None
}

/// The defender's chance of avoid `kind`: the game's formula hook first,
/// else D2RUST's `own` value.
fn avoid_chance<W: CombatWorld>(
    w: &mut W,
    d: W::Unit,
    kind: BlockResult,
    own: impl FnOnce(&W) -> i32,
) -> i32 {
    formulas::ask(w, None, Some(d), |f, h| f.avoid_chance(h, kind)).unwrap_or_else(|| own(w))
}

/// Weapon block chance of a unit: walks at most 32 `passive_weaponblock` entries in order
/// from 0; a layer-0 entry sets the value unconditionally, a typed entry matching the
/// left- or right-hand item's type raises it to its value (signed max). No draws.
pub fn weapon_block<W: CombatWorld>(w: &W, u: Option<W::Unit>) -> i32 {
    let Some(u) = u else {
        return 0;
    };
    let right = w.item_at(u, 4);
    let left = w.item_at(u, 5);
    let mut wb = 0;
    for (layer, v) in w.stat_entries(u, PASSIVE_WEAPONBLOCK, 32) {
        if layer == 0 {
            // A layer-0 entry sets the value even when it is smaller than the current one.
            wb = v;
        } else {
            let l = i32::from(layer);
            let typed =
                left.is_some_and(|i| w.item_is(i, l)) || right.is_some_and(|i| w.item_is(i, l));
            if typed && v > wb {
                wb = v;
            }
        }
    }
    wb
}
