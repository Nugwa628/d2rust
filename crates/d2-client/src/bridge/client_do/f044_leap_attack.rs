//! cltdofunc 44 (Leap Attack): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 44 (Leap Attack). E := the used
/// entry, none → 0; F := E-flags, r := 0.
/// - F bit 0x100 (in the air): the landing test. U's arc
///   height (the record's z >> 11, 0 without one) 0, or U at
///   the point (E +0x18 / +0x1C), lands: the path goes back to a walk
///   (the server copy's here), the arc is removed, the
///   landing sounds (the audio layer's); then T := the unit of E +0x20 /
///   +0x24 (none when +0x20 is 6), else the pick with (U, 0, 3, 0,
///   −1); none → frame count := 0x400; a T (the face, the swing sound)
///   is not modelled (an error). Not landed: a player's sequence position
///   := (position & 0xFF) + 10 · 256 and frame count := (count & !0xFF) +
///   0x100, and the test fails (a monster's frame is not modelled: an
///   error); F bit 1 clear → 0, set → the end below.
/// - F bit 1 clear: F bit 0x80 (the takeoff): with a point,
///   E-flags := 0x1101 and U's path to it (path type 9 at U's run
///   velocity, the server copy's: a moving path);: the arc
///   of [`crate::world_view::skill_motion::leap_record`] (d := U to the
///   point, the run velocity); 1. Else E-flags := 0x1000;
///   r.
/// - F bit 1 set (the path ended): the walk path, E-flags
///   := 0; 1.
pub(super) fn leap_attack(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    let Some(e) = current_entry(w, key).copied() else {
        return Ok(0);
    };
    let f = e.flags;
    let mut r = 0;
    if f & 0x100 != 0 {
        let Some(u) = w.units.get(&key) else {
            return Ok(0);
        };
        let height = u
            .leap_arc
            .map_or(0, |a| if a.flags & 1 != 0 { 0 } else { a.pos[2] >> 11 });
        let at = u
            .position
            .is_some_and(|(x, y)| (i32::from(x), i32::from(y)) == (e.param, e.param2));
        if height == 0 || at {
            let u = w.units.get_mut(&key).expect("present");
            u.leap_arc = None;
            let t = (e.counter != 6)
                .then(|| UnitKey::new(e.counter as u8, e.param3 as u32))
                .filter(|t| w.units.contains_key(t));
            if t.is_some() || charge_search(w, key).is_some() {
                return Err(HandlerError::Invalid(
                    "client/client-do/f044-leap-attack.md §1 r1: the Leap Attack landing face is not modelled",
                ));
            }
            if let Some(u) = w.units.get_mut(&key) {
                u.frame_count = 0x400;
            }
            r = 0;
        } else {
            if key.unit_type != PLAYER {
                return Err(HandlerError::Invalid(
                    "client/client-do/f044-leap-attack.md §1 r1: a monster's Leap Attack frame is not modelled",
                ));
            }
            let u = w.units.get_mut(&key).expect("present");
            if let Some(q) = u.sequence.as_mut() {
                q.pos = (q.pos & 0xFF) + 10 * 256;
            }
            u.frame_count = (u.frame_count & !0xFF) + 0x100;
            if f & 2 == 0 {
                return Ok(0);
            }
        }
    }
    if f & 2 != 0 {
        if let Some(e) = current_entry_mut(w, key) {
            e.flags = 0;
        }
        return Ok(1);
    }
    if f & 0x80 == 0 {
        if let Some(e) = current_entry_mut(w, key) {
            e.flags = 0x1000;
        }
        return Ok(r);
    }
    // The takeoff.
    if e.param != 0 && e.param2 != 0 {
        if let Some(e) = current_entry_mut(w, key) {
            e.flags = 0x1101;
        }
        let Some(u) = w.units.get_mut(&key) else {
            return Ok(1);
        };
        u.path_live = Some(true);
        let run = inputs
            .tables
            .class_velocities
            .get(u.class as usize)
            .map_or(0, |v| i32::from(v[1]));
        let (ux, uy) = u.cell();
        let (dx, dy) = (
            (e.param - i32::from(ux)).abs(),
            (e.param2 - i32::from(uy)).abs(),
        );
        let d = dx.max(dy) + dx.min(dy) / 2;
        u.leap_arc = crate::world_view::skill_motion::leap_record(d, run);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_leap_attack_takes_off_holds_frame_10_in_the_air_and_lands() {
        use crate::bridge::use_state::client_start_apply;
        // From (5000, 5000) at (5008, 4999): d = 8, the point (5010, 4999).
        let (mut w, key) = aimed_at((5008, 4999));
        let mut i = inputs(44);
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 30;
        i.tables.class_velocities = vec![[6, 9]; 7];
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        let e = w.units[&key].skills.as_ref().unwrap().entries[0];
        assert_eq!(
            (e.flags, e.param, e.param2, e.counter),
            (0x1080, 5010, 4999, 6)
        );
        // The takeoff: flags 0x1101, a moving path, the arc of d 10 at run
        // velocity 9: n = (10 << 16) / (9 << 12) − 1 = 16.
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        let u = &w.units[&key];
        assert_eq!(u.skills.as_ref().unwrap().entries[0].flags, 0x1101);
        assert_eq!(u.path_live, Some(true));
        assert_eq!(u.leap_arc.unwrap().ticks_left, 16);
        // In the air: frame 10, the count kept; 0.
        let u = w.units.get_mut(&key).unwrap();
        u.sequence = Some(seq15(0xB20));
        u.frame_count = 0xB00;
        u.leap_arc.as_mut().unwrap().update(false, None).unwrap();
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 0);
        let u = &w.units[&key];
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0xA20, 0xC00)
        );
        // Landed (height 0), nothing in radius 3: count 0x400, flags 0x1000.
        w.units.get_mut(&key).unwrap().leap_arc = None;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 0);
        let u = &w.units[&key];
        assert_eq!(u.frame_count, 0x400);
        assert_eq!(u.skills.as_ref().unwrap().entries[0].flags, 0x1000);
    }
}
