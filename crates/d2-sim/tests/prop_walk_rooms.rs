// Spec: specs/sim/pathing.md §3, §9.3, §9.6, §9.8
//! Multi-room walk properties (`docs/handoff/prop-walk.md` §4 gap 1) on
//! the shared fake `walk_rooms_fake` (models written from the specs):
//!
//! - A player (walk / run request) or a monster (toward compute) walks
//!   across a grid of rooms: the path's room always holds the unit's
//!   cell (room-exit flag §3 step 10, room recache §9.6 rule 9), the room
//!   list removes and inserts keep the unit in exactly that room, every
//!   insert queues the unit for update, each room change sends exactly
//!   the §9.8 add / removal messages (and clears a monster's AI room
//!   memo), flag 0x2 is clear after every step, and the grids end with
//!   one footprint at the unit.
//! - §9.8 directly on random client arrays and client players.
//! - Set position (§9.6 rules 8–9) on random room rects (overlap, gaps,
//!   inactive rooms, partial adjacency), hints, flags and unit kinds,
//!   against a reference record.

mod walk_rooms_fake;

use d2_sim::path::coords::to_fp16_center;
use d2_sim::path::record::{flags, DynamicPath};
use d2_sim::path::walk::find::compute;
use d2_sim::path::walk::request::{handle_message, Outcome};
use d2_sim::path::walk::seams::{PathWorld, Point};
use d2_sim::path::walk::{Step, Walk};
use d2_sim::units::{ClientId, RoomId, UnitId, UnitType};
use proptest::prelude::*;
use walk_rooms_fake::*;

/// A layout: rooms per axis, room size, wall masks, start and target
/// (global sub-tiles), adjacency rotation.
#[derive(Clone, Debug)]
struct Layout {
    rx: i32,
    ry: i32,
    w: i32,
    h: i32,
    masks: Vec<u16>,
    start: Point,
    target: Point,
    rot: usize,
}

fn layout() -> impl Strategy<Value = Layout> {
    (1i32..=3, 1i32..=2, 6i32..14, 6i32..14, 0u32..25, 0usize..8)
        .prop_filter("two rooms at least", |(rx, ry, ..)| rx * ry >= 2)
        .prop_flat_map(|(rx, ry, w, h, d, rot)| {
            let (gw, gh) = (rx * w, ry * h);
            (
                proptest::collection::vec(
                    (0u32..100).prop_map(move |r| if r < d { WALL } else { 0 }),
                    1..400,
                ),
                1..gw - 1,
                1..gh - 1,
                -2..gw + 2,
                -2..gh + 2,
            )
                .prop_map(move |(masks, sx, sy, tx, ty)| Layout {
                    rx,
                    ry,
                    w,
                    h,
                    masks,
                    start: Point::new(OX + sx, OY + sy),
                    target: Point::new(OX + tx, OY + ty),
                    rot,
                })
        })
}

fn build(l: &Layout, cl: &[Vec<ClientId>]) -> World {
    let mut w = World::layout(
        l.rx,
        l.ry,
        l.w,
        l.h,
        &[],
        &l.masks,
        Adjacency::Neighbours(l.rot),
    );
    for (i, r) in w.rooms.iter_mut().enumerate() {
        if let Some(r) = r {
            r.clients = cl[i % cl.len()].clone();
        }
    }
    w.clear_plus(l.start);
    w
}

proptest! {
    #![proptest_config(config(256))]

    /// §1 / §3 → §9 across rooms.
    // Covers: specs/sim/pathing.md §3 r10, §9.6 r9, §9.8
    #[test]
    fn multi_room_walk_tracks_rooms_and_messages(
        l in layout(),
        cl in clients(6),
        own_client in 0u32..6,
        kind in 0u8..3,
        mvel in 0x400i32..0x1400,
    ) {
        let t = tables();
        let mut world = build(&l, &cl);
        // Client `own_client` is the walking player's (6: none of them).
        world.client_players.insert(ClientId(own_client), P);
        for c in 0..5 {
            world.client_players.entry(ClientId(c)).or_insert(UnitId(50 + c));
        }
        let monster = kind == 2;
        let u = if monster { M } else { P };
        // A player client exists in both cases; the monster walks alone.
        world.add_walker(u, if monster { UnitType::Monster } else { UnitType::Player }, l.start);
        let mut initial = world.clone();
        initial.stamp(l.start, 1, world.paths[&u].foot_mask, false);
        let n = if monster {
            let mut p = world.paths[&u].clone();
            p.velocity = mvel;
            p.put_target(l.target);
            let n = compute(t, &mut world, &mut p, u, false).unwrap();
            world.paths.insert(u, p);
            n
        } else {
            let id = if kind == 1 { 0x03 } else { 0x01 };
            let (r, o) = handle_message(t, &mut world, P, id, l.target.x as u32, l.target.y as u32).unwrap();
            prop_assert_eq!(r, 0);
            match o {
                Some(Outcome::Moving(n)) => n,
                _ => 0,
            }
        };
        if n == 0 {
            return Ok(());
        }
        let path0 = world.paths[&u].clone();
        let r0 = path0.room.unwrap();
        // §3 step 10: flag 0x1 iff some point lies outside the path's room.
        let outside = path0.live_points().iter().any(|&q| !world.contains(r0, q));
        prop_assert_eq!(path0.flags & flags::OUTSIDE_ROOM != 0, outside);
        let last = path0.point(path0.point_count as usize - 1);

        let mut member: Option<RoomId> = Some(r0);
        let mut refused = false;
        let mut stopped = false;
        let mut prev = l.start;
        for _tick in 0..3000 {
            world.events.clear();
            let before = world.paths[&u].room;
            let s = if monster {
                let mut p = world.paths[&u].clone();
                let s = Walk { t, c: &mut world }.step(u, &mut p).unwrap();
                world.paths.insert(u, p);
                s
            } else {
                Walk { t, c: &mut world }.player_event0(u).unwrap()
            };
            let path = world.paths[&u].clone();
            let cell = path.cell();
            // §9.6 rule 9: the path's room holds the unit's cell.
            let room = path.room;
            prop_assert!(room.is_some_and(|r| world.contains(r, cell)), "{:?} not in {:?}", cell, room);
            prop_assert!(world.free(cell, path.pattern), "unit at blocked {:?}", cell);
            prop_assert!((cell.x - prev.x).abs() <= 2 && (cell.y - prev.y).abs() <= 2);
            // Room lists and the update queue (§9.6 rule 9).
            let mut inserts = 0;
            let mut queued = 0;
            let mut msgs = Vec::new();
            let mut memo = 0;
            for e in &world.events {
                match *e {
                    Ev::ListRemove(x, r) => {
                        prop_assert_eq!(x, u);
                        prop_assert_eq!(member, Some(r), "removed from a room it is not in");
                        member = None;
                    }
                    Ev::ListInsert(x, r) => {
                        prop_assert_eq!(x, u);
                        prop_assert_eq!(member, None, "inserted twice");
                        member = Some(r);
                        inserts += 1;
                    }
                    Ev::Queue(x) => {
                        prop_assert_eq!(x, u);
                        queued += 1;
                    }
                    Ev::AiMemo(x) => {
                        prop_assert_eq!(x, u);
                        memo += 1;
                    }
                    Ev::Removal(..) | Ev::Add(..) => msgs.push(*e),
                    Ev::UnitFlag(..) => prop_assert!(false, "re-path without budget"),
                }
            }
            prop_assert_eq!(member, room);
            prop_assert_eq!(queued, inserts);
            // §9.8: flag 0x2 is cleared by the step's messages.
            prop_assert_eq!(path.flags & flags::ROOM_CHANGED, 0);
            if before != room {
                prop_assert_eq!(path.prev_room, before);
                let old = world.room_clients(before.unwrap());
                let new = world.room_clients(room.unwrap());
                prop_assert_eq!(msgs, ref_messages(&old, &new, u, &world.client_players));
                prop_assert_eq!(memo, usize::from(monster));
            } else {
                prop_assert!(msgs.is_empty() && memo == 0, "{:?}", world.events);
            }
            if path.collided_mask != 0 {
                refused = true;
            }
            prev = cell;
            if s == Step::Stopped {
                stopped = true;
                break;
            }
        }
        prop_assert!(stopped, "still moving after 3000 ticks");
        let path = &world.paths[&u];
        prop_assert_eq!(path.precise_x & 0xFFFF, 0x8000);
        prop_assert_eq!(path.precise_y & 0xFFFF, 0x8000);
        let exhausted = kind == 1 && world.unit(P).stats[&10] == 0;
        if !refused && !exhausted {
            prop_assert_eq!(path.cell(), last);
        }
        // One footprint, at the unit, across the rooms.
        let mut expect = initial;
        expect.stamp(path.cell(), path.pattern, path.foot_mask, true);
        prop_assert_eq!(world.grids(), expect.grids());
    }
}

proptest! {
    #![proptest_config(config(512))]

    /// §9.8 on random client arrays.
    // Covers: specs/sim/pathing.md §9.8
    #[test]
    fn room_change_messages_merge(
        cl in clients(3),
        players in proptest::collection::vec(prop_oneof![Just(1u32), Just(3), 10u32..14], 5),
        prev in prop_oneof![Just(None), (0u32..3).prop_map(Some)],
        now in prop_oneof![Just(None), (0u32..3).prop_map(Some)],
        changed in any::<bool>(),
        monster in any::<bool>(),
    ) {
        let t = tables();
        let mut world = World::layout(3, 1, 6, 6, &[], &[0], Adjacency::Neighbours(0));
        for (i, r) in world.rooms.iter_mut().enumerate() {
            r.as_mut().unwrap().clients = cl[i].clone();
        }
        for (c, p) in players.iter().enumerate() {
            world.client_players.insert(ClientId(c as u32), UnitId(*p));
        }
        let u = if monster { M } else { P };
        world.add_walker(u, if monster { UnitType::Monster } else { UnitType::Player }, Point::new(OX + 2, OY + 2));
        let mut path = world.paths[&u].clone();
        path.prev_room = prev.map(RoomId);
        path.room = now.map(RoomId);
        path.flags = (path.flags & !flags::ROOM_CHANGED) | if changed { flags::ROOM_CHANGED } else { 0 };
        let mut expect_path = path.clone();
        world.events.clear();
        Walk { t, c: &mut world }.room_change_messages(u, &mut path);
        let mut expect = Vec::new();
        if changed {
            if monster {
                expect.push(Ev::AiMemo(u));
            }
            let old = prev.map(|r| cl[r as usize].clone()).unwrap_or_default();
            let new = now.map(|r| cl[r as usize].clone()).unwrap_or_default();
            expect.extend(ref_messages(&old, &new, u, &world.client_players));
            expect_path.flags &= !flags::ROOM_CHANGED;
        }
        prop_assert_eq!(&world.events, &expect);
        prop_assert_eq!(path, expect_path);
    }
}

/// Rooms with random rects (overlap and gaps allowed), some inactive,
/// random adjacency arrays.
#[derive(Clone, Debug)]
struct Rooms {
    rects: Vec<Option<(i32, i32, i32, i32)>>,
    adj: Vec<Vec<u32>>,
}

fn rooms() -> impl Strategy<Value = Rooms> {
    (1usize..6).prop_flat_map(|n| {
        (
            proptest::collection::vec(
                proptest::option::weighted(0.85, (0i32..30, 0i32..30, 1i32..12, 1i32..12)),
                n,
            ),
            proptest::collection::vec(proptest::collection::vec(0u32..7, 0..5), n),
        )
            .prop_map(|(rects, adj)| Rooms { rects, adj })
    })
}

fn world_of(r: &Rooms) -> World {
    let mut w = World::layout(1, 1, 1, 1, &[], &[0], Adjacency::Neighbours(0));
    w.rooms = r
        .rects
        .iter()
        .zip(&r.adj)
        .map(|(rect, adj)| {
            rect.map(|(x, y, rw, rh)| Room {
                grid: d2_sim::drlg::CollisionGrid::new(d2_sim::drlg::TileRect::new(
                    OX + x,
                    OY + y,
                    rw,
                    rh,
                )),
                adj: adj.iter().map(|&a| RoomId(a)).collect(),
                clients: Vec::new(),
                town: false,
            })
        })
        .collect();
    w
}

proptest! {
    #![proptest_config(config(1024))]

    /// §9.6 rules 8–9 against the reference record.
    // Covers: specs/sim/pathing.md §9.6 r8, §9.6 r9
    #[test]
    fn set_position_recaches_by_spec(
        r in rooms(),
        room in proptest::option::of(0u32..7),
        prev_room in proptest::option::of(0u32..7),
        hint in proptest::option::of(0u32..7),
        q in (-4i32..46, -4i32..46, any::<u16>(), any::<u16>()),
        flag_bits in 0u32..8,
        count in 0u32..5,
        missile in any::<bool>(),
    ) {
        let t = tables();
        let mut world = world_of(&r);
        let ty = if missile { UnitType::Missile } else { UnitType::Monster };
        world.units.insert(M, Unit::new(ty, 3));
        let mut path = DynamicPath {
            owner: Some(M),
            room: room.map(RoomId),
            prev_room: prev_room.map(RoomId),
            flags: flag_bits,
            point_count: count,
            precise_x: to_fp16_center(OX + 1),
            precise_y: to_fp16_center(OY + 1),
            ..DynamicPath::default()
        };
        path.update_client();
        let qx = (((OX + q.0) as u32) << 16) | q.2 as u32;
        let qy = (((OY + q.1) as u32) << 16) | q.3 as u32;
        let mut expect = path.clone();
        let mut ev = Vec::new();
        ref_set_position(&world, &mut expect, M, (qx, qy), hint.map(RoomId), missile, &mut ev);
        Walk { t, c: &mut world }.set_position(M, &mut path, (qx, qy), hint.map(RoomId));
        prop_assert_eq!(&world.events, &ev);
        prop_assert_eq!(path, expect);
    }
}
