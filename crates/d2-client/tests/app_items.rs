//! The item stream of the play game (gap G16) on the synthetic game:
//! after the join, S→C 0x9C item messages (a belt potion, a ground item)
//! place the items in the client model (`bridge::items`), the belt key
//! sends C→S 0x26 and a pick-up sends C→S 0x16, both taken by the
//! server thread's link.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::bridge::{belt, items, Bridge};
use d2_client::controls::Action;
use d2_client::ui::{ActionId, UiEvent};

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// An item stream head: flags, version,
/// mode, location, code.
fn stream(m: u8, loc: (u8, u16, u16, u8), code: &[u8; 4]) -> Vec<u8> {
    let mut bits: Vec<(u32, u32)> = vec![(0x10, 32), (0x65, 10), (u32::from(m), 3)];
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
fn item_messages_fill_the_model_and_the_belt_key_and_pickup_reach_the_server() {
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
    assert!(bridge.world().local_player.is_some(), "joined");

    // A potion put in belt slot 2 (0x0E) and a cap on the ground (0x00).
    let mut chunk = item_world(0x0E, 0x700, &stream(2, (0, 2, 0, 0), b"hp1 "));
    chunk.extend(item_world(
        0x00,
        0x701,
        &stream(3, (0, 500, 600, 0), b"cap "),
    ));
    bridge.receive_chunk(&chunk).unwrap();
    let w = bridge.world();
    // The new sorceress's start items put four potions in belt slots 0-3
    // (a known join's four 0x9C action
    // 0x0E), so every column is ready.
    assert_eq!(w.belt_ready, [true, true, true, true]);
    let belt = items::belt(w);
    assert_eq!(belt.get(&2).map(|i| i.key.guid), Some(0x700));
    let ground = items::ground_items(w);
    assert_eq!(
        ground
            .iter()
            .map(|i| (i.key.guid, i.x, i.y, i.code))
            .collect::<Vec<_>>(),
        [(0x701, 500, 600, Some(*b"cap "))]
    );

    // The belt keys of column 3 (slot 2) and column 1 (slot 0, a start
    // potion) each send 0x26 to the server.
    let keys =
        [Action::BeltSlot3, Action::BeltSlot1].map(|a| UiEvent::Action(ActionId(a.index() as u16)));
    let no_ui = belt::KeyFacts::default();
    assert_eq!(belt::send_keys(&mut bridge, &keys, no_ui).unwrap(), 2);
    assert_eq!(
        belt::key_message(bridge.world(), 2, no_ui),
        Some(vec![0x26, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
    );
    // The pick-up intent of the ground item is a valid C→S message.
    bridge.send(&items::pick(0x701, false)).unwrap();
    frame(&mut bridge);
}
