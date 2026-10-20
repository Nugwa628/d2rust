//! Tests for DRLG room activation ↔ the act room lists: streamed rooms live in
//! [`crate::units::UnitLists`]; the tick counts and removes inactive
//! rooms through the DRLG; a client's room change keeps rooms active.

use super::*;
use crate::units::lists::client_state;

fn act_rooms(fx: &Fx) -> Vec<RoomId> {
    let mut v = Vec::new();
    let mut cur = fx.game.lists.room_first(0);
    while let Some(r) = cur {
        v.push(r);
        cur = fx.game.lists.room_next(r);
    }
    v
}

#[test]
fn streamed_rooms_are_active_in_the_act_room_list() {
    let mut fx = Fx::new();
    let (a, b) = (fx.a, fx.b);
    // Prepended at activation (B streamed last).
    assert_eq!(act_rooms(&fx), [b, a]);
    for r in [a, b] {
        let e = fx.game.lists.room(r).unwrap();
        // Each adjacency array holds the room itself and its active
        // neighbour.
        let mut adj = e.adjacent.clone();
        adj.sort();
        assert_eq!(adj, [a, b]);
    }
    let game = &fx.game;
    let d = &fx.sim.sys.hooks.drlg;
    assert_eq!(d.level_id(game, a), Some(LEVEL));
    assert_eq!(d.subtiles(game, b), Some(TileRect::new(40, 0, 40, 40)));
    assert_eq!(d.find_room(game, a, 45, 3), Some(b));
    assert_eq!(d.find_room(game, a, 85, 3), None);
    assert!(!d.in_town(game, a));
    let _ = fx.sim.hooks();
    fx.assert_clean();
}

#[test]
fn inactive_rooms_are_removed_by_tick_step_9() {
    // No clients → +1 per pass (every 12 frames); removed when > 10, i.e.
    // on the 11th pass (frame 132): the act list unlink, then the DRLG's
    // rest.
    let mut fx = Fx::new();
    let (a, b) = (fx.a, fx.b);
    while fx.game.frame < 131 {
        fx.tick();
    }
    assert_eq!(act_rooms(&fx).len(), 2);
    fx.tick();
    assert_eq!(fx.game.frame, 132);
    assert!(act_rooms(&fx).is_empty());
    for r in [a, b] {
        assert!(fx.game.lists.room(r).is_none(), "record freed");
        let game = &fx.game;
        assert!(fx.sim.sys.hooks.drlg.drlg_room(game, r).is_none());
    }
    fx.assert_clean();
}

#[test]
fn client_room_change_keeps_its_rooms_active() {
    // The player's room differs from the client's → the room switch: the
    // client joins every room of A's adjacency array; their counters stay
    // 0.
    let mut fx = Fx::new();
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.tick();
    assert_eq!(fx.game.lists.client(c).unwrap().room, Some(a));
    while fx.game.frame < 200 {
        fx.tick();
    }
    assert_eq!(act_rooms(&fx).len(), 2);
    let game = &fx.game;
    let (d, r) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    let ar = d.active_room(r).unwrap();
    assert_eq!(ar.clients, [c]);
    assert_eq!(ar.inactivity, 0);
    fx.assert_clean();
}

/// The S→C 0x07 the room switch sends for DRLG room `r`.
fn reveal_of(d: &crate::drlg::Drlg, r: crate::drlg::DrlgRoomId) -> Vec<u8> {
    let room = d.room(r);
    crate::wiring::path::place::map_reveal(
        room.rect.x as u16,
        room.rect.y as u16,
        d.level(room.level).id as u8,
    )
    .to_vec()
}

#[test]
fn room_switch_reveals_each_joined_room_in_adjacency_order() {
    // The room switch sends, for each room of the new adjacency
    // array missing from the old: S→C 0x07 (tile x, tile y, level id) to
    // the client's player. A, B, C in a row: A's array is {A, B}, C's is
    // {B, C}; the first switch (no old room) reveals A's whole array, the
    // switch A → C only C.
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (LEVEL, TileRect::new(16, 0, 8, 8)),
    ]);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    fx.game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let reveals = |fx: &Fx| -> Vec<Vec<u8>> {
        fx.sim
            .sys
            .hooks
            .x
            .sent
            .iter()
            .filter(|(u, m)| *u == p && m[0] == 0x07)
            .map(|(_, m)| m.clone())
            .collect()
    };
    let game = &fx.game;
    let (d, ra) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    let want: Vec<Vec<u8>> = d
        .active_room(ra)
        .unwrap()
        .adjacency
        .iter()
        .map(|&r| reveal_of(d, r))
        .collect();
    assert_eq!(want.len(), 2);
    assert_eq!(reveals(&fx), want);
    // The third room is active now (B's array streamed it); move the
    // player there.
    let c = act_rooms(&fx)
        .into_iter()
        .find(|&r| {
            fx.sim.sys.hooks.drlg.subtiles(&fx.game, r) == Some(TileRect::new(80, 0, 40, 40))
        })
        .expect("room C active");
    fx.game.lists.change_room(p, c).unwrap();
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let game = &fx.game;
    let (d, rc) = fx.sim.sys.hooks.drlg.drlg_room(game, c).unwrap();
    assert_eq!(reveals(&fx), vec![reveal_of(d, rc)]);
    assert_eq!(reveal_of(d, rc), [0x07, 16, 0, 0, 0, LEVEL as u8]);
    fx.assert_clean();
}

#[test]
fn room_switch_sends_add_and_leave_messages_and_the_join_completes() {
    // A, B, C in a row (A's array {A, B}, C's {B, C}). An object and a
    // warp tile stand in A. A joining client (state 3) whose player is in
    // A: tick 1's per-client update switches to A (0x07 A, the object's
    // 0x51, the tile's 0x09, 0x07 B), the rooms are populated by the
    // tick's population step, so the room is ready and 0x04 follows, the
    // client in game, then the join sequence (0x5B, 0x65, the join 0x5A).
    // Then the player moves to C: 0x07 C, then A's leave: 0x0A for each
    // unit of A, 0x08 A.
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (LEVEL, TileRect::new(16, 0, 8, 8)),
    ]);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let o = fx.spawn(UnitType::Object, 7, a, 12, 14);
    let t = fx.spawn(UnitType::Tile, 3, a, 20, 21);
    let c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::JOINING);
    let mut name = [0u8; 16];
    name[..4].copy_from_slice(b"Jade");
    fx.sim.sys.hooks.session.names.insert(p, name);
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let sent = |fx: &mut Fx| -> Vec<Vec<u8>> {
        let v = fx
            .sim
            .sys
            .hooks
            .x
            .sent
            .iter()
            .filter(|(u, _)| *u == p)
            .map(|(_, m)| m.clone())
            .collect();
        fx.sim.sys.hooks.x.sent.clear();
        v
    };
    let guid = |fx: &Fx, u: UnitId| fx.game.lists.unit(u).unwrap().guid;
    let (go, gt) = (guid(&fx, o), guid(&fx, t));
    let game = &fx.game;
    let (d, ra) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    let adj = d.active_room(ra).unwrap().adjacency.clone();
    assert_eq!(adj.len(), 2);
    let (rev_a, rev_b) = (reveal_of(d, adj[0]), reveal_of(d, adj[1]));
    assert_eq!(adj[0], ra, "the room itself first in its array");
    let hide_a = {
        let mut m = rev_a.clone();
        m[0] = 0x08;
        m
    };
    let unit_list: Vec<UnitId> = fx.game.lists.room_units(a);
    let mut adds: Vec<Vec<u8>> = Vec::new();
    for &u in &unit_list {
        if u == o {
            adds.push(crate::units::messages::assign_object(go, 7, 12, 14, 1, 0).to_vec());
        } else if u == t {
            adds.push(crate::units::messages::assign_warp(5, gt, 3, 20, 21).to_vec());
        }
    }
    let mut want = vec![rev_a.clone()];
    want.extend(adds);
    want.push(rev_b);
    want.push(vec![0x04]);
    // q-fix-flow-server: after state 4 the inventory refresh's 0x48, then
    // the join sequence 0x5B, 0x65, 0x8D, the join 0x5A.
    let gp = guid(&fx, p);
    let level = fx.sim.sys.stats.unit_total(p, 12, 0) as u16;
    use crate::units::messages as m;
    want.push(crate::items::moves::layouts::relator2(0, 0, gp));
    want.push(m::player_joined(gp, 0, &name, level, m::NO_PARTY));
    want.push(m::player_kill_count(gp, 0).to_vec());
    want.push(m::assign_player_to_party(gp, m::NO_PARTY).to_vec());
    want.push(m::player_event(2, &name).to_vec());
    assert_eq!(sent(&mut fx), want);
    assert_eq!(
        fx.game.lists.client(c).unwrap().state,
        client_state::IN_GAME
    );
    // A → C.
    let rc_id = act_rooms(&fx)
        .into_iter()
        .find(|&r| {
            fx.sim.sys.hooks.drlg.subtiles(&fx.game, r) == Some(TileRect::new(80, 0, 40, 40))
        })
        .expect("room C active");
    fx.game.lists.change_room(p, rc_id).unwrap();
    fx.tick();
    let game = &fx.game;
    let (d, rc) = fx.sim.sys.hooks.drlg.drlg_room(game, rc_id).unwrap();
    let mut want = vec![reveal_of(d, rc)];
    for u in fx.game.lists.room_units(a) {
        let e = fx.game.lists.unit(u).unwrap();
        want.push(crate::units::messages::remove_unit(e.ty as u8, e.guid).to_vec());
    }
    want.push(hide_a);
    assert_eq!(sent(&mut fx), want);
    assert_eq!(fx.game.lists.room_units(a).len(), 2);
    fx.assert_clean();
}

/// The lent quest host of [`quest_event_3_runs_in_the_level_change`]:
/// records each quest event 3 with the number of messages sent before it.
struct LevelLog(Calls);

/// (player, from, to, messages sent before the call).
type Calls = std::rc::Rc<std::cell::RefCell<Vec<(UnitId, u32, u32, usize)>>>;

impl crate::wiring::action::QuestObjectHost<TestPending> for LevelLog {
    fn run(
        &mut self,
        _: &mut Game,
        _: &mut View<'_, TestPending>,
        _: crate::wiring::action::QuestObjectCall,
    ) -> Option<crate::wiring::action::ObjectRoute> {
        None
    }
    fn changed_level(
        &mut self,
        _: &mut Game,
        v: &mut View<'_, TestPending>,
        player: UnitId,
        from: u32,
        to: u32,
    ) {
        let n = v.h.x.sent.len();
        self.0.borrow_mut().push((player, from, to, n));
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

// The per-client update's level change: the
// player's room in another level than the client's room → quest event 3
// CHANGEDLEVEL (from the client room's level, to the player room's) on
// the lent quest control, before the room switch's messages; a room
// change within one level runs none.
#[test]
fn quest_event_3_runs_in_the_level_change() {
    let mut fx = Fx::with_rooms(&[
        (1, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (LEVEL, TileRect::new(16, 0, 8, 8)),
    ]);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let _c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::JOINING);
    fx.tick();
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    fx.sim.sys.hooks.quest_host = Some(Box::new(LevelLog(log.clone())));
    let room_at = |fx: &Fx, x: i32| {
        act_rooms(fx)
            .into_iter()
            .find(|&r| {
                fx.sim.sys.hooks.drlg.subtiles(&fx.game, r) == Some(TileRect::new(x, 0, 40, 40))
            })
            .expect("room active")
    };
    // The first tick switched the client to A (level 1): nothing yet.
    assert!(log.borrow().is_empty());
    let b = room_at(&fx, 40);
    fx.game.lists.change_room(p, b).unwrap();
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let got = log.borrow().clone();
    assert_eq!(got.len(), 1, "{got:?}");
    let (who, from, to, before) = got[0];
    assert_eq!((who, from, to), (p, 1, LEVEL));
    let reveal = fx
        .sim
        .sys
        .hooks
        .x
        .sent
        .iter()
        .position(|(_, m)| m[0] == 0x07 || m[0] == 0x08)
        .expect("the room switch's messages");
    assert!(before <= reveal, "event 3 before the room switch");
    // B → C, the same level: no event.
    let c = room_at(&fx, 80);
    fx.game.lists.change_room(p, c).unwrap();
    fx.tick();
    assert_eq!(log.borrow().len(), 1);
}

#[test]
fn a_room_left_without_clients_cancels_its_monsters_thinks() {
    // A, B, C in a row (A's array {A, B}, C's {B, C}). The player moves
    // from A to C: A has no client left, so its monster's type-2 and
    // type-3 events go; B keeps a client and its monster keeps them.
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (LEVEL, TileRect::new(16, 0, 8, 8)),
    ]);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let _ = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::JOINING);
    fx.tick();
    let rooms = act_rooms(&fx);
    let find = |fx: &Fx, x: i32| {
        rooms
            .iter()
            .copied()
            .find(|&r| {
                fx.sim.sys.hooks.drlg.subtiles(&fx.game, r) == Some(TileRect::new(x, 0, 40, 40))
            })
            .expect("room active")
    };
    let (rb, rc) = (find(&fx, 40), find(&fx, 80));
    let ma = fx.spawn(UnitType::Monster, 0, a, 12, 12);
    let mb = fx.spawn(UnitType::Monster, 0, rb, 52, 12);
    let f = fx.game.frame;
    for m in [ma, mb] {
        for ty in [2, 3, 9] {
            fx.game.schedule_event(m, ty, f + 30, None, 0, 0).unwrap();
        }
    }
    fx.game.lists.change_room(p, rc).unwrap();
    fx.tick();
    let types = |fx: &Fx, m| -> Vec<u8> { fx.timers(m).into_iter().map(|(t, _)| t).collect() };
    assert_eq!(types(&fx, ma), [9], "A: thinks and regen cancelled");
    assert!(types(&fx, mb).contains(&2), "B keeps its client");
    fx.assert_clean();
}

#[test]
fn a_room_left_without_clients_cancels_its_monsters_thinks_and_regeneration() {
    // A, B, C in a row (A's array {A, B}, C's {B, C}). Monster M in A and
    // N in B, each with an AI think (type 2) and a regeneration event
    // (type 3) far ahead, and M an event of another type (5). The player
    // walks A → C: A has no client left (its monsters get the death
    // clean-up's cancel: types 2 and 3, any argument); B keeps the client.
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (LEVEL, TileRect::new(16, 0, 8, 8)),
    ]);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    fx.game
        .lists
        .add_client(Some(p), None, client_state::JOINING);
    fx.tick();
    let room_at = |fx: &Fx, x: i32| {
        act_rooms(fx)
            .into_iter()
            .find(|&r| fx.sim.sys.hooks.drlg.subtiles(&fx.game, r).map(|t| t.x) == Some(x))
            .expect("room active")
    };
    let (b, c) = (room_at(&fx, 40), room_at(&fx, 80));
    let m = fx.spawn(UnitType::Monster, 0, a, 12, 12);
    let n = fx.spawn(UnitType::Monster, 0, b, 50, 12);
    for u in [m, n] {
        for (event, arg1) in [(2, 0), (3, 7)] {
            fx.game
                .schedule_event(u, event, 500, None, arg1, 0)
                .unwrap();
        }
    }
    fx.game.schedule_event(m, 5, 500, None, 0, 0).unwrap();
    let events = |fx: &Fx, u: UnitId| -> Vec<u8> {
        let t = &fx.game.timers;
        t.unit_timers(u)
            .into_iter()
            .filter_map(|id| t.event(id))
            .map(|e| e.0)
            .collect()
    };
    fx.game.lists.change_room(p, c).unwrap();
    fx.tick();
    assert_eq!(events(&fx, m), [5]);
    let mut kept = events(&fx, n);
    kept.sort_unstable();
    assert_eq!(kept, [2, 3]);
    fx.assert_clean();
}

/// A, B, C in a row with a 6-tile gap between A and C: C is outside A's
/// near ring (both gaps must be below 6) and inside a 16:9 view's ring
/// (gap 7, [`crate::drlg::view_gap_for_width`]).
fn view_rooms_fx(view_gap: u8) -> Fx {
    let mut fx = Fx::with_rooms(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 6, 8)),
        (LEVEL, TileRect::new(14, 0, 8, 8)),
    ]);
    fx.sim
        .sys
        .hooks
        .drlg
        .dungeon
        .set_view_gap(i32::from(view_gap));
    fx
}

fn reveals_to(fx: &Fx, p: UnitId) -> Vec<Vec<u8>> {
    fx.sim
        .sys
        .hooks
        .x
        .sent
        .iter()
        .filter(|(u, m)| *u == p && m[0] == 0x07)
        .map(|(_, m)| m.clone())
        .collect()
}

#[test]
fn the_view_gap_of_a_frame() {
    use crate::drlg::{view_gap_for_width, NEAR_GAP};
    assert_eq!(view_gap_for_width(640), NEAR_GAP);
    assert_eq!(view_gap_for_width(800), NEAR_GAP);
    assert_eq!(view_gap_for_width(1068), 7);
    assert_eq!(view_gap_for_width(1120), 7);
    assert_eq!(view_gap_for_width(1121), 8);
}

#[test]
fn a_4_3_game_joins_only_the_adjacency_array() {
    let mut fx = view_rooms_fx(0);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let game = &fx.game;
    let (d, ra) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    assert!(d.view_extras(ra).is_empty());
    let adj = d.active_room(ra).unwrap().adjacency.clone();
    assert_eq!(adj.len(), 2);
    let want: Vec<Vec<u8>> = adj.iter().map(|&r| reveal_of(d, r)).collect();
    assert_eq!(reveals_to(&fx, p), want);
    assert!(fx.game.lists.client(c).unwrap().view_rooms.is_empty());
    fx.assert_clean();
}

#[test]
fn a_wide_game_holds_and_joins_the_view_extras() {
    let mut fx = view_rooms_fx(7);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let game = &fx.game;
    let (d, ra) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    // The near ring is unchanged: A's adjacency array is {A, B}.
    let adj = d.active_room(ra).unwrap().adjacency.clone();
    assert_eq!(adj.len(), 2);
    let extras = d.view_extras(ra);
    assert_eq!(extras.len(), 1);
    let rc = extras[0];
    assert_eq!(d.room(rc).rect, TileRect::new(14, 0, 8, 8));
    // C is held at status 1: built, active, joined after the array.
    let cr = d.room(rc).active().expect("C built and active");
    assert_eq!(cr.clients, [c]);
    assert_eq!(d.room(rc).status, 1);
    let mut want: Vec<Vec<u8>> = adj.iter().map(|&r| reveal_of(d, r)).collect();
    want.push(reveal_of(d, rc));
    assert_eq!(reveals_to(&fx, p), want);
    let c_id = cr.id;
    assert_eq!(fx.game.lists.client(c).unwrap().view_rooms, [c_id]);

    // The player walks into B, whose near ring holds C: the hold from A
    // is released, C stays joined (no 0x08) and is now in the array.
    let b = fx.b;
    fx.game.lists.change_room(p, b).unwrap();
    fx.sim.sys.hooks.x.sent.clear();
    fx.tick();
    let hides = fx
        .sim
        .sys
        .hooks
        .x
        .sent
        .iter()
        .filter(|(u, m)| *u == p && m[0] == 0x08)
        .count();
    assert_eq!(hides, 0, "A and C are both in B's array");
    let game = &fx.game;
    let (d, _) = fx.sim.sys.hooks.drlg.drlg_room(game, b).unwrap();
    assert_eq!(d.room(rc).counts[1], 1, "only B's propagation holds C");
    assert!(fx.game.lists.client(c).unwrap().view_rooms.is_empty());
    fx.assert_clean();
}

#[test]
fn the_view_gap_of_a_frame_covers_both_isometric_axes() {
    use crate::drlg::{view_gap_for_frame, view_gap_for_width, NEAR_GAP};
    for width in [640, 800, 1068, 1120, 1121] {
        assert_eq!(view_gap_for_frame(width, 600), view_gap_for_width(width));
    }
    assert_eq!(view_gap_for_frame(800, 540), NEAR_GAP);
    assert_eq!(view_gap_for_frame(800, 600), NEAR_GAP);
    for (w, h) in [
        (960, 540),
        (1280, 720),
        (960, 600),
        (1280, 540),
        (1720, 720),
        (1365, 767),
        (1919, 1079),
        (800, 1600),
        (1920, 540),
        (801, 541),
    ] {
        let gap = view_gap_for_frame(w, h);
        for x in [-w, w] {
            for y in [-(h - 40), h - 40] {
                assert!(
                    i64::from(x + 2 * y).abs() <= 320 * i64::from(gap),
                    "{w}x{h}"
                );
                assert!(
                    i64::from(-x + 2 * y).abs() <= 320 * i64::from(gap),
                    "{w}x{h}"
                );
            }
        }
    }
    assert_eq!(view_gap_for_frame(800, 1600), 13);
    assert!(view_gap_for_frame(100_000, 100_000) > 255);
}

#[test]
fn a_live_view_resize_joins_and_releases_extras_without_moving_the_player() {
    let mut fx = view_rooms_fx(0);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
    let c = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.tick();
    let rc = {
        let game = &fx.game;
        let (d, ra) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
        assert!(d.view_extras(ra).is_empty());
        d.view_rooms(ra).len()
    };
    assert_eq!(rc, 2, "the near ring only");

    let changes = fx
        .sim
        .sys
        .hooks
        .drlg
        .resize_view_gap(&mut fx.game.lists, 7)
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, c);
    let game = &fx.game;
    let (d, ra) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    let extras = d.view_extras(ra);
    assert_eq!(extras.len(), 1, "C joins the wider view");
    let extra = extras[0];
    let extra_room = d.room(extra).active().expect("C built and active").id;
    assert_eq!(changes[0].1.view, [extra_room]);
    assert_eq!(changes[0].1.joined.len(), 1);
    assert_eq!(changes[0].1.joined[0].room, extra_room);
    assert_eq!(d.room(extra).active().unwrap().clients, [c]);
    assert_eq!(d.room(extra).status, 1);
    assert_eq!(game.lists.client(c).unwrap().room, Some(a));

    let changes = fx
        .sim
        .sys
        .hooks
        .drlg
        .resize_view_gap(&mut fx.game.lists, 6)
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].1.view.is_empty());
    assert_eq!(changes[0].1.left.len(), 1);
    assert_eq!(changes[0].1.left[0].room, extra_room);
    let game = &fx.game;
    let (d, _) = fx.sim.sys.hooks.drlg.drlg_room(game, a).unwrap();
    assert_eq!(d.room(extra).counts[1], 0, "the hold is released");
    assert_eq!(game.lists.client(c).unwrap().room, Some(a));
    assert!(game.lists.client(c).unwrap().view_rooms.is_empty());
}
