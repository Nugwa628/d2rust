//! Gold in the play game on the synthetic game: S→C 0x9C places a gold
//! pile on the ground with its amount (`bridge::items`), the pick-up
//! intent leaves for the server, and the vitals sync's gold messages
//! (0x19 / 0x1D / 0x1E / 0x1F, stat 14) change the gold the inventory
//! shows.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::bridge::{items, Bridge};

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// An item stream head: flags, version,
/// mode, location, code.
fn stream(m: u8, loc: (u8, u16, u16, u8), code: &[u8; 4], gold: u32) -> Vec<u8> {
    let mut bits: Vec<(u32, u32)> = vec![(0x10 | 0x20_0000, 32), (0x65, 10), (u32::from(m), 3)];
    if matches!(m, 3 | 5) {
        bits.extend([(u32::from(loc.1), 16), (u32::from(loc.2), 16)]);
    } else {
        bits.extend([
            (u32::from(loc.0), 4),
            (u32::from(loc.1), 4),
            (u32::from(loc.2), 4),
            (u32::from(loc.3), 3),
        ]);
    }
    bits.push((u32::from_le_bytes(*code), 32));
    bits.extend([(0, 1), (gold, 12)]);
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

/// S→C 0x9C: action, size, category, GUID,
/// stream.
fn item_world(action: u8, guid: u32, s: &[u8]) -> Vec<u8> {
    let mut m = vec![0x9C, action, (8 + s.len()) as u8, 0x10];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(s);
    m
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_gold_pile_reaches_the_model_and_gold_messages_change_the_total() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let mut bridge = Bridge::new(link).unwrap();
    let frame = |b: &mut Bridge<_>| {
        ms.fetch_add(40, Ordering::SeqCst);
        b.frame().unwrap()
    };
    bridge
        .send_bytes(&single_player::create_request().encode())
        .unwrap();
    frame(&mut bridge);
    frame(&mut bridge);
    bridge.send_bytes(&[0x6B]).unwrap();
    for _ in 0..3 {
        frame(&mut bridge);
    }
    let me = bridge.world().local_player.expect("joined");

    // A 250-gold pile on the ground.
    let chunk = item_world(0x00, 0x701, &stream(3, (0, 500, 600, 0), b"gld ", 250));
    bridge.receive_chunk(&chunk).unwrap();
    let ground = items::ground_items(bridge.world());
    assert_eq!(
        ground
            .iter()
            .map(|i| (i.key.guid, i.code, i.gold))
            .collect::<Vec<_>>(),
        [(0x701, Some(*b"gld "), Some(250))]
    );

    // The vitals sync's gold messages: delta 0x19, then 0x1D /
    // 0x1E / 0x1F with the new total.
    let gold = |b: &Bridge<_>| b.world().total(me, 14, 0);
    let before = gold(&bridge);
    bridge.receive_chunk(&[0x19, 200]).unwrap();
    assert_eq!(gold(&bridge), before + 200);
    bridge.receive_chunk(&[0x1D, 14, 250]).unwrap();
    assert_eq!(gold(&bridge), 250);
    bridge.receive_chunk(&[0x1E, 14, 0x10, 0x27]).unwrap();
    assert_eq!(gold(&bridge), 10_000);
    bridge
        .receive_chunk(&[0x1F, 14, 0x40, 0x42, 0x0F, 0x00])
        .unwrap();
    assert_eq!(gold(&bridge), 1_000_000);

    // The pick-up intent is a valid C→S message; the pile goes when the
    // server frees it (0x0A-style removal is the item stream's).
    bridge.send(&items::pick(0x701, false)).unwrap();
    frame(&mut bridge);
}
