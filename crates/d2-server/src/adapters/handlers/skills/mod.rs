//! Skill and combat intent handlers: skill use (0x05–0x11), select skill
//! (0x3C), stat points (0x3A), skill points (0x3B).
//!
//! The dispatcher (`crate::dispatch`) has already applied the gate, the
//! exact size and the point / unit parse when [`handle`] runs. The
//! handlers are the `d2-sim` functions that own these rules (skill use,
//! skill levels, vitals), run by the game's world host
//! ([`WorldHost::skill`], a [`SkillHost`] slot); a host without one leaves
//! the ids to the stub.
//! [`wired::WiredSkills`] is the slot of the wired sim
//! (`d2_sim::wiring::action::ActionSim`): it runs the handlers on the
//! skill use pipeline's `d2-sim` provider
//! (`d2_sim::wiring::interaction::UseView`, through [`world::World`]).
//!
//! Server messages: the pipeline's 0x15 (resync) goes to
//! [`SimGame::resyncs`] like the dispatcher's; 0x5A ("can't do that") is
//! queued to the caster's client as its 40 bytes ([`wired::CANT_DO`], sent
//! when the cast fails for lack of mana). No other S→C message is defined
//! for these handlers.

pub mod wired;
pub mod world;

#[cfg(test)]
mod tests;

use d2_sim::game::Game;
use d2_sim::tick::EventDispatch;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::Pending;
use d2_sim::wiring::interaction::UseRest;

use super::super::SimGame;
use super::world::WorldHost;
use crate::dispatch::{is_point, is_unit};
use crate::seams::{ClientId, Intents, MessageSink, Pos, ResultCode, UnitTarget};

/// Status of a client id in this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Routed to a handler here.
    Handled,
    /// No `d2-sim` code owns the behaviour yet: stays a stub.
    Stub,
    /// Owned by another handler module (named).
    OtherModule(&'static str),
}

/// Every skill / combat client id, its owner note and status.
pub const IDS: &[(u8, &str, &str, Status)] = &[
    (0x05, "ShiftLeftSkill", "skills/use.md §1", Status::Handled),
    (0x06, "LeftSkillOnUnit", "skills/use.md §1", Status::Handled),
    (
        0x07,
        "ShiftLeftSkillOnUnit",
        "skills/use.md §1",
        Status::Handled,
    ),
    (
        0x08,
        "ShiftLeftSkillHold",
        "skills/use.md §1 rule 6",
        Status::Handled,
    ),
    (
        0x09,
        "LeftSkillOnUnitHold",
        "skills/use.md §1 rule 6",
        Status::Handled,
    ),
    (
        0x0A,
        "ShiftLeftSkillOnUnitHold",
        "skills/use.md §1 rule 6",
        Status::Handled,
    ),
    (
        0x0B,
        "Unused0B",
        "sim/client-messages.tsv (returns 0)",
        Status::Handled,
    ),
    (0x0C, "RightSkill", "skills/use.md §1", Status::Handled),
    (
        0x0D,
        "RightSkillOnUnit",
        "skills/use.md §1",
        Status::Handled,
    ),
    (
        0x0E,
        "ShiftRightSkillOnUnit",
        "skills/use.md §1",
        Status::Handled,
    ),
    (
        0x0F,
        "RightSkillHold",
        "skills/use.md §1 rule 6",
        Status::Handled,
    ),
    (
        0x10,
        "RightSkillOnUnitHold",
        "skills/use.md §1 rule 6",
        Status::Handled,
    ),
    (
        0x11,
        "ShiftRightSkillOnUnitHold",
        "skills/use.md §1 rule 6",
        Status::Handled,
    ),
    (
        0x12,
        "EndInferno",
        "sim/intents-events.md §9 r2",
        Status::OtherModule("player"),
    ),
    (0x3A, "AddStatPoint", "combat/vitals.md §2", Status::Handled),
    (
        0x3B,
        "AddSkillPoint",
        "skills/levels.md §6.4",
        Status::Handled,
    ),
    (
        0x3C,
        "SelectSkill",
        "skills/use.md §7; sim/intents-events.md §2.4 rule 7",
        Status::Handled,
    ),
    (
        0x41,
        "Resurrect",
        "sim/intents-events.md §9 r6",
        Status::OtherModule("player"),
    ),
    (
        0x51,
        "BindHotkey",
        "sim/intents-events.md §9 r12 (fields: §2.4 rule 7)",
        Status::OtherModule("player"),
    ),
];

/// The ids routed here.
pub fn handled(id: u8) -> bool {
    IDS.iter()
        .any(|&(i, _, _, s)| i == id && s == Status::Handled)
}

/// What the server stages for one message (`adapters::sim` staging):
/// the inputs of the validators `d2-sim` re-runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    /// The client's player unit.
    pub player: UnitId,
    /// The player data's last accepted point-target frame; `None`: no player data.
    pub last_accept: Option<i32>,
    /// Staged positions of the player and the message's target.
    pub positions: Vec<(UnitId, Pos)>,
    /// The target is an item the player owns.
    pub owned_item: Option<UnitId>,
    /// The target is in another act.
    pub other_act: Option<UnitId>,
}

/// One routed message.
pub struct Call<'a, D> {
    pub game: &'a mut Game,
    pub events: &'a mut D,
    pub client: ClientId,
    /// The drained message (exact size, parsed by the dispatcher).
    pub msg: &'a [u8],
    pub staged: Staged,
}

/// A handler's result and the staged fields it wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handled {
    pub code: ResultCode,
    /// The new last accepted point-target frame.
    pub point_accept: Option<i32>,
    /// The pipeline asked for S→C 0x15.
    pub resync: bool,
    /// The pipeline sent S→C 0x5A "can't do that" ([`wired::CANT_DO`]).
    pub cant_do: bool,
}

/// The skill handlers of a game whose events are `D`: a slot of the
/// world host ([`WorldHost::skill`]).
pub trait SkillHost<D> {
    /// The handler of `call.msg[0]`, one of the [`Status::Handled`] ids;
    /// `None`: this slot runs no skill handler, the id stays a stub.
    fn handle(&mut self, call: Call<'_, D>) -> Option<Handled>;
}

/// No skill handlers: every skill id stays a stub.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSkills;

impl<D> SkillHost<D> for NoSkills {
    fn handle(&mut self, _call: Call<'_, D>) -> Option<Handled> {
        None
    }
}

/// The skill-point calls `d2-sim` does not implement itself, answered by
/// the action wiring's `Pending` value, the skill list's one owner
/// (`UseRest`).
pub trait LearnRest {
    /// Whether `skill` belongs to the unit's class.
    fn is_class_skill(&self, u: UnitId, skill: i32) -> bool;
    /// After the cost check: spend, add a level, refresh, apply the passive
    /// state; refund on failure.
    fn add_skill_level(&mut self, u: UnitId, skill: i32, cost: i32);
    /// The follow-up that runs after every skill-point message.
    fn after_skill_point(&mut self, u: UnitId);
}

/// What the wired skill handlers need of the action wiring's `Pending`
/// value: its own seams, the skill use pipeline's rest and the
/// skill-point calls.
pub trait SkillRest: Pending + UseRest + LearnRest {}

impl<T: Pending + UseRest + LearnRest> SkillRest for T {}

/// A `d2-sim` handler result as a dispatch result code (the handlers
/// return 0–3).
pub fn code(c: i32) -> ResultCode {
    match c {
        0 => ResultCode::Done,
        1 => ResultCode::Refused,
        2 => ResultCode::Invalid,
        _ => ResultCode::Malformed,
    }
}

/// The skill handler of `msg[0]`, if the id is routed here and the
/// world host has a skill slot that runs it; `None` leaves the id to the
/// stub.
pub fn handle<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    let id = *msg.first()?;
    if !handled(id) {
        return None;
    }
    let player = sim
        .sim_client(client)
        .and_then(|c| sim.game.lists.client(c)?.player)?;
    let staged = staged(sim, client, player, msg);
    let h = sim.world.skill(Call {
        game: &mut sim.game,
        events: &mut sim.events,
        client,
        msg,
        staged,
    })?;
    if let Some(f) = h.point_accept {
        sim.set_point_accept(client, f);
    }
    if h.resync {
        sim.queue_resync(client, out);
    }
    // A full queue is the host's fault elsewhere (as `queue_set_skill`).
    if h.cant_do {
        let _ = out.queue(client, &wired::CANT_DO);
    }
    if id == SELECT_SKILL_ID && h.code == ResultCode::Done {
        queue_set_skill(sim, client, player, msg, out);
    }
    Some(h.code)
}

/// 0x3C SelectSkill: the id.
const SELECT_SKILL_ID: u8 = 0x3C;

/// A successful selection of a player's skill ends with S→C 0x23
/// SetSkill (queued every time, even for the skill already selected):
/// type 0, the player's GUID, hand 1 left / 0 right, the skill and the
/// entry's owner GUID (`tables/sim/server-messages.tsv`, 0x23). The client
/// reads the selection from it; without it the mouse skill never changes
/// on screen.
fn queue_set_skill<D: EventDispatch, W: WorldHost<D>>(
    sim: &SimGame<D, W>,
    client: ClientId,
    player: UnitId,
    msg: &[u8],
    out: &mut dyn MessageSink,
) {
    let Some(guid) = sim.game.lists.unit(player).map(|e| e.guid) else {
        return;
    };
    if let Some(m) = set_skill_reply(guid, msg) {
        // A full queue is the host's fault elsewhere; the selection stands.
        let _ = out.queue(client, &m);
    }
}

/// The S→C 0x23 that answers a C→S 0x3C `msg` of the player `guid`
/// (`queue_set_skill`); `None` for a message of the wrong size.
pub fn set_skill_reply(guid: u32, msg: &[u8]) -> Option<[u8; 13]> {
    let msg: &[u8; 9] = msg.try_into().ok()?;
    let word = u32::from_le_bytes([msg[1], msg[2], msg[3], msg[4]]);
    let owner = u32::from_le_bytes([msg[5], msg[6], msg[7], msg[8]]);
    let skill = (word & 0x7FFF_FFFF) as u16;
    let hand = u8::from(word >> 31 != 0);
    Some(d2_sim::units::messages::set_skill(
        0, guid, hand, skill, owner,
    ))
}

/// The staged inputs of the message: the player's data and the positions
/// or target facts the point / unit validators need.
fn staged<D: EventDispatch, W: WorldHost<D>>(
    sim: &SimGame<D, W>,
    client: ClientId,
    player: UnitId,
    msg: &[u8],
) -> Staged {
    let mut s = Staged {
        player,
        last_accept: sim
            .player_fields(player)
            .and_then(|p| p.data)
            .map(|d| d.last_accept),
        positions: Vec::new(),
        owned_item: None,
        other_act: None,
    };
    let id = msg[0];
    if is_point(id) {
        if let Some(p) = sim.point_state(client) {
            s.positions.push((player, p.player));
        }
    } else if is_unit(id) {
        let word = |o: usize| {
            msg.get(o..o + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let (Some(ty), Some(guid)) = (word(1), word(5)) else {
            return s;
        };
        let target = UnitType::ALL
            .get(ty as usize)
            .and_then(|&t| sim.game.lists.find_unit(t, guid));
        let Some(target) = target else {
            return s;
        };
        match sim.unit_target(client, ty, guid) {
            UnitTarget::OwnedItem => s.owned_item = Some(target),
            UnitTarget::OtherAct => s.other_act = Some(target),
            UnitTarget::At {
                player: p,
                target: t,
            } => {
                s.positions.push((player, p));
                s.positions.push((target, t));
            }
            UnitTarget::Missing => {}
        }
    }
    s
}
