//! Tests for [`super`]: the index ↔ level table checked against
//! `waypoints.tsv`, record bit vectors, save section read and write, object
//! operate and init, the 0x49 validation and travel against a fake world.

use std::collections::BTreeMap;

use super::*;
use crate::world::{tsv_num, tsv_rows, TsvError};

pub(super) const WAYPOINTS_TSV: &str = include_str!("../../../../../tables/world/waypoints.tsv");
const HEADER: &[&str] = &["wp", "level", "act", "town", "tile_calc", "level_name"];

/// (wp, level, act, town, tile_calc) per row.
type Row = (u8, u32, u8, bool, u8);

pub(super) fn parse_tsv(text: &str) -> Result<Vec<Row>, TsvError> {
    let t = "waypoints.tsv";
    tsv_rows(t, text, HEADER)?
        .into_iter()
        .map(|(line, c)| {
            Ok((
                tsv_num(t, line, "wp", c[0])? as u8,
                tsv_num(t, line, "level", c[1])?,
                tsv_num(t, line, "act", c[2])? as u8,
                tsv_num(t, line, "town", c[3])? != 0,
                tsv_num(t, line, "tile_calc", c[4])? as u8,
            ))
        })
        .collect()
}

/// A levels table holding exactly the TSV's waypoint levels (act from the
/// TSV, 255 elsewhere).
pub(super) fn map_from_rows(rows: &[Row]) -> WaypointMap {
    let n = rows.iter().map(|r| r.1).max().unwrap() as usize + 1;
    let mut levels = vec![(NO_WAYPOINT, 0u8); n.max(150)];
    for r in rows {
        levels[r.1 as usize] = (r.0, r.2);
    }
    WaypointMap::from_pairs(levels)
}

/// The checker: every TSV row against the derivation; one line per
/// difference.
fn check(tsv: &[Row], map: &WaypointMap) -> Vec<String> {
    let got = map.rows();
    let mut out = Vec::new();
    for (i, want) in tsv.iter().enumerate() {
        match got.get(i) {
            Some(g) if g == want => {}
            g => out.push(format!("row {i}: tsv {want:?}, derived {g:?}")),
        }
    }
    if got.len() != tsv.len() {
        out.push(format!("{} rows derived, {} in tsv", got.len(), tsv.len()));
    }
    out
}

#[test]
fn tsv_matches_town_and_tile_rules() {
    let rows = parse_tsv(WAYPOINTS_TSV).unwrap();
    assert_eq!(rows.len(), 39);
    // Indexes 0..38, each once.
    assert!(rows.iter().enumerate().all(|(i, r)| r.0 as usize == i));
    assert_eq!(check(&rows, &map_from_rows(&rows)), Vec::<String>::new());
    // Act ranges.
    for r in &rows {
        let want = match r.0 {
            0..=8 => 0,
            9..=17 => 1,
            18..=26 => 2,
            27..=29 => 3,
            _ => 4,
        };
        assert_eq!(r.2, want, "index {}", r.0);
    }
}

/// The checker reports exactly a changed cell.
#[test]
fn tsv_check_catches_perturbations() {
    let rows = parse_tsv(WAYPOINTS_TSV).unwrap();
    let map = map_from_rows(&rows);
    let mut bad = rows.clone();
    bad[1].4 = 13; // Cold Plains tile code
    let errs = check(&bad, &map);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].starts_with("row 1:"), "{errs:?}");
    let mut bad = rows.clone();
    bad[5].3 = true; // a non-town marked town
    assert_eq!(check(&bad, &map).len(), 1);
    let text = WAYPOINTS_TSV.replacen("tile_calc", "tile", 1);
    assert!(matches!(parse_tsv(&text), Err(TsvError::Header { .. })));
    let text = WAYPOINTS_TSV.replacen("\t13\t", "\tx\t", 1);
    assert!(matches!(parse_tsv(&text), Err(TsvError::Value { .. })));
}

#[test]
fn index_mapping() {
    // Index order is not level order.
    let rows = parse_tsv(WAYPOINTS_TSV).unwrap();
    let map = map_from_rows(&rows);
    assert_eq!(map.index_of_level(48), Some(10));
    assert_eq!(map.index_of_level(42), Some(11));
    assert_eq!(map.level_of_index(10), Some(48));
    assert_eq!(map.index_of_level(2), None);
    assert_eq!(map.index_of_level(10_000), None);
    assert_eq!(map.level_of_index(255), None);
    // First match wins in level order.
    let m = WaypointMap::from_pairs(vec![(255, 0), (3, 0), (3, 0)]);
    assert_eq!(m.level_of_index(3), Some(1));
    for t in TOWNS {
        assert_eq!(tile_code(t), 13);
    }
    assert_eq!(tile_code(46), 13);
    assert_eq!(tile_code(3), 0);
}

pub(super) fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

pub(super) fn rec(s: &str) -> [u8; 16] {
    hex(s).try_into().unwrap()
}

#[test]
fn record_vectors() {
    let mut r = WaypointRecord::allocate();
    assert_eq!(r.0, rec("0201 0100 0000 0000 0000 0000 0000 0000"));
    r.set(2).unwrap();
    assert_eq!(r.0, rec("0201 0500 0000 0000 0000 0000 0000 0000"));
    let mut r = WaypointRecord::allocate();
    for n in [0, 9, 18, 27, 30] {
        r.set(n).unwrap();
    }
    assert_eq!(r.0, rec("0201 0102 0448 0000 0000 0000 0000 0000"));
    let mut r = WaypointRecord::allocate();
    for n in 0..39 {
        r.set(n).unwrap();
    }
    assert_eq!(r.0, rec("0201 ffff ffff 7f00 0000 0000 0000 0000"));
    assert!(r.test(38).unwrap() && !r.test(39).unwrap());
    // An index outside the flag table is an error.
    assert_eq!(r.set(0x70), Err(WaypointError::IndexOutOfRange(0x70)));
    assert_eq!(r.test(0x70), Err(WaypointError::IndexOutOfRange(0x70)));
    assert!(r.set(0x6F).is_ok());
}

#[test]
fn load_and_out_copy() {
    // The load wipe.
    let mut r = WaypointRecord::load_copy(&rec("0101 0300 0000 0000 0000 0000 0000 0000")).unwrap();
    assert_eq!(r.0, rec("0000 0100 0000 0000 0000 0000 0000 0000"));
    assert_eq!(
        r.out_copy().unwrap(),
        rec("0201 0100 0000 0000 0000 0000 0000 0000")
    );
    assert_eq!(r.magic(), MAGIC); // the source is normalised
    let r = WaypointRecord::load_copy(&rec("0201 0300 0000 0000 0000 0000 0000 0000")).unwrap();
    assert_eq!(r.0, rec("0201 0300 0000 0000 0000 0000 0000 0000"));
    let r = WaypointRecord::load_copy(&rec("0000 0200 0000 0000 0000 0000 0000 0000")).unwrap();
    assert_eq!(r.0, rec("0000 0100 0000 0000 0000 0000 0000 0000"));
    assert_eq!(
        WaypointRecord::load_copy(&rec("0301 0000 0000 0000 0000 0000 0000 0000")),
        Err(WaypointError::BadMagic(0x0103))
    );
    let mut bad = WaypointRecord(rec("0301 0000 0000 0000 0000 0000 0000 0000"));
    assert_eq!(bad.out_copy(), Err(WaypointError::BadMagic(0x0103)));
}

#[test]
fn save_section() {
    let mut recs = WaypointRecords::default();
    recs.0[0].set(1).unwrap();
    let s = write_section(&mut recs).unwrap();
    assert_eq!(s[..8], hex("5753 01000000 5000")[..]);
    assert_eq!(s[8..24], rec("0201 0300 0000 0000 0000 0000 0000 0000"));
    assert_eq!(s[24..32], [0; 8]);
    assert_eq!(s[32..48], rec("0201 0100 0000 0000 0000 0000 0000 0000"));
    assert_eq!(read_section(&s).unwrap(), recs);
    // Errors.
    assert_eq!(read_section(&s[..79]), Err(SaveError::Short));
    let mut b = s;
    b[0] = b'X';
    assert_eq!(read_section(&b), Err(SaveError::Header));
    let mut b = s;
    b[32] = 0x03;
    assert_eq!(read_section(&b), Err(SaveError::Magic(0x0103)));
    // Unread fields are ignored.
    let mut b = s;
    b[2] = 9;
    b[6] = 0;
    b[24] = 0xEE;
    assert_eq!(read_section(&b).unwrap(), recs);
}

#[test]
fn menu_message_vector() {
    let mut r = WaypointRecord::allocate();
    for n in 0..39 {
        r.set(n).unwrap();
    }
    let m = menu_message(0x33, &r.out_copy().unwrap());
    let want = hex("63 33000000 0201 ffff ffff 7f00 0000 0000 0000 0000");
    assert_eq!(m[..], want[..]);
}

// ----------------------------------------------------------- fake world

pub(super) const P: UnitId = UnitId(1);
pub(super) const WP_UNIT: UnitId = UnitId(50);
pub(super) const WP_CLASS: u16 = 119;
pub(super) const ROOM_A: RoomId = RoomId(7);
pub(super) const ROOM_DEST: RoomId = RoomId(9);

#[derive(Default)]
pub(super) struct Fake {
    pub(super) frame: i32,
    pub(super) difficulty: u8,
    pub(super) records: BTreeMap<UnitId, WaypointRecords>,
    pub(super) objects: Vec<(UnitId, ObjectFacts)>,
    pub(super) player: Option<PlayerFacts>,
    pub(super) busy: bool,
    pub(super) interact: Option<(u8, u32)>,
    pub(super) hostile: bool,
    /// Where `warp` places the player.
    pub(super) land: Option<(Option<RoomId>, i32, i32, u32)>,
    pub(super) spawn: Option<RoomId>,
    pub(super) log: Vec<String>,
    pub(super) sent: Vec<Vec<u8>>,
}

impl WaypointWorld for Fake {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn records(&mut self, player: UnitId) -> Option<&mut WaypointRecords> {
        self.records.get_mut(&player)
    }
    fn object(&self, guid: u32) -> Option<(UnitId, ObjectFacts)> {
        self.objects.iter().copied().find(|o| o.1.guid == guid)
    }
    fn player(&self, _: UnitId) -> PlayerFacts {
        self.player.unwrap()
    }
    fn room_rect(&self, _: RoomId) -> RoomRect {
        RoomRect {
            x: 100,
            y: 200,
            width: 40,
            height: 40,
        }
    }
    fn set_object_mode(&mut self, o: UnitId, mode: u8) {
        self.log.push(format!("mode {} {mode}", o.0));
    }
    fn schedule_endanim(&mut self, o: UnitId, frame: i32) {
        self.log.push(format!("endanim {} {frame}", o.0));
    }
    fn player_busy(&self, _: UnitId) -> bool {
        self.busy
    }
    fn set_interact(&mut self, _: UnitId, ty: u8, guid: u32) {
        if self.interact.is_none() {
            self.interact = Some((ty, guid));
        }
        self.log.push(format!("interact {ty} {guid:#x}"));
    }
    fn reset_interact(&mut self, _: UnitId) {
        self.interact = None;
        self.log.push("reset".into());
    }
    fn interact_guid(&self, _: UnitId) -> Option<u32> {
        self.interact.map(|i| i.1)
    }
    fn hostile_delay(&self, _: UnitId) -> bool {
        self.hostile
    }
    fn attach_sound(&mut self, _: UnitId, event: u8) {
        self.log.push(format!("sound {event:#x}"));
    }
    fn send(&mut self, _: UnitId, msg: &[u8]) {
        self.sent.push(msg.to_vec());
    }
    fn warp(&mut self, _: UnitId, level: u32, tile: u8) {
        self.log.push(format!("warp {level} {tile}"));
        if let Some((room, x, y, l)) = self.land {
            let p = self.player.as_mut().unwrap();
            (p.room, p.x, p.y, p.level) = (room, x, y, Some(l));
        }
    }
    fn spawn_room(&mut self, _: u32, _: u8) -> Option<RoomId> {
        self.spawn
    }
    fn set_player_mode_arrival(&mut self, _: UnitId) {
        self.log.push("player mode 2".into());
    }
}

pub(super) fn data() -> WaypointData {
    let rows = parse_tsv(WAYPOINTS_TSV).unwrap();
    let mut objects = vec![
        ObjectClass {
            operate_fn: 0,
            init_fn: 0,
            frame_cnt1: 0
        };
        600
    ];
    for c in [
        119u16, 145, 156, 157, 237, 238, 288, 323, 324, 398, 402, 429,
    ] {
        objects[c as usize] = ObjectClass {
            operate_fn: 23,
            init_fn: 17,
            frame_cnt1: 15 << 8,
        };
    }
    for c in [494u16, 496, 511, 539] {
        objects[c as usize] = ObjectClass {
            operate_fn: 23,
            init_fn: 17,
            frame_cnt1: 20 << 8,
        };
    }
    WaypointData {
        map: map_from_rows(&rows),
        objects,
    }
}

/// Player (amazon) and a waypoint object in Rogue Encampment (level 1).
pub(super) fn fake() -> Fake {
    let mut f = Fake {
        frame: 100,
        ..Fake::default()
    };
    f.records.insert(P, WaypointRecords::default());
    f.objects.push((
        WP_UNIT,
        ObjectFacts {
            guid: 0x0b,
            class: WP_CLASS,
            mode: 2,
            room: Some(ROOM_A),
            level: Some(1),
            x: 5000,
            y: 5000,
        },
    ));
    f.player = Some(PlayerFacts {
        guid: 1,
        class: 0,
        room: Some(ROOM_A),
        level: Some(1),
        x: 5010,
        y: 5000,
    });
    f
}

pub(super) fn msg49(wp: u32, level: u16) -> Vec<u8> {
    let mut m = vec![0x49];
    m.extend(wp.to_le_bytes());
    m.extend(level.to_le_bytes());
    m.extend([0, 0]);
    m
}

#[test]
fn waypoint_classes() {
    assert_eq!(
        data().waypoint_classes(),
        vec![119, 145, 156, 157, 237, 238, 288, 323, 324, 398, 402, 429, 494, 496, 511, 539]
    );
}

#[test]
fn operate_neutral_activates_without_menu() {
    // A neutral object is activated and the bit set, but no menu is sent.
    let d = data();
    let mut f = fake();
    let (u, mut o) = f.objects[0];
    o.mode = 0;
    o.level = Some(3); // Cold Plains, index 1
    o.class = 494; // FrameCnt1 20
    assert_eq!(d.operate(&mut f, u, &o, P), Ok(1));
    assert_eq!(f.log, ["mode 50 1", "endanim 50 121"]);
    assert!(f.sent.is_empty());
    assert!(f.records[&P].0[0].test(1).unwrap());
}

// B034: activating Stony Field's waypoint (level 4, index 2) sets only its
// own bit; the new record knows the town (bit 0) and nothing else.
#[test]
fn operating_one_waypoint_activates_only_its_own_bit() {
    let d = data();
    let mut f = fake();
    let (u, mut o) = f.objects[0];
    o.mode = 0;
    o.level = Some(4); // Stony Field, index 2
    o.class = 494;
    assert_eq!(
        WaypointRecord::allocate().0[2],
        1,
        "a new record: bit 0 only"
    );
    assert_eq!(d.operate(&mut f, u, &o, P), Ok(1));
    let r = &f.records[&P].0[0];
    assert!(r.test(0).unwrap() && r.test(2).unwrap());
    assert!(!r.test(1).unwrap(), "Cold Plains (index 1) stays unknown");
}

#[test]
fn operate_active_sends_menu() {
    // The menu message is `63 0a000000 0201 0300 …`.
    let d = data();
    let mut f = fake();
    let (u, mut o) = f.objects[0];
    o.guid = 0x0a;
    o.mode = 1;
    o.level = Some(3);
    assert_eq!(d.operate(&mut f, u, &o, P), Ok(1));
    assert_eq!(
        f.sent,
        [hex("63 0a000000 0201 0300 0000 0000 0000 0000 0000 0000")]
    );
    assert_eq!(f.interact, Some((2, 0x0a)));
    // Town, index 0 only.
    let mut f = fake();
    let (u, o) = f.objects[0];
    d.operate(&mut f, u, &o, P).unwrap();
    assert_eq!(
        f.sent,
        [hex("63 0b000000 0201 0100 0000 0000 0000 0000 0000 0000")]
    );
}

#[test]
fn operate_busy_sets_bit_only() {
    let d = data();
    let mut f = fake();
    f.busy = true;
    let (u, mut o) = f.objects[0];
    o.level = Some(4);
    d.operate(&mut f, u, &o, P).unwrap();
    assert!(f.sent.is_empty() && f.interact.is_none());
    assert!(f.records[&P].0[0].test(2).unwrap());
    // Other modes: bit only.
    let mut f = fake();
    o.mode = 3;
    d.operate(&mut f, u, &o, P).unwrap();
    assert!(f.sent.is_empty() && f.log.is_empty());
    // Per difficulty.
    let mut f = fake();
    f.difficulty = 2;
    o.mode = 2;
    d.operate(&mut f, u, &o, P).unwrap();
    assert!(!f.records[&P].0[0].test(2).unwrap());
    assert!(f.records[&P].0[2].test(2).unwrap());
}

#[test]
fn close_and_validation() {
    let d = data();
    // Level 0, |dx| = 10: result 0, interact reset, no message.
    let mut f = fake();
    f.interact = Some((2, 0x0b));
    let mut arr = ArrivalList::default();
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 0)), Ok(0));
    assert!(f.sent.is_empty());
    assert_eq!(f.interact, None);
    assert!(arr.0.is_empty());
    // Same level closes too.
    let mut f = fake();
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 1)), Ok(0));
    assert_eq!(f.log, ["reset"]);
    // Amazon |dx| = 11 → 1.
    let mut f = fake();
    f.player.as_mut().unwrap().x = 5011;
    assert_eq!(d.validate(&mut f, P, 0x0b, 0), Ok(1));
    // Sorceress |dy| = 22, level 3 known → 0.
    let mut f = fake();
    let p = f.player.as_mut().unwrap();
    (p.class, p.x, p.y) = (SORCERESS, 5000, 4978);
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    assert_eq!(d.validate(&mut f, P, 0x0b, 3), Ok(0));
    f.player.as_mut().unwrap().y = 4977;
    assert_eq!(d.validate(&mut f, P, 0x0b, 3), Ok(1));
    // Level 2 (no waypoint) → 3; level 0x200 → 3.
    let mut f = fake();
    assert_eq!(d.validate(&mut f, P, 0x0b, 2), Ok(3));
    assert_eq!(d.validate(&mut f, P, 0x0b, 0x200), Ok(3));
    // Level 4 with index 2 clear → 2 and interact reset if it is `wp`.
    f.interact = Some((2, 0x0b));
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 4)), Ok(2));
    assert_eq!(f.interact, None);
    f.interact = Some((2, 0x99));
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 4)), Ok(2));
    assert_eq!(f.interact, Some((2, 0x99)));
    // Object in another act → 2.
    let mut f = fake();
    f.objects[0].1.level = Some(40);
    assert_eq!(d.validate(&mut f, P, 0x0b, 0), Ok(2));
    // Unknown GUID → 1.
    assert_eq!(d.validate(&mut f, P, 0x77, 0), Ok(1));
    // Wrong size → 3.
    assert_eq!(
        d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 0)[..8]),
        Ok(3)
    );
    // Player data missing → error.
    let mut f = fake();
    f.records.clear();
    assert_eq!(
        d.validate(&mut f, P, 0x0b, 3),
        Err(WaypointError::NoPlayerData)
    );
}

#[test]
fn hostile_delay_refuses() {
    let d = data();
    let mut f = fake();
    f.hostile = true;
    f.interact = Some((2, 0x0b));
    let mut arr = ArrivalList::default();
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 0)), Ok(1));
    assert_eq!(f.log, ["sound 0x13", "reset"]);
}

#[test]
fn travel_same_act() {
    // Travel to level 3, player lands at
    // (0x131D, 0x1381); 0x0D carries + 3.
    let d = data();
    let mut f = fake();
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    f.land = Some((Some(ROOM_DEST), 0x131D, 0x1381, 3));
    f.spawn = Some(ROOM_DEST);
    let mut arr = ArrivalList::default();
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 3)), Ok(0));
    assert_eq!(f.log, ["reset", "warp 3 0", "player mode 2"]);
    assert_eq!(f.sent, [hex("0d 00 01000000 01 2013 8413 00 00")]);
    assert_eq!(
        arr.0,
        [ArrivalNode {
            room: Some(ROOM_DEST),
            x: 0x131D,
            y: 0x1381
        }]
    );
    // Town arrival: tile code 13 (0x0D carries `5d12 b311`).
    let mut f = fake();
    f.land = Some((Some(ROOM_A), 0x125A, 0x11B0, 1));
    f.spawn = Some(ROOM_A);
    f.objects[0].1.level = Some(3);
    f.player.as_mut().unwrap().level = Some(3);
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 1))
        .unwrap();
    assert_eq!(f.log, ["reset", "warp 1 13", "player mode 2"]);
    assert_eq!(f.sent, [hex("0d 00 01000000 01 5d12 b311 00 00")]);
    assert_eq!(arr.0.len(), 2); // prepended
    assert_eq!(arr.0[0].room, Some(ROOM_A));
    // Not in the spawn room: no 0x0D, node still recorded.
    let mut f = fake();
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    f.land = Some((None, 0, 0, 3));
    let mut arr = ArrivalList::default();
    d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 3))
        .unwrap();
    assert!(f.sent.is_empty());
    assert_eq!(arr.0[0].room, None);
}

#[test]
fn travel_through_non_waypoint_object_does_nothing() {
    // Validation passes, but travel stops at the operate-function check.
    let d = data();
    let mut f = fake();
    f.objects[0].1.class = 1;
    f.records.get_mut(&P).unwrap().0[0].set(1).unwrap();
    let mut arr = ArrivalList::default();
    assert_eq!(d.take_or_close(&mut f, &mut arr, P, &msg49(0x0b, 3)), Ok(0));
    assert!(f.log.is_empty() && f.sent.is_empty() && arr.0.is_empty());
}

#[test]
fn init17() {
    let d = data();
    let (u, mut o) = fake().objects[0];
    o.mode = 0;
    o.level = Some(3);
    // Room pointer match: mode 1, ENDANIM at frame + 15, whole list dropped.
    let mut f = fake();
    let mut arr = ArrivalList(vec![
        ArrivalNode {
            room: Some(RoomId(1)),
            x: 0,
            y: 0,
        },
        ArrivalNode {
            room: Some(ROOM_A),
            x: 0,
            y: 0,
        },
    ]);
    d.init_object(&mut f, &mut arr, u, &o);
    assert_eq!(f.log, ["mode 50 1", "endanim 50 115"]);
    assert!(arr.0.is_empty());
    // Rectangle match: x ≥ left, x < left + width.
    let mut f = fake();
    let node = |x, y| ArrivalNode { room: None, x, y };
    let mut arr = ArrivalList(vec![node(139, 239)]);
    d.init_object(&mut f, &mut arr, u, &o);
    assert_eq!(f.log.len(), 2);
    let mut f = fake();
    let mut arr = ArrivalList(vec![node(140, 200)]);
    d.init_object(&mut f, &mut arr, u, &o);
    assert!(f.log.is_empty());
    assert_eq!(arr.0.len(), 1); // kept (edge case 3)
                                // Match with the object not in mode 0: no mode change, list dropped.
    let mut f = fake();
    let mut arr = ArrivalList(vec![node(100, 200)]);
    let mut o2 = o;
    o2.mode = 2;
    d.init_object(&mut f, &mut arr, u, &o2);
    assert!(f.log.is_empty() && arr.0.is_empty());
    // No match in town: mode 2.
    let mut f = fake();
    let mut o3 = o;
    o3.level = Some(1);
    d.init_object(&mut f, &mut ArrivalList::default(), u, &o3);
    assert_eq!(f.log, ["mode 50 2"]);
}

#[test]
fn act5_bits_survive_load_and_pass_validation() {
    // A record from an edited save keeps its act-5 bit (index 38 is in
    // byte 6) and validation checks neither act nor expansion of the
    // destination.
    let src = rec("0201 0100 0000 0000 0000 0000 0000 0000");
    let mut r = WaypointRecords::default();
    let mut raw = src;
    raw[6] = 0x40; // index 38 = byte 2 + 4, bit 6
    r.0[0] = WaypointRecord::load_copy(&raw).unwrap();
    assert!(r.0[0].test(38).unwrap());
    assert_eq!(r.0[0].out_copy().unwrap()[6], 0x40);
    let d = data();
    let level = d.map.level_of_index(38).unwrap();
    let mut f = fake();
    f.records.insert(P, r);
    assert_eq!(d.validate(&mut f, P, 0x0b, level), Ok(0));
}
