//! Tests written from the item to kill mutants that survived
//! `cargo mutants` on `crates/d2-sim/src/items/`.
//! Each test asserts what the
//! rule says at the boundary or bit the mutant changed.

use super::*;
use crate::items::affixes::{crafted, magic, rare};
use crate::items::create::{class_skill_mods, init_item_stats, socket_count, socket_roll};
use crate::items::tables::{RareRec, SkillRec, UniqueRec};
use crate::items::{create_item, flag, q, req, stat, ItemRequest, PlayerInfo, RequestUnit};
use d2_data::tables::{Itemratio, Record};

/// Bit 4096 is a real bit; only an index above 4096
/// reads as dropped.
#[test]
fn unique_bit_4096_is_a_bit() {
    let mut b = UniqueBits::default();
    assert!(!b.get(4096));
    assert!(b.get(4097));
    b.set(4096);
    assert!(b.get(4096));
    assert!(!b.get(4095));
}

/// Tables for the rare kind-order model: one magic suffix (group 2), two
/// magic prefixes (groups 1, 3), one rare suffix and one rare prefix, all
/// fitting a helm. The kinds are asymmetric, so the kind order shows in
/// the draws.
fn rare_tables() -> (ItemTables, usize) {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.magic = vec![
        affix_row(ty::HELM, 2),
        affix_row(ty::HELM, 1),
        affix_row(ty::HELM, 3),
    ];
    t.n_suffix = 1;
    t.n_prefix = 2;
    let r = RareRec {
        itype: [ty::HELM as i16, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    t.rare = vec![r.clone(), r];
    t.n_rare_suffix = 1;
    (t, i)
}

/// on [`rare_tables`], step by step: rare names draw
/// roll(1) each, the count one step, then per affix the kind step (lo′
/// odd → suffix; only while both kinds are open) and the affix roll (coin step; then
/// with c rows of the kind whose group is not taken, roll(c + 1) when c >
/// 0, else 0). Returns the seed and the slot counts.
fn rare_model(mut s: Seed) -> (Seed, usize, usize) {
    s.roll(1);
    s.roll(1);
    let n = [3, 4, 4, 5, 5, 5, 6, 6][(s.step() & 7) as usize];
    let (mut np, mut ns, mut pdone, mut sdone, mut k) = (0, 0, false, false, 0);
    while k < n {
        if pdone && sdone {
            break;
        }
        let suffix = if pdone {
            true
        } else if sdone {
            false
        } else {
            s.step() % 2 == 1
        };
        s.step(); // coin
        let c = if suffix { 1 - ns } else { 2 - np };
        if c == 0 {
            if suffix {
                sdone = true;
            } else {
                pdone = true;
            }
            continue;
        }
        s.roll(c as i32 + 1);
        if suffix {
            ns += 1;
        } else {
            np += 1;
        }
        k += 1;
    }
    (s, np, ns)
}

/// The kind step picks a suffix on an odd lo′,
/// a prefix on an even one.
#[test]
fn rare_kind_step_parity() {
    let (t, i) = rare_tables();
    let rq = ItemRequest::default();
    let mut orders = std::collections::BTreeSet::new();
    for seed in 0..200 {
        let mut it = item(i, seed);
        it.quality = q::RARE;
        assert!(rare(&t, &mut it, &rq));
        let (s, np, ns) = rare_model(Seed::init_low(seed));
        assert_eq!(it.item_seed, s, "seed {seed}");
        let count = |a: &[u16; 3]| a.iter().filter(|&&x| x != 0).count();
        assert_eq!((count(&it.prefix), count(&it.suffix)), (np, ns));
        // Which kind came first (the first kind step).
        let mut f = Seed::init_low(seed);
        f.roll(1);
        f.roll(1);
        f.step();
        orders.insert(f.step() & 1);
    }
    assert_eq!(orders.len(), 2, "both first kinds exercised");
}

/// Success needs P or S > 0; suffixes alone pass.
/// Step 4.3: the third suffix closes the suffix kind (no fourth slot).
#[test]
fn rare_with_suffixes_only() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    // Six suffixes in distinct groups, no prefix row.
    t.magic = (0..6).map(|g| affix_row(ty::HELM, g + 1)).collect();
    t.n_suffix = 6;
    t.n_prefix = 0;
    let r = RareRec {
        itype: [ty::HELM as i16, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    t.rare = vec![r.clone(), r];
    t.n_rare_suffix = 1;
    let rq = ItemRequest::default();
    let mut seen_full = false;
    for seed in 0..60 {
        let mut it = item(i, seed);
        it.quality = q::RARE;
        assert!(rare(&t, &mut it, &rq), "seed {seed}");
        assert_eq!(it.prefix, [0; 3]);
        assert_ne!(it.suffix[0], 0);
        seen_full |= it.suffix.iter().all(|&a| a != 0);
    }
    assert!(seen_full, "a count ≥ 4 fills all three suffix slots");
}

/// The crafted minimum from the request ilvl
/// (> 70 → 4, > 50 → 3, > 30 → 2, else 1) when lo′ mod 5 = 0.
#[test]
fn crafted_minimum_by_ilvl() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.magic = (0..12).map(|g| affix_row(ty::HELM, g + 1)).collect();
    t.n_suffix = 6;
    t.n_prefix = 6;
    let r = RareRec {
        itype: [ty::HELM as i16, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    t.rare = vec![r.clone(), r];
    t.n_rare_suffix = 1;
    // Two rare-name roll(1) steps, then the count step with lo′ mod 5 = 0.
    let seed = find_seed(|s| {
        s.step();
        s.step();
        s.step() % 5 == 0
    });
    for (ilvl, want) in [(30, 1), (31, 2), (50, 2), (51, 3), (70, 3), (71, 4)] {
        let rq = ItemRequest {
            ilvl,
            ..Default::default()
        };
        let mut it = item(i, seed);
        it.quality = q::CRAFTED;
        assert_eq!(crafted(&t, &mut it, &rq), Ok(true));
        let n = it
            .prefix
            .iter()
            .chain(&it.suffix)
            .filter(|&&a| a != 0)
            .count();
        assert_eq!(n, want, "ilvl {ilvl}");
    }
}

/// Success clears only the identified flag.
#[test]
fn magic_clears_identified_only() {
    let (t, i) = rare_tables();
    let mut it = item(i, 1);
    it.quality = q::MAGIC;
    it.flags = flag::IDENTIFIED | flag::INSTORE | flag::INIT;
    let rq = ItemRequest {
        prefix: [1, 0, 0],
        ..Default::default()
    };
    assert!(magic(&t, &mut it, &rq));
    assert_eq!(it.flags, flag::INSTORE | flag::INIT);
}

/// A quantity > 0 replaces
/// the gold base; 0 or less keeps the roll.
#[test]
fn gold_quantity_override() {
    let mut t = tables();
    let g = push_item(&mut t, item_rec(ty::GOLD, b"gld "));
    for (ovr, replaced) in [(77, true), (0, false), (-5, false)] {
        let mut it = item(g, 1);
        it.ilvl = 10;
        it.unit_seed = Seed::init_low(9);
        let mut rq = ItemRequest {
            ilvl: 10,
            quantity_override: ovr,
            ..Default::default()
        };
        init_item_stats(&t, &mut FakeGame::default(), &mut it, Some(&mut rq), false).unwrap();
        let rolled = Seed::init_low(9).roll(50) as i32 + 10;
        let expect = if replaced { ovr } else { rolled };
        assert_eq!(it.stats.base(stat::GOLD, 0), expect, "override {ovr}");
    }
}

/// (Other): spawn stack equal to min stack keeps hi
/// = spawn stack (only `<` or 0 falls back), so r(0) draws nothing.
#[test]
fn misc_stack_spawnstack_equal_to_min() {
    let mut t = tables();
    let mut r = item_rec(RING, b"rin ");
    r.stackable = 1;
    r.minstack = 5;
    r.spawnstack = 5;
    r.maxstack = 20;
    let i = push_item(&mut t, r);
    let mut it = item(i, 1);
    it.unit_seed = Seed::init_low(4);
    init_item_stats(&t, &mut FakeGame::default(), &mut it, None, false).unwrap();
    assert_eq!(it.stats.base(stat::QUANTITY, 0), 5);
    assert_eq!(it.unit_seed, Seed::init_low(4), "r(0) draws nothing");
}

/// Staff tables with skills whose `itypea1` cycles through a fitting type,
/// a non-fitting type, 0 (always), 1 (a type the staff is not) and −1
/// (< 1: always).
fn staff_tables() -> (ItemTables, usize) {
    let mut t = tables();
    t.itemtypes[ty::STAF as usize].staffmods = 1;
    let i = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.skill_lists.counts[1] = 30;
    t.skill_lists.max = 30;
    t.skill_lists.lists = vec![0; 7 * 30];
    t.skill_lists.lists[30] = 36;
    t.skills = (0..80)
        .map(|k| SkillRec {
            charclass: 0xFF,
            itypea1: [ty::STAF as i16, ty::HELM as i16, 0, 1, -1][k % 5],
            reqlevel: 1,
            maxlvl: 20,
        })
        .collect();
    (t, i)
}

/// Written from the rules, apart from the code under test.
fn staffmods_model(
    t: &ItemTables,
    rec: usize,
    rq: &ItemRequest,
    format: u16,
    quality: u8,
    mut s: Seed,
) -> (Seed, BTreeMap<(u16, u16), i32>) {
    let mut out = BTreeMap::new();
    let pct = |s: &mut Seed| (s.step() % 100) as i32;
    let bonus = if rq.flags2 & 0x20 != 0 { rq.ilvl } else { 0 };
    let v = pct(&mut s) + bonus;
    let count = match v {
        v if v > 90 => 3,
        v if v > 70 => 2,
        v if v > 30 => 1,
        _ if bonus != 0 => 1,
        _ => return (s, out),
    };
    let l = rq.ilvl;
    let base: i32 = match () {
        _ if l > 36 && format >= 100 => 5,
        _ if l > 24 => 4,
        _ if l > 18 => 3,
        _ if l > 11 => 2,
        _ => 1,
    };
    let mut chosen = Vec::new();
    for _ in 0..count {
        let p = pct(&mut s);
        let mut tier = match p {
            p if p > 80 => base + 1,
            p if p > 30 => base,
            p if p > 10 => base - 1,
            _ => base - 2,
        };
        tier = tier.max(1);
        if quality == q::LOW && tier > 3 {
            tier = 4;
        }
        let mut skill = 0u16;
        for _ in 0..6 {
            skill = 36 + (5 * (tier - 1)) as u16 + (s.step() % 5) as u16;
            let fits = t
                .skills
                .get(usize::from(skill))
                .is_none_or(|r| r.itypea1 < 1 || t.is_type(rec, r.itypea1));
            if fits && !chosen.contains(&skill) {
                chosen.push(skill);
                break;
            }
        }
        let value = if format < 100 || quality != q::LOW {
            match pct(&mut s) + bonus / 2 {
                v if v >= 90 => 3,
                v if v >= 60 => 2,
                _ => 1,
            }
        } else {
            1
        };
        out.insert((stat::ITEM_SINGLESKILL, skill), value);
    }
    (s, out)
}

/// Every threshold at its boundary (ilvl 11/12,
/// 18/19, 24/25, 36/37; format 100; enough seeds for every pct value),
/// and the `itypea1` < 1 test, against the model.
#[test]
fn staffmods_boundaries() {
    let (t, i) = staff_tables();
    // Six seeds for every first pct value 0–99.
    let seeds: Vec<u32> = (0..100)
        .flat_map(|p| {
            (0u32..)
                .filter(move |&x| Seed::init_low(x).step() % 100 == p)
                .take(6)
        })
        .collect();
    let mut first_pcts = std::collections::BTreeSet::new();
    for ilvl in [11, 12, 18, 19, 24, 25, 36, 37] {
        for flags2 in [0, req::STAFFMODS_ILVL] {
            for format in [2u16, 100, 101] {
                for quality in [q::LOW, q::NORMAL] {
                    for &seed in &seeds {
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
                            staffmods_model(&t, i, &rq, format, quality, Seed::init_low(seed));
                        let ctx = (ilvl, flags2, format, quality, seed);
                        assert_eq!(it.item_seed, s, "{ctx:?}");
                        let got = it
                            .stats
                            .lists
                            .get(&ListKey::ITEM)
                            .cloned()
                            .unwrap_or_default();
                        assert_eq!(got, want, "{ctx:?}");
                        first_pcts.insert(Seed::init_low(seed).step() % 100);
                    }
                }
            }
        }
    }
    assert_eq!(first_pcts.len(), 100, "every first pct value is exercised");
}

/// Socket helm: 3 × 4 inventory, 6 gem sockets, maxsock 6 at every ilvl.
fn big_socket_helm(t: &mut ItemTables) -> usize {
    let mut r = item_rec(ty::HELM, b"cap ");
    r.hasinv = 1;
    r.gemsockets = 6;
    r.invwidth = 3;
    r.invheight = 4;
    let h = &mut t.itemtypes[ty::HELM as usize];
    (h.maxsock1, h.maxsock25, h.maxsock40) = (6, 6, 6);
    push_item(t, r)
}

/// Quality caps magic 4, rare 2, crafted and
/// tempered 3; other qualities keep the w × h cap.
#[test]
fn socket_count_quality_caps() {
    let mut t = tables();
    let i = big_socket_helm(&mut t);
    for (quality, want) in [
        (q::NORMAL, 6),
        (q::SUPERIOR, 6),
        (q::MAGIC, 4),
        (q::RARE, 2),
        (q::CRAFTED, 3),
        (q::TEMPERED, 3),
    ] {
        let mut it = item(i, 1);
        it.quality = quality;
        socket_count(&t, &mut it, 6);
        assert_eq!(
            it.stats.base(stat::NUMSOCKETS, 0),
            want,
            "quality {quality}"
        );
    }
}

/// Only format < 100 body armor stops before the
/// draw; format 100 draws.: p < 33 sockets, p = 33 does not.
#[test]
fn socket_roll_boundaries() {
    let mut t = tables();
    let mut r = item_rec(ty::TORS, b"qui ");
    r.hasinv = 1;
    r.gemsockets = 4;
    r.invwidth = 2;
    r.invheight = 3;
    let tors = push_item(&mut t, r);
    let tt = &mut t.itemtypes[ty::TORS as usize];
    (tt.maxsock1, tt.maxsock25, tt.maxsock40) = (4, 4, 4);
    let game = FakeGame::default();
    let rq = ItemRequest::default();
    for (format, draws) in [(99u16, false), (100, true)] {
        let mut it = item(tors, 5);
        it.format = format;
        it.quality = q::NORMAL;
        socket_roll(&t, &game, &mut it, &rq);
        assert_eq!(it.item_seed != Seed::init_low(5), draws, "format {format}");
    }
    for (p, want) in [(32, true), (33, false)] {
        let seed = find_seed(|s| s.roll(100) == p);
        let mut it = item(tors, seed);
        it.quality = q::NORMAL;
        socket_roll(&t, &game, &mut it, &rq);
        assert_eq!(it.flags & flag::SOCKETED != 0, want, "p {p}");
    }
}

fn forced_rq(i: usize, flags1: u32) -> ItemRequest {
    ItemRequest {
        item: i as i32,
        format: 101,
        force: true,
        quality: q::NORMAL,
        flags1,
        ..Default::default()
    }
}

/// A forced request copies exactly the named
/// bits from flags1; other flags stay as creation set them.
#[test]
fn forced_flag_copies() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let copied = flag::IDENTIFIED | flag::NOSELL;
    for flags1 in [0, copied, copied | flag::ETHEREAL | flag::NAMED] {
        let mut rq = forced_rq(i, flags1);
        let c = create_item(
            &t,
            &mut FakeGame::default(),
            &mut rq,
            false,
            FakeStats::default(),
            0,
        )
        .unwrap();
        assert_eq!(
            c.item.flags,
            flag::INIT | flags1 & copied,
            "flags1 {flags1:#x}"
        );
    }
}

/// A forced ear takes the name, ear level and the
/// 0x8000 bit of flags1; a player's ear takes the client's hardcore flag.
/// Other flags are untouched.
#[test]
fn ear_named_flag_copy() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::PLAY, b"ear "));
    let base = flag::INIT | flag::EAR;
    for flags1 in [0, flag::NAMED] {
        let mut rq = forced_rq(i, flags1);
        rq.name = *b"forced-ear-name\0";
        rq.ear_level = 33;
        let c = create_item(
            &t,
            &mut FakeGame::default(),
            &mut rq,
            false,
            FakeStats::default(),
            0,
        )
        .unwrap();
        assert_eq!(c.item.flags, base | flags1, "flags1 {flags1:#x}");
        assert_eq!(c.item.name, rq.name);
        assert_eq!(c.item.ear_level, 33);
    }
    for (hc, named) in [(Some(true), flag::NAMED), (Some(false), 0), (None, 0)] {
        let mut rq = ItemRequest {
            item: i as i32,
            format: 101,
            quality: q::NORMAL,
            unit: Some(RequestUnit {
                class: 3,
                player: Some(PlayerInfo {
                    name: *b"player-name\0\0\0\0\0",
                    level: 12,
                    hardcore: hc,
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
        assert_eq!(
            c.item.flags,
            base | flag::INSTORE | flag::IDENTIFIED | named,
            "hardcore {hc:?}"
        );
        assert_eq!(c.item.ear_level, 12);
    }
}

fn run_prop(t: &ItemTables, it: &mut Item<FakeStats>, r: PropRec) {
    use crate::items::props::{apply_property, PropCtx};
    apply_property(t, it, &mut PropCtx::item(0), &r);
}

/// Max < min swaps the bounds (min 15, max 5 →
/// 5 + roll(11)), on several seeds.
#[test]
fn value_roll_swaps_bounds() {
    use crate::items::props::roll_value;
    for seed in 0..20 {
        let mut s = Seed::init_low(seed);
        let want = 5 + Seed::init_low(seed).roll(11) as i32;
        assert_eq!(roll_value(&mut s, 15, 5), want, "seed {seed}");
    }
}

/// With set ≠ 0, stat 58 sets stat 326 to 1 only
/// when it is 0; a set of another stat leaves 326 alone.
#[test]
fn poison_count_on_set() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut p58 = prop1(1, stat::POISONMAXDAM);
    p58.slots[0].set = 1;
    let mut p57 = prop1(1, 57);
    p57.slots[0].set = 1;
    t.properties = vec![p58, p57];
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 0, 4, 4));
    assert_eq!(it.stats.item_list(stat::POISONMAXDAM, 0), 4);
    assert_eq!(it.stats.item_list(stat::POISON_COUNT, 0), 1);
    let mut it = item(i, 1);
    it.stats.list_set(ListKey::ITEM, stat::POISON_COUNT, 0, 5);
    run_prop(&t, &mut it, rec(0, 0, 4, 4));
    assert_eq!(it.stats.item_list(stat::POISON_COUNT, 0), 5);
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(1, 0, 4, 4));
    assert_eq!(it.stats.item_list(57, 0), 4);
    assert!(!it.stats.lists[&ListKey::ITEM].contains_key(&(stat::POISON_COUNT, 0)));
}

/// The damage base resets apply to weapons only; a
/// non-weapon with damage columns keeps its base stats (function 7).
#[test]
fn base_reset_damage_weapon_only() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    (r.mindam, r.maxdam, r.mindam2, r.maxdam2) = (3, 7, 4, 8);
    let i = push_item(&mut t, r);
    t.properties = vec![prop1(7, stat::MAXDAMAGE_PERCENT)];
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 0, 20, 20));
    for s in [21, 22, 23, 24] {
        assert_eq!(it.stats.base(s, 0), 0, "stat {s}");
    }
    assert_eq!(it.stats.item_list(stat::MAXDAMAGE_PERCENT, 0), 20);
}

/// With `max` = 0 the level is capped at the
/// skill's `maxlvl` when it is ≥ 1 (here 1), at 20 only when < 1.
#[test]
fn skill_event_level_cap_one() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(11, 195)];
    t.skills = vec![SkillRec {
        charclass: 0xFF,
        itypea1: 0,
        reqlevel: 1,
        maxlvl: 1,
    }];
    let mut it = item(i, 1);
    it.ilvl = 50;
    run_prop(&t, &mut it, rec(0, 0, 7, 0));
    assert_eq!(
        it.stats.lists[&ListKey::ITEM],
        BTreeMap::from([((195, 1), 7)])
    );
}

/// The item list as a map (empty when there is none).
fn list(it: &Item<FakeStats>) -> BTreeMap<(u16, u16), i32> {
    it.stats
        .lists
        .get(&ListKey::ITEM)
        .cloned()
        .unwrap_or_default()
}

/// Function 5 skips stat 21 when a weapon has only
/// two-handed damage, 23 when it has only one-handed damage, 159 for a
/// non-throwing weapon; a non-weapon gets all three.
#[test]
fn func5_target_stats() {
    let mut t = tables();
    t.properties = vec![prop1(5, stat::MINDAMAGE)];
    let mk = |t: &mut ItemTables, ty: u16, one: u8, two: u8| {
        let mut r = item_rec(ty, b"xxx ");
        r.mindam = one;
        r.mindam2 = two;
        push_item(t, r)
    };
    let two_only = mk(&mut t, AXE, 0, 4);
    let one_only = mk(&mut t, AXE, 4, 0);
    let both = mk(&mut t, AXE, 4, 4);
    let thrown = mk(&mut t, THROWN, 4, 0);
    let helm = mk(&mut t, ty::HELM, 0, 0);
    for (i, want) in [
        (two_only, vec![23]),
        (one_only, vec![21]),
        (both, vec![21, 23]),
        (thrown, vec![21, 159]),
        (helm, vec![21, 23, 159]),
    ] {
        let mut it = item(i, 1);
        run_prop(&t, &mut it, rec(0, 0, 2, 2));
        let got: Vec<u16> = list(&it).keys().map(|&(s, _)| s).collect();
        assert_eq!(got, want, "item {i}");
        assert!(list(&it).values().all(|&v| v == 2));
    }
}

/// Function 6's floor applies only when column + v
/// < 1 (then −column); column 3, v −2 sums to 1 and adds −2. Function 5
/// writes min damage only.
#[test]
fn func6_floor_boundary_and_func5_min_only() {
    let mut t = tables();
    let mut r = item_rec(AXE, b"axe ");
    (r.mindam, r.maxdam) = (2, 3);
    let i = push_item(&mut t, r);
    t.properties = vec![prop1(6, stat::MAXDAMAGE), prop1(5, stat::MINDAMAGE)];
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 0, -2, -2));
    assert_eq!(list(&it), BTreeMap::from([((22, 0), -2)]));
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 0, -5, -5));
    assert_eq!(list(&it), BTreeMap::from([((22, 0), -3)]));
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(1, 0, 1, 1));
    assert_eq!(list(&it), BTreeMap::from([((21, 0), 1)]));
}

/// Chance := `min`, 5 only when `min` < 1.
#[test]
fn skill_event_chance_one() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(11, 195)];
    t.skills = vec![SkillRec {
        charclass: 0xFF,
        itypea1: 0,
        reqlevel: 1,
        maxlvl: 20,
    }];
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 0, 1, 3));
    assert_eq!(list(&it), BTreeMap::from([((195, 3), 1)]));
}

/// `property-functions.tsv` (function 13: base reset in mode 1 only).
#[test]
fn func13_reset_mode_one_only() {
    use crate::items::props::{apply_property, PropCtx};
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    (r.minac, r.maxac) = (3, 9);
    let i = push_item(&mut t, r);
    t.properties = vec![prop1(13, stat::ARMORCLASS)];
    for (mode, base) in [(0u8, 0), (1, 10)] {
        let mut it = item(i, 1);
        apply_property(&t, &mut it, &mut PropCtx::item(mode), &rec(0, 0, 2, 2));
        assert_eq!(it.stats.base(stat::ARMORCLASS, 0), base, "mode {mode}");
        assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 2);
    }
}

/// C > 254 → 255; c = 254 stays.
#[test]
fn func19_charges_254() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.properties = vec![prop1(19, 204)];
    t.skills = vec![SkillRec::default(); 4];
    let mut it = item(i, 7);
    run_prop(&t, &mut it, rec(0, 3, 254, 1));
    let r = Seed::init_low(7).roll(254 - 254 / 8) as i32;
    let want = 254 * 256 + ((r + 254 / 8 + 1) & 0xFF);
    assert_eq!(list(&it), BTreeMap::from([((204, (3 << 6) + 1), want)]));
}

/// `property-functions.tsv` layers: 9 and 22 take `param`, 21 the slot's
/// `val`.
#[test]
fn generic_layers_param_and_val() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut p21 = prop1(21, 40);
    p21.slots[0].val = 5;
    t.properties = vec![prop1(9, 41), prop1(22, 42), p21];
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 7, 2, 2));
    run_prop(&t, &mut it, rec(1, 8, 3, 3));
    run_prop(&t, &mut it, rec(2, 9, 4, 4));
    assert_eq!(
        list(&it),
        BTreeMap::from([((40, 5), 4), ((41, 7), 2), ((42, 8), 3)])
    );
}

/// Cap from `invwidth` × `invheight`; a roll < 1
/// uses `param`, a roll of 1 stays 1; a cap of 1 still sets 1.
#[test]
fn func14_cap_and_param() {
    let mut t = tables();
    let mk = |t: &mut ItemTables, w: u8, h: u8| {
        let mut r = item_rec(ty::HELM, b"cap ");
        (r.invwidth, r.invheight, r.gemsockets) = (w, h, 6);
        push_item(t, r)
    };
    let i23 = mk(&mut t, 2, 3);
    let i11 = mk(&mut t, 1, 1);
    let h = &mut t.itemtypes[ty::HELM as usize];
    (h.maxsock1, h.maxsock25, h.maxsock40) = (6, 6, 6);
    t.properties = vec![prop1(14, stat::NUMSOCKETS)];
    for (i, r, want) in [
        (i23, rec(0, 0, 6, 6), 6),
        (i23, rec(0, 4, 1, 1), 1),
        (i23, rec(0, 3, 0, 0), 3),
        (i11, rec(0, 0, 4, 4), 1),
    ] {
        let mut it = item(i, 1);
        run_prop(&t, &mut it, r);
        assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), want, "{r:?}");
        assert_ne!(it.flags & flag::SOCKETED, 0);
    }
}

/// Partial record k goes to state 165 + k / 2.
#[test]
fn set_partial_state_per_record() {
    use crate::items::tables::SetItemRec;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = (0..10).map(|k| prop1(1, 100 + k)).collect();
    let mut aprops = [PropRec::NONE; 10];
    for (k, a) in aprops.iter_mut().enumerate() {
        *a = rec(k as i32, 0, 1, 1);
    }
    t.setitems = vec![SetItemRec {
        add_func: 1,
        props: [PropRec::NONE; 9],
        aprops,
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.file_index = 0;
    crate::items::props::apply_set_item(&t, &mut it);
    for k in 0..10u16 {
        let key = ListKey {
            state: 165 + k / 2,
            flags: 0x2040,
        };
        assert_eq!(it.stats.list_get(key, 100 + k, 0), 1, "record {k}");
    }
}

/// Quest items and items without an inventory get
/// no runeword; up to six fillers; the socket count must equal the filler
/// count; the rune list must cover the fillers.
#[test]
fn runeword_match_conditions() {
    use crate::items::props::runeword_match;
    use crate::items::tables::RuneRec;
    let mut t = tables();
    let mut r = item_rec(AXE, b"axe ");
    r.hasinv = 1;
    let base = push_item(&mut t, r.clone());
    r.quest = 1;
    let quest = push_item(&mut t, r.clone());
    r.quest = 0;
    r.hasinv = 0;
    let noinv = push_item(&mut t, r);
    let rune = push_item(&mut t, item_rec(ty::RUNE, b"r01 ")) as i32;
    t.runes = vec![
        RuneRec {
            complete: 1,
            itype: [ty::WEAP as i16, 0, 0, 0, 0, 0],
            runes: [rune; 6],
            ..Default::default()
        },
        RuneRec {
            complete: 1,
            itype: [ty::WEAP as i16, 0, 0, 0, 0, 0],
            runes: [rune, 0, 0, 0, 0, 0],
            ..Default::default()
        },
    ];
    let mk = |i: usize, sockets: i32| {
        let mut it = item(i, 1);
        it.quality = q::NORMAL;
        it.stats.set_base(stat::NUMSOCKETS, 0, sockets);
        it
    };
    let r = rune as usize;
    assert_eq!(runeword_match(&t, &mk(base, 6), &[r; 6]), Some(0));
    assert_eq!(runeword_match(&t, &mk(base, 1), &[r]), Some(1));
    // Two fillers: row 0 lists six, row 1 only one rune.
    assert_eq!(runeword_match(&t, &mk(base, 2), &[r; 2]), None);
    assert_eq!(runeword_match(&t, &mk(base, 2), &[r]), None);
    assert_eq!(runeword_match(&t, &mk(quest, 1), &[r]), None);
    assert_eq!(runeword_match(&t, &mk(noinv, 1), &[r]), None);
}

/// A `server` row needs the ladder flag; others
/// activate without it.
#[test]
fn runeword_server_rows() {
    use crate::items::props::activate_runeword;
    use crate::items::tables::RuneRec;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(AXE, b"axe "));
    t.runes = vec![
        RuneRec {
            server: 1,
            ..Default::default()
        },
        RuneRec::default(),
    ];
    for (row, ladder, want) in [
        (0, false, false),
        (0, true, true),
        (1, false, true),
        (1, true, true),
    ] {
        let (mut filler, mut it) = (item(i, 1), item(i, 1));
        assert_eq!(
            activate_runeword(&t, &mut filler, &mut it.stats, row, ladder),
            want,
            "{row} {ladder}"
        );
    }
}

/// N = min(c, count − 1); a full 3-item set takes
/// the first 2n − 2 = 2 partial records plus the full records.
#[test]
fn set_bonus_full_set() {
    use crate::items::props::set_bonuses;
    use crate::items::tables::{SetItemRec, SetRec};
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = (0..16).map(|k| prop1(1, 100 + k)).collect();
    let mut partial = [PropRec::NONE; 8];
    for (k, p) in partial.iter_mut().enumerate() {
        *p = rec(k as i32, 0, 1, 1);
    }
    let mut full = [PropRec::NONE; 8];
    full[0] = rec(15, 0, 1, 1);
    t.sets = vec![SetRec {
        count: 3,
        version: 0,
        partial,
        full,
    }];
    t.setitems = vec![SetItemRec::default()];
    let mut it = item(i, 1);
    it.file_index = 0;
    let key = ListKey {
        state: 1,
        flags: 0x40,
    };
    let mut owner = FakeStats::default();
    set_bonuses(&t, &mut it, 0b111, &mut owner, key);
    let got: Vec<u16> = owner.lists[&key].keys().map(|&(s, _)| s).collect();
    assert_eq!(got, vec![100, 101, 115]);
}

/// A non-ethereal item is not made ethereal.
#[test]
fn craft_list_non_ethereal() {
    use crate::items::props::apply_craft_list;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.properties = vec![prop1(1, 7)];
    let mut it = item(i, 1);
    it.stats.set_base(stat::ARMORCLASS, 0, 10);
    apply_craft_list(&t, &mut it, &[rec(0, 0, 5, 5)]);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 10);
    assert_eq!(it.flags & flag::ETHEREAL, 0);
}

fn ratio_rows() -> Vec<Itemratio> {
    [(0, 0), (1, 0), (0, 1), (1, 1)]
        .into_iter()
        .map(|(cs, uber)| {
            let mut r = Itemratio::decode(&[0u8; Itemratio::SIZE]);
            r.version = 1;
            r.class_specific = cs;
            r.uber = uber;
            r
        })
        .collect()
}

/// The item-type test as the quality roll uses it: `Class Specific`
/// is itemtypes `class` < 7; `Uber` needs a weapon or armor whose code is
/// its `ubercode` or `ultracode`, not of type 38 and not a quest item.
#[test]
fn ratio_row_class_and_uber() {
    use crate::items::quality::ratio_row;
    let mut t = tables();
    t.itemratio = ratio_rows();
    t.itemtypes[ty::GLOV as usize].class = 3;
    t.itemtypes[ty::BOOT as usize].class = 7;
    let mut ax = item_rec(AXE, b"axe ");
    ax.ubercode = *b"axe ";
    let uber_axe = push_item(&mut t, ax.clone());
    ax.quest = 1;
    let quest_axe = push_item(&mut t, ax);
    let mut h = item_rec(ty::HELM, b"cap ");
    h.ultracode = *b"cap ";
    let ultra_helm = push_item(&mut t, h);
    let plain_helm = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    let mut rg = item_rec(RING, b"rin ");
    rg.ubercode = *b"rin ";
    let ring = push_item(&mut t, rg);
    let gloves = push_item(&mut t, item_rec(ty::GLOV, b"glv "));
    let boots = push_item(&mut t, item_rec(ty::BOOT, b"bts "));
    for (i, want) in [
        (uber_axe, 2),
        (quest_axe, 0),
        (ultra_helm, 2),
        (plain_helm, 0),
        (ring, 0),
        (gloves, 1),
        (boots, 0),
    ] {
        assert_eq!(ratio_row(&t, i, 100), Some(want), "item {i}");
    }
}

/// L := max(ilvl − items `level`, 1) for a non-misc
/// item. Unique 15/1: ilvl 30, level 20 → c = 5, one roll(5); the other
/// steps have c < 1.
#[test]
fn quality_roll_level_difference() {
    use crate::items::quality::roll_quality;
    let mut t = tables();
    let mut r = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    r.version = 1;
    (r.unique, r.uniquedivisor) = (15, 1);
    (r.raredivisor, r.setdivisor, r.magicdivisor) = (1, 1, 1);
    (r.hiqualitydivisor, r.normaldivisor) = (1, 1);
    t.itemratio = vec![r];
    let mut h = item_rec(ty::HELM, b"cap ");
    h.level = 20;
    let i = push_item(&mut t, h);
    for seed in 0..20 {
        let mut it = item(i, seed);
        let rq = ItemRequest {
            ilvl: 30,
            ..Default::default()
        };
        let mut s = Seed::init_low(seed);
        let want = if s.roll(5) == 0 { q::UNIQUE } else { q::RARE };
        assert_eq!(roll_quality(&t, &mut it, &rq), Ok(want));
        assert_eq!(it.item_seed, s);
    }
}

/// Only quality 6 becomes 4 when itemtype `rare` is
/// 0; a normal request stays normal.
#[test]
fn rare_override_only_for_rare() {
    use crate::items::quality::dispatch;
    let mut t = tables();
    t.itemtypes[ty::HELM as usize].rare = 0;
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    // Magic affixes exist, so a wrong magic quality would stick.
    t.magic = vec![affix_row(ty::HELM, 1), affix_row(ty::HELM, 2)];
    t.n_suffix = 1;
    t.n_prefix = 1;
    let mut it = item(i, 1);
    let mut rq = ItemRequest {
        quality: q::NORMAL,
        ..Default::default()
    };
    assert_eq!(
        dispatch(&t, &mut FakeGame::default(), &mut it, &mut rq),
        Ok(true)
    );
    assert_eq!(it.quality, q::NORMAL);
    assert_eq!((it.prefix, it.suffix), ([0; 3], [0; 3]));
}

/// Each type column fits its own types only; `weapon`
/// and `armor` exclude the listed types.
#[test]
fn superior_fits_matrix() {
    use crate::items::quality::superior_fits;
    use crate::items::tables::QualityRec;
    let mut t = tables();
    let items: Vec<(u16, usize)> = [
        AXE,
        ty::HELM,
        ty::SHIE,
        ty::SCEP,
        ty::WAND,
        ty::STAF,
        ty::BOW,
        ty::XBOW,
        ty::BOOT,
        ty::GLOV,
        ty::BELT,
        RING,
    ]
    .into_iter()
    .map(|tp| (tp, push_item(&mut t, item_rec(tp, b"xxx "))))
    .collect();
    let col = |k: usize| {
        let mut r = QualityRec::default();
        *[
            &mut r.weapon,
            &mut r.armor,
            &mut r.shield,
            &mut r.scepter,
            &mut r.wand,
            &mut r.staff,
            &mut r.bow,
            &mut r.boots,
            &mut r.gloves,
            &mut r.belt,
        ][k] = 1;
        r
    };
    t.qualityitems = (0..10).map(col).collect();
    let fits: [&[u16]; 10] = [
        &[AXE],
        &[ty::HELM],
        &[ty::SHIE],
        &[ty::SCEP],
        &[ty::WAND],
        &[ty::STAF],
        &[ty::BOW, ty::XBOW],
        &[ty::BOOT],
        &[ty::GLOV],
        &[ty::BELT],
    ];
    for (row, want) in fits.iter().enumerate() {
        for &(tp, i) in &items {
            let it = item(i, 1);
            assert_eq!(
                superior_fits(&t, &it, row),
                want.contains(&tp),
                "row {row} type {tp}"
            );
        }
    }
}

fn unique_row(code: &[u8; 4]) -> UniqueRec {
    UniqueRec {
        code: *code,
        enabled: true,
        rarity: 1,
        lvl: 1,
        props: [PropRec::NONE; 12],
        ..Default::default()
    }
}

/// Success clears only the identified flag
/// (forced and rolled).
#[test]
fn unique_clears_identified_only() {
    use crate::items::quality::unique;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = vec![unique_row(b"rin ")];
    for force in [true, false] {
        let mut it = item(i, 1);
        it.flags = flag::IDENTIFIED | flag::INIT;
        let rq = ItemRequest {
            force,
            ..Default::default()
        };
        assert!(unique(&t, &mut FakeGame::default(), &mut it, &rq));
        assert_eq!(it.flags, flag::INIT, "force {force}");
        assert_eq!(it.file_index, 0);
    }
}

/// Candidates need `version` < 100 or format ≥ 100;
/// the ladder test passes with either ladder flag.
#[test]
fn unique_candidate_version_and_ladder() {
    use crate::items::quality::unique;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    for (version, format, ok) in [
        (99u16, 2u16, true),
        (100, 2, false),
        (100, 99, false),
        (100, 100, true),
        (0, 101, true),
    ] {
        let mut u = unique_row(b"rin ");
        u.version = version;
        t.uniques = vec![u];
        let mut it = item(i, 1);
        it.format = format;
        let got = unique(
            &t,
            &mut FakeGame::default(),
            &mut it,
            &ItemRequest::default(),
        );
        assert_eq!(got, ok, "version {version} format {format}");
    }
    let mut u = unique_row(b"rin ");
    u.ladder = true;
    t.uniques = vec![u];
    for (ladder, ok) in [
        ((false, false), false),
        ((true, false), true),
        ((false, true), true),
    ] {
        let mut game = FakeGame {
            ladder,
            ..Default::default()
        };
        let mut it = item(i, 1);
        let got = unique(&t, &mut game, &mut it, &ItemRequest::default());
        assert_eq!(got, ok, "ladder {ladder:?}");
    }
}

/// Without a preference (index 0) the pick draws.
#[test]
fn unique_no_preference_draws() {
    use crate::items::quality::unique;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = vec![unique_row(b"rin "), unique_row(b"rin ")];
    let seed = find_seed(|s| s.roll(2) == 0);
    let mut it = item(i, seed);
    assert!(unique(
        &t,
        &mut FakeGame::default(),
        &mut it,
        &ItemRequest::default()
    ));
    assert_eq!(it.file_index, 0);
    let mut s = Seed::init_low(seed);
    s.roll(2);
    assert_eq!(it.item_seed, s);
}

/// No candidate and items `unique` = 0 → file index
/// −1, failure.
#[test]
fn unique_no_candidate_file_index() {
    use crate::items::quality::unique;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut it = item(i, 1);
    it.file_index = 5;
    assert!(!unique(
        &t,
        &mut FakeGame::default(),
        &mut it,
        &ItemRequest::default()
    ));
    assert_eq!(it.file_index, -1);
}

/// Index 4096 is markable; a quest item ignores a set
/// bit but not an index above 4096.
#[test]
fn unique_marking_bounds_and_quest() {
    use crate::items::quality::unique;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut q_rec = item_rec(RING, b"qst ");
    q_rec.quest = 1;
    let qi = push_item(&mut t, q_rec);
    t.uniques = vec![unique_row(b"zzz "); 4098];
    t.uniques[4096] = unique_row(b"rin ");
    let mut game = FakeGame::default();
    let mut it = item(i, 1);
    assert!(unique(&t, &mut game, &mut it, &ItemRequest::default()));
    assert_eq!(it.file_index, 4096);
    assert!(game.uniques.get(4096));
    // Quest item: bit 5 already set → still accepted and marked.
    t.uniques[4096] = unique_row(b"zzz ");
    t.uniques[5] = unique_row(b"qst ");
    let mut game = FakeGame::default();
    game.uniques.set(5);
    let mut it = item(qi, 1);
    assert!(unique(&t, &mut game, &mut it, &ItemRequest::default()));
    assert_eq!(it.file_index, 5);
    // Quest item at index 4097: accepted, but the mark fails.
    t.uniques[5] = unique_row(b"zzz ");
    t.uniques[4097] = unique_row(b"qst ");
    let mut game = FakeGame::default();
    let mut it = item(qi, 1);
    assert!(!unique(&t, &mut game, &mut it, &ItemRequest::default()));
    assert_eq!(it.file_index, -1);
}

/// Set candidates need version < 100 or format ≥ 100.
#[test]
fn set_candidate_version() {
    use crate::items::quality::set_item;
    use crate::items::tables::SetItemRec;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    for (version, format, ok) in [
        (99u16, 2u16, true),
        (100, 2, false),
        (100, 99, false),
        (100, 100, true),
        (0, 101, true),
    ] {
        t.setitems = vec![SetItemRec {
            item: *b"rin ",
            version,
            lvl: 1,
            ..Default::default()
        }];
        let mut it = item(i, 1);
        it.format = format;
        assert_eq!(
            set_item(&t, &mut it, &ItemRequest::default()),
            ok,
            "{version} {format}"
        );
    }
}

/// The skills projection keeps `itypea1` (link16 read as i16, so 0xFFFF
/// is −1, "< 1"), `reqlevel` and `maxlvl`.
#[test]
fn skill_record_projection() {
    use d2_data::tables::Skills;
    let mut s = Skills::decode(&[0u8; Skills::SIZE]);
    (s.itypea1, s.reqlevel, s.maxlvl, s.charclass) = (0xFFFF, 12, 20, 3);
    assert_eq!(
        SkillRec::from(&s),
        SkillRec {
            charclass: 3,
            itypea1: -1,
            reqlevel: 12,
            maxlvl: 20
        }
    );
}

/// `type2` counts only when > 0 (here row 0 of the
/// matrix is made equivalent to the ring type to expose a 0 `type2`).
#[test]
fn type2_zero_not_tested() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.equiv.bits[RING as usize / 32] |= 1 << (RING % 32);
    assert!(t.equiv.get(0, RING as usize));
    assert!(!t.is_type(i, RING as i16));
}

/// The target-list stat set as the property functions use it: a stat id equal
/// to the stat count is out of range and writes nothing.
#[test]
fn stat_id_at_count_is_invalid() {
    let mut t = tables();
    let n = t.valshift.len() as u16;
    assert!(t.stat_valid(n - 1));
    assert!(!t.stat_valid(n));
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, n)];
    let mut it = item(i, 1);
    run_prop(&t, &mut it, rec(0, 0, 3, 3));
    assert!(it.stats.lists.is_empty());
}

/// Walk the candidates subtracting weights until r <
/// weight. Weights 1, 3, 2: r 0 → first, 1–3 → second, 4–5 → third.
#[test]
fn set_pick_by_weight() {
    use crate::items::quality::set_item;
    use crate::items::tables::SetItemRec;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.setitems = [1, 3, 2]
        .into_iter()
        .map(|rarity| SetItemRec {
            item: *b"rin ",
            rarity,
            lvl: 1,
            ..Default::default()
        })
        .collect();
    for (r, want) in [(0, 0), (1, 1), (2, 1), (3, 1), (4, 2), (5, 2)] {
        let seed = find_seed(|s| s.roll(6) == r);
        let mut it = item(i, seed);
        assert!(set_item(&t, &mut it, &ItemRequest::default()));
        assert_eq!(it.file_index, want, "r {r}");
    }
}

/// A preferred row (index − 1) is picked without
/// a draw, even when later candidates exist.
#[test]
fn unique_preferred_first_row() {
    use crate::items::quality::unique;
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.uniques = vec![unique_row(b"rin "), unique_row(b"rin ")];
    let rq = ItemRequest {
        index: 1,
        ..Default::default()
    };
    let mut it = item(i, 3);
    assert!(unique(&t, &mut FakeGame::default(), &mut it, &rq));
    assert_eq!(it.file_index, 0);
    assert_eq!(it.item_seed, Seed::init_low(3));
}
