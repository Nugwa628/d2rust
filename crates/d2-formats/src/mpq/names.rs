//! The file names a whole-install check enumerates.
//!
//! An archive's own `(listfile)` names most of its files, but
//! `patch_d2.mpq` has none, so a union of listfiles misses the patch's
//! files; [`EXTRA_NAMES`] adds the ones known to exist. Names are kept once
//! per archive lookup key ([`name_key`]), since listfiles of different
//! archives spell some names in different case. `mpq-tool formats` and the
//! `game_sweep` tests both take their names from [`known_names`], so their
//! counts share one scope.

use std::collections::BTreeMap;

use super::{Archive, MpqError};

/// Names known to exist but missing from every `(listfile)` (mostly in
/// `patch_d2.mpq`, which has none).
pub const EXTRA_NAMES: &[&str] = &[
    r"data\local\lng\eng\patchstring.tbl",
    r"data\local\lng\eng\string.tbl",
    r"data\local\lng\eng\expansionstring.tbl",
];

/// The archive's name key (`a`–`z` → `A`–`Z`, `/` → `\`, every other byte
/// unchanged, as [`super::crypto::hash`] normalizes). Two names with one
/// key hash alike, so they name the same file of an archive.
pub fn name_key(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' => '\\',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// The distinct file names of `lists`, keyed by [`name_key`]; the first
/// spelling met is kept, so one file spelled in different case (or with
/// `/` for `\`) by different listfiles counts once.
pub fn name_set(lists: impl IntoIterator<Item = Vec<String>>) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for name in lists.into_iter().flatten() {
        names.entry(name_key(&name)).or_insert(name);
    }
    names
}

/// [`EXTRA_NAMES`] plus every `(listfile)` of `archives`, as a [`name_set`].
pub fn known_names(archives: &[Archive]) -> Result<BTreeMap<String, String>, MpqError> {
    let mut lists = vec![EXTRA_NAMES.iter().map(|s| s.to_string()).collect()];
    for a in archives {
        if let Some(list) = a.listfile()? {
            lists.push(list);
        }
    }
    Ok(name_set(lists))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // Two listfiles spelling one file in different case (or with `/`) give
    // one name: the archive lookup normalizes case and separators.
    #[test]
    fn name_set_is_case_and_separator_insensitive() {
        let names = name_set([
            list(&[r"data\global\ui\panel\invchar6.DC6", r"data\global\a.dt1"]),
            list(&[r"DATA\GLOBAL\UI\PANEL\INVCHAR6.dc6", "data/global/a.dt1"]),
            list(&[r"data\global\b.dt1"]),
        ]);
        let kept: Vec<&str> = names.values().map(String::as_str).collect();
        assert_eq!(
            kept,
            [
                r"data\global\a.dt1",
                r"data\global\b.dt1",
                r"data\global\ui\panel\invchar6.DC6",
            ]
        );
        assert_eq!(
            name_key("data/global/ui/Panel.dc6"),
            r"DATA\GLOBAL\UI\PANEL.DC6"
        );
    }

    // An archive without a `(listfile)` (like `patch_d2.mpq`) adds nothing
    // of its own; the extra names are always present.
    #[test]
    fn extra_names_survive_without_listfiles() {
        let names = known_names(&[]).unwrap();
        assert_eq!(names.len(), EXTRA_NAMES.len());
        assert!(names.contains_key(&name_key(EXTRA_NAMES[0])));
    }
}
