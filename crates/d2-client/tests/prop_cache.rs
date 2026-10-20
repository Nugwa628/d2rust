//! Property tests on the budgeted asset pool under random
//! frame / get / insert / resolve / budget sequences. Expected results
//! come from a model of the cache: LRU by last frame used, ties by key order;
//! an entry used by the current frame is never evicted; over budget only
//! when the current frame alone exceeds it, logged once per frame; and of
//! `resolve` makes every key of the frame resident, each missing key
//! loaded once and counted as a stall.

use crate::prop_support;

use std::collections::BTreeMap;

use d2_client::assets::cache::{CacheError, CacheEvent, Clock, FrameNo, Pool};
use proptest::prelude::*;

use prop_support::{bounded, config};

/// A clock advancing 3 µs per read.
struct Ticks(u64);

impl Clock for Ticks {
    fn now_micros(&mut self) -> u64 {
        self.0 += 3;
        self.0
    }
}

#[derive(Debug, Clone)]
enum Op {
    /// Advance the frame by this much (0: a refused repeat).
    Begin(u64),
    Get(u8),
    Insert(u8, u64),
    /// Keys of a frame; keys ≥ 200 fail to load.
    Resolve(Vec<u8>),
    Budget(u64),
}

fn bytes() -> impl Strategy<Value = u64> {
    prop_oneof![6 => 0u64..64, 1 => 0u64..(1 << 40)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        2 => prop_oneof![Just(0u64), 1u64..3, 1u64..1000].prop_map(Op::Begin),
        3 => (0u8..24).prop_map(Op::Get),
        3 => (0u8..24, bytes()).prop_map(|(k, b)| Op::Insert(k, b)),
        2 => proptest::collection::vec(prop_oneof![20 => 0u8..24, 1 => 200u8..202], 0..8)
            .prop_map(Op::Resolve),
        1 => prop_oneof![0u64..128, any::<u64>()].prop_map(Op::Budget),
    ]
}

/// Size of a loaded key (deterministic, so the model knows it).
fn load_size(k: u8) -> u64 {
    u64::from(k % 7) * 5 + 1
}

/// The cache model: key → (bytes, last frame used).
struct Model {
    budget: u64,
    frame: FrameNo,
    entries: BTreeMap<u8, (u64, FrameNo)>,
    overrun_logged: Option<FrameNo>,
    events: Vec<CacheEvent>,
    /// An insert or budget change ran the eviction in this op.
    evicted_check: bool,
}

impl Model {
    fn used(&self) -> u64 {
        self.entries.values().map(|e| e.0).sum()
    }

    fn evict(&mut self) {
        self.evicted_check = true;
        while self.used() > self.budget {
            let victim = self
                .entries
                .iter()
                .map(|(k, e)| (e.1, *k))
                .min()
                .filter(|(last, _)| *last < self.frame);
            match victim {
                Some((last, k)) => {
                    let (b, _) = self.entries.remove(&k).unwrap();
                    self.events.push(CacheEvent::Evicted {
                        pool: "p",
                        key: format!("{k:?}"),
                        bytes: b,
                        last_used: last,
                    });
                }
                None => {
                    if self.overrun_logged != Some(self.frame) {
                        self.overrun_logged = Some(self.frame);
                        self.events.push(CacheEvent::Overrun {
                            pool: "p",
                            frame: self.frame,
                            used: self.used(),
                            budget: self.budget,
                        });
                    }
                    return;
                }
            }
        }
    }

    fn insert(&mut self, k: u8, b: u64) -> Result<(), ()> {
        if self.entries.contains_key(&k) {
            return Err(());
        }
        self.entries.insert(k, (b, self.frame));
        self.evict();
        Ok(())
    }

    fn touch(&mut self, k: u8) -> bool {
        match self.entries.get_mut(&k) {
            Some(e) => {
                e.1 = self.frame;
                true
            }
            None => false,
        }
    }
}

fn run(budget: u64, ops: Vec<Op>) {
    let mut pool: Pool<u8, u8> = Pool::new("p", budget);
    let mut m = Model {
        budget,
        frame: 0,
        entries: BTreeMap::new(),
        overrun_logged: None,
        events: Vec::new(),
        evicted_check: false,
    };
    let mut clock = Ticks(0);
    let mut stalls = 0u64;
    for op in ops {
        // Only inserts (direct or by resolve) and budget changes evict;
        // a new frame or a get leaves the bytes as they were.
        m.evicted_check = false;
        let used_before = pool.used();
        match op {
            Op::Begin(d) => {
                let next = m.frame + d;
                let r = pool.begin_frame(next);
                if d == 0 {
                    assert!(matches!(r, Err(CacheError::FrameOrder { .. })));
                } else {
                    r.unwrap();
                    m.frame = next;
                }
            }
            Op::Get(k) => {
                let hit = m.touch(k);
                assert_eq!(pool.get(&k).copied(), hit.then_some(k));
            }
            Op::Insert(k, b) => {
                let want = m.insert(k, b);
                match pool.insert(k, k, b) {
                    Ok(()) => assert!(want.is_ok()),
                    Err(CacheError::Duplicate { .. }) => assert!(want.is_err()),
                    Err(e) => panic!("unexpected insert error {e}"),
                }
            }
            Op::Resolve(keys) => {
                let r = pool.resolve(&keys, &mut clock, |k: &u8| {
                    if *k >= 200 {
                        Err(format!("no file for {k}"))
                    } else {
                        Ok((*k, load_size(*k)))
                    }
                });
                // Model: touch the resident keys, then load each missing
                // key in list order until a failing one.
                let mut seen = Vec::new();
                let mut resident = 0;
                for &k in &keys {
                    if !seen.contains(&k) {
                        seen.push(k);
                        if m.touch(k) {
                            resident += 1;
                        }
                    }
                }
                let mut loaded = Vec::new();
                let mut failed = false;
                for &k in &keys {
                    if m.entries.contains_key(&k) {
                        continue;
                    }
                    if k >= 200 {
                        failed = true;
                        break;
                    }
                    m.insert(k, load_size(k)).unwrap();
                    m.events.push(CacheEvent::Stall {
                        pool: "p",
                        frame: m.frame,
                        key: format!("{k:?}"),
                        micros: 3,
                    });
                    loaded.push(k);
                }
                stalls += loaded.len() as u64;
                match r {
                    Ok(rep) => {
                        assert!(!failed);
                        assert_eq!(rep.frame, m.frame);
                        assert_eq!(rep.resident, resident);
                        let got: Vec<String> = rep.stalls.iter().map(|s| s.0.clone()).collect();
                        let want: Vec<String> = loaded.iter().map(|k| format!("{k:?}")).collect();
                        assert_eq!(got, want);
                        // Every key of the frame is resident after.
                        for k in &keys {
                            assert!(pool.contains(k), "{k} not resident after resolve");
                        }
                    }
                    Err(CacheError::Load { .. }) => assert!(failed),
                    Err(e) => panic!("unexpected resolve error {e}"),
                }
            }
            Op::Budget(b) => {
                m.budget = b;
                m.evict();
                pool.set_budget(b);
            }
        }
        // State and the event log agree with the model.
        assert_eq!(
            pool.keys().copied().collect::<Vec<_>>(),
            m.entries.keys().copied().collect::<Vec<_>>()
        );
        assert_eq!(pool.used(), m.used());
        assert_eq!(pool.frame(), m.frame);
        assert_eq!(pool.drain_events(), std::mem::take(&mut m.events));
        assert_eq!(pool.stalls().count, stalls);
        // After an op that may evict, over budget only when every
        // resident entry is the current frame's.
        if !m.evicted_check {
            assert_eq!(pool.used(), used_before);
        } else if pool.used() > pool.budget() {
            assert!(m.entries.values().all(|e| e.1 == m.frame));
        }
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// Random op sequences: the pool equals the model after
    /// every op (resident keys, bytes used, events in order, stalls).
    #[test]
    fn pool_matches_model(budget in prop_oneof![0u64..64, 0u64..400, any::<u64>()], ops in proptest::collection::vec(op(), 0..60)) {
        bounded(move || run(budget, ops));
    }
}
