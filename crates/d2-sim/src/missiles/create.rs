//! `MISSILES_CreateMissileFromParams`.

use crate::game::Game;
use crate::rng::Seed;
use crate::units::{UnitId, UnitType};

use super::{
    clamp_frame, collide_mode, init_missile, param_flags as pf, stat, state, unit_flag, Ctx,
    MissileRow, MissileWorld, RowExt, UnitRef,
};

/// Inputs to missile creation: owner, position, target, movement, lifetime,
/// skill and optional initialization callback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissileParams {
    /// +0x00, bits of [`super::param_flags`].
    pub flags: u32,
    /// +0x04, required.
    pub owner: Option<UnitId>,
    /// +0x08.
    pub origin: Option<UnitId>,
    /// +0x0C.
    pub target: Option<UnitId>,
    /// +0x10 `missiles.txt` row.
    pub class: i32,
    /// +0x14, +0x18.
    pub x: i32,
    pub y: i32,
    /// +0x1C, +0x20.
    pub target_x: i32,
    pub target_y: i32,
    /// +0x28.
    pub velocity: i32,
    /// +0x2C.
    pub skill: i32,
    /// +0x30.
    pub level: i32,
    /// +0x34.
    pub loops: i32,
    /// +0x40.
    pub start_frame: i32,
    /// +0x44.
    pub activate: i32,
    /// +0x48.
    pub attack_bonus: i32,
    /// +0x4C.
    pub range: i32,
    /// +0x24 gfx argument (not read by creation).
    pub gfx: i32,
    /// +0x54, +0x58: init callback (opaque id for the skills code) and
    /// its argument.
    pub init: Option<(u32, u32)>,
}

/// The creation velocity. `slow` is stat 161 of the
/// owner's state-87 list when step 6 applies.
pub fn creation_velocity(row: &MissileRow, p: &MissileParams, slow: Option<i32>) -> i32 {
    let mut v = if p.flags & pf::VELOCITY == 0 {
        // Signed division truncating toward zero.
        (i32::from(row.vel) + p.level.wrapping_mul(i32::from(row.vellev)) / 8) << 8
    } else if p.flags & pf::VELOCITY_FIXED != 0 {
        p.velocity
    } else {
        p.velocity << 8
    };
    if let Some(s) = slow {
        v = s.wrapping_mul(v) / 100;
    }
    if v != 0 {
        v = v.wrapping_mul(75) / 100;
    }
    v
}

/// Frames from the distance to the target,
/// `(d << 16) / (v << 4)` unsigned, 0 when `v << 4` is 0; d at least 1.
pub fn frames_from_distance(d: i32, v: i32) -> u32 {
    let d = d.max(1) as u32;
    let div = (v as u32).wrapping_shl(4);
    d.wrapping_shl(16).checked_div(div).unwrap_or(0)
}

/// The pierce count from P and the owner's base
/// pierce_idx counter: up to 4 `lo' % 100` draws on the local seed
/// `{counter, 666}`, counting draws below P.
/// The compare is signed: a negative P never pierces.
pub fn pierce_count(p: i32, counter: i32) -> i32 {
    let mut seed = Seed::init_low(counter as u32);
    let mut n = 0;
    for _ in 0..4 {
        let d = (seed.step() % 100) as i32;
        if d >= p {
            break;
        }
        n += 1;
    }
    n
}

/// Returns the new missile, or `None` on failure.
/// A failure at step 14 (no path) leaves the allocated missile in place
/// with its every-tick event (edge case 4).
pub fn create_missile<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    p: &MissileParams,
) -> Option<UnitId> {
    // Step 1: class range and owner type first, then the record.
    let owner = p.owner?;
    let row = cx.row(p.class)?.clone();
    let owner_ty = game.lists.unit(owner)?.ty;
    if !matches!(owner_ty, UnitType::Player | UnitType::Monster) {
        return None;
    }
    crate::cov!(Missile, p.class, 0);
    // Step 2.
    let (x, y) = if p.flags & pf::POSITION != 0 {
        (p.x, p.y)
    } else {
        cx.world.position(p.origin?)
    };
    // Step 3.
    let owner_room = game.lists.unit(owner)?.room()?;
    let room = cx.world.find_room(game, owner_room, x, y)?;
    // Step 4.
    let mut tp = if p.flags & pf::TARGET_RELATIVE != 0 {
        (x.wrapping_add(p.target_x), y.wrapping_add(p.target_y))
    } else if p.flags & pf::TARGET_ABSOLUTE != 0 {
        (p.target_x, p.target_y)
    } else {
        (x, y)
    };
    // Steps 5–7.
    let slow = if row.canslow && cx.world.has_state(owner, state::SLOWMISSILES) {
        cx.world
            .state_stat(owner, state::SLOWMISSILES, stat::SKILL_HANDOFATHENA)
    } else {
        None
    };
    let v = creation_velocity(&row, p, slow);
    // Step 8.
    let mut target = p.target;
    if v != 0 {
        // TODO: The intended unit comparison order remains unresolved.
        // Here, a distinct target at the owner's position is cleared before aiming.
        if let Some(t) = target {
            if t != owner && cx.world.position(t) == cx.world.position(owner) {
                target = None;
                tp = (tp.0.wrapping_add(1), tp.1.wrapping_add(1));
            }
        } else if tp == (x, y) {
            tp = (x.wrapping_add(1), y.wrapping_add(1));
        }
        let aim = match target {
            Some(t) => cx.world.position(t),
            None => tp,
        };
        if aim.0.wrapping_sub(x).unsigned_abs() >= 100
            || aim.1.wrapping_sub(y).unsigned_abs() >= 100
        {
            return None;
        }
    }
    // Step 9: allocation (seed, GUID: units spec), then missile init.
    let class = p.class as u16;
    let m = cx
        .world
        .alloc_missile(game, class, x, y, room, row.collidetype)?;
    game.coverage.hit("missiles_spawned", || class as u32);
    init_missile(game, cx, m, class, row.collidetype);
    // Step 10.
    let mut frames = if p.flags & pf::RANGE != 0 {
        p.range
    } else {
        let mut f = i32::from(row.range_i16())
            .wrapping_add(p.level.wrapping_mul(i32::from(row.lev_range_i16())));
        if p.flags & pf::LOOPS != 0 && row.subloop != 0 {
            let span = i32::from(row.substop) - i32::from(row.substart);
            f = f.wrapping_add(p.loops.wrapping_mul(span));
        }
        f
    };
    // Step 11. The animation frame (unit +0x44) is client-visible state
    // only; TODO(units spec): store start frame << 8 when units own it.
    if p.flags & pf::START_FRAME != 0 {
        frames = frames.wrapping_sub(p.start_frame);
    }
    // Steps 12–13.
    let activate = if p.flags & pf::ACTIVATE != 0 {
        p.activate
    } else {
        i32::from(row.activate)
    };
    // The lifetime and the activation delay are stated in base ticks; the
    // counters count down one per tick, so both become ticks at the
    // game's rate. The missile collides once `activate` base ticks of its
    // life have passed.
    {
        let rate = game.tick_rate;
        let life = rate.ticks(frames);
        let d = cx.store.get_mut(m)?;
        d.total = clamp_frame(life);
        d.current = clamp_frame(life);
        d.activate = clamp_frame(life.wrapping_sub(rate.ticks(activate)));
    }
    // Step 14.
    if !cx.world.has_path(m) {
        return None;
    }
    // Step 15.
    cx.world.set_velocity(m, 0);
    match target {
        Some(t) => cx.world.set_target_unit(m, t),
        None => cx.world.set_target_point(m, tp.0, tp.1),
    }
    cx.world
        .set_footprint_mask(m, if row.collision != 0 { 0x40 } else { 0 });
    let move_mask = collide_mode(row.collidetype).map_or(0, |c| c.mask);
    cx.world.set_move_mask(m, move_mask);
    if row.candestroy {
        cx.world.set_unit_flag(m, unit_flag::CAN_BE_ATTACKED, true);
    }
    // Step 16.
    if v != 0 {
        cx.world.set_velocity(m, v);
        cx.world.build(game, m);
    }
    // Step 17.
    let owner_ref = UnitRef {
        ty: owner_ty,
        guid: game.lists.unit(owner)?.guid,
    };
    if row.lastcollide {
        cx.store.get_mut(m)?.last_collided = Some(owner_ref);
    }
    // Step 18.
    cx.world
        .set_acceleration(m, i32::from(row.accel_i16()), i32::from(row.maxvel) << 8);
    // Step 19: the missile lives as long as it takes to reach its target
    // at velocity `v` (per base tick), so the base-tick count becomes
    // ticks at the game's rate.
    if p.flags & pf::FRAMES_FROM_DISTANCE != 0 {
        let d = cx.world.target_distance(m);
        let f = frames_from_distance(d, v);
        cx.store.get_mut(m)?.current = clamp_frame(game.ticks(f as i32));
    }
    // Step 20.
    cx.world.alloc_stat_list(m);
    // Step 21.
    // The missiles module's own callback (`zigzag`) and the skills
    // callbacks ([`super::init_cb`]) run here with the
    // store; any other id is the host's.
    match p.init {
        Some((super::bodies_ext::ZIGZAG_CALLBACK, _)) => super::bodies_ext::zigzag(game, cx, m),
        Some((cb, arg)) if !super::init_cb::run(game, cx, m, cb, arg) => {
            cx.world.init_callback(game, m, cb, arg);
        }
        Some(_) => {}
        None => {}
    }
    // Step 22.
    {
        let d = cx.store.get_mut(m)?;
        d.skill = p.skill.clamp(0, 0x7FFF) as i16;
        d.level = p.level as i16;
    }
    // Step 23.
    let skill = i32::from(cx.store.get_mut(m)?.skill);
    let bits = cx
        .world
        .damage_setup(game, owner, p.origin, m, skill, p.level);
    cx.store.get_mut(m)?.flags |= bits;
    // Step 24.
    cx.store.get_mut(m)?.owner = Some(owner_ref);
    cx.world.set_unit_flag(m, unit_flag::IS_VALID_TARGET, false);
    // Step 25: the missile is type 3 and the owner a player or
    // monster here.
    if row.pierce {
        let pierce = cx
            .world
            .stat(owner, stat::SKILL_PIERCE)
            .wrapping_add(cx.world.stat(owner, stat::ITEM_PIERCE));
        if pierce != 0 {
            let counter = cx.world.base_stat(owner, stat::PIERCE_IDX);
            let n = pierce_count(pierce, counter);
            cx.world.set_stat(m, stat::PIERCE_IDX, n);
        }
    }
    // Step 26.
    if p.flags & pf::ATTACK_BONUS != 0 {
        cx.world.set_stat(m, stat::TOHIT, p.attack_bonus);
    }
    // Step 27.
    if p.flags & pf::DATA_FLAG_2 != 0 {
        cx.store.get_mut(m)?.flags |= 2;
    }
    // Step 28.
    if owner_ty == UnitType::Monster {
        cx.world.unique_mod_missile(game, cx.store, owner, m);
    }
    // Step 29.
    cx.world
        .set_stat(m, stat::DAMAGE_FRAMERATE, row.damagerate as i32);
    Some(m)
}
