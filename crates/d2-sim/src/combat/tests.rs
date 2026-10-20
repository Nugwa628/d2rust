//! Tests for the combat rules: the percent helper, hit chance, defense, attack rating,
//! blocking, the damage fill, resistances, leech, stun, freeze and poison, durability, and
//! applying a melee hit. Random draws use fixed seeds.
use super::*;
use crate::rng::Seed;
use crate::skills::fake::*;
use crate::skills::SkillTables;
use crate::units::UnitType;

/// `lo′ mod 100` of the first step from `seed`.
fn pct_draw(seed: Seed) -> i32 {
    let mut s = seed;
    (s.step() % 100) as i32
}

/// A seed whose next step gives `lo′ = x` (lo 0, hi x).
fn seed_giving(x: u32) -> Seed {
    Seed::new(0, x)
}

fn st() -> SkillTables {
    skill_tables(vec![skill_rec()])
}

fn ct() -> CombatTables {
    combat_tables(vec![monster_rec()])
}

fn world() -> Fake {
    Fake {
        hostile: true,
        in_range: true,
        expansion: true,
        ..Fake::default()
    }
}

// ================================================================ pct

#[test]
fn pct_vectors() {
    assert_eq!(pct(0x20_0000, 50, 100), 1_048_550);
    assert_eq!(pct(1_000, 0x2_0000, 100), 1_310_000);
    // d = 0x30000 > v >> 4 = 0x20000, so the 64-bit path (v × p) / d applies: 533.
    assert_eq!(pct(0x20_0000, 50, 0x3_0000), 533);
    assert_eq!(pct(-50, 30, 100), -15);
    assert_eq!(pct(7, 9, 0), 0);
    // The plain 32-bit product wraps.
    assert_eq!(pct(0x1_0000, 0x1_0000, 1), 0);
}

// ================================================================ hit

#[test]
fn hit_chance_vectors() {
    let c = |ar, pct_ar, def, alvl, dlvl| {
        hit_chance(HitTerms {
            ar,
            pct_ar,
            def,
            alvl,
            dlvl,
        })
    };
    assert_eq!(c(300, 0, 100, 10, 8), 83);
    assert_eq!(c(50, 0, 1_000, 1, 30), 5);
    assert_eq!(c(200, 0, -50, 20, 20), 95);
    assert_eq!(c(-10, 0, 40, 5, 5), 5);
    assert_eq!(c(0, 0, 0, 7, 7), 95);
    // pctAR through pct: 100 + 50 % = 150 vs 150 at equal levels: 100.
    assert_eq!(c(100, 50, 150, 5, 5), 50);
    assert_eq!(c(100, 50, 150, 1, 9), 10);
}

#[test]
#[should_panic(expected = "alvl + dlvl = 0")]
fn hit_chance_zero_levels_is_fatal() {
    hit_chance(HitTerms {
        ar: 1,
        pct_ar: 0,
        def: 1,
        alvl: 0,
        dlvl: 0,
    });
}

#[test]
fn defense_vectors() {
    let mut f = world();
    let s = st();
    let u = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(31, 100)
            .with(2, 50)
            .with(16, 50),
    );
    assert_eq!(defense(&mut f, &s, u), 168);
    f.set(u, 31, -32);
    assert_eq!(defense(&mut f, &s, u), -10);
    f.set(u, 31, 100);
    f.set(u, 182, 10);
    assert_eq!(defense(&mut f, &s, u), 184);
    // Skill armor percent (171) adds to the item percent.
    f.set(u, 182, 0);
    f.set(u, 171, 50);
    assert_eq!(defense(&mut f, &s, u), 112 + 112);
}

#[test]
fn defense_holy_shield() {
    let mut r = skill_rec();
    r.calc1 = 0;
    let mut s = skill_tables(vec![skill_rec(), r]);
    s.skills_code = vec![0x07, 100, 0x00];
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 3).with(31, 100));
    f.units[u].states.push(101);
    f.units[u].state_stats.insert((101, 350), 1);
    f.units[u].state_stats.insert((101, 351), 5);
    // No shield: no bonus.
    assert_eq!(defense(&mut f, &s, u), 100);
    f.units[u].shield = true;
    assert_eq!(defense(&mut f, &s, u), 200);
}

#[test]
fn attack_rating_player() {
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 3).with(19, 100).with(2, 27));
    // 100 + 5 × 20 + ToHitFactor(Paladin) 20.
    assert_eq!(attack_rating(&f, &ct(), u), 220);
}

#[test]
#[should_panic(expected = "non-player")]
fn attack_rating_monster_is_fatal() {
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Monster, 0));
    attack_rating(&f, &ct(), u);
}

/// Monster attacker with AR 300 vs defense 100, levels 10 / 8: chance 83.
fn duel(f: &mut Fake) -> (usize, usize) {
    let a = f.add(FUnit::new(UnitType::Monster, 0).with(19, 300).with(12, 10));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(31, 100).with(12, 8));
    (a, d)
}

#[test]
fn hit_test_draw() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let (a, d) = duel(&mut f);
    // Seed (1, 0): lo′ = 0x6AC690C5 = 1791398085, mod 100 = 85 → miss.
    assert_eq!(pct_draw(Seed::new(1, 0)), 85);
    f.units[a].seed = Seed::new(1, 0);
    assert!(!hit_test(&mut f, &s, &c, Some(a), Some(d), 0, false));
    assert_eq!(f.units[a].seed, Seed::new(0x6AC6_90C5, 0));
    f.units[a].seed = seed_giving(82);
    assert!(hit_test(&mut f, &s, &c, Some(a), Some(d), 0, false));
    f.units[a].seed = seed_giving(83);
    assert!(!hit_test(&mut f, &s, &c, Some(a), Some(d), 0, false));
    // The bonus is flat for monsters: +100 AR → factor 80, chance 88.
    f.units[a].seed = seed_giving(87);
    assert!(hit_test(&mut f, &s, &c, Some(a), Some(d), 100, false));
    // Null units: false, no draw.
    let before = f.units[a].seed;
    assert!(!hit_test(&mut f, &s, &c, Some(a), None, 0, false));
    assert_eq!(f.units[a].seed, before);
}

#[test]
fn hit_terms_player_adjustments() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(19, 100)
            .with(2, 7)
            .with(12, 5),
    );
    let d = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(31, 200)
            .with(33, 20)
            .with(32, 40),
    );
    let t = hit_terms(&mut f, &s, &c, a, d, 30, false);
    assert_eq!((t.ar, t.pct_ar, t.def), (105, 30, 220));
    assert_eq!(hit_terms(&mut f, &s, &c, a, d, 0, true).def, 240);
    // Ignore target defense on a plain monster; not on a unique.
    f.set(a, 115, 1);
    assert_eq!(hit_terms(&mut f, &s, &c, a, d, 0, false).def, 0);
    f.units[d].flags = 8;
    assert_eq!(hit_terms(&mut f, &s, &c, a, d, 0, false).def, 220);
    // Fractional target AC 50: a unique takes 50 %, a superunique 25 %.
    f.set(a, 115, 0);
    f.set(a, 116, 50);
    assert_eq!(hit_terms(&mut f, &s, &c, a, d, 0, false).def, 110);
    f.units[d].flags = 2;
    assert_eq!(hit_terms(&mut f, &s, &c, a, d, 0, false).def, 220 - 55);
    // Demon / undead to-hit; attack vs montype layer match.
    f.set(a, 116, 0);
    f.set(a, 123, 7);
    f.set(a, 124, 9);
    f.units[d].demon = true;
    f.units[a].entries.insert(179, vec![(4, 11), (5, 13)]);
    let mut m = monster_rec();
    m.montype = 4;
    let c2 = combat_tables(vec![m]);
    let t = hit_terms(&mut f, &s, &c2, a, d, 0, false);
    assert_eq!((t.ar, t.pct_ar), (112, 11));
}

#[test]
fn block_chance_vectors() {
    let c = ct();
    let mut f = world();
    let ama = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(20, 30)
            .with(2, 100)
            .with(12, 20),
    );
    f.units[ama].shield = true;
    assert_eq!(block_chance(&f, &c, ama, true), 75);
    assert_eq!(block_chance(&f, &c, ama, false), 55);
    f.units[ama].shield = false;
    assert_eq!(block_chance(&f, &c, ama, true), 0);
    let sor = f.add(
        FUnit::new(UnitType::Player, 1)
            .with(20, 10)
            .with(2, 10)
            .with(12, 5),
    );
    f.units[sor].shield = true;
    assert_eq!(block_chance(&f, &c, sor, true), -15);
    // Monsters: NoShldBlock, or a shield; class 359 never; 243 always.
    let mut m = monster_rec();
    m.noshldblock = true;
    let c2 = combat_tables(vec![m, monster_rec()]);
    let mo = f.add(FUnit::new(UnitType::Monster, 0).with(20, 90));
    assert_eq!(block_chance(&f, &c2, mo, true), 75);
    f.units[mo].class = 1;
    assert_eq!(block_chance(&f, &c2, mo, true), 0);
    f.units[mo].shield = true;
    assert_eq!(block_chance(&f, &c2, mo, true), 75);
    f.units[mo].class = 359;
    assert_eq!(block_chance(&f, &c2, mo, true), 0);
    f.units[mo].class = 243;
    f.units[mo].shield = false;
    assert_eq!(block_chance(&f, &c2, mo, true), 75);
}

#[test]
fn block_running_divides_by_three() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(20, 30)
            .with(2, 100)
            .with(12, 20),
    );
    f.units[d].shield = true;
    f.units[d].mode = 3;
    f.units[d].moving = true;
    f.units[d].seed = seed_giving(24);
    assert_eq!(
        block_or_dodge(&mut f, &c, a, d, false, true),
        BlockResult::Block
    );
    f.units[d].seed = seed_giving(25);
    assert_eq!(
        block_or_dodge(&mut f, &c, a, d, false, true),
        BlockResult::None
    );
    // Walking does not divide.
    f.units[d].mode = 2;
    f.units[d].seed = seed_giving(74);
    assert_eq!(
        block_or_dodge(&mut f, &c, a, d, false, true),
        BlockResult::Block
    );
}

#[test]
fn block_edge_cases() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    // Block 2 while running: 2 / 3 = 0, still one draw.
    let d = f.add(FUnit::new(UnitType::Player, 0).with(20, 2));
    f.units[d].shield = true;
    f.units[d].mode = 3;
    f.units[d].moving = true;
    f.expansion = false;
    let s0 = Seed::new(5, 5);
    f.units[d].seed = s0;
    // Running and no evade stat: dodge returns 0 with no draw.
    assert_eq!(
        block_or_dodge(&mut f, &c, a, d, false, true),
        BlockResult::None
    );
    let mut s1 = s0;
    s1.step();
    assert_eq!(f.units[d].seed, s1);
    // Negative block: no draw.
    let sor = f.add(
        FUnit::new(UnitType::Player, 1)
            .with(20, 10)
            .with(2, 10)
            .with(12, 5),
    );
    f.units[sor].shield = true;
    f.expansion = true;
    f.units[sor].seed = s0;
    assert_eq!(
        block_or_dodge(&mut f, &c, a, sor, false, true),
        BlockResult::None
    );
    assert_eq!(f.units[sor].seed, s0);
}

#[test]
fn dodge_avoid_evade_weapon_block() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(338, 30)
            .with(339, 40)
            .with(340, 50),
    );
    // Standing: dodge for melee, avoid for missiles (roll(100)).
    f.units[d].seed = seed_giving(29);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::Dodge);
    f.units[d].seed = seed_giving(30);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::None);
    f.units[d].seed = seed_giving(39);
    assert_eq!(dodge(&mut f, a, d, true), BlockResult::Avoid);
    // Moving: evade only.
    f.units[d].mode = 2;
    f.units[d].seed = seed_giving(49);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::Evade);
    f.units[d].seed = seed_giving(50);
    assert_eq!(dodge(&mut f, a, d, true), BlockResult::None);
    // Weapon block: claws (class 13), entries by layer.
    f.units[d].mode = 1;
    let claw = f.add_item(FItem {
        types: vec![67],
        ..FItem::default()
    });
    f.units[d].items.insert(4, claw);
    f.units[d]
        .entries
        .insert(348, vec![(0, 10), (67, 20), (99, 60)]);
    assert_eq!(weapon_block(&f, Some(d)), 20);
    assert_eq!(weapon_block(&f, None), 0);
    // A layer-0 entry after a larger typed entry lowers the value.
    f.units[d]
        .entries
        .insert(348, vec![(67, 20), (99, 60), (0, 10)]);
    assert_eq!(weapon_block(&f, Some(d)), 10);
    f.units[d].entries.insert(348, vec![(0, 20), (67, 35)]);
    assert_eq!(weapon_block(&f, Some(d)), 35);
    f.units[d].entries.insert(348, vec![(67, 35), (0, 20)]);
    assert_eq!(weapon_block(&f, Some(d)), 20);
    f.units[d].entries.insert(348, vec![(27, 35)]);
    assert_eq!(weapon_block(&f, Some(d)), 0);
    f.units[d]
        .entries
        .insert(348, vec![(0, 10), (67, 20), (99, 60)]);
    f.units[d].weapon_class = 13;
    f.units[d].seed = seed_giving(19);
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::WeaponBlock);
    // Failed weapon block then dodge: two draws.
    f.units[d].seed = seed_giving(20);
    let mut s = f.units[d].seed;
    let _ = dodge(&mut f, a, d, false);
    s.step();
    s.step();
    assert_eq!(f.units[d].seed, s);
}

#[test]
fn melee_result_flow() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let (a, d) = duel(&mut f);
    // Hit, no block / dodge: hit + get-hit.
    f.units[a].seed = seed_giving(0);
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0x0005);
    // Uninterruptable: no get-hit.
    f.units[d].states.push(54);
    f.units[a].seed = seed_giving(0);
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0x0001);
    f.units[d].states.clear();
    // Miss: 0.
    f.units[a].seed = seed_giving(90);
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0);
    // Out of range: 0, no draw.
    f.in_range = false;
    let before = f.units[a].seed;
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0);
    assert_eq!(f.units[a].seed, before);
    f.in_range = true;
    // Running player: hit with no hit test; block clears hit.
    f.units[d].mode = 3;
    f.units[d].moving = true;
    f.units[d].shield = true;
    f.set(d, 20, 60);
    f.set(d, 2, 100);
    f.set(d, 12, 1);
    f.units[d].seed = seed_giving(10);
    let before = f.units[a].seed;
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0x0010);
    assert_eq!(f.units[a].seed, before);
    // Evade clears the hit and sets nothing.
    f.units[d].shield = false;
    f.set(d, 340, 100);
    f.units[d].seed = seed_giving(0);
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0);
    // Not hostile: 0.
    f.hostile = false;
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0);
}

#[test]
fn prevent_heal_on_hit() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(19, 1_000)
            .with(12, 10)
            .with(117, 1),
    );
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(12, 10));
    f.units[a].seed = seed_giving(0);
    assert!(hit_test(&mut f, &s, &c, Some(a), Some(d), 0, false));
    assert_eq!(f.log, ["curse 1 0 52 31 0 120000 0 1"]);
}

// ================================================================ damage

#[test]
fn bonuses_vectors() {
    let s = st();
    let mut f = world();
    let u = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(0, 30)
            .with(21, 2)
            .with(22, 7),
    );
    let w = f.add_item(FItem {
        str_dex: (100, 0),
        ..FItem::default()
    });
    f.units[u].weapon = Some(w);
    let mut seed = f.units[u].seed;
    let v = bonuses(&mut f, &s, u, true, None, 0, 0, 0, 0, 128);
    assert_eq!(v, 665 + seed.roll(1_664) as i32);
    assert_eq!(f.units[u].seed, seed);
    // No weapon, mindamage 0, maxdamage 0, str 15: 294 + roll(294).
    let u2 = f.add(FUnit::new(UnitType::Player, 0).with(0, 15));
    let mut seed = f.units[u2].seed;
    let v = bonuses(&mut f, &s, u2, true, None, 0, 0, 0, 0, 128);
    assert_eq!(v, 294 + seed.roll(294) as i32);
    // Enhanced −120 is floored at −90 (str 0): 26 + roll(26).
    let u3 = f.add(FUnit::new(UnitType::Player, 0));
    let mut seed = f.units[u3].seed;
    let v = bonuses(&mut f, &s, u3, true, None, 0, 0, -120, 0, 128);
    assert_eq!(v, 26 + seed.roll(26) as i32);
    // SrcDam 64 halves through pct.
    let mut seed = f.units[u3].seed;
    let v = bonuses(&mut f, &s, u3, true, None, 0, 0, -120, 0, 64);
    assert_eq!(v, (26 + seed.roll(26) as i32) / 2);
}

#[test]
fn element_vectors() {
    let mut f = world();
    let u = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(48, 10)
            .with(49, 20)
            .with(329, 50),
    );
    let mut seed = f.units[u].seed;
    let v = element(&mut f, u, 49, 48, Some(329), 0, 0, 0);
    assert_eq!(v, 3_840 + seed.roll(3_840) as i32);
    // max < 8 (stat 0): current, no draw.
    let before = f.units[u].seed;
    assert_eq!(element(&mut f, u, 51, 50, Some(330), 0, 0, -5), 0);
    assert_eq!(element(&mut f, u, 51, 50, Some(330), 0, 0, 77), 77);
    assert_eq!(f.units[u].seed, before);
}

/// A plain fill: player attacker vs monster, seed (1, 0).
fn fill_case(f: &mut Fake) -> (usize, usize) {
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 100_000));
    (a, d)
}

#[test]
fn fill_draw_order_plain() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let (a, d) = fill_case(&mut f);
    f.set(a, 48, 10);
    f.set(a, 49, 20);
    f.set(a, 329, 50);
    let mut rec = DamageRecord::default();
    fill(&mut f, &s, &c, a, d, &mut rec, false, 64);
    let mut seed = Seed::new(1, 0);
    // Draws: physical roll(256) (no weapon: 1–2 → 256..512), fire, burn
    // roll(1).
    let phys = pct(256 + seed.roll(256) as i32, 64, 128);
    let fire = (3_840 + seed.roll(3_840) as i32) * 64 / 128;
    assert_eq!(seed.roll(1), 0);
    assert_eq!((rec.physical, rec.fire), (phys, fire));
    // Burn quirk: (0 × 64 + 316 + 0) / 128 = 2.
    assert_eq!(rec.burn, 2);
    assert_eq!(f.units[a].seed, seed);
    assert_eq!(rec.hit_flags & hitflag::ROLLED, hitflag::ROLLED);
    // Player attacker: event 3 on the defender.
    assert_eq!(f.log, ["event 3 1 0"]);
}

#[test]
fn fill_crit_chain() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let (a, d) = fill_case(&mut f);
    // Critical strike 1 fails (seed draw ≥ 1), deadly strike still draws.
    f.set(a, 337, 1);
    f.set(a, 141, 100);
    let mut rec = DamageRecord::default();
    fill(&mut f, &s, &c, a, d, &mut rec, false, 128);
    let mut seed = Seed::new(1, 0);
    let phys = 256 + seed.roll(256) as i32;
    assert!(seed.roll_range(0, 100) >= 1);
    assert!(seed.roll_range(0, 100) < 100);
    seed.roll(1);
    assert_eq!(rec.physical, phys * 2);
    assert_ne!(rec.result & hit::result::CRITICAL, 0);
    assert_eq!(f.units[a].seed, seed);
}

#[test]
fn fill_monster_drains_and_crit() {
    let s = st();
    let mut m = monster_rec();
    m.crit = 100;
    let c = combat_tables(vec![m]);
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0).with(60, 1).with(61, 1));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    // old + (old + roll) × s / 128 with s = 64; min = max
    // gives no roll: 10 + (10 + 256) / 2.
    let mut rec = DamageRecord {
        life_leech: 10,
        result: hit::result::NO_EVENTS,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 64);
    assert_eq!(rec.life_leech, 10 + 133);
    // Monster crit 100: everything doubled, hit class 0x10, no overlay
    // (the nibble was free).
    assert_eq!(rec.hit_class, 0x10);
    assert!(!f.log.iter().any(|l| l.starts_with("overlay")));
    // A taken high nibble stays; overlay 54 instead.
    let mut rec = DamageRecord {
        hit_class: 0x20,
        result: hit::result::NO_EVENTS,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 64);
    assert_eq!(rec.hit_class, 0x20);
    assert!(f.log.contains(&"overlay 1 54".to_string()));
    // Monsters raise no event 3.
    assert!(!f.log.iter().any(|l| l.starts_with("event")));
}

#[test]
fn fill_conversion_and_lengths() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let (a, d) = fill_case(&mut f);
    f.set(a, 21, 10);
    f.set(a, 22, 10);
    f.set(a, 57, 512);
    f.set(a, 58, 512);
    f.set(a, 59, 100);
    f.set(a, 326, 2);
    f.set(a, 66, 10);
    let mut rec = DamageRecord {
        conv_elem: 4,
        conv_pct: 50,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 128);
    // Physical 2560 + roll(256) (max ≤ min → min + 256); half to cold.
    let mut seed = Seed::new(1, 0);
    let p = 2_560 + seed.roll(256) as i32;
    let conv = pct(p, 50, 100);
    assert_eq!((rec.physical, rec.cold), (p - conv, conv));
    assert_eq!(rec.cold_len, 50);
    assert_eq!(rec.poison, 512);
    assert_eq!(rec.poison_len, 50);
    assert_eq!(rec.stun_len, 10);
}

#[test]
fn resist_vectors() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(39, 80));
    // Player fire res 80, max 0, Hell expansion (−100): −20 → × 120 %.
    f.difficulty = 2;
    let mut rec = DamageRecord {
        fire: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.fire, 1_200);
    // Fire res 200, max fire 10, Normal: cap 85 → × 15 %.
    f.difficulty = 0;
    f.set(d, 39, 200);
    f.set(d, 40, 10);
    let mut rec = DamageRecord {
        fire: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.fire, 150);
    // Classic Nightmare: −20.
    f.expansion = false;
    f.difficulty = 1;
    f.set(d, 39, 0);
    let mut rec = DamageRecord {
        fire: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.fire, 1_200);
    // Monster defender: res 120 with pierce 30 → no pierce, 100 → 0;
    // res 90 with pierce 30 → 60 → × 40 %.
    let p = f.add(FUnit::new(UnitType::Player, 0).with(333, 30));
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(39, 120));
    let mut rec = DamageRecord {
        fire: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(p), m, &mut rec);
    assert_eq!(rec.fire, 0);
    f.set(m, 39, 90);
    let mut rec = DamageRecord {
        fire: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(p), m, &mut rec);
    assert_eq!(rec.fire, 400);
    // Floor −100.
    f.set(m, 39, -300);
    let mut rec = DamageRecord {
        fire: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(p), m, &mut rec);
    assert_eq!(rec.fire, 2_000);
}

#[test]
fn damage_reduction_and_absorb() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(34, 3)
            .with(36, 20)
            .with(142, 50)
            .with(143, 2),
    );
    let mut rec = DamageRecord {
        physical: 2_560,
        fire: 2_560,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.physical, 1_433);
    assert_eq!(rec.fire, 1_024);
    assert_eq!(rec.absorbed, 1_536);
    assert_eq!(rec.total, 1_433 + 1_024);
    assert_eq!(f.log, ["event 11 1 0"]);
    // Pierce percent of damage reduction (/1024): 512 → half of 768.
    let mut rec = DamageRecord {
        physical: 2_560,
        pierce_pct: 512,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.physical, (2_560 - 384) * 80 / 100);
    // Damage resist cap 50 for players.
    f.set(d, 34, 0);
    f.set(d, 36, 90);
    let mut rec = DamageRecord {
        physical: 1_000,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.physical, 500);
}

#[test]
fn damage_percent_and_bypass() {
    let c = ct();
    let mut f = world();
    let p1 = f.add(FUnit::new(UnitType::Player, 0));
    let p2 = f.add(FUnit::new(UnitType::Player, 0));
    assert_eq!(damage_percent(&f, &c, Some(p1), p2), 17);
    assert_eq!(damage_percent(&f, &c, Some(p1), p1), 100);
    let h1 = f.add(FUnit::new(UnitType::Monster, 0));
    let h2 = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[h1].hireling = true;
    f.units[h2].hireling = true;
    assert_eq!(damage_percent(&f, &c, Some(h1), h2), 25);
    let boss = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[boss].boss = true;
    f.difficulty = 1;
    assert_eq!(damage_percent(&f, &c, Some(h1), boss), 35);
    f.difficulty = 0;
    let mut rec = DamageRecord {
        physical: 1_000,
        poison_len: 99,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(p1), p2, &mut rec);
    // PvP 17 % scales physical, not lengths.
    assert_eq!((rec.physical, rec.poison_len), (170, 99));
    // Bypass beasts: monster neither undead nor demon → no absorb, DR
    // skipped, positive resists dropped.
    let m = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(34, 3)
            .with(39, 50)
            .with(142, 40),
    );
    let mut rec = DamageRecord {
        physical: 1_000,
        fire: 1_000,
        hit_flags: hitflag::BYPASS_BEASTS,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(p1), m, &mut rec);
    assert_eq!((rec.physical, rec.fire, rec.absorbed), (1_000, 1_000, 0));
}

#[test]
fn freeze_and_poison_length_rules() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(118, 1));
    let mut rec = DamageRecord {
        cold_len: 101,
        freeze_len: 51,
        poison_len: 10,
        ..DamageRecord::default()
    };
    f.units[d].states.push(133);
    totals(&mut f, &c, Some(a), d, &mut rec);
    // Lengths are resisted rows too (cold res 0, Normal): halves stay.
    assert_eq!((rec.cold_len, rec.freeze_len, rec.poison_len), (50, 25, 0));
    f.set(d, 153, 1);
    let mut rec = DamageRecord {
        cold_len: 101,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(rec.cold_len, 0);
}

#[test]
fn leech_vectors() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(6, 0).with(7, 100_000));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let mut rec = DamageRecord {
        physical: 2_560,
        life_leech: 5,
        ..DamageRecord::default()
    };
    leech(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(f.get(a, 6), 128);
    assert_eq!(rec.life_leech, 320);
    assert_eq!(f.log.last().unwrap(), "overlay 0 151");
    // Hell vs Drain(H) 50: 21.
    let mut m = monster_rec();
    m.drain_h = 50;
    let c2 = combat_tables(vec![m]);
    f.difficulty = 2;
    f.set(a, 6, 0);
    let mut rec = DamageRecord {
        physical: 2_560,
        life_leech: 5,
        ..DamageRecord::default()
    };
    leech(&mut f, &c2, Some(a), d, &mut rec);
    assert_eq!(f.get(a, 6), 21);
    assert_eq!(rec.life_leech, 106);
    // Both leeches: roll(2) picks the overlay; physical 0: nothing.
    f.difficulty = 0;
    f.set(a, 9, 100_000);
    let mut seed = f.units[a].seed;
    let mut rec = DamageRecord {
        physical: 2_560,
        life_leech: 5,
        mana_leech: 5,
        ..DamageRecord::default()
    };
    leech(&mut f, &c, Some(a), d, &mut rec);
    let o = if seed.roll(2) != 0 { 152 } else { 151 };
    assert_eq!(f.log.last().unwrap(), &format!("overlay 0 {o}"));
    assert_eq!(f.get(a, 8), 128);
    let before = f.log.len();
    let mut rec = DamageRecord {
        life_leech: 5,
        ..DamageRecord::default()
    };
    leech(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(f.log.len(), before);
    // Monster Drain 0: stop before shifting.
    let mut m = monster_rec();
    m.drain = 0;
    let c3 = combat_tables(vec![m]);
    let mut rec = DamageRecord {
        physical: 2_560,
        life_leech: 5,
        ..DamageRecord::default()
    };
    leech(&mut f, &c3, Some(a), d, &mut rec);
    assert_eq!(rec.life_leech, 5);
}

#[test]
fn leech_monster_rule() {
    let c = ct();
    let mut f = world();
    let a = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 1_000)
            .with(7, 1_500),
    );
    let d = f.add(FUnit::new(UnitType::Player, 0).with(8, 50).with(10, 0));
    let mut rec = DamageRecord {
        physical: 300,
        life_leech: 400,
        mana_leech: 80,
        stamina_leech: 9,
        ..DamageRecord::default()
    };
    leech(&mut f, &c, Some(a), d, &mut rec);
    // L = min(400, 300), M = min(80, 50), S = 0: 350, capped at 500.
    assert_eq!(f.get(a, 6), 1_350);
    assert_eq!((rec.life_leech, rec.mana_leech), (400, 80));
}

#[test]
fn heal_and_mana_rules() {
    let mut f = world();
    let u = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(6, 900)
            .with(7, 1_000)
            .with(8, 0)
            .with(9, 50),
    );
    assert_eq!(heal(&mut f, u, 500), 100);
    assert_eq!(heal(&mut f, u, -1), 0);
    assert_eq!(add_mana(&mut f, u, 70), 50);
    f.units[u].states.push(92);
    f.set(u, 6, 10);
    assert_eq!(heal(&mut f, u, 5), 0);
}

#[test]
fn stun_rules() {
    let c = ct();
    let mut f = world();
    f.frame = 1_000;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[m].flags = 8;
    f.units[a].seed = seed_giving(89);
    stun(&mut f, &c, a, m, 400);
    assert!(f.log.is_empty());
    f.units[a].seed = seed_giving(90);
    stun(&mut f, &c, a, m, 400);
    assert_eq!(
        f.log,
        ["list 1 21 0 1250", "timer 1 12 1250", "state 1 21 true"]
    );
    f.log.clear();
    stun(&mut f, &c, a, m, 5);
    assert_eq!(f.log, ["expiry 1 21 1005", "timer 1 12 1005"]);
    // Hirelings: 13 frames.
    let h = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[h].hireling = true;
    f.log.clear();
    stun(&mut f, &c, a, h, 40);
    assert_eq!(f.log[0], "list 2 21 0 1013");
}

#[test]
fn cold_and_shatter() {
    let c = ct();
    let mut f = world();
    f.frame = 10;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    f.units[d].seed = seed_giving(19);
    cold(&mut f, &c, a, d, 30);
    assert_eq!(
        f.log,
        [
            "state 1 11 true",
            "list 1 11 0 40",
            "timer 1 12 40",
            "liststat 1 11 67 -50",
            "liststat 1 11 68 -50",
            "liststat 1 11 69 -50",
            "anim 1",
            "state 1 107 false",
        ]
    );
    // Monster with effect −30 on Hell: length / 4; shatter on r < 20.
    let mut m = monster_rec();
    m.coldeffect_h = (-30i8) as u8;
    let c2 = combat_tables(vec![m]);
    f.difficulty = 2;
    let mo = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[mo].seed = seed_giving(19);
    f.log.clear();
    cold(&mut f, &c2, a, mo, 41);
    assert_eq!(f.log[1], "list 2 11 0 20");
    assert_eq!(f.log.last().unwrap(), "state 2 107 true");
    // Effect 0: nothing, no draw.
    f.log.clear();
    let before = f.units[mo].seed;
    cold(&mut f, &c, a, mo, 41);
    assert!(f.log.is_empty());
    assert_eq!(f.units[mo].seed, before);
}

#[test]
fn freeze_rules() {
    let mut m = monster_rec();
    m.coldeffect = (-50i8) as u8;
    let c = combat_tables(vec![m]);
    let mut f = world();
    f.frame = 100;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let mo = f.add(FUnit::new(UnitType::Monster, 0));
    freeze(&mut f, &c, mo, a, 30);
    assert_eq!(
        f.log,
        [
            "state 1 1 true",
            "list 1 1 0 130",
            "timer 1 12 130",
            "cancel 1 2",
            "timer 1 2 131"
        ]
    );
    // Existing list keeps the later expiry.
    f.log.clear();
    freeze(&mut f, &c, mo, a, 5);
    assert_eq!(f.log[0], "expiry 1 1 130");
    // Uninterruptable: nothing.
    f.units[mo].states.push(54);
    f.log.clear();
    freeze(&mut f, &c, mo, a, 5);
    assert!(f.log.is_empty());
}

#[test]
fn poison_strength_rule() {
    let mut f = world();
    f.frame = 7;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    poison(&mut f, a, m, 300, 50);
    assert_eq!(
        f.log,
        [
            "cancel 1 3",
            "timer 1 3 8",
            "state 1 2 true",
            "list 1 2 0 57",
            "liststat 1 2 74 -300",
            "timer 1 12 57"
        ]
    );
    // Weaker poison: only the regen timer is touched.
    f.log.clear();
    poison(&mut f, a, m, 200, 90);
    assert_eq!(f.log, ["cancel 1 3", "timer 1 3 8"]);
    f.log.clear();
    burn(&mut f, a, m, 0, 90);
    assert!(f.log.is_empty());
}

#[test]
fn hit_recovery_vectors() {
    let c = ct();
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0).with(7, 25_600));
    let rec = DamageRecord {
        total: 1_700,
        ..DamageRecord::default()
    };
    let before = f.units[u].seed;
    assert!(no_get_hit(&mut f, &c.hitclass, u, &rec, 2));
    assert_eq!(f.units[u].seed, before);
    // hth (div 16), total 4000: ≥ 1600, ≥ 3200, < 6400: one mask(4) draw.
    let rec = DamageRecord {
        total: 4_000,
        ..DamageRecord::default()
    };
    let mut s = before;
    let want = s.mask(4) == 0;
    assert_eq!(no_get_hit(&mut f, &c.hitclass, u, &rec, 1), want);
    assert_eq!(f.units[u].seed, s);
    // Element nibble → divisor 16; 2hsl → 64.
    assert_eq!(get_hit_divisor(&c.hitclass, 0x22), 16);
    assert_eq!(get_hit_divisor(&c.hitclass, 6), 64);
    assert_eq!(get_hit_divisor(&c.hitclass, 10), 32);
    // Poison-only and small hits never get-hit.
    let rec = DamageRecord {
        poison: 9_000,
        total: 9_000,
        ..DamageRecord::default()
    };
    assert!(no_get_hit(&mut f, &c.hitclass, u, &rec, 1));
}

#[test]
fn element_hit_class_rotation() {
    let rec = DamageRecord {
        fire: 5,
        lightning: 5,
        ..DamageRecord::default()
    };
    let mut ctr = 0u8;
    // i = 0: cold (0), fire → 0x20; i = 1: fire; i = 2: lightning; i = 3:
    // poison (0), cold (0), fire.
    let got: Vec<u32> = (0..4)
        .map(|_| element_hit_class(&mut ctr, &rec, 2))
        .collect();
    assert_eq!(got, [0x22, 0x22, 0x42, 0x22]);
    let mut ctr = 255u8;
    let crit = DamageRecord {
        result: hit::result::CRITICAL,
        ..DamageRecord::default()
    };
    assert_eq!(element_hit_class(&mut ctr, &crit, 0), 13 | 0x10);
    assert_eq!(ctr, 0);
}

#[test]
fn crushing_blow_vectors() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(136, 100));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 25_600));
    let mut rec = DamageRecord::default();
    let arg = 136 << 16;
    assert_eq!(crushing_blow(&mut f, 5, a, d, &mut rec, arg), 1);
    assert_eq!(f.get(d, 6), 25_600 - 6_400);
    assert_eq!(f.log.last().unwrap(), "overlay 1 147");
    // Missile: × 2; boss: 8; damage resist 50 halves.
    f.set(d, 6, 25_600);
    f.units[d].boss = true;
    f.set(d, 36, 50);
    crushing_blow(&mut f, 6, a, d, &mut rec, arg);
    assert_eq!(f.get(d, 6), 25_600 - 800);
    // Players: 10; chance 0: no draw.
    let p = f.add(FUnit::new(UnitType::Player, 0).with(6, 1_000));
    crushing_blow(&mut f, 5, a, p, &mut rec, arg);
    assert_eq!(f.get(p, 6), 900);
    let before = f.units[a].seed;
    assert_eq!(crushing_blow(&mut f, 5, a, p, &mut rec, 999 << 16), 0);
    assert_eq!(f.units[a].seed, before);
}

#[test]
fn open_wounds_vectors() {
    assert_eq!(open_wounds_base(20), 216);
    assert_eq!(open_wounds_base(70), 1_791);
    assert_eq!(open_wounds_base(1), 0);
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(135, 100).with(12, 20));
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    assert_eq!(open_wounds(&mut f, 5, a, m, 135 << 16), Some(256));
    assert_eq!(f.log.last().unwrap(), "curse 1 0 62 74 -256 200 0 1");
    f.set(a, 12, 70);
    let p = f.add(FUnit::new(UnitType::Player, 0));
    assert_eq!(open_wounds(&mut f, 6, a, p, 135 << 16), Some(228));
}

#[test]
fn durability_selection() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    let helm = f.add_item(FItem {
        types: vec![50],
        durability: true,
        ..FItem::default()
    });
    let belt = f.add_item(FItem {
        types: vec![50],
        durability: true,
        ..FItem::default()
    });
    f.units[d].items.insert(1, helm);
    f.units[d].items.insert(8, belt);
    // W = 3 + 2 = 5: roll(7) then roll(5); the walk picks a kept slot,
    // then the 10 % armor draw.
    let mut s = f.units[d].seed;
    let mut i = s.roll(7) as usize;
    let mut w = s.roll(5) as i32;
    let kept = [(1usize, 3, helm), (4, 2, belt)];
    let item = loop {
        if let Some(&(_, wt, it)) = kept.iter().find(|k| k.0 == i) {
            if w < wt {
                break it;
            }
            w -= wt;
        }
        i = (i + 1) % 7;
    };
    let hit = (s.step() % 100) < 10;
    durability(&mut f, a, d);
    assert_eq!(f.units[d].seed, s);
    assert_eq!(f.log.is_empty(), !hit);
    if hit {
        assert_eq!(f.log[0], format!("durability {d} {item}"));
    }
    // No armor: no draws.
    let d2 = f.add(FUnit::new(UnitType::Player, 0));
    let before = f.units[d2].seed;
    durability(&mut f, a, d2);
    assert_eq!(f.units[d2].seed, before);
}

#[test]
fn durability_hit_chances() {
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    let wpn = f.add_item(FItem {
        types: vec![45],
        durability: true,
        throw: true,
        ..FItem::default()
    });
    // Classic game, throwing predicate: never (no draw).
    f.expansion = false;
    let before = f.units[u].seed;
    durability_hit(&mut f, u, wpn);
    assert_eq!(f.units[u].seed, before);
    // Plain weapon: 4 %.
    f.items[wpn].throw = false;
    f.units[u].seed = seed_giving(3);
    durability_hit(&mut f, u, wpn);
    assert_eq!(f.log, ["durability 0 0"]);
    f.units[u].seed = seed_giving(4);
    durability_hit(&mut f, u, wpn);
    assert_eq!(f.log.len(), 1);
}

#[test]
fn start_combat_and_apply_melee() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(21, 10).with(22, 10));
    let d = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 1_000)
            .with(7, 1_000),
    );
    let mut rec = DamageRecord {
        result: hit::result::HIT,
        ..DamageRecord::default()
    };
    start_combat(&mut f, &s, &c, Some(a), Some(d), &mut rec, 0);
    // 2560 + roll(256) physical > 1000 life: will die.
    assert_ne!(rec.result & hit::result::WILL_DIE, 0);
    assert_eq!(f.units[a].combat.len(), 1);
    assert_eq!(f.units[a].combat[0].record, rec);
    // A dodged hit is not rolled but still listed.
    let mut dodged = DamageRecord {
        result: hit::result::HIT | hit::result::DODGE,
        ..DamageRecord::default()
    };
    let seed = f.units[a].seed;
    start_combat(&mut f, &s, &c, Some(a), Some(d), &mut dodged, 0);
    assert_eq!((dodged.physical, f.units[a].seed), (0, seed));
    assert_eq!(f.units[a].combat.len(), 2);
    // apply_melee takes the first (newest) record for the defender.
    f.log.clear();
    apply_melee(&mut f, &c, a, d);
    assert!(f.units[a].combat.is_empty());
    assert!(f.log.contains(&"event 7 0 1".to_string()));
    assert_eq!(f.log.last().unwrap(), "reaction 0 1");
    assert_eq!(f.get(d, 6), 1_000);
}

#[test]
fn apply_kills_and_town_rule() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 1_000)
            .with(7, 1_000),
    );
    let mut rec = DamageRecord {
        result: hit::result::HIT,
        physical: 2_000,
        total: 2_000,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!(f.get(d, 6), 0);
    assert_ne!(rec.result & hit::result::WILL_DIE, 0);
    let tail: Vec<&str> = f.log.iter().rev().take(2).map(String::as_str).collect();
    assert_eq!(tail, ["event 9 0 1", "event 10 1 0"]);
    // A non-lethal hit keeps life ≥ 256, clears will-die.
    f.set(d, 6, 1_000);
    let mut rec = DamageRecord {
        result: hit::result::HIT | hit::result::WILL_DIE,
        total: 700,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!(f.get(d, 6), 300);
    assert_eq!(rec.result & hit::result::WILL_DIE, 0);
    assert_eq!(f.log.last().unwrap(), "mondamaged 1");
    // Below 256 after the hit: 0.
    let mut rec = DamageRecord {
        total: 100,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!(f.get(d, 6), 0);
    // Town: player attackers clear will-die and stop.
    f.set(d, 6, 1_000);
    f.units[d].room = RoomKind::Town;
    let mut rec = DamageRecord {
        result: hit::result::HIT | hit::result::WILL_DIE,
        total: 2_000,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!((f.get(d, 6), rec.result), (1_000, hit::result::HIT));
    // Not hostile: hit and will-die cleared.
    f.units[d].room = RoomKind::Field;
    f.hostile = false;
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!(rec.result, 0);
}

// ================================================================ game files

/// `charstats.bin` factors and `difficultylevels.bin` values equal the constants the
/// combat tests assume.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_table_constants() {
    use crate::skills::tests_game as game;
    use d2_data::tables::{decode_all, Charstats, Difficultylevels, Record};
    let cs: Vec<Charstats> = decode_all(&game::table("charstats", Charstats::SIZE)).unwrap();
    let tohit: Vec<i32> = cs.iter().take(7).map(|c| c.tohitfactor as i32).collect();
    let block: Vec<u8> = cs.iter().take(7).map(|c| c.blockfactor).collect();
    assert_eq!(tohit, [5, -15, -10, 20, 20, 5, 15]);
    assert_eq!(block, [25, 20, 20, 30, 25, 20, 25]);
    let dl: Vec<Difficultylevels> =
        decode_all(&game::table("difficultylevels", Difficultylevels::SIZE)).unwrap();
    let row = |d: &Difficultylevels| {
        [
            d.resistpenalty as i32,
            d.monsterfreezedivisor as i32,
            d.monstercolddivisor as i32,
            d.lifestealdivisor as i32,
            d.manastealdivisor as i32,
            d.hireablebossdamagepercent as i32,
        ]
    };
    let got: Vec<[i32; 6]> = dl.iter().map(row).collect();
    assert_eq!(
        got,
        [
            [0, 1, 1, 1, 1, 50],
            [-40, 2, 2, 2, 2, 35],
            [-100, 4, 4, 3, 3, 25]
        ]
    );
}
