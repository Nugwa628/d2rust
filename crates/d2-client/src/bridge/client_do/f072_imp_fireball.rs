//! cltdofunc 72 (Imp Fireball): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// State 143 `attached`: its `setfunc` 13 gives the unit a
/// motion record that follows its linked unit.
const ATTACHED: u8 = 143;
/// State 98 `sourceunit`: the linked unit (stats 353 type,
/// 354 GUID; setfunc 4).
const SOURCE_UNIT: u8 = 98;
/// Component 14 (S7), the layer the follow update reads.
const S7: u8 = 14;

/// `cltdofunc` 72 (Imp Fireball): the
/// row not valid → 0. m := [`missile_class`]; outside the missile
/// table → 0. Flag 0x40 on U; X := the create toward U's
/// cast target point (record flags 0x420, owner and origin U); none → 0.
/// s := the skill's `cltcalc2` (+0x118) + U's height ([`height`]); n :=
/// X's total frames; v := s / n (s / 2 when n = 0, C division). X's
/// position := (0, 0, s ≪ 11), velocity := (0, 0, −v ≪ 11), one motion
/// update; 1.
pub(super) fn imp_fireball(
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
    set_flag_40(w, key);
    let rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE | flag::FRAMES_FROM_DISTANCE,
        owner: Some(key),
        origin: Some(key),
        class: m as u32,
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    let Some(x) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? else {
        return Ok(0);
    };
    let s = skill_calc(w, inputs, key, skill, level, |r| r.cltcalc2)?
        .wrapping_add(height(w, inputs, key));
    let Some(mm) = w.objclient.missiles.get_mut(&x) else {
        return Ok(1);
    };
    let n = mm.total;
    let v = if n != 0 { s.wrapping_div(n) } else { s / 2 };
    mm.motion.pos = [0, 0, s.wrapping_shl(11)];
    mm.motion.vel = [0, 0, v.wrapping_neg().wrapping_shl(11)];
    mm.motion.update(true, None).map_err(|_| {
        HandlerError::Invalid("render/unit-composite.md §8 r4: a missile follows a non-monster")
    })?;
    Ok(1)
}

/// U's height with (U): its motion record's z ≫ 11, 0 without a
/// record. The only monster records the model knows of are the
/// follow records of state 143 `attached` (`setfunc` 13): update
/// sets z := −oz · 2048 with oz = b + K's oz, (a, b) the linked unit K's
/// S7 cel offset, so the height is −b (K's own oz is 0: no record).
/// PROVISIONAL: read at the do from K's current frame, not kept
/// from the record's last update; K without the S7 layer → 0.
fn height(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey) -> i32 {
    let Some(u) = w.units.get(&key) else {
        return 0;
    };
    if !u.states.contains(&ATTACHED) {
        return 0;
    }
    let Some(l) = u.state_lists.get(&SOURCE_UNIT) else {
        return 0;
    };
    let (Some(&t), Some(&g)) = (l.get(&(353, 0)), l.get(&(354, 0))) else {
        return 0;
    };
    let Ok(unit_type) = u8::try_from(t) else {
        return 0;
    };
    let k = UnitKey {
        unit_type,
        guid: g as u32,
    };
    w.units
        .get(&k)
        .zip(inputs.cel_offset.as_ref())
        .and_then(|(k, f)| f.cel_offset(k, S7))
        .map_or(0, |(_, b)| b.wrapping_neg())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_imp_fireball_do_fires_toward_the_target_with_flag_40() {
        let mut i = inputs(72);
        i.skill_tables = Some(calc12_tables(0, 0));
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].owner, Some(key));
        // No height and no cltcalc2: s = 0, so the in-do update stops it.
        assert_eq!(ms[0].motion.pos[2], 0);
        // Unknown skill: 0, nothing made.
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(client_do(&mut w, &i, key, 9999, 3).unwrap(), 0);
        assert!(missiles(&w).is_empty());
    }

    #[test]
    fn an_attached_caster_fires_from_its_linked_units_s7_height() {
        let (mut w, key, cow) = aimed_at_cow();
        let u = w.units.get_mut(&key).unwrap();
        u.states.insert(ATTACHED);
        let l = u.state_lists.entry(SOURCE_UNIT).or_default();
        l.insert((353, 0), i32::from(cow.unit_type));
        l.insert((354, 0), cow.guid as i32);
        let mut i = inputs(72);
        // cltcalc2 (offset 0 of the code) = 40.
        i.skill_tables = Some(calc12_tables(40, 0));
        i.cel_offset = Some(crate::bridge::world::CelOffsetFn::new(|_, c| {
            (c == 14).then_some((-14, -91))
        }));
        assert_eq!(height(&w, &i, key), 91);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        let n = ms[0].total;
        assert!(n > 0);
        // s = 40 + 91; one update from z = s ≪ 11 at vz = −(s / n) ≪ 11.
        assert_eq!(ms[0].motion.pos[2], (131 << 11) - ((131 / n) << 11));
        assert_eq!(ms[0].motion.flags & 1, 0);
    }
}
