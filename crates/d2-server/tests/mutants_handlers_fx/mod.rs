//! Shared fixture of the `mutants_handlers_*` tests: the wired host
//! (`SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>`) on synthetic
//! tables, as the server's own cube and trade tests build it (the
//! fixture shape of `prop_handle.rs`). [`Rest`] answers the interaction
//! seams the host leaves open; [`ItemLog`] is the cube's `ItemPending`
//! stand-in: it logs.
//!
//! Used by `mutants_handlers_items`, `_world`, `_skills` and `mutants_walk`, which
//! pull it in as a module; it carries no tests of its own.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use d2_data::bin::BinTable;
use d2_data::fixup::maps::{states as state_maps, EquivMatrix};
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Monstats, Record, States};
use d2_server::adapters::handlers::items::moves::{InvParts, MoveRest};
use d2_server::adapters::handlers::items::{CubeParts, ItemPending};
use d2_server::adapters::handlers::world::{ActionEvents, ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, SimGame, UnitFacts};
use d2_server::buffers::ClientBuffers;
use d2_server::seams::{Intents, PlayerGate, Pos, ResultCode};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::data::DrlgData;
use d2_sim::drlg::{Dungeon, NoLevelTypes, TileInfo, TileSource};
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{InvTables, UnitKind as InvKind};
use d2_sim::items::moves::{Guid, MovePending, Owner};
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{q, ty, ItemRequest, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{ClassStats, StatData, StatTable, StateTable};
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::economy::{GameFields, ItemSpawn, QuestRest};
use d2_sim::wiring::interaction::{HirelingRest, NpcRest, PlayerQuestsRef, VendorRest};
use d2_sim::wiring::inventory::InvRest;
use d2_sim::world::cube::{
    input_flags, kind as cube_kind, CraftMod, CubeData, InputSlot, ItemRecord, OutputSlot, Recipe,
    CUBE_PAGE,
};
use d2_sim::world::npc::{self, class, ImbueMods, InvEntry, ItemFacts, NpcControl};
use d2_sim::world::quests::{
    PlayerQuests, QuestChain, QuestControl, QuestTables, TextList, UnitKind,
};
use d2_sim::world::vendors::price::Bonus;
use d2_sim::world::vendors::{
    NpcPrices, Transaction, TypeRec, VendorItem, VendorTables, NO_CODE, XXX,
};

pub const N_STATS: usize = 359;
const N_STATES: usize = 64;
pub const N_TYPES: usize = 80;
pub const N_MONSTATS: usize = 400;

pub fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost (no ops, no shifts) through the d2-data fix-up.
pub fn stat_data() -> Arc<StatData> {
    let size = Itemstatcost::SIZE;
    let mut records = vec![0u8; N_STATS * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
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
    let states = BinTable {
        name: "states".into(),
        source: "synthetic".into(),
        count: N_STATES,
        record_size: States::SIZE,
        records: vec![0u8; N_STATES * States::SIZE],
    };
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        states: StateTable::new(&states, &state_maps(&states)).expect("states"),
        damage_regen: vec![0; 8],
        aurastate: vec![0; 8],
        rescale_precision: d2_sim::stats::DEFAULT_RESCALE_PRECISION,
    })
}

pub struct NoTiles;
impl TileSource for NoTiles {
    fn dt1(&self, _: &[u8]) -> Option<&[TileInfo]> {
        None
    }
}

fn no_drlg() -> DrlgWorld {
    DrlgWorld {
        dungeon: Dungeon::default(),
        data: Arc::new(DrlgData::default()),
        tiles: Box::new(NoTiles),
        types: Box::new(NoLevelTypes),
    }
}

/// Action tables with no skills (the economy hosts).
pub fn empty_action_tables() -> ActionTables {
    ActionTables {
        missiles: Vec::new(),
        skills: SkillTables {
            skills: Vec::new(),
            skilldesc: Vec::new(),
            missiles: Vec::new(),
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 0,
            stat_count: 0,
        },
        combat: CombatTables {
            charstats: Vec::new(),
            difficultylevels: Vec::new(),
            monstats: Vec::new(),
            monstats2: Vec::new(),
            hitclass: Vec::new(),
        },
        levels: Vec::new(),
        skill_modes: Vec::new(),
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    }
}

pub const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

// ---- the rests ---------------------------------------------------------------------------

/// The action wiring's seams: `Pending`'s defaults; sends kept; warp,
/// arrival mode and sounds logged; the hostile delay staged; positions
/// kept from allocation.
#[derive(Default)]
pub struct ActionRest {
    pub sent: Vec<(UnitId, Vec<u8>)>,
    pub log: Vec<String>,
    pub hostile: bool,
    pub pos: BTreeMap<UnitId, (i32, i32)>,
}

impl Pending for ActionRest {
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or_default()
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn warp(&mut self, _: &mut Game, player: UnitId, level: u32, tile_code: u8) {
        self.log
            .push(format!("warp {} {level} {tile_code}", player.0));
    }
    fn set_player_mode_arrival(&mut self, _: &mut Game, player: UnitId) {
        self.log.push(format!("arrival mode {}", player.0));
    }
    fn hostile_delay(&self, _: UnitId) -> bool {
        self.hostile
    }
    fn attach_sound(&mut self, unit: UnitId, event: u8) {
        self.log.push(format!("sound {} {event}", unit.0));
    }
}

impl Outbox for ActionRest {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// The interaction seams the host leaves open (talk range, the
/// staged inventory, room in the NPC grid, no item copy).
#[derive(Default)]
pub struct Rest {
    pub quests: BTreeMap<UnitId, PlayerQuests>,
    pub inventory: BTreeSet<UnitId>,
    pub last_bought: BTreeMap<UnitId, u32>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
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
    fn item_format(&self) -> u16 {
        1
    }
    fn distance(&self, _: UnitId, _: UnitId) -> i32 {
        3
    }
    fn axis_check(&self, _: UnitId, _: UnitId) -> u32 {
        0
    }
    fn unit_check(&self, _: UnitId, _: u32) -> u32 {
        0
    }
    fn clear_path(&mut self, _: UnitId) {}
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
    fn npc_ai_param(&mut self, _: UnitId, _: u32) {}
    fn respec_sound(&mut self, _: UnitId) {}
    fn encode_text_list(&self, _: &TextList) -> [u8; 34] {
        [0; 34]
    }
    fn socket_granted(&mut self, _: UnitId) {}
    fn personalize_granted(&mut self, _: UnitId) {}
    fn inventory_entries(&self, _: UnitId) -> Vec<InvEntry> {
        Vec::new()
    }
    fn identify(&mut self, _: UnitId) {}
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
    fn set_mode(&mut self, _: UnitId, _: u8) {}
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
    fn copy_item(&mut self, _: UnitId) -> Option<UnitId> {
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
    fn send_transaction(&mut self, p: UnitId, t: Transaction) {
        let m = npc::transaction(t.kind, t.code, t.guid, t.gold as u32);
        self.sent.push((p, m.to_vec()));
    }
    fn new_store_inventory(&mut self, _: u16, _: Option<UnitId>) {}
    fn place_in_store(&mut self, _: u16, _: UnitId) -> bool {
        true
    }
    fn remove_store_item(&mut self, _: u16, _: UnitId) {}
    fn take_from_store(&mut self, _: u16, _: UnitId) {}
    fn place_in_gamble(&mut self, _: u16, _: u32, _: UnitId) -> bool {
        true
    }
    fn remove_gamble_item(&mut self, _: u16, _: u32, _: UnitId) {}
    fn refresh_npc_inventory(&mut self, _: UnitId) {}
    fn add_trade_inventory(&mut self, _: u16, _: UnitId) {}
    fn owns_item(&self, _: UnitId, item: UnitId) -> bool {
        self.inventory.contains(&item)
    }
    fn in_inventory(&self, _: UnitId, item: UnitId) -> bool {
        self.inventory.contains(&item)
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
    fn place_in_backpack(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn take_from_cursor(&mut self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn lower_book_skill(&mut self, _: UnitId, _: UnitId, _: i32) {}
    fn remove_stored(&mut self, _: UnitId, item: UnitId) {
        self.inventory.remove(&item);
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
    fn unhandled(&mut self, _: u8, _: u32) {}
}

pub type WiredGame = SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>;

/// `SimGame::handle` for `client` (the dispatcher's gate and parse not
/// run): the result and what the client's buffer holds.
pub fn handle(sim: &mut WiredGame, client: u32, msg: &[u8]) -> (ResultCode, Vec<Vec<u8>>) {
    let mut out = ClientBuffers::new();
    for c in 0..4 {
        out.add_client(c);
    }
    let r = Intents::handle(sim, client, msg, msg.len(), &mut out);
    let mut got = Vec::new();
    while let Some(m) = out.pop(client) {
        got.push(m);
    }
    (r, got)
}

// ---- the cube host -----------------------------------------------------------------------

pub const T_RING: u16 = 10;
pub const T_BOX: u16 = 11;
pub const T_AMULET: u16 = 12;
/// Item records (combined index).
pub const CUBE: usize = 0;
pub const RING: usize = 1;
pub const AMULET: usize = 2;
/// An ear (type `play`; it carries the player's name).
pub const EAR: usize = 3;
/// A quest item with the `hst ` hook.
pub const HST: usize = 4;
pub const GAME_SEED: u32 = 0x5EED;

fn cube_equiv() -> EquivMatrix {
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
    for c in [T_RING, T_BOX, T_AMULET] {
        set(usize::from(c), usize::from(ty::MISC));
    }
    m
}

fn item_rec(t: u16, code: &[u8; 4]) -> ItemRec {
    ItemRec {
        code: *code,
        type_: t as i16,
        level: 1,
        ..ItemRec::default()
    }
}

/// Items: the cube (`box `), a ring, an amulet, an ear, a quest item.
pub fn cube_item_tables() -> ItemTables {
    let mut ratio: Itemratio = blank();
    ratio.version = 1;
    let itemtypes = (0..N_TYPES)
        .map(|_| {
            let mut t: Itemtypes = blank();
            t.class = 0xFF;
            t.staffmods = 0xFF;
            t.rare = 1;
            t
        })
        .collect();
    ItemTables {
        items: vec![
            item_rec(T_BOX, b"box "),
            item_rec(T_RING, b"rin "),
            item_rec(T_AMULET, b"amu "),
            item_rec(ty::PLAY, b"ear "),
            ItemRec {
                quest: 1,
                ..item_rec(60, b"hst ")
            },
        ],
        itemtypes,
        equiv: cube_equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// The inventory records of [`cube_item_tables`]' items (code, type,
/// invwidth, invheight).
pub fn cube_inv_items() -> Vec<([u8; 4], u16, u8, u8)> {
    vec![
        (*b"box ", T_BOX, 2, 2),
        (*b"rin ", T_RING, 1, 1),
        (*b"amu ", T_AMULET, 1, 1),
        (*b"ear ", ty::PLAY, 1, 1),
        (*b"hst ", 60, 1, 1),
    ]
}

/// A cube recipe shape without mods: one ring → a normal amulet.
pub fn ring_to_amulet() -> Recipe {
    let mut inputs = [InputSlot::default(); 7];
    inputs[0] = InputSlot {
        flags: input_flags::USEANY,
        item: RING as u16,
        ..InputSlot::default()
    };
    let out = OutputSlot {
        kind: cube_kind::ITEMCODE,
        item: AMULET as u16,
        quality: q::NORMAL,
        mods: [CraftMod {
            property: -1,
            ..CraftMod::default()
        }; 5],
        ..OutputSlot::default()
    };
    Recipe {
        enabled: 1,
        class: 0xFF,
        numinputs: 1,
        inputs,
        outputs: [out, OutputSlot::default(), OutputSlot::default()],
        ..Recipe::default()
    }
}

pub fn cube_data(t: &ItemTables, recipes: Vec<Recipe>) -> CubeData {
    CubeData {
        recipes,
        items: t
            .items
            .iter()
            .map(|r| ItemRecord {
                code: r.code,
                level: r.level,
                quest: r.quest,
                spawnable: 1,
                ..ItemRecord::default()
            })
            .collect(),
        valshift: vec![0; N_STATS],
        max_level: 99,
    }
}

/// What the [`ItemLog`] answers, staged by a test.
#[derive(Default)]
pub struct Script {
    /// `duplicate` hands these out in order (then `None`).
    pub copies: VecDeque<UnitId>,
    /// `tempered_affix` (prefix, suffix).
    pub tempered: (u16, u16),
    pub cow: bool,
    pub log: Vec<String>,
}

/// [`ItemPending`] stand-in: answers from the [`Script`]; every call is
/// logged. Placement, removal and the socketed items are the inventory
/// model's (`InvParts`).
#[derive(Clone, Default)]
pub struct ItemLog(pub Arc<Mutex<Script>>);

impl ItemLog {
    fn log(&self, s: String) {
        self.0.lock().unwrap().log.push(s);
    }
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap().log)
    }
    pub fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.0.lock().unwrap()
    }
}

impl ItemPending for ItemLog {
    fn inventory_pass(&mut self, _: UnitId, _: &mut Vec<Vec<u8>>) {
        self.log("pass".into());
    }
    fn duplicate(&mut self, item: UnitId, fillers: bool) -> Option<UnitId> {
        self.log(format!("duplicate {} {fillers}", item.0));
        self.script().copies.pop_front()
    }
    fn tempered_affix(&mut self, _: UnitId, prefix: bool) -> u16 {
        let t = self.script().tempered;
        if prefix {
            t.0
        } else {
            t.1
        }
    }
    fn drop_runeword_stats(&mut self, item: UnitId) {
        self.log(format!("runeword {}", item.0));
    }
    fn repair(&mut self, item: UnitId) {
        self.log(format!("repair {}", item.0));
    }
    fn recharge(&mut self, item: UnitId) {
        self.log(format!("recharge {}", item.0));
    }
    fn quest_item_hook(&mut self, _: UnitId, item: UnitId, code: [u8; 4]) {
        let code = String::from_utf8_lossy(&code).into_owned();
        self.log(format!("hook {} {code}", item.0));
    }
    fn cow_portal(&mut self, _: UnitId) -> bool {
        self.log("cow".into());
        self.script().cow
    }
}

/// The item-move seams no d2-sim module provides, answered narrowest
/// (`InvRest` / `MovePending` defaults: distance 0, no free spot, no
/// interaction); the sends are collected for the host.
#[derive(Default)]
pub struct InvStandIn {
    sent: Vec<(Owner, Vec<u8>)>,
}

impl MovePending for InvStandIn {
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.sent.push((player, bytes));
    }
}

impl InvRest for InvStandIn {
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

impl MoveRest for InvStandIn {
    fn take_sent(&mut self) -> Vec<(Owner, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// Inventory tables (grid records) over items of
/// (code, type, invwidth, invheight), in record order.
pub fn inv_tables(
    items: &[([u8; 4], u16, u8, u8)],
    n_types: usize,
    equiv: EquivMatrix,
) -> InvTables {
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
    InvTables {
        grids,
        belts: vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16],
        items: items
            .iter()
            .map(|&(code, t, w, h)| InvItemRec {
                code,
                type_: t as i16,
                invwidth: w,
                invheight: h,
                ..InvItemRec::default()
            })
            .collect(),
        itemtypes: vec![
            InvTypeRec {
                class: 7,
                ..InvTypeRec::default()
            };
            n_types
        ],
        equiv,
        books: Vec::new(),
        item_use: Default::default(),
    }
}

/// The inventory parts of a wired host, with the player's inventory
/// (added at player creation).
pub fn inv_parts(tables: InvTables, player: UnitId, class: u8, guid: u32) -> InvParts {
    let mut parts = InvParts::new(tables, Box::new(InvStandIn::default()));
    parts
        .state
        .add_inventory(player, InvKind::Player { class }, guid);
    parts
}

/// Places `item` (put on the cursor, page `page`) into the player's
/// inventory through the inventory model's placement (free position, no
/// "send"): a fixture's stored item.
pub fn store_item(sim: &mut WiredGame, player: UnitId, item: UnitId, page: u8) {
    sim.events.sys.units.get_mut(item).unwrap().mode = 4;
    sim.events.sys.hooks.items.get_mut(item).unwrap().inv_page = page;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    let placed = world.with_economy(game, events, |econ, p| {
        let inv = p.inventory.as_deref_mut().expect("inventory parts");
        inv.desk(econ).place(player, item, (0, 0), true, false)
    });
    assert!(placed);
}

/// The wired host with the cube's parts: game creation on the action
/// sim from `fields`; a player (class `class`) for client 0 with the
/// cube stored in its inventory and a ring in the cube (page 3), real
/// units of the action sim, items in the host's one store; the cube open
/// (interaction type 4).
pub struct CubeFx {
    pub sim: WiredGame,
    pub player: UnitId,
    pub cube: UnitId,
    pub ring: UnitId,
    pub log: ItemLog,
}

impl CubeFx {
    pub fn new(class: u32, fields: GameFields, recipes: Vec<Recipe>) -> Self {
        let tables = cube_item_tables();
        let hooks = ActionHooks::new(
            Arc::new(empty_action_tables()),
            no_drlg(),
            Seed::init(),
            ActionRest::default(),
        );
        let mut events = ActionSim::new(stat_data(), UnitData::default(), hooks);
        events.create_game(&fields);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let mut seed = events.hooks().game_seed;
        let npc = NpcControl::new(&[], Vec::new(), true, 0, &mut seed).unwrap();
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        events.hooks().game_seed = seed;
        let mut world = WiredWorld::new(
            ActionWorld::default(),
            tables,
            quests,
            npc,
            VendorTables::default(),
            Rest::default(),
            1000,
        );
        let log = ItemLog::default();
        let mut parts = CubeParts::new(cube_data(&world.tables, recipes), Box::new(log.clone()));
        parts.staged.local_date = (15, 3);
        world.cube = Some(parts);
        let req = AllocRequest {
            ty: UnitType::Player,
            class,
            room: None,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: false,
        };
        let player = events
            .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
            .unwrap();
        let pg = events.sys.units.get(player).unwrap().guid;
        world.inventory = Some(inv_parts(
            inv_tables(&cube_inv_items(), N_TYPES, cube_equiv()),
            player,
            class as u8,
            pg,
        ));
        let mut sim: WiredGame = SimGame::with_world(game, events, world);
        let cube = new_item(&mut sim, CUBE, 4);
        store_item(&mut sim, player, cube, 0);
        let ring = new_item(&mut sim, RING, 4);
        store_item(&mut sim, player, ring, CUBE_PAGE);
        sim.join(0, Some(player), None, client_state::IN_GAME)
            .unwrap();
        sim.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        sim.set_unit(
            player,
            UnitFacts {
                act: 0,
                pos: Pos { x: 100, y: 100 },
                owner: None,
            },
        );
        let cube_guid = sim.events.sys.units.get(cube).unwrap().guid;
        sim.events
            .sys
            .units
            .get_mut(player)
            .unwrap()
            .interact
            .set(4, cube_guid);
        CubeFx {
            sim,
            player,
            cube,
            ring,
            log,
        }
    }

    pub fn parts(&mut self) -> &mut CubeParts {
        self.sim.world.cube.as_mut().unwrap()
    }

    pub fn guid(&self, u: UnitId) -> u32 {
        self.sim.events.sys.units.get(u).unwrap().guid
    }

    /// C→S 0x4F button 0x18 (transmute): the result and the record that
    /// matched, read from what happened to the ring.
    pub fn transmute(&mut self) -> bool {
        let (r, _) = handle(&mut self.sim, 0, &[0x4F, 0x18, 0, 0, 0, 0, 0]);
        assert_eq!(r, ResultCode::Done);
        let ring = self.ring;
        let gone = !self.sim.events.sys.hooks.items.contains(ring);
        // The used input is removed from the inventory and freed (unit and data).
        let listed = self.items_of().contains(&ring);
        assert_eq!(
            (listed, self.sim.game.lists.unit(ring).is_some()),
            (!gone, !gone)
        );
        gone
    }

    /// The player's item list (the inventory model, link order).
    pub fn items_of(&self) -> Vec<UnitId> {
        let inv = self.sim.world.inventory.as_ref().unwrap();
        inv.state.items_of(self.player)
    }

    /// The output the last transmute placed (the last item linked).
    pub fn output(&mut self) -> UnitId {
        *self.items_of().last().unwrap()
    }

    /// The inventory model's parts.
    pub fn inv(&mut self) -> &mut InvParts {
        self.sim.world.inventory.as_mut().unwrap()
    }

    pub fn set_stat(&mut self, u: UnitId, s: u16, v: i32) {
        let g = &mut self.sim;
        g.events.with(&mut g.game, |_, view| view.set_base(u, s, v));
    }
}

pub fn new_item(sim: &mut WiredGame, class: usize, mode: u32) -> UnitId {
    let mut rq = ItemRequest {
        item: class as i32,
        format: 101,
        ilvl: 5,
        quality: q::NORMAL,
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: None,
        mode,
        init_flags: 1,
    };
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    world
        .with_economy(game, events, |econ, _| {
            econ.create_item(&mut rq, false, spawn)
        })
        .unwrap()
}

// ---- the trade host ----------------------------------------------------------------------

const CAP: usize = 0;
const BUC: usize = 1;

/// Every type is its own and type 0's; helm and shield are armor.
fn trade_equiv() -> EquivMatrix {
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

/// A cap (helm) and a buckler (shield), durability 12.
fn trade_item_tables() -> ItemTables {
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
                t.rare = 1;
                t
            })
            .collect(),
        equiv: trade_equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// Akara and Gheed: Akara's column holds the cap (permanent) and the
/// buckler (1–3); Gheed's column holds nothing. Both buy at 512/1024.
fn vendor_tables() -> VendorTables {
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
    let prices = |c: u16| NpcPrices {
        class: u32::from(c),
        sell: 1024,
        buy: 512,
        rep: 128,
        quests: [(0, 0, 0, 0); 3],
        max_buy: [5000; 3],
    };
    VendorTables {
        items: vec![cap, buc],
        itemtypes,
        equiv: trade_equiv(),
        stat_shift: 6,
        stat_mask: 0x3F,
        monster_levels: vec![[1, 1, 1]; N_MONSTATS],
        interact: vec![class::AKARA, class::GHEED],
        npc: vec![prices(class::AKARA), prices(class::GHEED)],
        difficulty: vec![Default::default(); 3],
        ..VendorTables::default()
    }
}

fn monstats() -> Vec<Monstats> {
    let mut v: Vec<Monstats> = (0..N_MONSTATS).map(|_| blank()).collect();
    for c in [class::AKARA, class::GHEED] {
        v[usize::from(c)].npc = true;
        v[usize::from(c)].interact = true;
    }
    v
}

/// The trade host: Akara and Gheed, a player (class 1, 5,000 gold) for
/// client `client` with a buckler and a cap in its inventory.
pub struct TradeFx {
    pub sim: WiredGame,
    pub player: UnitId,
    pub akara: UnitId,
    pub gheed: UnitId,
    pub buckler: UnitId,
    pub cap: UnitId,
}

impl TradeFx {
    pub fn new(client: u32) -> Self {
        let hooks = ActionHooks::new(
            Arc::new(empty_action_tables()),
            no_drlg(),
            Seed::init_low(7),
            ActionRest::default(),
        );
        let data = UnitData {
            monsters: vec![
                MonsterInfo {
                    mode_chart: false,
                    enabled: true,
                    aidel: [15; 3],
                    moves: 0,
                };
                N_MONSTATS
            ],
            ..UnitData::default()
        };
        let mut events = ActionSim::new(stat_data(), data, hooks);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let mut seed = events.hooks().game_seed;
        let ctl = NpcControl::new(&monstats(), Vec::new(), false, 0, &mut seed).expect("npc");
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        events.hooks().game_seed = seed;
        let mut alloc = |ty, class| {
            let req = AllocRequest {
                ty,
                class,
                room: None,
                add: true,
                fixed_guid: None,
                mode: 1,
                allied: ty == UnitType::Player,
            };
            events
                .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
                .expect("allocated")
        };
        let akara = alloc(UnitType::Monster, u32::from(class::AKARA));
        let gheed = alloc(UnitType::Monster, u32::from(class::GHEED));
        let player = alloc(UnitType::Player, 1);
        events.sys.units.get_mut(player).unwrap().mode = 1;
        events.with(&mut game, |_, v| {
            v.set_base(player, 12, 1);
            v.set_base(player, 14, 5000);
        });
        let mut rest = Rest::default();
        rest.quests.insert(player, PlayerQuests::default());
        let mut world = WiredWorld::new(
            ActionWorld::default(),
            trade_item_tables(),
            quests,
            ctl,
            vendor_tables(),
            rest,
            1000,
        );
        world.state.add_npc(akara);
        world.state.add_npc(gheed);
        let (buckler, cap) = world.with_economy(&mut game, &mut events, |econ, _| {
            let mut make = |record: usize| {
                let mut rq = ItemRequest {
                    item: record as i32,
                    ilvl: 1,
                    quality: 2,
                    format: 1,
                    ..ItemRequest::default()
                };
                let spawn = ItemSpawn {
                    room: None,
                    mode: 4,
                    init_flags: 1,
                };
                econ.create_item(&mut rq, false, spawn).expect("item")
            };
            (make(BUC), make(CAP))
        });
        let pg = events.sys.units.get(player).unwrap().guid;
        let items = [(*b"cap ", ty::HELM, 2, 2), (*b"buc ", ty::SHIE, 2, 2)];
        let tables = inv_tables(&items, N_TYPES, trade_equiv());
        world.inventory = Some(inv_parts(tables, player, 1, pg));
        let mut sim: WiredGame = SimGame::with_world(game, events, world);
        // The player's buckler and cap, stored (mode 0) in its inventory
        // (the one inventory the vendor calls read).
        store_item(&mut sim, player, buckler, 0);
        store_item(&mut sim, player, cap, 0);
        sim.join(client, Some(player), None, client_state::IN_GAME)
            .unwrap();
        sim.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        let at = UnitFacts {
            act: 0,
            pos: Pos { x: 100, y: 100 },
            owner: None,
        };
        for u in [player, akara, gheed] {
            sim.set_unit(u, at);
        }
        TradeFx {
            sim,
            player,
            akara,
            gheed,
            buckler,
            cap,
        }
    }

    pub fn guid(&self, u: UnitId) -> u32 {
        self.sim.game.lists.unit(u).unwrap().guid
    }

    pub fn gold(&mut self) -> i32 {
        let (p, g) = (self.player, &mut self.sim);
        g.events.with(&mut g.game, |_, v| v.stat(p, 14))
    }
}
