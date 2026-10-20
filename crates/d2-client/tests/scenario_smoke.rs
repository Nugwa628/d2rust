//! The scenario smoke set: the `scenario 2` files of `tests/smoke/` run
//! through `repro` (`app/repro`) on a synthetic install, with no game
//! files, so every pull request's `test` job runs them (CI also runs this
//! file when the change leaves `d2-client` out of the test scope). The
//! real-data scenarios of `tools/scenarios/` run in CI's `scenarios` job
//! on the merge train's batches instead (`tools/ci/scenarios.py`).
//!
//! The install is `test_fixtures::acts::all_acts` (every act, so the play
//! app's live build can create them all at start) plus the few things the
//! headless client and its data load read that the set leaves out: a
//! waypoint object class, padded `monmode` / `pettype` rows, the walk-back
//! field, the act palettes, the client's font tables and two UI images.
//! All of it is made up here; none of it comes from a game file.
//!
//! What the set covers: the client and the in-process server start and
//! join, input steps reach the world, dev lines reach the server, checks
//! read the server, the client view and the HUD, and the run is the same
//! every time.

use std::path::{Path, PathBuf};

use d2_client::app::repro::{self, exit, RunOpts};
use d2_client::scenario_script::Scenario;
use test_fixtures::sprites::{dc6_file, dc6_frames, Dc6Shape};
use test_fixtures::synth::Synthetic;

/// The smoke scenarios' folder.
fn smoke_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/smoke")
}

/// One test's work folder: the smoke install and the runs' artifacts,
/// removed when the test ends (passed or not), so CI's persistent target
/// folder doesn't collect installs.
struct Work(PathBuf);

impl Work {
    /// Builds the smoke install in a new folder named after `test`.
    fn new(test: &str) -> Work {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("scenario-smoke-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        build(&root.join("install"));
        Work(root)
    }

    /// Runs `s` on the install with its artifacts in `out`.
    fn run(&self, s: &Scenario, file: &str, out: &str) -> repro::ReproReport {
        let opts = RunOpts {
            out: self.0.join(out),
            game_dir: Some(self.0.join("install")),
            base_dir: smoke_dir(),
            keep_going: true,
            file: file.into(),
            ..RunOpts::default()
        };
        repro::run(s, &opts).unwrap_or_else(|e| panic!("{file}: {e}"))
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Pads `file` to `rows` rows, copies of its first with `id_col` renamed.
fn pad(data: &mut Synthetic, file: &str, id_col: &str, rows: usize) {
    let m = data.tables.files.get_mut(file).expect(file);
    let id = m.columns.iter().position(|c| c == id_col).expect(id_col);
    let mut row = m.rows[0].clone();
    while m.rows.len() < rows {
        row[id] = format!("pad{}", m.rows.len());
        m.rows.push(row.clone());
    }
}

/// A walk-back field: 256 × 256 cells, each the direction one step toward
/// the centre (128, 128), 8 at the centre.
fn expfield() -> Vec<u8> {
    let mut v = vec![0x0A, 0x01];
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend_from_slice(&256u32.to_le_bytes());
    for y in 0..256i32 {
        for x in 0..256i32 {
            v.push(match ((128 - x).signum(), (128 - y).signum()) {
                (0, -1) => 0,
                (1, -1) => 1,
                (1, 0) => 2,
                (1, 1) => 3,
                (0, 1) => 4,
                (-1, 1) => 5,
                (-1, 0) => 6,
                (-1, -1) => 7,
                _ => 8,
            });
        }
    }
    v
}

/// An act palette file at full size (the base palette, the remap tables
/// and the text colour maps the UI reads), filled with a pattern; the base
/// palette's unused fourth byte per colour is 0.
fn pl2(seed: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..443_175).map(|i| (i * seed % 251) as u8).collect();
    for c in v[..1024].chunks_mut(4) {
        c[3] = 0;
    }
    v
}

/// A font table (`Woo!`): 256 glyphs of 8 × 10, glyph N on DC6 frame N.
fn font_tbl() -> Vec<u8> {
    let mut d = b"Woo!".to_vec();
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&[0, 0]);
    d.extend_from_slice(&256u16.to_le_bytes());
    d.extend_from_slice(&[10, 8]);
    for c in 0..256u16 {
        d.extend_from_slice(&c.to_le_bytes());
        d.extend_from_slice(&[0, 8, 10, 0, 0, 0]);
        d.extend_from_slice(&c.to_le_bytes());
        d.extend_from_slice(&[0, 0, 0, 0]);
    }
    d
}

/// A one-direction DC6 of `frames` 8 × 8 frames.
/// A COF of no layers: 1 frame, 1 direction, version 20, one frame event.
fn empty_cof() -> Vec<u8> {
    let mut b = vec![0, 1, 1, 20, 0, 0, 0, 0];
    b.extend([0u8; 16]);
    b.extend(256u32.to_le_bytes());
    b.push(0);
    b
}

fn dc6(frames: u32) -> Vec<u8> {
    let shape = Dc6Shape {
        directions: 1,
        frames,
        width: 8,
        height: 8,
        panel: true,
    };
    dc6_file(&dc6_frames(shape, 3), 1, frames)
}

/// The font tables the client loads at start.
const FONTS: [&str; 6] = [
    "font8",
    "font16",
    "fontformal12",
    "font6",
    "fontformal11",
    "fontingamechat",
];

/// The UI images the headless client loads at start.
const UI_IMAGES: [&str; 2] = [
    r"data\global\ui\panel\800ctrlpnl7.dc6",
    r"data\global\ui\cursor\protate.dc6",
];

/// Builds the smoke install into `dir`.
fn build(dir: &Path) {
    let mut data = test_fixtures::acts::all_acts();
    // The data load's fix-ups read monmode row 15 and pettype row 7.
    pad(&mut data, "monmode.txt", "name", 16);
    pad(&mut data, "pettype.txt", "pet type", 8);
    // The town's waypoint object class: operate function 23, init function 17.
    let o = data.tables.files.get_mut("objects.txt").expect("objects");
    let name = o.columns.iter().position(|c| c == "Name").expect("Name");
    let wp = o
        .rows
        .iter()
        .position(|r| r[name] == "Waypoint")
        .expect("a Waypoint row");
    for (col, v) in [("OperateFn", "23"), ("InitFn", "17")] {
        let c = o.columns.iter().position(|x| x == col).expect(col);
        o.rows[wp][c] = v.into();
    }
    data.files
        .push((r"data\global\ExpField.D2".into(), expfield()));
    for act in 1..=5 {
        data.files.push((
            format!(r"data\global\palette\act{act}\pal.pl2"),
            pl2(act + 2),
        ));
    }
    for f in FONTS {
        data.files
            .push((format!(r"data\local\font\latin\{f}.tbl"), font_tbl()));
    }
    for f in UI_IMAGES {
        data.files.push((f.into(), dc6(64)));
    }
    // Each class's player modes: an empty COF (no layers), so the client
    // finds a COF for every mode a scenario puts the player in and draws
    // the player as nothing, instead of reporting the COF missing.
    for class in 0..7 {
        for mode in ["DT", "NU", "WL", "RN", "GH", "TN", "TW", "A1"] {
            data.files.push((
                format!(r"data\global\chars\C{class}\cof\C{class}{mode}hth.cof"),
                empty_cof(),
            ));
        }
    }
    test_fixtures::cache::copy_install(Path::new(env!("CARGO_TARGET_TMPDIR")), dir, &data)
        .expect("the smoke install builds");
}

fn parse(path: &Path) -> Scenario {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    Scenario::parse(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// Every `.scn` of the smoke folder, sorted.
fn smoke_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(smoke_dir())
        .expect("tests/smoke")
        .map(|e| e.expect("a folder entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "scn"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_smoke_scenario_passes() {
    let work = Work::new("every");
    let files = smoke_files();
    assert!(!files.is_empty(), "no scenarios in tests/smoke");
    let mut failed = Vec::new();
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let r = work.run(&parse(path), &name, &name);
        println!("{name}: {}", r.json);
        if r.exit != exit::PASS {
            failed.push(format!("{name}: {}", r.json["first_failure"]));
        }
    }
    assert!(failed.is_empty(), "failed: {failed:#?}");
}

/// A failing check fails the run (exit 1) and reports the value it read.
#[test]
fn a_failing_check_reports_the_value_it_read() {
    let work = Work::new("fails");
    let s = Scenario::parse(
        "scenario 2
tick_rate 60
name smoke-fails
seed 7
end 2
char class 0
at 1 check player.level == 2
",
    )
    .unwrap();
    let r = work.run(&s, "smoke-fails", "fails");
    assert_eq!(r.exit, exit::FAIL, "{}", r.json);
    assert_eq!(r.json["verdict"], "fail", "{}", r.json);
    let f = &r.json["first_failure"];
    assert_eq!(f["text"], "at 1 check player.level == 2", "{}", r.json);
    assert!(
        f["result"].as_str().is_some_and(|t| t.contains('1')),
        "{}",
        r.json
    );
}

/// Two runs of a scenario give the same report apart from the wall time.
#[test]
fn a_scenario_runs_the_same_way_twice() {
    let work = Work::new("twice");
    let s = parse(&smoke_dir().join("town-walk.scn"));
    let strip = |mut v: serde_json::Value| {
        v["wall_ms"] = serde_json::Value::Null;
        v
    };
    let a = work.run(&s, "town-walk", "a");
    let b = work.run(&s, "town-walk", "b");
    assert_eq!(strip(a.json), strip(b.json));
}

#[test]
fn invariant_opt_outs_leave_tick_results_unchanged() {
    let work = Work::new("checks-read-only");
    let mut s = parse(&smoke_dir().join("town-walk.scn"));
    let checked = work.run(&s, "town-walk", "checked");
    s.allow
        .extend(d2_client::app::checks::RULES.iter().map(|s| s.to_string()));
    let allowed = work.run(&s, "town-walk", "allowed");
    assert_eq!(checked.exit, exit::PASS, "{}", checked.json);
    assert_eq!(allowed.exit, exit::PASS, "{}", allowed.json);
    for file in ["state-tail.jsonl", "client-state.json"] {
        assert_eq!(
            std::fs::read(work.0.join("checked").join(file)).unwrap(),
            std::fs::read(work.0.join("allowed").join(file)).unwrap(),
            "{file}"
        );
    }
}
