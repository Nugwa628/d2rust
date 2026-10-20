//! Mutation-testing kills for `compile_set`, `crosscheck`, `links`, `schema`,
//! `strings`, `tables` and `txt`: tests that fail on mutants `cargo mutants`
//! reported as missed. Each test pins one behaviour of those modules.

use std::collections::BTreeMap;

use d2_formats::mpq::ArchiveSet;

use crate::bin::{excel_path, BinSet, BinTable, CodeFile};
use crate::compile::{Compiled, DiagKind, Diagnostic, Linkers, SETS_LINKER, UNIQUES_LINKER};
use crate::compile_set::{compile_all, CompileSetError, CompiledSet, CompiledTable};
use crate::crosscheck::{compare_sets, CrossCheck, Role, TableReport, REASON_NAMESTR_707};
use crate::links::{load_lookups, validate, validate_set, LinkerSizes};
use crate::schema::{schema, CalcBuffer, FieldType};
use crate::strings::StringTables;
use crate::txt::{ErrorCode, TxtTable};

// ------------------------------------------------------------ helpers

fn size(name: &str) -> usize {
    schema().table(name).unwrap().record_size
}

/// `count` zeroed records of the schema's size.
fn table(name: &str, count: usize) -> BinTable {
    BinTable {
        name: name.to_owned(),
        source: "patch_d2.mpq".to_owned(),
        count,
        record_size: size(name),
        records: vec![0; count * size(name)],
    }
}

fn poke(t: &mut BinTable, row: usize, offset: usize, bytes: &[u8]) {
    let o = row * t.record_size + offset;
    t.records[o..o + bytes.len()].copy_from_slice(bytes);
}

/// A unique, empty temporary directory, removed on drop.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "d2-data-mutants-misc-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn set(&self) -> ArchiveSet {
        ArchiveSet::open_dir(&self.0).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Encrypts `plain` so that `crypto::decrypt(_, key)` gives it back.
fn encrypt(plain: &[u8], key: u32) -> Vec<u8> {
    use d2_formats::mpq::crypto::decrypt;
    let mut cipher = vec![0u8; plain.len()];
    for w in (0..plain.len()).step_by(4) {
        let mut probe = cipher[..w + 4].to_vec();
        decrypt(&mut probe, key);
        for k in 0..4 {
            cipher[w + k] = probe[w + k] ^ plain[w + k];
        }
    }
    cipher
}

/// A minimal format-0 MPQ of stored, single-unit files.
fn mpq_bytes(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    use d2_formats::mpq::crypto::{hash, HashType, BLOCK_TABLE_KEY, HASH_TABLE_KEY};
    use d2_formats::mpq::flags;
    let n = (files.len() * 2).next_power_of_two().max(4);
    let mut out = vec![0u8; 32];
    let mut blocks = Vec::new();
    let mut hashes = vec![[u32::MAX; 4]; n];
    for (i, (name, bytes)) in files.iter().enumerate() {
        let len = bytes.len() as u32;
        blocks.push([
            out.len() as u32,
            len,
            len,
            flags::EXISTS | flags::SINGLE_UNIT,
        ]);
        out.extend_from_slice(bytes);
        let mut at = hash(name.as_bytes(), HashType::TableOffset) as usize & (n - 1);
        while hashes[at][3] != u32::MAX {
            at = (at + 1) & (n - 1);
        }
        hashes[at] = [
            hash(name.as_bytes(), HashType::NameA),
            hash(name.as_bytes(), HashType::NameB),
            0,
            i as u32,
        ];
    }
    let words =
        |t: &[[u32; 4]]| -> Vec<u8> { t.iter().flatten().flat_map(|w| w.to_le_bytes()).collect() };
    let hash_pos = out.len() as u32;
    out.extend(encrypt(&words(&hashes), HASH_TABLE_KEY));
    let block_pos = out.len() as u32;
    out.extend(encrypt(&words(&blocks), BLOCK_TABLE_KEY));
    let mut h = Vec::with_capacity(32);
    h.extend_from_slice(b"MPQ\x1A");
    for v in [32, out.len() as u32] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    h.extend_from_slice(&0u16.to_le_bytes()); // format version
    h.extend_from_slice(&3u16.to_le_bytes()); // sector size shift
    for v in [hash_pos, block_pos, n as u32, files.len() as u32] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    out[..32].copy_from_slice(&h);
    out
}

/// A `.bin` of `count` zero records of `size` bytes.
fn bin_file(count: u32, size: usize) -> Vec<u8> {
    let mut d = count.to_le_bytes().to_vec();
    d.resize(4 + count as usize * size, 0);
    d
}

/// Compiles a synthetic text set: per `.txt`, the union of the columns of
/// every list compiled from it as header and one record, empty except for
/// `cells` (txt file, column, value).
fn synthetic_compile(cells: &[(&str, &str, &str)]) -> Result<CompiledSet, CompileSetError> {
    let mut read = |file: &str| -> Result<Option<(String, Vec<u8>)>, String> {
        let mut cols: Vec<Vec<u8>> = Vec::new();
        for d in schema().called().filter(|d| d.txt_name == file) {
            for f in &d.fields {
                if !cols.contains(&f.column) {
                    cols.push(f.column.clone());
                }
            }
        }
        let mut out = cols.join(&b'\t');
        out.extend_from_slice(b"\r\n");
        let row: Vec<Vec<u8>> = cols
            .iter()
            .map(|c| {
                cells
                    .iter()
                    .find(|(f, col, _)| *f == file && col.as_bytes() == c.as_slice())
                    .map(|(_, _, v)| v.as_bytes().to_vec())
                    .unwrap_or_default()
            })
            .collect();
        out.extend(row.join(&b'\t'));
        out.extend_from_slice(b"\r\n");
        Ok(Some(("patch_d2.mpq".into(), out)))
    };
    compile_all(&mut read, &StringTables::default())
}

fn record<'a>(c: &'a CompiledSet, name: &str, r: usize) -> &'a [u8] {
    c.table(name).expect("compiled").compiled.record(r)
}

// ======================================================== compile_set.rs

/// `CompiledSet::table` finds a compiled table by its `tables.tsv` name.
#[test]
fn compiled_set_table_by_name() {
    let c = synthetic_compile(&[]).expect("synthetic set compiles");
    assert_eq!(c.tables.len(), schema().called().count());
    let w = c.table("weapons").expect("weapons compiled");
    assert_eq!(w.name, "weapons");
    assert_eq!(w.compiled.record_size, size("weapons"));
    assert_eq!(c.table("leveldefs").unwrap().name, "leveldefs");
    assert!(c.table("no such table").is_none());
}

/// `@uniques` and `@sets` are registered by the `uniqueitems` and
/// `setitems` loaders after their compiles, and cube inputs naming them resolve to
/// number + 1, the item of the unique's `code` / the set item's `item`,
/// quality 7 / 5.
#[test]
fn special_linkers_feed_cube_inputs() {
    let c = synthetic_compile(&[
        ("weapons.txt", "code", "axe"),
        ("armor.txt", "code", "cap"),
        ("uniqueitems.txt", "index", "Foolsgold"),
        ("uniqueitems.txt", "code", "axe"),
        ("setitems.txt", "index", "Barsetitem"),
        ("setitems.txt", "item", "cap"),
        ("cubemain.txt", "input 1", "Foolsgold"),
        ("cubemain.txt", "input 2", "Barsetitem"),
    ])
    .expect("synthetic set compiles");
    let uniques = c.linkers.name(UNIQUES_LINKER).expect("@uniques built");
    assert_eq!(uniques.find(b"foolsgold"), Some(0));
    let sets = c.linkers.name(SETS_LINKER).expect("@sets built");
    assert_eq!(sets.find(b"barsetitem"), Some(0));
    let cube = record(&c, "cubemain", 0);
    // weapons row 0 `axe` = item 0, armor row 0 `cap` = item 1.
    assert_eq!(cube[20..28], [0x41, 0, 0, 0, 1, 0, 7, 0]);
    assert_eq!(cube[28..36], [0x41, 0, 1, 0, 1, 0, 5, 0]);
}

/// `@treasureclass` is built at `treasureclassex` from the itemtypes
/// records (32 automatic TCs per type with byte 0x1D ≠ 0) and the TC names
/// before monstats reads it.
#[test]
fn treasure_class_linker_from_compiled_tables() {
    let c = synthetic_compile(&[
        ("itemtypes.txt", "code", "abc"),
        ("itemtypes.txt", "treasureclass", "1"),
        ("treasureclassex.txt", "treasure class", "Act 1 Foo"),
        ("monstats.txt", "TreasureClass1", "Act 1 Foo"),
        ("monstats.txt", "TreasureClass2", "abc3"),
    ])
    .expect("synthetic set compiles");
    let tc = c
        .linkers
        .name("@treasureclass")
        .expect("@treasureclass built");
    assert_eq!(tc.len(), 1 + 32 + 1);
    let m = record(&c, "monstats", 0);
    assert_eq!(u16::from_le_bytes([m[134], m[135]]), 33);
    assert_eq!(u16::from_le_bytes([m[136], m[137]]), 1);
}

// ========================================================= crosscheck.rs

fn compiled_table(name: &str, recs: &BinTable, diags: &[DiagKind]) -> CompiledTable {
    CompiledTable {
        name: name.to_owned(),
        txt_source: "patch_d2.mpq".into(),
        compiled: Compiled {
            count: recs.count,
            record_size: recs.record_size,
            records: recs.records.clone(),
            diagnostics: diags
                .iter()
                .map(|&kind| Diagnostic {
                    kind,
                    line: 2,
                    column: Some(0),
                    field: None,
                })
                .collect(),
        },
    }
}

fn empty_txt() -> TxtTable {
    TxtTable {
        header: Vec::new(),
        records: Vec::new(),
        removed_lines: Vec::new(),
    }
}

/// Runs `compare_sets` on `(compiled, shipped)` runtime pairs plus a
/// matching `hitclass`, a `_lookup` list, a `bodylocs` by-product with no
/// `.bin` in the (empty) archive set, and the four code buffers
/// `(shipped, compiled)`.
fn cross(pairs: Vec<(CompiledTable, BinTable)>, buffers: [(Vec<u8>, Vec<u8>); 4]) -> CrossCheck {
    let dir = TempDir::new("cross");
    let hitclass = table("hitclass", 2);
    let mut compiled = vec![
        compiled_table("hitclass", &hitclass, &[]),
        compiled_table("monmode_lookup", &table("monmode_lookup", 1), &[]),
        compiled_table("bodylocs", &table("bodylocs", 1), &[]),
    ];
    let mut tables = Vec::new();
    for (c, b) in pairs {
        compiled.push(c);
        tables.push(b);
    }
    let mut code = BTreeMap::new();
    let mut ours = BTreeMap::new();
    for (b, (shipped, compiled)) in CalcBuffer::ALL.into_iter().zip(buffers) {
        code.insert(
            b,
            CodeFile {
                buffer: b,
                source: "patch_d2.mpq".into(),
                bytes: shipped,
            },
        );
        ours.insert(b, compiled);
    }
    let data = BinSet {
        lod: true,
        rules: Default::default(),
        strings: StringTables::default(),
        tables,
        code,
        code_reports: BTreeMap::new(),
        hitclass,
        sounds: empty_txt(),
        soundenviron: empty_txt(),
    };
    let set = CompiledSet {
        tables: compiled,
        buffers: ours,
        unspecified_callbacks: BTreeMap::new(),
        calc_diagnostics: BTreeMap::new(),
        linkers: Linkers::default(),
    };
    compare_sets(&dir.set(), &data, &set).expect("comparison runs")
}

fn same_buffers() -> [(Vec<u8>, Vec<u8>); 4] {
    std::array::from_fn(|_| (vec![1, 2, 3], vec![1, 2, 3]))
}

fn report<'a>(c: &'a CrossCheck, name: &str) -> &'a TableReport {
    c.tables.iter().find(|t| t.name == name).expect("reported")
}

fn same(name: &str, t: BinTable, diags: &[DiagKind]) -> (CompiledTable, BinTable) {
    (compiled_table(name, &t, diags), t)
}

/// Roles and verdicts: count and every record byte compared for the
/// runtime tables and `hitclass`; a by-product absent from the archives is
/// not compared; `_lookup` lists are compared through their runtime table
/// Diagnostics are counted by kind over all compiled lists.
#[test]
fn crosscheck_roles_counts_and_verdicts() {
    let mut pet = table("pettype", 3);
    let pet_c = {
        let mut c = table("pettype", 2);
        poke(&mut c, 0, 0, &[0x55]);
        compiled_table("pettype", &c, &[])
    };
    poke(&mut pet, 0, 0, &[0x55]);
    let report_set = cross(
        vec![
            same("compcode", table("compcode", 2), &[DiagKind::LinkMiss]),
            same(
                "montype",
                table("montype", 1),
                &[DiagKind::LinkMiss, DiagKind::IntSyntax],
            ),
            (pet_c, pet),
        ],
        [
            (vec![1, 2, 3], vec![1, 2, 3]),
            (vec![1, 2, 3], vec![1, 2, 4]),
            (vec![1], vec![1, 5]),
            (vec![], vec![]),
        ],
    );
    let names: Vec<&str> = report_set.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["hitclass", "bodylocs", "compcode", "montype", "pettype"]
    );

    let hit = report(&report_set, "hitclass");
    assert_eq!(hit.role, Role::ClientOnly);
    assert!(hit.compared() && hit.counts_equal() && hit.identical() && hit.matches());

    let body = report(&report_set, "bodylocs");
    assert_eq!(body.role, Role::ByProduct);
    assert!(!body.compared() && !body.counts_equal());
    assert!(!body.identical() && !body.matches());
    assert!(body.note.is_some());

    let cc = report(&report_set, "compcode");
    assert_eq!(cc.role, Role::Runtime);
    assert!(cc.compared() && cc.counts_equal() && cc.identical() && cc.matches());

    // Counts differ (2 compiled, 3 shipped): no byte comparison, no match.
    let p = report(&report_set, "pettype");
    assert_eq!((p.records_txt, p.records_bin), (2, Some(3)));
    assert!(p.compared() && !p.counts_equal());
    assert!(p.explained.is_empty() && p.mismatches.is_empty());
    assert!(!p.identical() && !p.matches());

    // compcode and montype; hitclass matches but is not a runtime table.
    assert_eq!(report_set.runtime_matching(), 2);

    assert_eq!(
        report_set.diagnostics,
        BTreeMap::from([(DiagKind::IntSyntax, 1), (DiagKind::LinkMiss, 2)])
    );

    let firsts: Vec<Option<usize>> = report_set
        .buffers
        .iter()
        .map(|b| b.first_difference)
        .collect();
    assert_eq!(firsts, [None, Some(2), Some(1), None]);
    let identical: Vec<bool> = report_set.buffers.iter().map(|b| b.identical()).collect();
    assert_eq!(identical, [true, false, false, true]);
    assert_eq!(
        (
            report_set.buffers[2].shipped,
            report_set.buffers[2].compiled
        ),
        (1, 2)
    );
}

const NAMESTR: usize = 6;
const DESCSTR: usize = 8;
const FLAGS12: &str = "isSpawn/isMelee/noRatio/SetBoss/BossXfer/boss/primeevil/opendoors";

fn monstats_pair(edit: impl Fn(&mut BinTable, &mut BinTable)) -> CrossCheck {
    let mut shipped = table("monstats", 708);
    let mut ours = table("monstats", 708);
    edit(&mut shipped, &mut ours);
    let c = compiled_table("monstats", &ours, &[]);
    cross(vec![(c, shipped)], same_buffers())
}

/// The one difference the cross-check allows (monstats record 707
/// `NameStr`, shipped 5382, compiled 11154): explained, so the table
/// matches without being identical.
#[test]
fn crosscheck_monstats_707_exception() {
    let r = monstats_pair(|s, o| {
        poke(s, 707, NAMESTR, &5382u16.to_le_bytes());
        poke(o, 707, NAMESTR, &11154u16.to_le_bytes());
    });
    let m = report(&r, "monstats");
    assert_eq!(m.explained, BTreeMap::from([(REASON_NAMESTR_707, (2, 1))]));
    assert!(m.mismatches.is_empty());
    assert!(m.counts_equal() && m.matches() && !m.identical());
    assert_eq!(r.runtime_matching(), 1);
}

/// The exception needs both values: shipped 5382 with any other compiled
/// value is a difference.
#[test]
fn crosscheck_monstats_707_needs_both_values() {
    for (shipped, ours) in [(5382u16, 1234u16), (1234, 11154)] {
        let r = monstats_pair(|s, o| {
            poke(s, 707, NAMESTR, &shipped.to_le_bytes());
            poke(o, 707, NAMESTR, &ours.to_le_bytes());
        });
        let m = report(&r, "monstats");
        assert!(m.explained.is_empty(), "{shipped} {ours}");
        assert_eq!(m.mismatches.keys().collect::<Vec<_>>(), ["NameStr"]);
        assert!(!m.matches());
    }
}

/// Every other differing byte is reported against the field whose
/// footprint holds it (or as an unwritten byte), with its byte and record
/// counts: the exception covers only record 707's two `NameStr` bytes.
#[test]
fn crosscheck_attributes_differences() {
    let r = monstats_pair(|s, o| {
        // The exception, plus another byte of record 707.
        poke(s, 707, NAMESTR, &5382u16.to_le_bytes());
        poke(o, 707, NAMESTR, &11154u16.to_le_bytes());
        poke(o, 707, DESCSTR, &[1]);
        // The exception's values in another record.
        poke(s, 5, NAMESTR, &5382u16.to_le_bytes());
        poke(o, 5, NAMESTR, &11154u16.to_le_bytes());
        // Unwritten bytes and a bit group.
        poke(o, 3, 10, &[1, 2]);
        poke(o, 3, 12, &[1]);
        poke(o, 4, 12, &[4]);
        poke(o, 4, DESCSTR, &[7, 7]);
        // Record 6 identical in content: equal bytes are not differences.
        poke(s, 6, 0, &[9]);
        poke(o, 6, 0, &[9]);
    });
    let m = report(&r, "monstats");
    assert_eq!(m.explained, BTreeMap::from([(REASON_NAMESTR_707, (2, 1))]));
    let got: BTreeMap<&str, (usize, usize)> = m
        .mismatches
        .iter()
        .map(|(k, d)| (k.as_str(), (d.records, d.bytes)))
        .collect();
    let want = BTreeMap::from([
        ("DescStr", (2, 3)),
        ("NameStr", (1, 2)),
        ("unwritten byte +10", (1, 1)),
        ("unwritten byte +11", (1, 1)),
        (FLAGS12, (2, 2)),
    ]);
    assert_eq!(got, want);
    assert!(!m.matches() && !m.identical());
}

// ============================================================== links.rs

/// A `<table>_lookup` name list takes the size of its runtime table's own
/// key when both key the same column.
#[test]
fn name_lookup_list_takes_runtime_key_size() {
    let mut monstats = table("monstats", 3);
    poke(&mut monstats, 1, 0, &[1, 0]);
    poke(&mut monstats, 2, 0, &[2, 0]);
    let sizes = LinkerSizes::from_tables([&monstats]);
    assert_eq!(sizes.get("monstats.Id"), Some(3));
    assert_eq!(sizes.get("monstats_lookup.Id"), Some(3));
}

/// Sizes can be set by hand and listed.
#[test]
fn linker_sizes_insert_and_iter() {
    let mut sizes = LinkerSizes::default();
    sizes.insert("bodylocs.code", 7);
    sizes.insert("@range", 5);
    assert_eq!(sizes.get("bodylocs.code"), Some(7));
    assert_eq!(
        sizes.iter().collect::<Vec<_>>(),
        [("@range", 5), ("bodylocs.code", 7)]
    );
}

/// Values are read at the field's full width: a `link32` value 256 is not
/// index 0 (`v` is read at its width, little-endian).
#[test]
fn link_values_read_at_full_width() {
    let sizes =
        LinkerSizes::from_tables([&table("weapons", 3), &table("armor", 3), &table("misc", 3)]);
    assert_eq!(sizes.get("items.code"), Some(9));
    let mut gems = table("gems", 3);
    poke(&mut gems, 0, 40, &256u32.to_le_bytes());
    poke(&mut gems, 1, 40, &0x0100_0008u32.to_le_bytes());
    poke(&mut gems, 2, 40, &0x00FF_FFFFu32.to_le_bytes());
    let report = validate([&gems], &sizes);
    let broken: Vec<(usize, u32)> = report.broken.iter().map(|b| (b.row, b.value)).collect();
    assert_eq!(broken, [(0, 256), (1, 0x0100_0008), (2, 0x00FF_FFFF)]);
    assert_eq!((report.valid, report.misses), (0, 0));
    assert!(!report.is_clean());

    let clean = validate([&table("gems", 2)], &sizes);
    assert!(clean.is_clean());
    assert_eq!(clean.valid, 2);
}

/// `validate_set` takes sizes from the live tables, `hitclass` and the
/// lookup by-products, and checks the live tables and `hitclass`.
#[test]
fn validate_set_uses_lookups_for_sizes() {
    let mut charstats = table("charstats", 2);
    poke(&mut charstats, 0, 96, &[1]); // item1loc → bodylocs.code: valid
    poke(&mut charstats, 1, 96, &[2]); // broken: 2 bodylocs
    let data = BinSet {
        lod: true,
        rules: Default::default(),
        strings: StringTables::default(),
        tables: vec![charstats],
        code: BTreeMap::new(),
        code_reports: BTreeMap::new(),
        hitclass: table("hitclass", 1),
        sounds: empty_txt(),
        soundenviron: empty_txt(),
    };
    let (sizes, report) = validate_set(&data, &[table("bodylocs", 2)]);
    assert_eq!(sizes.get("bodylocs.code"), Some(2));
    assert_eq!(sizes.get("hitclass.code"), Some(1));
    assert_eq!(report.broken.len(), 1);
    assert_eq!((report.broken[0].row, report.broken[0].value), (1, 2));
}

/// `load_lookups` reads the `.bin` by-products of the compile-only lookup
/// lists and nothing else: not a runtime table's
/// `.bin` (the `_lookup` lists), not `hitclass`; absent files are skipped.
#[test]
fn load_lookups_reads_only_by_products() {
    let dir = TempDir::new("lookups");
    let files = vec![
        (excel_path("bodylocs.bin"), bin_file(3, 4)),
        (excel_path("monai.bin"), bin_file(2, 2)),
        (excel_path("hitclass.bin"), bin_file(5, 4)),
        // A runtime table's file under the lookup's name: never read.
        (excel_path("monmode.bin"), vec![1, 2]),
    ];
    std::fs::write(dir.0.join("patch_d2.mpq"), mpq_bytes(&files)).unwrap();
    let got = load_lookups(&dir.set()).expect("by-products load");
    let names: Vec<(&str, usize)> = got.iter().map(|t| (t.name.as_str(), t.count)).collect();
    assert_eq!(names, [("bodylocs", 3), ("monai", 2)]);
}

// ============================================================= schema.rs

/// Vocabulary words of the unused own-key types.
#[test]
fn vocabulary_of_key_types() {
    let word = |id| FieldType::from_id(id).unwrap().vocabulary();
    assert_eq!(word(12), "key(code1)");
    assert_eq!(word(14), "key(code2)");
    assert_eq!(word(16), "key(str)");
    let width = |id, len| FieldType::from_id(id).unwrap().width(len);
    assert_eq!(width(16, 0), Some(1));
    assert_eq!(width(16, 7), Some(7));
}

/// Lookup fields are exactly IDs 11, 13, 15, 19–21.
#[test]
fn lookup_types_are_exactly_the_link_ids() {
    let lookups: Vec<u32> = (1..=26)
        .filter(|&id| FieldType::from_id(id).unwrap().is_lookup())
        .collect();
    assert_eq!(lookups, [11, 13, 15, 19, 20, 21]);
}

// ============================================================ strings.rs

/// A string table with `key` as element `element` (one hash slot).
fn one_key_table(key: &[u8], element: u16) -> d2_formats::tbl::StringTable {
    use d2_formats::tbl::{key_hash, StringTable, TblEntry, TblHeader};
    StringTable {
        header: TblHeader {
            crc: 0,
            num_elements: element + 1,
            hash_table_size: 1,
            version: 0,
            data_start: 0,
            max_tries: 1,
            file_size: 0,
        },
        indices: vec![0; usize::from(element) + 1],
        entries: vec![TblEntry {
            used: 1,
            index: element,
            hash: key_hash(key),
            key: key.to_vec(),
            value: b"v".to_vec(),
        }],
    }
}

/// A key of `patchstring.tbl` gives element + 10,000.
#[test]
fn patch_string_id_adds_10000() {
    let t = StringTables {
        base: None,
        patch: Some(one_key_table(b"Lilith", 7)),
        expansion: None,
        layer: Default::default(),
    };
    assert_eq!(t.id(b"Lilith"), 10_007);
    assert_eq!(t.strkey(b"Lilith"), 10_007);
}

/// A key of `expansionstring.tbl` (LoD) gives element + 20,000.
#[test]
fn expansion_string_id_adds_20000() {
    let t = StringTables {
        base: None,
        patch: None,
        expansion: Some(one_key_table(b"ob1", 7)),
        layer: Default::default(),
    };
    assert_eq!(t.id(b"ob1"), 20_007);
    assert_eq!(t.strkey(b"ob1"), 20_007);
}

// ============================================================= tables/mod.rs

/// Records are decoded with their own table's layout: a table of another name or record size is refused,
/// even when its record size happens to be the same.
#[test]
fn decode_all_refuses_other_tables() {
    use crate::tables::{decode_all, Objtype, Plrtype};
    assert_eq!(size("objtype"), size("plrtype"));
    assert!(decode_all::<Plrtype>(&table("plrtype", 2)).is_ok());
    assert!(decode_all::<Plrtype>(&table("objtype", 2)).is_err());
    let mut wrong_size = table("objtype", 1);
    wrong_size.record_size += 1;
    wrong_size.records.push(0);
    assert!(decode_all::<Objtype>(&wrong_size).is_err());
}

// ================================================================ txt.rs

/// Error lines count CR LF pairs.
#[test]
fn error_lines_count_crlf_pairs() {
    let line = |data: &[u8]| TxtTable::parse("t", data).unwrap_err();
    let e = line(b"a\r\n1\r\n2\r\n\x00");
    assert_eq!((e.code, e.line), (ErrorCode::E9, Some(4)));
    let e = line(b"a\r\n1\r\n2\r\n3\n");
    assert_eq!((e.code, e.line), (ErrorCode::E5, Some(4)));
}
