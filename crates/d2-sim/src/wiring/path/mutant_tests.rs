//! Mutation tests of the placement adapters
//! (`wiring::path::place`): each test drives one adapter through its
//! wiring entry point ([`place_unit`], [`floor_drop`], [`warp_player`],
//! the game entry on [`Shared`] handles) or calls the seam method the
//! path code calls, on the action fixture with the path provider on,
//! and asserts the expected outcome: positions, rooms, the S→C bytes by
//! their `sim/server-messages.tsv` layout, unit flags 2, the timer event
//! 14 and its callback, the update queue.

use std::cell::RefCell;
use std::sync::Arc;

use crate::drlg::collision::bits;
use crate::drlg::TileRect;
use crate::path::coords::Point;
use crate::path::place_seams::{CollisionView, LevelView, PlaceError, PlaceHost};
use crate::path::search::ExpField;
use crate::path::warp::WarpOutcome;
use crate::path::CollisionRooms;
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::tests::{Fx, LEVEL};
use crate::wiring::action::WiringError;

use super::place::{floor_drop, place_unit, warp_player, Rooms, Shared};
use super::PathCtx;

/// `velocitypercent` at its creation value 100.
const STAT_VELOCITY: u16 = 67;
/// The town level of act 0, one room C at tiles
/// (0, 16).
const TOWN: u32 = 1;

fn fx_with(rooms: &[(u32, TileRect)]) -> Fx {
    let mut fx = Fx::with_rooms(rooms);
    let h = fx.sim.hooks();
    h.enable_paths().expect("embedded tables");
    Arc::make_mut(&mut h.tables).combat.charstats[0].walkvelocity = 6;
    fx
}

/// Rooms A (tiles 0..8) and B (8..16) of level 2.
fn fx() -> Fx {
    fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
    ])
}

fn player(fx: &mut Fx, x: i32, y: i32) -> UnitId {
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, x, y);
    fx.stats(p, &[(STAT_VELOCITY, 100)]);
    p
}

fn pos(fx: &mut Fx, u: UnitId) -> (i32, i32, Option<RoomId>) {
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(u).unwrap();
    (d.x(), d.y(), d.room)
}

fn guid(fx: &Fx, u: UnitId) -> u32 {
    fx.game.lists.unit(u).unwrap().guid
}

fn set_cell(fx: &mut Fx, room: RoomId, x: i32, y: i32, v: u16) {
    *fx.sim
        .hooks()
        .drlg
        .grid_mut(room)
        .unwrap()
        .get_mut(x, y)
        .unwrap() = v;
}

fn place(
    fx: &mut Fx,
    u: UnitId,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    exact: bool,
    alt: bool,
) -> bool {
    fx.sim.with(&mut fx.game, |g, v| {
        place_unit(PathCtx::of(v, g), u, room, x, y, exact, alt)
    })
}

fn sent(fx: &mut Fx) -> Vec<(UnitId, Vec<u8>)> {
    std::mem::take(&mut fx.sim.hooks().x.sent)
}

/// S→C 0x07 MapReveal (`server-messages.tsv`: x u16 @1, y u16 @3, level
/// u8 @5).
fn map_reveal(x: u16, y: u16, level: u8) -> Vec<u8> {
    let mut m = vec![0x07];
    m.extend_from_slice(&x.to_le_bytes());
    m.extend_from_slice(&y.to_le_bytes());
    m.push(level);
    m
}

/// S→C 0x15 ReassignPlayer (type u8 @1, guid u32 @2, x u16 @6, y u16 @8,
/// flag u8 @10).
fn reassign(ty: u8, guid: u32, x: u16, y: u16, flag: u8) -> Vec<u8> {
    let mut m = vec![0x15, ty];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&x.to_le_bytes());
    m.extend_from_slice(&y.to_le_bytes());
    m.push(flag);
    m
}

fn flags2(fx: &Fx, u: UnitId) -> u32 {
    fx.sim.sys.units.get(u).unwrap().flags2
}

#[test]
fn placing_a_player_searches_a_free_point_and_tells_its_client() {
    // Player (size 2, plus pattern) placed at (50, 12) in room B with a
    // wall at (51, 12): the plus at (50, 12) collides with mask 0x1C09,
    // ring 1 visits the left column first: (49, 11) d 2,
    // then (49, 12) d 1 (kept; no later candidate is nearer).
    let mut fx = fx();
    let (a, b) = (fx.a, fx.b);
    let p = player(&mut fx, 26, 10);
    set_cell(&mut fx, b, 51, 12, bits::WALL);
    fx.tick();
    assert_eq!(fx.game.lists.update_queue(a), Vec::<UnitId>::new());
    assert!(place(&mut fx, p, Some(b), 50, 12, false, false));
    assert_eq!(pos(&mut fx, p), (49, 12, Some(b)));
    assert_eq!(fx.game.lists.unit(p).unwrap().room(), Some(b));
    // Step 6: 0x07 for the destination room (tile x 8, tile y 0, level
    // 2) to the player's client, and nothing else.
    assert_eq!(sent(&mut fx), vec![(p, map_reveal(8, 0, LEVEL as u8))]);
    // Queued for update; flags 2 |= 0x10000 (alt 0); the room-change
    // messages ran (path flag 0x2 cleared).
    assert!(fx.game.lists.update_queue(b).contains(&p));
    assert_eq!(flags2(&fx, p) & 0x10800, 0x10000);
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(p).unwrap();
    assert_eq!(d.flags & 0x2, 0);
    // Event 14 at f + 50 with the placement's callback:
    // cancelling the events of type 14 with that callback removes it.
    let f = fx.game.frame;
    assert!(fx.timers(p).contains(&(14, f + 50)));
    fx.game.timers.cancel_unit_events_with_callback(
        p,
        14,
        Some(crate::units::dispatch::SKILL_COOLDOWN),
    );
    assert!(!fx.timers(p).iter().any(|&(e, _)| e == 14));
    fx.assert_clean();
}

#[test]
fn placing_a_monster_sends_nothing_and_sets_the_alt_flag() {
    // Not a player (step 5): no 0x07, no event 14; alt ≠ 0 sets 0x800.
    // Room null (step 2): the lookup from the unit's own room.
    let mut fx = fx();
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 10);
    assert!(place(&mut fx, m, None, 33, 12, true, true));
    assert_eq!(pos(&mut fx, m), (33, 12, Some(a)));
    assert_eq!(sent(&mut fx), vec![]);
    assert_eq!(flags2(&fx, m) & 0x10800, 0x800);
    assert!(!fx.timers(m).iter().any(|&(e, _)| e == 14));
    fx.assert_clean();
}

#[test]
fn a_null_room_places_from_the_units_own_room() {
    let mut fx = fx();
    let b = fx.b;
    let p = player(&mut fx, 37, 10);
    assert!(place(&mut fx, p, None, 42, 10, true, false));
    assert_eq!(pos(&mut fx, p), (42, 10, Some(b)));
    assert_eq!(sent(&mut fx), vec![(p, map_reveal(8, 0, LEVEL as u8))]);
    fx.assert_clean();
}

#[test]
fn placing_a_unit_without_a_path_is_fatal() {
    // An item outside mode 3 has no path.
    let mut fx = fx();
    let a = fx.a;
    let i = fx.spawn(UnitType::Item, 0, a, 30, 10);
    assert!(!fx.sim.hooks().path_has(i));
    assert!(!place(&mut fx, i, Some(a), 31, 10, true, false));
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().errors),
        vec![WiringError::Place(PlaceError::NoPath)]
    );
    assert_eq!(sent(&mut fx), vec![]);
}

/// The collision queries of `Rooms` and `Shared` on cell (5, 5) of room
/// A holding WALL | NOPLAYER (0x9).
fn queries<C: CollisionView<Room = RoomId>>(c: &C, a: RoomId, b: RoomId) {
    // The rooms' sub-tile rects (tiles × 5).
    assert_eq!(c.room_rect(a), TileRect::new(0, 0, 40, 40));
    assert_eq!(c.room_rect(b), TileRect::new(40, 0, 40, 40));
    // Step 2: the grid value unmasked; no room at the cell → 0x27.
    assert_eq!(c.cell_value(a, 5, 5), 0x9);
    assert_eq!(c.cell_value(a, 6, 5), 0);
    assert_eq!(c.cell_value(a, 500, 500), 0x27);
    // Step 5: the point query masks.
    assert_eq!(c.point_query(a, 5, 5, 0x8), 0x8);
    assert_eq!(c.point_query(a, 5, 5, 0x2), 0);
    assert_eq!(c.point_query(a, 500, 500, 0x2), 0x27);
    // Size 1: the point; size 2: the plus, which reaches (5, 5) from
    // (6, 5).
    assert_eq!(c.size_query(a, 6, 5, 1, 0xFFFF), 0);
    assert_eq!(c.size_query(a, 6, 5, 2, 0xFFFF), 0x9);
    assert_eq!(c.size_query(a, 6, 5, 2, 0x8), 0x8);
    // Step 4: a 3 × 3 box at (6, 6) spans (5..7, 5..7).
    assert_eq!(c.box_query(a, 6, 6, 3, 3, 0xFFFF), 0x9);
    assert_eq!(c.box_query(a, 7, 7, 3, 3, 0xFFFF), 0);
}

#[test]
fn the_collision_views_answer_the_section_4_queries() {
    let mut fx = fx();
    let (a, b) = (fx.a, fx.b);
    set_cell(&mut fx, a, 5, 5, bits::WALL | bits::NOPLAYER);
    queries(&Rooms(&fx.sim.hooks().drlg), a, b);
    fx.sim.with(&mut fx.game, |g, v| {
        let cell = RefCell::new(PathCtx::of(v, g));
        queries(&Shared(&cell), a, b);
    });
}

/// A 256 × 256 walk-back field whose every cell points one step toward
/// the centre (128, 128) (the free-spot search's walk-back directions).
fn field() -> ExpField {
    let mut cells = vec![0u8; 256 * 256];
    for fy in 0..256i32 {
        for fx in 0..256i32 {
            let d = match ((128 - fx).signum(), (128 - fy).signum()) {
                (0, -1) => 0,
                (1, -1) => 1,
                (1, 0) => 2,
                (1, 1) => 3,
                (0, 1) => 4,
                (-1, 1) => 5,
                (-1, 0) => 6,
                (-1, -1) => 7,
                _ => 8,
            };
            cells[(fy * 256 + fx) as usize] = d;
        }
    }
    ExpField::from_cells(256, 256, cells).unwrap()
}

#[test]
fn floor_drop_starts_below_right_and_walks_back() {
    // From (10, 10): start (12, 13) (step 1, a room there). Free with an
    // open walk back → the start itself.
    let mut fx = fx();
    let a = fx.a;
    let f = field();
    let drop = |fx: &mut Fx| {
        floor_drop(
            &fx.sim.hooks().drlg,
            &f,
            Some(a),
            Point::new(10, 10),
            1,
            false,
        )
        .unwrap()
    };
    assert_eq!(drop(&mut fx), (Some(a), Point::new(12, 13)));
    // A wall (field mask 0x801) on the start's walk back, at (11, 12):
    // ring 1 keeps (11, 13), d 1, first in scan order.
    set_cell(&mut fx, a, 11, 12, bits::WALL);
    assert_eq!(drop(&mut fx), (Some(a), Point::new(11, 13)));
    // A pet bit (in 0x3E01, not in the field mask) on the start and no
    // wall: the size query refuses the start; (11, 13) again.
    set_cell(&mut fx, a, 11, 12, 0);
    set_cell(&mut fx, a, 12, 13, bits::PET);
    assert_eq!(drop(&mut fx), (Some(a), Point::new(11, 13)));
}

#[test]
fn game_entry_places_the_player_in_the_town_and_sends_0x07_then_0x15() {
    // The act's start level is its town (act 0: level 1, room C at tiles
    // (0, 16)); the player, allocated without a room, is put in the world
    // at the spawn point: 0x07 for room C, then 0x15 flag 1 at the point.
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
    ]);
    let req = AllocRequest {
        ty: UnitType::Player,
        class: 0,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = fx
        .sim
        .with(&mut fx.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    let r = fx.sim.with(&mut fx.game, |g, v| {
        let cell = RefCell::new(PathCtx::of(v, g));
        let (mut cv, mut host, mut lv) = (Shared(&cell), Shared(&cell), Shared(&cell));
        assert_eq!(lv.act_start_level(0), TOWN);
        crate::path::place::game_entry(&mut cv, &mut host, &mut lv, p, 0)
    });
    assert_eq!(r, Ok(true));
    let (x, y, room) = pos(&mut fx, p);
    // The world add (flag 0) sets the room-changed flag (path flag 0x2).
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(p).unwrap();
    assert_eq!(d.flags & 0x2, 0x2);
    let c = room.expect("placed");
    assert_ne!(c, fx.a);
    assert_ne!(c, fx.b);
    assert_eq!(fx.game.lists.unit(p).unwrap().room(), Some(c));
    let rect = fx.sim.hooks().drlg.subtile_rect(c).unwrap();
    assert_eq!(rect, TileRect::new(0, 80, 40, 40));
    assert!(rect.contains(x, y), "({x}, {y})");
    let g = guid(&fx, p);
    assert_eq!(
        sent(&mut fx),
        vec![
            (p, map_reveal(0, 16, TOWN as u8)),
            (p, reassign(0, g, x as u16, y as u16, 1)),
            (p, vec![0x7E, 0, 0, 0, 0]),
        ]
    );
    fx.assert_clean();
}

/// The hireling callers of the placement wiring
/// (`ActionHooks::hireling_calls`, for the host holding the lists): the
/// game entry of a placed player queues its join follow;
/// a level warp to another
/// act queues the act change and
/// stays on its `Pending` route. Queue off (`None`): nothing recorded.
#[test]
fn game_entry_and_an_act_change_queue_the_hireling_calls() {
    use crate::wiring::action::HirelingCall;
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
    ]);
    let req = AllocRequest {
        ty: UnitType::Player,
        class: 0,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = fx
        .sim
        .with(&mut fx.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    fx.sim.hooks().hireling_calls = Some(Vec::new());
    let placed = fx.sim.with(&mut fx.game, |g, v| {
        super::place::game_entry(PathCtx::of(v, g), p, 0)
    });
    assert!(placed);
    assert_eq!(
        fx.sim.hooks().hireling_calls,
        Some(vec![HirelingCall::JoinFollow(p)])
    );
    // Level 40 is in act 1: the act change.
    let r = fx.sim.with(&mut fx.game, |g, v| {
        super::place::level_warp(PathCtx::of(v, g), p, 40, 0)
    });
    assert_eq!(r, None);
    assert_eq!(
        fx.sim.hooks().hireling_calls,
        Some(vec![
            HirelingCall::JoinFollow(p),
            HirelingCall::ActChange(p)
        ])
    );
    fx.sim.hooks().hireling_calls = None;
    let r = fx.sim.with(&mut fx.game, |g, v| {
        super::place::level_warp(PathCtx::of(v, g), p, 40, 0)
    });
    assert_eq!(r, None);
    assert_eq!(fx.sim.hooks().hireling_calls, None);
}

#[test]
fn every_acts_start_level_is_its_town() {
    // Act +0x08: the town level ids 1, 40, 75, 103, 109.
    let mut fx = fx();
    fx.sim.with(&mut fx.game, |g, v| {
        let cell = RefCell::new(PathCtx::of(v, g));
        let lv = Shared(&cell);
        let got: Vec<u32> = (0..5).map(|act| lv.act_start_level(act)).collect();
        assert_eq!(got, [1, 40, 75, 103, 109]);
    });
}

#[test]
fn a_warp_tile_without_a_destination_does_nothing() {
    // The wired level view has no lvlwarp destination:
    // step 1 ends the warp; the player, its room and the clients are
    // unchanged.
    let mut fx = fx();
    let a = fx.a;
    let p = player(&mut fx, 26, 10);
    let r = fx
        .sim
        .with(&mut fx.game, |g, v| warp_player(PathCtx::of(v, g), p, a, 0));
    assert_eq!(r, Some(WarpOutcome::NoDestination));
    assert_eq!(pos(&mut fx, p), (26, 10, Some(a)));
    assert_eq!(sent(&mut fx), vec![]);
    fx.assert_clean();
}

#[test]
fn the_warp_walk_out_requests_walk_mode_to_the_target() {
    // The walk-out is the point-form mode request 2:
    // the player starts walking to the target.
    let mut fx = fx();
    let p = player(&mut fx, 26, 10);
    fx.sim.with(&mut fx.game, |g, v| {
        let cell = RefCell::new(PathCtx::of(v, g));
        Shared(&cell).request_walk(p, 31, 10);
    });
    assert_eq!(fx.sim.sys.units.get(p).unwrap().mode, 2);
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(p).unwrap();
    assert_eq!((d.target_x, d.target_y), (31, 10));
    fx.assert_clean();
}

fn cell(fx: &mut Fx, x: i32, y: i32) -> u16 {
    let r = fx.a;
    crate::path::collision::point_value(&fx.sim.hooks().drlg, Some(r), x, y, 0xFFFF)
}

fn remove(fx: &mut Fx, u: UnitId) {
    fx.sim.with(&mut fx.game, |g, v| v.remove(g, u));
}

#[test]
fn a_missile_path_takes_its_set_up_and_a_still_missile_stays_put() {
    // Missile 0 with `Vel` 0, `Accel` 3, `MaxVel` 2, `Collision` 1 from
    // a monster at (10, 10) toward (20, 10), with the path provider on:
    // the path holds the target point, the move-test mask of collide
    // type 3 (0x184), acceleration 3 and maximum
    // velocity 2 << 8; footprint mask 0x40 restamped by size (missile
    // size 1: the cell, no marker). Velocity 0: no step,
    // the frames count down and the missile stays put.
    use crate::missiles::{create_missile, param_flags, MissileParams};
    let mut fx = fx();
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        t.missiles[0].vel = 0;
        t.missiles[0].accel = 3;
        t.missiles[0].maxvel = 2;
        t.missiles[0].collision = 1;
    }
    let a = fx.a;
    let owner = fx.spawn(UnitType::Monster, 0, a, 10, 10);
    let target = player(&mut fx, 15, 10);
    let fire = |fx: &mut Fx, target: Option<UnitId>| {
        let p = MissileParams {
            owner: Some(owner),
            origin: Some(owner),
            target,
            class: 0,
            flags: param_flags::TARGET_ABSOLUTE,
            target_x: 20,
            target_y: 10,
            ..MissileParams::default()
        };
        fx.sim
            .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
            .unwrap()
            .expect("created")
    };
    let m = fire(&mut fx, None);
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(m).unwrap();
    assert_eq!((d.target_x, d.target_y, d.target_unit), (20, 10, None));
    assert_eq!((d.foot_mask, d.move_mask), (0x40, 0x184));
    assert_eq!((d.acceleration, d.max_velocity), (3, 0x200));
    assert_eq!(d.velocity, 0);
    assert_eq!(fx.sim.hooks().path_velocity(m), 0);
    assert_ne!(cell(&mut fx, 10, 10) & 0x40, 0);
    assert_eq!(cell(&mut fx, 11, 10) & 0x40, 0);
    let total = fx.sim.hooks().missile_store().get(m).unwrap().total;
    for _ in 0..3 {
        fx.frame();
    }
    let rec = fx.sim.hooks().missile_store().get(m).expect("alive");
    assert_eq!(rec.current, total - 3);
    assert_eq!(pos(&mut fx, m), (10, 10, Some(a)));
    // A target unit (velocity 0: kept, step 8): the path's target unit
    // with its type and GUID.
    let n = fire(&mut fx, Some(target));
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(n).unwrap();
    let t = d.target_unit.expect("target unit");
    assert_eq!(
        (t.unit, t.ty, t.guid),
        (target, UnitType::Player, guid(&fx, target))
    );
    // Removal clears the footprint (size, mask 0x40); the
    // second missile on the same cell keeps its own.
    remove(&mut fx, n);
    remove(&mut fx, m);
    assert_eq!(cell(&mut fx, 10, 10) & 0x40, 0);
    fx.assert_clean();
}

#[test]
fn floor_items_stamp_by_size_and_removal_clears_each_kind() {
    // Item in mode 3: size 1, mask 0x200 (the cell). A monster of
    // monstats2 `SizeX` 0 has pattern 0, which stamps nothing; one
    // of `SizeX` 2 has pattern 1: the plus with mask 0x100 and NO_PATH on
    // its centre. Removal clears the footprint of types 0–3 only
    // (the monster's goes, the item's
    // stays). (A tile, size 0, stamps nothing.)
    let mut fx = fx();
    let a = fx.a;
    let req = AllocRequest {
        ty: UnitType::Item,
        class: 0,
        room: Some(a),
        add: true,
        fixed_guid: None,
        mode: 3,
        allied: false,
    };
    let i = fx
        .sim
        .with(&mut fx.game, |g, v| v.allocate(g, &req, 5, 5))
        .unwrap();
    let before = cell(&mut fx, 20, 20);
    let m0 = fx.spawn(UnitType::Monster, 0, a, 20, 20);
    assert_eq!(cell(&mut fx, 20, 20), before);
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        let ex = usize::from(t.combat.monstats[0].monstatsex);
        t.combat.monstats2[ex].sizex = 2;
    }
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 30);
    assert_eq!(cell(&mut fx, 5, 5), 0x200);
    assert_eq!(cell(&mut fx, 6, 5), 0);
    assert_eq!(cell(&mut fx, 30, 30), 0x100 | bits::NO_PATH);
    for (x, y) in [(29, 30), (31, 30), (30, 29), (30, 31)] {
        assert_eq!(cell(&mut fx, x, y), 0x100, "({x}, {y})");
    }
    assert_eq!(cell(&mut fx, 29, 29), 0);
    for u in [i, m0, m] {
        remove(&mut fx, u);
    }
    for (x, y) in [(30, 30), (29, 30), (31, 30), (30, 29), (30, 31)] {
        assert_eq!(cell(&mut fx, x, y), 0, "({x}, {y})");
    }
    assert_eq!(cell(&mut fx, 5, 5), 0x200);
    fx.assert_clean();
}

#[test]
fn a_walk_request_puts_the_walkers_footprint_back() {
    // The compute removes the unit's own footprint and puts it
    // back at its position after the path function:
    // right after the request, before any step, the
    let mut fx = fx();
    let p = player(&mut fx, 10, 10);
    let foot = fx
        .sim
        .hooks()
        .paths
        .as_ref()
        .unwrap()
        .dynamic(p)
        .unwrap()
        .foot_mask;
    assert_ne!(foot, 0);
    assert_eq!(cell(&mut fx, 10, 10) & foot, foot);
    let r = fx
        .sim
        .with(&mut fx.game, |g, v| PathCtx::of(v, g).walk_to(p, 2, 20, 10));
    assert!(r.is_some());
    let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(p).unwrap();
    assert_ne!(d.point_count, 0, "a path was found");
    assert_eq!((d.x(), d.y()), (10, 10));
    assert_eq!(cell(&mut fx, 10, 10) & foot, foot);
    fx.assert_clean();
}

#[test]
fn a_monsters_velocity_comes_from_its_monstats_row() {
    // Base = monstats `Velocity` × 256, p = stat 67 (100): walk mode 2 →
    // 5 · 256. A monster whose `npc` bit is set has the modifier in modes
    // 2 and 15 only: mode 8 (column b set for classes < 410) keeps the
    // velocity; without `npc` mode 8 sets it.
    use crate::path::walk::request::set_mode_and_velocity;
    let mut fx = fx();
    let a = fx.a;
    Arc::make_mut(&mut fx.sim.hooks().tables).combat.monstats[0].velocity = 5;
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 30);
    fx.stats(m, &[(STAT_VELOCITY, 100)]);
    let set = |fx: &mut Fx, mode: u32| {
        let t = fx.sim.hooks().paths.as_ref().unwrap().tables.clone();
        let mut d = fx
            .sim
            .hooks()
            .paths
            .as_ref()
            .unwrap()
            .dynamic(m)
            .unwrap()
            .clone();
        d.velocity = 7;
        fx.sim.with(&mut fx.game, |g, v| {
            set_mode_and_velocity(&t, &mut PathCtx::of(v, g), m, &mut d, mode)
        });
        d.velocity
    };
    assert_eq!(set(&mut fx, 2), 5 * 256);
    assert_eq!(set(&mut fx, 8), 5 * 256);
    Arc::make_mut(&mut fx.sim.hooks().tables).combat.monstats[0].npc = true;
    assert_eq!(set(&mut fx, 8), 7);
    assert_eq!(set(&mut fx, 15), 5 * 256);
}

#[test]
fn a_fatal_type_set_in_a_walk_request_is_reported() {
    // Set type asserts on a previous type of 8 (server
    // knockback): the request's type reset (type 7) hits it.
    // The wiring reports the fatal walk error and the request has no
    // outcome; nothing moves.
    use crate::path::walk::WalkError;
    use crate::path::PathError;
    let mut fx = fx();
    let p = player(&mut fx, 10, 10);
    fx.sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(p)
        .unwrap()
        .prev_path_type = 8;
    let r = fx
        .sim
        .with(&mut fx.game, |g, v| PathCtx::of(v, g).walk_to(p, 2, 20, 10));
    assert_eq!(r, None);
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().errors),
        vec![WiringError::Walk(WalkError::Path(PathError::PathType(7)))]
    );
    assert_eq!(pos(&mut fx, p), (10, 10, Some(fx.a)));
    fx.assert_clean();
}

#[test]
fn placing_a_unit_in_its_own_room_queues_it_for_update() {
    // Step 5: queue for update even without a room change
    // (the room list does not queue the unit then).
    let mut fx = fx();
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 10);
    fx.game.lists.clear_update_queue(a).unwrap();
    assert_eq!(fx.game.lists.update_queue(a), Vec::<UnitId>::new());
    assert!(place(&mut fx, m, Some(a), 33, 12, true, false));
    assert_eq!(pos(&mut fx, m), (33, 12, Some(a)));
    assert_eq!(fx.game.lists.update_queue(a), vec![m]);
    fx.assert_clean();
}

#[test]
fn placing_a_player_writes_its_position_history() {
    // Step 7: entry[index] := the placed point, index + 1.
    // A monster's placement writes nothing (step 5 has no history).
    let mut fx = fx();
    let a = fx.a;
    let p = player(&mut fx, 26, 10);
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 10);
    let before = fx
        .sim
        .hooks()
        .paths
        .as_ref()
        .unwrap()
        .history
        .get(&p)
        .cloned();
    let next = before.as_ref().map_or(0, |h| h.next);
    assert!(place(&mut fx, p, Some(a), 33, 12, true, false));
    assert!(place(&mut fx, m, Some(a), 35, 12, true, false));
    let paths = fx.sim.hooks().paths.as_ref().unwrap();
    let h = paths.history.get(&p).expect("history written");
    assert_eq!(h.newest(), (33, 12));
    assert_eq!(h.entries[usize::from(next)], (33, 12));
    assert_eq!(h.next, (next + 1) % 20);
    assert!(!paths.history.contains_key(&m));
    sent(&mut fx);
    fx.assert_clean();
}

#[test]
fn the_wired_game_entry_reports_placed_or_a_missing_spawn_room() {
    // `wiring::path::place::game_entry`: true when the player is placed
    // in the town; with no room of the town level the spawn lookup has
    // no room (fatal assert): logged as `WiringError::Place`, false.
    let alloc = |fx: &mut Fx| {
        let req = AllocRequest {
            ty: UnitType::Player,
            class: 0,
            room: None,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        fx.sim
            .with(&mut fx.game, |g, v| v.allocate(g, &req, 0, 0))
            .unwrap()
    };
    let mut town = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
    ]);
    let fx = &mut town;
    let p = alloc(fx);
    let r = fx.sim.with(&mut fx.game, |g, v| {
        super::place::game_entry(PathCtx::of(v, g), p, 0)
    });
    assert!(r);
    assert!(fx.game.lists.unit(p).unwrap().room().is_some());
    sent(fx);
    fx.assert_clean();

    let mut fx = self::fx();
    let p = alloc(&mut fx);
    let r = fx.sim.with(&mut fx.game, |g, v| {
        super::place::game_entry(PathCtx::of(v, g), p, 0)
    });
    assert!(!r);
    // The level view's lookup logs its own DRLG error first.
    let errors = std::mem::take(&mut fx.sim.hooks().errors);
    assert_eq!(
        errors.last(),
        Some(&WiringError::Place(PlaceError::NoSpawnRoom)),
        "{errors:?}"
    );
    assert_eq!(fx.game.lists.unit(p).unwrap().room(), None);
    assert_eq!(sent(&mut fx), vec![]);
    fx.assert_clean();
}

#[test]
fn the_wired_game_entry_seeds_the_by_time_t_of_a_fresh_act_with_0() {
    // ByTime's t is read live from the act's
    // environment record, 0 before the act's first advance; the entry
    // seeds it, so by-time stats worn at the join evaluate with t = 0.
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
    ]);
    let req = AllocRequest {
        ty: UnitType::Player,
        class: 0,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = fx
        .sim
        .with(&mut fx.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    assert_eq!(fx.sim.hooks().act_times.get(&p), None);
    let r = fx.sim.with(&mut fx.game, |g, v| {
        super::place::game_entry(PathCtx::of(v, g), p, 0)
    });
    assert!(r);
    assert_eq!(fx.sim.hooks().act_times.get(&p), Some(&0));
    sent(&mut fx);
    fx.assert_clean();
}

#[test]
fn a_moving_missile_reads_the_collision_word_its_step_cached() {
    // With velocity ≠ 0 the collision word is
    // the one the step cached in the path, the move test masked
    // by the mode's mask (0x184, no wall bit). A missile flying over
    // wall cells therefore keeps flying: the cached word has no bit 0.
    // (Recomputed with all bits at the position it would read the wall
    // and remove the missile without a hit.)
    use crate::missiles::{create_missile, param_flags, MissileParams};
    let mut fx = fx();
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        t.missiles[0].vel = 4;
        t.missiles[0].maxvel = 4;
        t.missiles[0].collision = 1;
    }
    let a = fx.a;
    for x in 11..=30 {
        set_cell(&mut fx, a, x, 10, bits::WALL);
    }
    let owner = fx.spawn(UnitType::Monster, 0, a, 10, 10);
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: 30,
        target_y: 10,
        ..MissileParams::default()
    };
    let m = fx
        .sim
        .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
        .unwrap()
        .expect("created");
    let mut over_wall = 0;
    for _ in 0..4 {
        fx.frame();
        let d = fx.sim.hooks().paths.as_ref().unwrap().dynamic(m).cloned();
        let d = d.expect("still flying");
        assert_ne!(d.velocity, 0);
        assert_eq!(d.collided_mask & 0x5, 0);
        if d.x() > 10 {
            over_wall += 1;
            assert_ne!(cell(&mut fx, d.x(), d.y()) & bits::WALL, 0);
        }
    }
    assert!(over_wall > 0, "the missile left its start cell");
    assert!(fx.sim.hooks().missile_store().get(m).is_some());
    fx.assert_clean();
}

/// The game entry writes the entry act into the player's unit record
/// (0-based: act 3 for an Act IV start; an allocation without a room gives 0).
#[test]
fn game_entry_sets_the_players_act() {
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
    ]);
    let req = AllocRequest {
        ty: UnitType::Player,
        class: 0,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = fx
        .sim
        .with(&mut fx.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    fx.sim
        .with(&mut fx.game, |_, v| v.units.get_mut(p).unwrap().act = 3);
    assert!(fx.sim.with(&mut fx.game, |g, v| {
        super::place::game_entry(PathCtx::of(v, g), p, 0)
    }));
    let act = fx
        .sim
        .with(&mut fx.game, |_, v| v.units.get(p).unwrap().act);
    assert_eq!(act, 0);
}

/// `SUNIT_Add`: an object gets its footprint
/// when `HasCollision[mode]` is set, with the class's mask, and its
/// removal clears it. Found by `a4-warp-plains-ama`: the object footprints
/// were never stamped at the add, so populated objects were placed on
/// each other's cells.
#[test]
fn an_object_with_collision_stamps_its_footprint_at_the_add_and_removal_clears_it() {
    use crate::world::objects::ObjectTables;
    use d2_data::tables::Objects;
    let mut fx = fx();
    let mut o: Objects = crate::skills::fake::blank();
    (o.sizex, o.sizey, o.hascollision0) = (2, 2, 1);
    let mut quiet: Objects = crate::skills::fake::blank();
    (quiet.sizex, quiet.sizey) = (2, 2);
    fx.sim.create_objects(Arc::new(ObjectTables {
        objects: vec![o, quiet],
        shrines: Vec::new(),
        levels: Vec::new(),
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    }));
    let a = fx.a;
    let o0 = fx
        .sim
        .with(&mut fx.game, |g, v| v.create_object(g, a, 0, 20, 20, 0))
        .expect("allocated");
    // The object mask 0x400 over its 2 × 2 box.
    assert_eq!(cell(&mut fx, 20, 20) & 0x400, 0x400);
    // A class with `HasCollision0` 0 stamps nothing.
    fx.sim
        .with(&mut fx.game, |g, v| v.create_object(g, a, 1, 30, 20, 0))
        .expect("allocated");
    assert_eq!(cell(&mut fx, 30, 20), 0);
    remove(&mut fx, o0);
    assert_eq!(cell(&mut fx, 20, 20) & 0x400, 0);
}

#[test]
fn every_monster_mode_request_restamps_a_live_monsters_footprint() {
    // The monster mode set calls the footprint add for a live unit
    // before the per-mode work, GH included: a footprint cleared under
    // the monster is back after a GH request.
    let mut fx = fx();
    let a = fx.a;
    {
        let t = Arc::make_mut(&mut fx.sim.hooks().tables);
        let ex = usize::from(t.combat.monstats[0].monstatsex);
        t.combat.monstats2[ex].sizex = 2;
    }
    let m = fx.spawn(UnitType::Monster, 0, a, 30, 30);
    let foot = cell(&mut fx, 29, 30);
    assert_ne!(foot, 0);
    fx.sim.with(&mut fx.game, |_, v| {
        assert!(v.path_remove_footprint(m, true));
    });
    assert_eq!(cell(&mut fx, 29, 30), 0);
    fx.sim.sys.with(&mut fx.game, |sim, hooks| {
        let _ = crate::units::modes::monster_set_mode(
            sim,
            hooks,
            m,
            crate::units::modes::monster_mode::GH,
        );
    });
    assert_eq!(cell(&mut fx, 29, 30), foot);
}
