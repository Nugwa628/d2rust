//! Public-API checks for `wav` and `animdata` behaviour the unit tests do not cover: the
//! WAV chunk walk (other chunks skipped, RIFF size ignored) and the `AnimData.d2` source
//! path, record size and error on missing data. Uses only `d2_formats`' public items.

use d2_formats::animdata::{AnimData, BUCKETS, PATH, RECORD_SIZE};
use d2_formats::wav::Wav;

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    v
}

fn fmt16() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&22_050u32.to_le_bytes());
    b.extend_from_slice(&44_100u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    chunk(b"fmt ", &b)
}

fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut v = b"RIFF".to_vec();
    // The RIFF size field is not read: store a wrong value.
    v.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(&body);
    v
}

#[test]
fn chunk_walk_skips_other_chunks_and_ignores_riff_size() {
    let f = riff(&[
        fmt16(),
        chunk(b"LIST", &[0xAA; 4]),
        chunk(b"smpl", &[0x55; 8]),
        chunk(b"data", &[1, 0, 0xFF, 0xFF]),
    ]);
    assert_eq!(Wav::parse(&f).unwrap().samples, vec![1, -1]);
}

#[test]
fn animdata_source_path_and_missing_data_is_an_error() {
    assert_eq!(PATH, "data\\global\\AnimData.d2");
    // A missing/empty file is a load error, not a null walk.
    assert!(AnimData::parse(&[]).is_err());
    assert!(AnimData::parse(&vec![0u8; 4 * BUCKETS - 1]).is_err());
    assert_eq!(RECORD_SIZE, 160);
}
