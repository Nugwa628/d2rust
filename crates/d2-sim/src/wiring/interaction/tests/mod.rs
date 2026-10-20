//! Integration tests for the wired modules: the real modules run together
//! through the interaction adapters on synthetic tables with fixed seeds.
//! Only the seams this wiring leaves open ([`super::NpcRest`],
//! [`super::VendorRest`], the economy's `QuestRest`, [`super::UseRest`],
//! [`super::VitalsRest`], the action wiring's `Pending`) are fakes.

mod amazon;
mod mod_events;
mod mod_stat_callbacks;
mod npc;
mod quest_npc;
mod regen;
mod skill_bodies;
mod skill_events;
mod skill_use;
mod summon;
mod unit_events;
mod vendors;
mod vitals;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Monstats, Record};

use super::{Desk, HirelingRest, InteractionState, NpcRest, PlayerQuestsRef, VendorRest};
use crate::game::Game;
use crate::items::tables::ItemRec;
use crate::items::{ty, ItemTables};
use crate::rng::Seed;
use crate::stats::{ClassStats, StatData, StatHost, StatLists, StatTable, StateTable};
use crate::units::hooks::{MonsterInfo, Sim, UnitData, UnitHooks};
use crate::units::lifecycle::{allocate, AllocRequest, LifecycleHooks};
use crate::units::record::Units;
use crate::units::{UnitId, UnitType};
use crate::wiring::economy::{Economy, GameFields, ItemStore, QuestRest};
use crate::world::hirelings::{HirelingRow, HirelingRows, HirelingTables};
use crate::world::npc::{class, HireRow, ImbueMods, InvEntry, ItemFacts, NpcControl, Place};
use crate::world::quests::{
    PlayerQuests, QuestChain, QuestControl, QuestTables, TextList, UnitKind,
};
use crate::world::vendors::price::Bonus;
use crate::world::vendors::{
    GambleOdds, GlobalLists, NpcPrices, Transaction, TypeRec, VendorItem, VendorTables, NO_CODE,
    XXX,
};

/// Stat count of the synthetic itemstatcost.
pub const N_STATS: usize = 359;
/// Synthetic itemtypes count.
const N_TYPES: usize = 80;
/// Synthetic states count.
pub const N_STATES: usize = 200;
/// A curable state (flag bit 12, `fields.tsv`) and a non-curable one.
pub const S_CURABLE: u16 = 60;
pub const S_PLAIN: u16 = 61;
/// Monstats rows (past the hireling classes, 359).
const N_MONSTATS: usize = 400;
/// Item records (combined index; the vendor tables use the same order).
pub const CAP: usize = 0;
pub const GOLD: usize = 1;
/// The game seed every world starts from.
pub const GAME_SEED: u32 = 0x5EED;
/// Gold of a new test player.
pub const PLAYER_GOLD: i32 = 5000;

pub mod st {
    pub const LIFE: u16 = 6;
    pub const MAXHP: u16 = 7;
    pub const MANA: u16 = 8;
    pub const MAXMANA: u16 = 9;
    pub const STAMINA: u16 = 10;
    pub const MAXSTAMINA: u16 = 11;
    pub const LEVEL: u16 = 12;
    pub const EXPERIENCE: u16 = 13;
    pub const GOLD: u16 = 14;
    pub const DURABILITY: u16 = 72;
    pub const MAXDURABILITY: u16 = 73;
}

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost (no ops, no callbacks, valshift 0) through the
/// d2-data fix-up, as the `stats` tests build theirs.
pub(super) fn itemstatcost() -> BinTable {
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

/// Stat data: plain stats, mana regeneration 4 for every class, one
/// curable state.
pub fn stat_data() -> Arc<StatData> {
    let class = ClassStats {
        mana_regen: 4,
        ..ClassStats::default()
    };
    Arc::new(StatData {
        stats: StatTable::from_fixed(&itemstatcost()).expect("itemstatcost"),
        classes: vec![class; 7],
        states: StateTable::synthetic(
            N_STATES,
            &[(u32::from(S_CURABLE), super::npc_world::STATE_CURABLE)],
        ),
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
    set(usize::from(ty::HELM), usize::from(ty::ARMO));
    set(usize::from(ty::GOLD), usize::from(ty::MISC));
    m
}

/// Items: a cap (helm, durability 12, defense 3–5) and gold.
pub fn item_tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 0;
    let rec = |t: u16, code: &[u8; 4]| ItemRec {
        code: *code,
        type_: t as i16,
        level: 1,
        ..ItemRec::default()
    };
    let mut cap = rec(ty::HELM, b"cap ");
    cap.durability = 12;
    cap.minac = 3;
    cap.maxac = 5;
    ItemTables {
        items: vec![cap, rec(ty::GOLD, b"gld ")],
        itemtypes: (0..N_TYPES)
            .map(|_| {
                let mut t = Itemtypes::decode(&[0u8; Itemtypes::SIZE]);
                t.class = 0xFF;
                t.staffmods = 0xFF;
                t.rare = 1;
                t
            })
            .collect(),
        equiv: equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// The vendor tables over the same items: the cap is a permanent store
/// item of Akara (column 0) and Asheara (column 10); one `npc.txt` row
/// each (sell 1024, buy 512, repair 128, max buy 5000).
pub fn vendor_tables() -> VendorTables {
    let code = |c: &[u8; 4]| *c;
    let mut cap = VendorItem {
        code: code(b"cap "),
        normcode: code(b"cap "),
        ubercode: NO_CODE,
        ultracode: NO_CODE,
        cost: 100,
        type_: ty::HELM as i16,
        type2: -1,
        level: 1,
        spawnable: 1,
        durability: 12,
        minac: 3,
        maxac: 5,
        nightmare_upgrade: XXX,
        hell_upgrade: XXX,
        perm_store: 1,
        ..VendorItem::default()
    };
    cap.columns[0] = [1, 1, 0, 0, 0];
    cap.columns[10] = [1, 1, 0, 0, 0];
    let gold = VendorItem {
        code: code(b"gld "),
        normcode: code(b"gld "),
        ubercode: NO_CODE,
        ultracode: NO_CODE,
        type_: ty::GOLD as i16,
        type2: -1,
        nightmare_upgrade: XXX,
        hell_upgrade: XXX,
        ..VendorItem::default()
    };
    let mut itemtypes = vec![
        TypeRec {
            repair: 1,
            class: 7,
            storepage: 3,
            staffmods: 0xFF,
            ..TypeRec::default()
        };
        N_TYPES
    ];
    itemtypes[usize::from(ty::HELM)].storepage = 0;
    let row = |c: u16| NpcPrices {
        class: u32::from(c),
        sell: 1024,
        buy: 512,
        rep: 128,
        quests: [(0, 0, 0, 0); 3],
        max_buy: [5000; 3],
    };
    VendorTables {
        items: vec![cap, gold],
        itemtypes,
        equiv: equiv(),
        stat_shift: 6,
        stat_mask: 0x3F,
        monster_levels: vec![[1, 1, 1]; N_MONSTATS],
        interact: vec![class::AKARA, class::ASHEARA],
        npc: vec![row(class::AKARA), row(class::ASHEARA)],
        difficulty: vec![GambleOdds::default(); 3],
        ..VendorTables::default()
    }
}

/// Monstats: Akara, Kashya, Asheara and the Cain of act 2 are `npc` and
/// `interact`; class 0 is a plain monster.
pub fn monstats() -> Vec<Monstats> {
    let mut v: Vec<Monstats> = (0..N_MONSTATS)
        .map(|_| Monstats::decode(&vec![0u8; Monstats::SIZE]))
        .collect();
    for c in [class::AKARA, class::KASHYA, class::ASHEARA, class::CAIN2] {
        v[usize::from(c)].npc = true;
        v[usize::from(c)].interact = true;
    }
    v
}

/// `hireling` rows: Asheara's Normal row (names 10–14) and its act-3 row
/// for the hire init (level 9, gold 150); Kashya's Normal row.
pub fn hirelings(version: u16) -> Vec<HireRow> {
    vec![
        HireRow {
            version,
            class: 271,
            act: 1,
            difficulty: 1,
            seller: u32::from(class::KASHYA),
            gold: 100,
            level: 3,
            name_first: 1,
            name_last: 5,
        },
        HireRow {
            version,
            class: 359,
            act: 3,
            difficulty: 1,
            seller: u32::from(class::ASHEARA),
            gold: 150,
            level: 9,
            name_first: 10,
            name_last: 14,
        },
    ]
}

/// The hireling tables of the same rows: Kashya's row is `Id` 0,
/// Asheara's `Id` 15; the per-level columns are 0; `pettype` row 7 as in
/// the game (warp, basemax 1), MaxLvl 99.
pub fn hireling_tables(rows: &[HireRow]) -> HirelingTables {
    let rows = rows
        .iter()
        .map(|r| HirelingRow {
            version: r.version,
            id: if r.act == 3 { 15 } else { 0 },
            class: r.class,
            act: r.act,
            difficulty: r.difficulty,
            seller: r.seller,
            gold: r.gold as i32,
            level: r.level as i32,
            name_first: r.name_first,
            name_last: r.name_last,
            ..HirelingRow::default()
        })
        .collect();
    HirelingTables {
        rows: HirelingRows::new(rows),
        exp_ratios: Default::default(),
        max_level: 99,
        pet_flags: HirelingTables::WARP,
        pet_basemax: 1,
    }
}

/// Hooks with every default.
#[derive(Default)]
pub struct Hooks;

impl StatHost for Hooks {}
impl UnitHooks for Hooks {}
impl LifecycleHooks for Hooks {}

/// A fake of every seam the wiring leaves open; logs what matters.
#[derive(Default)]
pub struct Rest {
    pub item_format: u16,
    pub distance: i32,
    pub quests: BTreeMap<UnitId, PlayerQuests>,
    pub pets: BTreeMap<UnitId, UnitId>,
    pub inventory: Vec<InvEntry>,
    /// What the fake mercenary spawn hands out.
    pub merc: Option<UnitId>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
    pub transactions: Vec<(UnitId, Transaction)>,
    pub store: Vec<(u16, UnitId)>,
    pub last_bought: BTreeMap<UnitId, u32>,
    pub log: Vec<String>,
    /// The quest sends (first byte; 0x27 for a text list) and the
    /// mercenary spawn / init, in call order.
    pub trace: Vec<String>,
}

impl Rest {
    pub fn new() -> Self {
        Self {
            // Item format ≥ 1: the request's quality is kept.
            item_format: 1,
            distance: 3,
            ..Self::default()
        }
    }
}

impl PlayerQuestsRef for Rest {
    fn quests_ref(&self, player: UnitId) -> Option<&PlayerQuests> {
        self.quests.get(&player)
    }
}

impl NpcRest for Rest {
    fn item_format(&self) -> u16 {
        self.item_format
    }
    fn distance(&self, _: UnitId, _: UnitId) -> i32 {
        self.distance
    }
    fn axis_check(&self, _: UnitId, _: UnitId) -> u32 {
        0
    }
    fn unit_check(&self, _: UnitId, _: u32) -> u32 {
        0
    }
    fn clear_path(&mut self, u: UnitId) {
        self.log.push(format!("clear path {}", u.0));
    }
    fn approach(&mut self, _: UnitId, _: UnitId) {}
    fn player_busy(&self, _: UnitId) -> u32 {
        0
    }
    fn start_allowed(&self, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn tristram_cain_busy(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn pet(&self, player: UnitId, _: u8, _: u8) -> Option<UnitId> {
        self.pets.get(&player).copied()
    }
    fn pets(&self, player: UnitId) -> Vec<UnitId> {
        self.pets.get(&player).copied().into_iter().collect()
    }
    fn player_name(&self, _: UnitId) -> Vec<u8> {
        b"tester".to_vec()
    }
    fn reset_stats(&mut self, _: UnitId) {}
    fn reset_skills(&mut self, _: UnitId) {}
    fn act_change(&mut self, _: UnitId, _: u32, _: u32) {}
    fn activate_waypoint(&mut self, _: UnitId, _: u32) {}
    fn npc_ai_param(&mut self, npc: UnitId, p: u32) {
        self.log.push(format!("ai param {} {p:#x}", npc.0));
    }
    fn respec_sound(&mut self, _: UnitId) {}
    fn encode_text_list(&self, _: &TextList) -> [u8; 34] {
        [0; 34]
    }
    fn socket_granted(&mut self, _: UnitId) {}
    fn personalize_granted(&mut self, _: UnitId) {}
    fn inventory_entries(&self, _: UnitId) -> Vec<InvEntry> {
        self.inventory.clone()
    }
    fn identify(&mut self, item: UnitId) {
        self.log.push(format!("identify {}", item.0));
    }
    fn cursor_item(&self, _: UnitId) -> Option<UnitId> {
        None
    }
    fn item_facts(&self, _: UnitId) -> ItemFacts {
        ItemFacts::default()
    }
    fn put_back(&mut self, _: UnitId, _: UnitId) {}
    fn remove_cursor_item(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn duplicate(&mut self, _: UnitId, _: UnitId) -> Option<UnitId> {
        None
    }
    fn create_imbued(&mut self, _: UnitId, _: UnitId, _: &ImbueMods) -> Option<UnitId> {
        None
    }
    fn item_refresh(&mut self, _: UnitId) {}
    fn personal_name(&self, _: UnitId) -> Vec<u8> {
        Vec::new()
    }
    fn set_personal_name(&mut self, _: UnitId, _: &[u8]) {}
    fn place_or_drop(&mut self, _: UnitId, _: UnitId) {}
    fn spawn_mercenary(&mut self, _: UnitId, class: u32, mode: u8) -> Option<UnitId> {
        self.log.push(format!("spawn merc {class} {mode}"));
        self.trace.push(format!("spawn merc {mode}"));
        self.merc
    }
}

impl HirelingRest for Rest {
    fn set_mode(&mut self, u: UnitId, mode: u8) {
        self.log.push(format!("mode {} {mode}", u.0));
    }
    fn skill_count(&self) -> u32 {
        0
    }
    fn skill_reqlevel(&self, _: u32) -> Option<i16> {
        None
    }
    fn set_skill_level(&mut self, _: UnitId, _: u32, _: i32) {}
    fn set_owner(&mut self, _: UnitId, _: u32, _: u8) {}
    fn owner(&self, _: UnitId) -> Option<(u32, u8)> {
        None
    }
    fn free_unit(&mut self, _: UnitId) {}
    fn queue_room_removal(&mut self, _: UnitId) {}
    fn death_event(&mut self, _: UnitId) {}
    fn dismiss(&mut self, _: UnitId) {}
    fn warp_to(&mut self, _: UnitId, _: UnitId) {}
    fn level_events(&mut self, _: UnitId, _: UnitId) {}
    fn reapply_item_stats(&mut self, _: UnitId) {}
}

impl VendorRest for Rest {
    fn players_in_level(&self, _: u16) -> i32 {
        1
    }
    fn player_level_id(&self, _: UnitId) -> u16 {
        1
    }
    fn gold_cap(&self, _: UnitId) -> i32 {
        100_000
    }
    fn stash_cap(&self, _: UnitId) -> i32 {
        100_000
    }
    fn drop_gold(&mut self, _: UnitId, _: i32) {}
    fn last_bought(&self, p: UnitId) -> u32 {
        self.last_bought.get(&p).copied().unwrap_or(u32::MAX)
    }
    fn set_last_bought(&mut self, p: UnitId, guid: u32) {
        self.last_bought.insert(p, guid);
    }
    fn has_cursor_item(&self, _: UnitId) -> bool {
        false
    }
    /// The item copy is not written yet: the fake hands back the
    /// store item itself (its fields and stats are the real ones).
    fn copy_item(&mut self, item: UnitId) -> Option<UnitId> {
        self.log.push(format!("copy {}", item.0));
        Some(item)
    }
    fn has_filled_sockets(&self, _: UnitId) -> bool {
        false
    }
    fn socketed(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn price_bonuses(&self, _: UnitId) -> Vec<Bonus> {
        Vec::new()
    }
    fn recharge(&mut self, _: UnitId) {}
    fn repair_broken(&mut self, _: UnitId) {}
    fn send_transaction(&mut self, p: UnitId, t: Transaction) {
        self.transactions.push((p, t));
    }
    fn new_store_inventory(&mut self, _: u16, _: Option<UnitId>) {}
    fn place_in_store(&mut self, class: u16, item: UnitId) -> bool {
        self.store.push((class, item));
        true
    }
    fn remove_store_item(&mut self, _: u16, _: UnitId) {}
    fn take_from_store(&mut self, _: u16, _: UnitId) {}
    fn place_in_gamble(&mut self, _: u16, _: u32, _: UnitId) -> bool {
        true
    }
    fn remove_gamble_item(&mut self, _: u16, _: u32, _: UnitId) {}
    fn refresh_npc_inventory(&mut self, _: UnitId) {}
    fn add_trade_inventory(&mut self, class: u16, item: UnitId) {
        self.log.push(format!("trade inv {class} {}", item.0));
    }
    fn owns_item(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn in_inventory(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn equipped_items(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn find_tome(&mut self, _: UnitId, _: UnitId) -> Option<(UnitId, i32)> {
        None
    }
    fn add_to_tome(&mut self, _: UnitId, _: i32) {}
    fn find_partial_stack(&mut self, _: UnitId, _: UnitId) -> Option<(UnitId, i32)> {
        None
    }
    fn can_belt(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn put_in_belt(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn equip_ammo(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn place_in_backpack(&mut self, _: UnitId, item: UnitId) -> bool {
        self.log.push(format!("backpack {}", item.0));
        true
    }
    fn take_from_cursor(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn lower_book_skill(&mut self, _: UnitId, _: UnitId, _: i32) {}
    fn remove_stored(&mut self, _: UnitId, _: UnitId) {}
    fn unequip(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
}

impl QuestRest for Rest {
    fn has_act2(&self) -> bool {
        false
    }
    fn players(&self) -> Vec<UnitId> {
        self.quests.keys().copied().collect()
    }
    fn first_client_player(&self) -> Option<UnitId> {
        self.quests.keys().next().copied()
    }
    fn quests(&mut self, player: UnitId) -> Option<&mut PlayerQuests> {
        self.quests.get_mut(&player)
    }
    fn player_byte_4c(&self, _: UnitId) -> u8 {
        0
    }
    fn set_player_byte_4c(&mut self, _: UnitId, _: u8) {}
    fn quest_chain(&mut self, _: UnitId) -> Option<&mut QuestChain> {
        None
    }
    fn unit_act(&self, _: UnitId) -> Option<u8> {
        Some(0)
    }
    fn unit_level(&self, _: UnitId) -> Option<u32> {
        Some(1)
    }
    fn unit_kind(&self, _: UnitId) -> UnitKind {
        UnitKind::Other
    }
    fn players_near(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn party_members(&self, _: UnitId) -> Option<Vec<UnitId>> {
        None
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.trace.push(format!("{:#04x}", msg[0]));
        self.sent.push((player, msg.to_vec()));
    }
    fn send_text_list(&mut self, _: UnitId, _: UnitId, _: &[(u16, u32)]) {
        self.trace.push("0x27".into());
    }
    fn inventory(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn delete_item(&mut self, _: UnitId, _: [u8; 4]) {}
    fn reward_item(&mut self, _: UnitId, _: [u8; 4], _: i32, _: u8, _: bool) -> Option<UnitId> {
        None
    }
    fn drop_item_at(&mut self, _: UnitId, _: [u8; 4], _: u8) -> bool {
        false
    }
    fn den_region(&self) -> (u32, u32, u32, u32) {
        (0, 0, 0, 0)
    }
    fn true_tomb_level(&self) -> u32 {
        0
    }
    fn free_spot(&mut self, _: UnitId, _: u32, _: u32, _: u32, _: u32) -> Option<(i32, i32)> {
        None
    }
    fn create_portal(&mut self, _: UnitId, _: i32, _: i32, _: u16, _: u32) -> bool {
        false
    }
    fn schedule_quest_event(&mut self, _: UnitId, _: i32) {}
    fn object_mode(&self, _: UnitId) -> i32 {
        0
    }
    fn set_object_mode(&mut self, _: UnitId, _: i32) {}
    fn mercenary_reward(&mut self, _: UnitId, _: u16) {}
    fn unit_position(&self, _: UnitId) -> Option<(i32, i32, crate::units::RoomId)> {
        None
    }
    fn room_contains(&self, _: crate::units::RoomId, _: i32, _: i32) -> bool {
        false
    }
    fn room_at(&self, _: crate::units::RoomId, _: i32, _: i32) -> Option<crate::units::RoomId> {
        None
    }
    fn free_spot_at(
        &mut self,
        _: crate::units::RoomId,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
    ) -> Option<(i32, i32, crate::units::RoomId)> {
        None
    }
    fn spawn_monster(
        &mut self,
        _: crate::units::RoomId,
        _: i32,
        _: i32,
        _: u16,
        _: u8,
        _: u32,
    ) -> Option<UnitId> {
        None
    }
    fn or_unit_flags(&mut self, _: UnitId, _: u32) {}
    fn monsters(&self) -> Vec<UnitId> {
        Vec::new()
    }
    fn npc_chat_clients(&self, _: UnitId) -> Option<Vec<UnitId>> {
        None
    }
    fn remove_monster(&mut self, _: UnitId) {}
    fn drop_preset_monster(&mut self, _: u8, _: u16) {}
    fn find_object_near(&self, _: UnitId, _: u16) -> Option<UnitId> {
        None
    }
    fn create_object(&mut self, _: crate::units::RoomId, _: i32, _: i32, _: u16) -> Option<UnitId> {
        None
    }
    fn object_anim_length(&self, _: UnitId) -> i32 {
        0
    }
    fn schedule_object_event(&mut self, _: UnitId, _: u8, _: i32) {}
    fn open_quest_message(&mut self, _: UnitId, _: UnitId, _: u16) {}
    fn unhandled(&mut self, chain: u8, function: u32) {
        self.log.push(format!("unhandled {chain} {function:#x}"));
    }
}

/// One game's interaction world: the real game, units, stats, items,
/// quests, NPC control and vendor state, and the fake rest.
pub struct World {
    pub game: Game,
    pub units: Units,
    pub stats: StatLists,
    pub data: UnitData,
    pub hooks: Hooks,
    pub fields: GameFields,
    pub tables: ItemTables,
    pub items: ItemStore,
    pub quests: QuestControl,
    pub vendor_tables: VendorTables,
    pub ctl: NpcControl,
    pub state: InteractionState,
    pub rest: Rest,
    pub now: u32,
}

impl World {
    /// Game creation: the NPC control (third seed) and the
    /// quests from the fixed game seed, the vendor records from the
    /// global lists.
    pub fn new(expansion: bool) -> Self {
        let mut fields = GameFields::new(Seed::init_low(GAME_SEED), expansion);
        let monsters: Vec<MonsterInfo> = (0..N_MONSTATS)
            .map(|_| MonsterInfo {
                enabled: true,
                aidel: [15, 15, 15],
                moves: 0,
                mode_chart: false,
            })
            .collect();
        let ctl = NpcControl::new(
            &monstats(),
            hirelings(if expansion { 100 } else { 0 }),
            expansion,
            0,
            &mut fields.seed,
        )
        .expect("npc control");
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut fields.seed).unwrap();
        let vendor_tables = vendor_tables();
        let mut state = InteractionState::new(&ctl, &GlobalLists::build(&vendor_tables));
        state.hireling_tables = Some(hireling_tables(&ctl.hirelings));
        Self {
            game: Game::new(),
            units: Units::default(),
            stats: StatLists::new(stat_data()),
            data: UnitData {
                monsters,
                expansion,
                ..UnitData::default()
            },
            hooks: Hooks,
            fields,
            tables: item_tables(),
            items: ItemStore::new(),
            quests,
            vendor_tables,
            ctl,
            state,
            rest: Rest::new(),
            now: 1000,
        }
    }

    /// Allocates a unit on the game seed; players get
    /// mode 1 (neutral) and quest records.
    pub fn spawn(&mut self, t: UnitType, class: u32) -> UnitId {
        let req = AllocRequest {
            ty: t,
            class,
            room: None,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: t == UnitType::Player,
        };
        let mut sim = Sim {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
        };
        let u = allocate(&mut sim, &mut self.hooks, &mut self.fields.seed, &req)
            .expect("allocate")
            .expect("allocated");
        if t == UnitType::Player {
            self.units.get_mut(u).unwrap().mode = 1;
            self.rest.quests.insert(u, PlayerQuests::default());
        }
        u
    }

    /// An NPC of `class` with its interaction list.
    pub fn npc(&mut self, class: u16) -> UnitId {
        let n = self.spawn(UnitType::Monster, u32::from(class));
        self.state.add_npc(n);
        n
    }

    pub fn guid(&self, u: UnitId) -> u32 {
        self.units.get(u).unwrap().guid
    }

    pub fn set(&mut self, u: UnitId, values: &[(u16, i32)]) {
        for &(s, v) in values {
            self.stats.unit_set(&mut self.hooks, u, s, v, 0);
        }
    }

    pub fn stat(&self, u: UnitId, s: u16) -> i32 {
        self.stats.unit_total(u, s, 0)
    }

    /// Runs `f` on the desk over this world and the NPC control.
    pub fn desk<T>(
        &mut self,
        f: impl FnOnce(&mut Desk<'_, '_, Hooks, Rest>, &mut NpcControl) -> T,
    ) -> T {
        let mut econ = Economy {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
            hooks: &mut self.hooks,
            fields: &mut self.fields,
            tables: &self.tables,
            items: &mut self.items,
        };
        let mut desk = Desk {
            econ: &mut econ,
            quests: &mut self.quests,
            vendor_tables: &self.vendor_tables,
            state: &mut self.state,
            rest: &mut self.rest,
            now: self.now,
            inv: None,
        };
        f(&mut desk, &mut self.ctl)
    }

    /// No adapter error so far.
    pub fn assert_clean(&self) {
        assert!(self.state.errors.is_empty(), "{:?}", self.state.errors);
    }
}

/// A C→S message: id then u32 fields.
pub fn msg(id: u8, fields: &[u32]) -> Vec<u8> {
    let mut m = vec![id];
    for f in fields {
        m.extend_from_slice(&f.to_le_bytes());
    }
    m
}

/// The S→C 0x2A messages sent (kind, code, GUID, gold) through the NPC
/// side.
pub fn npc_transactions(r: &Rest) -> Vec<(u8, u8, u32, u32)> {
    r.sent
        .iter()
        .filter(|(_, m)| m.first() == Some(&0x2A))
        .map(|(_, m)| {
            let u = |o: usize| u32::from_le_bytes(m[o..o + 4].try_into().unwrap());
            (m[1], m[2], u(7), u(11))
        })
        .collect()
}

/// An inventory entry in the backpack (unidentified).
pub fn backpack(item: UnitId) -> InvEntry {
    InvEntry {
        item,
        place: Place::Grid(0),
        flags: 0,
    }
}
