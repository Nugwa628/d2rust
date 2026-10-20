//! The walk and run intent handlers: C→S 0x01 walk to a point, 0x02 walk
//! to a unit, 0x03 run to a point, 0x04 run to a unit; and C→S 0x5F, the
//! client position resync ([`RESYNC_ID`]). `SimGame` routes these ids here
//! and the movement itself lives in `d2-sim`.
//!
//! The dispatcher (`crate::dispatch`) has applied the gate, the exact
//! size and the point / unit parse when [`handle`] runs. The handler is
//! `d2-sim`'s (`d2_sim::wiring::path::walk::walk_message`, the movement
//! mode request on the wired path provider), run by the game's world host
//! ([`WorldHost::walk`]); a host without the path provider
//! (`ActionHooks::paths` off, or no action wiring) leaves the ids to the
//! stub.
//!
//! Messages: the request itself replies nothing (the result is 0 whether
//! or not the mode starts). The movement messages come from elsewhere:
//! 0x0F / 0x10 / 0x15 from the update pass (`ActionSim`'s
//! `send_unit_update`), 0x0D from the placement and warp arrival; all go
//! through the action wiring's `Pending::send` and reach the clients by
//! [`WorldHost::take_sent`] (after a handler and after each tick).
//! 0x95 / 0x96 / 0x18 belong to the client vitals sync (`SimGame`'s sync
//! at the end of each tick), not to movement.

#[cfg(test)]
mod tests;

use d2_sim::game::Game;
use d2_sim::path::walk::Outcome;
use d2_sim::path::PathError;
use d2_sim::tick::EventDispatch;
use d2_sim::units::UnitId;

use super::super::SimGame;
use super::world::{ActionEvents, WorldError, WorldFault, WorldHost};
use crate::seams::{ClientId, MessageSink, ResultCode};

/// The target form of a walk / run message (the layout rows in
/// `client-messages.tsv`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `x:u16@1 y:u16@3`: walk or run to a point.
    Point,
    /// `type:u32@1 id:u32@5`: walk or run to a unit.
    Unit,
}

/// Every walk / run id: (id, `client-messages.tsv` name, a label naming
/// where the rules are described, target form, requested mode). The order
/// is the id order.
pub const WALK_IDS: &[(u8, &str, &str, Form, u32)] = &[
    (0x01, "Walk", "sim/pathing.md §1.1", Form::Point, 2),
    (0x02, "WalkToUnit", "sim/pathing.md §1.1", Form::Unit, 2),
    (0x03, "Run", "sim/pathing.md §1.1", Form::Point, 3),
    (0x04, "RunToUnit", "sim/pathing.md §1.1", Form::Unit, 3),
];

/// C→S 0x5F UpdatePlayerPos (`x:u16@1 y:u16@3`): the client position
/// resync, on the same path provider
/// (`d2_sim::wiring::path::walk::resync_message`). Not a point message:
/// the dispatcher parses nothing, because its point-id list does not
/// include 0x5F.
pub const RESYNC_ID: u8 = 0x5F;

/// The target form of `id`, if it is a walk / run id.
pub fn form(id: u8) -> Option<Form> {
    WALK_IDS
        .iter()
        .find_map(|&(i, _, _, f, _)| (i == id).then_some(f))
}

/// One routed walk / run message: the client's player and the message's
/// fields (point: x, y; unit: type, GUID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkCall {
    /// The client's player unit.
    pub player: UnitId,
    /// The message id (a walk / run id or [`RESYNC_ID`]).
    pub id: u8,
    /// Point form: x. Unit form: unit type.
    pub a: u32,
    /// Point form: y. Unit form: unit GUID.
    pub b: u32,
}

/// The handler result (0) and the mode request's outcome (`None`: a
/// fatal path, logged in `ActionHooks::errors`).
pub type WalkResult = (u32, Option<Outcome>);

/// Turns the path provider on for a game wired on `ActionSim`
/// (`ActionHooks::enable_paths`; call it before any unit is allocated, so
/// every unit gets its path record).
pub fn enable_paths<D: ActionEvents>(events: &mut D) -> Result<(), PathError> {
    events.action().hooks().enable_paths()
}

/// The walk / run handler on the action wiring:
/// `None` when the game's path provider is off.
pub fn run<D: ActionEvents>(game: &mut Game, events: &mut D, call: WalkCall) -> Option<WalkResult> {
    let action = events.action();
    action.hooks().paths.as_ref()?;
    if call.id == RESYNC_ID {
        let mut m = [RESYNC_ID, 0, 0, 0, 0];
        m[1..3].copy_from_slice(&(call.a as u16).to_le_bytes());
        m[3..5].copy_from_slice(&(call.b as u16).to_le_bytes());
        return Some(action.with(game, |g, v| {
            let (r, what) = d2_sim::wiring::path::walk::resync_message(v, g, call.player, &m);
            let o = match what {
                Some(d2_sim::path::walk::resync::Resync::Walk(o)) => Some(o),
                _ => None,
            };
            (r, o)
        }));
    }
    Some(action.with(game, |g, v| {
        d2_sim::wiring::path::walk::walk_message(v, g, call.player, call.id, call.a, call.b)
    }))
}

fn u16_at(m: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        m.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn u32_at(m: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(m.get(at..at + 4)?.try_into().ok()?))
}

/// The walk / run handler (and 0x5F, [`RESYNC_ID`]) for one dispatched
/// message. `None`: not a walk id, no player, or the host has no path
/// provider: the caller keeps its stub.
pub fn handle<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    let id = *msg.first()?;
    let (a, b) = match (id, form(id)) {
        (RESYNC_ID, _) | (_, Some(Form::Point)) => (u16_at(msg, 1)?, u16_at(msg, 3)?),
        (_, Some(Form::Unit)) => (u32_at(msg, 1)?, u32_at(msg, 5)?),
        (_, None) => return None,
    };
    let player = sim.player_of(client)?;
    let call = WalkCall { player, id, a, b };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let (r, _) = sim.world.walk(game, events, call)?;
    let mut faults = Vec::new();
    for (unit, bytes) in sim.world.take_sent(&mut sim.events) {
        // A player without a client receives nothing.
        if let Some(c) = sim.client_of(unit) {
            if let Err(e) = out.queue(c, &bytes) {
                faults.push(WorldError::from(e));
            }
        }
    }
    if !faults.is_empty() {
        for error in faults {
            sim.world.fault(WorldFault { client, id, error });
        }
        return Some(ResultCode::Malformed);
    }
    Some(super::skills::code(r as i32))
}
