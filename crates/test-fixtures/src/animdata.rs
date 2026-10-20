//! Writes `AnimData.d2` files that [`d2_formats::animdata::AnimData`] reads.

/// One record: name (≤ 7 bytes, stored uppercase), frames, speed, and the
/// frames that carry an event byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anim {
    pub name: String,
    pub frames: u32,
    pub speed: u32,
    pub events: Vec<(usize, u8)>,
}

/// The 256 bucket blocks, each record in the bucket of its name's byte
/// sum (mod 256), in the given order.
pub fn write(records: &[Anim]) -> Vec<u8> {
    let mut buckets: Vec<Vec<[u8; 160]>> = vec![Vec::new(); 256];
    for a in records {
        let name = a.name.to_ascii_uppercase();
        assert!(name.len() <= 7, "{name}: longer than 7 bytes");
        let mut r = [0u8; 160];
        r[..name.len()].copy_from_slice(name.as_bytes());
        r[8..12].copy_from_slice(&a.frames.to_le_bytes());
        r[12..16].copy_from_slice(&a.speed.to_le_bytes());
        for &(frame, event) in &a.events {
            r[16 + frame] = event;
        }
        let bucket = name.bytes().map(usize::from).sum::<usize>() % 256;
        buckets[bucket].push(r);
    }
    let mut out = Vec::new();
    for b in buckets {
        out.extend_from_slice(&(b.len() as i32).to_le_bytes());
        for r in b {
            out.extend_from_slice(&r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::animdata::AnimData;

    #[test]
    fn round_trip() {
        let bytes = write(&[
            Anim {
                name: "AMNUHTH".into(),
                frames: 20,
                speed: 256,
                events: vec![(9, 1)],
            },
            Anim {
                name: "s1nu1hs".into(),
                frames: 12,
                speed: 128,
                events: vec![],
            },
        ]);
        let a = AnimData::parse(&bytes).unwrap();
        let r = a.find(b"amnuhth").unwrap().unwrap();
        assert_eq!((r.frames, r.speed), (20, 256));
        assert_eq!(a.find(b"S1NU1HS").unwrap().unwrap().speed, 128);
    }
}
