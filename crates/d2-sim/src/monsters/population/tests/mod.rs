//! Population tests: every synthetic test vector of population,
//! the edge cases, and the
//! `preset-monsters.tsv` check with a perturbation test.

pub(super) mod fake;

use fake::{ctx, seed_for, state_with, tables, Fake, R0};

use super::placement::{
    flags, place, place_at, ring_search, spawn_mask, spawn_point, water_only, Placed, SpawnReq,
};
use super::preset::{
    check_special_table, class_for_level, place_presets, preset_spawn, spawn_mode_xy,
    PRESET_MONSTERS_TSV, SPECIAL_PRESETS,
};
use super::region::{variants, RegionEntry, Regions};
use super::room::{ambient, boss_or_pack, pick, pick_region, populate_room, tries};
use super::spawn::{
    boss_spawn, champion_minions, members, pack, party, random_boss, restore_plain, type_flag,
};
use super::{room_step, CoordRect, GameInfo, Mon2Pop, PopTables, PresetUnit, Region, TileRec};
use crate::rng::Seed;

/// A coordinate rectangle in tiles, index 1.
fn rect(l: i32, t: i32, r: i32, b: i32) -> CoordRect {
    CoordRect {
        rect: [l, t, r, b],
        node_flag: 0,
        index: 1,
    }
}

fn steps(mut s: Seed, n: usize) -> Seed {
    for _ in 0..n {
        s.step();
    }
    s
}

/// A region with a list of (class, rarity).
fn region_with(list: &[(i16, u8)]) -> Region {
    let mut r = Region {
        level_id: 2,
        room_count: 10,
        mon_den: 520,
        ..Default::default()
    };
    for (i, &(c, w)) in list.iter().enumerate() {
        r.entries[i].class = c;
        r.entries[i].rarity = w;
        r.total_rarity = r.total_rarity.wrapping_add(w);
    }
    r.mon_count = list.len() as u8;
    r.entry_count = list.len() as u8;
    r
}

// ---- density, tries, guard ----------------------------------------

#[test]
fn density_vectors_and_failed_pick() {
    let mut s = Seed::init_low(1);
    let lo: Vec<u32> = (0..6).map(|_| s.step()).collect();
    assert_eq!(
        lo,
        [1791398751, 791599131, 671516612, 3064641593, 3217527747, 716489901]
    );
    assert_eq!(
        lo.iter().map(|v| v % 100_000).collect::<Vec<_>>(),
        [98751, 99131, 16612, 41593, 27747, 89901]
    );
    let mut s = Seed::init_low(12345);
    let m: Vec<u32> = (0..6).map(|_| s.step() % 100_000).collect();
    assert_eq!(m, [52887, 85264, 82871, 88125, 94168, 24880]);
    // MonDen 520, hit chance 521 / 100000: ≤ 520 passes.
    let mut s = Seed::init_low(429);
    assert_eq!(s.step(), 4005600443);
    assert_eq!(4005600443u32 % 100_000, 443);

    let t = tables();
    // 6 tries (tiles (1,1)-(2,5): subtiles 20 × 5 → 6 · 1), no spawn.
    for lo in [1, 12345] {
        let mut st = state_with(&t, 2);
        let mut f = Fake::new();
        f.game_seed = Seed::init_low(lo);
        f.coords = vec![rect(1, 1, 2, 5)];
        populate_room(&mut ctx(&t, &mut st, &mut f), R0);
        assert_eq!(f.game_seed, steps(Seed::init_low(lo), 6));
        assert!(f.units.is_empty());
        assert_eq!(st.regions.get(2).unwrap().rooms_with_spawns, 0);
    }
    // Seed 429 hits on the first try; the region has no list, so the pick
    // fails and the whole room ends: no more density draws.
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.game_seed = Seed::init_low(429);
    f.coords = vec![rect(1, 1, 2, 5), rect(1, 1, 2, 5)];
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.game_seed, steps(Seed::init_low(429), 1));
    assert_eq!(f.room_seeds[&R0], Seed::init());
}

#[test]
fn tries_vectors_and_skipped_rects() {
    assert_eq!(tries(rect(0, 0, 8, 8).subtiles()), 169);
    // (20/3)·(40/3) = 6·13 = 78, not D2MOO's 86.
    assert_eq!(rect(2, 3, 10, 7).subtiles(), [10, 15, 50, 35]);
    assert_eq!(tries(rect(2, 3, 10, 7).subtiles()), 78);
    let t = tables();
    let skipped = [
        rect(0, 0, 0, 8),
        CoordRect {
            index: 0,
            ..rect(0, 0, 8, 8)
        },
        CoordRect {
            node_flag: 1,
            ..rect(0, 0, 8, 8)
        },
    ];
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.coords = skipped.to_vec();
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.game_seed, Seed::init());
}

#[test]
fn guard_rules() {
    let t = tables();
    let run = |st: &mut super::PopState, f: &mut Fake| {
        f.coords = vec![rect(1, 1, 2, 5)];
        populate_room(&mut ctx(&t, st, f), R0);
    };
    // No region for the room's level: nothing.
    let mut st = state_with(&t, 3);
    let mut f = Fake::new();
    run(&mut st, &mut f);
    assert_eq!(st.regions.get(3).unwrap().rooms_visited, 0);
    // Town, MonDen 0: visited +1, no draw.
    let mut st = state_with(&t, 2);
    st.regions.get_mut(2).unwrap().mon_den = 0;
    let mut f = Fake::new();
    run(&mut st, &mut f);
    assert_eq!(st.regions.get(2).unwrap().rooms_visited, 1);
    assert_eq!(f.game_seed, Seed::init());
    // Population-ineligible rooms affect neither coverage nor the room count.
    let mut st = state_with(&t, 2);
    st.regions.get_mut(2).unwrap().room_count = -1;
    let mut f = Fake::new();
    f.populated_level = Some(0);
    run(&mut st, &mut f);
    assert_eq!(st.regions.get(2).unwrap().room_count, -1);
    assert_eq!(st.regions.get(2).unwrap().rooms_visited, 0);
    // Room count set on first use; 0 → no population.
    let mut f = Fake::new();
    f.room_count = 0;
    f.populated_level = None;
    run(&mut st, &mut f);
    assert_eq!(st.regions.get(2).unwrap().room_count, 0);
    assert_eq!(f.game_seed, Seed::init());
    // Chaos Sanctum with the Diablo quest state.
    let mut st = state_with(&t, 108);
    let mut f = Fake::new();
    f.level = 108;
    f.chaos = true;
    run(&mut st, &mut f);
    assert_eq!(f.game_seed, Seed::init());
    // MonDen 12000 is stored as 10000.
    let mut st = state_with(&t, 2);
    st.regions.get_mut(2).unwrap().mon_den = 12000;
    let mut f = Fake::new();
    run(&mut st, &mut f);
    assert_eq!(st.regions.get(2).unwrap().mon_den, 10000);
    assert_eq!(f.game_seed, steps(Seed::init(), 6));
}

// ---- boss or pack -----------------------------------------------

#[test]
fn boss_or_pack_vectors() {
    let mut r = Region {
        rooms_visited: 2,
        room_count: 10,
        mon_umin: 1,
        mon_umax: 1,
        ..Default::default()
    };
    // Draw 19 < 100·2/10 = 20 → boss, 1 draw.
    let s0 = seed_for(100, &[19]);
    let mut s = s0;
    assert_eq!(boss_or_pack(&r, &mut s), 0);
    assert_eq!(s, steps(s0, 1));
    // Draws 20 then 5: step 1 fails, step 2 (5 ≤ 5) → boss, 2 draws.
    let s0 = seed_for(100, &[20, 5]);
    let mut s = s0;
    assert_eq!(boss_or_pack(&r, &mut s), 0);
    assert_eq!(s, steps(s0, 2));
    // U = MonUMin = MonUMax = 1: only step 3; 99 > 35 → pack (2).
    r.bosses = 1;
    let s0 = seed_for(100, &[99]);
    let mut s = s0;
    assert_eq!(boss_or_pack(&r, &mut s), 2);
    assert_eq!(s, steps(s0, 1));
    // 30 ≤ 35 → 1 (also a pack): the third draw always happens.
    let s0 = seed_for(100, &[30]);
    let mut s = s0;
    assert_eq!(boss_or_pack(&r, &mut s), 1);
    // Bosses compared as a byte: 256 reads as 0 (< MonUMin).
    r.bosses = 256;
    let mut s = seed_for(100, &[19]);
    assert_eq!(boss_or_pack(&r, &mut s), 0);
    // The helper vector: room seed {5, 666} → 99, 73, 5.
    let mut s = Seed::init_low(5);
    let d: Vec<u32> = (0..3).map(|_| s.step() % 100).collect();
    assert_eq!(d, [99, 73, 5]);
}

// ---- pick ---------------------------------------------------------

#[test]
fn pick_vectors() {
    let mut t = tables();
    // zombie1 5, fallen1 19, quillrat1 63, rarity 2 each: roll(6) = 2 →
    // w = 3: 3 − 2 = 1, 1 − 2 = −1 → fallen1.
    let r = region_with(&[(5, 2), (19, 2), (63, 2)]);
    let mut s = seed_for(6, &[2]);
    assert_eq!(pick_region(&t, &r, &mut s, 20).class, 19);
    // crownest1 (206) with placespawn → spawn (foulcrow1 210): draw 21 > 20
    // swaps, 20 does not.
    t.monstats[206].place_spawn = true;
    t.monstats[206].spawn = 210;
    let r = region_with(&[(206, 1)]);
    // roll(1) steps once, then the placespawn step.
    for (d, want) in [(21, 210), (20, 206)] {
        let mut lo = 1;
        let s0 = loop {
            let mut s = Seed::init_low(lo);
            s.step();
            if s.step() % 100 == d {
                break Seed::init_low(lo);
            }
            lo += 1;
        };
        let mut s = s0;
        let p = pick_region(&t, &r, &mut s, 20);
        assert_eq!((p.class, p.record), (want, true));
        assert_eq!(s, steps(s0, 2));
    }
    // No placespawn flag: no extra step.
    let r = region_with(&[(5, 1)]);
    let mut s = Seed::init();
    pick_region(&t, &r, &mut s, 20);
    assert_eq!(s, steps(Seed::init(), 1));
    // Empty region: class 0, no record, no draw.
    let mut s = Seed::init();
    let p = pick_region(&t, &region_with(&[]), &mut s, 20);
    assert_eq!((p.class, p.record, s), (0, false, Seed::init()));
    // Unique list in Normal: umon[roll(count)]; empty → no record.
    t.levels[2].umon = vec![30, 31, 32];
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.room_seeds.insert(R0, seed_for(3, &[1]));
    let p = pick(&mut ctx(&t, &mut st, &mut f), 2, R0, 0, true);
    assert_eq!(p.class, 31);
    t.levels[2].umon.clear();
    let mut f = Fake::new();
    let p = pick(&mut ctx(&t, &mut st, &mut f), 2, R0, 0, true);
    assert_eq!((p.class, p.record), (0, false));
    // Nightmare: the boss pick uses the region list.
    st.regions.slots[2] = Some(region_with(&[(44, 1)]));
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    cx.info.difficulty = 1;
    assert_eq!(pick(&mut cx, 2, R0, 0, true).class, 44);
}

#[test]
fn rarity_walk_overrun() {
    let t = tables();
    // Every rarity 0: w = 1 never reaches 0; the walk ends on entry
    // [monster count], a zeroed slot (class 0), with no draw.
    let r = region_with(&[(5, 0), (6, 0)]);
    let mut s = Seed::init();
    assert_eq!(pick_region(&t, &r, &mut s, 20).class, 0);
    assert_eq!(s, Seed::init());
    // A full list reads past the entries into MonDen.
    let mut r = region_with(&[(5, 0); 13]);
    r.mon_den = 520;
    assert_eq!(pick_region(&t, &r, &mut s, 20).class, 520);
    // A rarity-0 class in a mixed list is never picked.
    let r = region_with(&[(5, 0), (6, 1)]);
    let mut s = Seed::init();
    assert_eq!(pick_region(&t, &r, &mut s, 20).class, 6);
}

// ---- ring search --------------------------------------------------

#[test]
fn ring_vectors() {
    let walk = |seed: Seed, r: i32| {
        let mut s = seed;
        let mut cells = Vec::new();
        ring_search(&mut s, 0, 0, r, |x, y| {
            cells.push((x, y));
            false
        });
        (cells, s)
    };
    // d = 3, seed {1}: parity 1, roll(3) = 0, sx 0, sy 1 → (3, 0), (0, +1):
    // 24 cells clockwise, each once.
    let (cells, _) = walk(Seed::init_low(1), 1);
    assert_eq!(cells.len(), 24);
    assert_eq!(&cells[..4], &[(3, 1), (3, 2), (3, 3), (2, 3)]);
    assert_eq!(&cells[22..], &[(3, -1), (3, 0)]);
    let mut uniq = cells.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert_eq!(uniq.len(), 24);
    // d = 3, seed {42}: parity 0, roll(3) = 2, sx 0, sy 0 → (2, 3),
    // (+1, 0): first (3, 3), then back (2, 3), (1, 3) …, last (3, 2).
    let (cells, _) = walk(Seed::init_low(42), 1);
    assert_eq!(&cells[..3], &[(3, 3), (2, 3), (1, 3)]);
    assert_eq!(*cells.last().unwrap(), (3, 2));
    assert_eq!(cells.len(), 24);
    // Edge case 7: p = 1 with sx = 1 starts on the left edge going (0, +1),
    // the wrong way: down to (L, B), then back up, testing cells twice.
    let s0 = (1..)
        .map(Seed::init_low)
        .find(|&s| {
            let mut s = s;
            let p = s.step() & 1;
            s.roll(3);
            p == 1 && s.step() & 1 == 1
        })
        .unwrap();
    let (cells, _) = walk(s0, 1);
    let mut uniq = cells.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert!(uniq.len() < 24, "some cells twice, others never");
    assert_eq!(cells[0].0, -3);
    // r = −1, seed {7}: 3 steps (no roll(0) step), one test at (X0, Y0).
    let (cells, s) = walk(Seed::init_low(7), -1);
    assert_eq!(cells, [(0, 0)]);
    assert_eq!(s, steps(Seed::init_low(7), 3));
    // r = 0: failure, 0 steps.
    let (cells, s) = walk(Seed::init_low(7), 0);
    assert!(cells.is_empty());
    assert_eq!(s, Seed::init_low(7));
    // r = 2: rings 3 and 6, 4 draws each.
    let (cells, s) = walk(Seed::init_low(7), 2);
    assert_eq!(cells.len(), 24 + 48);
    assert_eq!(s, steps(Seed::init_low(7), 8));
}

// ---- placement ------------------------------------------------------

fn req(class: i32, x: i32, y: i32, r: i32, f: u16) -> SpawnReq {
    SpawnReq {
        room: Some(R0),
        cl: None,
        class,
        mode: 1,
        guid: None,
        x,
        y,
        r,
        flags: f,
    }
}

#[test]
fn water_only_placement_is_spawn_col_1_but_the_tentacles_and_153() {
    assert!(water_only(247, 1));
    assert!(!water_only(247, 0));
    assert!(!water_only(258, 1));
    assert!(!water_only(263, 1));
    assert!(!water_only(153, 1));
    assert!(water_only(264, 1));
}

#[test]
fn placement_checks() {
    assert_eq!(
        [0, 1, 2, 3, 4].map(spawn_mask),
        [0x3C01, 0x1C0, 0x3F11, 0, 0x3C01]
    );
    let mut t = tables();
    t.monstats2[0].size_x = -2;
    t.monstats[600 - 1].mon_stats_ex = 5; // no monstats2 row
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    // Invalid monstats or monstats2 row: null, no draws.
    assert_eq!(place(&mut cx, req(600, 10, 10, -1, 0)), Placed::Failed);
    assert_eq!(place(&mut cx, req(599, 10, 10, -1, 0)), Placed::Failed);
    assert_eq!(
        place(
            &mut cx,
            SpawnReq {
                room: None,
                ..req(5, 10, 10, -1, 0)
            }
        ),
        Placed::Failed
    );
    assert_eq!(cx.host.room_seeds[&R0], Seed::init());
    // Probe: accepted, nothing created; SizeX sign-extended, mask 0x3C01.
    assert_eq!(
        place(&mut cx, req(5, 10, 10, -1, flags::PROBE)),
        Placed::Probe
    );
    assert!(cx.host.units.is_empty());
    assert_eq!(cx.host.collide_calls.borrow()[0], (10, 10, -2, 0x3C01));
    // Flag 0x80 skips the collision test.
    cx.host.blocked.insert((10, 10));
    assert_eq!(
        place(&mut cx, req(5, 10, 10, -1, flags::PROBE)),
        Placed::Failed
    );
    assert_eq!(
        place(
            &mut cx,
            req(5, 10, 10, -1, flags::PROBE | flags::NO_COLLISION)
        ),
        Placed::Probe
    );
    // Outside the room box (PtInRect: right/bottom excluded).
    assert_eq!(
        place(&mut cx, req(5, 1000, 10, -1, flags::PROBE)),
        Placed::Failed
    );
    // A point at x = 0 counts as none.
    assert_eq!(
        place(&mut cx, req(5, 0, 10, -1, flags::PROBE)),
        Placed::Failed
    );
    // With cl: its rect bounds and the index check.
    let cl = rect(2, 2, 4, 4); // subtiles 10..20
    let r = SpawnReq {
        cl: Some(cl),
        ..req(5, 12, 12, -1, flags::PROBE)
    };
    assert_eq!(place(&mut cx, r), Placed::Probe);
    assert_eq!(place(&mut cx, SpawnReq { x: 20, ..r }), Placed::Failed);
    cx.host.index_map.insert((12, 12), 2);
    assert_eq!(place(&mut cx, r), Placed::Failed);
}

#[test]
fn special_footprints() {
    let mut t = tables();
    for (base, (dx, dy, mask)) in [
        (206, (0, 3, 0x3C01)),
        (228, (0, 2, 0x3C01)),
        (334, (-2, -2, 0x1C0)),
        (528, (2, 4, 0x3C01)),
    ] {
        t.monstats[base as usize].base_id = base;
        let mut st = state_with(&t, 2);
        let mut f = Fake::new();
        f.blocked.insert((10 + dx, 10 + dy));
        let mut cx = ctx(&t, &mut st, &mut f);
        assert_eq!(
            place(&mut cx, req(i32::from(base), 10, 10, -1, flags::PROBE)),
            Placed::Failed
        );
        assert!(cx
            .host
            .collide_calls
            .borrow()
            .contains(&(10 + dx, 10 + dy, 2, mask)));
        // Its room null → fails as well.
        cx.host.blocked.clear();
        cx.host.void.insert((10 + dx, 10 + dy));
        assert_eq!(
            place(&mut cx, req(i32::from(base), 10, 10, -1, flags::PROBE)),
            Placed::Failed
        );
    }
    // vilemother1 (298) always passes.
    t.monstats[298].base_id = 298;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.void.insert((10, 13));
    let mut cx = ctx(&t, &mut st, &mut f);
    assert_eq!(
        place(&mut cx, req(298, 10, 10, -1, flags::PROBE)),
        Placed::Probe
    );
}

#[test]
fn water_placement() {
    let mut t = tables();
    t.monstats2.push(Mon2Pop {
        spawn_col: 1,
        ..Mon2Pop::default()
    });
    t.monstats[247].mon_stats_ex = 1;
    t.monstats[258].mon_stats_ex = 1;
    let tile = |x| TileRec {
        water: true,
        x,
        y: 1,
    };
    // roll(3) = 0 → s = 1: tiles 1, 2 visited, tile 0 never.
    let run = |f: &mut Fake, class| {
        let mut st = state_with(&t, 2);
        let mut cx = ctx(&t, &mut st, f);
        place(&mut cx, req(class, 50, 50, -1, flags::PROBE))
    };
    let mut f = Fake::new();
    f.room_seeds.insert(R0, seed_for(3, &[0]));
    f.tiles = vec![tile(0), tile(1), tile(2)];
    f.masked.insert((5 + 3, 8)); // tile 1 fails the 0x100 test
    assert_eq!(run(&mut f, 247), Placed::Probe);
    // Tile 2 won: its neighbours were tested with size 2, mask 0x1C09.
    assert!(f.collide_calls.borrow().contains(&(13, 5, 2, 0x1C09)));
    f.masked.insert((13, 8));
    f.room_seeds.insert(R0, seed_for(3, &[0]));
    assert_eq!(run(&mut f, 247), Placed::Failed, "tile 0 is never tested");
    // All four neighbours blocked fails the point too.
    let mut f = Fake::new();
    f.room_seeds.insert(R0, seed_for(3, &[1]));
    f.tiles = vec![tile(0), tile(1), tile(2)];
    f.blocked.extend([(8, 5), (11, 8), (8, 11), (5, 8)]);
    f.masked.insert((13, 8));
    assert_eq!(run(&mut f, 247), Placed::Failed);
    // Non-water records are skipped; no tiles → none.
    let mut f = Fake::new();
    f.tiles = vec![TileRec::default(); 3];
    assert_eq!(run(&mut f, 247), Placed::Failed);
    // Tentacles (258) use the ring search with mask 0x1C0.
    let mut f = Fake::new();
    assert_eq!(run(&mut f, 258), Placed::Probe);
    assert_eq!(f.collide_calls.borrow()[0].3, 0x1C0);
}

#[test]
fn creation_call() {
    let mut t = tables();
    t.monstats[5].align = 1;
    t.monstats[6].align = 2;
    t.monstats[7].never_count = true;
    t.monstats[8].minion1 = 9;
    t.monstats[8].party_min = 1;
    t.monstats[8].party_max = 1;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    let u = place(&mut cx, req(5, 10, 10, -1, 0)).unit().unwrap();
    let u6 = place(&mut cx, req(6, 20, 20, -1, flags::NO_EXTRAS))
        .unit()
        .unwrap();
    let u7 = place(&mut cx, req(7, 30, 30, -1, 0)).unit().unwrap();
    let u8 = place(&mut cx, req(8, 40, 40, -1, flags::NO_COUNT))
        .unit()
        .unwrap();
    let _ = place(&mut cx, req(8, 50, 50, -1, flags::NO_PARTY))
        .unit()
        .unwrap();
    // Alignment: Align 1 → 2 + unit flag 0x20000; 2 → 1; else 0.
    assert_eq!((f.unit(u).align, f.unit(u).unit_flags), (2, 0x20000));
    assert_eq!((f.unit(u6).align, f.unit(u6).unit_flags), (1, 0));
    // neverCount / flag 8 → monster flag 2, not counted.
    assert_eq!(f.unit(u7).monster_flags, 2);
    assert_eq!(f.unit(u8).monster_flags, 2);
    // Counted: 5, 6, the party minion of 8, and the last 8.
    assert_eq!(st.regions.get(2).unwrap().evil_spawned, 4);
    // 0x02 skips extras; init always; the coord record from the room.
    assert_eq!(f.calls("extras"), 5);
    assert_eq!(f.calls("init"), 6);
    assert_eq!(f.calls("coord"), 6);
    // Party only without 0x40.
    assert_eq!(f.allocs(), [5, 6, 7, 8, 9, 8]);
    // Allocation steps the game seed once per unit.
    assert_eq!(f.game_seed, steps(Seed::init(), 6));
    // Allocation fails → null.
    f.alloc_fail = true;
    let mut cx = ctx(&t, &mut st, &mut f);
    assert_eq!(place(&mut cx, req(5, 10, 10, -1, 0)), Placed::Failed);
}

// ---- spawn point ----------------------------------------------------

#[test]
fn spawn_point_rules() {
    let mut t = tables();
    t.levels[2].warp_dist = 2025;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        super::RoomBox {
            x: 0,
            y: 0,
            width: 11,
            height: 11,
        },
    );
    // A warp in the middle: every point of the box is within 45 → reject.
    f.warps = vec![(5, 5)];
    let mut cx = ctx(&t, &mut st, &mut f);
    assert_eq!(spawn_point(&mut cx, R0, None, 5, true), None);
    // 20 tries × (roll(w), roll(h)), no probe.
    assert_eq!(f.room_seeds[&R0], steps(Seed::init(), 40));
    // The kind-11 location (tiles × 5) rejects as well; x = 0 is ignored.
    let mut f2 = Fake::new();
    f2.boxes = f.boxes.clone();
    f2.spawn_loc = Some((1, 1));
    let mut cx = ctx(&t, &mut st, &mut f2);
    assert_eq!(spawn_point(&mut cx, R0, None, 5, true), None);
    f2.spawn_loc = Some((0, 1));
    f2.room_seeds.insert(R0, Seed::init());
    let mut cx = ctx(&t, &mut st, &mut f2);
    let p = spawn_point(&mut cx, R0, None, 5, true).unwrap();
    // Bounds without cl: (x + 1, y + 1), w − 1, h − 1.
    let mut s = Seed::init();
    let x = s.roll(10) as i32 + 1;
    let y = s.roll(10) as i32 + 1;
    assert_eq!(p, (x, y));
    // Probe (3 steps) after the point (2 steps).
    assert_eq!(f2.room_seeds[&R0], steps(Seed::init(), 5));
    // Edge case 12: the creation repeats the ring (3 more steps).
    let mut cx = ctx(&t, &mut st, &mut f2);
    place_at(&mut cx, R0, None, p.0, p.1, 5, 1, -1, 0)
        .unit()
        .unwrap();
    assert_eq!(f2.room_seeds[&R0], steps(Seed::init(), 8));
    // Without warp check the warp is ignored.
    let mut cx = ctx(&t, &mut st, &mut f);
    assert!(spawn_point(&mut cx, R0, None, 5, false).is_some());
    // With cl: bounds (x0 + 1, y0 + 1), w = x1 − left; index mismatch
    // rejects without a probe.
    let mut f3 = Fake::new();
    f3.default_index = 7;
    let mut cx = ctx(&t, &mut st, &mut f3);
    assert_eq!(
        spawn_point(&mut cx, R0, Some(rect(2, 2, 4, 4)), 5, false),
        None
    );
    assert_eq!(f3.room_seeds[&R0], steps(Seed::init(), 40));
    let mut f3 = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f3);
    let (x, y) = spawn_point(&mut cx, R0, Some(rect(2, 2, 4, 4)), 5, false).unwrap();
    let mut s = Seed::init();
    assert_eq!((x, y), (s.roll(9) as i32 + 11, s.roll(9) as i32 + 11));
}

// ---- packs, parties ---------------------------------------------

#[test]
fn pack_member_vectors() {
    let t = tables();
    for (lo, n) in [(1, 1), (2, 0)] {
        let mut st = state_with(&t, 2);
        let mut f = Fake::new();
        let leader = f.add_unit(5, 100, 100, Seed::init_low(lo));
        members(&mut ctx(&t, &mut st, &mut f), None, leader, 5, 1, 2);
        assert_eq!(f.units.len() - 1, n, "seed {lo}");
        // Each member near the leader (r 3).
        assert!(f.units[1..].iter().all(|u| (u.x - 100).abs() <= 9));
    }
}

#[test]
fn party_vectors() {
    let mut t = tables();
    // fallenshaman1 (58): minion1 fallen1 (19), party 2–6, SetBoss.
    t.monstats[58].minion1 = 19;
    t.monstats[58].party_min = 2;
    t.monstats[58].party_max = 6;
    t.monstats[58].set_boss = true;
    t.monstats[19].minion1 = 19;
    t.monstats[19].party_min = 2;
    t.monstats[19].party_max = 3;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let shaman = f.add_unit(58, 100, 100, Seed::init_low(3));
    party(&mut ctx(&t, &mut st, &mut f), shaman, 58, 0);
    // 2 + roll(5) = 2 + 0 = 2 fallen, without their own parties (0x40).
    assert_eq!(f.allocs(), [58, 19, 19]);
    assert_eq!(f.log[0], "owner 0 Guid(UnitId(0)) 1 1 0");
    assert!(f
        .log
        .contains(&"owner 1 DataOf(UnitId(0)) 1 0 0".to_string()));
    assert_eq!(f.calls("minion 0"), 2);
    // Without SetBoss: no owner data, no minion list.
    t.monstats[58].set_boss = false;
    let mut f = Fake::new();
    let shaman = f.add_unit(58, 100, 100, Seed::init_low(3));
    party(&mut ctx(&t, &mut st, &mut f), shaman, 58, 0);
    assert_eq!(f.calls("owner"), 0);
    assert_eq!(f.calls("minion"), 0);
    // PartyMin ≥ PartyMax: count = PartyMin without a draw; with minion2
    // valid the classes alternate m1, m2, m1.
    t.monstats[70].minion1 = 71;
    t.monstats[70].minion2 = 72;
    t.monstats[70].party_min = 3;
    t.monstats[70].party_max = 3;
    let mut f = Fake::new();
    let l = f.add_unit(70, 100, 100, Seed::init());
    party(&mut ctx(&t, &mut st, &mut f), l, 70, 0);
    assert_eq!(f.allocs(), [70, 71, 72, 71]);
    assert_eq!(f.unit(l).seed, Seed::init());
    // Invalid minion1: no party.
    t.monstats[70].minion1 = 600;
    let mut f = Fake::new();
    let l = f.add_unit(70, 100, 100, Seed::init());
    party(&mut ctx(&t, &mut st, &mut f), l, 70, 0);
    assert_eq!(f.units.len(), 1);
}

#[test]
fn tentacle_offsets() {
    let mut t = tables();
    t.monstats[261].minion1 = 262;
    t.monstats[261].party_min = 3;
    t.monstats[261].party_max = 3;
    t.monstats[261].boss_xfer = true;
    let mut st = state_with(&t, 2);
    // flags 0 → set 1; k = 4: indices 4, 3, 2 → (1,0), (−2,1), (2,−1).
    let mut f = Fake::new();
    let head = f.add_unit(261, 100, 100, seed_for(6, &[4]));
    party(&mut ctx(&t, &mut st, &mut f), head, 261, 0);
    let pos: Vec<(i32, i32)> = f.units[1..].iter().map(|u| (u.x, u.y)).collect();
    assert_eq!(pos, [(101, 100), (98, 101), (102, 99)]);
    assert_eq!(f.log[0], "owner 0 Guid(UnitId(0)) 1 1 1");
    assert_eq!(f.calls("minion 0"), 3);
    // flags 4 → set 0: indices 4, 3, 2 → (0,2), (−1,3), (1,−3).
    let mut f = Fake::new();
    let head = f.add_unit(261, 100, 100, seed_for(6, &[4]));
    party(
        &mut ctx(&t, &mut st, &mut f),
        head,
        261,
        flags::TENTACLE_SET0,
    );
    let pos: Vec<(i32, i32)> = f.units[1..].iter().map(|u| (u.x, u.y)).collect();
    assert_eq!(pos, [(100, 102), (99, 103), (101, 97)]);
}

#[test]
fn pack_rules() {
    let mut t = tables();
    // fallen1: forced 1/1 → leader only (roll(1) steps once) + party.
    t.monstats[19].min_grp = 2;
    t.monstats[19].max_grp = 3;
    t.monstats[19].minion1 = 19;
    t.monstats[19].party_min = 2;
    t.monstats[19].party_max = 3;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let cl = rect(10, 10, 20, 20);
    let leader = pack(&mut ctx(&t, &mut st, &mut f), R0, cl, 19).unwrap();
    // Party 2 + roll(2) on the leader seed, drawn before the member count
    // roll(1) (one step, 0 members).
    let mut ls = Seed::init();
    let mut ls = ls.derive();
    let n = 1 + 2 + ls.roll(2) as usize;
    assert_eq!(f.units.len(), n, "leader + party, no members");
    assert_eq!(f.unit(leader).seed, steps(ls, 1));
    assert!(f.units.iter().all(|u| u.class == 19));
    // MinGrp 0 → no pack.
    t.monstats[5].min_grp = 0;
    t.monstats[5].max_grp = 2;
    let mut f = Fake::new();
    assert!(pack(&mut ctx(&t, &mut st, &mut f), R0, cl, 5).is_none());
    // max < min → no pack.
    t.monstats[5].min_grp = 3;
    let mut f = Fake::new();
    assert!(pack(&mut ctx(&t, &mut st, &mut f), R0, cl, 5).is_none());
    assert_eq!(f.room_seeds[&R0], Seed::init());
    // evilhut (528): sparsePopulate 40 on the game seed; objCol → object 562
    // at the leader.
    t.monstats[528].sparse_populate = 40;
    t.monstats[528].min_grp = 1;
    t.monstats[528].max_grp = 1;
    t.monstats[528].mon_stats_ex = 1;
    t.monstats2.push(Mon2Pop {
        obj_col: true,
        ..Mon2Pop::default()
    });
    let mut f = Fake::new();
    f.game_seed = seed_for(100, &[41]);
    assert!(pack(&mut ctx(&t, &mut st, &mut f), R0, cl, 528).is_none());
    assert!(f.units.is_empty());
    let mut f = Fake::new();
    f.game_seed = seed_for(100, &[40]);
    let l = pack(&mut ctx(&t, &mut st, &mut f), R0, cl, 528).unwrap();
    let (x, y) = (f.unit(l).x, f.unit(l).y);
    assert!(f.log.contains(&format!("object 562 {x} {y}")));
    // No point → no pack.
    t.monstats[6].min_grp = 1;
    t.monstats[6].max_grp = 1;
    let mut f = Fake::new();
    f.default_index = 9;
    assert!(pack(&mut ctx(&t, &mut st, &mut f), R0, cl, 6).is_none());
}

// ---- bosses ---------------------------------------------------------

#[test]
fn random_boss_flow() {
    let mut t = tables();
    t.monstats[30].minion1 = 31;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let cl = rect(10, 10, 30, 30);
    let boss = random_boss(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        Some(cl),
        30,
        true,
        0,
        0,
        true,
    )
    .unwrap();
    // Boss flags 8 | 1, counted once, quest hook, owner data.
    assert_eq!(
        f.unit(boss).type_flags,
        type_flag::UNIQUE | type_flag::OTHER_BOSS
    );
    assert_eq!(st.regions.get(2).unwrap().bosses, 1);
    let i = |p: &str| f.log.iter().position(|l| l.starts_with(p)).unwrap();
    assert!(i("quest 0") < i("owner 0 Guid(UnitId(0)) 1 1 0"));
    assert!(i("owner 0") < i("bossmods 0 true"));
    // 3 + roll(4) minions of minion1 on the boss seed, each with xfer,
    // owner data, minion list, owner and type flag 0x10; then mod init.
    let minions: Vec<_> = f.units[1..].iter().collect();
    assert!((3..=6).contains(&minions.len()));
    assert!(minions
        .iter()
        .all(|m| m.class == 31 && m.type_flags == type_flag::MINION));
    assert_eq!(f.calls("xfer 0"), minions.len());
    assert_eq!(f.calls("setowner"), minions.len());
    // Owner data with (game, minion, boss GUID, 1, 0, 0) (open
    // question 4).
    for m in 1..=minions.len() {
        let owner = format!("owner {m} Guid(UnitId(0)) 1 0 0");
        assert!(f.log.contains(&owner), "{owner}");
    }
    assert!(f.log.last().unwrap().starts_with("modinit 0"));
    // The minion count comes from the boss's seed, as derived at its
    // allocation: replay it.
    let mut g = Seed::init();
    let mut bs = g.derive();
    assert_eq!(minions.len() as u32, bs.roll(4) + 3);
    // Champion: no unique minions; minion1 invalid → own class.
    let mut f = Fake::new();
    f.champion = true;
    random_boss(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        Some(cl),
        30,
        true,
        0,
        0,
        true,
    )
    .unwrap();
    assert_eq!(f.units.len(), 1);
    let mut f = Fake::new();
    random_boss(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        40,
        true,
        50,
        50,
        true,
    )
    .unwrap();
    assert!(f.units[1..].iter().all(|m| m.class == 40));
    // Without cl at a blocked point: r = −1 fails, then r = 5.
    let mut f = Fake::new();
    f.blocked.insert((50, 50));
    let b = random_boss(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        40,
        false,
        50,
        50,
        true,
    )
    .unwrap();
    assert_ne!((f.unit(b).x, f.unit(b).y), (50, 50));
    // No point → null.
    let mut f = Fake::new();
    f.default_index = 9;
    assert!(random_boss(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        Some(cl),
        30,
        true,
        0,
        0,
        true
    )
    .is_none());
}

#[test]
fn boss_spawn_with_guid() {
    let mut t = tables();
    t.monstats[30].minion1 = 31;
    t.monstats[30].party_min = 1;
    t.monstats[30].party_max = 1;
    let mut st = state_with(&t, 2);
    // Flags 0x62: the caller's GUID, no extras, no party; r = −1 fails at
    // the blocked point, then r = 5.
    let mut f = Fake::new();
    f.blocked.insert((50, 50));
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        50,
        50,
        Some(77),
        30,
        false,
    )
    .unwrap();
    assert_eq!(f.allocs(), [30]);
    assert_eq!(
        f.log[0],
        format!("alloc 30 {} {} m1 g77", f.unit(b).x, f.unit(b).y)
    );
    assert_eq!(f.calls("extras"), 0);
    // Every ring blocked: a spawn point (no warp check), then the nearest
    // free point.
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        super::RoomBox {
            x: 0,
            y: 0,
            width: 200,
            height: 200,
        },
    );
    for x in 30..=70 {
        for y in 30..=70 {
            f.blocked.insert((x, y));
        }
    }
    let b = boss_spawn(
        &mut ctx(&t, &mut st, &mut f),
        R0,
        None,
        50,
        50,
        Some(77),
        30,
        false,
    )
    .unwrap();
    let (x, y) = (f.unit(b).x, f.unit(b).y);
    assert!(!f.blocked.contains(&(x, y)));
}

#[test]
fn restore_plain_chain() {
    let t = tables();
    let mut st = state_with(&t, 2);
    // Free point: created at the stored point with the stored GUID and
    // mode, no extras, no party.
    let mut f = Fake::new();
    let u = restore_plain(&mut ctx(&t, &mut st, &mut f), R0, 50, 50, 77, 30, 1, 0x62).unwrap();
    assert_eq!((f.unit(u).x, f.unit(u).y), (50, 50));
    assert_eq!(f.log[0], "alloc 30 50 50 m1 g77");
    assert_eq!(f.calls("extras"), 0);
    // Blocked point: the r = 5 ring.
    let mut f = Fake::new();
    f.blocked.insert((50, 50));
    let u = restore_plain(&mut ctx(&t, &mut st, &mut f), R0, 50, 50, 77, 30, 1, 0x62).unwrap();
    assert!(!f.blocked.contains(&(f.unit(u).x, f.unit(u).y)));
    // Every ring blocked, no point (the room box blocked too): the
    // nearest free point, in the room it returns.
    let mut f = Fake::new();
    let r1 = crate::units::RoomId(1);
    f.boxes.insert(
        R0,
        super::RoomBox {
            x: 40,
            y: 40,
            width: 20,
            height: 20,
        },
    );
    f.boxes.insert(
        r1,
        super::RoomBox {
            x: 110,
            y: 110,
            width: 20,
            height: 20,
        },
    );
    for x in 30..=70 {
        for y in 30..=70 {
            f.blocked.insert((x, y));
        }
    }
    f.nearest = Some((120, 121));
    f.nearest_room = Some(r1);
    let u = restore_plain(&mut ctx(&t, &mut st, &mut f), R0, 50, 50, 77, 30, 1, 0x62).unwrap();
    assert_eq!((f.unit(u).x, f.unit(u).y, f.unit(u).room), (120, 121, r1));
    assert_eq!(f.allocs(), [30]);
    // Every ring blocked, a free point: created there.
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        super::RoomBox {
            x: 0,
            y: 0,
            width: 200,
            height: 200,
        },
    );
    for x in 30..=70 {
        for y in 30..=70 {
            f.blocked.insert((x, y));
        }
    }
    f.nearest = Some((120, 121));
    let u = restore_plain(&mut ctx(&t, &mut st, &mut f), R0, 50, 50, 77, 30, 1, 0x62).unwrap();
    let (x, y) = (f.unit(u).x, f.unit(u).y);
    assert!(!f.blocked.contains(&(x, y)));
    assert_ne!((x, y), (120, 121));
}

#[test]
fn champion_minion_vectors() {
    let mut t = tables();
    t.monstats[58].minion1 = 19;
    t.monstats[58].party_min = 2;
    t.monstats[58].party_max = 2;
    let mut st = state_with(&t, 2);
    // Seed {3}: lo' mod 3 = 2 → 3 minions, each with modifier 16 and (flags
    // 0) its own party of 2.
    let mut f = Fake::new();
    let boss = f.add_unit(58, 100, 100, Seed::init_low(3));
    f.units[0].type_flags = type_flag::CHAMPION;
    champion_minions(&mut ctx(&t, &mut st, &mut f), None, boss, 58);
    assert_eq!(f.units.iter().filter(|u| u.class == 58).count(), 4);
    assert_eq!(f.units.iter().filter(|u| u.class == 19).count(), 6);
    assert_eq!(f.calls("mod "), 3);
    // Not a champion: nothing, no draw.
    let mut f = Fake::new();
    let boss = f.add_unit(58, 100, 100, Seed::init_low(3));
    champion_minions(&mut ctx(&t, &mut st, &mut f), None, boss, 58);
    assert_eq!(f.units.len(), 1);
    assert_eq!(f.unit(boss).seed, Seed::init_low(3));
}

#[test]
fn population_boss_path() {
    let t = tables();
    let mut st = state_with(&t, 2);
    {
        let r = st.regions.get_mut(2).unwrap();
        *r = Region {
            room_count: 1,
            mon_umin: 1,
            ..region_with(&[(5, 1)])
        };
    }
    let mut f = Fake::new();
    f.game_seed = Seed::init_low(429); // first try hits
    f.coords = vec![rect(1, 1, 2, 5)];
    f.room_count = 1;
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    // V = 1, N = 1: 100·1/1 = 100 → boss. No umon list in Normal → class 0.
    assert_eq!(f.units[0].class, 0);
    assert_eq!(f.units[0].type_flags & type_flag::UNIQUE, type_flag::UNIQUE);
    let r = st.regions.get(2).unwrap();
    assert_eq!((r.rooms_visited, r.bosses, r.rooms_with_spawns), (1, 1, 1));
}

#[test]
fn population_pack_path() {
    let mut t = tables();
    t.monstats[5].min_grp = 1;
    t.monstats[5].max_grp = 1;
    let mut st = state_with(&t, 2);
    *st.regions.get_mut(2).unwrap() = region_with(&[(5, 1)]);
    let mut f = Fake::new();
    f.game_seed = Seed::init_low(429);
    f.coords = vec![rect(1, 1, 2, 5)];
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.allocs(), [5]);
    assert_eq!(st.regions.get(2).unwrap().rooms_with_spawns, 1);
}

// ---- regions --------------------------------------------------------

fn region_tables() -> PopTables {
    let mut t = tables();
    t.levels.truncate(4);
    t.levels[1].mon = vec![5, 6, 7];
    t.levels[1].nmon = vec![8, 9];
    t.levels[1].num_mon = 2;
    t.levels[1].mon_den = [520, 600, 700];
    t.levels[1].mon_umin = [1, 4, 7];
    t.levels[1].mon_umax = [1, 5, 9];
    t.levels[1].mon_lvl = [2, 36, 67];
    t.levels[1].mon_lvl_ex = [1, 37, 68];
    t.levels[1].act = 0;
    t.levels[1].quest = 1;
    t.levels[1].mon_wndr = 1;
    t.monstats[6].rarity = 2;
    t.monstats[7].rarity = 1;
    t
}

#[test]
fn region_creation() {
    let t = region_tables();
    let mut g = Seed::init_low(77);
    let (regs, lo) = Regions::create(&t, GameInfo::default(), &mut g);
    assert_eq!(g, steps(Seed::init_low(77), 1));
    assert_eq!(lo, g.lo);
    assert!(regs.slots[0].is_none() && regs.get(0).is_none());
    assert_eq!(regs.slots.len(), 4);
    let r = regs.get(1).unwrap();
    assert_eq!(
        (
            r.room_count,
            r.mon_den,
            r.mon_umin,
            r.mon_umax,
            r.quest,
            r.ai_field
        ),
        (-1, 520, 1, 1, 1, -1)
    );
    assert_eq!(r.monster_level, [2, 2]);
    // The list: roll(3), remove, roll(2) on the region seed {lo', 666}.
    let mut s = Seed::init_low(lo);
    let mut list = vec![5, 6, 7];
    let a = list.remove(s.roll(3) as usize);
    let b = list.remove(s.roll(2) as usize);
    assert_eq!((r.entries[0].class, r.entries[1].class), (a, b));
    assert_eq!(r.mon_count, 2);
    let rar = |c: i16| t.monstats[c as usize].rarity;
    assert_eq!(r.total_rarity, rar(a) + rar(b));
    assert_eq!(r.entries[0].rarity, rar(a));
    // Nightmare: nmon list, MonDen(N), expansion monster level.
    let mut g = Seed::init_low(77);
    let info = GameInfo {
        difficulty: 1,
        expansion: true,
    };
    let (regs, _) = Regions::create(&t, info, &mut g);
    let r = regs.get(1).unwrap();
    assert_eq!((r.mon_den, r.mon_umin, r.mon_umax), (600, 4, 5));
    assert_eq!(r.monster_level, [37, 37]);
    let mut got = [r.entries[0].class, r.entries[1].class];
    got.sort_unstable();
    assert_eq!(got, [8, 9]);
    // Level 2 draws continue on the same seed after level 1.
    assert_eq!(regs.get(2).unwrap().mon_count, 0);
}

#[test]
fn region_list_ranged_and_non_spawn() {
    let mut t = region_tables();
    // No ranged class, rangedspawn set: 20 re-draws for the first pick.
    t.levels[1].ranged_spawn = 1;
    t.levels[1].num_mon = 1;
    let mut g = Seed::init_low(5);
    let (regs, lo) = Regions::create(&t, GameInfo::default(), &mut g);
    let mut s = Seed::init_low(lo);
    let mut idx = s.roll(3);
    for _ in 0..20 {
        idx = s.roll(3);
    }
    assert_eq!(
        i32::from(regs.get(1).unwrap().entries[0].class),
        [5, 6, 7][idx as usize]
    );
    // A ranged class stops the re-draws at once.
    t.monstats[5].ranged_type = true;
    t.monstats[6].ranged_type = true;
    t.monstats[7].ranged_type = true;
    let mut g = Seed::init_low(5);
    let (regs, lo) = Regions::create(&t, GameInfo::default(), &mut g);
    let mut s = Seed::init_low(lo);
    assert_eq!(
        i32::from(regs.get(1).unwrap().entries[0].class),
        [5, 6, 7][s.roll(3) as usize]
    );
    // Non-isSpawn classes are dropped, their draw still counted.
    for c in [5, 6, 7] {
        t.monstats[c].is_spawn = false;
    }
    t.levels[1].num_mon = 3;
    let mut g = Seed::init_low(5);
    let (regs, _) = Regions::create(&t, GameInfo::default(), &mut g);
    let r = regs.get(1).unwrap();
    assert_eq!((r.mon_count, r.entry_count, r.total_rarity), (0, 0, 0));
}

#[test]
fn appearance_variants() {
    // One slot with 2 choices, T = 1: k = min(3, 2 − 0) = 2.
    let mut m2 = Mon2Pop::default();
    m2.components[3] = 2;
    m2.components[0] = 1;
    m2.composit_total = 1;
    let s0 = Seed::init_low(9);
    let mut s = s0;
    let mut e = RegionEntry::default();
    variants(&mut s, &mut e, 3, Some(&m2));
    assert_eq!(e.variant_count, 2);
    // Replay: variant 0 slot 3 = roll(2); L = [3] (m = 1), so a = b = 3;
    // the new variant re-rolls slot 3 until it differs (3 tries).
    let mut r = s0;
    let v0 = r.roll(2) as u8;
    assert_eq!(e.variants[0][3], v0);
    let mut tries = 3;
    let mut v1;
    loop {
        v1 = r.roll(2) as u8;
        if v1 != v0 {
            break;
        }
        tries -= 1;
        if tries == 0 {
            break;
        }
    }
    assert_eq!(e.variants[1][3], v1);
    assert_eq!(s, r);
    // Full (v = 2, T = 1): 1 << 1 − 2 = 0 → no draws.
    let mut s = s0;
    variants(&mut s, &mut e, 3, Some(&m2));
    assert_eq!((e.variant_count, s), (2, s0));
    // T = 33 shifts by 1 (T & 31): the same as T = 1.
    m2.composit_total = 33;
    let mut e2 = RegionEntry::default();
    let mut s = s0;
    variants(&mut s, &mut e2, 3, Some(&m2));
    assert_eq!(e2.variant_count, 2);
    // No monstats2 row: nothing.
    let mut e3 = RegionEntry::default();
    let mut s = s0;
    variants(&mut s, &mut e3, 3, None);
    assert_eq!((e3.variant_count, s), (0, s0));
    // v + k > 3 caps at 3 − v; v = 3 → nothing.
    m2.composit_total = 4;
    let mut e4 = RegionEntry {
        variant_count: 3,
        ..Default::default()
    };
    let mut s = s0;
    variants(&mut s, &mut e4, 3, Some(&m2));
    assert_eq!(s, s0);
    // Two slots: r = roll(m), a = L[r], L[r] = L[m−1], b = L[roll(m−1)].
    let mut m2b = Mon2Pop::default();
    m2b.components[1] = 3;
    m2b.components[5] = 4;
    m2b.composit_total = 4;
    let mut e5 = RegionEntry::default();
    let mut s = s0;
    variants(&mut s, &mut e5, 2, Some(&m2b));
    let mut r = s0;
    let (c1, c5) = (r.roll(3) as u8, r.roll(4) as u8);
    assert_eq!((e5.variants[0][1], e5.variants[0][5]), (c1, c5));
    let ri = r.roll(2) as usize;
    let mut l = [1usize, 5];
    let a = l[ri];
    l[ri] = l[1];
    let b = l[r.roll(1) as usize];
    let mut want = e5.variants[0];
    want[a] = r.roll(if a == 1 { 3 } else { 4 }) as u8;
    if b != a {
        want[b] = r.roll(if b == 1 { 3 } else { 4 }) as u8;
    }
    if want != e5.variants[0] {
        assert_eq!(e5.variants[1], want);
        assert_eq!(s, r);
    }
    // Edge case 10: two identical old variants make tries drop by 2 per
    // duplicate, skipping 0: the loop runs until a unique variant (draws
    // 0, 0, 0, then 1).
    m2.composit_total = 2;
    let s0 = seed_for(2, &[0, 0, 0, 1]);
    let mut e6 = RegionEntry {
        variant_count: 2,
        ..Default::default()
    };
    let mut s = s0;
    variants(&mut s, &mut e6, 1, Some(&m2));
    assert_eq!(e6.variants[2][3], 1);
    assert_eq!(s, steps(s0, 4));
}

#[test]
fn entries_on_demand() {
    let mut t = tables();
    t.monstats2.push(Mon2Pop {
        total_pieces: 3,
        ..Mon2Pop::default()
    });
    t.monstats[50].mon_stats_ex = 1;
    let mut regs = state_with(&t, 2).regions;
    *regs.get_mut(2).unwrap() = region_with(&[(5, 1), (6, 1)]);
    let mut s = Seed::init();
    for c in [195, 196, 294, 296] {
        assert_eq!(regs.entry_for(&t, 2, c, &mut s), None);
    }
    assert_eq!(regs.entry_for(&t, 3, 5, &mut s), None, "no region");
    assert_eq!(regs.entry_for(&t, 2, 6, &mut s), Some(1));
    // TotalPieces 3 > 2: filled, entry count + 1, monster count unchanged.
    assert_eq!(regs.entry_for(&t, 2, 50, &mut s), Some(2));
    let r = regs.get(2).unwrap();
    assert_eq!((r.entry_count, r.mon_count, r.entries[2].class), (3, 2, 50));
    assert_eq!(r.entries[2].variant_count, 1, "variant 0 drawn (T = 0)");
    // TotalPieces ≤ 2: the empty slot is returned, nothing filled.
    assert_eq!(regs.entry_for(&t, 2, 7, &mut s), Some(3));
    let r = regs.get(2).unwrap();
    assert_eq!((r.entry_count, r.entries[3].class), (3, 0));
    // A full list: null.
    regs.get_mut(2).unwrap().entry_count = 13;
    assert_eq!(regs.entry_for(&t, 2, 51, &mut s), None);
}

// ---- bookkeeping ---------------------------------------------------

#[test]
fn region_bookkeeping() {
    let t = tables();
    let mut regs = state_with(&t, 8).regions;
    assert!(regs.count_spawn(8, true));
    assert!(!regs.count_spawn(8, false));
    assert_eq!(regs.get(8).unwrap().evil_spawned, 1);
    // Alignment: evil → good/neutral −1, → evil +1 unless old 4; dead,
    // flag 2, same value or a quest level: nothing.
    regs.alignment_changed(8, false, false, 0, 2);
    assert_eq!(regs.get(8).unwrap().evil_spawned, 0);
    regs.alignment_changed(8, false, false, 1, 0);
    regs.alignment_changed(8, false, false, 4, 0);
    regs.alignment_changed(8, true, false, 2, 0);
    regs.alignment_changed(8, false, true, 2, 0);
    regs.alignment_changed(8, false, false, 0, 0);
    assert_eq!(regs.get(8).unwrap().evil_spawned, 1);
    regs.get_mut(8).unwrap().quest = 1;
    regs.alignment_changed(8, false, false, 0, 1);
    assert_eq!(regs.get(8).unwrap().evil_spawned, 1);
    // Kills.
    regs.count_kill(8, false, 0, true);
    regs.count_kill(8, false, 0, true);
    regs.count_kill(8, false, 0, false);
    regs.count_kill(8, true, 0, true);
    regs.count_kill(8, false, 2, true);
    assert_eq!(regs.get(8).unwrap().evil_killed, 1);
    // Den of Evil: remaining = spawned − killed; complete when visited ≥
    // room count and killed = spawned.
    {
        let r = regs.get_mut(8).unwrap();
        r.rooms_visited = 10;
    }
    assert_eq!(regs.den_of_evil(), Some((0, true)));
    // Inactive units.
    regs.slots[2] = Some(Region::default());
    assert_eq!(
        regs.inactive_unit(2, 8, 0, false, false, Some((0, true))),
        Some(8)
    );
    let r = regs.get(8).unwrap();
    assert_eq!((r.evil_spawned, r.bosses), (0, -1));
    assert_eq!(
        regs.inactive_unit(8, 8, 0, false, true, Some((0, true))),
        None
    );
    regs.inactive_unit(2, 8, 0, false, false, None);
    assert_eq!(regs.get(2).unwrap().evil_spawned, -1);
    regs.inactive_unit(2, 8, 0, true, false, None);
    regs.inactive_unit(2, 8, 1, false, false, None);
    assert_eq!(regs.get(2).unwrap().evil_spawned, -1);
}

// ---- presets -------------------------------------------------------

#[test]
fn special_table_matches_tsv() {
    check_special_table(PRESET_MONSTERS_TSV, &SPECIAL_PRESETS).unwrap();
}

#[test]
fn special_table_check_catches_perturbations() {
    let bad = PRESET_MONSTERS_TSV.replacen(
        "267 bloodraven\tpreset\t-1\t0",
        "267 bloodraven\tpreset\t-1\t8",
        1,
    );
    let err = check_special_table(&bad, &SPECIAL_PRESETS).unwrap_err();
    assert!(err.starts_with("id 5: column 5"), "{err}");
    let bad = PRESET_MONSTERS_TSV.replacen("\n26\tdifficulty 0", "\n26\talways", 1);
    let err = check_special_table(&bad, &SPECIAL_PRESETS).unwrap_err();
    assert!(err.starts_with("id 26: column 1"), "{err}");
    let mut short = SPECIAL_PRESETS.to_vec();
    short.pop();
    assert!(check_special_table(PRESET_MONSTERS_TSV, &short).is_err());
}

fn preset(class: i32, x: i32, y: i32) -> PresetUnit {
    PresetUnit {
        unit_type: 1,
        mode: 1,
        class,
        x,
        y,
        has_data: false,
        done: false,
    }
}

#[test]
fn preset_pass_and_ranges() {
    let t = tables();
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    f.boxes.insert(
        R0,
        super::RoomBox {
            x: 100,
            y: 200,
            width: 50,
            height: 50,
        },
    );
    f.presets = vec![
        PresetUnit {
            unit_type: 2,
            ..preset(5, 1, 1)
        },
        PresetUnit {
            done: true,
            ..preset(6, 2, 2)
        },
        PresetUnit {
            has_data: true,
            ..preset(7, 3, 4)
        },
        preset(-1, 5, 5),
    ];
    room_step(&mut ctx(&t, &mut st, &mut f), R0, true);
    assert_eq!(f.allocs(), [7]);
    assert_eq!((f.units[0].x, f.units[0].y), (103, 204));
    assert_eq!(f.units[0].unit_flags, 0x300_0000);
    assert!(f.log.contains(&"preset 0 true".to_string()));
    // The preset's path (list index 2) moves to the monster's map AI
    // before the rest of the preset hooks.
    let at = |l: &str| f.log.iter().position(|x| x == l);
    assert!(at("path 0 2").unwrap() < at("preset 0 true").unwrap());
    // Order: presets, restore, objects, population.
    let i = |p: &str| f.log.iter().position(|l| l.starts_with(p)).unwrap();
    assert!(i("preset") < i("restore") && i("restore") < i("objects"));
    // Not the first population: ambient only.
    let mut f2 = Fake::new();
    f2.presets = f.presets.clone();
    room_step(&mut ctx(&t, &mut st, &mut f2), R0, false);
    assert!(f2.units.is_empty() && f2.log.is_empty());
    assert_eq!(f2.room_seeds[&R0], steps(Seed::init(), 1));
    // Level 136: no monster pass.
    let mut f = Fake::new();
    f.level = 136;
    f.presets = vec![preset(7, 3, 4)];
    place_presets(&mut ctx(&t, &mut st, &mut f), R0);
    assert!(f.units.is_empty());
    // Ranges: M = 600, S = 10 → 600..610 superunique, ≥ 610 special.
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    assert!(preset_spawn(&mut cx, R0, -5, 10, 10, 1).is_none());
    let su = preset_spawn(&mut cx, R0, 600, 10, 10, 1).unwrap();
    assert_eq!(cx.host.unit(su).type_flags & 2, 2, "superunique 0");
    let sp = preset_spawn(&mut cx, R0, 610 + 4, 10, 10, 1).unwrap();
    assert_eq!(cx.host.unit(sp).class, 266, "special id 4 (navi)");
}

#[test]
fn regular_presets() {
    let mut t = tables();
    t.monstats2.push(Mon2Pop {
        critter: true,
        ..Mon2Pop::default()
    });
    t.monstats2.push(Mon2Pop {
        obj_col: true,
        ..Mon2Pop::default()
    });
    t.monstats[100].mon_stats_ex = 1;
    t.monstats[432].mon_stats_ex = 2;
    t.monstats[7].never_count = true;
    let mut st = state_with(&t, 110);
    let mut f = Fake::new();
    f.level = 110;
    f.quest_flags.insert(0x1F);
    let mut cx = ctx(&t, &mut st, &mut f);
    let u = preset_spawn(&mut cx, R0, 498, 10, 10, 1).unwrap();
    assert_eq!(cx.host.unit(u).class, 499);
    let u = preset_spawn(&mut cx, R0, 517, 10, 10, 1).unwrap();
    assert_eq!(cx.host.unit(u).class, 518);
    // Normal places minion1 in level 110; Nightmare does not.
    assert!(preset_spawn(&mut cx, R0, 453, 10, 10, 1).is_some());
    cx.info.difficulty = 1;
    assert!(preset_spawn(&mut cx, R0, 453, 10, 10, 1).is_none());
    assert!(preset_spawn(&mut cx, R0, 529, 10, 10, 1).is_none());
    cx.info.difficulty = 0;
    // prisondoor: x − 1, dead unless quest flag 0x20.
    let u = preset_spawn(&mut cx, R0, 434, 10, 10, 1).unwrap();
    assert_eq!((cx.host.unit(u).x, cx.host.unit(u).mode), (9, 12));
    cx.host.quest_flags.insert(0x20);
    let u = preset_spawn(&mut cx, R0, 434, 10, 10, 1).unwrap();
    assert_eq!(cx.host.unit(u).mode, 1);
    // Critters are not placed.
    assert!(preset_spawn(&mut cx, R0, 100, 10, 10, 1).is_none());
    // neverCount → flag 8 → monster flag 2.
    let u = preset_spawn(&mut cx, R0, 7, 10, 10, 1).unwrap();
    assert_eq!(cx.host.unit(u).monster_flags, 2);
    // Blocked point: retry with r = 4, except the no-retry set.
    cx.host.blocked.insert((20, 20));
    let u = preset_spawn(&mut cx, R0, 8, 20, 20, 1).unwrap();
    assert_ne!((cx.host.unit(u).x, cx.host.unit(u).y), (20, 20));
    for c in [229, 284, 288, 392, 393] {
        assert!(preset_spawn(&mut cx, R0, c, 20, 20, 1).is_none(), "{c}");
    }
    // barricadedoor1 with objCol: object 571 at its position (open
    // question 3).
    let u = preset_spawn(&mut cx, R0, 432, 30, 30, 1).unwrap();
    let (x, y) = (cx.host.unit(u).x, cx.host.unit(u).y);
    assert!(f.log.contains(&format!("object 571 {x} {y}")));
}

#[test]
fn superunique_presets() {
    let mut t = tables();
    t.superuniques[0] = super::SuperPop {
        class: 58,
        hc_idx: 0,
        min_grp: 2,
        max_grp: 2,
        auto_pos: 1,
        stacks: 0,
    };
    t.superuniques[1] = super::SuperPop {
        class: 30,
        hc_idx: 42,
        min_grp: 0,
        max_grp: 0,
        auto_pos: 0,
        stacks: 1,
    };
    t.superuniques[2].class = -1;
    t.monstats[58].minion1 = 19;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    cx.info.difficulty = 1;
    // Bishibosh-like: AutoPos → point searched; flag bit set; type 2; min/max
    // + difficulty (3 each).
    let b = preset_spawn(&mut cx, R0, 600, 10, 10, 1).unwrap();
    assert!(cx.state.superunique_placed(0));
    assert_ne!((cx.host.unit(b).x, cx.host.unit(b).y), (10, 10));
    assert_eq!(cx.host.unit(b).type_flags & 0xB, 0xB);
    assert_eq!(cx.host.units.iter().filter(|u| u.class == 19).count(), 3);
    assert_eq!(cx.state.regions.get(2).unwrap().bosses, 1, "counts as boss");
    // Stacks 0: not placed twice.
    assert!(preset_spawn(&mut cx, R0, 600, 10, 10, 1).is_none());
    // Stacks 1, MinGrp = MaxGrp = 0: roll(1) still steps the boss seed; no
    // minions; hcIdx 42 group spawn; modifier 22 last.
    let n0 = cx.host.units.len();
    let b = preset_spawn(&mut cx, R0, 601, 40, 40, 1).unwrap();
    // No unique minions; the hcIdx 42 group (mode 1,
    // r 20, 20 spawns of 453, flags 0) is the only other creation.
    let group: Vec<_> = (n0 + 1..cx.host.units.len()).collect();
    assert!(group.iter().all(|&i| cx.host.units[i].class == 453));
    assert_eq!(group.len(), 20, "the hcIdx 42 group");
    // The boss seed: derived at allocation, then roll(1) (one step).
    let mut g = steps(Seed::init(), n0);
    let bs = g.derive();
    assert_eq!(cx.host.unit(b).seed, steps(bs, 1));
    assert_eq!((cx.host.unit(b).x, cx.host.unit(b).y), (40, 40));
    assert!(
        preset_spawn(&mut cx, R0, 601, 40, 40, 1).is_some(),
        "stacks"
    );
    let log = &f.log;
    // Each group unit: owner data (boss GUID, 1, 0, 0) and the boss's
    // minion list, before the closing modifier 22.
    let g0 = crate::units::UnitId(group[0] as u32);
    let owner = format!("owner {} Guid(UnitId({})) 1 0 0", g0.0, b.0);
    assert!(log.contains(&owner), "{log:?}");
    assert!(log.contains(&format!("minion {} {}", b.0, g0.0)));
    let last_b = log
        .iter()
        .rposition(|l| l.starts_with(&format!("mod {} 22", b.0)));
    let group_b = log.iter().position(|l| *l == owner);
    assert!(group_b < last_b);
    // Class < 0 or difficulty ≥ 3: nothing.
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    assert!(preset_spawn(&mut cx, R0, 602, 10, 10, 1).is_none());
    cx.info.difficulty = 3;
    assert!(preset_spawn(&mut cx, R0, 601, 10, 10, 1).is_none());
    assert!(f.units.is_empty());
}

#[test]
fn special_presets() {
    let mut t = tables();
    let sp = |id: i32| 600 + 10 + id;
    t.levels[2].umon = vec![30];
    t.monstats[261].base_id = 261;
    t.monstats[261].next_in_class = 262;
    t.monstats[262].next_in_class = 263;
    t.monstats[263].next_in_class = -1;
    t.monstats[438].never_count = true;
    t.monstats[492].min_grp = 2;
    t.monstats[492].max_grp = 3;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let mut cx = ctx(&t, &mut st, &mut f);
    // id 2: random unique, never a champion, nothing returned.
    cx.host.champion = true;
    assert!(preset_spawn(&mut cx, R0, sp(2), 10, 10, 1).is_none());
    assert_eq!(cx.host.units[0].class, 30);
    assert!(cx.host.log.contains(&"bossmods 0 false".to_string()));
    // id 3: champion at the preset point with modifier 16 and 1–3 minions.
    let n0 = cx.host.units.len();
    let c = preset_spawn(&mut cx, R0, sp(3), 50, 50, 1).unwrap();
    assert_eq!((cx.host.unit(c).x, cx.host.unit(c).y), (50, 50));
    assert!(cx.host.log.contains(&format!("mod {} 16", c.0)));
    let minions = cx.host.units.len() - n0 - 1;
    assert!((1..=3).contains(&minions));
    // ids 10/11: the chain, n by level.
    for (lvl, want) in [(2, 261), (77, 262), (92, 263)] {
        cx.host.level = lvl;
        let u = preset_spawn(&mut cx, R0, sp(11), 60, 60, 1).unwrap();
        assert_eq!(cx.host.unit(u).class, want, "level {lvl}");
    }
    cx.host.level = 2;
    // id 17: fallen1 → class for level → swap by level (7 → fallen3 21).
    cx.host.level = 7;
    let u = preset_spawn(&mut cx, R0, sp(17), 70, 70, 1).unwrap();
    assert_eq!(cx.host.unit(u).class, 21);
    cx.host.level = 6;
    let u = preset_spawn(&mut cx, R0, sp(18), 70, 70, 1).unwrap();
    assert_eq!(cx.host.unit(u).class, 59);
    cx.host.level = 12;
    let u = preset_spawn(&mut cx, R0, sp(18), 70, 70, 1).unwrap();
    assert_eq!(cx.host.unit(u).class, 60);
    cx.host.level = 2;
    // id 29–32: mode 12, unit flag 0x2000000; 438 schedules event 7.
    let u = preset_spawn(&mut cx, R0, sp(32), 80, 80, 1).unwrap();
    assert_eq!(cx.host.unit(u).mode, 12);
    assert_eq!(cx.host.unit(u).unit_flags & 0x200_0000, 0x200_0000);
    assert_eq!(cx.host.unit(u).monster_flags, 2, "neverCount");
    assert!(cx.host.log.contains(&format!("monumod {}", u.0)));
    // id 24: Normal only, imp1 pack at the preset point.
    let n0 = cx.host.units.len();
    let l = preset_spawn(&mut cx, R0, sp(24), 90, 90, 1).unwrap();
    assert_eq!(cx.host.unit(l).class, 492);
    assert!((1..=2).contains(&(cx.host.units.len() - n0 - 1)));
    cx.info.difficulty = 1;
    assert!(preset_spawn(&mut cx, R0, sp(24), 90, 90, 1).is_none());
    cx.info.difficulty = 0;
    // ids 25, 27, 28 and unlisted ids: nothing.
    for id in [25, 27, 28, 0, 1, 33] {
        assert!(
            preset_spawn(&mut cx, R0, sp(id), 90, 90, 1).is_none(),
            "{id}"
        );
    }
}

#[test]
fn class_for_level_rules() {
    let mut t = tables();
    // Chain 453 → 454 → 455 → 456, Level 10/20/30.
    for (c, next, lvl) in [(453, 454, 5), (454, 455, 10), (455, 456, 20), (456, -1, 30)] {
        t.monstats[c].base_id = 453;
        t.monstats[c].next_in_class = next;
        t.monstats[c].level = lvl;
        t.monstats[c].chain_len = 4;
    }
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    // No mon list: unchanged.
    assert_eq!(class_for_level(&ctx(&t, &mut st, &mut f), R0, 453), 453);
    // A list entry with the same BaseId wins.
    t.levels[2].mon = vec![5, 455];
    assert_eq!(class_for_level(&ctx(&t, &mut st, &mut f), R0, 453), 455);
    // Otherwise the chain while Level ≤ MonLvl1Ex + 1.
    t.levels[2].mon = vec![5];
    t.levels[2].mon_lvl_ex = [19, 0, 0];
    assert_eq!(class_for_level(&ctx(&t, &mut st, &mut f), R0, 453), 455);
    t.levels[2].mon_lvl_ex = [5, 0, 0];
    assert_eq!(class_for_level(&ctx(&t, &mut st, &mut f), R0, 453), 453);
    // The chain length limits the walk.
    t.monstats[453].chain_len = 1;
    t.levels[2].mon_lvl_ex = [40, 0, 0];
    assert_eq!(class_for_level(&ctx(&t, &mut st, &mut f), R0, 453), 454);
}

// ---- ambient -------------------------------------------------------

/// A room seed whose first step has `& 0x7FFF = 0` and second `mod 100 = d`.
fn ambient_seed(d: u32) -> Seed {
    for lo in 1.. {
        let mut s = Seed::init_low(lo);
        if s.step() & 0x7FFF == 0 && s.step() % 100 == d {
            return Seed::init_low(lo);
        }
    }
    unreachable!()
}

#[test]
fn ambient_rules() {
    let mut t = tables();
    t.levels[2].mon_wndr = 1;
    let mut st = state_with(&t, 2);
    // Gate 1: 1/32768.
    let mut f = Fake::new();
    ambient(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.room_seeds[&R0], steps(Seed::init(), 1));
    let s2 = ambient_seed(2);
    // Passes: rogue2 (270) at a spawn point, realigned (0, 8), counted.
    let mut f = Fake::new();
    f.room_seeds.insert(R0, s2);
    ambient(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(f.allocs(), [270]);
    assert!(f.log.contains(&"realign 0 0 8".to_string()));
    assert_eq!(st.regions.get(2).unwrap().wanderers, 1);
    // Clients present, MonWndr 0, draw ≥ 3, 3 wanderers, act ≥ 1: nothing.
    type Case<'a> = &'a dyn Fn(&mut Fake, &mut PopTables, &mut super::PopState);
    let cases: [Case; 5] = [
        &|f, _, _| f.clients = 1,
        &|_, t, _| t.levels[2].mon_wndr = 0,
        &|f, _, _| {
            f.room_seeds.insert(R0, ambient_seed(3));
        },
        &|_, _, s| s.regions.get_mut(2).unwrap().wanderers = 3,
        &|_, t, _| t.levels[2].act = 1,
    ];
    for (i, case) in cases.iter().enumerate() {
        let mut t2 = t.clone();
        let mut st2 = state_with(&t2, 2);
        let mut f = Fake::new();
        f.room_seeds.insert(R0, s2);
        case(&mut f, &mut t2, &mut st2);
        ambient(&mut ctx(&t2, &mut st2, &mut f), R0);
        assert!(f.units.is_empty(), "case {i}");
    }
}

// ------------------------------------------------------------------

#[test]
fn spawn_mode_xy_rule() {
    let mut t = tables();
    t.monstats[206].spawn = 210;
    t.monstats[206].spawn_x = -2;
    t.monstats[206].spawn_y = 3;
    t.monstats[206].spawn_mode = 16;
    let mut st = state_with(&t, 2);
    let mut f = Fake::new();
    let cx = ctx(&t, &mut st, &mut f);
    assert_eq!(spawn_mode_xy(&cx, 206, 100, 100), Some((210, 98, 103, 1)));
}

// Tests written against surviving mutants; a child module so
// they share this module's fakes.
#[path = "../../mutant_tests/population.rs"]
mod mutant_tests;
