//! The game tables of a native root, served to the `d2-data` table loader.
//!
//! A native root holds the excel tables as their `.txt` text, never as
//! compiled `.bin` files. [`NativeTables`] builds the `.bin` the loader
//! expects, in four steps:
//!
//! 1. the string tables load first, because compiling resolves string keys;
//! 2. the mods' table patches apply to the base `.txt` set, in layer order,
//!    and their string edits over the string tables;
//! 3. the patched set compiles to records;
//! 4. `_bin-overrides.toml` restores the few explained cells where the
//!    shipped `.bin` differs from a compile of its own text.
//!
//! The loader then runs every one of its checks on these bytes exactly as
//! it would on an archive's `.bin`.

use std::collections::BTreeMap;

use d2_data::bin::{LoadError, TableFiles};
use d2_data::compile_set::compile_all;
use d2_data::patch::{self, PatchData, StrEdit};
use d2_data::schema::schema;
use d2_data::strings::{StringEdit, StringTables, StringsError};
use d2_formats::tbl::StringTable;

use super::{fold, NativeAsset, NativeSource};
use crate::layers::ORDER_FILE;
use crate::tables::{BinOverrides, OVERRIDES_PATH};

/// The string tables the compile reads, in load order.
const STRING_TABLES: [&str; 3] = ["string.tbl", "patchstring.tbl", "expansionstring.tbl"];

/// Base `.txt` files served as text rather than compiled: the runtime sound
/// tables.
const TEXT_TABLES: [&str; 2] = ["sounds.txt", "soundenviron.txt"];

/// The table source over a native root: implements
/// `d2_data::bin::TableFiles`, so `bin::load_from` runs its checks on the
/// compiled bytes exactly as on the archive's `.bin`.
#[derive(Debug, Clone)]
pub struct NativeTables {
    lod: bool,
    strings: BTreeMap<String, StringTable>,
    /// Excel `.txt` bytes (after patches), by lowercase file name.
    txt: BTreeMap<String, Vec<u8>>,
    /// Compiled `.bin` bytes (count + records), by lowercase file name.
    bins: BTreeMap<String, Vec<u8>>,
    /// The mods' string edits (`stradd` / `strset`), in stack order;
    /// every load of the string tables applies them.
    string_edits: Vec<StringEdit>,
}

/// A `.txt` reader as the compile and the patcher take it: the file's
/// origin label and bytes, `None` when absent.
type TxtResult = Result<Option<(String, Vec<u8>)>, String>;

/// The patched `.txt` texts by lowercase file name, and the patches'
/// string edits in stack order.
type Patched = (BTreeMap<String, Vec<u8>>, Vec<StrEdit>);

fn source_err(file: &str, detail: impl ToString) -> LoadError {
    LoadError::Source {
        file: file.to_owned(),
        detail: detail.to_string(),
    }
}

/// Canonical path of an excel file name (`Armor.bin` → `data/global/excel/armor.bin`).
fn excel_canon(file: &str) -> String {
    format!("data/global/excel/{}", file.to_ascii_lowercase())
}

impl NativeTables {
    pub(super) fn build(src: &NativeSource, language: &str) -> Result<NativeTables, LoadError> {
        let mut t = NativeTables {
            lod: src.manifest.lod,
            strings: BTreeMap::new(),
            txt: BTreeMap::new(),
            bins: BTreeMap::new(),
            string_edits: Vec::new(),
        };
        for name in STRING_TABLES {
            let p = format!("data/local/lng/{language}/{name}");
            match src.read_native(&p) {
                Some(Ok(NativeAsset::Tbl(table))) => {
                    t.strings.insert(p, table);
                }
                Some(Ok(_)) => unreachable!("tbl path reads as tbl"),
                Some(Err(e)) => return Err(source_err(&p, e)),
                None => {}
            }
        }
        let mut strings = StringTables::load_from(&t, language, t.lod)?;

        // Excel text always comes from the base layer: mods change tables
        // with patches, never with copies.
        let base = &src.layers.layers[0];
        let mut read_base = |file: &str| -> TxtResult {
            match src.layers.read_in(base, &excel_canon(file)) {
                None => Ok(None),
                Some(Ok(b)) => Ok(Some(("native".to_owned(), b))),
                Some(Err(e)) => Err(e.to_string()),
            }
        };
        let (patched, edits) = patched_tables(src, &mut read_base)?;
        // The mods' strings first: compiling binds `strkey` cells to them.
        let findings = patch::apply_strings(&mut strings, &edits);
        if patch::has_errors(&findings) {
            return Err(source_err("string patches", findings_text(&findings)));
        }
        t.string_edits = edits.into_iter().map(|e| e.edit).collect();
        let mut read_txt = |file: &str| -> TxtResult {
            match patched.get(&file.to_ascii_lowercase()) {
                Some(b) => Ok(Some(("native".to_owned(), b.clone()))),
                None => read_base(file),
            }
        };

        let compiled =
            compile_all(&mut read_txt, &strings).map_err(|e| source_err("compile", e))?;
        let overrides = read_overrides(src)?;
        for def in schema().called() {
            let Some(c) = compiled.table(&def.name) else {
                continue;
            };
            let mut records = c.compiled.records.clone();
            overrides
                .apply(&def.name, c.compiled.record_size, &mut records)
                .map_err(|e| source_err(OVERRIDES_PATH, e))?;
            // A `.bin` is the record count, then the records.
            let mut bin = (c.compiled.count as u32).to_le_bytes().to_vec();
            bin.extend_from_slice(&records);
            t.bins.insert(def.bin_name.to_ascii_lowercase(), bin);
        }
        for (buffer, bytes) in &compiled.buffers {
            t.bins.insert(
                format!("{}.bin", buffer.name()).to_ascii_lowercase(),
                bytes.clone(),
            );
        }
        for name in TEXT_TABLES {
            if let Some((_, b)) = read_txt(name).map_err(|e| source_err(name, e))? {
                t.txt.insert(name.to_owned(), b);
            }
        }
        Ok(t)
    }
}

/// The `.txt` text of every table after the mods' patches, by lowercase
/// file name, and the patches' string edits; both empty when no mod ships
/// a patch. Patches apply in stack
/// order (layer order, then file name), and any finding that is an error
/// refuses the whole root.
fn patched_tables(
    src: &NativeSource,
    read_base: &mut dyn FnMut(&str) -> TxtResult,
) -> Result<Patched, LoadError> {
    let patches = src
        .layers
        .patches()
        .map_err(|e| source_err(ORDER_FILE, e))?;
    if patches.is_empty() {
        return Ok((BTreeMap::new(), Vec::new()));
    }
    let rules = patch::rules();
    let mut data = PatchData::from_base(&rules, read_base)
        .map_err(|f| source_err("table patches", findings_text(&f)))?;
    let mut layers = Vec::new();
    for (pos, (path, bytes)) in patches.iter().enumerate() {
        let (layer, findings) = patch::parse_layer(path, bytes, pos);
        if patch::has_errors(&findings) {
            return Err(source_err(path, findings_text(&findings)));
        }
        layers.push(layer);
    }
    let findings = patch::apply_stack(&mut data, &layers, ORDER_FILE);
    if patch::has_errors(&findings) {
        return Err(source_err("table patches", findings_text(&findings)));
    }
    let texts = data
        .tables
        .iter()
        .zip(&rules)
        .map(|(table, rule)| (rule.txt_name.to_ascii_lowercase(), table.render()))
        .collect();
    Ok((texts, data.strings))
}

/// The base layer's `_bin-overrides.toml`, or no overrides when it has none.
fn read_overrides(src: &NativeSource) -> Result<BinOverrides, LoadError> {
    let Some(read) = src.layers.read_in(&src.layers.layers[0], OVERRIDES_PATH) else {
        return Ok(BinOverrides::default());
    };
    let bytes = read.map_err(|e| source_err(OVERRIDES_PATH, e))?;
    let text = String::from_utf8(bytes).map_err(|_| source_err(OVERRIDES_PATH, "not UTF-8"))?;
    BinOverrides::from_toml(OVERRIDES_PATH, &text).map_err(|e| source_err(OVERRIDES_PATH, e))
}

/// The error findings of a patch run, joined for one message.
fn findings_text(f: &[patch::Finding]) -> String {
    f.iter()
        .filter(|f| f.code.is_error())
        .map(|f| format!("{f:?}"))
        .collect::<Vec<_>>()
        .join("; ")
}

impl TableFiles for NativeTables {
    fn read_excel(&self, file: &str) -> Result<Option<(String, Vec<u8>)>, LoadError> {
        let key = file.to_ascii_lowercase();
        let hit = self.bins.get(&key).or_else(|| self.txt.get(&key));
        Ok(hit.map(|b| ("native".to_owned(), b.clone())))
    }

    fn lod(&self) -> bool {
        self.lod
    }

    fn string_table(&self, path: &str) -> Result<StringTable, StringsError> {
        self.strings
            .get(&fold(path))
            .cloned()
            .ok_or_else(|| StringsError::Source {
                file: path.to_owned(),
                detail: "no native string table".into(),
            })
    }

    fn string_edits(&self) -> Vec<StringEdit> {
        self.string_edits.clone()
    }
}
