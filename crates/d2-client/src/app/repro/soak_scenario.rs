//! Lossless soak scenarios: retain the soak start, clock, actions and checks.
//! A separate versioned envelope avoids translating inventory GUID actions into
//! approximate mouse clicks or changing the soak's join/settle timing.

use super::{exit, CliArgs};
use crate::app::single_player::GameData;
use crate::app::soak::{self, log::SoakLog, SoakArgs};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub const FORMAT: &str = "soak-scenario 1";

pub struct Scenario {
    pub signature: String,
    pub steps: u32,
    pub log: SoakLog,
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Self> {
        let (format, body) = text.split_once('\n').context("missing scenario body")?;
        if format.trim() != FORMAT {
            bail!("expected {FORMAT}");
        }
        let v: Value = serde_json::from_str(body)?;
        let signature = v["signature"].as_str().context("signature must be text")?;
        if signature.is_empty() {
            bail!("empty signature");
        }
        let steps = v["steps"].as_u64().context("steps must be an integer")?;
        if steps == 0 || steps > u64::from(crate::scenario_script::MAX_END) {
            bail!("steps outside 1..=1000000");
        }
        let log = SoakLog::parse(v["log"].as_str().context("log must be text")?)
            .map_err(anyhow::Error::msg)?;
        if log.acts.iter().any(|(s, _)| u64::from(*s) >= steps) {
            bail!("action outside scenario duration");
        }
        Ok(Self {
            signature: signature.into(),
            steps: steps as u32,
            log,
        })
    }

    pub fn run(&self, data: &GameData, args: &SoakArgs) -> Value {
        let wall = std::time::Instant::now();
        let args = SoakArgs {
            steps: self.steps,
            keep_going: true,
            ..args.clone()
        };
        let out = soak::soak(data, &args, Some(&self.log), |_| {});
        let findings: Vec<Value> = out
            .findings
            .iter()
            .map(|f| serde_json::from_str(&f.to_json()).expect("finding JSON"))
            .collect();
        let steps: Vec<Value> = out
            .findings
            .iter()
            .map(|f| {
                json!({
                    "at": f.step, "kind": f.kind, "text": f.sig, "ok": false, "result": f.detail
                })
            })
            .collect();
        json!({"format": "repro 1", "source": FORMAT, "build": crate::launch::VERSION,
            "scenario": self.signature, "wall_ms": wall.elapsed().as_millis() as u64,
            "exit": if findings.is_empty() { exit::PASS } else { exit::FAIL },
            "first_failure": steps.first(), "steps": steps, "queries": [],
            "verdict": if findings.is_empty() { "pass" } else { "fail" },
            "ticks": out.steps, "signature": self.signature,
            "signature_reproduced": out.findings.iter().any(|f| f.sig == self.signature),
            "findings": findings})
    }
}

pub fn main(text: &str, cli: &CliArgs) -> u8 {
    let result = (|| -> Result<u8> {
        if cli.query.is_some() {
            bail!("soak scenarios do not support --query");
        }
        let mut s = Scenario::parse(text)?;
        if let Some(save) = &mut s.log.start.save {
            let path = Path::new(save);
            if path.is_relative() {
                *save = cli
                    .file
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(path)
                    .display()
                    .to_string();
            }
        }
        let dir = cli
            .game_dir
            .clone()
            .or_else(|| std::env::var_os("D2_GAME_DIR").map(Into::into));
        let data = match GameData::select(dir.as_deref()) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("repro: game files: {e:#}");
                return Ok(exit::NO_GAME);
            }
        };
        let out = cli
            .out
            .clone()
            .unwrap_or_else(|| Path::new("repro-out").join("soak"));
        std::fs::create_dir_all(&out)?;
        let report = s.run(
            &data,
            &SoakArgs {
                game_dir: dir,
                save_dir: Some(out.join("saves")),
                ..Default::default()
            },
        );
        std::fs::write(out.join("repro.json"), serde_json::to_vec_pretty(&report)?)?;
        println!(
            "repro: {} {} (signature reproduced: {})",
            report["verdict"], s.signature, report["signature_reproduced"]
        );
        Ok(if report["verdict"] == "pass" {
            exit::PASS
        } else {
            exit::FAIL
        })
    })();
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("repro: soak scenario: {e:#}");
            exit::INVALID
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(steps: Value, log: &str) -> String {
        format!(
            "{FORMAT}\n{}",
            json!({"signature":"test:failure", "steps":steps, "log":log})
        )
    }

    #[test]
    fn validates_version_duration_and_actions() {
        let log = "soak-log 1 tick_rate=60 new=amazon kit=0\n3 roundtrip\n";
        assert!(Scenario::parse(&envelope(json!(4), log)).is_ok());
        for legacy in [log.replace("tick_rate=60 ", ""), log.replace("60", "25")] {
            let error = Scenario::parse(&envelope(json!(4), &legacy))
                .err()
                .expect("legacy rate must be refused");
            assert!(error.to_string().contains("Re-record"), "{error}");
        }
        for steps in [json!(0), json!(-1), json!(3), json!(1_000_001), json!("4")] {
            assert!(Scenario::parse(&envelope(steps, log)).is_err());
        }
        assert!(Scenario::parse(&envelope(json!(4), "soak-log 2\n")).is_err());
        assert!(
            Scenario::parse(&envelope(json!(4), log).replace(FORMAT, "soak-scenario 2")).is_err()
        );
    }
}
