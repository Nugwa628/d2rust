//! Round trips of the test-support MPQ writer through the reader: every option
//! combination (compression method, encryption, sector size, single unit) is written to a
//! temporary archive and read back byte for byte. Test-only; the reader is the one
//! D2RUST uses for the player's game archives.

use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;

use super::writer::{implode, FileOptions, Method, MpqWriter, Pkware, WriteError};
use super::{explode, flags, Archive, SectorStats};

/// Writes `w` to a temporary file and opens it.
fn open(w: &MpqWriter) -> Archive {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "d2rust-mpq-writer-{}-{}.mpq",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    w.write(&path).unwrap();
    let a = Archive::open(&path).unwrap();
    #[cfg(unix)]
    let _ = std::fs::remove_file(&path);
    a
}

/// Deterministic noise (no RNG crate): a 32-bit LCG.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as u8
        })
        .collect()
}

/// Tab-separated text with repeats, shaped like an Excel table.
fn text(len: usize) -> Vec<u8> {
    let mut t = Vec::new();
    let mut i = 0u32;
    while t.len() < len {
        t.extend_from_slice(format!("row{i}\t{}\t{}\tcode\t\r\n", i * 7, i % 13).as_bytes());
        i += 1;
    }
    t.truncate(len);
    t
}

fn all_options() -> Vec<FileOptions> {
    let methods = [
        Method::Stored,
        Method::Compress(Pkware::default()),
        Method::Compress(Pkware {
            ascii: true,
            dict_bits: 4,
        }),
        Method::Implode(Pkware {
            ascii: false,
            dict_bits: 5,
        }),
    ];
    let mut out = Vec::new();
    for method in methods {
        for (encrypted, fix_key) in [(false, false), (true, false), (true, true)] {
            for single_unit in [false, true] {
                for sector_crc in [false, true] {
                    if sector_crc && (single_unit || method == Method::Stored) {
                        continue;
                    }
                    out.push(FileOptions {
                        method,
                        encrypted,
                        fix_key,
                        single_unit,
                        sector_crc,
                        locale: 0,
                    });
                }
            }
        }
    }
    out
}

/// Every option combination, on sizes around the sector boundary, reads
/// back byte for byte, with the expected block flags.
#[test]
fn every_layout_round_trips() {
    let sizes = [0usize, 1, 3, 4, 5, 511, 4095, 4096, 4097, 9000];
    let options = all_options();
    let mut w = MpqWriter::new();
    let mut expected = Vec::new();
    for (k, o) in options.iter().enumerate() {
        for (j, &len) in sizes.iter().enumerate() {
            for (v, data) in [text(len), noise(len, (k * 31 + j) as u32)]
                .into_iter()
                .enumerate()
            {
                let name = format!("data\\dir{k}\\file{j}_{v}.txt");
                w.add(&name, data.clone(), *o);
                expected.push((name, data, *o));
            }
        }
    }
    let a = open(&w);
    for (name, data, o) in &expected {
        assert_eq!(&a.read(name).unwrap(), data, "{name} {o:?}");
        let b = a.block_table()[a.find(name).unwrap()];
        assert!(b.has(flags::EXISTS));
        assert_eq!(b.has(flags::ENCRYPTED), o.encrypted, "{name}");
        assert_eq!(b.has(flags::FIX_KEY), o.fix_key, "{name}");
        assert_eq!(b.has(flags::SINGLE_UNIT), o.single_unit, "{name}");
        assert_eq!(b.has(flags::SECTOR_CRC), o.sector_crc, "{name}");
        assert_eq!(
            b.has(flags::IMPLODE),
            matches!(o.method, Method::Implode(_))
        );
        assert_eq!(
            b.has(flags::COMPRESS),
            matches!(o.method, Method::Compress(_))
        );
        assert_eq!(b.file_size as usize, data.len());
    }
}

/// Text compresses: the sectors carry mask 0x08 (COMPRESS) or are
/// imploded, and the block is smaller than the file.
#[test]
fn text_sectors_are_compressed() {
    let data = text(20_000);
    let mut w = MpqWriter::new();
    w.add_file("a.txt", data.clone());
    w.add(
        "b.txt",
        data.clone(),
        FileOptions {
            method: Method::Implode(Pkware::default()),
            ..FileOptions::encrypted_fix_key()
        },
    );
    let a = open(&w);
    for name in ["a.txt", "b.txt"] {
        let i = a.find(name).unwrap();
        let b = a.block_table()[i];
        assert!(b.compressed_size < b.file_size / 2, "{name}: {b:?}");
        let mut stats = SectorStats::default();
        let key = a.file_key(name, i);
        assert_eq!(a.read_block_stats(i, key, &mut stats).unwrap(), data);
        let compressed = stats.masks[0x08] + stats.imploded;
        assert_eq!(compressed, 5, "{name}: every 4096-byte sector shrinks");
        assert_eq!(stats.masks[0], 0);
    }
}

/// Noise does not shrink: its sectors are stored raw under COMPRESS, each
/// exactly as long as its decoded size.
#[test]
fn incompressible_sectors_are_stored() {
    let data = noise(5000, 7);
    let mut w = MpqWriter::new();
    w.add_file("n.bin", data.clone());
    let a = open(&w);
    let mut stats = SectorStats::default();
    assert_eq!(a.read_block_stats(0, None, &mut stats).unwrap(), data);
    assert_eq!(stats.masks[0], 2);
}

/// The key of an encrypted block is recovered without its name and equals
/// the FIX_KEY key the writer used.
#[test]
fn key_recovery_finds_the_written_key() {
    let mut w = MpqWriter::new();
    w.add(
        "speech\\x.wav",
        text(9000),
        FileOptions::encrypted_fix_key(),
    );
    let a = open(&w);
    let i = a.find("speech\\x.wav").unwrap();
    assert_eq!(a.recover_key(i).unwrap(), a.file_key("speech\\x.wav", i));
}

/// Probe chains, deleted entries, locales and case/separator folding.
#[test]
fn hash_lookup_rules() {
    let mut w = MpqWriter::new().hash_table_count(8);
    w.add_deleted("data\\Gone.txt");
    w.add(
        "data\\x.txt",
        b"german".to_vec(),
        FileOptions {
            locale: 0x407,
            ..FileOptions::stored()
        },
    );
    w.add("data\\x.txt", b"neutral".to_vec(), FileOptions::stored());
    w.add(
        "only\\local.txt",
        b"french".to_vec(),
        FileOptions {
            locale: 0x40C,
            ..FileOptions::stored()
        },
    );
    for i in 0..3 {
        w.add_file(&format!("f{i}"), vec![i as u8; 10]);
    }
    let a = open(&w);
    assert_eq!(a.header().hash_table_count, 8);
    assert_eq!(a.read("DATA/X.TXT").unwrap(), b"neutral");
    // No locale-0 entry: not found.
    assert!(!a.contains("only/LOCAL.txt"));
    assert!(!a.contains("data\\gone.txt"));
    assert_eq!(a.hash_table().iter().filter(|e| e.is_deleted()).count(), 1);
    for i in 0..3 {
        assert_eq!(a.read(&format!("F{i}")).unwrap(), vec![i as u8; 10]);
    }
}

#[test]
fn full_or_bad_hash_tables_are_errors() {
    let mut w = MpqWriter::new().hash_table_count(2);
    for i in 0..3 {
        w.add_file(&format!("f{i}"), vec![1]);
    }
    assert_eq!(w.to_bytes(), Err(WriteError::HashTableFull(2)));
    let w = MpqWriter::new().hash_table_count(12);
    assert_eq!(w.to_bytes(), Err(WriteError::BadHashCount(12)));
}

#[test]
fn listfile_names_every_file() {
    let mut w = MpqWriter::new().with_listfile();
    w.add_file("data\\a.txt", b"a".to_vec());
    w.add_file("data\\b.txt", b"b".to_vec());
    let a = open(&w);
    assert_eq!(
        a.listfile().unwrap().unwrap(),
        ["data\\a.txt", "data\\b.txt"]
    );
}

/// Other sector sizes (the header's `sector_size_shift`).
#[test]
fn sector_sizes() {
    for shift in [0u16, 1, 5] {
        let mut w = MpqWriter::new().sector_size_shift(shift);
        let data = text(70_000);
        w.add("t", data.clone(), FileOptions::encrypted_fix_key());
        let a = open(&w);
        assert_eq!(a.sector_size(), 0x200 << shift);
        assert_eq!(a.read("t").unwrap(), data);
    }
}

/// The reader notices a change to the written bytes: a flipped
/// sector byte in an encrypted compressed file no longer reads back.
#[test]
fn a_flipped_byte_is_detected() {
    let data = text(6000);
    let mut w = MpqWriter::new();
    w.add("t", data.clone(), FileOptions::encrypted_fix_key());
    let mut bytes = w.to_bytes().unwrap();
    let path = std::env::temp_dir().join(format!("d2rust-mpq-flip-{}.mpq", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(Archive::open(&path).unwrap().read("t").unwrap(), data);
    bytes[0x20 + 40] ^= 0x10;
    std::fs::write(&path, &bytes).unwrap();
    let got = Archive::open(&path).unwrap().read("t");
    let _ = std::fs::remove_file(&path);
    assert!(got.map_or(true, |g| g != data));
}

/// The ASCII literal codes and every dictionary size explode back.
#[test]
fn implode_modes() {
    let data = text(3000);
    for ascii in [false, true] {
        for dict_bits in 4..=6 {
            let p = Pkware { ascii, dict_bits };
            let c = implode(&data, p);
            assert_eq!(c[0], u8::from(ascii));
            assert_eq!(c[1], dict_bits);
            assert_eq!(explode::explode(&c, data.len()).unwrap(), data, "{p:?}");
        }
    }
}

/// Long runs use the longest copy (518) and the end marker's 8 extra bits.
#[test]
fn implode_long_runs() {
    let data = vec![b'x'; 5000];
    let c = implode(&data, Pkware::default());
    assert!(c.len() < 40, "{}", c.len());
    assert_eq!(explode::explode(&c, data.len()).unwrap(), data);
}

proptest! {
    #[test]
    fn implode_explode_round_trip(
        data in proptest::collection::vec(prop_oneof![Just(b'a'), Just(b'b'), any::<u8>()], 0..3000),
        ascii in any::<bool>(),
        dict_bits in 4u8..=6,
    ) {
        let c = implode(&data, Pkware { ascii, dict_bits });
        // explode wants more than 4 input bytes; an empty stream is 4.
        if data.is_empty() {
            prop_assert!(c.len() <= 5);
        } else {
            prop_assert_eq!(explode::explode(&c, data.len()).unwrap(), data);
        }
    }

    #[test]
    fn archive_round_trip(
        data in proptest::collection::vec(any::<u8>(), 0..10_000),
        k in 0usize..30,
    ) {
        let o = all_options()[k % all_options().len()];
        let mut w = MpqWriter::new();
        w.add("p\\q.bin", data.clone(), o);
        prop_assert_eq!(open(&w).read("p\\q.bin").unwrap(), data);
    }
}
