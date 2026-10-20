//! Runs scenarios: a fresh server per scenario, a relay and a bot per
//! bot slot, the steps in order, and the tool's own rules (the `core.*`
//! oracles) on every run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use d2_net::Lane;
use d2_proto::client::{CreateGame, Walk};

use crate::bot::{difficulty_number, Bot, HelloDefaults, Outcome, ServerCert, Status};
use crate::net::{Conditions, Relay};
use crate::play::{
    self,
    unit::{ITEM, MONSTER},
    Monster,
};
use crate::report::{Failure, RuleRef, ScenarioReport, Verdict};
use crate::scenario::{Op, Pred, Scenario, Step, Who};
use crate::server::{Ready, ServerProc, StartError, Target};
use crate::verbs::{self, Kind};

/// The tool's own rules: they hold in every scenario.
pub const CORE_RULES: &[(&str, &str)] = &[
    ("core.server-start", "the server starts with the scenario's settings and prints its ready line"),
    ("core.server-alive", "the server keeps running until the scenario stops it"),
    ("core.no-panic", "the server never panics"),
    ("core.conn-kept", "a bot's connection stays open unless the scenario ends it (disconnect, cut link, server stop)"),
    ("core.messages-parse", "every message the server delivers parses with its size rule and layout"),
    ("core.step", "a step without expectations succeeds (otherwise the scenario can't reach the state it tests)"),
];

fn core(id: &str) -> RuleRef {
    let text = CORE_RULES
        .iter()
        .find(|(i, _)| *i == id)
        .map_or("", |(_, t)| t);
    RuleRef {
        id: id.into(),
        text: text.into(),
    }
}

/// Expansion and bit 2: the flags of every 0x67 a bot sends.
const EXPANSION_FLAGS: u32 = (1 << 20) | 0x4;

/// How often the runner reads every bot while it waits.
const PUMP: Duration = Duration::from_millis(5);

/// Default time limits.
const ACTION_WITHIN: Duration = Duration::from_secs(10);
const QUERY_WITHIN: Duration = Duration::from_secs(3);

/// What the server under test can do, found by starting it once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// It runs; it announced these capabilities (or none).
    Serving(Option<BTreeSet<String>>),
    /// It says it can't run games yet.
    NotServing(String),
    /// It doesn't start.
    Broken(String),
}

/// Starts the server once in `dir` and stops it: what can it do?
pub fn probe(target: &Target, dir: PathBuf) -> Probe {
    probe_with(target, dir, Vec::new())
}

/// [`probe`] with a scenario's server settings: some capabilities depend
/// on them (a `dev = test` server lists `dev`).
pub fn probe_with(target: &Target, dir: PathBuf, settings: Vec<(String, String)>) -> Probe {
    let mut s = match ServerProc::new(target.clone(), dir, settings) {
        Ok(s) => s,
        Err(e) => return Probe::Broken(e.to_string()),
    };
    let p = match s.start() {
        Ok(r) => Probe::Serving(r.caps.clone()),
        Err(StartError::NotServing(t)) => Probe::NotServing(t),
        Err(StartError::Failed(t)) => Probe::Broken(t),
    };
    s.stop();
    p
}

/// Checks that every bot step uses a known verb.
pub fn validate(s: &Scenario) -> Result<(), String> {
    for step in &s.steps {
        if let Who::Bots(_) = step.who {
            if verbs::find(&step.verb).is_none() {
                return Err(format!(
                    "{}:{}: unknown verb {} (mp-test --verbs lists them)",
                    s.file.display(),
                    step.line,
                    step.verb
                ));
            }
        }
    }
    Ok(())
}

/// Why a scenario can't run here, or `None` when it can.
pub fn pending_reason(s: &Scenario, probe: &Probe) -> Option<String> {
    if let Some(p) = &s.pending {
        return Some(p.clone());
    }
    // A server with a lobby enters games through it (create-game and
    // join-game); it takes no direct game stream, so a scenario that only
    // says `enter` has no way in on it.
    if let Probe::Serving(Some(caps)) = probe {
        let direct = s.steps.iter().any(|st| st.verb == "enter");
        let through_lobby = s
            .steps
            .iter()
            .any(|st| matches!(st.verb.as_str(), "create-game" | "join-game"));
        if caps.contains("lobby") && direct && !through_lobby {
            return Some(
                "the server enters games through its lobby (create-game/join-game), not directly"
                    .into(),
            );
        }
    }
    let mut unbuilt = Vec::new();
    let mut needs: BTreeSet<String> = s.needs.clone();
    for step in &s.steps {
        if let (Who::Server, Some(cap)) = (&step.who, verbs::server_step_cap(&step.verb)) {
            needs.insert(cap.to_string());
        }
        if let Who::Bots(_) = step.who {
            match verbs::find(&step.verb) {
                Some(v) => {
                    needs.insert(v.cap.to_string());
                    if !v.built && !unbuilt.contains(&v.name) {
                        unbuilt.push(v.name);
                    }
                }
                None => return Some(format!("line {}: unknown verb {}", step.line, step.verb)),
            }
            needs.extend(
                verbs::arg_caps(&step.verb, &step.args)
                    .into_iter()
                    .map(String::from),
            );
        }
    }
    if !unbuilt.is_empty() {
        return Some(format!(
            "mp-test has no driver yet for: {}",
            unbuilt.join(", ")
        ));
    }
    let driven = verbs::driven();
    let undriven: Vec<&str> = needs
        .iter()
        .map(String::as_str)
        .filter(|c| !driven.contains(c))
        .collect();
    if !undriven.is_empty() {
        return Some(format!("mp-test can't drive: {}", undriven.join(", ")));
    }
    match probe {
        Probe::NotServing(t) => Some(format!(
            "the server can't run games yet ({})",
            first_line(t)
        )),
        Probe::Serving(caps) => {
            // A server that names no capabilities is taken to serve what
            // the tool drives, except the ones only it can vouch for.
            let missing: Vec<&str> = needs
                .iter()
                .map(String::as_str)
                .filter(|c| match caps {
                    Some(caps) => !caps.contains(*c),
                    None => verbs::SERVER_CAPS.contains(c),
                })
                .collect();
            (!missing.is_empty())
                .then(|| format!("the server doesn't serve: {}", missing.join(", ")))
        }
        Probe::Broken(_) => None,
    }
}

/// The line of a server's output that says why it stopped.
fn first_line(t: &str) -> &str {
    t.lines()
        .find(|l| l.contains("not built yet"))
        .or_else(|| t.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or(t)
}

/// The capabilities a scenario needs, for the report.
pub fn needs_of(s: &Scenario) -> Vec<String> {
    let mut n: BTreeSet<String> = s.needs.clone();
    for step in &s.steps {
        if let (Who::Server, Some(cap)) = (&step.who, verbs::server_step_cap(&step.verb)) {
            n.insert(cap.into());
        }
        if let (Who::Bots(_), Some(v)) = (&step.who, verbs::find(&step.verb)) {
            n.insert(v.cap.into());
            n.extend(
                verbs::arg_caps(&step.verb, &step.args)
                    .into_iter()
                    .map(String::from),
            );
        }
    }
    n.into_iter().collect()
}

/// Checks an outcome against a step's expectations; `Err` says what
/// didn't hold.
pub fn check(preds: &[Pred], o: &Outcome) -> Result<(), String> {
    let has_status = preds
        .iter()
        .any(|p| matches!(p, Pred::Ok | Pred::Refused(_) | Pred::Error | Pred::Closed));
    if !has_status && o.status != Status::Ok {
        return Err(o.status.to_string());
    }
    for p in preds {
        let held = match (p, &o.status) {
            (Pred::Ok, s) => *s == Status::Ok,
            (Pred::Refused(None), s) => matches!(s, Status::Refused { .. }),
            (Pred::Refused(Some(c)), Status::Refused { code, .. }) => code == c,
            (Pred::Refused(Some(_)), _) => false,
            (Pred::Error, s) => matches!(s, Status::Error(_)),
            (Pred::Closed, s) => matches!(s, Status::Closed(_)),
            (Pred::Fact { key, op, value }, _) => match o.facts.get(key) {
                None => return Err(format!("no fact {key} ({})", o.status)),
                Some(v) => match op {
                    Op::Eq => v == value,
                    Op::Ne => v != value,
                    Op::Ge | Op::Le => {
                        let (a, b) = (v.parse::<i64>(), value.parse::<i64>());
                        match (a, b) {
                            (Ok(a), Ok(b)) if *op == Op::Ge => a >= b,
                            (Ok(a), Ok(b)) => a <= b,
                            _ => false,
                        }
                    }
                },
            },
        };
        if !held {
            return Err(describe_for(preds, o));
        }
    }
    Ok(())
}

/// What an outcome says about the keys the expectations name.
fn describe_for(preds: &[Pred], o: &Outcome) -> String {
    let mut s = o.status.to_string();
    if let Some(d) = o.facts.get("diff") {
        s.push_str(&format!(" ({d})"));
    }
    for p in preds {
        if let Pred::Fact { key, .. } = p {
            match o.facts.get(key) {
                Some(v) => s.push_str(&format!(" {key}={v}")),
                None => s.push_str(&format!(" {key}=(none)")),
            }
        }
    }
    s
}

fn expected_text(preds: &[Pred]) -> String {
    if preds.is_empty() {
        "ok".into()
    } else {
        preds
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// One scenario in progress.
struct Run<'a> {
    s: &'a Scenario,
    server: ServerProc,
    bots: Vec<Bot>,
    hello: HelloDefaults,
    cert: ServerCert,
    failures: Vec<Failure>,
    /// Bots whose connection may end now (the scenario ended it).
    may_close: Vec<bool>,
    /// Connections that ended unasked: bot -> (line, why).
    unasked: BTreeMap<usize, (usize, String)>,
    /// The scenario stopped the server.
    server_down: bool,
    /// The line being run.
    line: usize,
    /// Monsters a dev command spawned, by the scenario's word for them.
    spawned: BTreeMap<String, Spawned>,
}

/// A monster a dev command spawned.
#[derive(Clone, Copy, Debug)]
struct Spawned {
    /// Its `monstats` row.
    class: u16,
    /// Where the character stood that it was spawned next to.
    at: Option<(u16, u16)>,
}

/// A bot's own player's position, once assigned.
fn own_position(bot: &Bot) -> Option<(u16, u16)> {
    let own = bot.view.own?;
    bot.view.players.get(&own).map(|p| (p.x, p.y))
}

/// How often a play driver repeats a request the server hasn't acted on.
const RETRY: Duration = Duration::from_millis(400);

/// Where a bot walks to, next to a waypoint, before it uses it: one side
/// after another while the waypoint doesn't open.
const WALK_SIDES: [(i16, i16); 4] = [(0, 3), (3, 0), (0, -3), (-3, 0)];

/// A C→S unit message: `id`, then the unit's type and id (u32 each).
fn unit_msg(id: u8, kind: u8, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend_from_slice(&u32::from(kind).to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

/// This build's own hello values.
fn own_hello() -> HelloDefaults {
    HelloDefaults {
        protocol: d2_proto::PROTOCOL_VERSION,
        game: "d2rust".into(),
        build: "0.0.0".into(),
        tick_rate: d2_sim::tick::TickRate::BASE.hz(),
    }
}

fn hello_from(r: &Ready) -> HelloDefaults {
    let own = own_hello();
    HelloDefaults {
        protocol: r.protocol.unwrap_or(own.protocol),
        game: r.game.clone().unwrap_or(own.game),
        build: r.build.clone().unwrap_or(own.build),
        tick_rate: r.tick.unwrap_or(own.tick_rate),
    }
}

/// A seed per scenario and bot, from the run's seed.
fn seed_for(base: u64, s: &Scenario, bot: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.name.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h ^ base.wrapping_mul(0x9E37_79B9) ^ s.seed.rotate_left(17) ^ (bot as u64)
}

/// The loopback address a bot's relay uses toward the server: its own
/// (127.0.0.2, .3, ...; contracts 10.1) unless the scenario tests a per-IP
/// limit with all bots on 127.0.0.1.
fn bot_address(bot: usize, shared: bool) -> std::net::IpAddr {
    if shared {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2 + bot as u8))
    }
}

/// Runs one scenario (which [`pending_reason`] cleared) with its server in
/// `dir`.
pub fn run(s: &Scenario, target: &Target, dir: PathBuf, seed: u64) -> ScenarioReport {
    let started = Instant::now();
    let mut report = ScenarioReport {
        name: s.name.clone(),
        file: s.file.display().to_string(),
        about: s.about.clone(),
        verdict: Verdict::Pass,
        pending: None,
        needs: needs_of(s),
        bots: s.bots,
        steps_run: 0,
        ms: 0,
        failures: Vec::new(),
        datagrams: (0, 0),
        server_tail: None,
    };
    let fail_start = |report: &mut ScenarioReport, actual: String, tail: Option<String>| {
        report.failures.push(Failure {
            rule: core("core.server-start"),
            line: 0,
            step: "start the server".into(),
            bot: None,
            expected: "a ready line".into(),
            actual,
        });
        report.verdict = Verdict::Fail;
        report.server_tail = tail;
    };
    let mut server = match ServerProc::new(target.clone(), dir.join("server"), s.server.clone()) {
        Ok(p) => p,
        Err(e) => {
            fail_start(&mut report, e.to_string(), None);
            return report;
        }
    };
    let ready = match server.start() {
        Ok(r) => r.clone(),
        Err(StartError::NotServing(t)) => {
            report.verdict = Verdict::Pending;
            report.pending = Some(format!(
                "the server can't run games yet ({})",
                first_line(&t)
            ));
            return report;
        }
        Err(StartError::Failed(t)) => {
            let tail = server.output_tail(20);
            fail_start(&mut report, t, Some(tail));
            return report;
        }
    };
    let mut start_cond = Conditions::default();
    if let Err(e) = start_cond.apply(&s.net) {
        report.failures.push(Failure {
            rule: core("core.step"),
            line: 0,
            step: "net".into(),
            bot: None,
            expected: "valid network conditions".into(),
            actual: e,
        });
        report.verdict = Verdict::Fail;
        return report;
    }
    let mut bots = Vec::new();
    for i in 0..s.bots {
        match Relay::start(
            ready.listen,
            seed_for(seed, s, i),
            bot_address(i, s.shared_address),
        ) {
            Ok(r) => {
                r.set(start_cond);
                bots.push(Bot::new(i, r));
            }
            Err(e) => {
                fail_start(&mut report, format!("relay for b{i}: {e}"), None);
                return report;
            }
        }
    }
    let mut run = Run {
        s,
        server,
        bots,
        hello: hello_from(&ready),
        cert: ServerCert {
            der: ready.cert.clone(),
            sha256: ready.cert_sha256,
        },
        failures: Vec::new(),
        may_close: vec![false; s.bots],
        unasked: BTreeMap::new(),
        server_down: false,
        line: 0,
        spawned: BTreeMap::new(),
    };
    for step in &s.steps {
        run.line = step.line;
        report.steps_run += 1;
        let setup_broke = run.step(step);
        if setup_broke {
            break;
        }
    }
    run.pump_all();
    run.final_checks();
    report.datagrams = run.bots.iter().fold((0, 0), |(a, b), bot| {
        let (r, d) = bot.relay.counts();
        (a + r, b + d)
    });
    report.failures = std::mem::take(&mut run.failures);
    if !report.failures.is_empty() {
        report.verdict = Verdict::Fail;
        report.server_tail = Some(run.server.output_tail(30));
    }
    // Close every bot at once: a goodbye to a dead server waits a moment.
    let bots = std::mem::take(&mut run.bots);
    std::thread::scope(|sc| {
        for b in bots {
            sc.spawn(move || drop(b));
        }
    });
    run.server.stop();
    report.ms = started.elapsed().as_millis() as u64;
    report
}

impl Run<'_> {
    fn fail(&mut self, step: &Step, bot: Option<usize>, expected: String, actual: String) {
        let rule = match self.s.rule_of(step) {
            Some(r) => RuleRef {
                id: r.id.clone(),
                text: r.text.clone(),
            },
            None => core("core.step"),
        };
        self.failures.push(Failure {
            rule,
            line: step.line,
            step: step.text.clone(),
            bot,
            expected,
            actual,
        });
    }

    /// Reads every bot once and watches the server.
    fn pump_all(&mut self) {
        for (i, b) in self.bots.iter_mut().enumerate() {
            if let Some(why) = b.pump() {
                // A new login to the same account takes the old session
                // over: the contracts have the server end it.
                if !self.may_close[i] && !b.taken() {
                    self.unasked.entry(i).or_insert((self.line, why));
                }
            }
        }
        if !self.server_down {
            if let Some(status) = self.server.exited() {
                self.server_down = true;
                self.failures.push(Failure {
                    rule: core("core.server-alive"),
                    line: self.line,
                    step: "(the server process)".into(),
                    bot: None,
                    expected: "running".into(),
                    actual: format!("exited: {status}"),
                });
                self.may_close.fill(true);
            }
        }
    }

    /// Pumps until `done` holds or `until` passes; returns whether it held.
    fn pump_until(&mut self, until: Instant, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        loop {
            self.pump_all();
            if done(self) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(PUMP);
        }
    }

    /// Runs one step; `true` when a step without expectations failed (the
    /// scenario stops: it can't reach the state it tests).
    fn step(&mut self, step: &Step) -> bool {
        match &step.who {
            Who::Wait => {
                let d = crate::scenario::parse_duration(&step.verb).unwrap_or_default();
                self.pump_until(Instant::now() + d, |_| false);
                false
            }
            Who::Server => self.server_step(step),
            Who::Net(bots) => {
                let kv: Vec<(String, String)> = step
                    .args
                    .iter()
                    .filter_map(|a| a.split_once('=').map(|(k, v)| (k.into(), v.into())))
                    .collect();
                for &b in bots {
                    let mut c = self.bots[b].relay.conditions();
                    if let Err(e) = c.apply(&kv) {
                        self.fail(step, Some(b), "valid network conditions".into(), e);
                        return true;
                    }
                    if c.cut {
                        self.may_close[b] = true;
                    }
                    self.bots[b].relay.set(c);
                }
                false
            }
            Who::Bots(bots) => self.bot_step(step, bots),
        }
    }

    fn server_step(&mut self, step: &Step) -> bool {
        let stop = |run: &mut Self, kill: bool| {
            run.server_down = true;
            run.may_close.fill(true);
            if kill {
                run.server.kill();
            } else {
                run.server.stop();
            }
        };
        match step.verb.as_str() {
            "stop" => stop(self, false),
            "kill" => stop(self, true),
            "admin" | "dev" | "progression" => {
                let r = match step.verb.as_str() {
                    "admin" => self.server.admin(&step.args),
                    "dev" => self
                        .server
                        .command_line(&format!("dev {}", step.args.join(" ")))
                        .map(|text| self.note_dev(&step.args, &text)),
                    // Contracts 10.1: the running test server takes
                    // `set-progression CHARACTER N` on its stdin.
                    _ => self
                        .server
                        .command_line(&format!("set-progression {}", step.args.join(" ")))
                        .map(drop),
                };
                if let Err(e) = r {
                    self.fail(step, None, expected_text(&step.expect), e);
                    return step.expect.is_empty();
                }
            }
            "start" | "restart" => {
                if step.verb == "restart" {
                    stop(self, false);
                }
                match self.server.start() {
                    Ok(r) => {
                        let r = r.clone();
                        self.server_down = false;
                        for b in &self.bots {
                            b.relay.retarget(r.listen);
                        }
                        self.cert = ServerCert {
                            der: r.cert.clone(),
                            sha256: r.cert_sha256,
                        };
                        self.hello = hello_from(&r);
                    }
                    Err(e) => {
                        let expected = expected_text(&step.expect);
                        let rule = self.s.rule_of(step).map(|r| RuleRef {
                            id: r.id.clone(),
                            text: r.text.clone(),
                        });
                        self.failures.push(Failure {
                            rule: rule.unwrap_or_else(|| core("core.server-start")),
                            line: step.line,
                            step: step.text.clone(),
                            bot: None,
                            expected,
                            actual: e.to_string(),
                        });
                        return true;
                    }
                }
            }
            _ => {}
        }
        false
    }

    /// Remembers what a dev command made that later steps name: a
    /// `spawn MONSTER` answers `spawned N x NAME (CLASS)`, and the
    /// scenario's word for the monster then means that class, nearest to
    /// where the character stood.
    fn note_dev(&mut self, args: &[String], text: &str) {
        let (Some(character), Some(cmd), Some(word)) = (args.first(), args.get(1), args.get(2))
        else {
            return;
        };
        if cmd != "spawn" {
            return;
        }
        let class = text
            .rfind('(')
            .and_then(|i| text[i + 1..].split(')').next())
            .and_then(|c| c.trim().parse::<u16>().ok());
        let Some(class) = class else {
            return;
        };
        let at = self
            .bots
            .iter()
            .find(|b| b.play_name().eq_ignore_ascii_case(character))
            .and_then(own_position);
        self.spawned
            .insert(word.to_ascii_lowercase(), Spawned { class, at });
    }

    fn bot_step(&mut self, step: &Step, bots: &[usize]) -> bool {
        let Some(v) = verbs::find(&step.verb) else {
            self.fail(step, None, "a known verb".into(), step.verb.clone());
            return true;
        };
        let within = step.within.unwrap_or(match v.kind {
            Kind::Action => ACTION_WITHIN,
            Kind::Query => QUERY_WITHIN,
        });
        let until = Instant::now() + within;
        let outcomes: Vec<(usize, Outcome)> = match (v.kind, v.name) {
            (Kind::Query, "agree") => {
                let mut last = Outcome::ok();
                let held = self.pump_until(until, |run| {
                    last = run.agree(bots);
                    check(&step.expect, &last).is_ok()
                });
                if held {
                    Vec::new()
                } else {
                    vec![(usize::MAX, last)]
                }
            }
            (Kind::Query, _) => {
                let mut last: BTreeMap<usize, Outcome> = BTreeMap::new();
                self.pump_until(until, |run| {
                    let mut all = true;
                    for &b in bots {
                        let o = run.query(v.name, &step.args, b, until);
                        if check(&step.expect, &o).is_err() {
                            all = false;
                        }
                        last.insert(b, o);
                    }
                    all
                });
                last.into_iter().collect()
            }
            (Kind::Action, _) => self.action(step, v.name, bots, until),
        };
        let mut broke = false;
        for (b, o) in outcomes {
            if let Err(actual) = check(&step.expect, &o) {
                let bot = (b != usize::MAX).then_some(b);
                self.fail(step, bot, expected_text(&step.expect), actual);
                broke |= step.expect.is_empty();
            } else if b != usize::MAX && step.expect.contains(&Pred::Closed) {
                // The scenario expected this connection to end.
                self.unasked.remove(&b);
            }
        }
        broke
    }

    /// One action for each bot of the step.
    fn action(
        &mut self,
        step: &Step,
        verb: &str,
        bots: &[usize],
        until: Instant,
    ) -> Vec<(usize, Outcome)> {
        let kv: Vec<(String, String)> = step
            .args
            .iter()
            .filter_map(|a| a.split_once('=').map(|(k, v)| (k.into(), v.into())))
            .collect();
        match verb {
            "connect" | "reconnect" => bots
                .iter()
                .map(|&b| {
                    let o = self.bots[b].connect(&kv, &self.hello, &self.cert, verb == "reconnect");
                    if o.status == Status::Ok {
                        self.may_close[b] = self.bots[b].relay.conditions().cut;
                        self.unasked.remove(&b);
                    }
                    (b, o)
                })
                .collect(),
            "disconnect" => bots
                .iter()
                .map(|&b| {
                    self.may_close[b] = true;
                    (b, self.bots[b].disconnect())
                })
                .collect(),
            "leave" => bots
                .iter()
                .map(|&b| {
                    self.bots[b].entered = false;
                    (b, self.bots[b].send(Lane::System, &[0x69]))
                })
                .collect(),
            "move" => bots
                .iter()
                .map(|&b| (b, self.walk(b, &step.args)))
                .collect(),
            "frames" => {
                let n: u64 = step.args.first().and_then(|a| a.parse().ok()).unwrap_or(1);
                let goal: Vec<(usize, u64)> = bots
                    .iter()
                    .map(|&b| (b, self.bots[b].view.frames + n))
                    .collect();
                self.pump_until(until, |run| {
                    goal.iter()
                        .all(|&(b, g)| run.bots[b].view.frames >= g || !run.bots[b].connected())
                });
                goal.iter()
                    .map(|&(b, g)| {
                        let bot = &self.bots[b];
                        let o = if !bot.connected() {
                            bot.closed_outcome()
                        } else if bot.view.frames >= g {
                            Outcome::ok()
                        } else {
                            Outcome::error(format!(
                                "{} of {n} frames arrived",
                                n + bot.view.frames - g
                            ))
                        };
                        (b, o)
                    })
                    .collect()
            }
            "enter" => self.enter(step, bots, &kv, until),
            "register" | "login" => bots
                .iter()
                .map(|&b| {
                    let (Some(account), Some(password)) = (step.args.first(), step.args.get(1))
                    else {
                        return (b, Outcome::error(format!("{verb} takes ACCOUNT PASSWORD")));
                    };
                    let wait = until.saturating_duration_since(Instant::now());
                    let o = if verb == "register" {
                        self.bots[b].register(account, password, wait)
                    } else {
                        self.bots[b].login(account, password, wait)
                    };
                    (b, o)
                })
                .collect(),
            "logout" => bots
                .iter()
                .map(|&b| {
                    let wait = until.saturating_duration_since(Instant::now());
                    let o = self.bots[b].logout(wait);
                    if o.status == Status::Ok {
                        self.may_close[b] = true;
                        self.unasked.remove(&b);
                    }
                    (b, o)
                })
                .collect(),
            "char-create" | "char-delete" | "char-select" => bots
                .iter()
                .map(|&b| {
                    let wait = until.saturating_duration_since(Instant::now());
                    let name = step.args.first().cloned().unwrap_or_default();
                    let o = match verb {
                        "char-create" => {
                            let Some(class) = step.args.get(1).and_then(|c| c.parse::<u8>().ok())
                            else {
                                return (
                                    b,
                                    Outcome::error(
                                        "char-create takes NAME CLASS [hardcore] [classic]",
                                    ),
                                );
                            };
                            self.bots[b].char_create(
                                &name,
                                class,
                                step.args.iter().any(|a| a == "hardcore"),
                                step.args.iter().any(|a| a == "classic"),
                                wait,
                            )
                        }
                        "char-select" => self.bots[b].char_select(&name, wait),
                        _ => {
                            let password = step.args.get(1).cloned().unwrap_or_default();
                            self.bots[b].char_delete(&name, &password, wait)
                        }
                    };
                    (b, o)
                })
                .collect(),
            "create-game" | "join-game" => bots
                .iter()
                .map(|&b| {
                    let wait = until.saturating_duration_since(Instant::now());
                    let name = step
                        .args
                        .iter()
                        .find(|a| !a.contains('='))
                        .cloned()
                        .unwrap_or_default();
                    let get =
                        |k: &str| kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
                    let password = get("password").unwrap_or_default();
                    let o = if verb == "create-game" {
                        let Some(difficulty) = get("difficulty").map_or(Some(0), difficulty_number)
                        else {
                            return (
                                b,
                                Outcome::error("difficulty takes normal, nightmare or hell"),
                            );
                        };
                        let max = get("max").map_or(Some(0), |m| m.parse::<u8>().ok());
                        let Some(max) = max else {
                            return (b, Outcome::error("max takes a number"));
                        };
                        self.bots[b].create_game(&name, password, difficulty, max, wait, until)
                    } else {
                        self.bots[b].join_game(&name, password, wait, until)
                    };
                    (b, o)
                })
                .collect(),
            "leave-game" => bots
                .iter()
                .map(|&b| {
                    let wait = until.saturating_duration_since(Instant::now());
                    (b, self.bots[b].leave_game(wait))
                })
                .collect(),
            "chat" => bots
                .iter()
                .map(|&b| {
                    let wait = until.saturating_duration_since(Instant::now());
                    let text = step.args.join(" ");
                    (b, self.bots[b].chat(&text, wait))
                })
                .collect(),
            "attack" => self.attack(step, bots, until),
            "drop" | "pickup" => bots
                .iter()
                .map(|&b| {
                    let Some(code) = step.args.first() else {
                        return (b, Outcome::error(format!("{verb} takes ITEM")));
                    };
                    let o = if verb == "drop" {
                        self.drop_item(b, code, until)
                    } else {
                        self.pickup(b, code, until)
                    };
                    (b, o)
                })
                .collect(),
            "waypoint" | "act" => bots
                .iter()
                .map(|&b| {
                    let word = step.args.first().map(String::as_str).unwrap_or_default();
                    let level = if verb == "waypoint" {
                        play::area_level(word)
                    } else {
                        word.parse().ok().and_then(play::town_of)
                    };
                    let Some(level) = level else {
                        let usage = if verb == "waypoint" {
                            "waypoint takes an area name or level id"
                        } else {
                            "act takes 1 to 5"
                        };
                        return (b, Outcome::error(usage));
                    };
                    (b, self.travel(b, level, until))
                })
                .collect(),
            _ => bots
                .iter()
                .map(|&b| (b, Outcome::error(format!("{verb} has no driver"))))
                .collect(),
        }
    }

    /// The monster `word` names for bot `b`: of the class its spawn gave,
    /// the one first seen nearest to where it was spawned.
    fn monster_for(&self, b: usize, word: &str) -> Result<Option<(u32, Monster)>, String> {
        let Some(sp) = self.spawned.get(&word.to_ascii_lowercase()) else {
            return Err(format!(
                "no monster {word} spawned (server dev CHARACTER spawn {word})"
            ));
        };
        let bot = &self.bots[b];
        let near = sp.at.or_else(|| own_position(bot)).unwrap_or_default();
        Ok(bot
            .view
            .play
            .monsters_of(sp.class, near)
            .first()
            .map(|(g, m)| (*g, (*m).clone())))
    }

    /// `attack MONSTER`: every bot of the step attacks it (C→S 0x06, its
    /// left skill on the unit) until it sees it die; ok for a bot that sent
    /// at least one attack (fact `attacks`).
    fn attack(&mut self, step: &Step, bots: &[usize], until: Instant) -> Vec<(usize, Outcome)> {
        let Some(word) = step.args.first().cloned() else {
            return bots
                .iter()
                .map(|&b| (b, Outcome::error("attack takes MONSTER")))
                .collect();
        };
        let mut out: BTreeMap<usize, Outcome> = BTreeMap::new();
        let mut sent: BTreeMap<usize, Instant> = BTreeMap::new();
        let mut hits: BTreeMap<usize, u32> = BTreeMap::new();
        loop {
            self.pump_all();
            for &b in bots {
                if out.contains_key(&b) {
                    continue;
                }
                if !self.bots[b].connected() {
                    out.insert(b, self.bots[b].closed_outcome());
                    continue;
                }
                match self.monster_for(b, &word) {
                    Err(e) => {
                        out.insert(b, Outcome::error(e));
                    }
                    Ok(None) => {}
                    Ok(Some((guid, m))) if m.dead => {
                        // Only a bot that fought it: one that just watched
                        // the death didn't attack.
                        let n = hits.get(&b).copied().unwrap_or(0);
                        let o = if n > 0 {
                            Outcome::ok()
                        } else {
                            Outcome::error(format!("{word} ({guid}) died before b{b} attacked it"))
                        };
                        out.insert(b, o.fact("guid", guid).fact("attacks", n));
                    }
                    Ok(Some((guid, _))) => {
                        if sent.get(&b).is_none_or(|t| t.elapsed() >= RETRY) {
                            let o = self.bots[b].send(Lane::Game, &unit_msg(0x06, MONSTER, guid));
                            if o.status != Status::Ok {
                                out.insert(b, o);
                                continue;
                            }
                            sent.insert(b, Instant::now());
                            *hits.entry(b).or_default() += 1;
                        }
                    }
                }
            }
            if bots.iter().all(|b| out.contains_key(b)) || Instant::now() >= until {
                break;
            }
            std::thread::sleep(PUMP);
        }
        for &b in bots {
            if out.contains_key(&b) {
                continue;
            }
            let n = hits.get(&b).copied().unwrap_or(0);
            let o = match self.monster_for(b, &word) {
                Ok(Some((guid, m))) => Outcome::error(format!(
                    "{word} ({guid}) still alive, life {} after {n} attacks",
                    m.life
                )),
                _ => Outcome::error(format!("no {word} in sight")),
            };
            out.insert(b, o.fact("attacks", n));
        }
        out.into_iter().collect()
    }

    /// `drop ITEM`: takes the held item to the cursor (C→S 0x19) and drops
    /// it (0x17); ok once the bot sees it on the ground.
    fn drop_item(&mut self, b: usize, code: &str, until: Instant) -> Outcome {
        let Some(guid) = self.bots[b].view.play.held_with(code) else {
            return Outcome::error(format!("b{b} doesn't hold {code}"));
        };
        let mut sent: Option<(u8, Instant)> = None;
        self.pump_until(until, |run| {
            let bot = &mut run.bots[b];
            if bot.view.play.ground.contains_key(&guid) || !bot.connected() {
                return true;
            }
            let on_cursor = bot
                .view
                .play
                .held
                .get(&guid)
                .is_some_and(|i| i.mode == play::MODE_CURSOR);
            // From a grid page to the cursor, then from the cursor down.
            let id = if on_cursor { 0x17 } else { 0x19 };
            if sent.is_none_or(|(m, t)| m != id || t.elapsed() >= RETRY * 3) {
                let mut msg = vec![id];
                msg.extend_from_slice(&guid.to_le_bytes());
                if bot.send(Lane::Game, &msg).status == Status::Ok {
                    sent = Some((id, Instant::now()));
                }
            }
            false
        });
        let bot = &self.bots[b];
        if !bot.connected() {
            bot.closed_outcome()
        } else if bot.view.play.ground.contains_key(&guid) {
            Outcome::ok().fact("guid", guid)
        } else {
            Outcome::error(format!("{code} ({guid}) never reached the ground"))
        }
    }

    /// `pickup ITEM`: walks to the nearest one on the ground and picks it
    /// up (C→S 0x16); ok once the bot holds it.
    fn pickup(&mut self, b: usize, code: &str, until: Instant) -> Outcome {
        let here = own_position(&self.bots[b]).unwrap_or_default();
        let Some((guid, at)) = self.bots[b].view.play.ground_with(code, here) else {
            return Outcome::error(format!("b{b} sees no {code} on the ground"));
        };
        let mut sent: Option<Instant> = None;
        self.pump_until(until, |run| {
            let bot = &mut run.bots[b];
            if bot.view.play.held.contains_key(&guid) || !bot.connected() {
                return true;
            }
            if sent.is_none_or(|t| t.elapsed() >= RETRY) {
                if own_position(bot).is_none_or(|p| play::distance(p, at) > 2) {
                    bot.send(Lane::Game, &Walk { x: at.0, y: at.1 }.encode());
                }
                let mut msg = unit_msg(0x16, ITEM, guid);
                // To the inventory, not the cursor.
                msg.extend_from_slice(&0u32.to_le_bytes());
                bot.send(Lane::Game, &msg);
                sent = Some(Instant::now());
            }
            false
        });
        let bot = &self.bots[b];
        if !bot.connected() {
            bot.closed_outcome()
        } else if bot.view.play.held.contains_key(&guid) {
            Outcome::ok().fact("guid", guid)
        } else {
            Outcome::error(format!("{code} ({guid}) not picked up"))
        }
    }

    /// `waypoint AREA` / `act N`: walks next to the nearest waypoint and
    /// travels to the level (C→S 0x49, which the server takes from a player
    /// within its range of a waypoint it knows); ok once the bot stands in
    /// that level.
    fn travel(&mut self, b: usize, level: u8, until: Instant) -> Outcome {
        // Walks next to the waypoint, travel requests, and when the next
        // step may go.
        let (mut walks, mut takes) = (0usize, 0usize);
        let mut next = Instant::now();
        let mut target: Option<(u32, (u16, u16))> = None;
        self.pump_until(until, |run| {
            let bot = &mut run.bots[b];
            if !bot.connected() {
                return true;
            }
            let Some(here) = own_position(bot) else {
                return false;
            };
            if bot.view.play.level_at(here) == Some(level) {
                return true;
            }
            if Instant::now() < next {
                return false;
            }
            let Some((wp, at)) = target.or_else(|| bot.view.play.nearest_waypoint(here)) else {
                return false;
            };
            target = Some((wp, at));
            // Walk next to it (another side after every two requests that
            // took the bot nowhere), then ask to travel.
            if walks <= takes / 2 && walks < WALK_SIDES.len() {
                let (dx, dy) = WALK_SIDES[walks];
                let to = (
                    at.0.saturating_add_signed(dx),
                    at.1.saturating_add_signed(dy),
                );
                bot.send(Lane::Game, &Walk { x: to.0, y: to.1 }.encode());
                let d = u64::from(play::distance(here, to));
                next = Instant::now() + Duration::from_millis(500 + 150 * d);
                walks += 1;
                return false;
            }
            let mut msg = vec![0x49];
            msg.extend_from_slice(&wp.to_le_bytes());
            msg.extend_from_slice(&u16::from(level).to_le_bytes());
            msg.extend_from_slice(&[0, 0]);
            bot.send(Lane::Game, &msg);
            takes += 1;
            next = Instant::now() + RETRY * 5;
            false
        });
        let bot = &self.bots[b];
        if !bot.connected() {
            return bot.closed_outcome();
        }
        let here = own_position(bot).unwrap_or_default();
        let now = bot.view.play.level_at(here);
        if now == Some(level) {
            return Outcome::ok().fact("area", play::area_name(level));
        }
        let why = match target {
            None => {
                let mut near: Vec<(u16, u16)> = bot
                    .view
                    .play
                    .objects
                    .values()
                    .map(|&(class, at)| (play::distance(here, at), class))
                    .collect();
                near.sort_unstable();
                let classes: Vec<String> =
                    near.iter().take(8).map(|(_, c)| c.to_string()).collect();
                format!(
                    "no waypoint in sight ({} objects; nearest classes {})",
                    near.len(),
                    classes.join(",")
                )
            }
            Some((wp, at)) => format!(
                "still in {} after {walks} walks to waypoint {wp} at {at:?} and {takes} requests",
                now.map_or_else(|| "an unknown area".into(), play::area_name)
            ),
        };
        Outcome::error(format!("travel to {}: {why}", play::area_name(level)))
    }

    /// `enter GAME`: every bot of the step sends its 0x67 at once, then
    /// its 0x6B two ticks later, then waits for its own player.
    fn enter(
        &mut self,
        step: &Step,
        bots: &[usize],
        kv: &[(String, String)],
        until: Instant,
    ) -> Vec<(usize, Outcome)> {
        let game = step
            .args
            .iter()
            .find(|a| !a.contains('='))
            .cloned()
            .unwrap_or_else(|| "game".into());
        let get = |k: &str| {
            kv.iter()
                .find(|(key, _)| key == k)
                .and_then(|(_, v)| v.parse::<u8>().ok())
        };
        let mut out: BTreeMap<usize, Outcome> = BTreeMap::new();
        let mut started: Vec<(usize, u64)> = Vec::new();
        for &b in bots {
            let bot = &mut self.bots[b];
            if bot.connected() {
                // A game is entered on its own stream (the stand-in takes
                // any ticket; the real server's lobby hands one out).
                let o = bot.open_game();
                if o.status != Status::Ok {
                    out.insert(b, o);
                    continue;
                }
            }
            bot.view.loaded = false;
            let mut game_name = [0u8; 16];
            let n = game.len().min(15);
            game_name[..n].copy_from_slice(&game.as_bytes()[..n]);
            let mut char_name = [0u8; 16];
            let name = bot.play_name();
            let n = name.len().min(15);
            char_name[..n].copy_from_slice(&name.as_bytes()[..n]);
            let req = CreateGame {
                game_name,
                class: get("class").unwrap_or((b % 7) as u8),
                difficulty: get("difficulty").unwrap_or(0),
                char_name,
                flags: EXPANSION_FLAGS,
                ..CreateGame::default()
            };
            let o = bot.send(Lane::System, &req.encode());
            if o.status == Status::Ok {
                bot.entered = true;
                started.push((b, bot.view.ticks));
            } else {
                out.insert(b, o);
            }
        }
        // The 0x6B goes once the game answered the 0x67 (S→C 0x02) or two
        // ticks came (a game that takes no player answers nothing).
        let mut joined: BTreeSet<usize> = BTreeSet::new();
        self.pump_until(until, |run| {
            started.iter().all(|&(b, t)| {
                let bot = &run.bots[b];
                bot.view.loaded || bot.view.ticks >= t + 2 || !bot.connected() || !bot.in_game()
            })
        });
        for &(b, t) in &started {
            let bot = &self.bots[b];
            if !bot.connected() {
                out.insert(b, bot.closed_outcome());
                continue;
            }
            if !bot.in_game() {
                out.insert(b, Outcome::error("the game stream ended after the 0x67"));
                continue;
            }
            if !bot.view.loaded && bot.view.ticks < t + 2 {
                out.insert(b, Outcome::error("no server ticks arrived after the 0x67"));
                continue;
            }
            let o = self.bots[b].send(Lane::System, &[0x6B]);
            if o.status == Status::Ok {
                joined.insert(b);
            } else {
                out.insert(b, o);
            }
        }
        self.pump_until(until, |run| {
            joined.iter().all(|&b| {
                run.bots[b].view.own.is_some() || !run.bots[b].connected() || !run.bots[b].in_game()
            })
        });
        for b in joined {
            let bot = &self.bots[b];
            let o = match bot
                .view
                .own
                .and_then(|g| bot.view.players.get(&g).map(|p| (g, p)))
            {
                Some((g, p)) => Outcome::ok().fact("guid", g).fact("x", p.x).fact("y", p.y),
                None if !bot.connected() => bot.closed_outcome(),
                None => Outcome {
                    status: Status::Refused {
                        code: "no-player".into(),
                        text: "no player assigned in time".into(),
                    },
                    facts: BTreeMap::new(),
                },
            };
            out.insert(b, o);
        }
        out.into_iter().collect()
    }

    /// `move X Y` or `move +DX +DY`.
    fn walk(&mut self, b: usize, args: &[String]) -> Outcome {
        let bot = &mut self.bots[b];
        let own = bot
            .view
            .own
            .and_then(|g| bot.view.players.get(&g).map(|p| (g, p.x, p.y)));
        let coord = |a: Option<&String>, base: Option<u16>| -> Option<u16> {
            let a = a?;
            if a.starts_with('+') || a.starts_with('-') {
                let d: i32 = a.parse().ok()?;
                u16::try_from(i32::from(base?) + d).ok()
            } else {
                a.parse().ok()
            }
        };
        let (x, y) = match (
            coord(args.first(), own.map(|o| o.1)),
            coord(args.get(1), own.map(|o| o.2)),
        ) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                return Outcome::error(
                    "move takes X Y or +DX +DY (relative needs the bot's own position)",
                )
            }
        };
        let o = bot.send(Lane::Game, &Walk { x, y }.encode());
        if o.status == Status::Ok {
            // The client walks where it asked; the server corrects it.
            if let Some((g, _, _)) = own {
                if let Some(p) = bot.view.players.get_mut(&g) {
                    (p.x, p.y) = (x, y);
                }
            }
        }
        o.fact("x", x).fact("y", y)
    }

    /// A query on one bot.
    fn query(&mut self, verb: &str, args: &[String], b: usize, until: Instant) -> Outcome {
        let wait = until
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        match verb {
            "char-list" => return self.bots[b].char_list(wait),
            "games" => return self.bots[b].games(wait),
            "heard" => return self.bots[b].heard(),
            "monster" => {
                let word = args.first().map(String::as_str).unwrap_or_default();
                return match self.monster_for(b, word) {
                    Err(e) => Outcome::error(e),
                    Ok(None) => Outcome::ok().fact("seen", "no").fact("dead", "no"),
                    Ok(Some((guid, m))) => Outcome::ok()
                        .fact("seen", "yes")
                        .fact("guid", guid)
                        .fact("life", m.life)
                        .fact("dead", if m.dead { "yes" } else { "no" }),
                };
            }
            _ => {}
        }
        let bot = &self.bots[b];
        let mut o = if bot.connected() {
            Outcome::ok()
        } else {
            bot.closed_outcome()
        };
        match verb {
            "stats" => {
                o = o
                    .fact("connected", if bot.connected() { "yes" } else { "no" })
                    .fact("frames", bot.view.frames)
                    .fact("ticks", bot.view.ticks)
                    .fact("delivered", bot.view.delivered)
                    .fact("bad", bot.view.bad.len())
                    .fact("account", bot.account())
                    .fact("session_error", bot.session_error())
                    .fact("game", bot.game())
                    .fact("difficulty", bot.difficulty())
                    .fact(
                        "ids",
                        bot.view
                            .ids
                            .iter()
                            .map(|(id, n)| format!("{id:02X}:{n}"))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
            }
            "see" => {
                o = o.fact("players", bot.view.players.len());
                if let Some(p) = bot.view.own.and_then(|g| bot.view.players.get(&g)) {
                    o = o.fact("x", p.x).fact("y", p.y);
                }
                for other in &self.bots {
                    if other.index == b {
                        continue;
                    }
                    let k = format!("b{}", other.index);
                    match bot.view.named(&other.play_name()) {
                        Some(p) => {
                            let moved = (p.x, p.y) != p.first;
                            o = o
                                .fact(k.clone(), "yes")
                                .fact(format!("{k}.x"), p.x)
                                .fact(format!("{k}.y"), p.y)
                                .fact(format!("{k}.moved"), if moved { "yes" } else { "no" });
                        }
                        None => o = o.fact(k, "no"),
                    }
                }
            }
            "inventory" | "ground" => {
                let play = &bot.view.play;
                let items = if verb == "inventory" {
                    &play.held
                } else {
                    &play.ground
                };
                o = o.fact("count", items.len());
                // Every code the bot has seen gets an answer, so `has.X=no`
                // reads as a fact rather than a missing one.
                for code in &play.codes {
                    let has = items.values().any(|i| &i.code == code);
                    o = o.fact(format!("has.{code}"), if has { "yes" } else { "no" });
                }
            }
            "area" => {
                let play = &bot.view.play;
                if let Some(act) = play.act {
                    o = o.fact("act", act);
                }
                if let Some(level) = own_position(bot).and_then(|p| play.level_at(p)) {
                    o = o.fact("area", play::area_name(level)).fact("level", level);
                }
            }
            _ => o = Outcome::error(format!("{verb} has no driver")),
        }
        o
    }

    /// `agree` on a group: do they all see the same players at the same
    /// places?
    fn agree(&self, bots: &[usize]) -> Outcome {
        let pic = |b: usize| -> BTreeMap<String, (u16, u16)> {
            self.bots[b]
                .view
                .players
                .values()
                .map(|p| (p.name.clone(), (p.x, p.y)))
                .collect()
        };
        let Some(&first) = bots.first() else {
            return Outcome::error("no bots");
        };
        let base = pic(first);
        for &b in &bots[1..] {
            let other = pic(b);
            if other != base {
                let diff = base
                    .iter()
                    .find(|(n, at)| other.get(*n) != Some(at))
                    .map(|(n, at)| {
                        format!("b{first} sees {n} at {at:?}, b{b} at {:?}", other.get(n))
                    })
                    .or_else(|| {
                        other
                            .keys()
                            .find(|n| !base.contains_key(*n))
                            .map(|n| format!("b{b} sees {n}, b{first} doesn't"))
                    })
                    .unwrap_or_default();
                return Outcome::ok()
                    .fact("agree", "no")
                    .fact("players", base.len())
                    .fact("diff", diff);
            }
        }
        Outcome::ok()
            .fact("agree", "yes")
            .fact("players", base.len())
    }

    /// The oracles after the last step.
    fn final_checks(&mut self) {
        for (b, (line, why)) in std::mem::take(&mut self.unasked) {
            self.failures.push(Failure {
                rule: core("core.conn-kept"),
                line,
                step: "(connection)".into(),
                bot: Some(b),
                expected: "open".into(),
                actual: format!("closed: {why}"),
            });
        }
        for b in &self.bots {
            if !b.view.bad.is_empty() {
                let first: Vec<&String> = b.view.bad.iter().take(3).collect();
                self.failures.push(Failure {
                    rule: core("core.messages-parse"),
                    line: 0,
                    step: "(delivered messages)".into(),
                    bot: Some(b.index),
                    expected: "every message parses".into(),
                    actual: format!("{} bad, first: {first:?}", b.view.bad.len()),
                });
            }
        }
        for p in self.server.panics() {
            self.failures.push(Failure {
                rule: core("core.no-panic"),
                line: 0,
                step: "(server output)".into(),
                bot: None,
                expected: "no panic".into(),
                actual: p,
            });
        }
    }
}
