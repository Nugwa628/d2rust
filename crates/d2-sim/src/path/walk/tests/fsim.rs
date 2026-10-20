use super::gaps::{other_info, other_setup, owned_path};
use super::tables;
use crate::path::walk::find::Finder;
use crate::path::walk::seams::Point;
use crate::units::UnitType;

#[test]
fn knockback_client_places_target_and_returns_one() {
    let t = tables();
    let mut c = other_setup(&[]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let mut path = owned_path(14);
    path.cur_point = 3;
    path.point_count = 7;
    path.velocity = 0x600;
    let info = other_info(11, (10, 10), (14, 10), 14);
    assert_eq!(
        crate::path::walk::other::knockback_client(&mut f, &mut path, &info),
        1
    );
    assert_eq!(path.point(0), Point::new(14, 10));
    assert_eq!(path.cur_point, 0);
    // The count is not written.
    assert_eq!(path.point_count, 7);
}

#[test]
fn knockback_client_zero_velocity_stays_put() {
    // Velocity 0 → P := position.
    let t = tables();
    let mut c = other_setup(&[]);
    let mut f = Finder {
        t: &t,
        c: &mut c,
        owner_ty: UnitType::Monster,
    };
    let mut path = owned_path(14);
    let info = other_info(11, (10, 10), (14, 10), 14);
    assert_eq!(
        crate::path::walk::other::knockback_client(&mut f, &mut path, &info),
        1
    );
    assert_eq!(path.point(0), Point::new(10, 10));
}
