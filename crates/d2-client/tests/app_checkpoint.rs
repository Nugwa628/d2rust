//! Goal-boundary checkpoints end to end: a warm session saves a boundary,
//! restores it with the same `state-1` bytes, and a chained tail gives the
//! same final state as the uninterrupted run.
//!
//! The synthetic part runs in CI; the real-data part is `#[ignore]` and
//! prints the BIGRIG measurements (cold start, warm fork, save/restore).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use d2_client::app::repro::{self, checkpoint::Checkpoint, exit, RunOpts, Session};
use d2_client::scenario_script::Scenario;

const SCRIPT: &str = "scenario 2
tick_rate 60
name checkpoint
seed 7
end 24
char class 0
char level 1
at 2 input move 400 300
at 14 input move 410 300
";

const TAIL: &str = "scenario 2
tick_rate 60
name tail
seed 7
end 12
char class 0
char level 1
at 2 input move 410 300
";

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("checkpoint-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn opts(root: &Path, dir: &Path, tag: &str) -> RunOpts {
    RunOpts {
        out: root.join(tag),
        game_dir: Some(dir.to_path_buf()),
        base_dir: dir.to_path_buf(),
        file: format!("{tag}.scn"),
        ..RunOpts::default()
    }
}

/// The last `state-1` line of a run's `state-end.jsonl`.
fn state_end(out: &Path) -> String {
    let text = std::fs::read_to_string(out.join("state-end.jsonl")).unwrap();
    text.lines()
        .rev()
        .find(|line| line.starts_with("{\"k\":\"snap\""))
        .expect("a state line")
        .to_owned()
}

fn install(dir: &Path) {
    let mut data = test_fixtures::play::playable();
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
    test_fixtures::cache::copy_install(Path::new(env!("CARGO_TARGET_TMPDIR")), dir, &data)
        .expect("the synthetic install builds");
}

fn check_a_boundary(dir: &Path, tag: &str) -> Checkpoint {
    let root = scratch(tag);
    let s = Scenario::parse(SCRIPT).unwrap();
    let tail = Scenario::parse(TAIL).unwrap();
    let mut session = Session::load(Some(dir)).unwrap();

    let full_opts = RunOpts {
        capture_state: true,
        ..opts(&root, dir, "full")
    };
    let full = session.run(&s, &full_opts).unwrap();
    assert_eq!(full.exit, exit::PASS, "{}", full.json);
    let uninterrupted = state_end(&full_opts.out);

    let save_opts = opts(&root, dir, "save");
    let cp = Checkpoint::save(&mut session, &s, 12, &save_opts).unwrap();
    assert_eq!(cp.at, 12);
    assert!(cp.state.starts_with("{\"k\":\"snap\""));

    let restore_opts = opts(&root, dir, "restore");
    let restored = cp.restore(&mut session, &restore_opts).unwrap();
    assert_eq!(restored.report.exit, exit::PASS, "{}", restored.report.json);
    assert_eq!(restored.state, cp.state);

    let merged = cp.chain(&tail).unwrap();
    assert_eq!(merged.end, 24);
    assert_eq!(
        merged.steps.iter().map(|s| s.tick).collect::<Vec<_>>(),
        [2, 14]
    );
    let merged_opts = RunOpts {
        capture_state: true,
        ..opts(&root, dir, "merged")
    };
    let merged = session.run(&merged, &merged_opts).unwrap();
    assert_eq!(merged.exit, exit::PASS, "{}", merged.json);
    assert_eq!(state_end(&merged_opts.out), uninterrupted);
    cp
}

#[test]
fn a_checkpoint_restores_and_a_chained_tail_matches_the_uninterrupted_run() {
    let root = scratch("synthetic-install");
    let dir = root.join("install");
    install(&dir);
    check_a_boundary(&dir, "synthetic");
}

/// BIGRIG numbers for the game explorer's G2: cold start (data load per
/// case), warm fork (one session), checkpoint save/restore time and size.
#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn bigrig_checkpoint_measurements() {
    let dir = PathBuf::from(std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let root = scratch("measure");
    let s = Scenario::parse(SCRIPT).unwrap();
    let tail = Scenario::parse(TAIL).unwrap();

    let mut cold = Vec::new();
    for i in 0..3 {
        let o = opts(&root, &dir, &format!("cold-{i}"));
        let began = Instant::now();
        let r = repro::run(&s, &o).unwrap();
        assert_eq!(r.exit, exit::PASS, "{}", r.json);
        cold.push(began.elapsed());
    }

    let began = Instant::now();
    let mut session = Session::load(Some(&dir)).unwrap();
    let load = began.elapsed();
    let mut warm = Vec::new();
    for i in 0..3 {
        let o = opts(&root, &dir, &format!("warm-{i}"));
        let began = Instant::now();
        let r = session.run(&s, &o).unwrap();
        assert_eq!(r.exit, exit::PASS, "{}", r.json);
        warm.push(began.elapsed());
    }

    let full_opts = RunOpts {
        capture_state: true,
        ..opts(&root, &dir, "full")
    };
    session.run(&s, &full_opts).unwrap();
    let uninterrupted = state_end(&full_opts.out);
    let began = Instant::now();
    let cp = Checkpoint::capture(&full_opts.out, &s, s.end).unwrap();
    let capture = began.elapsed();
    let size = cp.size_bytes().unwrap();
    cp.write(&root.join("boundary.ckpt")).unwrap();

    let save_opts = opts(&root, &dir, "save");
    let began = Instant::now();
    let saved = Checkpoint::save(&mut session, &s, 12, &save_opts).unwrap();
    let save = began.elapsed();

    let mut restore = Vec::new();
    for i in 0..3 {
        let o = opts(&root, &dir, &format!("restore-{i}"));
        let began = Instant::now();
        let r = saved.restore(&mut session, &o).unwrap();
        assert_eq!(r.report.exit, exit::PASS, "{}", r.report.json);
        restore.push(began.elapsed());
    }

    let merged = saved.chain(&tail).unwrap();
    let merged_opts = RunOpts {
        capture_state: true,
        ..opts(&root, &dir, "merged")
    };
    session.run(&merged, &merged_opts).unwrap();
    assert_eq!(state_end(&merged_opts.out), uninterrupted);

    let ms = |v: &[Duration]| -> String {
        v.iter()
            .map(|d| format!("{:.0}", d.as_secs_f64() * 1000.0))
            .collect::<Vec<_>>()
            .join("/")
    };
    println!(
        "checkpoint measurement: cold_ms={} warm_ms={} load_ms={:.0} capture_ms={:.1} save_ms={:.0} restore_ms={} size_bytes={} saved_size_bytes={}",
        ms(&cold),
        ms(&warm),
        load.as_secs_f64() * 1000.0,
        capture.as_secs_f64() * 1000.0,
        save.as_secs_f64() * 1000.0,
        ms(&restore),
        size,
        saved.size_bytes().unwrap(),
    );
}
