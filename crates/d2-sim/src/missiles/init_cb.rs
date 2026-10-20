//! The skills code's missile init callbacks (record +0x54, argument
//! +0x58), run by creation step 21 with the missile store in hand: the
//! jitter, DiabWall, lightning fan
//! and ring (each re-seeds the new missile) and the
//! damage percent. The bodies live in the skills module
//! ([`crate::skills::use_::bodies`]); this module lends them the missile
//! data (frames) and the path seams of [`super::MissileBodies`].

use crate::game::Game;
use crate::rng::Seed;
use crate::skills::use_::bodies::{
    diab_wall_cb, init_cb, jitter, zigzag_cb, zigzag_ring_cb, JitterMissile, PathMissile,
};
use crate::units::UnitId;

use super::{clamp_frame, stat, Ctx, MissileWorld};

/// Runs the init callback `cb` with argument `a` on the new missile `m`
/// when it is one of the skills spec's; false for any other id (the
/// caller hands it to [`super::MissileHooks::init_callback`]).
pub fn run<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    cb: u32,
    a: u32,
) -> bool {
    let mut w = InitCx { game, cx };
    match cb {
        init_cb::JITTER => jitter(&mut w, m, a),
        init_cb::DIAB_WALL => diab_wall_cb(&mut w, m, a),
        init_cb::ZIGZAG => zigzag_cb(&mut w, m, a),
        init_cb::ZIGZAG_RING => zigzag_ring_cb(&mut w, m, a),
        init_cb::DAMAGE_PERCENT => damage_percent(w.cx, m, a as i32),
        _ => return false,
    }
    true
}

/// a ≠ 0 → `damagepercent(25)` := its value + a.
fn damage_percent<W: MissileWorld + ?Sized>(cx: &mut Ctx<'_, W>, m: UnitId, a: i32) {
    if a != 0 {
        let v = cx.world.stat(m, stat::DAMAGEPERCENT);
        cx.world.set_stat(m, stat::DAMAGEPERCENT, v.wrapping_add(a));
    }
}

/// The callbacks' view of one missile: its data in the store, its unit
/// (seed, position) and its path on the world.
struct InitCx<'g, 'c, 'a, W: ?Sized> {
    game: &'g mut Game,
    cx: &'c mut Ctx<'a, W>,
}

impl<W: MissileWorld + ?Sized> JitterMissile for InitCx<'_, '_, '_, W> {
    type Missile = UnitId;
    /// The missile's lifetime in base ticks: the callbacks cap it and size
    /// the path by it as the tables state it, whatever the game's rate.
    fn total_frames(&self, m: UnitId) -> i32 {
        let total = self.cx.store.get(m).map_or(0, |d| i32::from(d.total));
        self.game.tick_rate.base_ticks(total)
    }
    /// Sets the lifetime (total and frames left) to `v` base ticks, stored
    /// as ticks at the game's rate.
    fn set_frames(&mut self, m: UnitId, v: i32) {
        let v = self.game.ticks(v);
        if let Some(d) = self.cx.store.get_mut(m) {
            d.total = clamp_frame(v);
            d.current = clamp_frame(v);
        }
    }
    fn path_target_x(&self, m: UnitId) -> i32 {
        self.cx.world.path_target_point(m).0
    }
    fn set_seed(&mut self, m: UnitId, s: Seed) {
        *self.cx.world.seed(m) = s;
    }
    fn set_path_type(&mut self, m: UnitId, ty: u8) {
        self.cx.world.set_path_type(m, i32::from(ty));
    }
    fn set_steps(&mut self, m: UnitId, n: i32) {
        self.cx.world.set_path_distance(m, n);
    }
    fn compute_path(&mut self, m: UnitId) {
        self.cx.world.build(self.game, m);
    }
}

impl<W: MissileWorld + ?Sized> PathMissile for InitCx<'_, '_, '_, W> {
    fn missile_seed(&mut self, m: UnitId) -> &mut Seed {
        self.cx.world.seed(m)
    }
    fn missile_position(&self, m: UnitId) -> (i32, i32) {
        self.cx.world.position(m)
    }
    fn path_target(&self, m: UnitId) -> (i32, i32) {
        self.cx.world.path_target_point(m)
    }
    fn missile_dir64(&self, m: UnitId, at: (i32, i32)) -> i32 {
        self.cx.world.dir64(m, at)
    }
    fn set_point(&mut self, m: UnitId, i: i32, at: (u16, u16)) {
        self.cx.world.set_path_point(m, i, at);
    }
    fn set_point_count(&mut self, m: UnitId, n: i32) {
        self.cx.world.set_path_point_count(m, n);
    }
}
