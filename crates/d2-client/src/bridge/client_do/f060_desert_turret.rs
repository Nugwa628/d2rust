//! cltdofunc 60 (DesertTurret): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// The step of the DesertTurret fan per sign of (dx, dy):
/// a quarter turn of the direction to the target, 2 sub-tiles per axis.
pub(super) fn turret_step(dx: i32, dy: i32) -> (i32, i32) {
    let a = match dy.signum() {
        -1 => 2,
        0 => 0,
        _ => -2,
    };
    let b = 2 * dx.signum();
    (a, b)
}

/// `cltdofunc` 60 (DesertTurret):
/// m := `cltmissilea`; outside the missile table → 0. Flag 0x40
/// on U; T := [`cast_target_point`] (none → 0); d := T − U, (0, 0) → 0.
/// The step (a, b) := [`turret_step`] of d; n := the skill's `calc1`;
/// the offset o := d − (n / 2) · (a, b). n creates, record flags 3
/// (position + relative target), owner U, class m, level (no skill):
/// start U + o / 6 per axis (truncating), relative target o; o += (a, b)
/// after each; 1.
pub(super) fn desert_turret(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let (tx, ty) = cast_target_point(w, key);
    if tx == 0 || ty == 0 {
        return Ok(0);
    }
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let (ux, uy) = (i32::from(ux), i32::from(uy));
    let (dx, dy) = (tx - ux, ty - uy);
    if dx == 0 && dy == 0 {
        return Ok(0);
    }
    let (a, b) = turret_step(dx, dy);
    let n = calc1(w, inputs, key, skill, level)?;
    let h = n / 2;
    let (mut ox, mut oy) = (dx - h * a, dy - h * b);
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(key),
        class: m as u32,
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for _ in 0..n {
        rec.x = ux + ox / 6;
        rec.y = uy + oy / 6;
        rec.tx = ox;
        rec.ty = oy;
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
        ox += a;
        oy += b;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_desert_turret_do_fans_calc1_missiles_a_sixth_of_the_way_out() {
        // A firetower cast, moved to (5000, 5000): d = (−5, 5),
        // step (−2, −2), n = 3.
        let (mut w, key) = aimed_at((4995, 5005));
        let mut i = inputs(60);
        i.skill_tables = Some(calc1_tables(3));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let mut got: Vec<_> = w
            .objclient
            .set_c
            .iter()
            .filter(|(k, _)| k.unit_type == MISSILE)
            .map(|(k, u)| (u.position, w.objclient.missiles[k].target_point))
            .collect();
        got.sort_unstable();
        let mut want = vec![
            (Some((5000, 5001)), (4997, 5008)),
            (Some((5000, 5000)), (4995, 5005)),
            (Some((4999, 5000)), (4992, 5003)),
        ];
        want.sort_unstable();
        assert_eq!(got, want);
        assert!(missiles(&w).iter().all(|m| (m.skill, m.level) == (0, 3)));
        // Aimed at itself: nothing.
        let (mut w, key) = aimed_at((5000, 5000));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
    }

    #[test]
    fn the_turret_step_is_a_quarter_turn_by_sign() {
        let cases = [
            ((-1, -1), (2, -2)),
            ((-1, 0), (0, -2)),
            ((-1, 1), (-2, -2)),
            ((0, -1), (2, 0)),
            ((0, 1), (-2, 0)),
            ((1, -1), (2, 2)),
            ((1, 0), (0, 2)),
            ((7, 9), (-2, 2)),
        ];
        for ((dx, dy), want) in cases {
            assert_eq!(turret_step(dx, dy), want, "{dx} {dy}");
        }
    }
}
