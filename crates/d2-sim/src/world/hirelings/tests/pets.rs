//! The hireling pet list and the 0x81 / 0x7A / 0x9B encoders.

use super::fake::{act1_ice_rows, tables, Fake};
use crate::units::UnitId;
use crate::world::hirelings::pets::{
    self, assign_merc, merc_dead_message, pet_action, ACTION_REMOVE,
};
use crate::world::hirelings::{flags, HirelingState, PetList, PetNode, UNIT_PLAYER};

const P_GUID: u32 = 1;
const M_GUID: u32 = 0x20;

/// Two players (ids 1, 2; GUIDs 1, 2) and one hireling unit (id 10,
/// class 271, GUID 0x20).
fn world() -> (Fake, UnitId, UnitId, UnitId) {
    let mut w = Fake::new(true);
    let p = w.add(1, UNIT_PLAYER, 0, P_GUID);
    let q = w.add(2, UNIT_PLAYER, 0, 2);
    let m = w.add(10, 1, 271, M_GUID);
    (w, p, q, m)
}

fn node(guid: u32, dead: bool) -> PetNode {
    PetNode {
        dead,
        guid,
        seed: 0x1234,
        name: 0x0F21,
        id: 1,
    }
}

fn remove_msg(guid: u32) -> Vec<u8> {
    pet_action(ACTION_REMOVE, 0, 0, guid, 0).to_vec()
}

#[test]
fn assign_merc_layout() {
    let b = assign_merc(271, 1, 0x20, 0x0102_0304, 0x0F21);
    assert_eq!(
        b,
        [0x81, 7, 0x0F, 0x01, 1, 0, 0, 0, 0x20, 0, 0, 0, 4, 3, 2, 1, 0x21, 0x0F, 0, 0]
    );
}

#[test]
fn pet_action_vector() {
    // Test vector: add, pet GUID 5, owner 1, class 363, type 4
    // → owner @5, pet @9.
    assert_eq!(
        pet_action(1, 4, 363, 5, 1),
        [0x7A, 0x01, 0x04, 0x6B, 0x01, 0x01, 0, 0, 0, 0x05, 0, 0, 0]
    );
    // Measured (Save And Exit with the merc
    // alive): the remove carries only the pet GUID, @9.
    assert_eq!(
        remove_msg(0x0D),
        [0x7A, 0, 0, 0, 0, 0, 0, 0, 0, 0x0D, 0, 0, 0]
    );
}

#[test]
fn merc_dead_layout() {
    assert_eq!(merc_dead_message(0xFFFF, 0), [0x9B, 0xFF, 0xFF, 0, 0, 0, 0]);
    assert_eq!(
        merc_dead_message(0x0F21, 6750),
        [0x9B, 0x21, 0x0F, 0x5E, 0x1A, 0, 0]
    );
}

#[test]
fn add_sets_max_and_broadcasts_assign_merc() {
    let (mut w, p, q, m) = world();
    let t = tables(act1_ice_rows());
    let mut st = HirelingState::default();
    assert_eq!(
        pets::add(&mut w, &t, &mut st, p, m, 0x1234, 0x0F21, 1),
        Ok(true)
    );
    let list = st.list(p).unwrap();
    assert_eq!(list.max, 1);
    assert_eq!(list.nodes, vec![node(M_GUID, false)]);
    let msg = assign_merc(271, P_GUID, M_GUID, 0x1234, 0x0F21).to_vec();
    assert_eq!(w.sent_to(p), vec![msg.clone()]);
    assert_eq!(w.sent_to(q), vec![msg]);
}

#[test]
fn add_without_seed_or_name_sends_pet_action_add() {
    let (mut w, p, _, m) = world();
    let t = tables(act1_ice_rows());
    let mut st = HirelingState::default();
    assert_eq!(pets::add(&mut w, &t, &mut st, p, m, 0, 0, 1), Ok(true));
    assert_eq!(
        w.sent_to(p),
        vec![pet_action(1, 7, 271, M_GUID, P_GUID).to_vec()]
    );
}

#[test]
fn add_with_zero_max_dismisses() {
    let (mut w, p, q, m) = world();
    let mut t = tables(act1_ice_rows());
    t.pet_basemax = 0;
    let mut st = HirelingState::default();
    assert_eq!(
        pets::add(&mut w, &t, &mut st, p, m, 0x1234, 0x0F21, 1),
        Ok(false)
    );
    assert!(st.list(p).unwrap().nodes.is_empty());
    assert_ne!(w.unit(m).flags & flags::NOXP, 0);
    assert_eq!(w.log, vec!["dismiss 10".to_string()]);
    assert_eq!(w.sent_to(p), vec![remove_msg(M_GUID)]);
    assert_eq!(w.sent_to(q), vec![remove_msg(M_GUID)]);
}

#[test]
fn add_when_full_kills_the_oldest() {
    let (mut w, p, _, m) = world();
    let old = w.add(11, 1, 271, 0x21);
    let t = tables(act1_ice_rows());
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(0x21, true)],
            max: 1,
        },
    );
    assert_eq!(
        pets::add(&mut w, &t, &mut st, p, m, 0x1234, 0x0F21, 1),
        Ok(true)
    );
    assert_eq!(st.list(p).unwrap().nodes, vec![node(M_GUID, false)]);
    assert_ne!(w.unit(old).flags & flags::NOXP, 0);
    assert_eq!(w.log, vec!["dismiss 11".to_string()]);
    // Unlink's remove, dismiss's remove, then the add.
    assert_eq!(
        w.sent_to(p),
        vec![
            remove_msg(0x21),
            remove_msg(0x21),
            assign_merc(271, P_GUID, M_GUID, 0x1234, 0x0F21).to_vec()
        ]
    );
}

#[test]
fn find_living_and_any() {
    let (mut w, p, _, m) = world();
    let mut st = HirelingState::default();
    assert_eq!(pets::any(&w, &st, p), None);
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(M_GUID, true)],
            max: 1,
        },
    );
    assert_eq!(pets::living(&w, &st, p), None);
    assert_eq!(pets::any(&w, &st, p), Some(m));
    st.list_mut(p).nodes[0].dead = false;
    assert_eq!(pets::living(&w, &st, p), Some(m));
    // A node whose unit is gone maps to none.
    w.units.remove(&m);
    assert_eq!(pets::any(&w, &st, p), None);
    assert!(st.first_node(p, true).is_some());
}

#[test]
fn remove_without_kill_clears_owned_and_broadcasts_twice() {
    let (mut w, p, q, m) = world();
    w.unit_mut(m).flags = flags::OWNED | flags::INIT;
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(M_GUID, false)],
            max: 1,
        },
    );
    assert_eq!(pets::remove(&mut w, &mut st, p, M_GUID, false), Ok(()));
    assert!(st.list(p).unwrap().nodes.is_empty());
    assert_eq!(w.unit(m).flags, flags::INIT);
    assert!(w.log.is_empty());
    assert_eq!(w.sent_to(p), vec![remove_msg(M_GUID), remove_msg(M_GUID)]);
    assert_eq!(w.sent_to(q), vec![remove_msg(M_GUID), remove_msg(M_GUID)]);
}

#[test]
fn remove_with_kill_dismisses() {
    let (mut w, p, _, m) = world();
    w.unit_mut(m).flags = flags::OWNED;
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(M_GUID, false)],
            max: 1,
        },
    );
    assert_eq!(pets::remove(&mut w, &mut st, p, M_GUID, true), Ok(()));
    assert!(st.list(p).unwrap().nodes.is_empty());
    assert_eq!(w.unit(m).flags, flags::OWNED | flags::NOXP);
    assert_eq!(w.log, vec!["dismiss 10".to_string()]);
    // Unlink, dismiss, Remove.
    assert_eq!(w.sent_to(p), vec![remove_msg(M_GUID); 3]);
}

#[test]
fn mark_living_clears_dead_and_broadcasts() {
    let (mut w, p, q, _) = world();
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(M_GUID, true)],
            max: 1,
        },
    );
    assert!(pets::mark_living(&mut w, &mut st, p, M_GUID));
    assert!(!st.list(p).unwrap().nodes[0].dead);
    let msg = assign_merc(271, P_GUID, M_GUID, 0x1234, 0x0F21).to_vec();
    assert_eq!(w.sent_to(p), vec![msg.clone()]);
    assert_eq!(w.sent_to(q), vec![msg]);
    assert!(!pets::mark_living(&mut w, &mut st, p, 0x99));
}

#[test]
fn join_sync_sends_living_nodes_with_a_unit() {
    let (mut w, p, q, _) = world();
    w.add(11, 1, 338, 0x21);
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            // Living with unit, dead with unit, living without unit.
            nodes: vec![node(M_GUID, false), node(0x21, true), node(0x22, false)],
            max: 3,
        },
    );
    pets::join_sync(&mut w, &st, p, q);
    assert_eq!(
        w.sent_to(q),
        vec![assign_merc(271, P_GUID, M_GUID, 0x1234, 0x0F21).to_vec()]
    );
    assert!(w.sent_to(p).is_empty());
}
