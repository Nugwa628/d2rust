//! Shared support for the `prop_*` property tests (proptest): a
//! wall-clock bound per call, the proptest config, and a small seeded
//! generator for bulk inputs (tables of records) that proptest would
//! shrink slowly. Mirrors `d2-data`'s crate-private `robust.rs`.

#![allow(dead_code)]

use std::sync::mpsc;
use std::time::Duration;

use proptest::test_runner::Config;

/// Wall-clock limit for one call in a property test (debug build).
pub const DEADLINE: Duration = Duration::from_secs(20);

/// Runs `f` on its own thread; panics (failing the case) if `f` panics or
/// does not return within [`DEADLINE`].
pub fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(DEADLINE) {
        Ok(v) => {
            let _ = handle.join();
            v
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("call did not return within {DEADLINE:?}"),
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(p) => std::panic::resume_unwind(p),
            Ok(()) => unreachable!("sender dropped without sending"),
        },
    }
}

/// `PROPTEST_CASES` from the environment, else `default`; no failure
/// persistence files in the source tree.
pub fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

/// SplitMix64: a seeded generator for bulk test inputs (the seed is the
/// proptest value, so a failure reports it).
pub struct Gen(u64);

impl Gen {
    pub fn new(seed: u64) -> Gen {
        Gen(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// True with probability `1 / n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    /// A u32 biased to boundary values.
    pub fn interesting_u32(&mut self) -> u32 {
        let (any, small) = (self.next() as u32, self.below(64) as u32);
        *self.pick(&[
            0,
            1,
            2,
            0x7F,
            0x80,
            0xFF,
            0xFFFF,
            0x1_0000,
            0x7FFF_FFFF,
            0x8000_0000,
            u32::MAX - 1,
            u32::MAX,
            any,
            small,
        ])
    }

    /// A record of `size` bytes: all random, or zeros with a few
    /// boundary values written at random offsets (reaches the code past
    /// early refusals that all-random bytes trigger).
    pub fn record(&mut self, size: usize) -> Vec<u8> {
        self.record_in(size, true)
    }

    /// [`Gen::record`]; with `nasty` false, every written byte is below
    /// 0x40 (no name byte refusals, no negative links), so a whole set of
    /// tables gets through the early checks to the later stages.
    pub fn record_in(&mut self, size: usize, nasty: bool) -> Vec<u8> {
        if !nasty {
            let mut r = vec![0u8; size];
            for _ in 0..(if size == 0 { 0 } else { self.below(6) }) {
                let at = self.below(size);
                r[at] = self.below(0x40) as u8;
            }
            return r;
        }
        if self.one_in(3) {
            return (0..size).map(|_| self.next() as u8).collect();
        }
        let mut r = vec![0u8; size];
        if size == 0 {
            return r;
        }
        for _ in 0..self.below(12) {
            let at = self.below(size);
            match self.below(3) {
                0 => r[at] = self.next() as u8,
                1 => r[at] = *self.pick(&[0x80, 0xFF, 0x7F, 1, 2, 7, 8]),
                _ => {
                    for (i, b) in self.interesting_u32().to_le_bytes().into_iter().enumerate() {
                        if let Some(s) = r.get_mut(at + i) {
                            *s = b;
                        }
                    }
                }
            }
        }
        r
    }
}
