//! The owner's death on the wired host: the player mode-17 start queues
//! the player (`ActionHooks::owner_deaths`) and the host runs the
//! owner-death rule on its hireling lists after the tick
//! (`d2_sim::world::hirelings::life::player_death`). The queued
//! hireling calls (`ActionHooks::hireling_calls`): the save restore, the
//! join follow and the act change, run on the host's lists after the
//! tick. Tests of `WiredWorld` (see `trade_quests` for its fixture).

use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::UnitId;
use d2_sim::units::UnitType;
use d2_sim::wiring::action::HirelingCall;
use d2_sim::world::hirelings::life::{Loader, SavedHireling};
use d2_sim::world::hirelings::{flags, HirelingRow, HirelingRows, HirelingTables, PetNode};
use d2_sim::world::npc::class;

use super::trade_quests::Fx;

#[test]
fn a_queued_owner_death_kills_the_hireling_after_the_tick() {
    let mut fx = Fx::new(|_| {});
    let req = AllocRequest {
        ty: UnitType::Monster,
        class: 0,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let s = &mut fx.h.game;
    let merc = s
        .events
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    let gm = fx.guid(merc);
    let p = fx.player;
    let w = fx.world();
    w.state.hireling_tables = Some(HirelingTables {
        rows: Default::default(),
        exp_ratios: Default::default(),
        max_level: 99,
        pet_flags: HirelingTables::WARP,
        pet_basemax: 1,
    });
    w.state.hirelings.list_mut(p).nodes = vec![PetNode {
        guid: gm,
        name: 0x0D68,
        ..PetNode::default()
    }];
    // The mode-17 start queues the owner (on from the first frame);
    // here staged.
    fx.h.connect(0);
    fx.h.clock.0 += 40;
    assert!(fx.h.frame().unwrap().ticked);
    let q = fx.h.game.events.hooks().owner_deaths.as_mut().unwrap();
    assert!(q.is_empty());
    q.push(p);
    fx.h.receive(0);
    fx.h.clock.0 += 40;
    assert!(fx.h.frame().unwrap().ticked);
    let node = fx.world().state.hirelings.list(p).unwrap().nodes[0];
    assert!(node.dead);
    // 0x7A remove (GUID only, @9), then 0x9B (name 0x0D68, cost at level
    // 0 = 0) to the owner; the unit stays.
    let mut remove = vec![0x7A, 0, 0, 0, 0, 0, 0, 0, 0];
    remove.extend_from_slice(&gm.to_le_bytes());
    let to_p: Vec<Vec<u8>> =
        fx.h.receive(0)
            .into_iter()
            .filter(|m| m[0] == 0x9B || m[0] == 0x7A)
            .collect();
    assert_eq!(to_p, vec![remove, vec![0x9B, 0x68, 0x0D, 0, 0, 0, 0]]);
    assert!(fx.h.game.events.sys.units.get(merc).is_some());
    assert_eq!(fx.h.game.events.hooks().owner_deaths, Some(vec![]));
}

/// Hireling tables with one classic row (`Id` 1, class 0, act 1,
/// Normal, sold by Kashya) whose name range starts at `name`.
fn tables(name: u16) -> HirelingTables {
    let row = HirelingRow {
        id: 1,
        class: 0,
        act: 1,
        difficulty: 1,
        seller: u32::from(class::KASHYA),
        level: 1,
        exp_lvl: 100,
        name_first: name,
        name_last: name + 10,
        ..HirelingRow::default()
    };
    HirelingTables {
        rows: HirelingRows::new(vec![row]),
        exp_ratios: Default::default(),
        max_level: 99,
        pet_flags: HirelingTables::WARP,
        pet_basemax: 1,
    }
}

/// Runs one frame with `calls` queued on the action hooks (on from the
/// first frame) and returns the owner's 0x7A / 0x9B messages.
fn run_calls(fx: &mut Fx, calls: Vec<HirelingCall>) -> Vec<Vec<u8>> {
    fx.h.connect(0);
    fx.h.clock.0 += 40;
    assert!(fx.h.frame().unwrap().ticked);
    fx.h.receive(0);
    let q = fx.h.game.events.hooks().hireling_calls.as_mut().unwrap();
    assert!(q.is_empty());
    q.extend(calls);
    fx.h.clock.0 += 40;
    assert!(fx.h.frame().unwrap().ticked);
    assert_eq!(fx.h.game.events.hooks().hireling_calls, Some(vec![]));
    fx.h.receive(0)
        .into_iter()
        .filter(|m| m[0] == 0x9B || m[0] == 0x7A)
        .collect()
}

/// The monsters of class 0 other than the fixture's NPCs.
fn mercs(fx: &Fx) -> Vec<UnitId> {
    let s = &fx.h.game;
    s.game
        .lists
        .units_of_type(UnitType::Monster)
        .into_iter()
        .filter(|&u| s.events.sys.units.get(u).is_some_and(|r| r.class == 0))
        .collect()
}

/// A queued restore of a living hireling (≥ 0x5C loader), then the join
/// follow: the hire slot of the name is marked hired unless offered, a
/// class-0 unit is allocated with no room in mode 1, its node holds the
/// saved seed, name and `Id`; the join follow warps it to the player
/// (flags 2 bit 0x10000).
#[test]
fn a_queued_restore_then_join_follow_restores_and_warps_the_hireling() {
    let mut fx = Fx::new(|_| {});
    let p = fx.player;
    let slot0 = fx
        .world()
        .npc
        .record(class::KASHYA)
        .unwrap()
        .hire
        .as_ref()
        .unwrap()
        .slots[0];
    fx.world().state.hireling_tables = Some(tables(slot0.name));
    let saved = SavedHireling {
        dead: false,
        seed: 5,
        name_index: 0,
        id: 1,
        experience: 0,
    };
    let sent = run_calls(
        &mut fx,
        vec![
            HirelingCall::Restore {
                player: p,
                saved,
                loader: Loader::Current,
            },
            HirelingCall::JoinFollow(p),
        ],
    );
    assert_eq!(sent, Vec::<Vec<u8>>::new());
    let m = mercs(&fx);
    assert_eq!(m.len(), 1);
    let merc = m[0];
    let gm = fx.guid(merc);
    assert!(fx.h.game.game.lists.unit(merc).unwrap().room().is_none());
    assert_eq!(fx.h.game.events.sys.units.get(merc).unwrap().mode, 1);
    let node = fx.world().state.hirelings.list(p).unwrap().nodes[0];
    assert_eq!(
        (node.guid, node.seed, node.name, node.id),
        (gm, 5, slot0.name, 1)
    );
    assert!(!node.dead);
    let after = fx
        .world()
        .npc
        .record(class::KASHYA)
        .unwrap()
        .hire
        .as_ref()
        .unwrap()
        .slots[0];
    assert_eq!(after.hired, !slot0.offered);
    assert!(fx
        .world()
        .rest
        .log
        .contains(&format!("warp {} {}", merc.0, p.0)));
    assert_ne!(
        fx.h.game.events.sys.units.get(merc).unwrap().flags2 & flags::WARP2,
        0
    );
}

/// A queued restore of a dead hireling: node dead, 0x9B then 0x7A to the
/// owner, flags |= 0x10000, mode 12; the join follow leaves it.
#[test]
fn a_queued_restore_of_a_dead_hireling_marks_it_dead() {
    let mut fx = Fx::new(|_| {});
    let p = fx.player;
    fx.world().state.hireling_tables = Some(tables(0x0D00));
    let saved = SavedHireling {
        dead: true,
        seed: 5,
        name_index: 2,
        id: 1,
        experience: 0,
    };
    let sent = run_calls(
        &mut fx,
        vec![
            HirelingCall::Restore {
                player: p,
                saved,
                loader: Loader::Current,
            },
            HirelingCall::JoinFollow(p),
        ],
    );
    let merc = mercs(&fx)[0];
    let node = fx.world().state.hirelings.list(p).unwrap().nodes[0];
    assert!(node.dead);
    assert_eq!(node.name, 0x0D02);
    assert_eq!(
        sent.iter().map(|m| m[0]).collect::<Vec<_>>(),
        vec![0x9B, 0x7A]
    );
    assert_ne!(
        fx.h.game.events.sys.units.get(merc).unwrap().flags & flags::DEAD,
        0
    );
    let log = &fx.world().rest.log;
    assert!(log.contains(&format!("mode {} 12", merc.0)));
    assert!(!log.iter().any(|l| l.starts_with("warp")));
}

/// A living hireling and a queued act change: classic → the owner-death
/// rule (node dead, 0x7A, 0x9B), the follow finds no living node; expansion
/// → the node stays living and the follow warps it.
#[test]
fn a_queued_act_change_kills_the_classic_hireling_and_follows_otherwise() {
    for expansion in [false, true] {
        let mut fx = Fx::new(|_| {});
        fx.h.game.events.sys.data.expansion = expansion;
        let req = AllocRequest {
            ty: UnitType::Monster,
            class: 0,
            room: None,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: false,
        };
        let s = &mut fx.h.game;
        let merc = s
            .events
            .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
            .unwrap();
        let gm = fx.guid(merc);
        let p = fx.player;
        fx.world().state.hireling_tables = Some(tables(0x0D00));
        fx.world().state.hirelings.list_mut(p).nodes = vec![PetNode {
            guid: gm,
            name: 0x0D00,
            ..PetNode::default()
        }];
        let sent = run_calls(&mut fx, vec![HirelingCall::ActChange(p)]);
        let node = fx.world().state.hirelings.list(p).unwrap().nodes[0];
        let warped = fx
            .world()
            .rest
            .log
            .contains(&format!("warp {} {}", merc.0, p.0));
        if expansion {
            assert!(!node.dead);
            assert!(sent.is_empty());
            assert!(warped);
        } else {
            assert!(node.dead);
            assert_eq!(
                sent.iter().map(|m| m[0]).collect::<Vec<_>>(),
                vec![0x7A, 0x9B]
            );
            assert!(!warped);
        }
    }
}

/// The save load's hireling step on the action wiring
/// (`ActionCharacter::restore_hireling`): a present block is queued as a
/// restore with the ≥ 0x5C loader (the dead bit from the saved flags);
/// an absent one queues nothing; with the queue off the step is
/// reported unapplied.
#[test]
fn the_load_queues_the_saved_hireling_for_the_host() {
    use crate::adapters::character::{ActionCharacter, CharacterWorld};
    use d2_formats::d2s::Hireling;
    let mut fx = Fx::new(|_| {});
    let p = fx.player;
    let block = Hireling {
        flags: Hireling::DEAD,
        seed: 7,
        name_index: 1,
        id: 3,
        experience: 9,
        rest: [0; 16],
    };
    let absent = Hireling {
        flags: 0,
        seed: 0,
        name_index: 0,
        id: 0,
        experience: 0,
        rest: [0; 16],
    };
    let s = &mut fx.h.game;
    s.events.hooks().hireling_calls = None;
    let off = s.events.with(&mut s.game, |_, v| {
        ActionCharacter { v, player: p }.restore_hireling(&block)
    });
    assert!(off.is_err());
    s.events.hooks().hireling_calls = Some(Vec::new());
    let r = s.events.with(&mut s.game, |_, v| {
        let mut cw = ActionCharacter { v, player: p };
        (cw.restore_hireling(&absent), cw.restore_hireling(&block))
    });
    assert_eq!(r, (Ok(()), Ok(())));
    assert_eq!(
        s.events.hooks().hireling_calls,
        Some(vec![HirelingCall::Restore {
            player: p,
            saved: SavedHireling {
                dead: true,
                seed: 7,
                name_index: 1,
                id: 3,
                experience: 9,
            },
            loader: Loader::Current,
        }])
    );
}

/// An NPC's act change (Warriv's "Go East", `NpcWorld::act_change`) is
/// queued on the action hooks for the host that runs it; a call in the
/// queue is drained by the next frame (no path provider here: dropped).
#[test]
fn an_npc_act_change_is_queued_for_the_host() {
    use d2_sim::world::npc::NpcWorld;
    let mut fx = Fx::new(|_| {});
    let p = fx.player;
    let s = &mut fx.h.game;
    s.world.desk(&mut s.game, &mut s.events, |desk, _, _| {
        NpcWorld::act_change(desk, p, 40, 0);
    });
    assert_eq!(fx.h.game.events.hooks().act_changes, vec![(p, 40, 0)]);
    fx.h.connect(0);
    fx.h.clock.0 += 40;
    assert!(fx.h.frame().unwrap().ticked);
    assert!(fx.h.game.events.hooks().act_changes.is_empty());
}
