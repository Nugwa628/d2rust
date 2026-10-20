//! test vectors and edge cases.

use super::*;
use crate::items::props::mode;
use crate::items::quality::{dispatch, roll_quality, superior_fits, unique};
use crate::items::tables::{QualityRec, UniqueRec};
use crate::items::{flag, q, req, stat, ItemRequest};

#[test]
fn request_quality_without_draw() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(AXE, b"axe "));
    let mut it = item(i, 3);
    let rq = ItemRequest {
        quality: q::RARE,
        ..Default::default()
    };
    assert_eq!(roll_quality(&t, &mut it, &rq), Ok(q::RARE));
    assert_eq!(it.item_seed, Seed::init_low(3));
}

#[test]
fn misc_item_level_one_first_draw() {
    let mut t = tables();
    let mut r = item_rec(RING, b"rin ");
    r.level = 30;
    let i = push_item(&mut t, r);
    let row = &mut t.itemratio[0];
    row.unique = 400;
    row.uniquedivisor = 1;
    row.rare = 0;
    row.raredivisor = 1;
    for seed in 0..20 {
        let mut it = item(i, seed);
        let rq = ItemRequest {
            ilvl: 80,
            ..Default::default()
        };
        let got = roll_quality(&t, &mut it, &rq).unwrap();
        let mut s = Seed::init_low(seed);
        let want = if s.roll(399) == 0 { q::UNIQUE } else { q::RARE };
        assert_eq!(got, want);
        assert_eq!(it.item_seed, s, "exactly one draw, roll(399)");
    }
}

#[test]
fn no_hit_fallback_and_divisor_zero() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(AXE, b"axe "));
    let row = &mut t.itemratio[0];
    (row.uniquedivisor, row.raredivisor, row.setdivisor) = (1, 1, 1);
    (row.magicdivisor, row.hiqualitydivisor, row.normaldivisor) = (1, 1, 1);
    (
        row.unique,
        row.rare,
        row.set,
        row.magic,
        row.hiquality,
        row.normal,
    ) = (1000, 1000, 1000, 1000, 1000, 1000);
    let seed = find_seed(|s| (0..6).all(|_| s.roll(999) != 0));
    let mut it = item(i, seed);
    let rq = ItemRequest {
        ilvl: 1,
        ..Default::default()
    };
    assert_eq!(roll_quality(&t, &mut it, &rq), Ok(q::LOW));
    let mut it = item(i, seed);
    let rq = ItemRequest {
        ilvl: 1,
        flags2: req::SUPERIOR,
        ..Default::default()
    };
    assert_eq!(roll_quality(&t, &mut it, &rq), Ok(q::SUPERIOR));
    t.itemratio[0].uniquedivisor = 0;
    let mut it = item(i, seed);
    assert_eq!(
        roll_quality(&t, &mut it, &ItemRequest::default()),
        Err(crate::items::Fatal::DivideByZero)
    );
}

/// Vector: a downgrade replays the item seed from the saved value.
#[test]
fn downgrade_restores_saved_seed() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut it = item(i, 0xDEAD_BEEF);
    let mut rq = ItemRequest {
        quality: q::MAGIC,
        ..Default::default()
    };
    // No affix rows: magic fails; no qualityitems: superior fails; normal.
    assert_eq!(
        dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
        Ok(true)
    );
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!(rq.quality, q::NORMAL);
    assert_eq!(it.item_seed, Seed::new(3735928559, 666));
    assert_eq!(it.start_seed, 0xDEAD_BEEF);
}

fn unique_rows(rarities: &[u32], code: &[u8; 4]) -> Vec<UniqueRec> {
    rarities
        .iter()
        .map(|&r| UniqueRec {
            code: *code,
            enabled: true,
            rarity: r,
            lvl: 1,
            props: [PropRec::NONE; 12],
            ..Default::default()
        })
        .collect()
}

/// Vector: weights 1, 0 (→ 1), 3 give starts 0, 1, 2 and total 5.
#[test]
fn unique_weights() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = unique_rows(&[1, 0, 3], b"rin ");
    for (r, want) in [(0, 0), (1, 1), (2, 2), (4, 2)] {
        let seed = find_seed(|s| s.roll(5) == r);
        let mut it = item(i, seed);
        let mut game = FakeGame::default();
        assert!(unique(&t, &mut game, &mut it, &ItemRequest::default()));
        assert_eq!(it.file_index, want, "r = {r}");
        assert!(game.uniques.get(want as u32));
        assert_eq!(it.flags & flag::IDENTIFIED, 0);
    }
}

/// Vector: index 4097 is not markable; the not-forced unique fails.
#[test]
fn unique_index_4097() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = unique_rows(&vec![1; 4098], b"rin ");
    let rq = ItemRequest {
        index: 4098,
        ..Default::default()
    };
    let mut game = FakeGame::default();
    let mut it = item(i, 1);
    assert!(!unique(&t, &mut game, &mut it, &rq));
    assert_eq!(it.file_index, -1);
    assert!(UniqueBits::default().get(4097));
    // With `nolimit` the accept test still fails (idx > 4096 reads as
    // dropped); only a quest item reaches the marking, which then succeeds.
    t.uniques[4097].nolimit = true;
    let mut it = item(i, 1);
    assert!(!unique(&t, &mut game, &mut it, &rq));
    t.items[i].quest = 1;
    let mut it = item(i, 1);
    assert!(unique(&t, &mut game, &mut it, &rq));
    assert_eq!(it.file_index, 4097);
}

#[test]
fn unique_already_dropped_and_forced_mismatch() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = unique_rows(&[1], b"rin ");
    let mut game = FakeGame::default();
    game.uniques.set(0);
    let mut it = item(i, 1);
    assert!(!unique(&t, &mut game, &mut it, &ItemRequest::default()));
    // Edge case 3: forced unique on a mismatched base.
    t.uniques[0].code = *b"amu ";
    let rq = ItemRequest {
        force: true,
        index: 0,
        ..Default::default()
    };
    let mut it = item(i, 1);
    it.flags = flag::IDENTIFIED;
    assert!(unique(&t, &mut game, &mut it, &rq));
    assert_eq!(it.file_index, 0);
    assert_ne!(it.flags & flag::IDENTIFIED, 0);
}

/// Edge cases 1 and 6: a failed unique becomes rare with triple
/// durability; with items `unique` set it stays unique.
#[test]
fn failed_unique_downgrades() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 20;
    let i = push_item(&mut t, r);
    let mut it = item(i, 9);
    it.stats.set_base(stat::DURABILITY, 0, 10);
    it.stats.set_base(stat::MAXDURABILITY, 0, 20);
    let mut rq = ItemRequest {
        quality: q::UNIQUE,
        flags2: req::NEVER_ETHEREAL,
        ..Default::default()
    };
    dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq).unwrap();
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 30);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 60);

    t.items[i].unique = 1;
    t.itemratio[0].uniquedivisor = 1;
    let mut it = item(i, 9);
    let mut rq = ItemRequest {
        flags2: req::NEVER_ETHEREAL,
        ..Default::default()
    };
    dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq).unwrap();
    assert_eq!(it.quality, q::UNIQUE);
    assert_eq!(it.file_index, -1);
}

#[test]
fn superior_row_fits() {
    let mut t = tables();
    let axe = push_item(&mut t, item_rec(AXE, b"axe "));
    let staf = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.qualityitems = vec![QualityRec {
        weapon: 1,
        ..Default::default()
    }];
    assert!(superior_fits(&t, &item(axe, 1), 0));
    assert!(!superior_fits(&t, &item(staf, 1), 0));
    t.qualityitems[0].staff = 1;
    assert!(superior_fits(&t, &item(staf, 1), 0));
}

/// Superior: redraws on tried rows, then mode-1 properties.
#[test]
fn superior_picks_fitting_row() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(AXE, b"axe "));
    t.properties = vec![prop1(1, stat::TOBLOCK)];
    t.qualityitems = (0..8)
        .map(|k| QualityRec {
            weapon: u8::from(k == 5),
            armor: 1,
            mods: [rec(0, 0, 4, 4), PropRec::NONE],
            ..Default::default()
        })
        .collect();
    let mut it = item(i, 21);
    assert!(crate::items::quality::superior(
        &t,
        &mut it,
        &ItemRequest::default()
    ));
    assert_eq!(it.file_index, 5);
    assert_eq!(it.stats.item_list(stat::TOBLOCK, 0), 4);
    let _ = mode::QUALITY;
}

/// Edge case 2: low-quality throwing damage clamps min ≥ 2, max ≥ 1.
#[test]
fn low_quality_throwing_bounds() {
    let mut t = tables();
    let mut r = item_rec(THROWN, b"tax ");
    r.durability = 30;
    let i = push_item(&mut t, r);
    t.n_lowquality = 4;
    let mut it = item(i, 2);
    it.unit_seed = Seed::init_low(8);
    for s in [21, 22, 159, 160] {
        it.stats.set_base(s, 0, 1);
    }
    assert!(crate::items::quality::low_quality(
        &t,
        &mut it,
        &ItemRequest::default()
    ));
    assert_eq!(it.stats.base(stat::THROW_MINDAMAGE, 0), 2);
    assert_eq!(it.stats.base(stat::THROW_MAXDAMAGE, 0), 1);
    assert_eq!(it.stats.base(stat::MINDAMAGE, 0), 1);
    assert_eq!(it.stats.base(stat::MAXDAMAGE, 0), 2);
    // Durability: m = 30 × 33 / 100 = 9; roll(4) + 4 on the unit seed.
    let mut u = Seed::init_low(8);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), u.roll(4) as i32 + 4);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 9);
    let mut s = Seed::init_low(2);
    assert_eq!(it.file_index, s.roll(4) as i32);
}

#[test]
fn set_item_weights_and_hellbovine() {
    use crate::items::tables::SetItemRec;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let row = |set, rarity| SetItemRec {
        item: *b"rin ",
        set,
        lvl: 1,
        rarity,
        props: [PropRec::NONE; 9],
        aprops: [PropRec::NONE; 10],
        ..Default::default()
    };
    t.setitems = vec![row(1, 2), row(29, 5), row(2, 0)];
    for (r, want) in [(0, 0), (1, 0), (2, 2)] {
        let seed = find_seed(|s| s.roll(3) == r);
        let mut it = item(i, seed);
        assert!(crate::items::quality::set_item(
            &t,
            &mut it,
            &ItemRequest::default()
        ));
        assert_eq!(it.file_index, want);
    }
    let rq = ItemRequest {
        flags2: req::HELLBOVINE,
        index: 2,
        ..Default::default()
    };
    let mut it = item(i, 1);
    assert!(crate::items::quality::set_item(&t, &mut it, &rq));
    assert_eq!(it.file_index, 1);
    assert_eq!(it.item_seed, Seed::init_low(1), "preferred: no draw");
}
