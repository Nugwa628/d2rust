//! The draw identity of a unit in a shape state (Werewolf `wolf` 139,
//! Werebear `bear` 140: `states` `gfxtype` 1, `gfxclass` 430 / 431): the
//! unit is drawn as that monster, in the monster mode its player mode maps
//! to (q-druid).
//!
//! The model keeps no flag-ex: the substitution applies whenever the
//! model has one of the listed states, tested in ascending id. This is the
//! base-game rule: flag-ex bit 3 is set exactly while
//! the unit has a `disguise` state (toggle), the load-time
//! list holds the `disguise` states in row order, and in the base game
//! the `disguise` states are exactly the
//! `gfxtype` ≠ 0 rows.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::bridge::world::ClientUnit;

/// Player mode (0…19) → monster mode.
const PLAYER_TO_MONSTER: [u8; 20] = [
    0, 1, 2, 15, 3, 1, 2, 4, 5, 6, 7, 4, 11, 8, 9, 10, 11, 12, 14, 13,
];
/// Monster mode (0…15) → player mode.
const MONSTER_TO_PLAYER: [u8; 16] = [0, 1, 2, 4, 7, 8, 9, 10, 13, 14, 15, 16, 17, 19, 18, 3];

/// The `states` rows with a `gfxtype` and the `monstats2` mode bits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disguise {
    /// (state id, `gfxtype`, `gfxclass`) of the rows with `gfxtype` 1 or 2.
    pub states: Vec<(u8, u8, u16)>,
    /// `monstats2` mode bits (mDT = bit 0 … mRN = bit 15) by monster row.
    pub mode_bits: BTreeMap<u32, u16>,
}

/// The mode the loop falls back to when the class lacks mode `m`.
fn fallback(m: u8) -> u8 {
    match m {
        2..=4 => 1,
        5 => 4,
        6 => 3,
        7 => 4,
        8 => 1,
        9..=11 => 8,
        12..=14 => 1,
        15 => 2,
        _ => 1,
    }
}

impl Disguise {
    /// `unit` as it is drawn: the first state of the list it has with a
    /// `gfxtype` 1 (player → monster) or 2 (monster → player).
    pub fn identity<'a>(&self, unit: &'a ClientUnit) -> Cow<'a, ClientUnit> {
        let Some(&(_, ty, class)) = self.states.iter().find(|(s, ..)| unit.states.contains(s))
        else {
            return Cow::Borrowed(unit);
        };
        let mut u = unit.clone();
        match (ty, unit.key.unit_type) {
            (1, 0) => {
                let bits = self
                    .mode_bits
                    .get(&u32::from(class))
                    .copied()
                    .unwrap_or(0xFFFF);
                let mut m = PLAYER_TO_MONSTER
                    .get(unit.mode as usize)
                    .copied()
                    .unwrap_or(1);
                while m != 1 && bits & (1 << m) == 0 {
                    m = fallback(m);
                }
                u.key.unit_type = 1;
                u.class = u32::from(class);
                u.mode = u32::from(m);
            }
            (2, 1) => {
                let m = MONSTER_TO_PLAYER
                    .get(unit.mode as usize)
                    .copied()
                    .unwrap_or(1);
                u.key.unit_type = 0;
                u.class = u32::from(class);
                u.mode = u32::from(if class == 6 && unit.mode == 4 { 12 } else { m });
            }
            _ => return Cow::Borrowed(unit),
        }
        Cow::Owned(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, UnitKey};

    fn player(mode: u32, state: Option<u8>) -> ClientUnit {
        let mut u = ClientUnit::new(UnitKey::new(0, 7));
        u.class = 5;
        u.mode = mode;
        u.states.extend(state);
        u
    }

    fn table() -> Disguise {
        Disguise {
            states: vec![(139, 1, 430), (140, 1, 431)],
            // Werewolf: NU, WL, A1, GH, S1; no SC, no RN.
            mode_bits: [(430, 0b0001_0001_0001_1110), (431, 0b0001_0001_0001_1110)].into(),
        }
    }

    #[test]
    fn a_werewolf_is_drawn_as_the_monster_in_its_mapped_mode() {
        let d = table();
        let u = player(2, Some(139));
        let v = d.identity(&u);
        assert_eq!((v.key.unit_type, v.class, v.mode), (1, 430, 2), "walk → WL");
        assert_eq!(v.key.guid, 7, "the guid stays");
        let cast = player(10, Some(140));
        let v = d.identity(&cast);
        // Player SC → monster 4 (A1 present).
        assert_eq!((v.class, v.mode), (431, 4));
    }

    #[test]
    fn a_mode_the_class_lacks_falls_back() {
        let d = table();
        // Player mode 3 (run) → monster 15 (RN, missing) → WL.
        let v = d.identity(&player(3, Some(139))).into_owned();
        assert_eq!(v.mode, 2);
    }

    #[test]
    fn no_shape_state_leaves_the_unit_alone() {
        let d = table();
        let u = player(1, None);
        assert!(matches!(d.identity(&u), Cow::Borrowed(_)));
    }

    #[test]
    fn the_werewolf_uses_the_monster_cof() {
        use crate::rules::unit_composite::{code, CompositeKind};
        use crate::world_view::unit_assets::{unit_cof, MonsterRow, UnitLooks};
        let mut player_modes = vec![code(b"NU"); 20];
        player_modes[2] = code(b"WL");
        let mut monster_modes = vec![code(b"NU"); 16];
        monster_modes[2] = code(b"WL");
        let looks = UnitLooks {
            player_tokens: vec![code(b"DZ"); 7],
            player_modes,
            monster_modes,
            monsters: [(
                430,
                MonsterRow {
                    token: code(b"40"),
                    base_w: None,
                    composite_death: false,
                    directions: [0; 16],
                },
            )]
            .into(),
            shapes: table(),
            ..UnitLooks::default()
        };
        let human = unit_cof(&looks, &player(2, None)).unwrap();
        assert_eq!(human.kind, CompositeKind::Player);
        let wolf = unit_cof(&looks, &player(2, Some(139))).unwrap();
        assert_eq!(wolf.kind, CompositeKind::Monster);
        assert_eq!(wolf.token, code(b"40"));
        assert_eq!(wolf.mode_token, code(b"WL"));
    }
}
