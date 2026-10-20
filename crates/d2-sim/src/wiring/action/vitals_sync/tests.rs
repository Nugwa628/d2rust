//! Tests for the client vitals sync on the action fixture: when it runs,
//! what the first sync, the gold / experience follow-ups and the potion
//! predictions send, and that stat messages come from the per-client
//! update's flush rather than from the sync.

use super::*;
use crate::combat::vitals::sync::{life_mana_update2, SyncCache};
use crate::path::walk::messages::walk_verify;
use crate::wiring::action::tests::Fx;

/// A player of class 4 in room A at (10, 12) with life 40, max life 50,
/// mana 20, stamina 30 (×256), and an in-game client.
fn fx() -> (Fx, UnitId, ClientId) {
    let mut f = Fx::new();
    let a = f.a;
    let p = f.spawn(UnitType::Player, 4, a, 10, 12);
    f.stats(
        p,
        &[
            (stat::HITPOINTS, 40 << 8),
            (stat::MAXHP, 50 << 8),
            (stat::MANA, 20 << 8),
            (stat::STAMINA, 30 << 8),
        ],
    );
    let c = f
        .game
        .lists
        .add_client(Some(p), Some(a), client_state::IN_GAME);
    (f, p, c)
}

fn run_fx(f: &mut Fx, c: ClientId, queued: bool) -> Option<Vec<Vec<u8>>> {
    run(&mut f.sim, &mut f.game, c, (700, 800), queued)
}

#[test]
fn off_by_default_and_only_in_game() {
    let (mut f, p, c) = fx();
    assert_eq!(run_fx(&mut f, c, false), None, "sync off");
    f.sim.sys.hooks.enable_vitals_sync();
    f.game.lists.client_mut(c).unwrap().state = client_state::JOINING;
    assert_eq!(run_fx(&mut f, c, false), None, "state 3");
    f.game.lists.client_mut(c).unwrap().state = client_state::IN_GAME;
    f.game.lists.client_mut(c).unwrap().player = None;
    assert_eq!(run_fx(&mut f, c, false), None, "no player");
    f.game.lists.client_mut(c).unwrap().player = Some(p);
    assert!(run_fx(&mut f, c, false).is_some());
}

#[test]
fn first_sync_sends_0x95_and_resets_the_counter() {
    let (mut f, p, c) = fx();
    f.sim.sys.hooks.enable_vitals_sync();
    f.game.lists.client_mut(c).unwrap().update_count = 7;
    // Cache all zero: Δlife 40 of 50 ≥ 10 %: 0x95 at the staged position.
    let got = run_fx(&mut f, c, false).unwrap();
    // Then the stat messages of what is nonzero (the maxima), once.
    assert_eq!(
        got[0],
        life_mana_update2(40, 20, 30, 700, 800, 0, 0).to_vec()
    );
    assert!(got[1..].iter().all(|m| matches!(m[0], 0x1D..=0x1F)));
    assert_eq!(f.game.lists.client(c).unwrap().update_count, 0);
    let cache = f.sim.sys.hooks.sync.as_ref().unwrap().caches[&c];
    assert_eq!(
        cache,
        SyncCache {
            life: 40,
            mana: 20,
            stamina: 30,
            x: 700,
            y: 800,
            ..SyncCache::default()
        }
    );
    // Unchanged, not forced: nothing, the counter stays.
    f.game.lists.client_mut(c).unwrap().update_count = 9;
    assert_eq!(run_fx(&mut f, c, true), Some(vec![]));
    assert_eq!(f.game.lists.client(c).unwrap().update_count, 9);
    // Stamina only, forced at 10 with a queued buffer: 0x96.
    f.stats(p, &[(stat::STAMINA, 29 << 8)]);
    f.game.lists.client_mut(c).unwrap().update_count = 10;
    let got = run_fx(&mut f, c, true).unwrap();
    assert_eq!(got, [walk_verify(29, 700, 800, 0, 0).to_vec()]);
    assert_eq!(f.game.lists.client(c).unwrap().update_count, 0);
}

#[test]
fn gold_and_experience_follow() {
    let (mut f, p, c) = fx();
    f.sim.sys.hooks.enable_vitals_sync();
    run_fx(&mut f, c, false).unwrap();
    f.stats(p, &[(STAT_GOLD, 100), (STAT_EXPERIENCE, 300)]);
    f.game.lists.client_mut(c).unwrap().update_count = 20;
    let got = run_fx(&mut f, c, false).unwrap();
    assert_eq!(got, [vec![0x19, 100], vec![0x1B, 0x2C, 0x01]]);
}

#[test]
fn life_prediction_reads_the_healthpot_list() {
    let (mut f, p, c) = fx();
    f.sim.sys.hooks.enable_vitals_sync();
    // A state 100 list: hpregen 128 per frame until frame + 50.
    let frame = f.game.frame;
    let s = &mut f.sim.sys;
    let l = s.stats.alloc(0x40, frame + 50, 0, 0);
    s.stats.set_state(l, STATE_HEALTHPOT);
    s.stats.set(&mut s.hooks, l, STAT_HPREGEN, 128, 0, None);
    s.stats.attach(&mut s.hooks, p, l, true);
    let now = current(&f.sim, &f.game, p, (0, 0));
    // q = (128 · 50 + 40 · 256) >> 8 = 65; 65 · 100 / 50 = 130 → low
    // byte 130 > 100 → 100.
    assert_eq!(now.lp, 100);
    let got = run_fx(&mut f, c, false).unwrap();
    assert_eq!(got[0][0], 0x18);
}

/// The messages one tick sent to `p`, in send order.
fn tick_sent(f: &mut Fx, p: UnitId) -> Vec<Vec<u8>> {
    f.sim.sys.hooks.x.sent.clear();
    crate::tick::tick(&mut f.game, &mut f.sim);
    f.sim
        .sys
        .hooks
        .x
        .sent
        .iter()
        .filter(|(u, _)| *u == p)
        .map(|(_, m)| m.clone())
        .collect()
}

// The stat messages are the per-client update's flush of the changed-stat
// array, in key order, every tick the key is in the array (the flush
// empties it); they are not a diff against a per-client cache in the
// vitals sync, which sends none.
#[test]
fn changed_mod_stats_flush_in_the_per_client_update_once() {
    let (mut f, p, c) = fx();
    f.sim.sys.hooks.enable_vitals_sync();
    // The fixture's stats are in the array: the first tick flushes them.
    tick_sent(&mut f, p);
    assert_eq!(tick_sent(&mut f, p), Vec::<Vec<u8>>::new());
    // Level 1 -> 2, 5 stat points, 300 strength: byte, byte, word
    // messages, in key order (stats 0, 4, 12).
    f.stats(p, &[(12, 2), (4, 5), (stat::STRENGTH, 300)]);
    let got = tick_sent(&mut f, p);
    assert_eq!(
        got,
        [
            vec![0x1E, 0, 0x2C, 0x01],
            vec![0x1D, 4, 5],
            vec![0x1D, 12, 2],
        ]
    );
    // The vitals sync sends no stat message of its own.
    f.stats(p, &[(12, 3)]);
    f.game.lists.client_mut(c).unwrap().update_count = 20;
    let sync = run_fx(&mut f, c, false).unwrap();
    assert!(
        !sync.iter().any(|m| matches!(m[0], 0x1D..=0x1F)),
        "{sync:02X?}"
    );
    // The flush emptied the array: sent once.
    assert_eq!(tick_sent(&mut f, p), [vec![0x1D, 12, 3]]);
    assert_eq!(tick_sent(&mut f, p), Vec::<Vec<u8>>::new());
}

// The flag-ex bit 21 refresh: S→C 0x48 after the stat messages, once (the
// flush clears the bit).
#[test]
fn flag_ex_bit_21_sends_the_inventory_refresh_after_the_stats() {
    let (mut f, p, _) = fx();
    tick_sent(&mut f, p);
    f.stats(p, &[(12, 2)]);
    f.sim.sys.units.get_mut(p).unwrap().flags2 |= crate::wiring::action::INVENTORY_REFRESH_EX;
    let guid = f.sim.sys.units.get(p).unwrap().guid.to_le_bytes();
    let got = tick_sent(&mut f, p);
    let at = got.iter().position(|m| m[0] == 0x48).expect("0x48");
    assert_eq!(
        got[at],
        [0x48, 0, 0, guid[0], guid[1], guid[2], guid[3], 0, 0, 0, 0]
    );
    assert!(got[..at].contains(&vec![0x1D, 12, 2]), "{got:02X?}");
    assert!(!tick_sent(&mut f, p).iter().any(|m| m[0] == 0x48));
}

// The stat messages carry the base only; the client sums its equipped
// items' lists itself, so linked item stats are never sent.
#[test]
fn linked_item_stats_are_never_sent_and_the_base_stays() {
    let (mut f, p, c) = fx();
    f.sim.sys.hooks.enable_vitals_sync();
    run_fx(&mut f, c, false).unwrap();
    let a = f.a;
    let item = f.spawn(UnitType::Item, 0, a, 10, 12);
    f.stats(item, &[(31, 12), (stat::MAXHP, 5 << 8)]);
    assert!(run_fx(&mut f, c, false).unwrap().is_empty());
    let s = &mut f.sim.sys;
    let ir = s.stats.unit_list(item).unwrap();
    s.stats.equip(&mut s.hooks, p, Some(ir), false, true);
    let got = run_fx(&mut f, c, false).unwrap();
    assert!(
        !got.iter().any(|m| m[0] == 0xA8 || m[0] == 0xA9),
        "no state message in {got:02X?}"
    );
    assert!(
        !got.iter()
            .any(|m| m[0] == 0x1D && (m[1] == 31 || m[1] == 7)),
        "the 0x1D stays base only: {got:02X?}"
    );
    let s = &mut f.sim.sys;
    s.stats.detach(&mut s.hooks, ir);
    let got = run_fx(&mut f, c, false).unwrap();
    assert!(got.is_empty(), "{got:02X?}");
}
