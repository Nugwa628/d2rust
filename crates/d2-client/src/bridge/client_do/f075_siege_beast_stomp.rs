//! cltdofunc 75 (Siege Beast Stomp): one function of the client skill do
//! table (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 75: with the skill row's
/// `Param1`…`Param4` (+0x148…+0x154; all 0 when the row is not valid)
/// the screen shake with (Param1, Param2 · 1000 / 25, Param3 · 1000
/// / 25, Param4 · 1000 / 25) (32-bit products,
/// quotients toward zero); 1. Flag 0x40 untouched.
pub(super) fn siege_beast_stomp(w: &mut ClientWorld, inputs: &ModelInputs, skill: u16) -> i32 {
    let p = inputs
        .tables
        .skills
        .get(usize::from(skill))
        .map_or([0; 4], |r| [r.param1, r.param2, r.param3, r.param4]);
    let ms = |v: i32| (v.wrapping_mul(1000) / 25) as u32;
    w.start_shake(p[0] as u32, ms(p[1]), ms(p[2]), ms(p[3]));
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[test]
    fn the_stomp_starts_a_screen_shake_from_its_params() {
        let (mut w, key) = world();
        let mut i = inputs(75);
        let r = &mut i.tables.skills[usize::from(FROST_NOVA)];
        (r.param1, r.param2, r.param3, r.param4) = (8, 5, 20, 15);
        w.server_ticks = 7;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        let want = crate::rules::camera::Shake::start(8, 200, 800, 600).unwrap();
        assert_eq!(w.shake.map(|s| (s.shake, s.start_tick)), Some((want, 7)));
        assert!(missiles(&w).is_empty());
        // A sustain of 0 starts nothing (camera).
        let (mut w, key) = world();
        i.tables.skills[usize::from(FROST_NOVA)].param3 = 0;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert!(w.shake.is_none());
    }
}
