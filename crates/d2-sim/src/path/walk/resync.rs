//! C→S 0x5F: the position the client believes its player is at. Small
//! differences are ignored, large ones walk the player there, middle ones
//! (15..=45) snap the player when the point is reachable, else the server
//! re-sends the player's position (S→C 0x15). A snap locks the player to
//! the walk branch for 125 frames, or 1500–4500 frames after five snaps
//! within 2250 frames.

use super::find::compute;
use super::messages::reassign_player;
use super::request::{mode, request, Outcome, WalkTarget};
use super::seams::{count, PathWorld, Point, TargetUnit, WalkError, WalkUnits};
use super::step::Walk;
use crate::path::collision::find_room;
use crate::path::tables::PathTables;
use crate::units::UnitId;

/// The message length (id, x u16, y u16).
pub const MESSAGE_LEN: usize = 5;
/// State 108: the resync lock.
pub const STATE_RESYNC: u16 = 108;
/// State 54 (also refused by the reachability test).
pub const STATE_54: u16 = 54;
/// Below this distance the message is ignored.
pub const IGNORE_BELOW: i32 = 5;
/// The snap band: 15 ≤ d ≤ 45.
pub const SNAP_MIN: i32 = 15;
pub const SNAP_MAX: i32 = 45;
/// The reachability compute's move mask, path type and distances.
pub const TEST_MASK: u16 = 0x409;
pub const TEST_TYPE: u32 = 15;
pub const TEST_DISTANCE: u8 = 77;
/// Resync ring: slots, and the age in base ticks (90 s) after which a
/// slot is stale.
pub const RING_SLOTS: usize = 5;
pub const RING_WINDOW: i32 = 2250;
/// Lock delays in base ticks: plain (5 s), and after five snaps within the
/// window one picked by r = seed step % 100 (r < 50: 60 s, r < 75: 120 s,
/// else 180 s). Both are converted through the game's tick rate.
pub const LOCK_DELAY: i32 = 125;
pub const LOCK_TABLE: [(u32, i32); 3] = [(50, 1500), (75, 3000), (100, 4500)];

/// The client's resync ring (client +0x3C0): the frames of the last
/// snaps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResyncRing {
    pub slots: [i32; RING_SLOTS],
}

impl ResyncRing {
    fn stale(frame: i32, slot: i32, window: i32) -> bool {
        slot == 0 || frame.wrapping_sub(slot) > window
    }

    /// `frame` goes into the first slot that is 0 or more than `window`
    /// frames old ([`RING_WINDOW`] at the game's tick rate); none →
    /// nothing recorded. Returns whether it recorded.
    pub fn record(&mut self, frame: i32, window: i32) -> bool {
        match self
            .slots
            .iter_mut()
            .find(|s| Self::stale(frame, **s, window))
        {
            Some(s) => {
                *s = frame;
                true
            }
            None => false,
        }
    }

    /// All five slots non-zero and at most `window` frames old.
    pub fn is_full(&self, frame: i32, window: i32) -> bool {
        self.slots.iter().all(|&s| !Self::stale(frame, s, window))
    }
}

/// What the handler did (the handler result is 0 except for a bad
/// length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resync {
    /// Length ≠ 5: result 3.
    BadLength,
    /// Used skill set, dead, or d < 5.
    Ignored,
    /// Walk branch with a target unit other than the player: nothing.
    KeepsTarget,
    /// Walk branch: the mode request's outcome.
    Walk(Outcome),
    /// Snap: placed, locked for `delay` frames.
    Snapped { delay: i32 },
    /// Not reachable, or the placement failed: S→C 0x15 sent.
    Reassigned,
}

/// Max(|dx|, |dy|) + ⌊min(|dx|, |dy|)/2⌋.
pub fn resync_distance(a: Point, b: Point) -> i32 {
    let (dx, dy) = ((a.x - b.x).abs(), (a.y - b.y).abs());
    dx.max(dy) + dx.min(dy) / 2
}

/// Handler. Returns the
/// handler result and what happened.
pub fn handle_resync<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    player: UnitId,
    msg: &[u8],
) -> Result<(u32, Resync), WalkError> {
    // Rule 1.
    if msg.len() != MESSAGE_LEN {
        return Ok((3, Resync::BadLength));
    }
    if !c.has_client(player) {
        return Err(WalkError::Fatal("0x5F from a player without a client"));
    }
    let x = i32::from(u16::from_le_bytes([msg[1], msg[2]]));
    let y = i32::from(u16::from_le_bytes([msg[3], msg[4]]));
    let to = Point::new(x, y);
    let Some(mut path) = c.load_path(player) else {
        return Err(WalkError::Fatal("0x5F for a player without a path"));
    };
    // Rule 2.
    let d = resync_distance(path.cell(), to);
    if c.used_skill(player).is_some() || c.is_dead(player) || d < IGNORE_BELOW {
        return Ok((0, Resync::Ignored));
    }
    // Rule 3: walk branch.
    if c.has_state(player, STATE_RESYNC) || !(SNAP_MIN..=SNAP_MAX).contains(&d) {
        Walk { t, c: &mut *c }.target_check(&mut path);
        c.store_path(player, &path);
        if path.target_unit.is_some_and(|tu| tu.unit != player) {
            return Ok((0, Resync::KeepsTarget));
        }
        let m = if c.mode(player) == mode::RUN {
            mode::RUN
        } else {
            mode::WALK
        };
        let o = request(t, c, player, None, m, WalkTarget::Point(to), false)?;
        return Ok((0, Resync::Walk(o)));
    }
    // Rule 4.1: reachability.
    let reachable = reachable(t, c, player, &mut path, to)?;
    c.store_path(player, &path);
    // Rule 4.2: placement.
    if reachable && c.place_resync(player, x, y) {
        let (frame, rate) = (c.frame(), c.tick_rate());
        let window = rate.ticks(RING_WINDOW);
        let full = match c.resync_ring(player) {
            Some(ring) => {
                ring.record(frame, window);
                ring.is_full(frame, window)
            }
            None => false,
        };
        let mut delay = LOCK_DELAY;
        if full && c.game_type() == 0 {
            let r = c.seed(player).step() % 100;
            delay = LOCK_TABLE
                .iter()
                .find(|&&(below, _)| r < below)
                .map_or(LOCK_TABLE[2].1, |&(_, d)| d);
        }
        let delay = rate.ticks(delay);
        c.resync_lock(player, frame.wrapping_add(delay));
        return Ok((0, Resync::Snapped { delay }));
    }
    // Rule 4.3: S→C 0x15.
    let at = c.load_path(player).map_or(path.cell(), |p| p.cell());
    let bytes = reassign_player(
        c.unit_type(player) as u8,
        c.guid(player),
        at.x as u16,
        at.y as u16,
        0,
    );
    c.send_to_client(player, &bytes);
    Ok((0, Resync::Reassigned))
}

/// Reachability test. The computed points
/// and count are not restored.
fn reachable<C: PathWorld + WalkUnits + ?Sized>(
    t: &PathTables,
    c: &mut C,
    player: UnitId,
    path: &mut crate::path::DynamicPath,
    to: Point,
) -> Result<bool, WalkError> {
    if find_room(&*c, path.room, to.x, to.y).is_none()
        || c.has_state(player, STATE_54)
        || c.has_state(player, STATE_RESYNC)
    {
        return Ok(false);
    }
    let (mask, ty, max) = (path.move_mask, path.path_type, path.max_distance);
    let target: Result<TargetUnit, Point> = path.target_unit.ok_or(path.target());
    path.move_mask = TEST_MASK;
    path.path_type = TEST_TYPE;
    path.dist_budget = TEST_DISTANCE;
    path.max_distance = TEST_DISTANCE;
    path.set_target_point(to.x as u16, to.y as u16);
    let n = compute(t, c, path, player, false)?;
    let ok = n != 0 && count(path) > 0 && path.point((count(path) - 1) as usize) == to;
    path.move_mask = mask;
    path.path_type = ty;
    match target {
        Ok(tu) => path.target_unit = Some(tu),
        Err(p) => path.set_target_point(p.x as u16, p.y as u16),
    }
    path.dist_budget = max;
    path.max_distance = max;
    Ok(ok)
}
