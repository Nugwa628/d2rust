//! The act change of a level warp, for a player whose destination level is
//! in another act. `level_warp` in [`super::place`] calls it after the
//! level is made ready; it moves the player and sends the client the new
//! act.
//!
//! Order of events: client state 5, spawn point and free point, leave the
//! old room, enter the new one, the client's room switch to none (the old
//! act's removals), S->C 0x05, the unit's act, S->C 0x03 then 0x53, the
//! room switch to the new room (0x07 and its units), the player's update
//! (0x15) and room-change messages, the pets follow. The player stays the
//! client's local player: it is not re-added.
//!
//! The messages of a cross-act travel, in order: the player's removal
//! for the other clients of its old room, the old act's removals (S->C
//! 0x0A / 0x08, the client's room switch to no room; never the player's
//! own unit, which left its room first), 0x05 UnloadComplete, 0x03 LoadAct
//! (the new act's init seed, its town, the object seed) and 0x53, then the
//! room switch's 0x07 joins and the unit adds. The player's own placement
//! reaches its client as S->C 0x15 from the next per-client update (unit
//! flag-ex 0x10000), and S->C 0x04 follows when the client's room is ready
//! (client state 5).
//!
//! The hireling part (the classic pet drop) is the
//! [`HirelingCall::ActChange`] the warp queued (`super::place::level_warp`);
//! it runs after this function.
//!
//! Not run here: the arena flag test (the single-player arena flags never
//! have bit value 2) and the disguise check (a shapeshifted player losing
//! its disguise states).
//!
//! [`HirelingCall::ActChange`]: crate::wiring::action::HirelingCall::ActChange

use crate::drlg::act_of_level;
use crate::path::place::level_spawn_point;
use crate::path::place_seams::{mask, CollisionView};
use crate::path::search::free_point_step;
use crate::units::lists::client_state;
use crate::units::UnitId;
use crate::wiring::action::{Pending, WiringError};

use super::place::{log, with_shared};
use super::walk::PathCtx;

/// Search step of the free-point search around the spawn point.
const ACT_CHANGE_STEP: i32 = 5;
/// Flag-ex 0x10000: S->C 0x15 at the unit's update.
const REASSIGN_EX: u32 = 0x1_0000;

/// S->C 0x05 UnloadComplete.
const UNLOAD_COMPLETE: u8 = 0x05;

/// Error codes pushed as `WiringError::ActChange`: the client is already in
/// the destination act, leaving the old room failed, entering the new one
/// failed.
const FATAL_SAME_ACT: u32 = 0x19F;
const FATAL_LEAVE: u32 = 0x1D1;
const FATAL_ENTER: u32 = 0x1D6;

/// S->C 0x03 LoadAct: act u8 at 1, init seed u32 at 2, town level u16 at 6,
/// object seed u32 at 8.
pub fn load_act(act: u8, init_seed: u32, town: u16, obj_seed: u32) -> [u8; 12] {
    let mut m = [0u8; 12];
    m[0] = 0x03;
    m[1] = act;
    m[2..6].copy_from_slice(&init_seed.to_le_bytes());
    m[6..8].copy_from_slice(&town.to_le_bytes());
    m[8..12].copy_from_slice(&obj_seed.to_le_bytes());
    m
}

/// Step 3: act `act`'s DRLG, created when missing
/// on the game's init seed and
/// difficulty, read from an act already built (its copies).
/// `false`: no act is built, or the creation
/// failed (logged).
// TODO: an arena game passes its arena level as the
// town; D2RUST keeps no arena flag, so the act's own town is used.
fn build_act<X: Pending>(c: &mut PathCtx<'_, X>, act: u8) -> bool {
    let drlg = &mut c.v.h.drlg;
    if drlg
        .dungeon
        .acts
        .get(usize::from(act))
        .is_some_and(Option::is_some)
    {
        return true;
    }
    let Some((init_seed, difficulty)) = drlg
        .dungeon
        .acts
        .iter()
        .flatten()
        .next()
        .map(|d| (d.init_seed, d.difficulty))
    else {
        return false;
    };
    let data = drlg.data.clone();
    let r =
        drlg.dungeon
            .get_or_create(act, init_seed, difficulty, None, &data, drlg.types.as_mut());
    match r {
        Ok(_) => true,
        Err(e) => {
            c.v.h
                .errors
                .push(crate::wiring::action::WiringError::Drlg(e));
            false
        }
    }
}

/// Move `player` to `level` (in another act than the player's client) at
/// `tile_index`: `true` when the player was moved. The client goes to
/// state 5; 0x04 is not sent here: the next tick's client pass sends it
/// when the new room is ready. No spawn room or no free point leaves the
/// player where it is: nothing is sent and the client state stays 5.
pub fn run<X: Pending>(mut c: PathCtx<'_, X>, player: UnitId, level: u32, tile_index: u32) -> bool {
    // Step 1: D2RUST has no arena record; its flag bit 1 is
    // clear in every game D2RUST runs.
    let act = act_of_level(level);
    let client = c
        .game
        .lists
        .clients()
        .into_iter()
        .find(|&k| c.game.lists.client(k).and_then(|e| e.player) == Some(player));
    // The client's act is the act of its room.
    let client_act = client
        .and_then(|k| c.game.lists.client(k))
        .and_then(|e| e.room)
        .and_then(|r| c.game.lists.room(r))
        .map(|r| r.act);
    if client_act == Some(act) {
        c.v.h.errors.push(WiringError::ActChange(FATAL_SAME_ACT));
        return false;
    }
    // Build the destination act if missing (the game's init seed and
    // difficulty, which every built act holds a copy of). A game with no
    // act built, or a failed build: nothing changes, nothing is sent.
    if !build_act(&mut c, act) {
        return false;
    }
    let Some(init_seed) =
        c.v.h
            .drlg
            .dungeon
            .acts
            .get(usize::from(act))
            .and_then(Option::as_ref)
            .map(|d| d.init_seed)
    else {
        return false;
    };
    let obj_seed = c.v.h.objects.as_ref().map_or(0, |o| o.obj_seed);
    let town = crate::drlg::TOWN_LEVELS
        .get(usize::from(act))
        .copied()
        .unwrap_or_default();
    // The client waits for the new act.
    if let Some(e) = client.and_then(|k| c.game.lists.client_mut(k)) {
        e.state = client_state::CHANGING_ACT;
    }
    // Steps 5–7: P is `player`; the classic pet drop is the queued
    // hireling call (module docs).
    // TODO: the disguise check
    // (a shapeshifted player loses its disguise states) has
    // no D2RUST implementation; the states stay.
    // Steps 8–9: the spawn search, then the free point
    // from it (mask 0x1C89). Failing, steps 3–7
    // stay done and nothing is sent (the client stays in state 5).
    let spawn = with_shared(PathCtx::of(&mut c.v, c.game), |cv, _, lv| {
        let size = cv.unit_size(player);
        let r = level_spawn_point(cv, lv, Some(act), level, tile_index, size);
        let (room, mut p) = log(cv, r)??;
        let r = free_point_step(
            cv,
            Some(room),
            &mut p,
            size,
            mask::ACT_CHANGE,
            ACT_CHANGE_STEP,
        );
        log(cv, r)?.map(|room| (room, p))
    });
    let Some((room, p)) = spawn else {
        return false;
    };
    // The player leaves its room O (teleport to (none, 0, 0): footprint
    // cleared, off O's list, previous room := O), then the room-change
    // messages with O as the old room (the other clients of O get the
    // player's removal).
    let old = c.game.lists.unit(player).and_then(|e| e.room());
    let errors = c.v.h.errors.len();
    c.teleport_clear(player, None, 0, 0);
    if c.v.h.errors.len() > errors {
        c.v.h.errors.push(WiringError::ActChange(FATAL_LEAVE));
        return false;
    }
    // O is the old room of the messages (the teleport's reset recaches
    // again and leaves the previous room none).
    if let Some(d) = c.v.h.paths.as_mut().and_then(|p| p.dynamic_mut(player)) {
        d.prev_room = old;
    }
    c.room_change_messages(player);
    // The player enters the new room R at the free point (R's list, queued).
    c.teleport_clear(player, Some(room), p.x, p.y);
    if c.v.h.errors.len() > errors {
        c.v.h.errors.push(WiringError::ActChange(FATAL_ENTER));
        return false;
    }
    queue(&mut c, player);
    // The client's room switch to none: the old act's removals (the player
    // is in no room of the old act any more).
    if let Some(k) = client {
        c.v.room_switch(c.game, k, None);
    }
    // UnloadComplete.
    c.v.h.x.send(player, &[UNLOAD_COMPLETE]);
    // Step 14: D2RUST keeps no client act of its own: the client's act is
    // its room's, which step 17 sets. Step 15: the unit record's act
    // (+0x18; the act record +0x1C is the act's DRLG).
    if let Some(r) = c.v.units.get_mut(player) {
        r.act = act;
    }
    // 0x03 (the game's init seed, the act's town, the object seed), then
    // 0x53 (the act's environment record, created at the act's first use).
    c.v.h
        .x
        .send(player, &load_act(act, init_seed, town as u16, obj_seed));
    let env = c.game.lists.act_mut(act).map(|r| {
        if !r.built {
            r.built = true;
            r.environment = crate::world::environment::Environment::CREATED;
        }
        r.environment
    });
    if let Some(env) = env {
        c.v.h.x.send(player, &env.message());
        // ByTime's t in the new act: read live from its environment record,
        // 0 for a fresh act.
        c.v.h.act_times.insert(player, env.time());
    }
    // The room switch to R: the new act's 0x07 and unit adds.
    if let Some(k) = client {
        c.v.room_switch(c.game, k, Some(room));
    }
    // Queued, flag-ex 0x10000 (S->C 0x15 at the update), the room-change
    // messages (the other clients of R get the player's adds), queued
    // again.
    queue(&mut c, player);
    if let Some(r) = c.v.units.get_mut(player) {
        r.flags2 |= REASSIGN_EX;
    }
    c.room_change_messages(player);
    queue(&mut c, player);
    // Pets follow; the host does that.
    c.v.h.queue_pet_follow(player);
    true
}

/// Queue the unit for update.
fn queue<X: Pending>(c: &mut PathCtx<'_, X>, unit: UnitId) {
    if let Err(e) = c.game.lists.queue_update(unit) {
        c.v.h
            .errors
            .push(WiringError::Unit(crate::units::modes::UnitError::Game(
                e.into(),
            )));
    }
}
