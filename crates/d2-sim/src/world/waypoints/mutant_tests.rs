//! Mutation-testing kills: each test pins a waypoint outcome that no earlier
//! test checked, so a change to it fails here.

use super::tests::*;
use super::*;

// A waypoint class has operate function 23 **and** init function 17.
#[test]
fn waypoint_class_needs_both_functions() {
    let mut d = data();
    d.objects[5] = ObjectClass {
        operate_fn: 23,
        init_fn: 0,
        frame_cnt1: 0,
    };
    d.objects[6] = ObjectClass {
        operate_fn: 0,
        init_fn: 17,
        frame_cnt1: 0,
    };
    let classes = d.waypoint_classes();
    assert!(!classes.contains(&5) && !classes.contains(&6));
}

// The room rectangle excludes its bottom edge: y < top + height.
#[test]
fn arrival_on_the_bottom_edge_does_not_match() {
    let d = data();
    let (u, mut o) = fake().objects[0];
    o.mode = 0;
    o.level = Some(3);
    let mut f = fake();
    let mut arr = ArrivalList(vec![ArrivalNode {
        room: None,
        x: 100,
        y: 240,
    }]);
    d.init_object(&mut f, &mut arr, u, &o);
    assert!(f.log.is_empty());
    assert_eq!(arr.0.len(), 1);
}
