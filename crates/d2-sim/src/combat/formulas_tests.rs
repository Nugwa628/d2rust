//! A mod game's formula hooks (MS3) on the combat formulas: each hook
//! answers first, `None` keeps D2RUST's formula, and D2RUST's games are
//! unchanged.

use std::sync::Arc;

use super::vitals::VitalsTables;
use super::*;
use crate::monsters::init::{GameInfo, PlayerBonus};
use crate::rng::Seed;
use crate::rules::{FormulaHooks, FormulaHost, ModRules, Ruleset, Side};
use crate::skills::fake::*;
use crate::units::UnitType;
use d2_data::tables::Experience;

const TOHIT: u16 = 19;
const TOBLOCK: u16 = 20;
const FIRERESIST: u16 = 39;
const PASSIVE_DODGE: u16 = 338;
const PASSIVE_AVOID: u16 = 339;
const PASSIVE_EVADE: u16 = 340;

/// The test mod game: hit chance from the attack rating alone, block
/// from twice the block stat, no dodge, sure evade, a flat resist
/// penalty and fire cap, a fixed physical hit, three players always and
/// levels past the table.
#[derive(Debug)]
struct Hooks;

impl FormulaHooks for Hooks {
    fn hit_chance(&self, h: &mut dyn FormulaHost, _: HitTerms) -> Option<i32> {
        Some(h.stat(Side::Attacker, TOHIT, 0) / 3)
    }
    fn block_chance(&self, h: &mut dyn FormulaHost) -> Option<i32> {
        Some(2 * h.stat(Side::Defender, TOBLOCK, 0))
    }
    fn avoid_chance(&self, _: &mut dyn FormulaHost, kind: BlockResult) -> Option<i32> {
        match kind {
            BlockResult::Dodge => Some(0),
            BlockResult::Evade => Some(100),
            _ => None,
        }
    }
    fn resist_penalty(&self, difficulty: usize, _: bool) -> Option<i32> {
        Some(-10 * difficulty as i32)
    }
    fn resist_cap(&self, _: &mut dyn FormulaHost, resist: Option<u16>) -> Option<i32> {
        (resist == Some(FIRERESIST)).then_some(90)
    }
    fn damage(
        &self,
        h: &mut dyn FormulaHost,
        rec: &DamageRecord,
        _: bool,
        _: i32,
    ) -> Option<DamageRecord> {
        // One draw, from the attacker's seed.
        let roll = h.seed(Side::Attacker)?.roll(4) as i32;
        Some(DamageRecord {
            physical: (100 + roll) << 8,
            ..*rec
        })
    }
    fn player_bonus(&self, _: &GameInfo, _: u8) -> Option<PlayerBonus> {
        Some(PlayerBonus {
            players: 3,
            hp: 100,
            xp: 100,
        })
    }
    fn max_level(&self, _: i32) -> Option<u32> {
        Some(5)
    }
    fn exp_threshold(&self, _: i32, level: u32) -> Option<u32> {
        (level >= 3).then_some(level * 1000)
    }
}

#[derive(Debug)]
struct Game;

impl ModRules for Game {
    fn name(&self) -> &str {
        "formulas"
    }
    fn formulas(&self) -> Option<&dyn FormulaHooks> {
        Some(&Hooks)
    }
}

fn modded() -> Ruleset {
    Ruleset::Mod(Arc::new(Game))
}

fn world(rules: Ruleset) -> Fake {
    Fake {
        hostile: true,
        in_range: true,
        expansion: true,
        rules,
        ..Fake::default()
    }
}

fn st() -> crate::skills::SkillTables {
    skill_tables(vec![skill_rec()])
}

fn ct() -> CombatTables {
    combat_tables(vec![monster_rec()])
}

/// A seed whose next step gives `lo′ = x`.
fn seed_giving(x: u32) -> Seed {
    Seed::new(0, x)
}

fn duel(f: &mut Fake) -> (usize, usize) {
    let a = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(TOHIT, 300)
            .with(12, 10),
    );
    let d = f.add(FUnit::new(UnitType::Player, 0).with(31, 100).with(12, 8));
    (a, d)
}

#[test]
fn hit_chance_hook() {
    let (s, c) = (st(), ct());
    // D2RUST: chance 83, so a draw of 85 misses.
    let mut f = world(Ruleset::Original);
    let (a, d) = duel(&mut f);
    f.units[a].seed = seed_giving(85);
    assert!(!hit_test(&mut f, &s, &c, Some(a), Some(d), 0, false));
    // The mod: 300 / 3 = 100, the same draw hits.
    let mut f = world(modded());
    let (a, d) = duel(&mut f);
    f.units[a].seed = seed_giving(85);
    assert!(hit_test(&mut f, &s, &c, Some(a), Some(d), 0, false));
}

#[test]
fn block_chance_hook() {
    let c = ct();
    for (rules, with_shield, without) in [(Ruleset::Original, 75, 0), (modded(), 60, 60)] {
        let mut f = world(rules);
        let u = f.add(
            FUnit::new(UnitType::Player, 0)
                .with(TOBLOCK, 30)
                .with(2, 100)
                .with(12, 20),
        );
        f.units[u].shield = true;
        assert_eq!(block_chance(&f, &c, u, true), with_shield);
        f.units[u].shield = false;
        assert_eq!(block_chance(&f, &c, u, true), without);
    }
}

#[test]
fn avoid_chance_hook() {
    let defender = |f: &mut Fake| {
        f.add(
            FUnit::new(UnitType::Player, 0)
                .with(PASSIVE_DODGE, 30)
                .with(PASSIVE_AVOID, 40)
                .with(PASSIVE_EVADE, 50),
        )
    };
    let mut f = world(Ruleset::Original);
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = defender(&mut f);
    f.units[d].seed = seed_giving(29);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::Dodge);
    f.units[d].mode = 2;
    f.units[d].seed = seed_giving(99);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::None);

    let mut f = world(modded());
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = defender(&mut f);
    // Dodge 0: no roll, the seed is untouched.
    f.units[d].seed = seed_giving(29);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::None);
    assert_eq!(f.units[d].seed, seed_giving(29));
    // Avoid keeps D2RUST's 40.
    f.units[d].seed = seed_giving(39);
    assert_eq!(dodge(&mut f, a, d, true), BlockResult::Avoid);
    // Evade 100.
    f.units[d].mode = 2;
    f.units[d].seed = seed_giving(99);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::Evade);
}

/// The fire damage left of 1000 (1/256 units) on a player with fire
/// resist `res` at `difficulty`.
fn fire_after(rules: Ruleset, res: i32, difficulty: usize) -> i32 {
    let c = ct();
    let mut f = world(rules);
    f.difficulty = difficulty;
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(FIRERESIST, res));
    let mut rec = DamageRecord {
        fire: 1000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    rec.fire
}

#[test]
fn resist_hooks() {
    // D2RUST: capped at 75; the test tables' hell penalty is -100.
    assert_eq!(fire_after(Ruleset::Original, 85, 0), 250);
    assert_eq!(fire_after(Ruleset::Original, 85, 2), 1150);
    // The mod: cap 90, penalty -10 per difficulty.
    assert_eq!(fire_after(modded(), 85, 0), 150);
    assert_eq!(fire_after(modded(), 95, 0), 100);
    assert_eq!(fire_after(modded(), 85, 2), 350);
}

#[test]
fn damage_hook() {
    let (s, c) = (st(), ct());
    let mut f = world(modded());
    let (a, d) = duel(&mut f);
    f.units[a].seed = seed_giving(2);
    let mut rec = DamageRecord {
        fire: 7,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 0);
    let mut expect = seed_giving(2);
    let roll = expect.roll(4) as i32;
    assert_eq!(rec.physical, (100 + roll) << 8);
    assert_eq!(f.units[a].seed, expect);
    assert_eq!(rec.fire, 7);
    assert_ne!(rec.hit_flags & hitflag::ROLLED, 0);
}

#[test]
fn player_bonus_hook() {
    let info = GameInfo {
        players: 1,
        ..GameInfo::default()
    };
    let own = Ruleset::Original.player_bonus(&info, 0);
    assert_eq!(own, crate::monsters::init::player_bonus(&info, 0));
    assert_eq!(own.players, 1);
    assert_eq!(modded().player_bonus(&info, 0).players, 3);
}

#[test]
fn experience_length_hooks() {
    let row = |v: u32| {
        let mut e: Experience = blank();
        e.amazon = v;
        e
    };
    // Max level 3; levels 1-3 at 0, 100, 400 experience.
    let t = VitalsTables {
        charstats: Vec::new(),
        experience: vec![row(3), row(0), row(100), row(400), row(900)],
    };
    let own = Ruleset::Original;
    assert_eq!(t.max_level_in(&own, 0), 3);
    assert_eq!(t.threshold_in(&own, 0, 3), 900);
    assert_eq!(t.level_from_exp_in(&own, 0, 3500), 3);
    let m = modded();
    assert_eq!(t.max_level_in(&m, 0), 5);
    assert_eq!(t.threshold_in(&m, 0, 2), 400);
    assert_eq!(t.threshold_in(&m, 0, 3), 3000);
    assert_eq!(t.level_from_exp_in(&m, 0, 3500), 4);
    assert_eq!(t.level_from_exp_in(&m, 0, u32::MAX), 5);
}
