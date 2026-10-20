//! Overlay create / advance tests from the rules and synthetic
//! test vectors.

use super::*;
use std::collections::{BTreeSet, VecDeque};

struct Env {
    expansion: bool,
    rows: Vec<OverlayRow>,
    blocks: bool,
    no_ovly: bool,
    unit: HeightUnit,
    frames: Option<i32>,
    rolls: VecDeque<i32>,
    asked: Vec<i32>,
    mode: i32,
    states: BTreeSet<i32>,
    aura_states: BTreeSet<i32>,
}

impl Env {
    fn new() -> Env {
        let row = OverlayRow {
            anim_rate: 16,
            ..OverlayRow::default()
        };
        Env {
            expansion: true,
            rows: vec![row; 300],
            blocks: false,
            no_ovly: false,
            unit: HeightUnit::Other,
            frames: Some(10),
            rolls: VecDeque::new(),
            asked: Vec::new(),
            mode: 0,
            states: BTreeSet::new(),
            aura_states: BTreeSet::new(),
        }
    }
}

impl OverlayEnv for Env {
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn row_count(&self) -> i32 {
        self.rows.len() as i32
    }
    fn row(&self, id: i32) -> Option<OverlayRow> {
        self.rows.get(id as usize).copied()
    }
    fn unit_blocks_overlays(&self) -> bool {
        self.blocks
    }
    fn unit_no_ovly(&self) -> bool {
        self.no_ovly
    }
    fn height_unit(&self) -> HeightUnit {
        self.unit
    }
    fn frame_count(&self, _id: i32) -> Option<i32> {
        self.frames
    }
    fn roll(&mut self, n: i32) -> i32 {
        self.asked.push(n);
        self.rolls.pop_front().unwrap_or(0)
    }
    fn mode(&self) -> i32 {
        self.mode
    }
    fn has_state(&self, s: i32) -> bool {
        self.states.contains(&s)
    }
    fn set_state(&mut self, s: i32, on: bool) {
        if on {
            self.states.insert(s);
        } else {
            self.states.remove(&s);
        }
    }
    fn has_aura(&self) -> bool {
        self.states.iter().any(|s| self.aura_states.contains(s))
    }
}

fn make(l: &mut OverlayList, e: &mut Env, id: i32, kind: u8) -> bool {
    l.create(e, id, kind, 0, 0, 0, 0, 0)
}

#[test]
fn record_is_zeroed() {
    let r = Overlay::default();
    assert_eq!(
        (r.kind, r.id, r.a, r.b, r.c, r.rate, r.frame, r.frames),
        (0, 0, 0, 0, 0, 0, 0, 0)
    );
    assert_eq!(
        (r.x, r.y, r.loop_wait, r.stamp, r.wait_start),
        (0, 0, 0, 0, 0)
    );
    assert!(!r.pre_draw && !r.active);
    assert_eq!((r.state, r.light), (0, 0));
    let r = Overlay { frame: 0x0A80, ..r };
    assert_eq!(r.frame_index(), 10);
}

#[test]
fn create_rejects_invalid_id() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    assert!(!make(&mut l, &mut e, -1, 2));
    assert!(!make(&mut l, &mut e, 300, 2));
    assert!(make(&mut l, &mut e, 299, 2));
    assert_eq!(l.records.len(), 1);
}

#[test]
fn create_classic_version_gate() {
    let mut e = Env::new();
    e.rows[5].version = 100;
    let mut l = OverlayList::new();
    e.expansion = false;
    assert!(!make(&mut l, &mut e, 5, 2));
    e.rows[5].version = 99;
    assert!(make(&mut l, &mut e, 5, 2));
    e.expansion = true;
    e.rows[6].version = 100;
    assert!(make(&mut l, &mut e, 6, 2));
}

#[test]
fn create_unit_gates() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    e.blocks = true;
    assert!(!make(&mut l, &mut e, 5, 2));
    e.blocks = false;
    e.no_ovly = true;
    assert!(!make(&mut l, &mut e, 5, 2));
    e.no_ovly = false;
    assert!(make(&mut l, &mut e, 5, 2));
}

#[test]
fn kind_8_duplicate_id() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    e.states.insert(40);
    assert!(l.create(&mut e, 5, 8, 0, 40, 0, 0, 0));
    assert!(!l.create(&mut e, 5, 8, 0, 40, 0, 0, 0));
    assert_eq!(l.records.len(), 1);
}

#[test]
fn create_replaces_same_id_except_stacking() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    make(&mut l, &mut e, 5, 2);
    assert_eq!(l.records.len(), 1);
    // Test vector: create id 140 twice -> two records.
    make(&mut l, &mut e, 140, 2);
    make(&mut l, &mut e, 140, 2);
    assert_eq!(l.records.iter().filter(|r| r.id == 140).count(), 2);
    for id in [141, 142, 158] {
        make(&mut l, &mut e, id, 2);
        make(&mut l, &mut e, id, 2);
        assert_eq!(l.records.iter().filter(|r| r.id == id).count(), 2);
    }
    // A kind 4 record naming id 7 is removed by a create of id 7.
    l.create(&mut e, 20, 4, 0, 7, 8, 0, 0);
    make(&mut l, &mut e, 7, 2);
    assert!(l.records.iter().all(|r| r.id != 20));
}

#[test]
fn frame_count_from_cel() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    assert_eq!(l.records[0].frames, 2560);
    // The new record is the list head.
    make(&mut l, &mut e, 6, 2);
    assert_eq!(l.records[0].id, 6);
}

#[test]
fn args_by_kind() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    l.create(&mut e, 1, 2, 0, 7, 8, 9, 0);
    assert_eq!((l.records[0].a, l.records[0].b, l.records[0].c), (1, 0, 0));
    l.create(&mut e, 2, 3, 0, 7, 8, 9, 0);
    assert_eq!((l.records[0].a, l.records[0].b, l.records[0].c), (7, 8, 9));
    // Kind 6: frame := roll(frames), A stays 0.
    e.rolls.push_back(1234);
    l.create(&mut e, 3, 6, 0, 7, 8, 9, 0);
    assert_eq!(e.asked.last(), Some(&2560));
    assert_eq!((l.records[0].frame, l.records[0].a), (1234, 0));
}

#[test]
fn offsets_and_heights() {
    let mut e = Env::new();
    e.rows[5] = OverlayRow {
        xoffset: 3,
        yoffset: 10,
        height: [100, 200, 300, 400],
        ..OverlayRow::default()
    };
    let y = |e: &mut Env| {
        let mut l = OverlayList::new();
        make(&mut l, e, 5, 2);
        assert_eq!(l.records[0].x, 3);
        l.records[0].y
    };
    e.unit = HeightUnit::Player;
    assert_eq!(y(&mut e), 210);
    e.unit = HeightUnit::Monster {
        overlay_height: Some(0),
    };
    assert_eq!(y(&mut e), 85);
    for h in 1..=4u8 {
        e.unit = HeightUnit::Monster {
            overlay_height: Some(h),
        };
        assert_eq!(y(&mut e), 10 + 100 * i32::from(h));
    }
    e.unit = HeightUnit::Monster {
        overlay_height: Some(5),
    };
    assert_eq!(y(&mut e), 10);
    e.unit = HeightUnit::Monster {
        overlay_height: None,
    };
    assert_eq!(y(&mut e), 110);
    e.unit = HeightUnit::Other;
    assert_eq!(y(&mut e), 110);
}

#[test]
fn kind_8_state_and_sync() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    e.states.extend([30, 40]);
    // First kind-8 record: active, A = 0.
    l.create(&mut e, 5, 8, 0, 30, 0, 0, 0);
    assert_eq!(
        (l.records[0].state, l.records[0].active, l.records[0].a),
        (30, true, 0)
    );
    // Test vector: state 40 with state 30 active -> new +0x3C = 0.
    l.create(&mut e, 6, 8, 0, 40, 0, 0, 0);
    assert_eq!((l.records[0].state, l.records[0].active), (40, false));
    // Same state as an existing record: copy its flag and counter.
    l.records[1].a = 17;
    l.create(&mut e, 7, 8, 0, 30, 0, 0, 0);
    assert_eq!((l.records[0].active, l.records[0].a), (true, 17));
}

#[test]
fn start_frame_roll() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    e.rolls.push_back(777);
    l.create(&mut e, 5, 2, 4, 0, 0, 0, 0);
    assert_eq!(e.asked, vec![1024]);
    assert_eq!(l.records[0].frame, 777);
}

#[test]
fn rate_and_roll() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    assert_eq!(l.records[0].rate, 256);
    e.rolls.push_back(5);
    l.create(&mut e, 6, 2, 0, 0, 0, 0, 3);
    assert_eq!(e.asked, vec![48]);
    assert_eq!(l.records[0].rate, 261);
}

#[test]
fn loop_wait_and_predraw() {
    let mut e = Env::new();
    e.rows[5].loop_wait_time = 900;
    e.rows[5].pre_draw = true;
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    assert_eq!(l.records[0].loop_wait, 900);
    assert!(l.records[0].pre_draw);
}

#[test]
fn rolls_in_draw_order() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    // kind 6, a, b: three draws in order.
    e.rolls.extend([1, 2, 3]);
    l.create(&mut e, 5, 6, 2, 0, 0, 0, 1);
    assert_eq!(e.asked, vec![2560, 512, 16]);
    assert_eq!(l.records[0].frame, 2);
    assert_eq!(l.records[0].rate, 256 + 3);
    // A create that stops at rules 1-5 draws nothing.
    let n = e.asked.len();
    assert!(!l.create(&mut e, -1, 6, 2, 0, 0, 0, 1));
    e.blocks = true;
    assert!(!l.create(&mut e, 5, 6, 2, 0, 0, 0, 1));
    assert_eq!(e.asked.len(), n);
    // Follow-up creates draw nothing.
    e.blocks = false;
    let mut l = OverlayList::new();
    l.create(&mut e, 9, 4, 0, 5, -1, 0, 0);
    l.records[0].frame = l.records[0].frames;
    let n = e.asked.len();
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(e.asked.len(), n);
}

#[test]
fn walk_runs_each_record_once_per_update() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 3);
    make(&mut l, &mut e, 6, 3);
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert!(l.records.iter().all(|r| r.stamp == 1 && r.frame == 256));
    // The same update number again runs nothing.
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert!(l.records.iter().all(|r| r.frame == 256));
    l.advance(&mut e, 2, 0, TickRate::BASE);
    assert!(l.records.iter().all(|r| r.frame == 512));
    // A record created during the walk (stamp 0) runs in the same update:
    // kind 4 ends, creating kind 3 overlay 7 which is advanced at once.
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 4, 0, 7, -1, 0, 0);
    l.records[0].frame = l.records[0].frames - 256;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(l.records.len(), 1);
    assert_eq!(
        (l.records[0].id, l.records[0].kind, l.records[0].frame),
        (7, 3, 256)
    );
}

#[test]
fn keep_tests_by_kind() {
    let mut e = Env::new();
    // Kind 0: kept while the mode equals A.
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 0, 0, 4, 0, 0, 0);
    e.mode = 4;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(l.records.len(), 1);
    e.mode = 5;
    l.advance(&mut e, 2, 0, TickRate::BASE);
    assert!(l.records.is_empty());
    // Test vector: kind 1, A = 3 kept on updates 1-3 (A 2, 1, 0), removed on 4.
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 1, 0, 3, 0, 0, 0);
    for (u, a) in [(1, 2), (2, 1), (3, 0)] {
        l.advance(&mut e, u, 0, TickRate::BASE);
        assert_eq!(l.records[0].a, a);
    }
    l.advance(&mut e, 4, 0, TickRate::BASE);
    assert!(l.records.is_empty());
    // Kind 2: A = 0 removes.
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    l.records[0].a = 0;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert!(l.records.is_empty());
    // Kind 6 waiting: skipped until LoopWaitTime passed, no advance.
    let mut l = OverlayList::new();
    e.rows[5].loop_wait_time = 500;
    l.create(&mut e, 5, 6, 0, 0, 0, 0, 0);
    l.records[0].wait_start = 1000;
    l.advance(&mut e, 1, 1499, TickRate::BASE);
    assert_eq!((l.records[0].a, l.records[0].frame), (0, 0));
    l.advance(&mut e, 2, 1500, TickRate::BASE);
    assert_eq!((l.records[0].a, l.records[0].frame), (1, 256));
    // Kind 8: the state ends -> removed; active and A < 50 -> A += 1.
    let mut l = OverlayList::new();
    e.states.insert(30);
    l.create(&mut e, 5, 8, 0, 30, 0, 0, 0);
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(l.records[0].a, 1);
    e.states.remove(&30);
    l.advance(&mut e, 2, 0, TickRate::BASE);
    assert!(l.records.is_empty());
    // Kinds 3, 4, 5, 7, 9 are always kept.
    for k in [3u8, 4, 5, 7, 9] {
        let mut l = OverlayList::new();
        l.create(&mut e, 5, k, 0, 1, 2, 3, 0);
        l.advance(&mut e, 1, 0, TickRate::BASE);
        assert_eq!(l.records.len(), 1, "kind {k}");
    }
}

#[test]
fn advance_adds_rate() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 3);
    l.advance(&mut e, 1, 0, TickRate::BASE);
    l.advance(&mut e, 2, 0, TickRate::BASE);
    assert_eq!(l.records[0].frame, 512);
    assert_eq!(l.records[0].frame_index(), 2);
}

#[test]
fn end_of_frames_by_kind() {
    let mut e = Env::new();
    // Test vector: kind 2, AnimRate 16, 10 frames: +0x1C 2560, +0x14 256;
    // update 10: frame 2560, A := 0, not drawn; update 11: removed.
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    for u in 1..=9 {
        l.advance(&mut e, u, 0, TickRate::BASE);
        assert_eq!(l.records[0].frame_index(), u as i32);
    }
    l.advance(&mut e, 10, 0, TickRate::BASE);
    assert_eq!((l.records[0].frame, l.records[0].a), (2560, 0));
    l.advance(&mut e, 11, 0, TickRate::BASE);
    assert!(l.records.is_empty());
    // Kinds 1, 3, 8 wrap; 0 runs past; 5 holds the last frame; 6 wraps and
    // waits.
    for (k, wraps) in [(3u8, true), (0, false)] {
        let mut l = OverlayList::new();
        l.create(&mut e, 5, k, 0, 0, 0, 0, 0);
        e.mode = 0;
        l.records[0].frame = 2560 - 256;
        l.advance(&mut e, 1, 0, TickRate::BASE);
        assert_eq!(l.records[0].frame, if wraps { 0 } else { 2560 }, "kind {k}");
    }
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 5, 0, 0, 0, 0, 0);
    l.records[0].frame = 2560 - 256;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    let r = l.records[0];
    assert_eq!((r.rate, r.frame, r.a), (0, 2559, 1));
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 6, 0, 0, 0, 0, 0);
    l.records[0].frame = 2560 - 256;
    l.advance(&mut e, 1, 777, TickRate::BASE);
    let r = l.records[0];
    assert_eq!((r.frame, r.a, r.wait_start), (0, 0, 777));
    // Kind 7: removed, kind 4 overlay 140 with A = 141, B = 142.
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 7, 0, 0, 0, 0, 0);
    l.records[0].frame = 2560 - 256;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(l.records.len(), 1);
    let r = l.records[0];
    assert_eq!((r.id, r.kind, r.a, r.b), (140, 4, 141, 142));
    // Kind 4 with A = 5, B = -1: one kind 3 overlay 5.
    let mut l = OverlayList::new();
    l.create(&mut e, 9, 4, 0, 5, -1, 0, 0);
    l.records[0].frame = 2560 - 256;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(l.records.len(), 1);
    assert_eq!((l.records[0].id, l.records[0].kind), (5, 3));
    // Kind 9: follow-ups only while U has state C.
    let mut l = OverlayList::new();
    e.states.insert(30);
    l.create(&mut e, 9, 9, 0, 5, 6, 30, 0);
    l.records[0].frame = 2560 - 256;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    let ids: Vec<(i32, u8, i32)> = l.records.iter().map(|r| (r.id, r.kind, r.state)).collect();
    assert_eq!(ids, vec![(6, 8, 30), (5, 8, 30)]);
    let mut l = OverlayList::new();
    e.states.clear();
    l.create(&mut e, 9, 9, 0, 5, 6, 30, 0);
    l.records[0].frame = 2560 - 256;
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert!(l.records.is_empty());
}

#[test]
fn kind_8_cycle() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    e.states.extend([10, 20, 30]);
    e.aura_states.insert(10);
    for s in [10, 20, 30] {
        l.create(&mut e, 100 + s, 8, 0, s, 0, 0, 0);
    }
    let act = |l: &OverlayList| -> Vec<i32> {
        let mut v: Vec<i32> = l
            .records
            .iter()
            .filter(|r| r.active)
            .map(|r| r.state)
            .collect();
        v.sort();
        v
    };
    assert_eq!(act(&l), vec![10]);
    // Run the active record up to 50, then the step moves to the next state.
    for u in 1..=50 {
        l.advance(&mut e, u, 0, TickRate::BASE);
    }
    assert_eq!(l.records.iter().find(|r| r.state == 10).unwrap().a, 50);
    l.advance(&mut e, 51, 0, TickRate::BASE);
    assert_eq!(act(&l), vec![20]);
    for u in 52..=102 {
        l.advance(&mut e, u, 0, TickRate::BASE);
    }
    assert_eq!(act(&l), vec![30]);
    // From the largest state the cycle wraps to the smallest.
    for u in 103..=153 {
        l.advance(&mut e, u, 0, TickRate::BASE);
    }
    assert_eq!(act(&l), vec![10]);
    // Removing the active kind-8 record steps the cycle to the next state.
    let i = l.records.iter().position(|r| r.state == 10).unwrap();
    l.remove_at(&mut e, i);
    assert_eq!(act(&l), vec![20]);
    // State 0 cannot be chosen.
    let mut l = OverlayList::new();
    e.states.insert(0);
    l.create(&mut e, 121, 8, 0, 10, 0, 0, 0);
    l.create(&mut e, 120, 8, 0, 0, 0, 0, 0);
    assert!(!l.records[0].active);
    let i = l.records.iter().position(|r| r.state == 10).unwrap();
    l.remove_at(&mut e, i);
    assert!(l.records.iter().all(|r| !r.active));
    // Aura-less state: the step with no aura state left sets A := 0 on the
    // reaching record, and the state is turned back on.
    let mut l = OverlayList::new();
    let mut e2 = Env::new();
    e2.states.insert(20);
    l.create(&mut e2, 5, 8, 0, 20, 0, 0, 0);
    l.records[0].a = 50;
    l.advance(&mut e2, 1, 0, TickRate::BASE);
    assert!(e2.states.contains(&20));
    assert_eq!(l.records[0].a, 0);
    assert!(l.records[0].active);
}

#[test]
fn removal_of_one_record() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 3);
    make(&mut l, &mut e, 6, 3);
    assert!(l.remove_at(&mut e, 5).is_none());
    assert_eq!(l.remove_at(&mut e, 0).unwrap().id, 6);
    assert_eq!(l.records.len(), 1);
}

#[test]
fn remove_by_id_rules() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    assert_eq!(l.remove_by_id(&mut e, 5), Ok(false));
    assert_eq!(
        l.remove_by_id(&mut e, 999),
        Err(OverlayError::InvalidId(999))
    );
    assert_eq!(l.remove_by_id(&mut e, -1), Err(OverlayError::InvalidId(-1)));
    make(&mut l, &mut e, 5, 3);
    l.create(&mut e, 9, 4, 0, 11, 12, 0, 0);
    l.create(&mut e, 10, 9, 0, 13, 14, 0, 0);
    assert_eq!(l.remove_by_id(&mut e, 12), Ok(true));
    assert_eq!(l.remove_by_id(&mut e, 13), Ok(true));
    assert_eq!(l.records.len(), 1);
    assert_eq!(l.remove_by_id(&mut e, 5), Ok(true));
    // Only kinds 4 and 9 match on A / B.
    l.create(&mut e, 9, 3, 0, 11, 0, 0, 0);
    assert_eq!(l.remove_by_id(&mut e, 11), Ok(false));
    // The first record in list order wins.
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 3);
    l.create(&mut e, 20, 4, 0, 5, 0, 0, 0);
    assert_eq!(l.remove_by_id(&mut e, 5), Ok(true));
    assert_eq!(l.records[0].id, 5);
}

#[test]
fn remove_all_records() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    for id in [5, 6, 140, 140] {
        make(&mut l, &mut e, id, 3);
    }
    l.remove_all(&mut e);
    assert!(l.records.is_empty());
}

#[test]
fn set_countdown_of_kind_1() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 1, 0, 3, 0, 0, 0);
    assert_eq!(l.set_countdown(5, 9), Ok(()));
    assert_eq!(l.records[0].a, 9);
    assert_eq!(l.set_countdown(6, 1), Ok(()));
    make(&mut l, &mut e, 7, 3);
    assert_eq!(l.set_countdown(7, 1), Err(OverlayError::WrongKind(7)));
}

#[test]
fn unloaded_file_never_draws() {
    let mut e = Env::new();
    e.frames = None;
    let mut l = OverlayList::new();
    make(&mut l, &mut e, 5, 2);
    assert_eq!(l.records[0].frames, 1);
    // It lives by its kind's rule: kind 2 ends after the first advance
    // (rate 256 >= 1) and is removed on the next.
    l.advance(&mut e, 1, 0, TickRate::BASE);
    assert_eq!(l.records[0].a, 0);
    l.advance(&mut e, 2, 0, TickRate::BASE);
    assert!(l.records.is_empty());
}

#[test]
fn kind_0_does_not_wrap() {
    let mut e = Env::new();
    let mut l = OverlayList::new();
    l.create(&mut e, 5, 0, 0, 0, 0, 0, 0);
    for u in 1..=30 {
        l.advance(&mut e, u, 0, TickRate::BASE);
    }
    // After one play the record stays, past the end, until the mode changes.
    assert_eq!(l.records.len(), 1);
    assert_eq!(l.records[0].frame, 30 * 256);
    e.mode = 1;
    l.advance(&mut e, 31, 0, TickRate::BASE);
    assert!(l.records.is_empty());
}

/// Row rates and the kind-8 cycle are stated per base-rate tick: a
/// looping record plays the same frames a second, and an aura cycle
/// holds a state the same 2 s, at 25 Hz and at the game's 60 Hz (F5).
#[test]
fn records_play_in_real_time_at_every_rate() {
    for hz in [25u32, 60] {
        let rate = TickRate::new(hz).unwrap();
        let mut e = Env::new();
        let mut l = OverlayList::new();
        // 10 frames at one a base tick: 25 frames a second, 2 wraps.
        make(&mut l, &mut e, 5, 3);
        for u in 1..=hz {
            l.advance(&mut e, u, 0, rate);
        }
        assert_eq!(l.records[0].frame, (25 - 20) * 256, "{hz} Hz");
        // The kind-8 cycle moves to the next state after 2 s.
        let mut e = Env::new();
        let mut l = OverlayList::new();
        e.states.extend([10, 20]);
        e.aura_states.insert(10);
        for s in [10, 20] {
            l.create(&mut e, 100 + s, 8, 0, s, 0, 0, 0);
        }
        let active = |l: &OverlayList| l.records.iter().find(|r| r.active).map(|r| r.state);
        assert_eq!(active(&l), Some(10));
        let mut u = 0;
        while active(&l) == Some(10) {
            u += 1;
            l.advance(&mut e, u, 0, rate);
            assert!(u < 1000, "the cycle moves");
        }
        assert_eq!(u, 2 * hz + 1, "{hz} Hz cycle");
    }
}
