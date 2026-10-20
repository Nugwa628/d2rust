use super::fakes::*;
use crate::drlg::*;
use crate::rng::Seed;

const INIT: u32 = 644_409_375;
const START: u32 = 4_014_346_869;

#[test]
fn drlg_seed_and_start_seed() {
    let mut w = World::new(data(), FakeTypes::default());
    let d = w.drlg(INIT);
    assert_eq!(d.start_seed, START);
    assert_eq!(d.seed, Seed::new(START, 268_778_232));
    assert_eq!((d.staff_tomb, d.boss_tomb, d.jungle_link), (0, 0, false));
}

#[test]
fn client_copy_has_the_same_seeds() {
    let mut w = World::new(data(), FakeTypes::default());
    let server = Drlg::create(1, INIT, 0, 0, false, &w.data, &mut w.types).unwrap();
    let client = Drlg::create(1, INIT, 0, 0, true, &w.data, &mut w.types).unwrap();
    assert_eq!(
        (server.seed, server.start_seed, server.init_seed),
        (client.seed, client.start_seed, client.init_seed)
    );
    assert_eq!(server.start_seed, START);
}

#[test]
fn act2_tombs_and_act3_jungle_bit() {
    let mut w = World::new(data(), FakeTypes::default());
    let d = Drlg::create(1, INIT, 0, 0, false, &w.data, &mut w.types).unwrap();
    assert_eq!((d.staff_tomb, d.boss_tomb), (71, 70));
    // Two draws after the start seed.
    let mut s = Seed::new(START, 268_778_232);
    s.step();
    s.step();
    assert_eq!(d.seed, s);

    let d = Drlg::create(2, INIT, 0, 0, false, &w.data, &mut w.types).unwrap();
    assert!(d.jungle_link);
    assert_eq!(d.seed.lo, 1_406_222_081);
}

#[test]
fn level_seed_and_room_seeds() {
    let mut dat = data();
    gen_level(&mut dat, 1, 2);
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(1, vec![preset(0, 0, 8, 8), preset(8, 0, 8, 8)]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 1).unwrap();
    assert_eq!(d.level(l).seed, Seed::new(4_014_346_870, 666));
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let rooms = d.level_rooms(l);
    // The first room's seed after its init step.
    assert_eq!(d.room(rooms[0]).init_seed, 4_134_077_858);
    let mut first = Seed::init_low(2_928_842_600);
    first.step();
    assert_eq!(d.room(rooms[0]).seed, first);
    let mut second = Seed::init_low(1_513_463_342);
    let init2 = second.step();
    assert_eq!(d.room(rooms[1]).init_seed, init2);
    assert_eq!(d.room(rooms[1]).status, room::STATUS_NONE);
    // Re-generation re-seeds the level: same room seeds.
    let before: Vec<_> = rooms.iter().map(|&r| d.room(r).init_seed).collect();
    d.free_level_rooms(&mut w.types, l);
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let after: Vec<_> = d
        .level_rooms(l)
        .iter()
        .map(|&r| d.room(r).init_seed)
        .collect();
    assert_eq!(before, after);
}

#[test]
fn act_of_level_and_towns() {
    assert_eq!([39, 40, 109, 1024].map(act_of_level), [0, 1, 4, 0]);
    assert_eq!(act_of_level(0), 0);
    assert!(is_town(75) && !is_town(76));
}

#[test]
fn lvlwarp_first_match() {
    let mut d = data();
    d.warps = vec![
        WarpDef {
            id: 1,
            direction: b'b',
            ..WarpDef::default()
        },
        WarpDef {
            id: 0,
            direction: b'l',
            ..WarpDef::default()
        },
        WarpDef {
            id: 0,
            direction: b'r',
            ..WarpDef::default()
        },
    ];
    assert_eq!(d.lvlwarp_row(0, b'b'), Ok(1));
    assert_eq!(d.lvlwarp_row(0, b'r'), Ok(2));
    assert_eq!(d.lvlwarp_row(1, b'l'), Ok(0));
    assert_eq!(d.lvlwarp_row(7, b'b'), Err(DrlgError::NoLvlWarp(7)));
}

#[test]
fn recorded_act1_level_list_order() {
    // Allocation order of the Act 1 placer and the
    // resulting list, head first.
    let mut dat = data();
    gen_level(&mut dat, 1, 2);
    let types = FakeTypes {
        act_levels: vec![
            4, 3, 2, 1, 17, 39, 26, 7, 6, 27, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        ],
        ..FakeTypes::default()
    };
    let mut w = World::new(dat, types);
    let d = Drlg::create(0, INIT, 0, 1, false, &w.data, &mut w.types).unwrap();
    let list: Vec<u32> = d.level_list().iter().map(|&l| d.level(l).id).collect();
    assert_eq!(
        list,
        [16, 15, 14, 13, 12, 11, 10, 9, 8, 5, 27, 6, 7, 26, 39, 17, 1, 2, 3, 4]
    );
    for &l in &d.level_list() {
        let id = d.level(l).id;
        let gen = id == 1;
        // Town generated: its seed was re-initialized, no rooms drew.
        assert_eq!(
            d.level(l).seed,
            Seed::init_low(START + id),
            "level {id} gen {gen}"
        );
    }
    assert_eq!(w.types.generated, [1]);
}

#[test]
fn unknown_drlg_type_gets_no_init_and_position_from_depend() {
    let mut dat = data();
    gen_level(&mut dat, 5, 2);
    dat.levels[5].depend = 6;
    dat.levels[5].offset = (10, -3);
    dat.levels[5].size = [(20, 30), (21, 31), (22, 32)];
    dat.levels[6].drlg_type = 9; // unknown: no init
    let mut w = World::new(dat, FakeTypes::default());
    let mut d = Drlg::create(0, INIT, 1, 0, false, &w.data, &mut w.types).unwrap();
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 5).unwrap();
    assert_eq!(d.level(l).rect, TileRect::new(10, -3, 21, 31));
    assert_eq!(w.types.inits, [5]);
    // Depend was allocated first (prepend): list 5, 6.
    let list: Vec<u32> = d.level_list().iter().map(|&l| d.level(l).id).collect();
    assert_eq!(list, [5, 6]);
    // Level 0 / unknown types never generate.
    let l6 = d.find_level(6).unwrap();
    d.generate_level(&w.data, &mut w.types, l6).unwrap();
    assert_eq!(w.types.generated, Vec::<u32>::new());
    assert_eq!(
        d.get_or_alloc_level(&w.data, &mut w.types, 500),
        Err(DrlgError::UnknownLevel(500))
    );
}

#[test]
fn client_copy_flags() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    let mut types = FakeTypes::default();
    types.rooms.insert(2, vec![preset(0, 0, 8, 8)]);
    let mut w = World::new(dat, types);
    let mut d = Drlg::create(0, INIT, 0, 0, true, &w.data, &mut w.types).unwrap();
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    assert_eq!(d.level(l).flags, level::LEVEL_FLAG_CLIENT);
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    let r = d.level_rooms(l)[0];
    assert_ne!(d.room(r).flags & room_flags::AUTOMAP_REVEAL, 0);
}

#[test]
fn warp_records_and_set_warp() {
    let mut dat = data();
    dat.levels[2].vis = [1, 3, 0, 0, 0, 0, 0, 0];
    dat.levels[2].warp = [5, 6, -1, -1, -1, -1, -1, -1];
    let mut w = World::new(dat, FakeTypes::default());
    let mut d = w.drlg(INIT);
    // Readers fall back to leveldefs.
    assert_eq!(d.vis_array(&w.data, 2).unwrap()[1], 3);
    assert_eq!(d.warp_id(&w.data, 2, 1), Ok(6));
    // Existing vis: warp replaced in place.
    d.set_warp(&w.data, 2, 3, 9, -1).unwrap();
    assert_eq!(d.warp_id(&w.data, 2, 1), Ok(9));
    // New vis, any slot: first slot with vis 0 and warp −1.
    d.set_warp(&w.data, 2, 4, 11, -1).unwrap();
    assert_eq!(d.vis_array(&w.data, 2).unwrap()[2], 4);
    assert_eq!(d.warp_id(&w.data, 2, 2), Ok(11));
    // Requested slot.
    d.set_warp(&w.data, 2, 7, 12, 5).unwrap();
    assert_eq!(d.vis_array(&w.data, 2).unwrap()[5], 7);
    assert_eq!(d.warp_records().len(), 1);
    // Overflow (edge case 2): vis → level-id field, warp → vis[7].
    let rec = d.warp_record_mut(&w.data, 8).unwrap();
    rec.vis = [1, 2, 3, 4, 5, 6, 7, 9];
    rec.warp = [0; 8];
    d.set_warp(&w.data, 8, 50, 77, -1).unwrap();
    let rec = &d.warp_records()[0];
    assert_eq!((rec.level_id, rec.vis[7]), (50, 77));
    // Records are prepended.
    assert_eq!(d.warp_records()[1].level_id, 2);
}

#[test]
fn warp_record_level_zero_is_fatal() {
    let mut w = World::new(data(), FakeTypes::default());
    let mut d = w.drlg(INIT);
    d.warp_record_mut(&w.data, 0).unwrap();
    assert_eq!(d.vis_array(&w.data, 0), Err(DrlgError::WarpRecordLevelZero));
}

#[test]
fn room_at_hint_level_and_null_level() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].size = [(24, 24); 3];
    let mut types = FakeTypes::default();
    grid3x3(&mut types, 2);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    // Point in a level without rooms: generated on demand.
    let r = d
        .room_at(&w.data, &mut w.types, 9, 17, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(w.types.generated, [2]);
    assert_eq!(d.room(r).rect, TileRect::new(8, 16, 8, 8));
    // Half-open: x = 16 is the next room.
    let r2 = d
        .room_at(&w.data, &mut w.types, 16, 16, None, Some(l))
        .unwrap()
        .unwrap();
    assert_eq!(d.room(r2).rect.x, 16);
    // Hint containing the point.
    assert_eq!(
        d.room_at(&w.data, &mut w.types, 9, 17, Some(r), None),
        Ok(Some(r))
    );
    // Hint's near array.
    d.build_near(&w.data, &mut w.types, r).unwrap();
    assert_eq!(
        d.room_at(&w.data, &mut w.types, 17, 17, Some(r), None),
        Ok(Some(r2))
    );
    // Outside every level: level 0 allocated (prepended), no room.
    assert_eq!(
        d.room_at(&w.data, &mut w.types, 500, 500, None, None),
        Ok(None)
    );
    assert_eq!(d.level(d.level_list()[0]).id, 0);
}

#[test]
fn activity_counts_and_freeing_with_memory() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    gen_level(&mut dat, 3, 2);
    dat.levels[2].size = [(24, 24); 3];
    dat.levels[3].offset = (100, 0);
    dat.levels[3].size = [(8, 8); 3];
    dat.levels[2].vis = [3, 4, 0, 0, 0, 0, 0, 0];
    let mut types = FakeTypes::default();
    grid3x3(&mut types, 2);
    types.rooms.insert(3, vec![preset(100, 0, 8, 8)]);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l2 = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l2).unwrap();
    let l3 = d.get_or_alloc_level(&w.data, &mut w.types, 3).unwrap();
    d.generate_level(&w.data, &mut w.types, l3).unwrap();
    let r0 = d.level_rooms(l2)[4];
    let r3 = d.level_rooms(l3)[0];
    let client = crate::units::ClientId(0);
    {
        let mut svc = w.svc();
        d.client_changes_room(&mut svc, client, None, Some(r0))
            .unwrap();
    }
    // Level 2 and its vis levels 3 and 4 (4 allocated by the lookup).
    let l4 = d.find_level(4).unwrap();
    assert_eq!(
        (
            d.level(l2).activity,
            d.level(l3).activity,
            d.level(l4).activity
        ),
        (1, 1, 1)
    );
    assert_eq!(d.level(l2).inactive_frames, 10);
    // Mark a room populated (the tick sets the act-list flag; removal
    // copies it to other flags bit 0), then leave and free.
    let a2 = d.active_room(d.level_rooms(l2)[2]).unwrap().id;
    w.lists.room_mut(a2).unwrap().populated = true;
    {
        let mut svc = w.svc();
        d.client_changes_room(&mut svc, client, Some(r0), Some(r3))
            .unwrap();
    }
    // Moving 2 → 3: −1 on 2, 3, 4; +1 on 3 (and its vis: none).
    assert_eq!(
        (
            d.level(l2).activity,
            d.level(l3).activity,
            d.level(l4).activity
        ),
        (0, 1, 0)
    );
    // Rooms of level 2 still have statuses / active rooms: the free test
    // fails until they are gone.
    let mut svc = w.svc();
    for (room, _) in d.active_rooms() {
        let r = d.drlg_room_of(room).unwrap();
        if d.room(r).level == l2 {
            d.remove_active_room(&mut svc, room).unwrap();
        }
    }
    for _ in 0..10 {
        d.free_inactive_levels(svc.data, svc.types).unwrap();
        assert!(d.level(l2).first_room.is_some());
    }
    // Statuses of level 2 still ≤ 3 through level 3's rooms? Level 3 is
    // far away: they dropped to 4 on the move.
    assert!(d.level_rooms(l2).iter().all(|&r| d.room(r).status == 4));
    d.free_inactive_levels(svc.data, svc.types).unwrap();
    assert!(d.level(l2).first_room.is_none());
    assert_eq!(d.level(l2).room_count, 0);
    assert_eq!(
        d.level(l2).populated_memory.as_deref(),
        Some(&[false, false, true, false, false, false, false, false, false][..])
    );
    assert_eq!(w.types.resets, [2]);
    // Regeneration restores bit 0 on the third room.
    d.generate_level(&w.data, &mut w.types, l2).unwrap();
    let flags: Vec<u32> = d
        .level_rooms(l2)
        .iter()
        .map(|&r| d.room(r).other_flags)
        .collect();
    assert_eq!(flags, [0, 0, 1, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn free_test_blocks_on_warp_linked_rooms() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    gen_level(&mut dat, 3, 2);
    dat.levels[2].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    dat.levels[3].vis = [0, 2, 0, 0, 0, 0, 0, 0];
    let mut types = FakeTypes::default();
    types.rooms.insert(2, vec![preset(0, 0, 8, 8)]);
    let mut linked = preset(100, 0, 8, 8);
    linked.flags = room_flags::WARP_0 << 1;
    types.rooms.insert(3, vec![preset(108, 0, 8, 8), linked]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l2 = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l2).unwrap();
    let l3 = d.get_or_alloc_level(&w.data, &mut w.types, 3).unwrap();
    d.generate_level(&w.data, &mut w.types, l3).unwrap();
    let t = d.level_rooms(l3)[1];
    d.build_near(&w.data, &mut w.types, t).unwrap();
    d.room_mut(t).status = 3;
    assert_eq!(d.level_free_test(&w.data, &mut w.types, l2), Ok(false));
    d.room_mut(t).status = 4;
    assert_eq!(d.level_free_test(&w.data, &mut w.types, l2), Ok(true));
    // The linked room's near array was freed; the other room's was not built.
    assert!(d.room(t).near().is_none());
}

#[test]
fn warp_room_centres() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].warp = [-1, 4, -1, -1, -1, -1, -1, -1];
    let mut types = FakeTypes::default();
    let mut wp = preset(0, 0, 8, 8);
    wp.flags = room_flags::WAYPOINT_SMALL;
    let mut warp_none = preset(8, 0, 8, 8);
    warp_none.flags = room_flags::WARP_0; // slot 0: warp −1
    let mut warp = preset(16, 0, 7, 9);
    warp.flags = room_flags::WARP_0 << 1;
    types.rooms.insert(2, vec![wp, warp_none, warp]);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let l = d.get_or_alloc_level(&w.data, &mut w.types, 2).unwrap();
    d.generate_level(&w.data, &mut w.types, l).unwrap();
    assert_eq!(d.level(l).warp_centres, [(20, 20), (95, 20)]);
}

fn spawn_world(position: u32) -> (World, Drlg) {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].position = position;
    dat.levels[2].size = [(24, 24); 3];
    dat.object_subclass = vec![0; 600];
    dat.object_subclass[119] = 0x40;
    let mut types = FakeTypes::default();
    grid3x3(&mut types, 2);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let d = w.drlg(INIT);
    (w, d)
}

#[test]
fn spawn_room_by_tile_records() {
    let (mut w, mut d) = spawn_world(1);
    w.types.spawn_tiles.insert(
        2,
        vec![
            SpawnTile {
                x: 1,
                y: 1,
                index: 3,
            },
            SpawnTile {
                x: 9,
                y: 9,
                index: 0,
            },
            SpawnTile {
                x: 17,
                y: 17,
                index: 4,
            },
            SpawnTile {
                x: 2,
                y: 20,
                index: 6,
            },
        ],
    );
    // t = 0: class (1, 0) matches records with b = 0 (indexes 0..4): three.
    let l = {
        let mut svc = w.svc();
        let p = d.spawn_room(&mut svc, 2, 0).unwrap();
        let l = d.find_level(2).unwrap();
        // Level seed after generation: 9 room allocations, then roll(3).
        let mut s = Seed::init_low(START + 2);
        for _ in 0..9 {
            s.step();
        }
        let r = s.roll(3) as usize;
        let want = [(1, 1), (9, 9), (17, 17)][r];
        assert_eq!((p.x, p.y), want);
        assert_eq!(d.level(l).seed, s);
        assert!(p.active.is_some(), "spawn room is streamed");
        l
    };
    // t = 6: exact index only (class a = 0); one match → still a draw.
    let before = d.level(l).seed;
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 6).unwrap();
    assert_eq!((p.x, p.y), (2, 20));
    let mut s = before;
    s.roll(1);
    assert_eq!(d.level(l).seed, s);
    // No match: record 0, no draw.
    let before = d.level(l).seed;
    let p = d.spawn_room(&mut svc, 2, 12).unwrap();
    assert_eq!((p.x, p.y), (1, 1));
    assert_eq!(d.level(l).seed, before);
}

#[test]
fn spawn_room_waypoint_and_fallbacks() {
    let (mut w, mut d) = spawn_world(0);
    // No waypoint, no warp room: the room at the level centre − 2.
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    assert_eq!(d.room(p.room).rect, TileRect::new(8, 8, 8, 8));
    assert_eq!((p.x, p.y), (12, 12));

    // Waypoint room with a waypoint object.
    let (mut w, mut d) = spawn_world(0);
    w.types.rooms.get_mut(&2).unwrap()[5].flags = room_flags::WAYPOINT;
    w.types.preset_units.insert(
        2,
        vec![
            PresetUnit {
                unit_type: 1,
                class: 119,
                x: 0,
                y: 0,
            },
            PresetUnit {
                unit_type: 2,
                class: 3,
                x: 0,
                y: 0,
            },
            PresetUnit {
                unit_type: 2,
                class: 119,
                x: 12,
                y: 27,
            },
        ],
    );
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    assert_eq!(d.room(p.room).rect, TileRect::new(16, 8, 8, 8));
    assert_eq!((p.x, p.y), (16 + 2, 8 + 5));
    assert!(p.active.is_some());
    // Position ≠ 0 with index 13 and no waypoint room.
    let (mut w, mut d) = spawn_world(1);
    let mut svc = w.svc();
    assert_eq!(
        d.spawn_room(&mut svc, 2, 13),
        Err(DrlgError::NoWaypointRoom)
    );
}

#[test]
fn spawn_room_random_when_centre_is_empty() {
    let mut dat = data();
    gen_level(&mut dat, 2, 2);
    dat.levels[2].size = [(40, 40); 3];
    let mut types = FakeTypes::default();
    types
        .rooms
        .insert(2, vec![preset(0, 0, 8, 8), preset(30, 30, 8, 8)]);
    types.default_grid = Some(floor_grid);
    let mut w = World::new(dat, types);
    let mut d = w.drlg(INIT);
    let mut svc = w.svc();
    let p = d.spawn_room(&mut svc, 2, 0).unwrap();
    let mut s = Seed::init_low(START + 2);
    s.step();
    s.step();
    let k = s.roll(2) as usize;
    let l = d.find_level(2).unwrap();
    assert_eq!(p.room, d.level_rooms(l)[k]);
    assert_eq!(d.level(l).seed, s);
}
