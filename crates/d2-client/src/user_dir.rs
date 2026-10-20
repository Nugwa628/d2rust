//! The player's D2RUST folder, `Documents\D2RUST`: characters (`saves\`),
//! `settings.toml`, `controls.toml` and the logs (`logs\`). The game
//! never writes into the Diablo II install.
//!
//! Earlier test builds used `Documents\d2rs` for the same things and kept
//! `controls.toml` in `%APPDATA%\d2rs`; the first hand-out build already
//! used `Documents\D2RUST` but kept `controls.toml` in `%APPDATA%\D2RUST`.
//! [`migrate`] moves `Documents\d2rs` to `Documents\D2RUST` when no D2RUST
//! folder exists yet (one rename, so testers keep their characters), and
//! moves an old `controls.toml` in whenever the D2RUST folder has none. A
//! move that fails leaves the old folder in use ([`resolve`]), and an
//! existing file is never overwritten, so nothing is ever lost or hidden.
//!
//! `D2RUST_HOME` names another folder to use instead (tests, portable
//! installs); no migration happens then.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The folder's name under Documents.
pub const FOLDER: &str = "D2RUST";

/// The name the test builds used, under Documents and under `%APPDATA%`.
pub const LEGACY: &str = "d2rs";

/// The variable that overrides the folder (the older `D2RS_HOME` is read
/// too, [`crate::launch::env_os`]).
pub const HOME_VAR: &str = "D2RUST_HOME";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// What [`migrate`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Migration {
    /// Nothing to move (a D2RUST folder exists, or there was no old one).
    None,
    /// `Documents\d2rs` became `Documents\D2RUST`; `controls` says whether
    /// the old `%APPDATA%` `controls.toml` came along too.
    Moved { controls: bool },
    /// Only an old `controls.toml` was moved in.
    Controls,
    /// The move failed; the old folder stays in use.
    Failed(String),
}

/// The user's Documents folder: `%USERPROFILE%\Documents` on Windows,
/// `$HOME/Documents` elsewhere.
pub fn documents() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join("Documents"))
}

/// The folders earlier builds kept `controls.toml` in, newest first:
/// `%APPDATA%\D2RUST` (the first hand-out build), then `%APPDATA%\d2rs`
/// (on Windows; the platform config folder elsewhere).
pub fn legacy_config_dirs() -> Vec<PathBuf> {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| vec![b.join(FOLDER), b.join(LEGACY)])
        .unwrap_or_default()
}

/// Moves the old files into `documents\D2RUST`: `documents\d2rs` is
/// renamed when the D2RUST folder does not exist yet, then the first
/// `controls.toml` found in `legacy_config` is moved in unless the folder
/// already has one.
pub fn migrate(documents: &Path, legacy_config: &[PathBuf]) -> Migration {
    let new = documents.join(FOLDER);
    let old = documents.join(LEGACY);
    let moved = if !new.is_dir() && old.is_dir() {
        if let Err(e) = std::fs::rename(&old, &new) {
            return Migration::Failed(format!(
                "moving {} to {}: {e}",
                old.display(),
                new.display()
            ));
        }
        true
    } else {
        false
    };
    let controls = legacy_config
        .iter()
        .map(|d| d.join("controls.toml"))
        .find(|c| c.is_file())
        .is_some_and(|c| {
            let to = new.join("controls.toml");
            !to.exists() && std::fs::create_dir_all(&new).is_ok() && move_file(&c, &to)
        });
    match (moved, controls) {
        (true, c) => Migration::Moved { controls: c },
        (false, true) => Migration::Controls,
        (false, false) => Migration::None,
    }
}

/// The folder to use after [`migrate`]: `documents\D2RUST`, unless it does
/// not exist while `documents\d2rs` does (a move that failed).
pub fn resolve(documents: &Path) -> PathBuf {
    let new = documents.join(FOLDER);
    let old = documents.join(LEGACY);
    if !new.is_dir() && old.is_dir() {
        old
    } else {
        new
    }
}

/// Runs the one-time move and fixes the folder for this run. Call once at
/// start, before anything reads or writes a user file. Returns the folder
/// and what the move did.
pub fn init() -> (PathBuf, Migration) {
    if let Some(home) = crate::launch::env_os(HOME_VAR).filter(|h| !h.is_empty()) {
        let root = PathBuf::from(home);
        let _ = ROOT.set(root.clone());
        return (root, Migration::None);
    }
    let (root, m) = match documents() {
        Some(docs) => {
            let m = migrate(&docs, &legacy_config_dirs());
            (resolve(&docs), m)
        }
        None => (PathBuf::from(FOLDER), Migration::None),
    };
    let _ = ROOT.set(root.clone());
    (root, m)
}

/// The D2RUST folder ([`init`]'s answer; resolved without moving anything
/// when `init` has not run, as in tests and tools).
pub fn root() -> PathBuf {
    ROOT.get().cloned().unwrap_or_else(|| {
        crate::launch::env_os(HOME_VAR)
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(|| documents().map(|d| resolve(&d)))
            .unwrap_or_else(|| PathBuf::from(FOLDER))
    })
}

/// Where characters are saved.
pub fn saves() -> PathBuf {
    root().join("saves")
}

/// Where the logs go.
pub fn logs() -> PathBuf {
    root().join("logs")
}

/// `controls.toml`, next to `settings.toml`.
pub fn controls_file() -> PathBuf {
    root().join("controls.toml")
}

/// Moves a file: a rename, else copy and delete (another drive).
fn move_file(from: &Path, to: &Path) -> bool {
    std::fs::rename(from, to).is_ok()
        || (std::fs::copy(from, to).is_ok() && std::fs::remove_file(from).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("d2rust-user-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_old_folder_moves_once_with_its_saves() {
        let docs = scratch("move");
        std::fs::create_dir_all(docs.join("d2rs/saves")).unwrap();
        std::fs::write(docs.join("d2rs/saves/Amy.d2s"), b"x").unwrap();
        std::fs::write(docs.join("d2rs/settings.toml"), b"version = 1\n").unwrap();
        assert_eq!(migrate(&docs, &[]), Migration::Moved { controls: false });
        assert!(docs.join("D2RUST/saves/Amy.d2s").is_file());
        assert!(docs.join("D2RUST/settings.toml").is_file());
        assert!(!docs.join("d2rs").exists());
        assert_eq!(resolve(&docs), docs.join("D2RUST"));
        // Second start: nothing left to do.
        assert_eq!(migrate(&docs, &[]), Migration::None);
        let _ = std::fs::remove_dir_all(docs);
    }

    #[test]
    fn an_existing_d2rust_folder_is_never_overwritten() {
        let docs = scratch("keep");
        std::fs::create_dir_all(docs.join("D2RUST/saves")).unwrap();
        std::fs::create_dir_all(docs.join("d2rs/saves")).unwrap();
        std::fs::write(docs.join("d2rs/saves/Old.d2s"), b"x").unwrap();
        assert_eq!(migrate(&docs, &[]), Migration::None);
        assert!(docs.join("d2rs/saves/Old.d2s").is_file());
        assert!(!docs.join("D2RUST/saves/Old.d2s").exists());
        assert_eq!(resolve(&docs), docs.join("D2RUST"));
        let _ = std::fs::remove_dir_all(docs);
    }

    #[test]
    fn the_old_controls_file_comes_along() {
        let docs = scratch("ctl-docs");
        let cfg = scratch("ctl-cfg");
        std::fs::create_dir_all(docs.join("d2rs")).unwrap();
        std::fs::write(cfg.join("controls.toml"), b"version = 1\n").unwrap();
        assert_eq!(
            migrate(&docs, std::slice::from_ref(&cfg)),
            Migration::Moved { controls: true }
        );
        assert!(docs.join("D2RUST/controls.toml").is_file());
        assert!(!cfg.join("controls.toml").exists());
        // Without an old Documents folder, the controls file alone moves.
        let docs2 = scratch("ctl-docs2");
        std::fs::write(cfg.join("controls.toml"), b"version = 1\n").unwrap();
        assert_eq!(
            migrate(&docs2, std::slice::from_ref(&cfg)),
            Migration::Controls
        );
        assert!(docs2.join("D2RUST/controls.toml").is_file());
        for d in [docs, cfg, docs2] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn the_hand_out_builds_controls_join_its_existing_folder() {
        // Documents\D2RUST exists (saves) and controls.toml sits in the
        // newer %APPDATA%\D2RUST; the older %APPDATA%\d2rs one stays.
        let docs = scratch("ho-docs");
        let newer = scratch("ho-newer");
        let older = scratch("ho-older");
        std::fs::create_dir_all(docs.join("D2RUST/saves")).unwrap();
        std::fs::write(newer.join("controls.toml"), b"version = 1\n# new\n").unwrap();
        std::fs::write(older.join("controls.toml"), b"version = 1\n# old\n").unwrap();
        let dirs = [newer.clone(), older.clone()];
        assert_eq!(migrate(&docs, &dirs), Migration::Controls);
        assert_eq!(
            std::fs::read(docs.join("D2RUST/controls.toml")).unwrap(),
            b"version = 1\n# new\n"
        );
        assert!(!newer.join("controls.toml").exists());
        // The folder has one now: the older file is never moved over it.
        assert_eq!(migrate(&docs, &dirs), Migration::None);
        assert!(older.join("controls.toml").is_file());
        for d in [docs, newer, older] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn a_fresh_install_uses_the_d2rust_folder() {
        let docs = scratch("fresh");
        assert_eq!(migrate(&docs, &[]), Migration::None);
        assert_eq!(resolve(&docs), docs.join("D2RUST"));
        // A failed move (simulated: only the old folder exists) keeps it.
        std::fs::create_dir_all(docs.join("d2rs")).unwrap();
        assert_eq!(resolve(&docs), docs.join("d2rs"));
        let _ = std::fs::remove_dir_all(docs);
    }
}
