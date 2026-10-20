//! The client skill list (unit +0xA8) and the shared list operations
//! the skill messages run: add, assign, remove, select. Entry fields
//! and the level formula have a single owner.
//!
//! The passive-state parts of add, remove and refresh (2.2,
//! 4) act on the unit's state bits and state lists, not on the list: the
//! operations record them in [`SkillList::fx`] in call order and the
//! message handler applies them ([`super::passive::apply`]).

use super::world::{ClientWorld, SkillRow, UnitKey, MONSTER, PLAYER};

/// Owner GUID of a native entry (−1).
pub const NATIVE: u32 = u32::MAX;

/// The used skill entry of a unit (: its
/// list's current entry, the entry
/// state the do and start functions keep); none without a list or a
/// current entry.
pub fn used_entry(w: &ClientWorld, key: UnitKey) -> Option<&SkillEntry> {
    let l = w.units.get(&key)?.skills.as_ref()?;
    l.entries.get(l.current?)
}

/// [`used_entry`], mutable.
pub fn used_entry_mut(w: &mut ClientWorld, key: UnitKey) -> Option<&mut SkillEntry> {
    let l = w.units.get_mut(&key)?.skills.as_mut()?;
    let i = l.current?;
    l.entries.get_mut(i)
}

/// The unit's own (native) entry of `skill` (step 3): the entry a start function writes, kept across skill
/// switches.
pub fn skill_entry(w: &ClientWorld, key: UnitKey, skill: u16) -> Option<&SkillEntry> {
    let l = w.units.get(&key)?.skills.as_ref()?;
    l.entries.get(l.native(skill)?)
}

/// [`skill_entry`], mutable.
pub fn skill_entry_mut(w: &mut ClientWorld, key: UnitKey, skill: u16) -> Option<&mut SkillEntry> {
    let l = w.units.get_mut(&key)?.skills.as_mut()?;
    let i = l.native(skill)?;
    l.entries.get_mut(i)
}

/// One entry (0x40 bytes, zeroed at creation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillEntry {
    /// The skill id (via the record pointer +0x00).
    pub skill: u16,
    /// +0x08.
    pub mode: u32,
    /// +0x0C entry flags: bit 0 a moving
    /// skill's path step, bit 1 its path ended; Charge's start sets
    /// 0x1001.
    pub flags: u32,
    /// +0x18: the skill's working value (reads,
    /// writes): Dragon Talon's kicks left,
    /// written by its
    /// `cltstfunc` 6; Blade Fury's parameter 1, its missile delay;
    /// the Charge start's
    /// target unit type, 6 for none.
    pub param: i32,
    /// +0x1C parameter 2: the Charge
    /// target's GUID; Strafe and Fend / Zeal / Fury's next
    /// target's unit type.
    pub param2: i32,
    /// +0x20 the counter: Charge's count;
    /// Strafe and Fend / Zeal / Fury's next
    /// target's GUID.
    pub counter: i32,
    /// +0x24 parameter 3: 7 from the Charge start.
    pub param3: i32,
    /// +0x28 base level.
    pub base: i32,
    /// +0x2C level bonus (writers: open question 2).
    pub level_bonus: i32,
    /// +0x30 quantity.
    pub quantity: i32,
    /// +0x34 owner GUID; [`NATIVE`] = the unit's own skill.
    pub owner: u32,
    /// +0x38 charges.
    pub charges: i32,
    /// +0x3C has-charges flag.
    pub has_charges: bool,
}

/// The list: entries in list order (new ones appended at the
/// tail) and the left (+0x08), right (+0x0C) and current (+0x10) entry
/// references, as indices into `entries`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillList {
    pub entries: Vec<SkillEntry>,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub current: Option<usize>,
    /// What the list operations owe the unit (passive state bits and
    /// refreshes, 2.2, 4), in call order; the handler applies
    /// and drains them ([`super::passive::apply`]).
    pub fx: Vec<SkillFx>,
}

/// An effect of a list operation on its unit (2.2, 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillFx {
    /// The passive state on.
    StateOn(u8),
    /// The passive state off.
    StateOff(u8),
    /// The refresh with (unit, skill).
    Refresh(u16),
}

/// A skill-list operation the base game does not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillError {
    /// Select with a skill outside the table (fatal 0x668).
    #[error("fatal assert 0x668 (select: skill {0} outside the skills table)")]
    BadSkill(u16),
    /// Remove would leave a hand pointing at the freed entry (no entry of
    /// skill 0, or skill 0's own entry removed): a dangling pointer in
    /// the base game.
    #[error("skill list: a hand references the removed entry")]
    Dangling,
}

/// The unit facts the operations read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub unit_type: u8,
    pub class: u32,
}

impl SkillList {
    /// "The entry of (skill, owner)": the first
    /// in list order.
    pub fn find(&self, skill: u16, owner: u32) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.skill == skill && e.owner == owner)
    }

    /// The native entry of `skill`.
    pub fn native(&self, skill: u16) -> Option<usize> {
        self.find(skill, NATIVE)
    }

    pub fn left_entry(&self) -> Option<&SkillEntry> {
        self.entries.get(self.left?)
    }

    pub fn right_entry(&self) -> Option<&SkillEntry> {
        self.entries.get(self.right?)
    }
}

/// The skills record of `skill` when it is inside the table (count =
/// the rows).
fn row(rows: &[SkillRow], skill: u16) -> Option<&SkillRow> {
    rows.get(usize::from(skill))
}

/// `max_level(skill)`: `maxlvl` if > 0,
/// else 20.
pub fn max_level(r: &SkillRow) -> i32 {
    let m = i32::from(r.maxlvl as i16);
    if m > 0 {
        m
    } else {
        20
    }
}

/// The skill's passive state when it has one (`passivestate` > 0).
fn passive_state(rows: &[SkillRow], skill: u16) -> Option<u16> {
    row(rows, skill)
        .map(|r| r.passivestate)
        .filter(|&p| p as i16 > 0)
}

/// Refresh: owed to the unit when the skill has
/// a passive state (the state list lives on the unit,
/// [`super::passive::refresh`]).
pub fn refresh(list: &mut SkillList, rows: &[SkillRow], skill: u16) -> Result<(), SkillError> {
    if passive_state(rows, skill).is_some() {
        list.fx.push(SkillFx::Refresh(skill));
    }
    Ok(())
}

/// Add. Returns the entry, or none for a skill
/// outside the table. The refresh error (a passive skill) comes after
/// the list is updated.
pub fn add(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
) -> Result<Option<usize>, SkillError> {
    let Some(r) = row(rows, skill) else {
        return Ok(None);
    };
    let i = match list.native(skill) {
        Some(i) => {
            let e = &mut list.entries[i];
            if e.base < max_level(r) || unit.unit_type == MONSTER {
                e.base += 1;
            }
            i
        }
        None => {
            let mode = if unit.unit_type == MONSTER {
                u32::from(r.monanim)
            } else if unit.unit_type == PLAYER && unit.class == 6 && skill == 5 {
                0x10
            } else {
                u32::from(r.anim)
            };
            list.entries.push(SkillEntry {
                skill,
                mode,
                base: 1,
                owner: NATIVE,
                ..SkillEntry::default()
            });
            list.entries.len() - 1
        }
    };
    // The passive state on,
    // then the refresh.
    if let Some(p) = passive_state(rows, skill) {
        list.fx.push(SkillFx::StateOn(p as u8));
    }
    refresh(list, rows, skill)?;
    Ok(Some(i))
}

/// The native skills of a player, right after
/// its list is created: unless the native entry of skill 0 exists, a
/// player of a valid class (`class_skills` is its `charstats` Skill 1–10,
/// `None` for a class outside the table → nothing at all) gets skill 0
/// and each of the ten ids that is a skill (0 ≤ id < row count), in
/// order; then an empty left / right hand selects (0, native).
pub fn init_player(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    class_skills: Option<&[u16; 10]>,
) -> Result<(), SkillError> {
    if list.native(0).is_none() {
        let Some(class_skills) = class_skills else {
            return Ok(());
        };
        add(list, rows, unit, 0)?;
        for &id in class_skills {
            let id = id as i16;
            if id >= 0 && (id as usize) < rows.len() {
                add(list, rows, unit, id as u16)?;
            }
        }
    }
    if list.left.is_none() {
        select(list, rows, true, 0, NATIVE)?;
    }
    if list.right.is_none() {
        select(list, rows, false, 0, NATIVE)?;
    }
    Ok(())
}

/// Select (left) / (right).
pub fn select(
    list: &mut SkillList,
    rows: &[SkillRow],
    left: bool,
    skill: u16,
    owner: u32,
) -> Result<(), SkillError> {
    if row(rows, skill).is_none() {
        return Err(SkillError::BadSkill(skill));
    }
    if let Some(i) = list.find(skill, owner) {
        if left {
            list.left = Some(i);
        } else {
            list.right = Some(i);
        }
    }
    Ok(())
}

/// Remove with (unit, s, d) (5, 6). In order: the
/// passive state off; a left or right hand on the native entry of `s`
/// is selected back to (skill 0, native) (step 3: not found → the hand
/// stays); a current reference to it is cleared. Then the native entry:
/// none → refresh only; `d` false → unlinked and freed; `d` true → its
/// base −= 1 and it is unlinked and freed only when the base is now
/// below 1. A hand left on a freed entry is refused (step 6): the list
/// is then unchanged ([`SkillError::Dangling`]).
pub fn remove(
    list: &mut SkillList,
    rows: &[SkillRow],
    skill: u16,
    d: bool,
) -> Result<(), SkillError> {
    let Some(i) = list.native(skill) else {
        return refresh(list, rows, skill);
    };
    let mut w = list.clone();
    // The passive state off, owed to the unit.
    if let Some(p) = passive_state(rows, skill) {
        w.fx.push(SkillFx::StateOff(p as u8));
    }
    if w.left == Some(i) {
        select(&mut w, rows, true, 0, NATIVE)?;
    }
    if w.right == Some(i) {
        select(&mut w, rows, false, 0, NATIVE)?;
    }
    if w.current == Some(i) {
        w.current = None;
    }
    let free = if d {
        w.entries[i].base -= 1;
        w.entries[i].base < 1
    } else {
        true
    };
    if free {
        if w.left == Some(i) || w.right == Some(i) {
            return Err(SkillError::Dangling);
        }
        w.entries.remove(i);
        for j in [&mut w.left, &mut w.right, &mut w.current]
            .into_iter()
            .flatten()
        {
            if *j > i {
                *j -= 1;
            }
        }
    }
    *list = w;
    refresh(list, rows, skill)
}

/// Assign with (unit, skill, level, remove).
pub fn assign(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
    level: i32,
    remove_flag: bool,
) -> Result<(), SkillError> {
    if level != 0 {
        let e = match list.native(skill) {
            Some(i) => Some(i),
            None => add(list, rows, unit, skill)?,
        };
        if let Some(i) = e {
            list.entries[i].base = level;
        }
        return refresh(list, rows, skill);
    }
    if remove_flag {
        // with d = the remove flag; it refreshes.
        return remove(list, rows, skill, true);
    }
    let e = match list.native(skill) {
        Some(i) => Some(i),
        None => {
            add(list, rows, unit, skill)?;
            list.native(skill)
        }
    };
    let Some(i) = e else {
        return Ok(());
    };
    list.entries[i].base = 0;
    refresh(list, rows, skill)
}

/// The native entry after the add path of the level-bonus writers:
/// add, look the native entry up again, adding once more
/// when still none; a found entry gets base 0 and a refresh.
fn add_for_bonus(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
) -> (Option<usize>, Result<(), SkillError>) {
    let mut r = add(list, rows, unit, skill).map(|_| ());
    let mut e = list.native(skill);
    if e.is_none() {
        let r2 = add(list, rows, unit, skill).map(|_| ());
        r = r.and(r2);
        e = list.native(skill);
    }
    if let Some(i) = e {
        list.entries[i].base = 0;
        r = r.and(refresh(list, rows, skill));
    }
    (e, r)
}

/// Set with (unit, skill, v): the native entry's
/// level bonus := `v`. No entry and `v` ≤ 0 → nothing; no entry and
/// `v` > 0 → the add path ([`add_for_bonus`]). Then the bonus is
/// written and the skill refreshed.
pub fn set_bonus(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
    v: i32,
) -> Result<(), SkillError> {
    bonus_write(list, rows, unit, skill, v, |_, v| v, true)
}

/// Add with (unit, skill, v): as [`set_bonus`] with
/// bonus += `v`, a negative result clamped to 0, and no refresh after
/// the write.
pub fn add_bonus(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
    v: i32,
) -> Result<(), SkillError> {
    bonus_write(list, rows, unit, skill, v, |old, v| (old + v).max(0), false)
}

fn bonus_write(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
    v: i32,
    f: impl Fn(i32, i32) -> i32,
    refresh_after: bool,
) -> Result<(), SkillError> {
    let (e, mut r) = match list.native(skill) {
        Some(i) => (Some(i), Ok(())),
        None if v <= 0 => return Ok(()),
        None => add_for_bonus(list, rows, unit, skill),
    };
    if let Some(i) = e {
        list.entries[i].level_bonus = f(list.entries[i].level_bonus, v);
        if refresh_after {
            r = r.and(refresh(list, rows, skill));
        }
    }
    r
}

/// Split level: with M := `maxlvl` of the skill (≤ 0 or no
/// row → 20) and a level `l`: `l` > M → assign (skill, M, remove 0) then
/// set the bonus to `l` − M; else assign (skill, `l`, remove 0) and the
/// bonus keeps its old value.
pub fn split_level(
    list: &mut SkillList,
    rows: &[SkillRow],
    unit: Owner,
    skill: u16,
    l: i32,
) -> Result<(), SkillError> {
    let m = row(rows, skill).map_or(20, max_level);
    if l > m {
        let a = assign(list, rows, unit, skill, m, false);
        a.and(set_bonus(list, rows, unit, skill, l - m))
    } else {
        assign(list, rows, unit, skill, l, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Vec<SkillRow> {
        vec![SkillRow::default(); n]
    }

    const PLAYER0: Owner = Owner {
        unit_type: PLAYER,
        class: 0,
    };

    #[test]
    fn add_appends_then_raises_up_to_max_level() {
        let mut t = rows(4);
        t[2].anim = 7;
        t[2].maxlvl = 2;
        let mut l = SkillList::default();
        assert_eq!(add(&mut l, &t, PLAYER0, 2), Ok(Some(0)));
        assert_eq!(
            l.entries[0],
            SkillEntry {
                skill: 2,
                mode: 7,
                base: 1,
                owner: NATIVE,
                ..SkillEntry::default()
            }
        );
        add(&mut l, &t, PLAYER0, 2).unwrap();
        add(&mut l, &t, PLAYER0, 2).unwrap();
        assert_eq!(l.entries[0].base, 2, "a player stops at max_level");
        let monster = Owner {
            unit_type: MONSTER,
            class: 0,
        };
        let mut m = SkillList::default();
        t[2].monanim = 3;
        for _ in 0..3 {
            add(&mut m, &t, monster, 2).unwrap();
        }
        assert_eq!((m.entries[0].mode, m.entries[0].base), (3, 3));
        assert_eq!(add(&mut l, &t, PLAYER0, 4), Ok(None), "outside the table");
    }

    #[test]
    fn assassin_skill_5_has_mode_0x10() {
        let t = rows(6);
        let mut l = SkillList::default();
        let sin = Owner {
            unit_type: PLAYER,
            class: 6,
        };
        add(&mut l, &t, sin, 5).unwrap();
        assert_eq!(l.entries[0].mode, 0x10);
    }

    #[test]
    fn max_level_defaults_to_20() {
        let mut r = SkillRow::default();
        assert_eq!(max_level(&r), 20);
        r.maxlvl = 0xFFFF;
        assert_eq!(max_level(&r), 20);
        r.maxlvl = 3;
        assert_eq!(max_level(&r), 3);
    }

    #[test]
    fn assign_level_zero_creates_at_base_zero_or_removes() {
        let t = rows(40);
        let mut l = SkillList::default();
        assign(&mut l, &t, PLAYER0, 36, 0, false).unwrap();
        assert_eq!((l.entries[0].skill, l.entries[0].base), (36, 0));
        assign(&mut l, &t, PLAYER0, 36, 1, false).unwrap();
        assert_eq!(l.entries[0].base, 1);
        assign(&mut l, &t, PLAYER0, 36, 0, true).unwrap();
        assert!(l.entries.is_empty());
        // Outside the table: nothing.
        assign(&mut l, &t, PLAYER0, 40, 0, false).unwrap();
        assign(&mut l, &t, PLAYER0, 40, 3, false).unwrap();
        assert!(l.entries.is_empty());
    }

    #[test]
    fn remove_resets_a_hand_to_skill_0() {
        let t = rows(40);
        let mut l = SkillList::default();
        assign(&mut l, &t, PLAYER0, 0, 1, false).unwrap();
        assign(&mut l, &t, PLAYER0, 36, 1, false).unwrap();
        assign(&mut l, &t, PLAYER0, 37, 1, false).unwrap();
        select(&mut l, &t, false, 36, NATIVE).unwrap();
        select(&mut l, &t, true, 37, NATIVE).unwrap();
        l.current = Some(1);
        assign(&mut l, &t, PLAYER0, 36, 0, true).unwrap();
        assert_eq!(
            l.entries.iter().map(|e| e.skill).collect::<Vec<_>>(),
            [0, 37]
        );
        assert_eq!(l.right_entry().map(|e| e.skill), Some(0));
        assert_eq!(l.left_entry().map(|e| e.skill), Some(37));
        assert_eq!(l.current, None);
        // A hand on the removed skill 0 itself would dangle.
        select(&mut l, &t, false, 0, NATIVE).unwrap();
        assert_eq!(
            assign(&mut l, &t, PLAYER0, 0, 0, true),
            Err(SkillError::Dangling)
        );
    }

    #[test]
    fn select_needs_the_skill_in_the_table_and_an_entry() {
        let t = rows(3);
        let mut l = SkillList::default();
        assert_eq!(
            select(&mut l, &t, true, 3, NATIVE),
            Err(SkillError::BadSkill(3))
        );
        select(&mut l, &t, true, 2, NATIVE).unwrap();
        assert_eq!(l.left, None, "no entry: unchanged");
    }

    #[test]
    fn a_passive_skill_owes_its_state_and_refreshes() {
        let mut t = rows(3);
        t[1].passivestate = 5;
        let mut l = SkillList::default();
        assert_eq!(assign(&mut l, &t, PLAYER0, 1, 1, false), Ok(()));
        assert_eq!((l.entries.len(), l.entries[0].base), (1, 1));
        // Add: state on, refresh; then assign's own refresh.
        assert_eq!(
            l.fx,
            [
                SkillFx::StateOn(5),
                SkillFx::Refresh(1),
                SkillFx::Refresh(1)
            ]
        );
        l.fx.clear();
        assert_eq!(refresh(&mut l, &t, 2), Ok(()));
        assert!(l.fx.is_empty(), "no passive state: nothing owed");
        // Remove: state off, then assign's refresh (no entry: the list of
        // the state is freed by the refresh).
        assign(&mut l, &t, PLAYER0, 1, 0, true).unwrap();
        assert_eq!(l.fx, [SkillFx::StateOff(5), SkillFx::Refresh(1)]);
    }

    fn entry(skill: u16, base: i32, owner: u32) -> SkillEntry {
        SkillEntry {
            skill,
            base,
            owner,
            ..SkillEntry::default()
        }
    }

    #[test]
    fn the_entry_of_skill_and_owner_is_the_first_in_list_order() {
        let l = SkillList {
            entries: vec![entry(5, 1, 77), entry(5, 2, NATIVE), entry(5, 3, NATIVE)],
            ..SkillList::default()
        };
        assert_eq!(l.find(5, NATIVE), Some(1));
        assert_eq!(l.native(5), Some(1));
        assert_eq!(l.find(5, 77), Some(0));
        assert_eq!(l.find(5, 78), None);
        assert_eq!(l.find(6, NATIVE), None);
    }

    #[test]
    fn remove_with_d_decrements_and_frees_below_one() {
        let t = rows(40);
        let list = |hand: Option<usize>| SkillList {
            entries: vec![
                entry(0, 1, NATIVE),
                entry(7, 3, NATIVE),
                entry(8, 1, NATIVE),
            ],
            left: hand,
            right: hand,
            current: hand,
            fx: Vec::new(),
        };
        // d = 0: unlinked and freed whatever the base; hands reset to
        // skill 0, current cleared.
        let mut l = list(Some(1));
        remove(&mut l, &t, 7, false).unwrap();
        assert_eq!(
            l.entries.iter().map(|e| e.skill).collect::<Vec<_>>(),
            [0, 8]
        );
        assert_eq!((l.left, l.right, l.current), (Some(0), Some(0), None));
        // d != 0, base 3: base 2, nothing freed, hands still reset.
        let mut l = list(Some(1));
        remove(&mut l, &t, 7, true).unwrap();
        assert_eq!(l.entries.len(), 3);
        assert_eq!(l.entries[1].base, 2);
        assert_eq!((l.left, l.right, l.current), (Some(0), Some(0), None));
        // d != 0, base 1: base 0 < 1, freed (indices after it shift).
        let mut l = list(Some(2));
        remove(&mut l, &t, 8, true).unwrap();
        assert_eq!(l.entries.len(), 2);
        assert_eq!((l.left, l.right), (Some(0), Some(0)));
        // No native entry: nothing but the refresh (Ok for no passive).
        let mut l = list(None);
        remove(&mut l, &t, 9, true).unwrap();
        assert_eq!(l, list(None));
        // A passive state: the list is updated, and the state off then the
        // refresh are owed to the unit (`SkillFx`).
        let mut tp = rows(40);
        tp[7].passivestate = 3;
        let mut l = list(None);
        remove(&mut l, &tp, 7, true).unwrap();
        assert_eq!(l.entries[1].base, 2);
        assert_eq!(l.fx, [SkillFx::StateOff(3), SkillFx::Refresh(7)]);
        // No list entries at all: nothing.
        let mut e = SkillList::default();
        remove(&mut e, &t, 7, true).unwrap();
        assert!(e.entries.is_empty());
    }

    #[test]
    fn a_hand_left_on_a_freed_entry_is_refused_and_the_list_unchanged() {
        let t = rows(40);
        // (a) no native skill-0 entry: select (0, native) finds nothing.
        let l0 = SkillList {
            entries: vec![entry(7, 1, NATIVE), entry(8, 1, NATIVE)],
            left: Some(0),
            right: Some(1),
            current: Some(0),
            fx: Vec::new(),
        };
        let mut l = l0.clone();
        assert_eq!(remove(&mut l, &t, 7, false), Err(SkillError::Dangling));
        assert_eq!(l, l0, "refused: the model is unchanged");
        // (b) s = 0: the select finds the entry being removed.
        let l1 = SkillList {
            entries: vec![entry(0, 1, NATIVE), entry(8, 1, NATIVE)],
            left: Some(0),
            right: Some(1),
            current: None,
            fx: Vec::new(),
        };
        let mut l = l1.clone();
        assert_eq!(remove(&mut l, &t, 0, false), Err(SkillError::Dangling));
        assert_eq!(l, l1);
        // With d != 0 and the base still >= 1 after the decrement nothing
        // is freed and the hand stays valid.
        let mut l2 = l1.clone();
        l2.entries[0].base = 2;
        remove(&mut l2, &t, 0, true).unwrap();
        assert_eq!(
            (l2.entries.len(), l2.entries[0].base, l2.left),
            (2, 1, Some(0))
        );
    }

    #[test]
    fn level_bonus_set_add_and_split() {
        let mut t = rows(6);
        t[2].maxlvl = 10;
        let mut l = SkillList::default();
        // Set with no entry and v <= 0: nothing.
        set_bonus(&mut l, &t, PLAYER0, 2, 0).unwrap();
        assert!(l.entries.is_empty());
        // v > 0 adds the entry, base := 0, bonus := v.
        set_bonus(&mut l, &t, PLAYER0, 2, 3).unwrap();
        assert_eq!((l.entries[0].base, l.entries[0].level_bonus), (0, 3));
        // Set overwrites; add accumulates and clamps at 0.
        set_bonus(&mut l, &t, PLAYER0, 2, 1).unwrap();
        assert_eq!(l.entries[0].level_bonus, 1);
        add_bonus(&mut l, &t, PLAYER0, 2, 4).unwrap();
        assert_eq!(l.entries[0].level_bonus, 5);
        add_bonus(&mut l, &t, PLAYER0, 2, -9).unwrap();
        assert_eq!(l.entries[0].level_bonus, 0);
        // Add with no entry and v <= 0: nothing; v > 0: the entry with
        // base 0 and bonus v.
        add_bonus(&mut l, &t, PLAYER0, 3, -2).unwrap();
        assert!(l.native(3).is_none());
        add_bonus(&mut l, &t, PLAYER0, 3, 2).unwrap();
        let e3 = l.entries[l.native(3).unwrap()];
        assert_eq!((e3.base, e3.level_bonus), (0, 2));
        // Split level: above maxlvl (10) the excess is the bonus.
        let mut l = SkillList::default();
        split_level(&mut l, &t, PLAYER0, 2, 14).unwrap();
        assert_eq!((l.entries[0].base, l.entries[0].level_bonus), (10, 4));
        // At or below: assign only, the old bonus is kept.
        split_level(&mut l, &t, PLAYER0, 2, 7).unwrap();
        assert_eq!((l.entries[0].base, l.entries[0].level_bonus), (7, 4));
        // maxlvl <= 0 means 20; a skill with no row: 20.
        let mut l = SkillList::default();
        split_level(&mut l, &t, PLAYER0, 4, 25).unwrap();
        assert_eq!((l.entries[0].base, l.entries[0].level_bonus), (20, 5));
    }
}
