//! Performance baselines of `d2-formats` (criterion): MPQ open / read / decompress of a
//! synthetic archive, Huffman sector decompression (`mpq_huffman`), DC6
//! and DCC decode of synthetic frames (the shallow ones below, and
//! live-shaped files from `test_fixtures::sprites`). Not run in CI;
//! `cargo bench -p d2-formats`.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::mpq::writer::{huffman as huffman_stream, FileOptions, Method, MpqWriter, Pkware};
use d2_formats::mpq::Archive;

/// Names and contents of the synthetic archive: a 256 KiB text file
/// PKWARE-compressed in 512-byte sectors and encrypted, a 64 KiB plain
/// file.
fn mpq_files() -> Vec<(&'static str, Vec<u8>)> {
    let words = [
        "strength",
        "dexterity",
        "vitality",
        "energy",
        "life",
        "mana",
    ];
    let mut text = Vec::new();
    let mut i = 0usize;
    while text.len() < 256 * 1024 {
        text.extend_from_slice(words[i % words.len()].as_bytes());
        text.push(if i.is_multiple_of(7) { b'\n' } else { b'\t' });
        i = i.wrapping_mul(31).wrapping_add(17);
    }
    let plain: Vec<u8> = (0..64 * 1024u32).map(|i| (i * 13) as u8).collect();
    vec![
        ("data\\global\\excel\\big.txt", text),
        ("data\\global\\plain.bin", plain),
    ]
}

fn bench_mpq(c: &mut Criterion) {
    let files = mpq_files();
    let mut w = MpqWriter::new().sector_size_shift(0);
    w.add(
        files[0].0,
        files[0].1.clone(),
        FileOptions {
            encrypted: true,
            ..FileOptions::default()
        },
    );
    w.add(files[1].0, files[1].1.clone(), FileOptions::stored());
    let path = std::env::temp_dir().join(format!("d2-formats-bench-{}.mpq", std::process::id()));
    w.write(&path).expect("write archive");
    let archive = Archive::open(&path).expect("open");
    for (name, content) in &files {
        assert_eq!(&archive.read(name).expect("read"), content, "{name}");
    }
    let mut g = c.benchmark_group("mpq");
    g.bench_function("open_archive", |b| {
        b.iter(|| black_box(Archive::open(&path).expect("open")))
    });
    let (big, big_content) = &files[0];
    g.throughput(Throughput::Bytes(big_content.len() as u64));
    g.bench_function("read_256k_pkware_encrypted_sectors", |b| {
        b.iter(|| black_box(archive.read(big).expect("read")))
    });
    let (plain, plain_content) = &files[1];
    g.throughput(Throughput::Bytes(plain_content.len() as u64));
    g.bench_function("read_64k_plain", |b| {
        b.iter(|| black_box(archive.read(plain).expect("read")))
    });
    g.finish();
    let _ = std::fs::remove_file(&path);
}

/// Huffman sectors, 256 KiB in 512-byte sectors: the text above under
/// tables 0–3 (table 0 is the adaptive one) and under table 0 then PKWARE
/// (mask 0x09); small signed deltas (-3..=3, like ADPCM output) under the
/// tables that shrink them. Every case is checked to store compressed
/// sectors, so none is measuring a raw copy.
fn bench_huffman(c: &mut Criterion) {
    let text = mpq_files().swap_remove(0).1;
    let mut s = 0x1234_5678u32;
    let deltas: Vec<u8> = (0..256 * 1024)
        .map(|_| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (((s >> 16) % 7) as i8 - 3) as u8
        })
        .collect();
    let huffman = |table, pkware| Method::Huffman { table, pkware };
    let mut cases: Vec<(String, &[u8], Method)> = (0..4u8)
        .map(|t| (format!("text_table{t}"), &text[..], huffman(t, None)))
        .collect();
    cases.push((
        "text_table0_pkware".into(),
        &text,
        huffman(0, Some(Pkware::default())),
    ));
    for t in [0, 1, 4, 5, 6] {
        cases.push((format!("deltas_table{t}"), &deltas, huffman(t, None)));
    }
    for (name, data, method) in &cases {
        if let Method::Huffman { table, .. } = method {
            let sector = &data[..512];
            assert!(
                huffman_stream(*table, sector).len() < sector.len(),
                "{name}"
            );
        }
    }

    let mut w = MpqWriter::new().sector_size_shift(0);
    for (name, data, method) in &cases {
        let options = FileOptions {
            method: *method,
            ..FileOptions::default()
        };
        w.add(name, data.to_vec(), options);
    }
    let path = std::env::temp_dir().join(format!(
        "d2-formats-bench-huffman-{}.mpq",
        std::process::id()
    ));
    w.write(&path).expect("write archive");
    let archive = Archive::open(&path).expect("open");
    let mut g = c.benchmark_group("mpq_huffman");
    for (name, data, _) in &cases {
        assert_eq!(&archive.read(name).expect("read")[..], *data, "{name}");
        g.throughput(Throughput::Bytes(data.len() as u64));
        g.bench_function(name.as_str(), |b| {
            b.iter(|| black_box(archive.read(name).expect("read")))
        });
    }
    g.finish();
    let _ = std::fs::remove_file(&path);
}

// ---- DC6 ---------------------------------------------------------------------

const DC6_FRAME: u32 = 64;
const DC6_FRAMES: usize = 32;

/// A DC6 of `DC6_FRAMES` 64 × 64 frames: every row skips 4 transparent
/// pixels, then 60 literal pixels (DC6 pixel decoding).
fn dc6_file() -> Vec<u8> {
    let mut rows = Vec::new();
    for _ in 0..DC6_FRAME {
        rows.extend_from_slice(&[0x84, 60]);
        rows.extend((0..60u32).map(|i| 1 + (i % 250) as u8));
        rows.push(0x80);
    }
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&(DC6_FRAMES as u32).to_le_bytes());
    let mut at = d.len() + 4 * DC6_FRAMES;
    let mut body = Vec::new();
    for _ in 0..DC6_FRAMES {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        for v in [0u32, DC6_FRAME, DC6_FRAME, 0, 0, 0, 0, rows.len() as u32] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&rows);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + rows.len() + 3;
    }
    d.extend(body);
    d
}

// ---- DCC ---------------------------------------------------------------------

/// LSB-first bit fields.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    fn put(&mut self, value: u32, n: u32) {
        for i in 0..n {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if value >> i & 1 == 1 {
                *self.bytes.last_mut().expect("byte") |= 1 << (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

const DCC_SIDE: u32 = 32;
const DCC_DIRECTIONS: usize = 8;

/// One direction of one `DCC_SIDE`² bottom-up frame, 32-bit header
/// fields, palette {0}, every first-touch cell code 0.
fn dcc_direction() -> Vec<u8> {
    let mut w = BitWriter::default();
    w.put(0, 32); // outsize coded
    w.put(0, 2); // flags
    for code in [0, 15, 15, 15, 15, 0, 0] {
        w.put(code, 4);
    }
    for v in [DCC_SIDE, DCC_SIDE, 0, 0] {
        w.put(v, 32); // width, height, x, y
    }
    w.put(1, 1); // bottom-up
    w.put(0, 20); // pixel mask stream size
    for i in 0..256u32 {
        w.put(u32::from(i == 0), 1);
    }
    for _ in 0..(DCC_SIDE / 4) * (DCC_SIDE / 4) {
        w.put(0, 4);
    }
    w.bytes
}

fn dcc_file() -> Vec<u8> {
    let dir = dcc_direction();
    let mut file = vec![0x74, 6, DCC_DIRECTIONS as u8];
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    let mut at = file.len() + 4 * DCC_DIRECTIONS;
    for _ in 0..DCC_DIRECTIONS {
        file.extend_from_slice(&(at as u32).to_le_bytes());
        at += dir.len();
    }
    for _ in 0..DCC_DIRECTIONS {
        file.extend_from_slice(&dir);
    }
    file
}

fn bench_sprites(c: &mut Criterion) {
    let dc6 = dc6_file();
    let parsed = Dc6::parse(&dc6).expect("dc6");
    assert_eq!(parsed.frames.len(), DC6_FRAMES);
    let dcc = dcc_file();
    let parsed = Dcc::parse(&dcc).expect("dcc");
    assert_eq!(parsed.directions.len(), DCC_DIRECTIONS);
    let mut g = c.benchmark_group("sprites");
    g.throughput(Throughput::Elements(DC6_FRAMES as u64));
    g.bench_function("dc6_parse_32_frames_64x64", |b| {
        b.iter(|| black_box(Dc6::parse(black_box(&dc6)).expect("dc6")))
    });
    g.throughput(Throughput::Elements(DCC_DIRECTIONS as u64));
    g.bench_function("dcc_parse_8_directions_32x32", |b| {
        b.iter(|| black_box(Dcc::parse(black_box(&dcc)).expect("dcc")))
    });
    g.finish();
}

/// Live-shaped files from `test_fixtures::sprites`:
/// many directions and frames at realistic sizes, every DCC sub-stream
/// in use, per-pixel colours. Throughput in decoded pixels.
fn bench_live_sprites(c: &mut Criterion) {
    use test_fixtures::sprites::{
        dc6_file, dc6_frames, dcc_file, Dc6Shape, DccShape, DC6_PANEL, DC6_SPRITE, DCC_LARGE,
        DCC_MONSTER,
    };
    let mut g = c.benchmark_group("sprites_live");
    g.sample_size(30);
    let dc6 = |name: &str, s: Dc6Shape| {
        let frames = dc6_frames(s, 7);
        let px: u64 = frames.iter().map(|f| u64::from(f.width * f.height)).sum();
        let file = dc6_file(&frames, s.directions, s.frames);
        println!("{name}: {} bytes, {px} pixels", file.len());
        (file, px)
    };
    for (name, s) in [
        ("dc6_panel_1x4_256", DC6_PANEL),
        ("dc6_sprite_8x16_96", DC6_SPRITE),
    ] {
        let (file, px) = dc6(name, s);
        g.throughput(Throughput::Elements(px));
        g.bench_function(name, |b| {
            b.iter(|| black_box(Dc6::parse(black_box(&file)).expect("dc6")))
        });
    }
    let dcc = |name: &str, s: DccShape| {
        let out = dcc_file(s, 11);
        let px: u64 = out
            .frames
            .iter()
            .flatten()
            .map(|f| u64::from(f.width * f.height))
            .sum();
        println!("{name}: {} bytes, {px} pixels", out.file.len());
        (out.file, px)
    };
    for (name, s) in [
        ("dcc_monster_8x8_70x100", DCC_MONSTER),
        ("dcc_large_16x24_110x130", DCC_LARGE),
    ] {
        let (file, px) = dcc(name, s);
        g.throughput(Throughput::Elements(px));
        g.bench_function(name, |b| {
            b.iter(|| black_box(Dcc::parse(black_box(&file)).expect("dcc")))
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_mpq,
    bench_huffman,
    bench_sprites,
    bench_live_sprites
);
criterion_main!(benches);
