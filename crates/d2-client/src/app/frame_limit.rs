//! The frame limit: how many frames per second D2RUST draws at most.
//!
//! The player picks Off or 30, 60, 120, 144, 165, 240 or 360 frames per
//! second (`[video] frame_limit`, [`super::config::FRAME_LIMITS`]; 360 is
//! the highest). Off is no limit of the game's own. Which limit is in force
//! depends on VSync ([`limit_in_force`]):
//! - VSync on: the monitor's refresh paces the frames, and a limit below
//!   the refresh rate holds on top of it (30 on a 60 Hz monitor shows
//!   every frame for two refreshes). A limit at the refresh rate is VSync's
//!   own rate and the limiter does not run; limits above it would do
//!   nothing and are greyed on the Video page. So two clocks never pace
//!   the same frames. When the platform does not report the refresh rate,
//!   the limit holds.
//! - VSync off: the limit holds whatever the refresh rate.
//!
//! The limiter waits at the very start of each frame, before the frame's
//! input is read and the world is updated, so the input a frame acts on is
//! as fresh as the limit allows (and the previous frame is drawn while it
//! waits). Frame slots are spaced one period apart on a running deadline,
//! not from the end of the last frame, so the average rate holds the limit
//! exactly; a frame that runs more than a period late starts a fresh
//! schedule rather than letting the next frames rush to catch up.
//!
//! The wait is precise so the frames come evenly: the thread sleeps until
//! just before the slot, then spins for the rest ([`SPIN`]). On Windows the
//! standard library's sleep uses a high-resolution waitable timer
//! (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`) where the system has one, so
//! the sleep itself lands within a fraction of a millisecond.
//!
//! The game logic is not affected: it ticks on its own fixed clock whatever
//! the frame rate.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::time::TimeSystems;

use super::config::{monitor_of, Monitors, PrimaryWindowOn};
use super::vsync::SyncChoice;

/// The highest frame limit.
pub const MAX_FPS: u32 = 360;

/// How long before a slot the limiter stops sleeping and spins: enough to
/// cover how late a sleep can wake on this platform.
#[cfg(windows)]
pub const SPIN: Duration = Duration::from_micros(1000);
#[cfg(not(windows))]
pub const SPIN: Duration = Duration::from_micros(250);

/// The frame rate the limiter holds, if any: `limit` (0: Off) with VSync
/// `vsync` on a monitor refreshing at `refresh_hz` (`None`: unknown).
/// With VSync on, a limit at or above the refresh rate is left to VSync.
pub fn limit_in_force(vsync: bool, limit: u16, refresh_hz: Option<u32>) -> Option<u32> {
    let limit = u32::from(limit).min(MAX_FPS);
    match refresh_hz {
        _ if limit == 0 => None,
        Some(r) if vsync && limit >= r => None,
        _ => Some(limit),
    }
}

/// Whether the Video page shows `limit` greyed: VSync on and the limit
/// above the monitor's refresh rate, where it has no effect. A limit equal
/// to the refresh rate stays: VSync holds it.
pub fn limit_greyed(vsync: bool, limit: u16, refresh_hz: Option<u32>) -> bool {
    vsync && refresh_hz.is_some_and(|r| u32::from(limit) > r)
}

/// A source of time the limiter reads and waits on (the real clock in the
/// game, a fake one in tests).
pub trait Clock {
    /// Time since an arbitrary fixed start.
    fn now(&self) -> Duration;
    /// Waits until `now` is at least `t`.
    fn wait_until(&mut self, t: Duration);
}

/// Spaces frames at most `rate` per second (module docs).
#[derive(Clone, Debug, Default)]
pub struct Limiter {
    rate: u32,
    /// When the next frame may start, on the clock's time line.
    next: Option<Duration>,
}

impl Limiter {
    /// The time between frames at `rate` frames per second.
    pub fn period(rate: u32) -> Duration {
        Duration::from_secs(1) / rate.max(1)
    }

    /// Sets the rate (`None`: no limit); a change starts a fresh schedule.
    pub fn set_rate(&mut self, rate: Option<u32>) {
        let rate = rate.unwrap_or(0);
        if rate != self.rate {
            self.rate = rate;
            self.next = None;
        }
    }

    /// Called once at the start of every frame: waits for the frame's slot.
    /// Without a rate it returns at once.
    pub fn wait(&mut self, clock: &mut impl Clock) {
        if self.rate == 0 {
            return;
        }
        let period = Self::period(self.rate);
        let now = clock.now();
        let slot = match self.next {
            // A frame that ran more than a whole period late: start again
            // from now instead of rushing the next frames.
            Some(next) if now <= next + period => next,
            _ => now,
        };
        if slot > now {
            clock.wait_until(slot);
        }
        self.next = Some(slot + period);
    }
}

/// The game's clock: monotonic time; a sleep to just before the deadline,
/// then a spin to it.
pub struct RealClock(pub Instant);

impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn wait_until(&mut self, t: Duration) {
        let now = self.now();
        if t > now + SPIN {
            std::thread::sleep(t - now - SPIN);
        }
        while self.now() < t {
            std::hint::spin_loop();
        }
    }
}

/// The limiter and its clock, kept between frames.
#[derive(Resource)]
pub struct FrameLimit {
    limiter: Limiter,
    clock: RealClock,
}

impl Default for FrameLimit {
    fn default() -> Self {
        FrameLimit {
            limiter: Limiter::default(),
            clock: RealClock(Instant::now()),
        }
    }
}

/// Adds the limiter: first in every frame, in the menus and in play, ahead
/// of the frame's clock and input. A fixed display clock
/// (`play --display-clock`) draws frames as fast as it can, so the limiter
/// stays off there.
pub fn add(app: &mut App) {
    use crate::world_view::display_clock::DisplayClockState;
    app.init_resource::<FrameLimit>().add_systems(
        First,
        limit_frame
            .before(TimeSystems)
            .run_if(not(resource_exists::<DisplayClockState>)),
    );
}

/// The refresh rate in whole Hz of the monitor the primary window is on,
/// when the platform reports it.
pub fn refresh_hz(on: Option<&bevy::window::OnMonitor>, monitors: &Monitors) -> Option<u32> {
    let mhz = monitor_of(on, monitors)?.refresh_rate_millihertz?;
    (mhz > 0).then(|| (mhz + 500) / 1000)
}

fn limit_frame(
    mut limit: ResMut<FrameLimit>,
    choice: Option<Res<SyncChoice>>,
    monitors: Monitors,
    windows: PrimaryWindowOn<Entity>,
) {
    let on = windows.single().ok().and_then(|(_, on)| on);
    let rate =
        choice.and_then(|c| limit_in_force(c.vsync, c.frame_limit, refresh_hz(on, &monitors)));
    let FrameLimit { limiter, clock } = &mut *limit;
    limiter.set_rate(rate);
    limiter.wait(clock);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::config::FRAME_LIMITS;

    /// A clock that only moves when the test or the limiter moves it.
    #[derive(Default)]
    struct FakeClock {
        now: Duration,
        slept: Duration,
    }

    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.now
        }
        fn wait_until(&mut self, t: Duration) {
            if t > self.now {
                self.slept += t - self.now;
                self.now = t;
            }
        }
    }

    /// Frames per second over `frames` frames at `rate`, each frame's work
    /// taken from `work` in turn.
    fn measured_fps(rate: u32, work: &[Duration], frames: u32) -> f64 {
        let mut clock = FakeClock::default();
        let mut l = Limiter::default();
        l.set_rate(Some(rate));
        l.wait(&mut clock);
        let start = clock.now;
        for i in 0..frames {
            clock.now += work[i as usize % work.len()];
            l.wait(&mut clock);
        }
        f64::from(frames) / (clock.now - start).as_secs_f64()
    }

    #[test]
    fn every_limit_holds_within_one_fps() {
        for &limit in FRAME_LIMITS.iter().filter(|&&l| l > 0) {
            let rate = u32::from(limit);
            let period = Limiter::period(rate);
            // Steady light frames, and frames whose work jitters between
            // almost nothing and almost the whole period.
            let steady = [period / 4];
            let jitter = [
                Duration::ZERO,
                period * 9 / 10,
                period / 3,
                period / 20,
                period * 3 / 4,
            ];
            for work in [&steady[..], &jitter[..]] {
                let fps = measured_fps(rate, work, rate * 10);
                assert!(
                    (fps - f64::from(rate)).abs() <= 1.0,
                    "limit {rate}: {fps:.3} fps"
                );
            }
        }
    }

    /// The stated rule (Display v2, V2): with VSync on, limits above the
    /// refresh rate are greyed, VSync paces at the refresh rate itself, and
    /// lower limits hold. With VSync off every limit holds.
    #[test]
    fn limits_below_the_refresh_rate_hold_with_vsync_on() {
        // Off is never a limit.
        assert_eq!(limit_in_force(true, 0, Some(144)), None);
        assert_eq!(limit_in_force(false, 0, Some(144)), None);
        assert!(!limit_greyed(true, 0, Some(60)));
        // VSync on: only limits below the refresh rate.
        assert_eq!(limit_in_force(true, 30, Some(60)), Some(30));
        assert_eq!(limit_in_force(true, 60, Some(144)), Some(60));
        assert_eq!(limit_in_force(true, 60, Some(60)), None);
        assert_eq!(limit_in_force(true, 144, Some(144)), None);
        assert_eq!(limit_in_force(true, 240, Some(144)), None);
        assert!(limit_greyed(true, 165, Some(144)));
        assert!(!limit_greyed(true, 144, Some(144)));
        assert!(!limit_greyed(true, 120, Some(144)));
        // An unknown refresh rate greys nothing: the limit holds.
        assert_eq!(limit_in_force(true, 240, None), Some(240));
        assert!(!limit_greyed(true, 360, None));
        // VSync off: every limit, whatever the refresh rate.
        for refresh in [None, Some(60), Some(144)] {
            for limit in FRAME_LIMITS {
                assert_eq!(
                    limit_in_force(false, limit, refresh),
                    (limit != 0).then_some(u32::from(limit))
                );
                assert!(!limit_greyed(false, limit, refresh));
            }
        }
    }

    #[test]
    fn frames_slower_than_the_limit_never_wait() {
        let mut clock = FakeClock::default();
        let mut l = Limiter::default();
        l.set_rate(Some(240));
        for _ in 0..100 {
            clock.now += Duration::from_millis(10);
            l.wait(&mut clock);
        }
        assert_eq!(clock.slept, Duration::ZERO);
    }

    #[test]
    fn without_a_limit_nothing_waits() {
        let mut clock = FakeClock::default();
        let mut l = Limiter::default();
        l.set_rate(None);
        for _ in 0..100 {
            clock.now += Duration::from_micros(100);
            l.wait(&mut clock);
        }
        assert_eq!(clock.slept, Duration::ZERO);
    }

    #[test]
    fn a_late_frame_does_not_make_the_next_ones_rush() {
        let mut clock = FakeClock::default();
        let mut l = Limiter::default();
        l.set_rate(Some(100)); // 10 ms slots
        l.wait(&mut clock);
        // One 50 ms hitch, then quick frames: each still waits its slot.
        clock.now += Duration::from_millis(50);
        l.wait(&mut clock);
        let before = clock.now;
        clock.now += Duration::from_millis(1);
        l.wait(&mut clock);
        assert_eq!(clock.now - before, Duration::from_millis(10));
    }

    #[test]
    fn a_new_rate_starts_a_fresh_schedule() {
        let mut clock = FakeClock::default();
        let mut l = Limiter::default();
        l.set_rate(Some(60));
        l.wait(&mut clock);
        l.set_rate(Some(120));
        clock.now += Duration::from_millis(1);
        let before = clock.now;
        l.wait(&mut clock);
        assert_eq!(
            clock.now, before,
            "the first frame at a new rate does not wait"
        );
    }

    #[test]
    fn the_limits_are_within_the_maximum() {
        assert!(FRAME_LIMITS.iter().all(|&c| u32::from(c) <= MAX_FPS));
        assert_eq!(FRAME_LIMITS.iter().copied().max(), Some(MAX_FPS as u16));
    }

    /// The real clock holds each limit within one frame per second over a
    /// second of light frames. Timing on a loaded machine is noisy, so this
    /// runs by hand: `cargo test -p d2-client frame_limit -- --ignored`.
    #[test]
    #[ignore = "wall-clock timing; run by hand"]
    fn the_real_clock_holds_every_limit() {
        for &limit in FRAME_LIMITS.iter().filter(|&&l| l > 0) {
            let rate = u32::from(limit);
            let mut clock = RealClock(Instant::now());
            let mut l = Limiter::default();
            l.set_rate(Some(rate));
            l.wait(&mut clock);
            let start = clock.now();
            let mut worst = Duration::ZERO;
            let mut last = start;
            for _ in 0..rate {
                let work = clock.now() + Limiter::period(rate) / 5;
                while clock.now() < work {
                    std::hint::spin_loop();
                }
                l.wait(&mut clock);
                let now = clock.now();
                worst = worst.max(now - last);
                last = now;
            }
            let fps = f64::from(rate) / (clock.now() - start).as_secs_f64();
            println!(
                "limit {rate}: {fps:.2} fps, longest frame {:.3} ms (period {:.3} ms)",
                worst.as_secs_f64() * 1e3,
                Limiter::period(rate).as_secs_f64() * 1e3
            );
            assert!(
                (fps - f64::from(rate)).abs() <= 1.0,
                "limit {rate}: {fps:.2} fps"
            );
        }
    }
}
