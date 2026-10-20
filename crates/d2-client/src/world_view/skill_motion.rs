//! The client motion of skill moves in the play preview (measured on a
//! Barbarian's Leap and Whirlwind): Leap's arc and Whirlwind's spin.
//!
//! The skill's mode request puts the local player in mode 18 (SQ): the
//! client plays the skill's sequence (`seqnum` of
//! the skills row, weapon class `hth` as the composite draws a preview
//! player), one sequence frame per client update (speed 256), drawing
//! each frame's mode and frame ([`SkillMotion::drawn`]). The client skill
//! do is the update where the sequence crosses its first event byte 1.
//!
//! - **Leap arc**: at the do (Leap: frame 5),
//!   the leap start with (unit, point) gives the unit a motion record whose timed
//!   arc (height 0) lifts the draw by `oz` pixels. The
//!   record is stepped once per client update from the update after the
//!   do ([`MotionRecord::update`]); the draw adds `(ox, oy + oz)`
//!   ([`MotionRecord::draw_offset`], fed through `ViewSource::unit_offset`).
//!   While the record lives the sequence holds frame 11
//!   ([`LEAP_HOLD_FRAME`]); it resumes at the landing and the mode ends
//!   after the last frame.
//! - **Whirlwind spin**: the sequence (`seqnum` 10: A1 0, 1, 2, 3, 3, 4,
//!   5, 6) loops over its last four frames ([`SPIN_LOOP`]) while the unit
//!   stays in mode 18, i.e. while its path runs: from the do (frame 3) the
//!   unit moves one walk step a client update and leaves mode 18 at the
//!   end of the loop its path's end falls in ([`whirl_updates`]).

use std::collections::BTreeMap;

use bevy::prelude::*;
use d2_sim::skills::sequences::{self, SeqFrame};

use crate::bridge::mirror::bridge_frame;
use crate::bridge::predict::Speeds;
use crate::bridge::world::{ClientWorld, ModeRequest, SkillRow, UnitKey};
use crate::bridge::BridgeResource;
use crate::rules::unit_composite::{motion, MotionRecord};

use super::walk::{preview_walk_frame, PreviewWalk};
use super::WorldViewState;

/// `srvdofunc` of Whirlwind, Leap and Leap Attack.
const WHIRLWIND: i16 = 76;
const LEAP: i16 = 77;
const LEAP_ATTACK: i16 = 78;

/// The COF weapon class the sequence is looked up with:
/// `hth`, the class the composite draws a
/// preview player with (`unit_assets` weapon class; D2RUST's own, unverified,
/// the preview).
const PREVIEW_CLASS: usize = 0;

/// The sequence speed: 8.8 frames a client update (+0x3C := 256;
/// measured `s` 256 in mode 18 for both skills).
pub const SEQ_SPEED: u32 = 256;

/// The Leap sequence frame held while the unit is airborne
/// (`a1-cold-plains-leap-bar.tsv`: `f` 2816 from the 11th update of mode
/// 18 until the record is done).
pub const LEAP_HOLD_FRAME: usize = 11;

/// Whirlwind's loop: the last four sequence frames (A1 3, 4, 5, 6;
/// `a1-cold-plains-whirlwind-bar.tsv`: `f` 1536 → 768 → 1024 → 1280 →
/// 1536).
pub const SPIN_LOOP: usize = 4;

/// `g` of the leap arc: `-270·d + 8,462` for `d < 27`, else
/// `-72·d + 1,224` (table), at least 500.
pub fn leap_gravity(d: i32) -> i32 {
    let g = if d < 27 {
        8462 - 270 * d
    } else {
        1224 - 72 * d
    };
    g.max(500)
}

/// The timed arc on `rec` (flag 2; ticks := max(`n`, 1);
/// x = y = 0; z := `h` ≪; vx = vy = 0; az := -0x1000 when 0; vz :=
/// (-`h`·2048 - az·`n`²/2) / `n`, C division).
pub fn timed_arc(rec: &mut MotionRecord, height: i32, n: i32) {
    let n = n.max(1);
    rec.flags |= motion::TIMED;
    rec.ticks_left = n;
    rec.pos = [0, 0, height.wrapping_shl(11)];
    rec.vel[0] = 0;
    rec.vel[1] = 0;
    if rec.acc[2] == 0 {
        rec.acc[2] = -0x1000;
    }
    let az = i64::from(rec.acc[2]);
    let n64 = i64::from(n);
    let vz = (-i64::from(height) * 2048 - az * n64 * n64 / 2) / n64;
    rec.vel[2] = vz as i32;
}

/// The leap's motion record: `d` the distance to the point
/// in sub-tiles (at least 1), `s` the unit's path speed (the class run
/// velocity, measured 9 for the barbarian). `None` when `s` is 0 (no
/// record is created).
pub fn leap_record(d: i32, s: i32) -> Option<MotionRecord> {
    if s == 0 {
        return None;
    }
    let d = d.max(1);
    // `n := (d << 16) / (s << 12)`, one less when above 1 (measured:
    // d 8 → q 14, n 13; d 11 → q 19, n 18).
    let q = (i64::from(d) << 16) / (i64::from(s) << 12);
    let n = if q > 1 { q - 1 } else { q } as i32;
    let mut rec = MotionRecord::default();
    // A fresh record is zeroed; `az := -g` (unshifted).
    rec.acc[2] = -leap_gravity(d);
    timed_arc(&mut rec, 0, n);
    Some(rec)
}

/// The mode-18 updates of a whirl from its do (the do's own update
/// included; the 3 updates before the do are not counted):
/// the path steps from update
/// 3 at `walk << 12` a client update (charstats
/// WalkVelocity << 8), so it arrives on update a = 2 + ⌈len / step⌉;
/// mode 18 ends at the end of the A1 3–6 loop running then (no end in the
/// do, machine step 5): N = max(8, 4·⌈a / 4⌉)
/// updates, the request's included (measured: d 8 → 24, d 14 → 40 at
/// step 0x6000). A walk of 0 (no speeds) ends on the do's update.
///
/// `len` is the path's Euclidean length in 16.16 sub-tiles
/// ([`path_length`]): the step is the velocity along the direction vector
/// (rows of length ≈ 4096), not the path distance
/// metric (`max + min / 2`) of the cast's reach. Measured on a recorded
/// diagonal whirl (client state): a whirl from (5143, 4263) to (5153,
/// 4268) steps 35132 a client update from update 4 (velocity with the
/// barbarian's speed bonus) and arrives on the 21st step, the Euclidean
/// length 732715 (11.18 sub-tiles; the steps sum to 732748; the metric 12
/// would take 23); mode 18 ends after N = 24 updates.
pub fn whirl_updates(len: i64, walk: i32) -> u32 {
    if walk <= 0 {
        return 1;
    }
    let len = len.max(0);
    let step = i64::from(walk) << 12;
    let a = 2 + (len + step - 1) / step;
    let n = (4 * ((a + 3) / 4)).max(8);
    (n - 3) as u32
}

/// Which motion a sequence run drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Leap,
    Whirl,
}

/// The local player's mode-18 run: its sequence position, the do, and the
/// leap record or the whirl path.
#[derive(Debug, Clone)]
pub struct SeqRun {
    kind: Kind,
    frames: &'static [SeqFrame],
    /// The sequence frame drawn now (one per update at [`SEQ_SPEED`]).
    pos: usize,
    /// The skill's point (sub-tiles).
    target: (i32, i32),
    /// The do ran (first event byte 1 crossed).
    done_do: bool,
    /// The leap record. The do's update makes it `pending`; it shows the
    /// update after, unstepped, and is stepped from the one after that
    /// (the leap facts: no record at frame 5, `mn` 13 / `oz` 0 at frame 6,
    /// 12 / −20 at frame 7).
    leap: Option<MotionRecord>,
    pending: Option<MotionRecord>,
    /// Whirl: mode-18 updates left after the do's.
    path_left: Option<u32>,
}

impl SeqRun {
    /// A run of `frames` toward `target`, at frame 0 (the request's update).
    fn new(kind: Kind, frames: &'static [SeqFrame], target: (i32, i32)) -> Self {
        Self {
            kind,
            frames,
            pos: 0,
            target,
            done_do: false,
            leap: None,
            pending: None,
            path_left: None,
        }
    }

    /// A Leap / Leap Attack run.
    pub fn leap(frames: &'static [SeqFrame], target: (i32, i32)) -> Self {
        Self::new(Kind::Leap, frames, target)
    }

    /// A Whirlwind run.
    pub fn whirl(frames: &'static [SeqFrame], target: (i32, i32)) -> Self {
        Self::new(Kind::Whirl, frames, target)
    }

    /// The sequence frame drawn now: (mode, frame).
    pub fn drawn(&self) -> (u32, usize) {
        let f = self.frames[self.pos.min(self.frames.len() - 1)];
        (u32::from(f.mode), usize::from(f.frame))
    }

    /// The live leap record.
    pub fn record(&self) -> Option<&MotionRecord> {
        self.leap.as_ref()
    }

    /// One client update; `at` the unit's cell (the do measures from it),
    /// `speeds` the class speeds. `false` when the unit left mode 18.
    pub fn update(&mut self, at: (i32, i32), speeds: Option<Speeds>) -> bool {
        // The leap record: stepped from the update after the do; done
        // (flag 1, the update after ticks 0) is the landing.
        if let Some(rec) = &mut self.leap {
            if rec.update(false, None).is_err() || rec.flags & motion::DONE != 0 {
                self.leap = None;
            }
        }
        if let Some(rec) = self.pending.take() {
            self.leap = Some(rec);
        }
        // The whirl path: the update that reaches its end leaves mode 18.
        if let Some(n) = &mut self.path_left {
            *n = n.saturating_sub(1);
            if *n == 0 {
                return false;
            }
        }
        let hold = self.kind == Kind::Leap && self.pos == LEAP_HOLD_FRAME && self.leap.is_some();
        if hold {
            return true;
        }
        let step = (SEQ_SPEED >> 8) as usize;
        let from = self.pos;
        let mut next = from + step;
        if next >= self.frames.len() {
            match self.kind {
                Kind::Leap => return false,
                Kind::Whirl => next = self.frames.len().saturating_sub(SPIN_LOOP),
            }
        }
        self.pos = next;
        // frame range: the event bytes of the frames
        // crossed, (from, next].
        let crossed = (from + 1..=next).any(|i| self.frames.get(i).is_some_and(|f| f.event == 1));
        if crossed && !self.done_do {
            self.done_do = true;
            self.skill_do(at, speeds);
        }
        true
    }

    /// The client skill do.
    fn skill_do(&mut self, at: (i32, i32), speeds: Option<Speeds>) {
        let d = distance(at, self.target);
        match self.kind {
            Kind::Leap => {
                let s = speeds.map_or(0, |s| i32::from(s.run));
                self.pending = leap_record(d, s);
            }
            Kind::Whirl => {
                let w = speeds.map_or(0, |s| i32::from(s.walk));
                self.path_left = Some(whirl_updates(path_length(at, self.target), w));
            }
        }
    }
}

/// What the local player's skill move shows now.
#[derive(Resource, Debug, Default)]
pub struct SkillMotion {
    /// The request the last start was made for.
    seen: Option<(UnitKey, ModeRequest)>,
    seen_ticks: u64,
    runs: BTreeMap<UnitKey, SeqRun>,
}

/// The sub-tile a skill mode request aims at: code 0x15 a point
/// (record 2, 3), code 0x16 a unit (type, GUID) and its cell. The skill is
/// record 0 (`bridge::modes::skill_mode`). Code 0x16's record 2, 3 are
/// the target unit type / GUID (by the request timing).
fn request_point(world: &ClientWorld, r: &ModeRequest) -> Option<(i32, i32)> {
    match r.code {
        0x15 => Some((r.record[2], r.record[3])),
        0x16 => {
            let key = UnitKey::new(u8::try_from(r.record[2]).ok()?, r.record[3] as u32);
            let (x, y) = world.units.get(&key)?.position?;
            Some((i32::from(x), i32::from(y)))
        }
        _ => None,
    }
}

/// Max(|dx|, |dy|) + min(|dx|, |dy|) / 2.
/// The Euclidean length (16.16) between two sub-tile points.
pub fn path_length(a: (i32, i32), b: (i32, i32)) -> i64 {
    let (dx, dy) = (f64::from(a.0 - b.0), f64::from(a.1 - b.1));
    (dx.hypot(dy) * 65536.0).round() as i64
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    let (dx, dy) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
    dx.max(dy) + dx.min(dy) / 2
}

impl SkillMotion {
    /// One bridge frame. `row_of` is the client skills table, `speeds`
    /// the class's charstats speeds. `cell` is the local player's own
    /// path cell when the client walks it
    /// ([`crate::bridge::predict::Predict::cell`]); the model's position
    /// otherwise. Each server tick is one client update.
    pub fn frame(
        &mut self,
        world: &ClientWorld,
        row_of: impl Fn(u16) -> Option<SkillRow>,
        speeds: Option<Speeds>,
        cell: Option<(u16, u16)>,
    ) {
        let updates = world.server_ticks.saturating_sub(self.seen_ticks);
        self.seen_ticks = world.server_ticks;
        let Some(local) = world.local() else {
            self.runs.clear();
            return;
        };
        let key = local.key;
        // measures from the client unit's position, its own
        // path cell (creator): the
        // server sends a walking player nothing, so the model's position
        // is the last placement.
        let at = cell
            .or(local.position)
            .map(|c| (i32::from(c.0), i32::from(c.1)));
        if let Some(at) = at {
            for _ in 0..updates.min(1024) {
                self.runs.retain(|_, run| run.update(at, speeds));
            }
        }
        // A 0x13 soft hit sets no mode and resets no path in the base game:
        // it neither ends a
        // running sequence nor hides a cast of the same tick.
        let req = local.last_path_request.map(|(r, _)| r);
        if self.seen != req.map(|r| (key, r)) {
            self.seen = req.map(|r| (key, r));
            self.runs.remove(&key);
            if let Some(r) = req {
                self.start(world, &row_of, key, &r);
            }
        }
    }

    /// The skill mode request: mode 18 from sequence frame 0 on this
    /// update, with no machine step (request
    /// timing: the queued request loads frame 0 and runs the client
    /// start; the next update is the first step; settles).
    fn start(
        &mut self,
        world: &ClientWorld,
        row_of: &impl Fn(u16) -> Option<SkillRow>,
        key: UnitKey,
        r: &ModeRequest,
    ) {
        let Some(row) = u16::try_from(r.record[0]).ok().and_then(row_of) else {
            return;
        };
        let Some(to) = request_point(world, r) else {
            return;
        };
        let Some(frames) = sequences::lookup(row.seqnum, PREVIEW_CLASS).filter(|f| !f.is_empty())
        else {
            return;
        };
        let run = match row.srvdofunc {
            // Leap Attack shares Leap's start, do (frame 5) and hold
            // (frame 11): one `cltstfunc` 30, and the leap step runs the
            // same frame-11 landing check. Its target step after landing
            // is not modelled.
            LEAP | LEAP_ATTACK => SeqRun::leap(frames, to),
            WHIRLWIND => SeqRun::whirl(frames, to),
            _ => return,
        };
        self.runs.insert(key, run);
    }

    /// The draw offsets `(ox, oy + oz)` of the units in an arc.
    pub fn offsets(&self) -> BTreeMap<UnitKey, (i32, i32)> {
        self.runs
            .iter()
            .filter_map(|(k, r)| Some((*k, r.record()?.draw_offset())))
            .collect()
    }

    /// The shadow offsets `(ox + oz / 2, oy + oz / 2)` of the units in an
    /// arc (half the height on both axes).
    pub fn shadow_offsets(&self) -> BTreeMap<UnitKey, (i32, i32)> {
        self.runs
            .iter()
            .filter_map(|(k, r)| {
                let [ox, oy, oz] = r.record()?.offset;
                Some((*k, (ox.wrapping_add(oz / 2), oy.wrapping_add(oz / 2))))
            })
            .collect()
    }

    /// The unit in mode 18 now and the sequence frame it draws: (unit,
    /// drawn mode, drawn frame).
    pub fn drawn(&self) -> Option<(UnitKey, u32, usize)> {
        self.runs.iter().next().map(|(k, r)| {
            let (m, f) = r.drawn();
            (*k, m, f)
        })
    }
}

/// Steps [`SkillMotion`] after the walk prediction and hands the offsets
/// to the feed and the drawn sequence frame to the unit art.
pub fn skill_motion_frame(
    bridge: Res<BridgeResource>,
    walk: Res<PreviewWalk>,
    mut motion: ResMut<SkillMotion>,
    mut state: ResMut<WorldViewState>,
) {
    let world = bridge.0.world();
    let cell = walk
        .predict
        .cell()
        .filter(|_| walk.predict.player() == world.local().map(|u| u.key));
    motion.frame(world, |id| bridge.0.skill_row(id), walk.speeds, cell);
    state.feed.set_motion_offsets(motion.offsets());
    state
        .feed
        .set_motion_shadow_offsets(motion.shadow_offsets());
    if let Some(art) = &walk.art {
        let mut art = art.write().unwrap_or_else(|e| e.into_inner());
        let drawn = motion.drawn();
        art.sequence = drawn.map(|(k, _, f)| (k, f));
        if let Some((k, m, _)) = drawn {
            art.pose_mode = Some((k, m));
        }
    }
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_skill_motion(app: &mut App) {
    app.add_systems(
        PreUpdate,
        skill_motion_frame
            .after(preview_walk_frame)
            .after(bridge_frame)
            .before(crate::bridge::mirror::mirror_units)
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<PreviewWalk>)
            .run_if(resource_exists::<WorldViewState>)
            .run_if(resource_exists::<SkillMotion>),
    );
}

/// Adds [`SkillMotion`] and [`skill_motion_frame`] after the walk frame.
pub fn add_skill_motion(app: &mut impl crate::app::session::Setup) {
    app.register("skill-motion", register_skill_motion);
    app.world_mut().init_resource::<SkillMotion>();
}

#[cfg(test)]
mod tests;
