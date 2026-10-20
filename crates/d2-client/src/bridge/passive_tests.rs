//! Tests of the client passive refresh.

use std::sync::Arc;

use d2_data::tables::Record;
use d2_data::tables::{Missiles, Skilldesc, Skills};
use d2_sim::skills::SkillTables;

use super::passive::{apply, refresh, refresh_all};
use super::skills::{SkillEntry, SkillFx, SkillList, NATIVE};
use super::world::{ClientUnit, ClientWorld, ModelInputs, SkillRow, UnitKey, PLAYER};

const P: UnitKey = UnitKey::new(PLAYER, 1);

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Skill 1: passive state 30, passivestat1 = stat 119 (layer 0) with the
/// formula at offset 0 (2), passivestat2 = stat 120 with the formula at
/// offset 4 (7); passivestat3 none (−1).
fn inputs() -> ModelInputs {
    let mut none: Skills = blank();
    none.passivestat1 = 0xFFFF;
    let mut s = none.clone();
    s.passivestate = 30;
    s.aurastate = 0xFFFF;
    (s.passivestat1, s.passivecalc1) = (119, 0);
    (s.passivestat2, s.passivecalc2) = (120, 4);
    s.passivestat3 = 0xFFFF;
    let mut i = ModelInputs {
        skill_tables: Some(Arc::new(SkillTables {
            skills: vec![none, s],
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            // Constants only (0x07 n pushes n,
            // 0x00 ends): offset 0 → 2, offset 4 → 7.
            skills_code: vec![0x07, 2, 0x00, 0x00, 0x07, 7, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        })),
        ..ModelInputs::default()
    };
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            passivestate: 30,
            maxlvl: 20,
            ..SkillRow::default()
        },
    ];
    i
}

fn world(base: i32) -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut u = ClientUnit::new(P);
    u.skills = Some(SkillList {
        entries: vec![SkillEntry {
            skill: 1,
            base,
            owner: NATIVE,
            ..SkillEntry::default()
        }],
        ..SkillList::default()
    });
    w.units.insert(P, u);
    w
}

#[test]
fn refresh_fills_the_passive_state_list() {
    let i = inputs();
    let mut w = world(3);
    refresh(&mut w, &i, P, 1).unwrap();
    let l = &w.units[&P].state_lists[&30];
    assert_eq!(l.get(&(119, 0)), Some(&2));
    assert_eq!(l.get(&(120, 0)), Some(&7));
    assert_eq!(l.get(&(350, 0)), Some(&1));
    assert_eq!(l.get(&(351, 0)), Some(&3));
    // The totals see the attached list.
    assert_eq!(w.total(P, 120, 0), 7);
    assert_eq!(w.base(P, 120, 0), 0);
    // Stat 351 = L: unchanged (an edited value stays).
    w.units
        .get_mut(&P)
        .unwrap()
        .state_lists
        .get_mut(&30)
        .unwrap()
        .insert((120, 0), 99);
    refresh(&mut w, &i, P, 1).unwrap();
    assert_eq!(w.units[&P].state_lists[&30].get(&(120, 0)), Some(&99));
    // A skill without a passive state: nothing.
    refresh(&mut w, &i, P, 0).unwrap();
}

#[test]
fn refresh_frees_the_list_without_an_entry_or_with_level_0() {
    let i = inputs();
    let mut w = world(3);
    refresh(&mut w, &i, P, 1).unwrap();
    w.units
        .get_mut(&P)
        .unwrap()
        .skills
        .as_mut()
        .unwrap()
        .entries[0]
        .base = 0;
    refresh(&mut w, &i, P, 1).unwrap();
    assert!(!w.units[&P].state_lists.contains_key(&30));
    let mut w = world(3);
    refresh(&mut w, &i, P, 1).unwrap();
    w.units
        .get_mut(&P)
        .unwrap()
        .skills
        .as_mut()
        .unwrap()
        .entries
        .clear();
    refresh(&mut w, &i, P, 1).unwrap();
    assert!(!w.units[&P].state_lists.contains_key(&30));
    // Without the skills tables a passive refresh is an error.
    let mut w = world(3);
    assert!(refresh(&mut w, &ModelInputs::default(), P, 1).is_err());
}

#[test]
fn owed_effects_apply_in_order_and_refresh_all_needs_the_state() {
    let i = inputs();
    let mut w = world(2);
    apply(
        &mut w,
        &i,
        P,
        vec![SkillFx::StateOn(30), SkillFx::Refresh(1)],
    )
    .unwrap();
    assert!(w.units[&P].states.contains(&30));
    assert_eq!(w.units[&P].state_lists[&30].get(&(351, 0)), Some(&2));
    // Refreshed only while the state is on.
    w.units
        .get_mut(&P)
        .unwrap()
        .skills
        .as_mut()
        .unwrap()
        .entries[0]
        .base = 5;
    refresh_all(&mut w, &i, P).unwrap();
    assert_eq!(w.units[&P].state_lists[&30].get(&(351, 0)), Some(&5));
    apply(&mut w, &i, P, vec![SkillFx::StateOff(30)]).unwrap();
    w.units
        .get_mut(&P)
        .unwrap()
        .skills
        .as_mut()
        .unwrap()
        .entries[0]
        .base = 6;
    refresh_all(&mut w, &i, P).unwrap();
    assert_eq!(w.units[&P].state_lists[&30].get(&(351, 0)), Some(&5));
}

#[test]
fn op_one_adds_the_percent_of_the_target_total() {
    // Increased Stamina at slvl 20: stat 163 = 315 on its passive state
    // list, op 1 → maxstamina (11); ScnBar's base 30976 → 128550.
    let mut w = ClientWorld::default();
    let mut u = ClientUnit::new(P);
    u.stats.insert(11, 30976);
    u.state_lists
        .insert(30, [((163, 0), 315)].into_iter().collect());
    w.units.insert(P, u);
    w.stat_ops = vec![Vec::new(); 164];
    w.stat_ops[11].push((163, 1));
    assert_eq!(w.total(P, 11, 0), 128550);
    // A zero target total gains nothing.
    w.units.get_mut(&P).unwrap().stats.insert(11, 0);
    assert_eq!(w.total(P, 11, 0), 0);
}
