//! Unit tests of the light map, the records and the contributions on
//! synthetic light maps.

use std::cell::RefCell;

use super::contribute::{self, oct};
use super::map::{Ambient, AmbientScene, LightMap, NearRoom, MAP_BYTES};
use super::records::{
    unit_light_pos, unit_type, LightError, LightKind, LightList, LightRecord, LightRooms,
    LightWorld, Owner, RoomId,
};

/// An owner entry: (owner, precise position, sub-tile, room).
type OwnerEntry = (Owner, (i32, i32), (i32, i32), Option<RoomId>);

/// A world with configurable owners, rooms and blockers.
#[derive(Default)]
struct World {
    /// Owners: (owner, precise position, sub-tile, room).
    owners: Vec<OwnerEntry>,
    local: Option<Owner>,
    blocked: Vec<(i32, i32)>,
    /// Cells `(x, y)` that belong to a room other than 1.
    rooms: Vec<((i32, i32), RoomId)>,
    probes: RefCell<Vec<(i32, i32)>>,
}

impl World {
    fn find(&self, o: &Owner) -> Option<&OwnerEntry> {
        self.owners.iter().find(|e| e.0 == *o)
    }
}

impl LightRooms for World {
    fn owner_position(&self, owner: &Owner) -> Option<(i32, i32)> {
        self.find(owner).map(|e| e.1)
    }
    fn owner_subtile(&self, owner: &Owner) -> Option<(i32, i32)> {
        self.find(owner).map(|e| e.2)
    }
    fn owner_room(&self, owner: &Owner) -> Option<RoomId> {
        self.find(owner).and_then(|e| e.3)
    }
    fn cell_room(&self, _room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.probes.borrow_mut().push((x, y));
        Some(self.rooms.iter().find(|e| e.0 == (x, y)).map_or(1, |e| e.1))
    }
}

impl LightWorld for World {
    fn is_local_player(&self, owner: &Owner) -> bool {
        self.local == Some(*owner)
    }
    fn owner_blocks(&self, _owner: &Owner, x: i32, y: i32) -> bool {
        self.blocked.contains(&(x, y))
    }
}

fn owner(t: u32, guid: u32) -> Owner {
    Owner {
        unit_type: t,
        guid,
        client_only: false,
    }
}

/// The shared test setting: a dark map around a player at (100, 100), origin 76.
fn empty_map() -> LightMap {
    LightMap::new((100, 100))
}

/// The shared test record: `I` 255, white, at 804, 804 (sub-tile 100, 100), radius 13.
fn rec(kind: LightKind, owner_type: u32) -> LightRecord {
    let mut l = LightList::new();
    let o = (owner_type != 6).then_some(owner(owner_type, 1));
    let id = l
        .create(o, (804, 804), kind, 13, 255, 255, 255, 255)
        .unwrap();
    l.get(id).unwrap().clone()
}

fn intensity(map: &LightMap, sx: i32, sy: i32) -> u8 {
    map.cell(sx - map.origin.0, sy - map.origin.1).unwrap().i
}

fn block(map: &mut LightMap, sx: i32, sy: i32) {
    let (ox, oy) = map.origin;
    map.cell_mut(sx - ox, sy - oy).unwrap().blocks = 1;
}

#[test]
fn oct_vectors() {
    assert_eq!((oct(4, 4), oct(36, 4), oct(108, 4)), (5, 36, 105));
    assert_eq!(oct(4, 36), 36);
}

#[test]
fn plain_vectors() {
    let mut map = empty_map();
    let r = rec(LightKind::Plain, 6);
    assert_eq!(r.radius, 104);
    contribute::plain(&mut map, &r, 2, false, true);
    for (sx, want) in [
        (100, 242),
        (101, 242),
        (105, 166),
        (110, 73),
        (112, 36),
        (113, 17),
        (114, 0),
    ] {
        assert_eq!(intensity(&map, sx, 100), want, "cell ({sx}, 100)");
    }
    // White light on black: the colour mix gives (255·242·T[242]) >> 16 = 254 (the
    // truncated reciprocal loses one step).
    let c = map.cell(24, 24).unwrap();
    assert_eq!((c.r, c.g, c.b), (254, 254, 254));
}

#[test]
fn radius_and_window_rejects() {
    let mut map = empty_map();
    let mut r = rec(LightKind::Plain, 6);
    r.radius = 0;
    contribute::plain(&mut map, &r, 2, false, true);
    r.radius = 256;
    contribute::plain(&mut map, &r, 2, false, true);
    assert_eq!(map, empty_map());
    assert!(contribute::window(&map, 804, 804, 104).is_some());
    // x0 >> 3 = 76 + 49 > ox + 48.
    assert!(contribute::window(&map, 8 * 125 + 104, 804, 104).is_none());
    assert!(contribute::window(&map, 8 * 124 + 104, 804, 104).is_some());
}

#[test]
fn q0_monster_radius_8() {
    let mut map = empty_map();
    let r = rec(LightKind::Plain, unit_type::MONSTER);
    contribute::plain(&mut map, &r, 0, false, true);
    for gy in 0..48 {
        for gx in 0..48 {
            let (sx, sy) = (gx + 76, gy + 76);
            let want = if (100..=101).contains(&sx) && (100..=101).contains(&sy) {
                95
            } else {
                0
            };
            assert_eq!(map.cell(gx, gy).unwrap().i, want, "({sx}, {sy})");
        }
    }
    // Smaller monster lights are forced up to 8 as well.
    let mut small = rec(LightKind::Plain, unit_type::MONSTER);
    small.radius = 8;
    assert_eq!(contribute::plain_radius(&small, 0, false), Some(8));
    small.radius = 16 * 8;
    assert_eq!(contribute::plain_radius(&small, 0, false), Some(8));
}

#[test]
fn q0_caps_by_owner_type() {
    let p = rec(LightKind::Plain, unit_type::PLAYER);
    assert_eq!(contribute::plain_radius(&p, 0, false), Some(16));
    assert_eq!(contribute::plain_radius(&p, 0, true), Some(104));
    assert_eq!(contribute::plain_radius(&p, 1, false), Some(104));
    let mut small = p.clone();
    small.radius = 8;
    assert_eq!(contribute::plain_radius(&small, 0, false), Some(8));
    let m = rec(LightKind::Plain, unit_type::MISSILE);
    assert_eq!(contribute::plain_radius(&m, 0, false), None);
    assert_eq!(contribute::plain_radius(&m, 1, false), Some(104));
    let o = rec(LightKind::Plain, unit_type::OBJECT);
    assert_eq!(contribute::plain_radius(&o, 0, false), Some(104));
    let mut map = empty_map();
    contribute::plain(&mut map, &m, 0, false, true);
    assert_eq!(map, empty_map());
}

#[test]
fn two_sources_saturate() {
    let mut map = empty_map();
    contribute::add(&mut map, 5, 5, 200, (255, 255, 255), true);
    contribute::add(&mut map, 5, 5, 200, (255, 255, 255), true);
    assert_eq!(map.cell(5, 5).unwrap().i, 255);
    // Non-positive values and cells outside the map are dropped.
    contribute::add(&mut map, 6, 5, 0, (255, 255, 255), true);
    contribute::add(&mut map, 48, 5, 10, (255, 255, 255), true);
    contribute::add(&mut map, -1, 5, 10, (255, 255, 255), true);
    assert_eq!(map.cell(6, 5).unwrap().i, 0);
}

#[test]
fn colored_light_rule() {
    assert_eq!(contribute::recip(0), 0);
    assert_eq!(contribute::recip(1), 65536);
    assert_eq!(contribute::recip(255), 257);
    let mut map = empty_map();
    // A red source on black: I 100, R = (255·100·655) >> 16 = 254.
    contribute::add(&mut map, 0, 0, 100, (255, 0, 0), true);
    let c = *map.cell(0, 0).unwrap();
    assert_eq!((c.i, c.r, c.g, c.b), (100, 254, 0, 0));
    // A blue source of 100 more: old I 100 weights the old color.
    contribute::add(&mut map, 0, 0, 100, (0, 0, 255), true);
    let c = *map.cell(0, 0).unwrap();
    let t = 65536 / 200;
    assert_eq!(c.i, 200);
    assert_eq!(i64::from(c.r), (254 * 100 * t) >> 16);
    assert_eq!(i64::from(c.b), (255 * 100 * t) >> 16);
    assert_eq!(c.g, 0);
    // Colored flag 0: R, G, B := 0.
    contribute::add(&mut map, 0, 0, 10, (0, 0, 255), false);
    let c = *map.cell(0, 0).unwrap();
    assert_eq!((c.i, c.r, c.g, c.b), (210, 0, 0, 0));
}

#[test]
fn shadowed_vectors() {
    let mut map = empty_map();
    block(&mut map, 102, 100);
    let r = rec(LightKind::Shadowed, 6);
    let g = contribute::shadow_grid(&map, &r).unwrap();
    let s = |sx: i32, sy: i32| g.shade(32 + sy - 100, 32 + sx - 100);
    assert_eq!((s(101, 100), s(102, 100)), (0, 0));
    assert_eq!((s(103, 100), s(104, 100), s(110, 100)), (16, 16, 16));
    assert_eq!((s(104, 101), s(106, 101), s(106, 102)), (7, 9, 4));
    // Ring 0 and 1 are never written.
    for (dx, dy) in [(0, 0), (1, 0), (1, 1), (-1, 1)] {
        assert_eq!(s(100 + dx, 100 + dy), 0);
    }
    contribute::shadowed(&mut map, &r, true);
    let i = |sx, sy| intensity(&map, sx, sy);
    assert_eq!((i(101, 100), i(102, 100)), (242, 223));
    assert_eq!((i(103, 100), i(104, 100), i(110, 100)), (0, 0, 0));
    assert_eq!((i(104, 101), i(106, 101), i(106, 102)), (116, 74, 104));

    let mut plain = empty_map();
    contribute::plain(&mut plain, &r, 2, false, true);
    let p = |sx, sy| intensity(&plain, sx, sy);
    assert_eq!((p(104, 101), p(106, 101), p(106, 102)), (186, 149, 139));
}

#[test]
fn shadowed_outside_map_blocks() {
    // A light near the map edge: grid cells outside the map are walls.
    let map = LightMap::new((100, 100));
    let mut r = rec(LightKind::Shadowed, 6);
    r.x = 8 * 77 + 4;
    r.y = 804;
    let g = contribute::shadow_grid(&map, &r).unwrap();
    // Sub-tile 75 is outside (origin 76): B = 16, so 74 is shaded.
    assert_eq!(g.v(32, 32 - 2), 16);
    assert_eq!(g.shade(32, 32 - 3), 16);
}

#[test]
fn cached_equals_shadowed_on_own_cell() {
    let mut shadow_map = empty_map();
    block(&mut shadow_map, 102, 100);
    block(&mut shadow_map, 98, 103);
    let r = rec(LightKind::Shadowed, 6);
    contribute::shadowed(&mut shadow_map, &r, true);

    let o = owner(unit_type::OBJECT, 7);
    let world = World {
        owners: vec![(o, (100 << 16, 100 << 16), (100, 100), Some(1))],
        blocked: vec![(102, 100), (98, 103)],
        ..World::default()
    };
    let mut lights = LightList::new();
    let id = lights
        .create(
            Some(o),
            (804, 804),
            LightKind::Cached,
            13,
            255,
            255,
            255,
            255,
        )
        .unwrap();
    let mut cached_map = empty_map();
    block(&mut cached_map, 102, 100);
    block(&mut cached_map, 98, 103);
    let mut c = lights.get(id).unwrap().clone();
    assert!(!c.cache_valid);
    contribute::cached(&mut cached_map, &mut c, &world, true).unwrap();
    assert!(c.cache_valid);
    assert_eq!(c.cache.len(), 27 * 27);
    assert_eq!(cached_map, shadow_map);
    // A valid cache is reused even when the blockers change.
    let mut again = empty_map();
    let world2 = World {
        blocked: vec![],
        ..world
    };
    contribute::cached(&mut again, &mut c, &world2, true).unwrap();
    let mut no_block = shadow_map.clone();
    for cell in no_block.cells_mut() {
        cell.blocks = 0;
    }
    assert_eq!(again, no_block);
}

#[test]
fn cached_without_owner_is_fatal() {
    let world = World::default();
    let mut map = empty_map();
    let mut c = rec(LightKind::Cached, 6);
    assert_eq!(
        contribute::cached(&mut map, &mut c, &world, true),
        Err(LightError::CacheWithoutOwner)
    );
    let mut c = rec(LightKind::Cached, unit_type::OBJECT);
    assert_eq!(
        contribute::cached(&mut map, &mut c, &world, true),
        Err(LightError::CacheWithoutOwner)
    );
}

#[test]
fn unit_position_rounds() {
    // 16.16 sub-tile 100 + 0.5 → 804 + 4: cell 101 (rounded).
    assert_eq!(unit_light_pos(100 << 16), 804);
    assert_eq!(unit_light_pos((100 << 16) + 0x8000) >> 3, 101);
    assert_eq!(unit_light_pos((100 << 16) + 0x7FFF) >> 3, 100);
}

#[test]
fn list_operations() {
    let mut l = LightList::new();
    assert_eq!(
        l.create(None, (0, 0), LightKind::Plain, 0, 255, 0, 0, 0),
        None
    );
    let a = l
        .create(None, (0, 0), LightKind::Plain, 1, 255, 0, 0, 0)
        .unwrap();
    let b = l
        .create(None, (0, 0), LightKind::Shadowed, 30, 255, 0, 0, 0)
        .unwrap();
    let order: Vec<_> = l.iter().map(|(id, _)| id).collect();
    assert_eq!(order, vec![b, a]);
    let rb = l.get(b).unwrap();
    assert_eq!(
        (rb.radius, rb.target, rb.owner_type, rb.owner_guid),
        (144, 144, 6, u32::MAX)
    );
    assert_eq!(l.radius(a), Some(1));

    l.set_radius(a, 0);
    assert_eq!(l.radius(a), Some(1));
    l.set_radius(a, 40);
    assert_eq!(
        (l.get(a).unwrap().radius, l.get(a).unwrap().target),
        (144, 144)
    );
    l.set_target(a, -3);
    assert_eq!(l.get(a).unwrap().target, 144);
    l.set_target(a, 5);
    assert_eq!(
        (l.get(a).unwrap().radius, l.get(a).unwrap().target),
        (144, 40)
    );
    l.set_target(a, 19);
    assert_eq!(l.get(a).unwrap().target, 144);
    l.set_color(a, 9, 1, 2, 3);
    let ra = l.get(a).unwrap();
    assert_eq!((ra.i, ra.r, ra.g, ra.b), (9, 1, 2, 3));
}

#[test]
fn set_radius_frees_cache() {
    let mut l = LightList::new();
    let o = owner(unit_type::OBJECT, 1);
    let id = l
        .create(Some(o), (0, 0), LightKind::Cached, 3, 255, 0, 0, 0)
        .unwrap();
    assert_eq!(l.get(id).unwrap().cache.len(), 49);
    l.set_radius(id, 4);
    assert!(l.get(id).unwrap().cache.is_empty());
    assert!(!l.get(id).unwrap().cache_valid);
}

#[test]
fn remove_and_die_errors() {
    let mut l = LightList::new();
    let a = l
        .create(None, (0, 0), LightKind::Plain, 2, 255, 0, 0, 0)
        .unwrap();
    let c = l
        .create(None, (0, 0), LightKind::Cached, 2, 255, 0, 0, 0)
        .unwrap();
    assert_eq!(l.die(c), Err(LightError::DieOnCached));
    assert!(l.remove(a).is_ok());
    assert_eq!(l.remove(a), Err(LightError::NotInList));
    assert_eq!(l.len(), 1);
}

#[test]
fn dying_shrinks_and_is_removed() {
    let world = World::default();
    let mut l = LightList::new();
    let a = l
        .create(None, (804, 804), LightKind::Plain, 2, 255, 0, 0, 0)
        .unwrap();
    l.die(a).unwrap();
    let rec = l.get(a).unwrap();
    assert!(rec.dying);
    assert_eq!((rec.radius, rec.target), (16, 0));
    let mut map = empty_map();
    l.frame(&mut map, 2, &world).unwrap();
    assert_eq!(l.radius(a), Some(1));
    l.frame(&mut map, 2, &world).unwrap();
    assert!(l.is_empty());
}

#[test]
fn frame_update_position_and_radius_walk() {
    let o = owner(unit_type::MONSTER, 3);
    let world = World {
        owners: vec![(o, (90 << 16, 95 << 16), (90, 95), Some(1))],
        ..World::default()
    };
    let mut l = LightList::new();
    let a = l
        .create(Some(o), (0, 0), LightKind::Plain, 2, 255, 0, 0, 0)
        .unwrap();
    l.set_target(a, 4);
    let mut map = empty_map();
    l.frame(&mut map, 1, &world).unwrap();
    let r = l.get(a).unwrap();
    assert_eq!((r.x, r.y, r.radius), (724, 764, 24));
    l.frame(&mut map, 1, &world).unwrap();
    l.frame(&mut map, 1, &world).unwrap();
    assert_eq!(l.get(a).unwrap().radius, 32);
    l.set_target(a, 1);
    l.frame(&mut map, 1, &world).unwrap();
    assert_eq!(l.get(a).unwrap().radius, 24);

    // A record whose owner is not found keeps its position.
    let gone = owner(unit_type::MONSTER, 4);
    let b = l
        .create(Some(gone), (11, 12), LightKind::Plain, 2, 255, 0, 0, 0)
        .unwrap();
    l.frame(&mut map, 1, &world).unwrap();
    assert_eq!((l.get(b).unwrap().x, l.get(b).unwrap().y), (11, 12));

    // A dying record no longer follows its owner.
    let c = l
        .create(Some(o), (5, 6), LightKind::Plain, 3, 255, 0, 0, 0)
        .unwrap();
    l.die(c).unwrap();
    l.frame(&mut map, 1, &world).unwrap();
    assert_eq!((l.get(c).unwrap().x, l.get(c).unwrap().y), (5, 6));
}

#[test]
fn radius_walk_frees_cache() {
    let o = owner(unit_type::OBJECT, 3);
    let world = World {
        owners: vec![(o, (100 << 16, 100 << 16), (100, 100), Some(1))],
        ..World::default()
    };
    let mut l = LightList::new();
    let a = l
        .create(Some(o), (0, 0), LightKind::Cached, 2, 255, 0, 0, 0)
        .unwrap();
    let mut map = empty_map();
    l.frame(&mut map, 0, &world).unwrap();
    assert!(l.get(a).unwrap().cache_valid);
    l.set_target(a, 3);
    l.frame(&mut map, 0, &world).unwrap();
    // Freed by the walk, then rebuilt at the new size by the contribution.
    let r = l.get(a).unwrap();
    assert!(r.cache_valid);
    assert_eq!(r.cache.len(), 49);
}

#[test]
fn frame_dispatch_by_kind_and_q() {
    let world = World::default();
    let mut base = empty_map();
    block(&mut base, 102, 100);
    // Kind 0 at q = 2 is shadowed, at q = 1 plain.
    for (q, want) in [(2u8, 0u8), (1, 166)] {
        let mut l = LightList::new();
        l.create(
            None,
            (804, 804),
            LightKind::Shadowed,
            13,
            255,
            255,
            255,
            255,
        );
        let mut map = base.clone();
        l.frame(&mut map, q, &world).unwrap();
        assert_eq!(intensity(&map, 105, 100), want);
    }
    // Kind 1 is never shadowed.
    let mut l = LightList::new();
    l.create(None, (804, 804), LightKind::Plain, 13, 255, 255, 255, 255);
    let mut map = base.clone();
    l.frame(&mut map, 2, &world).unwrap();
    assert_eq!(intensity(&map, 105, 100), 166);
    // Colors depend on the list order: the head is added first.
    let mut l = LightList::new();
    l.create(None, (804, 804), LightKind::Plain, 1, 100, 0, 0, 255);
    l.create(None, (804, 804), LightKind::Plain, 1, 100, 255, 0, 0);
    let mut map = empty_map();
    l.frame(&mut map, 2, &world).unwrap();
    let mut expect = empty_map();
    contribute::add(&mut expect, 24, 24, 37, (255, 0, 0), true);
    contribute::add(&mut expect, 24, 24, 37, (0, 0, 255), true);
    let (got, want) = (map.cell(24, 24).unwrap(), expect.cell(24, 24).unwrap());
    assert_eq!(want.i, 74);
    assert_eq!(got, want);
}

#[test]
fn new_room_invalidates_cache() {
    let o = owner(unit_type::OBJECT, 3);
    let mut world = World {
        owners: vec![(o, (100 << 16, 100 << 16), (100, 100), Some(1))],
        rooms: vec![((100, 103), 2), ((100, 97), 2)],
        ..World::default()
    };
    let mut l = LightList::new();
    let a = l
        .create(Some(o), (0, 0), LightKind::Cached, 3, 255, 0, 0, 0)
        .unwrap();
    // Plain records are not looked at.
    l.create(None, (0, 0), LightKind::Plain, 3, 255, 0, 0, 0);
    let mut map = empty_map();
    l.frame(&mut map, 2, &world).unwrap();
    assert!(l.get(a).unwrap().cache_valid);

    // The owner's own room is the new room: nothing, no probe.
    l.room_created(1, &world).unwrap();
    assert!(l.get(a).unwrap().cache_valid);
    assert!(world.probes.borrow().is_empty());

    // Room 3 is not reached: all four probes in order, cache kept.
    l.room_created(3, &world).unwrap();
    assert!(l.get(a).unwrap().cache_valid);
    assert_eq!(
        *world.probes.borrow(),
        vec![(103, 100), (97, 100), (100, 103), (100, 97)]
    );

    // Room 2 holds (100, 103): the third probe ends the tests; the cache
    // memory is kept.
    world.probes.borrow_mut().clear();
    l.room_created(2, &world).unwrap();
    let r = l.get(a).unwrap();
    assert!(!r.cache_valid);
    assert_eq!(r.cache.len(), 49);
    assert_eq!(
        *world.probes.borrow(),
        vec![(103, 100), (97, 100), (100, 103)]
    );

    // Owner not found: the new-room rule cannot continue.
    world.owners.clear();
    assert_eq!(l.room_created(2, &world), Err(LightError::Fatal0x591));
    let mut l2 = LightList::new();
    l2.create(None, (0, 0), LightKind::Cached, 3, 255, 0, 0, 0);
    assert_eq!(l2.room_created(2, &world), Err(LightError::Fatal0x591));
}

fn amb(i: u8) -> Ambient {
    Ambient {
        i,
        r: 255,
        g: 255,
        b: 255,
    }
}

#[test]
fn ambient_player_room_and_none() {
    let mut map = empty_map();
    map.cell_mut(3, 3).unwrap().blocks = 1;
    let scene = AmbientScene {
        player_ambient: amb(40),
        near: vec![],
    };
    map.fill_ambient(Some(&scene));
    assert!(map
        .cells()
        .iter()
        .all(|c| c.i == 40 && c.blocks == 0 && c.r == 255));
    map.fill_ambient(None);
    assert!(map.cells().iter().all(|c| *c == Default::default()));
}

#[test]
fn ambient_near_rooms() {
    let mut map = empty_map(); // origin 76
    let scene = AmbientScene {
        player_ambient: amb(1),
        near: vec![
            // The player's room in the list is skipped (would fill all).
            NearRoom {
                rect: (0, 0, 500, 500),
                ambient: amb(99),
                is_player_room: true,
            },
            NearRoom {
                rect: (80, 80, 5, 3),
                ambient: amb(2),
                is_player_room: false,
            },
            // Later rooms overwrite earlier ones.
            NearRoom {
                rect: (85, 83, 2, 2),
                ambient: amb(3),
                is_player_room: false,
            },
            // Clipped at the map's left and top edges.
            NearRoom {
                rect: (70, 70, 7, 6),
                ambient: amb(4),
                is_player_room: false,
            },
            // Skipped: x' = 49 > 48.
            NearRoom {
                rect: (125, 90, 4, 4),
                ambient: amb(5),
                is_player_room: false,
            },
            // Not skipped: x' = 48, but no column inside the map.
            NearRoom {
                rect: (124, 90, 4, 4),
                ambient: amb(6),
                is_player_room: false,
            },
            // x' + w = -1 < 0: skipped; x' + w = 0: column 0 filled.
            NearRoom {
                rect: (70, 100, 5, 1),
                ambient: amb(7),
                is_player_room: false,
            },
            NearRoom {
                rect: (70, 110, 6, 0),
                ambient: amb(8),
                is_player_room: false,
            },
        ],
    };
    map.fill_ambient(Some(&scene));
    let i = |sx: i32, sy: i32| intensity(&map, sx, sy);
    // Room (80, 80, 5, 3) covers 80…85 × 80…83 inclusive.
    assert_eq!((i(80, 80), i(84, 82), i(85, 80), i(80, 83)), (2, 2, 2, 2));
    assert_eq!((i(86, 80), i(80, 84), i(79, 80)), (1, 1, 1));
    // (85, 83) overwritten by the later room.
    assert_eq!((i(85, 83), i(87, 85), i(88, 85)), (3, 3, 1));
    assert_eq!((i(76, 76), i(77, 76), i(78, 76)), (4, 4, 1));
    assert_eq!(i(123, 92), 1);
    assert_eq!(i(76, 100), 1);
    assert_eq!((i(76, 110), i(77, 110), i(76, 111)), (8, 1, 1));
}

#[test]
fn blocks_flags_row_major() {
    let mut map = empty_map();
    let mut calls = Vec::new();
    map.fill_blocks(|x, y| {
        calls.push((x, y));
        (x, y) == (80, 77)
    });
    assert_eq!(calls.len(), 48 * 48);
    assert_eq!(&calls[..2], &[(76, 76), (77, 76)]);
    assert_eq!(calls[48], (76, 77));
    let flagged: Vec<_> = map
        .cells()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.blocks != 0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(flagged, vec![48 + 4]);
    assert_eq!(map.cell(4, 1).unwrap().blocks, 1);
}

#[test]
fn build_order() {
    let world = World::default();
    let scene = AmbientScene {
        player_ambient: amb(10),
        near: vec![],
    };
    let mut l = LightList::new();
    l.create(
        None,
        (804, 804),
        LightKind::Shadowed,
        13,
        255,
        255,
        255,
        255,
    );
    let map = LightMap::build(
        (100, 100),
        Some(&scene),
        |x, y| (x, y) == (102, 100),
        2,
        &mut l,
        &world,
    )
    .unwrap();
    assert_eq!(map.origin, (76, 76));
    assert_eq!(map.upper(), (124, 124));
    // Flags are set before the records run: (103, 100) is in shadow.
    assert_eq!(intensity(&map, 103, 100), 10);
    assert_eq!(intensity(&map, 101, 100), 252);
    assert_eq!(map.cell(26, 24).unwrap().blocks, 1);

    // No player room: zero map, no flags, records still contribute.
    let mut l = LightList::new();
    l.create(
        None,
        (804, 804),
        LightKind::Shadowed,
        13,
        255,
        255,
        255,
        255,
    );
    let map = LightMap::build((100, 100), None, |_, _| true, 2, &mut l, &world).unwrap();
    assert!(map.cells().iter().all(|c| c.blocks == 0));
    let mut plain = empty_map();
    contribute::plain(&mut plain, &rec(LightKind::Plain, 6), 2, false, true);
    assert_eq!(map, plain);
    assert_eq!(intensity(&map, 103, 100), 205);
}

#[test]
fn bytes_and_digest() {
    let mut map = empty_map();
    *map.cell_mut(1, 0).unwrap() = super::LightCell {
        blocks: 1,
        i: 2,
        r: 3,
        g: 4,
        b: 5,
    };
    let bytes = map.bytes();
    assert_eq!(bytes.len(), MAP_BYTES);
    assert_eq!(MAP_BYTES, 18_432);
    assert_eq!(&bytes[8..16], &[1, 0, 0, 0, 2, 3, 4, 5]);
    use sha2::{Digest, Sha256};
    let want: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(map.digest(), want);
    assert_ne!(map.digest(), empty_map().digest());
    // Zero map: SHA-256 of 18,432 zero bytes.
    let zero: [u8; 32] = Sha256::digest(vec![0u8; MAP_BYTES]).into();
    assert_eq!(empty_map().digest(), zero);
}

#[test]
fn build_is_a_pure_function_of_its_inputs() {
    // The same window, ambients, flags, q and records give the same bytes
    // (no seed is an input, so the build draws nothing); changing one
    // input (q, a record's radius) changes the map.
    let world = World::default();
    let scene = AmbientScene {
        player_ambient: amb(10),
        near: vec![],
    };
    let lights = || {
        let mut l = LightList::new();
        l.create(
            None,
            (804, 804),
            LightKind::Shadowed,
            13,
            255,
            255,
            255,
            255,
        );
        l.create(None, (820, 790), LightKind::Plain, 6, 200, 255, 0, 0);
        l
    };
    let build = |q: u8, l: &mut LightList| {
        LightMap::build(
            (100, 100),
            Some(&scene),
            |x, y| (x, y) == (102, 100),
            q,
            l,
            &world,
        )
        .unwrap()
    };
    let a = build(2, &mut lights());
    let b = build(2, &mut lights());
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(a.digest(), b.digest());
    assert_ne!(build(0, &mut lights()).digest(), a.digest(), "q");
    let mut l = lights();
    let first = l.iter().next().unwrap().0;
    l.set_radius(first, 3);
    assert_ne!(build(2, &mut l).digest(), a.digest(), "radius");
}
