//! The clock of the UI's own timers: screen-message lifetimes, the cube's
//! transmute art, the cursor, the overhead bubbles, the Controls screen's
//! blink and error line, the message log's held arrows, the shop and
//! socket click gates and the waiting note.
//!
//! The timers are stated in milliseconds (or in 40 ms steps), so they must
//! last the same in seconds at any frame rate. Live play reads the host's
//! wall clock ([`super::OriginalUi::set_menu_wall_ms`]); a fixed display
//! clock (`play --display-clock`) stands in for it, as for the Esc menu's
//! pentagrams, so shots repeat. Check runs have no wall clock: one bridge
//! frame is one 40 ms tick there, so the clock is the frame count at 40 ms
//! and their draws are the same on every run.

use crate::rules::camera::CLIENT_TICK_MS;

/// The length of one UI step (and of one check-run frame), in ms.
pub const STEP_MS: u64 = CLIENT_TICK_MS as u64;

/// The UI clock in ms: the wall clock `wall` (live play; a running display
/// clock replaces it), else `frames` bridge frames at [`STEP_MS`].
pub fn ui_ms(wall: Option<u64>, frames: u64) -> u64 {
    match wall {
        Some(w) => crate::world_view::display_clock::now_ms().unwrap_or(w),
        None => frames.wrapping_mul(STEP_MS),
    }
}

/// [`ui_ms`] cut to the 32-bit millisecond clock the timers compare
/// with wrapping arithmetic.
pub fn ui_ms32(wall: Option<u64>, frames: u64) -> u32 {
    ui_ms(wall, frames) as u32
}

/// A counter that steps once per [`STEP_MS`] of the UI clock, for rules
/// written as "once per frame" at 40 ms frames: started at a clock value,
/// it reads 0 there and `n` after `n` whole steps, however many draws fell
/// in between.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepCounter {
    start: Option<u64>,
    taken: u64,
}

impl StepCounter {
    /// Restarts the count at the next [`Self::steps`] call.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The steps since the start at `now_ms` (the first call starts the
    /// count and reads 0).
    pub fn count(&mut self, now_ms: u64) -> u64 {
        let start = *self.start.get_or_insert(now_ms);
        now_ms.saturating_sub(start) / STEP_MS
    }

    /// The steps that became due since the last call (the first call
    /// starts the count and gives one step, the draw that starts it).
    /// At most `max` at once, so a stalled frame does not replay a burst.
    pub fn steps(&mut self, now_ms: u64, max: u64) -> u64 {
        if self.start.is_none() {
            self.start = Some(now_ms);
            self.taken = 0;
            return 1;
        }
        let due = self.count(now_ms);
        let new = due.saturating_sub(self.taken);
        self.taken = due;
        new.min(max)
    }
}

/// The draw rates the timer checks run at (draws a second).
#[cfg(test)]
pub(crate) const RATES: [u64; 3] = [25, 60, 144];

/// The wall clock of draw `i` at `fps` draws a second (from 1000 ms).
#[cfg(test)]
pub(crate) fn draw_ms(fps: u64, i: u64) -> u64 {
    1000 + i * 1000 / fps
}

/// The longest gap between two draws at `fps`, in whole ms.
#[cfg(test)]
pub(crate) fn period_ms(fps: u64) -> u64 {
    1000_u64.div_ceil(fps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_runs_count_frames_at_40_ms() {
        assert_eq!(ui_ms(None, 0), 0);
        assert_eq!(ui_ms(None, 25), 1000);
        assert_eq!(ui_ms32(None, 3), 120);
    }

    #[test]
    fn live_play_reads_the_wall_clock() {
        assert_eq!(ui_ms(Some(12_345), 7), 12_345);
    }

    /// Draw times of `secs` seconds at `fps` draws a second.
    fn draws(fps: u64, secs: u64) -> impl Iterator<Item = u64> {
        (0..fps * secs).map(move |i| i * 1000 / fps)
    }

    #[test]
    fn step_counter_steps_once_per_40_ms_at_any_draw_rate() {
        for fps in [25, 60, 144] {
            let mut c = StepCounter::default();
            let total: u64 = draws(fps, 2).map(|t| c.steps(t, 25)).sum();
            // The first draw's step and one per 40 ms up to the last draw
            // (1960 ms or later): the same at every rate.
            assert_eq!(total, 50, "{fps} fps");
            let mut c = StepCounter::default();
            let last = draws(fps, 2).map(|t| c.count(t)).last().unwrap();
            assert_eq!(last, 49, "{fps} fps");
        }
    }

    #[test]
    fn step_counter_caps_a_stall_and_resets() {
        let mut c = StepCounter::default();
        assert_eq!(c.steps(1000, 25), 1);
        assert_eq!(c.steps(1039, 25), 0);
        assert_eq!(c.steps(1040, 25), 1);
        assert_eq!(c.steps(9000, 25), 25);
        c.reset();
        assert_eq!(c.steps(9001, 25), 1);
        assert_eq!(c.count(9001), 0);
    }
}
