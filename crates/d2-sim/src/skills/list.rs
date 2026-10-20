//! The server's skill list of a unit (unit +0xA8) and the shared list
//! operations ("the same code runs on the
//! server"): add, the set (assign with a
//! non-zero level), select
//! and the native skills of a player (run by the
//! server player init).
//!
//! The passive-state parts act on the unit's states and
//! state lists, not on the list: the operations record them in
//! [`SkillList::fx`] in call order for the unit's owner to apply.
//!
//! Entry fields and the level formula: (single
//! owner); [`SkillList::view`] gives the entries in that module's form.

use d2_data::tables::Skills;

use super::{max_level, SkillEntry};

/// Owner GUID of a native entry (−1).
pub const NATIVE: i32 = -1;

/// One entry (0x40 bytes, zeroed at creation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListEntry {
    /// The skill id (via the record pointer +0x00).
    pub skill: u16,
    /// +0x08.
    pub mode: u32,
    /// +0x0C entry flags (the Leap / Whirlwind phases).
    pub flags: u32,
    /// +0x18, +0x1C, +0x20, +0x24: params 1 to 4 (the bodies' param
    /// set and get).
    pub params: [i32; 4],
    /// +0x28 base level.
    pub base: i32,
    /// +0x2C level bonus.
    pub level_bonus: i32,
    /// +0x30 quantity.
    pub quantity: i32,
    /// +0x34 owner GUID; [`NATIVE`] = the unit's own skill.
    pub owner: i32,
    /// +0x38 charges.
    pub charges: i32,
    /// +0x3C has-charges flag.
    pub has_charges: bool,
}

/// What a list operation owes its unit, in call order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListFx {
    /// The passive state on.
    StateOn(u16),
    /// The passive-state stat list refresh.
    Refresh(u16),
}

/// The unit facts the operations read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListOwner {
    /// Unit type 1 (the add's `monanim` mode, no `max_level` cap).
    pub monster: bool,
    /// Unit +0x04 (player class 6 with skill 5: mode 0x10).
    pub class: i32,
}

impl ListOwner {
    /// A player of class `class`.
    pub fn player(class: i32) -> Self {
        Self {
            monster: false,
            class,
        }
    }
}

/// Select with a skill outside the `skills` table (fatal 0x668,
/// step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("fatal assert 0x668 (select: skill {0} outside the skills table)")]
pub struct BadSkill(pub i32);

/// The list: entries in list order (new ones appended at the
/// tail) and the left (+0x08), right (+0x0C) and current (+0x10) entry
/// references, as indices into `entries`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillList {
    pub entries: Vec<ListEntry>,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub current: Option<usize>,
    /// The mouse skills of the weapon set not in hand: player data
    /// +0x84 / +0x8C (left) and +0x80 / +0x88 (right) as (skill id, owner
    /// GUID) (saved in the character file; read by the weapon switch).
    /// `None`: unset, which the switch reads as (0, −1).
    pub swap_left: Option<(i32, i32)>,
    pub swap_right: Option<(i32, i32)>,
    /// The weapon switch state (header +0x10 bit 0, client +0x45C): the
    /// swap set is in hand.
    pub weapon_switch: bool,
    /// Effects owed to the unit, in call order; the owner drains them.
    pub fx: Vec<ListFx>,
}

fn row(rows: &[Skills], skill: i32) -> Option<&Skills> {
    usize::try_from(skill).ok().and_then(|i| rows.get(i))
}

/// The skill's passive state when > 0 (`passivestate`
/// +0x94, i16).
fn passive_state(r: &Skills) -> Option<u16> {
    let p = r.passivestate as i16;
    (p > 0).then_some(p as u16)
}

impl SkillList {
    /// The mouse pair of an entry reference: (skill id, owner); none →
    /// (0, −1).
    pub fn pair(&self, i: Option<usize>) -> (i32, i32) {
        i.and_then(|i| self.entries.get(i))
            .map_or((0, -1), |e| (i32::from(e.skill), e.owner))
    }

    /// Step 2 of the weapon switch (the inventory move, then the hand
    /// reads):
    /// returns the saved swap pairs (left, right; unset → (0, −1)), saves
    /// the current left and right mouse skills in their place, and flips
    /// the switch state (step 9, the header +0x10 bit 0 of the save).
    pub fn trade_swap_pairs(&mut self) -> [(i32, i32); 2] {
        let saved = [
            self.swap_left.unwrap_or((0, -1)),
            self.swap_right.unwrap_or((0, -1)),
        ];
        self.swap_left = Some(self.pair(self.left));
        self.swap_right = Some(self.pair(self.right));
        self.weapon_switch = !self.weapon_switch;
        saved
    }

    /// "The entry of (skill, owner)": the
    /// first in list order.
    pub fn find(&self, skill: i32, owner: i32) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| i32::from(e.skill) == skill && e.owner == owner)
    }

    /// The native entry (owner −1) of `skill`.
    pub fn native(&self, skill: i32) -> Option<usize> {
        self.find(skill, NATIVE)
    }

    /// The unit has an entry of `skill` (as
    /// reads it: "a skill 90 entry"), any
    /// owner.
    pub fn has(&self, skill: i32) -> bool {
        self.entries.iter().any(|e| i32::from(e.skill) == skill)
    }

    /// The entries in `skills::levels` form, list order.
    pub fn view(&self) -> Vec<SkillEntry> {
        self.entries
            .iter()
            .map(|e| SkillEntry {
                skill: i32::from(e.skill),
                base: e.base,
                level_bonus: e.level_bonus,
                owner_guid: e.owner,
                charges: e.charges,
                has_charges: e.has_charges,
            })
            .collect()
    }

    /// Refresh: owed to the unit when the skill
    /// has a passive state (the state list lives on the unit).
    fn refresh(&mut self, rows: &[Skills], skill: i32) {
        if let Some(r) = row(rows, skill) {
            if passive_state(r).is_some() {
                self.fx.push(ListFx::Refresh(skill as u16));
            }
        }
    }

    /// Add: none for a skill
    /// outside the table; else the passive state on, then the native
    /// entry's base += 1 (below `max_level`, or any monster), or a new
    /// native entry of base 1 appended; refresh; the entry.
    pub fn add(&mut self, rows: &[Skills], unit: ListOwner, skill: i32) -> Option<usize> {
        let r = row(rows, skill)?;
        if let Some(p) = passive_state(r) {
            self.fx.push(ListFx::StateOn(p));
        }
        let i = match self.native(skill) {
            Some(i) => {
                let e = &mut self.entries[i];
                if e.base < max_level(r) || unit.monster {
                    e.base += 1;
                }
                i
            }
            None => {
                let mode = if unit.monster {
                    u32::from(r.monanim)
                } else if unit.class == 6 && skill == 5 {
                    0x10
                } else {
                    u32::from(r.anim)
                };
                self.entries.push(ListEntry {
                    skill: skill as u16,
                    mode,
                    base: 1,
                    owner: NATIVE,
                    ..ListEntry::default()
                });
                self.entries.len() - 1
            }
        };
        self.refresh(rows, skill);
        Some(i)
    }

    /// The set `(unit, skill, level, 1)` with a level ≠ 0 (the assign):
    /// the native entry, else added; base := `level`; refresh.
    ///
    /// TODO: the passive refresh of the unit and, for a player, the
    /// player refresh that follow have no provider; not run here.
    pub fn set_base(&mut self, rows: &[Skills], unit: ListOwner, skill: i32, level: u8) {
        let i = match self.native(skill) {
            Some(i) => Some(i),
            None => self.add(rows, unit, skill),
        };
        if let Some(i) = i {
            self.entries[i].base = i32::from(level);
        }
        self.refresh(rows, skill);
    }

    /// Select (left) / (right):
    /// the entry of (skill, owner) found → that hand := it; not found →
    /// unchanged.
    pub fn select(
        &mut self,
        rows: &[Skills],
        left: bool,
        skill: i32,
        owner: i32,
    ) -> Result<(), BadSkill> {
        if row(rows, skill).is_none() {
            return Err(BadSkill(skill));
        }
        if let Some(i) = self.find(skill, owner) {
            if left {
                self.left = Some(i);
            } else {
                self.right = Some(i);
            }
        }
        Ok(())
    }

    /// The native skills of a player, right
    /// after its list is created: unless the native entry of skill 0
    /// exists, a player of a valid class (`class_skills` = its
    /// `charstats` `Skill 1`…`Skill 10`, i16 at +0xAE; `None` for a class
    /// outside the table → nothing at all, no select) gets skill 0 and
    /// each of the ten ids with 0 ≤ id < skill count, in order; then an
    /// empty left / right hand selects (0, −1).
    pub fn init_player(
        &mut self,
        rows: &[Skills],
        unit: ListOwner,
        class_skills: Option<&[u16; 10]>,
    ) -> Result<(), BadSkill> {
        if self.native(0).is_none() {
            let Some(class_skills) = class_skills else {
                return Ok(());
            };
            self.add(rows, unit, 0);
            for &id in class_skills {
                let id = i32::from(id as i16);
                if id >= 0 && (id as usize) < rows.len() {
                    self.add(rows, unit, id);
                }
            }
        }
        if self.left.is_none() {
            self.select(rows, true, 0, NATIVE)?;
        }
        if self.right.is_none() {
            self.select(rows, false, 0, NATIVE)?;
        }
        Ok(())
    }

    /// Remove the entry at `i` (assign
    /// with remove 1): the hands that referenced it were re-pointed
    /// first; the indices of later entries shift down. A hand still on
    /// the entry is cleared (D2RUST: the base game frees it, the client
    /// refuses that case; callers select Attack first).
    pub fn remove(&mut self, i: usize) {
        if i >= self.entries.len() {
            return;
        }
        self.entries.remove(i);
        for h in [&mut self.left, &mut self.right, &mut self.current] {
            *h = match *h {
                Some(x) if x == i => None,
                Some(x) if x > i => Some(x - 1),
                o => o,
            };
        }
    }

    /// The (skill, level) entries of the join's S→C 0x94.
    ///
    /// TODO(0x94 entries):
    /// which entries the sender takes, in which order and which level it
    /// writes are not stated. D2RUST's own, unverified: the native entries in
    /// list order with their base level (capped at 0xFF), which is what
    /// the measured joins carry for level-1 characters (test vectors:
    /// skill 0 and the class's start skills, each level 1).
    ///
    /// The base game sends it from the skills section's load, before
    /// the worn items are placed, so the list holds no entry their stat
    /// callbacks add yet (base 0, the 0x21 that follow);
    /// D2RUST builds it after the items, so those base-0 entries are left
    /// out (measured with an oskill item worn, frame 1: 10 entries, no
    /// oskill).
    pub fn base_levels(&self) -> Vec<(u16, u8)> {
        // D2RUST's own, unverified
        self.entries
            .iter()
            .filter(|e| e.owner == NATIVE && e.base > 0)
            .map(|e| (e.skill, e.base.clamp(0, 0xFF) as u8))
            .collect()
    }
}

/// The class skill list of `class`: the
/// skills whose i8 `charclass` is `class`, in record order, for 0 ≤
/// class < 7; empty otherwise (reader gives −1).
pub fn class_skills(rows: &[Skills], class: i32) -> Vec<i32> {
    if !(0..7).contains(&class) {
        return Vec::new();
    }
    rows.iter()
        .enumerate()
        .filter(|(_, r)| i32::from(r.charclass as i8) == class)
        .map(|(i, _)| i as i32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Vec<Skills> {
        (0..n)
            .map(|_| {
                let mut s = crate::skills::fake::skill_rec();
                s.charclass = 0xFF;
                s
            })
            .collect()
    }

    #[test]
    fn a_new_player_gets_attack_and_its_class_skills_with_attack_in_both_hands() {
        let mut r = rows(300);
        r[2].anim = 7;
        let cs = [2u16, 1, 217, 218, 219, 220, 4, 5, 3, 0xFFFF];
        let mut l = SkillList::default();
        l.init_player(&r, ListOwner::player(1), Some(&cs)).unwrap();
        let ids: Vec<u16> = l.entries.iter().map(|e| e.skill).collect();
        assert_eq!(ids, [0, 2, 1, 217, 218, 219, 220, 4, 5, 3]);
        assert!(l.entries.iter().all(|e| e.base == 1 && e.owner == NATIVE));
        assert_eq!(l.entries[1].mode, 7);
        assert_eq!((l.left, l.right), (Some(0), Some(0)));
        // Step 8 step 1: run again, nothing is added.
        l.init_player(&r, ListOwner::player(1), Some(&cs)).unwrap();
        assert_eq!(l.entries.len(), 10);
        // Step 2: a class outside charstats: no entry, no select.
        let mut l = SkillList::default();
        l.init_player(&r, ListOwner::player(9), None).unwrap();
        assert_eq!(l, SkillList::default());
        // The 0x94 fill: native entries in list order, base level.
        let mut l = SkillList::default();
        l.init_player(&r, ListOwner::player(1), Some(&cs)).unwrap();
        assert_eq!(l.base_levels()[..3], [(0, 1), (2, 1), (1, 1)]);
    }

    #[test]
    fn add_caps_at_max_level_and_set_base_overwrites() {
        let mut r = rows(40);
        r[36].maxlvl = 2;
        r[37].passivestate = 30;
        let mut l = SkillList::default();
        let p = ListOwner::player(1);
        for _ in 0..3 {
            l.add(&r, p, 36);
        }
        assert_eq!(l.entries[0].base, 2);
        assert_eq!(l.add(&r, p, 40), None);
        l.set_base(&r, p, 36, 9);
        assert_eq!(l.entries[0].base, 9);
        l.set_base(&r, p, 37, 3);
        assert_eq!(l.entries[1].base, 3);
        assert_eq!(
            l.fx,
            [
                ListFx::StateOn(30),
                ListFx::Refresh(37),
                ListFx::Refresh(37)
            ]
        );
        assert!(l.has(37) && !l.has(38));
        assert_eq!(l.select(&r, true, 40, NATIVE), Err(BadSkill(40)));
    }

    #[test]
    fn the_class_list_is_record_order_of_charclass() {
        let mut r = rows(10);
        r[3].charclass = 1;
        r[7].charclass = 1;
        r[5].charclass = 2;
        assert_eq!(class_skills(&r, 1), [3, 7]);
        assert!(class_skills(&r, 7).is_empty());
    }
}
