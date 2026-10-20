//! Tests for vitals: creation values, spending stat points, the stat reset, level-up,
//! the experience table lookups and the level factor. The `charstats` and `experience`
//! rows are rebuilt as synthetic records.
use super::*;
use d2_data::tables::Record;
use std::collections::BTreeMap;

const AMAZON: i32 = 0;
const SORCERESS: i32 = 1;
const BARBARIAN: i32 = 4;

/// One fake player: base stats, the unit getter equals the base getter.
#[derive(Default)]
struct Fake {
    kind: Option<UnitType>,
    class: i32,
    base: BTreeMap<u16, i32>,
    log: Vec<String>,
}

impl VitalsUnits for Fake {
    type Unit = ();
    fn unit_type(&self, _: ()) -> UnitType {
        self.kind.unwrap_or(UnitType::Player)
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

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Synthetic `charstats` rows for the seven classes.
fn charstats() -> Vec<Charstats> {
    // str dex int vit stamina hpadd L/Lvl S/Lvl M/Lvl L/Vit S/Vit M/Mag SPL
    let rows: [[u8; 13]; 7] = [
        [20, 25, 15, 20, 84, 30, 8, 4, 6, 12, 4, 6, 5],
        [10, 25, 35, 10, 74, 30, 4, 4, 8, 8, 4, 8, 5],
        [15, 25, 25, 15, 79, 30, 6, 4, 8, 8, 4, 8, 5],
        [25, 20, 15, 25, 89, 30, 8, 4, 6, 12, 4, 6, 5],
        [30, 20, 10, 25, 92, 30, 8, 4, 4, 16, 4, 4, 5],
        [15, 20, 20, 25, 84, 30, 6, 4, 8, 8, 4, 8, 5],
        [20, 20, 25, 20, 95, 30, 8, 5, 6, 12, 5, 7, 5],
    ];
    rows.iter()
        .map(|r| {
            let mut c: Charstats = blank();
            c.str = r[0];
            c.dex = r[1];
            c.int = r[2];
            c.vit = r[3];
            c.stamina = r[4];
            c.hpadd = r[5];
            c.lifeperlevel = r[6];
            c.staminaperlevel = r[7];
            c.manaperlevel = r[8];
            c.lifepervitality = r[9];
            c.staminapervitality = r[10];
            c.manapermagic = r[11];
            c.statperlevel = r[12];
            c
        })
        .collect()
}

/// Synthetic `experience.txt`: `MaxLvl` 99, level 0 → 0, level `L` →
/// 500 × L² (level 1 → 500), level 99 → 3,837,739,017.
fn level_exp(l: u32) -> u32 {
    match l {
        99 => 3_837_739_017,
        _ => 500 * l * l,
    }
}

fn tables() -> VitalsTables {
    let row = |v: u32| {
        let mut e: Experience = blank();
        e.amazon = v;
        e.sorceress = v;
        e.necromancer = v;
        e.paladin = v;
        e.barbarian = v;
        e.druid = v;
        e.assassin = v;
        e
    };
    let mut experience = vec![row(99)];
    experience.extend((0..=99).map(|l| row(level_exp(l))));
    VitalsTables {
        charstats: charstats(),
        experience,
    }
}

fn created(class: i32) -> (Fake, VitalsTables) {
    let t = tables();
    let mut f = Fake {
        class,
        ..Fake::default()
    };
    init_player_stats(&mut f, &t, (), 0);
    (f, t)
}

fn get(f: &Fake, s: u16) -> i32 {
    f.base_stat((), s)
}

// ------------------------------------------------------------ creation

#[test]
fn creation_vectors() {
    let (f, _) = created(SORCERESS);
    assert_eq!(get(&f, stat::HITPOINTS), 10240);
    assert_eq!(get(&f, stat::MAXHP), 10240);
    assert_eq!(get(&f, stat::MANA), 8960);
    assert_eq!(get(&f, stat::MAXMANA), 8960);
    assert_eq!(get(&f, stat::STAMINA), 18944);
    assert_eq!(get(&f, stat::MAXSTAMINA), 18944);
    assert_eq!(
        [0, 1, 2, 3].map(|s| get(&f, s)),
        [10, 35, 25, 10],
        "str, energy, dex, vit"
    );
    assert_eq!(get(&f, stat::LEVEL), 1);
    assert_eq!(get(&f, stat::NEXTEXP), 500);
    for s in [
        stat::ATTACKRATE,
        stat::VELOCITYPERCENT,
        stat::OTHER_ANIMRATE,
    ] {
        assert_eq!(get(&f, s), 100);
    }
    assert_eq!(get(&f, stat::TOHIT), 0);
    assert!(f.base.contains_key(&stat::TOBLOCK));
    assert!(f.log.is_empty());

    let (f, _) = created(BARBARIAN);
    assert_eq!(get(&f, stat::HITPOINTS), 14080);
    assert_eq!(get(&f, stat::MANA), 2560);
    assert_eq!(get(&f, stat::STAMINA), 23552);

    // No charstats row: nothing.
    let t = tables();
    let mut f = Fake {
        class: 9,
        ..Fake::default()
    };
    init_player_stats(&mut f, &t, (), 0);
    assert!(f.base.is_empty());
}

#[test]
fn creation_in_a_later_act_raises_experience() {
    let t = tables();
    for (act, target) in [(1, 15), (2, 20), (3, 26), (4, 32), (5, 32), (9, 32)] {
        let mut f = Fake {
            class: AMAZON,
            ..Fake::default()
        };
        init_player_stats(&mut f, &t, (), act);
        let exp = t.threshold(AMAZON, target);
        assert_eq!(get(&f, stat::EXPERIENCE) as u32, exp, "act {act}");
        let level = t.level_from_exp(AMAZON, exp) as i32;
        assert_eq!(get(&f, stat::LEVEL), level);
        assert_eq!(f.log, ["notify", "event12"]);
        assert_eq!(get(&f, stat::STATPTS), 5 * (level - 1));
    }
}

#[test]
fn target_level_needs_a_lower_base_level() {
    // `set_experience_for_target_level` adds only when the target is above the base level
    // (stat 12): at level 15 with too little experience, target 15 adds
    // nothing; target 16 raises the experience to row 17.
    let t = tables();
    let mut f = Fake {
        class: AMAZON,
        ..Fake::default()
    };
    f.base.insert(stat::LEVEL, 15);
    f.base.insert(stat::EXPERIENCE, 100);
    set_experience_for_target_level(&mut f, &t, (), 15);
    assert_eq!(get(&f, stat::EXPERIENCE), 100);
    assert!(f.log.is_empty());
    set_experience_for_target_level(&mut f, &t, (), 16);
    assert_eq!(get(&f, stat::EXPERIENCE) as u32, t.threshold(AMAZON, 16));
}

// ------------------------------------------------------------ stat points

#[test]
fn spend_vitality_and_energy_vectors() {
    let (mut f, t) = created(BARBARIAN);
    f.base.insert(stat::STATPTS, 10);
    let (hp, st) = (get(&f, stat::MAXHP), get(&f, stat::MAXSTAMINA));
    for _ in 0..10 {
        assert!(spend(&mut f, &t, (), 3));
    }
    assert_eq!(get(&f, stat::MAXHP) - hp, 10240);
    assert_eq!(get(&f, stat::HITPOINTS) - hp, 10240);
    assert_eq!(get(&f, stat::MAXSTAMINA) - st, 2560);
    assert_eq!(get(&f, stat::STATPTS), 0);
    assert_eq!(get(&f, stat::VITALITY), 35);

    let (mut f, t) = created(SORCERESS);
    f.base.insert(stat::STATPTS, 10);
    let m = get(&f, stat::MAXMANA);
    for _ in 0..10 {
        assert!(spend(&mut f, &t, (), 1));
    }
    assert_eq!(get(&f, stat::MAXMANA) - m, 5120);
    assert_eq!(get(&f, stat::MANA) - m, 5120);
    assert_eq!(get(&f, stat::ENERGY), 45);
}

#[test]
fn spend_strength_dexterity_refresh() {
    let (mut f, t) = created(AMAZON);
    f.base.insert(stat::STATPTS, 2);
    assert!(spend(&mut f, &t, (), 0));
    assert!(spend(&mut f, &t, (), 2));
    assert_eq!(get(&f, stat::STRENGTH), 21);
    assert_eq!(get(&f, stat::DEXTERITY), 26);
    assert_eq!(get(&f, stat::STATPTS), 0);
    assert_eq!(f.log, ["refresh", "refresh"]);
    assert!(!spend(&mut f, &t, (), 0), "no points left");
}

#[test]
fn message_0x3a() {
    let (mut f, t) = created(BARBARIAN);
    f.base.insert(stat::STATPTS, 3);
    // Vitality, count 5, three points: three spends, the fourth fails.
    assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, 3, 4]), 2);
    assert_eq!(get(&f, stat::VITALITY), 28);
    assert_eq!(get(&f, stat::STATPTS), 0);

    f.base.insert(stat::STATPTS, 5);
    assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, 3, 4]), 0);
    assert_eq!(get(&f, stat::VITALITY), 33);
    // Size, stat and count checks.
    assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, 3]), 3);
    assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, 3, 0, 0]), 3);
    assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, 16, 0]), 3);
    assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, 0, 100]), 3);
    // Ids 4–15 pass the range check and fail in spend.
    f.base.insert(stat::STATPTS, 5);
    for s in 4..=15 {
        assert_eq!(handle_add_stat_point(&mut f, &t, (), &[0x3A, s, 0]), 2);
    }
    assert_eq!(get(&f, stat::STATPTS), 5);
}

#[test]
fn gains_with_non_positive_n_only_clamp() {
    let (mut f, t) = created(BARBARIAN);
    let (hp, st) = (get(&f, stat::HITPOINTS), get(&f, stat::STAMINA));
    gain_vitality(&mut f, &t, (), -2);
    assert_eq!(get(&f, stat::MAXHP), hp - 2 * (16 << 6));
    assert_eq!(get(&f, stat::HITPOINTS), get(&f, stat::MAXHP), "clamped");
    assert_eq!(get(&f, stat::MAXSTAMINA), st - 2 * (4 << 6));
    assert_eq!(get(&f, stat::STAMINA), get(&f, stat::MAXSTAMINA));
    assert_eq!(get(&f, stat::STATPTS), 2);

    let (mut f, t) = created(SORCERESS);
    f.base.insert(stat::MANA, 100);
    gain_energy(&mut f, &t, (), -1);
    assert_eq!(get(&f, stat::MAXMANA), 8960 - (8 << 6));
    assert_eq!(get(&f, stat::MANA), 100, "below the max: unchanged");
    gain_vitality(&mut f, &t, (), 0);
    gain_energy(&mut f, &t, (), 0);
    assert_eq!(get(&f, stat::MANA), 100);
}

#[test]
fn stat_reset() {
    let (mut f, t) = created(BARBARIAN);
    f.base.insert(stat::STATPTS, 20);
    for s in [0, 0, 2, 1, 3, 3, 3] {
        assert!(spend(&mut f, &t, (), s));
    }
    let hp = 14080 + 3 * (16 << 6);
    assert_eq!(get(&f, stat::MAXHP), hp);
    f.log.clear();
    reset_stats(&mut f, &t, ());
    assert_eq!([0, 1, 2, 3].map(|s| get(&f, s)), [30, 10, 20, 25]);
    assert_eq!(get(&f, stat::STATPTS), 20);
    assert_eq!(get(&f, stat::MAXHP), 14080);
    assert_eq!(get(&f, stat::HITPOINTS), 14080, "clamped to the max");
    assert_eq!(f.log, ["refresh", "refresh"], "strength, dexterity");

    // Not a player: nothing.
    let mut m = Fake {
        kind: Some(UnitType::Monster),
        class: BARBARIAN,
        ..Fake::default()
    };
    m.base.insert(stat::STRENGTH, 99);
    reset_stats(&mut m, &t, ());
    assert_eq!(get(&m, stat::STRENGTH), 99);
}

// ------------------------------------------------------------ level-up

fn level_to(f: &mut Fake, t: &VitalsTables, level: u32) -> i32 {
    f.base
        .insert(stat::EXPERIENCE, t.threshold(f.class, level - 1) as i32);
    level_up(f, t, ())
}

#[test]
fn level_up_vectors() {
    let (mut f, t) = created(SORCERESS);
    assert_eq!(level_to(&mut f, &t, 10), 9);
    assert_eq!(get(&f, stat::LEVEL), 10);
    assert_eq!(get(&f, stat::NEXTEXP) as u32, t.threshold(SORCERESS, 10));
    assert_eq!(get(&f, stat::MAXHP), 12544);
    assert_eq!(get(&f, stat::HITPOINTS), 12544);
    assert_eq!(get(&f, stat::MAXMANA), 13568);
    assert_eq!(get(&f, stat::MANA), 13568);
    assert_eq!(get(&f, stat::MAXSTAMINA), 21248);
    assert_eq!(get(&f, stat::STAMINA), 21248);
    assert_eq!(get(&f, stat::STATPTS), 45);
    assert_eq!(get(&f, stat::NEWSKILLS), 9);
    assert_eq!(f.log, ["notify"]);

    let (mut f, t) = created(BARBARIAN);
    level_to(&mut f, &t, 10);
    assert_eq!(get(&f, stat::MAXHP), 18688);
    assert_eq!(get(&f, stat::MAXMANA), 4864);
    assert_eq!(get(&f, stat::MAXSTAMINA), 25856);

    // d ≤ 0: level and nextexp set, nothing else.
    let before = f.base.clone();
    f.log.clear();
    assert_eq!(level_up(&mut f, &t, ()), 0);
    assert_eq!(f.base, before);
    assert!(f.log.is_empty());
}

#[test]
fn level_up_refills_life_only_when_alive() {
    let (mut f, t) = created(BARBARIAN);
    f.base.insert(stat::HITPOINTS, 0);
    f.base.insert(stat::MANA, 0);
    f.base.insert(stat::STAMINA, 0);
    level_to(&mut f, &t, 2);
    assert_eq!(get(&f, stat::HITPOINTS), 0);
    assert_eq!(get(&f, stat::MANA), get(&f, stat::MAXMANA));
    assert_eq!(get(&f, stat::STAMINA), get(&f, stat::MAXSTAMINA));
}

// ------------------------------------------------------------ experience

#[test]
fn experience_lookups() {
    let t = tables();
    assert_eq!(t.level_from_exp(0, 499), 1);
    assert_eq!(t.level_from_exp(0, 500), 2);
    assert_eq!(t.level_from_exp(0, 0), 1);
    assert_eq!(t.threshold(0, 1), 500);
    assert_eq!(t.max_level(3), 99);
    assert_eq!(t.threshold(6, 99), 3_837_739_017);
    // Unsigned: past the level-99 threshold stops at max_level.
    assert_eq!(t.level_from_exp(0, 3_837_739_017), 99);
    assert_eq!(t.level_from_exp(0, u32::MAX), 99);
    // Classes outside 0–6 use class 0.
    let mut t2 = tables();
    t2.experience[2].amazon = 7;
    assert_eq!(t2.threshold(7, 1), 7);
    assert_eq!(t2.threshold(-1, 1), 7);
    assert_eq!(t2.threshold(1, 1), 500);
}

#[test]
fn level_factor_vectors() {
    assert_eq!(level_factor(1000, 30, 22), 429);
    assert_eq!(level_factor(1000, 10, 18), 359);
    assert_eq!(level_factor(1000, 40, 50), 800);
    // Factor 256: experience unchanged.
    assert_eq!(level_factor(1000, 10, 5), 1000);
    assert_eq!(level_factor(1000, 10, 15), 1000);
    // Differences past 10 use the last entry.
    assert_eq!(level_factor(1000, 50, 1), pct(1000, 13, 256));
    assert_eq!(level_factor(1000, 1, 24), pct(1000, 5, 256));
    // alvl ≥ 25 but dlvl ≤ 0 cannot be above alvl; dlvl > alvl ≥ 25 uses pct.
    assert_eq!(level_factor(1000, 25, 26), pct(1000, 25, 26));
}

#[test]
fn add_experience_caps_and_levels() {
    let (mut f, t) = created(AMAZON);
    add_experience(&mut f, &t, (), 499);
    assert_eq!(get(&f, stat::LEVEL), 1);
    assert!(f.log.is_empty());
    add_experience(&mut f, &t, (), 1);
    assert_eq!(get(&f, stat::LEVEL), 2);
    assert_eq!(f.log, ["notify", "event12"]);
    add_experience(&mut f, &t, (), i32::MAX as u32);
    assert_eq!(get(&f, stat::EXPERIENCE) as u32, t.threshold(AMAZON, 98));
    assert_eq!(get(&f, stat::LEVEL), 99);
}
