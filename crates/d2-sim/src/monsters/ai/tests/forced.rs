//! Forced targets on a world that logs calls.

use std::collections::BTreeMap;

use super::super::forced::{confused_alignment, forced_target, ForcedWorld};
use crate::units::{UnitId, UnitType};

const ME: UnitId = UnitId(1);

#[derive(Default)]
struct W {
    ov: Option<(u32, u32)>,
    by_guid: BTreeMap<(u8, u32), UnitId>,
    types: BTreeMap<UnitId, UnitType>,
    dead: Vec<UnitId>,
    dist: i32,
    blocked: bool,
    align: u8,
    bit: u32,
    scan_result: Option<(UnitId, i32)>,
    log: Vec<String>,
}

fn ty_id(t: UnitType) -> u8 {
    match t {
        UnitType::Player => 0,
        UnitType::Monster => 1,
        UnitType::Missile => 3,
        _ => 9,
    }
}

impl ForcedWorld for W {
    fn override_of(&self, _: UnitId) -> Option<(u32, u32)> {
        self.ov
    }
    fn clear_override(&mut self, _: UnitId) {
        self.log.push("clear".into());
        self.ov = self.ov.map(|_| (0, 0));
    }
    fn find_by_guid(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.by_guid.get(&(ty_id(ty), guid)).copied()
    }
    fn full_distance(&self, _: UnitId, _: UnitId) -> i32 {
        self.dist
    }
    fn line_blocked(&self, _: UnitId, _: UnitId) -> bool {
        self.blocked
    }
    fn alignment(&self, _: UnitId) -> u8 {
        self.align
    }
    fn set_alignment(&mut self, _: UnitId, v: u8) {
        self.log.push(format!("align {v}"));
        self.align = v;
    }
    fn seed_bit(&mut self, _: UnitId) -> u32 {
        self.log.push("draw".into());
        self.bit
    }
    fn scan(&mut self, _: UnitId, scan: u8, a: bool) -> Option<(UnitId, i32)> {
        self.log.push(format!("scan {scan} {a}"));
        self.scan_result
    }
    fn unit_type(&self, u: UnitId) -> Option<UnitType> {
        self.types.get(&u).copied()
    }
    fn is_dead(&self, u: UnitId) -> bool {
        self.dead.contains(&u)
    }
}

fn world(k: u32, g: u32) -> W {
    W {
        ov: Some((k, g)),
        dist: 12,
        ..W::default()
    }
}

#[test]
fn override_kinds_and_clear() {
    // The override is the kind / GUID pair; every failure clears it.
    let mut w = world(1, 77);
    assert_eq!(forced_target(&mut w, ME, false, false), None);
    assert_eq!(w.ov, Some((0, 0)));
}

#[test]
fn non_monster_or_kind_zero_returns_nothing() {
    let mut w = W::default();
    assert_eq!(forced_target(&mut w, ME, false, false), None);
    assert!(w.log.is_empty());
    let mut w = world(0, 5);
    assert_eq!(forced_target(&mut w, ME, false, false), None);
    assert!(w.log.is_empty(), "kind 0 changes nothing");
}

#[test]
fn kinds_1_2_4_look_up_the_guid_and_measure_the_distance() {
    for (k, ty) in [
        (1, UnitType::Player),
        (2, UnitType::Monster),
        (4, UnitType::Missile),
    ] {
        let mut w = world(k, 77);
        let u = UnitId(9);
        w.by_guid.insert((ty_id(ty), 77), u);
        w.types.insert(u, ty);
        assert_eq!(
            forced_target(&mut w, ME, false, false),
            Some((u, 12)),
            "k {k}"
        );
        assert!(w.log.is_empty());
        // The GUID of another type does not match.
        let mut w = world(k, 77);
        let other = if k == 1 {
            UnitType::Monster
        } else {
            UnitType::Player
        };
        w.by_guid.insert((ty_id(other), 77), u);
        w.types.insert(u, other);
        assert_eq!(forced_target(&mut w, ME, false, false), None);
        assert_eq!(w.log, ["clear"]);
    }
    // s ≠ 0 and a blocked line: cleared; s = 0 ignores the line.
    let mut w = world(1, 77);
    let u = UnitId(9);
    w.by_guid.insert((0, 77), u);
    w.types.insert(u, UnitType::Player);
    w.blocked = true;
    assert_eq!(forced_target(&mut w, ME, false, false), Some((u, 12)));
    assert_eq!(forced_target(&mut w, ME, false, true), None);
    assert_eq!(w.log, ["clear"]);
}

#[test]
fn kind_3_confuses_the_alignment_for_one_scan() {
    // Table of step 3: (A, r) → temporary alignment.
    for (a, r, want) in [
        (1, 1, 2),
        (1, 0, 0),
        (0, 1, 2),
        (0, 0, 0),
        (2, 1, 0),
        (2, 0, 2),
        (3, 1, 3),
    ] {
        assert_eq!(confused_alignment(a, r), want, "A {a} r {r}");
    }
    let p = UnitId(9);
    for (s, a, scan) in [(false, true, 5u8), (true, false, 6)] {
        let mut w = world(3, 0);
        w.align = 1;
        w.bit = 1;
        w.types.insert(p, UnitType::Player);
        w.scan_result = Some((p, 21));
        assert_eq!(forced_target(&mut w, ME, a, s), Some((p, 21)));
        // One draw, the temporary alignment, the scan, the restore.
        assert_eq!(
            w.log,
            [
                "draw".to_string(),
                "align 2".into(),
                format!("scan {scan} {a}"),
                "align 1".into()
            ]
        );
        assert_eq!(w.align, 1);
    }
}

#[test]
fn only_living_players_monsters_and_missiles_are_accepted() {
    let u = UnitId(9);
    for (ty, dead, ok) in [
        (UnitType::Player, false, true),
        (UnitType::Monster, false, true),
        (UnitType::Missile, false, true),
        (UnitType::Player, true, false),
        (UnitType::Monster, true, false),
        (UnitType::Object, false, false),
        (UnitType::Item, false, false),
        (UnitType::Tile, false, false),
    ] {
        let mut w = world(3, 0);
        w.types.insert(u, ty);
        if dead {
            w.dead.push(u);
        }
        w.scan_result = Some((u, 5));
        let got = forced_target(&mut w, ME, false, false);
        assert_eq!(got, ok.then_some((u, 5)), "{ty:?} dead {dead}");
        assert_eq!(w.ov == Some((0, 0)), !ok, "cleared exactly on failure");
    }
    // Nothing found by the scan: cleared too.
    let mut w = world(3, 0);
    assert_eq!(forced_target(&mut w, ME, false, false), None);
    assert_eq!(w.ov, Some((0, 0)));
}
