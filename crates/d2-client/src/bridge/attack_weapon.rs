//! The client's weapon picks over the model's worn items, the inputs of
//! the animation rate (`bridge::player_anim`): the weapon
//! pick, the weapon in use, the attack weapon
//! with (U, 1) and the dual-wield stat toggle with (U, 1).
//!
//! The weapon in use is the weapon at body location 4, else 5, as the
//! character panel reads it (`ui::char_desc`;: the weapon-switch
//! slot is not told apart).

use super::items::{self, mode, ItemView};
use super::world::{ClientWorld, KindData, UnitKey, MONSTER, PLAYER};

/// Item type 45 `weap` (0x2D).
const WEAP: i16 = 45;

/// A player of class 4 or 6, a monster of class 417 or 418.
pub fn dual_capable(w: &ClientWorld, key: UnitKey) -> bool {
    let Some(u) = w.units.get(&key) else {
        return false;
    };
    match key.unit_type {
        PLAYER => matches!(u.class, 4 | 6),
        MONSTER => matches!(u.class, 417 | 418),
        _ => false,
    }
}

/// The item `key` wears at body location `loc` (the first in key order).
pub fn worn_at(w: &ClientWorld, key: UnitKey, loc: u8) -> Option<ItemView> {
    w.units
        .keys()
        .filter(|k| k.unit_type == items::ITEM)
        .filter_map(|&k| items::item(w, k))
        .find(|v| v.owner == Some(key) && v.mode == mode::BODY && v.body == loc && !v.store)
}

/// "Of type `weap`" (with equivalence).
fn is_weap(w: &ClientWorld, v: &ItemView) -> bool {
    match (&w.item_tables.0, v.code) {
        (Some(t), Some(code)) => t.item_is_type(code, WEAP),
        _ => false,
    }
}

/// Identified (F 0x10), not broken (F 0x100), item flag
/// 0x4000 clear.
fn usable(w: &ClientWorld, v: &ItemView) -> bool {
    let unlinked = match w.units.get(&v.key).map(|u| &u.kind) {
        Some(KindData::Item(d)) => d.flags & 0x4000 != 0,
        _ => true,
    };
    v.flags & 0x10 != 0 && v.flags & 0x100 == 0 && !unlinked
}

/// The weapon pick: the item at
/// location 4, else 5, that is usable and of type `weap`.
pub fn weapon_pick(w: &ClientWorld, key: UnitKey) -> Option<ItemView> {
    [4u8, 5]
        .into_iter()
        .find_map(|l| usable_weapon_at(w, key, l))
}

/// The item at `loc` when it is usable and of type `weap` (the dual
/// wield test of the rate's step 8.3:).
pub fn usable_weapon_at(w: &ClientWorld, key: UnitKey, loc: u8) -> Option<ItemView> {
    worn_at(w, key, loc).filter(|v| usable(w, v) && is_weap(w, v))
}

/// The weapon in use (module docs).
pub fn weapon_in_use(w: &ClientWorld, key: UnitKey) -> Option<ItemView> {
    [4u8, 5]
        .into_iter()
        .filter_map(|l| worn_at(w, key, l))
        .find(|v| is_weap(w, v))
}

/// The attack weapon with (U, 1) of a unit
/// using the skill with `weapsel`; `None` used skill: the weapon pick.
pub fn attack_weapon(w: &ClientWorld, key: UnitKey, weapsel: Option<u8>) -> Option<ItemView> {
    let d = weapon_pick(w, key);
    if !dual_capable(w, key) {
        return d;
    }
    let Some(weapsel) = weapsel else {
        return d;
    };
    let tables = w.item_tables.0.as_ref();
    let in_use = weapon_in_use(w, key).map(|v| v.key);
    // q(k): the first equip slot whose item, for type
    // classes 2, 3, 12, is (k = 5) / is not (k = 6) the weapon in use;
    // for other items, whose `component` is k.
    let q = |k: u8| {
        (0..12u8).find_map(|loc| {
            let v = worn_at(w, key, loc)?;
            let code = v.code?;
            let tc = tables.map_or(0, |t| t.type_class(code));
            let ok = if matches!(tc, 2 | 3 | 12) {
                (k == 5) == (Some(v.key) == in_use)
            } else {
                tables.is_some_and(|t| t.component(code) == k)
            };
            ok.then_some(v)
        })
    };
    let weap = |v: &Option<ItemView>| v.as_ref().is_some_and(|v| is_weap(w, v));
    let x = match weapsel {
        1 => q(6),
        // The used entry's flag 0x2000 is not in the model: read clear.
        2 => {
            let (a, b) = (q(5), q(6));
            if weap(&a) {
                a
            } else {
                b
            }
        }
        3 => {
            // n := U +0x38 >> 8, the sequence position;
            // a client unit's
            // "odd" is n > 5.
            let n = w
                .units
                .get(&key)
                .and_then(|u| u.sequence.as_ref())
                .map_or(0, |s| s.pos >> 8);
            if n > 5 {
                q(6)
            } else {
                q(5)
            }
        }
        4 => return None,
        _ => {
            let a = q(5);
            if weap(&a) {
                a
            } else {
                q(6)
            }
        }
    };
    x.filter(|v| is_weap(w, v) && usable(w, v)).or(d)
}

/// The dual-wield stat toggle with (U, 1): S the attack weapon, W
/// the weapon in use; "active" a list attached to U, "on" / "off" its
/// stats counted in U's totals or not (`ItemData::stats_off`).
pub fn dual_wield_stat_toggle(w: &mut ClientWorld, key: UnitKey, weapsel: Option<u8>) {
    let s = attack_weapon(w, key, weapsel).map(|v| v.key);
    let Some(wu) = weapon_in_use(w, key).map(|v| v.key) else {
        return;
    };
    let other = [4u8, 5]
        .into_iter()
        .filter_map(|l| worn_at(w, key, l))
        .map(|v| v.key)
        .find(|&i| i != wu);
    let active = |w: &ClientWorld, i: UnitKey| super::item_lists::attached_to(w, i) == Some(key);
    let set = |w: &mut ClientWorld, i: UnitKey, on: bool| {
        if let Some(KindData::Item(d)) = w.units.get_mut(&i).map(|u| &mut u.kind) {
            d.stats_off = !on;
        }
    };
    match s {
        None => {
            if active(w, wu) {
                set(w, wu, true);
            }
        }
        Some(s) if s == wu => {
            if active(w, wu) {
                set(w, wu, true);
            }
            if let Some(o) = other.filter(|&o| active(w, o)) {
                set(w, o, false);
            }
        }
        Some(s) => {
            if active(w, s) {
                set(w, s, true);
            }
            if active(w, wu) {
                set(w, wu, false);
            }
        }
    }
}

/// The total of `stat` (layer 0) of the item `item`'s own list: its
/// properties and base array (the rate read with (item, 68)).
pub fn item_total(w: &ClientWorld, item: UnitKey, stat: u16) -> i32 {
    let Some(KindData::Item(d)) = w.units.get(&item).map(|u| &u.kind) else {
        return 0;
    };
    d.props
        .iter()
        .chain(d.fields.iter().flat_map(|f| f.base.iter()))
        .filter(|p| p.stat == stat && p.layer == 0)
        .fold(0i32, |a, p| a.wrapping_add(p.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::item_lists::{ItemProp, ItemTablesRef, StreamProps};
    use crate::bridge::world::{ClientUnit, ItemData, ItemRecord, MonsterSequence};
    use std::sync::Arc;

    /// Every code is a `weap` of type class 2 (`1hs`).
    struct Swords;
    impl StreamProps for Swords {
        fn props(&self, _: &[u8]) -> (Vec<ItemProp>, bool) {
            (Vec::new(), false)
        }
        fn item_is_type(&self, _: [u8; 4], t: i16) -> bool {
            t == WEAP
        }
        fn type_class(&self, _: [u8; 4]) -> u32 {
            2
        }
    }

    /// An identified item stream head at body
    /// location `loc` (mode 1).
    fn body_stream(loc: u8, code: &[u8; 4]) -> Vec<u8> {
        let bits: [(u32, u32); 8] = [
            (0x10, 32),
            (0x65, 10),
            (1, 3),
            (u32::from(loc), 4),
            (0, 4),
            (0, 4),
            (0, 3),
            (u32::from_le_bytes(*code), 32),
        ];
        let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
        for (v, w) in bits {
            acc |= u64::from(v) << n;
            n += w;
            while n >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    /// A barbarian with a throwing axe (attack rate −10) in each hand.
    fn two_axes() -> (ClientWorld, UnitKey, UnitKey, UnitKey) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 1);
        let mut p = ClientUnit::new(key);
        p.class = 4;
        w.units.insert(key, p);
        w.local_player = Some(key);
        w.item_tables = ItemTablesRef(Some(Arc::new(Swords)));
        let mut hand = |guid: u32, loc: u8| {
            let ik = UnitKey::new(items::ITEM, guid);
            let mut i = ClientUnit::new(ik);
            i.kind = KindData::Item(ItemData {
                last: Some(ItemRecord {
                    id: 0x9C,
                    action: 0x04,
                    category: 0,
                    owner: None,
                    seq: 0,
                    stream: body_stream(loc, b"tax "),
                }),
                props: vec![ItemProp {
                    stat: 68,
                    layer: 0,
                    value: -10,
                }],
                ..Default::default()
            });
            w.units.insert(ik, i);
            ik
        };
        let (a, b) = (hand(9, 4), hand(10, 5));
        (w, key, a, b)
    }

    fn off(w: &ClientWorld, i: UnitKey) -> bool {
        matches!(&w.units[&i].kind, KindData::Item(d) if d.stats_off)
    }

    // Double Throw (weapsel 3) at the mode start (sequence position 0):
    // the attack weapon is the weapon in use, the other hand goes off
    // (Double Throw: attack rate total 90).
    #[test]
    fn the_toggle_turns_the_hand_not_attacking_off() {
        let (mut w, key, right, left) = two_axes();
        assert_eq!(w.total(key, 68, 0), -20);
        assert_eq!(weapon_in_use(&w, key).map(|v| v.key), Some(right));
        assert_eq!(attack_weapon(&w, key, Some(3)).map(|v| v.key), Some(right));
        dual_wield_stat_toggle(&mut w, key, Some(3));
        assert!(!off(&w, right) && off(&w, left));
        assert_eq!(w.total(key, 68, 0), -10);
        // Past sequence frame 5 the second hand attacks: it is turned on
        // and the weapon in use off.
        w.units.get_mut(&key).unwrap().sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos: 0x600,
            step: 0x100,
            drawn_mode: 18,
        });
        assert_eq!(attack_weapon(&w, key, Some(3)).map(|v| v.key), Some(left));
        dual_wield_stat_toggle(&mut w, key, Some(3));
        assert!(off(&w, right) && !off(&w, left));
        assert_eq!(item_total(&w, right, 68), -10);
        // A unit that cannot dual-wield takes the weapon pick.
        w.units.get_mut(&key).unwrap().class = 1;
        assert_eq!(attack_weapon(&w, key, Some(3)).map(|v| v.key), Some(right));
    }

    /// Throwing axes: `weap`, type class 2, primary type `throwable`,
    /// `missiletype` 2.
    struct Axes;
    impl StreamProps for Axes {
        fn props(&self, _: &[u8]) -> (Vec<ItemProp>, bool) {
            (Vec::new(), false)
        }
        fn item_is_type(&self, _: [u8; 4], t: i16) -> bool {
            t == WEAP
        }
        fn type_class(&self, _: [u8; 4]) -> u32 {
            2
        }
        fn missile_and_type(&self, _: [u8; 4]) -> Option<(u16, i16)> {
            Some((2, 0))
        }
        fn type_throwable(&self, _: [u8; 4]) -> bool {
            true
        }
    }

    // Double Throw's do (`cltdofunc` 42): before frame 9 one axe missile
    // and no flag 0x40; from frame 9 on the flag too. Without a weapon,
    // nothing and 0.
    #[test]
    fn the_double_throw_do_throws_the_attack_weapon() {
        use crate::bridge::client_missiles::ClientMissileRow;
        use crate::bridge::world::{ModelInputs, SkillRow};
        let (mut w, key, right, left) = two_axes();
        w.item_tables = ItemTablesRef(Some(Arc::new(Axes)));
        let u = w.units.get_mut(&key).unwrap();
        u.position = Some((5000, 5000));
        u.path_dir = Some(0);
        u.sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos: 0x880,
            step: 0x100,
            drawn_mode: 18,
        });
        let mut i = ModelInputs::default();
        i.tables.skills = vec![SkillRow::default(); 141];
        i.tables.skills[140] = SkillRow {
            ingame: true,
            cltdofunc: 42,
            cltmissile: -1,
            cltmissile_abc: [-1; 3],
            cltcalc1: u32::MAX,
            dosound: -1,
            tgtoverlay: -1,
            tgtsound: -1,
            ..SkillRow::default()
        };
        i.tables.missiles = vec![
            ClientMissileRow {
                range: 10,
                ..ClientMissileRow::default()
            };
            3
        ];
        // The do's tail reads the skill delay (the use lock); no skill rows → 0.
        i.skill_tables = Some(Arc::new(d2_sim::skills::SkillTables {
            skills: Vec::new(),
            skilldesc: Vec::new(),
            missiles: Vec::new(),
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        let do_ = super::super::client_do::client_do;
        assert_eq!(do_(&mut w, &i, key, 140, 1).unwrap(), 1);
        assert!(!w.units[&key].flag_40);
        assert_eq!(w.objclient.missiles.len(), 1);
        w.units
            .get_mut(&key)
            .unwrap()
            .sequence
            .as_mut()
            .unwrap()
            .pos = 0x900;
        assert_eq!(do_(&mut w, &i, key, 140, 1).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        assert_eq!(w.objclient.missiles.len(), 2);
        w.units.remove(&right);
        w.units.remove(&left);
        w.units.get_mut(&key).unwrap().flag_40 = false;
        assert_eq!(do_(&mut w, &i, key, 140, 1).unwrap(), 0);
        assert!(!w.units[&key].flag_40);
    }
}
