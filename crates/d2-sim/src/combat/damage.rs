//! Damage: the damage record, rolling a hit's damage, totals and resistances, applying
//! damage with leech and the timed effects (stun, cold, freeze, poison, burn), hit class
//! and get-hit recovery, the open wounds and crushing blow event functions, and armor
//! durability loss.
//!
//! `hit` decides whether an attack lands; this file decides how much it does. Random
//! draws come from the attacker's or defender's own seed, in the order the functions
//! below make them.

use super::hit::result;
use super::{formulas, pct, scale, CombatEntry, CombatTables, CombatWorld, RoomKind};
use crate::skills::{weapon_mastery, SkillTables};
use crate::units::UnitType;

/// Hit flags of a damage record (`DamageRecord::hit_flags`).
pub mod hitflag {
    /// Do not roll physical damage.
    pub const SKIP_PHYSICAL: u32 = 0x1;
    /// Do not roll the record at all (its amounts are already set).
    pub const SKIP_ROLL: u32 = 0x2;
    /// The life leech amount is preset; do not add the attacker's.
    pub const LIFE_DRAIN_PRESET: u32 = 0x4;
    /// The mana leech amount is preset; do not add the attacker's.
    pub const MANA_DRAIN_PRESET: u32 = 0x8;
    /// The stamina leech amount is preset; do not add the attacker's.
    pub const STAMINA_DRAIN_PRESET: u32 = 0x10;
    /// The record has been rolled.
    pub const ROLLED: u32 = 0x20;
    /// Do not run the missile damage event.
    pub const NO_MISSILE_EVENT: u32 = 0x80;
    /// Damage bypasses undead resistance rules.
    pub const BYPASS_UNDEAD: u32 = 0x100;
    /// Damage bypasses demon resistance rules.
    pub const BYPASS_DEMONS: u32 = 0x200;
    /// Damage bypasses beast resistance rules.
    pub const BYPASS_BEASTS: u32 = 0x400;
    /// Apply the damage even when attacker and defender are not hostile.
    pub const IGNORE_HOSTILITY: u32 = 0x1000;
}

/// The damage record (D2MOO `D2DamageStrc`): what one hit does, by damage type. Amounts
/// are in 1/256 points unless noted; lengths are in frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DamageRecord {
    /// Hit flags ([`hitflag`]).
    pub hit_flags: u32,
    /// Result flags ([`result`]).
    pub result: u16,
    /// Physical damage.
    pub physical: i32,
    /// Enhanced-damage percent applied to physical damage.
    pub enh_pct: i32,
    /// Fire damage.
    pub fire: i32,
    /// Burning damage, total over `burn_len`.
    pub burn: i32,
    /// Burning length in frames.
    pub burn_len: i32,
    /// Lightning damage.
    pub lightning: i32,
    /// Magic damage.
    pub magic: i32,
    /// Cold damage.
    pub cold: i32,
    /// Poison damage, total over `poison_len`.
    pub poison: i32,
    /// Poison length in frames.
    pub poison_len: i32,
    /// Cold (slow) length in frames.
    pub cold_len: i32,
    /// Freeze length in frames.
    pub freeze_len: i32,
    /// Life leech: a percent for players, an amount for monsters.
    pub life_leech: i32,
    /// Mana leech.
    pub mana_leech: i32,
    /// Stamina leech.
    pub stamina_leech: i32,
    /// Stun length in frames.
    pub stun_len: i32,
    /// Life absorbed from the damage.
    pub absorbed: i32,
    /// Total damage, once `totals` has run.
    pub total: i32,
    /// Percent of the defender's damage reduction that the hit pierces (/1024).
    pub pierce_pct: i32,
    /// Hit class: low nibble weapon class, high nibble element.
    pub hit_class: u32,
    /// Nonzero when the hit class is fixed; `apply_melee` then does not add the
    /// attacker's weapon class to it.
    pub hit_class_fixed: u8,
    /// Conversion element (`elemtypes` index; 0 none).
    pub conv_elem: i8,
    /// Percent of physical damage converted to `conv_elem`.
    pub conv_pct: i32,
    /// Overlay applied to the defender after a melee hit.
    pub overlay: i32,
}

// Stats.
const STR: u16 = 0;
const DEX: u16 = 2;
const LIFE: u16 = 6;
const MAXHP: u16 = 7;
const MANA: u16 = 8;
const MAXMANA: u16 = 9;
const STAMINA: u16 = 10;
const LEVEL: u16 = 12;
const MAXDAMAGE_PERCENT: u16 = 17;
const MINDAMAGE_PERCENT: u16 = 18;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;
const SECONDARY_MINDAMAGE: u16 = 23;
const SECONDARY_MAXDAMAGE: u16 = 24;
const DAMAGEPERCENT: u16 = 25;
const ARMORCLASS: u16 = 31;
const NORMAL_DR: u16 = 34;
const MAGIC_DR: u16 = 35;
const DAMAGERESIST: u16 = 36;
const COLDLENGTH: u16 = 56;
const POISONMINDAM: u16 = 57;
const POISONMAXDAM: u16 = 58;
const POISONLENGTH: u16 = 59;
const LIFEDRAINMINDAM: u16 = 60;
const LIFEDRAINMAXDAM: u16 = 61;
const MANADRAINMINDAM: u16 = 62;
const MANADRAINMAXDAM: u16 = 63;
const STAMDRAINMINDAM: u16 = 64;
const STAMDRAINMAXDAM: u16 = 65;
const STUNLENGTH: u16 = 66;
const VELOCITYPERCENT: u16 = 67;
const ATTACKRATE: u16 = 68;
const OTHER_ANIMRATE: u16 = 69;
const HPREGEN: u16 = 74;
const MONSTER_PLAYERCOUNT: u16 = 100;
const POISON_OVERRIDE_LENGTH: u16 = 101;
const BYPASS_UNDEAD: u16 = 103;
const BYPASS_DEMONS: u16 = 104;
const BYPASS_BEASTS: u16 = 106;
const NORMALDAMAGE: u16 = 111;
const HALFFREEZE: u16 = 118;
const DAMAGETARGETAC: u16 = 120;
const DEMONDAMAGE_PERCENT: u16 = 121;
const UNDEADDAMAGE_PERCENT: u16 = 122;
const DEADLYSTRIKE: u16 = 141;
const CANNOTBEFROZEN: u16 = 153;
const DAMAGE_VS_MONTYPE: u16 = 180;
const FIRELENGTH: u16 = 315;
const POISON_COUNT: u16 = 326;
const PASSIVE_POIS_MASTERY: u16 = 332;
const PASSIVE_CRITICAL_STRIKE: u16 = 337;

// States.
const STATE_FREEZE: u16 = 1;
const STATE_POISON: u16 = 2;
const STATE_COLD: u16 = 11;
const STATE_STUNNED: u16 = 21;
const STATE_SANCTUARY: u16 = 47;
const STATE_UNINTERRUPTABLE: u16 = 54;
const STATE_OPENWOUNDS: u16 = 62;
const STATE_DEATH_DELAY: u16 = 92;
const STATE_SHATTER: u16 = 107;
const STATE_BURNING: u16 = 115;
const STATE_SHRINE_RESIST_FIRE: u16 = 131;
const STATE_SHRINE_RESIST_POISON: u16 = 133;

// Timer event types.
const TIMER_AITHINK: u8 = 2;
const TIMER_STATREGEN: u8 = 3;
const TIMER_REMOVESTATE: u8 = 12;

// Unit events (`events.txt`).
/// Event: the unit was damaged in melee.
pub const EV_DAMAGEDINMELEE: u8 = 1;
/// Event: the unit was damaged by a missile.
pub const EV_DAMAGEDBYMISSILE: u8 = 2;
/// Event: the unit was attacked in melee.
pub const EV_ATTACKEDINMELEE: u8 = 3;
/// Event: the unit deals melee damage.
pub const EV_DOMELEEDAMAGE: u8 = 5;
/// Event: the unit deals missile damage.
pub const EV_DOMISSILEDAMAGE: u8 = 6;
/// Event: the unit makes a melee attack.
pub const EV_DOMELEEATTACK: u8 = 7;
/// Event: the unit killed something.
pub const EV_KILL: u8 = 9;
/// Event: the unit was killed.
pub const EV_KILLED: u8 = 10;
/// Event: the unit absorbs damage.
pub const EV_ABSORBDAMAGE: u8 = 11;

// Monster type flags, as taken by `CombatWorld::monster_flag`.
const MON_SUPERUNIQUE: u32 = 2;
const MON_UNIQUE: u32 = 8;
const MON_UNIQUE_OR_CHAMPION: u32 = 0x0C;

/// Overlays.
const OVERLAY_LIFE_LEECH: i32 = 151;
const OVERLAY_MANA_LEECH: i32 = 152;
const OVERLAY_CRUSHING_BLOW: i32 = 147;
const OVERLAY_MONSTER_CRIT: i32 = 54;

/// A record field accessor.
type Field = fn(&mut DamageRecord) -> &mut i32;
/// A record field reader.
type Read = fn(&DamageRecord) -> i32;

/// True for a monster that is not a hireling.
fn is_monster_not_hireling<W: CombatWorld>(w: &W, u: W::Unit) -> bool {
    w.unit_type(u) == UnitType::Monster && !w.is_hireling(u)
}

// ---------------------------------------------------------------- rolling

/// Rolls a value between `min` and `max` (each raised by its percent), draws from the
/// unit's seed when the range is non-empty, and adds it to `current`. Never negative.
pub fn roll_in_range<W: CombatWorld>(
    w: &mut W,
    u: W::Unit,
    min: i32,
    max: i32,
    minpct: i32,
    maxpct: i32,
    current: i32,
) -> i32 {
    let mut cur = current;
    if max > 0 {
        let min = min.wrapping_add(pct(min, minpct, 100));
        let max = max.wrapping_add(pct(max, maxpct, 100));
        cur = cur.wrapping_add(min);
        if max > min {
            cur = cur.wrapping_add(w.seed(u).roll(max.wrapping_sub(min)) as i32);
        }
    }
    cur.max(0)
}

/// Rolls an elemental damage range from the unit's `maxstat` and `minstat` (whole points,
/// scaled to 1/256), with an optional mastery stat adding to both percents. Adds the
/// roll to `current`; returns `current` when the maximum is below 8.
#[allow(clippy::too_many_arguments)]
pub fn element<W: CombatWorld>(
    w: &mut W,
    u: W::Unit,
    maxstat: u16,
    minstat: u16,
    mastery: Option<u16>,
    minpct: i32,
    maxpct: i32,
    current: i32,
) -> i32 {
    let max = w.stat(u, maxstat, 0).wrapping_shl(8);
    if max < 8 {
        return current.max(0);
    }
    let min = w.stat(u, minstat, 0).wrapping_shl(8);
    let (mut lo, mut hi) = (minpct, maxpct);
    if let Some(m) = mastery {
        let v = w.stat(u, m, 0);
        lo = lo.wrapping_add(v);
        hi = hi.wrapping_add(v);
    }
    roll_in_range(w, u, min, max, lo, hi, current)
}

/// Rolls physical damage with all percent bonuses (strength and dexterity bonus of the
/// item, weapon mastery, damage percent stats) and adds it to `current`. With `get`, the
/// range is read from the unit's damage stats and `item = None` means the current weapon.
/// `s` is `SrcDam` (128 = unscaled).
#[allow(clippy::too_many_arguments)]
pub fn bonuses<W: CombatWorld>(
    w: &mut W,
    st: &SkillTables,
    u: W::Unit,
    get: bool,
    item: Option<W::Item>,
    min: i32,
    max: i32,
    pct_in: i32,
    current: i32,
    s: i32,
) -> i32 {
    let mut item = item;
    let (mut min, mut max) = (min, max);
    if get {
        if item.is_none() {
            item = w.current_weapon(u);
        }
        match item {
            Some(i) if w.wield_type(i) == 2 => {
                min = w.stat(u, SECONDARY_MINDAMAGE, 0);
                max = w.stat(u, SECONDARY_MAXDAMAGE, 0);
            }
            Some(_) => {
                min = w.stat(u, MINDAMAGE, 0);
                max = w.stat(u, MAXDAMAGE, 0);
            }
            None => {
                min = w.stat(u, MINDAMAGE, 0).max(1);
                max = w.stat(u, MAXDAMAGE, 0).max(2);
            }
        }
        min = min.wrapping_shl(8);
        max = max.wrapping_shl(8);
    }
    // `item_normaldamage` is read through the item getter; strength, dexterity and the
    // damage stats (17, 18, 21–25) use the unit getter.
    let n = w.item_stat(u, NORMALDAMAGE, 0).wrapping_shl(8);
    min = min.wrapping_add(n);
    max = max.wrapping_add(n);
    if min < 1 {
        min = 256;
    }
    if max <= min {
        max = min.wrapping_add(256);
    }
    let mut p = pct_in.wrapping_add(w.stat(u, DAMAGEPERCENT, 0));
    if let Some(i) = item {
        let (sb, db) = w.str_dex_bonus(i);
        if sb != 0 {
            p = p.wrapping_add(w.stat(u, STR, 0).wrapping_mul(sb) / 100);
        }
        if db != 0 {
            p = p.wrapping_add(w.stat(u, DEX, 0).wrapping_mul(db) / 100);
        }
        p = p.wrapping_add(weapon_mastery(w, st, Some(u), Some(i), None, 1));
    } else if get {
        p = p.wrapping_add(w.stat(u, STR, 0));
    }
    p = p.max(-90);
    let mut cur = current;
    if max > 0 {
        let min_t = min.wrapping_add(pct(
            min,
            p.wrapping_add(w.stat(u, MINDAMAGE_PERCENT, 0)),
            100,
        ));
        let max_t = max.wrapping_add(pct(
            max,
            p.wrapping_add(w.stat(u, MAXDAMAGE_PERCENT, 0)),
            100,
        ));
        cur = cur.wrapping_add(min_t);
        if max_t > min_t {
            cur = cur.wrapping_add(w.seed(u).roll(max_t.wrapping_sub(min_t)) as i32);
        }
    }
    cur = cur.max(0);
    if s != 128 {
        cur = pct(cur, s, 128);
    }
    cur
}

/// Rolls a hit's damage into the record: bonuses against the target, physical damage with
/// critical strike, elemental damage, leech, poison, cold, stun and burn, conversion, and
/// the monster critical hit. `s = 0` is treated as 128 (`SrcDam`, unscaled).
#[allow(clippy::too_many_arguments)]
pub fn fill<W: CombatWorld>(
    w: &mut W,
    st: &SkillTables,
    ct: &CombatTables,
    a: W::Unit,
    d: W::Unit,
    rec: &mut DamageRecord,
    offhand: bool,
    s: i32,
) {
    let s = if s == 0 { 128 } else { s };
    w.dual_wield_switch(a, offhand, true);
    rec.hit_flags |= hitflag::ROLLED;
    // A mod game's damage formula replaces the whole roll.
    let input = *rec;
    if let Some(r) = formulas::ask(w, Some(a), Some(d), |f, h| f.damage(h, &input, offhand, s)) {
        *rec = r;
        w.dual_wield_switch(a, offhand, false);
        return;
    }
    let a_type = w.unit_type(a);
    let d_mon = w.unit_type(d) == UnitType::Monster;
    // Bonuses against the target: armor reduction and enhanced damage against demons,
    // undead and monster types.
    if (a_type == UnitType::Player || w.alignment(a) == 2) && d_mon {
        let t = w.item_stat(a, DAMAGETARGETAC, 0);
        if t != 0 {
            let ac = w.stat(d, ARMORCLASS, 0).wrapping_add(t).max(0);
            w.set_stat(d, ARMORCLASS, ac);
        }
        if w.is_demon(d) {
            let p = w.item_stat(a, DEMONDAMAGE_PERCENT, 0);
            if p > 0 {
                rec.enh_pct = rec.enh_pct.wrapping_add(p);
            }
        }
        // Undead take 50% extra from blunt weapons (item type 57).
        if w.is_undead(d) {
            let blunt = w.current_weapon(a).is_some_and(|i| w.item_is(i, 57));
            let p = (if blunt { 50i32 } else { 0 }).wrapping_add(w.item_stat(
                a,
                UNDEADDAMAGE_PERCENT,
                0,
            ));
            if p > 0 {
                rec.enh_pct = rec.enh_pct.wrapping_add(p);
            }
        }
        // Damage against the defender's monster type.
        let mb = super::hit::montype_bonus(w, ct, a, d, DAMAGE_VS_MONTYPE);
        rec.enh_pct = rec.enh_pct.wrapping_add(mb);
    }
    // Physical damage and critical strike: weapon mastery (main hand only), then the
    // passive critical strike, then deadly strike; the first success doubles the damage.
    if rec.hit_flags & hitflag::SKIP_PHYSICAL == 0 {
        rec.physical = bonuses(w, st, a, true, None, 0, 0, rec.enh_pct, rec.physical, s);
        let mut crit = false;
        let weapon = w.current_weapon(a);
        if !offhand && weapon.is_some() {
            let m = weapon_mastery(w, st, Some(a), weapon, None, 2);
            if m > 0 {
                crit = w.seed(a).roll_range(0, 100) < m;
            }
        }
        if !crit {
            let c = w.stat(a, PASSIVE_CRITICAL_STRIKE, 0);
            if c > 0 {
                crit = w.seed(a).roll_range(0, 100) < c;
            }
        }
        if !crit {
            let ds = w.item_stat(a, DEADLYSTRIKE, 0);
            if ds > 0 {
                crit = w.seed(a).roll_range(0, 100) < ds;
            }
        }
        if crit {
            rec.physical = rec.physical.wrapping_mul(2);
            rec.result |= result::CRITICAL;
        }
    }
    // Fire, lightning, cold and magic damage.
    let elems: [(u16, u16, u16, Field); 4] = [
        (49, 48, 329, |r| &mut r.fire),
        (51, 50, 330, |r| &mut r.lightning),
        (55, 54, 331, |r| &mut r.cold),
        (53, 52, 357, |r| &mut r.magic),
    ];
    for (maxs, mins, mastery, field) in elems {
        let cur = *field(rec);
        let mut v = element(w, a, maxs, mins, Some(mastery), 0, 0, cur);
        if s != 128 {
            v = scale(v, s);
        }
        *field(rec) = v;
    }
    // Life, mana and stamina drain.
    if is_monster_not_hireling(w, a) {
        if rec.hit_flags & hitflag::LIFE_DRAIN_PRESET == 0 {
            let v = element(
                w,
                a,
                LIFEDRAINMAXDAM,
                LIFEDRAINMINDAM,
                None,
                0,
                0,
                rec.life_leech,
            );
            rec.life_leech = rec.life_leech.wrapping_add(scale(v, s));
        }
        if rec.hit_flags & hitflag::MANA_DRAIN_PRESET == 0 {
            let v = element(
                w,
                a,
                MANADRAINMAXDAM,
                MANADRAINMINDAM,
                None,
                0,
                0,
                rec.mana_leech,
            );
            rec.mana_leech = rec.mana_leech.wrapping_add(scale(v, s));
        }
        if rec.hit_flags & hitflag::STAMINA_DRAIN_PRESET == 0 {
            let v = element(
                w,
                a,
                STAMDRAINMAXDAM,
                STAMDRAINMINDAM,
                None,
                0,
                0,
                rec.stamina_leech,
            );
            rec.stamina_leech = rec.stamina_leech.wrapping_add(scale(v, s));
        }
    } else if a_type == UnitType::Player || (a_type == UnitType::Monster && w.is_hireling(a)) {
        if rec.hit_flags & hitflag::LIFE_DRAIN_PRESET == 0 {
            rec.life_leech = rec.life_leech.wrapping_add(w.stat(a, LIFEDRAINMINDAM, 0));
        }
        if rec.hit_flags & hitflag::MANA_DRAIN_PRESET == 0 {
            rec.mana_leech = rec.mana_leech.wrapping_add(w.stat(a, MANADRAINMINDAM, 0));
        }
        // Flags that make the hit bypass undead, demon and beast resistance rules.
        if w.stat(a, BYPASS_UNDEAD, 0) != 0 {
            rec.hit_flags |= hitflag::BYPASS_UNDEAD;
        }
        if w.stat(a, BYPASS_DEMONS, 0) != 0 {
            rec.hit_flags |= hitflag::BYPASS_DEMONS;
        }
        if w.stat(a, BYPASS_BEASTS, 0) != 0 {
            rec.hit_flags |= hitflag::BYPASS_BEASTS;
        }
    }
    // Poison damage and length.
    let pmin = w.stat(a, POISONMINDAM, 0);
    let pmax = w.stat(a, POISONMAXDAM, 0);
    let m = if pmax > 0 {
        w.stat(a, PASSIVE_POIS_MASTERY, 0)
    } else {
        0
    };
    let v = roll_in_range(w, a, pmin, pmax, m, m, rec.poison);
    rec.poison = scale(v, s);
    if rec.poison != 0 {
        let o = w.stat(a, POISON_OVERRIDE_LENGTH, 0);
        if o > 0 {
            rec.poison_len = rec.poison_len.wrapping_add(o);
        } else {
            rec.poison_len = rec.poison_len.wrapping_add(w.stat(a, POISONLENGTH, 0));
            let n = w.stat(a, POISON_COUNT, 0);
            if n > 1 {
                rec.poison_len /= n;
            }
        }
    }
    // Cold and stun lengths.
    if rec.cold > 0 {
        rec.cold_len = rec
            .cold_len
            .wrapping_add(scale(w.stat(a, COLDLENGTH, 0), s));
    }
    if rec.stun_len == 0 {
        rec.stun_len = rec
            .stun_len
            .wrapping_add(scale(w.stat(a, STUNLENGTH, 0), s));
    }
    // Burn: the amount is scaled by `s` and offset by a constant plus a draw of `roll(1)`;
    // the draw is always made.
    let r1 = w.seed(a).roll(1) as i32;
    let b = rec
        .burn
        .wrapping_mul(s)
        .wrapping_add(316)
        .wrapping_add(r1)
        .max(0);
    rec.burn = rec.burn.wrapping_add(b / 128);
    if rec.burn_len != 0 {
        rec.burn_len = rec
            .burn_len
            .wrapping_add(scale(w.stat(a, FIRELENGTH, 0), s));
    }
    // Players and hirelings run the attacked-in-melee event on the defender.
    let player_or_hireling =
        a_type == UnitType::Player || (a_type == UnitType::Monster && w.is_hireling(a));
    if player_or_hireling && rec.result & result::NO_EVENTS == 0 {
        w.unit_event(EV_ATTACKEDINMELEE, d, a, rec);
    }
    // Conversion of physical damage to another element (10 picks one of five at random).
    if rec.conv_elem > 0 {
        let c = pct(rec.physical, rec.conv_pct, 100);
        rec.physical = rec.physical.wrapping_sub(c).max(0);
        let mut e = i32::from(rec.conv_elem);
        if e == 10 {
            e = w.seed(a).roll(5) as i32 + 1;
        }
        match e {
            1 => rec.fire = rec.fire.wrapping_add(c),
            2 => rec.lightning = rec.lightning.wrapping_add(c),
            3 => rec.magic = rec.magic.wrapping_add(c),
            4 => {
                rec.cold = rec.cold.wrapping_add(c);
                rec.cold_len = rec.cold_len.max(50);
            }
            5 => {
                rec.poison = rec.poison.wrapping_add(c / 8);
                rec.poison_len = rec.poison_len.max(50);
            }
            11 => {
                rec.fire = rec.fire.wrapping_add(c);
                rec.burn_len = rec.burn_len.max(50);
            }
            12 => {
                rec.cold = rec.cold.wrapping_add(c);
                rec.freeze_len = rec.freeze_len.max(50);
            }
            _ => {}
        }
    }
    monster_crit(w, ct, a, d, rec);
    w.dual_wield_switch(a, offhand, false);
}

/// Monster critical hit, also used for missile area hits: a monster attacker's `crit`
/// chance (one `lo' % 100` draw on its seed) doubles the damage fields and marks the hit
/// class.
pub fn monster_crit<W: CombatWorld>(
    w: &mut W,
    ct: &CombatTables,
    a: W::Unit,
    d: W::Unit,
    rec: &mut DamageRecord,
) {
    if w.unit_type(a) == UnitType::Monster {
        let crit = ct.monstats(w.class_id(a)).map_or(0, |m| i32::from(m.crit));
        if crit != 0 && ((w.seed(a).step() % 100) as i32) < crit {
            for f in [
                &mut rec.physical,
                &mut rec.fire,
                &mut rec.lightning,
                &mut rec.magic,
                &mut rec.cold,
                &mut rec.poison,
            ] {
                *f = f.wrapping_mul(2);
            }
            // The hit class gets 0x10 when its high nibble is free; overlay 54 goes on
            // the defender only when the nibble was already taken.
            if rec.hit_class & 0xF0 == 0 {
                rec.hit_class |= 0x10;
            } else {
                w.overlay(d, OVERLAY_MONSTER_CRIT);
            }
        }
    }
}

/// Starts a combat: on a hit that was not avoided, rolls the record (unless `SKIP_ROLL`)
/// and totals it, flagging `WILL_DIE` when the damage exceeds the defender's life. Then
/// prepends a copy of the record to the attacker's combat list.
pub fn start_combat<W: CombatWorld>(
    w: &mut W,
    st: &SkillTables,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: Option<W::Unit>,
    rec: &mut DamageRecord,
    srcdam: i32,
) {
    let (Some(a), Some(d)) = (a, d) else {
        return;
    };
    const AVOIDED: u16 = result::DODGE | result::AVOID | result::EVADE | result::WEAPON_BLOCK;
    if rec.result & result::HIT != 0 && rec.result & AVOIDED == 0 {
        if rec.hit_flags & hitflag::SKIP_ROLL == 0 {
            fill(w, st, ct, a, d, rec, false, srcdam);
        }
        totals(w, ct, Some(a), d, rec);
        let mut t = rec
            .physical
            .wrapping_add(rec.fire)
            .wrapping_add(rec.lightning)
            .wrapping_add(rec.magic)
            .wrapping_add(rec.cold)
            .wrapping_add(rec.poison);
        if w.unit_type(a) == UnitType::Monster {
            t = t.wrapping_add(rec.life_leech);
        }
        if (t & !0xFF) > (w.stat(d, LIFE, 0) & !0xFF) {
            rec.result |= result::WILL_DIE;
        }
    }
    // Only players and monsters are tracked in the combat list.
    if matches!(w.unit_type(d), UnitType::Player | UnitType::Monster) {
        let entry = CombatEntry {
            attacker: w.ident(a),
            defender: w.ident(d),
            record: *rec,
        };
        w.combat_list(a).insert(0, entry);
    }
}

// ---------------------------------------------------------------- totals

/// One row of the resistance table: a damage field with its resistance, maximum
/// resistance, pierce and absorb stats, which damage reduction applies, and whether
/// the row is leech damage or scaled by the damage percent.
#[derive(Debug, Clone, Copy)]
struct ResRow {
    field: Field,
    resist: Option<u16>,
    max: Option<u16>,
    pierce: Option<u16>,
    absorb_pct: Option<u16>,
    absorb_flat: Option<u16>,
    dr: Dr,
    leech: bool,
    scaled: bool,
}

/// Which flat damage reduction applies to a resistance row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dr {
    Normal,
    Magic,
    None,
}

/// Builds a `ResRow`, splitting the optional absorb pair into its two stats.
#[allow(clippy::too_many_arguments)]
const fn row(
    field: Field,
    resist: Option<u16>,
    max: Option<u16>,
    pierce: Option<u16>,
    absorb: Option<(u16, u16)>,
    dr: Dr,
    leech: bool,
    scaled: bool,
) -> ResRow {
    let (absorb_pct, absorb_flat) = match absorb {
        Some((p, f)) => (Some(p), Some(f)),
        None => (None, None),
    };
    ResRow {
        field,
        resist,
        max,
        pierce,
        absorb_pct,
        absorb_flat,
        dr,
        leech,
        scaled,
    }
}

/// The resistance table, one row per damage field (rows 0–11).
const RES_ROWS: [ResRow; 12] = [
    row(
        |r| &mut r.physical,
        Some(36),
        None,
        None,
        None,
        Dr::Normal,
        false,
        true,
    ),
    row(
        |r| &mut r.fire,
        Some(39),
        Some(40),
        Some(333),
        Some((142, 143)),
        Dr::Magic,
        false,
        true,
    ),
    row(
        |r| &mut r.lightning,
        Some(41),
        Some(42),
        Some(334),
        Some((144, 145)),
        Dr::Magic,
        false,
        true,
    ),
    row(
        |r| &mut r.cold,
        Some(43),
        Some(44),
        Some(335),
        Some((148, 149)),
        Dr::Magic,
        false,
        true,
    ),
    row(
        |r| &mut r.magic,
        Some(37),
        Some(38),
        None,
        Some((146, 147)),
        Dr::Magic,
        false,
        true,
    ),
    row(
        |r| &mut r.cold_len,
        Some(43),
        Some(44),
        Some(335),
        None,
        Dr::None,
        false,
        false,
    ),
    row(
        |r| &mut r.freeze_len,
        Some(43),
        Some(44),
        Some(335),
        None,
        Dr::None,
        false,
        false,
    ),
    row(
        |r| &mut r.poison_len,
        Some(110),
        None,
        Some(336),
        None,
        Dr::None,
        false,
        false,
    ),
    row(
        |r| &mut r.poison,
        Some(45),
        Some(46),
        Some(336),
        None,
        Dr::None,
        false,
        true,
    ),
    row(
        |r| &mut r.life_leech,
        None,
        None,
        None,
        None,
        Dr::None,
        true,
        true,
    ),
    row(
        |r| &mut r.mana_leech,
        None,
        None,
        None,
        None,
        Dr::None,
        true,
        true,
    ),
    row(
        |r| &mut r.stamina_leech,
        None,
        None,
        None,
        None,
        Dr::None,
        true,
        true,
    ),
];

/// Percent that scales a hit's damage by who attacks whom: 17% for a player hit by a
/// player, hireling or revived unit, 25% between hirelings, the difficulty's hireable boss
/// damage percent for a hireling hitting a boss, 200% or 400% for a prime evil hitting a
/// revived unit, otherwise 100%.
pub fn damage_percent<W: CombatWorld>(
    w: &W,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: W::Unit,
) -> i32 {
    let Some(a) = a else {
        return 100;
    };
    if a == d {
        return 100;
    }
    let a_hire = w.unit_type(a) == UnitType::Monster && w.is_hireling(a);
    let d_hire = w.unit_type(d) == UnitType::Monster && w.is_hireling(d);
    if w.unit_type(d) == UnitType::Player
        && (w.unit_type(a) == UnitType::Player || a_hire || w.is_revived(a))
    {
        17
    } else if d_hire && a_hire {
        25
    } else if w.is_boss(d) && a_hire {
        ct.difficulty(w.difficulty())
            .map_or(100, |r| r.hireablebossdamagepercent as i32)
    } else if w.is_revived(d) && w.is_prime_evil(a) {
        if d_hire {
            200
        } else {
            400
        }
    } else {
        100
    }
}

/// The defender's effective resistance for one table row: its resist stat reduced by the
/// attacker's pierce, plus the difficulty penalty (non-monster defenders), capped by the
/// maximum resistance and floored at -100.
fn resist_value<W: CombatWorld>(
    w: &W,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: W::Unit,
    row: &ResRow,
    def_mon: bool,
) -> i32 {
    let mut r = row.resist.map_or(0, |s| w.stat(d, s, 0));
    if let Some(p) = row.pierce {
        if r < 100 || !def_mon {
            r = r.wrapping_sub(a.map_or(0, |a| w.stat(a, p, 0)));
        }
    }
    // Every row except physical (stat 36) and 37 takes the difficulty penalty; leech rows,
    // which have no resist stat, therefore take it too.
    if !def_mon && row.resist != Some(DAMAGERESIST) && row.resist != Some(37) {
        let (difficulty, expansion) = (w.difficulty(), w.expansion());
        let pen = w
            .rules()
            .formulas()
            .and_then(|f| f.resist_penalty(difficulty, expansion))
            .unwrap_or_else(|| match (expansion, difficulty) {
                (true, d) => ct.difficulty(d).map_or(0, |x| x.resistpenalty as i32),
                (false, 1) => -20,
                (false, 2) => -50,
                (false, _) => 0,
            });
        r = r.wrapping_add(pen);
    }
    if r > 0 {
        if !def_mon {
            let cap = formulas::ask_shared(w, a, Some(d), |f, h| f.resist_cap(h, row.resist))
                .unwrap_or_else(|| match row.max {
                    Some(m) => w.stat(d, m, 0).wrapping_add(75).min(95),
                    None if row.resist == Some(DAMAGERESIST) => 50,
                    None => 75,
                });
            r = r.min(cap);
        }
        if row.resist == Some(DAMAGERESIST)
            && a.is_some_and(|a| w.has_state(a, STATE_SANCTUARY))
            && w.is_undead(d)
        {
            r = 0;
        }
    } else {
        r = r.max(-100);
    }
    r
}

/// Computes the record's final amounts against the defender: damage percent scaling,
/// the absorb event, freeze and shrine immunities, then per row damage reduction,
/// resistance and absorption. Leaves the sum in `rec.total`.
pub fn totals<W: CombatWorld>(
    w: &mut W,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: W::Unit,
    rec: &mut DamageRecord,
) {
    let att_mon = a.is_some_and(|a| is_monster_not_hireling(w, a));
    let def_mon = is_monster_not_hireling(w, d);
    // Flat damage reduction, reduced by the hit's pierce percent.
    let mut dr_normal = w.stat(d, NORMAL_DR, 0).wrapping_shl(8);
    if dr_normal > 0 && rec.pierce_pct > 0 {
        dr_normal = pct(dr_normal, rec.pierce_pct, 1024);
    }
    let mut dr_magic = w.stat(d, MAGIC_DR, 0).wrapping_shl(8);
    if dr_magic > 0 && rec.pierce_pct > 0 {
        dr_magic = pct(dr_magic, rec.pierce_pct, 1024);
    }
    // Scale the damage rows by the damage percent.
    let p = damage_percent(w, ct, a, d);
    if p != 100 {
        for row in RES_ROWS.iter().filter(|r| r.scaled) {
            let f = (row.field)(rec);
            if *f > 0 {
                *f = pct(*f, p, 100);
            }
        }
    }
    // Absorb-damage event.
    if let Some(a) = a {
        w.unit_event(EV_ABSORBDAMAGE, d, a, rec);
    }
    // Cold, freeze, poison and burn lengths cut by immunities and shrines.
    if rec.cold_len > 0 || rec.freeze_len > 0 {
        if w.item_stat(d, CANNOTBEFROZEN, 0) != 0 {
            rec.cold_len = 0;
            rec.freeze_len = 0;
        } else if w.item_stat(d, HALFFREEZE, 0) != 0 {
            rec.cold_len /= 2;
            rec.freeze_len /= 2;
        }
    }
    if rec.poison_len > 0 && w.has_state(d, STATE_SHRINE_RESIST_POISON) {
        rec.poison_len = 0;
    }
    if rec.burn_len > 0 && w.has_state(d, STATE_SHRINE_RESIST_FIRE) {
        rec.burn_len = 0;
    }
    // Whether the hit bypasses absorption for this defender (bypass flags).
    let hf = rec.hit_flags;
    let no_absorb = if w.unit_type(d) == UnitType::Monster {
        let undead = w.is_undead(d);
        let demon = w.is_demon(d);
        (undead && hf & hitflag::BYPASS_UNDEAD != 0)
            || (demon && hf & hitflag::BYPASS_DEMONS != 0)
            || (!undead && !demon && hf & hitflag::BYPASS_BEASTS != 0)
    } else {
        hf & hitflag::BYPASS_BEASTS != 0
    };
    // Per row: damage reduction, resistance, then absorption.
    for row in &RES_ROWS {
        if row.leech && !att_mon {
            break;
        }
        let mut v = *(row.field)(rec);
        if v <= 0 {
            *(row.field)(rec) = 0;
            continue;
        }
        let mut r = resist_value(w, ct, a, d, row, def_mon);
        if no_absorb {
            r = r.min(0);
        } else {
            v = v.wrapping_sub(match row.dr {
                Dr::Normal => dr_normal,
                Dr::Magic => dr_magic,
                Dr::None => 0,
            });
        }
        if v > 0 && r != 0 {
            v = pct(v, 100 - r.min(100), 100);
        }
        if !no_absorb {
            if let (Some(ap), Some(af)) = (row.absorb_pct, row.absorb_flat) {
                let ab = w.stat(d, ap, 0).min(40);
                if ab > 0 {
                    let x = pct(v, ab, 100);
                    rec.absorbed = rec.absorbed.wrapping_add(x);
                    v = v.wrapping_sub(x);
                }
                let f = w.stat(d, af, 0).wrapping_shl(8);
                if f > 0 {
                    let x = f.min(v);
                    rec.absorbed = rec.absorbed.wrapping_add(x);
                    v = v.wrapping_sub(x);
                }
            }
        }
        *(row.field)(rec) = v;
    }
    // Sum into the total; a monster attacker's life leech counts too.
    let mut t = rec
        .physical
        .wrapping_add(rec.fire)
        .wrapping_add(rec.lightning)
        .wrapping_add(rec.magic)
        .wrapping_add(rec.cold)
        .wrapping_add(rec.poison);
    if att_mon {
        t = t.wrapping_add(rec.life_leech);
    }
    rec.total = t;
}

// ---------------------------------------------------------------- application

/// Heals the unit by `x`, up to its maximum life; returns the life added. Does nothing
/// for a dead unit or one in the death-delay state.
pub fn heal<W: CombatWorld>(w: &mut W, u: W::Unit, x: i32) -> i32 {
    if x <= 0 || w.is_dead(u) || w.has_state(u, STATE_DEATH_DELAY) {
        return 0;
    }
    let life = w.stat(u, LIFE, 0);
    let new = life.wrapping_add(x).min(w.stat(u, MAXHP, 0));
    w.set_stat(u, LIFE, new);
    new.wrapping_sub(life)
}

/// Adds `x` mana up to the unit's maximum; returns the mana added.
pub fn add_mana<W: CombatWorld>(w: &mut W, u: W::Unit, x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }
    let mana = w.stat(u, MANA, 0);
    let new = mana.wrapping_add(x).min(w.stat(u, MAXMANA, 0));
    w.set_stat(u, MANA, new);
    new.wrapping_sub(mana)
}

/// Applies the record's life and mana leech. A player or hireling attacker gains a
/// percent of the physical damage (scaled by the difficulty's steal divisors); any other
/// attacker takes life, mana and stamina from the defender's pools. A monster defender's
/// drain value for the difficulty scales the amount, and a drain of 0 disables it.
pub fn leech<W: CombatWorld>(
    w: &mut W,
    ct: &CombatTables,
    a: Option<W::Unit>,
    d: W::Unit,
    rec: &mut DamageRecord,
) {
    if rec.life_leech == 0 && rec.mana_leech == 0 {
        return;
    }
    let diff = w.difficulty();
    let drain = if w.unit_type(d) == UnitType::Monster {
        let m = ct.monstats(w.class_id(d));
        let v = m.map_or(0, |m| match diff {
            0 => m.drain,
            1 => m.drain_n,
            _ => m.drain_h,
        });
        let v = i32::from(v);
        if v <= 0 {
            return;
        }
        v
    } else {
        100
    };
    rec.life_leech = rec.life_leech.wrapping_shl(6);
    rec.mana_leech = rec.mana_leech.wrapping_shl(6);
    let Some(a) = a else {
        // No attacker: the fields are stored back as `(x << 6) / 64`, nothing is healed,
        // no overlay, no draw.
        rec.life_leech /= 64;
        rec.mana_leech /= 64;
        return;
    };
    let player_rule = match w.unit_type(a) {
        UnitType::Player => {
            if let Some(dl) = ct.difficulty(diff) {
                let (ld, md) = (dl.lifestealdivisor as i32, dl.manastealdivisor as i32);
                if ld != 0 {
                    rec.life_leech /= ld;
                }
                if md != 0 {
                    rec.mana_leech /= md;
                }
            }
            true
        }
        UnitType::Monster if w.is_hireling(a) => true,
        _ => w.converted_type(a) == 0,
    };
    if player_rule {
        if rec.physical <= 0 {
            return;
        }
        let mana = rec.mana_leech != 0;
        if mana {
            let mut m = pct(rec.physical, rec.mana_leech, 100);
            if drain != 100 {
                m = pct(m, drain, 100);
            }
            add_mana(w, a, m / 64);
        }
        if rec.life_leech != 0 {
            let mut l = pct(rec.physical, rec.life_leech, 100);
            if drain != 100 {
                l = pct(l, drain, 100);
            }
            heal(w, a, l / 64);
            let o = if mana && w.seed(a).roll(2) != 0 {
                OVERLAY_MANA_LEECH
            } else {
                OVERLAY_LIFE_LEECH
            };
            w.overlay(a, o);
        } else if mana {
            w.overlay(a, OVERLAY_MANA_LEECH);
        }
    } else {
        rec.life_leech /= 64;
        rec.mana_leech /= 64;
        let l = rec.life_leech.min(rec.physical);
        let m = rec.mana_leech.min(w.stat(d, MANA, 0));
        let s = rec.stamina_leech.min(w.stat(d, STAMINA, 0));
        let mut t = l.wrapping_add(m).wrapping_add(s);
        if t == 0 {
            return;
        }
        if drain != 100 {
            t = pct(t, drain, 100);
        }
        let life = w.stat(a, LIFE, 0);
        t = t.min(w.stat(a, MAXHP, 0).wrapping_sub(life));
        if t > 0 {
            w.set_stat(a, LIFE, life.wrapping_add(t));
            w.overlay(a, OVERLAY_LIFE_LEECH);
        }
    }
}

/// Stuns the defender for `n` frames. Unique monsters resist it 90% of the time, bosses and
/// immobile monsters always; the length is capped at 250 frames (13 for a hireling).
pub fn stun<W: CombatWorld>(w: &mut W, ct: &CombatTables, a: W::Unit, d: W::Unit, n: i32) {
    if n <= 0 {
        return;
    }
    let mut n = n;
    if w.unit_type(d) == UnitType::Monster {
        if w.monster_flag(d, MON_UNIQUE) && ((w.seed(a).step() % 100) as i32) < 90 {
            return;
        }
        if w.is_boss(d) {
            return;
        }
        if ct.monstats(w.class_id(d)).is_some_and(|m| m.velocity == 0) {
            return;
        }
        n = if w.is_hireling(d) && n >= 13 {
            13
        } else {
            n.min(250)
        };
    } else {
        n = n.min(250);
    }
    // The length and its caps are base ticks (250 = 10 s, 13 ≈ 0.5 s).
    let e = w.frame().wrapping_add(w.tick_rate().ticks(n));
    if w.state_list_expiry(d, STATE_STUNNED).is_some() {
        w.set_state_list_expiry(d, STATE_STUNNED, e);
        w.schedule_timer(d, TIMER_REMOVESTATE, e);
    } else {
        w.create_state_list(d, STATE_STUNNED, a, e);
        w.schedule_timer(d, TIMER_REMOVESTATE, e);
        w.set_state(d, STATE_STUNNED, true);
    }
}

/// The defender's cold effect: `monstats.ColdEffect` for the difficulty (signed byte), or
/// −50 for non-monsters.
fn cold_effect<W: CombatWorld>(w: &W, ct: &CombatTables, d: W::Unit) -> i32 {
    if w.unit_type(d) != UnitType::Monster {
        return -50;
    }
    ct.monstats(w.class_id(d)).map_or(0, |m| {
        i32::from(match w.difficulty() {
            0 => m.coldeffect,
            1 => m.coldeffect_n,
            _ => m.coldeffect_h,
        } as i8)
    })
}

/// Chills the defender for `length` frames: the cold state slows velocity, attack rate
/// and animation rate by the cold effect. Also rolls the chance (one draw) that a
/// monster shatters when it dies.
pub fn cold<W: CombatWorld>(w: &mut W, ct: &CombatTables, a: W::Unit, d: W::Unit, length: i32) {
    if length <= 0 {
        return;
    }
    let effect = cold_effect(w, ct, d);
    if effect == 0 {
        return;
    }
    let mon = w.unit_type(d) == UnitType::Monster;
    let mut length = length;
    if mon && effect < 0 {
        let div = ct
            .difficulty(w.difficulty())
            .map_or(0, |x| x.monstercolddivisor as i32);
        if div != 0 {
            length /= div;
        }
    }
    length = length.max(1);
    // The chill lasts `length` base ticks.
    let e = w.frame().wrapping_add(w.tick_rate().ticks(length));
    match w.state_list_expiry(d, STATE_COLD) {
        None => {
            w.set_state(d, STATE_COLD, true);
            w.create_state_list(d, STATE_COLD, a, e);
            w.schedule_timer(d, TIMER_REMOVESTATE, e);
            for s in [VELOCITYPERCENT, ATTACKRATE, OTHER_ANIMRATE] {
                w.set_state_list_stat(d, STATE_COLD, s, effect);
            }
            w.refresh_anim_rate(d);
        }
        Some(x) if x < e => {
            w.set_state_list_expiry(d, STATE_COLD, e);
            w.schedule_timer(d, TIMER_REMOVESTATE, e);
        }
        Some(_) => {}
    }
    // Shatter.
    let r = (w.seed(d).step() % 100) as i32;
    let dead_col = ct.monstats2(w.class_id(d)).is_some_and(|m| m.deadcol);
    w.set_state(d, STATE_SHATTER, r < 20 && mon && !dead_col);
}

/// Freezes the defender for `length` frames. Players, bosses, uniques and hirelings get
/// the cold effect instead; other monsters freeze for `length` divided by the difficulty's
/// freeze divisor and have their AI think rescheduled. Panics when that divisor is 0.
pub fn freeze<W: CombatWorld>(w: &mut W, ct: &CombatTables, d: W::Unit, a: W::Unit, length: i32) {
    if length <= 0 {
        return;
    }
    match w.unit_type(d) {
        UnitType::Player => return cold(w, ct, a, d, length),
        UnitType::Monster => {}
        _ => return,
    }
    if w.has_state(d, STATE_UNINTERRUPTABLE) {
        return;
    }
    if w.is_boss(d) || w.monster_flag(d, MON_UNIQUE) || w.is_hireling(d) {
        return cold(w, ct, a, d, length);
    }
    if cold_effect(w, ct, d) >= 0 {
        return;
    }
    let div = ct
        .difficulty(w.difficulty())
        .map_or(0, |x| x.monsterfreezedivisor as i32);
    assert!(div != 0, "MonsterFreezeDivisor 0 (damage.md §5.7)");
    // The freeze lasts `length / div` base ticks.
    let length = w.tick_rate().ticks(length / div);
    let e = w.frame().wrapping_add(length);
    let expiry = match w.state_list_expiry(d, STATE_FREEZE) {
        Some(x) => {
            let k = x.max(e);
            w.set_state_list_expiry(d, STATE_FREEZE, k);
            k
        }
        None => {
            // A new freeze list; switching the state on follows the poison and stun
            // rules.
            w.set_state(d, STATE_FREEZE, true);
            w.create_state_list(d, STATE_FREEZE, a, e);
            e
        }
    };
    w.schedule_timer(d, TIMER_REMOVESTATE, expiry);
    w.cancel_timers(d, TIMER_AITHINK);
    w.schedule_timer(
        d,
        TIMER_AITHINK,
        w.frame().wrapping_add(length).wrapping_add(1),
    );
}

/// Damage over time shared by poison (state 2) and burn (state 115): `damage` is the
/// total over `length` frames, stored as a negative hp regen on the state's stat list.
/// An existing list is only replaced by an equal or stronger one.
fn dot<W: CombatWorld>(w: &mut W, a: W::Unit, d: W::Unit, state: u16, damage: i32, length: i32) {
    if length <= 0 || damage <= 0 {
        return;
    }
    if w.unit_type(d) == UnitType::Monster {
        w.cancel_timers(d, TIMER_STATREGEN);
        w.schedule_timer(d, TIMER_STATREGEN, w.frame().wrapping_add(1));
    }
    // The poison or burn lasts `length` base ticks; its per-tick damage
    // (HPREGEN) is shared out per tick where regeneration applies it.
    let e = w.frame().wrapping_add(w.tick_rate().ticks(length));
    if w.state_list_expiry(d, state).is_some() {
        let regen = w.state_stat(d, state, HPREGEN).unwrap_or(0);
        if regen.wrapping_neg() <= damage {
            w.set_state_list_expiry(d, state, e);
            w.schedule_timer(d, TIMER_REMOVESTATE, e);
            w.set_state_list_stat(d, state, HPREGEN, damage.wrapping_neg());
        }
    } else {
        w.set_state(d, state, true);
        w.create_state_list(d, state, a, e);
        w.set_state_list_stat(d, state, HPREGEN, damage.wrapping_neg());
        w.schedule_timer(d, TIMER_REMOVESTATE, e);
    }
}

/// Poisons the defender: `damage` over `length` frames.
pub fn poison<W: CombatWorld>(w: &mut W, a: W::Unit, d: W::Unit, damage: i32, length: i32) {
    dot(w, a, d, STATE_POISON, damage, length);
}

/// Burns the defender: `damage` over `length` frames.
pub fn burn<W: CombatWorld>(w: &mut W, a: W::Unit, d: W::Unit, damage: i32, length: i32) {
    dot(w, a, d, STATE_BURNING, damage, length);
}

/// Applies a rolled record to the defender: checks hostility, room and killability, runs
/// the damage events, leech, life/mana/stamina loss, the timed effects (stun, cold,
/// freeze, poison, burn), sets or clears `WILL_DIE`, and runs the kill events. A missile
/// hit is totalled here; a melee record was totalled when it was rolled.
pub fn apply<W: CombatWorld>(
    w: &mut W,
    ct: &CombatTables,
    a: W::Unit,
    d: W::Unit,
    missile: bool,
    rec: &mut DamageRecord,
) {
    // Gates: hostility, room, killable monster, not already dead.
    if !w.hostile(a, d) && rec.hit_flags & hitflag::IGNORE_HOSTILITY == 0 {
        rec.result &= !(result::HIT | result::WILL_DIE);
        return;
    }
    match w.room(d) {
        RoomKind::None => {
            rec.result &= !(result::HIT | result::WILL_DIE);
            return;
        }
        RoomKind::Town => {
            let in_town = w.unit_type(a) == UnitType::Monster
                && ct.monstats(w.class_id(a)).is_some_and(|m| m.intown);
            if !in_town {
                rec.result &= !result::WILL_DIE;
                return;
            }
        }
        RoomKind::Field => {}
    }
    if w.unit_type(d) == UnitType::Monster
        && !ct.monstats(w.class_id(d)).is_some_and(|m| m.killable)
    {
        return;
    }
    if w.is_dead(d) {
        return;
    }
    // Remember the attacker; totals for a missile hit.
    w.set_last_attacker(d, a);
    if missile {
        totals(w, ct, Some(a), d, rec);
    }
    // Damage events, melee or missile.
    if rec.result & result::HIT != 0 && rec.result & result::NO_EVENTS == 0 {
        if missile {
            if rec.hit_flags & hitflag::ROLLED != 0
                && rec.hit_flags & hitflag::NO_MISSILE_EVENT == 0
            {
                w.unit_event(EV_DOMISSILEDAMAGE, a, d, rec);
            }
            w.unit_event(EV_DAMAGEDBYMISSILE, d, a, rec);
        } else {
            w.unit_event(EV_DOMELEEDAMAGE, a, d, rec);
            w.unit_event(EV_DAMAGEDINMELEE, d, a, rec);
        }
    }
    // Leech, and the monster hit hook.
    rec.physical = rec.physical.min(w.stat(d, LIFE, 0));
    leech(w, ct, Some(a), d, rec);
    if w.unit_type(a) == UnitType::Monster && rec.result & result::HIT != 0 {
        w.monster_hit_hook(a);
    }
    // Absorbed life heals the defender; life, mana and stamina losses are floored at 0
    // below one point (256).
    heal(w, d, rec.absorbed);
    if rec.total > 0 {
        let mut life = w.stat(d, LIFE, 0).wrapping_sub(rec.total);
        if life < 256 {
            life = 0;
        }
        w.set_stat(d, LIFE, life);
    }
    if rec.mana_leech > 0 {
        let mut m = w.stat(d, MANA, 0).wrapping_sub(rec.mana_leech);
        if m < 256 {
            m = 0;
        }
        w.set_stat(d, MANA, m);
    }
    if rec.stamina_leech > 0 {
        let mut s = w.stat(d, STAMINA, 0).wrapping_sub(rec.stamina_leech);
        if s < 256 {
            s = 0;
        }
        w.set_stat(d, STAMINA, s);
    }
    // Timed effects.
    stun(w, ct, a, d, rec.stun_len);
    cold(w, ct, a, d, rec.cold_len);
    freeze(w, ct, d, a, rec.freeze_len);
    poison(w, a, d, rec.poison, rec.poison_len);
    burn(w, a, d, rec.burn, rec.burn_len);
    // Clear or set WILL_DIE from the defender's remaining life; hurt monsters get their
    // regen timer and damaged hook.
    if w.stat(d, LIFE, 0) > 0 {
        rec.result &= !result::WILL_DIE;
        if rec.total != 0 && w.unit_type(d) == UnitType::Monster {
            if w.stat(d, HPREGEN, 0) != 0 {
                w.cancel_timers(d, TIMER_STATREGEN);
                w.schedule_timer(d, TIMER_STATREGEN, w.frame().wrapping_add(1));
            }
            w.monster_damaged_hook(d);
        }
    } else {
        rec.result |= result::WILL_DIE;
    }
    // Kill events.
    if rec.result & result::NO_EVENTS == 0 && rec.result & result::WILL_DIE != 0 {
        w.unit_event(EV_KILLED, d, a, rec);
        w.unit_event(EV_KILL, a, d, rec);
    }
}

/// Removes the attacker's combat list entries for this defender.
pub fn free_records<W: CombatWorld>(w: &mut W, a: W::Unit, d: W::Unit) {
    let (ia, id) = (w.ident(a), w.ident(d));
    w.combat_list(a)
        .retain(|e| !(e.attacker == ia && e.defender == id));
}

/// Applies the attacker's queued melee record against the defender: out of melee range
/// the record is dropped; on a hit it is applied, durability is lost, and thorns run.
/// Then the melee attack events and the defender's reaction run, and the record is freed.
pub fn apply_melee<W: CombatWorld>(w: &mut W, ct: &CombatTables, a: W::Unit, d: W::Unit) {
    let id = w.ident(d);
    let Some(mut rec) = w
        .combat_list(a)
        .iter()
        .find(|e| e.defender == id)
        .map(|e| e.record)
    else {
        return;
    };
    let range = 1 + 2 * i32::from(w.unit_type(a) == UnitType::Monster);
    if !w.in_melee_range(a, d, range) {
        free_records(w, a, d);
        return;
    }
    let hit = rec.result & result::HIT != 0;
    let a_type = w.unit_type(a);
    let pm = matches!(a_type, UnitType::Player | UnitType::Monster);
    if hit {
        // A dead attacker returns at once: the combat record is not freed, and no
        // events, thorns or reaction run.
        if pm && w.mode(a) == 0 {
            return;
        }
        rec.hit_flags = hitflag::ROLLED;
        apply(w, ct, a, d, false, &mut rec);
        if rec.overlay > 0 {
            w.overlay(d, rec.overlay);
        }
        if rec.hit_class_fixed == 0 && rec.hit_class & 0x0F == 0 {
            rec.hit_class |= w.weapon_hit_class(a);
        }
        durability(w, a, d);
    }
    w.unit_event(EV_DOMELEEATTACK, a, d, &mut rec);
    w.unit_event(EV_ATTACKEDINMELEE, d, a, &mut rec);
    // A player or monster attacker that died while the hit was applied returns at once,
    // as above; other types (objects included) and live attackers take thorns.
    if hit {
        if pm && w.mode(a) == 0 {
            return;
        }
        w.thorns(a, d, &mut rec);
    }
    w.reaction(a, d, &mut rec);
    free_records(w, a, d);
}

// ---------------------------------------------------------------- hit class

/// Element hit-class table: the high nibble and damage field for cold, fire, lightning,
/// poison.
const ELEMENT_CLASSES: [(u32, Read); 4] = [
    (0x30, |r| r.cold),
    (0x20, |r| r.fire),
    (0x40, |r| r.lightning),
    (0x50, |r| r.poison),
];

/// The hit class of a hit: `base` is the weapon class (low nibble; 0 becomes 13) and the
/// high nibble is the first element with damage, scanning the table from a start that
/// rotates with the shared counter (which this call steps). A critical hit with no
/// element gets 0x10.
pub fn element_hit_class(counter: &mut u8, rec: &DamageRecord, base: u32) -> u32 {
    let i = usize::from(*counter % 4);
    *counter = counter.wrapping_add(1);
    let mut nibble = 0;
    for k in 0..4 {
        let (n, f) = ELEMENT_CLASSES[(i + k) % 4];
        if f(rec) > 0 {
            nibble = n;
            break;
        }
    }
    if nibble == 0 && rec.result & result::CRITICAL != 0 {
        nibble = 0x10;
    }
    let base = if base == 0 { 13 } else { base };
    base | nibble
}

/// The get-hit divisor of a hit class, from the `hitclass` code of row `class`: 8 for
/// one-handed swords, thrown, bows and crossbows, 32 for two-handed swords and clubs,
/// 64 for two-handed long swords, otherwise 16.
pub fn get_hit_divisor(hitclass_codes: &[[u8; 4]], class: u32) -> i32 {
    match usize::try_from(class)
        .ok()
        .and_then(|i| hitclass_codes.get(i))
    {
        Some(b"1hss" | b"1ht " | b"bow " | b"xbow") => 8,
        Some(b"2hss" | b"club") => 32,
        Some(b"2hsl") => 64,
        _ => 16,
    }
}

/// Get-hit test: returns `true` when the unit does **not** enter its get-hit reaction
/// (frozen, tiny or pure-poison damage, or too little damage for the hit class's divisor
/// of the unit's life, with random chances for the middle bands).
pub fn no_get_hit<W: CombatWorld>(
    w: &mut W,
    hitclass_codes: &[[u8; 4]],
    u: W::Unit,
    rec: &DamageRecord,
    hitclass: u32,
) -> bool {
    if w.has_state(u, STATE_FREEZE) {
        return true;
    }
    if (rec.poison != 0 && rec.poison == rec.total) || rec.total < 256 {
        return true;
    }
    let div = get_hit_divisor(hitclass_codes, hitclass);
    let m = w.stat(u, MAXHP, 0);
    if rec.total < m / div {
        return true;
    }
    if rec.total < m / (div / 2) && w.seed(u).mask(2) == 0 {
        return true;
    }
    if rec.total < m / (div / 4) && w.seed(u).mask(4) == 0 {
        return true;
    }
    w.unit_type(u) == UnitType::Monster && !w.monster_has_mode(u, 3)
}

// ---------------------------------------------------------------- event functions

/// The chance stat an event function argument names (`stat id << 16 | layer`), read
/// from the attacker.
fn chance_of<W: CombatWorld>(w: &W, a: W::Unit, arg: u32) -> i32 {
    w.item_stat(a, (arg >> 16) as u16, arg as u16)
}

/// Crushing blow, event function 16. `a` owns the event (the attacker), `d` is the other
/// unit. Takes a fraction of the defender's current life, reduced by its damage
/// resistance. Returns 1 when the chance roll succeeded, else 0.
pub fn crushing_blow<W: CombatWorld>(
    w: &mut W,
    event: u8,
    a: W::Unit,
    d: W::Unit,
    rec: &mut DamageRecord,
    arg: u32,
) -> i32 {
    let c = chance_of(w, a, arg);
    if c <= 0 {
        return 0;
    }
    if ((w.seed(a).step() % 100) as i32) >= c {
        return 0;
    }
    let mut div = match w.unit_type(d) {
        UnitType::Player => 10,
        UnitType::Monster if w.is_hireling(d) => 10,
        UnitType::Monster => {
            let mut v: i32 = if w.is_boss(d) || w.monster_flag(d, MON_SUPERUNIQUE) {
                8
            } else {
                4
            };
            // The player-count term belongs to the non-hireling monster branch.
            let h = w.player_count_bonus(w.stat(d, MONSTER_PLAYERCOUNT, 0).max(1));
            if h != 0 {
                v = v.wrapping_add(pct(v, h, 100));
            }
            v
        }
        _ => 4,
    };
    if event == EV_DOMISSILEDAMAGE {
        div = div.wrapping_mul(2);
    }
    let life = w.stat(d, LIFE, 0);
    let mut x = if div != 0 { life / div } else { 0 };
    let dr = w.stat(d, DAMAGERESIST, 0).min(100);
    if dr > 0 {
        x = x.wrapping_sub(pct(x, dr, 100));
    }
    let new = life.wrapping_sub(x).max(0);
    w.set_stat(d, LIFE, new);
    if new <= 0 {
        rec.result |= result::WILL_DIE;
    }
    if x > 0 {
        w.overlay(d, OVERLAY_CRUSHING_BLOW);
    }
    1
}

/// Open wounds' level term: a piecewise-linear total that steepens every 15 levels.
pub fn open_wounds_base(lvl: i32) -> i32 {
    if lvl <= 1 {
        0
    } else if lvl <= 15 {
        9 * (lvl - 1)
    } else if lvl <= 30 {
        18 * (lvl - 15) + 14 * 9
    } else if lvl <= 45 {
        27 * (lvl - 30) + 15 * 18 + 14 * 9
    } else if lvl <= 60 {
        36 * (lvl - 45) + 15 * (18 + 27) + 14 * 9
    } else {
        45i32
            .wrapping_mul(lvl - 60)
            .wrapping_add(15 * (18 + 27 + 36) + 14 * 9)
    }
}

/// Open wounds, event function 15: on a successful chance roll, curses the defender with
/// negative hp regen and returns the hpregen magnitude `h` of the curse (`None` when the
/// chance failed). Players take a quarter of it (halved again for missiles); unique and
/// champion monsters take half.
pub fn open_wounds<W: CombatWorld>(
    w: &mut W,
    event: u8,
    a: W::Unit,
    d: W::Unit,
    arg: u32,
) -> Option<i32> {
    let c = chance_of(w, a, arg);
    if c <= 0 || ((w.seed(a).step() % 100) as i32) >= c {
        return None;
    }
    let lvl = w.stat(a, LEVEL, 0).max(1);
    let mut h = open_wounds_base(lvl).wrapping_add(40);
    match w.unit_type(d) {
        UnitType::Player => {
            h /= 4;
            if event == EV_DOMISSILEDAMAGE {
                h /= 2;
            }
        }
        UnitType::Monster if w.monster_flag(d, MON_UNIQUE_OR_CHAMPION) => h /= 2,
        _ => {}
    }
    w.curse(d, a, STATE_OPENWOUNDS, HPREGEN, h.wrapping_neg(), 200, 0, 1);
    Some(h)
}

// ---------------------------------------------------------------- durability

/// Armor durability weights: (body location, weight).
pub const DURABILITY_WEIGHTS: [(u8, i32); 7] =
    [(1, 3), (3, 5), (4, 4), (5, 4), (8, 2), (9, 2), (10, 2)];

/// Rolls the chance that `owner`'s armor or weapon loses durability when it takes part in
/// a hit, and applies the loss on success. Armor has 10%, weapons 4%; throwing weapons
/// 10% in expansion games and never in classic.
pub fn durability_hit<W: CombatWorld>(w: &mut W, owner: W::Unit, item: W::Item) {
    let armor = w.item_is(item, 50);
    if !(armor || w.item_is(item, 45)) || !w.item_has_durability(item) {
        return;
    }
    let chance = if armor {
        10
    } else if w.item_flag_throw(item) {
        if w.expansion() {
            10
        } else {
            return;
        }
    } else {
        4
    };
    if ((w.seed(owner).step() % 100) as i32) < chance {
        w.durability_loss(owner, item);
    }
}

/// Durability loss for a melee hit: the attacker's weapon (players only), and for a player
/// defender one worn armor piece picked by the location weights.
pub fn durability<W: CombatWorld>(w: &mut W, a: W::Unit, d: W::Unit) {
    if w.unit_type(a) == UnitType::Player {
        if let Some(weapon) = w.current_weapon(a) {
            durability_hit(w, a, weapon);
        }
    }
    if w.unit_type(d) != UnitType::Player {
        return;
    }
    // Every player defender is treated as having an inventory.
    let kept: Vec<Option<W::Item>> = DURABILITY_WEIGHTS
        .iter()
        .map(|&(loc, _)| w.item_at(d, loc).filter(|&i| w.item_is(i, 50)))
        .collect();
    let total: i32 = DURABILITY_WEIGHTS
        .iter()
        .zip(&kept)
        .filter(|(_, k)| k.is_some())
        .map(|(&(_, wt), _)| wt)
        .sum();
    if total <= 0 {
        return;
    }
    let mut i = w.seed(d).roll(7) as usize;
    let mut x = w.seed(d).roll(total) as i32;
    while x >= 0 {
        if let Some(item) = kept[i] {
            let wt = DURABILITY_WEIGHTS[i].1;
            if x < wt {
                durability_hit(w, d, item);
                return;
            }
            x -= wt;
        }
        i = (i + 1) % 7;
    }
}
