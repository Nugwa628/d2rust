//! Checks D2RUST treasure-class construction, lookup, drop quality and
//! walking against expected vectors and the player's tables. Sweeps check
//! every treasure class, monster treasure-class reference and item quality.
//! Data is loaded from the install in `D2_GAME_DIR` at run time.
//!
//! `D2_GAME_DIR=<install> cargo test -p d2-sim --test game_treasure -- --ignored`

mod items_treasure_live;

use std::collections::BTreeSet;
use std::sync::OnceLock;

use d2_data::bin::tc_count;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{
    text, Armor, Itemratio, Itemtypes, Levels, Misc, Monstats, Setitems, Superuniques,
    Treasureclassex, Uniqueitems, Weapons,
};
use d2_sim::rng::Seed;
use d2_sim::treasure::runtime::{
    find_item_code, FLAG_NOT_CLASSIC, FLAG_SET, FLAG_TC, FLAG_UNIQUE, TYPE_TPOT,
};
use d2_sim::treasure::{
    area_level, chest_tier, item_list, monster_tc, nodrop, ratio_row, roll_quality, select_entry,
    walk, DropRequest, DropSink, Dropper, DropperKind, GameFacts, ItemData, MonsterRank, Recipient,
    TcSources, TreasureClass, TreasureClasses, TreasureData, WalkArgs,
};
use items_treasure_live::{code, fixed, typed};

/// Index of the first `treasureclassex` TC: TC 0 and the 160 automatic
/// TCs come first (§1.2, §1.3).
const FIRST_TCX: usize = 161;

struct Live {
    items: Vec<ItemData>,
    itemtypes: Vec<Itemtypes>,
    equiv: EquivMatrix,
    itemratio: Vec<Itemratio>,
    uniques: Vec<Uniqueitems>,
    tcs: TreasureClasses,
}

fn live() -> &'static Live {
    static LIVE: OnceLock<Live> = OnceLock::new();
    LIVE.get_or_init(|| {
        let items = item_list(&typed::<Weapons>(), &typed::<Armor>(), &typed::<Misc>());
        let itemtypes = typed::<Itemtypes>();
        let equiv = fixed().itemtypes_equiv.clone();
        let uniques = typed::<Uniqueitems>();
        let setitems = typed::<Setitems>();
        let tcs = TreasureClasses::build(&TcSources {
            treasureclassex: &typed::<Treasureclassex>(),
            itemtypes: &itemtypes,
            items: &items,
            equiv: &equiv,
            uniqueitems: &uniques,
            setitems: &setitems,
        })
        .expect("the live TCs build");
        Live {
            items,
            itemtypes,
            equiv,
            itemratio: typed::<Itemratio>(),
            uniques,
            tcs,
        }
    })
}

impl Live {
    fn data(&self) -> TreasureData<'_> {
        TreasureData {
            tcs: &self.tcs,
            items: &self.items,
            itemtypes: &self.itemtypes,
            equiv: &self.equiv,
            itemratio: &self.itemratio,
        }
    }

    fn tc(&self, i: usize) -> &TreasureClass {
        &self.tcs.tcs[i]
    }

    /// The TC index stored under `name`.
    fn find(&self, name: &str) -> usize {
        self.tcs
            .tcs
            .iter()
            .position(|t| t.name == name.as_bytes())
            .unwrap_or_else(|| panic!("TC {name:?} exists"))
    }
}

fn name(tc: &TreasureClass) -> String {
    String::from_utf8_lossy(&tc.name).into_owned()
}

/// Entry probabilities from the expansion starts and total (§1.1: each
/// start is the total before the entry).
fn probs(tc: &TreasureClass) -> Vec<i32> {
    let e = &tc.entries;
    (0..e.len())
        .map(|i| {
            let next = e
                .get(i + 1)
                .map_or(tc.total_expansion, |n| n.start_expansion);
            next - e[i].start_expansion
        })
        .collect()
}

// ------------------------------------------------------------ §1 vectors

/// Test vector "TC count, kinds" and the measurements of §1.3 and §1.5.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_tc_counts_and_kinds() {
    let l = live();
    let tcs = &l.tcs.tcs;
    assert_eq!(tcs.len(), 1_013, "TC array count, TC 0 included");
    let f = fixed();
    let count = tc_count(
        f.table("itemtypes").unwrap(),
        f.table("treasureclassex").unwrap(),
    );
    assert_eq!(tcs.len(), count, "loading.md §10.6 count");
    assert!(l.tcs.notes.is_empty(), "0 misses: {:?}", l.tcs.notes);

    // §1.3: 160 automatic TCs, 763 entries; 85 / 44 with total 0.
    let auto = &tcs[1..FIRST_TCX];
    assert_eq!(auto.len(), 160);
    assert_eq!(auto.iter().map(|t| t.entries.len()).sum::<usize>(), 763);
    assert_eq!(auto.iter().filter(|t| t.total_classic == 0).count(), 85);
    assert_eq!(auto.iter().filter(|t| t.total_expansion == 0).count(), 44);

    // §1.5: entry kinds over the treasureclassex TCs.
    let tcx = &tcs[FIRST_TCX..];
    let entries = || tcx.iter().flat_map(|t| t.entries.iter());
    let is_item = |f: u8| f & (FLAG_TC | FLAG_UNIQUE | FLAG_SET) == 0;
    assert_eq!(entries().filter(|e| e.flags & FLAG_TC != 0).count(), 2_742);
    assert_eq!(entries().filter(|e| is_item(e.flags)).count(), 660);
    assert_eq!(entries().filter(|e| e.flags & FLAG_UNIQUE != 0).count(), 2);
    assert_eq!(entries().filter(|e| e.flags & FLAG_SET != 0).count(), 0);
    let mul: Vec<u16> = entries()
        .filter(|e| is_item(e.flags) && e.row != 0)
        .map(|e| e.row)
        .collect();
    assert_eq!(mul.len(), 81);
    assert!(
        mul.iter().all(|m| [1280, 1536, 2048].contains(m)),
        "{mul:?}"
    );
    // No other parameter key: no entry mods.
    assert!(entries().all(|e| e.mods == [0; 6]));
    assert_eq!(tcx.iter().filter(|t| t.picks < 0).count(), 184);
    assert_eq!(tcx.iter().filter(|t| t.nodrop != 0).count(), 300);
    assert_eq!(tcx.iter().filter(|t| t.mods != [0; 6]).count(), 240);
}

/// §1.3: the itemtypes records with `treasureclass` ≠ 0, the group
/// offset `A` and the automatic TC names and levels; record 38 is `tpot`.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_automatic_tcs() {
    let l = live();
    let with_tc: Vec<(usize, String)> = l
        .itemtypes
        .iter()
        .enumerate()
        .filter(|(_, t)| t.treasureclass != 0)
        .map(|(i, t)| (i, code(t.code)))
        .collect();
    let want = [
        (27, "bow "),
        (45, "weap"),
        (46, "mele"),
        (50, "armo"),
        (85, "abow"),
    ];
    assert_eq!(
        with_tc,
        want.map(|(i, c)| (i, c.to_string())).to_vec(),
        "itemtypes with treasureclass"
    );
    assert_eq!(code(l.itemtypes[TYPE_TPOT].code), "tpot");
    assert_eq!(l.tcs.group_offset, 5, "A");
    let mut i = 1;
    for c in ["bow", "weap", "mele", "armo", "abow"] {
        for lv in (3..=96).step_by(3) {
            let tc = l.tc(i);
            assert_eq!(name(tc), format!("{c}{lv}"));
            assert_eq!((tc.group, tc.level, tc.picks, tc.nodrop), (0, lv - 3, 1, 0));
            assert_eq!(tc.mods, [0; 6]);
            i += 1;
        }
    }
    assert_eq!(i, FIRST_TCX);
}

/// Test vector `Act 1 H2H A` (§1.4).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_act1_h2h_a() {
    let l = live();
    let i = l.find("Act 1 H2H A");
    assert_eq!(i, 430);
    let tc = l.tc(i);
    assert_eq!(tc.group, 12, "7 + A");
    assert_eq!((tc.picks, tc.nodrop), (1, 100));
    assert_eq!((tc.total_classic, tc.total_expansion), (60, 60));
    let e = &tc.entries;
    assert_eq!(e.len(), 4);
    assert_eq!(e[0].flags & (FLAG_TC | FLAG_UNIQUE | FLAG_SET), 0);
    assert_eq!(e[0].id, 523);
    assert_eq!(code(l.items[523].code), "gld ");
    for (k, (tci, n)) in [
        (218, "Act 1 Equip A"),
        (203, "Act 1 Junk"),
        (370, "Act 1 Good"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_ne!(e[k + 1].flags & FLAG_TC, 0);
        assert_eq!(e[k + 1].id, tci);
        assert_eq!(name(l.tc(usize::from(tci))), n);
    }
    assert_eq!(probs(tc), [21, 16, 21, 2]);
    let starts: Vec<(i32, i32)> = e
        .iter()
        .map(|e| (e.start_classic, e.start_expansion))
        .collect();
    assert_eq!(starts, [(0, 0), (21, 21), (37, 37), (58, 58)]);
}

/// Test vector `ROP (N)` (§1.5): a TC entry and the `Annihilus` unique
/// entry (expansion only).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_rop_n() {
    let l = live();
    let tc = l.tc(l.find("ROP (N)"));
    let e = &tc.entries;
    assert_eq!(e.len(), 2);
    assert_ne!(e[0].flags & FLAG_TC, 0);
    assert_eq!(e[0].id, 855);
    assert_eq!(name(l.tc(855)), "Diablo (N)");
    assert_eq!((e[1].flags, e[1].row), (0x11, 381));
    assert_eq!(text(&l.uniques[381].index), b"Annihilus");
    assert_eq!(
        Some(usize::from(e[1].id)),
        find_item_code(&l.items, l.uniques[381].code),
        "id = item index of the unique's code"
    );
    assert_eq!(probs(tc), [4, 1]);
    assert_eq!((tc.total_classic, tc.total_expansion), (4, 5));
}

/// Test vector `Act 1 Champ A` (§1.4): negative picks and two TC entries.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_act1_champ_a() {
    let l = live();
    let tc = l.tc(l.find("Act 1 Champ A"));
    assert_eq!(tc.picks, -2);
    let named: Vec<String> = tc
        .entries
        .iter()
        .map(|e| {
            assert_ne!(e.flags & FLAG_TC, 0);
            name(l.tc(usize::from(e.id)))
        })
        .collect();
    assert_eq!(named, ["Act 1 Citem A", "Act 1 Cpot A"]);
    assert_eq!(probs(tc), [1, 2]);
}

/// Every treasureclassex-derived TC has zero in modifier slots 5 and 6.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_tcx_slots_5_6_zero() {
    let t = fixed().table("treasureclassex").unwrap();
    for (i, r) in t.iter().enumerate() {
        assert_eq!(r[0x30..0x34], [0; 4], "treasureclassex record {i}");
    }
}

/// Test vector "chest table": all 45 chest TCs found, `Act 1 Chest A` =
/// TC 385 (§1.6).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_chest_table() {
    let l = live();
    for (d, dn) in ["", " (N)", " (H)"].iter().enumerate() {
        for act in 0..5 {
            for (tier, tn) in ["A", "B", "C"].iter().enumerate() {
                let want = format!("Act {}{dn} Chest {tn}", act + 1);
                let got =
                    l.tcs.chest[(d * 5 + act) * 3 + tier].unwrap_or_else(|| panic!("{want} found"));
                assert_eq!(name(l.tc(usize::from(got))), want);
            }
        }
    }
    assert_eq!(l.tcs.chest_tc(0, 0, 0), Some(385));
}

// ------------------------------------------------------------ §2, §4

/// Test vectors `get(430, 40)`, `get(430, 0)`, `get(430, 85)`,
/// `get(0, 40)` (§2).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_get_by_level() {
    let l = live();
    let t = &l.tcs;
    assert_eq!(t.get(430, 40), Some(445));
    assert_eq!(name(l.tc(445)), "Act 1 (N) H2H B");
    let levels: Vec<u16> = (444..=446).map(|i| l.tc(i).level).collect();
    assert_eq!(levels, [38, 40, 41]);
    assert!((444..=446).all(|i| l.tc(i).group == l.tc(430).group));
    assert_eq!(t.get(430, 0), Some(430));
    assert_eq!(t.get(430, 85), Some(471));
    assert_eq!(name(l.tc(471)), "Act 5 (H) H2H C");
    assert_ne!(
        l.tc(472).group,
        l.tc(430).group,
        "471 is the last of the group"
    );
    assert_eq!(t.get(0, 40), None);
}

/// Test vectors "chest tier" (§4 steps 2–5): expansion Normal act 0 and
/// expansion Hell act 4.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_chest_tiers() {
    let lv = typed::<Levels>();
    let a = |level, d| area_level(&lv, level, d, true);
    // Normal, act 0: lo 1 (level 2), hi 12 (level 37), s 4.
    assert_eq!((a(2, 0), a(37, 0)), (1, 12));
    assert_eq!(a(8, 0), 1);
    assert_eq!(chest_tier(&lv, 0, 8, 0, true), Ok(0));
    assert_eq!(chest_tier(&lv, 0, 37, 0, true), Ok(2));
    let l = live();
    let tc = l.tcs.chest_tc(0, 0, 0).unwrap();
    assert_eq!(
        (tc, name(l.tc(usize::from(tc))).as_str()),
        (385, "Act 1 Chest A")
    );
    // Hell, act 4: lo 0 (level 109), hi 83 (level 136), s 28.
    assert_eq!((a(109, 2), a(136, 2)), (0, 83));
}

// ------------------------------------------------------------ §5.4, §6

/// §5.4 step 5 on every live (nodrop, total) pair: 23 expansion and 15
/// classic pairs (a TC with total 0 ends its slot before step 5); for
/// `n` = 2…8 the binary64 result equals the exact rational form
/// floor(C·n0^n / ((n0 + C)^n − n0^n)), which the spec states is a valid
/// check on the live data.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_nodrop_pairs() {
    let l = live();
    for (exp, want) in [(true, 23), (false, 15)] {
        let pairs: BTreeSet<(i32, i32)> = l
            .tcs
            .tcs
            .iter()
            .filter(|t| t.nodrop != 0 && t.total(exp) > 0)
            .map(|t| (t.nodrop, t.total(exp)))
            .collect();
        assert_eq!(pairs.len(), want, "expansion {exp}: {pairs:?}");
        for &(n0, c) in &pairs {
            for n in 2..=8u32 {
                let (a, b) = (n0 as u128, c as u128);
                let den = (a + b).checked_pow(n).unwrap() - a.pow(n);
                let num = b.checked_mul(a.checked_pow(n).unwrap()).unwrap();
                let exact = (num / den) as i32;
                assert_eq!(nodrop(n0, c, n as i32), Ok(exact), "n0 {n0} C {c} n {n}");
            }
        }
    }
}

/// Every (`Class Specific`, `Uber`) pair selects a `Version` 1 itemratio
/// row in both classic and expansion games.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_ratio_rows_are_version_1() {
    let rows = &live().itemratio;
    for cs in [false, true] {
        for uber in [false, true] {
            let r = ratio_row(rows, cs, uber).expect("a ratio row");
            assert_eq!(r.version, 1, "class specific {cs}, uber {uber}");
        }
    }
}

/// §6 on every item: no fatal error, a quality 1–7, step 2's direct
/// results exact, and the gates of `treasure-quality.tsv` kept; for
/// every live slot-mod set, item level and magic find tried.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_drop_quality_every_item() {
    let l = live();
    let data = l.data();
    let mut mods: BTreeSet<[u16; 6]> = l.tcs.tcs.iter().map(|t| t.mods).collect();
    mods.insert([0; 6]);
    let mut seed = Seed::init_low(0x5EED);
    for (id, item) in l.items.iter().enumerate() {
        let ty = &l.itemtypes[usize::from(item.type_)];
        let direct = if ty.normal != 0 {
            Some(2)
        } else if item.unique != 0 || (ty.magic != 0 && item.quest != 0) {
            Some(7)
        } else {
            None
        };
        for m in &mods {
            for lvl in [0, 1, 2, 30, 60, 99] {
                for mf in [-100, 0, 100, 1000] {
                    let before = seed;
                    let q = roll_quality(&data, id as u16, lvl, mf, m, &mut seed)
                        .unwrap_or_else(|e| panic!("item {id}: {e}"));
                    let at = format!("item {id} L {lvl} M {mf} mods {m:?}");
                    assert!((1..=7).contains(&q), "{at}: quality {q}");
                    if let Some(d) = direct {
                        assert_eq!(q, d, "{at}");
                        assert_eq!(seed, before, "{at}: step 2 draws nothing");
                        continue;
                    }
                    if q == 6 {
                        assert_ne!(ty.rare, 0, "{at}: rare gate");
                    }
                    // The magic gate is ladder step 4 (`treasure-quality.tsv`),
                    // run in §6 step 6; `M` ≤ −100 skips straight to step 7
                    // (§6 step 5), past the gate, so a `magic` type (item
                    // 520, first live failure 2026-10-06) can come out ≤ 3.
                    if ty.magic != 0 && mf > -100 {
                        assert!(q >= 4, "{at}: magic gate");
                    }
                    if mf <= -100 {
                        assert!(q <= 3, "{at}: M ≤ −100 skips to step 7");
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------ sweeps

/// §1 structure on every TC: starts follow the totals (§1.1), TC entries
/// point backwards to a real TC (§1.2), item entries to a real item,
/// unique/set entries are expansion only, picks never 0 (§1.4).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_tc_structure() {
    let l = live();
    for (i, tc) in l.tcs.tcs.iter().enumerate().skip(1) {
        let at = format!("TC {i} {}", name(tc));
        assert_ne!(tc.picks, 0, "{at}");
        let mut prev: Option<(i32, i32)> = None;
        for e in &tc.entries {
            if let Some((c, x)) = prev {
                assert!(e.start_classic >= c, "{at}: classic starts");
                assert!(e.start_expansion > x, "{at}: expansion starts");
            } else {
                assert_eq!((e.start_classic, e.start_expansion), (0, 0), "{at}");
            }
            prev = Some((e.start_classic, e.start_expansion));
            if e.flags & FLAG_TC != 0 {
                let id = usize::from(e.id);
                assert!((1..i).contains(&id), "{at}: TC entry {id}");
            } else {
                assert!(usize::from(e.id) < l.items.len(), "{at}: item {}", e.id);
            }
            if e.flags & (FLAG_UNIQUE | FLAG_SET) != 0 {
                assert_ne!(e.flags & FLAG_NOT_CLASSIC, 0, "{at}");
            }
        }
        if let Some((c, x)) = prev {
            assert!(tc.total_expansion > x, "{at}: expansion total");
            assert!(tc.total_classic >= c, "{at}: classic total");
        } else {
            assert_eq!((tc.total_classic, tc.total_expansion), (0, 0), "{at}");
        }
    }
}

/// §2 and §5.5 on every TC: `get` resolves for every level, and every
/// pick value `r` < total selects an entry whose range holds `r`
/// (expansion) or a classic entry starting at or below `r` (classic).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_every_tc_resolves_and_picks() {
    let l = live();
    let n = l.tcs.len();
    for (i, tc) in l.tcs.tcs.iter().enumerate().skip(1) {
        let at = format!("TC {i} {}", name(tc));
        for lvl in 0..=120 {
            let g = l.tcs.get(i as u16, lvl).unwrap_or_else(|| panic!("{at}"));
            let g = usize::from(g);
            assert!(g >= i && g < n, "{at}: get(_, {lvl}) = {g}");
            if g != i {
                assert_eq!(l.tc(g).group, tc.group, "{at}");
            }
        }
        for exp in [true, false] {
            for r in 0..tc.total(exp) {
                let k = select_entry(tc, r, exp).unwrap_or_else(|| panic!("{at}: r {r}"));
                let e = &tc.entries[k];
                if exp {
                    let end = tc
                        .entries
                        .get(k + 1)
                        .map_or(tc.total_expansion, |x| x.start_expansion);
                    assert!(e.start_expansion <= r && r < end, "{at}: r {r} → {k}");
                } else {
                    assert_eq!(e.flags & FLAG_NOT_CLASSIC, 0, "{at}: r {r} → {k}");
                    assert!(e.start_classic <= r, "{at}: r {r} → {k}");
                }
            }
        }
    }
}

/// A sink that accepts every placement and creation and records the
/// requests.
#[derive(Default)]
struct Sink {
    reqs: Vec<DropRequest<()>>,
    gold: Vec<i32>,
}

impl DropSink for Sink {
    type Spot = ();
    type Item = usize;
    fn place(&mut self, _x: i32, _y: i32) -> Option<()> {
        Some(())
    }
    fn create(&mut self, req: DropRequest<()>) -> Option<usize> {
        self.reqs.push(req);
        self.gold.push(100);
        Some(self.reqs.len() - 1)
    }
    fn gold(&self, item: usize) -> i32 {
        self.gold[item]
    }
    fn set_gold(&mut self, item: usize, value: i32) {
        self.gold[item] = value;
    }
}

/// §5 from every TC, classic and expansion, 1 and 8 players, with and
/// without magic find, at three item levels and two seeds: no fatal
/// error, at most 6 items, every request names a real item with a
/// quality 1–7; classic games create no expansion item (§5.7 step 2);
/// drop flags 0x04 / 0x10 never set (TC modifier slots 5 and 6 are zero).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_walk_every_tc() {
    let l = live();
    let data = l.data();
    for i in 1..l.tcs.len() {
        let at = format!("TC {i} {}", name(l.tc(i)));
        for exp in [true, false] {
            for players in [1, 8] {
                for mf in [0, 500] {
                    for lvl in [1, 50, 99] {
                        for s in [1u32, 0x5EED] {
                            let game = GameFacts {
                                expansion: exp,
                                difficulty: 0,
                                game_type: 3,
                                living_players: 1,
                                players_setting: players,
                                item_format: if exp { 101 } else { 2 },
                            };
                            let dropper = Dropper {
                                kind: DropperKind::Monster {
                                    class: 0,
                                    level: lvl,
                                    playercount: 8,
                                },
                                x: 0,
                                y: 0,
                            };
                            let recipient = Recipient {
                                party: None,
                                magic_find: mf,
                                gold_find: 0,
                            };
                            let args = WalkArgs {
                                tc: Some(i as u16),
                                quality: 0,
                                level: lvl,
                                find_item: false,
                                list: false,
                                max: 6,
                            };
                            let mut seed = Seed::init_low(s);
                            let mut sink = Sink::default();
                            let out = walk(
                                &data,
                                &game,
                                &dropper,
                                &mut seed,
                                Some(&recipient),
                                &args,
                                &mut sink,
                            )
                            .unwrap_or_else(|e| panic!("{at} exp {exp}: {e}"));
                            assert!(out.len() <= 6, "{at}");
                            for r in &sink.reqs {
                                let item = l
                                    .items
                                    .get(usize::from(r.id))
                                    .unwrap_or_else(|| panic!("{at}: id {}", r.id));
                                assert!((1..=7).contains(&r.quality), "{at}: {r:?}");
                                assert_eq!(r.drop_flags & 0x14, 0, "{at}: {r:?}");
                                if !exp {
                                    assert!(item.version < 100, "{at}: classic {r:?}");
                                    assert_eq!(r.index, 0, "{at}: classic {r:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// §3.2 on every monster and superunique, every difficulty and rank:
/// the TC column is 0 (no drop) or names a TC that `get` resolves for
/// every level (§3.4).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_monster_tcs_resolve() {
    let l = live();
    let ms = typed::<Monstats>();
    let su = typed::<Superuniques>();
    let n = l.tcs.len();
    let check = |at: &str, id: u16| {
        if id == 0 {
            return;
        }
        assert!(usize::from(id) < n, "{at}: TC {id}");
        for lvl in 0..=120 {
            assert!(l.tcs.get(id, lvl).is_some(), "{at}: get({id}, {lvl})");
        }
    };
    for (c, m) in ms.iter().enumerate() {
        for d in 0..3u8 {
            for (rank, rn) in [
                (MonsterRank::Normal, "normal"),
                (MonsterRank::Champion, "champion"),
                (MonsterRank::Unique, "unique"),
                (MonsterRank::Superunique(None), "superunique without record"),
            ] {
                for quest in [false, true] {
                    let id = monster_tc(m, rank, d, true, |_| quest).unwrap();
                    check(&format!("monstats {c} d {d} {rn} quest {quest}"), id);
                }
            }
        }
    }
    for (i, s) in su.iter().enumerate() {
        let m = ms
            .get(s.class as usize)
            .unwrap_or_else(|| panic!("superunique {i}: class {}", s.class));
        for d in 0..3u8 {
            let id = monster_tc(m, MonsterRank::Superunique(Some(s)), d, false, |_| false).unwrap();
            check(&format!("superunique {i} d {d}"), id);
        }
    }
}
