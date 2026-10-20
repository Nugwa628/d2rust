//! Equipped weapon and ammo snapshots used by the server's skill pipeline.
//! [`sync`] copies facts from the wired host's inventory into [`Weapons`]
//! before intents and ticks. Weapon, grip, ammo, range and shield answers
//! then read the same snapshot, including monster equipment.

use std::collections::{BTreeMap, BTreeSet};

use crate::adapters::handlers::world::WiredWorld;
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::InvTables;
use d2_sim::items::inventory::{body, mode};
use d2_sim::items::moves::iflag;
use d2_sim::units::UnitId;
use d2_sim::wiring::worldgen::WorldSim;

use super::local_seams::LocalSeams;

/// Hand classes: 0 hand to hand, 1 bow, 3 one-hand thrust, 7 crossbow.
pub mod class {
    pub const HAND_TO_HAND: i32 = 0;
    pub const BOW: i32 = 1;
    pub const ONE_HAND_SWING: i32 = 2;
    pub const ONE_HAND_THRUST: i32 = 3;
    pub const CROSSBOW: i32 = 7;
}

/// What the skill pipeline asks of one item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemFacts {
    /// The `itemtypes` rows the item is of (its own and its parents').
    pub types: Vec<i16>,
    /// The hand class when held in the right hand.
    pub class: i32,
    /// The type has a `shoots` value (bows, crossbows).
    pub shoots: bool,
    /// The primary type's `shoots` row, or zero without ammo.
    pub ammo: i16,
    pub stackable: bool,
    pub max_stack: i32,
    /// Grip: 2 for a two-handed base item, otherwise 1.
    pub grip: i32,
    /// Whether the item allows either one or two hands (`1or2handed`).
    pub one_or_two: bool,
    /// The item is unusable: broken (0x100) or flagged invalid (0x4000).
    pub invalid: bool,
    /// A shield (type `shld`), with its armor `mindam` / `maxdam` (D2RUST's
    /// own, unverified: q-skill-gaps).
    pub shield: bool,
    pub dam: (i32, i32),
    /// `component` (items), `wclass` / `2handedwclass` and "of type 45
    /// `weap`", for the COF weapon class.
    pub component: u8,
    pub wclass: [u8; 4],
    pub wclass2: [u8; 4],
    pub weap: bool,
    /// `rangeadder`: the player's melee reach.
    pub range_adder: i32,
    /// The weapon's hit class (combat/damage.md section 5.1).
    pub hit_class: u8,
    /// `StrBonus` / `DexBonus`.
    pub str_bonus: i32,
    pub dex_bonus: i32,
    /// Positive row durability with `nodurability` clear.
    pub breakable: bool,
    /// The base item's `missiletype` used by throwing skills.
    pub missile_type: i32,
    /// Whether the item's own type has `Throwable` set.
    pub throwable: bool,
}

/// A player's hands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hands {
    pub right: Option<UnitId>,
    pub left: Option<UnitId>,
    /// The weapon in use: the right-hand item with a hand class.
    pub weapon: Option<UnitId>,
    /// The inventory's selected weapon in use, updated by body link and unlink.
    pub in_use: Option<UnitId>,
    /// The COF weapon class for animation names (render/unit-composite.md section 2.1).
    pub cof: i32,
    /// Items on body locations 0-11, used by skill equipment checks.
    pub worn: [Option<UnitId>; 12],
    /// Whether the unit supports dual wield: player class 4 or 6.
    pub dual: bool,
}

/// The copy of the inventory model's weapon facts ([`sync`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Weapons {
    pub hands: BTreeMap<UnitId, Hands>,
    pub items: BTreeMap<UnitId, ItemFacts>,
    /// The items that pass the Iron Golem item test ([`golem_item_ok`]).
    pub golem: BTreeSet<UnitId>,
}

impl Weapons {
    pub fn weapon(&self, u: UnitId) -> Option<UnitId> {
        self.hands.get(&u)?.weapon
    }

    /// The item on body location 4 (right hand) or 5 (left hand).
    pub fn item_at(&self, u: UnitId, loc: u8) -> Option<UnitId> {
        let h = self.hands.get(&u)?;
        match loc {
            body::RIGHT_HAND => h.right,
            body::LEFT_HAND => h.left,
            _ => h.worn.get(usize::from(loc)).copied().flatten(),
        }
    }

    /// Pick the damage weapon for the skill's weapon selection, frame event
    /// index and hand flag (skills/bodies-2.md section 2.27).
    pub fn damage_weapon(&self, u: UnitId, weapsel: u8, idx: i32, flag: bool) -> Option<UnitId> {
        let h = self.hands.get(&u)?;
        let is_weap = |i: Option<UnitId>| i.filter(|i| self.items.get(i).is_some_and(|f| f.weap));
        let d = is_weap(h.right).or_else(|| is_weap(h.left));
        if !h.dual {
            return d;
        }
        // q(k): the first equipped item of a swing class (2, 3, 12) that is
        // (k = 5) / is not (k = 6) the weapon in use; for other items, the
        // one whose `component` is k.
        let q = |k: u8| {
            (0..12u8).find_map(|loc| {
                let i = self.item_at(u, loc)?;
                let f = self.items.get(&i)?;
                let ok = if matches!(type_class(f), 2 | 3 | 12) {
                    (k == 5) == (Some(i) == h.weapon)
                } else {
                    f.component == k
                };
                ok.then_some(i)
            })
        };
        let x = match weapsel {
            1 => q(6),
            2 => {
                let (a, b) = (q(5), q(6));
                if is_weap(a).is_none() || (is_weap(b).is_some() && flag) {
                    b
                } else {
                    a
                }
            }
            3 => {
                if idx % 2 != 0 {
                    q(6)
                } else {
                    q(5)
                }
            }
            4 => return None,
            _ => is_weap(q(5)).or_else(|| q(6)),
        };
        is_weap(x).or(d)
    }

    /// The COF weapon class, or hand to hand (0) without equipped hands.
    pub fn cof_class(&self, u: UnitId) -> i32 {
        self.hands.get(&u).map_or(0, |h| h.cof)
    }

    /// The hand class of the weapon in use (0 without one).
    pub fn hand_class(&self, u: UnitId) -> i32 {
        self.weapon(u)
            .and_then(|w| self.items.get(&w))
            .map_or(class::HAND_TO_HAND, |f| f.class)
    }

    /// The selected weapon's hit class, or 1 without a weapon.
    pub fn weapon_hit_class(&self, u: UnitId) -> u32 {
        self.weapon(u)
            .and_then(|w| self.items.get(&w))
            .map_or(1, |f| u32::from(f.hit_class))
    }

    /// The selected weapon's `rangeadder`, or zero without a weapon.
    pub fn range_adder(&self, u: UnitId) -> i32 {
        self.weapon(u)
            .and_then(|w| self.items.get(&w))
            .map_or(0, |f| f.range_adder)
    }

    /// The equipped left-hand shield.
    pub fn shield(&self, u: UnitId) -> Option<UnitId> {
        let l = self.hands.get(&u)?.left?;
        self.items.get(&l)?.shield.then_some(l)
    }

    /// The item's weapon type class, or zero when absent from the snapshot.
    pub fn type_class(&self, item: UnitId) -> u32 {
        self.items.get(&item).map_or(0, |f| type_class(f) as u32)
    }

    pub fn facts(&self, item: UnitId) -> ItemFacts {
        self.items.get(&item).cloned().unwrap_or_default()
    }

    /// Whether the item qualifies for Iron Golem (skills/bodies-2b.md section 7.11).
    pub fn golem_item(&self, item: UnitId) -> bool {
        self.golem.contains(&item)
    }

    /// Whether the item has the requested type.
    pub fn item_is(&self, item: UnitId, itype: i32) -> bool {
        self.items
            .get(&item)
            .is_some_and(|f| i16::try_from(itype).is_ok_and(|t| f.types.contains(&t)))
    }
}

/// The `itemtypes` rows whose code is one of `codes` (trailing spaces and
/// NULs of the 4-byte code ignored).
fn type_rows(t: &InvTables, codes: &[&[u8]]) -> Vec<i16> {
    t.itemtypes
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            let c = r.code.split(|&b| b == 0 || b == b' ').next().unwrap_or(&[]);
            codes.contains(&c)
        })
        .map(|(i, _)| i as i16)
        .collect()
}

fn is_any(t: &InvTables, record: usize, rows: &[i16]) -> bool {
    rows.iter().any(|&r| t.is_type(record, r))
}

/// The facts of an item of items-row `record`.
// D2RUST's own, unverified (REC-150): see the module docs.
pub fn facts_of(t: &InvTables, record: usize) -> ItemFacts {
    let bow = type_rows(t, &[b"bow"]);
    let xbow = type_rows(t, &[b"xbow"]);
    let jave = type_rows(t, &[b"jave", b"ajav"]);
    let weap = type_rows(t, &[b"weap"]);
    let shld = type_rows(t, &[b"shld"]);
    let (class, shoots) = if is_any(t, record, &bow) {
        (class::BOW, true)
    } else if is_any(t, record, &xbow) {
        (class::CROSSBOW, true)
    } else if is_any(t, record, &jave) {
        (class::ONE_HAND_THRUST, false)
    } else if is_any(t, record, &weap) {
        (class::ONE_HAND_SWING, false)
    } else {
        (class::HAND_TO_HAND, false)
    };
    let grip = if t.item(record).is_some_and(|r| r.twohanded != 0) {
        2
    } else {
        1
    };
    let (stackable, max_stack) = t
        .item(record)
        .map_or((false, 0), |r| (r.stackable != 0, r.maxstack as i32));
    let types = (0..t.itemtypes.len() as i16)
        .filter(|&row| t.is_type(record, row))
        .collect();
    ItemFacts {
        types,
        class,
        shoots,
        ammo: 0,
        stackable,
        max_stack,
        grip,
        one_or_two: t.item(record).is_some_and(|r| r.onetwohanded != 0),
        invalid: false,
        shield: is_any(t, record, &shld),
        dam: t
            .item(record)
            .map_or((0, 0), |r| (i32::from(r.mindam), i32::from(r.maxdam))),
        component: t.item(record).map_or(0, |r| r.component),
        wclass: t.item(record).map_or([0; 4], |r| r.wclass),
        wclass2: t.item(record).map_or([0; 4], |r| r.wclass2),
        weap: is_any(t, record, &weap),
        range_adder: t.item(record).map_or(0, |r| i32::from(r.rangeadder)),
        hit_class: t.item(record).map_or(0, |r| r.hit_class),
        str_bonus: t.item(record).map_or(0, |r| i32::from(r.strbonus)),
        dex_bonus: t.item(record).map_or(0, |r| i32::from(r.dexbonus)),
        breakable: t
            .item(record)
            .is_some_and(|r| r.durability > 0 && r.nodurability == 0),
        missile_type: t.item(record).map_or(0, |r| i32::from(r.missiletype)),
        throwable: t
            .item(record)
            .and_then(|r| t.itemtypes.get(usize::try_from(r.type_).ok()?))
            .is_some_and(|ty| ty.throwable != 0),
    }
}

/// The index of a COF weapon class code (`sequences::CLASSES`; trailing
/// spaces and NULs ignored); unknown → `hth` (0).
fn class_index(code: &[u8; 4]) -> i32 {
    let c = code.split(|&b| b == 0 || b == b' ').next().unwrap_or(&[]);
    d2_sim::skills::sequences::CLASSES
        .iter()
        .position(|k| k.as_bytes() == c)
        .map_or(0, |i| i as i32)
}

/// The weapon class index from the item's `wclass`, or zero.
fn type_class(f: &ItemFacts) -> i32 {
    match class_index(&f.wclass) {
        c @ (1..=7 | 12) => c,
        _ => 0,
    }
}

/// Resolve hand grip for the player class. Either hand may provide a
/// one-or-two-handed item; otherwise use the base item's grip.
fn hands_grip(hands: &Hands, items: &BTreeMap<UnitId, ItemFacts>, class: u32) -> i32 {
    let fact = |i: Option<UnitId>| i.and_then(|i| items.get(&i));
    let barb = |f: &ItemFacts| class == 4 && f.one_or_two;
    match (fact(hands.right), fact(hands.left)) {
        (Some(r), Some(l)) if barb(l) || barb(r) => 1,
        (Some(r), Some(l)) if l.grip == 2 || r.grip == 2 => 2,
        (Some(_), Some(_)) | (None, None) => -1,
        (Some(f), None) | (None, Some(f)) if f.grip == 2 || barb(f) => 2,
        _ => -1,
    }
}

/// Resolve the player's COF weapon class outside death modes. The player
/// class and valid equipped items determine the animation class.
fn cof_class(hands: &Hands, items: &BTreeMap<UnitId, ItemFacts>, class: u32) -> i32 {
    // Only usable hand items participate in the class selection.
    let fact =
        |i: Option<UnitId>| i.and_then(|i| items.get(&i).filter(|f| !f.invalid).map(|f| (i, f)));
    let (right, left) = (fact(hands.right), fact(hands.left));
    let hand = right.filter(|(_, f)| matches!(f.component, 5 | 6)).or(left);
    let Some((_, h)) = hand.filter(|(_, f)| matches!(f.component, 5 | 6)) else {
        return 0;
    };
    if let (Some((ri, rf)), Some((_, lf))) = (right, left) {
        if rf.weap && lf.weap {
            match class {
                4 => {
                    // r-dual: A the weapon in use (else the right hand).
                    let a_id = hands.weapon.unwrap_or(ri);
                    let (a, b) = if a_id == ri { (rf, lf) } else { (lf, rf) };
                    return class_index(match (type_class(a), type_class(b)) {
                        (2, 3) => b"1js ",
                        (3, 3) => b"1jt ",
                        (3, 2) => b"1st ",
                        _ => b"1ss ",
                    });
                }
                6 => return class_index(b"ht2 "),
                _ => {}
            }
        }
    }
    if hands_grip(hands, items, class) == 2 {
        class_index(&h.wclass2)
    } else {
        class_index(&h.wclass)
    }
}

/// [`cof_class`] of a player of `class` holding `right` (body location
/// 4) and `left` (5), the weapon in use being the right-hand item with a
/// hand class: the client model's reading of the same items
/// (the client animation-name lookup).
pub fn cof_class_of(right: Option<ItemFacts>, left: Option<ItemFacts>, class: u32) -> i32 {
    let (r, l) = (UnitId(0), UnitId(1));
    let mut items = BTreeMap::new();
    let weapon = right
        .as_ref()
        .filter(|f| f.class != class::HAND_TO_HAND)
        .map(|_| r);
    let hands = Hands {
        right: right.map(|f| {
            items.insert(r, f);
            r
        }),
        left: left.map(|f| {
            items.insert(l, f);
            l
        }),
        weapon,
        in_use: None,
        cof: 0,
        worn: [None; 12],
        dual: false,
    };
    cof_class(&hands, &items, class)
}

/// Iron Golem requires a ground item, identified, metallic, and inactive
/// on any unit (skills/bodies-2b.md section 7.11).
fn golem_item_ok(t: &InvTables, unit_mode: u32, flags: u32, record: usize) -> bool {
    unit_mode == u32::from(mode::GROUND)
        && flags & iflag::IDENTIFIED != 0
        && t.item(record).is_some_and(|r| r.bitfield1 & 0x2 != 0)
}

/// The play host's world sync ([`crate::adapters::SimGame::set_world_sync`]):
/// the hands of every player and the facts of the items in them.
pub fn sync<R, S>(game: &Game, sim: &mut WorldSim<LocalSeams>, world: &mut WiredWorld<R, S>) {
    let Some(inv) = world.inventory.as_ref() else {
        return;
    };
    let mut w = Weapons::default();
    for (&owner, inventory) in &inv.state.inventories {
        let mut hands = Hands::default();
        for &item in inventory.items() {
            let Some(d) = inv.state.items.get(&item) else {
                continue;
            };
            if d.mode != mode::EQUIPPED || d.inv != Some(owner) {
                continue;
            }
            let slot = match d.body_loc {
                body::RIGHT_HAND => &mut hands.right,
                body::LEFT_HAND => &mut hands.left,
                l => match hands.worn.get_mut(usize::from(l)) {
                    Some(s) => s,
                    None => continue,
                },
            };
            *slot = Some(item);
            let mut f = facts_of(&inv.tables, d.record);
            f.invalid = d.flags & (iflag::BROKEN | iflag::NOEQUIP) != 0;
            f.ammo = world
                .tables
                .itype_of(d.record)
                .map_or(0, |t| t.shoots as i16);
            w.items.insert(item, f);
        }
        hands.weapon = monster_weapon(&hands, &w.items);
        // Select the first usable weapon in body location order: right, then left.
        hands.in_use = [hands.right, hands.left].into_iter().flatten().find(|i| {
            inv.state
                .items
                .get(i)
                .is_some_and(|d| d.guid == inventory.weapon_guid)
                && w.items.get(i).is_some_and(|f| f.weap)
        });
        let class = sim.action.sys.units.get(owner).map_or(0, |u| u.class);
        hands.cof = cof_class(&hands, &w.items, class);
        hands.dual = matches!(class, 4 | 6);
        // Every inventory owner has the unit's inventory, also with
        // nothing worn (`has_inventory`: a bare-handed kick still runs its
        // `fill`).
        w.hands.insert(owner, hands);
    }
    w.golem = game
        .lists
        .units_of_type(d2_sim::units::UnitType::Item)
        .into_iter()
        .filter(|&i| {
            let m = sim.action.sys.units.get(i).map(|u| u.mode);
            match (m, sim.action.sys.hooks.items.get(i)) {
                (Some(m), Some(it)) => golem_item_ok(&inv.tables, m, it.flags, it.record),
                _ => false,
            }
        })
        .collect();
    // Copy monster equipment so skills and missiles use the same weapon facts.
    // The selected weapon must be usable and of type 45 (`weap`).
    for (&owner, held) in &sim.action.sys.hooks.monster_equip {
        let mut hands = Hands::default();
        for (&loc, &item) in held {
            let slot = match loc {
                body::RIGHT_HAND => &mut hands.right,
                body::LEFT_HAND => &mut hands.left,
                _ => continue,
            };
            let Some(record) = sim.action.sys.units.get(item).map(|u| u.class as usize) else {
                continue;
            };
            *slot = Some(item);
            w.items.insert(item, facts_of(&inv.tables, record));
        }
        hands.weapon = monster_weapon(&hands, &w.items);
        if hands.right.is_some() || hands.left.is_some() {
            w.hands.insert(owner, hands);
        }
    }
    // Snapshot monsters with a usable shield for the block check.
    let mut shielded = std::collections::BTreeSet::new();
    if let Some(looks) = sim.action.sys.hooks.x.looks.clone() {
        for &u in sim.action.sys.hooks.x.sides.keys() {
            let Some(r) = sim.action.sys.units.get(u) else {
                continue;
            };
            if r.ty != d2_sim::units::UnitType::Monster {
                continue;
            }
            let Some(mask) = looks.shield_choices(r.class) else {
                continue;
            };
            let v = sim
                .world
                .monsters
                .get(u)
                .map_or(0, |m| usize::from(m.components[7]));
            if v < 16 && mask & (1 << v) != 0 {
                shielded.insert(u);
            }
        }
    }
    sim.action.sys.hooks.x.shielded = shielded;
    if sim.action.sys.hooks.x.weapons != w {
        sim.action.sys.hooks.x.weapons = w;
    }
}

/// Pick the first usable weapon of type 45: right hand, then left hand.
fn monster_weapon(hands: &Hands, items: &BTreeMap<UnitId, ItemFacts>) -> Option<UnitId> {
    let pick = |i: Option<UnitId>| i.filter(|i| items.get(i).is_some_and(|f| f.weap && !f.invalid));
    pick(hands.right).or_else(|| pick(hands.left))
}

#[cfg(test)]
mod tests {
    use d2_data::fixup::maps::EquivMatrix;
    use d2_sim::items::inventory::tables::{InvItemRec, InvTypeRec};

    use super::*;

    /// Types: 0 `weap`, 1 `bow` (a `weap`), 2 `jave` (a `weap`), 3 `aqv`
    /// (arrows). Items: 0 a bow, 1 a javelin, 2 arrows, 3 a sword (a `weap`).
    fn tables() -> InvTables {
        let n = 5;
        let mut m = EquivMatrix {
            n,
            words: 1,
            bits: vec![0; n],
        };
        for i in 0..n {
            m.bits[i] |= 1 << i;
        }
        m.bits[1] |= 1;
        m.bits[2] |= 1;
        let ty = |c: &[u8; 4]| InvTypeRec {
            code: *c,
            ..InvTypeRec::default()
        };
        let it = |t: i16, stack: u8| InvItemRec {
            type_: t,
            stackable: stack,
            maxstack: if stack != 0 { 500 } else { 0 },
            ..InvItemRec::default()
        };
        InvTables {
            itemtypes: vec![
                ty(b"weap"),
                ty(b"bow\0"),
                ty(b"jave"),
                ty(b"aqv\0"),
                ty(b"misc"),
            ],
            items: vec![it(1, 0), it(2, 0), it(3, 1), it(0, 0)],
            equiv: m,
            ..InvTables::default()
        }
    }

    #[test]
    fn a_bow_is_hand_class_1_and_shoots() {
        let f = facts_of(&tables(), 0);
        assert_eq!((f.class, f.shoots), (class::BOW, true));
        assert!(f.types.contains(&1) && f.types.contains(&0));
    }

    #[test]
    fn a_javelin_is_a_thrusting_weapon_that_does_not_shoot() {
        let f = facts_of(&tables(), 1);
        assert_eq!((f.class, f.shoots), (class::ONE_HAND_THRUST, false));
    }

    #[test]
    fn arrows_are_a_stack_and_no_weapon() {
        let f = facts_of(&tables(), 2);
        assert_eq!(f.class, class::HAND_TO_HAND);
        assert!(f.stackable);
        assert_eq!(f.max_stack, 500);
    }

    #[test]
    fn a_monster_weapon_is_the_right_hand_weap_else_the_left() {
        let t = tables();
        let (bow, arrows, sword) = (UnitId(7), UnitId(8), UnitId(9));
        let mut items = BTreeMap::new();
        items.insert(bow, facts_of(&t, 0));
        items.insert(arrows, facts_of(&t, 2));
        items.insert(sword, facts_of(&t, 3));
        let hands = |right, left| Hands {
            right,
            left,
            ..Hands::default()
        };
        // Location 4 precedes 5; arrows are not weapons.
        assert_eq!(
            monster_weapon(&hands(Some(sword), Some(bow)), &items),
            Some(sword)
        );
        assert_eq!(
            monster_weapon(&hands(Some(arrows), Some(bow)), &items),
            Some(bow)
        );
        assert_eq!(monster_weapon(&hands(None, Some(bow)), &items), Some(bow));
        assert_eq!(monster_weapon(&hands(Some(arrows), None), &items), None);
        // An unusable item is skipped.
        items.get_mut(&sword).unwrap().invalid = true;
        assert_eq!(
            monster_weapon(&hands(Some(sword), Some(bow)), &items),
            Some(bow)
        );
    }

    #[test]
    fn any_other_weapon_swings() {
        let t = tables();
        assert_eq!(facts_of(&t, 3).class, class::ONE_HAND_SWING);
    }

    #[test]
    fn the_hand_class_follows_the_weapon_in_use() {
        let t = tables();
        let (bow, quiver) = (UnitId(7), UnitId(8));
        let mut w = Weapons::default();
        w.items.insert(bow, facts_of(&t, 0));
        w.items.insert(quiver, facts_of(&t, 2));
        let p = UnitId(1);
        w.hands.insert(
            p,
            Hands {
                right: Some(bow),
                left: Some(quiver),
                weapon: Some(bow),
                ..Hands::default()
            },
        );
        assert_eq!(w.hand_class(p), class::BOW);
        assert_eq!(w.item_at(p, body::LEFT_HAND), Some(quiver));
        assert_eq!(w.item_at(p, body::HEAD), None);
        assert!(w.item_is(bow, 1) && !w.item_is(quiver, 1));
        assert_eq!(w.hand_class(UnitId(2)), class::HAND_TO_HAND);
    }

    #[test]
    fn throw_reads_the_inventory_weapon_in_use_and_the_type_throwable() {
        use d2_sim::wiring::action::Pending;
        let mut t = tables();
        t.itemtypes[2].throwable = 1;
        assert!(facts_of(&t, 1).throwable, "jave type: throwable");
        assert!(!facts_of(&t, 0).throwable, "bow type: not throwable");
        let (knife, p) = (UnitId(7), UnitId(1));
        let mut seams = LocalSeams::default();
        assert_eq!(seams.weapon_in_use(p), None, "no hands");
        seams.weapons.items.insert(knife, facts_of(&t, 1));
        seams.weapons.hands.insert(
            p,
            Hands {
                right: Some(knife),
                in_use: Some(knife),
                ..Hands::default()
            },
        );
        assert_eq!(seams.weapon_in_use(p), Some(knife));
        assert!(seams.item_flag_throw(knife));
        assert!(!seams.item_flag_throw(UnitId(9)), "unknown item");
    }

    #[test]
    fn combat_reads_the_weapon_in_use() {
        use d2_sim::wiring::action::Pending;
        let t = tables();
        let (sword, p) = (UnitId(7), UnitId(1));
        let mut seams = LocalSeams::default();
        assert_eq!(seams.current_weapon(p), None, "bare hands");
        let w = &mut seams.weapons;
        w.items.insert(sword, facts_of(&t, 3));
        w.hands.insert(
            p,
            Hands {
                right: Some(sword),
                left: None,
                weapon: Some(sword),
                ..Hands::default()
            },
        );
        assert_eq!(seams.current_weapon(p), Some(sword));
        assert_eq!(seams.weapon(p), Some(sword));
        assert_eq!(seams.item_at(p, body::RIGHT_HAND), Some(sword));
        assert_eq!(seams.wield_type(sword), 1);
        assert_eq!(seams.current_weapon(UnitId(2)), None);
    }

    #[test]
    fn the_cof_class_follows_the_hand_items() {
        let claw = ItemFacts {
            component: 5,
            wclass: *b"ht1 ",
            wclass2: *b"ht1 ",
            weap: true,
            grip: 1,
            ..ItemFacts::default()
        };
        let axe = ItemFacts {
            wclass: *b"1hs ",
            ..claw.clone()
        };
        let buckler = ItemFacts {
            component: 7,
            ..ItemFacts::default()
        };
        let (a, b) = (UnitId(5), UnitId(6));
        let both = Hands {
            right: Some(a),
            left: Some(b),
            weapon: Some(a),
            in_use: None,
            cof: 0,
            worn: [None; 12],
            dual: false,
        };
        let items = |r: &ItemFacts, l: &ItemFacts| BTreeMap::from([(a, r.clone()), (b, l.clone())]);
        // One claw and a shield: its `wclass`.
        assert_eq!(cof_class(&both, &items(&claw, &buckler), 6), 12);
        // Two weapons: the assassin's `ht2`, the barbarian's r-dual
        // (1hs, 1hs) → `1ss`.
        assert_eq!(cof_class(&both, &items(&claw, &claw), 6), 13);
        assert_eq!(cof_class(&both, &items(&axe, &axe), 4), 10);
        // A two-handed grip: `2handedwclass`.
        let staff = ItemFacts {
            wclass: *b"stf ",
            wclass2: *b"2hs ",
            grip: 2,
            ..axe.clone()
        };
        let right = Hands { left: None, ..both };
        assert_eq!(cof_class(&right, &items(&staff, &buckler), 1), 5);
        // No hand item: the `charstats` class, `hth`.
        assert_eq!(cof_class(&Hands::default(), &BTreeMap::new(), 6), 0);
    }

    #[test]
    fn the_golem_item_is_a_metal_identified_ground_item() {
        let metal = |b| InvItemRec {
            bitfield1: b,
            ..InvItemRec::default()
        };
        let t = InvTables {
            items: vec![metal(3), metal(1)],
            ..InvTables::default()
        };
        let id = iflag::IDENTIFIED;
        let ground = u32::from(mode::GROUND);
        assert!(golem_item_ok(&t, ground, id, 0));
        // Leather (bitfield1 bit 1 clear), unidentified, not on the
        // ground, an unknown class: refused.
        assert!(!golem_item_ok(&t, ground, id, 1));
        assert!(!golem_item_ok(&t, ground, 0, 0));
        assert!(!golem_item_ok(&t, u32::from(mode::EQUIPPED), id, 0));
        assert!(!golem_item_ok(&t, ground, id, 2));
    }

    // Covers player grip and item validity in COF weapon-class selection.
    #[test]
    fn the_cof_grip_reads_both_hands_and_skips_unusable_items() {
        let bastard = ItemFacts {
            component: 5,
            wclass: *b"1hs ",
            wclass2: *b"2hs ",
            weap: true,
            grip: 2,
            one_or_two: true,
            ..ItemFacts::default()
        };
        let buckler = ItemFacts {
            component: 7,
            ..ItemFacts::default()
        };
        let claw = ItemFacts {
            component: 5,
            wclass: *b"ht1 ",
            wclass2: *b"ht1 ",
            weap: true,
            grip: 1,
            ..ItemFacts::default()
        };
        let (a, b) = (UnitId(5), UnitId(6));
        let both = Hands {
            right: Some(a),
            left: Some(b),
            weapon: Some(a),
            ..Hands::default()
        };
        let alone = Hands { left: None, ..both };
        let items = |r: &ItemFacts, l: &ItemFacts| BTreeMap::from([(a, r.clone()), (b, l.clone())]);
        // A barbarian's one-or-two-handed sword next to a shield: grip 1,
        // `wclass`; alone: grip 2, `2handedwclass`.
        assert_eq!(hands_grip(&both, &items(&bastard, &buckler), 4), 1);
        assert_eq!(cof_class(&both, &items(&bastard, &buckler), 4), 2);
        assert_eq!(cof_class(&alone, &items(&bastard, &buckler), 4), 5);
        // Another class: the sword is two-handed, grip 2.
        assert_eq!(cof_class(&both, &items(&bastard, &buckler), 1), 5);
        // A broken right-hand item is skipped: the left hand's shield has
        // no weapon component → `hth`.
        let broken = ItemFacts {
            invalid: true,
            ..bastard.clone()
        };
        assert_eq!(cof_class(&both, &items(&broken, &buckler), 4), 0);
        // Two claws, one unusable: no `ht2`, the usable claw's `wclass`.
        let bad_claw = ItemFacts {
            invalid: true,
            ..claw.clone()
        };
        assert_eq!(cof_class(&both, &items(&bad_claw, &claw), 6), 12);
    }
}
