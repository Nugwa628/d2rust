//! Tests written against surviving mutants: each asserts a
//! value the stat rules decide and that a mutant of `stats/mod.rs` changed.

use std::sync::Arc;

use d2_data::tables::{Charstats, Itemstatcost, Record};

use super::lists::{flag, owner};
use super::tests::{data, item_list, itemstatcost, itemstatcost_with, player, set_u16, Log, N, P};
use super::*;
use crate::units::{UnitId, UnitType};

/// MulDiv branch choice at its boundaries and the truncation each
/// branch keeps (`(a / c) · b` loses the remainder, the 64-bit product
/// does not).
#[test]
fn muldiv_branch_boundaries() {
    // A > 0x100000 and c ≤ a >> 4: (a / c) · b, not the 32-bit
    // product (which would wrap to 0).
    assert_eq!(muldiv(0x20_0000, 0x1000, 0x10), 0x2000_0000);
    // a = 0x100000 is not > 0x100000:'s 32-bit product wraps to 0
    // (would give (a / 3) · b = 0x5555_0000).
    assert_eq!(muldiv(0x10_0000, 0x1_0000, 3), 0);
    // c ≤ a >> 4: the quotient is truncated before the product.
    assert_eq!(muldiv(0x20_0001, 0x10, 0x10), 0x20_0000);
    // c > a >> 4: 64-bit (a · b) / c = 0x600000 / 0x30000 = 32, not
    // (a / c) · b = 10 · 3.
    assert_eq!(muldiv(0x20_0000, 3, 0x3_0000), 32);
    // The same two cases on b.
    assert_eq!(muldiv(0x10, 0x2_0001, 0x10), 0x2_0000);
    assert_eq!(muldiv(3, 0x2_0000, 0x3000), 32);
}

/// By-time angles between 180 and 359 fold to 360 − a.
#[test]
fn by_time_folds_above_180() {
    // lo 50, hi 100, period 0.
    let v = (356 << 12) | (306 << 2);
    // d = 195: a = 195 → 165; 100 − (50 · 165) / 180 = 100 − 45.
    assert_eq!(by_time(v, 195), 55);
    // d = 270: a = 270 → 90; 100 − 25.
    assert_eq!(by_time(v, 270), 75);
}

/// Life fraction when the current life exceeds max, and with max 0
/// and negative life: both 128 ("else 128").
#[test]
fn life_fraction_outside_the_ratio_branch() {
    assert_eq!(life_fraction(51_200, 25_600), 128);
    assert_eq!(life_fraction(-256, 0), 128);
}

/// n is the itemstatcost count; ValShift is 8 for 6–11,
/// 216, 217 and 0 for the others; the getter of an invalid
/// id returns 0.
#[test]
fn table_size_and_valshift() {
    let t = StatTable::from_fixed(&itemstatcost()).unwrap();
    assert_eq!(t.len(), N);
    assert!(!t.is_empty());
    assert!(StatTable::default().is_empty());
    assert_eq!(StatTable::default().len(), 0);
    for s in [6, 7, 8, 9, 10, 11, 216, 217] {
        assert_eq!(t.valshift(s), 8, "stat {s}");
    }
    for s in [0, 5, 12, 215, 218, 358] {
        assert_eq!(t.valshift(s), 0, "stat {s}");
    }
    assert_eq!(t.valshift(N as u16), 0);
    assert!(t.get(N as u16 - 1).is_some() && t.get(N as u16).is_none());
}

/// The four charstats columns the stat code reads (Inputs),
/// at their record offsets.
#[test]
fn class_stats_from_charstats() {
    let mut r = [0u8; Charstats::SIZE];
    r[0x3A] = 3;
    r[0x46] = 16;
    r[0x47] = 4;
    r[0x48] = 8;
    let c = ClassStats::from(&Charstats::decode(&r));
    assert_eq!(
        c,
        ClassStats {
            mana_regen: 3,
            life_per_vitality: 16,
            stamina_per_vitality: 4,
            mana_per_magic: 8,
        }
    );
}

/// The max-rescale product new / o · c is 0 when new or c is 0
/// (new = 0 then sets the current value to 0).
#[test]
fn rescale_of_zero_is_zero() {
    assert_eq!(x87_rescale(0, 256, 100, 53), 0);
    assert_eq!(x87_rescale(512, 256, 0, 53), 0);
}

/// The sign of the product new / o · c is the product of the signs
/// (q is that x87 product). The callback only
/// passes o ≥ 256 and c > 0; new can be negative.
#[test]
fn rescale_sign_is_the_product_of_signs() {
    assert_eq!(x87_rescale(-512, 256, 100, 53), -200);
    assert_eq!(x87_rescale(512, -256, 100, 53), -200);
    assert_eq!(x87_rescale(512, 256, -100, 53), -200);
    assert_eq!(x87_rescale(-512, -256, 100, 53), 200);
    assert_eq!(x87_rescale(-512, 256, -100, 53), 200);
    assert_eq!(x87_rescale(512, -256, -100, 53), 200);
    assert_eq!(x87_rescale(-512, -256, -100, 53), -200);
}

/// The minimum rule compares `v < MinAccr`: a value equal to MinAccr
/// reads as itself (maxhp MinAccr 1, ValShift 8).
#[test]
fn minimum_rule_is_strictly_below() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    lists.set(&mut log, p, 7, 1, 0, None);
    assert_eq!(lists.base(p, 7, 0), 1);
    assert_eq!(lists.total(p, 7, 0), 1);
}

/// `unit_pm` guard: an op-4 source on a list attached
/// to a unit that is not a player or monster contributes nothing, even
/// when that unit's list has the op base.
#[test]
fn per_level_needs_a_player_or_monster_wearer() {
    const HOLDER: UnitId = UnitId(3);
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let holder = lists.alloc_extended(&mut log, HOLDER, UnitType::Item, 3, 0, 0, None);
    lists.set(&mut log, holder, 12, 10, 0, None);
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    lists.attach(&mut log, HOLDER, i, true);
    assert_eq!(lists.total(i, 31, 0), 0);
    assert_eq!(lists.total(holder, 31, 0), 0);
    // The same list on the player (level 10) does count.
    let _ = player(&mut lists, &mut log);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(i, 31, 0), (8 * 10) >> 3);
}

/// Flag bits callers own: set and cleared without
/// touching the other bits.
#[test]
fn set_flags_touches_only_the_given_bits() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let before = lists.flags(p);
    assert_ne!(before & flag::EXTENDED, 0);
    lists.set_flags(p, 0x48, true);
    assert_eq!(lists.flags(p), before | 0x48);
    lists.set_flags(p, 0x40, false);
    assert_eq!(lists.flags(p), before | 0x08);
    lists.set_flags(p, 0x08, false);
    assert_eq!(lists.flags(p), before);
}

// ---- stat lists ------------------------------------------

fn rec(records: &mut [u8], s: usize) -> &mut [u8] {
    let size = Itemstatcost::SIZE;
    &mut records[s * size..(s + 1) * size]
}

/// [`data`] with the synthetic itemstatcost edited by `edit`.
fn data_with(edit: impl FnOnce(&mut [u8])) -> Arc<StatData> {
    let mut d = (*data()).clone();
    d.stats = StatTable::from_fixed(&itemstatcost_with(edit)).expect("itemstatcost");
    Arc::new(d)
}

/// A host recording the provider calls of the max-rescale.
#[derive(Default)]
struct Calls {
    item_events: Vec<(UnitId, u16, i32)>,
    skill_stats: Vec<(UnitId, i32, i32, i32)>,
}

impl StatHost for Calls {
    fn item_event(&mut self, _: &mut StatLists, owner: UnitId, stat: u16, new: i32) {
        self.item_events.push((owner, stat, new));
    }
    fn skill_stat_changed(
        &mut self,
        _: &mut StatLists,
        owner: UnitId,
        key: i32,
        old: i32,
        new: i32,
    ) {
        self.skill_stats.push((owner, key, old, new));
    }
}

/// `owner_player` guard: ops 8/9 count only on a
/// player's list, even when the class has a charstats record. (The
/// list's own energy is subtracted, d = r − own base; the energy comes
/// from an attached list.)
#[test]
fn energy_bonus_needs_a_player_owner() {
    const MON: UnitId = UnitId(5);
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let m = lists.alloc_extended(&mut log, MON, UnitType::Monster, 5, 0, 0, None);
    let i = item_list(&mut lists, &mut log, &[(1, 4)]);
    lists.attach(&mut log, MON, i, true);
    assert_eq!(lists.total(m, 1, 0), 4);
    assert_eq!(lists.total(m, 9, 0), 0);
    // The same on the player: (8 · 4) << 6.
    let p = player(&mut lists, &mut log);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 9, 0), (8 * 4) << 6);
}

/// `owner_item_base` only replaces prev when the owner is an item:
/// on a player, op 1 takes the percent of v, the sum
/// with the attached lists.
#[test]
fn op1_prev_is_the_sum_on_a_player() {
    let d = data_with(|r| {
        let x = rec(r, 50);
        x[0x54] = 1;
        set_u16(x, 0x56, NO_STAT);
        set_u16(x, 0x58, 51);
    });
    let mut log = Log::default();
    let mut lists = StatLists::new(d);
    let p = player(&mut lists, &mut log);
    lists.set(&mut log, p, 51, 100, 0, None);
    let i = item_list(&mut lists, &mut log, &[(51, 20)]);
    lists.attach(&mut log, P, i, true);
    lists.set(&mut log, p, 50, 10, 0, None);
    // v = prev = 100 + 20; + MulDiv(120, 10, 100).
    assert_eq!(lists.total(p, 51, 0), 132);
}

/// Set-full of 0 on an absent entry writes nothing, even for a
/// `keepzero` stat.
#[test]
fn set_full_zero_on_absent_writes_nothing() {
    let d = data_with(|r| rec(r, 31)[0x50] = 1);
    let mut log = Log::default();
    let mut lists = StatLists::new(d);
    // 214 on an unattached item: eval(31) = 0 (no wearer).
    let i = item_list(&mut lists, &mut log, &[(214, 8)]);
    assert!(lists
        .full_entries(i)
        .iter()
        .all(|&(k, _)| key_stat(k) != 31));
}

/// A damage-related change on a static (not DYNAMIC) list keeps
/// propagating to the parent.
#[test]
fn damage_related_change_reaches_a_static_parent() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(20, 1)]);
    lists.attach(&mut log, P, i, true);
    lists.set(&mut log, i, 19, 5, 0, None);
    assert_eq!(lists.total(p, 19, 0), 5);
}

/// Recompute block of op 2
/// (`listtype_pm_and_entrybase_list_total_pos`): with strength 0 the
/// player's raw total of the unused entry's base (stat 0) is not > 0,
/// so 216 does get its own full entry.
#[test]
fn per_level_stat_kept_when_strength_is_zero() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = lists.alloc_extended(&mut log, P, UnitType::Player, 1, 0, 0, None);
    lists.set(&mut log, p, 12, 10, 0, None);
    lists.set(&mut log, p, 216, 8, 0, None);
    assert_eq!(lists.total(p, 7, 0), (8 * 10) >> 3);
    assert!(lists.full_entries(p).contains(&(key(216, 0), 8)));
    // Strength > 0 blocks it (the recompute block's consequences).
    lists.set(&mut log, p, 0, 1, 0, None);
    lists.set(&mut log, p, 216, 16, 0, None);
    assert!(lists.full_entries(p).contains(&(key(216, 0), 8)));
    assert_eq!(lists.total(p, 7, 0), (16 * 10) >> 3);
}

/// Item events only when `itemevent1` > 0.
#[test]
fn item_event_needs_a_positive_itemevent1() {
    for (ev, want) in [(0u16, false), (1, true), (0xFFFF, false)] {
        let d = data_with(|r| set_u16(rec(r, 7), 0x48, ev));
        let mut host = Calls::default();
        let mut lists = StatLists::new(d);
        let p = lists.alloc_extended(
            &mut host,
            P,
            UnitType::Player,
            1,
            0,
            0,
            Some(ValueCallback::Server),
        );
        lists.set(&mut host, p, 7, 256, 0, None);
        assert_eq!(!host.item_events.is_empty(), want, "itemevent1 {ev}");
    }
}

/// The skill and state stats go to their handler.
#[test]
fn skill_stats_reach_their_handler() {
    let d = data_with(|r| {
        for s in [83, 151] {
            rec(r, s)[5] |= 0x08; // fCallback
        }
    });
    let mut host = Calls::default();
    let mut lists = StatLists::new(d);
    let p = lists.alloc_extended(
        &mut host,
        P,
        UnitType::Player,
        1,
        0,
        0,
        Some(ValueCallback::Server),
    );
    lists.set(&mut host, p, 83, 3, 7, None);
    lists.set(&mut host, p, 151, 2, 0, None);
    assert_eq!(
        host.skill_stats,
        [(P, key(83, 7), 0, 3), (P, key(151, 0), 0, 2)]
    );
}

/// No rescale when old max ≤ 0 or current ≤ 0; stat 74 only
/// for a monster's max life.
#[test]
fn max_life_rescale_conditions() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    // Class 1: damage_regen 8 in the synthetic monstats.
    let p = lists.alloc_extended(
        &mut log,
        P,
        UnitType::Player,
        1,
        1,
        0,
        Some(ValueCallback::Server),
    );
    // old = 0: current stays.
    lists.set(&mut log, p, 6, 12800, 0, None);
    lists.set(&mut log, p, 7, 25600, 0, None);
    assert_eq!(lists.base(p, 6, 0), 12800);
    // A player's max life never writes stat 74.
    assert_eq!(lists.base(p, 74, 0), 0);
    // c = 0: nothing written.
    lists.set(&mut log, p, 6, 0, 0, None);
    lists.set(&mut log, p, 7, 51200, 0, None);
    assert_eq!(
        lists.base_entries(p).iter().find(|e| key_stat(e.0) == 6),
        None
    );
    // Max mana on a monster does not write stat 74.
    const MON: UnitId = UnitId(5);
    let m = lists.alloc_extended(
        &mut log,
        MON,
        UnitType::Monster,
        5,
        1,
        0,
        Some(ValueCallback::Server),
    );
    lists.set(&mut log, m, 9, 25600, 0, None);
    assert_eq!(lists.base(m, 74, 0), 0);
    lists.set(&mut log, m, 7, 25600, 0, None);
    assert_eq!(lists.base(m, 74, 0), (100 * 8) >> 4);
}

/// Mod insert only for an extended list whose owner type is 0.
#[test]
fn mod_insert_only_for_players() {
    const MON: UnitId = UnitId(5);
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let m = lists.alloc_extended(&mut log, MON, UnitType::Monster, 5, 0, 0, None);
    lists.set(&mut log, m, 0, 5, 0, None);
    assert!(lists.mods(m).is_empty());
    let p = player(&mut lists, &mut log);
    assert!(lists.mods(p).contains(&key(0, 0)));
}

/// Set of 0 on an absent stat returns 0 and stores nothing (step 4: a
/// base array never holds 0).
#[test]
fn set_zero_on_absent_stores_nothing() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let before = lists.base_entries(p);
    assert!(!lists.set(&mut log, p, 40, 0, 0, None));
    assert_eq!(lists.base_entries(p), before);
}

/// Only a TEMPONLY list sets NEWLENGTH on the unit's list.
#[test]
fn attach_without_temponly_leaves_newlength() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(20, 1)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.flags(p) & flag::NEWLENGTH, 0);
}

/// detaches like.
#[test]
fn unit_detach_removes_the_values() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(20, 3)]);
    lists.attach(&mut log, P, i, true);
    assert_eq!(lists.total(p, 20, 0), 3);
    lists.unit_detach(&mut log, i);
    assert_eq!(lists.total(p, 20, 0), 0);
    assert_eq!(lists.parent(i), None);
}

/// frees a plain list and leaves an extended one.
#[test]
fn free_plain_frees_only_plain_lists() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let c = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.free_plain(&mut log, c);
    assert!(!lists.is_live(c));
    lists.free_plain(&mut log, p);
    assert!(lists.is_live(p));
}

/// on a list already attached to U: reset and DYNAMIC → static;
/// no reset and not DYNAMIC → dynamic; otherwise nothing.
#[test]
fn equip_toggles_an_attached_list() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(19, 5)]);
    lists.equip(&mut log, P, Some(i), false, true);
    assert_eq!(lists.total(p, 19, 0), 5);
    lists.equip(&mut log, P, Some(i), false, true);
    assert_eq!(
        (lists.total(p, 19, 0), lists.flags(i) & flag::DYNAMIC),
        (5, 0)
    );
    lists.equip(&mut log, P, Some(i), false, false);
    assert_eq!(lists.total(p, 19, 0), 0);
    assert_ne!(lists.flags(i) & flag::DYNAMIC, 0);
    lists.equip(&mut log, P, Some(i), false, false);
    assert_eq!(lists.total(p, 19, 0), 0);
    lists.equip(&mut log, P, Some(i), false, true);
    assert_eq!(
        (lists.total(p, 19, 0), lists.flags(i) & flag::DYNAMIC),
        (5, 0)
    );
}

/// Make static on a list not attached to U is equip(U, I, 1):
/// attached with reset, so its damage-related stats count.
#[test]
fn make_static_on_an_unattached_list_equips_with_reset() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let i = item_list(&mut lists, &mut log, &[(19, 5)]);
    lists.make_static(&mut log, P, i, false);
    assert_eq!(lists.total(p, 19, 0), 5);
    assert_eq!(lists.flags(i) & flag::DYNAMIC, 0);
    // make dynamic: equip(U, I, 0), attached without reset.
    let j = lists.alloc_extended(&mut log, UnitId(4), UnitType::Item, 4, 0, 0, None);
    lists.set(&mut log, j, 19, 2, 0, None);
    lists.make_dynamic(&mut log, P, j, false);
    assert_ne!(lists.flags(j) & flag::DYNAMIC, 0);
    assert_eq!(lists.total(p, 19, 0), 5);
}

/// The owner (type, GUID) of a unit's state list (the list of a state;
/// owner fields).
#[test]
fn state_list_owner_reads_the_list() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let _ = player(&mut lists, &mut log);
    assert_eq!(lists.state_list_owner(P, 30), None);
    let c = lists.alloc(0, 0, owner::ITEM, 9);
    lists.set_state(c, 30);
    lists.attach(&mut log, P, c, true);
    assert_eq!(lists.state_list_owner(P, 30), Some((owner::ITEM, 9)));
}

/// Only NEWLENGTH lists count down on the client frame 0, and
/// only NEWLENGTH lists expire.
#[test]
fn expiry_needs_newlength() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let _ = player(&mut lists, &mut log);
    // Expire stored without NEWLENGTH: never expires.
    let a = lists.alloc(0, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, a, true);
    lists.expire_lists(&mut log, P, 10).unwrap();
    assert!(lists.is_live(a));
    // Frame 0 decrements only NEWLENGTH lists.
    let b = lists.alloc(0, 2, owner::PLAYER, 1);
    lists.attach(&mut log, P, b, true);
    lists.expire_lists(&mut log, P, 0).unwrap();
    assert_eq!(lists.expire(b), 2);
    lists.set_flags(b, flag::NEWLENGTH, true);
    lists.expire_lists(&mut log, P, 0).unwrap();
    assert_eq!(lists.expire(b), 1);
    assert!(lists.is_live(b));
}

/// A state id ≥ count has no flag (0 ≤ s < count).
#[test]
fn state_flags_stop_at_count() {
    let t = StateTable::synthetic(64, &[(63, 0)]);
    assert!(t.has_flag(63, 0));
    assert!(!t.has_flag(64, 0));
}

/// Flag groups are g < 40; a larger group has no
/// state.
#[test]
fn state_flag_groups_stop_at_40() {
    let t = StateTable::synthetic(64, &[(3, 39)]);
    assert!(t.has_flag(3, 39));
    assert!(!t.has_flag(3, 40));
}

/// sets or clears the second-half ("changed") bit
/// of s only; a state past the W words is ignored.
#[test]
fn set_state_changed_touches_one_changed_bit() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let _ = player(&mut lists, &mut log);
    lists.toggle_state(P, 3, true);
    let (on, changed) = lists.state_bits(P).unwrap();
    let words = on.len();
    assert_eq!(words, 6);
    lists.set_state_changed(P, 40, true);
    let mut want = changed.clone();
    want[1] |= 1 << 8;
    assert_eq!(lists.state_bits(P).unwrap(), (on.clone(), want));
    lists.set_state_changed(P, 3, false);
    lists.set_state_changed(P, 40, false);
    let mut want = changed.clone();
    want[0] &= !(1 << 3);
    assert_eq!(lists.state_bits(P).unwrap(), (on.clone(), want.clone()));
    lists.set_state_changed(P, 6 * 32, true);
    assert_eq!(lists.state_bits(P).unwrap(), (on, want));
}

/// By flags: the active chain unless 0x2000 is
/// asked; the first list with any asked flag.
#[test]
fn list_by_flags_searches_the_active_chain() {
    let mut log = Log::default();
    let mut lists = StatLists::new(data());
    let p = player(&mut lists, &mut log);
    let a = lists.alloc(0x08, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, a, true);
    let b = lists.alloc(flag::SET | 0x08, 0, owner::PLAYER, 1);
    lists.attach(&mut log, P, b, true);
    assert_eq!(lists.list_by_flags(p, 0x08), Some(a));
    assert_eq!(lists.list_by_flags(p, flag::SET), Some(b));
    assert_eq!(lists.list_by_flags(p, 0x40), None);
}
