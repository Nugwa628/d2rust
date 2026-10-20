//! Provisional own-walk motion of the local player (first playable
//! preview).
//!
//! The server sends the walking player nothing while it walks,
//! and the client model moves a unit only when
//! a message places it. How the base game moves its
//! own player between messages is an open question, settled by a
//! capture of the own walk. Until then this module stands in:
//!
//! - every walk / run C→S message the client sends (0x01–0x04)
//!   sets the target ([`walk_of`], recorded by [`PredictLink`]);
//! - so does the server's walk request for the local player: an S→C
//!   0x0D with code 1 is the player mode
//!   request code 0x01, "walk to".
//!   The server sends it for the arrival walk-outs of a warp,
//!   a portal and a waypoint, each right
//!   before the 0x15 that places the player at the arrival point; the
//!   walk is held until the player's client room holds its target (so it
//!   starts from the arrival point, after the level change) and a walk
//!   the player sends replaces it ([`Predict::server_walk`]); after a
//!   waypoint travel the client sent (C→S 0x49) the walk is dropped, as
//!   the base game never walks a waypoint arrival out (measured);
//! - each server tick the predicted position steps along the player's own
//!   path over the client DRLG ([`ClientPath`]: the server's path code,
//!   so it stops at the same wall and goes
//!   round the same obstacle); without a client DRLG (or no active room
//!   at the predicted cell) it steps toward the target in a straight line
//!   at the charstats walk / run speed ([`Speeds`]);
//! - every change of the model's local position or server point (0x15
//!   placement, 0x0F / position-check correction) snaps it back to the
//!   model ([`Predict::observe`]);
//! - the facing (`dir64`, [`Predict::facing`]) is the direction
//!   from the predicted position toward the target
//!   (facing without the client's turn), set when a walk is
//!   taken and each step; it stays when the walk ends.
//!
//! The server stays the authority (step 7): nothing here is sent or
//! written to the model; the prediction only feeds the view and the
//! click's screen → world conversion ([`Predict::position`]).
//!
//! D2RUST's own, unverified. PROVISIONAL: the client path step
//! (`ClientPath`'s module doc), the
//! straight-line fallback, the
//! snap rule, the tick step and the facing (the client turns) are
//! not the base-game facts. PROVISIONAL (settled the waypoint
//! case): for a warp or portal arrival, that the 0x0D walk outlives
//! the 0x15 placement after it (the placement's teleport sets the path's
//! point count to 0; whether the client
//! then walks on to the request's target is an open question), and the hold until the
//! target is in the player's room. Measured on a waypoint arrival
//! the base-game client stands at
//! the 0x15 point with the server and never walks to the 0x0D's x + 3,
//! y + 3, where this module draws it (warps and portals not recorded
//! yet). Used only by the `play` preview; the
//! strict path never builds one.

use std::sync::OnceLock;

use d2_sim::path::tables::PathTables;
use d2_sim::path::walk::geom::direction_vector;
use d2_sim::path::walk::velocity::{STAT_FASTERMOVE, STAT_VELOCITYPERCENT, VELOCITY_PERCENT_FLOOR};

use super::client_path::{ClientPath, Own, PathTo};
use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use super::world::{ClientWorld, UnitKey, PLAYER};

/// The direction `dir64` (0–63) from precise (16.16) point `from` toward
/// `to` (`tan` table of `path-tables.tsv`, without
/// the path flag 0x200 flip); `None` when the points are equal
/// (no direction toward the point the unit is on) or the tables do not
/// parse.
pub fn facing(from: (u32, u32), to: (u32, u32)) -> Option<u8> {
    if from == to {
        return None;
    }
    let tables = path_tables()?;
    // D2RUST's own, unverified: takes the 32-bit `127 × l`, which
    // wraps past about 258 sub-tiles and then indexes the `tan` table out
    // of range; the base game only aims at near path points, but the model
    // can hold a position from before a level change. Far points are
    // brought near by halving both deltas (the direction keeps its ratio).
    let mut d = (
        i64::from(to.0) - i64::from(from.0),
        i64::from(to.1) - i64::from(from.1),
    );
    while d.0.abs().max(d.1.abs()) >= FACING_REACH {
        d = (d.0 / 2, d.1 / 2);
    }
    let near = (
        (i64::from(from.0) + d.0) as u32,
        (i64::from(from.1) + d.1) as u32,
    );
    if near == from {
        return None;
    }
    Some(direction_vector(tables, from, near).1)
}

/// The path tables (`path-tables.tsv`); `None` when they do not
/// parse.
pub(crate) fn path_tables() -> Option<&'static PathTables> {
    static TABLES: OnceLock<Option<PathTables>> = OnceLock::new();
    TABLES.get_or_init(|| PathTables::spec().ok()).as_ref()
}

/// The precise distance (16.16) below which `127 × l` stays in `i32`.
const FACING_REACH: i64 = 1 << 23;

/// The precise (16.16) centre of a sub-tile, as `u32`.
pub fn cell_centre((x, y): (u16, u16)) -> (u32, u32) {
    ((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000)
}

/// The charstats speeds of the local player's class (`WalkVelocity`
/// +0x40, `RunVelocity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Speeds {
    pub walk: u8,
    pub run: u8,
}

impl Speeds {
    /// The precise (16.16 sub-tile) distance of one base-rate tick:
    /// velocity = `WalkVelocity` · 256 · p / 100
    /// with p = 100 walking and 100 + (100 · `RunVelocity` / `WalkVelocity`
    /// − 100) running (the run list is skipped when `WalkVelocity`
    /// is 0); with base 0x400: step = velocity · 16 (case M1:
    /// velocity 0x600 moves 0x6000 a tick). Stats 67 / 96 at their
    /// creation values ([`Self::step_with`]).
    pub fn step(self, run: bool) -> i64 {
        self.step_with(run, MoveStats::CREATION)
    }

    /// [`Self::step`] with the player's stats 67 / 96 `s`:
    /// p = max(f + stat 67 + run bonus, 25),
    /// f = base · raw / (base + raw) of animstat row 4 over stat 96 (0
    /// without it).
    pub fn step_with(self, run: bool, s: MoveStats) -> i64 {
        let walk = i64::from(self.walk);
        let bonus = if run && walk != 0 {
            100 * i64::from(self.run) / walk - 100
        } else {
            0
        };
        let f = match path_tables() {
            Some(t) if s.faster != 0 => {
                let b = i64::from(t.animstat[4][1]);
                b * i64::from(s.faster) / (b + i64::from(s.faster))
            }
            _ => 0,
        };
        let p = (f + i64::from(s.percent) + bonus).max(i64::from(VELOCITY_PERCENT_FLOOR));
        (walk * 256 * p / 100) * 16
    }
}

/// The local player's velocity stats the prediction reads from the model:
/// stat 67 `velocitypercent`
/// total outside the run list (the path adds the run list itself, as
/// the server's does) and stat 96
/// `item_fastermovevelocity` total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveStats {
    pub percent: i32,
    pub faster: i32,
}

impl MoveStats {
    /// A player's creation values (stat
    /// 67 := 100; no items).
    pub const CREATION: MoveStats = MoveStats {
        percent: 100,
        faster: 0,
    };

    /// The local player's stats 67 and 96 (the list `total`). A model
    /// unit without its base stat 67 (no creation
    /// values: synthetic fixtures) reads the creation base 100 under its
    /// lists. No local player: [`Self::CREATION`].
    pub fn of_local(world: &ClientWorld) -> Self {
        let Some(p) = world.local() else {
            return Self::CREATION;
        };
        let base = if p.stats.contains_key(&STAT_VELOCITYPERCENT) {
            0
        } else {
            Self::CREATION.percent
        };
        MoveStats {
            percent: base + world.total(p.key, STAT_VELOCITYPERCENT, 0),
            faster: world.total(p.key, STAT_FASTERMOVE, 0),
        }
    }
}

/// Where a walk C→S message goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkTo {
    /// 0x01 / 0x03: a sub-tile point.
    Point(u16, u16),
    /// 0x02 / 0x04: a unit (its position each tick).
    Unit(UnitKey),
}

/// One walk or run intent the client sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    pub to: WalkTo,
    pub run: bool,
}

/// The walk intent of a C→S message (codes 1 / 3
/// walk / run to a point, {id, x u16, y u16}; codes 2 / 4 to a unit, {id,
/// type u32, guid u32}); any other message → `None`.
pub fn walk_of(msg: &[u8]) -> Option<Walk> {
    let (&id, rest) = msg.split_first()?;
    let run = matches!(id, 3 | 4);
    let to = match id {
        1 | 3 if rest.len() == 4 => WalkTo::Point(
            u16::from_le_bytes([rest[0], rest[1]]),
            u16::from_le_bytes([rest[2], rest[3]]),
        ),
        2 | 4 if rest.len() == 8 => {
            let t = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
            let guid = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
            WalkTo::Unit(UnitKey::new(u8::try_from(t).ok()?, guid))
        }
        _ => return None,
    };
    Some(Walk { to, run })
}

/// The centre of a sub-tile in precise (16.16) coordinates.
fn centre((x, y): (u16, u16)) -> (i64, i64) {
    ((i64::from(x) << 16) | 0x8000, (i64::from(y) << 16) | 0x8000)
}

/// Floor square root of `n`.
fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = 1u64 << (64 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// The local player's predicted position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Predict {
    /// The local player the prediction is for.
    player: Option<UnitKey>,
    /// The model's (position, rule-8 follow count) at the last
    /// observation.
    seen: Option<((u16, u16), u32)>,
    /// Precise (16.16) predicted position.
    at: Option<(i64, i64)>,
    /// The walk under way.
    walk: Option<Walk>,
    /// The predicted facing `dir64`; kept when the walk ends.
    dir: Option<u8>,
    /// The local player's stamina (stat 10) is zero: the server walks
    /// instead of running, so the
    /// prediction does too. The base-game client ends its run on model stat
    /// 10 = 0, i.e. server raw stamina < 256;
    /// the server runs while raw
    /// is 1..255, a gap the position check corrects.
    exhausted: bool,
    /// The local player's level at the last observation.
    level: Option<u16>,
    /// The client's act at the last level change.
    act: Option<u8>,
    /// The local player's mode request count (`ClientUnit::mode_requests`)
    /// at the last look.
    requests: u32,
    /// A server walk (S→C 0x0D code 1) whose target is not yet in the
    /// player's client room ([`Self::server_walk`]).
    held: Option<(u16, u16)>,
    /// The client sent a waypoint travel (C→S 0x49) whose arrival walk
    /// request has not come yet ([`Self::waypoint_sent`]).
    waypoint: bool,
    /// The local player's mode request count of a waypoint arrival's
    /// walk-out that was not walked ([`Self::stood_mode`]).
    stood: Option<u32>,
    /// The local player's `ClientUnit::faces` at the last look.
    faces: u32,
    /// A cast or attack turn is under way: the facing follows the model's
    /// stepped `path_dir` until it reaches the new direction.
    turning: bool,
    /// The mode request count of the server walk (S→C 0x0D code 1, a
    /// portal arrival's walk-out) the prediction is walking: when it ends
    /// the player stands ([`Self::stood_mode`]); a click drops it.
    served: Option<u32>,
    /// The server tick the walk under way started on (kept while a new
    /// click re-targets it; cleared when it ends): the walk animation's
    /// start ([`Self::walk_since`]).
    since: Option<(u64, u32)>,
    /// The model's living monsters, stamped on the client path's grids
    /// ([`Self::set_others`]).
    others: Vec<super::client_path::OtherUnit>,
    /// The model's objects with collision in their mode, stamped with
    /// them ([`Self::set_others`]).
    objects: Vec<super::client_path::OtherObject>,
    /// The player's own path over the client DRLG ([`ClientPath`]): the
    /// step of a walk when the client has a DRLG.
    path: ClientPath,
    /// The walk and the predicted position the path was last stepped
    /// for: a new walk or a snap ([`Self::observe`]) re-places it.
    path_for: Option<(Walk, (i64, i64))>,
    /// The model's local player was in its walk mode during the walk to a
    /// unit under way ([`Self::unit_walk_end`]).
    unit_walked: bool,
    /// The model's local player mode at the last frame: a step from a
    /// walking to a neutral mode is the server copy's stop ([`Self::frame`]).
    model_mode: Option<u32>,
}

/// C→S 0x49, waypoint travel.
const C2S_WAYPOINT: u8 = 0x49;

/// The player mode request code "walk to" (,
/// code 0x01).
const CODE_WALK_TO_POINT: u8 = 0x01;

impl Predict {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snaps to the model: a new local player starts at its
    /// position; a change of the model's position (0x15, a check
    /// correction) or a check that took the server's point
    /// ([`ClientUnit::follows`]) moves the
    /// prediction to it, server point first. The walk target is kept, unless the player's
    /// level changed (a warp, portal or act change: the target is a point
    /// of the old level). A level change the prediction did not walk into
    /// is a placement (0x15 / 0x59): the prediction moves to the model's
    /// position even when that is the cell the model held before (an act
    /// change that lands on the old act's arrival cell). No local player,
    /// or one with no position: no prediction.
    pub fn observe(&mut self, world: &ClientWorld) {
        let Some(p) = world.local().filter(|p| p.key.unit_type == PLAYER) else {
            *self = Self::default();
            return;
        };
        let Some(pos) = p.position else {
            *self = Self::default();
            return;
        };
        let now = (pos, p.follows);
        let level = world.player_level();
        let act = world.act.as_ref().map(|a| a.act);
        if level != self.level {
            // The prediction's own walk across a level border (the room
            // recache of `world_view::walk_room`) is no placement: same
            // act, the predicted cell in the player's room.
            let walked_in = act == self.act
                && self.cell().is_some_and(|(x, y)| {
                    world
                        .local_room()
                        .is_some_and(|r| r.contains(i32::from(x), i32::from(y)))
                });
            self.level = level;
            self.act = act;
            // B002: a walk that carried the player over the border goes
            // on (the server's does); only a placement ends it.
            if !(walked_in && self.player == Some(p.key)) {
                self.walk = None;
            }
            if self.player == Some(p.key) && !walked_in {
                self.at = Some(centre(pos));
                self.seen = Some(now);
                return;
            }
        }
        if self.player != Some(p.key) {
            *self = Self {
                player: Some(p.key),
                seen: Some(now),
                at: Some(centre(pos)),
                walk: None,
                dir: None,
                exhausted: false,
                level: world.player_level(),
                act: world.act.as_ref().map(|a| a.act),
                requests: p.mode_requests,
                held: None,
                waypoint: self.waypoint,
                stood: None,
                served: None,
                faces: p.faces,
                turning: false,
                since: None,
                others: std::mem::take(&mut self.others),
                objects: std::mem::take(&mut self.objects),
                path: ClientPath::new(world.tick_rate),
                path_for: None,
                unit_walked: false,
                model_mode: None,
            };
            return;
        }
        let Some((old_pos, old_follows)) = self.seen else {
            self.seen = Some(now);
            self.at = Some(centre(pos));
            return;
        };
        // Only a check that took the server's point (the
        // local player's `Checked::Followed`) moves the prediction: a
        // kept check (step 7) leaves the client's own position as it is,
        // so a waypoint arrival's walk-out stays drawn while its server
        // player stands at the arrival point within the tolerance.
        if p.follows != old_follows && p.server_point.0 != 0 && p.server_point.1 != 0 {
            self.at = Some(centre(p.server_point));
        } else if pos != old_pos && self.cell() != Some(pos) {
            self.at = Some(centre(pos));
        }
        self.seen = Some(now);
    }

    /// The server's walk request for the local player (module doc): a
    /// new mode request with code 0x01 (S→C 0x0D code 1) is held; any
    /// other new request drops a held one. A held walk starts once the
    /// player's client room holds its target: after the 0x15 of an
    /// arrival, from the arrival point (a target in another level is
    /// never stepped toward from the old position).
    pub fn server_walk(&mut self, world: &ClientWorld) {
        let Some(p) = world.local().filter(|p| Some(p.key) == self.player) else {
            return;
        };
        if p.mode_requests != self.requests {
            // The player stands in an arrival's unwalked walk-out until a
            // request changes its mode ([`Self::stood_mode`]).
            let was_stood = self.stood == Some(self.requests);
            self.requests = p.mode_requests;
            // A request that sets a mode other than walk / run ends the
            // walk under way: get-hit (code 6, mode 4), death (8, 9),
            // a skill (0x12–0x16), knockback (0x14), …;
            // the client steps the path only in a walking mode,
            // as the server stops its player's walk on the same mode
            // change. The walk codes (0x00, 0x01, 0x17, 0x18) and the
            // interact sender (0x02, no mode) keep it.
            // The soft hit 0x13, and a 0x12 that sets no mode, leave the
            // mode: the walking unit keeps stepping (measured: the walk
            // `cur` runs on through the hit).
            // Several requests can land between two looks: only the last
            // code is kept, so a soft hit after a cast of the same pass
            // keeps the walk only while the mode is still a walking one.
            let keeps = |code: u8| {
                matches!(code, 0x00 | 0x01 | 0x02 | 0x17 | 0x18)
                    || (matches!(code, 0x12 | 0x13) && matches!(p.mode, 2 | 3 | 6))
            };
            if p.last_mode_request.is_some_and(|r| !keeps(r.code)) {
                self.walk = None;
                self.path_for = None;
            }
            // The requests that set no mode (the interact sender 0x02, the
            // unchanged 0x12, the soft hit 0x13)
            // leave the standing pose: the model mode is still the
            // arrival's walk, which the view must keep drawing neutral
            // (measured: the unit stays in `nuhth` under the soft hit of
            // a Fallen Shaman's fire bolt).
            if was_stood
                && p.last_mode_request
                    .is_some_and(|r| matches!(r.code, 0x02 | 0x12 | 0x13))
            {
                self.stood = Some(p.mode_requests);
            }
            // A waypoint arrival: the base game takes
            // the 0x15 point and never walks to the 0x0D's x + 3, y + 3
            // (the teleport zeroes the path;
            // measured).
            if std::mem::take(&mut self.waypoint) {
                self.held = None;
                self.stood = Some(p.mode_requests);
                return;
            }
            self.held = p
                .last_mode_request
                .filter(|r| r.code == CODE_WALK_TO_POINT)
                .and_then(|r| {
                    Some((
                        u16::try_from(r.record[0]).ok()?,
                        u16::try_from(r.record[1]).ok()?,
                    ))
                });
        }
        let Some((x, y)) = self.held else {
            return;
        };
        if self.at.is_none()
            || !world
                .local_room()
                .is_some_and(|room| room.contains(i32::from(x), i32::from(y)))
        {
            return;
        }
        self.held = None;
        self.served = Some(p.mode_requests);
        self.walk = Some(Walk {
            to: WalkTo::Point(x, y),
            run: false,
        });
    }

    /// The client sent a waypoint travel (C→S 0x49): the next server walk
    /// request is that arrival's walk-out, which is not walked.
    pub fn waypoint_sent(&mut self) {
        self.waypoint = true;
    }

    /// A walk intent the client sent: the new target (a held server walk
    /// is dropped).
    pub fn walk(&mut self, walk: Walk) {
        self.held = None;
        self.served = None;
        self.exhausted = false;
        if self.at.is_some() {
            if self.walk.map(|w| w.to) != Some(walk.to) {
                self.unit_walked = false;
            }
            self.walk = Some(walk);
        }
    }

    /// Stops the walk (the prediction stays where it is).
    pub fn stop(&mut self) {
        self.walk = None;
    }

    /// One server tick: a straight step of [`Speeds::step`] toward the
    /// target's sub-tile centre; reaching it ends the walk. A unit target
    /// not in the model ends the walk.
    pub fn tick(&mut self, world: &ClientWorld, speeds: Speeds) {
        let (Some((x, y)), Some(walk)) = (self.at, self.walk) else {
            return;
        };
        let target = match walk.to {
            WalkTo::Point(tx, ty) => Some((tx, ty)),
            WalkTo::Unit(k) => world.units.get(&k).and_then(|u| u.position),
        };
        let Some(target) = target else {
            self.walk = None;
            return;
        };
        let (tx, ty) = centre(target);
        if let Some(d) = facing((x as u32, y as u32), (tx as u32, ty as u32)) {
            self.dir = Some(d);
        }
        let (dx, dy) = (tx - x, ty - y);
        self.exhausted = world
            .local()
            .is_some_and(|p| p.stats.get(&10).is_some_and(|s| *s == 0));
        let moves = MoveStats::of_local(world);
        if self.path_step(world, speeds, moves, walk, target) {
            return;
        }
        // This tick's share of the base-rate step (the client steps once
        // per server tick).
        let step = super::motion::tick_share(
            world.tick_rate,
            speeds.step_with(walk.run && !self.exhausted, moves),
            world.share_frame().rem_euclid(world.tick_rate.hz() as i32) as u64,
        );
        let dist = isqrt(dx.unsigned_abs().pow(2) + dy.unsigned_abs().pow(2)) as i64;
        if dist <= step || step <= 0 {
            if step > 0 {
                self.at = Some((tx, ty));
            }
            self.walk = None;
            return;
        }
        self.at = Some((x + dx * step / dist, y + dy * step / dist));
    }

    /// The tick on the player's own path (module doc): with a client DRLG
    /// whose active rooms hold the predicted cell, the path is placed
    /// there and requested for a new walk (or after a snap), then stepped
    /// one tick; its position is the prediction
    /// and its stop ends the walk. `false`: no client path (no DRLG, no
    /// room, no tables), so the straight step runs.
    fn path_step(
        &mut self,
        world: &ClientWorld,
        speeds: Speeds,
        moves: MoveStats,
        walk: Walk,
        target: (u16, u16),
    ) -> bool {
        let (Some(at), Some(cell), Some(t)) = (self.at, self.cell(), path_tables()) else {
            return false;
        };
        let Some(drlg) = world.drlg.as_ref().map(|d| &d.drlg) else {
            return false;
        };
        let to = match walk.to {
            WalkTo::Point(x, y) => PathTo::Point(x, y),
            WalkTo::Unit(k) => {
                let Some(&ty) = d2_sim::units::UnitType::ALL.get(usize::from(k.unit_type)) else {
                    return false;
                };
                PathTo::Unit(ty, k.guid, target)
            }
        };
        // The model's stamina; only zero is read (a run starts as a walk).
        let stamina = world
            .local()
            .and_then(|p| p.stats.get(&10).copied())
            .unwrap_or(1);
        let own = Own { stamina, moves };
        // A walk to a unit re-requests every tick: the pending action
        // runs each client tick while out of
        // reach, and its mode request recomputes the path from the
        // current position.
        let rerun = matches!(walk.to, WalkTo::Unit(_));
        if rerun || self.path_for != Some((walk, at)) {
            // A re-target while the path moves (no snap since its last
            // step) recomputes from the current precise position, the
            // fraction kept (edge case 8;
            // measured: re-placing at the cell centre drifted 3 px).
            // A walk from rest or after a snap places the path anew.
            let moving_here = self.path_for.is_some_and(|(_, a)| a == at);
            if !moving_here && !self.path.place(t, drlg, cell.0, cell.1) {
                self.path_for = None;
                return false;
            }
            self.path.stamp_others(t, drlg, &self.others, &self.objects);
            if !self.path.request(t, drlg, speeds, own, to, walk.run) {
                // No path: the server's request stands still too.
                self.walk = None;
                self.path_for = None;
                return true;
            }
        }
        self.path.stamp_others(t, drlg, &self.others, &self.objects);
        if !world.tick_rate.is_base() {
            self.path.set_frame(world.share_frame());
        }
        let moving = self.path.tick(t, drlg, speeds, own, Some(to));
        if let Some((x, y)) = self.path.position() {
            let now = (i64::from(x), i64::from(y));
            if let Some(d) = facing((at.0 as u32, at.1 as u32), (x, y)) {
                self.dir = Some(d);
            }
            self.at = Some(now);
        }
        if moving {
            self.path_for = self.at.map(|a| (walk, a));
        } else {
            self.walk = None;
            self.path_for = None;
        }
        true
    }

    /// The end of a walk to a unit: the
    /// model's player walks while its path runs, and the mode end
    /// sets neutral (town 5, else 1) when the path ends at the
    /// unit and the pending action has run its action. The base game
    /// draws that neutral mode from there on: an Act I waypoint click
    /// shows TN 15 ticks
    /// later. Once the model's player was in its walk mode during this
    /// walk and is back in neutral without a new request, the predicted
    /// walk ends too (it would otherwise keep stepping toward the
    /// object's occupied cell and draw the walk until the warp).
    fn unit_walk_end(&mut self, world: &ClientWorld) {
        let Some(Walk {
            to: WalkTo::Unit(_),
            ..
        }) = self.walk
        else {
            self.unit_walked = false;
            return;
        };
        let Some(p) = world.local().filter(|p| Some(p.key) == self.player) else {
            return;
        };
        let (neutral, walk) = super::modes::neutral_walk(world, p.key);
        if matches!(p.mode, 2 | 3) || p.mode == walk {
            self.unit_walked = true;
        } else if self.unit_walked && p.mode == neutral {
            self.walk = None;
            self.path_for = None;
            self.unit_walked = false;
        }
    }

    /// Faces the walk's target from the predicted position (module doc);
    /// no walk, no target or already on it: the facing stays.
    fn face(&mut self, world: &ClientWorld) {
        let (Some((x, y)), Some(walk)) = (self.at, self.walk) else {
            return;
        };
        let target = match walk.to {
            WalkTo::Point(tx, ty) => Some((tx, ty)),
            WalkTo::Unit(k) => world.units.get(&k).and_then(|u| u.position),
        };
        if let Some(d) = target.and_then(|t| facing((x as u32, y as u32), cell_centre(t))) {
            self.dir = Some(d);
        }
    }

    /// One bridge frame: [`Self::observe`], [`Self::server_walk`], the
    /// walks sent since the last frame (last one wins), the facing toward
    /// the target, then [`Self::tick`] if the server ticked.
    pub fn frame(
        &mut self,
        world: &ClientWorld,
        walks: impl IntoIterator<Item = Walk>,
        ticked: bool,
        speeds: Speeds,
    ) {
        self.observe(world);
        self.server_walk(world);
        // B025: a cast or attack at a target turns the player toward it
        // (`critter_path::face_point`, from the model's local mode
        // request); the facing kept after a walk gives way to it.
        if let Some(p) = world.local().filter(|p| Some(p.key) == self.player) {
            if p.faces != self.faces {
                self.faces = p.faces;
                self.turning = p.path_turn.is_some();
            }
            // The drawn direction is the stepped one, not the target
            // (measured: dir 0, 2, 4, 6, 7 over the frames).
            if self.turning {
                if let Some((new, _)) = p.path_turn {
                    let cur = p.path_dir.unwrap_or(new);
                    self.dir = Some(cur);
                    self.turning = cur != new;
                }
            }
        }
        let mut new_walk = false;
        for w in walks {
            self.walk(w);
            new_walk = true;
            self.turning = false;
        }
        // A walk to a point faces its target from where it started and
        // keeps that facing (measured: dir 32, the start subtile
        // (4873, 4228) to the clicked (4867, 4222); PROVISIONAL:
        // settled by a capture of the player's `d` per frame);
        // a walk to a unit follows the unit.
        if new_walk
            || matches!(
                self.walk,
                Some(Walk {
                    to: WalkTo::Unit(_),
                    ..
                })
            )
        {
            self.face(world);
        }
        if ticked {
            self.tick(world, speeds);
        }
        self.unit_walk_end(world);
        // The single-player server copy sets
        // the client player neutral (1, town 5) when the server's stands
        // and the client's walks (2, 3, 6); the client path then steps no
        // more (it runs only in a walking mode), so the walk ends here
        // even where the client path itself still reads as moving (a walk
        // to a unit whose last point is blocked by it: measured TN when
        // the player reaches Kashya).
        let model_mode = world
            .local()
            .filter(|p| Some(p.key) == self.player)
            .map(|p| p.mode);
        if !new_walk
            && matches!(self.model_mode, Some(2 | 3 | 6))
            && matches!(model_mode, Some(1 | 5))
        {
            self.walk = None;
            self.path_for = None;
        }
        self.model_mode = model_mode;
        // B012: a server walk the prediction finished (a portal arrival's
        // walk-out): the model's walk mode never ends by itself, so the
        // player stands from here on, as after a waypoint arrival.
        if self.walk.is_none() {
            if let Some(r) = self.served.take() {
                self.stood = Some(r);
            }
        }
        // In single player the client unit takes
        // the server unit's precise position at the end of every client
        // update, so the local player stands where the
        // server's does; the copy wins over the predicted step (measured:
        // the drawn client point of every tick is the server point of
        // that tick).
        if let Some((x, y)) = world.local().and_then(|p| p.precise) {
            self.at = Some((i64::from(x), i64::from(y)));
        }
        self.follow_path_facing(world);
        // The animation restarts with the mode (walk ↔ run: the run
        // frames count from the first run click).
        // Keyed on walk vs run (the path's 2 → 6 town-walk change the
        // tick after the request is the same walk). The walk was sent in
        // the previous pass and starts on the server tick after it was
        // sent; the drawn position is the server's (above), so the
        // walk frame counts from the tick it was sent on: the base game
        // draws walk frame 0 on the walk's first tick
        // (40), 1 on the next. A pass that pumped one tick sees a walk
        // sent on the tick before; a held pass (no pump, the undrawn tick
        // of the Wine schedule) sees one sent on this tick.
        let kind = self.mode().map(|m| u32::from(m == 3));
        self.since = match (kind, self.since) {
            (None, _) => None,
            (Some(m), Some((t, was))) if was == m => Some((t, m)),
            (Some(m), _) => Some((world.server_ticks.saturating_sub(u64::from(ticked)), m)),
        };
    }

    /// The model's living monsters and colliding
    /// objects whose footprints the client path sees, set before each
    /// [`Self::frame`].
    pub fn set_others(
        &mut self,
        others: Vec<super::client_path::OtherUnit>,
        objects: Vec<super::client_path::OtherObject>,
    ) {
        self.others = others;
        self.objects = objects;
    }

    /// The server tick the walk under way started on (the walk frame
    /// counts from there).
    pub fn walk_since(&self) -> Option<u64> {
        self.since.map(|(t, _)| t)
    }

    /// The predicted precise position (16.16 sub-tiles, the form
    /// `rules::camera::moving_to_client` takes): the hook of the world
    /// view's camera and of the click conversion.
    pub fn position(&self) -> Option<(u32, u32)> {
        self.at.map(|(x, y)| (x as u32, y as u32))
    }

    /// The predicted sub-tile.
    pub fn cell(&self) -> Option<(u16, u16)> {
        self.at.map(|(x, y)| ((x >> 16) as u16, (y >> 16) as u16))
    }

    /// The local player the prediction is for.
    pub fn player(&self) -> Option<UnitKey> {
        self.player
    }

    /// The facing of the model's player once the player update turns its
    /// path facing (+0x64; the copy leaves it
    /// alone): that is what the base game draws (a recorded walk: 0 →
    /// 60, 56, 52, 49 over ticks 40-43). Run at the end of
    /// [`Self::frame`], and again after a late aim
    /// (`Bridge::aim_local`).
    pub fn follow_path_facing(&mut self, world: &ClientWorld) {
        if let Some(d) = world
            .local()
            .filter(|p| p.path_turn.is_some())
            .and_then(|p| p.path_dir)
        {
            self.dir = Some(d);
        }
    }

    /// The walk path's new direction (+0x65) while a walk is under way.
    pub fn new_direction(&self) -> Option<u8> {
        self.walk.and(self.path_for).and(self.path.new_direction())
    }

    /// The walk path's new direction (+0x65) while a walk to a unit is
    /// under way: that walk re-plans the client path every tick before
    /// the update, so the update's aim reads
    /// it; a walk to a point keeps the copied server points.
    pub fn unit_new_direction(&self) -> Option<u8> {
        match self.walk?.to {
            WalkTo::Unit(_) => self.new_direction(),
            WalkTo::Point(..) => None,
        }
    }

    /// The walk under way, if any.
    pub fn walking(&self) -> Option<Walk> {
        self.walk
    }

    /// The predicted facing `dir64` (0–63, module doc); `None` before the
    /// first walk. Stays after the walk ends (a standing unit keeps its
    /// direction).
    pub fn facing(&self) -> Option<u8> {
        self.dir
    }

    /// The mode the view shows for a waypoint arrival's unwalked walk-out:
    /// the model's walk mode (S→C 0x0D code 1) never ends, as the client
    /// path has no points after the teleport, so the player stands
    /// (neutral 1, town 5). The base game draws the arrival in the
    /// neutral pose (`nuhth`, Act 4 / 5 outdoors). Measured: the
    /// base-game model mode
    /// itself is neutral (5) after both a waypoint arrival and a portal
    /// arrival's walk-out.
    pub fn stood_mode(&self, world: &ClientWorld) -> Option<u32> {
        let p = world.local().filter(|p| Some(p.key) == self.player)?;
        if self.walk.is_some() || self.stood != Some(p.mode_requests) {
            return None;
        }
        let (neutral, walk) = super::modes::neutral_walk(world, p.key);
        (p.mode == walk).then_some(neutral)
    }

    /// The player mode the view shows while the prediction moves: 2
    /// (walk) or 3 (run); `None`: the model's mode.
    pub fn mode(&self) -> Option<u32> {
        // A walk in a town room is the town walk 6, as the client path's
        // mode request sets it.
        let town = self.path.mode() == 6;
        self.walk.map(|w| match (w.run && !self.exhausted, town) {
            (true, _) => 3,
            (false, true) => 6,
            (false, false) => 2,
        })
    }
}

/// The walks a [`PredictLink`] recorded, shared with whoever runs
/// [`Predict::frame`] (the play app boxes the link, so it cannot be
/// reached through the bridge's `link_mut`).
#[derive(Clone, Debug, Default)]
pub struct WalkTap(
    std::sync::Arc<std::sync::Mutex<Vec<Walk>>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);

impl WalkTap {
    /// Whether a walk was sent since the last [`Self::take`].
    pub fn pending(&self) -> bool {
        !self.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// The walks sent since the last call, in send order.
    pub fn take(&self) -> Vec<Walk> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn push(&self, w: Walk) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(w);
    }

    /// Whether a waypoint travel (C→S 0x49) was sent since the last call.
    pub fn take_waypoint(&self) -> bool {
        self.1.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// Records `msg` when it is a walk intent ([`walk_of`]) or a waypoint
    /// travel (C→S 0x49), as [`PredictLink`] does for what it sends: for a
    /// sender that reaches the server another way (a test rig's direct
    /// link).
    pub fn record(&self, msg: &[u8]) {
        if let Some(w) = walk_of(msg) {
            self.push(w);
        }
        if msg.first() == Some(&C2S_WAYPOINT) {
            self.1.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// A [`ServerLink`] that records the walk intents sent through it for
/// [`Predict::frame`]; everything else passes through unchanged.
pub struct PredictLink<L> {
    inner: L,
    walks: WalkTap,
}

impl<L> PredictLink<L> {
    pub fn new(inner: L) -> Self {
        Self {
            inner,
            walks: WalkTap::default(),
        }
    }

    /// The walks sent since the last call, in send order.
    pub fn take_walks(&mut self) -> Vec<Walk> {
        self.walks.take()
    }

    /// A handle on the recorded walks ([`WalkTap::take`] drains the same
    /// list as [`Self::take_walks`]).
    pub fn tap(&self) -> WalkTap {
        self.walks.clone()
    }

    pub fn inner(&self) -> &L {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut L {
        &mut self.inner
    }
}

impl<L: ServerLink> ServerLink for PredictLink<L> {
    fn protocol_version(&self) -> u32 {
        self.inner.protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }

    fn server_frame(&mut self) -> Option<i32> {
        self.inner.server_frame()
    }

    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        let sent = self.inner.send(queue, msg)?;
        self.walks.record(msg);
        Ok(sent)
    }

    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.inner.pump()
    }

    fn set_display_catch_up(&mut self, enabled: bool) {
        self.inner.set_display_catch_up(enabled)
    }

    fn tick_pending(&mut self) -> bool {
        self.inner.tick_pending()
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.inner.receive()
    }

    fn server_paths(&mut self) -> Vec<super::critter_path::ServerPath> {
        self.inner.server_paths()
    }

    fn step_paths(&mut self) -> Vec<super::critter_path::ServerPath> {
        self.inner.step_paths()
    }

    fn take_resume(&mut self) -> bool {
        self.inner.take_resume()
    }

    fn set_send_now(&mut self, now: Option<u32>) {
        self.inner.set_send_now(now)
    }
}

/// Pokes pass through (`state-dump` runs its bridge on a [`PredictLink`]).
impl<L: super::poke::PokeTarget> super::poke::PokeTarget for PredictLink<L> {
    type Error = L::Error;
    fn poke(&mut self, op: &d2_sim::poke::PokeOp) -> Result<d2_sim::poke::PokeResult, L::Error> {
        self.inner.poke(op)
    }
    fn goto_step(
        &mut self,
        target: d2_sim::poke::GotoTarget,
        walk: d2_sim::poke::GotoWalk,
    ) -> Result<(d2_sim::poke::PokeResult, d2_sim::poke::GotoWalk), L::Error> {
        self.inner.goto_step(target, walk)
    }
}

/// Scripted messages pass through (`state-dump --send`).
impl<L: super::inject::InjectTarget> super::inject::InjectTarget for PredictLink<L> {
    type Error = L::Error;
    fn inject(
        &mut self,
        msg: &crate::scenario_script::StepMsg,
    ) -> Result<super::inject::Injected, L::Error> {
        self.inner.inject(msg)
    }
}

/// Snapshots pass through (`state-dump`).
impl<L: super::state::StateSource> super::state::StateSource for PredictLink<L> {
    type Error = L::Error;
    fn state_snapshot(&mut self) -> Result<d2_sim::debug::state::StateSnapshot, L::Error> {
        self.inner.state_snapshot()
    }

    fn player_new_directions(&mut self) -> Result<Vec<(u32, u8)>, L::Error> {
        self.inner.player_new_directions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::ClientUnit;

    /// A walk to a point faces the clicked subtile from the walk's start
    /// (`Predict::frame` aims it only when the walk arrives).
    #[test]
    fn a_point_walk_faces_the_click_from_its_start() {
        let mut p = Predict {
            at: Some(((4873 << 16) + 0x8000, (4228 << 16) + 0x8000)),
            ..Predict::default()
        };
        let w = ClientWorld {
            server_ticks: 22,
            ..ClientWorld::default()
        };
        let walk = Walk {
            to: WalkTo::Point(4867, 4222),
            run: false,
        };
        p.walk = Some(walk);
        p.face(&w);
        // A recorded walk north: the base game draws dir64 32 (start (4873, 4228) to the
        // clicked (4867, 4222)).
        assert_eq!(p.facing(), Some(32));
    }

    /// The drawn mode of a predicted walk: the town walk 6 when the client
    /// path's mode request made it one (a recorded walk: the base game draws
    /// `soshlittwhth`, D2RUST drew mode 2 `wl`); a run stays 3 in town
    /// (finding 3).
    #[test]
    fn a_walk_in_town_is_drawn_in_the_town_walk_mode() {
        let to = WalkTo::Point(1, 2);
        let mode = |run, path| {
            Predict {
                walk: Some(Walk { to, run }),
                path: ClientPath::with_mode(path),
                ..Predict::default()
            }
            .mode()
        };
        assert_eq!(mode(false, 6), Some(6));
        assert_eq!(mode(false, 2), Some(2));
        assert_eq!(mode(true, 3), Some(3));
        assert_eq!(mode(true, 6), Some(3));
        assert_eq!(Predict::default().mode(), None);
    }

    /// Only a walk to a unit hands its client plan to the update's aim; a
    /// point walk (and no walk) keeps the server copy's.
    #[test]
    fn a_point_walk_keeps_the_server_aim() {
        let walk = |to| Predict {
            walk: Some(Walk { to, run: false }),
            ..Predict::default()
        };
        assert_eq!(walk(WalkTo::Point(1, 2)).unit_new_direction(), None);
        assert_eq!(Predict::default().unit_new_direction(), None);
    }

    // Synthetic fixture: charstats-shaped speeds (walk 6, run 9).
    const SPEEDS: Speeds = Speeds { walk: 6, run: 9 };

    fn world_at(x: u16, y: u16) -> (ClientWorld, UnitKey) {
        let key = UnitKey::new(PLAYER, 7);
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(key);
        u.position = Some((x, y));
        u.server_point = (x, y);
        w.units.insert(key, u);
        w.local_player = Some(key);
        (w, key)
    }

    #[test]
    fn step_matches_the_velocity_rules() {
        assert_eq!(SPEEDS.step(false), 0x6000);
        // Run: p = 100 + (100·9/6 − 100) = 150 → velocity 0x900.
        assert_eq!(SPEEDS.step(true), 0x9000);
        assert_eq!(Speeds { walk: 0, run: 9 }.step(true), 0);
    }

    #[test]
    fn walk_of_reads_the_four_walk_codes() {
        assert_eq!(
            walk_of(&[1, 0x34, 0x12, 0x78, 0x56]),
            Some(Walk {
                to: WalkTo::Point(0x1234, 0x5678),
                run: false
            })
        );
        assert_eq!(walk_of(&[3, 1, 0, 2, 0]).map(|w| w.run), Some(true));
        assert_eq!(
            walk_of(&[4, 1, 0, 0, 0, 9, 0, 0, 0]),
            Some(Walk {
                to: WalkTo::Unit(UnitKey::new(1, 9)),
                run: true
            })
        );
        assert_eq!(walk_of(&[2, 0, 1, 0, 0, 9, 0, 0, 0]), None);
        assert_eq!(walk_of(&[5, 1, 0, 2, 0]), None);
        assert_eq!(walk_of(&[1, 1, 0]), None);
        assert_eq!(walk_of(&[]), None);
    }

    #[test]
    fn isqrt_floors() {
        for n in [0u64, 1, 2, 3, 4, 15, 16, 17, 1 << 40, u64::from(u32::MAX)] {
            let r = isqrt(n);
            assert!(r * r <= n && (r + 1) * (r + 1) > n, "{n}");
        }
    }

    #[test]
    fn walks_straight_at_the_walk_speed_and_stops_on_the_target() {
        let (w, _) = world_at(100, 100);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(105, 100, false)], false, SPEEDS);
        assert_eq!(p.position(), Some((0x64_8000, 0x64_8000)));
        for k in 1..=13 {
            p.frame(&w, [], true, SPEEDS);
            assert_eq!(p.position(), Some((0x64_8000 + k * 0x6000, 0x64_8000)));
        }
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.position(), Some((0x69_8000, 0x64_8000)));
        assert_eq!(p.cell(), Some((105, 100)));
        assert_eq!(p.walking(), None);
        assert_eq!(p.mode(), None);
    }

    #[test]
    fn frames_without_a_tick_do_not_move() {
        let (w, _) = world_at(100, 100);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(110, 100, true)], false, SPEEDS);
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.cell(), Some((100, 100)));
        assert_eq!(p.mode(), Some(3));
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.position(), Some((0x64_8000 + 0x9000, 0x64_8000)));
    }

    #[test]
    fn diagonal_steps_keep_the_speed() {
        let (w, _) = world_at(100, 100);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(120, 120, false)], true, SPEEDS);
        let (x, y) = p.position().unwrap();
        let (dx, dy) = (i64::from(x) - 0x64_8000, i64::from(y) - 0x64_8000);
        assert_eq!(dx, dy);
        // 0x6000 · d / ⌊√(2·d²)⌋ with d = 20 sub-tiles, truncated: 0x6000 / √2.
        let d = 20i64 << 16;
        let dist = isqrt(2 * (d as u64).pow(2)) as i64;
        assert_eq!(dx, 0x6000 * d / dist);
        assert_eq!(dx, 17377);
    }

    #[test]
    fn model_changes_snap_the_prediction() {
        let (mut w, key) = world_at(100, 100);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(120, 100, false)], true, SPEEDS);
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.position(), Some((0x65_4000, 0x64_8000)));
        // A kept check (step 7) stores its point but does not move the
        // prediction.
        w.units.get_mut(&key).unwrap().server_point = (104, 101);
        let before = p.position();
        p.frame(&w, [], false, SPEEDS);
        assert_ne!(p.position(), Some((0x68_8000, 0x65_8000)));
        assert!(before.is_some());
        // A check that took the server's point (step 8) snaps; the walk
        // goes on from there.
        w.units.get_mut(&key).unwrap().server_point = (104, 101);
        w.units.get_mut(&key).unwrap().follows += 1;
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.position(), Some((0x68_8000, 0x65_8000)));
        assert!(p.walking().is_some());
        // A placement (0x15) snaps to the new position.
        w.units.get_mut(&key).unwrap().position = Some((50, 60));
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.position(), Some((0x32_8000, 0x3C_8000)));
        // Nothing changed: no snap.
        p.frame(&w, [], true, SPEEDS);
        assert_ne!(p.position(), Some((0x32_8000, 0x3C_8000)));
    }

    /// A walk under way when the player changes level (a warp or portal
    /// placement) ends: its target is a point of the old level, which may
    /// lie hundreds of sub-tiles away in the new level's coordinates.
    #[test]
    fn a_level_change_ends_the_walk() {
        use super::super::world::ActiveRoom;
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = world_at(20, 20);
        let room = |level: u16, id: u32, x0: i32| ActiveRoom {
            x0,
            y0: 0,
            w: 40,
            h: 40,
            level,
            room: DrlgRoomId(id),
        };
        w.active_rooms = Some(vec![room(3, 1, 0), room(1, 2, 1000)]);
        w.room_units.place(key, Some(DrlgRoomId(1)));
        let mut p = Predict::new();
        p.frame(&w, [walk_point(30, 20, false)], true, SPEEDS);
        assert!(p.walking().is_some());
        // The placement in level 1, far east.
        w.units.get_mut(&key).unwrap().position = Some((1010, 20));
        w.room_units.place(key, Some(DrlgRoomId(2)));
        p.frame(&w, [], true, SPEEDS);
        assert!(p.walking().is_none());
        assert_eq!(p.cell(), Some((1010, 20)));
    }

    /// Two 40 × 40 rooms: level 3 at x 0, level 1 at x 1000; the player
    /// in the first.
    fn two_levels(x: u16, y: u16) -> (ClientWorld, UnitKey) {
        use super::super::world::ActiveRoom;
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = world_at(x, y);
        let room = |level: u16, id: u32, x0: i32| ActiveRoom {
            x0,
            y0: 0,
            w: 40,
            h: 40,
            level,
            room: DrlgRoomId(id),
        };
        w.active_rooms = Some(vec![room(3, 1, 0), room(1, 2, 1000)]);
        w.room_units.place(key, Some(DrlgRoomId(1)));
        (w, key)
    }

    /// The player's mode request `(code, x, y)` as the bridge stores it.
    fn request(w: &mut ClientWorld, key: UnitKey, code: u8, x: i32, y: i32) {
        let u = w.units.get_mut(&key).unwrap();
        u.last_mode_request = Some(super::super::world::ModeRequest {
            code,
            record: [x, y, 0, 0, 0, 0, 0],
        });
        u.mode_requests += 1;
    }

    #[test]
    fn an_arrival_walk_out_starts_at_the_arrival_point() {
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [], true, SPEEDS);
        // One receive: the warp's 0x0D code 1 to (1013, 23), then the 0x15
        // at the arrival point (1010, 20) in level 1.
        request(&mut w, key, 1, 1013, 23);
        w.units.get_mut(&key).unwrap().position = Some((1010, 20));
        w.room_units.place(key, Some(DrlgRoomId(2)));
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.walking().map(|w| w.to), Some(WalkTo::Point(1013, 23)));
        assert_eq!(p.cell(), Some((1010, 20)));
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        assert_eq!(p.cell(), Some((1013, 23)));
        assert!(p.walking().is_none());
    }

    /// B025: after a walk the facing is the walk's; a cast or attack at a
    /// target (the model's `face_point`, counted in `faces`) turns the
    /// player and the prediction's facing follows, also to a direction the
    /// player had before.
    #[test]
    fn a_cast_turns_the_kept_walk_facing() {
        let (mut w, key) = world_at(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(30, 20, false)], true, SPEEDS);
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        let walked = p.facing().expect("facing after a walk");
        // The model turns the player to 20 (a cast at a target).
        let u = w.units.get_mut(&key).unwrap();
        u.path_turn = Some((20, 2));
        u.faces = 1;
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.facing(), Some(20));
        // Another turn to the walk's own direction counts as well.
        let u = w.units.get_mut(&key).unwrap();
        u.path_turn = Some((walked, 2));
        u.faces = 2;
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.facing(), Some(walked));
    }

    /// B012: a portal arrival's walk-out (S→C 0x0D code 1) is walked by
    /// the prediction; the model's walk mode never ends by itself, so
    /// when the prediction's walk ends the player stands (neutral, not the
    /// model's walk, which the art would draw forever).
    #[test]
    fn a_server_walk_out_ends_in_the_neutral_pose() {
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [], true, SPEEDS);
        request(&mut w, key, 1, 1013, 23);
        w.units.get_mut(&key).unwrap().position = Some((1010, 20));
        w.room_units.place(key, Some(DrlgRoomId(2)));
        // Level 1 is a town: the walk is mode 6, the neutral 5.
        let (neutral, walk) = crate::bridge::modes::neutral_walk(&w, key);
        w.units.get_mut(&key).unwrap().mode = walk;
        p.frame(&w, [], false, SPEEDS);
        assert!(p.walking().is_some());
        assert_eq!(p.stood_mode(&w), None, "walking: the walk mode is right");
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        assert!(p.walking().is_none());
        assert_eq!(
            p.stood_mode(&w),
            Some(neutral),
            "the walk-out ended: neutral"
        );
        // A click drops it: a later server request starts afresh.
        p.walk(walk_point(1015, 25, false));
        assert_eq!(p.stood_mode(&w), None);
    }

    /// The soft hit (S→C 0x0D code 0x13) of a monster's spell sets no
    /// mode: the player standing in a waypoint arrival's unwalked walk-out
    /// stays in the neutral pose (R3 D-fx-3: it drew the walk cel from the
    /// hit on); a mode-setting request (get-hit, code 6) ends the pose.
    #[test]
    fn a_soft_hit_keeps_the_arrival_standing_pose() {
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [], true, SPEEDS);
        p.waypoint_sent();
        request(&mut w, key, 1, 1013, 23);
        w.units.get_mut(&key).unwrap().position = Some((1010, 20));
        w.room_units.place(key, Some(DrlgRoomId(2)));
        let (neutral, walk) = crate::bridge::modes::neutral_walk(&w, key);
        w.units.get_mut(&key).unwrap().mode = walk;
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.stood_mode(&w), Some(neutral));
        request(&mut w, key, 0x13, 1010, 20);
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.stood_mode(&w), Some(neutral), "the soft hit sets no mode");
        // A request that sets a mode ends the standing pose.
        request(&mut w, key, 0x06, 1010, 20);
        w.units.get_mut(&key).unwrap().mode = 4;
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.stood_mode(&w), None);
    }

    /// A soft hit (0x13) on a walking player sets no mode: the predicted
    /// walk and its animation start go on (measured); with the mode
    /// changed in the same pass (a cast, then
    /// the soft hit as the last request) the walk ends.
    #[test]
    fn a_soft_hit_while_walking_keeps_the_walk() {
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(40, 20, false)], true, SPEEDS);
        w.units.get_mut(&key).unwrap().mode = 2;
        p.frame(&w, [], true, SPEEDS);
        let since = p.walk_since();
        assert!(p.walking().is_some() && since.is_some());
        request(&mut w, key, 0x13, 22, 20);
        p.frame(&w, [], true, SPEEDS);
        assert!(p.walking().is_some(), "the soft hit keeps the walk");
        assert_eq!(p.walk_since(), since, "and its animation start");
        // A cast set mode 10 before the soft hit of the same pass.
        request(&mut w, key, 0x16, 30, 20);
        request(&mut w, key, 0x13, 23, 20);
        w.units.get_mut(&key).unwrap().mode = 10;
        p.frame(&w, [], true, SPEEDS);
        assert!(p.walking().is_none());
    }

    #[test]
    fn a_level_change_to_the_same_cell_moves_the_prediction() {
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(30, 20, false)], true, SPEEDS);
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        assert_eq!(p.cell(), Some((30, 20)));
        // An act change places the player at (20, 20) again, now in the
        // other act's room at the same coordinates: the model position
        // does not change.
        w.act = Some(super::super::world::ActLoad {
            act: 4,
            init_seed: 0,
            town_level: 109,
            f8: 0,
        });
        w.room_units.place(key, Some(DrlgRoomId(2)));
        w.active_rooms.as_mut().unwrap()[1].x0 = 0;
        w.active_rooms.as_mut().unwrap()[0].x0 = 1000;
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.cell(), Some((20, 20)));
    }

    #[test]
    fn walking_across_a_level_border_is_no_placement() {
        use super::super::world::ActiveRoom;
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = world_at(35, 20);
        let room = |level: u16, id: u32, x0: i32| ActiveRoom {
            x0,
            y0: 0,
            w: 40,
            h: 40,
            level,
            room: DrlgRoomId(id),
        };
        w.active_rooms = Some(vec![room(1, 1, 0), room(2, 2, 40)]);
        w.room_units.place(key, Some(DrlgRoomId(1)));
        let mut p = Predict::new();
        p.frame(&w, [walk_point(50, 20, false)], true, SPEEDS);
        while p.cell().is_some_and(|c| c.0 < 41) {
            p.frame(&w, [], true, SPEEDS);
        }
        // The view's room recache links the player to the next level's
        // room; the model position stays (35, 20).
        w.room_units.place(key, Some(DrlgRoomId(2)));
        let at = p.cell();
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.cell(), at);
        // B002: the walk goes on over the border and ends on its target,
        // as the server's does (it used to stop at the border).
        assert!(p.walking().is_some(), "the walk survives the border");
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        assert_eq!(p.cell(), Some((50, 20)));
        assert!(p.walking().is_none());
    }

    #[test]
    fn a_walk_out_ahead_of_its_placement_is_held() {
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [], true, SPEEDS);
        // The 0x0D arrives one frame before the 0x15: its target is in
        // the other level; nothing steps toward it from (20, 20).
        request(&mut w, key, 1, 1013, 23);
        p.frame(&w, [], true, SPEEDS);
        assert!(p.walking().is_none());
        assert_eq!(p.cell(), Some((20, 20)));
        w.units.get_mut(&key).unwrap().position = Some((1010, 20));
        w.room_units.place(key, Some(DrlgRoomId(2)));
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.walking().map(|w| w.to), Some(WalkTo::Point(1013, 23)));
    }

    /// The measured waypoint arrival (Rogue Encampment → Cold Plains):
    /// after the
    /// client's C→S 0x49 the server's 0x0D walk to the arrival point
    /// (x + 3, y + 3) comes with the 0x15 at (5168, 4658); the base game takes the 0x15
    /// point and never walks on.
    #[test]
    fn a_waypoint_arrival_is_not_walked_out() {
        use crate::bridge::drlg::DrlgRoomId;
        let (mut w, key) = two_levels(20, 20);
        w.active_rooms.as_mut().unwrap()[1].x0 = 5150;
        w.active_rooms.as_mut().unwrap()[1].y0 = 4640;
        let mut p = Predict::new();
        p.frame(&w, [], true, SPEEDS);
        let tap = WalkTap::default();
        tap.record(&[0x49, 1, 0, 0, 0, 3, 0, 0, 0]);
        assert!(tap.take_waypoint());
        assert!(!tap.take_waypoint(), "taken once");
        p.waypoint_sent();
        request(&mut w, key, 1, 5168 + 3, 4658 + 3);
        w.units.get_mut(&key).unwrap().position = Some((5168, 4658));
        w.room_units.place(key, Some(DrlgRoomId(2)));
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        assert!(p.walking().is_none());
        assert_eq!(p.cell(), Some((5168, 4658)));
        // The next server walk (not a waypoint arrival) is held again.
        request(&mut w, key, 1, 5170, 4660);
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.walking().map(|w| w.to), Some(WalkTo::Point(5170, 4660)));
    }

    #[test]
    fn a_player_walk_or_another_request_drops_a_held_walk_out() {
        let (mut w, key) = two_levels(20, 20);
        let mut p = Predict::new();
        p.frame(&w, [], true, SPEEDS);
        request(&mut w, key, 1, 1013, 23);
        p.frame(&w, [walk_point(30, 20, false)], false, SPEEDS);
        assert_eq!(p.walking().map(|w| w.to), Some(WalkTo::Point(30, 20)));
        // Not a walk request: no server walk, and its mode (0xD) ends the
        // walk under way.
        request(&mut w, key, 1, 25, 25);
        request(&mut w, key, 0x19, 25, 25);
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.walking(), None);
        // The same request again is still a new one.
        request(&mut w, key, 1, 25, 25);
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.walking().map(|w| w.to), Some(WalkTo::Point(25, 25)));
        request(&mut w, key, 1, 25, 25);
        p.frame(&w, [], false, SPEEDS);
        assert_eq!(p.walking().map(|w| w.to), Some(WalkTo::Point(25, 25)));
    }

    #[test]
    fn a_hit_or_death_request_ends_the_walk_and_a_walk_code_keeps_it() {
        let (mut w, key) = world_at(100, 100);
        let mut p = Predict::new();
        p.frame(&w, [walk_point(120, 100, false)], true, SPEEDS);
        let at = p.position();
        // The interact sender (code 2) sets no mode: the walk goes on.
        request(&mut w, key, 0x02, 4, 9);
        p.frame(&w, [], true, SPEEDS);
        assert!(p.walking().is_some());
        assert_ne!(p.position(), at);
        // Get-hit (code 6, mode 4): the player stands from here.
        request(&mut w, key, 0x06, 101, 100);
        p.frame(&w, [], true, SPEEDS);
        let stood = p.position();
        assert_eq!(p.walking(), None);
        assert_eq!(p.mode(), None);
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.position(), stood);
        // A new click walks again; death (code 8) ends it as well.
        p.frame(&w, [walk_point(120, 100, false)], true, SPEEDS);
        assert!(p.walking().is_some());
        request(&mut w, key, 0x08, 101, 100);
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.walking(), None);
        // A skill start (0x15 / 0x16, the local cast of a click) too.
        p.frame(&w, [walk_point(120, 100, true)], true, SPEEDS);
        request(&mut w, key, 0x16, 0, 0);
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.walking(), None);
    }

    #[test]
    fn no_local_player_no_prediction() {
        let mut p = Predict::new();
        let w = ClientWorld::default();
        p.frame(&w, [walk_point(1, 1, false)], true, SPEEDS);
        assert_eq!(p.position(), None);
        assert_eq!(p.walking(), None);
        let (mut w, key) = world_at(10, 10);
        p.observe(&w);
        assert_eq!(p.player(), Some(key));
        w.local_player = None;
        p.observe(&w);
        assert_eq!(p.player(), None);
    }

    #[test]
    fn unit_target_follows_the_unit() {
        let (mut w, _) = world_at(100, 100);
        let npc = UnitKey::new(1, 3);
        let mut u = ClientUnit::new(npc);
        u.position = Some((100, 101));
        w.units.insert(npc, u);
        let mut p = Predict::new();
        let to_npc = Walk {
            to: WalkTo::Unit(npc),
            run: false,
        };
        p.frame(&w, [to_npc], true, SPEEDS);
        p.frame(&w, [], true, SPEEDS);
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.cell(), Some((100, 101)));
        assert_eq!(p.walking(), None);
        p.frame(&w, [to_npc], true, SPEEDS);
        w.units.remove(&npc);
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.walking(), None);
    }

    #[test]
    fn unit_walk_ends_when_the_model_walk_ends() {
        let (mut w, key) = world_at(100, 100);
        let wp = UnitKey::new(2, 10);
        let mut u = ClientUnit::new(wp);
        u.position = Some((140, 100));
        w.units.insert(wp, u);
        let mut p = Predict::new();
        let to_wp = Walk {
            to: WalkTo::Unit(wp),
            run: false,
        };
        // Neutral before the model starts its walk: the walk goes on.
        w.units.get_mut(&key).unwrap().mode = 1;
        p.frame(&w, [to_wp], true, SPEEDS);
        assert!(p.walking().is_some());
        // The model walks, then its walk ends in neutral (the operate
        // reached the unit): the prediction ends with it.
        w.units.get_mut(&key).unwrap().mode = 2;
        p.frame(&w, [], true, SPEEDS);
        assert!(p.walking().is_some());
        w.units.get_mut(&key).unwrap().mode = 1;
        p.frame(&w, [], true, SPEEDS);
        assert_eq!(p.walking(), None);
    }

    #[test]
    fn facing_follows_the_direction_vector() {
        let c = |x: i32, y: i32| cell_centre(((100 + x) as u16, (100 + y) as u16));
        let o = c(0, 0);
        assert_eq!(facing(o, c(10, 0)), Some(56));
        assert_eq!(facing(o, c(0, -10)), Some(40));
        assert_eq!(facing(o, c(5, 5)), Some(0));
        assert_eq!(facing(o, c(3, 1)), Some(59));
        assert_eq!(facing(o, o), None);
    }

    #[test]
    fn facing_a_far_point_keeps_the_direction() {
        // A position from before a level change: thousands of sub-tiles.
        let c = |x: u16, y: u16| cell_centre((x, y));
        assert_eq!(facing(c(100, 100), c(5100, 100)), Some(56));
        assert_eq!(
            facing(c(5100, 5100), c(100, 5100)),
            facing(c(110, 100), c(100, 100))
        );
        assert_eq!(facing(c(100, 100), c(3100, 1100)), Some(59));
        assert_eq!(
            facing(c(9000, 9000), c(20, 20)),
            facing(c(110, 110), c(100, 100))
        );
    }

    #[test]
    fn the_walk_sets_the_facing_and_it_stays_when_standing() {
        let (w, _) = world_at(100, 100);
        let mut p = Predict::new();
        p.observe(&w);
        assert_eq!(p.facing(), None);
        // Taken before any tick: faces the target at once.
        p.frame(&w, [walk_point(100, 90, false)], false, SPEEDS);
        assert_eq!(p.facing(), Some(40));
        for _ in 0..40 {
            p.frame(&w, [], true, SPEEDS);
        }
        assert_eq!((p.walking(), p.cell()), (None, Some((100, 90))));
        assert_eq!(p.facing(), Some(40), "kept after the walk ends");
        p.frame(&w, [walk_point(110, 90, true)], true, SPEEDS);
        assert_eq!(p.facing(), Some(56));
    }

    #[test]
    fn a_run_with_no_stamina_walks() {
        let (mut w, key) = world_at(100, 100);
        w.units.get_mut(&key).unwrap().stats.insert(10, 5 << 8);
        let mut p = Predict::new();
        p.observe(&w);
        p.walk(walk_point(110, 100, true));
        p.tick(&w, SPEEDS);
        assert_eq!(p.mode(), Some(3));
        let run_at = p.position().unwrap().0;
        w.units.get_mut(&key).unwrap().stats.insert(10, 0);
        p.tick(&w, SPEEDS);
        assert_eq!(p.mode(), Some(2));
        let walked = p.position().unwrap().0 - run_at;
        assert_eq!(walked, 6 * 0x1000);
    }

    #[test]
    fn a_raw_stamina_below_256_walks_on_the_client() {
        // Server raw stamina 100 reaches the model as 0x96 stamina
        // 100 >> 8 = 0: the client walks while the server (raw > 0)
        // still runs. The gap is the base game's own.
        let raw: i32 = 100;
        let (mut w, key) = world_at(100, 100);
        w.units.get_mut(&key).unwrap().stats.insert(10, raw >> 8);
        let mut p = Predict::new();
        p.observe(&w);
        p.walk(walk_point(110, 100, true));
        p.tick(&w, SPEEDS);
        assert_eq!(p.mode(), Some(2));
    }

    fn walk_point(x: u16, y: u16, run: bool) -> Walk {
        Walk {
            to: WalkTo::Point(x, y),
            run,
        }
    }
}
