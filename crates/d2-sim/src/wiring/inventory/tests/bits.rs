//! Tests for the item bit stream on the real item (`bits.rs`): the
//! runeword record's name id from the item's own inventory, and the
//! writer's changes to the item, written back to the item store.

use super::*;
use crate::items::tables::RuneRec;

/// A ground sword with `sockets` (stat 194) and an inventory holding one
/// key per entry of `fillers`; returns (sword GUID, sword unit).
fn socketed(w: &mut World, sockets: i32, fillers: usize) -> (Guid, UnitId) {
    let s = w.ground_item(SWORD, 20, 20);
    let u = w.unit(s).unwrap();
    w.set_stat(u, super::super::inv_world::STAT_SOCKETS, sockets);
    w.state.add_inventory(u, UnitKind::Item, s);
    for i in 0..fillers {
        let k = w.ground_item(KEY, 30 + i as i32, 20);
        let ku = w.unit(k).unwrap();
        w.desk(|d| {
            let mut inv = d.state.inventories.remove(&u).unwrap();
            inv.link(d, ku, None);
            d.state.inventories.insert(u, inv);
        });
    }
    (s, u)
}

/// The runeword record (fillers = the class ids of the item's inventory
/// in list order, socket count = stat 194) gives its name id; no record (count mismatch, empty inventory, a
/// magic item, a row whose runes differ) → 0xFFFF.
#[test]
fn runeword_name_from_the_items_inventory() {
    let mut w = World::new();
    w.tables.runes = vec![
        RuneRec {
            complete: 1,
            itype: [T_WEAP as i16, 0, 0, 0, 0, 0],
            runes: [HP1 as i32, 0, 0, 0, 0, 0],
            name_id: 0x0BAD,
            ..RuneRec::default()
        },
        RuneRec {
            complete: 1,
            itype: [T_WEAP as i16, 0, 0, 0, 0, 0],
            runes: [KEY as i32, KEY as i32, 0, 0, 0, 0],
            name_id: 0x1234,
            ..RuneRec::default()
        },
    ];
    let (s, u) = socketed(&mut w, 2, 2);
    assert_eq!(w.desk(|d| d.runeword_name(u)), 0x1234);
    assert_eq!(
        w.desk(|d| d.stream_item(s, 0, 0).map(|v| v.runeword)),
        Some(0x1234)
    );
    // Socket count ≠ filler count.
    w.set_stat(u, super::super::inv_world::STAT_SOCKETS, 3);
    assert_eq!(w.desk(|d| d.runeword_name(u)), 0xFFFF);
    // Quality 4 (magic).
    w.set_stat(u, super::super::inv_world::STAT_SOCKETS, 2);
    w.items.get_mut(u).unwrap().quality = q::MAGIC;
    assert_eq!(w.desk(|d| d.runeword_name(u)), 0xFFFF);
    // An empty inventory with a row of no runes: none.
    let (_, e) = socketed(&mut w, 0, 0);
    w.tables.runes[0].runes = [0; 6];
    assert_eq!(w.desk(|d| d.runeword_name(e)), 0xFFFF);
}

/// Item level 0 and quality 12 are written as 1 and 2, and the item store holds them after the
/// call. The overwrite branch skips the property lists (only the main
/// list's 0x1FF), so the next stream, of a quality-2 item, carries them
/// and differs; it is stable from then on.
#[test]
fn writer_changes_reach_the_item() {
    let mut w = World::new();
    let s = w.ground_item(CAP, 20, 20);
    let u = w.unit(s).unwrap();
    {
        let it = w.items.get_mut(u).unwrap();
        it.ilvl = 0;
        it.quality = 12;
    }
    let first = w.desk(|d| d.item_stream(s, 0, 0));
    assert!(!first.is_empty());
    let it = w.items.get(u).unwrap();
    assert_eq!((it.ilvl, it.quality), (0, 12), "queued, not yet written");
    w.desk(|_| ());
    let it = w.items.get(u).unwrap();
    assert_eq!((it.ilvl, it.quality), (1, q::NORMAL));
    let second = w.desk(|d| d.item_stream(s, 0, 0));
    assert_ne!(second, first);
    assert_eq!(w.desk(|d| d.item_stream(s, 0, 0)), second);
    assert!(w.state.write_backs.borrow().is_empty());
}

/// The store stream of an item: an unidentified quality 4–9 item with
/// the vendor flag (bit 2 of the unit flags) goes out as an alt-code record
/// (flags with 0x2000000, the head, the `normcode` and nothing more:
/// 109 bits for this ground item); identified, quality 2 or no vendor
/// flag → the full record.
#[test]
fn store_stream_of_an_unidentified_rare_vendor_item_is_alt_code() {
    let mut w = World::new();
    let s = w.ground_item(CAP, 20, 20);
    let u = w.unit(s).unwrap();
    {
        let it = w.items.get_mut(u).unwrap();
        it.quality = q::RARE;
        it.flags &= !crate::items::moves::iflag::IDENTIFIED;
    }
    w.units.get_mut(u).unwrap().flags2 |= crate::items::moves::deferred::VENDOR_ITEM;
    let alt = w.desk(|d| d.store_stream(s, 0));
    let flags = u32::from_le_bytes(alt[0..4].try_into().unwrap());
    assert_ne!(flags & 0x0200_0000, 0, "alt-code flag");
    // Flags 32, version 10, mode 3, ground x / y 16 + 16, base code 32.
    assert_eq!(alt.len(), 14, "109 bits: flags, head, base code");
    let bits = u128::from_le_bytes({
        let mut b = [0u8; 16];
        b[..14].copy_from_slice(&alt);
        b
    });
    assert_eq!(bits >> 109, 0, "nothing after the base code");
    let code = ((bits >> 77) & 0xFFFF_FFFF) as u32;
    let normcode = w.tables.items[CAP].normcode;
    let normcode = if normcode == [0; 4] {
        w.tables.items[CAP].code
    } else {
        normcode
    };
    assert_eq!(code.to_le_bytes(), normcode);
    let full = |w: &mut World| {
        let b = w.desk(|d| d.store_stream(s, 0));
        u32::from_le_bytes(b[0..4].try_into().unwrap()) & 0x0200_0000
    };
    // Identified: full record.
    w.items.get_mut(u).unwrap().flags |= crate::items::moves::iflag::IDENTIFIED;
    assert_eq!(full(&mut w), 0);
    // Quality 2: full record.
    {
        let it = w.items.get_mut(u).unwrap();
        it.flags &= !crate::items::moves::iflag::IDENTIFIED;
        it.quality = q::NORMAL;
    }
    assert_eq!(full(&mut w), 0);
    // No vendor flag: full record.
    w.items.get_mut(u).unwrap().quality = q::RARE;
    w.units.get_mut(u).unwrap().flags2 &= !crate::items::moves::deferred::VENDOR_ITEM;
    assert_eq!(full(&mut w), 0);
}
