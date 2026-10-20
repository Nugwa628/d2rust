// Test vectors: "Test vectors", "Edge cases".
use super::fake::*;
use super::*;
use crate::combat::DamageRecord;
use crate::rng::Seed;
use crate::units::UnitType;

fn tables_with(rec: Skills) -> SkillTables {
    skill_tables(vec![rec])
}

fn player(f: &mut Fake) -> usize {
    f.add(FUnit::new(UnitType::Player, 1))
}

// ------------------------------------------------------------ blocks

#[test]
fn dm_vectors() {
    // Reference vectors (base-game operand order).
    assert_eq!(dm(1, 25, 70), 31);
    assert_eq!(dm(20, 25, 70), 62);
    let crit: Vec<i32> = [1, 2, 5, 10, 20, 30, 99]
        .iter()
        .map(|&l| dm(l, 5, 80))
        .collect();
    assert_eq!(crit, [16, 25, 42, 56, 68, 73, 80]);
    assert_eq!((dm(1, 10, 65), dm(20, 10, 65)), (18, 56));
    // Only the upper clamp exists; level 0 gives `a`.
    assert_eq!(dm(0, 7, 9), 7);
}

#[test]
#[should_panic(expected = "divides by zero")]
fn dm_minus_six_is_fatal() {
    dm(-6, 1, 2);
}

#[test]
fn bracket_vectors() {
    assert_eq!(bracket(29, [1, 2, 3, 4, 5]), 70);
    assert_eq!(bracket(1, [9, 9, 9, 9, 9]), 0);
    assert_eq!(bracket(8, [2, 0, 0, 0, 0]), 14);
    assert_eq!(bracket(9, [2, 3, 0, 0, 0]), 17);
    assert_eq!(bracket(17, [1, 1, 1, 0, 0]), 16);
    assert_eq!(bracket(23, [1, 1, 1, 1, 0]), 22);
}

#[test]
fn linear_specials() {
    let mut r = skill_rec();
    (r.param1, r.param2, r.param3, r.param4) = (50, 5, 200, 75);
    (r.param7, r.param8) = (5, 80);
    (r.tohit, r.levtohit) = (20, 7);
    let t = tables_with(r);
    let mut f = Fake::default();
    let sp = |f: &mut Fake, c: u8, l: i32| special(f, &t, None, c, 0, l);
    // Bash ln12 (50, 5) L1 / L20.
    assert_eq!((sp(&mut f, 0, 1), sp(&mut f, 0, 20)), (50, 145));
    // Amplify Damage ln34 (200, 75) L1 / L10.
    assert_eq!((sp(&mut f, 2, 1), sp(&mut f, 2, 10)), (200, 875));
    // Guards: lnXY and dm12/34/56 give 0 at level 0; dm78 and parN do not.
    assert_eq!(sp(&mut f, 0, 0), 0);
    assert_eq!(sp(&mut f, 1, 0), 0);
    assert_eq!(sp(&mut f, 7, 0), 5);
    assert_eq!(sp(&mut f, 7, 20), 68);
    assert_eq!(sp(&mut f, 15, -3), 80);
    assert_eq!(sp(&mut f, 16, 13), 13);
    // Sacrifice to-hit (20, 7) L1 / L20; `toht` at level 0 is 0.
    assert_eq!(
        (sp(&mut f, 20, 1), sp(&mut f, 20, 20), sp(&mut f, 20, 0)),
        (20, 153, 0)
    );
    // Out of range index or skill.
    assert_eq!(sp(&mut f, 73, 5), 0);
    assert_eq!(special(&mut f, &t, None, 8, 1, 5), 0);
}

// ------------------------------------------------------------ damage

fn fire_bolt() -> Skills {
    let mut r = skill_rec();
    (r.emin, r.emax, r.hitshift, r.etype) = (6, 12, 7, 1);
    (r.eminlev1, r.eminlev2, r.eminlev3, r.eminlev4, r.eminlev5) = (3, 5, 6, 7, 8);
    (r.emaxlev1, r.emaxlev2, r.emaxlev3, r.emaxlev4, r.emaxlev5) = (3, 7, 8, 9, 10);
    (r.mana, r.lvlmana, r.manashift) = (5, 0, 7);
    r
}

#[test]
fn fire_bolt_vectors() {
    let t = tables_with(fire_bolt());
    let mut f = Fake::default();
    let mm = |f: &mut Fake, t: &SkillTables, l| {
        (
            elem_min(f, t, None, 0, l, false),
            elem_max(f, t, None, 0, l, false),
        )
    };
    assert_eq!(mm(&mut f, &t, 1), (768, 1536));
    assert_eq!(mm(&mut f, &t, 20), (11_648, 15_488));
    assert_eq!(elem_min(&mut f, &t, None, 0, 0, false), 0);
    // Synergy p = 80 through a formula: `EDmgSymPerCalc` = `80`.
    let mut r = fire_bolt();
    r.edmgsympercalc = 0;
    let mut t = tables_with(r);
    t.skills_code = vec![0x07, 80, 0x00];
    assert_eq!(mm(&mut f, &t, 20), (20_966, 27_878));
    assert_eq!(mm(&mut f, &t, 1), (1_382, 2_764));
    // edmn / edmx are >> 8 of the same; edns / edxs unshifted.
    assert_eq!(special(&mut f, &t, None, 17, 0, 20), 20_966 >> 8);
    assert_eq!(special(&mut f, &t, None, 39, 0, 20), 27_878);
    // Mana: Fire Bolt (5, 0, shift 7) L1: 640 stored, `mana` 2.
    assert_eq!(special(&mut f, &t, None, 42, 0, 1), 640);
    assert_eq!(special(&mut f, &t, None, 21, 0, 1), 2);
}

#[test]
fn elemental_min_synergy_gate() {
    // Lightning: EMin 1, EMinLev1 0, HitShift 8: L20 min with p = 64 stays
    // 256 (Edge case 1); the maximum takes the synergy.
    let mut r = skill_rec();
    (r.emin, r.emax, r.hitshift, r.edmgsympercalc) = (1, 40, 8, 0);
    let mut t = tables_with(r);
    t.skills_code = vec![0x07, 64, 0x00];
    let mut f = Fake::default();
    assert_eq!(elem_min(&mut f, &t, None, 0, 20, false), 256);
    assert_eq!(elem_max(&mut f, &t, None, 0, 1, false), 10_240 + 6_553);
}

#[test]
fn elemental_min_gate_skips_formula() {
    // The min's gate is tested before `EDmgSymPerCalc`: a gated-out min
    // (v = 256, EMinLev1 0) does not evaluate `rand(1, 6)`, so the
    // caster's seed is not stepped; the max evaluates it and draws.
    let mut r = skill_rec();
    (r.emin, r.emax, r.hitshift, r.edmgsympercalc) = (1, 40, 8, 0);
    let mut t = tables_with(r);
    t.skills_code = vec![0x07, 1, 0x07, 6, 0x01, 2, 0x00];
    let mut f = Fake::default();
    let u = player(&mut f);
    let before = f.units[u].seed;
    assert_eq!(elem_min(&mut f, &t, Some(u), 0, 20, false), 256);
    assert_eq!(f.units[u].seed, before);
    elem_max(&mut f, &t, Some(u), 0, 1, false);
    assert_eq!(f.units[u].seed, Seed::new(0x6AC6_90C5, 0));
}

#[test]
fn elemental_mastery() {
    let t = tables_with(fire_bolt());
    let mut f = Fake::default();
    let u = player(&mut f);
    f.set(u, 329, 50);
    assert_eq!(elem_min(&mut f, &t, Some(u), 0, 1, true), 768 + 384);
    // Mastery 0 ignores the stat; no unit ignores it too.
    assert_eq!(elem_min(&mut f, &t, Some(u), 0, 1, false), 768);
    assert_eq!(elem_min(&mut f, &t, None, 0, 1, true), 768);
    assert_eq!(special(&mut f, &t, Some(u), 52, 0, 1), 1_152);
    assert_eq!(special(&mut f, &t, Some(u), 49, 0, 1), 1_152 >> 8);
    assert_eq!(elem_mastery_stat(12), Some(331));
    assert_eq!(elem_mastery_stat(3), None);
}

#[test]
fn frozen_orb_length_and_mana() {
    let mut r = skill_rec();
    (r.elen, r.elevlen1, r.elevlen2, r.elevlen3) = (200, 25, 25, 25);
    (r.mana, r.lvlmana, r.manashift) = (50, 1, 7);
    let t = tables_with(r);
    let mut f = Fake::default();
    let len: Vec<i32> = [1, 10, 20]
        .iter()
        .map(|&l| elem_len(&mut f, &t, None, 0, l))
        .collect();
    assert_eq!(len, [200, 425, 675]);
    assert_eq!(elem_len(&mut f, &t, None, 0, 0), 0);
    let rec = &t.skills[0];
    assert_eq!(
        [1, 10, 20].map(|l| mana_cost(rec, l)),
        [6_400, 7_552, 8_832]
    );
}

#[test]
fn teleport_mana() {
    let mut r = skill_rec();
    (r.mana, r.lvlmana, r.manashift, r.minmana) = (24, 0xFFFF, 8, 1);
    let t = tables_with(r);
    let rec = &t.skills[0];
    assert_eq!(
        [1, 10, 20].map(|l| mana_cost(rec, l)),
        [6_144, 3_840, 1_280]
    );
    let mut f = Fake::default();
    let u = player(&mut f);
    // L25: usmc 0, shifted 0; consume charges minmana: 256.
    assert_eq!(special(&mut f, &t, None, 42, 0, 25), 0);
    assert_eq!(mana_cost_shifted(&t, 0, 25), 0);
    f.set(u, 8, 1_000);
    assert!(consume_mana(&mut f, &t, Some(u), 0, 25));
    assert_eq!(f.get(u, 8), 1_000 - 256);
    // L30: usmc −1280, `mana` −5, consume 256.
    assert_eq!(special(&mut f, &t, None, 42, 0, 30), -1_280);
    assert_eq!(special(&mut f, &t, None, 21, 0, 30), -5);
    assert_eq!(mana_cost_shifted(&t, 0, 30), 0);
    assert!(consume_mana(&mut f, &t, Some(u), 0, 30));
    assert_eq!(f.get(u, 8), 1_000 - 512);
    // Not enough mana: nothing deducted.
    f.set(u, 8, 255);
    assert!(!consume_mana(&mut f, &t, Some(u), 0, 30));
    assert_eq!(f.get(u, 8), 255);
}

#[test]
fn mana_rules() {
    let mut r = skill_rec();
    (r.mana, r.lvlmana, r.manashift, r.minmana) = (10, 2, 8, 50);
    let mut zero = skill_rec();
    zero.mana = 0;
    let t = skill_tables(vec![r, zero]);
    let mut f = Fake::default();
    let u = player(&mut f);
    // mps: (10 + 2·2) · 25 / 2 = 175 at level 3.
    assert_eq!(special(&mut f, &t, None, 22, 0, 3), 175);
    assert_eq!(special(&mut f, &t, None, 22, 0, 0), 0);
    // Non-player: 1, nothing deducted.
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    assert!(consume_mana(&mut f, &t, Some(m), 0, 1));
    assert!(consume_mana(&mut f, &t, None, 0, 1));
    // mana = lvlmana = 0 gives 0.
    assert!(!consume_mana(&mut f, &t, Some(u), 1, 1));
    // can_afford ignores minmana (Edge case 5): cost 10 << 8 at level 1.
    let e = SkillEntry {
        skill: 0,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    f.set(u, 8, 2_560);
    assert!(can_afford(&f, &t, u, &e));
    f.set(u, 8, 2_559);
    assert!(!can_afford(&f, &t, u, &e));
    // ... while consume applies it: max(2560, 50 << 8).
    f.set(u, 8, 20_000);
    assert!(consume_mana(&mut f, &t, Some(u), 0, 1));
    assert_eq!(f.get(u, 8), 20_000 - 12_800);
    // Blood mana pays with life; can_afford reads life.
    f.units[u].states.push(114);
    f.set(u, 6, 2_560);
    assert!(can_afford(&f, &t, u, &e));
    assert!(consume_mana(&mut f, &t, Some(u), 0, 1));
    assert_eq!(f.log.last().unwrap(), "paylife 12800");
    // Charge entries: charges > 0; consume takes the charges path.
    let c = SkillEntry {
        skill: 0,
        base: 5,
        owner_guid: 77,
        charges: 0,
        ..SkillEntry::default()
    };
    assert!(!can_afford(&f, &t, u, &c));
    f.units[u].used = Some(c);
    assert!(consume_mana(&mut f, &t, Some(u), 0, 1));
    assert_eq!(f.log.last().unwrap(), "charges");
}

#[test]
fn tornado_physical_synergy_before_shift() {
    let mut r = skill_rec();
    (r.mindam, r.maxdam, r.hitshift) = (25, 35, 8);
    (r.minlevdam1, r.minlevdam2, r.minlevdam3) = (8, 16, 16);
    (r.maxlevdam1, r.maxlevdam2, r.maxlevdam3) = (8, 16, 19);
    let t = tables_with(r.clone());
    let mut f = Fake::default();
    let pm = |f: &mut Fake, t: &SkillTables, l| {
        (
            phys_min(f, t, None, 0, l, false),
            phys_max(f, t, None, 0, l, false),
        )
    };
    assert_eq!(pm(&mut f, &t, 1), (6_400, 8_960));
    assert_eq!(pm(&mut f, &t, 20), (69_888, 75_520));
    r.dmgsympercalc = 0;
    let mut t = tables_with(r);
    t.skills_code = vec![0x07, 90, 0x00];
    assert_eq!(pm(&mut f, &t, 20), (132_608, 143_360));
    // Invalid skill: 1–2 unshifted (Edge case 8).
    assert_eq!(
        (
            phys_min(&mut f, &t, None, 9, 1, false),
            phys_max(&mut f, &t, None, 9, 1, false)
        ),
        (1, 2)
    );
    assert_eq!(elem_min(&mut f, &t, None, 9, 1, false), 0);
}

#[test]
fn kick_and_srcdam() {
    let mut r = skill_rec();
    r.kick = true;
    let mut s = skill_rec();
    (s.srcdam, s.mindam, s.maxdam, s.hitshift) = (64, 1, 2, 8);
    let t = skill_tables(vec![r, s]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 6).with(0, 100).with(2, 60));
    assert_eq!(phys_min(&mut f, &t, Some(u), 0, 1, false), 8_960);
    assert_eq!(phys_max(&mut f, &t, Some(u), 0, 1, false), 11_776);
    // Monster: 3 × level; no unit: x = max(0, 1).
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(12, 10));
    assert_eq!(phys_min(&mut f, &t, Some(m), 0, 1, false), (30 / 4) << 8);
    assert_eq!(phys_max(&mut f, &t, None, 0, 1, false), 0);
    // SrcDam 64 of mindamage 10 / maxdamage 20 with no weapon.
    f.set(u, 21, 10);
    f.set(u, 22, 20);
    assert_eq!(phys_min(&mut f, &t, Some(u), 1, 1, true), (5 + 1) << 8);
    assert_eq!(phys_max(&mut f, &t, Some(u), 1, 1, true), (10 + 2) << 8);
    assert_eq!(phys_min(&mut f, &t, Some(u), 1, 1, false), 1 << 8);
    // A two-handed weapon (wield type 2) gives its own min / max.
    let w = f.add_item(FItem {
        wield: 2,
        damage: (40, 80),
        ..FItem::default()
    });
    f.units[u].weapon = Some(w);
    assert_eq!(phys_max(&mut f, &t, Some(u), 1, 1, true), (40 + 2) << 8);
}

#[test]
fn kick_damage_helper() {
    let mut f = Fake::default();
    let u = f.add(
        FUnit::new(UnitType::Player, 6)
            .with(0, 100)
            .with(2, 50)
            .with(137, 3)
            .with(17, 10),
    );
    let boots = f.add_item(FItem {
        boots: (10, 5),
        str_dex: (10, 20),
        ..FItem::default()
    });
    f.units[u].items.insert(9, boots);
    let (mut min, mut max, mut pc) = (0, 0, 0);
    kick_damage(&mut f, u, &mut min, &mut max, &mut pc);
    // k counted twice (Edge case 6); bmin 13 ≥ bmax 8 → bmax 13.
    assert_eq!((min, max), (3 + 13, 3 + 13));
    assert_eq!(pc, 10 + 10 + 10);
    assert_eq!(f.log, ["weaponlists false", "weaponlists true"]);
}

#[test]
fn rolls_step_unit_seed() {
    let mut r = fire_bolt();
    (r.mindam, r.maxdam, r.hitshift) = (2, 4, 8);
    let t = tables_with(r);
    let mut f = Fake::default();
    let u = player(&mut f);
    let mut rec = DamageRecord {
        physical: 7,
        ..DamageRecord::default()
    };
    roll_physical(&mut f, &t, u, &mut rec, 0, 1);
    let mut s = Seed::new(1, 0);
    let want = 512 + s.roll(512) as i32;
    assert_eq!(rec.physical, 7 + want);
    assert_eq!(f.units[u].seed, s);
    // One HitShift (8 here) for both: 1536 + roll(1536) on the next step.
    // EType 1: the value goes to fire (`add_element`).
    let mut rec = DamageRecord::default();
    let v = roll_elemental(&mut f, &t, u, &mut rec, 0, 1);
    let want = 1_536 + s.roll(1_536) as i32;
    assert_eq!((v, rec.fire, rec.physical), (want, want, 0));
    // A zero range does not step.
    let mut r = skill_rec();
    (r.mindam, r.maxdam) = (3, 3);
    let t = tables_with(r);
    let before = f.units[u].seed;
    let mut rec = DamageRecord::default();
    roll_physical(&mut f, &t, u, &mut rec, 0, 1);
    assert_eq!((rec.physical, f.units[u].seed), (3, before));
}

#[test]
fn add_element_by_etype() {
    let mut f = Fake::default();
    let u = player(&mut f);
    let mut r = DamageRecord::default();
    let add = |f: &mut Fake, r: &mut DamageRecord, e, v, len| add_element(f, u, r, e, v, len);
    let before = f.units[u].seed;
    // Fixed elements: field, length, resist; no draw.
    assert_eq!(add(&mut f, &mut r, 1, 5, 9).resist, 39);
    assert_eq!(r.hit_class, 0x20);
    assert_eq!(add(&mut f, &mut r, 2, 6, 9).resist, 41);
    assert_eq!(r.hit_class, 0x40);
    // Magic has no hit class: +0x60 unchanged.
    assert_eq!(add(&mut f, &mut r, 3, 7, 9).resist, 37);
    assert_eq!(r.hit_class, 0x40);
    let c = add(&mut f, &mut r, 4, 8, 25);
    assert_eq!((c.resist, c.hit_class), (43, 0x30));
    assert_eq!(add(&mut f, &mut r, 5, 9, 30).resist, 45);
    for e in [6, 7, 8] {
        assert_eq!(add(&mut f, &mut r, e, 1, 0).resist, -1);
    }
    add(&mut f, &mut r, 9, 2, 3);
    add(&mut f, &mut r, 11, 4, 12);
    add(&mut f, &mut r, 12, 10, 40);
    add(&mut f, &mut r, 0, 100, 0);
    add(&mut f, &mut r, 13, 1, 0);
    assert_eq!(f.units[u].seed, before);
    let want = DamageRecord {
        fire: 5,
        lightning: 6,
        magic: 7,
        cold: 18,
        cold_len: 25,
        poison: 9,
        poison_len: 30,
        life_leech: 1,
        mana_leech: 1,
        stamina_leech: 1,
        stun_len: 5,
        burn: 4,
        burn_len: 12,
        freeze_len: 40,
        physical: 101,
        // A plain store of the last class given (e = 12: 0x30); 11, 0 and
        // 13 leave it.
        hit_class: 0x30,
        ..DamageRecord::default()
    };
    assert_eq!(r, want);
    // e = 10: one step, {1, 2, 4, 5}[lo' & 3], len ≤ 0 → 50.
    let mut s = f.units[u].seed;
    let e = [1, 2, 4, 5][(s.step() & 3) as usize];
    let mut r = DamageRecord::default();
    let got = add(&mut f, &mut r, 10, 3, 0);
    assert_eq!(got.element, e);
    assert_eq!(f.units[u].seed, s);
    let len = match e {
        4 => r.cold_len,
        5 => r.poison_len,
        _ => 50,
    };
    assert_eq!(len, 50);
}

// ------------------------------------------------------------ levels

fn entry(skill: i32, base: i32, owner: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base,
        owner_guid: owner,
        ..SkillEntry::default()
    }
}

#[test]
fn skill_level_and_bonuses() {
    let mut r = skill_rec();
    r.charclass = 1;
    r.skilldesc = 0;
    r.etype = 1;
    let mut t = tables_with(r);
    t.skilldesc[0].skillpage = 2;
    let mut f = Fake::default();
    let u = player(&mut f);
    let e = SkillEntry {
        level_bonus: 1,
        ..entry(0, 5, -1)
    };
    f.set(u, 127, 2); // allskills
    f.units[u].stats.insert((83, 1), 1); // class skills, layer = class
    f.units[u].stats.insert((188, 2 + 8 - 1), 1); // tab, page + 8·class − 1
    f.units[u].stats.insert((97, 0), 5); // nonclassskill, capped 3
    f.units[u].stats.insert((126, 1), 1); // elemskill
    f.units[u].stats.insert((107, 0), 1); // singleskill
    f.units[u].states.push(134); // shrine +2
    assert_eq!(bonus_level(&f, &t, u, &e), 1 + 2 + 2 + 1 + 1 + 3 + 1 + 1);
    assert_eq!(skill_level(&f, &t, Some(u), Some(&e), true), 5 + 12);
    assert_eq!(skill_level(&f, &t, Some(u), Some(&e), false), 5);
    assert_eq!(skill_level(&f, &t, None, Some(&e), true), 0);
    assert_eq!(skill_level(&f, &t, Some(u), None, true), 0);
    // Item-granted entries never get bonuses.
    assert_eq!(skill_level(&f, &t, Some(u), Some(&entry(0, 5, 9)), true), 5);
    // Cap 99 and floor 0.
    assert_eq!(
        skill_level(&f, &t, Some(u), Some(&entry(0, 150, -1)), false),
        99
    );
    assert_eq!(
        skill_level(&f, &t, Some(u), Some(&entry(0, -4, -1)), false),
        0
    );
    // Other class: nonclassskill uncapped.
    f.units[u].class = 3;
    assert_eq!(bonus_level(&f, &t, u, &e), 1 + 2 + 2 + 5 + 1 + 1);
    // Non-player: base ≤ 0 uncapped, else min(n, 3).
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[m].stats.insert((97, 0), 5);
    assert_eq!(bonus_level(&f, &t, m, &entry(0, 0, -1)), 5);
    assert_eq!(bonus_level(&f, &t, m, &entry(0, 1, -1)), 3);
    f.units[m].stats.insert((97, 0), -7);
    assert_eq!(bonus_level(&f, &t, m, &entry(0, 1, -1)), -7);
}

#[test]
#[should_panic(expected = "skilldesc")]
fn class_skill_without_skilldesc_is_fatal() {
    let mut r = skill_rec();
    r.charclass = 1;
    let t = tables_with(r);
    let mut f = Fake::default();
    let u = player(&mut f);
    bonus_level(&f, &t, u, &entry(0, 1, -1));
}

#[test]
fn highest_entry_rules() {
    let charged = SkillEntry {
        has_charges: true,
        ..entry(4, 50, 3)
    };
    let list = [
        entry(4, 3, 7),
        charged,
        entry(4, 5, 8),
        entry(4, 1, -1),
        entry(4, 9, 9),
        entry(5, 20, -1),
    ];
    // Item entries: strictly higher base wins; a native one replaces and
    // then holds; charged entries are skipped.
    assert_eq!(highest_entry(&list, 4), Some(entry(4, 1, -1)));
    assert_eq!(highest_entry(&list[..3], 4), Some(entry(4, 5, 8)));
    assert_eq!(highest_entry(&list, 6), None);
}

#[test]
fn formula_functions() {
    // skill 0: Param8 = 42, calc1 = skill(1, par1) + sklvl(1, lvl, par2)
    // + stat(12, 0) + min(3, 4) + max(3, 4).
    let mut a = skill_rec();
    a.param8 = 42;
    a.calc1 = 0;
    let mut b = skill_rec();
    (b.param1, b.param2) = (100, 7);
    let mut t = skill_tables(vec![a, b]);
    t.skills_code = vec![
        0x07, 1, 0x07, 8, 0x01, 3, // skill(1, par1)
        0x07, 1, 0x07, 16, 0x07, 9, 0x01, 6, 0x10, // + sklvl(1, lvl, par2)
        0x07, 12, 0x07, 0, 0x01, 5, 0x10, // + stat(12, 0)
        0x07, 3, 0x07, 4, 0x01, 0, 0x10, // + min
        0x07, 3, 0x07, 4, 0x01, 1, 0x10, 0x00, // + max
    ];
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 1).with(12, 30));
    // No entry for skill 1: special at level 0, par1 still 100.
    let v = special(&mut f, &t, Some(u), 55, 0, 9);
    assert_eq!(v, 100 + 7 + 30 + 3 + 4);
    // No unit: stat gives 0.
    assert_eq!(special(&mut f, &t, None, 55, 0, 9), 100 + 7 + 3 + 4);
    // ulvl / blvl.
    f.units[u].skills = vec![entry(0, 6, -1)];
    assert_eq!(special(&mut f, &t, Some(u), 40, 0, 1), 30);
    assert_eq!(special(&mut f, &t, Some(u), 41, 0, 1), 6);
    assert_eq!(special(&mut f, &t, None, 41, 0, 1), 0);
    // Calc-backed specials: field −1 → 0, offset ≥ buffer → 0.
    assert_eq!(special(&mut f, &t, Some(u), 56, 0, 9), 0);
    t.skills[0].calc2 = 999;
    assert_eq!(special(&mut f, &t, Some(u), 56, 0, 9), 0);
}

#[test]
fn formula_rand_draws_on_caster_seed() {
    let mut a = skill_rec();
    a.calc1 = 0;
    let mut t = skill_tables(vec![a]);
    t.skills_code = vec![0x07, 1, 0x07, 6, 0x01, 2, 0x00];
    let mut f = Fake::default();
    let u = player(&mut f);
    assert_eq!(special(&mut f, &t, Some(u), 55, 0, 1), 4);
    assert_eq!(f.units[u].seed, Seed::new(0x6AC6_90C5, 0));
}

#[test]
fn mastery_special() {
    let mut r = skill_rec();
    (r.passivestat1, r.passivecalc1) = (999, 0xFFFF_FFFF);
    (r.passivestat2, r.passivecalc2) = (346, 3);
    let mut t = tables_with(r);
    t.skills_code = vec![0x00, 0x00, 0x00, 0x07, 33, 0x00];
    let mut f = Fake::default();
    assert_eq!(special(&mut f, &t, None, 24, 0, 1), 33);
    assert_eq!(special(&mut f, &t, None, 24, 0, 0), 0);
    assert_eq!(special(&mut f, &t, None, 23, 0, 1), 0);
}

#[test]
fn weapon_mastery_layers_and_throw() {
    let mut s = skill_rec();
    (s.range, s.itypea1) = (2, 48);
    let t = skill_tables(vec![skill_rec(), s]);
    let mut f = Fake::default();
    let u = player(&mut f);
    let sword = f.add_item(FItem {
        types: vec![30, 45],
        ..FItem::default()
    });
    f.units[u]
        .entries
        .insert(342, vec![(30, 20), (31, 50), (45, 25)]);
    f.units[u].entries.insert(345, vec![(30, 70)]);
    assert_eq!(weapon_mastery(&f, &t, Some(u), Some(sword), Some(0), 0), 25);
    assert_eq!(weapon_mastery(&f, &t, None, Some(sword), Some(0), 0), 0);
    assert_eq!(weapon_mastery(&f, &t, Some(u), None, Some(0), 0), 0);
    // Throw path: throwable item, skill range 2 with itypea1 is-a 48.
    f.items[sword].throw = true;
    assert_eq!(weapon_mastery(&f, &t, Some(u), Some(sword), Some(1), 0), 70);
    // No skill given: the used skill.
    f.units[u].used = Some(entry(1, 1, -1));
    assert_eq!(weapon_mastery(&f, &t, Some(u), Some(sword), None, 0), 70);
    // Negative values give 0.
    f.units[u].entries.insert(343, vec![(30, -5)]);
    assert_eq!(weapon_mastery(&f, &t, Some(u), Some(sword), Some(0), 1), 0);
}

#[test]
fn throw_mastery_is_zero_off_the_throw_gate() {
    let mut s = skill_rec();
    (s.range, s.itypea1) = (2, 48);
    let t = skill_tables(vec![skill_rec(), s]);
    let mut f = Fake::default();
    let u = player(&mut f);
    let axe = f.add_item(FItem {
        types: vec![30],
        ..FItem::default()
    });
    f.units[u].entries.insert(342, vec![(30, 20)]);
    f.units[u].entries.insert(343, vec![(30, 40)]);
    f.units[u].entries.insert(345, vec![(30, 70)]);
    f.units[u].entries.insert(346, vec![(30, 90)]);
    f.units[u].used = Some(entry(1, 1, -1));
    // Not `throwable`: 0, not the plain masteries 342 / 343.
    assert_eq!(throw_mastery(&f, &t, Some(u), Some(axe), None, 0), 0);
    assert_eq!(throw_mastery(&f, &t, Some(u), Some(axe), None, 1), 0);
    f.items[axe].throw = true;
    assert_eq!(throw_mastery(&f, &t, Some(u), Some(axe), None, 0), 70);
    assert_eq!(throw_mastery(&f, &t, Some(u), Some(axe), None, 1), 90);
    // A used skill off the gate (range ≠ 2): 0.
    assert_eq!(throw_mastery(&f, &t, Some(u), Some(axe), Some(0), 0), 0);
    f.units[u].used = None;
    assert_eq!(throw_mastery(&f, &t, Some(u), Some(axe), None, 0), 0);
}

#[test]
fn concentration_helper() {
    let mut r = skill_rec();
    r.param1 = 12;
    let t = tables_with(r);
    let mut f = Fake::default();
    let u = player(&mut f);
    assert_eq!(concentration(&f, &t, u, 0), 0);
    f.units[u].states.push(42);
    f.units[u].state_stats.insert((42, 25), 100);
    assert_eq!(concentration(&f, &t, u, 0), 150);
}

// ------------------------------------------------------------ missiles

#[test]
fn missile_specials() {
    let mut m = missile_rec();
    (m.param1, m.param2, m.cltparam3, m.cltparam4) = (10, 3, 25, 70);
    (m.range, m.levrange) = (5, 0xFFFF);
    (m.emin, m.emax, m.hitshift, m.elen, m.elevlen1) = (2, 4, 8, 50, 5);
    (m.mindamage, m.maxdamage) = (1, 2);
    let mut t = skill_tables(vec![skill_rec()]);
    t.missiles = vec![m];
    let mut f = Fake::default();
    let ms = |f: &mut Fake, c: u8, l: i32| miss_special(f, &t, None, None, c, 0, l);
    assert_eq!(ms(&mut f, 0, 9), 10);
    assert_eq!(ms(&mut f, 18, 9), 9);
    // No level guards: sl12 at 0 is Param1 − Param2, cd34 at 1 is DM.
    assert_eq!(ms(&mut f, 29, 0), 7);
    assert_eq!(ms(&mut f, 36, 1), 31);
    assert_eq!(ms(&mut f, 28, 3), 2);
    assert_eq!((ms(&mut f, 22, 1), ms(&mut f, 23, 1)), (512, 1_024));
    assert_eq!(
        (ms(&mut f, 19, 1), ms(&mut f, 26, 1), ms(&mut f, 25, 1)),
        (2, 256, 2)
    );
    assert_eq!((ms(&mut f, 21, 0), ms(&mut f, 21, 3)), (50, 60));
    assert_eq!(ms(&mut f, 43, 1), 0);
    assert_eq!(miss_special(&mut f, &t, None, None, 0, 1, 1), 0);
    // A missile unit that is not a missile gives 0.
    let u = player(&mut f);
    assert_eq!(miss_special(&mut f, &t, Some(u), None, 0, 0, 1), 0);
}

#[test]
fn missile_synergy_before_shift() {
    let mut m = missile_rec();
    (m.emin, m.emax, m.hitshift, m.edmgsympercalc) = (3, 3, 8, 0);
    let mut t = skill_tables(vec![skill_rec()]);
    t.missiles = vec![m];
    t.miss_code = vec![0x07, 50, 0x00];
    let mut f = Fake::default();
    // (3 + pct(3, 50, 100)) << 8 = 4 << 8 (skills elemental would give
    // 768 + 384).
    assert_eq!(miss_elem_min(&mut f, &t, None, None, 0, 1), 1_024);
}

#[test]
fn skill_descmissile_specials() {
    let mut r = skill_rec();
    r.skilldesc = 0;
    let mut m = missile_rec();
    (m.emin, m.emax, m.hitshift, m.range, m.levrange) = (2, 4, 8, 3, 2);
    let mut t = tables_with(r);
    t.missiles = vec![missile_rec(), m];
    t.skilldesc[0].descmissile1 = 1;
    t.skilldesc[0].descmissile2 = 0xFFFF;
    let mut f = Fake::default();
    let sp = |f: &mut Fake, c: u8, l: i32| special(f, &t, None, c, 0, l);
    assert_eq!((sp(&mut f, 26, 1), sp(&mut f, 27, 1)), (2, 4));
    assert_eq!((sp(&mut f, 43, 1), sp(&mut f, 44, 1)), (512, 1_024));
    // m2eo / m2ey read descmissile1 (Edge case 2); m2en reads slot 2.
    assert_eq!((sp(&mut f, 45, 1), sp(&mut f, 46, 1)), (512, 1_024));
    assert_eq!(sp(&mut f, 29, 1), 0);
    assert_eq!(sp(&mut f, 26, 0), 0);
    // m1rn: Range + lvl × LevRange, no guard.
    assert_eq!((sp(&mut f, 35, 0), sp(&mut f, 35, 4)), (3, 11));
}

// ------------------------------------------------------------ learning

#[test]
fn learning_rules() {
    let mut a = skill_rec();
    (a.reqlevel, a.maxlvl, a.ingame, a.reqstr) = (6, 0, true, 30);
    a.reqskill1 = 1;
    let mut b = skill_rec();
    (b.reqlevel, b.maxlvl, b.ingame) = (1, 3, true);
    let t = skill_tables(vec![a, b]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 1).with(12, 6).with(0, 30));
    assert_eq!(max_level(&t.skills[0]), 20);
    assert_eq!(max_level(&t.skills[1]), 3);
    assert_eq!(req_level(&f, &t, u, 0), 6);
    assert_eq!(req_level(&f, &t, u, 9), i32::MAX);
    // Missing required skill: code 3.
    assert_eq!(check_skill_point(&f, &t, u, 0), SkillPointCheck::Code3);
    f.units[u].skills = vec![entry(1, 1, -1)];
    assert_eq!(check_skill_point(&f, &t, u, 0), SkillPointCheck::Ok);
    // Hard points raise the required level.
    f.units[u].skills.push(entry(0, 1, -1));
    assert_eq!(req_level(&f, &t, u, 0), 7);
    assert_eq!(check_skill_point(&f, &t, u, 0), SkillPointCheck::Code3);
    // At max level: code 2. Bad id: code 2.
    f.units[u].skills.push(entry(1, 3, -1));
    f.units[u].skills.remove(0);
    assert_eq!(check_skill_point(&f, &t, u, 1), SkillPointCheck::Code2);
    assert_eq!(check_skill_point(&f, &t, u, 9), SkillPointCheck::Code2);
    // Spending: 1 point with skpoints empty, base stat 5 must cover it.
    assert!(!spend_skill_point(&mut f, &t, u, 1));
    f.units[u].base.insert((5, 0), 1);
    assert!(spend_skill_point(&mut f, &t, u, 1));
    assert_eq!(f.log.last().unwrap(), "addskill 0 1 1");
}

// ------------------------------------------------------------ game files

/// Real base-game `skills.txt` vectors. Needs the
/// extracted `.bin` tables (`mpq-tool extract`) under
/// `D2_GAME_DIR/extracted/patch_d2/data/global/excel/`.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn real_skill_vectors() {
    let t = game_loader::skill_tables();
    let mut f = Fake::default();
    let mm = |f: &mut Fake, s: i32, l: i32| {
        (
            elem_min(f, &t, None, s, l, false),
            elem_max(f, &t, None, s, l, false),
        )
    };
    const FIRE_BOLT: i32 = 36;
    const FIRE_BALL: i32 = 47;
    const FROZEN_ORB: i32 = 64;
    const TELEPORT: i32 = 54;
    assert_eq!(mm(&mut f, FIRE_BOLT, 1), (768, 1_536));
    assert_eq!(mm(&mut f, FIRE_BOLT, 20), (11_648, 15_488));
    assert_eq!(mm(&mut f, FIRE_BALL, 1), (1_536, 3_584));
    assert_eq!(mm(&mut f, FIRE_BALL, 20), (51_072, 57_984));
    assert_eq!(mm(&mut f, FROZEN_ORB, 1), (10_240, 11_520));
    assert_eq!(mm(&mut f, FROZEN_ORB, 20), (67_072, 70_784));
    assert_eq!(
        [1, 10, 20].map(|l| elem_len(&mut f, &t, None, FROZEN_ORB, l)),
        [200, 425, 675]
    );
    assert_eq!(mana_cost(&t.skills[FIRE_BOLT as usize], 1), 640);
    assert_eq!(
        [1, 10, 20].map(|l| mana_cost(&t.skills[FROZEN_ORB as usize], l)),
        [6_400, 7_552, 8_832]
    );
    assert_eq!(
        [1, 10, 20].map(|l| mana_cost(&t.skills[TELEPORT as usize], l)),
        [6_144, 3_840, 1_280]
    );
}

/// Loading the extracted the base game tables (tests only).
pub(crate) mod game_loader {
    use super::super::SkillTables;
    use d2_data::bin::BinTable;

    /// Reads one extracted excel file. Tests may read game files
    /// (conventions); the sim never does.
    #[allow(clippy::disallowed_methods)]
    pub fn read(file: &str) -> Vec<u8> {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let path = format!("{dir}/extracted/patch_d2/data/global/excel/{file}");
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    pub fn table(name: &str, size: usize) -> BinTable {
        let file = format!("{name}.bin");
        BinTable::parse(name, "patch_d2.mpq", &file, &read(&file), size).expect("table parses")
    }

    pub fn skill_tables() -> SkillTables {
        use d2_data::tables::{decode_all, Missiles, Record, Skilldesc, Skills};
        SkillTables {
            skills: decode_all(&table("skills", Skills::SIZE)).unwrap(),
            skilldesc: decode_all(&table("skilldesc", Skilldesc::SIZE)).unwrap(),
            missiles: decode_all(&table("missiles", Missiles::SIZE)).unwrap(),
            skills_code: read("skillscode.bin"),
            miss_code: read("misscode.bin"),
            level_cap: super::super::LEVEL_CAP_114D,
            stat_count: table("itemstatcost", d2_data::tables::Itemstatcost::SIZE).count as i32,
        }
    }
}
