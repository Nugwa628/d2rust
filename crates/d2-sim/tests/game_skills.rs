//! Checks D2RUST skill, missile and vitals APIs against expected table
//! columns and vectors, then sweeps loaded rows for formula, event,
//! missile and level-up constraints. The player's extracted tables come
//! from `D2_GAME_DIR` through `game_common`; tests are ignored by default.
//!
//! Skill ids identify `skills.txt` rows. Vector tests check relevant
//! columns before comparing calculated values.

mod game_common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use d2_data::tables::{Charstats, Experience, Missiles, Skilldesc, Skills, States};
use d2_sim::combat::vitals::{
    self, gain_energy, handle_add_stat_point, init_player_stats, level_up, VitalsTables,
    VitalsUnits,
};
use d2_sim::missiles::catalogue::{SRVDO_TSV, SRVHIT_TSV, SRV_DO, SRV_HIT};
use d2_sim::missiles::{
    creation_velocity, MissileParams, RowExt, SRV_DMG_COUNT, SRV_DO_COUNT, SRV_HIT_COUNT,
};
use d2_sim::rng::Seed;
use d2_sim::skills::special::{MISSCALC_CODES, SKILLCALC_CODES};
use d2_sim::skills::use_::table::{lookup, Kind, Status, FUNCS};
use d2_sim::skills::{
    elem_len, elem_max, elem_min, eval_missile, eval_skill, mana_cost, max_level, miss_elem_len,
    miss_elem_max, miss_elem_min, miss_phys_max, miss_phys_min, miss_special, phys_max, phys_min,
    special, to_hit, SkillEntry, SkillTables, SkillUnits, LEVEL_CAP_114D, NO_CALC,
};
use d2_sim::units::UnitType;

use game_common::{read, rows, tsv_rows};

// skills.txt rows.
const CRITICAL_STRIKE: i32 = 9;
const MULTIPLE_SHOT: i32 = 12;
const DODGE: i32 = 13;
const FIRE_BOLT: i32 = 36;
const FIRE_BALL: i32 = 47;
const FIRE_WALL: i32 = 51;
const TELEPORT: i32 = 54;
const METEOR: i32 = 56;
const THUNDER_STORM: i32 = 57;
const BLIZZARD: i32 = 59;
const HYDRA: i32 = 62;
const FROZEN_ORB: i32 = 64;
const AMPLIFY_DAMAGE: i32 = 66;
const LOWER_RESIST: i32 = 91;
const SACRIFICE: i32 = 96;
const MIGHT: i32 = 98;
const BASH: i32 = 126;
const TORNADO: i32 = 245;
const BLADE_SHIELD: i32 = 277;

// missiles.txt rows.
const SPIKE1: usize = 7;
const SHAFIRE1: usize = 22;
const FIREBOLT_MISSILE: usize = 58;
const ROGUE1: usize = 120;

/// Levels the sweeps evaluate (0 and the skill-level cap included).
const SWEEP_LEVELS: std::ops::RangeInclusive<i32> = 0..=LEVEL_CAP_114D;

fn tables() -> &'static SkillTables {
    static T: OnceLock<SkillTables> = OnceLock::new();
    T.get_or_init(|| SkillTables {
        skills: rows::<Skills>(),
        skilldesc: rows::<Skilldesc>(),
        missiles: rows::<Missiles>(),
        skills_code: read("skillscode.bin"),
        miss_code: read("misscode.bin"),
        level_cap: LEVEL_CAP_114D,
        stat_count: game_common::table("itemstatcost", d2_data::tables::Itemstatcost::SIZE).count
            as i32,
    })
}

use d2_data::tables::Record;

fn s16(v: u16) -> i32 {
    i32::from(v as i16)
}

fn skill(id: i32) -> &'static Skills {
    &tables().skills[id as usize]
}

fn code(codes: &[&str], name: &str) -> u8 {
    codes.iter().position(|c| *c == name).expect("code") as u8
}

// ------------------------------------------------------------ unit seam

/// A unit with no stats, items, states or skills: every read is 0 / none.
/// `Some(0)` is a level-1 player of class 0 (Amazon), `Some(1)` a missile.
#[derive(Default)]
struct Bare {
    seed: Seed,
}

impl SkillUnits for Bare {
    type Unit = u8;
    type Item = u8;
    fn unit_type(&self, u: u8) -> UnitType {
        if u == 1 {
            UnitType::Missile
        } else {
            UnitType::Player
        }
    }
    fn class_id(&self, _: u8) -> i32 {
        0
    }
    fn stat(&self, _: u8, _: u16, _: u16) -> i32 {
        0
    }
    fn item_stat(&self, _: u8, _: u16, _: u16) -> i32 {
        0
    }
    fn base_stat(&self, _: u8, _: u16, _: u16) -> i32 {
        0
    }
    fn formula_stat(&self, _: u8, _: u16, _: i32) -> i32 {
        0
    }
    fn stat_entries(&self, _: u8, _: u16, _: usize) -> Vec<(u16, i32)> {
        Vec::new()
    }
    fn has_state(&self, _: u8, _: u16) -> bool {
        false
    }
    fn state_stat(&self, _: u8, _: u16, _: u16) -> Option<i32> {
        None
    }
    fn seed(&mut self, _: u8) -> &mut Seed {
        &mut self.seed
    }
    fn skill_list(&self, _: u8) -> Vec<SkillEntry> {
        Vec::new()
    }
    fn used_skill(&self, _: u8) -> Option<SkillEntry> {
        None
    }
    fn current_weapon(&self, _: u8) -> Option<u8> {
        None
    }
    fn weapon(&self, _: u8) -> Option<u8> {
        None
    }
    fn item_at(&self, _: u8, _: u8) -> Option<u8> {
        None
    }
    fn item_is(&self, _: u8, _: i32) -> bool {
        false
    }
    fn itype_is(&self, _: i32, _: i32) -> bool {
        false
    }
    fn wield_type(&self, _: u8) -> i32 {
        0
    }
    fn item_damage(&self, _: u8, _: bool) -> i32 {
        0
    }
    fn str_dex_bonus(&self, _: u8) -> (i32, i32) {
        (0, 0)
    }
    fn item_flag_throw(&self, _: u8) -> bool {
        false
    }
    fn missile_level(&self, _: u8) -> i32 {
        0
    }
}

/// Every `skillscode` formula field of a skills record.
fn skill_calcs(s: &Skills) -> [u32; 36] {
    [
        s.prgcalc1,
        s.prgcalc2,
        s.prgcalc3,
        s.auralencalc,
        s.aurarangecalc,
        s.aurastatcalc1,
        s.aurastatcalc2,
        s.aurastatcalc3,
        s.aurastatcalc4,
        s.aurastatcalc5,
        s.aurastatcalc6,
        s.passivecalc1,
        s.passivecalc2,
        s.passivecalc3,
        s.passivecalc4,
        s.passivecalc5,
        s.petmax,
        s.sumsk1calc,
        s.sumsk2calc,
        s.sumsk3calc,
        s.sumsk4calc,
        s.sumsk5calc,
        s.cltcalc1,
        s.cltcalc2,
        s.cltcalc3,
        s.perdelay,
        s.skpoints,
        s.delay,
        s.calc1,
        s.calc2,
        s.calc3,
        s.calc4,
        s.tohitcalc,
        s.dmgsympercalc,
        s.edmgsympercalc,
        s.elensympercalc,
    ]
}

/// Every `misscode` formula field of a missiles record.
fn miss_calcs(m: &Missiles) -> [u32; 7] {
    [
        m.srvcalc1,
        m.cltcalc1,
        m.shitcalc1,
        m.chitcalc1,
        m.dmgcalc1,
        m.dmgsympercalc,
        m.edmgsympercalc,
    ]
}

// ================================================================ level calculations

/// Every formula field of every skills row, at every level 0..=99, with no
/// unit and with a bare player: no panic; an unset field (−1) gives 0.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_skill_formula_evaluates() {
    let t = tables();
    let mut w = Bare::default();
    for (id, s) in t.skills.iter().enumerate() {
        for field in skill_calcs(s) {
            for lvl in SWEEP_LEVELS {
                let none = eval_skill(&mut w, t, None, field, id as i32, lvl);
                eval_skill(&mut w, t, Some(0), field, id as i32, lvl);
                if field == NO_CALC {
                    assert_eq!(none, 0, "skill {id} lvl {lvl}");
                }
            }
        }
    }
}

/// Every formula field of every missiles row, at every level 0..=99, with
/// no unit and with a bare missile and owner: no panic; an unset field
/// gives 0.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_missile_formula_evaluates() {
    let t = tables();
    let mut w = Bare::default();
    for (id, m) in t.missiles.iter().enumerate() {
        for field in miss_calcs(m) {
            for lvl in SWEEP_LEVELS {
                let none = eval_missile(&mut w, t, None, None, field, id as i32, lvl);
                eval_missile(&mut w, t, Some(1), Some(0), field, id as i32, lvl);
                if field == NO_CALC {
                    assert_eq!(none, 0, "missile {id} lvl {lvl}");
                }
            }
        }
    }
}

/// Every special-value code (`skillcalc.tsv`, `misscalc.tsv`) and every
/// damage, length, to-hit and mana function on every skills and missiles
/// row at every level 0..=99, no unit: no panic. The shifted mana cost is
/// never negative.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_skill_value_evaluates() {
    let t = tables();
    let mut w = Bare::default();
    for id in 0..t.skills.len() as i32 {
        let rec = skill(id);
        assert!(max_level(rec) > 0, "skill {id}");
        for lvl in SWEEP_LEVELS {
            for c in 0..SKILLCALC_CODES.len() as u8 {
                special(&mut w, t, None, c, id, lvl);
            }
            elem_min(&mut w, t, None, id, lvl, false);
            elem_max(&mut w, t, None, id, lvl, false);
            elem_len(&mut w, t, None, id, lvl);
            phys_min(&mut w, t, None, id, lvl, false);
            phys_max(&mut w, t, None, id, lvl, false);
            to_hit(&mut w, t, None, id, lvl);
            mana_cost(rec, lvl);
            assert!(d2_sim::skills::mana_cost_shifted(t, id, lvl) >= 0);
        }
    }
    for id in 0..t.missiles.len() as i32 {
        for lvl in SWEEP_LEVELS {
            for c in 0..MISSCALC_CODES.len() as u8 {
                miss_special(&mut w, t, None, None, c, id, lvl);
            }
            miss_phys_min(&mut w, t, None, None, id, lvl);
            miss_phys_max(&mut w, t, None, None, id, lvl);
            miss_elem_min(&mut w, t, None, None, id, lvl);
            miss_elem_max(&mut w, t, None, None, id, lvl);
            miss_elem_len(&mut w, t, None, id, lvl);
        }
    }
}

/// Additional vectors for Tornado physical damage, the dm / ln specials,
/// Sacrifice to-hit, the `usmc` / `mana` specials of Fire Bolt and Teleport.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_level_vectors() {
    let t = tables();
    let mut w = Bare::default();
    let mut sp = |name: &str, id: i32, lvl: i32| {
        special(&mut w, t, None, code(&SKILLCALC_CODES, name), id, lvl)
    };
    let p = |id: i32| {
        let s = skill(id);
        [s.param1, s.param2, s.param3, s.param4, s.param5, s.param6].map(|v| v as i32)
    };
    // dm12 Critical Strike (5, 80).
    assert_eq!(p(CRITICAL_STRIKE)[..2], [5, 80]);
    assert_eq!(
        [1, 2, 5, 10, 20, 30, 99].map(|l| sp("dm12", CRITICAL_STRIKE, l)),
        [16, 25, 42, 56, 68, 73, 80]
    );
    // dm12 Dodge (10, 65).
    assert_eq!(p(DODGE)[..2], [10, 65]);
    assert_eq!([1, 20].map(|l| sp("dm12", DODGE, l)), [18, 56]);
    // dm56 Lower Resist (25, 70).
    assert_eq!(p(LOWER_RESIST)[4..6], [25, 70]);
    assert_eq!([1, 20].map(|l| sp("dm56", LOWER_RESIST, l)), [31, 62]);
    // ln34 Amplify Damage (200, 75).
    assert_eq!(p(AMPLIFY_DAMAGE)[2..4], [200, 75]);
    assert_eq!([1, 10].map(|l| sp("ln34", AMPLIFY_DAMAGE, l)), [200, 875]);
    // ln12 Bash (50, 5).
    assert_eq!(p(BASH)[..2], [50, 5]);
    assert_eq!([1, 20].map(|l| sp("ln12", BASH, l)), [50, 145]);
    // Mana specials.
    assert_eq!(sp("mana", FIRE_BOLT, 1), 2);
    assert_eq!([25, 30].map(|l| sp("usmc", TELEPORT, l)), [0, -1_280]);
    assert_eq!(sp("mana", TELEPORT, 30), -5);
    // Sacrifice to-hit (ToHit 20, LevToHit 7).
    let s = skill(SACRIFICE);
    assert_eq!((s.tohit, s.levtohit), (20, 7));
    let mut w = Bare::default();
    assert_eq!(
        [1, 20].map(|l| to_hit(&mut w, t, None, SACRIFICE, l)),
        [20, 153]
    );
    // Tornado physical damage.
    let mm = |w: &mut Bare, l: i32| {
        (
            phys_min(w, t, None, TORNADO, l, false),
            phys_max(w, t, None, TORNADO, l, false),
        )
    };
    assert_eq!(mm(&mut w, 1), (6_400, 8_960));
    assert_eq!(mm(&mut w, 20), (69_888, 75_520));
}

/// The skills table has 357 rows; formula columns are set in EDmgSymPerCalc
/// 64 rows, DmgSymPerCalc 6, ELenSymPerCalc 4, ToHitCalc 3, skpoints 0;
/// SrcDam in 63; HitShift 8 in 316 rows and 7 in 20; 7 negative
/// `lvlmana`. Selected mana columns match the vectors below.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_skills_table_facts() {
    let t = tables();
    let sk = &t.skills;
    assert_eq!(sk.len(), 357);
    let n = |f: fn(&Skills) -> bool| sk.iter().filter(|s| f(s)).count();
    assert_eq!(n(|s| s.edmgsympercalc != NO_CALC), 64);
    assert_eq!(n(|s| s.dmgsympercalc != NO_CALC), 6);
    assert_eq!(n(|s| s.elensympercalc != NO_CALC), 4);
    assert_eq!(n(|s| s.tohitcalc != NO_CALC), 3);
    assert_eq!(n(|s| s.skpoints != NO_CALC), 0);
    assert_eq!(n(|s| s.srcdam != 0), 63);
    assert_eq!(n(|s| s.hitshift == 8), 316);
    assert_eq!(n(|s| s.hitshift == 7), 20);
    assert_eq!(n(|s| (s.lvlmana as i16) < 0), 7);
    // (mana, lvlmana, manashift) for the selected skills.
    let mana = |id: i32| {
        let s = skill(id);
        (s16(s.mana), s16(s.lvlmana), s16(s.manashift))
    };
    assert_eq!(mana(FIRE_BOLT), (5, 0, 7));
    assert_eq!(mana(FROZEN_ORB), (50, 1, 7));
    assert_eq!(mana(TELEPORT), (24, -1, 8));
    assert_eq!(skill(FIRE_BOLT).minmana, 1);
    assert_eq!(
        [1, 20, 25, 30].map(|l| mana_cost(skill(TELEPORT), l)),
        [6_144, 1_280, 0, -1_280]
    );
}

// ================================================================ skill use

/// Every skills row's `srvstfunc`, `srvdofunc`, `srvprgfunc1–3` and
/// `ItemEffect`, and every states row's `srvactivefunc`, names a filled
/// slot of `table::FUNCS` (or 0); the filled slots referenced are exactly
/// entries marked SpecdHere or Mapped in the catalogue. Entries marked
/// Unreferenced must not be used by any loaded row.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_skill_function_in_table() {
    let t = tables();
    let states: Vec<States> = rows();
    let mut used: BTreeSet<(u8, u16)> = BTreeSet::new();
    let mut add = |kind: Kind, v: u16, what: String| {
        let i = v as i16;
        if i <= 0 {
            return;
        }
        let f =
            lookup(kind, i as u16).unwrap_or_else(|| panic!("{what}: {} {i} empty", kind.code()));
        used.insert((u8::from(kind == Kind::Do), f.index));
    };
    for (id, s) in t.skills.iter().enumerate() {
        add(Kind::Start, s.srvstfunc, format!("skill {id} srvstfunc"));
        add(Kind::Do, s.srvdofunc, format!("skill {id} srvdofunc"));
        for (k, v) in [s.srvprgfunc1, s.srvprgfunc2, s.srvprgfunc3]
            .into_iter()
            .enumerate()
        {
            add(Kind::Do, v, format!("skill {id} srvprgfunc{}", k + 1));
        }
        add(Kind::Do, s.itemeffect, format!("skill {id} ItemEffect"));
    }
    for (id, s) in states.iter().enumerate() {
        add(
            Kind::Do,
            s.srvactivefunc,
            format!("state {id} srvactivefunc"),
        );
    }
    let mapped: BTreeSet<(u8, u16)> = FUNCS
        .iter()
        .filter(|f| matches!(f.status, Status::SpecdHere | Status::Mapped))
        .map(|f| (u8::from(f.kind == Kind::Do), f.index))
        .collect();
    assert_eq!(used, mapped);
    let unreferenced: Vec<u16> = FUNCS
        .iter()
        .filter(|f| f.status == Status::Unreferenced)
        .map(|f| f.index)
        .collect();
    assert_eq!(unreferenced, [53, 138, 142]);
}

/// Expected flag counts: InGame on every row; interrupt 335; InTown 63; aura
/// 26; passive 26; AttackNoMana 41; TargetableOnly 45; immediate 10;
/// periodic 2 (Thunder Storm, Blade Shield); the `delay` of Frozen Orb 25,
/// Fire Wall 35, Meteor 30, Blizzard 45, Hydra 40; Might `perdelay` 50 and
/// immediate; Multiple Shot srvst 4.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_use_table_facts() {
    let t = tables();
    let sk = &t.skills;
    let n = |f: fn(&Skills) -> bool| sk.iter().filter(|s| f(s)).count();
    assert_eq!(n(|s| s.ingame), sk.len());
    assert_eq!(n(|s| s.interrupt), 335);
    assert_eq!(n(|s| s.intown), 63);
    assert_eq!(n(|s| s.aura), 26);
    assert_eq!(n(|s| s.passive), 26);
    assert_eq!(n(|s| s.attacknomana), 41);
    assert_eq!(n(|s| s.targetableonly), 45);
    assert_eq!(n(|s| s.immediate), 10);
    let periodic: Vec<i32> = (0..sk.len() as i32)
        .filter(|&i| skill(i).periodic)
        .collect();
    assert_eq!(periodic, [THUNDER_STORM, BLADE_SHIELD]);
    let mut w = Bare::default();
    for (id, delay) in [
        (FROZEN_ORB, 25),
        (FIRE_WALL, 35),
        (METEOR, 30),
        (BLIZZARD, 45),
        (HYDRA, 40),
    ] {
        let got = eval_skill(&mut w, t, None, skill(id).delay, id, 1);
        assert_eq!(got, delay, "skill {id}");
    }
    assert!(skill(MIGHT).immediate);
    assert_eq!(
        eval_skill(&mut w, t, None, skill(MIGHT).perdelay, MIGHT, 1),
        50
    );
}

/// Skill-use vectors on loaded rows: Fire Bolt (5, 0, shift 7,
/// minmana 1, no start / do function, srvmissile firebolt), Fire Ball L10
/// 2,432, Teleport 6,144 / 1,280 / 0 / −1,280, Multiple Shot L10 (srvst 4;
/// 4, +1, shift 8) 3,328.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_use_vectors() {
    let fb = skill(FIRE_BOLT);
    assert_eq!((fb.srvstfunc, fb.srvdofunc), (0, 0));
    assert_eq!(usize::from(fb.srvmissile), FIREBOLT_MISSILE);
    assert_eq!(mana_cost(fb, 1), 640);
    assert_eq!(mana_cost(skill(FIRE_BALL), 10), 2_432);
    let ms = skill(MULTIPLE_SHOT);
    assert_eq!(ms.srvstfunc, 4);
    assert_eq!(
        (s16(ms.mana), s16(ms.lvlmana), s16(ms.manashift)),
        (4, 1, 8)
    );
    assert_eq!(mana_cost(ms, 10), 3_328);
}

// ================================================================ missiles

/// `missiles.bin`: 684 records of 420 bytes, the count stored at offset 0
/// in the file header; row 568's `pSrvHitFunc` holds 0xFDB4, read signed
/// as no server-hit function.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_missiles_record_layout() {
    let raw = game_common::raw::<Missiles>();
    assert_eq!((raw.count, raw.record_size), (684, 420));
    let bytes = read("missiles.bin");
    assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 684);
    let m = &tables().missiles;
    assert_eq!(m.len(), 684);
    assert_eq!(m[568].psrvhitfunc, 0xFDB4);
    assert!(m[568].srv_hit() <= 0);
}

/// The low 16 bits of each raw flags dword match the typed flag columns
/// in the order enumerated below.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_missile_flags_dword() {
    let raw = game_common::raw::<Missiles>();
    for (id, (r, m)) in raw.iter().zip(&tables().missiles).enumerate() {
        let f = u32::from_le_bytes(r[4..8].try_into().unwrap());
        let bits = [
            m.lastcollide,
            m.explosion,
            m.pierce,
            m.canslow,
            m.candestroy,
            m.clientsend,
            m.gethit,
            m.softhit,
            m.applymastery,
            m.returnfire,
            m.town,
            m.srctown,
            m.nomultishot,
            m.nouniquemod,
            m.half2hsrc,
            m.missileskill,
        ];
        for (n, b) in bits.into_iter().enumerate() {
            assert_eq!(f & (1 << n) != 0, b, "missile {id} bit {n}");
        }
    }
}

/// Every missiles row's server functions are in the catalogues: server-do
/// in 0..53 and never a null slot; server-hit ≤ 0 (none) or a filled slot
/// of 1..71; server-damage in
/// 0..31 and 0 or a filled slot. Per-index row counts equal
/// the `rows` column of `srvdo.tsv` / `srvhit.tsv` (numeric cells only:
/// row 568 is not counted); usage server-do 1: 551, 0: 53; server-hit 0:
/// 591.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_missile_function_in_catalogue() {
    let m = &tables().missiles;
    let mut do_rows: BTreeMap<i16, usize> = BTreeMap::new();
    let mut hit_rows: BTreeMap<i16, usize> = BTreeMap::new();
    for (id, r) in m.iter().enumerate() {
        let d = r.srv_do();
        assert!(
            (0..SRV_DO_COUNT).contains(&d),
            "missile {id}: server-do {d}"
        );
        assert!(
            d == 0 || SRV_DO[d as usize].is_some(),
            "missile {id}: null server-do {d}"
        );
        *do_rows.entry(d).or_default() += 1;
        let h = r.srv_hit();
        if h >= 0 {
            assert!(h < SRV_HIT_COUNT, "missile {id}: server-hit {h}");
            assert!(
                h == 0 || SRV_HIT[h as usize].is_some(),
                "missile {id}: null server-hit {h}"
            );
            *hit_rows.entry(h).or_default() += 1;
        } else {
            assert_eq!(id, 568, "missile {id}: negative server-hit {h}");
        }
        let g = r.srv_dmg();
        assert!(
            (0..SRV_DMG_COUNT).contains(&g),
            "missile {id}: server-damage {g}"
        );
        assert!(g <= 14, "missile {id}: null server-damage {g}");
    }
    for (tsv, got) in [(SRVDO_TSV, &do_rows), (SRVHIT_TSV, &hit_rows)] {
        for row in tsv_rows(tsv) {
            let i: i16 = row[0].parse().unwrap();
            let n: usize = row[3].parse().unwrap();
            assert_eq!(
                got.get(&i).copied().unwrap_or(0),
                n,
                "index {i} ({})",
                row[2]
            );
        }
    }
    assert_eq!((do_rows[&1], do_rows[&0], hit_rows[&0]), (551, 53, 591));
}

/// Four missile rows share server-do 1, server-hit 0,
/// server-damage 0, CollideType 3, CollideKill 1, LastCollide, Size 1,
/// Activate 0, Accel 0, LevRange 0, and the per-row columns of the table;
/// their creation speeds and lifetime formula
/// `Range + level × LevRange` at the tested levels.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_recorded_missiles() {
    let m = &tables().missiles;
    let raw = game_common::raw::<Missiles>();
    // (row, Vel, VelLev, MaxVel, Range, flags dword, ToHit, AlwaysExplode,
    // Pierce, speed at level 0).
    let rows = [
        (SPIKE1, 10, 8, 10, 40, 0x249, 1, 0, false, 1_920),
        (SHAFIRE1, 8, 0, 8, 40, 0x24D, 0, 0, true, 1_536),
        (FIREBOLT_MISSILE, 20, 0, 20, 50, 0x249, 0, 0, false, 3_840),
        (ROGUE1, 24, 0, 24, 40, 0x24D, 1, 1, true, 4_608),
    ];
    for (row, vel, vellev, maxvel, range, flags, tohit, always, pierce, v) in rows {
        let r = &m[row];
        let f = u32::from_le_bytes(raw.record(row)[4..8].try_into().unwrap());
        assert_eq!(
            (r.srv_do(), r.srv_hit(), r.srv_dmg()),
            (1, 0, 0),
            "missile {row}"
        );
        assert_eq!(
            (
                r.collidetype,
                r.collidekill,
                r.lastcollide,
                r.size,
                r.activate
            ),
            (3, 1, true, 1, 0),
            "missile {row}"
        );
        assert_eq!((r.accel_i16(), r.lev_range_i16()), (0, 0), "missile {row}");
        assert_eq!(
            (r.vel, r.vellev, r.maxvel, r.range_i16()),
            (vel, vellev, maxvel, range),
            "missile {row}"
        );
        assert_eq!(f, flags, "missile {row}");
        assert_eq!(
            (r.tohit, r.alwaysexplode, r.pierce),
            (tohit, always, pierce),
            "missile {row}"
        );
        assert!(!r.explosion, "missile {row}");
        let p = MissileParams {
            class: row as i32,
            ..MissileParams::default()
        };
        assert_eq!(creation_velocity(r, &p, None), v, "missile {row}");
        // Lifetime in runs, any level (LevRange 0).
        for level in [0, 1, 20] {
            let n = i32::from(r.range_i16()) + level * i32::from(r.lev_range_i16());
            assert_eq!(n, i32::from(range), "missile {row}");
        }
    }
    // spike1 at a skill level: ((10 + level × 8 / 8) << 8) × 75 / 100.
    for level in [1, 3, 10] {
        let p = MissileParams {
            class: SPIKE1 as i32,
            level,
            ..MissileParams::default()
        };
        assert_eq!(
            creation_velocity(&m[SPIKE1], &p, None),
            ((10 + level) << 8) * 75 / 100
        );
    }
    // Damage columns.
    assert_eq!((m[SPIKE1].mindamage, m[SPIKE1].maxdamage), (1, 2));
    assert_eq!((m[SHAFIRE1].emin, m[SHAFIRE1].emax), (1, 4));
    assert_eq!((m[ROGUE1].mindamage, m[ROGUE1].maxdamage), (1, 1));
}

/// The creation speed of every missile row at every level 0..=99
/// evaluates without panic: v = ((Vel + level × VelLev / 8) << 8) ×
/// 75 / 100.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_missile_creation_velocity() {
    for (id, r) in tables().missiles.iter().enumerate() {
        for level in SWEEP_LEVELS {
            let p = MissileParams {
                class: id as i32,
                level,
                ..MissileParams::default()
            };
            let raw = (i32::from(r.vel) + level * i32::from(r.vellev) / 8) << 8;
            let want = raw * 75 / 100;
            assert_eq!(
                creation_velocity(r, &p, None),
                want,
                "missile {id} level {level}"
            );
        }
    }
}

// ================================================================ player vitals

/// A player with base stats only (no items or states): the unit getter and
/// the maxima read the base list.
struct Player {
    class: i32,
    stats: BTreeMap<u16, i32>,
}

impl Player {
    fn new(class: i32) -> Self {
        Self {
            class,
            stats: BTreeMap::new(),
        }
    }
    fn s(&self, s: u16) -> i32 {
        self.stats.get(&s).copied().unwrap_or(0)
    }
}

impl VitalsUnits for Player {
    type Unit = ();
    fn unit_type(&self, _: ()) -> UnitType {
        UnitType::Player
    }
    fn class_id(&self, _: ()) -> i32 {
        self.class
    }
    fn base_stat(&self, _: (), stat: u16) -> i32 {
        self.s(stat)
    }
    fn stat(&self, _: (), stat: u16) -> i32 {
        self.s(stat)
    }
    fn set_base_stat(&mut self, _: (), stat: u16, v: i32) {
        self.stats.insert(stat, v);
    }
    fn add_base_stat(&mut self, _: (), stat: u16, v: i32) {
        *self.stats.entry(stat).or_default() += v;
    }
    fn max_life(&self, _: ()) -> i32 {
        self.s(vitals::stat::MAXHP)
    }
    fn max_mana(&self, _: ()) -> i32 {
        self.s(vitals::stat::MAXMANA)
    }
    fn max_stamina(&self, _: ()) -> i32 {
        self.s(vitals::stat::MAXSTAMINA)
    }
    fn refresh(&mut self, _: ()) {}
    fn level_up_notify(&mut self, _: ()) {}
    fn level_up_event(&mut self, _: ()) {}
}

fn vitals_tables() -> &'static VitalsTables {
    static T: OnceLock<VitalsTables> = OnceLock::new();
    T.get_or_init(|| VitalsTables {
        charstats: rows::<Charstats>(),
        experience: rows::<Experience>(),
    })
}

const SORCERESS: i32 = 1;
const BARBARIAN: i32 = 4;

/// Checks charstats columns for all seven classes and
/// `experience.txt` MaxLvl 99, level 1 → 500, level 99 → 3,837,739,017.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_charstats_and_experience() {
    let t = vitals_tables();
    // str dex int vit, stamina, hpadd, Life/Lvl, Stam/Lvl, Mana/Lvl,
    // Life/Vit, Stam/Vit, Mana/Magic, StatPerLevel.
    let want: [[u8; 13]; 7] = [
        [20, 25, 15, 20, 84, 30, 8, 4, 6, 12, 4, 6, 5],
        [10, 25, 35, 10, 74, 30, 4, 4, 8, 8, 4, 8, 5],
        [15, 25, 25, 15, 79, 30, 6, 4, 8, 8, 4, 8, 5],
        [25, 20, 15, 25, 89, 30, 8, 4, 6, 12, 4, 6, 5],
        [30, 20, 10, 25, 92, 30, 8, 4, 4, 16, 4, 4, 5],
        [15, 20, 20, 25, 84, 30, 6, 4, 8, 8, 4, 8, 5],
        [20, 20, 25, 20, 95, 30, 8, 5, 6, 12, 5, 7, 5],
    ];
    for (class, w) in want.iter().enumerate() {
        let c = t.charstats(class as i32).unwrap();
        let got = [
            c.str,
            c.dex,
            c.int,
            c.vit,
            c.stamina,
            c.hpadd,
            c.lifeperlevel,
            c.staminaperlevel,
            c.manaperlevel,
            c.lifepervitality,
            c.staminapervitality,
            c.manapermagic,
            c.statperlevel,
        ];
        assert_eq!(&got, w, "class {class}");
        assert_eq!(t.max_level(class as i32), 99, "class {class}");
        assert_eq!(t.threshold(class as i32, 1), 500, "class {class}");
        assert_eq!(
            t.threshold(class as i32, 99),
            3_837_739_017,
            "class {class}"
        );
    }
    assert_eq!(t.level_from_exp(0, 499), 1);
    assert_eq!(t.level_from_exp(0, 500), 2);
}

/// Checks creation values of the Sorceress and the
/// Barbarian, level 1 → 10, +10 vitality, +10 energy.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_vitals_vectors() {
    use vitals::stat::*;
    let t = vitals_tables();
    // (class, created life / mana / stamina, after level 10).
    for (class, created, lvl10) in [
        (SORCERESS, [10_240, 8_960, 18_944], [12_544, 13_568, 21_248]),
        (BARBARIAN, [14_080, 2_560, 23_552], [18_688, 4_864, 25_856]),
    ] {
        let mut p = Player::new(class);
        init_player_stats(&mut p, t, (), 0);
        assert_eq!(
            [p.s(MAXHP), p.s(MAXMANA), p.s(MAXSTAMINA)],
            created,
            "class {class}"
        );
        assert_eq!(
            [p.s(HITPOINTS), p.s(MANA), p.s(STAMINA)],
            created,
            "class {class}"
        );
        assert_eq!((p.s(LEVEL), p.s(NEXTEXP)), (1, 500), "class {class}");
        p.set_base_stat((), EXPERIENCE, t.threshold(class, 9) as i32);
        assert_eq!(level_up(&mut p, t, ()), 9, "class {class}");
        assert_eq!(
            [p.s(MAXHP), p.s(MAXMANA), p.s(MAXSTAMINA)],
            lvl10,
            "class {class}"
        );
        assert_eq!(
            [p.s(HITPOINTS), p.s(MANA), p.s(STAMINA)],
            lvl10,
            "class {class}"
        );
        assert_eq!(
            (p.s(LEVEL), p.s(STATPTS), p.s(NEWSKILLS)),
            (10, 45, 9),
            "class {class}"
        );
    }
    // Barbarian, +10 vitality through message 0x3A (stat 3, count 10).
    let mut p = Player::new(BARBARIAN);
    init_player_stats(&mut p, t, (), 0);
    p.set_base_stat((), STATPTS, 10);
    let (hp, st) = (p.s(MAXHP), p.s(MAXSTAMINA));
    assert_eq!(handle_add_stat_point(&mut p, t, (), &[0x3A, 3, 9]), 0);
    assert_eq!(
        (p.s(MAXHP) - hp, p.s(MAXSTAMINA) - st, p.s(STATPTS)),
        (10_240, 2_560, 0)
    );
    // Sorceress, +10 energy.
    let mut p = Player::new(SORCERESS);
    init_player_stats(&mut p, t, (), 0);
    p.set_base_stat((), STATPTS, 10);
    let mana = p.s(MAXMANA);
    gain_energy(&mut p, t, (), 10);
    assert_eq!((p.s(MAXMANA) - mana, p.s(STATPTS)), (5_120, 0));
}

/// Every class created in every act and levelled to every level from 2 to
/// 99: no panic; the level is `level_from_exp`, the maxima grow by
/// the class's per-level columns × d (<< 6), statpts by StatPerLevel × d,
/// newskills by d; and the thresholds bracket each level.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn every_class_every_level() {
    use vitals::stat::*;
    let t = vitals_tables();
    for class in 0..7 {
        for act in 0..=5 {
            let mut p = Player::new(class);
            init_player_stats(&mut p, t, (), act);
        }
        let c = t.charstats(class).unwrap();
        for level in 1..=98 {
            let need = t.threshold(class, level);
            assert_eq!(t.level_from_exp(class, need), level + 1, "class {class}");
            assert_eq!(t.level_from_exp(class, need - 1), level, "class {class}");
        }
        assert_eq!(t.level_from_exp(class, u32::MAX), 99, "class {class}");
        for target in 2..=99u32 {
            let mut p = Player::new(class);
            init_player_stats(&mut p, t, (), 0);
            let before = [p.s(MAXHP), p.s(MAXMANA), p.s(MAXSTAMINA)];
            p.set_base_stat((), EXPERIENCE, t.threshold(class, target - 1) as i32);
            let d = level_up(&mut p, t, ());
            let ctx = format!("class {class} target {target}");
            assert_eq!(d, target as i32 - 1, "{ctx}");
            assert_eq!(p.s(LEVEL), target as i32, "{ctx}");
            assert_eq!(p.s(NEXTEXP), t.threshold(class, target) as i32, "{ctx}");
            let grow = |per: u8| (i32::from(per) * d) << 6;
            assert_eq!(
                [p.s(MAXHP), p.s(MAXMANA), p.s(MAXSTAMINA)],
                [
                    before[0] + grow(c.lifeperlevel),
                    before[1] + grow(c.manaperlevel),
                    before[2] + grow(c.staminaperlevel)
                ],
                "{ctx}"
            );
            assert_eq!(p.s(STATPTS), i32::from(c.statperlevel as i8) * d, "{ctx}");
            assert_eq!(p.s(NEWSKILLS), d, "{ctx}");
            // A second level-up with the same experience changes nothing.
            assert_eq!(level_up(&mut p, t, ()), 0, "{ctx}");
        }
    }
}
