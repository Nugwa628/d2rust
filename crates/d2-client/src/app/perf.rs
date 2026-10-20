//! Frame and tick timing for the perf report, on only when `D2_PERF_OUT`
//! names a file.
//!
//! Per frame of the Bevy app: `First` start → after `PreUpdate` (the
//! bridge pump: the server's drain, tick and flush run inside it) → after
//! `Update` (client update: model, UI, world view packing) → after `Last`
//! (post-update and last), with the frame's start on the report clock
//! ([`d2_server::perf::epoch`]) and whether it produced a new picture: a
//! new tick picture ([`ComposeJob`]) or a new between-tick picture
//! ([`MotionJob`]). The jobs are only read here, so timing never changes
//! what is drawn. A change of the atlas pages is marked as an `atlas`
//! event. Per render-world frame: the `Render` schedule from
//! `ExtractCommands` to `PostCleanup` (prepare, the compositor node,
//! submit, present). Per server tick and event span: [`d2_server::perf`].
//! At exit [`write_report`] writes everything plus the peak RSS as one
//! `perf 2` JSON file with its summary ([`super::perf_summary`]).
//! Wall-clock time only reaches this report, never the game.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use bevy::app::MainScheduleOrder;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};

use super::perf_summary::{self, Frame, Report, Span, Tick, DREW, MOVED};
use crate::world_view::node::{ComposeJob, MotionJob};

/// The report format line.
pub const FORMAT: &str = "perf 2";

static ON: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Log {
    frames: Vec<Frame>,
    render: Vec<u32>,
    /// Headless (`state-dump`) bridge frames that ran a tick: the whole
    /// `Bridge::frame` (server frame, receive, client model), µs.
    bridge: Vec<u32>,
    start: Option<Instant>,
    marks: [Option<Instant>; 3],
    /// [`DREW`] / [`MOVED`] of the frame in progress.
    flags: u8,
    render_start: Option<Instant>,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    frames: Vec::new(),
    render: Vec::new(),
    bridge: Vec::new(),
    start: None,
    marks: [None; 3],
    flags: 0,
    render_start: None,
});

/// Turns timing on when `D2_PERF_OUT` is set (call first in `main`).
pub fn enable_from_env() {
    if std::env::var_os("D2_PERF_OUT").is_some_and(|v| !v.is_empty()) {
        ON.store(true, Ordering::Relaxed);
        d2_server::perf::enable();
    }
}

/// Whether timing is on.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

#[derive(ScheduleLabel, Debug, Hash, PartialEq, Eq, Clone)]
struct AfterPreUpdate;
#[derive(ScheduleLabel, Debug, Hash, PartialEq, Eq, Clone)]
struct AfterUpdate;
#[derive(ScheduleLabel, Debug, Hash, PartialEq, Eq, Clone)]
struct AfterLast;

fn us(d: std::time::Duration) -> u32 {
    u32::try_from(d.as_micros()).unwrap_or(u32::MAX)
}

fn frame_start() {
    let now = Instant::now();
    let Ok(mut log) = LOG.lock() else { return };
    if let (Some(s), [Some(a), Some(b), Some(c)]) = (log.start, log.marks) {
        let t = Frame {
            interval: us(now - s),
            pre: us(a - s),
            update: us(b - a),
            post: us(c - b),
            start_us: d2_server::perf::at_us(s),
            flags: Some(log.flags),
        };
        log.frames.push(t);
    }
    log.start = Some(now);
    log.marks = [None; 3];
    log.flags = 0;
}

fn mark<const I: usize>() {
    if let Ok(mut log) = LOG.lock() {
        log.marks[I] = Some(Instant::now());
    }
}

/// The last seen compose and motion job numbers and atlas pages version.
#[derive(Default)]
struct Seen {
    compose: Option<u64>,
    motion: Option<u64>,
    pages: Option<u64>,
}

/// Notes the frame's new pictures and atlas changes (read-only on the
/// jobs).
fn pictures(
    compose: Option<Res<ComposeJob>>,
    motion: Option<Res<MotionJob>>,
    mut seen: Local<Seen>,
) {
    let mut flags = 0;
    if let Some(job) = compose {
        if seen.compose != Some(job.seq) {
            seen.compose = Some(job.seq);
            flags |= DREW;
        }
        if seen.pages.is_some_and(|v| v != job.pages_version) {
            d2_server::perf::mark("atlas");
        }
        seen.pages = Some(job.pages_version);
    }
    if let Some(job) = motion {
        if seen.motion != Some(job.seq) {
            seen.motion = Some(job.seq);
            flags |= MOVED;
        }
    }
    if flags != 0 {
        if let Ok(mut log) = LOG.lock() {
            log.flags |= flags;
        }
    }
}

fn render_start() {
    if let Ok(mut log) = LOG.lock() {
        log.render_start = Some(Instant::now());
    }
}

fn render_end() {
    if let Ok(mut log) = LOG.lock() {
        if let Some(s) = log.render_start.take() {
            log.render.push(us(s.elapsed()));
        }
    }
}

/// Headless: one `Bridge::frame` that started at `t0` (kept when it ticked).
pub fn record_bridge_frame(t0: Instant, ticked: bool) {
    if !ticked {
        return;
    }
    if let Ok(mut log) = LOG.lock() {
        log.bridge.push(us(t0.elapsed()));
    }
}

/// Adds the timing systems to `app` when timing is on.
pub fn add(app: &mut App) {
    if !enabled() {
        return;
    }
    app.init_schedule(AfterPreUpdate)
        .init_schedule(AfterUpdate)
        .init_schedule(AfterLast);
    {
        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_after(PreUpdate, AfterPreUpdate);
        order.insert_after(Update, AfterUpdate);
        order.insert_after(Last, AfterLast);
    }
    app.add_systems(First, frame_start)
        .add_systems(AfterPreUpdate, mark::<0>)
        .add_systems(AfterUpdate, mark::<1>)
        .add_systems(Last, pictures)
        .add_systems(AfterLast, mark::<2>);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .add_systems(Render, render_start.in_set(RenderSystems::ExtractCommands))
            .add_systems(Render, render_end.in_set(RenderSystems::PostCleanup));
    }
}

/// Peak resident set size in KiB (`VmHWM`), Linux only.
fn peak_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Writes the report to `D2_PERF_OUT` (when timing is on).
pub fn write_report() {
    if !enabled() {
        return;
    }
    let Some(path) = std::env::var_os("D2_PERF_OUT") else {
        return;
    };
    let log = LOG
        .lock()
        .map(|mut l| std::mem::take(&mut *l))
        .unwrap_or_default();
    let report = Report {
        version: 2,
        peak_rss_kib: peak_rss_kib().unwrap_or(0),
        ticks: d2_server::perf::take()
            .into_iter()
            .map(|t| Tick {
                frame: t.frame,
                drain_us: t.drain_us,
                tick_us: t.tick_us,
                flush_us: t.flush_us,
                at_us: Some(t.at_us),
            })
            .collect(),
        frames: log.frames,
        render: log.render,
        bridge: log.bridge,
        spans: d2_server::perf::take_spans()
            .into_iter()
            .map(|s| Span {
                kind: s.kind.to_owned(),
                start_us: s.start_us,
                dur_us: s.dur_us,
            })
            .collect(),
    };
    let text = report.to_json(perf_summary::HITCH_MS_DEFAULT);
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("perf: {}: {e}", std::path::Path::new(&path).display());
    }
}
