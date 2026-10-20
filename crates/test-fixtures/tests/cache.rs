//! Parallel nextest processes share one completed synthetic archive build.
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn cache_worker() {
    let Some(root) = std::env::var_os("FIXTURE_CACHE_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let out = root.join(format!("worker-{}", std::process::id()));
    test_fixtures::cache::copy_install(&root, &out, &test_fixtures::synth::synthetic()).unwrap();
    let archives = d2_formats::mpq::ArchiveSet::open_dir(&out).unwrap();
    d2_data::bin::load(&archives, d2_data::bin::DEFAULT_LANGUAGE).unwrap();
}

fn worker(root: &Path) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cache_worker", "--nocapture"])
        .env("FIXTURE_CACHE_TEST_ROOT", root)
        .spawn()
        .unwrap()
}

#[test]
fn parallel_processes_share_one_build() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("fixture-cache-processes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut children: Vec<_> = (0..4).map(|_| worker(&root)).collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let cache = root.join("synthetic-install-cache-v1");
    let entries: Vec<_> = std::fs::read_dir(cache)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(entries.len(), 1);
    let marker = entries[0].join("ready-v1");
    let modified = std::fs::metadata(&marker).unwrap().modified().unwrap();
    assert!(worker(&root).wait().unwrap().success());
    assert_eq!(
        modified,
        std::fs::metadata(marker).unwrap().modified().unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}
