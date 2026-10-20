// Spec: specs/sim/unit-order.md
//! State-machine property test of the game unit lists
//! (`d2_sim::units::lists::UnitLists`) against a reference model written
//! from `unit-order.md`: GUID counters (§1.3, wrap 0xFFFFFFFF → 1), hash
//! buckets sorted by GUID descending and type iteration bucket by bucket
//! (§2), `SUNIT_Add` and removal bookkeeping (§3), act room lists
//! newest-activated first (§4), room unit lists newest arrival first and
//! room changes (§5), update queues (§6, incl. flag bit 2 and units
//! without a room), the client list (§7) and the allied counts (§5.2–
//! §5.3), under random sequences of allocation, removal, moves, queueing
//! and room / client changes. Calls the spec forbids (duplicate GUID,
//! unknown ids, a unit already in a room, act ≥ 5) must be rejected with
//! no state change.

use d2_sim::units::lists::HASH_BUCKETS;
use d2_sim::units::{ClientId, ListError, RoomId, UnitId, UnitLists, UnitType};
use proptest::prelude::*;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    EnsureAct(u8),
    CreateRoom(usize),
    Activate(usize),
    Deactivate(usize),
    FreeRoom(usize),
    NoUpdate(usize, bool),
    /// Counter allocation + `SUNIT_Add` (room: selector, or none).
    Spawn(UnitType, Option<usize>, bool),
    /// `SUNIT_Add` with a caller GUID (§1.4): a live unit's GUID
    /// (duplicate) or a fresh value.
    Restore(UnitType, Option<usize>, Option<usize>, u32),
    /// Allocation without `SUNIT_Add` (`alloc.rs`), added later.
    Alloc(UnitType, bool),
    AddAllocated(usize, Option<usize>),
    Remove(usize),
    ChangeRoom(usize, usize),
    RoomInsert(usize, usize),
    RoomRemove(usize),
    Queue(usize),
    Unqueue(usize),
    ClearQueue(usize),
    AddClient,
    RemoveClient(usize),
    /// Sets a counter near the wrap (§1.3).
    NearWrap(UnitType, u32),
    /// Calls with ids that name nothing.
    Unknown,
}

fn unit_type() -> impl Strategy<Value = UnitType> {
    prop::sample::select(UnitType::ALL.to_vec())
}

fn op() -> impl Strategy<Value = Op> {
    let s = any::<usize>;
    let room = || prop::option::weighted(0.8, any::<usize>());
    prop_oneof![
        1 => (0u8..7).prop_map(Op::EnsureAct),
        2 => s().prop_map(Op::CreateRoom),
        2 => s().prop_map(Op::Activate),
        1 => s().prop_map(Op::Deactivate),
        1 => s().prop_map(Op::FreeRoom),
        1 => (s(), any::<bool>()).prop_map(|(r, b)| Op::NoUpdate(r, b)),
        6 => (unit_type(), room(), any::<bool>()).prop_map(|(t, r, a)| Op::Spawn(t, r, a)),
        1 => (unit_type(), room(), prop::option::of(s()), any::<u32>())
            .prop_map(|(t, r, d, g)| Op::Restore(t, r, d, g)),
        2 => (unit_type(), any::<bool>()).prop_map(|(t, a)| Op::Alloc(t, a)),
        2 => (s(), room()).prop_map(|(u, r)| Op::AddAllocated(u, r)),
        3 => s().prop_map(Op::Remove),
        3 => (s(), s()).prop_map(|(u, r)| Op::ChangeRoom(u, r)),
        1 => (s(), s()).prop_map(|(u, r)| Op::RoomInsert(u, r)),
        1 => s().prop_map(Op::RoomRemove),
        3 => s().prop_map(Op::Queue),
        1 => s().prop_map(Op::Unqueue),
        1 => s().prop_map(Op::ClearQueue),
        1 => Just(Op::AddClient),
        1 => s().prop_map(Op::RemoveClient),
        1 => (unit_type(), 0u32..4).prop_map(|(t, d)| Op::NearWrap(t, d)),
        1 => Just(Op::Unknown),
    ]
}

#[derive(Clone, Debug)]
struct MUnit {
    id: UnitId,
    ty: UnitType,
    guid: u32,
    allied: bool,
    /// In the hash lists (added, not only allocated).
    hashed: bool,
    room: Option<RoomId>,
    queued: bool,
}

#[derive(Clone, Debug)]
struct MRoom {
    id: RoomId,
    act: u8,
    active: bool,
    no_update: bool,
    /// Room unit list, head first.
    units: Vec<UnitId>,
    /// Update queue, head first.
    queue: Vec<UnitId>,
}

#[derive(Clone, Debug, Default)]
struct Model {
    counters: [u32; 6],
    acts: [bool; 5],
    /// Act room lists, head first.
    act_rooms: [Vec<RoomId>; 5],
    pending_rooms: [bool; 5],
    pending_updates: [bool; 5],
    units: Vec<MUnit>,
    rooms: Vec<MRoom>,
    clients: Vec<ClientId>,
}

impl Model {
    fn unit(&mut self, id: UnitId) -> &mut MUnit {
        self.units
            .iter_mut()
            .find(|u| u.id == id)
            .expect("model unit")
    }

    fn room(&mut self, id: RoomId) -> &mut MRoom {
        self.rooms
            .iter_mut()
            .find(|r| r.id == id)
            .expect("model room")
    }

    fn alloc_guid(&mut self, ty: UnitType) -> u32 {
        let c = &mut self.counters[ty.index()];
        let mut next = c.wrapping_add(1);
        if next == u32::MAX {
            next = 1;
        }
        *c = next;
        next
    }

    fn guid_taken(&self, ty: UnitType, guid: u32) -> bool {
        self.units
            .iter()
            .any(|u| u.hashed && u.ty == ty && u.guid == guid)
    }

    /// §6.2.
    fn queue(&mut self, id: UnitId) {
        let u = self.unit(id).clone();
        let Some(r) = u.room else { return };
        if u.queued {
            return;
        }
        let room = self.room(r);
        if room.no_update {
            return;
        }
        room.queue.insert(0, id);
        let act = room.act;
        self.unit(id).queued = true;
        self.pending_updates[act as usize] = true;
    }

    /// §5.2 then §6.
    fn room_insert(&mut self, id: UnitId, r: RoomId) {
        self.room(r).units.insert(0, id);
        self.unit(id).room = Some(r);
        self.queue(id);
    }

    /// §5.3.
    fn room_remove(&mut self, id: UnitId) {
        let Some(r) = self.unit(id).room else { return };
        let room = self.room(r);
        room.units.retain(|&u| u != id);
        room.queue.retain(|&u| u != id);
        let u = self.unit(id);
        u.room = None;
        u.queued = false;
    }

    /// §3.1, after the duplicate check.
    fn add(&mut self, id: UnitId, room: Option<RoomId>) {
        if let Some(r) = room {
            self.room_insert(id, r);
        }
        self.unit(id).hashed = true;
        self.queue(id);
    }
}

fn pick<T: Copy>(v: &[T], s: usize) -> Option<T> {
    (!v.is_empty()).then(|| v[s % v.len()])
}

/// A model room selector: `None` = no room.
fn pick_room(m: &Model, s: Option<usize>) -> Option<RoomId> {
    let ids: Vec<RoomId> = m.rooms.iter().map(|r| r.id).collect();
    s.and_then(|s| pick(&ids, s))
}

fn apply(l: &mut UnitLists, m: &mut Model, op: Op) {
    let units: Vec<UnitId> = m.units.iter().map(|u| u.id).collect();
    let rooms: Vec<RoomId> = m.rooms.iter().map(|r| r.id).collect();
    match op {
        Op::EnsureAct(a) => {
            let got = l.ensure_act(a);
            if a < 5 {
                assert_eq!(got, Ok(()));
                m.acts[a as usize] = true;
            } else {
                assert_eq!(got, Err(ListError::UnknownAct(a)));
            }
        }
        Op::CreateRoom(s) => {
            let act = (s % 6) as u8;
            let got = l.create_room(act);
            if act < 5 && m.acts[act as usize] {
                let id = got.expect("room");
                m.rooms.push(MRoom {
                    id,
                    act,
                    active: false,
                    no_update: false,
                    units: Vec::new(),
                    queue: Vec::new(),
                });
            } else {
                assert_eq!(got, Err(ListError::UnknownAct(act)));
            }
        }
        Op::Activate(s) => {
            let Some(r) = pick(&rooms, s) else { return };
            l.activate_room(r).expect("room");
            let room = m.room(r);
            if !room.active {
                room.active = true;
                let act = room.act as usize;
                m.act_rooms[act].insert(0, r);
                m.pending_rooms[act] = true;
            }
        }
        Op::Deactivate(s) => {
            let Some(r) = pick(&rooms, s) else { return };
            l.deactivate_room(r).expect("room");
            let room = m.room(r);
            room.active = false;
            let act = room.act as usize;
            m.act_rooms[act].retain(|&x| x != r);
        }
        Op::FreeRoom(s) => {
            let Some(r) = pick(&rooms, s) else { return };
            let freed = l.free_room(r).expect("room");
            assert_eq!(freed.act, m.room(r).act);
            // The room record is gone (rooms.md §8.2): units still in
            // it are unlinked from it (§5.3), in list order.
            for u in m.room(r).units.clone() {
                m.room_remove(u);
            }
            let act = m.room(r).act as usize;
            m.act_rooms[act].retain(|&x| x != r);
            m.rooms.retain(|x| x.id != r);
        }
        Op::NoUpdate(s, on) => {
            let Some(r) = pick(&rooms, s) else { return };
            l.room_mut(r).unwrap().no_update = on;
            m.room(r).no_update = on;
        }
        Op::Spawn(ty, rs, allied) => {
            let room = pick_room(m, rs);
            let guid = l.guids.alloc(ty);
            assert_eq!(guid, m.alloc_guid(ty), "GUID counter (§1.3)");
            restore(l, m, ty, guid, room, allied);
        }
        Op::Restore(ty, rs, dup, fresh) => {
            let room = pick_room(m, rs);
            let same: Vec<u32> = m
                .units
                .iter()
                .filter(|u| u.hashed && u.ty == ty)
                .map(|u| u.guid)
                .collect();
            let guid = dup.and_then(|s| pick(&same, s)).unwrap_or(fresh);
            restore(l, m, ty, guid, room, false);
        }
        Op::Alloc(ty, allied) => {
            let guid = l.guids.alloc(ty);
            assert_eq!(guid, m.alloc_guid(ty));
            let id = l.alloc_unit(ty, guid, allied);
            m.units.push(MUnit {
                id,
                ty,
                guid,
                allied,
                hashed: false,
                room: None,
                queued: false,
            });
        }
        Op::AddAllocated(s, rs) => {
            let Some(u) = pick(&units, s) else { return };
            let room = pick_room(m, rs);
            let mu = m.unit(u).clone();
            let got = l.add_allocated(u, room);
            if m.guid_taken(mu.ty, mu.guid) {
                assert_eq!(
                    got,
                    Err(ListError::DuplicateGuid {
                        ty: mu.ty,
                        guid: mu.guid
                    })
                );
            } else if room.is_some() && mu.room.is_some() {
                // Placed by a room insert before (allocated, never added).
                assert_eq!(got, Err(ListError::AlreadyInRoom(u)));
            } else {
                assert_eq!(got, Ok(()));
                m.add(u, room);
            }
        }
        Op::Remove(s) => {
            let Some(u) = pick(&units, s) else { return };
            let e = l.remove_unit(u).expect("live unit");
            assert_eq!(e.guid, m.unit(u).guid);
            m.room_remove(u);
            m.units.retain(|x| x.id != u);
        }
        Op::ChangeRoom(s, rs) => {
            let (Some(u), Some(r)) = (pick(&units, s), pick(&rooms, rs)) else {
                return;
            };
            l.change_room(u, r).expect("live unit and room");
            m.room_remove(u);
            m.room_insert(u, r);
        }
        Op::RoomInsert(s, rs) => {
            let (Some(u), Some(r)) = (pick(&units, s), pick(&rooms, rs)) else {
                return;
            };
            let got = l.room_insert(u, r);
            if m.unit(u).room.is_some() {
                assert_eq!(got, Err(ListError::AlreadyInRoom(u)));
            } else {
                assert_eq!(got, Ok(()));
                m.room_insert(u, r);
            }
        }
        Op::RoomRemove(s) => {
            let Some(u) = pick(&units, s) else { return };
            l.room_remove(u).expect("live unit");
            m.room_remove(u);
        }
        Op::Queue(s) => {
            let Some(u) = pick(&units, s) else { return };
            l.queue_update(u).expect("live unit");
            m.queue(u);
        }
        Op::Unqueue(s) => {
            let Some(u) = pick(&units, s) else { return };
            l.unqueue_update(u).expect("live unit");
            if let Some(r) = m.unit(u).room {
                m.room(r).queue.retain(|&x| x != u);
            }
            m.unit(u).queued = false;
        }
        Op::ClearQueue(s) => {
            let Some(r) = pick(&rooms, s) else { return };
            l.clear_update_queue(r).expect("room");
            for u in std::mem::take(&mut m.room(r).queue) {
                m.unit(u).queued = false;
            }
        }
        Op::AddClient => {
            let c = l.add_client(None, None, 4);
            m.clients.insert(0, c);
        }
        Op::RemoveClient(s) => {
            let Some(c) = pick(&m.clients.clone(), s) else {
                return;
            };
            l.remove_client(c).expect("client");
            m.clients.retain(|&x| x != c);
        }
        Op::NearWrap(ty, d) => {
            let v = u32::MAX - 1 - d;
            l.guids.set(ty, v);
            m.counters[ty.index()] = v;
        }
        Op::Unknown => {
            let (u, r, c) = (UnitId(1 << 30), RoomId(1 << 30), ClientId(1 << 30));
            assert_eq!(l.remove_unit(u), Err(ListError::UnknownUnit(u)));
            assert_eq!(l.change_room(u, r), Err(ListError::UnknownRoom(r)));
            assert_eq!(l.queue_update(u), Err(ListError::UnknownUnit(u)));
            assert_eq!(l.unqueue_update(u), Err(ListError::UnknownUnit(u)));
            assert_eq!(l.room_remove(u), Err(ListError::UnknownUnit(u)));
            assert_eq!(l.add_allocated(u, None), Err(ListError::UnknownUnit(u)));
            assert_eq!(l.activate_room(r), Err(ListError::UnknownRoom(r)));
            assert_eq!(l.deactivate_room(r), Err(ListError::UnknownRoom(r)));
            assert_eq!(l.clear_update_queue(r), Err(ListError::UnknownRoom(r)));
            assert!(l.free_room(r).is_err());
            assert_eq!(l.remove_client(c), Err(ListError::UnknownClient(c)));
            assert_eq!(l.next_of_type(u), None);
            assert!(l.room_units(r).is_empty());
            // Tiles have one list and ignore the bucket.
            for ty in &UnitType::ALL[..5] {
                assert!(l.hash_bucket(*ty, HASH_BUCKETS).is_empty());
            }
            if let Some(&u) = units.first() {
                // Rejected (unit already placed or unknown room).
                assert!(l.room_insert(u, r).is_err());
                assert_eq!(
                    l.add_unit(UnitType::Monster, 1 << 31, Some(r), false),
                    Err(ListError::UnknownRoom(r))
                );
            }
        }
    }
}

/// `SUNIT_Add` of a new unit with `guid` (§3.1): a duplicate is the
/// fatal error of §2.1, rejected with no partial insert.
fn restore(
    l: &mut UnitLists,
    m: &mut Model,
    ty: UnitType,
    guid: u32,
    room: Option<RoomId>,
    allied: bool,
) {
    let got = l.add_unit(ty, guid, room, allied);
    if m.guid_taken(ty, guid) {
        assert_eq!(got, Err(ListError::DuplicateGuid { ty, guid }));
        return;
    }
    let id = got.expect("add");
    m.units.push(MUnit {
        id,
        ty,
        guid,
        allied,
        hashed: false,
        room: None,
        queued: false,
    });
    m.add(id, room);
}

fn check(l: &UnitLists, m: &Model) {
    for ty in UnitType::ALL {
        assert_eq!(l.guids.get(ty), m.counters[ty.index()]);
        // §2.1: each bucket sorted by GUID, descending; tiles one list.
        let mut want: Vec<&MUnit> = m.units.iter().filter(|u| u.hashed && u.ty == ty).collect();
        want.sort_by_key(|u| std::cmp::Reverse(u.guid));
        if ty == UnitType::Tile {
            let got = l.hash_bucket(ty, 0);
            assert_eq!(got, want.iter().map(|u| u.id).collect::<Vec<_>>());
            assert_eq!(l.units_of_type(ty), got);
        } else {
            let mut order = Vec::new();
            for b in 0..HASH_BUCKETS {
                let bucket: Vec<UnitId> = want
                    .iter()
                    .filter(|u| u.guid as usize % HASH_BUCKETS == b)
                    .map(|u| u.id)
                    .collect();
                assert_eq!(l.hash_bucket(ty, b), bucket, "{ty:?} bucket {b}");
                order.extend(bucket);
            }
            // §2.4: buckets in index order.
            assert_eq!(l.units_of_type(ty), order);
        }
        for u in &want {
            assert_eq!(l.find_unit(ty, u.guid), Some(u.id), "lookup §2.3");
        }
    }
    for a in 0..5u8 {
        assert_eq!(l.act(a).is_some(), m.acts[a as usize]);
        assert_eq!(l.active_rooms(a), m.act_rooms[a as usize], "act {a} rooms");
        if let Some(e) = l.act(a) {
            assert_eq!(e.pending_rooms, m.pending_rooms[a as usize]);
            assert_eq!(e.pending_updates, m.pending_updates[a as usize]);
        }
    }
    for r in &m.rooms {
        let e = l.room(r.id).expect("live room");
        assert_eq!(
            (e.act, e.is_active(), e.no_update),
            (r.act, r.active, r.no_update)
        );
        assert_eq!(l.room_units(r.id), r.units, "room unit list §5");
        assert_eq!(l.update_queue(r.id), r.queue, "update queue §6");
        let allied = r
            .units
            .iter()
            .filter(|&&u| m.units.iter().any(|x| x.id == u && x.allied))
            .count();
        assert_eq!(e.allied_count() as usize, allied, "allied count");
    }
    for u in &m.units {
        let e = l.unit(u.id).expect("live unit");
        assert_eq!((e.ty, e.guid, e.allied), (u.ty, u.guid, u.allied));
        assert_eq!(e.room(), u.room);
        assert_eq!(e.is_queued(), u.queued);
    }
    assert_eq!(l.clients(), m.clients, "client list §7");
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn unit_lists_match_the_model(ops in prop::collection::vec(op(), 0..150)) {
        let mut l = UnitLists::new();
        let mut m = Model::default();
        for op in ops {
            apply(&mut l, &mut m, op);
            check(&l, &m);
        }
    }
}

/// `unit-order.md` §1.3 wrap: the counter skips 0xFFFFFFFF and restarts
/// at 1; a wrapped GUID that is still live is the fatal duplicate of
/// §2.1 (edge case 2), rejected without a partial insert.
#[test]
fn guid_wrap_duplicate_is_rejected() {
    let mut l = UnitLists::new();
    l.ensure_act(0).unwrap();
    let r = l.create_room(0).unwrap();
    let g = l.guids.alloc(UnitType::Monster);
    let a = l.add_unit(UnitType::Monster, g, Some(r), false).unwrap();
    l.guids.set(UnitType::Monster, u32::MAX - 1);
    let g = l.guids.alloc(UnitType::Monster);
    assert_eq!(g, 1);
    assert_eq!(
        l.add_unit(UnitType::Monster, g, Some(r), true),
        Err(ListError::DuplicateGuid {
            ty: UnitType::Monster,
            guid: 1
        })
    );
    assert_eq!(l.room_units(r), [a]);
    assert_eq!(l.room(r).unwrap().allied_count(), 0);
}

/// Regression (found by `unit_lists_match_the_model`): freeing a room
/// with units in it (the DRLG path `remove_active_room`, `rooms.md`
/// §8.2) left them linked to the freed id; the next room reused the
/// slot, and removing an old unit rewrote the new room's list head.
#[test]
fn regress_free_room_unlinks_its_units() {
    let mut l = UnitLists::new();
    l.ensure_act(0).unwrap();
    let r = l.create_room(0).unwrap();
    l.activate_room(r).unwrap();
    let g = l.guids.alloc(UnitType::Monster);
    let a = l.add_unit(UnitType::Monster, g, Some(r), true).unwrap();
    let g = l.guids.alloc(UnitType::Monster);
    let b = l.add_unit(UnitType::Monster, g, Some(r), true).unwrap();
    l.free_room(r).unwrap();
    for u in [a, b] {
        assert_eq!(l.unit(u).unwrap().room(), None);
        assert!(!l.unit(u).unwrap().is_queued());
    }
    // The next room takes the freed slot.
    let r2 = l.create_room(0).unwrap();
    assert_eq!(r2, r);
    let g = l.guids.alloc(UnitType::Player);
    let p = l.add_unit(UnitType::Player, g, Some(r2), true).unwrap();
    l.remove_unit(b).unwrap();
    assert_eq!(l.room_units(r2), [p]);
    assert_eq!(l.update_queue(r2), [p]);
    assert_eq!(l.room(r2).unwrap().allied_count(), 1);
}
