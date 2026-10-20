//! The Den of Evil maze through [`WorldTypes`]:
//! the cells' DS1 maps are real preset maps, built into real preset
//! rooms that stream with their DS1s.

use super::*;
use crate::drlg::maze::Rotation;
use crate::drlg::{RoomKind, TileRect};

/// The three Den of Evil cells' DS1s (24 × 24 each, no units).
fn den_ds1s() -> Ds1s {
    let mut m = BTreeMap::new();
    for def in [57, 86, 96] {
        m.insert(format!("def{def}.ds1").into_bytes(), ds1(24, 24, &[]));
    }
    Ds1s(m)
}

#[test]
fn den_of_evil_cells_build_real_preset_rooms() {
    let mut fx = Fx::new(den_ds1s());
    let (l, rooms) = fx.generate(DEN).unwrap();
    fx.assert_clean();
    assert_eq!(fx.drlg().level(l).rect, TileRect::new(1500, 1000, 200, 200));
    // Level seed: the vector's 4 cell steps, then per cell in build
    // order D, P, F: the map's roll(1) and one step per 8 × 8 room (9);
    // F also draws its rotation roll(1).
    let ls = Seed::init_low(START + DEN);
    assert_eq!(fx.drlg().level(l).seed, stepped(ls, 4 + 10 + 10 + 11));
    // 27 preset rooms: the three 24 × 24 cells cut into 8 × 8 rooms.
    assert_eq!(rooms.len(), 27);
    let types = fx.types();
    let p = types.act_presets(0).unwrap();
    let maps: Vec<(u32, TileRect, i32)> = p
        .level_maps(l)
        .iter()
        .map(|&m| {
            let m = p.map(m).unwrap();
            (m.def, m.rect, m.picked_file)
        })
        .collect();
    // Head first: F (57, rotated to file 0), P (86), D (96), at the
    // vector's cells after normalization (F (1588, 1088), P (1588, 1112),
    // D (1564, 1088) moved by (−64, −88)).
    assert_eq!(
        maps,
        [
            (57, TileRect::new(1524, 1000, 24, 24), 0),
            (86, TileRect::new(1524, 1024, 24, 24), 0),
            (96, TileRect::new(1500, 1000, 24, 24), 0),
        ]
    );
    for &r in &rooms {
        let dr = fx.drlg().room(r);
        assert_eq!(dr.kind, RoomKind::Preset);
        assert_eq!((dr.rect.w, dr.rect.h), (8, 8));
        let pr = p.room(r).unwrap();
        let m = p.map(pr.map).unwrap();
        assert!(m.rect.contains(dr.rect.x, dr.rect.y));
        assert_eq!(pr.def, m.def);
        assert!(!pr.single);
    }
    assert_eq!(
        types.maze.level_data(fx.drlg(), l).unwrap().rotation,
        [Rotation {
            def: 57,
            n: 1,
            v: 0
        }]
    );
    drop(types);
    // The rooms stream: status 3 loads the DS1 lazily,
    // the grids come from it, the active room joins the act list.
    let first = rooms[0];
    let active = fx.stream(&[first]).unwrap();
    fx.assert_clean();
    assert_eq!(fx.game.lists.active_rooms(0), active);
    assert!(fx.drlg().room(first).tiles().is_some());
}

#[test]
fn den_population_counts_rooms_that_have_not_streamed_yet() {
    use crate::monsters::population::room;
    use crate::wiring::action::MonsterWorld;

    let mut fx = Fx::with_tables(den_ds1s(), |t| {
        t.pop.levels[DEN as usize] = t.pop.levels[ISLE as usize].clone();
    });
    let (_, rooms) = fx.generate(DEN).unwrap();
    fx.sim.create_regions();
    let first = fx.stream(&rooms[..1]).unwrap()[0];
    fx.sim
        .population(&mut fx.game, |cx| room::populate_room(cx, first));
    let counts = MonsterWorld::<TestPending>::den_counts(&fx.sim.world).unwrap();
    assert_eq!((counts.2, counts.3), (1, rooms.len() as u32));
    assert!(counts.0 > 0, "generated Den population contains monsters");
    for u in super::population::monsters(&fx) {
        MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 0);
    }
    let counts = MonsterWorld::<TestPending>::den_counts(&fx.sim.world).unwrap();
    assert_eq!(counts.0, counts.1);
    assert!(
        counts.2 < counts.3,
        "unvisited rooms still prevent completion"
    );
    let remaining = fx.stream(&rooms[1..]).unwrap();
    for r in remaining {
        fx.sim
            .population(&mut fx.game, |cx| room::populate_room(cx, r));
    }
    let counts = MonsterWorld::<TestPending>::den_counts(&fx.sim.world).unwrap();
    assert_eq!(
        (counts.2, counts.3),
        (rooms.len() as u32, rooms.len() as u32)
    );
    // Streaming an already active room is not a new population.
    fx.stream(&rooms[..1]).unwrap();
    assert_eq!(
        MonsterWorld::<TestPending>::den_counts(&fx.sim.world),
        Some(counts)
    );
    fx.assert_clean();
}

#[test]
fn den_ineligible_rooms_do_not_cover_an_unvisited_population_room() {
    use crate::monsters::population::room;
    use crate::wiring::action::MonsterWorld;
    let mut fx = Fx::new(den_ds1s());
    let (_, rooms) = fx.generate(DEN).unwrap();
    fx.drlg_mut().room_mut(rooms[0]).flags |= crate::drlg::room_flags::NO_POPULATION;
    fx.sim.create_regions();
    let active = fx.stream(&rooms[..rooms.len() - 1]).unwrap();
    for r in active {
        fx.sim
            .population(&mut fx.game, |cx| room::populate_room(cx, r));
    }
    let counts = MonsterWorld::<TestPending>::den_counts(&fx.sim.world).unwrap();
    assert_eq!((counts.2, counts.3), (25, 26));
    fx.assert_clean();
}
