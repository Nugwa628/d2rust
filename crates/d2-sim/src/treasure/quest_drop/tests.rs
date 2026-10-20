//! Synthetic vectors for the quest drop helper (RNG steps: seed
//! {12345, 666} gives `lo'` 22752887,
//! then 2337785264).

use super::*;
use crate::treasure::tests::Sink;
use crate::treasure::walk::DropperKind;

const LO1: u32 = 22_752_887;
const LO2: u32 = 2_337_785_264;

fn seed() -> Seed {
    Seed::new(12345, 666)
}

fn rec(code: &[u8; 4], type_: i16, level: u8) -> ItemRec {
    ItemRec {
        code: *code,
        type_,
        level,
        spawnable: 1,
        ..ItemRec::default()
    }
}

/// weapons 0..2, armor 2..4, misc 4..
fn data(items: &[ItemRec], expansion: bool) -> PickData<'_> {
    PickData {
        items,
        parts: [Some((0, 2)), Some((2, 2)), Some((4, items.len() - 4))],
        expansion,
    }
}

fn steps(mut s: Seed, n: usize) -> Seed {
    for _ in 0..n {
        s.step();
    }
    s
}

#[test]
fn sub_pick_parts_body_parts_and_classic_filter() {
    let mut items = vec![
        rec(b"wp0 ", 1, 1),
        rec(b"wp1 ", 1, 1),
        rec(b"ar0 ", 2, 1),
        rec(b"ar1 ", 2, 1),
        rec(b"bp0 ", TYPE_BODY_PART, 1),
        rec(b"ms0 ", 3, 1),
        rec(b"ms1 ", 3, 1),
    ];
    items[6].version = 100;
    // Misc, a monster, expansion: body part, ms0, ms1 (n = 3, mod).
    let mut s = seed();
    let c = sub_pick(&data(&items, true), Part::Misc, &mut s, 1, -1, 1, true);
    assert_eq!(c, [4, 5, 6][(LO1 % 3) as usize]);
    assert_eq!(s, steps(seed(), 1));
    // Not a monster: no body part; classic: no version-100 row → ms0 only.
    let mut s = seed();
    assert_eq!(
        sub_pick(&data(&items, false), Part::Misc, &mut s, 1, -1, 1, false),
        5
    );
    // Armor is the armor picker's range: ar0 / ar1 (n = 2, mask).
    let mut s = seed();
    assert_eq!(
        sub_pick(&data(&items, true), Part::Armor, &mut s, 1, -1, 1, false),
        [2, 3][(LO1 & 1) as usize]
    );
    // Weapons: wp0 / wp1.
    let mut s = seed();
    assert_eq!(
        sub_pick(&data(&items, true), Part::Weapons, &mut s, 1, -1, 1, false),
        [0, 1][(LO1 & 1) as usize]
    );
}

#[test]
fn sub_pick_filter_spawnable_quest_level_and_type() {
    let mut items = vec![
        rec(b"wp0 ", 1, 1),
        rec(b"wp1 ", 1, 1),
        rec(b"ar0 ", 2, 1),
        rec(b"ar1 ", 2, 1),
        rec(b"a   ", 3, 5),
        rec(b"b   ", 3, 6),
        rec(b"c   ", 3, 1),
        rec(b"d   ", 3, 1),
        rec(b"e   ", 4, 1),
    ];
    items[6].spawnable = 0;
    items[7].quest = 1;
    let d = data(&items, true);
    // L = 5: a passes (level 5 ≤ 5), b not, c / d filtered, e passes.
    let mut s = seed();
    let c = sub_pick(&d, Part::Misc, &mut s, 5, -1, 1, false);
    assert_eq!(c, [4, 8][(LO1 & 1) as usize]);
    // p6 = 4: only e (n = 1).
    let mut s = seed();
    assert_eq!(sub_pick(&d, Part::Misc, &mut s, 5, 4, 1, false), 8);
    assert_eq!(s, steps(seed(), 1));
    // L < 1 counts as 1: only e (a's level 5 > 1).
    let mut s = seed();
    assert_eq!(sub_pick(&d, Part::Misc, &mut s, 0, -1, 1, false), 8);
}

#[test]
fn sub_pick_rarity_roll_uses_level_as_level_id() {
    // Two misc rows of rarity 2: at L = 1 (act 0) d = 2, one roll(2) each,
    // in index order; L = 40 reads act 1: d = 1, roll(1) = 0 always passes
    // (still one step each).
    let mut items = vec![
        rec(b"wp0 ", 1, 1),
        rec(b"wp1 ", 1, 1),
        rec(b"ar0 ", 2, 1),
        rec(b"ar1 ", 2, 1),
        rec(b"a   ", 3, 1),
        rec(b"b   ", 3, 1),
    ];
    items[4].rarity = 2;
    items[5].rarity = 2;
    let d = data(&items, true);
    let mut s = seed();
    let got = sub_pick(&d, Part::Misc, &mut s, 1, -1, 0, false);
    let a_in = LO1.is_multiple_of(2);
    let b_in = LO2.is_multiple_of(2);
    let mut want = seed();
    want.step();
    want.step();
    let cands: Vec<i32> = [(4, a_in), (5, b_in)]
        .iter()
        .filter(|x| x.1)
        .map(|x| x.0)
        .collect();
    let expect = if cands.is_empty() {
        -1
    } else {
        let lo = want.step();
        cands[(lo % cands.len() as u32) as usize]
    };
    assert_eq!(got, expect);
    assert_eq!(s, want);
    // L = 40: act 1, d = 1: both pass, 2 roll steps + the pick step.
    let mut s = seed();
    let lo3 = steps(seed(), 3).lo;
    assert_eq!(
        sub_pick(&d, Part::Misc, &mut s, 40, -1, 0, false),
        [4, 5][(lo3 & 1) as usize]
    );
    // rarity ≤ act: no roll.
    items[4].rarity = 1;
    items[5].rarity = 1;
    let d = data(&items, true);
    let mut s = seed();
    sub_pick(&d, Part::Misc, &mut s, 40, -1, 0, false);
    assert_eq!(s, steps(seed(), 1));
}

#[test]
fn sub_pick_absent_part_or_no_candidate_is_minus_one_without_draw() {
    let items = vec![rec(b"wp0 ", 1, 9)];
    let d = PickData {
        items: &items,
        parts: [Some((0, 1)), None, None],
        expansion: true,
    };
    let mut s = seed();
    assert_eq!(sub_pick(&d, Part::Armor, &mut s, 1, -1, 1, false), -1);
    assert_eq!(sub_pick(&d, Part::Weapons, &mut s, 1, -1, 1, false), -1);
    assert_eq!(s, seed());
}

#[test]
fn class_pick_bands_and_fatal_level() {
    let items = vec![
        rec(b"wp0 ", 1, 1),
        rec(b"wp1 ", 1, 1),
        rec(b"ar0 ", 2, 1),
        rec(b"ar1 ", 2, 1),
        rec(b"gld ", 4, 1),
        rec(b"ms0 ", 3, 1),
    ];
    let d = data(&items, true);
    assert_eq!(
        class_pick(&d, &mut seed(), 66, -1, 1, false),
        Err(TreasureError::PickLevel(66))
    );
    // L = 1: r = 87 ≥ 64 + 5 + 11 = 80 → misc (gld, ms0: n = 2).
    assert_eq!(LO1 % 100, 87);
    let mut s = seed();
    let c = class_pick(&d, &mut s, 1, -1, 1, false).unwrap();
    assert_eq!(c, [4, 5][(LO2 & 1) as usize]);
    // Every band at L = 1 (gold < 64, armor < 69, weapons < 80, misc),
    // over a run of seeds.
    for lo in 0..200u32 {
        let mut s = Seed::new(lo, 666);
        let mut probe = s;
        let r = probe.roll(100);
        let pick = probe.step() & 1;
        let want = match r {
            0..=63 => 4,
            64..=68 => [2, 3][pick as usize],
            69..=79 => [0, 1][pick as usize],
            _ => [4, 5][pick as usize],
        };
        assert_eq!(class_pick(&d, &mut s, 1, -1, 1, false), Ok(want), "lo {lo}");
    }
    // L = 65: gold band empty, armor r < 37, weapons r < 80 (odd: + 1).
    for lo in 0..200u32 {
        let mut s = Seed::new(lo, 666);
        let mut probe = s;
        let r = probe.roll(100);
        let pick = probe.step() & 1;
        let want = match r {
            0..=36 => [2, 3][pick as usize],
            37..=79 => [0, 1][pick as usize],
            _ => [4, 5][pick as usize],
        };
        assert_eq!(
            class_pick(&d, &mut s, 65, -1, 1, false),
            Ok(want),
            "lo {lo}"
        );
    }
}

#[test]
fn drop_code_wins_without_draw() {
    let items = vec![rec(b"wp0 ", 1, 1), rec(b"key ", 3, 1)];
    let d = PickData {
        items: &items,
        parts: [Some((0, 1)), None, Some((1, 1))],
        expansion: true,
    };
    let mut s = seed();
    assert_eq!(
        quest_class(&d, Some(*b"key "), 4, &mut s, 1, -1, 0, false),
        Ok(1)
    );
    assert_eq!(s, seed());
    assert_eq!(
        quest_class(&d, Some(*b"zzz "), 2, &mut s, 1, -1, 0, false),
        Err(TreasureError::DropCode(*b"zzz "))
    );
}

#[test]
fn magic_loop_retries_eleven_class_picks_then_weapons() {
    // A non-magic gold row and no parts: every class pick draws one
    // roll(100) and gives gold or −1; after the 11th retry the weapons
    // picker (absent) draws nothing, so the loop spends exactly 12 steps
    // (D2RUST guard ends it). The gold row is needed: a missing `gld ` row
    // is fatal 0x17C before the roll.
    let items = vec![rec(b"wp0 ", 1, 1), rec(b"gld ", 4, 1)];
    let d = PickData {
        items: &items,
        parts: [None, None, None],
        expansion: true,
    };
    let mut s = seed();
    assert_eq!(
        quest_class(&d, None, QUALITY_MAGIC, &mut s, 1, -1, 1, false),
        Err(TreasureError::QuestDropHang)
    );
    assert_eq!(s, steps(seed(), 12));
    // Not magic: one pick, −1 kept.
    let mut s = seed();
    assert_eq!(quest_class(&d, None, 2, &mut s, 1, -1, 1, false), Ok(-1));
    assert_eq!(s, steps(seed(), 1));
}

#[test]
fn magic_loop_keeps_first_magic_class() {
    // Weapons: wp0 magic. Retry until the roll lands in the weapons band.
    // The (non-magic) gold row keeps the gold band out of fatal 0x17C;
    // it is retried like −1.
    let mut items = vec![rec(b"wp0 ", 1, 1), rec(b"gld ", 4, 1)];
    items[0].bitfield1 = 1;
    let d = PickData {
        items: &items,
        parts: [Some((0, 1)), None, None],
        expansion: true,
    };
    let mut s = seed();
    assert_eq!(
        quest_class(&d, None, QUALITY_MAGIC, &mut s, 1, -1, 1, false),
        Ok(0)
    );
    // Reference: count the class picks by hand.
    let mut r = seed();
    let mut calls = 0;
    loop {
        calls += 1;
        if calls > 12 {
            // The weapons picker: one step.
            r.step();
            break;
        }
        if (69..80).contains(&r.roll(100)) {
            r.step();
            break;
        }
    }
    assert_eq!(s, r);
}

#[test]
fn quest_drop_level_spot_and_request() {
    let items = vec![rec(b"wp0 ", 1, 1), rec(b"key ", 3, 1)];
    let d = PickData {
        items: &items,
        parts: [Some((0, 1)), None, Some((1, 1))],
        expansion: true,
    };
    let args = QuestDropArgs {
        quality: 2,
        drop_code: Some(*b"key "),
        p6: -1,
        p7: 0,
        item_format: 101,
    };
    let u = Dropper {
        kind: DropperKind::Monster {
            class: 5,
            level: 0,
            playercount: 0,
        },
        x: 10,
        y: 20,
    };
    // No spot: level written (L < 2 → 1), no request, no item.
    let mut sink = Sink {
        no_spot: true,
        ..Sink::default()
    };
    let out = quest_drop(&d, &u, true, &mut seed(), &args, &mut sink).unwrap();
    assert_eq!((out.level, out.request, out.item), (1, None, None));
    assert!(sink.reqs.is_empty());
    // A spot: one request (spawn 3, init 1, the item, quality, ilvl).
    let u = Dropper {
        kind: DropperKind::Player { level: 17 },
        ..u
    };
    let mut sink = Sink::default();
    let out = quest_drop(&d, &u, false, &mut seed(), &args, &mut sink).unwrap();
    assert_eq!(out.level, 17);
    assert_eq!(out.item, Some(0));
    let rq = out.request.unwrap();
    assert_eq!(
        (
            rq.id,
            rq.quality,
            rq.item_level,
            rq.spawn_type,
            rq.init_flags,
            rq.item_format,
            rq.index,
            rq.drop_flags
        ),
        (1, 2, 17, 3, 1, 101, 0, 0)
    );
    assert_eq!(sink.reqs.len(), 1);
    // Other units: the area level; none: 1.
    let u = Dropper {
        kind: DropperKind::Other { area_level: 42 },
        ..u
    };
    let out = quest_drop(&d, &u, false, &mut seed(), &args, &mut Sink::default()).unwrap();
    assert_eq!(out.level, 42);
    let u = Dropper {
        kind: DropperKind::None,
        ..u
    };
    let out = quest_drop(&d, &u, false, &mut seed(), &args, &mut Sink::default()).unwrap();
    assert_eq!(out.level, 1);
}

#[test]
fn quest_drop_minus_one_class_creates_nothing() {
    // The gold row keeps the class pick out of fatal 0x17C;
    // r = 87 lands in the (absent) misc part.
    let items = vec![rec(b"wp0 ", 1, 9), rec(b"gld ", 4, 1)];
    let d = PickData {
        items: &items,
        parts: [None, None, None],
        expansion: true,
    };
    let args = QuestDropArgs {
        quality: 2,
        drop_code: None,
        p6: -1,
        p7: 0,
        item_format: 0,
    };
    let u = Dropper {
        kind: DropperKind::None,
        x: 0,
        y: 0,
    };
    let mut sink = Sink::default();
    let out = quest_drop(&d, &u, false, &mut seed(), &args, &mut sink).unwrap();
    assert!(out.item.is_none());
    assert!(out.request.is_some());
    assert!(sink.reqs.is_empty());
}
