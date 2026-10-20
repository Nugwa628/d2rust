//! [`WORLD_IDS`], the world handlers' id table, checked against the C→S
//! message table `tables/sim/client-messages.tsv`, and the stub fallback
//! of a host without world systems.

use d2_sim::game::Game;
use d2_sim::units::lists::client_state;
use d2_sim::units::UnitType;

use super::*;
use crate::adapters::handlers::world::{system, Status, System, WORLD_IDS};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::seams::PlayerGate;

/// (id, kind, scope) of every C→S row.
fn tsv() -> Vec<(u8, String, String)> {
    let mut lines = CLIENT_TSV.lines();
    let head: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = |n: &str| head.iter().position(|h| *h == n).unwrap();
    let (id, kind, scope) = (col("id"), col("kind"), col("scope"));
    lines
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let n = u8::from_str_radix(f[id].trim_start_matches("0x"), 16).unwrap();
            (n, f[kind].to_string(), f[scope].to_string())
        })
        .collect()
}

/// The table's problems: unsorted or repeated ids, or an id handled here
/// whose TSV row is not a `handler` of scope `sim`.
fn problems(ids: &[(u8, &str, Status)], rows: &[(u8, String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for w in ids.windows(2) {
        if w[0].0 >= w[1].0 {
            out.push(format!("order {:#04x}", w[1].0));
        }
    }
    for &(id, _, status) in ids {
        let row = rows.iter().find(|r| r.0 == id);
        let ok = row.is_some_and(|r| r.1 == "handler" && r.2 == "sim");
        if matches!(status, Status::Implemented(_)) && !ok {
            out.push(format!("{id:#04x} not a sim handler"));
        }
    }
    out
}

#[test]
fn world_ids_are_sim_handlers() {
    let rows = tsv();
    assert_eq!(problems(WORLD_IDS, &rows), Vec::<String>::new());
    assert_eq!(system(0x49), Some(System::Waypoints));
    assert_eq!(system(0x3E), None);
    assert_eq!(system(0x01), None);
    // A row turned into a stub is reported.
    let mut bad = rows.clone();
    bad.iter_mut().find(|r| r.0 == 0x58).unwrap().1 = "stub0".into();
    assert_eq!(problems(WORLD_IDS, &bad), ["0x58 not a sim handler"]);
}

/// Without world systems ([`super::super::NoWorld`], the default), a
/// world id keeps the stub: recorded as unhandled, result 0.
#[test]
fn no_world_keeps_the_stub() {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let room = game.lists.create_room(0).unwrap();
    game.lists.activate_room(room).unwrap();
    let player = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    let mut s = SimGame::new(game);
    s.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    s.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode: 1,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    let mut h = host(s);
    let (code, got) = send(&mut h, &hex("58 0500"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(h.game.unhandled, vec![(0, 0x58, 3)]);
}
