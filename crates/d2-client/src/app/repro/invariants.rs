//! Always-on checks at the same between-tick boundary as scenario probes.
use super::*;
use crate::app::checks::{self, always::Always};
use crate::bridge::mirror::BridgeResource;

#[derive(Default)]
pub(crate) struct Checks {
    tracker: Always,
    pub violations: Vec<Value>,
    frame: Option<u32>,
    ledger: checks::conservation::Conservation,
    sent: Value,
}

impl Checks {
    pub fn tick(
        &mut self,
        host: &mut Host,
        scenario: &Scenario,
        tick: u32,
        log: &StepLog,
        gotos_finished: &[u32],
    ) -> Result<()> {
        let (mut violations, server, extra, assets) = host
            .server()
            .lock()
            .map_err(|_| anyhow!("server lock"))?
            .with(|link| {
                let sim = &link.host().game;
                let (violations, server) = checks::server_check(sim);
                (
                    violations,
                    server,
                    checks::always::server_view(sim),
                    checks::conservation::snapshot(sim),
                )
            })
            .map_err(|e| anyhow!("{e:?}"))?;
        if self.frame == Some(server.frame) {
            return Ok(());
        }
        self.frame = Some(server.frame);
        let state = host.state()?;
        let sent = &state["client"]["sent"];
        let mut events = checks::conservation::events(scenario, tick, sent, &self.sent);
        if checks::conservation::goto_in_flight(scenario, gotos_finished, tick) {
            events.open_all();
        }
        violations.extend(self.ledger.observe(assets, events));
        self.sent = sent.clone();
        let world = host
            .app_mut()
            .world()
            .resource::<BridgeResource>()
            .0
            .world();
        violations.extend(self.tracker.observe(world, &server, &extra, &state));
        for violation in self.tracker.retain_new(violations, &scenario.allow) {
            push(
                log,
                Some(tick),
                "invariant",
                violation.signature(),
                false,
                violation.detail.clone(),
            );
            self.violations.push(json!({"rule":violation.kind,"signature":violation.signature(),"detail":violation.detail,"at":tick}));
        }
        Ok(())
    }
}

pub(super) fn run(
    scenario: &Scenario,
    opts: &RunOpts,
    f: impl FnOnce() -> Result<ReproReport, ReproError>,
) -> Result<ReproReport, ReproError> {
    let capture = checks::capture::Capture::default();
    let result = bevy::log::tracing::dispatcher::with_default(&capture.dispatch(), || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
    });
    let mut captured = capture.take();
    let (mut report, panic) = match result {
        Ok(Ok(report)) => (report, None),
        Ok(Err(error)) => {
            if captured.is_empty() || !matches!(error, ReproError::Run(_)) {
                return Err(error);
            }
            (
                ReproReport {
                    exit: exit::FAIL,
                    json: json!({"format":FORMAT,"scenario":scenario.name,"file":opts.file,"violations":[],"steps":[],"allowed_rules":scenario.allow,"verdict":"fail","exit":exit::FAIL,"run_error":error.to_string()}),
                },
                None,
            )
        }
        Err(payload) => {
            let detail = checks::capture::panic_text(payload.as_ref());
            (
                ReproReport {
                    exit: exit::FAIL,
                    json: json!({"format":FORMAT,"scenario":scenario.name,"file":opts.file,"violations":[],"steps":[],"allowed_rules":scenario.allow,"verdict":"fail","exit":exit::FAIL,"run_error":detail}),
                },
                Some(checks::Violation {
                    units: Vec::new(),
                    kind: "panic",
                    sig: "panic:repro".into(),
                    detail,
                }),
            )
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    captured.extend(capture.take());
    for violation in captured.into_iter().chain(panic) {
        if scenario.allow.contains(violation.kind) || !seen.insert(violation.signature()) {
            continue;
        }
        let failure = json!({"at":null,"kind":"invariant","text":violation.sig,"ok":false,"result":violation.detail});
        if report.json["first_failure"].is_null() {
            report.json["first_failure"] = failure;
        }
        report.json["violations"].as_array_mut().expect("violations array").push(json!({"rule":violation.kind,"signature":violation.signature(),"detail":violation.detail,"at":null}));
        report.exit = exit::FAIL;
    }
    if report.exit == exit::FAIL {
        report.json["exit"] = json!(exit::FAIL);
        report.json["verdict"] = json!("fail");
    }
    // run_inner has already written the state artifacts. Only replace the
    // report after draining errors, which may arrive during host teardown.
    std::fs::create_dir_all(&opts.out).with_context(|| format!("create {}", opts.out.display()))?;
    std::fs::write(
        opts.out.join("repro.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&report.json).context("serialize invariant report")?
        ),
    )
    .context("write invariant report")?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(
        tag: &str,
        allow: &str,
        f: impl FnOnce() -> Result<ReproReport, ReproError>,
    ) -> ReproReport {
        let s = Scenario::parse(&format!(
            "scenario 2\nname capture\nseed 7\nend 1\nchar class 0\n{allow}"
        ))
        .unwrap();
        let out = std::env::temp_dir().join(format!("d2-invariant-{tag}-{}", std::process::id()));
        let report = run(
            &s,
            &RunOpts {
                out: out.clone(),
                ..Default::default()
            },
            f,
        )
        .unwrap();
        let saved: Value =
            serde_json::from_slice(&std::fs::read(out.join("repro.json")).unwrap()).unwrap();
        assert_eq!(saved, report.json);
        std::fs::remove_dir_all(out).unwrap();
        report
    }

    fn pass() -> Result<ReproReport, ReproError> {
        Ok(ReproReport {
            exit: exit::PASS,
            json: json!({"violations":[],"verdict":"pass","exit":0}),
        })
    }

    #[test]
    fn panic_produces_a_failed_report() {
        let r = exercise("panic", "", || panic!("planted panic"));
        assert_eq!(r.exit, exit::FAIL);
        assert_eq!(r.json["violations"][0]["rule"], "panic");
        assert_eq!(r.json["violations"][0]["detail"], "planted panic");
    }

    #[test]
    fn errors_fail_and_repeated_callsites_are_deduplicated() {
        let r = exercise("errors", "", || {
            for tick in 0..3 {
                bevy::log::tracing::error!(tick, "planted error");
            }
            pass()
        });
        assert_eq!(r.exit, exit::FAIL);
        assert_eq!(r.json["violations"].as_array().unwrap().len(), 1);
        assert_eq!(r.json["violations"][0]["rule"], "error");
    }

    #[test]
    fn allowed_errors_and_missing_assets_stay_quiet() {
        let r = exercise("allowed", "allow error\nallow missing-asset\n", || {
            bevy::log::tracing::error!("expected failure");
            bevy::log::tracing::warn!(target: "d2_asset_fallback", "expected missing file");
            pass()
        });
        assert_eq!(r.exit, exit::PASS);
        assert!(r.json["violations"].as_array().unwrap().is_empty());
    }
}
