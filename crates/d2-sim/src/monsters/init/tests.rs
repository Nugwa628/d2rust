//! Monster init tests: levels, boss and champion mods, umod init and
//! callbacks, and the umod table check, against a small fake host.
use std::collections::{BTreeMap, BTreeSet};

use d2_data::tables::{
    Difficultylevels, Levels, Monequip, Monlvl, Monprop, Monstats, Monstats2, Monumod, Record,
    Superuniques,
};

use super::*;
use crate::game::Game;
use crate::rng::Seed;
use crate::units::record::{UnitRecord, Units};
use crate::units::{RoomId, UnitType};

/// monumod `constants` of the base game's tables.
const K: [u32; 34] = [
    20, 100, 75, 50, 200, 150, 100, 300, 200, 100, 75, 100, 50, 100, 75, 150, 0, 33, 33, 0, 50, 50,
    33, 33, 33, 50, 50, 50, 66, 66, 66, 100, 100, 100,
];

fn zero<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// A monstats row: enabled, no skills, no monprop, no minions.
fn mon(level: u16, min_hp: u16, max_hp: u16, exp: u16) -> Monstats {
    let mut m: Monstats = zero();
    m.enabled = true;
    m.level = level;
    m.level_n = level;
    m.level_h = level;
    m.minhp = min_hp;
    m.maxhp = max_hp;
    m.exp = exp;
    m.ac = 10;
    m.monprop = 0xFFFF;
    m.minion1 = 0xFFFF;
    m.minion2 = 0xFFFF;
    for s in [
        &mut m.skill1,
        &mut m.skill2,
        &mut m.skill3,
        &mut m.skill4,
        &mut m.skill5,
        &mut m.skill6,
        &mut m.skill7,
        &mut m.skill8,
    ] {
        *s = 0xFFFF;
    }
    m
}

/// monlvl: every value 100, `DM`/`L-DM` = the level.
fn monlvl() -> Vec<Monlvl> {
    (0..111u32)
        .map(|i| {
            let mut r: Monlvl = zero();
            for v in [
                &mut r.hp,
                &mut r.hp_n,
                &mut r.hp_h,
                &mut r.l_hp,
                &mut r.l_hp_n,
                &mut r.l_hp_h,
                &mut r.ac,
                &mut r.ac_n,
                &mut r.ac_h,
                &mut r.l_ac,
                &mut r.l_ac_n,
                &mut r.l_ac_h,
                &mut r.xp,
                &mut r.xp_n,
                &mut r.xp_h,
                &mut r.l_xp,
                &mut r.l_xp_n,
                &mut r.l_xp_h,
            ] {
                *v = 100;
            }
            for v in [
                &mut r.dm,
                &mut r.dm_n,
                &mut r.dm_h,
                &mut r.l_dm,
                &mut r.l_dm_n,
                &mut r.l_dm_h,
            ] {
                *v = i;
            }
            r
        })
        .collect()
}

/// monumod: live constants; champion umods 16, 36–39 (cpick 1,
/// 36–39 version 100); unique umods 5, 6, 8, 9, 17, 18 (upick 1); 5 and
/// 9 transfer.
fn monumod() -> Vec<Monumod> {
    (0..43usize)
        .map(|i| {
            let mut r: Monumod = zero();
            r.enabled = 1;
            r.constants = K.get(i).copied().unwrap_or(0);
            if [16, 36, 37, 38, 39].contains(&i) {
                r.champion = 1;
                r.cpick = 1;
                r.cpick_n = 1;
                r.cpick_h = 1;
                if i >= 36 {
                    r.version = 100;
                }
            }
            if [5, 6, 8, 9, 17, 18].contains(&i) {
                r.upick = 1;
                r.upick_n = 1;
                r.upick_h = 1;
            }
            if [5, 9].contains(&i) {
                r.xfer = 1;
            }
            r
        })
        .collect()
}

fn difficulties() -> Vec<Difficultylevels> {
    [(0, 90), (3, 75), (7, 66)]
        .iter()
        .map(|&(s, b)| {
            let mut r: Difficultylevels = zero();
            r.monsterskillbonus = s;
            r.championdamagebonus = b;
            r
        })
        .collect()
}

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
    ids: NamedIds,
}

impl Tables {
    fn new(monstats: Vec<Monstats>) -> Self {
        let n = monstats.len();
        Self {
            monstats,
            monstats2: vec![zero()],
            monlvl: monlvl(),
            levels: Vec::new(),
            monprop: Vec::new(),
            monequip: Vec::new(),
            monumod: monumod(),
            superuniques: Vec::new(),
            difficultylevels: difficulties(),
            extra: vec![
                MonstatsExtra {
                    skill_modes: [-1; 8]
                };
                n
            ],
            components: vec![[0; 16]],
            ids: NamedIds::default(),
        }
    }

    fn ctx(self) -> Ctx<'static> {
        let t: &'static Tables = Box::leak(Box::new(self));
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
                ids: t.ids,
            },
        }
    }
}

const SEEDS: [u32; 4] = [1, 12345, 3_735_928_559, 4_014_346_870];

struct Fake {
    cx: Ctx<'static>,
    game: Game,
    units: Units,
    store: MonsterStore,
    info: GameInfo,
    room: RoomId,
    stats: BTreeMap<(UnitId, u16), i32>,
    log: Vec<String>,
    ai_flags: BTreeMap<UnitId, u16>,
    inventory: bool,
    items_at: BTreeSet<u8>,
    /// The owner's item codes by body location (`owner_item_code`).
    owner_items: BTreeMap<u8, [u8; 4]>,
    region: Vec<[u8; 16]>,
    level_id: i32,
    minions: BTreeMap<UnitId, Vec<UnitId>>,
    region_bosses: u32,
    next_seed: u32,
    /// Extra (type, parent) nestings for `montype_is` beyond equality.
    montype_nest: BTreeSet<(u16, u16)>,
    /// The server callback's max-life rescale (stat 7): a maxhp write
    /// sets hitpoints to new / old · current.
    rescale_hp: bool,
    /// The game's rules (a mod game's umods, MS5).
    rules: crate::rules::Ruleset,
}

impl Fake {
    fn new(cx: Ctx<'static>) -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = game.lists.create_room(0).unwrap();
        Self {
            cx,
            game,
            units: Units::new(),
            store: MonsterStore::new(),
            info: GameInfo {
                expansion: true,
                game_type: 3,
                players: 1,
                ..GameInfo::default()
            },
            room,
            stats: BTreeMap::new(),
            log: Vec::new(),
            ai_flags: BTreeMap::new(),
            inventory: false,
            items_at: BTreeSet::new(),
            owner_items: BTreeMap::new(),
            montype_nest: BTreeSet::new(),
            rescale_hp: false,
            rules: crate::rules::Ruleset::Original,
            region: Vec::new(),
            level_id: 2,
            minions: BTreeMap::new(),
            region_bosses: 0,
            next_seed: 0,
        }
    }

    /// A monster unit with a record and seed, without type init.
    fn unit(&mut self, class: u32, seed: u32) -> UnitId {
        let u = self
            .game
            .spawn_unit(UnitType::Monster, Some(self.room), false)
            .unwrap();
        let mut r = UnitRecord::new(UnitType::Monster, class, u.0);
        r.seed = Seed::init_low(seed);
        r.mode = mode::NEUTRAL;
        self.units.insert(u, r);
        u
    }

    /// A unit after the type init.
    fn monster(&mut self, class: u32, seed: u32) -> UnitId {
        let u = self.unit(class, seed);
        let cx = self.cx;
        type_init(&cx, self, u);
        u
    }

    fn s(&self, u: UnitId, s: u16) -> i32 {
        self.stats.get(&(u, s)).copied().unwrap_or(0)
    }

    fn data(&self, u: UnitId) -> MonsterData {
        self.store.get(u).cloned().unwrap_or_default()
    }

    fn seed_of(&self, u: UnitId) -> Seed {
        self.units.get(u).unwrap().seed
    }

    fn fresh_seed(&mut self) -> u32 {
        self.next_seed += 1;
        SEEDS[self.next_seed as usize % SEEDS.len()].wrapping_add(self.next_seed)
    }
}

impl InitHost for Fake {
    fn game(&mut self) -> &mut Game {
        &mut self.game
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn montype_is(&mut self, montype: u16, ty: u16) -> bool {
        montype == ty || self.montype_nest.contains(&(montype, ty))
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
        let old = self.stats.insert((unit, stat), value).unwrap_or(0);
        if self.rescale_hp && stat == stat::MAXHP && value != old && old > 0 {
            let c = self.s(unit, stat::HITPOINTS);
            if c > 0 {
                let q = (i64::from(value) * i64::from(c) / i64::from(old.max(256))) as i32;
                self.stats
                    .insert((unit, stat::HITPOINTS), q.max(1).min(value));
            }
        }
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
    fn region_variant_count(&mut self, _: UnitId, _: u32) -> u8 {
        self.region.len() as u8
    }
    fn region_variant(&mut self, _: UnitId, _: u32, i: u32) -> [u8; 16] {
        self.region[i as usize]
    }
    fn attach_quest_chain(&mut self, unit: UnitId) {
        self.log.push(format!("quest_chain_attach {}", unit.0));
    }
    fn set_combat_mode(&mut self, unit: UnitId) {
        self.log.push(format!("combat_mode {}", unit.0));
    }
    fn new_inventory(&mut self, _: UnitId, npc: bool) {
        self.log.push(format!("inventory {npc}"));
    }
    fn has_inventory(&mut self, _: UnitId) -> bool {
        self.inventory
    }
    fn give_skill(&mut self, _: UnitId, skill: u16, level: i32, mode: Option<u8>) {
        self.log.push(format!("skill {skill} {level} {mode:?}"));
    }
    fn apply_property(&mut self, _: UnitId, prop: i32, par: i32, min: i32, max: i32) {
        self.log.push(format!("prop {prop} {par} {min} {max}"));
    }
    fn has_item_at(&mut self, _: UnitId, loc: u8) -> bool {
        self.items_at.contains(&loc)
    }
    fn owner_item_code(&mut self, _: UnitId, loc: u8) -> Option<[u8; 4]> {
        self.owner_items.get(&loc).copied()
    }
    fn create_equip_item(&mut self, _: UnitId, code: [u8; 4], loc: u8, m: u8, level: i32) {
        self.log.push(format!(
            "item {} {loc} {m} {level}",
            String::from_utf8_lossy(&code)
        ));
    }
    fn place(&mut self, req: &CreateRequest) -> Option<(i32, i32)> {
        Some((req.x, req.y))
    }
    fn allocate(&mut self, req: &CreateRequest, _: i32, _: i32) -> Option<UnitId> {
        let s = self.fresh_seed();
        Some(self.monster(req.class, s))
    }
    fn set_alignment(&mut self, unit: UnitId, a: u8) {
        self.log.push(format!("align {} {a}", unit.0));
    }
    fn register_spawn(&mut self, unit: UnitId, _: &CreateRequest, never_count: u32) {
        self.log.push(format!("register {} {never_count}", unit.0));
    }
    fn party_minions(&mut self, unit: UnitId, _: &CreateRequest) {
        self.log.push(format!("party {}", unit.0));
    }
    fn quest_chain(&mut self, _: UnitId, chain: u32) {
        self.log.push(format!("quest_chain {chain}"));
    }
    fn create_boss_item(&mut self, _: UnitId, code: [u8; 4], loc: u8, _: i32) {
        let c = String::from_utf8_lossy(&code).trim_end().to_string();
        self.log.push(format!("boss_item {c} {loc}"));
    }
    fn set_corpse_noselect(&mut self, _: UnitId) {
        self.log.push("corpse_noselect".into());
    }
    fn boss_spawn(&mut self, req: &CreateRequest, guid: Option<u32>, _: bool) -> Option<UnitId> {
        self.log.push(format!("boss_spawn {guid:?}"));
        let s = self.fresh_seed();
        Some(self.monster(req.class, s))
    }
    fn count_region_boss(&mut self, _: UnitId) {
        self.region_bosses += 1;
    }
    fn spawn_boss_minion(&mut self, boss: UnitId, class: u32, _: Option<u32>) -> Option<UnitId> {
        let s = self.fresh_seed();
        let m = self.monster(class, s);
        self.log.push(format!("minion {} of {}", m.0, boss.0));
        Some(m)
    }
    fn link_minion(&mut self, boss: UnitId, minion: UnitId) {
        self.minions.entry(boss).or_default().push(minion);
    }
    fn minions(&mut self, boss: UnitId) -> Vec<UnitId> {
        self.minions.get(&boss).cloned().unwrap_or_default()
    }
    fn set_ai_flag(&mut self, unit: UnitId, flag: u16) {
        *self.ai_flags.entry(unit).or_default() |= flag;
    }
    fn give_aura(&mut self, _: UnitId, skill: u16, level: i32) {
        self.log.push(format!("aura {skill} {level}"));
    }
    fn spawn_near_unit(&mut self, _: UnitId, class: u32, mode: u32, spread: i32, flags: u32) {
        self.log
            .push(format!("near {class} {mode} {spread} {flags:#x}"));
    }
    fn spawn_group(&mut self, _: UnitId, class: u32, r: i32, n: i32, flags: u32) {
        self.log.push(format!("group {class} {r} {n} {flags:#x}"));
    }
    fn quest_preset_boss(&mut self, _: UnitId) {
        self.log.push("preset_boss".into());
    }
    fn owner_data_self(&mut self, _: UnitId) {
        self.log.push("owner_self".into());
    }
    fn class_for_level(&mut self, _: UnitId, class: u32) -> u32 {
        class + 1
    }
    fn monster_teardown(&mut self, _: UnitId, free_inventory: bool) {
        self.log.push(format!("teardown {free_inventory}"));
    }
    fn set_state(&mut self, _: UnitId, state: u16) {
        self.log.push(format!("state {state}"));
    }
    fn spawn_with_guid(&mut self, req: &CreateRequest) -> Option<UnitId> {
        self.log
            .push(format!("spawn_guid {} {:#x}", req.guid, req.flags));
        let s = self.fresh_seed();
        Some(self.monster(req.class, s))
    }
}

fn fake(monstats: Vec<Monstats>) -> Fake {
    Fake::new(Tables::new(monstats).ctx())
}

fn fake_with(t: Tables) -> Fake {
    Fake::new(t.ctx())
}

// ----------

#[test]
fn pct_vectors() {
    assert_eq!(pct(1792, 200, 100), 3584);
    assert_eq!(pct(0x7FFF_FF00, 50, 100), 1_073_741_650);
    assert_eq!(pct(5, 70000, 100), 3500);
    assert_eq!(pct(7, -75, 100), -5);
    assert_eq!(pct(7, 3, 0), 0);
    // Branch 2, d > v >> 4: 64-bit.
    assert_eq!(pct(0x20_0000, 1000, 0x30_0000), 666);
    // Branch 3, d > p >> 4: 64-bit.
    assert_eq!(pct(3, 0x2_0000, 0x3_0000), 2);
    // Branch 4 wraps 32-bit.
    assert_eq!(pct(0x10_0000, 0x1_0000, 1), 0);
}

// --------

#[test]
fn player_count_vectors() {
    let info = |players, game_type, players_x| GameInfo {
        players,
        game_type,
        players_x,
        ..GameInfo::default()
    };
    for (n, hp, xp) in [
        (1, 0, 0),
        (2, 50, 50),
        (3, 100, 100),
        (8, 350, 350),
        (9, 350, 350),
    ] {
        let b = player_bonus(&info(n, 0, 0), 0);
        assert_eq!((b.players, b.hp, b.xp), (n, hp, xp), "n = {n}");
    }
    assert_eq!(
        player_bonus(&info(12, 0, 0), 0),
        PlayerBonus {
            players: 12,
            hp: 500,
            xp: 380
        }
    );
    // n ≥ 1; players-X only for game types 1–3.
    assert_eq!(player_bonus(&info(0, 0, 0), 0).players, 1);
    assert_eq!(player_bonus(&info(1, 3, 8), 0).hp, 350);
    assert_eq!(player_bonus(&info(1, 0, 8), 0).hp, 0);
    assert_eq!(player_bonus(&info(1, 4, 8), 0).hp, 0);
    // Align ≠ 0.
    assert_eq!(
        player_bonus(&info(8, 3, 8), 1),
        PlayerBonus {
            players: 1,
            hp: 0,
            xp: 0
        }
    );
}

#[test]
fn skill_bonus_and_difficulty_write() {
    let mut m = mon(1, 1, 1, 1);
    m.skill1 = 40;
    m.sk1lvl = 2;
    let mut t = Tables::new(vec![m]);
    t.extra[0].skill_modes[0] = 4;
    let mut f = fake_with(t);
    f.info.difficulty = 5;
    f.monster(0, 1);
    assert_eq!(f.info.difficulty, 2);
    assert!(
        f.log.contains(&"skill 40 9 Some(4)".to_string()),
        "{:?}",
        f.log
    );
}

// --------------

fn levels() -> Vec<Levels> {
    (0..4u16)
        .map(|i| {
            let mut l: Levels = zero();
            l.monlvl1 = 10 + i;
            l.monlvl2 = 20 + i;
            l.monlvl3 = 30 + i;
            l.monlvl1ex = 40 + i;
            l.monlvl2ex = 50 + i;
            l.monlvl3ex = 60 + i;
            l
        })
        .collect()
}

#[test]
fn monster_level_rules() {
    let mut m = mon(1, 1, 1, 1);
    m.level_n = 5;
    m.level_h = 9;
    let lv = levels();
    let x = GameInfo {
        expansion: true,
        ..GameInfo::default()
    };
    let c = GameInfo::default();
    assert_eq!(monster_level(&m, &lv, &x, 0, 2), 1);
    assert_eq!(monster_level(&m, &lv, &x, 1, 2), 52);
    assert_eq!(monster_level(&m, &lv, &x, 2, 3), 63);
    assert_eq!(monster_level(&m, &lv, &c, 1, 2), 5);
    assert_eq!(monster_level(&m, &lv, &c, 2, 2), 9);
    // Level id ≤ 0 or ≥ rows → 1.
    assert_eq!(monster_level(&m, &lv, &x, 1, 0), 1);
    assert_eq!(monster_level(&m, &lv, &x, 1, 4), 1);
    assert_eq!(area_level(&lv, 2, 3, true), 1);
    assert_eq!(area_level(&lv, 2, 1, false), 22);
    // noRatio / boss use the column.
    m.noratio = true;
    assert_eq!(monster_level(&m, &lv, &x, 1, 2), 5);
    m.noratio = false;
    m.boss = true;
    assert_eq!(monster_level(&m, &lv, &x, 2, 2), 9);
    // Hirelings use d = 0.
    let mut ms = vec![mon(1, 1, 1, 1); 272];
    ms[271].level = 7;
    ms[271].level_h = 60;
    let mut t = Tables::new(ms);
    t.levels = levels();
    let mut f = fake_with(t);
    f.info.difficulty = 2;
    let u = f.monster(271, 1);
    assert_eq!(f.s(u, stat::LEVEL), 7);
}

#[test]
fn stats_by_level_rules() {
    let mut m = mon(1, 7, 12, 33);
    m.minhp_n = 20;
    m.ac = 5;
    let mut lvl = monlvl();
    lvl[3].hp = 200;
    lvl[3].l_hp = 300;
    lvl[3].hp_n = 400;
    lvl[110].l_xp = 1000;
    let r = stats_by_level(&m, &lvl, false, 0, 3);
    assert_eq!(
        r,
        LevelStats {
            min_hp: 14,
            max_hp: 24,
            ac: 5,
            xp: 33
        }
    );
    let r = stats_by_level(&m, &lvl, true, 0, 3);
    assert_eq!((r.min_hp, r.max_hp), (21, 36));
    assert_eq!(stats_by_level(&m, &lvl, false, 1, 3).min_hp, 80);
    // Level clamped to rows − 1; negative gives nothing.
    assert_eq!(stats_by_level(&m, &lvl, true, 0, 500).xp, 330);
    assert_eq!(stats_by_level(&m, &lvl, true, 0, -1), LevelStats::default());
    // noRatio: the monstats values.
    m.noratio = true;
    assert_eq!(
        stats_by_level(&m, &lvl, true, 0, 3),
        LevelStats {
            min_hp: 7,
            max_hp: 12,
            ac: 5,
            xp: 33
        }
    );
    // Signed 16-bit reads.
    m.exp = 0xFFFF;
    assert_eq!(stats_by_level(&m, &lvl, true, 0, 3).xp, -1);
}

// --------

#[test]
fn hp_roll_and_base_stats() {
    let mut m = mon(1, 7, 12, 33);
    m.noratio = true;
    m.resdm = 10;
    m.respo = 50;
    m.toblock = 3;
    for &s in &SEEDS {
        let mut f = fake(vec![m.clone()]);
        let u = f.monster(0, s);
        let expect = (7 + Seed::init_low(s).roll(6) as i32) * 256;
        assert_eq!(f.s(u, stat::MAXHP), expect);
        assert_eq!(f.s(u, stat::HITPOINTS), expect);
        assert_eq!(f.s(u, stat::ARMORCLASS), 10);
        assert_eq!(f.s(u, stat::EXPERIENCE), 33);
        assert_eq!(f.s(u, stat::LEVEL), 1);
        assert_eq!(f.s(u, stat::MONSTER_PLAYERCOUNT), 1);
        assert_eq!(f.s(u, stat::DAMAGERESIST), 10);
        assert_eq!(f.s(u, stat::POISONRESIST), 50);
        assert_eq!(f.s(u, stat::TOBLOCK), 3);
        assert_eq!(f.s(u, stat::ATTACKRATE), 100);
        assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75);
        assert_eq!(f.s(u, stat::OTHER_ANIMRATE), 100);
        assert_eq!(f.s(u, stat::LAST_SENT_HP_PCT), 128);
        // One draw only.
        let mut one = Seed::init_low(s);
        one.step();
        assert_eq!(f.seed_of(u), one);
    }
    // Player bonus: 3 players → HP and XP +100 %.
    let mut f = fake(vec![m.clone()]);
    f.info.players = 3;
    let u = f.monster(0, 1);
    assert_eq!(
        f.s(u, stat::MAXHP),
        (7 + Seed::init_low(1).roll(6) as i32) * 2 * 256
    );
    assert_eq!(f.s(u, stat::EXPERIENCE), 66);
    assert_eq!(f.s(u, stat::MONSTER_PLAYERCOUNT), 3);
    // maxHP < minHP: no step, base = minHP.
    let mut m2 = m.clone();
    m2.maxhp = 3;
    let mut f = fake(vec![m2]);
    let u = f.monster(0, 1);
    assert_eq!(f.s(u, stat::MAXHP), 7 * 256);
    assert_eq!(f.seed_of(u), Seed::init_low(1));
}

#[test]
fn hp_cap() {
    let m = mon(1, 100, 100, 1);
    let mut t = Tables::new(vec![m]);
    t.monlvl[1].l_hp = 0x90_0000;
    let mut f = fake_with(t);
    let u = f.monster(0, 1);
    assert_eq!(f.s(u, stat::MAXHP), 0x7F_FFFF * 256);
}

#[test]
fn hp_regen_rule() {
    assert_eq!(hp_regen(1792, 2), Some(0));
    assert_eq!(hp_regen(4096 * 10, 3), Some(30));
    // Overflow branch: (maxhp >> 12) × DamageRegen.
    assert_eq!(
        hp_regen(0x7FFF_FF00, 0xFFF),
        Some((0x7FFF_FF00 >> 12) * 0xFFF)
    );
    assert_eq!(hp_regen(1, 0x1000), None);
    assert_eq!(hp_regen(5000, 0), Some(0));
}

#[test]
fn classic_scaling_rule() {
    let mut m = mon(4, 20, 20, 170);
    m.noratio = true;
    m.ac_n = 120;
    m.exp_n = 170;
    m.minhp_n = 20;
    m.maxhp_n = 20;
    let mut f = fake(vec![m.clone()]);
    f.info.expansion = false;
    f.info.difficulty = 1;
    let u = f.monster(0, 1);
    assert_eq!(f.s(u, stat::MAXHP), 20 * 256 / 2);
    assert_eq!(f.s(u, stat::HITPOINTS), 20 * 256);
    assert_eq!(f.s(u, stat::ARMORCLASS), 100);
    assert_eq!(f.s(u, stat::EXPERIENCE), 100);
    assert_eq!(f.s(u, stat::LEVEL), 4 + 25);
    // Hell XP (10, 26); level + 50.
    assert_eq!(
        classic_scaling(&f.info, 2, &m, 512, 26, 260),
        Some((256, 21, 100, 54))
    );
    // Not applied: expansion, Normal, Align 1.
    let x = GameInfo {
        expansion: true,
        ..GameInfo::default()
    };
    assert_eq!(classic_scaling(&x, 1, &m, 512, 26, 260), None);
    assert_eq!(
        classic_scaling(&GameInfo::default(), 0, &m, 512, 26, 260),
        None
    );
    m.align = 1;
    assert_eq!(
        classic_scaling(&GameInfo::default(), 1, &m, 512, 26, 260),
        None
    );
}

#[test]
fn inventory_skills_flags() {
    let mut m = mon(1, 1, 1, 1);
    m.inventory = true;
    m.interact = true;
    m.petignore = true;
    m.skill1 = 10;
    m.sk1lvl = 3;
    m.skill2 = 11;
    m.sk2lvl = 0;
    m.skill3 = 12;
    m.sk3lvl = 1;
    let mut t = Tables::new(vec![m]);
    t.extra[0].skill_modes = [7, -1, -1, -1, -1, -1, -1, -1];
    t.monstats2[0].isatt = true;
    let mut f = fake_with(t);
    f.info.difficulty = 1;
    let u = f.monster(0, 1);
    let rel: Vec<&String> = f
        .log
        .iter()
        .filter(|l| l.starts_with("skill") || l.starts_with("inventory"))
        .collect();
    assert_eq!(
        rel,
        ["inventory true", "skill 10 6 Some(7)", "skill 12 4 None"]
    );
    let flags = f.units.get(u).unwrap().flags;
    assert_eq!(
        flags & (unit_flag::AT_INIT | unit_flag::IS_ATT | unit_flag::PET_IGNORE),
        unit_flag::AT_INIT | unit_flag::IS_ATT | unit_flag::PET_IGNORE
    );
}

#[test]
fn type_init_sequence() {
    let mut f = fake(vec![mon(1, 1, 1, 1)]);
    let u = f.unit(0, 1);
    f.game.schedule_event(u, 2, 5, None, 0, 0).unwrap();
    let cx = f.cx;
    type_init(&cx, &mut f, u);
    assert!(f.game.timers.unit_timers(u).is_empty(), "timers cancelled");
    assert_eq!(f.data(u).level_id, 2);
    assert_eq!(f.data(u).class, 0);
    assert_eq!(
        f.log,
        [
            format!("alloc_ai {}", u.0),
            format!("ai_install {} 0", u.0),
            format!("quest_chain_attach {}", u.0)
        ]
    );
    // Death / dead mode: combat mode instead.
    let v = f.unit(0, 1);
    f.units.get_mut(v).unwrap().mode = mode::DEAD;
    f.log.clear();
    type_init(&cx, &mut f, v);
    assert_eq!(f.log.last().unwrap(), &format!("combat_mode {}", v.0));
}

// ---------

#[test]
fn components_rules() {
    let counts = [3, 3, 3, 3, 3, 0, 0, 0, 3, 3, 3, 1, 0, 0, 0, 0];
    let mut t = Tables::new(vec![mon(1, 1, 1, 1); 313]);
    t.components = vec![counts];
    let mut f = fake_with(t);
    for &s in &SEEDS {
        let u = f.monster(5, s);
        let mut r = Seed::init_low(s);
        let want: Vec<u8> = counts.iter().map(|&c| r.roll(i32::from(c)) as u8).collect();
        assert_eq!(f.data(u).components.to_vec(), want);
        // Doomknight3: components 10 and 11 = one roll(4) after.
        let u = f.monster(312, s);
        let mut r = Seed::init_low(s);
        counts.iter().for_each(|&c| {
            r.roll(i32::from(c));
        });
        let v = r.roll(4) as u8;
        assert_eq!(f.data(u).components[10..12], [v, v]);
        let u = f.monster(311, s);
        assert_eq!(f.data(u).components[10..12], [v, want[11]]);
    }
    // Region variants: one roll(count).
    f.region = vec![[1; 16], [2; 16], [3; 16]];
    let u = f.monster(5, 7);
    let i = Seed::init_low(7).roll(3) as usize;
    assert_eq!(f.data(u).components, f.region[i]);
}

// --------------

#[test]
fn monprop_rule() {
    let mut m = mon(1, 1, 1, 1);
    m.monprop = 0;
    let mut p: Monprop = zero();
    p.prop1 = 5;
    p.par1 = 1;
    p.min1 = 2;
    p.max1 = 3;
    p.prop2 = 6;
    p.chance2 = 50;
    p.prop3 = u32::MAX;
    p.prop4 = 9;
    p.prop1_n = 77;
    p.prop2_n = u32::MAX;
    let mut t = Tables::new(vec![m]);
    t.monprop = vec![p];
    let mut f = fake_with(t);
    for &s in &SEEDS {
        f.log.clear();
        let u = f.monster(0, s);
        let mut r = Seed::init_low(s);
        r.step(); // HP roll (roll(1))
        let hit = r.step() % 100 < 50;
        let props: Vec<&String> = f.log.iter().filter(|l| l.starts_with("prop")).collect();
        let mut want = vec!["prop 5 1 2 3".to_string()];
        if hit {
            want.push("prop 6 0 0 0".into());
        }
        assert_eq!(props, want.iter().collect::<Vec<_>>());
        assert_eq!(f.seed_of(u), r);
    }
    f.info.difficulty = 1;
    f.log.clear();
    f.monster(0, 1);
    assert!(f.log.contains(&"prop 77 0 0 0".to_string()));
}

fn equip(class: u16, level: u16, oninit: u8, slots: &[(&[u8; 4], u8)]) -> Monequip {
    let mut r: Monequip = zero();
    r.monster = class;
    r.level = level;
    r.oninit = oninit;
    let mut s = slots.iter();
    if let Some(&(i, l)) = s.next() {
        r.item1 = *i;
        r.loc1 = l;
        r.mod1 = 9;
    }
    if let Some(&(i, l)) = s.next() {
        r.item2 = *i;
        r.loc2 = l;
        r.mod2 = 3;
    }
    if let Some(&(i, l)) = s.next() {
        r.item3 = *i;
        r.loc3 = l;
    }
    r
}

#[test]
fn monequip_rule() {
    let mut t = Tables::new(vec![mon(1, 1, 1, 1), mon(5, 1, 1, 1)]);
    t.monequip = vec![
        equip(0, 0, 0, &[(b"axe ", 4)]),
        equip(1, 9, 1, &[(b"sbw ", 5)]),
        equip(1, 3, 1, &[(b"aaa ", 4), (b"bbb ", 5), (b"ccc ", 11)]),
        equip(1, 0, 1, &[(b"ddd ", 0)]),
        equip(1, 0, 1, &[(b"eee ", 7)]),
    ];
    let mut f = fake_with(t);
    // No inventory: nothing.
    f.monster(1, 1);
    assert!(!f.log.iter().any(|l| l.starts_with("item")));
    f.inventory = true;
    for &s in &SEEDS {
        f.log.clear();
        let u = f.monster(1, s);
        let mut r = Seed::init_low(s);
        r.step(); // HP
        let k = r.roll(2);
        let items: Vec<&String> = f.log.iter().filter(|l| l.starts_with("item")).collect();
        let first = if k == 0 {
            "item aaa  4 0 5"
        } else {
            "item bbb  5 3 5"
        };
        r.roll(1);
        assert_eq!(items, [first, "item eee  7 0 5"]);
        assert_eq!(f.seed_of(u), r);
    }
    // A held location is skipped; oninit 0 on the first row: nothing.
    f.items_at.insert(7);
    f.log.clear();
    f.monster(1, 1);
    assert!(!f.log.iter().any(|l| l.starts_with("item eee")));
    f.log.clear();
    f.monster(0, 1);
    assert!(!f.log.iter().any(|l| l.starts_with("item")));
}

#[test]
fn summon_equipment_rows() {
    let mut t = Tables::new(vec![mon(1, 1, 1, 1), mon(5, 1, 1, 1)]);
    t.monequip = vec![
        equip(0, 0, 0, &[(b"axe ", 4)]),
        equip(1, 9, 0, &[(b"hi  ", 4)]),
        equip(1, 3, 0, &[(b"lo  ", 5)]),
        equip(1, 0, 0, &[(b"    ", 7)]),
    ];
    let rows = t.monequip.clone();
    let mut f = fake_with(t);
    f.inventory = true;
    // oninit 0 rows are used (no test), rows above L are skipped, the
    // item level is the given one.
    let u = f.monster(1, 1);
    f.log.clear();
    monequip_rows(&rows, &mut f, u, None, 4, 12, false);
    let items: Vec<&String> = f.log.iter().filter(|l| l.starts_with("item")).collect();
    // lo at 5 (mod1 9 → 0), and the four-space row with no owner item: none.
    assert_eq!(items, ["item lo   5 0 12"]);
    // With the oninit test the same rows give nothing.
    f.log.clear();
    monequip_rows(&rows, &mut f, u, None, 4, 12, true);
    assert!(!f.log.iter().any(|l| l.starts_with("item")));
}

#[test]
fn summon_equipment_copies_the_owners_item_code() {
    // The Shadow rows: `tors` (3) and `head` (1) with no item code.
    let mut t = Tables::new(vec![mon(1, 1, 1, 1), mon(5, 1, 1, 1)]);
    t.monequip = vec![
        equip(1, 0, 0, &[(b"    ", 3)]),
        equip(1, 0, 0, &[(b"    ", 1)]),
    ];
    let rows = t.monequip.clone();
    let mut f = fake_with(t);
    f.inventory = true;
    let owner = f.monster(5, 1);
    let u = f.monster(1, 1);
    // The owner wears armor only: the armor code is copied, the helm row
    // makes nothing.
    f.owner_items.insert(3, *b"qui ");
    f.log.clear();
    monequip_rows(&rows, &mut f, u, Some(owner), 0, 7, false);
    let items: Vec<&String> = f.log.iter().filter(|l| l.starts_with("item")).collect();
    assert_eq!(items, ["item qui  3 0 7"]);
    // Both: both copied, in row order.
    f.owner_items.insert(1, *b"cap ");
    f.log.clear();
    monequip_rows(&rows, &mut f, u, Some(owner), 0, 7, false);
    let items: Vec<&String> = f.log.iter().filter(|l| l.starts_with("item")).collect();
    assert_eq!(items, ["item qui  3 0 7", "item cap  1 0 7"]);
    // No owner: nothing.
    f.log.clear();
    monequip_rows(&rows, &mut f, u, None, 0, 7, false);
    assert!(!f.log.iter().any(|l| l.starts_with("item")));
}

// -------------

#[test]
fn creation_sequence_and_normal_mods() {
    let mut ms = vec![mon(1, 1, 1, 1); 30];
    ms[28].baseid = 24; // brute1 → brute2's base
    ms[28].align = 1;
    ms[29].baseid = 211;
    let mut f = fake(ms);
    let req = CreateRequest {
        class: 28,
        mode: 1,
        x: 10,
        y: 20,
        ..CreateRequest::default()
    };
    let cx = f.cx;
    let u = create(&cx, &mut f, &req).unwrap().unwrap();
    assert_eq!(f.data(u).umod_list(), [13]);
    assert_eq!(f.data(u).type_flags, 0);
    assert!(f.units.get(u).unwrap().flags & unit_flag::ALIGN1 != 0);
    let tail: Vec<&String> = f.log.iter().rev().take(2).collect();
    assert_eq!(
        tail,
        [&format!("party {}", u.0), &format!("align {} 2", u.0)]
    );
    // Flags 0x02 and 0x40.
    f.log.clear();
    let u = create(&cx, &mut f, &CreateRequest { flags: 0x42, ..req })
        .unwrap()
        .unwrap();
    assert!(f.data(u).umod_list().is_empty());
    assert!(!f.log.iter().any(|l| l.starts_with("party")));
    // Probe: nothing allocated.
    f.log.clear();
    assert_eq!(
        create(&cx, &mut f, &CreateRequest { flags: 1, ..req }),
        Some(None)
    );
    assert!(f.log.is_empty());
    // Duriel: 11 then 22 with unique.
    let u = create(&cx, &mut f, &CreateRequest { class: 29, ..req })
        .unwrap()
        .unwrap();
    assert_eq!(f.data(u).umod_list(), [11, 22]);
    assert!(f.data(u).has_flag(type_flag::UNIQUE));
    // The table.
    for (b, want) in [
        (91, vec![(20, false)]),
        (96, vec![(10, false)]),
        (326, vec![(14, false)]),
        (330, vec![(14, false)]),
        (354, vec![(14, false)]),
        (340, vec![(15, false)]),
        (343, vec![(15, false)]),
        (436, vec![(34, false)]),
        (461, vec![(33, false)]),
        (501, vec![(35, false)]),
        (540, vec![(22, true)]),
        (542, vec![(22, true)]),
        (25, vec![]),
        (331, vec![]),
    ] {
        assert_eq!(normal_mods_for(b).to_vec(), want, "BaseId {b}");
    }
}

#[test]
fn boss_mods_bloodraven() {
    let mut ms = vec![mon(1, 1, 1, 1); 2];
    ms[1].baseid = 267;
    let t = Tables::new(ms);
    let mut f = fake_with(t);
    let cx = f.cx;
    let u = create(
        &cx,
        &mut f,
        &CreateRequest {
            class: 1,
            ..CreateRequest::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(f.data(u).umod_list(), [12, 22]);
    assert!(f.data(u).has_flag(type_flag::UNIQUE));
    assert_eq!(f.region_bosses, 1);
    assert!(f.log.contains(&"quest_chain 2".to_string()));
    assert!(f.log.contains(&"corpse_noselect".to_string()));
}

#[test]
fn boss_mods_case_table() {
    use super::create::{boss_mods_for, BossStep::*};
    // Uber classes take their own branch of the shared BaseId.
    assert_eq!(
        boss_mods_for(156, 707),
        [Umod(23, true), Umod(6, true), Umod(29, true)]
    );
    assert_eq!(boss_mods_for(156, 156), [Umod(22, true), Chain(6)]);
    assert_eq!(boss_mods_for(211, 211), [Chain(13), Chain(9)]);
    assert_eq!(boss_mods_for(211, 708), [Umod(6, true), Umod(18, true)]);
    assert_eq!(
        boss_mods_for(242, 704),
        [
            Umod(22, true),
            Umod(30, true),
            Umod(17, true),
            Umod(8, true),
            Umod(6, true)
        ]
    );
    assert_eq!(boss_mods_for(242, 242), [Chain(20), Umod(22, true)]);
    assert_eq!(boss_mods_for(243, 243), [Umod(22, true), Chain(23)]);
    assert_eq!(
        boss_mods_for(250, 250),
        [Chain(12), UnitFlags(0x800), DataFlag1]
    );
    assert_eq!(boss_mods_for(292, 292), [Umod(31, false)]);
    assert_eq!(boss_mods_for(343, 343), [UnitFlags(0x20000)]);
    assert_eq!(
        boss_mods_for(366, 366),
        [Chain(19), UnitFlags(0x20000), Umod(22, true)]
    );
    assert_eq!(boss_mods_for(540, 541), [AncientEquip]);
    assert_eq!(
        boss_mods_for(544, 709),
        [Umod(22, true), Umod(18, true), Umod(8, true), Umod(6, true)]
    );
    // warriv2's hook acts only for 201 / 331; no other BaseId has a case.
    assert!(boss_mods_for(175, 175).is_empty());
    assert!(boss_mods_for(24, 24).is_empty());
    // No case assigns umods 40 or 41.
    for b in 0..800 {
        for c in [b as u32, 704, 705, 706, 707, 708, 709] {
            assert!(!boss_mods_for(b, c)
                .iter()
                .any(|s| matches!(s, Umod(40 | 41, _))));
        }
    }
}

#[test]
fn boss_chains_follow_the_base_id() {
    // Rows 0..=709, each its own base; 707 (uberandariel) and 706
    // (uberizual) share Andariel's and Izual's bases.
    let mut ms: Vec<Monstats> = (0..710u16)
        .map(|i| {
            let mut r = mon(1, 1, 1, 1);
            r.baseid = i;
            r
        })
        .collect();
    ms[707].baseid = 156;
    ms[706].baseid = 256;
    let chains = |c: u32| boss_chains(&ms, c).collect::<Vec<_>>();
    assert_eq!(chains(156), [6]);
    assert_eq!(chains(211), [13, 9]);
    assert_eq!(chains(242), [20]);
    assert_eq!(chains(243), [23]);
    assert_eq!(chains(256), [22]);
    assert_eq!(chains(267), [2]);
    assert_eq!(chains(409), [24]);
    assert!(chains(707).is_empty(), "the uber case links no chain");
    assert!(chains(706).is_empty());
    assert!(chains(1).is_empty());
    // A missing row keys on the class itself.
    assert_eq!(boss_chains(&[], 229).collect::<Vec<_>>(), [8]);
}

#[test]
fn boss_mods_summoner_and_ancient_equipment() {
    let mut ms = vec![mon(1, 1, 1, 1); 542];
    ms[1].baseid = 250;
    ms[2].baseid = 540;
    ms[541].baseid = 540;
    let mut f = fake_with(Tables::new(ms));
    let cx = f.cx;
    let u = create(
        &cx,
        &mut f,
        &CreateRequest {
            class: 1,
            ..CreateRequest::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(f.data(u).data_flag1);
    assert_ne!(f.units.get(u).unwrap().flags & 0x800, 0);
    assert!(f.log.contains(&"quest_chain 12".to_string()));
    // Class 2 has BaseId 540 but the equipment table is keyed by the
    // class: only 540–542 read a row, so nothing is created.
    create(
        &cx,
        &mut f,
        &CreateRequest {
            class: 2,
            ..CreateRequest::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(!f.log.iter().any(|l| l.starts_with("boss_item")));
    // Class 541 (BaseId 540) reads its own row, in k order.
    create(
        &cx,
        &mut f,
        &CreateRequest {
            class: 541,
            ..CreateRequest::default()
        },
    )
    .unwrap()
    .unwrap();
    let items: Vec<_> = f
        .log
        .iter()
        .filter(|l| l.starts_with("boss_item"))
        .collect();
    assert_eq!(
        items,
        [
            "boss_item tax 4",
            "boss_item tax 5",
            "boss_item hgl 10",
            "boss_item hbt 9"
        ]
    );
}

// ---------

fn velocity_mon(v: u16, base: u16) -> Monstats {
    let mut m = mon(2, 5, 9, 32);
    m.velocity = v;
    m.baseid = base;
    m
}

#[test]
fn champion_function_vectors() {
    for (base, dmg) in [(58, 90), (118, 45)] {
        let mut f = fake(vec![velocity_mon(6, base)]);
        let u = f.monster(0, 1);
        f.set_stat(u, stat::EXPERIENCE, 160);
        f.set_stat(u, stat::LEVEL, 5);
        let cx = f.cx;
        run_umod_init(&cx, &mut f, u, 16, true);
        assert_eq!(f.s(u, stat::EXPERIENCE), 96);
        assert_eq!(f.s(u, stat::LEVEL), 4);
        assert_eq!(f.s(u, stat::DAMAGEPERCENT), dmg);
        assert_eq!(f.s(u, stat::ITEM_TOHIT_PERCENT), 67);
        assert_eq!(f.s(u, stat::VELOCITYPERCENT), 95);
        // unique = 0: nothing.
        run_umod_init(&cx, &mut f, u, 16, false);
        assert_eq!(f.s(u, stat::EXPERIENCE), 96);
    }
    // e = 3 stays 3; Velocity 0: no velocity change.
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let u = f.monster(0, 1);
    f.set_stat(u, stat::EXPERIENCE, 3);
    let cx = f.cx;
    run_umod_init(&cx, &mut f, u, 16, true);
    assert_eq!(f.s(u, stat::EXPERIENCE), 3);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75);
}

#[test]
fn fast_and_strong() {
    for (v, plus) in [(4, 100), (15, 10), (12, 42)] {
        let mut f = fake(vec![velocity_mon(v, 1)]);
        let u = f.monster(0, 1);
        let cx = f.cx;
        run_umod_init(&cx, &mut f, u, 6, false);
        assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75 + plus, "Velocity {v}");
    }
    // Strong, Normal (B 90): unique 150/100, other 75/50.
    for (base, unique, dmg, th) in [(1, true, 135, 90), (1, false, 67, 45), (118, true, 67, 90)] {
        let mut f = fake(vec![velocity_mon(0, base)]);
        let u = f.monster(0, 1);
        let cx = f.cx;
        run_umod_init(&cx, &mut f, u, 5, unique);
        assert_eq!(f.s(u, stat::DAMAGEPERCENT), dmg);
        assert_eq!(f.s(u, stat::ITEM_TOHIT_PERCENT), th);
    }
}

#[test]
fn fixed_umods() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let u = f.monster(0, 1);
    let cx = f.cx;
    // Umod 1: unique only; low 16 bits of one step.
    run_umod_init(&cx, &mut f, u, 1, false);
    assert_eq!(f.data(u).name_seed, 0);
    let mut s = f.seed_of(u);
    run_umod_init(&cx, &mut f, u, 1, true);
    assert_eq!(f.data(u).name_seed, s.step() as u16);
    // Umod 2: champion K[4] (×3), unique K[7] (×4), minion K[1] (×2).
    f.set_stat(u, stat::HPREGEN, 9);
    for (flags, unique, mul) in [(type_flag::CHAMPION, true, 3), (0, true, 4), (0, false, 2)] {
        f.store.entry(u).type_flags = flags;
        f.set_stat(u, stat::MAXHP, 1792);
        run_umod_init(&cx, &mut f, u, 2, unique);
        assert_eq!(f.s(u, stat::MAXHP), 1792 * mul);
        assert_eq!(f.s(u, stat::HITPOINTS), 1792 * mul);
    }
    assert_eq!(f.s(u, stat::HPREGEN), 0);
    // NM champion × 2.5.
    f.info.difficulty = 1;
    f.store.entry(u).type_flags = type_flag::CHAMPION;
    f.set_stat(u, stat::MAXHP, 1000);
    run_umod_init(&cx, &mut f, u, 2, true);
    assert_eq!(f.s(u, stat::MAXHP), 2500);
    // Umod 4 ignores the flag.
    f.set_stat(u, stat::LEVEL, 2);
    f.set_stat(u, stat::EXPERIENCE, 32);
    run_umod_init(&cx, &mut f, u, 4, false);
    assert_eq!((f.s(u, stat::LEVEL), f.s(u, stat::EXPERIENCE)), (5, 160));
}

#[test]
fn resistances() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    let res = |f: &Fake, u| {
        [
            stat::FIRERESIST,
            stat::LIGHTRESIST,
            stat::COLDRESIST,
            stat::POISONRESIST,
            stat::DAMAGERESIST,
            stat::MAGICRESIST,
        ]
        .map(|s| f.s(u, s))
    };
    // 8: cold reaches 100 (1 immunity + poison = 2) → stops.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::COLDRESIST, 60);
    f.set_stat(u, stat::POISONRESIST, 100);
    run_umod_init(&cx, &mut f, u, 8, true);
    assert_eq!(res(&f, u), [0, 0, 100, 100, 0, 0]);
    // 8: none immune → cold, fire, light +40.
    let u = f.monster(0, 1);
    run_umod_init(&cx, &mut f, u, 8, true);
    assert_eq!(res(&f, u), [40, 40, 40, 0, 0, 0]);
    // 8: fire already ≥ 100 is skipped, not counted twice.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::FIRERESIST, 100);
    run_umod_init(&cx, &mut f, u, 8, true);
    assert_eq!(res(&f, u), [100, 40, 40, 0, 0, 0]);
    // 27: +20 below 75.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::COLDRESIST, 75);
    run_umod_init(&cx, &mut f, u, 27, true);
    assert_eq!(res(&f, u), [20, 20, 75, 0, 0, 0]);
    // 28: armor doubled always; damage +50 when < 2 immunities.
    let u = f.monster(0, 1);
    run_umod_init(&cx, &mut f, u, 28, true);
    assert_eq!(f.s(u, stat::ARMORCLASS), 20);
    assert_eq!(res(&f, u)[4], 50);
    let u = f.monster(0, 1);
    f.set_stat(u, stat::FIRERESIST, 100);
    f.set_stat(u, stat::MAGICRESIST, 100);
    run_umod_init(&cx, &mut f, u, 28, true);
    assert_eq!((f.s(u, stat::ARMORCLASS), res(&f, u)[4]), (20, 0));
    // Not unique: nothing.
    let u = f.monster(0, 1);
    run_umod_init(&cx, &mut f, u, 28, false);
    assert_eq!(f.s(u, stat::ARMORCLASS), 10);
}

#[test]
fn elemental_umods() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    // Fire, unique, Normal, level 10: DM 10 × K[28] 66 / 100, × K[31] 100.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 10);
    run_umod_init(&cx, &mut f, u, 9, true);
    assert_eq!(
        (f.s(u, stat::FIREMINDAM), f.s(u, stat::FIREMAXDAM)),
        (6, 10)
    );
    assert_eq!(f.s(u, stat::FIRERESIST), 75);
    // Minion: K[16] 0, K[19] 0; no resist.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 10);
    run_umod_init(&cx, &mut f, u, 9, false);
    assert_eq!((f.s(u, stat::FIREMAXDAM), f.s(u, stat::FIRERESIST)), (0, 0));
    // Cold length and poison length.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::LEVEL, 10);
    run_umod_init(&cx, &mut f, u, 18, true);
    assert_eq!(f.s(u, stat::COLDLENGTH), 150);
    assert_eq!(f.s(u, stat::COLDRESIST), 75);
    run_umod_init(&cx, &mut f, u, 23, true);
    assert_eq!(f.s(u, stat::POISONLENGTH), 400);
    run_umod_init(&cx, &mut f, u, 25, true);
    assert_eq!(f.s(u, stat::MANADRAINMINDAM), 6 * 256);
    assert_eq!(f.s(u, stat::MAGICRESIST), 20);
    run_umod_init(&cx, &mut f, u, 17, true);
    assert_eq!(f.s(u, stat::LIGHTMAXDAM), 10);
    // Level clamped to 1.. rows − 1.
    assert_eq!(monlvl_dm(&monlvl(), true, 0, 0), 1);
    assert_eq!(monlvl_dm(&monlvl(), true, 0, 900), 110);
}

#[test]
fn aura_vectors() {
    let seed_for = |n: i32, want: u32| {
        (0u16..)
            .find(|&s| Seed::init_low(u32::from(s)).roll(n) == want)
            .unwrap()
    };
    let s = seed_for(6, 3);
    assert_eq!(aura_choice(5, s, 1, None), (114, 1));
    let s = seed_for(7, 6);
    assert_eq!(aura_choice(40, s, 1, None), (118, 5));
    // Level < 1 counts as 1; Lord De Seis; class 704.
    assert_eq!(aura_choice(0, seed_for(6, 0), 1, None), (98, 1));
    assert_eq!(aura_choice(80, 0, 1, Some(37)), (122, 10));
    assert_eq!(aura_choice(5, 0, 704, None), (123, 20));
    // Through the umod, unit seed untouched.
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let u = f.monster(0, 1);
    f.store.entry(u).name_seed = seed_for(7, 6);
    f.set_stat(u, stat::LEVEL, 40);
    let before = f.seed_of(u);
    let cx = f.cx;
    run_umod_init(&cx, &mut f, u, 30, true);
    assert_eq!(f.log.last().unwrap(), "aura 118 5");
    assert_eq!(f.seed_of(u), before);
}

#[test]
fn champion_types_and_others() {
    let mut t = Tables::new(vec![velocity_mon(6, 1)]);
    t.ids.monteleport = Some(289);
    let mut f = fake_with(t);
    let cx = f.cx;
    // 37 fanatic: armor −70, velocity by the 37 rule.
    let u = f.monster(0, 1);
    run_umod_init(&cx, &mut f, u, 37, true);
    assert_eq!(f.s(u, stat::ITEM_ARMOR_PERCENT), -70);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 75 + 100);
    // 38 possessed.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::MAXHP, 1000);
    f.set_stat(u, stat::HITPOINTS, 1000);
    run_umod_init(&cx, &mut f, u, 38, true);
    assert_eq!((f.s(u, stat::MAXHP), f.s(u, stat::HITPOINTS)), (2000, 2000));
    assert!(f.data(u).has_flag(type_flag::POSSESSED));
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 95);
    // 39 berserk: no champion function.
    let u = f.monster(0, 1);
    f.set_stat(u, stat::MAXHP, 1000);
    f.set_stat(u, stat::HITPOINTS, 1000);
    f.set_stat(u, stat::EXPERIENCE, 100);
    run_umod_init(&cx, &mut f, u, 39, true);
    assert_eq!((f.s(u, stat::MAXHP), f.s(u, stat::HITPOINTS)), (250, 250));
    assert_eq!(f.s(u, stat::DAMAGEPERCENT), 270);
    assert_eq!(f.s(u, stat::ITEM_TOHIT_PERCENT), 270);
    assert_eq!(f.s(u, stat::EXPERIENCE), 100);
    // Both read before the writes: with the max-life rescale on, the
    // Cave Level 1 berserker (12288) ends at 3072 / 3072.
    let w = f.monster(0, 1);
    f.rescale_hp = true;
    f.set_stat(w, stat::MAXHP, 12288);
    f.set_stat(w, stat::HITPOINTS, 12288);
    run_umod_init(&cx, &mut f, w, 39, true);
    assert_eq!((f.s(w, stat::MAXHP), f.s(w, stat::HITPOINTS)), (3072, 3072));
    f.rescale_hp = false;
    // 26 teleport.
    run_umod_init(&cx, &mut f, u, 26, true);
    assert_eq!(f.log.last().unwrap(), "skill 289 1 Some(4)");
    // 41: type-7 event at frame + 75.
    f.game.frame = 10;
    run_umod_init(&cx, &mut f, u, 41, false);
    let t = f.game.timers.unit_timers(u);
    assert_eq!(t.len(), 1);
    // 3, 7, 13: nothing.
    let before = f.stats.clone();
    for umod in [3, 7, 13, 22, 0, 99] {
        run_umod_init(&cx, &mut f, u, umod, true);
    }
    assert_eq!(f.stats, before);
}

// -------------

/// fallenshaman1-like (level 2, HP 5–9, XP 32, Velocity 6) and
/// fallen1-like (level 1, HP 1–4, XP 18).
fn boss_tables() -> Tables {
    let mut shaman = velocity_mon(6, 58);
    shaman.minion1 = 1;
    let fallen = mon(1, 1, 4, 18);
    Tables::new(vec![shaman, fallen])
}

#[test]
fn champion_boss() {
    let mut t = boss_tables();
    t.monumod[0].constants = 100; // always champion
    let mut f = fake_with(t);
    // Not expansion → 36–39 are not eligible: always 16.
    f.info.expansion = false;
    let cx = f.cx;
    let req = CreateRequest {
        class: 0,
        ..CreateRequest::default()
    };
    let u = random_boss(&cx, &mut f, &req, true, true).unwrap();
    let d = f.data(u);
    assert_eq!(d.umod_list(), [16]);
    assert_eq!(
        d.type_flags,
        type_flag::BOSS | type_flag::CHAMPION | type_flag::UNIQUE
    );
    assert_eq!(f.region_bosses, 1);
    // HP: base rolled once at type init.
    let base = f.s(u, stat::MAXHP) / 256 / 3;
    assert!((5..=9).contains(&base));
    assert_eq!(f.s(u, stat::MAXHP), base * 3 * 256);
    assert_eq!(f.s(u, stat::LEVEL), 4);
    assert_eq!(f.s(u, stat::EXPERIENCE), 96);
    assert_eq!(f.s(u, stat::HPREGEN), 0);
    assert_eq!(f.s(u, stat::DAMAGEPERCENT), 90);
    assert_eq!(f.s(u, stat::ITEM_TOHIT_PERCENT), 67);
    assert_eq!(f.s(u, stat::VELOCITYPERCENT), 95);
    assert!(!f.log.iter().any(|l| l.starts_with("minion")));
}

/// The draws of a random boss, replayed from its start seed
/// (Randomness 2, 5, 11).
#[test]
fn random_boss_draw_order() {
    let mut t = boss_tables();
    t.monumod[0].constants = 100;
    let mut f = fake_with(t);
    f.info.expansion = false;
    let cx = f.cx;
    let start = SEEDS[1].wrapping_add(1); // first fresh_seed
    let u = random_boss(&cx, &mut f, &CreateRequest::default(), true, true).unwrap();
    let mut s = Seed::init_low(start);
    s.roll(5); // HP 5..9
    s.roll(100); // champion test
    s.roll(1); // champion pick, total 1
    let name = s.step() as u16;
    assert_eq!(f.data(u).name_seed, name);
    assert_eq!(f.seed_of(u), s);
}

#[test]
fn unique_umod_count() {
    for d in 0..3u8 {
        let mut t = boss_tables();
        t.monumod[0].constants = 0;
        let mut f = fake_with(t);
        f.info.difficulty = d;
        let cx = f.cx;
        let u = f.monster(1, 1);
        choose_umods(&cx, &mut f, u, true);
        let list = f.data(u).umod_list().to_vec();
        assert_eq!(list.len(), usize::from(d) + 1, "d = {d}");
        let mut sorted = list.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len(), "no repeats");
        assert!(list.iter().all(|u| [5, 6, 8, 9, 17, 18].contains(u)));
        assert!(!f.data(u).has_flag(type_flag::CHAMPION));
    }
    // Count limited by the 9 slots; ≥ 8 → nothing; difficulty ≥ 3 → nothing.
    let mut f = fake_with(boss_tables());
    f.info.difficulty = 2;
    let cx = f.cx;
    let u = f.monster(1, 1);
    f.store.entry(u).umods = [13, 14, 15, 20, 21, 22, 33, 0, 0];
    choose_umods(&cx, &mut f, u, false);
    assert_eq!(f.data(u).umod_count(), 9);
    let before = f.seed_of(u);
    choose_umods(&cx, &mut f, u, false);
    assert_eq!(f.seed_of(u), before);
    let v = f.monster(1, 1);
    f.info.difficulty = 3;
    let before = f.seed_of(v);
    choose_umods(&cx, &mut f, v, true);
    assert_eq!(f.seed_of(v), before);
    // Unique pick with no candidates: no step, 0.
    let mut t = boss_tables();
    t.monumod.iter_mut().for_each(|r| r.upick = 0);
    let mut f = fake_with(t);
    let cx = f.cx;
    let u = f.monster(1, 1);
    let before = f.seed_of(u);
    assert_eq!(pick_unique(&cx, &mut f, u, 0, &[]), 0);
    assert_eq!(f.seed_of(u), before);
}

#[test]
fn eligibility() {
    let mut t = boss_tables();
    t.monstats[0].montype = 7;
    t.monstats[0].ismelee = true;
    t.monumod[5].exclude1 = 7;
    t.monumod[6].fpick = 1;
    t.monumod[8].fpick = 2;
    t.monumod[9].fpick = 3;
    t.monumod[17].enabled = 0;
    t.monstats2[0].mwl = true;
    let mut f = fake_with(t);
    let cx = f.cx;
    let e = |f: &mut Fake, id| eligible(&cx, f, 0, id);
    assert!(!e(&mut f, 5), "exclude1");
    assert!(!e(&mut f, 6), "fPick 1 needs A1");
    assert!(!e(&mut f, 8), "fPick 2 melee");
    assert!(e(&mut f, 9), "fPick 3 WL");
    assert!(!e(&mut f, 17), "disabled");
    assert!(e(&mut f, 18));
    f.info.expansion = false;
    assert!(!e(&mut f, 36), "version 100 in classic");
    f.info.expansion = true;
    assert!(e(&mut f, 36));
}

#[test]
fn eligibility_exclude_is_the_matrix_row() {
    // Spec vectors: type 2 has equiv1 1. Class MonType 1, exclude 2 →
    // excluded (2 is a sub-type of 1); MonType 2, exclude 1 → not.
    let mut t = boss_tables();
    t.monumod[5].exclude1 = 2;
    t.monumod[6].exclude1 = 1;
    t.monstats[0].montype = 1;
    t.monstats[1].montype = 2;
    let mut f = fake_with(t);
    f.montype_nest.insert((2, 1));
    let cx = f.cx;
    assert!(!eligible(&cx, &mut f, 0, 5));
    assert!(eligible(&cx, &mut f, 1, 6));
}

#[test]
fn unique_boss_with_minions() {
    let mut t = boss_tables();
    t.monumod[0].constants = 0;
    t.monstats[1].minion1 = 1;
    let mut f = fake_with(t);
    let cx = f.cx;
    let req = CreateRequest {
        class: 1,
        ..CreateRequest::default()
    };
    let u = random_boss(&cx, &mut f, &req, true, false).unwrap();
    let d = f.data(u);
    assert_eq!(d.umod_count(), 1);
    assert_eq!(d.type_flags, type_flag::BOSS | type_flag::UNIQUE);
    assert_eq!(f.s(u, stat::LEVEL), 4);
    assert_eq!(f.s(u, stat::EXPERIENCE), 90);
    let ms = f.minions(u);
    assert!((3..=6).contains(&ms.len()), "{}", ms.len());
    for &m in &ms {
        assert!(f.data(m).has_flag(type_flag::MINION));
        assert_eq!(f.s(m, stat::LEVEL), 4);
        assert_eq!(f.s(m, stat::EXPERIENCE), 90);
        let hp = f.s(m, stat::MAXHP);
        assert!(hp % 512 == 0 && (1..=4).contains(&(hp / 512)), "{hp}");
    }
}

#[test]
fn xfer_counts_visited() {
    let mut f = fake_with(boss_tables());
    let cx = f.cx;
    let b = f.unit(0, 1);
    let m = f.unit(1, 1);
    // Boss [6, 5, 9, 6, 6, 6, 6, 5, 9]: xfer 5 and 9 only.
    f.store.entry(b).umods = [6, 5, 9, 6, 6, 6, 6, 5, 9];
    f.store.entry(m).umods = [13, 14, 15, 0, 0, 0, 0, 0, 0];
    xfer_umods(&cx, &mut f, b, m);
    // 6 slots visited (9 − 3): 5, 9 copied; the 5 and 9 at slots 7, 8 not.
    assert_eq!(f.data(m).umod_list(), [13, 14, 15, 5, 9]);
}

#[test]
fn champion_pack_members() {
    let mut ms = vec![velocity_mon(6, 24)];
    ms[0].baseid = 24;
    let mut f = fake(ms);
    let cx = f.cx;
    let u = create(&cx, &mut f, &CreateRequest::default())
        .unwrap()
        .unwrap();
    let before_seed = f.seed_of(u);
    champion_pack_member(&cx, &mut f, u, 16);
    let d = f.data(u);
    assert_eq!(d.umod_list(), [13, 16]);
    assert_eq!(
        d.type_flags,
        type_flag::BOSS | type_flag::CHAMPION | type_flag::UNIQUE
    );
    let mut s = before_seed;
    assert_eq!(d.name_seed, s.step() as u16);
    assert_eq!(f.seed_of(u), s, "one step: the name seed");
    assert_eq!(f.region_bosses, 1);
    assert_eq!(f.s(u, stat::EXPERIENCE), 96);
    // Already a champion: nothing.
    champion_pack_member(&cx, &mut f, u, 16);
    assert_eq!(f.data(u).umod_list(), [13, 16]);
}

// --------------

fn su_row(class: u32, hc: u32, mods: [u32; 3]) -> Superuniques {
    let mut r: Superuniques = zero();
    r.class = class;
    r.hcidx = hc;
    r.mod1 = mods[0];
    r.mod2 = mods[1];
    r.mod3 = mods[2];
    r
}

#[test]
fn superunique() {
    let mut t = boss_tables();
    t.superuniques = (0..8).map(|i| su_row(0, i, [24, 30, 5])).collect();
    t.superuniques[6] = su_row(0, 6, [5, 0, 30]);
    let mut f = fake_with(t);
    f.info.difficulty = 1;
    let cx = f.cx;
    let u = f.monster(0, 1);
    f.store.entry(u).type_flags |= type_flag::SUPERUNIQUE;
    f.log.clear();
    superunique_init(&cx, &mut f, u, 3, 0, 0);
    let d = f.data(u);
    assert_eq!(d.boss_hc_idx, 3);
    // 24 skipped, 30 and 5, one difficulty pick, then 22.
    assert_eq!(d.umod_list()[..2], [30, 5]);
    assert_eq!(d.umod_count(), 4);
    assert_eq!(*d.umod_list().last().unwrap(), 22);
    assert!(d.has_flag(type_flag::UNIQUE));
    let auras: Vec<&String> = f.log.iter().filter(|l| l.starts_with("aura")).collect();
    assert_eq!(auras.len(), 2);
    assert_eq!(auras[0], auras[1]);
    // hcIdx 3 has no case.
    assert!(!f.log.iter().any(|l| l.starts_with("quest_chain")));
    // Countess: mods stop at the first 0; state 118, chain 5, AI state 13
    // in that order.
    let v = f.monster(0, 1);
    f.log.clear();
    superunique_init(&cx, &mut f, v, 6, 0, 0);
    assert_eq!(f.data(v).umod_list()[0], 5);
    assert!(!f.data(v).has_umod(30));
    let pos = |s: String| f.log.iter().position(|l| *l == s).unwrap();
    let (a, b, c) = (
        pos("state 118".into()),
        pos("quest_chain 5".into()),
        pos(format!("ai_install {} 13", v.0)),
    );
    assert!(a < b && b < c);
}

#[test]
fn superunique_hcidx_cases() {
    let mut t = boss_tables();
    t.superuniques = (0..63).map(|i| su_row(0, i, [0, 0, 0])).collect();
    let mut f = fake_with(t);
    let cx = f.cx;
    // Radament (10): roll(5) + 2 class-4 spawns on the unit seed, then
    // 276, 382, 385, 389.
    let u = f.monster(0, 1);
    f.log.clear();
    superunique_init(&cx, &mut f, u, 10, 0, 0);
    let near: Vec<&String> = f.log.iter().filter(|l| l.starts_with("near")).collect();
    let n = near.len() - 4;
    assert!((2..=6).contains(&n), "{n}");
    assert!(near[..n].iter().all(|l| *l == "near 4 1 4 0x40"));
    assert_eq!(
        near[n..],
        [
            "near 276 1 4 0x40",
            "near 382 1 4 0x40",
            "near 385 1 4 0x40",
            "near 389 1 4 0x40"
        ]
    );
    // Siege boss (42): group, chain 31, preset hook, state 118.
    let v = f.monster(0, 1);
    f.log.clear();
    superunique_init(&cx, &mut f, v, 42, 0, 0);
    let want = [
        "group 453 20 20 0x0",
        "quest_chain 31",
        "preset_boss",
        "state 118",
    ];
    let got: Vec<&String> = f
        .log
        .iter()
        .filter(|l| want.iter().any(|w| l == w))
        .collect();
    assert_eq!(got, want);
    // Nihlathak (60): owner data, the class for the level, chain 34.
    let w = f.monster(0, 1);
    f.log.clear();
    superunique_init(&cx, &mut f, w, 60, 0, 0);
    let i = f.log.iter().position(|l| l == "owner_self").unwrap();
    assert_eq!(f.log[i + 1], "group 454 10 20 0x40");
    assert_eq!(f.log[i + 2], "quest_chain 34");
    // Every case ends with umod 22 (unique).
    assert_eq!(*f.data(w).umod_list().last().unwrap(), 22);
}

#[test]
fn make_unique_and_the_warping_pick() {
    let t = boss_tables();
    let mut f = fake_with(t);
    let cx = f.cx;
    let u = f.monster(0, 1);
    super::make_unique(&cx, &mut f, u);
    let d = f.data(u);
    assert!(d.has_flag(type_flag::BOSS) && d.has_flag(type_flag::UNIQUE));
    assert!(d.data_flag1);
    assert_ne!(f.units.get(u).unwrap().flags & 0x800, 0);
    assert_eq!(f.region_bosses, 1);
    // No minions are spawned (spawn minions 0).
    assert!(!f.log.iter().any(|l| l.starts_with("minion")));
    use super::{nearest_eligible, warp_eligible, WarpCandidate};
    let ok = WarpCandidate {
        monster: true,
        mode: 1,
        has_walk: true,
        m2_byte_0b: Some(1),
        has_data: true,
        ..WarpCandidate::default()
    };
    assert!(warp_eligible(&ok));
    for bad in [
        WarpCandidate {
            is_operator: true,
            ..ok
        },
        WarpCandidate {
            monster: false,
            ..ok
        },
        WarpCandidate { relation: 1, ..ok },
        WarpCandidate { alignment: 2, ..ok },
        WarpCandidate {
            test_63ea40: true,
            ..ok
        },
        WarpCandidate { mode: 3, ..ok },
        WarpCandidate {
            has_walk: false,
            ..ok
        },
        WarpCandidate {
            m2_byte_0b: Some(0),
            ..ok
        },
        WarpCandidate {
            m2_byte_0b: None,
            ..ok
        },
        WarpCandidate {
            has_data: false,
            ..ok
        },
        WarpCandidate { boss: true, ..ok },
        WarpCandidate {
            prime_evil: true,
            ..ok
        },
        WarpCandidate {
            type_flags: 0x10,
            ..ok
        },
    ] {
        assert!(!warp_eligible(&bad), "{bad:?}");
    }
    assert!(warp_eligible(&WarpCandidate {
        mode: 2,
        type_flags: 0x20,
        ..ok
    }));
    // The nearest eligible one; the first of equal distances; limit 0 is
    // 0x10000 but the best starts at 0xFFFF.
    assert_eq!(
        nearest_eligible([(1, 9, true), (2, 5, false), (3, 7, true), (4, 7, true)], 0),
        Some(3)
    );
    assert_eq!(nearest_eligible([(1, 0xFFFF, true)], 0), None);
    assert_eq!(nearest_eligible([(1, 0xFFFE, true)], 0), Some(1));
    assert_eq!(nearest_eligible([(1, 10, true)], 10), None);
}

#[test]
fn class_reinit() {
    let mut ms = vec![mon(1, 1, 1, 1); 4];
    ms[2].interact = true;
    ms[3].enabled = false;
    let mut f = fake_with(Tables::new(ms));
    let cx = f.cx;
    let u = f.monster(2, 1);
    f.store.entry(u).umods[0] = 5;
    f.log.clear();
    // A disabled or out-of-range class changes nothing.
    assert!(!reinit(&cx, &mut f, u, 3, 1));
    assert!(!reinit(&cx, &mut f, u, 4, 1));
    assert!(!reinit(&cx, &mut f, u, -1, 1));
    assert!(f.log.is_empty());
    // The old class (2) is `interact`: the inventory is kept; the class,
    // the type init and the plain mode set follow; umods stay.
    assert!(reinit(&cx, &mut f, u, 1, 3));
    assert_eq!(f.log[0], "teardown false");
    assert_eq!(f.units.get(u).unwrap().class, 1);
    assert_eq!(f.units.get(u).unwrap().mode, 3);
    assert_eq!(f.data(u).class, 1);
    assert_eq!(f.data(u).umod_list(), [5]);
    assert!(reinit(&cx, &mut f, u, 0, 1));
    assert!(f.log.contains(&"teardown true".to_string()));
}

#[test]
fn restore_paths() {
    let mut t = boss_tables();
    t.superuniques = vec![su_row(0, 0, [0, 0, 0]); 3];
    let mut f = fake_with(t);
    let cx = f.cx;
    let saved = Saved {
        umods: [16, 30, 0, 0, 0, 0, 0, 0, 0],
        name_seed: 777,
        champion: true,
        superunique: Some(2),
    };
    let u = restore_boss(&cx, &mut f, &CreateRequest::default(), 55, &saved).unwrap();
    let d = f.data(u);
    assert_eq!(d.name_seed, 777);
    assert_eq!(d.umod_list(), [16, 30]);
    assert_eq!(
        d.type_flags,
        type_flag::BOSS | type_flag::UNIQUE | type_flag::CHAMPION | type_flag::SUPERUNIQUE
    );
    assert_eq!(d.boss_hc_idx, 2);
    assert!(f.log.contains(&"boss_spawn Some(55)".to_string()));
    // Aura twice: in the list run (new name seed) and again (saved seed).
    let auras = f.log.iter().filter(|l| l.starts_with("aura")).count();
    assert_eq!(auras, 2);
    assert!(!f.log.iter().any(|l| l.starts_with("minion")));
    // Minion restore: umods with unique = 0 (no name seed draw).
    let req = CreateRequest {
        class: 1,
        guid: 9,
        flags: 0x62,
        ..CreateRequest::default()
    };
    let saved = Saved {
        umods: [5, 0, 0, 0, 0, 0, 0, 0, 0],
        ..Saved::default()
    };
    let m = restore_minion(&cx, &mut f, &req, &saved).unwrap();
    assert!(f.log.contains(&"spawn_guid 9 0x62".to_string()));
    let d = f.data(m);
    assert_eq!(d.umod_list(), [5]);
    assert_eq!(d.type_flags, type_flag::MINION);
    assert_eq!(d.name_seed, 0);
    assert_eq!(f.s(m, stat::DAMAGEPERCENT), 67);
    assert_eq!(f.s(m, stat::LEVEL), 4);
}

// ---------

#[test]
fn dispatcher_and_event7() {
    let mut f = fake(vec![velocity_mon(0, 1)]);
    let cx = f.cx;
    let u = f.monster(0, 1);
    // Empty list: nothing.
    dispatch(&cx, &mut f, u, None, 1);
    assert!(f.store.unhandled.is_empty());
    // Every slot, even after a 0: two umod-41 runs, each a think restart
    // (mode NU: type 2 cancelled, event 2 at F + 2) and event 7 at F + 75.
    f.store.entry(u).umods = [41, 0, 41, 0, 0, 0, 0, 0, 0];
    handle_event7(&cx, &mut f, u);
    let types = |f: &Fake| -> Vec<u8> {
        let t = &f.game.timers;
        let mut v: Vec<u8> = t
            .unit_timers(u)
            .into_iter()
            .filter_map(|id| t.event(id).map(|e| e.0))
            .collect();
        v.sort_unstable();
        v
    };
    assert_eq!(types(&f), [2, 7, 7]);
    // Dead: the 41 handler does nothing.
    f.units.get_mut(u).unwrap().mode = mode::DEATH;
    f.store.entry(u).umods = [41, 0, 0, 0, 0, 0, 0, 0, 0];
    handle_event7(&cx, &mut f, u);
    assert_eq!(types(&f), [2, 7, 7]);
    // Mode 1, new mode 0: 9 needs unique; 10 does not; 18 needs unique.
    for (umod, unique, n) in [
        (9, false, 0),
        (9, true, 1),
        (10, false, 1),
        (18, false, 0),
        (18, true, 1),
    ] {
        let v = f.monster(0, 1);
        f.units.get_mut(v).unwrap().mode = mode::DEATH;
        f.store.entry(v).umods = [umod, 0, 0, 0, 0, 0, 0, 0, 0];
        if unique {
            f.store.entry(v).type_flags = type_flag::UNIQUE;
        }
        dispatch(&cx, &mut f, v, None, 1);
        assert_eq!(
            f.game.timers.unit_timers(v).len(),
            n,
            "umod {umod} unique {unique}"
        );
    }
    // 17: unique and mode 3.
    let v = f.monster(0, 1);
    f.units.get_mut(v).unwrap().mode = mode::GETHIT;
    f.store.entry(v).umods = [17, 0, 0, 0, 0, 0, 0, 0, 0];
    f.store.entry(v).type_flags = type_flag::UNIQUE;
    dispatch(&cx, &mut f, v, None, 1);
    assert_eq!(f.game.timers.unit_timers(v).len(), 1);
    // Every callback of the table has a body:
    // nothing is recorded as unhandled; mode 5 hands the callbacks the
    // missile (here a monster, which multishot ignores).
    let w = f.monster(0, 1);
    f.store.entry(v).umods = [29, 0, 0, 0, 0, 0, 0, 0, 0];
    dispatch(&cx, &mut f, v, Some(w), 5);
    assert!(f.store.unhandled.is_empty());
}

#[test]
fn ai_after_death_draw() {
    let mut m = velocity_mon(0, 1);
    m.aip8 = 50;
    m.aip1 = 3;
    let mut f = fake(vec![m]);
    let cx = f.cx;
    for &s in &SEEDS {
        let u = f.monster(0, s);
        f.units.get_mut(u).unwrap().mode = mode::DEATH;
        f.store.entry(u).umods = [34, 0, 0, 0, 0, 0, 0, 0, 0];
        f.game
            .schedule_event(u, EVENT_UMOD, 99, None, 0, 0)
            .unwrap();
        let mut r = f.seed_of(u);
        dispatch(&cx, &mut f, u, None, 1);
        let hit = r.step() % 100 < 50;
        assert_eq!(f.seed_of(u), r);
        assert_eq!(
            f.game.timers.unit_timers(u).len(),
            usize::from(hit),
            "seed {s}"
        );
    }
}

// ---- umods.tsv ----

fn check_umods(tsv: &str, table: &[UmodRow]) -> Result<(), String> {
    let mut lines = tsv.lines();
    let header: Vec<&str> = lines.next().ok_or("empty")?.split('\t').collect();
    let col = |n: &str| {
        header
            .iter()
            .position(|h| *h == n)
            .ok_or(format!("no column {n}"))
    };
    let (id, name, init, gate) = (
        col("id")?,
        col("uniquemod")?,
        col("init_fn")?,
        col("unique_gate")?,
    );
    let cbs = [
        col("cb_mode0")?,
        col("cb_mode1")?,
        col("cb_mode2_event7")?,
        col("cb_mode3")?,
        col("cb_mode4")?,
        col("cb_mode5")?,
    ];
    let addr = |s: &str| -> Result<u32, String> {
        if s == "-" {
            return Ok(0);
        }
        u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("{s}: {e}"))
    };
    let mut n = 0;
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        let i: usize = f[id].parse().map_err(|_| format!("bad id {}", f[id]))?;
        let r = table.get(i).ok_or(format!("id {i}: missing"))?;
        let g = match f[gate] {
            "-" => Gate::None,
            "yes" => Gate::Unique,
            "no" => Gate::Any,
            "branch" => Gate::Branch,
            o => return Err(format!("id {i}: gate {o}")),
        };
        let mut want = [0; 6];
        for (w, &c) in want.iter_mut().zip(&cbs) {
            *w = addr(f[c])?;
        }
        if r.id as usize != i
            || r.name != f[name]
            || r.init_fn != addr(f[init])?
            || r.gate != g
            || r.callbacks != want
        {
            return Err(format!("id {i}: {r:?} differs from the TSV"));
        }
        n += 1;
    }
    if n != table.len() {
        return Err(format!("{n} rows in the TSV, {} in the table", table.len()));
    }
    Ok(())
}

#[test]
fn umods_match_tsv() {
    check_umods(UMODS_TSV, &UMODS).unwrap();
    // Every implemented callback is one of the table's
    // (`callbacks::every_table_callback_has_a_body` checks the converse).
    for a in callback::ALL {
        assert!(UMODS.iter().any(|r| r.callbacks.contains(&a)), "{a:#x}");
    }
}

#[test]
fn umods_check_catches_perturbations() {
    let bad = UMODS_TSV.replacen("0x005A0E80", "0x005A0E81", 1);
    let err = check_umods(&bad, &UMODS).unwrap_err();
    assert!(err.starts_with("id 16:"), "{err}");
    let bad = UMODS_TSV.replacen("0x005A3610", "-", 1);
    let err = check_umods(&bad, &UMODS).unwrap_err();
    assert!(err.starts_with("id 29:"), "{err}");
    let bad = UMODS_TSV.replacen("\t0x005A1910\tno\t", "\t0x005A1910\tyes\t", 1);
    let err = check_umods(&bad, &UMODS).unwrap_err();
    assert!(err.starts_with("id 6:"), "{err}");
}

// --------------

#[test]
fn unique_name_draws() {
    for s in [0u16, 8013, 62586, 56351] {
        let mut r = Seed::init_low(u32::from(s));
        let (mut suf, mut pre) = (r.roll(20), r.roll(30));
        let mut app = None;
        if r.roll(100) < 50 {
            app = Some(r.roll(10));
            suf = r.roll(20);
            pre = r.roll(30);
        }
        assert_eq!(
            unique_name(s, 20, 30, 10),
            UniqueName {
                prefix: pre,
                suffix: suf,
                appellation: app
            }
        );
    }
}

#[test]
fn assign_fields() {
    assert_eq!(LIFE_AT_SPAWN, 128);
    assert_eq!(
        [0, 1, 2, 3, 4, 8, 9, 12, 15].map(assign_mode),
        [0, 1, 1, 1, 1, 8, 9, 12, 1]
    );
    assert_eq!(
        [0, 1, 2, 3, 4, 5, 8, 9, 17].map(component_bits),
        [1, 1, 1, 2, 2, 3, 3, 4, 5]
    );
    assert_eq!(components_field(&[0; 16], &[3; 16]), None);
    let mut c = [0u8; 16];
    c[0] = 2;
    let f = components_field(&c, &[3; 16]).unwrap();
    assert_eq!(f[0], (2, 2));
    // One presence bit, then the components (here 16 × 2 bits).
    let mut w = BitWriter::new();
    write_components(&mut w, &[0; 16], &[3; 16]);
    assert_eq!((w.bits, w.bytes.clone()), (1, vec![0]));
    let mut w = BitWriter::new();
    write_components(&mut w, &c, &[3; 16]);
    assert_eq!(w.bits, 1 + 16 * 2);
    // Bit 0 = 1 (present), bits 1–2 = component 0 (2).
    assert_eq!(w.bytes[0] & 0b111, 0b101);
    // Boss section: none without umods.
    let mut w = BitWriter::new();
    assert!(!write_boss_section(&mut w, &MonsterData::default()));
    assert!(w.bytes.is_empty());
    // [13, 16] champion + unique, name seed 56351 (measured brute1).
    let m = MonsterData {
        umods: [13, 16, 0, 0, 0, 0, 0, 0, 0],
        type_flags: type_flag::BOSS | type_flag::CHAMPION | type_flag::UNIQUE,
        name_seed: 56351,
        ..MonsterData::default()
    };
    let mut w = BitWriter::new();
    assert!(write_boss_section(&mut w, &m));
    assert_eq!(w.bits, 5 + 3 * 8 + 16);
    let bit = |i: usize| (w.bytes[i / 8] >> (i % 8)) & 1;
    assert_eq!((0..5).map(bit).collect::<Vec<_>>(), [1, 1, 0, 0, 0]);
    let field = |at: usize, n: usize| (0..n).fold(0u32, |a, i| a | u32::from(bit(at + i)) << i);
    assert_eq!([field(5, 8), field(13, 8), field(21, 8)], [13, 16, 0]);
    assert_eq!(field(29, 16), 56351);
    // Superunique: hcIdx after the flags.
    let m = MonsterData {
        umods: [30, 0, 0, 0, 0, 0, 0, 0, 0],
        type_flags: type_flag::SUPERUNIQUE | type_flag::UNIQUE,
        boss_hc_idx: 0x1234,
        ..MonsterData::default()
    };
    let mut w = BitWriter::new();
    write_boss_section(&mut w, &m);
    let bit = |i: usize| (w.bytes[i / 8] >> (i % 8)) & 1;
    let field = |at: usize, n: usize| (0..n).fold(0u32, |a, i| a | u32::from(bit(at + i)) << i);
    assert_eq!(field(5, 16), 0x1234);
    assert_eq!(field(21, 8), 30);
}

#[test]
fn ordering_of_type_init_draws() {
    // Components, HP, monprop, monequip, in that order, on the unit seed.
    let mut m = mon(3, 1, 10, 1);
    m.monprop = 0;
    let mut p: Monprop = zero();
    p.prop1 = 1;
    p.chance1 = 100;
    p.prop2 = u32::MAX;
    let mut t = Tables::new(vec![m]);
    t.monprop = vec![p];
    t.components = vec![[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]];
    t.monequip = vec![equip(0, 0, 1, &[(b"a   ", 1), (b"b   ", 2)])];
    let mut f = fake_with(t);
    f.inventory = true;
    for &s in &SEEDS {
        let u = f.monster(0, s);
        let mut r = Seed::init_low(s);
        let c0 = r.roll(2) as u8;
        let hp = r.roll(10) as i32 + 1;
        r.step();
        r.roll(2);
        assert_eq!(f.data(u).components[0], c0);
        assert_eq!(f.s(u, stat::MAXHP), hp * 256);
        assert_eq!(f.seed_of(u), r);
    }
}

// ---- Real values (the game's own tables) ----

#[test]
#[ignore = "needs extracted game tables in D2_GAME_DIR"]
fn real_boss_chains_follow_the_live_base_ids() {
    use crate::skills::tests_game as game;
    use d2_data::tables::decode_all;
    let ms: Vec<Monstats> = decode_all(&game::table("monstats", Monstats::SIZE)).unwrap();
    let chains = |c: u32| boss_chains(&ms, c).collect::<Vec<_>>();
    // monstats.txt rows: Andariel 156, Duriel 211, Radament 229, Mephisto
    // 242, Diablo 243, Izual 256, Blood Raven 267, Hephasto 409; the uber
    // rows 704-707 take their base's BaseId, whose uber case links none.
    assert_eq!(chains(156), [6]);
    assert_eq!(chains(211), [13, 9]);
    assert_eq!(chains(229), [8]);
    assert_eq!(chains(242), [20]);
    assert_eq!(chains(243), [23]);
    assert_eq!(chains(256), [22]);
    assert_eq!(chains(267), [2]);
    assert_eq!(chains(409), [24]);
    for uber in [704, 705, 706, 707] {
        assert!(chains(uber).is_empty(), "class {uber}");
    }
}

#[test]
#[ignore = "needs extracted game tables in D2_GAME_DIR"]
fn real_level_stats() {
    use crate::skills::tests_game as game;
    use d2_data::tables::decode_all;
    let ms: Vec<Monstats> = decode_all(&game::table("monstats", Monstats::SIZE)).unwrap();
    let ml: Vec<Monlvl> = decode_all(&game::table("monlvl", Monlvl::SIZE)).unwrap();
    let lv: Vec<Levels> = decode_all(&game::table("levels", Levels::SIZE)).unwrap();
    let x = GameInfo {
        expansion: true,
        game_type: 3,
        ..GameInfo::default()
    };
    // (class, Normal, NM Blood Moor (2), Hell Blood Moor, Hell Cold Plains (3)).
    type Row = (i32, i32, i32, i32, i32);
    let rows: [(u32, Row, Row, Row, Row); 6] = [
        (
            5,
            (1, 7, 12, 5, 33),
            (36, 551, 787, 422, 2696),
            (67, 4317, 6168, 1067, 28069),
            (68, 4438, 6340, 1081, 29496),
        ),
        (
            19,
            (1, 1, 4, 5, 18),
            (36, 131, 288, 369, 1669),
            (67, 1028, 2261, 933, 17376),
            (68, 1056, 2324, 946, 18259),
        ),
        (
            28,
            (2, 11, 19, 10, 48),
            (36, 761, 1102, 448, 3081),
            (67, 5962, 8635, 1133, 32079),
            (68, 6129, 8876, 1149, 33710),
        ),
        (
            58,
            (2, 5, 9, 10, 32),
            (36, 288, 472, 396, 3852),
            (67, 2261, 3700, 1000, 40099),
            (68, 2324, 3804, 1014, 42138),
        ),
        (
            63,
            (1, 1, 5, 5, 21),
            (36, 105, 367, 422, 1797),
            (67, 822, 2878, 1067, 18713),
            (68, 845, 2958, 1081, 19664),
        ),
        (
            165,
            (2, 7, 11, 10, 36),
            (36, 420, 603, 501, 2182),
            (67, 3289, 4728, 1267, 22723),
            (68, 3381, 4861, 1284, 23878),
        ),
    ];
    for (class, n, nm, h2, h3) in rows {
        let m = &ms[class as usize];
        for (d, level_id, want) in [(0, 2, n), (1, 2, nm), (1, 3, nm), (2, 2, h2), (2, 3, h3)] {
            let l = monster_level(m, &lv, &x, d, level_id);
            let s = stats_by_level(m, &ml, true, d, l);
            assert_eq!(
                (l, s.min_hp, s.max_hp, s.ac, s.xp),
                want,
                "class {class} d {d} level {level_id}"
            );
        }
    }
    // L-flag 0: zombie1 Hell Blood Moor.
    let l = monster_level(&ms[5], &lv, &x, 2, 2);
    let s = stats_by_level(&ms[5], &ml, false, 2, l);
    assert_eq!((s.min_hp, s.max_hp, s.ac, s.xp), (3238, 4626, 907, 28069));
}

#[path = "tests_c2mon.rs"]
mod c2mon;
mod callbacks;
mod find;
mod mod_umods;
mod spawn_tools;
mod umod_init_bodies;

// Tests written against surviving mutants; a child module so
// they share this module's fakes.
#[path = "../mutant_tests/init_seams.rs"]
mod mutant_seams;
#[path = "../mutant_tests/init.rs"]
mod mutant_tests;
#[path = "../mutant_tests/umods.rs"]
mod mutant_umods;
