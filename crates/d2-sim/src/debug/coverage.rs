//! Per-session behavioural counts. These integers are diagnostic data only:
//! no game decision reads them, and state-1 deliberately excludes them.

use std::collections::BTreeMap;

/// Numeric event keys owned by one game or client; reporting formats the IDs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub rows: BTreeMap<(&'static str, u32), u64>,
    /// Only diagnostic runners opt in; normal games do no bookkeeping.
    pub enabled: bool,
}

impl Counters {
    /// Evaluate the ID only when enabled. The disabled path is one branch.
    #[inline]
    pub fn hit(&mut self, category: &'static str, id: impl FnOnce() -> u32) {
        if self.enabled {
            let count = self.rows.entry((category, id())).or_default();
            *count = count.saturating_add(1);
        }
    }
}

/// `cov!(Cat, a, b)`: a coverage hook; expands to nothing.
#[macro_export]
macro_rules! cov {
    ($cat:ident, $a:expr, $b:expr) => {{}};
}

/// Retained compatibility hook; counts with a game owner use [`Counters`].
pub fn flush() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_does_not_evaluate_event_ids() {
        let mut counters = Counters::default();
        counters.hit("skills_cast", || panic!("disabled ID evaluated"));
        assert!(counters.rows.is_empty());
    }

    #[test]
    fn enabled_events_share_numeric_keys_and_saturate() {
        let mut counters = Counters {
            enabled: true,
            ..Default::default()
        };
        counters.hit("skills_cast", || 47);
        counters.hit("skills_cast", || 47);
        assert_eq!(counters.rows[&("skills_cast", 47)], 2);
        counters.rows.insert(("skills_cast", 47), u64::MAX);
        counters.hit("skills_cast", || 47);
        assert_eq!(counters.rows[&("skills_cast", 47)], u64::MAX);
    }
}
