//! cltstfunc 42 (Mosquito): one function of the client skill start
//! table (`super::client_start_apply`); the shared helpers live in `super`.

use super::*;

/// `cltstfunc` 42: T := U's
/// path target unit (none → nothing); the loop count E.param := max(1,
/// calc1 + a draw below calc2 − calc1 on the seed {U.GUID + T.GUID, 666});
/// E +0x1C := T's GUID, E +0x20 := T's type.
pub(super) fn mosquito_start(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    caster: UnitKey,
    skill: u16,
    level: i32,
) -> Result<(), HandlerError> {
    let Some(t) = crate::bridge::client_do::path_target_unit(w, caster) else {
        return Ok(());
    };
    let c1 = crate::bridge::client_do::calc1(w, inputs, caster, skill, level)?;
    let c2 = crate::bridge::client_do::calc2(w, inputs, caster, skill, level)?;
    let mut seed = d2_sim::rng::Seed::new(caster.guid.wrapping_add(t.guid), 666);
    let n = c1
        .wrapping_add(seed.roll(c2.wrapping_sub(c1)) as i32)
        .max(1);
    if let Some(e) = crate::bridge::skills::used_entry_mut(w, caster) {
        (e.param, e.param2, e.counter) = (n, t.guid as i32, i32::from(t.unit_type));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::skills::SkillList;
    use crate::bridge::world::{ClientUnit, PLAYER};
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};

    fn blank<T: Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }

    /// U GUID 8 at a target T GUID 1 (seed {9, 666}); `calc1` 4, `calc2`
    /// 7: E.param = 4 + one draw below 3 on that seed; E +0x1C / +0x20 :=
    /// T's GUID and type. No target: the entry is left alone.
    #[test]
    fn the_loop_count_draws_on_the_two_guids() {
        const SKILL: u16 = 3;
        let mut w = ClientWorld::default();
        let t = UnitKey::new(PLAYER, 1);
        w.units.insert(t, ClientUnit::new(t));
        let key = UnitKey::new(PLAYER, 8);
        let mut u = ClientUnit::new(key);
        u.cast_target = Some(CastTarget::Unit(t));
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: SKILL,
                base: 1,
                owner: NATIVE,
                ..SkillEntry::default()
            }],
            current: Some(0),
            ..SkillList::default()
        });
        w.units.insert(key, u);
        let mut i = ModelInputs::default();
        i.tables.skills = vec![SkillRow::default(); 4];
        let mut row: Skills = blank();
        (row.calc1, row.calc2) = (0, 3);
        let mut skills = vec![blank::<Skills>(); 4];
        skills[usize::from(SKILL)] = row;
        i.skill_tables = Some(std::sync::Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, 4, 0x00, 0x07, 7, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        let mut seed = d2_sim::rng::Seed::new(9, 666);
        let want = 4 + seed.roll(3) as i32;
        mosquito_start(&mut w, &i, key, SKILL, 1).unwrap();
        let e = w.units[&key].skills.as_ref().unwrap().entries[0];
        assert_eq!((e.param, e.param2, e.counter), (want, 1, i32::from(PLAYER)));
        assert!((4..7).contains(&e.param));
        w.units.get_mut(&key).unwrap().cast_target = None;
        w.units
            .get_mut(&key)
            .unwrap()
            .skills
            .as_mut()
            .unwrap()
            .entries[0]
            .param = 0;
        mosquito_start(&mut w, &i, key, SKILL, 1).unwrap();
        assert_eq!(w.units[&key].skills.as_ref().unwrap().entries[0].param, 0);
    }
}
