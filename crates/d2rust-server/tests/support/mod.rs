//! A fresh, empty folder per test under the system temp folder.

use std::path::PathBuf;

pub fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("d2rust-server-tests")
        .join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
