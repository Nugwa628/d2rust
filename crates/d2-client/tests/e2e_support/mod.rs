//! Fixtures shared by the end-to-end tests that run the server's
//! `WiredWorld` (`e2e_vendor.rs`, `e2e_single_player.rs`,
//! `prop_worldsim.rs`): the rest of
//! the NPC / vendor / quest wiring no written spec provides (staged
//! answers and a call log, never behaviour), and the synthetic item,
//! vendor and NPC tables, and the item-move seams no d2-sim module
//! provides ([`InvFx`]); and, in `world.rs` (declared by path beside
//! this module, it needs `d2-sim`'s `bench-fixtures`), the wired
//! single-player world's seams, sources and tables. `d2-server`'s
//! `prop_handle.rs`
//! includes this module by path (one copy for both crates' tests). Each
//! test crate uses a part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;

use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{Itemratio, Itemtypes, Monstats, Record};
use d2_server::adapters::handlers::items::moves::{InvParts, MoveRest};
use d2_server::adapters::handlers::world::{ActionEvents, Outbox, WiredWorld};
use d2_sim::game::Game;
use d2_sim::items::bitstream::Isc;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{InvTables, UnitKind as InvKind};
use d2_sim::items::moves::{Guid, MovePending, Owner, Spot};
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{ty, ItemTables};
use d2_sim::units::UnitId;
use d2_sim::wiring::economy::QuestRest;
use d2_sim::wiring::interaction::{HirelingRest, NpcRest, PlayerQuestsRef, VendorRest};
use d2_sim::wiring::inventory::InvRest;
use d2_sim::world::npc::{self, class, ImbueMods, InvEntry, ItemFacts};
use d2_sim::world::quests::{PlayerQuests, QuestChain, TextList, UnitKind};
use d2_sim::world::vendors::price::Bonus;
use d2_sim::world::vendors::{
    NpcPrices, Transaction, TypeRec, VendorItem, VendorTables, NO_CODE, XXX,
};

/// Item records (combined index; the vendor tables use the same order).
pub const CAP: usize = 0;
pub const BUC: usize = 1;
/// Rows of the synthetic tables.
pub const N_STATS: usize = 359;
pub const N_TYPES: usize = 80;
pub const N_MONSTATS: usize = 400;

pub fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// The vendors' player-inventory calls the server's `WiredWorld` answers
/// from its inventory model (`handlers::items::InvVendors`) before they
/// reach a rest.
const MODEL: &str = "WiredWorld answers from the inventory model";

/// The interaction seams no written spec provides:
/// staged answers (positions as a fixed
/// distance, the player's interact unit, the player's quest records,
/// the NPC grid always having room, the carried-gold caps) and a log of
/// every call that would change state outside `d2-sim`. The item copy
/// answers null: no rule writes it yet. The player's inventory
/// is the host's inventory model, not this rest's.
#[derive(Debug, Default)]
pub struct Rest {
    pub quests: BTreeMap<UnitId, PlayerQuests>,
    pub last_bought: BTreeMap<UnitId, u32>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
    pub log: Vec<String>,
    /// The prices the host publishes for the shop panel.
    pub prices: BTreeMap<u32, u32>,
}

impl Outbox for Rest {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

impl PlayerQuestsRef for Rest {
    fn quests_ref(&self, player: UnitId) -> Option<&PlayerQuests> {
        self.quests.get(&player)
    }
}

impl NpcRest for Rest {
    /// Quality override; ≥ 1 keeps the request's quality.
    fn item_format(&self) -> u16 {
        1
    }
    /// Unit distance: within talk range (≤ 6).
    fn distance(&self, _: UnitId, _: UnitId) -> i32 {
        3
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
    fn approach(&mut self, _: UnitId, _: UnitId) {
        self.log.push("approach".into());
    }
    fn player_busy(&self, _: UnitId) -> u32 {
        0
    }
    fn start_allowed(&self, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn tristram_cain_busy(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn pet(&self, _: UnitId, _: u8, _: u8) -> Option<UnitId> {
        None
    }
    fn pets(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
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
    /// The 0x27 `partial` message: 34 zero bytes.
    fn encode_text_list(&self, _: &TextList) -> [u8; 34] {
        [0; 34]
    }
    fn socket_granted(&mut self, _: UnitId) {}
    fn personalize_granted(&mut self, _: UnitId) {}
    fn inventory_entries(&self, _: UnitId) -> Vec<InvEntry> {
        Vec::new()
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
    fn spawn_mercenary(&mut self, _: UnitId, _: u32, _: u8) -> Option<UnitId> {
        None
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
    fn drop_gold(&mut self, _: UnitId, amount: i32) {
        self.log.push(format!("drop gold {amount}"));
    }
    fn last_bought(&self, p: UnitId) -> u32 {
        self.last_bought.get(&p).copied().unwrap_or(u32::MAX)
    }
    fn set_last_bought(&mut self, p: UnitId, guid: u32) {
        self.last_bought.insert(p, guid);
    }
    fn has_cursor_item(&self, _: UnitId) -> bool {
        unreachable!("{MODEL}")
    }
    /// No item rule writes the copy yet. Null: the vendor code's own
    /// refusal runs.
    fn copy_item(&mut self, item: UnitId) -> Option<UnitId> {
        self.log.push(format!("copy {}", item.0));
        None
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
    /// S→C 0x2A through its builder.
    fn send_transaction(&mut self, p: UnitId, t: Transaction) {
        let m = npc::transaction(t.kind, t.code, t.guid, t.gold as u32);
        self.sent.push((p, m.to_vec()));
    }
    fn new_store_inventory(&mut self, class: u16, _: Option<UnitId>) {
        self.log.push(format!("new store {class}"));
    }
    /// The NPC grid: always room.
    fn place_in_store(&mut self, class: u16, item: UnitId) -> bool {
        self.log.push(format!("store {class} {}", item.0));
        true
    }
    fn remove_store_item(&mut self, _: u16, item: UnitId) {
        self.log.push(format!("unstore {}", item.0));
    }
    fn take_from_store(&mut self, _: u16, item: UnitId) {
        self.log.push(format!("take {}", item.0));
    }
    fn store_price(&mut self, _: UnitId, guid: u32, price: u32) {
        self.prices.insert(guid, price);
    }
    fn place_in_gamble(&mut self, _: u16, _: u32, _: UnitId) -> bool {
        true
    }
    fn remove_gamble_item(&mut self, _: u16, _: u32, _: UnitId) {}
    fn refresh_npc_inventory(&mut self, npc: UnitId) {
        self.log.push(format!("refresh {}", npc.0));
    }
    fn add_trade_inventory(&mut self, class: u16, item: UnitId) {
        self.log.push(format!("trade inv {class} {}", item.0));
    }
    fn owns_item(&self, _: UnitId, _: UnitId) -> bool {
        unreachable!("{MODEL}")
    }
    fn in_inventory(&self, _: UnitId, _: UnitId) -> bool {
        unreachable!("{MODEL}")
    }
    fn equipped_items(&self, _: UnitId) -> Vec<UnitId> {
        unreachable!("{MODEL}")
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
    fn place_in_backpack(&mut self, _: UnitId, _: UnitId) -> bool {
        unreachable!("{MODEL}")
    }
    fn take_from_cursor(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn lower_book_skill(&mut self, _: UnitId, _: UnitId, _: i32) {}
    fn remove_stored(&mut self, _: UnitId, _: UnitId) {
        unreachable!("{MODEL}")
    }
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
        self.sent.push((player, msg.to_vec()));
    }
    fn send_text_list(&mut self, _: UnitId, _: UnitId, _: &[(u16, u32)]) {}
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
    fn unit_position(&self, _: UnitId) -> Option<(i32, i32, d2_sim::units::RoomId)> {
        None
    }
    fn room_contains(&self, _: d2_sim::units::RoomId, _: i32, _: i32) -> bool {
        false
    }
    fn room_at(&self, _: d2_sim::units::RoomId, _: i32, _: i32) -> Option<d2_sim::units::RoomId> {
        None
    }
    fn free_spot_at(
        &mut self,
        _: d2_sim::units::RoomId,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
    ) -> Option<(i32, i32, d2_sim::units::RoomId)> {
        None
    }
    fn spawn_monster(
        &mut self,
        _: d2_sim::units::RoomId,
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
    fn create_object(
        &mut self,
        _: d2_sim::units::RoomId,
        _: i32,
        _: i32,
        _: u16,
    ) -> Option<UnitId> {
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

/// Every type is its own and type 0's; helm and shield are armor.
pub fn equiv() -> EquivMatrix {
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
    set(usize::from(ty::SHIE), usize::from(ty::ARMO));
    m
}

/// Items: a cap (helm, durability 12, defense 3–5) and a buckler
/// (shield, durability 12, defense 4–6).
pub fn item_tables() -> ItemTables {
    let mut ratio: Itemratio = blank();
    ratio.version = 0;
    let rec = |t: u16, code: &[u8; 4], (minac, maxac): (u32, u32)| ItemRec {
        code: *code,
        type_: t as i16,
        level: 1,
        durability: 12,
        minac,
        maxac,
        ..ItemRec::default()
    };
    ItemTables {
        items: vec![
            rec(ty::HELM, b"cap ", (3, 5)),
            rec(ty::SHIE, b"buc ", (4, 6)),
        ],
        itemtypes: (0..N_TYPES)
            .map(|_| {
                let mut t: Itemtypes = blank();
                t.class = 0xFF;
                t.staffmods = 0xFF;
                // Empty `shoots`: the link miss (link16 −1), as the
                // other inventory fixtures.
                t.shoots = 0xFFFF;
                t.rare = 1;
                t
            })
            .collect(),
        equiv: equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        isc: save_columns(),
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// The itemstatcost save columns (`Save Bits`, `Save Add`) of the stats
/// the items' save records carry (defense, durability and its maximum),
/// so the item copy (a save-format round trip) keeps them. Every other
/// stat: none.
fn save_columns() -> Vec<Isc> {
    let mut t = vec![Isc::default(); N_STATS];
    let bits = |save_bits: u8, save_add: u32| Isc {
        save_bits,
        save_add,
        ..Isc::default()
    };
    t[usize::from(d2_sim::items::stat::ARMORCLASS)] = bits(11, 10);
    t[usize::from(d2_sim::items::stat::DURABILITY)] = bits(9, 0);
    t[usize::from(d2_sim::items::stat::MAXDURABILITY)] = bits(8, 0);
    t
}

/// The vendor tables over the same items: Akara's column (0) holds the
/// cap as a permanent item and the buckler as a list entry (Min 1, Max
/// 3: one range draw on the NPC-control seed); one `npc.txt` row (sell
/// 1024, buy 512, repair 128, max buy 5000: Akara).
pub fn vendor_tables() -> VendorTables {
    let item = |code: &[u8; 4], t: u16, cost: u32, (minac, maxac): (u32, u32)| VendorItem {
        code: *code,
        normcode: *code,
        ubercode: NO_CODE,
        ultracode: NO_CODE,
        cost,
        type_: t as i16,
        type2: -1,
        level: 1,
        spawnable: 1,
        durability: 12,
        minac,
        maxac,
        nightmare_upgrade: XXX,
        hell_upgrade: XXX,
        ..VendorItem::default()
    };
    let mut cap = item(b"cap ", ty::HELM, 100, (3, 5));
    cap.perm_store = 1;
    cap.columns[0] = [1, 1, 0, 0, 0];
    let mut buc = item(b"buc ", ty::SHIE, 80, (4, 6));
    buc.columns[0] = [1, 3, 0, 0, 0];
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
    itemtypes[usize::from(ty::SHIE)].storepage = 0;
    VendorTables {
        items: vec![cap, buc],
        itemtypes,
        equiv: equiv(),
        stat_shift: 6,
        stat_mask: 0x3F,
        monster_levels: vec![[1, 1, 1]; N_MONSTATS],
        interact: vec![class::AKARA, class::GHEED, class::CHARSI],
        npc: [class::AKARA, class::GHEED, class::CHARSI]
            .into_iter()
            .map(|c| NpcPrices {
                class: u32::from(c),
                sell: 1024,
                buy: 512,
                rep: 128,
                quests: [(0, 0, 0, 0); 3],
                max_buy: [5000; 3],
            })
            .collect(),
        // Gheed's gamble list draws from both items.
        gamble_index: Some(vec![0, 1]),
        gamble_thresholds: vec![2; 100],
        difficulty: vec![Default::default(); 3],
        ..VendorTables::default()
    }
}

/// Monstats: Akara is `npc` and `interact`.
pub fn monstats() -> Vec<Monstats> {
    let mut v: Vec<Monstats> = (0..N_MONSTATS).map(|_| blank()).collect();
    v[usize::from(class::AKARA)].npc = true;
    v[usize::from(class::AKARA)].interact = true;
    v[usize::from(class::GHEED)].npc = true;
    v[usize::from(class::GHEED)].interact = true;
    v[usize::from(class::CHARSI)].npc = true;
    v[usize::from(class::CHARSI)].interact = true;
    v
}

/// The messages of a stored item's sale: S→C 0x9D action 5 for `guid`,
/// then the 0x2A `last`; with the trade open, the sold copy placed in
/// the store follows as one 0x9C action 11 (at frame 24 of a sale).
pub fn assert_stored_sale(got: &[Vec<u8>], guid: u32, last: Vec<u8>) {
    assert!(matches!(got.len(), 2 | 3), "{got:02X?}");
    assert_eq!((got[0][0], got[0][1]), (0x9D, 0x05));
    assert_eq!(got[0][4..8], guid.to_le_bytes());
    assert_eq!(got[1], last);
    if let Some(shown) = got.get(2) {
        assert_eq!((shown[0], shown[1]), (0x9C, 0x0B), "{got:02X?}");
    }
}

/// S→C 0x2A, bytes 3–6 zero.
pub fn tx(kind: u8, code: u8, guid: u32, gold: i32) -> Vec<u8> {
    let mut m = vec![0x2A, kind, code, 0, 0, 0, 0];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&gold.to_le_bytes());
    m
}

/// The item-move seams no d2-sim module provides
/// (`d2_sim::wiring::inventory::InvRest` with its `MovePending` part,
/// staged answers, never behaviour. The player-to-item distance is a
/// fixed staged value; there is no room at the drop's offset start, and
/// the free-spot search answers the start
/// spot as is in the player's room (as the drop fixture's search does);
/// non-item positions are staged; requirements, hands and auto-equip
/// answer the narrowest readings (no level requirement, nothing
/// two-handed, every location allowed). Every state-changing call is
/// logged; sends are collected for the host.
#[derive(Clone, Default)]
pub struct InvFx(pub std::sync::Arc<std::sync::Mutex<InvFxState>>);

#[derive(Default)]
pub struct InvFxState {
    pub mouse_skill: Option<(i32, i32)>,
    pub distance: i32,
    pub room: Option<d2_sim::units::RoomId>,
    pub pos: BTreeMap<Owner, (i32, i32)>,
    pub log: Vec<String>,
    pub sent: Vec<(Owner, Vec<u8>)>,
}

impl InvFx {
    pub fn with<T>(&self, f: impl FnOnce(&mut InvFxState) -> T) -> T {
        f(&mut self.0.lock().unwrap())
    }
    fn log(&self, s: String) {
        self.with(|r| r.log.push(s));
    }
}

impl MovePending for InvFx {
    fn distance(&self, _: Owner, _: Owner) -> i32 {
        self.with(|r| r.distance)
    }
    fn free_spot(
        &self,
        start: (i32, i32),
        _: (i32, i32),
        _: u32,
        _: u32,
        _: u32,
        _: u32,
    ) -> Option<Spot> {
        self.with(|r| {
            r.room.map(|room| Spot {
                room,
                x: start.0,
                y: start.1,
            })
        })
    }
    fn sound(&mut self, u: Owner, id: u32) {
        self.log(format!("sound {} {id:#x}", u.guid));
    }
    fn pickup_sound(&mut self, player: Owner, item: Guid) {
        self.log(format!("pickup_sound {} {item}", player.guid));
    }
    fn quest_item_picked(&mut self, _: Owner, item: Guid) {
        self.log(format!("quest_item_picked {item}"));
    }
    fn quest_item_dropped(&mut self, item: Guid) {
        self.log(format!("quest_item_dropped {item}"));
    }
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.with(|r| r.sent.push((player, bytes)));
    }
}

impl InvRest for InvFx {
    fn mouse_skill(&self, _: Owner, _: bool) -> Option<(i32, i32)> {
        self.with(|r| r.mouse_skill)
    }
    fn pos(&self, u: Owner) -> (i32, i32) {
        self.with(|r| r.pos.get(&u).copied().unwrap_or((0, 0)))
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        self.with(|r| r.pos.insert(u, (x, y)));
    }
    fn item_active_on(&self, _: Guid, _: Owner) -> bool {
        false
    }
    fn own_contribution(&self, _: Guid, _: Owner, _: u16) -> i32 {
        0
    }
    fn level_requirement(&self, _: Guid, _: Owner) -> i32 {
        -1
    }
    fn two_handed(&self, _: Guid) -> bool {
        false
    }
    fn one_or_two_handed(&self, _: Owner, _: Guid) -> bool {
        false
    }
    fn ammo_type(&self, _: Guid) -> Option<i16> {
        None
    }
    fn has_allowed_location(&self, _: Guid) -> bool {
        true
    }
    fn quiver_kind(&self, _: Guid) -> bool {
        false
    }
    fn player_data_4c(&self, _: Owner) -> u32 {
        0
    }
    fn player_data_50(&self, _: Owner) -> u32 {
        0
    }
    fn npc_talking(&self, _: Owner, _: Owner) -> bool {
        false
    }
    fn player_trade_gate(&self, _: Owner) -> Option<bool> {
        None
    }
}

impl MoveRest for InvFx {
    fn take_sent(&mut self) -> Vec<(Owner, Vec<u8>)> {
        self.with(|r| std::mem::take(&mut r.sent))
    }
}

/// Inventory tables (grid records: player classes
/// 10 × 4, the cube 3 × 4, ...) over the item tables' records with the
/// given (invwidth, invheight); helms on the head, shields in either
/// hand.
pub fn inv_tables(t: &ItemTables, sizes: &[(u8, u8)]) -> InvTables {
    let g = |x, y| GridRec {
        grid_x: x,
        grid_y: y,
    };
    let mut grids = vec![g(10, 4); 16];
    grids[5] = g(10, 10);
    grids[8] = g(6, 4);
    grids[9] = g(3, 4);
    grids[12] = g(6, 8);
    grids[13] = g(0, 0);
    let mut itemtypes = vec![
        InvTypeRec {
            class: 7,
            ..InvTypeRec::default()
        };
        t.itemtypes.len()
    ];
    for (ty, loc1, loc2) in [(ty::HELM, 1, 1), (ty::SHIE, 5, 4)] {
        let r = &mut itemtypes[usize::from(ty)];
        r.body = 1;
        r.bodyloc1 = loc1;
        r.bodyloc2 = loc2;
    }
    InvTables {
        grids,
        belts: vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16],
        items: t
            .items
            .iter()
            .zip(sizes)
            .map(|(r, &(w, h))| InvItemRec {
                code: r.code,
                type_: r.type_,
                invwidth: w,
                invheight: h,
                ..InvItemRec::default()
            })
            .collect(),
        itemtypes,
        equiv: t.equiv.clone(),
        books: Vec::new(),
        item_use: Default::default(),
    }
}

/// The server host's inventory model over `tables` with the item-move
/// seams of `inv`, and the player's inventory (made at player creation,
/// done here).
pub fn inv_parts(tables: InvTables, inv: InvFx, player: UnitId, class: u8, guid: u32) -> InvParts {
    let mut parts = InvParts::new(tables, Box::new(inv));
    parts
        .state
        .add_inventory(player, InvKind::Player { class }, guid);
    parts
}

/// Puts `item` on page `page` of the player's inventory through
/// the cursor drop (a free position, no "send"): a
/// fixture's stored item, as a loaded character's would be.
pub fn store<D: ActionEvents, R, S>(
    world: &mut WiredWorld<R, S>,
    game: &mut Game,
    events: &mut D,
    (player, item): (UnitId, UnitId),
    page: u8,
) {
    let sys = &mut events.action().sys;
    sys.units.get_mut(item).expect("item unit").mode = 4;
    sys.hooks.items.get_mut(item).expect("item").inv_page = page;
    let placed = world.with_economy(game, events, |econ, p| {
        let inv = p.inventory.as_deref_mut().expect("inventory parts");
        inv.desk(econ).place(player, item, (0, 0), true, false)
    });
    assert!(placed, "stored");
}
