//! Coverage tests for the level rules (spawn-room edge paths).

use super::fakes::*;
use crate::drlg::*;

const INIT: u32 = 644_409_375;

#[test]
fn spawn_room_position_path_has_no_centre_default() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].position = 1;
    dat.levels[2].size = [(24, 24); 3];
    dat.object_subclass = vec![0; 600];
    let mut types = FakeTypes::default();
    let mut wp = preset(0, 0, 8, 8);
    wp.flags = room_flags::WAYPOINT;
    types.rooms.insert(2, vec![wp]);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 13).unwrap();
    assert_eq!((p.x, p.y), (-1, -1));
}

#[test]
fn spawn_room_without_any_room_fails() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].size = [(24, 24); 3];
    let mut w = World::new(dat, FakeTypes::default());
    let mut d = w.drlg(INIT);
    let mut svc = w.svc();
    assert_eq!(d.spawn_room(&mut svc, 2, 0), Err(DrlgError::NoSpawnRoom));
}
