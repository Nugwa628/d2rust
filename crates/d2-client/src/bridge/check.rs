//! The position check: compares a point the server states
//! with the unit's own position and, when they disagree beyond the
//! tolerance (or the unit would be drawn off-screen), corrects it: the
//! local player asks the server with C→S 0x5F, other units are moved.

use crate::rules::camera::{moving_to_client, static_to_client};

use super::dispatch::HandlerError;
use super::world::{
    ClientUnit, ClientWorld, LocalWalk, ModelInputs, UnitKey, ITEM, MISSILE, MONSTER, OBJECT,
    PLAYER, TILE,
};

/// What the check did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checked {
    /// x or y is 0, or the unit is dead (rules 1–2), or the unit is not in
    /// the model.
    Skipped,
    /// Within tolerance (or accepted by step 5) and visible: no change.
    Kept,
    /// The local player: C→S 0x5F appended to `outgoing` (step 8).
    Asked,
    /// The local player while the play preview predicts its walk: step 8
    /// takes the server's point (the prediction snaps to it), no C→S 0x5F
    /// ([`correct`]). D2RUST's own, PROVISIONAL.
    Followed,
    /// Another unit moved to (x, y) (step 8).
    Moved,
    /// Step 8 found no room for (x, y) in the client DRLG: nothing.
    NoRoom,
}

/// `L` of step 4: `(+ 0x32) >> 7`, read from the model's
/// ping round trip (D2RUST sends no ping, so 0).
fn latency(world: &ClientWorld) -> u32 {
    world.ping.rtt.wrapping_add(0x32) >> 7
}

/// Static-path kinds (step 3): objects, items, tiles.
fn is_static(unit_type: u8) -> bool {
    matches!(unit_type, OBJECT | ITEM | TILE)
}

/// Tolerance T (step 4). `own`: the local player's own walk (its mode is
/// the walk / run the client moves it in).
fn tolerance(world: &ClientWorld, unit: &ClientUnit, own: Option<LocalWalk>, kind: u8) -> u32 {
    match kind {
        1 => 10,
        2 => 0,
        _ if world.local_player == Some(unit.key) => {
            let l = latency(world);
            match own.and_then(|w| w.mode).unwrap_or(unit.mode) {
                1 => 3 + l,
                3 => 7 + l,
                _ => 5 + l,
            }
        }
        _ if unit.key.unit_type == MONSTER && (3..=5).contains(&unit.mode) => 5,
        _ if unit.key.unit_type == MONSTER && (6..=11).contains(&unit.mode) => 7,
        _ => 15,
    }
}

/// The unit's client pixel point (step 6:):
/// the static path's point, or the dynamic path's precise position (the
/// cell centre, model) projected by the camera.
/// `own`: the local player's own walk, whose precise position is its
/// dynamic path's.
fn client_point(unit: &ClientUnit, own: Option<LocalWalk>) -> (i32, i32) {
    let (cx, cy) = unit.cell();
    let p = if is_static(unit.key.unit_type) {
        static_to_client(i32::from(cx), i32::from(cy))
    } else if let Some(w) = own {
        moving_to_client(w.pos.0, w.pos.1)
    } else {
        moving_to_client(
            (u32::from(cx) << 16) | 0x8000,
            (u32::from(cy) << 16) | 0x8000,
        )
    };
    (p.x, p.y)
}

fn sq(d: i32) -> i64 {
    i64::from(d) * i64::from(d)
}

/// `check(U, x, y, kind, tx, ty)`. `tx`, `ty` are signed as in
/// the base game (step 5 tests `tx > 0`).
#[allow(clippy::too_many_arguments)]
pub fn check(
    world: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    x: u16,
    y: u16,
    kind: u8,
    tx: i32,
    ty: i32,
) -> Result<Checked, HandlerError> {
    // Step 1.
    if x == 0 || y == 0 {
        return Ok(Checked::Skipped);
    }
    let Some(unit) = world.units.get_mut(&key) else {
        return Ok(Checked::Skipped);
    };
    // Step 2.
    if unit.is_dead() {
        return Ok(Checked::Skipped);
    }
    // Step 3.
    unit.server_point = (x, y);
    let unit = &world.units[&key];
    // U's position is its path cell: for the local player while the
    // client moves it, its own path's cell, not the last placement.
    let own = world.predicted(unit);
    let (cx, cy) = own.map_or_else(|| unit.cell(), |w| w.cell());
    let (xi, yi, cxi, cyi) = (i32::from(x), i32::from(y), i32::from(cx), i32::from(cy));
    // Step 4.
    let t = tolerance(world, unit, own, kind);
    // Step 5.
    let mut far = xi.abs_diff(cxi) > t;
    if far || yi.abs_diff(cyi) > t {
        if kind != 0 || tx <= 0 {
            return Ok(correct(world, key, x, y));
        }
        let d1 = sq(cxi - xi) + sq(cyi - yi);
        if d1 >= 100 {
            return Ok(correct(world, key, x, y));
        }
        let d2 = sq(cxi - tx) + sq(cyi - ty);
        if d2 >= d1 {
            return Ok(correct(world, key, x, y));
        }
        far = false;
    }
    // Step 6.
    let visible = if x == cx || y == cy {
        true
    } else {
        let pred = inputs.visible.as_ref().ok_or(HandlerError::Unspecified(
            "model.md open question 7: the visibility predicate",
        ))?;
        let (a, b) = client_point(unit, own);
        let p = static_to_client(xi, yi);
        pred.visible(unit, a, b) || pred.visible(unit, p.x, p.y)
    };
    // Step 7.
    if visible && !far {
        Ok(Checked::Kept)
    } else {
        Ok(correct(world, key, x, y))
    }
}

/// Step 8: room' := the cell lookup from U's room, else the act lookup;
/// none → nothing. Without a client DRLG (no
/// DRLG source) the room is taken as found.
fn correct(world: &mut ClientWorld, key: UnitKey, x: u16, y: u16) -> Checked {
    let mut room = None;
    if world.active_rooms.is_some() {
        let start = world.unit_room(key).copied();
        let Some(found) = world.room_from(start.as_ref(), x, y) else {
            return Checked::NoRoom;
        };
        room = Some(found.room);
    }
    // Only while the prediction moves the player (walk / run): standing,
    // the model cell is the client's own position and the base game asks with
    // 0x5F (a2-quest-tombs frame 71: a `goto` placement stated by 0x96).
    if world
        .predicted(&world.units[&key])
        .is_some_and(|w| w.mode.is_some())
    {
        // D2RUST's own, unverified. PROVISIONAL:
        // the base-game client's own position is its own path,
        // stepped by the same path code as the server's over the same
        // rooms, so the two end together and step 8 asks only when they
        // truly part. The two positions are equal at every tick of walk
        // and run legs into obstacles; whether step 8 skips the local
        // player or just finds no difference is not known. The play preview's position is
        // a straight-line guess that does not see collision: past a wall
        // or around an obstacle it parts from the server's path although
        // the server is right, and C→S 0x5F would make the server walk
        // (or snap) the player to the guess. So
        // the guess yields: the server's point is taken (step 3 stored
        // it; the prediction snaps to it, `Predict::observe`) and nothing
        // is sent.
        if let Some(u) = world.units.get_mut(&key) {
            u.follows = u.follows.wrapping_add(1);
        }
        return Checked::Followed;
    }
    if world.local_player == Some(key) {
        // C→S 0x5F with the unit's own position.
        let (cx, cy) = world.units[&key].cell();
        let mut m = vec![0x5F];
        m.extend_from_slice(&cx.to_le_bytes());
        m.extend_from_slice(&cy.to_le_bytes());
        world.outgoing.push(m);
        return Checked::Asked;
    }
    // Another player (re-placed, walking modes restarted,
    // Phase 6) or a teleport: the
    // model's position is (x, y) either way.
    if let Some(u) = world.units.get_mut(&key) {
        u.position = Some((x, y));
    }
    // A dynamic-path unit placed at (x, y) is recached into room'.
    if room.is_some() && matches!(key.unit_type, PLAYER | MONSTER | MISSILE) {
        world.room_units.place(key, room);
    }
    Checked::Moved
}

#[cfg(test)]
pub(crate) fn latency_for_test(world: &ClientWorld) -> u32 {
    latency(world)
}
