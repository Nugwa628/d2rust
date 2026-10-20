//! `repro` (`app/repro`) end to end: a `scenario 2` script with an inline
//! character runs on the headless play client, passes, and writes its
//! artifacts; a second run gives the same report apart from the wall time.
//! The BUG-013 scenario's checks read the gold label through the client,
//! and a failing check reports the value it read.
//!
//! The town-walk test runs in CI on the synthetic install of
//! `test_fixtures::play::playable` (the five acts plus the palettes, fonts
//! and HUD stand-ins the client reads). The same check and the BUG-013
//! ones on the user's game files (`D2_GAME_DIR`) are `#[ignore]`. The
//! parts that need no install (planning, checks, queries, the CLI, the
//! exit codes) are unit tests in `app/repro`.

use std::path::{Path, PathBuf};

use d2_client::app::repro::{self, exit, RunOpts};
use d2_client::scenario_script::Scenario;

const SCRIPT: &str = "scenario 2
tick_rate 60
name town-walk
seed 7
end 12
char class 0
char level 1
at 2 input move 400 300
";

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("repro-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Normalize only the numeric wall-time field, preserving all other bytes.
fn without_wall(mut report: String) -> String {
    let key = "\"wall_ms\": ";
    let start = report.find(key).expect("wall_ms field") + key.len();
    let end = start + report[start..].find(|c: char| !c.is_ascii_digit()).unwrap();
    report.replace_range(start..end, "0");
    report
}

/// Runs [`SCRIPT`] twice on the install in `game_dir`: both runs pass
/// with every artifact written, and their reports match apart from the
/// wall time.
fn passes_the_same_way_twice(game_dir: &Path, tag: &str) {
    let s = Scenario::parse(SCRIPT).expect("the script parses");
    let queries =
        repro::query::parse_names("hud.gold, ui.open(inventory), player.pos, player.mode").unwrap();
    let run = |n: &str| {
        let opts = RunOpts {
            out: scratch(&format!("{tag}-{n}")),
            game_dir: Some(game_dir.to_path_buf()),
            base_dir: game_dir.to_path_buf(),
            keep_going: false,
            file: "town-walk.scn".into(),
            queries: queries.clone(),
            query_at: vec![0, 12],
            capture_state: false,
        };
        let r = repro::run(&s, &opts).unwrap_or_else(|e| panic!("{e}"));
        assert!(opts.out.join("repro.json").is_file());
        assert!(opts.out.join("state-tail.jsonl").is_file());
        assert!(opts.out.join("client-state.json").is_file());
        let tail = std::fs::read_to_string(opts.out.join("state-tail.jsonl")).unwrap();
        let header: serde_json::Value = serde_json::from_str(tail.lines().next().unwrap()).unwrap();
        assert_eq!(header["tick_rate"], 60);
        let client: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(opts.out.join("client-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(client["tick_rate"], 60);
        r
    };
    let a = run("a");
    assert_eq!(a.exit, exit::PASS, "{}", a.json);
    assert_eq!(a.json["verdict"], "pass");
    assert_eq!(a.json["ticks"], 12);
    let b = run("b");
    let strip = |mut v: serde_json::Value| {
        v["wall_ms"] = serde_json::Value::Null;
        v
    };
    let expected = strip(a.json);
    assert_eq!(expected, strip(b.json));
    let mut session = repro::Session::load(Some(game_dir)).unwrap();
    let opts = RunOpts {
        out: scratch(&format!("{tag}-warm")),
        game_dir: Some(game_dir.to_path_buf()),
        base_dir: game_dir.to_path_buf(),
        file: "town-walk.scn".into(),
        queries: queries.clone(),
        query_at: vec![0, 12],
        ..RunOpts::default()
    };
    assert_eq!(expected, strip(session.run(&s, &opts).unwrap().json));
    let dirty_text = SCRIPT.replace(
        "at 2 input move 400 300",
        "at 2 input action toggle_inventory\nat 3 dev gold 123\nat 4 input down L 600 300",
    );
    let dirty = Scenario::parse(&dirty_text).unwrap();
    let changed = session.run(&dirty, &opts).unwrap();
    assert_eq!(changed.exit, exit::PASS, "{}", changed.json);
    assert_eq!(
        changed.json["queries"][1]["values"]["ui.open(inventory)"],
        true
    );
    assert_eq!(expected, strip(session.run(&s, &opts).unwrap().json));

    // Exercise the same reset through the actual batch scheduler and compare
    // first/last reports with a standalone run using the same file metadata.
    let file = scratch(&format!("{tag}-batch-input")).with_extension("scn");
    std::fs::write(&file, SCRIPT).unwrap();
    let dirty_file = file.with_extension("dirty.scn");
    std::fs::write(&dirty_file, dirty_text).unwrap();
    let batch_opts = RunOpts {
        out: scratch(&format!("{tag}-batch")),
        ..opts.clone()
    };
    let batch =
        repro::batch::run_batch(&[file.clone(), dirty_file, file.clone()], 1, &batch_opts).unwrap();
    assert_eq!(batch["totals"]["passed"], 3);
    let single_opts = RunOpts {
        file: file.display().to_string(),
        ..opts
    };
    let single = strip(repro::run(&s, &single_opts).unwrap().json);
    let single_bytes =
        without_wall(std::fs::read_to_string(single_opts.out.join("repro.json")).unwrap());
    for index in ["0001", "0003"] {
        let report = std::fs::read(batch_opts.out.join(index).join("repro.json")).unwrap();
        assert_eq!(single, strip(serde_json::from_slice(&report).unwrap()));
        assert_eq!(
            single_bytes,
            without_wall(String::from_utf8(report).unwrap())
        );
    }
    let parallel_opts = RunOpts {
        out: scratch(&format!("{tag}-parallel")),
        ..batch_opts
    };
    let batch = repro::batch::run_batch(&[file.clone(), file], 2, &parallel_opts).unwrap();
    assert_eq!(batch["totals"]["passed"], 2);
    for index in ["0001", "0002"] {
        let report = std::fs::read(parallel_opts.out.join(index).join("repro.json")).unwrap();
        assert_eq!(single, strip(serde_json::from_slice(&report).unwrap()));
        assert_eq!(
            single_bytes,
            without_wall(String::from_utf8(report).unwrap())
        );
    }

    // Legacy scenario time must have the same meaning through standalone and
    // warm-batch execution, including explicit query deadlines.
    let legacy_text = SCRIPT.replace("tick_rate 60\n", "");
    let legacy = Scenario::parse(&legacy_text).unwrap();
    let legacy_file = scratch(&format!("{tag}-legacy-input")).with_extension("scn");
    std::fs::write(&legacy_file, legacy_text).unwrap();
    let legacy_opts = RunOpts {
        out: scratch(&format!("{tag}-legacy")),
        file: legacy_file.display().to_string(),
        ..single_opts.clone()
    };
    let standalone = repro::run(&legacy, &legacy_opts).unwrap();
    assert_eq!(standalone.exit, exit::PASS);
    assert_eq!(standalone.json["scenario_tick_rate"], 25);
    assert_eq!(standalone.json["tick_rate"], 60);
    assert_eq!(standalone.json["ticks"], 29);
    assert_eq!(standalone.json["queries"][1]["at"], 29);
    let legacy_batch_opts = RunOpts {
        out: scratch(&format!("{tag}-legacy-batch")),
        ..legacy_opts
    };
    let batch = repro::batch::run_batch(&[legacy_file], 1, &legacy_batch_opts).unwrap();
    assert_eq!(batch["totals"]["passed"], 1);
    let report = std::fs::read(legacy_batch_opts.out.join("0001/repro.json")).unwrap();
    assert_eq!(
        strip(standalone.json),
        strip(serde_json::from_slice(&report).unwrap())
    );
}

#[test]
fn a_scenario_passes_the_same_way_twice_on_synthetic_data() {
    let dir = scratch("install");
    let mut data = test_fixtures::play::playable();
    // The dirty middle run opens inventory; its art is generated stand-ins.
    for name in [
        "invchar6",
        "800borderframe",
        "goldcoinbtn",
        "buysellbtn",
        "inv_armor",
        "inv_belt",
        "inv_boots",
        "inv_helm_glove",
        "inv_ring_amulet",
        "inv_weapons",
    ] {
        data.stored.push((
            format!(r"data\global\ui\panel\{name}.dc6"),
            test_fixtures::play::stand_in_dc6(),
        ));
    }
    test_fixtures::cache::copy_install(Path::new(env!("CARGO_TARGET_TMPDIR")), &dir, &data)
        .expect("the synthetic install builds");
    passes_the_same_way_twice(&dir, "synthetic");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn a_scenario_passes_the_same_way_twice() {
    let dir = PathBuf::from(std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR"));
    passes_the_same_way_twice(&dir, "real");
}

/// The BUG-013 scenario of `tools/scenarios/bugs/`.
fn bug_013() -> Scenario {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/scenarios/bugs/BUG-013.scn"
    );
    let text = std::fs::read_to_string(path).expect("BUG-013.scn");
    Scenario::parse(&text).expect("BUG-013.scn parses")
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn checks_read_the_gold_label_and_queries_answer() {
    let dir = PathBuf::from(std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let opts = RunOpts {
        out: scratch("bug-013"),
        game_dir: Some(dir.clone()),
        base_dir: dir,
        file: "BUG-013.scn".into(),
        queries: repro::query::parse_names("ground_item(gold).label, ground_item(gold).gold")
            .unwrap(),
        query_at: vec![2, 14],
        ..RunOpts::default()
    };
    let r = repro::run(&bug_013(), &opts).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.exit, exit::PASS, "{:#}", r.json);
    let checks: Vec<&serde_json::Value> = r.json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == "check")
        .collect();
    assert_eq!(checks.len(), 2, "{:#}", r.json);
    assert_eq!(
        r.json["queries"],
        serde_json::json!([
            {"at": 5, "values": {"ground_item(gold).label": null, "ground_item(gold).gold": null}},
            {"at": 34, "values": {"ground_item(gold).label": "1234 Gold", "ground_item(gold).gold": 1234}},
        ])
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn a_failing_check_reports_the_value_it_read() {
    let dir = PathBuf::from(std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let mut s = bug_013();
    let text = s
        .to_text()
        .replace("label == \"1234 Gold\"", "label == \"Gold\"");
    s = Scenario::parse(&text).unwrap();
    let opts = RunOpts {
        out: scratch("bug-013-fail"),
        game_dir: Some(dir.clone()),
        base_dir: dir,
        file: "BUG-013-fail.scn".into(),
        ..RunOpts::default()
    };
    let r = repro::run(&s, &opts).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.exit, exit::FAIL, "{:#}", r.json);
    let f = &r.json["first_failure"];
    assert_eq!(f["kind"], "check");
    assert_eq!(f["at"], 29);
    assert_eq!(
        f["result"], "never held by tick 29; last: \"1234 Gold\"",
        "{:#}",
        r.json
    );
}

#[path = "farm_soak_scenario/mod.rs"]
mod farm_soak_scenario;
