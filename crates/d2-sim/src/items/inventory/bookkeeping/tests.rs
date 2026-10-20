//! Synthetic checks of the equipment bookkeeping on a fake world.

use std::collections::BTreeMap;

use super::*;
use crate::items::inventory::page;

#[derive(Clone, Default)]
struct It {
    ty: i16,
    is: Vec<i16>,
    flags: u32,
    mode: u8,
    node: u8,
    page: u8,
    quality: u8,
    quantity: i32,
    book: Option<i32>,
    active: bool,
    usable: bool,
}

#[derive(Default)]
struct W {
    unit_ty: u8,
    unit_flags: u32,
    inv: bool,
    list: Vec<UnitId>,
    body: BTreeMap<u8, UnitId>,
    weapon: Option<UnitId>,
    items: BTreeMap<UnitId, It>,
    linked: Vec<UnitId>,
    stats: BTreeMap<u16, i32>,
    skills: BTreeMap<i32, i32>,
    /// Skills learn does not add (the fatal case).
    unlearnable: Vec<i32>,
    mouse: [Option<SkillRef>; 2],
    saved: [SkillRef; 2],
    unusable: Vec<SkillRef>,
    throw_rows: Vec<i32>,
    log: Vec<String>,
}

const U: UnitId = UnitId(1);

fn it(n: u32) -> UnitId {
    UnitId(100 + n)
}

impl W {
    fn player() -> Self {
        Self {
            inv: true,
            ..Self::default()
        }
    }
    fn add(&mut self, i: UnitId, x: It) {
        self.list.push(i);
        self.items.insert(i, x);
    }
    fn side(left: bool) -> usize {
        usize::from(!left)
    }
}

impl EquipWorld for W {
    fn unit_type(&self, _: UnitId) -> Option<u8> {
        Some(self.unit_ty)
    }
    fn unit_flags(&self, _: UnitId) -> u32 {
        self.unit_flags
    }
    fn has_inventory(&self, _: UnitId) -> bool {
        self.inv
    }
    fn item_list(&self, _: UnitId) -> Vec<UnitId> {
        self.list.clone()
    }
    fn body_item(&self, _: UnitId, loc: u8) -> Option<UnitId> {
        self.body.get(&loc).copied()
    }
    fn weapon_in_use(&self, _: UnitId) -> Option<UnitId> {
        self.weapon
    }
    fn add_unit_stat(&mut self, _: UnitId, stat: u16, d: i32) {
        *self.stats.entry(stat).or_default() += d;
    }
    fn item_type(&self, i: UnitId) -> i16 {
        self.items[&i].ty
    }
    fn item_is_type(&self, i: UnitId, t: i16) -> bool {
        let x = &self.items[&i];
        x.ty == t || x.is.contains(&t)
    }
    fn item_flags(&self, i: UnitId) -> u32 {
        self.items[&i].flags
    }
    fn set_item_flag(&mut self, i: UnitId, bits: u32, on: bool) {
        let f = &mut self.items.get_mut(&i).unwrap().flags;
        if on {
            *f |= bits;
        } else {
            *f &= !bits;
        }
        self.log.push(format!("flag {} {bits:#x} {on}", i.0));
    }
    fn item_mode(&self, i: UnitId) -> u8 {
        self.items[&i].mode
    }
    fn item_node(&self, i: UnitId) -> u8 {
        self.items[&i].node
    }
    fn item_page(&self, i: UnitId) -> u8 {
        self.items[&i].page
    }
    fn item_quality(&self, i: UnitId) -> u8 {
        self.items[&i].quality
    }
    fn item_stat(&self, i: UnitId, stat: u16) -> i32 {
        assert_eq!(stat, STAT_QUANTITY);
        self.items[&i].quantity
    }
    fn book_skill(&self, i: UnitId, _scroll: bool) -> Option<i32> {
        self.items[&i].book
    }
    fn active_inventory_item(&self, _: UnitId, i: UnitId) -> bool {
        self.items[&i].active
    }
    fn usable(&self, _: UnitId, i: UnitId) -> bool {
        self.items[&i].usable
    }
    fn stat_linked(&self, _: UnitId, i: UnitId) -> bool {
        self.linked.contains(&i)
    }
    fn stat_link(&mut self, _: UnitId, i: UnitId) {
        self.linked.push(i);
        self.log.push(format!("link {}", i.0));
    }
    fn stat_unlink(&mut self, _: UnitId, i: UnitId) {
        self.linked.retain(|&x| x != i);
        self.log.push(format!("unlink {}", i.0));
    }
    fn deactivate(&mut self, i: UnitId, _: UnitId) {
        self.log.push(format!("deactivate {}", i.0));
    }
    fn stat_refresh(&mut self, i: UnitId, _: UnitId) {
        self.log.push(format!("refresh {}", i.0));
    }
    fn owner_refresh(&mut self, _: UnitId) {
        self.log.push("owner refresh".into());
    }
    fn send_unit_refresh(&mut self, _: UnitId) {
        self.log.push("0x48".into());
    }
    fn skill_quantity(&self, _: UnitId, s: i32) -> Option<i32> {
        self.skills.get(&s).copied()
    }
    fn set_skill_quantity(&mut self, _: UnitId, s: i32, q: i32) {
        self.skills.insert(s, q);
    }
    fn learn_skill(&mut self, _: UnitId, s: i32) {
        self.log.push(format!("learn {s}"));
        if !self.unlearnable.contains(&s) {
            self.skills.insert(s, 0);
        }
    }
    fn send_skill_quantity(&mut self, _: UnitId, s: i32, q: i32) {
        self.log.push(format!("0x22 {s} {q}"));
    }
    fn mouse_skill(&self, _: UnitId, left: bool) -> Option<SkillRef> {
        self.mouse[Self::side(left)]
    }
    fn select_skill(&mut self, _: UnitId, left: bool, s: SkillRef) {
        self.mouse[Self::side(left)] = Some(s);
        self.log
            .push(format!("select {} {s:?}", if left { "L" } else { "R" }));
    }
    fn has_skill(&self, _: UnitId, s: SkillRef) -> bool {
        self.skills.contains_key(&s.0) || s.0 == 0 || s.0 == SKILL_THROW
    }
    fn use_state(&mut self, _: UnitId, s: SkillRef) -> u8 {
        if self.unusable.contains(&s) {
            USE_NO_QUANTITY
        } else {
            0
        }
    }
    fn throw_skill_row(&self, skill: i32) -> bool {
        self.throw_rows.contains(&skill)
    }
    fn saved_mouse_skill(&self, _: UnitId, left: bool) -> SkillRef {
        self.saved[Self::side(left)]
    }
    fn set_saved_mouse_skill(&mut self, _: UnitId, left: bool, s: SkillRef) {
        self.saved[Self::side(left)] = s;
    }
}

const TP: i32 = 220;

fn tome(q: i32) -> It {
    It {
        ty: TYPE_BOOK,
        quantity: q,
        book: Some(TP),
        ..It::default()
    }
}

fn scroll(q: i32) -> It {
    It {
        ty: TYPE_SCROLL,
        quantity: q,
        book: Some(TP),
        ..It::default()
    }
}

#[test]
fn item_skill_link_needs_a_player_and_a_scroll_or_tome() {
    let mut w = W::player();
    w.add(
        it(1),
        It {
            ty: 3,
            book: Some(TP),
            ..It::default()
        },
    );
    assert_eq!(item_skill_link(&mut w, U, it(1), true), Ok(false));
    // A tome with no row, a tome with quantity 0: nothing.
    w.add(
        it(2),
        It {
            book: None,
            ..tome(5)
        },
    );
    w.add(it(3), tome(0));
    assert_eq!(item_skill_link(&mut w, U, it(2), true), Ok(false));
    assert_eq!(item_skill_link(&mut w, U, it(3), true), Ok(false));
    // A scroll of quantity 0 counts 1.
    w.skills.insert(TP, 4);
    w.add(it(4), scroll(0));
    assert_eq!(item_skill_link(&mut w, U, it(4), true), Ok(true));
    assert_eq!(w.skills[&TP], 5);
    // Not a player: nothing.
    w.unit_ty = 1;
    assert_eq!(item_skill_link(&mut w, U, it(4), true), Ok(false));
    assert_eq!(w.skills[&TP], 5);
}

#[test]
fn item_skill_link_adds_learns_and_unlinks() {
    let mut w = W::player();
    w.add(it(1), tome(12));
    // Absent: newskills += q, learn, quantity := q.
    assert_eq!(item_skill_link(&mut w, U, it(1), true), Ok(true));
    assert_eq!(w.stats[&STAT_NEWSKILLS], 12);
    assert_eq!(w.skills[&TP], 12);
    assert_eq!(w.log, [format!("learn {TP}"), format!("0x22 {TP} 12")]);
    // Present: + q.
    assert_eq!(item_skill_link(&mut w, U, it(1), true), Ok(true));
    assert_eq!(w.skills[&TP], 24);
    // Unlink: − q, floored at 0.
    w.skills.insert(TP, 5);
    assert_eq!(item_skill_link(&mut w, U, it(1), false), Ok(true));
    assert_eq!(w.skills[&TP], 0);
    // Unlink of an absent skill, learn that adds nothing: fatal.
    w.skills.clear();
    assert_eq!(
        item_skill_link(&mut w, U, it(1), false),
        Err(EquipFatal::UnlinkSkillMissing(TP))
    );
    w.unlearnable.push(TP);
    assert_eq!(
        item_skill_link(&mut w, U, it(1), true),
        Err(EquipFatal::LearnedSkillMissing(TP))
    );
}

#[test]
fn item_skill_unlink_to_zero_deselects_both_sides() {
    let mut w = W::player();
    w.add(it(1), tome(3));
    w.skills.insert(TP, 3);
    w.mouse = [Some((TP, -1)), Some((TP, -1))];
    assert_eq!(item_skill_link(&mut w, U, it(1), false), Ok(true));
    assert_eq!(w.mouse, [Some((0, -1)), Some((0, -1))]);
    // Not zero: the selection stays.
    w.skills.insert(TP, 10);
    w.mouse = [Some((TP, -1)), None];
    item_skill_link(&mut w, U, it(1), false).unwrap();
    assert_eq!(w.mouse, [Some((TP, -1)), None]);
}

#[test]
fn cube_recount_zeroes_then_relinks_page_zero_items() {
    let mut w = W::player();
    w.skills.insert(TP, 99);
    // Page 0 tome (5), stash tome (7), page 0 scroll (2).
    w.add(
        it(1),
        It {
            page: page::INVENTORY,
            ..tome(5)
        },
    );
    w.add(
        it(2),
        It {
            page: page::STASH,
            ..tome(7)
        },
    );
    w.add(
        it(3),
        It {
            page: page::INVENTORY,
            ..scroll(2)
        },
    );
    cube_recount(&mut w, U).unwrap();
    // The stash tome stops counting.
    assert_eq!(w.skills[&TP], 7);
}

fn equipped(usable: bool) -> It {
    It {
        mode: mode::EQUIPPED,
        usable,
        ..It::default()
    }
}

#[test]
fn inventory_pass_only_for_players_and_flag_0x200_units() {
    let mut w = W::player();
    w.unit_ty = 1;
    inventory_pass(&mut w, U, true);
    assert!(w.log.is_empty());
    w.unit_flags = UNIT_FLAG_200;
    inventory_pass(&mut w, U, true);
    assert_eq!(w.log, ["owner refresh", "0x48"]);
    // Without an inventory: nothing after step 1.
    let mut w = W::player();
    w.inv = false;
    inventory_pass(&mut w, U, true);
    assert!(w.log.is_empty());
    // send = 0: no 0x48.
    let mut w = W::player();
    inventory_pass(&mut w, U, false);
    assert_eq!(w.log, ["owner refresh"]);
}

#[test]
fn inventory_pass_links_usable_active_charms() {
    let mut w = W::player();
    let charm = It {
        node: node::PAGE,
        mode: mode::STORED,
        active: true,
        usable: true,
        ..It::default()
    };
    w.add(it(1), charm.clone());
    w.add(
        it(2),
        It {
            usable: false,
            ..charm.clone()
        },
    );
    w.add(
        it(3),
        It {
            node: node::BELT,
            ..charm
        },
    );
    inventory_pass(&mut w, U, false);
    assert_eq!(
        w.log,
        [
            format!("flag {} 0x4000 false", it(1).0),
            format!("link {}", it(1).0),
            format!("refresh {}", it(1).0),
            "owner refresh".into()
        ]
    );
}

#[test]
fn inventory_pass_switches_unusable_off_and_usable_on() {
    let mut w = W::player();
    // Head: linked, unusable → off; torso: flagged, usable → on.
    w.add(it(1), equipped(false));
    w.add(
        it(2),
        It {
            flags: iflag::F4000,
            ..equipped(true)
        },
    );
    // Broken and not linked: left alone by both sweeps.
    w.add(
        it(3),
        It {
            flags: iflag::BROKEN,
            ..equipped(false)
        },
    );
    w.body.insert(body::HEAD, it(1));
    w.body.insert(body::TORSO, it(2));
    w.body.insert(body::FEET, it(3));
    w.linked.push(it(1));
    inventory_pass(&mut w, U, false);
    assert_eq!(w.items[&it(1)].flags & iflag::F4000, iflag::F4000);
    assert!(!w.linked.contains(&it(1)));
    assert_eq!(w.items[&it(2)].flags & iflag::F4000, 0);
    assert!(w.linked.contains(&it(2)));
    assert!(!w.linked.contains(&it(3)));
    assert_eq!(
        w.log,
        [
            format!("flag {} 0x4000 true", it(1).0),
            format!("unlink {}", it(1).0),
            format!("deactivate {}", it(1).0),
            format!("flag {} 0x4000 false", it(2).0),
            format!("link {}", it(2).0),
            format!("refresh {}", it(2).0),
            "owner refresh".into()
        ]
    );
}

#[test]
fn inventory_pass_reapplies_set_items() {
    let mut w = W::player();
    w.add(
        it(1),
        It {
            quality: QUALITY_SET,
            ..equipped(true)
        },
    );
    w.add(
        it(2),
        It {
            quality: QUALITY_SET,
            flags: iflag::BROKEN,
            ..equipped(true)
        },
    );
    w.body.insert(body::GLOVES, it(1));
    w.body.insert(body::BELT, it(2));
    w.linked.extend([it(1), it(2)]);
    inventory_pass(&mut w, U, false);
    assert_eq!(
        w.log,
        [
            format!("deactivate {}", it(1).0),
            format!("refresh {}", it(1).0),
            "owner refresh".into()
        ]
    );
}

#[test]
fn inventory_pass_restores_the_saved_mouse_skills() {
    let mut w = W::player();
    w.skills.insert(36, 1);
    w.skills.insert(37, 1);
    w.mouse = [Some((36, -1)), Some((37, -1))];
    // A switch-off changes the selection during the pass.
    w.add(it(1), equipped(false));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.linked.push(it(1));
    inventory_pass(&mut w, U, false);
    // Already selected: nothing to restore.
    assert!(!w.log.iter().any(|l| l.starts_with("select")));
    // Changed selection: the left one comes back, the unusable right
    // one does not.
    let mut w = W::player();
    w.skills.insert(36, 1);
    w.skills.insert(37, 1);
    w.mouse = [Some((36, -1)), Some((37, -1))];
    w.unusable.push((37, -1));
    // Run the pass on a world whose mouse skills change after step 1:
    // emulate by a world that deselects on the first deactivate.
    struct Switching(W);
    impl EquipWorld for Switching {
        fn unit_type(&self, u: UnitId) -> Option<u8> {
            self.0.unit_type(u)
        }
        fn unit_flags(&self, u: UnitId) -> u32 {
            self.0.unit_flags(u)
        }
        fn has_inventory(&self, u: UnitId) -> bool {
            self.0.has_inventory(u)
        }
        fn item_list(&self, u: UnitId) -> Vec<UnitId> {
            self.0.item_list(u)
        }
        fn body_item(&self, u: UnitId, l: u8) -> Option<UnitId> {
            self.0.body_item(u, l)
        }
        fn weapon_in_use(&self, u: UnitId) -> Option<UnitId> {
            self.0.weapon_in_use(u)
        }
        fn add_unit_stat(&mut self, u: UnitId, s: u16, d: i32) {
            self.0.add_unit_stat(u, s, d)
        }
        fn item_type(&self, i: UnitId) -> i16 {
            self.0.item_type(i)
        }
        fn item_is_type(&self, i: UnitId, t: i16) -> bool {
            self.0.item_is_type(i, t)
        }
        fn item_flags(&self, i: UnitId) -> u32 {
            self.0.item_flags(i)
        }
        fn set_item_flag(&mut self, i: UnitId, b: u32, on: bool) {
            self.0.set_item_flag(i, b, on)
        }
        fn item_mode(&self, i: UnitId) -> u8 {
            self.0.item_mode(i)
        }
        fn item_node(&self, i: UnitId) -> u8 {
            self.0.item_node(i)
        }
        fn item_page(&self, i: UnitId) -> u8 {
            self.0.item_page(i)
        }
        fn item_quality(&self, i: UnitId) -> u8 {
            self.0.item_quality(i)
        }
        fn item_stat(&self, i: UnitId, s: u16) -> i32 {
            self.0.item_stat(i, s)
        }
        fn book_skill(&self, i: UnitId, s: bool) -> Option<i32> {
            self.0.book_skill(i, s)
        }
        fn active_inventory_item(&self, u: UnitId, i: UnitId) -> bool {
            self.0.active_inventory_item(u, i)
        }
        fn usable(&self, u: UnitId, i: UnitId) -> bool {
            self.0.usable(u, i)
        }
        fn stat_linked(&self, u: UnitId, i: UnitId) -> bool {
            self.0.stat_linked(u, i)
        }
        fn stat_link(&mut self, u: UnitId, i: UnitId) {
            self.0.stat_link(u, i)
        }
        fn stat_unlink(&mut self, u: UnitId, i: UnitId) {
            self.0.stat_unlink(u, i)
        }
        fn deactivate(&mut self, i: UnitId, u: UnitId) {
            self.0.mouse = [Some((0, -1)), Some((0, -1))];
            self.0.deactivate(i, u)
        }
        fn stat_refresh(&mut self, i: UnitId, u: UnitId) {
            self.0.stat_refresh(i, u)
        }
        fn owner_refresh(&mut self, u: UnitId) {
            self.0.owner_refresh(u)
        }
        fn send_unit_refresh(&mut self, u: UnitId) {
            self.0.send_unit_refresh(u)
        }
        fn skill_quantity(&self, u: UnitId, s: i32) -> Option<i32> {
            self.0.skill_quantity(u, s)
        }
        fn set_skill_quantity(&mut self, u: UnitId, s: i32, q: i32) {
            self.0.set_skill_quantity(u, s, q)
        }
        fn learn_skill(&mut self, u: UnitId, s: i32) {
            self.0.learn_skill(u, s)
        }
        fn send_skill_quantity(&mut self, u: UnitId, s: i32, q: i32) {
            self.0.send_skill_quantity(u, s, q)
        }
        fn mouse_skill(&self, u: UnitId, l: bool) -> Option<SkillRef> {
            self.0.mouse_skill(u, l)
        }
        fn select_skill(&mut self, u: UnitId, l: bool, s: SkillRef) {
            self.0.select_skill(u, l, s)
        }
        fn has_skill(&self, u: UnitId, s: SkillRef) -> bool {
            self.0.has_skill(u, s)
        }
        fn use_state(&mut self, u: UnitId, s: SkillRef) -> u8 {
            self.0.use_state(u, s)
        }
        fn throw_skill_row(&self, s: i32) -> bool {
            self.0.throw_skill_row(s)
        }
        fn saved_mouse_skill(&self, u: UnitId, l: bool) -> SkillRef {
            self.0.saved_mouse_skill(u, l)
        }
        fn set_saved_mouse_skill(&mut self, u: UnitId, l: bool, s: SkillRef) {
            self.0.set_saved_mouse_skill(u, l, s)
        }
    }
    w.add(it(1), equipped(false));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.linked.push(it(1));
    let mut s = Switching(w);
    inventory_pass(&mut s, U, false);
    assert_eq!(s.0.mouse, [Some((36, -1)), Some((0, -1))]);
    assert!(s.0.log.contains(&"select L (36, -1)".to_string()));
}

fn weapon(is: &[i16]) -> It {
    It {
        mode: mode::EQUIPPED,
        is: is.to_vec(),
        ..It::default()
    }
}

#[test]
fn weapon_bookkeeping_selects_throw_for_a_throw_only_weapon() {
    // Example (a): a throwing potion (thro, not mele) in the weapon hand,
    // left skill Attack → Attack saved, Throw selected and usable.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP, TYPE_THRO]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.weapon = Some(it(1));
    w.mouse = [Some((0, -1)), Some((0, -1))];
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse[0], Some((SKILL_THROW, -1)));
    assert_eq!(w.saved[0], (0, -1));
    // A melee throwing weapon (comb): not throw-only, nothing.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP, TYPE_THRO, TYPE_MELE]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.weapon = Some(it(1));
    w.mouse = [Some((0, -1)), Some((0, -1))];
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse[0], Some((0, -1)));
    // A ranged throw skill already on the left: kept.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP, TYPE_THRO]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.weapon = Some(it(1));
    w.skills.insert(140, 1);
    w.throw_rows.push(140);
    w.mouse = [Some((140, -1)), Some((0, -1))];
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse[0], Some((140, -1)));
    // Not a player, or no weapon in use (example b, edge case 14):
    // nothing.
    let mut w = W::player();
    w.mouse = [Some((SKILL_THROW, -1)), Some((0, -1))];
    w.unusable.push((SKILL_THROW, -1));
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse[0], Some((SKILL_THROW, -1)));
    w.unit_ty = 1;
    w.weapon = Some(it(9));
    weapon_bookkeeping(&mut w, U).unwrap();
    assert!(w.log.is_empty());
}

#[test]
fn weapon_bookkeeping_restores_an_unusable_left_skill() {
    // Example (c): a sword in use, left skill Throw unusable → the saved
    // Attack comes back.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP, TYPE_MELE]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.weapon = Some(it(1));
    w.mouse = [Some((SKILL_THROW, -1)), Some((0, -1))];
    w.saved = [(0, -1), (0, -1)];
    w.unusable.push((SKILL_THROW, -1));
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse[0], Some((0, -1)));
    // No mouse skill: the mouse-skill getter's fatal assert.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.weapon = Some(it(1));
    assert_eq!(weapon_bookkeeping(&mut w, U), Err(EquipFatal::NoMouseSkill));
}

#[test]
fn weapon_bookkeeping_right_side_uses_the_other_hand_weapon() {
    // W in the right hand, a throw-only weapon in the left: Left Hand
    // Throw on the right; a non-weapon in the other hand is no O.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP, TYPE_MELE]));
    w.add(it(2), weapon(&[TYPE_WEAP, TYPE_THRO]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.body.insert(body::LEFT_HAND, it(2));
    w.weapon = Some(it(1));
    w.skills.insert(SKILL_LEFT_HAND_THROW, 1);
    w.mouse = [Some((0, -1)), Some((0, -1))];
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse, [Some((0, -1)), Some((SKILL_LEFT_HAND_THROW, -1))]);
    assert_eq!(w.saved[1], (0, -1));
    // The same item as a shield-type (not `weap`): no O, right kept.
    let mut w = W::player();
    w.add(it(1), weapon(&[TYPE_WEAP, TYPE_MELE]));
    w.add(it(2), weapon(&[TYPE_THRO]));
    w.body.insert(body::RIGHT_HAND, it(1));
    w.body.insert(body::LEFT_HAND, it(2));
    w.weapon = Some(it(1));
    w.mouse = [Some((0, -1)), Some((0, -1))];
    weapon_bookkeeping(&mut w, U).unwrap();
    assert_eq!(w.mouse[1], Some((0, -1)));
}

/// The tome's skill quantity += n (not the tome's own
/// quantity), S→C 0x22.
#[test]
fn item_skill_add_adds_n_to_the_tomes_skill() {
    let mut w = W::player();
    w.add(it(1), tome(12));
    w.skills.insert(TP, 12);
    assert_eq!(item_skill_add(&mut w, U, it(1), 1), Ok(true));
    assert_eq!(w.skills[&TP], 13);
    assert_eq!(w.log, [format!("0x22 {TP} 13")]);
    // Not a tome or scroll: nothing.
    w.add(
        it(2),
        It {
            ty: 3,
            book: Some(TP),
            ..It::default()
        },
    );
    assert_eq!(item_skill_add(&mut w, U, it(2), 4), Ok(false));
    assert_eq!(w.skills[&TP], 13);
}
