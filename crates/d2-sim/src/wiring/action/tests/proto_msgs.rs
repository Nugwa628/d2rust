//! Tests for the add messages of a player's part B (0x20 stat updates) and of a
//! `ClientSend` missile (0x73 field sources).

use super::*;
use crate::missiles::{create_missile, param_flags, MissileParams};

/// Sends the add messages of `unit` to `receiver` and returns them.
fn add(fx: &mut Fx, receiver: UnitId, unit: UnitId) -> Vec<Vec<u8>> {
    let s = &mut fx.sim.sys;
    let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
    v.add_messages(&fx.game, receiver, unit);
    std::mem::take(&mut fx.sim.hooks().x.sent)
        .into_iter()
        .map(|(to, b)| {
            assert_eq!(to, receiver);
            b
        })
        .collect()
}

/// Part B of the add messages: another player's base velocity, attack
/// rate, level, strength and dexterity (stats 67, 68, 12, 0, 2) go first
/// as 0x20; the receiver's own part B has none.
#[test]
fn player_part_b_sends_the_five_0x20_stats_to_others_only() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let p0 = fx.spawn(UnitType::Player, 0, a, 20, 20);
    let p1 = fx.spawn(UnitType::Player, 0, a, 22, 20);
    fx.stats(p1, &[(67, 0), (68, 0), (12, 5), (0, 20), (2, 25)]);
    let g1 = fx.game.lists.unit(p1).unwrap().guid;
    let got = add(&mut fx, p0, p1);
    // Message 0 is the 0x59 assign, 1 its 0x75 party info; part B follows.
    let want: Vec<Vec<u8>> = [(67u8, 0u32), (68, 0), (12, 5), (0, 20), (2, 25)]
        .iter()
        .map(|&(s, v)| crate::units::messages::stat_update(g1, s, v).to_vec())
        .collect();
    assert_eq!(got[0][0], 0x59);
    assert_eq!(got[1][0], 0x75);
    assert_eq!(&got[2..7], &want[..]);
    // The receiver's own part B: no 0x20.
    let own = add(&mut fx, p0, p0);
    assert!(own.iter().all(|m| m[0] != 0x20));
}

/// The position is the path's cells
/// (`precise >> 16`), the first point is the target while the velocity is
/// non-zero (else 0, 0), and the message is sent for velocity 0 too.
#[test]
fn client_missile_0x73_field_sources() {
    for (velocity, first) in [(5u32, (300u32, 400u32)), (0, (0, 0))] {
        let mut fx = Fx::new();
        fx.sim.hooks().enable_paths().expect("embedded tables");
        Arc::make_mut(&mut fx.sim.sys.hooks.tables).missiles[0].clientsend = true;
        let a = fx.a;
        let owner = fx.spawn(UnitType::Monster, 0, a, 10, 10);
        let p0 = fx.spawn(UnitType::Player, 0, a, 20, 20);
        let params = MissileParams {
            owner: Some(owner),
            origin: Some(owner),
            class: 0,
            flags: param_flags::TARGET_ABSOLUTE,
            target_x: 13,
            target_y: 10,
            ..MissileParams::default()
        };
        let m = fx
            .sim
            .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &params))
            .unwrap()
            .expect("created");
        {
            let p = fx
                .sim
                .sys
                .hooks
                .paths
                .as_mut()
                .and_then(|p| p.dynamic_mut(m))
                .expect("dynamic path");
            p.precise_x = 0x0012_3456;
            p.precise_y = 0x0045_6789;
            p.target_x = 300;
            p.target_y = 400;
            p.velocity = velocity as i32;
        }
        let got = add(&mut fx, p0, m);
        assert_eq!(got.len(), 1, "velocity {velocity}");
        let b = &got[0];
        assert_eq!(b[0], 0x73);
        assert_eq!(u32::from_le_bytes(b[7..11].try_into().unwrap()), 0x12);
        assert_eq!(u32::from_le_bytes(b[11..15].try_into().unwrap()), 0x45);
        assert_eq!(u32::from_le_bytes(b[15..19].try_into().unwrap()), first.0);
        assert_eq!(u32::from_le_bytes(b[19..23].try_into().unwrap()), first.1);
    }
}

/// The join with two clients: the joiner J gets C's 0x5B, its own 0x5B,
/// one 0x65 per in-game player, one 0x8D per player unit and the 0x5A; C
/// gets J's 0x5B and the 0x5A.
#[test]
fn join_sequence_with_two_clients_sends_per_recipient() {
    use crate::tick::TickHooks;
    use crate::units::lists::client_state;
    use crate::units::messages as m;
    let mut fx = Fx::new();
    let a = fx.a;
    let p0 = fx.spawn(UnitType::Player, 1, a, 20, 20);
    let p1 = fx.spawn(UnitType::Player, 2, a, 22, 20);
    let _c0 = fx
        .game
        .lists
        .add_client(Some(p0), Some(a), client_state::IN_GAME);
    let c1 = fx
        .game
        .lists
        .add_client(Some(p1), Some(a), client_state::IN_GAME);
    let mut n0 = [0u8; 16];
    n0[..3].copy_from_slice(b"Old");
    let mut n1 = [0u8; 16];
    n1[..3].copy_from_slice(b"New");
    fx.sim.sys.hooks.session.names.insert(p0, n0);
    fx.sim.sys.hooks.session.names.insert(p1, n1);
    fx.stats(p0, &[(12, 7)]);
    fx.stats(p1, &[(12, 9)]);
    let (g0, g1) = (
        fx.game.lists.unit(p0).unwrap().guid,
        fx.game.lists.unit(p1).unwrap().guid,
    );
    fx.sim.hooks().x.sent.clear();
    fx.sim.join_sequence(&mut fx.game, c1);
    let sent = std::mem::take(&mut fx.sim.hooks().x.sent);
    let to = |u: UnitId| -> Vec<Vec<u8>> {
        sent.iter()
            .filter(|(t, _)| *t == u)
            .map(|(_, b)| b.clone())
            .collect()
    };
    let j5b = |g, class, name: &[u8; 16], lvl| m::player_joined(g, class, name, lvl, m::NO_PARTY);
    let order: Vec<u32> = fx
        .game
        .lists
        .units_of_type(UnitType::Player)
        .into_iter()
        .map(|u| fx.game.lists.unit(u).unwrap().guid)
        .collect();
    // The 0x65 follow the client list's order (new clients are prepended).
    let list: Vec<u32> = fx
        .game
        .lists
        .clients()
        .into_iter()
        .filter_map(|c| fx.game.lists.client(c).and_then(|e| e.player))
        .map(|p| fx.game.lists.unit(p).unwrap().guid)
        .collect();
    assert_eq!(list, [g1, g0]);
    let mut want_j = vec![j5b(g0, 1, &n0, 7), j5b(g1, 2, &n1, 9)];
    want_j.extend(list.iter().map(|&g| m::player_kill_count(g, 0).to_vec()));
    want_j.extend(
        order
            .iter()
            .map(|&g| m::assign_player_to_party(g, m::NO_PARTY).to_vec()),
    );
    want_j.push(m::player_event(2, &n1).to_vec());
    assert_eq!(to(p1), want_j);
    assert_eq!(
        to(p0),
        vec![j5b(g1, 2, &n1, 9), m::player_event(2, &n1).to_vec()]
    );
}

/// The unit update for the client's own player: the
/// state messages first, then the stat sends of 67, 68, 12, 0, 2 whose
/// keys are in the mod array. At a town arrival that is: 0xA8 of
/// state 105, `1d 0c 01`, `1d 00 14`, `1d 02 19`, nothing for 67 / 68.
#[test]
fn own_player_update_sends_states_then_its_changed_stats() {
    use crate::tick::TickHooks;
    use crate::units::lists::client_state;
    let mut fx = Fx::new();
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 20, 20);
    let c = fx
        .game
        .lists
        .add_client(Some(p), Some(a), client_state::IN_GAME);
    fx.tick();
    fx.stats(p, &[(12, 1), (0, 20), (2, 25), (67, 100), (68, 100)]);
    fx.sim.with(&mut fx.game, |g, v| v.set_alignment(g, p, 2));
    fx.sim.hooks().x.sent.clear();
    fx.sim.send_unit_update(&mut fx.game, c, p);
    let sent: Vec<Vec<u8>> = std::mem::take(&mut fx.sim.hooks().x.sent)
        .into_iter()
        .map(|(_, b)| b)
        .collect();
    // The fixture has no itemstatcost send columns, so the 0xA8 stream
    // is the list bit and 0x1FF only (its bytes: `messages.rs` tests).
    let guid = fx.game.lists.unit(p).unwrap().guid.to_le_bytes();
    let mut a8 = vec![0xA8, 0];
    a8.extend_from_slice(&guid);
    a8.extend_from_slice(&[0x0A, 0x69, 0xFF, 0x01]);
    assert_eq!(
        sent,
        [a8, vec![0x1D, 12, 1], vec![0x1D, 0, 20], vec![0x1D, 2, 25]]
    );
    // With the mod array cleared, the stat sends are empty.
    fx.sim.sys.stats.clear_mods(p);
    fx.sim.send_unit_update(&mut fx.game, c, p);
    let sent = std::mem::take(&mut fx.sim.hooks().x.sent);
    assert!(sent.iter().all(|(_, b)| b[0] != 0x1D), "{sent:?}");
}

/// Part B, item case: the
/// join's ground-update mark goes out only for an item whose unit flag
/// 0x10 is clear; a new item (flag set at alloc) sends nothing at the
/// join. The flag is read at the join, before the room clean-up.
#[test]
fn the_join_item_mark_waits_for_flag_0x10_to_clear() {
    use crate::units::record::flags::SEED_SET;
    let mut fx = Fx::new();
    fx.sim.sys.hooks.item_marks = true;
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 0, a, 20, 20);
    let item = fx.spawn(UnitType::Item, 0, a, 21, 20);
    assert_ne!(fx.sim.sys.units.get(item).unwrap().flags & SEED_SET, 0);
    assert_eq!(add(&mut fx, p, item), Vec::<Vec<u8>>::new());
    fx.sim.sys.units.get_mut(item).unwrap().flags &= !SEED_SET;
    let g = fx.game.lists.unit(item).unwrap().guid;
    let mut mark = vec![super::super::JOIN_ITEM_MARK];
    mark.extend_from_slice(&g.to_le_bytes());
    assert_eq!(add(&mut fx, p, item), [mark]);
}
