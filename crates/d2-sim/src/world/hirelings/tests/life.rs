//! Init, replace, death, revive, follow, the classic act change and the
//! restore.

use super::fake::{row, tables, Fake};
use crate::units::UnitId;
use crate::world::hirelings::life::{
    self, Loader, RestorePlan, RestoreSkip, SavedHireling, MODE_DEAD, MODE_NEUTRAL,
};
use crate::world::hirelings::pets::{assign_merc, merc_dead_message, pet_action};
use crate::world::hirelings::{
    flags, stat, HirelingRow, HirelingState, HirelingTables, PetList, PetNode, Slot, NEW_HIRE,
    UNIT_PLAYER,
};

const P_GUID: u32 = 1;
const M_GUID: u32 = 0x20;
const OLD_GUID: u32 = 0x21;

/// Expansion Act 1 Normal: Id 0 (bracket 3) and Id 1 (brackets 3 / 36 /
/// 67), Exp/Lvl 105, names 100–140 (act 0).
fn act1_rows() -> Vec<HirelingRow> {
    let mk = |id: u32, level: i32| HirelingRow {
        exp_lvl: 105,
        name_first: 100,
        name_last: 140,
        ..row(100, id, 1, 1, level)
    };
    vec![mk(0, 3), mk(1, 3), mk(1, 36), mk(1, 67)]
}

fn act1_tables() -> HirelingTables {
    tables(act1_rows())
}

/// Two players (ids 1, 2; GUIDs 1, 2; player 1 level 10) and the new
/// hireling unit (id 10, class 271, GUID 0x20).
fn world() -> (Fake, UnitId, UnitId, UnitId) {
    let mut w = Fake::new(true);
    let p = w.add(1, UNIT_PLAYER, 0, P_GUID);
    let q = w.add(2, UNIT_PLAYER, 0, 2);
    let m = w.add(10, 1, 271, M_GUID);
    w.set(p, stat::LEVEL, 10);
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

fn with_nodes(player: UnitId, nodes: Vec<PetNode>) -> HirelingState {
    let mut st = HirelingState::default();
    let max = nodes.len() as i32;
    st.lists.insert(player, PetList { nodes, max });
    st
}

fn remove_msg(guid: u32) -> Vec<u8> {
    pet_action(0, 0, 0, guid, 0).to_vec()
}

/// Asserts that `want` appears in `log` in this order (other entries may
/// sit between them).
fn assert_in_order(log: &[String], want: &[&str]) {
    let mut it = log.iter();
    for w in want {
        assert!(
            it.any(|l| l == w),
            "`{w}` missing or out of order in {log:?}"
        );
    }
}

#[test]
fn death_sends_name_and_cost() {
    let (mut w, p, q, m) = world();
    w.set(m, stat::LEVEL, 30);
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    assert!(life::death(&mut w, &mut st, p, m));
    assert!(st.list(p).unwrap().nodes[0].dead);
    // Test vector: name id 0x0F21, level 30 → 6750.
    assert_eq!(
        w.sent_to(p),
        vec![
            vec![0x9b, 0x21, 0x0f, 0x5e, 0x1a, 0x00, 0x00],
            remove_msg(M_GUID)
        ]
    );
    assert_eq!(w.sent_to(q), vec![remove_msg(M_GUID)]);
    // A unit in no hireling node.
    let other = w.add(11, 1, 271, OLD_GUID);
    assert!(!life::death(&mut w, &mut st, p, other));
}

#[test]
fn kill_marks_the_node_of_a_player_owned_hireling() {
    let (mut w, p, _, m) = world();
    w.set(m, stat::LEVEL, 30);
    // Flag 0 (the expired-pet kill): nothing.
    w.unit_mut(m).owner = Some((P_GUID, UNIT_PLAYER));
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    assert!(!life::on_kill(&mut w, &mut st, m, false));
    assert!(!st.list(p).unwrap().nodes[0].dead);
    assert!(w.sent.is_empty());
    // No owner, or an owner that is not a player: nothing.
    w.unit_mut(m).owner = None;
    assert!(!life::on_kill(&mut w, &mut st, m, true));
    w.unit_mut(m).owner = Some((P_GUID, 1));
    assert!(!life::on_kill(&mut w, &mut st, m, true));
    assert!(w.sent.is_empty());
    // Flag 1 and a player owner: the death.
    w.unit_mut(m).owner = Some((P_GUID, UNIT_PLAYER));
    assert!(life::on_kill(&mut w, &mut st, m, true));
    assert!(st.list(p).unwrap().nodes[0].dead);
    assert_eq!(
        w.sent_to(p),
        vec![
            vec![0x9b, 0x21, 0x0f, 0x5e, 0x1a, 0x00, 0x00],
            remove_msg(M_GUID)
        ]
    );
}

#[test]
fn init_replaces_the_old_hireling() {
    let (mut w, p, q, m) = world();
    let old = w.add(11, 1, 271, OLD_GUID);
    w.unit_mut(old).flags = flags::OWNED;
    w.unit_mut(old).owner = Some((P_GUID, 0));
    let t = act1_tables();
    // The old hireling is dead: "any" node counts.
    let mut st = with_nodes(p, vec![node(OLD_GUID, true)]);
    let slot = Slot {
        name: 105,
        seed: 22_752_887,
    };
    // A saved id (no level stats): only the replace and the link run.
    let offer = life::init(&mut w, &t, &mut st, p, m, 1, slot, false)
        .unwrap()
        .expect("offer");
    assert_eq!(offer.id, 1);
    assert!(!w.units.contains_key(&old));
    assert_in_order(
        &w.log,
        &[
            "state_stat 10 105 172 2",
            "team 10 1",
            "owner 11 0xffffffff 1",
            "free 11",
            "owner 10 0x1 0",
            "ai 10",
        ],
    );
    assert!(!w.log.iter().any(|l| l.starts_with("set 10 13")));
    assert_eq!(w.unit(m).flags, flags::INIT | flags::OWNED);
    assert_eq!(w.unit(m).owner, Some((P_GUID, UNIT_PLAYER)));
    assert_eq!(
        st.list(p).unwrap().nodes,
        vec![PetNode {
            dead: false,
            guid: M_GUID,
            seed: 22_752_887,
            name: 105,
            id: 1,
        }]
    );
    let add = assign_merc(271, P_GUID, M_GUID, 22_752_887, 105).to_vec();
    // 0x9B ffff/0 first, then the two removes of the old node, then the
    // add.
    assert_eq!(
        w.sent_to(p),
        vec![
            merc_dead_message(0xFFFF, 0).to_vec(),
            remove_msg(OLD_GUID),
            remove_msg(OLD_GUID),
            add.clone()
        ]
    );
    assert_eq!(
        w.sent_to(q),
        vec![remove_msg(OLD_GUID), remove_msg(OLD_GUID), add]
    );
}

#[test]
fn init_new_hire_sets_offer_experience_and_level() {
    let (mut w, p, _, m) = world();
    let t = act1_tables();
    let mut st = HirelingState::default();
    let slot = Slot {
        name: 105,
        seed: 22_752_887,
    };
    let offer = life::init(&mut w, &t, &mut st, p, m, NEW_HIRE, slot, false)
        .unwrap()
        .expect("offer");
    // Test vector: seed 22752887, 2 candidates, player level 10 → Id 1,
    // L 6; experience (6 + 1)·105·6·6 = 26460.
    assert_eq!((offer.id, offer.level, offer.experience), (1, 6, 26460));
    assert_eq!(w.base(m, stat::EXPERIENCE), 26460);
    assert_eq!(w.base(m, stat::LEVEL), 6);
    assert_in_order(
        &w.log,
        &["owner 10 0x1 0", "set 10 13 26460", "set 10 12 6", "ai 10"],
    );
    // No old hireling: no 0x9B.
    assert!(w.sent_to(p).iter().all(|b| b[0] != 0x9B));
}

#[test]
fn init_without_offer_or_player_stops() {
    let (mut w, p, _, m) = world();
    let t = act1_tables();
    let mut st = HirelingState::default();
    // Hell has no candidate rows.
    w.difficulty = 2;
    let slot = Slot { name: 105, seed: 7 };
    assert_eq!(
        life::init(&mut w, &t, &mut st, p, m, NEW_HIRE, slot, false),
        Ok(None)
    );
    assert!(st.list(p).is_none_or(|l| l.nodes.is_empty()));
    assert_eq!(w.unit(m).owner, None);
    assert_eq!(w.unit(m).flags, flags::INIT);
    // The "player" is a monster: nothing happens.
    let mut w2 = Fake::new(true);
    let not_player = w2.add(3, 1, 271, 3);
    let m2 = w2.add(10, 1, 271, M_GUID);
    assert_eq!(
        life::init(&mut w2, &t, &mut st, not_player, m2, NEW_HIRE, slot, false),
        Ok(None)
    );
    assert_eq!(w2.unit(m2).flags, 0);
    assert!(w2.log.is_empty());
}

#[test]
fn revive_order() {
    let (mut w, p, q, m) = world();
    let other = w.add(11, 1, 271, OLD_GUID);
    w.unit_mut(m).mode = u32::from(MODE_DEAD);
    w.unit_mut(m).max_life = Some(5000);
    w.unit_mut(m).flags = flags::OWNED;
    w.unit_mut(m).flags2 = flags::REVIVE_CLEAR2 | 1;
    let t = act1_tables();
    let mut st = with_nodes(p, vec![node(M_GUID, true), node(OLD_GUID, false)]);
    life::revive(&mut w, &t, &mut st, p, m).unwrap();
    // Step 3: the living hireling leaves first.
    assert!(!w.units.contains_key(&other));
    assert_in_order(
        &w.log,
        &[
            "owner 11 0xffffffff 1",
            "free 11",
            "mode 10 1",
            "set 10 6 5000",
            "team 10 1",
            "remove_state 10 1",
            "remove_state 10 107",
            "remove_state 10 154",
            "reapply 10",
            "warp 10 1",
        ],
    );
    assert_eq!(st.list(p).unwrap().nodes, vec![node(M_GUID, false)]);
    assert_eq!(w.unit(m).flags, flags::OWNED | flags::REVIVE);
    assert_eq!(w.unit(m).flags2, 1 | flags::WARP2);
    let add = assign_merc(271, P_GUID, M_GUID, 0x1234, 0x0F21).to_vec();
    assert_eq!(
        w.sent_to(q),
        vec![remove_msg(OLD_GUID), remove_msg(OLD_GUID), add.clone()]
    );
    // The stats are queued on the merc (14 records), not sent.
    let to_p = w.sent_to(p);
    assert_eq!(to_p, [remove_msg(OLD_GUID), remove_msg(OLD_GUID), add]);
    assert_eq!(w.queued[&m].len(), 14);
}

#[test]
fn follow_warps_living_only() {
    let (mut w, p, _, m) = world();
    let dead = w.add(11, 1, 271, OLD_GUID);
    let t = act1_tables();
    let mut st = with_nodes(p, vec![node(M_GUID, false), node(OLD_GUID, true)]);
    life::follow(&mut w, &t, &mut st, p);
    assert_eq!(w.log, vec!["warp 10 1".to_string()]);
    assert_eq!(w.unit(m).flags2, flags::WARP2);
    assert_eq!(w.unit(dead).flags2, 0);
    assert_eq!(st.list(p).unwrap().nodes.len(), 2);
}

#[test]
fn follow_without_warp_or_range_frees_nodes() {
    let (mut w, p, _, m) = world();
    let mut t = act1_tables();
    t.pet_flags = 0;
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::follow(&mut w, &t, &mut st, p);
    assert!(st.list(p).unwrap().nodes.is_empty());
    // Freed with its unit.
    assert_eq!(w.log, vec!["free 10".to_string()]);
    assert!(!w.units.contains_key(&m));
}

#[test]
fn classic_act_change_marks_dead() {
    let (mut w, p, q, m) = world();
    w.expansion = false;
    w.set(m, stat::LEVEL, 30);
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::classic_act_change(&mut w, &mut st, p);
    assert_eq!(st.list(p).unwrap().nodes, vec![node(M_GUID, true)]);
    assert_eq!(
        w.log,
        vec!["room_remove 10".to_string(), "death_event 10".to_string()]
    );
    assert_eq!(
        w.sent_to(p),
        vec![remove_msg(M_GUID), merc_dead_message(0x0F21, 6750).to_vec()]
    );
    assert_eq!(w.sent_to(q), vec![remove_msg(M_GUID)]);
    // Again: the node is dead now (kept; no 0x9B, no death event).
    w.log.clear();
    w.sent.clear();
    life::classic_act_change(&mut w, &mut st, p);
    assert_eq!(w.log, vec!["room_remove 10".to_string()]);
    assert_eq!(w.sent_to(p), vec![remove_msg(M_GUID)]);
    assert_eq!(st.list(p).unwrap().nodes.len(), 1);
}

#[test]
fn restore_plan_clamps_name_and_checks_act() {
    let t = act1_tables();
    let saved = SavedHireling {
        dead: false,
        seed: 0x1234,
        name_index: 5,
        id: 1,
        experience: 26460,
    };
    assert_eq!(
        life::restore_plan(&t, true, 0, &saved, 3, Loader::Current),
        Ok(RestorePlan {
            name: 105,
            mode: MODE_NEUTRAL,
            class: 271,
            id: 1,
            seller: 150,
            loader: Loader::Current,
        })
    );
    // 100 + 40 = NameLast stays; 100 + 41 → NameFirst.
    let last = SavedHireling {
        name_index: 40,
        ..saved
    };
    assert_eq!(
        life::restore_plan(&t, true, 0, &last, 0, Loader::Current)
            .unwrap()
            .name,
        140
    );
    let above = SavedHireling {
        name_index: 41,
        dead: true,
        ..saved
    };
    assert_eq!(
        life::restore_plan(&t, true, 0, &above, 0, Loader::Current),
        Ok(RestorePlan {
            name: 100,
            mode: MODE_DEAD,
            class: 271,
            id: 1,
            seller: 150,
            loader: Loader::Current,
        })
    );
    // Step 1.
    let none = SavedHireling {
        dead: true,
        id: 1,
        ..SavedHireling::default()
    };
    assert_eq!(
        life::restore_plan(&t, true, 0, &none, 0, Loader::Current),
        Err(RestoreSkip::NoHireling)
    );
    // Step 2: no row of the Id.
    let bad = SavedHireling { id: 9, ..saved };
    assert_eq!(
        life::restore_plan(&t, true, 0, &bad, 0, Loader::Current),
        Err(RestoreSkip::NoRow)
    );
    // The old loader fails the load instead.
    assert_eq!(
        life::restore_plan(&t, true, 0, &bad, 0, Loader::Old),
        Err(RestoreSkip::LoadFailed)
    );
    // Step 3: classic, only in the name's act (act 0).
    let classic = tables(
        act1_rows()
            .into_iter()
            .map(|r| HirelingRow { version: 0, ..r })
            .collect(),
    );
    assert_eq!(
        life::restore_plan(&classic, false, 0, &saved, 1, Loader::Current),
        Err(RestoreSkip::OtherAct)
    );
    assert_eq!(
        life::restore_plan(&classic, false, 0, &saved, 0, Loader::Current)
            .unwrap()
            .name,
        105
    );
}

#[test]
fn restore_sets_node_values_and_dead() {
    let (mut w, p, _, m) = world();
    let t = act1_tables();
    let mut st = HirelingState::default();
    let saved = SavedHireling {
        dead: true,
        seed: 0x5555,
        name_index: 5,
        id: 1,
        experience: 26460,
    };
    let plan = life::restore_plan(&t, true, 0, &saved, 0, Loader::Current).unwrap();
    assert!(life::restore(&mut w, &t, &mut st, p, m, &plan, &saved).unwrap());
    let want = PetNode {
        dead: false,
        guid: M_GUID,
        seed: 0x5555,
        name: 105,
        id: 1,
    };
    assert_eq!(st.list(p).unwrap().nodes, vec![want]);
    // Dead restore: no team join; no experience reset, no level stats.
    assert!(!w.log.iter().any(|l| l.starts_with("team")));
    assert!(!w.log.iter().any(|l| l.starts_with("set 10")));
    assert_eq!(w.unit(m).flags, flags::INIT | flags::OWNED);

    w.sent.clear();
    w.set(m, stat::LEVEL, 30);
    life::restore_dead(&mut w, &mut st, p, m);
    assert!(st.list(p).unwrap().nodes[0].dead);
    assert_eq!(w.unit(m).flags, flags::INIT | flags::OWNED | flags::DEAD);
    assert_eq!(w.unit(m).mode, u32::from(MODE_DEAD));
    assert_eq!(
        w.sent_to(p),
        vec![merc_dead_message(105, 6750).to_vec(), remove_msg(M_GUID)]
    );
    // The AI and regeneration timers are cancelled after the mode.
    assert_in_order(&w.log, &["mode 10 12", "cancel 10 2 0", "cancel 10 3 0"]);
}

#[test]
fn death_leaves_the_corpse_with_its_stats() {
    let (mut w, p, _, m) = world();
    w.set(m, stat::LEVEL, 30);
    w.set(m, stat::EXPERIENCE, 1_000_000);
    w.unit_mut(m).flags = flags::OWNED;
    w.unit_mut(m).mode = 0;
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    assert!(life::death(&mut w, &mut st, p, m));
    // The unit stays (no free, no kill, nothing dropped), with its level
    // and experience; the dead flag and mode are the death mode's, not
    // this path's.
    assert!(w.units.contains_key(&m));
    assert!(w.log.is_empty(), "{:?}", w.log);
    assert_eq!(w.base(m, stat::LEVEL), 30);
    assert_eq!(w.base(m, stat::EXPERIENCE), 1_000_000);
    assert_eq!((w.unit(m).flags, w.unit(m).mode), (flags::OWNED, 0));
    // The restore path sets the dead flag itself.
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::restore_dead(&mut w, &mut st, p, m);
    assert_eq!(w.unit(m).flags, flags::OWNED | flags::DEAD);
    assert!(w.units.contains_key(&m));
}

#[test]
fn player_death_kills_the_hireling_in_every_game_type() {
    // Test vector: living hireling L 20 in a room, name id 4000.
    for expansion in [true, false] {
        let (mut w, p, q, m) = world();
        w.expansion = expansion;
        w.set(m, stat::LEVEL, 20);
        let mut st = with_nodes(
            p,
            vec![PetNode {
                name: 4000,
                ..node(M_GUID, false)
            }],
        );
        life::player_death(&mut w, &mut st, p);
        assert!(st.list(p).unwrap().nodes[0].dead);
        // Room notice then the death mode request; the unit is not freed.
        assert_eq!(
            w.log,
            vec!["room_remove 10".to_string(), "death_event 10".to_string()]
        );
        assert!(w.units.contains_key(&m));
        let dead = merc_dead_message(4000, 3000).to_vec();
        assert_eq!(dead, [0x9b, 0xa0, 0x0f, 0xb8, 0x0b, 0x00, 0x00]);
        assert_eq!(w.sent_to(p), vec![remove_msg(M_GUID), dead]);
        assert_eq!(w.sent_to(q), vec![remove_msg(M_GUID)]);
    }
}

#[test]
fn player_death_with_a_dead_or_missing_hireling() {
    // Already dead: 0x7A and the room notice only.
    let (mut w, p, _, _) = world();
    let mut st = with_nodes(p, vec![node(M_GUID, true)]);
    life::player_death(&mut w, &mut st, p);
    assert_eq!(w.log, vec!["room_remove 10".to_string()]);
    assert_eq!(w.sent_to(p), vec![remove_msg(M_GUID)]);
    // Living but out of any room: no death mode request, 0x9B still sent.
    let (mut w, p, _, m) = world();
    w.unit_mut(m).in_room = false;
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::player_death(&mut w, &mut st, p);
    assert_eq!(w.log, vec!["room_remove 10".to_string()]);
    assert_eq!(w.sent_to(p).len(), 2);
    assert!(st.list(p).unwrap().nodes[0].dead);
    // No unit for the GUID: the node is left as it is.
    let (mut w, p, _, _) = world();
    let mut st = with_nodes(p, vec![node(0x77, false)]);
    life::player_death(&mut w, &mut st, p);
    assert!(w.log.is_empty() && w.sent.is_empty());
    assert_eq!(st.list(p).unwrap().nodes, vec![node(0x77, false)]);
}

#[test]
fn owner_kill_makes_the_living_node_the_head() {
    let (mut w, p, _, _) = world();
    w.add(11, 1, 271, OLD_GUID);
    let mut st = with_nodes(p, vec![node(OLD_GUID, true), node(M_GUID, false)]);
    life::kill_with_owner(&mut w, &mut st, p);
    assert_eq!(st.list(p).unwrap().nodes, vec![node(M_GUID, true)]);
    assert_eq!(st.list(p).unwrap().max, 2);
}

#[test]
fn range_pets_farther_than_1600_are_removed_with_kill() {
    assert_eq!(life::range_distance((100, 100), (130, 125)), 1525);
    assert_eq!(life::range_distance((100, 100), (141, 100)), 1681);
    let mut t = act1_tables();
    t.pet_flags = HirelingTables::RANGE;
    // 1525 ≤ 1600: kept.
    let (mut w, p, _, m) = world();
    w.unit_mut(m).pos = (100, 100);
    w.unit_mut(p).pos = (130, 125);
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::follow(&mut w, &t, &mut st, p);
    assert!(w.log.is_empty() && w.sent.is_empty());
    assert_eq!(st.list(p).unwrap().nodes.len(), 1);
    // 1681 > 1600: removed with kill (unlink 0x7A, dismiss, 0x7A again).
    w.unit_mut(p).pos = (141, 100);
    life::follow(&mut w, &t, &mut st, p);
    assert!(st.list(p).unwrap().nodes.is_empty());
    assert_eq!(w.log, vec!["dismiss 10".to_string()]);
    assert_eq!(
        w.sent_to(p),
        vec![remove_msg(M_GUID), remove_msg(M_GUID), remove_msg(M_GUID)]
    );
    // A dead node is not tested.
    let (mut w, p, _, m) = world();
    w.unit_mut(p).pos = (1000, 1000);
    let mut st = with_nodes(p, vec![node(M_GUID, true)]);
    life::follow(&mut w, &t, &mut st, p);
    assert!(w.log.is_empty());
    assert!(w.units.contains_key(&m));
}

#[test]
fn join_follow_only_with_a_living_hireling() {
    let t = act1_tables();
    let (mut w, p, _, _) = world();
    let mut st = with_nodes(p, vec![node(M_GUID, true)]);
    life::join_follow(&mut w, &t, &mut st, p);
    assert!(w.log.is_empty());
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::join_follow(&mut w, &t, &mut st, p);
    assert_eq!(w.log, vec!["warp 10 1".to_string()]);
}

#[test]
fn version_47_restore_is_a_class_0_new_hire() {
    let t = act1_tables();
    let saved = SavedHireling {
        dead: true,
        seed: 0x1234,
        name_index: 105,
        id: 1,
        experience: 26460,
    };
    let plan = life::restore_plan(&t, true, 0, &saved, 0, Loader::OldV47).unwrap();
    assert_eq!(
        plan,
        RestorePlan {
            name: 105,
            mode: MODE_NEUTRAL,
            class: 0,
            id: NEW_HIRE,
            seller: 150,
            loader: Loader::OldV47,
        }
    );
    // Seed or name word 0: no hireling.
    for s in [
        SavedHireling { seed: 0, ..saved },
        SavedHireling {
            name_index: 0,
            ..saved
        },
    ] {
        assert_eq!(
            life::restore_plan(&t, true, 0, &s, 0, Loader::OldV47),
            Err(RestoreSkip::NoHireling)
        );
    }
    // No candidate of (expansion, act, difficulty): fatal 0x744.
    assert_eq!(
        life::restore_plan(&t, true, 1, &saved, 0, Loader::OldV47),
        Err(RestoreSkip::Fatal(0x744))
    );
    // The init takes the new-hire branch: the node keeps the offer's
    // values (no saved node write).
    let (mut w, p, _, m) = world();
    let mut st = HirelingState::default();
    assert!(life::restore(&mut w, &t, &mut st, p, m, &plan, &saved).unwrap());
    let n = st.list(p).unwrap().nodes[0];
    assert_eq!((n.seed, n.name, n.dead), (0x1234, 105, false));
    let offer = t
        .rows
        .offer(true, 10, 0x1234, 0, 0)
        .expect("an offer for act 1");
    assert_eq!(n.id, offer.id);
    assert_eq!(w.base(m, stat::EXPERIENCE), offer.experience);
}

#[test]
fn living_restore_joins_team() {
    let (mut w, p, _, m) = world();
    let t = act1_tables();
    let mut st = HirelingState::default();
    let saved = SavedHireling {
        dead: false,
        seed: 0x5555,
        name_index: 5,
        id: 1,
        experience: 26460,
    };
    let plan = life::restore_plan(&t, true, 0, &saved, 0, Loader::Current).unwrap();
    assert!(life::restore(&mut w, &t, &mut st, p, m, &plan, &saved).unwrap());
    let team = format!("team {} {}", m.0, p.0);
    assert!(w.log.contains(&team));
}

#[test]
fn restore_tail_order_per_loader() {
    let (mut w, p, _, m) = world();
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::restore_tail(&mut w, &mut st, p, m, true, Loader::Current);
    assert_eq!(
        w.log,
        vec![
            "mode 10 12".to_string(),
            "cancel 10 2 0".to_string(),
            "cancel 10 3 0".to_string(),
            "inventory 10".to_string(),
        ]
    );
    let (mut w, p, _, m) = world();
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::restore_tail(&mut w, &mut st, p, m, true, Loader::Old);
    assert_eq!(w.log[0], "inventory 10");
    assert_eq!(w.log.len(), 4);
    // Alive: the inventory step only.
    let (mut w, p, _, m) = world();
    let mut st = with_nodes(p, vec![node(M_GUID, false)]);
    life::restore_tail(&mut w, &mut st, p, m, false, Loader::Current);
    assert_eq!(w.log, vec!["inventory 10".to_string()]);
    assert!(!st.list(p).unwrap().nodes[0].dead);
}
