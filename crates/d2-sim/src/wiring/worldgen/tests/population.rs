//! Population on streamed preset rooms: its room queries answered by the
//! real DRLG, its creations run through monster init and the unit
//! allocator.

use super::*;
use crate::drlg::TileRect;
use crate::monsters::population::placement::MASK_DEFAULT;
use crate::monsters::population::{preset, room, PopWorld, PresetUnit as PopPreset, RoomBox};
use crate::units::UnitType;

/// [`ISLE`] generated, its rooms (8000, 8000) and (8008, 8000)
/// streamed; the active rooms.
pub fn isle(fx: &mut Fx) -> (RoomId, RoomId) {
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let at = |fx: &Fx, x| {
        *rooms
            .iter()
            .find(|&&r| fx.drlg().room(r).rect == TileRect::new(x, 8000, 8, 8))
            .unwrap()
    };
    let (a, b) = (at(fx, 8000), at(fx, 8008));
    let active = fx.stream(&[a, b]).unwrap();
    (active[0], active[1])
}

/// The monsters of the game, by allocation.
pub fn monsters(fx: &Fx) -> Vec<UnitId> {
    let mut v = fx.game.lists.units_of_type(UnitType::Monster);
    v.sort();
    v
}

#[test]
fn population_queries_read_the_real_rooms() {
    let mut fx = Fx::new(isle_ds1s());
    let (a, b) = isle(&mut fx);
    let dr = fx.drlg().drlg_room_of(a).unwrap();
    let seed = fx.drlg().active_room(dr).unwrap().seed;
    let (x, y) = ISLE_MONSTER;
    fx.sim.host(&mut fx.game, |h| {
        assert_eq!(h.room_level(a), ISLE as i32);
        assert_eq!(
            h.room_box(a),
            RoomBox {
                x: 40000,
                y: 40000,
                width: 40,
                height: 40
            }
        );
        // A point of B found from A through its adjacency.
        assert_eq!(h.room_at(a, 40045, 40005), Some(b));
        assert_eq!(h.room_at(a, 40005, 40005), Some(a));
        assert_eq!(
            h.preset_units(a),
            [PopPreset {
                unit_type: 1,
                mode: 1,
                class: 0,
                x: x as i32,
                y: y as i32,
                has_data: false,
                done: false,
            }]
        );
        assert!(h.preset_units(b).is_empty());
        let recs = h.tile_records(a);
        assert!(!recs.is_empty());
        assert!(recs
            .iter()
            .all(|r| (8000..8009).contains(&r.x) && (8000..8009).contains(&r.y)));
        assert!(!h.collides(a, 40012, 40010, 1, MASK_DEFAULT));
        assert_eq!(h.client_count(a), 0);
        // The room seed is the active room's (+0x6C): a draw advances it.
        assert_eq!(*h.room_seed(a), seed);
        h.room_seed(a).step();
    });
    let mut s = seed;
    s.step();
    assert_eq!(fx.drlg().active_room(dr).unwrap().seed, s);
    fx.assert_clean();
}

#[test]
fn preset_monster_is_created_through_monster_init() {
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    let game_seed = fx.sim.action.sys.hooks.game_seed;
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    fx.assert_clean();
    let m = monsters(&fx);
    assert_eq!(m.len(), 1);
    let u = m[0];
    assert_eq!(fx.game.lists.unit(u).unwrap().room(), Some(a));
    // Allocation: one game-seed step for the unit seed.
    let mut g = game_seed;
    let init_seed = g.step();
    assert_eq!(fx.sim.action.sys.hooks.game_seed, g);
    let rec = fx.sim.action.sys.units.get(u).unwrap();
    assert_eq!(rec.init_seed, init_seed);
    assert_eq!(rec.class, 0);
    // Type init: unit flags 0x0A, then the preset flags 0x3000000.
    assert_eq!(rec.flags & 0x0300_000A, 0x0300_000A);
    // At the preset point: room box + DS1 offset.
    let (x, y) = ISLE_MONSTER;
    assert_eq!(
        fx.sim.action.sys.hooks.x.pos[&u],
        (40000 + x as i32, 40000 + y as i32)
    );
    let md = fx.sim.world.monsters.get(u).unwrap();
    assert_eq!((md.class, md.level_id), (0, ISLE as i32));
    // The AI control exists and the first AI setup installed a function.
    let ai = fx
        .sim
        .action
        .sys
        .hooks
        .ai_store()
        .control(u)
        .cloned()
        .unwrap();
    assert_ne!(ai.function, 0);
    // Counted in the level's region.
    assert_eq!(
        fx.sim
            .world
            .pop
            .regions
            .get(ISLE as i32)
            .unwrap()
            .evil_spawned,
        1
    );
    assert_eq!(
        fx.sim.action.sys.hooks.x.log,
        [
            format!("align {} 0", u.0),
            format!("preset {} class 0 at {x},{y}", u.0),
        ]
    );
}

/// Monsters with their positions and seeds.
type Spawned = Vec<(UnitId, (i32, i32), Seed)>;

/// A room population run on room A, whose real coordinate list is
/// one record covering it; the monsters, the
/// region's spawned and rooms-with-spawns counts.
fn populate() -> (Spawned, i32, i32) {
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.host(&mut fx.game, |h| {
        assert_eq!(
            h.coord_list(a),
            [CoordRect {
                rect: [8000, 8000, 8008, 8008],
                node_flag: 0,
                index: 1,
            }]
        );
    });
    fx.sim.create_regions();
    fx.sim
        .population(&mut fx.game, |cx| room::populate_room(cx, a));
    fx.assert_clean();
    let units = monsters(&fx);
    let out = units
        .iter()
        .map(|&u| {
            let md = fx.sim.world.monsters.get(u).unwrap();
            assert_eq!((md.class, md.level_id), (0, ISLE as i32));
            assert!(fx.sim.action.sys.hooks.ai_store().control(u).is_some());
            (
                u,
                fx.sim.action.sys.hooks.x.pos[&u],
                fx.sim.action.sys.units.get(u).unwrap().seed,
            )
        })
        .collect();
    let r = fx.sim.world.pop.regions.get(ISLE as i32).unwrap();
    (out, r.evil_spawned, r.rooms_with_spawns)
}

#[test]
fn room_population_creates_packs_through_monster_init() {
    let (units, spawned, rooms) = populate();
    assert!(!units.is_empty());
    assert_eq!(spawned, units.len() as i32);
    assert_eq!(rooms, 1);
    // Every monster stands inside the coordinate rectangle (sub-tiles).
    for (_, (x, y), _) in &units {
        assert!((40000..40040).contains(x) && (40000..40040).contains(y));
    }
    assert_eq!(units, populate().0);
}

#[test]
fn drlg_population_reads_are_the_act_drlgs() {
    let mut fx = Fx::new(isle_ds1s());
    let (a, b) = isle(&mut fx);
    let dr = fx.drlg().drlg_room_of(a).unwrap();
    let l = fx.drlg().room(dr).level;
    let n = fx.drlg().level_rooms(l).len() as i32;
    fx.sim.host(&mut fx.game, |h| {
        // The populated level and the populated room count.
        assert_eq!(h.populated_level(a), ISLE as i32);
        assert_eq!(h.populated_room_count(0, ISLE as i32), n);
        // The coordinate record index: B's one record (index 1) found from A
        // through the adjacency array; a point in no room → 0.
        assert_eq!(h.coord_index_at(a, 40045, 40005), 1);
        assert_eq!(h.coord_index_at(a, 0, 0), 0);
        // No warp-room centres on this level.
        assert!(h.warp_points(a).is_empty());
        // The spawn location: Position 0, no waypoint or warp room: the room
        // holding the level's centre (8018, 8007), i.e. (8016, 8000, 8, 8),
        // and its centre in tiles.
        assert_eq!(h.spawn_location(a, 11), Some((8020, 8004)));
    });
    // A flag-0x800000 room is not populated and not counted; the warp
    // points are the level's warp-room centres.
    fx.drlg_mut().room_mut(dr).flags |= crate::drlg::room_flags::NO_POPULATION;
    fx.drlg_mut().level_mut(l).warp_centres.push((40020, 40020));
    fx.sim.host(&mut fx.game, |h| {
        assert_eq!(h.populated_level(a), 0);
        assert_eq!(h.populated_level(b), ISLE as i32);
        assert_eq!(h.populated_room_count(0, ISLE as i32), n - 1);
        assert_eq!(h.warp_points(b), [(40020, 40020)]);
    });
    fx.assert_clean();
}

// The owner data's f1/f2 control flags.
#[test]
fn owner_data_stores_the_f1_f2_control_flags() {
    use crate::monsters::population::seams::MonsterInit;
    use crate::monsters::population::OwnerKey;
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    let u = monsters(&fx)[0];
    let flags = |fx: &mut Fx, o: OwnerKey, a: i32, b: i32, c: i32| {
        fx.sim
            .population(&mut fx.game, |cx| cx.host.set_owner_data(u, o, a, b, c));
        fx.sim.action.sys.hooks.ai_store().control(u).unwrap().flags & 3
    };
    // Leader with SetBoss + BossXfer (f1 1, f2 1) / SetBoss alone / minion.
    assert_eq!(flags(&mut fx, OwnerKey::Guid(u), 1, 1, 1), 3);
    let before = flags(&mut fx, OwnerKey::Guid(u), 1, 0, 0);
    assert_eq!(before, 3, "a zero flag never clears its bit");
    // The bits stand even when the owner lookup fails (type -1).
    assert_eq!(flags(&mut fx, OwnerKey::Guid(u), -1, 1, 0), 3);
}
