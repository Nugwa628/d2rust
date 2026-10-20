//! Waypoint rows: the row rebuild of the current tab, the tab chosen when the
//! menu opens and the quest-record gates of the tab draw and the tab setter.

/// Level ranges of tabs 0–4 (table).
pub const TAB_LEVELS: [(u32, u32); 5] = [(1, 38), (40, 74), (75, 102), (103, 107), (109, 136)];

/// Rows per tab (≤ 9).
pub const MAX_ROWS: usize = 9;

/// The per-tab cache of the lowest and highest waypoint index (computed on
/// the tab's first use, once per process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabCache {
    pub cached: bool,
    /// Initially 0xFF.
    pub lowest: u8,
    pub highest: u8,
}

impl Default for TabCache {
    fn default() -> Self {
        Self {
            cached: false,
            lowest: 0xFF,
            highest: 0,
        }
    }
}

/// Waypoint data the rebuild reads.
pub trait WaypointData {
    /// The `levels.txt` `Waypoint` index of a level.
    fn index_of_level(&self, level: u32) -> Option<u8>;
    /// The waypoint is known.
    fn known(&self, index: u8) -> bool;
    /// The level of waypoint index `w` (first match).
    fn level_of_index(&self, index: u8) -> Option<u32>;
}

/// One row: the level and the used byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub level: u32,
    pub used: bool,
}

/// The rebuilt rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    /// The row count = `rows.len`.
    pub rows: Vec<Row>,
    /// Other known waypoints.
    pub other_known: u32,
    /// The travel latch was cleared (tab out of range).
    pub latch_cleared: bool,
}

/// Row rebuild. `current` = the level of the
/// player's room.
pub fn rebuild(
    tab: usize,
    cache: &mut [TabCache; 5],
    current: u32,
    data: &dyn WaypointData,
) -> Rows {
    let mut out = Rows::default();
    // The game tests t > 5, but its tab setter turns
    // any tab ≥ 5 (the tab count) into 0, so only 0–4 reach
    // here; `> 4` keeps the cache index in range and gives the same rows.
    if tab > 4 {
        out.latch_cleared = true;
        return out;
    }
    let c = &mut cache[tab];
    if !c.cached {
        c.cached = true;
        let (first, last) = TAB_LEVELS[tab];
        for level in first..=last {
            if let Some(idx) = data.index_of_level(level) {
                if c.lowest == 0xFF {
                    c.lowest = idx;
                }
                c.highest = c.highest.max(idx);
            }
        }
    }
    if c.lowest == 0xFF {
        return out;
    }
    for w in c.lowest..=c.highest {
        if out.rows.len() >= MAX_ROWS {
            break;
        }
        let Some(level) = data.level_of_index(w) else {
            continue;
        };
        if data.known(w) {
            let used = level != current;
            if used {
                out.other_known += 1;
            }
            out.rows.push(Row { level, used });
        } else {
            out.rows.push(Row { level, used: false });
        }
    }
    out
}

/// The tab at open: the act of the player's level; 0 without a
/// player, room or level.
pub fn tab_at_open(act: Option<u8>) -> u8 {
    act.unwrap_or(0)
}

/// The tab draw gate: tab 0 always; tabs 1–4
/// test client quest records 7, 15, 23, 26 bit 0.
pub fn tab_reachable_for_draw(tab: u8, quest_bit: &dyn Fn(u8, u8) -> bool) -> bool {
    match tab {
        1 => quest_bit(7, 0),
        2 => quest_bit(15, 0),
        3 => quest_bit(23, 0),
        4 => quest_bit(26, 0),
        _ => true,
    }
}

/// The tab setter: `t` ≥ 5 → 0; walking down from `t`, tab 4 needs record 28
/// bit 0, tab 3 record 23, tab 2 record 15, tab 1 record 7; a failed check sets
/// `t`:= that tab − 1 and the walk continues.
pub fn set_tab(t: u8, quest_bit: &dyn Fn(u8, u8) -> bool) -> u8 {
    if t >= 5 {
        return 0;
    }
    let mut t = t;
    while t > 0 {
        let record = match t {
            4 => 28,
            3 => 23,
            2 => 15,
            _ => 7,
        };
        if quest_bit(record, 0) {
            break;
        }
        t -= 1;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    struct Data {
        /// level → index
        levels: HashMap<u32, u8>,
        known: HashSet<u8>,
    }

    impl WaypointData for Data {
        fn index_of_level(&self, level: u32) -> Option<u8> {
            self.levels.get(&level).copied()
        }
        fn known(&self, index: u8) -> bool {
            self.known.contains(&index)
        }
        fn level_of_index(&self, index: u8) -> Option<u32> {
            let mut v: Vec<u32> = self
                .levels
                .iter()
                .filter(|(_, &i)| i == index)
                .map(|(&l, _)| l)
                .collect();
            v.sort();
            v.first().copied()
        }
    }

    fn act1() -> Data {
        // act 1 waypoints: indices 0–8 on levels 1, 3, 4, 5, 6, 27, 29, 32, 35
        let levels = [1, 3, 4, 5, 6, 27, 29, 32, 35]
            .iter()
            .enumerate()
            .map(|(i, &l)| (l, i as u8))
            .collect();
        Data {
            levels,
            known: HashSet::new(),
        }
    }

    #[test]
    fn row_rebuild() {
        let mut d = act1();
        d.known.extend([0u8, 1, 4]);
        let mut cache = [TabCache::default(); 5];
        // standing on level 3: known rows used except the current level's
        let r = rebuild(0, &mut cache, 3, &d);
        assert_eq!(r.rows.len(), 9);
        assert_eq!(
            r.rows[0],
            Row {
                level: 1,
                used: true
            }
        );
        assert_eq!(
            r.rows[1],
            Row {
                level: 3,
                used: false
            }
        );
        // unknown rows: used 0
        assert_eq!(
            r.rows[2],
            Row {
                level: 4,
                used: false
            }
        );
        assert_eq!(
            r.rows[4],
            Row {
                level: 6,
                used: true
            }
        );
        // other known = known waypoints other than the current level's
        assert_eq!(r.other_known, 2);
        assert!(!r.latch_cleared);
        // the first use filled the cache
        assert_eq!(
            cache[0],
            TabCache {
                cached: true,
                lowest: 0,
                highest: 8
            }
        );
        // a later rebuild uses the cache: changing the data does not
        let empty = Data {
            levels: HashMap::new(),
            known: HashSet::new(),
        };
        let r2 = rebuild(0, &mut cache, 3, &empty);
        assert!(r2.rows.is_empty(), "the indices have no levels now");
        // a tab with no waypoints: no rows, the cache holds the sentinel
        let r3 = rebuild(1, &mut cache, 3, &d);
        assert!(r3.rows.is_empty());
        assert_eq!(cache[1].lowest, 0xFF);
        // tab out of range: latch cleared, no rows
        let r4 = rebuild(5, &mut cache, 3, &d);
        assert!(r4.latch_cleared && r4.rows.is_empty());
        // every mapped index adds a row in index order; at most 9
        let mut many = act1();
        many.levels.insert(36, 9);
        let mut c = [TabCache::default(); 5];
        assert_eq!(rebuild(0, &mut c, 1, &many).rows.len(), 9);
    }

    #[test]
    fn open_tab() {
        assert_eq!(tab_at_open(Some(2)), 2);
        assert_eq!(tab_at_open(None), 0);
    }

    #[test]
    fn current_level_row_cannot_be_chosen() {
        // rows hit tests skip used = 0 rows: the current level's row has
        // used = false after a rebuild
        let mut d = act1();
        d.known.insert(0);
        let mut cache = [TabCache::default(); 5];
        let r = rebuild(0, &mut cache, 1, &d);
        assert!(!r.rows[0].used);
        assert_eq!(r.other_known, 0);
    }

    #[test]
    fn tab_gates() {
        let q = |set: &'static [u8]| move |rec: u8, bit: u8| bit == 0 && set.contains(&rec);
        // draw: 7, 15, 23 and 26 for tabs 1–4
        assert!(tab_reachable_for_draw(0, &q(&[])));
        assert!(tab_reachable_for_draw(1, &q(&[7])));
        assert!(!tab_reachable_for_draw(1, &q(&[15])));
        assert!(tab_reachable_for_draw(4, &q(&[26])));
        assert!(!tab_reachable_for_draw(4, &q(&[28])));
        // the setter uses record 28 for tab 4 (reproduced)
        assert_eq!(set_tab(4, &q(&[28, 23, 15, 7])), 4);
        assert_eq!(set_tab(4, &q(&[26, 23, 15, 7])), 3);
        assert_eq!(set_tab(4, &q(&[15, 7])), 2);
        assert_eq!(set_tab(4, &q(&[7])), 1);
        assert_eq!(set_tab(4, &q(&[])), 0);
        assert_eq!(set_tab(2, &q(&[7])), 1);
        assert_eq!(set_tab(5, &q(&[28])), 0);
        assert_eq!(set_tab(0, &q(&[])), 0);
    }
}
