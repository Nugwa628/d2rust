//! `state-dump --send "<f> <Name> <field>=<value>..."` and `--send "<f>
//! hex <byte>..."`: scripted C→S messages, injected through the bridge
//! ([`Bridge::inject`](crate::bridge::Bridge::inject)) after the snapshot
//! of frame f − 1 (after the pokes and the input of that point), before
//! frame f's drain: the point a scripted check injects at (the first
//! drain-call stop after tick f − 1).
//!
//! The message syntax, encoding and references are the scenario script's
//! (`crate::scenario_script::parse_message`, `encode`); references
//! are resolved on the server game on the server thread (the unit lists
//! the state snapshot reads): `@player` the
//! local player, `@x` / `@y` its path position, `@wp` from the waypoint
//! table of the game. The bytes go to the host's transport send
//! (`Host::send_system`: the classifier and the queues, no duplicate
//! filter, for game and system ids alike).

use std::collections::BTreeSet;

use crate::scenario_script::{self as script, StepMsg, UnitRef, Unresolved};
use d2_server::seams::Clock;
use d2_server::transport::Classified;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};

use super::poke::waypoint_classes;
use super::server_thread::{ThreadLink, ThreadStopped};
use super::single_player::{local_player, Link, Sim};
use crate::bridge::inject::{InjectTarget, Injected};
use crate::bridge::link::LOCAL_CLIENT;

/// One scheduled message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendEntry {
    /// Absolute server frame f: injected once frame f − 1 has run.
    pub frame: i32,
    pub msg: StepMsg,
}

impl SendEntry {
    /// The message as written canonically (`Name field=value...` or `hex...`).
    pub fn text(&self) -> String {
        script::message_text(&self.msg).unwrap_or_default()
    }
}

/// Parses one `--send` value: `<f> <Name> <field>=<value>...` or `<f> hex
/// <byte>...` (strict errors).
pub fn parse_send_arg(s: &str) -> Result<SendEntry, String> {
    let toks: Vec<&str> = s.split_ascii_whitespace().collect();
    let Some((f, rest)) = toks.split_first() else {
        return Err(
            "--send \"<frame> <Name> <field>=<value>...\" or \"<frame> hex <byte>...\"".into(),
        );
    };
    let frame: i32 = f
        .parse()
        .ok()
        .filter(|&f| f >= 1)
        .ok_or_else(|| format!("--send: frame {f:?}: a server frame ≥ 1"))?;
    let msg = script::parse_message(rest).map_err(|e| format!("--send {s:?}: {e}"))?;
    Ok(SendEntry { frame, msg })
}

/// The game state references resolve against.
struct View<'a> {
    sim: &'a Sim,
    player: UnitId,
    waypoints: BTreeSet<u32>,
}

impl View<'_> {
    fn class(&self, id: UnitId) -> u32 {
        self.sim
            .events
            .action
            .sys
            .units
            .get(id)
            .map_or(0, |u| u.class)
    }
}

impl script::World for View<'_> {
    fn player(&self) -> Option<(u32, i32, i32)> {
        let guid = self.sim.game.lists.unit(self.player)?.guid;
        let (x, y) = self.sim.events.action.sys.hooks.path_position(self.player);
        Some((guid, x, y))
    }

    fn units(&self) -> Vec<UnitRef> {
        let lists = &self.sim.game.lists;
        UnitType::ALL
            .iter()
            .flat_map(|&ty| {
                lists
                    .units_of_type(ty)
                    .into_iter()
                    .filter_map(move |id| lists.unit(id).map(|e| (ty, e.guid, id)))
            })
            .map(|(ty, guid, id)| {
                let class = self.class(id);
                UnitRef {
                    ty: ty.index() as u8,
                    class,
                    guid,
                    pos: Some(self.sim.events.action.sys.hooks.path_position(id)),
                    waypoint: ty == UnitType::Object && self.waypoints.contains(&class),
                }
            })
            .collect()
    }
}

/// The pending sends of a `play` run: due once frame f − 1 has run (the
/// state the host stands at before its next pump), in the order given.
#[derive(Debug, Default)]
pub struct SendSchedule {
    pending: Vec<SendEntry>,
    done: usize,
}

impl SendSchedule {
    pub fn new(entries: Vec<SendEntry>) -> Self {
        Self {
            pending: entries,
            done: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Injects every due entry and prints its
    /// record line.
    pub fn run_due<C: Clock>(&mut self, l: &mut Link<C>) {
        if self.pending.is_empty() {
            return;
        }
        let frame = l.host().game.game.frame;
        let (due, later): (Vec<SendEntry>, Vec<SendEntry>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|e| frame >= e.frame - 1);
        self.pending = later;
        for e in due {
            let r = inject_now(l, &e.msg);
            let src = script::message_text(&e.msg).unwrap_or_default();
            eprintln!("{}", record_line(e.frame, self.done, &src, &r));
            self.done += 1;
        }
    }
}

/// `play --poke` / `--send`: the pokes due after a tick run at the tick end (as
/// `state-dump`); the others and the sends in one hook before every host frame,
/// pokes first, then sends (as `state-dump` orders them after the snapshot of f
/// − 1).
pub fn install<C: Clock + Send + 'static>(
    link: &mut ThreadLink<Link<C>>,
    pokes: Vec<super::poke::Entry>,
    sends: Vec<SendEntry>,
) -> Result<(), ThreadStopped> {
    let (tick_end, pokes) = super::poke::split_tick_end(pokes);
    super::poke::install_tick_end(link, tick_end)?;
    if pokes.is_empty() && sends.is_empty() {
        return Ok(());
    }
    let mut pokes = super::poke::Schedule::new(pokes);
    let mut sends = SendSchedule::new(sends);
    link.set_before_pump(move |l: &mut Link<C>| {
        pokes.run_due(l);
        sends.run_due(l);
    })
}

/// Resolves and injects `msg` on the server now (between two frames). Before
/// the local client is in game (state 4) nothing is sent, as in a scripted
/// check.
pub fn inject_now<C: Clock>(l: &mut Link<C>, msg: &StepMsg) -> Injected {
    let s = &l.host().game;
    let in_game = s
        .game
        .lists
        .client(d2_sim::units::ClientId(LOCAL_CLIENT))
        .is_some_and(|c| c.state == client_state::IN_GAME);
    let unresolved = |reference: &str, why: &str| {
        Injected::Unresolved(Unresolved {
            reference: reference.into(),
            why: why.into(),
        })
    };
    if !in_game {
        return unresolved(
            "@player",
            "client 0 not in state 4 yet (original-hooks.md §1 rule 5)",
        );
    }
    let Some((player, _)) = local_player(s) else {
        return unresolved("@player", "no local player");
    };
    let view = View {
        sim: s,
        player,
        waypoints: waypoint_classes(s),
    };
    let bytes = match script::encode(msg, &view) {
        Ok(b) => b,
        Err(u) => return Injected::Unresolved(u),
    };
    match l.host_mut().send_system(LOCAL_CLIENT, &bytes) {
        Ok(Classified::Queued(_)) => Injected::Sent {
            bytes,
            queued: true,
            why: None,
        },
        Ok(c) => Injected::Sent {
            bytes,
            queued: false,
            why: Some(format!("classifier: {c:?}")),
        },
        Err(e) => Injected::Sent {
            bytes,
            queued: false,
            why: Some(format!("transport send: {e}")),
        },
    }
}

/// The server thread injects between two frames.
impl<C: Clock + Send + 'static> InjectTarget for ThreadLink<Link<C>> {
    type Error = ThreadStopped;
    fn inject(&mut self, msg: &StepMsg) -> Result<Injected, Self::Error> {
        let msg = msg.clone();
        self.with(move |l| inject_now(l, &msg))
    }
}

/// The `send` record of one result (the fields of the send
/// record): `f` the frame the message precedes, `frame` = f − 1, `i` its
/// index among the sends of that point, `r` (`ok`: queued; `dropped`:
/// the classifier refused it, `note` says how; `unresolved`: nothing
/// sent, `note` the reference), `bytes` (hex) when sent, `src` the
/// message as written.
pub fn record_line(f: i32, i: usize, src: &str, r: &Injected) -> String {
    use d2_sim::debug::state::json_string;
    let mut o = format!("{{\"k\":\"send\",\"f\":{f},\"frame\":{},\"i\":{i}", f - 1);
    match r {
        Injected::Sent { bytes, queued, why } => {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            o.push_str(&format!(
                ",\"r\":{},\"bytes\":{}",
                json_string(if *queued { "ok" } else { "dropped" }),
                json_string(&hex)
            ));
            if let Some(w) = why {
                o.push_str(&format!(",\"note\":{}", json_string(w)));
            }
        }
        Injected::Unresolved(u) => {
            let note = if u.why.starts_with(&u.reference) {
                u.why.clone()
            } else {
                format!("{}: {}", u.reference, u.why)
            };
            o.push_str(&format!(
                ",\"r\":\"unresolved\",\"note\":{}",
                json_string(&note)
            ));
        }
    }
    o.push_str(&format!(",\"src\":{}}}", json_string(src)));
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_options_parse_and_encode_as_scenario_steps() {
        struct W;
        impl script::World for W {
            fn player(&self) -> Option<(u32, i32, i32)> {
                Some((1, 100, 200))
            }
            fn units(&self) -> Vec<UnitRef> {
                vec![UnitRef {
                    ty: 1,
                    class: 148,
                    guid: 9,
                    pos: Some((100, 200)),
                    waypoint: false,
                }]
            }
        }
        let e = parse_send_arg("12 Walk x=10 y=20").unwrap();
        assert_eq!(e.frame, 12);
        assert_eq!(script::encode(&e.msg, &W).unwrap(), [1, 0x0a, 0, 0x14, 0]);
        let e = parse_send_arg("3 SelectSkill skill=36 left=0 item=0xFFFFFFFF").unwrap();
        assert_eq!(
            script::encode(&e.msg, &W).unwrap(),
            [0x3c, 0x24, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]
        );
        let e = parse_send_arg("5 InteractWithEntity type=1 id=@1:148").unwrap();
        assert_eq!(e.text(), "InteractWithEntity type=1 id=@1:148");
        assert_eq!(
            script::encode(&e.msg, &W).unwrap(),
            [0x13, 1, 0, 0, 0, 9, 0, 0, 0]
        );
        let e = parse_send_arg("5 hex 2f 00 00 00 00 09 00 00 00").unwrap();
        assert_eq!(e.text(), "hex 2f 00 00 00 00 09 00 00 00");
        for bad in [
            "",
            "0 Walk x=1 y=2",
            "x Walk x=1 y=2",
            "4",
            "4 Walk x=1",
            "4 hex zz",
            "4 Chat",
        ] {
            assert!(parse_send_arg(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn send_record_lines() {
        let ok = Injected::Sent {
            bytes: vec![1, 10, 0, 20, 0],
            queued: true,
            why: None,
        };
        assert_eq!(
            record_line(12, 0, "Walk x=10 y=20", &ok),
            r#"{"k":"send","f":12,"frame":11,"i":0,"r":"ok","bytes":"010a001400","src":"Walk x=10 y=20"}"#
        );
        let un = Injected::Unresolved(Unresolved {
            reference: "@1:148".into(),
            why: "@1:148: no such unit".into(),
        });
        assert_eq!(
            record_line(5, 1, "InteractWithEntity type=1 id=@1:148", &un),
            r#"{"k":"send","f":5,"frame":4,"i":1,"r":"unresolved","note":"@1:148: no such unit","src":"InteractWithEntity type=1 id=@1:148"}"#
        );
    }
}
