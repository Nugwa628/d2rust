//! Mutation-testing kills (METHODS M08) for `bin`, `calc` and codegen: tests that fail on
//! mutants `cargo mutants` reported as missed. They build small synthetic installs and
//! tables, so they need no game files.

use d2_formats::mpq::ArchiveSet;
use d2_formats::tbl::{key_hash, StringTable, TblEntry, TblHeader};

use crate::bin::{
    buffer_tables, excel_path, formula_fields, item_code_map, load, post_load_check, BinTable,
    LoadError,
};
use crate::calc::{compile, eval_const, validate_buffer, CalcLinks, Family};
use crate::codegen::generate;
use crate::schema::{schema, CalcBuffer, Schema};
use crate::strings::StringTables;

// ------------------------------------------------------------ helpers

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

/// A table with `count` records and no record bytes: enough for the
/// count checks, which read only the count.
fn counted(name: &str, count: usize) -> BinTable {
    BinTable {
        name: name.to_owned(),
        source: "p".to_owned(),
        count,
        record_size: 1,
        records: Vec::new(),
    }
}

/// A table of `count` zero records of `size` bytes.
fn zeroed(name: &str, count: usize, size: usize) -> BinTable {
    BinTable {
        name: name.to_owned(),
        source: "p".to_owned(),
        count,
        record_size: size,
        records: vec![0; count * size],
    }
}

fn size(name: &str) -> usize {
    schema().table(name).unwrap().record_size
}

/// String tables whose `patchstring.tbl` holds `keys` as elements 0, 1,
/// … (string ids 10000, 10001, …).
fn patch_strings(keys: &[&str]) -> StringTables {
    let n = 2 * keys.len() + 1;
    let mut entries = vec![
        TblEntry {
            used: 0,
            index: 0,
            hash: 0,
            key: Vec::new(),
            value: Vec::new(),
        };
        n
    ];
    let mut indices = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let mut slot = key_hash(k.as_bytes()) as usize % n;
        while entries[slot].used != 0 {
            slot = (slot + 1) % n;
        }
        entries[slot] = TblEntry {
            used: 1,
            index: i as u16,
            hash: key_hash(k.as_bytes()),
            key: k.as_bytes().to_vec(),
            value: k.as_bytes().to_vec(),
        };
        indices.push(slot as u16);
    }
    let table = StringTable {
        header: TblHeader {
            crc: 0,
            num_elements: keys.len() as u16,
            hash_table_size: n as u32,
            version: 0,
            data_start: 0,
            max_tries: n as u32,
            file_size: 0,
        },
        indices,
        entries,
    };
    StringTables {
        base: None,
        patch: Some(table),
        expansion: None,
        layer: Default::default(),
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
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&3u16.to_le_bytes());
    for v in [hash_pos, block_pos, n as u32, files.len() as u32] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    out[..32].copy_from_slice(&h);
    out
}

/// A temporary install directory holding one `patch_d2.mpq`.
struct Install(std::path::PathBuf);

impl Install {
    fn new(tag: &str, files: &[(String, Vec<u8>)]) -> Install {
        let dir =
            std::env::temp_dir().join(format!("d2-data-mutants-bin-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("patch_d2.mpq"), mpq_bytes(files)).unwrap();
        Install(dir)
    }

    fn set(&self) -> ArchiveSet {
        ArchiveSet::open_dir(&self.0).unwrap()
    }
}

impl Drop for Install {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bin(count: u32, size: usize) -> Vec<u8> {
    let mut d = count.to_le_bytes().to_vec();
    d.resize(4 + count as usize * size, 0);
    d
}

fn empty_tbl() -> Vec<u8> {
    let mut t = vec![0u8; 21];
    t[9..13].copy_from_slice(&21u32.to_le_bytes());
    t[17..21].copy_from_slice(&21u32.to_le_bytes());
    t
}

/// A classic install whose live set passes the load-time checks; each code
/// buffer is one END byte.
fn valid_files() -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    for def in schema().runtime() {
        let count = match def.name.as_str() {
            "inventory" => 32,
            "belts" => 14,
            "difficultylevels" | "armtype" => 3,
            "arena" => 1,
            "composit" => 16,
            "experience" => 101,
            "superuniques" => 66,
            _ => 0,
        };
        let mut b = bin(count, def.record_size);
        if def.name == "superuniques" {
            for i in 0..66 {
                b[4 + i * def.record_size + 8] = i as u8;
            }
        }
        files.push((excel_path(&def.bin_name), b));
    }
    for buffer in CalcBuffer::ALL {
        files.push((excel_path(&format!("{}.bin", buffer.name())), vec![0]));
    }
    files.push((excel_path("hitclass.bin"), bin(0, 4)));
    for txt in ["sounds.txt", "soundenviron.txt"] {
        files.push((excel_path(txt), b"Sound\r\nx\r\n".to_vec()));
    }
    for tbl in ["string.tbl", "patchstring.tbl"] {
        files.push((format!(r"data\local\lng\eng\{tbl}"), empty_tbl()));
    }
    files
}

// ------------------------------------------------------------ loading

/// The tables whose formula columns point into
/// each buffer.
#[test]
fn code_buffers_compiled_from() {
    assert_eq!(buffer_tables(CalcBuffer::MissCode), ["missiles"]);
    assert_eq!(buffer_tables(CalcBuffer::SkillsCode), ["skills"]);
    assert_eq!(buffer_tables(CalcBuffer::SkillDescCode), ["skilldesc"]);
    assert_eq!(
        buffer_tables(CalcBuffer::ItemsCode),
        ["weapons", "armor", "misc"]
    );
}

/// Each code buffer is read right after its
/// table (skilldesccode right after skillscode), before the next table;
/// D2RUST treats a missing code file as a load error. With the buffer
/// missing and the next table broken, the load fails on the buffer.
#[test]
fn code_buffers_load_right_after_their_table() {
    let ok = Install::new("all", &valid_files());
    let data = load(&ok.set(), "eng").expect("synthetic set loads");
    assert_eq!(
        data.code.keys().copied().collect::<Vec<_>>(),
        CalcBuffer::ALL.to_vec()
    );
    for b in CalcBuffer::ALL {
        assert_eq!(data.code[&b].bytes, [0]);
    }
    let cases = [
        (CalcBuffer::MissCode, "states"),
        (CalcBuffer::SkillsCode, "charstats"),
        (CalcBuffer::SkillDescCode, "charstats"),
        (CalcBuffer::ItemsCode, "magicsuffix"),
    ];
    for (buffer, next) in cases {
        let file = excel_path(&format!("{}.bin", buffer.name()));
        let next_file = excel_path(&schema().table(next).unwrap().bin_name);
        let mut files = valid_files();
        files.retain(|(p, _)| *p != file);
        files.iter_mut().find(|(p, _)| *p == next_file).unwrap().1 = vec![1, 0, 0, 0];
        let install = Install::new(buffer.name(), &files);
        match load(&install.set(), "eng") {
            Err(LoadError::Missing { file: f }) => assert_eq!(f, file),
            other => panic!("{}: {other:?}", buffer.name()),
        }
    }
}

/// The formula fields of a buffer are the
/// u32s at its calc-column offsets in the records of its tables.
#[test]
fn formula_fields_are_the_calc_columns() {
    // Every aligned u32 of record `r` holds r × 100000 + its offset.
    let filled = |name: &str, count: usize| {
        let size = size(name);
        let mut t = zeroed(name, count, size);
        for r in 0..count {
            for o in (0..size - 3).step_by(4) {
                let v = (r * 100_000 + o) as u32;
                t.records[r * size + o..r * size + o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        t
    };
    let sorted = |mut v: Vec<u32>| {
        v.sort_unstable();
        v
    };
    let want = |count: usize, tables: usize, offsets: &[usize]| {
        let mut v = Vec::new();
        for _ in 0..tables {
            for r in 0..count {
                v.extend(offsets.iter().map(|&o| (r * 100_000 + o) as u32));
            }
        }
        sorted(v)
    };
    let tables = vec![
        filled("states", 2),
        filled("missiles", 2),
        filled("weapons", 1),
        filled("armor", 1),
        filled("misc", 1),
    ];
    assert_eq!(
        sorted(formula_fields(CalcBuffer::MissCode, &tables)),
        want(2, 1, &[128, 132, 136, 140, 144, 224, 280])
    );
    assert_eq!(
        sorted(formula_fields(CalcBuffer::ItemsCode, &tables)),
        want(1, 3, &[164, 168, 172, 176, 184])
    );
    // A buffer whose table is not loaded has no fields.
    assert!(formula_fields(CalcBuffer::SkillsCode, &tables).is_empty());
}

/// Table row-count limits not checked by the existing tests.
#[test]
fn count_limits() {
    let s = StringTables::default();
    let ok = |name: &str, n: usize| post_load_check(&counted(name, n), &[], &s, true).is_ok();
    assert!(ok("itemstatcost", 511));
    assert!(!ok("itemstatcost", 512));
    for name in [
        "skills",
        "skilldesc",
        "uniqueitems",
        "sets",
        "setitems",
        "monstats",
        "monequip",
    ] {
        assert!(ok(name, 32_766), "{name}");
        assert!(!ok(name, 32_767), "{name}");
    }
    assert!(ok("levels", 1_023));
    assert!(!ok("levels", 1_024));
    // composit 16 and armtype 3, exactly.
    assert!(ok("composit", 16));
    assert!(!ok("composit", 15));
    assert!(!ok("composit", 17));
    assert!(ok("armtype", 3));
    assert!(!ok("armtype", 2));
    assert!(!ok("armtype", 4));
}

/// Gamble: each row's code (u32 +0x00) is a key of the item code map (`code` +0x80 of
/// weapons, armor, misc).
#[test]
fn gamble_codes_must_be_items() {
    let s = StringTables::default();
    let mut items = Vec::new();
    for (name, code) in [("weapons", b"axe "), ("armor", b"cap "), ("misc", b"elx ")] {
        let mut t = zeroed(name, 1, size(name));
        t.records[0x80..0x84].copy_from_slice(code);
        items.push(t);
    }
    let map = item_code_map(&items);
    for code in [b"axe ", b"cap ", b"elx "] {
        assert!(map.find(u32::from_le_bytes(*code)).is_some());
    }
    assert!(map.find(u32::from_le_bytes(*b"zzz ")).is_none());
    let gamble = |codes: &[&[u8; 4]]| {
        let mut t = zeroed("gamble", codes.len(), size("gamble"));
        for (i, c) in codes.iter().enumerate() {
            let at = i * t.record_size;
            t.records[at..at + 4].copy_from_slice(*c);
        }
        post_load_check(&t, &items, &s, true).is_ok()
    };
    assert!(gamble(&[b"axe ", b"cap ", b"elx "]));
    assert!(!gamble(&[b"axe ", b"zzz "]));
}

/// Treasure classes: the TC count must be ≤ 65,534.
#[test]
fn treasure_class_limit() {
    let s = StringTables::default();
    let itemtypes = zeroed("itemtypes", 0, size("itemtypes"));
    let tcs = |rows: usize| BinTable {
        name: "treasureclassex".to_owned(),
        source: "p".to_owned(),
        count: rows,
        record_size: 1,
        records: vec![b'x'; rows],
    };
    let earlier = std::slice::from_ref(&itemtypes);
    // 1 empty TC + rows.
    assert!(post_load_check(&tcs(65_533), earlier, &s, true).is_ok());
    assert!(post_load_check(&tcs(65_534), earlier, &s, true).is_err());
    assert!(post_load_check(&tcs(70_000), earlier, &s, true).is_err());
}

/// Hireling, only when d2exp exists: Id ≤ 255, NameFirst id ≠ 0,
/// NameLast id > NameFirst id.
#[test]
fn hireling_checks() {
    // Ids: a 10000, b 10001, c 10002.
    let s = patch_strings(&["a", "b", "c"]);
    let row = |id: u32, first: &[u8], last: &[u8]| {
        let size = size("hireling");
        let mut t = zeroed("hireling", 1, size);
        t.records[0x04..0x08].copy_from_slice(&id.to_le_bytes());
        t.records[0xD3..0xD3 + first.len()].copy_from_slice(first);
        t.records[0xF3..0xF3 + last.len()].copy_from_slice(last);
        t
    };
    let ok = |t: &BinTable, lod: bool| post_load_check(t, &[], &s, lod).is_ok();
    assert!(ok(&row(0, b"a", b"b"), true));
    assert!(ok(&row(255, b"b", b"c"), true));
    assert!(!ok(&row(256, b"a", b"b"), true));
    assert!(!ok(&row(1_000, b"a", b"b"), true));
    assert!(!ok(&row(1, b"", b"b"), true));
    assert!(!ok(&row(1, b"zz", b"b"), true));
    assert!(!ok(&row(1, b"b", b"b"), true));
    assert!(!ok(&row(1, b"c", b"a"), true));
    // Classic: not checked.
    assert!(ok(&row(256, b"", b""), false));
    assert!(ok(&row(1, b"c", b"a"), false));
}

// ------------------------------------------------------------------- calc

struct NoLinks;

impl CalcLinks for NoLinks {
    fn skill(&self, _: &[u8]) -> Option<u32> {
        None
    }
    fn missile(&self, _: &[u8]) -> Option<u32> {
        None
    }
    fn stat(&self, _: &[u8]) -> Option<u32> {
        None
    }
    fn skillcalc(&self, _: u32) -> Option<u32> {
        None
    }
    fn misscalc(&self, _: u32) -> Option<u32> {
        None
    }
}

/// Emitting an operator: `count += 1 − arity`, so a binary operator leaves one
/// value, and a later operator without two values fails.
#[test]
fn emit_leaves_one_value() {
    let code = |t: &str| {
        compile(Family::Skills, &NoLinks, t.as_bytes())
            .unwrap()
            .code
    };
    assert_eq!(code("1+2"), hex("07 03 00"));
    for t in ["1+2+", "2*3*", "1+2*3-", "min(1,2)+", "1<2<"] {
        assert!(code(t).is_empty(), "{t}");
    }
}

/// Signed comparisons in opcode order, and CALL with an index ≥ the function count
/// (there are none when folding) pushes 0.
#[test]
fn const_evaluator_comparisons_and_call() {
    let rows: [(u8, [i32; 3]); 6] = [
        (0x0A, [1, 0, 0]),
        (0x0B, [0, 0, 1]),
        (0x0C, [1, 1, 0]),
        (0x0D, [0, 1, 1]),
        (0x0E, [0, 1, 0]),
        (0x0F, [1, 0, 1]),
    ];
    for (op, want) in rows {
        for (pair, w) in ["07 FF 07 01", "07 02 07 02", "07 03 07 02"]
            .iter()
            .zip(want)
        {
            let mut b = hex(pair);
            b.extend([op, 0x00]);
            assert_eq!(eval_const(&b), w, "{pair} {op:02X}");
        }
    }
    assert_eq!(eval_const(&hex("07 05 07 06 01 09 00")), 0);
    // CALL pops nothing: 5, 0, 3 → 0 − 3.
    assert_eq!(eval_const(&hex("07 05 01 00 07 03 11 00")), -3);
}

/// Each opcode 0x02 is reported.
#[test]
fn validator_counts_open_parens() {
    let fire_wall = hex("07 25 07 29 01 03 04 0F 12 07 29 07 29 01 03 04 0E 12 10 02 00");
    let r = validate_buffer(Family::Skills, &fire_wall, [0]).unwrap();
    assert_eq!(r.paren, 1);
    let r = validate_buffer(Family::Skills, &hex("07 01 02 00 02 00"), [0, 4]).unwrap();
    assert_eq!(r.paren, 2);
    let r = validate_buffer(Family::Skills, &hex("07 01 00"), [0]).unwrap();
    assert_eq!(r.paren, 0);
}

// ---------------------------------------------------------------- codegen

/// Types 14 (`key(code2)`, 2 code bytes) and 16 (`key(str(N))`,
/// max(N, 1) bytes: up to max(N, 1) − 1 text bytes and the NUL) get a
/// generated field over their text bytes, like the other own keys.
#[test]
fn codegen_code2_and_key_str() {
    let tables =
        "table\ttxt_name\tbin_name\trecord_size\tload_step\tkey_column\tlive_source\tnotes\n\
                  tk\ttk.txt\ttk.bin\t32\t1\tk2\tpatch_d2/tk.bin\t-\n";
    let fields = "table\tseq\tcolumn\ttype_id\ttype\tlength\toffset\tlink\n\
                  tk\t0\tk2\t14\tkey(code2)\t0\t0\t\n\
                  tk\t1\tks\t16\tkey(str)\t8\t4\t\n\
                  tk\t2\tkz\t16\tkey(str)\t0\t20\t\n";
    let s = Schema::parse(tables, fields).unwrap();
    let out = generate(&s);
    assert!(out.contains("pub k2: [u8; 2],"), "{out}");
    assert!(out.contains("k2: bytes(r, 0),"), "{out}");
    assert!(out.contains("pub ks: [u8; 7],"), "{out}");
    assert!(out.contains("ks: bytes(r, 4),"), "{out}");
    assert!(out.contains("pub kz: [u8; 0],"), "{out}");
}
