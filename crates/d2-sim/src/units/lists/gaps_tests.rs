//! Gap tests: list rules not yet claimed by other tests.

#[allow(unused_imports)]
use super::*;

use crate::game::{Game, GameError};
use crate::tick::timer::{TimerId, TimerRun};
use crate::tick::{tick, EventDispatch, TickHooks};

fn guid(g: &Game, u: UnitId) -> u32 {
    g.lists.unit(u).unwrap().guid
}

fn one_room() -> (Game, RoomId) {
    let mut g = Game::new();
    g.lists.ensure_act(0).unwrap();
    let r = g.lists.create_room(0).unwrap();
    g.lists.activate_room(r).unwrap();
    (g, r)
}

#[test]
fn units_identified_by_type_and_guid() {
    let numbers: Vec<usize> = UnitType::ALL.iter().map(|t| t.index()).collect();
    assert_eq!(numbers, [0, 1, 2, 3, 4, 5]);
    assert_eq!(
        UnitType::ALL,
        [
            UnitType::Player,
            UnitType::Monster,
            UnitType::Object,
            UnitType::Missile,
            UnitType::Item,
            UnitType::Tile
        ]
    );
    // The same GUID names a different unit in each type.
    let mut l = UnitLists::new();
    let ids: Vec<UnitId> = UnitType::ALL
        .iter()
        .map(|&t| l.add_unit(t, 1, None, false).unwrap())
        .collect();
    for (&t, &u) in UnitType::ALL.iter().zip(&ids) {
        assert_eq!(l.find_unit(t, 1), Some(u), "{t:?}");
        assert_eq!(l.unit(u).unwrap().ty, t);
    }
}

#[test]
fn guids_never_reused_before_the_wrap() {
    let (mut g, r) = one_room();
    let a = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    let b = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    g.remove_unit(b).unwrap();
    g.remove_unit(a).unwrap();
    assert_eq!(g.lists.guids.get(UnitType::Monster), 2);
    // The slot is reused, the GUID is not.
    let c = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    assert_eq!(guid(&g, c), 3);
    assert_eq!(g.lists.guids.get(UnitType::Monster), 3);
}

#[test]
fn guid_wrap_collides_with_a_live_unit() {
    let (mut g, r) = one_room();
    let first = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    assert_eq!(guid(&g, first), 1);
    g.lists.guids.set(UnitType::Monster, 0xFFFF_FFFE);
    // The counter wraps to 1, still live: the insert's fatal error.
    assert_eq!(
        g.spawn_unit(UnitType::Monster, Some(r), false),
        Err(GameError::List(ListError::DuplicateGuid {
            ty: UnitType::Monster,
            guid: 1
        }))
    );
    assert_eq!(g.lists.room_units(r), [first]);
}

#[test]
fn hash_list_table() {
    // List index per type (missile and item swap), tiles one list.
    let lists: Vec<Option<usize>> = UnitType::ALL.iter().map(|t| t.hash_list()).collect();
    assert_eq!(lists, [Some(0), Some(1), Some(2), Some(4), Some(3), None]);
    // 128 buckets, GUID & 0x7F.
    assert_eq!(HASH_BUCKETS, 128);
    let mut l = UnitLists::new();
    let u = l.add_unit(UnitType::Item, 0x285, None, false).unwrap();
    assert_eq!(l.hash_bucket(UnitType::Item, 0x05), [u]);
    assert!(l.hash_bucket(UnitType::Missile, 0x05).is_empty());
    // Tiles: one list whatever the GUID.
    let t1 = l.add_unit(UnitType::Tile, 1, None, false).unwrap();
    let t2 = l.add_unit(UnitType::Tile, 0x80, None, false).unwrap();
    assert_eq!(l.hash_bucket(UnitType::Tile, 0), [t2, t1]);
}

#[test]
fn removal_unlinks_lists_and_cancels_timers() {
    let (mut g, r) = one_room();
    let a = g.spawn_unit(UnitType::Missile, Some(r), false).unwrap();
    let b = g.spawn_unit(UnitType::Missile, Some(r), false).unwrap();
    g.schedule_event(a, 0, -1, None, 0, 0).unwrap();
    g.schedule_event(a, 3, 5, None, 0, 0).unwrap();
    let tb = g.schedule_event(b, 3, 5, None, 0, 0).unwrap().unwrap();
    g.remove_unit(a).unwrap();
    assert_eq!(g.lists.room_units(r), [b]);
    assert_eq!(g.lists.update_queue(r), [b]);
    assert_eq!(g.lists.units_of_type(UnitType::Missile), [b]);
    assert_eq!(g.lists.find_unit(UnitType::Missile, 1), None);
    assert!(g.timers.unit_timers(a).is_empty());
    use crate::tick::timer::TimerClass;
    assert!(g.timers.every_tick(TimerClass::Missile).is_empty());
    assert_eq!(g.timers.bucket(TimerClass::Missile, 5), [tb]);
}

#[test]
fn five_acts() {
    let mut l = UnitLists::new();
    for act in 0..5 {
        l.ensure_act(act).unwrap();
        assert!(l.act(act).is_some());
    }
    assert_eq!(ACTS, 5);
    assert_eq!(l.ensure_act(5), Err(ListError::UnknownAct(5)));
    // One room list per act, walked from its head.
    let r0 = l.create_room(0).unwrap();
    let r4 = l.create_room(4).unwrap();
    l.activate_room(r0).unwrap();
    l.activate_room(r4).unwrap();
    assert_eq!((l.room_first(0), l.room_first(4)), (Some(r0), Some(r4)));
    assert_eq!((l.room_next(r0), l.room_next(r4)), (None, None));
}

#[test]
fn room_unit_list_links() {
    let (mut g, r) = one_room();
    let r2 = g.lists.create_room(0).unwrap();
    g.lists.activate_room(r2).unwrap();
    let a = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    let b = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    let c = g.spawn_unit(UnitType::Monster, Some(r2), false).unwrap();
    // Head per room, one next link per unit.
    assert_eq!(g.lists.room_unit_first(r), Some(b));
    assert_eq!(g.lists.room_unit_next(b), Some(a));
    assert_eq!(g.lists.room_unit_next(a), None);
    assert_eq!(g.lists.room_unit_first(r2), Some(c));
    // A unit is in one room list at a time.
    assert_eq!(g.lists.room_insert(a, r2), Err(ListError::AlreadyInRoom(a)));
}

#[test]
fn update_queue_links_and_flag() {
    let (mut g, r) = one_room();
    let a = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    let b = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    assert_eq!(g.lists.update_first(r), Some(b));
    assert_eq!(g.lists.update_next(b), Some(a));
    assert_eq!(g.lists.update_next(a), None);
    // Membership flag follows the queue.
    assert!(g.lists.unit(a).unwrap().is_queued());
    g.lists.unqueue_update(a).unwrap();
    assert!(!g.lists.unit(a).unwrap().is_queued());
    assert_eq!(g.lists.update_queue(r), [b]);
    // Room list membership is separate.
    assert_eq!(g.lists.room_units(r), [b, a]);
}

/// Client pass / environment probe: logs each client visited and removes
/// one of them inside its body.
#[derive(Default)]
struct ClientProbe {
    log: Vec<String>,
    remove_in_update: Option<ClientId>,
    remove_in_env: Option<ClientId>,
    /// Unit removed when its update message is sent.
    remove_unit: Option<UnitId>,
    /// Step 9 deactivates every room (units compressed and removed).
    deactivate: bool,
    /// While a timer of the unit runs, cancel this timer.
    cancel_on_run: Option<(UnitId, TimerId)>,
}

impl EventDispatch for ClientProbe {
    fn run_event(&mut self, g: &mut Game, run: &TimerRun) {
        self.log.push(format!("run g{}", run.owner.guid));
        if let Some((u, t)) = self.cancel_on_run {
            if run.owner.unit == u {
                g.timers.cancel(t);
            }
        }
    }
}

impl TickHooks for ClientProbe {
    fn advance_environment(&mut self, _: &mut Game, act: u8) -> bool {
        act == 0
    }
    fn environment_changed(&mut self, g: &mut Game, _: u8, c: ClientId) {
        self.log.push(format!("env c{}", c.0));
        if self.remove_in_env == Some(c) {
            g.lists.remove_client(c).unwrap();
        }
    }
    fn send_removed_units(&mut self, g: &mut Game, c: ClientId) {
        self.log.push(format!("update c{}", c.0));
        if self.remove_in_update == Some(c) {
            g.lists.remove_client(c).unwrap();
        }
    }
    fn send_unit_update(&mut self, g: &mut Game, _: ClientId, u: UnitId) {
        let e = g.lists.unit(u).unwrap();
        self.log.push(format!("send {:?}{}", e.ty, e.guid));
        if self.remove_unit == Some(u) {
            g.remove_unit(u).unwrap();
        }
    }
    fn room_inactivity(&mut self, _: &mut Game, _: RoomId) -> u32 {
        if self.deactivate {
            11
        } else {
            0
        }
    }
    fn act_allows_room_removal(&mut self, _: &mut Game, _: u8, _: RoomId) -> bool {
        true
    }
    fn compress_unit(&mut self, g: &mut Game, u: UnitId) {
        self.log.push(format!("compress g{}", guid(g, u)));
        g.remove_unit(u).unwrap();
    }
}

#[test]
fn client_list_walks_newest_first_with_next_saved() {
    let (mut g, _) = one_room();
    let c1 = g.lists.add_client(None, None, client_state::IN_GAME);
    let c2 = g.lists.add_client(None, None, client_state::IN_GAME);
    let c3 = g.lists.add_client(None, None, client_state::IN_GAME);
    assert_eq!(g.lists.client_first(), Some(c3));
    assert_eq!(g.lists.client_next(c3), Some(c2));
    assert_eq!(g.lists.client_next(c2), Some(c1));
    assert_eq!(g.lists.client_next(c1), None);
    // Environment step removes c3 inside its body, client pass c2: each
    // walk goes on to the next client.
    let mut p = ClientProbe {
        remove_in_env: Some(c3),
        remove_in_update: Some(c2),
        ..ClientProbe::default()
    };
    tick(&mut g, &mut p);
    let (a, b, c) = (c1.0, c2.0, c3.0);
    assert_eq!(
        p.log,
        [
            format!("env c{c}"),
            format!("env c{b}"),
            format!("env c{a}"),
            format!("update c{b}"),
            format!("update c{a}"),
        ]
    );
    assert_eq!(g.lists.clients(), [c1]);
}

#[test]
fn client_update_walks_the_adjacent_array_in_order() {
    let mut g = Game::new();
    g.lists.ensure_act(0).unwrap();
    let rooms: Vec<RoomId> = (0..3).map(|_| g.lists.create_room(0).unwrap()).collect();
    for &r in &rooms {
        g.lists.activate_room(r).unwrap();
    }
    // Act list [r2, ...]; adjacency array of r0 in another order.
    let p = g
        .spawn_unit(UnitType::Player, Some(rooms[0]), true)
        .unwrap();
    for &r in &rooms {
        g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    }
    g.lists.room_mut(rooms[0]).unwrap().adjacent = vec![rooms[0], rooms[2], rooms[1]];
    g.lists
        .add_client(Some(p), Some(rooms[0]), client_state::IN_GAME);
    let mut probe = ClientProbe::default();
    tick(&mut g, &mut probe);
    let sends: Vec<&str> = probe
        .log
        .iter()
        .filter(|s| s.starts_with("send"))
        .map(String::as_str)
        .collect();
    // Monster 1 then player 1 (most recent first): monster 3:
    // monster 2.
    assert_eq!(
        sends,
        [
            "send Monster1",
            "send Player1",
            "send Monster3",
            "send Monster2"
        ]
    );
}

#[test]
fn iteration_while_modifying() {
    // Room update queue in the client update: the body may remove the
    // current unit; the walk goes on (next saved before the body).
    let (mut g, r) = one_room();
    g.lists.room_mut(r).unwrap().adjacent = vec![r];
    let p = g.spawn_unit(UnitType::Player, Some(r), true).unwrap();
    let m1 = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    let _m2 = g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    g.lists.add_client(Some(p), Some(r), client_state::IN_GAME);
    let mut probe = ClientProbe {
        remove_unit: Some(m1),
        ..ClientProbe::default()
    };
    // Queue [m2, m1, p]: m1 removed while its message is sent.
    tick(&mut g, &mut probe);
    let sends: Vec<&str> = probe
        .log
        .iter()
        .filter(|s| s.starts_with("send"))
        .map(String::as_str)
        .collect();
    assert_eq!(sends, ["send Monster2", "send Monster1", "send Player1"]);
    assert!(g.lists.unit(m1).is_none());

    // Head-insert lists: a unit added during the walk lands behind the
    // walker and is not visited.
    let (mut g, r) = one_room();
    g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    let mut seen = Vec::new();
    let mut cur = g.lists.room_unit_first(r);
    while let Some(u) = cur {
        cur = g.lists.room_unit_next(u);
        seen.push(guid(&g, u));
        g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    }
    assert_eq!(seen, [2, 1]);

    // Hash buckets: a unit added during the walk is visited only if its
    // bucket comes later and it sorts after the walker.
    let mut g = Game::new();
    g.lists.add_unit(UnitType::Player, 1, None, true).unwrap();
    let mut seen = Vec::new();
    g.for_each_player(
        |_, _| false,
        |g, u| {
            let gu = guid(g, u);
            seen.push(gu);
            if gu == 1 {
                // Bucket 0 (earlier), bucket 1 ahead of GUID 1 (sorts
                // before it), bucket 2 (later).
                for n in [0x80, 129, 2] {
                    g.lists.add_unit(UnitType::Player, n, None, true).unwrap();
                }
            }
        },
    );
    assert_eq!(seen, [1, 2]);

    // Step 9: the act room list and each room's unit list may lose the
    // current entry (next saved before the body).
    let (mut g, r1) = one_room();
    let r2 = g.lists.create_room(0).unwrap();
    g.lists.activate_room(r2).unwrap();
    for r in [r1, r1, r2, r2] {
        g.spawn_unit(UnitType::Monster, Some(r), false).unwrap();
    }
    g.frame = 11;
    let mut probe = ClientProbe {
        deactivate: true,
        ..ClientProbe::default()
    };
    tick(&mut g, &mut probe);
    let compressed: Vec<&str> = probe
        .log
        .iter()
        .filter(|s| s.starts_with("compress"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        compressed,
        ["compress g4", "compress g3", "compress g2", "compress g1"]
    );
    assert!(g.lists.active_rooms(0).is_empty());

    // Timer lists: the cursor; a timer cancelled before the cursor
    // reaches it does not run.
    let (mut g, r) = one_room();
    let a = g.spawn_unit(UnitType::Missile, Some(r), false).unwrap();
    let b = g.spawn_unit(UnitType::Missile, Some(r), false).unwrap();
    let ta = g.schedule_event(a, 0, -1, None, 0, 0).unwrap().unwrap();
    g.schedule_event(b, 0, -1, None, 0, 0).unwrap();
    let mut probe = ClientProbe {
        cancel_on_run: Some((b, ta)),
        ..ClientProbe::default()
    };
    tick(&mut g, &mut probe);
    let runs: Vec<&str> = probe
        .log
        .iter()
        .filter(|s| s.starts_with("run"))
        .map(String::as_str)
        .collect();
    assert_eq!(runs, ["run g2"]);
}
