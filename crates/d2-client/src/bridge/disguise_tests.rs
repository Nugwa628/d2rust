//! Tests of the disguise bit, the draw identity
//! and the client total's
//! percent ops.

use super::disguise::{identity, toggle, FLAG_EX_DISGUISE};
use super::world::{
    ClientUnit, ClientWorld, ModelInputs, MonsterClass, StateRow, UnitKey, MONSTER, PLAYER,
};

const P: UnitKey = UnitKey::new(PLAYER, 1);
const WOLF: u8 = 139;
const CLASS: u16 = 7;

/// State 139 a `disguise` state shown as monster class 7 (`gfxtype` 1),
/// state 140 a second `disguise` state without a gfx type; class 7 has
/// the monster modes NU, WL and A1 only.
fn inputs() -> ModelInputs {
    let mut i = ModelInputs::default();
    i.tables.states = vec![StateRow::default(); 141];
    i.tables.states[usize::from(WOLF)] = StateRow {
        disguise: true,
        gfxtype: 1,
        gfxclass: CLASS,
        ..StateRow::default()
    };
    i.tables.states[140].disguise = true;
    i.tables.monsters = vec![None; 8];
    i.tables.monsters[usize::from(CLASS)] = Some(MonsterClass {
        modes: (1 << 1) | (1 << 2) | (1 << 4),
        ..MonsterClass::default()
    });
    i
}

fn world() -> ClientWorld {
    let mut w = ClientWorld::default();
    w.units.insert(P, ClientUnit::new(P));
    w
}

fn set_state(w: &mut ClientWorld, i: &ModelInputs, s: u8, on: bool) {
    let u = w.units.get_mut(&P).unwrap();
    if on {
        u.states.insert(s);
    } else {
        u.states.remove(&s);
    }
    toggle(w, i, P, s, on);
}

#[test]
fn disguise_bit_follows_the_last_disguise_state() {
    let i = inputs();
    let mut w = world();
    set_state(&mut w, &i, 3, true);
    assert_eq!(w.units[&P].flag_ex & FLAG_EX_DISGUISE, 0);
    set_state(&mut w, &i, WOLF, true);
    set_state(&mut w, &i, 140, true);
    assert_ne!(w.units[&P].flag_ex & FLAG_EX_DISGUISE, 0);
    set_state(&mut w, &i, WOLF, false);
    assert_ne!(w.units[&P].flag_ex & FLAG_EX_DISGUISE, 0);
    set_state(&mut w, &i, 140, false);
    assert_eq!(w.units[&P].flag_ex & FLAG_EX_DISGUISE, 0);
}

#[test]
fn identity_maps_player_modes_to_the_class_modes() {
    let i = inputs();
    let mut w = world();
    // No disguise bit: the unit's own identity.
    assert_eq!(identity(&w, &i, P, 3), (PLAYER, 0, 3));
    set_state(&mut w, &i, WOLF, true);
    let c = u32::from(CLASS);
    // Walk (2) → WL (2), held by the class.
    assert_eq!(identity(&w, &i, P, 2), (MONSTER, c, 2));
    // Run (3) → RN (15), missing → WL (2).
    assert_eq!(identity(&w, &i, P, 3), (MONSTER, c, 2));
    // Attack 2 (8) → A2 (5), missing → A1 (4).
    assert_eq!(identity(&w, &i, P, 8), (MONSTER, c, 4));
    // Cast (10) → SC (7), missing → A1 (4).
    assert_eq!(identity(&w, &i, P, 10), (MONSTER, c, 4));
    // Knockback (0x12) → SQ (14), missing → NU (1).
    assert_eq!(identity(&w, &i, P, 0x12), (MONSTER, c, 1));
}
