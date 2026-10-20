//! cltdofunc 4 (Dragon Talon): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 4 (Dragon Talon, skill 255): E
/// := U's used skill entry and T := [`path_target_unit`],
/// either none → 0. n := E +0x18 (the kicks its `cltstfunc` 6 wrote);
/// n − 1 > 0 → E +0x18 := n − 1 and the frame rewind with (U, 100) sets U's frame
/// (+0x44) to 0, so the kick plays again. 1; no flag 0x40.
pub(super) fn dragon_talon(w: &mut ClientWorld, key: UnitKey) -> i32 {
    let target = path_target_unit(w, key);
    let Some(u) = w.units.get_mut(&key) else {
        return 0;
    };
    let Some(e) = u
        .skills
        .as_mut()
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    else {
        return 0;
    };
    if target.is_none() {
        return 0;
    }
    let n = e.param - 1;
    if n > 0 {
        e.param = n;
        rewind(&mut u.frame, 100);
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_dragon_talon_do_restarts_the_kick_while_kicks_are_left() {
        let (mut w, key) = world();
        let param = |w: &ClientWorld| w.units[&key].skills.as_ref().unwrap().entries[0].param;
        let set = |w: &mut ClientWorld, p: i32| {
            let u = w.units.get_mut(&key).unwrap();
            u.skills.as_mut().unwrap().entries[0].param = p;
            u.frame = 0x700;
        };
        // No target unit → 0, nothing written.
        set(&mut w, 4);
        assert_eq!(
            client_do(&mut w, &inputs(4), key, FROST_NOVA, 3).unwrap(),
            0
        );
        assert_eq!((param(&w), w.units[&key].frame), (4, 0x700));
        // Kicks left: one less, the frame back to 0, no flag 0x40.
        let t = UnitKey::new(MONSTER, 9);
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(t));
        assert_eq!(
            client_do(&mut w, &inputs(4), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert_eq!((param(&w), w.units[&key].frame), (3, 0));
        assert!(!w.units[&key].flag_40);
        // The last kick: n − 1 = 0 leaves both.
        set(&mut w, 1);
        assert_eq!(
            client_do(&mut w, &inputs(4), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert_eq!((param(&w), w.units[&key].frame), (1, 0x700));
    }
}
