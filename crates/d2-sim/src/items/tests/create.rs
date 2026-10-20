//! test vectors and edge cases.

use super::*;
use crate::items::create::{
    apply_ethereal, class_skill_mods, init_item_stats, max_sockets, socket_count, socket_roll,
    ELIXIRS,
};
use crate::items::tables::SkillRec;
use crate::items::{
    create_item, flag, q, req, stat, CreateError, ItemRequest, PlayerInfo, RequestUnit,
};

fn armor(t: &mut ItemTables) -> usize {
    let mut r = item_rec(ty::TORS, b"qui ");
    r.durability = 24;
    r.minac = 3;
    r.maxac = 5;
    r.block = 7;
    r.speed = 5;
    push_item(t, r)
}

#[test]
fn armor_durability_and_defense_vector() {
    let mut t = tables();
    let i = armor(&mut t);
    let mut it = item(i, 1);
    it.unit_seed = Seed::new(1000, 666);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 18);
    assert_eq!(it.stats.base(stat::MAXDURABILITY, 0), 24);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 5);
    assert_eq!(it.stats.base(stat::TOBLOCK, 0), 7);
    assert_eq!(it.stats.base(stat::VELOCITYPERCENT, 0), -5);
    assert_eq!(it.unit_seed, Seed::new(2466107339, 165470233));
}

#[test]
fn socket_count_from_start_seed() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.gemsockets = 4;
    r.hasinv = 1;
    r.invwidth = 2;
    r.invheight = 2;
    let i = push_item(&mut t, r);
    t.itemtypes[ty::HELM as usize].maxsock40 = 4;
    let game = FakeGame {
        difficulty: 2,
        ..Default::default()
    };
    let mut it = item(i, 7);
    it.start_seed = 0x1234_5679;
    it.quality = q::NORMAL;
    let rq = ItemRequest {
        flags2: req::ALWAYS_SOCKETS,
        ..Default::default()
    };
    socket_roll(&t, &game, &mut it, &rq);
    assert_eq!(0x1234_5679u32 % 4 + 1, 2);
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 2);
    assert_ne!(it.flags & flag::SOCKETED, 0);
}

fn sized(t: &mut ItemTables, w: u8, h: u8, sockets: u8) -> usize {
    let mut r = item_rec(ty::HELM, b"cap ");
    r.gemsockets = sockets;
    r.invwidth = w;
    r.invheight = h;
    t.itemtypes[ty::HELM as usize].maxsock1 = 6;
    t.itemtypes[ty::HELM as usize].maxsock25 = 6;
    t.itemtypes[ty::HELM as usize].maxsock40 = 6;
    push_item(t, r)
}

#[test]
fn socket_count_caps() {
    let mut t = tables();
    let i = sized(&mut t, 3, 4, 6);
    let mut it = item(i, 1);
    it.quality = q::SUPERIOR;
    socket_count(&t, &mut it, 9);
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 6);

    let j = sized(&mut t, 2, 2, 6);
    let mut it = item(j, 1);
    it.quality = q::MAGIC;
    socket_count(&t, &mut it, 6);
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 4);

    // Edge case 6: a set/unique item's present count replaces n.
    let mut it = item(i, 1);
    it.quality = q::UNIQUE;
    it.stats.set_base(stat::NUMSOCKETS, 0, 3);
    socket_count(&t, &mut it, 6);
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 3);
    let mut it = item(i, 1);
    it.quality = q::SET;
    socket_count(&t, &mut it, 6);
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 1);
}

#[test]
fn max_sockets_by_ilvl() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.gemsockets = 4;
    let i = push_item(&mut t, r);
    let h = &mut t.itemtypes[ty::HELM as usize];
    (h.maxsock1, h.maxsock25, h.maxsock40) = (3, 4, 6);
    for (ilvl, want) in [(25, 3), (26, 4), (41, 4)] {
        let mut it = item(i, 1);
        it.ilvl = ilvl;
        assert_eq!(max_sockets(&t, &it), want, "ilvl {ilvl}");
    }
}

#[test]
fn ethereal_apply_weapon() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(AXE, b"axe "));
    let mut it = item(i, 1);
    it.stats.set_base(stat::MINDAMAGE, 0, 10);
    it.stats.set_base(stat::MAXDAMAGE, 0, 21);
    apply_ethereal(&t, &mut it);
    assert_eq!(it.stats.base(stat::MINDAMAGE, 0), 15);
    assert_eq!(it.stats.base(stat::MAXDAMAGE, 0), 31);
    assert_ne!(it.flags & flag::ETHEREAL, 0);
}

/// Edge case 1: stack rolls exclude the maximum.
#[test]
fn stack_roll_excludes_max() {
    let mut t = tables();
    let mut r = item_rec(RING, b"key ");
    r.stackable = 1;
    r.minstack = 1;
    r.maxstack = 3;
    let i = push_item(&mut t, r);
    for s in 0..200 {
        let mut it = item(i, 1);
        it.unit_seed = Seed::init_low(s);
        let mut want = it.unit_seed;
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
        let qn = it.stats.base(stat::QUANTITY, 0);
        assert_eq!(qn, want.roll(2) as i32 + 1);
        assert!((1..3).contains(&qn));
    }
}

/// Edge cases 2 and 3: classic body armor gets no socket draw; the roll
/// draws even with "no sockets".
#[test]
fn socket_roll_draws() {
    let mut t = tables();
    let mut r = item_rec(ty::TORS, b"qui ");
    r.gemsockets = 3;
    r.hasinv = 1;
    r.invwidth = 2;
    r.invheight = 3;
    t.itemtypes[ty::TORS as usize].maxsock40 = 3;
    let i = push_item(&mut t, r);
    let game = FakeGame::default();
    let mut it = item(i, 99);
    it.quality = q::NORMAL;
    it.format = 2;
    socket_roll(&t, &game, &mut it, &ItemRequest::default());
    assert_eq!(it.item_seed, Seed::init_low(99));
    it.format = 101;
    let rq = ItemRequest {
        flags2: req::NO_SOCKETS,
        ..Default::default()
    };
    socket_roll(&t, &game, &mut it, &rq);
    let mut want = Seed::init_low(99);
    want.step();
    assert_eq!(it.item_seed, want);
    assert_eq!(it.flags & flag::SOCKETED, 0);
}

/// Edge case 7: no stat list → no durability → no ethereal draw.
#[test]
fn no_stat_list_no_durability() {
    let mut t = tables();
    let mut r = item_rec(AXE, b"axe ");
    r.durability = 10;
    let i = push_item(&mut t, r);
    let mut it = item(i, 5);
    it.quality = q::NORMAL;
    let rq = ItemRequest::default();
    crate::items::create::ethereal_roll(&t, &mut it, &rq);
    assert_eq!(it.item_seed, Seed::init_low(5));
    it.stats.set_base(stat::DURABILITY, 0, 5);
    crate::items::create::ethereal_roll(&t, &mut it, &rq);
    assert_ne!(it.item_seed, Seed::init_low(5));
}

/// Staffmods and edge case 5: after 6 rejected tries the last
/// tried skill is used.
#[test]
fn staffmods_last_rejected_skill() {
    let mut t = tables();
    t.itemtypes[ty::STAF as usize].staffmods = 1;
    let i = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.skill_lists.counts[1] = 30;
    t.skill_lists.max = 30;
    t.skill_lists.lists = vec![0; 7 * 30];
    t.skill_lists.lists[30] = 36;
    // Every skill demands itype "helm": all tries rejected.
    t.skills = vec![
        SkillRec {
            charclass: 0xFF,
            itypea1: ty::HELM as i16,
            reqlevel: 1,
            maxlvl: 20,
        };
        80
    ];
    let seed = find_seed(|s| (s.step() % 100) > 90);
    let mut it = item(i, seed);
    let rq = ItemRequest {
        ilvl: 1,
        ..Default::default()
    };
    class_skill_mods(&t, &mut it, &rq);
    // Replay the draws: count, then per mod tier pct, 6 tries, value pct.
    let mut s = Seed::init_low(seed);
    let v = (s.step() % 100) as i32;
    assert!(v > 90);
    let mut want = BTreeMap::new();
    for _ in 0..3 {
        let p = (s.step() % 100) as i32;
        let tr = if p > 80 { 2 } else { 1 };
        let mut skill = 0;
        for _ in 0..6 {
            skill = 36 + 5 * (tr - 1) + (s.step() % 5) as u16;
        }
        let pv = (s.step() % 100) as i32;
        let val = if pv >= 90 {
            3
        } else if pv >= 60 {
            2
        } else {
            1
        };
        want.insert((stat::ITEM_SINGLESKILL, skill), val);
    }
    assert_eq!(it.item_seed, s);
    assert_eq!(it.stats.lists[&ListKey::ITEM], want);
}

#[test]
fn elixir_table() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::ELIX, b"elx "));
    for seed in 0..50 {
        let mut it = item(i, seed);
        init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
        let mut s = Seed::init_low(seed);
        let k = ELIXIRS[s.roll(6) as usize];
        assert_eq!(it.file_index, k);
        let v = if k == 9 || k == 7 {
            ((s.step() & 3) as i32 + 1) * 256
        } else {
            1
        };
        assert_eq!(it.stats.base(stat::VALUE, 0), v);
        assert_eq!(it.item_seed, s);
    }
}

fn helm_tables() -> (ItemTables, usize) {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 12;
    r.minac = 3;
    r.maxac = 5;
    r.level = 1;
    let i = push_item(&mut t, r);
    (t, i)
}

/// The pipeline derives the seeds from the game seed, writes the
/// flags and is deterministic.
#[test]
fn pipeline_seeds_and_flags() {
    let (t, i) = helm_tables();
    let mut game = FakeGame::default();
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        ilvl: 0,
        quality: q::NORMAL,
        ..Default::default()
    };
    let c = create_item(&t, &mut game, &mut rq, false, FakeStats::default(), 100).unwrap();
    let mut g = Seed::init_low(12345);
    let unit = g.derive();
    let start = g.step();
    assert_eq!(game.seed, g);
    assert_eq!(rq.ilvl, 1);
    assert_eq!(c.item.ilvl, 1);
    assert_eq!(c.item.start_seed, start);
    assert_eq!(c.item.init_seed, unit.lo);
    assert_eq!(c.item.flags, flag::INIT | flag::INSTORE | flag::IDENTIFIED);
    assert_eq!(c.item.inv_page, 0xFF);
    assert_eq!(c.item.quality, q::NORMAL);
    assert_eq!(c.event3_at, None);
    let mut u = unit;
    let dur = u.roll(6) as i32 + 6;
    assert_eq!(c.item.stats.base(stat::DURABILITY, 0), dur);
    let mut game2 = FakeGame::default();
    let mut rq2 = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::NORMAL,
        ..Default::default()
    };
    let c2 = create_item(&t, &mut game2, &mut rq2, false, FakeStats::default(), 100).unwrap();
    assert_eq!(c, c2);
}

#[test]
fn pipeline_failures() {
    let (mut t, i) = helm_tables();
    let mut game = FakeGame::default();
    let mut rq = ItemRequest {
        item: 99,
        ..Default::default()
    };
    let e = create_item(&t, &mut game, &mut rq, false, FakeStats::default(), 0);
    assert_eq!(e.unwrap_err(), CreateError::BadIndex);
    assert_eq!(game.seed, Seed::init_low(12345));
    t.items[i].version = 100;
    game.expansion = false;
    rq.item = i as i32;
    let e = create_item(&t, &mut game, &mut rq, false, FakeStats::default(), 0);
    assert_eq!(e.unwrap_err(), CreateError::Classic);
}

#[test]
fn forced_request() {
    let (t, i) = helm_tables();
    let mut game = FakeGame::default();
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        force: true,
        quality: q::NORMAL,
        seed: 77,
        item_seed: 88,
        flags1: flag::IDENTIFIED | flag::ETHEREAL | flag::BROKEN,
        quantity: 4,
        min_dur: 300,
        max_dur: 400,
        ..Default::default()
    };
    let c = create_item(&t, &mut game, &mut rq, false, FakeStats::default(), 0).unwrap();
    let it = &c.item;
    assert_eq!(it.init_seed, 77);
    assert_eq!(it.start_seed, 88);
    assert_eq!(it.flags & flag::INSTORE, 0);
    assert_ne!(it.flags & flag::IDENTIFIED, 0);
    assert_ne!(it.flags & flag::BROKEN, 0);
    assert_eq!((rq.min_dur, rq.max_dur), (255, 255));
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 255);
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 4);
    // The base durability drew from the forced unit seed {77, 666}.
    let mut u = Seed::init_low(77);
    u.roll(6);
    let mut u2 = u;
    let _ = u2.roll_range(3, 3);
    assert_eq!(it.unit_seed, u2);
}

#[test]
fn ears() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::PLAY, b"ear "));
    let mut rq = ItemRequest {
        item: i as i32,
        format: 101,
        quality: q::NORMAL,
        unit: Some(RequestUnit {
            class: 3,
            player: Some(PlayerInfo {
                name: *b"bob\0\0\0\0\0\0\0\0\0\0\0\0\0",
                level: 42,
                hardcore: Some(true),
            }),
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
    assert_eq!(c.item.file_index, 3);
    assert_eq!(c.item.ear_level, 42);
    assert_eq!(&c.item.name[..3], b"bob");
    assert_eq!(
        c.item.flags & (flag::EAR | flag::NAMED),
        flag::EAR | flag::NAMED
    );
    rq.unit.as_mut().unwrap().player = None;
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
fn replenish_period() {
    let mut s = FakeStats::default();
    assert_eq!(crate::items::replenish_timer(&s, false, 10), None);
    s.set_base(stat::REPLENISH_QUANTITY, 0, 3);
    assert_eq!(
        crate::items::replenish_timer(&s, false, 10),
        Some(10 + 833 + 1)
    );
    s.set_base(stat::REPLENISH_DURABILITY, 0, 100);
    assert_eq!(
        crate::items::replenish_timer(&s, false, 10),
        Some(10 + 25 + 1)
    );
    assert_eq!(crate::items::replenish_timer(&s, true, 10), None);
}

#[test]
fn gold_amount() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::GOLD, b"gld "));
    let mut it = item(i, 1);
    it.ilvl = 10;
    it.unit_seed = Seed::init_low(5);
    let mut rq = ItemRequest::default();
    init_item_stats(&t, &mut FakeGame::default(), &mut it, Some(&mut rq), false).unwrap();
    let mut s = Seed::init_low(5);
    assert_eq!(it.stats.base(stat::GOLD, 0), s.roll(50) as i32 + 10);
    let mut it = item(i, 1);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::GOLD, 0), 1);
}
