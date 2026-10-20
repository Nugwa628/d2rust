//! Footprint mask change, dead-body footprint and teleport
//! (`docs/handoff/prop-walk.md` §4 gap 4), on the shared multi-room fake
//! `walk_rooms_fake` with every room adjacent to every other (so the
//! lookup room of §5.1 and §6 rule 4 does not change which cell is
//! found) and random cell bits, against reference models:
//!
//! - `set_foot_mask` (§5.3 rule 1): clear with the current pattern (or
//!   size, missiles) and the old mask (marker only when it is not 0),
//!   store the mask, stamp again.
//! - `make_corpse_footprint` (§5.3 rule 3): forced remove, pattern 5,
//!   mask 0x8000 through rule 1.
//! - `teleport` (§6 rule 4): the fatal assert, the footprint move or
//!   clear, flags 0x8 / 0x1, the missile's collided mask and saved step,
//!   set position with the room recache (`pathing.md` §9.6 rules 8–9,
//!   destination room as the hint) and the movement reset (§9.7);
//!   `teleport_and_clear` then clears the count.

mod walk_rooms_fake;

use d2_sim::path::coords::to_fp16_center;
use d2_sim::path::footprint::{make_corpse_footprint, set_foot_mask, teleport, teleport_and_clear};
use d2_sim::path::record::{flags, DynamicPath, PathPoint};
use d2_sim::path::walk::seams::Point;
use d2_sim::units::{RoomId, UnitType};
use proptest::prelude::*;
use walk_rooms_fake::*;

const BITS: [u16; 8] = [0x1, 0x4, 0x80, 0x100, 0x400, 0x1000, 0x2000, 0x8000];

fn mask() -> impl Strategy<Value = u16> {
    prop_oneof![
        1 => Just(0u16),
        4 => proptest::collection::vec(0usize..8, 1..3)
            .prop_map(|v| v.into_iter().fold(0, |a, i| a | BITS[i])),
    ]
}

/// A 3 × 2 grid of 6 × 6 rooms with up to one hole, random cell bits.
#[derive(Clone, Debug)]
struct Grid {
    hole: Option<usize>,
    cells: Vec<u16>,
    rot: usize,
}

fn grid() -> impl Strategy<Value = Grid> {
    (
        proptest::option::weighted(0.3, 0usize..6),
        proptest::collection::vec(mask(), 1..200),
        0usize..6,
    )
        .prop_map(|(hole, cells, rot)| Grid { hole, cells, rot })
}

const RW: i32 = 6;

fn build(g: &Grid) -> World {
    let mut holes = [false; 6];
    if let Some(h) = g.hole {
        holes[h] = true;
    }
    World::layout(3, 2, RW, RW, &holes, &g.cells, Adjacency::Complete(g.rot))
}

/// A cell in or just around the union.
fn cell() -> impl Strategy<Value = Point> {
    (-2i32..3 * RW + 2, -2i32..2 * RW + 2).prop_map(|(x, y)| Point::new(OX + x, OY + y))
}

fn path_at(w: &World, p: Point, room: Option<RoomId>) -> DynamicPath {
    let mut d = DynamicPath {
        owner: Some(M),
        precise_x: to_fp16_center(p.x),
        precise_y: to_fp16_center(p.y),
        room,
        ..DynamicPath::default()
    };
    let _ = w;
    d.update_client();
    d
}

/// The path's room: the one holding the cell, sometimes another one or
/// none.
fn room_choice() -> impl Strategy<Value = u8> {
    prop_oneof![6 => Just(0u8), 1 => Just(1u8), 1 => Just(2u8)]
}

fn pick_room(w: &World, p: Point, choice: u8, other: u32) -> Option<RoomId> {
    match choice {
        0 => w.room_at(p),
        1 => Some(RoomId(other % 6)).filter(|&r| w.room(r).is_some()),
        _ => None,
    }
}

/// §5.3 rule 1 on the model world.
fn ref_set_mask(w: &mut World, path: &mut DynamicPath, missile: bool, mask: u16) {
    if path.room.is_some() {
        if missile {
            w.stamp_size(path.cell(), path.unit_size, path.foot_mask, false);
        } else {
            w.stamp(path.cell(), path.pattern, path.foot_mask, false);
        }
    }
    path.foot_mask = mask;
    if path.room.is_some() {
        if missile {
            w.stamp_size(path.cell(), path.unit_size, mask, true);
        } else {
            w.stamp(path.cell(), path.pattern, mask, true);
        }
    }
}

proptest! {
    #![proptest_config(config(1024))]

    /// §5.3 rule 1.
    #[test]
    fn set_foot_mask_restamps(
        g in grid(),
        at in cell(),
        choice in room_choice(),
        other in any::<u32>(),
        pattern in 0u32..7,
        size in -1i32..5,
        old in mask(),
        new in mask(),
        missile in any::<bool>(),
    ) {
        let mut world = build(&g);
        let room = pick_room(&world, at, choice, other);
        let mut path = path_at(&world, at, room);
        path.pattern = pattern;
        path.unit_size = size;
        path.foot_mask = old;
        let mut expect_w = world.clone();
        let mut expect_p = path.clone();
        ref_set_mask(&mut expect_w, &mut expect_p, missile, new);
        set_foot_mask(&mut world, &mut path, missile, new);
        prop_assert_eq!(path, expect_p);
        prop_assert_eq!(world.grids(), expect_w.grids());
    }

    /// §5.3 rule 3 (with §5.2 remove, force, and rule 1).
    #[test]
    fn corpse_footprint_is_a_plus_of_0x8000(
        g in grid(),
        at in cell(),
        choice in room_choice(),
        other in any::<u32>(),
        pattern in 0u32..7,
        old in mask(),
    ) {
        let mut world = build(&g);
        let room = pick_room(&world, at, choice, other);
        let mut path = path_at(&world, at, room);
        path.pattern = pattern;
        path.foot_mask = old;
        let mut expect_w = world.clone();
        let mut expect_p = path.clone();
        if room.is_some() {
            expect_w.stamp(at, pattern, old, false);
        }
        expect_p.pattern = 5;
        ref_set_mask(&mut expect_w, &mut expect_p, false, 0x8000);
        make_corpse_footprint(&mut world, &mut path);
        prop_assert_eq!(path.pattern, 5);
        prop_assert_eq!(path.foot_mask, 0x8000);
        prop_assert_eq!(path, expect_p);
        prop_assert_eq!(world.grids(), expect_w.grids());
    }
}

/// Box query (§4 rule 4): the room of (left, bottom) from `room`, none →
/// 0x27; the inside box ORs its cells; the strip right of the room (full
/// height) and the strip above it (inside width) are queried again from
/// that room.
fn ref_box(w: &World, room: Option<RoomId>, l: i32, b: i32, r: i32, t: i32, mask: u16) -> u16 {
    let Some(rm) = w.lookup(room, Point::new(l, b)) else {
        return MISSING;
    };
    let rect = w.room(rm).unwrap().grid.rect;
    let ir = r.min(rect.x + rect.w - 1);
    let it = t.min(rect.y + rect.h - 1);
    let mut v = 0;
    for y in b..=it {
        for x in l..=ir {
            v |= w.masked(Point::new(x, y), mask);
        }
    }
    if r > ir {
        v |= ref_box(w, Some(rm), ir + 1, b, r, t, mask);
    }
    if t > it {
        v |= ref_box(w, Some(rm), l, it + 1, ir, t, mask);
    }
    v
}

/// §9.7 reset on the model record.
fn ref_reset(w: &World, path: &mut DynamicPath, missile: bool, ev: &mut Vec<Ev>) {
    let c = path.cell();
    ref_set_position(
        w,
        path,
        M,
        (to_fp16_center(c.x), to_fp16_center(c.y)),
        None,
        missile,
        ev,
    );
    path.flags &= !flags::ACTIVE;
    path.point_count = 0;
    path.cur_point = 0;
    path.vel_vec_x = 0;
    path.vel_vec_y = 0;
}

proptest! {
    #![proptest_config(config(1024))]

    /// §6 rule 4 against the model.
    #[test]
    fn teleport_follows_the_spec(
        g in grid(),
        from in cell(),
        to in prop_oneof![6 => cell(), 1 => Just(Point::new(0, 0))],
        from_choice in room_choice(),
        to_choice in room_choice(),
        other in (any::<u32>(), any::<u32>()),
        pattern in 0u32..6,
        size in -1i32..5,
        foot in mask(),
        movemask in mask(),
        flag_bits in any::<u32>(),
        count in 0u32..5,
        missile in any::<bool>(),
        clear in any::<bool>(),
        same in proptest::bool::weighted(0.15),
    ) {
        // A teleport onto its own cell (flag 0x8 stays clear).
        let to = if same { from } else { to };
        let mut world = build(&g);
        let ty = if missile { UnitType::Missile } else { UnitType::Monster };
        world.units.insert(M, Unit::new(ty, 3));
        let room0 = pick_room(&world, from, from_choice, other.0);
        let room = pick_room(&world, to, to_choice, other.1);
        let mut path = path_at(&world, from, room0);
        path.pattern = pattern;
        path.unit_size = size;
        path.foot_mask = foot;
        path.move_mask = movemask;
        path.flags = flag_bits & !flags::MISSILE;
        path.point_count = count;
        path.cur_point = count / 2;
        path.vel_vec_x = 3;
        path.vel_vec_y = -3;

        let zero = (to.x, to.y) == (0, 0);
        if !zero && room.is_none() {
            let (w0, p0) = (world.clone(), path.clone());
            let r = teleport(&mut world, &mut path, missile, room, to.x, to.y);
            prop_assert!(r.is_err());
            prop_assert_eq!(path, p0);
            prop_assert_eq!(world, w0);
            return Ok(());
        }
        let mut ew = world.clone();
        let mut ep = path.clone();
        let mut ev = Vec::new();
        let old = from;
        if missile {
            if zero {
                if ep.room.is_some() {
                    ew.stamp_size(old, size, foot, false);
                }
                ep.collided_mask = 0;
            } else {
                ep.flags = (ep.flags & !flags::MOVED) | if old != to { flags::MOVED } else { 0 };
                // Missile teleport: clear at old (path room), then the size
                // query and the stamp at new from the destination room.
                if ep.room.is_some() {
                    ew.stamp_size(old, size, foot, false);
                }
                ep.collided_mask = match size {
                    // §4 rule 3: a centre without a room gives 0x27 alone.
                    0..=2 if ew.lookup(room, to).is_none() => MISSING,
                    0..=2 => ew.query(to, size_query_cells(size), movemask),
                    3 => ref_box(&ew, room, to.x - 1, to.y - 1, to.x + 1, to.y + 1, movemask),
                    _ => 0xFFFF,
                };
                ew.stamp_size(to, size, foot, true);
                ep.saved_count = 1;
                ep.saved_steps[0] = PathPoint::from_point(to);
            }
        } else if ep.room.is_some() {
            ew.stamp(old, pattern, foot, false);
            if !zero {
                ew.stamp(to, pattern, foot, true);
            }
        }
        if room != ep.room {
            ep.flags |= flags::OUTSIDE_ROOM;
        }
        ref_set_position(&ew, &mut ep, M, (to_fp16_center(to.x), to_fp16_center(to.y)), room, missile, &mut ev);
        ref_reset(&ew, &mut ep, missile, &mut ev);
        if clear {
            ep.point_count = 0;
            teleport_and_clear(&mut world, &mut path, missile, room, to.x, to.y).unwrap();
        } else {
            teleport(&mut world, &mut path, missile, room, to.x, to.y).unwrap();
        }
        prop_assert_eq!(&world.events, &ev);
        prop_assert_eq!(path, ep);
        prop_assert_eq!(world.grids(), ew.grids());
    }
}
