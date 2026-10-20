//! Tests for loading a save entry as an item of the owner's inventory
//! (`load.rs`): written with `save_view`, the source item taken out and
//! freed, the entry read back and placed where it was saved.

use super::*;
use crate::items::bitstream::{self, read, BitWriter, Isc};
use crate::items::{flag, stat};
use crate::wiring::inventory::load::LoadFault;

/// Save columns for the stats the tests write (as `copy.rs`).
fn isc() -> Vec<Isc> {
    let mut t = vec![Isc::default(); N_STATS];
    let bits = |b: u8, add: u32| Isc {
        valshift: 0,
        save_bits: b,
        save_add: add,
        save_param_bits: 0,
    };
    t[19] = bits(10, 0);
    t[usize::from(stat::ARMORCLASS)] = bits(11, 10);
    t[usize::from(stat::DURABILITY)] = bits(9, 0);
    t[usize::from(stat::MAXDURABILITY)] = bits(8, 0);
    t[usize::from(stat::NUMSOCKETS)] = bits(4, 0);
    t
}

/// The save entry of `u`, as the d2s writer makes it.
fn entry_of(w: &mut World, u: UnitId) -> read::ReadEntry {
    let bytes = w.desk(|d| {
        let v = d.save_view(u).expect("view");
        let mut bw = BitWriter::new(bitstream::SAVE_BUFFER);
        bitstream::write_save_into(&mut bw, &v, &d.econ.tables.isc);
        bw.finish().expect("fits")
    });
    read::read_save_entry(&bytes, &w.tables).expect("reads back")
}

fn world() -> World {
    let mut w = World::new();
    w.tables.isc = isc();
    w
}

/// Lets `record` take `n` sockets (synthetic `gemsockets` and the type's
/// `maxsock` columns; the socket setter clamps to them on read).
pub(super) fn allow_sockets(t: &mut crate::items::ItemTables, record: usize, n: u8) {
    t.items[record].gemsockets = n;
    let ty_ = t.items[record].type_ as usize;
    let it = &mut t.itemtypes[ty_];
    (it.maxsock1, it.maxsock25, it.maxsock40) = (n, n, n);
}

/// A stored item comes back on its page at its cell, in mode 0, with the
/// record's fields and the load's flags (0x80000 set, 0x2000 clear).
#[test]
fn a_stored_item_returns_to_its_cell() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    w.items.get_mut(u).unwrap().ilvl = 23;
    assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
    w.drain();
    let e = entry_of(&mut w, u);
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));
    assert!(w.state.items_of(p).is_empty());

    let n = w.desk(|d| d.load_entry(p, &e)).expect("loaded");
    let g = w.units.get(n).unwrap().guid;
    let d = w.data(g);
    assert_eq!((d.page, d.x, d.y), (0, 4, 2));
    assert_eq!(w.mode(g), 0);
    assert_eq!(w.state.items_of(p), [n]);
    assert_eq!(w.inventory().item_at(2, 4, 2), Some(n));
    let it = w.items.get(n).unwrap();
    assert_eq!((it.record, it.ilvl), (CAP, 23));
    assert_ne!(it.flags & flag::INIT, 0);
    assert_eq!(it.flags & flag::INSTORE, 0);
    assert!(w.state.errors.is_empty());
}

/// A cursor item (mode 4) goes back to the owner's cursor, whatever its
/// saved cell.
#[test]
fn a_cursor_item_returns_to_the_cursor() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    let e = entry_of(&mut w, u);
    w.desk(|d| d.free(u));
    let n = w.desk(|d| d.load_entry(p, &e)).expect("loaded");
    let g = w.units.get(n).unwrap().guid;
    assert_eq!(w.mode(g), 4);
    assert_eq!(w.state.cursor_of(p), Some(n));
    assert!(w.state.items_of(p).is_empty());
}

/// The saved cell is taken: the item is freed (exact cell, no
/// free-position fallback).
#[test]
fn a_taken_cell_frees_the_item() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u, (0, 0), false, true)));
    let e = entry_of(&mut w, u);
    // The source item stays at (0, 0): the copy cannot take its cell.
    assert_eq!(w.desk(|d| d.load_entry(p, &e)), Err(LoadFault::NoRoom));
    assert_eq!(w.state.items_of(p), [u]);
}

/// An owner without an inventory in the model loads nothing.
#[test]
fn no_inventory_no_item() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u, (0, 0), false, true)));
    let e = entry_of(&mut w, u);
    assert_eq!(
        w.desk(|d| d.load_entry(UnitId(9999), &e)),
        Err(LoadFault::NoInventory)
    );
}

/// An equipped item comes back at its body location, in mode 1.
#[test]
fn an_equipped_item_returns_to_its_body_location() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.handle(&body(0x1A, k, 1)), Ok(0));
    w.drain();
    let u = w.unit(k).unwrap();
    assert_eq!(w.state.body_items(p), [u]);
    let e = entry_of(&mut w, u);
    assert_eq!(e.item.item.mode, 1);
    assert_eq!(w.handle(&unequip(1)), Ok(0));
    w.drain();
    // The helm is on the cursor: put it down and free it.
    let u = w.unit(k).unwrap();
    assert!(w.desk(|d| d.place(p, u, (0, 0), true, false)));
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));

    let n = w.desk(|d| d.load_entry(p, &e)).expect("loaded");
    let g = w.units.get(n).unwrap().guid;
    assert_eq!(w.mode(g), 1);
    assert_eq!(w.state.body_items(p), [n]);
    assert_eq!(w.data(g).body_loc, 1);
}

/// A stored item with the runeword flag whose sockets match no runeword
/// row is freed on load.
#[test]
fn a_stale_runeword_item_is_freed() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    w.items.get_mut(u).unwrap().flags |= bitstream::hflag::RUNEWORD;
    assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
    w.drain();
    let e = entry_of(&mut w, u);
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));
    assert_eq!(
        w.desk(|d| d.load_entry(p, &e)),
        Err(LoadFault::StaleRuneword)
    );
    assert!(w.state.items_of(p).is_empty());
}

/// An equipped stale runeword item is equipped, then taken off detached: still
/// allocated, mode 4, off the body, not the cursor, not in the item list.
#[test]
fn an_equipped_stale_runeword_item_is_detached_not_freed() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    assert_eq!(w.handle(&body(0x1A, k, 1)), Ok(0));
    w.drain();
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().flags |= bitstream::hflag::RUNEWORD;
    let e = entry_of(&mut w, u);
    assert_eq!(e.item.item.mode, 1);
    assert_eq!(w.handle(&unequip(1)), Ok(0));
    w.drain();
    let u = w.unit(k).unwrap();
    assert!(w.desk(|d| d.place(p, u, (0, 0), true, false)));
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));
    let before = w.items.len();

    assert_eq!(
        w.desk(|d| d.load_entry(p, &e)),
        Err(LoadFault::StaleRuneword)
    );
    assert_eq!(w.items.len(), before + 1, "not freed");
    assert!(w.state.body_items(p).is_empty());
    assert_eq!(w.state.cursor_of(p), None);
    assert!(w.state.items_of(p).is_empty());
}

/// Without the runeword flag the same entry loads.
#[test]
fn an_item_without_the_runeword_flag_is_not_refreshed() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
    let e = entry_of(&mut w, u);
    // The saved cell must be free (placement is exact).
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));
    assert!(w.desk(|d| d.load_entry(p, &e)).is_ok());
}

/// A record that failed to read makes no item; the entry is skipped.
#[test]
fn a_failed_record_is_skipped() {
    let mut w = world();
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
    let mut e = entry_of(&mut w, u);
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));
    let before = w.items.len();
    e.item.failed = true;
    assert_eq!(
        w.desk(|d| d.load_entry(p, &e)),
        Err(LoadFault::RecordFailed)
    );
    assert_eq!(w.items.len(), before);
    assert!(w.state.items_of(p).is_empty());
}

/// The trailer's two values go to the item's `realm_data` and are
/// written back; the trailer bit is 1 exactly when the second is not 0.
#[test]
fn the_trailer_values_stay_on_the_item() {
    let mut w = world();
    let p = w.player;
    for (realm, back) in [([7, 9], Some((7, 9))), ([7, 0], None)] {
        let k = w.cursor_item(CAP);
        let u = w.unit(k).unwrap();
        w.items.get_mut(u).unwrap().inv_page = 0;
        w.items.get_mut(u).unwrap().realm_data = realm;
        assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
        let e = entry_of(&mut w, u);
        assert_eq!(e.item.item.save_trailer, back);
        assert!(w.desk(|d| d.remove(p, u)));
        w.desk(|d| d.free(u));
        let n = w.desk(|d| d.load_entry(p, &e)).expect("loaded");
        let want = back.map_or([0, 0], |(a, b)| [a, b]);
        assert_eq!(w.items.get(n).unwrap().realm_data, want);
        assert!(w.desk(|d| d.remove(p, n)));
        w.desk(|d| d.free(n));
    }
}

/// The socket count goes through the socket setter: at least 1, at most
/// min(w × h, 6) and max sockets.
#[test]
fn the_socket_count_is_clamped_on_read() {
    let mut w = world();
    allow_sockets(&mut w.tables, CAP, 3);
    let p = w.player;
    let k = w.cursor_item(CAP);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().inv_page = 0;
    w.items.get_mut(u).unwrap().flags |= flag::SOCKETED;
    assert!(w.desk(|d| d.place(p, u, (4, 2), false, true)));
    let e = entry_of(&mut w, u);
    assert!(w.desk(|d| d.remove(p, u)));
    w.desk(|d| d.free(u));
    let cap = {
        let r = w.tables.item(CAP).unwrap();
        (i32::from(r.invwidth) * i32::from(r.invheight)).min(6)
    };
    let max = crate::items::create::max_sockets_at(&w.tables, CAP, e.item.item.ilvl);
    assert!(cap.min(max) >= 2, "the fixture cap takes 2 or more sockets");
    for (v, want) in [(0, 1), (2, 2), (15, cap.min(max))] {
        let mut e = e.clone();
        e.item.item.base_sockets = v;
        let n = w.desk(|d| d.load_entry(p, &e)).expect("loaded");
        assert_eq!(w.stats.unit_base(n, stat::NUMSOCKETS, 0), want, "v {v}");
        assert!(w.desk(|d| d.remove(p, n)));
        w.desk(|d| d.free(n));
    }
}
