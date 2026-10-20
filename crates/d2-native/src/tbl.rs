//! String tables (`.tbl`) as `P.tsv` + `P.toml`: every piece of text the
//! game shows (item names, skill descriptions, quest dialogue, menus) is
//! looked up by key in one of these tables.
//!
//! `P.tsv` has one row per element, `id key value`, in element order:
//! the file a translator or modder edits. `P.toml` holds the header fields
//! the rows cannot give (`crc`, `version`, `hash_table_size`, `max_tries`)
//! and, for a converted table, an `[encoding]` table: the hash slot of
//! every element, plus any slot record whose stored `index` / `hash`
//! differ from what a rebuild would give. With `[encoding]` the table
//! comes back exactly as shipped; a table a modder writes from scratch
//! leaves it out and the reader rebuilds the slots from the rows.

use d2_formats::tbl::{key_hash, StringTable, TblEntry, TblHeader};

use crate::toml_kinds::{
    first_difference, tsv_escape, tsv_rows, tsv_unescape, Out, Tab, TextError,
};

const TSV_HEADER: &str = "id\tkey\tvalue";
/// Largest hash table a sidecar may ask for (live tables are under 12,000).
const MAX_SLOTS: u32 = 1 << 20;

/// The two native files of one string table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TblFiles {
    /// `P.tsv`.
    pub tsv: String,
    /// `P.toml`.
    pub toml: String,
}

/// The slot each element lands in when the rows are inserted in element
/// order: each key starts at its hash modulo the table size and takes the
/// next free slot after any taken ones (linear probing). `None` when the
/// rows do not fit (a table with no slots fits no rows).
pub fn rebuild_slots(keys: &[&[u8]], size: u32) -> Option<Vec<u16>> {
    if keys.is_empty() {
        return Some(Vec::new());
    }
    if size == 0 || keys.len() > size as usize {
        return None;
    }
    let mut taken = vec![false; size as usize];
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        let mut s = (key_hash(k) % size) as usize;
        while taken[s] {
            s = (s + 1) % size as usize;
        }
        taken[s] = true;
        out.push(u16::try_from(s).ok()?);
    }
    Some(out)
}

fn utf8<'a>(file: &str, what: &str, id: usize, b: &'a [u8]) -> Result<&'a str, TextError> {
    std::str::from_utf8(b).map_err(|_| {
        TextError::new(
            file,
            format!("not representable: element {id} {what} is not UTF-8"),
        )
    })
}

/// True if rebuilding the slots from the element order differs from the
/// table's own (the report counts these).
pub fn rebuild_differs(t: &StringTable) -> bool {
    let keys: Option<Vec<&[u8]>> = (0..t.indices.len())
        .map(|i| t.element(i).map(|e| e.key.as_slice()))
        .collect();
    match keys.and_then(|k| rebuild_slots(&k, t.header.hash_table_size)) {
        Some(s) => s != t.indices,
        None => true,
    }
}

/// Writes `P.tsv` and `P.toml`. With `encoding` the stored hash slots (and
/// the per-slot `index` / `hash` records that differ from what a rebuild
/// would produce) go in `[encoding]`; the converter always passes `true`.
pub fn write_tbl(file: &str, t: &StringTable, encoding: bool) -> Result<TblFiles, TextError> {
    let h = &t.header;
    let bad = |d: String| TextError::new(file, format!("not representable: {d}"));
    if t.entries.len() != h.hash_table_size as usize {
        return Err(bad("entries differ from hash_table_size".into()));
    }
    let mut tsv = String::from(TSV_HEADER);
    tsv.push('\n');
    // Slot → element number, for the check that every used slot is an element's.
    let mut owner: Vec<Option<usize>> = vec![None; t.entries.len()];
    for (id, &slot) in t.indices.iter().enumerate() {
        let e = t
            .element(id)
            .ok_or_else(|| bad(format!("element {id} points at an unused slot")))?;
        if owner[usize::from(slot)].replace(id).is_some() {
            return Err(bad(format!("two elements share slot {slot}")));
        }
        tsv.push_str(&format!(
            "{id}\t{}\t{}\n",
            tsv_escape(utf8(file, "key", id, &e.key)?),
            tsv_escape(utf8(file, "value", id, &e.value)?)
        ));
    }
    if let Some(s) = (0..t.entries.len()).find(|&s| t.entries[s].used != 0 && owner[s].is_none()) {
        return Err(bad(format!("used slot {s} belongs to no element")));
    }

    let mut o = Out::header("tbl");
    o.int("crc", h.crc);
    o.int("version", h.version);
    o.int("hash_table_size", h.hash_table_size);
    o.int("max_tries", h.max_tries);
    if encoding {
        o.table("[encoding]");
        o.ints("slots", t.indices.iter());
        for (s, e) in t.entries.iter().enumerate() {
            let (index, hash) = match owner[s] {
                Some(id) => (id as u16, key_hash(&e.key)),
                None => (0, 0),
            };
            if e.index != index || e.hash != hash {
                o.table("[[encoding.slot]]");
                o.int("slot", s);
                o.int("index", e.index);
                o.int("hash", e.hash);
            }
        }
    }
    Ok(TblFiles { tsv, toml: o.0 })
}

/// Reads `P.tsv` + `P.toml` back. `data_start` and `file_size` (layout
/// fields) are 0 in the result.
pub fn read_tbl(file: &str, tsv: &str, toml: &str) -> Result<StringTable, TextError> {
    let mut t = Tab::parse(file, toml, "tbl")?;
    let crc = t.u16("crc")?;
    let version = t.u8("version")?;
    let size = t.u32("hash_table_size")?;
    let max_tries = t.u32("max_tries")?;
    if size > MAX_SLOTS {
        return Err(t.err(format!("hash_table_size {size} is over {MAX_SLOTS}")));
    }

    let mut rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for (i, cells) in tsv_rows(file, tsv, TSV_HEADER, 3)?.into_iter().enumerate() {
        if cells[0] != i.to_string() {
            return Err(TextError::new(
                file,
                format!("line {}: id {:?}, expected {i}", i + 2, cells[0]),
            ));
        }
        let key = tsv_unescape(file, i + 2, cells[1])?;
        let value = tsv_unescape(file, i + 2, cells[2])?;
        rows.push((key.into_bytes(), value.into_bytes()));
    }
    let n =
        u16::try_from(rows.len()).map_err(|_| TextError::new(file, "more than 65,535 elements"))?;
    let keys: Vec<&[u8]> = rows.iter().map(|r| r.0.as_slice()).collect();
    let rebuilt =
        rebuild_slots(&keys, size).ok_or_else(|| t.err("rows do not fit in hash_table_size"))?;

    let mut exceptions: Vec<(u32, u16, u32)> = Vec::new();
    let slots = match t.table_opt("encoding")? {
        None => rebuilt,
        Some(mut e) => {
            let stored: Vec<u16> = e
                .int_array("slots", 0, i64::from(u16::MAX))?
                .into_iter()
                .map(|v| v as u16)
                .collect();
            for mut s in e.tables("slot")? {
                exceptions.push((s.u32("slot")?, s.u16("index")?, s.u32("hash")?));
                s.finish()?;
            }
            e.finish()?;
            if stored.len() != rows.len() {
                return Err(t.err(format!(
                    "[encoding] has {} slots for {} rows",
                    stored.len(),
                    rows.len()
                )));
            }
            if stored != rebuilt {
                let at = (0..stored.len())
                    .find(|&i| stored[i] != rebuilt[i])
                    .unwrap_or(0);
                return Err(t.err(format!(
                    "hash rebuild differs from [encoding] slots at element {at}: stored {}, rebuilt {}",
                    stored[at], rebuilt[at]
                )));
            }
            stored
        }
    };
    t.finish()?;

    let mut entries: Vec<TblEntry> = (0..size)
        .map(|_| TblEntry {
            used: 0,
            index: 0,
            hash: 0,
            key: Vec::new(),
            value: Vec::new(),
        })
        .collect();
    for (id, ((key, value), &slot)) in rows.into_iter().zip(&slots).enumerate() {
        let hash = key_hash(&key);
        entries[usize::from(slot)] = TblEntry {
            used: 1,
            index: id as u16,
            hash,
            key,
            value,
        };
    }
    for (slot, index, hash) in exceptions {
        let e = entries.get_mut(slot as usize).ok_or_else(|| {
            TextError::new(file, format!("[encoding] slot {slot} outside the table"))
        })?;
        e.index = index;
        e.hash = hash;
    }
    Ok(StringTable {
        header: TblHeader {
            crc,
            num_elements: n,
            hash_table_size: size,
            version,
            data_start: 0,
            max_tries,
            file_size: 0,
        },
        indices: slots,
        entries,
    })
}

/// The table with the layout fields zeroed, the form the table check compares.
fn normalized(t: &StringTable) -> StringTable {
    let mut n = t.clone();
    n.header.data_start = 0;
    n.header.file_size = 0;
    n
}

/// Table check: write, read back, compare the tables without
/// `data_start` / `file_size`, then look every key up in both.
pub fn check_tbl(file: &str, original: &StringTable) -> Result<(), TextError> {
    let f = write_tbl(file, original, true)?;
    let back = read_tbl(file, &f.tsv, &f.toml)?;
    let (a, b) = (normalized(original), normalized(&back));
    if a != b {
        return Err(TextError::new(file, first_difference(&a, &b)));
    }
    for i in 0..original.indices.len() {
        if let Some(e) = original.element(i) {
            let (x, y) = (original.find_slot(&e.key), back.find_slot(&e.key));
            if x != y {
                return Err(TextError::new(
                    file,
                    format!("lookup of element {i}: original slot {x:?}, native slot {y:?}"),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table laid out the way the game does: slots by hash and linear
    /// probe, `index` = element number, `hash` = the full key hash.
    fn build(pairs: &[(&str, &str)], size: u32) -> StringTable {
        let keys: Vec<&[u8]> = pairs.iter().map(|p| p.0.as_bytes()).collect();
        let slots = rebuild_slots(&keys, size).unwrap();
        let mut entries: Vec<TblEntry> = (0..size)
            .map(|_| TblEntry {
                used: 0,
                index: 0,
                hash: 0,
                key: vec![],
                value: vec![],
            })
            .collect();
        for (i, ((k, v), &s)) in pairs.iter().zip(&slots).enumerate() {
            entries[usize::from(s)] = TblEntry {
                used: 1,
                index: i as u16,
                hash: key_hash(k.as_bytes()),
                key: k.as_bytes().to_vec(),
                value: v.as_bytes().to_vec(),
            };
        }
        StringTable {
            header: TblHeader {
                crc: 0x1234,
                num_elements: pairs.len() as u16,
                hash_table_size: size,
                version: 1,
                data_start: 99,
                max_tries: size,
                file_size: 1234,
            },
            indices: slots,
            entries,
        }
    }

    fn sample() -> StringTable {
        // "A" and "Q" collide in a table of 16.
        build(
            &[
                ("A", "first"),
                ("Q", "tab\there\nline \\ back"),
                ("Key3", "ÿc1red"),
                ("Empty", ""),
            ],
            16,
        )
    }

    #[test]
    fn round_trip_with_escapes_and_utf8() {
        let t = sample();
        let f = write_tbl("x.tbl", &t, true).unwrap();
        assert!(f.tsv.starts_with("id\tkey\tvalue\n0\tA\tfirst\n"));
        assert!(f.tsv.contains("1\tQ\ttab\\there\\nline \\\\ back\n"));
        let back = read_tbl("x.tbl", &f.tsv, &f.toml).unwrap();
        assert_eq!(normalized(&t), back);
        check_tbl("x.tbl", &t).unwrap();
        // Deterministic.
        assert_eq!(f, write_tbl("x.tbl", &t, true).unwrap());
    }

    #[test]
    fn rebuild_without_encoding() {
        let t = sample();
        let f = write_tbl("x.tbl", &t, false).unwrap();
        assert!(!f.toml.contains("encoding"));
        let back = read_tbl("x.tbl", &f.tsv, &f.toml).unwrap();
        assert_eq!(normalized(&t), back);
        assert!(!rebuild_differs(&t));
    }

    #[test]
    fn odd_index_and_hash_fields_survive() {
        let mut t = sample();
        let s = usize::from(t.indices[0]);
        t.entries[s].hash ^= 0xff; // a stored hash that is not the key hash
        let unused = t.entries.iter().position(|e| e.used == 0).unwrap();
        t.entries[unused].index = 7; // an unused-slot record
        t.entries[unused].hash = 9;
        check_tbl("x.tbl", &t).unwrap();
        let f = write_tbl("x.tbl", &t, true).unwrap();
        assert_eq!(f.toml.matches("[[encoding.slot]]").count(), 2);
    }

    #[test]
    fn rebuild_difference_is_a_load_error_and_is_counted() {
        // Element 1 moved to a free slot a rebuild would not choose.
        let mut t = sample();
        let a = usize::from(t.indices[1]);
        let free = (0..16)
            .find(|&s| !t.entries[s].used != 0 && s != a)
            .unwrap();
        t.entries.swap(a, free);
        t.indices[1] = free as u16;
        assert!(rebuild_differs(&t));
        let f = write_tbl("x.tbl", &t, true).unwrap();
        let err = read_tbl("x.tbl", &f.tsv, &f.toml).unwrap_err();
        assert!(err.detail.contains("hash rebuild differs"), "{err}");
    }

    #[test]
    fn perturbation_reports_the_cell() {
        let t = sample();
        let f = write_tbl("x.tbl", &t, true).unwrap();
        let tsv = f.tsv.replace("\tfirst\n", "\tfirsu\n");
        let back = read_tbl("x.tbl", &tsv, &f.toml).unwrap();
        assert_ne!(normalized(&t), back);
        let d = first_difference(&normalized(&t), &back);
        assert!(d.contains("116") && d.contains("117"), "{d}"); // 't' vs 'u'
    }

    #[test]
    fn strict_readers() {
        let t = sample();
        let f = write_tbl("x.tbl", &t, true).unwrap();
        let ok = |tsv: &str, toml: &str| read_tbl("x.tbl", tsv, toml);
        assert!(ok(
            &f.tsv,
            &f.toml.replace("native_version = 1", "native_version = 2")
        )
        .is_err());
        assert!(ok(&f.tsv, &f.toml.replace("\"tbl\"", "\"wav\"")).is_err());
        assert!(ok(
            &f.tsv,
            &format!("{}extra = 1\n", f.toml.replace("[encoding]", ""))
        )
        .is_err());
        assert!(ok(&f.tsv.replace("id\tkey", "id\tkee"), &f.toml).is_err());
        assert!(ok(&f.tsv.replace("\\t", "\\x"), &f.toml).is_err());
        assert!(ok(&f.tsv.replacen("\n1\t", "\n2\t", 1), &f.toml).is_err());
        assert!(ok(f.tsv.trim_end(), &f.toml).is_err());
        let err = ok(&f.tsv, &f.toml.replace("crc = 4660", "crc = 70000")).unwrap_err();
        assert!(err.to_string().starts_with("x.tbl: "), "{err}");
    }

    #[test]
    fn non_utf8_text_is_refused() {
        let mut t = sample();
        let s = usize::from(t.indices[0]);
        t.entries[s].value = vec![0xff, 0xfe];
        let err = write_tbl("x.tbl", &t, true).unwrap_err();
        assert!(err.detail.contains("not UTF-8"), "{err}");
    }

    #[test]
    fn empty_table() {
        let t = build(&[], 1);
        check_tbl("e.tbl", &t).unwrap();
    }
}
