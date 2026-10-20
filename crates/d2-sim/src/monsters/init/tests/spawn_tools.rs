//! A tool that calls the boss functions itself (explicit umods, region
//! counters, draw order) and the data a class reinit keeps.

use super::*;

fn units_created(f: &Fake) -> usize {
    f.log.iter().filter(|l| l.starts_with("minion")).count()
}

#[test]
fn umod_count_is_the_bytes_before_the_first_zero_at_most_nine() {
    let mut d = MonsterData::default();
    assert_eq!(d.umod_count(), 0);
    d.umods = [5, 6, 0, 7, 0, 0, 0, 0, 0];
    assert_eq!((d.umod_count(), d.umod_list()), (2, &[5, 6][..]));
    // Nine entries: no terminator, count 9, and no tenth is written.
    d.umods = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    assert_eq!(d.umod_count(), 9);
    assert_eq!(d.umod_list().len(), 9);
    d.push_umod(42);
    assert_eq!(d.umods, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    // push appends at index count.
    let mut d = MonsterData::default();
    d.push_umod(8);
    d.push_umod(9);
    assert_eq!(d.umods, [8, 9, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn choose_umods_writes_only_the_list_and_the_champion_flag() {
    for &s in &SEEDS {
        for chance in [0, 100] {
            let mut t = boss_tables();
            t.monumod[0].constants = chance;
            let mut f = fake_with(t);
            f.info.expansion = false;
            let cx = f.cx;
            let u = f.monster(1, s);
            let before = f.data(u);
            choose_umods(&cx, &mut f, u, true);
            let mut after = f.data(u);
            assert!(after.umod_count() >= 1, "chance {chance}");
            let champion = after.type_flags & !before.type_flags;
            assert_eq!(
                champion,
                if chance == 100 {
                    type_flag::CHAMPION
                } else {
                    0
                }
            );
            // Everything but the list and flag 4 is as before.
            after.umods = before.umods;
            after.type_flags = before.type_flags;
            assert_eq!(after, before, "seed {s} chance {chance}");
        }
    }
}

#[test]
fn scripted_champion_gets_no_minions_and_unique_inits_scripted_boss_may_have_none() {
    // A champion with scripted umod 9 (fire): flag 4 and the list by
    // hand, then.
    let mut f = fake_with(boss_tables());
    let cx = f.cx;
    let u = f.monster(1, 1);
    f.store.entry(u).push_umod(9);
    f.store.entry(u).type_flags |= type_flag::CHAMPION;
    f.set_stat(u, stat::LEVEL, 10);
    boss_minions_and_init(&cx, &mut f, u, 3, 6, None, true);
    assert_eq!(units_created(&f), 0, "flag 4: no minions (§18 step 1)");
    // Umod 9 ran with unique = 1: damage and the +75 fire resist.
    assert_eq!(f.s(u, stat::FIRERESIST), 75);
    assert!(f.s(u, stat::FIREMAXDAM) > 0);

    // A non-champion boss with min 0 / max 0: no minions either, the
    // umod inits still run unique.
    let mut f = fake_with(boss_tables());
    let cx = f.cx;
    let u = f.monster(1, 1);
    f.store.entry(u).push_umod(9);
    f.set_stat(u, stat::LEVEL, 10);
    boss_minions_and_init(&cx, &mut f, u, 0, 0, None, true);
    assert_eq!(units_created(&f), 0);
    assert_eq!(f.s(u, stat::FIRERESIST), 75);
    // With min 3 / max 6 and no flag 4 a non-champion does get 3..=6.
    let mut f = fake_with(boss_tables());
    let cx = f.cx;
    let u = f.monster(1, 1);
    f.store.entry(u).push_umod(9);
    boss_minions_and_init(&cx, &mut f, u, 3, 6, None, true);
    assert!((3..=6).contains(&units_created(&f)));
}

#[test]
fn skipping_choose_umods_skips_its_unit_seed_draws() {
    for &s in &SEEDS {
        let mut t = boss_tables();
        t.monumod[0].constants = 0; // no champion: roll(100), roll(1), picks
        let mut f = fake_with(t);
        f.info.expansion = false;
        let cx = f.cx;
        // A: the real function. B: the same list written by hand.
        let a = f.monster(1, s);
        let b = f.monster(1, s);
        assert_eq!(f.seed_of(a), f.seed_of(b));
        let start = f.seed_of(a);
        choose_umods(&cx, &mut f, a, true);
        assert_ne!(f.seed_of(a), start, "choose_umods draws on the unit seed");
        f.store.entry(b).umods = f.data(a).umods;
        assert_eq!(f.seed_of(b), start, "writing the list draws nothing");
        for u in [a, b] {
            f.set_stat(u, stat::LEVEL, 10);
            boss_minions_and_init(&cx, &mut f, u, 0, 0, None, true);
        }
        assert_ne!(f.seed_of(a), f.seed_of(b), "seed {s}: later draws differ");
        assert_ne!(f.data(a).name_seed, f.data(b).name_seed);
    }
}

#[test]
fn tool_bosses_and_pack_members_count_in_the_region_once() {
    let mut f = fake_with(boss_tables());
    let cx = f.cx;
    // boss.
    let u = random_boss(&cx, &mut f, &CreateRequest::default(), false, true).unwrap();
    assert_eq!(f.region_bosses, 1);
    // member: +1; a second call on a champion does nothing.
    let m = f.monster(1, 5);
    champion_pack_member(&cx, &mut f, m, 16);
    assert_eq!(f.region_bosses, 2);
    champion_pack_member(&cx, &mut f, m, 16);
    assert_eq!(f.region_bosses, 2);
    // The boss mark itself counts only while flag 8 is clear.
    mark_unique(&mut f, u);
    assert_eq!(f.region_bosses, 2);
    assert!(f.data(m).has_flag(type_flag::CHAMPION));
}

#[test]
fn random_boss_order_is_spawn_choose_minions_and_init() {
    let mut t = boss_tables();
    t.monumod[0].constants = 100; // champion: member count follows
    let mut f = fake_with(t);
    f.info.expansion = false;
    let cx = f.cx;
    let u = random_boss(&cx, &mut f, &CreateRequest::default(), true, true).unwrap();
    // first (log), then the umod choice (champion list [16],
    // flag 4), then the minions / init of a champion: none.
    assert!(f.log.iter().any(|l| l.starts_with("boss_spawn")));
    assert_eq!(f.data(u).umod_list(), [16]);
    assert_eq!(units_created(&f), 0);
    // The same call with the champion branch off: choose picks uniques and
    // spawns the 3..=6 minions after them; the umod inits run last, so
    // each minion already has the boss's transferable umods.
    let mut t = boss_tables();
    t.monumod[0].constants = 0;
    let mut f = fake_with(t);
    f.info.expansion = false;
    let cx = f.cx;
    let u = random_boss(&cx, &mut f, &CreateRequest::default(), true, true).unwrap();
    assert!((3..=6).contains(&units_created(&f)));
    let log_pos = |p: &str| f.log.iter().position(|l| l.starts_with(p)).unwrap();
    assert!(log_pos("boss_spawn") < log_pos("minion"));
    assert!(f.data(u).umod_count() >= 1);
}

#[test]
fn reinit_keeps_umods_flags_and_name_seed_and_runs_no_umod_init() {
    // Class 0: HP 5..9; class 1: HP 50 exactly, so the new class's HP is
    // known; umod 5 (strong) would multiply it if the init re-ran.
    let a = mon(2, 5, 9, 32);
    let b = mon(2, 50, 50, 32);
    let mut f = fake_with(Tables::new(vec![a, b]));
    let cx = f.cx;
    let u = f.monster(0, 1);
    let reference = f.monster(1, 1);
    {
        let d = f.store.entry(u);
        d.umods[0] = 5;
        d.name_seed = 0x1234;
        d.type_flags |= type_flag::UNIQUE | type_flag::BOSS;
    }
    assert!(reinit(&cx, &mut f, u, 1, 1));
    let d = f.data(u);
    assert_eq!(d.umod_list(), [5]);
    assert_eq!(d.name_seed, 0x1234);
    assert_eq!(d.type_flags, type_flag::UNIQUE | type_flag::BOSS);
    // The new class's stats, unmodified by umod 5.
    assert_eq!(f.s(u, stat::MAXHP), f.s(reference, stat::MAXHP));
    assert_eq!(f.s(u, stat::MAXHP), 50 * 256);
    assert_eq!(
        f.s(u, stat::DAMAGEPERCENT),
        f.s(reference, stat::DAMAGEPERCENT)
    );
}
