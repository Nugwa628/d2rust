//! The server log: lines are appended, and a reopened log keeps the old ones.

mod support;

use d2rust_server::log::{ServerLog, FILE_NAME};

#[test]
fn lines_append_across_opens() {
    let dir = support::temp_dir("log").join("logs");
    {
        let log = ServerLog::open(&dir).unwrap();
        log.line("first");
    }
    let log = ServerLog::open(&dir).unwrap();
    log.line("second");
    assert_eq!(log.path(), dir.join(FILE_NAME));
    let text = std::fs::read_to_string(log.path()).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].ends_with("] first") && lines[1].ends_with("] second"));
}
