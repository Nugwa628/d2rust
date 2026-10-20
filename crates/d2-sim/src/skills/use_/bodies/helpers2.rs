//! The shared helpers of the batch 2 bodies: summon class and spawn,
//! the target-node insert, summon base / skill stats, the progressive
//! missile, the missile ring, the shout state, the sentry spawn, the
//! charge counter, golem stats and summon resistance, the free target
//! point, the missile at the target point, the Paladin raise penalty,
//! skeleton components, Inferno, the missile fan, shadow stats and the
//! source-unit link. Plus small shared pieces (target position,
//! distance, record helpers) the batch 2 and 3 bodies reuse.

use super::effects::BodyEffect;
use super::helpers::*;
use super::{callback, init_cb, BodyWorld, MissileRequest};
use crate::combat::{apply, melee_result, pct, CombatTables, CombatWorld, DamageRecord};
use crate::skills::{to_hit, SkillTables};
use crate::units::UnitType;
use d2_data::tables::Skills;

/// Monster mode 12 (dead), the corpse tests' mode.
pub const MODE_DEAD: u32 = 12;
/// Target-list slot "in no list" (unit +0xD0).
pub const NO_NODE: i32 = 11;

/// States of batch 2 and 3.
pub mod st {
    pub const INFERNO: i32 = 12;
    pub const SKILL_MOVE: i32 = 18;
    pub const STUNNED: i32 = 21;
    pub const UNINTERRUPTABLE: i32 = 54;
    pub const CONVERSION: i32 = 53;
    pub const DEATH_DELAY: i32 = 92;
    pub const VALKYRIE: i32 = 93;
    pub const REVIVE: i32 = 96;
    pub const SKEL_MASTERY: i32 = 97;
    pub const SOURCEUNIT: i32 = 98;
    pub const REDEEMED: i32 = 99;
    pub const HOLYSHIELD: i32 = 101;
    pub const CORPSE_NODRAW: i32 = 104;
    pub const SHATTER: i32 = 107;
    pub const CONVERSION_SAVE: i32 = 109;
    pub const CORPSE_NOSELECT: i32 = 118;
    pub const VINE_BEAST: i32 = 150;
}

/// Stats of batch 2 and 3.
pub mod sid {
    pub const LIFE: u16 = 6;
    pub const MAXHP: u16 = 7;
    pub const MANA: u16 = 8;
    pub const LEVEL: u16 = 12;
    pub const TOHIT: u16 = 19;
    pub const MINDAMAGE: i32 = 21;
    pub const MAXDAMAGE: i32 = 22;
    pub const DAMAGEPERCENT: u16 = 25;
    pub const ARMORCLASS: u16 = 31;
    pub const HPREGEN: u16 = 74;
    pub const DURABILITY: u16 = 72;
    pub const QUANTITY: u16 = 70;
    pub const VELOCITYPERCENT: u16 = 67;
    pub const MONSTER_PLAYERCOUNT: u16 = 100;
    pub const ABSORB_FIRE_PCT: u16 = 142;
    pub const ABSORB_LIGHT_PCT: u16 = 144;
    pub const ABSORB_COLD_PCT: u16 = 148;
    pub const SKILL_FRENZY: i32 = 169;
    pub const CONVERSION_LEVEL: i32 = 176;
    pub const CONVERSION_MAXHP: i32 = 177;
    pub const PROGRESSIVE_TOHIT: u16 = 325;
    pub const PASSIVE_FIRE_MASTERY: u16 = 329;
    pub const PASSIVE_LTNG_MASTERY: u16 = 330;
    pub const PASSIVE_COLD_MASTERY: u16 = 331;
    pub const PASSIVE_SUMMON_RESIST: u16 = 349;
    pub const SOURCE_UNIT_TYPE: i32 = 353;
    pub const SOURCE_UNIT_ID: i32 = 354;
}

// ---------------------------------------------------------------- shared

/// Target position: `None` when it fails or either
/// coordinate is 0.
pub fn tpos<W: BodyWorld>(w: &W, u: W::Unit) -> Option<(i32, i32)> {
    w.target_position(u).filter(|&(x, y)| x != 0 && y != 0)
}

/// Distance: max(|dx|, |dy|) + min(|dx|, |dy|) / 2.
pub fn distance((ax, ay): (i32, i32), (bx, by): (i32, i32)) -> i32 {
    let dx = ax.wrapping_sub(bx).wrapping_abs();
    let dy = ay.wrapping_sub(by).wrapping_abs();
    dx.max(dy).wrapping_add(dx.min(dy) / 2)
}

/// Squared distance.
pub fn dist_sq((ax, ay): (i32, i32), (bx, by): (i32, i32)) -> i32 {
    let (dx, dy) = (ax.wrapping_sub(bx), ay.wrapping_sub(by));
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

/// The unit's (type, GUID).
pub fn ident<W: BodyWorld>(w: &mut W, u: W::Unit) -> (UnitType, u32) {
    w.combat().ident(u)
}

/// The unit's GUID.
pub fn guid<W: BodyWorld>(w: &mut W, u: W::Unit) -> u32 {
    ident(w, u).1
}

/// The unit's type index (0 player … 5 tile).
pub fn type_index<W: BodyWorld>(w: &mut W, u: W::Unit) -> i32 {
    ident(w, u).0.index() as i32
}

/// Unit flags (+0xC4) |= bits.
pub fn flags_or<W: BodyWorld>(w: &mut W, u: W::Unit, bits: u32) {
    let f = w.unit_flags(u);
    w.set_unit_flags(u, f | bits);
}

/// Unit flags (+0xC4) &= !bits.
pub fn flags_clear<W: BodyWorld>(w: &mut W, u: W::Unit, bits: u32) {
    let f = w.unit_flags(u);
    w.set_unit_flags(u, f & !bits);
}

/// 0 ≤ m < missiles count.
pub fn missile_ok(t: &SkillTables, m: i32) -> bool {
    usize::try_from(m).is_ok_and(|i| i < t.missiles.len())
}

/// `SrcDam`, or 128 when 0.
pub fn srcdam_or_128(r: &Skills) -> i32 {
    match i32::from(r.srcdam) {
        0 => 128,
        s => s,
    }
}

/// The skill's `ParamN` (1…8) as an i32; 0 for an invalid skill.
pub fn param(t: &SkillTables, skill: i32, n: u8) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    (match n {
        1 => r.param1,
        2 => r.param2,
        3 => r.param3,
        4 => r.param4,
        5 => r.param5,
        6 => r.param6,
        7 => r.param7,
        _ => r.param8,
    }) as i32
}

/// A zeroed record with `melee_result(game, unit, T, bonus, range)`.
#[allow(clippy::too_many_arguments)]
pub fn melee_rec<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    bonus: i32,
    range: i32,
) -> DamageRecord {
    DamageRecord {
        result: melee_result(w.combat(), t, ct, Some(u), Some(tg), bonus, range),
        ..DamageRecord::default()
    }
}

/// `to_hit(unit, skill, L)`.
pub fn skill_to_hit<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    to_hit(w, t, Some(u), skill, lvl)
}

/// The batch 3 conversion: `EType` ≠ 0 → c =
/// `eval(calc4)`; conversion % := c; c > 0 → conversion element :=
/// `EType`. Returns whether `EType` ≠ 0.
pub fn convert<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    record: &mut DamageRecord,
    skill: i32,
    lvl: i32,
) -> bool {
    let Some(r) = rec(t, skill) else {
        return false;
    };
    let (etype, calc4) = (r.etype, r.calc4);
    if etype == 0 {
        return false;
    }
    let c = eval(w, t, u, calc4, skill, lvl);
    record.conv_pct = c;
    if c > 0 {
        record.conv_elem = etype as i8;
    }
    true
}

/// `auraevent1–3` / `auraeventfunc1–3` (i16 views).
pub fn aura_events(r: &Skills) -> [(i32, i32); 3] {
    [
        (s16(r.auraevent1), s16(r.auraeventfunc1)),
        (s16(r.auraevent2), s16(r.auraeventfunc2)),
        (s16(r.auraevent3), s16(r.auraeventfunc3)),
    ]
}

/// The event step of the buff bodies:
/// `auraevent1` ≥ 0 → unregister (1, state); for i = 1…3 while
/// `auraevent_i` ≥ 0: register (event_i, skill, L, func_i, 1, state).
pub fn reregister<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    state: i32,
) {
    let Some(r) = rec(t, skill) else {
        return;
    };
    let ev = aura_events(r);
    if ev[0].0 < 0 {
        return;
    }
    unregister(w, u, 1, state);
    for (e, f) in ev {
        if e < 0 {
            break;
        }
        register(w, u, e, skill, lvl, f, 1, state);
    }
}

/// The unit's list of state `s`, or a new one (flags, expire, owner
/// `owner`, state set, callback, attached, state on). `None`: the alloc
/// failed.
#[allow(clippy::too_many_arguments)]
pub fn list_or_new<W: BodyWorld>(
    w: &mut W,
    u: W::Unit,
    s: i32,
    flags: u32,
    expire: i32,
    owner: W::Unit,
    cb: u32,
) -> Option<W::List> {
    if let Some(l) = w.state_list(u, s) {
        return Some(l);
    }
    let l = w.alloc_list(flags, expire, Some(owner))?;
    w.set_list_state(l, s);
    w.set_remove_callback(l, cb);
    w.attach(u, l);
    w.state_on(u, s, true);
    Some(l)
}

// -----------------------------------------------------------------

/// `summon_class(unit, skill, L, &mode)`: the
/// monster class (−1 none) and the mode.
pub fn summon_class<W: BodyWorld>(
    w: &W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
) -> (i32, i32) {
    let count = ct.monstats.len() as i32;
    // The summon mode read writes `mode` only with a
    // valid class from `summon`; R invalid or `summon` outside 0…count − 1
    // leave the caller's variable unwritten, and every caller's is an
    // uninitialised local: D2RUST gives them 0 (no base-game row reaches it).
    let (c, mode) = match rec(t, skill) {
        Some(r) => {
            let c = s16(r.summon);
            if c < 0 || c >= count {
                (-1, 0)
            } else {
                let m = i32::from(r.summode as i8);
                (c, if (0..=15).contains(&m) { m } else { 1 })
            }
        }
        None => (-1, 0),
    };
    if (0..count).contains(&c) {
        return (c, mode);
    }
    // The AI control's spawn class (a non-monster reads
    // through a null pointer, Edge case 11: refused).
    match w.minion_spawn_class(u) {
        Some(c2) if c2 > 0 && c2 < count => (c2, mode),
        _ => (-1, mode),
    }
}

// -----------------------------------------------------------------

/// The summon request (D2MOO `D2SummonArgStrc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summon<U> {
    /// 1 position given, 2 replace the linked unit, 4 no second try, 8
    /// keep unit flag 0x80000000 clear.
    pub flags: u32,
    pub owner: U,
    pub class: i32,
    pub ai: i32,
    pub mode: i32,
    pub x: i32,
    pub y: i32,
    pub pet_type: i32,
    pub pet_max: i32,
}

/// Summon spawn.
pub fn spawn<W: BodyWorld>(w: &mut W, q: Summon<W::Unit>) -> Option<W::Unit> {
    let o = q.owner;
    let (x, y) = if q.flags & 1 != 0 {
        (q.x, q.y)
    } else {
        // Its result is not tested.
        w.target_position(o).unwrap_or((q.x, q.y))
    };
    let room = w.unit_room(o).and_then(|r| w.room_at(r, x, y))?;
    let m = match w.create_monster(room, (x, y), q.class, q.mode, -1) {
        Some(m) => m,
        None if q.flags & 4 != 0 => return None,
        None => w.create_monster(room, (x, y), q.class, q.mode, 4)?,
    };
    // Finish.
    if q.flags & 8 == 0 {
        flags_or(w, m, 0x8000_0000);
    }
    if q.flags & 2 != 0 {
        if let Some(k) = w.linked_unit(o) {
            if w.unit_c8(k) & 0x100 != 0 {
                w.effect(BodyEffect::RemoveUnit(k));
            } else {
                flags_or(w, k, 0x400_0000);
                w.effect(BodyEffect::KillReplaced(k));
            }
            w.effect(BodyEffect::SetLinked { owner: o, m: None });
        }
        w.effect(BodyEffect::SetLinked {
            owner: o,
            m: Some(m),
        });
    }
    w.effect(BodyEffect::PetAdd {
        owner: o,
        pet: m,
        t: q.pet_type,
        max: q.pet_max.max(1),
    });
    flags_or(w, m, 0x2_0000);
    w.effect(BodyEffect::OwnerData {
        m,
        owner: Some(o),
        a: 0,
        b: 0,
    });
    if w.has_state(m, st::UNINTERRUPTABLE as u16) {
        // A fatal assertion in the base game; D2RUST refuses.
        return None;
    }
    w.set_ai_state(m, q.ai);
    w.effect(BodyEffect::AiRefresh(m));
    w.delete_timers(m, 2, 0);
    // The summon first thinks one second (25 base ticks) later.
    w.schedule(m, 2, super::after(w, 25), 0, 0);
    Some(m)
}

// -----------------------------------------------------------------

/// Joins `m` to the player target list `slot` names (a summon joins
/// its owner's list).
pub fn node_insert<W: BodyWorld>(w: &mut W, m: W::Unit, slot: i32) {
    if w.node_slot(m) != NO_NODE
        || u32::try_from(slot).map_or(true, |s| crate::game::node::player_list(s).is_none())
        || !matches!(w.unit_type(m), UnitType::Player | UnitType::Monster)
    {
        return;
    }
    w.effect(BodyEffect::NodeInsert { m, slot });
}

/// `node_insert(game, m, 0, owner +0xD0)`.
pub fn node_insert_owner<W: BodyWorld>(w: &mut W, m: W::Unit, owner: W::Unit) {
    let slot = w.node_slot(owner);
    node_insert(w, m, slot);
}

// -----------------------------------------------------------------

/// Summon base stats.
pub fn base_stats<W: BodyWorld>(w: &mut W, owner: W::Unit, m: W::Unit, p: i32, lvl: i32) -> i32 {
    let mut p = p;
    if p <= 0 {
        let c = w.stat(owner, sid::LEVEL, 0);
        p = lvl.wrapping_add(c.wrapping_mul(3) / 4);
        if p < 1 {
            p = 1;
        }
        if p >= c {
            p = c;
        }
    }
    if p != 0 {
        w.set_stat(m, sid::LEVEL, p);
    }
    let count = w.monlvl().len() as i32;
    let i = p.min(count - 1);
    if !(0..count).contains(&i) {
        return 0;
    }
    let d = w.combat().difficulty().min(2);
    let e = w.l_flag();
    let row = &w.monlvl()[i as usize];
    let pick = |a: [u32; 3], l: [u32; 3]| (if e { l } else { a })[d] as i32;
    let ac = pick(
        [row.ac, row.ac_n, row.ac_h],
        [row.l_ac, row.l_ac_n, row.l_ac_h],
    );
    let th = pick(
        [row.th, row.th_n, row.th_h],
        [row.l_th, row.l_th_n, row.l_th_h],
    );
    w.add_stat(m, sid::ARMORCLASS, ac);
    w.add_stat(m, sid::TOHIT, th);
    1
}

// -----------------------------------------------------------------

/// The stat event step of the summon stats' step 2 for stat `s` on m (Edge case 10:
/// looked up by `s`, registered with `s << 16`).
fn stat_event<W: BodyWorld>(w: &mut W, m: W::Unit, s: i32) {
    let Some(info) = w.stat_info(s) else {
        return;
    };
    let key = guid(w, m) as i32;
    // `itemevent2` is registered only inside
    // the `itemevent1` > 0 and "no handler found" branch.
    if info.itemevent[0] > 0 && !w.has_handler(m, 2, key, s) {
        register(
            w,
            m,
            info.itemevent[0],
            s.wrapping_shl(16),
            0,
            info.itemeventfunc[0],
            2,
            key,
        );
        if info.itemevent[1] > 0 {
            register(
                w,
                m,
                info.itemevent[1],
                s.wrapping_shl(16),
                0,
                info.itemeventfunc[1],
                2,
                key,
            );
        }
    }
}

/// Summon skill stats; formulas on the owner.
#[allow(clippy::too_many_arguments)]
pub fn skill_stats<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    owner: W::Unit,
    m: W::Unit,
    skill: i32,
    lvl: i32,
    ilvl: i32,
) -> i32 {
    if skill == 0 {
        return 0;
    }
    // An invalid skill reads a null record: refused.
    let Some(r) = rec(t, skill).cloned() else {
        return 0;
    };
    let passive = [
        (r.passivestat1, r.passivecalc1),
        (r.passivestat2, r.passivecalc2),
        (r.passivestat3, r.passivecalc3),
        (r.passivestat4, r.passivecalc4),
        (r.passivestat5, r.passivecalc5),
    ];
    for (s, c) in passive {
        let s = s16(s);
        if !stat_ok(t, s) {
            continue;
        }
        let v = eval(w, t, owner, c, skill, lvl);
        w.add_stat(m, s as u16, v);
        stat_event(w, m, s);
    }
    let aura = [
        (r.aurastat1, r.aurastatcalc1),
        (r.aurastat2, r.aurastatcalc2),
        (r.aurastat3, r.aurastatcalc3),
        (r.aurastat4, r.aurastatcalc4),
        (r.aurastat5, r.aurastatcalc5),
        (r.aurastat6, r.aurastatcalc6),
    ];
    let mut list = None;
    for (s, c) in aura {
        let s = s16(s);
        if !stat_ok(t, s) {
            continue;
        }
        let v = eval(w, t, owner, c, skill, lvl);
        if v == 0 {
            continue;
        }
        let l = match list {
            Some(l) => l,
            None => {
                let Some(l) = w.alloc_list(0, 0, Some(m)) else {
                    return 0;
                };
                w.attach(m, l);
                list = Some(l);
                l
            }
        };
        w.list_set(l, s, v);
        stat_event(w, m, s);
    }
    let a = s16(r.aurastate);
    // Edge case 12: the count itself is accepted.
    if a >= 1 && a <= w.state_count() {
        w.state_on(m, a, true);
        if let Some(l) = list {
            w.set_list_state(l, a);
        }
    }
    let h = w.stat_max(m, sid::LIFE);
    let p = eval(w, t, owner, r.calc1, skill, lvl);
    let h2 = h.wrapping_add(pct(h, p, 100));
    w.set_stat(m, sid::MAXHP, h2);
    w.set_stat(m, sid::LIFE, h2);
    let sum = [
        (r.sumskill1, r.sumsk1calc),
        (r.sumskill2, r.sumsk2calc),
        (r.sumskill3, r.sumsk3calc),
        (r.sumskill4, r.sumsk4calc),
        (r.sumskill5, r.sumsk5calc),
    ];
    let n = t.skills.len() as i32;
    for (k, c) in sum {
        let k = s16(k);
        if k < 1 || k >= n {
            continue;
        }
        let v = eval(w, t, owner, c, skill, lvl);
        if v <= 0 {
            continue;
        }
        w.effect(BodyEffect::SetSkill {
            m,
            skill: k,
            lvl: v,
        });
        if rec(t, k).is_some_and(|x| x.aura) {
            w.effect(BodyEffect::SelectSkill {
                u: m,
                side: 0,
                skill: k,
                owner: -1,
            });
        }
    }
    reregister(w, t, m, skill, lvl, a);
    let um = s16(r.sumumod);
    if (1..=42).contains(&um) {
        w.effect(BodyEffect::Umod {
            m,
            umod: um,
            arg: 1,
        });
    }
    let ov = s16(r.sumoverlay);
    if ov >= 1 && ov < w.overlay_count() {
        w.combat().overlay(m, ov);
    }
    let mut ilvl = ilvl;
    // The clamps apply only when the given ilvl
    // is 0; a non-zero ilvl is passed unchanged.
    if ilvl == 0 {
        ilvl = lvl.wrapping_mul(3);
        if ilvl < 1 {
            ilvl = 1;
        }
        let ol = w.stat(owner, sid::LEVEL, 0);
        if ilvl >= ol {
            ilvl = ol;
        }
    }
    w.effect(BodyEffect::Equipment {
        owner,
        m,
        skill,
        lvl,
        ilvl,
    });
    1
}

// -----------------------------------------------------------------

/// `prog_missile(unit, skill)`: −1 for an invalid
/// skill.
pub fn prog_missile<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, skill: i32) -> i32 {
    let Some(r) = rec(t, skill) else {
        return -1;
    };
    let (a, b, c) = (s16(r.srvmissilea), s16(r.srvmissileb), s16(r.srvmissilec));
    let (st_, a1) = (s16(r.aurastate), s16(r.aurastat1));
    if r.progressive && state_ok(w, st_) && stat_ok(t, a1) {
        if let Some(l) = w.state_list(u, st_) {
            let n = w.list_get(l, a1);
            if n >= 2 {
                return if n >= 3 { c } else { b };
            }
        }
    }
    a
}

// -----------------------------------------------------------------

/// Ring cosines: trunc(30 cos(2πj/64)).
const RING_C: [i32; 17] = [
    30, 29, 29, 28, 27, 26, 24, 23, 21, 19, 16, 14, 11, 8, 5, 2, 0,
];

fn ring_x(i: usize) -> i32 {
    match i {
        0..=16 => RING_C[i],
        17..=32 => -RING_C[32 - i],
        33..=48 => -RING_C[i - 32],
        _ => RING_C[64 - i],
    }
}

/// Ring offset i (0…63) of the missile ring.
pub fn ring_offset(i: usize) -> (i32, i32) {
    (ring_x(i), ring_x((i + 48) % 64))
}

/// Missile ring.
#[allow(clippy::too_many_arguments)]
pub fn ring<W: BodyWorld>(
    w: &mut W,
    owner: W::Unit,
    at: W::Unit,
    m: i32,
    skill: i32,
    lvl: i32,
    v: i32,
) {
    let (x, y) = w.position(at);
    let mut req = MissileRequest {
        flags: 3,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(owner, m)
    };
    if v != 0 {
        req.flags |= 4;
        req.velocity = v;
    }
    for i in 0..64 {
        let (ox, oy) = ring_offset(i);
        req.target_x = ox;
        req.target_y = oy;
        w.spawn_missile(req);
    }
}

// -----------------------------------------------------------------

/// Shout state.
pub fn shout_state<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    tg: W::Unit,
    src: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (a, len) = (s16(r.aurastate), r.auralencalc);
    if !state_ok(w, a) {
        return 0;
    }
    let len = eval(w, t, src, len, skill, lvl);
    let e = super::after(w, len);
    let l = match w.state_list(tg, a) {
        Some(l) => l,
        None => {
            let Some(l) = w.alloc_list(2, e, Some(src)) else {
                return 0;
            };
            w.set_list_state(l, a);
            w.set_remove_callback(l, callback::DEFAULT);
            w.attach(tg, l);
            w.state_on(tg, a, true);
            l
        }
    };
    w.mark_state_changed(tg, a);
    w.set_list_expire(l, e);
    w.combat().schedule_timer(tg, 12, e);
    aura_fill(w, t, src, l, skill, lvl);
    w.buff_refresh(tg);
    1
}

// -----------------------------------------------------------------

/// Sentry spawn.
#[allow(clippy::too_many_arguments)]
pub fn sentry<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    (x, y): (i32, i32),
    skill: i32,
    lvl: i32,
) -> Option<W::Unit> {
    let (c, mode) = summon_class(w, t, ct, u, skill);
    if c < 0 {
        return None;
    }
    let r = rec(t, skill)?;
    let (pet, petmax, intown) = (i32::from(r.pettype as i8), r.petmax, r.intown);
    let pt = if (0..w.pettype_count()).contains(&pet) {
        pet
    } else {
        0
    };
    let pm = eval(w, t, u, petmax, skill, lvl);
    let (mut x, mut y) = (x, y);
    if x == 0 || y == 0 {
        (x, y) = w.position(u);
    }
    let mut owner = u;
    while w.unit_type(owner) == UnitType::Monster {
        owner = w.minion_owner(owner)?;
    }
    if x == 0 || y == 0 {
        (x, y) = tpos(w, owner)?;
    }
    let room = w.unit_room(owner).and_then(|r| w.room_at(r, x, y))?;
    if !intown && w.room_in_town(room) {
        return None;
    }
    let m = spawn(
        w,
        Summon {
            flags: 1,
            owner,
            class: c,
            ai: 0,
            mode,
            x,
            y,
            pet_type: pt,
            pet_max: pm,
        },
    )?;
    base_stats(w, owner, m, 0, lvl);
    skill_stats(w, t, owner, m, skill, lvl, 0);
    w.effect(BodyEffect::Alignment { u: m, a: 2, v: 1 });
    w.mode_request(m, mode, None);
    Some(m)
}

// ------------------------------------------------------------------

/// Charge add.
pub fn charge_add<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    (s, c): (i32, i32),
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let (len, a2, calc2) = (r.auralencalc, s16(r.aurastat2), r.aurastatcalc2);
    let len = eval(w, t, u, len, skill, lvl);
    let e = super::after(w, len);
    let l = match w.state_list(u, s) {
        Some(l) => l,
        None => {
            let Some(l) = w.alloc_list(2, e, Some(u)) else {
                return 0;
            };
            w.set_list_state(l, s);
            w.set_remove_callback(l, callback::CHARGE);
            w.attach(u, l);
            w.list_set(l, stat::MODIFIERLIST_SKILL, skill);
            w.list_set(l, stat::MODIFIERLIST_LEVEL, lvl);
            l
        }
    };
    w.set_list_expire(l, e);
    w.combat().schedule_timer(u, 12, e);
    let n = w.list_get(l, c);
    let n2 = n.wrapping_add(1).min(3);
    if n2 == n {
        return 1;
    }
    w.list_set(l, c, n2);
    if stat_ok(t, a2) {
        let v = eval(w, t, u, calc2, skill, lvl);
        w.list_add(l, a2, v);
    }
    w.state_on(u, s, true);
    w.mark_state_changed(u, s);
    1
}

// ------------------------------------------------------------------

/// Summon resistance.
pub fn summon_resist<W: BodyWorld>(w: &mut W, owner: W::Unit, m: W::Unit) {
    let r = w.stat(owner, sid::PASSIVE_SUMMON_RESIST, 0);
    if r == 0 {
        return;
    }
    let Some(l) = w.alloc_list(0, 0, Some(m)) else {
        return;
    };
    w.attach(m, l);
    if w.stat(m, sid::ABSORB_FIRE_PCT, 0) <= 0 {
        w.list_set(l, 39, r);
    }
    if w.stat(m, sid::ABSORB_LIGHT_PCT, 0) <= 0 {
        w.list_set(l, 41, r);
    }
    if w.stat(m, sid::ABSORB_COLD_PCT, 0) <= 0 {
        w.list_set(l, 43, r);
    }
    w.list_set(l, 45, r);
}

/// Golem stats.
pub fn golem_stats<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    owner: W::Unit,
    m: W::Unit,
    skill: i32,
    lvl: i32,
) {
    base_stats(w, owner, m, 0, lvl);
    skill_stats(w, t, owner, m, skill, lvl, 0);
    summon_resist(w, owner, m);
}

// ------------------------------------------------------------------

/// Free target point.
pub fn point_free<W: BodyWorld>(w: &mut W, t: &SkillTables, u: W::Unit, m: i32) -> bool {
    // No missile record: a fatal assertion; refused.
    let Some(size) = t.missile(m).map(|r| i32::from(r.size)) else {
        return false;
    };
    let Some(room) = w.unit_room(u) else {
        return false;
    };
    let Some(at) = tpos(w, u) else {
        return false;
    };
    !w.box_collides(room, at, size, 5)
}

// ------------------------------------------------------------------

/// Missile at the target point `(game, unit, skill, L, m, tx, ty)`.
#[allow(clippy::too_many_arguments)]
pub fn missile_at<W: BodyWorld>(
    w: &mut W,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    m: i32,
    (tx, ty): (i32, i32),
) -> Option<W::Unit> {
    let (mut tx, mut ty) = (tx, ty);
    if tx == 0 && ty == 0 {
        (tx, ty) = w.target_position(u).unwrap_or((0, 0));
        if tx == 0 && ty == 0 {
            return None;
        }
    }
    if distance(w.position(u), (tx, ty)) as u32 > 100 {
        return None;
    }
    w.spawn_missile(MissileRequest {
        flags: 1,
        x: tx,
        y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    })
}

// ------------------------------------------------------------------

/// Paladin raise penalty.
pub fn raise_penalty<W: BodyWorld>(w: &mut W, ct: &CombatTables, u: W::Unit) {
    if w.unit_type(u) != UnitType::Player || w.class_id(u) != 3 {
        return;
    }
    let h = w.stat_max(u, sid::LIFE) / 8;
    let mut r = DamageRecord {
        hit_flags: 0x1000,
        result: 4,
        physical: h,
        total: h,
        ..DamageRecord::default()
    };
    apply(w.combat(), ct, u, u, false, &mut r);
    w.combat().reaction(u, u, &mut r);
}

// ------------------------------------------------------------------

/// Component table: rows lvl 0…10 of (t0…t8).
pub const COMPONENTS: [[i32; 9]; 11] = [
    [0; 9],
    [0; 9],
    [0, 1, 0, 0, 0, 0, 0, 0, 1],
    [0, 1, 0, 0, 0, 0, 0, 1, 1],
    [1, 1, 0, 0, 0, 0, 1, 1, 2],
    [1, 1, 0, 0, 0, 0, 1, 1, 2],
    [1, 1, 1, 0, 0, 1, 1, 1, 3],
    [2, 2, 1, 0, 0, 1, 1, 1, 3],
    [2, 2, 1, 0, 0, 1, 1, 2, 4],
    [2, 3, 1, 0, 0, 1, 2, 2, 4],
    [2, 3, 1, 0, 0, 1, 2, 2, 4],
];

/// Skeleton components.
pub fn components<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    owner: W::Unit,
    m: W::Unit,
    skill: i32,
    lvl: i32,
) {
    let mage = match w.class_id(m) {
        363 => false,
        364 => true,
        _ => return,
    };
    let mut row = 1;
    if w.has_state(owner, st::SKEL_MASTERY as u16) {
        if let Some(l) = w.state_list(owner, st::SKEL_MASTERY) {
            row = w.list_get(l, stat::MODIFIERLIST_LEVEL);
        }
    }
    if row > 10 {
        row = 10;
    }
    let mut shield = false;
    if !mage && lvl > 2 {
        let p = param(t, skill, 1);
        let r = w.seed(owner).step() % 100;
        shield = (r as i32) < p;
    }
    // No lower clamp; a negative level reads
    // before the table, unreachable with base-game data: a fatal assertion
    // (refused in a release build).
    debug_assert!(
        row >= 0,
        "negative skeleton mastery level {row} (bodies.md §6.15)"
    );
    let Some(c) = usize::try_from(row).ok().and_then(|i| COMPONENTS.get(i)) else {
        return;
    };
    let c = *c;
    for (k, v) in [
        (0, c[0]),
        (1, c[2]),
        (8, c[3]),
        (9, c[4]),
        (2, c[5]),
        (3, c[6]),
        (4, c[7]),
    ] {
        w.effect(BodyEffect::Component { m, k, v });
    }
    if shield {
        w.effect(BodyEffect::Component { m, k: 7, v: c[1] });
    }
    if mage {
        let v = (w.seed(owner).step() & 3) as i32;
        w.effect(BodyEffect::Component { m, k: 11, v });
        w.effect(BodyEffect::Component { m, k: 12, v });
        w.effect(BodyEffect::AiParams {
            m,
            p0: 1,
            p1: 0,
            p2: -666,
        });
    } else {
        w.effect(BodyEffect::Component { m, k: 5, v: c[8] });
    }
}

// ------------------------------------------------------------------

/// Inferno start.
#[allow(clippy::too_many_arguments)]
pub fn inferno_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    m: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let calc2 = r.calc2;
    let f = w.frame();
    if w.unit_type(u) == UnitType::Monster {
        let n = eval(w, t, u, calc2, skill, lvl).max(1);
        let v = f.wrapping_add(super::ticks(w, n));
        if let Some(e) = w.used_skill(u) {
            w.set_entry_param_of(u, &e, 1, v);
        }
    }
    if let Some(l) = w.state_list(u, st::INFERNO) {
        w.anim_restart(u, 1);
        // Each breath keeps the channel 6 base ticks longer.
        let e = super::after(w, 6);
        w.set_list_expire(l, e);
        w.combat().schedule_timer(u, 12, e);
        inferno_do(w, t, ct, u, skill, lvl, m);
        return 1;
    }
    // The channel starts with 20 base ticks.
    let e = super::after(w, 20);
    let Some(l) = w.alloc_list(2, e, Some(u)) else {
        return 0;
    };
    w.combat().schedule_timer(u, 12, e);
    w.attach(u, l);
    w.set_remove_callback(l, callback::INFERNO);
    w.set_list_state(l, st::INFERNO);
    w.state_on(u, st::INFERNO, true);
    if let Some(e) = w.used_skill(u) {
        w.set_entry_param_of(u, &e, 1, 0);
    }
    1
}

// ------------------------------------------------------------------

/// Inferno do.
#[allow(clippy::too_many_arguments)]
pub fn inferno_do<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    skill: i32,
    lvl: i32,
    m: i32,
) -> i32 {
    let Some(r) = rec(t, skill) else {
        return 0;
    };
    let calc1 = r.calc1;
    if !missile_ok(t, m) {
        return 0;
    }
    let Some((tx, ty)) = tpos(w, u) else {
        return 0;
    };
    let Some(e) = w.used_skill(u) else {
        return 0;
    };
    if w.entry_param(u, &e, 1) != 0 {
        let range = eval(w, t, u, calc1, skill, lvl).max(1);
        w.spawn_missile(MissileRequest {
            flags: 0x8020,
            origin: Some(u),
            target_x: tx,
            target_y: ty,
            skill,
            level: lvl,
            range,
            ..MissileRequest::new(u, m)
        });
    }
    w.set_entry_param_of(u, &e, 1, 1);
    if w.unit_type(u) != UnitType::Monster {
        w.anim_restart(u, 1);
        return 1;
    }
    w.set_anim_frame(u, 0xB00);
    let f = w.frame();
    if f < w.entry_param(u, &e, 1) && w.has_state(u, st::INFERNO as u16) {
        w.delete_timers(u, 1, 0);
        w.schedule(u, 0, super::after(w, 2), 4, 0);
    } else {
        w.state_on(u, st::INFERNO, false);
        w.delete_timers(u, 0, 0);
        let d = ct
            .monstats2(w.class_id(u))
            .map_or(1, |m| i32::from(m.infernolen));
        // The wind-down lasts `infernolen` base ticks.
        w.schedule(u, 1, super::after(w, d), 0, 0);
    }
    1
}

// ------------------------------------------------------------------

/// Missile fan at the target `(game, unit, n, m, skill, L, first)`.
#[allow(clippy::too_many_arguments)]
pub fn fan<W: BodyWorld>(
    w: &mut W,
    u: W::Unit,
    n: i32,
    m: i32,
    skill: i32,
    lvl: i32,
    first: bool,
) -> i32 {
    let mut n = n;
    if first {
        skill_missile(w, m, u, skill, lvl, (0, 0), (0, 0), false, false);
        n = n.wrapping_sub(1);
        if n <= 0 {
            return 1;
        }
    }
    let (x, y) = w.position(u);
    let mut req = MissileRequest {
        flags: 0x21,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(u, m)
    };
    let Some((tx, ty)) = tpos(w, u) else {
        return 1;
    };
    req.target_x = tx;
    req.target_y = ty;
    for i in 0..n {
        req.init = Some((init_cb::JITTER, i as u32));
        w.spawn_missile(req);
    }
    1
}

// ------------------------------------------------------------------

/// Shadow stats `(game, R, skill, L)` on m; formulas
/// on the shadow.
pub fn shadow_stats<W: BodyWorld>(w: &mut W, t: &SkillTables, m: W::Unit, skill: i32, lvl: i32) {
    if lvl <= 1 {
        return;
    }
    let p = param(t, skill, 1);
    let h = w.stat_max(m, sid::LIFE);
    let h2 = h.wrapping_add(pct(h, lvl.wrapping_sub(1).wrapping_mul(p), 100));
    w.set_stat(m, sid::MAXHP, h2);
    w.set_stat(m, sid::LIFE, h2);
    let Some(l) = w.alloc_list(0, 0, Some(m)) else {
        return;
    };
    w.attach(m, l);
    let Some(r) = rec(t, skill).cloned() else {
        return;
    };
    // Edge case 18: always the second formula.
    for s in [
        r.aurastat1,
        r.aurastat2,
        r.aurastat3,
        r.aurastat4,
        r.aurastat5,
        r.aurastat6,
    ] {
        let s = s16(s);
        if stat_ok(t, s) {
            let v = eval(w, t, m, r.aurastatcalc2, skill, lvl);
            w.list_set(l, s, v);
        }
    }
    for s in [
        r.passivestat1,
        r.passivestat2,
        r.passivestat3,
        r.passivestat4,
        r.passivestat5,
    ] {
        let s = s16(s);
        if stat_ok(t, s) {
            let v = eval(w, t, m, r.passivecalc2, skill, lvl);
            w.list_set(l, s, v);
        }
    }
    let um = s16(r.sumumod);
    if (1..=42).contains(&um) {
        w.effect(BodyEffect::Umod {
            m,
            umod: um,
            arg: 1,
        });
    }
}

// ------------------------------------------------------------------

/// Source-unit link.
pub fn link_source<W: BodyWorld>(w: &mut W, m: W::Unit, owner: Option<W::Unit>) {
    w.effect(BodyEffect::SourceFields { m, owner });
    let holder = w.has_stat_holder(m);
    match owner {
        Some(o) => {
            if holder {
                w.state_on(m, st::SOURCEUNIT, true);
                let l = match w.state_list(m, st::SOURCEUNIT) {
                    Some(l) => l,
                    None => {
                        let Some(l) = w.alloc_list(0, 0, Some(m)) else {
                            return;
                        };
                        w.set_list_state(l, st::SOURCEUNIT);
                        w.attach(m, l);
                        l
                    }
                };
                let (ty, g) = (type_index(w, o), guid(w, o));
                w.list_set(l, sid::SOURCE_UNIT_TYPE, ty);
                w.list_set(l, sid::SOURCE_UNIT_ID, g as i32);
            }
            let c = w.unit_c8(m);
            w.set_unit_c8(m, c | 0x400);
        }
        None => {
            if holder {
                w.state_on(m, st::SOURCEUNIT, false);
                if let Some(l) = w.state_list(m, st::SOURCEUNIT) {
                    w.detach_free(m, l);
                }
            }
            let c = w.unit_c8(m);
            w.set_unit_c8(m, c & !0x400);
        }
    }
}

/// `melee_result` with `to_hit(unit, skill, L)`, as most melee bodies.
#[allow(clippy::too_many_arguments)]
pub fn skill_melee<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    tg: W::Unit,
    skill: i32,
    lvl: i32,
) -> DamageRecord {
    let h = skill_to_hit(w, t, u, skill, lvl);
    melee_rec(w, t, ct, u, tg, h, 0)
}
