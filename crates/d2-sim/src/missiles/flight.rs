//! Shared missile flight and collision handling (server-do 1): advance movement,
//! decrement lifetime, then check terrain and eligible unit collisions.

use crate::game::Game;
use crate::units::{UnitId, UnitType};

use super::{
    clamp_frame, coll, collide_mode, hit::hit_handler, state, unit_flag, Accept, Ctx, MissileWorld,
};

/// The missile fields of the path velocity code. The path itself
/// belongs to the path module; this is the
/// acceleration and step-vector arithmetic, for the path
/// provider to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PathVelocity {
    /// Path +0x7C.
    pub velocity: i32,
    /// Path +0x84 (`MaxVel << 8`).
    pub max: i32,
    /// Path +0x88 (`Accel`).
    pub accel: i32,
    /// Path +0x8C.
    pub counter: i32,
}

impl PathVelocity {
    /// One step's velocity update: with acceleration, the
    /// counter increments; at 5 it resets and velocity += acceleration;
    /// above max → max and acceleration 0; below 0 → 0.
    pub fn advance(&mut self) {
        if self.accel == 0 {
            return;
        }
        self.counter += 1;
        if self.counter >= 5 {
            self.counter = 0;
            self.velocity = self.velocity.wrapping_add(self.accel);
            if self.velocity > self.max {
                self.velocity = self.max;
                self.accel = 0;
            }
            if self.velocity < 0 {
                self.velocity = 0;
            }
        }
    }

    /// Step vector component for a direction component,
    /// base 0x400: `((velocity × 0x400) >> 6) × dir >> 12`, arithmetic
    /// shifts.
    pub fn step_component(&self, dir: i32) -> i32 {
        (self.velocity.wrapping_mul(0x400) >> 6).wrapping_mul(dir) >> 12
    }
}

/// The collide-type callback test: the shared unit filter
/// and the callback's unit kinds.
fn accepts<W: MissileWorld + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    m: UnitId,
    accept: Accept,
    unit: UnitId,
) -> bool {
    let Some(entry) = game.lists.unit(unit) else {
        return false;
    };
    let (Some(d), Some(row)) = (cx.store.get(m), cx.row_of(m)) else {
        return false;
    };
    let kind = match accept {
        Accept::PlayersAndGoodMonsters => {
            entry.ty == UnitType::Player
                || (entry.ty == UnitType::Monster && cx.world.alignment(unit) == 2)
        }
        Accept::Monsters => entry.ty == UnitType::Monster,
        Accept::PlayersAndMonsters => matches!(entry.ty, UnitType::Player | UnitType::Monster),
        Accept::DestroyableMissiles => {
            entry.ty == UnitType::Missile
                && cx
                    .store
                    .get(unit)
                    .and_then(|o| cx.row(i32::from(o.class)))
                    .is_some_and(|r| r.candestroy)
        }
    };
    if !kind {
        return false;
    }
    if !(cx.world.unit_flag(unit, unit_flag::IS_VALID_TARGET)
        && cx.world.unit_flag(unit, unit_flag::CAN_BE_ATTACKED))
    {
        return false;
    }
    if row.nexthit != 0 && cx.world.has_state(unit, state::JUSTHIT) {
        return false;
    }
    if d.last_collided
        .is_some_and(|l| l.ty == entry.ty && l.guid == entry.guid)
    {
        return false;
    }
    match cx.owner(game, m) {
        Some(o) => cx.world.may_attack(o, unit) || row.collidefriend != 0,
        None => true,
    }
}

/// Returns the handler result (2 = remove).
pub fn default_flight<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
) -> i32 {
    // Step 1.
    let Some(row) = cx.row_of(m).cloned() else {
        return hit_handler(game, cx, m, None, true);
    };
    // Step 2.
    if cx.world.velocity(m) != 0 && !cx.world.step(game, m) {
        return hit_handler(game, cx, m, None, true);
    }
    // Step 3.
    let Some(d) = cx.store.get_mut(m) else {
        return 1;
    };
    d.current = clamp_frame(i32::from(d.current) - 1);
    let (current, activate, mode) = (d.current, d.activate, d.mode);
    if current < 1 {
        return hit_handler(game, cx, m, None, true);
    }
    // Step 4.
    if mode == 0 {
        return 1;
    }
    // Step 5.
    let Some(room) = game.lists.unit(m).and_then(|e| e.room()) else {
        return 2;
    };
    if !cx.world.has_path(m) {
        return 2;
    }
    // Step 6.
    let word = cx.world.collision_word(game, m);
    if word & (coll::WALL | coll::MISSILE_BARRIER) != 0 {
        return 2;
    }
    // Step 7.
    if mode == 6 || current > activate || word == 0 {
        return 1;
    }
    // Step 8.
    let entry = collide_mode(mode).unwrap_or(super::CollideMode {
        callback: None,
        mask: 0,
    });
    let Some(accept) = entry.callback else {
        return 2;
    };
    // Step 9.
    let size = i32::from(row.size);
    for (x, y) in cx.world.crossed_subtiles(m) {
        let hit = cx.world.collision_mask(game, room, x, y, size, entry.mask);
        if hit == 0 {
            continue;
        }
        let found = cx
            .world
            .units_at(game, room, x, y, size)
            .into_iter()
            .find(|&u| accepts(game, cx, m, accept, u));
        if let Some(u) = found {
            return hit_handler(game, cx, m, Some(u), false);
        }
        if hit & coll::MISSILE_BARRIER != 0 {
            hit_handler(game, cx, m, None, true);
            return 2;
        }
    }
    // Step 10.
    1
}
