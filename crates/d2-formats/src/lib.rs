//! Readers for the Diablo II: Lord of Destruction file formats that D2RUST loads from the
//! player's own game install at run time.
//!
//! Every reader takes a byte slice (or, for MPQ, a path), checks it and returns an owned
//! model or a [`FormatError`]; none of them panics on malformed input. Binary parsing
//! uses explicit little-endian reads, never transmutes. Other D2RUST crates (client,
//! server, tools) read files out of the archives with [`mpq`] and hand the bytes to the
//! readers below.
//!
//! Modules:
//!
//! - [`mpq`]: `.mpq` archives, the container for all game data; opens one archive or a
//!   stacked set, decrypts and decompresses file sectors.
//! - [`dc6`]: DC6 sprites, run-length encoded 8-bit indexed frames (UI, items, text).
//! - [`dcc`]: DCC animations, cell-based compressed indexed frames (characters, monsters).
//! - [`dt1`]: DT1 tile libraries, tile metadata and block pixels for the map.
//! - [`ds1`]: DS1 map presets, layered tile grids, objects, groups and NPC paths.
//! - [`cof`]: COF files, which layers make up an animation, frame events and draw order.
//! - [`animdata`]: `AnimData.d2`, frames, speed and event bytes per animation name.
//! - [`palette`]: `.dat` palettes and `.pl2` palette transforms.
//! - [`font`]: font tables, per-character metrics for a font's DC6 glyph sheet.
//! - [`tbl`]: `.tbl` string tables, the game's localised text.
//! - [`wav`]: RIFF/WAVE sound files decoded to 16-bit samples.
//! - [`d2s`]: `.d2s` character saves, byte layout, checksum and sections; the one format
//!   that is written as well as read.
//!
//! [`REQUIRED_MPQS`] lists the archives an install must contain, and [`FormatError`] is
//! the error type the single-file readers share; `mpq` and `d2s` have their own.

pub mod animdata;
pub mod cof;
mod cursor;
pub mod d2s;
pub mod dc6;
pub mod dcc;
pub mod ds1;
pub mod dt1;
pub mod font;
pub mod mpq;
pub mod palette;
pub mod tbl;
pub mod wav;

#[cfg(test)]
mod mutant_tests_dcc;
#[cfg(test)]
mod mutant_tests_formats;
#[cfg(test)]
mod robust;
#[cfg(test)]
mod robust_tests;

pub use cursor::FormatError;

/// The data archives every supported install must contain (see
/// `docs/REQUIRED_FILES.md`). Video archives are optional for engine work.
pub const REQUIRED_MPQS: &[&str] = &[
    "d2data.mpq",
    "d2exp.mpq",
    "d2char.mpq",
    "d2sfx.mpq",
    "d2speech.mpq",
    "d2xtalk.mpq",
    "d2music.mpq",
    "d2xmusic.mpq",
    "patch_d2.mpq",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn game_dir() -> PathBuf {
        PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set"))
    }

    /// Finds `name` in `dir`, ignoring case (installs vary: `d2data.mpq`, `D2Data.mpq`).
    fn find_ci(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
        std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
    }

    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn game_dir_contains_required_mpqs() {
        let dir = game_dir();
        let missing: Vec<_> = REQUIRED_MPQS
            .iter()
            .filter(|name| find_ci(&dir, name).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "missing in {}: {missing:?}",
            dir.display()
        );
    }
}
