//! The bundle parser against hostile uploads: path tricks in file names and
//! random bytes in any part. Whatever arrives, a bundle either parses into
//! the fixed file names or is rejected with a reason; it never panics and
//! never names a path of its own choosing.

use d2_report::*;
use proptest::prelude::*;

fn meta(note: &str) -> Meta {
    Meta {
        game_version: "0.1.0.42".into(),
        launcher_version: Some("0.1.0.7".into()),
        os: "windows".into(),
        time_unix: 1_760_000_000,
        note: note.into(),
    }
}

#[test]
fn path_tricks_in_file_names_are_rejected() {
    let b = Bundle::new(meta(""))
        .unwrap()
        .with_file(FileKind::Log, log_tail(&["INFO a: b"]))
        .unwrap();
    let json = b.manifest().to_json();
    let log = b.file(FileKind::Log).unwrap().to_vec();
    for name in [
        "../log.txt",
        "..\\log.txt",
        "/etc/passwd",
        "C:\\Windows\\log.txt",
        "log.txt/../../x",
        "./log.txt",
        "log.txt ",
        "log.txt\u{0}.png",
        "screenshot.png.exe",
        "",
    ] {
        let parts = vec![(name.to_owned(), log.clone())];
        assert!(
            matches!(
                Bundle::from_parts(&json, parts),
                Err(ReportError::UnknownFile(_))
            ),
            "{name:?}"
        );
    }
    // A manifest naming a path is rejected too.
    let bad = String::from_utf8(json.clone())
        .unwrap()
        .replace("\"log.txt\"", "\"../../log.txt\"");
    assert!(Bundle::from_parts(bad.as_bytes(), vec![("log.txt".into(), log)]).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Random bytes in any part of an upload never panic the parser, and
    /// whatever it accepts holds at most one file of each kind.
    #[test]
    fn random_uploads_never_panic(
        raw in proptest::collection::vec(any::<u8>(), 0..400),
        name in r"\PC{0,20}",
        body in proptest::collection::vec(any::<u8>(), 0..400),
    ) {
        if let Ok(b) = Bundle::from_parts(&raw, vec![(name, body)]) {
            prop_assert!(b.manifest().files.len() <= FileKind::ALL.len());
        }
    }
}
