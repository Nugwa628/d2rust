//! The bundle's round trip and every limit: a valid bundle survives writing
//! and reading unchanged, and each broken rule is rejected with its reason.

use std::path::PathBuf;

use image::{DynamicImage, RgbImage};

use crate::*;

fn meta() -> Meta {
    Meta {
        game_version: "0.1.0.42".into(),
        launcher_version: None,
        os: "windows".into(),
        time_unix: 1_760_000_000,
        note: "stuck in a wall".into(),
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    screenshot_png(&DynamicImage::ImageRgb8(RgbImage::new(w, h))).unwrap()
}

fn full() -> Bundle {
    Bundle::new(meta())
        .unwrap()
        .with_file(FileKind::Log, log_tail(&["INFO a: one", "WARN b: two"]))
        .unwrap()
        .with_file(FileKind::State, state_text(&["area Cold Plains (3)"]))
        .unwrap()
        .with_file(FileKind::Screenshot, png(8, 6))
        .unwrap()
}

/// A fresh empty folder under the temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("d2-report-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parts(b: &Bundle) -> Vec<(String, Vec<u8>)> {
    FileKind::ALL
        .into_iter()
        .filter_map(|k| b.file(k).map(|f| (k.file_name().to_owned(), f.to_vec())))
        .collect()
}

#[test]
fn a_bundle_round_trips_through_a_folder_and_through_parts() {
    let b = full();
    let d = TempDir::new("round");
    b.write_dir(&d.0).unwrap();
    assert_eq!(Bundle::read_dir(&d.0).unwrap(), b);
    assert_eq!(
        Bundle::from_parts(&b.manifest().to_json(), parts(&b)).unwrap(),
        b
    );
    let json = String::from_utf8(b.manifest().to_json()).unwrap();
    assert!(json.contains("\"format\": \"d2rust-bug-report\""));
    assert!(json.contains("\"version\": 1"));
    assert!(json.contains("\"name\": \"screenshot.png\""));
}

#[test]
fn the_screenshot_can_arrive_later_and_files_can_be_left_out() {
    let d = TempDir::new("later");
    let b = full().without_file(FileKind::Screenshot);
    b.write_dir(&d.0).unwrap();
    assert!(Bundle::read_dir(&d.0)
        .unwrap()
        .file(FileKind::Screenshot)
        .is_none());
    Bundle::add_to_dir(&d.0, FileKind::Screenshot, png(4, 4)).unwrap();
    let back = Bundle::read_dir(&d.0).unwrap();
    assert_eq!(back.manifest().files.len(), 3);
    // Writing a smaller bundle over it removes the dropped file.
    back.without_file(FileKind::Log).write_dir(&d.0).unwrap();
    assert!(!d.0.join("log.txt").exists());
    assert_eq!(Bundle::read_dir(&d.0).unwrap().manifest().files.len(), 2);
}

#[test]
fn field_limits_are_enforced() {
    let mut m = meta();
    m.note = "x".repeat(MAX_NOTE_CHARS + 1);
    assert!(matches!(Bundle::new(m), Err(ReportError::TooMany { .. })));
    m = meta();
    m.note = "é".repeat(MAX_NOTE_CHARS); // characters, not bytes
    assert!(Bundle::new(m).is_ok());
    for bad_os in ["", "win dows", "../x", "a\u{202e}b"] {
        let mut m = meta();
        m.os = bad_os.into();
        assert!(
            matches!(Bundle::new(m), Err(ReportError::Malformed(_))),
            "{bad_os:?}"
        );
    }
    let mut m = meta();
    m.game_version = "1".repeat(MAX_LABEL_CHARS + 1);
    assert!(Bundle::new(m).is_err());
}

#[test]
fn file_limits_are_enforced() {
    let b = || Bundle::new(meta()).unwrap();
    // Screenshot: PNG only, at most 4 MB.
    let not_png = b().with_file(FileKind::Screenshot, b"GIF89a....".to_vec());
    assert!(matches!(not_png, Err(ReportError::BadContent { .. })));
    let mut huge = png(2, 2);
    huge.resize(MAX_SCREENSHOT_BYTES as usize + 1, 0);
    assert!(matches!(
        b().with_file(FileKind::Screenshot, huge),
        Err(ReportError::TooLarge { .. })
    ));
    // Log: UTF-8, at most 2,000 lines.
    let lines: Vec<String> = (0..MAX_LOG_LINES + 1).map(|i| format!("l{i}")).collect();
    let long = lines.join("\n").into_bytes();
    assert!(matches!(
        b().with_file(FileKind::Log, long),
        Err(ReportError::TooMany { .. })
    ));
    assert!(b().with_file(FileKind::Log, vec![0xff, 0xfe]).is_err());
    // log_tail keeps the newest lines within the limits.
    let tail = log_tail(&lines);
    let text = String::from_utf8(tail.clone()).unwrap();
    assert_eq!(text.lines().count(), MAX_LOG_LINES);
    assert!(text.starts_with("l1\n") && text.ends_with("l2000\n"));
    let fat: Vec<String> = (0..MAX_LOG_LINES).map(|_| "y".repeat(3000)).collect();
    assert!(log_tail(&fat).len() as u64 <= MAX_LOG_BYTES);
    assert!(b().with_file(FileKind::Log, log_tail(&fat)).is_ok());
    // State: the strict dump format.
    assert!(b()
        .with_file(FileKind::State, b"area x\n".to_vec())
        .is_err());
    assert!(b()
        .with_file(FileKind::State, state_text(&["a\u{0}b\tc"]))
        .is_ok());
}

#[test]
fn the_screenshot_is_halved_until_it_fits() {
    // Noise doesn't compress: 2000 x 1200 RGB is about 7 MB as PNG.
    let mut img = RgbImage::new(2000, 1200);
    let mut x: u32 = 1;
    for p in img.pixels_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        p.0 = [x as u8, (x >> 8) as u8, (x >> 16) as u8];
    }
    let out = screenshot_png(&DynamicImage::ImageRgb8(img)).unwrap();
    assert!(out.len() as u64 <= MAX_SCREENSHOT_BYTES);
    let back = image::load_from_memory(&out).unwrap();
    assert!(back.width() < 2000);
}

#[test]
fn malformed_manifests_are_rejected_with_a_reason() {
    let b = full();
    let good = String::from_utf8(b.manifest().to_json()).unwrap();
    let check = |json: &str| Bundle::from_parts(json.as_bytes(), parts(&b));
    assert!(check(&good).is_ok());
    let cases: [(&str, String); 8] = [
        ("not json", "{".into()),
        ("not an object", "[]".into()),
        ("other format", good.replace("d2rust-bug-report", "zip")),
        (
            "newer version",
            good.replace("\"version\": 1", "\"version\": 2"),
        ),
        (
            "unknown field",
            good.replacen('{', "{\"run\": \"rm -rf /\",", 1),
        ),
        (
            "wrong type",
            good.replace("\"time_unix\": 1760000000", "\"time_unix\": \"now\""),
        ),
        (
            "path in a name",
            good.replace("\"log.txt\"", "\"../log.txt\""),
        ),
        (
            "bad hash",
            good.replacen("\"sha256\": \"", "\"sha256\": \"zz", 1),
        ),
    ];
    for (what, json) in cases {
        let e = check(&json).unwrap_err();
        assert!(!e.to_string().is_empty(), "{what}");
    }
    assert!(matches!(
        check(&good.replace("\"version\": 1", "\"version\": 2")),
        Err(ReportError::Unsupported { version: 2, .. })
    ));
    let huge = format!("{} {}", good, " ".repeat(MAX_MANIFEST_BYTES as usize));
    assert!(matches!(check(&huge), Err(ReportError::TooLarge { .. })));
}

#[test]
fn files_must_match_the_manifest() {
    let b = full();
    let json = b.manifest().to_json();
    let mut p = parts(&b);
    // Changed bytes: the hash (or size) no longer matches.
    p[1].1[0] ^= 1;
    assert!(matches!(
        Bundle::from_parts(&json, p),
        Err(ReportError::Mismatch { .. })
    ));
    let mut p = parts(&b);
    p.pop();
    assert!(matches!(
        Bundle::from_parts(&json, p),
        Err(ReportError::Missing(_))
    ));
    let lean = b.clone().without_file(FileKind::Log);
    assert!(matches!(
        Bundle::from_parts(&lean.manifest().to_json(), parts(&b)),
        Err(ReportError::Unlisted(_))
    ));
    let mut p = parts(&b);
    p.push(p[0].clone());
    assert!(matches!(
        Bundle::from_parts(&json, p),
        Err(ReportError::Duplicate(_))
    ));
    for name in ["../log.txt", "LOG.TXT", "log.txt\0", "report.json", "a.exe"] {
        let mut p = parts(&b);
        p.push((name.into(), b"x".to_vec()));
        assert!(
            matches!(
                Bundle::from_parts(&json, p),
                Err(ReportError::UnknownFile(_))
            ),
            "{name:?}"
        );
    }
    let fat = vec![(
        String::from("log.txt"),
        vec![b'a'; MAX_TOTAL_BYTES as usize],
    )];
    assert!(matches!(
        Bundle::from_parts(&json, fat),
        Err(ReportError::TooLarge { .. })
    ));
}

#[test]
fn a_folder_with_anything_extra_is_rejected() {
    let d = TempDir::new("extra");
    full().write_dir(&d.0).unwrap();
    std::fs::write(d.0.join("payload.exe"), b"MZ").unwrap();
    assert!(matches!(
        Bundle::read_dir(&d.0),
        Err(ReportError::UnknownFile(_))
    ));
    std::fs::remove_file(d.0.join("payload.exe")).unwrap();
    std::fs::create_dir(d.0.join("sub")).unwrap();
    assert!(Bundle::read_dir(&d.0).is_err());
    std::fs::remove_dir(d.0.join("sub")).unwrap();
    std::fs::write(d.0.join("log.txt"), vec![b'a'; MAX_LOG_BYTES as usize + 1]).unwrap();
    assert!(matches!(
        Bundle::read_dir(&d.0),
        Err(ReportError::TooLarge { .. })
    ));
    std::fs::remove_file(d.0.join("report.json")).unwrap();
    std::fs::remove_file(d.0.join("log.txt")).unwrap();
    assert!(matches!(
        Bundle::read_dir(&d.0),
        Err(ReportError::Missing("report.json"))
    ));
}

#[test]
fn state_dumps_parse_strictly() {
    let ok = state_text(&["build local build", "area Cold Plains (3)"]);
    assert_eq!(
        StateDump::parse(&ok).unwrap().lines,
        vec!["build local build", "area Cold Plains (3)"]
    );
    let many: Vec<String> = (0..100).map(|i| format!("{i}")).collect();
    assert_eq!(
        StateDump::parse(&state_text(&many)).unwrap().lines.len(),
        state::MAX_STATE_LINES
    );
    for bad in [
        &b"d2rust-state 2\n"[..],
        b"d2rust-state 1",
        b"d2rust-state 1\na\rb\n",
        b"d2rust-state 1\n\xff\n",
    ] {
        assert!(StateDump::parse(bad).is_err(), "{bad:?}");
    }
}
