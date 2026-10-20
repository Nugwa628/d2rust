//! Tests for creation flag 0x400 on the wired units and the path provider:
//! the current frame comes from the distance between the missile's path
//! position and its path target point, so a lobbed missile (Fire Blast)
//! lands at its target instead of expiring on its first run.

use crate::drlg::TileRect;
use crate::missiles::{create_missile, frames_from_distance, param_flags, MissileParams};
use crate::units::UnitType;
use crate::wiring::action::tests::{Fx, LEVEL};

/// The owner's sub-tile (east of it is open floor).
const OX: i32 = 41;
const OY: i32 = 30;

/// Creates the action fixture's arrow (class 0) from (41, 30) toward
/// (tx, ty) with `flags`; its (total, current) frames and path velocity.
fn fire(tx: i32, ty: i32, flags: u32) -> ((i16, i16), i32) {
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
    ]);
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, OX, OY);
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE | flags,
        target_x: tx,
        target_y: ty,
        ..MissileParams::default()
    };
    let (m, frames) = fx
        .sim
        .missiles(&mut fx.game, |g, cx| {
            let m = create_missile(g, cx, &p)?;
            let d = cx.store.get(m)?;
            Some((m, (d.total, d.current)))
        })
        .unwrap()
        .expect("created");
    fx.assert_clean();
    let v = fx.sim.hooks().path_velocity(m);
    (frames, v)
}

#[test]
fn frames_from_distance_reads_the_path_target_distance() {
    // (41, 30) → (49, 29): max(8, 1) + 1 / 2 = 8. Total frames stay the
    // row's (50); current = (8 << 16) / (v << 4).
    let ((total, current), v) = fire(49, 29, param_flags::FRAMES_FROM_DISTANCE);
    assert!(v > 0);
    assert_eq!(total, 50);
    assert_eq!(i32::from(current), frames_from_distance(8, v) as i32);
    // M: the distance is not the "at least 1" floor.
    assert_ne!(i32::from(current), frames_from_distance(1, v) as i32);
    // (41, 30) → (47, 34): max(6, 4) + 4 / 2 = 8 as well.
    let ((_, c2), _) = fire(47, 34, param_flags::FRAMES_FROM_DISTANCE);
    assert_eq!(c2, current);
    // Without the flag the current frame is the row's range.
    let ((_, c0), _) = fire(49, 29, 0);
    assert_eq!(c0, 50);
}
