//! Loader gap tests on synthetic installs: the post-load and count checks of
//! [`load`], the order of link targets across load steps, and the treasure class
//! list order. Archives are built in memory, so no game files are needed.

use d2_formats::mpq::ArchiveSet;

use crate::bin::{excel_path, load, tc_count, BinTable, LoadError};
use crate::compile::tc_linker;
use crate::schema::{schema, CalcBuffer, Link};

// ------------------------------------------------------------ helpers

/// Encrypts `plain` so that `crypto::decrypt(_, key)` gives it back
/// (word by word, from the key stream a zero word decrypts to).
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

/// A temporary install directory holding one `patch_d2.mpq`.
struct Install(std::path::PathBuf);

impl Install {
    fn new(tag: &str, files: &[(String, Vec<u8>)]) -> Install {
        let dir =
            std::env::temp_dir().join(format!("d2-data-gaps-loading-{}-{tag}", std::process::id()));
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

/// An empty `.tbl` (21-byte header, no elements).
fn empty_tbl() -> Vec<u8> {
    let mut t = vec![0u8; 21];
    t[9..13].copy_from_slice(&21u32.to_le_bytes());
    t[17..21].copy_from_slice(&21u32.to_le_bytes());
    t
}

/// A `.bin` of `count` zero records of `size` bytes.
fn bin(count: u32, size: usize) -> Vec<u8> {
    let mut d = count.to_le_bytes().to_vec();
    d.resize(4 + count as usize * size, 0);
    d
}

/// A synthetic classic install (no `d2exp.mpq`) whose live set passes
/// the strict size rule and the post-load and count checks: every runtime table
/// at a count its checks accept, the four code buffers (one end-of-expression byte each), `hitclass`,
/// the sound `.txt` files and the string tables.
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

/// `valid_files` with the file at `path` replaced by `bytes`.
fn with(path: &str, bytes: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let mut files = valid_files();
    let f = files.iter_mut().find(|(p, _)| p == path).expect(path);
    f.1 = bytes;
    files
}

fn size(name: &str) -> usize {
    schema().table(name).unwrap().record_size
}

// -------------------------------------------------------------- tests

/// The load applies, per table, the strict size rule, the post-load
/// checks and the count checks: a set that passes them
/// loads, and breaking any one of them fails the load on that table.
#[test]
fn load_applies_post_load_and_count_checks() {
    let ok = Install::new("valid", &valid_files());
    let data = load(&ok.set(), "eng").expect("synthetic set loads");
    assert_eq!(data.tables.len(), schema().runtime().count());
    assert_eq!(data.table("inventory").unwrap().count, 32);

    let fails_on = |tag: &str, files: Vec<(String, Vec<u8>)>, table: &str| {
        let install = Install::new(tag, &files);
        match load(&install.set(), "eng") {
            Err(LoadError::Check { table: t, .. }) => assert_eq!(t, table, "{tag}"),
            other => panic!("{tag}: {other:?}"),
        }
    };
    // Post-load checks: inventory count = 32; belts count ÷ 2 = 7; superuniques hcIdx
    // coverage.
    fails_on(
        "inventory",
        with(&excel_path("inventory.bin"), bin(31, size("inventory"))),
        "inventory",
    );
    fails_on(
        "belts",
        with(&excel_path("belts.bin"), bin(16, size("belts"))),
        "belts",
    );
    let su = size("superuniques");
    let mut b = bin(66, su);
    for i in 0..66 {
        b[4 + i * su + 8] = i as u8;
    }
    b[4 + 5 * su + 8] = 70;
    fails_on(
        "superuniques",
        with(&excel_path("superuniques.bin"), b),
        "superuniques",
    );
    // Count checks: experience 101 and arena 1 exactly; leveldefs = levels.
    fails_on(
        "experience",
        with(&excel_path("experience.bin"), bin(100, size("experience"))),
        "experience",
    );
    fails_on(
        "arena",
        with(&excel_path("arena.bin"), bin(2, size("arena"))),
        "arena",
    );
    fails_on(
        "leveldefs",
        with(&excel_path("leveldefs.bin"), bin(1, size("leveldefs"))),
        "leveldefs",
    );
    // The size rule runs in the same pass: a wrong file length is rejected.
    let mut long = bin(0, size("armor"));
    long.push(0);
    let install = Install::new("size", &with(&excel_path("armor.bin"), long));
    assert!(matches!(
        load(&install.set(), "eng"),
        Err(LoadError::SizeMismatch { file, .. }) if file == excel_path("armor.bin")
    ));
}

/// Checked on the field lists: every link a table's `.txt` columns use names a
/// linker built by an earlier load step (or by the table itself, for its own
/// key), so the called-table order is a valid compile order. Formula links are
/// not in `fields.tsv` and are not covered.
#[test]
fn links_point_to_earlier_steps() {
    let called: Vec<&str> = schema().called().map(|t| t.name.as_str()).collect();
    let pos = |n: &str| called.iter().position(|c| *c == n);
    for (i, def) in schema().called().enumerate() {
        for f in &def.fields {
            let Link::Linker(name) = &f.link else {
                continue;
            };
            let owner = match name.as_str() {
                "@range" => "skills",
                "@treasureclass" => "treasureclassex",
                "items.code" => "weapons",
                n => n.split('.').next().unwrap(),
            };
            let at = pos(owner).unwrap_or_else(|| panic!("{}: {name}", def.name));
            if owner == def.name || (name == "items.code" && at <= i) {
                continue;
            }
            assert!(at < i, "{}.{}: {name}", def.name, f.name());
        }
    }
    // treasureclass.txt is never read: no loader call names it.
    assert!(schema().called().all(|t| t.txt_name != "treasureclass.txt"));
}

/// The treasure class list (names and order only): TC 0 empty, 32 automatic
/// TCs per itemtypes record with byte 0x1D ≠ 0 in record order (code with
/// pad spaces removed + level 3, 6, …, 96), then the treasureclassex rows
/// up to the first empty name. The automatic TCs' item lists (level in
/// (L−3, L]) are covered by the `d2-sim` treasure tests.
#[test]
fn treasure_class_list_order() {
    let itemtypes_size = size("itemtypes");
    let tcx_size = size("treasureclassex");
    let mut it = vec![0u8; itemtypes_size * 3];
    it[..4].copy_from_slice(b"bow ");
    it[0x1D] = 1;
    it[itemtypes_size..itemtypes_size + 4].copy_from_slice(b"none");
    it[2 * itemtypes_size..2 * itemtypes_size + 4].copy_from_slice(b"armo");
    it[2 * itemtypes_size + 0x1D] = 1;
    let mut tcx = vec![0u8; tcx_size * 3];
    tcx[..4].copy_from_slice(b"Gold");
    // Row 1 has an empty name: rows from there on are not TCs.
    tcx[2 * tcx_size..2 * tcx_size + 4].copy_from_slice(b"Late");
    let l = tc_linker(it.chunks_exact(itemtypes_size), tcx.chunks_exact(tcx_size)).unwrap();
    assert_eq!(l.find(b""), Some(0));
    assert_eq!(l.find(b"bow3"), Some(1));
    assert_eq!(l.find(b"bow 3"), None);
    assert_eq!(l.find(b"bow96"), Some(32));
    assert_eq!(l.find(b"none3"), None);
    assert_eq!(l.find(b"armo3"), Some(33));
    assert_eq!(l.find(b"armo96"), Some(64));
    assert_eq!(l.find(b"gold"), Some(65));
    assert_eq!(l.find(b"late"), None);
    assert_eq!(l.len(), 66);
    let t = |name: &str, data: &[u8], size: usize| {
        BinTable::parse(
            name,
            "p",
            name,
            &[&((data.len() / size) as u32).to_le_bytes()[..], data].concat(),
            size,
        )
        .unwrap()
    };
    assert_eq!(
        tc_count(
            &t("itemtypes", &it, itemtypes_size),
            &t("treasureclassex", &tcx, tcx_size)
        ),
        66
    );
}
