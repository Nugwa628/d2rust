//! The scenario format (`.mps`): one multiplayer situation, written as
//! lines a person can read.
//!
//! ```text
//! # Two players see each other and one leaves.
//! name   join-leave
//! about  Two bots enter one game; each sees the other; a leave is seen.
//! rule   SEE   every player in a game sees every other player in it
//! rule   LEAVE a player that leaves is gone for the others
//! bots   2
//! server players=8 grace=0
//! net    latency=0
//!
//! all  connect                     expect ok
//! all  enter game1                 expect ok
//! b0   see players=2               within 3s  rule SEE
//! b1   leave
//! b0   see players=1               within 3s  rule LEAVE
//! ```
//!
//! **Header lines** (before or between steps; each at most once except
//! `rule`, `needs`):
//! - `name ID`, `about TEXT`: the scenario's id and what it shows.
//! - `rule ID TEXT`: a stated rule. Every failure names the rule it
//!   breaks: the step's `rule ID`, or the scenario's only rule. A step
//!   that expects more than `ok` names its rule when there are several;
//!   a setup step that fails breaks `core.step`.
//! - `bots N` (1..=64): how many bot clients.
//! - `server KEY=VALUE..`: the fresh server's settings (`players`,
//!   `grace` in ms, `build`).
//! - `net KEY=VALUE..`: the starting network conditions for every bot.
//! - `addresses per-bot|shared`: whether each bot's relay has its own
//!   loopback address (the default; contracts 10.1) or all bots share
//!   127.0.0.1 (for a per-IP limit).
//! - `needs CAP..`: capabilities beyond those its steps imply.
//! - `pending TEXT`: written ahead of the feature; reported, never run.
//! - `seed N`: the seed of the injected loss and jitter.
//!
//! **Steps:** `WHO VERB ARGS.. [expect PRED..] [within DURATION] [rule ID]`.
//! `WHO` is `bN`, `bA-B` (a range), `all`, `server`, `net` or `wait`.
//! - `server start|stop|kill|restart`: `stop` is a clean stop (SIGTERM
//!   where there is one), `kill` a crash.
//! - `server admin ARGS..`: runs the server program once with its config
//!   and these arguments (an offline admin command), expects exit 0.
//! - `server dev CHARACTER COMMAND..`: a dev console command for that
//!   character's game, on a test server (one stdin line).
//! - `net [bN|bA-B|all] KEY=VALUE..` (`latency`, `jitter` in ms, `loss`
//!   in percent, `cut=on|off`)
//! - `wait DURATION`
//!
//! Predicates: `ok`, `refused`, `refused=CODE`, `error`, `closed`,
//! `KEY=VALUE`, `KEY!=VALUE`, `KEY>=N`, `KEY<=N`.
//! Durations: `500ms`, `3s`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The most bots one scenario may drive.
pub const MAX_BOTS: usize = 64;

/// A parse error, with the line it is on.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{file}:{line}: {msg}")]
pub struct ParseError {
    /// The scenario file.
    pub file: String,
    /// The line number, from 1 (0: the whole file).
    pub line: usize,
    /// What is wrong.
    pub msg: String,
}

/// A stated rule a scenario checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Its id, unique in the scenario.
    pub id: String,
    /// The rule in words.
    pub text: String,
}

/// Who a step is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    /// These bots (indices, ascending).
    Bots(Vec<usize>),
    /// The server process.
    Server,
    /// The network between bots and server.
    Net(Vec<usize>),
    /// Nobody: the clock.
    Wait,
}

/// One expectation on a step's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pred {
    /// The action succeeded.
    Ok,
    /// It was refused (with this code, when given).
    Refused(Option<String>),
    /// It failed with an error that isn't a refusal.
    Error,
    /// The connection is closed.
    Closed,
    /// A fact compares to a value.
    Fact {
        /// The fact's key.
        key: String,
        /// How it compares.
        op: Op,
        /// The value it compares to.
        value: String,
    },
}

/// A fact comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `>=` (numbers)
    Ge,
    /// `<=` (numbers)
    Le,
}

impl std::fmt::Display for Pred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pred::Ok => write!(f, "ok"),
            Pred::Refused(None) => write!(f, "refused"),
            Pred::Refused(Some(c)) => write!(f, "refused={c}"),
            Pred::Error => write!(f, "error"),
            Pred::Closed => write!(f, "closed"),
            Pred::Fact { key, op, value } => {
                let op = match op {
                    Op::Eq => "=",
                    Op::Ne => "!=",
                    Op::Ge => ">=",
                    Op::Le => "<=",
                };
                write!(f, "{key}{op}{value}")
            }
        }
    }
}

/// One step of a scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Its line in the file.
    pub line: usize,
    /// The line as written (trimmed).
    pub text: String,
    /// Who does it.
    pub who: Who,
    /// The verb (`connect`, `see`, `start`, ...).
    pub verb: String,
    /// The verb's arguments.
    pub args: Vec<String>,
    /// What must hold afterwards (empty: it must simply succeed).
    pub expect: Vec<Pred>,
    /// How long it may take (a query is retried until it holds).
    pub within: Option<Duration>,
    /// The rule a failure breaks.
    pub rule: Option<String>,
}

/// A parsed scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    /// The file it came from.
    pub file: PathBuf,
    /// Its id.
    pub name: String,
    /// What it shows.
    pub about: String,
    /// Its stated rules, in order.
    pub rules: Vec<Rule>,
    /// How many bots it drives.
    pub bots: usize,
    /// The fresh server's settings.
    pub server: Vec<(String, String)>,
    /// The starting network conditions.
    pub net: Vec<(String, String)>,
    /// Every bot shares 127.0.0.1 instead of getting its own address.
    pub shared_address: bool,
    /// Capabilities named with `needs`.
    pub needs: BTreeSet<String>,
    /// Why it is written ahead of its feature, if it is.
    pub pending: Option<String>,
    /// The seed of injected loss and jitter.
    pub seed: u64,
    /// Its steps, in order.
    pub steps: Vec<Step>,
}

impl Scenario {
    /// The rule a step's failure breaks: its own, or the scenario's only
    /// rule.
    pub fn rule_of(&self, step: &Step) -> Option<&Rule> {
        match &step.rule {
            Some(id) => self.rules.iter().find(|r| &r.id == id),
            // A plain `expect ok` with several rules is setup: no rule of
            // its own.
            None if self.rules.len() == 1 && !step.expect.is_empty() => self.rules.first(),
            None => None,
        }
    }

    /// A server setting, if the scenario gives one.
    pub fn server_opt(&self, key: &str) -> Option<&str> {
        self.server
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Parses a duration: `250ms`, `3s`.
pub fn parse_duration(s: &str) -> Option<Duration> {
    if let Some(ms) = s.strip_suffix("ms") {
        return ms.parse().ok().map(Duration::from_millis);
    }
    s.strip_suffix('s')?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(Duration::from_secs_f64)
}

/// Parses `KEY=VALUE` arguments.
fn key_values(args: &[&str]) -> Result<Vec<(String, String)>, String> {
    args.iter()
        .map(|a| {
            a.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("`{a}` is not KEY=VALUE"))
        })
        .collect()
}

/// Parses `bN`, `bA-B` or `all` against `bots` bots.
fn parse_bots(s: &str, bots: usize) -> Result<Option<Vec<usize>>, String> {
    if s == "all" {
        return Ok(Some((0..bots).collect()));
    }
    let Some(rest) = s.strip_prefix('b') else {
        return Ok(None);
    };
    let (a, b) = match rest.split_once('-') {
        Some((a, b)) => (a, b),
        None => (rest, rest),
    };
    let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else {
        return Ok(None);
    };
    if a > b || b >= bots {
        return Err(format!(
            "`{s}`: the scenario has bots b0-b{}",
            bots.saturating_sub(1)
        ));
    }
    Ok(Some((a..=b).collect()))
}

/// Parses one predicate.
fn parse_pred(s: &str) -> Result<Pred, String> {
    Ok(match s {
        "ok" => Pred::Ok,
        "refused" => Pred::Refused(None),
        "error" => Pred::Error,
        "closed" => Pred::Closed,
        _ => {
            let (key, op, value) = if let Some((k, v)) = s.split_once("!=") {
                (k, Op::Ne, v)
            } else if let Some((k, v)) = s.split_once(">=") {
                (k, Op::Ge, v)
            } else if let Some((k, v)) = s.split_once("<=") {
                (k, Op::Le, v)
            } else if let Some((k, v)) = s.split_once('=') {
                (k, Op::Eq, v)
            } else {
                return Err(format!("`{s}` is not a predicate"));
            };
            if key.is_empty() {
                return Err(format!("`{s}` has no key"));
            }
            if key == "refused" && op == Op::Eq {
                return Ok(Pred::Refused(Some(value.to_string())));
            }
            if matches!(op, Op::Ge | Op::Le) && value.parse::<i64>().is_err() {
                return Err(format!("`{s}`: {value} is not a number"));
            }
            Pred::Fact {
                key: key.to_string(),
                op,
                value: value.to_string(),
            }
        }
    })
}

/// Reads and parses a scenario file.
pub fn load(path: &Path) -> Result<Scenario, ParseError> {
    let text = std::fs::read_to_string(path).map_err(|e| ParseError {
        file: path.display().to_string(),
        line: 0,
        msg: format!("reading: {e}"),
    })?;
    parse(path, &text)
}

/// Parses a scenario from its text.
pub fn parse(path: &Path, text: &str) -> Result<Scenario, ParseError> {
    let file = path.display().to_string();
    let err = |line: usize, msg: String| ParseError {
        file: file.clone(),
        line,
        msg,
    };
    let mut s = Scenario {
        file: path.to_path_buf(),
        name: String::new(),
        about: String::new(),
        rules: Vec::new(),
        bots: 1,
        server: Vec::new(),
        net: Vec::new(),
        shared_address: false,
        needs: BTreeSet::new(),
        pending: None,
        seed: 1,
        steps: Vec::new(),
    };
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let body = raw.split_once('#').map_or(raw, |(b, _)| b).trim();
        if body.is_empty() {
            continue;
        }
        let words: Vec<&str> = body.split_whitespace().collect();
        let rest = || body[words[0].len()..].trim().to_string();
        match words[0] {
            "name" => s.name = rest(),
            "about" => s.about = rest(),
            "rule" => {
                let id = words
                    .get(1)
                    .ok_or_else(|| err(line, "rule needs an id and its text".into()))?;
                let text = body["rule".len()..].trim_start()[id.len()..].trim();
                if text.is_empty() {
                    return Err(err(line, format!("rule {id} has no text")));
                }
                if s.rules.iter().any(|r| r.id == *id) {
                    return Err(err(line, format!("rule {id} is stated twice")));
                }
                s.rules.push(Rule {
                    id: id.to_string(),
                    text: text.to_string(),
                });
            }
            "bots" => {
                if !s.steps.is_empty() {
                    return Err(err(line, "bots comes before the first step".into()));
                }
                let n: usize = words
                    .get(1)
                    .and_then(|w| w.parse().ok())
                    .filter(|n| (1..=MAX_BOTS).contains(n))
                    .ok_or_else(|| err(line, format!("bots takes 1..={MAX_BOTS}")))?;
                s.bots = n;
            }
            "server" if words.get(1).is_some_and(|w| w.contains('=')) => {
                s.server = key_values(&words[1..]).map_err(|m| err(line, m))?;
            }
            "net" if s.steps.is_empty() && words.get(1).is_some_and(|w| w.contains('=')) => {
                s.net = key_values(&words[1..]).map_err(|m| err(line, m))?;
            }
            "addresses" => {
                s.shared_address = match words.get(1).copied() {
                    Some("per-bot") => false,
                    Some("shared") => true,
                    _ => return Err(err(line, "addresses takes per-bot or shared".into())),
                }
            }
            "needs" => s.needs.extend(words[1..].iter().map(|w| w.to_string())),
            "pending" => s.pending = Some(rest()),
            "seed" => {
                s.seed = words
                    .get(1)
                    .and_then(|w| w.parse().ok())
                    .ok_or_else(|| err(line, "seed takes a number".into()))?;
            }
            _ => {
                let step = parse_step(line, body, &words, s.bots).map_err(|m| err(line, m))?;
                s.steps.push(step);
            }
        }
    }
    if s.name.is_empty() {
        s.name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    if s.rules.is_empty() {
        return Err(err(0, "a scenario states at least one rule".into()));
    }
    for step in &s.steps {
        if let Some(id) = &step.rule {
            if !s.rules.iter().any(|r| &r.id == id) {
                return Err(err(step.line, format!("rule {id} is not stated")));
            }
        } else if step.expect.iter().any(|p| *p != Pred::Ok) && s.rules.len() > 1 {
            return Err(err(
                step.line,
                "a step with expect names its rule (the scenario states several)".into(),
            ));
        }
    }
    if s.steps.is_empty() && s.pending.is_none() {
        return Err(err(0, "a scenario has steps (or is pending)".into()));
    }
    Ok(s)
}

/// Parses one step line.
fn parse_step(line: usize, body: &str, words: &[&str], bots: usize) -> Result<Step, String> {
    let (who, mut i) = match words[0] {
        "server" => (Who::Server, 1),
        "wait" => (Who::Wait, 1),
        "net" => match words.get(1).map(|w| parse_bots(w, bots)).transpose()? {
            Some(Some(b)) => (Who::Net(b), 2),
            _ => (Who::Net((0..bots).collect()), 1),
        },
        w => match parse_bots(w, bots)? {
            Some(b) => (Who::Bots(b), 1),
            None => return Err(format!("`{w}` is not a header, bot, server, net or wait")),
        },
    };
    let verb = match who {
        Who::Net(_) => "set".to_string(),
        _ => {
            let v = words.get(i).ok_or("the step has no verb")?.to_string();
            i += 1;
            v
        }
    };
    let mut args = Vec::new();
    let mut expect = Vec::new();
    let mut within = None;
    let mut rule = None;
    let mut mode = 0u8; // 0 args, 1 expect
    while i < words.len() {
        match words[i] {
            "expect" => mode = 1,
            "within" => {
                i += 1;
                let d = words.get(i).ok_or("within takes a duration")?;
                within = Some(parse_duration(d).ok_or(format!("`{d}` is not a duration"))?);
            }
            "rule" => {
                i += 1;
                rule = Some(words.get(i).ok_or("rule takes an id")?.to_string());
            }
            w if mode == 1 => expect.push(parse_pred(w)?),
            w => args.push(w.to_string()),
        }
        i += 1;
    }
    match (&who, verb.as_str()) {
        (Who::Server, "start" | "stop" | "kill" | "restart") => {}
        (Who::Server, "admin" | "dev" | "progression") if !args.is_empty() => {}
        (Who::Server, v) => {
            return Err(format!(
                "server takes start|stop|kill|restart|admin ARGS|dev CHARACTER COMMAND|progression CHARACTER N, not {v}"
            ))
        }
        (Who::Wait, _) => {
            if parse_duration(&verb).is_none() {
                return Err(format!("wait takes a duration, not {verb}"));
            }
        }
        (Who::Net(_), _) => {
            key_values(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        }
        _ => {}
    }
    Ok(Step {
        line,
        text: body.to_string(),
        who,
        verb,
        args,
        expect,
        within,
        rule,
    })
}
