//! cltdofunc 70 (Wake of Destruction Sentry): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 70 (Wake of Destruction
/// Sentry): m := [`missile_class`]; invalid → 0. T := the cast
/// target point; flag 0x40 on U; one create with record
/// flags 2, owner and origin U, class m, target offset (dx, dy) := T −
/// U's position. Made → its d28 := −dy, d2C := dx,
/// 1; else 0.
pub(super) fn wake_maker(
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
    let (tx, ty) = cast_target_point(w, key);
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(0);
    };
    set_flag_40(w, key);
    let (dx, dy) = (tx - i32::from(x), ty - i32::from(y));
    let rec = CreateRecord {
        flags: flag::TARGET_RELATIVE,
        owner: Some(key),
        origin: Some(key),
        class: m as u32,
        tx: dx,
        ty: dy,
        skill: i32::from(skill),
        level,
        ..CreateRecord::default()
    };
    let Some(made) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? else {
        return Ok(0);
    };
    if let Some(mm) = w.objclient.missiles.get_mut(&made) {
        mm.d28 = -dy;
        mm.d2c = dx;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_wake_maker_do_stores_the_quarter_turned_aim() {
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(
            client_do(&mut w, &inputs(70), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].owner, Some(key));
        assert_eq!(ms[0].target_point, (5006, 4997));
        assert_eq!((ms[0].d28, ms[0].d2c), (3, 6));
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(client_do(&mut w, &inputs(70), key, 9999, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
    }
}
