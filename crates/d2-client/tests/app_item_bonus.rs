//! Item bonuses on the wire: the stat messages carry
//! the base only; the client adds the property lists of the equipped
//! items itself. A synthetic character gets a cap with +5 strength, +20
//! fire resist and +10 max life through S→C 0x9D, then moves it to the
//! grid.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::bridge::Bridge;
use d2_sim::items::bitstream::{write, Isc, StatEntry, StreamItem};
use d2_sim::items::ItemTables;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

const STR: u16 = 0;
const MAX_LIFE: u16 = 7;
const FIRE_RES: u16 = 39;

fn tables() -> (ItemTables, Vec<Isc>) {
    let mut isc = vec![Isc::default(); 64];
    for (s, shift) in [(STR, 0u8), (MAX_LIFE, 8), (FIRE_RES, 0)] {
        isc[usize::from(s)] = Isc {
            valshift: shift,
            save_bits: 12,
            save_add: 0,
            save_param_bits: 0,
        };
    }
    let t = ItemTables {
        items: vec![d2_sim::items::tables::ItemRec {
            code: *b"cap ",
            ..Default::default()
        }],
        valshift: isc.iter().map(|i| i.valshift).collect(),
        isc: isc.clone(),
        ..ItemTables::default()
    };
    (t, isc)
}

/// The cap's 0x9D record: owner = the player, `mode` / `body` / `page`
/// as given.
#[allow(clippy::ptr_arg)] // `write` takes a `&dyn IscTable`, implemented for `Vec`
fn record(isc: &Vec<Isc>, owner: u32, mode: u32, body: u8, page: u8) -> Vec<u8> {
    let item = StreamItem {
        flags: 0x10,
        version: 0x65,
        mode,
        body_loc: body,
        page,
        code: *b"cap ",
        base_code: *b"cap ",
        ilvl: 10,
        quality: 2,
        main: Some(vec![
            StatEntry {
                stat: STR,
                param: 0,
                value: 5,
            },
            StatEntry {
                stat: MAX_LIFE,
                param: 0,
                value: 10 << 8,
            },
            StatEntry {
                stat: FIRE_RES,
                param: 0,
                value: 20,
            },
        ]),
        ..StreamItem::default()
    };
    let (s, _) = write(&item, isc).unwrap();
    let mut m = vec![0x9D, 0x06, (13 + s.len()) as u8, 0x10];
    m.extend_from_slice(&0x700u32.to_le_bytes());
    m.push(0);
    m.extend_from_slice(&owner.to_le_bytes());
    m.extend_from_slice(&s);
    m
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn equipped_item_lists_add_to_the_total_and_the_base_stays() {
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
    let (t, isc) = tables();
    bridge.set_item_tables(Arc::new(d2_client::app::items::TableDecoder(Arc::new(t))));
    let read = |b: &Bridge<_>| {
        let w = b.world();
        [STR, MAX_LIFE, FIRE_RES].map(|s| (w.base(me, s, 0), w.total(me, s, 0)))
    };
    let before = read(&bridge);
    assert!(before.iter().all(|&(b, t)| b == t), "{before:?}");

    // Equipped on the head (body location 1, mode 1).
    bridge
        .receive_chunk(&record(&isc, me.guid, 1, 1, 0xFF))
        .unwrap();
    let on = read(&bridge);
    let add = [5, 10 << 8, 20];
    for i in 0..3 {
        assert_eq!(on[i].0, before[i].0, "the base is unchanged");
        assert_eq!(on[i].1, before[i].1 + add[i], "total = base + item");
    }

    // Moved to the inventory grid (mode 0, page 0): the list detaches.
    bridge
        .receive_chunk(&record(&isc, me.guid, 0, 0, 1))
        .unwrap();
    assert_eq!(read(&bridge), before, "unequip restores the total");

    // Back on the body: attached again.
    bridge
        .receive_chunk(&record(&isc, me.guid, 1, 1, 0xFF))
        .unwrap();
    assert_eq!(read(&bridge)[0].1, before[0].1 + 5);
}
