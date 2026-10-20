//! Coverage tests for combat: flag values, missile and melee application, leech edge
//! cases, event ids with the shared hit-class counter, and the leech resist penalty.
//! Expected values are hand-computed from the rules.

use super::*;
use crate::rng::Seed;
use crate::skills::fake::*;
use crate::units::UnitType;

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

// Record flag and result flag bit values.
#[test]
fn record_flag_values() {
    use hitflag::*;
    assert_eq!(
        [
            SKIP_PHYSICAL,
            SKIP_ROLL,
            LIFE_DRAIN_PRESET,
            MANA_DRAIN_PRESET,
            STAMINA_DRAIN_PRESET,
            ROLLED,
            NO_MISSILE_EVENT,
            BYPASS_UNDEAD,
            BYPASS_DEMONS,
            BYPASS_BEASTS,
            IGNORE_HOSTILITY
        ],
        [0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0x80, 0x100, 0x200, 0x400, 0x1000]
    );
    use hit::result::*;
    assert_eq!(
        [
            HIT,
            WILL_DIE,
            GET_HIT,
            KNOCKBACK,
            BLOCK,
            NO_EVENTS,
            DODGE,
            AVOID,
            EVADE,
            CRITICAL,
            SOFT_HIT,
            WEAPON_BLOCK
        ],
        [1, 2, 4, 8, 0x10, 0x20, 0x80, 0x100, 0x200, 0x2000, 0x4000, 0x8000]
    );
    // A default record is all zero.
    let r = DamageRecord::default();
    assert_eq!((r.hit_flags, r.result, r.total, r.overlay), (0, 0, 0, 0));
}

// A missile hit totals an untotalled record, takes the life, then runs the events.
#[test]
fn missile_apply_computes_totals_then_reaction() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(6, 100_000)
            .with(7, 100_000),
    );
    // The record is not totalled yet: `apply(missile = 1)` computes the
    // totals itself, takes the life, then runs the reaction.
    let mut rec = DamageRecord {
        result: hit::result::HIT,
        hit_flags: hitflag::ROLLED,
        fire: 1_000,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, true, &mut rec);
    assert_eq!(rec.total, 1_000);
    assert_eq!(f.get(d, 6), 99_000);
    // Events 6 then 2 (missile); the reaction is the caller's call.
    let log = &f.log;
    let e6 = log.iter().position(|l| *l == format!("event 6 {a} {d}"));
    let e2 = log.iter().position(|l| *l == format!("event 2 {d} {a}"));
    assert!(e6.unwrap() < e2.unwrap());
}

// A melee apply uses the defender's own record from the attacker's combat list.
#[test]
fn apply_melee_uses_the_defenders_own_record() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d1 = f.add(FUnit::new(UnitType::Monster, 0).with(6, 100_000));
    let d2 = f.add(FUnit::new(UnitType::Monster, 0).with(6, 100_000));
    let mk = |t| DamageRecord {
        result: hit::result::HIT,
        total: t,
        ..DamageRecord::default()
    };
    let entry = |f: &Fake, d, t| CombatEntry {
        attacker: f.ident(a),
        defender: f.ident(d),
        record: mk(t),
    };
    let list = vec![entry(&f, d1, 300), entry(&f, d2, 700)];
    f.units[a].combat = list;
    apply_melee(&mut f, &c, a, d2);
    // The copy of d2's record was applied; d1's record stays listed.
    assert_eq!(f.get(d2, 6), 100_000 - 700);
    assert_eq!(f.get(d1, 6), 100_000);
    assert_eq!(f.units[a].combat.len(), 1);
    assert_eq!(f.units[a].combat[0].record.total, 300);
}

// Leech with no attacker changes nothing and draws nothing.
#[test]
fn leech_without_attacker_changes_nothing() {
    let c = ct();
    let mut f = world();
    let d = f.add(FUnit::new(UnitType::Player, 0).with(8, 50));
    let seed = f.units[d].seed;
    let mut rec = DamageRecord {
        physical: 2_560,
        life_leech: 5,
        mana_leech: 7,
        ..DamageRecord::default()
    };
    leech(&mut f, &c, None, d, &mut rec);
    // `(x << 6) / 64` = x: stored back unchanged, nothing healed.
    assert_eq!((rec.life_leech, rec.mana_leech), (5, 7));
    assert_eq!(f.get(d, 8), 50);
    assert!(f.log.is_empty());
    assert_eq!(f.units[d].seed, seed);
}

// Event ids, and the single hit-class counter shared by all element hits.
#[test]
fn event_ids_and_shared_hit_class_counter() {
    assert_eq!(
        [
            EV_DAMAGEDINMELEE,
            EV_DAMAGEDBYMISSILE,
            EV_ATTACKEDINMELEE,
            EV_DOMELEEDAMAGE,
            EV_DOMISSILEDAMAGE,
            EV_DOMELEEATTACK,
            EV_KILL,
            EV_KILLED,
            EV_ABSORBDAMAGE
        ],
        [1, 2, 3, 5, 6, 7, 9, 10, 11]
    );
    // One counter steps on every element hit class, whoever asks: two
    // hits with cold and fire take successive table starts.
    let rec = DamageRecord {
        cold: 1,
        fire: 1,
        ..DamageRecord::default()
    };
    let mut counter = 0u8;
    assert_eq!(element_hit_class(&mut counter, &rec, 2), 0x32); // cold first
    assert_eq!(counter, 1);
    assert_eq!(element_hit_class(&mut counter, &rec, 2), 0x22); // fire next
    assert_eq!(counter, 2);
    // Start at lightning: cold and fire absent from the rest, wraps to cold.
    assert_eq!(element_hit_class(&mut counter, &rec, 2), 0x32);
    assert_eq!(counter, 3);
}

// The difficulty resist penalty scales a player's leech taken from a monster.
#[test]
fn leech_rows_resist_penalty() {
    // A monster draining a Hell player takes x2: the leech is scaled by
    // (100 - r) / 100 with the resist penalty r = -100.
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Monster, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    for (diff, want) in [(0usize, 1_000), (1, 1_400), (2, 2_000)] {
        f.difficulty = diff;
        let mut rec = DamageRecord {
            life_leech: 1_000,
            ..DamageRecord::default()
        };
        totals(&mut f, &c, Some(a), d, &mut rec);
        assert_eq!(rec.life_leech, want, "difficulty {diff}");
    }
    // Classic game: -20 / -50 -> x1.2 / x1.5.
    f.expansion = false;
    for (diff, want) in [(1usize, 1_200), (2, 1_500)] {
        f.difficulty = diff;
        let mut rec = DamageRecord {
            life_leech: 1_000,
            ..DamageRecord::default()
        };
        totals(&mut f, &c, Some(a), d, &mut rec);
        assert_eq!(rec.life_leech, want, "classic difficulty {diff}");
    }
    let _ = Seed::new(0, 0);
}
