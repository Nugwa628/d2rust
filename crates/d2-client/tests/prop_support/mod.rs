//! Shared support of the `prop_*` property tests: the proptest config and
//! a deadline runner. Same shape as `d2-formats`' `robust` module:
//! every untrusted input is an error or a value, never a panic, a hang or
//! an unbounded allocation.
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder (it overrides every default).

#![allow(dead_code)]

use std::sync::mpsc;
use std::time::Duration;

use proptest::test_runner::Config;

/// Wall-clock limit for one property case body (debug build).
pub const DEADLINE: Duration = Duration::from_secs(20);

/// Proptest config with `default` cases, or `PROPTEST_CASES` when set.
/// (`ProptestConfig::with_cases` would ignore the env var.)
pub fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        // Minimized failures become `regress_*` tests instead.
        failure_persistence: None,
        ..Config::default()
    }
}

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
