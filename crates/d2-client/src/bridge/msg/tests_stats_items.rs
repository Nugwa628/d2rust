//! Test vectors of the stat and item messages: "A" and "B" are two
//! recorded packet traces, seq numbers in the comments; synthetic where
//! marked.

use super::super::world::{
    ItemData, ItemRecord, KindData, PlayerData, UnitKey, UseCursor, ITEM, PLAYER,
};
use super::support::{hex, Model};

const P1: UnitKey = UnitKey::new(PLAYER, 1);

fn with_local() -> Model {
    let mut m = Model::default();
    m.put(P1).kind = KindData::Player(PlayerData::default());
    m.w.local_player = Some(P1);
    m
}

#[test]
fn local_player_stats() {
    let mut m = with_local();
    // B 115, 119, 122, 141.
    m.hex("1d 00 0a")
        .hex("1e 07 00 28")
        .hex("1d 0c 01")
        .hex("1b ad 01");
    let u = m.unit(P1);
    assert_eq!(
        (u.stat(0), u.stat(7), u.stat(12), u.stat(13)),
        (10, 0x2800, 1, 0x1AD)
    );
    // Synthetic: 0x1A adds, 0x1C sets absolutely, 0x1F a dword.
    m.hex("1a 42");
    assert_eq!(m.unit(P1).stat(13), 0x1EF);
    m.hex("1c 00 00 01 00");
    assert_eq!(m.unit(P1).stat(13), 0x10000);
    m.hex("1f 0e 78 56 34 12");
    assert_eq!(m.unit(P1).stat(14), 0x1234_5678);
    m.hex("19 05");
    assert_eq!(m.unit(P1).stat(14), 0x1234_567D);
    assert!(m.log.rejected.is_empty());
    // No local player: fatal 0x9AA.
    let mut m = Model::default();
    m.hex("1d 00 0a");
    assert_eq!(m.rejected(), [(0x1D, "fatal assert 0x9AA".to_owned())]);
}

#[test]
fn stat_update_players_only() {
    let mut m = with_local();
    let mut b = hex("20 01 00 00 00 0c");
    b.extend(7u32.to_le_bytes());
    m.recv(&b);
    assert_eq!(m.unit(P1).stat(12), 7);
    // Another GUID: no player, nothing.
    b[1] = 2;
    m.recv(&b);
    assert_eq!(m.w.units.len(), 1);
}

#[test]
fn item_actions() {
    // B 123: 0x9C action 0x0E creates item (4, 1).
    let mut m = Model::default();
    m.hex("9c 0e 14 10 01 00 00 00 10 00 a2 00 65 08 00 80 06 17 03 02");
    let u = m.unit(UnitKey::new(ITEM, 1));
    assert_eq!(u.position, None);
    assert_eq!(
        u.kind,
        KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: 0x9C,
                action: 0x0E,
                category: 0x10,
                owner: None,
                seq: 0,
                stream: hex("10 00 a2 00 65 08 00 80 06 17 03 02"),
            }),
            flags4: false,
            // +0x18 := the stream's flag word.
            flags: 0x00A2_0010,
            fields: None,
            props: Vec::new(),
            charm: false,
            unlinked: false,
            stats_off: false,
        })
    );
    // A fatal action and an ignored one.
    m.hex("9c 05 08 10 02 00 00 00")
        .hex("9c 20 08 10 02 00 00 00");
    assert_eq!(m.rejected(), [(0x9C, "fatal assert 0xF6D".to_owned())]);
    assert_eq!(m.w.units.len(), 1);

    // B 129: 0x9D action 6 with owner (0, 1) needs the local player and
    // the owner.
    let mut b = hex("9d 06 21 05 07 00 00 00 00 01 00 00 00 11 00");
    b.resize(0x21, 0);
    let mut m = Model::default();
    m.recv(&b);
    assert!(m.w.units.is_empty());
    let mut m = with_local();
    m.recv(&b);
    let KindData::Item(d) = &m.unit(UnitKey::new(ITEM, 7)).kind else {
        panic!("item data");
    };
    let last = d.last.as_ref().unwrap();
    assert_eq!(
        (
            last.id,
            last.action,
            last.category,
            last.owner,
            last.stream.len()
        ),
        (0x9D, 6, 5, Some(P1), 0x21 - 13)
    );
    // An action of 0x9C in 0x9D: fatal 0xFB7.
    let mut b2 = b.clone();
    b2[1] = 0x0E;
    m.recv(&b2);
    assert_eq!(m.rejected(), [(0x9D, "fatal assert 0xFB7".to_owned())]);
}

#[test]
fn clear_cursor() {
    let mut m = with_local();
    // Without a cursor item: no change.
    let before = m.w.clone();
    m.hex("42 00 01 00 00 00");
    assert_eq!(m.w, before);
    // A 77949 with a cursor item (4, 9) (cursor state synthetic).
    m.put(UnitKey::new(ITEM, 9));
    if let KindData::Player(p) = &mut m.w.units.get_mut(&P1).unwrap().kind {
        p.cursor_item = Some(9);
    }
    m.hex("42 00 01 00 00 00");
    assert!(!m.w.units.contains_key(&UnitKey::new(ITEM, 9)));
    assert_eq!(m.unit(P1).kind, KindData::Player(PlayerData::default()));
}

#[test]
fn use_stackable_item() {
    let k = UnitKey::new(ITEM, 5);
    let mut m = Model::default();
    m.put(k).kind = KindData::Item(ItemData::default());
    let flag = |m: &Model| match &m.unit(k).kind {
        KindData::Item(d) => d.flags4,
        _ => panic!("item data"),
    };
    m.hex("3f 04 05 00 00 00 ff ff");
    assert!(flag(&m));
    assert_eq!(m.w.use_cursor, Some(UseCursor { item: k, code: 4 }));
    m.hex("3f ff 05 00 00 00 ff ff");
    assert!(!flag(&m));
    assert_eq!(m.w.use_cursor, None);
    // Another argument: the flag stays, the cursor is set.
    m.hex("3f 02 05 00 00 00 01 00");
    assert!(!flag(&m));
    assert_eq!(m.w.use_cursor, Some(UseCursor { item: k, code: 2 }));
    // Item not in the set: nothing more.
    m.hex("3f 04 06 00 00 00 ff ff");
    assert_eq!(m.w.use_cursor, Some(UseCursor { item: k, code: 2 }));
}

#[test]
fn relators_change_no_field() {
    let mut m = with_local();
    let before = m.w.clone();
    // A 76132.
    m.hex("47 00 00 01 00 00 00 00 00 00 00")
        .hex("48 00 00 01 00 00 00 00 00 00 00");
    assert_eq!(m.w, before);
    assert_eq!((m.log.handled, m.log.rejected.len()), (2, 0));
}

#[test]
fn item_header_and_cursor_writes() {
    use super::stats_items::ItemHeader;
    // B 123's stream: version 101, mode 2 (belt, the 0x0E PutInBelt).
    let h = ItemHeader::peek(&hex("10 00 a2 00 65 08 00 80 06 17 03 02")).unwrap();
    assert_eq!((h.flags, h.mode, h.page), (0x00A2_0010, 2, 0xFF));
    assert_eq!(ItemHeader::peek(&hex("10 00 a2")), None);
    // A synthetic ground header: mode 3, x 0x1234, y 0x5678.
    let ground = |action: u8, mode: u32, guid: u8| {
        let mut b = vec![0x9C, action, 0, 0x10, guid, 0, 0, 0];
        let mut bits: Vec<(u32, u32)> = vec![(0x10, 32), (101, 10), (mode, 3)];
        if mode == 3 {
            bits.extend([(0x1234, 16), (0x5678, 16)]);
        } else {
            bits.extend([(0, 4), (0, 4), (0, 4), (0, 3)]);
        }
        let mut out = Vec::new();
        let mut pos = 0usize;
        for (v, n) in bits {
            for i in 0..n {
                if pos / 8 == out.len() {
                    out.push(0);
                }
                out[pos / 8] |= (((v >> i) & 1) as u8) << (pos % 8);
                pos += 1;
            }
        }
        b.extend(out);
        b[2] = b.len() as u8;
        b
    };
    let mut m = with_local();
    m.recv(&ground(0x03, 3, 5));
    assert_eq!(
        m.unit(UnitKey::new(ITEM, 5)).position,
        Some((0x1234, 0x5678))
    );
    // GroundToCursor with a mode-4 header: the local player's cursor.
    m.recv(&ground(0x01, 4, 6));
    let cursor = |m: &Model| match &m.unit(P1).kind {
        KindData::Player(p) => p.cursor_item,
        _ => None,
    };
    assert_eq!(cursor(&m), Some(6));
    assert_eq!(m.unit(UnitKey::new(ITEM, 6)).position, None);
    // GroundToCursor with another mode: nothing.
    m.recv(&ground(0x01, 3, 7));
    assert!(!m.w.units.contains_key(&UnitKey::new(ITEM, 7)));
    // PutInBelt always clears.
    m.recv(&ground(0x0E, 2, 6));
    assert_eq!(cursor(&m), None);
    assert!(m.log.rejected.is_empty());
    // Each write on the local player is followed
    // by the cursor refresh, also when the cursor was already empty.
    use crate::bridge::output::Output;
    m.recv(&ground(0x0E, 2, 8));
    let refresh: Vec<bool> = m
        .out
        .iter()
        .filter_map(|o| match o {
            Output::CursorRefresh { item } => Some(*item),
            _ => None,
        })
        .collect();
    assert_eq!(refresh, [true, false, false]);
}

/// A 0x9C item message for item `guid` whose stream header has `mode`
/// (grid, belt and cursor modes: body location, x, y and page all 0).
fn owned_item(action: u8, mode: u32, guid: u8) -> Vec<u8> {
    let mut b = vec![0x9C, action, 0, 0x10, guid, 0, 0, 0];
    let bits: Vec<(u32, u32)> = vec![(0x10, 32), (101, 10), (mode, 3), (0, 15)];
    let mut out = Vec::new();
    let mut pos = 0usize;
    for (v, n) in bits {
        for i in 0..n {
            if pos / 8 == out.len() {
                out.push(0);
            }
            out[pos / 8] |= (((v >> i) & 1) as u8) << (pos % 8);
            pos += 1;
        }
    }
    b.extend(out);
    b[2] = b.len() as u8;
    b
}

/// Owner bug 2026-10-20 #2: a swap in a grid (0x0D, the cursor weapon
/// clicked on a backpack weapon) or in the belt (0x10) sends one message
/// per item. The item taken out (cursor header, mode 4) becomes the cursor
/// item; the item put in (grid mode 0 / belt mode 2) must not, or the old
/// item's art stays on the cursor and every later click names an item the
/// server no longer has on the cursor. Both arrival orders end the same.
#[test]
fn a_swap_puts_the_taken_item_on_the_cursor() {
    let cursor = |m: &Model| match &m.unit(P1).kind {
        KindData::Player(p) => p.cursor_item,
        _ => None,
    };
    for (action, placed) in [(0x0D, 0), (0x10, 2)] {
        for placed_first in [false, true] {
            let mut m = with_local();
            // Item 6 is on the cursor; item 9 sits where it is clicked.
            m.recv(&owned_item(0x04, 0, 9));
            m.recv(&owned_item(0x01, 4, 6));
            assert_eq!(cursor(&m), Some(6));
            let taken = owned_item(action, 4, 9);
            let put = owned_item(action, placed, 6);
            if placed_first {
                m.recv(&put);
                m.recv(&taken);
            } else {
                m.recv(&taken);
                m.recv(&put);
            }
            assert_eq!(
                cursor(&m),
                Some(9),
                "action {action:#04x}, placed item first: {placed_first}"
            );
            assert!(m.log.rejected.is_empty());
        }
    }
}

/// An `hst ` or `qf2 ` the local player gets in
/// page 3 (0x9C action 4) hands the Horadric start to the UI; a gem
/// there, or an `hst ` on page 0, does not.
#[test]
fn a_staff_in_page_3_starts_the_horadric_animation() {
    use crate::bridge::output::Output;
    let stored = |guid: u8, page: u32, code: &[u8; 4]| {
        let mut b = vec![0x9C, 0x04, 0, 0x10, guid, 0, 0, 0];
        let bits: Vec<(u32, u32)> = vec![
            (0x10, 32),
            (101, 10),
            (0, 3),
            (0, 4),
            (0, 4),
            (0, 4),
            (page + 1, 3),
            (u32::from_le_bytes(*code), 32),
        ];
        let mut out = Vec::new();
        let mut pos = 0usize;
        for (v, n) in bits {
            for i in 0..n {
                if pos / 8 == out.len() {
                    out.push(0);
                }
                out[pos / 8] |= (((v >> i) & 1) as u8) << (pos % 8);
                pos += 1;
            }
        }
        b.extend(out);
        b[2] = b.len() as u8;
        b
    };
    let horadric = |m: &Model| {
        m.out
            .iter()
            .filter_map(|o| match o {
                Output::HoradricItem { code } => Some(*code),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let mut m = with_local();
    m.recv(&stored(5, 3, b"gsw "));
    m.recv(&stored(6, 0, b"hst "));
    assert!(horadric(&m).is_empty());
    m.recv(&stored(7, 3, b"hst "));
    m.recv(&stored(8, 3, b"qf2 "));
    assert_eq!(horadric(&m), [*b"hst ", *b"qf2 "]);
}

/// 0x0B / 0x0C place into / take out of the store
/// unit, never the local player: no cursor write, no refresh, no Horadric
/// start; the same item by 0x04 clears and refreshes (gen-ui-shop tick
/// 100, the cursor frame clock).
#[test]
fn store_actions_never_write_the_cursor() {
    use crate::bridge::output::Output;
    let stored = |action: u8, guid: u8, page: u32, code: &[u8; 4]| {
        let mut b = vec![0x9C, action, 0, 0x10, guid, 0, 0, 0];
        let bits: Vec<(u32, u32)> = vec![
            (0x10, 32),
            (101, 10),
            (0, 3),
            (0, 4),
            (0, 4),
            (0, 4),
            (page + 1, 3),
            (u32::from_le_bytes(*code), 32),
        ];
        let mut out = Vec::new();
        let mut pos = 0usize;
        for (v, n) in bits {
            for i in 0..n {
                if pos / 8 == out.len() {
                    out.push(0);
                }
                out[pos / 8] |= (((v >> i) & 1) as u8) << (pos % 8);
                pos += 1;
            }
        }
        b.extend(out);
        b[2] = b.len() as u8;
        b
    };
    let writes = |m: &Model| {
        m.out
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    Output::CursorRefresh { .. } | Output::HoradricItem { .. }
                )
            })
            .count()
    };
    let mut m = with_local();
    m.recv(&stored(0x0B, 5, 0, b"hp1 "));
    m.recv(&stored(0x0B, 6, 3, b"hst "));
    m.recv(&stored(0x0C, 7, 0, b"hp1 "));
    assert_eq!(writes(&m), 0);
    m.recv(&stored(0x04, 8, 0, b"hp1 "));
    assert_eq!(writes(&m), 1);
}

/// Body swaps send an outgoing cursor record followed by the incoming body
/// record. Neither packet may leave the cursor pointing at the incoming item.
#[test]
fn body_swap_packets_preserve_items_and_cursor_identity() {
    fn packet(action: u8, guid: u32, mode: u32, loc: u32, flags: u32, code: [u8; 4]) -> Vec<u8> {
        let mut b = vec![0x9D, action, 0, 0x10];
        b.extend(guid.to_le_bytes());
        b.push(PLAYER);
        b.extend(P1.guid.to_le_bytes());
        let mut stream = vec![0u8; 12];
        let mut pos = 0;
        for (value, count) in [
            (flags | 0x20_0010, 32),
            (101, 10),
            (mode, 3),
            (loc, 4),
            (0, 4),
            (0, 4),
            (0, 3),
            (u32::from_le_bytes(code), 32),
        ] {
            for bit in 0..count {
                stream[pos / 8] |= (((value >> bit) & 1) as u8) << (pos % 8);
                pos += 1;
            }
        }
        b.extend(stream);
        b[2] = b.len() as u8;
        b
    }
    use crate::bridge::items;
    for loc in 1..=12 {
        for code in [
            *b"opl ", *b"tkf ", *b"jav ", *b"aqv ", *b"buc ", *b"2hs ", *b"cap ",
        ] {
            let mut m = with_local();
            m.recv(&packet(6, 8, 1, loc, 0, *b"ssd "));
            m.recv(&packet(8, 9, 4, 0, 0, code));
            assert_eq!(items::cursor_item(&m.w).unwrap().key.guid, 9);
            m.recv(&packet(9, 8, 4, loc, 0x80, *b"ssd "));
            assert_eq!(items::cursor_item(&m.w).unwrap().key.guid, 8);
            m.recv(&packet(9, 9, 1, loc, 0x40, code));
            assert_eq!(items::cursor_item(&m.w).unwrap().key.guid, 8);
            assert_eq!(items::cursor_item(&m.w).unwrap().mode, 4);
            assert_eq!(items::local_items(&m.w).len(), 2);
            assert_eq!(items::drop(8).item, 8);
            // Storing the displaced sword clears the cursor. Both units survive.
            let mut stored = packet(4, 8, 0, 0, 0, *b"ssd ");
            stored[0] = 0x9C;
            stored.drain(8..13);
            stored[2] = stored.len() as u8;
            m.recv(&stored);
            assert!(items::cursor_item(&m.w).is_none());
            assert!(m.w.units.contains_key(&UnitKey::new(ITEM, 8)));
            assert!(m.w.units.contains_key(&UnitKey::new(ITEM, 9)));
            assert!(m.log.rejected.is_empty());
        }
    }
}
