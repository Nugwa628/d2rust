//! Residency and budgets: byte-budgeted pools with deterministic LRU eviction. Plain Rust; the
//! render stage owns the frame counter and the Bevy edge logs the events.
//!
//! The client keeps parsed files, frame sets, atlas pages and sounds in separate [`Pool`]s, each
//! with its own byte budget ([`Budgets`]). Eviction never depends on hash order or timing, so the
//! same sequence of uses always keeps the same entries and the drawn image does not depend on
//! what happened to be cached.
//!
//! - Eviction is least recently used by **last frame used**, ties broken
//!   by key order (`BTreeSet<(frame, key)>`; no hash iteration).
//! - An entry used by the current frame is never evicted. If the current
//!   frame alone exceeds the budget, the pool stays over budget, an
//!   [`CacheEvent::Overrun`] is logged once for that frame, and nothing it
//!   uses is dropped.
//! - [`Pool::resolve`] makes every key of a frame resident before compose:
//!   resident keys are marked first (so loading a missing key cannot evict
//!   one this frame still needs), then each missing key is loaded
//!   synchronously and counted as a stall with its duration.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Instant;

/// Render frame number (the render stage's counter, not a sim tick).
pub type FrameNo = u64;

/// One mebibyte, for writing budgets.
pub const MIB: u64 = 1 << 20;

/// Pool budgets in bytes. The defaults are guesses; they have not been measured against real
/// play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budgets {
    /// Parsed files (CPU), counted by [`super::size::ByteSize`].
    pub parsed_files: u64,
    /// FrameSets (CPU): `Σ width × height` + headers.
    pub frame_sets: u64,
    /// Atlas pages (GPU), whole pages (64 pages of 2048² R8).
    pub atlas_pages: u64,
    /// Sounds (CPU): samples × 2 bytes.
    pub sounds: u64,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            parsed_files: 128 * MIB,
            frame_sets: 256 * MIB,
            atlas_pages: 256 * MIB,
            sounds: 64 * MIB,
        }
    }
}

/// Something the Bevy edge logs. Keys are rendered with `Debug`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheEvent {
    /// An entry was dropped to make room.
    Evicted {
        pool: &'static str,
        key: String,
        bytes: u64,
        last_used: FrameNo,
    },
    /// The current frame's entries alone exceed the budget (logged once
    /// per pool and frame).
    Overrun {
        pool: &'static str,
        frame: FrameNo,
        used: u64,
        budget: u64,
    },
    /// A key that was not resident when its frame needed it, loaded
    /// synchronously.
    Stall {
        pool: &'static str,
        frame: FrameNo,
        key: String,
        micros: u64,
    },
}

impl fmt::Display for CacheEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheEvent::Evicted {
                pool,
                key,
                bytes,
                last_used,
            } => write!(
                f,
                "{pool}: evicted {key} ({bytes} bytes, last used frame {last_used})"
            ),
            CacheEvent::Overrun {
                pool,
                frame,
                used,
                budget,
            } => write!(
                f,
                "{pool}: frame {frame} uses {used} bytes, over the {budget}-byte budget"
            ),
            CacheEvent::Stall {
                pool,
                frame,
                key,
                micros,
            } => write!(f, "{pool}: frame {frame} stalled {micros} µs loading {key}"),
        }
    }
}

/// Why a pool operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    /// A frame was started out of order: frames count up by one.
    #[error("{pool}: frame {next} does not follow frame {current}")]
    FrameOrder {
        pool: &'static str,
        current: FrameNo,
        next: FrameNo,
    },
    /// The key is already resident.
    #[error("{pool}: {key} is already resident")]
    Duplicate { pool: &'static str, key: String },
    /// Loading a missing key failed; there is no fallback asset.
    #[error("{pool}: loading {key} failed: {message}")]
    Load {
        pool: &'static str,
        key: String,
        message: String,
    },
}

/// Cumulative counts of synchronous loads (stalls) a pool has had to do, and how long they took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StallMetric {
    /// Number of stalls.
    pub count: u64,
    /// Total stall time in microseconds.
    pub total_micros: u64,
    /// Longest single stall in microseconds.
    pub max_micros: u64,
}

/// What [`Pool::resolve`] did for one frame. Stalls are reported, not treated as failures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameReport {
    /// The frame resolved.
    pub frame: FrameNo,
    /// Distinct keys already resident.
    pub resident: usize,
    /// Keys loaded synchronously, in list order, with their durations.
    pub stalls: Vec<(String, u64)>,
}

/// What [`Pool::offer`] did with a prefetched value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Inserted.
    Taken,
    /// Already resident; the offered value is dropped.
    Resident,
    /// Would exceed the budget; dropped (the frame that needs it loads it).
    NoRoom,
}

/// Time source for stall durations; [`WallClock`] in the client, a fake in
/// tests. Durations are logged only, never used for a decision.
pub trait Clock {
    /// Microseconds on a monotonic scale.
    fn now_micros(&mut self) -> u64;
}

/// Monotonic wall clock, microseconds since creation.
pub struct WallClock(Instant);

impl Default for WallClock {
    fn default() -> Self {
        WallClock(Instant::now())
    }
}

impl Clock for WallClock {
    fn now_micros(&mut self) -> u64 {
        u64::try_from(self.0.elapsed().as_micros()).unwrap_or(u64::MAX)
    }
}

struct Entry<V> {
    value: V,
    bytes: u64,
    last_used: FrameNo,
}

/// One budgeted pool. `K` orders ties; `V` is the resident value.
pub struct Pool<K, V> {
    name: &'static str,
    budget: u64,
    used: u64,
    frame: FrameNo,
    entries: BTreeMap<K, Entry<V>>,
    /// `(last_used, key)`: the first element is the next to evict.
    order: BTreeSet<(FrameNo, K)>,
    overrun_logged: Option<FrameNo>,
    stalls: StallMetric,
    events: Vec<CacheEvent>,
}

impl<K: Ord + Clone + fmt::Debug, V> Pool<K, V> {
    /// An empty pool at frame 0.
    pub fn new(name: &'static str, budget: u64) -> Self {
        Pool {
            name,
            budget,
            used: 0,
            frame: 0,
            entries: BTreeMap::new(),
            order: BTreeSet::new(),
            overrun_logged: None,
            stalls: StallMetric::default(),
            events: Vec::new(),
        }
    }

    /// The pool's name, used in logs and errors.
    pub fn name(&self) -> &'static str {
        self.name
    }
    /// The byte budget.
    pub fn budget(&self) -> u64 {
        self.budget
    }
    /// Bytes charged by resident entries.
    pub fn used(&self) -> u64 {
        self.used
    }
    /// Number of resident entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether nothing is resident.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// The current frame.
    pub fn frame(&self) -> FrameNo {
        self.frame
    }
    /// Stalls so far.
    pub fn stalls(&self) -> StallMetric {
        self.stalls
    }
    /// Resident keys, in key order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.keys()
    }

    /// Starts frame `frame`; it must follow the current one. Entries used
    /// by earlier frames lose their protection. Nothing is evicted here:
    /// eviction happens when an insert needs room.
    pub fn begin_frame(&mut self, frame: FrameNo) -> Result<(), CacheError> {
        if frame <= self.frame {
            return Err(CacheError::FrameOrder {
                pool: self.name,
                current: self.frame,
                next: frame,
            });
        }
        self.frame = frame;
        Ok(())
    }

    /// Changes the budget and evicts down to it (never the current frame).
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.evict_to_budget();
    }

    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// The value without marking it used.
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|e| &e.value)
    }

    /// The value, marked used by the current frame.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if !self.touch(key) {
            return None;
        }
        self.entries.get(key).map(|e| &e.value)
    }

    /// Adds a value of `bytes` bytes, used by the current frame, then
    /// evicts least recently used entries until the pool fits its budget
    /// or only current-frame entries remain. A resident key is an error.
    pub fn insert(&mut self, key: K, value: V, bytes: u64) -> Result<(), CacheError> {
        if self.entries.contains_key(&key) {
            return Err(CacheError::Duplicate {
                pool: self.name,
                key: format!("{key:?}"),
            });
        }
        self.order.insert((self.frame, key.clone()));
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                last_used: self.frame,
            },
        );
        self.used += bytes;
        self.evict_to_budget();
        Ok(())
    }

    /// Makes every key of the current frame resident: marks
    /// the resident ones used, then loads each missing one with `load`
    /// (value and byte count), timing it with `clock`. A failed load is an
    /// error naming the key; there is no fallback asset.
    pub fn resolve<E: fmt::Display>(
        &mut self,
        keys: &[K],
        clock: &mut dyn Clock,
        mut load: impl FnMut(&K) -> Result<(V, u64), E>,
    ) -> Result<FrameReport, CacheError> {
        let mut report = FrameReport {
            frame: self.frame,
            ..FrameReport::default()
        };
        let mut seen = BTreeSet::new();
        for key in keys {
            if seen.insert(key) && self.touch(key) {
                report.resident += 1;
            }
        }
        for key in keys {
            if self.entries.contains_key(key) {
                continue;
            }
            let start = clock.now_micros();
            let (value, bytes) = load(key).map_err(|e| CacheError::Load {
                pool: self.name,
                key: format!("{key:?}"),
                message: e.to_string(),
            })?;
            let micros = clock.now_micros().saturating_sub(start);
            self.insert(key.clone(), value, bytes)?;
            let name = format!("{key:?}");
            self.stalls.count += 1;
            self.stalls.total_micros = self.stalls.total_micros.saturating_add(micros);
            self.stalls.max_micros = self.stalls.max_micros.max(micros);
            self.events.push(CacheEvent::Stall {
                pool: self.name,
                frame: self.frame,
                key: name.clone(),
                micros,
            });
            report.stalls.push((name, micros));
        }
        Ok(report)
    }

    /// Offers a value loaded ahead of need (prefetch). It is
    /// taken only if the key is not resident and it fits the budget as is:
    /// a prefetch never evicts and never overruns, so it changes timing
    /// only. A taken entry counts as last used by the frame before the
    /// current one (it is not protected by the current frame, and is
    /// evicted ahead of anything this frame uses); a frame that lists it
    /// marks it used as usual.
    pub fn offer(&mut self, key: K, value: V, bytes: u64) -> Offer {
        if self.entries.contains_key(&key) {
            return Offer::Resident;
        }
        if self.used.saturating_add(bytes) > self.budget {
            return Offer::NoRoom;
        }
        let last_used = self.frame.saturating_sub(1);
        self.order.insert((last_used, key.clone()));
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                last_used,
            },
        );
        self.used += bytes;
        Offer::Taken
    }

    /// Takes the events logged since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<CacheEvent> {
        std::mem::take(&mut self.events)
    }

    /// Marks `key` used by the current frame; false if not resident.
    fn touch(&mut self, key: &K) -> bool {
        let Some(e) = self.entries.get_mut(key) else {
            return false;
        };
        if e.last_used != self.frame {
            self.order.remove(&(e.last_used, key.clone()));
            e.last_used = self.frame;
            self.order.insert((self.frame, key.clone()));
        }
        true
    }

    fn evict_to_budget(&mut self) {
        while self.used > self.budget {
            match self.order.first() {
                Some((last_used, _)) if *last_used < self.frame => {
                    let (last_used, key) = self.order.pop_first().expect("non-empty");
                    let e = self.entries.remove(&key).expect("order and entries agree");
                    self.used -= e.bytes;
                    self.events.push(CacheEvent::Evicted {
                        pool: self.name,
                        key: format!("{key:?}"),
                        bytes: e.bytes,
                        last_used,
                    });
                }
                _ => {
                    if self.overrun_logged != Some(self.frame) {
                        self.overrun_logged = Some(self.frame);
                        self.events.push(CacheEvent::Overrun {
                            pool: self.name,
                            frame: self.frame,
                            used: self.used,
                            budget: self.budget,
                        });
                    }
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inserts `key` (1 byte) in a new frame.
    fn use_in_frame(p: &mut Pool<&'static str, ()>, frame: FrameNo, key: &'static str) {
        p.begin_frame(frame).unwrap();
        if p.get(&key).is_none() {
            p.insert(key, (), 1).unwrap();
        }
    }

    fn keys(p: &Pool<&'static str, ()>) -> Vec<&'static str> {
        p.keys().copied().collect()
    }

    #[test]
    fn lru_evicts_least_recent() {
        // Budget 2 entries, use a, b, c → a evicted.
        let mut p = Pool::new("t", 2);
        use_in_frame(&mut p, 1, "a");
        use_in_frame(&mut p, 2, "b");
        use_in_frame(&mut p, 3, "c");
        assert_eq!(keys(&p), ["b", "c"]);
        assert_eq!(
            p.drain_events(),
            [CacheEvent::Evicted {
                pool: "t",
                key: "\"a\"".into(),
                bytes: 1,
                last_used: 1
            }]
        );
        assert_eq!(p.used(), 2);
    }

    #[test]
    fn use_refreshes_recency() {
        // Variation of the case above: touching a in frame 3
        // moves the eviction to b.
        let mut p = Pool::new("t", 2);
        use_in_frame(&mut p, 1, "a");
        use_in_frame(&mut p, 2, "b");
        use_in_frame(&mut p, 3, "a");
        use_in_frame(&mut p, 4, "c");
        assert_eq!(keys(&p), ["a", "c"]);
    }

    #[test]
    fn ties_evict_lower_key_first() {
        // Equal last-use frames → lower key first, not
        // insertion order.
        let mut p = Pool::new("t", 2);
        p.begin_frame(1).unwrap();
        p.insert("b", (), 1).unwrap();
        p.insert("a", (), 1).unwrap();
        use_in_frame(&mut p, 2, "c");
        assert_eq!(keys(&p), ["b", "c"]);
    }

    #[test]
    fn current_frame_is_never_evicted() {
        // Entries used this frame, budget exceeded → kept,
        // overrun logged (once for the frame).
        let mut p = Pool::new("t", 2);
        p.begin_frame(1).unwrap();
        for k in ["a", "b", "c", "d"] {
            p.insert(k, (), 1).unwrap();
        }
        assert_eq!(keys(&p), ["a", "b", "c", "d"]);
        assert_eq!(
            p.drain_events(),
            [CacheEvent::Overrun {
                pool: "t",
                frame: 1,
                used: 3,
                budget: 2
            }]
        );
        // Next frame: the old entries are evictable again, oldest key first.
        use_in_frame(&mut p, 2, "e");
        assert_eq!(keys(&p), ["d", "e"]);
        assert_eq!(p.used(), 2);
    }

    #[test]
    fn oversize_single_entry_is_kept_for_its_frame() {
        let mut p: Pool<u32, ()> = Pool::new("t", 10);
        p.begin_frame(1).unwrap();
        p.insert(7, (), 50).unwrap();
        assert!(p.contains(&7));
        assert!(matches!(
            p.drain_events()[..],
            [CacheEvent::Overrun { used: 50, .. }]
        ));
    }

    #[test]
    fn strict_inputs() {
        let mut p: Pool<u32, ()> = Pool::new("t", 10);
        p.begin_frame(3).unwrap();
        assert_eq!(
            p.begin_frame(3),
            Err(CacheError::FrameOrder {
                pool: "t",
                current: 3,
                next: 3
            })
        );
        p.insert(1, (), 1).unwrap();
        assert!(matches!(
            p.insert(1, (), 1),
            Err(CacheError::Duplicate { .. })
        ));
    }

    #[test]
    fn set_budget_evicts_old_frames_only() {
        let mut p = Pool::new("t", 10);
        use_in_frame(&mut p, 1, "a");
        use_in_frame(&mut p, 2, "b");
        p.set_budget(0);
        assert_eq!(keys(&p), ["b"]);
    }

    /// Clock advancing 5 µs per reading.
    struct Tick(u64);
    impl Clock for Tick {
        fn now_micros(&mut self) -> u64 {
            self.0 += 5;
            self.0
        }
    }

    #[test]
    fn resolve_counts_stalls_and_protects_listed_keys() {
        let mut p: Pool<u32, u32> = Pool::new("t", 2);
        let mut clock = Tick(0);
        let load = |k: &u32| Ok::<_, String>((k * 10, 1));
        p.begin_frame(1).unwrap();
        p.insert(1, 10, 1).unwrap();
        p.insert(2, 20, 1).unwrap();

        // Frame 2 lists 3 (missing) before 1 (resident). Without marking
        // first, loading 3 would evict 1 (lowest key of frame 1); with it,
        // 1 is protected and 2 goes.
        p.begin_frame(2).unwrap();
        let r = p.resolve(&[3, 1, 3], &mut clock, load).unwrap();
        assert_eq!(r.frame, 2);
        assert_eq!(r.resident, 1);
        assert_eq!(r.stalls, [("3".to_string(), 5)]);
        assert_eq!(p.keys().copied().collect::<Vec<_>>(), [1, 3]);
        assert_eq!(p.peek(&3), Some(&30));
        assert_eq!(
            p.stalls(),
            StallMetric {
                count: 1,
                total_micros: 5,
                max_micros: 5
            }
        );
        let ev = p.drain_events();
        assert!(ev.contains(&CacheEvent::Stall {
            pool: "t",
            frame: 2,
            key: "3".into(),
            micros: 5
        }));

        // Frame 3: everything resident, no stall.
        p.begin_frame(3).unwrap();
        let r = p.resolve(&[1, 3], &mut clock, load).unwrap();
        assert_eq!((r.resident, r.stalls.len()), (2, 0));
        assert_eq!(p.stalls().count, 1);
    }

    #[test]
    fn resolved_frame_has_every_listed_key_even_over_budget() {
        // Budget for one entry; the frame lists four (one twice). After
        // resolve every listed key is resident: no missing sprite.
        let mut p: Pool<u32, u32> = Pool::new("t", 1);
        let mut clock = Tick(0);
        p.begin_frame(1).unwrap();
        p.insert(9, 90, 1).unwrap();
        p.begin_frame(2).unwrap();
        let keys = [4, 2, 4, 7, 1];
        p.resolve(&keys, &mut clock, |k: &u32| Ok::<_, String>((k * 10, 1)))
            .unwrap();
        for k in keys {
            assert_eq!(p.peek(&k), Some(&(k * 10)), "key {k}");
        }
        assert_eq!(p.used(), 4);
        assert!(p.used() > p.budget());
    }

    #[test]
    fn resolve_load_error_names_the_key() {
        let mut p: Pool<&str, ()> = Pool::new("parsed", 10);
        p.begin_frame(1).unwrap();
        let err = p
            .resolve(&["data/x.dc6"], &mut Tick(0), |_| {
                Err::<((), u64), _>("not found")
            })
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "parsed: loading \"data/x.dc6\" failed: not found"
        );
        assert!(p.is_empty());
    }

    #[test]
    fn eviction_is_independent_of_insert_order() {
        // Same uses, different insertion order within each frame: same
        // survivors (no hash or insertion-order dependence).
        let run = |order: &[&'static str]| {
            let mut p = Pool::new("t", 3);
            p.begin_frame(1).unwrap();
            for k in order {
                p.insert(*k, (), 1).unwrap();
            }
            use_in_frame(&mut p, 2, "z");
            use_in_frame(&mut p, 3, "y");
            keys(&p)
        };
        assert_eq!(run(&["a", "b", "c"]), run(&["c", "a", "b"]));
        assert_eq!(run(&["b", "c", "a"]), ["c", "y", "z"]);
    }

    #[test]
    fn default_budgets() {
        let b = Budgets::default();
        assert_eq!(
            (b.parsed_files, b.frame_sets, b.atlas_pages, b.sounds),
            (128 * MIB, 256 * MIB, 256 * MIB, 64 * MIB)
        );
        assert_eq!(b.atlas_pages, 64 * 2048 * 2048);
    }
}
