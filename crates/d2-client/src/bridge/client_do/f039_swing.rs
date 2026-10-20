//! cltdofunc 39 (Double Swing, Frenzy): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 39 (Double Swing, Frenzy): below
/// sequence frame 6 U faces T := [`path_target_unit`] (the swing sound is
/// not modelled); from frame 6 on U faces X := the next-GUID pick
/// with (U, pos(U), 3, 0, T's GUID or 0xFFFFFFFF). 1 either way;
/// no flag 0x40, nothing created. Players and monsters always carry a
/// path here, so the path test is not repeated.
pub(super) fn swing(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    let p = w
        .units
        .get(&key)
        .and_then(|u| u.sequence.as_ref())
        .map_or(0, |s| s.pos);
    let t = path_target_unit(w, key);
    let to = if p & !0xff < 0x600 {
        t
    } else {
        let env = crate::bridge::client_missiles::Env {
            rows: &inputs.tables.missiles,
            lights: inputs.high_light_quality,
            skills: inputs.skill_tables.as_deref(),
            monsters: &inputs.tables.monsters,
            model: Some(inputs),
        };
        let g = t.map_or(u32::MAX, |t| t.guid);
        crate::bridge::client_missiles::next_guid(w, &env, key, (0, 0), 3, 0, g)?
    };
    if let Some((x, y)) = to.and_then(|v| w.units.get(&v)).map(|v| v.cell()) {
        crate::bridge::critter_path::face_point(w, key, x, y);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_swing_do_faces_the_target_unit_and_returns_1() {
        let (mut w, key, _) = aimed_at_unit();
        assert_eq!(
            client_do(&mut w, &inputs(39), key, FROST_NOVA, 3).unwrap(),
            1
        );
        let u = &w.units[&key];
        assert!(!u.flag_40 && missiles(&w).is_empty());
        assert_eq!(u.faces, 1);
        assert!(u.path_turn.is_some());
        // No target unit: still 1, no turn.
        let (mut w, key) = aimed_at((5010, 5000));
        assert_eq!(
            client_do(&mut w, &inputs(39), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert_eq!(w.units[&key].faces, 0);
    }
}
