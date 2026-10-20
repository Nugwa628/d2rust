//! Tests from the item specs' test vectors and edge cases, on synthetic
//! tables and small fakes of the seams.

mod affixes;
mod cov_items;
mod create;
mod format0;
mod gaps_affixes;
mod gaps_generation;
mod gaps_props;
mod gaps_quality;
mod mod_props;
mod mod_quality;
mod mutants;
#[cfg(test)]
mod prop_legal_affixes;
mod props;
mod quality;

use std::collections::BTreeMap;

use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{Itemratio, Itemtypes, Record};

use super::tables::{AffixRec, ItemRec, ItemTables, PropRec, PropSlot, PropertyRec};
use super::{ty, Item, ItemGame, ItemStats, ListKey, UniqueBits};
use crate::rng::Seed;

/// Fake of the stats seam: base stats and keyed lists in ordered maps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FakeStats {
    pub any: bool,
    pub base: BTreeMap<(u16, u16), i32>,
    pub lists: BTreeMap<ListKey, BTreeMap<(u16, u16), i32>>,
}

impl ItemStats for FakeStats {
    fn has_stats(&self) -> bool {
        self.any
    }
    fn stat(&self, id: u16, layer: u16) -> i32 {
        self.base(id, layer)
            + self
                .lists
                .values()
                .map(|l| l.get(&(id, layer)).copied().unwrap_or(0))
                .sum::<i32>()
    }
    fn base(&self, id: u16, layer: u16) -> i32 {
        self.base.get(&(id, layer)).copied().unwrap_or(0)
    }
    fn set_base(&mut self, id: u16, layer: u16, value: i32) {
        self.any = true;
        self.base.insert((id, layer), value);
    }
    fn has_list(&self, key: ListKey) -> bool {
        self.lists.contains_key(&key)
    }
    fn list_set(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        self.any = true;
        self.lists
            .entry(key)
            .or_default()
            .insert((id, layer), value);
    }
    fn list_add(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        self.any = true;
        *self
            .lists
            .entry(key)
            .or_default()
            .entry((id, layer))
            .or_default() += value;
    }
    fn list_get(&self, key: ListKey, id: u16, layer: u16) -> i32 {
        self.lists
            .get(&key)
            .and_then(|l| l.get(&(id, layer)))
            .copied()
            .unwrap_or(0)
    }
}

impl FakeStats {
    /// The item list (state 0, flags 0x40).
    pub fn item_list(&self, id: u16, layer: u16) -> i32 {
        self.list_get(ListKey::ITEM, id, layer)
    }
}

/// Fake of the game seam.
pub struct FakeGame {
    pub seed: Seed,
    pub difficulty: u8,
    pub expansion: bool,
    pub ladder: (bool, bool),
    pub uniques: UniqueBits,
}

impl Default for FakeGame {
    fn default() -> Self {
        Self {
            seed: Seed::init_low(12345),
            difficulty: 0,
            expansion: true,
            ladder: (false, false),
            uniques: UniqueBits::default(),
        }
    }
}

impl ItemGame for FakeGame {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn seed(&mut self) -> &mut Seed {
        &mut self.seed
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn ladder_flags(&self) -> (bool, bool) {
        self.ladder
    }
    fn uniques(&mut self) -> &mut UniqueBits {
        &mut self.uniques
    }
}

/// Synthetic itemtypes row indices beyond the named ones.
pub const AXE: u16 = 28;
pub const THROWN: u16 = 42;
pub const RING: u16 = 10;
pub const N_TYPES: usize = 80;

/// Parent links of the synthetic itemtypes (child → parents).
const PARENTS: &[(u16, &[u16])] = &[
    (ty::SHIE, &[ty::ARMO]),
    (ty::TORS, &[ty::ARMO]),
    (ty::HELM, &[ty::ARMO]),
    (ty::BOOT, &[ty::ARMO]),
    (ty::GLOV, &[ty::ARMO]),
    (ty::BELT, &[ty::ARMO]),
    (AXE, &[ty::WEAP]),
    (THROWN, &[ty::WEAP]),
    (ty::STAF, &[ty::WEAP]),
    (ty::BOW, &[ty::WEAP]),
    (ty::SCEP, &[ty::WEAP]),
    (ty::WAND, &[ty::WEAP]),
    (ty::XBOW, &[ty::WEAP]),
    (ty::GOLD, &[ty::MISC]),
    (ty::ELIX, &[ty::MISC]),
    (ty::CHAR, &[ty::MISC]),
    (ty::JEWL, &[ty::MISC]),
    (ty::GEM, &[ty::MISC]),
    (ty::RUNE, &[ty::MISC]),
    (ty::BOOK, &[ty::MISC]),
    (ty::SCRO, &[ty::MISC]),
    (RING, &[ty::MISC]),
    (ty::PLAY, &[ty::MISC]),
    (ty::BODY, &[ty::MISC]),
];

fn equiv() -> EquivMatrix {
    let n = N_TYPES;
    let words = n.div_ceil(32);
    let mut m = EquivMatrix {
        n,
        words,
        bits: vec![0; n * words],
    };
    let mut set = |i: usize, j: usize| m.bits[i * words + j / 32] |= 1 << (j % 32);
    for i in 1..n {
        set(i, 0);
        set(i, i);
    }
    for &(c, ps) in PARENTS {
        for &p in ps {
            set(c as usize, p as usize);
        }
    }
    m
}

fn itemtypes() -> Vec<Itemtypes> {
    let mut v: Vec<Itemtypes> = (0..N_TYPES)
        .map(|_| Itemtypes::decode(&[0u8; Itemtypes::SIZE]))
        .collect();
    for it in &mut v {
        it.class = 0xFF;
        it.staffmods = 0xFF;
        it.rare = 1;
    }
    v[THROWN as usize].throwable = 1;
    v[ty::CHAR as usize].magic = 1;
    v[ty::GEM as usize].normal = 1;
    v[ty::RUNE as usize].normal = 1;
    v
}

/// A properties row with slot 0 = (func, stat), the rest empty.
pub fn prop1(func: u8, stat: u16) -> PropertyRec {
    let mut p = PropertyRec::default();
    p.slots[0] = PropSlot {
        func,
        stat,
        set: 0,
        val: 0,
    };
    p
}

pub fn rec(code: i32, param: i32, min: i32, max: i32) -> PropRec {
    PropRec {
        code,
        param,
        min,
        max,
    }
}

/// A fitting affix row: itype `itype`, level 1, frequency 1, group = id.
pub fn affix_row(itype: u16, group: i32) -> AffixRec {
    AffixRec {
        spawnable: 1,
        level: 1,
        group,
        rare: 1,
        classspecific: 0xFF,
        frequency: 1,
        itype: [itype as i16, 0, 0, 0, 0, 0, 0],
        mods: [PropRec::NONE; 3],
        ..Default::default()
    }
}

/// Tables with the synthetic itemtypes, one ratio row, 360 stats.
pub fn tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    ItemTables {
        itemtypes: itemtypes(),
        equiv: equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; 360],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..Default::default()
    }
}

/// An item record of type `t`, code `c`.
pub fn item_rec(t: u16, c: &[u8; 4]) -> ItemRec {
    ItemRec {
        code: *c,
        type_: t as i16,
        type2: 0,
        ..Default::default()
    }
}

/// Adds an item record, returning its combined index.
pub fn push_item(t: &mut ItemTables, r: ItemRec) -> usize {
    t.items.push(r);
    t.items.len() - 1
}

/// A fresh item of record `i` with format 101 and item seed `seed`.
pub fn item(i: usize, seed: u32) -> Item<FakeStats> {
    let mut it = Item::new(i, 101, FakeStats::default());
    it.ilvl = 50;
    it.item_seed = Seed::init_low(seed);
    it.start_seed = seed;
    it
}

/// The first low word x ≥ 0 such that `f(Seed::init_low(x))` holds.
pub fn find_seed(f: impl Fn(&mut Seed) -> bool) -> u32 {
    (0u32..)
        .find(|&x| f(&mut Seed::init_low(x)))
        .expect("a seed exists")
}
