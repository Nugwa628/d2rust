//! cltdofunc 20 (Strafe): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 20 (Strafe, skill 26):
/// an invalid row or no used skill entry E → 0; n := E +0x18, n ≤
/// 0 → 0. v := `aurarangecalc` (+0x64). T := the unit (E +0x1C type, E
/// +0x20 GUID), else the pick with (U, 0, 0, v, 3,
/// E +0x20); a T becomes U's target. P :=
/// [`cast_target_point`]. The class: `cltmissilea`, or `cltmissileb`
/// (when ≥ 0) without a bow (hand class ≠ 1); outside the
/// missiles table → 0. One create (flags 0x21: start U's position,
/// target P; owner U, skill, level); E +0x18 := n − 1. n − 1 > 0: N :=
/// the pick with g := T's GUID (no T: −1); none → 0; U faces N, E +0x1C,
/// +0x20 := N's type and GUID, and the frame rewind with (U, Param6)
/// (skills +0x15C) runs. 1; no flag 0x40.
pub(super) fn strafe(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let entry = |w: &ClientWorld| {
        w.units
            .get(&key)
            .and_then(|u| u.skills.as_ref())
            .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
            .copied()
    };
    let Some(e) = entry(w) else {
        return Ok(0);
    };
    let n = e.param;
    if n <= 0 {
        return Ok(0);
    }
    let t = inputs.skill_tables.as_deref().ok_or(HandlerError::Invalid(
        "client/client-do/f020-strafe.md §1 r1: function 20 needs the skills tables",
    ))?;
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let (calc, p) = (srow.aurarangecalc, srow.param6 as i32);
    let v = {
        let mut units = crate::bridge::passive::ClientSkills::new(w, key);
        d2_sim::skills::levels::eval_skill(&mut units, t, Some(key), calc, i32::from(skill), level)
    };
    let env = search_env(inputs);
    let stored = u8::try_from(e.param2)
        .ok()
        .map(|ty| UnitKey::new(ty, e.counter as u32))
        .filter(|k| w.units.contains_key(k));
    let target = match stored {
        Some(k) => Some(k),
        None => {
            crate::bridge::client_missiles::search_pick(
                w,
                &env,
                key,
                (0, 0),
                v,
                3,
                e.counter as u32,
            )?
            .0
        }
    };
    if let Some(k) = target {
        if let Some(u) = w.units.get_mut(&key) {
            u.cast_target = Some(CastTarget::Unit(k));
        }
    }
    let (tx, ty) = cast_target_point(w, key);
    let [a, b, _] = row.cltmissile_abc.map(i32::from);
    let m = if hand_class(w, inputs, key) != 1 && b >= 0 {
        b
    } else {
        a
    };
    if !(0..inputs.tables.missiles.len() as i32).contains(&m) {
        return Ok(0);
    }
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(key),
        class: m as u32,
        x: i32::from(ux),
        y: i32::from(uy),
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    let left = n - 1;
    set_entry(w, key, |e| e.param = left);
    if left > 0 {
        let g = target.map_or(u32::MAX, |k| k.guid);
        let Some(next) =
            crate::bridge::client_missiles::search_pick(w, &env, key, (0, 0), v, 3, g)?.0
        else {
            return Ok(0);
        };
        face_unit(w, key, next);
        set_entry(w, key, |e| {
            e.param2 = i32::from(next.unit_type);
            e.counter = next.guid as i32;
        });
        if let Some(u) = w.units.get_mut(&key) {
            rewind(&mut u.frame, p);
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_strafe_do_needs_arrows_left() {
        let (mut w, key) = world();
        assert_eq!(
            client_do(&mut w, &inputs(20), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert!(missiles(&w).is_empty() && !w.units[&key].flag_40);
    }
}
