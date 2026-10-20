//! Tests of the skill stat callback handlers and the skill reset on
//! [`BodyFake`].

use super::stat_cb::*;
use super::use_::bodies::fake::BodyFake;
use super::use_::bodies::tests2::{body_rec, tabs, Code};
use super::use_::bodies::BodyWorld;
use super::{SkillEntry, SkillTables, SkillUnits};
use crate::skills::fake::FUnit;
use crate::units::UnitType;

impl StatCbWorld for BodyFake {
    fn add_entry(&mut self, u: usize, skill: i32) {
        self.c.log.push(format!("add_entry {u} {skill}"));
        self.c.units[u].skills.push(SkillEntry {
            skill,
            base: 1,
            owner_guid: -1,
            ..SkillEntry::default()
        });
    }
    fn assign_entry(&mut self, u: usize, skill: i32, level: i32, remove: bool) {
        self.c
            .log
            .push(format!("assign {u} {skill} {level} {remove}"));
        let l = &mut self.c.units[u].skills;
        if remove {
            if let Some(i) = l.iter().position(|e| e.skill == skill && e.is_native()) {
                l.remove(i);
            }
        } else if let Some(e) = l.iter_mut().find(|e| e.skill == skill && e.is_native()) {
            e.base = level;
        }
    }
    fn remove_skill(&mut self, u: usize, skill: i32) {
        self.c.log.push(format!("remove_skill {u} {skill}"));
        self.assign_entry(u, skill, 0, true);
    }
    fn send_skill_msg(&mut self, to: usize, skill: i32, base: i32, remove: bool) {
        self.c
            .log
            .push(format!("msg21 {to} {skill} {base} {remove}"));
    }
    fn set_pet_max(&mut self, u: usize, pet_type: i32, max: i32) {
        self.c.log.push(format!("petmax {u} {pet_type} {max}"));
    }
    fn pettype_skills(&self, pet_type: i32) -> Vec<i32> {
        self.pet_skills.get(&pet_type).cloned().unwrap_or_default()
    }
    fn is_hireling(&self, u: usize) -> bool {
        self.c.units[u].hireling
    }
    fn draw_identity(&self, u: usize) -> (UnitType, i32) {
        (self.c.units[u].kind, self.c.units[u].class)
    }
    fn select_attack(&mut self, u: usize, left: bool) {
        self.c.log.push(format!("attack {u} {left}"));
        self.sel.insert(
            (u, left),
            SkillEntry {
                skill: 0,
                base: 1,
                owner_guid: -1,
                ..SkillEntry::default()
            },
        );
    }
    fn selected_skill(&self, u: usize, left: bool) -> Option<SkillEntry> {
        self.sel.get(&(u, left)).copied()
    }
    fn entry_update(&mut self, u: usize, skill: i32, owner: i32, base: i32, ch: i32) -> bool {
        let l = &mut self.c.units[u].skills;
        match l
            .iter_mut()
            .find(|e| e.skill == skill && e.owner_guid == owner)
        {
            Some(e) => {
                e.base = base;
                e.charges = ch;
                e.has_charges = true;
                true
            }
            None => false,
        }
    }
    fn entry_append(&mut self, u: usize, e: SkillEntry, mode: u32) {
        self.c
            .log
            .push(format!("append {u} {} mode {mode}", e.skill));
        self.c.units[u].skills.push(e);
    }
    fn entry_unlink(&mut self, u: usize, skill: i32, owner: i32) {
        let l = &mut self.c.units[u].skills;
        if let Some(i) = l
            .iter()
            .position(|e| e.skill == skill && e.owner_guid == owner)
        {
            l.remove(i);
        }
    }
    fn item_charges(&self, item: usize, layer: u16) -> i32 {
        self.c.units[item]
            .stats
            .get(&(204, layer))
            .copied()
            .unwrap_or(0)
    }
    fn item_attached(&self, item: usize, _: usize) -> bool {
        self.c.units[item].flags & 1 != 0
    }
    fn client_skill_reset_updates(&mut self, u: usize) {
        self.c.log.push(format!("clientupdates {u}"));
    }
}

fn attack() -> SkillEntry {
    SkillEntry {
        skill: 0,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

/// Skills 0 (blank) and 1…: class 2 skills with `n` more rows.
fn tables(rows: Vec<d2_data::tables::Skills>, code: Code) -> SkillTables {
    let mut t = tabs(rows[0].clone(), code, 1);
    t.skills = std::iter::once(body_rec()).chain(rows).collect();
    t
}

fn player(f: &mut BodyFake, class: i32) -> usize {
    let mut p = FUnit::new(UnitType::Player, class);
    p.guid = 100;
    f.add(p, (0, 0))
}

fn native_of(f: &BodyFake, u: usize, s: i32) -> Option<SkillEntry> {
    f.c.units[u].skills.iter().find(|e| e.skill == s).copied()
}

#[test]
fn oskill_gates() {
    let mut r = body_rec();
    r.charclass = 3;
    let t = tables(vec![r], Code::new());
    let (mut f, _) = (BodyFake::new(), ());
    let p = player(&mut f, 2);
    // Skill 0: nothing, whichever stat.
    oskill(&mut f, &t, p, 97, 0, 1);
    // Stat 107: the player's class must equal the skill's charclass.
    oskill(&mut f, &t, p, 107, 1, 1);
    assert!(f.c.units[p].skills.is_empty());
    // A monster never passes the 107 gate.
    let m = f.add(FUnit::new(UnitType::Monster, 3), (0, 0));
    oskill(&mut f, &t, m, 107, 1, 1);
    assert!(f.c.units[m].skills.is_empty());
    // Stat 97 has no class test.
    oskill(&mut f, &t, p, 97, 1, 1);
    assert_eq!(native_of(&f, p, 1).map(|e| e.base), Some(0));
}

#[test]
fn oskill_creates_the_native_entry_at_base_zero() {
    let t = tables(vec![body_rec()], Code::new());
    let mut f = BodyFake::new();
    let p = player(&mut f, 0);
    oskill(&mut f, &t, p, 97, 1, 3);
    // add (base 1), assign base 0, message 0x21 (level 0, remove 0),
    // refresh, pet maximum; new > 0 → done and the entry stays.
    assert_eq!(native_of(&f, p, 1).map(|e| e.base), Some(0));
    let log = f.c.log.clone();
    let pos = |k: &str| log.iter().position(|l| l.starts_with(k)).unwrap();
    assert!(pos("add_entry") < pos("assign"));
    assert!(pos("assign") < pos("msg21 0 1 0 false"));
    // A monster of a player's minion sends to the owner's client.
    let m = f.add(FUnit::new(UnitType::Monster, 3), (0, 0));
    f.minion_owner.insert(m, p);
    oskill(&mut f, &t, m, 97, 1, 3);
    assert!(f.c.log.iter().any(|l| l == &format!("msg21 {p} 1 0 false")));
    // A monster without an owner: no message; an item unit: none either.
    let before = f.c.log.len();
    let m2 = f.add(FUnit::new(UnitType::Monster, 3), (0, 0));
    oskill(&mut f, &t, m2, 97, 1, 3);
    assert!(!f.c.log[before..].iter().any(|l| l.starts_with("msg21")));
}

#[test]
fn oskill_removal_and_attack_fallback() {
    let t = tables(vec![body_rec()], Code::new());
    let mut f = BodyFake::new();
    let p = player(&mut f, 0);
    oskill(&mut f, &t, p, 97, 1, 3);
    let e = native_of(&f, p, 1).unwrap();
    f.sel.insert((p, true), e);
    f.sel.insert((p, false), e);
    // Value drops to 0 with base 0: removed, left and right back to Attack.
    oskill(&mut f, &t, p, 97, 1, 0);
    assert!(native_of(&f, p, 1).is_none());
    assert_eq!(f.sel[&(p, true)], attack());
    assert_eq!(f.sel[&(p, false)], attack());
    assert!(f.c.log.iter().any(|l| l == &format!("assign {p} 1 0 true")));
    // Hard points: the entry stays and the value 0 changes nothing.
    oskill(&mut f, &t, p, 97, 1, 3);
    f.c.units[p].skills[0].base = 4;
    f.sel.insert((p, true), f.c.units[p].skills[0]);
    let n = f.c.log.len();
    oskill(&mut f, &t, p, 97, 1, 0);
    assert!(native_of(&f, p, 1).is_some());
    assert!(!f.c.log[n..].iter().any(|l| l.starts_with("attack")));
    // Edge case 7: with base 0 the entry is removed even when another
    // bonus still gives it a level.
    f.c.units[p].skills[0].base = 0;
    f.c.units[p].stats.insert((127, 0), 2);
    oskill(&mut f, &t, p, 97, 1, 0);
    assert!(native_of(&f, p, 1).is_none());
}

#[test]
fn class_bonus_refreshes_only_for_the_class() {
    let mut r = body_rec();
    r.passivestate = 7;
    let t = tables(vec![r], Code::new());
    let mut f = BodyFake::new();
    let p = player(&mut f, 2);
    f.c.units[p].skills.push(SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    });
    f.c.units[p].skills.push(attack());
    class_bonus(&mut f, &t, p, 3);
    assert!(!f.has_state(p, 7));
    // Layer >> 3 for the tab stat is the caller's: class 2 matches.
    class_bonus(&mut f, &t, p, 2);
    assert!(f.has_state(p, 7));
    // A hireling refreshes for any class, a draw-identity player too.
    let h = f.add(FUnit::new(UnitType::Monster, 9), (0, 0));
    f.c.units[h].hireling = true;
    f.c.units[h].skills.push(SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    });
    class_bonus(&mut f, &t, h, 5);
    assert!(f.has_state(h, 7));
    // Stats 126 / 127 / 83 / 188 through the dispatcher.
    let q = player(&mut f, 1);
    let e1 = native_of(&f, p, 1).unwrap();
    f.c.units[q].skills.push(e1);
    skill_stat_callback(&mut f, &t, q, None, 0, 188, 8, 1, (0, 0));
    assert!(f.has_state(q, 7));
}

#[test]
fn item_states_clear_the_group() {
    let mut f = BodyFake::new();
    f.state_groups.insert(5, 9);
    f.state_groups.insert(6, 9);
    let p = player(&mut f, 0);
    // New ≠ 0, state 6 of the same group on: cleared keeping 5, then 5 on.
    f.state_on(p, 6, true);
    item_state(&mut f, p, 5, 1);
    assert!(f.has_state(p, 5));
    assert!(!f.has_state(p, 6));
    assert!(f.c.log.iter().any(|l| l == &format!("changed {p} 5")));
    // Already on: nothing (no second mark).
    let n = f.c.log.len();
    item_state(&mut f, p, 5, 1);
    assert_eq!(f.c.log.len(), n);
    // New = 0: off including itself; a lacking state does nothing.
    item_state(&mut f, p, 5, 0);
    assert!(!f.has_state(p, 5));
    let n = f.c.log.len();
    item_state(&mut f, p, 5, 0);
    assert_eq!(f.c.log.len(), n);
    // Out of range: nothing.
    item_state(&mut f, p, 9999, 1);
    assert_eq!(f.c.log.len(), n);
}

#[test]
fn pet_max_takes_the_best_skill_of_the_pettype() {
    let mut code = Code::new();
    let c3 = code.f(3);
    let mut a = body_rec();
    a.pettype = 2;
    a.petmax = c3;
    let mut b = body_rec();
    b.pettype = 2;
    b.petmax = u32::MAX; // formula missing: 0, floored to 1 at level > 0
    let t = tables(vec![a, b], code);
    let mut f = BodyFake::new();
    f.pet_skills.insert(2, vec![1, 2]);
    let p = player(&mut f, 0);
    let ent = |s| SkillEntry {
        skill: s,
        base: 4,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    f.c.units[p].skills.push(ent(2));
    pet_max(&mut f, &t, p, 1);
    assert!(f.c.log.iter().any(|l| l == &format!("petmax {p} 2 1")));
    f.c.units[p].skills.push(ent(1));
    pet_max(&mut f, &t, p, 1);
    assert!(f.c.log.iter().any(|l| l == &format!("petmax {p} 2 3")));
    // No level on any: 0 (removes the pets).
    f.c.units[p].skills.clear();
    pet_max(&mut f, &t, p, 1);
    assert!(f.c.log.iter().any(|l| l == &format!("petmax {p} 2 0")));
    // Pet type 0, and non-players: nothing.
    let n = f.c.log.len();
    pet_max(&mut f, &t, p, 0);
    let m = f.add(FUnit::new(UnitType::Monster, 1), (0, 0));
    pet_max(&mut f, &t, m, 1);
    assert_eq!(f.c.log.len(), n);
    // The list builder: id order, pettype in range, at most 15.
    let mut rows = Vec::new();
    for i in 0..20 {
        let mut r = body_rec();
        r.pettype = if i == 19 { 99 } else { 4 };
        rows.push(r);
    }
    let t = tables(rows, Code::new());
    let lists = pettype_skill_lists(&t, 10);
    assert_eq!(lists[4].len(), 15);
    assert_eq!(lists[4][..3], [1, 2, 3]);
    assert!(lists[0].contains(&0));
}

#[test]
fn item_auras_schedule_and_cancel() {
    let mut code = Code::new();
    let c10 = code.f(10);
    let mut a = body_rec();
    a.aura = true;
    a.aurastate = 20;
    a.perdelay = c10;
    let mut b = a.clone();
    b.immediate = true;
    let mut no_state = a.clone();
    no_state.aurastate = 0xFFFF;
    let mut no_aura = a.clone();
    no_aura.aura = false;
    let t = tables(vec![a, b, no_state, no_aura], code);
    let mut f = BodyFake::new();
    let p = player(&mut f, 0);
    f.c.frame = 23;
    aura_on(&mut f, &t, p, 77, 1, 2);
    // Cancel own timers (arg G), then schedule type 9 at the period:
    // ((23 + 10 − 1) / 10) × 10 + 1 = 31.
    let lg = f.c.log.to_vec();
    assert_eq!(lg[0], format!("deltimers {p} 9 77"));
    assert_eq!(lg[1], format!("schedule {p} 9 31 77 1"));
    // Not immediate: no do core (no further effect lines).
    let n = lg.len();
    // G = 0 cancels all type-9 timers (the seam's arg 0).
    aura_on(&mut f, &t, p, 0, 1, 2);
    assert_eq!(f.c.log[n], format!("deltimers {p} 9 0"));
    // Bad skills: nothing at all.
    let n = f.c.log.len();
    for s in [3, 4, 99, -1] {
        aura_on(&mut f, &t, p, 1, s, 1);
        aura_off(&mut f, &t, p, 1, s);
    }
    assert_eq!(f.c.log.len(), n);
    // Off: state off, its list freed, own timers cancelled.
    let l = f.alloc_list(2, 0, Some(p)).unwrap();
    f.set_list_state(l, 20);
    f.lists[l].unit = Some(p);
    f.state_on(p, 20, true);
    aura_off(&mut f, &t, p, 77, 1);
    assert!(!f.has_state(p, 20));
    assert!(f.lists[l].freed);
    assert_eq!(f.c.log.last().unwrap(), &format!("deltimers {p} 9 77"));
    // Dispatcher: new ≠ 0 on, = 0 off.
    let n = f.c.log.len();
    skill_stat_callback(&mut f, &t, p, None, 5, 151, 1, 0, (0, 0));
    assert!(f.c.log[n..]
        .iter()
        .any(|l| l == &format!("deltimers {p} 9 5")));
}

#[test]
fn charged_skills_set_and_remove_entries() {
    let mut a = body_rec();
    a.anim = 8; // A2
    let mut b = body_rec();
    b.anim = 3; // not in the table: SC
    let t = tables(vec![a, b], Code::new());
    let mut f = BodyFake::new();
    let p = player(&mut f, 0);
    let it = f.add(FUnit::new(UnitType::Item, 1), (0, 0));
    f.c.units[it].flags = 1; // attached to the unit
                             // layer = skill << 6 | level (shift 6, mask 63); charges in the stat.
    let layer = (1u16 << 6) | 5;
    f.c.units[it].stats.insert((204, layer), 0x0103);
    skill_stat_callback(&mut f, &t, p, Some(it), 40, 204, layer, 0, (6, 63));
    // c = total & 0xFF = 3, new entry: base 5, charges 3, owner 40, mode A2.
    let e = native_of(&f, p, 1).unwrap();
    assert_eq!(
        (e.base, e.charges, e.owner_guid, e.has_charges),
        (5, 3, 40, true)
    );
    assert!(f.c.log.iter().any(|l| l.ends_with("mode 8")));
    // The level is not compared: a later set updates the same entry.
    let layer2 = (1u16 << 6) | 7;
    f.c.units[it].stats.insert((204, layer2), 2);
    skill_stat_callback(&mut f, &t, p, Some(it), 40, 204, layer2, 0, (6, 63));
    assert_eq!(f.c.units[p].skills.len(), 1);
    assert_eq!(f.c.units[p].skills[0].base, 7);
    // Mode default SC for another anim.
    let layer3 = (2u16 << 6) | 1;
    f.c.units[it].stats.insert((204, layer3), 1);
    skill_stat_callback(&mut f, &t, p, Some(it), 40, 204, layer3, 0, (6, 63));
    assert!(f.c.log.iter().any(|l| l.ends_with("mode 10")));
    // Remove (charges 0): selected skill with that owner → Attack; the
    // current skill with the skill → none; entry unlinked.
    let e = native_of(&f, p, 1).unwrap();
    f.sel.insert((p, true), e);
    f.c.units[p].used = Some(e);
    f.c.units[it].stats.insert((204, layer2), 0);
    skill_stat_callback(&mut f, &t, p, Some(it), 40, 204, layer2, 0, (6, 63));
    assert!(native_of(&f, p, 1).is_none());
    assert_eq!(f.sel[&(p, true)], attack());
    assert!(f.c.units[p].used.is_none());
    // No propagation item, or a monster: nothing.
    let n = f.c.units[p].skills.len();
    skill_stat_callback(&mut f, &t, p, None, 40, 204, layer3, 0, (6, 63));
    let m = f.add(FUnit::new(UnitType::Monster, 1), (0, 0));
    skill_stat_callback(&mut f, &t, m, Some(it), 40, 204, layer3, 0, (6, 63));
    assert_eq!(f.c.units[p].skills.len(), n);
    assert!(f.c.units[m].skills.is_empty());
    // Level 0: nothing.
    let layer0 = 2u16 << 6;
    f.c.units[it].stats.insert((204, layer0), 4);
    skill_stat_callback(&mut f, &t, p, Some(it), 40, 204, layer0, 0, (6, 63));
    assert_eq!(f.c.units[p].skills.len(), n);
}

#[test]
fn skill_reset_refunds_hard_points() {
    let t = tables(vec![body_rec(), body_rec(), body_rec()], Code::new());
    let mut f = BodyFake::new();
    let p = player(&mut f, 0);
    let ent = |s, base, owner| SkillEntry {
        skill: s,
        base,
        owner_guid: owner,
        ..SkillEntry::default()
    };
    f.c.units[p].skills = vec![ent(1, 5, -1), ent(2, 3, -1), ent(2, 4, 99), ent(3, 6, -1)];
    f.c.units[p].stats.insert((5, 0), 2);
    // The class list is 1, 2 (skill 3 is outside it; the item entry of 2
    // is untouched; skill 0 has no entry).
    reset_skills(&mut f, &t, p, &[0, 1, 2]);
    assert_eq!(f.c.get(p, 5), 2 + 5 + 3);
    let left: Vec<_> = f.c.units[p]
        .skills
        .iter()
        .map(|e| (e.skill, e.owner_guid))
        .collect();
    assert_eq!(left, vec![(2, 99), (3, -1)]);
    let lg = f.c.log.clone();
    assert!(lg.iter().any(|l| l == &format!("msg21 {p} 1 0 true")));
    assert!(lg.iter().any(|l| l == &format!("msg21 {p} 2 0 true")));
    assert!(!lg.iter().any(|l| l == &format!("msg21 {p} 0 0 true")));
    assert_eq!(lg.last().unwrap(), &format!("clientupdates {p}"));
    // A non-player: nothing.
    let m = f.add(FUnit::new(UnitType::Monster, 1), (0, 0));
    f.c.units[m].skills = vec![ent(1, 5, -1)];
    reset_skills(&mut f, &t, m, &[1]);
    assert_eq!(f.c.units[m].skills.len(), 1);
}
