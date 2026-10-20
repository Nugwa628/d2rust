// D2RUST's own: dev tools.
//! The dev panel's end of the dev commands: the channel between the
//! overlay and the server thread, a loaded slot's way back, and the
//! before-pump hook that runs both. The handlers themselves are
//! [`d2_server::dev`], shared with the dedicated test server; here they
//! run for the local player, between two server frames, as plain functions
//! on the server's game ([`Sim`]), never a Bevy system.

use std::sync::{Arc, Mutex};

use d2_proto::dev::{DevCommand, DevReply};
use d2_sim::poke::{Coord, Directive, PokeOp};
use d2_sim::units::ClientId;

use super::slots::DevStart;
use crate::app::single_player::{local_player, Link, Sim};
use crate::bridge::link::LOCAL_CLIENT;
use d2_server::dev::{result, switches, DevServer as Core};

pub use d2_server::dev::DevCatalog;

/// The channel between the overlay and the server thread: commands in, replies
/// out, in order.
#[derive(Debug, Default)]
pub struct DevQueue {
    pub commands: Vec<DevCommand>,
    pub replies: Vec<DevReply>,
}

/// The overlay's end of the channel.
pub type DevChannel = Arc<Mutex<DevQueue>>;

/// A loaded slot's way back: frames waited after the join,
/// then the warp, then the placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartStep {
    Wait(u32),
    Warping(u32),
    Place,
}

/// Frames after the join before a loaded slot's warp.
const START_WAIT: u32 = 25;
/// Frames a loaded slot's warp may take.
const START_WARP: u32 = 500;

/// The handlers' state on the server thread, and a loaded slot's way back.
#[derive(Debug, Default)]
pub struct DevServer {
    core: Core,
    start: Option<(DevStart, StartStep)>,
}

impl DevServer {
    pub fn new(catalog: DevCatalog) -> Self {
        Self {
            core: Core::new(catalog),
            start: None,
        }
    }

    /// With a loaded slot's place and switches.
    pub fn with_start(mut self, start: Option<DevStart>) -> Self {
        self.start = start.map(|s| (s, StartStep::Wait(0)));
        self
    }

    /// One step of a loaded slot's way back; the reply when it ends.
    fn step_start(&mut self, s: &mut Sim) -> Option<DevReply> {
        let (start, step) = self.start?;
        let in_game = s
            .game
            .lists
            .client(ClientId(LOCAL_CLIENT))
            .is_some_and(|c| c.state == d2_sim::units::lists::client_state::IN_GAME);
        let (p, _) = local_player(s)?;
        if !in_game {
            return None;
        }
        let next = match step {
            StartStep::Wait(n) if n < START_WAIT => StartStep::Wait(n + 1),
            StartStep::Wait(_) => {
                if player_level(s, p) == Some(start.level) {
                    StartStep::Place
                } else if let Err(e) = warp(s, p, start.level) {
                    self.start = None;
                    return Some(DevReply::error(format!("load: warp {}: {e}", start.level)));
                } else {
                    StartStep::Warping(0)
                }
            }
            StartStep::Warping(n) if n < START_WARP && player_level(s, p) != Some(start.level) => {
                StartStep::Warping(n + 1)
            }
            StartStep::Warping(_) | StartStep::Place => {
                self.start = None;
                let op = PokeOp::Directive(Directive::Pos {
                    unit: d2_sim::poke::UnitArg::Player,
                    x: Coord::Num(start.x),
                    y: Coord::Num(start.y),
                    free: true,
                });
                let placed = result(d2_server::dev::apply_now(s, p, &op));
                let w = start.switches;
                for c in [
                    DevCommand::God(Some(w.god)),
                    DevCommand::NoCooldown(Some(w.nocd)),
                    DevCommand::FreeMana(Some(w.freemana)),
                ] {
                    self.core.apply(s, Some(p), &c);
                }
                if w.speed != 0 {
                    self.core.apply(s, Some(p), &DevCommand::Speed(w.speed));
                }
                let mut r = match placed {
                    Ok(_) => DevReply::ok(format!(
                        "slot loaded: level {} at ({}, {})",
                        start.level, start.x, start.y
                    )),
                    Err(e) => DevReply::error(format!("slot loaded, placement: {e}")),
                };
                r.switches = switches(s, p);
                return Some(r);
            }
        };
        self.start = Some((start, next));
        None
    }

    /// The hook's body: the queued commands, then the upkeep.
    pub fn run_due<C: d2_server::seams::Clock>(&mut self, chan: &DevChannel, l: &mut Link<C>) {
        let commands = match chan.lock() {
            Ok(mut q) => std::mem::take(&mut q.commands),
            Err(_) => return,
        };
        let s = &mut l.host_mut().game;
        let player = local_player(s).map(|(p, _)| p);
        let mut replies: Vec<DevReply> = self.step_start(s).into_iter().collect();
        replies.extend(commands.iter().map(|c| self.apply(s, c)));
        if let Some(p) = player {
            d2_server::dev::upkeep(s, p);
        }
        if !replies.is_empty() {
            if let Ok(mut q) = chan.lock() {
                q.replies.extend(replies);
            }
        }
    }

    /// Runs one command for the local player now; the reply carries the
    /// switch states after it.
    pub fn apply(&mut self, s: &mut Sim, cmd: &DevCommand) -> DevReply {
        self.core.apply(s, local_player(s).map(|(p, _)| p), cmd)
    }
}

/// The level of the room `p` stands in.
fn player_level(s: &mut Sim, p: d2_sim::units::UnitId) -> Option<u32> {
    let room = s.game.lists.unit(p)?.room()?;
    s.events.action.hooks().drlg.level_id(&s.game, room)
}

fn warp(s: &mut Sim, p: d2_sim::units::UnitId, level: u32) -> Result<(), String> {
    let op = PokeOp::Directive(Directive::Warp { level, tile: None });
    result(d2_server::dev::apply_now(s, p, &op)).map(|_| ())
}

/// Builds the before-pump hook body of the dev tools: the server state
/// and the channel's server end.
pub fn hook<C: d2_server::seams::Clock>(
    mut server: DevServer,
    chan: DevChannel,
) -> impl FnMut(&mut Link<C>) + Send {
    move |l| server.run_due(&chan, l)
}
