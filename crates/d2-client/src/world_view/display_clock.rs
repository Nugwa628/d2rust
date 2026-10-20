//! The fixed display clock (`play --display-clock`): the frame count drives
//! the clock instead of the wall clock, so a run draws the same frames
//! every time.
//!
//! The frames drawn between two ticks depend on time: the smooth camera
//! and unit motion place their picture by how far the display is between
//! ticks, and the server ticks when its clock has moved one tick on. With
//! the wall clock, a frame that shows a smear "40% between ticks" falls
//! somewhere else on the next run. With a display clock, frame `n` always
//! has the same time:
//!
//! - `fixed:FPS`: frame `n` is at `n * 1000 / FPS` ms, as if the display
//!   ran at exactly FPS frames a second.
//! - `phases:0,0.25,0.5,0.75`: each tick gets one frame per phase, at that
//!   fraction of the tick. The first phase must be 0, the frame the tick
//!   runs on; the others are the between-tick frames to look at. The ticks
//!   are those of the game's tick rate ([`PLAY_RATE`], 60 Hz: 16.67 ms), so
//!   the phases line up with the server's ticks.
//!
//! One clock feeds everything that reads time: the server's host clock
//! (through [`crate::bridge::mirror::ScriptedClock`], which `install_play`
//! shares with the server), the bridge's host clock, and Bevy's `Time`, so
//! the smooth camera and unit motion see the same moments. The client's
//! other wall clocks that change pixels read it too ([`now_ms`]): the Esc
//! menu's pentagrams and the light events' wall seconds
//! ([`wall_seconds`]). The frame limiter does not run, and shots name the
//! tick on screen (`super::shots`). The game's
//! tick rules are unchanged: they only see a clock that moves in exact
//! steps. Frames are drawn as fast as the machine allows, so the game runs
//! faster or slower than real time.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::{TimeSystems, TimeUpdateStrategy};

use crate::bridge::mirror::ScriptedClock;

use d2_sim::tick::TickRate;

/// The tick rate play runs its game at (`single_player`'s game): the ticks
/// `phases:` frames are placed in.
pub const PLAY_RATE: TickRate = TickRate::GAME;

/// The parts a tick is cut into for a phase: a phase is kept in
/// thousandths of a tick (0.25 is 250).
const PARTS: u64 = 1000;

/// The real time of the start of server tick `tick` at `rate`, in ms after
/// tick 0 (rounded down): 40 ms a tick at 25 Hz, 16.67 ms at 60 Hz.
pub fn tick_ms(tick: u64, rate: TickRate) -> u64 {
    rate.progress(u64::from(crate::rules::camera::CLIENT_TICK_MS), tick)
}

/// How far `ms` is into its server tick at `rate`, in ms.
fn ms_into_tick(ms: u64, rate: TickRate) -> u64 {
    let mut t = ms * u64::from(rate.hz()) / 1000;
    while tick_ms(t + 1, rate) <= ms {
        t += 1;
    }
    while t > 0 && tick_ms(t, rate) > ms {
        t -= 1;
    }
    ms - tick_ms(t, rate)
}

/// The clock's value at frame 0, in milliseconds: not 0, because the
/// server's tick driver reads a clock of 0 as "not started".
pub const START_MS: u32 = 1000;

/// The Unix seconds [`wall_seconds`] gives at frame 0: a fixed date
/// (2024-01-01 12:00 UTC), so light events start the same every run.
const START_UNIX_SECONDS: i32 = 1_704_110_400;

/// The display clock's milliseconds after frame 0, for the client's
/// other wall clocks; `u64::MAX` while no display clock runs. One play
/// app runs per process, so one value serves it.
static NOW_MS: AtomicU64 = AtomicU64::new(u64::MAX);

/// The display clock's milliseconds after frame 0 when one runs: the
/// wall clock for UI animations that would otherwise read the host's.
pub fn now_ms() -> Option<u64> {
    let ms = NOW_MS.load(Ordering::Relaxed);
    (ms != u64::MAX).then_some(ms)
}

/// Unix seconds for the light events (the bridge's `wall_seconds` host
/// input) under a display clock: a fixed date plus the display clock's
/// seconds. `None` while no display clock runs.
pub fn wall_seconds() -> Option<i32> {
    now_ms().map(|ms| START_UNIX_SECONDS.wrapping_add((ms / 1000) as i32))
}

/// How the display clock moves from frame to frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayClock {
    /// `fixed:FPS`: a steady frame rate.
    Fixed { fps: u32 },
    /// `phases:P[,P..]`: one frame at each fraction of every tick, kept in
    /// thousandths of a tick (0.25 is 250), ascending from 0. The tick's
    /// length comes from the tick rate ([`Self::at`]).
    Phases(Vec<u64>),
}

impl DisplayClock {
    /// Parses `fixed:FPS` (FPS from 1 to 1000) or `phases:P[,P..]` (the
    /// first 0, then ascending fractions below 1, at least 1 ms apart in a
    /// tick at [`PLAY_RATE`]).
    pub fn parse(s: &str) -> Result<Self, String> {
        let bad = |why: &str| format!("--display-clock {s}: {why}");
        match s.trim().split_once(':') {
            Some(("fixed", fps)) => match fps.trim().parse::<u32>() {
                Ok(fps @ 1..=1000) => Ok(Self::Fixed { fps }),
                _ => Err(bad("FPS is a whole number from 1 to 1000")),
            },
            Some(("phases", list)) => {
                let phases = list
                    .split(',')
                    .map(|p| p.trim().parse::<f64>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| bad("phases are numbers like 0,0.25,0.5"))?;
                if phases.first() != Some(&0.0) {
                    return Err(bad("the first phase is 0, the frame the tick runs on"));
                }
                if phases.iter().any(|&p| !(0.0..1.0).contains(&p)) {
                    return Err(bad("phases stay below 1"));
                }
                let parts: Vec<u64> = phases.iter().map(|&p| part_of(p)).collect();
                let clock = Self::Phases(parts);
                if !clock.moves_every_frame(PLAY_RATE) {
                    return Err(bad(
                        "phases go up, at least 1 ms apart in a 60 Hz tick (0.0625)",
                    ));
                }
                Ok(clock)
            }
            _ => Err(bad("expected fixed:FPS or phases:0,0.25,0.5,0.75")),
        }
    }

    /// `--shot-phase P`: how many frames after a tick's first frame the
    /// frame at phase `P` comes. Only a `phases:` clock has the phase.
    pub fn phase_frame(&self, phase: &str) -> Result<u64, String> {
        let bad = || format!("--shot-phase {phase}: one of the --display-clock phases");
        let Self::Phases(phases) = self else {
            return Err(bad());
        };
        let p: f64 = phase.trim().parse().map_err(|_| bad())?;
        if !(0.0..1.0).contains(&p) {
            return Err(bad());
        }
        let part = part_of(p);
        phases
            .iter()
            .position(|&m| m == part)
            .map(|i| i as u64)
            .ok_or_else(bad)
    }

    /// The time of frame `frame`, in milliseconds after frame 0, with the
    /// server ticking at `rate` (a `phases:` frame is placed in that tick:
    /// its start plus the phase's share of its length, rounded down).
    pub fn at(&self, frame: u64, rate: TickRate) -> u64 {
        match self {
            Self::Fixed { fps } => frame * 1000 / u64::from(*fps),
            Self::Phases(phases) => {
                let k = phases.len() as u64;
                let tick = frame / k;
                let (start, end) = (tick_ms(tick, rate), tick_ms(tick + 1, rate));
                start + (end - start) * phases[(frame % k) as usize] / PARTS
            }
        }
    }

    /// Whether every frame is later than the one before at `rate` over a
    /// second of ticks (the tick lengths repeat each second).
    fn moves_every_frame(&self, rate: TickRate) -> bool {
        let Self::Phases(phases) = self else {
            return true;
        };
        let frames = phases.len() as u64 * u64::from(rate.hz()) + 1;
        (1..frames).all(|f| self.at(f, rate) > self.at(f - 1, rate))
    }
}

/// A phase fraction in thousandths of a tick.
fn part_of(p: f64) -> u64 {
    (p * PARTS as f64).round() as u64
}

/// The running display clock: the clock, the server's tick rate, the next
/// frame's number and the shared millisecond value (the server's and the
/// bridge's host clock).
#[derive(Resource)]
pub struct DisplayClockState {
    clock: DisplayClock,
    rate: TickRate,
    frame: u64,
    ms: Arc<AtomicU32>,
    /// The first server tick was reported ([`report_first_tick`]).
    reported: bool,
}

/// Adds the display clock to a play app: inserts the shared
/// [`ScriptedClock`] and moves the clock and Bevy's `Time` on at the start
/// of every frame. Must run before `install_play`, which hands the same
/// clock to the server. The server ticks at [`PLAY_RATE`].
pub fn add(app: &mut App, clock: DisplayClock) {
    add_at(app, clock, PLAY_RATE);
}

/// [`add`] for a server ticking at `rate`.
pub fn add_at(app: &mut App, clock: DisplayClock, rate: TickRate) {
    let ms = Arc::new(AtomicU32::new(START_MS));
    app.insert_resource(ScriptedClock(ms.clone()))
        .insert_resource(DisplayClockState {
            clock,
            rate,
            frame: 0,
            ms,
            reported: false,
        })
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .add_systems(First, advance.before(TimeSystems))
        .add_systems(Last, report_first_tick);
}

/// Moves the clock to this frame's time and tells Bevy's `Time` how far it
/// moved since the last frame.
fn advance(mut state: ResMut<DisplayClockState>, mut strategy: ResMut<TimeUpdateStrategy>) {
    let (frame, rate) = (state.frame, state.rate);
    let now = state.clock.at(frame, rate);
    let step = frame
        .checked_sub(1)
        .map_or(0, |f| now - state.clock.at(f, rate));
    state
        .ms
        .store(START_MS.wrapping_add(now as u32), Ordering::SeqCst);
    NOW_MS.store(now, Ordering::Relaxed);
    *strategy = TimeUpdateStrategy::ManualDuration(Duration::from_millis(step));
    state.frame += 1;
}

/// Prints once which frame the first server tick reached the client on,
/// and how far into the server's tick grid that frame is: with `phases:`
/// it is 0 when the phases line up with the ticks.
fn report_first_tick(
    mut state: ResMut<DisplayClockState>,
    bridge: Option<Res<crate::bridge::mirror::BridgeResource>>,
) {
    if state.reported || bridge.is_none_or(|b| b.0.world().server_ticks == 0) {
        return;
    }
    state.reported = true;
    let frame = state.frame - 1;
    let at = state.clock.at(frame, state.rate);
    println!(
        "display clock: the first server tick reached the client on frame {frame}, {} ms into its tick",
        ms_into_tick(at, state.rate)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_parse() {
        assert_eq!(
            DisplayClock::parse("fixed:60"),
            Ok(DisplayClock::Fixed { fps: 60 })
        );
        assert_eq!(
            DisplayClock::parse("phases:0, 0.25,0.5"),
            Ok(DisplayClock::Phases(vec![0, 250, 500]))
        );
        for bad in [
            "fixed:0",
            "fixed:x",
            "phases:0.25,0.5",
            "phases:0,0.5,0.5",
            "phases:0,1",
            // 0.04 of a 16.67 ms tick is under 1 ms.
            "phases:0,0.04",
            "60",
        ] {
            assert!(DisplayClock::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_fixed_clock_steps_by_the_frame_time() {
        let c = DisplayClock::Fixed { fps: 60 };
        let t: Vec<u64> = (0..7).map(|f| c.at(f, PLAY_RATE)).collect();
        assert_eq!(t, [0, 16, 33, 50, 66, 83, 100]);
        assert_eq!(DisplayClock::Fixed { fps: 25 }.at(3, PLAY_RATE), 120);
    }

    // The phases are fractions of the server's tick at its rate: at 60 Hz
    // a 40 ms tick put the "tick" frames 2.4 ticks apart, off the ticks.
    #[test]
    fn phases_put_frames_at_fractions_of_each_tick_at_25_and_60_hz() {
        let base = TickRate::BASE;
        let c = DisplayClock::parse("phases:0,0.25,0.5,0.75").unwrap();
        let t: Vec<u64> = (0..9).map(|f| c.at(f, base)).collect();
        assert_eq!(t, [0, 10, 20, 30, 40, 50, 60, 70, 80]);
        // 60 Hz: ticks start at 0, 16, 33, 50 ms.
        let t: Vec<u64> = (0..13).map(|f| c.at(f, TickRate::GAME)).collect();
        assert_eq!(t, [0, 4, 8, 12, 16, 20, 24, 28, 33, 37, 41, 45, 50]);
        // Every phase-0 frame is a tick's start at either rate.
        for rate in [base, TickRate::GAME] {
            for tick in 0..200 {
                assert_eq!(c.at(tick * 4, rate), tick_ms(tick, rate), "{rate:?}");
                assert_eq!(ms_into_tick(c.at(tick * 4, rate), rate), 0);
                assert!(ms_into_tick(c.at(tick * 4 + 1, rate), rate) > 0);
            }
        }
        assert_eq!(c.phase_frame("0.5"), Ok(2));
        assert_eq!(c.phase_frame("0"), Ok(0));
        assert!(c.phase_frame("0.3").is_err());
        assert!(DisplayClock::Fixed { fps: 60 }.phase_frame("0").is_err());
        let c = DisplayClock::parse("phases:0,0.4").unwrap();
        let t: Vec<u64> = (0..5).map(|f| c.at(f, base)).collect();
        assert_eq!(t, [0, 16, 40, 56, 80]);
        let t: Vec<u64> = (0..5).map(|f| c.at(f, TickRate::GAME)).collect();
        assert_eq!(t, [0, 6, 16, 22, 33]);
    }

    #[test]
    fn every_frame_moves_the_shared_clock_and_bevy_time() {
        let run = |rate: TickRate| {
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin);
            add_at(&mut app, DisplayClock::parse("phases:0,0.5").unwrap(), rate);
            let ms = app.world().resource::<ScriptedClock>().clone();
            let mut seen = Vec::new();
            for _ in 0..4 {
                app.update();
                let elapsed = app.world().resource::<Time<Real>>().elapsed();
                seen.push((ms.now(), elapsed.as_millis() as u64));
            }
            seen
        };
        assert_eq!(
            run(TickRate::BASE),
            [(1000, 0), (1020, 20), (1040, 40), (1060, 60)]
        );
        // Play's 60 Hz ticks start at 0, 16 and 33 ms.
        assert_eq!(
            run(PLAY_RATE),
            [(1000, 0), (1008, 8), (1016, 16), (1024, 24)]
        );
    }
}
