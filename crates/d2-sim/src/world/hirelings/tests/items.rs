//! The item swap against a recording [`HirelingItems`].

use std::collections::{BTreeMap, BTreeSet};

use crate::items::moves::{Guid, Owner};
use crate::world::hirelings::class;
use crate::world::hirelings::items::{res, swap, HirelingItems};

const PLAYER: Owner = Owner {
    ty: Owner::PLAYER,
    guid: 1,
};
const MERC: Owner = Owner {
    ty: Owner::MONSTER,
    guid: 2,
};
/// The cursor item C.
const C: Guid = 10;
/// The merc's current item at the target.
const OLD: Guid = 20;

#[derive(Default)]
struct Fake {
    log: Vec<String>,
    player_inv: bool,
    merc_inv: bool,
    merc_class: u32,
    locs: (u8, u8),
    shield: bool,
    body: BTreeMap<u8, Guid>,
    req_pass: bool,
    next_guid: Guid,
    /// Items whose duplicate fails.
    dup_fails: BTreeSet<Guid>,
}

impl Fake {
    fn new() -> Self {
        Self {
            player_inv: true,
            merc_inv: true,
            merc_class: class::ACT2,
            locs: (4, 5),
            req_pass: true,
            next_guid: 100,
            ..Self::default()
        }
    }
    fn note(&mut self, s: String) {
        self.log.push(s);
    }
}

fn u(o: Owner) -> &'static str {
    if o == PLAYER {
        "P"
    } else {
        "M"
    }
}

impl HirelingItems for Fake {
    fn has_inventory(&self, unit: Owner) -> bool {
        if unit == PLAYER {
            self.player_inv
        } else {
            self.merc_inv
        }
    }
    fn create_inventory(&mut self, unit: Owner) {
        self.note(format!("create_inventory {}", u(unit)));
    }
    fn body_locs(&self, _item: Guid) -> (u8, u8) {
        self.locs
    }
    fn class(&self, _unit: Owner) -> u32 {
        self.merc_class
    }
    fn is_type(&self, _item: Guid, ty: u32) -> bool {
        ty == 2 && self.shield
    }
    fn body_item(&self, _unit: Owner, loc: u8) -> Option<Guid> {
        self.body.get(&loc).copied()
    }
    fn duplicate(&mut self, owner: Owner, item: Guid) -> Option<Guid> {
        if self.dup_fails.contains(&item) {
            self.note(format!("duplicate {} {item} -> none", u(owner)));
            return None;
        }
        let g = self.next_guid;
        self.next_guid += 1;
        self.note(format!("duplicate {} {item} -> {g}", u(owner)));
        Some(g)
    }
    fn set_mode(&mut self, item: Guid, mode: u8) {
        self.note(format!("mode {item} {mode}"));
    }
    fn cancel_timers(&mut self, unit: Owner, ty: u8, a: Guid) {
        self.note(format!("cancel {} {ty} {a}", u(unit)));
    }
    fn equip_from_cursor(&mut self, unit: Owner, item: Guid, loc: u8, skip: bool) {
        self.note(format!("equip {} {item} {loc} {skip}", u(unit)));
    }
    fn consume(&mut self, _player: Owner, item: Guid) {
        self.note(format!("consume {item}"));
    }
    fn clear_cursor(&mut self, player: Owner) {
        self.note(format!("cursor none {}", u(player)));
    }
    fn inventory_pass(&mut self, unit: Owner) {
        self.note(format!("0055DF00 {}", u(unit)));
    }
    fn refresh_0055f4f0(&mut self, unit: Owner) {
        self.note(format!("0055F4F0 {} 0", u(unit)));
    }
    fn event_next_frame(&mut self, unit: Owner, id: u32) {
        self.note(format!("event {} {id} +1", u(unit)));
    }
    fn unlink(&mut self, unit: Owner, item: Guid) {
        self.note(format!("unlink {} {item}", u(unit)));
    }
    fn clear_slot(&mut self, unit: Owner, loc: u8) {
        self.note(format!("clear_slot {} {loc}", u(unit)));
    }
    fn stat_refresh_unlink(&mut self, unit: Owner) {
        self.note(format!("0055C730 {}", u(unit)));
    }
    fn requirements(&self, item: Guid, unit: Owner, equipping: bool) -> bool {
        assert_eq!((item, unit, equipping), (C, MERC, false));
        self.req_pass
    }
    fn item_flag_on(&mut self, item: Guid, flag: u32) {
        self.note(format!("flag {item} {flag:#x}"));
    }
    fn leave_inventory(&mut self, unit: Owner, item: Guid) {
        self.note(format!("leave {} {item}", u(unit)));
    }
    fn call_00621000(&mut self, unit: Owner, arg: u32) {
        self.note(format!("00621000 {} {arg}", u(unit)));
    }
    fn become_cursor(&mut self, player: Owner, item: Option<Guid>) {
        let item = item.map_or("none".to_string(), |g| g.to_string());
        self.note(format!("become_cursor {} {item}", u(player)));
    }
    fn put_back(&mut self, unit: Owner, item: Guid, page: u8, loc: u8) {
        self.note(format!("put_back {} {item} {page} {loc}", u(unit)));
    }
    fn call_00628280(&mut self, item: Guid, arg: u8) {
        self.note(format!("00628280 {item} {arg:#x}"));
    }
    fn refresh_0055c460(&mut self, merc: Owner) {
        self.note(format!("0055C460 {}", u(merc)));
    }
}

/// Step 3's calls for a copy `copy` equipped at `loc`.
fn rule3(copy: Guid, loc: u8) -> Vec<String> {
    vec![
        format!("duplicate M {C} -> {copy}"),
        format!("mode {copy} 4"),
        format!("cancel M 9 {C}"),
        format!("cancel P 9 {C}"),
        format!("equip M {copy} {loc} true"),
        format!("consume {C}"),
        "cursor none P".into(),
    ]
}

fn rule3_tail() -> Vec<String> {
    vec![
        "0055DF00 M".into(),
        "0055F4F0 M 0".into(),
        "cancel M 3 0".into(),
        "event M 3 +1".into(),
    ]
}

#[test]
fn failed_duplicate_loses_the_item() {
    // Empty target: no mode, no equip; the cancels, the consume, the
    // cursor clear and the tail run; result 1.
    let mut f = Fake::new();
    f.dup_fails.insert(C);
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), res::SWAPPED);
    let mut want: Vec<String> = vec![
        format!("duplicate M {C} -> none"),
        format!("cancel M 9 {C}"),
        format!("cancel P 9 {C}"),
        format!("consume {C}"),
        "cursor none P".into(),
    ];
    want.extend(rule3_tail());
    assert_eq!(f.log, want);

    // Occupied target, C fails and old's copy fails too: old's copy step
    // still runs with none (cursor := none).
    let mut f = Fake::new();
    f.body.insert(4, OLD);
    f.dup_fails.extend([C, OLD]);
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), res::SWAPPED);
    let tail: Vec<String> = f.log[8..].to_vec();
    let mut want: Vec<String> = vec![
        format!("duplicate M {C} -> none"),
        format!("cancel M 9 {C}"),
        format!("cancel P 9 {C}"),
        format!("consume {C}"),
        "cursor none P".into(),
        format!("duplicate P {OLD} -> none"),
        "become_cursor P none".into(),
    ];
    want.extend(rule3_tail());
    assert_eq!(tail, want);
}

#[test]
fn classic_is_3_and_touches_nothing() {
    let mut f = Fake::new();
    assert_eq!(swap(&mut f, false, PLAYER, MERC, C), res::CLASSIC);
    assert_eq!(res::CLASSIC, 3);
    assert!(f.log.is_empty());
}

#[test]
fn no_player_inventory_is_0_and_missing_merc_inventory_is_created() {
    let mut f = Fake::new();
    f.player_inv = false;
    f.merc_inv = false;
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), 0);
    assert!(f.log.is_empty());

    let mut f = Fake::new();
    f.merc_inv = false;
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), 1);
    assert_eq!(f.log[0], "create_inventory M");
}

#[test]
fn empty_target_duplicates_equips_and_consumes() {
    let mut f = Fake::new();
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), res::SWAPPED);
    assert_eq!(res::SWAPPED, 1);
    let mut want = rule3(100, 4);
    want.extend(rule3_tail());
    assert_eq!(f.log, want);
}

#[test]
fn act3_shield_goes_to_location_2() {
    let mut f = Fake::new();
    f.merc_class = class::ACT3;
    f.shield = true;
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), 1);
    assert!(f.log.contains(&"equip M 100 5 true".to_string()));

    // A shield on another class, or a non-shield on Act 3: location 1.
    for (cls, shield) in [(class::ACT2, true), (class::ACT3, false)] {
        let mut f = Fake::new();
        f.merc_class = cls;
        f.shield = shield;
        swap(&mut f, true, PLAYER, MERC, C);
        assert!(f.log.contains(&"equip M 100 4 true".to_string()));
    }

    // The item already at location 2 is the one swapped out.
    let mut f = Fake::new();
    f.merc_class = class::ACT3;
    f.shield = true;
    f.body.insert(5, OLD);
    swap(&mut f, true, PLAYER, MERC, C);
    assert_eq!(f.log[0], format!("unlink M {OLD}"));
    assert_eq!(f.log[1], "clear_slot M 5");
}

#[test]
fn occupied_pass_swaps_and_gives_old_copy_to_cursor() {
    let mut f = Fake::new();
    f.body.insert(4, OLD);
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), 1);
    let mut want: Vec<String> = vec![
        format!("unlink M {OLD}"),
        "clear_slot M 4".into(),
        "0055C730 M".into(),
        "0055DF00 M".into(),
        format!("flag {OLD} 0x10"),
        format!("flag {OLD} 0x20"),
        format!("leave M {OLD}"),
        "00621000 M 1".into(),
    ];
    want.extend(rule3(100, 4));
    want.push(format!("duplicate P {OLD} -> 101"));
    want.push("become_cursor P 101".into());
    want.extend(rule3_tail());
    assert_eq!(f.log, want);
}

#[test]
fn occupied_fail_puts_old_back() {
    let mut f = Fake::new();
    f.body.insert(4, OLD);
    f.req_pass = false;
    assert_eq!(swap(&mut f, true, PLAYER, MERC, C), 0);
    let want: Vec<String> = vec![
        format!("unlink M {OLD}"),
        "clear_slot M 4".into(),
        "0055C730 M".into(),
        "0055DF00 M".into(),
        format!("put_back M {OLD} 3 4"),
        format!("mode {OLD} 1"),
        format!("00628280 {OLD} 0xff"),
        "0055C460 M".into(),
        "0055F4F0 M 0".into(),
    ];
    assert_eq!(f.log, want);
}
