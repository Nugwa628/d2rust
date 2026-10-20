//! The unit find (`find.rs`) on a table-driven world: room selection,
//! town skip, the filter's type / mode / flag tests, the excluded unit
//! and the negative-radius variants.

use std::collections::BTreeMap;

use super::super::find::{find_flag as ff, find_units, room_rejected, FindQuery, FindWorld};
use crate::units::{RoomId, UnitId, UnitType};

#[derive(Clone)]
struct U {
    ty: UnitType,
    mode: u32,
    pos: (i32, i32),
    flags: u32,
}

#[derive(Default)]
struct W {
    boxes: BTreeMap<u32, (i32, i32, i32, i32)>,
    adj: BTreeMap<u32, Vec<u32>>,
    town: Vec<u32>,
    units: BTreeMap<u32, Vec<UnitId>>,
    data: BTreeMap<UnitId, U>,
    blocked: bool,
    undead: Vec<UnitId>,
    explosion: BTreeMap<UnitId, bool>,
}

impl W {
    fn add(&mut self, room: u32, id: u32, ty: UnitType, mode: u32, pos: (i32, i32)) -> UnitId {
        let u = UnitId(id);
        self.units.entry(room).or_default().push(u);
        self.data.insert(
            u,
            U {
                ty,
                mode,
                pos,
                flags: 0,
            },
        );
        u
    }
}

impl FindWorld for W {
    fn room_box(&self, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        self.boxes.get(&room.0).copied()
    }
    fn adjacent(&self, room: RoomId) -> Vec<RoomId> {
        self.adj
            .get(&room.0)
            .map_or_else(|| vec![room], |v| v.iter().map(|&r| RoomId(r)).collect())
    }
    fn room_in_town(&self, room: RoomId) -> bool {
        self.town.contains(&room.0)
    }
    fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        self.units.get(&room.0).cloned().unwrap_or_default()
    }
    fn unit_type(&self, unit: UnitId) -> Option<UnitType> {
        self.data.get(&unit).map(|u| u.ty)
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.data[&unit].mode
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.data[&unit].pos
    }
    fn unit_flags(&self, unit: UnitId) -> u32 {
        self.data[&unit].flags
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.undead.contains(&unit)
    }
    fn missile_explosion(&self, unit: UnitId) -> Option<bool> {
        self.explosion.get(&unit).copied()
    }
    fn line_blocked(&self, _: (i32, i32), _: UnitId) -> bool {
        self.blocked
    }
}

fn q(flags: u32, r: i32) -> FindQuery {
    FindQuery {
        flags,
        x: 100,
        y: 100,
        r,
        ..FindQuery::default()
    }
}

fn find(w: &mut W, room: u32, q: &FindQuery) -> Vec<UnitId> {
    find_units(w, Some(RoomId(room)), q)
}

#[test]
fn excluded_unit_is_tested_only_for_players() {
    let mut w = W::default();
    let p = w.add(0, 1, UnitType::Player, 1, (100, 100));
    let m = w.add(0, 2, UnitType::Monster, 1, (100, 100));
    let o = w.add(0, 3, UnitType::Object, 0, (100, 100));
    let mut query = q(ff::PLAYERS | ff::MONSTERS | ff::OBJECTS, 5);
    // Excluding the player drops it; excluding a monster or object does
    // not.
    query.exclude = Some(p);
    assert_eq!(find(&mut w, 0, &query), [m, o]);
    query.exclude = Some(m);
    assert_eq!(find(&mut w, 0, &query), [p, m, o]);
    query.exclude = Some(o);
    assert_eq!(find(&mut w, 0, &query), [p, m, o]);
    // Also on the dead-only path (player mode 17).
    w.data.get_mut(&p).unwrap().mode = 17;
    query.flags = ff::PLAYERS | ff::DEAD_ONLY;
    query.exclude = Some(p);
    assert!(find(&mut w, 0, &query).is_empty());
    query.exclude = None;
    assert_eq!(find(&mut w, 0, &query), [p]);
}

#[test]
fn negative_radius_squares_and_rejects_rooms_inside_the_square() {
    // The rule on the box (x0, y0, w, h) = (90, 90, 20, 20) around (100,
    // 100): with r = −a the room is rejected when its box lies strictly
    // inside (x − a, x + a) or (y − a, y + a).
    let b = (90, 90, 20, 20);
    assert!(!room_rejected(b, 100, 100, 10));
    assert!(!room_rejected(b, 100, 100, 0));
    assert!(room_rejected(b, 100, 100, -11), "90 > 89 and 110 < 111");
    assert!(!room_rejected(b, 100, 100, -10), "touching is not inside");
    assert!(!room_rejected(b, 100, 100, -9));
    // One axis is enough (y inside, x not).
    assert!(room_rejected((0, 95, 500, 5), 100, 100, -8));
    // Positive radius never rejects a box of non-negative size.
    for r in 0..40 {
        assert!(!room_rejected(b, 100, 100, r));
        assert!(!room_rejected((0, 0, 0, 0), 100, 100, r));
    }

    // Through the find: r = −a uses a² as the distance bound.
    let mut w = W::default();
    w.boxes.insert(0, (0, 0, 1000, 1000));
    let near = w.add(0, 1, UnitType::Monster, 1, (103, 104)); // d² = 25
    let far = w.add(0, 2, UnitType::Monster, 1, (103, 105)); // d² = 34
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, -5)), [near]);
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, 6)), [near, far]);
    // R is taken alone only when the square overlaps its box (open
    // bounds): a box that the square touches does not count, so the
    // adjacency array is used.
    let mut w = W::default();
    w.boxes.insert(0, (105, 0, 100, 1000)); // x0 = x + a: touching
    w.adj.insert(0, vec![7, 0]);
    let in7 = w.add(7, 1, UnitType::Monster, 1, (100, 100));
    let in0 = w.add(0, 2, UnitType::Monster, 1, (100, 100));
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, -5)), [in7, in0]);
    w.boxes.insert(0, (104, 0, 100, 1000)); // overlapping: R alone
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, -5)), [in0]);
    // A room whose box lies strictly inside the square is rejected.
    let mut w = W::default();
    w.adj.insert(0, vec![0, 8]);
    w.boxes.insert(8, (96, 0, 8, 1000)); // x in (95, 105) strictly inside
    let in8 = w.add(8, 1, UnitType::Monster, 1, (100, 100));
    let in0 = w.add(0, 2, UnitType::Monster, 1, (100, 100));
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, 5)), [in0, in8]);
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, -5)), [in0]);
}

#[test]
fn line_test_uses_the_units_room_not_the_radius_box() {
    let mut w = W::default();
    let a = w.add(0, 1, UnitType::Monster, 1, (100, 100));
    let query = q(ff::MONSTERS | ff::LINE, 5);
    assert_eq!(find(&mut w, 0, &query), [a]);
    w.blocked = true;
    assert!(find(&mut w, 0, &query).is_empty());
    // Without the flag a blocked line is irrelevant.
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, 5)), [a]);
}

#[test]
fn find_returns_every_accepted_unit_in_room_then_list_order_without_a_cap() {
    // The 15-slot result array grows by 15 without limit: 40 units all
    // come back; found order = room order, then each room's list order;
    // the count returned is the number appended.
    let mut w = W::default();
    w.adj.insert(0, vec![1, 0]);
    let mut want = Vec::new();
    for i in 0..20 {
        want.push(w.add(1, 100 + i, UnitType::Monster, 1, (100, 100)));
    }
    for i in 0..20 {
        want.push(w.add(0, 200 + i, UnitType::Monster, 1, (100, 100)));
    }
    let got = find(&mut w, 0, &q(ff::MONSTERS, 5));
    assert_eq!(got.len(), 40);
    assert_eq!(got, want);
    // No start room: nothing.
    assert!(find_units(&mut w, None, &q(ff::MONSTERS, 5)).is_empty());
    // G (the room flags 0x2000) skips town rooms; without it they count.
    w.town.push(1);
    let got = find(&mut w, 0, &q(ff::MONSTERS | ff::SKIP_TOWN_ROOMS, 5));
    assert_eq!(got, want[20..]);
    assert_eq!(find(&mut w, 0, &q(ff::MONSTERS, 5)).len(), 40);
    // The limit (0x40) stops at the accepted count.
    let mut query = q(ff::MONSTERS | ff::LIMIT, 5);
    query.limit = 3;
    assert_eq!(find(&mut w, 0, &query), want[..3]);
}
