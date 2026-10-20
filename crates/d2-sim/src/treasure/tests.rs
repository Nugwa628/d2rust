//! Synthetic vectors of the treasure rules, edge cases, and the table checks of
//! `treasure-quality.tsv` / `treasure-chest-acts.tsv` with their
//! perturbation tests. Real-data vectors are game-file checks (queued).

use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{
    Itemratio, Itemtypes, Levels, Monstats, Record, Setitems, Superuniques, Treasureclassex,
    Uniqueitems,
};

use super::quality::{step_chance, step_tsv, MfFactor, Ratio, Step};
use super::runtime::{FLAG_NOT_CLASSIC, FLAG_SET, FLAG_TC, FLAG_UNIQUE};
use super::*;
use crate::rng::Seed;

// ---------------------------------------------------------------- fixtures

pub(super) fn zeroed<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

pub(super) fn code(s: &str) -> [u8; 4] {
    let mut c = [b' '; 4];
    c[..s.len()].copy_from_slice(s.as_bytes());
    c
}

pub(super) fn cell<const N: usize>(s: &str) -> [u8; N] {
    let mut c = [0u8; N];
    c[..s.len()].copy_from_slice(s.as_bytes());
    c
}

pub(super) fn item(c: &str, ty: u16, level: u8) -> ItemData {
    ItemData {
        code: code(c),
        ubercode: code("xxx"),
        ultracode: code("yyy"),
        version: 100,
        level,
        type_: ty,
        type2: 0,
        unique: 0,
        quest: 0,
        spawnable: 1,
    }
}

/// itemtypes rows 0..=n: plain types with class 0xFF (not class
/// specific), rarity 3.
pub(super) fn itemtypes(n: usize) -> Vec<Itemtypes> {
    (0..n)
        .map(|_| {
            let mut t: Itemtypes = zeroed();
            t.class = 0xFF;
            t.rarity = 3;
            t
        })
        .collect()
}

/// Equivalence: identity plus column 0 (itemtypes rule) plus `extra`
/// pairs (i is of type j).
pub(super) fn equiv(n: usize, extra: &[(usize, usize)]) -> EquivMatrix {
    let words = n.div_ceil(32);
    let mut m = EquivMatrix {
        n,
        words,
        bits: vec![0; n * words],
    };
    let mut set = |i: usize, j: usize| m.bits[i * words + j / 32] |= 1 << (j % 32);
    for i in 0..n {
        set(i, 0);
        if i > 0 {
            set(i, i);
        }
    }
    for &(i, j) in extra {
        set(i, j);
    }
    m
}

/// The quality vector row.
pub(super) fn ratio_row() -> Itemratio {
    let mut r: Itemratio = zeroed();
    r.version = 1;
    (r.unique, r.uniquedivisor, r.uniquemin) = (400, 1, 6400);
    (r.set, r.setdivisor, r.setmin) = (160, 2, 5600);
    (r.rare, r.raredivisor, r.raremin) = (100, 2, 3200);
    (r.magic, r.magicdivisor, r.magicmin) = (34, 3, 192);
    (r.hiquality, r.hiqualitydivisor) = (12, 8);
    (r.normal, r.normaldivisor) = (2, 2);
    r
}

/// A TC from (id, prob, flags) entries, starts computed as the runtime form does.
pub(super) fn tc(picks: i32, nodrop: i32, entries: &[(u16, i32, u8)]) -> TreasureClass {
    let mut t = TreasureClass {
        name: b"t".to_vec(),
        group: 0,
        level: 0,
        total_classic: 0,
        total_expansion: 0,
        picks,
        nodrop,
        mods: [0; 6],
        entries: Vec::new(),
    };
    for &(id, p, flags) in entries {
        t.entries.push(TcEntry {
            start_classic: t.total_classic,
            start_expansion: t.total_expansion,
            id,
            row: 0,
            flags,
            mods: [0; 6],
        });
        t.total_expansion += p;
        if flags & FLAG_NOT_CLASSIC == 0 {
            t.total_classic += p;
        }
    }
    t
}

pub(super) fn classes(tcs: Vec<TreasureClass>) -> TreasureClasses {
    let mut all = vec![tc(1, 0, &[])];
    all.extend(tcs);
    TreasureClasses {
        tcs: all,
        group_offset: 0,
        chest: [None; 45],
        notes: Vec::new(),
    }
}

pub(super) struct World {
    pub(super) tcs: TreasureClasses,
    pub(super) items: Vec<ItemData>,
    pub(super) itemtypes: Vec<Itemtypes>,
    pub(super) equiv: EquivMatrix,
    pub(super) ratio: Vec<Itemratio>,
}

impl World {
    pub(super) fn new(tcs: Vec<TreasureClass>, items: Vec<ItemData>) -> Self {
        World {
            tcs: classes(tcs),
            items,
            itemtypes: itemtypes(60),
            equiv: equiv(60, &[]),
            ratio: vec![ratio_row()],
        }
    }

    pub(super) fn data(&self) -> TreasureData<'_> {
        TreasureData {
            tcs: &self.tcs,
            items: &self.items,
            itemtypes: &self.itemtypes,
            equiv: &self.equiv,
            itemratio: &self.ratio,
        }
    }
}

/// Items `a`–`d` (ids 0–3) of type 10.
pub(super) fn abcd() -> Vec<ItemData> {
    ["a", "b", "c", "d"]
        .iter()
        .map(|c| item(c, 10, 1))
        .collect()
}

#[derive(Default)]
pub(super) struct Sink {
    pub(super) reqs: Vec<DropRequest<()>>,
    pub(super) gold: Vec<i32>,
    /// Gold base given to every created item.
    pub(super) base_gold: i32,
    pub(super) no_spot: bool,
}

impl DropSink for Sink {
    type Spot = ();
    type Item = usize;
    fn place(&mut self, _x: i32, _y: i32) -> Option<()> {
        (!self.no_spot).then_some(())
    }
    fn create(&mut self, req: DropRequest<()>) -> Option<usize> {
        self.reqs.push(req);
        self.gold.push(self.base_gold);
        Some(self.reqs.len() - 1)
    }
    fn gold(&self, item: usize) -> i32 {
        self.gold[item]
    }
    fn set_gold(&mut self, item: usize, value: i32) {
        self.gold[item] = value;
    }
}

pub(super) fn game(expansion: bool, players: i32) -> GameFacts {
    GameFacts {
        expansion,
        difficulty: 0,
        game_type: 0,
        living_players: players,
        players_setting: 0,
        item_format: 0,
    }
}

pub(super) const OBJECT: Dropper = Dropper {
    kind: DropperKind::Other { area_level: 5 },
    x: 10,
    y: 20,
};

pub(super) fn args(tc: u16, quality: u8) -> WalkArgs {
    WalkArgs {
        tc: Some(tc),
        quality,
        level: 1,
        find_item: false,
        list: false,
        max: 0,
    }
}

pub(super) fn h2h() -> TreasureClass {
    // `Act 1 H2H A` shape: picks 2, nodrop 100, a 21, b 16, c 21, d 2.
    tc(2, 100, &[(0, 21, 0), (1, 16, 0), (2, 21, 0), (3, 2, 0)])
}

// ------------------------------------------------------------- walk

#[test]
fn walk_vector_nodrop_then_c() {
    let w = World::new(vec![h2h()], abcd());
    let mut seed = Seed::new(12345, 666);
    let mut sink = Sink::default();
    let out = walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(sink.reqs[0].id, 2, "c");
    assert_eq!(sink.reqs[0].quality, 4);
    assert_eq!(seed, Seed::new(2_337_785_264, 9_490_055));
}

#[test]
fn walk_vector_three_players() {
    let w = World::new(vec![h2h()], abcd());
    let mut seed = Seed::new(12345, 666);
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 3),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert_eq!(player_factor(&game(true, 3), &OBJECT, None), 2);
    assert_eq!(nodrop(100, 60, 2), Ok(38));
    assert_eq!(sink.reqs.len(), 1);
    assert_eq!(sink.reqs[0].id, 1, "b");
}

#[test]
fn search_vectors() {
    let t = h2h();
    for (r, want) in [(37, 2), (0, 0), (59, 3), (20, 0), (21, 1), (58, 3)] {
        assert_eq!(select_entry(&t, r, true), Some(want), "expansion r {r}");
        assert_eq!(select_entry(&t, r, false), Some(want), "classic r {r}");
    }
}

#[test]
fn classic_search_skips_expansion_entries() {
    // Entry 1 is expansion only: classic starts 0, 21, 21, 42.
    let t = tc(
        1,
        0,
        &[(0, 21, 0), (1, 16, FLAG_NOT_CLASSIC), (2, 21, 0), (3, 2, 0)],
    );
    assert_eq!(t.total_classic, 44);
    assert_eq!(select_entry(&t, 20, false), Some(0));
    assert_eq!(select_entry(&t, 21, false), Some(2));
    assert_eq!(select_entry(&t, 43, false), Some(3));
    // Only expansion entries: none.
    let t = tc(1, 0, &[(0, 5, FLAG_NOT_CLASSIC)]);
    assert_eq!(select_entry(&t, 0, false), None);
}

#[test]
fn nodrop_vectors() {
    let got: Vec<i32> = (2..=8).map(|n| nodrop(100, 60, n).unwrap()).collect();
    assert_eq!(got, [38, 19, 10, 6, 3, 2, 1]);
    let got: Vec<i32> = (2..=8).map(|n| nodrop(19, 81, n).unwrap()).collect();
    assert_eq!(got, [3, 0, 0, 0, 0, 0, 0]);
    assert_eq!(nodrop(100, 60, 1), Ok(100));
    assert_eq!(nodrop(100, 60, -3), Ok(100));
}

#[test]
fn nodrop_matches_exact_rational_on_vectors() {
    // C·n0^n / ((n0 + C)^n − n0^n), floored, on the vector pairs.
    for (n0, c) in [(100i128, 60i128), (19, 81)] {
        for n in 2..=8u32 {
            let want = c * n0.pow(n) / ((n0 + c).pow(n) - n0.pow(n));
            assert_eq!(
                i128::from(nodrop(n0 as i32, c as i32, n as i32).unwrap()),
                want,
                "{n0} {c} {n}"
            );
        }
    }
}

#[test]
fn player_factor_rules() {
    let party = |k| Recipient {
        party: Some(k),
        magic_find: 0,
        gold_find: 0,
    };
    // R absent: 1 + (P − 1) / 2 (Edge case 5).
    assert_eq!(player_factor(&game(true, 8), &OBJECT, None), 4);
    // Party of 3 in an 8-player game: 3 + 5 / 2.
    assert_eq!(player_factor(&game(true, 8), &OBJECT, Some(&party(3))), 5);
    // Party size caps at 8; a party of 1 counts as 1.
    assert_eq!(
        player_factor(&game(true, 1), &OBJECT, Some(&party(12))),
        8 + (1 - 8) / 2
    );
    assert_eq!(player_factor(&game(true, 3), &OBJECT, Some(&party(1))), 2);
    // The players setting applies for game types 1–3 only.
    let mut g = game(true, 1);
    g.players_setting = 8;
    assert_eq!(player_factor(&g, &OBJECT, None), 1);
    g.game_type = 3;
    assert_eq!(player_factor(&g, &OBJECT, None), 4);
    // Monsters cap at max(stat 100, 1).
    let m = |pc| Dropper {
        kind: DropperKind::Monster {
            class: 1,
            level: 10,
            playercount: pc,
        },
        x: 0,
        y: 0,
    };
    assert_eq!(player_factor(&g, &m(2), None), 2);
    assert_eq!(player_factor(&g, &m(0), None), 1);
}

#[test]
fn negative_picks_draw_nothing() {
    let w = World::new(vec![tc(-2, 0, &[(0, 1, 0), (1, 2, 0)])], abcd());
    let mut seed = Seed::new(12345, 666);
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    let ids: Vec<u16> = sink.reqs.iter().map(|r| r.id).collect();
    assert_eq!(ids, [0, 1]);
    assert_eq!(seed, Seed::new(12345, 666));
}

#[test]
fn negative_picks_end_when_r_reaches_total() {
    // picks −5 over total 3: r = 0, 1, 2, then 3 ≥ 3 ends the slot.
    let w = World::new(vec![tc(-5, 0, &[(0, 1, 0), (1, 2, 0)])], abcd());
    let mut seed = Seed::default();
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    let ids: Vec<u16> = sink.reqs.iter().map(|r| r.id).collect();
    assert_eq!(ids, [0, 1, 1]);
}

#[test]
fn find_item_ignores_nodrop() {
    let w = World::new(vec![h2h()], abcd());
    let mut seed = Seed::new(12345, 666);
    let mut sink = Sink::default();
    let mut a = args(1, 4);
    a.find_item = true;
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed,
        None,
        &a,
        &mut sink,
    )
    .unwrap();
    // roll(60): 22752887 % 60 = 47 → c; 2337785264 % 60 = 44 → c.
    let ids: Vec<u16> = sink.reqs.iter().map(|r| r.id).collect();
    assert_eq!(ids, [2, 2]);
}

#[test]
fn sub_tc_slots_and_mods() {
    // TC 1 picks 2 → TC 2 (picks 1, magic mod 900) each time; TC 1 mod
    // magic 1000 wins (unsigned max), rare from TC 2.
    let mut top = tc(2, 0, &[(2, 1, FLAG_TC)]);
    top.mods = [1000, 0, 0, 0, 0, 0];
    let mut sub = tc(1, 0, &[(0, 1, 0)]);
    sub.mods = [900, 7, 0, 0, 0, 0];
    let w = World::new(vec![top, sub], abcd());
    let mut seed = Seed::default();
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.reqs.len(), 2);
    // 4 draws: two picks in TC 1, one in each TC 2 slot.
    let mut s = Seed::default();
    for _ in 0..4 {
        s.step();
    }
    assert_eq!(seed, s);
}

#[test]
fn slot_overflow_is_fatal() {
    // TC 1 picks 2 → itself: each descent adds a slot.
    let w = World::new(vec![tc(2, 0, &[(1, 1, FLAG_TC)])], abcd());
    let mut sink = Sink::default();
    let r = walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &args(1, 4),
        &mut sink,
    );
    assert_eq!(r, Err(TreasureError::SlotOverflow));
}

#[test]
fn max_rules() {
    let w = World::new(vec![tc(8, 0, &[(0, 1, 0)])], abcd());
    let run = |list, max| {
        let mut sink = Sink::default();
        let a = WalkArgs {
            list,
            max,
            ..args(1, 4)
        };
        walk(
            &w.data(),
            &game(true, 1),
            &OBJECT,
            &mut Seed::default(),
            None,
            &a,
            &mut sink,
        )
        .map(|v| v.len())
    };
    assert_eq!(run(false, 0), Ok(6));
    assert_eq!(run(false, -1), Ok(6));
    assert_eq!(run(false, 3), Ok(3));
    assert_eq!(run(true, 2), Ok(2));
    assert_eq!(run(true, 0), Err(TreasureError::ZeroMax));
    let mut a = args(1, 4);
    a.tc = None;
    let r = walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &a,
        &mut Sink::default(),
    );
    assert_eq!(r, Err(TreasureError::NoTc));
}

#[test]
fn unique_and_set_entries() {
    let w = World::new(
        vec![tc(
            -2,
            0,
            &[
                (0, 1, FLAG_UNIQUE | FLAG_NOT_CLASSIC),
                (1, 1, FLAG_SET | FLAG_NOT_CLASSIC),
            ],
        )],
        abcd(),
    );
    let mut w = w;
    w.tcs.tcs[1].entries[0].row = 381;
    w.tcs.tcs[1].entries[1].row = 4;
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &args(1, 0),
        &mut sink,
    )
    .unwrap();
    assert_eq!((sink.reqs[0].quality, sink.reqs[0].index), (7, 382));
    assert_eq!((sink.reqs[1].quality, sink.reqs[1].index), (5, 5));
}

#[test]
fn missing_item_id_skips() {
    let w = World::new(vec![tc(-1, 0, &[(0xFFFF, 1, 0)])], abcd());
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert!(sink.reqs.is_empty());
}

#[test]
fn drop_flag_draws() {
    let mut t = tc(-1, 0, &[(0, 1, 0)]);
    t.mods[4] = 0x400; // always below
    t.mods[5] = 1; // only when lo' & 0x3FF = 0
    let w = World::new(vec![t], abcd());
    let mut seed = Seed::default();
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    // Two steps from {1, 666}: 1791398751 & 0x3FF = 0x35F, 791599131 & 0x3FF ≠ 0.
    assert_eq!(sink.reqs[0].drop_flags, 0x04);
    let mut s = Seed::default();
    s.step();
    s.step();
    assert_eq!(seed, s);
}

#[test]
fn creation_request_fields() {
    let w = World::new(vec![tc(-1, 0, &[(0, 1, 0)])], abcd());
    let bovine = Dropper {
        kind: DropperKind::Monster {
            class: 391,
            level: 0,
            playercount: 1,
        },
        x: 0,
        y: 0,
    };
    let mut g = game(true, 1);
    g.item_format = 7;
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &g,
        &bovine,
        &mut Seed::default(),
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    let r = &sink.reqs[0];
    assert_eq!(
        (
            r.drop_flags,
            r.item_level,
            r.spawn_type,
            r.init_flags,
            r.item_format
        ),
        (0x01, 1, 3, 1, 7)
    );
    assert_eq!(item_level(&OBJECT), 5);
    assert_eq!(
        item_level(&Dropper {
            kind: DropperKind::None,
            x: 0,
            y: 0
        }),
        1
    );
    assert_eq!(
        item_level(&Dropper {
            kind: DropperKind::Player { level: 40 },
            x: 0,
            y: 0
        }),
        40
    );
    // No free spot: nothing created, the walk goes on.
    let mut sink = Sink {
        no_spot: true,
        ..Sink::default()
    };
    let out = walk(
        &w.data(),
        &g,
        &OBJECT,
        &mut Seed::default(),
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert!(out.is_empty());
}

use super::walk::item_level;

// ------------------------------------------------------- classic

#[test]
fn classic_skips_expansion_items_and_raises_mods() {
    let mut items = abcd();
    items[0].version = 0;
    items[1].version = 100;
    let mut t = tc(-2, 0, &[(0, 1, 0), (1, 1, 0)]);
    t.entries[0].mods = [0, 0, 0, 0, 0x400, 0];
    let w = World::new(vec![t], items);
    let mut seed = Seed::default();
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(false, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    // Item 0 with the raised slot mod 5 (one draw, flag 0x04); item 1 is
    // expansion only in a classic game.
    assert_eq!(sink.reqs.len(), 1);
    assert_eq!(sink.reqs[0].drop_flags, 0x04);
    // Expansion games ignore entry mods.
    let mut seed2 = Seed::default();
    let mut sink = Sink::default();
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut seed2,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.reqs[0].drop_flags, 0);
    assert_eq!(seed2, Seed::default());
}

#[test]
fn classic_throwables_repick_then_long_sword() {
    let mut items = abcd();
    items.push(item("lsd", 10, 1)); // id 4
    for i in &mut items {
        i.version = 0;
    }
    items[0].type_ = 11;
    let mut w = World::new(vec![tc(1, 0, &[(0, 1, 0)])], items);
    w.itemtypes[11].throwable = 1;
    let mut sink = Sink::default();
    let mut seed = Seed::default();
    walk(
        &w.data(),
        &game(false, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    // 10 re-picks, then the 11th pick becomes `lsd`.
    assert_eq!(sink.reqs.len(), 1);
    assert_eq!(sink.reqs[0].id, 4);
    let mut s = Seed::default();
    for _ in 0..11 {
        s.roll(1);
    }
    assert_eq!(seed, s);
}

// ------------------------------------------------------------ gold

#[test]
fn gold_multiplier_and_gold_find() {
    let mut items = abcd();
    items[0].type_ = 4;
    let mut t = tc(-1, 0, &[(0, 1, 0)]);
    t.entries[0].row = 1280;
    let w = World::new(vec![t], items);
    let r = Recipient {
        party: None,
        magic_find: 0,
        gold_find: 50,
    };
    let mut sink = Sink {
        base_gold: 100,
        ..Sink::default()
    };
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        Some(&r),
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    // 100 × 1280 >> 8 = 500; × 150 / 100 = 750.
    assert_eq!(sink.gold[0], 750);
    // Negative gold find stores 0.
    let r = Recipient {
        gold_find: -300,
        ..r
    };
    let mut sink = Sink {
        base_gold: 100,
        ..Sink::default()
    };
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        Some(&r),
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.gold[0], 0);
}

#[test]
fn sixth_item_gets_no_gold_find() {
    let mut items = abcd();
    items[0].type_ = 4;
    let w = World::new(vec![tc(6, 0, &[(0, 1, 0)])], items);
    let r = Recipient {
        party: None,
        magic_find: 0,
        gold_find: 100,
    };
    let mut sink = Sink {
        base_gold: 10,
        ..Sink::default()
    };
    walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        Some(&r),
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.gold, [20, 20, 20, 20, 20, 10]);
}

#[test]
fn gold_base_rules() {
    let mut s = Seed::default();
    // roll(50) from {1, 666}: 1791398751 % 50 = 1, + 10.
    assert_eq!(gold_base(&mut s, 10, 0), 11);
    assert_eq!(gold_base(&mut Seed::default(), 0, 0), 1);
    assert_eq!(gold_base(&mut Seed::default(), 10, 77), 77);
}

// ---------------------------------------------------------- quality

fn quality_world(rare: bool, magic: bool) -> World {
    let mut w = World::new(vec![], vec![item("q", 10, 30)]);
    w.itemtypes[10].rare = u8::from(rare);
    w.itemtypes[10].magic = u8::from(magic);
    w
}

#[test]
fn quality_vector() {
    let w = quality_world(true, false);
    let mut seed = Seed::new(12345, 666);
    let q = roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed).unwrap();
    assert_eq!(q, 2);
    // Replay: the five rolls and their ranges.
    let mut s = Seed::new(12345, 666);
    let rolls: Vec<u32> = [48640, 19200, 11520, 3584, 1280]
        .iter()
        .map(|&n| s.roll(n))
        .collect();
    assert_eq!(rolls, [38007, 12464, 2551, 1693, 1048]);
    assert_eq!(seed, s);
    let row = ratio_row();
    let normal = &LADDER[6];
    assert_eq!(
        step_chance(normal, Ratio::Normal, &row, 20, 0, &[0; 6]),
        Ok(-1024)
    );
}

#[test]
fn quality_vector_magic_find() {
    let row = ratio_row();
    assert_eq!(quality::mf_divisor(MfFactor::Factor(250), 100), Some(171));
    assert_eq!(quality::mf_divisor(MfFactor::Factor(500), 100), Some(183));
    assert_eq!(quality::mf_divisor(MfFactor::Factor(600), 100), Some(185));
    assert_eq!(quality::mf_divisor(MfFactor::Linear, 100), Some(200));
    assert_eq!(quality::mf_divisor(MfFactor::Factor(250), 0), None);
    // M + 100 ≤ 110: the linear divisor.
    assert_eq!(quality::mf_divisor(MfFactor::Factor(250), 10), Some(110));
    assert_eq!(
        step_chance(&LADDER[0], Ratio::Unique, &row, 20, 100, &[0; 6]),
        Ok(28444)
    );
    assert_eq!(
        step_chance(&LADDER[1], Ratio::Set, &row, 20, 100, &[0; 6]),
        Ok(10491)
    );
    assert_eq!(
        step_chance(&LADDER[4], Ratio::Magic, &row, 20, 100, &[0; 6]),
        Ok(1792)
    );
}

#[test]
fn quality_slot_mod_full_magic() {
    // Not rare: unique and set draw and fail, magic chance 0 → 4.
    let w = quality_world(false, false);
    let mut seed = Seed::new(12345, 666);
    let q = roll_quality(&w.data(), 0, 50, 0, &[1024, 0, 0, 0, 0, 0], &mut seed).unwrap();
    assert_eq!(q, 4);
    let mut s = Seed::new(12345, 666);
    s.step();
    s.step();
    assert_eq!(seed, s);
}

#[test]
fn quality_gates_and_shortcuts() {
    // itemtypes magic: unique and set draw, rare skipped, magic step 4
    // returns without a draw.
    let w = quality_world(false, true);
    let mut seed = Seed::new(12345, 666);
    assert_eq!(roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed), Ok(4));
    // normal type → 2; unique item → 7; magic + quest → 7; no draws.
    let mut w = quality_world(false, false);
    w.itemtypes[10].normal = 1;
    let mut seed = Seed::default();
    assert_eq!(roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed), Ok(2));
    let mut w = quality_world(false, false);
    w.items[0].unique = 1;
    assert_eq!(roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed), Ok(7));
    let mut w = quality_world(false, true);
    w.items[0].quest = 1;
    assert_eq!(roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed), Ok(7));
    assert_eq!(seed, Seed::default());
    // Missing item → 0.
    assert_eq!(roll_quality(&w.data(), 9, 50, 0, &[0; 6], &mut seed), Ok(0));
}

#[test]
fn quality_mf_floor_skips_to_superior() {
    let w = quality_world(true, false);
    let mut seed = Seed::new(12345, 666);
    let q = roll_quality(&w.data(), 0, 50, -100, &[0; 6], &mut seed).unwrap();
    // Superior roll(1280) = 22752887 % 1280 = 247 ≥ 128; normal ≤ 0 → 2.
    assert_eq!(q, 2);
    let mut s = Seed::new(12345, 666);
    s.step();
    assert_eq!(seed, s);
}

#[test]
fn ratio_row_selection() {
    let mk = |v: u16, cs: u8, uber: u8, unique: u32| {
        let mut r: Itemratio = zeroed();
        (r.version, r.class_specific, r.uber, r.unique) = (v, cs, uber, unique);
        r
    };
    let rows = [
        mk(0, 0, 0, 1),
        mk(1, 0, 0, 2),
        mk(1, 0, 0, 3),
        mk(101, 0, 0, 4),
        mk(1, 1, 0, 5),
        mk(1, 0, 1, 6),
    ];
    assert_eq!(
        quality::ratio_row(&rows, false, false).map(|r| r.unique),
        Some(3)
    );
    assert_eq!(
        quality::ratio_row(&rows, true, false).map(|r| r.unique),
        Some(5)
    );
    assert_eq!(
        quality::ratio_row(&rows, false, true).map(|r| r.unique),
        Some(6)
    );
    assert!(quality::ratio_row(&rows, true, true).is_none());
    let mut w = quality_world(false, false);
    w.ratio.clear();
    assert_eq!(
        roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut Seed::default()),
        Err(TreasureError::NoRatioRow)
    );
}

#[test]
fn quality_uber_and_class_rows() {
    // weap item with code = ubercode reads the uber row.
    let mut w = quality_world(false, false);
    w.equiv = equiv(60, &[(10, 45)]);
    w.items[0].ubercode = w.items[0].code;
    let mut uber = ratio_row();
    uber.uber = 1;
    uber.unique = 1000;
    w.ratio = vec![uber];
    assert!(roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut Seed::default()).is_ok());
    w.items[0].quest = 1; // quest items are never uber
    assert_eq!(
        roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut Seed::default()),
        Err(TreasureError::NoRatioRow)
    );
}

// ---------------------------------------------------- TSV checks

const QUALITY_TSV: &str = include_str!("../../../../tables/items/treasure-quality.tsv");
const CHEST_TSV: &str = include_str!("../../../../tables/items/treasure-chest-acts.tsv");

/// Differences between `treasure-quality.tsv` and [`LADDER`], one line
/// each.
fn check_quality_tsv(tsv: &str, ladder: &[Step]) -> Vec<String> {
    let mut lines = tsv.lines();
    let mut errs = Vec::new();
    if lines.next()
        != Some("step\tresult\tgate\tratio\tdivisor\tmin\tmf_factor\ttc_slot\tdraw_when\ton_pass")
    {
        errs.push("header".to_string());
    }
    let rows: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if rows.len() != ladder.len() {
        errs.push(format!("{} rows, {} steps", rows.len(), ladder.len()));
    }
    for (i, (row, step)) in rows.iter().zip(ladder).enumerate() {
        let want = format!("{}\t{}", i + 1, step_tsv(step));
        if *row != want {
            errs.push(format!("step {}: tsv {row:?} code {want:?}", i + 1));
        }
    }
    errs
}

fn check_chest_tsv(tsv: &str, acts: &[(i32, i32)]) -> Vec<String> {
    let mut lines = tsv.lines();
    let mut errs = Vec::new();
    if lines.next() != Some("act\tfirst_level\tlast_level") {
        errs.push("header".to_string());
    }
    let rows: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if rows.len() != acts.len() {
        errs.push(format!("{} rows, {} acts", rows.len(), acts.len()));
    }
    for (i, (row, (a, b))) in rows.iter().zip(acts).enumerate() {
        let want = format!("{i}\t{a}\t{b}");
        if *row != want {
            errs.push(format!("act {i}: tsv {row:?} code {want:?}"));
        }
    }
    errs
}

#[test]
fn ladder_matches_tsv() {
    assert_eq!(
        check_quality_tsv(QUALITY_TSV, &LADDER),
        Vec::<String>::new()
    );
}

#[test]
fn chest_acts_match_tsv() {
    assert_eq!(
        check_chest_tsv(CHEST_TSV, &CHEST_ACTS),
        Vec::<String>::new()
    );
}

#[test]
fn tsv_checks_catch_perturbations() {
    // Change one cell, expect exactly that row reported.
    let bad = QUALITY_TSV.replace(
        "2\t5\talways\tSet\tSetDivisor\tSetMin\t500",
        "2\t5\talways\tSet\tSetDivisor\tSetMin\t501",
    );
    assert_ne!(bad, QUALITY_TSV);
    let errs = check_quality_tsv(&bad, &LADDER);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].starts_with("step 2:"), "{errs:?}");
    let mut ladder = LADDER;
    ladder[2].tc_slot = Some(0);
    let errs = check_quality_tsv(QUALITY_TSV, &ladder);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].starts_with("step 3:"), "{errs:?}");

    let bad = CHEST_TSV.replace("4\t109\t136", "4\t109\t132");
    let errs = check_chest_tsv(&bad, &CHEST_ACTS);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].starts_with("act 4:"), "{errs:?}");
}

// --------------------------------------------------------- runtime

pub(super) struct Src {
    pub(super) tcx: Vec<Treasureclassex>,
    pub(super) itemtypes: Vec<Itemtypes>,
    pub(super) items: Vec<ItemData>,
    pub(super) equiv: EquivMatrix,
    pub(super) uniques: Vec<Uniqueitems>,
    pub(super) sets: Vec<Setitems>,
}

impl Src {
    pub(super) fn build(&self) -> Result<TreasureClasses, TreasureError> {
        TreasureClasses::build(&TcSources {
            treasureclassex: &self.tcx,
            itemtypes: &self.itemtypes,
            items: &self.items,
            equiv: &self.equiv,
            uniqueitems: &self.uniques,
            setitems: &self.sets,
        })
    }
}

pub(super) fn tcx(name: &str, picks: u32, nodrop: u32, items: &[(&str, u32)]) -> Treasureclassex {
    let mut r: Treasureclassex = zeroed();
    r.treasure_class = cell(name);
    r.picks = picks;
    r.nodrop = nodrop;
    let slots: [(&mut [u8; 64], &mut u32); 10] = [
        (&mut r.item1, &mut r.prob1),
        (&mut r.item2, &mut r.prob2),
        (&mut r.item3, &mut r.prob3),
        (&mut r.item4, &mut r.prob4),
        (&mut r.item5, &mut r.prob5),
        (&mut r.item6, &mut r.prob6),
        (&mut r.item7, &mut r.prob7),
        (&mut r.item8, &mut r.prob8),
        (&mut r.item9, &mut r.prob9),
        (&mut r.item10, &mut r.prob10),
    ];
    for ((c, p), &(s, prob)) in slots.into_iter().zip(items) {
        *c = cell(s);
        *p = prob;
    }
    r
}

/// itemtypes: 4 gold, 10 `bow ` (treasureclass 1, rarity 3), 38 tpot,
/// 39 a potion type equivalent to 38. Items: `gld` (classic),
/// `sbw` (bow, level 2), `hbw` (bow, level 5, expansion), `qbw` (quest
/// bow), `pot` (type 39, level 2).
pub(super) fn src() -> Src {
    let mut itemtypes = itemtypes(60);
    itemtypes[10].code = code("bow");
    itemtypes[10].treasureclass = 1;
    let mut gld = item("gld", 4, 0);
    gld.version = 0;
    let mut sbw = item("sbw", 10, 2);
    sbw.version = 0;
    let hbw = item("hbw", 10, 5);
    let mut qbw = item("qbw", 10, 2);
    qbw.quest = 1;
    let mut pot = item("pot", 39, 2);
    pot.type2 = 10;
    let mut u0: Uniqueitems = zeroed();
    u0.index = cell("The Gnasher");
    u0.code = code("sbw");
    let mut u1: Uniqueitems = zeroed();
    u1.index = cell("Annihilus");
    u1.code = code("hbw");
    let mut s0: Setitems = zeroed();
    s0.index = cell("Some Set Item");
    s0.item = code("zzz");
    Src {
        tcx: Vec::new(),
        itemtypes,
        items: vec![gld, sbw, hbw, qbw, pot],
        equiv: equiv(60, &[(39, 38)]),
        uniques: vec![u0, u1],
        sets: vec![s0],
    }
}

#[test]
fn automatic_tcs() {
    let s = src();
    let t = s.build().unwrap();
    assert_eq!(t.len(), 1 + 32);
    assert_eq!(t.group_offset, 1);
    assert_eq!(t.tcs[1].name, b"bow3");
    assert_eq!((t.tcs[1].level, t.tcs[1].picks, t.tcs[1].nodrop), (0, 1, 0));
    // bow3: sbw (level 2, classic). qbw is a quest item; pot is of type
    // 38 (equivalent) and bow via type2, excluded by step 3.
    let e: Vec<(u16, i32, u8)> = t.tcs[1]
        .entries
        .iter()
        .map(|e| (e.id, e.start_expansion, e.flags))
        .collect();
    assert_eq!(e, [(1, 0, 0)]);
    assert_eq!((t.tcs[1].total_classic, t.tcs[1].total_expansion), (3, 3));
    // bow6: hbw (level 5, expansion only).
    assert_eq!(t.tcs[2].name, b"bow6");
    assert_eq!(t.tcs[2].entries[0].flags, FLAG_NOT_CLASSIC);
    assert_eq!((t.tcs[2].total_classic, t.tcs[2].total_expansion), (0, 3));
    assert_eq!(t.tcs[32].name, b"bow96");
    assert_eq!(t.tcs[32].level, 93);
}

#[test]
fn treasureclassex_rows() {
    let mut s = src();
    let mut row = tcx(
        "Row A",
        0,
        100,
        &[
            ("gld,mul=1280", 21),
            ("\"gld,mul=1536,cu=900,ce=5\"", 3),
            ("bow3", 16),
            ("BOW6", 2),
            ("Annihilus", 1),
            ("The Gnasher", 7),
            ("Some Set Item", 1),
            ("Later Row", 5),
            ("sbw", 0),
            ("", 9),
        ],
    );
    row.group = 7;
    row.level = 12;
    row.magic = 1024;
    s.tcx = vec![
        row,
        tcx("Later Row", 0xFFFF_FFFE, 0, &[("gld", 1)]),
        tcx("", 0, 0, &[]),
    ];
    let t = s.build().unwrap();
    assert_eq!(t.len(), 1 + 32 + 2);
    let a = &t.tcs[33];
    assert_eq!((a.group, a.level, a.picks, a.nodrop), (8, 12, 1, 100));
    assert_eq!(a.mods, [1024, 0, 0, 0, 0, 0]);
    let e: Vec<(u16, u16, u8, i32, i32)> = a
        .entries
        .iter()
        .map(|e| (e.id, e.row, e.flags, e.start_classic, e.start_expansion))
        .collect();
    assert_eq!(
        e,
        [
            (0, 1280, 0, 0, 0),
            (0, 1536, 0, 21, 21),
            (1, 0, FLAG_TC, 24, 24),
            // bow6 has classic total 0: expansion only.
            (2, 0, FLAG_TC | FLAG_NOT_CLASSIC, 40, 40),
            (2, 1, FLAG_UNIQUE | FLAG_NOT_CLASSIC, 40, 42),
            // Set item whose code is not an item: id 0xFFFF.
            (0xFFFF, 0, FLAG_SET | FLAG_NOT_CLASSIC, 40, 43),
        ]
    );
    assert_eq!(a.entries[1].mods, [0, 0, 0, 900, 5, 0]);
    assert_eq!((a.total_classic, a.total_expansion), (40, 44));
    // "The Gnasher" (record 0) and the forward reference are dropped;
    // prob 0 adds nothing; the empty cell ends the list.
    assert_eq!(t.notes.len(), 2, "{:?}", t.notes);
    assert_eq!(t.tcs[34].picks, -2);
}

#[test]
fn item_string_parameters_stop_at_unknown_key() {
    let mut s = src();
    s.tcx = vec![tcx(
        "R",
        1,
        0,
        &[
            ("gld,cm=5,xx=3,cr=7", 1),
            ("gld,cm=6,cr", 1),
            ("gld,mul= -2", 1),
        ],
    )];
    let t = s.build().unwrap();
    let e = &t.tcs[33].entries;
    assert_eq!(e[0].mods, [5, 0, 0, 0, 0, 0]);
    assert_eq!(e[1].mods, [6, 0, 0, 0, 0, 0]);
    assert_eq!(e[2].row, 0xFFFE);
}

pub(super) fn grouped() -> TreasureClasses {
    let mk = |group, level| {
        let mut t = tc(1, 0, &[(0, 1, 0)]);
        t.group = group;
        t.level = level;
        t
    };
    classes(vec![
        mk(12, 1),
        mk(12, 38),
        mk(12, 40),
        mk(12, 41),
        mk(13, 2),
        mk(0, 50),
    ])
}

#[test]
fn get_by_level() {
    let t = grouped();
    assert_eq!(t.get(1, 40), Some(3));
    assert_eq!(t.get(1, 0), Some(1));
    assert_eq!(t.get(1, 85), Some(4), "last of the group");
    assert_eq!(t.get(0, 40), None);
    assert_eq!(t.get(7, 40), None);
    assert_eq!(t.get(6, 99), Some(6), "group 0: no upgrade");
    // The starting TC's own level is never compared.
    assert_eq!(t.get(4, 1), Some(4));
}

#[test]
fn chest_table_by_name() {
    let mut s = src();
    s.tcx = vec![
        tcx("act 1 chest a", 1, 0, &[("gld", 1)]),
        tcx("Act 5 (H) Chest C", 1, 0, &[("gld", 1)]),
    ];
    let t = s.build().unwrap();
    assert_eq!(t.chest_tc(0, 0, 0), Some(33));
    assert_eq!(t.chest_tc(2, 4, 2), Some(34));
    assert_eq!(t.chest_tc(9, 9, 9), Some(34), "indices clamp");
    assert_eq!(t.chest_tc(0, 0, 1), None);
}

#[test]
fn non_ascii_names_are_rejected() {
    let mut s = src();
    s.tcx = vec![tcx("Caf\u{e9}", 1, 0, &[])];
    assert!(matches!(s.build(), Err(TreasureError::NonAsciiName(_))));
}

// ------------------------------------------------ monster, chest

#[test]
fn monster_tc_choice() {
    let mut m: Monstats = zeroed();
    (
        m.treasureclass1,
        m.treasureclass2,
        m.treasureclass3,
        m.treasureclass4,
    ) = (11, 12, 13, 14);
    (m.treasureclass1_h, m.treasureclass3_h) = (21, 23);
    let mut su: Superuniques = zeroed();
    (su.tc, su.tc_n, su.tc_h) = (31, 32, 33);
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 0, true, |_| true),
        Ok(11)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Champion, 0, true, |_| true),
        Ok(12)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Unique, 0, true, |_| true),
        Ok(13)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Superunique(Some(&su)), 1, true, |_| true),
        Ok(32)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Superunique(None), 2, true, |_| true),
        Ok(23)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 2, true, |_| true),
        Ok(21)
    );
    // Quest TC: needs TCQuestId, column 4, R, and an open quest.
    m.tcquestid = 1;
    m.tcquestcp = 9;
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 0, true, |cp| cp == 9),
        Ok(14)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 0, false, |_| true),
        Ok(11)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 0, true, |_| false),
        Ok(11)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 1, true, |_| panic!(
            "column 4 is 0"
        )),
        Ok(0)
    );
    assert_eq!(
        monster_tc(&m, MonsterRank::Normal, 3, true, |_| true),
        Err(TreasureError::Difficulty(3))
    );
}

// (exact order: the bonewall test comes before the collision test, so a bone wall on a blocked spot is
// fatal too)
#[test]
fn monster_gate_bonewall_before_collision() {
    assert_eq!(
        monster_drop_gate(0, 1, 344),
        Err(TreasureError::BonewallDrop)
    );
    assert_eq!(monster_drop_gate(0x20000, 1, 344), Ok(false));
}

#[test]
fn monster_gate_and_upgrade() {
    assert_eq!(monster_drop_gate(0x20000, 0, 344), Ok(false));
    assert_eq!(monster_drop_gate(0, 1, 5), Ok(false));
    assert_eq!(monster_drop_gate(0, 0, 5), Ok(true));
    assert_eq!(
        monster_drop_gate(0, 0, 344),
        Err(TreasureError::BonewallDrop)
    );
    let mut m: Monstats = zeroed();
    let mon = Dropper {
        kind: DropperKind::Monster {
            class: 1,
            level: 40,
            playercount: 1,
        },
        x: 0,
        y: 0,
    };
    let mut g = game(true, 1);
    assert_eq!(upgrade_level(&g, &mon, &m), 0, "Normal");
    g.difficulty = 1;
    assert_eq!(upgrade_level(&g, &mon, &m), 40);
    assert_eq!(upgrade_level(&g, &OBJECT, &m), 0);
    m.boss = true;
    assert_eq!(upgrade_level(&g, &mon, &m), 0);
    m.boss = false;
    m.noratio = true;
    assert_eq!(upgrade_level(&g, &mon, &m), 0);
    g.expansion = false;
    m.noratio = false;
    assert_eq!(upgrade_level(&g, &mon, &m), 0);
}

#[test]
fn monster_drop_walks_the_upgraded_tc() {
    let mut w = World::new(vec![], abcd());
    w.tcs = grouped();
    let mut m: Monstats = zeroed();
    m.treasureclass1_n = 1;
    let mon = Dropper {
        kind: DropperKind::Monster {
            class: 1,
            level: 40,
            playercount: 1,
        },
        x: 0,
        y: 0,
    };
    let mut g = game(true, 1);
    g.difficulty = 1;
    let md = MonsterDrop {
        monstats: &m,
        rank: MonsterRank::Normal,
        find_item: false,
    };
    let mut sink = Sink::default();
    let mut seed = Seed::default();
    let out = monster_drop(
        &w.data(),
        &g,
        &md,
        &mon,
        &mut seed,
        None,
        |_| false,
        &mut sink,
    )
    .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(sink.reqs[0].item_level, 40);
    // TC 0: no drop.
    let mut m0 = m.clone();
    m0.treasureclass1_n = 0;
    let md = MonsterDrop {
        monstats: &m0,
        ..md
    };
    let out = monster_drop(
        &w.data(),
        &g,
        &md,
        &mon,
        &mut seed,
        None,
        |_| false,
        &mut sink,
    )
    .unwrap();
    assert!(out.is_empty());
}

pub(super) fn levels() -> Vec<Levels> {
    // Act 1 range: level 2 area 1, level 37 area 12; level 8 area 1.
    let mut ls: Vec<Levels> = (0..137).map(|_| zeroed()).collect();
    ls[2].monlvl1ex = 1;
    ls[37].monlvl1ex = 12;
    ls[8].monlvl1ex = 1;
    ls[20].monlvl1ex = 5;
    ls[30].monlvl1ex = 9;
    ls[109].monlvl3ex = 0;
    ls[136].monlvl3ex = 83;
    ls[120].monlvl3ex = 0xFFFF; // −1 as i16
    ls
}

#[test]
fn area_levels_and_chest_tiers() {
    let ls = levels();
    assert_eq!(area_level(&ls, 37, 0, true), 12);
    assert_eq!(area_level(&ls, 37, 0, false), 0);
    assert_eq!(area_level(&ls, 0, 0, true), 1);
    assert_eq!(area_level(&ls, 137, 0, true), 1);
    assert_eq!(area_level(&ls, 37, 3, true), 1);
    assert_eq!(area_level(&ls, 120, 2, true), -1);
    // lo 1, hi 12, s 4: tiers below 5, below 9, else 2.
    assert_eq!(chest_tier(&ls, 0, 8, 0, true), Ok(0));
    assert_eq!(chest_tier(&ls, 0, 20, 0, true), Ok(1));
    assert_eq!(chest_tier(&ls, 0, 30, 0, true), Ok(2));
    assert_eq!(chest_tier(&ls, 0, 37, 0, true), Ok(2));
    // Hell act 5: lo 0, hi 83, s 28.
    assert_eq!(chest_tier(&ls, 4, 120, 2, true), Ok(0));
    assert_eq!(chest_tier(&ls, 4, 136, 2, true), Ok(2));
    assert_eq!(chest_tier(&ls, 5, 1, 0, true), Err(TreasureError::Act(5)));
}

#[test]
fn chest_drop_passes_the_tier_as_level() {
    let mut w = World::new(vec![tc(1, 0, &[(0, 1, 0)])], abcd());
    w.tcs.chest[2] = Some(1); // normal, act 1, tier C
    let ls = levels();
    let mut sink = Sink::default();
    let first = chest_drop(
        &w.data(),
        &game(true, 1),
        &ls,
        true,
        0,
        30,
        0,
        &OBJECT,
        &mut Seed::default(),
        None,
        &mut sink,
    )
    .unwrap();
    assert_eq!(first, Some(0));
    // No room, or no TC: none.
    let none = chest_drop(
        &w.data(),
        &game(true, 1),
        &ls,
        false,
        0,
        30,
        0,
        &OBJECT,
        &mut Seed::default(),
        None,
        &mut sink,
    )
    .unwrap();
    assert_eq!(none, None);
    let none = chest_drop(
        &w.data(),
        &game(true, 1),
        &ls,
        true,
        0,
        8,
        0,
        &OBJECT,
        &mut Seed::default(),
        None,
        &mut sink,
    )
    .unwrap();
    assert_eq!(none, None);
}

#[test]
fn chest_level_is_the_tier_for_quality() {
    // Edge case 1: D = tier − item level. Tier 2, item level 30: D = −28.
    let row = ratio_row();
    assert_eq!(
        step_chance(&LADDER[0], Ratio::Unique, &row, 2 - 30, 0, &[0; 6]),
        Ok((400 + 28) * 128)
    );
}
