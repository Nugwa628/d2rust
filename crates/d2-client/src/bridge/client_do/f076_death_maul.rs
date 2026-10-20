//! cltdofunc 76 (DeathMaul): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// The path distance from U to (x, y): with dx, dy := |x − U.x|, |y − U.y|, the
/// path length max + min / 2, as (min + 2·max) / 2.
pub(super) fn path_distance(w: &ClientWorld, key: UnitKey, (x, y): (i32, i32)) -> i32 {
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return 0;
    };
    let dx = x.wrapping_sub(i32::from(ux)).wrapping_abs().max(0);
    let dy = y.wrapping_sub(i32::from(uy)).wrapping_abs().max(0);
    let (lo, hi) = if dy < dx { (dy, dx) } else { (dx, dy) };
    lo.wrapping_add(hi.wrapping_mul(2)) / 2
}

/// The 16-bit store of the missile data setters:
/// below −0x7FFF → −0x8000, above 0x7FFE →
/// 0x7FFF.
pub(super) fn clamp16(v: i32) -> i32 {
    if v < -0x7FFF {
        -0x8000
    } else {
        v.min(0x7FFF)
    }
}

/// Total := current := k + j, activate := k − j (,
/// k the missile's own total),
/// then unit flag-ex 0x40000 (not drawn during `InitSteps`).
pub(super) fn delay_missile(w: &mut ClientWorld, x: UnitKey, j: i32) {
    if let Some(m) = w.objclient.missiles.get_mut(&x) {
        let k = m.total;
        m.total = clamp16(k.wrapping_add(j));
        m.current = clamp16(k.wrapping_add(j));
        m.activate = clamp16(k.wrapping_sub(j));
    }
    if let Some(u) = w.objclient.set_c.get_mut(&x) {
        u.flag_ex |= crate::bridge::client_missiles::FLAG_EX_NOT_DRAWN;
    }
}

/// `cltdofunc` 76 (DeathMaul):
/// b := `cltmissileb`, outside the table → 0; T := the cast target
/// point; d := max(1, [`path_distance`] T); the step (sx, sy) := (T − U)
/// · 100 / d. With s := max(1, b's `Param1`), i := max(1, b's `Param2`):
/// at a = s, 2s, … while a < d one create at U + a·(sx, sy)/100
/// (the create with skill and level), each made one facing U's direction and
/// delayed by j = 0, i, 2i, … ([`delay_missile`]). Then a :=
/// `cltmissilea` in the table → X := [`cast_at`] a (none → 0), its
/// animation speed := min(calc1, 0x7FFF) when calc1 > 0, delayed by n·i
/// (n trail steps); 1.
pub(super) fn death_maul(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let b = i32::from(row.cltmissile_abc[1]);
    let Some(trail) = usize::try_from(b)
        .ok()
        .and_then(|i| inputs.tables.missiles.get(i))
        .copied()
    else {
        return Ok(0);
    };
    let t = cast_target_point(w, key);
    let d = path_distance(w, key, t).max(1);
    let Some((ux, uy)) = w
        .units
        .get(&key)
        .map(|u| (i32::from(u.cell().0), i32::from(u.cell().1)))
    else {
        return Ok(0);
    };
    let sx = t.0.wrapping_sub(ux).wrapping_mul(100) / d;
    let sy = t.1.wrapping_sub(uy).wrapping_mul(100) / d;
    let step = trail.param[0].max(1);
    let inc = trail.param[1].max(1);
    let (mut a, mut j, mut n) = (step, 0i32, 0i32);
    let (mut ax, mut ay) = (step.wrapping_mul(sx), step.wrapping_mul(sy));
    while a < d {
        let at = (ux.wrapping_add(ax / 100), uy.wrapping_add(ay / 100));
        if let Some(x) = create_at(w, inputs, key, b, at, skill, level)? {
            // U's direction, unless the path keeps its facing (flag
            // 0x40).
            let dir = owner_dir64(w, key);
            if let Some(m) = w.objclient.missiles.get_mut(&x) {
                if let (false, Some(dir)) = (m.keep_facing, dir) {
                    m.direction = dir & 0x3F;
                }
            }
            delay_missile(w, x, j);
        }
        a = a.wrapping_add(step);
        ax = ax.wrapping_add(step.wrapping_mul(sx));
        ay = ay.wrapping_add(step.wrapping_mul(sy));
        n += 1;
        j = j.wrapping_add(inc);
    }
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(1);
    }
    let Some(x) = cast_at(w, inputs, key, m, skill, level)? else {
        return Ok(0);
    };
    let v = calc1(w, inputs, key, skill, level)?;
    if v > 0 {
        if let Some(mm) = w.objclient.missiles.get_mut(&x) {
            mm.anim_speed = v.min(0x7FFF);
        }
    }
    delay_missile(w, x, n.wrapping_mul(inc));
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_death_maul_do_lays_delayed_trails_then_the_delayed_maul() {
        use crate::bridge::client_missiles::FLAG_EX_NOT_DRAWN;
        let mut i = inputs(76);
        i.skill_tables = Some(calc1_tables(9));
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, 2, -1];
        i.tables.missiles[2] = ClientMissileRow {
            range: 10,
            param: [2, 1],
            ..ClientMissileRow::default()
        };
        // U (5000, 5000), T (5006, 5000): d 6, trails at a = 2, 4.
        let (mut w, key) = aimed_at((5006, 5000));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let mut trails: Vec<_> = w
            .objclient
            .missiles
            .iter()
            .filter(|(k, _)| w.objclient.set_c[k].class == 2)
            .map(|(k, m)| (w.objclient.set_c[k].position, m.total - m.activate, *k))
            .collect();
        trails.sort_unstable();
        assert_eq!(
            trails.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>(),
            [(Some((5002, 5000)), 0), (Some((5004, 5000)), 2)]
        );
        let (mk, maul) = w
            .objclient
            .missiles
            .iter()
            .find(|(k, _)| w.objclient.set_c[k].class == CLASS)
            .unwrap();
        // Two trail steps · Param2 1: total − activate = 2 · 2.
        assert_eq!(maul.total - maul.activate, 4);
        assert_eq!(maul.anim_speed, 9);
        assert!(w.objclient.set_c[mk].flag_ex & FLAG_EX_NOT_DRAWN != 0);
        // cltmissileb outside the table: 0, nothing made.
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [CLASS as i16, 99, -1];
        let (mut w, key) = aimed_at((5006, 5000));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
    }
}
