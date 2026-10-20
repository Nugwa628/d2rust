//! A preset level (DrlgType 2) generated, streamed and reset through
//! [`WorldTypes`] on the act DRLG.

use super::*;
use crate::drlg::{room_flags, RoomKind, TileRect};

/// The level's rooms by rect: 5 × 3 rooms (the last row 2 tiles high).
fn isle_rects() -> Vec<TileRect> {
    let mut v = Vec::new();
    for (y, h) in [(8000, 8), (8008, 8), (8016, 2)] {
        for x in (8000..8040).step_by(8) {
            v.push(TileRect::new(x, y, 8, h));
        }
    }
    v
}

fn sorted(fx: &Fx, rooms: &[DrlgRoomId]) -> Vec<TileRect> {
    let mut v: Vec<TileRect> = rooms.iter().map(|&r| fx.drlg().room(r).rect).collect();
    v.sort_by_key(|r| (r.y, r.x));
    v
}

#[test]
fn preset_level_generates_through_the_dispatcher() {
    let mut fx = Fx::new(isle_ds1s());
    let (l, rooms) = fx.generate(ISLE).unwrap();
    fx.assert_clean();
    assert_eq!(fx.drlg().level(l).rect, TileRect::new(8000, 8000, 40, 18));
    assert_eq!(sorted(&fx, &rooms), isle_rects());
    // Seed reset {start + id, 666}, the map's roll(1), one step per room.
    let ls = Seed::init_low(START + ISLE);
    assert_eq!(fx.drlg().level(l).seed, stepped(ls, 1 + 15));
    let types = fx.types();
    let p = types.act_presets(0).unwrap();
    let info = *p.info(l).unwrap();
    let map = p.map(info.map.unwrap()).unwrap();
    assert_eq!(map.def, ISLE_DEF);
    assert_eq!(map.picked_file, info.direction);
    // Scan 0, Pops 0: no DS1 load at generation.
    assert_eq!(map.ds1, None);
    for &r in &rooms {
        assert_eq!(fx.drlg().room(r).kind, RoomKind::Preset);
        assert_eq!(fx.drlg().room(r).dt1_mask, 1);
        assert_eq!(fx.drlg().room(r).flags & room_flags::NO_POPULATION, 0);
    }
}

#[test]
fn preset_room_streams_with_its_ds1_and_units() {
    let mut fx = Fx::new(isle_ds1s());
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let a = *rooms
        .iter()
        .find(|&&r| fx.drlg().room(r).rect == TileRect::new(8000, 8000, 8, 8))
        .unwrap();
    let active = fx.stream(&[a]);
    fx.assert_clean();
    let active = active.unwrap();
    assert_eq!(fx.drlg().drlg_room_of(active[0]), Some(a));
    let dr = fx.drlg().room(a);
    assert_ne!(dr.flags & room_flags::PRESET_UNITS_ADDED, 0);
    assert_ne!(dr.flags & room_flags::HAS_ROOM, 0);
    assert!(!dr.tiles().unwrap().floors.is_empty());
    // The DS1's monster moved from the map into the room's list,
    // room-relative (the unit transfer); the DRLG seam sees it.
    let (x, y) = ISLE_MONSTER;
    let units = fx
        .with_act(|d, svc| Ok(svc.types.preset_units(d, a)))
        .unwrap();
    assert_eq!(
        units,
        [crate::drlg::PresetUnit {
            unit_type: 1,
            class: 0,
            x: x as i32,
            y: y as i32,
        }]
    );
    let sub = fx.drlg().active_room(a).unwrap().subtiles;
    assert_eq!(sub, TileRect::new(40000, 40000, 40, 40));
}

#[test]
fn level_reset_frees_the_preset_maps_and_regenerates_the_same_rooms() {
    let mut fx = Fx::new(isle_ds1s());
    let (l, rooms) = fx.generate(ISLE).unwrap();
    let first = sorted(&fx, &rooms);
    let seed = fx.drlg().level(l).seed;
    fx.with_act(|d, svc| {
        d.free_level_rooms(svc.types, l);
        Ok(())
    })
    .unwrap();
    {
        let types = fx.types();
        let p = types.act_presets(0).unwrap();
        assert!(p.level_maps(l).is_empty());
        // keep = 1: the info stays, its map is gone.
        assert_eq!(p.info(l).unwrap().map, None);
        assert_eq!(p.info(l).unwrap().direction, 0);
    }
    let (_, again) = fx
        .with_act(|d, svc| {
            d.generate_level(svc.data, svc.types, l)?;
            Ok((l, d.level_rooms(l)))
        })
        .unwrap();
    fx.assert_clean();
    assert_eq!(sorted(&fx, &again), first);
    assert_eq!(fx.drlg().level(l).seed, seed);
}
