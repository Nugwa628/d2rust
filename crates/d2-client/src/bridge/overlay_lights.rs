//! The light of a unit's overlay records (create,
//! advance, removal). The bridge keeps only
//! the records that carry a light (`Radius` ≠ 0) and those of kinds 4
//! and 9, whose follow-ups may carry one: an other overlay without a
//! light changes nothing the model or the light map reads. Their drawing
//! stays in the render layer ([`crate::world_view::missiles`]).
//!
//! Callers: the client skill start (`castoverlay` +0x106 → the overlay
//! with (U, o, 2, …), before the cast light), the hit overlays
//! ([`super::modes::hit_overlays`]) and the
//! state overlays ([`super::state_overlays`]).

pub use crate::app::single_player::rows::OverlayLightRow;

use super::world::{ClientWorld, ModelInputs, UnitKey, MONSTER};
use crate::rules::lighting::records::{unit_light_pos, LightId, LightKind, Owner};

/// One overlay record of a unit that carries a light, or of kind 4 / 9
/// (their follow-ups may carry one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayLight {
    pub unit: UnitKey,
    /// +0x00 kind.
    pub kind: u8,
    /// +0x04 overlay id.
    pub id: u16,
    /// +0x08 (kind 2: 1 while playing, 0 once the last frame passed;
    /// kind 4 / 9: the first follow-up).
    pub live: i32,
    /// +0x0C arg B (kind 4 / 9: the second follow-up).
    pub b: i32,
    /// +0x10 arg C (kind 9), or +0x38 the state (kind 8).
    pub state: i32,
    /// +0x14 step, +0x18 frame, +0x1C end, all in 1/256 frame.
    pub step: i32,
    pub frame: i32,
    pub end: i32,
    /// +0x40 the light (`None`: `Radius` 0).
    pub light: Option<LightId>,
    /// +0x20 the update stamp of the walk.
    pub stamp: u64,
}

/// With (U, id): U has a record of this id. Only lit records and
/// kinds 4 / 9 are kept, so an other unlit one of the same id is not
/// seen (it would make the skill start skip a create that adds no light
/// either way).
pub fn has(w: &ClientWorld, key: UnitKey, id: u16) -> bool {
    w.overlay_lights.iter().any(|o| o.unit == key && o.id == id)
}

/// The overlay create with (U, id, kind, 0, 0, 0, 0, 0); see [`create_with`].
pub fn create(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, id: u16, kind: u8) {
    create_with(w, inputs, key, i32::from(id), kind, [0; 3]);
}

/// The overlay create with (U, id, kind, 0, A, B, C, 0) for the records that carry
/// a light: (id in range), (classic install and `version` ≥ 100),
/// (U's flag-ex bit 0x40000; a monster whose `monstats2` row has
/// `noOvly`), (kind 8 with a record of the id: none),
/// (by id, except ids 140, 141, 142, 158), (A, B, C;
/// kind 8 keeps A as its state), (the light, kind 1 at U's
/// sub-tile, radius `InitRadius`, `I` 255, target `Radius` when it
/// differs), (pushed at the list head), (step `AnimRate` × 16).
pub fn create_with(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    id: i32,
    kind: u8,
    [arg_a, arg_b, arg_c]: [i32; 3],
) {
    let Some(row) = usize::try_from(id)
        .ok()
        .and_then(|i| inputs.tables.overlays.get(i).copied())
    else {
        return;
    };
    let Ok(id) = u16::try_from(id) else {
        return;
    };
    if !inputs.expansion_installed && row.version >= 100 {
        return;
    }
    let Some(unit) = w.units.get(&key) else {
        return;
    };
    if unit.flag_ex & 0x40000 != 0 {
        return;
    }
    if key.unit_type == MONSTER
        && inputs
            .tables
            .monsters
            .get(unit.class as usize)
            .and_then(|c| c.as_ref())
            .is_some_and(|c| c.no_ovly)
    {
        return;
    }
    let cell = unit.cell();
    if kind == 8 && has(w, key, id) {
        return;
    }
    if !matches!(id, 140 | 141 | 142 | 158) {
        remove_by_id(w, key, id);
    }
    if row.radius == 0 && !matches!(kind, 4 | 9) {
        return;
    }
    let light = if row.radius == 0 {
        None
    } else {
        let (x, y) = cell;
        let owner = Owner {
            unit_type: u32::from(key.unit_type),
            guid: key.guid,
            client_only: false,
        };
        let pos = (
            unit_light_pos(i32::from(x) << 16),
            unit_light_pos(i32::from(y) << 16),
        );
        let (r, g, b) = row.rgb;
        let radius = i32::try_from(row.init_radius).unwrap_or(i32::MAX);
        let Some(light) =
            w.lights
                .create(Some(owner), pos, LightKind::Plain, radius, 0xFF, r, g, b)
        else {
            return;
        };
        if row.radius != row.init_radius {
            w.lights
                .set_target(light, i32::try_from(row.radius).unwrap_or(i32::MAX));
        }
        Some(light)
    };
    let frames = i32::try_from(row.frames.max(1)).unwrap_or(i32::MAX / 256);
    let (live, b, state) = match kind {
        2 => (1, 0, 0),
        8 => (arg_a, arg_b, arg_a),
        _ => (arg_a, arg_b, arg_c),
    };
    w.overlay_lights.insert(
        0,
        OverlayLight {
            unit: key,
            kind,
            id,
            live,
            b,
            state,
            step: i32::try_from(row.anim_rate).unwrap_or(0).saturating_mul(16),
            frame: 0,
            end: frames.saturating_mul(256),
            light,
            stamp: 0,
        },
    );
}

/// With (U, id): the first record of U in list order with
/// the id, or of kind 4 / 9 with A or B = id, leaves the list.
pub fn remove_by_id(w: &mut ClientWorld, key: UnitKey, id: u16) {
    let id32 = i32::from(id);
    if let Some(i) = w.overlay_lights.iter().position(|o| {
        o.unit == key
            && (o.id == id || (matches!(o.kind, 4 | 9) && (o.live == id32 || o.b == id32)))
    }) {
        remove_at(w, i);
    }
}

/// The overlay advance of U, the kept records of U in list order:
/// each record not yet stamped with this update runs once, a record
/// created during the walk too (it goes to the head). Kind 1 counts
/// down, kinds 1 / 2 with +0x08 = 0 and kind 8 without its state go;
/// then the advance and the end: kinds 1 / 3 / 8 wrap,
/// kind 2 sets +0x08 := 0, kind 4 goes and makes A, B ≠ −1 as kind 3,
/// kind 9 goes and, its state C still on, makes them as kind 8 (state
/// C). Kinds 0, 5, 6, 7 have no caller here and no end rule.
pub fn advance(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    // The base game stamps with a counter of updates; a fresh mark per
    // call is the same walk.
    w.overlay_stamp = w.overlay_stamp.wrapping_add(1);
    let stamp = w.overlay_stamp;
    // The step is 8.8 frames per base-rate tick: this tick's share of it.
    let (rate, tick) = (w.tick_rate, w.share_frame());
    while let Some(i) = w
        .overlay_lights
        .iter()
        .position(|o| o.unit == key && o.stamp != stamp)
    {
        w.overlay_lights[i].stamp = stamp;
        let o = w.overlay_lights[i];
        let gone = match o.kind {
            1 if o.live != 0 => {
                w.overlay_lights[i].live -= 1;
                false
            }
            1 | 2 => o.live == 0,
            8 => !has_state(w, key, o.state),
            _ => false,
        };
        if gone {
            remove_at(w, i);
            continue;
        }
        let o = &mut w.overlay_lights[i];
        o.frame = o.frame.saturating_add(rate.share(o.step, tick));
        if o.frame < o.end {
            continue;
        }
        let o = *o;
        match o.kind {
            1 | 3 | 8 => w.overlay_lights[i].frame -= o.end,
            2 => w.overlay_lights[i].live = 0,
            4 => {
                remove_at(w, i);
                for id in [o.live, o.b].into_iter().filter(|&id| id != -1) {
                    create_with(w, inputs, key, id, 3, [0; 3]);
                }
            }
            9 => {
                remove_at(w, i);
                if has_state(w, key, o.state) {
                    for id in [o.live, o.b].into_iter().filter(|&id| id != -1) {
                        create_with(w, inputs, key, id, 8, [o.state, 0, 0]);
                    }
                }
            }
            _ => {}
        }
    }
}

fn has_state(w: &ClientWorld, key: UnitKey, s: i32) -> bool {
    u8::try_from(s).is_ok_and(|s| w.units.get(&key).is_some_and(|u| u.states.contains(&s)))
}

/// The record leaves the list and its light is freed.
fn remove_at(w: &mut ClientWorld, i: usize) {
    let o = w.overlay_lights.remove(i);
    if let Some(light) = o.light {
        let _ = w.lights.remove(light);
    }
}

/// With (U): every record of U leaves the list, in list
/// order.
pub fn remove_all(w: &mut ClientWorld, key: UnitKey) {
    while let Some(i) = w.overlay_lights.iter().position(|o| o.unit == key) {
        remove_at(w, i);
    }
}

/// The unit free: its records go with it (their lights go with the
/// unit's owned lights).
pub fn forget(w: &mut ClientWorld, key: UnitKey) {
    w.overlay_lights.retain(|o| o.unit != key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, PLAYER};

    const P: UnitKey = UnitKey::new(PLAYER, 1);

    /// Overlay 5: 14 frames at rate 16 (step 256, one frame per update),
    /// radius 3 → 6, colour (255, 128, 0). Overlay 6: no radius.
    fn inputs() -> ModelInputs {
        let mut i = ModelInputs {
            expansion_installed: true,
            ..ModelInputs::default()
        };
        i.tables.overlays = vec![OverlayLightRow::default(); 7];
        i.tables.overlays[5] = OverlayLightRow {
            version: 100,
            frames: 14,
            anim_rate: 16,
            init_radius: 3,
            radius: 6,
            rgb: (255, 128, 0),
            ..OverlayLightRow::default()
        };
        i.tables.overlays[6] = OverlayLightRow {
            frames: 14,
            anim_rate: 16,
            ..OverlayLightRow::default()
        };
        i
    }

    fn world() -> ClientWorld {
        let mut w = ClientWorld::default();
        w.units.insert(P, ClientUnit::new(P));
        w
    }

    #[test]
    fn create_lights_the_unit_with_init_radius_and_target() {
        let mut w = world();
        create(&mut w, &inputs(), P, 5, 2);
        assert_eq!(w.overlay_lights.len(), 1);
        let o = w.overlay_lights[0];
        assert_eq!(
            (o.id, o.kind, o.live, o.step, o.end),
            (5, 2, 1, 256, 14 * 256)
        );
        let l = w.lights.get(o.light.unwrap()).unwrap();
        assert_eq!((l.radius, l.target), (3 * 8, 6 * 8));
        assert_eq!((l.i, l.r, l.g, l.b), (0xFF, 255, 128, 0));
        assert_eq!((l.owner_type, l.owner_guid), (u32::from(PLAYER), 1));
        assert!(has(&w, P, 5));
    }

    #[test]
    fn no_light_without_radius_or_on_classic() {
        let mut w = world();
        create(&mut w, &inputs(), P, 6, 2);
        let mut classic = inputs();
        classic.expansion_installed = false;
        create(&mut w, &classic, P, 5, 2);
        assert!(w.overlay_lights.is_empty());
        assert_eq!(w.lights.len(), 0);
    }

    #[test]
    fn same_id_replaces_the_record_and_its_light() {
        let mut w = world();
        let i = inputs();
        create(&mut w, &i, P, 5, 2);
        let first = w.overlay_lights[0].light.unwrap();
        create(&mut w, &i, P, 5, 2);
        assert_eq!(w.overlay_lights.len(), 1);
        assert!(w.lights.get(first).is_none());
        assert_eq!(w.lights.len(), 1);
    }

    #[test]
    fn kind_two_ends_after_its_frames_then_frees_the_light() {
        let mut w = world();
        create(&mut w, &inputs(), P, 5, 2);
        let light = w.overlay_lights[0].light.unwrap();
        for _ in 0..13 {
            advance(&mut w, &inputs(), P);
        }
        assert_eq!(w.overlay_lights[0].live, 1);
        advance(&mut w, &inputs(), P);
        assert_eq!(w.overlay_lights[0].live, 0);
        assert!(w.lights.get(light).is_some());
        advance(&mut w, &inputs(), P);
        assert!(w.overlay_lights.is_empty());
        assert!(w.lights.get(light).is_none());
    }

    /// The step is per base-rate tick: a looping overlay (kind 3) plays
    /// the same frames a second at 25 Hz and at the game's 60 Hz (F5).
    #[test]
    fn a_looping_overlay_plays_in_real_time_at_every_rate() {
        for hz in [25u32, 60] {
            let mut w = world();
            w.tick_rate = d2_sim::tick::TickRate::new(hz).unwrap();
            // 14 frames at one a base tick: one second wraps once (25 − 14).
            create_with(&mut w, &inputs(), P, 5, 3, [0; 3]);
            for t in 0..hz as i32 {
                w.server_frame = Some(t);
                advance(&mut w, &inputs(), P);
            }
            assert_eq!(w.overlay_lights[0].frame, (25 - 14) * 256, "{hz} Hz");
        }
    }
}
