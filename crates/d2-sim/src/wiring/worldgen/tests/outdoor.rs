//! The act placer runs through [`WorldTypes`]: preset levels get their
//! real preset init on allocation and the placer's directions; an
//! outdoor preset cell builds real preset rooms.

use super::*;
use crate::drlg::outdoor::OutdoorPresets;
use crate::drlg::preset::MapId;
use crate::drlg::{room_flags, RoomKind, TileRect};
use crate::wiring::worldgen::outdoor_presets::OutdoorToPreset;

#[test]
fn act_placer_allocates_preset_levels_and_hands_over_their_directions() {
    let fx = Fx::new(Ds1s::default());
    fx.assert_clean();
    let d = fx.drlg();
    assert_eq!(d.start_seed, START);
    // The preset inits draw on level seeds only: the DRLG seed is the
    // expected one (only the Black Marsh draw advanced it).
    assert_eq!(d.seed, Seed::new(1406222081, 1674353446));
    let mut order: Vec<u32> = d.level_list().into_iter().map(|l| d.level(l).id).collect();
    order.reverse();
    // The placer's rows, then the
    // neighbour-entry walk over 1..17 allocates 8..16.
    assert_eq!(
        order,
        [4, 3, 2, 1, 17, 39, 26, 7, 6, 27, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    let find = |id| d.find_level(id).unwrap();
    // Preset init: roll(Files) on the level seed when
    // Files ≠ 0 (Gate 1, Outer Cloister 3); the town has Files 0.
    assert_eq!(d.level(find(1)).seed, Seed::init_low(START + 1));
    assert_eq!(
        d.level(find(GATE)).seed,
        stepped(Seed::init_low(START + GATE), 1)
    );
    assert_eq!(
        d.level(find(27)).seed,
        stepped(Seed::init_low(START + 27), 1)
    );
    // Outdoor levels: lazy init, no draw.
    assert_eq!(d.level(find(2)).seed, Seed::init_low(START + 2));
    let types = fx.types();
    let p = types.act_presets(0).unwrap();
    // The placer's directions overwrite the
    // preset infos: town 3, Outer Cloister 1 (its roll(3) replaced).
    assert_eq!(p.info(find(1)).unwrap().direction, 3);
    assert_eq!(p.info(find(27)).unwrap().direction, 1);
    // The Gate keeps its own draw (roll(1) = 0).
    assert_eq!(p.info(find(GATE)).unwrap().direction, 0);
    assert!(p.info(find(2)).is_none());
    assert!(types.act_outdoor(0).unwrap().level(find(2)).is_some());
}

#[test]
fn outdoor_preset_cell_builds_real_preset_rooms() {
    let mut fx = Fx::new(Ds1s::default());
    let l = fx.drlg().find_level(2).unwrap();
    let rect = fx.drlg().level(l).rect;
    assert_eq!(rect, TileRect::new(904, 1064, 56, 96));
    let seed0 = fx.drlg().level(l).seed;
    let types = fx.sim.world.types.clone();
    let data = fx.sim.action.sys.hooks.drlg.data.clone();
    {
        let mut t = types.borrow_mut();
        let p = t.parts(0);
        let mut op = OutdoorToPreset {
            presets: p.presets,
            pd: p.pd,
            src: p.src,
            cache: p.cache,
            errors: p.errors,
        };
        op.build_preset_cell(
            fx.drlg_mut(),
            &data,
            l,
            CELL_DEF,
            904,
            1064,
            1,
            room_flags::WAYPOINT,
        )
        .unwrap();
    }
    fx.assert_clean();
    // roll(2) for the default file, then one level-seed step per room
    // of the 16 × 16 map (2 × 2 rooms of 8 × 8).
    assert_eq!(fx.drlg().level(l).seed, stepped(seed0, 5));
    let rooms = fx.drlg().level_rooms(l);
    let mut rects: Vec<TileRect> = rooms.iter().map(|&r| fx.drlg().room(r).rect).collect();
    rects.sort_by_key(|r| (r.y, r.x));
    assert_eq!(
        rects,
        [
            TileRect::new(904, 1064, 8, 8),
            TileRect::new(912, 1064, 8, 8),
            TileRect::new(904, 1072, 8, 8),
            TileRect::new(912, 1072, 8, 8),
        ]
    );
    let t = types.borrow();
    let p = t.act_presets(0).unwrap();
    // The map's file is the cell's, its size lvlprest's.
    let map = p.level_maps(l)[0];
    assert_eq!(map, MapId(0));
    assert_eq!(p.map(map).unwrap().picked_file, 1);
    assert_eq!(p.map(map).unwrap().rect, TileRect::new(904, 1064, 16, 16));
    for &r in &rooms {
        let dr = fx.drlg().room(r);
        assert_eq!(dr.kind, RoomKind::Preset);
        assert_ne!(dr.flags & room_flags::WAYPOINT, 0);
        assert_eq!(p.room(r).unwrap().def, CELL_DEF);
    }
}
