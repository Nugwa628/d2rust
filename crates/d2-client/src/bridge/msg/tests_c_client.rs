//! The monster set-up of S→C 0xAC
//! with the `monstats` / `monstats2` columns of the tables.

use super::super::skills::NATIVE;
use super::super::world::{MonsterClass, MonsterSetup, SkillRow, UnitKey, MONSTER};
use super::support::Model;

fn setup() -> MonsterSetup {
    let mut res = [[0u16; 3]; 6];
    for (i, r) in res.iter_mut().enumerate() {
        *r = [i as u16 + 1, 10 + i as u16, 0xFFEC];
    }
    MonsterSetup {
        level: [5, 30, 60],
        res,
        velocity: 6,
        align: 0,
        skills: [
            (3, 2, 4),
            (-1, 5, 0),
            (4, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
        ],
        is_sel: true,
        shadow: true,
        is_att: true,
    }
}

fn model(difficulty: u8, expansion: u32) -> Model {
    let mut m = Model::default();
    let row = MonsterClass {
        setup: Some(setup()),
        ..MonsterClass::default()
    };
    m.inputs.tables.monsters = vec![Some(row); 155];
    m.inputs.tables.skills = vec![SkillRow::default(); 5];
    m.inputs.tables.monster_skill_bonus = [0, 1, 2];
    m.w.difficulty = difficulty;
    m.w.expansion = expansion;
    m
}

const B157: &str = "ac 06 00 00 00 9a 00 1a 12 b9 11 80 0e 01";

#[test]
fn monster_setup_stats_flags_and_skills() {
    let mut m = model(0, 1);
    m.hex(B157);
    let u = m.unit(UnitKey::new(MONSTER, 6));
    // (step 3 then overwrites 6 and 7).
    let stat = |s: u16| u.stat(s);
    assert_eq!(
        [12, 68, 67, 69, 36, 37, 39, 41, 43, 45].map(stat),
        [5, 100, 75, 100, 1, 2, 3, 4, 5, 6]
    );
    assert_eq!((stat(6), stat(7)), (0x8000, 0x8000));
    assert_eq!((u.flag_2, u.flag_4), (Some(true), true));
    // Skill1 (3) at level 2 + bonus 0 with mode 4; Skill2 < 0 and
    // Skill3 at level 0 are skipped.
    let l = u.skills.as_ref().unwrap();
    let got: Vec<_> = l
        .entries
        .iter()
        .map(|e| (e.skill, e.base, e.mode, e.owner))
        .collect();
    assert_eq!(got, [(3, 2, 4, NATIVE)]);
}

#[test]
fn classic_scaling_and_the_act_skill_bonus() {
    // Nightmare classic: level 30 + 25; resists of column 1; the skill at
    // 2 + bonus 1. A negative resist reads signed.
    let mut m = model(1, 0);
    m.hex(B157);
    let u = m.unit(UnitKey::new(MONSTER, 6));
    assert_eq!((u.stat(12), u.stat(36)), (55, 10));
    assert_eq!(u.skills.as_ref().unwrap().entries[0].base, 3);
    // Hell expansion: no scaling.
    let mut m = model(2, 1);
    m.hex(B157);
    let u = m.unit(UnitKey::new(MONSTER, 6));
    assert_eq!((u.stat(12), u.stat(36)), (60, -20));
}
