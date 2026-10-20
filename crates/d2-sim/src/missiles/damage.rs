//! Missile damage setup at creation: the
//! damage record from the missile row, its skill row
//! and the owner's (or the origin's) stats, the bonus check
//! with its unit-seed draws, the stat writes and
//! the wrapper with its weapon switch / restore.
//!
//! Unit and item reads go through [`CombatWorld`] / [`SkillUnits`]; the
//! few setup-only queries are [`SetupWorld`].

use crate::combat::{pct, CombatWorld};
use crate::skills::levels::{
    elem_len, elem_mastery_stat, elem_max, elem_min, miss_elem_len, miss_elem_max, miss_elem_min,
    miss_phys_max, miss_phys_min, phys_max, phys_min,
};
use crate::skills::{weapon_mastery, SkillTables, SkillUnits};
use crate::units::UnitType;

/// Stats read and written (`ItemStatCost` ids).
mod st {
    pub const STRENGTH: u16 = 0;
    pub const DEXTERITY: u16 = 2;
    pub const LEVEL: u16 = 12;
    pub const MAXDAMAGE_PERCENT: u16 = 17;
    pub const MINDAMAGE_PERCENT: u16 = 18;
    pub const MINDAMAGE: u16 = 21;
    pub const MAXDAMAGE: u16 = 22;
    pub const SECONDARY_MINDAMAGE: u16 = 23;
    pub const SECONDARY_MAXDAMAGE: u16 = 24;
    pub const DAMAGEPERCENT: u16 = 25;
    pub const FIREMINDAM: u16 = 48;
    pub const FIREMAXDAM: u16 = 49;
    pub const LIGHTMINDAM: u16 = 50;
    pub const LIGHTMAXDAM: u16 = 51;
    pub const MAGICMINDAM: u16 = 52;
    pub const MAGICMAXDAM: u16 = 53;
    pub const COLDMINDAM: u16 = 54;
    pub const COLDMAXDAM: u16 = 55;
    pub const COLDLENGTH: u16 = 56;
    pub const POISONMINDAM: u16 = 57;
    pub const POISONMAXDAM: u16 = 58;
    pub const POISONLENGTH: u16 = 59;
    pub const LIFEDRAINMINDAM: u16 = 60;
    pub const LIFEDRAINMAXDAM: u16 = 61;
    pub const MANADRAINMINDAM: u16 = 62;
    pub const MANADRAINMAXDAM: u16 = 63;
    pub const STAMDRAINMINDAM: u16 = 64;
    pub const STAMDRAINMAXDAM: u16 = 65;
    pub const STUNLENGTH: u16 = 66;
    pub const POISON_LENGTH_OVERRIDE: u16 = 101;
    pub const BYPASS_UNDEAD: u16 = 103;
    pub const BYPASS_DEMONS: u16 = 104;
    pub const BYPASS_BEASTS: u16 = 106;
    pub const DAMAGETARGETAC: u16 = 120;
    pub const DEMONDAMAGE_PERCENT: u16 = 121;
    pub const UNDEADDAMAGE_PERCENT: u16 = 122;
    pub const DEADLYSTRIKE: u16 = 141;
    pub const ITEM_THROW_MINDAMAGE: u16 = 159;
    pub const ITEM_THROW_MAXDAMAGE: u16 = 160;
    pub const DAMAGE_VS_MONTYPE: u16 = 180;
    pub const FIRELENGTH: u16 = 315;
    pub const BURNINGMIN: u16 = 316;
    pub const BURNINGMAX: u16 = 317;
    pub const POISON_COUNT: u16 = 326;
    pub const PASSIVE_CRITICAL_STRIKE: u16 = 337;
}

/// Record flags (+0x00).
pub mod rec_flag {
    /// Owner-sourced (`SrcDam` ≠ 0).
    pub const OWNER: u32 = 0x1;
    /// The bonus check passed (deadly / critical strike).
    pub const BONUS: u32 = 0x2;
    /// Never set by the record build; read by the bonus check.
    pub const UNUSED_4: u32 = 0x4;
    pub const BYPASS_UNDEAD: u32 = 0x100;
    pub const BYPASS_DEMONS: u32 = 0x200;
    pub const BYPASS_BEASTS: u32 = 0x400;
}

/// Damage ranges, durations, source flags and bonuses assembled by [`record`]
/// and applied to the missile by [`write_stats`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SetupRecord {
    pub flags: u32,
    pub phys_min: i32,
    pub phys_max: i32,
    pub fire_min: i32,
    pub fire_max: i32,
    pub fire_len: i32,
    pub light_min: i32,
    pub light_max: i32,
    pub magic_min: i32,
    pub magic_max: i32,
    pub cold_min: i32,
    pub cold_max: i32,
    pub cold_len: i32,
    pub poison_min: i32,
    pub poison_max: i32,
    pub poison_len: i32,
    pub poison_count: i32,
    pub life_min: i32,
    pub life_max: i32,
    pub mana_min: i32,
    pub mana_max: i32,
    pub stam_min: i32,
    pub stam_max: i32,
    pub stun_len: i32,
    pub burn_min: i32,
    pub burn_max: i32,
    pub burn_len: i32,
    pub demon_pct: i32,
    pub undead_pct: i32,
    pub target_ac: i32,
    pub damage_pct: i32,
}

/// The queries of the setup that combat's world does not have.
pub trait SetupWorld: CombatWorld {
    /// Weapon W: a player's attack weapon; a monster with an
    /// inventory: its weapon; else none.
    fn setup_weapon(&self, owner: Self::Unit) -> Option<Self::Item>;
    /// The unit has an inventory (unit +0x60).
    fn has_inventory(&self, u: Self::Unit) -> bool;
    /// The item is two-handed.
    fn two_handed(&self, item: Self::Item) -> bool;
    /// Unit set.
    fn set_layer_stat(&mut self, u: Self::Unit, stat: u16, layer: u16, value: i32);
    /// Dual-wield stat toggle.
    fn dual_wield_toggle(&mut self, owner: Self::Unit);
}

/// `r = lo′ mod 100` of one step of `u`'s unit seed.
fn draw<W: SkillUnits>(w: &mut W, u: W::Unit) -> i32 {
    (w.seed(u).step() % 100) as i32
}

/// `bonus(u, W)`.
pub fn bonus<W: SetupWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    weapon: Option<W::Item>,
) -> bool {
    let c = w.stat(u, st::PASSIVE_CRITICAL_STRIKE, 0);
    if draw(w, u) < c {
        return true;
    }
    let d = w.item_stat(u, st::DEADLYSTRIKE, 0);
    if d != 0 && draw(w, u) < d {
        return true;
    }
    if weapon.is_some() {
        let m = weapon_mastery(w, t, Some(u), weapon, None, 2);
        if m != 0 && draw(w, u) < m {
            return true;
        }
    }
    false
}

/// `mastery(owner, E, v)`.
fn mastery<W: SetupWorld>(w: &W, owner: W::Unit, etype: u8, v: i32) -> i32 {
    match elem_mastery_stat(etype).map(|s| w.stat(owner, s, 0)) {
        Some(p) if p != 0 => pct(v, p, 100),
        _ => 0,
    }
}

/// `add(u, sh)`.
fn add<W: SetupWorld>(w: &W, u: W::Unit, sh: u32, r: &mut SetupRecord) {
    let s = |stat| w.stat(u, stat, 0);
    let sl = |stat| s(stat).wrapping_shl(sh);
    r.fire_min = r.fire_min.wrapping_add(sl(st::FIREMINDAM));
    r.fire_max = r.fire_max.wrapping_add(sl(st::FIREMAXDAM));
    r.light_min = r.light_min.wrapping_add(sl(st::LIGHTMINDAM));
    r.light_max = r.light_max.wrapping_add(sl(st::LIGHTMAXDAM));
    r.magic_min = r.magic_min.wrapping_add(sl(st::MAGICMINDAM));
    r.magic_max = r.magic_max.wrapping_add(sl(st::MAGICMAXDAM));
    r.cold_min = r.cold_min.wrapping_add(sl(st::COLDMINDAM));
    r.cold_max = r.cold_max.wrapping_add(sl(st::COLDMAXDAM));
    r.cold_len = r.cold_len.wrapping_add(s(st::COLDLENGTH));
    r.poison_min = r.poison_min.wrapping_add(s(st::POISONMINDAM));
    r.poison_max = r.poison_max.wrapping_add(s(st::POISONMAXDAM));
    let o = s(st::POISON_LENGTH_OVERRIDE);
    if o >= 1 {
        r.poison_len = r.poison_len.wrapping_add(o);
    } else {
        r.poison_len = r.poison_len.wrapping_add(s(st::POISONLENGTH));
        r.poison_count = r.poison_count.wrapping_add(s(st::POISON_COUNT));
    }
    r.life_min = r.life_min.wrapping_add(s(st::LIFEDRAINMINDAM));
    r.mana_min = r.mana_min.wrapping_add(s(st::MANADRAINMINDAM));
    r.burn_min = r.burn_min.wrapping_add(s(st::BURNINGMIN));
    r.burn_max = r.burn_max.wrapping_add(s(st::BURNINGMAX));
    r.burn_len = r.burn_len.wrapping_add(s(st::FIRELENGTH));
}

/// `scale(s)`: `trunc(v × s / 128)` (32-bit
/// product); fire length and poison length are not scaled.
fn scale(r: &mut SetupRecord, s: i32) {
    if s == 128 {
        return;
    }
    for v in [
        &mut r.fire_min,
        &mut r.fire_max,
        &mut r.light_min,
        &mut r.light_max,
        &mut r.magic_min,
        &mut r.magic_max,
        &mut r.cold_min,
        &mut r.cold_max,
        &mut r.cold_len,
        &mut r.poison_min,
        &mut r.poison_max,
        &mut r.poison_count,
        &mut r.life_min,
        &mut r.mana_min,
        &mut r.burn_min,
        &mut r.burn_max,
        &mut r.burn_len,
    ] {
        *v = v.wrapping_mul(s) / 128;
    }
}

/// The damage record
/// `skill` is the missile's stored skill (data +0x0A).
pub fn record<W: SetupWorld>(
    w: &mut W,
    t: &SkillTables,
    owner: W::Unit,
    origin: Option<W::Unit>,
    missile: W::Unit,
    skill: i32,
    level: i32,
) -> SetupRecord {
    let mut r = SetupRecord::default();
    // Step 1.
    if w.unit_type(missile) != UnitType::Missile {
        return r;
    }
    let class = w.class_id(missile);
    let Some(row) = t.missile(class).cloned() else {
        return r;
    };
    let row_skill = i32::from(row.skill as i16);
    let (mut s, m, pmin, pmax, etype, emin, emax, len);
    if row_skill < 1 && !row.missileskill {
        // Step 2: plain missile.
        s = i32::from(row.srcdamage);
        m = i32::from(row.srcmissdmg);
        pmin = miss_phys_min(w, t, Some(missile), Some(owner), class, level);
        pmax = miss_phys_max(w, t, Some(missile), Some(owner), class, level);
        etype = row.etype;
        let mut lo = miss_elem_min(w, t, Some(missile), Some(owner), class, level);
        let mut hi = miss_elem_max(w, t, Some(missile), Some(owner), class, level);
        len = miss_elem_len(w, t, Some(missile), class, level);
        if row.applymastery {
            lo = lo.wrapping_add(mastery(w, owner, etype, lo));
            hi = hi.wrapping_add(mastery(w, owner, etype, hi));
        }
        (emin, emax) = (lo, hi);
    } else {
        // Step 3: skill missile.
        let k = if row.missileskill && skill > 0 {
            skill
        } else {
            row_skill
        };
        let Some(srow) = t.skill(k) else {
            return r;
        };
        s = if row.srcdamage == 255 {
            0
        } else {
            i32::from(srow.srcdam)
        };
        m = 0;
        etype = srow.etype;
        pmin = phys_min(w, t, Some(owner), k, level, false);
        pmax = phys_max(w, t, Some(owner), k, level, false);
        emin = elem_min(w, t, Some(owner), k, level, true);
        emax = elem_max(w, t, Some(owner), k, level, true);
        len = elem_len(w, t, Some(owner), k, level);
    }
    // Step 4.
    for (bit, f) in [
        (1, rec_flag::BYPASS_UNDEAD),
        (2, rec_flag::BYPASS_DEMONS),
        (4, rec_flag::BYPASS_BEASTS),
    ] {
        if row.holy & bit != 0 {
            r.flags |= f;
        }
    }
    // Step 5.
    let (mut p, mut b, mut mn, mut mx, mut smin, mut smax) = (0i32, 0i32, 0, 0, 0, 0);
    if s != 0 {
        // Step 6: owner-sourced.
        r.flags |= rec_flag::OWNER;
        let weapon = w.setup_weapon(owner);
        if bonus(w, t, owner, weapon) {
            r.flags |= rec_flag::BONUS;
        }
        let (bmin, bmax) = if let Some(item) = weapon {
            // Step 6.1.
            if row.half2hsrc && w.two_handed(item) {
                s = ((s as u32) / 2) as i32;
            }
            b = weapon_mastery(w, t, Some(owner), Some(item), None, 1);
            let (sb, db) = w.str_dex_bonus(item);
            if sb != 0 {
                b = b.wrapping_add(w.stat(owner, st::STRENGTH, 0).wrapping_mul(sb) / 100);
            }
            if db != 0 {
                b = b.wrapping_add(w.stat(owner, st::DEXTERITY, 0).wrapping_mul(db) / 100);
            }
            let (lo, hi) = if w.item_flag_throw(item) {
                (st::ITEM_THROW_MINDAMAGE, st::ITEM_THROW_MAXDAMAGE)
            } else if w.has_inventory(owner) && w.wield_type(item) == 2 {
                (st::SECONDARY_MINDAMAGE, st::SECONDARY_MAXDAMAGE)
            } else {
                (st::MINDAMAGE, st::MAXDAMAGE)
            };
            if w.item_is(item, 57) {
                r.undead_pct = r.undead_pct.wrapping_add(50);
            }
            (
                w.stat(owner, lo, 0).wrapping_shl(8),
                w.stat(owner, hi, 0).wrapping_shl(8),
            )
        } else {
            // Step 6.2.
            if w.unit_type(owner) == UnitType::Monster && w.is_hireling(owner) {
                p = w.stat(owner, st::DEXTERITY, 0);
            }
            (
                w.stat(owner, st::MINDAMAGE, 0).wrapping_shl(8),
                w.stat(owner, st::MAXDAMAGE, 0).wrapping_shl(8),
            )
        };
        // Step 6.3.
        smin = bmin.wrapping_mul(s) / 128;
        smax = bmax.wrapping_mul(s) / 128;
        // Step 6.4.
        p = p
            .wrapping_add(w.stat(owner, st::DAMAGEPERCENT, 0))
            .wrapping_add(b);
        mn = w.stat(owner, st::MINDAMAGE_PERCENT, 0);
        mx = w.stat(owner, st::MAXDAMAGE_PERCENT, 0);
        sources(w, owner, &mut r);
        for (stat, f) in [
            (st::BYPASS_UNDEAD, rec_flag::BYPASS_UNDEAD),
            (st::BYPASS_DEMONS, rec_flag::BYPASS_DEMONS),
            (st::BYPASS_BEASTS, rec_flag::BYPASS_BEASTS),
        ] {
            if w.stat(owner, stat, 0) != 0 {
                r.flags |= f;
            }
        }
    } else if let (true, Some(o)) = (m != 0, origin) {
        // Step 7: origin-sourced.
        if bonus(w, t, o, None) {
            r.flags |= rec_flag::BONUS;
        }
        smin = w.stat(o, st::MINDAMAGE, 0).wrapping_mul(m) / 128;
        smax = w.stat(o, st::MAXDAMAGE, 0).wrapping_mul(m) / 128;
        p = w.stat(o, st::DAMAGEPERCENT, 0);
        mn = w.stat(o, st::MINDAMAGE_PERCENT, 0);
        mx = w.stat(o, st::MAXDAMAGE_PERCENT, 0);
        sources(w, o, &mut r);
    }
    // Step 8.
    r.damage_pct = p
        .wrapping_add(w.stat(missile, st::DAMAGEPERCENT, 0))
        .wrapping_add(mn.max(mx))
        .max(-90);
    r.phys_min = pmin.wrapping_add(smin);
    r.phys_max = pmax.wrapping_add(smax);
    // Step 9.
    match etype {
        1 => (r.fire_min, r.fire_max, r.fire_len) = (emin, emax, len),
        2 => (r.light_min, r.light_max) = (emin, emax),
        3 => (r.magic_min, r.magic_max) = (emin, emax),
        4 | 12 => (r.cold_min, r.cold_max, r.cold_len) = (emin, emax, len),
        5 => (r.poison_min, r.poison_max, r.poison_len) = (emin, emax, len),
        6 => (r.life_min, r.life_max) = (emin, emax),
        7 => (r.mana_min, r.mana_max) = (emin, emax),
        8 => (r.stam_min, r.stam_max) = (emin, emax),
        9 => r.stun_len = len,
        11 => (r.burn_min, r.burn_max, r.burn_len) = (emin, emax, len),
        _ => {}
    }
    // Step 10.
    if s != 0 {
        w.dual_wield_toggle(owner);
        add(w, owner, 8, &mut r);
        scale(&mut r, s);
    } else if let (true, Some(o)) = (m != 0, origin) {
        add(w, o, 0, &mut r);
        scale(&mut r, m);
    }
    r
}

/// +0x6C / +0x70 / +0x74 += stats 121 / 122 / 120 of `u`.
fn sources<W: SetupWorld>(w: &W, u: W::Unit, r: &mut SetupRecord) {
    r.demon_pct = r
        .demon_pct
        .wrapping_add(w.stat(u, st::DEMONDAMAGE_PERCENT, 0));
    r.undead_pct = r
        .undead_pct
        .wrapping_add(w.stat(u, st::UNDEADDAMAGE_PERCENT, 0));
    r.target_ac = r.target_ac.wrapping_add(w.stat(u, st::DAMAGETARGETAC, 0));
}

/// The stats. Returns the
/// missile data flag bits (+0x14) to set.
pub fn write_stats<W: SetupWorld>(
    w: &mut W,
    owner: Option<W::Unit>,
    missile: W::Unit,
    r: &SetupRecord,
    level: i32,
) -> u32 {
    if w.unit_type(missile) != UnitType::Missile {
        return 0;
    }
    let mut data = 0;
    if r.flags & rec_flag::OWNER != 0 {
        data |= 1;
    }
    if r.flags & rec_flag::UNUSED_4 != 0 {
        data |= 2;
    }
    for (stat, v) in [
        (st::LEVEL, level),
        (st::MINDAMAGE, r.phys_min),
        (st::MAXDAMAGE, r.phys_max),
        (st::FIREMINDAM, r.fire_min),
        (st::FIREMAXDAM, r.fire_max),
        (st::FIRELENGTH, r.fire_len),
        (st::LIGHTMINDAM, r.light_min),
        (st::LIGHTMAXDAM, r.light_max),
        (st::MAGICMINDAM, r.magic_min),
        (st::MAGICMAXDAM, r.magic_max),
        (st::COLDMINDAM, r.cold_min),
        (st::COLDMAXDAM, r.cold_max),
        (st::COLDLENGTH, r.cold_len),
        (st::POISONMINDAM, r.poison_min),
        (st::POISONMAXDAM, r.poison_max),
        (st::POISONLENGTH, r.poison_len),
        (st::POISON_COUNT, r.poison_count),
        (st::LIFEDRAINMINDAM, r.life_min),
        (st::LIFEDRAINMAXDAM, r.life_max),
        (st::MANADRAINMINDAM, r.mana_min),
        (st::MANADRAINMAXDAM, r.mana_max),
        (st::STAMDRAINMINDAM, r.stam_min),
        (st::STAMDRAINMAXDAM, r.stam_max),
        (st::STUNLENGTH, r.stun_len),
        (st::BURNINGMIN, r.burn_min),
        (st::BURNINGMAX, r.burn_max),
        // Edge case 1: written twice, the burn length wins.
        (st::FIRELENGTH, r.burn_len),
        (st::DEMONDAMAGE_PERCENT, r.demon_pct),
        (st::UNDEADDAMAGE_PERCENT, r.undead_pct),
        (st::DAMAGETARGETAC, r.target_ac),
        (st::DAMAGEPERCENT, r.damage_pct),
    ] {
        w.set_layer_stat(missile, stat, 0, v);
    }
    for (f, stat) in [
        (rec_flag::BYPASS_UNDEAD, st::BYPASS_UNDEAD),
        (rec_flag::BYPASS_DEMONS, st::BYPASS_DEMONS),
        (rec_flag::BYPASS_BEASTS, st::BYPASS_BEASTS),
        (rec_flag::BONUS, st::DEADLYSTRIKE),
    ] {
        if r.flags & f != 0 {
            w.set_layer_stat(missile, stat, 0, 1);
        }
    }
    if let Some(o) = owner {
        for (layer, v) in w.stat_entries(o, st::DAMAGE_VS_MONTYPE, 128) {
            w.set_layer_stat(missile, st::DAMAGE_VS_MONTYPE, layer, v);
        }
    }
    data
}

/// The wrapper: weapon switch, record, stats, restore.
/// Returns the missile data flag bits (+0x14) to set.
pub fn setup<W: SetupWorld>(
    w: &mut W,
    t: &SkillTables,
    owner: W::Unit,
    origin: Option<W::Unit>,
    missile: W::Unit,
    skill: i32,
    level: i32,
) -> u32 {
    w.dual_wield_switch(owner, false, true);
    let r = record(w, t, owner, origin, missile, skill, level);
    let data = write_stats(w, Some(owner), missile, &r, level);
    w.dual_wield_switch(owner, false, false);
    data
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
