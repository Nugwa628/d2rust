//! The shared helpers of the batch 3 bodies: monster mode damage
//! and minion skill damage, the jitter callback, potion codes, kick
//! damage and kick hits, the knockback column, the uninterruptable
//! state, the nearest unit, scatter, the progressive count, the claw
//! hit, the Leap helpers, weapon wear, the Charge helpers, the missile
//! burst, the skill result and element length, Plague, the Leap Attack
//! helpers, the base weapon roll, unit counts, pack and alignment
//! helpers, conversion, the Frenzy and Whirlwind helpers and the Blade
//! Shield pulse; plus the batch 2 / 3 remove callbacks and `area_damage`
//! over [`BodyWorld`].

use super::effects::{BodyEffect, PathOp};
use super::helpers::*;
use super::helpers2::*;
use super::{callback, BodyWorld, MissileRequest};
use crate::combat::{
    apply_melee, fill, melee_result, pct, start_combat, CombatTables, CombatWorld, DamageRecord,
};
use crate::rng::Seed;
use crate::skills::{
    bonus_level, highest_entry, kick_damage as kick_stats, phys_max, phys_min, roll_elemental,
    SkillEntry, SkillTables,
};
use crate::units::UnitType;

// -----------------------------------------------------------------

/// Player-count damage multiplier.
pub fn player_mult(d: usize, n: i32) -> i32 {
    const T: [i32; 9] = [0, 0, 8, 16, 24, 32, 40, 48, 56];
    if d == 0 || n < 2 {
        0
    } else if n < 9 {
        T[n as usize]
    } else {
        n.wrapping_mul(8).wrapping_sub(16)
    }
}

/// Minion skill damage: (min, max,
/// to-hit), zero when the unit has none.
pub fn minion_damage<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
) -> (i32, i32, i32) {
    if w.unit_type(u) != UnitType::Monster {
        return (0, 0, 0);
    }
    let Some(o) = w.minion_owner(u) else {
        return (0, 0, 0);
    };
    let Some(ms) = ct.monstats(w.class_id(u)) else {
        return (0, 0, 0);
    };
    let k = i32::from(ms.skilldamage as i16);
    if k <= 0 || k > t.skills.len() as i32 {
        return (0, 0, 0);
    }
    let list = w.skill_list(o);
    let lvl = match highest_entry(&list, k) {
        None => 0,
        Some(e) => {
            let mut l = e.base;
            if e.owner_guid == -1 {
                l = l.wrapping_add(bonus_level(w, t, o, &e));
            }
            l.max(0).min(t.level_cap)
        }
    };
    let mn = phys_min(w, t, Some(o), k, lvl, true) >> 8;
    let mx = phys_max(w, t, Some(o), k, lvl, true) >> 8;
    let th = skill_to_hit(w, t, o, k, lvl);
    (mn, mx, th)
}

/// Monster mode damage.
pub fn mode_damage<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    mode: i32,
) {
    if w.unit_type(u) != UnitType::Monster {
        return;
    }
    let Some(ms) = ct.monstats(w.class_id(u)).cloned() else {
        return;
    };
    let s = minion_damage(w, t, ct, u);
    let Some(l) = w.first_list_with_flags(u, 1) else {
        return;
    };
    w.list_clear(l);
    let d = w.combat().difficulty().min(2);
    let o = w.l_flag();
    let lv = w.stat(u, sid::LEVEL, 0);
    let mut mult = 0;
    if w.combat().alignment(u) == 0 {
        let n = w.stat(u, sid::MONSTER_PLAYERCOUNT, 0).max(1);
        mult = player_mult(w.combat().difficulty(), n);
    }
    let s16a = |a: [u16; 3]| i32::from(a[d] as i16);
    let (th0, mn0, mx0) = match mode {
        5 => (
            s16a([ms.a2th, ms.a2th_n, ms.a2th_h]),
            s16a([ms.a2mind, ms.a2mind_n, ms.a2mind_h]),
            s16a([ms.a2maxd, ms.a2maxd_n, ms.a2maxd_h]),
        ),
        7 | 8 => (
            s16a([ms.s1th, ms.s1th_n, ms.s1th_h]),
            s16a([ms.s1mind, ms.s1mind_n, ms.s1mind_h]),
            s16a([ms.s1maxd, ms.s1maxd_n, ms.s1maxd_h]),
        ),
        _ => (
            s16a([ms.a1th, ms.a1th_n, ms.a1th_h]),
            s16a([ms.a1mind, ms.a1mind_n, ms.a1mind_h]),
            s16a([ms.a1maxd, ms.a1maxd_n, ms.a1maxd_h]),
        ),
    };
    // The monlvl row of the monster stat init (level clamped to rows −
    // 1; a negative level gives nothing).
    let row = (lv >= 0 && !w.monlvl().is_empty())
        .then(|| w.monlvl()[(lv as usize).min(w.monlvl().len() - 1)].clone());
    let col = |a: [u32; 3], b: [u32; 3]| (if o { b } else { a })[d] as i32;
    let (dm, th_l) = match &row {
        Some(r) => (
            col([r.dm, r.dm_n, r.dm_h], [r.l_dm, r.l_dm_n, r.l_dm_h]),
            col([r.th, r.th_n, r.th_h], [r.l_th, r.l_th_n, r.l_th_h]),
        ),
        None => (0, 0),
    };
    let (mut mn, mut mx, mut th) = if row.is_none() {
        (0, 0, 0)
    } else if ms.noratio {
        (mn0, mx0, th0)
    } else {
        // Monster init flag 8: TH =
        // pct(monlvl TH / L-TH, the monstats to-hit, 100).
        (pct(dm, mn0, 100), pct(dm, mx0, 100), pct(th_l, th0, 100))
    };
    mn = mn.wrapping_add(s.0);
    mx = mx.wrapping_add(s.1);
    th = th.wrapping_add(s.2);
    let expansion = w.combat().expansion();
    let scale = |v: i32| v.wrapping_add(((i64::from(v) * i64::from(mult)) / 128) as i32);
    if expansion {
        if mult != 0 {
            mn = scale(mn);
            mx = scale(mx);
            th = scale(th);
        }
    } else if d != 0 && ms.align != 1 {
        mn = mn.wrapping_mul(10) / 12;
        mx = mx.wrapping_mul(10) / 12;
        th = th.wrapping_mul(10) / 15;
    }
    w.list_set(l, sid::MINDAMAGE, mn);
    w.list_set(l, sid::MAXDAMAGE, mx);
    w.list_set(l, i32::from(sid::TOHIT), th);
    let slots = [
        (
            ms.el1mode,
            ms.el1type,
            [ms.el1pct, ms.el1pct_n, ms.el1pct_h],
        ),
        (
            ms.el2mode,
            ms.el2type,
            [ms.el2pct, ms.el2pct_n, ms.el2pct_h],
        ),
        (
            ms.el3mode,
            ms.el3type,
            [ms.el3pct, ms.el3pct_n, ms.el3pct_h],
        ),
    ];
    for (emode, etype, epct) in slots {
        let p = i32::from(epct[d]);
        if emode == 0 || i32::from(emode) != mode || p == 0 {
            continue;
        }
        if p < 100 && (w.seed(u).roll(100) as i32) >= p {
            continue;
        }
        // Edge case 1: the El1 columns for every slot; `noRatio` gives 0.
        let dur = i32::from([ms.el1dur, ms.el1dur_n, ms.el1dur_h][d] as i16);
        let (mut emn, mut emx, mut len) = if ms.noratio || row.is_none() {
            (0, 0, dur)
        } else {
            (
                pct(
                    dm,
                    i32::from([ms.el1mind, ms.el1mind_n, ms.el1mind_h][d] as i16),
                    100,
                ),
                pct(
                    dm,
                    i32::from([ms.el1maxd, ms.el1maxd_n, ms.el1maxd_h][d] as i16),
                    100,
                ),
                dur,
            )
        };
        if expansion && mult != 0 {
            emn = scale(emn);
            emx = scale(emx);
            len = scale(len);
        }
        let mut ty = i32::from(etype);
        if ty == 10 {
            ty = w.seed(u).roll(5) as i32 + 1;
            if len == 0 {
                len = 25;
            }
        }
        let sets: &[(i32, i32)] = &match ty {
            1 => [(48, emn), (49, emx), (-1, 0)],
            2 => [(50, emn), (51, emx), (-1, 0)],
            3 => [(52, emn), (53, emx), (-1, 0)],
            4 => [(54, emn), (55, emx), (56, len)],
            5 => [
                (57, emn.wrapping_mul(10)),
                (58, emx.wrapping_mul(10)),
                (59, len.wrapping_mul(2)),
            ],
            6 => [(60, emn), (61, emx), (-1, 0)],
            7 => [(62, emn), (63, emx), (-1, 0)],
            8 => [(64, emn), (65, emx), (-1, 0)],
            9 => [(66, len), (-1, 0), (-1, 0)],
            11 => [(316, emn), (317, emx), (315, len)],
            _ => [(-1, 0); 3],
        };
        for &(s, v) in sets {
            if s >= 0 {
                w.list_set(l, s, v);
            }
        }
    }
}

// -----------------------------------------------------------------

/// What the jitter callback reads and writes on a missile (missile
/// data, path).
pub trait JitterMissile {
    type Missile: Copy;
    /// Total frames (missile data +0x0E).
    fn total_frames(&self, m: Self::Missile) -> i32;
    /// Total and current frames := v.
    fn set_frames(&mut self, m: Self::Missile, v: i32);
    /// The path's target x.
    fn path_target_x(&self, m: Self::Missile) -> i32;
    /// Missile seed (+0x20).
    fn set_seed(&mut self, m: Self::Missile, s: Seed);
    /// Path type.
    fn set_path_type(&mut self, m: Self::Missile, ty: u8);
    /// Step counts (capped 77).
    fn set_steps(&mut self, m: Self::Missile, n: i32);
    fn compute_path(&mut self, m: Self::Missile);
}

/// Jitter callback.
pub fn jitter<W: JitterMissile>(w: &mut W, m: W::Missile, a: u32) {
    let mut n = w.total_frames(m);
    if n >= 78 {
        n = 77;
        w.set_frames(m, 77);
    }
    let x = w.path_target_x(m) as u32;
    w.set_seed(m, Seed::init_low(x.wrapping_add(a)));
    w.set_path_type(m, 10);
    w.set_steps(m, n.min(77));
    w.compute_path(m);
}

// -----------------------------------------------------------------

/// Potion code table (rows i = act + 5·difficulty, columns
/// healing, mana, rejuvenation).
const POTIONS: [[&[u8; 4]; 3]; 15] = {
    const R0: [&[u8; 4]; 3] = [b"hp2 ", b"mp2 ", b"rvs "];
    const R1: [&[u8; 4]; 3] = [b"hp3 ", b"mp3 ", b"rvs "];
    const R3: [&[u8; 4]; 3] = [b"hp4 ", b"mp4 ", b"rvl "];
    const R6: [&[u8; 4]; 3] = [b"hp4 ", b"mp5 ", b"rvl "];
    const R7: [&[u8; 4]; 3] = [b"hp5 ", b"mp5 ", b"rvl "];
    [R0, R1, R1, R3, R3, R3, R6, R7, R7, R7, R7, R7, R7, R7, R7]
};

/// `potion_code(unit, game, skill)`; `None` for 0.
pub fn potion_code<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
) -> Option<[u8; 4]> {
    let room = w.unit_room(u)?;
    let a = w.room_act(room);
    if a > 4 {
        return None;
    }
    let i = a.wrapping_add(5 * w.combat().difficulty() as i32);
    let r = (w.seed(u).step() % 100) as i32;
    let (p3, p4) = (param(t, skill, 3), param(t, skill, 4));
    let c = if r < p3 {
        1
    } else if r < p3.wrapping_add(p4) {
        2
    } else {
        0
    };
    // Row 15 would read past the table (bound = 15); i ≤ 14
    // always.
    usize::try_from(i)
        .ok()
        .and_then(|i| POTIONS.get(i))
        .map(|row| *row[c])
}

// -----------------------------------------------------------------

/// Kick damage.
#[allow(clippy::too_many_arguments)]
pub fn kick_damage<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    record: &mut DamageRecord,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
) {
    charges_before(w, t, u, record);
    record.hit_flags |= 3;
    let mn = phys_min(w, t, Some(u), skill, lvl, false);
    let mx = phys_max(w, t, Some(u), skill, lvl, false);
    let e = record.enh_pct;
    let mn2 = mn.wrapping_add(pct(mn, e, 100));
    let mx2 = mx.wrapping_add(pct(mx, e, 100));
    let (mut kmin, mut kmax, mut e2) = (0, 0, e);
    kick_stats(w, u, &mut kmin, &mut kmax, &mut e2);
    kmin = kmin.wrapping_shl(8);
    kmax = kmax.wrapping_shl(8);
    let kmin2 = kmin.wrapping_add(pct(kmin, e2, 100));
    let kmax2 = kmax.wrapping_add(pct(kmax, e2, 100));
    let lo = kmin2.wrapping_add(mn2);
    let hi = kmax2.wrapping_add(mx2);
    let d = hi.wrapping_sub(lo);
    let x = if d < 1 { 0 } else { w.seed(u).roll(d) as i32 };
    record.physical = record.physical.wrapping_add(lo.wrapping_add(x));
    roll_elemental(w, t, u, record, skill, lvl);
    if !w.has_inventory(u) {
        return;
    }
    let mut wpn = w.weapon_in_use(u);
    let other = wpn.and_then(|i| {
        let loc = if w.body_loc(i) == 4 { 5 } else { 4 };
        w.item_at(u, loc)
    });
    let counts = |w: &W, i: W::Item| w.item_is(i, 45) && w.item_usable(i) && w.item_active(i);
    if let Some(i) = wpn {
        if counts(w, i) {
            w.effect(BodyEffect::ItemLists {
                u,
                item: i,
                on: false,
            });
        } else {
            wpn = None;
        }
    }
    if let Some(o) = other {
        if Some(o) != wpn && counts(w, o) {
            w.effect(BodyEffect::ItemLists {
                u,
                item: o,
                on: false,
            });
        }
    }
    fill(w.combat(), t, ct, u, tg, record, true, 128);
    charges_after(w, t, u, record);
    // Edge case 2: the other hand is not toggled back.
    if let Some(i) = wpn {
        w.effect(BodyEffect::ItemLists {
            u,
            item: i,
            on: true,
        });
    }
}

/// Kick hit.
#[allow(clippy::too_many_arguments)]
pub fn kick_hit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
    knock: bool,
) {
    let bonus =
        skill_to_hit(w, t, u, skill, lvl).wrapping_add(w.stat(u, sid::PROGRESSIVE_TOHIT, 0));
    let mut record = melee_rec(w, t, ct, u, tg, bonus, 0);
    if record.result & 1 != 0 {
        if knock {
            record.result |= 0xC;
        }
        record.enh_pct = if rec(t, skill).is_some() && lvl >= 1 {
            param(t, skill, 1).wrapping_add(lvl.wrapping_sub(1).wrapping_mul(param(t, skill, 2)))
        } else {
            0
        };
        kick_damage(w, t, ct, u, &mut record, tg, skill, lvl);
    }
    record.hit_class = 1;
    let s = rec(t, skill).map_or(128, srcdam_or_128);
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
}

// -----------------------------------------------------------------

/// The knockback chance by T's kind; `ordinary` is the value for
/// an ordinary target (`None`: `calc1`).
pub fn knock_chance<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
    ordinary: Option<i32>,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return ordinary.unwrap_or(0);
    };
    let (c1, c2, c3, c4) = (r.calc1, r.calc2, r.calc3, r.calc4);
    let ty = w.unit_type(tg);
    let col = if ty == UnitType::Player || w.combat().is_hireling(tg) {
        c4
    } else if ty == UnitType::Monster && w.combat().is_boss(tg) {
        c3
    } else if ty == UnitType::Monster && w.combat().monster_flag(tg, 8) {
        c2
    } else {
        match ordinary {
            Some(v) => return v,
            None => c1,
        }
    };
    // L' = clamp(T level + L − T level) = L (Edge case 3).
    eval(w, t, u, col, skill, lvl)
}

// -----------------------------------------------------------------

/// `set_uninterruptable(unit, v)`.
pub fn set_uninterruptable<W: BodyWorld>(w: &mut W, u: W::Unit, v: bool) {
    w.state_on(u, st::UNINTERRUPTABLE, v);
    if !v && w.has_state(u, st::DEATH_DELAY as u16) {
        w.state_on(u, st::DEATH_DELAY, false);
        match w.killer_of(u) {
            Some(k) => {
                let mut r = DamageRecord {
                    result: 2,
                    ..DamageRecord::default()
                };
                w.combat().reaction(k, u, &mut r);
            }
            None => w.effect(BodyEffect::Kill { u, a: 0, b: 1 }),
        }
    }
    if w.unit_type(u) == UnitType::Monster {
        w.delete_timers(u, 2, 0);
    }
}

// -----------------------------------------------------------------

/// Nearest accepted unit.
pub fn nearest<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    r: i32,
    test: &dyn Fn(&mut W, W::Unit) -> bool,
) -> Option<W::Unit> {
    let at = w.position(u);
    let mut best: Option<W::Unit> = None;
    let mut best_d = i32::MAX;
    scan_unit(w, t, ct, u, (0, 0), r, 0x8783, false, &mut |w, x| {
        if !test(w, x) {
            return 0;
        }
        let d = dist_sq(w.position(x), at);
        if d < best_d {
            best_d = d;
            best = Some(x);
        }
        1
    });
    best
}

// ------------------------------------------------------------------

/// Scatter.
#[allow(clippy::too_many_arguments)]
pub fn scatter<W: BodyWorld>(
    w: &mut W,
    u: W::Unit,
    m: i32,
    n: i32,
    r: i32,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    *w.seed(u) = Seed::init_low(tx as u32);
    let mut req = MissileRequest {
        flags: 0x420,
        origin: Some(u),
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    if n <= 1 || r < 2 {
        req.target_x = tx;
        req.target_y = ty;
        w.spawn_missile(req);
        return 1;
    }
    let pos = w.position(u);
    for _ in 0..n {
        let x = tx
            .wrapping_sub(r)
            .wrapping_add(w.seed(u).roll(r.wrapping_mul(2)) as i32);
        let y = ty
            .wrapping_sub(r)
            .wrapping_add(w.seed(u).roll(r.wrapping_mul(2)) as i32);
        req.target_x = x;
        req.target_y = y;
        if dist_sq(pos, (x, y)) >= 4 {
            w.spawn_missile(req);
        }
    }
    1
}

// ------------------------------------------------------------------

/// `prog_count(unit, skill, L)`.
pub fn prog_count<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let calcs = [r.prgcalc1, r.prgcalc2, r.prgcalc3];
    let (a, a1, prog) = (s16(r.aurastate), s16(r.aurastat1), r.progressive);
    let mut c = calcs[0];
    if prog && state_ok(w, a) && stat_ok(t, a1) {
        if let Some(l) = w.state_list(u, a) {
            let n = w.list_get(l, a1).clamp(1, 3);
            c = calcs[(n - 1) as usize];
        }
    }
    eval(w, t, u, c, skill, lvl)
}

// ------------------------------------------------------------------

/// Claw hit.
#[allow(clippy::too_many_arguments)]
pub fn claw_hit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let (calc1, srcdam) = (r.calc1, i32::from(r.srcdam));
    let bonus =
        skill_to_hit(w, t, u, skill, lvl).wrapping_add(w.stat(u, sid::PROGRESSIVE_TOHIT, 0));
    let mut record = melee_rec(w, t, ct, u, tg, bonus, 0);
    if record.result & 1 != 0 {
        record.hit_flags |= 2;
        charges_before(w, t, u, &mut record);
        record.enh_pct = record
            .enh_pct
            .wrapping_add(eval(w, t, u, calc1, skill, lvl));
        // The `roll_elemental` of this clause runs only with an `EType`
        // (as the elemental clause reads for the same wording).
        if convert(w, t, u, &mut record, skill, lvl) {
            roll_elemental(w, t, u, &mut record, skill, lvl);
        }
        fill(w.combat(), t, ct, u, tg, &mut record, false, srcdam);
        charges_after(w, t, u, &mut record);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, 128);
}

// ------------------------------------------------------------------

/// Entry flags of the Leap phases.
pub mod leap {
    pub const LAUNCH: u32 = 0x80;
    pub const FLIGHT: u32 = 0x1101;
    pub const LANDED: u32 = 0x200;
}

/// Leap room test.
pub fn leap_room_test<W: BodyWorld>(
    w: &W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    (x, y): (i32, i32),
) -> bool {
    let Some(r) = rec(t, skill) else {
        return false;
    };
    let Some(room) = w.unit_room(u).and_then(|ro| w.room_at(ro, x, y)) else {
        return false;
    };
    r.intown || !w.room_in_town(room)
}

/// Leap clamp: the landing point,
/// `None` after three failed candidates.
pub fn leap_clamp<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    (x, y): (i32, i32),
) -> Option<(i32, i32)> {
    let r = rec(t, skill)?.aurarangecalc;
    let (ux, uy) = w.position(u);
    let r = eval(w, t, u, r, skill, lvl);
    let d = distance((ux, uy), (x, y));
    if d < 2 {
        return Some((x, y));
    }
    let (mut x, mut y) = (x, y);
    if r < d {
        x = x.wrapping_sub(ux).wrapping_mul(r) / d + ux;
        y = y.wrapping_sub(uy).wrapping_mul(r) / d + uy;
    }
    let a = y.wrapping_sub(uy).clamp(-2, 2);
    let b = ux.wrapping_sub(x).clamp(-2, 2);
    let size = w.unit_size(u);
    for (cx, cy) in [(x, y), (x + a, y + b), (x - a, y - b)] {
        let Some(room) = w.unit_room(u).and_then(|ro| w.room_at(ro, cx, cy)) else {
            continue;
        };
        let Some((r2, p)) = w.free_point(room, (cx, cy), size, 0x1C09, true) else {
            continue;
        };
        if !w.pattern_collides(r2, p, u, 0x1C09)
            && !w.line_blocked(r2, (ux, uy), p, 0x804)
            && distance((ux, uy), p) <= r
        {
            return Some(p);
        }
    }
    None
}

/// Melee set-up with (unit, game) ("Monster pre-hit"): T'
/// := `target(game, unit)`, a zeroed record whose result is
/// `melee_result(game, unit, T', 0, 0)`, then `start_combat(game, unit,
/// T', record, 128)`. Also the monster attack event's melee
/// (the monster path of the skill use).
pub fn melee_setup<W: BodyWorld>(w: &mut W, t: &SkillTables, ct: &CombatTables, u: W::Unit) {
    let t2 = target(w, u);
    let mut record = DamageRecord {
        result: melee_result(w.combat(), t, ct, Some(u), t2, 0, 0),
        ..DamageRecord::default()
    };
    start_combat(w.combat(), t, ct, Some(u), t2, &mut record, 128);
}

/// Monster pre-hit.
pub fn monster_prehit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    e: &SkillEntry,
) -> i32 {
    mode_damage(w, t, ct, u, 4);
    melee_setup(w, t, ct, u);
    apply_melee(w.combat(), ct, u, tg);
    let (ty, g) = (type_index(w, tg), guid(w, tg));
    w.set_entry_param_of(u, e, 3, ty);
    w.set_entry_param_of(u, e, 4, g as i32);
    1
}

/// Leap launch.
pub fn leap_launch<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit, e: &SkillEntry) -> i32 {
    if !w.has_path(u) {
        return 0;
    }
    w.set_entry_flags(u, e, leap::FLIGHT);
    w.path_op(u, PathOp::MoveMask(0));
    w.path_op(u, PathOp::FootprintMask(0));
    let (x, y) = (w.entry_param(u, e, 1), w.entry_param(u, e, 2));
    if x == 0 || y == 0 {
        return 0;
    }
    w.path_op(u, PathOp::TargetPoint(x, y));
    w.path_op(u, PathOp::Type(9));
    let v = match w.unit_type(u) {
        UnitType::Monster => match ct.monstats(w.class_id(u)) {
            Some(m) => i32::from(m.run as i16),
            None => return 0,
        },
        _ => ct
            .charstats(w.class_id(u))
            .map_or(0, |c| i32::from(c.runvelocity)),
    };
    w.path_op(u, PathOp::Velocity(v.wrapping_shl(8)));
    w.path_op(u, PathOp::Compute);
    1
}

/// Landing message: state 18 off,
/// 0xA5 queued, the unit queued for update.
pub fn landing_msg<W: BodyWorld>(w: &mut W, u: W::Unit, skill: i32) {
    w.state_on(u, st::SKILL_MOVE, false);
    w.effect(BodyEffect::MsgA5 { u, skill });
    w.queue_update(u);
}

/// The monstats `BaseId` of a monster; −1 otherwise.
fn base_id<W: BodyWorld>(w: &W, ct: &CombatTables, u: W::Unit) -> i32 {
    if w.unit_type(u) != UnitType::Monster {
        return -1;
    }
    ct.monstats(w.class_id(u))
        .map_or(-1, |m| i32::from(m.baseid as i16))
}

/// Leap land.
pub fn leap_land<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit, e: &SkillEntry) -> i32 {
    if !w.has_path(u) {
        return 0;
    }
    let Some(room) = w.unit_room(u) else {
        return 0;
    };
    let (x, y) = (w.entry_param(u, e, 1), w.entry_param(u, e, 2));
    let b = base_id(w, ct, u);
    let player = w.unit_type(u) == UnitType::Player;
    let f = w.frame();
    if w.position(u) == (x, y) {
        if player {
            w.effect(BodyEffect::PatternClear {
                room,
                x,
                y,
                u,
                mask: 0x80,
            });
            w.path_op(u, PathOp::FootprintMask(0x80));
            w.path_op(u, PathOp::MoveMask(0x1C09));
            w.path_op(u, PathOp::Type(7));
            w.set_entry_flags(u, e, leap::LANDED);
            w.delete_timers(u, 1, 0);
            w.schedule(u, 1, f.wrapping_add(1), 0, 0);
        } else {
            w.effect(BodyEffect::PatternClear {
                room,
                x,
                y,
                u,
                mask: 0x100,
            });
            w.path_op(u, PathOp::FootprintMask(0x100));
            w.path_op(u, PathOp::MoveMask(0x3C01));
            w.path_op(u, PathOp::Type(2));
            if b == 78 {
                leap_hop(w, u, e, (x, y));
            } else {
                w.set_entry_flags(u, e, leap::LANDED);
                let c = w.frame_count(u);
                w.set_frame_count(u, c.wrapping_sub(0x100));
            }
        }
        set_uninterruptable(w, u, false);
        landing_msg(w, u, e.skill);
        return 1;
    }
    if player {
        w.anim_from(u, 10);
        return 0;
    }
    let i = match b {
        78 => 8,
        540 if distance(w.position(u), (x, y)) < 2 => 12,
        540 => 10,
        _ => 0,
    };
    w.set_frame_event_index(u, i);
    let c = w.frame_count(u);
    w.set_frame_count(u, c.wrapping_add(0x100));
    0
}

/// Sandleaper hop.
pub fn leap_hop<W: BodyWorld>(w: &mut W, u: W::Unit, e: &SkillEntry, (x, y): (i32, i32)) -> i32 {
    if !w.has_path(u) {
        return 0;
    }
    let (p3, p4) = (w.entry_param(u, e, 3), w.entry_param(u, e, 4));
    if w.find_unit(p3 as u32, p4 as u32).is_none() {
        return 0;
    }
    w.set_entry_flags(u, e, 1);
    w.set_frame_event_index(u, 12);
    let c = w.frame_count(u);
    w.set_frame_count(u, c.wrapping_add(0x100));
    w.path_op(u, PathOp::SnapCenter);
    w.path_op(u, PathOp::Steps(5));
    w.path_op(u, PathOp::Op649070(1));
    w.path_op(u, PathOp::TargetUnit(None));
    let (ux, uy) = w.position(u);
    let p = (
        x.wrapping_mul(3).wrapping_sub(ux.wrapping_mul(2)),
        y.wrapping_mul(3).wrapping_sub(uy.wrapping_mul(2)),
    );
    if let Some(room) = w.unit_room(u).and_then(|r| w.room_at(r, p.0, p.1)) {
        if w.room_in_town(room) {
            return 0;
        }
    }
    w.path_op(u, PathOp::TargetPoint(p.0, p.1));
    w.path_op(u, PathOp::Type(8));
    w.path_op(u, PathOp::Op648E40(5));
    w.path_op(u, PathOp::Compute);
    1
}

// ------------------------------------------------------------------

/// Weapon wear.
pub fn wear<W: BodyWorld>(w: &mut W, u: W::Unit, wpn: W::Item, chance: i32, amount: i32) {
    if !w.item_is(wpn, 45) {
        return;
    }
    if w.item_stackable(wpn) {
        if w.item_stat_of(wpn, sid::QUANTITY) > 0 {
            let r = (w.seed(u).step() % 100) as i32;
            if r < chance {
                dec_quantity(w, u);
            }
        }
    } else if w.item_breakable(wpn) {
        let r = (w.seed(u).step() % 100) as i32;
        if r < chance {
            let d = w.item_stat_of(wpn, sid::DURABILITY).wrapping_sub(amount);
            if d > 0 {
                w.set_item_stat(wpn, sid::DURABILITY, d);
                w.send_item_stat(u, wpn, sid::DURABILITY, d);
            } else {
                w.effect(BodyEffect::BreakItem { u, item: wpn });
            }
        }
    }
}

// ------------------------------------------------------------------

/// Charge swing.
pub fn swing<W: BodyWorld>(w: &mut W, u: W::Unit, tg: W::Unit) -> i32 {
    w.set_mode(u, 1);
    w.set_used_skill(u, None);
    match w.unit_type(u) {
        UnitType::Player => w.effect(BodyEffect::UnitModeRequest {
            u,
            skill: 0,
            mode: 7,
            target: tg,
        }),
        UnitType::Monster => {
            w.mode_request(u, 4, Some(tg));
        }
        _ => {}
    }
    1
}

/// Hit frame with (unit).
pub fn hit_frame<W: BodyWorld>(w: &W, u: W::Unit) -> i32 {
    match w.skill_sequence(u) {
        Some(s) if !s.is_empty() && s[0][5] == 0 => -1,
        _ => 7,
    }
}

// ------------------------------------------------------------------

/// Burst offsets (X, Y).
pub const BURST_X: [i32; 16] = [0, 1, 2, 2, 2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1];
pub const BURST_Y: [i32; 16] = [2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1, 0, 1, 2, 2];

/// Missile burst with (game, owner, origin, m, skill, L, step2,
/// step1, loops).
#[allow(clippy::too_many_arguments)]
pub fn burst<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    owner: W::Unit,
    origin: W::Unit,
    m: i32,
    skill: i32,
    lvl: i32,
    (step2, step1, loops): (i32, i32, i32),
) -> i32 {
    let Some(row) = t.missile(m) else {
        return 0;
    };
    let (p1, p2) = (row.param1 as i32, row.param2 as i32);
    let (x, y) = w.position(origin);
    let mut req = MissileRequest {
        flags: 0x17,
        origin: Some(origin),
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(owner, m)
    };
    if loops > 0 {
        req.flags = 0x1F;
        req.loops = loops;
    }
    req.velocity = p1.wrapping_shl(7);
    let s1 = step1.max(1) as usize;
    for i in (0..16).step_by(s1) {
        req.target_x = BURST_X[i];
        req.target_y = BURST_Y[i];
        w.spawn_missile(req);
    }
    if step2 != 0 {
        // A negative step never ends the
        // base game's loop (it walks memory until it faults); no base-game
        // caller passes one: a fatal assertion (ring 2 refused in a
        // release build).
        debug_assert!(
            step2 > 0,
            "negative burst step2 {step2} (bodies-2.md §2.16)"
        );
        if step2 < 0 {
            return 1;
        }
        req.velocity = p2.wrapping_shl(7);
        for i in (0..15).step_by(step2 as usize) {
            req.target_x = BURST_X[i + 1];
            req.target_y = BURST_Y[i + 1];
            w.spawn_missile(req);
        }
    }
    1
}

// ------------------------------------------------------------------

/// `skill_result(game, unit, T, R, skill, L, record, range)` =.
#[allow(clippy::too_many_arguments)]
pub fn skill_result<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
    record: &mut DamageRecord,
    range: i32,
) -> u16 {
    let rf = rec(t, skill).map_or(0, |r| r.resultflags);
    // Bit 0 (value 1) of ResultFlags skips the to-hit roll.
    let res = if rf & 1 != 0 {
        rf
    } else {
        let h = skill_to_hit(w, t, u, skill, lvl);
        let mut res = melee_result(w.combat(), t, ct, Some(u), Some(tg), h, range);
        if res & 1 != 0 {
            res |= rf;
        }
        res
    };
    record.result = res;
    res
}

/// `set_len(unit, record, len, skill)`.
pub fn set_len(t: &SkillTables, record: &mut DamageRecord, len: i32, skill: i32) {
    match t.skill(skill).map_or(0, |r| r.etype) {
        4 => record.cold_len = len,
        5 => record.poison_len = len,
        9 => record.stun_len = len,
        11 => record.burn_len = len,
        12 => record.freeze_len = len,
        _ => {}
    }
}

// ------------------------------------------------------------------

/// Plague.
#[allow(clippy::too_many_arguments)]
pub fn plague<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    tg: W::Unit,
    len: i32,
    skill: i32,
    lvl: i32,
) -> i32 {
    if lvl < 1 {
        return 0;
    }
    if infect(w, t, u, tg, len, skill, lvl) {
        spreader(w, t, u, tg, len, skill, lvl);
    }
    1
}

/// Infect.
fn infect<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    tg: W::Unit,
    len: i32,
    skill: i32,
    lvl: i32,
) -> bool {
    let Some(r) = rec(t, skill) else {
        return false;
    };
    let s = s16(r.auratargetstate);
    if s < 0 || s > w.state_count() {
        return false;
    }
    if w.state_list(tg, s).is_some() {
        return false;
    }
    // The infection lasts `len` base ticks.
    let e = super::after(w, len);
    let Some(l) = w.alloc_list(2, e, Some(tg)) else {
        return false;
    };
    w.set_list_state(l, s);
    w.set_remove_callback(l, callback::DEFAULT);
    w.attach(tg, l);
    w.state_on(tg, s, true);
    w.set_list_expire(l, e);
    w.combat().schedule_timer(tg, 12, e);
    aura_fill(w, t, u, l, skill, lvl);
    true
}

/// Spreader.
fn spreader<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    tg: W::Unit,
    len: i32,
    skill: i32,
    lvl: i32,
) {
    if rec(t, skill).is_none() {
        return;
    }
    let m = prog_missile(w, t, u, skill);
    if m <= 0 {
        return;
    }
    let made = w.spawn_missile(MissileRequest {
        flags: 0x8000,
        origin: Some(tg),
        skill,
        level: lvl,
        range: len,
        ..MissileRequest::new(tg, m)
    });
    if let Some(mm) = made {
        let (ty, g) = (type_index(w, u), guid(w, u));
        w.effect(BodyEffect::MissileData28 { missile: mm, v: ty });
        w.effect(BodyEffect::MissileData2C {
            missile: mm,
            v: g as i32,
        });
    }
}

// ------------------------------------------------------------------

/// Leap Attack aim.
pub fn leap_aim<W: BodyWorld>(w: &mut W, u: W::Unit, tg: Option<W::Unit>) -> Option<(i32, i32)> {
    if let Some(tg) = tg {
        if w.combat().in_melee_range(u, tg, 0) {
            return None;
        }
    }
    let (px, py) = match tg {
        Some(tg) => w.position(tg),
        None => w.path_target_point(u),
    };
    let (ux, uy) = w.position(u);
    let d = distance((ux, uy), (px, py));
    if d == 0 {
        return None;
    }
    let mut p = (
        px.wrapping_add(px.wrapping_sub(ux).wrapping_mul(2) / d),
        py.wrapping_add(py.wrapping_sub(uy).wrapping_mul(2) / d),
    );
    // No room: a fatal assertion; refused.
    let mut room = w.unit_room(u)?;
    if w.pattern_collides(room, p, u, 0x1C09) {
        let size = w.unit_size(u);
        let (r2, q) = w.free_point(room, (px, py), size, 0x1C09, false)?;
        room = r2;
        p = q;
    }
    if w.line_blocked(room, (ux, uy), p, 0x804) {
        return None;
    }
    let ok = w
        .unit_room(u)
        .and_then(|r| w.room_at(r, p.0, p.1))
        .is_some_and(|r| !w.room_in_town(r));
    ok.then_some(p)
}

/// Leap Attack pick.
pub fn leap_pick<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    e: Option<&SkillEntry>,
) -> Option<W::Unit> {
    let e = e?;
    let tg = target(w, u).filter(|&x| w.combat().in_melee_range(u, x, 0));
    let k = match tg {
        Some(k) => Some(k),
        None => {
            let p3 = w.entry_param(u, e, 3);
            let k = if p3 == 6 {
                None
            } else {
                let p4 = w.entry_param(u, e, 4);
                w.find_unit(p3 as u32, p4 as u32)
            };
            match k {
                Some(k) => Some(k),
                None => {
                    let r = w.combat().melee_range(u).wrapping_add(4);
                    next_unit(w, t, ct, u, (0, 0), r, 0x20003, u32::MAX).0
                }
            }
        }
    };
    let Some(k) = k else {
        w.set_entry_param_of(u, e, 3, 6);
        return None;
    };
    let (ty, g) = (type_index(w, k), guid(w, k));
    w.set_entry_param_of(u, e, 3, ty);
    w.set_entry_param_of(u, e, 4, g as i32);
    Some(k)
}

/// Leap Attack strike.
pub fn leap_strike<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (calc1, s, ov) = (r.calc1, srcdam_or_128(r), s16(r.srvoverlay));
    let Some(e) = w.find_entry(u, skill) else {
        return 0;
    };
    let Some(k) = leap_pick(w, t, ct, u, Some(&e)) else {
        return 0;
    };
    let mut record = skill_melee(w, t, ct, u, k, skill, lvl);
    // answer 7: on a hit, the conversion (inside its
    // `EType` clause), then `roll_elemental` always, `start_combat`,
    // `apply_melee`, overlay and stun removal; a miss does none of them.
    if record.result & 1 != 0 {
        record.result |= 8;
        record.hit_class = w.combat().weapon_hit_class(u);
        record.hit_flags |= 0x20;
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
        start_combat(w.combat(), t, ct, Some(u), Some(k), &mut record, s);
        apply_melee(w.combat(), ct, u, k);
        if ov >= 1 && ov <= w.overlay_count() {
            w.combat().overlay(k, ov);
        }
        if let Some(l) = w.state_list(k, st::STUNNED) {
            w.detach_free(k, l);
        }
    }
    1
}

// ------------------------------------------------------------------

/// Base weapon roll.
pub fn base_roll<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    let (mut mn, mut mx) = match w.current_weapon(u) {
        None => (
            w.stat(u, sid::MINDAMAGE as u16, 0).max(1),
            w.stat(u, sid::MAXDAMAGE as u16, 0).max(2),
        ),
        Some(i) => {
            let (a, b) = if w.wield_type(i) == 2 {
                (23, 24)
            } else {
                (21, 22)
            };
            (w.stat(u, a, 0), w.stat(u, b, 0))
        }
    };
    mn = mn.wrapping_shl(8);
    mx = mx.wrapping_shl(8);
    if mn < 1 {
        mn = 256;
    }
    if mx <= mn {
        mx = mn.wrapping_add(256);
    }
    mn.wrapping_add(w.seed(u).roll(mx.wrapping_sub(mn)) as i32)
}

// ------------------------------------------------------------------

/// `next_unit(game, source, x, y, r, f, g, &count)`:
/// the accepted unit with the smallest GUID > g,
/// else the one with the smallest GUID ≤ g; and the accepted count.
#[allow(clippy::too_many_arguments)]
pub fn next_unit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    source: W::Unit,
    at: (i32, i32),
    r: i32,
    f: u32,
    g: u32,
) -> (Option<W::Unit>, i32) {
    let mut above: Option<(u32, W::Unit)> = None;
    let mut below: Option<(u32, W::Unit)> = None;
    let mut count = 0;
    scan_unit(w, t, ct, source, at, r, f | 0xA783, false, &mut |w, x| {
        let id = guid(w, x);
        count += 1;
        if id > g {
            if above.is_none_or(|(b, _)| id < b) {
                above = Some((id, x));
            }
        } else if below.is_none_or(|(b, _)| id <= b) {
            below = Some((id, x));
        }
        1
    });
    (above.or(below).map(|(_, x)| x), count)
}

/// `count_units(game, source, x, y, r, f)`.
#[allow(clippy::too_many_arguments)]
pub fn count_units<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    source: W::Unit,
    at: (i32, i32),
    r: i32,
    f: u32,
) -> i32 {
    let mut n = 0;
    scan_unit(w, t, ct, source, at, r, f | 0xA783, false, &mut |_, _| {
        n += 1;
        1
    });
    n
}

/// `area_damage(game, owner, x, y, r, record, f)`
/// on the bodies' scan: every accepted unit gets
/// the per-unit area hit on a copy.
#[allow(clippy::too_many_arguments)]
pub fn area_damage<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    owner: W::Unit,
    at: (i32, i32),
    r: i32,
    record: &DamageRecord,
    f: u32,
) -> i32 {
    let f = if f == 0 { 0x8583 } else { f };
    scan_unit(w, t, ct, owner, at, r, f, false, &mut |w, x| {
        crate::missiles::bodies::area_hit(w.combat(), ct, owner, x, record);
        1
    });
    1
}

// ------------------------------------------------------------------

/// Leave pack.
pub fn leave_pack<W: BodyWorld>(w: &mut W, tg: W::Unit) {
    let Some(o) = w.pack_owner(tg) else {
        return;
    };
    if w.unit_type(o) != UnitType::Monster {
        return;
    }
    if o == tg {
        w.effect(BodyEffect::DissolvePack(tg));
    } else {
        w.effect(BodyEffect::LeaveLeader(tg));
    }
}

// ------------------------------------------------------------------

/// Conversion of T by the caster; `mind_blast` clears the aura
/// lists before the caster's path target.
#[allow(clippy::too_many_arguments)]
pub fn conversion<W: BodyWorld>(
    w: &mut W,
    tg: W::Unit,
    caster: W::Unit,
    s: i32,
    e: i32,
    cb: u32,
    mind_blast: bool,
) -> i32 {
    let l = match w.state_list(tg, s) {
        Some(l) => l,
        None => {
            let Some(l) = w.alloc_list(0x802, e, Some(caster)) else {
                return 0;
            };
            w.set_list_state(l, s);
            w.set_remove_callback(l, cb);
            w.attach(tg, l);
            w.state_on(tg, s, true);
            l
        }
    };
    w.set_list_expire(l, e);
    w.combat().schedule_timer(tg, 12, e);
    leave_pack(w, tg);
    w.effect(BodyEffect::Alignment { u: tg, a: 2, v: 1 });
    node_insert_owner(w, tg, caster);
    let clear_auras = |w: &mut W| {
        while let Some(a) = w.first_list_with_flags(tg, 8) {
            w.detach_free(tg, a);
        }
    };
    if mind_blast {
        clear_auras(w);
        w.path_op(caster, PathOp::TargetUnit(None));
    } else {
        w.path_op(caster, PathOp::TargetUnit(None));
        clear_auras(w);
    }
    let tl = w.stat(tg, sid::LEVEL, 0);
    let cl = w.stat(caster, sid::LEVEL, 0);
    if tl != 0 && cl < tl && w.state_list(tg, st::CONVERSION_SAVE).is_none() {
        if let Some(sl) = w.alloc_list(0, 0, Some(tg)) {
            w.set_list_state(sl, st::CONVERSION_SAVE);
            w.attach(tg, sl);
            let m = w.stat_max(tg, sid::LIFE) >> 8;
            let h = w.stat(tg, sid::LIFE, 0) >> 8;
            w.list_set(sl, sid::CONVERSION_LEVEL, tl);
            w.list_set(sl, sid::CONVERSION_MAXHP, m);
            let m2 = pct(m, cl, tl).wrapping_shl(8).max(1);
            let h2 = pct(h, cl, tl).wrapping_shl(8).clamp(1, m2);
            w.set_stat(tg, sid::LEVEL, cl);
            w.set_stat(tg, sid::LIFE, h2);
            w.set_stat(tg, sid::MAXHP, m2);
        }
    }
    1
}

/// Conversion / Mind Blast remove callbacks.
pub fn remove_conversion<W: BodyWorld>(w: &mut W, tg: W::Unit, s: i32, mind_blast: bool) {
    w.state_on(tg, s, false);
    w.effect(BodyEffect::Alignment { u: tg, a: 0, v: 1 });
    w.effect(BodyEffect::NodeRemove(tg));
    if let Some(l) = w.state_list(tg, st::CONVERSION_SAVE) {
        let lvl = w.list_get(l, sid::CONVERSION_LEVEL);
        let mm = w.list_get(l, sid::CONVERSION_MAXHP);
        let m = w.stat_max(tg, sid::LIFE) >> 8;
        let h = w.stat(tg, sid::LIFE, 0) >> 8;
        let mut v = if m != 0 { pct(mm, h, m) } else { 1 };
        if mind_blast {
            v = v.clamp(1, mm.max(1));
        }
        w.set_stat(tg, sid::LEVEL, lvl);
        w.set_stat(tg, sid::LIFE, v.wrapping_shl(8));
        // Edge case 22: Conversion restores the unshifted value.
        w.set_stat(
            tg,
            sid::MAXHP,
            if mind_blast { mm.wrapping_shl(8) } else { mm },
        );
        w.detach_free(tg, l);
        while let Some(a) = w.first_list_with_flags(tg, 8) {
            w.detach_free(tg, a);
        }
    }
}

/// Confuse / Attract remove callbacks: alignment
/// 0, state off, target list remove.
pub fn remove_alignment<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32) {
    w.effect(BodyEffect::Alignment { u, a: 0, v: 1 });
    w.state_on(u, s, false);
    w.effect(BodyEffect::NodeRemove(u));
}

/// Holy Freeze self-list remove callback.
pub fn remove_holy_freeze<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32) {
    let alive = w.is_alive(u);
    if alive || !w.stays_on_death(u, s) {
        w.state_on(u, s, false);
        if alive {
            w.state_on(u, st::SHATTER, false);
        }
        w.combat().refresh_anim_rate(u);
    }
}

/// Werewolf / Werebear remove callback.
pub fn remove_shape<W: BodyWorld>(w: &mut W, u: W::Unit, s: i32, l: W::List) {
    clear_group(w, u, s, true);
    let k = w.list_get(l, stat::MODIFIERLIST_SKILL);
    if let Some(e) = w.find_entry(u, k) {
        let m = w.disguise_mode(u, 10);
        w.set_entry_mode(u, &e, m);
    }
    w.state_on(u, s, false);
}

// ------------------------------------------------------------------

/// Frenzy charge.
pub fn frenzy_charge<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (a, len) = (s16(r.aurastate), r.auralencalc);
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if w.entry_param(u, &e, 1) == 0 {
        return 0;
    }
    if !state_ok(w, a) {
        return 1;
    }
    let len = eval(w, t, u, len, skill, lvl);
    let ex = super::after(w, len);
    let l = match w.state_list(u, a) {
        Some(l) => l,
        None => {
            let Some(l) = w.alloc_list(2, ex, Some(u)) else {
                return 0;
            };
            w.set_list_state(l, a);
            w.set_remove_callback(l, callback::DEFAULT);
            w.list_set(l, sid::SKILL_FRENZY, 0);
            w.attach(u, l);
            w.state_on(u, a, true);
            l
        }
    };
    w.set_list_expire(l, ex);
    w.combat().schedule_timer(u, 12, ex);
    w.mark_state_changed(u, a);
    let n = w.list_get(l, sid::SKILL_FRENZY).wrapping_add(1).min(lvl);
    w.list_set(l, sid::SKILL_FRENZY, n);
    aura_fill(w, t, u, l, skill, n);
    1
}

/// Frenzy swing.
#[allow(clippy::too_many_arguments)]
pub fn frenzy_swing<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: Option<W::Unit>,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (rf, hf, hc, calc1, s) = (
        r.resultflags,
        r.hitflags,
        r.hitclass,
        r.calc1,
        srcdam_or_128(r),
    );
    let Some(tg) = tg else {
        return 0;
    };
    w.path_op(u, PathOp::TargetUnit(Some(tg)));
    let mut record = skill_melee(w, t, ct, u, tg, skill, lvl);
    let hit = record.result & 1 != 0;
    if hit {
        record.result |= rf;
        record.hit_flags |= hf;
        if hc != 0 {
            record.hit_class = hc;
        }
        record.enh_pct = eval(w, t, u, calc1, skill, lvl);
        convert(w, t, u, &mut record, skill, lvl);
        roll_elemental(w, t, u, &mut record, skill, lvl);
    }
    start_combat(w.combat(), t, ct, Some(u), Some(tg), &mut record, s);
    apply_melee(w.combat(), ct, u, tg);
    if let Some(e) = w.used_skill(u) {
        w.set_entry_param_of(u, &e, 1, i32::from(hit));
    }
    1
}

// ------------------------------------------------------------------

/// Whirlwind pacing: the hits this do.
pub fn ww_pacing<W: BodyWorld>(w: &mut W, u: W::Unit, e: &SkillEntry) -> i32 {
    if w.unit_type(u) == UnitType::Monster {
        return ((w.seed(u).step() & 1) ^ 1) as i32;
    }
    if !w.combat().expansion() {
        return 1;
    }
    let f = w.frame();
    let n = w.entry_param(u, e, 4);
    if n == 0 {
        // The first strike waits 4 base ticks.
        let first = super::after(w, 4);
        w.set_entry_param_of(u, e, 4, first);
        return 1;
    }
    if f < n {
        return 0;
    }
    let d = match w.current_weapon(u) {
        None => 10,
        Some(i) => {
            // No AnimData frames: a fatal assertion; refused.
            let Some(fr) = w.attack_frames(u, i) else {
                return 0;
            };
            ww_gap(fr)
        }
    };
    // The gap between strikes is in base ticks.
    let d = super::ticks(w, d);
    let mut next = n.wrapping_add(d);
    if next < f {
        next = f.wrapping_add(d);
    }
    w.set_entry_param_of(u, e, 4, next);
    if w.two_melee_weapons(u) {
        2
    } else {
        1
    }
}

/// The Whirlwind hit gap by attack frames f (pacing).
pub fn ww_gap(f: i32) -> i32 {
    match f {
        _ if f < 12 => 4,
        _ if f < 15 => 6,
        _ if f < 18 => 8,
        _ if f < 20 => 10,
        _ if f < 23 => 12,
        _ if f <= 25 => 14,
        _ => 16,
    }
}

/// Whirlwind end.
pub fn ww_end<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    e: &SkillEntry,
    skill: i32,
    (x, y): (i32, i32),
) -> i32 {
    w.set_entry_flags(u, e, 0);
    set_uninterruptable(w, u, false);
    landing_msg(w, u, e.skill);
    if !w.has_path(u) {
        return 0;
    }
    let player = w.unit_type(u) == UnitType::Player;
    let k = if player { 0x80 } else { 0x100 };
    if x != 0 && y != 0 {
        if let Some(room) = w.unit_room(u) {
            w.effect(BodyEffect::PatternClear {
                room,
                x,
                y,
                u,
                mask: k,
            });
        }
    }
    w.path_op(u, PathOp::FootprintMask(k));
    w.path_op(u, PathOp::MoveMask(if player { 0x1C09 } else { 0x3C01 }));
    let a = rec(t, skill).map_or(-1, |r| s16(r.aurastate));
    if state_ok(w, a) {
        w.state_on(u, a, false);
        if let Some(l) = w.state_list(u, a) {
            w.detach_free(u, l);
        }
    }
    1
}

// ------------------------------------------------------------------

/// Blade Shield pulse (srvdo 142).
pub fn blade_pulse<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (range, hc, rf, hf, sd) = (
        r.aurarangecalc,
        r.hitclass,
        r.resultflags,
        r.hitflags,
        i32::from(r.srcdam),
    );
    let mut rr = prog_count(w, t, u, skill, lvl);
    if rr == 0 {
        rr = eval(w, t, u, range, skill, lvl);
    }
    let mut d = DamageRecord {
        hit_flags: 2,
        ..DamageRecord::default()
    };
    crate::skills::roll_physical(w, t, u, &mut d, skill, lvl);
    roll_elemental(w, t, u, &mut d, skill, lvl);
    if hc != 0 {
        d.hit_class = hc;
    }
    let h = if rf & 2 == 0 {
        skill_to_hit(w, t, u, skill, lvl)
    } else {
        0
    };
    let filter = rec(t, skill).map_or(0, |r| r.aurafilter);
    scan_unit(w, t, ct, u, (0, 0), rr, filter, false, &mut |w, x| {
        let mut c = d;
        c.result = if rf & 2 != 0 {
            rf
        } else {
            let mut res = melee_result(w.combat(), t, ct, Some(u), Some(x), h, 1);
            if res & 1 != 0 {
                res |= rf;
            }
            res
        };
        if c.result & 1 != 0 {
            c.hit_flags |= hf;
            if sd != 0 {
                fill(w.combat(), t, ct, u, x, &mut c, false, sd);
            }
            start_combat(w.combat(), t, ct, Some(u), Some(x), &mut c, 128);
            apply_melee(w.combat(), ct, u, x);
        }
        1
    });
    1
}

/// Target-list prepend: only for a
/// player or monster in no list.
pub fn node_prepend<W: BodyWorld>(w: &mut W, u: W::Unit, slot: i32) {
    if w.node_slot(u) != NO_NODE || !matches!(w.unit_type(u), UnitType::Player | UnitType::Monster)
    {
        return;
    }
    w.effect(BodyEffect::NodePrepend { u, slot });
}

/// `ln12` of the skill: `Param1` + (L − 1)·`Param2`, 0
/// for L ≤ 0.
pub fn ln12(t: &SkillTables, skill: i32, lvl: i32) -> i32 {
    if lvl <= 0 {
        return 0;
    }
    param(t, skill, 1).wrapping_add(lvl.wrapping_sub(1).wrapping_mul(param(t, skill, 2)))
}
