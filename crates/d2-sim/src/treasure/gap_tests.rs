//! Unit tests for treasure rules the first test set left unclaimed.
//! Synthetic inputs from the treasure rules; fixtures from [`super::tests`].

use d2_data::tables::{Itemratio, Monstats};

use super::runtime::{FLAG_NOT_CLASSIC, FLAG_TC, FLAG_UNIQUE};
use super::tests::ratio_row;
use super::tests::*;
use super::*;
use crate::rng::Seed;

// ---------------------------------------------------------------- fixtures

/// A sink that counts placements and can fail the first creations.
#[derive(Default)]
struct CountSink {
    reqs: Vec<DropRequest<()>>,
    places: usize,
    /// Creations that fail before the first success.
    fail: usize,
    created: usize,
}

impl DropSink for CountSink {
    type Spot = ();
    type Item = usize;
    fn place(&mut self, _x: i32, _y: i32) -> Option<()> {
        self.places += 1;
        Some(())
    }
    fn create(&mut self, req: DropRequest<()>) -> Option<usize> {
        self.reqs.push(req);
        if self.fail > 0 {
            self.fail -= 1;
            return None;
        }
        self.created += 1;
        Some(self.created - 1)
    }
    fn gold(&self, _item: usize) -> i32 {
        0
    }
    fn set_gold(&mut self, _item: usize, _value: i32) {}
}

fn steps(seed: Seed, n: usize) -> Seed {
    let mut s = seed;
    for _ in 0..n {
        s.step();
    }
    s
}

fn ids(reqs: &[DropRequest<()>]) -> Vec<u16> {
    reqs.iter().map(|r| r.id).collect()
}

fn run(
    w: &World,
    g: &GameFacts,
    seed: &mut Seed,
    a: &WalkArgs,
) -> (Result<Vec<usize>, TreasureError>, Sink) {
    let mut sink = Sink::default();
    let r = walk(&w.data(), g, &OBJECT, seed, None, a, &mut sink);
    (r, sink)
}

// --------------------------------------------------------- runtime

#[test]
fn tc_order_and_name_visibility() {
    let mut s = src();
    s.tcx = vec![
        tcx("First", 1, 0, &[("gld", 1)]),
        // A backward reference resolves, a forward one is a miss.
        tcx("Second", 1, 0, &[("first", 2), ("Third", 3)]),
        tcx("Third", 1, 0, &[("gld", 1)]),
    ];
    let t = s.build().unwrap();
    // TC 0: empty name, no entries; then the 32 automatic TCs; then rows.
    assert_eq!(t.tcs[0].name, b"");
    assert!(t.tcs[0].entries.is_empty());
    assert_eq!(t.tcs[1].name, b"bow3");
    assert_eq!(t.tcs[32].name, b"bow96");
    assert_eq!(t.tcs[33].name, b"First");
    assert_eq!(t.tcs[34].name, b"Second");
    assert_eq!(t.tcs[35].name, b"Third");
    assert_eq!(t.len(), 36);
    let e: Vec<(u16, u8)> = t.tcs[34].entries.iter().map(|e| (e.id, e.flags)).collect();
    assert_eq!(e, [(33, FLAG_TC)]);
    assert_eq!(t.tcs[34].total_expansion, 2);
    assert_eq!(t.notes.len(), 1, "{:?}", t.notes);

    // More than 65,534 TCs is fatal. No itemtypes with `treasureclass`:
    // TC 0 plus the rows.
    let mut s = src();
    s.itemtypes[10].treasureclass = 0;
    let row = tcx("x", 1, 0, &[]);
    s.tcx = vec![row.clone(); 65_533];
    assert_eq!(s.build().map(|t| t.len()), Ok(65_534));
    s.tcx.push(row);
    assert_eq!(
        s.build().map(|t| t.len()),
        Err(TreasureError::TooManyTcs(65_535))
    );
}

// ----------------------------------------------------- monster drop

fn monster(level: i32) -> Dropper {
    Dropper {
        kind: DropperKind::Monster {
            class: 1,
            level,
            playercount: 8,
        },
        x: 3,
        y: 4,
    }
}

/// Unique ratio 0 / divisor 1 / min 0: chance 0 (instant unique, no
/// draw) when `D` ≥ 0, a `roll(128)` when `D` = −1.
fn instant_unique_row() -> Itemratio {
    let mut r = ratio_row();
    (r.unique, r.uniquedivisor, r.uniquemin) = (0, 1, 0);
    r
}

#[test]
fn monster_walk_arguments() {
    // TC 1: picks 8 of `a` (item level 1); TC 2: picks 8, NoDrop 100.
    let mut items = abcd();
    items[0].level = 1;
    let mut w = World::new(
        vec![tc(8, 0, &[(0, 1, 0)]), {
            let mut t = h2h();
            t.picks = 8;
            t
        }],
        items,
    );
    w.ratio = vec![instant_unique_row()];
    let mut m: Monstats = zeroed();
    m.treasureclass1 = 1;
    let md = MonsterDrop {
        monstats: &m,
        rank: MonsterRank::Normal,
        find_item: false,
    };
    let g = game(true, 1);

    // Q = 0 (quality rolled), L = monster level at least 1 (level 0 → 1,
    // D = 0: unique without a draw), no list and max 6.
    let mut seed = Seed::default();
    let mut sink = CountSink::default();
    let out = monster_drop(
        &w.data(),
        &g,
        &md,
        &monster(0),
        &mut seed,
        None,
        |_| false,
        &mut sink,
    )
    .unwrap();
    assert_eq!(out.len(), 6);
    assert!(sink
        .reqs
        .iter()
        .all(|r| r.quality == 7 && r.item_level == 1));
    // Only the six pick draws `roll(1)`: L = 1, not 0.
    assert_eq!(seed, steps(Seed::default(), 6));

    // U = the monster, R = the death target, F as given: the same as the walk
    // called with those arguments.
    let mut m2 = m.clone();
    m2.treasureclass1 = 2;
    let r = Recipient {
        party: Some(3),
        magic_find: 0,
        gold_find: 0,
    };
    let mut g8 = game(true, 8);
    g8.game_type = 3;
    for f in [false, true] {
        let md = MonsterDrop {
            monstats: &m2,
            rank: MonsterRank::Normal,
            find_item: f,
        };
        let mut s1 = Seed::new(12345, 666);
        let mut k1 = CountSink::default();
        let got = monster_drop(
            &w.data(),
            &g8,
            &md,
            &monster(7),
            &mut s1,
            Some(&r),
            |_| false,
            &mut k1,
        )
        .unwrap();
        let a = WalkArgs {
            tc: Some(2),
            quality: 0,
            level: 7,
            find_item: f,
            list: false,
            max: 6,
        };
        let mut s2 = Seed::new(12345, 666);
        let mut k2 = CountSink::default();
        let want = walk(&w.data(), &g8, &monster(7), &mut s2, Some(&r), &a, &mut k2).unwrap();
        assert_eq!(got, want, "F {f}");
        assert_eq!(k1.reqs, k2.reqs, "F {f}");
        assert_eq!(s1, s2, "F {f}");
    }
}

// ------------------------------------------------------- chest drop

/// A chest world: TC 1 in the normal act 1 tier C slot; item `a` of
/// level 3. Ratio row: unique 1/1/0, set 0/1000/0 (instant set).
fn chest_world(t: TreasureClass) -> World {
    let mut items = abcd();
    items[0].level = 3;
    let mut w = World::new(vec![t], items);
    w.tcs.chest[2] = Some(1);
    let mut r = ratio_row();
    (r.unique, r.uniquedivisor, r.uniquemin) = (1, 1, 0);
    (r.set, r.setdivisor, r.setmin) = (0, 1000, 0);
    w.ratio = vec![r];
    w
}

#[test]
fn chest_walk_arguments() {
    // Picks 8, NoDrop 100 (F = 0 draws it), items a–d.
    let mut t = h2h();
    t.picks = 8;
    let w = chest_world(t);
    let ls = levels();
    let r = Recipient {
        party: Some(2),
        magic_find: 0,
        gold_find: 0,
    };
    for q in [0u8, 4] {
        let mut s1 = Seed::new(12345, 666);
        let mut k1 = CountSink::default();
        let first = chest_drop(
            &w.data(),
            &game(true, 3),
            &ls,
            true,
            0,
            30, // tier 2
            q,
            &OBJECT,
            &mut s1,
            Some(&r),
            &mut k1,
        )
        .unwrap();
        // with U, R, Q, L = tier 2, F = 0, a list and max 6.
        let a = WalkArgs {
            tc: Some(1),
            quality: q,
            level: 2,
            find_item: false,
            list: true,
            max: 6,
        };
        let mut s2 = Seed::new(12345, 666);
        let mut k2 = CountSink::default();
        let all = walk(
            &w.data(),
            &game(true, 3),
            &OBJECT,
            &mut s2,
            Some(&r),
            &a,
            &mut k2,
        )
        .unwrap();
        assert_eq!(k1.reqs, k2.reqs, "Q {q}");
        assert_eq!(s1, s2, "Q {q}");
        if q == 4 {
            assert!(k1.reqs.iter().all(|r| r.quality == 4));
        }
        // Returns the first item.
        assert_eq!(first, all.first().copied(), "Q {q}");
        assert!(first.is_some());
    }
    // max 6 with a list: picks 8 without NoDrop stop at 6.
    let w = chest_world(tc(8, 0, &[(0, 1, 0)]));
    let mut sink = CountSink::default();
    let first = chest_drop(
        &w.data(),
        &game(true, 1),
        &ls,
        true,
        0,
        30,
        4,
        &OBJECT,
        &mut Seed::default(),
        None,
        &mut sink,
    )
    .unwrap();
    assert_eq!(first, Some(0));
    assert_eq!(sink.created, 6);
}

#[test]
fn chest_tier_as_level_lowers_d() {
    // Tier 2, item level 3: D = −1, unique b = (1 + 1) × 128 = 256, a draw.
    // (With the object's area level 5, D = 2 and b ≤ 0: unique, no draw.)
    let w = chest_world(tc(1, 0, &[(0, 1, 0)]));
    let ls = levels();
    let mut seed = Seed::new(12345, 666);
    let mut sink = CountSink::default();
    chest_drop(
        &w.data(),
        &game(true, 1),
        &ls,
        true,
        0,
        30,
        0,
        &OBJECT,
        &mut seed,
        None,
        &mut sink,
    )
    .unwrap();
    // Pick roll(1); roll(256) = 2337785264 % 256 = 176 ≥ 128; set b = 0 → 5.
    assert_eq!(sink.reqs[0].quality, 5);
    assert_eq!(seed, steps(Seed::new(12345, 666), 2));
}

// ------------------------------------------------------- walk

#[test]
fn slot_ends_on_zero_total_or_no_tc() {
    // Total 0: no draw, no item, whatever the picks.
    let w = World::new(vec![tc(5, 100, &[])], abcd());
    let mut seed = Seed::default();
    let (r, sink) = run(&w, &game(true, 1), &mut seed, &args(1, 4));
    assert_eq!(r.map(|v| v.len()), Ok(0));
    assert!(sink.reqs.is_empty());
    assert_eq!(seed, Seed::default());

    // Classic games read the classic total: expansion-only entries → 0.
    let w = World::new(vec![tc(3, 0, &[(0, 5, FLAG_NOT_CLASSIC)])], abcd());
    let mut seed = Seed::default();
    let (_, sink) = run(&w, &game(false, 1), &mut seed, &args(1, 4));
    assert!(sink.reqs.is_empty());
    assert_eq!(seed, Seed::default());
    let mut seed = Seed::default();
    run(&w, &game(true, 1), &mut seed, &args(1, 4)).0.unwrap();
    assert_eq!(seed, steps(Seed::default(), 3));

    // A TC entry to an empty TC (picks 5) or to TC 0 (none): the new slot
    // ends at once; only the parent's pick draws.
    for target in [2u16, 0] {
        let w = World::new(
            vec![tc(1, 0, &[(target, 1, FLAG_TC)]), tc(5, 0, &[])],
            abcd(),
        );
        let mut seed = Seed::default();
        let (r, sink) = run(&w, &game(true, 1), &mut seed, &args(1, 4));
        assert_eq!(r.map(|v| v.len()), Ok(0), "target {target}");
        assert!(sink.reqs.is_empty());
        assert_eq!(seed, steps(Seed::default(), 1), "target {target}");
    }
}

#[test]
fn no_entry_goes_to_next_pick() {
    // Classic, hand-made starts: a (start 0), b expansion-only (start 1),
    // total classic 2. r = 0 → a; r = 1 → no entry.
    let mut t = tc(3, 0, &[(0, 1, 0), (1, 1, FLAG_NOT_CLASSIC)]);
    t.entries[1].start_classic = 1;
    t.total_classic = 2;
    let mut items = abcd();
    for i in &mut items {
        i.version = 0;
    }
    let w = World::new(vec![t], items);
    let mut seed = Seed::default();
    let (r, sink) = run(&w, &game(false, 1), &mut seed, &args(1, 4));
    // roll(2) from {1, 666}: 1, 1, 0 → none, none, then a.
    assert_eq!(r.map(|v| v.len()), Ok(1));
    assert_eq!(ids(&sink.reqs), [0]);
    assert_eq!(seed, steps(Seed::default(), 3));
}

#[test]
fn entry_kind_dispatch() {
    // r = 0 → TC entry (TC 2: d), r = 1 → item a.
    let w = World::new(
        vec![
            tc(-2, 0, &[(2, 1, FLAG_TC), (0, 1, 0)]),
            tc(-1, 0, &[(3, 1, 0)]),
        ],
        abcd(),
    );
    let (r, sink) = run(&w, &game(true, 1), &mut Seed::default(), &args(1, 4));
    assert_eq!(r.map(|v| v.len()), Ok(2));
    assert_eq!(ids(&sink.reqs), [3, 0]);
}

#[test]
fn picks_loop_until_none_left() {
    for picks in [1, 3, 5] {
        let w = World::new(vec![tc(picks, 0, &[(0, 1, 0), (1, 1, 0)])], abcd());
        let mut seed = Seed::default();
        let (r, sink) = run(&w, &game(true, 1), &mut seed, &args(1, 4));
        assert_eq!(r.map(|v| v.len()), Ok(picks as usize));
        assert_eq!(sink.reqs.len(), picks as usize);
        assert_eq!(seed, steps(Seed::default(), picks as usize));
    }
}

#[test]
fn nodrop_scaling_only_when_nodrop_and_draws_nothing() {
    // NoDrop 0 with 8 players: N = 0, one roll(T) per pick.
    let mut t = h2h();
    t.nodrop = 0;
    let w = World::new(vec![t], abcd());
    let mut seed = Seed::new(12345, 666);
    run(&w, &game(true, 8), &mut seed, &args(1, 4)).0.unwrap();
    let mut s = Seed::new(12345, 666);
    s.roll(60);
    s.roll(60);
    assert_eq!(seed, s);
    // NoDrop 100, P = 3: N = 38, one roll(98) per pick, no other draw.
    let w = World::new(vec![h2h()], abcd());
    let mut seed = Seed::new(12345, 666);
    let (_, sink) = run(&w, &game(true, 3), &mut seed, &args(1, 4));
    let mut s = Seed::new(12345, 666);
    let a = s.roll(98);
    let b = s.roll(98);
    assert_eq!((a, b), (31, 66));
    assert_eq!(seed, s);
    assert_eq!(ids(&sink.reqs), [1]);
}

/// A chain of `n` TCs (1..=n), each picks `picks` with one TC entry to the
/// next, then TC `n` + 1 with item `a` (picks −1).
fn chain(n: usize, picks: i32) -> World {
    let mut tcs: Vec<TreasureClass> = (1..=n)
        .map(|i| tc(picks, 0, &[((i + 1) as u16, 1, FLAG_TC)]))
        .collect();
    tcs.push(tc(-1, 0, &[(0, 1, 0)]));
    World::new(tcs, abcd())
}

#[test]
fn tc_entry_rules() {
    let g = game(true, 1);

    // X = get(e.id, 0): no group upgrade even with a high L.
    let mut g1 = tc(-1, 0, &[(0, 1, 0)]);
    (g1.group, g1.level) = (12, 1);
    let mut g2 = tc(-1, 0, &[(1, 1, 0)]);
    (g2.group, g2.level) = (12, 38);
    let w = World::new(vec![tc(-1, 0, &[(2, 1, FLAG_TC)]), g1, g2], abcd());
    assert_eq!(w.tcs.get(2, 99), Some(3));
    let mut a = args(1, 4);
    a.level = 99;
    let (_, sink) = run(&w, &g, &mut Seed::default(), &a);
    assert_eq!(ids(&sink.reqs), [0]);

    // Picks left > 0: a new slot above (64 slots at most, fatal 0xFEA).
    // Each TC of the chain has a pick left when it descends.
    let (r, sink) = run(&chain(63, -2), &g, &mut Seed::default(), &args(1, 4));
    assert_eq!(r.map(|v| v.len()), Ok(1));
    assert_eq!(ids(&sink.reqs), [0]);
    let (r, _) = run(&chain(64, -2), &g, &mut Seed::default(), &args(1, 4));
    assert_eq!(r, Err(TreasureError::SlotOverflow));
    // Picks left 0: the slot is reused, so any depth works.
    let (r, _) = run(&chain(200, -1), &g, &mut Seed::default(), &args(1, 4));
    assert_eq!(r.map(|v| v.len()), Ok(1));

    // New slot picks max(|X.picks|, 1): −3 → three picks, 0 → one.
    let w = World::new(
        vec![
            tc(-1, 0, &[(2, 1, FLAG_TC)]),
            tc(-3, 0, &[(0, 1, 0), (1, 1, 0), (2, 1, 0)]),
        ],
        abcd(),
    );
    let (_, sink) = run(&w, &g, &mut Seed::default(), &args(1, 4));
    assert_eq!(ids(&sink.reqs), [0, 1, 2]);
    let w = World::new(
        vec![tc(-1, 0, &[(2, 1, FLAG_TC)]), tc(0, 0, &[(3, 1, 0)])],
        abcd(),
    );
    let mut seed = Seed::default();
    let (_, sink) = run(&w, &g, &mut seed, &args(1, 4));
    assert_eq!(ids(&sink.reqs), [3]);
    assert_eq!(seed, steps(Seed::default(), 1));

    // Mods = max(previous slot, X) per slot, unsigned: parent 0x8000 beats
    // X's 1 (signed it would not). First step & 0x3FF = 863 < 0x8000.
    let mut top = tc(-1, 0, &[(2, 1, FLAG_TC)]);
    top.mods[4] = 0x8000;
    let mut x = tc(-1, 0, &[(0, 1, 0)]);
    x.mods[4] = 1;
    x.mods[5] = 0x400;
    let w = World::new(vec![top, x], abcd());
    let mut seed = Seed::default();
    let (_, sink) = run(&w, &g, &mut seed, &args(1, 4));
    assert_eq!(sink.reqs[0].drop_flags, 0x04 | 0x10);
    assert_eq!(seed, steps(Seed::default(), 2));
}

#[test]
fn create_request_and_failure() {
    let mut t = tc(
        -4,
        0,
        &[(0, 1, FLAG_UNIQUE), (1, 1, 0), (2, 1, 0), (3, 1, 0)],
    );
    t.entries[0].row = 9;
    t.mods[4] = 0x400;
    let w = World::new(vec![t], abcd());
    let mut sink = CountSink {
        fail: 1,
        ..CountSink::default()
    };
    let mut a = args(1, 4);
    a.max = 2;
    let out = walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &a,
        &mut sink,
    )
    .unwrap();
    // The request carries id, quality, index and d.
    let got: Vec<(u16, u8, i32, u8)> = sink
        .reqs
        .iter()
        .map(|r| (r.id, r.quality, r.index, r.drop_flags))
        .collect();
    assert_eq!(got, [(0, 7, 10, 0x04), (1, 4, 0, 0x04), (2, 4, 0, 0x04)]);
    // The failed creation is not counted: b and c fill max 2.
    assert_eq!(out, [0, 1]);
}

// ------------------------------------------------------------

#[test]
fn negative_item_id_creates_nothing() {
    // Classic throwable `a` without an `lsd` item: the 11th pick has id −1.
    let mut items = abcd();
    for i in &mut items {
        i.version = 0;
    }
    items[0].type_ = 11;
    let mut w = World::new(vec![tc(1, 0, &[(0, 1, 0)])], items);
    w.itemtypes[11].throwable = 1;
    let mut seed = Seed::default();
    let mut sink = CountSink::default();
    let out = walk(
        &w.data(),
        &game(false, 1),
        &OBJECT,
        &mut seed,
        None,
        &args(1, 4),
        &mut sink,
    )
    .unwrap();
    assert!(out.is_empty());
    assert!(sink.reqs.is_empty());
    assert_eq!(sink.places, 0);
    assert_eq!(seed, steps(Seed::default(), 11));
}

// ---------------------------------------------------------- edge cases

#[test]
fn classic_uses_the_highest_version_ratio_row() {
    // Rows: Version 1 (unique at once), Version 0 and Version 101 (set at
    // once). Classic and expansion games alike read the Version 1 row.
    let mut v1 = ratio_row();
    (v1.unique, v1.uniquedivisor, v1.uniquemin) = (0, 1, 0);
    let mut v0 = ratio_row();
    v0.version = 0;
    (v0.set, v0.setdivisor, v0.setmin) = (0, 1, 0);
    let mut v101 = v0.clone();
    v101.version = 101;
    let mut items = abcd();
    items[0].version = 0;
    items[0].level = 1;
    let mut w = World::new(vec![tc(-1, 0, &[(0, 1, 0)])], items);
    w.ratio = vec![v1, v0, v101];
    for exp in [false, true] {
        let mut seed = Seed::default();
        let (_, sink) = run(&w, &game(exp, 1), &mut seed, &args(1, 0));
        assert_eq!(sink.reqs[0].quality, 7, "expansion {exp}");
        assert_eq!(seed, Seed::default(), "expansion {exp}");
    }
}

#[test]
fn differences_from_d2moo_1_10f() {
    // Exact hit returns m: starts [0, 21, 37, 58], r = 21 → b (D2MOO: a).
    assert_eq!(select_entry(&h2h(), 21, true), Some(1));
    // Act 5 chest range ends at level 136 (D2MOO: 132).
    assert_eq!(CHEST_ACTS[4], (109, 136));
    // The player count includes S: 1 living player, S = 8 → n = 4
    // (D2MOO: 1).
    let mut g = game(true, 1);
    g.game_type = 3;
    g.players_setting = 8;
    assert_eq!(player_factor(&g, &OBJECT, None), 4);
}
