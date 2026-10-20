// Spec: specs/sim/tick.md §3, §4; specs/missiles/missiles.md §R2–§R6; specs/combat/damage.md §5.2, §7.1, §7.2; specs/drlg/levels.md §5; specs/drlg/preset.md §3, §8, §9; specs/items/generation.md §3; specs/items/treasure.md §5; specs/sim/stat-lists.md §6, §8
//! Performance baselines of `d2-sim` (criterion; `docs/handoff/bench-baselines.md`).
//!
//! The game ticks at 25 Hz, so one tick has a 40 ms budget. Every case
//! runs on the synthetic fixtures of the unit tests (`bench_fixtures`,
//! no game files). Not run in CI; `cargo bench -p d2-sim`.

use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{Itemratio, Itemtypes, Record};
use d2_sim::bench_fixtures::{
    ds1, item_rec, item_tables, push_item, stat_data, Ds1s, Fx, ISLE, ISLE_DEF,
};
use d2_sim::items::{create_item, q, ty, ItemRequest};
use d2_sim::rng::Seed;
use d2_sim::stats::lists::{owner, NoHost};
use d2_sim::stats::{key, StatLists, ValueCallback};
use d2_sim::tick::events::event;
use d2_sim::treasure::runtime::{TcEntry, TreasureClass, FLAG_NOT_CLASSIC};
use d2_sim::treasure::walk::{
    walk, DropRequest, DropSink, Dropper, DropperKind, GameFacts, WalkArgs,
};
use d2_sim::treasure::{ItemData, TreasureClasses, TreasureData};
use d2_sim::units::{UnitId, UnitType};

/// Monsters placed by the synthetic DS1 of the tick bench.
const MONSTERS: usize = 24;

/// [`ISLE`]'s 40 × 18 DS1 with `MONSTERS` monsters of class 0, spread
/// over the two 8 × 8 rooms the level is built from.
fn crowded_ds1s() -> Ds1s {
    let spots: Vec<(u32, u32, u32)> = (0..MONSTERS as u32)
        .map(|i| (0, 2 + (i % 6) * 3, 2 + (i / 6) * 3))
        .collect();
    let mut m = BTreeMap::new();
    m.insert(
        format!("def{ISLE_DEF}.ds1").into_bytes(),
        ds1(40, 18, &spots),
    );
    Ds1s(m)
}

/// A game with the level generated and every room streamed.
fn level() -> Fx {
    let mut fx = Fx::new(crowded_ds1s());
    fx.sim.create_regions();
    let (_, rooms) = fx.generate(ISLE).expect("level");
    fx.stream(&rooms).expect("stream");
    fx
}

/// [`level`], first tick done (the room pass populated the rooms) and
/// every monster's think scheduled.
fn populated() -> Fx {
    let mut fx = level();
    d2_sim::tick::tick(&mut fx.game, &mut fx.sim);
    let monsters = fx.game.lists.units_of_type(UnitType::Monster);
    for (i, &u) in monsters.iter().enumerate() {
        fx.game
            .schedule_event(
                u,
                u32::from(event::AI_THINK),
                2 + (i as i32 % 20),
                None,
                0,
                0,
            )
            .expect("event");
    }
    fx
}

fn bench_tick(c: &mut Criterion) {
    let n = populated()
        .game
        .lists
        .units_of_type(UnitType::Monster)
        .len();
    println!("tick bench: {n} monsters");
    let mut g = c.benchmark_group("sim_tick");
    g.sample_size(20);
    // 200 frames: every monster thinks once (the idle think reschedules
    // itself 200 frames later). Divide by 200 for one tick.
    g.bench_function("populated_level_200_ticks", |b| {
        b.iter_batched(
            populated,
            |mut fx| {
                for _ in 0..200 {
                    d2_sim::tick::tick(&mut fx.game, &mut fx.sim);
                }
                fx
            },
            criterion::BatchSize::LargeInput,
        )
    });
    // Setup alone: the per-iteration cost the case above includes.
    g.bench_function("populated_setup_only", |b| b.iter(populated));
    g.finish();
}

/// The loaded tick (`docs/handoff/bench-fight.md`): 36 rows of a player
/// firing an arrow every 5 frames at a monster 18 sub-tiles away; every
/// monster dies on its 8th hit (death mode, experience, level-up check).
/// The first 60 ticks are the fight (the last kills land near frame 58);
/// 200 ticks add the steady load after it (122 missiles in flight
/// throughout; the corpses keep their collision bit, so later arrows
/// still hit them).
fn bench_fight(c: &mut Criterion) {
    use d2_sim::bench_fixtures::combat::{Fight, MAX_ROWS};
    let mut g = c.benchmark_group("sim_fight");
    g.sample_size(20);
    for ticks in [60, 200] {
        g.bench_function(format!("fight_36_rows_{ticks}_ticks"), |b| {
            b.iter_batched(
                || Fight::new(MAX_ROWS),
                |mut f| {
                    for _ in 0..ticks {
                        f.tick();
                    }
                    f
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }
    g.bench_function("fight_setup_only", |b| b.iter(|| Fight::new(MAX_ROWS)));
    g.finish();
}

fn bench_drlg(c: &mut Criterion) {
    let mut g = c.benchmark_group("drlg");
    g.sample_size(30);
    g.bench_function("create_act_0", |b| b.iter(|| Fx::new(crowded_ds1s())));
    g.bench_function("act_0_plus_generate_level", |b| {
        b.iter(|| {
            let mut fx = Fx::new(crowded_ds1s());
            black_box(fx.generate(ISLE).expect("level"))
        })
    });
    g.finish();
}

// ---- items -----------------------------------------------------------------

fn bench_items(c: &mut Criterion) {
    let mut t = item_tables();
    let mut r = item_rec(ty::TORS, b"qui ");
    r.durability = 24;
    r.minac = 3;
    r.maxac = 5;
    r.block = 7;
    r.speed = 5;
    let armor = push_item(&mut t, r);
    let mut r = item_rec(ty::HELM, b"cap ");
    r.gemsockets = 4;
    r.durability = 12;
    r.invwidth = 2;
    r.invheight = 2;
    let helm = push_item(&mut t, r);
    t.itemtypes[usize::from(ty::HELM)].maxsock40 = 4;
    let mut g = c.benchmark_group("items");
    g.bench_function("create_item_normal_armor_x100", |b| {
        b.iter(|| {
            let mut game = d2_sim::bench_fixtures::FakeGame::default();
            for i in 0..100 {
                let mut rq = ItemRequest {
                    item: if i % 2 == 0 { armor } else { helm } as i32,
                    format: 101,
                    ilvl: 30,
                    quality: q::NORMAL,
                    ..Default::default()
                };
                let stats = d2_sim::bench_fixtures::FakeStats::default();
                black_box(create_item(&t, &mut game, &mut rq, false, stats, 100).expect("item"));
            }
        })
    });
    g.finish();
}

// ---- treasure ----------------------------------------------------------------

fn tc(picks: i32, nodrop: i32, entries: &[(u16, i32, u8)]) -> TreasureClass {
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

#[derive(Default)]
struct Sink(u32);

impl DropSink for Sink {
    type Spot = ();
    type Item = u32;
    fn place(&mut self, _x: i32, _y: i32) -> Option<()> {
        Some(())
    }
    fn create(&mut self, _req: DropRequest<()>) -> Option<u32> {
        self.0 += 1;
        Some(self.0)
    }
    fn gold(&self, _item: u32) -> i32 {
        0
    }
    fn set_gold(&mut self, _item: u32, _value: i32) {}
}

fn bench_treasure(c: &mut Criterion) {
    const N: usize = 60;
    let items: Vec<ItemData> = (0..16u8)
        .map(|i| ItemData {
            code: [b'a' + i, b' ', b' ', b' '],
            ubercode: *b"xxx ",
            ultracode: *b"yyy ",
            version: 100,
            level: 1 + i,
            type_: 10,
            type2: 0,
            unique: 0,
            quest: 0,
            spawnable: 1,
        })
        .collect();
    let mut itemtypes: Vec<Itemtypes> = (0..N)
        .map(|_| Itemtypes::decode(&[0u8; Itemtypes::SIZE]))
        .collect();
    for t in &mut itemtypes {
        t.class = 0xFF;
        t.rarity = 3;
    }
    let words = N.div_ceil(32);
    let mut equiv = EquivMatrix {
        n: N,
        words,
        bits: vec![0; N * words],
    };
    for i in 0..N {
        equiv.bits[i * words] |= 1;
        if i > 0 {
            equiv.bits[i * words + i / 32] |= 1 << (i % 32);
        }
    }
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    (ratio.unique, ratio.uniquedivisor, ratio.uniquemin) = (400, 1, 6400);
    (ratio.set, ratio.setdivisor, ratio.setmin) = (160, 2, 5600);
    (ratio.rare, ratio.raredivisor, ratio.raremin) = (100, 2, 3200);
    (ratio.magic, ratio.magicdivisor, ratio.magicmin) = (34, 3, 192);
    (ratio.hiquality, ratio.hiqualitydivisor) = (12, 8);
    (ratio.normal, ratio.normaldivisor) = (2, 2);
    // TC 0 is the empty class; 1 = nested sub-class of items; 2 = root:
    // three picks over item entries and the sub-class (FLAG_TC = 1 << 0
    // is not exported; entries here are items only, plus one chain).
    let sub: Vec<(u16, i32, u8)> = (0..16).map(|i| (i, 1 + i as i32, 0)).collect();
    let tcs = TreasureClasses {
        tcs: vec![tc(1, 0, &[]), tc(4, 40, &sub)],
        group_offset: 0,
        chest: [None; 45],
        notes: Vec::new(),
    };
    let data = TreasureData {
        tcs: &tcs,
        items: &items,
        itemtypes: &itemtypes,
        equiv: &equiv,
        itemratio: std::slice::from_ref(&ratio),
    };
    let game = GameFacts {
        expansion: true,
        difficulty: 0,
        game_type: 0,
        living_players: 1,
        players_setting: 0,
        item_format: 101,
    };
    let dropper = Dropper {
        kind: DropperKind::Monster {
            class: 0,
            level: 30,
            playercount: 1,
        },
        x: 10,
        y: 20,
    };
    let args = WalkArgs {
        tc: Some(1),
        quality: 0,
        level: 30,
        find_item: false,
        list: false,
        max: 0,
    };
    let mut g = c.benchmark_group("treasure");
    g.bench_function("walk_x1000", |b| {
        b.iter(|| {
            let mut seed = Seed::init_low(4242);
            let mut sink = Sink::default();
            for _ in 0..1000 {
                black_box(walk(&data, &game, &dropper, &mut seed, None, &args, &mut sink).ok());
            }
            sink.0
        })
    });
    g.finish();
}

// ---- stat lists ----------------------------------------------------------------

fn bench_stats(c: &mut Criterion) {
    const ITEMS: u32 = 12;
    let data = stat_data();
    let p = UnitId(1);
    // The stats an item list carries (strength, dexterity, life, defense…).
    let stats: [(u16, i32); 6] = [(0, 5), (2, 4), (3, 7), (7, 640), (19, 3), (216, 2)];
    let mut g = c.benchmark_group("stat_lists");
    g.bench_function("equip_12_items_then_unit_totals_then_detach", |b| {
        b.iter(|| {
            let mut host = NoHost;
            let mut lists = StatLists::new(data.clone());
            let pl = lists.alloc_extended(
                &mut host,
                p,
                UnitType::Player,
                1,
                0,
                0,
                Some(ValueCallback::Server),
            );
            for (s, v) in [(0, 30), (12, 10), (3, 25), (7, 12800), (6, 12800)] {
                lists.set(&mut host, pl, s, v, 0, None);
            }
            let mut ids = Vec::new();
            for i in 0..ITEMS {
                let l =
                    lists.alloc_extended(&mut host, UnitId(100 + i), UnitType::Item, 7, 0, 0, None);
                for &(s, v) in &stats {
                    lists.set(&mut host, l, s, v + i as i32, 0, None);
                }
                lists.attach(&mut host, p, l, true);
                ids.push(l);
            }
            let mut sum = 0i64;
            for s in 0..20u16 {
                sum += i64::from(lists.unit_total(p, s, 0));
                sum += i64::from(lists.recompute(&mut host, pl, key(s, 0), Some(p)));
            }
            for l in ids {
                lists.detach(&mut host, l);
            }
            let _ = owner::PLAYER;
            sum
        })
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_tick,
    bench_fight,
    bench_drlg,
    bench_items,
    bench_treasure,
    bench_stats
);
criterion_main!(benches);
