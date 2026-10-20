//! cltdofunc 1 (Attack, Left Hand Swing): one function of the client
//! skill do table (`super::client_do`); the shared helpers live in
//! `super`.

use super::*;

/// The COF weapon classes `bow` and `xbw`.
const BOW: i32 = 1;
const CROSSBOW: i32 = 7;

/// `cltdofunc` 1: flag
/// 0x40 on U. U's COF weapon class (here the hand class:
/// a monster reads 0)
/// `bow` or `xbw` → (m, level′) :=
/// the bow missile, m ≠ −1 → [`cast_toward`] m at level′.
/// Any other weapon: nothing (the target read and an empty
/// call). 1 either way.
pub(super) fn attack(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    set_flag_40(w, key);
    if !matches!(hand_class(w, inputs, key), BOW | CROSSBOW) {
        return Ok(1);
    }
    let (m, level) = super::f066_missile_skill::bow_missile(w, inputs, key, level);
    if m != -1 {
        cast_toward(w, inputs, key, m, skill, level)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    #[derive(Debug)]
    struct Hand(i32);
    impl PlayerAnims for Hand {
        fn anim(&self, _: &ClientWorld, _: UnitKey, _: u32) -> Option<PlayerAnim> {
            Some(PlayerAnim {
                frames: 6,
                speed: 256,
                weapon: self.0,
                events: None,
            })
        }
    }

    /// The local player aimed at (5010, 5000) holding COF weapon class
    /// `weapon`, the missile table 64 rows long.
    fn holding(weapon: i32) -> (ClientWorld, ModelInputs, UnitKey) {
        let (w, key) = aimed_at((5010, 5000));
        let mut i = inputs(1);
        i.tables.missiles = vec![ClientMissileRow::default(); 64];
        i.player_anims = Some(Arc::new(Hand(weapon)));
        (w, i, key)
    }

    fn made(w: &ClientWorld) -> Vec<(u32, i32)> {
        w.objclient
            .set_c
            .iter()
            .filter(|(k, _)| k.unit_type == MISSILE)
            .map(|(k, u)| (u.class, w.objclient.missiles[k].level))
            .collect()
    }

    #[test]
    fn a_melee_attack_runs_and_makes_nothing() {
        for weapon in [0, 2, 3, 6, 12] {
            let (mut w, i, key) = holding(weapon);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            assert!(w.units[&key].flag_40);
            assert!(made(&w).is_empty());
        }
    }

    #[test]
    fn a_bow_fires_the_arrow_and_a_crossbow_the_bolt() {
        for (weapon, want) in [(1, (0, 3)), (7, (31, 3))] {
            let (mut w, i, key) = holding(weapon);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            assert_eq!(made(&w), vec![want]);
        }
    }

    #[test]
    fn arrow_stats_pick_the_magic_or_exploding_arrow_at_their_level() {
        for (stats, want) in [
            (vec![(157, 5), (158, 7)], (27, 5)),
            (vec![(158, 7)], (41, 7)),
        ] {
            let (mut w, i, key) = holding(1);
            w.units.get_mut(&key).unwrap().stats.extend(stats);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            assert_eq!(made(&w), vec![want]);
        }
    }

    #[test]
    fn a_monster_takes_the_melee_branch() {
        let (mut w, i, p) = holding(1);
        let key = UnitKey::new(MONSTER, 9);
        let mut u = w.units.remove(&p).unwrap();
        u.key = key;
        w.units.insert(key, u);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        assert!(made(&w).is_empty());
    }
}
