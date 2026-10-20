//! Tests for waypoint behaviours the main tests in `tests` don't reach: new
//! records, which waypoints get activated, the arrival list's lifetime,
//! towns and the arrival message offset.

#[allow(unused_imports)]
use super::*;

use super::tests::{data, fake, hex, msg49, rec, P, ROOM_DEST};

#[test]
fn rogue_encampment_always_known() {
    // Allocation, every difficulty.
    let recs = WaypointRecords::default();
    for d in 0..3 {
        assert!(recs.0[d].test(0).unwrap());
    }
    // After every load copy: valid magic with bit 0 clear, and the wipe.
    for src in [
        "0201 0000 0000 0000 0000 0000 0000 0000",
        "0201 0200 0000 0000 0000 0000 0000 0000",
        "0000 0000 0000 0000 0000 0000 0000 0000",
        "0101 fe00 0000 0000 0000 0000 0000 0000",
    ] {
        let r = WaypointRecord::load_copy(&rec(src)).unwrap();
        assert!(r.test(0).unwrap(), "{src}");
    }
    // Through a save section with no bits at all.
    let mut s = [0u8; SAVE_SECTION_LEN];
    s[0..2].copy_from_slice(b"WS");
    let recs = read_section(&s).unwrap();
    assert!(recs.0.iter().all(|r| r.test(0).unwrap()));
}

#[test]
fn activated_index_follows_the_room_level_not_the_class() {
    let d = data();
    let set_by = |class: u16, level: u32| {
        let mut f = fake();
        let (u, mut o) = f.objects[0];
        (o.class, o.level, o.mode) = (class, Some(level), 3);
        d.operate(&mut f, u, &o, P).unwrap();
        let r = f.records[&P].0[0];
        (0..39)
            .filter(|&n| r.test(n).unwrap())
            .collect::<Vec<u32>>()
    };
    // Level 3 (Cold Plains) is index 1, level 4 index 2.
    assert_eq!(set_by(119, 3), [0, 1]);
    assert_eq!(set_by(494, 3), [0, 1]);
    assert_eq!(set_by(119, 4), [0, 2]);
    // A level without a waypoint sets nothing, whatever the class.
    assert_eq!(set_by(119, 2), [0]);
}

#[test]
fn arrival_list_written_by_travel_read_by_init() {
    let d = data();
    let mut arr = ArrivalList::default();
    assert!(arr.0.is_empty());
    // Operate, validate and close leave it alone.
    let mut f = fake();
    let (u, o) = f.objects[0];
    d.operate(&mut f, u, &o, P).unwrap();
    d.validate(&mut f, P, 0x0b, 0).unwrap();
    d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 0))
        .unwrap();
    assert!(arr.0.is_empty());
    // Travel writes one node.
    let mut f = fake();
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    f.land = Some((Some(ROOM_DEST), 300, 400, 3));
    f.spawn = Some(ROOM_DEST);
    d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 3))
        .unwrap();
    assert_eq!(arr.0.len(), 1);
    // Init 17 of an object in that room consumes it.
    let mut f = fake();
    let (u, mut o) = f.objects[0];
    (o.room, o.level, o.mode) = (Some(ROOM_DEST), Some(3), 0);
    d.init_object(&mut f, &mut arr, u, &o);
    assert!(arr.0.is_empty());
    assert_eq!(f.log[0], "mode 50 1");
}

#[test]
fn operate_queues_only_the_menu() {
    let d = data();
    let mut f = fake();
    let (u, o) = f.objects[0];
    d.operate(&mut f, u, &o, P).unwrap();
    assert_eq!(f.sent.len(), 1);
    assert_eq!(f.sent[0][0], 0x63);
    assert_eq!(f.log, ["interact 2 0xb"]);
}

#[test]
fn travel_never_sets_town_bits() {
    let d = data();
    // Lut Gholein (level 40, index 9) unknown: refused, nothing set.
    let mut f = fake();
    let before = f.records[&P];
    let mut arr = ArrivalList::default();
    assert_eq!(
        d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 40)),
        Ok(2)
    );
    assert_eq!(f.records[&P], before);
    assert!(!f.log.iter().any(|l| l.starts_with("warp")));
    // A successful travel changes no record either.
    let mut f = fake();
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    let before = f.records[&P];
    f.land = Some((Some(ROOM_DEST), 300, 400, 3));
    f.spawn = Some(ROOM_DEST);
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 3)), Ok(0));
    assert_eq!(f.records[&P], before);
    // Init 17 in a town sets no bit.
    let mut f = fake();
    let (u, mut o) = f.objects[0];
    (o.level, o.mode) = (Some(40), 0);
    d.init_object(&mut f, &mut ArrivalList::default(), u, &o);
    assert_eq!(f.records[&P], WaypointRecords::default());
}

#[test]
fn arrival_message_is_offset_by_three() {
    let d = data();
    let mut f = fake();
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    f.land = Some((Some(ROOM_DEST), 0x0100, 0x0200, 3));
    f.spawn = Some(ROOM_DEST);
    let mut arr = ArrivalList::default();
    d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 3))
        .unwrap();
    assert_eq!(f.sent, [hex("0d 00 01000000 01 0301 0302 00 00")]);
    // The player and the arrival node keep the true position.
    let p = f.player.unwrap();
    assert_eq!((p.x, p.y), (0x0100, 0x0200));
    assert_eq!((arr.0[0].x, arr.0[0].y), (0x0100, 0x0200));
}
