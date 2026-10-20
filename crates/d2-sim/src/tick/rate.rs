//! The simulation's tick rate as a unit of time.
//!
//! D2RUST's game data states speeds, rates and durations "per tick" at the
//! base rate of 25 ticks per second ([`crate::TICKS_PER_SECOND`]): a walk
//! velocity is the distance covered in one 40 ms tick, a timer of 5 ticks
//! lasts 200 ms. [`TickRate`] is the rate a game actually runs at, and this
//! module is the one place that turns those base-rate numbers into amounts
//! for the current rate, so that a game at a faster rate plays at the same
//! real-time speed.
//!
//! Every conversion is integer math (the simulation has no floating
//! point) and is exact at the base rate: at 25 Hz each helper returns its
//! input unchanged, so a game at the base rate is bit-identical to one
//! that never converts.

/// Ticks per second a game runs at. The game's tables are stated at
/// [`TickRate::BASE`]; a game at a faster rate converts them through this
/// value and plays at the same real-time speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TickRate {
    hz: u32,
}

impl Default for TickRate {
    fn default() -> Self {
        Self::BASE
    }
}

impl TickRate {
    /// The base rate, 25 Hz: the rate the game's tables are stated at.
    pub const BASE: TickRate = TickRate {
        hz: crate::TICKS_PER_SECOND,
    };
    /// The base game runs at 60 Hz; table units remain at [`Self::BASE`].
    pub const GAME: TickRate = TickRate { hz: 60 };
    /// The fastest supported rate, 1000 Hz (one tick per millisecond).
    pub const MAX_HZ: u32 = 1000;

    /// A rate of `hz` ticks per second, from the base rate up to
    /// [`Self::MAX_HZ`]. Slower than the base rate is not supported: a
    /// base-rate tick would then have to cover more than one step.
    pub const fn new(hz: u32) -> Option<TickRate> {
        if hz >= crate::TICKS_PER_SECOND && hz <= Self::MAX_HZ {
            Some(TickRate { hz })
        } else {
            None
        }
    }

    /// Ticks per second.
    pub const fn hz(self) -> u32 {
        self.hz
    }

    /// Whether this is the base rate, where every conversion is the
    /// identity.
    pub const fn is_base(self) -> bool {
        self.hz == crate::TICKS_PER_SECOND
    }

    /// This tick's share of an amount stated per base-rate tick (a speed
    /// or a drain from the tables). `tick` is the game frame.
    ///
    /// Over any `hz` consecutive ticks the shares add up to exactly one
    /// second's worth at the base rate (`v · 25`): tick `i = tick mod hz`
    /// gets `⌊v·25·(i+1)/hz⌋ − ⌊v·25·i/hz⌋`. Nothing is lost to rounding,
    /// a small amount is spread over the second instead of rounding to 0,
    /// and the result depends only on the frame, so it is deterministic.
    /// Identity at the base rate.
    pub fn share(self, per_base_tick: i32, tick: i32) -> i32 {
        if self.is_base() {
            return per_base_tick;
        }
        let hz = i64::from(self.hz);
        let i = i64::from(tick).rem_euclid(hz);
        let per_second = i64::from(per_base_tick) * i64::from(crate::TICKS_PER_SECOND);
        let upto = |k: i64| (per_second * k).div_euclid(hz);
        (upto(i + 1) - upto(i)) as i32
    }

    /// The amount `ticks` ticks at this rate add up to, for an amount stated
    /// per base-rate tick (`v · ticks · 25 / hz`, rounded down): the closed
    /// form of [`Self::share`] summed from a start, for a clock that reads
    /// "ticks since" instead of stepping (a drawn animation frame).
    /// Identity at the base rate.
    pub fn progress(self, per_base_tick: u64, ticks: u64) -> u64 {
        let v = u128::from(per_base_tick) * u128::from(ticks);
        if self.is_base() {
            return v as u64;
        }
        (v * u128::from(crate::TICKS_PER_SECOND) / u128::from(self.hz)) as u64
    }

    /// A duration of `base_ticks` base-rate ticks as a tick count at this
    /// rate (`n · hz / 25`), rounded to the nearest tick. A non-zero
    /// duration never rounds to 0 ticks, so a timer always runs at least
    /// one tick.
    pub fn ticks(self, base_ticks: i32) -> i32 {
        if self.is_base() || base_ticks == 0 {
            return base_ticks;
        }
        let base = i64::from(crate::TICKS_PER_SECOND);
        let n = i64::from(base_ticks) * i64::from(self.hz);
        let rounded = (n.abs() + base / 2) / base;
        let t = rounded.max(1).min(i64::from(i32::MAX)) as i32;
        if base_ticks < 0 {
            -t
        } else {
            t
        }
    }

    /// The inverse of [`Self::ticks`]: `ticks` ticks at this rate as a
    /// count of base ticks (`n · 25 / hz`), rounded to the nearest. For
    /// rules that multiply a per-base-tick amount by a remaining time.
    /// Identity at the base rate.
    pub fn base_ticks(self, ticks: i32) -> i32 {
        if self.is_base() {
            return ticks;
        }
        let hz = i64::from(self.hz);
        let n = i64::from(ticks) * i64::from(crate::TICKS_PER_SECOND);
        let r = (n.abs() + hz / 2) / hz;
        (if ticks < 0 { -r } else { r }) as i32
    }

    /// A real-time duration in milliseconds as a tick count at this rate,
    /// rounded to the nearest tick (at least 1 for a non-zero duration).
    pub fn ticks_for_ms(self, ms: u32) -> u32 {
        if ms == 0 {
            return 0;
        }
        let t = (u64::from(ms) * u64::from(self.hz) + 500) / 1000;
        t.clamp(1, u64::from(u32::MAX)) as u32
    }

    /// The real-time length of `ticks` ticks at this rate in milliseconds,
    /// rounded down.
    pub fn ms_for_ticks(self, ticks: u32) -> u64 {
        u64::from(ticks) * 1000 / u64::from(self.hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUES: [i32; 9] = [0, 1, -1, 5, 0x600, 0x900, 0x1000, -0x6000, i32::MAX];

    #[test]
    fn base_rate_is_25_hz_and_the_default() {
        assert_eq!(TickRate::BASE.hz(), 25);
        assert_eq!(TickRate::default(), TickRate::BASE);
        assert_eq!(TickRate::new(25), Some(TickRate::BASE));
        assert!(TickRate::BASE.is_base());
    }

    #[test]
    fn rates_outside_25_to_1000_are_rejected() {
        assert_eq!(TickRate::new(0), None);
        assert_eq!(TickRate::new(24), None);
        assert_eq!(TickRate::new(1001), None);
        assert_eq!(TickRate::new(1000).map(TickRate::hz), Some(1000));
    }

    #[test]
    fn every_conversion_is_the_identity_at_the_base_rate() {
        let r = TickRate::BASE;
        for v in VALUES {
            for tick in [0, 1, 24, 25, -1, i32::MAX] {
                assert_eq!(r.share(v, tick), v);
            }
            assert_eq!(r.ticks(v), v);
            assert_eq!(r.base_ticks(v), v);
        }
        assert_eq!(r.ticks_for_ms(40), 1);
        assert_eq!(r.ticks_for_ms(1000), 25);
        assert_eq!(r.ms_for_ticks(25), 1000);
    }

    #[test]
    fn faster_rates_keep_real_time_speed() {
        let r = TickRate::new(100).unwrap();
        // Walk speed 0x600 per 40 ms is 0x180 per 10 ms.
        assert_eq!(r.share(0x600, 7), 0x180);
        assert_eq!(r.share(-0x600, 7), -0x180);
        // 5 base ticks (200 ms) are 20 ticks at 100 Hz.
        assert_eq!(r.ticks(5), 20);
        assert_eq!(r.ticks_for_ms(200), 20);
        assert_eq!(r.ms_for_ticks(20), 200);
        // Large values do not overflow.
        assert_eq!(TickRate::new(1000).unwrap().ticks(i32::MAX), i32::MAX);
        let big: i64 = (0..1000)
            .map(|t| i64::from(TickRate::new(1000).unwrap().share(i32::MAX, t)))
            .sum();
        assert_eq!(big, i64::from(i32::MAX) * 25);
    }

    #[test]
    fn shares_add_up_over_any_second() {
        for hz in [30, 60, 100, 144, 1000] {
            let r = TickRate::new(hz).unwrap();
            for v in [1, 3, 0x600, 1080, -5] {
                // Any window of `hz` frames, including one across 0.
                for start in [0, 17, -40, 1_000_003] {
                    let sum: i64 = (0..hz as i32)
                        .map(|k| i64::from(r.share(v, start + k)))
                        .sum();
                    assert_eq!(sum, i64::from(v) * 25, "{hz} Hz, {v}, from {start}");
                }
            }
        }
        // A drain of 1 per base tick at 60 Hz: 25 ticks of 1, 35 of 0.
        let r = TickRate::new(60).unwrap();
        assert_eq!((0..60).filter(|&t| r.share(1, t) == 1).count(), 25);
    }

    #[test]
    fn progress_is_the_summed_share_from_tick_zero() {
        assert_eq!(TickRate::BASE.progress(213, 33), 213 * 33);
        for hz in [25, 30, 60, 144, 1000] {
            let r = TickRate::new(hz).unwrap();
            for v in [0u64, 1, 151, 213, 0x600] {
                let mut sum = 0u64;
                for t in 0..3 * u64::from(hz) {
                    assert_eq!(r.progress(v, t), sum, "{hz} Hz, {v}, tick {t}");
                    sum += r.share(v as i32, t as i32) as u64;
                }
                // One second is 25 base ticks' worth at every rate.
                assert_eq!(r.progress(v, u64::from(hz)), v * 25);
            }
        }
        // No overflow on a long session.
        let r = TickRate::new(60).unwrap();
        assert_eq!(
            r.progress(u64::from(u32::MAX), 60),
            u64::from(u32::MAX) * 25
        );
    }

    #[test]
    fn rounding_rules() {
        let r = TickRate::new(30).unwrap();
        // 7 · 25 / 30 = 5.83: shares of 5 and 6 that make 175 a second.
        let shares: Vec<i32> = (0..30).map(|t| r.share(7, t)).collect();
        assert!(shares.iter().all(|&s| s == 5 || s == 6));
        assert_eq!(shares.iter().sum::<i32>(), 7 * 25);
        // 3 · 30 / 25 = 3.6, nearest.
        assert_eq!(r.ticks(3), 4);
        assert_eq!(r.ticks(-3), -4);
        // The inverse: 60 ticks at 60 Hz are 25 base ticks; 7 are 2.9 → 3.
        let r60 = TickRate::new(60).unwrap();
        assert_eq!((r60.base_ticks(60), r60.base_ticks(7)), (25, 3));
        assert_eq!(r60.base_ticks(-7), -3);
        // A short non-zero duration keeps at least one tick.
        assert_eq!(r.ticks_for_ms(1), 1);
        assert_eq!(r.ticks_for_ms(0), 0);
    }
}
