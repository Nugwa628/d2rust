//! Synthetic checks of the set-item state update on a fake world.

use std::collections::BTreeMap;

use super::*;

const O: UnitId = UnitId(1);
const I: UnitId = UnitId(2);

#[derive(Default)]
struct W {
    item_units: Vec<UnitId>,
    inv: bool,
    in_body: bool,
    mask: u32,
    quality: BTreeMap<UnitId, u8>,
    row: Option<SetItemRow>,
    sets: usize,
    /// O's lists: state → stat 71.
    lists: BTreeMap<u32, i32>,
    log: Vec<String>,
}

impl W {
    fn new(add_func: u8, slot: i16, set: i16) -> Self {
        Self {
            item_units: vec![I],
            inv: true,
            in_body: true,
            quality: [(I, QUALITY_SET)].into_iter().collect(),
            row: Some(SetItemRow {
                add_func,
                slot,
                set,
            }),
            sets: 10,
            ..Self::default()
        }
    }
    fn parks(&self) -> Vec<String> {
        self.log
            .iter()
            .filter(|l| l.starts_with("park") || l.starts_with("unpark"))
            .cloned()
            .collect()
    }
}

impl SetWorld for W {
    fn is_item(&self, u: UnitId) -> bool {
        self.item_units.contains(&u)
    }
    fn has_inventory(&self, _: UnitId) -> bool {
        self.inv
    }
    fn in_body_of(&self, _: UnitId, _: UnitId) -> bool {
        self.in_body
    }
    fn set_mask(&self, _: UnitId, _: UnitId) -> u32 {
        self.mask
    }
    fn quality(&self, i: UnitId) -> u8 {
        self.quality.get(&i).copied().unwrap_or(0)
    }
    fn setitem_row(&self, _: UnitId) -> Option<SetItemRow> {
        self.row
    }
    fn sets_count(&self) -> usize {
        self.sets
    }
    fn park(&mut self, _: UnitId, state: u32, park: bool) {
        self.log
            .push(format!("{} {state}", if park { "park" } else { "unpark" }));
    }
    fn owner_list_tag(&self, _: UnitId, state: u32) -> Option<i32> {
        self.lists.get(&state).copied()
    }
    fn reset_owner_list(&mut self, _: UnitId, state: u32, set: i32) {
        self.lists.insert(state, set);
        self.log.push(format!("reset {state} {set}"));
    }
    fn free_owner_list(&mut self, _: UnitId, state: u32) {
        self.lists.remove(&state);
        self.log.push(format!("free {state}"));
    }
    fn new_owner_list(&mut self, _: UnitId, _: UnitId, state: u32) {
        self.lists.insert(state, 0);
        self.log.push(format!("new {state}"));
    }
    fn tag_owner_list(&mut self, _: UnitId, state: u32, set: i32) {
        self.lists.insert(state, set);
        self.log.push(format!("tag {state} {set}"));
    }
    fn set_bonuses(&mut self, _: UnitId, _: UnitId, state: u32) {
        self.log.push(format!("bonus {state}"));
    }
}

#[test]
fn update_needs_owner_item_and_inventory() {
    let mut w = W::new(1, 0, 3);
    assert!(!set_item_update(&mut w, None, Some(I), 0, 0));
    assert!(!set_item_update(&mut w, Some(O), None, 0, 0));
    assert!(!set_item_update(&mut w, Some(O), Some(UnitId(9)), 0, 0));
    w.inv = false;
    assert!(!set_item_update(&mut w, Some(O), Some(I), 0, 0));
    assert!(w.log.is_empty());
    w.inv = true;
    assert!(set_item_update(&mut w, Some(O), Some(I), 0, 0));
}

#[test]
fn mask_only_in_the_body_and_parking_only_for_add_or_repark() {
    // Not in the body: mask 0; f = 2 → park S[0..4] after the dword
    // before the table.
    let mut w = W::new(2, 0, 3);
    w.in_body = false;
    w.mask = 0b11;
    set_item_update(&mut w, Some(O), Some(I), 0, 0);
    assert_eq!(
        w.parks(),
        [
            "park 3375",
            "park 165",
            "park 166",
            "park 167",
            "park 168",
            "park 169"
        ]
    );
    // r = 1, p = 0 (the deactivation): no park step, owner list freed.
    let mut w = W::new(2, 0, 3);
    w.lists.insert(165, 3);
    set_item_update(&mut w, Some(O), Some(I), 1, 0);
    assert!(w.parks().is_empty());
    assert_eq!(w.log, ["free 165"]);
    // r = 1, p = 1 (the break): both.
    let mut w = W::new(2, 0, 3);
    w.lists.insert(165, 3);
    w.mask = 0b1;
    set_item_update(&mut w, Some(O), Some(I), 1, 1);
    assert_eq!(w.log.last().unwrap(), "free 165");
    assert!(!w.parks().is_empty());
}

#[test]
fn partial_lists_add_func_one_follow_the_slot_bits() {
    // Slot n = 2, mask bits 0, 3, 5 → i = 0 (k 0) unpark, 1 (k 1) park,
    // 3 (k 2) unpark, 4 (k 3) park, 5 (k 4) unpark.
    let mut w = W::new(1, 2, 3);
    partial_lists(&mut w, I, 0b101001);
    assert_eq!(
        w.parks(),
        [
            "unpark 165",
            "park 166",
            "unpark 167",
            "park 168",
            "unpark 169"
        ]
    );
}

#[test]
fn partial_lists_add_func_two_count_the_pieces() {
    // b = 3 → k = 2: unpark S[0], S[1]; park S[2..4].
    let mut w = W::new(2, 0, 3);
    partial_lists(&mut w, I, 0b10101);
    assert_eq!(
        w.parks(),
        [
            "unpark 165",
            "unpark 166",
            "park 167",
            "park 168",
            "park 169"
        ]
    );
    // b = 6 → k = 5: unpark S[0..4], no park. Mask ≥ 64 counts 0.
    let mut w = W::new(2, 0, 3);
    partial_lists(&mut w, I, 0b111111);
    assert_eq!(w.parks().len(), 5);
    assert!(w.parks().iter().all(|p| p.starts_with("unpark")));
    let mut w = W::new(2, 0, 3);
    partial_lists(&mut w, I, 64);
    assert_eq!(w.parks()[0], "park 3375");
    // f = 0 or another value, not a set item, no row: nothing.
    for f in [0, 3] {
        let mut w = W::new(f, 0, 3);
        partial_lists(&mut w, I, 0b11);
        assert!(w.log.is_empty());
    }
    let mut w = W::new(1, 0, 3);
    w.quality.insert(I, 4);
    partial_lists(&mut w, I, 0b11);
    w.row = None;
    w.quality.insert(I, QUALITY_SET);
    partial_lists(&mut w, I, 0b11);
    assert!(w.log.is_empty());
}

#[test]
fn owner_list_refills_or_creates_the_set_list() {
    // A list tagged with the set: reset, then the bonuses.
    let mut w = W::new(0, 0, 3);
    w.lists.insert(166, 3);
    assert!(owner_list(&mut w, O, I, 0));
    assert_eq!(w.log, ["reset 166 3", "bonus 166"]);
    // Other sets only, a free state: a new list in the first free state.
    let mut w = W::new(0, 0, 3);
    w.lists.insert(165, 7);
    assert!(owner_list(&mut w, O, I, 0));
    assert_eq!(w.log, ["new 166", "tag 166 3", "bonus 166"]);
    // r < 0: the new list only.
    let mut w = W::new(0, 0, 3);
    assert!(owner_list(&mut w, O, I, -1));
    assert_eq!(w.log, ["new 165"]);
    // r = 1 without a list, or no free state: 0.
    let mut w = W::new(0, 0, 3);
    assert!(!owner_list(&mut w, O, I, 1));
    for s in SET_STATES {
        w.lists.insert(s, 9);
    }
    assert!(!owner_list(&mut w, O, I, 0));
    // Guards: r > 1, item not of quality 5, owner a magic+ item, set out
    // of range.
    let mut w = W::new(0, 0, 3);
    assert!(!owner_list(&mut w, O, I, 2));
    w.item_units.push(O);
    w.quality.insert(O, 4);
    assert!(!owner_list(&mut w, O, I, 0));
    let mut w = W::new(0, 0, 10);
    assert!(!owner_list(&mut w, O, I, 0));
    let mut w = W::new(0, 0, -1);
    assert!(!owner_list(&mut w, O, I, 0));
    assert!(w.log.is_empty());
}

#[test]
fn stat_refresh_runs_the_set_update_for_set_items_only() {
    let mut w = W::new(0, 0, 3);
    assert!(stat_refresh_set(&mut w, O, I, false));
    assert_eq!(w.log, ["new 165", "tag 165 3", "bonus 165"]);
    let mut w = W::new(0, 0, 3);
    assert!(!stat_refresh_set(&mut w, O, I, true));
    w.quality.insert(I, 7);
    assert!(!stat_refresh_set(&mut w, O, I, false));
    assert!(w.log.is_empty());
}
