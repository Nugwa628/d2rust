//! Tests for the inactive store on the action wiring: the tick's compress of
//! freed rooms and the restore of the room's next population.

use super::*;
use crate::tick::TickHooks;
use crate::units::inactive::{AreaNode, InactiveStore};

fn node(fx: &Fx) -> Vec<AreaNode> {
    fx.sim.sys.hooks.inactive.as_ref().unwrap().acts[0].clone()
}

/// With the store off, nothing happens (the old behaviour).
#[test]
fn without_the_store_compress_does_nothing() {
    let mut fx = Fx::new();
    let a = fx.a;
    let m = fx.spawn(UnitType::Missile, 0, a, 5, 5);
    let mut g = std::mem::take(&mut fx.game);
    fx.sim.compress_unit(&mut g, m);
    fx.game = g;
    assert!(fx.game.lists.unit(m).is_some());
}

/// A missile is freed without a record; a monster with
/// `SaveMonsters` 0 is freed without a record; an object with S off is
/// freed; the node of the room holds nothing.
#[test]
fn missiles_and_unsaved_monsters_are_freed() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_inactive_store();
    let a = fx.a;
    let m = fx.spawn(UnitType::Missile, 0, a, 5, 5);
    let mon = fx.spawn(UnitType::Monster, 0, a, 6, 6);
    let mut g = std::mem::take(&mut fx.game);
    fx.sim.compress_unit(&mut g, m);
    fx.sim.compress_unit(&mut g, mon);
    fx.game = g;
    assert!(fx.game.lists.unit(m).is_none());
    assert!(fx.game.lists.unit(mon).is_none());
    assert!(node(&fx).is_empty());
}

/// A living monster with node index < 8 is
/// stored (K := 1) and freed; the record carries its position, class,
/// GUID and frame; the restore of its room hands it to the host's
/// re-spawn, after the node is unlinked.
#[test]
fn a_stored_monster_is_restored_with_its_guid() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_inactive_store();
    let a = fx.a;
    // monstats2 `restore` 1 (the monster is stored).
    std::sync::Arc::make_mut(&mut fx.sim.sys.hooks.tables)
        .combat
        .monstats2[0]
        .restore = 1;
    let mon = fx.spawn(UnitType::Monster, 0, a, 6, 6);
    fx.sim.sys.units.get_mut(mon).unwrap().node_index = 3;
    let guid = fx.sim.sys.units.get(mon).unwrap().guid;
    fx.game.frame = 77;
    let mut g = std::mem::take(&mut fx.game);
    fx.sim.compress_unit(&mut g, mon);
    fx.game = g;
    assert!(fx.game.lists.unit(mon).is_none());
    let n = node(&fx);
    assert_eq!(n.len(), 1);
    let rec = &n[0].monsters[0];
    assert_eq!((rec.class, rec.guid, rec.frame), (0, guid, 77));
    let mut g = std::mem::take(&mut fx.game);
    assert!(fx.sim.restore(&mut g, a));
    fx.game = g;
    assert!(node(&fx).is_empty());
    assert_eq!(fx.sim.sys.hooks.inactive, Some(InactiveStore::default()));
    // Spawned again with the stored GUID, in mode 1, at its place.
    let back = fx
        .game
        .lists
        .find_unit(UnitType::Monster, guid)
        .expect("the monster is back");
    assert_eq!(fx.game.lists.unit(back).and_then(|e| e.room()), Some(a));
    let r = fx.sim.sys.units.get(back).unwrap();
    assert_eq!((r.class, r.mode), (0, 1));
    assert_eq!(fx.sim.sys.hooks.path_position(back), (6, 6));
}

/// An object with unit flag 0x2000000 (S) is
/// stored and freed; the restore creates a new object (new GUID) of its
/// class and mode at its place, with unit flags 0x3000000.
#[test]
fn a_stored_object_comes_back_as_a_new_unit() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_inactive_store();
    let a = fx.a;
    let o = fx.spawn(UnitType::Object, 3, a, 7, 4);
    fx.sim.sys.units.get_mut(o).unwrap().flags |= 0x200_0000;
    let guid = fx.sim.sys.units.get(o).unwrap().guid;
    let mut g = std::mem::take(&mut fx.game);
    fx.sim.compress_unit(&mut g, o);
    fx.game = g;
    assert!(fx.game.lists.unit(o).is_none());
    let n = node(&fx);
    assert_eq!((n[0].others[0].ty, n[0].others[0].class), (2, 3));
    let mut g = std::mem::take(&mut fx.game);
    assert!(fx.sim.restore(&mut g, a));
    fx.game = g;
    let back: Vec<_> = fx
        .game
        .lists
        .room_units(a)
        .into_iter()
        .filter(|&u| {
            fx.game
                .lists
                .unit(u)
                .is_some_and(|e| e.ty == UnitType::Object)
        })
        .collect();
    assert_eq!(back.len(), 1);
    let r = fx.sim.sys.units.get(back[0]).unwrap();
    assert_eq!((r.class, r.mode), (3, 1));
    assert_ne!(r.guid, guid);
    assert_eq!(r.flags & 0x300_0000, 0x300_0000);
    assert_eq!(fx.sim.sys.hooks.path_position(back[0]), (7, 4));
}

/// A portal (class 59) is kept: detached, not freed,
/// flags 2 |= 0x100; the restore places the same unit again with unit
/// flag 0x10.
#[test]
fn a_kept_portal_is_placed_again() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_inactive_store();
    let a = fx.a;
    let p = fx.spawn(UnitType::Object, 59, a, 5, 3);
    let mut g = std::mem::take(&mut fx.game);
    fx.sim.compress_unit(&mut g, p);
    fx.game = g;
    assert!(fx.game.lists.unit(p).is_some_and(|e| e.room().is_none()));
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags2 & 0x100, 0);
    let mut g = std::mem::take(&mut fx.game);
    assert!(fx.sim.restore(&mut g, a));
    fx.game = g;
    assert_eq!(fx.game.lists.unit(p).and_then(|e| e.room()), Some(a));
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags & 0x10, 0);
}

/// The town round trip (q-fix-pc1-proto-items): the preset pass gives
/// every unit it creates unit flags 0x3000000, so in a level without
/// `SaveMonsters` a preset object (a waypoint, a stash; their objects
/// rows have `Restore` 1) is `S` (flag 0x2000000) and stored when its
/// room is freed, and a warp tile is stored as always; the room's restore
/// re-creates both at their places, as new units with the flags. Before
/// the fix the preset object had no flags, was freed without a record and
/// never came back.
#[test]
fn preset_objects_and_tiles_come_back_after_their_room_is_freed() {
    use crate::drlg::PresetUnit;
    use crate::world::objects::ObjectTables;
    use d2_data::tables::{Levels, Objects};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    const WAYPOINT: u32 = 0;
    const STASH: u32 = 1;
    const TILE: u32 = 3;
    let preset = |unit_type, class, x, y| PresetUnit {
        unit_type,
        class,
        x,
        y,
    };
    let mut presets = BTreeMap::new();
    presets.insert(
        (0, 0),
        vec![
            preset(2, WAYPOINT, 20, 20),
            preset(2, STASH, 12, 26),
            preset(5, TILE, 30, 10),
        ],
    );
    let mut fx = Fx::with_presets(
        &[
            (LEVEL, TileRect::new(0, 0, 8, 8)),
            (LEVEL, TileRect::new(8, 0, 8, 8)),
        ],
        presets,
    );
    // objects rows with `Restore` 1 (the waypoint and stash rows come back
    // in the live run, `test-fixtures/tests/town_round_trip.rs`); no
    // leveldefs, so `SaveMonsters` is the seam's answer (0).
    let row = |operatefn| {
        let mut o: Objects = crate::skills::fake::blank();
        o.operatefn = operatefn;
        o.restore = 1;
        o
    };
    fx.sim.create_objects(Arc::new(ObjectTables {
        objects: vec![row(23), row(0)],
        shrines: Vec::new(),
        levels: vec![crate::skills::fake::blank::<Levels>(); 150],
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    }));
    fx.sim.hooks().enable_inactive_store();
    let a = fx.a;
    let units_of = |fx: &Fx| {
        let mut v: Vec<(UnitType, u32, (i32, i32), u32)> = fx
            .game
            .lists
            .room_units(a)
            .into_iter()
            .map(|u| {
                let ty = fx.game.lists.unit(u).unwrap().ty;
                let r = fx.sim.sys.units.get(u).unwrap();
                (
                    ty,
                    r.class,
                    fx.sim.sys.hooks.path_position(u),
                    r.flags & 0x300_0000,
                )
            })
            .collect();
        v.sort_by_key(|e| (e.0 as u8, e.1, e.2));
        v
    };
    let made = fx.sim.with(&mut fx.game, |g, v| v.spawn_preset_units(g, a));
    assert_eq!(made, 3);
    let before = units_of(&fx);
    assert_eq!(
        before,
        vec![
            (UnitType::Object, WAYPOINT, (20, 20), 0x300_0000),
            (UnitType::Object, STASH, (12, 26), 0x300_0000),
            (UnitType::Tile, TILE, (30, 10), 0x300_0000),
        ]
    );
    // The tick frees the room: every unit compressed.
    let mut g = std::mem::take(&mut fx.game);
    for u in g.lists.room_units(a) {
        fx.sim.compress_unit(&mut g, u);
    }
    fx.game = g;
    assert!(fx.game.lists.room_units(a).is_empty());
    assert_eq!(node(&fx)[0].others.len(), 3, "three records");
    // The room's next population restores them.
    let mut g = std::mem::take(&mut fx.game);
    assert!(fx.sim.restore(&mut g, a));
    fx.game = g;
    assert_eq!(units_of(&fx), before);
    fx.assert_clean();
}

#[test]
fn monster_region_accounting_survives_unloading_and_restoring() {
    use crate::monsters::init::MonsterData;
    use crate::wiring::worldgen::tests::{Ds1s, Fx as WorldFx};
    for (not_counted, death_counted) in [(false, false), (true, false), (false, true)] {
        let mut fx = Fx::new();
        fx.sim.hooks().enable_inactive_store();
        std::sync::Arc::make_mut(&mut fx.sim.sys.hooks.tables)
            .combat
            .monstats2[0]
            .restore = 1;
        let a = fx.a;
        let mon = fx.spawn(UnitType::Monster, 0, a, 6, 6);
        fx.sim.sys.units.get_mut(mon).unwrap().node_index = 3;
        let guid = fx.sim.sys.units.get(mon).unwrap().guid;
        let mut world = WorldFx::new(Ds1s::default()).sim.world;
        *world.monsters.entry(mon) = MonsterData {
            not_counted,
            death_counted,
            ..MonsterData::default()
        };
        fx.sim.sys.hooks.monster_world = Some(Box::new(world));
        let mut game = std::mem::take(&mut fx.game);
        fx.sim.compress_unit(&mut game, mon);
        assert!(fx.sim.restore(&mut game, a));
        fx.game = game;
        let back = fx.game.lists.find_unit(UnitType::Monster, guid).unwrap();
        let data = fx.sim.sys.hooks.monster_data(back).unwrap();
        assert_eq!(
            (data.not_counted, data.death_counted),
            (not_counted, death_counted)
        );
    }
}
