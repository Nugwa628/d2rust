//! C→S 0x31, 0x40 and 0x58 on the wired host: `SimGame<ActionSim,
//! WiredWorld>` (`WiredWorld::quests`: the real `QuestControl` on
//! `wiring::economy::EconomyQuests` over the action sim's own units, on
//! the interaction `Desk`), through the real host frame. Only the seams
//! this host does not provide are staged (`Rest`).
//!
//! Act II–V callbacks stay `QuestRest::unhandled`: every test asserts the
//! exact list they reach.
//!
//! Part of the world handler's test suite; its [`Fx`] fixture is shared by
//! `quests_act1`, `quests_act2`, `quests_act5` and `quest_objects`.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Monstats, Record};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::{DrlgData, Dungeon, LevelTypes, TileInfo, TileSource};
use d2_sim::game::Game;
use d2_sim::items::ItemTables;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::StatData;
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{
    ActionHooks, ActionSim, ActionTables, DrlgWorld, ObjectRoute, Pending,
};
use d2_sim::wiring::economy::{EconomyQuests, QuestRest};
use d2_sim::wiring::interaction::{HirelingRest, NpcRest, PlayerQuestsRef, VendorRest};
use d2_sim::world::npc::{class, HireRow, ImbueMods, InvEntry, ItemFacts, NpcControl};
use d2_sim::world::quests::{
    PlayerQuests, QuestChain, QuestControl, QuestTables, TextList, UnitKind,
};
use d2_sim::world::vendors::price::Bonus;
use d2_sim::world::vendors::{Transaction, VendorTables};

use super::*;
use crate::adapters::handlers::world::{ActionWorld, Outbox, WiredWorld};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::seams::PlayerGate;

const N_MONSTATS: usize = 400;
const GAME_SEED: u32 = 1234;
/// Quest flag bits.
const REWARD_GRANTED: u8 = 0;
const REWARD_PENDING: u8 = 1;
const STARTED: u8 = 2;
const UPDATE_QUEST_LOG: u8 = 12;
/// Kashya's hire names (synthetic `hireling` row).
const NAME_FIRST: u16 = 100;
const NAME_LAST: u16 = 104;

// ---- seams without a provider -----------------------------------------------------------

/// The action wiring's seams: `Pending`'s defaults; sends kept; the
/// object interact range staged (`in_range`) and the object routes
/// handed back kept (`quest_objects` tests).
#[derive(Default)]
pub struct ActionRest {
    /// Messages the action wiring sent.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// Whether the operator is in interact range of objects.
    pub in_range: bool,
    /// Object routes handed back to the host.
    pub routes: Vec<ObjectRoute>,
    /// Quest events the tests queue for the host's `after_tick`.
    pub quest_events: Vec<d2_sim::wiring::action::QuestEvent>,
}

impl Pending for ActionRest {
    fn take_quest_events(&mut self) -> Vec<d2_sim::wiring::action::QuestEvent> {
        std::mem::take(&mut self.quest_events)
    }
    fn queue_quest_event(&mut self, e: d2_sim::wiring::action::QuestEvent) {
        self.quest_events.push(e);
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn object_in_range(&self, _: &Game, _: UnitId, _: UnitId) -> bool {
        self.in_range
    }
    fn object_route(&mut self, _: &mut Game, route: ObjectRoute) {
        self.routes.push(route);
    }
}

impl Outbox for ActionRest {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// The interaction and quest seams this host does not provide: staged
/// answers and a log of every call that would change state outside
/// `d2-sim`.
#[derive(Default)]
pub struct Rest {
    /// Staged quest records by player.
    pub quests: BTreeMap<UnitId, PlayerQuests>,
    /// GUID of each NPC unit (for the staged 0x27).
    pub guids: BTreeMap<UnitId, u32>,
    /// Messages the quest control sent, by recipient.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// Log of every call that would change state outside `d2-sim`.
    pub log: Vec<String>,
    /// Staged quest-side seams (`quests_act1`): unit quest chains, level
    /// 8's monster region, J3's near players, parties, room levels (1
    /// when absent), object modes, inventories, the drop result.
    pub chains: BTreeMap<UnitId, QuestChain>,
    /// Level 8's monster region.
    pub den: (u32, u32, u32, u32),
    /// Players near the unit being asked about.
    pub near: Vec<UnitId>,
    /// Party members by player.
    pub party: BTreeMap<UnitId, Vec<UnitId>>,
    /// Room levels by player (1 when absent).
    pub levels: BTreeMap<UnitId, u32>,
    /// Object modes set through the quest world.
    pub object_modes: BTreeMap<UnitId, i32>,
    /// Staged inventories by player.
    pub inventory: BTreeMap<UnitId, Vec<UnitId>>,
    /// Result of the item drop seam.
    pub drop_ok: bool,
    /// The item the next `reward_item` creates (placed in the player's
    /// staged inventory).
    pub reward: Option<UnitId>,
    /// Staged monster owners (GUID, unit type).
    pub owners: BTreeMap<UnitId, (u32, u8)>,
    /// Client save flags by player. Every player with a
    /// staged quest record has a client (a host gives every player one);
    /// absent here = 0.
    pub client_flags: BTreeMap<UnitId, u16>,
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
    /// No hireling: the quest mercenary is granted.
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
    /// The monster spawn: none, so the quest mercenary stops after S→C 0x50.
    fn spawn_mercenary(&mut self, _: UnitId, class: u32, mode: u8) -> Option<UnitId> {
        self.log.push(format!("spawn merc {class} {mode}"));
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
    fn owner(&self, merc: UnitId) -> Option<(u32, u8)> {
        self.owners.get(&merc).copied()
    }
    fn free_unit(&mut self, _: UnitId) {}
    fn queue_room_removal(&mut self, _: UnitId) {}
    fn death_event(&mut self, _: UnitId) {}
    fn dismiss(&mut self, _: UnitId) {}
    fn warp_to(&mut self, pet: UnitId, player: UnitId) {
        self.log.push(format!("warp {} {}", pet.0, player.0));
    }
    fn level_events(&mut self, _: UnitId, _: UnitId) {}
    fn reapply_item_stats(&mut self, _: UnitId) {}
}

/// No vendor path runs in these tests.
impl VendorRest for Rest {
    fn players_in_level(&self, _: u16) -> i32 {
        1
    }
    fn player_level_id(&self, _: UnitId) -> u16 {
        1
    }
    fn gold_cap(&self, _: UnitId) -> i32 {
        0
    }
    fn stash_cap(&self, _: UnitId) -> i32 {
        0
    }
    fn drop_gold(&mut self, _: UnitId, _: i32) {}
    fn last_bought(&self, _: UnitId) -> u32 {
        u32::MAX
    }
    fn set_last_bought(&mut self, _: UnitId, _: u32) {}
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
    fn send_transaction(&mut self, _: UnitId, _: Transaction) {}
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
    fn place_in_backpack(&mut self, _: UnitId, _: UnitId) -> bool {
        false
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
    fn client_save_flags(&self, player: UnitId) -> Option<u16> {
        self.quests
            .contains_key(&player)
            .then(|| self.client_flags.get(&player).copied().unwrap_or(0))
    }
    fn set_client_save_flags(&mut self, player: UnitId, flags: u16) {
        self.client_flags.insert(player, flags);
    }
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
    fn quest_chain(&mut self, u: UnitId) -> Option<&mut QuestChain> {
        self.chains.get_mut(&u)
    }
    fn unit_act(&self, _: UnitId) -> Option<u8> {
        Some(0)
    }
    fn unit_level(&self, u: UnitId) -> Option<u32> {
        Some(self.levels.get(&u).copied().unwrap_or(1))
    }
    /// Players are the units with quest records; the rest is `Other`.
    fn unit_kind(&self, u: UnitId) -> UnitKind {
        if self.quests.contains_key(&u) {
            UnitKind::Player
        } else if self.chains.get(&u).is_some_and(|c| c.0.contains(&31)) {
            // Shenk: monster init links superunique 42 to chain 31.
            UnitKind::Monster {
                class: 0,
                superunique: Some(42),
                owner: None,
            }
        } else {
            UnitKind::Other
        }
    }
    fn players_near(&self, _: UnitId) -> Vec<UnitId> {
        self.near.clone()
    }
    fn party_members(&self, p: UnitId) -> Option<Vec<UnitId>> {
        self.party.get(&p).cloned()
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    /// S→C 0x27: type 1, the NPC's GUID, then 34 list bytes (staged as
    /// zeros). The entries are logged.
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, list: &[(u16, u32)]) {
        self.log.push(format!("text list {} {list:?}", npc.0));
        let mut m = vec![0x27, 1];
        m.extend_from_slice(&self.guids[&npc].to_le_bytes());
        m.extend_from_slice(&[0; 34]);
        self.sent.push((player, m));
    }
    fn inventory(&self, p: UnitId) -> Vec<UnitId> {
        self.inventory.get(&p).cloned().unwrap_or_default()
    }
    /// Logged; the staged inventory is emptied.
    fn delete_item(&mut self, p: UnitId, code: [u8; 4]) {
        self.log
            .push(format!("delete {} {}", p.0, String::from_utf8_lossy(&code)));
        self.inventory.remove(&p);
    }
    fn reward_item(
        &mut self,
        p: UnitId,
        code: [u8; 4],
        level: i32,
        quality: u8,
        _: bool,
    ) -> Option<UnitId> {
        self.log.push(format!(
            "reward {} {level} {quality}",
            String::from_utf8_lossy(&code)
        ));
        let item = self.reward.take()?;
        self.inventory.entry(p).or_default().push(item);
        Some(item)
    }
    fn drop_item_at(&mut self, u: UnitId, code: [u8; 4], quality: u8) -> bool {
        self.log.push(format!(
            "drop {} {} {quality}",
            u.0,
            String::from_utf8_lossy(&code)
        ));
        self.drop_ok
    }
    fn den_region(&self) -> (u32, u32, u32, u32) {
        self.den
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
    fn object_mode(&self, o: UnitId) -> i32 {
        self.object_modes.get(&o).copied().unwrap_or(0)
    }
    fn set_object_mode(&mut self, o: UnitId, mode: i32) {
        self.object_modes.insert(o, mode);
    }
    /// Reached only when a reward is not routed to the NPC control
    /// block: the tests assert it never is.
    fn mercenary_reward(&mut self, p: UnitId, npc: u16) {
        self.log.push(format!("rest merc reward {} {npc}", p.0));
    }
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

struct NoTiles;

impl TileSource for NoTiles {
    fn dt1(&self, _: &[u8]) -> Option<&[TileInfo]> {
        None
    }
}

struct NoLevelTypes;

impl LevelTypes for NoLevelTypes {}

// ---- the game ---------------------------------------------------------------------------

/// The wired world with the staged seams.
pub type World = WiredWorld<Rest>;
/// The game under test: action sim plus wired world.
pub type Sim = SimGame<ActionSim<ActionRest>, World>;

fn monstats() -> Vec<Monstats> {
    let mut v: Vec<Monstats> = (0..N_MONSTATS)
        .map(|_| Monstats::decode(&vec![0u8; Monstats::SIZE]))
        .collect();
    for c in [class::AKARA, class::KASHYA] {
        v[usize::from(c)].npc = true;
        v[usize::from(c)].interact = true;
    }
    v
}

/// Kashya's Normal row (version 0: classic), five names.
fn hirelings() -> Vec<HireRow> {
    vec![HireRow {
        version: 0,
        class: 271,
        act: 1,
        difficulty: 1,
        seller: u32::from(class::KASHYA),
        gold: 100,
        level: 1,
        name_first: NAME_FIRST,
        name_last: NAME_LAST,
    }]
}

/// The shared quest fixture: a host with Akara, Kashya and one player.
pub struct Fx {
    /// The host frame wrapper around the game.
    pub h: TestHost<Sim>,
    /// The joined player.
    pub player: UnitId,
    /// Akara's unit.
    pub akara: UnitId,
    /// Kashya's unit.
    pub kashya: UnitId,
}

impl Fx {
    /// Game creation: the action sim, the NPC control and the quests on
    /// the game seed, Akara, Kashya and the player allocated (real unit
    /// records), Kashya's hire list made; the player enters (mode 0) with
    /// `flags` as their current record, then joins.
    pub fn new(flags: impl FnOnce(&mut PlayerQuests)) -> Self {
        let tables = ActionTables {
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
        };
        let drlg = DrlgWorld {
            dungeon: Dungeon::default(),
            data: Arc::new(DrlgData::default()),
            tiles: Box::new(NoTiles),
            types: Box::new(NoLevelTypes),
        };
        let hooks = ActionHooks::new(
            Arc::new(tables),
            drlg,
            Seed::init_low(GAME_SEED),
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
        let mut events = ActionSim::new(Arc::new(StatData::default()), data, hooks);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();

        let mut seed = events.hooks().game_seed;
        let mut ctl = NpcControl::new(&monstats(), hirelings(), false, 0, &mut seed).unwrap();
        let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        events.hooks().game_seed = seed;
        ctl.make_hire_list(class::KASHYA).unwrap();

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
                .unwrap()
        };
        let akara = alloc(UnitType::Monster, u32::from(class::AKARA));
        let kashya = alloc(UnitType::Monster, u32::from(class::KASHYA));
        let player = alloc(UnitType::Player, 1);
        events.sys.units.get_mut(player).unwrap().mode = 1;

        let mut rest = Rest::default();
        for u in [akara, kashya] {
            rest.guids.insert(u, game.lists.unit(u).unwrap().guid);
        }
        let mut pq = PlayerQuests::default();
        flags(&mut pq);
        rest.quests.insert(player, pq);
        let mut world: World = WiredWorld::new(
            ActionWorld::default(),
            ItemTables::default(),
            quests,
            ctl,
            VendorTables::default(),
            rest,
            1000,
        );
        world.state.add_npc(akara);
        world.state.add_npc(kashya);
        world.with_economy(&mut game, &mut events, |econ, p| {
            let mut w = EconomyQuests::new(econ, &mut *p.rest);
            p.quests.player_enters(&mut w, player, 0).unwrap();
        });
        world.rest.sent.clear();
        world.rest.log.clear();

        let mut s: Sim = SimGame::with_world(game, events, world);
        s.join(0, Some(player), None, client_state::IN_GAME)
            .unwrap();
        s.set_player(
            player,
            PlayerFields {
                gate: PlayerGate {
                    mode: 1,
                    uninterruptable: false,
                },
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        Fx {
            h: host(s),
            player,
            akara,
            kashya,
        }
    }

    /// The wired world.
    pub fn world(&mut self) -> &mut World {
        &mut self.h.game.world
    }

    /// The GUID of `u`.
    pub fn guid(&self, u: UnitId) -> u32 {
        self.h.game.game.lists.unit(u).unwrap().guid
    }

    /// The player's current (Normal) record.
    pub fn record(&self) -> [u8; 96] {
        self.h.game.world.rest.quests[&self.player].flags[0].0
    }

    /// The game-wide quest record.
    pub fn game_record(&self) -> [u8; 96] {
        self.h.game.world.quests.game.0
    }

    /// Every error: action adapters, unit dispatch, interaction state,
    /// handler faults.
    pub fn errors(&self) -> Vec<String> {
        let s = &self.h.game;
        let mut e: Vec<String> = s
            .events
            .sys
            .hooks
            .errors
            .iter()
            .map(|e| format!("{e:?}"))
            .collect();
        e.extend(s.events.sys.errors.iter().map(|e| format!("{e:?}")));
        e.extend(s.world.state.errors.iter().map(|e| format!("{e:?}")));
        e.extend(s.world.action.faults.iter().map(|f| format!("{f:?}")));
        e
    }

    /// The rest's log, led by the sound events the quests queued on the
    /// game (they are not the rest's), which are cleared.
    pub fn take_log(&mut self) -> Vec<String> {
        let g = &mut self.h.game.game;
        let mut log: Vec<String> = g
            .sounds
            .queued()
            .map(|(u, s)| format!("sound {} {}", u.0, s.event))
            .collect();
        let units: Vec<_> = g.sounds.queued().map(|(u, _)| u).collect();
        for u in units {
            g.sounds.clear(u);
        }
        log.extend(std::mem::take(&mut self.world().rest.log));
        log
    }
}

/// C→S 0x31 (9 bytes): NPC GUID, message, pad.
fn quest_message(guid: u32, msg: u16) -> Vec<u8> {
    let mut m = vec![0x31];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&msg.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    m
}

// ---- tests ------------------------------------------------------------------------------

/// The first offered, not hired, slot of Kashya's hire list.
fn first_offer(f: &mut Fx) -> u16 {
    let h = f.world().npc.record(class::KASHYA).unwrap().hire.as_ref();
    let slot = h.unwrap().slots.iter().find(|s| s.offered && !s.hired);
    slot.unwrap().name
}

fn get(r: &[u8; 96], q: usize, b: u8) -> bool {
    u16::from_le_bytes([r[2 * q], r[2 * q + 1]]) & (1 << b) != 0
}

// Akara's message 64 starts Den of Evil; the chat end sets the quest slot.
#[test]
fn akara_message_64_starts_den_of_evil_then_chat_end() {
    // 0x31 (Akara, message 64) → 0x27, 0x29; the chat end →
    // `5d 01 00 01 0000`; slot 1 = `04 00`.
    let mut f = Fx::new(|_| {});
    let g = f.guid(f.akara);
    let before = f.record();
    let (code, got) = send(&mut f.h, &quest_message(g, 64));
    assert_eq!(code, ResultCode::Done);
    let mut m27 = vec![0x27, 1];
    m27.extend_from_slice(&g.to_le_bytes());
    m27.extend_from_slice(&[0; 34]);
    let m29 = [&[0x29u8][..], &f.game_record()[..]].concat();
    assert_eq!(got, vec![m27, m29]);
    let mut want = before;
    want[2] |= 1 << STARTED;
    assert_eq!(f.record(), want);
    assert_eq!(f.world().quests.record(1).unwrap().state, 2);
    // The refresh at state 2 (records newest first): the Act I intro's
    // first-talk line for a sorceress (state 1, line 12), then A1Q1's
    // message state 1 line (state 2 → 1, line 65).
    let log = vec![format!("text list {} [(12, 0), (65, 2)]", f.akara.0)];
    assert_eq!(f.take_log(), log);

    // Chat end through the NPC module.
    let mut end = vec![0x30, 1, 0, 0, 0];
    end.extend_from_slice(&g.to_le_bytes());
    let (code, got) = send(&mut f.h, &end);
    assert_eq!(
        (code, got),
        (ResultCode::Done, vec![hex("5d 01 00 01 0000")])
    );
    assert_eq!(f.record()[2..4], [0x04, 0x00]);
    assert_eq!(f.errors(), Vec::<String>::new());
}

/// Routing only: the NPC module's own tests cover the expansion and refill
/// branches.
#[test]
fn kashya_message_92_grants_the_mercenary_on_the_npc_control() {
    // Blood Raven's kill gave 2.13 and 2.1.
    let mut f = Fx::new(|q| {
        q.flags[0].set(2, 13);
        q.flags[0].set(2, REWARD_PENDING);
    });
    let g = f.guid(f.kashya);
    let name = first_offer(&mut f);
    let before = f.record();
    let (code, got) = send(&mut f.h, &quest_message(g, 92));
    assert_eq!(code, ResultCode::Done);
    // 0x28, S→C 0x50 (15 bytes): u16 2, the slot's name, zeros, then the
    // text refresh.
    let mut m50 = vec![0x50, 2, 0];
    m50.extend_from_slice(&name.to_le_bytes());
    m50.extend_from_slice(&[0; 10]);
    assert_eq!(got.len(), 4);
    assert_eq!((got[0][0], got[2][0], got[3][0]), (0x28, 0x27, 0x29));
    assert_eq!(got[1], m50);
    let mut want = before;
    want[4] = (want[4] | 1 << REWARD_GRANTED) & !(1 << REWARD_PENDING);
    assert_eq!(f.record(), want);
    assert_eq!(f.world().quests.record(2).unwrap().state, 5);
    let ctl = &f.world().npc;
    let slots = &ctl
        .record(class::KASHYA)
        .unwrap()
        .hire
        .as_ref()
        .unwrap()
        .slots;
    assert!(slots.iter().any(|s| s.name == name && s.hired));
    // The reward ran on the NPC control (the spawn seam: mode 1, spreads
    // 4, 6, 12; the rest knows only spread 4, so it sees one call) after
    // the quest call, never on `QuestRest::mercenary_reward`, and
    // before the refresh; the refreshed Kashya lines: the Act I intro's
    // (line 24) and A1Q2's message state 4 (line 92).
    let mut log: Vec<String> = vec!["spawn merc 271 1".to_string()];
    log.push(format!("text list {} [(24, 0), (92, 2)]", f.kashya.0));
    assert_eq!(f.take_log(), log);
    assert_eq!(f.errors(), Vec::<String>::new());
}

#[test]
fn kashya_message_92_without_reward_pending_does_nothing() {
    let mut f = Fx::new(|_| {});
    let g = f.guid(f.kashya);
    let before = f.record();
    let (code, got) = send(&mut f.h, &quest_message(g, 92));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    assert_eq!(f.record(), before);
    assert_eq!(f.take_log(), Vec::<String>::new());
    assert_eq!(f.errors(), Vec::<String>::new());
}

// Requesting quest data (0x40) sends the quest record (0x28) and the 0x52 block.
#[test]
fn request_quest_data() {
    let mut f = Fx::new(|_| {});
    let (code, got) = send(&mut f.h, &[0x40]);
    assert_eq!(code, ResultCode::Done);
    assert_eq!(
        got,
        vec![
            [&hex("28 06 00000000 00")[..], &f.record()[..]].concat(),
            [&[0x52u8][..], &[0u8; 41][..]].concat(),
        ]
    );
    assert_eq!(f.take_log(), Vec::<String>::new());
    assert_eq!(f.errors(), Vec::<String>::new());
}

// One flag bit in the player's record changes exactly one byte of the 0x28.
#[test]
fn one_flag_bit_changes_one_byte_of_the_0x28() {
    // Slot 3 bit 2 set in the player's record → exactly byte 7 + 6
    // of the 0x28 differs (bit 2), nothing else in the frame.
    let run = |perturb: bool| {
        let mut f = Fx::new(|_| {});
        if perturb {
            let p = f.player;
            f.world().rest.quests.get_mut(&p).unwrap().flags[0].set(3, STARTED);
        }
        send(&mut f.h, &[0x40])
    };
    let (a, b) = (run(false), run(true));
    assert_eq!(a.0, b.0);
    assert_eq!(a.1.len(), b.1.len());
    let diffs: Vec<(usize, usize, u8)> =
        a.1.iter()
            .zip(&b.1)
            .enumerate()
            .flat_map(|(m, (x, y))| {
                assert_eq!(x.len(), y.len());
                x.iter()
                    .zip(y)
                    .enumerate()
                    .filter(|(_, (p, q))| p != q)
                    .map(move |(i, (p, q))| (m, i, p ^ q))
            })
            .collect();
    assert_eq!(diffs, [(0, 7 + 6, 1 << STARTED)]);
}

// Quest completed (0x58) sets the log bit; slot 0x2A is out of range.
#[test]
fn quest_completed_sets_the_log_bit() {
    let mut f = Fx::new(|_| {});
    let before = f.record();
    let (code, got) = send(&mut f.h, &hex("58 0500"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    let mut want = before;
    want[11] |= 1 << (UPDATE_QUEST_LOG - 8);
    assert_eq!(f.record(), want);
    assert!(get(&f.record(), 5, UPDATE_QUEST_LOG));
    // Slot 41 is the last below 0x2A; 0x2A is result 2, nothing set.
    let (code, got) = send(&mut f.h, &hex("58 2900"));
    assert_eq!((code, got), (ResultCode::Done, vec![]));
    want[83] |= 1 << (UPDATE_QUEST_LOG - 8);
    assert_eq!(f.record(), want);
    let (code, got) = send(&mut f.h, &hex("58 2a00"));
    assert_eq!((code, got), (ResultCode::Invalid, vec![]));
    assert_eq!(f.record(), want);
    assert_eq!(f.take_log(), Vec::<String>::new());
    assert_eq!(f.errors(), Vec::<String>::new());
}

#[test]
fn same_seed_same_run() {
    let run = || {
        let mut f = Fx::new(|q| q.flags[0].set(2, REWARD_PENDING));
        let (a, k) = (f.guid(f.akara), f.guid(f.kashya));
        let mut out = Vec::new();
        for m in [quest_message(a, 64), quest_message(k, 92), vec![0x40]] {
            out.push(send(&mut f.h, &m));
        }
        (out, f.record(), f.game_record(), f.take_log())
    };
    assert_eq!(run(), run());
}

/// Host requests a quest rule raised (here queued on the quest control
/// directly, as `QuestControl::end_game` / `save_pass` do from the rules)
/// are drained after the next tick's
/// steps into `SimGame::host_requests`, in call order, for the session
/// layer; the quest control keeps none.
#[test]
fn quest_host_requests_are_drained_after_the_tick_in_call_order() {
    use d2_sim::world::quests::HostRequest;
    let mut f = Fx::new(|_| {});
    f.world().quests.save_pass();
    f.world().quests.end_game();
    assert!(f.h.game.host_requests.is_empty(), "not before the tick");
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
    assert!(f.world().quests.host_requests.is_empty());
    assert_eq!(
        f.h.game.take_host_requests(),
        [HostRequest::SavePass, HostRequest::EndGame]
    );
    assert!(f.h.game.take_host_requests().is_empty());
}

// A tick's messages leave in the order they were produced.
#[test]
fn a_ticks_messages_leave_in_production_order() {
    // A rest (quest) message queued before the tick's last steps, then a
    // player death announced by the action wiring in `after_tick`: the
    // client gets the quest message first, not grouped after the action
    // wiring's sends.
    let mut f = Fx::new(|_| {});
    let p = f.player;
    let marker = vec![0x5D, 0x01, 0x00, 0x01, 0x00, 0x00];
    f.world().rest.sent.push((p, marker.clone()));
    let s = &mut f.h.game;
    s.events.start_death(&mut s.game, p);
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
    let got = f.h.receive(0);
    let at = got
        .iter()
        .position(|m| *m == marker)
        .expect("the quest message");
    assert!(
        at + 1 < got.len(),
        "the death's messages follow the quest message: {got:02x?}"
    );
}

/// A queued object operate waits for the player's stop (listed in
/// `ActionHooks::player_stops`); a mode change for another reason (a hit)
/// keeps it queued.
#[test]
fn a_queued_object_operate_waits_for_the_players_stop() {
    let mut f = Fx::new(|_| {});
    let p = f.player;
    let g = f.guid(f.akara);
    f.world().object_queued.push((p, g));
    // Hit recovery (mode 4): not a stop, the operate stays queued.
    f.h.game.events.sys.units.get_mut(p).unwrap().mode = 4;
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
    assert_eq!(f.world().object_queued, [(p, g)]);
    // The stop: the 0x13 object case runs again and the queue is taken.
    f.h.game.events.hooks().player_stops.push(p);
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
    assert!(f.world().object_queued.is_empty());
    assert!(f.h.game.events.hooks().player_stops.is_empty());
}

/// A tile walk (the pending 0x13 `{5, GUID}`) waits for the player's stop
/// like the object walk, then the tile case runs again.
#[test]
fn a_queued_tile_walk_waits_for_the_players_stop() {
    let mut f = Fx::new(|_| {});
    let p = f.player;
    let g = f.guid(f.akara);
    f.world().tile_queued.push((p, g));
    f.h.game.events.sys.units.get_mut(p).unwrap().mode = 4;
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
    assert_eq!(f.world().tile_queued, [(p, g)]);
    f.h.game.events.hooks().player_stops.push(p);
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
    assert!(f.world().tile_queued.is_empty());
}

/// The client position resync 0x5F is no new walk request: it keeps the
/// pending tile 0x13, a real walk drops it.
#[test]
fn a_position_resync_keeps_the_queued_tile_walk() {
    use crate::adapters::handlers::walk::{WalkCall, RESYNC_ID};
    use crate::adapters::handlers::world::WorldHost;
    let mut f = Fx::new(|_| {});
    let p = f.player;
    let g = f.guid(f.akara);
    f.world().tile_queued.push((p, g));
    let s = &mut f.h.game;
    let call = |id| WalkCall {
        player: p,
        id,
        a: 0,
        b: 0,
    };
    let _ = WorldHost::walk(&mut s.world, &mut s.game, &mut s.events, call(RESYNC_ID));
    assert_eq!(f.world().tile_queued, [(p, g)]);
    let s = &mut f.h.game;
    let _ = WorldHost::walk(&mut s.world, &mut s.game, &mut s.events, call(0x03));
    assert!(f.world().tile_queued.is_empty());
}
