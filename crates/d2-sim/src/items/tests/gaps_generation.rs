//! Gap tests for item-creation rules the `create.rs` tests do not pin
//! (one rule or a small group per test; synthetic tables, seeds replayed).

use super::*;
use crate::items::create::{
    class_skill_mods, ethereal_roll, has_durability, init_item_stats, normal, socket_roll,
};
use crate::items::quality::{dispatch, low_quality, superior};
use crate::items::tables::{QualityRec, RareRec, SkillRec};
use crate::items::{create_item, flag, q, req, stat, CreateError, Fatal, ItemRequest, RequestUnit};

/// An itemtypes row with `quiver` set and no parent (synthetic).
const QUIVER: u16 = 60;

#[test]
fn quality_ids() {
    let ids = [
        (q::NONE, 0),
        (q::LOW, 1),
        (q::NORMAL, 2),
        (q::SUPERIOR, 3),
        (q::MAGIC, 4),
        (q::SET, 5),
        (q::RARE, 6),
        (q::UNIQUE, 7),
        (q::CRAFTED, 8),
        (q::TEMPERED, 9),
    ];
    for (k, (got, want)) in ids.into_iter().enumerate() {
        assert_eq!(got, want, "entry {k}");
    }
}

#[test]
fn item_format_by_game() {
    let g = FakeGame::default();
    assert_eq!(g.item_format(), 101);
    let g = FakeGame {
        expansion: false,
        ..Default::default()
    };
    assert_eq!(g.item_format(), 2);
}

#[test]
fn type_tests_and_helpers() {
    // Type numbers.
    for (got, want) in [
        (ty::WEAP, 45),
        (ty::ARMO, 50),
        (ty::MISC, 52),
        (ty::TORS, 3),
        (ty::HELM, 37),
        (ty::CHAR, 13),
        (ty::BODY, 40),
        (ty::PLAY, 7),
        (ty::SCRO, 22),
        (ty::BOOK, 18),
        (ty::GOLD, 4),
        (ty::ELIX, 11),
        (ty::JEWL, 58),
        (ty::GEM, 20),
        (ty::RUNE, 74),
    ] {
        assert_eq!(got, want);
    }
    let mut t = tables();
    // "Item is type T": `type` equivalent, or `type2` when > 0.
    let mut r = item_rec(ty::HELM, b"cap ");
    r.type2 = AXE as i16;
    let both = push_item(&mut t, r);
    assert!(t.is_type(both, ty::ARMO as i16));
    assert!(t.is_type(both, ty::WEAP as i16));
    let mut r = item_rec(ty::HELM, b"cap ");
    r.type2 = -1;
    let neg = push_item(&mut t, r);
    assert!(!t.is_type(neg, ty::WEAP as i16));
    // The itemtypes row used for flags is the primary type's: a `type2`
    // quiver does not make the item a quiver (takes the armor branch).
    t.itemtypes[QUIVER as usize].quiver = 1;
    let mut r = item_rec(ty::HELM, b"cap ");
    r.type2 = QUIVER as i16;
    r.durability = 10;
    r.minac = 2;
    r.maxac = 2;
    let h = push_item(&mut t, r);
    let mut it = item(h, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 2);
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 0);
    // Has durability: nodurability 0, durability ≠ 0, a stat list, stat
    // 152 < 1.
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 10;
    let d = push_item(&mut t, r.clone());
    let mut it = item(d, 1);
    assert!(!has_durability(&t, &it), "no stat list");
    it.stats.set_base(stat::DURABILITY, 0, 5);
    assert!(has_durability(&t, &it));
    it.stats.set_base(stat::INDESTRUCTIBLE, 0, 1);
    assert!(!has_durability(&t, &it));
    r.nodurability = 1;
    let nd = push_item(&mut t, r.clone());
    let mut it = item(nd, 1);
    it.stats.set_base(stat::DURABILITY, 0, 5);
    assert!(!has_durability(&t, &it));
    r.nodurability = 0;
    r.durability = 0;
    let z = push_item(&mut t, r);
    let mut it = item(z, 1);
    it.stats.set_base(stat::DURABILITY, 0, 5);
    assert!(!has_durability(&t, &it));
    // Total max stack: maxstack + stat 254, capped at 511 (seen through
    // the quiver roll r(total − min) + min).
    let mut r = item_rec(QUIVER, b"aqv ");
    r.maxstack = 10;
    let qv = push_item(&mut t, r);
    let mut it = item(qv, 1);
    it.unit_seed = Seed::init_low(9);
    it.stats.set_base(stat::EXTRA_STACK, 0, 5);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    let mut s = Seed::init_low(9);
    assert_eq!(it.stats.base(stat::QUANTITY, 0), (s.roll(15) as i32).max(1));
    assert_eq!(it.unit_seed, s);
    let mut it = item(qv, 1);
    it.unit_seed = Seed::init_low(9);
    it.stats.set_base(stat::EXTRA_STACK, 0, 600);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    let mut s = Seed::init_low(9);
    assert_eq!(
        it.stats.base(stat::QUANTITY, 0),
        (s.roll(511) as i32).max(1)
    );
    // Item level: a stored value < 1 is set to 1 first.
    let mut it = item(qv, 1);
    it.ilvl = -4;
    assert_eq!(it.item_level(), 1);
    assert_eq!(it.ilvl, 1);
    // Is magic or better: quality 4–9.
    for x in 0..=10u8 {
        it.quality = x;
        assert_eq!(it.magic_or_better(), (4..=9).contains(&x), "quality {x}");
    }
}

#[test]
fn item_flag_bits() {
    for (got, want) in [
        (flag::IDENTIFIED, 0x10),
        (flag::BROKEN, 0x100),
        (flag::SOCKETED, 0x800),
        (flag::NOSELL, 0x1000),
        (flag::INSTORE, 0x2000),
        (flag::NAMED, 0x8000),
        (flag::EAR, 0x10000),
        (flag::STARTITEM, 0x20000),
        (flag::INIT, 0x80000),
        (flag::ETHEREAL, 0x400000),
        (flag::PERSONALIZED, 0x1000000),
        (flag::RUNEWORD, 0x4000000),
    ] {
        assert_eq!(got, want);
    }
}

#[test]
fn request_flag_bits() {
    for (got, want) in [
        (req::HELLBOVINE, 0x01),
        (req::NEVER_ETHEREAL, 0x02),
        (req::ALWAYS_ETHEREAL, 0x04),
        (req::NO_SOCKETS, 0x08),
        (req::ALWAYS_SOCKETS, 0x10),
        (req::STAFFMODS_ILVL, 0x20),
        (req::SUPERIOR, 0x40),
    ] {
        assert_eq!(got, want);
    }
}

#[test]
fn stat_ids_and_set_stat() {
    for (got, want) in [
        (stat::GOLD, 14),
        (stat::TOBLOCK, 20),
        (stat::MINDAMAGE, 21),
        (stat::MAXDAMAGE, 22),
        (stat::SECONDARY_MINDAMAGE, 23),
        (stat::SECONDARY_MAXDAMAGE, 24),
        (stat::ARMORCLASS, 31),
        (stat::VELOCITYPERCENT, 67),
        (stat::ATTACKRATE, 68),
        (stat::QUANTITY, 70),
        (stat::VALUE, 71),
        (stat::DURABILITY, 72),
        (stat::MAXDURABILITY, 73),
        (stat::ITEM_SINGLESKILL, 107),
        (stat::THROW_MINDAMAGE, 159),
        (stat::THROW_MAXDAMAGE, 160),
        (stat::NUMSOCKETS, 194),
        (stat::QUESTITEMDIFFICULTY, 356),
    ] {
        assert_eq!(got, want);
    }
    // "Set stat" writes base stats, not a stat list.
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 8;
    r.minac = 1;
    r.maxac = 1;
    let i = push_item(&mut t, r);
    let mut it = item(i, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert!(it.stats.lists.is_empty());
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 8);
}

fn helm(t: &mut ItemTables) -> usize {
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 12;
    r.minac = 3;
    r.maxac = 5;
    push_item(t, r)
}

/// A caller-seeded (not forced) request overwrites both seeds after the
/// allocation drew its two game-seed steps.
#[test]
fn use_seed_overwrites_both_seeds() {
    let mut t = tables();
    let i = helm(&mut t);
    let mut game = FakeGame::default();
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::NORMAL,
        seed: 500,
        item_seed: 600,
        ..Default::default()
    };
    let c = create_item(&t, &mut game, &mut rq, true, FakeStats::default(), 0).unwrap();
    let mut g = Seed::init_low(12345);
    g.step();
    g.step();
    assert_eq!(game.seed, g, "allocation steps stay made");
    let it = &c.item;
    assert_eq!(it.init_seed, 500);
    assert_eq!(it.start_seed, 600);
    // Not forced: still in store.
    assert_ne!(it.flags & flag::INSTORE, 0);
    // Base stats drew from {500, 666}: durability r(6), defense r(3).
    let mut u = Seed::init_low(500);
    let dur = u.roll(6) as i32 + 6;
    let ac = u.roll_range(3, 3);
    assert_eq!(it.unit_seed, u);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), dur);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), ac);
}

fn socket_helm(t: &mut ItemTables) -> usize {
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 12;
    r.minac = 3;
    r.maxac = 5;
    r.hasinv = 1;
    r.gemsockets = 6;
    r.invwidth = 2;
    r.invheight = 3;
    let h = &mut t.itemtypes[ty::HELM as usize];
    (h.maxsock1, h.maxsock25, h.maxsock40) = (6, 6, 6);
    push_item(t, r)
}

/// Unit seed: base stats and graphics only; item seed: the quality
/// routine's, ethereal and socket rolls. Each sequence is independent of
/// the other.
#[test]
fn unit_and_item_seed_sequences() {
    let mut t = tables();
    let i = socket_helm(&mut t);
    t.itemtypes[ty::HELM as usize].varinvgfx = 3;
    let make = |unit: u32, its: u32| {
        let mut rq = ItemRequest {
            item: i as i32,
            format: 101,
            quality: q::NORMAL,
            ilvl: 10,
            seed: unit,
            item_seed: its,
            ..Default::default()
        };
        create_item(
            &t,
            &mut FakeGame::default(),
            &mut rq,
            true,
            FakeStats::default(),
            0,
        )
        .unwrap()
        .item
    };
    let a = make(70, 80);
    // Unit seed: durability, defense, graphics, nothing else.
    let mut u = Seed::init_low(70);
    u.roll(6);
    u.roll_range(3, 3);
    let gfx = u.roll_range(0, 3);
    assert_eq!(a.unit_seed, u);
    assert_eq!(a.gfx, gfx);
    // Item seed: ethereal roll(100), then socket roll(100).
    let mut s = Seed::init_low(80);
    s.roll(100);
    s.roll(100);
    assert_eq!(a.item_seed, s);
    // Independent: another unit seed changes nothing drawn from the item
    // seed, and the other way round.
    let b = make(71, 80);
    assert_eq!(b.item_seed, a.item_seed);
    assert_eq!(b.flags, a.flags);
    assert_eq!(
        b.stats.base(stat::NUMSOCKETS, 0),
        a.stats.base(stat::NUMSOCKETS, 0)
    );
    let c = make(70, 81);
    assert_ne!(c.item_seed, a.item_seed);
    assert_eq!(c.unit_seed, a.unit_seed);
    assert_eq!(c.gfx, a.gfx);
}

/// The start seed: read without a draw by the socket count; rewritten by
/// the downgrade chain.
#[test]
fn start_seed_use_and_rewrite() {
    let mut t = tables();
    let i = socket_helm(&mut t);
    let game = FakeGame {
        difficulty: 2,
        ..Default::default()
    };
    let mut it = item(i, 41);
    it.quality = q::NORMAL;
    let rq = ItemRequest {
        flags2: req::ALWAYS_SOCKETS,
        ..Default::default()
    };
    socket_roll(&t, &game, &mut it, &rq);
    let mut s = Seed::init_low(41);
    s.roll(100);
    assert_eq!(it.item_seed, s, "only the roll(100) draw");
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 41 % 6 + 1);
    assert_eq!(it.start_seed, 41);

    // Downgrade: the itemratio roll draws once (unique: c = 2 − 1 = 1,
    // roll(1)); unique fails (no rows), so the chain restarts the item
    // seed from the saved low word and writes it as the start seed.
    let mut t = tables();
    let i = helm(&mut t);
    let row = &mut t.itemratio[0];
    (row.unique, row.uniquedivisor) = (2, 1);
    let mut it = item(i, 0x1234);
    let mut rq = ItemRequest {
        ilvl: 1,
        ..Default::default()
    };
    assert_eq!(
        dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
        Ok(true)
    );
    assert_eq!(it.quality, q::NORMAL);
    // Saved after the dispatch's roll (one step), restarted, rolled once
    // more (the request quality is still 0), saved again; the later
    // iterations take the request quality without a draw.
    let l1 = Seed::init_low(0x1234).step();
    let l2 = Seed::init_low(l1).step();
    assert_eq!(it.start_seed, l2, "rewritten");
}

#[test]
fn dispatch_failure_fails_creation() {
    let mut t = tables();
    let i = helm(&mut t);
    let mut game = FakeGame::default();
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: 10,
        ..Default::default()
    };
    let e = create_item(&t, &mut game, &mut rq, false, FakeStats::default(), 0);
    assert_eq!(e.unwrap_err(), CreateError::Failed);
    let mut g = Seed::init_low(12345);
    g.step();
    g.step();
    assert_eq!(game.seed, g);
    // Success: ran the quality dispatch.
    rq.quality = q::NORMAL;
    let c = create_item(&t, &mut game, &mut rq, false, FakeStats::default(), 0).unwrap();
    assert_eq!(c.item.quality, q::NORMAL);
}

/// Run only for forced requests.
#[test]
fn not_forced_skips_forced_steps() {
    let mut t = tables();
    let i = helm(&mut t);
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::NORMAL,
        flags1: flag::IDENTIFIED | flag::NOSELL | flag::BROKEN | flag::STARTITEM,
        quantity: 9,
        min_dur: 300,
        max_dur: 400,
        ..Default::default()
    };
    let c = create_item(
        &t,
        &mut FakeGame::default(),
        &mut rq,
        false,
        FakeStats::default(),
        0,
    )
    .unwrap();
    assert_eq!(c.item.flags, flag::INIT | flag::INSTORE | flag::IDENTIFIED);
    assert_eq!(c.item.stats.base(stat::QUANTITY, 0), 0);
    assert_eq!((rq.min_dur, rq.max_dur), (300, 400));
    assert!(c.item.stats.base(stat::DURABILITY, 0) <= 12);
}

/// The ear step tests the primary type: `play` as `type2` does not need a
/// player.
#[test]
fn ear_step_on_primary_type_only() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.type2 = ty::PLAY as i16;
    let i = push_item(&mut t, r);
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::NORMAL,
        ..Default::default()
    };
    let c = create_item(
        &t,
        &mut FakeGame::default(),
        &mut rq,
        false,
        FakeStats::default(),
        0,
    );
    assert!(c.is_ok());
    let j = push_item(&mut t, item_rec(ty::PLAY, b"ear "));
    rq.item = j as i32;
    let e = create_item(
        &t,
        &mut FakeGame::default(),
        &mut rq,
        false,
        FakeStats::default(),
        0,
    );
    assert_eq!(e.unwrap_err(), CreateError::NotPlayer);
}

#[test]
fn pipeline_schedules_replenish() {
    let mut t = tables();
    let i = helm(&mut t);
    let mut stats = FakeStats::default();
    stats.set_base(stat::REPLENISH_DURABILITY, 0, 10);
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::NORMAL,
        ..Default::default()
    };
    let c = create_item(&t, &mut FakeGame::default(), &mut rq, false, stats, 100).unwrap();
    assert_eq!(c.event3_at, Some(100 + 250 + 1));
}

/// "r(n)" draws nothing for n < 1; branches are exclusive and tested in
/// order (gold, quiver, then the record branches).
#[test]
fn base_stat_draws_and_branch_order() {
    let mut t = tables();
    // Durability 1: r(0) draws nothing; only the defense roll steps.
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 1;
    r.minac = 3;
    r.maxac = 5;
    let i = push_item(&mut t, r);
    let mut it = item(i, 1);
    it.unit_seed = Seed::init_low(33);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    let mut u = Seed::init_low(33);
    let ac = u.roll_range(3, 3);
    assert_eq!(it.unit_seed, u);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 0);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 1);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), ac);
    // A quiver type that is also armor (type2): only the quiver branch.
    t.itemtypes[QUIVER as usize].quiver = 1;
    let mut r = item_rec(QUIVER, b"aqv ");
    r.type2 = ty::ARMO as i16;
    r.maxstack = 20;
    r.minac = 3;
    r.maxac = 5;
    r.durability = 10;
    let qv = push_item(&mut t, r);
    let mut it = item(qv, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_ne!(it.stats.base(stat::QUANTITY, 0), 0);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 0);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 0);
    // Gold whose itemtype is also a quiver: only gold.
    t.itemtypes[ty::GOLD as usize].quiver = 1;
    let g = push_item(&mut t, item_rec(ty::GOLD, b"gld "));
    let mut it = item(g, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::GOLD, 0), 1);
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 0);
}

#[test]
fn quiver_quantity() {
    let mut t = tables();
    t.itemtypes[QUIVER as usize].quiver = 1;
    let mut r = item_rec(QUIVER, b"aqv ");
    r.minstack = 20;
    r.maxstack = 50;
    let i = push_item(&mut t, r);
    for seed in 0..30 {
        let mut it = item(i, 1);
        it.unit_seed = Seed::init_low(seed);
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
        let mut u = Seed::init_low(seed);
        assert_eq!(it.stats.base(stat::QUANTITY, 0), u.roll(30) as i32 + 20);
        assert_eq!(it.unit_seed, u);
    }
    // q < 1 → 1.
    let z = push_item(&mut t, item_rec(QUIVER, b"aqv "));
    let mut it = item(z, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 1);
    // Override > 0 replaces the roll (which is still drawn).
    let mut it = item(i, 1);
    it.unit_seed = Seed::init_low(3);
    let mut rq = ItemRequest {
        quantity_override: 7,
        ..Default::default()
    };
    init_item_stats(&t, &mut FakeGame::default(), &mut it, Some(&mut rq), false).unwrap();
    let mut u = Seed::init_low(3);
    u.roll(30);
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 7);
    assert_eq!(it.unit_seed, u);
}

/// Armor (reference vector), weapon and other branches; missing record and the
/// defense check are fatal.
#[test]
fn record_branches() {
    let mut t = tables();
    // Armor: the reference vector.
    let mut r = item_rec(ty::TORS, b"qui ");
    r.durability = 24;
    r.minac = 3;
    r.maxac = 5;
    r.block = 7;
    r.speed = 5;
    let a = push_item(&mut t, r);
    let mut it = item(a, 1);
    it.unit_seed = Seed::new(1000, 666);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::TOBLOCK, 0), 7);
    assert_eq!(it.stats.base(stat::VELOCITYPERCENT, 0), -5);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 18);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 24);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 5);
    assert_eq!(it.unit_seed, Seed::new(2466107339, 165470233));
    // Defense above maxac (minac > maxac: n < 1 gives minac) is fatal.
    let mut r = item_rec(ty::HELM, b"cap ");
    r.minac = 6;
    r.maxac = 5;
    let bad = push_item(&mut t, r);
    let mut it = item(bad, 1);
    assert_eq!(
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false),
        Err(Fatal::Defense)
    );
    // Missing record (not gold, not a quiver): fatal 0x686.
    let mut it = Item::new(999, 101, FakeStats::default());
    assert_eq!(
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false),
        Err(Fatal::NoItemRecord)
    );

    // Weapon: stack roll, durability, damage (non-zero columns; throw
    // damage only with maxmisdam ≠ 0), attack rate.
    let mut r = item_rec(THROWN, b"tax ");
    r.stackable = 1;
    r.minstack = 5;
    r.maxstack = 20;
    r.durability = 31;
    r.mindam = 2;
    r.maxdam = 7;
    r.maxdam2 = 9;
    r.minmisdam = 4;
    r.speed = -10;
    let w = push_item(&mut t, r.clone());
    let mut it = item(w, 1);
    it.unit_seed = Seed::init_low(77);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    let mut u = Seed::init_low(77);
    let qn = u.roll(15) as i32 + 5;
    let dur = u.roll(15) as i32 + 15;
    assert_eq!(it.unit_seed, u);
    let b = &it.stats.base;
    assert_eq!(b[&(stat::QUANTITY, 0)], qn);
    assert_eq!(b[&(stat::DURABILITY, 0)], dur);
    assert_eq!(b[&(stat::MAXDURABILITY, 0)], 31);
    assert_eq!(b[&(stat::MINDAMAGE, 0)], 2);
    assert_eq!(b[&(stat::MAXDAMAGE, 0)], 7);
    assert_eq!(b[&(stat::SECONDARY_MAXDAMAGE, 0)], 9);
    assert!(!b.contains_key(&(stat::SECONDARY_MINDAMAGE, 0)));
    assert!(!b.contains_key(&(stat::THROW_MINDAMAGE, 0)));
    assert!(!b.contains_key(&(stat::THROW_MAXDAMAGE, 0)));
    assert_eq!(b[&(stat::ATTACKRATE, 0)], 10);
    r.maxmisdam = 6;
    r.minmisdam = 0;
    let w2 = push_item(&mut t, r.clone());
    let mut it = item(w2, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::THROW_MAXDAMAGE, 0), 6);
    assert!(!it.stats.base.contains_key(&(stat::THROW_MINDAMAGE, 0)));
    // Weapon stack: override; q = 0 → 1; magic or better → 1.
    let mut it = item(w, 1);
    let mut rq = ItemRequest {
        quantity_override: 3,
        ..Default::default()
    };
    init_item_stats(&t, &mut FakeGame::default(), &mut it, Some(&mut rq), false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 3);
    let mut it = item(w, 1);
    it.quality = q::MAGIC;
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 1);
    r.minstack = 0;
    r.maxstack = 0;
    let w0 = push_item(&mut t, r);
    let mut it = item(w0, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 1);

    // Other: lo = minstack, hi = spawnstack unless < lo or 0, then
    // max(lo, total max stack).
    let other = |t: &mut ItemTables, min: u32, spawn: u32, max: u32| {
        let mut r = item_rec(RING, b"key ");
        r.stackable = 1;
        r.minstack = min;
        r.spawnstack = spawn;
        r.maxstack = max;
        push_item(t, r)
    };
    for (min, spawn, max, n) in [
        (2, 6, 40, 4),
        (2, 0, 40, 38),
        (2, 1, 40, 38),
        (50, 0, 40, 0),
        (0, 0, 0, 0),
    ] {
        let o = other(&mut t, min, spawn, max);
        let mut it = item(o, 1);
        it.unit_seed = Seed::init_low(5);
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
        let mut u = Seed::init_low(5);
        let want = (u.roll(n) as i32 + min as i32).max(1);
        assert_eq!(
            it.stats.base(stat::QUANTITY, 0),
            want,
            "{min} {spawn} {max}"
        );
        assert_eq!(it.unit_seed, u);
    }
    let o = other(&mut t, 2, 6, 40);
    let mut it = item(o, 1);
    let mut rq = ItemRequest {
        quantity_override: 9,
        ..Default::default()
    };
    init_item_stats(&t, &mut FakeGame::default(), &mut it, Some(&mut rq), false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 9);
    let mut it = item(o, 1);
    it.quality = q::RARE;
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 1);
}

#[test]
fn variable_graphics() {
    let mut t = tables();
    t.itemtypes[RING as usize].varinvgfx = 3;
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    for seed in 0..20 {
        let mut it = item(i, 1);
        it.unit_seed = Seed::init_low(seed);
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
        let mut u = Seed::init_low(seed);
        assert_eq!(it.gfx, u.roll(3) as i32);
        assert_eq!(it.unit_seed, u);
        assert_eq!(it.item_seed, Seed::init_low(1));
    }
}

/// The dispatch runs only with "quest" and a request; quest difficulty
/// follows it even when it failed; its result is returned.
#[test]
fn quest_step_and_dispatch_result() {
    let mut t = tables();
    let mut r = item_rec(RING, b"qst ");
    r.quest = 1;
    r.questdiffcheck = 1;
    let i = push_item(&mut t, r.clone());
    let game = || FakeGame {
        difficulty: 2,
        ..Default::default()
    };
    // No request: no dispatch, result 1.
    let mut it = item(i, 1);
    assert_eq!(
        init_item_stats(&t, &mut game(), &mut it, None, true),
        Ok(true)
    );
    assert_eq!(it.quality, q::NONE);
    // "quest" false: no dispatch either.
    let mut rq = ItemRequest {
        quality: 10,
        ..Default::default()
    };
    let mut it = item(i, 1);
    assert_eq!(
        init_item_stats(&t, &mut game(), &mut it, Some(&mut rq), false),
        Ok(true)
    );
    assert_eq!(it.quality, q::NONE);
    // Failed dispatch: result 0, quest difficulty still written.
    let mut it = item(i, 1);
    assert_eq!(
        init_item_stats(&t, &mut game(), &mut it, Some(&mut rq), true),
        Ok(false)
    );
    assert_eq!(it.stats.item_list(stat::QUESTITEMDIFFICULTY, 0), 2);
    assert_ne!(it.flags & flag::IDENTIFIED, 0);
    // Success.
    rq.quality = q::NORMAL;
    let mut it = item(i, 1);
    assert_eq!(
        init_item_stats(&t, &mut game(), &mut it, Some(&mut rq), true),
        Ok(true)
    );
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!(it.stats.item_list(stat::QUESTITEMDIFFICULTY, 0), 2);
    // Quest without questdiffcheck: no quest difficulty.
    r.questdiffcheck = 0;
    let j = push_item(&mut t, r);
    let mut it = item(j, 1);
    init_item_stats(&t, &mut game(), &mut it, Some(&mut rq), true).unwrap();
    assert!(it.stats.lists.is_empty());
    assert_eq!(it.flags & flag::IDENTIFIED, 0);
}

/// A body part whose itemtype forces normal quality is handled by the
/// normal routine (file index := class).
#[test]
fn body_part_via_normal_routine() {
    let mut t = tables();
    t.itemtypes[ty::BODY as usize].normal = 1;
    let i = push_item(&mut t, item_rec(ty::BODY, b"hrt "));
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::MAGIC,
        unit: Some(RequestUnit {
            class: 17,
            player: None,
        }),
        ..Default::default()
    };
    let c = create_item(
        &t,
        &mut FakeGame::default(),
        &mut rq,
        false,
        FakeStats::default(),
        0,
    )
    .unwrap();
    assert_eq!(c.item.quality, q::NORMAL);
    assert_eq!(c.item.file_index, 17);
}

#[test]
fn normal_charm_runs_charm_affixes() {
    let mut t = tables();
    let c = push_item(&mut t, item_rec(ty::CHAR, b"cm1 "));
    // No affix rows: the charm routine finds none (fatal 0x372), proving
    // it ran; a ring does not run it.
    let mut it = item(c, 1);
    assert_eq!(
        normal(&t, &mut it, &ItemRequest::default()),
        Err(Fatal::Charm)
    );
    let mut t2 = tables();
    let r = push_item(&mut t2, item_rec(RING, b"rin "));
    let mut it = item(r, 1);
    assert_eq!(normal(&t2, &mut it, &ItemRequest::default()), Ok(()));
    // With a fitting suffix, the charm gets it.
    t.magic = vec![affix_row(ty::CHAR, 1)];
    t.n_suffix = 1;
    let mut it = item(c, 1);
    normal(&t, &mut it, &ItemRequest::default()).unwrap();
    assert_eq!(it.prefix[0] + it.suffix[0], 1);
}

#[test]
fn body_file_index() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::BODY, b"hrt "));
    let mut it = item(i, 1);
    let rq = ItemRequest {
        index: 5,
        unit: Some(RequestUnit {
            class: 33,
            player: None,
        }),
        ..Default::default()
    };
    normal(&t, &mut it, &rq).unwrap();
    assert_eq!(it.file_index, 33);
    assert_eq!(it.flags & flag::EAR, 0);
    let rq = ItemRequest {
        index: 5,
        ..Default::default()
    };
    let mut it = item(i, 1);
    normal(&t, &mut it, &rq).unwrap();
    assert_eq!(it.file_index, 5);
}

/// Scrolls and books: suffix slot 0 := the books row by spell code (no
/// row → the books count); the tests are independent and draw nothing.
#[test]
fn scroll_and_book_rows() {
    let mut t = tables();
    t.books = vec![
        (*b"tsc ", *b"tbk "),
        (*b"isc ", *b"ibk "),
        (*b"xxx ", *b"zbk "),
    ];
    let sc = push_item(&mut t, item_rec(ty::SCRO, b"isc "));
    let bk = push_item(&mut t, item_rec(ty::BOOK, b"zbk "));
    let none = push_item(&mut t, item_rec(ty::SCRO, b"ask "));
    let mut both = item_rec(ty::SCRO, b"tbk ");
    both.type2 = ty::BOOK as i16;
    let both = push_item(&mut t, both);
    let rq = ItemRequest::default();
    for (i, want) in [(sc, 1), (bk, 2), (none, 3), (both, 0)] {
        let mut it = item(i, 8);
        normal(&t, &mut it, &rq).unwrap();
        assert_eq!(it.suffix[0], want, "record {i}");
        assert_eq!(it.item_seed, Seed::init_low(8), "no draw");
    }
}

/// Class skill test tables: staff (staffmods 1), class 1's list starting
/// at skill 36, skill rows with mixed `itypea1`.
fn staff_tables() -> (ItemTables, usize) {
    let mut t = tables();
    t.itemtypes[ty::STAF as usize].staffmods = 1;
    let i = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.skill_lists.counts[1] = 30;
    t.skill_lists.max = 30;
    t.skill_lists.lists = vec![0; 7 * 30];
    t.skill_lists.lists[30] = 36;
    t.skills = (0..50)
        .map(|k| SkillRec {
            charclass: 0xFF,
            itypea1: match k % 3 {
                0 => ty::HELM as i16,
                1 => ty::STAF as i16,
                _ => 0,
            },
            reqlevel: 1,
            maxlvl: 20,
        })
        .collect();
    (t, i)
}

#[test]
fn normal_runs_class_skill_mods() {
    let (t, i) = staff_tables();
    let rq = ItemRequest {
        ilvl: 30,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    let mut a = item(i, 21);
    normal(&t, &mut a, &rq).unwrap();
    let mut b = item(i, 21);
    class_skill_mods(&t, &mut b, &rq);
    assert!(!a.stats.lists.is_empty());
    assert_eq!(a.stats, b.stats);
    assert_eq!(a.item_seed, b.item_seed);
}

/// Staffmods run after each routine's success (normal, superior, low,
/// magic, rare, crafted), not after a failure.
#[test]
fn staffmods_after_each_success() {
    let (mut t, i) = staff_tables();
    t.n_lowquality = 1;
    t.qualityitems = vec![QualityRec {
        mods: [crate::items::tables::PropRec::NONE; 2],
        staff: 1,
        ..Default::default()
    }];
    let mut magic: Vec<_> = (0..10).map(|k| affix_row(ty::STAF, k + 1)).collect();
    magic.extend((0..10).map(|k| affix_row(ty::STAF, k + 11)));
    t.magic = magic;
    t.n_suffix = 10;
    t.n_prefix = 10;
    let rare_row = RareRec {
        itype: [ty::STAF as i16, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    t.rare = vec![rare_row.clone(), rare_row];
    t.n_rare_suffix = 1;
    // Bonus ilvl 100: at least one mod always.
    let rq = ItemRequest {
        ilvl: 100,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    let has_mods = |it: &Item<FakeStats>| {
        it.stats
            .lists
            .get(&ListKey::ITEM)
            .is_some_and(|l| l.keys().any(|&(s, _)| s == stat::ITEM_SINGLESKILL))
    };
    let mut it = item(i, 5);
    normal(&t, &mut it, &rq).unwrap();
    assert!(has_mods(&it), "normal");
    let mut it = item(i, 5);
    it.quality = q::LOW;
    assert!(low_quality(&t, &mut it, &rq));
    assert!(has_mods(&it), "low");
    let mut it = item(i, 5);
    it.quality = q::SUPERIOR;
    assert!(superior(&t, &mut it, &rq));
    assert!(has_mods(&it), "superior");
    let mut it = item(i, 5);
    it.quality = q::MAGIC;
    assert!(crate::items::affixes::magic(&t, &mut it, &rq));
    assert!(has_mods(&it), "magic");
    let mut it = item(i, 5);
    it.quality = q::RARE;
    assert!(crate::items::affixes::rare(&t, &mut it, &rq));
    assert!(has_mods(&it), "rare");
    let mut it = item(i, 5);
    it.quality = q::CRAFTED;
    assert_eq!(crate::items::affixes::crafted(&t, &mut it, &rq), Ok(true));
    assert!(has_mods(&it), "crafted");
    // Failure: magic with no affix rows.
    let mut t2 = t.clone();
    t2.magic.clear();
    (t2.n_suffix, t2.n_prefix) = (0, 0);
    let mut it = item(i, 5);
    it.quality = q::MAGIC;
    assert!(!crate::items::affixes::magic(&t2, &mut it, &rq));
    assert!(!has_mods(&it), "failed magic");
}

#[test]
fn staffmods_stop_conditions() {
    let (mut t, i) = staff_tables();
    let rq = ItemRequest {
        ilvl: 100,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    // c ≥ 7 → stop, no draw.
    t.itemtypes[ty::STAF as usize].staffmods = 7;
    let mut it = item(i, 3);
    class_skill_mods(&t, &mut it, &rq);
    assert_eq!(it.item_seed, Seed::init_low(3));
    assert!(it.stats.lists.is_empty());
    // Skill count 0 → stop, no draw.
    t.itemtypes[ty::STAF as usize].staffmods = 2;
    let mut it = item(i, 3);
    class_skill_mods(&t, &mut it, &rq);
    assert_eq!(it.item_seed, Seed::init_low(3));
    assert!(it.stats.lists.is_empty());
    // first := entry 0 of the class's list: class 2 starting at 7.
    t.skill_lists.counts[2] = 30;
    t.skill_lists.lists[60] = 7;
    t.skills.clear();
    let mut it = item(i, 3);
    class_skill_mods(&t, &mut it, &rq);
    let l = &it.stats.lists[&ListKey::ITEM];
    // Tier ≤ 6 (base 5 + 1): skills in 7 .. 7 + 30.
    assert!(l
        .keys()
        .all(|&(s, k)| s == stat::ITEM_SINGLESKILL && (7..37).contains(&k)));
}

/// Model of the staff-mods roll (class skill bonus, tiers, skill picks)
/// for class list start `first`, written apart from the code under test.
fn staffmods_model(
    t: &ItemTables,
    rec: usize,
    first: u16,
    rq: &ItemRequest,
    format: u16,
    quality: u8,
    mut s: Seed,
) -> (Seed, BTreeMap<(u16, u16), i32>) {
    let mut out = BTreeMap::new();
    let pct = |s: &mut Seed| (s.step() % 100) as i32;
    let ilvl = rq.ilvl;
    let bonus = if rq.flags2 & 0x20 != 0 { rq.ilvl } else { 0 };
    let v = pct(&mut s) + bonus;
    let count = if v > 90 {
        3
    } else if v > 70 {
        2
    } else if v > 30 || bonus != 0 {
        1
    } else {
        return (s, out);
    };
    let base: i32 = if ilvl > 36 && format >= 100 {
        5
    } else if ilvl > 24 {
        4
    } else if ilvl > 18 {
        3
    } else if ilvl > 11 {
        2
    } else {
        1
    };
    let mut chosen = Vec::new();
    for _ in 0..count {
        let p = pct(&mut s);
        let mut tier = if p > 80 {
            base + 1
        } else if p > 30 {
            base
        } else if p > 10 {
            base - 1
        } else {
            base - 2
        };
        if tier < 1 {
            tier = 1;
        }
        if quality == 1 && tier > 3 {
            tier = 4;
        }
        let mut skill = 0u16;
        for _ in 0..6 {
            skill = first + (5 * (tier - 1)) as u16 + (s.step() % 5) as u16;
            let fits = match t.skills.get(usize::from(skill)) {
                None => true,
                Some(r) => r.itypea1 < 1 || t.is_type(rec, r.itypea1),
            };
            if fits && !chosen.contains(&skill) {
                chosen.push(skill);
                break;
            }
        }
        let value = if format < 100 || quality != 1 {
            let v = pct(&mut s) + bonus / 2;
            if v >= 90 {
                3
            } else if v >= 60 {
                2
            } else {
                1
            }
        } else {
            1
        };
        out.insert((stat::ITEM_SINGLESKILL, skill), value);
    }
    (s, out)
}

/// Bonus, tiers, skill acceptance and values against the model,
/// over ilvl thresholds, both formats, low and normal quality.
#[test]
fn staffmods_model_sweep() {
    let (t, i) = staff_tables();
    let mut runs = 0;
    let mut tiers_seen = std::collections::BTreeSet::new();
    for ilvl in [1, 12, 19, 25, 37, 99] {
        for flags2 in [0, req::STAFFMODS_ILVL] {
            for format in [2u16, 101] {
                for quality in [q::LOW, q::NORMAL] {
                    for seed in 0..40 {
                        let rq = ItemRequest {
                            ilvl,
                            flags2,
                            ..Default::default()
                        };
                        let mut it = item(i, seed);
                        it.format = format;
                        it.quality = quality;
                        class_skill_mods(&t, &mut it, &rq);
                        let (s, want) =
                            staffmods_model(&t, i, 36, &rq, format, quality, Seed::init_low(seed));
                        assert_eq!(it.item_seed, s, "{ilvl} {flags2} {format} {quality} {seed}");
                        let got = it
                            .stats
                            .lists
                            .get(&ListKey::ITEM)
                            .cloned()
                            .unwrap_or_default();
                        assert_eq!(got, want, "{ilvl} {flags2} {format} {quality} {seed}");
                        for &(_, k) in want.keys() {
                            tiers_seen.insert((k - 36) / 5 + 1);
                        }
                        runs += 1;
                    }
                }
            }
        }
    }
    assert_eq!(runs, 6 * 2 * 2 * 2 * 40);
    // Every tier 1–6 is exercised.
    assert_eq!(tiers_seen.len(), 6, "{tiers_seen:?}");
    // Expansion low quality: value 1 without the value draw.
    let rq = ItemRequest {
        ilvl: 99,
        flags2: req::STAFFMODS_ILVL,
        ..Default::default()
    };
    let mut it = item(i, 2);
    it.quality = q::LOW;
    class_skill_mods(&t, &mut it, &rq);
    assert!(it.stats.lists[&ListKey::ITEM].values().all(|&v| v == 1));
}

fn ethereal_axe(t: &mut ItemTables) -> usize {
    let mut r = item_rec(AXE, b"axe ");
    r.durability = 25;
    push_item(t, r)
}

fn durable(i: usize, seed: u32) -> Item<FakeStats> {
    let mut it = item(i, seed);
    it.quality = q::NORMAL;
    it.stats.set_base(stat::DURABILITY, 0, 25);
    it.stats.set_base(stat::MAXDURABILITY, 0, 25);
    it.stats.set_base(stat::MINDAMAGE, 0, 10);
    it
}

#[test]
fn ethereal_stop_conditions() {
    let mut t = tables();
    let axe = ethereal_axe(&mut t);
    let mut r = item_rec(RING, b"rin ");
    r.durability = 25;
    let ring = push_item(&mut t, r);
    let mut r = item_rec(AXE, b"axe ");
    r.durability = 25;
    r.nodurability = 1;
    let nodur = push_item(&mut t, r);
    let mut r = item_rec(AXE, b"axe ");
    r.durability = 25;
    r.quest = 1;
    let quest = push_item(&mut t, r);
    let always = ItemRequest {
        flags2: req::ALWAYS_ETHEREAL,
        ..Default::default()
    };
    let never = ItemRequest {
        flags2: req::ALWAYS_ETHEREAL | req::NEVER_ETHEREAL,
        ..Default::default()
    };
    let cases: Vec<(&str, usize, u8, &ItemRequest)> = vec![
        ("never", axe, q::NORMAL, &never),
        ("not weap/armo", ring, q::NORMAL, &always),
        ("no durability", nodur, q::NORMAL, &always),
        ("low", axe, q::LOW, &always),
        ("set", axe, q::SET, &always),
        ("quest", quest, q::NORMAL, &always),
    ];
    for (name, i, quality, rq) in cases {
        let mut it = durable(i, 4);
        it.quality = quality;
        ethereal_roll(&t, &mut it, rq);
        assert_eq!(it.item_seed, Seed::init_low(4), "{name}: no draw");
        assert_eq!(it.flags & flag::ETHEREAL, 0, "{name}");
    }
    let mut it = durable(axe, 4);
    ethereal_roll(&t, &mut it, &always);
    assert_ne!(it.flags & flag::ETHEREAL, 0, "control");
}

/// One roll(100); p < 5 applies; flags2 0x04 and flags1 0x400000 force.
#[test]
fn ethereal_roll_and_durability() {
    let mut t = tables();
    let axe = ethereal_axe(&mut t);
    let rq = ItemRequest::default();
    let hit = find_seed(|s| s.roll(100) < 5);
    let edge = find_seed(|s| s.roll(100) == 5);
    let mut it = durable(axe, hit);
    ethereal_roll(&t, &mut it, &rq);
    let mut s = Seed::init_low(hit);
    s.roll(100);
    assert_eq!(it.item_seed, s);
    assert_ne!(it.flags & flag::ETHEREAL, 0);
    assert_eq!(it.stats.base(stat::MINDAMAGE, 0), 15);
    // 25 / 2 + 1 = 13 for both.
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 13);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 13);
    let mut it = durable(axe, edge);
    ethereal_roll(&t, &mut it, &rq);
    let mut s = Seed::init_low(edge);
    s.roll(100);
    assert_eq!(it.item_seed, s);
    assert_eq!(it.flags & flag::ETHEREAL, 0);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 25);
    for rq in [
        ItemRequest {
            flags2: req::ALWAYS_ETHEREAL,
            ..Default::default()
        },
        ItemRequest {
            flags1: flag::ETHEREAL,
            ..Default::default()
        },
    ] {
        let mut it = durable(axe, edge);
        ethereal_roll(&t, &mut it, &rq);
        assert_eq!(it.item_seed, s, "the roll is still drawn");
        assert_ne!(it.flags & flag::ETHEREAL, 0);
        assert_eq!(it.stats.base(stat::DURABILITY, 0), 13);
    }
}

/// The ethereal roll runs from the finishing steps only for format ≥ 100.
#[test]
fn ethereal_only_expansion_format() {
    let mut t = tables();
    let axe = ethereal_axe(&mut t);
    for (format, want) in [(101u16, true), (2, false)] {
        let mut it = durable(axe, 6);
        it.format = format;
        let mut rq = ItemRequest {
            quality: q::NORMAL,
            flags2: req::ALWAYS_ETHEREAL,
            ..Default::default()
        };
        assert_eq!(
            dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
            Ok(true)
        );
        assert_eq!(it.flags & flag::ETHEREAL != 0, want, "format {format}");
    }
}

/// The socket roll runs from the finishing steps for qualities 1–3 only.
#[test]
fn sockets_only_low_normal_superior() {
    let mut t = tables();
    let i = socket_helm(&mut t);
    t.items[i].durability = 0;
    t.qualityitems = vec![QualityRec {
        mods: [crate::items::tables::PropRec::NONE; 2],
        armor: 1,
        ..Default::default()
    }];
    t.magic = vec![affix_row(ty::HELM, 1)];
    t.n_suffix = 1;
    for (quality, want) in [(q::NORMAL, true), (q::SUPERIOR, true), (q::MAGIC, false)] {
        let mut it = item(i, 6);
        let mut rq = ItemRequest {
            quality,
            flags2: req::ALWAYS_SOCKETS,
            ..Default::default()
        };
        assert_eq!(
            dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
            Ok(true)
        );
        assert_eq!(it.quality, quality);
        assert_eq!(it.flags & flag::SOCKETED != 0, want, "quality {quality}");
    }
}

#[test]
fn socket_stop_conditions() {
    let mut t = tables();
    let base = socket_helm(&mut t);
    let mut r = t.items[base].clone();
    r.hasinv = 0;
    let noinv = push_item(&mut t, r.clone());
    r.hasinv = 1;
    r.stackable = 1;
    let stack = push_item(&mut t, r.clone());
    r.stackable = 0;
    r.gemsockets = 0;
    let zero = push_item(&mut t, r);
    let rq = ItemRequest {
        flags2: req::ALWAYS_SOCKETS,
        ..Default::default()
    };
    let game = FakeGame::default();
    for (name, i, quality) in [
        ("low", base, q::LOW),
        ("no inventory", noinv, q::NORMAL),
        ("stackable", stack, q::NORMAL),
        ("max sockets 0", zero, q::NORMAL),
    ] {
        let mut it = item(i, 6);
        it.quality = quality;
        socket_roll(&t, &game, &mut it, &rq);
        assert_eq!(it.item_seed, Seed::init_low(6), "{name}: no draw");
        assert_eq!(it.flags & flag::SOCKETED, 0, "{name}");
    }
    let mut it = item(base, 6);
    it.quality = q::NORMAL;
    socket_roll(&t, &game, &mut it, &rq);
    assert_ne!(it.flags & flag::SOCKETED, 0, "control");
}

/// Difficulty caps 3 / 4 / 6: start seed 59 gives 59 mod m + 1 = m.
#[test]
fn socket_difficulty_caps() {
    let mut t = tables();
    let i = socket_helm(&mut t);
    let rq = ItemRequest {
        flags2: req::ALWAYS_SOCKETS,
        ..Default::default()
    };
    for (difficulty, want) in [(0u8, 3), (1, 4), (2, 6)] {
        let game = FakeGame {
            difficulty,
            ..Default::default()
        };
        let mut it = item(i, 6);
        it.start_seed = 59;
        it.quality = q::NORMAL;
        socket_roll(&t, &game, &mut it, &rq);
        assert_eq!(
            it.stats.base(stat::NUMSOCKETS, 0),
            want,
            "difficulty {difficulty}"
        );
    }
}
