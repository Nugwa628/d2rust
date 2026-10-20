// Unit tests for skill-level rules the first test set left unclaimed.
// Values come from the rules; the fake world is `skills::fake`.
use super::fake::*;
use super::*;
use crate::units::UnitType;

fn native(skill: i32, base: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

// `bonus_level` step 1: a missing skill record (id out of range, or
// negative) gives 0, whatever the unit's bonus stats and the entry's
// level bonus.
#[test]
fn bonus_level_bad_skill_is_zero() {
    let t = skill_tables(vec![skill_rec()]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 1).with(127, 4));
    f.units[u].states.push(134); // shrine +2
    for id in [1, 57, -1] {
        f.units[u].stats.insert((107, id as u16), 3); // singleskill
        f.units[u].stats.insert((97, id as u16), 3); // nonclassskill
        let e = SkillEntry {
            level_bonus: 5,
            ..native(id, 2)
        };
        assert_eq!(bonus_level(&f, &t, u, &e), 0, "skill {id}");
        // The level is then the base alone.
        assert_eq!(skill_level(&f, &t, Some(u), Some(&e), true), 2);
    }
    // Control: the valid id 0 gets allskills + shrine + level bonus.
    let e = SkillEntry {
        level_bonus: 5,
        ..native(0, 2)
    };
    assert_eq!(bonus_level(&f, &t, u, &e), 5 + 2 + 4);
}

// `InGame` set; `level ≥ req_level`; `reqstr ≤
// str(0)`, `reqdex ≤ dex(2)`, `reqint ≤ energy(1)`, `reqvit ≤
// vitality(3)`; requirements are i16.
#[test]
fn attribute_requirements() {
    let mut r = skill_rec();
    (r.ingame, r.reqlevel) = (true, 6);
    (r.reqstr, r.reqdex, r.reqint, r.reqvit) = (30, 40, 50, 60);
    let t = skill_tables(vec![r]);
    let mut f = Fake::default();
    let u = f.add(
        FUnit::new(UnitType::Player, 1)
            .with(12, 6)
            .with(0, 30)
            .with(2, 40)
            .with(1, 50)
            .with(3, 60),
    );
    assert!(meets_attr_reqs(&f, &t, u, 0));
    // Each requirement one short fails.
    for (stat, v) in [(12u16, 5), (0, 29), (2, 39), (1, 49), (3, 59)] {
        let old = f.get(u, stat);
        f.set(u, stat, v);
        assert!(!meets_attr_reqs(&f, &t, u, 0), "stat {stat}");
        f.set(u, stat, old);
    }
    // req_level includes the native entry's base.
    f.units[u].skills = vec![native(0, 1)];
    assert!(!meets_attr_reqs(&f, &t, u, 0));
    f.set(u, 12, 7);
    assert!(meets_attr_reqs(&f, &t, u, 0));
    // InGame clear fails.
    let mut t2 = t.clone();
    t2.skills[0].ingame = false;
    assert!(!meets_attr_reqs(&f, &t2, u, 0));
    // Requirements are i16: 0xFFFF is −1, met by a stat of −1.
    let mut t3 = t.clone();
    t3.skills[0].reqstr = 0xFFFF;
    f.set(u, 0, -1);
    assert!(meets_attr_reqs(&f, &t3, u, 0));
    f.set(u, 0, -2);
    assert!(!meets_attr_reqs(&f, &t3, u, 0));
}

// Edge case 9: the cap is the class-0 maximum (99 in the base game) for every
// unit: players of any class and monsters alike.
#[test]
fn level_cap_same_for_every_unit() {
    let t = skill_tables(vec![skill_rec()]);
    assert_eq!(t.level_cap, 99);
    let mut f = Fake::default();
    let mut units = Vec::new();
    for class in 0..7 {
        units.push(f.add(FUnit::new(UnitType::Player, class)));
    }
    units.push(f.add(FUnit::new(UnitType::Monster, 0)));
    units.push(f.add(FUnit::new(UnitType::Monster, 300)));
    for u in units {
        for (base, want) in [(98, 98), (99, 99), (100, 99), (150, 99)] {
            let e = native(0, base);
            assert_eq!(skill_level(&f, &t, Some(u), Some(&e), true), want);
        }
    }
}

// The Kick flag. Player x = dex + str − 20; monster x = 3 ×
// level; x ≥ 1; min (x / 4) << 8, max (x / 3) << 8.
#[test]
fn kick_flag_damage() {
    let mut k = skill_rec();
    k.kick = true;
    let t = skill_tables(vec![k]);
    let mut f = Fake::default();
    let p = f.add(FUnit::new(UnitType::Player, 1).with(0, 100).with(2, 50));
    assert_eq!(phys_min(&mut f, &t, Some(p), 0, 1, true), (130 / 4) << 8);
    assert_eq!(phys_max(&mut f, &t, Some(p), 0, 1, true), (130 / 3) << 8);
    let m = f.add(FUnit::new(UnitType::Monster, 1).with(12, 20));
    assert_eq!(phys_min(&mut f, &t, Some(m), 0, 1, true), (60 / 4) << 8);
    assert_eq!(phys_max(&mut f, &t, Some(m), 0, 1, true), (60 / 3) << 8);
    // x is at least 1.
    let w = f.add(FUnit::new(UnitType::Player, 1));
    assert_eq!(phys_min(&mut f, &t, Some(w), 0, 1, true), 0);
    assert_eq!(phys_max(&mut f, &t, Some(w), 0, 1, true), 0);
    // A bad skill is 1 / 2 unshifted.
    assert_eq!(phys_min(&mut f, &t, Some(p), 9, 1, true), 1);
    assert_eq!(phys_max(&mut f, &t, Some(p), 9, 1, true), 2);
}

// Replies and results: validator 2 and the maximum level send the
// Attack reset (message 0x21) and return 2; validator 3 sends nothing and
// returns 3; a failed spend returns 2 silently; success returns 0.
#[test]
fn add_skill_point_results() {
    let mut a = skill_rec();
    (a.reqlevel, a.maxlvl, a.ingame) = (1, 2, true);
    a.reqstr = 50;
    let mut b = skill_rec();
    (b.reqlevel, b.maxlvl, b.ingame) = (1, 2, true);
    let t = skill_tables(vec![a, b]);
    let mut f = Fake::default();
    let u = f.add(FUnit::new(UnitType::Player, 1).with(12, 5).with(0, 10));
    // Requirement fails (strength): 3, no message.
    assert_eq!(add_skill_point(&mut f, &t, u, 0), 3);
    assert!(f.log.is_empty());
    // Bad id: 2 with the reset message.
    assert_eq!(add_skill_point(&mut f, &t, u, 9), 2);
    assert_eq!(f.log, ["msg21 0"]);
    f.log.clear();
    // No point to spend: 2, silent.
    assert_eq!(add_skill_point(&mut f, &t, u, 1), 2);
    assert!(f.log.is_empty());
    // Success: 0.
    f.units[u].base.insert((5, 0), 1);
    assert_eq!(add_skill_point(&mut f, &t, u, 1), 0);
    assert_eq!(f.log, ["addskill 0 1 1", "pointupdates 0"]);
    f.log.clear();
    // At the maximum level: 2 with the reset message.
    f.units[u].skills.push(native(1, 2));
    assert_eq!(add_skill_point(&mut f, &t, u, 1), 2);
    assert_eq!(f.log, ["msg21 0"]);
}
