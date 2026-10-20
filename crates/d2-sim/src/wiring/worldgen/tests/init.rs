//! Monster init on the real unit records, AI store and population
//! regions, reached through population's seams.

use super::population::{isle, monsters};
use super::*;
use crate::monsters::init::type_flag;
use crate::monsters::population::{preset, MonsterInit};

/// Monster class 0 with two component choices (3 and 2 pieces): its
/// region entries get appearance variants.
fn composite(t: &mut WorldTables) {
    let m2 = &mut t.pop.monstats2[0];
    m2.components[0] = 3;
    m2.components[1] = 2;
    m2.composit_total = 2;
    m2.total_pieces = 3;
}

#[test]
fn components_come_from_the_region_entry_on_the_unit_seed() {
    let mut fx = Fx::with_tables(isle_ds1s(), composite);
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    let entry = {
        let r = fx.sim.world.pop.regions.get(ISLE as i32).unwrap();
        assert_eq!(r.entries[0].class, 0);
        r.entries[0]
    };
    assert!(entry.variant_count > 1);
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    fx.assert_clean();
    let u = monsters(&fx)[0];
    // The type init's first unit-seed draw picks the variant.
    let rec = fx.sim.action.sys.units.get(u).unwrap();
    let i = Seed::init_low(rec.init_seed).roll(i32::from(entry.variant_count));
    let md = fx.sim.world.monsters.get(u).unwrap();
    assert_eq!(md.components, entry.variants[i as usize]);
    // The found entry is not added again.
    let r = fx.sim.world.pop.regions.get(ISLE as i32).unwrap();
    assert_eq!(r.entry_count, 1);
}

#[test]
fn champion_pack_member_counts_in_the_real_region() {
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    let u = monsters(&fx)[0];
    let bosses = fx.sim.world.pop.regions.get(ISLE as i32).unwrap().bosses;
    let seed = fx.sim.action.sys.units.get(u).unwrap().seed;
    // Population's modifier 16 call, with the regions it
    // holds.
    fx.sim
        .population(&mut fx.game, |cx| cx.host.add_modifier(u, 16, cx.state));
    fx.assert_clean();
    let md = fx.sim.world.monsters.get(u).unwrap();
    assert!(md.has_flag(type_flag::BOSS | type_flag::CHAMPION));
    assert!(md.has_flag(type_flag::UNIQUE));
    assert!(md.has_umod(16));
    // The boss counter of the monster's region.
    assert_eq!(
        fx.sim.world.pop.regions.get(ISLE as i32).unwrap().bosses,
        bosses + 1
    );
    // The boss-modifier init's step 2 ran umod 1 (the name seed draw) on the unit seed.
    let mut s = seed;
    let v = s.step() as u16;
    assert_eq!(md.name_seed, v);
    // A second call does nothing (type flag 4).
    fx.sim
        .population(&mut fx.game, |cx| cx.host.add_modifier(u, 16, cx.state));
    assert_eq!(
        fx.sim.world.pop.regions.get(ISLE as i32).unwrap().bosses,
        bosses + 1
    );
}

/// A montype matrix of 4 rows: 1 and 2 are their
/// own types, 2 is nested in 1, 3 in 2 (so in 1 too); row and column 0
/// empty.
fn montype_matrix() -> d2_data::fixup::maps::EquivMatrix {
    let row = |cols: &[usize]| cols.iter().fold(0u32, |w, &c| w | 1 << c);
    d2_data::fixup::maps::EquivMatrix {
        n: 4,
        words: 1,
        bits: vec![0, row(&[1]), row(&[1, 2]), row(&[1, 2, 3])],
    }
}

#[test]
fn montype_nesting_reads_the_montype_matrix() {
    use crate::monsters::init::InitHost;
    let mut fx = Fx::with_tables(isle_ds1s(), |t| t.montype_equiv = montype_matrix());
    let got = fx.sim.host(&mut fx.game, |h| {
        [
            (1, 1),
            (2, 1),
            (3, 1),
            (3, 2),
            (1, 2),
            (2, 3),
            (0, 0),
            (0, 1),
            (1, 0),
            (4, 4),
            (0xFFFF, 1),
        ]
        .map(|(m, t)| h.montype_is(m, t))
    });
    // Nested types answer yes; parents are not of a child's type; row 0,
    // column 0 and out-of-range rows are no (unlike plain equality).
    assert_eq!(
        got,
        [true, true, true, true, false, false, false, false, false, false, false]
    );
}

#[test]
fn init_host_answers_from_the_wired_world() {
    use crate::monsters::init::InitHost;
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    fx.assert_clean();
    let u = monsters(&fx)[0];
    let (level, before, after, inv, item) = fx.sim.host(&mut fx.game, |h| {
        let before = h.minions(u);
        h.link_minion(u, UnitId(900));
        h.link_minion(u, UnitId(901));
        (
            h.level_id(u),
            before,
            h.minions(u),
            h.has_inventory(u),
            h.has_item_at(u, 4),
        )
    });
    // The level of the unit's room; a unit in no room is in level 0.
    assert_eq!(level, ISLE as i32);
    assert_eq!(fx.sim.host(&mut fx.game, |h| h.level_id(UnitId(999))), 0);
    // The minion list, in link order; other units have none.
    assert!(before.is_empty());
    assert_eq!(after, [UnitId(900), UnitId(901)]);
    assert!(fx
        .sim
        .host(&mut fx.game, |h| h.minions(UnitId(900)))
        .is_empty());
    // No monster inventory exists in this wiring (`new_inventory` pending).
    assert!(!inv && !item);
}

#[test]
fn init_host_state_reaches_the_action_systems() {
    use crate::monsters::init::InitHost;
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    let u = monsters(&fx)[0];
    fx.sim.action.sys.hooks.x.log.clear();
    let (base, info) = fx.sim.host(&mut fx.game, |h| {
        // Base stats (layer 0) round-trip through the unit's stat list.
        h.set_stat(u, 7, 1234);
        h.set_stat(u, 6, 77);
        h.set_difficulty(2);
        InitHost::set_alignment(h, u, 1);
        ([h.stat(u, 7), h.stat(u, 6)], h.info())
    });
    fx.assert_clean();
    assert_eq!(base, [1234, 77]);
    assert_eq!(fx.sim.action.sys.stats.unit_base(u, 7, 0), 1234);
    // The type init's difficulty 2 is the game's for init, population
    // and the AI.
    assert_eq!(info.difficulty, 2);
    assert_eq!(fx.sim.world.pop_info.difficulty, 2);
    assert_eq!(fx.sim.action.sys.hooks.ai_info.difficulty, 2);
    // The alignment value goes to its pending provider
    // and into the unit's state-105 list (stat 172),
    // which the monster's 0xAA sends.
    assert_eq!(fx.sim.action.sys.hooks.x.log, [format!("align {} 1", u.0)]);
    let st = &fx.sim.action.sys.stats;
    let l = st
        .unit_list(u)
        .and_then(|r| st.list_of_state(r, 105))
        .expect("state-105 list");
    assert_eq!(st.base(l, 172, 0), 1);
    assert!(st.has_state(u, 105));
}

#[test]
fn a_lent_world_spawns_like_population() {
    // `MonsterWorld::spawn_at` (a quest's spawn from inside a timer
    // event or tick hook, world lent): the same unit as population's own
    // `place_at` (seed draws, position, monster data); Jerhyn's unit seed
    // at the start of `act-travel-lut-ama` is the case.
    let run = |lent: bool| {
        let mut fx = Fx::new(isle_ds1s());
        let (a, _) = isle(&mut fx);
        fx.sim.create_regions();
        let u = if lent {
            let game = &mut fx.game;
            fx.sim.lend(|act| {
                let s = &mut act.sys;
                let mut sim = crate::units::hooks::Sim {
                    game,
                    units: &mut s.units,
                    stats: &mut s.stats,
                    data: &s.data,
                };
                s.hooks
                    .with_monster_world(|w, h| {
                        w.spawn_at(&mut sim, h, a, 40010, 40010, 0, 1, -1, 0)
                    })
                    .flatten()
                    .flatten()
            })
        } else {
            fx.sim.population(&mut fx.game, |cx| {
                crate::monsters::population::placement::place_at(
                    cx, a, None, 40010, 40010, 0, 1, -1, 0,
                )
                .unit()
            })
        }
        .expect("placed");
        fx.assert_clean();
        let r = fx.sim.action.sys.units.get(u).unwrap();
        let seed = (r.seed, r.init_seed);
        let pos = fx.sim.action.sys.hooks.path_position(u);
        let md = fx.sim.world.monsters.get(u).is_some();
        (seed, pos, md, fx.sim.action.sys.hooks.game_seed)
    };
    let direct = run(false);
    assert!(direct.2, "type init ran");
    assert_eq!(run(true), direct);
}

#[test]
fn death_counts_in_the_region_unless_flag_2_or_aligned() {
    use crate::wiring::action::MonsterWorld;
    let mut fx = Fx::new(isle_ds1s());
    let (a, _) = isle(&mut fx);
    fx.sim.create_regions();
    fx.sim
        .population(&mut fx.game, |cx| preset::place_presets(cx, a));
    let u = monsters(&fx)[0];
    let killed = |fx: &Fx| {
        fx.sim
            .world
            .pop
            .regions
            .get(ISLE as i32)
            .unwrap()
            .evil_killed
    };
    assert_eq!(killed(&fx), 0);
    // Summons and hirelings have non-hostile alignment.
    MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 2);
    assert_eq!(killed(&fx), 0);
    assert!(!fx.sim.world.monsters.get(u).unwrap().death_counted);
    fx.sim.world.monsters.entry(u).not_counted = true;
    MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 0);
    assert_eq!(killed(&fx), 0);
    assert!(!fx.sim.world.monsters.get(u).unwrap().death_counted);
    fx.sim.world.monsters.entry(u).not_counted = false;
    // An aligned (non-evil) monster does not count.
    MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 1);
    assert_eq!(killed(&fx), 0);
    MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 0);
    assert_eq!(killed(&fx), 1);
    // Repeated death notifications and a raised unit dying again do not count.
    MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 0);
    assert_eq!(killed(&fx), 1);
    // Flag 2 (not counted) does not count.
    fx.sim.world.monsters.entry(u).not_counted = true;
    MonsterWorld::<TestPending>::count_death(&mut fx.sim.world, u, 0);
    assert_eq!(killed(&fx), 1);
}

#[test]
fn creation_links_the_level_quest_chain() {
    let chains = |quest: u8| {
        let mut fx = Fx::with_tables(isle_ds1s(), |t| {
            t.pop.levels[ISLE as usize].quest = quest;
        });
        let (a, _) = isle(&mut fx);
        fx.sim.create_regions();
        fx.sim
            .population(&mut fx.game, |cx| preset::place_presets(cx, a));
        let u = monsters(&fx)[0];
        let want = format!("chain {} 1", u.0);
        fx.sim
            .action
            .sys
            .hooks
            .x
            .log
            .iter()
            .filter(|l| **l == want)
            .count()
    };
    assert_eq!(chains(1), 1);
    assert_eq!(chains(0), 0);
}
