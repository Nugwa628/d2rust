//! cltdofunc 66 (MissileSkill1 / HireableMissile / RogueMissile): one
//! function of the client skill do table (`super::client_do`); the
//! shared helpers live in `super`.

use super::*;

/// `item_magicarrow` (157) and `item_explosivearrow` (158).
const MAGIC_ARROW: u16 = 157;
const EXPLOSIVE_ARROW: u16 = 158;

/// The bow missile with (U, &level):
/// a player takes the arrows with [`hand_class`] 1, the bolts
/// with 7, else −1; any other unit the arrows. Arrows / bolts: U's stat
/// 157 > 0 → (27, it), else 158 > 0 → (41, it), else arrows 0 and bolts
/// 31 with the level unchanged.
pub(super) fn bow_missile(
    w: &ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    level: i32,
) -> (i32, i32) {
    let fallback = if key.unit_type != PLAYER {
        0
    } else {
        match hand_class(w, inputs, key) {
            1 => 0,
            7 => 31,
            _ => return (-1, level),
        }
    };
    for (stat, m) in [(MAGIC_ARROW, 27), (EXPLOSIVE_ARROW, 41)] {
        let v = w.total(key, stat, 0);
        if v > 0 {
            return (m, v);
        }
    }
    (fallback, level)
}

/// `cltdofunc` 66: the skill
/// row outside the table → 0. Flag 0x40 on U; (m, level) := the bow
/// missile. m neither 0 nor 31 → m outside the missile table → 0, else
/// [`cast_toward`] m with the bow level; 1. m 0 or 31 → m :=
/// `cltmissilea` (outside the table → 0); the skill's `lob` → the lob
/// create toward the cast target, else [`cast_toward`]; 1.
pub(super) fn missile_skill(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    set_flag_40(w, key);
    let (m, level) = bow_missile(w, inputs, key, level);
    if m != 0 && m != 31 {
        if !valid_missile(inputs, m) {
            return Ok(0);
        }
        cast_toward(w, inputs, key, m, skill, level)?;
        return Ok(1);
    }
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    if row.lob {
        lob(w, inputs, key, m as u32, (0, 0), skill, level)?;
    } else {
        cast_toward(w, inputs, key, m, skill, level)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    /// A rogue (monster) aimed at (5010, 5004), the missile table 64
    /// rows long (27 magic arrow, 41 exploding arrow).
    fn rogue() -> (ClientWorld, ModelInputs, UnitKey) {
        let (mut w, p) = aimed_at((5010, 5004));
        let key = UnitKey::new(MONSTER, 9);
        let mut u = w.units.remove(&p).unwrap();
        u.key = key;
        w.units.insert(key, u);
        let mut i = inputs(66);
        i.tables.missiles = vec![ClientMissileRow::default(); 64];
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
    fn a_rogue_without_arrow_stats_fires_cltmissilea() {
        let (mut w, i, key) = rogue();
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        assert_eq!(made(&w), vec![(CLASS, 3)]);
    }

    #[test]
    fn arrow_stats_pick_the_magic_then_the_exploding_arrow_at_their_level() {
        for (stats, want) in [
            (vec![(157, 5), (158, 7)], (27, 5)),
            (vec![(158, 7)], (41, 7)),
        ] {
            let (mut w, i, key) = rogue();
            w.units.get_mut(&key).unwrap().stats.extend(stats);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
            assert_eq!(made(&w), vec![want]);
        }
    }

    #[test]
    fn a_player_without_a_bow_or_crossbow_makes_nothing() {
        let (mut w, key) = aimed_at((5010, 5004));
        let i = inputs(66);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(w.units[&key].flag_40);
        assert!(made(&w).is_empty());
    }

    #[test]
    fn a_missing_cltmissilea_makes_nothing() {
        let (mut w, mut i, key) = rogue();
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [-1, -1, -1];
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(made(&w).is_empty());
    }
}
