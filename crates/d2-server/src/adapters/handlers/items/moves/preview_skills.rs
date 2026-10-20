//! The skill seams of the preview item-move rest (`InvRest`: mouse
//! skills, `has_skill_owned`, the ranged-throw test, the saved mouse
//! skills) over the players' server skill lists
//! (`ActionHooks::skill_lists`, `d2_sim::skills::list`: never a second
//! copy). The lists are lent to the rest for the length of one move call
//! ([`SkillStage`]) and returned after it, when [`sync_oskills`] has run
//! the stat 97 / 107 callback for the items worn.
//!
//! The stat 97 / 107 callback runs once per
//! change of a player's total, in write order (the stat host queues
//! them, `ActionHooks::oskill_changes`), at the move call's end and
//! after each loaded item (`world/item_save.rs`). PROVISIONAL (D2RUST's
//! own): this callback skips the right hand's state 85 / aura teardown and
//! the skill refresh and pet maximum that follow it; the full callback is
//! `d2_sim::skills::stat_cb`.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::fixup::maps::EquivMatrix;
use d2_sim::items::moves::Owner;
use d2_sim::skills::list::{ListOwner, SkillList, NATIVE};
use d2_sim::skills::use_::item_type::ItemTypeSkill;
use d2_sim::units::messages::{set_skill, update_oskill};
use d2_sim::units::UnitId;
use d2_sim::wiring::action::ActionTables;

/// `thro` (item type 48), the item type of throwing weapons.
const TYPE_THRO: usize = 48;
/// Stat 107 `item_singleskill`.
const SINGLESKILL: u16 = 107;
/// State 134 `shrine_skill`: +2 to every skill level.
pub const SKILL_SHRINE_STATE: u32 = 134;

/// One player's list, lent for a call.
#[derive(Debug)]
pub struct StagedList {
    /// The player's skill list.
    pub list: SkillList,
    /// The player's unit.
    pub unit: UnitId,
    /// The player's character class.
    pub class: i32,
}

/// The players' skill lists lent to the rest for a move call.
#[derive(Debug)]
pub struct SkillStage {
    /// The action tables (skills.txt rows).
    pub tables: Arc<ActionTables>,
    /// The staged lists by player.
    pub lists: BTreeMap<Owner, StagedList>,
}

/// The skills.txt part of the ranged-throw test: `itypea1` > 0 and is-a
/// `thro`, `range` 2, per skill row.
pub fn throw_rows(t: &ActionTables, equiv: &EquivMatrix) -> Vec<bool> {
    t.skills
        .skills
        .iter()
        .map(|r| r.itypea1 > 0 && equiv.get(usize::from(r.itypea1), TYPE_THRO) && r.range == 2)
        .collect()
}

/// The preview rest's skill state.
#[derive(Debug, Default)]
pub struct PreviewSkills {
    /// The lent lists, present during a move call.
    pub stage: Option<SkillStage>,
    /// Per skill row: whether it is a throwing skill ([`throw_rows`]).
    pub throw_rows: Vec<bool>,
    /// The player data's saved mouse skill per (player, left).
    saved: BTreeMap<(Owner, bool), (i32, i32)>,
    /// S→C messages the selections made.
    /// S→C messages the selections made.
    pub sent: Vec<(Owner, Vec<u8>)>,
}

impl PreviewSkills {
    fn list(&self, u: Owner) -> Option<&StagedList> {
        self.stage.as_ref()?.lists.get(&u)
    }

    /// The skill selected on the left or right mouse button, as
    /// (id, owner).
    pub fn mouse_skill(&self, u: Owner, left: bool) -> Option<(i32, i32)> {
        let l = &self.list(u)?.list;
        let i = if left { l.left } else { l.right }?;
        l.entries.get(i).map(|e| (i32::from(e.skill), e.owner))
    }

    /// Select on a side; the client is told (S→C 0x23).
    pub fn select_skill(&mut self, u: Owner, left: bool, s: (i32, i32)) {
        let Some(st) = self.stage.as_mut() else {
            return;
        };
        let rows = &st.tables.skills.skills;
        // No skill (-1) or one outside the table: the select finds no
        // entry, so nothing changes and nothing is sent; a 0x23 with such
        // a skill would be fatal on the client.
        if usize::try_from(s.0).map_or(true, |i| i >= rows.len()) {
            return;
        }
        let Some(sl) = st.lists.get_mut(&u) else {
            return;
        };
        let before = if left { sl.list.left } else { sl.list.right };
        if sl.list.select(rows, left, s.0, s.1).is_err() {
            return;
        }
        let after = if left { sl.list.left } else { sl.list.right };
        if before != after {
            let hand = u8::from(left);
            let owner = s.1 as u32;
            self.sent
                .push((u, set_skill(0, u.guid, hand, s.0 as u16, owner).to_vec()));
        }
    }

    /// Whether the list has the (id, owner) skill.
    pub fn has_skill_owned(&self, u: Owner, s: (i32, i32)) -> bool {
        self.list(u)
            .is_some_and(|l| l.list.find(s.0, s.1).is_some())
    }

    /// Whether the skill row is a throwing skill.
    pub fn throw_skill_row(&self, skill: i32) -> bool {
        usize::try_from(skill)
            .ok()
            .and_then(|i| self.throw_rows.get(i))
            .copied()
            .unwrap_or(false)
    }

    /// Use-state tests 1–4 on the staged list:
    /// no record or `InGame` clear → 3; level (`base + level_bonus`) ≤ 0
    /// → 7; `aura` → 6; `passive` → 5; else 0. No stage, list or entry: 0
    /// (the callers pass an existing entry). The desk runs test 5.
    pub fn skill_use_state(&self, u: Owner, s: (i32, i32)) -> u8 {
        let Some(st) = self.stage.as_ref() else {
            return 0;
        };
        let Some(r) = st.tables.skills.skill(s.0) else {
            return 3;
        };
        if !r.ingame {
            return 3;
        }
        let Some(e) = st
            .lists
            .get(&u)
            .and_then(|sl| sl.list.find(s.0, s.1).map(|i| &sl.list.entries[i]))
        else {
            return 0;
        };
        if e.base + e.level_bonus <= 0 {
            7
        } else if r.aura {
            6
        } else if r.passive {
            5
        } else {
            0
        }
    }

    /// The skills row's item-type sets, from the staged
    /// tables.
    pub fn skill_item_types(&self, skill: i32) -> Option<ItemTypeSkill> {
        let st = self.stage.as_ref()?;
        st.tables
            .skills
            .skill(skill)
            .map(|r| ItemTypeSkill::of(skill, r))
    }

    /// The saved mouse skill of a side, (id, owner); `(0, -1)` when none.
    pub fn saved_mouse_skill(&self, u: Owner, left: bool) -> (i32, i32) {
        self.saved.get(&(u, left)).copied().unwrap_or((0, -1))
    }

    /// Save the mouse skill of a side.
    pub fn set_saved_mouse_skill(&mut self, u: Owner, left: bool, s: (i32, i32)) {
        self.saved.insert((u, left), s);
    }

    /// The quantity of the native entry of `skill`.
    pub fn skill_quantity(&self, u: Owner, skill: i32) -> Option<i32> {
        let l = &self.list(u)?.list;
        l.native(skill).map(|i| l.entries[i].quantity)
    }

    /// Set the quantity of the native entry of `skill`.
    pub fn set_skill_quantity(&mut self, u: Owner, skill: i32, q: i32) {
        if let Some(sl) = self.stage.as_mut().and_then(|s| s.lists.get_mut(&u)) {
            if let Some(i) = sl.list.native(skill) {
                sl.list.entries[i].quantity = q;
            }
        }
    }

    /// The skill is added to the list.
    pub fn learn_skill(&mut self, u: Owner, skill: i32) {
        let Some(st) = self.stage.as_mut() else {
            return;
        };
        let rows = &st.tables.skills.skills;
        if let Some(sl) = st.lists.get_mut(&u) {
            sl.list.add(rows, ListOwner::player(sl.class), skill);
        }
    }
}

/// The stat 97 / 107 callback for each of the
/// player's `changes` (stat, layer = skill, new total) in write order:
/// 107 only for the skill's own class; no native entry → one (base 0)
/// and the client its 0x21; a new total ≤ 0 with no hard points → the
/// hands on the entry back to Attack (S→C 0x23 each) and the entry
/// removed, no message. `total(stat, skill)` reads the player's stat
/// list now (the 0x21 bonus); `shrine` the player's state 134 (skill
/// shrine), which adds 2 to that bonus.
pub fn sync_oskills(
    st: &mut SkillStage,
    u: Owner,
    changes: &[(u16, u16, i32)],
    total: impl Fn(&StagedList, u16, i32) -> i32,
    shrine: impl Fn(&StagedList) -> bool,
    sent: &mut Vec<(Owner, Vec<u8>)>,
) {
    let rows = &st.tables.skills.skills;
    let Some(sl) = st.lists.get_mut(&u) else {
        return;
    };
    let lo = ListOwner::player(sl.class);
    for &(stat, layer, new) in changes {
        let s = i32::from(layer);
        // Skill 0 and rows outside the table are skipped.
        let Some(row) = rows.get(usize::from(layer)).filter(|_| s != 0) else {
            continue;
        };
        if stat == SINGLESKILL && i32::from(row.charclass) != sl.class {
            continue;
        }
        // No native entry yet: add one with base 0 and tell the client.
        if sl.list.native(s).is_none() {
            sl.list.set_base(rows, lo, s, 0);
            // Bonus u8@10 = `bonus_level` of the new entry (base 0, no
            // stored bonus), read after the store, with the skill-shrine
            // term (+2 while state 134 is on).
            let bonus = d2_sim::skills::bonus_level_of(
                &st.tables.skills,
                (s, 0, 0),
                Some(sl.class),
                shrine(sl),
                |stat, layer| total(sl, stat, i32::from(layer)),
            );
            sent.push((
                u,
                update_oskill(0, false, u.guid, s as u16, 0, bonus as u8).to_vec(),
            ));
        }
        // A positive total keeps the entry; so do hard points, with
        // which the level is > 0.
        let Some(i) = sl.list.native(s) else { continue };
        if new > 0 || sl.list.entries[i].base > 0 {
            continue;
        }
        // The hands on the entry go back to Attack: the select sends
        // 0x23 whenever it finds the entry (no comparison with the old
        // selection).
        for left in [true, false] {
            let h = if left { sl.list.left } else { sl.list.right };
            if h == Some(i) && sl.list.find(0, NATIVE).is_some() {
                let _ = sl.list.select(rows, left, 0, NATIVE);
                let hand = u8::from(left);
                sent.push((u, set_skill(0, u.guid, hand, 0, NATIVE as u32).to_vec()));
            }
        }
        // The entry is removed, no message.
        sl.list.remove(i);
    }
}
