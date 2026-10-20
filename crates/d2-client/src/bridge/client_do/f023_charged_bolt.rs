//! cltdofunc 23 (Charged Bolt): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 23 (Charged Bolt):
/// m := [`missile_class`]; invalid → 0. Flag 0x40 on U; n := the
/// skill's `calc1`; a record with flags 0x21, owner U, class m, start U's
/// cell, target [`cast_target_point`] (read once), skill, level and the
/// callback ([`crate::bridge::client_missiles::bolt_init`]) with i
/// (record +0x58) = 0 … n − 1, one create each; 1.
pub(super) fn charged_bolt(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = missile_class(w, key, &row);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let n = calc1(w, inputs, key, skill, level)?;
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    let (tx, ty) = cast_target_point(w, key);
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(key),
        class: m as u32,
        x: i32::from(x),
        y: i32::from(y),
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    for i in 0..n {
        if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
            crate::bridge::client_missiles::bolt_init(w, k, i);
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_charged_bolt_do_makes_calc1_jittered_bolts() {
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(23);
        i.skill_tables = Some(calc1_tables(3));
        i.tables.missiles[CLASS as usize].range = 98;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 3);
        let start = w.units[&key].cell();
        let start = (i32::from(start.0), i32::from(start.1));
        let mut paths = Vec::new();
        for m in &ms {
            // Range 98 → 77 frames, 38 jitter points and the end.
            assert_eq!((m.total, m.current), (77, 77));
            let b = m.bolt.expect("a bolt path");
            assert_eq!((b.count, b.index), (39, 0));
            assert_eq!(b.points[0], start);
            for k in 1..39 {
                let (p, q) = (b.points[k - 1], b.points[k]);
                assert!((q.0 - p.0).abs() == 2 || (q.1 - p.1).abs() == 2);
                assert!((q.0 - p.0).abs() <= 2 && (q.1 - p.1).abs() <= 2);
            }
            paths.push(b.points);
        }
        // Each seed is init_low(target x + i): the paths differ.
        assert!(paths[0] != paths[1] && paths[1] != paths[2]);
    }
}
