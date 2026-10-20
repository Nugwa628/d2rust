// mutation run found unchecked; fakes from the parent test module)
use super::*;

/// A unit with an (empty) monster data entry and no type init, so its
/// seed is still `Seed::init_low(seed)`.
fn bare(f: &mut Fake, class: u32, seed: u32, umods: [u8; MAX_UMODS]) -> UnitId {
    let u = f.unit(class, seed);
    f.store.entry(u).umods = umods;
    u
}

// ---------

#[test]
fn fpick_3_needs_mode_wl() {
    // fPick 3: the class must have mode WL in monstats2; without it, no.
    let mut t = boss_tables();
    t.monumod[9].fpick = 3;
    let mut f = fake_with(t);
    let cx = f.cx;
    assert!(!eligible(&cx, &mut f, 0, 9));
}

#[test]
fn champion_pick_takes_first_weight_above_remainder() {
    // Candidates 16, 36, 37, 38, 39 (weight 1 each, expansion): the first
    // whose weight exceeds the remainder of roll(5), i.e. index roll(5).
    let cands = [16, 36, 37, 38, 39];
    let mut f = fake_with(boss_tables());
    let cx = f.cx;
    let mut later = false;
    for s in 1..20u32 {
        let r = Seed::init_low(s).roll(5) as usize;
        later |= r > 0;
        let u = bare(&mut f, 0, s, [0; MAX_UMODS]);
        assert_eq!(pick_champion(&cx, &mut f, u, 0), cands[r], "seed {s}");
    }
    assert!(later, "some seed rolls past the first candidate");
}

#[test]
fn champion_chance_is_strict() {
    // roll(100) < chance makes a champion; roll(100) = chance does not.
    for &s in &SEEDS {
        let r = Seed::init_low(s).roll(100);
        for (chance, champion) in [(r, false), (r + 1, true)] {
            let mut t = boss_tables();
            t.monumod[0].constants = chance;
            let mut f = fake_with(t);
            let cx = f.cx;
            let u = bare(&mut f, 1, s, [0; MAX_UMODS]);
            choose_umods(&cx, &mut f, u, true);
            let has = f.data(u).has_flag(type_flag::CHAMPION);
            assert_eq!(has, champion, "seed {s} chance {chance}");
        }
    }
}

#[test]
fn umod_count_clamped_by_sum() {
    // Hell (d 2): count 3. With 4 existing, 3 + 4 < 9 → 3 picks (7 umods).
    let mut t = boss_tables();
    t.monumod[0].constants = 0;
    let mut f = fake_with(t);
    f.info.difficulty = 2;
    let cx = f.cx;
    let u = bare(&mut f, 1, 1, [13, 14, 15, 20, 0, 0, 0, 0, 0]);
    choose_umods(&cx, &mut f, u, false);
    assert_eq!(f.data(u).umod_count(), 7);
    // With 7 existing, 3 + 7 ≥ 9 → count 9 − 7 = 2: roll(1) and two
    // picks, three steps in all.
    let u = bare(&mut f, 1, 1, [13, 14, 15, 20, 21, 22, 33, 0, 0]);
    choose_umods(&cx, &mut f, u, false);
    let mut s = Seed::init_low(1);
    for _ in 0..3 {
        s.step();
    }
    assert_eq!(f.seed_of(u), s);
    assert_eq!(f.data(u).umod_count(), 9);
}

// ---------

#[test]
fn minion_class_is_minion1_when_valid() {
    // minion1 a valid class → that class; minion1 = the row count (not a
    // class) → the boss's own class.
    for (minion1, want) in [(1u16, 1u32), (2, 0)] {
        let mut t = boss_tables();
        t.monstats[0].minion1 = minion1;
        let mut f = fake_with(t);
        let cx = f.cx;
        let b = f.monster(0, 1);
        boss_minions_and_init(&cx, &mut f, b, 1, 1, None, true);
        let ms = f.minions(b);
        assert_eq!(ms.len(), 1);
        assert_eq!(f.units.get(ms[0]).unwrap().class, want, "minion1 {minion1}");
    }
}

// ---------

#[test]
fn ghostly() {
    // 36: type flag 0x40, damageresist 80, champion function (no velocity
    // change for 36), then cold damage at the new level: DM 30 (monlvl DM
    // = level) × K[22] 33 / 100 = 9, × K[25] 50 / 100 = 15; coldlength
    // += 150.
    let mut f = fake(vec![velocity_mon(6, 1)]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 31);
    f.set_stat(u, stat::EXPERIENCE, 160);
    f.set_stat(u, stat::COLDLENGTH, 10);
    run_umod_init(&cx, &mut f, u, 36, true);
    assert!(f.data(u).has_flag(type_flag::GHOSTLY));
    assert_eq!(f.s(u, stat::DAMAGERESIST), 80);
    assert_eq!(f.s(u, stat::LEVEL), 30);
    assert_eq!(f.s(u, stat::EXPERIENCE), 96);
    assert_eq!(f.s(u, stat::DAMAGEPERCENT), 90);
    assert_eq!(f.s(u, stat::ITEM_TOHIT_PERCENT), 67);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75);
    assert_eq!(f.s(u, stat::COLDMINDAM), 9);
    assert_eq!(f.s(u, stat::COLDMAXDAM), 15);
    assert_eq!(f.s(u, stat::COLDLENGTH), 160);
    // Unique only (`unique_gate` yes).
    let v = f.monster(0, 1);
    run_umod_init(&cx, &mut f, v, 36, false);
    assert!(!f.data(v).has_flag(type_flag::GHOSTLY));
    assert_eq!(f.s(v, stat::DAMAGERESIST), 0);
}

#[test]
fn fanatic_velocity_inside_clamp() {
    // 37: velocitypercent += clamp(2048 / 12 − 128, 10, 100) = 42.
    let mut f = fake(vec![velocity_mon(12, 1)]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    run_umod_init(&cx, &mut f, u, 37, true);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75 + 42);
}

#[test]
fn fast_needs_positive_velocity() {
    // 6 with Velocity 0: no change.
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    run_umod_init(&cx, &mut f, u, 6, false);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75);
}

#[test]
fn lightning_and_poison_resist() {
    // 17 / 23 tail: light / poison +75.
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    for (umod, s) in [(17, stat::LIGHTRESIST), (23, stat::POISONRESIST)] {
        let u = f.monster(0, 1);
        run_umod_init(&cx, &mut f, u, umod, true);
        assert_eq!(f.s(u, s), 75, "umod {umod}");
    }
}

#[test]
fn resist_counts_only_reaching_100() {
    // 8: cold 3 + 40 = 43 stays below 100, so it is not counted; with
    // poison immune (1), fire and light still get +40.
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    f.set_stat(u, stat::COLDRESIST, 3);
    f.set_stat(u, stat::POISONRESIST, 100);
    run_umod_init(&cx, &mut f, u, 8, true);
    let res = [stat::FIRERESIST, stat::LIGHTRESIST, stat::COLDRESIST].map(|s| f.s(u, s));
    assert_eq!(res, [40, 40, 43]);
}

#[test]
fn elemental_minion_and_mana_drain() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    // unique = 0, Normal: K[16] = 0 → firemindam += 0.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 10);
    run_umod_init(&cx, &mut f, u, 9, false);
    assert_eq!(f.s(u, stat::FIREMINDAM), 0);
    // 25: DM 10 × K[28] 66 / 100 × 256, DM 10 × K[31] 100 / 100 × 256.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 10);
    run_umod_init(&cx, &mut f, u, 25, true);
    assert_eq!(f.s(u, stat::MANADRAINMINDAM), 6 * 256);
    assert_eq!(f.s(u, stat::MANADRAINMAXDAM), 10 * 256);
}

#[test]
fn aura_thorns_row() {
    // Level 999: all 8 rows; index 7 thorns (999, 0, 0, 1): level
    // clamp(999 × 0 / 1, 1, 99) = 1.
    let s = (0u16..)
        .find(|&s| Seed::init_low(u32::from(s)).roll(8) == 7)
        .unwrap();
    assert_eq!(aura_choice(999, s, 1, None), (103, 1));
}

// ---------

#[test]
fn superunique_mods_need_fewer_than_5() {
    let mut t = boss_tables();
    t.superuniques = vec![su_row(0, 0, [5, 0, 0]), su_row(0, 0, [30, 0, 0])];
    let mut f = fake_with(t);
    let cx = f.cx;
    // 5 umods already: nothing appended.
    let u = bare(&mut f, 0, 1, [13, 14, 15, 20, 21, 0, 0, 0, 0]);
    superunique_mods(&cx, &mut f, u, 0);
    assert_eq!(f.data(u).umod_list(), [13, 14, 15, 20, 21]);
    // The result says whether 30 was among Mod1–Mod3.
    let u = bare(&mut f, 0, 1, [0; MAX_UMODS]);
    assert_eq!(superunique_mods(&cx, &mut f, u, 0), Some(false));
    assert_eq!(f.data(u).umod_list(), [5]);
    let u = bare(&mut f, 0, 1, [0; MAX_UMODS]);
    assert_eq!(superunique_mods(&cx, &mut f, u, 1), Some(true));
}

// ---------

#[test]
fn lightning_mode_needs_unique_and_gethit() {
    // 17 mode 1: unique and new mode 3, both required.
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    for (unique, m) in [(false, mode::GETHIT), (true, mode::NEUTRAL)] {
        let v = f.monster(0, 1);
        f.units.get_mut(v).unwrap().mode = m;
        f.store.entry(v).umods = [17, 0, 0, 0, 0, 0, 0, 0, 0];
        if unique {
            f.store.entry(v).type_flags = type_flag::UNIQUE;
        }
        dispatch(&cx, &mut f, v, None, 1);
        assert!(
            f.game.timers.unit_timers(v).is_empty(),
            "unique {unique} mode {m}"
        );
    }
}

#[test]
fn ai_after_death_only_on_death() {
    // 34 mode 1 acts only when the new mode is 0: in
    // another mode no cancel and no seed step.
    let mut m = velocity_mon(0, 1);
    m.aip8 = 100;
    let mut f = fake(vec![m]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    f.store.entry(u).umods = [34, 0, 0, 0, 0, 0, 0, 0, 0];
    f.game
        .schedule_event(u, EVENT_UMOD, 99, None, 0, 0)
        .unwrap();
    let before = f.seed_of(u);
    dispatch(&cx, &mut f, u, None, 1);
    assert_eq!(f.seed_of(u), before);
    let t = f.game.timers.unit_timers(u);
    assert_eq!(t.len(), 1);
    assert_eq!(f.game.timers.expire(t[0]), Some(99));
}

#[test]
fn ai_after_death_strict_chance_and_frame() {
    // lo' % 100 < aip8 schedules at frame + 10 × aip1 + 1; lo' % 100 =
    // aip8 does not.
    for &s in &SEEDS {
        for strict in [true, false] {
            let mut m = velocity_mon(0, 1);
            m.aip1 = 3;
            let mut f = fake(vec![m.clone()]);
            let u = f.monster(0, s);
            let draw = (f.seed_of(u).step() % 100) as u16;
            // Rebuild with aip8 at the draw (or one above); same seed.
            m.aip8 = if strict { draw + 1 } else { draw };
            let mut f = fake(vec![m]);
            let cx = f.cx;
            let u = f.monster(0, s);
            f.game.frame = 10;
            f.units.get_mut(u).unwrap().mode = mode::DEATH;
            f.store.entry(u).umods = [34, 0, 0, 0, 0, 0, 0, 0, 0];
            dispatch(&cx, &mut f, u, None, 1);
            let t = f.game.timers.unit_timers(u);
            if strict {
                assert_eq!(t.len(), 1, "seed {s}");
                assert_eq!(f.game.timers.expire(t[0]), Some(10 + 10 * 3 + 1));
            } else {
                assert!(t.is_empty(), "seed {s}");
            }
        }
    }
}
