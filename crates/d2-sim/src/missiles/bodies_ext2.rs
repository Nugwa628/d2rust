//! The server-do bodies 12, 13, 15, 16, 18, 19, 20, 21, 27, 29, 30, 32,
//! 33, 36, 37 and the server-hit bodies 5, 6, 11, 20, 21, 22, 23, 24,
//! 25, 28, 29, 31, 32, 33, 35, 37, 39, 40, 43, 47, 48, 50, 51, 53, 54,
//! 55, 57, 59, with `ring8`, the Frozen Orb
//! circle, `corpse_effect`, `spawn_for_level`,
//! `scatter_at_target`, the owner follow and
//! Tyrael's spawn helper. What other systems own goes through
//! [`super::seams::MissileBodies`].

use crate::game::Game;
use crate::rng::Seed;
use crate::units::{UnitId, UnitType};

use super::bodies::{area_damage, elem_roll, i16v, pos, skill_level};
use super::bodies_ext::{
    calc_max, data, elapsed, fatal, full_record, heal, left, missile_at, open_portal, par, ring,
    room_of, row_flags, scatter, set_d28, set_d2c, skill_lin, sub_params, type_of,
};
use super::catalogue::run_srv_do;
use super::create::{create_missile, MissileParams};
use super::flight::default_flight;
use super::hit::{damage_tail, fill_damage, hit_handler, pct};
use super::seams::{SkillCalc, SkillField};
use super::{clamp_frame, param_flags as pf, Ctx, MissileWorld};

/// The unit a (type, GUID) pair in the target fields names.
fn unit_by(game: &Game, ty: i32, guid: i32) -> Option<UnitId> {
    let ty = *UnitType::ALL.get(usize::try_from(ty).ok()?)?;
    game.lists.find_unit(ty, guid as u32)
}

/// The scan context of `fury_cb` / `fist_cb`: one
/// `HitSubMissile1` aimed at each unit until `n` were counted (`limit`
/// false: the count ≥ n test applies only when n > 0).
#[allow(clippy::too_many_arguments)]
fn bolts_at<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    owner: UnitId,
    units: Vec<UnitId>,
    n: i32,
    class: i32,
    always_limit: bool,
) {
    let mut count = 0;
    for u in units {
        if (always_limit || n > 0) && count >= n {
            continue;
        }
        let (tx, ty) = cx.world.position(u);
        let p = MissileParams {
            flags: pf::TARGET_ABSOLUTE,
            origin: Some(m),
            target_x: tx,
            target_y: ty,
            ..sub_params(cx, m, Some(owner), class)
        };
        create_missile(game, cx, &p);
        count += 1;
    }
}

// --------------------------------------------------------------------

/// Server-do 12 Diablo wall maker.
pub fn srv_do_12<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let sub = i16v(row.submissile1);
    if sub < 0 {
        return 2;
    }
    if cx.owner(game, m).is_none() {
        return 2;
    }
    if cx.world.path_new_step(m) {
        let owner = cx.owner(game, m);
        let (x, y) = pos(cx, m);
        let p = MissileParams {
            flags: pf::POSITION | pf::TARGET_ABSOLUTE,
            x,
            y,
            target_x: x,
            target_y: y,
            ..sub_params(cx, m, owner, sub)
        };
        create_missile(game, cx, &p);
    }
    default_flight(game, cx, m)
}

/// Server-hit 20 Lightning Fury.
pub fn srv_hit_20<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let at = pos(cx, m);
    let mut r = par(row.shitpar1);
    if r <= 0 {
        r = calc_max(game, cx, Some(o), k, SkillCalc::AuraRange, level, 1);
    }
    let mut n = par(row.shitpar2);
    if n <= 0 {
        n = calc_max(game, cx, Some(o), k, SkillCalc::Calc1, level, 1);
    }
    let mut f = cx.world.skill_field(k, SkillField::AuraFilter) as u32;
    if f == 0 {
        f = 0xA783;
    }
    let units = cx.world.scan_units(game, o, at, r, f, true);
    bolts_at(game, cx, m, o, units, n, h, true);
    3
}

/// Server-do 13 Bone Wall maker.
pub fn srv_do_13<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    if cx.owner(game, m).is_none() {
        return 2;
    }
    let (anchor, pieces) = data(cx, m);
    if pieces == 0 {
        return 2;
    }
    if !cx.world.path_new_step(m) {
        return default_flight(game, cx, m);
    }
    let Some(o) = cx.owner(game, m) else {
        return 2;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 2;
    }
    let sc = cx.world.summon_class(game, o, k, level);
    if sc.class < 0 {
        return 0;
    }
    let mut pt = cx.world.skill_field(k, SkillField::PetType);
    if pt >= cx.world.pet_type_count() {
        pt = 0;
    }
    let Some(anchor) = game.lists.find_unit(UnitType::Monster, anchor as u32) else {
        return default_flight(game, cx, m);
    };
    let at = pos(cx, m);
    let Some(piece) = cx.world.summon_spawn(game, o, sc.class, sc.mode, at, pt) else {
        return default_flight(game, cx, m);
    };
    cx.world
        .bind_bone_wall_piece(game, o, anchor, piece, k, level);
    set_d2c(cx, m, pieces.wrapping_sub(1));
    default_flight(game, cx, m)
}

/// Server-hit 21 Battle Cry.
pub fn srv_hit_21<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let (Some(o), Some(u)) = (cx.owner(game, m), unit) else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 1;
    }
    let s = cx.world.skill_field(k, SkillField::AuraTargetState);
    // `>`, so s = count passes.
    if s < 0 || s > cx.world.states_count() {
        return 1;
    }
    let f = if cx.world.skill_field(k, SkillField::AuraFilter) != 0 {
        0xA783
    } else {
        0
    };
    if !cx.world.accepts(game, o, u, f) {
        return 1;
    }
    let dur = cx
        .world
        .skill_calc(game, Some(o), k, SkillCalc::AuraLen, level);
    if cx.world.apply_state(game, o, u, k, level, dur, s) {
        cx.world.aura_fill(game, u, s, k, level);
    }
    0
}

/// Server-hit 22 Fist of the Heavens delay.
pub fn srv_hit_22<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 1;
    }
    let (ty, guid) = data(cx, m);
    let Some(t) = unit_by(game, ty, guid) else {
        return 0;
    };
    let at = pos(cx, m);
    let mut r = par(row.shitpar1);
    if r <= 0 {
        r = calc_max(game, cx, Some(o), k, SkillCalc::AuraRange, level, 1);
    }
    let mut n = par(row.shitpar2);
    if n <= 0 {
        n = calc_max(game, cx, Some(o), k, SkillCalc::Calc4, level, 1);
    }
    let mut dmg = fill_damage(cx, m, Some(t));
    damage_tail(game, cx, m, t, &mut dmg);
    let mut f = cx.world.skill_field(k, SkillField::AuraFilter) as u32;
    if f == 0 {
        f = 0xA683;
    }
    let units = cx.world.scan_units(game, o, at, r, f, false);
    bolts_at(game, cx, m, o, units, n, h, false);
    1
}

/// Server-hit 24 panther pot orange.
pub fn srv_hit_24<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let mut r = par(row.shitpar1);
    if r <= 0 {
        let (k, level) = skill_level(cx, m);
        if !cx.world.skill_exists(k) {
            return 1;
        }
        r = calc_max(game, cx, Some(o), k, SkillCalc::Calc1, level, 1);
    }
    let at = pos(cx, m);
    let mut rec = full_record(cx, m, unit);
    row_flags(&mut rec, &row);
    area_damage(game, cx, Some(o), at, r, &rec, 0);
    1
}

/// Panther green GX.
pub const RING8_X: [i32; 8] = [0, 2, 2, 2, 0, -2, -2, -2];
/// Panther green GY.
pub const RING8_Y: [i32; 8] = [2, 2, 0, -2, -2, -2, 0, 2];

/// `ring8(game, owner, origin, class, skill, level, s)`.
#[allow(clippy::too_many_arguments)]
pub fn ring8<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    owner: UnitId,
    origin: UnitId,
    class: i32,
    skill: i32,
    level: i32,
    s: i32,
) -> i32 {
    let Some(crow) = cx.row(class).cloned() else {
        return 0;
    };
    let (x, y) = cx.world.position(origin);
    // Flags 0x1F as stated (the same open 0x10 question as the
    // other bodies).
    let mut p = MissileParams {
        flags: pf::POSITION | pf::TARGET_RELATIVE | pf::VELOCITY | pf::LOOPS | 0x10,
        owner: Some(owner),
        origin: Some(origin),
        class,
        x,
        y,
        skill,
        level,
        loops: level.wrapping_sub(1),
        velocity: par(crow.param1).wrapping_shl(7),
        ..MissileParams::default()
    };
    let s = s.max(1);
    let mut i = 0;
    while i < 8 {
        p.target_x = RING8_X[i as usize];
        p.target_y = RING8_Y[i as usize];
        create_missile(game, cx, &p);
        i += s;
    }
    1
}

/// Server-hit 25 panther pot green.
pub fn srv_hit_25<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let g = unit.unwrap_or(m);
    let (k, level) = skill_level(cx, m);
    ring8(game, cx, o, g, h, k, level, par(row.shitpar1).max(1));
    3
}

/// Server-hit 28 Grim Ward scare.
pub fn srv_hit_28<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 1;
    }
    if cx.owner(game, m).is_none() {
        return 1;
    }
    let Some(u) = unit.filter(|&u| type_of(game, u) == Some(UnitType::Monster)) else {
        return 1;
    };
    let Some(w) = game
        .lists
        .find_unit(UnitType::Missile, data(cx, m).0 as u32)
    else {
        return 1;
    };
    let d = skill_lin(cx, k, level, 1, 2);
    let ((wx, wy), (ux, uy)) = (cx.world.position(w), cx.world.position(u));
    let (dx, dy) = (ux.wrapping_sub(wx), uy.wrapping_sub(wy));
    let sq = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    if sq >= d.wrapping_mul(d) {
        return 1;
    }
    let a = cx.world.skill_field(k, SkillField::Param(5));
    let b = cx.world.skill_field(k, SkillField::Param(6));
    cx.world.terror(game, w, u, k, a, b);
    1
}

// --------------------------------------------------------------------

/// Frozen Orb circle C64 = trunc(30 cos(2πi/64)).
pub const ORB_C: [i32; 64] = [
    30, 29, 29, 28, 27, 26, 24, 23, 21, 19, 16, 14, 11, 8, 5, 2, 0, -2, -5, -8, -11, -14, -16, -19,
    -21, -23, -24, -26, -27, -28, -29, -29, -30, -29, -29, -28, -27, -26, -24, -23, -21, -19, -16,
    -14, -11, -8, -5, -2, 0, 2, 5, 8, 11, 14, 16, 19, 21, 23, 24, 26, 27, 28, 29, 29,
];
/// S64[i] = C64[(i − 16) mod 64].
pub const ORB_S: [i32; 64] = [
    0, 2, 5, 8, 11, 14, 16, 19, 21, 23, 24, 26, 27, 28, 29, 29, 30, 29, 29, 28, 27, 26, 24, 23, 21,
    19, 16, 14, 11, 8, 5, 2, 0, -2, -5, -8, -11, -14, -16, -19, -21, -23, -24, -26, -27, -28, -29,
    -29, -30, -29, -29, -28, -27, -26, -24, -23, -21, -19, -16, -14, -11, -8, -5, -2,
];

/// Server-do 15 Frozen Orb.
pub fn srv_do_15<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let sub = i16v(row.submissile1);
    if sub < 0 {
        return 2;
    }
    let Some(o) = cx.owner(game, m) else {
        return 2;
    };
    // A shard every `param1` base ticks.
    let n = game.ticks(par(row.param1)).max(1);
    if elapsed(cx, m) % n > 0 {
        return default_flight(game, cx, m);
    }
    let i = (data(cx, m).0 % 64).abs();
    let mut p = MissileParams {
        flags: pf::TARGET_RELATIVE,
        origin: Some(m),
        ..sub_params(cx, m, Some(o), sub)
    };
    p.target_x = ORB_C[i as usize];
    p.target_y = ORB_S[i as usize];
    set_d28(cx, m, i.wrapping_add(par(row.param2)) % 64);
    create_missile(game, cx, &p);
    default_flight(game, cx, m)
}

/// The 64-step circle of server-hit 29 step 4 and server-hit 5.
fn orb_circle<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    owner: UnitId,
    class: i32,
    s: i32,
) {
    let mut p = MissileParams {
        flags: pf::TARGET_RELATIVE,
        origin: Some(m),
        ..sub_params(cx, m, Some(owner), class)
    };
    let s = s.max(1);
    let mut i = 0;
    while i < 64 {
        let (c, sn) = (ORB_C[i as usize], ORB_S[i as usize]);
        p.target_x = c;
        p.target_y = sn;
        if let Some(n) = create_missile(game, cx, &p) {
            set_d28(cx, n, c);
            set_d2c(cx, n, sn);
        }
        i += s;
    }
}

/// Server-hit 29 Frozen Orb.
pub fn srv_hit_29<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    if left(cx, m) != 0 {
        return 2;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    orb_circle(game, cx, m, o, h, par(row.shitpar1));
    3
}

/// Server-do 16 Frozen Orb nova.
pub fn srv_do_16<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    // Every `param2` base ticks during the first `param1`.
    let s = game.ticks(par(row.param2)).max(1);
    let e = elapsed(cx, m);
    if e < game.ticks(par(row.param1)) && e % s == 0 {
        let (a, b) = data(cx, m);
        let a2 = a.wrapping_sub(b) / 2;
        let b2 = a.wrapping_add(b) / 2;
        let (x, y) = pos(cx, m);
        cx.world
            .set_target_point(m, x.wrapping_add(a2), y.wrapping_add(b2));
        cx.world.build(game, m);
        set_d28(cx, m, a2);
        set_d2c(cx, m, b2);
    }
    default_flight(game, cx, m)
}

/// Server-hit 31 fire head.
pub fn srv_hit_31<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let (Some(o), Some(_)) = (cx.owner(game, m), unit) else {
        return 1;
    };
    let mut rec = crate::combat::DamageRecord::default();
    // V is the rolled fire amount (the server-hit return value).
    let v = elem_roll(cx, m, unit, &mut rec);
    heal(game, cx, o, v.max(0));
    if row.collidekill != 0 {
        3
    } else {
        2
    }
}

/// Server-hit 32 Cairn Stones.
pub fn srv_hit_32<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 0;
    };
    if unit.is_some() || data(cx, m).0 != 0 {
        return 0;
    }
    open_portal(game, cx, m, par(row.param4));
    0
}

/// Server-do 18 tower chest spawner.
pub fn srv_do_18<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let f = left(cx, m);
    if f == 1 {
        if let Some(c) = game.lists.find_unit(UnitType::Object, data(cx, m).0 as u32) {
            cx.world.unit_sound(game, c, 0x5C);
        }
    }
    // `param1` base ticks after the start; the dropping repeats every
    // `param2 · 4` base ticks.
    let rate = game.tick_rate;
    if f == rate.ticks(i32::from(row.range as i16).wrapping_sub(par(row.param1))) {
        if let Some(c) = game.lists.find_unit(UnitType::Object, data(cx, m).0 as u32) {
            cx.world.chest_drop(game, c);
        }
        set_d2c(cx, m, 1);
    }
    if data(cx, m).1 != 0 && f % rate.ticks(par(row.param2).wrapping_mul(4)).max(1) == 0 {
        let (mut px, mut py) = pos(cx, m);
        let r = par(row.param3);
        let n = r.wrapping_mul(2).wrapping_add(1);
        let seed = cx.world.seed(m);
        px = px.wrapping_add((seed.roll(n) as i32).wrapping_sub(r));
        py = py.wrapping_add((seed.roll(n) as i32).wrapping_sub(r));
        // With no room every cell lookup of the search
        // gives none, so the spot is none: no gold.
        if let Some(room) = room_of(game, m) {
            if let Some((out_room, ox, oy)) = cx.world.floor_drop_spot(game, room, (px, py)) {
                cx.world.create_gold(game, m, out_room, (ox, oy));
            }
        }
    }
    default_flight(game, cx, m)
}

/// Server-hit 33 tower chest spawner.
pub fn srv_hit_33<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    if unit.is_none() {
        if let Some(r) = room_of(game, m) {
            cx.world.refresh_room(game, r);
        }
    }
    0
}

/// `corpse_effect(game, flags, x, y, unit, skill, level, cb)` =
/// `corpse_effect(game, flags, x, y, unit, skill, level, cb)`,
/// with the Redemption callbacks (`last` false and true).
#[allow(clippy::too_many_arguments)]
pub fn corpse_effect<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    flags: u32,
    at: (i32, i32),
    unit: UnitId,
    skill: i32,
    level: i32,
    last: bool,
) {
    let r = skill_lin(cx, skill, level, 1, 2);
    let Some(near) = room_of(game, unit) else {
        return;
    };
    let Some(room) = cx.world.find_room(game, near, at.0, at.1) else {
        return;
    };
    // G := the request's flags.
    let filter = FindFilter {
        flags,
        room_flags: flags,
        source: Some(unit),
        at,
        r,
    };
    for u in unit_find(game, cx, Some(room), &filter) {
        cx.world
            .redemption_effect(game, unit, u, skill, level, last);
    }
}

/// The filter record of the default filter as the unit find
/// builds it: flags F, unit E, centre, radius; limit, accepted count,
/// line iterator and extra test are zero. `room_flags` is the finder's
/// G, which the caller writes after init (0 unless set).
#[derive(Clone, Copy, Debug)]
pub struct FindFilter {
    pub flags: u32,
    pub room_flags: u32,
    pub source: Option<UnitId>,
    pub at: (i32, i32),
    pub r: i32,
}

/// The unit find with the default filter:
/// The units of `room` (or of its adjacency array,
/// unless the square x ± r, y ± r lies strictly inside the room's sub-tile
/// rectangle), in room order then each room's unit-list order. Town
/// rooms (levels 1, 40, 75, 103, 109) are
/// `MissileRooms::in_town`.
pub fn unit_find<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    room: Option<crate::units::RoomId>,
    a: &FindFilter,
) -> Vec<UnitId> {
    let mut accepted = 0;
    unit_find_by(game, cx, room, a.at, a.r, a.room_flags, |game, cx, u| {
        default_filter(game, cx, u, a, &mut accepted)
    })
}

/// The finder with a caller filter: the
/// room selection of [`unit_find`] (centre `at`, radius `r`, finder flags
/// `room_flags`), each unit of the chosen rooms kept when `keep` accepts
/// it. A custom filter does no distance test of its own, so the radius
/// only chooses the rooms (as the storm objects use it).
pub fn unit_find_by<W, F>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    room: Option<crate::units::RoomId>,
    at: (i32, i32),
    r: i32,
    room_flags: u32,
    mut keep: F,
) -> Vec<UnitId>
where
    W: MissileWorld + ?Sized,
    F: FnMut(&mut Game, &mut Ctx<'_, W>, UnitId) -> bool,
{
    let a = FindFilter {
        flags: 0,
        room_flags,
        source: None,
        at,
        r,
    };
    let Some(room) = room else {
        return Vec::new();
    };
    let (x, y, r) = (a.at.0, a.at.1, a.r);
    // Step 2: strictly inside → the room alone.
    let inside = cx
        .world
        .room_subtiles(game, room)
        .is_some_and(|(rx, ry, rw, rh)| {
            x.wrapping_sub(r) > rx
                && y.wrapping_sub(r) > ry
                && x.wrapping_add(r) < rx.wrapping_add(rw)
                && y.wrapping_add(r) < ry.wrapping_add(rh)
        });
    let rooms = if inside {
        vec![room]
    } else {
        game.lists
            .room(room)
            .map(|e| e.adjacent.clone())
            .unwrap_or_default()
    };
    let mut found = Vec::new();
    for rm in rooms {
        // Step 3: town rooms skipped under 0x2000; the overlap test
        // Never rejects for r ≥ 0 and is left to the filter.
        // The skip tests the finder's room flags G, not the filter's F.
        if a.room_flags & 0x2000 != 0 && cx.world.in_town(game, rm) {
            continue;
        }
        if r < 0 {
            if let Some((rx, ry, rw, rh)) = cx.world.room_subtiles(game, rm) {
                let off_x = x.wrapping_add(r) < rx && x.wrapping_sub(r) > rx.wrapping_add(rw);
                let off_y = y.wrapping_add(r) < ry && y.wrapping_sub(r) > ry.wrapping_add(rh);
                if off_x || off_y {
                    continue;
                }
            }
        }
        for u in game.lists.room_units(rm) {
            if keep(game, cx, u) {
                found.push(u);
            }
        }
    }
    found
}

/// The default filter (filter steps 1–5).
/// filter steps 1–5).
fn default_filter<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    a: &FindFilter,
    accepted: &mut i32,
) -> bool {
    let f = a.flags;
    // Step 1: the limit is 0 in this record.
    if f & 0x40 != 0 && *accepted >= 0 {
        return false;
    }
    // Step 2.
    let (ux, uy) = cx.world.position(u);
    let (dx, dy) = (ux.wrapping_sub(a.at.0), uy.wrapping_sub(a.at.1));
    if dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) > a.r.wrapping_mul(a.r) {
        return false;
    }
    // Step 3.
    let Some(ty) = game.lists.unit(u).map(|e| e.ty) else {
        return false;
    };
    let mode = cx.world.unit_mode(u);
    let ok = match ty {
        UnitType::Player => {
            f & 1 != 0
                && if f & 0x1000 == 0 {
                    mode != 17 && mode != 0
                } else {
                    mode == 17
                }
                && Some(u) != a.source
        }
        UnitType::Monster => {
            f & 2 != 0
                && if f & 0x1000 == 0 {
                    mode != 12 && mode != 0
                } else {
                    mode == 12
                }
                && (f & 4 == 0 || cx.world.is_undead(u))
        }
        UnitType::Object => f & 0x10 != 0,
        UnitType::Missile => {
            f & 8 != 0
                && cx
                    .store
                    .get(u)
                    .and_then(|d| cx.row(i32::from(d.class)))
                    .is_some_and(|row| !row.explosion)
        }
        UnitType::Item => f & 0x20 != 0,
        _ => false,
    };
    if !ok {
        return false;
    }
    // Step 4. The coordinate list is empty in this record, so
    // 0x200 rejects nothing; the extra test is null, which 0x800
    // would call (a read through null; D2RUST skips it).
    if f & 0x80 != 0 && !cx.world.unit_flag(u, 0x4) {
        return false;
    }
    if f & 0x400 != 0 && !cx.world.unit_flag(u, 0x8) {
        return false;
    }
    if f & 0x100 != 0 {
        if let Some(rm) = room_of(game, u) {
            if cx.world.in_town(game, rm) {
                return false;
            }
        }
    }
    if f & 0x800 != 0 {
        fatal(cx, 0x0065AA40, u);
        return false;
    }
    // Step 5.
    *accepted += 1;
    true
}

/// Server-do 19 Radament death.
pub fn srv_do_19<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let f = left(cx, m);
    let at = pos(cx, m);
    // Effects through the last 24 base ticks.
    if (2..=game.ticks(24)).contains(&f) {
        corpse_effect(game, cx, 0x3002, at, m, 124, 1, false);
    } else if f == 1 {
        corpse_effect(game, cx, 0x3002, at, m, 124, 1, true);
    }
    default_flight(game, cx, m)
}

/// Server-hit 35 orb mist.
pub fn srv_hit_35<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    if unit.is_some() {
        return 0;
    }
    let Some(b) = game.lists.find_unit(UnitType::Object, data(cx, m).0 as u32) else {
        return 0;
    };
    if let Some(r) = room_of(game, b) {
        cx.world.refresh_room(game, r);
    }
    if cx.world.unit_mode(b) == 0 {
        cx.world.set_unit_mode(b, 1);
        if let Some(fc) = cx.world.object_frame_cnt1(b) {
            // The animation's frames, one per base tick.
            let at = game.after(fc >> 8);
            // An object has a timer class: the error cannot happen.
            let _ = game.schedule_event(b, 1, at, None, 0, 0);
        }
    }
    1
}

/// The missile's path to its owner's position.
fn follow_owner<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    owner: UnitId,
) {
    if !cx.world.has_path(m) {
        return;
    }
    let room = room_of(game, owner);
    let (x, y) = cx.world.position(owner);
    cx.world.path_teleport(game, m, room, x, y);
}

/// Server-do 20 blade creeper.
pub fn srv_do_20<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(o) = cx.owner(game, m).filter(|&o| !cx.world.is_dead(o)) else {
        hit_handler(game, cx, m, None, true);
        return 2;
    };
    follow_owner(game, cx, m, o);
    if let Some(d) = cx.store.get_mut(m) {
        d.current = clamp_frame(game.ticks(10));
    }
    default_flight(game, cx, m);
    1
}

/// Server-do 21 Distraction.
pub fn srv_do_21<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let sub = i16v(row.submissile1);
    if sub != 0 && cx.world.path_new_step(m) {
        let owner = cx.owner(game, m);
        let (k, level) = skill_level(cx, m);
        let (x, y) = pos(cx, m);
        missile_at(game, cx, owner, k, level, sub, x, y);
    }
    run_srv_do(game, cx, 20, m)
}

/// Server-hit 37 blade creeper.
pub fn srv_hit_37(unit: Option<UnitId>) -> i32 {
    if unit.is_some() {
        2
    } else {
        0
    }
}

// --------------------------------------------------------------------

/// `spawn_for_level(game, room, x, y)`.
pub fn spawn_for_level<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    at: (i32, i32),
) -> i32 {
    // No room reads level 0, which has no levels record:
    // the `mon` count read goes through null (unreachable for a live
    // missile, whose room is always set).
    let Some(room) = room_of(game, m) else {
        fatal(cx, 0x005B3570, m);
        return 0;
    };
    let list = cx.world.level_mon_list(game, room);
    let n = list.len() as i32;
    if n == 0 {
        fatal(cx, 0x005B3570, m);
        return 0;
    }
    // The active room's seed. A room without an
    // seam without a provider answers none: nothing is drawn.
    let Some(seed) = cx.world.room_seed(game, room) else {
        return 0;
    };
    let mut i = seed.roll(n) as i32;
    let mut found = None;
    for _ in 0..n {
        i += 1;
        if i == n {
            i = 0;
        }
        let c = list[i as usize];
        match cx.world.monster_is_spawn(c) {
            None => {
                // An invalid class reads through a null record.
                fatal(cx, 0x005B3570, m);
                return 0;
            }
            Some(true) => {
                found = Some(c);
                break;
            }
            Some(false) => {}
        }
    }
    // Step 4: all n without `isSpawn` → fatal; the "c = −1 → 0" test is
    // dead (an invalid class already failed inside the search).
    let Some(c) = found else {
        fatal(cx, 0x005B3570, m);
        return 0;
    };
    i32::from(cx.world.create_monster(game, room, c, at))
}

/// Server-hit 39 imp spawn monsters.
pub fn srv_hit_39<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let at = pos(cx, m);
    spawn_for_level(game, cx, m, at);
    1
}

/// `scatter_at_target(game, U, owner, class, n, r, skill, level)` =.
#[allow(clippy::too_many_arguments)]
pub fn scatter_at_target<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    owner: Option<UnitId>,
    class: i32,
    n: i32,
    r: i32,
    skill: i32,
    level: i32,
) {
    let Some((tx, ty)) = cx.world.target_position(game, u) else {
        return;
    };
    *cx.world.seed(u) = Seed::init_low(tx as u32);
    let mut p = MissileParams {
        flags: pf::TARGET_ABSOLUTE | pf::FRAMES_FROM_DISTANCE,
        owner,
        origin: Some(u),
        class,
        skill,
        level,
        ..MissileParams::default()
    };
    if n <= 1 || r < 2 {
        p.target_x = tx;
        p.target_y = ty;
        create_missile(game, cx, &p);
        return;
    }
    let (ux, uy) = cx.world.position(u);
    for _ in 0..n {
        let seed = cx.world.seed(u);
        let px = tx
            .wrapping_sub(r)
            .wrapping_add(seed.roll(r.wrapping_mul(2)) as i32);
        let py = ty
            .wrapping_sub(r)
            .wrapping_add(seed.roll(r.wrapping_mul(2)) as i32);
        let (dx, dy) = (px.wrapping_sub(ux), py.wrapping_sub(uy));
        if dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) >= 4 {
            p.target_x = px;
            p.target_y = py;
            create_missile(game, cx, &p);
        }
    }
}

/// Server-hit 40 catapult spike ball.
pub fn srv_hit_40<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    let mut n =
        par(row.shitpar1).wrapping_add(level.wrapping_sub(1).wrapping_mul(par(row.shitpar2)));
    if n <= 0 {
        if !cx.world.skill_exists(k) {
            return 1;
        }
        n = calc_max(game, cx, Some(o), k, SkillCalc::Calc4, level, 1);
    }
    let owner = cx.owner(game, m);
    scatter_at_target(game, cx, m, owner, h, n, n / 4, k, level);
    1
}

/// Server-hit 43 Healing Vortex.
pub fn srv_hit_43<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let Some(u) = unit else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 1;
    }
    let owner = cx.owner(game, m);
    let (lo, hi) = cx.world.skill_phys(game, owner, k, level);
    let v = lo.wrapping_add(cx.world.seed(m).roll(hi.wrapping_sub(lo)) as i32);
    let life = cx.world.stat(u, 6);
    let max = cx.world.max_life(u);
    let po = i32::from(row.progoverlay as i16);
    if life != max && po > 0 {
        cx.world.overlay(game, u, po);
    }
    cx.world.set_stat(u, 6, life.wrapping_add(v).min(max));
    if row.collidekill != 0 {
        1
    } else {
        0
    }
}

/// Server-hit 47 Molten Boulder.
pub fn srv_hit_47<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    if let Some(u) = unit {
        if type_of(game, u) != Some(UnitType::Monster) || !cx.world.is_large_monster(u) {
            return 2;
        }
    }
    let at = pos(cx, m);
    let mut c = 1;
    let mut rec = full_record(cx, m, unit);
    let mut r = par(row.shitpar1);
    if r <= 0 {
        let (k, level) = skill_level(cx, m);
        if !cx.world.skill_exists(k) {
            return 1;
        }
        r = calc_max(game, cx, Some(o), k, SkillCalc::AuraRange, level, 1);
    }
    row_flags(&mut rec, &row);
    if area_damage(game, cx, Some(o), at, r, &rec, 0) == 0 {
        c = 3;
    }
    scatter(
        game,
        cx,
        m,
        i16v(row.hitsubmissile1),
        r,
        par(row.shitpar2).max(1),
    );
    c
}

/// Server-hit 48 Molten Boulder emerge.
pub fn srv_hit_48<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let owner = cx.owner(game, m);
    let (x, y) = pos(cx, m);
    let (tx, ty) = cx.world.path_target_point(m);
    let p = MissileParams {
        flags: pf::POSITION | pf::TARGET_ABSOLUTE,
        x,
        y,
        target_x: tx,
        target_y: ty,
        ..sub_params(cx, m, owner, h)
    };
    create_missile(game, cx, &p);
    1
}

/// Server-hit 50 plague vines trail.
pub fn srv_hit_50<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let total = cx.store.get(m).map_or(0, |d| i32::from(d.total));
    // Hits stop `shitpar1` base ticks before the end.
    let stop = total.wrapping_sub(game.ticks(par(row.shitpar1)));
    if unit.is_some() && elapsed(cx, m) < stop {
        2
    } else {
        0
    }
}

/// Server-do 27 Tornado.
pub fn srv_do_27<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 2;
    }
    let owner = cx.owner(game, m);
    let e = elapsed(cx, m);
    let mut n = par(row.param1);
    if n <= 0 {
        n = calc_max(game, cx, owner, k, SkillCalc::Calc4, level, 1);
    }
    // A hit every `n` base ticks.
    let n = game.ticks(n);
    if e % n == 0 {
        let mut rec = full_record(cx, m, None);
        let mut r = par(row.param2);
        if r <= 0 {
            r = calc_max(game, cx, owner, k, SkillCalc::AuraRange, level, 1);
        }
        row_flags(&mut rec, &row);
        let f = cx.world.skill_field(k, SkillField::AuraFilter) as u32;
        // The tornado is the source; (0, 0) makes the scan
        // use its position.
        area_damage(game, cx, Some(m), (0, 0), r, &rec, f);
    }
    default_flight(game, cx, m)
}

/// Server-hit 51 volcano debris.
pub fn srv_hit_51<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h1 = i16v(row.hitsubmissile1);
    if h1 < 0 {
        return 1;
    }
    let owner = cx.owner(game, m);
    let mut p = MissileParams {
        origin: Some(m),
        ..sub_params(cx, m, owner, h1)
    };
    create_missile(game, cx, &p);
    for h in [row.hitsubmissile2, row.hitsubmissile3] {
        if i16v(h) >= 0 {
            p.class = i16v(h);
            create_missile(game, cx, &p);
        }
    }
    1
}

// --------------------------------------------------------------------

/// Server-do 29 recycler delay (`mana` false) and 33
/// vine recycler delay (`mana` true).
fn recycler<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    mana: bool,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let (k, level) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 2;
    }
    let Some(o) = cx.owner(game, m).filter(|&o| !cx.world.is_dead(o)) else {
        return default_flight(game, cx, m);
    };
    // Once, `param1` base ticks in.
    if elapsed(cx, m) == game.ticks(par(row.param1)) {
        let (s, max) = if mana {
            (8, cx.world.max_mana(o))
        } else {
            (6, cx.world.max_life(o))
        };
        let v = cx.world.stat(o, s) >> 8;
        let mx = max >> 8;
        if v < mx {
            let p = cx
                .world
                .skill_calc(game, Some(o), k, SkillCalc::Calc1, level);
            let v2 = v.wrapping_add(pct(mx, p)).min(mx);
            cx.world.set_stat(o, s, v2.wrapping_shl(8));
            let po = i32::from(row.progoverlay as i16);
            if po >= 1 && po < cx.world.overlay_count() {
                cx.world.overlay(game, o, po);
            }
        }
    }
    default_flight(game, cx, m)
}

/// Server-do 29 recycler delay.
pub fn srv_do_29<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    recycler(game, cx, m, false)
}

/// Server-do 33 vine recycler delay.
pub fn srv_do_33<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    recycler(game, cx, m, true)
}

/// Server-do 30 rabies plague.
pub fn srv_do_30<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let sub = i16v(row.submissile1);
    if sub < 0 {
        return 2;
    }
    let (k, _) = skill_level(cx, m);
    if !cx.world.skill_exists(k) {
        return 2;
    }
    let (ty, guid) = data(cx, m);
    let u = unit_by(game, ty, guid);
    let owner = cx.owner(game, m);
    let (Some(u), Some(o)) = (u, owner) else {
        hit_handler(game, cx, m, None, true);
        return 2;
    };
    if cx.world.is_dead(o) || left(cx, m) < 0 {
        hit_handler(game, cx, m, None, true);
        return 2;
    }
    follow_owner(game, cx, m, o);
    // Every `param1` base ticks.
    if elapsed(cx, m) % game.ticks(par(row.param1)).max(1) == 0 {
        let r = par(row.param2);
        let span = r.wrapping_mul(2).wrapping_add(1);
        let seed = cx.world.seed(u);
        let dx = (seed.roll(span) as i32).wrapping_sub(r);
        let dy = (seed.roll(span) as i32).wrapping_sub(r);
        let p = MissileParams {
            flags: pf::TARGET_RELATIVE,
            origin: Some(m),
            target_x: dx,
            target_y: dy,
            ..sub_params(cx, m, Some(u), sub)
        };
        let n = create_missile(game, cx, &p);
        let s = cx.world.skill_field(k, SkillField::AuraTargetState);
        if s > 0 {
            if let (Some(n), Some(exp)) = (n, cx.world.state_list_expiry(o, s)) {
                set_d28(cx, n, exp);
            }
        }
    }
    default_flight(game, cx, m);
    1
}

/// Server-hit 53 rabies contagion.
pub fn srv_hit_53<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let (Some(o), Some(u)) = (cx.owner(game, m), unit) else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    // What is left of the carrier's rabies state (its expiry frame), as
    // base ticks like the skill's lengths.
    let t = game
        .tick_rate
        .base_ticks(data(cx, m).0.wrapping_sub(game.frame));
    if t < 10 {
        return 1;
    }
    if t > cx.world.skill_elem_len(game, o, k, level) {
        return 1;
    }
    cx.world.rabies_poison(game, o, u, t, k, level);
    2
}

/// Server-do 32 Tiger Fury.
pub fn srv_do_32<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let sub = i16v(row.submissile1);
    if sub < 0 {
        return 2;
    }
    if cx.world.path_new_step(m) {
        let owner = cx.owner(game, m);
        // The position is never written (start (0, 0)).
        let p = MissileParams {
            flags: pf::POSITION,
            origin: Some(m),
            ..sub_params(cx, m, owner, sub)
        };
        create_missile(game, cx, &p);
    }
    run_srv_do(game, cx, 7, m)
}

/// Server-hit 54 Baal spawn monsters.
pub fn srv_hit_54<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    if unit.is_some() {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let (k, _) = skill_level(cx, m);
    let Some(class) = cx.world.skill_entry_param1(o, k) else {
        return 1;
    };
    let room = room_of(game, m);
    let at = pos(cx, m);
    cx.world.spawn_monster(game, room, class, at, 1);
    1
}

/// Server-hit 55 Baal inferno.
pub fn srv_hit_55<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(u) = unit.filter(|&u| type_of(game, u) == Some(UnitType::Player)) else {
        return 2;
    };
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let p = par(row.shitpar1);
    if p <= 0 {
        return 1;
    }
    let mana = cx.world.stat(u, 8);
    if mana <= 0 {
        return 1;
    }
    let loss = pct(mana, p.clamp(1, 100)).max(1).min(mana);
    cx.world.set_stat(u, 8, mana - loss);
    2
}

/// `tyrael(game, missile)`.
fn tyrael<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) {
    set_d28(cx, m, 1);
    let room = room_of(game, m);
    if cx.world.quest_test(game, 36) {
        cx.world.spawn_tyrael(game, room, m);
    }
    // The refresh is outside the test branch, whatever it gave.
    if let Some(r) = room {
        cx.world.refresh_room(game, r);
    }
}

/// Server-do 36 Baal FX control.
pub fn srv_do_36<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    // In the last 100 base ticks.
    if data(cx, m).0 == 0 && left(cx, m) <= game.ticks(100) {
        tyrael(game, cx, m);
    }
    default_flight(game, cx, m)
}

/// Server-hit 57 Baal FX control.
pub fn srv_hit_57<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    if unit.is_none() && data(cx, m).0 == 0 {
        tyrael(game, cx, m);
    }
    0
}

/// Server-hit 59 Baal taunt poison control.
pub fn srv_hit_59<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    ring(
        game,
        cx,
        m,
        Some(o),
        Some(m),
        h,
        k,
        level,
        par(row.shitpar1).max(1),
        2,
        0,
    );
    1
}

/// Server-do 37 (unused).
pub fn srv_do_37<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 2;
    };
    let e = elapsed(cx, m);
    for (sub, at) in [
        (row.submissile1, row.param1),
        (row.submissile2, row.param2),
        (row.submissile3, row.param3),
    ] {
        let sub = i16v(sub);
        // Each sub-missile once, `param` base ticks in.
        if sub > 0 && e == game.ticks(par(at)) {
            let owner = cx.owner(game, m);
            let (k, level) = skill_level(cx, m);
            let (x, y) = pos(cx, m);
            missile_at(game, cx, owner, k, level, sub, x, y);
        }
    }
    default_flight(game, cx, m)
}

/// Server-hit 5 (unused).
pub fn srv_hit_5<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let h = i16v(row.hitsubmissile1);
    if h < 0 {
        return 1;
    }
    let Some(o) = cx.owner(game, m) else {
        return 1;
    };
    orb_circle(game, cx, m, o, h, par(row.shitpar1));
    3
}

/// Server-hit 6 (unused).
pub fn srv_hit_6<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let c = par(row.shitpar1);
    if (0..cx.world.monstats_count()).contains(&c) {
        let mode = match par(row.shitpar2) {
            v @ 0..=15 => v,
            _ => 1,
        };
        let room = room_of(game, m);
        let at = pos(cx, m);
        cx.world.spawn_monster(game, room, c, at, mode);
    }
    1
}

/// Server-hit 11 (unused).
pub fn srv_hit_11<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
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
        let p = MissileParams {
            origin: Some(m),
            ..sub_params(cx, m, owner, class)
        };
        if let Some(n) = create_missile(game, cx, &p) {
            if par(row.shitpar1) > 0 {
                hit_handler(game, cx, n, unit, true);
            }
        }
    }
    1
}

/// Server-hit 23 (unused).
pub fn srv_hit_23<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    unit: Option<UnitId>,
) -> i32 {
    let Some(row) = cx.row_of(m).cloned() else {
        return 1;
    };
    let (k, level) = skill_level(cx, m);
    if k > 0 && level > 0 && cx.owner(game, m).is_some() {
        if let Some(u) = unit {
            cx.world.set_target_unit(m, u);
        }
        cx.world
            .skill_srv_do(game, cx.store, m, par(row.shitpar1), k, level);
    }
    1
}
