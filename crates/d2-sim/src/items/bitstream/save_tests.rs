//! Synthetic save-format vectors, computed by hand from the writer's
//! rules (fields LSB first). No measured save covers them yet.
use super::tests::isc_114d;
use super::*;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// `hp1 ` (compact), mode 0, body 0, x 1, y 2, page 1, identified.
fn hp1() -> StreamItem {
    StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code: *b"hp1 ",
        x: 1,
        y: 2,
        page: 1,
        ..StreamItem::default()
    }
}

fn save(item: &StreamItem) -> Vec<u8> {
    write_save(item, &isc_114d()).unwrap().0
}

/// Bits: 16 0x4D4A | 32 F = 0x10 | 0x800000 | 0x200000 = 0xA00010 |
/// 10 version 101 | 3 mode 0 | 4 body 0 | 4 x 1 | 4 y 2 | 3 page+1 2 |
/// 32 `hp1 ` | 1 trailer 0 = 109 bits → 14 bytes.
#[test]
fn save_compact_marker_and_empty_trailer() {
    assert_eq!(save(&hp1()), hex("4a4d1000a0006500428406170302"));
}

/// As above, but trailer 1 bit 1, 32 bits 0x11223344, 32 bits
/// 0xAABBCCDD, 32 bits 0 = 205 bits → 26 bytes.
#[test]
fn save_compact_trailer_with_values() {
    let i = StreamItem {
        save_trailer: Some((0x1122_3344, 0xAABB_CCDD)),
        ..hp1()
    };
    assert_eq!(
        save(&i),
        hex("4a4d1000a0006500428406170392684624a29b79571500000000")
    );
}

/// `cap ` armor, identified, mode 0, body 0, x 3, y 1, page 0.
/// Bits: 16 0x4D4A | 32 F 0x800010 | 10 101 | 3 mode 0 | 4 body 0 |
/// 4 x 3 | 4 y 1 | 3 page+1 1 | 32 `cap ` | 3 filled 0 |
/// 32 unit+0x28 0xDEADBEEF | 7 ilvl 5 | 4 quality 2 | 1 varinvgfx 0 |
/// 1 auto 0 | 1 trailer 0 (after) | 11 defense 3 + 10 = 13 |
/// 8 max dur 12 | 9 dur 12 | 9 0x1FF = 194 bits → 25 bytes.
#[test]
fn save_full_armor_with_unit28() {
    let i = StreamItem {
        flags: 0x10,
        version: 101,
        x: 3,
        y: 1,
        page: 0,
        code: *b"cap ",
        unit28: 0xDEAD_BEEF,
        ilvl: 5,
        quality: 2,
        file_index: -1,
        runeword: 0xFFFF,
        kind: Kind {
            armor: true,
            ..Kind::default()
        },
        base_defense: 3,
        base_max_dur: 12,
        total_dur: 12,
        main: Some(Vec::new()),
        ..StreamItem::default()
    };
    assert_eq!(
        save(&i),
        hex("4a4d10008000650026321606078277df56ef82a0010c0cfe03")
    );
}

/// Not identified, socketed, magic `lax `: the save format keeps 0x800
/// and writes everything ("shown" = save format).
/// Bits: 16 0x4D4A | 32 F 0x800800 | 10 101 | 3 mode 0 | 4 body 0 |
/// 4 x 0 | 4 y 0 | 3 page+1 1 | 32 `lax ` | 3 filled 0 | 32 unit+0x28 0 |
/// 7 ilvl 10 | 4 quality 4 | 1 varinvgfx 0 | 1 auto 0 |
/// 11 prefix 750 − 747 = 3 | 11 suffix 20 | 1 trailer 0 | 8 max dur 30 |
/// 9 dur 25 | 4 sockets 2 | 9 stat 19 | 10 value 5 | 9 0x1FF
/// = 228 bits → 29 bytes.
#[test]
fn save_unidentified_socketed_keeps_everything() {
    let i = StreamItem {
        flags: 0x800,
        version: 101,
        page: 0,
        code: *b"lax ",
        ilvl: 10,
        quality: 4,
        prefix: [750, 0, 0],
        suffix: [20, 0, 0],
        runeword: 0xFFFF,
        kind: Kind {
            weapon: true,
            ..Kind::default()
        },
        base_max_dur: 30,
        total_dur: 25,
        base_sockets: 2,
        main: Some(vec![StatEntry {
            stat: 19,
            param: 0,
            value: 5,
        }]),
        ..StreamItem::default()
    };
    assert_eq!(
        save(&i),
        hex("4a4d00088000650000c216860702000000000531000af0c820130af80f")
    );
    // The network stream of the same item clears 0x800 and stops after
    // the type values.
    let (net, _) = write(&i, &isc_114d()).unwrap();
    assert_eq!(&net[..4], &0x0080_0000u32.to_le_bytes());
    // 129 bits: no marker, no unit+0x28, no affixes, no trailer, no list;
    // the 4-bit socket count is sent from the item's own flags (step 5
    // of the write).
    assert_eq!(net.len(), 17);
}

/// Parent `lax ` (identified, socketed, 1 socket filled) then its child
/// `gcv ` (compact, mode 6, x 0, y 0, page none).
/// Parent bits: 16 0x4D4A | 32 F 0x800810 | 10 101 | 3 mode 0 | 4 body 0
/// | 4 x 0 | 4 y 0 | 3 page+1 1 | 32 `lax ` | 3 filled 1 | 32 unit+0x28 0
/// | 7 ilvl 10 | 4 quality 2 | 1 varinvgfx 0 | 1 auto 0 | 1 trailer 0 |
/// 8 max dur 30 | 9 dur 30 | 4 sockets 1 | 9 0x1FF = 187 bits, padded to
/// 192 (24 bytes). Child bits: 16 0x4D4A | 32 F 0xA00010 | 10 101 |
/// 3 mode 6 | 4 body 0 | 4 x 0 | 4 y 0 | 3 page 0xFF + 1 = 0 | 32 `gcv ` |
/// 1 trailer 0 = 109 bits → 14 bytes. Total 38 bytes.
#[test]
fn save_parent_with_child_appended_after_padding() {
    let gem = StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        mode: 6,
        page: 0xFF,
        code: *b"gcv ",
        ..StreamItem::default()
    };
    let parent = StreamItem {
        flags: 0x810,
        version: 101,
        page: 0,
        code: *b"lax ",
        filled: 1,
        ilvl: 10,
        quality: 2,
        file_index: -1,
        runeword: 0xFFFF,
        kind: Kind {
            weapon: true,
            ..Kind::default()
        },
        base_max_dur: 30,
        total_dur: 30,
        base_sockets: 1,
        main: Some(Vec::new()),
        children: vec![gem.clone()],
        ..StreamItem::default()
    };
    let bytes = save(&parent);
    assert_eq!(
        bytes,
        hex("4a4d10088000650000c2168607120000000085c0c343fc074a4d1000a0006518007036660702")
    );
    assert_eq!(&bytes[24..], save(&gem).as_slice());
    // Children never travel on the wire.
    let (net, _) = write(&parent, &isc_114d()).unwrap();
    // 187 − 16 (marker) − 32 (unit+0x28) − 1 (trailer) = 138 bits.
    assert_eq!(net.len(), 18);
}

/// The save writer's capacity is the d2s file buffer, not 0xF4.
#[test]
fn save_capacity_is_file_buffer() {
    let parent = StreamItem {
        children: vec![hp1(); 20],
        ..hp1()
    };
    let (bytes, _) = write_save(&parent, &isc_114d()).unwrap();
    assert_eq!(bytes.len(), 21 * 14);
    assert!(bytes.len() > BUFFER);
}
