//! The game's string tables and the `strkey` resolver.
//!
//! Loads `string.tbl`, `patchstring.tbl` and (with the expansion) `expansionstring.tbl`
//! through `d2_formats::tbl`, resolves a text key to a string id for `strkey` columns
//! (used by `compile`), and looks up display strings by id. `bin` loads a
//! [`StringTables`] with the rest of the data tables.
//!
//! A mod game changes strings with patch layers (`stradd` / `strset`, see
//! `crate::patch`), never with copies of the string tables: the edits form
//! a [`StringLayer`] over the loaded tables. A changed key keeps its id
//! and reads its new text; an added key gets the next id from
//! [`MOD_BASE`]. With no layer (D2RUST's own game) every lookup is the
//! tables' own.

use std::collections::BTreeMap;

use d2_formats::mpq::{ArchiveSet, MpqError};
use d2_formats::tbl::StringTable;
use d2_formats::FormatError;

/// The no-string ID (`string.tbl` element 5382, key `dummy`).
pub const NO_STRING: u16 = 5382;
/// Id offset of `patchstring.tbl` elements.
pub const PATCH_BASE: u32 = 10_000;
/// Id offset of `expansionstring.tbl` elements.
pub const EXPANSION_BASE: u32 = 20_000;
/// First id of the strings a mod game's layers add; ids stay below
/// 65,536 because `strkey` cells store 16 bits.
pub const MOD_BASE: u32 = 30_000;

/// Decodes string-table bytes as UTF-8 into UTF-16 units; an invalid
/// sequence ends the string at that point.
pub fn decode_utf8_units(bytes: &[u8]) -> Vec<u16> {
    let valid = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    };
    valid.encode_utf16().collect()
}

/// Lookup by id failed: the element is missing from its table.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("string id {0}: element missing")]
pub struct NoSuchString(pub u32);

/// The element within one table: a
/// number >= `num_elements` reads element 500.
fn table_element(t: &StringTable, n: u32) -> Option<Vec<u16>> {
    let n = if n >= u32::from(t.header.num_elements) {
        500
    } else {
        n
    };
    t.element(n as usize).map(|e| decode_utf8_units(&e.value))
}

/// A string table that failed to load.
#[derive(Debug, thiserror::Error)]
pub enum StringsError {
    /// The archive set failed to read the table.
    #[error("{file}: {source}")]
    Mpq { file: String, source: MpqError },
    /// The table file could not be parsed.
    #[error("{file}: {source}")]
    Format { file: String, source: FormatError },
    /// A read error from a native (converted) source.
    #[error("{file}: {detail}")]
    Source { file: String, detail: String },
}

/// `string.tbl`, `patchstring.tbl` and (LoD only) `expansionstring.tbl`.
#[derive(Debug, Clone, Default)]
pub struct StringTables {
    /// `string.tbl`.
    pub base: Option<StringTable>,
    /// `patchstring.tbl`.
    pub patch: Option<StringTable>,
    /// Present exactly when the expansion is installed ("LoD").
    pub expansion: Option<StringTable>,
    /// The strings a mod game's patch layers added or changed (empty for
    /// D2RUST's own game); read before the tables.
    pub layer: StringLayer,
}

/// One string change from a mod game's patch layer. The text is UTF-8, as
/// the string tables store it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringEdit {
    /// `stradd`: a new key; it must not name a string the tables or an
    /// earlier edit already have.
    Add { key: Vec<u8>, text: Vec<u8> },
    /// `strset`: new text for a key the tables or an earlier `Add` have.
    Set { key: Vec<u8>, text: Vec<u8> },
}

/// Why a [`StringEdit`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StringEditError {
    /// `Add` of a key that already names a string.
    #[error("string key already exists")]
    KeyExists,
    /// `Set` of a key no string has.
    #[error("no string has this key")]
    NoSuchKey,
    /// `Add` past the last id a `strkey` cell can hold.
    #[error("no string id left for a new key")]
    Full,
    /// An empty key: it reads as "no string" everywhere.
    #[error("empty string key")]
    EmptyKey,
}

/// The strings a mod game's layers added or changed, over the loaded
/// tables. Empty for D2RUST's own game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringLayer {
    /// Every edited key and its id (a table's id for a changed key, an id
    /// from [`MOD_BASE`] for an added one).
    keys: BTreeMap<Vec<u8>, u32>,
    /// The current text of every edited id, as UTF-16 units.
    texts: BTreeMap<u32, Vec<u16>>,
    /// Keys added so far: the next added key gets `MOD_BASE + added`.
    added: u32,
}

impl StringLayer {
    /// No string was added or changed.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The id of an added key (changed keys resolve through the tables).
    fn added_id(&self, key: &[u8]) -> Option<u32> {
        self.keys.get(key).copied().filter(|&id| id >= MOD_BASE)
    }

    /// The edited text of `id`, `None` when no layer touched it.
    pub fn text(&self, id: u32) -> Option<&[u16]> {
        self.texts.get(&id).map(Vec::as_slice)
    }

    /// Every edited key with its id and text, in key order.
    pub fn entries(&self) -> impl Iterator<Item = (&[u8], u32, &[u16])> {
        self.keys
            .iter()
            .map(|(k, id)| (k.as_slice(), *id, self.texts[id].as_slice()))
    }
}

/// Element number of `key` in `t`.
fn element(t: &StringTable, key: &[u8]) -> Option<u32> {
    t.find_slot(key).map(|s| u32::from(t.entries[s].index))
}

impl StringTables {
    /// Loads the tables of language `lang` (e.g. `eng`) from the archive
    /// set: `string.tbl`, `patchstring.tbl`, then `expansionstring.tbl`
    /// when `lod`.
    pub fn load(set: &ArchiveSet, lang: &str, lod: bool) -> Result<StringTables, StringsError> {
        StringTables::load_from(set, lang, lod)
    }

    /// [`StringTables::load`] over any table source: the archive set or a
    /// native folder.
    pub fn load_from(
        set: &dyn crate::bin::TableFiles,
        lang: &str,
        lod: bool,
    ) -> Result<StringTables, StringsError> {
        let read = |name: &str| set.string_table(&format!("data\\local\\lng\\{lang}\\{name}"));
        let mut t = StringTables {
            base: Some(read("string.tbl")?),
            patch: Some(read("patchstring.tbl")?),
            expansion: if lod {
                Some(read("expansionstring.tbl")?)
            } else {
                None
            },
            layer: StringLayer::default(),
        };
        let edits = set.string_edits();
        t.apply_all(&edits).map_err(|(i, e)| StringsError::Source {
            file: "string patches".into(),
            detail: format!("edit {}: {e}", i + 1),
        })?;
        Ok(t)
    }

    /// Applies one mod string edit; returns the id the key now reads at.
    pub fn apply(&mut self, edit: &StringEdit) -> Result<u32, StringEditError> {
        let (key, text, add) = match edit {
            StringEdit::Add { key, text } => (key, text, true),
            StringEdit::Set { key, text } => (key, text, false),
        };
        if key.is_empty() {
            return Err(StringEditError::EmptyKey);
        }
        let id = match (self.id(key), add) {
            (0, true) => {
                let id = MOD_BASE + self.layer.added;
                if id > u32::from(u16::MAX) {
                    return Err(StringEditError::Full);
                }
                self.layer.added += 1;
                id
            }
            (_, true) => return Err(StringEditError::KeyExists),
            (0, false) => return Err(StringEditError::NoSuchKey),
            (id, false) => id,
        };
        self.layer.keys.insert(key.clone(), id);
        self.layer.texts.insert(id, decode_utf8_units(text));
        Ok(id)
    }

    /// Applies `edits` in order; stops at the first refused one, returning
    /// its position and the reason.
    pub fn apply_all(&mut self, edits: &[StringEdit]) -> Result<(), (usize, StringEditError)> {
        for (i, e) in edits.iter().enumerate() {
            self.apply(e).map_err(|err| (i, err))?;
        }
        Ok(())
    }

    /// The string ID of `text`, without the no-string substitution: 0 for
    /// an empty or unknown key.
    pub fn id(&self, text: &[u8]) -> u32 {
        if text.is_empty() {
            return 0;
        }
        if let Some(e) = self.patch.as_ref().and_then(|t| element(t, text)) {
            return e + PATCH_BASE;
        }
        if let Some(e) = self.expansion.as_ref().and_then(|t| element(t, text)) {
            return e + EXPANSION_BASE;
        }
        if let Some(e) = self.base.as_ref().and_then(|t| element(t, text)) {
            return e;
        }
        self.layer.added_id(text).unwrap_or(0)
    }

    /// The display string for `id` as UTF-16 units: the mod layer's text
    /// when a layer edited `id`; else id >= 20,000 reads
    /// `expansionstring.tbl` element `id - 20,000` when it is loaded (else
    /// the id becomes 11,078 and goes on), id >= 10,000 reads
    /// `patchstring.tbl` element `id - 10,000` when loaded, else element
    /// `id` of `string.tbl`.
    pub fn by_index(&self, id: u32) -> Result<Vec<u16>, NoSuchString> {
        if let Some(text) = self.layer.text(id) {
            return Ok(text.to_vec());
        }
        let mut id = id;
        if id >= EXPANSION_BASE {
            if let Some(t) = &self.expansion {
                return table_element(t, id - EXPANSION_BASE).ok_or(NoSuchString(id));
            }
            id = 11_078;
        }
        if id >= PATCH_BASE {
            if let Some(t) = &self.patch {
                return table_element(t, id - PATCH_BASE).ok_or(NoSuchString(id));
            }
        }
        let t = self.base.as_ref().ok_or(NoSuchString(id))?;
        table_element(t, id).ok_or(NoSuchString(id))
    }

    /// The `strkey` value stored for a bound cell: the ID, or 5382 when it
    /// is 0. `text` is the first 256 cell bytes.
    pub fn strkey(&self, text: &[u8]) -> u16 {
        match self.id(text) {
            0 => NO_STRING,
            id => id as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of `n` elements where element `i` has value `vals[i]`
    /// (slot = element number).
    fn table(vals: &[&[u8]]) -> StringTable {
        use d2_formats::tbl::{TblEntry, TblHeader};
        StringTable {
            header: TblHeader {
                crc: 0,
                num_elements: vals.len() as u16,
                hash_table_size: vals.len() as u32,
                version: 0,
                data_start: 0,
                max_tries: 1,
                file_size: 0,
            },
            indices: (0..vals.len() as u16).collect(),
            entries: vals
                .iter()
                .enumerate()
                .map(|(i, v)| TblEntry {
                    used: 1,
                    index: i as u16,
                    hash: 0,
                    key: Vec::new(),
                    value: v.to_vec(),
                })
                .collect(),
        }
    }

    #[test]
    fn utf8_decode_ends_at_invalid() {
        // ÿ is stored as C3 BF and decodes to U+00FF.
        assert_eq!(
            decode_utf8_units(b"\xC3\xBFc4Gold"),
            [0xFF, 0x63, 0x34, 0x47, 0x6F, 0x6C, 0x64]
        );
        assert_eq!(decode_utf8_units(b"ab\xFFcd"), [0x61, 0x62]);
        assert_eq!(decode_utf8_units(b"ab\xC3"), [0x61, 0x62]);
    }

    #[test]
    fn lookup_by_id() {
        let mut base_vals: Vec<&[u8]> = vec![b"b"; 600];
        base_vals[500] = b"fallback500";
        base_vals[3] = b"three";
        let mut patch_vals: Vec<&[u8]> = vec![b"p"; 1100];
        patch_vals[1078] = b"patch1078";
        let mut t = StringTables {
            base: Some(table(&base_vals)),
            patch: Some(table(&patch_vals)),
            expansion: Some(table(&[b"exp0", b"exp1"])),
            layer: Default::default(),
        };
        let s = |v: Vec<u16>| String::from_utf16(&v).unwrap();
        assert_eq!(s(t.by_index(3).unwrap()), "three");
        assert_eq!(s(t.by_index(10_000 + 1078).unwrap()), "patch1078");
        assert_eq!(s(t.by_index(20_000).unwrap()), "exp0");
        // element number >= num_elements reads element 500
        assert_eq!(s(t.by_index(599 + 1).unwrap()), "fallback500");
        // ... which a two-element table does not have: an error
        assert_eq!(t.by_index(20_007), Err(NoSuchString(20_007)));
        // expansion not loaded: 20,000 becomes 11,078 -> patch element 1,078
        t.expansion = None;
        assert_eq!(s(t.by_index(20_000).unwrap()), "patch1078");
        // patch not loaded either: element 11,078 of string.tbl -> 500
        t.patch = None;
        assert_eq!(s(t.by_index(10_005).unwrap()), "fallback500");
    }

    #[test]
    fn empty_tables_give_no_string() {
        let t = StringTables::default();
        assert_eq!(t.id(b""), 0);
        assert_eq!(t.strkey(b""), NO_STRING);
        assert_eq!(t.strkey(b"cap"), NO_STRING);
    }

    fn game_strings() -> Option<StringTables> {
        let dir = std::env::var("D2_GAME_DIR").ok()?;
        let set = ArchiveSet::open_dir(dir).ok()?;
        Some(StringTables::load(&set, "eng", true).expect("string tables"))
    }

    /// Key-to-id results on a real English LoD install.
    #[test]
    #[ignore = "needs original game files in D2_GAME_DIR"]
    fn strkey_vectors() {
        let t = game_strings().expect("D2_GAME_DIR must be set");
        let cases: [(&[u8], u16); 12] = [
            (b"", 5382),
            (b"cap", 1930),
            (b"Cap", 5382),
            (b"CAP", 5382),
            (b"x", 10016),
            (b"Cutthroat1", 10000),
            (b"ModStr5d", 10001),
            (b"A4Q2ExpansionSuccessTyrael", 20000),
            (b"ob1", 20281),
            (b"Lilith", 11154),
            (b"WarrivAct1IntroGossip1", 5382),
            (b"nonexistent_key_zz", 5382),
        ];
        for (text, id) in cases {
            assert_eq!(t.strkey(text), id, "{}", String::from_utf8_lossy(text));
        }
        assert_eq!(t.strkey(b"dummy"), 5382);
    }

    /// `patchstring.tbl` holding one key at element 3 (id 10,003).
    fn keyed() -> StringTables {
        use d2_formats::tbl::{key_hash, TblEntry, TblHeader};
        let patch = StringTable {
            header: TblHeader {
                crc: 0,
                num_elements: 4,
                hash_table_size: 1,
                version: 0,
                data_start: 0,
                max_tries: 1,
                file_size: 0,
            },
            indices: vec![0; 4],
            entries: vec![TblEntry {
                used: 1,
                index: 3,
                hash: key_hash(b"Hello"),
                key: b"Hello".to_vec(),
                value: b"Hi".to_vec(),
            }],
        };
        StringTables {
            patch: Some(patch),
            ..Default::default()
        }
    }

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// A mod layer: a changed key keeps its id and reads its new text, an
    /// added key reads at the next id from 30,000 and binds in `strkey`;
    /// every other lookup is the tables' own.
    #[test]
    fn string_layer_adds_and_changes() {
        let mut t = keyed();
        let before = t.clone();
        let add = |k: &str, v: &str| StringEdit::Add {
            key: k.into(),
            text: v.into(),
        };
        let set = |k: &str, v: &str| StringEdit::Set {
            key: k.into(),
            text: v.into(),
        };
        assert_eq!(t.apply(&set("Hello", "Bonjour")), Ok(10_003));
        assert_eq!(t.apply(&add("NewKey", "Fresh")), Ok(MOD_BASE));
        assert_eq!(t.apply(&add("Other", "Two")), Ok(MOD_BASE + 1));
        assert_eq!(t.apply(&set("NewKey", "Fresher")), Ok(MOD_BASE));
        assert_eq!(t.id(b"Hello"), 10_003);
        assert_eq!(t.by_index(10_003), Ok(u("Bonjour")));
        assert_eq!(t.strkey(b"NewKey"), 30_000);
        assert_eq!(t.by_index(MOD_BASE), Ok(u("Fresher")));
        assert_eq!(t.by_index(MOD_BASE + 1), Ok(u("Two")));
        // Untouched ids read as before.
        for id in [0, 10_000, 10_002, MOD_BASE + 2] {
            assert_eq!(t.by_index(id), before.by_index(id), "{id}");
        }
        assert_eq!(t.strkey(b"Missing"), NO_STRING);
        let keys: Vec<_> = t
            .layer
            .entries()
            .map(|(k, id, _)| (k.to_vec(), id))
            .collect();
        assert_eq!(
            keys,
            [
                (b"Hello".to_vec(), 10_003),
                (b"NewKey".to_vec(), MOD_BASE),
                (b"Other".to_vec(), MOD_BASE + 1)
            ]
        );

        // Refusals change nothing.
        let snapshot = t.layer.clone();
        assert_eq!(t.apply(&add("Hello", "x")), Err(StringEditError::KeyExists));
        assert_eq!(t.apply(&add("Other", "x")), Err(StringEditError::KeyExists));
        assert_eq!(t.apply(&set("Nope", "x")), Err(StringEditError::NoSuchKey));
        assert_eq!(t.apply(&set("", "x")), Err(StringEditError::EmptyKey));
        assert_eq!(t.layer, snapshot);
        assert_eq!(
            t.apply_all(&[add("A", "1"), set("B", "2"), add("C", "3")]),
            Err((1, StringEditError::NoSuchKey))
        );
        assert_eq!(t.id(b"A"), MOD_BASE + 2);
        assert_eq!(t.id(b"C"), 0);
    }

    /// Added ids stop at 65,535, the largest a `strkey` cell holds.
    #[test]
    fn string_layer_id_limit() {
        let mut t = StringTables::default();
        let room = u32::from(u16::MAX) - MOD_BASE + 1;
        for n in 0..room {
            let key = format!("k{n}").into_bytes();
            let e = StringEdit::Add {
                key,
                text: Vec::new(),
            };
            assert!(t.apply(&e).is_ok(), "{n}");
        }
        let last = StringEdit::Add {
            key: b"one more".to_vec(),
            text: Vec::new(),
        };
        assert_eq!(t.apply(&last), Err(StringEditError::Full));
        assert_eq!(t.id(b"k0"), MOD_BASE);
        assert_eq!(t.strkey(&format!("k{}", room - 1).into_bytes()), u16::MAX);
    }
}
