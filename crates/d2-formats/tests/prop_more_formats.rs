//! Property tests (METHODS M07) beyond the in-crate parser properties
//! (`robust_tests.rs`, `mpq/robust_tests.rs`): files built from random
//! contents read back what was written (string-table lookups, AnimData
//! name queries, COF draw order), the same files mutated parse or refuse,
//! and the accessors take any caller value (keys, names, indices) without
//! a panic, an overflow or a hang. Also the MPQ name hash, cipher and
//! archive-name helpers on arbitrary input.
//!
//! Default case counts keep `cargo test` fast; set `PROPTEST_CASES` to
//! hunt harder.

use std::sync::mpsc;
use std::time::Duration;

use proptest::prelude::*;
use proptest::test_runner::Config;

use d2_formats::animdata::{self, AnimData, BUCKETS, RECORD_SIZE};
use d2_formats::cof::Cof;
use d2_formats::mpq::crypto::{self, HashType};
use d2_formats::mpq::{priority, PRIORITY};
use d2_formats::tbl::{key_hash, StringTable};

// ----------------------------------------------------------------- support

/// Runs `f` on its own thread; panics if it panics or takes over 20 s.
fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(v) => {
            let _ = handle.join();
            v
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("call did not return within 20 s"),
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(p) => std::panic::resume_unwind(p),
            Ok(()) => unreachable!("sender dropped without sending"),
        },
    }
}

/// `PROPTEST_CASES` from the environment, else `default`.
fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

/// One edit of a valid file: (kind, position, value).
type Edit = (u8, usize, u32);

/// Applies edits: flip a bit, set a byte, set a u32, truncate, insert.
fn mutate(mut data: Vec<u8>, edits: &[Edit]) -> Vec<u8> {
    for &(kind, at, v) in edits {
        let len = data.len();
        match kind % 5 {
            0 if len > 0 => data[at % len] ^= 1 << (v % 8),
            1 if len > 0 => data[at % len] = v as u8,
            2 if len > 0 => {
                let at = at % len;
                for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                    if let Some(s) = data.get_mut(at + i) {
                        *s = b;
                    }
                }
            }
            3 => data.truncate(at % (len + 1)),
            4 => {
                let at = at % (len + 1);
                data.splice(at..at, v.to_le_bytes());
            }
            _ => {}
        }
    }
    data
}

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    let value = prop_oneof![
        Just(0u32),
        Just(1),
        Just(0xFF),
        Just(0xFFFF),
        Just(0x7FFF_FFFF),
        Just(0x8000_0000),
        Just(u32::MAX),
        0u32..0x100,
        any::<u32>(),
    ];
    prop::collection::vec((any::<u8>(), any::<usize>(), value), 1..5)
}

/// Caller values for index arguments: small, at the edges, huge.
fn index() -> impl Strategy<Value = usize> {
    prop_oneof![
        0usize..16,
        Just(255),
        Just(256),
        Just(usize::MAX),
        Just(usize::MAX / 2),
        any::<usize>(),
    ]
}

/// Name-like bytes: letters, digits, NUL, high bytes, up to 20 long.
fn name_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            4 => b'A'..=b'Z',
            2 => b'a'..=b'z',
            1 => b'0'..=b'9',
            1 => Just(0u8),
            1 => Just(b' '),
            1 => 0x80u8..=0xFF,
        ],
        0..20,
    )
}

// --------------------------------------------------------------------- tbl

/// A `.tbl` holding `pairs`: header, element indices, the hash
/// slots (key hash mod size, linear probing), then the strings. Keys are
/// distinct and NUL-free; values NUL-free.
fn build_tbl(pairs: &[(Vec<u8>, Vec<u8>)], extra_slots: usize) -> Vec<u8> {
    let size = pairs.len() + extra_slots;
    let mut slots: Vec<Option<usize>> = vec![None; size];
    let mut indices = Vec::new();
    let mut max_tries = 0;
    for (e, (k, _)) in pairs.iter().enumerate() {
        let mut s = key_hash(k) as usize % size;
        let mut tries = 1;
        while slots[s].is_some() {
            s = (s + 1) % size;
            tries += 1;
        }
        max_tries = max_tries.max(tries);
        slots[s] = Some(e);
        indices.push(s as u16);
    }
    let header_len = 21;
    let data_start = header_len + 2 * pairs.len() + 17 * size;
    let mut strings = Vec::new();
    let mut offsets = Vec::new();
    for (k, v) in pairs {
        let ko = data_start + strings.len();
        strings.extend_from_slice(k);
        strings.push(0);
        let vo = data_start + strings.len();
        strings.extend_from_slice(v);
        strings.push(0);
        offsets.push((ko, vo, v.len() + 1));
    }
    let file_size = data_start + strings.len();
    let mut f = Vec::new();
    f.extend(0u16.to_le_bytes());
    f.extend((pairs.len() as u16).to_le_bytes());
    f.extend((size as u32).to_le_bytes());
    f.push(0);
    f.extend((data_start as u32).to_le_bytes());
    f.extend((max_tries as u32).to_le_bytes());
    f.extend((file_size as u32).to_le_bytes());
    for i in &indices {
        f.extend(i.to_le_bytes());
    }
    for slot in &slots {
        match slot {
            None => f.extend([0u8; 17]),
            Some(e) => {
                let (ko, vo, vl) = offsets[*e];
                f.push(1);
                f.extend((*e as u16).to_le_bytes());
                f.extend(key_hash(&pairs[*e].0).to_le_bytes());
                f.extend((ko as u32).to_le_bytes());
                f.extend((vo as u32).to_le_bytes());
                f.extend((vl as u16).to_le_bytes());
            }
        }
    }
    f.extend(strings);
    assert_eq!(f.len(), file_size);
    f
}

fn tbl_pairs() -> impl Strategy<Value = Vec<(Vec<u8>, Vec<u8>)>> {
    prop::collection::btree_map(
        prop::collection::vec(1u8..=0xFF, 0..12),
        prop::collection::vec(1u8..=0xFF, 0..24),
        0..24,
    )
    .prop_map(|m| m.into_iter().collect())
}

/// Every lookup on a parsed table: by element, by key, by probing.
fn query_tbl(t: &StringTable, keys: &[Vec<u8>], idx: &[usize]) {
    for k in keys {
        let _ = t.find_slot(k);
        let _ = t.get(k);
    }
    for &i in idx {
        let _ = t.element(i);
    }
}

// ---------------------------------------------------------------- animdata

/// An `AnimData.d2` of `records` (name ≤ 7 bytes uppercase, frames,
/// speed, one event byte at `event`), each in its name's bucket.
fn build_animdata(records: &[(Vec<u8>, u32, u32, usize, u8)]) -> Vec<u8> {
    let mut buckets: Vec<Vec<Vec<u8>>> = vec![Vec::new(); BUCKETS];
    for (name, frames, speed, at, ev) in records {
        let mut r = vec![0u8; RECORD_SIZE];
        r[..name.len()].copy_from_slice(name);
        r[8..12].copy_from_slice(&frames.to_le_bytes());
        r[12..16].copy_from_slice(&speed.to_le_bytes());
        r[16 + at % animdata::EVENTS] = *ev;
        buckets[animdata::hash(name)].push(r);
    }
    let mut f = Vec::new();
    for b in &buckets {
        f.extend((b.len() as u32).to_le_bytes());
        for r in b {
            f.extend(r);
        }
    }
    f
}

type AnimRec = (Vec<u8>, u32, u32, usize, u8);

fn anim_records() -> impl Strategy<Value = Vec<AnimRec>> {
    prop::collection::vec(
        (
            prop::collection::vec(b'A'..=b'Z', 1..8),
            any::<u32>(),
            any::<u32>(),
            any::<usize>(),
            any::<u8>(),
        ),
        0..12,
    )
}

fn query_anim(a: &AnimData, names: &[Vec<u8>]) {
    for n in names {
        if let Ok(Some(r)) = a.find(n) {
            assert!(r.name.contains(&0));
        }
        let _ = a.record(n);
        if let Ok(info) = a.info(n) {
            assert!(!info.found || info.first_event <= info.frames.min(animdata::EVENTS as u32));
        }
    }
}

// --------------------------------------------------------------------- cof

/// A COF: 28-byte header, 9 bytes per layer, `frames` event
/// bytes (plus `pad`), then the draw order.
fn build_cof(layers: u8, frames: u8, dirs: u8, pad: usize, order: &[u8]) -> Vec<u8> {
    let mut f = vec![layers, frames, dirs, 20, 0, 0, 0, 0];
    for v in [-10i32, 10, -20, 20] {
        f.extend(v.to_le_bytes());
    }
    f.extend(256u32.to_le_bytes());
    for i in 0..layers {
        f.extend([i % 16, 1, 1, 0, 0]);
        f.extend(*b"hth\0");
    }
    f.extend((0..frames as usize + pad).map(|i| (i % 5) as u8));
    f.extend(order);
    f
}

fn cof_parts() -> impl Strategy<Value = (u8, u8, u8, usize, Vec<u8>)> {
    (0u8..6, 0u8..8, 0u8..5, 0usize..3).prop_flat_map(|(l, f, d, pad)| {
        let n = l as usize * f as usize * d as usize;
        (
            Just(l),
            Just(f),
            Just(d),
            Just(pad),
            prop::collection::vec(0u8..16, n..=n),
        )
    })
}

fn query_cof(c: &Cof, idx: &[usize]) {
    for &d in idx {
        for &f in idx {
            for &s in idx {
                let _ = c.component_at(d, f, s);
            }
        }
    }
}

// ------------------------------------------------------------------ props

proptest! {
    #![proptest_config(config(128))]

    /// Key lookup: every written key reads back its value, and
    /// element `e` is the `e`-th written pair.
    #[test]
    fn tbl_reads_back(pairs in tbl_pairs(), extra in 1usize..8) {
        let file = build_tbl(&pairs, extra);
        let t = StringTable::parse(&file).expect("built table parses");
        for (e, (k, v)) in pairs.iter().enumerate() {
            prop_assert_eq!(t.get(k), Some(v.as_slice()));
            prop_assert_eq!(&t.element(e).expect("element").key, k);
        }
        prop_assert!(t.element(pairs.len()).is_none());
    }

    #[test]
    fn tbl_mutated(
        pairs in tbl_pairs(),
        extra in 0usize..4,
        e in edits(),
        keys in prop::collection::vec(name_bytes(), 0..8),
        idx in prop::collection::vec(index(), 0..6),
    ) {
        let mut pairs = pairs;
        if pairs.is_empty() && extra == 0 {
            pairs.push((b"k".to_vec(), b"v".to_vec()));
        }
        let mut keys = keys;
        keys.extend(pairs.iter().map(|p| p.0.clone()));
        let file = mutate(build_tbl(&pairs, extra), &e);
        bounded(move || {
            if let Ok(t) = StringTable::parse(&file) {
                query_tbl(&t, &keys, &idx);
            }
        });
    }

    /// Each record is found by its name in any case,
    /// with its frames and speed; a missing name gives the default.
    #[test]
    fn animdata_reads_back(records in anim_records()) {
        let file = build_animdata(&records);
        let a = AnimData::parse(&file).expect("built AnimData parses");
        for (name, ..) in &records {
            // The first record of that name in its bucket wins.
            let first = records.iter().find(|r| &r.0 == name).expect("listed");
            let lower: Vec<u8> = name.iter().map(u8::to_ascii_lowercase).collect();
            for q in [name, &lower] {
                let r = a.find(q).expect("name of 8 or fewer").expect("found");
                prop_assert_eq!((r.frames, r.speed), (first.1, first.2));
            }
        }
        let info = a.info(b"").expect("empty name");
        prop_assert_eq!(info.found, records.iter().any(|r| r.0.is_empty()));
    }

    #[test]
    fn animdata_mutated(
        records in anim_records(),
        e in edits(),
        names in prop::collection::vec(name_bytes(), 0..8),
    ) {
        let mut names = names;
        names.extend(records.iter().map(|r| r.0.clone()));
        let file = mutate(build_animdata(&records), &e);
        bounded(move || {
            if let Ok(a) = AnimData::parse(&file) {
                query_anim(&a, &names);
            }
        });
    }

    /// The draw order is `[(d * frames + f) * layers + slot]`.
    #[test]
    fn cof_reads_back((l, f, d, pad, order) in cof_parts()) {
        let file = build_cof(l, f, d, pad, &order);
        let c = Cof::parse(&file).expect("built COF parses");
        prop_assert_eq!(c.events.len(), f as usize);
        prop_assert_eq!(c.event_padding.len(), pad);
        for di in 0..d as usize {
            for fi in 0..f as usize {
                for s in 0..l as usize {
                    let at = (di * f as usize + fi) * l as usize + s;
                    prop_assert_eq!(c.component_at(di, fi, s), Some(order[at]));
                }
            }
        }
        prop_assert_eq!(c.component_at(d as usize, 0, 0), None);
    }

    #[test]
    fn cof_mutated(
        (l, f, d, pad, order) in cof_parts(),
        e in edits(),
        idx in prop::collection::vec(index(), 0..5),
    ) {
        let file = mutate(build_cof(l, f, d, pad, &order), &e);
        bounded(move || {
            if let Ok(c) = Cof::parse(&file) {
                query_cof(&c, &idx);
            }
        });
    }

    /// The Storm cipher: any length, any key; whole dwords are
    /// changed, a tail of 1–3 bytes is left as is.
    #[test]
    fn crypto_any_input(data in prop::collection::vec(any::<u8>(), 0..64), key in any::<u32>()) {
        let mut d = data.clone();
        crypto::decrypt(&mut d, key);
        let whole = data.len() / 4 * 4;
        prop_assert_eq!(&d[whole..], &data[whole..]);
    }

    #[test]
    fn name_helpers_any_input(name in prop::collection::vec(any::<u8>(), 0..300)) {
        for kind in [HashType::TableOffset, HashType::NameA, HashType::NameB, HashType::FileKey] {
            let _ = crypto::hash(&name, kind);
        }
        let s = String::from_utf8_lossy(&name).into_owned();
        prop_assert!(priority(&s) <= PRIORITY.len());
        let _ = animdata::hash(&name);
        let mut up = name.clone();
        animdata::uppercase(&mut up);
        prop_assert!(!up.iter().any(u8::is_ascii_lowercase));
        let _ = key_hash(&name);
    }
}
