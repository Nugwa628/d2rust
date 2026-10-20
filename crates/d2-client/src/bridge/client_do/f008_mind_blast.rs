//! cltdofunc 8 (Mind Blast): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 8 (Mind Blast, skill 273):
/// an invalid skill row → 0. P := [`cast_target_point`]; a :=
/// `cltmissilea`, b := `cltmissileb`, either negative → 0. v := the
/// skill's `aurarangecalc` (+0x64) through. One create at
/// P (flags 1, owner U, class a, skill, level); a made missile X takes
/// data +4 := b and +6 := v, the stream
/// sub-missile and radius its client do 17 reads. 1; no flag 0x40.
pub(super) fn mind_blast(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let p = cast_target_point(w, key);
    let [a, b, _] = row.cltmissile_abc.map(i32::from);
    if a < 0 || b < 0 {
        return Ok(0);
    }
    let t = inputs.skill_tables.as_deref().ok_or(HandlerError::Invalid(
        "client/client-do/f008-mind-blast.md §1 r1: function 8 needs the skills tables",
    ))?;
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let calc = srow.aurarangecalc;
    let v = {
        let mut units = crate::bridge::passive::ClientSkills::new(w, key);
        d2_sim::skills::levels::eval_skill(&mut units, t, Some(key), calc, i32::from(skill), level)
    };
    if !valid_missile(inputs, a) {
        return Ok(1);
    }
    if let Some(x) = create_at(w, inputs, key, a, p, skill, level)? {
        if let Some(m) = w.objclient.missiles.get_mut(&x) {
            m.d04 = b as i16;
            m.d06 = v as i16;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_mind_blast_do_needs_both_client_missiles() {
        // cltmissileb −1 → 0, no create, no flag 0x40.
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5004, 5000));
        assert_eq!(
            client_do(&mut w, &inputs(8), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(missiles(&w).is_empty());
        assert!(!w.units[&key].flag_40);
    }
}
