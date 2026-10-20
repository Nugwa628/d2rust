//! Side tables a mod game ships (`docs/plans/mods/seams.md` MS10c).
//!
//! A mod game patches D2RUST's tables (`d2patch`) and may also ship tables
//! of its own: plain tab-separated `.txt` files under its layer's
//! `files/tables/`. The game never reads them; they load into a
//! [`SideTables`] store the mod game's own code reads by table name, row
//! and column. A side table is never named like one of the game's tables
//! (that would be a copy of it, which mods may only patch).

use std::collections::BTreeMap;

use crate::schema::schema;
use crate::txt::{names_equal, TxtError, TxtTable};

/// Why a side table did not load.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SideError {
    /// The file is not a `.txt` file.
    #[error("{0}: a side table is a .txt file")]
    NotTxt(String),
    /// The file is named like one of the game's tables.
    #[error("{0}: named like the game table `{1}`; patch it with d2patch instead")]
    GameTable(String, String),
    /// The text did not parse.
    #[error(transparent)]
    Txt(#[from] TxtError),
}

/// The game's own table names (lowercase file stems, without `.txt`).
fn is_game_table(stem: &str) -> Option<String> {
    let stem_of = |f: &str| {
        f.rsplit_once('.')
            .map_or(f, |(s, _)| s)
            .to_ascii_lowercase()
    };
    for t in &schema().tables {
        for n in [t.name.as_str(), t.txt_name.as_str(), t.bin_name.as_str()] {
            if stem_of(n) == stem {
                return Some(t.txt_name.clone());
            }
        }
    }
    ["sounds", "soundenviron"]
        .contains(&stem)
        .then(|| format!("{stem}.txt"))
}

/// A mod game's side tables, by name (the lowercase file stem).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideTables {
    tables: BTreeMap<String, TxtTable>,
}

impl SideTables {
    /// Loads `(path, bytes)` files in layer order; a later file with the
    /// same name replaces an earlier one (a later layer overrides).
    pub fn from_files<I, P>(files: I) -> Result<SideTables, SideError>
    where
        I: IntoIterator<Item = (P, Vec<u8>)>,
        P: AsRef<str>,
    {
        let mut out = SideTables::default();
        for (path, bytes) in files {
            let path = path.as_ref();
            let file = path.rsplit('/').next().unwrap_or(path);
            let Some(stem) = file
                .len()
                .checked_sub(4)
                .filter(|&n| n > 0 && file[n..].eq_ignore_ascii_case(".txt"))
                .map(|n| file[..n].to_ascii_lowercase())
            else {
                return Err(SideError::NotTxt(path.to_owned()));
            };
            if let Some(game) = is_game_table(&stem) {
                return Err(SideError::GameTable(path.to_owned(), game));
            }
            out.tables.insert(stem, TxtTable::parse(path, &bytes)?);
        }
        Ok(out)
    }

    /// True when the mod ships no side table.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// The side table names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }

    /// Side table `name` (case-insensitive).
    pub fn table(&self, name: &str) -> Option<&TxtTable> {
        self.tables.get(&name.to_ascii_lowercase())
    }

    /// The row count of `name` (0 when there is no such table).
    pub fn rows(&self, name: &str) -> usize {
        self.table(name).map_or(0, |t| t.records.len())
    }

    /// The cell of row `row`, column `column` (header names compare without
    /// case); a row shorter than the header reads empty cells.
    pub fn cell(&self, name: &str, row: usize, column: &str) -> Option<&[u8]> {
        let t = self.table(name)?;
        let c = t
            .header
            .iter()
            .position(|h| names_equal(h, column.as_bytes()))?;
        let r = t.records.get(row)?;
        Some(r.cells.get(c).map_or(&[][..], Vec::as_slice))
    }

    /// The cell as an integer; an empty cell reads 0, text that is not a
    /// whole number reads `None`.
    pub fn int(&self, name: &str, row: usize, column: &str) -> Option<i64> {
        let cell = std::str::from_utf8(self.cell(name, row, column)?)
            .ok()?
            .trim();
        if cell.is_empty() {
            Some(0)
        } else {
            cell.parse().ok()
        }
    }

    /// The first row whose `column` cell equals `key` (byte for byte).
    pub fn find(&self, name: &str, column: &str, key: &[u8]) -> Option<usize> {
        (0..self.rows(name)).find(|&r| self.cell(name, r, column) == Some(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(files: &[(&str, &str)]) -> Result<SideTables, SideError> {
        SideTables::from_files(files.iter().map(|(p, t)| (*p, t.as_bytes().to_vec())))
    }

    #[test]
    fn reads_cells_by_name_row_and_column() {
        let s = load(&[(
            "tables/Bank.txt",
            "Name\tSlots\tNote\r\nsmall\t4\t\r\nbig\t12\tx\r\n",
        )])
        .unwrap();
        assert_eq!(s.names().collect::<Vec<_>>(), ["bank"]);
        assert_eq!(s.rows("BANK"), 2);
        assert_eq!(s.cell("bank", 1, "note"), Some(&b"x"[..]));
        assert_eq!(s.cell("bank", 0, "Note"), Some(&b""[..]));
        assert_eq!(s.int("bank", 1, "slots"), Some(12));
        assert_eq!(s.int("bank", 0, "note"), Some(0));
        assert_eq!(s.int("bank", 0, "name"), None);
        assert_eq!(s.find("bank", "Name", b"big"), Some(1));
        assert_eq!(s.find("bank", "Name", b"none"), None);
        assert_eq!(s.cell("bank", 2, "Name"), None);
        assert_eq!(s.cell("other", 0, "Name"), None);
        assert!(!s.is_empty());
    }

    #[test]
    fn a_later_layer_replaces_a_table() {
        let s = load(&[("a/x.txt", "K\r\n1\r\n"), ("b/X.TXT", "K\r\n2\r\n3\r\n")]).unwrap();
        assert_eq!(s.rows("x"), 2);
        assert_eq!(s.int("x", 0, "k"), Some(2));
    }

    #[test]
    fn game_table_names_and_other_files_are_rejected() {
        for name in [
            "tables/Levels.txt",
            "tables/itemstatcost.txt",
            "tables/sounds.txt",
        ] {
            assert!(
                matches!(load(&[(name, "A\r\n")]), Err(SideError::GameTable(..))),
                "{name}"
            );
        }
        assert!(matches!(
            load(&[("tables/x.tsv", "A\r\n")]),
            Err(SideError::NotTxt(_))
        ));
        assert!(matches!(
            load(&[("tables/.txt", "A\r\n")]),
            Err(SideError::NotTxt(_))
        ));
        assert!(load(&[]).unwrap().is_empty());
    }
}
