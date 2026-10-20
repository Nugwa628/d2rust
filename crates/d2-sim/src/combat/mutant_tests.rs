//! Boundary and branch tests for combat, written against mutants of the combat code that
//! survived the other tests (found with `cargo mutants`). Each test asserts the rule at the
//! boundary or branch a mutant changed; the fake world is `skills::fake`.
use super::*;
use crate::rng::Seed;
use crate::skills::fake::*;
use crate::skills::SkillTables;
use crate::units::UnitType;

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

/// `seed` after `n` steps.
fn stepped(seed: Seed, n: usize) -> Seed {
    let mut s = seed;
    for _ in 0..n {
        s.step();
    }
    s
}

/// A seed whose next step gives `lo′ = x`.
fn seed_giving(x: u32) -> Seed {
    Seed::new(0, x)
}

// ---------------------------------------------------- percent helper

// The thresholds are strict (`v > 0x100000`, `p > 0x10000`)
// and the `p` branch compares `d` with `p >> 4`.
#[test]
fn pct_thresholds() {
    // v = 0x100000 does not take the large-`v` branch: the plain 32-bit product wraps to 0.
    assert_eq!(pct(0x10_0000, 0x1000, 3), 0);
    // p = 0x10000 does not take the large-`p` branch: the plain 32-bit product again.
    assert_eq!(pct(0x1_0000, 0x1_0000, 3), 0);
    // p > 0x10000 and d > p >> 4 (0x2000): the 64-bit product.
    // 7 × 0x20000 / 0x3000 = 74 (the shortcut would give 10 × 7 = 70).
    assert_eq!(pct(7, 0x2_0000, 0x3000), 74);
    // d ≤ p >> 4: the shortcut (p / d) × v.
    assert_eq!(pct(7, 0x2_0000, 0x2000), 112);
}

// ----------------------------------------------------- roll in range

// `roll_in_range`: nothing is added and nothing drawn unless `max > 0`.
#[test]
fn roll_in_range_needs_positive_max() {
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    let before = f.units[u].seed;
    assert_eq!(roll_in_range(&mut f, u, -512, 0, 0, 0, 1000), 1000);
    assert_eq!(f.units[u].seed, before);
}

// ------------------------------------------------------ damage range

// `min < 1` → 256, so a given min of 1 stays 1 (and `max ≤ min`
// → `max = min + 256`).
#[test]
fn bonuses_min_one_is_kept() {
    let s = st();
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    // lo′ = 0 on every step: roll(256) = 0.
    f.units[u].seed = Seed::new(0, 0);
    assert_eq!(bonuses(&mut f, &s, u, false, None, 1, 1, 0, 0, 128), 1);
    assert_eq!(bonuses(&mut f, &s, u, false, None, 0, 1, 0, 0, 128), 256);
}

// ------------------------------------------------------- damage fill

/// A player with a weapon of item type 50 (min 1, max 3) against a
/// monster.
fn armed_case(f: &mut Fake) -> (usize, usize) {
    let a = f.add(FUnit::new(UnitType::Player, 0).with(21, 1).with(22, 3));
    let w = f.add_item(FItem {
        types: vec![50],
        ..FItem::default()
    });
    f.units[a].weapon = Some(w);
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 100_000));
    (a, d)
}

/// A seed whose second draw (the crit draw after the physical roll)
/// gives `r = lo′ mod 100 ≥ 1`, and that `r`.
fn crit_seed() -> (Seed, i32) {
    (1u32..)
        .map(|lo| {
            let s = Seed::new(lo, 0);
            let mut t = stepped(s, 1);
            (s, (t.step() % 100) as i32)
        })
        .find(|&(_, r)| r >= 1)
        .unwrap()
}

/// Fills `rec` for the armed case with `offhand`, returning whether the
/// result is critical and how many attacker draws were made.
fn crit_fill(f: &mut Fake, a: usize, d: usize, offhand: bool, seed: Seed) -> (bool, usize) {
    let (s, c) = (st(), ct());
    f.units[a].seed = seed;
    let mut rec = DamageRecord::default();
    fill(f, &s, &c, a, d, &mut rec, offhand, 128);
    let crit = rec.result & result::CRITICAL != 0;
    let n = (0..8)
        .find(|&n| stepped(seed, n) == f.units[a].seed)
        .expect("draw count");
    (crit, n)
}

// The weapon-mastery crit needs `offhand = 0`, a current
// weapon and `m > 0`; it draws once and crits on `r < m`. Draws: physical
// roll, crit draw (when made), burn roll(1).
#[test]
fn fill_mastery_crit_threshold() {
    let mut f = world();
    let (a, d) = armed_case(&mut f);
    let (seed, r) = crit_seed();
    // m = r + 1: crit.
    f.units[a].entries.insert(344, vec![(50, r + 1)]);
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (true, 3));
    // m = r: no crit (and no other source).
    f.units[a].entries.insert(344, vec![(50, r)]);
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (false, 3));
    // Off-hand: no mastery draw.
    f.units[a].entries.insert(344, vec![(50, r + 1)]);
    assert_eq!(crit_fill(&mut f, a, d, true, seed), (false, 2));
    // m = 0: no draw.
    f.units[a].entries.clear();
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (false, 2));
}

// `passive_critical_strike` then
// `item_deadlystrike`, each `> 0` → draw, `r < value` → crit.
#[test]
fn fill_passive_and_deadly_crit_threshold() {
    let mut f = world();
    let (a, d) = armed_case(&mut f);
    let (seed, r) = crit_seed();
    f.set(a, 337, r + 1);
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (true, 3));
    f.set(a, 337, r);
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (false, 3));
    f.set(a, 337, 0);
    f.set(a, 141, r + 1);
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (true, 3));
    f.set(a, 141, r);
    assert_eq!(crit_fill(&mut f, a, d, false, seed), (false, 3));
}

/// Fills a fresh record with `hit_flags`, returning it.
fn fill_flags(f: &mut Fake, a: usize, d: usize, hit_flags: u32) -> DamageRecord {
    let (s, c) = (st(), ct());
    let mut rec = DamageRecord {
        hit_flags,
        ..DamageRecord::default()
    };
    fill(f, &s, &c, a, d, &mut rec, false, 128);
    rec
}

// Monster attacker (not a hireling): each drain is rolled
// unless its preset flag (4 / 8 / 0x10) is set. min = max: no draw.
#[test]
fn fill_monster_drains_and_presets() {
    let mut f = world();
    let a = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(60, 2)
            .with(61, 2)
            .with(62, 3)
            .with(63, 3)
            .with(64, 4)
            .with(65, 4),
    );
    let d = f.add(FUnit::new(UnitType::Player, 0));
    let r = fill_flags(&mut f, a, d, 0);
    assert_eq!(
        (r.life_leech, r.mana_leech, r.stamina_leech),
        (512, 768, 1024)
    );
    let r = fill_flags(&mut f, a, d, hitflag::LIFE_DRAIN_PRESET);
    assert_eq!(
        (r.life_leech, r.mana_leech, r.stamina_leech),
        (0, 768, 1024)
    );
    let r = fill_flags(&mut f, a, d, hitflag::MANA_DRAIN_PRESET);
    assert_eq!(
        (r.life_leech, r.mana_leech, r.stamina_leech),
        (512, 0, 1024)
    );
    let r = fill_flags(&mut f, a, d, hitflag::STAMINA_DRAIN_PRESET);
    assert_eq!((r.life_leech, r.mana_leech, r.stamina_leech), (512, 768, 0));
}

// Player or hireling attacker: plain percents unless preset,
// and the bypass flags; other attackers: nothing.
#[test]
fn fill_player_and_hireling_drains() {
    let mut f = world();
    let mut stats = FUnit::new(UnitType::Player, 0)
        .with(60, 7)
        .with(61, 9)
        .with(62, 5)
        .with(63, 9)
        .with(103, 1)
        .with(104, 1)
        .with(106, 1);
    let p = f.add(stats.clone());
    stats.kind = UnitType::Monster;
    stats.hireling = true;
    let h = f.add(stats.clone());
    stats.kind = UnitType::Object;
    stats.hireling = false;
    let o = f.add(stats);
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let bypass = hitflag::BYPASS_UNDEAD | hitflag::BYPASS_DEMONS | hitflag::BYPASS_BEASTS;
    for a in [p, h] {
        let r = fill_flags(&mut f, a, d, 0);
        assert_eq!((r.life_leech, r.mana_leech), (7, 5), "{a}");
        assert_eq!(r.hit_flags & bypass, bypass, "{a}");
        let r = fill_flags(&mut f, a, d, hitflag::LIFE_DRAIN_PRESET);
        assert_eq!((r.life_leech, r.mana_leech), (0, 5), "{a}");
        let r = fill_flags(&mut f, a, d, hitflag::MANA_DRAIN_PRESET);
        assert_eq!((r.life_leech, r.mana_leech), (7, 0), "{a}");
    }
    // Only the bypass stats that are set give their flag.
    f.set(p, 104, 0);
    let r = fill_flags(&mut f, p, d, 0);
    assert_eq!(
        r.hit_flags & bypass,
        hitflag::BYPASS_UNDEAD | hitflag::BYPASS_BEASTS
    );
    f.set(p, 103, 0);
    f.set(p, 106, 0);
    f.set(p, 104, 1);
    let r = fill_flags(&mut f, p, d, 0);
    assert_eq!(r.hit_flags & bypass, hitflag::BYPASS_DEMONS);
    // Flags already set are kept.
    f.set(p, 104, 0);
    let r = fill_flags(&mut f, p, d, hitflag::BYPASS_UNDEAD);
    assert_eq!(r.hit_flags & bypass, hitflag::BYPASS_UNDEAD);
    // An object attacker: no drains, no bypass flags.
    let r = fill_flags(&mut f, o, d, 0);
    assert_eq!(
        (r.life_leech, r.mana_leech, r.hit_flags & bypass),
        (0, 0, 0)
    );
}

// `m = passive_pois_mastery` when `max > 0`, applied to both
// ends. min = max = 256, m = 50: 384, no draw.
#[test]
fn fill_poison_mastery() {
    let mut f = world();
    let a = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(57, 256)
            .with(58, 256)
            .with(332, 50),
    );
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let r = fill_flags(&mut f, a, d, hitflag::SKIP_PHYSICAL);
    assert_eq!(r.poison, 384);
}

// ----------------------------------------------------------- shatter

// A monster with `monstats2.deadCol` never shatters.
#[test]
fn cold_shatter_needs_no_deadcol() {
    let mut m = monster_rec();
    m.coldeffect = (-30i8) as u8;
    let mut c = combat_tables(vec![m]);
    c.monstats2[0].deadcol = true;
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let mo = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[mo].seed = seed_giving(19);
    cold(&mut f, &c, a, mo, 41);
    assert_eq!(f.log.last().unwrap(), "state 1 107 false");
}

// Event 3 only when the result lacks 0x20.
#[test]
fn fill_event_needs_events() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let (s, c) = (st(), ct());
    let mut rec = DamageRecord {
        result: result::NO_EVENTS,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 128);
    assert!(f.log.is_empty(), "{:?}", f.log);
    rec.result = 0;
    fill(&mut f, &s, &c, a, d, &mut rec, false, 128);
    assert_eq!(f.log, ["event 3 1 0"]);
}

// A monster attacker sends event 3 only when it is a
// hireling.
#[test]
fn fill_event_monster_needs_hireling() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    fill_flags(&mut f, a, d, 0);
    assert!(f.log.is_empty(), "{:?}", f.log);
    f.units[a].hireling = true;
    fill_flags(&mut f, a, d, 0);
    assert_eq!(f.log, ["event 3 1 0"]);
}

/// Fills a player's record with physical 1000 preset (hit flag 1),
/// conversion `elem` at 50 %.
fn convert(f: &mut Fake, a: usize, d: usize, elem: i8) -> DamageRecord {
    let (s, c) = (st(), ct());
    let mut rec = DamageRecord {
        hit_flags: hitflag::SKIP_PHYSICAL,
        physical: 1000,
        conv_elem: elem,
        conv_pct: 50,
        ..DamageRecord::default()
    };
    fill(f, &s, &c, a, d, &mut rec, false, 128);
    rec
}

// Conversion only when the conversion element is > 0; c = 500 goes to the element's
// field; poison gets c / 8; burn and freeze set their length to at least 50.
#[test]
fn fill_conversion_elements() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let r = convert(&mut f, a, d, 0);
    assert_eq!(r.physical, 1000);
    let r = convert(&mut f, a, d, 1);
    assert_eq!((r.physical, r.fire), (500, 500));
    let r = convert(&mut f, a, d, 2);
    assert_eq!((r.physical, r.lightning), (500, 500));
    let r = convert(&mut f, a, d, 3);
    assert_eq!((r.physical, r.magic), (500, 500));
    let r = convert(&mut f, a, d, 5);
    assert_eq!((r.physical, r.poison, r.poison_len), (500, 62, 50));
    let r = convert(&mut f, a, d, 11);
    assert_eq!((r.physical, r.fire, r.burn_len), (500, 500, 50));
    let r = convert(&mut f, a, d, 12);
    assert_eq!((r.physical, r.cold, r.freeze_len), (500, 500, 50));
    for e in 6..=9 {
        let r = convert(&mut f, a, d, e);
        assert_eq!(
            (r.physical, r.fire, r.lightning, r.magic, r.cold, r.poison),
            (500, 0, 0, 0, 0, 0),
            "{e}"
        );
    }
}

// Element 10 is `roll(5) + 1` on the attacker seed, drawn
// after the burn roll(1). roll(5) = 1 → lightning.
#[test]
fn fill_conversion_random_element() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let seed = (1u32..)
        .map(|lo| Seed::new(lo, 0))
        .find(|&s| stepped(s, 1).step() % 5 == 1)
        .unwrap();
    f.units[a].seed = seed;
    let r = convert(&mut f, a, d, 10);
    assert_eq!((r.fire, r.lightning), (0, 500));
    assert_eq!(f.units[a].seed, stepped(seed, 2));
}

// The monster crit draws only when `Crit ≠ 0` and doubles
// on `r < Crit`. Draws: burn roll(1), then the crit step.
#[test]
fn fill_monster_crit_threshold() {
    let (seed, r) = crit_seed();
    let s = st();
    for (crit, doubled, draws) in [(0, false, 1), (r, false, 2), (r + 1, true, 2)] {
        let mut m = monster_rec();
        m.crit = crit as u8;
        let c = combat_tables(vec![m]);
        let mut f = world();
        let a = f.add(FUnit::new(UnitType::Monster, 0));
        let d = f.add(FUnit::new(UnitType::Player, 0));
        f.units[a].seed = seed;
        let mut rec = DamageRecord {
            hit_flags: hitflag::SKIP_PHYSICAL,
            physical: 1000,
            ..DamageRecord::default()
        };
        fill(&mut f, &s, &c, a, d, &mut rec, false, 128);
        let want = if doubled { 2000 } else { 1000 };
        assert_eq!(rec.physical, want, "crit {crit}");
        assert_eq!(f.units[a].seed, stepped(seed, draws), "crit {crit}");
    }
}

// ------------------------------------------------------ start combat

/// `start_combat` with a preset record (hit flags `hf`, result `res`,
/// physical `phys`) from `a` on `d`.
fn start(f: &mut Fake, a: usize, d: usize, hf: u32, res: u16, phys: i32) -> DamageRecord {
    let (s, c) = (st(), ct());
    let mut rec = DamageRecord {
        hit_flags: hf,
        result: res,
        physical: phys,
        ..DamageRecord::default()
    };
    start_combat(f, &s, &c, Some(a), Some(d), &mut rec, 128);
    rec
}

// No roll when the result has dodge, avoid, evade or weapon
// block (mask 0x8380).
#[test]
fn start_combat_avoided_results() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 1));
    let r = start(&mut f, a, d, 0, result::HIT, 0x1_0000);
    assert_ne!(r.hit_flags & hitflag::ROLLED, 0);
    assert_ne!(r.result & result::WILL_DIE, 0);
    for flag in [
        result::DODGE,
        result::AVOID,
        result::EVADE,
        result::WEAPON_BLOCK,
    ] {
        let r = start(&mut f, a, d, 0, result::HIT | flag, 0x1_0000);
        assert_eq!(r.hit_flags & hitflag::ROLLED, 0, "{flag:#x}");
        assert_eq!(r.result & result::WILL_DIE, 0, "{flag:#x}");
    }
}

// The comparison drops the low byte of both sides and is
// strict; a monster attacker adds its life leech.
#[test]
fn start_combat_will_die_compare() {
    let mut f = world();
    let p = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 0x1C0));
    let hf = hitflag::SKIP_ROLL;
    // 0x180 & ~0xFF = 0x100 = 0x1C0 & ~0xFF: not greater.
    let r = start(&mut f, p, d, hf, result::HIT, 0x180);
    assert_eq!(r.result & result::WILL_DIE, 0);
    let r = start(&mut f, p, d, hf, result::HIT, 0x200);
    assert_ne!(r.result & result::WILL_DIE, 0);
    // Life leech counts for a monster attacker only.
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    let pd = f.add(FUnit::new(UnitType::Player, 0).with(6, 0x100));
    let (s, c) = (st(), ct());
    for (a, dies) in [(m, true), (p, false)] {
        let mut rec = DamageRecord {
            hit_flags: hf,
            result: result::HIT,
            life_leech: 0x200,
            ..DamageRecord::default()
        };
        start_combat(&mut f, &s, &c, Some(a), Some(pd), &mut rec, 128);
        assert_eq!(rec.result & result::WILL_DIE != 0, dies, "{a}");
    }
}

// ------------------------------------------------------------ totals

// The first matching row wins.
#[test]
fn damage_percent_rows() {
    let c = ct();
    let mut f = world();
    let p = f.add(FUnit::new(UnitType::Player, 0));
    let mut hire = FUnit::new(UnitType::Monster, 0);
    hire.hireling = true;
    let h = f.add(hire);
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    let mut b = FUnit::new(UnitType::Monster, 0);
    b.boss = true;
    let boss = f.add(b);
    assert_eq!(damage_percent(&f, &c, Some(h), p), 17);
    assert_eq!(damage_percent(&f, &c, Some(m), p), 100);
    assert_eq!(damage_percent(&f, &c, Some(h), boss), 50);
    assert_eq!(damage_percent(&f, &c, Some(m), boss), 100);
    assert_eq!(damage_percent(&f, &c, Some(p), boss), 100);
}

/// `totals` of `rec` from `a` on `d`.
fn totals_of(f: &mut Fake, a: usize, d: usize, rec: DamageRecord) -> DamageRecord {
    let c = ct();
    let mut rec = rec;
    totals(f, &c, Some(a), d, &mut rec);
    rec
}

fn fire(v: i32) -> DamageRecord {
    DamageRecord {
        fire: v,
        ..DamageRecord::default()
    }
}

// Magic damage reduction is pierced only when the pierce percent
// is > 0: 5 << 8 = 1280 × 512 / 1024 = 640.
#[test]
fn totals_magic_dr_pierce() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(35, 5));
    let r = totals_of(
        &mut f,
        a,
        d,
        DamageRecord {
            pierce_pct: 512,
            ..fire(2560)
        },
    );
    assert_eq!(r.fire, 1920);
    let r = totals_of(&mut f, a, d, fire(2560));
    assert_eq!(r.fire, 1280);
}

// A monster at 100 % is not pierced (`r < 100`).
#[test]
fn resist_monster_immunity_not_pierced() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(333, 20));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(39, 100));
    assert_eq!(totals_of(&mut f, a, d, fire(1000)).fire, 0);
    f.set(d, 39, 99);
    // 99 − 20 = 79: 1000 × 21 / 100.
    assert_eq!(totals_of(&mut f, a, d, fire(1000)).fire, 210);
}

// Classic game, difficulty 1 → −20, difficulty 2 → −50.
#[test]
fn resist_classic_penalty() {
    let mut f = world();
    f.expansion = false;
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(39, 60));
    f.difficulty = 1;
    assert_eq!(totals_of(&mut f, a, d, fire(1000)).fire, 600);
    f.difficulty = 2;
    assert_eq!(totals_of(&mut f, a, d, fire(1000)).fire, 900);
}

// The cap applies to `r > 0` only; r = 0 stays 0 even with
// a max stat below −75.
#[test]
fn resist_cap_only_positive() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(40, -100));
    assert_eq!(totals_of(&mut f, a, d, fire(1000)).fire, 1000);
}

// Sanctuary zeroes the physical resist (stat 36) of an
// undead defender only.
#[test]
fn resist_sanctuary() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    f.units[a].states.push(47);
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(36, 50).with(39, 50));
    let rec = DamageRecord {
        physical: 1000,
        fire: 1000,
        ..DamageRecord::default()
    };
    let r = totals_of(&mut f, a, d, rec);
    assert_eq!((r.physical, r.fire), (500, 500));
    f.units[d].undead = true;
    let r = totals_of(&mut f, a, d, rec);
    assert_eq!((r.physical, r.fire), (1000, 500));
}

// Lengths are touched only when > 0.
#[test]
fn totals_length_guards() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(118, 1));
    let len = |cold_len, freeze_len| DamageRecord {
        cold_len,
        freeze_len,
        ..DamageRecord::default()
    };
    // Half freeze: a freeze length alone is enough.
    let r = totals_of(&mut f, a, d, len(0, 10));
    assert_eq!((r.cold_len, r.freeze_len), (0, 5));
    let r = totals_of(&mut f, a, d, len(10, 0));
    assert_eq!((r.cold_len, r.freeze_len), (5, 0));
    // Burn length and state 131.
    let burn = DamageRecord {
        burn_len: 10,
        ..DamageRecord::default()
    };
    assert_eq!(totals_of(&mut f, a, d, burn).burn_len, 10);
    f.units[d].states.push(131);
    assert_eq!(totals_of(&mut f, a, d, burn).burn_len, 0);
}

// `no_absorb` for a monster defender by its kind and the
// matching bypass flag (seen through a 20 % fire absorb).
#[test]
fn totals_no_absorb_matrix() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(142, 20));
    let bu = hitflag::BYPASS_UNDEAD;
    let bd = hitflag::BYPASS_DEMONS;
    let bb = hitflag::BYPASS_BEASTS;
    // (undead, demon, flags, no_absorb)
    for (undead, demon, hf, none) in [
        (true, false, bu, true),
        (true, false, bd | bb, false),
        (false, true, bd, true),
        (false, true, bu | bb, false),
        (false, false, bb, true),
        (false, false, bu | bd, false),
        (true, true, bb, false),
    ] {
        f.units[d].undead = undead;
        f.units[d].demon = demon;
        let r = totals_of(
            &mut f,
            a,
            d,
            DamageRecord {
                hit_flags: hf,
                ..fire(1000)
            },
        );
        let want = if none { (1000, 0) } else { (800, 200) };
        assert_eq!((r.fire, r.absorbed), want, "{undead} {demon} {hf:#x}");
    }
    // A player defender (monster attacker: damage percent 100): only
    // 0x400.
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    let p = f.add(FUnit::new(UnitType::Player, 0).with(142, 20));
    let r = totals_of(
        &mut f,
        m,
        p,
        DamageRecord {
            hit_flags: bu | bd,
            ..fire(1000)
        },
    );
    assert_eq!((r.fire, r.absorbed), (800, 200));
    let r = totals_of(
        &mut f,
        m,
        p,
        DamageRecord {
            hit_flags: bb,
            ..fire(1000)
        },
    );
    assert_eq!((r.fire, r.absorbed), (1000, 0));
}

// The flat absorb needs `f > 0`; a field left negative by
// damage reduction stays negative.
#[test]
fn totals_negative_field_kept() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(35, 5));
    let r = totals_of(&mut f, a, d, fire(100));
    assert_eq!((r.fire, r.absorbed), (100 - 1280, 0));
}

// ------------------------------------------------------------- leech

/// A leech case: attacker life 0 / max 100000, mana 0 / max 100000.
fn leecher(kind: UnitType) -> FUnit {
    FUnit::new(kind, 0)
        .with(6, 0)
        .with(7, 100_000)
        .with(8, 0)
        .with(9, 100_000)
}

fn leech_rec(life: i32, mana: i32, physical: i32) -> DamageRecord {
    DamageRecord {
        life_leech: life,
        mana_leech: mana,
        physical,
        ..DamageRecord::default()
    }
}

// Nightmare drain column and the player divisors:
// 10 << 6 = 640 / 2 = 320; 1000 × 320 % = 3200; drain 50 → 1600; / 64
// = 25.
#[test]
fn leech_nightmare_drain() {
    let mut m = monster_rec();
    m.drain_n = 50;
    let c = combat_tables(vec![m]);
    let mut f = world();
    f.difficulty = 1;
    let a = f.add(leecher(UnitType::Player));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let mut rec = leech_rec(10, 0, 1000);
    leech(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(f.get(a, 6), 25);
}

// A hireling takes the player rule without divisors: 640 % of
// 1000 = 6400 / 64 = 100 life (the monster rule would give 10).
#[test]
fn leech_hireling_player_rule() {
    let c = ct();
    let mut f = world();
    f.difficulty = 1;
    let mut h = leecher(UnitType::Monster);
    h.hireling = true;
    let a = f.add(h);
    let d = f.add(FUnit::new(UnitType::Player, 0));
    let mut rec = leech_rec(10, 0, 1000);
    leech(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!(f.get(a, 6), 100);
}

// Life only → overlay 151, no draw; mana only → overlay 152.
#[test]
fn leech_player_overlays() {
    let c = ct();
    let mut f = world();
    let a = f.add(leecher(UnitType::Player));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    let before = f.units[a].seed;
    leech(&mut f, &c, Some(a), d, &mut leech_rec(10, 0, 1000));
    assert_eq!((f.get(a, 6), f.get(a, 8)), (100, 0));
    assert_eq!(f.log.last().unwrap(), "overlay 0 151");
    assert_eq!(f.units[a].seed, before);
    f.log.clear();
    leech(&mut f, &c, Some(a), d, &mut leech_rec(0, 10, 1000));
    assert_eq!((f.get(a, 6), f.get(a, 8)), (100, 100));
    assert_eq!(f.log.last().unwrap(), "overlay 0 152");
    assert_eq!(f.units[a].seed, before);
}

// Monster rule: T = 10 is added with overlay 151; at full life
// T = 0 after the cap: nothing.
#[test]
fn leech_monster_rule() {
    let c = ct();
    let mut f = world();
    let a = f.add(leecher(UnitType::Monster).with(6, 100).with(7, 1000));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    leech(&mut f, &c, Some(a), d, &mut leech_rec(10, 0, 1000));
    assert_eq!(f.get(a, 6), 110);
    assert_eq!(f.log, ["set 0 6 110", "overlay 0 151"]);
    f.log.clear();
    f.set(a, 6, 1000);
    leech(&mut f, &c, Some(a), d, &mut leech_rec(10, 0, 1000));
    assert!(f.log.is_empty(), "{:?}", f.log);
}

// --------------------------------------------------- cold and freeze

// The Nightmare column of ColdEffect.
#[test]
fn cold_effect_nightmare() {
    let mut m = monster_rec();
    m.coldeffect_n = (-30i8) as u8;
    let c = combat_tables(vec![m]);
    let mut f = world();
    f.difficulty = 1;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    cold(&mut f, &c, a, d, 40);
    assert!(
        f.log.contains(&"liststat 1 11 67 -30".to_string()),
        "{:?}",
        f.log
    );
}

// Only a monster's length is divided (Hell divisor 4).
#[test]
fn cold_divisor_monsters_only() {
    let c = ct();
    let mut f = world();
    f.difficulty = 2;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    cold(&mut f, &c, a, d, 40);
    assert_eq!(f.log[1], "list 1 11 0 40");
}

// An existing list is extended only when its expiry < e.
#[test]
fn cold_existing_list_expiry() {
    let c = ct();
    for (x, extended) in [(39, true), (40, false), (41, false)] {
        let mut f = world();
        let a = f.add(FUnit::new(UnitType::Player, 0));
        let d = f.add(FUnit::new(UnitType::Player, 0));
        f.units[d].state_expiry.insert(11, x);
        cold(&mut f, &c, a, d, 40);
        let want: Vec<String> = if extended {
            vec!["expiry 1 11 40".into(), "timer 1 12 40".into()]
        } else {
            vec![]
        };
        assert_eq!(f.log[..f.log.len() - 1], want[..], "{x}");
    }
}

// With r = lo′ mod 100, shatter happens on r < 20.
#[test]
fn cold_shatter_draw() {
    let mut m = monster_rec();
    m.coldeffect = (-30i8) as u8;
    let c = combat_tables(vec![m]);
    for (x, on) in [(2019, true), (20, false), (19, true)] {
        let mut f = world();
        let a = f.add(FUnit::new(UnitType::Player, 0));
        let d = f.add(FUnit::new(UnitType::Monster, 0));
        f.units[d].seed = seed_giving(x);
        cold(&mut f, &c, a, d, 40);
        assert_eq!(f.log.last().unwrap(), &format!("state 1 107 {on}"), "{x}");
    }
}

// A player defender gets cold; a boss, unique or hireling monster
// gets cold; another monster's length is divided by the Hell freeze
// divisor (4).
#[test]
fn freeze_targets() {
    let mut m = monster_rec();
    m.coldeffect_h = (-50i8) as u8;
    let c = combat_tables(vec![m]);
    let cold_log = |f: &Fake| f.log.first().cloned();
    let mut f = world();
    f.difficulty = 2;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let p = f.add(FUnit::new(UnitType::Player, 0));
    freeze(&mut f, &c, p, a, 40);
    assert_eq!(cold_log(&f).as_deref(), Some("state 1 11 true"));
    for kind in 0..3 {
        let mut u = FUnit::new(UnitType::Monster, 0);
        match kind {
            0 => u.boss = true,
            1 => u.flags = 8,
            _ => u.hireling = true,
        }
        let mut f = world();
        f.difficulty = 2;
        let a = f.add(FUnit::new(UnitType::Player, 0));
        let d = f.add(u);
        freeze(&mut f, &c, d, a, 40);
        assert_eq!(cold_log(&f).as_deref(), Some("state 1 11 true"), "{kind}");
    }
    let mut f = world();
    f.difficulty = 2;
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    freeze(&mut f, &c, d, a, 40);
    assert_eq!(f.log[..2], ["state 1 1 true", "list 1 1 0 10"]);
}

// ------------------------------------------------------------- apply

/// `apply` of a melee hit record with `total` on a defender with life
/// 1000.
fn apply_case(
    f: &mut Fake,
    c: &CombatTables,
    a: usize,
    d: usize,
    rec: DamageRecord,
) -> DamageRecord {
    let mut rec = rec;
    apply(f, c, a, d, false, &mut rec);
    rec
}

// No room clears result 1 and 2 only; town returns unless the
// attacker is a monster with `inTown`.
#[test]
fn apply_room_rules() {
    let mut m = monster_rec();
    m.intown = true;
    let c = combat_tables(vec![m.clone(), monster_rec()]);
    let rec = DamageRecord {
        result: result::HIT | result::WILL_DIE | result::BLOCK,
        total: 100,
        ..DamageRecord::default()
    };
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(6, 1000));
    f.units[d].room = RoomKind::None;
    assert_eq!(apply_case(&mut f, &c, a, d, rec).result, result::BLOCK);
    f.units[d].room = RoomKind::Town;
    let mon_in_town = f.add(FUnit::new(UnitType::Monster, 0));
    let mon_out = f.add(FUnit::new(UnitType::Monster, 1));
    // Player (class 0 is an inTown monster class): returns.
    let r = apply_case(&mut f, &c, a, d, rec);
    assert_eq!((r.result, f.get(d, 6)), (result::HIT | result::BLOCK, 1000));
    let r = apply_case(&mut f, &c, mon_out, d, rec);
    assert_eq!((r.result, f.get(d, 6)), (result::HIT | result::BLOCK, 1000));
    apply_case(&mut f, &c, mon_in_town, d, rec);
    assert_eq!(f.get(d, 6), 900);
}

// Only totals / leeches > 0 subtract; the result is zeroed
// below 256 (256 itself stays).
#[test]
fn apply_subtract_bounds() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(6, 100)
            .with(8, 100)
            .with(10, 100),
    );
    let r = apply_case(&mut f, &c, a, d, DamageRecord::default());
    assert_eq!((f.get(d, 6), f.get(d, 8), f.get(d, 10)), (100, 100, 100));
    assert_eq!(r.result & result::WILL_DIE, 0);
    // The mana leech is shifted by leech: 16 << 6 = 1024.
    f.set(d, 6, 1256);
    f.set(d, 8, 1280);
    f.set(d, 10, 1256);
    let rec = DamageRecord {
        total: 1000,
        mana_leech: 16,
        stamina_leech: 1000,
        ..DamageRecord::default()
    };
    apply_case(&mut f, &c, a, d, rec);
    assert_eq!((f.get(d, 6), f.get(d, 8), f.get(d, 10)), (256, 256, 256));
}

// The monster-damaged hook only for a monster defender.
#[test]
fn apply_damaged_hook_monsters_only() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0).with(6, 10_000));
    let rec = DamageRecord {
        total: 1000,
        ..DamageRecord::default()
    };
    apply_case(&mut f, &c, a, d, rec);
    assert!(
        !f.log.iter().any(|l| l.starts_with("mondamaged")),
        "{:?}",
        f.log
    );
}

// ------------------------------------------------------- apply melee

/// Stores a hit record from `a` on `d` and runs `apply_melee`.
fn melee(f: &mut Fake, a: usize, d: usize, rec: DamageRecord) {
    let c = ct();
    let entry = CombatEntry {
        attacker: f.ident(a),
        defender: f.ident(d),
        record: rec,
    };
    f.units[a].combat.push(entry);
    apply_melee(f, &c, a, d);
}

// The hit overlay goes on only when the record's overlay is > 0. A non-player
// non-monster attacker in mode 0 still gets thorns.
#[test]
fn apply_melee_overlay_and_thorns() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 10_000));
    let hit = DamageRecord {
        result: result::HIT,
        ..DamageRecord::default()
    };
    melee(&mut f, a, d, hit);
    assert!(
        !f.log.iter().any(|l| l.starts_with("overlay")),
        "{:?}",
        f.log
    );
    let mut ms = FUnit::new(UnitType::Missile, 0);
    ms.mode = 0;
    let m = f.add(ms);
    f.log.clear();
    melee(&mut f, m, d, hit);
    assert!(f.log.contains(&"thorns 2 1".to_string()), "{:?}", f.log);
}

// ----------------------------------------------------------- get-hit

// The critical nibble 0x10 only when no element matched and the
// result has 0x2000.
#[test]
fn element_hit_class_crit_fallback() {
    let mut counter = 0u8;
    let rec = fire(10);
    assert_eq!(element_hit_class(&mut counter, &rec, 2), 0x22);
    assert_eq!(
        element_hit_class(&mut counter, &DamageRecord::default(), 2),
        2
    );
    let crit = DamageRecord {
        result: result::CRITICAL,
        ..fire(10)
    };
    assert_eq!(element_hit_class(&mut counter, &crit, 2), 0x22);
}

// Poison alone (poison = total) skips get-hit; mixed damage
// goes on to the size tests (1000 ≥ 1000 / 4: get-hit).
#[test]
fn no_get_hit_poison_only() {
    let c = ct();
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0).with(7, 1000));
    let rec = DamageRecord {
        poison: 100,
        total: 1000,
        ..DamageRecord::default()
    };
    assert!(!no_get_hit(&mut f, &c.hitclass, u, &rec, 0));
    let rec = DamageRecord {
        poison: 1000,
        total: 1000,
        ..DamageRecord::default()
    };
    assert!(no_get_hit(&mut f, &c.hitclass, u, &rec, 0));
}

/// The first `Seed::new(lo, 0)` whose first two steps satisfy `p`.
fn seed_where(p: impl Fn(u32, u32) -> bool) -> Seed {
    (1u32..)
        .map(|lo| Seed::new(lo, 0))
        .find(|&s| {
            let mut t = s;
            let (a, b) = (t.step(), t.step());
            p(a, b)
        })
        .unwrap()
}

// The size tests are strict and a total < 256 alone skips get-hit.
// Max life 16000, hit class 0 (div 16): M / 16 = 1000, M / 8 = 2000,
// M / 4 = 4000.
#[test]
fn no_get_hit_size_bounds() {
    let c = ct();
    let total = |t| DamageRecord {
        total: t,
        ..DamageRecord::default()
    };
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    // Total 100 with max life 0 (no size test passes).
    assert!(no_get_hit(&mut f, &c.hitclass, u, &total(100), 0));
    f.set(u, 7, 1000);
    // Total 256 is not < 256; 256 ≥ 1000 / 4: get-hit.
    assert!(!no_get_hit(&mut f, &c.hitclass, u, &total(256), 0));
    f.set(u, 7, 16_000);
    // 1000: the M / 8 and M / 4 tests both draw; mask(2) = 1, mask(4) ≠ 0 → false.
    f.units[u].seed = seed_where(|a, b| a & 1 == 1 && b & 3 != 0);
    assert!(!no_get_hit(&mut f, &c.hitclass, u, &total(1000), 0));
    // 2000: only the M / 4 test draws; first step ≡ 2 (mod 4) → false.
    f.units[u].seed = seed_where(|a, _| a & 3 == 2);
    assert!(!no_get_hit(&mut f, &c.hitclass, u, &total(2000), 0));
    // 4000: no draw; a first step ≡ 0 (mod 4) would pass mask(4).
    f.units[u].seed = seed_where(|a, _| a & 3 == 0);
    assert!(!no_get_hit(&mut f, &c.hitclass, u, &total(4000), 0));
}

// ------------------------------------- crushing blow and open wounds

/// Crushing blow (event 5, chance stat 136 = 100 %) on `d`.
fn crush(f: &mut Fake, a: usize, d: usize) -> DamageRecord {
    let mut rec = DamageRecord::default();
    assert_eq!(crushing_blow(f, 5, a, d, &mut rec, 136 << 16), 1);
    rec
}

// Crushing blow: hireling divisor 10; the player-count term
// `div += pct(div, h, 100)`; no will-die while life stays > 0; life 0 →
// will die without the overlay (x = 0).
#[test]
fn crushing_blow_divisors_and_result() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(136, 100));
    let mut h = FUnit::new(UnitType::Monster, 0).with(6, 1000);
    h.hireling = true;
    let hd = f.add(h);
    let r = crush(&mut f, a, hd);
    assert_eq!(f.get(hd, 6), 900);
    assert_eq!(r.result & result::WILL_DIE, 0);
    // Player count 3: bonus (3 − 1) × 50 = 100 % → div 8.
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(6, 1000).with(100, 3));
    crush(&mut f, a, m);
    assert_eq!(f.get(m, 6), 875);
    let z = f.add(FUnit::new(UnitType::Monster, 0).with(6, 0));
    f.log.clear();
    let r = crush(&mut f, a, z);
    assert_ne!(r.result & result::WILL_DIE, 0);
    assert!(
        !f.log.iter().any(|l| l.starts_with("overlay")),
        "{:?}",
        f.log
    );
}

// Open wounds `ow(lvl)`: one level inside each branch and each upper
// bound.
#[test]
fn open_wounds_base_branches() {
    for (lvl, v) in [
        (1, 0),
        (7, 54),
        (15, 126),
        (20, 216),
        (30, 396),
        (37, 585),
        (45, 801),
        (50, 981),
        (60, 1341),
        (70, 1791),
    ] {
        assert_eq!(open_wounds_base(lvl), v, "{lvl}");
    }
}

// Open wounds: no draw when the chance is ≤ 0; a failed draw gives
// `None`; `h = ow(1) + 40 = 40`, halved for a unique or champion monster
// (flag 0xC), quartered for a player.
#[test]
fn open_wounds_chance_and_targets() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(12, 1));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let before = f.units[a].seed;
    assert_eq!(open_wounds(&mut f, 5, a, d, 135 << 16), None);
    assert_eq!(f.units[a].seed, before);
    // Chance 50, r = 70: no curse.
    f.set(a, 135, 50);
    f.units[a].seed = seed_giving(70);
    assert_eq!(open_wounds(&mut f, 5, a, d, 135 << 16), None);
    f.set(a, 135, 100);
    assert_eq!(open_wounds(&mut f, 5, a, d, 135 << 16), Some(40));
    f.units[d].flags = 4;
    assert_eq!(open_wounds(&mut f, 5, a, d, 135 << 16), Some(20));
    let p = f.add(FUnit::new(UnitType::Player, 0));
    assert_eq!(open_wounds(&mut f, 5, a, p, 135 << 16), Some(10));
}

// -------------------------------------------------------- durability

/// A player defender wearing armor (type 50, with durability) in every
/// slot of the weight table; returns (defender, items by slot index).
fn armored(f: &mut Fake) -> (usize, Vec<usize>) {
    let d = f.add(FUnit::new(UnitType::Player, 0));
    let mut items = Vec::new();
    for (loc, _) in DURABILITY_WEIGHTS {
        let i = f.add_item(FItem {
            types: vec![50],
            durability: true,
            ..FItem::default()
        });
        f.units[d].items.insert(loc, i);
        items.push(i);
    }
    (d, items)
}

// `i = roll(7)`, `w = roll(22)`; walk the slots from `i`,
// `w < weight` picks (strict), else `w −= weight`, `i = (i + 1) mod 7`.
// The pick then draws `r < 10` (seeds chosen so).
#[test]
fn durability_slot_walk() {
    // (i, w, picked slot index)
    for (i0, w0, slot) in [(0, 0, 0), (0, 3, 1), (0, 10, 2), (6, 3, 0)] {
        let mut f = world();
        let a = f.add(FUnit::new(UnitType::Monster, 0));
        let (d, items) = armored(&mut f);
        let seed = (1u32..)
            .map(|lo| Seed::new(lo, 0))
            .find(|&s| {
                let mut t = s;
                t.roll(7) == i0 && t.roll(22) == w0 && t.step() % 100 < 10
            })
            .unwrap();
        f.units[d].seed = seed;
        durability(&mut f, a, d);
        assert_eq!(
            f.log,
            [format!("durability {d} {}", items[slot])],
            "{i0} {w0}"
        );
    }
}

// Only armor or weapons *with durability* lose durability.
#[test]
fn durability_hit_needs_durability() {
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    f.units[u].seed = seed_giving(0);
    for t in [50, 45] {
        let i = f.add_item(FItem {
            types: vec![t],
            ..FItem::default()
        });
        durability_hit(&mut f, u, i);
    }
    assert!(f.log.is_empty(), "{:?}", f.log);
}

// --------------------------------------------------------- hit rules

// Holy Shield needs skill > 0 and level > 0.
#[test]
fn defense_holy_shield_needs_skill_and_level() {
    let mut r = skill_rec();
    r.calc1 = 0;
    let mut s = skill_tables(vec![r.clone(), r]);
    s.skills_code = vec![0x07, 100, 0x00];
    let mut f = world();
    let u = f.add(FUnit::new(UnitType::Player, 3).with(31, 100));
    f.units[u].shield = true;
    f.units[u].states.push(101);
    f.units[u].state_stats.insert((101, 350), 0);
    f.units[u].state_stats.insert((101, 351), 5);
    assert_eq!(defense(&mut f, &s, u), 100);
    f.units[u].state_stats.insert((101, 350), 1);
    f.units[u].state_stats.insert((101, 351), 0);
    assert_eq!(defense(&mut f, &s, u), 100);
    f.units[u].state_stats.insert((101, 351), 5);
    assert_eq!(defense(&mut f, &s, u), 200);
}

fn terms(ar: i32, def: i32) -> HitTerms {
    HitTerms {
        ar,
        pct_ar: 0,
        def,
        alvl: 1,
        dlvl: 1,
    }
}

// `toHit < 0` moves to def (factor 0 → 5); the final
// `def < 0 → 0` matters when `def − toHit` wraps (AR = i32::MIN).
#[test]
fn hit_chance_negative_terms() {
    assert_eq!(hit_chance(terms(-50, 50)), 5);
    // def = 0 − i32::MIN wraps to i32::MIN → 0; sum 0 → factor 100.
    assert_eq!(hit_chance(terms(i32::MIN, 0)), 95);
}

// Fractional target AC is halved against a
// hireling; demon / undead to-hit only against that kind.
#[test]
fn hit_terms_fractional_and_kind_bonuses() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(116, 50));
    let mut h = FUnit::new(UnitType::Monster, 0).with(31, 100);
    h.hireling = true;
    let hd = f.add(h);
    // f = 25: 100 − 25.
    assert_eq!(hit_terms(&mut f, &s, &c, a, hd, 0, false).def, 75);
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(31, 100));
    assert_eq!(hit_terms(&mut f, &s, &c, a, m, 0, false).def, 50);
    f.set(a, 116, 0);
    let base = hit_terms(&mut f, &s, &c, a, m, 0, false).ar;
    f.set(a, 123, 30);
    assert_eq!(hit_terms(&mut f, &s, &c, a, m, 0, false).ar, base);
    f.units[m].demon = true;
    assert_eq!(hit_terms(&mut f, &s, &c, a, m, 0, false).ar, base + 30);
    f.units[m].demon = false;
    f.set(a, 123, 0);
    f.set(a, 124, 20);
    assert_eq!(hit_terms(&mut f, &s, &c, a, m, 0, false).ar, base);
    f.units[m].undead = true;
    assert_eq!(hit_terms(&mut f, &s, &c, a, m, 0, false).ar, base + 20);
}

// Prevent-heal only from a player attacker on a monster
// with `item_preventheal ≠ 0`.
#[test]
fn hit_test_prevent_heal_targets() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(12, 1).with(117, 1));
    let p = f.add(FUnit::new(UnitType::Player, 0).with(12, 1));
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(12, 1));
    let b = f.add(FUnit::new(UnitType::Monster, 0).with(12, 1).with(117, 1));
    for (x, y) in [(a, p), (b, m)] {
        f.units[x].seed = seed_giving(0);
        assert!(hit_test(&mut f, &s, &c, Some(x), Some(y), 0, false));
    }
    f.set(a, 117, 0);
    f.units[a].seed = seed_giving(0);
    assert!(hit_test(&mut f, &s, &c, Some(a), Some(m), 0, false));
    assert!(f.log.is_empty(), "{:?}", f.log);
    f.set(a, 117, 1);
    f.units[a].seed = seed_giving(0);
    assert!(hit_test(&mut f, &s, &c, Some(a), Some(m), 0, false));
    assert_eq!(f.log.len(), 1);
}

// Both units must be players or monsters and hostile; the range is 1 (player) or 3
// (monster) minus `range_offset + melee_range` when the offset ≠ 0; the hit test sets hit.
#[test]
fn melee_result_gates_and_range() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let p = f.add(FUnit::new(UnitType::Player, 0).with(12, 1));
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(12, 1));
    let o = f.add(FUnit::new(UnitType::Object, 0).with(12, 1));
    for u in [p, m] {
        f.units[u].seed = seed_giving(0);
    }
    assert_eq!(melee_result(&mut f, &s, &c, Some(o), Some(m), 0, 0), 0);
    assert_eq!(melee_result(&mut f, &s, &c, Some(p), Some(o), 0, 0), 0);
    f.hostile = false;
    assert_eq!(melee_result(&mut f, &s, &c, Some(p), Some(m), 0, 0), 0);
    f.hostile = true;
    let hit = result::HIT | result::GET_HIT;
    for (a, d, offset, range) in [(p, m, 0, 1), (m, p, 0, 3), (m, p, 2, 1)] {
        f.units[a].seed = seed_giving(0);
        f.range_needed = Some(range + 1);
        assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, offset), 0);
        f.range_needed = Some(range);
        assert_eq!(
            melee_result(&mut f, &s, &c, Some(a), Some(d), 0, offset),
            hit,
            "{a} {offset}"
        );
    }
}

// A monster in mode 2 or 15 is moving (no dodge);
// e = 0 → no evade draw.
#[test]
fn dodge_monster_moving_modes() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(338, 100));
    for (mode, want) in [
        (2, BlockResult::None),
        (15, BlockResult::None),
        (1, BlockResult::Dodge),
    ] {
        f.units[d].mode = mode;
        f.units[d].seed = seed_giving(0);
        let before = f.units[d].seed;
        assert_eq!(dodge(&mut f, a, d, false), want, "{mode}");
        if want == BlockResult::None {
            assert_eq!(f.units[d].seed, before, "{mode}");
        }
    }
}

// `skill_poison_override_length` > 0 replaces
// `poisonlength`; the burn length grows only when ≠ 0.
#[test]
fn fill_poison_override_and_burn_length() {
    let mut f = world();
    let a = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(57, 256)
            .with(58, 256)
            .with(59, 50)
            .with(101, 30)
            .with(315, 5),
    );
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let r = fill_flags(&mut f, a, d, hitflag::SKIP_PHYSICAL);
    assert_eq!((r.poison_len, r.burn_len), (30, 0));
    let (s, c) = (st(), ct());
    let mut rec = DamageRecord {
        hit_flags: hitflag::SKIP_PHYSICAL,
        burn_len: 10,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 128);
    assert_eq!(rec.burn_len, 15);
}

// No roll without the hit flag.
#[test]
fn start_combat_needs_hit() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 1));
    let r = start(&mut f, a, d, 0, 0, 0x1_0000);
    assert_eq!(r.hit_flags & hitflag::ROLLED, 0);
    assert_eq!(r.result & result::WILL_DIE, 0);
}

// Revived attackers on players (17); prime evil on a
// revived defender: 400, 200 if the defender is a hireling; neither
// alone.
#[test]
fn damage_percent_revived_rows() {
    let c = ct();
    let mut f = world();
    let p = f.add(FUnit::new(UnitType::Player, 0));
    let mut rv = FUnit::new(UnitType::Monster, 0);
    rv.revived = true;
    let rv = f.add(rv);
    let mut pe = FUnit::new(UnitType::Monster, 0);
    pe.prime_evil = true;
    let pe = f.add(pe);
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    let mut rh = FUnit::new(UnitType::Monster, 0);
    rh.revived = true;
    rh.hireling = true;
    let rh = f.add(rh);
    assert_eq!(damage_percent(&f, &c, Some(rv), p), 17);
    assert_eq!(damage_percent(&f, &c, Some(pe), rv), 400);
    assert_eq!(damage_percent(&f, &c, Some(pe), rh), 200);
    assert_eq!(damage_percent(&f, &c, Some(m), rv), 100);
    assert_eq!(damage_percent(&f, &c, Some(pe), m), 100);
}

// The drain percent (≠ 100) scales the player rule's
// mana and the monster rule's total. Drain 50: mana (10 << 6) % of 1000 =
// 6400 → 3200 / 64 = 50; monster rule T = 10 → 5.
#[test]
fn leech_drain_scales_mana_and_monster_rule() {
    let mut m = monster_rec();
    m.drain = 50;
    let c = combat_tables(vec![m]);
    let mut f = world();
    let a = f.add(leecher(UnitType::Player));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    leech(&mut f, &c, Some(a), d, &mut leech_rec(0, 10, 1000));
    assert_eq!(f.get(a, 8), 50);
    let ma = f.add(leecher(UnitType::Monster).with(7, 1000));
    leech(&mut f, &c, Some(ma), d, &mut leech_rec(10, 0, 1000));
    assert_eq!(f.get(ma, 6), 5);
}

// Block / dodge only after a hit (a miss with dodge
// 100 stays 0).
#[test]
fn melee_result_miss_no_dodge() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(12, 1));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(12, 1).with(338, 100));
    // r = 99 ≥ 95: miss.
    f.units[a].seed = seed_giving(99);
    f.units[d].seed = seed_giving(0);
    assert_eq!(melee_result(&mut f, &s, &c, Some(a), Some(d), 0, 0), 0);
}

// The weapon-block draw needs `w > 0` (a two-claw
// defender without weapon block draws nothing).
#[test]
fn dodge_weapon_block_needs_value() {
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    f.units[d].weapon_class = 13;
    let before = f.units[d].seed;
    assert_eq!(dodge(&mut f, a, d, false), BlockResult::None);
    assert_eq!(f.units[d].seed, before);
}

// ------------------------------------------------------------ vitals

mod vitals_mutants {
    use super::super::vitals::*;
    use crate::skills::fake::blank;
    use crate::units::UnitType;
    use d2_data::tables::Experience;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct V {
        class: i32,
        base: BTreeMap<u16, i32>,
        log: Vec<String>,
    }

    impl VitalsUnits for V {
        type Unit = ();
        fn unit_type(&self, _: ()) -> UnitType {
            UnitType::Player
        }
        fn class_id(&self, _: ()) -> i32 {
            self.class
        }
        fn base_stat(&self, _: (), s: u16) -> i32 {
            self.base.get(&s).copied().unwrap_or(0)
        }
        fn stat(&self, _: (), s: u16) -> i32 {
            self.base_stat((), s)
        }
        fn set_base_stat(&mut self, _: (), s: u16, v: i32) {
            self.base.insert(s, v);
        }
        fn add_base_stat(&mut self, _: (), s: u16, v: i32) {
            *self.base.entry(s).or_default() += v;
        }
        fn max_life(&self, _: ()) -> i32 {
            self.base_stat((), stat::MAXHP)
        }
        fn max_mana(&self, _: ()) -> i32 {
            self.base_stat((), stat::MAXMANA)
        }
        fn max_stamina(&self, _: ()) -> i32 {
            self.base_stat((), stat::MAXSTAMINA)
        }
        fn refresh(&mut self, _: ()) {
            self.log.push("refresh".into());
        }
        fn level_up_notify(&mut self, _: ()) {
            self.log.push("notify".into());
        }
        fn level_up_event(&mut self, _: ()) {
            self.log.push("event12".into());
        }
    }

    /// `experience.txt` with a distinct column per class: level `L` of
    /// class `c` needs `500 × L² × (c + 1)`; `MaxLvl` 99.
    fn tables() -> VitalsTables {
        let row = |f: &dyn Fn(u32) -> u32| {
            let mut e: Experience = blank();
            e.amazon = f(1);
            e.sorceress = f(2);
            e.necromancer = f(3);
            e.paladin = f(4);
            e.barbarian = f(5);
            e.druid = f(6);
            e.assassin = f(7);
            e
        };
        let mut experience = vec![row(&|_| 99)];
        experience.extend((0..=99u32).map(|l| row(&move |k| 500 * l * l * k)));
        let charstats = (0..7).map(|_| blank()).collect();
        VitalsTables {
            charstats,
            experience,
        }
    }

    // Each class reads its own column; ids outside 0–6 use class 0.
    #[test]
    fn threshold_class_columns() {
        let t = tables();
        for class in 0..7 {
            assert_eq!(t.threshold(class, 1), 500 * (class as u32 + 1), "{class}");
        }
        assert_eq!(t.threshold(7, 1), 500);
    }

    // `level_from_exp` counts the rows with `exp ≥ row(i + 1)`.
    #[test]
    fn level_from_exp_counts() {
        let t = tables();
        // Rows of class 0: 0, 500, 2000, 4500 → 2000 is level 3.
        assert_eq!(t.level_from_exp(0, 2000), 3);
        assert_eq!(t.level_from_exp(0, 1999), 2);
    }

    // `set_experience_for_target_level` adds `threshold − experience` when positive; equal
    // experience adds nothing (no level-up).
    #[test]
    fn target_level_adds_difference() {
        let t = tables();
        let mut v = V::default();
        v.base.insert(stat::LEVEL, 1);
        v.base.insert(stat::EXPERIENCE, 100);
        let need = t.threshold(0, 15);
        set_experience_for_target_level(&mut v, &t, (), 15);
        assert_eq!(v.base_stat((), stat::EXPERIENCE), need as i32);
        let mut v = V::default();
        v.base.insert(stat::LEVEL, 1);
        v.base.insert(stat::EXPERIENCE, need as i32);
        set_experience_for_target_level(&mut v, &t, (), 15);
        assert_eq!(v.base_stat((), stat::LEVEL), 1);
        assert!(v.log.is_empty(), "{:?}", v.log);
    }

    /// Amazon-like per-point columns: ManaPerMagic 6, LifePerVitality
    /// 12, StaminaPerVitality 4.
    fn point_tables() -> VitalsTables {
        let mut t = tables();
        let c = &mut t.charstats[0];
        c.manapermagic = 6;
        c.lifepervitality = 12;
        c.staminapervitality = 4;
        t
    }

    // `count − 1 = 99` is accepted (only > 99 is bad).
    #[test]
    fn add_stat_point_count_100() {
        let t = point_tables();
        let mut v = V::default();
        v.base.insert(stat::STATPTS, 100);
        assert_eq!(handle_add_stat_point(&mut v, &t, (), &[0x3A, 0, 99]), 0);
        assert_eq!(v.base_stat((), stat::STRENGTH), 100);
        assert_eq!(v.base_stat((), stat::STATPTS), 0);
    }

    // A negative `n` lowers max mana; the current mana is clamped.
    #[test]
    fn gain_energy_negative_clamps() {
        let t = point_tables();
        let mut v = V::default();
        v.base.insert(stat::MANA, 1000);
        v.base.insert(stat::MAXMANA, 1000);
        gain_energy(&mut v, &t, (), -2);
        // (6 × −2) << 6 = −768.
        assert_eq!(v.base_stat((), stat::MAXMANA), 232);
        assert_eq!(v.base_stat((), stat::MANA), 232);
    }

    // A positive `n` adds to the current stamina too.
    #[test]
    fn gain_vitality_adds_stamina() {
        let t = point_tables();
        let mut v = V::default();
        v.base.insert(stat::STAMINA, 1000);
        v.base.insert(stat::MAXSTAMINA, 1000);
        gain_vitality(&mut v, &t, (), 1);
        assert_eq!(v.base_stat((), stat::MAXSTAMINA), 1256);
        assert_eq!(v.base_stat((), stat::STAMINA), 1256);
    }
}
