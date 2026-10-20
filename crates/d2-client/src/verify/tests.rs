//! Tests for the verify harness: the CPU half of every synthetic case file, the `--perturb`
//! corruption per case, strict case parsing, COF unit composites, and the GPU seam's statuses.

use super::case::{self, CaseErrorKind, CaseKind};
use super::*;

fn cases() -> Vec<Case> {
    load_dir(&default_case_dir()).expect("render-cases load")
}

fn synthetic(c: &Case) -> Option<&case::Synthetic> {
    match &c.kind {
        CaseKind::Synthetic(s) => Some(s),
        CaseKind::Map(_) | CaseKind::Scene(_) => None,
    }
}

// Every synthetic case file passes its CPU half.
#[test]
fn every_synthetic_case_passes_its_cpu_half() {
    let cases = cases();
    let synth: Vec<_> = cases
        .iter()
        .filter_map(|c| synthetic(c).map(|s| (c, s)))
        .collect();
    assert!(
        synth.len() >= 8,
        "one case per Test vectors row with a CPU half"
    );
    for (c, s) in synth {
        let r = run_synthetic(&c.name, s, 0, &mut NotWired);
        assert_eq!(r.status, Status::GpuNotWired, "{}: {:?}", c.name, r.lines);
        assert!(
            !s.expects.is_empty(),
            "{}: a case states its expected pixels",
            c.name
        );
    }
}

// `--perturb N` makes every synthetic case fail with exactly N differing pixels.
#[test]
fn perturb_fails_every_synthetic_case_with_exactly_n() {
    for c in cases() {
        let Some(s) = synthetic(&c) else { continue };
        for n in [1, 7, 64] {
            let (_, _, cpu, _) = run_cpu(s, n).unwrap();
            assert_eq!(cpu.binned.mismatched, n, "{} --perturb {n}", c.name);
            let r = run_synthetic(&c.name, s, n, &mut NotWired);
            assert!(matches!(r.status, Status::Fail(_)), "{}: {:?}", c.name, r);
        }
    }
}

#[test]
fn perturb_counts_exactly_and_rejects_too_many() {
    let view = Rect::new(-2, 3, 7, 5);
    let clean = vec![10u8; 7 * 5 * 4];
    for n in 0..=35 {
        let mut img = clean.clone();
        perturb(&mut img, n).unwrap();
        assert_eq!(compare(&img, &clean, view).unwrap().mismatched, n);
    }
    assert!(perturb(&mut clean.clone(), 36).is_err());
}

#[test]
fn compare_reports_count_and_first_mismatch_in_screen_coords() {
    let view = Rect::new(-2, 3, 4, 2);
    let expected = vec![0u8; 4 * 2 * 4];
    let mut actual = expected.clone();
    actual[5 * 4 + 1] = 9; // pixel (1, 1) of the view
    actual[7 * 4] = 1;
    let m = compare(&actual, &expected, view).unwrap();
    assert_eq!(m.mismatched, 2);
    let f = m.first.unwrap();
    assert_eq!(
        (f.x, f.y, f.expected, f.actual),
        (-1, 4, [0; 4], [0, 9, 0, 0])
    );
    assert!(compare(&actual[4..], &expected, view).is_err());
}

/// A GPU stand-in that returns the CPU image (or a corrupted one).
struct Echo {
    corrupt: usize,
}

impl GpuCompositor for Echo {
    fn compose(&mut self, job: &GpuJob<'_>) -> GpuOutcome {
        let indexed = scene::compose_binned(job.items, job.bins, job.frames, job.maps, job.view);
        let mut indices = indexed.unwrap();
        perturb_indices(&mut indices, self.corrupt).unwrap();
        let rgba = scene::to_rgba(&indices, job.palette);
        GpuOutcome::Image { indices, rgba }
    }
}

struct Broken;

impl GpuCompositor for Broken {
    fn compose(&mut self, _job: &GpuJob<'_>) -> GpuOutcome {
        GpuOutcome::Error("device lost".into())
    }
}

struct Adapterless;

impl GpuCompositor for Adapterless {
    fn compose(&mut self, _job: &GpuJob<'_>) -> GpuOutcome {
        GpuOutcome::NoAdapter("none found".into())
    }
}

// The GPU seam reports each status (pass, not wired, no adapter, error) distinctly.
#[test]
fn gpu_seam_statuses() {
    let cases = cases();
    let c = cases.iter().find(|c| c.name == "synth-opaque").unwrap();
    let s = synthetic(c).unwrap();
    let r = run_synthetic(&c.name, s, 0, &mut Echo { corrupt: 0 });
    assert_eq!(r.status, Status::Pass);
    let r = run_synthetic(&c.name, s, 0, &mut Echo { corrupt: 3 });
    assert_eq!(r.status, Status::Fail("GPU half".into()));
    assert!(r
        .lines
        .iter()
        .any(|l| l.starts_with("GPU: 3 of 1024 pixels differ")));
    assert!(r
        .lines
        .iter()
        .any(|l| l.starts_with("GPU indices: 3 of 1024 bytes differ")));
    let r = run_synthetic(&c.name, s, 0, &mut Broken);
    assert!(matches!(r.status, Status::Error(_)));
    // No adapter is reported as such: never a pass, never a GPU failure.
    let r = run_synthetic(&c.name, s, 0, &mut Adapterless);
    assert_eq!(r.status, Status::NoAdapter("none found".into()));
    assert_eq!(r.status.label(), "NO ADAPTER");
    let mut sum = Summary::default();
    sum.add(&r.status);
    assert_eq!((sum.no_adapter, sum.exit_code()), (1, 2));
    // --perturb with a stand-in GPU that matches: both halves report exactly N, so the
    // perturbation proves the GPU comparison too.
    let r = run_synthetic(&c.name, s, 5, &mut Echo { corrupt: 0 });
    assert_eq!(r.status, Status::Fail("CPU and GPU halves".into()));
    assert!(r
        .lines
        .iter()
        .any(|l| l.starts_with("CPU binned: 5 of 1024")));
    assert!(r
        .lines
        .iter()
        .any(|l| l.starts_with("GPU indices: 5 of 1024")));
    assert!(r.lines.iter().any(|l| l.starts_with("GPU: 5 of 1024")));
    // Not wired is never a pass, and it changes the exit code.
    let mut sum = Summary::default();
    sum.add(&Status::Pass);
    assert_eq!(sum.exit_code(), 0);
    sum.add(&Status::GpuNotWired);
    assert_eq!(sum.exit_code(), 2);
    sum.add(&Status::Fail("x".into()));
    assert_eq!(sum.exit_code(), 1);
}

#[test]
fn map_case_is_todays_default() {
    let cases = cases();
    let c = cases.iter().find(|c| c.name == "map").unwrap();
    assert_eq!(
        c.kind,
        CaseKind::Map(case::MapCase {
            ds1: r"data\global\tiles\ACT1\TOWN\townN1.ds1".into(),
            wall_base: 80,
            view: None,
        })
    );
}

#[test]
fn map_helpers() {
    assert_eq!(
        map::file_stem(r"data\global\tiles\ACT1\TOWN\townN1.ds1"),
        "townN1"
    );
    let b = |x0, y0, x1, y1| crate::map::Bounds { x0, y0, x1, y1 };
    let v = map::full_view(b(-3, -5, 10, 8));
    assert_eq!((v.left, v.top, v.width, v.height), (-3, -5, 14, 14));
    let v = map::full_view(b(0, 0, 20000, 100));
    assert_eq!((v.width, v.left), (map::MAX_TEXTURE_SIDE, 10000 - 4096));
}

const MINIMAL: &str = "version = 1\nkind = \"synthetic\"\n\
    [[frame]]\nwidth = 1\nheight = 1\npixels = [3]\n\
    [[item]]\nframe = 0\nx = 0\ny = 0\n";

fn parse_err(text: &str) -> case::CaseError {
    case::parse("t", text).expect_err(text)
}

// Case files are parsed strictly: unknown keys and malformed values are errors.
#[test]
fn case_parsing_is_strict() {
    assert!(case::parse("t", MINIMAL).is_ok());
    let e = parse_err(&MINIMAL.replace("version = 1\n", ""));
    assert_eq!(e.kind, CaseErrorKind::MissingVersion);
    let e = parse_err(&MINIMAL.replace("version = 1", "version = 2"));
    assert_eq!(e.kind, CaseErrorKind::UnsupportedVersion(2));
    let e = parse_err(&MINIMAL.replace("version = 1", "version = \"1\""));
    assert_eq!(e.kind, CaseErrorKind::WrongType("an integer"));
    let e = parse_err(&MINIMAL.replace("synthetic", "sprite"));
    assert_eq!(e.kind, CaseErrorKind::UnknownValue("sprite".into()));
    let e = parse_err(&format!("{MINIMAL}colour = 3\n"));
    assert_eq!(
        (e.at.as_str(), e.kind),
        ("item[0].colour", CaseErrorKind::UnknownKey)
    );
    let e = parse_err(&format!("ds1 = 'x'\n{MINIMAL}"));
    assert_eq!((e.at.as_str(), e.kind), ("ds1", CaseErrorKind::UnknownKey));
    let e = parse_err(&MINIMAL.replace("pixels = [3]", "pixels = [3, 4]"));
    assert_eq!(e.at, "frame[0].pixels");
    let e = parse_err(&MINIMAL.replace("pixels = [3]", "pixels = [256]"));
    assert_eq!(
        (e.at.as_str(), e.kind),
        (
            "frame[0].pixels[0]",
            CaseErrorKind::Range {
                value: 256,
                min: 0,
                max: 255
            }
        )
    );
    let e = parse_err(&MINIMAL.replace("pixels = [3]", "pixels = [3]\nfill = 3"));
    assert_eq!(e.at, "frame[0]");
    let e = parse_err(&MINIMAL.replace("x = 0\n", ""));
    assert_eq!(
        (e.at.as_str(), e.kind),
        ("item[0].x", CaseErrorKind::Missing)
    );
    let e = parse_err(&format!("view = [0, 0, 0, 4]\n{MINIMAL}"));
    assert_eq!(e.at, "view[2]");
    let e = parse_err(&format!("view = [0, 0, 4]\n{MINIMAL}"));
    assert_eq!(e.at, "view");
    let e = parse_err(&format!(
        "{MINIMAL}[[map]]\nbase = \"identity\"\nset = [[1, 2], [1, 3]]\n"
    ));
    assert_eq!(e.at, "map[0].set[1]");
    let e = parse_err(&format!("{MINIMAL}[[table]]\nrule = \"mul\"\n"));
    assert_eq!(e.kind, CaseErrorKind::UnknownValue("mul".into()));
    let e = parse_err("version = 1\nkind = \"synthetic\"\n");
    assert_eq!(e.at, "item");
    let e = parse_err("version = 1\nkind = \"map\"\nds1 = 'a'\n");
    assert_eq!(
        (e.at.as_str(), e.kind),
        ("wall_base", CaseErrorKind::Missing)
    );
    let e = parse_err("version = 1\nkind = \"map\"\nds1 = 'a'\nwall_base = 1\nframe = 1\n");
    assert_eq!(
        (e.at.as_str(), e.kind),
        ("frame", CaseErrorKind::UnknownKey)
    );
    let e = parse_err("version = 1\nkind = [");
    assert!(matches!(e.kind, CaseErrorKind::Toml(_)), "{e}");
}

#[test]
fn build_rejects_undefined_references_and_scene_errors() {
    let with = |extra: &str| {
        let c = case::parse("t", &format!("{MINIMAL}{extra}")).unwrap();
        let CaseKind::Synthetic(s) = c.kind else {
            unreachable!()
        };
        s
    };
    let s = with("shade = [0]\n");
    assert!(matches!(
        build(&s),
        Err(BuildError::Undefined { what: "map", .. })
    ));
    let s = with("table = 0\n");
    assert!(matches!(
        build(&s),
        Err(BuildError::Undefined { what: "table", .. })
    ));
    let s = with("[[item]]\nframe = 1\nx = 0\ny = 0\n");
    assert!(matches!(
        build(&s),
        Err(BuildError::Undefined {
            item: 1,
            what: "frame",
            ..
        })
    ));
    let s = with("key = [16, 0, 0, 0]\n");
    assert!(matches!(build(&s), Err(BuildError::Scene { item: 0, .. })));
    let s = with("key = [0, 0, 0, 256]\n");
    assert!(matches!(build(&s), Err(BuildError::Scene { item: 0, .. })));
    let s = with(
        "[[map]]\nbase = \"zero\"\n[[item]]\nframe = 0\nx = 0\ny = 0\nshade = [0, 0, 0, 0, 0]\n",
    );
    assert!(matches!(build(&s), Err(BuildError::Scene { item: 1, .. })));
    // flip_x is reserved: the compositor rejects it, so the case errors.
    let s = with("flip_x = true\n");
    let r = run_synthetic("t", &s, 0, &mut NotWired);
    assert!(matches!(r.status, Status::Error(_)), "{r:?}");
    // An expectation outside the view fails the CPU half.
    let s = with("[[expect]]\nx = 900\ny = 0\nindex = 0\n");
    let r = run_synthetic("t", &s, 0, &mut NotWired);
    assert_eq!(r.status, Status::Fail("CPU half".into()));
}

#[test]
fn a_wrong_expectation_fails_the_cpu_half() {
    let c = case::parse(
        "t",
        &format!("{MINIMAL}[[expect]]\nx = 0\ny = 0\nindex = 4\n"),
    )
    .unwrap();
    let CaseKind::Synthetic(s) = c.kind else {
        unreachable!()
    };
    let r = run_synthetic("t", &s, 0, &mut NotWired);
    assert_eq!(r.status, Status::Fail("CPU half".into()));
    assert!(
        r.lines.iter().any(|l| l == "CPU: expect (0, 0) = 4, got 3"),
        "{r:?}"
    );
}

// COF cases compose units from COF bytes into scene items.
#[test]
fn cof_cases_compose_units_from_cof_bytes() {
    let cases = cases();
    for name in ["synth-cof-units", "synth-cof-frames"] {
        let c = cases.iter().find(|c| c.name == name).expect(name);
        let s = synthetic(c).unwrap();
        assert!(!s.units.is_empty(), "{name}");
        let built = build(s).unwrap();
        // One item per COF layer (3) per unit, plus the plain items.
        assert_eq!(built.items.len(), s.items.len() + 3 * s.units.len());
        // Units keep their slot order as key `sub`.
        let units: Vec<_> = built
            .items
            .iter()
            .filter(|i| matches!(i.tag, ItemTag::Unit(_)))
            .collect();
        for w in units.windows(2) {
            if w[0].tag == w[1].tag {
                assert!(w[0].key.sub() < w[1].key.sub(), "{name}");
            }
        }
    }
    // Cross-unit order: unit 1 (major 50) is listed second but drawn first.
    let c = cases.iter().find(|c| c.name == "synth-cof-units").unwrap();
    let built = build(synthetic(c).unwrap()).unwrap();
    let tags: Vec<ItemTag> = built.items.iter().map(|i| i.tag).collect();
    assert_eq!(
        tags,
        [
            ItemTag::None,
            ItemTag::Unit(1),
            ItemTag::Unit(1),
            ItemTag::Unit(1),
            ItemTag::Unit(0),
            ItemTag::Unit(0),
            ItemTag::Unit(0),
        ]
    );
}

const UNIT: &str = "version = 1\nkind = \"synthetic\"\n\
    [[frame]]\nwidth = 1\nheight = 1\nfill = 3\n\
    [[unit]]\ncof = \"01 01 01 14 00000000 00000000 00000000 00000000 00000000 19000000 \
    01 00 01 00 00 68746800 00 01\"\ndir = 0\nframe = 0\nkey = [0, 0, 0]\n";

fn unit_case(extra: &str) -> case::Synthetic {
    let c = case::parse("t", &format!("{UNIT}{extra}")).unwrap();
    let CaseKind::Synthetic(s) = c.kind else {
        unreachable!()
    };
    s
}

#[test]
fn unit_parsing_and_building_are_strict() {
    // A minimal one-layer COF (TR) with its component: one item.
    let s = unit_case("[[unit.component]]\ncomponent = 1\nframe = 0\nx = 2\ny = 3\n");
    assert_eq!(s.units[0].cof.len(), 28 + 9 + 1 + 1);
    let built = build(&s).unwrap();
    assert_eq!(built.items.len(), 1);
    assert_eq!((built.items[0].x, built.items[0].y), (2, 3));
    // The COF draws component 1 and the case does not answer for it.
    let s = unit_case("[[unit.component]]\ncomponent = 0\nframe = 0\nx = 0\ny = 0\n");
    assert!(matches!(
        build(&s),
        Err(BuildError::Composite {
            unit: 0,
            error: CompositeError::Unresolved { component: 1, .. }
        })
    ));
    // Undefined frame / map in a component answer.
    let s = unit_case("[[unit.component]]\ncomponent = 1\nframe = 1\nx = 0\ny = 0\n");
    assert!(matches!(
        build(&s),
        Err(BuildError::UnitUndefined { what: "frame", .. })
    ));
    let s = unit_case("[[unit.component]]\ncomponent = 1\nframe = 0\nx = 0\ny = 0\nshade = [0]\n");
    assert!(matches!(
        build(&s),
        Err(BuildError::UnitUndefined { what: "map", .. })
    ));
    // COF direction out of range, and COF bytes the parser refuses.
    let mut s = unit_case("[[unit.component]]\ncomponent = 1\nframe = 0\nx = 0\ny = 0\n");
    s.units[0].dir = 1;
    assert!(matches!(
        build(&s),
        Err(BuildError::Composite {
            error: CompositeError::Direction { .. },
            ..
        })
    ));
    s.units[0].dir = 0;
    s.units[0].cof.truncate(20);
    assert!(matches!(build(&s), Err(BuildError::Cof { unit: 0, .. })));
    // Parse errors: bad hex, duplicate component, unknown key.
    for (extra, at) in [
        (
            "[[unit.component]]\ncomponent = 1\nframe = 0\nx = 0\ny = 0\ncolour = 1\n",
            "unit[0].component[0].colour",
        ),
        (
            "[[unit.component]]\ncomponent = 1\nframe = 0\nx = 0\ny = 0\n\
             [[unit.component]]\ncomponent = 1\nframe = 0\nx = 0\ny = 0\n",
            "unit[0].component[1].component",
        ),
        (
            "[[unit.component]]\ncomponent = 16\nframe = 0\nx = 0\ny = 0\n",
            "unit[0].component[0].component",
        ),
    ] {
        let e = case::parse("t", &format!("{UNIT}{extra}")).unwrap_err();
        assert_eq!(e.at, at, "{e}");
    }
    let bad_hex = UNIT.replace("19000000", "19 0g 0000");
    let e = case::parse("t", &bad_hex).unwrap_err();
    assert_eq!(e.at, "unit[0].cof", "{e}");
    let odd_hex = UNIT.replace("19000000", "190000 0");
    assert_eq!(case::parse("t", &odd_hex).unwrap_err().at, "unit[0].cof");
}

/// The GPU half on this machine's adapter (`gpu::Wgpu`): every synthetic
/// case 0 differing bytes, and `--perturb 7` exactly 7 on both images. No
/// adapter is reported and fails the test: it is not a pass.
#[test]
#[ignore = "needs a GPU adapter (a software one such as lavapipe will do)"]
fn gpu_half_matches_cpu_on_every_synthetic_case() {
    let mut gpu = gpu::Wgpu::new();
    let line = gpu.open();
    println!("{line}");
    assert!(line.starts_with("adapter: "), "{line}");
    for c in cases() {
        let Some(s) = synthetic(&c) else { continue };
        let r = run_synthetic(&c.name, s, 0, &mut gpu);
        println!("{} {}: {:?}", r.status.label(), c.name, r.lines);
        assert_eq!(r.status, Status::Pass, "{}: {:?}", c.name, r.lines);
        let r = run_synthetic(&c.name, s, 7, &mut gpu);
        assert_eq!(
            r.status,
            Status::Fail("CPU and GPU halves".into()),
            "{}",
            c.name
        );
        for prefix in ["CPU binned: 7 of", "GPU indices: 7 of", "GPU: 7 of"] {
            assert!(
                r.lines.iter().any(|l| l.starts_with(prefix)),
                "{}: {prefix} {:?}",
                c.name,
                r.lines
            );
        }
    }
}
