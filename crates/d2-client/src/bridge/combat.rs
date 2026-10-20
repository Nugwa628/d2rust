//! What the world-click dispatcher ([`super::click::ModelClick`]) needs
//! to attack a monster: the skill row facts from the client
//! `skills` rows, `range(P, skill)`, and the play preview's answers for
//! the inputs the model does not hold (hover, hostility, melee range).
//! Each preview answer is marked `D2RUST's own, unverified`.

use crate::controls::click::{range, SkillRowFacts};

use super::world::{ClientUnit, ClientWorld, ModelInputs, SkillRow, UnitKey, MONSTER};

/// The flag word of a `skills` row, by `skills.txt` bit
/// (`controls::click::skill_flag`).
pub use crate::app::single_player::skill_flags;

/// The facts of skill `id`, `None` for an id outside the table.
pub fn row_facts(inputs: &ModelInputs, id: u16) -> Option<SkillRowFacts> {
    let r = inputs.tables.skills.get(usize::from(id))?;
    Some(SkillRowFacts {
        flags: r.flags,
        range: r.range,
        srvdofunc: r.srvdofunc,
    })
}

/// `range(P, skill)` of a player:
/// the row's `range`; `both` (3) is rng with a bow or crossbow equipped
/// (`bow`), else h2h; rng (2) with a state of the state-mask group 0x26
/// (`melee_only`) is h2h.
pub fn skill_range(row: &SkillRow, bow: bool, melee_only: bool) -> u8 {
    let r = match row.range {
        3 if bow => range::RNG,
        3 => range::H2H,
        r @ (range::NONE | range::H2H | range::RNG | range::LOC) => r,
        _ => range::NONE,
    };
    if r == range::RNG && melee_only {
        range::H2H
    } else {
        r
    }
}

/// The state-mask test with (U, 0x26):
/// U has a state whose `states.txt` row sets `meleeonly` (flag bit 0x26).
pub fn in_melee_only_state(inputs: &ModelInputs, u: &ClientUnit) -> bool {
    u.states.iter().any(|&s| {
        inputs
            .tables
            .states
            .get(usize::from(s))
            .is_some_and(|r| r.meleeonly)
    })
}

/// [`skill_range`] of skill `id` for the player `p`; `NONE` outside the
/// table. D2RUST's own, unverified (preview): the preview player holds no
/// items, so no bow or crossbow is equipped (`both` reads as h2h).
pub fn range_of(inputs: &ModelInputs, p: Option<&ClientUnit>, id: u16) -> u8 {
    let melee_only = p.is_some_and(|u| in_melee_only_state(inputs, u));
    inputs
        .tables
        .skills
        .get(usize::from(id))
        .map_or(range::NONE, |r| skill_range(r, false, melee_only))
}

/// A monster that is a town NPC (`monstats` `npc`).
fn is_npc(inputs: &ModelInputs, u: &ClientUnit) -> bool {
    inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref())
        .is_some_and(|c| c.npc)
}

/// The hovered unit for a click at world sub-tile `at`.
///
/// D2RUST's own, unverified (TODO: hover): the model has no hover (no
/// sprite boxes), so the
/// preview hovers the nearest monster that is not a town NPC (NPC
/// clicks stay with their own path) and passes the selectable gate
/// ([`super::hover::monster_selectable`]) whose
/// feet are within one sub-tile
/// of `at`, or of `at` moved up to four sub-tiles down-screen (+1, +1
/// per step: a click on the body lands behind the feet). Nearest = the
/// fewest down-screen steps, then the smallest Chebyshev distance, then
/// the lowest key (stable).
pub fn hover_at(world: &ClientWorld, inputs: &ModelInputs, at: (i32, i32)) -> Option<UnitKey> {
    let mut best: Option<((i32, i32), UnitKey)> = None;
    for (key, u) in &world.units {
        if key.unit_type != MONSTER
            || Some(*key) == world.local_player
            || is_npc(inputs, u)
            || !super::hover::monster_selectable(u)
        {
            continue;
        }
        let Some((x, y)) = u.position else {
            continue;
        };
        let (ux, uy) = (i32::from(x), i32::from(y));
        let score = (0..=4).find_map(|k| {
            let d = (at.0 + k - ux).abs().max((at.1 + k - uy).abs());
            (d <= 1).then_some((k, d))
        });
        if let Some(s) = score {
            if best.is_none_or(|(b, _)| s < b) {
                best = Some((s, *key));
            }
        }
    }
    best.map(|(_, k)| k)
}

/// The hostility test with (P, U) for a monster.
///
/// In town: 1 (no pet list in the preview). Out of town:
/// D2RUST's own, unverified (the relation flags, `alSel` / `noSel` and
/// the unit alignment are not in the model): a monster is hostile
/// unless its set-up `Align` is 1 (an ally); without set-up columns it
/// is hostile.
pub fn hostile(world: &ClientWorld, inputs: &ModelInputs, u: UnitKey) -> bool {
    if u.unit_type != MONSTER {
        return false;
    }
    if let Some(p) = world.local_player {
        if super::modes::in_town(world, p) {
            return true;
        }
    }
    let Some(unit) = world.units.get(&u) else {
        return false;
    };
    inputs
        .tables
        .monsters
        .get(unit.class as usize)
        .and_then(|c| c.as_ref())
        .and_then(|c| c.setup.as_ref())
        .is_none_or(|s| s.align != 1)
}

/// The hostility test with (P, U) between any two
/// client units, as the client missile code calls it.
///
/// Followed: P's room in town → 1 (the dead-player and pet clauses need
/// the party and `TargetPet` reads the model lacks); P = U → 0; a player
/// P and an object or item U → 1; the `alSel` / `noSel` clauses run
/// (measured with a Fire Bolt). PROVISIONAL (D2RUST's own, as
/// [`hostile`]; the pet's state-105 stat 172 := 2 is known, the arrow's
/// fate beside it is not: a draws comparison of the ticks around the
/// Teeth release):
/// the player-player relation flags (flag 8) read "not
/// hostile" (single player: no other player), and the alignment test
/// reads players and monsters whose set-up `Align` is 1 as
/// one side, every other monster as the other (the base game reads stat
/// 172 of the state-105 list; the model keeps none); units of other
/// types are not hostile.
pub fn hostile_between(
    world: &ClientWorld,
    monsters: &[Option<super::world::MonsterClass>],
    p: UnitKey,
    u: UnitKey,
) -> bool {
    use super::world::{ITEM, OBJECT, PLAYER};
    if super::modes::in_town(world, p) {
        return true;
    }
    if p == u {
        return false;
    }
    if p.unit_type == PLAYER && u.unit_type == PLAYER {
        return false;
    }
    if p.unit_type == PLAYER && matches!(u.unit_type, OBJECT | ITEM) {
        return true;
    }
    // before the alignment test: a monster U whose
    // `monstats2` `alSel` is set is hostile, one with `noSel` is not.
    if u.unit_type == super::world::MONSTER {
        let class = world
            .units
            .get(&u)
            .and_then(|x| monsters.get(x.class as usize))
            .and_then(|c| c.as_ref());
        if let Some(c) = class {
            if c.al_sel {
                return true;
            }
            if c.no_sel {
                return false;
            }
        }
    }
    let side = |k: UnitKey| match k.unit_type {
        PLAYER => Some(true),
        MONSTER => Some(
            world
                .units
                .get(&k)
                .and_then(|unit| monsters.get(unit.class as usize))
                .and_then(|c| c.as_ref())
                .and_then(|c| c.setup.as_ref())
                .is_some_and(|s| s.align == 1),
        ),
        _ => None,
    };
    matches!((side(p), side(u)), (Some(a), Some(b)) if a != b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{MonsterClass, PLAYER};

    fn world_with(units: &[(UnitKey, (u16, u16), u32)]) -> ClientWorld {
        let mut w = ClientWorld::default();
        for &(k, at, class) in units {
            let mut u = ClientUnit::new(k);
            u.position = Some(at);
            u.class = class;
            w.units.insert(k, u);
        }
        w
    }

    #[test]
    fn hover_picks_the_monster_under_the_click() {
        let m1 = UnitKey::new(MONSTER, 5);
        let m2 = UnitKey::new(MONSTER, 6);
        let npc = UnitKey::new(MONSTER, 7);
        let p = UnitKey::new(PLAYER, 1);
        let mut w = world_with(&[
            (m1, (100, 100), 0),
            (m2, (110, 100), 0),
            (npc, (120, 100), 1),
            (p, (90, 90), 0),
        ]);
        w.local_player = Some(p);
        let mut inputs = ModelInputs::default();
        inputs.tables.monsters = vec![
            Some(MonsterClass::default()),
            Some(MonsterClass {
                npc: true,
                ..MonsterClass::default()
            }),
        ];
        assert_eq!(hover_at(&w, &inputs, (100, 101)), Some(m1));
        // A click on the body: up-screen of the feet.
        assert_eq!(hover_at(&w, &inputs, (107, 97)), Some(m2));
        assert_eq!(
            hover_at(&w, &inputs, (120, 100)),
            None,
            "NPCs are not hovered"
        );
        assert_eq!(
            hover_at(&w, &inputs, (90, 90)),
            None,
            "the player is not a monster"
        );
        assert_eq!(hover_at(&w, &inputs, (105, 120)), None);
    }

    /// Measured in the Spider Forest: the Act3Female (class 296,
    /// `npc` 0, `isSel` 0) under the click is not hovered, so the click
    /// walks to the ground.
    #[test]
    fn a_monster_without_is_sel_is_not_hovered() {
        let m = UnitKey::new(MONSTER, 8);
        let mut w = world_with(&[(m, (100, 100), 0)]);
        let mut inputs = ModelInputs::default();
        inputs.tables.monsters = vec![Some(MonsterClass::default())];
        assert_eq!(hover_at(&w, &inputs, (100, 100)), Some(m));
        w.units.get_mut(&m).unwrap().flag_2 = Some(false);
        assert_eq!(hover_at(&w, &inputs, (100, 100)), None);
        let u = w.units.get_mut(&m).unwrap();
        u.flag_2 = Some(true);
        u.quest_untargetable = true;
        assert_eq!(hover_at(&w, &inputs, (100, 100)), None);
    }

    #[test]
    fn range_reads_both_by_the_bow_and_rng_by_the_melee_only_states() {
        let row = |r| SkillRow {
            range: r,
            ..SkillRow::default()
        };
        assert_eq!(skill_range(&row(1), false, false), range::H2H);
        assert_eq!(skill_range(&row(2), false, false), range::RNG);
        assert_eq!(skill_range(&row(3), false, false), range::H2H);
        assert_eq!(skill_range(&row(3), true, false), range::RNG);
        assert_eq!(skill_range(&row(4), false, true), range::LOC);
        assert_eq!(skill_range(&row(9), false, false), range::NONE);
        // A state of mask group 0x26: rng is h2h (also `both` with a bow).
        assert_eq!(skill_range(&row(2), false, true), range::H2H);
        assert_eq!(skill_range(&row(3), true, true), range::H2H);
        assert_eq!(skill_range(&row(1), false, true), range::H2H);
    }

    #[test]
    fn a_player_in_a_melee_only_state_uses_a_rng_skill_as_h2h() {
        use crate::bridge::world::StateRow;
        let mut inputs = ModelInputs::default();
        inputs.tables.skills = vec![
            SkillRow::default(),
            SkillRow {
                range: range::RNG,
                ..SkillRow::default()
            },
        ];
        inputs.tables.states = vec![StateRow::default(); 4];
        inputs.tables.states[3].meleeonly = true;
        let mut p = ClientUnit::new(UnitKey::new(PLAYER, 1));
        assert_eq!(range_of(&inputs, Some(&p), 1), range::RNG);
        // A state outside the group changes nothing.
        p.states.insert(2);
        assert_eq!(range_of(&inputs, Some(&p), 1), range::RNG);
        p.states.insert(3);
        assert_eq!(range_of(&inputs, Some(&p), 1), range::H2H);
        assert_eq!(range_of(&inputs, None, 1), range::RNG);
        assert_eq!(range_of(&inputs, Some(&p), 7), range::NONE);
    }
}
