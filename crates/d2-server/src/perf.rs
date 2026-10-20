//! Server timing and event spans for the perf report (off unless [`enable`]
//! is called).
//!
//! [`crate::host::Host::frame`] measures the drain and, when a tick runs,
//! the tick plus its flush with the wall clock and appends one
//! [`TickTime`] here. Slow one-off work (a save write, a room build, an
//! atlas change, a settings write) records a [`Span`] through [`span`] or
//! [`mark`], so the perf summary can name what was running during a slow
//! client frame. Every time is on one clock: microseconds since [`epoch`],
//! which the client's frame log shares. The times are only read by the
//! perf report; they never reach the game (the sim stays clock-free,
//! CLAUDE.md rule 5).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// One server frame that ran a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickTime {
    /// The game frame after the tick.
    pub frame: u32,
    /// Drain (C→S messages) in microseconds.
    pub drain_us: u32,
    /// The tick (`Tick::tick`) in microseconds.
    pub tick_us: u32,
    /// The flush after the tick in microseconds.
    pub flush_us: u32,
    /// When the drain started, microseconds since [`epoch`].
    pub at_us: u64,
}

/// One timed piece of one-off work, on the [`epoch`] clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// What ran: `save`, `room`, `atlas`, `settings`, `audio`.
    pub kind: &'static str,
    /// Start, microseconds since [`epoch`].
    pub start_us: u64,
    /// Length in microseconds (0 for a [`mark`]).
    pub dur_us: u32,
}

static ON: AtomicBool = AtomicBool::new(false);
static TIMES: Mutex<Vec<TickTime>> = Mutex::new(Vec::new());
static SPANS: Mutex<Vec<Span>> = Mutex::new(Vec::new());
static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Starts recording (process-wide) and fixes the [`epoch`].
pub fn enable() {
    epoch();
    ON.store(true, Ordering::Relaxed);
}

/// Whether times are recorded.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The instant every report time counts from (the first call fixes it).
pub fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

/// `t` in microseconds since [`epoch`] (0 for an instant before it).
pub fn at_us(t: Instant) -> u64 {
    u64::try_from(t.saturating_duration_since(epoch()).as_micros()).unwrap_or(u64::MAX)
}

pub(crate) fn record(t: TickTime) {
    if let Ok(mut v) = TIMES.lock() {
        v.push(t);
    }
}

/// Takes the recorded tick times.
pub fn take() -> Vec<TickTime> {
    TIMES
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// Takes the recorded spans.
pub fn take_spans() -> Vec<Span> {
    SPANS
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

fn push_span(kind: &'static str, start: Instant, dur_us: u32) {
    if let Ok(mut v) = SPANS.lock() {
        v.push(Span {
            kind,
            start_us: at_us(start),
            dur_us,
        });
    }
}

/// Times the work until the returned guard drops, as a `kind` span. Does
/// nothing while recording is off.
#[must_use = "the span ends when the guard drops"]
pub fn span(kind: &'static str) -> SpanGuard {
    SpanGuard(enabled().then(|| (kind, Instant::now())))
}

/// Records a `kind` span from `start` to now, for work that only knows
/// afterwards whether it was worth naming. Does nothing while recording
/// is off.
pub fn span_since(kind: &'static str, start: Instant) {
    if enabled() {
        push_span(kind, start, us_since(start));
    }
}

/// Records a `kind` event of no length at this moment (work done
/// elsewhere, such as on the GPU, that can't be timed from here).
pub fn mark(kind: &'static str) {
    if enabled() {
        push_span(kind, Instant::now(), 0);
    }
}

/// An open [`span`]; recorded when dropped.
pub struct SpanGuard(Option<(&'static str, Instant)>);

impl Drop for SpanGuard {
    fn drop(&mut self) {
        if let Some((kind, start)) = self.0.take() {
            push_span(kind, start, us_since(start));
        }
    }
}

/// Microseconds since `t`, saturated to u32.
pub(crate) fn us_since(t: Instant) -> u32 {
    u32::try_from(t.elapsed().as_micros()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_record_only_when_enabled_and_share_the_epoch() {
        // Off: a guard records nothing.
        drop(span("room"));
        mark("atlas");
        assert!(take_spans().is_empty());
        enable();
        let before = at_us(Instant::now());
        drop(span("save"));
        mark("atlas");
        let spans = take_spans();
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].kind, spans[1].kind), ("save", "atlas"));
        assert!(spans.iter().all(|s| s.start_us >= before));
        assert_eq!(spans[1].dur_us, 0);
        assert!(take_spans().is_empty());
    }
}
