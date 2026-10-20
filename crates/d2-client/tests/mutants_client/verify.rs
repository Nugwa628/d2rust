//! Mutation-testing gaps of the verify harness, CPU half.

use d2_client::scene::Rect;
use d2_client::verify::{compare, perturb, perturb_indices, FirstMismatch};

/// The first mismatch is reported at its screen position (view
/// origin plus column and row), not at a transposed one.
#[test]
fn first_mismatch_is_at_its_column_and_row() {
    let view = Rect::new(10, 20, 4, 3);
    let clean = vec![0u8; 4 * 3 * 4];
    let mut bad = clean.clone();
    // Pixel 6: column 2, row 1.
    bad[6 * 4] = 9;
    bad[11 * 4 + 1] = 7;
    let m = compare(&bad, &clean, view).unwrap();
    assert_eq!(m.pixels, 12);
    assert_eq!(m.mismatched, 2);
    assert_eq!(
        m.first,
        Some(FirstMismatch {
            x: 12,
            y: 21,
            expected: [0, 0, 0, 0],
            actual: [9, 0, 0, 0],
        })
    );
}

/// `--perturb N` changes exactly N pixels whatever their
/// value, top bit already set included.
#[test]
fn perturb_changes_exactly_n_pixels_with_the_top_bit_set() {
    let clean = vec![0xFFu8; 10 * 4];
    let mut rgba = clean.clone();
    perturb(&mut rgba, 3).unwrap();
    let changed = rgba
        .chunks(4)
        .zip(clean.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(changed, 3);

    let clean = vec![0x80u8; 10];
    let mut idx = clean.clone();
    perturb_indices(&mut idx, 4).unwrap();
    assert_eq!(idx.iter().zip(&clean).filter(|(a, b)| a != b).count(), 4);
}

/// Index half: the first differing byte is reported at its screen
/// position; buffers of the wrong size are an error, either one.
#[test]
fn index_compare_position_and_sizes() {
    use d2_client::verify::compare_indices;

    let view = Rect::new(10, 20, 4, 3);
    let clean = vec![0u8; 12];
    let mut bad = clean.clone();
    bad[6] = 9;
    let m = compare_indices(&bad, &clean, view).unwrap();
    assert_eq!(
        (m.bytes, m.mismatched, m.first),
        (12, 1, Some((12, 21, 0, 9)))
    );
    assert!(compare_indices(&bad[..11], &clean, view).is_err());
    assert!(compare_indices(&bad, &clean[..11], view).is_err());
}

const OFFSET_CASE: &str = r#"
version = 1
kind = "synthetic"
view = [10, 20, 8, 6]

[[frame]]
width = 1
height = 1
fill = 5

[[item]]
frame = 0
x = 13
y = 22

[[expect]]
x = 13
y = 22
index = 5

[[expect]]
x = 14
y = 22
index = 0
"#;

fn offset_case() -> d2_client::verify::case::Synthetic {
    use d2_client::verify::case::{parse, CaseKind};

    match parse("offset", OFFSET_CASE).unwrap().kind {
        CaseKind::Synthetic(s) => s,
        k => panic!("not synthetic: {}", k.name()),
    }
}

/// `[[expect]]` rows are screen positions: in a view not at the
/// origin they are read at (x − view.x, y − view.y) of the reference.
#[test]
fn expectations_are_read_relative_to_the_view() {
    let (_, _, cpu, reference) = d2_client::verify::run_cpu(&offset_case(), 0).unwrap();
    assert_eq!(cpu.expect_failures, Vec::<String>::new());
    assert_eq!(reference.indices[2 * 8 + 3], 5);
}

/// The report marks a perturbed run (it must fail) and only a
/// perturbed run.
#[test]
fn only_a_perturbed_run_is_marked() {
    use d2_client::verify::{run_synthetic, NotWired};

    let marked = |n| {
        run_synthetic("offset", &offset_case(), n, &mut NotWired)
            .lines
            .iter()
            .any(|l| l.contains("verify must FAIL"))
    };
    assert!(!marked(0));
    assert!(marked(1));
}

/// Per case "compare byte for byte": a GPU image whose indices match
/// but whose RGBA differs fails the case.
#[test]
fn gpu_rgba_mismatch_alone_fails_the_case() {
    use d2_client::scene::{compose_binned, to_rgba};
    use d2_client::verify::{run_synthetic, GpuCompositor, GpuJob, GpuOutcome, Status};

    struct BadRgba;
    impl GpuCompositor for BadRgba {
        fn compose(&mut self, job: &GpuJob<'_>) -> GpuOutcome {
            let indices =
                compose_binned(job.items, job.bins, job.frames, job.maps, job.view).unwrap();
            let mut rgba = to_rgba(&indices, job.palette);
            rgba[0] ^= 1;
            GpuOutcome::Image { indices, rgba }
        }
    }
    let r = run_synthetic("offset", &offset_case(), 0, &mut BadRgba);
    assert!(matches!(r.status, Status::Fail(_)), "{:?}", r.status);
}

/// Runner totals and exit code: 0 all pass, 1 any failure or error,
/// 2 otherwise incomplete (module doc of `Summary::exit_code`).
#[test]
fn summary_counts_every_status_and_exit_codes() {
    use d2_client::verify::{Status, Summary};

    let mut s = Summary::default();
    for st in [
        Status::Pass,
        Status::GpuNotWired,
        Status::GpuNotWired,
        Status::NoAdapter("x".into()),
        Status::NoAdapter("x".into()),
    ] {
        s.add(&st);
    }
    assert_eq!(
        (s.pass, s.not_wired, s.no_adapter, s.fail, s.error),
        (1, 2, 2, 0, 0)
    );
    assert_eq!(s.exit_code(), 2);
    let mut e = s;
    e.add(&Status::Error("x".into()));
    e.add(&Status::Error("x".into()));
    assert_eq!(e.error, 2);
    assert_eq!(e.exit_code(), 1);
    let mut f = s;
    f.add(&Status::Fail("x".into()));
    assert_eq!(f.exit_code(), 1);
    f.add(&Status::Error("x".into()));
    assert_eq!(f.exit_code(), 1);
    assert_eq!(Summary::default().exit_code(), 0);
}

/// Case kinds are reported by their file names (`kind = "…"`).
#[test]
fn case_kind_names() {
    use d2_client::verify::case::{CaseKind, MapCase};

    assert_eq!(CaseKind::Synthetic(offset_case()).name(), "synthetic");
    let map = MapCase {
        ds1: String::new(),
        wall_base: 0,
        view: None,
    };
    assert_eq!(CaseKind::Map(map).name(), "map");
}

/// Synthetic case format: every documented table rule parses
/// (`rule = "src" | "dest" | "add" | "xor"`).
#[test]
fn every_table_rule_parses() {
    use d2_client::verify::case::{parse, CaseKind, TableRule};

    let text = r#"
version = 1
kind = "synthetic"

[[table]]
rule = "src"

[[table]]
rule = "dest"

[[table]]
rule = "add"

[[table]]
rule = "xor"

[[frame]]
width = 1
height = 1
fill = 1

[[item]]
frame = 0
x = 0
y = 0
"#;
    let CaseKind::Synthetic(s) = parse("rules", text).unwrap().kind else {
        panic!("not synthetic");
    };
    assert_eq!(
        s.tables,
        vec![
            TableRule::Src,
            TableRule::Dest,
            TableRule::Add,
            TableRule::Xor
        ]
    );
}

/// Case files are strict: a TOML syntax error names its line.
#[test]
fn toml_syntax_error_names_its_line() {
    use d2_client::verify::case::{parse, CaseErrorKind};

    let e = parse("bad", "version = 1\nkind = \"synthetic\"\nview = [\n").unwrap_err();
    assert!(matches!(e.kind, CaseErrorKind::Toml(_)), "{e:?}");
    let e = parse("bad", "version = 1\n\n\nkind = = 2\n").unwrap_err();
    assert_eq!(e.at, "line 4");
}
