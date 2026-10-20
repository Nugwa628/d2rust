//! The animation table (`animdata.d2`) as `P.tsv`.
//!
//! The table gives every unit animation (named by its COF, such as
//! `AIA1HTH`) its frame count, its speed, and the event on each frame: the
//! frame an attack hits, a missile leaves the hand, a sound plays. The game
//! looks records up through a hash of the name into 256 buckets.
//!
//! The TSV has one row per record, bucket by bucket, in file order. Order
//! matters: some names appear twice, and a lookup returns the first one in
//! its bucket. Events are written as `frame:code` pairs for the frames that
//! have one. [`check_animdata`] proves the TSV rebuilds the same records.

use d2_formats::animdata::{hash, AnimData, AnimRecord, BUCKETS, EVENTS};

use crate::toml_kinds::{
    check_round_trip, latin1, tsv_escape, tsv_rows, tsv_unescape, unlatin1, TextError,
};

const TSV_HEADER: &str = "cof\tframes\tspeed\tevents";

/// Writes `P.tsv`. A record whose name has non-zero bytes after its first
/// NUL, or sits in a bucket other than its name's, cannot be written
/// (the row has no place for either); that is an error, never a silent loss.
pub fn write_animdata(file: &str, a: &AnimData) -> Result<String, TextError> {
    let bad = |d: String| TextError::new(file, format!("not representable: {d}"));
    if a.buckets.len() != BUCKETS {
        return Err(bad(format!("{} buckets", a.buckets.len())));
    }
    let mut o = String::from(TSV_HEADER);
    o.push('\n');
    for (b, bucket) in a.buckets.iter().enumerate() {
        for r in bucket {
            let len = r.name.iter().position(|&c| c == 0).unwrap_or(r.name.len());
            if r.name[len..].iter().any(|&c| c != 0) {
                return Err(bad(format!("bucket {b}: name bytes after the first NUL")));
            }
            if len == r.name.len() {
                return Err(bad(format!("bucket {b}: name has no NUL")));
            }
            if hash(&r.name[..len]) != b {
                return Err(bad(format!(
                    "record {:?} is in bucket {b}, its name hashes to {}",
                    latin1(&r.name[..len]),
                    hash(&r.name[..len])
                )));
            }
            let events: Vec<String> = r
                .events
                .iter()
                .enumerate()
                .filter(|(_, &e)| e != 0)
                .map(|(f, e)| format!("{f}:{e}"))
                .collect();
            o.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                tsv_escape(&latin1(&r.name[..len])),
                r.frames,
                r.speed,
                events.join(" ")
            ));
        }
    }
    Ok(o)
}

/// Reads `P.tsv` back; each record goes to the bucket its name hashes to,
/// in row order.
pub fn read_animdata(file: &str, tsv: &str) -> Result<AnimData, TextError> {
    let mut buckets: Vec<Vec<AnimRecord>> = vec![Vec::new(); BUCKETS];
    for (i, cells) in tsv_rows(file, tsv, TSV_HEADER, 4)?.into_iter().enumerate() {
        let line = i + 2;
        let err = |d: String| TextError::new(file, format!("line {line}: {d}"));
        let name = unlatin1(file, "cof", &tsv_unescape(file, line, cells[0])?)?;
        if name.len() > 7 || name.contains(&0) {
            return Err(err("name is over 7 bytes or holds a NUL".into()));
        }
        let num = |s: &str, what: &str| {
            // One spelling per value: plain decimal, no sign, no leading zero.
            if s.is_empty()
                || (s.len() > 1 && s.starts_with('0'))
                || !s.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(err(format!("{what} {s:?} is not a plain decimal")));
            }
            s.parse::<u32>()
                .map_err(|_| err(format!("{what} {s:?} out of range")))
        };
        let frames = num(cells[1], "frames")?;
        let speed = num(cells[2], "speed")?;
        let mut events = [0u8; EVENTS];
        if !cells[3].is_empty() {
            let mut last = None;
            for pair in cells[3].split(' ') {
                let (f, c) = pair
                    .split_once(':')
                    .ok_or_else(|| err(format!("event {pair:?} is not frame:code")))?;
                let f = num(f, "event frame")? as usize;
                let c = num(c, "event code")?;
                if f >= EVENTS || c == 0 || c > 255 {
                    return Err(err(format!("event {pair:?} out of range")));
                }
                if last.is_some_and(|l| f <= l) {
                    return Err(err("events are not in increasing frame order".into()));
                }
                last = Some(f);
                events[f] = c as u8;
            }
        }
        let mut padded = [0u8; 8];
        padded[..name.len()].copy_from_slice(&name);
        buckets[hash(&name)].push(AnimRecord {
            name: padded,
            frames,
            speed,
            events,
        });
    }
    Ok(AnimData { buckets })
}

/// Round-trip check for animdata: write, read back, compare the records.
pub fn check_animdata(file: &str, original: &AnimData) -> Result<(), TextError> {
    check_round_trip(file, original, write_animdata, read_animdata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toml_kinds::first_difference;

    fn rec(name: &str, frames: u32, speed: u32, events: &[(usize, u8)]) -> AnimRecord {
        let mut n = [0u8; 8];
        n[..name.len()].copy_from_slice(name.as_bytes());
        let mut e = [0u8; EVENTS];
        for &(f, c) in events {
            e[f] = c;
        }
        AnimRecord {
            name: n,
            frames,
            speed,
            events: e,
        }
    }

    fn sample() -> AnimData {
        let mut buckets = vec![Vec::new(); BUCKETS];
        for r in [
            rec("AIA1HTH", 12, 256, &[(3, 1), (143, 4)]),
            rec("AIA1HTH", 99, 100, &[]), // duplicate name, second in its bucket
            rec("A", 1, 2, &[(0, 3)]),
            rec("", 4, 5, &[]),
        ] {
            buckets[hash(&r.name)].push(r);
        }
        AnimData { buckets }
    }

    #[test]
    fn round_trip_keeps_file_order_and_duplicates() {
        let a = sample();
        let t = write_animdata("a.tsv", &a).unwrap();
        assert!(t.starts_with("cof\tframes\tspeed\tevents\n"));
        assert!(t.contains("AIA1HTH\t12\t256\t3:1 143:4\nAIA1HTH\t99\t100\t\n"));
        assert_eq!(read_animdata("a.tsv", &t).unwrap(), a);
        check_animdata("a.tsv", &a).unwrap();
        assert_eq!(t, write_animdata("a.tsv", &a).unwrap());
        // Bucket order: "" (hash 0) first.
        assert!(t.lines().nth(1).unwrap().starts_with("\t4\t5"));
    }

    #[test]
    fn unrepresentable_records_are_errors() {
        let mut a = sample();
        a.buckets[hash(b"A")][0].name = *b"A\0\0\0\0\0\0x";
        assert!(write_animdata("a.tsv", &a).is_err());
        let mut a = sample();
        let r = a.buckets[hash(b"A")].pop().unwrap();
        a.buckets[(hash(b"A") + 1) % BUCKETS].push(r);
        assert!(write_animdata("a.tsv", &a).is_err());
    }

    #[test]
    fn perturbation_and_strict_reader() {
        let a = sample();
        let t = write_animdata("a.tsv", &a).unwrap();
        let p = t.replace("\t12\t", "\t13\t");
        let back = read_animdata("a.tsv", &p).unwrap();
        assert_ne!(back, a);
        assert!(first_difference(&a, &back).contains("12"));
        for bad in [
            t.replace("3:1", "3:0"),
            t.replace("3:1 143:4", "143:4 3:1"),
            t.replace("3:1", "144:1"),
            t.replace("\t12\t", "\t012\t"),
            t.replace("A\t1\t2", "AAAAAAAA\t1\t2"),
            t.replace("cof\t", "name\t"),
            t.trim_end().to_owned(),
        ] {
            assert!(read_animdata("a.tsv", &bad).is_err(), "{bad}");
        }
    }
}
