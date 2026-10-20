use super::*;

/// The base-game itemstatcost columns the vectors use (Constants; 22, 60 and
/// 75 are the widths under which B5, B6 and B8 end in their last byte).
pub(crate) fn isc_114d() -> Vec<Isc> {
    let mut t = vec![Isc::default(); 360];
    let mut set = |s: usize, valshift: u8, bits: u8, add: u32, param: u32| {
        t[s] = Isc {
            valshift,
            save_bits: bits,
            save_add: add,
            save_param_bits: param,
        }
    };
    set(9, 8, 8, 32, 0);
    set(17, 0, 9, 0, 0);
    set(18, 0, 9, 0, 0);
    set(19, 0, 10, 0, 0);
    set(22, 0, 7, 0, 0);
    set(31, 0, 11, 10, 0);
    set(48, 0, 8, 0, 0);
    set(49, 0, 9, 0, 0);
    set(60, 0, 7, 0, 0);
    set(72, 0, 9, 0, 0);
    set(73, 0, 8, 0, 0);
    set(75, 0, 7, 20, 0);
    set(107, 0, 3, 0, 9);
    set(194, 0, 4, 0, 0);
    set(356, 0, 2, 0, 0);
    t
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// The stream part of a measured message (from byte 8 for 0x9C, 13 for
/// 0x9D).
fn stream(msg: &str) -> Vec<u8> {
    let b = hex(msg);
    let from = if b[0] == 0x9C { 8 } else { 13 };
    b[from..].to_vec()
}

fn code(s: &[u8; 4]) -> [u8; 4] {
    *s
}

fn weapon() -> Kind {
    Kind {
        weapon: true,
        ..Kind::default()
    }
}

fn armor() -> Kind {
    Kind {
        armor: true,
        ..Kind::default()
    }
}

fn e(stat: u16, param: u16, value: i32) -> StatEntry {
    StatEntry { stat, param, value }
}

/// A full-record item in an inventory slot (version 101, identified
/// flags as measured).
fn full(flags: u32, mode: u32, slot: (u8, i32, i32, u8), c: &[u8; 4], ilvl: i32) -> StreamItem {
    StreamItem {
        flags,
        version: 101,
        mode,
        body_loc: slot.0,
        x: slot.1,
        y: slot.2,
        page: slot.3,
        code: code(c),
        ilvl,
        quality: 2,
        file_index: -1,
        runeword: 0xFFFF,
        main: Some(Vec::new()),
        ..StreamItem::default()
    }
}

fn check(item: &StreamItem, msg: &str, bits: usize) {
    let t = isc_114d();
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, item, &t);
    assert_eq!(w.bit_len(), bits, "bit length");
    assert_eq!(w.finish().unwrap(), stream(msg));
}

#[test]
fn b1_b2_compact_records() {
    let hp1 = StreamItem {
        flags: 0x20010,
        compact: true,
        version: 101,
        mode: 2,
        page: 0xFF,
        code: *b"hp1 ",
        ..StreamItem::default()
    };
    check(&hp1, "9c0e1410010000001000a2006508008006170302", 92);
    let isc = StreamItem {
        flags: 0x20010,
        compact: true,
        version: 101,
        mode: 0,
        x: 9,
        y: 2,
        page: 0,
        code: *b"isc ",
        ..StreamItem::default()
    };
    check(&isc, "9c041410060000001000a2006500529236370602", 92);
}

#[test]
fn b3_weapon_on_body() {
    let mut i = full(0x20011, 1, (4, 4, 0, 0xFF), b"hax ", 1);
    i.kind = weapon();
    i.base_max_dur = 28;
    i.total_dur = 28;
    check(
        &i,
        "9d061e0508000000000100000011008200658408801686078280c0c1e13f",
        134,
    );
}

#[test]
fn b4_armor_defense_with_save_add() {
    let mut i = full(0x20011, 1, (5, 5, 0, 0xFF), b"buc ", 1);
    i.kind = armor();
    i.base_defense = 5;
    i.base_max_dur = 12;
    i.total_dur = 12;
    check(
        &i,
        "9d0620060900000000010000001100820065a40a205637068280f0000606ff01",
        145,
    );
}

#[test]
fn b5_superior_socketed_with_list() {
    let mut i = full(0x2810, 0, (0, 0, 6, 1), b"lax ", 6);
    i.quality = 3;
    i.file_index = 5;
    i.kind = weapon();
    i.base_max_dur = 30;
    i.total_dur = 34;
    i.base_sockets = 3;
    i.main = Some(vec![e(19, 0, 1), e(75, 0, 14)]);
    check(
        &i,
        "9c0b1e050c000000102880006500c0c416860702c3500f1133218025a2ff",
        176,
    );
}

#[test]
fn b6_magic_with_fire_damage_group() {
    let mut i = full(0x2010, 0, (0, 3, 0, 1), b"scm ", 6);
    i.quality = 4;
    i.prefix[0] = 186 + PREFIX_OFFSET;
    i.suffix[0] = 183;
    i.kind = weapon();
    i.base_max_dur = 22;
    i.total_dur = 22;
    i.main = Some(vec![e(22, 0, 1), e(48, 0, 1), e(49, 0, 4)]);
    check(
        &i,
        "9c0b210511000000102080006500063437d6060203a18b5b5858b010801140e03f",
        198,
    );
}

#[test]
fn b7_stackable_quantity() {
    let mut i = full(0x2010, 0, (0, 0, 0, 3), b"tkf ", 6);
    i.kind = weapon();
    i.stackable = true;
    i.base_max_dur = 4;
    i.total_dur = 4;
    i.total_quantity = 160;
    check(
        &i,
        "9c0b1a05140000001020800065000048b766060283404000d47f",
        143,
    );
}

#[test]
fn b8_grouped_damage_percent_skips_equal_partner() {
    let mut i = full(0x2010, 0, (0, 8, 3, 1), b"sbw ", 6);
    i.quality = 4;
    i.prefix[0] = 187 + PREFIX_OFFSET;
    i.suffix[0] = 352;
    i.kind = weapon();
    i.base_max_dur = 20;
    i.total_dur = 19;
    i.main = Some(vec![e(17, 0, 29), e(18, 0, 29), e(60, 0, 4)]);
    check(
        &i,
        "9c0b21061d00000010208000650070342776070203b10bb0504c88d0a1030fc27f",
        199,
    );
}

#[test]
fn b9_valshift_and_save_add() {
    let mut i = full(0x2010, 0, (0, 0, 2, 0), b"cap ", 6);
    i.quality = 4;
    i.prefix[0] = 304 + PREFIX_OFFSET;
    i.kind = armor();
    i.base_defense = 3;
    i.base_max_dur = 12;
    i.total_dur = 12;
    i.main = Some(vec![e(9, 0, 5 << 8)]);
    check(
        &i,
        "9c0b1f002300000010208000650040321606070203011300348081418292ff",
        184,
    );
}

#[test]
fn b10_param_bits() {
    let mut i = full(0x20011, 1, (4, 4, 0, 0xFF), b"sst ", 1);
    i.kind = weapon();
    i.base_max_dur = 20;
    i.total_dur = 20;
    i.main = Some(vec![e(107, 36, 1)]);
    check(
        &i,
        "9d062105060000000001000000110082006584083037470782804041610d89fc07",
        155,
    );
}

/// Bits of a written stream as a string, bit 0 first.
fn bits_of(w: BitWriter) -> String {
    let n = w.bit_len();
    let b = w.finish().unwrap();
    (0..n)
        .map(|i| {
            if (b[i / 8] >> (i % 8)) & 1 != 0 {
                '1'
            } else {
                '0'
            }
        })
        .collect()
}

#[test]
fn synthetic_gold_and_clamp() {
    let mut w = BitWriter::new(BUFFER);
    put_gold(&mut w, 4095);
    assert_eq!(bits_of(w), format!("0{}", "1".repeat(12)));
    let mut w = BitWriter::new(BUFFER);
    put_gold(&mut w, 4096);
    let mut want = String::from("1");
    for i in 0..32 {
        want.push(if (0x1000u32 >> i) & 1 != 0 { '1' } else { '0' });
    }
    assert_eq!(bits_of(w), want);
    // A stat value −1 with Save Add 0 in 8 bits → 0xFF.
    assert_eq!(clamp(8, -1), 0xFF);
    assert_eq!(clamp(8, 300), 0xFF);
    assert_eq!(clamp(8, 7), 7);
    assert_eq!(clamp(32, -1), u32::MAX);
}

#[test]
fn overflow_writes_nothing_further() {
    let mut w = BitWriter::new(2);
    w.raw(12, 0xFFF);
    w.raw(8, 0xFF);
    assert!(w.overflowed());
    w.raw(1, 1);
    assert_eq!(w.bit_len(), 12);
    assert_eq!(w.finish(), Err(Overflow));
    // A list longer than the buffer fails the whole stream.
    let mut i = full(0x10, 0, (0, 0, 0, 0), b"lax ", 6);
    i.main = Some((0..300).map(|_| e(19, 0, 1)).collect());
    assert_eq!(write(&i, &isc_114d()), Err(Overflow));
}

#[test]
fn header_forced_and_cleared_bits() {
    let mut i = StreamItem {
        flags: 0x80000 | 0x800 | 0x400000,
        ..StreamItem::default()
    };
    // Not identified: 0x800 cleared; 0x80000 cleared; 0x800000 set.
    assert_eq!(header_flags(&i), 0x400000 | 0x800000);
    i.flags |= 0x10;
    assert_eq!(header_flags(&i), 0x400000 | 0x800000 | 0x800 | 0x10);
    i.alt = true;
    assert_eq!(header_flags(&i), 0x2000000 | 0x800000 | 0x800 | 0x10);
    i.compact = true;
    assert_ne!(header_flags(&i) & 0x200000, 0);
}

#[test]
fn alt_code_ends_after_base_code() {
    let mut i = full(0x10, 0, (0, 1, 2, 0), b"lax ", 6);
    i.alt = true;
    let t = isc_114d();
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    // 32 + 10 + 3 + 15 + 32
    assert_eq!(w.bit_len(), 92);
    let b = w.finish().unwrap();
    // base code 0 → items code; it sits at bits 60..92.
    let mut v = 0u64;
    for k in 0..32 {
        let at = 60 + k;
        v |= u64::from((b[at / 8] >> (at % 8)) & 1) << k;
    }
    assert_eq!(v as u32, u32::from_le_bytes(*b"lax "));
    i.base_code = *b"axe ";
    let (b2, _) = write(&i, &t).unwrap();
    assert_ne!(b, b2);
}

#[test]
fn quality_outside_range_is_rewritten_and_lists_skipped() {
    let t = isc_114d();
    let mut i = full(0x10, 0, (0, 0, 0, 0), b"lax ", 0);
    i.quality = 0;
    i.main = Some(vec![e(19, 0, 1)]);
    let (b, wb) = write(&i, &t).unwrap();
    assert_eq!(
        wb,
        WriteBack {
            ilvl: 1,
            quality: 2
        }
    );
    let mut j = i.clone();
    j.main = Some(Vec::new());
    assert_eq!(
        b,
        write(&j, &t).unwrap().0,
        "stats skipped, terminator kept"
    );
    // Quality 2 keeps its lists (B10).
    i.quality = 2;
    let (b2, wb2) = write(&i, &t).unwrap();
    assert_eq!(wb2.quality, 2);
    assert_ne!(b2, write(&j, &t).unwrap().0);
}

#[test]
fn rare_slots_are_sent_unidentified() {
    let t = isc_114d();
    let mut i = full(0, 0, (0, 0, 0, 0), b"lax ", 6);
    i.quality = 6;
    i.prefix = [PREFIX_OFFSET + 5, 0, 0];
    i.suffix = [0, 7, 0];
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    // head 32+10+3+15+32+3+7+4+1+1 = 108; slots: 1+11, 1, 1, 1+11, 1, 1.
    assert_eq!(w.bit_len(), 108 + 28);
    // Identified: 8 + 8 rare names first, then the lists' terminator.
    i.flags = 0x10;
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(w.bit_len(), 108 + 16 + 28 + 9);
    // Set / unique and tempered: shown only.
    for (q, n) in [(5u8, 12 + 5 + 9), (7, 12 + 9), (9, 16 + 9)] {
        i.quality = q;
        let mut w = BitWriter::new(BUFFER);
        write_into(&mut w, &i, &t);
        assert_eq!(w.bit_len(), 108 + n, "quality {q}");
    }
}

#[test]
fn set_mask_and_runeword_terminators() {
    let t = isc_114d();
    let mut i = full(0x10, 0, (0, 0, 0, 0), b"lax ", 6);
    i.quality = 5;
    i.sets[2] = Some(vec![e(19, 0, 1)]);
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    // head 108 + file index 12 + mask 5 + main 9 + list 2 (9+10+9).
    assert_eq!(w.bit_len(), 108 + 12 + 5 + 9 + 28);
    // Runeword: L = 3 + 1; every slot ends with a terminator.
    i.flags |= hflag::RUNEWORD;
    i.runeword = 0x1234;
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(w.bit_len(), 108 + 12 + 16 + 5 + 9 + 9 + 9 + 28 + 9);
}

#[test]
fn ear_personalized_and_quest_difficulty() {
    let t = isc_114d();
    let mut i = full(0x10 | hflag::PERSONALIZED, 0, (0, 0, 0, 0), b"lax ", 6);
    i.name[..3].copy_from_slice(b"abc");
    let base = {
        let mut j = i.clone();
        j.flags &= !hflag::PERSONALIZED;
        let mut w = BitWriter::new(BUFFER);
        write_into(&mut w, &j, &t);
        w.bit_len()
    };
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(w.bit_len(), base + 4 * 7);
    // Ear: class 3, level 7, name; replaces the personal name.
    i.flags |= hflag::EAR;
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(w.bit_len(), base + 10 + 4 * 7);
    // Compact quest item: ISC(356) after the code.
    let q = StreamItem {
        compact: true,
        quest_diff: true,
        total_quest_diff: 2,
        code: *b"bks ",
        ..StreamItem::default()
    };
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &q, &t);
    assert_eq!(w.bit_len(), 32 + 10 + 3 + 15 + 32 + 2);
}

#[test]
fn unidentified_record_ends_after_type_values() {
    let t = isc_114d();
    let mut i = full(0x800, 0, (0, 0, 0, 0), b"lax ", 6);
    i.kind = weapon();
    i.base_max_dur = 10;
    i.total_dur = 10;
    i.base_sockets = 2;
    i.main = Some(vec![e(19, 0, 1)]);
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    // Socketed cleared in F (not shown): no lists, but the 4-bit socket
    // count is sent from the item's own flags.
    assert_eq!(w.bit_len(), 108 + 8 + 9 + 4);
}

/// Edge case 5: stat 326 writes only its 9-bit id (no param, no value),
/// even with save bits and a param width set.
#[test]
fn stat_326_writes_only_its_id() {
    let mut t = isc_114d();
    t[326] = Isc {
        valshift: 0,
        save_bits: 8,
        save_add: 0,
        save_param_bits: 4,
    };
    let mut i = full(0x10, 0, (0, 0, 0, 0), b"lax ", 6);
    i.main = Some(Vec::new());
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    let base = w.bit_len();
    i.main = Some(vec![e(326, 3, 5)]);
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(w.bit_len(), base + 9);
}

/// Edge case 8: a compact ear carries no item code: class, level and
/// the name replace the 32 code bits.
#[test]
fn compact_ear_has_no_code() {
    let t = isc_114d();
    let mut i = StreamItem {
        compact: true,
        code: *b"ear ",
        ..StreamItem::default()
    };
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    let plain = w.bit_len();
    i.flags = hflag::EAR;
    i.ear_class = 2;
    i.ear_level = 30;
    i.name[..2].copy_from_slice(b"ab");
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(w.bit_len(), plain - 32 + 3 + 7 + 3 * 7);
}

/// Edge case 7: names are written up to their first 0; the setter keeps
/// at most 15 characters and rejects a longer name, so a stored name
/// always has its terminator inside the 16 bytes.
#[test]
fn name_setter_bounds_the_name() {
    use crate::items::{Item, NameTooLong};
    let mut it = Item::new(0, 101, ());
    assert_eq!(it.set_name(b"fifteen-chars-x"), Ok(()));
    assert_eq!(&it.name[..15], b"fifteen-chars-x");
    assert_eq!(it.name[15], 0);
    assert_eq!(it.set_name(b"sixteen-chars-xy"), Err(NameTooLong(16)));
    assert_eq!(&it.name[..15], b"fifteen-chars-x", "unchanged");
    assert_eq!(it.set_name(b"ab\0cd"), Ok(()));
    assert_eq!(&it.name[..4], b"ab\0\0");
    let mut i = StreamItem {
        flags: hflag::PERSONALIZED | 0x10,
        name: it.name,
        ..StreamItem::default()
    };
    let t = isc_114d();
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    let two = w.bit_len();
    i.name = [0; 16];
    let mut w = BitWriter::new(BUFFER);
    write_into(&mut w, &i, &t);
    assert_eq!(two, w.bit_len() + 2 * 7, "up to the first 0");
}
