//! Round trips: every save record the writer produces reads back into a
//! view that writes the same bytes (`read::read_save_entry`), with the
//! named fields restored; the decoder's flag rule (0x80000 and the
//! alt-code bit dropped) and its errors.

use super::read::{read_save_entry, BitReader, ReadError};
use super::tests::isc_114d;
use super::*;
use crate::items::tests::{item_rec, push_item, tables, AXE, RING};
use crate::items::{ty, ItemTables};

struct Codes {
    t: ItemTables,
}

fn codes() -> Codes {
    let mut t = tables();
    t.isc = isc_114d();
    for (ty_, c) in [
        (ty::GOLD, b"gld "),
        (ty::HELM, b"cap "),
        (AXE, b"axe "),
        (ty::CHAR, b"cm1 "),
        (ty::BOOK, b"tbk "),
        (ty::SCRO, b"tsc "),
        (RING, b"rin "),
        (ty::BODY, b"ear "),
        (ty::BODY, b"hrt "),
        (ty::MISC, b"hp1 "),
    ] {
        push_item(&mut t, item_rec(ty_, c));
    }
    let hp1 = t.items.iter().position(|r| &r.code == b"hp1 ").unwrap();
    t.items[hp1].stackable = 0;
    let rin = t.items.iter().position(|r| &r.code == b"rin ").unwrap();
    t.items[rin].stackable = 1;
    // A set record names a `setitems`
    // row (80 rows; the round trips use row 77).
    t.setitems.resize(80, Default::default());
    Codes { t }
}

fn kind(c: &Codes, code: &[u8; 4]) -> Kind {
    read::kind_of(&c.t, read::record_of(&c.t, *code).unwrap())
}

/// Writes `item`, reads it back, writes the read view again.
fn round_trip(c: &Codes, item: &StreamItem) -> StreamItem {
    let (bytes, _) = write_save(item, &c.t.isc).unwrap();
    let e = read_save_entry(&bytes, &c.t).unwrap();
    assert_eq!(e.len, bytes.len(), "the entry is the whole stream");
    let (again, _) = write_save(&e.item.item, &c.t.isc).unwrap();
    assert_eq!(again, bytes, "re-written bytes differ for {:?}", item.code);
    e.item.item
}

fn full(c: &Codes, code: &[u8; 4]) -> StreamItem {
    StreamItem {
        flags: 0x10,
        version: 101,
        code: *code,
        kind: kind(c, code),
        ilvl: 12,
        quality: 2,
        unit28: 0xDEAD_BEEF,
        runeword: 0xFFFF,
        main: Some(Vec::new()),
        ..StreamItem::default()
    }
}

fn e(stat: u16, param: u16, value: i32) -> StatEntry {
    StatEntry { stat, param, value }
}

/// Compact records: position, page + 1, gold, the trailer; the
/// ear's class, level and name replace the code (edge case 8).
#[test]
fn compact_records_read_back() {
    let c = codes();
    let hp1 = StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code: *b"hp1 ",
        x: 3,
        y: 1,
        page: 0xFF,
        save_trailer: Some((7, 9)),
        ..StreamItem::default()
    };
    let r = round_trip(&c, &hp1);
    assert_eq!(
        (r.x, r.y, r.page, r.save_trailer),
        (3, 1, 0xFF, Some((7, 9)))
    );
    let gold = StreamItem {
        code: *b"gld ",
        kind: kind(&c, b"gld "),
        total_gold: 5000,
        mode: 3,
        x: 600,
        y: 700,
        ..hp1.clone()
    };
    let r = round_trip(&c, &gold);
    assert_eq!((r.total_gold, r.mode, r.x, r.y), (5000, 3, 600, 700));
    let mut ear = StreamItem {
        flags: 0x10 | hflag::EAR,
        ear_class: 3,
        ear_level: 44,
        code: *b"ear ",
        ..hp1
    };
    ear.name[..4].copy_from_slice(b"Bomb");
    let r = round_trip(&c, &ear);
    assert_eq!(
        (r.ear_class, r.ear_level, &r.name[..5]),
        (3, 44, &b"Bomb\0"[..])
    );
}

/// Full records: head, by-quality fields with the prefix and
/// automagic offsets undone, type values, sockets, the lists with
/// `ValShift` and the grouped partners, set lists by the mask, the
/// runeword list; all re-written byte for byte.
#[test]
fn full_records_read_back() {
    let c = codes();
    // Armor: defense, durability, sockets, a list with ValShift 8.
    let mut cap = full(&c, b"cap ");
    cap.flags |= hflag::SOCKETED;
    cap.base_defense = 7;
    cap.base_max_dur = 12;
    cap.total_dur = 9;
    cap.base_sockets = 2;
    cap.filled = 0;
    cap.main = Some(vec![e(9, 0, 5 << 8), e(19, 0, 3), e(107, 36, 2)]);
    let r = round_trip(&c, &cap);
    assert_eq!(
        (r.base_defense, r.base_max_dur, r.total_dur, r.base_sockets),
        (7, 12, 9, 2)
    );
    assert_eq!(r.main, cap.main);
    assert_eq!(r.unit28, 0xDEAD_BEEF);
    // Magic weapon with an automagic affix and a grouped stat (17 → 18).
    let mut axe = full(&c, b"axe ");
    axe.quality = 4;
    axe.prefix[0] = PREFIX_OFFSET + 9;
    axe.suffix[0] = 33;
    axe.auto_affix = AUTO_OFFSET + 2;
    axe.varinvgfx = true;
    axe.gfx = 5;
    axe.main = Some(vec![e(17, 0, 40), e(18, 0, 40)]);
    let r = round_trip(&c, &axe);
    assert_eq!(
        (r.prefix[0], r.suffix[0], r.auto_affix),
        (axe.prefix[0], 33, axe.auto_affix)
    );
    assert_eq!((r.varinvgfx, r.gfx), (true, 5));
    // The partner equal to the recorded value is not written twice.
    assert_eq!(r.main, Some(vec![e(17, 0, 40), e(18, 0, 40)]));
    // Rare with slots, then crafted.
    let mut rin = full(&c, b"rin ");
    rin.quality = 6;
    rin.rare_prefix = 12;
    rin.rare_suffix = 40;
    rin.prefix = [PREFIX_OFFSET + 1, 0, PREFIX_OFFSET + 3];
    rin.suffix = [0, 8, 0];
    rin.total_quantity = 0;
    rin.stackable = true;
    let r = round_trip(&c, &rin);
    assert_eq!((r.prefix, r.suffix), (rin.prefix, rin.suffix));
    rin.quality = 8;
    round_trip(&c, &rin);
    // Set with lists 0 and 2; unique with a negative file index.
    let mut set = full(&c, b"cap ");
    set.quality = 5;
    set.file_index = 77;
    set.sets[0] = Some(vec![e(19, 0, 1)]);
    set.sets[2] = Some(vec![e(19, 0, 2)]);
    let r = round_trip(&c, &set);
    assert_eq!((r.file_index, &r.sets), (77, &set.sets));
    set.quality = 7;
    set.file_index = -1;
    set.sets = Default::default();
    assert_eq!(round_trip(&c, &set).file_index, -1);
    // Runeword: name id and its own list; personalized name.
    let mut rw = full(&c, b"axe ");
    rw.flags |= hflag::RUNEWORD | hflag::PERSONALIZED;
    rw.runeword = 0x1234;
    rw.runeword_list = Some(vec![e(19, 0, 4)]);
    rw.name[..2].copy_from_slice(b"Al");
    let r = round_trip(&c, &rw);
    assert_eq!((r.runeword, &r.runeword_list), (0x1234, &rw.runeword_list));
    assert_eq!(&r.name[..3], b"Al\0");
    // Other qualities: charm prefix / suffix, body part, scroll spell.
    let mut cm = full(&c, b"cm1 ");
    cm.prefix[0] = PREFIX_OFFSET + 4;
    assert_eq!(round_trip(&c, &cm).prefix[0], PREFIX_OFFSET + 4);
    cm.prefix[0] = 0;
    cm.suffix[0] = 6;
    assert_eq!(round_trip(&c, &cm).suffix[0], 6);
    let mut hrt = full(&c, b"hrt ");
    hrt.file_index = 300;
    assert_eq!(round_trip(&c, &hrt).file_index, 300);
    let mut tsc = full(&c, b"tsc ");
    tsc.suffix[0] = 17;
    assert_eq!(round_trip(&c, &tsc).suffix[0], 17);
}

/// and 8: a full record's filled count of children
/// follows it, each its own padded entry; the decoder keeps the stored
/// flags except 0x80000 and the alt-code bit.
#[test]
fn children_follow_their_parent() {
    let c = codes();
    let mut cap = full(&c, b"cap ");
    cap.flags |= hflag::SOCKETED | hflag::INIT;
    cap.base_sockets = 2;
    cap.filled = 2;
    let child = StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code: *b"hp1 ",
        mode: 6,
        ..StreamItem::default()
    };
    cap.children = vec![child.clone(), child];
    let (bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    let got = read_save_entry(&bytes, &c.t).unwrap();
    assert_eq!(got.len, bytes.len());
    assert_eq!(got.children.len(), 2);
    assert_eq!(got.children[0].item.item.mode, 6);
    assert_eq!(got.item.item.flags & hflag::INIT, 0, "0x80000 dropped");
    assert_ne!(got.item.item.flags & hflag::FORCED, 0, "kept as stored");
}

/// Errors: a missing marker, an unknown code, a short record, a set
/// padding bit.
#[test]
fn reader_errors() {
    let c = codes();
    let hp1 = StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code: *b"hp1 ",
        ..StreamItem::default()
    };
    let (mut bytes, _) = write_save(&hp1, &c.t.isc).unwrap();
    assert_eq!(
        read::read_save_record(&mut BitReader::new(&bytes[..4]), &c.t),
        Err(ReadError::Short(16)),
        "the flags do not fit after the marker"
    );
    let last = bytes.len() - 1;
    bytes[last] |= 0x80;
    assert!(matches!(
        read_save_entry(&bytes, &c.t),
        Err(ReadError::Padding(_))
    ));
    bytes[0] = 0;
    assert!(matches!(
        read_save_entry(&bytes, &c.t),
        Err(ReadError::BadMarker(_))
    ));
    let unknown = StreamItem {
        code: *b"zzz ",
        ..hp1
    };
    let (bytes, _) = write_save(&unknown, &c.t.isc).unwrap();
    assert_eq!(
        read_save_entry(&bytes, &c.t),
        Err(ReadError::UnknownCode(*b"zzz "))
    );
}

/// An alt-code record ends after its base code; the reader
/// gives the item level 1 and quality 1 whatever the writer's item held
/// (edge case 9).
#[test]
fn an_alt_code_record_reads_level_1_quality_1() {
    let c = codes();
    let mut cap = full(&c, b"cap ");
    cap.alt = true;
    cap.ilvl = 40;
    cap.quality = 4;
    let (bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    let r = read_save_entry(&bytes, &c.t).unwrap().item.item;
    assert!(r.alt);
    assert_eq!((r.code, r.ilvl, r.quality), (*b"cap ", 1, 1));
}

/// The decoder's rebuilt fields: a
/// compact record gets item level 1, quality 2, seed field 0 and the
/// suffix slot 0 of its scroll / tome code; a full record's level below
/// 1 reads as 1 and a unique's index at or above the `uniqueitems` count
/// as −1.
#[test]
fn decoder_rebuilds_level_quality_and_unique_index() {
    let mut c = codes();
    push_item(&mut c.t, item_rec(ty::BOOK, b"isc "));
    let compact = |code: &[u8; 4]| StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code: *code,
        unit28: 0x1234,
        ..StreamItem::default()
    };
    let (bytes, _) = write_save(&compact(b"tsc "), &c.t.isc).unwrap();
    let r = read_save_entry(&bytes, &c.t).unwrap().item.item;
    assert_eq!((r.ilvl, r.quality, r.unit28, r.suffix[0]), (1, 2, 0, 0));
    let (bytes, _) = write_save(&compact(b"isc "), &c.t.isc).unwrap();
    assert_eq!(
        read_save_entry(&bytes, &c.t).unwrap().item.item.suffix[0],
        1
    );
    let (bytes, _) = write_save(&compact(b"hp1 "), &c.t.isc).unwrap();
    let r = read_save_entry(&bytes, &c.t).unwrap().item.item;
    assert_eq!((r.ilvl, r.quality, r.suffix[0]), (1, 2, 0));
    // Full record: level 0 reads as 1.
    let mut cap = full(&c, b"cap ");
    cap.ilvl = 0;
    let (bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    assert_eq!(read_save_entry(&bytes, &c.t).unwrap().item.item.ilvl, 1);
    // Unique: index below the count is kept, at the count it is −1.
    let n = c.t.uniques.len() as i32;
    let mut u = full(&c, b"cap ");
    u.quality = 7;
    for (idx, want) in [(n - 1, n - 1), (n, -1), (n + 40, -1)] {
        if idx < 0 {
            continue;
        }
        u.file_index = idx;
        let (bytes, _) = write_save(&u, &c.t.isc).unwrap();
        assert_eq!(
            read_save_entry(&bytes, &c.t).unwrap().item.item.file_index,
            want
        );
    }
}

/// The bit position just past the record read from `bytes`.
fn end_of(c: &Codes, bytes: &[u8]) -> usize {
    let mut r = BitReader::new(bytes);
    read::read_save_record(&mut r, &c.t).unwrap();
    r.pos()
}

/// (current version, > 0x5D): a and b are kept, the third u32
/// is read and dropped whatever it holds.
#[test]
fn the_third_trailer_word_is_read_and_dropped() {
    let c = codes();
    let mut cap = full(&c, b"cap ");
    cap.save_trailer = Some((7, 9));
    let (mut bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    let end = end_of(&c, &bytes);
    // The third word is the record's last 32 bits: make it 0x80000001.
    for at in [end - 32, end - 1] {
        bytes[at / 8] |= 1 << (at % 8);
    }
    let e = read_save_entry(&bytes, &c.t).unwrap();
    assert_eq!(e.len, bytes.len());
    assert!(!e.item.failed);
    assert_eq!(e.item.item.save_trailer, Some((7, 9)));
}

/// Edge case 6: a quality outside 1–9
/// reads nothing in the quality step, is read on to its end and fails;
/// the bytes used are the whole record.
#[test]
fn a_quality_outside_1_to_9_reads_on_and_fails() {
    let c = codes();
    let cap = full(&c, b"cap ");
    let (bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    // Marker 16, flags 32, format 10, mode 3, location 15, code 32,
    // filled 3, unit +0x28 32, item level 7: quality at bit 150.
    const Q: usize = 150;
    for q in [0u8, 10, 15] {
        let mut b = bytes.clone();
        for i in 0..4 {
            let at = Q + i;
            b[at / 8] &= !(1 << (at % 8));
            b[at / 8] |= ((q >> i) & 1) << (at % 8);
        }
        let e = read_save_entry(&b, &c.t).unwrap();
        assert_eq!((e.item.item.quality, e.item.failed), (q, true), "q {q}");
        assert_eq!(e.len, bytes.len(), "read on to the end");
    }
    assert!(!read_save_entry(&bytes, &c.t).unwrap().item.failed);
}

/// (v ≥ 0x5D): the 12 bits name a
/// `setitems` row; no row fails the record.
#[test]
fn a_set_index_without_a_row_fails() {
    let c = codes();
    let mut set = full(&c, b"cap ");
    set.quality = 5;
    set.file_index = 79;
    let (bytes, _) = write_save(&set, &c.t.isc).unwrap();
    let e = read_save_entry(&bytes, &c.t).unwrap();
    assert_eq!((e.item.failed, e.item.item.file_index), (false, 79));
    set.file_index = 80;
    let (bytes, _) = write_save(&set, &c.t.isc).unwrap();
    assert!(read_save_entry(&bytes, &c.t).unwrap().item.failed);
}

/// Edge cases 1 and 2: id 0 alone is
/// strength; id 0 after id 0 fails the record; an id with no
/// `itemstatcost` row ends the list without a failure.
#[test]
fn list_ids_0_0_fail_and_an_id_without_a_row_ends_the_list() {
    let mut c = codes();
    // Synthetic columns for stat 0 (those of row 19).
    c.t.isc[0] = c.t.isc[19];
    let mut cap = full(&c, b"cap ");
    cap.main = Some(vec![e(0, 0, 5), e(19, 0, 1)]);
    let (bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    let r = read_save_entry(&bytes, &c.t).unwrap();
    assert!(!r.item.failed);
    assert_eq!(r.item.item.main, cap.main);
    cap.main = Some(vec![e(0, 0, 5), e(0, 0, 6)]);
    let (bytes, _) = write_save(&cap, &c.t.isc).unwrap();
    let r = read_save_entry(&bytes, &c.t).unwrap();
    assert!(r.item.failed);
    assert_eq!(r.item.item.main, Some(vec![e(0, 0, 5)]));
    // Id 380 written with a table that has its row, read with one that
    // ends at 360: the list ends at the id (its value bits are then left
    // unread).
    let mut wide = c.t.isc.clone();
    wide.resize(381, Default::default());
    wide[380] = c.t.isc[19];
    cap.main = Some(vec![e(380, 0, 3)]);
    let (bytes, _) = write_save(&cap, &wide).unwrap();
    let mut rd = BitReader::new(&bytes);
    let r = read::read_save_record(&mut rd, &c.t).unwrap();
    assert!(!r.failed);
    assert_eq!(r.item.main, Some(Vec::new()));
}

/// Outputs: the save writer returns every
/// written item's write-back, children too, in write order.
#[test]
fn the_save_writer_returns_every_items_write_back() {
    let c = codes();
    let mut cap = full(&c, b"cap ");
    cap.ilvl = 0;
    cap.filled = 2;
    let mut a = full(&c, b"rin ");
    a.quality = 0;
    let mut b = full(&c, b"rin ");
    b.quality = 12;
    b.ilvl = 5;
    let mut inner = full(&c, b"hp1 ");
    inner.quality = 3;
    a.children = vec![inner];
    cap.children = vec![a, b];
    let (_, wbs) = write_save(&cap, &c.t.isc).unwrap();
    let got: Vec<(i32, u8)> = wbs.iter().map(|w| (w.ilvl, w.quality)).collect();
    assert_eq!(got, [(1, 2), (12, 2), (12, 3), (5, 2)]);
}
