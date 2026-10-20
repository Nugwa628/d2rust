//! `repro FILE`: runs a `scenario 2` script (`crate::scenario_script`) on
//! the headless play client and answers pass or fail, with the run's
//! artifacts in a folder. It is the tool a bug lane uses to rebuild a bug
//! in seconds, without a window: who plays where, what they do, and (with
//! checks) what must be true.
//!
//! The engine is [`Host`] (`autoplay-host`): the real client on Bevy's
//! `MinimalPlugins`, UI included, with the game-rate stepping clock, so one
//! client pass is one server tick and the run is the same every time.
//! The runner drives it in process:
//!
//! - **Start:** the character comes from `save FILE` or is built from the
//!   inline `char` lines ([`inline_character`]); after the join the setup
//!   raises the level (the dev `level` command), sets `char stat` values,
//!   warps to `char area` and places the player at `char at`, then lets
//!   the client settle for [`SETTLE_TICKS`] ticks. Tick 0 of the script is
//!   the first tick after that.
//! - **Steps:** `at T` runs once T ticks have passed since tick 0, before
//!   the next tick. Pokes, messages and dev lines run on the server thread
//!   between frames (the same hooks `play --poke` / `--send` and the dev
//!   panel use); input steps become one `--input` script the client's
//!   input system plays, exactly as `play --input` does.
//! - **Checks:** `at T check EXPR` reads the client's `state` reply once
//!   T ticks have passed, before that tick's steps run; `by T check EXPR`
//!   reads it after every tick from 0 and passes at the first tick it
//!   holds, failing at T. EXPR names a value ([`query`]); a failing check
//!   reports the value it read.
//! - **Queries:** `--query "NAME, ..." --at T,...` reads the named values
//!   at those ticks (default: the end) and prints one JSON line per tick.
//! - **End:** the run stops once `end` ticks have passed.
//!
//! Nothing here changes the game: the runner only adds hooks and an input
//! script to its own headless app. `play` and every other mode are
//! untouched.
//!
//! Output (`OUT`, default `repro-out/<name>`): `repro.json` (format
//! [`FORMAT`]), `state-tail.jsonl` (the server `state-1` snapshot of the
//! last [`TAIL_TICKS`] ticks) and `client-state.json` (the client's view
//! at the end, the `autoplay-host` `state` reply). Exit codes: [`exit`].

pub mod batch;
pub mod checkpoint;
pub(crate) mod invariants;
pub mod query;
pub mod soak_scenario;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use d2_formats::d2s::{self, Body, D2s, Golem, Header, StatEntry, Stats};
use serde_json::{json, Value};

use super::autoplay_host::Host;
use super::dev::server::{DevCatalog, DevChannel, DevQueue, DevServer};
use super::poke::{self, Entry, ResultLog, When};
use super::single_player::{self, Character, GameData, LiveData};
use super::state_dump::StepClock;
use crate::bridge::state::StateSource;
use crate::scenario_script::{Inline, Scenario, Start, StepMsg, SCENARIO_FORMAT_V2};

/// The report's format word.
pub const FORMAT: &str = "repro 1";

/// Ticks the client gets after the start setup, before the script's
/// tick 0 (a warp's level load and the first draws).
pub const SETTLE_TICKS: i32 = 8;

/// Server snapshots kept for `state-tail.jsonl`.
pub const TAIL_TICKS: usize = 10;

/// Passes allowed past `end` before the run gives up (a server that
/// stopped ticking).
const SLACK_PASSES: u64 = 2000;

/// The town level of each act (where a character of that act starts).
const ACT_TOWNS: [u32; 5] = [1, 40, 75, 103, 109];

/// The process exit codes of `repro`.
pub mod exit {
    /// Every step ran and every check held.
    pub const PASS: u8 = 0;
    /// A step or a check failed.
    pub const FAIL: u8 = 1;
    /// The scenario does not parse, or asks for something the runner
    /// cannot build.
    pub const INVALID: u8 = 2;
    /// The game files are missing or do not load.
    pub const NO_GAME: u8 = 3;
}

/// How a run is set up.
#[derive(Clone, Debug, Default)]
pub struct RunOpts {
    /// The artifacts folder.
    pub out: PathBuf,
    /// The game folder; `None`: `D2_GAME_DIR`.
    pub game_dir: Option<PathBuf>,
    /// The scenario file's folder: `save FILE` paths are relative to it.
    pub base_dir: PathBuf,
    /// Run every step even after a failure (the report lists them all).
    pub keep_going: bool,
    /// The scenario file as given, for the report.
    pub file: String,
    /// Names to read (`--query`).
    pub queries: Vec<query::Name>,
    /// The script ticks to read them at (`--at`); empty: the end.
    pub query_at: Vec<u32>,
    /// Write `state-end.jsonl`: the server `state-1` line after the run.
    /// [`checkpoint`] saves use it; ordinary runs leave it off.
    pub capture_state: bool,
}

/// What a run produced.
#[derive(Clone, Debug, PartialEq)]
pub struct ReproReport {
    /// [`exit::PASS`] or [`exit::FAIL`].
    pub exit: u8,
    /// `repro.json`'s content.
    pub json: Value,
}

/// Why a run could not start or finish.
#[derive(Debug, thiserror::Error)]
pub enum ReproError {
    /// The scenario is not one the runner can build ([`exit::INVALID`]).
    #[error("invalid scenario: {0}")]
    Invalid(String),
    /// The game files are missing ([`exit::NO_GAME`]).
    #[error("game files: {0}")]
    NoGame(String),
    /// The run broke down (a stopped server thread, an unwritable folder).
    #[error("{0:#}")]
    Run(anyhow::Error),
}

impl ReproError {
    /// The process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Invalid(_) => exit::INVALID,
            Self::NoGame(_) => exit::NO_GAME,
            Self::Run(_) => exit::FAIL,
        }
    }
}

impl From<anyhow::Error> for ReproError {
    fn from(e: anyhow::Error) -> Self {
        Self::Run(e)
    }
}

impl From<std::io::Error> for ReproError {
    fn from(e: std::io::Error) -> Self {
        Self::Run(anyhow!(e))
    }
}

impl From<serde_json::Error> for ReproError {
    fn from(e: serde_json::Error) -> Self {
        Self::Run(anyhow!(e))
    }
}

/// One step's outcome, in the order the steps ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepOutcome {
    /// Script tick (ticks after tick 0); `None` for start setup.
    pub at: Option<u32>,
    /// `setup`, `poke`, `msg`, `dev`, `input` or `check`.
    pub kind: &'static str,
    /// The step as written.
    pub text: String,
    /// Whether it did what it asked.
    pub ok: bool,
    /// The result as the game reported it.
    pub result: String,
}

impl StepOutcome {
    fn json(&self) -> Value {
        json!({
            "at": self.at,
            "kind": self.kind,
            "text": self.text,
            "ok": self.ok,
            "result": self.result,
        })
    }
}

/// The scenario's steps sorted into the channels that run them, with
/// ticks made absolute (server frames) from tick 0 at `start`.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Pokes, due as `play --poke` frames.
    pub pokes: Vec<Entry>,
    /// C→S messages: (frame f, the message), injected once f − 1 ran.
    pub sends: Vec<(i32, StepMsg)>,
    /// Dev lines: (frame f, the line), run once f − 1 ran.
    pub dev: Vec<(i32, String)>,
    /// The `--input` script text (`frame F; step;...`).
    pub input: String,
    /// The checks, in script order.
    pub checks: Vec<PlannedCheck>,
    /// Read-only save/load checkpoints in game ticks.
    pub checkpoints: Vec<u32>,
    /// Conditional player keys: script tick, virtual key and read-only check.
    pub conditional_keys: Vec<(u32, u32, String)>,
    /// The last server frame the run reaches.
    pub end_frame: i32,
}

/// A check line of the script.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedCheck {
    /// Its script tick.
    pub tick: u32,
    /// `by T check`: holds after some tick up to `tick`.
    pub by: bool,
    pub check: query::Check,
    /// The expression as written.
    pub text: String,
}

/// The frame a step at script tick `t` precedes, with tick 0 at server
/// frame `start`: it runs once frame `start + t` has run.
pub fn step_frame(start: i32, t: u32) -> i32 {
    start
        .saturating_add(i32::try_from(t).unwrap_or(i32::MAX))
        .saturating_add(1)
}

/// Sorts `s`'s steps for a run whose tick 0 is server frame `start`.
/// Scenario times are rounded up to game ticks, never earlier than requested.
pub fn plan(s: &Scenario, start: i32) -> Result<Plan, ReproError> {
    let s = game_timing(s)?;
    let mut p = Plan {
        end_frame: start.saturating_add(i32::try_from(s.end).unwrap_or(i32::MAX)),
        ..Plan::default()
    };
    let mut input = Vec::new();
    let mut input_frame = None;
    for step in &s.steps {
        let f = step_frame(start, step.tick);
        match &step.msg {
            StepMsg::Checkpoint => p.checkpoints.push(step.tick),
            StepMsg::ConditionalKey { key, expr } => {
                p.conditional_keys.push((step.tick, *key, expr.clone()))
            }
            StepMsg::Poke(d) => p.pokes.push(Entry {
                when: When::Frame(f),
                op: d2_sim::poke::PokeOp::Directive(d.clone()),
            }),
            StepMsg::Spawn(_) => {
                // A v1 spawn step is the poke `spawn` with the same words.
                let text = spawn_text(&step.msg);
                let toks: Vec<&str> = text.split_whitespace().collect();
                let op = d2_sim::poke::parse_op(&toks)
                    .map_err(|e| ReproError::Invalid(format!("at {} {text}: {e}", step.tick)))?;
                p.pokes.push(Entry {
                    when: When::Frame(f),
                    op,
                });
            }
            StepMsg::Hex(_) | StepMsg::Typed { .. } => p.sends.push((f, step.msg.clone())),
            StepMsg::Dev(line) => p.dev.push((f, line.clone())),
            StepMsg::Input(text) => {
                // Consecutive paced keys share their frame marker, so closing
                // a pause menu can run on client passes while the server waits.
                if input_frame != Some(f) {
                    input.push(format!("frame {f}"));
                    input_frame = Some(f);
                }
                input.push(text.clone());
            }
            StepMsg::Check { by, expr } => {
                p.checks.push(PlannedCheck {
                    tick: step.tick,
                    by: *by,
                    check: query::parse_check(expr).map_err(ReproError::Invalid)?,
                    text: expr.clone(),
                });
            }
        }
    }
    p.input = input.join("; ");
    Ok(p)
}

/// Convert a scenario deadline or duration without shortening its real time.
fn game_ticks(t: u32, rate: u32) -> Result<u32, ReproError> {
    if !(1..=1000).contains(&rate) {
        return Err(ReproError::Invalid("tick_rate outside 1..=1000".into()));
    }
    u32::try_from(
        (u64::from(t) * u64::from(d2_sim::tick::TickRate::GAME.hz())).div_ceil(u64::from(rate)),
    )
    .map_err(|_| ReproError::Invalid("scenario time overflows game ticks".into()))
}

fn game_timing(source: &Scenario) -> Result<Scenario, ReproError> {
    let mut s = source.clone();
    if source.tick_rate == d2_sim::tick::TickRate::GAME.hz() {
        return Ok(s);
    }
    s.end = game_ticks(s.end, source.tick_rate)?;
    s.snapshot_at = source
        .snapshot_ticks()
        .iter()
        .map(|&t| game_ticks(t, source.tick_rate))
        .collect::<Result<_, _>>()?;
    s.snapshot_every = None;
    for step in &mut s.steps {
        step.tick = game_ticks(step.tick, source.tick_rate)?;
        let StepMsg::Input(text) = &mut step.msg else {
            continue;
        };
        use crate::world_view::input_script::Step as Input;
        let parsed = crate::world_view::input_script::parse_step(text, (4096, 4096))
            .map_err(ReproError::Invalid)?;
        *text = match parsed {
            Input::WaitUntil(wait) => format!(
                "wait-until {} max {}",
                wait.expr,
                game_ticks(wait.max, source.tick_rate)?
            ),
            Input::Hold(at, n) => format!(
                "hold {} {} {}",
                at.x,
                at.y,
                game_ticks(n, source.tick_rate)?
            ),
            _ => text.clone(),
        };
    }
    s.tick_rate = d2_sim::tick::TickRate::GAME.hz();
    Ok(s)
}

/// A spawn step's words (`spawn <class> <x> <y> <kind> [umod...]`), as the
/// scenario writer prints them.
fn spawn_text(msg: &StepMsg) -> String {
    let probe = Scenario {
        version: SCENARIO_FORMAT_V2,
        tick_rate: d2_sim::tick::TickRate::GAME.hz(),
        name: "x".into(),
        seed: 0,
        init: 0,
        difficulty: crate::scenario_script::Difficulty::Normal,
        expansion: true,
        end: 0,
        variant: None,
        save: None,
        save_file: Some("x".into()),
        character: None,
        record: Vec::new(),
        snapshot_every: None,
        snapshot_at: Vec::new(),
        allow: Default::default(),
        steps: vec![crate::scenario_script::Step {
            tick: 0,
            msg: msg.clone(),
        }],
    };
    probe
        .to_text()
        .lines()
        .last()
        .and_then(|l| l.strip_prefix("at 0 "))
        .unwrap_or_default()
        .to_owned()
}

/// What the runner cannot build from inline `char` lines yet.
pub fn unsupported_inline(c: &Inline) -> Option<&'static str> {
    if !c.items.is_empty() {
        Some("`char item` (give items with `at T poke item` or `at T dev item`)")
    } else if !c.waypoints.is_empty() {
        Some("`char waypoint` (use `at 0 dev waypoints`)")
    } else if c.map.is_some() {
        Some("`char map`")
    } else {
        None
    }
}

/// The character of inline `char` lines: a save of that class at level 1
/// with the class's starting attributes (as a new character has them),
/// the listed skills at their base levels and the mouse skills of
/// `char hand`, standing in the town of its act. The level, the stat
/// values and the place are set after the join ([`Host`] setup), so the
/// game's own level-up rules give the life, mana and points of a
/// character of that level.
pub fn inline_character(
    c: &Inline,
    live: &LiveData,
    difficulty: u8,
) -> Result<Character, ReproError> {
    use d2_data::tables::{Charstats, Skills};
    if let Some(what) = unsupported_inline(c) {
        return Err(ReproError::Invalid(format!(
            "repro cannot build {what} yet"
        )));
    }
    if c.act > 4 {
        return Err(ReproError::Invalid(format!("char area: act {}", c.act)));
    }
    let charstats = live
        .tables
        .rows::<Charstats>()
        .map_err(|e| ReproError::NoGame(format!("charstats: {e}")))?;
    let skills_rows = live
        .tables
        .rows::<Skills>()
        .map_err(|e| ReproError::NoGame(format!("skills: {e}")))?;
    let row = charstats
        .get(usize::from(c.class))
        .ok_or_else(|| ReproError::Invalid(format!("char class {}: no such class", c.class)))?;
    let mut header = Header::default();
    header
        .set_name(b"Repro")
        .ok_or_else(|| ReproError::Run(anyhow!("the name `Repro` does not fit")))?;
    header.class = c.class;
    header.level = 1;
    header.status = d2s::status::EXPANSION;
    header.towns = [0; 3];
    header.towns[usize::from(difficulty.min(2))] = 0x80 | c.act;
    for (slot, skill) in header.mouse.iter_mut().zip(c.hands) {
        if let Some(id) = skill {
            *slot = d2s::Slot::encode(i32::from(id), false, 0)
                .ok_or_else(|| ReproError::Invalid(format!("char hand: skill {id}")))?;
        }
    }
    let (str_, int, dex, vit) = (
        i32::from(row.str),
        i32::from(row.int),
        i32::from(row.dex),
        i32::from(row.vit),
    );
    let life = (vit + i32::from(row.hpadd)) << 8;
    let stamina = i32::from(row.stamina) << 8;
    let mut stats: Vec<StatEntry> = [
        (0u16, str_),
        (1, int),
        (2, dex),
        (3, vit),
        (6, life),
        (7, life),
        (8, int << 8),
        (9, int << 8),
        (10, stamina),
        (11, stamina),
        (12, 1),
    ]
    .into_iter()
    .map(|(id, value)| StatEntry {
        id,
        layer: 0,
        value,
    })
    .collect();
    stats.sort_by_key(|e| e.id);
    let class_skills = d2_sim::skills::list::class_skills(&skills_rows, i32::from(c.class));
    let mut levels = vec![0u8; 30];
    for &(id, lvl) in &c.skills {
        let i = class_skills
            .iter()
            .position(|&s| s == i32::from(id))
            .filter(|&i| i < levels.len())
            .ok_or_else(|| {
                ReproError::Invalid(format!("char skill {id}: not a skill of class {}", c.class))
            })?;
        levels[i] = lvl;
    }
    let mut body = Body {
        stats: Stats::Bits(stats),
        skills: levels,
        hireling_items: Some(None),
        golem: Some(Golem {
            flag: 0,
            item: None,
        }),
        ..Body::default()
    };
    // `char quest <slot> <flags>`: the 16-bit flag word of the slot, in every
    // difficulty (the query's flags are the same in each).
    for &(slot, flags) in &c.quests {
        let at = 2 * usize::from(slot);
        for record in &mut body.quests.records {
            if let Some(word) = record.get_mut(at..at + 2) {
                word.copy_from_slice(&flags.to_le_bytes());
            }
        }
    }
    let save = D2s {
        header,
        body: Some(body),
    };
    Ok(Character::Save(
        Box::new(save),
        d2_server::adapters::character::LoadContext {
            difficulty,
            map_seed_applies: false,
        },
    ))
}

/// The start setup of inline `char` lines, run on the server once the
/// player exists: the level, the stat values, the area and the place.
fn inline_setup(c: &Inline) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if c.level > 1 {
        out.push(("dev", format!("level {}", c.level)));
    }
    for (id, v) in &c.stats {
        out.push(("poke", format!("stat @player {id} 0 {v}")));
    }
    if ACT_TOWNS.get(usize::from(c.act)) != Some(&c.area) {
        out.push(("poke", format!("warp {}", c.area)));
    }
    if let Start::At(x, y) = c.at {
        out.push(("poke", format!("pos @player {x} {y}")));
    }
    out
}

/// Runs one setup line on the server now.
fn run_setup(host: &Host, kind: &'static str, text: &str) -> Result<StepOutcome> {
    let server = host.server();
    let mut link = server.lock().map_err(|_| anyhow!("server lock"))?;
    let (ok, result) = match kind {
        "dev" => {
            let cmd = d2_proto::dev::parse(text)
                .map_err(|e| anyhow!("{text}: {e}"))?
                .ok_or_else(|| anyhow!("{text}: not a server command"))?;
            link.with(move |l| {
                let r = DevServer::new(DevCatalog::default()).apply(&mut l.host_mut().game, &cmd);
                (r.ok, r.text)
            })
            .map_err(|e| anyhow!("{e:?}"))?
        }
        _ => {
            let toks: Vec<&str> = text.split_whitespace().collect();
            let op = d2_sim::poke::parse_op(&toks).map_err(|e| anyhow!("{text}: {e}"))?;
            link.with(move |l| {
                let r = poke::apply_on_link(l, &op);
                (r.code() == "ok", r.code().to_owned())
            })
            .map_err(|e| anyhow!("{e:?}"))?
        }
    };
    Ok(StepOutcome {
        at: None,
        kind: "setup",
        text: format!("{kind} {text}"),
        ok,
        result,
    })
}

/// The steps' log, filled on the server thread.
pub(crate) type StepLog = Arc<Mutex<Vec<StepOutcome>>>;

/// The script ticks the `goto` results were logged on, in order.
fn gotos_finished(log: &ResultLog, start: i32) -> Vec<u32> {
    log.lock()
        .map(|l| {
            l.iter()
                .filter(|e| e.op.starts_with("goto "))
                .filter_map(|e| script_tick(start, e.frame + 1))
                .collect()
        })
        .unwrap_or_default()
}

/// The script tick of server frame `f` (the step that precedes `f`).
fn script_tick(start: i32, f: i32) -> Option<u32> {
    u32::try_from(f - 1 - start).ok()
}

/// Installs the plan's server-side steps: pokes (with their result log),
/// sends and dev lines, in one hook before every pump, as `play --poke`
/// / `--send` and the dev panel run them.
fn install(
    host: &Host,
    plan: &mut Plan,
    start: i32,
    live: &LiveData,
    pokes_log: ResultLog,
    log: StepLog,
) -> Result<()> {
    let server = host.server();
    let mut link = server.lock().map_err(|_| anyhow!("server lock"))?;
    let (tick_end, before) = poke::split_tick_end(std::mem::take(&mut plan.pokes));
    poke::install_tick_end_logged(&mut link, tick_end, Some(pokes_log.clone()))
        .map_err(|e| anyhow!("{e:?}"))?;
    let mut pokes = poke::Schedule::new(before).with_log(pokes_log);
    let mut sends = std::mem::take(&mut plan.sends);
    let mut dev = std::mem::take(&mut plan.dev);
    let mut dev_end: Option<(DevServer, DevChannel)> = if dev.is_empty() {
        None
    } else {
        let strings =
            super::strings::TableStrings::load(live.archives.as_ref(), super::strings::LANG)
                .map_err(anyhow::Error::msg)?;
        let catalog = super::dev::catalog(live, &strings);
        Some((
            DevServer::new(catalog),
            Arc::new(Mutex::new(DevQueue::default())),
        ))
    };
    link.set_before_pump(move |l: &mut single_player::Link<StepClock>| {
        pokes.run_due(l);
        let frame = l.host().game.game.frame;
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut sends)
            .into_iter()
            .partition(|(f, _)| frame >= f - 1);
        sends = later;
        for (i, (f, msg)) in due.into_iter().enumerate() {
            let r = super::send::inject_now(l, &msg);
            let src = crate::scenario_script::message_text(&msg).unwrap_or_default();
            eprintln!("{}", super::send::record_line(f, i, &src, &r));
            let (ok, result) = match &r {
                crate::bridge::inject::Injected::Sent { queued: true, .. } => (true, "ok".into()),
                crate::bridge::inject::Injected::Sent { why, .. } => (
                    false,
                    format!("dropped: {}", why.clone().unwrap_or_default()),
                ),
                crate::bridge::inject::Injected::Unresolved(u) => {
                    (false, format!("unresolved {}: {}", u.reference, u.why))
                }
            };
            push(&log, script_tick(start, f), "msg", src, ok, result);
        }
        if let Some((server, chan)) = dev_end.as_mut() {
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut dev)
                .into_iter()
                .partition(|(f, _)| frame >= f - 1);
            dev = later;
            let mut asked = Vec::new();
            if let Ok(mut q) = chan.lock() {
                for (f, line) in due {
                    if let Ok(Some(cmd)) = d2_proto::dev::parse(&line) {
                        q.commands.push(cmd);
                        asked.push((f, line));
                    }
                }
            }
            server.run_due(chan, l);
            let replies = chan
                .lock()
                .map(|mut q| std::mem::take(&mut q.replies))
                .unwrap_or_default();
            for ((f, line), r) in asked.into_iter().zip(replies) {
                eprintln!("dev: after frame {frame}: {line}: {}", r.text);
                push(&log, script_tick(start, f), "dev", line, r.ok, r.text);
            }
        }
    })
    .map_err(|e| anyhow!("{e:?}"))
}

/// The checks and queries still to answer. They read the client's
/// `state` reply after a tick, and only at the ticks one is due (with the
/// tick before an `at` check, for `changes`).
struct Probes {
    checks: Vec<PlannedCheck>,
    queries: Vec<query::Name>,
    query_at: Vec<u32>,
    /// Each query tick's reply: `{"at": T, "values": {...}}`.
    answers: Vec<Value>,
    /// The reply of the tick before, if it was read.
    prev: Option<(u32, Value)>,
}

impl Probes {
    /// Whether script tick `t` needs a `state` reply.
    fn wants(&self, t: u32) -> bool {
        self.query_at.contains(&t)
            || self
                .checks
                .iter()
                .any(|c| if c.by { t <= c.tick } else { t + 1 >= c.tick })
    }

    /// Answers what is due at script tick `t` from `state`.
    fn observe(&mut self, t: u32, state: Value, log: &StepLog) {
        let prev = self
            .prev
            .take()
            .filter(|(pt, _)| pt + 1 == t)
            .map(|(_, v)| v);
        self.checks.retain(|c| {
            if !c.by && t < c.tick {
                return true;
            }
            let (ok, v) = c.check.eval(&state, prev.as_ref());
            if c.by && !ok && t < c.tick {
                return true;
            }
            let result = match (ok, c.by) {
                (true, _) => format!("ok: {v}"),
                (false, false) => format!("actual: {v}"),
                (false, true) => format!("never held by tick {}; last: {v}", c.tick),
            };
            let kw = if c.by { "by" } else { "at" };
            let text = format!("{kw} {} check {}", c.tick, c.text);
            push(log, Some(t), "check", text, ok, result);
            false
        });
        if self.query_at.contains(&t) {
            self.answers.push(json!({
                "at": t,
                "values": query::query(&self.queries, &state),
            }));
        }
        self.prev = Some((t, state));
    }

    /// Reads the state at script tick `t` if anything is due.
    fn tick(&mut self, host: &mut Host, t: u32, log: &StepLog) -> Result<()> {
        if self.wants(t) {
            let state = host.state()?;
            self.observe(t, state, log);
        }
        Ok(())
    }
}

/// The input script's side of script tick `t`, after the tick ran: the
/// steps that could not run become failed `input` outcomes (a `clickunit`
/// with no such unit, a skill the player lacks, a `wait-until` that never
/// held), and a `wait-until` the script is paused on has its check read
/// from the client's `state` reply (`prev`: the reply of the tick before,
/// while the wait lasts, for `changes`). A check that holds lets the
/// script go on from the next tick.
fn input_tick(
    host: &mut Host,
    t: u32,
    log: &StepLog,
    prev: &mut Option<(u32, Value)>,
) -> Result<()> {
    use crate::world_view::input_script::InputScript;
    let world = host.app_mut().world_mut();
    let Some(mut script) = world.get_resource_mut::<InputScript>() else {
        return Ok(());
    };
    for why in script.take_failures() {
        push(log, Some(t), "input", why, false, "did not run".into());
    }
    let Some(wait) = script.waiting().cloned() else {
        *prev = None;
        return Ok(());
    };
    let state = host.state()?;
    let check = query::parse_check(&wait.expr).map_err(anyhow::Error::msg)?;
    let before = prev.take().filter(|(pt, _)| pt + 1 == t).map(|(_, v)| v);
    let (ok, v) = check.eval(&state, before.as_ref());
    if ok {
        if let Some(mut script) = host.app_mut().world_mut().get_resource_mut::<InputScript>() {
            script.wait_met();
        }
        let text = format!("wait-until {} max {}", wait.expr, wait.max);
        push(log, Some(t), "input", text, true, format!("held: {v}"));
    } else {
        *prev = Some((t, state));
    }
    Ok(())
}

fn conditional_keys(
    host: &mut Host,
    keys: &mut VecDeque<(u32, u32, String)>,
    tick: u32,
    log: &StepLog,
) -> Result<()> {
    while keys.front().is_some_and(|(at, _, _)| *at <= tick) {
        let (_, key, expr) = keys.pop_front().expect("due key");
        let check = query::parse_check(&expr).map_err(anyhow::Error::msg)?;
        let (held, actual) = check.eval(&host.state()?, None);
        if held {
            host.run(super::control::protocol::Command::Key(key))?;
        }
        push(
            log,
            Some(tick),
            "key-if",
            format!("key-if {key} {expr}"),
            true,
            format!("{}: {actual}", if held { "sent" } else { "skipped" }),
        );
    }
    Ok(())
}

fn push(
    log: &StepLog,
    at: Option<u32>,
    kind: &'static str,
    text: String,
    ok: bool,
    result: String,
) {
    if let Ok(mut l) = log.lock() {
        l.push(StepOutcome {
            at,
            kind,
            text,
            ok,
            result,
        });
    }
}

/// Loaded game data retained across scenarios. Each run creates a fresh server
/// game and client join, discarding all mutable state from the preceding run.
pub struct Session {
    data: GameData,
}
impl Session {
    /// Load the install once for this worker.
    pub fn load(game_dir: Option<&Path>) -> Result<Self, ReproError> {
        let dir = game_dir
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from))
            .ok_or_else(|| ReproError::NoGame("give --game-dir or set D2_GAME_DIR".into()))?;
        let data = GameData::select(Some(&dir)).map_err(|e| ReproError::NoGame(format!("{e}")))?;
        Ok(Self { data })
    }
    /// Retain already loaded tables, including an explicit mod layer selection.
    pub fn from_data(data: GameData) -> Self {
        Self { data }
    }
    fn start_host(
        &self,
        s: &Scenario,
        opts: &RunOpts,
    ) -> Result<(Host, Arc<LiveData>, D2s), ReproError> {
        let GameData::Live(live) = self.data.clone();
        let difficulty = s.difficulty.index();
        let character = match (&s.save_file, &s.character) {
            (Some(f), _) => {
                let path = resolve_path(&opts.base_dir, f);
                if !path.is_file() {
                    return Err(ReproError::Invalid(format!(
                        "save {}: no such file",
                        path.display()
                    )));
                }
                single_player::load_character(&self.data, &path, difficulty)
                    .map_err(|e| anyhow!(e))?
            }
            (None, Some(c)) => inline_character(c, &live, difficulty)?,
            _ => {
                return Err(ReproError::Invalid(
                    "no character: save or inline char required".into(),
                ))
            }
        };
        let base = super::save::base_save(&character);
        let host =
            Host::start_with_coverage(self.data.clone(), character, None, false, Some(s.seed))?;
        Ok((host, live, base))
    }
    /// Run a scenario with fresh mutable state and the retained data.
    pub fn run(&mut self, s: &Scenario, opts: &RunOpts) -> Result<ReproReport, ReproError> {
        invariants::run(s, opts, || {
            validate(s)?;
            run_loaded(self, s, opts, std::time::Instant::now())
        })
    }
}

/// `f` against `base` unless it is absolute.
fn resolve_path(base: &Path, f: &str) -> PathBuf {
    let p = PathBuf::from(f);
    if p.is_absolute() {
        p
    } else {
        base.join(p)
    }
}

/// Check runner support before loading any data.
fn validate(s: &Scenario) -> Result<(), ReproError> {
    if s.version != SCENARIO_FORMAT_V2 {
        return Err(ReproError::Invalid(
            "repro runs `scenario 2` scripts (the first line `scenario 2`)".into(),
        ));
    }
    if !s.expansion {
        return Err(ReproError::Invalid(
            "`expansion no`: repro runs expansion games only".into(),
        ));
    }
    if let Some(c) = &s.character {
        if let Some(what) = unsupported_inline(c) {
            return Err(ReproError::Invalid(format!(
                "repro cannot build {what} yet"
            )));
        }
    }
    Ok(())
}
/// Runs a standalone scenario, including its data load in wall time.
pub fn run(s: &Scenario, opts: &RunOpts) -> Result<ReproReport, ReproError> {
    invariants::run(s, opts, || {
        validate(s)?;
        let wall = std::time::Instant::now();
        let session = Session::load(opts.game_dir.as_deref())?;
        run_loaded(&session, s, opts, wall)
    })
}
fn run_loaded(
    session: &Session,
    s: &Scenario,
    opts: &RunOpts,
    wall: std::time::Instant,
) -> Result<ReproReport, ReproError> {
    let source = s;
    if let Some(t) = opts.query_at.iter().find(|&&t| t > source.end) {
        return Err(ReproError::Invalid(format!(
            "--at {t} is after `end {}` at {} Hz",
            source.end, source.tick_rate
        )));
    }
    let timed = game_timing(source)?;
    let s = &timed;
    let (mut host, live, base) = session.start_host(s, opts)?;
    let mut outcomes: Vec<StepOutcome> = Vec::new();
    if let Some(c) = &s.character {
        for (kind, text) in inline_setup(c) {
            outcomes.push(run_setup(&host, kind, &text)?);
        }
    }
    host.step(SETTLE_TICKS as u32);
    let start = host.server_frame()?;
    let mut p = plan(s, start)?;
    if !p.input.is_empty() {
        let steps = crate::world_view::input_script::parse(&p.input)
            .map_err(|e| ReproError::Invalid(format!("input: {e}")))?;
        if crate::world_view::input_script::uses_skills(&steps) {
            // `skill` / `cast` steps name skills by the player's `skills.txt`.
            if let Some(names) =
                crate::world_view::script_skills::SkillNames::load(live.archives.as_ref())
                    .map_err(|e| ReproError::Run(anyhow!("skill names: {e}")))?
            {
                host.app_mut().insert_resource(names);
            }
        }
        host.app_mut()
            .insert_resource(crate::world_view::input_script::InputScript::new(steps));
    }
    let pokes_log = ResultLog::default();
    let log = StepLog::default();
    let query_at = if opts.queries.is_empty() {
        Vec::new()
    } else if opts.query_at.is_empty() {
        vec![s.end]
    } else {
        opts.query_at
            .iter()
            .map(|&t| game_ticks(t, source.tick_rate))
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut probes = Probes {
        checks: std::mem::take(&mut p.checks),
        queries: opts.queries.clone(),
        query_at,
        answers: Vec::new(),
        prev: None,
    };
    install(&host, &mut p, start, &live, pokes_log.clone(), log.clone())?;
    let mut keys = VecDeque::from(std::mem::take(&mut p.conditional_keys));
    let mut generic = invariants::Checks::default();
    generic.tick(&mut host, s, 0, &log, &[])?;
    checkpoint::tick(&host, session, &base, s, &p.checkpoints, 0, &log);
    probes.tick(&mut host, 0, &log)?;
    conditional_keys(&mut host, &mut keys, 0, &log)?;
    let mut wait_prev = None;
    input_tick(&mut host, 0, &log, &mut wait_prev)?;
    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL_TICKS);
    let limit = host.passes() + u64::from(s.end) + SLACK_PASSES;
    let mut frame = start;
    while frame < p.end_frame {
        if host.passes() >= limit {
            return Err(ReproError::Run(anyhow!(
                "the server stopped ticking at frame {frame} (end frame {})",
                p.end_frame
            )));
        }
        host.step(1);
        frame = host.server_frame()?;
        if let Ok(t) = u32::try_from(frame - start) {
            generic.tick(&mut host, s, t, &log, &gotos_finished(&pokes_log, start))?;
            checkpoint::tick(&host, session, &base, s, &p.checkpoints, t, &log);
            probes.tick(&mut host, t, &log)?;
            conditional_keys(&mut host, &mut keys, t, &log)?;
            input_tick(&mut host, t, &log, &mut wait_prev)?;
        }
        if p.end_frame - frame < TAIL_TICKS as i32 {
            let snap = host
                .server()
                .lock()
                .map_err(|_| anyhow!("server lock"))?
                .state_snapshot()
                .map_err(|e| anyhow!("{e:?}"))?;
            if tail.len() == TAIL_TICKS {
                tail.pop_front();
            }
            tail.push_back(snap.to_json_line());
        }
        if !opts.keep_going && failed(&outcomes, &pokes_log, &log) {
            break;
        }
    }
    // A `wait-until` still waiting when the run ends never held.
    if let Some(w) = host
        .app_mut()
        .world()
        .get_resource::<crate::world_view::input_script::InputScript>()
        .and_then(|s| s.waiting())
    {
        let text = format!("wait-until {} max {}", w.expr, w.max);
        let at = u32::try_from(frame - start).ok();
        push(&log, at, "input", text, false, "the run ended first".into());
    }
    // Every result in the order it ran: setup, then the steps by tick.
    let mut steps = outcomes;
    let mut ran: Vec<StepOutcome> = pokes_log
        .lock()
        .map(|l| {
            l.iter()
                .map(|e| StepOutcome {
                    at: script_tick(start, e.frame + 1),
                    kind: "poke",
                    text: e.op.clone(),
                    ok: e.code == "ok",
                    result: match &e.note {
                        Some(n) => format!("{} ({n})", e.code),
                        None => e.code.to_owned(),
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    ran.extend(log.lock().map(|l| l.clone()).unwrap_or_default());
    ran.sort_by_key(|o| o.at);
    steps.extend(ran);
    let first_failure = steps.iter().find(|o| !o.ok).map(StepOutcome::json);
    let exit = if first_failure.is_some() {
        exit::FAIL
    } else {
        exit::PASS
    };
    let state_end = if opts.capture_state {
        Some(
            host.server()
                .lock()
                .map_err(|_| anyhow!("server lock"))?
                .state_snapshot()
                .map_err(|e| anyhow!("{e:?}"))?
                .to_json_line(),
        )
    } else {
        None
    };
    let client = host
        .state()
        .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
    let json = json!({
        "format": FORMAT,
        "tick_rate": d2_sim::tick::TickRate::GAME.hz(),
        "build": crate::launch::VERSION,
        "scenario": s.name,
        "file": opts.file,
        "sha256": source.sha256(),
        "scenario_tick_rate": source.tick_rate,
        "scenario_end": source.end,
        "verdict": if exit == exit::PASS { "pass" } else { "fail" },
        "exit": exit,
        "first_failure": first_failure,
        "seed": s.seed,
        "start_frame": start,
        "end": s.end,
        "ticks": frame - start,
        "steps": steps.iter().map(StepOutcome::json).collect::<Vec<_>>(),
        "coverage": super::coverage::snapshot(&mut host)?,
        "violations": generic.violations,
        "allowed_rules": s.allow,
        "queries": probes.answers,
        "wall_ms": wall.elapsed().as_millis() as u64,
    });
    write_artifacts(&opts.out, &json, &tail, &client, state_end.as_deref())?;
    Ok(ReproReport { exit, json })
}

/// Whether a step failed so far.
fn failed(setup: &[StepOutcome], pokes: &ResultLog, log: &StepLog) -> bool {
    setup.iter().any(|o| !o.ok)
        || pokes.lock().is_ok_and(|l| l.iter().any(|e| e.code != "ok"))
        || log.lock().is_ok_and(|l| l.iter().any(|o| !o.ok))
}

fn write_artifacts(
    out: &Path,
    report: &Value,
    tail: &VecDeque<String>,
    client: &Value,
    state_end: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let write = |name: &str, text: String| {
        let p = out.join(name);
        std::fs::write(&p, text).with_context(|| format!("write {}", p.display()))
    };
    write(
        "repro.json",
        format!("{}\n", serde_json::to_string_pretty(report)?),
    )?;
    write(
        "state-tail.jsonl",
        std::iter::once(format!(
            "{}\n",
            json!({
                "k": "header", "format": "state-1", "tick_rate": d2_sim::tick::TickRate::GAME.hz(),
            })
        ))
        .chain(tail.iter().map(|l| format!("{l}\n")))
        .collect(),
    )?;
    let mut client = client.clone();
    client["tick_rate"] = d2_sim::tick::TickRate::GAME.hz().into();
    write(
        "client-state.json",
        format!("{}\n", serde_json::to_string_pretty(&client)?),
    )?;
    match state_end {
        Some(line) => write("state-end.jsonl", format!("{line}\n"))?,
        None => {
            // A run without capture leaves no stale boundary for the next one.
            let _ = std::fs::remove_file(out.join("state-end.jsonl"));
        }
    }
    Ok(())
}

/// The options after `repro FILE`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliArgs {
    pub file: PathBuf,
    pub batch: Option<PathBuf>,
    pub jobs: usize,
    pub out: Option<PathBuf>,
    pub game_dir: Option<PathBuf>,
    pub native: Option<PathBuf>,
    pub mod_name: Option<String>,
    pub keep_going: bool,
    /// `--query`: the names as written.
    pub query: Option<String>,
    /// `--at`: the query ticks.
    pub at: Vec<u32>,
}

/// The usage line.
const USAGE: &str =
    "usage: repro FILE | --batch DIR|LIST [--jobs N] [--out DIR] [--game-dir DIR | --native DIR [--mod NAME]] [--keep-going] \
                     [--query \"NAME, ...\" [--at T,...]]";

/// Parses `repro FILE [--out DIR] [--game-dir DIR] [--keep-going]
/// [--query "NAME, ..." [--at T,...]]`.
pub fn parse_args(args: &[String]) -> Result<CliArgs, String> {
    let mut a = CliArgs::default();
    let mut file = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--batch" => a.batch = Some(PathBuf::from(value()?)),
            "--jobs" => {
                a.jobs = value()?
                    .parse()
                    .map_err(|_| "--jobs needs a positive integer")?;
                if a.jobs == 0 {
                    return Err("--jobs needs a positive integer".into());
                }
            }
            "--out" => a.out = Some(PathBuf::from(value()?)),
            "--game-dir" => a.game_dir = Some(PathBuf::from(value()?)),
            "--native" => a.native = Some(PathBuf::from(value()?)),
            "--mod" => a.mod_name = Some(value()?),
            "--keep-going" => a.keep_going = true,
            "--query" => a.query = Some(value()?),
            "--at" => {
                a.at = value()?
                    .split(',')
                    .map(|t| {
                        t.trim()
                            .parse()
                            .map_err(|_| format!("--at `{t}`: a tick number"))
                    })
                    .collect::<Result<_, _>>()?
            }
            f if !f.starts_with("--") && file.is_none() => file = Some(PathBuf::from(f)),
            other => return Err(format!("repro: unknown option {other}")),
        }
    }
    if a.batch.is_some() {
        if file.is_some() {
            return Err("use FILE or --batch, not both".into());
        }
    } else {
        if a.jobs != 0 {
            return Err("--jobs needs --batch".into());
        }
        a.file = file.ok_or(USAGE)?;
    }
    if !a.at.is_empty() && a.query.is_none() {
        return Err("--at needs --query".into());
    }
    if a.native.is_some() && a.game_dir.is_some() {
        return Err("choose --native or --game-dir".into());
    }
    if a.mod_name.is_some() && a.native.is_none() {
        return Err("--mod requires --native".into());
    }
    if a.mod_name.as_ref().is_some_and(|n| {
        n.is_empty()
            || !n
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }) {
        return Err("--mod needs a single folder name".into());
    }
    Ok(a)
}

fn selected_data(a: &CliArgs) -> Result<Option<GameData>, ReproError> {
    a.native
        .as_ref()
        .map(|dir| {
            GameData::select_native_with(
                dir,
                &single_player::Layers::List(a.mod_name.iter().cloned().collect()),
            )
            .map_err(|e| ReproError::NoGame(e.to_string()))
        })
        .transpose()
}

/// `d2-client repro...`: runs the scenario, prints one verdict line and returns
/// the exit code ([`exit`]).
pub fn main(args: &[String]) -> u8 {
    let a = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return exit::INVALID;
        }
    };
    if a.batch.is_some() {
        return batch::main(&a);
    }
    let text = match std::fs::read_to_string(&a.file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("repro: {}: {e}", a.file.display());
            return exit::INVALID;
        }
    };
    if text
        .lines()
        .next()
        .is_some_and(|l| l.trim() == soak_scenario::FORMAT)
    {
        if a.native.is_some() {
            eprintln!("repro: soak scenarios do not support --native/--mod");
            return exit::INVALID;
        }
        return soak_scenario::main(&text, &a);
    }
    let queries = match a.query.as_deref().map(query::parse_names).transpose() {
        Ok(q) => q.unwrap_or_default(),
        Err(e) => {
            println!("repro: INVALID --query: {e}");
            return exit::INVALID;
        }
    };
    let s = match Scenario::parse(&text) {
        Ok(s) => s,
        Err(e) => {
            println!("repro: INVALID {}: {e}", a.file.display());
            return exit::INVALID;
        }
    };
    let opts = RunOpts {
        out: a
            .out
            .clone()
            .unwrap_or_else(|| PathBuf::from("repro-out").join(&s.name)),
        game_dir: a.game_dir.clone(),
        base_dir: a.file.parent().map(Path::to_path_buf).unwrap_or_default(),
        keep_going: a.keep_going,
        file: a.file.display().to_string(),
        queries,
        query_at: a.at.clone(),
        capture_state: false,
    };
    let result = selected_data(&a).and_then(|data| match data {
        Some(data) => Session::from_data(data).run(&s, &opts),
        None => run(&s, &opts),
    });
    match result {
        Ok(r) => {
            let j = &r.json;
            for q in j["queries"].as_array().into_iter().flatten() {
                println!("{q}");
            }
            let why = j["first_failure"]
                .as_object()
                .map(|f| {
                    format!(
                        " at {}: {} {} -> {}",
                        f["at"], f["kind"], f["text"], f["result"]
                    )
                })
                .unwrap_or_default();
            println!(
                "repro: {} {} ({} ticks, {} ms){why}; artifacts in {}",
                if r.exit == exit::PASS { "PASS" } else { "FAIL" },
                s.name,
                j["ticks"],
                j["wall_ms"],
                opts.out.display()
            );
            r.exit
        }
        Err(e) => {
            let word = match e.exit_code() {
                exit::INVALID => "INVALID",
                exit::NO_GAME => "NO GAME",
                _ => "ERROR",
            };
            println!("repro: {word} {}: {e}", s.name);
            e.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scn(text: &str) -> Scenario {
        let mut s = Scenario::parse(text).unwrap();
        s.tick_rate = 60;
        s
    }

    #[test]
    fn scenario_rate_preserves_deadlines_waits_and_legacy_meaning() {
        let text = "scenario 2\nname timed\nend 70\nchar class 1\nat 16 input wait-until player.mode == walk max 25\nby 25 check player.mode == walk\n";
        let legacy = Scenario::parse(text).unwrap();
        assert_eq!(legacy.tick_rate, 25);
        let explicit =
            Scenario::parse(&text.replace("scenario 2", "scenario 2\ntick_rate 25")).unwrap();
        assert_eq!(legacy, explicit);
        assert_eq!(Scenario::parse(&explicit.to_text()).unwrap(), explicit);
        let p = plan(&legacy, 100).unwrap();
        assert_eq!(p.end_frame, 268);
        assert_eq!(p.input, "frame 140; wait-until player.mode == walk max 60");
        assert_eq!(p.checks[0].tick, 60);
        assert_eq!(game_ticks(1, 25).unwrap(), 3);
        assert_eq!(game_ticks(25, 60).unwrap(), 25);
        let held = Scenario::parse(
            &text.replace("wait-until player.mode == walk max 25", "hold 400 300 25"),
        )
        .unwrap();
        assert_eq!(
            game_timing(&held).unwrap().steps[0].msg,
            StepMsg::Input("hold 400 300 60".into())
        );
        let snapshots = Scenario::parse(&text.replace(
            "char class 1",
            "record units\nsnapshot every 1\nchar class 1",
        ))
        .unwrap();
        let timed = game_timing(&snapshots).unwrap();
        assert_eq!(&timed.snapshot_at[..4], &[0, 3, 5, 8]);
        assert_eq!(timed.snapshot_at.last(), Some(&168));
        assert_eq!(game_timing(&timed).unwrap(), timed);
        assert!(game_ticks(u32::MAX, 1).is_err());
        assert!(game_ticks(1, 0).is_err());
        for header in [
            "tick_rate 0",
            "tick_rate 1001",
            "tick_rate 25\ntick_rate 60",
        ] {
            assert!(
                Scenario::parse(&text.replace("scenario 2", &format!("scenario 2\n{header}")))
                    .is_err()
            );
        }
    }

    #[test]
    fn plan_sorts_steps_into_channels_from_tick_zero() {
        let s = scn("scenario 2\nname t\nend 30\nchar class 1\nat 0 poke warp 2\nat 2 msg Walk x=@x+1 y=@y\nat 3 dev god on\nat 5 input rclickunit 1 *\nat 5 input click 400 300\nat 9 spawn 0 @x+12 @y normal\n");
        let p = plan(&s, 100).unwrap();
        assert_eq!(p.end_frame, 130);
        assert_eq!(p.pokes.len(), 2);
        assert_eq!(p.pokes[0].when, When::Frame(101));
        assert_eq!(p.pokes[0].op.to_string(), "warp 2");
        assert_eq!(p.pokes[1].when, When::Frame(110));
        assert!(matches!(p.pokes[1].op, d2_sim::poke::PokeOp::Spawn(_)));
        assert_eq!(p.sends.len(), 1);
        assert_eq!(p.sends[0].0, 103);
        assert_eq!(p.dev, [(104, "god on".to_owned())]);
        assert_eq!(p.input, "frame 106; rclickunit 1 *; click 400 300");
        crate::world_view::input_script::parse(&p.input).unwrap();
        assert_eq!(script_tick(100, 106), Some(5));
        assert_eq!(script_tick(100, 100), None);
    }

    #[test]
    fn spawn_steps_become_the_same_poke() {
        let s =
            scn("scenario 2\nname t\nend 5\nchar class 0\nat 1 spawn 19 @x+4 @y champion umod 5\n");
        assert_eq!(
            spawn_text(&s.steps[0].msg),
            "spawn 19 @x+4 @y champion umod 5"
        );
    }

    #[test]
    fn inline_setup_raises_sets_warps_and_places() {
        let s = scn("scenario 2\nname t\nend 5\nchar class 1\nchar level 30\nchar area 0 2\nchar at 5000 6000\nchar stat 14 500\n");
        let c = s.character.unwrap();
        assert_eq!(
            inline_setup(&c),
            [
                ("dev", "level 30".to_owned()),
                ("poke", "stat @player 14 0 500".to_owned()),
                ("poke", "warp 2".to_owned()),
                ("poke", "pos @player 5000 6000".to_owned()),
            ]
        );
        // In the act's town at level 1: nothing to do.
        let s = scn("scenario 2\nname t\nend 5\nchar class 1\nchar area 1 40\n");
        assert!(inline_setup(&s.character.unwrap()).is_empty());
        // Each setup poke is a valid directive.
        for t in ["stat @player 14 0 500", "warp 2", "pos @player 5000 6000"] {
            let toks: Vec<&str> = t.split_whitespace().collect();
            d2_sim::poke::parse_op(&toks).unwrap();
        }
    }

    #[test]
    fn unsupported_inline_lines_are_invalid() {
        let s = scn("scenario 2\nname t\nend 5\nchar class 1\nchar item hp1 belt 0\n");
        assert!(unsupported_inline(s.character.as_ref().unwrap())
            .unwrap()
            .contains("char item"));
        let e = run(&s, &RunOpts::default()).unwrap_err();
        assert_eq!(e.exit_code(), exit::INVALID);
        let mut v1 = scn("scenario 2\nname t\nend 10\nchar class 1\nchar area 0 1\n");
        v1.version = crate::scenario_script::SCENARIO_FORMAT_VERSION;
        assert_eq!(
            run(&v1, &RunOpts::default()).unwrap_err().exit_code(),
            exit::INVALID
        );
    }

    #[test]
    fn a_quest_line_is_buildable() {
        let s = scn("scenario 2\nname t\nend 5\nchar class 1\nchar quest 39 0x8000\n");
        assert!(unsupported_inline(s.character.as_ref().unwrap()).is_none());
    }

    #[test]
    fn a_missing_game_folder_is_exit_3() {
        let s = scn("scenario 2\nname t\nend 5\nchar class 1\n");
        let opts = RunOpts {
            game_dir: Some(PathBuf::from("this/folder/does/not/exist")),
            ..RunOpts::default()
        };
        assert_eq!(run(&s, &opts).unwrap_err().exit_code(), exit::NO_GAME);
    }

    #[test]
    fn cli_options_parse() {
        let a: Vec<String> = ["x.scn", "--out", "o", "--keep-going"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = parse_args(&a).unwrap();
        assert_eq!(c.file, PathBuf::from("x.scn"));
        assert_eq!(c.out, Some(PathBuf::from("o")));
        assert!(c.keep_going);
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["x".into(), "--fly".into()]).is_err());
        let args = |a: &[&str]| parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let c = args(&["x.scn", "--query", "player.pos, hud.life", "--at", "40,80"]).unwrap();
        assert_eq!(c.query.as_deref(), Some("player.pos, hud.life"));
        assert_eq!(c.at, [40, 80]);
        assert!(args(&["x.scn", "--at", "4"]).is_err(), "--at needs --query");
        assert!(args(&["x.scn", "--query", "a", "--at", "4,x"]).is_err());
    }

    #[test]
    fn checkpoint_deadlines_are_retimed_like_checks() {
        let s = Scenario::parse("scenario 2\ntick_rate 25\nname t\nend 25\nchar class 0\nat 10 checkpoint\nat 25 checkpoint\n").unwrap();
        assert_eq!(plan(&s, 100).unwrap().checkpoints, [24, 60]);
    }

    #[test]
    fn plan_keeps_the_checks_in_script_order() {
        let s = scn("scenario 2\nname t\nend 30\nchar class 1\nat 0 check ui.open(inventory)\nby 30 check player.mode == cast\n");
        let p = plan(&s, 100).unwrap();
        let got: Vec<(u32, bool, &str)> = p
            .checks
            .iter()
            .map(|c| (c.tick, c.by, c.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (0, false, "ui.open(inventory)"),
                (30, true, "player.mode == cast")
            ]
        );
    }

    /// A `state` reply whose player is in mode `m` with `life` points.
    fn state(m: u32, life: i32) -> Value {
        json!({
            "local": 1,
            "snap": {"units": [{"ut": 0, "g": 1, "m": m, "hp": life << 8}]},
            "hud": {"life": life},
        })
    }

    fn probes(script: &str, queries: &str, at: &[u32]) -> Probes {
        let s = scn(&format!(
            "scenario 2\nname t\nend 20\nchar class 1\n{script}"
        ));
        Probes {
            checks: plan(&s, 0).unwrap().checks,
            queries: query::parse_names(queries).unwrap_or_default(),
            query_at: at.to_vec(),
            answers: Vec::new(),
            prev: None,
        }
    }

    /// Feeds `states` as script ticks 0, 1, ... (only the wanted ones) and
    /// returns the check outcomes as (tick, ok, result).
    fn feed(p: &mut Probes, states: &[Value]) -> Vec<(Option<u32>, bool, String)> {
        let log = StepLog::default();
        for (t, st) in states.iter().enumerate() {
            let t = t as u32;
            if p.wants(t) {
                p.observe(t, st.clone(), &log);
            }
        }
        let l = log.lock().unwrap();
        l.iter().map(|o| (o.at, o.ok, o.result.clone())).collect()
    }

    #[test]
    fn at_checks_read_their_tick_and_report_the_actual_value() {
        let mut p = probes(
            "at 1 check player.mode == cast\nat 2 check hud.life == 40\n",
            "",
            &[],
        );
        let got = feed(&mut p, &[state(1, 50), state(10, 50), state(10, 45)]);
        assert_eq!(
            got,
            [
                (Some(1), true, "ok: \"cast\"".to_owned()),
                (Some(2), false, "actual: 45".to_owned()),
            ]
        );
        assert!(p.checks.is_empty() && !p.wants(3));
    }

    #[test]
    fn by_checks_pass_at_the_first_tick_they_hold_or_fail_at_their_tick() {
        let mut p = probes(
            "by 3 check player.mode == cast\nby 3 check hud.life < 10\n",
            "",
            &[],
        );
        let got = feed(
            &mut p,
            &[state(1, 50), state(2, 50), state(10, 50), state(1, 40)],
        );
        assert_eq!(
            got,
            [
                (Some(2), true, "ok: \"cast\"".to_owned()),
                (Some(3), false, "never held by tick 3; last: 40".to_owned()),
            ]
        );
    }

    #[test]
    fn changes_reads_the_tick_before_and_queries_answer_their_ticks() {
        let mut p = probes(
            "at 2 check player.life changes\nat 3 check player.life changes\n",
            "player.mode, hud.life",
            &[0, 3],
        );
        let got = feed(
            &mut p,
            &[state(1, 50), state(1, 50), state(1, 50), state(2, 49)],
        );
        assert_eq!(
            got,
            [
                (Some(2), false, "actual: 50".to_owned()),
                (Some(3), true, "ok: 49".to_owned()),
            ]
        );
        assert_eq!(
            p.answers,
            [
                json!({"at": 0, "values": {"player.mode": "neutral", "hud.life": 50}}),
                json!({"at": 3, "values": {"player.mode": "walk", "hud.life": 49}}),
            ]
        );
    }
}
