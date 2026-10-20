//! Warp tiles end to end on synthetic tables and DS1 files: the exit
//! cells of a preset level's DS1 give type-5 preset units through
//! [`WorldTypes`], the first population creates the tile units
//! before the preset monsters and only once per DRLG room,
//! a deactivated room's tiles come back as new units from
//! the restore, and a player using a tile arrives in the linked level.

use super::*;
use crate::drlg::{room_flags, WarpDef};
use crate::tick::TickHooks;
use crate::units::lifecycle::AllocRequest;
use crate::units::UnitType;
use crate::wiring::action::warp_tile::CREATED_UNIT_FLAGS;

/// A second preset level (40 × 18 at (8100, 8000)), linked to [`ISLE`]
/// by a tile pair in [`linked`].
const OTHER: u32 = 31;
const OTHER_DEF: u32 = 1105;

/// One exit cell of a DS1 wall layer: tile (x, y), orientation (10 or
/// 11), style (main index), sub index, hidden (bit 31) or a visible
/// wall (bit 0).
struct Exit {
    x: u32,
    y: u32,
    orientation: u32,
    style: u32,
    sub: u32,
    hidden: bool,
}

const fn exit(x: u32, y: u32, orientation: u32, style: u32, sub: u32, hidden: bool) -> Exit {
    Exit {
        x,
        y,
        orientation,
        style,
        sub,
        hidden,
    }
}

/// [`ds1`] with one wall layer holding `exits`.
fn exit_ds1(w: u32, h: u32, exits: &[Exit], monsters: &[(u32, u32, u32)]) -> Ds1Input {
    let mut d = ds1(w, h, monsters);
    let cells = ((w + 1) * (h + 1)) as usize;
    let (mut walls, mut orients) = (vec![0; cells], vec![0; cells]);
    for e in exits {
        let i = (e.y * (w + 1) + e.x) as usize;
        let bit = if e.hidden { cell::HIDDEN } else { cell::WALL };
        walls[i] = e.style << 20 | e.sub << 8 | bit;
        orients[i] = e.orientation;
    }
    d.walls = vec![walls];
    d.orientations = vec![orients];
    d
}

fn warp_row(id: i32, direction: u8, offset_x: i32, offset_y: i32) -> WarpDef {
    WarpDef {
        id,
        direction,
        offset_x,
        offset_y,
        ..WarpDef::default()
    }
}

/// [`ISLE`]'s exit cells (8-tile rooms: room 0 at (8000, 8000), room 1
/// at (8008, 8000), room 2 at (8016, 8000)) and warp slots 2 → `Id` 9
/// ('b', offset (−1, 1)), 3 → `Id` 11 (an 'l' row (0, 0) before an 'r'
/// row (2, −3)).
fn isle_exits() -> Vec<Exit> {
    vec![
        // Room 0, row 1: sub 1, not 0 or 4: no unit.
        exit(5, 1, 10, 3, 1, false),
        // Row 2: main 8: step 1, nothing.
        exit(6, 2, 10, 8, 0, true),
        // Row 4: hidden, 'l' → Id 9 at (5·3 − 1, 5·4 + 1).
        exit(3, 4, 10, 2, 0, true),
        // Row 6: visible, sub 0, orientation 11 → the 'r' row of Id 11.
        exit(6, 6, 11, 3, 0, false),
        // Room 1: visible, sub 4, orientation 10 → the 'l' row of Id 11.
        exit(12, 3, 10, 3, 4, false),
        // Shared edge of rooms 1 and 2: room 1's far column (nothing),
        // room 2's column 0.
        exit(16, 4, 11, 3, 0, true),
        // The level's last column: only room 4's far edge.
        exit(40, 2, 10, 2, 0, true),
    ]
}

fn isle_tables(d: &mut DrlgData) {
    d.levels[ISLE as usize].warp = [-1, -1, 9, 11, -1, -1, -1, -1];
    d.warps = vec![
        warp_row(9, b'b', -1, 1),
        warp_row(11, b'l', 0, 0),
        warp_row(11, b'r', 2, -3),
    ];
}

fn isle_fx() -> Fx {
    let mut m = BTreeMap::new();
    let (x, y) = ISLE_MONSTER;
    m.insert(
        format!("def{ISLE_DEF}.ds1").into_bytes(),
        exit_ds1(40, 18, &isle_exits(), &[(0, x, y)]),
    );
    Fx::with_data(Ds1s(m), |d, _| isle_tables(d), |_| {})
}

/// The DRLG room of `level` whose rect starts at tile (x, y).
fn room_at(fx: &Fx, rooms: &[DrlgRoomId], x: i32, y: i32) -> DrlgRoomId {
    *rooms
        .iter()
        .find(|&&r| {
            let rect = fx.drlg().room(r).rect;
            (rect.x, rect.y) == (x, y)
        })
        .unwrap_or_else(|| panic!("a room at ({x}, {y})"))
}

/// The type-5 preset units of a DRLG room, list order: (class, mode, x,
/// y, flags), room-relative sub-tiles.
fn tile_presets(fx: &Fx, r: DrlgRoomId) -> Vec<(i32, u32, i32, i32, u32)> {
    fx.types()
        .act_presets(0)
        .expect("act 0 presets")
        .room_units(r)
        .iter()
        .filter(|u| u.unit_type == 5)
        .map(|u| (u.class, u.mode, u.x, u.y, u.flags))
        .collect()
}

/// The tile units of an active room: (class, sub-tile position, GUID),
/// by GUID.
fn tiles(fx: &mut Fx, room: RoomId) -> Vec<(u32, (i32, i32), u32)> {
    let units = fx.game.lists.room_units(room);
    let mut v: Vec<_> = units
        .into_iter()
        .filter(|&u| {
            fx.game
                .lists
                .unit(u)
                .is_some_and(|e| e.ty == UnitType::Tile)
        })
        .map(|u| {
            let class = fx.sim.action.sys.units.get(u).unwrap().class;
            let at = fx.sim.action.sys.hooks.path_position(u);
            (class, at, fx.game.lists.unit(u).unwrap().guid)
        })
        .collect();
    v.sort_by_key(|t| t.2);
    v
}

#[test]
fn exit_cells_give_one_tile_preset_each() {
    let mut fx = isle_fx();
    let (_, rooms) = fx.generate(ISLE).unwrap();
    fx.stream(&rooms).unwrap();
    fx.assert_clean();
    let r0 = room_at(&fx, &rooms, 8000, 8000);
    let r1 = room_at(&fx, &rooms, 8008, 8000);
    let r2 = room_at(&fx, &rooms, 8016, 8000);
    // Prepended in fill order (row 4, then row 6): the last added first.
    assert_eq!(
        tile_presets(&fx, r0),
        [
            (11, 0, 5 * 6 + 2, 5 * 6 - 3, 0),
            (9, 0, 5 * 3 - 1, 5 * 4 + 1, 0)
        ]
    );
    assert_eq!(tile_presets(&fx, r1), [(11, 0, 5 * 4, 5 * 3, 0)]);
    assert_eq!(tile_presets(&fx, r2), [(11, 0, 2, 5 * 4 - 3, 0)]);
    // Nowhere else (the level's far column gives none).
    let total: usize = rooms.iter().map(|&r| tile_presets(&fx, r).len()).sum();
    assert_eq!(total, 4);
    // The seam view has them too (unit type 5, class = lvlwarp `Id`).
    let seam = fx
        .with_act(|d, svc| Ok(svc.types.preset_units(d, r1)))
        .unwrap();
    assert!(seam
        .iter()
        .any(|u| (u.unit_type, u.class, u.x, u.y) == (5, 11, 20, 15)));
}

#[test]
fn an_exit_cell_without_a_lvlwarp_row_is_fatal() {
    let mut m = BTreeMap::new();
    m.insert(
        format!("def{ISLE_DEF}.ds1").into_bytes(),
        exit_ds1(40, 18, &[exit(3, 4, 10, 2, 0, true)], &[]),
    );
    // Slot 2 has warp id 9; no row has Id 9.
    let mut fx = Fx::with_data(
        Ds1s(m),
        |d, _| {
            d.levels[ISLE as usize].warp = [-1, -1, 9, -1, -1, -1, -1, -1];
            d.warps = vec![warp_row(11, b'b', 0, 0)];
        },
        |_| {},
    );
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let r0 = room_at(&fx, &rooms, 8000, 8000);
    assert_eq!(fx.stream(&[r0]), Err(DrlgError::NoLvlWarp(9)));
}

#[test]
fn first_population_creates_the_tiles_before_the_preset_monsters_once() {
    let mut fx = isle_fx();
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let r0 = room_at(&fx, &rooms, 8000, 8000);
    let a = fx.stream(&[r0]).unwrap()[0];
    crate::tick::tick(&mut fx.game, &mut fx.sim);
    fx.assert_clean();
    // Both tiles of room 0 at their absolute sub-tiles, list order (head
    // first: the 'r' tile of row 6, then row 4's).
    let t = tiles(&mut fx, a);
    assert_eq!(
        t.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(),
        [(11, (40032, 40027)), (9, (40014, 40021))]
    );
    for u in fx.game.lists.room_units(a) {
        if fx.game.lists.unit(u).unwrap().ty == UnitType::Tile {
            let r = fx.sim.action.sys.units.get(u).unwrap();
            assert_eq!(r.flags & CREATED_UNIT_FLAGS, CREATED_UNIT_FLAGS);
            assert_eq!(r.mode, 0);
        }
    }
    // The preset monster came after both tiles (first walk, then the
    // monster walk).
    let monsters = fx.game.lists.units_of_type(UnitType::Monster);
    assert_eq!(monsters.len(), 1);
    let tile_ids: Vec<UnitId> = fx
        .game
        .lists
        .room_units(a)
        .into_iter()
        .filter(|&u| fx.game.lists.unit(u).unwrap().ty == UnitType::Tile)
        .collect();
    assert!(
        tile_ids.iter().all(|&u| u < monsters[0]),
        "{tile_ids:?} {monsters:?}"
    );
    // The DRLG room handed its list out: flag 0x4000000, and a second
    // preset pass places nothing.
    assert_ne!(fx.drlg().room(r0).flags & room_flags::PRESETS_HANDED_OUT, 0);
    fx.sim.spawn_presets(&mut fx.game, a);
    assert_eq!(tiles(&mut fx, a), t);
    assert_eq!(fx.game.lists.units_of_type(UnitType::Monster).len(), 1);
    fx.assert_clean();
}

/// Deactivates the active `room` as tick step 9 does: every unit through
/// the compress, the room out of the act list, the DRLG's removal.
fn deactivate(fx: &mut Fx, room: RoomId) {
    let mut cur = fx.game.lists.room_unit_first(room);
    while let Some(u) = cur {
        cur = fx.game.lists.room_unit_next(u);
        fx.sim.compress_unit(&mut fx.game, u);
    }
    let _ = fx.game.lists.deactivate_room(room);
    fx.sim.room_deactivated(&mut fx.game, 0, room);
}

/// Tiles before and after a round trip, and the tile classes in room
/// list order at deactivation (the store order).
type RoundTrip = (
    Vec<(u32, (i32, i32), u32)>,
    Vec<(u32, (i32, i32), u32)>,
    Vec<u32>,
);

/// Room 0 populated, deactivated and streamed again; the tiles before
/// and after (the new active room's).
fn round_trip(fx: &mut Fx) -> RoundTrip {
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let r0 = room_at(fx, &rooms, 8000, 8000);
    let a = fx.stream(&[r0]).unwrap()[0];
    crate::tick::tick(&mut fx.game, &mut fx.sim);
    let before = tiles(fx, a);
    let ids: Vec<UnitId> = fx
        .game
        .lists
        .room_units(a)
        .into_iter()
        .filter(|&u| fx.game.lists.unit(u).unwrap().ty == UnitType::Tile)
        .collect();
    let stored: Vec<u32> = ids
        .iter()
        .map(|&u| fx.sim.action.sys.units.get(u).unwrap().class)
        .collect();
    deactivate(fx, a);
    // Stored and freed: the tile units are gone.
    for u in ids {
        assert!(fx.game.lists.unit(u).is_none(), "tile {u:?} freed");
    }
    assert!(fx.drlg().active_room(r0).is_none());
    let b = fx.stream(&[r0]).unwrap()[0];
    crate::tick::tick(&mut fx.game, &mut fx.sim);
    fx.assert_clean();
    (before, tiles(fx, b), stored)
}

#[test]
fn a_reactivated_room_gets_its_tiles_back_from_the_restore() {
    for store in [false, true] {
        let mut fx = isle_fx();
        if store {
            fx.sim.action.sys.hooks.enable_inactive_store();
        }
        let (before, after, stored) = round_trip(&mut fx);
        assert_eq!(before.len(), 2, "store {store}");
        // Same classes and places, new GUIDs.
        let key = |v: &[(u32, (i32, i32), u32)]| {
            let mut k: Vec<_> = v.iter().map(|t| (t.0, t.1)).collect();
            k.sort();
            k
        };
        assert_eq!(key(&before), key(&after), "store {store}");
        let max_before = before.iter().map(|t| t.2).max().unwrap();
        assert!(after.iter().all(|t| t.2 > max_before), "store {store}");
        // Restored from the list head: created in the reverse of the
        // store order, which is the room list order at deactivation.
        let created: Vec<u32> = after.iter().map(|t| t.0).collect();
        let mut want = stored.clone();
        want.reverse();
        assert_eq!(created, want, "store {store}");
        // The preset list was not handed out again: no second monster
        // from it.
        assert!(
            fx.game.lists.units_of_type(UnitType::Monster).len() <= 1,
            "store {store}"
        );
    }
}

/// [`ISLE`] slot 2 ↔ [`OTHER`] slot 0 by tiles (lvlwarp `Id` 9 and 10,
/// both 'b'); the defs scan their exit cells.
fn linked() -> Fx {
    let mut m = BTreeMap::new();
    m.insert(
        format!("def{ISLE_DEF}.ds1").into_bytes(),
        exit_ds1(40, 18, &[exit(3, 4, 10, 2, 0, true)], &[]),
    );
    m.insert(
        format!("def{OTHER_DEF}.ds1").into_bytes(),
        exit_ds1(40, 18, &[exit(4, 5, 10, 0, 0, true)], &[]),
    );
    Fx::with_data(
        Ds1s(m),
        |d, pd| {
            let other = &mut d.levels[OTHER as usize];
            other.drlg_type = 2;
            other.size = [(40, 18); 3];
            other.offset = (8100, 8000);
            other.vis = [ISLE, 0, 0, 0, 0, 0, 0, 0];
            other.warp = [10, -1, -1, -1, -1, -1, -1, -1];
            let isle = &mut d.levels[ISLE as usize];
            isle.vis = [0, 0, OTHER, 0, 0, 0, 0, 0];
            isle.warp = [-1, -1, 9, -1, -1, -1, -1, -1];
            d.warps = vec![warp_row(9, b'b', -1, 1), warp_row(10, b'b', 2, 1)];
            d.warp_exits = vec![(0, 0), (1, 2)];
            let def = &mut pd.defs[OTHER_DEF as usize];
            def.level_id = OTHER;
            pd.defs[ISLE_DEF as usize].scan = 1;
            pd.defs[OTHER_DEF as usize].scan = 1;
        },
        |_| {},
    )
}

#[test]
fn using_a_tile_moves_the_player_to_the_linked_level() {
    let mut fx = linked();
    fx.sim
        .action
        .sys
        .hooks
        .enable_paths()
        .expect("embedded tables");
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let s = room_at(&fx, &rooms, 8000, 8000);
    let a = fx.stream(&[s]).unwrap()[0];
    fx.assert_clean();
    // The link of slot 2 (the rooms-near build) and the
    // room at its far end, built as a client next to the tile would have
    // it (status 1 through the near array).
    let links = fx.drlg().room(s).warp_links.clone();
    assert_eq!(links.len(), 1);
    let t = links[0].target;
    let other = fx.drlg().room(t).level;
    assert_eq!(fx.drlg().level(other).id, OTHER);
    let b = fx.stream(&[t]).unwrap()[0];
    crate::tick::tick(&mut fx.game, &mut fx.sim);
    fx.assert_clean();
    let ta = tiles(&mut fx, a);
    let tb = tiles(&mut fx, b);
    assert_eq!(
        ta.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(),
        [(9, (40014, 40021))]
    );
    let t_rect = fx.drlg().room(t).rect;
    let arrival = (5 * (8100 + 4) + 2, 5 * (8000 + 5) + 1);
    assert_eq!(
        tb.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(),
        [(10, arrival)],
        "{t_rect:?}"
    );
    // A player in ISLE uses the tile: it stands in OTHER at a free point
    // near the arrival tile and walks out by the far side's ExitWalk.
    let req = AllocRequest {
        ty: UnitType::Player,
        class: 0,
        room: Some(a),
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = fx
        .sim
        .action
        .with(&mut fx.game, |g, v| v.allocate(g, &req, 40014, 40021))
        .expect("player");
    let guid = ta[0].2;
    let r = fx.sim.action.warp_tile_message(&mut fx.game, p, guid);
    assert_eq!(r, Some(0), "the warp ran");
    let room = fx
        .game
        .lists
        .unit(p)
        .and_then(|e| e.room())
        .expect("in a room");
    let dr = fx.drlg().drlg_room_of(room).expect("a DRLG room");
    assert_eq!(fx.drlg().level(fx.drlg().room(dr).level).id, OTHER);
    let at = fx.sim.action.sys.hooks.path_position(p);
    assert!(
        (at.0 - arrival.0).abs() <= 3 && (at.1 - arrival.1).abs() <= 3,
        "{at:?} near {arrival:?}"
    );
    fx.assert_clean();
}

#[test]
fn the_tile_message_warps_under_5_walks_to_50_and_refuses_beyond() {
    let mut fx = linked();
    fx.sim
        .action
        .sys
        .hooks
        .enable_paths()
        .expect("embedded tables");
    let (_, rooms) = fx.generate(ISLE).unwrap();
    let s = room_at(&fx, &rooms, 8000, 8000);
    let a = fx.stream(&[s]).unwrap()[0];
    crate::tick::tick(&mut fx.game, &mut fx.sim);
    let t = fx.drlg().room(s).warp_links[0].target;
    fx.stream(&[t]).unwrap();
    crate::tick::tick(&mut fx.game, &mut fx.sim);
    let guid = tiles(&mut fx, a)[0].2;
    let tile = (40014, 40021);
    for (at, want, moved) in [
        // Distance 28 (15 apart): a walk toward the tile, answer 0, no warp yet.
        ((tile.0 - 15, tile.1), Some(0), false),
        // Distance over 50: nothing, answer 1.
        ((tile.0 - 40, tile.1), Some(1), false),
    ] {
        let req = AllocRequest {
            ty: UnitType::Player,
            class: 0,
            room: Some(a),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        let p = fx
            .sim
            .action
            .with(&mut fx.game, |g, v| v.allocate(g, &req, at.0, at.1))
            .expect("player");
        let r = fx.sim.action.warp_tile_message(&mut fx.game, p, guid);
        assert_eq!(r, Some(want.unwrap_or(1)), "from {at:?}");
        let room = fx.game.lists.unit(p).and_then(|e| e.room()).unwrap();
        assert_eq!(room != a, moved, "from {at:?}");
        // Only the walk leaves the pending 0x13 `{5, GUID}` for the host.
        let walks = std::mem::take(&mut fx.sim.action.sys.hooks.tile_walks);
        let walked: Vec<_> = walks.into_iter().collect();
        assert_eq!(walked.is_empty(), want == Some(1), "from {at:?}");
        if want == Some(0) {
            assert_eq!(walked, [(p, guid)]);
        }
    }
    fx.assert_clean();
}
