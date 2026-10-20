use super::*;

fn guids(l: &UnitLists, ids: &[UnitId]) -> Vec<u32> {
    ids.iter().map(|&u| l.unit(u).unwrap().guid).collect()
}

fn one_room() -> (UnitLists, RoomId) {
    let mut l = UnitLists::new();
    l.ensure_act(0).unwrap();
    let r = l.create_room(0).unwrap();
    l.activate_room(r).unwrap();
    (l, r)
}

#[test]
fn guid_allocation_per_type() {
    // Vector: fresh game; monster, monster, item, monster → 1, 2, 1, 3.
    let mut g = GuidCounters::default();
    let got = [
        g.alloc(UnitType::Monster),
        g.alloc(UnitType::Monster),
        g.alloc(UnitType::Item),
        g.alloc(UnitType::Monster),
    ];
    assert_eq!(got, [1, 2, 1, 3]);
}

#[test]
fn guid_wraps_to_one() {
    // Vector: monster counter 0xFFFFFFFE; allocate → 1.
    let mut g = GuidCounters::default();
    g.set(UnitType::Monster, 0xFFFF_FFFE);
    assert_eq!(g.alloc(UnitType::Monster), 1);
    assert_eq!(g.get(UnitType::Monster), 1);
    assert_eq!(g.get(UnitType::Player), 0);
}

#[test]
fn hash_bucket_sorted_descending_any_order() {
    // Vector: monster GUIDs 1, 129, 257 into bucket 1 → [257, 129, 1].
    for order in [[1, 129, 257], [257, 1, 129], [129, 257, 1]] {
        let mut l = UnitLists::new();
        for g in order {
            l.add_unit(UnitType::Monster, g, None, false).unwrap();
        }
        assert_eq!(
            guids(&l, &l.hash_bucket(UnitType::Monster, 1)),
            [257, 129, 1]
        );
    }
}

#[test]
fn hash_iteration_order() {
    // Vector: players 1, 2, 128, 129 → 128, 129, 1, 2.
    let mut l = UnitLists::new();
    for g in [1, 2, 128, 129] {
        l.add_unit(UnitType::Player, g, None, true).unwrap();
    }
    assert_eq!(
        guids(&l, &l.units_of_type(UnitType::Player)),
        [128, 129, 1, 2]
    );
    // Edge case 1: 256, 128 (bucket 0), then 257, 129, 1 (bucket 1).
    let mut l = UnitLists::new();
    for g in [1, 128, 129, 256, 257] {
        l.add_unit(UnitType::Monster, g, None, false).unwrap();
    }
    assert_eq!(
        guids(&l, &l.units_of_type(UnitType::Monster)),
        [256, 128, 257, 129, 1]
    );
}

#[test]
fn hash_lists_separate_per_type_and_tiles_single_list() {
    let mut l = UnitLists::new();
    let m = l.add_unit(UnitType::Monster, 5, None, false).unwrap();
    let i = l.add_unit(UnitType::Item, 5, None, false).unwrap();
    for g in [3, 200, 7] {
        l.add_unit(UnitType::Tile, g, None, false).unwrap();
    }
    assert_eq!(l.find_unit(UnitType::Monster, 5), Some(m));
    assert_eq!(l.find_unit(UnitType::Item, 5), Some(i));
    assert_eq!(l.find_unit(UnitType::Missile, 5), None);
    assert_eq!(guids(&l, &l.hash_bucket(UnitType::Tile, 0)), [200, 7, 3]);
    assert_eq!(guids(&l, &l.units_of_type(UnitType::Tile)), [200, 7, 3]);
}

#[test]
fn duplicate_guid_is_fatal_and_changes_nothing() {
    // / edge case 2.
    let (mut l, r) = one_room();
    l.add_unit(UnitType::Monster, 9, Some(r), false).unwrap();
    assert_eq!(
        l.add_unit(UnitType::Monster, 9, Some(r), false),
        Err(ListError::DuplicateGuid {
            ty: UnitType::Monster,
            guid: 9
        })
    );
    assert_eq!(l.room_units(r).len(), 1);
}

#[test]
fn hash_remove_keeps_order() {
    let mut l = UnitLists::new();
    let ids: Vec<_> = [1, 129, 257, 385]
        .iter()
        .map(|&g| l.add_unit(UnitType::Object, g, None, false).unwrap())
        .collect();
    l.remove_unit(ids[2]).unwrap();
    assert_eq!(
        guids(&l, &l.hash_bucket(UnitType::Object, 1)),
        [385, 129, 1]
    );
    assert_eq!(l.find_unit(UnitType::Object, 257), None);
}

#[test]
fn room_list_prepends() {
    // Vector: add A, B, C → [C, B, A].
    let (mut l, r) = one_room();
    let a = l.add_unit(UnitType::Monster, 1, Some(r), false).unwrap();
    let b = l.add_unit(UnitType::Monster, 2, Some(r), false).unwrap();
    let c = l.add_unit(UnitType::Monster, 3, Some(r), false).unwrap();
    assert_eq!(l.room_units(r), [c, b, a]);
}

#[test]
fn walking_back_in_moves_to_head() {
    // Vector: [C, B, A]; B walks out and back in → [B, C, A].
    let (mut l, r) = one_room();
    let r2 = l.create_room(0).unwrap();
    l.activate_room(r2).unwrap();
    let a = l.add_unit(UnitType::Monster, 1, Some(r), false).unwrap();
    let b = l.add_unit(UnitType::Monster, 2, Some(r), false).unwrap();
    let c = l.add_unit(UnitType::Monster, 3, Some(r), false).unwrap();
    l.change_room(b, r2).unwrap();
    assert_eq!(l.room_units(r), [c, a]);
    l.change_room(b, r).unwrap();
    assert_eq!(l.room_units(r), [b, c, a]);
    assert!(l.room_units(r2).is_empty());
}

#[test]
fn room_activation_prepends() {
    // Vector: activate R1, R2, R3 in act 0 → [R3, R2, R1].
    let mut l = UnitLists::new();
    l.ensure_act(0).unwrap();
    let rs: Vec<_> = (0..3).map(|_| l.create_room(0).unwrap()).collect();
    for &r in &rs {
        l.activate_room(r).unwrap();
    }
    assert!(l.act(0).unwrap().pending_rooms);
    assert_eq!(l.active_rooms(0), [rs[2], rs[1], rs[0]]);
    l.deactivate_room(rs[1]).unwrap();
    assert_eq!(l.active_rooms(0), [rs[2], rs[0]]);
    assert!(!l.room(rs[1]).unwrap().is_active());
}

#[test]
fn update_queue_keeps_first_position() {
    // Vector: queue A, B, A in one tick → [B, A].
    let (mut l, r) = one_room();
    let a = l.add_unit(UnitType::Monster, 1, None, false).unwrap();
    let b = l.add_unit(UnitType::Monster, 2, None, false).unwrap();
    // Units added without a room are not queued.
    assert!(!l.unit(a).unwrap().is_queued());
    l.room_insert(a, r).unwrap(); // queues A
    l.room_insert(b, r).unwrap(); // queues B
    l.queue_update(a).unwrap(); // already queued: unchanged
    assert_eq!(l.update_queue(r), [b, a]);
    assert!(l.act(0).unwrap().pending_updates);
}

#[test]
fn update_queue_ignores_flag_bit_2() {
    // Edge case 4.
    let (mut l, r) = one_room();
    l.room_mut(r).unwrap().no_update = true;
    let a = l.add_unit(UnitType::Monster, 1, Some(r), false).unwrap();
    assert!(l.update_queue(r).is_empty());
    assert!(!l.unit(a).unwrap().is_queued());
    assert!(!l.act(0).unwrap().pending_updates);
}

/// A later alignment change (`set_allied`) keeps the room's allied
/// count in step, so the unit removal lowers what was raised.
#[test]
fn set_allied_keeps_the_room_count() {
    let (mut l, r) = one_room();
    let a = l.add_unit(UnitType::Monster, 1, Some(r), false).unwrap();
    assert_eq!(l.room(r).unwrap().allied_count(), 0);
    l.set_allied(a, true);
    l.set_allied(a, true);
    assert!(l.unit(a).unwrap().allied);
    assert_eq!(l.room(r).unwrap().allied_count(), 1);
    l.room_remove(a).unwrap();
    assert_eq!(l.room(r).unwrap().allied_count(), 0);
    // Out of a room only the flag changes.
    l.set_allied(a, false);
    assert!(!l.unit(a).unwrap().allied);
    assert_eq!(l.room(r).unwrap().allied_count(), 0);
}

#[test]
fn room_remove_unqueues_and_clear_resets_flags() {
    let (mut l, r) = one_room();
    let a = l.add_unit(UnitType::Player, 1, Some(r), true).unwrap();
    let b = l.add_unit(UnitType::Monster, 1, Some(r), true).unwrap();
    let c = l.add_unit(UnitType::Monster, 2, Some(r), false).unwrap();
    assert_eq!(l.room(r).unwrap().allied_count(), 2);
    l.room_remove(b).unwrap();
    assert_eq!(l.update_queue(r), [c, a]);
    assert!(!l.unit(b).unwrap().is_queued());
    assert_eq!(l.room(r).unwrap().allied_count(), 1);
    l.clear_update_queue(r).unwrap();
    assert!(l.update_queue(r).is_empty());
    assert!(!l.unit(a).unwrap().is_queued());
    // Queuing again after the clear works.
    l.queue_update(a).unwrap();
    assert_eq!(l.update_queue(r), [a]);
}

#[test]
fn removal_unlinks_everywhere() {
    let (mut l, r) = one_room();
    let a = l.add_unit(UnitType::Missile, 1, Some(r), false).unwrap();
    let b = l.add_unit(UnitType::Missile, 2, Some(r), false).unwrap();
    l.remove_unit(a).unwrap();
    assert_eq!(l.room_units(r), [b]);
    assert_eq!(l.update_queue(r), [b]);
    assert_eq!(l.units_of_type(UnitType::Missile), [b]);
    assert!(l.unit(a).is_none());
}

#[test]
fn client_list_prepends() {
    // Vector: X then Y → [Y, X].
    let mut l = UnitLists::new();
    let x = l.add_client(None, None, client_state::JOINING);
    let y = l.add_client(None, None, client_state::JOINING);
    assert_eq!(l.clients(), [y, x]);
    let z = l.add_client(None, None, client_state::JOINING);
    l.remove_client(y).unwrap();
    assert_eq!(l.clients(), [z, x]);
}

#[test]
fn added_during_iteration_lands_behind_walker() {
    // A head insert during an iteration is not visited.
    let (mut l, r) = one_room();
    l.add_unit(UnitType::Monster, 1, Some(r), false).unwrap();
    l.add_unit(UnitType::Monster, 2, Some(r), false).unwrap();
    let mut seen = Vec::new();
    let mut cur = l.room_unit_first(r);
    let mut next_guid = 10;
    while let Some(u) = cur {
        cur = l.room_unit_next(u);
        seen.push(l.unit(u).unwrap().guid);
        l.add_unit(UnitType::Monster, next_guid, Some(r), false)
            .unwrap();
        next_guid += 1;
    }
    assert_eq!(seen, [2, 1]);
}
