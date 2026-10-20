//! Backups preserve server records across restart, refuse damaged data and
//! existing destinations, and mark completion only after all copies succeed.

mod support;

use d2rust_server::backup::{snapshot, BackupError, COMPLETE};
use d2rust_server::characters::CharacterStore;

#[test]
fn snapshot_restores_records_and_leaves_live_saves_independent() {
    let dir = support::temp_dir("backup-roundtrip");
    let store = CharacterStore::open(dir.join("live")).unwrap();
    store.create("Ama", b"first record").unwrap();
    store.create("Barb", b"second record").unwrap();
    std::fs::write(store.dir().join("Ama.d2rc.tmp"), b"unfinished").unwrap();
    std::fs::write(store.dir().join("unrelated.txt"), b"excluded").unwrap();
    let backup = dir.join("snapshot");
    assert_eq!(snapshot(&store, &backup).unwrap(), 2);
    assert!(backup.join(COMPLETE).is_file());
    assert_eq!(std::fs::read_dir(&backup).unwrap().count(), 3);
    assert_eq!(
        std::fs::read(store.dir().join("Ama.d2rc")).unwrap(),
        std::fs::read(backup.join("Ama.d2rc")).unwrap()
    );
    store.save("Ama", b"later live save").unwrap();
    drop(store);
    let restored = CharacterStore::open(&backup).unwrap();
    assert_eq!(restored.load("ama").unwrap(), b"first record");
    assert_eq!(restored.load("Barb").unwrap(), b"second record");
    assert_eq!(
        CharacterStore::open(dir.join("live"))
            .unwrap()
            .load("Ama")
            .unwrap(),
        b"later live save"
    );
}

#[test]
fn corrupt_records_never_complete_a_backup() {
    let dir = support::temp_dir("backup-corrupt");
    let store = CharacterStore::open(dir.join("live")).unwrap();
    store.create("Ama", b"good").unwrap();
    store.create("Zed", b"bad").unwrap();
    std::fs::write(store.dir().join("Zed.d2rc"), b"broken").unwrap();
    let backup = dir.join("snapshot");
    assert!(matches!(
        snapshot(&store, &backup),
        Err(BackupError::Character(_))
    ));
    assert!(!backup.join(COMPLETE).exists());
    assert_eq!(store.load("Ama").unwrap(), b"good");
}

#[test]
fn newer_records_never_complete_a_backup() {
    let dir = support::temp_dir("backup-newer");
    let store = CharacterStore::open(dir.join("live")).unwrap();
    let mut bytes = d2rust_server::characters::encode(b"record");
    bytes[4..6].copy_from_slice(&(d2rust_server::characters::FORMAT_VERSION + 1).to_le_bytes());
    bytes.truncate(bytes.len() - 4);
    let checksum = bytes.iter().fold(0x811C_9DC5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    bytes.extend_from_slice(&checksum.to_le_bytes());
    std::fs::write(store.dir().join("Ama.d2rc"), &bytes).unwrap();
    let backup = dir.join("snapshot");
    assert!(matches!(
        snapshot(&store, &backup),
        Err(BackupError::Character(
            d2rust_server::characters::CharacterError::TooNew { .. }
        ))
    ));
    assert!(!backup.join(COMPLETE).exists());
    assert_eq!(std::fs::read(store.dir().join("Ama.d2rc")).unwrap(), bytes);
}

#[test]
fn existing_destination_and_nested_paths_are_refused() {
    let dir = support::temp_dir("backup-existing");
    let store = CharacterStore::open(dir.join("live")).unwrap();
    store.create("Ama", b"record").unwrap();
    let backup = dir.join("snapshot");
    snapshot(&store, &backup).unwrap();
    let before = std::fs::read(backup.join("Ama.d2rc")).unwrap();
    store.save("Ama", b"changed").unwrap();
    assert!(snapshot(&store, &backup).is_err());
    assert_eq!(std::fs::read(backup.join("Ama.d2rc")).unwrap(), before);
    assert!(matches!(
        snapshot(&store, &store.dir().join("nested")),
        Err(BackupError::InsideStore)
    ));
    assert!(!store.dir().join("nested").exists());
}

#[test]
fn non_file_character_entries_fail_without_completion() {
    let dir = support::temp_dir("backup-nonfile");
    let store = CharacterStore::open(dir.join("live")).unwrap();
    std::fs::create_dir(store.dir().join("Ama.d2rc")).unwrap();
    let backup = dir.join("snapshot");
    assert!(matches!(
        snapshot(&store, &backup),
        Err(BackupError::InvalidEntry(_))
    ));
    assert!(!backup.join(COMPLETE).exists());
}

#[test]
fn empty_store_has_a_completed_snapshot() {
    let dir = support::temp_dir("backup-empty");
    let store = CharacterStore::open(dir.join("live")).unwrap();
    let backup = dir.join("snapshot");
    assert_eq!(snapshot(&store, &backup).unwrap(), 0);
    assert!(backup.join(COMPLETE).is_file());
}

#[test]
fn cli_backup_copies_accounts_and_characters_without_game_data() {
    let dir = support::temp_dir("backup-cli");
    let chars = dir.join("data").join("characters").join("1");
    std::fs::create_dir_all(&chars).unwrap();
    let file = d2rust_server::characters::encode(b"record");
    std::fs::write(chars.join("Ama.d2rc"), &file).unwrap();
    let cfg = dir.join("server.cfg");
    std::fs::write(
        &cfg,
        "game_data = missing
argon2 = test
",
    )
    .unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_d2rust-server"))
            .current_dir(&dir)
            .arg("--config")
            .arg(&cfg)
            .args(args)
            .output()
            .unwrap()
    };
    let out = run(&["--backup", "snapshot"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("backed up 1 characters"));
    assert!(!dir.join("logs").exists());
    let snap = dir.join("snapshot");
    assert!(snap.join(COMPLETE).is_file());
    assert_eq!(
        std::fs::read(snap.join("characters").join("1").join("Ama.d2rc")).unwrap(),
        file
    );
    // An existing destination is refused; without one, a dated folder.
    assert!(!run(&["--backup", "snapshot"]).status.success());
    let out = run(&["--backup"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 1);
    assert_eq!(
        run(&["--check", "--backup", "other"]).status.code(),
        Some(2)
    );
    // Not while a server runs on the data folder.
    #[cfg(target_os = "linux")]
    {
        std::fs::write(
            dir.join("data").join("server.lock"),
            "1
",
        )
        .unwrap();
        let out = run(&["--backup", "while-running"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("the server runs"));
    }
}
