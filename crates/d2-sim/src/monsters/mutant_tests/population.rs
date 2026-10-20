// unchecked; fakes from the parent test module)
use super::*;

use d2_data::bin::BinTable;
use d2_data::tables::{Levels, Monstats, Monstats2, Record, Superuniques};

use crate::monsters::population::data::{chain_lengths, composits};
use crate::monsters::population::{LevelPop, MonPop, RoomBox, SuperPop};
use crate::units::{RoomId, UnitId};

// ---- data readers (Constants & data dependencies) -----------------------

/// A zeroed record of `R` with `set` applied.
fn record<R: Record>(set: impl FnOnce(&mut [u8])) -> R {
    let mut b = vec![0u8; R::SIZE];
    set(&mut b);
    R::decode(&b)
}

/// A `.bin` table of `rows` records of `size` bytes.
fn bin(name: &str, size: usize, rows: &[Vec<u8>]) -> BinTable {
    BinTable {
        name: name.to_owned(),
        source: "test".to_owned(),
        count: rows.len(),
        record_size: size,
        records: rows.concat(),
    }
}

#[test]
fn levels_lists_stop_at_first_negative() {
    // fixups: the count is the entries before the first one < 0;
    // mon1 at +0x36, nmon1 at +0x68, umon1 at +0x9A (i16).
    let l: Levels = record(|b| {
        for (base, list) in [
            (0x36, [3u16, 4, 0xFFFF, 9]),
            (0x68, [7, 0xFFFF, 8, 9]),
            (0x9A, [0xFFFF, 1, 2, 3]),
        ] {
            for (i, v) in list.iter().enumerate() {
                b[base + 2 * i..base + 2 * i + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
    });
    let p = LevelPop::from_record(&l);
    assert_eq!(p.mon, [3, 4]);
    assert_eq!(p.nmon, [7]);
    assert!(p.umon.is_empty());
}

#[test]
fn monstats_links_are_signed() {
    // Constants table: BaseId (+0x02), NextInClass (+0x04), MonStatsEx
    // (+0x18) are i16 links; all-ones is −1.
    let m: Monstats = record(|b| {
        b[2..4].copy_from_slice(&7u16.to_le_bytes());
        b[4..6].copy_from_slice(&0xFFFFu16.to_le_bytes());
        b[0x18..0x1A].copy_from_slice(&2u16.to_le_bytes());
    });
    let p = MonPop::from_record(&m);
    assert_eq!((p.base_id, p.next_in_class, p.mon_stats_ex), (7, -1, 2));
}

#[test]
fn monstats2_and_superunique_columns() {
    // Constants table: monstats2 SizeX (+0x08, sign-extended),
    // spawnCol (+0x0A), TotalPieces (+0xEC), critter (flag 13), objCol
    // (flag 18; flags from +0x04).
    let m2: Monstats2 = record(|b| {
        b[0x08] = 0xFE;
        b[0x0A] = 1;
        b[0xEC] = 5;
        b[0x04 + 13 / 8] |= 1 << (13 % 8);
        b[0x04 + 18 / 8] |= 1 << (18 % 8);
    });
    let p = Mon2Pop::from_record(&m2);
    assert_eq!((p.size_x, p.spawn_col, p.total_pieces), (-2, 1, 5));
    assert!(p.critter && p.obj_col);
    // superuniques Class (+0x04), hcIdx (+0x08), MinGrp (+0x1C), MaxGrp
    // (+0x20), AutoPos (+0x24), Stacks (+0x26).
    let su: Superuniques = record(|b| {
        b[0x04..0x08].copy_from_slice(&58u32.to_le_bytes());
        b[0x08..0x0C].copy_from_slice(&10u32.to_le_bytes());
        b[0x1C..0x20].copy_from_slice(&2u32.to_le_bytes());
        b[0x20..0x24].copy_from_slice(&3u32.to_le_bytes());
        b[0x24] = 1;
        b[0x26] = 1;
    });
    assert_eq!(
        SuperPop::from_record(&su),
        SuperPop {
            class: 58,
            hc_idx: 10,
            min_grp: 2,
            max_grp: 3,
            auto_pos: 1,
            stacks: 1,
        }
    );
}

/// monstats.bin rows with chain length bytes; monstats2.bin rows with
/// composit counts and totals.
fn bins() -> (BinTable, BinTable) {
    let ms = |len: u8| {
        let mut r = vec![0u8; Monstats::SIZE];
        r[0x4A] = len;
        r
    };
    let ms2 = |first: u8, total: u8| {
        let mut r = vec![0u8; Monstats2::SIZE];
        for i in 0..16 {
            r[0x15 + i] = first + i as u8;
        }
        r[0x25] = total;
        r
    };
    (
        bin("monstats", Monstats::SIZE, &[ms(3), ms(7)]),
        bin("monstats2", Monstats2::SIZE, &[ms2(1, 49), ms2(20, 9)]),
    )
}

#[test]
fn bin_reader_offsets() {
    // fixups: chain length u8 at monstats +0x4A. callbacks: count u8
    // at 21 + i (0x15…0x24), total u8 at 37 (0x25).
    let (ms, ms2) = bins();
    assert_eq!(chain_lengths(&ms), [3, 7]);
    let c = composits(&ms2);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].0, core::array::from_fn(|i| 1 + i as u8));
    assert_eq!(c[0].1, 49);
    assert_eq!(c[1].0, core::array::from_fn(|i| 20 + i as u8));
    assert_eq!(c[1].1, 9);
}

#[test]
fn with_bins_fills_rows_and_keeps_the_rest() {
    let (ms, ms2) = bins();
    let mut t = PopTables {
        levels: vec![LevelPop::default(); 2],
        monstats: vec![MonPop::default(); 2],
        monstats2: vec![Mon2Pop::default(); 2],
        superuniques: vec![SuperPop::default(); 1],
    };
    t.monstats[1].base_id = 1;
    let t = t.with_bins(&ms, &ms2);
    assert_eq!((t.levels.len(), t.superuniques.len()), (2, 1));
    assert_eq!(t.monstats[1].base_id, 1);
    assert_eq!((t.monstats[0].chain_len, t.monstats[1].chain_len), (3, 7));
    assert_eq!(t.monstats2[1].components[0], 20);
    assert_eq!(t.monstats2[0].composit_total, 49);
}

// ---- / placement -------------------------------------------

#[test]
fn ring_start_negates_y_offset() {
    // R1–: p = 0 → offset (q, d), direction (+1, 0); sy = 1 negates
    // the y offset, so the walk starts on the top edge (y = −3).
    let s0 = (1..)
        .map(Seed::init_low)
        .find(|&s| {
            let mut s = s;
            let p = s.step() & 1;
            s.roll(3);
            s.step();
            p == 0 && s.step() & 1 == 1
        })
        .unwrap();
    let mut s = s0;
    s.step();
    let q = s.roll(3) as i32;
    let ox = if s.step() & 1 == 1 { -q } else { q };
    let mut cells = Vec::new();
    let mut s = s0;
    ring_search(&mut s, 0, 0, 1, |x, y| {
        cells.push((x, y));
        false
    });
    // |ox| ≤ 2: no corner at the start, one step right.
    assert_eq!(cells[0], (ox + 1, -3));
}

#[test]
fn water_point_needs_one_free_neighbour() {
    // The point passes when at least one of (0,−3), (3,0), (0,3),
    // (−3,0) is free; here only (−3, 0) is.
    let mut t = tables();
    t.monstats2.push(Mon2Pop {
        spawn_col: 1,
        ..Mon2Pop::default()
    });
    t.monstats[247].mon_stats_ex = 1;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    // roll(3) = 1 → s = 1: tile 1 first.
    f.room_seeds.insert(R0, seed_for(3, &[1]));
    f.tiles = (0..3)
        .map(|x| TileRec {
            water: true,
            x,
            y: 1,
        })
        .collect();
    f.blocked.extend([(8, 5), (11, 8), (8, 11)]);
    let u = place(&mut ctx(&t, &mut st, &mut f), req(247, 50, 50, -1, 0))
        .unit()
        .unwrap();
    assert_eq!((f.unit(u).x, f.unit(u).y), (8, 8));
}

#[test]
fn ring_bounds_exclude_bottom() {
    // (PtInRect): top ≤ y < bottom; the box is 0..1000.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    assert_eq!(
        place(&mut cx, req(5, 10, 1000, -1, flags::PROBE)),
        Placed::Failed
    );
    assert_eq!(
        place(&mut cx, req(5, 10, 999, -1, flags::PROBE)),
        Placed::Probe
    );
}

#[test]
fn guid_only_with_flag_0x20() {
    // 0x20 = allocate with the caller's GUID; without it, none.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    let r = SpawnReq {
        guid: Some(77),
        ..req(5, 10, 10, -1, 0)
    };
    place(&mut cx, r).unit().unwrap();
    place(
        &mut cx,
        SpawnReq {
            flags: flags::GUID,
            x: 20,
            ..r
        },
    )
    .unit()
    .unwrap();
    assert_eq!(f.log[0], "alloc 5 10 10 m1");
    assert!(f.log.contains(&"alloc 5 20 10 m1 g77".to_string()));
}

// ---- spawn point ----------------------------------------------------

#[test]
fn spawn_point_cl_height() {
    // With cl, top = y0 + 1 and h = y1 − top. cl tiles (2,2)-(4,10)
    // → subtiles 10..20 × 10..50: left 11, w 9, top 11, h 39.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    let p = spawn_point(&mut cx, R0, Some(rect(2, 2, 4, 10)), 5, false).unwrap();
    let mut s = Seed::init();
    let want = (s.roll(9) as i32 + 11, s.roll(39) as i32 + 11);
    assert_eq!(p, want);
    assert!(want.1 >= 15, "the draw separates h = 39 from smaller ones");
}

/// On a 2 × 2 room box at (10, 10): the only point is (11, 11).
fn warp_checked(dist: u32, warps: &[(i32, i32)], loc: Option<(i32, i32)>) -> Option<(i32, i32)> {
    let mut t = tables();
    t.levels[2].warp_dist = dist;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        RoomBox {
            x: 10,
            y: 10,
            width: 2,
            height: 2,
        },
    );
    f.warps = warps.to_vec();
    f.spawn_loc = loc;
    spawn_point(&mut ctx(&t, &mut st, &mut f), R0, None, 5, true)
}

#[test]
fn warp_distance_is_strict_squared() {
    // Reject if dx² + dy² < WarpDist. Warp (14, 15) from (11, 11):
    // 9 + 16 = 25.
    assert_eq!(warp_checked(25, &[(14, 15)], None), Some((11, 11)));
    assert_eq!(warp_checked(26, &[(14, 15)], None), None);
    // Kind-11 location in tiles × 5: (2, 3) → (10, 15), 1 + 16 = 17.
    assert_eq!(warp_checked(17, &[], Some((2, 3))), Some((11, 11)));
    assert_eq!(warp_checked(18, &[], Some((2, 3))), None);
    // Only when x > 0 and y > 0: y = 0 is ignored.
    assert_eq!(warp_checked(2025, &[], Some((1, 0))), Some((11, 11)));
}

// ---- presets ---------------------------------------------------------

#[test]
fn barricade_needs_class_and_objcol() {
    // ObjCol and class 432/433 both needed for the object.
    let mut t = tables();
    t.monstats2.push(Mon2Pop {
        obj_col: true,
        ..Mon2Pop::default()
    });
    t.monstats[100].mon_stats_ex = 1;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    preset_spawn(&mut cx, R0, 432, 10, 10, 1).unwrap();
    preset_spawn(&mut cx, R0, 100, 20, 20, 1).unwrap();
    assert_eq!(f.calls("barricade"), 0);
}

/// Superunique 0 (class 58, minion1 19) at (10, 10) with MinGrp/MaxGrp and
/// difficulty: (minions, boss seed after, boss seed at allocation).
fn su_minions(min: u32, max: u32, difficulty: u8) -> (usize, Seed, Seed) {
    let mut t = tables();
    t.superuniques[0] = SuperPop {
        class: 58,
        hc_idx: 0,
        min_grp: min,
        max_grp: max,
        auto_pos: 0,
        stacks: 1,
    };
    t.monstats[58].minion1 = 19;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    cx.info.difficulty = difficulty;
    let b = preset_spawn(&mut cx, R0, 600, 10, 10, 1).unwrap();
    let n = f.units.iter().filter(|u| u.class == 19).count();
    // The boss is the first unit: its seed is the first game-seed derive.
    (n, f.unit(b).seed, Seed::init().derive())
}

#[test]
fn superunique_minion_range() {
    // Both ≠ 0 → + difficulty;: count = roll(max − min
    // + 1) + min on the boss seed.
    let (n, after, bs) = su_minions(3, 3, 2);
    assert_eq!((n, after), (5, steps(bs, 1)));
    let (n, after, bs) = su_minions(3, 3, 1);
    assert_eq!((n, after), (4, steps(bs, 1)));
    // MinGrp 0: no difficulty added, (0, 2) → roll(3).
    let (n, after, mut bs) = su_minions(0, 2, 2);
    let r = bs.roll(3) as usize;
    assert_eq!((n, after), (r, bs));
}

/// Superunique 0 of class 30 with `hc_idx`, no minions: (fake, boss).
fn su_hc(hc_idx: u32, game_seed: Seed, t: &mut PopTables) -> (Fake, UnitId) {
    t.superuniques[0] = SuperPop {
        class: 30,
        hc_idx,
        min_grp: 0,
        max_grp: 0,
        auto_pos: 0,
        stacks: 1,
    };
    let mut st = state_with(t, 2);
    let mut f = Fake::new();
    f.game_seed = game_seed;
    let b = preset_spawn(&mut ctx(t, &mut st, &mut f), R0, 600, 10, 10, 1).unwrap();
    (f, b)
}

#[test]
fn superunique_hcidx_spawns() {
    // R6, hcIdx 10: roll(5) + 2 skeleton5 (class 4), then one each of
    // 276, 382, 385, 389. The roll's seed is not stated: only the range.
    let mut t = tables();
    for lo in 1..=20 {
        let (f, _) = su_hc(10, Seed::init_low(lo), &mut t);
        let n = |c: i32| f.units.iter().filter(|u| u.class == c).count();
        assert!((2..=6).contains(&n(4)), "seed {lo}: {}", n(4));
        assert_eq!([276, 382, 385, 389].map(n), [1; 4]);
    }
    // hcIdx 60: owner data (own GUID, 1, 1, 0),
    // then class-for-level(453) (here 454 by the mon list),
    // mode 1, r 10, 20 spawns, 0x40. The creation chain's step 5
    // call writes the same owner data first; the hcIdx call is the second.
    t.levels[2].mon = vec![454];
    t.monstats[454].base_id = 453;
    let (f, b) = su_hc(60, Seed::init(), &mut t);
    let boss_owner = format!("owner {} Guid({b:?}) 1 1 0", b.0);
    assert_eq!(f.log.iter().filter(|l| **l == boss_owner).count(), 2);
    assert!(!f.log.contains(&format!("owner {} Guid({b:?}) 1 0 0", b.0)));
    let owner = f.log.iter().rposition(|l| *l == boss_owner);
    let members: Vec<_> = (0..f.units.len())
        .map(|i| UnitId(i as u32))
        .filter(|&u| f.unit(u).class == 454)
        .collect();
    assert_eq!(members.len(), 20);
    let first = format!("owner {} Guid({b:?}) 1 0 0", members[0].0);
    let group = f.log.iter().position(|l| *l == first);
    assert!(owner.is_some() && owner < group);
    for m in &members {
        assert!(f.log.contains(&format!("minion {} {}", b.0, m.0)));
    }
    // hcIdx 62: 381, mode 1, r 20, 10 spawns, 0x40.
    let (f, _) = su_hc(62, Seed::init(), &mut t);
    assert_eq!(f.units.iter().filter(|u| u.class == 381).count(), 10);
}

/// The preset class of special id `id` with the fake tables (M 600, S 10).
fn sp(id: i32) -> i32 {
    600 + 10 + id
}

#[test]
fn special_once_rows() {
    // TSV rows 4 and 8: r −1 at the preset point, flags 0 / 8, no modifier
    // (only id 3 is a champion).
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    let u = preset_spawn(&mut cx, R0, sp(4), 50, 50, 1).unwrap();
    let q = preset_spawn(&mut cx, R0, sp(8), 60, 60, 1).unwrap();
    assert_eq!(f.allocs(), [266, 284]);
    assert_eq!((f.unit(u).x, f.unit(u).y), (50, 50));
    assert_eq!((f.unit(q).x, f.unit(q).y), (60, 60));
    assert_eq!(f.unit(u).type_flags & 4, 0);
    assert_eq!(f.calls("mod "), 0);
    // Flags 8 → monster flag 2.
    assert_eq!((f.unit(u).monster_flags, f.unit(q).monster_flags), (0, 2));
}

#[test]
fn special_retry_rows() {
    // r −1, then r = 4 only on failure; fallen1 → fallen2
    // (20) in level 6.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    let u = preset_spawn(&mut cx, R0, sp(17), 70, 70, 1).unwrap();
    assert_eq!(f.allocs(), [19]);
    assert_eq!((f.unit(u).x, f.unit(u).y), (70, 70));
    let mut f = Fake::new();
    f.blocked.insert((70, 70));
    let mut cx = ctx(&t, &mut st, &mut f);
    let u = preset_spawn(&mut cx, R0, sp(17), 70, 70, 1).unwrap();
    assert_eq!(f.allocs(), [19]);
    assert_ne!((f.unit(u).x, f.unit(u).y), (70, 70));
    let mut f = Fake::new();
    f.level = 6;
    let mut cx = ctx(&t, &mut st, &mut f);
    preset_spawn(&mut cx, R0, sp(17), 70, 70, 1).unwrap();
    assert_eq!(f.allocs(), [20]);
}

#[test]
fn special_chain_stops_at_invalid() {
    // Level 92 → n = 2, but 262's NextInClass is invalid, so the
    // walk stops early at 262.
    let mut t = tables();
    t.monstats[261].next_in_class = 262;
    let mut st = state_with(&t, 92);
    let mut f = Fake::new();
    f.level = 92;
    let u = preset_spawn(&mut ctx(&t, &mut st, &mut f), R0, sp(11), 60, 60, 1).unwrap();
    assert_eq!(f.unit(u).class, 262);
}

#[test]
fn special_pack_group_limits() {
    // MinGrp ≥ 1 and MaxGrp ≥ MinGrp; leader at the preset point
    // (r −1); roll(MaxGrp − MinGrp + 1) + MinGrp − 1 members.
    let run = |min: u8, max: u8| {
        let mut t = tables();
        t.monstats[453].min_grp = min;
        t.monstats[453].max_grp = max;
        let mut st = state_with(&t, 2);
        let mut f = Fake::new();
        let u = preset_spawn(&mut ctx(&t, &mut st, &mut f), R0, sp(26), 90, 90, 1);
        (u.map(|u| (f.unit(u).x, f.unit(u).y)), f.units.len())
    };
    // 1/1: the leader only.
    assert_eq!(run(1, 1), (Some((90, 90)), 1));
    assert_eq!(run(0, 2), (None, 0));
    assert_eq!(run(3, 2), (None, 0));
}

// ---- regions ----------------------------------------------------------

#[test]
fn ranged_redraws_only_for_the_first_pick() {
    // Only i = 0 re-draws for a `rangedtype` class (7 here).
    let mut t = region_tables();
    t.levels[1].ranged_spawn = 1;
    t.monstats[7].ranged_type = true;
    // A game seed whose region seed's first roll(3) is not 7's index.
    let lo = (1..)
        .find(|&lo| {
            let mut g = Seed::init_low(lo);
            Seed::init_low(g.step()).roll(3) != 2
        })
        .unwrap();
    let (regs, _) = Regions::create(&t, GameInfo::default(), &mut Seed::init_low(lo));
    let r = regs.get(1).unwrap();
    assert_eq!(r.entry_count, 2);
    assert_eq!(r.entries[0].class, 7);
    assert!([5, 6].contains(&r.entries[1].class));
}

/// A monstats2 row with choice counts `c` (other slots 0) and total `tot`.
fn m2_with(c: &[u8], tot: u8) -> Mon2Pop {
    let mut m = Mon2Pop {
        composit_total: tot,
        ..Mon2Pop::default()
    };
    m.components[..c.len()].copy_from_slice(c);
    m
}

#[test]
fn variants_capped_at_three() {
    // V = 1, k = 3 → k = 3 − v = 2 new variants.
    let m2 = m2_with(&[2], 5);
    let mut e = RegionEntry {
        variant_count: 1,
        ..RegionEntry::default()
    };
    variants(&mut Seed::init_low(3), &mut e, 3, Some(&m2));
    assert_eq!(e.variant_count, 3);
}

#[test]
fn variants_second_slot_from_shortened_list() {
    // R3– with c = [2, 2]: variant 0 = [0, 0]; r = roll(2) = 1 →
    // a = 1, L = [0, 1], b = L[roll(1)] = 0; new slot 1 = 1, slot 0 = 1.
    let s0 = seed_for(2, &[0, 0, 1, 1, 1, 1]);
    let m2 = m2_with(&[2, 2], 5);
    let mut e = RegionEntry::default();
    let mut s = s0;
    variants(&mut s, &mut e, 2, Some(&m2));
    assert_eq!(e.variant_count, 2);
    assert_eq!(e.variants[0][..2], [0, 0]);
    assert_eq!(e.variants[1][..2], [1, 1]);
    assert_eq!(s, steps(s0, 6));
}

#[test]
fn variants_duplicate_kept_after_three_tries() {
    // R5, edge case 10: one slot with 2 choices; variant 2 can only
    // repeat an old one, so after 3 duplicate draws (tries 3 → 0) it is
    // kept. Draws: [0], then 1, then 0, 0, 0.
    let s0 = seed_for(2, &[0, 1, 0, 0, 0]);
    let m2 = m2_with(&[2], 5);
    let mut e = RegionEntry::default();
    let mut s = s0;
    variants(&mut s, &mut e, 3, Some(&m2));
    assert_eq!(e.variant_count, 3);
    assert_eq!([e.variants[1][0], e.variants[2][0]], [1, 0]);
    assert_eq!(s, steps(s0, 5));
}

#[test]
fn on_demand_entry_needs_more_than_two_pieces() {
    // TotalPieces 2 → the free entry is returned unfilled.
    let mut t = tables();
    t.monstats2[0].total_pieces = 2;
    let mut st = state_with(&t, 2);
    let i = st.regions.entry_for(&t, 2, 5, &mut Seed::init()).unwrap();
    let r = st.regions.get(2).unwrap();
    assert_eq!((i, r.entry_count, r.entries[0].class), (0, 0, 0));
}

/// Regions with one region (level 2) set by `f`.
fn regions_with(f: impl FnOnce(&mut Region)) -> Regions {
    let mut r = Region::default();
    f(&mut r);
    let mut slots = vec![None; 9];
    slots[2] = Some(r.clone());
    slots[8] = Some(r);
    Regions { slots }
}

#[test]
fn alignment_change_counts() {
    // Old 0 → new 1/2: −1; new 0 with old ≠ 4: +1; else nothing.
    let spawned = |old, new| {
        let mut g = regions_with(|r| r.evil_spawned = 10);
        g.alignment_changed(2, false, false, old, new);
        g.get(2).unwrap().evil_spawned
    };
    assert_eq!(spawned(0, 2), 9);
    assert_eq!(spawned(1, 2), 10);
    assert_eq!(spawned(1, 0), 11);
    assert_eq!(spawned(4, 0), 10);
}

#[test]
fn kill_counts_only_alignment_zero() {
    // Alignment 0 moves killed by ±1; others nothing.
    let mut g = regions_with(|_| {});
    g.count_kill(2, false, 0, true);
    g.count_kill(2, false, 1, true);
    assert_eq!(g.get(2).unwrap().evil_killed, 1);
    g.count_kill(2, false, 0, false);
    assert_eq!(g.get(2).unwrap().evil_killed, 0);
}

#[test]
fn inactive_unit_keep_blocks_the_decrement() {
    // With a unit, only keep = 0 and alignment 0 decrement B.
    let mut g = regions_with(|r| r.evil_spawned = 5);
    assert_eq!(
        g.inactive_unit(2, 2, 0, false, true, Some((0, false))),
        None
    );
    assert_eq!(g.get(2).unwrap().evil_spawned, 5);
    g.inactive_unit(2, 2, 0, false, false, Some((1, false)));
    assert_eq!(g.get(2).unwrap().evil_spawned, 5);
    g.inactive_unit(2, 2, 0, false, false, Some((0, false)));
    assert_eq!(g.get(2).unwrap().evil_spawned, 4);
}

#[test]
fn den_of_evil_reading() {
    // Remaining = spawned − killed; complete when visited ≥ count
    // and killed = spawned.
    let g = regions_with(|r| {
        r.evil_spawned = 5;
        r.evil_killed = 2;
        r.rooms_visited = 10;
        r.room_count = 10;
    });
    assert_eq!(g.den_of_evil(), Some((3, false)));
    let g = regions_with(|r| {
        r.evil_spawned = 5;
        r.evil_killed = 5;
        r.rooms_visited = 9;
        r.room_count = 10;
    });
    assert_eq!(g.den_of_evil(), Some((0, false)));
    let g = regions_with(|r| {
        r.evil_spawned = 5;
        r.evil_killed = 5;
        r.rooms_visited = 10;
        r.room_count = 10;
    });
    assert_eq!(g.den_of_evil(), Some((0, true)));
}

// ---- room population ---------------------------------------------

#[test]
fn rarity_walk_ends_one_past_monster_count() {
    // All rarities 0 → w = 1 never reaches 0; the walk ends on entry
    // [monster count] = 1, here an entry past the list (class 7).
    let t = tables();
    let mut r = region_with(&[(5, 0)]);
    r.entries[1].class = 7;
    r.entry_count = 2;
    let mut s = Seed::init();
    assert_eq!(pick_region(&t, &r, &mut s, 20).class, 7);
}

#[test]
fn boss_or_pack_step3_threshold() {
    // > 35 → 2, else 1.
    let r = Region::default();
    assert_eq!(boss_or_pack(&r, &mut seed_for(100, &[35])), 1);
    assert_eq!(boss_or_pack(&r, &mut seed_for(100, &[36])), 2);
}

#[test]
fn room_count_zero_is_not_reset() {
    // R4–: only a count < 0 is set; 0 → no population.
    let t = tables();
    let mut st = state_with(&t, 2);
    st.regions.get_mut(2).unwrap().room_count = 0;
    let mut f = Fake::new();
    f.coords = vec![rect(1, 1, 2, 5)];
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(st.regions.get(2).unwrap().room_count, 0);
    assert_eq!(f.game_seed, Seed::init());
}

#[test]
fn chaos_state_blocks_only_level_108() {
    // The quest state stops level 108 only; level 2 runs its 6
    // tries.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.chaos = true;
    f.coords = vec![rect(1, 1, 2, 5)];
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.game_seed, steps(Seed::init(), 6));
}

#[test]
fn rect_skipped_only_when_left_and_right_zero() {
    // R1–: left 0, right 2 → 6 × 3 = 18 tries; left −3, right 0 →
    // 6 × 5 = 30 tries. MonDen 1 so no try hits.
    let t = tables();
    let mut st = state_with(&t, 2);
    st.regions.get_mut(2).unwrap().mon_den = 1;
    let mut f = Fake::new();
    f.coords = vec![rect(0, 1, 2, 5), rect(-3, 1, 0, 5)];
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.game_seed, steps(Seed::init(), 48));
}

#[test]
fn density_draw_equal_to_monden_hits() {
    // Lo' mod 100000 ≤ MonDen continues to the pick. Seed {429}
    // draws 443; MonDen 443 hits, the pick fails (no list): 1 step.
    let t = tables();
    let mut st = state_with(&t, 2);
    st.regions.get_mut(2).unwrap().mon_den = 443;
    let mut f = Fake::new();
    f.game_seed = Seed::init_low(429);
    f.coords = vec![rect(1, 1, 2, 5)];
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.game_seed, steps(Seed::init_low(429), 1));
}

#[test]
fn ambient_rogue_at_the_point() {
    // R5–: roll(1), then the spawn point (box 1000: roll(999) + 1 each),
    // then with r −1 there.
    let mut t = tables();
    t.levels[2].mon_wndr = 1;
    let mut st = state_with(&t, 2);
    let s0 = ambient_seed(2);
    let mut f = Fake::new();
    f.room_seeds.insert(R0, s0);
    ambient(&mut ctx(&t, &mut st, &mut f), R0);
    let mut s = steps(s0, 3);
    let p = (s.roll(999) as i32 + 1, s.roll(999) as i32 + 1);
    assert_eq!(f.allocs(), [270]);
    assert_eq!((f.units[0].x, f.units[0].y), p);
}

// ---- boss spawn, packs -----------------------------------------

#[test]
fn boss_point_searched_only_when_both_zero() {
    // R1–: (12, 0) is not searched; with cl, r −1 at a point with
    // y = 0 fails.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let cl = Some(rect(2, 2, 4, 4));
    assert!(boss_spawn(&mut ctx(&t, &mut st, &mut f), R0, cl, 12, 0, None, 5, false).is_none());
    assert!(f.units.is_empty());
}

#[test]
fn boss_with_cl_at_the_point() {
    // R −1 inside cl at (x, y).
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let cl = Some(rect(2, 2, 4, 4));
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        cl,
        12,
        13,
        None,
        5,
        false,
    )
    .unwrap();
    assert_eq!((f.unit(b).x, f.unit(b).y), (12, 13));
}

#[test]
fn boss_restore_paths() {
    // With a GUID, r −1 at (x, y) first.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        12,
        13,
        Some(9),
        5,
        false,
    )
    .unwrap();
    assert_eq!((f.unit(b).x, f.unit(b).y), (12, 13));
    // (5000, 5000) is outside the room: r −1 (3 steps) and r 5 (5 rings,
    // 20 steps) fail; then the spawn point P, r −1 at P.
    let mut f = Fake::new();
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        5000,
        5000,
        Some(9),
        5,
        false,
    )
    .unwrap();
    let mut s = steps(Seed::init(), 23);
    let p = (s.roll(999) as i32 + 1, s.roll(999) as i32 + 1);
    assert_eq!((f.unit(b).x, f.unit(b).y), p);
    // A 1 × 1 box at (10, 10): only tries (11, 11), outside; last the
    // nearest free point (10, 10) with r −1.
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        RoomBox {
            x: 10,
            y: 10,
            width: 1,
            height: 1,
        },
    );
    f.nearest = Some((10, 10));
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        5000,
        5000,
        Some(9),
        5,
        false,
    )
    .unwrap();
    assert_eq!((f.unit(b).x, f.unit(b).y), (10, 10));
}

#[test]
fn boss_spawn_nearest_free_point_uses_its_room() {
    // The last fallback places in the room holding the nearest free point:
    // a point outside the asked room's box but
    // inside its own room's box is accepted there.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let r1 = RoomId(1);
    f.boxes.insert(
        R0,
        RoomBox {
            x: 10,
            y: 10,
            width: 1,
            height: 1,
        },
    );
    f.boxes.insert(
        r1,
        RoomBox {
            x: 290,
            y: 290,
            width: 20,
            height: 20,
        },
    );
    f.nearest = Some((300, 300));
    f.nearest_room = Some(r1);
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        5000,
        5000,
        Some(9),
        5,
        false,
    )
    .unwrap();
    let u = f.unit(b);
    assert_eq!((u.x, u.y, u.room), (300, 300, r1));
}

#[test]
fn boss_spawn_nearest_free_point_searches_from_the_section8_point() {
    // The spawn-point search finds P but the creation at P fails: the nearest free
    // point then searches from P, not from the input point.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut s = steps(Seed::init(), 23);
    let p = (s.roll(999) as i32 + 1, s.roll(999) as i32 + 1);
    f.alloc_fail_at.insert(p);
    f.nearest = Some((10, 10));
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        5000,
        5000,
        Some(9),
        5,
        false,
    )
    .unwrap();
    assert_eq!((f.unit(b).x, f.unit(b).y), (10, 10));
    assert_eq!(*f.nearest_asked.borrow(), [p]);
    // Fails (1 × 1 box): the search starts from the input point.
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        RoomBox {
            x: 10,
            y: 10,
            width: 1,
            height: 1,
        },
    );
    f.nearest = Some((10, 10));
    boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        5000,
        5000,
        Some(9),
        5,
        false,
    )
    .unwrap();
    assert_eq!(*f.nearest_asked.borrow(), [(5000, 5000)]);
}

#[test]
fn pack_leader_at_the_point() {
    // R4–: leader with r −1 at the spawn point (cl tiles (2,2)-(4,4):
    // roll(9) + 11 each); MinGrp = MaxGrp = 1 → no members.
    let mut t = tables();
    t.monstats[5].min_grp = 1;
    t.monstats[5].max_grp = 1;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let l = pack(&mut ctx(&t, &mut st, &mut f), R0, rect(2, 2, 4, 4), 5).unwrap();
    let mut s = Seed::init();
    let p = (s.roll(9) as i32 + 11, s.roll(9) as i32 + 11);
    assert_eq!((f.unit(l).x, f.unit(l).y), p);
    assert_eq!(f.allocs(), [5]);
}

#[test]
fn pack_object_needs_evilhut_and_objcol() {
    // Object 562 only for class 528 with objCol.
    let mut t = tables();
    t.monstats2.push(Mon2Pop {
        obj_col: true,
        ..Mon2Pop::default()
    });
    for c in [5, 528] {
        t.monstats[c].min_grp = 1;
        t.monstats[c].max_grp = 1;
    }
    t.monstats[5].mon_stats_ex = 1;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    pack(&mut ctx(&t, &mut st, &mut f), R0, rect(2, 2, 4, 4), 5).unwrap();
    pack(&mut ctx(&t, &mut st, &mut f), R0, rect(2, 2, 4, 4), 528).unwrap();
    assert_eq!(f.calls("object"), 0);
}

#[test]
fn unlisted_special_preset_ids_create_and_draw_nothing() {
    // An id with no TSV row and no rule makes nothing and draws nothing.
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    use crate::monsters::population::seams::PopWorld;
    let (room_before, game_before) = (*f.room_seed(R0), *f.game_seed());
    let mut cx = ctx(&t, &mut st, &mut f);
    for id in [0, 1, 25, 27, 28, 33, 34, 99, 1000] {
        assert!(
            preset_spawn(&mut cx, R0, sp(id), 90, 90, 1).is_none(),
            "{id}"
        );
    }
    assert!(f.units.is_empty());
    assert!(f.log.is_empty(), "{:?}", f.log);
    assert_eq!(
        (*f.room_seed(R0), *f.game_seed()),
        (room_before, game_before)
    );
}
