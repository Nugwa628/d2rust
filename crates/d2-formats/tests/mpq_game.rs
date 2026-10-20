//! MPQ reader tests against the player's own Diablo II install: the
//! required archives open with the expected header, listed files resolve
//! and decode to their block-table size, the patch archive's imploded blocks
//! decode, and an encrypted sound file's key is recovered without its name.
//!
//! Ignored by default (they need `D2_GAME_DIR`); run with
//! `cargo test -p d2-formats -- --ignored`. The full every-block check is
//! `cargo run --release -p mpq-tool -- check`.

use std::path::PathBuf;

use d2_formats::mpq::{flags, Archive};

/// The path of archive `name` in `D2_GAME_DIR`, matched case-insensitively.
fn game_file(name: &str) -> PathBuf {
    let dir = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set"));
    std::fs::read_dir(&dir)
        .expect("D2_GAME_DIR readable")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .unwrap_or_else(|| panic!("{name} not found in {}", dir.display()))
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn required_archives_open() {
    for name in d2_formats::REQUIRED_MPQS {
        let a = Archive::open(game_file(name)).unwrap();
        assert_eq!(a.header().format_version, 0, "{name}");
        assert_eq!(a.sector_size(), 4096, "{name}");
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn listed_files_resolve_and_decode() {
    let a = Archive::open(game_file("d2data.mpq")).unwrap();
    let names = a.listfile().unwrap().expect("d2data.mpq has a (listfile)");
    assert!(names.len() > 10_000);
    // Sample every 50th file to keep the debug-build test quick.
    for name in names.iter().step_by(50) {
        let index = a
            .find(name)
            .unwrap_or_else(|| panic!("{name} not in hash table"));
        let data = a.read(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            data.len(),
            a.block_table()[index].file_size as usize,
            "{name}"
        );
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn imploded_patch_archive_decodes() {
    let a = Archive::open(game_file("patch_d2.mpq")).unwrap();
    for (i, block) in a.block_table().iter().enumerate() {
        if block.has(flags::EXISTS) {
            assert!(
                !block.has(flags::ENCRYPTED),
                "patch blocks are not encrypted"
            );
            a.read_block(i, None)
                .unwrap_or_else(|e| panic!("block {i}: {e}"));
        }
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn unnamed_encrypted_wav_recovers_key() {
    let a = Archive::open(game_file("d2sfx.mpq")).unwrap();
    let (index, _) = a
        .block_table()
        .iter()
        .enumerate()
        .find(|(_, b)| b.has(flags::EXISTS) && b.has(flags::ENCRYPTED))
        .expect("an encrypted block");
    let key = a.recover_key(index).unwrap().expect("key recovered");
    let wav = a.read_block(index, Some(key)).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    let riff = u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]) as usize;
    assert_eq!(riff + 8, wav.len());
}
