//! The client DRLG copy through the S→C handlers (synthetic level data:
//! level 2 is a row of three 8×8-tile preset rooms of floor tiles).

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::tiles::{cell, ACT_EDGE_TILE, FIXED_LIBRARY};
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, GridPass, LevelDef, LevelIdx, LevelTypes,
    RoomGrids, RoomKind, TileInfo, TileRect,
};
use d2_sim::rng::Seed;

use super::super::drlg::{ClientDrlg, DrlgSource};
use super::super::world::{ActiveRoom, LevelRow, MonsterClass, UnitKey, MONSTER, PLAYER};
use super::support::{hex, Model};

/// Level 2's rooms (tile rectangles), allocation order.
const ROOMS: [(i32, i32); 3] = [(0, 0), (8, 0), (16, 0)];

struct RowTypes;

impl LevelTypes for RowTypes {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        if drlg.level(level).id == 2 {
            for (x, y) in ROOMS {
                let r = drlg.alloc_room(level, RoomKind::Preset, TileRect::new(x, y, 8, 8));
                drlg.room_mut(r).dt1_mask = 1;
                drlg.link_room(r, LinkAt::Tail);
            }
        }
        Ok(())
    }

    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        let r = drlg.room(room).rect;
        let (w, h) = (r.w as usize + 1, r.h as usize + 1);
        let mut g = CellGrid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                g.set(x, y, cell::FLOOR);
            }
        }
        Ok(RoomGrids {
            passes: vec![GridPass {
                cells: g,
                orientation: None,
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        })
    }
}

fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation: o,
        main,
        sub,
        rarity,
        material: FLOOR_MATERIAL,
        subtile_flags: [0; 25],
        roof_height: 0,
        height: 0,
        light_direction: light_direction(o),
    }
}

/// The light direction every base-game
/// DT1 header carries for an orientation.
fn light_direction(orientation: u32) -> u32 {
    match orientation {
        1 | 5 | 8 | 10 => 1,
        2 | 6 | 9 | 11 => 2,
        0 | 3 | 4 | 12..=14 => 3,
        7 => 4,
        15 => 5,
        o => o - 10,
    }
}

/// The DT1 material flags of every synthetic tile: 0x10 (outdoor stone,
/// footstep k 3); no record-flag bit reads it.
const FLOOR_MATERIAL: u16 = 0x10;

fn source() -> DrlgSource {
    let mut data = DrlgData {
        levels: vec![LevelDef::default(); 4],
        ..DrlgData::default()
    };
    for l in &mut data.levels {
        l.warp = [-1; 8];
    }
    data.levels[2].drlg_type = 2;
    data.levels[2].level_type = 1;
    data.levels[2].size = [(24, 8); 3];
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    data.lvltypes = vec![vec![Vec::new(); 32], files];
    let mut t = BTreeMap::new();
    t.insert(b"floor.dt1".to_vec(), vec![tile(0, 0, 0, 1)]);
    t.insert(FIXED_LIBRARY[0].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    // Act I's base library: key (0, 0, 0) at indexes 0 and 2.
    t.insert(
        ACT_EDGE_TILE[0].unwrap().0.to_vec(),
        vec![tile(0, 0, 0, 1), tile(1, 0, 0, 0), tile(0, 0, 0, 3)],
    );
    DrlgSource {
        data: Arc::new(data),
        tiles: Arc::new(d2_server::world_data::Dt1Files(t)),
        types: Arc::new(|| Box::new(RowTypes)),
    }
}

/// A model with the client DRLG source, Levels rows (level 2 in act 0)
/// and `monstats` rows for 0xAC.
fn model() -> Model {
    let mut m = Model::default();
    m.inputs.drlg = Some(source());
    m.inputs.tables.levels = vec![LevelRow::default(); 4];
    m.inputs.tables.monsters = vec![Some(MonsterClass::default()); 155];
    m
}

/// 0x07 / 0x08 for level 2 at tile (x, y).
fn sight(show: bool, x: u16, y: u16) -> Vec<u8> {
    let mut b = vec![if show { 0x07 } else { 0x08 }];
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    b.push(2);
    b
}

/// 0x59 for player 1 at (x, y).
fn assign_player(x: u16, y: u16) -> Vec<u8> {
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72");
    b.resize(0x16, 0);
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    b
}

/// 0xAC for monster `guid` of class 0 at (x, y) with an empty stream.
fn assign_monster(guid: u32, x: u16, y: u16) -> Vec<u8> {
    let mut b = vec![0xAC];
    b.extend_from_slice(&guid.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    // Life 0x80, size 14, stream: mode 1, no optional parts.
    b.extend_from_slice(&[0x80, 0x0E, 0x01]);
    b
}

/// The sub-tile rectangle of level-2 room `k`.
fn rect(k: usize) -> (i32, i32, i32, i32) {
    let (x, y) = ROOMS[k];
    (x * 5, y * 5, 40, 40)
}

fn rects(rooms: &[ActiveRoom]) -> Vec<(i32, i32, i32, i32)> {
    rooms.iter().map(|r| (r.x0, r.y0, r.w, r.h)).collect()
}

/// The act seed of 0x03 below.
const INIT: u32 = 0x1038_88C4;

#[test]
fn load_act_builds_the_client_drlg() {
    let mut m = model();
    m.hex("01 02 04 00 10 00 01 00");
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    let d = m.w.drlg.as_ref().expect("built");
    // The client copy: init seed u32@2, the game's difficulty, client
    // flag, town id 0 (no level generated at creation).
    assert_eq!(
        (d.drlg.act, d.drlg.init_seed, d.drlg.difficulty),
        (0, INIT, 2)
    );
    assert!(d.drlg.on_client);
    assert!(d.drlg.find_level(1).is_none());
    // The same draws as the server's DRLG creation of that seed.
    let mut s = Seed::init_low(INIT);
    assert_eq!(d.drlg.start_seed, s.step());
    assert_eq!(m.w.active_rooms, Some(Vec::new()));
    // Without a DRLG source no client DRLG is built.
    let mut bare = Model::default();
    bare.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert!(bare.w.drlg.is_none() && bare.w.active_rooms.is_none());
}

#[test]
fn rooms_come_in_sight_and_go() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    // 0x07: the room at tile (8, 0) of level 2 is built and listed.
    m.recv(&sight(true, 8, 0));
    let rooms = m.w.active_rooms.clone().unwrap();
    assert_eq!(rects(&rooms), [rect(1)]);
    assert_eq!(rooms[0].level, 2);
    // New active rooms are prepended (act list, newest first).
    m.recv(&sight(true, 0, 0));
    assert_eq!(
        rects(m.w.active_rooms.as_ref().unwrap()),
        [rect(0), rect(1)]
    );
    // The rooms-in-sight record still lists every message.
    assert_eq!(m.w.rooms_in_sight.len(), 2);
    // 0x08 for room 1: its status-1 count drops to 0, but room 0 (still
    // in sight) keeps it at status 2, so its tiles and active room stay.
    m.recv(&sight(false, 8, 0));
    assert_eq!(
        rects(m.w.active_rooms.as_ref().unwrap()),
        [rect(0), rect(1)]
    );
    let st = |m: &Model, k: usize| {
        let d = &m.w.drlg.as_ref().unwrap().drlg;
        let l = d.find_level(2).unwrap();
        d.room(d.level_rooms(l)[k]).status
    };
    assert_eq!((st(&m, 0), st(&m, 1), st(&m, 2)), (1, 2, 3));
    // 0x08 for room 0: every count drops to 0; the client copy frees the
    // tiles of rooms back at status 4 (unset handler 3).
    m.recv(&sight(false, 0, 0));
    assert_eq!(m.w.active_rooms, Some(Vec::new()));
    assert_eq!((st(&m, 0), st(&m, 1), st(&m, 2)), (4, 4, 4));
    // A point in no room of the level: 0x07 reads the null
    // room's count, an access violation that ends the base game (refused and
    // recorded); 0x08 tests the room and does nothing. Both are recorded.
    m.recv(&sight(true, 200, 200));
    assert_eq!(m.rejected().len(), 1);
    assert_eq!(m.rejected()[0].0, 0x07);
    assert_eq!(
        m.rejected()[0].1,
        "invalid client state: 0x07 map reveal requires a room at the destination"
    );
    m.recv(&sight(false, 200, 200));
    assert_eq!(m.rejected().len(), 1);
    assert_eq!(m.w.rooms_in_sight.len(), 6);
    // A new 0x03 frees the act and builds a new one; each handled 0x03
    // counts as an act load.
    let loads = m.w.act_loads;
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert_eq!(m.w.active_rooms, Some(Vec::new()));
    assert_eq!(m.w.act_loads, loads + 1);
}

#[test]
fn units_created_at_a_point_take_the_rooms_seed() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0));
    let room = m.w.active_rooms.as_ref().unwrap()[0].room;
    let seed = |m: &Model| {
        m.w.drlg
            .as_ref()
            .unwrap()
            .drlg
            .active_room(room)
            .unwrap()
            .seed
    };
    // A monster at sub-tile (45, 5) in room 1: the room's seed is stepped
    // once and the unit seed is init_low(lo').
    let mut room_seed = seed(&m);
    let lo = room_seed.step();
    m.recv(&assign_monster(6, 45, 5));
    assert_eq!(seed(&m), room_seed);
    let u = m.unit(UnitKey::new(MONSTER, 6));
    assert_eq!(u.seed, Some((lo, 666)));
    // A player: the room step, then one step of the new unit's seed.
    let lo = room_seed.step();
    m.recv(&assign_player(46, 6));
    let mut unit = Seed::init_low(lo);
    unit.step();
    assert_eq!(
        m.unit(UnitKey::new(PLAYER, 1)).seed,
        Some((unit.lo, unit.hi))
    );
    // At (0, 0): no room, no step.
    m.recv(&assign_monster(7, 0, 0));
    assert_eq!(seed(&m), room_seed);
    // At a point in no active room: fatal 0x13C, nothing created.
    let before = m.rejected().len();
    m.recv(&assign_monster(8, 100, 5));
    assert_eq!(m.rejected().len(), before + 1);
    assert_eq!(m.rejected()[before].1, "fatal assert 0x13C");
    assert!(!m.w.units.contains_key(&UnitKey::new(MONSTER, 8)));
}

#[test]
fn the_players_level_is_its_rooms() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    assert_eq!(m.w.player_level(), None);
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    // 0x15 into room 1 (sub-tile (45, 5)).
    m.hex("15 00 01 00 00 00 2d 00 05 00 01");
    assert_eq!(m.w.player_level(), Some(2));
    let own = m.w.local_room().copied().unwrap();
    assert_eq!((own.x0, own.y0), (40, 0));
    // The cell lookup from the player's room: room 2 is in its adjacency
    // array, so the point resolves there as the act lookup would.
    let other = m.w.room_at(85, 5).unwrap();
    assert_eq!((other.x0, other.y0), (80, 0));
    assert!(m
        .w
        .drlg
        .as_ref()
        .unwrap()
        .adjacency(own.room)
        .contains(&other.room));
    // A point in no active room: none (0x15 there is fatal 0x168).
    assert_eq!(m.w.room_at(5, 5), None);
    m.hex("15 00 01 00 00 00 05 00 05 00 01");
    assert_eq!(m.rejected().last().unwrap().1, "fatal assert 0x168");
    // The client DRLG is the bridge's own: building one directly from
    // the same source and seed gives the same rooms.
    let mut own_copy = ClientDrlg::build(&source(), 0, INIT, 0).unwrap();
    own_copy.set_in_sight(2, 8, 0, None).unwrap();
    own_copy.set_in_sight(2, 16, 0, None).unwrap();
    assert_eq!(own_copy.active_rooms(), m.w.active_rooms.clone().unwrap());
}

#[test]
fn a_correction_needs_a_room_for_the_point() {
    use super::super::check::{check, Checked};
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_monster(6, 45, 5));
    let k = UnitKey::new(MONSTER, 6);
    // Far (> 15) and in no active room (room 0 is not in sight): nothing.
    let inputs = m.inputs.clone();
    assert_eq!(
        check(&mut m.w, &inputs, k, 5, 5, 0, 0, 0).unwrap(),
        Checked::NoRoom
    );
    assert_eq!(m.unit(k).position, Some((45, 5)));
    // Far and in room 2 (adjacent to the monster's room): teleported.
    assert_eq!(
        check(&mut m.w, &inputs, k, 85, 5, 0, 0, 0).unwrap(),
        Checked::Moved
    );
    assert_eq!(m.unit(k).position, Some((85, 5)));
}

#[test]
fn units_live_in_their_rooms_lists() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    let rooms = m.w.active_rooms.clone().unwrap();
    let (r2, r1) = (rooms[0].room, rooms[1].room);
    let mon = UnitKey::new(MONSTER, 6);
    let pl = UnitKey::new(PLAYER, 1);
    // Creation at a point: the head of the creation room's list.
    m.recv(&assign_monster(6, 45, 5));
    m.recv(&assign_player(46, 6));
    assert_eq!(m.w.room_units.list(r1), [pl, mon]);
    assert_eq!(m.w.unit_room(pl).map(|r| r.room), Some(r1));
    // At (0, 0): no room, in no list.
    m.recv(&assign_monster(7, 0, 0));
    assert_eq!(m.w.room_units.room_of(UnitKey::new(MONSTER, 7)), None);
    // 0x15 to room 2: the room recache (leave, head of the new list).
    m.hex("0b 00 01 00 00 00");
    m.hex("15 00 01 00 00 00 55 00 05 00 01");
    assert_eq!(m.w.room_units.list(r1), [mon]);
    assert_eq!(m.w.room_units.list(r2), [pl]);
    assert_eq!(m.w.local_room().map(|r| r.room), Some(r2));
    // 0x0A: the unit free leaves the list.
    m.hex("0a 01 06 00 00 00");
    assert!(m.w.room_units.list(r1).is_empty());
    // Both rooms out of sight: the client room free empties the lists.
    m.recv(&sight(false, 8, 0)).recv(&sight(false, 16, 0));
    assert_eq!(m.w.active_rooms, Some(Vec::new()));
    assert_eq!(m.w.room_units.room_of(pl), None);
    assert_eq!(m.w.local_room(), None);
}

#[test]
fn a_0x15_placement_moves_a_monsters_client_path() {
    use super::super::critter_path::{cell, server_walk, Who};
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    let mon = UnitKey::new(MONSTER, 6);
    m.recv(&assign_monster(6, 45, 5));
    assert_eq!(cell(&m.w, Who::server(mon)), Some((45, 5)));
    let dir = super::super::critter_path::direction(&m.w, Who::server(mon));
    // 0x15 to room 2: the path cell, the precise centre and the position
    // follow; the direction does not.
    m.hex("15 01 06 00 00 00 55 00 05 00 00");
    assert_eq!(cell(&m.w, Who::server(mon)), Some((85, 5)));
    let p = &m.w.objclient.s_paths[&mon];
    assert_eq!(
        (p.precise_x, p.precise_y),
        ((85 << 16) | 0x8000, (5 << 16) | 0x8000)
    );
    assert_eq!(p.point_count, 0);
    assert_eq!(m.w.units[&mon].position, Some((85, 5)));
    assert_eq!(
        super::super::critter_path::direction(&m.w, Who::server(mon)),
        dir
    );
    // A walk now starts from the placed cell.
    let inputs = m.inputs.clone();
    assert!(server_walk(&mut m.w, &inputs, mon, 90, 5));
    assert_eq!(m.w.objclient.s_paths[&mon].x(), 85);
}

#[test]
fn the_draw_order_of_a_list_is_written_back() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0));
    let r1 = m.w.active_rooms.as_ref().unwrap()[0].room;
    m.recv(&assign_monster(6, 45, 5))
        .recv(&assign_monster(7, 46, 5));
    let (a, b) = (UnitKey::new(MONSTER, 6), UnitKey::new(MONSTER, 7));
    assert_eq!(m.w.room_units.list(r1), [b, a]);
    // A permutation is kept; anything else is refused.
    assert!(m.w.room_units.set_order(r1, &[a, b]));
    assert_eq!(m.w.room_units.list(r1), [a, b]);
    assert!(!m.w.room_units.set_order(r1, &[a]));
    assert!(!m.w.room_units.set_order(r1, &[a, UnitKey::new(MONSTER, 9)]));
    assert_eq!(m.w.room_units.list(r1), [a, b]);
    // Later inserts prepend into the kept order.
    m.recv(&assign_monster(8, 47, 5));
    assert_eq!(m.w.room_units.list(r1), [UnitKey::new(MONSTER, 8), a, b]);
}

#[test]
fn the_update_pass_runs_the_build_timer() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0));
    let two = |m: &Model| m.w.drlg.as_ref().unwrap().drlg.status_list(2).to_vec();
    let waiting = two(&m);
    assert_eq!(waiting.len(), 2);
    let active = |m: &Model| {
        m.w.active_rooms
            .as_ref()
            .unwrap()
            .iter()
            .map(|r| r.room)
            .collect::<Vec<_>>()
    };
    assert_eq!(active(&m).len(), 1);
    // Set handler 1 built room 1 (T := 5, B = 1): the fifth update builds
    // the first status-2 room, the tenth the next.
    for _ in 0..4 {
        m.drain();
    }
    assert_eq!(active(&m).len(), 1);
    m.drain();
    assert_eq!(active(&m)[0], waiting[0]);
    for _ in 0..4 {
        m.drain();
    }
    assert_eq!(active(&m).len(), 2);
    m.drain();
    assert_eq!(active(&m)[0], waiting[1]);
    assert_eq!(m.w.drlg_updates, 10);
    // Every room is now active: the units' room lookups see them.
    assert!(m.w.room_at(5, 5).is_some() && m.w.room_at(85, 5).is_some());
    assert!(m.rejected().is_empty());
}

#[test]
fn a_new_active_room_drops_light_caches_reaching_it() {
    use crate::rules::lighting::records::{LightKind, Owner};
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0));
    // Two monsters in room 1 (sub-tiles 40…79): one 5 sub-tiles from
    // room 2, one 35 away.
    m.recv(&assign_monster(6, 75, 5))
        .recv(&assign_monster(7, 45, 5));
    let owner = |guid| Owner {
        unit_type: 1,
        guid,
        client_only: false,
    };
    let mut cached = |guid| {
        let id =
            m.w.lights
                .create(
                    Some(owner(guid)),
                    (0, 0),
                    LightKind::Cached,
                    10,
                    255,
                    0,
                    0,
                    0,
                )
                .unwrap();
        m.w.lights.get_mut(id).unwrap().cache_valid = true;
        id
    };
    let (near, far) = (cached(6), cached(7));
    // 0x07 builds room 2: the callback probes (75 + 10, 5) from room 1,
    // which its adjacency array resolves to room 2.
    m.recv(&sight(true, 16, 0));
    assert!(!m.w.lights.get(near).unwrap().cache_valid);
    assert!(m.w.lights.get(far).unwrap().cache_valid);
    // An owner the model does not hold: fatal 0x591.
    m.w.lights
        .create(Some(owner(99)), (0, 0), LightKind::Cached, 3, 255, 0, 0, 0)
        .unwrap();
    m.recv(&sight(true, 0, 0));
    assert_eq!(m.rejected().last().unwrap().1, "fatal assert 0x591");
}

/// An inner feed whose unit facts are all zero / false (the facts the
/// model does not hold), for the near-room build.
struct ZeroFacts;

impl crate::rules::ViewSource for ZeroFacts {
    fn unit_position(
        &self,
        u: &crate::bridge::ClientUnit,
    ) -> Result<crate::rules::UnitPosition, String> {
        crate::world_view::model_feed::unit_position(u)
    }
    fn unit_offset(
        &self,
        _: &crate::bridge::ClientUnit,
        _: &crate::world_view::UnitPose,
    ) -> Result<(i32, i32), String> {
        Ok((0, 0))
    }
    fn map_tiles(
        &self,
        _: &super::super::world::ClientWorld,
        _: &crate::world_view::ViewAssets,
    ) -> Result<Vec<crate::rules::MapTile>, crate::world_view::ViewError> {
        Ok(Vec::new())
    }
}

impl crate::world_view::ViewFeed for ZeroFacts {
    fn player(
        &self,
        _: &super::super::world::ClientWorld,
    ) -> Result<Option<crate::rules::UnitPosition>, crate::world_view::ViewError> {
        Ok(None)
    }
    fn open_mode(
        &self,
        _: &super::super::world::ClientWorld,
    ) -> Result<crate::rules::OpenMode, crate::world_view::ViewError> {
        Ok(crate::rules::OpenMode::new(0).unwrap())
    }
    fn shake(
        &self,
        _: &super::super::world::ClientWorld,
    ) -> Result<Option<crate::world_view::feed::RunningShake>, crate::world_view::ViewError> {
        Ok(None)
    }
    fn player_seed(
        &mut self,
        _: &super::super::world::ClientWorld,
    ) -> Result<&mut Seed, crate::world_view::ViewError> {
        unreachable!("not read by the near-room build")
    }
    fn blank_screen(
        &self,
        _: &super::super::world::ClientWorld,
    ) -> Result<bool, crate::world_view::ViewError> {
        Ok(true)
    }
    fn unit_facts(
        &self,
        _: &super::super::world::ClientWorld,
        _: &crate::bridge::ClientUnit,
    ) -> Result<crate::rules::draw_order::UnitFacts, crate::world_view::ViewError> {
        Ok(crate::rules::draw_order::UnitFacts::default())
    }
}

#[test]
fn near_rooms_come_from_the_client_drlg() {
    use crate::rules::draw_order::{TileArray, REC_DRAWN, UNIT_DRAWN, UNIT_EX_VISIBLE};
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::ViewFeed;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    m.recv(&assign_monster(6, 47, 9));
    let mut feed = ModelFeed::new(ZeroFacts).with_map();
    // Without Levels rows the level facts (DrawEdges) are unknown.
    assert!(feed.near_rooms(&m.w).is_err());
    let mut rows = vec![LevelRow::default(); 4];
    rows[2].draw_edges = true;
    feed.levels = Some(rows);
    // The map off: the inner feed's answer (none).
    assert!(ModelFeed::new(ZeroFacts)
        .near_rooms(&m.w)
        .unwrap()
        .is_none());
    let cd = m.w.drlg.clone().unwrap();
    let own = m.w.local_room().copied().unwrap();
    let adjacency = cd.adjacency(own.room);
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    // The local player's room's adjacency array, in order.
    assert_eq!(near.rooms.len(), adjacency.len());
    assert_eq!((near.level.id, near.level.draw_edges), (2, true));
    assert_eq!(near.player_tile, (9, 1));
    let rec = cd.drlg.coord_at(own.room, 46, 6).unwrap();
    assert_eq!(near.player_logical, rec.index as i32);
    let k = adjacency.iter().position(|&r| r == own.room).unwrap();
    let room = &near.rooms[k];
    let drlg_room = cd.drlg.room(own.room);
    assert_eq!(
        (room.tiles.x, room.tiles.y, room.tiles.w, room.tiles.h),
        (8, 0, 8, 8)
    );
    assert_eq!(room.subtile_origin, (40, 0));
    let tiles = drlg_room.tiles().unwrap();
    assert_eq!(room.floors.len(), tiles.floors.len());
    assert_eq!(room.walls.len(), tiles.walls.len());
    assert!(!room.floors.is_empty());
    assert_eq!(room.floors[0].flags, tiles.floors[0].flags);
    assert_eq!(
        (room.floors[0].dt1.roof_height, room.floors[0].dt1.height),
        (0, 0)
    );
    // The DT1 light direction crosses the sim seam:
    // a floor's orientation 0 carries direction 3.
    assert_eq!(room.floors[0].dt1.orientation, 0);
    assert_eq!(room.floors[0].dt1.light_direction, 3);
    // The unit list in the client's order (newest first), the model's
    // facts over the inner feed's.
    let keys: Vec<UnitKey> = room.units.iter().map(|u| u.key).collect();
    assert_eq!(keys, [UnitKey::new(MONSTER, 6), UnitKey::new(PLAYER, 1)]);
    assert!(room.units[1].facts.local && !room.units[0].facts.local);
    assert_eq!(room.units[1].facts.mode, 5);
    // The draw writes flags into a record and sorts the list (here by
    // hand): the flags persist into the next frame's build, the order
    // goes back to the client's list.
    room_mut(near, k).floors[0].flags |= REC_DRAWN;
    // The order's unit bits: the monster
    // passed the sight test and was drawn.
    room_mut(near, k).units[0].facts.flag_ex |= UNIT_EX_VISIBLE;
    room_mut(near, k).units[0].facts.flags |= UNIT_DRAWN;
    room_mut(near, k).units.reverse();
    room_mut(near, k).units_sorted = true;
    let orders = feed.take_unit_orders();
    assert_eq!(orders.len(), 1);
    assert!(feed.take_unit_orders().is_empty());
    assert!(m.w.room_units.set_order(orders[0].0, &orders[0].1));
    m.w.frames += 1;
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_ne!(near.rooms[k].floors[0].flags & REC_DRAWN, 0);
    let keys: Vec<UnitKey> = near.rooms[k].units.iter().map(|u| u.key).collect();
    assert_eq!(keys, [UnitKey::new(PLAYER, 1), UnitKey::new(MONSTER, 6)]);
    // The unit bits persist into the next frame's facts; the player has
    // none yet.
    let (p, mo) = (&near.rooms[k].units[0].facts, &near.rooms[k].units[1].facts);
    assert_eq!((p.flag_ex & UNIT_EX_VISIBLE, p.flags & UNIT_DRAWN), (0, 0));
    assert_eq!(
        (mo.flag_ex & UNIT_EX_VISIBLE, mo.flags & UNIT_DRAWN),
        (UNIT_EX_VISIBLE, UNIT_DRAWN)
    );
    // The player's path sub-tile, and Act I's edge floor record: the
    // first entry of key (0, 0, 0) in reverse file order, index 2
    // (open question 2).
    assert_eq!(near.player_subtile, (46, 6));
    let edge = near.edge.unwrap();
    assert_eq!(
        (
            edge.flags,
            edge.dt1.orientation,
            edge.dt1.main,
            edge.dt1.sub
        ),
        (0, 0, 0, 0)
    );
    // Each record's DT1 entry is (path, index in file order).
    let map = feed.map.as_ref().unwrap();
    assert_eq!(
        map.entry(0, TileArray::Edge, 0),
        Some(&(ACT_EDGE_TILE[0].unwrap().0.to_vec(), 2))
    );
    assert_eq!(
        map.entry(k, TileArray::Floor, 0),
        Some(&(b"floor.dt1".to_vec(), 0))
    );
    // The default inner feed refuses the facts the model lacks.
    let mut bare = ModelFeed::<crate::world_view::feed::NoFeed>::default().with_map();
    bare.levels = Some(vec![LevelRow::default(); 4]);
    assert!(bare.near_rooms(&m.w).is_err());
}

#[test]
fn the_preview_runs_the_sight_test_over_the_client_drlg() {
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::preview::Preview;
    use crate::world_view::unit_facts::UnitFactTables;
    use crate::world_view::ViewFeed;
    use d2_sim::drlg::collision::bits;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    m.recv(&assign_monster(6, 46, 20));
    let mut feed = ModelFeed::new(ZeroFacts)
        .with_map()
        .with_preview(Preview::default());
    feed.levels = Some(vec![LevelRow::default(); 4]);
    let mut los_draw = vec![false; 4];
    los_draw[2] = true;
    feed.set_unit_fact_tables(UnitFactTables {
        unflat_dead: vec![Some(false)],
        monster_size: vec![Some(1)],
        los_draw,
        ..UnitFactTables::default()
    });
    // The facts of the monster through both preview paths (the feed's
    // `unit_facts` and the near-room build), which must agree.
    fn hidden(
        feed: &mut ModelFeed<ZeroFacts>,
        w: &super::super::world::ClientWorld,
    ) -> Option<bool> {
        let monster = &w.units[&UnitKey::new(MONSTER, 6)];
        let by_feed = feed.unit_facts(w, monster).unwrap().sight_hidden;
        let near = feed.near_rooms(w).unwrap().unwrap();
        let in_room = near
            .rooms
            .iter()
            .flat_map(|r| &r.units)
            .find(|u| u.key == monster.key)
            .map(|u| u.facts.sight_hidden);
        assert_eq!(in_room, Some(by_feed));
        by_feed
    }
    // An open line: the monster is seen.
    assert_eq!(hidden(&mut feed, &m.w), Some(false));
    // A sight-blocking cell (collision 0x2) on the line hides it in a
    // `LOSDraw` level (the preview answered "seen" for every unit).
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(46, 14).unwrap() |= bits::VISIBLE;
    // The near-room build is per drawn frame.
    m.w.frames += 1;
    assert_eq!(hidden(&mut feed, &m.w), Some(true));
    // `LOSDraw` 0: every unit passes.
    feed.unit_tables.as_mut().unwrap().los_draw[2] = false;
    m.w.frames += 1;
    assert_eq!(hidden(&mut feed, &m.w), Some(false));
}

#[test]
fn missiles_take_the_sight_test() {
    use crate::world_view::missiles::missile_hidden;
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::preview::Preview;
    use crate::world_view::unit_facts::UnitFactTables;
    use crate::world_view::ViewFeed;
    use d2_sim::drlg::collision::bits;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let mut feed = ModelFeed::new(ZeroFacts)
        .with_map()
        .with_preview(Preview::default());
    feed.levels = Some(vec![LevelRow::default(); 4]);
    let mut los_draw = vec![false; 4];
    los_draw[2] = true;
    // Missile row 0 has `Size` 1.
    feed.set_unit_fact_tables(UnitFactTables {
        missile_size: vec![1],
        los_draw,
        ..UnitFactTables::default()
    });
    // A missile on sub-tile (46, 20), 0.5 into it.
    let at = ((46 << 16) + 0x8000, (20 << 16) + 0x8000);
    assert!(!missile_hidden(&m.w, &feed, 0, at), "open line: drawn");
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(46, 14).unwrap() |= bits::VISIBLE;
    assert!(missile_hidden(&m.w, &feed, 0, at), "behind a wall: hidden");
    // `LOSDraw` 0: every missile passes.
    feed.unit_tables.as_mut().unwrap().los_draw[2] = false;
    assert!(!missile_hidden(&m.w, &feed, 0, at));
}

#[test]
fn a_room_that_is_not_active_is_left_out_of_the_view_not_an_error() {
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::ViewFeed;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let mut feed = ModelFeed::new(ZeroFacts).with_map();
    feed.levels = Some(vec![LevelRow::default(); 4]);
    let own = m.w.local_room().copied().unwrap();
    let cd = m.w.drlg.as_mut().unwrap();
    let level = cd.drlg.room(own.room).level;
    let inactive = cd
        .drlg
        .level_rooms(level)
        .into_iter()
        .find(|&r| cd.drlg.room(r).active().is_none())
        .expect("a room out of sight");
    // A level transition: the automap callback list names a room the
    // client DRLG freed again before the frame read it, next to one
    // that is still active.
    cd.drlg.automap_rooms.extend([inactive, own.room]);
    let ids: Vec<_> = feed
        .automap_rooms(&m.w)
        .unwrap()
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert!(
        ids.contains(&own.room) && !ids.contains(&inactive),
        "{ids:?}"
    );
    assert!(feed.near_rooms(&m.w).unwrap().is_some());
}

#[test]
fn a_new_client_drlg_drops_the_tile_draw_state() {
    use crate::rules::draw_order::REC_DRAWN;
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::ViewFeed;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let mut feed = ModelFeed::new(ZeroFacts).with_map();
    let mut rows = vec![LevelRow::default(); 4];
    rows[2].draw_edges = true;
    feed.levels = Some(rows);
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_eq!(near.rooms[0].floors[0].flags & REC_DRAWN, 0);
    room_mut(near, 0).floors[0].flags |= REC_DRAWN;
    // Same DRLG next frame: the flag is kept.
    m.w.frames += 1;
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_ne!(near.rooms[0].floors[0].flags & REC_DRAWN, 0);
    // A new client DRLG (another 0x03; here the same act with a new init
    // seed, so the act's tile libraries stay) whose room slots are the
    // same: nothing carried.
    m.w.drlg.as_mut().unwrap().drlg.init_seed ^= 1;
    m.w.frames += 1;
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_eq!(near.rooms[0].floors[0].flags & REC_DRAWN, 0);
}

fn room_mut(
    near: &mut crate::rules::draw_order::NearRooms,
    k: usize,
) -> &mut crate::rules::draw_order::Room {
    &mut near.rooms[k]
}

#[test]
fn a_freed_room_leaves_a_server_unit_roomless_and_it_sends_0x4b_once() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 0, 0)).recv(&sight(true, 16, 0));
    let (a, b, d) = (
        UnitKey::new(MONSTER, 6),
        UnitKey::new(MONSTER, 7),
        UnitKey::new(MONSTER, 8),
    );
    // A in room 2, B in room 0, D created at (0, 0): in no room.
    m.recv(&assign_monster(6, 85, 5));
    m.recv(&assign_monster(7, 5, 5));
    m.recv(&assign_monster(8, 0, 0));
    let flags = |m: &Model, k| (m.unit(k).room_freed, m.unit(k).flag_ex & 0x20 != 0);
    // A room that stays in sight frees nothing (room 2 is still an
    // active room after its own 0x08).
    m.recv(&sight(false, 16, 0));
    assert_eq!(m.w.active_rooms.as_ref().unwrap().len(), 2);
    assert_eq!(flags(&m, a), (false, false));
    // First path: the last 0x08 takes every room to status 4 and the
    // active rooms are freed.
    m.recv(&sight(false, 0, 0));
    assert_eq!(m.w.active_rooms, Some(Vec::new()));
    // Each unit still linked in a freed room gets 0x800000 and, as a
    // server unit (no flag 0x400000), flags-2 0x20; D, in no room, nothing.
    assert_eq!(flags(&m, a), (true, true));
    assert_eq!(flags(&m, b), (true, true));
    assert_eq!(flags(&m, d), (false, false));
    // The dynamic-path units' client point is zeroed.
    assert_eq!(m.unit(a).position, Some((0, 0)));
    assert_eq!(m.unit(a).precise, Some((0, 0)));
    // The next update pass skips A's update and queue, sends C→S 0x4B
    // (type 1, its GUID) and clears both bits; D sends nothing.
    m.w.units
        .get_mut(&a)
        .unwrap()
        .queue
        .push(hex("6d 06 00 00 00 1a 12 b9 11 80"));
    m.drain();
    assert_eq!(
        m.w.outgoing,
        [
            hex("4b 01 00 00 00 06 00 00 00"),
            hex("4b 01 00 00 00 07 00 00 00")
        ]
    );
    assert_eq!(flags(&m, a), (false, false));
    assert_eq!(m.unit(a).queue.len(), 1, "the queue was not drained");
    m.drain();
    assert_eq!(m.w.outgoing.len(), 2, "once");
    // Second path: the whole client act freed (a new 0x03) frees the
    // rooms still linked the same way.
    m.recv(&sight(true, 0, 0));
    m.recv(&assign_monster(9, 5, 5));
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert_eq!(flags(&m, UnitKey::new(MONSTER, 9)), (true, true));
}

#[test]
fn the_predicted_walk_recaches_the_local_players_room() {
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::ViewFeed;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 0, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let room_of = |m: &Model| m.w.local_room().map(|r| (r.x0, r.y0, r.w, r.h));
    assert_eq!(room_of(&m), Some(rect(1)));
    // The prediction walked the player west into room 0: its room
    // follows (the model position is the prediction's, not written).
    assert!(m.w.recache_local_room(10, 6));
    assert_eq!(room_of(&m), Some(rect(0)));
    assert_eq!(m.w.local().unwrap().cell(), (46, 6));
    let pl = UnitKey::new(PLAYER, 1);
    assert_eq!(m.w.room_units.room_of(pl), m.w.local_room().map(|r| r.room));
    // The near rooms are room 0's adjacency, with the player listed there.
    let mut rows = vec![LevelRow::default(); 4];
    rows[2].draw_edges = false;
    let mut feed = ModelFeed::new(ZeroFacts).with_map();
    feed.levels = Some(rows);
    let own = m.w.local_room().copied().unwrap();
    let cd = m.w.drlg.clone().unwrap();
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_eq!(near.rooms.len(), cd.adjacency(own.room).len());
    assert!(near
        .rooms
        .iter()
        .any(|r| r.tiles.x == 0 && r.units.iter().any(|u| u.key == pl)));
    // Same room again, or a point in no active room: unchanged.
    assert!(!m.w.recache_local_room(11, 6));
    assert!(!m.w.recache_local_room(1000, 1000));
    assert_eq!(room_of(&m), Some(rect(0)));
    // Unlinked from every room (a room free): the act lookup finds it.
    m.w.room_units.leave(pl);
    assert_eq!(m.w.local_room(), None);
    assert!(m.w.recache_local_room(46, 6));
    assert_eq!(room_of(&m), Some(rect(1)));
}

/// 0x9C `action` for item `guid` with a stream head of `mode` at (x, y).
fn item_9c(action: u8, guid: u8, mode: u32, x: u16, y: u16) -> Vec<u8> {
    let mut b = vec![0x9C, action, 0, 0x10, guid, 0, 0, 0];
    let mut bits: Vec<(u32, u32)> = vec![(0x10, 32), (101, 10), (mode, 3)];
    if matches!(mode, 3 | 5) {
        bits.extend([(u32::from(x), 16), (u32::from(y), 16)]);
    } else {
        bits.extend([(0, 4), (0, 4), (0, 4), (0, 3)]);
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    for (v, n) in bits {
        for i in 0..n {
            if pos / 8 == out.len() {
                out.push(0);
            }
            out[pos / 8] |= (((v >> i) & 1) as u8) << (pos % 8);
            pos += 1;
        }
    }
    b.extend(out);
    b[2] = b.len() as u8;
    b
}

#[test]
fn ground_items_join_the_room_of_their_point() {
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let own = m.w.local_room().copied().unwrap();
    let item = UnitKey::new(4, 5);
    // Dropped (mode 3) at the player's feet: at the head of the room's
    // list (the item mode set's insert).
    m.recv(&item_9c(0x03, 5, 3, 47, 6));
    assert!(m.rejected().is_empty(), "{:?}", m.rejected());
    assert_eq!(m.w.room_units.room_of(item), Some(own.room));
    assert_eq!(m.w.room_units.list(own.room)[0], item);
    // Picked up to the cursor (mode 4): it leaves the room.
    m.recv(&item_9c(0x01, 5, 4, 0, 0));
    assert_eq!(m.w.room_units.room_of(item), None);
    // A point in no active room: in no list.
    m.recv(&item_9c(0x03, 6, 3, 2000, 2000));
    assert_eq!(m.w.room_units.room_of(UnitKey::new(4, 6)), None);
}

#[test]
fn each_client_update_steps_the_environment_and_the_overrides() {
    use crate::rules::lighting::environment::{Ambient, PeriodTables};
    let mut m = model();
    let lit = Ambient {
        i: 90,
        r: 10,
        g: 20,
        b: 30,
    };
    m.inputs.tables.levels[2].ambient = lit;
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    assert_eq!(m.w.player_level(), Some(2));
    let periods = PeriodTables::builtin().unwrap();
    let mut env = m.w.environment.expect("the act's record");
    m.w.overrides.den_counter = 0;
    m.w.overrides.start_darkness(2, 1, 2, 2);
    let mut overrides = m.w.overrides;
    for _ in 0..3 {
        m.drain();
        // The same steps by hand, level 2 (base = the
        // leveldefs ambient, which has a colour).
        overrides.update_darkness(lit, 2);
        overrides.update_counters();
        env.update(&periods, 2);
        assert_eq!(m.w.environment, Some(env));
        assert_eq!(m.w.overrides, overrides);
    }
    assert_eq!(m.w.overrides.den_counter, 3);
    assert_eq!(m.w.overrides.darkness.map(|d| d.c), Some(3));
}

#[test]
fn the_fade_player_tile_is_the_predicted_sub_tile() {
    use crate::world_view::model_feed::ModelFeed;
    use crate::world_view::preview::Preview;
    use crate::world_view::ViewFeed;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let mut feed = ModelFeed::new(ZeroFacts).with_preview(Preview::default());
    feed.levels = Some(vec![LevelRow::default(); 4]);
    // The model cell (46, 6) without a prediction.
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_eq!(near.player_tile, (9, 1));
    // The walk prediction at sub-tile (52, 11): the camera and the
    // player draw use it, so the fade centre does too (the path
    // sub-tile / 5).
    m.w.set_local_walk(Some(((52 << 16) | 0x8000, (11 << 16) | 0x8000)), None);
    m.w.frames += 1;
    let near = feed.near_rooms(&m.w).unwrap().unwrap();
    assert_eq!(near.player_tile, (10, 2));
}

#[test]
fn the_den_counter_places_the_den_lights_and_later_rooms_get_them() {
    use crate::bridge::client_missiles::ClientMissileRow;
    use crate::bridge::msg::lighting::lighting_update;
    use crate::rules::lighting::overrides::den_light_points;
    use crate::rules::lighting::records::LightKind;
    use d2_sim::rng::Seed;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    // The fixture's rooms stand in for the Den of Evil (level 8).
    for r in m.w.active_rooms.as_mut().unwrap() {
        r.level = 8;
    }
    m.inputs.tables.missiles = vec![ClientMissileRow::default(); 288];
    m.inputs.tables.missiles[287] = ClientMissileRow {
        light: 10,
        rgb: (255, 255, 255),
        clt_do_func: 23,
        ..ClientMissileRow::default()
    };
    m.inputs.high_light_quality = true;
    let p = m.w.local_player.unwrap();
    // The same draws by hand: per room in list order, the local
    // player's seed, the point test with mask 5.
    let rooms = m.w.active_rooms.clone().unwrap();
    let (lo, hi) = m.w.units[&p].seed.unwrap();
    let mut seed = Seed::new(lo, hi);
    let drlg = &m.w.drlg.as_ref().unwrap().drlg;
    let mut want = Vec::new();
    for r in &rooms {
        want.extend(den_light_points(
            (r.x0, r.y0, r.w, r.h),
            &mut seed,
            |x, y| drlg.collision_at(x, y).map_or(0x27, u32::from) & 5,
        ));
    }
    assert!(!want.is_empty());
    // Counter 29 → 30 on this update: the flag and the lights.
    m.w.overrides.den_counter = 29;
    lighting_update(&mut m.w, &m.inputs).unwrap();
    assert!(m.w.overrides.den_flag);
    let mut got: Vec<(i32, i32)> =
        m.w.objclient
            .set_c
            .values()
            .filter(|u| u.class == 287)
            .map(|u| {
                let (x, y) = u.cell();
                (i32::from(x), i32::from(y))
            })
            .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(
        m.w.units[&p].seed,
        Some((seed.lo, seed.hi)),
        "the player's seed stepped"
    );
    let den_lights =
        m.w.lights
            .iter()
            .filter(|(_, r)| r.owner_type == 3 && r.lookup_flag)
            .inspect(|(_, r)| assert_eq!((r.kind, r.radius), (LightKind::Plain, 80)))
            .count();
    assert_eq!(den_lights, want.len());
    // Once the flag is set, a level-8 room loaded later gets its lights;
    // nothing else on later updates.
    let n = m.w.objclient.set_c.len();
    lighting_update(&mut m.w, &m.inputs).unwrap();
    assert_eq!(m.w.objclient.set_c.len(), n);
    m.w.rooms_loaded.push(rooms[0].room);
    let (lo, hi) = m.w.units[&p].seed.unwrap();
    let mut seed = Seed::new(lo, hi);
    let more = den_light_points(
        (rooms[0].x0, rooms[0].y0, rooms[0].w, rooms[0].h),
        &mut seed,
        |x, y| drlg_point(&m.w, x, y),
    );
    lighting_update(&mut m.w, &m.inputs).unwrap();
    assert_eq!(m.w.objclient.set_c.len(), n + more.len());
}

fn drlg_point(w: &super::super::world::ClientWorld, x: i32, y: i32) -> u32 {
    w.drlg
        .as_ref()
        .unwrap()
        .drlg
        .collision_at(x, y)
        .map_or(0x27, u32::from)
        & 5
}

#[test]
fn a_client_missile_ends_on_a_wall_of_the_client_drlg() {
    use crate::bridge::client_missiles::{create, flag, update, ClientMissileRow, CreateRecord};
    use d2_sim::drlg::collision::bits;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    let rows = vec![ClientMissileRow {
        vel: 16,
        range: 40,
        collide_type: 3,
        clt_do_func: 1,
        ..ClientMissileRow::default()
    }];
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        x: 46,
        y: 8,
        ty: 10,
        ..CreateRecord::default()
    };
    // A wall four sub-tiles south: WALL with the missile barrier 0x4, as
    // tile walls carry (the walk's missile move tests the move mask
    // 0x184, so 0x4 is what stops it).
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(46, 12).unwrap() |= bits::WALL | bits::MISSILE_BARRIER;
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    let mut n = 0;
    while m.w.objclient.set_c.contains_key(&k) {
        let before = m.w.objclient.set_c[&k].position;
        update(&mut m.w, &rows, k, true).unwrap();
        n += 1;
        assert!(n < 40, "ends before its frames run out");
        if !m.w.objclient.set_c.contains_key(&k) {
            // It ended on the update that reached the wall's sub-tile.
            assert!(before.is_some_and(|(_, y)| y < 12));
        }
    }
    assert!(n < 40);
}

#[test]
fn a_fast_client_missile_walks_its_cells_and_stops_before_a_thin_wall() {
    use crate::bridge::client_missiles::{create, flag, update, ClientMissileRow, CreateRecord};
    use d2_sim::drlg::collision::bits;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    // `Vel` 50: (50 << 8) × 75 / 100 = 9600 per step, ((0x400 × 9600) >>
    // 6) × 4096 >> 12 = 153,600 = 2.34 sub-tiles per update due south.
    let rows = vec![ClientMissileRow {
        vel: 50,
        range: 40,
        collide_type: 3,
        clt_do_func: 1,
        ..ClientMissileRow::default()
    }];
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        x: 46,
        y: 8,
        ty: 20,
        ..CreateRecord::default()
    };
    // Free path: the first update crosses (46, 9) and (46, 10), two saved
    // steps, the new-step flag set.
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    assert!(m.w.objclient.missiles[&k].room.is_some());
    update(&mut m.w, &rows, k, true).unwrap();
    let mm = m.w.objclient.missiles[&k];
    assert_eq!(
        (mm.step_count, &mm.steps[..2], mm.new_step),
        (2, &[(46, 9), (46, 10)][..], true)
    );
    assert_eq!(mm.collided & 5, 0);
    // WALL alone (0x1) is outside the move mask 0x184: no stop.
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(46, 12).unwrap() |= bits::WALL;
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    update(&mut m.w, &rows, k, true).unwrap();
    update(&mut m.w, &rows, k, true).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    assert_eq!(m.w.objclient.missiles[&k].pos.1 >> 16, 13, "passed y = 12");
    // A one-cell missile wall at y = 12, which the second step (10.8 →
    // 13.2) would jump: the walk is refused entering it, the missile
    // holds the centre of (46, 11) and the cached collided mask ends it.
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(46, 12).unwrap() |= bits::MISSILE_BARRIER;
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    update(&mut m.w, &rows, k, true).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    update(&mut m.w, &rows, k, true).unwrap();
    assert!(!m.w.objclient.set_c.contains_key(&k), "ended on the wall");
}

#[test]
fn a_still_client_missile_reads_the_wall_under_it() {
    use crate::bridge::client_missiles::{create, flag, update, ClientMissileRow, CreateRecord};
    use d2_sim::drlg::collision::bits;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    // Velocity 0 (no target): the collision word is the size query at
    // its position (`Size` 2: the plus) with all bits.
    let rows = vec![ClientMissileRow {
        range: 40,
        collide_type: 3,
        size: 2,
        clt_do_func: 1,
        ..ClientMissileRow::default()
    }];
    let rec = CreateRecord {
        flags: flag::POSITION,
        x: 46,
        y: 8,
        ..CreateRecord::default()
    };
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    update(&mut m.w, &rows, k, true).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    // A wall on the plus cell east of it.
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(47, 8).unwrap() |= bits::WALL;
    update(&mut m.w, &rows, k, true).unwrap();
    assert!(!m.w.objclient.set_c.contains_key(&k));
}

/// The model with the local player at (46, 6) and monster 7 at (46, 12)
/// (class 0, size 2 from `monstats2`), the monster's cell stamped with
/// the MONSTER bit (the model stamps no unit footprints; the base-game
/// client grid carries them).
fn hit_model() -> Model {
    use d2_sim::drlg::collision::bits;
    let mut m = model();
    m.inputs.tables.monsters[0] = Some(MonsterClass {
        size_x: 2,
        ..MonsterClass::default()
    });
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    m.recv(&assign_monster(7, 46, 12));
    attackable(&mut m);
    let drlg = &mut m.w.drlg.as_mut().unwrap().drlg;
    *drlg.collision_at_mut(46, 12).unwrap() |= bits::MONSTER;
    m
}

/// The model's monsters get unit flag 0x4 (`monstats2` `isAtt`, which
/// these fixture rows leave clear), as every attackable monster has.
fn attackable(m: &mut Model) {
    for u in
        m.w.units
            .values_mut()
            .filter(|u| u.key.unit_type == MONSTER)
    {
        u.flag_4 = true;
    }
}

#[test]
fn a_dying_monster_clears_the_cells_it_shared() {
    use crate::bridge::client_missiles::stamp_unit_footprints;
    use crate::bridge::world::UnitKey;
    let mut m = model();
    m.inputs.tables.monsters[0] = Some(MonsterClass {
        size_x: 2,
        ..MonsterClass::default()
    });
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    m.recv(&assign_monster(7, 46, 12));
    m.recv(&assign_monster(8, 47, 12));
    let monsters = m.inputs.tables.monsters.clone();
    stamp_unit_footprints(&mut m.w, &monsters);
    let stamped = |m: &Model, x: i32, y: i32| {
        let d = &m.w.drlg.as_ref().unwrap().drlg;
        let r = d
            .active_rooms()
            .into_iter()
            .find(|&(_, r)| d.active_room(r).is_some_and(|a| a.subtiles.contains(x, y)))
            .unwrap()
            .0;
        m.w.objclient.unit_grids[&r].get(x, y).unwrap()
    };
    // The two pluses share (46, 12) and (47, 12).
    for (x, y) in [(46, 11), (46, 12), (47, 12), (47, 11)] {
        assert_eq!(stamped(&m, x, y) & 0x100, 0x100, "({x}, {y})");
    }
    // Monster 7 dies: its clear ANDs 0x100 out of its whole plus, the
    // cells monster 8 still stands on included; 8 has not moved, so it
    // stamps nothing again (no reference count).
    m.w.units.get_mut(&UnitKey::new(1, 7)).unwrap().mode = 12;
    stamp_unit_footprints(&mut m.w, &monsters);
    stamp_unit_footprints(&mut m.w, &monsters);
    for (x, y) in [(46, 11), (46, 12), (47, 12)] {
        assert_eq!(stamped(&m, x, y) & 0x100, 0, "({x}, {y})");
    }
    assert_eq!(stamped(&m, 47, 11) & 0x100, 0x100);
    assert_eq!(stamped(&m, 48, 12) & 0x100, 0x100);
}

#[test]
fn a_client_missile_hits_the_monster_on_its_walk() {
    use crate::bridge::client_missiles::{
        create, flag, update_with, ClientMissileRow, CreateRecord, Env,
    };
    use crate::bridge::world::UnitKey;
    let mut m = hit_model();
    let mon = UnitKey::new(1, 7);
    assert!(m.w.units.contains_key(&mon));
    let p = m.w.local_player.unwrap();
    let row = ClientMissileRow {
        vel: 16,
        range: 40,
        collide_type: 3,
        client_col: true,
        collide_kill: true,
        size: 1,
        last_collide: true,
        clt_do_func: 1,
        ..ClientMissileRow::default()
    };
    let rows = vec![row];
    let monsters = m.inputs.tables.monsters.clone();
    let env = Env {
        rows: &rows,
        lights: true,
        skills: None,
        monsters: &monsters,
        model: None,
    };
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(p),
        x: 46,
        y: 8,
        ty: 10,
        ..CreateRecord::default()
    };
    // `CollideKill`: the walk enters (46, 12), where the point query of
    // size 1 meets the monster's plus (size 2: dx + dy ≤ 1): ended.
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    let mut n = 0;
    let mut last = None;
    while m.w.objclient.set_c.contains_key(&k) {
        last = m.w.objclient.set_c[&k].position;
        update_with(&mut m.w, &env, k).unwrap();
        n += 1;
        assert!(n < 10, "hit before its frames run out");
    }
    // The search runs only when the walk's collided mask has a unit bit:
    // the stamped cell (46, 12), entered from (46, 11) (0.75 sub-tile per
    // update), far before its frames or the target (46, 18).
    assert_eq!(last, Some((46, 11)));
    // Without `CollideKill` it flies on, the monster its last-collided
    // unit (not hit again).
    let rows = vec![ClientMissileRow {
        collide_kill: false,
        ..row
    }];
    let env = Env { rows: &rows, ..env };
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    for _ in 0..6 {
        update_with(&mut m.w, &env, k).unwrap();
    }
    assert!(m.w.objclient.set_c.contains_key(&k));
    assert_eq!(m.w.objclient.missiles[&k].last_collided, Some(mon));
    // A dead monster (mode 12) is not a candidate: no hit.
    m.w.units.get_mut(&mon).unwrap().mode = 12;
    let rows = vec![row];
    let env = Env { rows: &rows, ..env };
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    for _ in 0..6 {
        update_with(&mut m.w, &env, k).unwrap();
    }
    assert!(m.w.objclient.set_c.contains_key(&k));
    assert_eq!(m.w.objclient.missiles[&k].last_collided, Some(p));
}

#[test]
fn piercing_and_next_hit_on_a_client_missile() {
    use crate::bridge::client_missiles::{
        create, end_with, flag, ClientMissileRow, CreateRecord, Env,
    };
    use crate::bridge::world::UnitKey;
    let mut m = hit_model();
    let mon = UnitKey::new(1, 7);
    let p = m.w.local_player.unwrap();
    let row = ClientMissileRow {
        range: 40,
        collide_type: 3,
        collide_kill: true,
        pierce: true,
        next_hit: true,
        next_delay: 3,
        light: 4,
        clt_do_func: 1,
        ..ClientMissileRow::default()
    };
    let rows = vec![row];
    let monsters = m.inputs.tables.monsters.clone();
    let env = Env {
        rows: &rows,
        lights: true,
        skills: None,
        monsters: &monsters,
        model: None,
    };
    let rec = CreateRecord {
        flags: flag::POSITION,
        owner: Some(p),
        x: 46,
        y: 8,
        ..CreateRecord::default()
    };
    let k = create(&mut m.w, &rows, &rec, true).unwrap().expect("made");
    // The owner in state 69 `pierce` with one pierce left: r = 2, the
    // missile stays (its light dies; Edge case 2), U gets state 86.
    m.w.units.get_mut(&p).unwrap().states.insert(69);
    m.w.objclient.missiles.get_mut(&k).unwrap().pierce = 1;
    end_with(&mut m.w, &env, k, Some(mon), false).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    assert_eq!(m.w.objclient.missiles[&k].pierce, 0);
    assert_eq!(m.w.objclient.just_hit.get(&mon), Some(&3));
    // `NextHit`: U in state 86 → no hit at all.
    end_with(&mut m.w, &env, k, Some(mon), false).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    // The state runs out after `NextDelay` client updates.
    for _ in 0..3 {
        crate::bridge::client_missiles::tick_just_hit(&mut m.w);
    }
    assert!(m.w.objclient.just_hit.is_empty());
    // No pierce left: r = 3, removed.
    end_with(&mut m.w, &env, k, Some(mon), false).unwrap();
    assert!(!m.w.objclient.set_c.contains_key(&k));
}

/// [`hit_model`] with monsters 5 at (48, 12) and 9 at (46, 14), a hit
/// row (class 1) of function `h` with c1 `c1`, its missile owned by the
/// local player at (46, 12), and skills tables with one blank skill.
fn search_model(
    h: i16,
    c1: i32,
) -> (
    Model,
    crate::bridge::world::UnitKey,
    Vec<crate::bridge::client_missiles::ClientMissileRow>,
    d2_sim::skills::SkillTables,
) {
    use crate::bridge::client_missiles::{create, flag, ClientMissileRow, CreateRecord};
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
    fn blank<T: Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }
    let mut m = hit_model();
    m.recv(&assign_monster(5, 48, 12))
        .recv(&assign_monster(9, 46, 14));
    attackable(&mut m);
    let base = ClientMissileRow {
        range: 40,
        clt_do_func: 1,
        explosion_missile: -1,
        ..ClientMissileRow::default()
    };
    let mut rows = vec![base; 3];
    (rows[1].clt_hit_func, rows[1].clt_hit_sub, rows[1].c_hit_par) =
        (h, [2, -1, -1, -1], [c1, 0, 0]);
    let p = m.w.local_player.unwrap();
    let rec = CreateRecord {
        flags: flag::POSITION,
        owner: Some(p),
        class: 1,
        x: 46,
        y: 12,
        level: 1,
        ..CreateRecord::default()
    };
    let k = create(&mut m.w, &rows, &rec, true).unwrap().unwrap();
    let t = d2_sim::skills::SkillTables {
        skills: vec![blank::<Skills>()],
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![blank::<Missiles>()],
        skills_code: Vec::new(),
        miss_code: Vec::new(),
        level_cap: 99,
        stat_count: 359,
    };
    (m, k, rows, t)
}

#[test]
fn hit_16_chains_to_the_next_guid_in_range() {
    use crate::bridge::client_missiles::{end_with, Env};
    use crate::bridge::world::UnitKey;
    let (mut m, k, rows, t) = search_model(16, 10);
    let monsters = m.inputs.tables.monsters.clone();
    let env = Env {
        rows: &rows,
        lights: true,
        skills: Some(&t),
        monsters: &monsters,
        model: None,
    };
    m.w.objclient.missiles.get_mut(&k).unwrap().d28 = 3;
    let (m7, m9, m5) = (UnitKey::new(1, 7), UnitKey::new(1, 9), UnitKey::new(1, 5));
    end_with(&mut m.w, &env, k, Some(m7), false).unwrap();
    // GUIDs in range {5, 7, 9}, g = 7 → 9: m's class toward it, d28 2,
    // last-collided 7.
    let next: Vec<_> =
        m.w.objclient
            .missiles
            .iter()
            .filter(|(c, _)| **c != k)
            .map(|(_, c)| (c.target_unit, c.d28, c.last_collided, c.target_point))
            .collect();
    assert_eq!(next, vec![(Some(m9), 2, Some(m7), (46, 14))]);
    // From 9 (g = 9): no larger GUID → the smallest, 5.
    let c = *m.w.objclient.missiles.keys().find(|c| **c != k).unwrap();
    m.w.objclient.missiles.get_mut(&c).unwrap().d28 = 2;
    end_with(&mut m.w, &env, c, Some(m9), false).unwrap();
    assert!(m
        .w
        .objclient
        .missiles
        .values()
        .any(|x| x.target_unit == Some(m5) && x.d28 == 1));
    // d28 ≤ 1: no hop.
    let before = m.w.objclient.missiles.len();
    let last =
        *m.w.objclient
            .missiles
            .iter()
            .find(|(_, x)| x.d28 == 1)
            .unwrap()
            .0;
    end_with(&mut m.w, &env, last, Some(m5), false).unwrap();
    assert!(m.w.objclient.missiles.len() < before + 1);
    assert!(!m
        .w
        .objclient
        .missiles
        .values()
        .any(|x| x.d28 == 0 && x.target_unit.is_some()));
}

#[test]
fn hits_25_and_26_bolt_the_units_in_range() {
    use crate::bridge::client_missiles::{end_with, Env};
    // 26: V = (d28, d2C) = monster 7; mask 0 → 0x583 (no hostility,
    // line or state test): a bolt of H1 at each unit in r = 10 but O.
    let (mut m, k, mut rows, t) = search_model(26, 10);
    rows[1].hit_sub1_server = 0;
    let monsters = m.inputs.tables.monsters.clone();
    let env = Env {
        rows: &rows,
        lights: true,
        skills: Some(&t),
        monsters: &monsters,
        model: None,
    };
    let mm = m.w.objclient.missiles.get_mut(&k).unwrap();
    (mm.d28, mm.d2c) = (1, 7);
    end_with(&mut m.w, &env, k, None, true).unwrap();
    let mut bolts: Vec<_> =
        m.w.objclient
            .set_c
            .iter()
            .filter(|(_, u)| u.class == 2)
            .map(|(c, _)| m.w.objclient.missiles[c].target_point)
            .collect();
    bolts.sort();
    assert_eq!(bolts, vec![(46, 12), (46, 14), (48, 12)]);
    // No V: 0, the missile stays.
    let (mut m, k, mut rows, t) = search_model(26, 10);
    rows[1].hit_sub1_server = 0;
    let env = Env {
        rows: &rows,
        lights: true,
        skills: Some(&t),
        monsters: &monsters,
        model: None,
    };
    m.w.objclient.missiles.get_mut(&k).unwrap().d28 = 1;
    m.w.objclient.missiles.get_mut(&k).unwrap().d2c = 99;
    end_with(&mut m.w, &env, k, None, true).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    // 25 searches with the skip flag (monstats `noaura` skipped) and mask
    // 0xA783 (hostile, line of sight, not in state 86); max := c2, here
    // 2: the first two in search order.
    let (mut m, k, mut rows, t) = search_model(25, 10);
    rows[1].c_hit_par[1] = 2;
    let mut monsters = monsters;
    monsters[0] = monsters[0].map(|c| crate::bridge::world::MonsterClass {
        no_aura: false,
        undead: false,
        demon: false,
        ..c
    });
    let env = Env {
        rows: &rows,
        lights: true,
        skills: Some(&t),
        monsters: &monsters,
        model: None,
    };
    end_with(&mut m.w, &env, k, None, true).unwrap();
    let bolts =
        m.w.objclient
            .set_c
            .values()
            .filter(|u| u.class == 2)
            .count();
    assert_eq!(bolts, 2);
    // All `noaura`: none.
    let (mut m, k, mut rows, t) = search_model(25, 10);
    rows[1].c_hit_par[1] = 2;
    monsters[0] = monsters[0].map(|c| crate::bridge::world::MonsterClass { no_aura: true, ..c });
    let env = Env {
        rows: &rows,
        lights: true,
        skills: Some(&t),
        monsters: &monsters,
        model: None,
    };
    end_with(&mut m.w, &env, k, None, true).unwrap();
    assert_eq!(
        m.w.objclient
            .set_c
            .values()
            .filter(|u| u.class == 2)
            .count(),
        0
    );
}

#[test]
fn hit_13_retargets_the_lowest_guid() {
    use crate::bridge::client_missiles::{end_with, Env};
    use crate::bridge::world::UnitKey;
    let (mut m, k, mut rows, t) = search_model(13, 0);
    (rows[1].param, rows[1].lev_range, rows[1].collide_kill) = ([0, 20], 2, true);
    let monsters = m.inputs.tables.monsters.clone();
    let env = Env {
        rows: &rows,
        lights: true,
        skills: Some(&t),
        monsters: &monsters,
        model: None,
    };
    // d28 2, frames left 0: the retarget picks GUID 5 (the lowest in
    // `Param2` 20), frames := `Range` + `LevRange` × (level − 1) = 40,
    // d28 := 5; the hit returns 0: m stays.
    let mm = m.w.objclient.missiles.get_mut(&k).unwrap();
    (mm.d28, mm.current) = (2, 0);
    end_with(&mut m.w, &env, k, None, true).unwrap();
    let mm = m.w.objclient.missiles[&k];
    assert_eq!(
        (mm.target_unit, mm.d28, mm.total, mm.current),
        (Some(UnitKey::new(1, 5)), 5, 40, 40)
    );
    // d28 1 (locked): a hit on another unit than its target → 0; on its
    // target → 1 (ended).
    let mm = m.w.objclient.missiles.get_mut(&k).unwrap();
    mm.d28 = 1;
    end_with(&mut m.w, &env, k, Some(UnitKey::new(1, 7)), false).unwrap();
    assert!(m.w.objclient.set_c.contains_key(&k));
    end_with(&mut m.w, &env, k, Some(UnitKey::new(1, 5)), false).unwrap();
    assert!(!m.w.objclient.set_c.contains_key(&k));
}

#[test]
fn client_missiles_see_the_units_footprints() {
    use crate::bridge::client_missiles::{
        create, flag, stamp_unit_footprints, update_with, ClientMissileRow, CreateRecord, Env,
    };
    use crate::bridge::world::UnitKey;
    let mut m = model();
    m.inputs.tables.monsters[0] = Some(MonsterClass {
        size_x: 2,
        ..MonsterClass::default()
    });
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    m.recv(&assign_monster(7, 46, 12));
    attackable(&mut m);
    let monsters = m.inputs.tables.monsters.clone();
    stamp_unit_footprints(&mut m.w, &monsters);
    // The monster (size 2: pattern 1, the plus) stamps 0x100 on its plus,
    // with the NO_PATH marker on its cell; the local player 0x80. The
    // model's own grid is untouched.
    let stamped = |m: &Model, x: i32, y: i32| {
        let d = &m.w.drlg.as_ref().unwrap().drlg;
        let r = d
            .active_rooms()
            .into_iter()
            .find(|&(_, r)| d.active_room(r).is_some_and(|a| a.subtiles.contains(x, y)))
            .unwrap()
            .0;
        m.w.objclient.unit_grids[&r].get(x, y).unwrap()
    };
    assert_eq!(stamped(&m, 46, 11) & 0x100, 0x100);
    assert_eq!(stamped(&m, 46, 12) & 0x1100, 0x1100);
    assert_eq!(stamped(&m, 46, 6) & 0x80, 0x80);
    assert_eq!(
        m.w.drlg
            .as_ref()
            .unwrap()
            .drlg
            .collision_at(46, 11)
            .unwrap()
            & 0x100,
        0
    );
    // A missile flying south ends on the monster when its walk enters the
    // plus, at (46, 11).
    let p = m.w.local_player.unwrap();
    let rows = vec![ClientMissileRow {
        vel: 16,
        range: 40,
        collide_type: 3,
        client_col: true,
        collide_kill: true,
        size: 1,
        clt_do_func: 1,
        ..ClientMissileRow::default()
    }];
    let env = Env {
        rows: &rows,
        lights: true,
        skills: None,
        monsters: &monsters,
        model: None,
    };
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(p),
        x: 46,
        y: 8,
        ty: 10,
        ..CreateRecord::default()
    };
    let k = create(&mut m.w, &rows, &rec, true).unwrap().unwrap();
    let mut last = None;
    while m.w.objclient.set_c.contains_key(&k) {
        last = m.w.objclient.set_c[&k].position;
        update_with(&mut m.w, &env, k).unwrap();
    }
    assert_eq!(last, Some((46, 10)));
    // A dead monster stamps nothing.
    m.w.units.get_mut(&UnitKey::new(1, 7)).unwrap().mode = 12;
    stamp_unit_footprints(&mut m.w, &monsters);
    assert_eq!(stamped(&m, 46, 11) & 0x100, 0);
}

#[test]
fn a_mode_request_stamps_a_living_units_footprint_again() {
    use crate::bridge::client_missiles::stamp_unit_footprints;
    use crate::bridge::world::UnitKey;
    let mut m = model();
    m.inputs.tables.monsters[0] = Some(MonsterClass {
        size_x: 2,
        ..MonsterClass::default()
    });
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6)).hex("0b 00 01 00 00 00");
    m.recv(&assign_monster(7, 46, 12));
    let monsters = m.inputs.tables.monsters.clone();
    stamp_unit_footprints(&mut m.w, &monsters);
    let room = |m: &Model| {
        let d = &m.w.drlg.as_ref().unwrap().drlg;
        d.active_rooms()
            .into_iter()
            .find(|&(_, r)| {
                d.active_room(r)
                    .is_some_and(|a| a.subtiles.contains(46, 11))
            })
            .unwrap()
            .0
    };
    let r = room(&m);
    let stamped = |m: &Model| m.w.objclient.unit_grids[&r].get(46, 11).unwrap() & 0x100;
    assert_eq!(stamped(&m), 0x100);
    // A neighbour's clear takes the cell;
    // the unit does not move: the cell stays clear.
    *m.w.objclient
        .foot_layer
        .get_mut(&r)
        .unwrap()
        .get_mut(46, 11)
        .unwrap() &= !0x100;
    stamp_unit_footprints(&mut m.w, &monsters);
    assert_eq!(stamped(&m), 0);
    // Its next mode request stamps the footprint again.
    m.w.units
        .get_mut(&UnitKey::new(1, 7))
        .unwrap()
        .mode_requests += 1;
    stamp_unit_footprints(&mut m.w, &monsters);
    assert_eq!(stamped(&m), 0x100);
}

#[test]
fn a_hit_body_the_model_cannot_run_still_ends_the_missile() {
    use crate::bridge::client_missiles::{end_with, ClientMissileRow, Env};
    use crate::bridge::world::UnitKey;
    let (mut m, k, mut rows, _) = search_model(29, 0);
    rows[1] = ClientMissileRow {
        light: 3,
        ..rows[1]
    };
    let monsters = m.inputs.tables.monsters.clone();
    let env = Env {
        rows: &rows,
        lights: true,
        skills: None,
        monsters: &monsters,
        model: None,
    };
    // Hit 29 reads the skills tables, which this model lacks: the error
    // is reported and the missile is gone (no update loop on it).
    assert!(end_with(&mut m.w, &env, k, Some(UnitKey::new(1, 7)), false).is_err());
    assert!(!m.w.objclient.set_c.contains_key(&k));
}

#[test]
fn a_footstep_reads_the_floor_tile_under_the_unit() {
    use crate::audio::triggers::movement::{footstep_material, Floor};
    use crate::audio::unit_feed::floor_under;
    let mut m = model();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&sight(true, 8, 0)).recv(&sight(true, 16, 0));
    m.recv(&assign_player(46, 6));
    let pl = UnitKey::new(PLAYER, 1);
    // Room 1 holds the tile (9, 1): its DT1 flags 0x10 give k 3 whatever
    // the environment's Material 1.
    assert_eq!(floor_under(&m.w, pl), Floor::Tile(FLOOR_MATERIAL));
    assert_eq!(footstep_material(1, floor_under(&m.w, pl)), 3);
    // A point outside the unit's room but in an adjacent one: the near
    // room search finds it.
    m.w.units.get_mut(&pl).unwrap().position = Some((85, 5));
    assert_eq!(floor_under(&m.w, pl), Floor::Tile(FLOOR_MATERIAL));
    // Outside every searched room: not found, the environment default.
    m.w.units.get_mut(&pl).unwrap().position = Some((5, 5));
    assert_eq!(floor_under(&m.w, pl), Floor::NotFound);
    // A unit in no room: no room (k 0).
    m.recv(&assign_monster(7, 0, 0));
    assert_eq!(floor_under(&m.w, UnitKey::new(MONSTER, 7)), Floor::NoRoom);
}
