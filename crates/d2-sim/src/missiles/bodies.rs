//! The server-do bodies 2, 3, 5, 7, 8, 10, 25 and the server-hit bodies
//! 1, 4, 12, 13, with
//! their helpers (`sub_at_step`, the distance,
//! `elem_roll`, `elem_len`, `area_damage`
//! and its per-unit hit, `next_unit`). What other systems own goes through
//! [`super::seams::MissileBodies`].

use crate::combat::{
    apply, block_or_dodge, monster_crit, BlockResult, CombatTables, CombatWorld, DamageRecord,
};
use crate::game::Game;
use crate::units::UnitId;

use super::catalogue::create_with_collision_check;
use super::create::{create_missile, MissileParams};
use super::flight::default_flight;
use super::hit::{damage_roll, hit_handler};
use super::seams::SkillCalc;
use super::{param_flags, stat, Ctx, MissileWorld};

/// i32 view of a 16-bit row field.
pub(super) fn i16v(v: u16) -> i32 {
    i32::from(v as i16)
}

/// The missile's position (path x / y).
pub(super) fn pos<W: MissileWorld + ?Sized>(cx: &Ctx<'_, W>, m: UnitId) -> (i32, i32) {
    cx.world.position(m)
}

/// (skill, level) of the missile (data +0x0A / +0x0C).
pub(super) fn skill_level<W: MissileWorld + ?Sized>(cx: &Ctx<'_, W>, m: UnitId) -> (i32, i32) {
    cx.store
        .get(m)
        .map_or((0, 0), |d| (i32::from(d.skill), i32::from(d.level)))
}

// ------------------------------------------------------------------

/// `sub_at_step(game, missile, class, range, loops)`.
pub fn sub_at_step<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    class: i32,
    range: i32,
    loops: i32,
) -> Option<UnitId> {
    if !cx.world.path_new_step(m) {
        return None;
    }
    let owner = cx.owner(game, m)?;
    let (x, y) = pos(cx, m);
    let (skill, level) = skill_level(cx, m);
    let mut p = MissileParams {
        flags: param_flags::POSITION | param_flags::VELOCITY,
        owner: Some(owner),
        origin: Some(m),
        class,
        x,
        y,
        skill,
        level,
        // Whatever `loops` was: the argument only switches flag 8.
        loops: level.wrapping_mul(2).wrapping_sub(2),
        ..MissileParams::default()
    };
    if range > 0 {
        p.flags |= param_flags::RANGE;
        p.range = range;
    }
    if loops > 0 {
        p.flags |= param_flags::LOOPS;
    }
    create_missile(game, cx, &p)
}

/// Server-do 2 (Poison Javelin, poison traps).
pub fn srv_do_2<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    if let Some(row) = cx.row_of(m).cloned() {
        let sub = i16v(row.submissile1);
        if sub >= 0 {
            let owner = cx.owner(game, m);
            let class = cx.store.get(m).map_or(0, |d| i32::from(d.class));
            let (_, level) = skill_level(cx, m);
            let v = cx
                .world
                .missile_calc(game, m, owner, row.srvcalc1, class, level);
            sub_at_step(game, cx, m, sub, 0, v);
        }
    }
    default_flight(game, cx, m)
}

/// Server-do 3 (poison cloud, Blizzard, Thunder Storm, Hand of God).
pub fn srv_do_3<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    if cx.world.has_path(m) && cx.world.velocity(m) != 0 {
        return default_flight(game, cx, m);
    }
    if let Some(room) = game.lists.unit(m).and_then(|e| e.room()) {
        let (x, y) = pos(cx, m);
        cx.world
            .or_collision(game, room, x, y, super::coll::FOOTPRINT);
    }
    default_flight(game, cx, m)
}

/// Server-do 5 (Fire Wall, Immolation fire, Meteor fire, Molten Boulder
/// path).
pub fn srv_do_5<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return default_flight(game, cx, m);
    };
    let f = cx.world.anim_frame(m) >> 8;
    cx.world.stamp_collision(game, m, super::coll::FOOTPRINT);
    let (s, e) = (i32::from(row.substart), i32::from(row.substop));
    // Frames left in base ticks: the animation shows one frame per base
    // tick, so this compares with frame numbers.
    let left = cx.store.get(m).map_or(0, |d| i32::from(d.current));
    let left = game.tick_rate.base_ticks(left);
    if f == s - 1 {
        let r = cx.world.seed(m).roll(e - s) as i32;
        cx.world.set_anim_frame(m, (s - 1 + r) << 8);
    } else if left == s {
        cx.world.set_anim_frame(m, (s - 3) << 8);
    } else if left < s {
        cx.world.set_anim_frame(m, (f - 2).max(0) << 8);
    }
    default_flight(game, cx, m)
}

/// The size-reduced distance.
pub fn unit_distance<W: MissileWorld + ?Sized>(w: &W, a: UnitId, b: UnitId) -> i32 {
    let ((ax, ay), (bx, by)) = (w.position(a), w.position(b));
    let gap = w.size(a) / 2 + w.size(b) / 2;
    let dx = (bx - ax).abs().wrapping_sub(gap).max(0);
    let dy = (by - ay).abs().wrapping_sub(gap).max(0);
    (2 * dx.max(dy) + dx.min(dy)) / 2
}

/// Server-do 7 (Guided Arrow, Bone Spirit).
pub fn srv_do_7<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let Some(owner) = cx.owner(game, m).filter(|&o| !cx.world.is_dead(o)) else {
        return 2;
    };
    if game
        .lists
        .unit(m)
        .and_then(|e| e.room())
        .is_some_and(|r| cx.world.in_town(game, r))
    {
        return 2;
    }
    let homing = cx.store.get(m).is_some_and(|d| d.target.0 & 1 != 0);
    if homing {
        let t = cx
            .world
            .path_target(game, m)
            .filter(|&t| !cx.world.is_dead(t) && cx.world.may_attack(owner, t));
        let mut n = row.param1 as i32;
        if n <= 0 {
            n = 5;
        }
        let left = cx.store.get(m).map_or(0, |d| i32::from(d.current));
        // The missile re-aims every `n` base ticks of its flight.
        let n = game.ticks(n).max(1);
        if let Some(t) = t {
            if left % n == 0 {
                let d = unit_distance(&*cx.world, m, t);
                if (4..=24).contains(&d) {
                    cx.world.build(game, m);
                }
            }
        }
    }
    default_flight(game, cx, m)
}

/// Server-do 8 (MonBlizzCenter).
pub fn srv_do_8<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        // A null record is read (class ids are always
        // valid): D2RUST keeps the missile.
        return 1;
    };
    let (_, level) = skill_level(cx, m);
    let q = level / (row.param3 as i32).max(1);
    let range = (row.param1 as i32).wrapping_add(q.max(2));
    let interval = (row.param2 as i32).wrapping_sub(q).max(3);
    create_with_collision_check(game, cx, m, range, interval, i16v(row.submissile1), 5);
    default_flight(game, cx, m)
}

/// Server-do 10 (BlizzardCenter) and 25 (EruptionCenter):
/// The helper with range = skill `calc1`, interval =
/// `calc2`.
fn skill_center<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    sub: i32,
    mask: u16,
) -> i32 {
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 2;
    }
    let owner = cx.owner(game, m);
    let range = cx.world.skill_calc(game, owner, k, SkillCalc::Calc1, level);
    let interval = cx.world.skill_calc(game, owner, k, SkillCalc::Calc2, level);
    create_with_collision_check(game, cx, m, range, interval, sub, mask);
    default_flight(game, cx, m)
}

/// Server-do 10 (BlizzardCenter).
pub fn srv_do_10<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        // As server-do 8.
        return 1;
    };
    skill_center(game, cx, m, i16v(row.submissile1), 5)
}

/// Server-do 25 (EruptionCenter).
pub fn srv_do_25<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let sub = i16v(row.submissile1);
    if sub == 0 {
        return 2;
    }
    skill_center(game, cx, m, sub, 0x45)
}

// ------------------------------------------------------------------

/// `elem_roll(game, missile, unit, record)`: rolls only
/// the row's `EType` element on the missile's seed; returns the rolled
/// amount (8.8): the final physical, the fire / lightning / magic / cold
/// roll, poison or burn × 25, else 0 (the server-hit return value).
pub fn elem_roll<W: MissileWorld + ?Sized>(
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
    rec: &mut DamageRecord,
) -> i32 {
    let etype = cx.row_of(m).map_or(0, |r| i32::from(r.etype));
    let roll = |cx: &mut Ctx<'_, W>, min_s: u16, max_s: u16, mastery_s: u16| {
        let max = cx.world.stat(m, max_s);
        let min = cx.world.stat(m, min_s);
        let mastery = if mastery_s == 0 {
            0
        } else {
            cx.world.stat(m, mastery_s)
        };
        damage_roll(cx.world.seed(m), min, max, mastery)
    };
    match etype {
        0 => {
            let mut ph = roll(cx, stat::MINDAMAGE, stat::MAXDAMAGE, 0);
            if let Some(u) = unit {
                let w = &*cx.world;
                let mut p = w.stat(m, stat::DAMAGEPERCENT);
                if w.is_demon(u) {
                    p = p.wrapping_add(w.stat(m, 121));
                }
                if w.is_undead(u) {
                    p = p.wrapping_add(w.stat(m, 122));
                }
                let p = p.max(-90);
                ph = ph.wrapping_add(ph.wrapping_mul(p) / 100);
                if w.stat(m, stat::DEADLY_STRIKE) != 0 {
                    ph = ph.wrapping_mul(2);
                    rec.result |= 0x2000;
                }
            }
            rec.physical = ph;
            ph
        }
        1 => {
            rec.fire = roll(cx, stat::FIREMINDAM, stat::FIREMAXDAM, stat::FIRE_MASTERY);
            rec.fire
        }
        2 => {
            rec.lightning = roll(
                cx,
                stat::LIGHTMINDAM,
                stat::LIGHTMAXDAM,
                stat::LIGHT_MASTERY,
            );
            rec.lightning
        }
        3 => {
            rec.magic = roll(
                cx,
                stat::MAGICMINDAM,
                stat::MAGICMAXDAM,
                stat::MAGIC_MASTERY,
            );
            rec.magic
        }
        4 => {
            rec.cold = roll(cx, stat::COLDMINDAM, stat::COLDMAXDAM, stat::COLD_MASTERY);
            rec.cold_len = cx.world.stat(m, stat::COLDLENGTH);
            rec.cold
        }
        5 => {
            rec.poison = roll(
                cx,
                stat::POISONMINDAM,
                stat::POISONMAXDAM,
                stat::POISON_MASTERY,
            );
            let mut len = cx.world.stat(m, stat::POISONLENGTH);
            let count = cx.world.stat(m, stat::POISON_COUNT);
            if count > 1 {
                len /= count;
            }
            rec.poison_len = len;
            rec.poison.wrapping_mul(25)
        }
        6 => {
            rec.life_leech = cx.world.stat(m, stat::LIFEDRAINMINDAM);
            0
        }
        7 => {
            rec.mana_leech = cx.world.stat(m, stat::MANADRAINMINDAM);
            0
        }
        8 => {
            rec.stamina_leech = cx.world.stat(m, stat::STAMDRAINMINDAM);
            0
        }
        9 => {
            rec.stun_len = cx.world.stat(m, stat::STUNLENGTH);
            0
        }
        11 => {
            rec.burn = roll(
                cx,
                stat::BURNINGMINDAM,
                stat::BURNINGMAXDAM,
                stat::FIRE_MASTERY,
            );
            rec.burn_len = cx.world.stat(m, stat::BURNINGLENGTH);
            rec.burn.wrapping_mul(25)
        }
        12 => {
            rec.cold = roll(cx, stat::COLDMINDAM, stat::COLDMAXDAM, stat::COLD_MASTERY);
            rec.freeze_len = cx.world.stat(m, stat::COLDLENGTH);
            rec.cold
        }
        _ => 0,
    }
}

/// `elem_len(record, len)` by `EType`.
pub fn elem_len(rec: &mut DamageRecord, etype: i32, len: i32) {
    match etype {
        4 => rec.cold_len = len,
        5 => rec.poison_len = len,
        9 => rec.stun_len = len,
        11 => rec.burn_len = len,
        12 => rec.freeze_len = len,
        _ => {}
    }
}

/// `area_damage(game, owner, x, y, r, record, f)`: every
/// unit the scan accepts gets [`area_hit`] on a copy. Returns 1.
pub fn area_damage<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    owner: Option<UnitId>,
    at: (i32, i32),
    r: i32,
    rec: &DamageRecord,
    f: u32,
) -> i32 {
    let f = if f == 0 { 0x8583 } else { f };
    let Some(owner) = owner else {
        return 1;
    };
    for u in cx.world.area_units(game, owner, at, r, f) {
        cx.world.area_hit(game, owner, u, rec);
    }
    1
}

/// The per-unit area hit:
/// block or dodge (avoid 1, block 0), get-hit, the monster critical hit,
/// the damage application and the reaction.
/// The attacker is the missile's owner (area damage: a missile
/// attacker becomes its owner before the walk).
pub fn area_hit<W: CombatWorld>(
    w: &mut W,
    ct: &CombatTables,
    a: W::Unit,
    u: W::Unit,
    rec: &DamageRecord,
) {
    let mut c = *rec;
    if c.result == 0 {
        return;
    }
    let b = block_or_dodge(w, ct, a, u, true, false);
    c.result |= match b {
        BlockResult::Avoid => 0x100,
        BlockResult::Dodge => 0x80,
        BlockResult::WeaponBlock => 0x8000,
        BlockResult::Block => 0x10,
        // Evade sets no result bit; only the hit bit is cleared.
        BlockResult::Evade | BlockResult::None => 0,
    };
    if b != BlockResult::None {
        c.result &= !1;
    }
    if c.result & 1 != 0 && !w.has_state(u, 54) {
        c.result |= 4;
    }
    monster_crit(w, ct, a, u, &mut c);
    if c.result & 1 != 0 {
        apply(w, ct, a, u, true, &mut c);
    }
    w.reaction(a, u, &mut c);
}

/// `next_unit(game, source, x, y, r, f, g)`: the accepted
/// unit with the smallest GUID > g, else the one with the smallest GUID
/// ≤ g (a later equal one replaces it).
pub fn next_unit<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    source: UnitId,
    at: (i32, i32),
    r: i32,
    f: u32,
    g: u32,
) -> Option<UnitId> {
    let mut above: Option<(u32, UnitId)> = None;
    let mut below: Option<(u32, UnitId)> = None;
    for u in cx.world.area_units(game, source, at, r, f | 0xA783) {
        let Some(guid) = game.lists.unit(u).map(|e| e.guid) else {
            continue;
        };
        if guid > g {
            if above.is_none_or(|(b, _)| guid < b) {
                above = Some((guid, u));
            }
        } else if below.is_none_or(|(b, _)| guid <= b) {
            below = Some((guid, u));
        }
    }
    above.or(below).map(|(_, u)| u)
}

/// The radius / length of server-hit 1, 12, 13: the row value when > 0,
/// else `max(eval(skill calc), 1)` (`None`: no skill record).
pub(super) fn row_or_skill<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    v: i32,
    calc: SkillCalc,
) -> Option<i32> {
    if v > 0 {
        return Some(v);
    }
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return None;
    }
    let owner = cx.owner(game, m);
    Some(cx.world.skill_calc(game, owner, k, calc, level).max(1))
}

/// Server-hit 1 (Fireball, Exploding / Freezing Arrow explosion).
pub fn srv_hit_1<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let Some(owner) = cx.owner(game, m) else {
        return 1;
    };
    let Some(r) = row_or_skill(game, cx, m, row.shitpar1 as i32, SkillCalc::Calc1) else {
        return 1;
    };
    let mut rec = DamageRecord::default();
    elem_roll(cx, m, unit, &mut rec);
    rec.hit_flags |= row.hitflags;
    rec.result |= row.resultflags;
    let at = pos(cx, m);
    area_damage(game, cx, Some(owner), at, r, &rec, 0);
    1
}

/// Server-hit 4 (Exploding Arrow, Freezing Arrow, Royal Strike meteor
/// center).
pub fn srv_hit_4<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let (skill, level) = skill_level(cx, m);
    let owner = cx.owner(game, m);
    for sub in [
        row.hitsubmissile1,
        row.hitsubmissile2,
        row.hitsubmissile3,
        row.hitsubmissile4,
    ] {
        let class = i16v(sub);
        if class <= 0 {
            continue;
        }
        // Flags 0: start at the origin, target = start.
        let p = MissileParams {
            owner,
            origin: Some(m),
            class,
            skill,
            level,
            ..MissileParams::default()
        };
        if let Some(n) = create_missile(game, cx, &p) {
            if row.shitpar1 as i32 > 0 {
                hit_handler(game, cx, n, unit, true);
            }
        }
    }
    3
}

/// Server-hit 12 (Chain Lightning, Lightning Strike).
pub fn srv_hit_12<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let (Some(owner), Some(hit)) = (cx.owner(game, m), unit) else {
        return 3;
    };
    let n = cx.store.get(m).map_or(0, |d| d.target.0);
    if n <= 1 {
        return 3;
    }
    let Some(r) = row_or_skill(game, cx, m, row.shitpar1 as i32, SkillCalc::AuraRange) else {
        return 3;
    };
    let g = game.lists.unit(hit).map_or(0, |e| e.guid);
    let at = pos(cx, m);
    let Some(next) = next_unit(game, cx, owner, at, r, 0x88583, g) else {
        return 3;
    };
    if next == hit {
        return 3;
    }
    let (skill, level) = skill_level(cx, m);
    let (tx, ty) = cx.world.position(next);
    let p = MissileParams {
        flags: param_flags::POSITION | param_flags::TARGET_ABSOLUTE,
        owner: Some(owner),
        target: Some(next),
        class: cx.store.get(m).map_or(0, |d| i32::from(d.class)),
        x: at.0,
        y: at.1,
        target_x: tx,
        target_y: ty,
        skill,
        level,
        ..MissileParams::default()
    };
    if let Some(c) = create_missile(game, cx, &p) {
        if let Some(d) = cx.store.get_mut(c) {
            d.target.0 = n - 1;
        }
    }
    3
}

/// Server-hit 13 (Glacial Spike, Hell Meteor down).
pub fn srv_hit_13<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 1;
    }
    let owner = cx.owner(game, m);
    let r = match row.shitpar1 as i32 {
        v if v > 0 => v,
        _ => cx
            .world
            .skill_calc(game, owner, k, SkillCalc::AuraRange, level)
            .max(1),
    };
    // Body 4: len = sHitPar2 when > 0, else the `auralencalc`
    // formula.
    let len = match row.shitpar2 as i32 {
        v if v > 0 => v,
        _ => cx
            .world
            .skill_calc(game, owner, k, SkillCalc::AuraLen, level),
    };
    let mut rec = DamageRecord::default();
    elem_roll(cx, m, unit, &mut rec);
    let etype = i32::from(row.etype);
    if len > 0 {
        elem_len(&mut rec, etype, len);
    }
    rec.hit_flags |= row.hitflags;
    rec.result |= row.resultflags;
    let at = pos(cx, m);
    area_damage(game, cx, owner, at, r, &rec, 0);
    1
}
