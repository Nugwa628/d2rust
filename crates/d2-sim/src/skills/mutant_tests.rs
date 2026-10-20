// Tests written against surviving mutants of `cargo mutants`:
// each asserts what the rule says at the boundary or branch the
// mutant changed. The fake world is `skills::fake`.
use super::fake::*;
use super::*;
use crate::units::UnitType;

/// Skill tables with `skills`, the formula buffer `code` and one blank
/// skilldesc record.
fn tables_with(skills: Vec<Skills>, code: Vec<u8>) -> SkillTables {
    let mut t = skill_tables(skills);
    t.skills_code = code;
    t
}

fn item_entry(skill: i32, base: i32, owner: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base,
        owner_guid: owner,
        ..SkillEntry::default()
    }
}

// ---------------------------------------------------------------- calc

// op 0x01: an index ≥ the function count (7 for
// skills) pushes 0 and pops nothing. 5 6 9 CALL7 + → 9 + 0 = 9.
#[test]
fn call_past_function_count_pops_nothing() {
    let t = tables_with(
        vec![skill_rec()],
        vec![0x07, 5, 0x07, 6, 0x07, 9, 0x01, 7, 0x10, 0x00],
    );
    let mut f = Fake::default();
    assert_eq!(eval_skill(&mut f, &t, None, 0, 0, 1), 9);
}

// Skills `rand(a, b)` with no unit: `a ≥ b` → a, else 0 (the
// implementation's narrowest reading, TODO in `levels.rs`).
#[test]
fn rand_without_unit() {
    // rand(5, 3), rand(3, 5), rand(4, 4).
    for (a, b, want) in [(5, 3, 5), (3, 5, 0), (4, 4, 4)] {
        let t = tables_with(vec![skill_rec()], vec![0x07, a, 0x07, b, 0x01, 2, 0x00]);
        let mut f = Fake::default();
        assert_eq!(eval_skill(&mut f, &t, None, 0, 0, 1), want, "{a} {b}");
    }
}

// Skills `stat(s, mode)`: `s < 0` or `s ≥ itemstatcost count` → 0.
#[test]
fn stat_function_range() {
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    for s in [65535u16, 359, 358] {
        f.units[u].stats.insert((s, 0), 77);
    }
    // stat(-1, 0), stat(359, 0), stat(358, 0).
    for (code, want) in [
        (vec![0x07, 0xFF, 0x07, 0, 0x01, 5, 0x00], 0),
        (vec![0x08, 0x67, 0x01, 0x07, 0, 0x01, 5, 0x00], 0),
        (vec![0x08, 0x66, 0x01, 0x07, 0, 0x01, 5, 0x00], 77),
    ] {
        let t = tables_with(vec![skill_rec()], code);
        assert_eq!(eval_skill(&mut f, &t, Some(u), 0, 0, 1), want);
    }
}

// Skills `miss(m, c)`: missile special `c` of missile `m` at the
// context level (misscalc 18 `lvl`).
#[test]
fn miss_function() {
    let t = tables_with(vec![skill_rec()], vec![0x07, 0, 0x07, 18, 0x01, 4, 0x00]);
    let mut f = Fake::default();
    assert_eq!(eval_skill(&mut f, &t, None, 0, 0, 7), 7);
}

// Skills parameter callback: special value `c` of the context skill
// (skillcalc 8 `par1`, 16 `lvl`).
#[test]
fn skill_param_callback() {
    let mut r = skill_rec();
    r.param1 = 42;
    let t = tables_with(vec![r], vec![0x04, 8, 0x04, 16, 0x10, 0x00]);
    let mut f = Fake::default();
    assert_eq!(eval_skill(&mut f, &t, None, 0, 0, 5), 47);
}

// ---------------------------------------------------------------

// `highest_entry`: among non-native entries the larger base wins, the
// first on a tie.
#[test]
fn highest_entry_item_entries() {
    let a = item_entry(3, 2, 7);
    let b = item_entry(3, 5, 8);
    let c = item_entry(3, 2, 9);
    assert_eq!(highest_entry(&[a, b], 3), Some(b));
    assert_eq!(highest_entry(&[b, a], 3), Some(b));
    assert_eq!(highest_entry(&[a, c], 3), Some(a));
}

// `bonus_level`: a class skill of the player's class adds
// `addskill_tab(188)` at layer `page + 8 × class − 1`.
#[test]
fn bonus_level_skill_tab_layer() {
    let mut r = skill_rec();
    r.charclass = 3;
    r.skilldesc = 0;
    let mut t = skill_tables(vec![r]);
    t.skilldesc[0].skillpage = 2;
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 3));
    // 2 + 24 − 1 = 25.
    f.units[u].stats.insert((188, 25), 4);
    f.units[u].stats.insert((188, 3), 100);
    let e = item_entry(0, 1, -1);
    assert_eq!(bonus_level(&f, &t, u, &e), 4);
}

// ---------------------------------------------------------------

// skillcalc 12 / 13: Param5, Param6.
#[test]
fn special_par5_par6() {
    let mut r = skill_rec();
    r.param5 = 55;
    r.param6 = 66;
    let t = skill_tables(vec![r]);
    let mut f = Fake::default();
    assert_eq!(special(&mut f, &t, None, 12, 0, 1), 55);
    assert_eq!(special(&mut f, &t, None, 13, 0, 1), 66);
}

/// Tables with skill 0 whose skilldesc 0 names missiles 1, 2, 3 (the
/// second has Range 10, LevRange 2).
fn desc_tables() -> SkillTables {
    let mut r = skill_rec();
    r.skilldesc = 0;
    let mut t = skill_tables(vec![r]);
    t.skilldesc[0].descmissile1 = 1;
    t.skilldesc[0].descmissile2 = 2;
    t.skilldesc[0].descmissile3 = 3;
    t.missiles = vec![missile_rec(); 4];
    for (i, m) in t.missiles.iter_mut().enumerate() {
        m.range = 10 * i as u16;
        m.levrange = 2;
    }
    t
}

// skillcalc 35–37: Range + lvl × LevRange of descmissile 1–3.
#[test]
fn special_missile_range_slots() {
    let t = desc_tables();
    let mut f = Fake::default();
    for (c, want) in [(35, 10 + 6), (36, 20 + 6), (37, 30 + 6)] {
        assert_eq!(special(&mut f, &t, None, c, 0, 3), want, "{c}");
    }
}

// skillcalc 23–25: mastery type 0 reads passivestat 342 / 345, type 1
// 343 / 346, type 2 344 / 347. The formulas at offsets 3 and 6 run from
// their offset to the buffer end.
#[test]
fn special_mastery_types() {
    let mut r = skill_rec();
    r.passivestat1 = 342;
    r.passivecalc1 = 0;
    r.passivestat2 = 343;
    r.passivecalc2 = 3;
    r.passivestat3 = 347;
    r.passivecalc3 = 6;
    let t = tables_with(
        vec![r],
        vec![0x07, 11, 0x00, 0x07, 22, 0x00, 0x07, 33, 0x00],
    );
    let mut f = Fake::default();
    assert_eq!(special(&mut f, &t, None, 23, 0, 1), 11);
    assert_eq!(special(&mut f, &t, None, 24, 0, 1), 22);
    assert_eq!(special(&mut f, &t, None, 25, 0, 1), 33);
}

// skillcalc 17 / 18 shift elem min / max right by 8; 38 / 39 do not.
#[test]
fn special_elem_shift() {
    let mut r = skill_rec();
    r.emin = 512;
    r.emax = 1024;
    let t = skill_tables(vec![r]);
    let mut f = Fake::default();
    assert_eq!(special(&mut f, &t, None, 17, 0, 1), 2);
    assert_eq!(special(&mut f, &t, None, 18, 0, 1), 4);
    assert_eq!(special(&mut f, &t, None, 38, 0, 1), 512);
    assert_eq!(special(&mut f, &t, None, 39, 0, 1), 1024);
}

// skillcalc 28: `lvl > 0 ? miss_elem_len(descmissile1) : 0`; the length
// at lvl ≤ 0 is the missile's ELen otherwise.
#[test]
fn special_missile_len_guard() {
    let mut t = desc_tables();
    t.missiles[1].elen = 25;
    let mut f = Fake::default();
    assert_eq!(special(&mut f, &t, None, 28, 0, 1), 25);
    assert_eq!(special(&mut f, &t, None, 28, 0, 0), 0);
}

// ---------------------------------------------------------------

// `bracket`: one level inside each band (L = 1, 10, 100, 1000, 10000).
#[test]
fn bracket_bands() {
    let l = [1, 10, 100, 1000, 10_000];
    for (lvl, want) in [
        (1, 0),
        (5, 4),
        (12, 7 + 40),
        (20, 7 + 80 + 400),
        (25, 7 + 80 + 600 + 3000),
        (40, 7 + 80 + 600 + 6000 + 120_000),
    ] {
        assert_eq!(bracket(lvl, l), want, "{lvl}");
    }
}

// Elem_len brackets `lvl ≤ 8`: (lvl − 1) × LL1.
#[test]
fn elem_len_low_band() {
    let mut r = skill_rec();
    r.elen = 100;
    r.elevlen1 = 10;
    r.elevlen2 = 1000;
    let t = skill_tables(vec![r]);
    let mut f = Fake::default();
    assert_eq!(elem_len(&mut f, &t, None, 0, 5), 140);
    assert_eq!(elem_len(&mut f, &t, None, 0, 10), 100 + 70 + 2000);
}

// Mastery stat by EType.
#[test]
fn elem_mastery_stats() {
    for (e, s) in [
        (1, Some(329)),
        (2, Some(330)),
        (4, Some(331)),
        (5, Some(332)),
        (12, Some(331)),
        (3, None),
        (0, None),
    ] {
        assert_eq!(elem_mastery_stat(e), s, "{e}");
    }
}

// (Edge case 1): the min synergy (50 %) applies only if
// `v > 256` or `EMinLev1 ≠ 0`; the max synergy always.
#[test]
fn elem_min_synergy_gate() {
    let mk = |emin: u32, lev1: u32| {
        let mut r = skill_rec();
        r.emin = emin;
        r.emax = emin;
        r.eminlev1 = lev1;
        r.edmgsympercalc = 0;
        tables_with(vec![r], vec![0x07, 50, 0x00])
    };
    let mut f = Fake::default();
    let t = mk(256, 0);
    assert_eq!(elem_min(&mut f, &t, None, 0, 1, false), 256);
    assert_eq!(elem_max(&mut f, &t, None, 0, 1, false), 384);
    let t = mk(512, 0);
    assert_eq!(elem_min(&mut f, &t, None, 0, 1, false), 768);
    let t = mk(100, 1);
    assert_eq!(elem_min(&mut f, &t, None, 0, 1, false), 150);
}

// ---------------------------------------------------------------- missiles

/// Missile tables: missile 0 with Param1 42 and damage fields; skill 0
/// with Param1 42; the missile formula buffer `code`.
fn miss_tables(code: Vec<u8>) -> SkillTables {
    let mut r = skill_rec();
    r.param1 = 42;
    let mut t = skill_tables(vec![r]);
    let mut m = missile_rec();
    m.mindamage = 512;
    m.maxdamage = 1024;
    m.emin = 768;
    m.emax = 1280;
    t.missiles = vec![m];
    t.miss_code = code;
    t
}

fn eval_m(code: Vec<u8>, lvl: i32) -> i32 {
    let t = miss_tables(code);
    let mut f = Fake::default();
    eval_missile(&mut f, &t, None, None, 0, 0, lvl)
}

// Missiles family: 4 functions of arity
// 2: min, max, rand (no draw, Open question 1: 0), skill(s, c) on the
// owner; `param(c)` is the missile special value (misscalc 18 `lvl`).
#[test]
fn missile_context_functions() {
    assert_eq!(eval_m(vec![0x07, 3, 0x07, 5, 0x01, 0, 0x00], 1), 3);
    assert_eq!(eval_m(vec![0x07, 3, 0x07, 5, 0x01, 1, 0x00], 1), 5);
    assert_eq!(eval_m(vec![0x07, 0, 0x07, 8, 0x01, 2, 0x00], 1), 0);
    // skill(0, 8): Param1 of skill 0 (no owner: level 0, `par1` is
    // unguarded).
    assert_eq!(eval_m(vec![0x07, 0, 0x07, 8, 0x01, 3, 0x00], 1), 42);
    // Index 4 is past the missiles table: 0, nothing popped (7 + 0).
    assert_eq!(
        eval_m(vec![0x07, 7, 0x07, 1, 0x07, 2, 0x01, 4, 0x10, 0x00], 1),
        2
    );
    assert_eq!(eval_m(vec![0x04, 18, 0x00], 7), 7);
}

// With a unit, class < 0 is the
// unit's class (lightningjavelin's `dl12` at server damage 12 reads its
// own dParam1 through class −1); a unit that is not a missile → 0.
#[test]
fn missile_eval_negative_class_is_the_units() {
    let mut t = miss_tables(vec![0x04, 16, 0x00]);
    let mut m1 = missile_rec();
    m1.dparam1 = 100;
    t.missiles.push(m1);
    let mut f = Fake::default();
    let m = f.add(FUnit::new(UnitType::Missile, 1));
    assert_eq!(eval_missile(&mut f, &t, Some(m), None, 0, -1, -1), 100);
    assert_eq!(eval_missile(&mut f, &t, Some(m), None, 0, 0, -1), 0);
    let p = f.add(FUnit::new(UnitType::Player, 1));
    assert_eq!(eval_missile(&mut f, &t, Some(p), None, 0, 1, 1), 0);
}

// misscalc 24 / 25 shift the physical min / max right by 8; 26 / 27 do
// not; 19 / 20 and 22 / 23 likewise for the elemental values.
#[test]
fn missile_special_shifts() {
    let t = miss_tables(Vec::new());
    let mut f = Fake::default();
    for (c, want) in [
        (24, 2),
        (25, 4),
        (26, 512),
        (27, 1024),
        (19, 3),
        (20, 5),
        (22, 768),
        (23, 1280),
    ] {
        assert_eq!(miss_special(&mut f, &t, None, None, c, 0, 1), want, "{c}");
    }
}

// `ELenSymPerCalc ≠ −1` and `p ≠ 0` → `v += pct(v, p, 100)`.
#[test]
fn elem_len_synergy() {
    let mut r = skill_rec();
    r.elen = 100;
    r.elensympercalc = 0;
    let t = tables_with(vec![r], vec![0x07, 50, 0x00]);
    let mut f = Fake::default();
    assert_eq!(elem_len(&mut f, &t, None, 0, 1), 150);
}

// With `SrcDam`, only a wield-type-2 weapon gives the
// weapon's own damage; another weapon reads `mindamage(21)` /
// `maxdamage(22)`.
#[test]
fn phys_srcdam_weapon_source() {
    let mut r = skill_rec();
    r.srcdam = 128;
    let t = skill_tables(vec![r]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 0).with(21, 5).with(22, 7));
    let i = f.add_item(FItem {
        damage: (10, 20),
        ..FItem::default()
    });
    f.units[u].weapon = Some(i);
    assert_eq!(phys_min(&mut f, &t, Some(u), 0, 1, true), 5);
    assert_eq!(phys_max(&mut f, &t, Some(u), 0, 1, true), 7);
    f.items[i].wield = 2;
    assert_eq!(phys_min(&mut f, &t, Some(u), 0, 1, true), 10);
    assert_eq!(phys_max(&mut f, &t, Some(u), 0, 1, true), 20);
}

// A missile unit that is not a missile → 0; missile id < 0 with a
// unit → the unit's class; `lvl ≤ 0` with a unit → its stored level
// (the fake's is 0), else `lvl`.
#[test]
fn missile_unit_rules() {
    let mut t = skill_tables(vec![skill_rec()]);
    let mut m0 = missile_rec();
    m0.emin = 100;
    m0.minelev1 = 10;
    let mut m1 = m0.clone();
    m1.emin = 300;
    t.missiles = vec![m0, m1];
    let mut f = Fake::default();
    let p = f.add(FUnit::new(UnitType::Player, 1));
    let m = f.add(FUnit::new(UnitType::Missile, 1));
    assert_eq!(miss_elem_min(&mut f, &t, Some(p), None, 0, 1), 0);
    assert_eq!(miss_elem_min(&mut f, &t, Some(m), None, -1, 1), 300);
    assert_eq!(miss_elem_min(&mut f, &t, Some(m), None, 0, 1), 100);
    // lvl 3: 100 + 2 × 10.
    assert_eq!(miss_elem_min(&mut f, &t, Some(m), None, 0, 3), 120);
}

// Weapon mastery: the throw path (stats 345–347) needs a throwing
// item, a skill with range 2 and an `itypea1` that is-a 48; type 1 reads
// 343 / 346.
#[test]
fn weapon_mastery_throw_path() {
    let throw_skill = |range: u8, itype: u16| {
        let mut r = skill_rec();
        r.range = range;
        r.itypea1 = itype;
        r
    };
    let t = skill_tables(vec![
        throw_skill(2, 48),
        throw_skill(0, 48),
        throw_skill(2, 5),
    ]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    f.units[u].entries.extend([
        (343, vec![(50, 43)]),
        (344, vec![(50, 44)]),
        (346, vec![(50, 46)]),
    ]);
    let thrown = f.add_item(FItem {
        types: vec![50],
        throw: true,
        ..FItem::default()
    });
    let plain = f.add_item(FItem {
        types: vec![50],
        ..FItem::default()
    });
    let wm = |f: &Fake, item, skill| weapon_mastery(f, &t, Some(u), Some(item), Some(skill), 1);
    assert_eq!(wm(&f, thrown, 0), 46);
    assert_eq!(wm(&f, plain, 0), 43);
    assert_eq!(wm(&f, thrown, 1), 43);
    assert_eq!(wm(&f, thrown, 2), 43);
}

// ---------------------------------------------------------------

// Shifted cost: `max(cost >> 8, 0)`.
#[test]
fn mana_cost_shifted_value() {
    let mut r = skill_rec();
    r.mana = 512;
    let t = skill_tables(vec![r]);
    assert_eq!(mana_cost_shifted(&t, 0, 1), 2);
}

// `can_afford`: item entries need charges > 0; `srvdofunc` 116 is free
// only while shapeshifted.
#[test]
fn can_afford_rules() {
    let mut r = skill_rec();
    r.mana = 10;
    let mut r116 = r.clone();
    r116.srvdofunc = 116;
    let t = skill_tables(vec![r, r116]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    let mut item = item_entry(0, 1, 7);
    item.charges = 3;
    assert!(can_afford(&f, &t, u, &item));
    item.charges = 0;
    assert!(!can_afford(&f, &t, u, &item));
    let e0 = item_entry(0, 1, -1);
    let e116 = item_entry(1, 1, -1);
    assert!(!can_afford(&f, &t, u, &e116));
    f.shifted = true;
    assert!(can_afford(&f, &t, u, &e116));
    assert!(!can_afford(&f, &t, u, &e0));
}

// ---------------------------------------------------------------

// Step 3: a required skill needs a native entry with base > 0.
#[test]
fn skill_reqs_need_base() {
    let mut r = skill_rec();
    r.reqskill1 = 0;
    let t = skill_tables(vec![skill_rec(), r]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 0));
    f.units[u].skills = vec![item_entry(0, 0, -1)];
    assert!(!meets_skill_reqs(&f, &t, u, 1));
    f.units[u].skills = vec![item_entry(0, 1, -1)];
    assert!(meets_skill_reqs(&f, &t, u, 1));
}

// ----------------------------------------------------------------

mod use_mutants {
    use crate::combat::RoomKind;
    use crate::skills::use_::tests::*;
    use crate::skills::use_::*;
    use crate::units::UnitType;
    use d2_data::tables::Skills;

    // A resync only when more than 25 frames passed.
    #[test]
    fn resync_after_more_than_25_frames() {
        let mut f = F::new();
        let p = f.player(&[]);
        f.frame = 35;
        f.units[p].last_point = 10;
        assert_eq!(validate_point(&mut f, p, &point(5, 500, 500)), Err(1));
        assert!(f.take_log().is_empty());
        f.frame = 36;
        assert_eq!(validate_point(&mut f, p, &point(5, 500, 500)), Err(1));
        assert_eq!(f.take_log(), ["send 0 Resync"]);
    }

    // Only an item in the unit's own inventory skips the act
    // and distance tests.
    #[test]
    fn unit_validator_own_inventory_items_only() {
        let mut f = F::new();
        let p = f.player(&[]);
        let mut item = U::new(UnitType::Item, 0);
        item.act = 1;
        let i = f.add(item);
        let m = f.add(U::new(UnitType::Monster, 0));
        f.units[m].act = 1;
        f.units[m].owner = Some(p);
        assert_eq!(
            validate_unit(&f, p, &unit_msg(6, 4, i as u32)),
            Err(TargetError::OtherAct)
        );
        assert_eq!(
            validate_unit(&f, p, &unit_msg(6, 1, m as u32)),
            Err(TargetError::OtherAct)
        );
        f.units[i].owner = Some(p);
        assert!(validate_unit(&f, p, &unit_msg(6, 4, i as u32)).is_ok());
    }

    // And 6: 0x07 is a plain handler; every hold id with its
    // hand's skill returns 0.
    #[test]
    fn handlers_shift_and_hold_ids() {
        let t = tables(1, &[]);
        let mut f = F::new();
        let p = f.player(&[(0, 1)]);
        f.units[p].left = Some(native(0, 1));
        f.units[p].right = Some(native(0, 1));
        let m = f.add(U::new(UnitType::Monster, 0));
        let r = handle_message(&mut f, &t, p, &unit_msg(msg::LEFT_UNIT_SHIFT, 1, m as u32));
        assert_eq!(r, Some(MsgResult::Code(0)));
        for id in [
            msg::LEFT_UNIT_HOLD,
            msg::LEFT_UNIT_SHIFT_HOLD,
            msg::RIGHT_UNIT_HOLD,
            msg::RIGHT_UNIT_SHIFT_HOLD,
        ] {
            let r = handle_hold(&mut f, &t, p, &unit_msg(id, 1, m as u32));
            assert_eq!(r, MsgResult::Code(0), "{id:#x}");
        }
        let r = handle_hold(&mut f, &t, p, &point(msg::RIGHT_POINT_HOLD, 100, 100));
        assert_eq!(r, MsgResult::Code(0));
        f.units[p].right = None;
        let r = handle_hold(&mut f, &t, p, &point(msg::RIGHT_POINT_HOLD, 100, 100));
        assert_eq!(r, MsgResult::Code(3));
    }

    // Both hands must hold an equippable weapon (type 45) that
    // is not type 38.
    #[test]
    fn dual_wield_needs_two_weapons() {
        let mut f = F::new();
        let p = dual_wielder(&mut f);
        // A non-weapon in the left hand.
        f.items[1] = vec![50];
        assert_eq!(dual_wield(&mut f, p, 0), 0);
        assert_eq!(f.units[p].param4, 0);
        // A weapon that is also type 38.
        f.items[1] = vec![45, 38];
        assert_eq!(dual_wield(&mut f, p, 0), 0);
        assert_eq!(f.units[p].param4, 0);
        f.items[1] = vec![45];
        assert_eq!(dual_wield(&mut f, p, 0), 5);
    }

    // Arriving sets the arrived flag (kept when already set).
    #[test]
    fn arrived_flag_is_set() {
        let t = tables(1, &[]);
        let mut f = F::new();
        let p = caster(&mut f, 0, 1, 0);
        f.units[p].used_flags = SKILL_MOVING | SKILL_ARRIVED;
        f.units[p].path = 2;
        attack_frame_event(&mut f, &t, p, 1, 0);
        assert_eq!(f.units[p].used_flags, SKILL_MOVING | SKILL_ARRIVED);
    }

    // Only an item skill with 0 charges stops; a native
    // entry has no charges.
    #[test]
    fn start_native_entry_without_charges() {
        let t = tables(1, &[]);
        let mut f = F::new();
        let p = caster(&mut f, 0, 1, 0);
        assert_eq!(start(&mut f, &t, p), 1);
    }

    // TargetableOnly accepts a hostile, pet or ally target.
    #[test]
    fn targetable_only_accepts_ally() {
        let t = tables(
            1,
            &[(0, &|r: &mut Skills| {
                r.targetableonly = true;
                r.targetally = true;
            })],
        );
        let mut f = F::new();
        let p = caster(&mut f, 0, 1, 0);
        let m = f.add(U::new(UnitType::Monster, 0));
        f.units[p].target = Some(m);
        f.ally = true;
        assert_eq!(start(&mut f, &t, p), 1);
    }

    // `srvdofunc` 116 skips the mana check only while
    // shapeshifted.
    #[test]
    fn mana_check_were_form() {
        let t = tables(
            2,
            &[
                (0, &|r: &mut Skills| r.mana = 10),
                (1, &|r: &mut Skills| {
                    r.mana = 10;
                    r.srvdofunc = 116;
                }),
            ],
        );
        let mut f = F::new();
        let p = f.player(&[(0, 1), (1, 1)]);
        assert!(!mana_check(&f, &t, p, &native(1, 1), 1));
        f.units[p].shapeshifted = true;
        assert!(mana_check(&f, &t, p, &native(1, 1), 1));
        assert!(!mana_check(&f, &t, p, &native(0, 1), 1));
    }

    // Only a skill without InTown fails in town; step
    // 6.4: line of sight 5 uses the fifth mask.
    #[test]
    fn start_core_town_and_los() {
        let t = tables(
            2,
            &[
                (0, &|r: &mut Skills| r.intown = false),
                (1, &|r: &mut Skills| r.lineofsight = 5),
            ],
        );
        let mut f = F::new();
        let p = caster(&mut f, 0, 1, 0);
        assert_eq!(start(&mut f, &t, p), 1);
        let q = caster(&mut f, 1, 1, 0);
        f.units[q].room = RoomKind::Town;
        let m = f.add(U::new(UnitType::Monster, 0));
        f.units[q].target = Some(m);
        f.hostile = true;
        assert_eq!(start(&mut f, &t, q), 1);
    }

    /// Tables for the do core: 1 do function 1; 2 do function 1, mana
    /// 10 paid at do; 3 do function 2, ItemEffect 1; 4 a missing
    /// srvmissile 5, no do function; 5 do function 1, mana 10.
    fn do_tables() -> crate::skills::SkillTables {
        tables(
            6,
            &[
                (1, &|r: &mut Skills| r.srvdofunc = 1),
                (2, &|r: &mut Skills| {
                    r.srvdofunc = 1;
                    r.mana = 10;
                    r.usemanaondo = true;
                }),
                (3, &|r: &mut Skills| {
                    r.srvdofunc = 2;
                    r.itemeffect = 1;
                }),
                (4, &|r: &mut Skills| r.srvmissile = 5),
                (5, &|r: &mut Skills| {
                    r.srvdofunc = 1;
                    r.mana = 10;
                }),
            ],
        )
    }

    // The used skill with this id and level > 0 needs no
    // entry of its own.
    #[test]
    fn do_core_used_skill_level() {
        let t = do_tables();
        let mut f = F::new();
        let p = caster(&mut f, 1, 1, 0);
        f.units[p].skills.clear();
        assert_eq!(do_core(&mut f, &t, p, 1, 1, false, false, false), 1);
    }

    // The mana check runs only with `usemanaondo`, on the
    // used entry when it is this skill, else on a native entry of it.
    #[test]
    fn do_core_mana_at_do() {
        let t = do_tables();
        let mut f = F::new();
        // No usemanaondo: no check (cost 10, mana 0).
        let p = caster(&mut f, 5, 1, 0);
        assert_eq!(do_core(&mut f, &t, p, 5, 1, false, false, false), 1);
        // No used skill: a native entry of skill 2 (cost 10).
        let p = caster(&mut f, 2, 1, 20);
        f.units[p].used = None;
        assert_eq!(do_core(&mut f, &t, p, 2, 1, false, false, false), 1);
        f.units[p].stats.insert(crate::stats::stat::MANA, 5);
        assert_eq!(do_core(&mut f, &t, p, 2, 1, false, false, false), 0);
        // The used item entry of this skill: its charges decide.
        let p = caster(&mut f, 2, 1, 0);
        f.units[p].used = Some(crate::skills::SkillEntry {
            skill: 2,
            base: 1,
            owner_guid: 9,
            charges: 3,
            ..Default::default()
        });
        assert_eq!(do_core(&mut f, &t, p, 2, 1, false, false, false), 1);
    }

    // ItemEffect replaces the index only when > 1.
    #[test]
    fn do_core_item_effect_one() {
        let t = do_tables();
        let mut f = F::new();
        let p = caster(&mut f, 3, 1, 0);
        f.take_log();
        do_core(&mut f, &t, p, 3, 1, false, true, true);
        assert_eq!(f.take_log()[0], "srvdo 2 0 3 1 false true true");
    }

    // Only a valid srvmissile creates a missile.
    #[test]
    fn do_core_invalid_missile() {
        let t = do_tables();
        let mut f = F::new();
        let p = caster(&mut f, 4, 1, 0);
        f.take_log();
        assert_eq!(do_core(&mut f, &t, p, 4, 1, false, false, false), 0);
        assert!(f.take_log().iter().all(|l| !l.starts_with("missile")));
    }

    // The missile sets flag 0x40 (kept when already set);
    // the aimed position needs both `item` and `aim`.
    #[test]
    fn do_core_missile_flag_and_aim() {
        let t = tables(2, &[(1, &|r: &mut Skills| r.srvmissile = 0)]);
        let mut f = F::new();
        let p = caster(&mut f, 1, 1, 0);
        let m = f.add(U::new(UnitType::Monster, 0));
        f.units[p].target = Some(m);
        f.units[p].flags |= FLAG_MISSILE_FIRED;
        f.take_log();
        assert_eq!(do_core(&mut f, &t, p, 1, 1, false, false, true), 1);
        assert_ne!(f.units[p].flags & FLAG_MISSILE_FIRED, 0);
        assert_eq!(f.take_log(), ["missile 0 1 1 0 false None"]);
    }
}

// ---------------------------------------------------------------- functions.tsv check

mod table_check_mutants {
    use crate::skills::use_::table::check_tsv;

    const TSV: &str = include_str!("../../../../tables/skills/functions.tsv");

    fn whats(tsv: &str) -> Vec<String> {
        check_tsv(tsv).into_iter().map(|m| m.what).collect()
    }

    // A repeated slot is reported by its kind code and index.
    #[test]
    fn repeated_slot_names_kind() {
        let row = TSV.lines().nth(2).unwrap();
        let tsv = format!("{TSV}\n{row}\n");
        assert_eq!(whats(&tsv), ["srvst 1 repeated"]);
    }

    // An index range must be ordered and below the slot count (91 start
    // slots): `91` and `5-3` are bad indices.
    #[test]
    fn index_bounds() {
        for idx in ["91", "5-3"] {
            let tsv = format!("{TSV}\nsrvst\t{idx}\tnull\t-\tnull\t(none)\t\n");
            assert_eq!(whats(&tsv), [format!("index {idx}")], "{idx}");
        }
    }
}
