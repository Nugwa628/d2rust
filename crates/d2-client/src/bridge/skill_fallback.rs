//! The skill fallback (run by the base game from the control
//! panel's skill-button draw): a local player hand on a skill whose level
//! with bonuses is 0 loses the native entry of that skill (remove with
//! d = 1) and goes back to (skill 0, native). The bridge runs it as the
//! last step of a frame whose pump ran a tick while `in_game` (the
//! base game's drawn pass).

use super::dispatch::HandlerError;
use super::msg::skills::level_with_bonuses;
use super::passive;
use super::skills::{self, NATIVE};
use super::world::{ClientWorld, ModelInputs};

/// Left hand, then right. The first error
/// is returned after both hands ran.
pub fn skill_fallback(world: &mut ClientWorld, inputs: &ModelInputs) -> Result<(), HandlerError> {
    let Some(key) = world.local_player else {
        return Ok(());
    };
    let rows = &inputs.tables.skills;
    let desc = &inputs.tables.skilldesc;
    let mut first = Ok(());
    for left in [true, false] {
        let Some(list) = world.units.get(&key).and_then(|u| u.skills.as_ref()) else {
            return first;
        };
        let hand = if left {
            list.left_entry()
        } else {
            list.right_entry()
        };
        let Some(e) = hand.copied() else {
            continue;
        };
        if level_with_bonuses(world, key, rows, desc, &e) > 0 {
            continue;
        }
        let list = world.units.get_mut(&key).and_then(|u| u.skills.as_mut());
        let list = list.expect("checked above");
        let mut r = skills::remove(list, rows, e.skill, true).map_err(HandlerError::from);
        let fx = std::mem::take(&mut list.fx);
        r = r.and(passive::apply(world, inputs, key, fx));
        let list = world.units.get_mut(&key).and_then(|u| u.skills.as_mut());
        let list = list.expect("checked above");
        r = r.and(skills::select(list, rows, left, 0, NATIVE).map_err(HandlerError::from));
        if first.is_ok() {
            first = r;
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::super::skills::{SkillEntry, SkillList, NATIVE};
    use super::super::world::{ClientUnit, ClientWorld, ModelInputs, SkillRow, UnitKey, PLAYER};
    use super::skill_fallback;

    const P: UnitKey = UnitKey::new(PLAYER, 1);

    fn inputs() -> ModelInputs {
        let mut i = ModelInputs::default();
        i.tables.skills = vec![SkillRow::default(); 37];
        i
    }

    fn native(skill: u16, base: i32, level_bonus: i32) -> SkillEntry {
        SkillEntry {
            skill,
            base,
            level_bonus,
            owner: NATIVE,
            ..SkillEntry::default()
        }
    }

    /// The local player: skill 0 (base 1) and skill 36; left on 36,
    /// right on 0.
    fn world(e36: SkillEntry) -> ClientWorld {
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(P);
        u.skills = Some(SkillList {
            entries: vec![native(0, 1, 0), e36],
            left: Some(1),
            right: Some(0),
            ..SkillList::default()
        });
        w.units.insert(P, u);
        w.local_player = Some(P);
        w
    }

    fn list(w: &ClientWorld) -> &SkillList {
        w.units[&P].skills.as_ref().unwrap()
    }

    #[test]
    fn a_level_0_hand_falls_back_to_skill_0() {
        // test vector: entry 36 base 0, no bonus.
        let mut w = world(native(36, 0, 0));
        skill_fallback(&mut w, &inputs()).unwrap();
        let l = list(&w);
        assert_eq!(l.entries, vec![native(0, 1, 0)], "entry 36 unlinked");
        assert_eq!(l.left_entry().map(|e| e.skill), Some(0));
        assert_eq!(l.right_entry().map(|e| e.skill), Some(0));
    }

    #[test]
    fn a_bonus_to_level_0_keeps_the_entry_with_one_base_less() {
        // Entry 36 base 2 with a −2 bonus (level 0): base 1, kept.
        let mut w = world(native(36, 2, -2));
        skill_fallback(&mut w, &inputs()).unwrap();
        let l = list(&w);
        assert_eq!(l.entries[1], native(36, 1, -2));
        assert_eq!(l.left_entry().map(|e| e.skill), Some(0));
    }

    #[test]
    fn a_hand_with_a_level_is_kept() {
        let mut w = world(native(36, 1, 0));
        skill_fallback(&mut w, &inputs()).unwrap();
        let l = list(&w);
        assert_eq!(l.entries.len(), 2);
        assert_eq!(l.left_entry().map(|e| e.skill), Some(36));
    }

    #[test]
    fn no_local_player_does_nothing() {
        let mut w = world(native(36, 0, 0));
        w.local_player = None;
        skill_fallback(&mut w, &inputs()).unwrap();
        assert_eq!(list(&w).entries.len(), 2);
    }
}
