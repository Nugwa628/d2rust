//! The quest code's player walk on the real unit lists and states.

use std::cmp::Reverse;

use super::*;
use crate::wiring::economy::quest_items::quest_players;

#[test]
fn every_player_walks_the_hash_buckets_without_state_7() {
    let mut w = World::new();
    // 130 players: GUIDs past 127 share buckets with the first ones.
    let mut players: Vec<(u32, UnitId)> = (0..130)
        .map(|_| {
            let p = w.spawn(UnitType::Player, 0);
            (w.units.get(p).unwrap().guid, p)
        })
        .collect();
    assert!(players.iter().any(|&(g, _)| g >= 128));
    let created: Vec<UnitId> = players.iter().map(|&(_, p)| p).collect();
    // Buckets 0 … 127 (GUID & 0x7F), each from its head: GUIDs
    // descending; not creation order.
    players.sort_by_key(|&(g, _)| (g & 0x7F, Reverse(g)));
    let want: Vec<UnitId> = players.iter().map(|&(_, p)| p).collect();
    let got = quest_players(&w.game, &w.stats);
    assert_eq!(got, want);
    assert_ne!(got, created);
    // A player in state 7 is skipped.
    let skipped = players.iter().find(|&&(g, _)| g >= 128).unwrap().1;
    assert!(w.stats.toggle_state(skipped, 7, true).changed);
    let want: Vec<UnitId> = want.into_iter().filter(|&p| p != skipped).collect();
    assert_eq!(quest_players(&w.game, &w.stats), want);
}
