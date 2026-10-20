//! cltdofunc 37 (Charge): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 37 (Charge). E := the used
/// entry, none → 0; F := E-flags. The sequence gives n :=
/// its count (none: 15) and the loop end k := −1 when its first frame's
/// event byte is 0 (reads only the first frame), else 7. An
/// action event of 3 (+0x4E) on a player or monster runs
/// (not modelled: an error). Else T := the unit of E's parameters 1 / 2
/// (none when parameter 1 is 6). With F bit 0:
/// - T in reach (not modelled: an error) → F := 0, the
///   position := (position & 0xFF) + (k + 1) · 256, the count := n · 256.
/// - F bit 1 clear (the charge runs): a position at or past k →
///   position := position & 0xFF, count := n · 256 and F := 1; flag 0x40
///   off; the counter + 1 while below 2. Result 1.
/// - F bit 1 set (the path ended): the search [`charge_search`]; none →
///   F := 0, the count := 0x100 (the mode ends this update). Result 1.
///
/// Without F bit 0 the branch that re-aims the path is not
/// modelled (an error).
pub(super) fn charge(w: &mut ClientWorld, key: UnitKey) -> Result<i32, HandlerError> {
    let Some(e) = current_entry(w, key).copied() else {
        return Ok(0);
    };
    let Some(u) = w.units.get(&key) else {
        return Ok(0);
    };
    let (n, k) = match &u.sequence {
        Some(q) => {
            let n = q.length >> 8;
            let first = q.frames.first().map_or(1, |f| f.event);
            (n, if n > 0 && first == 0 { -1 } else { 7 })
        }
        None => (15, 7),
    };
    if key.unit_type <= MONSTER && u.action_frame == 3 {
        return Err(HandlerError::Invalid(
            "client/client-do/f037-charge.md §1 r1: the Charge hit is not modelled",
        ));
    }
    let t = (e.param != 6)
        .then(|| UnitKey::new(e.param as u8, e.param2 as u32))
        .filter(|t| w.units.contains_key(t));
    if e.flags & 1 == 0 {
        return Err(HandlerError::Invalid(
            "client/client-do/f037-charge.md §1 r1: the Charge path re-aim is not modelled",
        ));
    }
    if t.is_some() {
        return Err(HandlerError::Invalid(
            "client/client-do/f037-charge.md §1 r1: the Charge reach test is not modelled",
        ));
    }
    if e.flags & 2 == 0 {
        let u = w.units.get_mut(&key).expect("present");
        if let Some(q) = u.sequence.as_mut() {
            if k <= q.pos >> 8 {
                q.pos &= 0xFF;
                u.frame_count = n << 8;
                if let Some(e) = current_entry_mut(w, key) {
                    e.flags = 1;
                }
            }
        }
        let u = w.units.get_mut(&key).expect("present");
        u.flag_40 = false;
        if let Some(e) = current_entry_mut(w, key) {
            if e.counter < 2 {
                e.counter += 1;
            }
        }
        return Ok(1);
    }
    if charge_search(w, key).is_some() {
        return Err(HandlerError::Invalid(
            "client/client-do/f037-charge.md §1 r1: the Charge search pick is not modelled",
        ));
    }
    if let Some(e) = current_entry_mut(w, key) {
        e.flags = 0;
    }
    if let Some(u) = w.units.get_mut(&key) {
        u.frame_count = 0x100;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_charge_do_loops_while_the_path_runs_and_ends_it_after() {
        use crate::bridge::use_state::client_start_apply;
        let (mut w, key) = aimed_at((5010, 5000));
        let mut i = inputs(37);
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 25;
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        let e = w.units[&key].skills.as_ref().unwrap().entries[0];
        assert_eq!((e.flags, e.param, e.param2, e.param3), (0x1001, 6, 0, 7));
        assert_eq!(w.units[&key].path_live, Some(true));
        let frames = vec![
            d2_sim::skills::sequences::SeqFrame {
                mode: 3,
                frame: 0,
                event: 1,
            };
            15
        ];
        let u = w.units.get_mut(&key).unwrap();
        u.sequence = Some(crate::bridge::world::MonsterSequence {
            frames,
            length: 15 << 8,
            pos: 0x710,
            step: 0x100,
            drawn_mode: 18,
        });
        u.flag_40 = true;
        // The charge runs: past frame 7 the loop restarts, flag 0x40 off.
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        let u = &w.units[&key];
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0x10, 15 << 8)
        );
        assert!(!u.flag_40);
        assert_eq!(u.skills.as_ref().unwrap().entries[0].counter, 1);
        // The path ended (bit 1) and nothing in radius 3: count 0x100.
        let l = w.units.get_mut(&key).unwrap().skills.as_mut().unwrap();
        l.entries[0].flags |= 2;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        let u = &w.units[&key];
        assert_eq!(u.frame_count, 0x100);
        assert_eq!(u.skills.as_ref().unwrap().entries[0].flags, 0);
        // A live monster 3 cells away is a search hit (its pick is an error).
        let m = UnitKey::new(MONSTER, 9);
        let mut c = ClientUnit::new(m);
        (c.position, c.mode) = (Some((5003, 5000)), 1);
        w.units.insert(m, c);
        assert_eq!(charge_search(&w, key), Some(m));
        w.units.get_mut(&m).unwrap().position = Some((5003, 5001));
        assert_eq!(charge_search(&w, key), None);
    }
}
