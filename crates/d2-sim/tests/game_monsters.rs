//! Checks D2RUST monster initialization, population and AI APIs against
//! expected vectors and constraints over the player's extracted tables.
//! `D2_GAME_DIR` supplies the data through `game_common`; the tests are
//! ignored by default.
//!
//! Named monster constants identify rows paired with their `Id` in
//! `ai-functions.tsv` (`5 zombie1`, `19 fallen1`, ...).

mod game_common;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use d2_data::tables::{
    Difficultylevels, Levels, Monequip, Monlvl, Monprop, Monstats, Monstats2, Monumod, Superuniques,
};
use d2_sim::game::Game;
use d2_sim::monsters::ai::AI_TABLE;
use d2_sim::monsters::init::{
    self, boss_minions_and_init, champion_pack_member, choose_umods, component_counts, eligible,
    mark_boss, monstats_extra, monster_level, normal_mods, normal_mods_for, run_umod_init,
    stats_and_skills, stats_by_level, type_flag, type_init, unit_flag, Ctx, GameInfo, InitHost,
    InitTables, MonstatsExtra, MonsterData, MonsterStore, NamedIds, UMODS, UMODS_TSV,
};
use d2_sim::monsters::population::{self as pop, LevelPop, PopTables, Regions};
use d2_sim::rng::Seed;
use d2_sim::units::record::{UnitRecord, Units};
use d2_sim::units::{RoomId, UnitId, UnitType};

use game_common::{rows, tsv_rows};

// Monster rows (`ai-functions.tsv` pairs).
const SKELETON1: u32 = 0;
const ZOMBIE1: u32 = 5;
const BIGHEAD2: u32 = 11;
const FOULCROW1: u16 = 15;
const FALLEN1: u32 = 19;
const FALLEN2: u32 = 20;
const BRUTE2: u32 = 24;
const BRUTE1: u32 = 28;
const CORRUPTROGUE1: u32 = 43;
const CORRUPTROGUE3: u32 = 45;
const FALLENSHAMAN1: u32 = 58;
const QUILLRAT1: u32 = 63;
const CR_ARCHER1: u32 = 160;
const CR_LANCER1: u32 = 165;
const SKMAGE_POIS3: u32 = 276;
const GRISWOLD: u32 = 365;

/// Hireling classes (`init.md` §6 step 4).
const HIRELINGS: [u32; 5] = [271, 338, 359, 560, 561];

/// Blood Moor and Cold Plains level ids (`population.md` Test vectors).
const BLOOD_MOOR: i32 = 2;
const COLD_PLAINS: i32 = 3;

// ------------------------------------------------------------ tables

struct Tables {
    monstats: Vec<Monstats>,
    monstats2: Vec<Monstats2>,
    monlvl: Vec<Monlvl>,
    levels: Vec<Levels>,
    monprop: Vec<Monprop>,
    monequip: Vec<Monequip>,
    monumod: Vec<Monumod>,
    superuniques: Vec<Superuniques>,
    difficultylevels: Vec<Difficultylevels>,
    extra: Vec<MonstatsExtra>,
    components: Vec<[u8; 16]>,
    pop: PopTables,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let ms_bin = game_common::raw::<Monstats>();
        let ms2_bin = game_common::raw::<Monstats2>();
        let monstats: Vec<Monstats> = rows();
        let monstats2: Vec<Monstats2> = rows();
        let levels: Vec<Levels> = rows();
        let superuniques: Vec<Superuniques> = rows();
        let pop = PopTables::from_records(&levels, &monstats, &monstats2, &superuniques)
            .with_bins(&ms_bin, &ms2_bin);
        Tables {
            extra: monstats_extra(&ms_bin),
            components: component_counts(&ms2_bin),
            monstats,
            monstats2,
            monlvl: rows(),
            levels,
            monprop: rows(),
            monequip: rows(),
            monumod: rows(),
            superuniques,
            difficultylevels: rows(),
            pop,
        }
    })
}

fn ctx() -> Ctx<'static> {
    let t = tables();
    Ctx {
        tables: InitTables {
            monstats: &t.monstats,
            monstats2: &t.monstats2,
            monlvl: &t.monlvl,
            levels: &t.levels,
            monprop: &t.monprop,
            monequip: &t.monequip,
            monumod: &t.monumod,
            superuniques: &t.superuniques,
            difficultylevels: &t.difficultylevels,
            monstats_extra: &t.extra,
            components: &t.components,
            ids: NamedIds::default(),
        },
    }
}

fn s16(v: u16) -> i32 {
    i32::from(v as i16)
}

/// The monstats2 component counts of a class (via `MonStatsEx`).
fn counts_of(class: u32) -> [u8; 16] {
    let t = tables();
    let ex = usize::from(t.monstats[class as usize].monstatsex);
    t.components.get(ex).copied().unwrap_or([0; 16])
}

// ------------------------------------------------------------ host

/// An init host with real tables: stats in a map, every outward call
/// logged, boss minions created through the type init.
struct Host {
    cx: Ctx<'static>,
    game: Game,
    units: Units,
    store: MonsterStore,
    info: GameInfo,
    room: RoomId,
    stats: BTreeMap<(UnitId, u16), i32>,
    log: Vec<String>,
    level_id: i32,
    minions: BTreeMap<UnitId, Vec<UnitId>>,
    /// maxhp of each boss minion right after its type init.
    minion_base: BTreeMap<UnitId, i32>,
    next_seed: u32,
}

impl Host {
    /// Expansion single player (game type 3, L-flag 1), player count 1,
    /// Blood Moor.
    fn new(difficulty: u8) -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = game.lists.create_room(0).unwrap();
        Self {
            cx: ctx(),
            game,
            units: Units::new(),
            store: MonsterStore::new(),
            info: GameInfo {
                difficulty,
                expansion: true,
                game_type: 3,
                players: 1,
                ..GameInfo::default()
            },
            room,
            stats: BTreeMap::new(),
            log: Vec::new(),
            level_id: BLOOD_MOOR,
            minions: BTreeMap::new(),
            minion_base: BTreeMap::new(),
            next_seed: 0,
        }
    }

    /// A monster unit (mode neutral) with `seed`, before the type init.
    fn unit(&mut self, class: u32, seed: u32) -> UnitId {
        let u = self
            .game
            .spawn_unit(UnitType::Monster, Some(self.room), false)
            .unwrap();
        let mut r = UnitRecord::new(UnitType::Monster, class, u.0);
        r.seed = Seed::init_low(seed);
        r.mode = init::mode::NEUTRAL;
        self.units.insert(u, r);
        u
    }

    /// A monster after the type init.
    fn monster(&mut self, class: u32, seed: u32) -> UnitId {
        let u = self.unit(class, seed);
        let cx = self.cx;
        type_init(&cx, self, u);
        u
    }

    fn s(&self, u: UnitId, s: u16) -> i32 {
        self.stats.get(&(u, s)).copied().unwrap_or(0)
    }

    fn stats_of(&self, u: UnitId) -> BTreeMap<u16, i32> {
        self.stats
            .iter()
            .filter(|((v, _), _)| *v == u)
            .map(|((_, s), v)| (*s, *v))
            .collect()
    }

    fn data(&self, u: UnitId) -> MonsterData {
        self.store.get(u).cloned().unwrap_or_default()
    }

    fn set_flags(&mut self, u: UnitId, f: u16) {
        self.store.entry(u).type_flags |= f;
    }

    /// Resets a unit for another init: stats, monster data, log, seed.
    fn reset(&mut self, u: UnitId, seed: u32) {
        self.stats.retain(|(v, _), _| *v != u);
        self.store.remove(u);
        self.log.clear();
        let r = self.units.get_mut(u).unwrap();
        r.seed = Seed::init_low(seed);
        r.flags = 0;
    }

    fn calls(&self, prefix: &str) -> Vec<String> {
        self.log
            .iter()
            .filter(|l| l.starts_with(prefix))
            .cloned()
            .collect()
    }
}

impl InitHost for Host {
    fn game(&mut self) -> &mut Game {
        &mut self.game
    }
    fn units(&mut self) -> &mut Units {
        &mut self.units
    }
    fn monsters(&mut self) -> &mut MonsterStore {
        &mut self.store
    }
    fn info(&self) -> GameInfo {
        self.info
    }
    fn set_difficulty(&mut self, d: u8) {
        self.info.difficulty = d;
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.s(unit, stat)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.stats.insert((unit, stat), value);
    }
    fn alloc_ai(&mut self, unit: UnitId) {
        self.log.push(format!("alloc_ai {}", unit.0));
    }
    fn ai_install(&mut self, unit: UnitId, state: u32) {
        self.log.push(format!("ai_install {} {state}", unit.0));
    }
    fn level_id(&mut self, _: UnitId) -> i32 {
        self.level_id
    }
    fn attach_quest_chain(&mut self, unit: UnitId) {
        self.log.push(format!("quest_chain_attach {}", unit.0));
    }
    fn set_combat_mode(&mut self, unit: UnitId) {
        self.log.push(format!("combat_mode {}", unit.0));
    }
    fn give_skill(&mut self, _: UnitId, skill: u16, level: i32, mode: Option<u8>) {
        self.log.push(format!("skill {skill} {level} {mode:?}"));
    }
    fn apply_property(&mut self, _: UnitId, prop: i32, par: i32, min: i32, max: i32) {
        self.log.push(format!("prop {prop} {par} {min} {max}"));
    }
    fn create_equip_item(&mut self, _: UnitId, code: [u8; 4], loc: u8, m: u8, level: i32) {
        self.log.push(format!("item {code:?} {loc} {m} {level}"));
    }
    fn spawn_boss_minion(&mut self, boss: UnitId, class: u32, _: Option<u32>) -> Option<UnitId> {
        self.next_seed += 1;
        let m = self.monster(class, 1000 + self.next_seed);
        let hp = self.s(m, init::stat::MAXHP);
        self.minion_base.insert(m, hp);
        self.log.push(format!("minion {} of {}", m.0, boss.0));
        Some(m)
    }
    fn link_minion(&mut self, boss: UnitId, minion: UnitId) {
        self.minions.entry(boss).or_default().push(minion);
    }
    fn minions(&mut self, boss: UnitId) -> Vec<UnitId> {
        self.minions.get(&boss).cloned().unwrap_or_default()
    }
    fn give_aura(&mut self, _: UnitId, skill: u16, level: i32) {
        self.log.push(format!("aura {skill} {level}"));
    }
    fn set_ai_flag(&mut self, _: UnitId, flag: u16) {
        self.log.push(format!("ai_flag {flag}"));
    }
}

// ================================================================ init.md

/// The level `init.md` §7 gives, from the rule text.
fn want_level(t: &Tables, m: &Monstats, d: usize, level_id: i32) -> i32 {
    if d == 0 {
        return s16(m.level);
    }
    if !m.noratio && !m.boss {
        // Expansion game: the area level.
        let Some(l) = usize::try_from(level_id)
            .ok()
            .filter(|&i| i > 0)
            .and_then(|i| t.levels.get(i))
        else {
            return 1;
        };
        return i32::from([l.monlvl2ex, l.monlvl3ex][d - 1]);
    }
    s16([m.level_n, m.level_h][d - 1])
}

/// `(maxhp × R) >> 12`, or `(maxhp >> 12) × R` past the overflow bound
/// (`init.md` §6 step 10).
fn want_regen(maxhp: i32, r: u32) -> i32 {
    let r = r as i32;
    if r == 0 {
        return 0;
    }
    if maxhp > i32::MAX / r {
        (maxhp >> 12).wrapping_mul(r)
    } else {
        maxhp.wrapping_mul(r) >> 12
    }
}

/// Whole-table sweep: the stats and skills of every monstats row, at every
/// level id (and one past the table) and difficulty, in an expansion
/// single-player game. Must not panic; every stat §6 sets equals the rule.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn stats_and_skills_every_class_level_difficulty() {
    let t = tables();
    let cx = ctx();
    let mut h = Host::new(0);
    let level_ids = 0..=t.levels.len() as i32;
    for class in 0..t.monstats.len() as u32 {
        let m = &t.monstats[class as usize];
        // Only this class's unit is read below: keep the map small.
        h.stats.clear();
        let u = h.unit(class, 1);
        let counts = counts_of(class);
        let isatt = cx.monstats2(class).is_some_and(|m2| m2.isatt);
        for game_d in 0..3u8 {
            h.info.difficulty = game_d;
            let d = if HIRELINGS.contains(&class) {
                0
            } else {
                usize::from(game_d)
            };
            let bonus = t.difficultylevels[usize::from(game_d)].monsterskillbonus as i32;
            for level_id in level_ids.clone() {
                let ctx_s = format!("class {class} d {game_d} level id {level_id}");
                h.reset(u, class.wrapping_mul(7919).wrapping_add(level_id as u32));
                stats_and_skills(&cx, &mut h, u, level_id);
                // §7 level, §6 step 6 stats.
                let level = want_level(t, m, d, level_id);
                assert_eq!(h.s(u, init::stat::LEVEL), level, "{ctx_s}");
                let col = |a: [u16; 3]| s16(a[d]);
                for (s, want) in [
                    (init::stat::MONSTER_PLAYERCOUNT, 1),
                    (
                        init::stat::DAMAGERESIST,
                        col([m.resdm, m.resdm_n, m.resdm_h]),
                    ),
                    (
                        init::stat::MAGICRESIST,
                        col([m.resma, m.resma_n, m.resma_h]),
                    ),
                    (init::stat::FIRERESIST, col([m.resfi, m.resfi_n, m.resfi_h])),
                    (
                        init::stat::LIGHTRESIST,
                        col([m.resli, m.resli_n, m.resli_h]),
                    ),
                    (init::stat::COLDRESIST, col([m.resco, m.resco_n, m.resco_h])),
                    (
                        init::stat::POISONRESIST,
                        col([m.respo, m.respo_n, m.respo_h]),
                    ),
                    (
                        init::stat::TOBLOCK,
                        i32::from([m.toblock, m.toblock_n, m.toblock_h][d]),
                    ),
                    (init::stat::ATTACKRATE, 100),
                    (init::stat::VELOCITYPERCENT, 75),
                    (init::stat::OTHER_ANIMRATE, 100),
                    (init::stat::LAST_SENT_HP_PCT, 128),
                ] {
                    assert_eq!(h.s(u, s), want, "{ctx_s} stat {s}");
                }
                // §6 steps 7–10 (player count 1: no bonus).
                let b = stats_by_level(m, &t.monlvl, true, d, level);
                let maxhp = h.s(u, init::stat::MAXHP);
                assert_eq!(h.s(u, init::stat::HITPOINTS), maxhp, "{ctx_s}");
                assert_eq!(maxhp % 256, 0, "{ctx_s}");
                let cap = |v: i32| if v >= 0x80_0000 { init::HP_CAP } else { v };
                let hp = maxhp / 256;
                if b.max_hp >= b.min_hp {
                    let (a, z) = (cap(b.min_hp), cap(b.max_hp));
                    assert!(a.min(z) <= hp && hp <= a.max(z), "{ctx_s}: hp {hp} {b:?}");
                } else {
                    assert_eq!(hp, cap(b.min_hp), "{ctx_s}: no roll");
                }
                assert_eq!(h.s(u, init::stat::ARMORCLASS), b.ac, "{ctx_s}");
                assert_eq!(h.s(u, init::stat::EXPERIENCE), b.xp, "{ctx_s}");
                assert_eq!(
                    h.s(u, init::stat::HPREGEN),
                    want_regen(maxhp, m.damageregen),
                    "{ctx_s}"
                );
                // §10 step 2 (no region data here).
                let comps = h.data(u).components;
                for i in 0..16 {
                    let n = i32::from(counts[i]).max(1);
                    let n = if (class == 311 && i == 10) || (class == 312 && (10..12).contains(&i))
                    {
                        4
                    } else {
                        n
                    };
                    assert!(i32::from(comps[i]) < n, "{ctx_s}: component {i}");
                }
                // §6 step 14.
                let modes = t.extra[class as usize].skill_modes;
                let want: Vec<String> = [
                    (m.skill1, m.sk1lvl),
                    (m.skill2, m.sk2lvl),
                    (m.skill3, m.sk3lvl),
                    (m.skill4, m.sk4lvl),
                    (m.skill5, m.sk5lvl),
                    (m.skill6, m.sk6lvl),
                    (m.skill7, m.sk7lvl),
                    (m.skill8, m.sk8lvl),
                ]
                .into_iter()
                .zip(modes)
                .filter(|((s, l), _)| s16(*s) >= 0 && *l > 0)
                .map(|((s, l), md)| {
                    format!(
                        "skill {s} {} {:?}",
                        i32::from(l) + bonus,
                        u8::try_from(md).ok()
                    )
                })
                .collect();
                assert_eq!(h.calls("skill "), want, "{ctx_s}");
                // §6 step 15.
                let f = h.units.get(u).unwrap().flags;
                assert_eq!(f & unit_flag::IS_ATT != 0, isatt, "{ctx_s}");
                assert_eq!(f & unit_flag::PET_IGNORE != 0, m.petignore, "{ctx_s}");
            }
        }
    }
}

/// Whole-table sweep of the type init itself (§5) for every row and
/// difficulty, in a classic and an expansion game: no panic, and the
/// steps §5 names happen.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn type_init_every_class() {
    let t = tables();
    for expansion in [true, false] {
        for d in 0..3u8 {
            let mut h = Host::new(d);
            h.info.expansion = expansion;
            h.info.game_type = u8::from(expansion) * 3;
            for class in 0..t.monstats.len() as u32 {
                h.log.clear();
                let u = h.monster(class, class + 1);
                let c = format!("class {class} d {d} expansion {expansion}");
                assert_eq!(h.data(u).class, class, "{c}");
                assert_eq!(h.data(u).level_id, BLOOD_MOOR, "{c}");
                let f = h.units.get(u).unwrap().flags;
                assert_eq!(f & unit_flag::AT_INIT, unit_flag::AT_INIT, "{c}");
                assert_eq!(h.calls("alloc_ai"), [format!("alloc_ai {}", u.0)], "{c}");
                assert_eq!(
                    h.calls("ai_install"),
                    [format!("ai_install {} 0", u.0)],
                    "{c}"
                );
                assert_eq!(
                    h.calls("quest_chain_attach"),
                    [format!("quest_chain_attach {}", u.0)],
                    "{c}"
                );
                assert!(h.calls("combat_mode").is_empty(), "{c}");
                // §6 step 8: maxhp = hitpoints. §13 / Edge cases 11: in a
                // classic game at d > 0 (hirelings have d = 0, §6 step 4)
                // with `Align` != 1, maxhp alone is scaled by (1, 2) for
                // both NM and Hell; hitpoints keep the unscaled value.
                let m = &t.monstats[class as usize];
                let d_eff = if HIRELINGS.contains(&class) { 0 } else { d };
                let hp = h.s(u, init::stat::HITPOINTS);
                let want_max = if !expansion && d_eff > 0 && m.align != 1 {
                    init::pct(hp, 1, 2)
                } else {
                    hp
                };
                assert_eq!(h.s(u, init::stat::MAXHP), want_max, "{c}");
            }
        }
    }
}

/// Checks six Act I monster classes' resistances at each difficulty.
/// Each class is expected to keep the same ToBlock value across all three
/// difficulties; resistance vectors vary by difficulty.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_resistances_and_block() {
    // (class, [Normal, NM, Hell] of (Dm, Ma, Fi, Li, Co, Po), block).
    type Res = [i32; 6];
    let rows: [(u32, [Res; 3], i32); 6] = [
        (
            ZOMBIE1,
            [
                [0, 0, 0, 0, 0, 50],
                [0, 0, 0, 0, 0, 75],
                [50, 0, 0, 0, 120, 75],
            ],
            3,
        ),
        (FALLEN1, [[0; 6], [0; 6], [15, 0, 100, 0, 40, 0]], 9),
        (BRUTE1, [[0; 6], [0; 6], [50, 0, 0, 0, 100, 0]], 4),
        (
            FALLENSHAMAN1,
            [
                [0, 0, 25, 0, 0, 0],
                [0, 0, 50, 0, 0, 0],
                [15, 0, 100, 0, 0, 0],
            ],
            4,
        ),
        (QUILLRAT1, [[0; 6], [0; 6], [50, 0, 0, 0, 50, 0]], 3),
        (CR_LANCER1, [[0; 6], [0; 6], [45, 0, 25, 100, 25, 25]], 4),
    ];
    for (class, res, block) in rows {
        for d in 0..3u8 {
            let mut h = Host::new(d);
            let u = h.monster(class, 1);
            let got = [
                init::stat::DAMAGERESIST,
                init::stat::MAGICRESIST,
                init::stat::FIRERESIST,
                init::stat::LIGHTRESIST,
                init::stat::COLDRESIST,
                init::stat::POISONRESIST,
            ]
            .map(|s| h.s(u, s));
            assert_eq!(got, res[usize::from(d)], "class {class} d {d}");
            assert_eq!(h.s(u, init::stat::TOBLOCK), block, "class {class} d {d}");
        }
    }
}

/// The live constants `init.md` states: monumod `constants` K (§19),
/// difficultylevels `MonsterSkillBonus` 0 / 3 / 7 (§9 step 3) and
/// `ChampionDamageBonus` 90 / 75 / 66 (§19), monumod row 0 = 20 (§17
/// step 1); the live champion candidates 16, 36–39 with weight 1, 36–39
/// at `version` 100 (§17.1).
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_umod_constants() {
    let t = tables();
    const K: [u32; 34] = [
        20, 100, 75, 50, 200, 150, 100, 300, 200, 100, 75, 100, 50, 100, 75, 150, 0, 33, 33, 0, 50,
        50, 33, 33, 33, 50, 50, 50, 66, 66, 66, 100, 100, 100,
    ];
    let got: Vec<u32> = t.monumod.iter().take(34).map(|r| r.constants).collect();
    assert_eq!(got, K);
    let dl = |f: fn(&Difficultylevels) -> u32| -> Vec<u32> {
        t.difficultylevels.iter().take(3).map(f).collect()
    };
    assert_eq!(dl(|r| r.monsterskillbonus), [0, 3, 7]);
    assert_eq!(dl(|r| r.championdamagebonus), [90, 75, 66]);
    let cands: Vec<(usize, [u16; 3], u16)> = t
        .monumod
        .iter()
        .enumerate()
        .filter(|(_, r)| r.champion != 0 && [r.cpick, r.cpick_n, r.cpick_h].iter().any(|&w| w > 0))
        .map(|(i, r)| (i, [r.cpick, r.cpick_n, r.cpick_h], r.version))
        .collect();
    assert_eq!(
        cands,
        [
            (16, [1; 3], cands[0].2),
            (36, [1; 3], 100),
            (37, [1; 3], 100),
            (38, [1; 3], 100),
            (39, [1; 3], 100),
        ]
    );
    assert!(cands[0].2 < 100, "umod 16 is not expansion-only");
}

/// brute1's BaseId is brute2 (24), whose normal mod is rage (13).
/// For each tested seed, normal mods produce umods [13] and no type flags.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_brute1_normal_mods() {
    let t = tables();
    assert_eq!(u32::from(t.monstats[BRUTE1 as usize].baseid), BRUTE2);
    assert_eq!(
        normal_mods_for(t.monstats[BRUTE1 as usize].baseid),
        [(13, false)]
    );
    let cx = ctx();
    for seed in [1, 12345, 3_735_928_559] {
        let mut h = Host::new(0);
        let u = h.monster(BRUTE1, seed);
        normal_mods(&cx, &mut h, u);
        assert_eq!(h.data(u).umod_list(), [13]);
        assert_eq!(h.data(u).type_flags, 0);
    }
}

/// For three brute1 seeds, marking a champion member produces umods
/// [13, 16] and champion, unique and boss flags. Repeating the call adds
/// no modifiers or minions.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_brute1_champion_pack() {
    let cx = ctx();
    let mut h = Host::new(0);
    for seed in [56351, 54141, 54177] {
        let u = h.monster(BRUTE1, seed);
        normal_mods(&cx, &mut h, u);
        champion_pack_member(&cx, &mut h, u, 16);
        let d = h.data(u);
        assert_eq!(d.umod_list(), [13, 16]);
        assert!(d.has_flag(type_flag::CHAMPION) && d.has_flag(type_flag::UNIQUE));
        assert!(d.has_flag(type_flag::BOSS));
        // A second call does nothing (type flag 4 already set).
        champion_pack_member(&cx, &mut h, u, 16);
        assert_eq!(h.data(u).umod_list(), [13, 16]);
        assert!(h.minions(u).is_empty());
    }
}

/// A random boss after its umods are chosen (`init.md` §16.1 steps 2–3):
/// mark boss, flags, the given umods, then §18.
fn boss(h: &mut Host, class: u32, seed: u32, champion: bool, umods: &[u8]) -> UnitId {
    let cx = ctx();
    let u = h.monster(class, seed);
    mark_boss(h, u);
    if champion {
        h.set_flags(u, type_flag::CHAMPION);
    }
    for &m in umods {
        h.store.entry(u).push_umod(m);
    }
    boss_minions_and_init(&cx, h, u, 3, 6, None, true);
    u
}

/// Champion fallenshaman1, Normal, Blood Moor (`init.md` "Bosses"):
/// umods [16] and the ghostly [36].
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_champion_fallenshaman1() {
    for seed in [1, 8013, 62586, 3_735_928_559] {
        for ghostly in [false, true] {
            let mut h = Host::new(0);
            let plain = h.monster(FALLENSHAMAN1, seed);
            let base = h.s(plain, init::stat::MAXHP);
            let u = boss(
                &mut h,
                FALLENSHAMAN1,
                seed,
                true,
                &[if ghostly { 36 } else { 16 }],
            );
            let c = format!("seed {seed} ghostly {ghostly}");
            assert!((5..=9).contains(&(base / 256)), "{c}");
            assert_eq!(h.s(u, init::stat::LEVEL), 4, "{c}");
            // Same seed, same base roll.
            assert_eq!(h.s(u, init::stat::MAXHP), 3 * base, "{c}");
            assert_eq!(h.s(u, init::stat::HITPOINTS), 3 * base, "{c}");
            assert_eq!(h.s(u, init::stat::HPREGEN), 0, "{c}");
            assert_eq!(h.s(u, init::stat::EXPERIENCE), 96, "{c}");
            assert_eq!(h.s(u, init::stat::DAMAGEPERCENT), 90, "{c}");
            assert_eq!(h.s(u, init::stat::ITEM_TOHIT_PERCENT), 67, "{c}");
            let d = h.data(u);
            assert!(d.has_flag(type_flag::BOSS | type_flag::CHAMPION), "{c}");
            assert_eq!(
                d.type_flags & (type_flag::BOSS | type_flag::CHAMPION | type_flag::UNIQUE),
                type_flag::BOSS | type_flag::CHAMPION | type_flag::UNIQUE,
                "{c}"
            );
            assert!(h.minions(u).is_empty(), "{c}");
            if ghostly {
                assert!(d.has_flag(type_flag::GHOSTLY), "{c}");
                assert_eq!(h.s(u, init::stat::DAMAGERESIST), 80, "{c}");
                assert_eq!(h.s(u, init::stat::VELOCITYPERCENT), 75, "{c}");
                assert_eq!(h.s(u, init::stat::COLDMINDAM), 1, "{c}");
                assert_eq!(h.s(u, init::stat::COLDMAXDAM), 2, "{c}");
                assert_eq!(h.s(u, init::stat::COLDLENGTH), 150, "{c}");
            } else {
                assert!(!d.has_flag(type_flag::GHOSTLY), "{c}");
                assert_eq!(h.s(u, init::stat::VELOCITYPERCENT), 95, "{c}");
            }
        }
    }
}

/// Unique fallen1, Normal: level 4, maxhp 4 × base, experience 90, one
/// unique umod, 3..6 fallen1 minions with HP × 2, level 4, experience × 5
/// (`init.md` "Bosses").
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_unique_fallen1() {
    let cx = ctx();
    for seed in [1, 2, 3, 12345, 3_735_928_559, 4_014_346_870] {
        let mut h = Host::new(0);
        let plain = h.monster(FALLEN1, seed);
        let base = h.s(plain, init::stat::MAXHP);
        assert!((1..=4).contains(&(base / 256)), "seed {seed}");
        let u = h.monster(FALLEN1, seed);
        mark_boss(&mut h, u);
        choose_umods(&cx, &mut h, u, false);
        assert_eq!(h.data(u).umod_count(), 1, "seed {seed}");
        boss_minions_and_init(&cx, &mut h, u, 3, 6, None, true);
        assert_eq!(h.s(u, init::stat::LEVEL), 4, "seed {seed}");
        assert_eq!(h.s(u, init::stat::MAXHP), 4 * base, "seed {seed}");
        assert_eq!(h.s(u, init::stat::EXPERIENCE), 90, "seed {seed}");
        let minions = h.minions(u);
        assert!((3..=6).contains(&minions.len()), "seed {seed}");
        for m in minions {
            assert_eq!(h.units.get(m).unwrap().class, FALLEN1);
            let hp = h.stats_of(m);
            assert!(h.data(m).has_flag(type_flag::MINION));
            assert_eq!(hp[&init::stat::LEVEL], 4, "seed {seed}");
            assert_eq!(hp[&init::stat::EXPERIENCE], 90, "seed {seed}");
            let mb = h.minion_base[&m];
            assert!((1..=4).contains(&(mb / 256)), "seed {seed}");
            assert_eq!(hp[&init::stat::MAXHP], 2 * mb, "seed {seed}");
        }
    }
}

/// HP factors NM / Hell (`init.md` "Bosses"): champion × 2.5 / × 2,
/// unique × 3 / × 2, boss minion × 1.75 / × 1.5.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_boss_hp_factors() {
    // (difficulty, champion ×4, unique ×4, minion ×4).
    for (d, champ, uniq, minion) in [(1u8, 10, 12, 7), (2, 8, 8, 6)] {
        for seed in [1, 12345, 3_735_928_559] {
            let mut h = Host::new(d);
            let plain = h.monster(FALLENSHAMAN1, seed);
            let base = h.s(plain, init::stat::MAXHP);
            let c = boss(&mut h, FALLENSHAMAN1, seed, true, &[16]);
            assert_eq!(
                4 * h.s(c, init::stat::MAXHP),
                champ * base,
                "d {d} champion"
            );
            let plain = h.monster(FALLEN1, seed);
            let base = h.s(plain, init::stat::MAXHP);
            let u = boss(&mut h, FALLEN1, seed, false, &[]);
            assert_eq!(4 * h.s(u, init::stat::MAXHP), uniq * base, "d {d} unique");
            let minions = h.minions(u);
            assert!((3..=6).contains(&minions.len()), "d {d}");
            for m in minions {
                let hp = h.s(m, init::stat::MAXHP);
                assert_eq!(4 * hp, minion * h.minion_base[&m], "d {d} minion");
            }
        }
    }
}

/// Every umod init function (`umods.tsv`) on every monstats row, unique and
/// not, at every difficulty: no panic; an umod whose `init_fn` is `-` does
/// nothing at init, and a `unique_gate` = yes umod does nothing when called
/// with unique = 0.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_umod_on_every_class() {
    let t = tables();
    let cx = ctx();
    let tsv = tsv_rows(UMODS_TSV);
    assert_eq!(tsv.len(), UMODS.len());
    for d in 0..3u8 {
        let mut h = Host::new(d);
        for class in 0..t.monstats.len() as u32 {
            h.stats.clear();
            h.store = MonsterStore::new();
            let u = h.monster(class, class + 7);
            let stats = h.stats_of(u);
            let data = h.data(u);
            for row in &tsv {
                let id: u8 = row[0].parse().unwrap();
                for unique in [true, false] {
                    h.stats.retain(|(v, _), _| *v != u);
                    for (&s, &v) in &stats {
                        h.stats.insert((u, s), v);
                    }
                    *h.store.entry(u) = data.clone();
                    h.log.clear();
                    run_umod_init(&cx, &mut h, u, id, unique);
                    let inert = row[2] == "-" || (!unique && row[3] == "yes");
                    if inert {
                        let c = format!("umod {id} class {class} d {d} unique {unique}");
                        assert_eq!(h.stats_of(u), stats, "{c}");
                        assert_eq!(h.data(u), data, "{c}");
                        assert!(h.log.is_empty(), "{c}: {:?}", h.log);
                    }
                }
            }
        }
    }
}

/// Every class through the umod choice (§17) at every difficulty, with and
/// without the champion chance: no panic; a champion gets one umod from
/// the live candidates (16, 36–39), a unique gets at most 1 + d umods, none
/// repeated, each with `upick` > 0, `champion` = 0 and eligible (§17.3).
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn choose_umods_every_class() {
    let t = tables();
    let cx = ctx();
    for d in 0..3u8 {
        let mut h = Host::new(d);
        for class in 0..t.monstats.len() as u32 {
            for id in 0..t.monumod.len() + 2 {
                eligible(&cx, &mut h, class, id);
            }
            for (i, champion_allowed) in [(0u32, true), (1, true), (2, false), (3, false)] {
                let u = h.monster(class, class * 4 + i + 1);
                choose_umods(&cx, &mut h, u, champion_allowed);
                let data = h.data(u);
                let got = data.umod_list().to_vec();
                let c = format!("class {class} d {d} {got:?}");
                if data.has_flag(type_flag::CHAMPION) {
                    assert!(champion_allowed, "{c}");
                    assert!(got.len() <= 1, "{c}");
                    assert!(got.iter().all(|u| [16, 36, 37, 38, 39].contains(u)), "{c}");
                } else {
                    assert!(got.len() <= 1 + usize::from(d), "{c}");
                    let mut sorted = got.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    assert_eq!(sorted.len(), got.len(), "{c}");
                    for &m in &got {
                        let r = &t.monumod[usize::from(m)];
                        let w = [r.upick, r.upick_n, r.upick_h][usize::from(d)];
                        assert!(w > 0 && r.champion == 0, "{c}");
                        assert!(eligible(&cx, &mut h, class, usize::from(m)), "{c}");
                        assert!(usize::from(m) < UMODS.len(), "{c}");
                    }
                }
            }
        }
    }
}

/// Checks monstats2 component counts: zombie1 starts with
/// [3,3,3,3,3,0,0,0,3,3,3, ...]; brute1 and quillrat1 have no count above 1.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_component_counts() {
    assert_eq!(counts_of(ZOMBIE1)[..11], [3, 3, 3, 3, 3, 0, 0, 0, 3, 3, 3]);
    for class in [BRUTE1, QUILLRAT1] {
        assert!(counts_of(class).iter().all(|&n| n <= 1), "class {class}");
        let mut h = Host::new(0);
        let u = h.monster(class, 5);
        assert_eq!(h.data(u).components, [0; 16], "class {class}");
    }
}

/// `monster_level` and `stats_by_level` over every row, level id (and past
/// the table), difficulty and L-flag: no panic; levels past the monlvl
/// table clamp (§8.1).
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn level_stats_every_row() {
    let t = tables();
    for m in &t.monstats {
        for expansion in [true, false] {
            let info = GameInfo {
                expansion,
                game_type: 3,
                ..GameInfo::default()
            };
            for d in 0..3 {
                for level_id in -1..=t.levels.len() as i32 {
                    let l = monster_level(m, &t.levels, &info, d, level_id);
                    let out = level_id <= 0 || level_id >= t.levels.len() as i32;
                    if d > 0 && expansion && !m.noratio && !m.boss && out {
                        assert_eq!(l, 1);
                    }
                    for l_flag in [true, false] {
                        stats_by_level(m, &t.monlvl, l_flag, d, l);
                    }
                }
            }
            let last = t.monlvl.len() as i32 - 1;
            for l_flag in [true, false] {
                assert_eq!(
                    stats_by_level(m, &t.monlvl, l_flag, 0, last + 50),
                    stats_by_level(m, &t.monlvl, l_flag, 0, last)
                );
            }
        }
    }
}

// ================================================================ ai.md

/// Every monstats row's `AI` is an index of the AI table (148 records), and
/// the per-index row counts equal `ai-functions.tsv` `monstats_rows`; the
/// rows it names use that index (`ai.md` §10).
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn ai_index_of_every_row() {
    let t = tables();
    let mut count = vec![0usize; AI_TABLE.len()];
    for (row, m) in t.monstats.iter().enumerate() {
        let ai = usize::from(m.ai);
        assert!(ai < AI_TABLE.len(), "row {row}: AI {ai}");
        count[ai] += 1;
    }
    let tsv = tsv_rows(d2_sim::monsters::ai::table::AI_FUNCTIONS_TSV);
    assert_eq!(tsv.len(), AI_TABLE.len());
    // Gather all count and row mismatches before failing so the report
    // includes every difference from the AI catalogue.
    let mut bad_counts = Vec::new();
    let mut bad_rows = Vec::new();
    for row in tsv {
        let index: usize = row[0].parse().unwrap();
        let mut parts = row[7].splitn(2, ':');
        let n: usize = parts.next().unwrap().trim().parse().unwrap();
        if count[index] != n {
            bad_counts.push(format!("AI {index} ({}): {} != {n}", row[1], count[index]));
        }
        let pairs: Vec<&str> = parts.next().unwrap_or("").split_whitespace().collect();
        for pair in pairs.chunks(2) {
            let r: usize = pair[0].parse().unwrap();
            let live = usize::from(t.monstats[r].ai);
            if live != index {
                bad_rows.push(format!(
                    "row {r} {}: live AI {live}, tsv AI {index}",
                    pair[1]
                ));
            }
        }
    }
    for s in bad_counts.iter().chain(&bad_rows) {
        eprintln!("{s}");
    }
    assert!(
        bad_counts.is_empty(),
        "{} AI counts differ",
        bad_counts.len()
    );
    assert!(bad_rows.is_empty(), "{} row pairs differ", bad_rows.len());
}

/// The selected classes' AI index, Normal aip1..aip5 and `aidel` (`ai-bodies.md`
/// §9.1).
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_recorded_ai_params() {
    let t = tables();
    let rows: [(u32, u16, Option<[i32; 5]>); 14] = [
        (ZOMBIE1, 3, Some([30, 10, 0, 20, 0])),
        (FALLEN1, 6, Some([30, 10, 50, 20, 0])),
        (BRUTE1, 7, Some([0, 0, 100, 45, 0])),
        (FALLENSHAMAN1, 13, Some([45, 60, 100, 24, 15])),
        (QUILLRAT1, 14, Some([10, 35, 0, 2, 0])),
        (CR_LANCER1, 36, Some([60, 75, 9, 0, 15])),
        (179, 1, None),
        (152, 1, None),
        (147, 32, None),
        (148, 32, None),
        (150, 32, None),
        (154, 32, None),
        (155, 32, None),
        (266, 58, None),
    ];
    for (class, ai, aip) in rows {
        let m = &t.monstats[class as usize];
        assert_eq!(m.ai, ai, "row {class}");
        assert_eq!(m.aidel, 15, "row {class}");
        if let Some(aip) = aip {
            assert_eq!(
                [m.aip1, m.aip2, m.aip3, m.aip4, m.aip5].map(s16),
                aip,
                "row {class}"
            );
        }
    }
    for i in [3, 6, 7, 13, 14, 36] {
        assert_eq!(AI_TABLE[i].target_mode, 1, "AI {i}");
    }
    for i in [1, 32, 58] {
        assert_eq!(AI_TABLE[i].target_mode, 0, "AI {i}");
    }
}

// ================================================================ population.md

fn level(id: i32) -> &'static LevelPop {
    tables().pop.level(id).unwrap()
}

/// levels.txt values (`population.md` Real): Blood Moor, Cold Plains, Den
/// of Evil, Crypt / Mausoleum, the Act 1 town, Act 1 `WarpDist`. Where the
/// spec gives one MonDen without "×3", only Normal is checked.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_levels_rows() {
    let bm = level(BLOOD_MOOR);
    assert_eq!(bm.mon_den, [520; 3]);
    assert_eq!((bm.mon_umin, bm.mon_umax), ([0, 4, 7], [0, 5, 9]));
    assert_eq!(bm.mon_wndr, 1);
    assert_eq!(bm.num_mon, 3);
    let bm_list = [ZOMBIE1 as i16, FALLEN1 as i16, QUILLRAT1 as i16];
    assert_eq!(
        (&bm.mon[..], &bm.nmon[..], &bm.umon[..]),
        (&bm_list[..], &bm_list[..], &bm_list[..])
    );
    let cp = level(COLD_PLAINS);
    assert_eq!(cp.mon_den, [520; 3]);
    assert_eq!((cp.mon_umin, cp.mon_umax), ([1, 4, 7], [1, 5, 9]));
    assert_eq!(cp.num_mon, 3);
    assert_eq!(
        cp.mon,
        [BRUTE1, CORRUPTROGUE1, FALLENSHAMAN1, CR_LANCER1].map(|c| c as i16)
    );
    let den = level(8);
    assert_eq!(den.mon_den[0], 600);
    assert_eq!((den.mon_umin, den.mon_umax), ([0; 3], [0; 3]));
    assert_eq!((den.quest, den.mon_wndr), (1, 0));
    assert_eq!(level(18).mon_den[0], 1056);
    assert_eq!(level(19).mon_den[0], 1056);
    assert_eq!(level(1).mon_den[0], 0);
    let t = tables();
    let mut act1 = 0;
    let mut other = Vec::new();
    for (id, l) in t.pop.levels.iter().enumerate().skip(1) {
        if l.act == 0 {
            act1 += 1;
            if l.warp_dist != 2025 {
                other.push((id, l.warp_dist));
            }
        }
    }
    assert!(act1 > 0);
    // §8 / Real: Act I WarpDist is 2025 except level 15 (3800) and levels
    // 20, 21, 23, 25 (100) (live values, PC 2 local run C11).
    assert_eq!(
        other,
        [(15, 3800), (20, 100), (21, 100), (23, 100), (25, 100)],
        "act 0 WarpDist != 2025"
    );
}

/// monstats.txt values (`population.md` Real): Rarity, groups, parties,
/// the placespawn and sparsePopulate rows; superuniques 0–9 (§11.4);
/// monumod row 0 = 20.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_monstats_population_rows() {
    let t = tables();
    let p = &t.pop;
    for (class, rarity) in [
        (ZOMBIE1, 2),
        (FALLEN1, 2),
        (QUILLRAT1, 2),
        (BRUTE1, 1),
        (CR_LANCER1, 1),
        (CORRUPTROGUE1, 2),
        (FALLENSHAMAN1, 2),
    ] {
        assert_eq!(p.mon(class as i32).unwrap().rarity, rarity, "class {class}");
    }
    for (class, min, max) in [
        (ZOMBIE1, 1, 2),
        (FALLEN1, 2, 3),
        (QUILLRAT1, 1, 2),
        (BRUTE1, 1, 1),
        (FALLENSHAMAN1, 1, 1),
        (CR_LANCER1, 1, 2),
        (CORRUPTROGUE1, 2, 3),
    ] {
        let m = p.mon(class as i32).unwrap();
        assert_eq!((m.min_grp, m.max_grp), (min, max), "class {class}");
    }
    let f = p.mon(FALLEN1 as i32).unwrap();
    assert_eq!(
        (f.minion1, f.party_min, f.party_max, f.set_boss, f.boss_xfer),
        (FALLEN1 as i16, 2, 3, true, true)
    );
    let s = p.mon(FALLENSHAMAN1 as i32).unwrap();
    assert_eq!(
        (s.minion1, s.party_min, s.party_max, s.set_boss),
        (FALLEN1 as i16, 2, 6, true)
    );
    let placespawn: Vec<usize> = (0..p.monstats.len())
        .filter(|&i| p.monstats[i].place_spawn)
        .collect();
    assert_eq!(placespawn, [206, 207, 208, 209]);
    assert_eq!(p.monstats[206].spawn, FOULCROW1 as i16);
    // The spec names the row 528 here and `ai-functions.tsv` names evilhut
    // 529: only the single row and its value are checked (see the note).
    let sparse: Vec<u8> = p
        .monstats
        .iter()
        .map(|m| m.sparse_populate)
        .filter(|&v| v != 0)
        .collect();
    assert_eq!(sparse, [40]);
    assert_eq!(t.monumod[0].constants, 20);
    // Superuniques 0–9: (class, MinGrp, MaxGrp); AutoPos of 0 and 3.
    let su: [(Option<u32>, u32); 10] = [
        (Some(FALLENSHAMAN1), 2),
        (Some(SKELETON1), 5),
        (Some(CR_ARCHER1), 4),
        (Some(FALLEN2), 8),
        (Some(BRUTE2), 2),
        (Some(GRISWOLD), 0),
        (Some(CORRUPTROGUE3), 6),
        (Some(BIGHEAD2), 4),
        (None, 6),
        (Some(SKMAGE_POIS3), 0),
    ];
    for (i, (class, grp)) in su.into_iter().enumerate() {
        let r = &p.superuniques[i];
        if let Some(c) = class {
            assert_eq!(r.class, c as i32, "superunique {i}");
        }
        assert_eq!((r.min_grp, r.max_grp), (grp, grp), "superunique {i}");
    }
    assert_eq!(p.superuniques[0].auto_pos, 1);
    assert_eq!(p.superuniques[3].auto_pos, 0);
}

/// `population.md` §2.3 step 3: no level list holds a non-`isSpawn` class
/// except level 120; `rangedspawn` only on Act 5 levels 110–119, 123–131,
/// 135; no listed class has `Rarity` above 2.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_level_list_facts() {
    let p = &tables().pop;
    for (id, l) in p.levels.iter().enumerate() {
        for &c in l.mon.iter().chain(&l.nmon) {
            let m = p
                .mon(i32::from(c))
                .unwrap_or_else(|| panic!("level {id} class {c}"));
            if id != 120 {
                assert!(m.is_spawn, "level {id} class {c}");
            }
            assert!(m.rarity <= 2, "level {id} class {c}");
        }
        let ranged = (110..=119).contains(&id) || (123..=131).contains(&id) || id == 135;
        assert_eq!(l.ranged_spawn != 0, ranged, "level {id}");
    }
}

/// Region creation (§2.1–§2.4) on the live tables for every difficulty and
/// both game kinds, several game seeds: no panic; every record field §2.2
/// copies equals its levels column; the list holds at most min(NumMon, 13,
/// list count) entries, each from the difficulty's list and `isSpawn`; the
/// rarity total is their sum; variants stay within the component counts.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn regions_every_level() {
    let p = &tables().pop;
    for expansion in [true, false] {
        for difficulty in 0..3u8 {
            for lo in [1, 12345, 4_014_346_869] {
                let info = pop::GameInfo {
                    difficulty,
                    expansion,
                };
                let mut seed = Seed::init_low(lo);
                let mut want_seed = Seed::init_low(lo);
                let (regions, mon_seed) = Regions::create(p, info, &mut seed);
                assert_eq!(mon_seed, want_seed.step());
                assert_eq!(seed, want_seed, "one game-seed step");
                assert!(regions.get(0).is_none());
                assert_eq!(regions.slots.len(), p.levels.len());
                let d = usize::from(difficulty);
                for id in 1..p.levels.len() as i32 {
                    let c = format!("level {id} d {d} expansion {expansion} seed {lo}");
                    let r = regions.get(id).unwrap_or_else(|| panic!("{c}"));
                    let l = level(id);
                    let lvl = i32::from(if expansion {
                        l.mon_lvl_ex[d]
                    } else {
                        l.mon_lvl[d]
                    });
                    assert_eq!(
                        (
                            r.act,
                            r.room_count,
                            r.mon_den,
                            r.mon_umin,
                            r.mon_umax,
                            r.mon_wndr,
                            r.level_id,
                            r.ai_field,
                            r.quest,
                            r.monster_level
                        ),
                        (
                            l.act,
                            -1,
                            l.mon_den[d] as i32,
                            l.mon_umin[d],
                            l.mon_umax[d],
                            l.mon_wndr,
                            id,
                            -1,
                            l.quest,
                            [lvl; 2]
                        ),
                        "{c}"
                    );
                    let list = if d != 0 { &l.nmon } else { &l.mon };
                    let n = usize::from(l.num_mon).min(13).min(list.len());
                    let k = usize::from(r.entry_count);
                    assert!(k <= n, "{c}");
                    assert_eq!(r.mon_count, r.entry_count, "{c}");
                    let mut total = 0u8;
                    for e in &r.entries[..k] {
                        assert!(list.contains(&e.class), "{c}: class {}", e.class);
                        let m = p.mon(i32::from(e.class)).unwrap();
                        assert!(m.is_spawn, "{c}");
                        assert_eq!(e.rarity, m.rarity, "{c}");
                        total = total.wrapping_add(e.rarity);
                        assert!(e.variant_count <= 3, "{c}");
                        let counts = p
                            .mon2(i32::from(e.class))
                            .map_or([0; 16], |m2| m2.components);
                        for v in &e.variants[..usize::from(e.variant_count)] {
                            for i in 0..16 {
                                let lim = if counts[i] > 1 { counts[i] } else { 1 };
                                assert!(v[i] < lim, "{c}: class {} slot {i}", e.class);
                            }
                        }
                    }
                    assert_eq!(r.total_rarity, total, "{c}");
                    if id == BLOOD_MOOR && d == 0 {
                        assert_eq!(k, 3, "{c}");
                        assert_eq!(total, 6, "{c}: Blood Moor list total");
                    }
                    if id == COLD_PLAINS && d == 0 {
                        assert_eq!(k, 3, "{c}: the region keeps 3 of 4");
                    }
                }
            }
        }
    }
}
