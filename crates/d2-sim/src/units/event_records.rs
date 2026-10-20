//! The unit event records (unit +0x90): a unit's doubly linked list of
//! 0x20-byte records that hooks run. Add,
//! find, remove
//! and trigger.
//!
//! The list is held head first. A record keeps an identity ([`RecordId`])
//! so the trigger can read the next record after a call, the way the
//! base game follows the record's own link.

/// Flags bit 1 (u16 +0x02): the record's function is running.
pub const RUNNING: u16 = 1;
/// Flags bit 2: remove pending (removed while running).
pub const REMOVE_PENDING: u16 = 2;
/// Highest table index of the by-index add.
pub const MAX_INDEX: u32 = 0x31;

/// One record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventRecord {
    /// Event id (`events.txt` index, +0x00).
    pub event: u8,
    /// +0x02.
    pub flags: u16,
    /// Owner kind (+0x04); 0 is a one-shot.
    pub kind: i32,
    /// Key (+0x08).
    pub key: i32,
    /// First value passed to the function (+0x0C).
    pub v0: i32,
    /// Second value (+0x10).
    pub v1: i32,
    /// Function (+0x14): a function id of the caller's table.
    pub func: i32,
}

/// A record's identity while it is linked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordId(u32);

/// A unit's record list (null head = empty).
#[derive(Clone, Debug, Default)]
pub struct UnitEvents {
    /// Head first.
    list: Vec<(RecordId, EventRecord)>,
    next_id: u32,
}

impl UnitEvents {
    /// The records, head first.
    pub fn records(&self) -> impl Iterator<Item = &EventRecord> {
        self.list.iter().map(|(_, r)| r)
    }

    /// Number of linked records.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn pos(&self, id: RecordId) -> Option<usize> {
        self.list.iter().position(|(i, _)| *i == id)
    }

    /// The record of `id`, if still linked.
    pub fn get(&self, id: RecordId) -> Option<&EventRecord> {
        self.pos(id).map(|p| &self.list[p].1)
    }

    /// Add: a zeroed record filled and **prepended**.
    pub fn add(&mut self, event: u8, v0: i32, v1: i32, func: i32, kind: i32, key: i32) -> RecordId {
        let id = RecordId(self.next_id);
        self.next_id += 1;
        let rec = EventRecord {
            event,
            flags: 0,
            kind,
            key,
            v0,
            v1,
            func,
        };
        self.list.insert(0, (id, rec));
        id
    }

    /// Add by table index: index > 0x31 or a null table entry
    /// (`has_function(index)` false) adds nothing. The function field is
    /// the index itself (same order as the table).
    #[allow(clippy::too_many_arguments)]
    pub fn add_by_index(
        &mut self,
        has_function: impl Fn(u32) -> bool,
        event: u8,
        v0: i32,
        v1: i32,
        index: u32,
        kind: i32,
        key: i32,
    ) -> Option<RecordId> {
        if index > MAX_INDEX || !has_function(index) {
            return None;
        }
        Some(self.add(event, v0, v1, index as i32, kind, key))
    }

    /// Find: the first record from the head with equal kind,
    /// key and v0.
    pub fn find(&self, kind: i32, key: i32, v0: i32) -> Option<RecordId> {
        self.list
            .iter()
            .find(|(_, r)| r.kind == kind && r.key == key && r.v0 == v0)
            .map(|(i, _)| *i)
    }

    /// Remove: every record with that kind and key; a
    /// running one is marked remove-pending, the others are unlinked.
    pub fn remove(&mut self, kind: i32, key: i32) {
        self.list.retain_mut(|(_, r)| {
            if r.kind != kind || r.key != key {
                return true;
            }
            if r.flags & RUNNING != 0 {
                r.flags |= REMOVE_PENDING;
                true
            } else {
                false
            }
        });
    }

    /// Trigger for a unit that exists: from the head, each
    /// record of `event` is marked running and `call(list, record)` runs
    /// (it may add, find and remove records); then the running mark is
    /// cleared and, if remove-pending or `kind` = 0, the record is
    /// unlinked. The next record is read after the call, so a record
    /// added at the head by the call is not visited. Returns the last
    /// call's result, 0 when none ran.
    pub fn trigger(
        &mut self,
        event: u8,
        mut call: impl FnMut(&mut UnitEvents, EventRecord) -> i32,
    ) -> i32 {
        let mut last = 0;
        let mut cur = self.list.first().map(|(i, _)| *i);
        while let Some(id) = cur {
            let Some(p) = self.pos(id) else {
                break;
            };
            let mut rec = self.list[p].1;
            if rec.event == event {
                self.list[p].1.flags |= RUNNING;
                rec.flags |= RUNNING;
                last = call(self, rec);
                // The running record cannot have been unlinked.
                let p = self.pos(id).expect("a running record stays linked");
                self.list[p].1.flags &= !RUNNING;
                let next = self.list.get(p + 1).map(|(i, _)| *i);
                let r = self.list[p].1;
                if r.flags & REMOVE_PENDING != 0 || r.kind == 0 {
                    self.list.remove(p);
                }
                cur = next;
            } else {
                cur = self.list.get(p + 1).map(|(i, _)| *i);
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_prepends_and_by_index_filters() {
        let mut e = UnitEvents::default();
        let a = e.add(9, 1, 2, 5, 1, 7);
        let b = e.add(9, 3, 4, 6, 1, 8);
        assert_eq!(e.records().map(|r| r.key).collect::<Vec<_>>(), [8, 7]);
        assert_eq!(e.get(a).unwrap().flags, 0);
        assert_eq!(e.get(b).map(|r| (r.v0, r.v1, r.func)), Some((3, 4, 6)));
        let has = |i: u32| i != 4;
        assert!(e.add_by_index(has, 1, 0, 0, 3, 1, 1).is_some());
        assert!(e.add_by_index(has, 1, 0, 0, 4, 1, 1).is_none());
        assert!(e.add_by_index(has, 1, 0, 0, 0x32, 1, 1).is_none());
        assert!(e.add_by_index(has, 1, 0, 0, 0x31, 1, 1).is_some());
        assert_eq!(e.len(), 4);
    }

    #[test]
    fn find_is_first_from_the_head() {
        let mut e = UnitEvents::default();
        let a = e.add(1, 5, 0, 1, 2, 3);
        let b = e.add(1, 5, 9, 1, 2, 3);
        assert_eq!(e.find(2, 3, 5), Some(b));
        assert_eq!(e.find(2, 3, 6), None);
        assert_eq!(e.find(2, 4, 5), None);
        e.remove(2, 3);
        assert_eq!(e.find(2, 3, 5), None);
        assert!(e.get(a).is_none());
    }

    #[test]
    fn remove_marks_running_records_pending() {
        let mut e = UnitEvents::default();
        e.add(1, 0, 0, 1, 1, 5);
        e.add(1, 0, 0, 1, 1, 6);
        e.add(1, 0, 0, 1, 1, 5);
        e.trigger(1, |l, r| {
            if r.key == 5 && r.kind == 1 && l.len() == 3 {
                l.remove(1, 5);
            }
            0
        });
        // The running one (the head) was marked and then dropped by the
        // trigger; the unvisited same-key record was unlinked at once.
        assert_eq!(e.records().map(|r| r.key).collect::<Vec<_>>(), [6]);
    }

    #[test]
    fn trigger_order_one_shots_and_next_after_the_call() {
        let mut e = UnitEvents::default();
        e.add(2, 0, 0, 100, 1, 1); // runs last
        e.add(3, 0, 0, 101, 1, 2); // other event
        e.add(2, 7, 8, 102, 0, 3); // one-shot
        e.add(2, 0, 0, 103, 1, 4); // head: runs first
        let mut seen = Vec::new();
        let r = e.trigger(2, |l, r| {
            seen.push((r.func, r.flags, r.v0, r.v1));
            if r.func == 103 {
                // Added at the head: not visited in this run.
                l.add(2, 0, 0, 104, 1, 9);
            }
            r.func
        });
        assert_eq!(
            seen,
            [
                (103, RUNNING, 0, 0),
                (102, RUNNING, 7, 8),
                (100, RUNNING, 0, 0)
            ]
        );
        // The result of the last function that ran.
        assert_eq!(r, 100);
        // The one-shot is gone; the new record stands at the head.
        let keys: Vec<_> = e.records().map(|r| r.key).collect();
        assert_eq!(keys, [9, 4, 2, 1]);
        assert!(e.records().all(|r| r.flags == 0));
        // No match → 0.
        assert_eq!(e.trigger(60, |_, _| 5), 0);
        // A function removing the next record: it is not visited.
        let mut e = UnitEvents::default();
        e.add(2, 0, 0, 1, 1, 20);
        e.add(2, 0, 0, 2, 1, 10);
        let mut ran = Vec::new();
        e.trigger(2, |l, r| {
            ran.push(r.func);
            l.remove(1, 20);
            0
        });
        assert_eq!(ran, [2]);
        // Remove-pending while running: unlinked after the call.
        let mut e = UnitEvents::default();
        e.add(2, 0, 0, 1, 1, 10);
        e.trigger(2, |l, _| {
            l.remove(1, 10);
            assert_eq!(l.len(), 1);
            0
        });
        assert!(e.is_empty());
    }
}
