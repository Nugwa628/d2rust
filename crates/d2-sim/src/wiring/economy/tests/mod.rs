//! Integration tests: the real modules run together on synthetic tables,
//! with a fixed game seed. Only the
//! seams this wiring leaves open ([`super::CubeRest`],
//! [`super::QuestRest`], [`super::DropPlacer`]) are fakes.

mod cube;
mod items;
mod quest_host;
mod quest_players;
mod quest_reward;
mod quests;
mod treasure;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Record};

use super::{Economy, GameFields, ItemStore};
use crate::game::Game;
use crate::items::tables::{ItemRec, PropRec, PropSlot, PropertyRec, UniqueRec};
use crate::items::{ty, ItemStats, ItemTables, ListKey};
use crate::rng::Seed;
use crate::stats::{ClassStats, StatData, StatHost, StatLists, StatTable, StateTable};
use crate::units::hooks::{MonsterInfo, UnitData, UnitHooks};
use crate::units::lifecycle::{allocate, AllocRequest, LifecycleHooks};
use crate::units::record::Units;
use crate::units::{UnitId, UnitType};

/// Stat count of the synthetic itemstatcost.
const N_STATS: usize = 359;
/// Synthetic itemtypes count.
const N_TYPES: usize = 80;
/// Item records (combined index).
pub const CAP: usize = 0;
pub const GOLD: usize = 1;
pub const RING: usize = 2;
pub const AMULET: usize = 3;
pub const HORADRIC_MALUS: usize = 4;
/// Itemtypes rows beyond `items::ty`.
const T_RING: u16 = 10;
const T_AMULET: u16 = 12;
const T_QUEST: u16 = 39;
/// properties row the unique and the craft list use: function 1 on
/// stat 16 (`item_armor_percent`), "roll(min..max)".
pub const PROP_ROW: i32 = 7;
pub const PROP_STAT: u16 = 16;
/// Monster class of the dropping monster.
pub const MONSTER_CLASS: u32 = 5;

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost (no ops, no callbacks, valshift 0) through the
/// d2-data fix-up, as `stats` tests build theirs.
fn itemstatcost() -> BinTable {
    let size = Itemstatcost::SIZE;
    let mut records = vec![0u8; N_STATS * size];
    for s in 0..N_STATS {
        let r = &mut records[s * size..(s + 1) * size];
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            set_u16(r, o, 0xFFFF);
        }
        set_u16(r, 0, s as u16);
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: N_STATS,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    t
}

fn stat_data() -> Arc<StatData> {
    Arc::new(StatData {
        stats: StatTable::from_fixed(&itemstatcost()).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        states: StateTable::synthetic(200, &[]),
        damage_regen: vec![0; 8],
        aurastate: vec![0; 8],
        rescale_precision: crate::stats::DEFAULT_RESCALE_PRECISION,
    })
}

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
    for (c, p) in [
        (ty::HELM, ty::ARMO),
        (ty::GOLD, ty::MISC),
        (T_RING, ty::MISC),
        (T_AMULET, ty::MISC),
        (T_QUEST, ty::MISC),
    ] {
        set(usize::from(c), usize::from(p));
    }
    m
}

fn itemtypes() -> Vec<Itemtypes> {
    (0..N_TYPES)
        .map(|_| {
            let mut t = Itemtypes::decode(&[0u8; Itemtypes::SIZE]);
            t.class = 0xFF;
            t.staffmods = 0xFF;
            t.rare = 1;
            t
        })
        .collect()
}

fn item_rec(t: u16, code: &[u8; 4]) -> ItemRec {
    ItemRec {
        code: *code,
        type_: t as i16,
        level: 1,
        ..ItemRec::default()
    }
}

/// Items: a cap (helm, durability 12, defense 3–5), gold, a ring, an
/// amulet and the Horadric Malus (quest item).
pub fn tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    let mut cap = item_rec(ty::HELM, b"cap ");
    cap.durability = 12;
    cap.minac = 3;
    cap.maxac = 5;
    let mut malus = item_rec(T_QUEST, b"hdm ");
    malus.quest = 3;
    let mut prop = PropertyRec::default();
    prop.slots[0] = PropSlot {
        func: 1,
        stat: PROP_STAT,
        set: 0,
        val: 0,
    };
    let mut properties = vec![PropertyRec::default(); PROP_ROW as usize];
    properties.push(prop);
    let mut props = [PropRec::NONE; 12];
    props[0] = PropRec {
        code: PROP_ROW,
        param: 0,
        min: 10,
        max: 20,
    };
    ItemTables {
        items: vec![
            cap,
            item_rec(ty::GOLD, b"gld "),
            item_rec(T_RING, b"rin "),
            item_rec(T_AMULET, b"amu "),
            malus,
        ],
        itemtypes: itemtypes(),
        equiv: equiv(),
        itemratio: vec![ratio],
        properties,
        uniques: vec![UniqueRec {
            code: *b"cap ",
            enabled: true,
            rarity: 1,
            lvl: 1,
            props,
            ..UniqueRec::default()
        }],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// Hooks with every default; counts stat callbacks of non-item lists.
#[derive(Default)]
pub struct Hooks {
    pub callbacks: usize,
}

impl StatHost for Hooks {
    fn on_callback(&mut self, _: &StatLists, _: &crate::stats::CallbackEvent) {
        self.callbacks += 1;
    }
}
impl UnitHooks for Hooks {}
impl LifecycleHooks for Hooks {}

/// One game's state, owned.
pub struct World {
    pub game: Game,
    pub units: Units,
    pub stats: StatLists,
    pub data: UnitData,
    pub hooks: Hooks,
    pub fields: GameFields,
    pub tables: ItemTables,
    pub items: ItemStore,
}

/// The fixed game seed of every test.
pub const GAME_SEED: u32 = 0x5EED;

impl World {
    pub fn new() -> Self {
        let mut monsters = vec![MonsterInfo::default(); 400];
        monsters[MONSTER_CLASS as usize].enabled = true;
        monsters[usize::from(crate::world::quests::npc::AKARA)].enabled = true;
        Self {
            game: Game::new(),
            units: Units::new(),
            stats: StatLists::new(stat_data()),
            data: UnitData {
                monsters,
                expansion: true,
                ..UnitData::default()
            },
            hooks: Hooks::default(),
            fields: GameFields::new(Seed::init_low(GAME_SEED), true),
            tables: tables(),
            items: ItemStore::new(),
        }
    }

    pub fn econ(&mut self) -> Economy<'_, Hooks> {
        Economy {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
            hooks: &mut self.hooks,
            fields: &mut self.fields,
            tables: &self.tables,
            items: &mut self.items,
        }
    }

    /// A unit through the real allocator.
    pub fn spawn(&mut self, ty: UnitType, class: u32) -> UnitId {
        let req = AllocRequest {
            ty,
            class,
            room: None,
            add: true,
            fixed_guid: None,
            mode: 0,
            allied: false,
        };
        let mut sim = crate::units::hooks::Sim {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
        };
        allocate(&mut sim, &mut self.hooks, &mut self.fields.seed, &req)
            .expect("allocate")
            .expect("allocated")
    }

    /// Sets a unit's base stat (layer 0).
    pub fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.stats.unit_set(&mut self.hooks, unit, stat, value, 0);
    }
}

/// Map oracle of the stats seam (the same rules as the items tests'
/// fake): the real lists must agree with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapStats {
    pub base: BTreeMap<(u16, u16), i32>,
    pub lists: BTreeMap<ListKey, BTreeMap<(u16, u16), i32>>,
}

impl ItemStats for MapStats {
    fn has_stats(&self) -> bool {
        true
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
        self.base.insert((id, layer), value);
    }
    fn has_list(&self, key: ListKey) -> bool {
        self.lists.contains_key(&key)
    }
    fn list_set(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        self.lists
            .entry(key)
            .or_default()
            .insert((id, layer), value);
    }
    fn list_add(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
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
