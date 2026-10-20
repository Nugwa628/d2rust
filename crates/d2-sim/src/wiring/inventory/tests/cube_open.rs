//! Tests for 0x20 UseGridItem on the Horadric Cube.

use super::*;

/// The cube in the grid is opened by 0x20: the interaction is (type 4,
/// cube), S→C 0x77 0x15 is queued, the cube stays.
#[test]
fn use_opens_the_cube() {
    let mut w = World::new();
    let cube = w.cursor_item(BOX);
    assert_eq!(w.handle(&insert(cube, 0, 0, 0)), Ok(0));
    w.drain();
    assert_eq!(w.handle(&msg(0x20, &[cube, 0, 0])), Ok(0));
    assert!(w.unit(cube).is_some(), "the cube is not consumed");
    let me = w.player;
    assert_eq!(w.units.get(me).unwrap().interact.get(), Some((4, cube)));
    assert!(w.rest.sent.iter().any(|(_, m)| m == &[0x77, 0x15]));
}
