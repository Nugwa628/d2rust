//! The perf report as data, and its summary: what a lane reads after a
//! timed run instead of writing its own analyser.
//!
//! A run with `D2_PERF_OUT=FILE` writes a report ([`super::perf`]); this
//! module reads it back (`perf 2`, and the older `perf 1` that has no
//! clock offsets, picture flags or spans), and sums it up:
//! - rates: frames, server ticks, drawn ticks (new tick pictures) and
//!   picture changes (tick or between-tick motion pictures) per second;
//! - p50/p95/p99/max per phase: the client frame and its parts, the render
//!   world, the server drain/tick/flush;
//! - hitches: every client frame longer than the hitch limit, with the
//!   phase that took the most time and the events that ran during it
//!   (room loads, save writes, atlas changes, settings writes, audio stream loads);
//! - a verdict against budgets (`tick=40,frame=17,hitch=0`).
//!
//! `perf-summary FILE` prints it ([`run_cli`]); the writer embeds the same
//! summary in every `perf 2` report. Timing on a shared PC is noisy, so
//! budgets are a lane's verdict, not a CI gate.

use std::fmt::Write as _;

use serde_json::{json, Value};

/// Frames longer than this many milliseconds are hitches by default.
pub const HITCH_MS_DEFAULT: u32 = 50;
/// Hitches listed in full; the count covers all of them.
const HITCH_LIST_MAX: usize = 100;

/// Frame flag: a new tick picture was composed this frame.
pub const DREW: u8 = 1;
/// Frame flag: a new between-tick (motion) picture was composed this frame.
pub const MOVED: u8 = 2;

/// One server frame that ran a tick, microseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    pub frame: u32,
    pub drain_us: u32,
    pub tick_us: u32,
    pub flush_us: u32,
    /// Drain start on the report clock (`None` in `perf 1`).
    pub at_us: Option<u64>,
}

impl Tick {
    fn total(&self) -> u32 {
        self.drain_us
            .saturating_add(self.tick_us)
            .saturating_add(self.flush_us)
    }
}

/// One client (main-world) frame, microseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// Start to the next frame's start.
    pub interval: u32,
    /// `First` + `PreUpdate` (the bridge pump with the server frame).
    pub pre: u32,
    /// The client update.
    pub update: u32,
    /// `PostUpdate` + `Last`.
    pub post: u32,
    /// Start on the report clock (`perf 1`: the sum of the earlier
    /// intervals, so only relative).
    pub start_us: u64,
    /// [`DREW`] / [`MOVED`] bits (`None` in `perf 1`).
    pub flags: Option<u8>,
}

impl Frame {
    /// Time outside the main schedules: waiting on the render world,
    /// presenting, frame pacing.
    fn wait(&self) -> u32 {
        self.interval
            .saturating_sub(self.pre)
            .saturating_sub(self.update)
            .saturating_sub(self.post)
    }
}

/// One event that ran (`save`, `room`, `atlas`, `settings`, `audio`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub kind: String,
    pub start_us: u64,
    /// 0 for an event that is only a moment (an atlas change).
    pub dur_us: u32,
}

/// A whole perf report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The format number read (1 or 2).
    pub version: u8,
    pub peak_rss_kib: u64,
    pub ticks: Vec<Tick>,
    pub frames: Vec<Frame>,
    /// Render-world frames, microseconds each.
    pub render: Vec<u32>,
    /// Headless bridge frames that ticked, microseconds each.
    pub bridge: Vec<u32>,
    pub spans: Vec<Span>,
}

/// Why a report can't be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unknown format {0:?} (expected \"perf 1\" or \"perf 2\")")]
    Format(String),
    #[error("bad {0} entry")]
    Entry(&'static str),
}

fn uint(v: &Value, what: &'static str) -> Result<u64, ReadError> {
    v.as_u64().ok_or(ReadError::Entry(what))
}

fn u32_of(v: &Value, what: &'static str) -> Result<u32, ReadError> {
    u32::try_from(uint(v, what)?).map_err(|_| ReadError::Entry(what))
}

fn rows<'a>(d: &'a Value, key: &str) -> &'a [Value] {
    d.get(key)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

/// A row of at least `n` numbers.
fn row<'a>(v: &'a Value, n: usize, what: &'static str) -> Result<&'a [Value], ReadError> {
    match v.as_array() {
        Some(a) if a.len() >= n => Ok(a),
        _ => Err(ReadError::Entry(what)),
    }
}

impl Report {
    /// Reads a `perf 1` or `perf 2` report.
    pub fn parse(text: &str) -> Result<Self, ReadError> {
        let d: Value = serde_json::from_str(text)?;
        let format = d.get("format").and_then(Value::as_str).unwrap_or("");
        let version = match format {
            "perf 1" => 1,
            "perf 2" => 2,
            f => return Err(ReadError::Format(f.to_owned())),
        };
        let v2 = version >= 2;
        let mut r = Report {
            version,
            peak_rss_kib: d.get("peak_rss_kib").and_then(Value::as_u64).unwrap_or(0),
            ..Report::default()
        };
        for t in rows(&d, "ticks") {
            let a = row(t, if v2 { 5 } else { 4 }, "ticks")?;
            r.ticks.push(Tick {
                frame: u32_of(&a[0], "ticks")?,
                drain_us: u32_of(&a[1], "ticks")?,
                tick_us: u32_of(&a[2], "ticks")?,
                flush_us: u32_of(&a[3], "ticks")?,
                at_us: if v2 {
                    Some(uint(&a[4], "ticks")?)
                } else {
                    None
                },
            });
        }
        let mut clock = 0u64;
        for f in rows(&d, "frames") {
            let a = row(f, if v2 { 6 } else { 4 }, "frames")?;
            let interval = u32_of(&a[0], "frames")?;
            let (start_us, flags) = if v2 {
                let flags =
                    u8::try_from(uint(&a[5], "frames")?).map_err(|_| ReadError::Entry("frames"))?;
                (uint(&a[4], "frames")?, Some(flags))
            } else {
                (clock, None)
            };
            clock = start_us + u64::from(interval);
            r.frames.push(Frame {
                interval,
                pre: u32_of(&a[1], "frames")?,
                update: u32_of(&a[2], "frames")?,
                post: u32_of(&a[3], "frames")?,
                start_us,
                flags,
            });
        }
        for v in rows(&d, "render") {
            r.render.push(u32_of(v, "render")?);
        }
        for v in rows(&d, "bridge") {
            r.bridge.push(u32_of(v, "bridge")?);
        }
        for s in rows(&d, "spans") {
            let a = row(s, 3, "spans")?;
            r.spans.push(Span {
                kind: a[0].as_str().ok_or(ReadError::Entry("spans"))?.to_owned(),
                start_us: uint(&a[1], "spans")?,
                dur_us: u32_of(&a[2], "spans")?,
            });
        }
        Ok(r)
    }

    /// The report as `perf 2` JSON, with its [`Summary`] at `hitch_ms`.
    pub fn to_json(&self, hitch_ms: u32) -> String {
        let list = |v: Vec<String>| v.join(",");
        let ticks = list(
            self.ticks
                .iter()
                .map(|t| {
                    format!(
                        "[{},{},{},{},{}]",
                        t.frame,
                        t.drain_us,
                        t.tick_us,
                        t.flush_us,
                        t.at_us.unwrap_or(0)
                    )
                })
                .collect(),
        );
        let frames = list(
            self.frames
                .iter()
                .map(|f| {
                    format!(
                        "[{},{},{},{},{},{}]",
                        f.interval,
                        f.pre,
                        f.update,
                        f.post,
                        f.start_us,
                        f.flags.unwrap_or(0)
                    )
                })
                .collect(),
        );
        let spans = list(
            self.spans
                .iter()
                .map(|s| json!([s.kind, s.start_us, s.dur_us]).to_string())
                .collect(),
        );
        let nums = |v: &[u32]| list(v.iter().map(u32::to_string).collect());
        format!(
            "{{\"format\":\"perf 2\",\"peak_rss_kib\":{},\n\"summary\":{},\n\"ticks\":[{ticks}],\n\"frames\":[{frames}],\n\"render\":[{}],\n\"bridge\":[{}],\n\"spans\":[{spans}]}}\n",
            self.peak_rss_kib,
            Summary::of(self, hitch_ms).to_json(None),
            nums(&self.render),
            nums(&self.bridge),
        )
    }
}

/// Nearest-rank percentiles of a list of microsecond times.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub n: usize,
    pub p50: u32,
    pub p95: u32,
    pub p99: u32,
    pub max: u32,
}

impl Stat {
    pub fn of(values: impl IntoIterator<Item = u32>) -> Self {
        let mut v: Vec<u32> = values.into_iter().collect();
        v.sort_unstable();
        let pct = |p: usize| {
            if v.is_empty() {
                return 0;
            }
            let k = (p * v.len()).div_ceil(100).clamp(1, v.len()) - 1;
            v[k]
        };
        Stat {
            n: v.len(),
            p50: pct(50),
            p95: pct(95),
            p99: pct(99),
            max: v.last().copied().unwrap_or(0),
        }
    }
}

/// Events of one kind during a hitch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cause {
    pub kind: String,
    pub count: usize,
    /// Their summed length, microseconds.
    pub us: u64,
}

/// One client frame over the hitch limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hitch {
    /// Index in the report's frames.
    pub frame: usize,
    pub start_us: u64,
    pub us: u32,
    /// The longest part: `pre`, `update`, `post` or `wait`.
    pub phase: &'static str,
    pub phase_us: u32,
    /// Server drain + tick + flush that started in the frame (`None` when
    /// the report has no tick clock).
    pub server_us: Option<u32>,
    /// Events that overlapped the frame, by kind in first-seen order.
    pub causes: Vec<Cause>,
}

/// The summary of a report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub hitch_ms: u32,
    /// Run length: the client frames' time, or the server ticks' span
    /// headless.
    pub seconds: Option<f64>,
    pub frames: usize,
    pub ticks: usize,
    pub fps: Option<f64>,
    pub ticks_per_s: Option<f64>,
    /// New tick pictures per second (`None` without frame flags).
    pub drawn_per_s: Option<f64>,
    /// New pictures (tick or motion) per second.
    pub pictures_per_s: Option<f64>,
    /// `(name, stat)` for each phase with samples.
    pub phases: Vec<(&'static str, Stat)>,
    pub hitch_count: usize,
    /// The first [`HITCH_LIST_MAX`] hitches.
    pub hitches: Vec<Hitch>,
    pub peak_rss_kib: u64,
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

impl Summary {
    pub fn of(r: &Report, hitch_ms: u32) -> Self {
        let frame_us: u64 = r.frames.iter().map(|f| u64::from(f.interval)).sum();
        let tick_span = match (r.ticks.first(), r.ticks.last()) {
            (Some(Tick { at_us: Some(a), .. }), Some(Tick { at_us: Some(b), .. })) if b > a => {
                Some(b - a)
            }
            _ => None,
        };
        let seconds = if frame_us > 0 {
            Some(frame_us as f64 / 1e6)
        } else {
            tick_span.map(|us| us as f64 / 1e6)
        };
        let rate = |n: usize| seconds.map(|s| round1(n as f64 / s));
        let flagged = |bits: u8| -> Option<usize> {
            let flags: Option<Vec<u8>> = r.frames.iter().map(|f| f.flags).collect();
            flags
                .filter(|f| !f.is_empty())
                .map(|f| f.iter().filter(|&&x| x & bits != 0).count())
        };
        let mut phases = Vec::new();
        let mut add = |name: &'static str, s: Stat| {
            if s.n > 0 {
                phases.push((name, s));
            }
        };
        add("frame", Stat::of(r.frames.iter().map(|f| f.interval)));
        add("pre", Stat::of(r.frames.iter().map(|f| f.pre)));
        add("update", Stat::of(r.frames.iter().map(|f| f.update)));
        add("post", Stat::of(r.frames.iter().map(|f| f.post)));
        add("wait", Stat::of(r.frames.iter().map(Frame::wait)));
        add("render", Stat::of(r.render.iter().copied()));
        add("server", Stat::of(r.ticks.iter().map(Tick::total)));
        add("drain", Stat::of(r.ticks.iter().map(|t| t.drain_us)));
        add("tick", Stat::of(r.ticks.iter().map(|t| t.tick_us)));
        add("flush", Stat::of(r.ticks.iter().map(|t| t.flush_us)));
        add("bridge", Stat::of(r.bridge.iter().copied()));

        let limit = hitch_ms.saturating_mul(1000);
        let has_clock = r.ticks.iter().all(|t| t.at_us.is_some());
        let mut hitch_count = 0;
        let mut hitches = Vec::new();
        for (i, f) in r.frames.iter().enumerate() {
            if f.interval <= limit {
                continue;
            }
            hitch_count += 1;
            if hitches.len() >= HITCH_LIST_MAX {
                continue;
            }
            let end = f.start_us + u64::from(f.interval);
            let (phase, phase_us) = [
                ("pre", f.pre),
                ("update", f.update),
                ("post", f.post),
                ("wait", f.wait()),
            ]
            .into_iter()
            .fold(("pre", 0), |best, p| if p.1 > best.1 { p } else { best });
            let server_us = has_clock.then(|| {
                r.ticks
                    .iter()
                    .filter(|t| t.at_us.is_some_and(|a| a >= f.start_us && a < end))
                    .map(Tick::total)
                    .fold(0u32, u32::saturating_add)
            });
            let mut causes: Vec<Cause> = Vec::new();
            for s in &r.spans {
                let s_end = s.start_us + u64::from(s.dur_us);
                if s.start_us > end || s_end < f.start_us {
                    continue;
                }
                match causes.iter_mut().find(|c| c.kind == s.kind) {
                    Some(c) => {
                        c.count += 1;
                        c.us += u64::from(s.dur_us);
                    }
                    None => causes.push(Cause {
                        kind: s.kind.clone(),
                        count: 1,
                        us: u64::from(s.dur_us),
                    }),
                }
            }
            hitches.push(Hitch {
                frame: i,
                start_us: f.start_us,
                us: f.interval,
                phase,
                phase_us,
                server_us,
                causes,
            });
        }
        Summary {
            hitch_ms,
            seconds: seconds.map(|s| (s * 1000.0).round() / 1000.0),
            frames: r.frames.len(),
            ticks: r.ticks.len(),
            fps: if r.frames.is_empty() {
                None
            } else {
                rate(r.frames.len())
            },
            ticks_per_s: rate(r.ticks.len()),
            drawn_per_s: flagged(DREW).and_then(rate),
            pictures_per_s: flagged(DREW | MOVED).and_then(rate),
            phases,
            hitch_count,
            hitches,
            peak_rss_kib: r.peak_rss_kib,
        }
    }

    fn phase(&self, name: &str) -> Option<Stat> {
        self.phases.iter().find(|p| p.0 == name).map(|p| p.1)
    }

    /// The budget lines this summary breaks (empty: within budget).
    pub fn over(&self, b: &Budget) -> Vec<String> {
        let mut out = Vec::new();
        let mut check = |name: &str, phase: &str, ms: Option<f64>| {
            if let (Some(ms), Some(s)) = (ms, self.phase(phase)) {
                if f64::from(s.p95) > ms * 1000.0 {
                    out.push(format!(
                        "{name} p95 {:.2} ms > {ms} ms",
                        f64::from(s.p95) / 1000.0
                    ));
                }
            }
        };
        check("tick", "server", b.tick_ms);
        check("frame", "frame", b.frame_ms);
        if let Some(n) = b.hitches {
            if self.hitch_count > n {
                out.push(format!(
                    "{} hitches over {} ms > {n}",
                    self.hitch_count, self.hitch_ms
                ));
            }
        }
        out
    }

    /// The summary as JSON; with a budget, a `verdict` too.
    pub fn to_json(&self, budget: Option<&Budget>) -> Value {
        let stat =
            |s: &Stat| json!({"n": s.n, "p50": s.p50, "p95": s.p95, "p99": s.p99, "max": s.max});
        let phases: serde_json::Map<String, Value> = self
            .phases
            .iter()
            .map(|(n, s)| ((*n).to_owned(), stat(s)))
            .collect();
        let hitches: Vec<Value> = self
            .hitches
            .iter()
            .map(|h| {
                json!({
                    "frame": h.frame, "start_us": h.start_us, "us": h.us,
                    "phase": h.phase, "phase_us": h.phase_us, "server_us": h.server_us,
                    "causes": h.causes.iter().map(|c| json!({"kind": c.kind, "count": c.count, "us": c.us})).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mut v = json!({
            "seconds": self.seconds, "frames": self.frames, "ticks": self.ticks,
            "fps": self.fps, "ticks_per_s": self.ticks_per_s,
            "drawn_per_s": self.drawn_per_s, "pictures_per_s": self.pictures_per_s,
            "phases_us": phases, "hitch_ms": self.hitch_ms,
            "hitch_count": self.hitch_count, "hitches": hitches,
            "peak_rss_kib": self.peak_rss_kib,
        });
        if let Some(b) = budget {
            let over = self.over(b);
            v["verdict"] = json!({"pass": over.is_empty(), "over": over});
        }
        v
    }

    /// The summary as text for a terminal.
    pub fn to_text(&self, budget: Option<&Budget>) -> String {
        let ms = |us: u32| f64::from(us) / 1000.0;
        let opt = |x: Option<f64>, unit: &str| x.map_or("-".to_owned(), |x| format!("{x}{unit}"));
        let mut t = String::new();
        let _ = writeln!(
            t,
            "{} s, {} frames ({}), {} ticks ({}), drawn {}, pictures {}, peak {} MiB",
            opt(self.seconds, ""),
            self.frames,
            opt(self.fps, " fps"),
            self.ticks,
            opt(self.ticks_per_s, "/s"),
            opt(self.drawn_per_s, "/s"),
            opt(self.pictures_per_s, "/s"),
            self.peak_rss_kib / 1024
        );
        let _ = writeln!(
            t,
            "{:<8}{:>7}{:>9}{:>9}{:>9}{:>9}  (ms)",
            "phase", "n", "p50", "p95", "p99", "max"
        );
        for (n, s) in &self.phases {
            let _ = writeln!(
                t,
                "{n:<8}{:>7}{:>9.2}{:>9.2}{:>9.2}{:>9.2}",
                s.n,
                ms(s.p50),
                ms(s.p95),
                ms(s.p99),
                ms(s.max)
            );
        }
        let _ = writeln!(t, "hitches over {} ms: {}", self.hitch_ms, self.hitch_count);
        for h in &self.hitches {
            let causes: Vec<String> = h
                .causes
                .iter()
                .map(|c| format!("{} x{} ({:.1} ms)", c.kind, c.count, c.us as f64 / 1000.0))
                .collect();
            let server = h
                .server_us
                .map_or(String::new(), |s| format!(", server {:.1} ms", ms(s)));
            let _ = writeln!(
                t,
                "  frame {} at {:.2} s: {:.1} ms, {} {:.1} ms{server}, causes: {}",
                h.frame,
                h.start_us as f64 / 1e6,
                ms(h.us),
                h.phase,
                ms(h.phase_us),
                if causes.is_empty() {
                    "none marked".to_owned()
                } else {
                    causes.join(", ")
                }
            );
        }
        if self.hitch_count > self.hitches.len() {
            let _ = writeln!(t, "  ... {} more", self.hitch_count - self.hitches.len());
        }
        if let Some(b) = budget {
            let over = self.over(b);
            if over.is_empty() {
                let _ = writeln!(t, "verdict: PASS");
            } else {
                let _ = writeln!(t, "verdict: FAIL: {}", over.join("; "));
            }
        }
        t
    }
}

/// Limits for a verdict: `tick` and `frame` are p95 milliseconds (the
/// server's drain + tick + flush, the client frame), `hitch` the most
/// hitches allowed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Budget {
    pub tick_ms: Option<f64>,
    pub frame_ms: Option<f64>,
    pub hitches: Option<usize>,
}

impl Budget {
    /// Reads `tick=40,frame=17,hitch=0` (any subset).
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut b = Budget::default();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| format!("budget {part:?}: expected KEY=VALUE"))?;
            let bad = || format!("budget {part:?}: bad value");
            match k.trim() {
                "tick" => b.tick_ms = Some(v.trim().parse().map_err(|_| bad())?),
                "frame" => b.frame_ms = Some(v.trim().parse().map_err(|_| bad())?),
                "hitch" => b.hitches = Some(v.trim().parse().map_err(|_| bad())?),
                _ => return Err(format!("budget {part:?}: keys are tick, frame, hitch")),
            }
        }
        Ok(b)
    }
}

const USAGE: &str =
    "usage: D2RUST-Game perf-summary FILE [--hitch-ms N] [--budget tick=40,frame=17,hitch=0] [--json]";

/// `perf-summary FILE [--hitch-ms N] [--budget SPEC] [--json]`: prints the
/// summary of a report. Exit code: 0 within budget (or no budget), 1 over
/// budget, 2 a bad argument or an unreadable report.
pub fn run_cli(args: &[String]) -> i32 {
    let mut file = None;
    let mut hitch_ms = HITCH_MS_DEFAULT;
    let mut budget = None;
    let mut as_json = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--hitch-ms" => match it.next().and_then(|v| v.parse().ok()) {
                Some(n) => hitch_ms = n,
                None => {
                    eprintln!("perf-summary: --hitch-ms needs a whole number of ms\n{USAGE}");
                    return 2;
                }
            },
            "--budget" => match it.next().map(|v| Budget::parse(v)) {
                Some(Ok(b)) => budget = Some(b),
                Some(Err(e)) => {
                    eprintln!("perf-summary: {e}");
                    return 2;
                }
                None => {
                    eprintln!("perf-summary: --budget needs a value\n{USAGE}");
                    return 2;
                }
            },
            "--json" => as_json = true,
            f if file.is_none() && !f.starts_with("--") => file = Some(f.to_owned()),
            other => {
                eprintln!("perf-summary: unexpected {other:?}\n{USAGE}");
                return 2;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("{USAGE}");
        return 2;
    };
    let report = match std::fs::read_to_string(&file)
        .map_err(|e| e.to_string())
        .and_then(|t| Report::parse(&t).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("perf-summary: {file}: {e}");
            return 2;
        }
    };
    let s = Summary::of(&report, hitch_ms);
    if as_json {
        println!("{}", s.to_json(budget.as_ref()));
    } else {
        print!("perf {}: {}", report.version, s.to_text(budget.as_ref()));
    }
    match budget {
        Some(b) if !s.over(&b).is_empty() => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(start_us: u64, interval: u32, flags: u8) -> Frame {
        Frame {
            interval,
            pre: interval / 4,
            update: interval / 4,
            post: 100,
            start_us,
            flags: Some(flags),
        }
    }

    /// One second at 60 fps with one 80 ms frame during a room load and a
    /// save; ticks every 40 ms.
    fn synthetic() -> Report {
        let mut frames = Vec::new();
        let mut t = 0u64;
        for i in 0..60u32 {
            let interval = if i == 30 { 80_000 } else { 15_000 };
            let flags = if i % 2 == 0 { DREW } else { MOVED };
            frames.push(frame(t, interval, flags));
            t += u64::from(interval);
        }
        let ticks = (0..25u32)
            .map(|i| Tick {
                frame: i + 1,
                drain_us: 100,
                tick_us: if i == 12 { 9_000 } else { 2_000 },
                flush_us: 300,
                at_us: Some(u64::from(i) * 40_000),
            })
            .collect();
        let hitch_start = 30 * 15_000;
        Report {
            version: 2,
            peak_rss_kib: 4096,
            ticks,
            frames,
            render: vec![5_000; 60],
            bridge: Vec::new(),
            spans: vec![
                Span {
                    kind: "room".into(),
                    start_us: hitch_start + 1_000,
                    dur_us: 30_000,
                },
                Span {
                    kind: "room".into(),
                    start_us: hitch_start + 40_000,
                    dur_us: 10_000,
                },
                Span {
                    kind: "save".into(),
                    start_us: hitch_start + 70_000,
                    dur_us: 20_000,
                },
                // Before the hitch: not a cause.
                Span {
                    kind: "atlas".into(),
                    start_us: 1_000,
                    dur_us: 0,
                },
            ],
        }
    }

    #[test]
    fn synthetic_frames_give_the_expected_hitches() {
        let s = Summary::of(&synthetic(), 50);
        assert_eq!(s.hitch_count, 1);
        let h = &s.hitches[0];
        assert_eq!((h.frame, h.us, h.start_us), (30, 80_000, 450_000));
        // 80 ms: pre 20, update 20, post 0.1, so the rest (39.9) is waiting.
        assert_eq!((h.phase, h.phase_us), ("wait", 39_900));
        assert_eq!(
            h.causes,
            vec![
                Cause {
                    kind: "room".into(),
                    count: 2,
                    us: 40_000
                },
                Cause {
                    kind: "save".into(),
                    count: 1,
                    us: 20_000
                },
            ]
        );
        // Ticks starting at 480 and 520 ms (inside 450..530); 480 is the slow one.
        assert_eq!(h.server_us, Some(11_800));
        // A lower limit finds every frame over it.
        assert_eq!(Summary::of(&synthetic(), 10).hitch_count, 60);
    }

    #[test]
    fn rates_and_phases() {
        let s = Summary::of(&synthetic(), 50);
        assert_eq!(s.seconds, Some(0.965));
        assert_eq!(s.frames, 60);
        assert_eq!(s.fps, Some(62.2));
        assert_eq!(s.ticks_per_s, Some(25.9));
        assert_eq!(s.drawn_per_s, Some(31.1));
        assert_eq!(s.pictures_per_s, Some(62.2));
        let frame = s.phase("frame").unwrap();
        assert_eq!(
            (frame.n, frame.p50, frame.p95, frame.max),
            (60, 15_000, 15_000, 80_000)
        );
        assert_eq!(s.phase("server").unwrap().max, 9_400);
        assert!(s.phase("bridge").is_none());
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let s = Stat::of(1..=100);
        assert_eq!((s.p50, s.p95, s.p99, s.max), (50, 95, 99, 100));
        assert_eq!(Stat::of([7]).p50, 7);
        assert_eq!(Stat::of([]), Stat::default());
    }

    #[test]
    fn perf_2_round_trips_and_carries_the_summary() {
        let r = synthetic();
        let text = r.to_json(HITCH_MS_DEFAULT);
        assert_eq!(Report::parse(&text).unwrap(), r);
        let d: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(d["summary"]["hitch_count"], 1);
        assert_eq!(d["summary"]["hitches"][0]["causes"][0]["kind"], "room");
    }

    #[test]
    fn perf_1_reports_still_read() {
        let text = r#"{"format":"perf 1","peak_rss_kib":2048,
"ticks":[[1,10,2000,300],[2,10,2000,300]],
"frames":[[16000,4000,5000,100],[70000,60000,5000,100],[16000,4000,5000,100]],
"render":[3000],
"bridge":[]}"#;
        let r = Report::parse(text).unwrap();
        assert_eq!(r.version, 1);
        assert_eq!(r.ticks[1].at_us, None);
        assert_eq!(r.frames[2].start_us, 86_000);
        assert_eq!(r.frames[0].flags, None);
        let s = Summary::of(&r, 50);
        assert_eq!(s.hitch_count, 1);
        let h = &s.hitches[0];
        assert_eq!((h.frame, h.phase, h.server_us), (1, "pre", None));
        assert_eq!(s.drawn_per_s, None);
        assert_eq!(s.pictures_per_s, None);
    }

    #[test]
    fn bad_reports_are_errors() {
        assert!(matches!(Report::parse("{"), Err(ReadError::Json(_))));
        assert!(matches!(
            Report::parse(r#"{"format":"perf 9"}"#),
            Err(ReadError::Format(_))
        ));
        assert!(matches!(
            Report::parse(r#"{"format":"perf 2","frames":[[1,2,3]]}"#),
            Err(ReadError::Entry("frames"))
        ));
    }

    #[test]
    fn headless_runs_take_their_length_from_the_tick_clock() {
        let r = Report {
            version: 2,
            ticks: (0..26u32)
                .map(|i| Tick {
                    frame: i,
                    at_us: Some(u64::from(i) * 40_000),
                    ..Tick::default()
                })
                .collect(),
            ..Report::default()
        };
        let s = Summary::of(&r, 50);
        assert_eq!(s.seconds, Some(1.0));
        assert_eq!(s.ticks_per_s, Some(26.0));
        assert_eq!(s.fps, None);
    }

    #[test]
    fn budgets_set_the_verdict() {
        let s = Summary::of(&synthetic(), 50);
        let pass = Budget::parse("tick=40,frame=17").unwrap();
        assert!(s.over(&pass).is_empty());
        let fail = Budget::parse("tick=2, frame=10,hitch=0").unwrap();
        let over = s.over(&fail);
        assert_eq!(over.len(), 3, "{over:?}");
        assert_eq!(s.to_json(Some(&fail))["verdict"]["pass"], false);
        assert!(s.to_text(Some(&fail)).contains("verdict: FAIL"));
        assert!(s.to_text(Some(&pass)).contains("verdict: PASS"));
        assert!(Budget::parse("fps=60").is_err());
        assert!(Budget::parse("tick").is_err());
        assert!(Budget::parse("hitch=x").is_err());
    }
}
