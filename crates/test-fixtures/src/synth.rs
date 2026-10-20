//! The synthetic `.txt` table set. Headers come from the schema: each
//! `.txt` gets every column of every field list compiled from it, in
//! list order. Rows are written here from scratch; cells not given are
//! empty. A file with no row gets one all-empty row (the table loader
//! rejects a table with zero records).

use std::collections::BTreeMap;

use d2_data::schema::schema;

/// One `.txt` file: header columns and rows of cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtFile {
    /// File name as the schema spells it (`armor.txt`).
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl TxtFile {
    /// The file bytes: tab-separated, CR LF after every line.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = self.columns.join("\t").into_bytes();
        out.extend_from_slice(b"\r\n");
        let empty = vec![String::new(); self.columns.len()];
        let rows: Vec<&Vec<String>> = if self.rows.is_empty() {
            vec![&empty]
        } else {
            self.rows.iter().collect()
        };
        for r in rows {
            out.extend_from_slice(r.join("\t").as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out
    }

    fn column(&self, name: &str) -> usize {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("{}: no column {name:?} in the schema", self.name))
    }
}

/// Every `.txt` the load compiles, by lowercase file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSet {
    pub files: BTreeMap<String, TxtFile>,
}

impl TableSet {
    /// Headers for every called table's `.txt`, no rows.
    pub fn headers() -> TableSet {
        let mut files: BTreeMap<String, TxtFile> = BTreeMap::new();
        for def in schema().called() {
            let f = files
                .entry(def.txt_name.to_ascii_lowercase())
                .or_insert_with(|| TxtFile {
                    name: def.txt_name.clone(),
                    columns: Vec::new(),
                    rows: Vec::new(),
                });
            for field in &def.fields {
                let name = field.name();
                if !f.columns.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                    f.columns.push(name);
                }
            }
        }
        TableSet { files }
    }

    pub fn file(&self, txt: &str) -> &TxtFile {
        &self.files[&txt.to_ascii_lowercase()]
    }

    /// Appends a row to `txt` (`armor.txt` or `armor`); `cells` are
    /// (column, value), the rest empty. Unknown columns panic.
    pub fn row(&mut self, txt: &str, cells: &[(&str, &str)]) -> &mut Self {
        let key = txt_key(txt);
        let f = self
            .files
            .get_mut(&key)
            .unwrap_or_else(|| panic!("{key}: not a compiled .txt"));
        let mut row = vec![String::new(); f.columns.len()];
        for (column, value) in cells {
            assert!(
                !value.contains(['\t', '\r', '\n']),
                "{key} {column}: cell {value:?} has a separator"
            );
            row[f.column(column)] = (*value).to_owned();
        }
        f.rows.push(row);
        self
    }

    /// Sets `column` of row `i` of `txt`.
    pub fn set(&mut self, txt: &str, i: usize, column: &str, value: &str) -> &mut Self {
        let key = txt_key(txt);
        let f = self.files.get_mut(&key).expect("compiled .txt");
        let c = f.column(column);
        f.rows[i][c] = value.to_owned();
        self
    }

    /// The bytes of each file, by schema file name.
    pub fn render(&self) -> Vec<(String, Vec<u8>)> {
        self.files
            .values()
            .map(|f| (f.name.clone(), f.bytes()))
            .collect()
    }
}

fn txt_key(txt: &str) -> String {
    let t = txt.to_ascii_lowercase();
    if t.ends_with(".txt") {
        t
    } else {
        format!("{t}.txt")
    }
}

/// A string table: (key, value) in element order.
pub type Strings = Vec<(String, String)>;

/// The three string tables: base, expansion and patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringSet {
    pub base: Strings,
    pub patch: Strings,
    pub expansion: Strings,
}

/// The whole synthetic data set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthetic {
    pub tables: TableSet,
    pub strings: StringSet,
    /// `sounds.txt` is also a compiled table (in `tables`);
    /// `soundenviron.txt` is only parsed at run time, never compiled.
    pub soundenviron: Vec<u8>,
    pub animdata: Vec<crate::animdata::Anim>,
    /// Other files of `d2data.mpq`, (archive name, bytes): the DRLG's
    /// DS1 / DT1 files ([`crate::drlg::files`]), compressed.
    pub files: Vec<(String, Vec<u8>)>,
    /// Files of `d2data.mpq` written stored (uncompressed): the large
    /// generated art and palettes of [`crate::play`], which would take
    /// most of a debug-build install to compress.
    pub stored: Vec<(String, Vec<u8>)>,
}

/// The synthetic set.
pub fn synthetic() -> Synthetic {
    let mut tables = TableSet::headers();
    let (strings, animdata) = crate::content::fill(&mut tables);
    Synthetic {
        tables,
        strings,
        soundenviron: b"Handle\tIndex\r\nnone\t0\r\n".to_vec(),
        animdata,
        files: crate::drlg::files(),
        stored: Vec::new(),
    }
}
