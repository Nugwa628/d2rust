//! The game's archives, searched in priority order.
//!
//! A Diablo II install holds up to eleven `.mpq` archives. [`ArchiveSet`]
//! opens the ones present in the install directory and answers "which
//! archive has this file" by checking them from highest to lowest priority,
//! so `patch_d2.mpq` overrides the expansion archives, which override the
//! base archives such as `d2data.mpq`. Asset loaders in D2RUST read game
//! files through this set.
//!
//! # Priority rules
//!
//! Every archive is opened with a priority (base 1000, expansion 3000,
//! patch 5000), in the order of [`OPEN_ORDER`]. Lookups search higher
//! priorities first; among equal priorities the archive opened later is
//! searched first. [`search_order`] applies those rules, and [`PRIORITY`] is
//! their result for [`OPEN_ORDER`].

use std::path::Path;

use super::{Archive, MpqError};

/// One archive the game opens: its file name and the priority it is opened
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveSpec {
    /// Archive file name, lowercase.
    pub name: &'static str,
    /// Opening priority; higher is searched first.
    pub priority: u32,
}

/// The archives D2RUST opens, in the order the game opens them: the startup
/// group, then the second group, then the video path. `d2delta.mpq` and
/// `d2kfixup.mpq` (optional, startup group) are not opened. How the second
/// group and the video path order relative to each other is not pinned down;
/// this order follows the table order. Excel lookups do not depend on it.
pub const OPEN_ORDER: [ArchiveSpec; 11] = [
    ArchiveSpec {
        name: "d2data.mpq",
        priority: 1000,
    },
    ArchiveSpec {
        name: "d2sfx.mpq",
        priority: 1000,
    },
    ArchiveSpec {
        name: "d2speech.mpq",
        priority: 1000,
    },
    ArchiveSpec {
        name: "patch_d2.mpq",
        priority: 5000,
    },
    ArchiveSpec {
        name: "d2exp.mpq",
        priority: 3000,
    },
    ArchiveSpec {
        name: "d2char.mpq",
        priority: 1000,
    },
    ArchiveSpec {
        name: "d2music.mpq",
        priority: 1000,
    },
    ArchiveSpec {
        name: "d2xmusic.mpq",
        priority: 3000,
    },
    ArchiveSpec {
        name: "d2xtalk.mpq",
        priority: 3000,
    },
    ArchiveSpec {
        name: "d2xvideo.mpq",
        priority: 3000,
    },
    ArchiveSpec {
        name: "d2video.mpq",
        priority: 1000,
    },
];

/// Archive names in lookup order (first match wins): higher priority
/// first; among equal priorities, the archive opened later first.
/// Equal to [`search_order`] applied to [`OPEN_ORDER`] (checked by a test).
pub const PRIORITY: [&str; 11] = [
    "patch_d2.mpq",
    "d2xvideo.mpq",
    "d2xtalk.mpq",
    "d2xmusic.mpq",
    "d2exp.mpq",
    "d2video.mpq",
    "d2music.mpq",
    "d2char.mpq",
    "d2speech.mpq",
    "d2sfx.mpq",
    "d2data.mpq",
];

/// Sorts archives (given in open order) into search order: priority
/// descending, then open order descending (each archive is inserted before
/// the first list entry whose priority is ≤ its own).
pub fn search_order(open_order: &[ArchiveSpec]) -> Vec<ArchiveSpec> {
    let mut list: Vec<ArchiveSpec> = Vec::with_capacity(open_order.len());
    for &a in open_order {
        let at = list
            .iter()
            .position(|b| b.priority <= a.priority)
            .unwrap_or(list.len());
        list.insert(at, a);
    }
    list
}

/// Lookup rank of an archive file name (case-insensitive); unknown names
/// sort last.
pub fn priority(file_name: &str) -> usize {
    PRIORITY
        .iter()
        .position(|p| p.eq_ignore_ascii_case(file_name))
        .unwrap_or(PRIORITY.len())
}

/// The open archives of one install, in search order.
#[derive(Debug)]
pub struct ArchiveSet {
    archives: Vec<Archive>,
}

impl ArchiveSet {
    /// Opens every known archive present in `dir` (names matched
    /// case-insensitively). Archives that are absent are skipped.
    pub fn open_dir(dir: impl AsRef<Path>) -> Result<ArchiveSet, MpqError> {
        let mut found: Vec<(usize, std::path::PathBuf)> = std::fs::read_dir(dir.as_ref())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?.to_owned();
                let rank = priority(&name);
                (rank < PRIORITY.len()).then_some((rank, p))
            })
            .collect();
        found.sort();
        let archives = found
            .into_iter()
            .map(|(_, p)| Archive::open(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ArchiveSet { archives })
    }

    /// The open archives, highest priority first.
    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    /// Whether the archive file `file_name` (e.g. `d2exp.mpq`) is open,
    /// compared case-insensitively.
    pub fn has_archive(&self, file_name: &str) -> bool {
        self.archives
            .iter()
            .any(|a| archive_file_name(a).eq_ignore_ascii_case(file_name))
    }

    /// The highest-priority archive containing `name`.
    pub fn find(&self, name: &str) -> Option<&Archive> {
        self.archives.iter().find(|a| a.contains(name))
    }

    /// True if any open archive has `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// Reads `name` from the highest-priority archive that has it.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, MpqError> {
        self.find(name)
            .ok_or_else(|| MpqError::NotFound(name.to_owned()))?
            .read(name)
    }

    /// Reads `name` from the highest-priority archive that has it, with
    /// that archive's file name in lowercase (e.g. `patch_d2.mpq`).
    /// `Ok(None)` when no archive has it.
    pub fn read_with_source(&self, name: &str) -> Result<Option<(String, Vec<u8>)>, MpqError> {
        match self.find(name) {
            None => Ok(None),
            Some(a) => Ok(Some((
                archive_file_name(a).to_ascii_lowercase(),
                a.read(name)?,
            ))),
        }
    }
}

/// The file name of an open archive (`""` if the path has none).
pub fn archive_file_name(a: &Archive) -> &str {
    a.path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_order() {
        assert_eq!(priority("Patch_D2.mpq"), 0);
        assert!(priority("d2exp.mpq") < priority("d2data.mpq"));
        assert!(priority("D2XVIDEO.MPQ") < priority("d2data.mpq"));
        assert_eq!(priority("other.mpq"), PRIORITY.len());
    }

    /// Excel files resolve patch, then expansion, then base data.
    #[test]
    fn excel_order_is_patch_exp_data() {
        assert!(priority("patch_d2.mpq") < priority("d2exp.mpq"));
        assert!(priority("d2exp.mpq") < priority("d2data.mpq"));
    }

    /// Priority descending, ties newest first.
    #[test]
    fn priority_table_matches_open_order() {
        let order: Vec<&str> = search_order(&OPEN_ORDER).iter().map(|a| a.name).collect();
        assert_eq!(order, PRIORITY);
        // d2data was opened first of the 1000 group, so it is searched last.
        assert_eq!(PRIORITY[PRIORITY.len() - 1], "d2data.mpq");
    }

    #[test]
    fn ties_put_later_archives_first() {
        let a = ArchiveSpec {
            name: "a",
            priority: 1,
        };
        let b = ArchiveSpec {
            name: "b",
            priority: 1,
        };
        let c = ArchiveSpec {
            name: "c",
            priority: 2,
        };
        let order: Vec<&str> = search_order(&[a, b, c]).iter().map(|x| x.name).collect();
        assert_eq!(order, ["c", "b", "a"]);
    }
}
