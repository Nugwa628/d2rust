//! Python export -> repro CLI -> identical soak finding on synthetic data.
use d2_client::app::{
    single_player::GameData,
    soak::{self, log::SoakLog, SoakArgs},
};
use std::{path::PathBuf, process::Command};

#[test]
fn exported_failure_reproduces_its_signature() {
    let out = PathBuf::from(
        std::env::var_os("D2_FARM_TEST_ARTIFACTS")
            .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into()),
    )
    .join(format!("farm-export-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let install = out.join("game");
    test_fixtures::install::build(&install, &test_fixtures::play::playable()).unwrap();
    let data = GameData::select(Some(&install)).unwrap();
    // An invalid class is a deterministic setup failure, not a fabricated
    // invariant. No actions are needed, so this is already a minimal log.
    // A separate healthy replay below checks the normal join path.
    let text = "soak-log 1 tick_rate=60 new=invalid-class difficulty=0 kit=0\n";
    let log = SoakLog::parse(text).unwrap();
    let direct = soak::soak(
        &data,
        &SoakArgs {
            steps: 5,
            ..Default::default()
        },
        Some(&log),
        |_| {},
    );
    assert_eq!(direct.findings[0].sig, "setup:start");
    let path = out.join("reduced.log");
    std::fs::write(&path, text).unwrap();
    let scenario = out.join("failure.scn");
    let mut python = if cfg!(windows) {
        let mut c = Command::new("py");
        c.arg("-3");
        c
    } else {
        Command::new("python3")
    };
    let status = python
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/soak/soak.py"
        ))
        .arg("export")
        .arg(path)
        .args(["--sig", "setup:start", "--steps", "5", "--out"])
        .arg(&scenario)
        .status()
        .unwrap();
    assert!(status.success());
    let report_dir = out.join("report");
    let code = d2_client::app::repro::main(&[
        scenario.display().to_string(),
        "--out".into(),
        report_dir.display().to_string(),
        "--game-dir".into(),
        install.display().to_string(),
    ]);
    assert_eq!(code, 1);
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_dir.join("repro.json")).unwrap()).unwrap();
    assert_eq!(report["signature_reproduced"], true);
    assert_eq!(report["findings"][0]["sig"], direct.findings[0].sig);
    assert_eq!(report["findings"][0]["detail"], direct.findings[0].detail);
    let healthy = text.replace("invalid-class", "sorceress");
    let scenario = d2_client::app::repro::soak_scenario::Scenario::parse(&format!(
        "soak-scenario 1\n{}",
        serde_json::json!({"signature":"setup:start", "steps":5, "log":healthy})
    ))
    .unwrap();
    let report = scenario.run(
        &data,
        &SoakArgs {
            save_dir: Some(out.join("healthy-saves")),
            ..Default::default()
        },
    );
    assert_eq!(report["signature_reproduced"], false);
    assert_eq!(report["verdict"], "pass", "{report}");
}
