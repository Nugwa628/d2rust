//! Round trips of the Huffman encoder (`huffman::compress`,
//! `writer::Method::Huffman`) through the reader, for every weight table
//! and every compression mask with a Huffman stage, and the decoder held
//! byte for byte to the plain reference model of the Huffman tree (`huffman_model::Model`)
//! on valid and arbitrary streams, so decoder optimisations cannot change
//! an output or an error.
//!
//! Test-only: the encoder lives in the MPQ test-support writer, and the reader being
//! checked is the one D2RUST uses to load the player's game archives.

use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;

use super::huffman::{compress, decompress};
use super::huffman_model::Model;
use super::sector::decompress_masked;
use super::writer::{huffman, FileOptions, Method, MpqWriter, Pkware, WriteError};
use super::{adpcm, compression, Archive, SectorStats};

/// Writes `w` to a temporary file and opens it.
fn open(w: &MpqWriter) -> Archive {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "d2rust-mpq-huffman-{}-{}.mpq",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    w.write(&path).unwrap();
    let a = Archive::open(&path).unwrap();
    #[cfg(unix)]
    let _ = std::fs::remove_file(&path);
    a
}

/// Bytes of three shapes: any value (mostly escapes under the static
/// tables), a small alphabet (deep adaptive trees, long weight blocks),
/// and small signed deltas (ADPCM-like).
fn data(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..max),
        proptest::collection::vec(prop_oneof![Just(b'a'), Just(b'b'), Just(b'\t')], 0..max),
        proptest::collection::vec((-3i8..=3).prop_map(|d| d as u8), 0..max),
    ]
}

fn pkware() -> impl Strategy<Value = Option<Pkware>> {
    prop_oneof![
        Just(None),
        (any::<bool>(), 4u8..=6).prop_map(|(ascii, dict_bits)| Some(Pkware { ascii, dict_bits })),
    ]
}

#[test]
fn every_table_round_trips_fixed_data() {
    let text = b"Stay awhile and listen.\r\n\t0\t1\t2\xff\xfe\x00".repeat(40);
    for t in 0..9u8 {
        for d in [&b""[..], b"a", &text, &text[..17]] {
            let s = compress(t, d);
            assert_eq!(s[0], t, "the first byte names the table");
            assert_eq!(decompress(&s, d.len()).unwrap(), d, "table {t}");
            assert_eq!(decompress(&s, d.len() + 64).unwrap(), d, "end symbol");
            assert_eq!(Model::decode(&s, d.len() + 64, 2).unwrap(), d);
            assert_eq!(huffman(t, d), s, "writer::huffman is the encoder");
        }
    }
}

#[test]
fn writer_uses_the_huffman_masks() {
    let text = b"row\t1\t2\tcode\r\n".repeat(700);
    for (pkware, mask) in [
        (None, compression::HUFFMAN),
        (
            Some(Pkware::default()),
            compression::HUFFMAN | compression::PKWARE,
        ),
    ] {
        let mut w = MpqWriter::new();
        let method = Method::Huffman { table: 0, pkware };
        w.add(
            "t.txt",
            text.clone(),
            FileOptions {
                method,
                ..FileOptions::default()
            },
        );
        let a = open(&w);
        let mut stats = SectorStats::default();
        assert_eq!(a.read_block_stats(0, None, &mut stats).unwrap(), text);
        assert_eq!(stats.masks[usize::from(mask)], 3, "{mask:#x}: every sector");
        assert_eq!(stats.masks.iter().sum::<u64>(), 3);
    }
}

#[test]
fn writer_rejects_bad_huffman_options() {
    let bad = |method| {
        let mut w = MpqWriter::new();
        w.add(
            "x",
            vec![1u8; 100],
            FileOptions {
                method,
                ..FileOptions::default()
            },
        );
        w.to_bytes().unwrap_err()
    };
    let table = |table| Method::Huffman {
        table,
        pkware: None,
    };
    assert_eq!(bad(table(9)), WriteError::BadHuffmanTable(9));
    assert_eq!(bad(table(255)), WriteError::BadHuffmanTable(255));
    let pk = Method::Huffman {
        table: 1,
        pkware: Some(Pkware {
            ascii: false,
            dict_bits: 7,
        }),
    };
    assert_eq!(bad(pk), WriteError::BadDictBits(7));
}

proptest! {
    #[test]
    fn huffman_round_trip(t in 0u8..9, d in data(3000), extra in 0usize..100) {
        let s = compress(t, &d);
        // Capped at the length (end symbol not read) and past it (read).
        prop_assert_eq!(&decompress(&s, d.len()).unwrap(), &d);
        let full = decompress(&s, d.len() + extra).unwrap();
        prop_assert_eq!(&full, &d);
        prop_assert_eq!(Model::decode(&s, d.len() + extra, 2).unwrap(), full);
        // Every shorter cap gives the prefix.
        let cap = extra.min(d.len());
        prop_assert_eq!(&decompress(&s, cap).unwrap()[..], &d[..cap]);
    }

    /// Arbitrary streams, truncated valid streams included: the decoder
    /// gives the model's output or fails where the model fails.
    #[test]
    fn decoder_matches_the_model(
        t in 0u8..9,
        tail in proptest::collection::vec(any::<u8>(), 0..200),
        valid in data(400),
        cut in any::<prop::sample::Index>(),
        max in 0usize..2000,
    ) {
        let mut arbitrary = vec![t];
        arbitrary.extend_from_slice(&tail);
        let s = compress(t, &valid);
        let truncated = &s[..cut.index(s.len())];
        for input in [&arbitrary[..], truncated] {
            prop_assert_eq!(decompress(input, max).map_err(|_| ()), Model::decode(input, max, 2));
        }
    }

    #[test]
    fn archive_huffman_round_trip(
        d in data(9000),
        table in 0u8..9,
        pkware in pkware(),
        shift in 0u16..=3,
        single_unit in any::<bool>(),
        encrypted in any::<bool>(),
        sector_crc in any::<bool>(),
    ) {
        let o = FileOptions {
            method: Method::Huffman { table, pkware },
            encrypted,
            fix_key: encrypted,
            single_unit,
            sector_crc: sector_crc && !single_unit,
            locale: 0,
        };
        let mut w = MpqWriter::new().sector_size_shift(shift);
        w.add("snd\\h.wav", d.clone(), o);
        prop_assert_eq!(open(&w).read("snd\\h.wav").unwrap(), d);
    }

    /// Masks 0x41 and 0x81 (the layout of the game's `.wav` files): the Huffman stage
    /// hands its output to ADPCM unchanged.
    #[test]
    fn huffman_then_adpcm(
        payload in proptest::collection::vec(any::<u8>(), 0..600),
        t in 0u8..9,
        stereo in any::<bool>(),
    ) {
        let (mask, channels) = if stereo {
            (compression::HUFFMAN | compression::ADPCM_STEREO, 2)
        } else {
            (compression::HUFFMAN | compression::ADPCM_MONO, 1)
        };
        let expected = payload.len() * 4 + 8;
        let got = decompress_masked(mask, &compress(t, &payload), expected).unwrap();
        prop_assert_eq!(got, adpcm::decompress(&payload, channels, expected));
    }
}
