// Spec: specs/world/objects-2.md §16–§18 (Test vectors)
use super::super::fake::{blank_object, Call, Fake};
use super::super::{dispatch, object_event, oevent, oflags, Dispatch, ObjectData, Operator};
use super::*;
use crate::rng::Seed;

/// Test conventions on unit 0's stat map: free point / placement fail.
const NO_FREE_POINT: u16 = 0xE001;
const PLACE_FAILS: u16 = 0xE002;
/// On an item: `item_subtype` / `remove_cursor_item` fails.
const SUBTYPE: u16 = 0xE003;
const NO_UNCURSOR: u16 = 0xE004;

/// Every part-2 seam call is recorded as `Call::Other`.
impl MechWorld for Fake {
    fn warp_through_tile(&mut self, player: UnitId, tile: UnitId) {
        self.calls
            .push(Call::Other(format!("warp {} {}", player.0, tile.0)));
    }
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        self.calls.push(Call::Other(format!(
            "interact {} {unit_type} {guid:#x}",
            player.0
        )));
    }
    fn remove_cursor_item(&mut self, player: UnitId, item: UnitId) -> bool {
        self.calls
            .push(Call::Other(format!("uncursor {} {}", player.0, item.0)));
        !self
            .stats
            .get(&(item, NO_UNCURSOR))
            .is_some_and(|&v| v != 0)
    }
    fn item_subtype(&self, item: UnitId) -> u8 {
        self.stats.get(&(item, SUBTYPE)).copied().unwrap_or(0) as u8
    }
    fn power_up_add_stat(&mut self, player: UnitId, stat: u16, delta: i32) {
        self.calls
            .push(Call::Other(format!("addstat {} {stat} {delta}", player.0)));
    }
    fn has_gem(&mut self, player: UnitId) -> bool {
        self.keys.contains(&player)
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.calls
            .push(Call::Other(format!("send {} {msg:02x?}", player.0)));
    }
    fn stand_drop(&mut self, object: UnitId, weapon: bool) {
        self.calls
            .push(Call::Other(format!("stand {} {weapon}", object.0)));
    }
    fn drop_code_quality(&mut self, object: UnitId, code: u32, quality: u8) {
        let s = String::from_utf8_lossy(&code.to_le_bytes()).into_owned();
        self.calls
            .push(Call::Other(format!("dropcode {} {s} q{quality}", object.0)));
    }
    fn trap_damage_arg(&mut self, object: UnitId, target: UnitId, arg: u32) {
        self.calls.push(Call::Other(format!(
            "damage {} {} {arg}",
            object.0, target.0
        )));
    }
    fn in_town(&self, _room: RoomId) -> bool {
        true
    }
    fn adjacent_rooms(&self, room: RoomId) -> Vec<RoomId> {
        self.adjacent.get(&room).cloned().unwrap_or_default()
    }
    fn send_room_reveal(&mut self, player: UnitId, room: RoomId) {
        self.calls
            .push(Call::Other(format!("reveal {} {}", player.0, room.0)));
    }
    fn set_flags2(&mut self, unit: UnitId, bits: u32) {
        self.calls
            .push(Call::Other(format!("flags2 {} {bits:#x}", unit.0)));
    }
    /// The point itself in the same room.
    fn free_point(
        &self,
        room: RoomId,
        x: i32,
        y: i32,
        _size: i32,
        _mask: u32,
    ) -> Option<(RoomId, i32, i32)> {
        if self
            .stats
            .get(&(UnitId(0), NO_FREE_POINT))
            .is_some_and(|&v| v != 0)
        {
            return None;
        }
        Some((room, x, y))
    }
    fn place_unit(&mut self, unit: UnitId, room: RoomId, x: i32, y: i32) -> bool {
        self.calls
            .push(Call::Other(format!("place {} {} {x} {y}", unit.0, room.0)));
        !self
            .stats
            .get(&(UnitId(0), PLACE_FAILS))
            .is_some_and(|&v| v != 0)
    }
    fn recount_tomes(&mut self, player: UnitId) {
        self.calls
            .push(Call::Other(format!("recount {}", player.0)));
    }
}

const O: UnitId = UnitId(10);
const P: UnitId = UnitId(20);
const ROOM: RoomId = RoomId(3);
const CLASS: u16 = 5;

/// One `Sync` row (no unit-seed draw), `FrameCnt1` 10 · 256.
fn tables() -> ObjectTables {
    let mut o = blank_object();
    o.sync = 1;
    o.framecnt1 = 10 << 8;
    o.framecnt3 = 4 << 8;
    o.mode2 = 1;
    o.selectable0 = 1;
    ObjectTables {
        objects: vec![o; 400],
        ..Default::default()
    }
}

fn setup(class: u16, mode: u8, fn_: u8, t: &mut ObjectTables) -> (ObjectControl, Fake) {
    t.objects[class as usize].operatefn = fn_;
    let mut ctl = ObjectControl {
        seed: Seed::new(1, 666),
        regions: Vec::new(),
        shrine_lists: Default::default(),
        data: Default::default(),
    };
    ctl.data.insert(
        O,
        ObjectData {
            class,
            ..Default::default()
        },
    );
    let mut f = Fake {
        frame: 100,
        ..Fake::default()
    };
    f.modes.insert(O, mode);
    f.rooms.insert(O, ROOM);
    f.rooms.insert(P, ROOM);
    f.flags.insert(O, oflags::SELECTABLE);
    f.operators.insert(P, Operator::Player(1));
    f.guids.insert(O, 0x55);
    f.guids.insert(P, 0x77);
    (ctl, f)
}

fn others(f: &Fake) -> Vec<String> {
    f.calls
        .iter()
        .filter_map(|c| match c {
            Call::Other(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

fn modes(f: &Fake) -> Vec<u8> {
    f.calls
        .iter()
        .filter_map(|c| match c {
            Call::Mode(_, m, _) => Some(*m),
            _ => None,
        })
        .collect()
}

// Covers: specs/world/objects-2.md §16.1
#[test]
fn torch_tiki_toggles() {
    let mut t = tables();
    for (from, to) in [(0u8, Some(1u8)), (1, Some(0)), (2, None)] {
        let (mut c, mut f) = setup(CLASS, from, 13, &mut t);
        assert_eq!(
            dispatch(&mut c, &t, &mut f, O, Some(P)),
            Ok(Dispatch::Done(0))
        );
        assert_eq!(modes(&f), to.into_iter().collect::<Vec<_>>());
    }
}

// Covers: specs/world/objects-2.md §16.4, §16.5
#[test]
fn secret_door_and_stand_order() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 0, 18, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert_eq!(modes(&f), vec![1]);
    assert_eq!(f.flags[&O] & oflags::SELECTABLE, 0);
    assert!(f.calls.contains(&Call::Free(O)));
    assert!(f.calls.contains(&Call::Schedule(O, oevent::END_ANIM, 111)));
    // Stand: drop before the mode change.
    let (mut c, mut f) = setup(CLASS, 0, 20, &mut t);
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    let i_drop = f
        .calls
        .iter()
        .position(|c| *c == Call::Other("stand 10 true".into()))
        .unwrap();
    let i_mode = f
        .calls
        .iter()
        .position(|c| matches!(c, Call::Mode(_, 2, _)))
        .unwrap();
    assert!(i_drop < i_mode);
    // Not mode 0: nothing.
    let (mut c, mut f) = setup(CLASS, 2, 19, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert!(f.calls.is_empty());
}

// Covers: specs/world/objects-2.md §16.6
#[test]
fn bookshelf_code_draws() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 0, 26, &mut t);
    let mut e = Seed::new(1, 666);
    let r = e.step() % 20;
    let b = e.step() & 1;
    // {1, 666}: lo' 1791398751 mod 20 = 11 < 13; lo' 791599131 & 1 = 1.
    assert_eq!((r, b), (11, 1));
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert_eq!(c.seed, e);
    assert_eq!(others(&f), vec!["dropcode 10 isc  q2".to_string()]);
    assert_eq!(c.get(O).unwrap().drop_code, u32::from_le_bytes(*b"isc "));
    assert_eq!(modes(&f), vec![2]);
}

// Covers: specs/world/objects-2.md §16.3; specs/world/objects.md §edge-cases-original-bugs r14
#[test]
fn obelisk_needs_a_gem() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 0, 17, &mut t);
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert!(f
        .calls
        .contains(&Call::Sound(P, sound::PORTAL_REFUSED, Some(P), false)));
    assert!(modes(&f).is_empty());
    let (mut c, mut f) = setup(CLASS, 0, 17, &mut t);
    f.keys.insert(P);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert_eq!(modes(&f), vec![3]);
    assert_eq!(
        others(&f),
        vec![
            "interact 20 2 0x55".to_string(),
            "send 20 [58, 55, 00, 00, 00, 00, 00]".to_string()
        ]
    );
}

// Covers: specs/world/objects-2.md §16.8, §16.9, §16.10
#[test]
fn slime_door_exploding_chest_bank() {
    let mut t = tables();
    t.objects[CLASS as usize].selectable1 = 1;
    let (mut c, mut f) = setup(CLASS, 0, 29, &mut t);
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert_ne!(f.flags[&O] & oflags::SELECTABLE, 0, "Selectable[1]");
    assert!(f.calls.contains(&Call::Free(O)));
    let (mut c, mut f) = setup(CLASS, 0, 30, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(0))
    );
    assert_eq!(others(&f), vec!["damage 10 20 0", "damage 10 20 1"]);
    assert_eq!(modes(&f), vec![1]);
    let (mut c, mut f) = setup(BANK_CLASS, 0, 32, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(0))
    );
    assert_eq!(
        others(&f),
        vec!["interact 20 2 0x55", "send 20 [77, 10]", "recount 20"]
    );
}

// Covers: specs/world/objects-2.md §16.12, §18.5
#[test]
fn jungle_stash_schedules_drop_and_keeps_owner() {
    let mut t = tables();
    t.objects[CLASS as usize].parm1 = 11;
    let (mut c, mut f) = setup(CLASS, 0, 51, &mut t);
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert!(f
        .calls
        .contains(&Call::Schedule(O, mevent::STASH_DROP, 112)));
    assert!(f.calls.contains(&Call::Free(O)), "HasCollision1 0");
    assert_eq!(c.get(O).unwrap().owner, Some(0x77));
}

// Covers: specs/world/objects-2.md §16.13
#[test]
fn gate_debounce_and_close() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 0, 61, &mut t);
    f.tick = 1000;
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert_eq!(modes(&f), vec![1]);
    assert_eq!(c.get(O).unwrap().last_tick, 1000);
    f.calls.clear();
    f.modes.insert(O, 2);
    f.tick = 1499;
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert!(f.calls.is_empty(), "within 500");
    f.tick = 1500;
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert_eq!(f.calls[0], Call::Stamp(O));
    assert_eq!(modes(&f), vec![0]);
}

// Covers: specs/world/objects-2.md §18.4
#[test]
fn trapped_soul_operate_and_events() {
    let mut t = tables();
    // r = 51 < 90: D(0), no item → no mode.
    let (mut c, mut f) = setup(CLASS, 0, 48, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(0))
    );
    assert!(modes(&f).is_empty());
    // Event 9 handler by mode.
    let (mut c, mut f) = setup(CLASS, 1, 48, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::SOUL).unwrap();
    assert_eq!(f.schedules(), vec![(O, mevent::SOUL, 110)]);
    let (mut c, mut f) = setup(CLASS, 3, 48, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::SOUL).unwrap();
    assert_eq!(modes(&f), vec![4]);
    // Mode 2, InteractType 0 → 1 < 2: burn (no units), event 9 at +25.
    let (mut c, mut f) = setup(CLASS, 2, 48, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::SOUL).unwrap();
    assert_eq!(c.get(O).unwrap().interact, 1);
    assert_eq!(f.schedules(), vec![(O, mevent::SOUL, 125)]);
}

// Covers: specs/world/objects-2.md §18.1, §18.6, §edge-cases-original-bugs r1; specs/world/objects.md §edge-cases-original-bugs r13
#[test]
fn fire_event_and_endanim_write_mode_directly() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 1, 0, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::FIRE).unwrap();
    assert_eq!(f.modes[&O], 2);
    assert!(modes(&f).is_empty(), "no mode set");
    let mut e = Seed::new(1, 666);
    let lo = e.step();
    assert_eq!(
        f.schedules(),
        vec![(O, mevent::FIRE, 115 + (lo % 35) as i32)]
    );
    // ENDANIM: direct write, footprint freed (HasCollision2 0), no flag.
    let (mut c, mut f) = setup(CLASS, 1, 0, &mut t);
    f.flags.insert(O, 0);
    object_event(&mut c, &t, &mut f, O, oevent::END_ANIM).unwrap();
    assert_eq!(
        f.calls,
        vec![Call::Other("store 10 2".into()), Call::Free(O)]
    );
    assert_eq!(f.flags[&O], 0);
    // Mode 2: nothing.
    let (mut c, mut f) = setup(CLASS, 2, 0, &mut t);
    object_event(&mut c, &t, &mut f, O, oevent::END_ANIM).unwrap();
    assert!(f.calls.is_empty());
}

// Covers: specs/world/objects-2.md §18.2, §18.3; specs/world/objects.md §edge-cases-original-bugs r20
#[test]
fn spike_and_fissure_events() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 0, 0, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::SPIKE).unwrap();
    assert_eq!(f.schedules(), vec![(O, mevent::SPIKE, 115)], "nobody on it");
    c.get_mut(O).unwrap().interact = 2;
    f.modes.insert(O, 1);
    f.calls.clear();
    object_event(&mut c, &t, &mut f, O, mevent::SPIKE).unwrap();
    assert_eq!(modes(&f), vec![0]);
    assert_eq!(c.get(O).unwrap().interact, 0);
    // Fissure in mode 1: nothing, no reschedule.
    let (mut c, mut f) = setup(FISSURE_CLASS, 1, 0, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::EVENT8).unwrap();
    assert!(f.calls.is_empty());
    let (mut c, mut f) = setup(FISSURE_CLASS, 0, 0, &mut t);
    object_event(&mut c, &t, &mut f, O, mevent::EVENT8).unwrap();
    assert_eq!(modes(&f), vec![1]);
    let mut e = Seed::new(1, 666);
    let lo = e.step();
    assert_eq!(
        f.schedules()[0],
        (O, mevent::EVENT8, 125 + (lo % 250) as i32)
    );
}

// Covers: specs/world/objects-2.md §17
#[test]
fn small_inits() {
    let mut t = tables();
    t.objects[CLASS as usize].mode2 = 1;
    for (n, mode, want_modes, want_sched) in [
        (8u8, 0u8, vec![2u8], vec![]),
        (14, 0, vec![1], vec![]),
        (26, 0, vec![1], vec![]),
        (22, 0, vec![2], vec![(O, mevent::FIRE, 125)]),
        (24, 0, vec![], vec![(O, mevent::SPIKE, 125)]),
        (51, 1, vec![], vec![(O, mevent::SOUL, 135)]),
        (51, 0, vec![], vec![]),
        (13, 0, vec![2], vec![]),
        (13, 2, vec![], vec![]),
    ] {
        let (mut c, mut f) = setup(CLASS, mode, 0, &mut t);
        init(&mut c, &t, &mut f, O, n, Some(ROOM), 5, 5).unwrap();
        assert_eq!(modes(&f), want_modes, "init {n}");
        assert_eq!(f.schedules(), want_sched, "init {n}");
    }
    // 27: lo' 1791398751 mod 1000 = 751 > 332 → 0.
    let (mut c, mut f) = setup(CLASS, 0, 0, &mut t);
    c.get_mut(O).unwrap().interact = 9;
    init(&mut c, &t, &mut f, O, 27, Some(ROOM), 5, 5).unwrap();
    assert_eq!(c.get(O).unwrap().interact, 0);
    // 34: lo' & 1 = 1 → mode 1.
    let (mut c, mut f) = setup(CLASS, 0, 0, &mut t);
    init(&mut c, &t, &mut f, O, 34, Some(ROOM), 5, 5).unwrap();
    assert_eq!(modes(&f), vec![1]);
    // 58: event 8 at f + 25 + 751 mod 250.
    let (mut c, mut f) = setup(CLASS, 0, 0, &mut t);
    init(&mut c, &t, &mut f, O, 58, Some(ROOM), 5, 5).unwrap();
    assert_eq!(f.schedules(), vec![(O, mevent::EVENT8, 125 + 1)]);
    // 28: mode 2, n = 1791398751 mod 9 + 1 = 7 rounds of 2 steps, no
    // room match (the fake finds none): no gold.
    let (mut c, mut f) = setup(CLASS, 0, 0, &mut t);
    init(&mut c, &t, &mut f, O, 28, Some(ROOM), 5, 5).unwrap();
    assert_eq!(modes(&f), vec![2]);
    let mut e = Seed::new(1, 666);
    assert_eq!(e.step() % 9 + 1, 7);
    for _ in 0..14 {
        e.step();
    }
    assert_eq!(c.seed, e);
}

// Covers: specs/world/objects-2.md §16.11
#[test]
fn stairs_open_and_stair2_gate() {
    let mut t = tables();
    let (mut c, mut f) = setup(CLASS, 0, 47, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert_eq!(modes(&f), vec![1]);
    let (mut c, mut f) = setup(CLASS, 0, 50, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(0))
    );
    assert!(f.calls.is_empty());
}

// Covers: specs/world/objects-2.md §16.2; specs/world/objects.md §edge-cases-original-bugs r16
#[test]
fn trap_door_modes() {
    let mut t = tables();
    // r1: mode 0 → mode 2, result 1.
    let (mut c, mut f) = setup(CLASS, 0, 16, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert_eq!(modes(&f), vec![2]);
    // r2: mode 2: warp P through the first type-5 unit of O's room list;
    // none → fatal.
    let (mut c, mut f) = setup(CLASS, 2, 16, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Err(ObjectError::NoWarpTile(O))
    );
    f.room_unit_lists
        .insert(ROOM, vec![UnitId(40), UnitId(41), UnitId(42)]);
    f.unit_types.insert(UnitId(40), 2);
    f.unit_types.insert(UnitId(41), 5);
    f.unit_types.insert(UnitId(42), 5);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert_eq!(others(&f), vec!["warp 20 41"]);
    assert!(modes(&f).is_empty());
    // r3: other modes: result 1, nothing.
    for m in [1, 3, 7] {
        let (mut c, mut f) = setup(CLASS, m, 16, &mut t);
        assert_eq!(
            dispatch(&mut c, &t, &mut f, O, Some(P)),
            Ok(Dispatch::Done(1))
        );
        assert!(f.calls.is_empty());
    }
}

// Covers: specs/world/objects-2.md §16.7 r1, §16.7 r2, §16.7 r3, §16.7 r4, §16.7 r5
#[test]
fn teleport_pad_partner_and_arrival() {
    let mut t = tables();
    let pad = |f: &mut Fake, room: u32, unit: u32, ty: u8, class: u32, at: (i32, i32)| {
        let u = UnitId(unit);
        f.room_unit_lists.entry(RoomId(room)).or_default().push(u);
        f.unit_types.insert(u, ty);
        f.unit_classes.insert(u, class);
        f.positions.insert(u, at);
    };
    let arrive = |room: u32, x: i32, y: i32| {
        vec![
            format!("place 20 {room} {x} {y}"),
            format!("reveal 20 {room}"),
            "flags2 20 0x10000".to_string(),
        ]
    };
    let run = |f: &mut Fake, c: &mut ObjectControl, t: &ObjectTables| {
        assert_eq!(dispatch(c, t, f, O, Some(P)), Ok(Dispatch::Done(0)));
    };
    // r1: O without a room → 0, nothing.
    let (mut c, mut f) = setup(CLASS, 0, 27, &mut t);
    f.rooms.remove(&O);
    run(&mut f, &mut c, &t);
    assert!(f.calls.is_empty());
    // r2/r3/r4/r5: the partner in O's own room (skipping O itself and a
    // different class or type), placed at its position; the reveal, the
    // queue and the flag follow.
    let (mut c, mut f) = setup(CLASS, 0, 27, &mut t);
    f.unit_classes.insert(O, u32::from(CLASS));
    pad(&mut f, 3, 10, 2, u32::from(CLASS), (1, 1));
    pad(&mut f, 3, 38, 2, 99, (2, 2));
    pad(&mut f, 3, 39, 5, u32::from(CLASS), (3, 3));
    pad(&mut f, 3, 40, 2, u32::from(CLASS), (70, 80));
    run(&mut f, &mut c, &t);
    assert_eq!(others(&f), arrive(3, 70, 80));
    assert!(f.calls.contains(&Call::Queue(P)));
    // r2: not in O's room: the adjacency array in order (O's room
    // skipped); the first room holding a match wins.
    let (mut c, mut f) = setup(CLASS, 0, 27, &mut t);
    f.unit_classes.insert(O, u32::from(CLASS));
    pad(&mut f, 3, 10, 2, u32::from(CLASS), (1, 1));
    f.adjacent
        .insert(ROOM, vec![RoomId(3), RoomId(4), RoomId(5), RoomId(6)]);
    pad(&mut f, 4, 41, 2, 99, (5, 5));
    pad(&mut f, 5, 42, 2, u32::from(CLASS), (50, 60));
    pad(&mut f, 6, 43, 2, u32::from(CLASS), (90, 90));
    run(&mut f, &mut c, &t);
    assert_eq!(others(&f), arrive(5, 50, 60));
    // r2: no partner anywhere → 0, nothing.
    let (mut c, mut f) = setup(CLASS, 0, 27, &mut t);
    f.unit_classes.insert(O, u32::from(CLASS));
    pad(&mut f, 3, 10, 2, u32::from(CLASS), (1, 1));
    run(&mut f, &mut c, &t);
    assert!(f.calls.is_empty());
    // r3: no free point → 0, nothing placed.
    let (mut c, mut f) = setup(CLASS, 0, 27, &mut t);
    f.unit_classes.insert(O, u32::from(CLASS));
    pad(&mut f, 3, 40, 2, u32::from(CLASS), (70, 80));
    f.stats.insert((UnitId(0), NO_FREE_POINT), 1);
    run(&mut f, &mut c, &t);
    assert!(f.calls.is_empty());
    // r4: placement fails → 0, no reveal, no queue, no flag.
    let (mut c, mut f) = setup(CLASS, 0, 27, &mut t);
    f.unit_classes.insert(O, u32::from(CLASS));
    pad(&mut f, 3, 40, 2, u32::from(CLASS), (70, 80));
    f.stats.insert((UnitId(0), PLACE_FAILS), 1);
    run(&mut f, &mut c, &t);
    assert_eq!(others(&f), vec!["place 20 3 70 80"]);
    assert!(!f.calls.contains(&Call::Queue(P)));
}

const ITEM: UnitId = UnitId(60);

/// P's unit seed such that `roll(100)` is `r` (one step from {lo, 0}).
fn seed_roll(r: u32) -> Seed {
    let lo = (0..20_000u32)
        .find(|&lo| Seed::new(lo, 0).step() % 100 == r)
        .unwrap();
    Seed::new(lo, 0)
}

fn insert(s: u8, r: u32) -> (ObjectControl, Fake, i32) {
    let mut t = tables();
    t.objects[CLASS as usize].framecnt1 = 10 << 8;
    let (c, mut f) = setup(CLASS, 3, 17, &mut t);
    f.seeds.insert(P, seed_roll(r));
    f.stats.insert((ITEM, SUBTYPE), i32::from(s));
    f.stats.insert((P, 9), 1000);
    f.stats.insert((P, 7), 2000);
    let out = obelisk_insert(&t, &mut f, P, O, CLASS, ITEM).unwrap();
    (c, f, out)
}

// Covers: specs/world/objects-2.md §19 r1, §19 r2, §19 r3, §19 r4, §edge-cases-original-bugs r2
#[test]
fn obelisk_insert_power_up() {
    let msg = |b: u8| format!("send 20 [58, 55, 00, 00, 00, 05, {b:02x}]");
    // Entry 0: always accepted (chance 100): max mana + 256 into mana and
    // max mana; interact cleared; mode 1 then ENDANIM at f + 10 + 1.
    let (_, f, out) = insert(0, 99);
    assert_eq!(out, 1);
    assert_eq!(
        others(&f),
        vec![
            "uncursor 20 60".to_string(),
            "stat 8 1256".into(),
            "stat 9 1256".into(),
            msg(1),
        ]
    );
    assert_eq!(modes(&f), vec![1]);
    assert_eq!(f.schedules(), vec![(O, oevent::END_ANIM, 111)]);
    // One draw on P's unit seed.
    let mut e = seed_roll(99);
    e.step();
    assert_eq!(f.seeds[&P], e);
    // Table: (s, chance, stat id or 0xFF for mana / life, value).
    let table: [(u8, u32, &str); 21] = [
        (0, 100, "mana1"),
        (1, 100, "mana1"),
        (2, 100, "mana2"),
        (3, 5, "addstat 20 1 1"),
        (4, 10, "addstat 20 1 1"),
        (5, 15, "addstat 20 1 1"),
        (6, 5, "addstat 20 2 1"),
        (7, 10, "addstat 20 2 1"),
        (8, 15, "addstat 20 2 1"),
        (9, 5, "addstat 20 3 1"),
        (10, 10, "addstat 20 3 1"),
        (11, 15, "addstat 20 3 1"),
        (12, 5, "addstat 20 0 1"),
        (13, 10, "addstat 20 0 1"),
        (14, 15, "addstat 20 0 1"),
        (15, 100, "life1"),
        (16, 100, "life1"),
        (17, 100, "life2"),
        (18, 3, "addstat 20 5 1"),
        (19, 6, "addstat 20 5 1"),
        (20, 10, "addstat 20 5 1"),
    ];
    for (s, chance, effect) in table {
        let mut cases = vec![(chance - 1, true)];
        if chance < 100 {
            cases.push((chance, false));
        }
        for (r, ok) in cases {
            let (_, f, _) = insert(s, r);
            let o = others(&f);
            assert_eq!(o[0], "uncursor 20 60");
            if ok {
                match effect {
                    "mana1" => assert_eq!(o[1], "stat 8 1256", "s {s}"),
                    "mana2" => assert_eq!(o[1], "stat 8 1512", "s {s}"),
                    "life1" => assert_eq!(o[1], "stat 6 2256", "s {s}"),
                    "life2" => assert_eq!(o[1], "stat 6 2512", "s {s}"),
                    e => assert_eq!(o[1], e, "s {s}"),
                }
                assert_eq!(*o.last().unwrap(), msg(1));
                assert_eq!(modes(&f), vec![1]);
            } else {
                assert_eq!(o, vec!["uncursor 20 60".to_string(), msg(0)], "s {s} r {r}");
                assert_eq!(modes(&f), vec![0]);
                assert!(f.schedules().is_empty());
            }
        }
    }
    // Table count: s ≥ 21 → b = 0 with no draw.
    let (_, f, _) = insert(21, 0);
    assert_eq!(others(&f), vec!["uncursor 20 60".to_string(), msg(0)]);
    assert_eq!(f.seeds[&P], seed_roll(0), "no draw");
    assert_eq!(modes(&f), vec![0]);
    // The cursor removal fails: result 4, b 0, nothing else.
    let mut t = tables();
    let (_, mut f) = setup(CLASS, 3, 17, &mut t);
    f.stats.insert((ITEM, NO_UNCURSOR), 1);
    obelisk_insert(&t, &mut f, P, O, CLASS, ITEM).unwrap();
    assert_eq!(
        others(&f),
        vec![
            "uncursor 20 60".to_string(),
            "send 20 [58, 55, 00, 00, 00, 04, 00]".into()
        ]
    );
    // Edge 2: the object's mode is not tested (mode 0 here).
    let (_, f, _) = {
        let mut t = tables();
        let (c, mut f) = setup(CLASS, 0, 17, &mut t);
        f.seeds.insert(P, seed_roll(0));
        obelisk_insert(&t, &mut f, P, O, CLASS, ITEM).unwrap();
        (c, f, 0)
    };
    assert_eq!(modes(&f), vec![1]);
}

// Covers: specs/world/objects-2.md §18 text, §18.1 r2
#[test]
fn burn_scans_only_own_room_players_in_reach() {
    let mut t = tables();
    t.objects[CLASS as usize].parm0 = 4;
    let (mut c, mut f) = setup(CLASS, 1, 0, &mut t);
    f.positions.insert(O, (100, 100));
    let units = [
        // (unit, type, mode, dx): in reach, out of reach, dead, monster.
        (41u32, 0u8, 0u8, 4i32),
        (42, 0, 0, 6),
        (43, 0, 17, 1),
        (44, 1, 0, 1),
        (45, 0, 0, 5),
    ];
    for (u, ty, mode, dx) in units {
        let u = UnitId(u);
        f.room_unit_lists.entry(ROOM).or_default().push(u);
        f.unit_types.insert(u, ty);
        f.modes.insert(u, mode);
        f.positions.insert(u, (100 + dx, 100));
    }
    // A player in another room is not scanned.
    let far = UnitId(46);
    f.room_unit_lists.entry(RoomId(9)).or_default().push(far);
    f.unit_types.insert(far, 0);
    f.positions.insert(far, (100, 100));
    f.adjacent.insert(ROOM, vec![RoomId(9)]);
    object_event(&mut c, &t, &mut f, O, mevent::FIRE).unwrap();
    // Reach = Parm0 + 1 = 5 (inclusive): units 41 and 45.
    assert_eq!(
        others(&f),
        vec!["store 10 2", "damage 10 41 1", "damage 10 45 1"]
    );
}

// Covers: specs/world/objects-2.md §17; specs/world/objects.md §edge-cases-original-bugs r18
#[test]
fn gold_placeholder_room_lookup_drifts() {
    let mut t = tables();
    t.objects[CLASS as usize].mode2 = 1;
    // The init room covers x ≥ 7 only (init point (5, 5)).
    fn inside(x: i32, _y: i32) -> bool {
        x >= 7
    }
    let (mut c, mut f) = setup(CLASS, 0, 0, &mut t);
    f.room_at_fn = Some(inside);
    f.free_all = true;
    init(&mut c, &t, &mut f, O, 28, Some(ROOM), 5, 5).unwrap();
    // Reference of the spec text: the lookup uses the last accepted point
    // L plus the new offset; the drop uses the init point plus the offset.
    let mut e = Seed::new(1, 666);
    let n = e.step() % 9 + 1;
    let (mut l, mut drift, mut naive) = ((5, 5), vec![], vec![]);
    for _ in 0..n {
        let (dx, dy) = ((e.step() & 3) as i32, (e.step() & 3) as i32);
        if inside(l.0 + dx, l.1 + dy) {
            l = (5 + dx, 5 + dy);
            drift.push(format!("gold {} {}", 5 + dx, 5 + dy));
        }
        if inside(5 + dx, 5 + dy) {
            naive.push(format!("gold {} {}", 5 + dx, 5 + dy));
        }
    }
    assert!(!drift.is_empty());
    assert_ne!(drift, naive, "the vector exercises the drift");
    assert_eq!(
        others(&f)
            .into_iter()
            .filter(|s| s.starts_with("gold"))
            .collect::<Vec<_>>(),
        drift
    );
    assert_eq!(c.seed, e);
}

// Covers: specs/world/objects-2.md §16.13, §18.4; specs/world/objects.md §edge-cases-original-bugs r19, §edge-cases-original-bugs r21
#[test]
fn gate_ignores_occupancy_and_dropless_soul_goes_inert() {
    let mut t = tables();
    // Gate in mode 2 with units in the doorway (cells 0xFFFF): restamped
    // and closed anyway.
    let (mut c, mut f) = setup(CLASS, 2, 61, &mut t);
    f.tick = 5000;
    f.stats.insert((O, 0xFFFF), 0xFFFF);
    dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
    assert_eq!(f.calls[0], Call::Stamp(O));
    assert_eq!(modes(&f), vec![0]);
    // Trapped soul, mode 2, counter 1 → 2: the drop finds nothing: no
    // mode, no event pending.
    let (mut c, mut f) = setup(CLASS, 2, 48, &mut t);
    c.get_mut(O).unwrap().interact = 1;
    object_event(&mut c, &t, &mut f, O, mevent::SOUL).unwrap();
    assert_eq!(c.get(O).unwrap().interact, 2);
    assert!(modes(&f).is_empty());
    assert!(f.schedules().is_empty());
}

// Covers: specs/world/objects.md §edge-cases-original-bugs r15
#[test]
fn teleport_pad_ignores_its_own_mode() {
    let mut t = tables();
    for mode in [0u8, 1, 2, 3, 5] {
        let (mut c, mut f) = setup(CLASS, mode, 27, &mut t);
        f.unit_classes.insert(O, u32::from(CLASS));
        f.room_unit_lists.entry(ROOM).or_default().push(UnitId(40));
        f.unit_types.insert(UnitId(40), 2);
        f.unit_classes.insert(UnitId(40), u32::from(CLASS));
        f.positions.insert(UnitId(40), (70, 80));
        assert_eq!(
            dispatch(&mut c, &t, &mut f, O, Some(P)),
            Ok(Dispatch::Done(0))
        );
        assert!(
            others(&f).contains(&"place 20 3 70 80".to_string()),
            "mode {mode}"
        );
    }
}

// Covers: specs/world/objects-2.md §16.11; specs/world/objects.md §edge-cases-original-bugs r16, §edge-cases-original-bugs r17
#[test]
fn stairs_warp_searches_own_then_adjacent_rooms() {
    let mut t = tables();
    let tile = |f: &mut Fake, room: u32, unit: u32| {
        let u = UnitId(unit);
        f.room_unit_lists.entry(RoomId(room)).or_default().push(u);
        f.unit_types.insert(u, 5);
    };
    for fn_ in [47u8, 50] {
        // Mode 2, no tile in O's room: the adjacency rooms in order, O's
        // room skipped.
        let (mut c, mut f) = setup(CLASS, 2, fn_, &mut t);
        f.adjacent.insert(ROOM, vec![ROOM, RoomId(4), RoomId(5)]);
        tile(&mut f, 5, 52);
        tile(&mut f, 4, 51);
        assert_eq!(
            dispatch(&mut c, &t, &mut f, O, Some(P)),
            Ok(Dispatch::Done(1)),
            "fn {fn_}"
        );
        assert_eq!(others(&f), vec!["warp 20 51"]);
        // A tile in O's own room wins.
        tile(&mut f, 3, 50);
        f.calls.clear();
        dispatch(&mut c, &t, &mut f, O, Some(P)).unwrap();
        assert_eq!(others(&f), vec!["warp 20 50"]);
        // None anywhere: nothing, result 1 (not fatal, unlike the trap door).
        let (mut c, mut f) = setup(CLASS, 2, fn_, &mut t);
        assert_eq!(
            dispatch(&mut c, &t, &mut f, O, Some(P)),
            Ok(Dispatch::Done(1))
        );
        assert!(f.calls.is_empty());
    }
    // 50 from mode 0: returns 0 and never opens (47 opens).
    let (mut c, mut f) = setup(CLASS, 0, 50, &mut t);
    assert_eq!(
        dispatch(&mut c, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(0))
    );
    assert!(f.calls.is_empty());
}
