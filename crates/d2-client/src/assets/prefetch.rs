//! Prefetch queue: when the bridge reports a newly active
//! room or a new unit type, the keys of its assets are queued here; the
//! client's loader takes them in request order, loads them off the frame
//! (Bevy task pool) and hands each result to [`Pool::offer`]. Prefetch only
//! changes timing: a frame still resolves every key it lists
//! ([`Pool::resolve`]), so the drawn image is the same with or without it.
//!
//! Plain Rust. Which keys a room or unit type needs comes from the render
//! stage's key lists; the Bevy task edge that runs the loads
//! is wired by the app.
//!
//! [`Pool::offer`]: super::cache::Pool::offer
//! [`Pool::resolve`]: super::cache::Pool::resolve

use std::collections::{BTreeSet, VecDeque};

/// Pending and in-flight prefetch keys. A key is queued at most once while
/// pending or in flight; order is request order (deterministic, no hash
/// iteration).
#[derive(Debug, Clone)]
pub struct PrefetchQueue<K> {
    pending: VecDeque<K>,
    queued: BTreeSet<K>,
    in_flight: BTreeSet<K>,
}

impl<K> Default for PrefetchQueue<K> {
    fn default() -> Self {
        PrefetchQueue {
            pending: VecDeque::new(),
            queued: BTreeSet::new(),
            in_flight: BTreeSet::new(),
        }
    }
}

impl<K: Ord + Clone> PrefetchQueue<K> {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `keys` in order, skipping any already pending or in flight.
    /// Returns how many were added.
    pub fn request(&mut self, keys: impl IntoIterator<Item = K>) -> usize {
        let mut added = 0;
        for key in keys {
            if self.in_flight.contains(&key) || !self.queued.insert(key.clone()) {
                continue;
            }
            self.pending.push_back(key);
            added += 1;
        }
        added
    }

    /// Takes up to `max` pending keys in request order and marks them in
    /// flight. Keys for which `resident` is true are dropped without being
    /// returned (nothing to load).
    pub fn start(&mut self, max: usize, resident: impl Fn(&K) -> bool) -> Vec<K> {
        let mut out = Vec::new();
        while out.len() < max {
            let Some(key) = self.pending.pop_front() else {
                break;
            };
            self.queued.remove(&key);
            if resident(&key) {
                continue;
            }
            self.in_flight.insert(key.clone());
            out.push(key);
        }
        out
    }

    /// A started load ended (offered, refused or failed). False if `key`
    /// was not in flight.
    pub fn finish(&mut self, key: &K) -> bool {
        self.in_flight.remove(key)
    }

    /// Keys queued and not yet started.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Keys started and not yet finished.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Whether nothing is pending or in flight.
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.in_flight.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::cache::{Clock, Offer, Pool};

    struct ZeroClock;

    impl Clock for ZeroClock {
        fn now_micros(&mut self) -> u64 {
            0
        }
    }

    fn load(key: &&'static str) -> Result<(String, u64), String> {
        Ok((format!("decoded {key}"), 1))
    }

    /// Resolves `frames` in order and returns, per frame, the value of
    /// every listed key and the number of stalls.
    fn render(
        pool: &mut Pool<&'static str, String>,
        frames: &[&[&'static str]],
        first: u64,
    ) -> Vec<(Vec<String>, usize)> {
        let mut out = Vec::new();
        for (i, keys) in frames.iter().enumerate() {
            pool.begin_frame(first + i as u64).unwrap();
            let report = pool.resolve(keys, &mut ZeroClock, load).unwrap();
            let values = keys
                .iter()
                .map(|k| pool.peek(k).expect("resolved key is resident").clone())
                .collect();
            out.push((values, report.stalls.len()));
        }
        out
    }

    #[test]
    fn prefetch_changes_stalls_not_what_is_drawn() {
        let frames: [&[&'static str]; 3] = [
            &["room1/floor", "hero"],
            &["room2/wall", "hero"],
            &["room2/wall", "room2/floor", "hero"],
        ];

        // Without prefetch.
        let mut cold = Pool::new("t", 3);
        let cold_out = render(&mut cold, &frames, 1);

        // With prefetch: room 2 became active before frame 1, its keys
        // were queued and loaded off the frame.
        let mut warm = Pool::new("t", 3);
        let mut queue = PrefetchQueue::new();
        assert_eq!(queue.request(["room2/wall", "room2/floor"]), 2);
        for key in queue.start(8, |k| warm.contains(k)) {
            let (value, bytes) = load(&key).unwrap();
            assert_eq!(warm.offer(key, value, bytes), Offer::Taken);
            assert!(queue.finish(&key));
        }
        assert!(queue.is_idle());
        let warm_out = render(&mut warm, &frames, 1);

        // Same values drawn every frame; only the stall counts differ.
        let values = |o: &[(Vec<String>, usize)]| o.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
        assert_eq!(values(&cold_out), values(&warm_out));
        let stalls = |o: &[(Vec<String>, usize)]| o.iter().map(|f| f.1).sum::<usize>();
        assert_eq!(stalls(&cold_out), 4);
        assert!(stalls(&warm_out) < stalls(&cold_out), "{warm_out:?}");
    }

    #[test]
    fn offers_never_evict_or_overrun() {
        let mut p: Pool<&'static str, ()> = Pool::new("t", 2);
        p.begin_frame(1).unwrap();
        p.insert("a", (), 1).unwrap();
        assert_eq!(p.offer("a", (), 1), Offer::Resident);
        assert_eq!(p.offer("b", (), 2), Offer::NoRoom);
        assert_eq!(p.offer("b", (), 1), Offer::Taken);
        assert_eq!(p.offer("c", (), 1), Offer::NoRoom);
        assert_eq!(p.used(), 2);
        assert!(p.drain_events().is_empty());
        // A prefetched entry is not protected by the current frame: a
        // listed key that needs room evicts it, not an entry in use.
        p.insert("d", (), 1).unwrap();
        assert_eq!(p.keys().copied().collect::<Vec<_>>(), ["a", "d"]);
    }

    #[test]
    fn queue_keeps_request_order_without_duplicates() {
        let mut q = PrefetchQueue::new();
        assert_eq!(q.request(["b", "a", "b"]), 2);
        assert_eq!(q.request(["a", "c"]), 1);
        assert_eq!(q.pending(), 3);
        assert_eq!(q.start(2, |_| false), ["b", "a"]);
        // In flight: not queued again until finished.
        assert_eq!(q.request(["b"]), 0);
        assert!(q.finish(&"b"));
        assert!(!q.finish(&"b"));
        assert_eq!(q.request(["b"]), 1);
        // Resident keys are dropped, not started.
        assert_eq!(q.start(8, |k| *k == "c"), ["b"]);
        assert_eq!(q.in_flight(), 2);
        assert_eq!(q.pending(), 0);
    }
}
