//! cltdofunc 45 (Whirlwind): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 45 (Whirlwind). U's path and
/// used entry E, either missing → 0. F := E-flags. F & 3 = 3 (the path
/// ended): the room and path flags of the end (the server copy's
/// here), F := 0, the
/// whirl sound stops (the audio layer's);
/// then 1. Else F bit 0: the sequence position := (position & 0xFF) +
/// 3 · 256 and the frame count := 0x500; 1. Else 1.
pub(super) fn whirlwind(w: &mut ClientWorld, key: UnitKey) -> i32 {
    let Some(e) = current_entry(w, key).copied() else {
        return 0;
    };
    if e.flags & 3 == 3 {
        if let Some(e) = current_entry_mut(w, key) {
            e.flags = 0;
        }
        return 1;
    }
    if e.flags & 1 != 0 {
        if let Some(u) = w.units.get_mut(&key) {
            if let Some(q) = u.sequence.as_mut() {
                q.pos = (q.pos & 0xFF) + 3 * 256;
            }
            u.frame_count = 0x500;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_whirlwind_do_loops_frames_3_to_6_until_the_path_ends() {
        use crate::bridge::use_state::client_start_apply;
        let (mut w, key) = aimed_at((5008, 4999));
        let mut i = inputs(45);
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 31;
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        assert_eq!(w.units[&key].skills.as_ref().unwrap().entries[0].flags, 1);
        assert_eq!(w.units[&key].path_live, Some(true));
        w.units.get_mut(&key).unwrap().sequence = Some(seq15(0x610));
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        let u = &w.units[&key];
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0x310, 0x500)
        );
        // The path ended (bit 1): the flags clear, the frame runs on.
        let l = w.units.get_mut(&key).unwrap().skills.as_mut().unwrap();
        l.entries[0].flags |= 2;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        let u = &w.units[&key];
        assert_eq!(u.skills.as_ref().unwrap().entries[0].flags, 0);
        assert_eq!(u.sequence.as_ref().unwrap().pos, 0x310);
        // A target unit whose reach is not modelled (no item tables):
        // the start changes nothing.
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target =
            Some(CastTarget::Unit(UnitKey::new(MONSTER, 9)));
        client_start_apply(&mut w, &i, key, FROST_NOVA, 1).unwrap();
        assert_eq!(w.units[&key].skills.as_ref().unwrap().entries[0].flags, 0);
    }
}
