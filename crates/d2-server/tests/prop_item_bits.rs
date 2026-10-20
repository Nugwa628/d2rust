//! Round trip of the item bit stream: the server's writer
//! (`d2_sim::items::bitstream::write`) and the client's reader
//! (`d2_proto::item_bits::decode`) agree on every field, for random items
//! of every record shape (compact, alt-code, full; every quality; ear,
//! personalized, runeword, socketed, set lists; identified or not) and
//! random values, including values outside their field widths (clamps).
//! The two sides are written separately, so a misread rule on either side
//! shows as a mismatch. Both ends are D2RUST code: this test is what keeps
//! the server's item records and the client's item parser in step.
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder.

use d2_proto::item_bits::{
    decode, CodeFacts, IscSave, ItemBits, ItemBitsError, ItemLookup, Location,
};
use d2_sim::items::bitstream::{
    self, clamp, header_flags, prefix_id, write, Isc, Kind, StatEntry, StreamItem, AUTO_OFFSET,
    BUFFER,
};
use proptest::prelude::*;

/// Itemstatcost save columns (`Save Bits`, `Save Add`, `Save Param
/// Bits`, `ValShift`) of the stats the items draw (the widths the game files use).
const ISC: [(u16, u8, u32, u32, u8); 16] = [
    (9, 8, 32, 0, 8),
    (17, 9, 0, 0, 0),
    (18, 9, 0, 0, 0),
    (19, 10, 0, 0, 0),
    (22, 7, 0, 0, 0),
    (31, 11, 10, 0, 0),
    (48, 8, 0, 0, 0),
    (49, 9, 0, 0, 0),
    (60, 7, 0, 0, 0),
    (72, 9, 0, 0, 0),
    (73, 8, 0, 0, 0),
    (75, 7, 20, 0, 0),
    (107, 3, 0, 9, 0),
    (194, 4, 0, 0, 0),
    (326, 0, 0, 0, 0),
    (356, 2, 0, 0, 0),
];

/// List stats the items draw (not grouped: the groups are covered by the
/// vectors and `grouped_partners_round_trip`).
const LIST_STATS: [u16; 6] = [9, 19, 22, 60, 75, 107];

fn isc_table() -> Vec<Isc> {
    let mut t = vec![Isc::default(); 360];
    for (s, bits, add, param, shift) in ISC {
        t[usize::from(s)] = Isc {
            valshift: shift,
            save_bits: bits,
            save_add: add,
            save_param_bits: param,
        };
    }
    t
}

/// Codes and their facts: (code, kind, stackable, varinvgfx, quest_diff).
const CODES: [([u8; 4], Kind, bool, bool, bool); 8] = [
    (
        *b"lax ",
        k(false, true, false, false, false, false),
        false,
        false,
        false,
    ),
    (
        *b"cap ",
        k(true, false, false, false, false, false),
        false,
        true,
        false,
    ),
    (
        *b"tkf ",
        k(false, true, false, false, false, false),
        true,
        false,
        false,
    ),
    (
        *b"gld ",
        k(false, false, true, false, false, false),
        false,
        false,
        false,
    ),
    (
        *b"cm1 ",
        k(false, false, false, true, false, false),
        false,
        true,
        false,
    ),
    (
        *b"ear ",
        k(false, false, false, false, true, false),
        false,
        false,
        false,
    ),
    (
        *b"tbk ",
        k(false, false, false, false, false, true),
        true,
        false,
        false,
    ),
    (
        *b"bks ",
        k(false, false, false, false, false, false),
        false,
        false,
        true,
    ),
];

const fn k(armor: bool, weapon: bool, gold: bool, charm: bool, body: bool, book: bool) -> Kind {
    Kind {
        armor,
        weapon,
        gold,
        charm,
        body_part: body,
        scroll_or_book: book,
    }
}

struct Lookup;

impl ItemLookup for Lookup {
    fn code(&self, code: [u8; 4]) -> Option<CodeFacts> {
        let (_, kind, stackable, varinvgfx, quest_diff) =
            CODES.iter().find(|c| c.0 == code).copied()?;
        Some(CodeFacts {
            armor: kind.armor,
            weapon: kind.weapon,
            gold: kind.gold,
            charm: kind.charm,
            body_part: kind.body_part,
            scroll_or_book: kind.scroll_or_book,
            stackable,
            varinvgfx,
            quest_diff,
        })
    }
    fn isc(&self, stat: u16) -> Option<IscSave> {
        let c = isc_table().get(usize::from(stat)).copied()?;
        Some(IscSave {
            save_bits: c.save_bits,
            save_add: c.save_add,
            save_param_bits: c.save_param_bits,
        })
    }
}

fn entries() -> impl Strategy<Value = Vec<StatEntry>> {
    prop::collection::vec(
        (
            prop::sample::select(LIST_STATS.to_vec()),
            0u16..600,
            -40i32..3000,
        )
            .prop_map(|(stat, param, value)| StatEntry { stat, param, value }),
        0..5,
    )
}

fn name() -> impl Strategy<Value = [u8; 16]> {
    (prop::collection::vec(1u8..128, 0..15)).prop_map(|v| {
        let mut n = [0u8; 16];
        n[..v.len()].copy_from_slice(&v);
        n
    })
}

prop_compose! {
    fn item()(
        code in 0..CODES.len(),
        flag_bits in prop::collection::vec(any::<bool>(), 6),
        compact in any::<bool>(),
        alt in prop::bool::weighted(0.1),
        mode in 0u32..8,
        x in -5i32..70000,
        y in -5i32..70000,
        body_loc in 0u8..16,
        page in prop::sample::select(vec![0u8, 1, 3, 4, 7, 0xFF]),
        filled in 0u32..10,
        ilvl in -3i32..130,
        quality in 0u8..12,
        file_index in -2i32..5000,
        gfx in 0i32..9,
        auto_affix in prop::sample::select(vec![0u16, 5, AUTO_OFFSET, AUTO_OFFSET + 30]),
        prefix in prop::array::uniform3(prop::sample::select(vec![0u16, 3, 747, 748, 1200])),
        suffix in prop::array::uniform3(0u16..2100),
        rare in (0u16..300, 0u16..300),
        runeword in any::<u16>(),
        ear in (-1i32..9, 0i32..140),
        name in name(),
        nums in prop::array::uniform8(-20i32..0x2_0000),
        main in prop::option::weighted(0.9, entries()),
        sets in prop::array::uniform5(prop::option::weighted(0.3, entries())),
        rw_list in prop::option::of(entries()),
    ) -> StreamItem {
        let mut flags = 0;
        for (b, f) in flag_bits.iter().zip([0x10u32, 0x800, 0x10000, 0x1000000, 0x4000000, 0x400000]) {
            if *b { flags |= f; }
        }
        // An ear is its own record (a compact ear carries no code, so the
        // reader cannot test the quest-item rule on it; no ear record is a
        // quest item).
        let code = if flags & 0x10000 != 0 { 5 } else { code };
        let (c, kind, stackable, varinvgfx, quest_diff) = CODES[code];
        StreamItem {
            flags,
            alt,
            compact,
            version: 101,
            mode,
            x,
            y,
            body_loc,
            page,
            code: c,
            base_code: if alt && x % 2 == 0 { *b"zzz " } else { [0; 4] },
            ear_class: ear.0,
            ear_level: ear.1,
            name,
            filled,
            ilvl,
            quality,
            file_index,
            varinvgfx,
            gfx,
            auto_affix,
            prefix,
            suffix,
            rare_prefix: rare.0,
            rare_suffix: rare.1,
            runeword,
            kind,
            stackable,
            quest_diff,
            base_defense: nums[0],
            base_max_dur: nums[1] % 300,
            base_sockets: nums[2] % 20,
            total_dur: nums[3],
            total_gold: nums[4],
            total_quantity: nums[5],
            total_quest_diff: nums[6] % 4,
            main,
            sets,
            runeword_list: rw_list,
            ..Default::default()
        }
    }
}

fn isc_raw(s: u16, v: i32) -> u32 {
    let c = isc_table()[usize::from(s)];
    clamp(u32::from(c.save_bits), v.wrapping_add(c.save_add as i32))
}

/// The list the reader should see for `l` (stat lists, no groups).
fn expected_list(l: &[StatEntry]) -> Vec<(u16, u32, u32)> {
    let t = isc_table();
    l.iter()
        .filter(|e| {
            let c = t[usize::from(e.stat)];
            c.save_bits != 0 && e.value >> c.valshift != 0
        })
        .map(|e| {
            let c = t[usize::from(e.stat)];
            let p = if c.save_param_bits > 0 {
                clamp(c.save_param_bits, i32::from(e.param))
            } else {
                0
            };
            (e.stat, p, isc_raw(e.stat, e.value >> c.valshift))
        })
        .collect()
}

fn check(item: &StreamItem, d: &ItemBits) -> Result<(), TestCaseError> {
    let f = header_flags(item);
    prop_assert_eq!(d.flags, f);
    prop_assert_eq!(d.version, item.version);
    prop_assert_eq!(u32::from(d.mode), clamp(3, item.mode as i32));
    let want = if item.mode == 3 || item.mode == 5 {
        Location::Ground {
            x: clamp(16, item.x) as u16,
            y: clamp(16, item.y) as u16,
        }
    } else {
        Location::Slot {
            body: clamp(4, i32::from(item.body_loc)) as u8,
            x: clamp(4, item.x) as u8,
            y: clamp(4, item.y) as u8,
            page1: clamp(3, i32::from(item.page.wrapping_add(1))) as u8,
        }
    };
    prop_assert_eq!(d.location, Some(want));
    let name_of = |n: &[u8; 16]| {
        n.iter()
            .take_while(|&&c| c != 0)
            .copied()
            .collect::<Vec<u8>>()
    };
    if f & 0x200000 != 0 {
        if f & 0x10000 != 0 {
            let ear = d.ear.as_ref().unwrap();
            prop_assert_eq!(u32::from(ear.class), clamp(3, item.ear_class));
            prop_assert_eq!(u32::from(ear.level), clamp(7, item.ear_level));
            prop_assert_eq!(&ear.name, &name_of(&item.name));
        } else {
            prop_assert_eq!(d.code, item.code);
            if item.kind.gold {
                let g = item.total_gold;
                let want = if g >= 0x1000 { g as u32 } else { clamp(12, g) };
                prop_assert_eq!(d.gold, Some(want));
            }
            if item.quest_diff {
                prop_assert_eq!(
                    d.quest_diff.unwrap().raw,
                    isc_raw(356, item.total_quest_diff)
                );
            }
        }
        return Ok(());
    }
    if f & 0x2000000 != 0 {
        let base = if item.base_code == [0; 4] {
            item.code
        } else {
            item.base_code
        };
        prop_assert_eq!(d.base_code, Some(base));
        return Ok(());
    }
    prop_assert_eq!(d.code, item.code);
    prop_assert_eq!(u32::from(d.filled), clamp(3, item.filled as i32));
    prop_assert_eq!(i32::from(d.ilvl), item.ilvl.clamp(1, 99));
    prop_assert_eq!(d.quality, item.quality & 0xF);
    prop_assert_eq!(d.gfx.is_some(), item.varinvgfx);
    if item.varinvgfx {
        prop_assert_eq!(u32::from(d.gfx.unwrap()), clamp(3, item.gfx));
    }
    let a = if item.auto_affix > AUTO_OFFSET {
        item.auto_affix - AUTO_OFFSET
    } else {
        item.auto_affix
    };
    prop_assert_eq!(d.auto_affix, (a != 0).then_some(a));
    let shown = f & 0x10 != 0;
    let q = &d.quality_fields;
    match item.quality {
        1 | 3 => prop_assert_eq!(q.file_index, Some(clamp(3, item.file_index))),
        4 if shown => prop_assert_eq!(
            q.magic,
            Some((prefix_id(item.prefix[0]), item.suffix[0].min(0x7FF)))
        ),
        5 | 7 if shown => prop_assert_eq!(q.file_index, Some(clamp(12, item.file_index))),
        6 | 8 => {
            let slots =
                std::array::from_fn(|i| (prefix_id(item.prefix[i]), item.suffix[i].min(0x7FF)));
            prop_assert_eq!(q.rare_slots, Some(slots));
        }
        _ => {}
    }
    if f & 0x4000000 != 0 {
        prop_assert_eq!(d.runeword, Some(item.runeword));
    }
    if f & 0x10000 == 0 && f & 0x1000000 != 0 {
        prop_assert_eq!(d.name.as_ref(), Some(&name_of(&item.name)));
    }
    if item.kind.armor {
        prop_assert_eq!(d.defense.unwrap().raw, isc_raw(31, item.base_defense));
    }
    if item.kind.armor || item.kind.weapon {
        prop_assert_eq!(
            d.max_durability.unwrap().raw,
            isc_raw(73, item.base_max_dur)
        );
        prop_assert_eq!(
            d.durability.map(|s| s.raw),
            (item.base_max_dur != 0).then(|| isc_raw(72, item.total_dur))
        );
    }
    if item.stackable {
        prop_assert_eq!(
            d.quantity.map(u32::from),
            Some(clamp(9, item.total_quantity))
        );
    }
    if f & 0x800 != 0 {
        prop_assert_eq!(d.sockets.map(u32::from), Some(clamp(4, item.base_sockets)));
    }
    if !shown {
        prop_assert!(d.lists.is_empty());
        return Ok(());
    }
    // A quality outside 1–9 other than 2 writes no stats.
    let skip = !(1..=9).contains(&item.quality) && item.quality != 2;
    let got = |l: &Option<Vec<d2_proto::item_bits::Stat>>| {
        l.as_ref().map(|l| {
            l.iter()
                .map(|s| (s.stat, s.param, s.raw))
                .collect::<Vec<_>>()
        })
    };
    let want_list = |l: &Option<Vec<StatEntry>>| -> Vec<(u16, u32, u32)> {
        match l {
            Some(l) if !skip => expected_list(l),
            _ => Vec::new(),
        }
    };
    prop_assert_eq!(got(&d.lists[0]), Some(want_list(&item.main)));
    let runeword = f & 0x4000000 != 0;
    let mut l = 0;
    if item.quality == 5 {
        l = item
            .sets
            .iter()
            .rposition(|s| s.is_some())
            .map_or(0, |i| i + 1);
    }
    if runeword {
        l += 1;
    }
    prop_assert_eq!(d.lists.len(), l + 1);
    for c in 0..l {
        let src = if runeword && c == l - 1 {
            &item.runeword_list
        } else {
            &item.sets[c]
        };
        let present = src.is_some() || runeword;
        prop_assert_eq!(d.lists[c + 1].is_some(), present, "slot {}", c);
        if present {
            prop_assert_eq!(got(&d.lists[c + 1]), Some(want_list(src)));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// decode(write(item)) gives the item's fields (after the writer's
    /// clamps) and ends in the stream's last byte.
    #[test]
    fn writer_and_reader_agree(item in item()) {
        let t = isc_table();
        match write(&item, &t) {
            Ok((bytes, _)) => {
                // The reader fails a full record whose stream quality is
                // outside 1–9 (the writer sends it before its rewrite to
                // 2) and reads on misaligned; D2RUST's reader
                // reports a read past the end as `Short`. Nothing more is
                // compared.
                let full = !item.compact && !item.alt;
                if full && !(1..=9).contains(&(item.quality & 0xF)) {
                    match decode(&bytes, &Lookup) {
                        Ok(d) => prop_assert!(d.failed),
                        Err(e) => prop_assert!(matches!(e, ItemBitsError::Short(_)), "{e}"),
                    }
                    return Ok(());
                }
                let d = decode(&bytes, &Lookup).map_err(|e| TestCaseError::fail(format!("{e}")))?;
                prop_assert!(!d.failed);
                prop_assert_eq!(d.bits.div_ceil(8), bytes.len());
                check(&item, &d)?;
            }
            Err(_) => {
                // Only a stream past the buffer overflows.
                let mut w = bitstream::BitWriter::new(4096);
                bitstream::write_into(&mut w, &item, &t);
                prop_assert!(w.bit_len() > BUFFER * 8);
            }
        }
    }
}

/// The grouped stats: the partners travel with their
/// head, a partner equal to the recorded value is not repeated, a
/// different one is.
#[test]
fn grouped_partners_round_trip() {
    let t = isc_table();
    let mut item = StreamItem {
        flags: 0x10,
        code: *b"lax ",
        kind: CODES[0].1,
        quality: 2,
        ilvl: 5,
        runeword: 0xFFFF,
        main: Some(vec![
            StatEntry {
                stat: 17,
                param: 0,
                value: 30,
            },
            StatEntry {
                stat: 18,
                param: 0,
                value: 30,
            },
            StatEntry {
                stat: 48,
                param: 0,
                value: 2,
            },
            StatEntry {
                stat: 49,
                param: 0,
                value: 9,
            },
        ]),
        ..StreamItem::default()
    };
    let (b, _) = write(&item, &t).unwrap();
    let d = decode(&b, &Lookup).unwrap();
    let l: Vec<(u16, u32)> = d.lists[0]
        .as_ref()
        .unwrap()
        .iter()
        .map(|s| (s.stat, s.raw))
        .collect();
    assert_eq!(l, [(17, 30), (18, 30), (48, 2), (49, 9)]);
    // 18 differs from 17's partner value only when it is its own entry:
    // the same list cannot hold two values of 18, so the record skips
    // every later 18; a list without 18 sends 0 as the partner.
    item.main = Some(vec![StatEntry {
        stat: 17,
        param: 0,
        value: 30,
    }]);
    let (b, _) = write(&item, &t).unwrap();
    let d = decode(&b, &Lookup).unwrap();
    let l: Vec<(u16, u32)> = d.lists[0]
        .as_ref()
        .unwrap()
        .iter()
        .map(|s| (s.stat, s.raw))
        .collect();
    assert_eq!(l, [(17, 30), (18, 0)]);
}
