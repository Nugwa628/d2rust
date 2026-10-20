//! Builds seed corpora for the fuzz targets from the user's own game files
//! (`D2_GAME_DIR`). Output goes to `fuzz/corpus/<target>/` (gitignored):
//! Blizzard bytes are never committed.
//!
//! Usage: `D2_GAME_DIR=... cargo +nightly run --release --bin mkseeds`
//! (from `fuzz/`). Files over 60000 bytes are skipped (targets run with
//! `-max_len=65536`).

use d2_formats::mpq::{Archive, ArchiveSet};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_FILE: usize = 60_000;
const PER_TARGET: usize = 80;

fn put(out: &Path, target: &str, n: &mut HashMap<String, usize>, bytes: &[u8]) {
    let c = n.entry(target.to_owned()).or_insert(0);
    if *c >= PER_TARGET * 4 || bytes.len() > 65_000 {
        return;
    }
    let dir = out.join(target);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("seed{c:04}")), bytes).unwrap();
    *c += 1;
}

fn main() {
    let game = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let out = PathBuf::from("corpus");
    let set = ArchiveSet::open_dir(&game).expect("open archives");
    let mut n: HashMap<String, usize> = HashMap::new();

    // (extension, target, per-extension cap)
    let by_ext: &[(&str, &str)] = &[
        (".dc6", "dc6"),
        (".dcc", "dcc"),
        (".dt1", "dt1"),
        (".ds1", "ds1"),
        (".cof", "cof"),
        (".tbl", "tbl"),
        (".tbl", "font_tbl"),
        (".dat", "palette"),
        (".pl2", "pl2"),
        (".d2", "animdata"),
        (".txt", "txt"),
    ];
    let mut taken: HashMap<&str, usize> = HashMap::new();
    for a in set.archives() {
        let Ok(Some(names)) = a.listfile() else {
            continue;
        };
        // Smallest files first within an archive: more variety per byte.
        let mut sized: Vec<(usize, String)> = Vec::new();
        for name in names {
            let lower = name.to_ascii_lowercase();
            if by_ext.iter().any(|(e, _)| lower.ends_with(e)) {
                if let Some(i) = a.find(&name) {
                    sized.push((a.block_table()[i].file_size as usize, name));
                }
            }
        }
        sized.sort();
        for (size, name) in sized {
            if size > MAX_FILE {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            for (ext, target) in by_ext {
                if !lower.ends_with(ext) {
                    continue;
                }
                let t = taken.entry(target).or_insert(0);
                if *t >= PER_TARGET {
                    continue;
                }
                let Ok(bytes) = a.read(&name) else {
                    continue;
                };
                *t += 1;
                put(&out, target, &mut n, &bytes);
            }
            let _ = size;
        }
    }

    // .bin: first byte selects the schema table by bin name.
    let defs: Vec<_> = d2_data::schema::schema().runtime().collect();
    for (i, d) in defs.iter().enumerate() {
        let path = format!("data\\global\\excel\\{}", d.bin_name);
        if let Ok(bytes) = set.read(&path) {
            let mut seed = vec![i as u8];
            seed.extend_from_slice(&bytes);
            put(&out, "bin", &mut n, &seed);
            // A one-record truncation exercises the size check from the other side.
            let cut = (4 + d.record_size).min(bytes.len());
            let mut small = vec![i as u8];
            small.extend_from_slice(&1u32.to_le_bytes());
            small.extend_from_slice(&bytes[4.min(bytes.len())..cut]);
            put(&out, "bin", &mut n, &small);
        }
    }

    // Files too large (animdata) or unlisted (pl2) for the listfile sweep.
    // The animdata seed exceeds 64 KiB: run that target with a larger -max_len.
    for (path, target) in [
        ("data\\global\\animdata.d2", "animdata"),
        ("data\\global\\palette\\act1\\pal.pl2", "pl2"),
        ("data\\global\\palette\\act2\\pal.pl2", "pl2"),
        ("data\\global\\palette\\act3\\pal.pl2", "pl2"),
        ("data\\global\\palette\\act4\\pal.pl2", "pl2"),
        ("data\\global\\palette\\act5\\pal.pl2", "pl2"),
        ("data\\global\\palette\\endgame\\pal.pl2", "pl2"),
        ("data\\global\\palette\\loading\\pal.pl2", "pl2"),
    ] {
        match set.read(path) {
            Ok(b) => {
                let dir = out.join(target);
                fs::create_dir_all(&dir).unwrap();
                let c = n.entry(target.to_owned()).or_insert(0);
                fs::write(dir.join(format!("seed{c:04}")), &b).unwrap();
                *c += 1;
            }
            Err(e) => eprintln!("{path}: {e}"),
        }
    }

    // ADPCM: synthetic streams (header, per-channel initial samples, nibbles).
    for ch in [0u8, 1] {
        let mut s = 4096u32.to_le_bytes().to_vec();
        s.push(ch);
        s.extend_from_slice(&[0, 3, 0x10, 0x00, 0x20, 0x00]);
        s.extend((0..200u32).map(|i| (i.wrapping_mul(37) & 0xFF) as u8));
        put(&out, "mpq_adpcm", &mut n, &s);
    }

    // Patch layers: synthetic text (no game bytes).
    for s in [
        &b"d2patch 1\n# c\ntable items\nset axe lvl 1 -> 4\ncheck #1 clb dam@2 0\nadd #4 spr like axe sha:da7b748ddf3353a9\nset #4 spr name Axe -> [Spear x]\ntable recipes\nset B output x -> y\n"[..],
        b"d2stack 1\n\nlayer a.d2patch\n# c\nlayer sub/b-1.d2patch\n",
    ] {
        put(&out, "patch_layer", &mut n, s);
    }

    // MPQ sector seeds: the first sector of live blocks, grouped by codec.
    let mut sector_seeds = 0usize;
    for a in set.archives() {
        sector_seeds += sector_seeds_from(a, &out, &mut n);
    }
    eprintln!("sector seeds: {sector_seeds}");

    // MPQ archive seeds: tiny archives rebuilt by the writer from live
    // files (stored / PKWARE / Huffman), plus mutated copies come from the
    // fuzzer itself.
    archive_seeds(&set, &out, &mut n);

    let mut keys: Vec<_> = n.iter().collect();
    keys.sort();
    for (k, v) in keys {
        eprintln!("{k}: {v}");
    }
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Reads the first sector of each unencrypted compressed block of `a`
/// (sampling up to 3000 blocks) and files it under the codec targets.
fn sector_seeds_from(a: &Archive, out: &Path, n: &mut HashMap<String, usize>) -> usize {
    const COMPRESS: u32 = 0x200;
    const IMPLODE: u32 = 0x100;
    const ENCRYPTED: u32 = 0x1_0000;
    const SINGLE: u32 = 0x100_0000;
    let Ok(mut f) = fs::File::open(a.path()) else {
        return 0;
    };
    let mut per_mask: HashMap<u8, usize> = HashMap::new();
    let mut count = 0;
    let base = a.header().offset;
    let sector = a.sector_size() as usize;
    for b in a.block_table().iter().take(30_000) {
        if b.flags & ENCRYPTED != 0
            || b.flags & (COMPRESS | IMPLODE) == 0
            || b.compressed_size == 0
            || b.compressed_size > 4_000_000
        {
            continue;
        }
        let single = b.flags & SINGLE != 0;
        let mut head = vec![0u8; 8];
        if f.seek(SeekFrom::Start(base + u64::from(b.file_pos)))
            .is_err()
        {
            continue;
        }
        let (start, end, expected) = if single {
            (0usize, b.compressed_size as usize, b.file_size as usize)
        } else {
            if f.read_exact(&mut head).is_err() {
                continue;
            }
            let (Some(o0), Some(o1)) = (le32(&head, 0), le32(&head, 4)) else {
                continue;
            };
            (o0 as usize, o1 as usize, sector.min(b.file_size as usize))
        };
        if end <= start || end > b.compressed_size as usize || end - start > 5000 {
            continue;
        }
        let mut raw = vec![0u8; end];
        if f.seek(SeekFrom::Start(base + u64::from(b.file_pos)))
            .is_err()
            || f.read_exact(&mut raw).is_err()
        {
            continue;
        }
        let s = &raw[start..end];
        let mask = if b.flags & IMPLODE != 0 { 0xFF } else { s[0] };
        let c = per_mask.entry(mask).or_insert(0);
        if *c >= 60 {
            continue;
        }
        *c += 1;
        count += 1;
        let mut seed = b.flags.to_le_bytes().to_vec();
        seed.extend_from_slice(&(expected as u32).to_le_bytes());
        seed.extend_from_slice(s);
        put(out, "mpq_sector", n, &seed);
        let mut with_max = (expected as u32).to_le_bytes().to_vec();
        let payload = if b.flags & IMPLODE != 0 { s } else { &s[1..] };
        with_max.extend_from_slice(payload);
        match mask {
            0xFF | 0x08 => put(out, "mpq_explode", n, &with_max),
            0x01 | 0x09 => put(out, "mpq_huffman", n, &with_max),
            0x40 | 0x80 | 0x41 | 0x81 => {
                let ch = u8::from(mask & 0x80 != 0);
                let mut w = (expected as u32).to_le_bytes().to_vec();
                w.push(ch);
                w.extend_from_slice(payload);
                if mask & 1 == 1 {
                    put(out, "mpq_huffman", n, &with_max);
                } else {
                    put(out, "mpq_adpcm", n, &w);
                }
            }
            _ => {}
        }
    }
    count
}

fn archive_seeds(set: &ArchiveSet, out: &Path, n: &mut HashMap<String, usize>) {
    use d2_formats::mpq::writer::{FileOptions, Method, MpqWriter, Pkware};
    let names = [
        "data\\global\\excel\\armor.txt",
        "data\\global\\excel\\misc.bin",
        "data\\global\\excel\\levels.bin",
        "data\\local\\lng\\eng\\string.tbl",
        "data\\global\\palette\\act1\\pal.dat",
    ];
    let tmp = std::env::temp_dir().join("d2mkseeds.mpq");
    let methods = [
        Method::Stored,
        Method::Implode(Pkware::default()),
        Method::Compress(Pkware {
            ascii: true,
            dict_bits: 4,
        }),
        Method::Huffman {
            table: 0,
            pkware: None,
        },
        Method::Huffman {
            table: 5,
            pkware: Some(Pkware::default()),
        },
    ];
    for name in names {
        let Ok(bytes) = set.read(name) else {
            continue;
        };
        let bytes = &bytes[..bytes.len().min(20_000)];
        for (k, m) in methods.iter().enumerate() {
            for shift in [0u16, 3] {
                let mut w = MpqWriter::new().sector_size_shift(shift).with_listfile();
                w.add(
                    name,
                    bytes,
                    FileOptions {
                        method: *m,
                        encrypted: k == 2,
                        fix_key: k == 2,
                        ..FileOptions::default()
                    },
                );
                if w.write(&tmp).is_ok() {
                    if let Ok(data) = fs::read(&tmp) {
                        put(out, "mpq_archive", n, &data);
                    }
                }
            }
        }
    }
    let _ = fs::remove_file(tmp);
}
