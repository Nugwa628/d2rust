//! The environment record of the client act: created with the act
//! (0x03, [`create_environment`]), set by S→C 0x53 ([`darkness`]) and by
//! the eclipse of S→C 0x5D ([`eclipse`]) and 0x5A code 0x12
//! ([`event_eclipse`]); S→C 0x89 ([`unique_event`]) sets the scripted
//! override globals. The record and its rules are
//! `crate::rules::lighting::environment`; the handlers here pick its
//! inputs from the model.

use super::super::dispatch::{HandlerError, Message};
use super::super::world::{ClientWorld, UnitKey, OBJECT};
use super::Bytes;
use crate::rules::lighting::environment::{
    act_index, EnvError, Environment, PeriodTables, ECLIPSE_SET,
};
use crate::rules::lighting::records::{unit_light_pos, LightKind, Owner};
use crate::rules::lighting::sources::{self, ObjectLight};

/// The period tables of `render/env-periods.tsv`, parsed once.
pub(crate) fn periods() -> Result<PeriodTables, HandlerError> {
    static TABLES: std::sync::OnceLock<Option<PeriodTables>> = std::sync::OnceLock::new();
    TABLES
        .get_or_init(|| PeriodTables::builtin().ok())
        .ok_or(HandlerError::Invalid("render/env-periods.tsv"))
}

/// The lighting part of a client update (once per client
/// update, never per drawn frame), with `L` = the local player's room's
/// level id (0 when none):
/// 1. the scripted overrides: the darkness step
///    (base = the room ambient without override),
///    then the Den and levels 107/108 counters; the Den counter passing
///    29 places the Den lights in every loaded level-8 room
///    ([`den_lights`]);
/// 2. the Den lights of level-8 rooms loaded since the last update once
///    the Den flag is set;
/// 3. the environment record.
///
/// The base game runs this at its own point and does not order it
/// against the lights or the unit walk; neither reads the other's
/// state. D2RUST's own, unverified: runs at the room load;
/// here the rooms loaded since the previous client
/// update get their lights in this update's lighting part (nothing reads
/// the lights in between).
pub fn lighting_update(
    w: &mut ClientWorld,
    inputs: &super::super::world::ModelInputs,
) -> Result<(), HandlerError> {
    let levels = &inputs.tables.levels;
    let level = w.player_level().map_or(0, u32::from);
    let advance = w.tick_rate.share(1, w.share_frame()) != 0;
    if advance && w.overrides.darkness.is_some() {
        let defs = levels
            .get(level as usize)
            .map(|r| r.ambient)
            .unwrap_or_default();
        let base = match &w.environment {
            Some(env) => {
                crate::rules::lighting::environment::room_ambient_without_override(defs, env)
            }
            None => defs,
        };
        w.overrides.update_darkness(base, level);
    }
    let loaded = std::mem::take(&mut w.rooms_loaded);
    if advance && w.overrides.update_counters() {
        // Every loaded level-8 room.
        let rooms: Vec<_> = w
            .active_rooms
            .iter()
            .flatten()
            .filter(|r| u32::from(r.level) == LEVEL_DEN_OF_EVIL)
            .copied()
            .collect();
        for r in rooms {
            den_lights(w, inputs, &r)?;
        }
    } else {
        // A level-8 room loaded once the flag is set.
        for room in loaded {
            let Some(r) = w
                .active_rooms
                .iter()
                .flatten()
                .find(|r| r.room == room)
                .copied()
            else {
                continue;
            };
            if w.overrides.room_load_gets_den_lights(u32::from(r.level)) {
                den_lights(w, inputs, &r)?;
            }
        }
    }
    let (rate, frame) = (w.tick_rate, w.share_frame());
    if let Some(env) = w.environment.as_mut() {
        env.update_at(&periods()?, level, rate, frame);
    }
    Ok(())
}

/// The level whose rooms get Den lights.
const LEVEL_DEN_OF_EVIL: u32 = crate::rules::lighting::overrides::LEVEL_DEN_OF_EVIL;

/// The Den lights of one room: up to 25 tries
/// until 3 are placed, each try two steps of the local player unit's
/// seed (x = room x + rnd(w), y = room y + rnd(h)); a point whose
/// collision point test with mask 5 is 0 becomes client missile 287
/// `denofevillight` ([`super::super::client_missiles::create`]). No local
/// player or seed: nothing (the draws need the seed).
///
/// The create record: flags 0x2003, owner the
/// local player unit, class 287, level 1; no origin, target or skill.
/// A point in no room of the client DRLG tests as blocked (the point
/// test's no-room answer).
pub fn den_lights(
    w: &mut ClientWorld,
    inputs: &super::super::world::ModelInputs,
    room: &super::super::world::ActiveRoom,
) -> Result<(), HandlerError> {
    use super::super::client_missiles::{create, flag, CreateRecord};
    use crate::rules::lighting::overrides::{den_light_points, DEN_LIGHT_MISSILE};
    let Some(key) = w.local_player else {
        return Ok(());
    };
    let Some((lo, hi)) = w.units.get(&key).and_then(|u| u.seed) else {
        return Ok(());
    };
    let mut seed = d2_sim::rng::Seed::new(lo, hi);
    let drlg = w.drlg.as_ref();
    let points = den_light_points((room.x0, room.y0, room.w, room.h), &mut seed, |x, y| {
        drlg.and_then(|d| d.drlg.collision_at(x, y))
            .map_or(0x27, u32::from)
            & 5
    });
    if let Some(u) = w.units.get_mut(&key) {
        u.seed = Some((seed.lo, seed.hi));
    }
    for (x, y) in points {
        let rec = CreateRecord {
            flags: flag::POSITION | flag::TARGET_RELATIVE | flag::RANDOM_DIRECTION,
            owner: Some(key),
            class: DEN_LIGHT_MISSILE,
            x,
            y,
            level: 1,
            ..CreateRecord::default()
        };
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    Ok(())
}

impl From<EnvError> for HandlerError {
    fn from(e: EnvError) -> Self {
        match e {
            EnvError::BadIndex(_) => HandlerError::Invalid("0x53: period index outside 0..=5"),
            EnvError::NegativeTicks(_) => HandlerError::Invalid("0x53: negative ticks"),
        }
    }
}

/// The act's environment record at creation.
/// `GetTickCount` at creation (+0x10) has no client reader and
/// the bridge reads no clock: 0.
pub fn create_environment() -> Result<Environment, HandlerError> {
    Ok(Environment::new(&periods()?, 0))
}

/// The act load's eclipse: with the pending flag
/// set and act 2 (byte 1) loaded, the setter runs with the eclipse.
pub fn act_load_eclipse(w: &mut ClientWorld, act: u8) -> Result<(), HandlerError> {
    if !(w.eclipse_pending && act == 1) {
        return Ok(());
    }
    set_eclipse(w)
}

/// S→C 0x5A code 0x12: with a client act,
/// the setter with (act, room of the local player, 5, 0, 1).
pub fn event_eclipse(w: &mut ClientWorld) -> Result<(), HandlerError> {
    if w.act.is_none() {
        return Ok(());
    }
    set_eclipse(w)
}

/// S→C 0x89 UniqueEvent: id u8@1;
/// id ≥ 32 is fatal 0x1D9, ≥ 20 fatal 0x1DA; else the id's bit and its
/// handler's override state (`crate::rules::lighting::overrides`).
/// The client effects of ids 1, 3, 6, 12 (level 108), 14, 16, 17, 19
/// (client missiles) are Phase 6 effects with no
/// output row.
pub fn unique_event(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 2 {
        return Err(HandlerError::Invalid("0x89 is 2 bytes"));
    }
    let id = b.u8(1)?;
    match id {
        32.. => return Err(HandlerError::Fatal(0x1D9)),
        20.. => return Err(HandlerError::Fatal(0x1DA)),
        _ => {}
    }
    let level = w.player_level().map_or(0, u32::from);
    // Id 13: the clock + 90: wall-clock
    // seconds, no RNG draw (the host's input).
    let now = msg.inputs.wall_seconds.map_or(0, |f| f());
    w.overrides
        .unique_event(id, level, || now)
        .map_err(|_| HandlerError::Invalid("0x89 id"))?;
    Ok(())
}

fn set_eclipse(w: &mut ClientWorld) -> Result<(), HandlerError> {
    let level = w.player_level().map_or(0, u32::from);
    let env = w.environment.as_mut().ok_or(HandlerError::Invalid(
        "client act without its environment record",
    ))?;
    let (index, ticks, eclipse) = ECLIPSE_SET;
    env.set_from_server(&periods()?, index, ticks, eclipse, act_index(level), level)?;
    Ok(())
}

/// The eclipse of S→C 0x5D: with a client act the
/// setter runs now with index 5, ticks 0, eclipse 1; without one the
/// pending flag is set.
pub fn eclipse(w: &mut ClientWorld) -> Result<(), HandlerError> {
    if w.act.is_none() {
        w.eclipse_pending = true;
        return Ok(());
    }
    set_eclipse(w)
}

/// The object day/night refresh: every object of set S in
/// bucket order (`GUID & 0x7F` ascending, each chain in descending GUID)
/// gets with (obj, 0) (open question 11).
fn day_refresh(w: &mut ClientWorld, msg: &Message<'_>, p: i32) -> Result<(), HandlerError> {
    let mut objects: Vec<UnitKey> = w
        .units
        .keys()
        .filter(|k| k.unit_type == OBJECT)
        .copied()
        .collect();
    objects.sort_by_key(|k| (k.guid & 0x7F, std::cmp::Reverse(k.guid)));
    for key in objects {
        object_env_refresh(w, msg, key, p)?;
    }
    Ok(())
}

/// The day-period refresh of one object: only objects
/// whose `EnvEffect` ≠ 0 change. p 1–3: mode 0 → mode 1 and flag 0x2 :=
/// `Selectable1`; then the light of the (new) mode. p 0: mode 1 or 2 →
/// mode 0, flag 0x2 := `Selectable0`, the light of mode 0; mode 0 →
/// nothing. p > 3: fatal 0x66. The graphics refresh and animation
/// re-init are render state. Without the class's `objects.txt` row the
/// test cannot run: nothing changes.
fn object_env_refresh(
    w: &mut ClientWorld,
    msg: &Message<'_>,
    key: UnitKey,
    p: i32,
) -> Result<(), HandlerError> {
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    let Some(row) = msg.inputs.tables.objects.get(u.class as usize).copied() else {
        return Ok(());
    };
    if !row.env_effect {
        return Ok(());
    }
    let mode = u.mode;
    let new_mode = match p {
        1..=3 => {
            if mode == 0 {
                Some(1)
            } else {
                None
            }
        }
        0 => match mode {
            1 | 2 => Some(0),
            _ => return Ok(()),
        },
        _ => return Err(HandlerError::Fatal(0x66)),
    };
    let mode = new_mode.unwrap_or(mode);
    if let Some(m) = new_mode {
        let u = w.units.get_mut(&key).expect("present");
        u.mode = m;
        u.flag_2 = Some(row.selectable[m as usize & 7]);
    }
    let lit = row.lit[mode as usize & 7];
    object_light(w, key, lit, row.rgb);
    Ok(())
}

/// The object light:
/// kind 2, radius `Lit` / 2, the objects color; `Lit` = 0 removes it;
/// an object with a light gets a new target.
pub fn object_light(w: &mut ClientWorld, key: UnitKey, lit: u8, rgb: (u8, u8, u8)) {
    object_light_of(w, key, false, lit, rgb);
}

/// [`object_light`] for an object of set S (`client_only` false) or set
/// C (the `ObjFx::Light` of the client object code).
pub fn object_light_of(
    w: &mut ClientWorld,
    key: UnitKey,
    client_only: bool,
    lit: u8,
    rgb: (u8, u8, u8),
) {
    let owner = Owner {
        unit_type: u32::from(key.unit_type),
        guid: key.guid,
        client_only,
    };
    let current = w
        .lights
        .iter()
        .find(|(_, r)| r.owner() == Some(owner))
        .map(|(id, _)| id);
    match sources::object_light(lit, current.is_some(), rgb) {
        ObjectLight::Remove => {
            if let Some(id) = current {
                let _ = w.lights.remove(id);
            }
        }
        ObjectLight::SetTarget(r) => {
            if let Some(id) = current {
                w.lights.set_target(id, r);
            }
        }
        ObjectLight::Create(req) => {
            let set = if client_only {
                &w.objclient.set_c
            } else {
                &w.units
            };
            let (x, y) = set.get(&key).map_or((0, 0), |u| u.cell());
            let pos = (
                unit_light_pos(i32::from(x) << 16),
                unit_light_pos(i32::from(y) << 16),
            );
            w.lights.create(
                Some(owner),
                pos,
                LightKind::Cached,
                req.radius,
                req.i,
                req.r,
                req.g,
                req.b,
            );
        }
    }
}

/// S→C 0x53 Darkness: u32@1 period index, u32@5 ticks, u8@9
/// eclipse, applied to the client act's record when the client act is
/// the local player's.
pub fn darkness(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 10 {
        return Err(HandlerError::Invalid("0x53 is 10 bytes"));
    }
    let (index, ticks, flag) = (b.u32(1)? as i32, b.u32(5)? as i32, b.u8(9)?);
    let p = w
        .local()
        .ok_or(HandlerError::Invalid("0x53 without a local player"))?
        .key;
    // P has an act once it is placed (creation at a point,
    // or 0x15); the model has one client act.
    let placed = w.units[&p].position.is_some();
    if w.act.is_none() {
        if placed {
            return Ok(());
        }
        // Both pointers null: the base game goes on with no act.
        return Err(HandlerError::Unspecified(
            "render/lighting.md §9.2 r4.2: 0x53 with no client act and an unplaced player",
        ));
    }
    if !placed {
        return Ok(());
    }
    // The setter with P's room (none → `L` = 0).
    let level = w.unit_room(p).map_or(0, |r| u32::from(r.level));
    let env = w.environment.as_mut().ok_or(HandlerError::Invalid(
        "client act without its environment record",
    ))?;
    env.set_from_server(&periods()?, index, ticks, flag, act_index(level), level)?;
    // The day period p (env +0x04) against the cache.
    let p = env.kind;
    if p != w.env_period_cache {
        w.env_period_cache = p;
        day_refresh(w, msg, p)?;
    }
    // The requirement refresh of 0x47 on P sets no model field.
    Ok(())
}

/// The set-S owner of a model unit (`+0x00..+0x08`).
fn owner_of(key: UnitKey) -> Owner {
    Owner {
        unit_type: u32::from(key.unit_type),
        guid: key.guid,
        client_only: false,
    }
}

/// The unit's light (unit `+0x64`): the record its owner fields name
/// that no other slot holds. The unit's cast light (stat list,
/// `ClientWorld::cast_lights`) and its overlays' lights (overlay `+0x40`,
/// `ClientWorld::overlay_lights`) carry the same owner but are not
/// `+0x64` (unit light slots).
// Settled: draws-fx-charged-bolt-hit, fresh --pixels run, draw list equal
// 416/416, 156 of 480000 pixels differ (PIXELS-NEAR, run-queue w1-c1-charged-bolt-hit).
pub fn unit_light(
    w: &ClientWorld,
    key: UnitKey,
) -> Option<crate::rules::lighting::records::LightId> {
    let owner = owner_of(key);
    let cast = w.cast_lights.get(&key).copied();
    let held = |id| {
        cast == Some(id)
            || w.overlay_lights
                .iter()
                .any(|o| o.unit == key && o.light == Some(id))
    };
    w.lights
        .iter()
        .find(|&(id, r)| r.owner() == Some(owner) && !held(id))
        .map(|(id, _)| id)
}

/// With (U): the unit's light (unit `+0x64`) removed at once
/// and `+0x64` := 0; a unit without one: nothing.
pub fn remove_own_light(w: &mut ClientWorld, key: UnitKey) {
    if let Some(id) = unit_light(w, key) {
        let _ = w.lights.remove(id);
    }
}

/// Creates `req` as the light of `key` (position
/// from the unit's sub-tile, static path), replacing the light it
/// had, then sets `req`'s target.
pub(crate) fn replace_unit_light(w: &mut ClientWorld, key: UnitKey, req: sources::LightRequest) {
    if let Some(id) = unit_light(w, key) {
        let _ = w.lights.remove(id);
    }
    let Some(u) = w.units.get(&key) else {
        return;
    };
    let (x, y) = u.cell();
    let pos = (
        unit_light_pos(i32::from(x) << 16),
        unit_light_pos(i32::from(y) << 16),
    );
    let kind = match req.kind {
        0 => LightKind::Shadowed,
        2 => LightKind::Cached,
        _ => LightKind::Plain,
    };
    let id = w.lights.create(
        Some(owner_of(key)),
        pos,
        kind,
        req.radius,
        req.i,
        req.r,
        req.g,
        req.b,
    );
    if let (Some(id), Some(t)) = (id, req.target) {
        w.lights.set_target(id, t);
    }
}

/// The player light of the player init (,
/// player row): kind 0 for the local player or while no local player
/// exists, else 1; radius 13, white.
pub fn player_light(w: &mut ClientWorld, key: UnitKey) {
    let local = w.local_player.is_none_or(|l| l == key);
    replace_unit_light(w, key, sources::player_light(local));
}

/// The monster light of a
/// created monster: kind 0, radius `max(L_c, Light)`, 3 in level 8 (the
/// monster's room's level) with client quest byte 1 set and `Align` ∉
/// {1, 2}; none when 0; replaces the unit's light.
///
/// `L_c`: over the 16 components, the choice byte
/// of the unit's monster data; a choice at or above the class's count
/// gives no code, else the code's item `lightradius`.
/// `D2RUST's own, unverified`: a quest byte read before any 0x5E reads 0
/// (the fatal 0x60 is named only for the quest read).
pub fn monster_light(w: &mut ClientWorld, key: UnitKey, class: &super::super::world::MonsterClass) {
    let choices = w.units.get(&key).and_then(|u| match &u.kind {
        super::super::world::KindData::Monster(m) => Some(m.components),
        _ => None,
    });
    let input = sources::MonsterLightInput {
        l_c: choices.map_or(0, |c| component_radius(class, &c)),
        light: i32::from(class.light),
        rgb: class.light_rgb,
        level: w.unit_level(key).map_or(0, u32::from),
        quest_byte1: w.client_quest_byte(1).is_some_and(|b| b != 0),
        client_only: false,
        align: class.setup.map_or(0, |s| i32::from(s.align)),
    };
    if let Some(req) = sources::monster_light(&input) {
        replace_unit_light(w, key, req);
    }
}

/// `L_c` of a monster with these component choices.
pub fn component_radius(class: &super::super::world::MonsterClass, choices: &[u8; 16]) -> i32 {
    sources::component_light((0..16).filter_map(|i| {
        let v = choices[i];
        (v < class.components[i])
            .then(|| class.component_light[i].get(usize::from(v)).copied())
            .flatten()
    }))
}

/// Whether the colour call's light part can change anything for `key`:
/// the local player with a light.
pub fn colour_call_lights(w: &ClientWorld, key: UnitKey) -> bool {
    w.local_player == Some(key) && unit_light(w, key).is_some()
}

/// The colour call, run by
/// state on / off for a state with `colorshift` ≠ 0.
/// Over the on states with a
/// `states` row, by id, a state wins when its `colorpri` is greater than
/// the best so far (which starts at 0): `colorpri` 0 never wins, ties keep
/// the lowest id. A winner with id > 0 sets the unit's palette index
/// (+0x6C) to its `colorshift` (0 allowed), none (or id 0) to 0. The
/// `'h'` exception (index 0 for the local player under render kind ≥ 4)
/// never applies under the GDI reference. Then, for the local player with
/// a light, R, G, B := the winner's `light-r`, `light-g`,
/// `light-b`, or 255, 255, 255 without one, over any
/// `item_lightcolor`.
pub fn state_colour_call(
    w: &mut ClientWorld,
    rows: &[super::super::world::StateRow],
    key: UnitKey,
) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let mut best = None;
    let mut pri = 0;
    for &s in &u.states {
        let Some(row) = rows.get(usize::from(s)) else {
            continue;
        };
        if row.colorpri > pri {
            pri = row.colorpri;
            best = Some((s, *row));
        }
    }
    let win = best.filter(|&(id, _)| id > 0).map(|(_, row)| row);
    u.palette_index = win.map_or(0, |row| row.colorshift);
    if !colour_call_lights(w, key) {
        return;
    }
    let id = unit_light(w, key).expect("checked above");
    let (r, g, b) = win.map_or((255, 255, 255), |row| row.light_rgb);
    let i = w
        .lights
        .get(id)
        .map_or(sources::SOURCE_INTENSITY, |rec| rec.i);
    w.lights.set_color(id, i, r, g, b);
}

/// The player stat callback's light part (player row:
/// run when the stat's value changes): stat
/// 89 `item_lightradius` sets the player's light radius to the
/// new value plus 13; stat 90 `item_lightcolor` sets its R, G, B (0 →
/// white). Other stats, other units, a player without a light or an
/// unchanged value: nothing.
pub fn player_light_stat(w: &mut ClientWorld, key: UnitKey, stat: u16, old: i32, new: i32) {
    if key.unit_type != super::super::world::PLAYER || old == new || !matches!(stat, 89 | 90) {
        return;
    }
    let Some(id) = unit_light(w, key) else {
        return;
    };
    if stat == 89 {
        w.lights.set_radius(id, sources::player_light_radius(new));
    } else {
        let (r, g, b) = sources::player_light_color(new as u32);
        let i = w
            .lights
            .get(id)
            .map_or(sources::SOURCE_INTENSITY, |rec| rec.i);
        w.lights.set_color(id, i, r, g, b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::overlay_lights::OverlayLight;
    use crate::bridge::world::{ClientUnit, MONSTER};

    const FALLEN: UnitKey = UnitKey::new(MONSTER, 24);

    fn light(
        w: &mut ClientWorld,
        kind: LightKind,
        r: i32,
    ) -> crate::rules::lighting::records::LightId {
        w.lights
            .create(Some(owner_of(FALLEN)), (0, 0), kind, r, 255, 255, 255, 255)
            .unwrap()
    }

    fn overlay(light: crate::rules::lighting::records::LightId) -> OverlayLight {
        OverlayLight {
            unit: FALLEN,
            kind: 1,
            id: 0,
            live: 1,
            b: 0,
            state: 0,
            step: 0,
            frame: 0,
            end: 0,
            light: Some(light),
            stamp: 0,
        }
    }

    #[test]
    fn the_death_removes_the_units_own_light_not_its_overlay_or_cast_light() {
        let mut w = ClientWorld::default();
        w.units.insert(FALLEN, ClientUnit::new(FALLEN));
        // A Fallen without a light of its own, hit by a charged bolt: the
        // hit overlay's light (kind 1, at the head) carries its owner.
        let lit = light(&mut w, LightKind::Plain, 11);
        w.overlay_lights.push(overlay(lit));
        assert_eq!(unit_light(&w, FALLEN), None);
        remove_own_light(&mut w, FALLEN);
        assert!(
            w.lights.get(lit).is_some(),
            "light removal reads only the light handle"
        );
        // Its cast light is not +0x64 either.
        let cast = light(&mut w, LightKind::Plain, 5);
        w.cast_lights.insert(FALLEN, cast);
        assert_eq!(unit_light(&w, FALLEN), None);
        // The monster light is, wherever the others sit in the list.
        let own = w
            .lights
            .create(
                Some(owner_of(FALLEN)),
                (0, 0),
                LightKind::Shadowed,
                6,
                255,
                255,
                255,
                255,
            )
            .unwrap();
        let cast2 = light(&mut w, LightKind::Plain, 5);
        w.cast_lights.insert(FALLEN, cast2);
        let _ = w.lights.remove(cast);
        assert_eq!(unit_light(&w, FALLEN), Some(own));
        remove_own_light(&mut w, FALLEN);
        assert!(w.lights.get(own).is_none());
        assert!(w.lights.get(lit).is_some() && w.lights.get(cast2).is_some());
    }
}
