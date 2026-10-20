//! The bug scenarios of `tools/scenarios/bugs/`: one `scenario 2` file per
//! bug ticket, each asserting the right behaviour so
//! it fails while the bug is there and passes once it's fixed.
//!
//! Without game files (CI): every file parses, plans the way `repro` would
//! (steps, checks and the input script), uses only what `repro` can build,
//! and follows the folder's rules: its name matches its file, and it holds
//! at least one check. With game files (`D2_GAME_DIR`, `#[ignore]`): every
//! scenario runs on the headless client and passes.

use std::path::{Path, PathBuf};

use d2_client::app::repro::{self, exit, RunOpts};
use d2_client::scenario_script::{Scenario, SCENARIO_FORMAT_V2};

/// The folder of bug scenarios.
fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/scenarios/bugs")
}

/// Every `BUG-<number>.scn` in the folder, sorted, with its text.
fn scenarios() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(folder())
        .expect("tools/scenarios/bugs")
        .map(|e| e.expect("a folder entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "scn"))
        .map(|p| {
            let file = p.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{file}: {e}"));
            (file, text)
        })
        .collect();
    out.sort();
    out
}

/// `BUG-013.scn` -> `013`.
fn number(file: &str) -> &str {
    file.strip_prefix("BUG-")
        .and_then(|f| f.strip_suffix(".scn"))
        .unwrap_or_else(|| panic!("{file}: bug scenarios are named BUG-<number>.scn"))
}

#[test]
fn every_bug_scenario_parses_and_plans() {
    let all = scenarios();
    assert!(!all.is_empty(), "no scenarios in {}", folder().display());
    for (file, text) in &all {
        let n = number(file);
        assert!(
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()),
            "{file}: bug scenarios are named BUG-<number>.scn"
        );
        let s = Scenario::parse(text).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(
            s.version, SCENARIO_FORMAT_V2,
            "{file}: `repro` runs `scenario 2`"
        );
        assert!(s.expansion, "{file}: `repro` runs expansion games only");
        assert!(
            s.name.starts_with(&format!("bug-{n}-")),
            "{file}: name `{}` should be bug-{n}-<short-name>",
            s.name
        );
        if let Some(c) = &s.character {
            assert_eq!(
                repro::unsupported_inline(c),
                None,
                "{file}: `repro` can't build this character yet"
            );
        }
        let p = repro::plan(&s, 0).unwrap_or_else(|e| panic!("{file}: {e}"));
        if !p.input.is_empty() {
            d2_client::world_view::input_script::parse(&p.input)
                .unwrap_or_else(|e| panic!("{file}: input: {e}"));
        }
        assert!(
            !p.checks.is_empty() || p.input.contains("wait-until"),
            "{file}: a bug scenario asserts the right behaviour with at least one check"
        );
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn every_bug_scenario_passes() {
    let dir = PathBuf::from(std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let mut failed = Vec::new();
    for (file, text) in scenarios() {
        let s = Scenario::parse(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
        let opts = RunOpts {
            out: tmp.join(format!("bugs-{}-{}", s.name, std::process::id())),
            game_dir: Some(dir.clone()),
            base_dir: folder(),
            keep_going: true,
            file: file.clone(),
            ..RunOpts::default()
        };
        match repro::run(&s, &opts) {
            Ok(r) if r.exit == exit::PASS => {}
            Ok(r) => failed.push(format!("{file}: {}", r.json["first_failure"])),
            Err(e) => failed.push(format!("{file}: {e}")),
        }
    }
    assert!(
        failed.is_empty(),
        "failing bug scenarios:\n{}",
        failed.join("\n")
    );
}
