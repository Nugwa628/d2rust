//! Synthetic tables stand in for the game's records (the costs, levels
//! and multipliers the tests need); facts of the real data are checked
//! against game files elsewhere. The seams are one fake world.

mod gamble;
mod price;
mod store;
mod trade;
mod tsv;

mod mutant_tests;

use std::collections::{BTreeMap, VecDeque};

use d2_data::fixup::maps::EquivMatrix;

use super::*;

// ------------------------------------------------------------------ types

pub const N_TYPES: usize = 64;
pub const T_ARROW: i16 = 5;
pub const T_PLAY: i16 = 7;
pub const T_RING: i16 = 10;
pub const T_AMU: i16 = 12;
pub const T_NOPAGE: i16 = 13;
pub const T_POT: i16 = 14;
pub const T_BOOK: i16 = 18;
pub const T_SCRO: i16 = 22;
pub const T_HELM: i16 = 37;
pub const T_QUEST: i16 = 39;
pub const T_BODY: i16 = 40;
pub const T_THROW: i16 = 42;
pub const T_WEAP: i16 = 45;
pub const T_STAFF: i16 = 46;
pub const T_ARMO: i16 = 50;

fn itemtypes() -> Vec<TypeRec> {
    let mut v = vec![
        TypeRec {
            repair: 1,
            class: 7,
            storepage: 3,
            staffmods: 0xFF,
            ..Default::default()
        };
        N_TYPES
    ];
    let set = |v: &mut Vec<TypeRec>, t: i16, f: &dyn Fn(&mut TypeRec)| f(&mut v[t as usize]);
    set(&mut v, T_ARROW, &|y| {
        y.quiver = 1;
        y.autostack = 1;
    });
    set(&mut v, T_POT, &|y| y.repair = 0);
    set(&mut v, T_SCRO, &|y| y.repair = 0);
    set(&mut v, T_BOOK, &|y| y.repair = 0);
    set(&mut v, T_NOPAGE, &|y| y.storepage = 0xFF);
    set(&mut v, T_WEAP, &|y| y.storepage = 1);
    set(&mut v, T_STAFF, &|y| {
        y.storepage = 1;
        y.staffmods = 7;
    });
    set(&mut v, T_THROW, &|y| {
        y.storepage = 1;
        y.throwable = 1;
    });
    set(&mut v, T_ARMO, &|y| y.storepage = 0);
    set(&mut v, T_HELM, &|y| y.storepage = 0);
    set(&mut v, T_RING, &|y| y.storepage = 2);
    set(&mut v, T_AMU, &|y| y.storepage = 2);
    v
}

/// Identity, plus helm → armor.
fn equiv() -> EquivMatrix {
    let words = N_TYPES.div_ceil(32);
    let mut m = EquivMatrix {
        n: N_TYPES,
        words,
        bits: vec![0; N_TYPES * words],
    };
    let mut set = |i: usize, j: usize| m.bits[i * words + j / 32] |= 1 << (j % 32);
    for i in 0..N_TYPES {
        set(i, i);
    }
    set(T_HELM as usize, T_ARMO as usize);
    m
}

// ------------------------------------------------------------------ items

pub fn code(s: &str) -> [u8; 4] {
    let mut c = *b"    ";
    c[..s.len()].copy_from_slice(s.as_bytes());
    c
}

pub fn item(c: &str, cost: u32, type_: i16) -> VendorItem {
    VendorItem {
        code: code(c),
        normcode: code(c),
        ubercode: NO_CODE,
        ultracode: NO_CODE,
        cost,
        type_,
        type2: -1,
        level: 1,
        spawnable: 1,
        durability: 20,
        nightmare_upgrade: XXX,
        hell_upgrade: XXX,
        ..Default::default()
    }
}

/// The recorded Charsi store (Test vectors): (code, normal count).
pub const CHARSI_STORE: [(&str, u8); 23] = [
    ("hax", 2),
    ("lax", 2),
    ("spc", 1),
    ("ssd", 1),
    ("scm", 2),
    ("dgr", 2),
    ("tkf", 2),
    ("jav", 1),
    ("spr", 2),
    ("bar", 2),
    ("sbw", 2),
    ("hbw", 2),
    ("ktr", 2),
    ("cap", 2),
    ("skp", 1),
    ("qui", 2),
    ("lea", 1),
    ("hla", 1),
    ("buc", 2),
    ("sml", 2),
    ("lgl", 2),
    ("lbt", 2),
    ("lbl", 2),
];

/// Charsi's column.
pub const CHARSI_COL: usize = 2;

/// Synthetic tables. Items (combined index): the Charsi store codes in
/// order, then `axe` (level 7), `aqv`, `cqv`, `skc`, `yps`, `rin`, `amu`,
/// `9ha`, `7ha`, `isc`, `ibk`, `hp4`, `tkn`, `hdm`, `ear`.
pub fn tables() -> VendorTables {
    let mut items = Vec::new();
    for (c, n) in CHARSI_STORE {
        let ty = match c {
            "cap" | "skp" => T_HELM,
            "qui" | "lea" | "hla" | "buc" | "sml" | "lgl" | "lbt" | "lbl" => T_ARMO,
            _ => T_WEAP,
        };
        let mut r = item(c, 100, ty);
        // n_norm = range(Min, Max + 1): Min = n, Max = n − 1 → n, no draw.
        // jav: Min = Max = 1 → one draw, plus one magic item.
        r.columns[CHARSI_COL] = if c == "jav" {
            [1, 1, 1, 0, 1]
        } else {
            [n, n - 1, 0, 1, 0]
        };
        if c == "jav" {
            r.bitfield1 = 1;
        }
        if c == "ktr" {
            r.version = 100;
        }
        if c == "hax" {
            r.cost = 170;
            r.level = 3;
            r.ubercode = code("9ha");
            r.ultracode = code("7ha");
        }
        if c == "dgr" {
            r.cost = 60;
        }
        items.push(r);
    }
    let mut axe = item("axe", 100, T_WEAP);
    axe.level = 7;
    axe.columns[CHARSI_COL] = [1, 1, 0, 1, 0];
    items.push(axe);
    let mut aqv = item("aqv", 256, T_ARROW);
    aqv.stackable = 1;
    aqv.maxstack = 500;
    aqv.durability = 0;
    aqv.perm_store = 1;
    aqv.columns[CHARSI_COL] = [1, 1, 0, 0, 0];
    items.push(aqv);
    let mut cqv = item("cqv", 256, T_ARROW);
    cqv.stackable = 1;
    cqv.maxstack = 350;
    cqv.durability = 0;
    cqv.perm_store = 1;
    cqv.columns[CHARSI_COL] = [1, 1, 0, 0, 0];
    items.push(cqv);
    items.push(item("skc", 1000, T_WEAP));
    let mut yps = item("yps", 40, T_POT);
    yps.durability = 0;
    yps.perm_store = 1;
    yps.columns[0] = [1, 1, 0, 0, 0];
    items.push(yps);
    items.push(item("rin", 1000, T_RING));
    items.push(item("amu", 1000, T_AMU));
    let mut uber = item("9ha", 810, T_WEAP);
    uber.level = 31;
    uber.version = 100;
    items.push(uber);
    let mut ultra = item("7ha", 14033, T_WEAP);
    ultra.level = 54;
    ultra.version = 100;
    items.push(ultra);
    let mut isc = item("isc", 16, T_SCRO);
    isc.durability = 0;
    items.push(isc);
    let mut ibk = item("ibk", 100, T_BOOK);
    ibk.durability = 0;
    items.push(ibk);
    let mut hp4 = item("hp4", 30, T_POT);
    hp4.durability = 0;
    items.push(hp4);
    let mut tkn = item("tkn", 100, T_THROW);
    tkn.stackable = 1;
    tkn.maxstack = 60;
    items.push(tkn);
    let mut hdm = item("hdm", 100, T_QUEST);
    hdm.quest = 1;
    items.push(hdm);
    items.push(item("ear", 1, T_PLAY));
    VendorTables {
        items,
        itemtypes: itemtypes(),
        equiv: equiv(),
        magic: vec![
            CostMod {
                mult: 2048,
                add: 100,
            };
            4
        ],
        uniques: vec![UniqueCost {
            cost: CostMod {
                mult: 5120,
                add: 5000,
            },
            flags: 0,
        }],
        setitems: vec![CostMod {
            mult: 3072,
            add: 1000,
        }],
        books: vec![7, 16],
        skills: vec![
            SkillCost {
                cost: CostMod {
                    mult: 1024,
                    add: 10,
                },
                reqlevel: 12,
            };
            8
        ],
        stats: vec![
            StatCost {
                cost: CostMod { mult: 512, add: 3 },
                valshift: 0,
                encode: 0,
            };
            300
        ],
        stat_shift: 6,
        stat_mask: 0x3F,
        monster_levels: vec![[10, 40, 70]; 4],
        interact: vec![class::GHEED, class::AKARA, class::CHARSI, class::NIHLATHAK],
        npc: npc_rows(),
        difficulty: vec![
            GambleOdds {
                rare: 10000,
                set: 100,
                unique: 50,
                uber: 90,
                ultra: 33,
            };
            3
        ],
        lowquality: vec![b"Cracked".to_vec(), b"Crude".to_vec()],
        gamble_index: Some(vec![]),
        gamble_thresholds: vec![0; 100],
        unique_nosell_mask: crate::world::vendors::UNIQUE_NOSELL_MASK,
    }
}

pub fn index(t: &VendorTables, c: &str) -> usize {
    t.find_code(code(c)).expect(c)
}

fn npc_row(class: u16, sell: i32, quests: [(u32, i32, i32, i32); 3], max_buy: i32) -> NpcPrices {
    NpcPrices {
        class: u32::from(class),
        sell,
        buy: 512,
        rep: 128,
        quests,
        max_buy: [max_buy, 30000, 35000],
    }
}

/// `npc.txt` price rows (the vendors the tests use), plus a test NPC
/// 600 with a large `max buy` and repair multiplier 1024.
pub fn npc_rows() -> Vec<NpcPrices> {
    let a4 = [(4, 922, 1024, 1024), (0, 0, 0, 0), (0, 0, 0, 0)];
    let none = [(0, 0, 0, 0); 3];
    vec![
        npc_row(class::GHEED, 1088, a4, 5000),
        npc_row(class::CHARSI, 960, a4, 5000),
        npc_row(class::AKARA, 1024, a4, 5000),
        npc_row(
            class::MALAH,
            2048,
            [(41, 922, 1024, 1024), (35, 512, 1024, 1024), (0, 0, 0, 0)],
            25000,
        ),
        NpcPrices {
            class: 600,
            sell: 1024,
            buy: 512,
            rep: 1024,
            quests: none,
            max_buy: [1_000_000; 3],
        },
    ]
}

// ------------------------------------------------------------------ fake

#[derive(Clone, Debug, Default)]
pub struct Unit {
    pub record: usize,
    pub quality: u8,
    pub file_index: i32,
    pub flags: u32,
    pub uflags: u32,
    pub mode: u32,
    pub page: u8,
    pub sockets: bool,
    pub stats: BTreeMap<u16, i32>,
    /// Entries and affixes for [`VendorWorld::price_item`].
    pub extra: PriceItem,
}

/// The fake world. Units are keyed by GUID (= `UnitId`).
pub struct Fake {
    pub difficulty: u8,
    pub expansion: bool,
    pub format: u16,
    pub game_type: u8,
    pub units: BTreeMap<u32, Unit>,
    pub next: u32,
    /// NPC GUID → (class, interaction list empty).
    pub npcs: BTreeMap<u32, (u16, bool)>,
    /// Player → interact NPC GUID.
    pub interact: BTreeMap<u32, u32>,
    pub hire_made: Vec<u16>,
    pub quest: BTreeMap<u32, u16>,
    pub players_in: BTreeMap<u16, i32>,
    pub level_id: u16,
    pub gold_cap: i32,
    pub stash_cap: i32,
    pub dropped_gold: i32,
    pub last_bought: u32,
    pub cursor: bool,
    /// Store slots per page.
    pub store_room: [usize; 4],
    pub store_used: [usize; 4],
    pub gamble_room: usize,
    /// Pending create results: Some(true) cracked, None null.
    pub create_queue: VecDeque<Option<bool>>,
    /// Record a creation of record r produces instead.
    pub create_as: BTreeMap<usize, usize>,
    pub created: Vec<(usize, u8, i32)>,
    pub destroyed: Vec<u32>,
    pub trade_inv: Vec<u32>,
    pub removed: Vec<u32>,
    pub taken: Vec<u32>,
    pub sent: Vec<Transaction>,
    pub stat_msgs: Vec<(u32, u16)>,
    pub new_inventories: Vec<(u16, Option<u32>)>,
    pub refreshed: Vec<u32>,
    pub intros: Vec<u16>,
    pub tome: Option<(u32, i32)>,
    pub stack: Option<(u32, i32)>,
    pub belt: bool,
    pub belt_room: usize,
    pub ammo: bool,
    pub backpack_ok: bool,
    pub backpack: Vec<u32>,
    pub equipped: Vec<u32>,
    pub inventory: Vec<u32>,
    pub owned: bool,
    pub cursor_ok: bool,
    pub unequip_ok: bool,
    pub book_lowered: Vec<i32>,
    pub log: Vec<String>,
    /// The game's rules (`Original` unless a mod test sets them).
    pub rules: crate::rules::Ruleset,
}

pub const PLAYER: u32 = 1;
pub const NPC: u32 = 2;

impl Fake {
    pub fn new(npc_class: u16) -> Self {
        let mut f = Fake {
            difficulty: 0,
            expansion: true,
            format: 101,
            game_type: 0,
            units: BTreeMap::new(),
            next: 10,
            npcs: BTreeMap::from([(NPC, (npc_class, false))]),
            interact: BTreeMap::from([(PLAYER, NPC)]),
            hire_made: Vec::new(),
            quest: BTreeMap::new(),
            players_in: BTreeMap::new(),
            level_id: 1,
            gold_cap: 10_000,
            stash_cap: 100_000,
            dropped_gold: 0,
            last_bought: NO_GUID,
            cursor: false,
            store_room: [1000; 4],
            store_used: [0; 4],
            gamble_room: 1000,
            create_queue: VecDeque::new(),
            create_as: BTreeMap::new(),
            created: Vec::new(),
            destroyed: Vec::new(),
            trade_inv: Vec::new(),
            removed: Vec::new(),
            taken: Vec::new(),
            sent: Vec::new(),
            stat_msgs: Vec::new(),
            new_inventories: Vec::new(),
            refreshed: Vec::new(),
            intros: Vec::new(),
            tome: None,
            stack: None,
            belt: false,
            belt_room: 1000,
            ammo: false,
            backpack_ok: true,
            backpack: Vec::new(),
            equipped: Vec::new(),
            inventory: Vec::new(),
            owned: true,
            cursor_ok: true,
            unequip_ok: true,
            book_lowered: Vec::new(),
            log: Vec::new(),
            rules: crate::rules::Ruleset::Original,
        };
        f.units.insert(PLAYER, Unit::default());
        f.set(PLAYER, stat::LEVEL, 1);
        f
    }

    pub fn set(&mut self, unit: u32, id: u16, v: i32) {
        self.units.get_mut(&unit).unwrap().stats.insert(id, v);
    }

    pub fn get(&self, unit: u32, id: u16) -> i32 {
        self.units[&unit].stats.get(&id).copied().unwrap_or(0)
    }

    /// Adds an item unit; returns its GUID.
    pub fn add_item(&mut self, record: usize, quality: u8, flags: u32) -> u32 {
        let g = self.next;
        self.next += 1;
        self.units.insert(
            g,
            Unit {
                record,
                quality,
                file_index: -1,
                flags,
                ..Default::default()
            },
        );
        g
    }

    pub fn unit(&self, g: u32) -> &Unit {
        &self.units[&g]
    }

    pub fn gold(&self) -> i32 {
        self.get(PLAYER, stat::GOLD)
    }
}

fn id(u: UnitId) -> u32 {
    u.0
}

impl NpcLink for Fake {
    fn npc_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.npcs.contains_key(&guid).then_some(UnitId(guid))
    }
    fn npc_class(&self, npc: UnitId) -> u16 {
        self.npcs[&npc.0].0
    }
    fn is_interact_unit(&self, player: UnitId, npc: UnitId) -> bool {
        self.interact.get(&player.0) == Some(&npc.0)
    }
    fn interaction_empty(&self, npc: UnitId) -> bool {
        self.npcs[&npc.0].1
    }
    fn hire_list_made(&self, class: u16) -> bool {
        self.hire_made.contains(&class)
    }
    fn set_hire_list_made(&mut self, class: u16) {
        self.hire_made.push(class);
    }
    fn make_hire_list(&mut self, class: u16, _: &mut crate::rng::Seed) {
        self.log.push(format!("hire list {class}"));
    }
}

impl VendorWorld for Fake {
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn item_format(&self) -> u16 {
        self.format
    }
    fn game_type(&self) -> u8 {
        self.game_type
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn guid(&self, unit: UnitId) -> u32 {
        unit.0
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        (guid == PLAYER).then_some(UnitId(guid))
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        (guid != PLAYER && !self.npcs.contains_key(&guid) && self.units.contains_key(&guid))
            .then_some(UnitId(guid))
    }
    fn stat(&self, unit: UnitId, id: u16, _layer: u16) -> i32 {
        self.get(unit.0, id)
    }
    fn base_stat(&self, unit: UnitId, id: u16, _layer: u16) -> i32 {
        self.get(unit.0, id)
    }
    fn set_stat(&mut self, unit: UnitId, id: u16, _layer: u16, value: i32) {
        self.set(unit.0, id, value);
    }
    fn quest_slot(&self, _player: UnitId, _d: u8, slot: u32) -> u16 {
        self.quest.get(&slot).copied().unwrap_or(0)
    }
    fn players_in_level(&self, level: u16) -> i32 {
        self.players_in.get(&level).copied().unwrap_or(0)
    }
    fn player_level_id(&self, _player: UnitId) -> u16 {
        self.level_id
    }
    fn town_entered(&mut self, _player: UnitId, level: u16) {
        self.intros.push(level);
    }
    fn gold_cap(&self, _player: UnitId) -> i32 {
        self.gold_cap
    }
    fn stash_cap(&self, _player: UnitId) -> i32 {
        self.stash_cap
    }
    fn drop_gold(&mut self, _player: UnitId, amount: i32) {
        self.dropped_gold += amount;
    }
    fn last_bought(&self, _player: UnitId) -> u32 {
        self.last_bought
    }
    fn set_last_bought(&mut self, _player: UnitId, guid: u32) {
        self.last_bought = guid;
    }
    fn has_cursor_item(&self, _player: UnitId) -> bool {
        self.cursor
    }
    fn create_item(
        &mut self,
        _npc_class: u16,
        record: usize,
        quality: u8,
        ilvl: i32,
    ) -> Option<UnitId> {
        self.created.push((record, quality, ilvl));
        let (quality, file_index) = match self.create_queue.pop_front() {
            None => (quality, -1),
            Some(None) => return None,
            Some(Some(true)) => (1, 0),
            Some(Some(false)) => (quality, -1),
        };
        let record = self.create_as.get(&record).copied().unwrap_or(record);
        // Normal-and-below items are created identified.
        let flags = if quality <= 3 { flag::IDENTIFIED } else { 0 };
        let g = self.add_item(record, quality, flags);
        self.units.get_mut(&g).unwrap().file_index = file_index;
        // Worn: durability 10 of 20 (repaired when placed in a store).
        self.set(g, stat::MAXDURABILITY, 20);
        self.set(g, stat::DURABILITY, 10);
        Some(UnitId(g))
    }
    fn copy_item(&mut self, item: UnitId) -> Option<UnitId> {
        let mut u = self.units.get(&item.0)?.clone();
        u.uflags = 0;
        let g = self.next;
        self.next += 1;
        self.units.insert(g, u);
        Some(UnitId(g))
    }
    fn destroy_item(&mut self, item: UnitId) {
        self.destroyed.push(item.0);
    }
    fn item_record(&self, item: UnitId) -> usize {
        self.unit(id(item)).record
    }
    fn item_quality(&self, item: UnitId) -> u8 {
        self.unit(id(item)).quality
    }
    fn item_file_index(&self, item: UnitId) -> i32 {
        self.unit(id(item)).file_index
    }
    fn item_flags(&self, item: UnitId) -> u32 {
        self.unit(id(item)).flags
    }
    fn set_item_flags(&mut self, item: UnitId, flags: u32) {
        self.units.get_mut(&item.0).unwrap().flags = flags;
    }
    fn or_unit_flags(&mut self, item: UnitId, bits: u32) {
        self.units.get_mut(&item.0).unwrap().uflags |= bits;
    }
    fn item_mode(&self, item: UnitId) -> u32 {
        self.unit(id(item)).mode
    }
    fn set_item_mode(&mut self, item: UnitId, mode: u32) {
        self.units.get_mut(&item.0).unwrap().mode = mode;
    }
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        self.units.get_mut(&item.0).unwrap().page = page;
    }
    fn has_filled_sockets(&self, item: UnitId) -> bool {
        self.unit(id(item)).sockets
    }
    fn price_item(&self, item: UnitId) -> Option<PriceItem> {
        let u = self.units.get(&item.0)?;
        let s = |i| u.stats.get(&i).copied().unwrap_or(0);
        Some(PriceItem {
            record: u.record,
            quality: u.quality,
            flags: u.flags,
            file_index: u.file_index,
            quantity: s(stat::QUANTITY),
            armor_base: s(stat::ARMORCLASS),
            indestructible: s(stat::INDESTRUCTIBLE),
            durability: s(stat::DURABILITY),
            max_durability: s(stat::MAXDURABILITY),
            replenish_durability: s(stat::REPLENISH_DURABILITY),
            replenish_quantity: s(stat::REPLENISH_QUANTITY),
            extra_stack: s(stat::EXTRA_STACK),
            ..u.extra.clone()
        })
    }
    fn recharge(&mut self, item: UnitId) {
        self.log.push(format!("recharge {}", item.0));
    }
    fn repair_broken(&mut self, item: UnitId) {
        self.log.push(format!("repair broken {}", item.0));
    }
    fn identify(&mut self, item: UnitId) {
        self.units.get_mut(&item.0).unwrap().flags |= flag::IDENTIFIED;
    }
    fn send_item_stat(&mut self, _player: UnitId, item: UnitId, stat: u16) {
        self.stat_msgs.push((item.0, stat));
    }
    fn send_transaction(&mut self, _player: UnitId, t: Transaction) {
        self.sent.push(t);
    }
    fn new_store_inventory(&mut self, npc_class: u16, npc: Option<UnitId>) {
        self.store_used = [0; 4];
        self.new_inventories.push((npc_class, npc.map(id)));
    }
    fn place_in_store(&mut self, _npc_class: u16, item: UnitId) -> bool {
        let p = usize::from(self.unit(item.0).page);
        if self.store_used[p] < self.store_room[p] {
            self.store_used[p] += 1;
            true
        } else {
            false
        }
    }
    fn remove_store_item(&mut self, _npc_class: u16, item: UnitId) {
        self.removed.push(item.0);
    }
    fn take_from_store(&mut self, _npc_class: u16, item: UnitId) {
        self.taken.push(item.0);
    }
    fn place_in_gamble(&mut self, _npc_class: u16, _player: u32, _item: UnitId) -> bool {
        if self.gamble_room > 0 {
            self.gamble_room -= 1;
            true
        } else {
            false
        }
    }
    fn remove_gamble_item(&mut self, _npc_class: u16, _player: u32, item: UnitId) {
        self.removed.push(item.0);
    }
    fn refresh_npc_inventory(&mut self, npc: UnitId) {
        self.refreshed.push(npc.0);
    }
    fn add_trade_inventory(&mut self, _npc_class: u16, item: UnitId) {
        self.trade_inv.push(item.0);
    }
    fn owns_item(&self, _player: UnitId, _item: UnitId) -> bool {
        self.owned
    }
    fn in_inventory(&self, _player: UnitId, item: UnitId) -> bool {
        self.inventory.contains(&item.0)
    }
    fn equipped_items(&self, _player: UnitId) -> Vec<UnitId> {
        self.equipped.iter().map(|&g| UnitId(g)).collect()
    }
    fn find_tome(&mut self, _player: UnitId, _scroll: UnitId) -> Option<(UnitId, i32)> {
        self.tome.map(|(g, f)| (UnitId(g), f))
    }
    fn add_to_tome(&mut self, tome: UnitId, k: i32) {
        let q = self.get(tome.0, stat::QUANTITY);
        self.set(tome.0, stat::QUANTITY, q + k);
    }
    fn find_partial_stack(&mut self, _player: UnitId, _item: UnitId) -> Option<(UnitId, i32)> {
        self.stack.map(|(g, f)| (UnitId(g), f))
    }
    fn can_belt(&mut self, _player: UnitId, _item: UnitId) -> bool {
        self.belt
    }
    fn put_in_belt(&mut self, _player: UnitId, _item: UnitId) -> bool {
        if self.belt_room > 0 {
            self.belt_room -= 1;
            true
        } else {
            false
        }
    }
    fn equip_ammo(&mut self, _player: UnitId, _item: UnitId) -> bool {
        self.ammo
    }
    fn place_in_backpack(&mut self, _player: UnitId, item: UnitId) -> bool {
        if self.backpack_ok {
            self.backpack.push(item.0);
        }
        self.backpack_ok
    }
    fn take_from_cursor(&mut self, _player: UnitId, _item: UnitId) -> bool {
        self.cursor_ok
    }
    fn lower_book_skill(&mut self, _player: UnitId, _item: UnitId, n: i32) {
        self.book_lowered.push(n);
    }
    fn remove_stored(&mut self, _player: UnitId, item: UnitId) {
        self.removed.push(item.0);
    }
    fn unequip(&mut self, _player: UnitId, _item: UnitId) -> bool {
        self.unequip_ok
    }
}

pub fn p() -> UnitId {
    UnitId(PLAYER)
}

pub fn npc() -> UnitId {
    UnitId(NPC)
}

/// A record of `class` built from the synthetic tables' global lists.
pub fn record(t: &VendorTables, class: u16, act: u8) -> VendorRecord {
    VendorRecord::new(class, act, true, &GlobalLists::build(t))
}
