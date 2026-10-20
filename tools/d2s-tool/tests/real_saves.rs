//! Real-save checks on the user's own game saves (never committed):
//! every `*.d2s` in `D2_SAVE_DIR`, with tables from `D2_GAME_DIR`.
//!
//! `D2_GAME_DIR=<install> D2_SAVE_DIR=<save folder> cargo test -p d2s-tool --test real_saves -- --ignored`

use std::path::PathBuf;

use d2s_tool::tables::Tables;
use d2s_tool::{read_options, round_trip, RoundTrip};

/// Every save passes the checksum and header checks, its sections parse
/// in order, and the parsed model writes back byte for byte (the model
/// keeps +0x30 and the corpse u32, so no difference is allowed).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR and saves in D2_SAVE_DIR"]
fn real_saves_round_trip() {
    let Some(saves) = std::env::var_os("D2_SAVE_DIR").map(PathBuf::from) else {
        eprintln!(
            "D2_SAVE_DIR is not set: skipping (point it at the folder holding the .d2s files)"
        );
        return;
    };
    let game = std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR must name the game folder");
    let t = Tables::load_dir(&PathBuf::from(game)).unwrap_or_else(|e| panic!("{e:#}"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&saves)
        .unwrap_or_else(|e| panic!("{}: {e}", saves.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("d2s")))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .d2s in {}", saves.display());
    let mut bad = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let opts = read_options(&bytes, None);
        match round_trip(&bytes, &opts, &t) {
            Ok(RoundTrip::Same(_)) => eprintln!("{}: OK ({} bytes)", f.display(), bytes.len()),
            Ok(RoundTrip::Differs { offset, .. }) => {
                bad.push(format!("{}: rewrite differs at {offset:#x}", f.display()))
            }
            Err(e) => bad.push(format!("{}: {e:#}", f.display())),
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} saves failed:\n{}",
        bad.len(),
        files.len(),
        bad.join("\n")
    );
}
