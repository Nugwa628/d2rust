//! The overlays of state on and state off (the `removerlay` create and
//! the group removals). The model draws the ids (the `1ofN` roll on U's seed) and
//! records each call in [`ClientWorld::hit_overlays`]; the effect layer
//! makes the records, the lights are made here
//! ([`super::overlay_lights`]).

use super::hit_overlays::HitOverlay;
use super::world::{ClientWorld, ModelInputs, UnitKey};

/// With (id): the overlay's `1ofN` (+0x4C); 0 for an id with no
/// row.
fn one_of_n(inputs: &ModelInputs, id: i32) -> i32 {
    usize::try_from(id)
        .ok()
        .and_then(|i| inputs.tables.overlays.get(i))
        .map_or(0, |r| i32::try_from(r.one_of_n).unwrap_or(i32::MAX))
}

/// id + a roll of `1ofN(id)` on U's seed (a `1ofN` < 1 does not step).
fn pick(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, id: i32) -> i32 {
    let n = one_of_n(inputs, id);
    id.wrapping_add(super::client_do::unit_roll(w, key, n) as i32)
}

/// With (U): U has a state with the `aura` flag.
fn has_aura(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey) -> bool {
    w.units.get(&key).is_some_and(|u| {
        u.states.iter().any(|&s| {
            inputs
                .tables
                .states
                .get(usize::from(s))
                .is_some_and(|r| r.aura)
        })
    })
}

/// With (U, id, kind, 0, A, B, C, 0): recorded for the effect
/// layer, the light made here.
pub(super) fn create(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    id: i32,
    kind: u8,
    args: [i32; 3],
) {
    if let Ok(id16) = u16::try_from(id) {
        w.hit_overlays.push(HitOverlay {
            tick: w.server_ticks,
            unit: key,
            id: id16,
            kind,
            args,
            remove: false,
        });
    }
    super::overlay_lights::create_with(w, inputs, key, id, kind, args);
}

/// State on with (U, state) (after the colour call):
/// nothing for a `nooverlays` state. cast := `castoverlay`, ov1 :=
/// `overlay1`, ov2 := `overlay2` (negative → none); each one set, in
/// that order, adds a roll of its `1ofN` on U's seed. An `aura` state
/// with U holding another aura state (the state's own bit
/// cleared for the test, then set again) drops cast. Then
/// cast set → one create, kind 4 (A = ov1, B = ov2), or kind 9 (also C =
/// the state) for an aura state; else ov1 and ov2, each set, kind 3 (A
/// = 0), or kind 8 (A = the state) for an aura state.
pub fn on(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, state: u8) {
    let Some(row) = inputs.tables.states.get(usize::from(state)).copied() else {
        return;
    };
    if row.nooverlays || !w.units.contains_key(&key) {
        return;
    }
    let links = row.links;
    let mut ids = [links.cast, links.overlay1, links.overlay2].map(|v| i32::from(v.max(-1)));
    for id in &mut ids {
        if *id != -1 {
            *id = pick(w, inputs, key, *id);
        }
    }
    let [mut cast, ov1, ov2] = ids;
    if row.aura {
        let had = w
            .units
            .get_mut(&key)
            .is_some_and(|u| u.states.remove(&state));
        if has_aura(w, inputs, key) {
            cast = -1;
        }
        if had {
            if let Some(u) = w.units.get_mut(&key) {
                u.states.insert(state);
            }
        }
    }
    let s = i32::from(state);
    if cast != -1 {
        if row.aura {
            create(w, inputs, key, cast, 9, [ov1, ov2, s]);
        } else {
            create(w, inputs, key, cast, 4, [ov1, ov2, 0]);
        }
        return;
    }
    for id in [ov1, ov2] {
        if id == -1 {
            continue;
        }
        let a = if row.aura { s } else { 0 };
        create(w, inputs, key, id, if row.aura { 8 } else { 3 }, [a, 0, 0]);
    }
}

/// The overlays of state off (after the colour
/// call): `removerlay` ≥ 0 → id + a roll of its `1ofN` on U's seed,
/// create kind 4 with A = B = −1; then `overlay1`, `overlay2` ≥ 0 →:
/// the overlay removal with (U, base + i) for i in 0..`1ofN`(base).
pub fn off(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, state: u8) {
    let Some(row) = inputs.tables.states.get(usize::from(state)).copied() else {
        return;
    };
    if !w.units.contains_key(&key) {
        return;
    }
    let links = row.links;
    if links.remove >= 0 {
        let id = pick(w, inputs, key, i32::from(links.remove));
        create(w, inputs, key, id, 4, [-1, -1, 0]);
    }
    for base in [links.overlay1, links.overlay2] {
        if base < 0 {
            continue;
        }
        let base = i32::from(base);
        for i in 0..one_of_n(inputs, base) {
            let Ok(id) = u16::try_from(base + i) else {
                break;
            };
            w.hit_overlays.push(HitOverlay {
                tick: w.server_ticks,
                unit: key,
                id,
                kind: 0,
                args: [0; 3],
                remove: true,
            });
            super::overlay_lights::remove_by_id(w, key, id);
        }
    }
}

/// With (U, 0, id): the overlay record `id` of U removed.
fn remove_one(w: &mut ClientWorld, key: UnitKey, id: i32) {
    let Ok(id) = u16::try_from(id) else {
        return;
    };
    w.hit_overlays.push(HitOverlay {
        tick: w.server_ticks,
        unit: key,
        id,
        kind: 0,
        args: [0; 3],
        remove: true,
    });
    super::overlay_lights::remove_by_id(w, key, id);
}

/// The guard of setfunc 1 and remfunc 1:
/// the state's `pgsv` flag, 0 < `pgsvoverlay` <
/// the overlay count and 0 ≤ `stat` ≤ the `itemstatcost` count (≤ as
/// the base game); the overlay base and the stat.
fn pgsv_row(inputs: &ModelInputs, state: u8) -> Option<(i32, u16)> {
    let row = inputs.tables.states.get(usize::from(state))?;
    let base = i32::from(row.pgsv_overlay);
    // The overlay count `[+0xBC0]`: one row per `overlay` row.
    let count = i32::try_from(inputs.tables.overlays.len()).unwrap_or(i32::MAX);
    let stat = u16::try_from(row.stat).ok()?;
    (row.pgsv && base > 0 && base < count && usize::from(stat) <= inputs.tables.stats.len())
        .then_some((base, stat))
}

/// Setfunc 1 (the state on hooks): the progressive
/// overlay. Guard passed → remove `pgsvoverlay` + 0, 1, 2; v := U's
/// total of the state's `stat`; 1 ≤ v ≤ 3 →
/// create `pgsvoverlay` + v − 1, kind 3 (the charge count's overlay,
/// e.g. `progressive_damage_3` at three Tiger Strike charges).
pub fn pgsv_set(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, state: u8) {
    let Some((base, stat)) = pgsv_row(inputs, state) else {
        return;
    };
    for i in 0..3 {
        remove_one(w, key, base + i);
    }
    let v = w.total(key, stat, 0);
    if (1..=3).contains(&v) {
        create(w, inputs, key, base + v - 1, 3, [0; 3]);
    }
}

/// Remfunc 1 (state off before the bit is
/// cleared): setfunc 1's guard → remove `pgsvoverlay` + 0, 1, 2.
pub fn pgsv_rem(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, state: u8) {
    let Some((base, _)) = pgsv_row(inputs, state) else {
        return;
    };
    for i in 0..3 {
        remove_one(w, key, base + i);
    }
}

#[cfg(test)]
#[path = "state_overlays_tests.rs"]
mod tests;
