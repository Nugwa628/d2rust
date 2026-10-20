//! Extra `AnimData` tests, included from `animdata.rs`.
//!
//! Covers rejection of negative bucket counts when parsing `AnimData.d2`.

use super::*;

/// A file with every bucket empty except `bucket`, whose count is
/// `count` (no record bytes follow).
fn counts_only(bucket: usize, count: i32) -> Vec<u8> {
    let mut f = vec![0u8; 4 * BUCKETS];
    f[4 * bucket..4 * bucket + 4].copy_from_slice(&count.to_le_bytes());
    f
}

#[test]
fn negative_bucket_count_is_rejected() {
    // Counts are signed; zero everywhere is a valid (empty) file.
    assert!(AnimData::parse(&counts_only(0, 0)).is_ok());
    for (bucket, count) in [(0, -1), (5, -1), (255, i32::MIN)] {
        let err = AnimData::parse(&counts_only(bucket, count)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(&format!("bucket {bucket}: count {count}")),
            "{msg}"
        );
    }
}
