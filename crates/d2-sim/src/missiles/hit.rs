//! `MISSMODE_SrvDmgHitHandler` and the missile-owned part of
//! the damage stage.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::{catalogue, stat, state, Ctx, MissileWorld, RowExt, UnitRef, SRV_DMG_COUNT};
use crate::combat::DamageRecord;

/// Hit result flags set by the missile.
pub mod result_flag {
    /// TODO: the bit values of the result flags are not
    /// fixed yet; these are D2RUST-local names for now.
    pub const HIT: u32 = 1 << 0;
    pub const GETHIT: u32 = 1 << 1;
    pub const SOFTHIT: u32 = 1 << 2;
    pub const KNOCKBACK: u32 = 1 << 3;
}

/// The damage record the missile fills, handed to
/// the server-damage function and the damage application.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    /// Rolled elements in order: physical, fire, magic, lightning,
    /// cold, poison, burning (8.8 fixed point).
    pub phys: i32,
    pub fire: i32,
    pub magic: i32,
    pub light: i32,
    pub cold: i32,
    pub poison: i32,
    pub burn: i32,
    pub cold_length: i32,
    pub poison_length: i32,
    pub life_drain: i32,
    pub mana_drain: i32,
    pub stamina_drain: i32,
    pub burn_length: i32,
    pub stun_length: i32,
    /// Deadly strike applied (phys doubled).
    pub crit: bool,
    /// Stats 103, 104, 106.
    pub ignore_target_ac: i32,
    pub fractional_target_ac: i32,
    pub ignore_target_defense: i32,
    /// Result flags of [`result_flag`].
    pub result: u32,
    /// Record fields only the server-damage functions write:
    /// freeze length, hit class and result-flag bits of
    /// the record (`combat::result`: soft hit 0x4000, knockback 8).
    pub freeze_length: i32,
    pub hit_class: Option<u32>,
    pub record_result: u16,
    /// Hit flags: the bypass bits of stats 103 / 104 / 106
    /// (stat bypass), cleared by `clear_elems`.
    pub hit_flags: u32,
}

impl Damage {
    /// The 0x70-byte record these fields are
    /// (the record is the damage record itself): deadly strike →
    /// result 0x2000.
    pub fn to_record(&self) -> DamageRecord {
        let mut rec = DamageRecord {
            hit_flags: self.hit_flags,
            result: self.record_result,
            physical: self.phys,
            fire: self.fire,
            burn: self.burn,
            burn_len: self.burn_length,
            lightning: self.light,
            magic: self.magic,
            cold: self.cold,
            poison: self.poison,
            poison_len: self.poison_length,
            cold_len: self.cold_length,
            freeze_len: self.freeze_length,
            life_leech: self.life_drain,
            mana_leech: self.mana_drain,
            stamina_leech: self.stamina_drain,
            stun_len: self.stun_length,
            ..DamageRecord::default()
        };
        if self.crit {
            rec.result |= crate::combat::result::CRITICAL;
        }
        if let Some(c) = self.hit_class {
            rec.hit_class = c;
        }
        rec
    }

    /// Takes the fields back from a record a server-damage function
    /// adjusted.
    fn take_record(&mut self, rec: &DamageRecord, hit_class: u32) {
        self.hit_flags = rec.hit_flags;
        self.record_result = rec.result & !crate::combat::result::CRITICAL;
        self.phys = rec.physical;
        self.fire = rec.fire;
        self.burn = rec.burn;
        self.burn_length = rec.burn_len;
        self.light = rec.lightning;
        self.magic = rec.magic;
        self.cold = rec.cold;
        self.poison = rec.poison;
        self.poison_length = rec.poison_len;
        self.cold_length = rec.cold_len;
        self.freeze_length = rec.freeze_len;
        self.life_drain = rec.life_leech;
        self.mana_drain = rec.mana_leech;
        self.stamina_drain = rec.stamina_leech;
        self.stun_length = rec.stun_len;
        if rec.hit_class != hit_class {
            self.hit_class = Some(rec.hit_class);
        }
    }
}

/// `pct(a, b) = a × b / 100` with the overflow-safe evaluation of
/// (exact 64-bit product; equal to the 32-bit shortcut
/// wherever that applies).
pub fn pct(a: i32, b: i32) -> i32 {
    (i64::from(a) * i64::from(b) / 100) as i32
}

/// One element roll: `max ≤ 0` or `min ≤ 0` → 0
/// with no draw; swap when min > max; mastery adds `pct` to both; result
/// `min + roll(max − min)` (`roll(0)` does not step).
pub fn damage_roll(seed: &mut crate::rng::Seed, min: i32, max: i32, mastery: i32) -> i32 {
    if max <= 0 || min <= 0 {
        return 0;
    }
    let (mut min, mut max) = if min > max { (max, min) } else { (min, max) };
    if mastery != 0 {
        min = min.wrapping_add(pct(min, mastery));
        max = max.wrapping_add(pct(max, mastery));
    }
    min.wrapping_add(seed.roll(max.wrapping_sub(min)) as i32)
}

/// elements in roll order: (min stat, max stat, mastery stat; 0 =
/// none).
pub const ELEMENTS: [(u16, u16, u16); 7] = [
    (stat::MINDAMAGE, stat::MAXDAMAGE, 0),
    (stat::FIREMINDAM, stat::FIREMAXDAM, stat::FIRE_MASTERY),
    (stat::MAGICMINDAM, stat::MAGICMAXDAM, stat::MAGIC_MASTERY),
    (stat::LIGHTMINDAM, stat::LIGHTMAXDAM, stat::LIGHT_MASTERY),
    (stat::COLDMINDAM, stat::COLDMAXDAM, stat::COLD_MASTERY),
    (stat::POISONMINDAM, stat::POISONMAXDAM, stat::POISON_MASTERY),
    (stat::BURNINGMINDAM, stat::BURNINGMAXDAM, stat::FIRE_MASTERY),
];

/// The damage record from the missile's stats, rolled
/// on the missile's own seed. Every stat, the masteries included, is the
/// missile's own total; `phys += phys × pct / 100` is a wrapping 32-bit
/// product, then a truncating signed ÷ 100.
pub fn fill_damage<W: MissileWorld + ?Sized>(
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> Damage {
    let mut rolled = [0i32; 7];
    for (i, &(min_s, max_s, mastery_s)) in ELEMENTS.iter().enumerate() {
        let max = cx.world.stat(m, max_s);
        let min = cx.world.stat(m, min_s);
        let mastery = if mastery_s == 0 {
            0
        } else {
            cx.world.stat(m, mastery_s)
        };
        rolled[i] = damage_roll(cx.world.seed(m), min, max, mastery);
    }
    let w = &*cx.world;
    let count = w.stat(m, stat::POISON_COUNT);
    let mut poison_length = w.stat(m, stat::POISONLENGTH);
    if count > 1 {
        poison_length /= count;
    }
    let mut d = Damage {
        phys: rolled[0],
        fire: rolled[1],
        magic: rolled[2],
        light: rolled[3],
        cold: rolled[4],
        poison: rolled[5],
        burn: rolled[6],
        cold_length: w.stat(m, stat::COLDLENGTH),
        poison_length,
        mana_drain: w.stat(m, stat::MANADRAINMINDAM),
        life_drain: w.stat(m, stat::LIFEDRAINMINDAM),
        stamina_drain: w.stat(m, stat::STAMDRAINMINDAM),
        burn_length: w.stat(m, stat::BURNINGLENGTH),
        stun_length: w.stat(m, stat::STUNLENGTH),
        ..Damage::default()
    };
    if let Some(u) = unit {
        let p = w
            .stat(m, stat::DAMAGEPERCENT)
            .wrapping_add(w.target_damage_bonus(m, u))
            .max(-90);
        d.phys = d.phys.wrapping_add(d.phys.wrapping_mul(p) / 100);
        if w.stat(m, stat::DEADLY_STRIKE) != 0 {
            d.crit = true;
            d.phys = d.phys.wrapping_mul(2);
        }
        d.ignore_target_ac = w.stat(m, stat::IGNORE_TARGET_AC);
        d.fractional_target_ac = w.stat(m, stat::FRACTIONAL_TARGET_AC);
        d.ignore_target_defense = w.stat(m, stat::IGNORE_TARGET_DEFENSE);
        // Bypass: 103 → 0x100, 104 → 0x200, 106 → 0x400 (undead, demons,
        // beasts).
        use crate::combat::hitflag;
        for (v, bit) in [
            (d.ignore_target_ac, hitflag::BYPASS_UNDEAD),
            (d.fractional_target_ac, hitflag::BYPASS_DEMONS),
            (d.ignore_target_defense, hitflag::BYPASS_BEASTS),
        ] {
            if v != 0 {
                d.hit_flags |= bit;
            }
        }
    }
    d
}

/// The missile-owned result flags: none without
/// an owner; "hit"; unless the unit has state 54: get-hit or soft-hit by
/// the row, then the knockback `roll(100)` on the missile's seed.
pub fn result_flags<W: MissileWorld + ?Sized>(
    game: &Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: UnitId,
) -> u32 {
    if cx.owner(game, m).is_none() {
        return 0;
    }
    let Some(row) = cx.row_of(m) else {
        return 0;
    };
    let (gethit, softhit, knockback) = (row.gethit, row.softhit, i32::from(row.knockback));
    let mut f = result_flag::HIT;
    if !cx.world.has_state(unit, state::UNINTERRUPTABLE) {
        if gethit {
            f |= result_flag::GETHIT;
        } else if softhit {
            f |= result_flag::SOFTHIT;
        }
        if knockback > 0 && (cx.world.seed(m).roll(100) as i32) < knockback {
            f |= result_flag::KNOCKBACK;
        }
    }
    f
}

/// The damage stage: fill, server-damage function, result flags,
/// damage application, then target armor for non-hireling monsters.
fn damage_stage<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: UnitId,
) {
    let mut dmg = fill_damage(cx, m, Some(unit));
    let srv_dmg = cx.row_of(m).map_or(0, |r| r.srv_dmg());
    if (1..SRV_DMG_COUNT).contains(&srv_dmg) {
        if srv_dmg <= 14 {
            // On the record itself.
            let mut rec = dmg.to_record();
            let class = rec.hit_class;
            super::srv_dmg::run(game, cx, srv_dmg, m, unit, &mut rec);
            dmg.take_record(&rec, class);
        } else {
            cx.store.unhandled.push(super::Unhandled::NullSrvDmg {
                index: srv_dmg,
                missile: m,
            });
        }
    }
    damage_tail(game, cx, m, unit, &mut dmg);
}

/// Result flags, the damage application,
/// then target armor for a non-hireling monster.
pub fn damage_tail<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: UnitId,
    dmg: &mut Damage,
) {
    // Adds its flags to the record's (a server-damage soft
    // hit or knockback stays).
    dmg.result |= result_flags(game, cx, m, unit);
    let owner = cx.owner(game, m);
    // TODO: whether the damage application runs without an owner.
    let data = cx.store.get(m).cloned();
    cx.world
        .apply_damage(game, cx.store, owner, m, unit, dmg, data.as_ref());
    if game
        .lists
        .unit(unit)
        .is_some_and(|e| e.ty == UnitType::Monster)
        && !cx.world.is_hireling(unit)
    {
        let delta = cx.world.stat(m, stat::ITEM_DAMAGETARGETAC);
        cx.world.add_target_ac(game, unit, delta);
    }
}

/// With `Pierce` and pierce_idx > 0, use one
/// (word 2); else word 3.
fn pierce_word<W: MissileWorld + ?Sized>(cx: &mut Ctx<'_, W>, m: UnitId, row_pierce: bool) -> u32 {
    let n = cx.world.stat(m, stat::PIERCE_IDX);
    if row_pierce && n > 0 {
        cx.world.set_stat(m, stat::PIERCE_IDX, n - 1);
        2
    } else {
        3
    }
}

/// `justhit`.
fn justhit<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    unit: Option<UnitId>,
    next_hit: bool,
    next_delay: i32,
) {
    if let (true, Some(u)) = (next_hit, unit) {
        // The unit is safe from the next hit for `next_delay` base ticks.
        let expire = game.after(next_delay);
        cx.world.apply_justhit(game, u, expire);
    }
}

/// `a4` is 1 for expiry or a barrier. Returns 2 when
/// the missile dies, else 1.
pub fn hit_handler<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
    a4: bool,
) -> i32 {
    // Step 1.
    let owner = cx.owner(game, m);
    let Some(row) = cx.row_of(m).cloned() else {
        // Unreachable (creation fails without a record), so
        // the record is asserted; release builds return 1.
        debug_assert!(false, "missile hit handler: no missiles.txt record");
        return 1;
    };
    let srv_hit = row.srv_hit();
    let next_hit = row.nexthit != 0;
    let next_delay = i32::from(row.nextdelay);
    let mut c: i32 = if row.explosion { 0 } else { 2 };
    let mut p: u32 = 3;
    // Step 2.
    if let Some(u) = unit {
        let Some(e) = game.lists.unit(u) else {
            return 1;
        };
        let uref = UnitRef {
            ty: e.ty,
            guid: e.guid,
        };
        if next_hit && cx.world.has_state(u, state::JUSTHIT) {
            return 1;
        }
        if row.lastcollide {
            let d = cx.store.get_mut(m);
            if let Some(d) = d {
                if d.last_collided == Some(uref) {
                    return 1;
                }
                d.last_collided = Some(uref);
            }
        }
        let mode = cx.store.get(m).map_or(0, |d| d.mode);
        let rejected = match mode {
            0 => true,
            1 => uref.ty != UnitType::Player,
            2 => uref.ty != UnitType::Monster,
            _ => false,
        };
        if rejected {
            return 1;
        }
        if let Some(o) = owner {
            if !cx.world.may_attack(o, u) && row.collidefriend == 0 {
                return 1;
            }
            if row.pierce
                && (cx.world.stat(o, stat::SKILL_PIERCE) != 0
                    || cx.world.stat(o, stat::ITEM_PIERCE) != 0)
            {
                p = pierce_word(cx, m, row.pierce);
            }
        }
    }
    // Step 3.
    if row.collidekill != 0 {
        c |= 1;
    }
    // Step 4.
    let skip_to_exit = unit.is_none() && !a4 && row.alwaysexplode == 0;
    if !skip_to_exit {
        // Step 5.
        if let (Some(u), true) = (unit, row.tohit != 0) {
            let hit = match owner {
                Some(o) => {
                    let tohit = cx.world.stat(m, stat::TOHIT);
                    cx.world.hit_test(game, o, u, tohit)
                }
                None => false,
            };
            if !hit {
                justhit(game, cx, unit, next_hit, next_delay);
                cx.world.hit_by_missile_event(game, m, unit);
                if row.alwaysexplode != 0 && catalogue::srv_hit_called(cx, srv_hit) {
                    let r = catalogue::run_srv_hit(game, cx, srv_hit, m, unit, c);
                    if r & 4 != 0 {
                        return 1;
                    }
                }
                return 2;
            }
        }
        // Step 6 (`p & 2` always holds: p is 2 or 3).
        justhit(game, cx, unit, next_hit, next_delay);
        cx.world.hit_by_missile_event(game, m, unit);
        if catalogue::srv_hit_called(cx, srv_hit) {
            c = catalogue::run_srv_hit(game, cx, srv_hit, m, unit, c);
            if c & 4 != 0 {
                return 1;
            }
        }
        // Step 7.
        if let (Some(u), true) = (unit, c & 2 != 0) {
            damage_stage(game, cx, m, u);
        }
    }
    // Step 8.
    if (c & 1 != 0 || a4) && p & 1 != 0 {
        cx.world.clear_footprint(game, m);
        return 2;
    }
    1
}
