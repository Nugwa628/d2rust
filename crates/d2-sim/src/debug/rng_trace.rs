//! The RNG draw log (`rng-trace` feature, off by default): every seeded
//! draw of [`Seed`] (step, the helpers, `derive`) with its op, arguments,
//! state before and after, returned value, the seed's address and the
//! draw site (`#[track_caller]`: the file and line that called the
//! helper), plus a marker at the start of every tick.
//!
//! The log is per thread and off until [`start`] is called on that
//! thread; only the debug export ([`drain`]) reads it. Recording copies
//! values that the draw computed anyway: no draw, state or order depends
//! on whether the feature is compiled in or the log is on (the sim stays
//! deterministic; `tests::outcomes_do_not_depend_on_the_log`).
//!
//! [`Owners`] assigns each drained draw its owner: the game seed, a
//! unit's seed (`unit T:G`), or `other`, by following each known seed's
//! value chain, and writes the `rng-raw-1` draw lines.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::panic::Location;

use crate::game::Game;
use crate::rng::Seed;
use crate::units::dispatch::UnitSystem;
use crate::units::UnitType;
use crate::wiring::action::ActionHooks;
use crate::wiring::worldgen::WorldSim;

/// The format of the D2RUST file (the same as `record_rng.py`'s raw files).
pub const FORMAT: &str = "rng-raw-1";

/// A draw's op and arguments: one variant per `Seed` draw helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// A bare step of the seed.
    Step,
    /// A roll below `n`.
    Roll(i32),
    /// A draw masked with `n`.
    Mask(u32),
    /// A masked draw offset from `min`: (`min`, `n`).
    MaskRange(i32, u32),
    /// A roll over a range starting at `min`: (`min`, `n`).
    RollRange(i32, i32),
}

impl Op {
    /// The op name written in the `op` key of a draw line.
    pub fn name(self) -> &'static str {
        match self {
            Op::Step => "step",
            Op::Roll(_) => "roll",
            Op::Mask(_) => "mask",
            Op::MaskRange(..) => "mask_range",
            Op::RollRange(..) => "roll_range",
        }
    }

    /// (`min`, `n`) as written to the log, both as u32.
    fn args(self) -> (Option<u32>, Option<u32>) {
        match self {
            Op::Step => (None, None),
            Op::Roll(n) => (None, Some(n as u32)),
            Op::Mask(n) => (None, Some(n)),
            Op::MaskRange(min, n) => (Some(min as u32), Some(n)),
            Op::RollRange(min, n) => (Some(min as u32), Some(n as u32)),
        }
    }
}

/// One recorded draw.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    /// The helper that ran and its arguments.
    pub op: Op,
    /// The seed before the draw.
    pub before: Seed,
    /// The seed after the draw.
    pub after: Seed,
    /// The returned value as a u32 (signed results wrap).
    pub ret: u32,
    /// The address of the seed drawn from (an identity hint only).
    pub addr: usize,
    /// The caller of the helper.
    pub site: &'static Location<'static>,
}

/// One log entry.
#[derive(Clone, Copy, Debug)]
pub enum Entry {
    /// A tick started; `Game::frame` is now this value.
    Frame(i32),
    /// A seeded draw.
    Draw(Draw),
}

thread_local! {
    static LOG: RefCell<Option<Vec<Entry>>> = const { RefCell::new(None) };
}

/// Turns the log on for this thread (empty).
pub fn start() {
    LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
}

/// Turns the log off for this thread; returns what it still held.
pub fn stop() -> Vec<Entry> {
    LOG.with(|l| l.borrow_mut().take().unwrap_or_default())
}

/// Whether the log is on for this thread.
pub fn is_on() -> bool {
    LOG.with(|l| l.borrow().is_some())
}

/// Takes the entries logged so far (the log stays on).
pub fn drain() -> Vec<Entry> {
    LOG.with(|l| {
        l.borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

/// Called by every [`Seed`] draw (after it ran).
#[track_caller]
pub(crate) fn draw(seed: &Seed, op: Op, before: Seed, ret: u32) {
    let site = Location::caller();
    LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push(Entry::Draw(Draw {
                op,
                before,
                after: *seed,
                ret,
                addr: std::ptr::from_ref(seed) as usize,
                site,
            }));
        }
    });
}

/// Called at the start of every tick, after `frame += 1`.
pub(crate) fn mark_frame(frame: i32) {
    LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push(Entry::Frame(frame));
        }
    });
}

/// Who owns a seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    /// The game seed.
    Game,
    /// A server unit's seed: unit type, GUID.
    Unit(u8, u32),
}

impl Owner {
    /// `game` or `unit T:G`.
    pub fn label(self) -> String {
        match self {
            Owner::Game => "game".into(),
            Owner::Unit(t, g) => format!("unit {t}:{g}"),
        }
    }
}

/// A seed whose owner is known, with its value and address now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Known {
    /// Who the seed belongs to.
    pub owner: Owner,
    /// The seed's current value.
    pub seed: Seed,
    /// The seed's address now.
    pub addr: usize,
}

/// The game seed and every unit seed of the server's unit lists, read
/// only (the units of `debug::state::snapshot`).
pub fn known<X>(game: &Game, sys: &UnitSystem<ActionHooks<X>>) -> Vec<Known> {
    let g = &sys.hooks.game_seed;
    let mut out = vec![Known {
        owner: Owner::Game,
        seed: *g,
        addr: std::ptr::from_ref(g) as usize,
    }];
    for ty in UnitType::ALL {
        for id in game.lists.units_of_type(ty) {
            let (Some(e), Some(r)) = (game.lists.unit(id), sys.units.get(id)) else {
                continue;
            };
            out.push(Known {
                owner: Owner::Unit(ty as u8, e.guid),
                seed: r.seed,
                addr: std::ptr::from_ref(&r.seed) as usize,
            });
        }
    }
    out
}

/// [`known`] of a [`WorldSim`].
pub fn known_world<X>(game: &Game, sim: &WorldSim<X>) -> Vec<Known> {
    known(game, &sim.action.sys)
}

/// A drained draw with its frame and owner.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// The tick the draw ran in.
    pub frame: i32,
    /// The seed's owner, or `None` when no known seed matches.
    pub owner: Option<Owner>,
    /// The draw itself.
    pub draw: Draw,
}

/// A draw in the DRLG code: never a game or unit draw (with `-seed N` the
/// DRLG seed replays the game seed's states, so matching by value would
/// mislabel it).
fn is_drlg(d: &Draw) -> bool {
    d.site
        .file()
        .replace('\\', "/")
        .contains("d2-sim/src/drlg/")
}

/// The owner assignment across drains: the values each owner had at
/// the previous drain, the frame, and a stable number per unknown seed
/// address (for the `seed` key).
#[derive(Debug, Default)]
pub struct Owners {
    prev: Vec<Known>,
    frame: i32,
    others: Vec<usize>,
    seq: u64,
}

impl Owners {
    /// An assignment with no earlier drain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Frames and owners of `entries` (one drain), given the known seeds
    /// now (`end`). Forward from the previous drain's values, then
    /// backward from `end` for the draws still open.
    pub fn resolve(&mut self, entries: Vec<Entry>, end: Vec<Known>) -> Vec<Resolved> {
        let mut out = Vec::new();
        for e in entries {
            match e {
                Entry::Frame(f) => self.frame = f,
                Entry::Draw(d) => out.push(Resolved {
                    frame: self.frame,
                    owner: None,
                    draw: d,
                }),
            }
        }
        let addr_of = |o: Owner| end.iter().find(|k| k.owner == o).map(|k| k.addr);
        // Seeds are found by value (the game seed is drawn on copies and
        // unit records move); the address only breaks ties.
        let pick = |cands: Vec<Owner>, addr: usize| -> Option<Owner> {
            cands
                .iter()
                .copied()
                .find(|&o| addr_of(o) == Some(addr))
                .or_else(|| cands.first().copied())
        };
        // forward from the previous values
        let mut cur: Vec<(Owner, Seed)> = self.prev.iter().map(|k| (k.owner, k.seed)).collect();
        let mut linked: Vec<Owner> = Vec::new();
        for r in out.iter_mut() {
            let d = r.draw;
            if d.before == d.after || is_drlg(&d) {
                continue; // no step (roll with n < 1): no link
            }
            let cands: Vec<Owner> = cur
                .iter()
                .filter(|(_, s)| *s == d.before)
                .map(|(o, _)| *o)
                .collect();
            // A unit freed in this drain (a missile that expired) is not in
            // `end`; its seed, set in place before the draws, is found by
            // the address it had at the previous drain.
            // Unit records move (a freed unit's slot is reused), so the
            // address counts only when no live unit sits there now.
            let gone = || {
                if end.iter().any(|e| e.addr == d.addr) {
                    return None;
                }
                self.prev
                    .iter()
                    .find(|k| k.addr == d.addr && end.iter().all(|e| e.owner != k.owner))
                    .map(|k| k.owner)
            };
            if let Some(o) = pick(cands, d.addr).or_else(gone) {
                r.owner = Some(o);
                if let Some(c) = cur.iter_mut().find(|(x, _)| *x == o) {
                    c.1 = d.after;
                }
                linked.push(o);
            }
        }
        let fwd = cur;
        // backward from the values now; an owner whose forward chain
        // reached its value now takes no more draws
        let mut cur: Vec<(Owner, Seed)> = end
            .iter()
            .filter(|k| {
                // a seed set in place (init_low before the roll) has no
                // forward link; only a chain that moved is finished
                !linked.contains(&k.owner) || fwd.iter().all(|(o, s)| *o != k.owner || *s != k.seed)
            })
            .map(|k| (k.owner, k.seed))
            .collect();
        for r in out.iter_mut().rev() {
            let d = r.draw;
            if d.before == d.after || is_drlg(&d) {
                continue;
            }
            match r.owner {
                Some(o) => {
                    if let Some(c) = cur.iter_mut().find(|(x, s)| *x == o && *s == d.after) {
                        c.1 = d.before;
                    }
                }
                None => {
                    let cands: Vec<Owner> = cur
                        .iter()
                        .filter(|(_, s)| *s == d.after)
                        .map(|(o, _)| *o)
                        .collect();
                    if let Some(o) = pick(cands, d.addr) {
                        r.owner = Some(o);
                        if let Some(c) = cur.iter_mut().find(|(x, _)| *x == o) {
                            c.1 = d.before;
                        }
                    }
                }
            }
        }
        // a no-step draw takes the owner its address names, if any
        for r in out.iter_mut() {
            if r.owner.is_none() && r.draw.before == r.draw.after {
                r.owner = end
                    .iter()
                    .find(|k| k.addr == r.draw.addr && k.seed == r.draw.after)
                    .map(|k| k.owner);
            }
        }
        self.prev = end;
        out
    }

    /// The `draw` line of `r` (no newline).
    pub fn line(&mut self, r: &Resolved) -> String {
        let d = r.draw;
        let seed = match r.owner {
            Some(o) => o.label(),
            None => {
                let i = match self.others.iter().position(|&a| a == d.addr) {
                    Some(i) => i,
                    None => {
                        self.others.push(d.addr);
                        self.others.len() - 1
                    }
                };
                format!("D2RUST#{i}")
            }
        };
        let owner = r.owner.map_or_else(|| "other".to_owned(), Owner::label);
        let mut s = String::with_capacity(200);
        let _ = write!(
            s,
            "{{\"type\":\"draw\",\"via\":\"helper\",\"op\":\"{}\"",
            d.op.name()
        );
        let (min, n) = d.op.args();
        if let Some(n) = n {
            let _ = write!(s, ",\"n\":{n}");
        }
        if let Some(m) = min {
            let _ = write!(s, ",\"min\":{m}");
        }
        let _ = write!(
            s,
            ",\"site\":\"{}:{}\",\"seed\":\"{seed}\",\"before\":[{},{}],\"after\":[{},{}],\
             \"ret\":{},\"frame\":{},\"owner\":\"{owner}\",\"tid\":0,\"seq\":{}}}",
            d.site.file().replace('\\', "/"),
            d.site.line(),
            d.before.lo,
            d.before.hi,
            d.after.lo,
            d.after.hi,
            d.ret,
            r.frame,
            self.seq
        );
        self.seq += 1;
        s
    }
}

/// The header line of a draw log (no newline).
pub fn header_line(tool: &str, date: &str, command: &str, seed: u32) -> String {
    format!(
        "{{\"type\":\"header\",\"format\":\"{FORMAT}\",\"side\":\"d2rust\",\"tool\":{},\
         \"date\":{},\"command\":{},\"seed\":{seed},\"frames\":true,\"owners\":true}}",
        json_str(tool),
        json_str(date),
        json_str(command)
    )
}

/// The footer line of a draw log (no newline): the event count and notes.
pub fn footer_line(events: u64, notes: &[String]) -> String {
    let notes: Vec<String> = notes.iter().map(|n| json_str(n)).collect();
    format!(
        "{{\"type\":\"footer\",\"events\":{events},\"notes\":[{}]}}",
        notes.join(",")
    )
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests;
