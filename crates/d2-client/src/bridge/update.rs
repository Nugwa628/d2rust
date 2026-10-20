//! The client update pass: in a frame whose pump ran a server tick, while
//! `in_game`, each unit's queue is drained in the base-game unit order and
//! every queued message goes to its unit handler. Before an object's
//! drain its per-type update runs ([`super::objects`]);
//! the C objects (set C) run theirs, then the second `ClientFn` call, at
//! their place in the walk (after the S and C missiles, before
//! the S players). After the units, the client DRLG's part of the update
//! runs: the build timer and, every 13th update, the level free.
//!
//! The mode machines themselves run inside the queued messages' mode
//! requests ([`super::modes`]). The other per-type unit
//! updates that run before each drain (player, monster,
//! missile, item) change no model field, except the
//! monster anim step ([`super::monster_anim`]) and the player's mode end
//! ([`super::player_anim`]):
//! PROVISIONAL (open questions): no other client-side
//! mode steps, no client seed draws in the animation and no local walk
//! prediction (the local player follows the server position).

use d2_sim::rng::Seed;

use super::dispatch::{Dispatch, Handle, HandlerError, UnitMessage};
use super::drlg::DrlgRoomId;
use super::msg::lighting::{lighting_update, object_light_of};
use super::objects::{self, ObjFx, ObjUnit};
use super::output::{move_freed, Output, Outputs};
use super::receive::{ReceiveLog, Rejected};
use super::world::{update_order, ClientWorld, ModelInputs, UnitKey, MONSTER, OBJECT};

/// Drains every unit's queue. Returns the number of
/// messages applied. A handler error is recorded as a rejection; the
/// drain goes on with the next message. Handler outputs are appended to
/// `outputs` in update order.
pub fn update_pass(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    dispatch: &Dispatch,
    log: &mut ReceiveLog,
    outputs: &mut Vec<Output>,
) -> usize {
    update_pass_steps(world, inputs, dispatch, log, outputs, &[])
}

/// [`update_pass`] with this tick's server paths (`ServerLink::
/// step_paths`): the local player's walk steps at its place in the pass,
/// so the S monsters after it see
/// where it stands this tick.
pub fn update_pass_steps(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    dispatch: &Dispatch,
    log: &mut ReceiveLog,
    outputs: &mut Vec<Output>,
    steps: &[super::critter_path::ServerPath],
) -> usize {
    let mut stepped = None;
    let mut applied = 0;
    // The room pass runs first (critters and
    // client presets of the rooms not yet populated).
    super::critters::room_pass(world, inputs);
    let order = update_order(&world.units);
    // S missiles first, then the C missiles
    // and the C objects; then the other S
    // types.
    let missiles = order
        .iter()
        .take_while(|k| k.unit_type == super::world::MISSILE)
        .count();
    let mut c_objects_done = false;
    for (i, &key) in order.iter().enumerate() {
        if i >= missiles && !c_objects_done {
            c_objects_done = true;
            c_missiles(world, inputs, log, outputs);
            c_objects(world, inputs, log, outputs);
        }
        let local = world.local_player == Some(key);
        // Unit flag 0x800000 (the client room free) on a unit
        // other than the local player runs nothing (no per-type update,
        // no drain); with flag-ex 0x20 the client sends C→S
        // 0x4B [type u32][GUID u32] and clears both bits (a server unit
        // stays until the server answers).
        if let Some(unit) = world.units.get_mut(&key) {
            if unit.room_freed && !local {
                if unit.flag_ex & 0x20 != 0 {
                    unit.room_freed = false;
                    unit.flag_ex &= !0x20;
                    let mut m = vec![0x4B];
                    m.extend_from_slice(&u32::from(key.unit_type).to_le_bytes());
                    m.extend_from_slice(&key.guid.to_le_bytes());
                    world.outgoing.push(m);
                }
                continue;
            }
        }
        // The unit update first runs the timed lists
        // with (U, 0).
        if let Some(u) = world.units.get_mut(&key) {
            super::world::expire_timed_lists(u);
        }
        // The unit's overlays advance before its per-type update.
        super::overlay_lights::advance(world, inputs, key);
        if key.unit_type == OBJECT && world.units.contains_key(&key) {
            // The object update.
            let unit = ObjUnit {
                key,
                client_only: false,
            };
            let start = outputs.len();
            if let Err(error) = objects::object_update(world, inputs, unit, outputs) {
                log.rejected.push(Rejected { id: 0, error });
            }
            apply_object_lights(world, &outputs[start..]);
            move_freed(world, outputs);
        }
        // The monster update, before
        // the drain.
        if key.unit_type == MONSTER {
            // The anim step, the mode missile step
            // and the end run inside the update.
            if let Err(error) = super::critter_path::server_update(world, inputs, key) {
                log.rejected.push(Rejected { id: 0, error });
            }
        }
        // The item update: a dropping item (mode 5) steps
        // its frame by the speed and lands (mode 3) once the next step
        // reaches the frame count. The selectable flag (+0xC4 bit 0x2)
        // is set by the mode-3 branch only, so a
        // landed item becomes a hover target one update after it lands;
        // modes 0, 1, 2, 4 and 6 clear it.
        if key.unit_type == super::items::ITEM {
            if let Some(u) = world.units.get_mut(&key) {
                match u.mode {
                    3 => u.flag_2 = Some(true),
                    5 => {}
                    _ => u.flag_2 = Some(false),
                }
            }
            item_update(world, key);
        }
        // The player update's mode end (an attack, cast or
        // hit animation over → neutral), before the drain
        // (client mode machine).
        if key.unit_type == super::world::PLAYER {
            // The copy turned a walking player for this tick already
            // (`critter_path::player_aim`). A click that set a cast or
            // attack before this update (`click::apply`) left the walk:
            // the update turns it toward the new target (a Nova: dir 24 at
            // tick 265, the first frame of the cast that ends a walk).
            // The turn is the update's step 7,
            // after the do and the advance: a face the do sets (Strafe's
            // next target) turns in the same
            // update (a Strafe: dir 0 → 63 at frame 29):
            // the path step of path kind 1 (WL, RN, TW, KB).
            if local {
                if let Some(s) = steps.iter().find(|s| s.key == key) {
                    stepped = super::critter_path::local_path_step(world, s);
                }
            }
            let copied = world
                .units
                .get_mut(&key)
                .is_some_and(|u| std::mem::take(&mut u.copy_turned) && matches!(u.mode, 2 | 3 | 6));
            if let Err(error) = super::player_anim::step(world, inputs, key) {
                log.rejected.push(Rejected { id: 0, error });
            }
            // A moving skill's turn is its path step's, after the aim
            // (`critter_path::player_aim`), once an update.
            if !copied && !super::critter_path::path_skill(world, key) {
                super::critter_path::player_turn(world, key);
            }
            // Whirlwind's swing helper leaves the new direction equal to the
            // direction (`critter_path::swing_restore`).
            super::critter_path::swing_restore(world, inputs, key);
            // The room-change test: flag cleared for every
            // player; the local player's handler copies the
            // animated-tile phase.
            if let Some(prev) = world.room_units.take_change(key) {
                anim_room_change(world, local.then_some(key), prev);
            }
        }
        // Looked up again: an earlier unit's messages or its own update
        // may have removed it.
        let Some(unit) = world.units.get_mut(&key) else {
            continue;
        };
        let queue = std::mem::take(&mut unit.queue);
        for bytes in &queue {
            let id = bytes[0];
            let sink = Outputs::default();
            let result = match dispatch.get(id).map(|e| e.handle) {
                Some(Handle::Unit(handle)) => handle(
                    world,
                    &UnitMessage {
                        id,
                        bytes,
                        unit: key,
                        inputs,
                        out: &sink,
                    },
                ),
                // Only unit-handler ids are queued; the base game
                // asserts on anything else.
                _ => Err(HandlerError::Invalid(
                    "queued message without a unit handler",
                )),
            };
            match result {
                Ok(()) => {
                    applied += 1;
                    log.drained += 1;
                    outputs.extend(sink.take());
                }
                Err(error) => log.rejected.push(Rejected { id, error }),
            }
            move_freed(world, outputs);
        }
    }
    if !c_objects_done {
        c_missiles(world, inputs, log, outputs);
        c_objects(world, inputs, log, outputs);
    }
    // The C monsters walk last; each runs the critter AI.
    // Their per-unit monster update (the anim step)
    // is not run on set C. The local player stands at this pass's step
    // while they walk: its path step ran earlier in the same update, so
    // the AI's nearest-player distance reads the stepped cell
    // (measured after a Howl: the chicken's D > 30 test).
    super::critters::c_monsters(world, inputs);
    if let Some(step) = stepped {
        super::critter_path::local_path_step_end(world, step);
    }
    follow_path_target(world);
    animate_tiles(world);
    // The draw that follows the update writes a dead monster's snapped
    // direction into its path.
    super::monster_anim::death_snap(world, inputs);
    if let Err(error) = drlg_update(world) {
        // Not a message: recorded under the client update's own id 0.
        // A DRLG error is a fatal error of the DRLG code.
        log.rejected.push(Rejected { id: 0, error });
    }
    // Once per client update.
    if let Err(error) = lighting_update(world, inputs) {
        log.rejected.push(Rejected { id: 0, error });
    }
    // The draw that follows the update writes a dead monster's snapped
    // direction into its path.
    super::monster_anim::death_snap(world, inputs);
    // The Den lights' creates.
    missile_sounds(world, outputs);
    move_freed(world, outputs);
    applied
}

/// A walk to a unit keeps the local player's path target on the unit's
/// point while it walks; the point stays when the walk ends (D2RUST's own
/// bookkeeping). Once per client update,
/// after the units' updates.
/// The item update (item update). Mode
/// 5 (dropping) steps the frame by the speed; once the next step reaches
/// the frame count the item lands (mode 3) and gets its ground overlay.
/// Mode 3 creates the overlay when the item has none.
/// The create with (item, 70, 6, 8, 0, 0, 0, 0) (call)
/// draws twice on the local player's client
/// seed: `roll(frames)` for kind 6 and `roll(8 × 256)` for
/// a = 8; both counts are > 0, so each is one step. The
/// create draws nothing for an item with flag-ex bit 0x40000 or
/// with no local player.
fn item_update(world: &mut ClientWorld, key: UnitKey) {
    let (rate, frame) = (world.tick_rate, world.share_frame());
    let Some(u) = world.units.get_mut(&key) else {
        return;
    };
    match u.mode {
        5 => {
            // The speed is per base-rate tick: this tick's share of it.
            let sp = rate.share(u.speed.unwrap_or(0), frame);
            if u.frame_count > sp + u.frame {
                u.frame += sp;
                if u.frame_count <= u.frame {
                    u.frame -= u.frame_count;
                }
                return;
            }
            u.mode = 3;
        }
        3 if u.ground_overlay.is_none() => {}
        _ => return,
    }
    if u.flag_ex & 0x40000 != 0 {
        return;
    }
    let tick = world.server_ticks;
    let mut start = 0;
    let mut before = None;
    if let Some(p) = world.local_player.and_then(|k| world.units.get_mut(&k)) {
        if let Some((lo, hi)) = p.seed {
            before = Some((lo, hi));
            let mut s = Seed::new(lo, hi);
            // The kind-6 start `roll(frames)` is one step whatever the
            // count; the a = 8 draw then replaces it.
            s.step();
            start = s.roll(8 * 256) as i32;
            p.seed = Some((s.lo, s.hi));
        }
    }
    if let Some(u) = world.units.get_mut(&key) {
        u.ground_overlay = Some((tick, start));
        u.ground_overlay_seed = before;
    }
}

fn follow_path_target(world: &mut ClientWorld) {
    let Some(key) = world.local_player else {
        return;
    };
    let Some(u) = world.units.get(&key) else {
        return;
    };
    let Some(follow) = u.path_follow else {
        return;
    };
    let at = world.units.get(&follow).and_then(|t| t.position);
    let walking = matches!(u.mode, 2 | 3 | 6);
    let u = world.units.get_mut(&key).expect("present");
    if walking {
        if at.is_some() {
            u.path_target = at;
        }
    } else {
        u.path_follow = None;
    }
}

/// The C objects' walk: each runs the object
/// update (call site A), then, still in set C, the dispatch once more
/// (call site B). Errors are recorded under the update's id 0.
/// The C missiles' walk (before the C objects): each runs the
/// client missile dispatch.
fn c_missiles(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    log: &mut ReceiveLog,
    outputs: &mut Vec<Output>,
) {
    // The client's state-86 lists count
    // down first (PROVISIONAL).
    super::client_missiles::tick_just_hit(world);
    // PROVISIONAL: the unit footprints the missiles read.
    super::client_missiles::stamp_unit_footprints(world, &inputs.tables.monsters);
    super::client_missiles::stamp_object_footprints(world, &inputs.objclient.rows);
    let mut walk = objects::CWalk::new(super::world::MISSILE);
    while let Some(key) = walk.next(world) {
        let env = super::client_missiles::Env {
            rows: &inputs.tables.missiles,
            lights: inputs.high_light_quality,
            skills: inputs.skill_tables.as_deref(),
            monsters: &inputs.tables.monsters,
            model: Some(inputs),
        };
        let r = super::client_missiles::update_with(world, &env, key);
        if let Err(error) = r {
            log.rejected.push(Rejected { id: 0, error });
        }
    }
    missile_sounds(world, outputs);
    move_freed(world, outputs);
}

/// The client missiles' sound calls in
/// update order, before the frees of the same walk.
fn missile_sounds(world: &mut ClientWorld, outputs: &mut Vec<Output>) {
    outputs.extend(
        world
            .objclient
            .missile_sounds
            .drain(..)
            .map(Output::MissileSound),
    );
}

fn c_objects(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    log: &mut ReceiveLog,
    outputs: &mut Vec<Output>,
) {
    let mut walk = objects::CWalk::new(OBJECT);
    while let Some(key) = walk.next(world) {
        if !world.objclient.set_c.contains_key(&key) {
            continue;
        }
        let unit = ObjUnit {
            key,
            client_only: true,
        };
        let start = outputs.len();
        let r = objects::object_update(world, inputs, unit, outputs)
            .and_then(|()| objects::site_b(world, inputs, key, outputs));
        if let Err(error) = r {
            log.rejected.push(Rejected { id: 0, error });
        }
        apply_object_lights(world, &outputs[start..]);
        move_freed(world, outputs);
    }
}

/// The object lights the client object code asked for
/// (`ObjFx::Light`; object row):
/// the light list is model state, so the update pass applies them at
/// once, in call order; the output stays in the list for the effects
/// layer.
fn apply_object_lights(world: &mut ClientWorld, new: &[Output]) {
    for o in new {
        if let Output::ObjectFx(ObjFx::Light { unit, lit, rgb }) = *o {
            object_light_of(world, unit.key, unit.client_only, lit, rgb);
        }
    }
}

/// The animated-tile tick of the client update (after the units, before
/// the lighting and the build timer): over the near rooms of the local
/// player's room.
fn animate_tiles(world: &mut ClientWorld) {
    let room = world.local_player.and_then(|k| world.room_units.room_of(k));
    if let (Some(room), Some(drlg)) = (room, world.drlg.as_mut()) {
        drlg.animate_tiles(room);
    }
}

/// The room-change handler's phase copy
/// for the local player `local` (none: another
/// player, nothing) whose previous room was `prev`; a player with no
/// room now runs nothing.
fn anim_room_change(world: &mut ClientWorld, local: Option<UnitKey>, prev: Option<DrlgRoomId>) {
    let new = local.and_then(|k| world.room_units.room_of(k));
    if let (Some(new), Some(drlg)) = (new, world.drlg.as_mut()) {
        drlg.copy_anim_phase(prev, new);
    }
}

/// The client DRLG part of the update (last
/// paragraph): the counter += 1, the build timer on the
/// client act's DRLG, then the level free when the counter is a multiple
/// of 13; new active rooms then run the act room callback. Nothing
/// without a client DRLG.
pub fn drlg_update(world: &mut ClientWorld) -> Result<(), HandlerError> {
    world.drlg_updates = world.drlg_updates.wrapping_add(1);
    let free = world.drlg_updates.is_multiple_of(13);
    let Some(drlg) = world.drlg.as_mut() else {
        return Ok(());
    };
    drlg.client_update(free)?;
    world.refresh_active_rooms()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::ClientUnit;

    /// A local player with seed {1, 666} and a dropping item 4/9 (frame
    /// count 4, speed 2, frame 0).
    fn world() -> (ClientWorld, UnitKey, UnitKey) {
        let mut w = ClientWorld::default();
        let p = UnitKey::new(0, 1);
        let mut u = ClientUnit::new(p);
        u.seed = Some((1, 666));
        w.units.insert(p, u);
        w.local_player = Some(p);
        let k = UnitKey::new(super::super::items::ITEM, 9);
        let mut u = ClientUnit::new(k);
        u.mode = 5;
        u.frame_count = 4;
        u.speed = Some(2);
        w.units.insert(k, u);
        (w, p, k)
    }

    fn stepped(n: usize) -> Option<(u32, u32)> {
        let mut s = Seed::new(1, 666);
        for _ in 0..n {
            s.step();
        }
        Some((s.lo, s.hi))
    }

    #[test]
    fn landing_item_draws_twice_on_the_local_seed_once() {
        let (mut w, p, k) = world();
        item_update(&mut w, k); // frame 0 → 2, still dropping
        assert_eq!(w.units[&k].mode, 5);
        assert_eq!(w.units[&p].seed, stepped(0));
        item_update(&mut w, k); // 2 + 2 reaches 4: lands, overlay 70
        assert_eq!(w.units[&k].mode, 3);
        assert_eq!(w.units[&p].seed, stepped(2));
        let mut s = Seed::new(1, 666);
        s.step();
        let start = s.roll(2048) as i32;
        assert_eq!(w.units[&k].ground_overlay, Some((0, start)));
        item_update(&mut w, k); // overlay present: no draws
        assert_eq!(w.units[&p].seed, stepped(2));
    }

    #[test]
    fn ground_item_without_overlay_draws_unless_flag_40000() {
        let (mut w, p, k) = world();
        w.units.get_mut(&k).unwrap().mode = 3;
        w.units.get_mut(&k).unwrap().flag_ex = 0x40000;
        item_update(&mut w, k);
        assert_eq!(w.units[&p].seed, stepped(0));
        w.units.get_mut(&k).unwrap().flag_ex = 0;
        item_update(&mut w, k);
        item_update(&mut w, k);
        assert_eq!(w.units[&p].seed, stepped(2));
    }

    /// F5: the drop speed is per base-rate tick, so a dropping item lands
    /// after the same real time at 25 Hz and at the game's 60 Hz (20
    /// frames at one a base tick: 800 ms, within one base tick).
    #[test]
    fn a_dropping_item_lands_after_the_same_real_time_at_every_rate() {
        for hz in [25u32, 60] {
            let (mut w, _, k) = world();
            w.tick_rate = d2_sim::tick::TickRate::new(hz).unwrap();
            let u = w.units.get_mut(&k).unwrap();
            u.frame_count = 20 << 8;
            u.speed = Some(256);
            let mut t = 0;
            while w.units[&k].mode == 5 {
                w.server_frame = Some(t);
                item_update(&mut w, k);
                t += 1;
                assert!(t < 1000, "the item lands");
            }
            let ms = i64::from(t) * 1000 / i64::from(hz);
            assert!((ms - 800).abs() <= 40, "{hz} Hz: landed after {ms} ms");
        }
    }
}
