//! Test support for the robustness property tests in `d2-data` (a copy of
//! `d2-formats`'s `robust.rs`, which is test-only and not exported). The rule the
//! tests check: every malformed input is an error, never a panic, a hang or an
//! unbounded allocation. Each parser's property tests use [`bounded`] and
//! [`mutated`].
//!
//! `proptest` (dev-dependency) generates and shrinks the inputs. Its own
//! `timeout` option needs the `fork` feature, so [`bounded`] runs the call
//! on a thread with a deadline instead.

use std::sync::mpsc;
use std::time::Duration;

use proptest::prelude::*;

/// Wall-clock limit for one parser call in a property test (debug build).
pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

/// Runs `f` on its own thread; panics (failing the case) if `f` panics or
/// does not return within [`DEADLINE`].
pub(crate) fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(DEADLINE) {
        Ok(v) => {
            let _ = handle.join();
            v
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("parser did not return within {DEADLINE:?}"),
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(p) => std::panic::resume_unwind(p),
            Ok(()) => unreachable!("sender dropped without sending"),
        },
    }
}

/// One edit applied to a valid file.
#[derive(Debug, Clone)]
pub(crate) enum Mutation {
    /// Flip bit `bit % 8` of byte `at % len`.
    FlipBit { at: usize, bit: u8 },
    /// Overwrite byte `at % len`.
    SetByte { at: usize, value: u8 },
    /// Overwrite the little-endian u32 at `at % len` (clipped at the end);
    /// models extended lengths, counts and offsets.
    SetU32 { at: usize, value: u32 },
    /// Cut the file to `len % (len + 1)` bytes.
    Truncate { len: usize },
    /// Insert bytes at `at % (len + 1)`.
    Insert { at: usize, bytes: Vec<u8> },
}

fn interesting_u32() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0u32),
        Just(1),
        Just(0x7fff_ffff),
        Just(0x8000_0000),
        Just(u32::MAX),
        Just(u32::MAX - 1),
        Just(0xffff),
        Just(0x1_0000),
        0u32..0x100,
        any::<u32>(),
    ]
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        (any::<usize>(), any::<u8>()).prop_map(|(at, bit)| Mutation::FlipBit { at, bit }),
        (any::<usize>(), any::<u8>()).prop_map(|(at, value)| Mutation::SetByte { at, value }),
        (any::<usize>(), interesting_u32()).prop_map(|(at, value)| Mutation::SetU32 { at, value }),
        any::<usize>().prop_map(|len| Mutation::Truncate { len }),
        (any::<usize>(), prop::collection::vec(any::<u8>(), 1..16))
            .prop_map(|(at, bytes)| Mutation::Insert { at, bytes }),
    ]
}

/// Applies `m` to `data`.
pub(crate) fn apply(data: &mut Vec<u8>, m: &Mutation) {
    let len = data.len();
    match m {
        Mutation::FlipBit { at, bit } if len > 0 => data[at % len] ^= 1 << (bit % 8),
        Mutation::SetByte { at, value } if len > 0 => data[at % len] = *value,
        Mutation::SetU32 { at, value } if len > 0 => {
            let at = at % len;
            for (i, b) in value.to_le_bytes().into_iter().enumerate() {
                if let Some(slot) = data.get_mut(at + i) {
                    *slot = b;
                }
            }
        }
        Mutation::Truncate { len: n } => data.truncate(n % (len + 1)),
        Mutation::Insert { at, bytes } => {
            let at = at % (len + 1);
            data.splice(at..at, bytes.iter().copied());
        }
        _ => {}
    }
}

/// `valid` with 1–4 random mutations applied.
pub(crate) fn mutated(valid: Vec<u8>) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(mutation(), 1..5).prop_map(move |ms| {
        let mut data = valid.clone();
        for m in &ms {
            apply(&mut data, m);
        }
        data
    })
}

/// Property-test config: `PROPTEST_CASES` from the environment, else
/// `default` (`ProptestConfig::with_cases` would override the variable).
/// No failure persistence files are written into the source tree.
pub(crate) fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Byte strings up to `max_parts` pieces long, each piece drawn from
/// `parts`: inputs biased to a text format's alphabet and keywords.
pub(crate) fn text_of(
    parts: &'static [&'static [u8]],
    max_parts: usize,
) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop::sample::select(parts), 0..max_parts).prop_map(|ps| ps.concat())
}

/// Arbitrary bytes up to `max` long.
pub(crate) fn bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reports_panic_and_hang() {
        // The harness fails on a panic and on a hang.
        assert!(std::panic::catch_unwind(|| bounded(|| panic!("boom"))).is_err());
        assert_eq!(bounded(|| 7), 7);
    }

    #[test]
    fn apply_edits() {
        let mut d = vec![0u8; 4];
        apply(
            &mut d,
            &Mutation::SetU32 {
                at: 2,
                value: 0x0403_0201,
            },
        );
        assert_eq!(d, [0, 0, 1, 2]);
        apply(&mut d, &Mutation::Truncate { len: 7 });
        assert_eq!(d, [0, 0]);
    }
}
