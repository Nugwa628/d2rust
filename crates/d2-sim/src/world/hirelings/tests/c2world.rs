//! Gap tests for the hireling death-with-owner node bookkeeping and the
//! dead-restore timer cancels.

use super::fake::Fake;
use crate::world::hirelings::life;
use crate::world::hirelings::{stat, HirelingState, PetList, PetNode, UNIT_PLAYER};

const M_GUID: u32 = 0x20;

fn node(dead: bool) -> PetNode {
    PetNode {
        dead,
        guid: M_GUID,
        seed: 0x1234,
        name: 4000,
        id: 1,
    }
}

#[test]
fn player_death_keeps_the_node_and_the_count() {
    let mut w = Fake::new(false);
    let p = w.add(1, UNIT_PLAYER, 0, 1);
    let _m = w.add(10, 1, 271, M_GUID);
    w.set(p, stat::LEVEL, 10);
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(false)],
            max: 1,
        },
    );
    life::player_death(&mut w, &mut st, p);
    let l = st.list(p).unwrap();
    // The node stays (now dead) so it can be revived / replaced; the
    // count and the maximum are unchanged.
    assert_eq!(l.nodes.len(), 1);
    assert_eq!(l.max, 1);
    assert!(l.nodes[0].dead);
}

#[test]
fn dead_restored_hireling_has_ai_and_regen_timers_cancelled() {
    let mut w = Fake::new(true);
    let p = w.add(1, UNIT_PLAYER, 0, 1);
    let m = w.add(10, 1, 271, M_GUID);
    w.set(p, stat::LEVEL, 10);
    let mut st = HirelingState::default();
    st.lists.insert(
        p,
        PetList {
            nodes: vec![node(false)],
            max: 1,
        },
    );
    life::restore_dead(&mut w, &mut st, p, m);
    assert!(w.log.contains(&"cancel 10 2 0".to_string()));
    assert!(w.log.contains(&"cancel 10 3 0".to_string()));
}
