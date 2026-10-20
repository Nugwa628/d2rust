//! `AnimData.d2`: frames, speed and event bytes per COF name, in 256
//! hash buckets, with a default record for names not in the file.
//!
//! [`AnimData::parse`] takes the bytes of `AnimData.d2` from the player's game data.
//! Animation code in D2RUST looks up a COF name here for its frame count, speed and
//! frame events.

use crate::cursor::{invalid, Cursor, FormatError};

#[cfg(test)]
mod gaps_numbered_tests;

const FORMAT: &str = "animdata";

/// Archive path of the file.
pub const PATH: &str = "data\\global\\AnimData.d2";
/// Number of hash buckets.
pub const BUCKETS: usize = 256;
/// Size of one record in bytes.
pub const RECORD_SIZE: usize = 160;
/// Event bytes per record.
pub const EVENTS: usize = 144;
/// Frame count of the default record.
pub const DEFAULT_FRAMES: u32 = 2048;
/// Speed of the default record.
pub const DEFAULT_SPEED: u32 = 256;

/// One 160-byte record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimRecord {
    /// Name, NUL-padded to 8 bytes.
    pub name: [u8; 8],
    /// Number of frames in the animation.
    pub frames: u32,
    /// Playback speed word.
    pub speed: u32,
    /// One event byte per frame, up to 144 frames.
    pub events: [u8; EVENTS],
}

/// The default record: name all zero, 2048 frames, speed 256, no
/// events.
pub const DEFAULT_RECORD: AnimRecord = AnimRecord {
    name: [0; 8],
    frames: DEFAULT_FRAMES,
    speed: DEFAULT_SPEED,
    events: [0; EVENTS],
};

/// Result of [`AnimData::info`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimInfo {
    /// Frame count of the record, or of the default record.
    pub frames: u32,
    /// Speed of the record, or of the default record.
    pub speed: u32,
    /// Index of the first frame with a non-zero event byte, or the limit if none.
    pub first_event: u32,
    /// True if the name was found in the file.
    pub found: bool,
}

/// The loaded table: 256 buckets of records in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimData {
    /// The 256 buckets, indexed by [`hash`] of the name.
    pub buckets: Vec<Vec<AnimRecord>>,
}

/// Uppercases `a`..`z` (first step of a lookup); bytes ≥ 0x80 stay.
pub fn uppercase(name: &mut [u8]) {
    for b in name.iter_mut() {
        if b.is_ascii_lowercase() {
            *b -= 0x20;
        }
    }
}

/// Bucket of a name (second step of a lookup): sum of its bytes up to the first NUL,
/// mod 256. No uppercasing here.
pub fn hash(name: &[u8]) -> usize {
    name.iter()
        .take_while(|&&b| b != 0)
        .fold(0u32, |s, &b| s.wrapping_add(u32::from(b))) as usize
        % BUCKETS
}

impl AnimData {
    /// Parses the file with these checks: non-negative
    /// counts, blocks ending exactly at the end of the file, a NUL in
    /// every record name.
    pub fn parse(data: &[u8]) -> Result<AnimData, FormatError> {
        let mut c = Cursor::new(data, FORMAT);
        let mut buckets = Vec::with_capacity(BUCKETS);
        for b in 0..BUCKETS {
            let count = c.i32()?;
            if count < 0 {
                return Err(invalid(FORMAT, format!("bucket {b}: count {count}")));
            }
            // Checked against the bytes left before allocating.
            let fits = (count as usize)
                .checked_mul(RECORD_SIZE)
                .is_some_and(|n| n <= data.len() - c.pos());
            if !fits {
                return Err(invalid(
                    FORMAT,
                    format!("bucket {b}: {count} records cannot fit in the file"),
                ));
            }
            let mut records = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let at = c.pos();
                let raw = c.bytes(RECORD_SIZE)?;
                let mut name = [0u8; 8];
                name.copy_from_slice(&raw[..8]);
                if !name.contains(&0) {
                    return Err(invalid(
                        FORMAT,
                        format!("record at {at:#x}: name has no NUL"),
                    ));
                }
                let mut events = [0u8; EVENTS];
                events.copy_from_slice(&raw[0x10..]);
                records.push(AnimRecord {
                    name,
                    frames: u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
                    speed: u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]),
                    events,
                });
            }
            buckets.push(records);
        }
        if c.pos() != data.len() {
            return Err(invalid(
                FORMAT,
                format!(
                    "blocks end at {:#x}, file has {:#x} bytes",
                    c.pos(),
                    data.len()
                ),
            ));
        }
        Ok(AnimData { buckets })
    }

    /// Name lookup: uppercases, hashes, and returns the first record
    /// whose 8 name bytes equal the NUL-padded query. A query longer than
    /// 8 characters is an error when its bucket is non-empty (the lookup
    /// reports an error). `name` ends at its first NUL or its end.
    pub fn find(&self, name: &[u8]) -> Result<Option<&AnimRecord>, FormatError> {
        let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        let mut key = name[..len].to_vec();
        uppercase(&mut key);
        let bucket = &self.buckets[hash(&key)];
        if bucket.is_empty() {
            return Ok(None);
        }
        if key.len() > 8 {
            return Err(invalid(
                FORMAT,
                format!("query {:?} longer than 8", String::from_utf8_lossy(&key)),
            ));
        }
        let mut padded = [0u8; 8];
        padded[..key.len()].copy_from_slice(&key);
        Ok(bucket.iter().find(|r| r.name == padded))
    }

    /// The record for a composed name, or the default record.
    pub fn record(&self, name: &[u8]) -> Result<&AnimRecord, FormatError> {
        Ok(self.find(name)?.unwrap_or(&DEFAULT_RECORD))
    }

    /// Frame count, speed and first event frame for a name (defaults if not found).
    pub fn info(&self, name: &[u8]) -> Result<AnimInfo, FormatError> {
        Ok(match self.find(name)? {
            Some(r) => {
                let limit = r.frames.min(EVENTS as u32);
                let first_event = (0..limit)
                    .find(|&i| r.events[i as usize] != 0)
                    .unwrap_or(limit);
                AnimInfo {
                    frames: r.frames,
                    speed: r.speed,
                    first_event,
                    found: true,
                }
            }
            None => AnimInfo {
                frames: DEFAULT_FRAMES,
                speed: DEFAULT_SPEED,
                first_event: DEFAULT_FRAMES,
                found: false,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (name, frames, speed, events as (frame, value)).
    type Rec<'a> = (&'a [u8], u32, u32, &'a [(usize, u8)]);

    /// A file with the given records in their hash buckets.
    fn file(records: &[Rec]) -> Vec<u8> {
        let mut buckets: Vec<Vec<u8>> = vec![Vec::new(); BUCKETS];
        let mut counts = [0i32; BUCKETS];
        for (name, frames, speed, events) in records {
            let b = hash(name);
            counts[b] += 1;
            let mut r = vec![0u8; RECORD_SIZE];
            r[..name.len()].copy_from_slice(name);
            r[8..12].copy_from_slice(&frames.to_le_bytes());
            r[12..16].copy_from_slice(&speed.to_le_bytes());
            for &(i, v) in *events {
                r[0x10 + i] = v;
            }
            buckets[b].extend(r);
        }
        let mut out = Vec::new();
        for b in 0..BUCKETS {
            out.extend(counts[b].to_le_bytes());
            out.extend(&buckets[b]);
        }
        out
    }

    #[test]
    fn hash_vectors() {
        assert_eq!(hash(b""), 0);
        assert_eq!(hash(b"A"), 65);
        assert_eq!(hash(b"ZZZZZZZ"), 118);
        let mut n = *b"skwl1hs";
        uppercase(&mut n);
        assert_eq!(&n, b"SKWL1HS");
        assert_eq!(hash(&n), 13);
    }

    #[test]
    fn empty_file_gives_default() {
        let a = AnimData::parse(&vec![0; 1024]).unwrap();
        assert_eq!(a.record(b"X").unwrap(), &DEFAULT_RECORD);
        assert_eq!(
            a.info(b"X").unwrap(),
            AnimInfo {
                frames: 2048,
                speed: 256,
                first_event: 2048,
                found: false
            }
        );
    }

    #[test]
    fn first_event_frame() {
        let a = AnimData::parse(&file(&[
            (b"AAWL1HS", 4, 128, &[(2, 2)]),
            (b"BBWL1HS", 3, 64, &[]),
            (b"CCDTHTH", 200, 168, &[]),
        ]))
        .unwrap();
        let i = a.info(b"aawl1hs").unwrap();
        assert_eq!(
            (i.frames, i.speed, i.first_event, i.found),
            (4, 128, 2, true)
        );
        assert_eq!(a.info(b"BBWL1HS").unwrap().first_event, 3);
        assert_eq!(a.info(b"CCDTHTH").unwrap().first_event, 144);
    }

    #[test]
    fn first_duplicate_wins() {
        let a = AnimData::parse(&file(&[
            (b"VMS1HTH", 17, 200, &[]),
            (b"VMS1HTH", 17, 160, &[]),
        ]))
        .unwrap();
        assert_eq!(a.record(b"VMS1HTH").unwrap().speed, 200);
    }

    #[test]
    fn strict_layout() {
        assert!(AnimData::parse(&[0; 1023]).is_err());
        assert!(AnimData::parse(&[0; 1025]).is_err());
        let mut f = vec![0u8; 1024];
        f[..4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(AnimData::parse(&f).is_err());
    }

    #[test]
    fn long_query_in_used_bucket_is_an_error() {
        let a = AnimData::parse(&file(&[(b"A", 1, 1, &[])])).unwrap();
        // "AAAAAAAAA" hashes to 9 × 65 = 585 → 73, an empty bucket.
        assert!(a.find(b"AAAAAAAAA").unwrap().is_none());
        // 9 bytes summing to 65 mod 256: "A" + eight 0x20 → 65 + 256.
        assert!(a.find(b"A        ").is_err());
    }

    #[test]
    fn integers_are_little_endian() {
        let mut f = file(&[(b"A", 0x0102_0304, 0x0A0B_0C0D, &[])]);
        let a = AnimData::parse(&f).unwrap();
        let r = a.record(b"A").unwrap();
        assert_eq!((r.frames, r.speed), (0x0102_0304, 0x0A0B_0C0D));
        // Bucket 65's count is the LE i32 1; as big-endian it would be
        // 0x0100_0000 records and the file would not parse.
        let at = 65 * 4;
        assert_eq!(f[at..at + 4], 1i32.to_le_bytes());
        f[at..at + 4].copy_from_slice(&1i32.to_be_bytes());
        assert!(AnimData::parse(&f).is_err());
    }

    #[test]
    fn fatal_checks_only_in_non_empty_buckets() {
        // Empty bucket: no checks, just "not found".
        let a = AnimData::parse(&file(&[(b"A", 1, 1, &[])])).unwrap();
        assert!(a.find(b"AAAAAAAAA").unwrap().is_none());
        // Non-empty bucket: an over-long query is an error ...
        assert!(a.find(b"A        ").is_err());
        assert!(a.info(b"A        ").is_err());
        // ... and a record name with no NUL in its 8 bytes is
        // reported as an error at load.
        let mut f = file(&[(b"ABCDEFG", 1, 1, &[])]);
        // The only record sits after the counts of buckets 0..=b.
        let b = hash(b"ABCDEFG");
        let name_at = 4 * (b + 1);
        assert_eq!(&f[name_at..name_at + 7], b"ABCDEFG");
        f[name_at + 7] = b'H';
        assert!(AnimData::parse(&f).is_err());
    }

    #[test]
    fn regress_huge_bucket_count() {
        // A count of i32::MAX reserved ~350 GB before reading a record
        // (allocation failure aborts the process).
        let mut f = vec![0u8; 1024];
        f[..4].copy_from_slice(&i32::MAX.to_le_bytes());
        let err = AnimData::parse(&f).unwrap_err();
        assert!(err.to_string().contains("cannot fit"), "{err}");
    }

    #[test]
    fn regress_hash_of_long_name() {
        // 0xFF × 16,843,010 sums past u32::MAX (overflow panic in debug
        // builds); the bucket is the true sum mod 256.
        let n = 0x0101_0102usize;
        let name = vec![0xFFu8; n];
        assert_eq!(hash(&name), (255 * n as u64 % 256) as usize);
        let a = AnimData::parse(&vec![0; 1024]).unwrap();
        assert!(a.find(&name).unwrap().is_none());
    }

    mod robust {
        use super::*;
        use crate::robust::{bounded, bytes, mutated};
        use crate::robust_tests::config;
        use proptest::prelude::*;

        fn valid() -> Vec<u8> {
            file(&[(b"AAWL1HS", 4, 128, &[(2, 2)]), (b"A", 300, 1, &[(143, 1)])])
        }

        #[test]
        fn builder_is_valid() {
            assert!(AnimData::parse(&valid()).is_ok());
        }

        proptest! {
            #![proptest_config(config(32))]

            #[test]
            fn mutated_file(data in mutated(valid()), name in bytes(24)) {
                bounded(move || {
                    if let Ok(a) = AnimData::parse(&data) {
                        let _ = a.info(&name);
                        let _ = a.record(&name);
                        let _ = a.info(b"AAWL1HS");
                    }
                });
            }
        }
    }
}
