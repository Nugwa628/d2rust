//! cltdofunc 35 (Blessed Hammer): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 35 (Blessed Hammer):
/// m := [`missile_class`]; invalid → 0. Flag 0x40 on U; the
/// point T := [`cast_target_point`] (no zero test: already
/// nudged it). One [`create_toward`] T; made with a path → the path takes
/// type 14 and its spiral (`client_missiles::hammer_init`), 1; else 0.
pub(super) fn blessed_hammer(
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
    let t = cast_target_point(w, key);
    let Some(k) = create_toward(w, inputs, key, m, t, skill, level)? else {
        return Ok(0);
    };
    crate::bridge::client_missiles::hammer_init(w, k);
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_blessed_hammer_do_lays_a_spiral_point_path() {
        let (mut w, key) = aimed_at((5010, 5000));
        assert_eq!(
            client_do(&mut w, &inputs(35), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].owner, Some(key));
        let b = ms[0].bolt.expect("a point path");
        assert_eq!((b.count, b.index), (77, 0));
        let cells = d2_sim::path::walk::missile::hammer_cells(ms[0].pos).unwrap();
        assert_eq!(b.points[76], (cells[76].0 as i32, cells[76].1 as i32));
    }
}
