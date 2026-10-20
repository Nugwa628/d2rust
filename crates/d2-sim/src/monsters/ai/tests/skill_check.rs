//! The monster skill check on a world that logs calls: each rule's pass and
//! fail, the masks passed to the tests, and the order of the rules.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use d2_data::tables::{Record, Skills};

use super::super::skill_check::{
    skill_check, SkillCheckWorld, LINE_MASK, LINE_MASK_203, PATTERN_MASK,
};
use crate::units::{RoomId, UnitId};

const U: UnitId = UnitId(1);
const T: UnitId = UnitId(2);
const R1: RoomId = RoomId(1);
const R2: RoomId = RoomId(2);

#[derive(Default)]
struct W {
    pos: BTreeMap<UnitId, (i32, i32)>,
    room: BTreeMap<UnitId, RoomId>,
    flag2: BTreeSet<UnitId>,
    dead: bool,
    town: BTreeSet<RoomId>,
    /// (room, unit, x, y) patterns that are NOT free.
    blocked_pattern: BTreeSet<(u32, u32, i32, i32)>,
    free_room: Option<RoomId>,
    blocked_line: bool,
    room_at: Option<RoomId>,
    diab: bool,
    log: RefCell<Vec<String>>,
}

impl SkillCheckWorld for W {
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos[&unit]
    }
    fn room(&self, unit: UnitId) -> Option<RoomId> {
        self.room.get(&unit).copied()
    }
    fn has_unit_flag(&self, unit: UnitId, bit: u32) -> bool {
        bit == 0x2 && self.flag2.contains(&unit)
    }
    fn dead_or_dying(&self, _: UnitId) -> bool {
        self.dead
    }
    fn in_town(&self, room: RoomId) -> bool {
        self.town.contains(&room)
    }
    fn pattern_free(&self, room: Option<RoomId>, unit: UnitId, x: i32, y: i32, mask: u16) -> bool {
        self.log
            .borrow_mut()
            .push(format!("pattern {room:?} {} {x} {y} {mask:#x}", unit.0));
        !self
            .blocked_pattern
            .contains(&(room.map_or(0, |r| r.0), unit.0, x, y))
    }
    fn free_point_room(
        &self,
        _: Option<RoomId>,
        _: UnitId,
        p: (i32, i32),
        mask: u16,
    ) -> Option<RoomId> {
        self.log.borrow_mut().push(format!("free {p:?} {mask:#x}"));
        self.free_room
    }
    fn line_clear(&self, from: (i32, i32), to: (i32, i32), room: RoomId, mask: u16) -> bool {
        self.log
            .borrow_mut()
            .push(format!("line {from:?} {to:?} {} {mask:#x}", room.0));
        !self.blocked_line
    }
    fn room_at(&self, _: UnitId, _: i32, _: i32) -> Option<RoomId> {
        self.room_at
    }
    fn diab_prison_ok(&self, room: Option<RoomId>, target: Option<UnitId>) -> bool {
        self.log
            .borrow_mut()
            .push(format!("diab {room:?} {target:?}"));
        self.diab
    }
}

fn world() -> W {
    let mut w = W::default();
    w.pos.insert(U, (10, 10));
    w.pos.insert(T, (14, 12));
    w.room.insert(U, R1);
    w.room.insert(T, R2);
    w.room_at = Some(R1);
    w.free_room = Some(R1);
    w.diab = true;
    w
}

/// Rows 0..=210 of zeroed skills; `set` edits the one under test.
fn skills(id: usize, set: impl FnOnce(&mut Skills)) -> Vec<Skills> {
    let mut v: Vec<Skills> = (0..=210)
        .map(|_| Skills::decode(&vec![0u8; Skills::SIZE]))
        .collect();
    set(&mut v[id]);
    v
}

fn check(w: &W, s: &[Skills], skill: i32, t: Option<UnitId>, x: i32, y: i32) -> bool {
    skill_check(w, s, U, skill, t, x, y)
}

#[test]
fn missing_row_and_skill_167_fail() {
    let w = world();
    let s = skills(5, |_| {});
    for skill in [-1, 211, 100_000] {
        assert!(!check(&w, &s, skill, Some(T), 0, 0), "{skill}");
    }
    // 167 fails although the row exists; step 9 would have said yes.
    assert!(!check(&w, &s, 167, Some(T), 0, 0));
    assert!(check(&w, &s, 5, Some(T), 0, 0));
}

#[test]
fn target_placement_check_needs_target_flag_and_free_pattern() {
    let mut w = world();
    let s = skills(9, |r| r.tgtplacecheck = true);
    assert!(!check(&w, &s, 9, None, 0, 0));
    assert!(!check(&w, &s, 9, Some(T), 0, 0), "T lacks unit flag 0x2");
    w.flag2.insert(T);
    assert!(check(&w, &s, 9, Some(T), 0, 0));
    // T's own pattern at T's position in T's room, mask 0x3C01.
    assert_eq!(
        w.log.borrow().last().unwrap(),
        &format!("pattern Some(RoomId(2)) 2 14 12 {PATTERN_MASK:#x}")
    );
    w.blocked_pattern.insert((2, 2, 14, 12));
    assert!(!check(&w, &s, 9, Some(T), 0, 0));
    // The bit decides before every other rule (skill 164 would pass on T).
    let s = skills(164, |r| r.tgtplacecheck = true);
    let mut w = world();
    assert!(!check(&w, &s, 164, Some(T), 0, 0));
    w.flag2.insert(T);
    assert!(check(&w, &s, 164, Some(T), 0, 0));
}

#[test]
fn skill_164_needs_a_target() {
    let w = world();
    let s = skills(164, |_| {});
    assert!(check(&w, &s, 164, Some(T), 0, 0));
    assert!(!check(&w, &s, 164, None, 0, 0));
}

#[test]
fn mirrored_point_skills_77_78() {
    for func in [77, 78] {
        let s = skills(40, |r| r.srvdofunc = func);
        let mut w = world();
        // P = 2 T − U = (18, 14).
        assert!(check(&w, &s, 40, Some(T), 0, 0), "func {func}");
        let log = w.log.borrow().clone();
        assert_eq!(
            log,
            [
                format!("free (18, 14) {PATTERN_MASK:#x}"),
                format!("pattern Some(RoomId(1)) 1 18 14 {PATTERN_MASK:#x}"),
                format!("line (10, 10) (18, 14) 1 {LINE_MASK:#x}"),
            ]
        );
        // No target, dying unit, no free point, town, blocked pattern at
        // the supplied P, blocked line: each fails.
        assert!(!check(&w, &s, 40, None, 0, 0));
        w.dead = true;
        assert!(!check(&w, &s, 40, Some(T), 0, 0));
        w.dead = false;
        w.free_room = None;
        assert!(!check(&w, &s, 40, Some(T), 0, 0));
        w.free_room = Some(R2);
        w.town.insert(R2);
        assert!(!check(&w, &s, 40, Some(T), 0, 0));
        w.town.clear();
        w.blocked_pattern.insert((2, 1, 18, 14));
        assert!(!check(&w, &s, 40, Some(T), 0, 0));
        w.blocked_pattern.clear();
        w.blocked_line = true;
        assert!(!check(&w, &s, 40, Some(T), 0, 0));
    }
}

#[test]
fn skill_199_is_the_diab_prison_placement_test() {
    let mut w = world();
    let s = skills(199, |_| {});
    assert!(check(&w, &s, 199, Some(T), 0, 0));
    assert_eq!(
        w.log.borrow().last().unwrap(),
        "diab Some(RoomId(2)) Some(UnitId(2))"
    );
    w.diab = false;
    assert!(!check(&w, &s, 199, Some(T), 0, 0));
}

#[test]
fn monster_teleport_184_needs_room_not_town_free_pattern_and_line() {
    let mut w = world();
    let s = skills(184, |_| {});
    assert!(check(&w, &s, 184, None, 30, 31));
    let log = w.log.borrow().clone();
    assert_eq!(
        log,
        [
            format!("pattern Some(RoomId(1)) 1 30 31 {PATTERN_MASK:#x}"),
            format!("line (10, 10) (30, 31) 1 {LINE_MASK:#x}"),
        ]
    );
    w.town.insert(R1);
    assert!(!check(&w, &s, 184, None, 30, 31));
    w.town.clear();
    w.blocked_pattern.insert((1, 1, 30, 31));
    assert!(!check(&w, &s, 184, None, 30, 31));
    w.blocked_pattern.clear();
    w.blocked_line = true;
    assert!(!check(&w, &s, 184, None, 30, 31));
    w.blocked_line = false;
    w.room_at = None;
    assert!(!check(&w, &s, 184, None, 30, 31));
}

#[test]
fn skills_67_and_203_need_room_not_town_and_a_clear_line() {
    // srvdofunc 67 with mask 0xC01; skill 203 with mask 0x805; neither
    // tests the pattern.
    for (s, id, mask) in [
        (skills(50, |r| r.srvdofunc = 67), 50, LINE_MASK),
        (skills(203, |_| {}), 203, LINE_MASK_203),
    ] {
        let mut w = world();
        assert!(check(&w, &s, id, None, 7, 8));
        assert_eq!(
            *w.log.borrow(),
            [format!("line (10, 10) (7, 8) 1 {mask:#x}")],
            "skill {id}"
        );
        w.town.insert(R1);
        assert!(!check(&w, &s, id, None, 7, 8));
        w.town.clear();
        w.blocked_line = true;
        assert!(!check(&w, &s, id, None, 7, 8));
        w.blocked_line = false;
        w.room_at = None;
        assert!(!check(&w, &s, id, None, 7, 8));
    }
}

#[test]
fn any_other_skill_passes_and_the_check_draws_nothing() {
    let mut w = world();
    w.town.insert(R1);
    w.blocked_line = true;
    let s = skills(70, |r| r.srvdofunc = 3);
    // Not 164 / 167 / 184 / 199 / 203 and srvdofunc not 67 / 77 / 78: 1,
    // whatever the world says, and the world is not even asked.
    assert!(check(&w, &s, 70, None, 0, 0));
    assert!(w.log.borrow().is_empty());
}
