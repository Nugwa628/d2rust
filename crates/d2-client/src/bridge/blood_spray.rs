//! The client blood spray with (U, n, roll): up to n blood
//! missiles (`monstats2` `Bleed` picks the class) fired from U to the
//! free cells around it. `cltdofunc` 38 / 40 (`client_do`) run it on
//! the corpse they target with n = 5.
//!
//! Not wired here: the monster update codes 0x06 (n = 1, no roll) and
//! 0x13 (n = 1, roll) and setfunc 18 call it too; the model's
//! handlers of those do not yet.

use super::client_do::create_toward;
use super::critter_path::{self, Who};
use super::dispatch::HandlerError;
use super::world::{ClientWorld, ModelInputs, UnitKey, ITEM, MONSTER, OBJECT, TILE};

/// Stat 140 `item_extrablood`: replaces n when non-zero.
const STAT_EXTRABLOOD: u16 = 140;

/// The first blood missile class (class 0x12 + r).
const FIRST_CLASS: i32 = 0x12;

/// The direction steps: x and y.
const DX: [i32; 8] = [0, -1, -2, -1, 0, 1, 2, 1];
const DY: [i32; 8] = [2, 1, 0, -1, -2, -1, 0, 1];

/// The monster mode the second gate draws in (GH).
const MODE_GET_HIT: u32 = 3;

/// One draw on U's seed: the new low word. No seed: 0
/// and no draw.
fn draw(w: &mut ClientWorld, key: UnitKey) -> u32 {
    let Some(u) = w.units.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut seed = d2_sim::rng::Seed::new(lo, hi);
    let r = seed.step();
    u.seed = Some((seed.lo, seed.hi));
    r
}

/// U's cell as the spray reads it: an object, item or tile its static
/// position, a monster its path cell,
/// else the model's cell.
fn origin(w: &ClientWorld, key: UnitKey) -> (i32, i32) {
    let unit_cell = || {
        w.units.get(&key).map_or((0, 0), |u| {
            let (x, y) = u.cell();
            (i32::from(x), i32::from(y))
        })
    };
    match key.unit_type {
        OBJECT | ITEM | TILE => unit_cell(),
        MONSTER => critter_path::cell(w, Who::server(key)).unwrap_or_else(unit_cell),
        _ => unit_cell(),
    }
}

/// The blood spray with (U, n, roll).
pub fn spray(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    n: i32,
    roll: bool,
) -> Result<(), HandlerError> {
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    let (class, mode, extra) = (u.class, u.mode, u.stat(STAT_EXTRABLOOD));
    // The gates: 1 in 25 with the roll, 1 in 3 for a monster in GH.
    if roll && !draw(w, key).is_multiple_of(25) {
        return Ok(());
    }
    if key.unit_type == MONSTER && mode == MODE_GET_HIT && !draw(w, key).is_multiple_of(3) {
        return Ok(());
    }
    let n = if extra != 0 { extra } else { n };
    // U's class read as a `monstats` row, whatever U's type
    // (`param_1[1]`), then its `monstats2` `Bleed`.
    let b = inputs
        .tables
        .monsters
        .get(class as usize)
        .copied()
        .flatten()
        .map_or(0, |c| c.bleed);
    if b == 0 || n <= 0 {
        return Ok(());
    }
    let span = u32::from(b) * 2;
    for _ in 0..n {
        let moving = key.unit_type == MONSTER
            && critter_path::velocity(w, Who::server(key)).is_some_and(|v| v != 0);
        let d0 = if moving {
            let facing = critter_path::direction(w, Who::server(key)).unwrap_or(0);
            let r = draw(w, key);
            // r + r / 7 − 3 + facing / 8: r % 7 − 3 + facing / 8 mod 8.
            (u32::from(facing) >> 3)
                .wrapping_add(r)
                .wrapping_add(r / 7)
                .wrapping_sub(3)
                & 7
        } else {
            draw(w, key) & 7
        };
        // From d0, at most 7 directions; d0 − 1 is never tried.
        let (x, y) = origin(w, key);
        let stop = d0.wrapping_sub(1) & 7;
        let mut d = d0;
        let cell = loop {
            let (cx, cy) = (x + DX[d as usize], y + DY[d as usize]);
            let blocked = super::client_missiles::unit_room_query(w, key, cx, cy, 1, 1) != Some(0);
            if !blocked {
                break (cx, cy);
            }
            d = (d + 1) & 7;
            if d == stop {
                return Ok(());
            }
        };
        // The class draw: & (2b − 1) for a power of two, else % 2b.
        let r = draw(w, key);
        let pick = if span.is_power_of_two() {
            r & (span - 1)
        } else {
            r % span
        };
        create_toward(w, inputs, key, FIRST_CLASS + pick as i32, cell, 0, 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, MonsterClass};

    fn world_with(seed: (u32, u32), mode: u32, bleed: u8) -> (ClientWorld, ModelInputs, UnitKey) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(MONSTER, 7);
        let mut u = ClientUnit::new(key);
        u.class = 5;
        u.position = Some((100, 100));
        u.mode = mode;
        u.seed = Some(seed);
        w.units.insert(key, u);
        let mut inputs = ModelInputs::default();
        inputs.tables.monsters = vec![None; 6];
        inputs.tables.monsters[5] = Some(MonsterClass {
            bleed,
            ..MonsterClass::default()
        });
        (w, inputs, key)
    }

    fn seed_of(w: &ClientWorld, key: UnitKey) -> (u32, u32) {
        w.units[&key].seed.unwrap()
    }

    fn steps(seed: (u32, u32), k: usize) -> (u32, u32) {
        let mut s = d2_sim::rng::Seed::new(seed.0, seed.1);
        for _ in 0..k {
            s.step();
        }
        (s.lo, s.hi)
    }

    #[test]
    fn no_bleed_draws_nothing_without_gates() {
        let (mut w, inputs, key) = world_with((1234, 666), 12, 0);
        spray(&mut w, &inputs, key, 5, false).unwrap();
        assert_eq!(seed_of(&w, key), (1234, 666));
    }

    #[test]
    fn get_hit_gate_draws_once_even_without_bleed() {
        let (mut w, inputs, key) = world_with((1234, 666), MODE_GET_HIT, 0);
        spray(&mut w, &inputs, key, 1, false).unwrap();
        assert_eq!(seed_of(&w, key), steps((1234, 666), 1));
    }

    #[test]
    fn each_missile_draws_direction_then_class() {
        // No client DRLG: every cell is free, so each of the n missiles
        // takes two draws (direction, class) and one create.
        let (mut w, inputs, key) = world_with((1234, 666), 12, 2);
        spray(&mut w, &inputs, key, 5, false).unwrap();
        assert_eq!(seed_of(&w, key), steps((1234, 666), 10));
    }

    #[test]
    fn extrablood_stat_replaces_n() {
        let (mut w, inputs, key) = world_with((1234, 666), 12, 1);
        w.units
            .get_mut(&key)
            .unwrap()
            .stats
            .insert(STAT_EXTRABLOOD, 2);
        spray(&mut w, &inputs, key, 5, false).unwrap();
        assert_eq!(seed_of(&w, key), steps((1234, 666), 4));
    }

    #[test]
    fn roll_gate_stops_unless_a_multiple_of_25() {
        let (mut w, inputs, key) = world_with((1234, 666), 12, 2);
        let mut s = d2_sim::rng::Seed::new(1234, 666);
        let first = s.step();
        spray(&mut w, &inputs, key, 1, true).unwrap();
        let want = if first.is_multiple_of(25) { 3 } else { 1 };
        assert_eq!(seed_of(&w, key), steps((1234, 666), want));
    }
}
