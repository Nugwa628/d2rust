// Spec: specs/sim/pathing.md §1.5, §2, §9.2, §9.5, §9.6, §9.10
//! Knockback, path-type set, moving target units and the re-path budget
//! (`docs/handoff/prop-walk.md` §4 gap 2), on the shared fake
//! `walk_rooms_fake` (models written from the specs):
//!
//! - Knockback (mode 19, type 8, §1.5 step 2): the request sets type 8,
//!   the previous type and the saved velocity (§2), budget 5 and
//!   velocity 0x1000 (§8.1 rule 1); the walk never decrements the
//!   distance budget (§9.6 rule 4), while the same walk under type 7
//!   decrements it once per tick that leaves the cell, until 0. Mode 19
//!   drains no stamina (§9.2 step 3).
//! - Path-type set (§2) on random records and type sequences against a
//!   model of the table rule.
//! - A monster (type 2) or a player (type 7) chasing a target unit that
//!   moves, vanishes or is replaced: target check (§9.2 step 1), the
//!   arrival check's stop distance and its re-path on a target that moved
//!   more than 5 (§9.5 rule 3), the re-path budget gate (§9.10: monsters
//!   only, path +0x94; unit flag 1, update queue, budget −= index clamped
//!   to 0..255; types 2 / 13 / 15 → 13, else 15),
//!   and the distance budget per crossed cell (§9.6 rule 4).

mod walk_rooms_fake;

use d2_sim::path::record::{flags, DynamicPath};
use d2_sim::path::walk::find::compute;
use d2_sim::path::walk::request::{handle_message, request, Outcome, WalkTarget};
use d2_sim::path::walk::seams::Point;
use d2_sim::path::walk::{Step, Walk};
use d2_sim::units::{UnitId, UnitType};
use proptest::prelude::*;
use walk_rooms_fake::*;

/// One room of `w` × `h` with walls, the walker at `start`.
fn arena() -> impl Strategy<Value = (i32, i32, Vec<u16>, Point)> {
    (8i32..24, 8i32..24, prop_oneof![2 => 0u32..5, 1 => 0u32..30]).prop_flat_map(|(w, h, d)| {
        (
            Just(w),
            Just(h),
            proptest::collection::vec(
                (0u32..100).prop_map(move |r| if r < d { WALL } else { 0 }),
                1..300,
            ),
            (1..w - 1, 1..h - 1).prop_map(|(x, y)| Point::new(OX + x, OY + y)),
        )
    })
}

fn one_room(w: i32, h: i32, masks: &[u16], start: Point) -> World {
    let mut world = World::layout(1, 1, w, h, &[], masks, Adjacency::Neighbours(0));
    world.clear_plus(start);
    world
}

/// Runs player event 0 until the unit stops (≤ 400 ticks); returns
/// (cell, distance budget, crossed-or-refused) per tick.
fn knock_walk(world: &mut World) -> Vec<(Point, u8, bool)> {
    let t = tables();
    let mut out = Vec::new();
    let mut prev = world.paths[&P].cell();
    for _ in 0..400 {
        let s = Walk { t, c: &mut *world }.player_event0(P).unwrap();
        let p = &world.paths[&P];
        let attempted = p.cell() != prev || p.collided_mask != 0;
        out.push((p.cell(), p.dist_budget, attempted));
        prev = p.cell();
        if s == Step::Stopped {
            break;
        }
    }
    out
}

proptest! {
    #![proptest_config(config(256))]

    /// §1.5 step 2, §2, §8.1 rule 1, §9.2 step 3, §9.6 rule 4.
    // Covers: specs/sim/pathing.md §1.5 r2, §8.1 r1, §9.2 r3, §9.6 r4
    #[test]
    fn knockback_keeps_the_distance_budget(
        (w, h, masks, start) in arena(),
        dir in proptest::sample::select(vec![(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]),
        k in 1i32..8,
        v0 in 0x100i32..0x2000,
        stamina in 1i32..(100 << 8),
    ) {
        let t = tables();
        let mut world = one_room(w, h, &masks, start);
        world.add_walker(P, UnitType::Player, start);
        world.paths.get_mut(&P).unwrap().velocity = v0;
        world.unit_mut(P).stats.insert(10, stamina);
        let target = Point::new(start.x + dir.0 * k, start.y + dir.1 * k);
        let o = request(t, &mut world, P, None, 19, WalkTarget::Point(target), false).unwrap();
        let inside = world.contains(d2_sim::units::RoomId(0), target);
        if !inside {
            // §3 step 5: no target room, no path.
            prop_assert_eq!(o, Outcome::Neutral);
            return Ok(());
        }
        // §12.5: the type-8 point is the ray's last free cell from the
        // start toward the target; a blocked first cell leaves the start,
        // so no path.
        let first = Point::new(start.x + dir.0, start.y + dir.1);
        if !world.free(first, 1) {
            prop_assert_eq!(o, Outcome::Neutral);
            return Ok(());
        }
        prop_assert_eq!(o, Outcome::Moving(1));
        let p0 = world.paths[&P].clone();
        prop_assert_eq!(world.unit(P).mode, 19);
        prop_assert_eq!(p0.path_type, 8);
        prop_assert_eq!(p0.prev_path_type, 7);
        prop_assert_eq!(p0.saved_velocity, v0);
        prop_assert_eq!(p0.velocity, 0x1000);
        prop_assert_eq!(p0.dist_budget, 5);
        prop_assert_eq!(p0.flags & 0x7FF00, t.pathtype_flags[8] & 0x7FF00);

        // The same record under type 7 walks the same cells.
        let mut control = world.clone();
        control.paths.get_mut(&P).unwrap().path_type = 7;
        let knock = knock_walk(&mut world);
        let ctl = knock_walk(&mut control);
        prop_assert_eq!(knock.len(), ctl.len());
        let mut b = 5u8;
        for (i, (a, c)) in knock.iter().zip(&ctl).enumerate() {
            prop_assert_eq!(a.0, c.0, "tick {}", i);
            prop_assert!(world.free(a.0, 1), "knocked into blocked {:?}", a.0);
            prop_assert_eq!(a.1, 5, "type 8 budget changed at tick {}", i);
            if c.2 && b > 0 {
                b -= 1;
            }
            prop_assert_eq!(c.1, b, "type 7 budget at tick {}", i);
        }
        prop_assert!(knock.len() < 400);
        // No stamina drain outside mode 3.
        prop_assert_eq!(world.unit(P).stats[&10], stamina);
        let end = world.paths[&P].cell();
        prop_assert!((end.x - start.x).abs() <= k && (end.y - start.y).abs() <= k);
    }
}

/// §2 type set as written, on a model record: (flags, type, previous
/// type, saved velocity, direction offset), or `None` for a fatal assert.
fn ref_set_type(
    m: &mut (u32, u32, u32, i32, i32),
    velocity: i32,
    max_distance: u8,
    player: bool,
    ty: u32,
) -> bool {
    let t = tables();
    let Some(&tf) = t.pathtype_flags.get(ty as usize) else {
        return false;
    };
    if player && ty == 2 {
        return false;
    }
    if tf & 0x2000 != 0 && m.0 & 0x4000 == 0 {
        m.2 = m.1;
    }
    if tf & 0x8000 != 0 && m.0 & 0x10000 == 0 {
        m.3 = velocity;
    }
    m.0 = (m.0 & !0x7FF00) | tf;
    m.1 = ty;
    m.4 = t.pathtype_diroff[ty as usize];
    !(matches!(m.2, 8 | 11) || (ty == 4 && max_distance >= 78))
}

proptest! {
    #![proptest_config(config(1024))]

    /// §2 set type on random records and sequences.
    // Covers: specs/sim/pathing.md §2
    #[test]
    fn set_path_type_follows_the_table(
        flags0 in any::<u32>(),
        ty0 in 0u32..18,
        prev0 in 0u32..18,
        velocity in any::<i32>(),
        saved in any::<i32>(),
        max_distance in any::<u8>(),
        ops in proptest::collection::vec((any::<bool>(), 0u32..20), 1..12),
    ) {
        let t = tables();
        let mut path = DynamicPath {
            flags: flags0,
            path_type: ty0,
            prev_path_type: prev0,
            velocity,
            saved_velocity: saved,
            max_distance,
            ..DynamicPath::default()
        };
        let mut m = (flags0, ty0, prev0, saved, path.dir_offset);
        for (player, ty) in ops {
            let ok = ref_set_type(&mut m, velocity, max_distance, player, ty);
            let r = path.set_path_type(t, player, ty);
            prop_assert_eq!(r.is_ok(), ok, "type {} player {}", ty, player);
            if !ok {
                // A fatal assert: the game does not go on.
                break;
            }
            prop_assert_eq!(
                (path.flags, path.path_type, path.prev_path_type, path.saved_velocity, path.dir_offset),
                m
            );
            prop_assert_eq!(path.velocity, velocity);
        }
    }
}

/// What happens to the target unit before a tick.
#[derive(Clone, Copy, Debug)]
enum Move {
    Step(i32, i32),
    Jump(i32, i32),
    Vanish,
    Replace,
}

fn moves() -> impl Strategy<Value = Vec<Move>> {
    proptest::collection::vec(
        prop_oneof![
            8 => (-2i32..=2, -2i32..=2).prop_map(|(x, y)| Move::Step(x, y)),
            4 => (-8i32..=8, -8i32..=8).prop_map(|(x, y)| Move::Jump(x, y)),
            1 => Just(Move::Vanish),
            1 => Just(Move::Replace),
        ],
        1..200,
    )
}

proptest! {
    #![proptest_config(config(256))]

    /// §9.2 step 1, §9.5 rule 3, §9.6 rule 4, §9.10.
    // Covers: specs/sim/pathing.md §9.2 r1, §9.5 r3, §9.10
    #[test]
    fn chase_a_moving_target(
        (w, h, masks, start) in arena(),
        tpos in (1i32..23, 1i32..23),
        target_monster in any::<bool>(),
        chaser_monster in any::<bool>(),
        budget in prop_oneof![Just(0i32), 1i32..4],
        stop in 0u8..4,
        tsize in 0i32..4,
        mvel in 0x400i32..0x1400,
        moves in moves(),
    ) {
        let t = tables();
        let mut world = one_room(w, h, &masks, start);
        let u = if chaser_monster { M } else { P };
        world.add_walker(u, if chaser_monster { UnitType::Monster } else { UnitType::Player }, start);
        let tty = if target_monster { UnitType::Monster } else { UnitType::Player };
        let mut tu = Unit::new(tty, 77);
        tu.pos = Point::new(OX + tpos.0.min(w - 1), OY + tpos.1.min(h - 1));
        tu.size = tsize;
        world.units.insert(T, tu);
        let n = if chaser_monster {
            let mut p = world.paths[&M].clone();
            // §9.10: the monster re-path budget lives at path +0x94.
            p.repath_budget = budget as u8;
            p.velocity = mvel;
            p.stop_distance = stop;
            p.target_unit = Some(d2_sim::path::record::TargetUnit { unit: T, ty: tty, guid: 77 });
            let n = compute(t, &mut world, &mut p, M, false).unwrap();
            world.paths.insert(M, p);
            n
        } else {
            world.paths.get_mut(&P).unwrap().stop_distance = stop;
            let (_, o) = handle_message(t, &mut world, P, 0x02, tty as u32, 77).unwrap();
            match o {
                Some(Outcome::Moving(n)) => n,
                _ => 0,
            }
        };
        if n == 0 {
            return Ok(());
        }
        let mut gone = false;
        for mv in moves {
            // The target moves (it has no path; its position is the unit's).
            match mv {
                Move::Step(dx, dy) | Move::Jump(dx, dy) => {
                    if let Some(tu) = world.units.get_mut(&T) {
                        tu.pos.x = (tu.pos.x + dx).clamp(OX, OX + w - 1);
                        tu.pos.y = (tu.pos.y + dy).clamp(OY, OY + h - 1);
                    }
                }
                Move::Vanish => {
                    world.units.remove(&T);
                    gone = true;
                }
                Move::Replace => {
                    // Another unit takes the GUID (§9.2 step 1: the unit
                    // found differs from the stored one).
                    if let Some(old) = world.units.remove(&T) {
                        world.units.insert(UnitId(9), old);
                    }
                    gone = true;
                }
            }
            let p0 = world.paths[&u].clone();
            let b0 = p0.dist_budget;
            world.events.clear();
            let s = if chaser_monster {
                let mut p = p0.clone();
                let s = Walk { t, c: &mut world }.step(M, &mut p).unwrap();
                world.paths.insert(M, p);
                s
            } else {
                Walk { t, c: &mut world }.player_event0(P).unwrap()
            };
            let p1 = world.paths[&u].clone();
            prop_assert!(world.free(p1.cell(), p1.pattern), "unit at blocked {:?}", p1.cell());
            let flagged = world.events.iter().filter(|e| matches!(e, Ev::UnitFlag(x, 1) if *x == u)).count();
            let queued = world.events.iter().filter(|e| matches!(e, Ev::Queue(x) if *x == u)).count();
            let attempted = p1.cell() != p0.cell() || p1.collided_mask != 0;
            let step_budget = |b: u8| if attempted && b > 0 { b - 1 } else { b };
            if gone {
                // §9.2 step 1: the stale target is dropped.
                prop_assert_eq!(p1.target_unit, None);
                if s == Step::Stopped {
                    break;
                }
                continue;
            }
            let tp = world.units[&T].pos;
            let d = ref_unit_distance(p0.cell(), p0.unit_size, tp, tsize);
            let prev = p0.prev_target();
            let moved = (tp.x - prev.x).abs() > 5 || (tp.y - prev.y).abs() > 5;
            if d <= stop as i32 {
                // §9.5 rule 3: close enough, the unit stops.
                prop_assert_eq!(s, Step::Stopped);
                prop_assert_eq!((flagged, queued), (0, 0));
                prop_assert_eq!(p1.path_type, p0.path_type);
                prop_assert_eq!(p1.cell(), p0.cell());
                break;
            }
            if moved {
                if p0.repath_budget == 0 && chaser_monster {
                    // §9.10: a monster without budget (it starts at
                    // `budget` and loses the point index at each
                    // re-path) does not re-path; the arrival fails
                    // (players skip the budget test).
                    prop_assert_eq!(s, Step::Stopped);
                    prop_assert_eq!((flagged, queued), (0, 0));
                    prop_assert_eq!(p1.path_type, p0.path_type);
                    prop_assert_eq!(p1.dist_budget, b0);
                    break;
                }
                prop_assert_eq!((flagged, queued), (1, 1));
                // §9.10: the re-path adjusts the re-path budget (+0x94),
                // not the distance budget (+0x90).
                prop_assert_eq!(p1.dist_budget, step_budget(b0));
                prop_assert_eq!(
                    p1.repath_budget,
                    (i32::from(p0.repath_budget) - p0.cur_point as i32).clamp(0, 255) as u8
                );
                // §9.10: type 13, and when that compute gives 0, type 15
                // and a second compute, which may still move the unit.
                let ty_ok = if chaser_monster {
                    matches!(p1.path_type, 13 | 15)
                } else {
                    p1.path_type == 7
                };
                prop_assert!(ty_ok, "type {} after re-path ({:?})", p1.path_type, s);
                // §3 step 10: previous target := the refreshed target,
                // unless §4 prepared a blocked one (type 7 has 0x1000).
                if s == Step::Moving && (chaser_monster || world.free(tp, p1.pattern)) {
                    prop_assert_eq!(p1.prev_target(), tp);
                }
            } else {
                prop_assert_eq!((flagged, queued), (0, 0));
                prop_assert_eq!(p1.dist_budget, step_budget(b0));
                prop_assert_eq!(p1.path_type, p0.path_type);
            }
            if s == Step::Stopped {
                prop_assert_eq!(p1.flags & flags::ACTIVE, 0);
                break;
            }
        }
    }
}
