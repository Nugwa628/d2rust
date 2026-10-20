//! Tests for experience on a kill and on death: gain, distribution, party share, death
//! penalties and corpse experience, over synthetic tables.
use super::experience::*;
use super::*;
use d2_data::tables::Record;
use std::collections::BTreeMap;

#[derive(Default)]
struct U {
    ty: Option<UnitType>,
    base: BTreeMap<u16, i32>,
}

#[derive(Default)]
struct W {
    units: Vec<U>,
    log: Vec<String>,
    credited: Option<usize>,
    party: Option<Vec<usize>>,
}

impl W {
    fn add(&mut self, ty: UnitType, stats: &[(u16, i32)]) -> usize {
        self.units.push(U {
            ty: Some(ty),
            base: stats.iter().copied().collect(),
        });
        self.units.len() - 1
    }
    fn get(&self, u: usize, s: u16) -> i32 {
        self.base_stat(u, s)
    }
}

impl VitalsUnits for W {
    type Unit = usize;
    fn unit_type(&self, u: usize) -> UnitType {
        self.units[u].ty.unwrap_or(UnitType::Player)
    }
    fn class_id(&self, _: usize) -> i32 {
        0
    }
    fn base_stat(&self, u: usize, s: u16) -> i32 {
        self.units[u].base.get(&s).copied().unwrap_or(0)
    }
    fn stat(&self, u: usize, s: u16) -> i32 {
        self.base_stat(u, s)
    }
    fn set_base_stat(&mut self, u: usize, s: u16, v: i32) {
        self.units[u].base.insert(s, v);
    }
    fn add_base_stat(&mut self, u: usize, s: u16, v: i32) {
        *self.units[u].base.entry(s).or_default() += v;
    }
    fn max_life(&self, _: usize) -> i32 {
        0
    }
    fn max_mana(&self, _: usize) -> i32 {
        0
    }
    fn max_stamina(&self, _: usize) -> i32 {
        0
    }
    fn refresh(&mut self, _: usize) {}
    fn level_up_notify(&mut self, u: usize) {
        self.log.push(format!("notify {u}"));
    }
    fn level_up_event(&mut self, u: usize) {
        self.log.push(format!("event12 {u}"));
    }
}

impl ExpShare for W {
    fn credited_player(&self, _: usize, _: usize) -> Option<usize> {
        self.credited
    }
    fn hireling_share(&mut self, p: usize, a: usize, d: usize, e: i32) {
        self.log.push(format!("hireling {p} {a} {d} {e}"));
    }
    fn in_party(&self, _: usize) -> bool {
        self.party.is_some()
    }
    fn party_members(&self, _: usize, _: usize) -> Vec<usize> {
        self.party.clone().unwrap_or_default()
    }
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// `MaxLvl` 99, level L → 500 × L²; `ExpRatio`: shift row 10, 1024 for levels 1–69,
/// −48 per level to 256 at 85, then the tail.
fn tables() -> VitalsTables {
    let row = |v: u32, ratio: u32| {
        let mut e: Experience = blank();
        e.amazon = v;
        e.expratio = ratio;
        e
    };
    const TAIL: [u32; 14] = [192, 144, 108, 81, 61, 46, 35, 26, 20, 15, 11, 8, 6, 5];
    let ratio = |l: u32| match l {
        0..=69 => 1024,
        70..=85 => 1024 - 48 * (l - 69),
        _ => TAIL[(l - 86) as usize],
    };
    let mut experience = vec![row(99, 10)];
    experience.extend((0..=99).map(|l| row(500 * l * l, ratio(l))));
    VitalsTables {
        charstats: Vec::new(),
        experience,
    }
}

const XP: u16 = stat::EXPERIENCE;
const LVL: u16 = stat::LEVEL;

#[test]
fn exp_ratio_lookup_and_step() {
    let t = tables();
    assert_eq!(t.exp_ratio(0), 10);
    assert_eq!(t.exp_ratio(-5), 10);
    assert_eq!(t.exp_ratio(1), 1024);
    assert_eq!(t.exp_ratio(70), 976);
    assert_eq!(t.exp_ratio(85), 256);
    assert_eq!(t.exp_ratio(99), 5);
    assert_eq!(t.exp_ratio(100), 0);
    // r = 1024: unchanged up to 1048575 (limit 0x7FFFFFFF >> 11), above
    // the low 10 bits drop.
    assert_eq!(t.apply_exp_ratio(1_048_575, 10), 1_048_575);
    assert_eq!(t.apply_exp_ratio(1_048_576 + 1023, 10), 1_048_576);
    // r = 256 at 85: a quarter; non-positive g unchanged.
    assert_eq!(t.apply_exp_ratio(1000, 85), 250);
    assert_eq!(t.apply_exp_ratio(-7, 85), -7);
    // s − 1 ≥ 31 (s = 0): unchanged.
    let mut z = tables();
    z.experience[0].expratio = 0;
    assert_eq!(z.apply_exp_ratio(1000, 85), 1000);
    // No table: ratio 0.
    let empty = VitalsTables {
        charstats: Vec::new(),
        experience: Vec::new(),
    };
    assert_eq!(empty.exp_ratio(5), 0);
}

#[test]
fn player_gain_rules() {
    let t = tables();
    let mut w = W::default();
    let p = w.add(UnitType::Player, &[]);
    // e ≤ 0 → 1; alvl ≥ max_level → 0.
    assert_eq!(player_gain(&w, &t, 0, p, 10, 10), 1);
    assert_eq!(player_gain(&w, &t, -5, p, 10, 10), 1);
    assert_eq!(player_gain(&w, &t, 1000, p, 99, 10), 0);
    // Level factor (alvl 30, dlvl 22 → 429), ratio 1024 (unchanged).
    assert_eq!(player_gain(&w, &t, 1000, p, 30, 22), 429);
    // Cap 0x7FFFFF before the factor; ratio drops the low bits.
    assert_eq!(
        player_gain(&w, &t, 0x7FFF_FFFF, p, 10, 10),
        (0x7F_FFFF >> 10) * 1024
    );
    // Stat 85: + pct(g, x, 100).
    w.set_base_stat(p, ADDEXPERIENCE, 50);
    assert_eq!(player_gain(&w, &t, 1000, p, 10, 10), 1500);
}

#[test]
fn add_sets_lastexp_caps_and_levels_against_l0() {
    let t = tables();
    let mut w = W::default();
    let p = w.add(UnitType::Player, &[(XP, 400), (LVL, 1)]);
    add_experience_at(&mut w, &t, p, 1, 50);
    assert_eq!((w.get(p, XP), w.get(p, 29)), (450, 50));
    assert!(w.log.is_empty());
    // Level from 600 is 2 ≠ L0 1: level-up and event 12.
    add_experience_at(&mut w, &t, p, 1, 150);
    assert_eq!(w.get(p, XP), 600);
    assert!(w.log.contains(&format!("event12 {p}")));
    // A level equal to L0 does nothing more (L0 is the caller's).
    w.log.clear();
    add_experience_at(&mut w, &t, p, 2, 10);
    assert!(w.log.is_empty());
    // Cap at threshold(max_level − 1) (unsigned compare); lastexp is the
    // capped difference.
    let cap = t.threshold(0, 98);
    add_experience_at(&mut w, &t, p, 99, i32::MAX);
    add_experience_at(&mut w, &t, p, 99, i32::MAX);
    assert_eq!(w.get(p, XP) as u32, cap);
    assert_eq!(w.get(p, 29), 0);
    // Not a player: nothing.
    let m = w.add(UnitType::Monster, &[]);
    add_experience_at(&mut w, &t, m, 1, 100);
    assert_eq!(w.get(m, XP), 0);
}

#[test]
fn distribution_solo_and_credit() {
    let t = tables();
    let mut w = W::default();
    // Level 30 holds [500 · 29², 500 · 30²).
    let p = w.add(UnitType::Player, &[(LVL, 30), (XP, 420_500)]);
    let d = w.add(UnitType::Monster, &[(XP, 1000), (LVL, 22)]);
    distribute(&mut w, &t, p, d);
    assert_eq!(w.get(p, XP), 420_500 + 429);
    // The hireling share runs first and does not reduce the player's.
    assert_eq!(w.log[0], format!("hireling {p} {p} {d} 1000"));
    // A monster attacker credits its player.
    let pet = w.add(UnitType::Monster, &[]);
    w.credited = Some(p);
    distribute(&mut w, &t, pet, d);
    assert_eq!(w.get(p, XP), 420_500 + 858);
    // No credited player, an object attacker, or no experience: nothing.
    w.credited = None;
    w.log.clear();
    distribute(&mut w, &t, pet, d);
    let o = w.add(UnitType::Object, &[]);
    distribute(&mut w, &t, o, d);
    let d0 = w.add(UnitType::Monster, &[(LVL, 22)]);
    distribute(&mut w, &t, p, d0);
    assert!(w.log.is_empty());
    assert_eq!(w.get(p, XP), 420_500 + 858);
}

#[test]
fn party_share_rules() {
    let t = tables();
    let mut w = W::default();
    let p = w.add(UnitType::Player, &[(LVL, 10), (XP, 40_500)]);
    let q = w.add(UnitType::Player, &[(LVL, 30), (XP, 420_500)]);
    let d = w.add(UnitType::Monster, &[(XP, 1000), (LVL, 10)]);
    // n = 1: P's own gain at P's level (even when the counted member is
    // another unit).
    w.party = Some(vec![q]);
    distribute(&mut w, &t, p, d);
    assert_eq!((w.get(p, XP), w.get(q, XP)), (41_500, 420_500));
    // n = 2: t = 1000 + (1 · 1000 · 89) / 256 = 1347; q = 1347 / 40;
    // shares trunc(10 · q) = 336, trunc(30 · q) = 1010.
    w.party = Some(vec![p, q]);
    distribute(&mut w, &t, p, d);
    assert_eq!(w.get(p, XP), 41_500 + 336);
    // Member q: level 30 vs dlvl 10 → factor 13/256 of 1010.
    assert_eq!(w.get(q, XP), 420_500 + crate::combat::pct(1010, 13, 256));
    // S ≤ 0: nothing.
    let z = w.add(UnitType::Player, &[]);
    w.party = Some(vec![z]);
    let before = w.get(p, XP);
    distribute(&mut w, &t, p, d);
    assert_eq!(w.get(p, XP), before);
}

#[test]
fn party_float_emulation_matches_ieee() {
    // The integer emulation against IEEE float32 / f64 (test-only floats).
    let mut s = crate::rng::Seed::init_low(7);
    for _ in 0..20_000 {
        let t = s.roll(0x7FFF_FFFF) as i32 * if s.roll(5) == 0 { -1 } else { 1 };
        let sum = 1 + s.roll(792) as i32;
        let want = (f64::from(t) / f64::from(sum)) as f32;
        let q = party_quotient(t, sum);
        for l in [1, 7, 50, 99] {
            let exact = (f64::from(l) * f64::from(want)).trunc();
            // Outside i32: i32::MIN (0x80000000).
            let exact = if exact.abs() >= 2_147_483_648.0 {
                f64::from(i32::MIN)
            } else {
                exact
            };
            assert_eq!(party_member_share(l, q) as f64, exact, "{t} / {sum} × {l}");
        }
    }
    assert_eq!(party_member_share(5, party_quotient(0, 3)), 0);
}

#[test]
fn gold_penalty_vectors() {
    // Single player (type 3): L 10, 1000 + 9000: q = 1000, keep 9000 ≥
    // 5000; q = min(1000, 1000). Not pvp, q ≤ gi: drop gi − q = 0.
    let g = gold_penalty(10, 1000, 9000, false, 3, i32::MAX, i32::MAX);
    assert_eq!(
        g,
        GoldPenalty {
            drop: Some(0),
            gold: Some(0),
            goldbank: None,
            goldlost: 1000
        }
    );
    // keep < base: q = max(T − base, 0) = 0.
    let g = gold_penalty(10, 100, 100, false, 3, i32::MAX, i32::MAX);
    assert_eq!((g.drop, g.goldlost), (Some(100), 0));
    // Other game type, q > gi, not pvp: the bank pays the rest.
    let g = gold_penalty(20, 100, 9900, false, 0, i32::MAX, i32::MAX);
    assert_eq!(
        g,
        GoldPenalty {
            drop: None,
            gold: Some(0),
            goldbank: Some(9900 - 1900),
            goldlost: 2000
        }
    );
    // pvp, q > gi: stat 14 := q, then drop q.
    let g = gold_penalty(20, 100, 9900, true, 0, i32::MAX, 1000);
    assert_eq!(
        (g.gold, g.drop, g.goldbank),
        (Some(0), Some(2000), Some(8000))
    );
}

#[test]
fn death_experience_vectors() {
    let t = tables();
    let mut w = W::default();
    // L 10: lo = thr(9) = 40500, hi = thr(10) = 50000; 10 % of 9500.
    let p = w.add(UnitType::Player, &[(LVL, 10), (XP, 45_000)]);
    assert_eq!(death_experience(&mut w, &t, p, 10), Some(950));
    assert_eq!(w.get(p, XP), 44_050);
    // Below the level's first point: loss = x − lo, new = lo + 1.
    w.set_base_stat(p, XP, 41_000);
    assert_eq!(death_experience(&mut w, &t, p, 10), Some(500));
    assert_eq!(w.get(p, XP), 40_501);
    // Penalty 0 or level 1: nothing.
    assert_eq!(death_experience(&mut w, &t, p, 0), None);
    let n = w.add(UnitType::Player, &[(LVL, 1), (XP, 400)]);
    assert_eq!(death_experience(&mut w, &t, n, 10), None);
    assert_eq!(w.get(n, XP), 400);
}

#[test]
fn corpse_experience_round_trip() {
    let t = tables();
    // 75 %, toward zero; above 0x100000 the division comes first.
    assert_eq!(corpse_experience(1050), 787);
    assert_eq!(corpse_experience(0), 0);
    assert_eq!(corpse_experience(0x10_0001), (0x10_0001 / 100) * 75);
    assert_eq!(corpse_experience(0x10_0000), 0x10_0000 * 75 / 100);
    let mut w = W::default();
    let p = w.add(UnitType::Player, &[(LVL, 10), (XP, 53_950)]);
    let c = w.add(UnitType::Player, &[(XP, 787)]);
    // Another player's corpse: nothing.
    assert_eq!(corpse_pickup(&mut w, &t, p, c, false), 0);
    assert_eq!(corpse_pickup(&mut w, &t, p, c, true), 787);
    assert_eq!((w.get(p, XP), w.get(c, XP)), (54_737, 0));
    // A second pickup returns nothing.
    assert_eq!(corpse_pickup(&mut w, &t, p, c, true), 0);
}
