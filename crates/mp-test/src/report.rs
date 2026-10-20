//! The run's report: `mp-test.json` for tools and `mp-test.md` for
//! people. Every failure names the stated rule it breaks, the step and
//! line, what was expected and what happened.

use std::path::Path;

use serde_json::{json, Value};

/// A rule as it appears in a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleRef {
    /// Its id (`core.*` for the tool's own rules).
    pub id: String,
    /// The rule in words.
    pub text: String,
}

/// One broken rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The rule it breaks.
    pub rule: RuleRef,
    /// The scenario line (0: not one step, e.g. the server's start).
    pub line: usize,
    /// The step as written (or what was checked).
    pub step: String,
    /// Which bot, when it was one bot's.
    pub bot: Option<usize>,
    /// What should have held.
    pub expected: String,
    /// What happened.
    pub actual: String,
}

/// How a scenario ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every rule held.
    Pass,
    /// A rule broke.
    Fail,
    /// Written ahead of its feature; not run.
    Pending,
}

/// One scenario's result.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioReport {
    /// Its id.
    pub name: String,
    /// Its file.
    pub file: String,
    /// What it shows.
    pub about: String,
    /// How it ended.
    pub verdict: Verdict,
    /// Why it is pending.
    pub pending: Option<String>,
    /// The capabilities it needs.
    pub needs: Vec<String>,
    /// Its bots.
    pub bots: usize,
    /// Steps run.
    pub steps_run: usize,
    /// Wall time, milliseconds.
    pub ms: u64,
    /// Its broken rules.
    pub failures: Vec<Failure>,
    /// Datagrams relayed and dropped by the injected network conditions.
    pub datagrams: (u64, u64),
    /// The server's last output lines, when it failed.
    pub server_tail: Option<String>,
}

/// The whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The tool and its report format.
    pub tool: String,
    /// The server program (`synth` or `d2rust-server`) and its binary.
    pub server: String,
    /// The binary.
    pub server_bin: String,
    /// The capabilities the server announced (`None`: it didn't say).
    pub server_caps: Option<Vec<String>>,
    /// The base seed.
    pub seed: u64,
    /// Counts.
    pub passed: usize,
    /// Counts.
    pub failed: usize,
    /// Counts.
    pub pending: usize,
    /// Every scenario, in run order.
    pub scenarios: Vec<ScenarioReport>,
}

impl RuleRef {
    fn to_json(&self) -> Value {
        json!({ "id": self.id, "text": self.text })
    }
}

impl Failure {
    fn to_json(&self) -> Value {
        json!({
            "rule": self.rule.to_json(),
            "line": self.line,
            "step": self.step,
            "bot": self.bot,
            "expected": self.expected,
            "actual": self.actual,
        })
    }
}

impl Verdict {
    /// Its name in reports.
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::Pending => "pending",
        }
    }
}

impl ScenarioReport {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "file": self.file,
            "about": self.about,
            "verdict": self.verdict.name(),
            "pending": self.pending,
            "needs": self.needs,
            "bots": self.bots,
            "steps_run": self.steps_run,
            "ms": self.ms,
            "failures": self.failures.iter().map(Failure::to_json).collect::<Vec<_>>(),
            "datagrams": { "relayed": self.datagrams.0, "dropped": self.datagrams.1 },
            "server_tail": self.server_tail,
        })
    }
}

impl Report {
    /// The report for tools (`mp-test.json`, format 1).
    pub fn to_json(&self) -> Value {
        json!({
            "format": 1,
            "tool": self.tool,
            "server": self.server,
            "server_bin": self.server_bin,
            "server_caps": self.server_caps,
            "seed": self.seed,
            "passed": self.passed,
            "failed": self.failed,
            "pending": self.pending,
            "scenarios": self.scenarios.iter().map(ScenarioReport::to_json).collect::<Vec<_>>(),
        })
    }

    /// Writes `mp-test.json` and `mp-test.md` into `dir`.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(&self.to_json()).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("mp-test.json"), json)?;
        std::fs::write(dir.join("mp-test.md"), self.markdown())
    }

    /// The report for people.
    pub fn markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("# Multiplayer test report\n\n");
        s.push_str(&format!(
            "Server: `{}` ({}). Seed {}. **{} passed, {} failed, {} pending.**\n\n",
            self.server, self.server_bin, self.seed, self.passed, self.failed, self.pending
        ));
        if let Some(caps) = &self.server_caps {
            s.push_str(&format!("Server capabilities: {}\n\n", caps.join(", ")));
        }
        s.push_str(
            "| Scenario | Result | Bots | Steps | Time | Notes |\n|---|---|---|---|---|---|\n",
        );
        for r in &self.scenarios {
            let result = match r.verdict {
                Verdict::Fail => "**FAIL**",
                v => v.name(),
            };
            let notes = match r.verdict {
                Verdict::Pending => r.pending.clone().unwrap_or_default(),
                Verdict::Fail => format!("{} broken rule(s)", r.failures.len()),
                Verdict::Pass if r.datagrams.1 > 0 => {
                    format!(
                        "{} of {} datagrams dropped",
                        r.datagrams.1,
                        r.datagrams.0 + r.datagrams.1
                    )
                }
                Verdict::Pass => String::new(),
            };
            s.push_str(&format!(
                "| {} | {} | {} | {} | {:.1} s | {} |\n",
                r.name,
                result,
                r.bots,
                r.steps_run,
                r.ms as f64 / 1000.0,
                notes.replace('|', "/")
            ));
        }
        let failed: Vec<&ScenarioReport> = self
            .scenarios
            .iter()
            .filter(|r| r.verdict == Verdict::Fail)
            .collect();
        if !failed.is_empty() {
            s.push_str("\n## Broken rules\n");
            for r in failed {
                s.push_str(&format!("\n### {} (`{}`)\n\n", r.name, r.file));
                for f in &r.failures {
                    let who = f.bot.map(|b| format!(" b{b}")).unwrap_or_default();
                    s.push_str(&format!(
                        "- **{}**: {}\n  - line {}{}: `{}`\n  - expected: {}\n  - actual: {}\n",
                        f.rule.id, f.rule.text, f.line, who, f.step, f.expected, f.actual
                    ));
                }
                if let Some(t) = &r.server_tail {
                    s.push_str(&format!("\n<details><summary>server output</summary>\n\n```\n{t}\n```\n</details>\n"));
                }
            }
        }
        s
    }
}
