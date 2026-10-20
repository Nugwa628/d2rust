// Spec: specs/sim/stat-lists.md; specs/sim/stats.md §4, §6
//! Stat lists: plain and extended records, flags, sorted stat arrays,
//! allocation, base writes, total recomputation, value-change callbacks,
//! attached chains, expiry and the modified-stat array.
//!
//! Lists live in an arena ([`StatLists`]) addressed by generational
//! [`ListId`]s; a unit's extended list is found by its [`UnitId`].
//! Items, skills, states and the environment are reached through
//! [`StatHost`], which receives the lists to write stats during callbacks.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::ops::{op_row, Contribution, Guard, Operand, Prev, RecomputeBlock};
use super::{by_time, key, key_stat, muldiv, stat, x87_rescale, StatData, StatInfo, NO_STAT};
use crate::units::{UnitId, UnitType};

/// Flags controlling attachment, expiry and stat propagation.
pub mod flag {
    pub const BASIC: u32 = 0x0000_0001;
    pub const NEWLENGTH: u32 = 0x0000_0002;
    pub const TEMPONLY: u32 = 0x0000_0004;
    pub const OVERLAY: u32 = 0x0000_0080;
    /// On a unit's list: remove the overlay list's stats.
    pub const REMOVE_OVERLAY: u32 = 0x0000_0100;
    /// Parked (SET): lives in the parked chain, counts nowhere.
    pub const SET: u32 = 0x0000_2000;
    pub const PERMANENT: u32 = 0x2000_0000;
    pub const DYNAMIC: u32 = 0x4000_0000;
    pub const EXTENDED: u32 = 0x8000_0000;
}

/// Unit type numbers as stored in a list's owner-type field (+0x08).
pub mod owner {
    pub const PLAYER: u32 = 0;
    pub const MONSTER: u32 = 1;
    pub const ITEM: u32 = 4;
}

/// Stats never put in the mod array.
const MOD_EXCLUDED: [u16; 5] = [6, 8, 10, 13, 14];

/// A list, by arena slot and generation.
///
/// A stale handle names no list. Readers return zero or empty results
/// and writes do nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListId {
    index: u32,
    generation: u32,
}

/// A stat-list operation that cannot complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StatListError {
    /// An expired extended list in the active chain prevents the expiry
    /// walk from making progress.
    #[error("expired extended stat list {0:?}: expiry would loop endlessly (stat-lists.md §10.4)")]
    EndlessExpiry(ListId),
}

/// An extended list's value-change callback (+0x5C).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueCallback {
    /// Server-side changes to player and monster stats.
    Server,
}

/// A list's remove callback (+0x38), opaque: the [`StatHost`] gives ids
/// their meaning (state code, skills).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RemoveCallback(pub u32);

/// One value-change notification, as handed to
/// [`StatHost::on_callback`] before the callback body runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackEvent {
    pub list: ListId,
    /// The list's owner unit (+0x44).
    pub owner: UnitId,
    /// The propagation's unit argument.
    pub unit: Option<UnitId>,
    pub key: i32,
    pub old: i32,
    pub new: i32,
}

/// What the stat code needs from systems other specs own. Each default
/// is the narrowest reading: nothing happens.
#[allow(unused_variables)]
pub trait StatHost {
    /// The base time t of the unit's act, or `None` when
    /// the unit has no act record (guard `owner_act` fails). Provider:
    /// the environment (world group; an open question).
    fn act_time(&self, unit: UnitId) -> Option<i32> {
        None
    }

    /// A mod's callback after the changed total is stored and the trace
    /// notification runs. `Some(())` handles it; `None` runs the server
    /// callback.
    fn mod_stat_callback(&mut self, lists: &mut StatLists, ev: &CallbackEvent) -> Option<()> {
        None
    }

    /// Called for every value-change callback before its body runs
    /// (trace comparison, Test vectors 2).
    fn on_callback(&mut self, lists: &StatLists, ev: &CallbackEvent) {}

    /// Registers item events for a nonzero value and removes them at zero.
    fn item_event(&mut self, lists: &mut StatLists, owner: UnitId, stat: u16, new: i32) {}

    /// with the full key and the row's events: `events[i]` =
    /// (`itemevent1/2`, `itemeventfunc1/2`) as i16; called with
    /// [`StatHost::item_event`] when `itemevent1` > 0. Provider: the
    /// unit event handlers.
    fn item_event_register(&mut self, owner: UnitId, key: i32, new: i32, events: [(i16, i16); 2]) {}

    /// Stats 83, 97, 98, 107, 126, 127, 151, 188, 204:
    /// skill and state handlers. Provider: skills.
    fn skill_stat_changed(
        &mut self,
        lists: &mut StatLists,
        owner: UnitId,
        key: i32,
        old: i32,
        new: i32,
    ) {
    }

    /// A detached list's remove callback (ECX unit, EDX
    /// state, list). Provider: state code (skills).
    fn list_removed(
        &mut self,
        lists: &mut StatLists,
        unit: UnitId,
        state: u32,
        list: ListId,
        callback: RemoveCallback,
    ) {
    }

    /// Whether a state persists when a unit dies. State providers override
    /// this; the default is false.
    fn stays_on_death(&self, lists: &StatLists, unit: UnitId, state: u32) -> bool {
        false
    }
}

/// A host with every default: no act time, no item or skill handlers.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHost;

impl StatHost for NoHost {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    key: i32,
    value: i32,
}

#[derive(Clone, Debug)]
struct Extended {
    /// +0x3C, +0x40: heads of the active and parked chains.
    active: Option<ListId>,
    parked: Option<ListId>,
    /// +0x44.
    owner: UnitId,
    /// The owner's class (charstats row for players, monstats row for
    /// monsters).
    class: u32,
    /// +0x48.
    full: Vec<Entry>,
    /// +0x50, sorted unique keys.
    mods: Vec<i32>,
    /// +0x58: W state words, then W "changed" words.
    states: Vec<u32>,
    /// +0x5C.
    callback: Option<ValueCallback>,
}

/// A list record.
#[derive(Clone, Debug)]
struct List {
    /// +0x04.
    unit: Option<UnitId>,
    /// +0x08, +0x0C.
    owner_type: u32,
    owner_guid: u32,
    /// +0x10.
    flags: u32,
    /// +0x14.
    state: u32,
    /// +0x18.
    expire: i32,
    /// +0x1C, +0x20.
    skill: u32,
    skill_level: u32,
    /// +0x24.
    base: Vec<Entry>,
    /// +0x2C, +0x30, +0x34.
    prev: Option<ListId>,
    next: Option<ListId>,
    parent: Option<ListId>,
    /// +0x38.
    remove_callback: Option<RemoveCallback>,
    ext: Option<Extended>,
}

/// The stat lists of one game.
#[derive(Clone, Debug)]
pub struct StatLists {
    data: Arc<StatData>,
    slots: Vec<(u32, Option<List>)>,
    free: Vec<u32>,
    /// Unit +0x5C.
    units: BTreeMap<UnitId, ListId>,
}

fn find(a: &[Entry], k: i32) -> Result<usize, usize> {
    a.binary_search_by(|e| e.key.cmp(&k))
}

fn get(a: &[Entry], k: i32) -> Option<i32> {
    find(a, k).ok().map(|i| a[i].value)
}

impl StatLists {
    pub fn new(data: Arc<StatData>) -> Self {
        Self {
            data,
            slots: Vec::new(),
            free: Vec::new(),
            units: BTreeMap::new(),
        }
    }

    pub fn data(&self) -> &StatData {
        &self.data
    }

    fn info(&self, s: u16) -> Option<&StatInfo> {
        self.data.stats.get(s)
    }

    fn l(&self, id: ListId) -> &List {
        self.try_l(id).expect("live stat list")
    }

    fn try_l(&self, id: ListId) -> Option<&List> {
        match self.slots.get(id.index as usize) {
            Some((g, Some(l))) if *g == id.generation => Some(l),
            _ => None,
        }
    }

    /// [`Self::lm`] that answers `None` for a stale handle.
    fn try_lm(&mut self, id: ListId) -> Option<&mut List> {
        match self.slots.get_mut(id.index as usize) {
            Some((g, Some(l))) if *g == id.generation => Some(l),
            _ => None,
        }
    }

    fn lm(&mut self, id: ListId) -> &mut List {
        match self.slots.get_mut(id.index as usize) {
            Some((g, Some(l))) if *g == id.generation => l,
            _ => panic!("live stat list {id:?}"),
        }
    }

    fn ext(&self, id: ListId) -> Option<&Extended> {
        self.try_l(id)?.ext.as_ref()
    }

    fn ext_mut(&mut self, id: ListId) -> Option<&mut Extended> {
        self.lm(id).ext.as_mut()
    }

    /// [`Self::ext_mut`] that answers `None` for a freed list.
    fn try_ext_mut(&mut self, id: ListId) -> Option<&mut Extended> {
        self.try_lm(id)?.ext.as_mut()
    }

    fn insert(&mut self, list: List) -> ListId {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.1 = Some(list);
            ListId {
                index,
                generation: slot.0,
            }
        } else {
            self.slots.push((0, Some(list)));
            ListId {
                index: self.slots.len() as u32 - 1,
                generation: 0,
            }
        }
    }

    // ---- allocation -------------------------------------------------

    /// Allocates a plain list. The expiry frame does not set NEWLENGTH.
    pub fn alloc(&mut self, flags: u32, expire: i32, owner_type: u32, owner_guid: u32) -> ListId {
        self.insert(List {
            unit: None,
            owner_type,
            owner_guid,
            flags,
            state: 0,
            expire,
            skill: 0,
            skill_level: 0,
            base: Vec::new(),
            prev: None,
            next: None,
            parent: None,
            remove_callback: None,
            ext: None,
        })
    }

    /// Allocates the unit's extended list, freeing its previous list first.
    /// Keeps BASIC from the supplied flags and adds EXTENDED.
    #[allow(clippy::too_many_arguments)]
    pub fn alloc_extended(
        &mut self,
        host: &mut dyn StatHost,
        unit: UnitId,
        unit_type: UnitType,
        guid: u32,
        class: u32,
        flags: u32,
        callback: Option<ValueCallback>,
    ) -> ListId {
        self.free_unit_list(host, unit);
        let words = self.data.states.words();
        let id = self.insert(List {
            unit: None,
            owner_type: unit_type.index() as u32,
            owner_guid: guid,
            flags: (flags & flag::BASIC) | flag::EXTENDED,
            state: 0,
            expire: 0,
            skill: 0,
            skill_level: 0,
            base: Vec::new(),
            prev: None,
            next: None,
            parent: None,
            remove_callback: None,
            ext: Some(Extended {
                active: None,
                parked: None,
                owner: unit,
                class,
                full: Vec::new(),
                mods: Vec::new(),
                states: vec![0; 2 * words],
                callback,
            }),
        });
        self.units.insert(unit, id);
        id
    }

    /// Frees the unit's extended list, if present.
    pub fn free_unit_list(&mut self, host: &mut dyn StatHost, unit: UnitId) {
        if let Some(&l) = self.units.get(&unit) {
            if self.ext(l).is_some() {
                self.free(host, l);
            }
        }
    }

    /// The unit's list (unit +0x5C).
    pub fn unit_list(&self, unit: UnitId) -> Option<ListId> {
        self.units.get(&unit).copied()
    }

    /// Whether `id` names a live list.
    pub fn is_live(&self, id: ListId) -> bool {
        self.try_l(id).is_some()
    }

    // ---- record fields -------------------------------------------------

    pub fn flags(&self, l: ListId) -> u32 {
        self.try_l(l).map_or(0, |l| l.flags)
    }

    /// Sets or clears flag bits the callers own (open question 3).
    pub fn set_flags(&mut self, l: ListId, bits: u32, on: bool) {
        let Some(list) = self.try_lm(l) else { return };
        let f = &mut list.flags;
        if on {
            *f |= bits;
        } else {
            *f &= !bits;
        }
    }

    pub fn is_extended(&self, l: ListId) -> bool {
        self.ext(l).is_some()
    }

    pub fn owner_type(&self, l: ListId) -> u32 {
        self.try_l(l).map_or(0, |l| l.owner_type)
    }

    pub fn owner_guid(&self, l: ListId) -> u32 {
        self.try_l(l).map_or(0, |l| l.owner_guid)
    }

    /// Extended list owner unit (+0x44).
    pub fn owner(&self, l: ListId) -> Option<UnitId> {
        self.ext(l).map(|e| e.owner)
    }

    /// Unit the list is attached to (+0x04).
    pub fn attached_unit(&self, l: ListId) -> Option<UnitId> {
        self.try_l(l)?.unit
    }

    pub fn parent(&self, l: ListId) -> Option<ListId> {
        self.try_l(l)?.parent
    }

    pub fn prev(&self, l: ListId) -> Option<ListId> {
        self.try_l(l)?.prev
    }

    pub fn next(&self, l: ListId) -> Option<ListId> {
        self.try_l(l)?.next
    }

    /// Heads of the active and parked chains of an extended list.
    pub fn heads(&self, l: ListId) -> (Option<ListId>, Option<ListId>) {
        self.ext(l).map_or((None, None), |e| (e.active, e.parked))
    }

    /// The list's state id.
    pub fn state(&self, l: ListId) -> u32 {
        self.try_l(l).map_or(0, |l| l.state)
    }

    /// Sets the list's state id.
    pub fn set_state(&mut self, l: ListId, state: u32) {
        if let Some(list) = self.try_lm(l) {
            list.state = state;
        }
    }

    pub fn expire(&self, l: ListId) -> i32 {
        self.try_l(l).map_or(0, |l| l.expire)
    }

    /// Sets the expiry frame and enables NEWLENGTH for a positive frame.
    pub fn set_expire(&mut self, l: ListId, frame: i32) {
        let Some(list) = self.try_lm(l) else { return };
        list.expire = frame;
        if frame > 0 {
            list.flags |= flag::NEWLENGTH;
        }
    }

    /// Skill id and level (+0x1C, +0x20; callers' bookkeeping).
    pub fn skill(&self, l: ListId) -> (u32, u32) {
        self.try_l(l).map_or((0, 0), |l| (l.skill, l.skill_level))
    }

    pub fn set_skill(&mut self, l: ListId, skill: u32, level: u32) {
        let Some(list) = self.try_lm(l) else { return };
        list.skill = skill;
        list.skill_level = level;
    }

    /// The remove callback (+0x38).
    pub fn remove_callback(&self, l: ListId) -> Option<RemoveCallback> {
        self.try_l(l).and_then(|x| x.remove_callback)
    }

    pub fn set_remove_callback(&mut self, l: ListId, cb: Option<RemoveCallback>) {
        if let Some(list) = self.try_lm(l) {
            list.remove_callback = cb;
        }
    }

    /// The base array as sorted (key, value) pairs.
    pub fn base_entries(&self, l: ListId) -> Vec<(i32, i32)> {
        self.try_l(l)
            .map(|l| l.base.iter().map(|e| (e.key, e.value)).collect())
            .unwrap_or_default()
    }

    /// The full array as sorted (key, value) pairs (empty for a plain
    /// list).
    pub fn full_entries(&self, l: ListId) -> Vec<(i32, i32)> {
        self.ext(l)
            .map(|e| e.full.iter().map(|e| (e.key, e.value)).collect())
            .unwrap_or_default()
    }

    /// The mod array keys.
    pub fn mods(&self, l: ListId) -> Vec<i32> {
        self.ext(l).map(|e| e.mods.clone()).unwrap_or_default()
    }

    /// The active chain from the head through prev.
    pub fn active_chain(&self, l: ListId) -> Vec<ListId> {
        self.chain(self.heads(l).0)
    }

    /// The parked chain from the head through prev.
    pub fn parked_chain(&self, l: ListId) -> Vec<ListId> {
        self.chain(self.heads(l).1)
    }

    fn chain(&self, mut cur: Option<ListId>) -> Vec<ListId> {
        let mut v = Vec::new();
        while let Some(c) = cur {
            v.push(c);
            cur = self.l(c).prev;
        }
        v
    }

    pub(super) fn state_words(&self, l: ListId) -> Option<&[u32]> {
        self.ext(l).map(|e| e.states.as_slice())
    }

    pub(super) fn state_words_mut(&mut self, l: ListId) -> Option<&mut Vec<u32>> {
        self.ext_mut(l).map(|e| &mut e.states)
    }

    /// The unit type of a unit with an extended list (its owner type).
    pub fn unit_type_raw(&self, unit: UnitId) -> Option<u32> {
        self.unit_list(unit).map(|l| self.l(l).owner_type)
    }

    // ---- readers ------------------------------------------

    /// Minimum rule on a present value.
    fn min_rule(&self, l: ListId, s: u16, v: i32) -> i32 {
        let Some(info) = self.info(s) else {
            return v;
        };
        let list = self.l(l);
        if info.fmin
            && list.ext.is_some()
            && matches!(list.owner_type, owner::PLAYER | owner::MONSTER)
            && v < info.minaccr
        {
            info.minaccr << info.valshift
        } else {
            v
        }
    }

    /// Reads the base array and applies the stat minimum to a present value.
    pub fn base(&self, l: ListId, s: u16, layer: u16) -> i32 {
        if self.info(s).is_none() || !self.is_live(l) {
            return 0;
        }
        match get(&self.l(l).base, key(s, layer)) {
            Some(v) => self.min_rule(l, s, v),
            None => 0,
        }
    }

    /// Reads the full array for an extended list, otherwise the base array,
    /// then applies the stat minimum to a present value.
    pub fn total(&self, l: ListId, s: u16, layer: u16) -> i32 {
        if self.info(s).is_none() || !self.is_live(l) {
            return 0;
        }
        match self.raw_total(l, key(s, layer)) {
            Some(v) => self.min_rule(l, s, v),
            None => 0,
        }
    }

    fn raw_total(&self, l: ListId, k: i32) -> Option<i32> {
        let list = self.l(l);
        match &list.ext {
            Some(e) => get(&e.full, k),
            None => get(&list.base, k),
        }
    }

    fn raw_base(&self, l: ListId, k: i32) -> Option<i32> {
        get(&self.l(l).base, k)
    }

    /// The unit's total; zero when it has no list.
    pub fn unit_total(&self, unit: UnitId, s: u16, layer: u16) -> i32 {
        self.unit_list(unit).map_or(0, |l| self.total(l, s, layer))
    }

    /// The unit's base value; zero when it has no list.
    pub fn unit_base(&self, unit: UnitId, s: u16, layer: u16) -> i32 {
        self.unit_list(unit).map_or(0, |l| self.base(l, s, layer))
    }

    /// The unit's total minus its base value.
    pub fn unit_bonus(&self, unit: UnitId, s: u16, layer: u16) -> i32 {
        self.unit_total(unit, s, layer)
            .wrapping_sub(self.unit_base(unit, s, layer))
    }

    /// Max life.
    pub fn max_life(&self, unit: UnitId) -> i32 {
        self.unit_total(unit, stat::MAXHP, 0)
    }

    /// Max mana.
    pub fn max_mana(&self, unit: UnitId) -> i32 {
        self.unit_total(unit, stat::MAXMANA, 0)
    }

    /// Max stamina.
    pub fn max_stamina(&self, unit: UnitId) -> i32 {
        self.unit_total(unit, stat::MAXSTAMINA, 0)
    }

    /// Percent-adjusted value.
    pub fn percent_adjusted(&self, l: ListId, s: u16, pct: u16, flag: bool) -> i32 {
        let b = self.base(l, s, 0);
        let mut v = b.wrapping_add(muldiv(b, self.total(l, pct, 0), 100));
        if flag {
            v = v.wrapping_add(self.total(l, s, 0).wrapping_sub(b));
        }
        v
    }

    // ---- evaluation ---------------------------------------

    /// Evaluates an extended list's total without changing the lists.
    pub fn eval(&self, host: &dyn StatHost, l: ListId, k: i32) -> i32 {
        let s = key_stat(k);
        let Some(list) = self.try_l(l) else { return 0 };
        let damagerelated = self.info(s).is_some_and(|i| i.damagerelated);
        // Sum the base value and eligible active-list contributions.
        let mut v = get(&list.base, k).unwrap_or(0);
        let mut cur = list.ext.as_ref().and_then(|e| e.active);
        while let Some(c) = cur {
            let child = self.l(c);
            if !(damagerelated && child.flags & flag::DYNAMIC != 0) {
                let cv = match &child.ext {
                    Some(e) => get(&e.full, k),
                    None => get(&child.base, k),
                };
                v = v.wrapping_add(cv.unwrap_or(0));
            }
            cur = child.prev;
        }
        let Some(info) = self.info(s) else {
            return v;
        };
        if !info.a52 {
            return v;
        }
        // op loop.
        let ext = list.ext.as_ref();
        let owner_unit = ext.map(|e| e.owner);
        let owner_type = list.owner_type;
        let mut acc = v;
        let mut prev = v;
        for e in info.entries.iter().take(16) {
            let Some(row) = op_row(e.op) else {
                continue;
            };
            let pass = match row.guard {
                Guard::Owner => owner_unit.is_some(),
                Guard::OwnerItem => owner_type == owner::ITEM,
                Guard::OwnerPlayer => owner_type == owner::PLAYER && self.class_stats(l).is_some(),
                Guard::OwnerPmPrev => {
                    prev != 0 && matches!(owner_type, owner::PLAYER | owner::MONSTER)
                }
                Guard::OwnerAct => owner_unit.is_some_and(|u| host.act_time(u).is_some()),
                Guard::ListtypePm => matches!(owner_type, owner::PLAYER | owner::MONSTER),
                Guard::UnitPm => list.unit.is_some_and(|u| {
                    matches!(self.unit_type_raw(u), Some(owner::PLAYER | owner::MONSTER))
                }),
                Guard::Never => false,
            };
            if !pass {
                continue;
            }
            if row.prev == Prev::OwnerItemBase && owner_type == owner::ITEM {
                if let Some(o) = owner_unit {
                    prev = self.unit_base(o, s, 0);
                }
            }
            let x = match row.operand {
                Operand::None => 0,
                Operand::OpbaseListTotal | Operand::OpbaseUnitTotal => {
                    let Some(binfo) = (e.base != NO_STAT).then(|| self.info(e.base)).flatten()
                    else {
                        continue;
                    };
                    let x = if row.operand == Operand::OpbaseListTotal {
                        self.raw_total(l, key(e.base, 0)).unwrap_or(0)
                    } else {
                        list.unit.map_or(0, |u| self.unit_total(u, e.base, 0))
                    };
                    let x = x >> binfo.valshift;
                    if x <= 0 {
                        continue;
                    }
                    x
                }
            };
            if row.contribution == Contribution::None {
                continue;
            }
            let r = self.eval(host, l, key(e.source, 0));
            if r == 0 {
                continue;
            }
            let shifted = || r.wrapping_mul(x) >> e.param;
            let t = || owner_unit.and_then(|u| host.act_time(u)).unwrap_or(0);
            let add = match row.contribution {
                Contribution::None => continue,
                Contribution::MuldivPrevR => {
                    if prev == 0 {
                        continue;
                    }
                    muldiv(prev, r, 100)
                }
                Contribution::ShiftRX => shifted(),
                Contribution::MuldivPrevShiftRX => muldiv(prev, shifted(), 100),
                Contribution::BytimeR => by_time(r, t()),
                Contribution::MuldivAccBytimeR => muldiv(acc, by_time(r, t()), 100),
                Contribution::CharstatManaBonus | Contribution::CharstatVitBonus => {
                    let d = r.wrapping_sub(get(&list.base, key(e.source, 0)).unwrap_or(0));
                    if d == 0 {
                        continue;
                    }
                    let c = self.class_stats(l).unwrap_or_default();
                    let per = if row.contribution == Contribution::CharstatManaBonus {
                        c.mana_per_magic
                    } else if s == stat::MAXSTAMINA {
                        c.stamina_per_vitality
                    } else {
                        c.life_per_vitality
                    };
                    i32::from(per).wrapping_mul(d) << 6
                }
            };
            acc = acc.wrapping_add(add);
        }
        acc
    }

    /// Charstats record of a player list's owner class.
    fn class_stats(&self, l: ListId) -> Option<super::ClassStats> {
        let e = self.ext(l)?;
        self.data.classes.get(e.class as usize).copied()
    }

    // ---- full entries ---------------------------------------------

    /// Sets a full value and notifies its change.
    fn set_full(
        &mut self,
        host: &mut dyn StatHost,
        l: ListId,
        k: i32,
        v: i32,
        unit: Option<UnitId>,
    ) {
        let s = key_stat(k);
        let (keepzero, a53) = self.info(s).map_or((false, false), |i| (i.keepzero, i.a53));
        let list = self.lm(l);
        let Some(e) = list.ext.as_mut() else {
            return;
        };
        let i = match find(&e.full, k) {
            Ok(i) => i,
            Err(_) if v == 0 => return,
            Err(i) => {
                e.full.insert(i, Entry { key: k, value: 0 });
                i
            }
        };
        let old = e.full[i].value;
        if v == 0 && !keepzero {
            e.full.remove(i);
        } else {
            e.full[i].value = v;
            if a53 {
                list.flags |= flag::PERMANENT;
            }
        }
        if old != v {
            self.notify(host, l, k, old, v, unit);
        }
    }

    /// Adds to a full value and notifies its change.
    fn add_full(
        &mut self,
        host: &mut dyn StatHost,
        l: ListId,
        k: i32,
        d: i32,
        unit: Option<UnitId>,
    ) {
        let s = key_stat(k);
        let (keepzero, a53) = self.info(s).map_or((false, false), |i| (i.keepzero, i.a53));
        let list = self.lm(l);
        let Some(e) = list.ext.as_mut() else {
            return;
        };
        let i = find(&e.full, k).unwrap_or_else(|i| {
            e.full.insert(i, Entry { key: k, value: 0 });
            i
        });
        let old = e.full[i].value;
        let new = old.wrapping_add(d);
        e.full[i].value = new;
        if new > 0 && a53 {
            list.flags |= flag::PERMANENT;
        }
        if new == 0 && !keepzero {
            e.full.remove(i);
        }
        self.notify(host, l, k, old, new, unit);
    }

    // ---- propagate, recompute --------------------------------

    /// Propagates a value delta through parent lists, carrying the source unit.
    pub fn propagate(
        &mut self,
        host: &mut dyn StatHost,
        l: ListId,
        k: i32,
        d: i32,
        unit: Option<UnitId>,
    ) {
        let s = key_stat(k);
        let Some(info) = self.info(s) else {
            return;
        };
        let (simple, damagerelated) = (!info.a51 && !info.a52, info.damagerelated);
        if d == 0 || !self.is_live(l) || self.l(l).flags & flag::SET != 0 {
            return;
        }
        let mut cur = if self.is_extended(l) {
            Some(l)
        } else {
            self.l(l).parent
        };
        while let Some(c) = cur {
            if simple {
                self.add_full(host, c, k, d, unit);
            } else {
                self.recompute(host, c, k, unit);
            }
            let list = self.l(c);
            if list.flags & flag::SET != 0 || (damagerelated && list.flags & flag::DYNAMIC != 0) {
                break;
            }
            cur = list.parent;
        }
    }

    /// Recomputes a stat's full value and propagates the difference.
    pub fn recompute(
        &mut self,
        host: &mut dyn StatHost,
        l: ListId,
        k: i32,
        unit: Option<UnitId>,
    ) -> i32 {
        if !self.is_live(l) {
            return 0;
        }
        let v = self.eval(host, l, k);
        let s = key_stat(k);
        let data = Arc::clone(&self.data);
        let Some(info) = data.stats.get(s) else {
            return v;
        };
        if !info.a51 {
            self.set_full(host, l, k, v, unit);
            return v;
        }
        if v == 0 {
            self.set_full(host, l, k, 0, unit);
        }
        let mut update = true;
        let block = op_row(info.op).map_or(RecomputeBlock::None, |r| r.recompute_block);
        for (j, &os) in info.op_stats.iter().enumerate() {
            // TODO: an op stat ≥ n other than 0xFFFF
            // is not described; the load fix-up stops at it, so does this.
            if os == NO_STAT || self.info(os).is_none() {
                break;
            }
            let ks = key(os, 0);
            let vs = self.recompute(host, l, ks, unit);
            self.set_full(host, l, ks, vs, unit);
            if vs != 0 && self.blocks(l, block, info.entry(j).base) {
                update = false;
            }
        }
        if update && v != 0 {
            self.set_full(host, l, k, v, unit);
            for &d in info.deps.iter().take(64) {
                let kd = key(d, 0);
                let vd = self.recompute(host, l, kd, unit);
                self.set_full(host, l, kd, vd, unit);
            }
        }
        v
    }

    /// Whether a recompute block clears update.
    fn blocks(&self, l: ListId, block: RecomputeBlock, base: u16) -> bool {
        let list = self.l(l);
        let pm = |t: u32| matches!(t, owner::PLAYER | owner::MONSTER);
        match block {
            RecomputeBlock::None => false,
            RecomputeBlock::Always => true,
            RecomputeBlock::ListtypePm => pm(list.owner_type),
            RecomputeBlock::ListtypeItem => list.owner_type == owner::ITEM,
            RecomputeBlock::ListtypePmAndEntrybaseListTotalPos => {
                pm(list.owner_type)
                    && base != NO_STAT
                    && self.raw_total(l, key(base, 0)).unwrap_or(0) > 0
            }
            RecomputeBlock::UnitPmAndEntrybaseUnitTotalPos => list.unit.is_some_and(|u| {
                self.unit_type_raw(u).is_some_and(pm)
                    && base != NO_STAT
                    && self.unit_total(u, base, 0) > 0
            }),
        }
    }

    // ---- notification -----------------------------------------------

    fn notify(
        &mut self,
        host: &mut dyn StatHost,
        l: ListId,
        k: i32,
        old: i32,
        new: i32,
        unit: Option<UnitId>,
    ) {
        let Some(e) = self.ext(l) else {
            return;
        };
        let (Some(cb), owner) = (e.callback, e.owner) else {
            return;
        };
        if !self.info(key_stat(k)).is_some_and(|i| i.fcallback) {
            return;
        }
        let ev = CallbackEvent {
            list: l,
            owner,
            unit,
            key: k,
            old,
            new,
        };
        host.on_callback(self, &ev);
        if host.mod_stat_callback(self, &ev).is_some() {
            return;
        }
        match cb {
            ValueCallback::Server => self.server_callback(host, l, &ev),
        }
    }

    /// Registers item events and updates skills or current vital values after a
    /// stat total changes.
    fn server_callback(&mut self, host: &mut dyn StatHost, l: ListId, ev: &CallbackEvent) {
        let s = key_stat(ev.key);
        let Some(info) = self.info(s) else {
            return;
        };
        let owner = ev.owner;
        if info.itemevent1 > 0 {
            let events = [
                (info.itemevent1, info.itemeventfunc1),
                (info.itemevent2, info.itemeventfunc2),
            ];
            host.item_event(self, owner, s, ev.new);
            host.item_event_register(owner, ev.key, ev.new, events);
        }
        match s {
            stat::MAXHP | stat::MAXMANA | stat::MAXSTAMINA => {
                let current = s - 1;
                let (new, old) = (ev.new, ev.old);
                if new != old && old > 0 {
                    let c = self.unit_total(owner, current, 0);
                    if c > 0 {
                        let o = if old <= 256 { 256 } else { old };
                        let q = x87_rescale(new, o, c, self.data.rescale_precision);
                        let m = q.max(1);
                        let v = if m >= new { new } else { m };
                        self.unit_set(host, owner, current, v, 0);
                    }
                }
                // TODO: read as a step of its own after
                // the rescale, whatever its condition; confirm with a
                // monster max-life recording.
                if s == stat::MAXHP && self.l(l).owner_type == owner::MONSTER {
                    let class = self.ext(l).map_or(0, |e| e.class);
                    let dr = self
                        .data
                        .damage_regen
                        .get(class as usize)
                        .copied()
                        .unwrap_or(0);
                    if dr != 0 {
                        let v = (ev.new >> 8).wrapping_mul(dr as i32) >> 4;
                        self.unit_set(host, owner, stat::HPREGEN, v, 0);
                    }
                }
            }
            83 | 97 | 98 | 107 | 126 | 127 | 151 | 188 | 204 => {
                host.skill_stat_changed(self, owner, ev.key, ev.old, ev.new);
            }
            _ => {}
        }
    }

    // ---- base writes --------------------------------------------------

    /// Adds a saved stat key to the modified-stat array unless excluded.
    fn mod_insert(&mut self, l: ListId, k: i32) {
        let s = key_stat(k);
        if MOD_EXCLUDED.contains(&s) || !self.info(s).is_some_and(|i| i.saved) {
            return;
        }
        if let Some(e) = self.ext_mut(l) {
            if let Err(i) = e.mods.binary_search(&k) {
                e.mods.insert(i, k);
            }
        }
    }

    fn mod_insert_player(&mut self, l: ListId, k: i32) {
        let list = self.l(l);
        if list.ext.is_some() && list.owner_type == owner::PLAYER {
            self.mod_insert(l, k);
        }
    }

    /// Sets a base stat and propagates its change, with an optional unit.
    /// Returns whether the value changed.
    pub fn set(
        &mut self,
        host: &mut dyn StatHost,
        l: ListId,
        s: u16,
        value: i32,
        layer: u16,
        unit: Option<UnitId>,
    ) -> bool {
        let k = key(s, layer);
        let Some(list) = self.try_lm(l) else {
            return false;
        };
        let i = match find(&list.base, k) {
            Ok(i) => i,
            Err(_) if value == 0 => return false,
            Err(i) => {
                list.base.insert(i, Entry { key: k, value: 0 });
                i
            }
        };
        let old = list.base[i].value;
        let d = value.wrapping_sub(old);
        if d == 0 {
            return false;
        }
        if value == 0 {
            list.base.remove(i);
        } else {
            list.base[i].value = value;
        }
        self.propagate(host, l, k, d, unit);
        self.mod_insert_player(l, k);
        true
    }

    /// Sets a base value on the unit's list and marks it modified for a player.
    pub fn unit_set(
        &mut self,
        host: &mut dyn StatHost,
        unit: UnitId,
        s: u16,
        value: i32,
        layer: u16,
    ) {
        if let Some(l) = self.unit_list(unit) {
            self.set(host, l, s, value, layer, None);
            self.mod_insert_player(l, key(s, layer));
        }
    }

    /// Adds to a base stat and propagates the change.
    pub fn add(&mut self, host: &mut dyn StatHost, l: ListId, s: u16, d: i32, layer: u16) {
        if d == 0 {
            return;
        }
        let k = key(s, layer);
        let Some(list) = self.try_lm(l) else { return };
        let i = find(&list.base, k).unwrap_or_else(|i| {
            list.base.insert(i, Entry { key: k, value: 0 });
            i
        });
        let v = list.base[i].value.wrapping_add(d);
        if v == 0 {
            list.base.remove(i);
        } else {
            list.base[i].value = v;
        }
        self.propagate(host, l, k, d, None);
        self.mod_insert_player(l, k);
    }

    /// Adds to a base value on the unit's list.
    pub fn unit_add(&mut self, host: &mut dyn StatHost, unit: UnitId, s: u16, d: i32, layer: u16) {
        if let Some(l) = self.unit_list(unit) {
            self.add(host, l, s, d, layer);
        }
    }

    /// Removes all base values and propagates their removal.
    pub fn remove_all(&mut self, host: &mut dyn StatHost, l: ListId) {
        while let Some(&e) = self.try_l(l).and_then(|l| l.base.first()) {
            self.lm(l).base.remove(0);
            self.propagate(host, l, e.key, e.value.wrapping_neg(), None);
            self.mod_insert_player(l, e.key);
        }
    }

    /// Adds each source base entry to the target in sorted order.
    pub fn merge(&mut self, host: &mut dyn StatHost, target: ListId, source: ListId) {
        if !self.is_live(target) {
            return;
        }
        let Some(source) = self.try_l(source) else {
            return;
        };
        for e in source.base.clone() {
            self.add(
                host,
                target,
                key_stat(e.key),
                e.value,
                super::key_layer(e.key),
            );
        }
    }

    // ---- chain operations ---------------------------------------------

    /// Keys of L's full array whose stat has A53 (first 16).
    pub(super) fn a53_keys(&self, l: ListId) -> Vec<i32> {
        self.ext(l)
            .map(|e| {
                e.full
                    .iter()
                    .filter(|e| self.info(key_stat(e.key)).is_some_and(|i| i.a53))
                    .take(16)
                    .map(|e| e.key)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The list values A (as the value send reads them) and the owner.
    fn values(&self, l: ListId) -> (Vec<Entry>, Option<UnitId>) {
        let list = self.l(l);
        match &list.ext {
            Some(e) => (e.full.clone(), Some(e.owner)),
            None => (list.base.clone(), None),
        }
    }

    fn is_damagerelated(&self, k: i32) -> bool {
        self.info(key_stat(k)).is_some_and(|i| i.damagerelated)
    }

    /// Attaches a list to the unit. A reset counts every stat; otherwise
    /// damage-related stats are excluded from propagation.
    pub fn attach(&mut self, host: &mut dyn StatHost, unit: UnitId, l: ListId, reset: bool) {
        let Some(r) = self.unit_list(unit).filter(|&r| self.is_extended(r)) else {
            return;
        };
        if !self.is_live(l) {
            return;
        }
        self.detach(host, l);
        let mut a = Some(r);
        while let Some(x) = a {
            if x == l {
                return;
            }
            // A freed parent (edge case 6) ends the ancestor walk.
            a = self.try_l(x).and_then(|x| x.parent);
        }
        if self.l(l).flags & flag::TEMPONLY != 0 {
            self.lm(r).flags |= flag::NEWLENGTH;
        }
        let parked = self.l(l).flags & flag::SET != 0;
        let head = {
            let e = self.ext(r).expect("extended");
            if parked {
                e.parked
            } else {
                e.active
            }
        };
        {
            let list = self.lm(l);
            list.prev = head;
            list.next = None;
            list.parent = Some(r);
            list.unit = Some(unit);
        }
        if let Some(h) = head {
            self.lm(h).next = Some(l);
        }
        let e = self.ext_mut(r).expect("extended");
        if parked {
            e.parked = Some(l);
            return;
        }
        e.active = Some(l);
        if self.is_extended(l) && self.l(l).flags & flag::PERMANENT != 0 {
            for k in self.a53_keys(l) {
                self.recompute(host, l, k, None);
            }
        }
        let (a, owner) = self.values(l);
        if reset {
            self.lm(l).flags &= !flag::DYNAMIC;
        } else {
            self.lm(l).flags |= flag::DYNAMIC;
        }
        for e in a {
            if !reset && self.is_damagerelated(e.key) {
                continue;
            }
            self.propagate(host, r, e.key, e.value, owner);
        }
    }

    /// Unlinks a list and removes its contribution from its parent. A parked
    /// child of a freed parent has only its own links cleared.
    pub fn detach(&mut self, host: &mut dyn StatHost, l: ListId) {
        let Some(list) = self.try_l(l) else { return };
        let p = list.parent;
        if let Some(p) = p {
            let prev = self.l(l).prev;
            if let Some(e) = self.try_ext_mut(p) {
                if e.active == Some(l) {
                    e.active = prev;
                }
                if e.parked == Some(l) {
                    e.parked = prev;
                }
            }
            self.lm(l).parent = None;
        }
        let (prev, next) = (self.l(l).prev, self.l(l).next);
        if let Some(n) = next {
            self.lm(n).prev = prev;
        }
        if let Some(pv) = prev {
            self.lm(pv).next = next;
        }
        let list = self.lm(l);
        list.prev = None;
        list.next = None;
        let old_unit = list.unit.take();
        if list.flags & flag::SET != 0 {
            return;
        }
        if self.is_extended(l) && self.l(l).flags & flag::PERMANENT != 0 {
            for k in self.a53_keys(l) {
                self.recompute(host, l, k, None);
            }
        }
        if let Some(p) = p {
            let dynamic = self.l(l).flags & flag::DYNAMIC != 0;
            let (a, owner) = self.values(l);
            for e in a {
                if dynamic && self.is_damagerelated(e.key) {
                    continue;
                }
                self.propagate(host, p, e.key, e.value.wrapping_neg(), owner);
            }
        }
        let (state, cb) = (self.l(l).state, self.l(l).remove_callback);
        if let (Some(u), Some(cb)) = (old_unit, cb) {
            host.list_removed(self, u, state, l, cb);
        }
    }

    /// Detaches the list through [`Self::detach`].
    pub fn unit_detach(&mut self, host: &mut dyn StatHost, l: ListId) {
        self.detach(host, l);
    }

    /// Frees a list. Parked children retain a stale parent handle that no
    /// longer resolves to a live list.
    pub fn free(&mut self, host: &mut dyn StatHost, l: ListId) {
        if !self.is_live(l) {
            return;
        }
        self.detach(host, l);
        if self.is_extended(l) {
            let mut cur = self.heads(l).0;
            while let Some(c) = cur {
                {
                    let child = self.lm(c);
                    child.parent = None;
                    child.unit = None;
                }
                if self.is_extended(c) {
                    cur = self.l(c).prev;
                    continue;
                }
                if self.heads(l).0 == Some(c) {
                    let prev = self.l(c).prev;
                    self.ext_mut(l).expect("extended").active = prev;
                }
                self.free(host, c);
                cur = self.heads(l).0;
            }
            let owner = self.ext(l).map(|e| e.owner);
            if let Some(o) = owner {
                if self.units.get(&o) == Some(&l) {
                    self.units.remove(&o);
                }
            }
        }
        let slot = &mut self.slots[l.index as usize];
        slot.1 = None;
        slot.0 = slot.0.wrapping_add(1);
        self.free.push(l.index);
    }

    /// Frees the list only when it is plain.
    pub fn free_plain(&mut self, host: &mut dyn StatHost, l: ListId) {
        if self.is_live(l) && !self.is_extended(l) {
            self.free(host, l);
        }
    }

    /// Equips an item's list on a unit. `swap_location` identifies body
    /// locations 11 and 12.
    pub fn equip(
        &mut self,
        host: &mut dyn StatHost,
        unit: UnitId,
        item_list: Option<ListId>,
        swap_location: bool,
        reset: bool,
    ) {
        let Some(il) = item_list.filter(|&il| self.is_live(il)) else {
            return;
        };
        if swap_location {
            self.detach(host, il);
        } else if self.l(il).unit == Some(unit) {
            let dynamic = self.l(il).flags & flag::DYNAMIC != 0;
            if reset && dynamic {
                self.make_static(host, unit, il, swap_location);
            } else if !reset && !dynamic {
                self.make_dynamic(host, unit, il, swap_location);
            }
        } else {
            self.attach(host, unit, il, reset);
            self.lm(il).unit = Some(unit);
        }
    }

    /// Makes the item's list static on the unit.
    pub fn make_static(&mut self, host: &mut dyn StatHost, unit: UnitId, il: ListId, swap: bool) {
        self.toggle_dynamic(host, unit, il, swap, false);
    }

    /// Makes the item's list dynamic on the unit.
    pub fn make_dynamic(&mut self, host: &mut dyn StatHost, unit: UnitId, il: ListId, swap: bool) {
        self.toggle_dynamic(host, unit, il, swap, true);
    }

    fn toggle_dynamic(
        &mut self,
        host: &mut dyn StatHost,
        unit: UnitId,
        il: ListId,
        swap: bool,
        dynamic: bool,
    ) {
        let Some(list) = self.try_l(il) else { return };
        if list.unit != Some(unit) {
            self.equip(host, unit, Some(il), swap, !dynamic);
            return;
        }
        let Some(r) = self.unit_list(unit).filter(|&r| self.is_extended(r)) else {
            return;
        };
        let is_dynamic = self.l(il).flags & flag::DYNAMIC != 0;
        if is_dynamic == dynamic {
            return;
        }
        let item = self.owner(il);
        if dynamic {
            self.lm(il).flags |= flag::DYNAMIC;
        } else {
            self.lm(il).flags &= !flag::DYNAMIC;
        }
        let (a, _) = self.values(il);
        for e in a {
            if self.is_damagerelated(e.key) {
                let d = if dynamic {
                    e.value.wrapping_neg()
                } else {
                    e.value
                };
                self.propagate(host, r, e.key, d, item);
            }
        }
    }

    /// Parks or unparks the list of a state; returns whether it changed.
    pub fn park(&mut self, host: &mut dyn StatHost, unit: UnitId, state: u32, park: bool) -> bool {
        let Some(r) = self.unit_list(unit) else {
            return false;
        };
        let Some(l) = self.list_of_state(r, state) else {
            return false;
        };
        let parked = self.l(l).flags & flag::SET != 0;
        if parked == park {
            return false;
        }
        self.detach(host, l);
        if park {
            self.lm(l).flags |= flag::SET;
        } else {
            self.lm(l).flags &= !flag::SET;
        }
        self.attach(host, unit, l, true);
        true
    }

    /// Refreshes an item's time-dependent stats on the unit.
    pub fn by_time_refresh(&mut self, host: &mut dyn StatHost, unit: UnitId, item_list: ListId) {
        let Some(r) = self.unit_list(unit).filter(|&r| self.is_extended(r)) else {
            return;
        };
        if !matches!(self.l(r).owner_type, owner::PLAYER | owner::MONSTER) {
            return;
        }
        let entries: Vec<Entry> = self
            .ext(item_list)
            .map(|e| e.full.clone())
            .unwrap_or_default();
        for e in entries {
            let Some(info) = self.info(key_stat(e.key)) else {
                continue;
            };
            if !matches!(info.op, 6 | 7) {
                continue;
            }
            for os in info.op_stats {
                if os == NO_STAT {
                    break;
                }
                let k = key(os, 0);
                let v = self.eval(host, r, k);
                self.set_full(host, r, k, v, Some(unit));
            }
        }
    }

    /// Applies death-time list removal and state persistence rules.
    pub fn death(&mut self, host: &mut dyn StatHost, unit: UnitId) {
        let Some(r) = self.unit_list(unit).filter(|&r| self.is_extended(r)) else {
            return;
        };
        let mut cur = self.heads(r).0;
        while let Some(c) = cur {
            let list = self.l(c);
            let next = list.prev;
            let drop = list.owner_type != owner::ITEM
                && list.flags & 0x181 == 0
                && !host.stays_on_death(self, unit, list.state);
            if drop && list.ext.is_none() {
                self.free(host, c);
                cur = self.heads(r).0;
            } else {
                cur = next;
            }
        }
    }

    /// Removes temporary lists after a unit changes mode.
    pub fn remove_temporary_lists(&mut self, host: &mut dyn StatHost, unit: UnitId) {
        let Some(r) = self.unit_list(unit) else {
            return;
        };
        if self.l(r).flags & flag::NEWLENGTH == 0 {
            return;
        }
        let mut cur = self.heads(r).0;
        while let Some(c) = cur {
            let list = self.l(c);
            let (temp, state, extended, prev) = (
                list.flags & flag::TEMPONLY != 0,
                list.state,
                list.ext.is_some(),
                list.prev,
            );
            // An extended TEMPONLY list cannot exist (extended lists
            // keep only bit 0x1), so it is passed over instead of looping.
            if temp && !extended {
                if state != 0 {
                    self.toggle_state(unit, state, false);
                }
                self.free(host, c);
                cur = self.heads(r).0;
            } else {
                cur = prev;
            }
        }
        self.lm(r).flags &= !flag::NEWLENGTH;
    }

    /// Removes the unit's overlay list when marked for removal.
    pub fn remove_overlay(&mut self, host: &mut dyn StatHost, unit: UnitId) {
        let Some(r) = self.unit_list(unit) else {
            return;
        };
        self.lm(r).flags &= !flag::REMOVE_OVERLAY;
        if let Some(o) = self.list_by_flags(r, flag::OVERLAY) {
            self.remove_all(host, o);
        }
    }

    // ---- queries ------------------------------------------------------

    /// Finds the first list of a state, searching the active chain before the
    /// parked chain.
    pub fn list_of_state(&self, r: ListId, state: u32) -> Option<ListId> {
        let (active, parked) = self.heads(r);
        [active, parked].into_iter().find_map(|head| {
            self.chain(head)
                .into_iter()
                .find(|&c| self.l(c).state == state)
        })
    }

    /// Searches the parked chain when SET is requested, otherwise the active
    /// chain, returning the first list with any requested flag.
    pub fn list_by_flags(&self, r: ListId, flags: u32) -> Option<ListId> {
        let (active, parked) = self.heads(r);
        let head = if flags & flag::SET != 0 {
            parked
        } else {
            active
        };
        self.chain(head)
            .into_iter()
            .find(|&c| self.l(c).flags & flags != 0)
    }

    /// The overlay id for a monster update, when its unit list has the
    /// remove-overlay flag and its overlay list contains stat 178 at layer
    /// zero. The overlay list is plain, so its total equals its base value.
    pub fn overlay_to_send(&self, unit: UnitId) -> Option<i32> {
        let r = self.unit_list(unit)?;
        if self.flags(r) & flag::REMOVE_OVERLAY == 0 {
            return None;
        }
        let o = self.list_by_flags(r, flag::OVERLAY)?;
        Some(self.base(o, 178, 0))
    }

    /// Finds a state list on an extended unit list. SET searches the parked
    /// chain; otherwise the active chain is searched. Non-SET flags must share
    /// at least one bit with the list when supplied.
    pub fn list_by_state_flags(&self, unit: UnitId, state: u32, flags: u32) -> Option<ListId> {
        let r = self.unit_list(unit)?;
        if !self.is_extended(r) {
            return None;
        }
        let (active, parked) = self.heads(r);
        let head = if flags & flag::SET != 0 {
            parked
        } else {
            active
        };
        let f = flags & !flag::SET;
        self.chain(head).into_iter().find(|&c| {
            let l = self.l(c);
            l.state == state && (f == 0 || l.flags & f != 0)
        })
    }

    /// The owner (type, GUID) of the unit's list of `state`, if any.
    pub fn state_list_owner(&self, unit: UnitId, state: u32) -> Option<(u32, u32)> {
        let r = self.unit_list(unit)?;
        let l = self.list_of_state(r, state)?;
        let list = self.l(l);
        Some((list.owner_type, list.owner_guid))
    }

    /// Detaches and frees (when plain) the unit's list of `state`
    /// (regeneration).
    pub fn free_state_list(&mut self, host: &mut dyn StatHost, unit: UnitId, state: u32) {
        let Some(r) = self.unit_list(unit) else {
            return;
        };
        if let Some(l) = self.list_of_state(r, state) {
            self.detach(host, l);
            self.free_plain(host, l);
        }
    }

    // ---- expiry ------------------------------------------------------

    /// Expires lists at the supplied frame. Frame zero uses the client
    /// countdown: decrement first, then expire at zero or below. An expired
    /// extended list prevents the restarted walk from progressing. Returns
    /// [`StatListError::EndlessExpiry`] at that point without changing the
    /// lists further.
    pub fn expire_lists(
        &mut self,
        host: &mut dyn StatHost,
        unit: UnitId,
        frame: i32,
    ) -> Result<(), StatListError> {
        let Some(r) = self.unit_list(unit).filter(|&r| self.is_extended(r)) else {
            return Ok(());
        };
        if frame == 0 {
            for c in self.active_chain(r) {
                let list = self.lm(c);
                if list.flags & flag::NEWLENGTH != 0 {
                    list.expire = list.expire.wrapping_sub(1);
                }
            }
        }
        let mut cur = self.heads(r).0;
        while let Some(c) = cur {
            let list = self.l(c);
            let next = list.prev;
            if list.flags & flag::NEWLENGTH != 0 && list.expire <= frame {
                if list.ext.is_some() {
                    return Err(StatListError::EndlessExpiry(c));
                }
                self.free(host, c);
                cur = self.heads(r).0;
            } else {
                cur = next;
            }
        }
        Ok(())
    }

    // ---- mod array -------------------------------------------------------

    /// Reads the unit's base value for each modified key in order, using zero
    /// for an absent value. The caller sends these values to the client.
    pub fn mod_values(&self, unit: UnitId) -> Vec<(i32, i32)> {
        let Some(r) = self.unit_list(unit) else {
            return Vec::new();
        };
        self.mods(r)
            .into_iter()
            .map(|k| (k, self.raw_base(r, k).unwrap_or(0)))
            .collect()
    }

    /// Clears the unit's modified-stat keys.
    pub fn clear_mods(&mut self, unit: UnitId) {
        if let Some(e) = self.unit_list(unit).and_then(|r| self.ext_mut(r)) {
            e.mods.clear();
        }
    }

    /// Reads a base stat at layer zero only when its key is marked modified and
    /// present in the base array.
    pub fn single_stat(&self, unit: UnitId, s: u16) -> Option<i32> {
        let r = self.unit_list(unit)?;
        let k = key(s, 0);
        if self.ext(r)?.mods.binary_search(&k).is_err() {
            return None;
        }
        self.raw_base(r, k)
    }

    // ---- ---------------------------------------------------------

    /// Clamps current life, mana and stamina to their maxima.
    pub fn clamp_to_max(&mut self, host: &mut dyn StatHost, unit: UnitId) {
        for (cur, max) in [
            (stat::STAMINA, stat::MAXSTAMINA),
            (stat::MANA, stat::MAXMANA),
            (stat::HITPOINTS, stat::MAXHP),
        ] {
            let c = self.unit_total(unit, cur, 0);
            let m = self.unit_total(unit, max, 0);
            if c > m {
                self.unit_add(host, unit, cur, m.wrapping_sub(c), 0);
            }
        }
    }
}
