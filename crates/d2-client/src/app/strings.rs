//! The play app's string tables: `string.tbl`, `patchstring.tbl` and
//! `expansionstring.tbl` loaded through `d2_data::strings`, held as UTF-16
//! by id and by key, and bound as the world view's [`StringLookup`]
//! (replacing [`crate::ui::NoStrings`]). Panels that take a closure
//! (`Fn(u16) -> Vec<u16>`) use [`TableStrings::by_id`]. A mod game's
//! string edits (`d2_data::strings::StringLayer`) read before the tables.
//!
//! Every element of the three tables is decoded once at load, so a lookup
//! is an index into a vector.

use std::collections::HashMap;

use bevy::prelude::*;
use d2_data::bin::TableFiles;
use d2_data::strings::{StringLayer, StringTables, StringsError, EXPANSION_BASE, PATCH_BASE};
use d2_formats::tbl::StringTable;

use crate::ui::StringLookup;
use crate::world_view::WorldViewUi;

/// The language of the play preview (`UiParts::live` loads `eng` too).
pub const LANG: &str = "eng";

/// All string-table text, UTF-16 as the fonts take it.
#[derive(Debug, Clone, Default)]
pub struct TableStrings {
    /// Every element of `string.tbl`, `patchstring.tbl` and
    /// `expansionstring.tbl`, by element number (`None`: table not loaded).
    base: Option<Vec<Vec<u16>>>,
    patch: Option<Vec<Vec<u16>>>,
    expansion: Option<Vec<Vec<u16>>>,
    by_key: HashMap<String, Vec<u16>>,
    /// The mod game's added and changed strings, read first.
    layer: StringLayer,
}

fn elements(t: &StringTable, keys: &mut Vec<(Vec<u8>, Vec<u16>)>) -> Vec<Vec<u16>> {
    (0..usize::from(t.header.num_elements))
        .map(|n| {
            let Some(e) = t.element(n) else {
                return Vec::new();
            };
            let text = d2_data::strings::decode_utf8_units(&e.value);
            if !e.key.is_empty() {
                keys.push((e.key.clone(), text.clone()));
            }
            text
        })
        .collect()
}

/// Element `n` of a table: a number past the
/// table's end reads element 500.
fn element_of(t: &[Vec<u16>], n: u32) -> Option<&[u16]> {
    let n = if n as usize >= t.len() {
        500
    } else {
        n as usize
    };
    t.get(n).map(Vec::as_slice)
}

impl TableStrings {
    /// Reads the tables of `lang` from `files`; the expansion table only when
    /// `files.lod`.
    pub fn load(files: &dyn TableFiles, lang: &str) -> Result<Self, StringsError> {
        Ok(Self::from_tables(&StringTables::load_from(
            files,
            lang,
            files.lod(),
        )?))
    }

    /// Every element of `t`. A key found in several tables resolves in the
    /// order of `StringTables::id`: a mod edit, then patch, expansion and
    /// base.
    pub fn from_tables(t: &StringTables) -> Self {
        let mut keys = Vec::new();
        // Lowest key priority first, later inserts win.
        let base = t.base.as_ref().map(|t| elements(t, &mut keys));
        let expansion = t.expansion.as_ref().map(|t| elements(t, &mut keys));
        let patch = t.patch.as_ref().map(|t| elements(t, &mut keys));
        for (key, _, text) in t.layer.entries() {
            keys.push((key.to_vec(), text.to_vec()));
        }
        let by_key = keys
            .into_iter()
            .map(|(k, v)| (String::from_utf8_lossy(&k).into_owned(), v))
            .collect();
        TableStrings {
            base,
            patch,
            expansion,
            by_key,
            layer: t.layer.clone(),
        }
    }

    /// The text of string `id`: a mod edit's text when one touched `id`;
    /// else id ≥ 20,000 reads `expansionstring.tbl` element `id − 20,000` when it is
    /// loaded, else the id becomes 11,078 and goes on; id ≥ 10,000 reads
    /// `patchstring.tbl` element `id − 10,000` when loaded; else element
    /// `id` of `string.tbl`.
    fn lookup(&self, id: u32) -> Option<&[u16]> {
        if let Some(text) = self.layer.text(id) {
            return Some(text);
        }
        let mut id = id;
        if id >= EXPANSION_BASE {
            if let Some(t) = &self.expansion {
                return element_of(t, id - EXPANSION_BASE);
            }
            id = 11_078;
        }
        if id >= PATCH_BASE {
            if let Some(t) = &self.patch {
                return element_of(t, id - PATCH_BASE);
            }
        }
        element_of(self.base.as_ref()?, id)
    }

    /// The text of string id `id`, empty when absent (the closure form
    /// the HUD / NPC / message panels take).
    pub fn by_id(&self, id: u16) -> Vec<u16> {
        self.lookup(u32::from(id))
            .map(<[u16]>::to_vec)
            .unwrap_or_default()
    }
}

impl StringLookup for TableStrings {
    fn get(&self, key: &str) -> Option<&[u16]> {
        self.by_key.get(key).map(Vec::as_slice)
    }

    fn get_id(&self, id: u16) -> Option<&[u16]> {
        self.lookup(u32::from(id))
    }
}

/// Binds `strings` as the play UI's lookup (no-op without a UI).
pub fn install_strings(app: &mut impl crate::app::session::Setup, strings: TableStrings) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        ui.strings = Box::new(strings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::tbl::{TblEntry, TblHeader};

    fn table(vals: &[(&str, &str)]) -> StringTable {
        let n = vals.len();
        StringTable {
            header: TblHeader {
                crc: 0,
                num_elements: n as u16,
                hash_table_size: n as u32,
                version: 0,
                data_start: 0,
                max_tries: 1,
                file_size: 0,
            },
            indices: (0..n as u16).collect(),
            entries: vals
                .iter()
                .enumerate()
                .map(|(i, (k, v))| TblEntry {
                    used: 1,
                    index: i as u16,
                    hash: 0,
                    key: k.as_bytes().to_vec(),
                    value: v.as_bytes().to_vec(),
                })
                .collect(),
        }
    }

    fn u16s(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn labels_resolve_by_id_and_key_across_tables() {
        let tables = StringTables {
            base: Some(table(&[("a", "Strength"), ("b", "Dexterity")])),
            patch: Some(table(&[("b", "Patched")])),
            expansion: Some(table(&[("x", "Expansion")])),
            layer: Default::default(),
        };
        let s = TableStrings::from_tables(&tables);
        assert_eq!(s.get_id(0), Some(u16s("Strength").as_slice()));
        assert_eq!(s.get_id(1), Some(u16s("Dexterity").as_slice()));
        assert_eq!(s.get_id(10_000), Some(u16s("Patched").as_slice()));
        assert_eq!(s.get_id(20_000), Some(u16s("Expansion").as_slice()));
        assert_eq!(s.get("b"), Some(u16s("Patched").as_slice()));
        assert_eq!(s.get("x"), Some(u16s("Expansion").as_slice()));
        assert_eq!(s.get_id(7), None);
        assert_eq!(s.by_id(0), u16s("Strength"));
        assert!(s.by_id(7).is_empty());
    }

    #[test]
    fn install_replaces_no_strings_in_the_ui() {
        use crate::ui::{NoPanelRules, UiRoot};
        let mut app = App::new();
        app.insert_non_send(WorldViewUi::new(
            UiRoot::new(Box::new(NoPanelRules)),
            Box::new(crate::ui::NoStrings),
        ));
        let tables = StringTables {
            base: Some(table(&[("a", "Strength")])),
            ..Default::default()
        };
        install_strings(&mut app, TableStrings::from_tables(&tables));
        let ui = app.world().non_send::<WorldViewUi>();
        assert_eq!(ui.strings.get_id(0), Some(u16s("Strength").as_slice()));
    }

    /// A mod game's added string reads by key and by its id; the tables'
    /// own strings read as before.
    #[test]
    fn mod_strings_read_by_key_and_id() {
        let mut tables = StringTables {
            base: Some(table(&[("a", "Strength")])),
            ..Default::default()
        };
        let id = tables
            .apply(&d2_data::strings::StringEdit::Add {
                key: b"zz_mod".to_vec(),
                text: b"Mod line".to_vec(),
            })
            .unwrap();
        let s = TableStrings::from_tables(&tables);
        assert_eq!(id, d2_data::strings::MOD_BASE);
        assert_eq!(s.get("zz_mod"), Some(u16s("Mod line").as_slice()));
        assert_eq!(s.get_id(id as u16), Some(u16s("Mod line").as_slice()));
        assert_eq!(s.by_id(0), u16s("Strength"));
        assert_eq!(s.get("a"), Some(u16s("Strength").as_slice()));
    }
}
