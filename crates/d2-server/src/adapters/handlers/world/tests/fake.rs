//! A fake of the seams with no `d2-sim` provider: `NpcWorld`,
//! `NpcVendors`, `NpcLink`, `VendorWorld`, `QuestWorld`. It holds the
//! units, stats, interaction lists, items and quest records the tests
//! need; the NPC seam's quest calls go to the game's real
//! [`QuestControl`]. Everything else does nothing and is logged. The tests
//! of this module tree use it to run the real `d2-sim` NPC, vendor and
//! quest modules without the wired host.

use std::collections::BTreeMap;

use d2_data::tables::{Monstats, Record};
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::tick::timer::TimerRun;
use d2_sim::tick::{EventDispatch, TickHooks};
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::world::npc::{
    self, ImbueMods, InteractionList, InvEntry, ItemFacts, MercInit, NpcControl, NpcError,
    NpcVendors, NpcWorld,
};
use d2_sim::world::quests::{
    self, PlayerQuests, QuestChain, QuestControl, QuestFlags, QuestTables, QuestWorld, TextList,
    UnitKind,
};
use d2_sim::world::vendors::{
    NpcLink, PriceItem, Transaction, VendorRecord, VendorTables, VendorWorld,
};

use crate::adapters::handlers::world::{
    NpcCall, QuestCall, VendorCall, WorldError, WorldFault, WorldHost,
};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::seams::PlayerGate;

/// A fake unit.
#[derive(Clone, Debug, Default)]
pub struct FUnit {
    /// The unit's GUID.
    pub guid: u32,
    /// Unit type (0 player, 1 monster, 4 item).
    pub ty: u8,
    /// Class id.
    pub class: u16,
    /// Animation mode.
    pub mode: u8,
    /// Act the unit is in.
    pub act: u8,
    /// Position, x.
    pub x: i32,
    /// Position, y.
    pub y: i32,
    /// Items: item flags (0x10 identified).
    pub flags: u32,
    /// Items: owner.
    pub owner: Option<UnitId>,
}

/// The fake seam provider: units, stats and records held in maps.
#[derive(Default)]
pub struct Fake {
    /// The fake units.
    pub units: BTreeMap<UnitId, FUnit>,
    /// Unit stats by (unit, stat id).
    pub stats: BTreeMap<(UnitId, u16), u32>,
    /// Each player's interaction (type, GUID).
    pub interact: BTreeMap<UnitId, (u8, u32)>,
    /// The NPCs' interaction lists.
    pub lists: BTreeMap<UnitId, InteractionList>,
    /// The players' inventory entries.
    pub inventory: BTreeMap<UnitId, Vec<InvEntry>>,
    /// The players' quest records.
    pub quests: BTreeMap<UnitId, PlayerQuests>,
    /// The quest chains of units.
    pub chains: BTreeMap<UnitId, QuestChain>,
    /// The last item GUID each player bought.
    pub last_bought: BTreeMap<UnitId, u32>,
    /// The players' busy byte of the player data.
    pub byte_4c: BTreeMap<UnitId, u8>,
    /// The game seed.
    pub seed: Seed,
    /// Whether the game is an expansion game.
    pub expansion: bool,
    /// The game's quest control, lent to the NPC seam's quest calls.
    pub ctl: Option<QuestControl>,
    /// The messages sent, with the receiving unit.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// A log of the calls that do nothing else.
    pub log: Vec<String>,
}

impl Fake {
    /// Adds the unit `u`.
    pub fn add(&mut self, u: UnitId, f: FUnit) {
        self.units.insert(u, f);
    }

    fn by_guid(&self, ty: u8, guid: u32) -> Option<UnitId> {
        self.units
            .iter()
            .find(|(_, f)| f.ty == ty && f.guid == guid)
            .map(|(&u, _)| u)
    }

    fn unit(&self, u: UnitId) -> FUnit {
        self.units.get(&u).cloned().unwrap_or_default()
    }

    fn with_ctl(&mut self, f: impl FnOnce(&mut QuestControl, &mut Self)) {
        let mut ctl = self.ctl.take().expect("quest control");
        f(&mut ctl, self);
        self.ctl = Some(ctl);
    }
}

impl NpcWorld for Fake {
    fn item_format(&self) -> u16 {
        0
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.unit(unit).guid
    }
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.by_guid(1, guid)
    }
    fn unit_by_guid(&self, guid: u32) -> Option<(u8, UnitId)> {
        self.units
            .iter()
            .find(|(_, f)| f.guid == guid)
            .map(|(&u, f)| (f.ty, u))
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        self.units.get(&unit).filter(|f| f.ty == 1).map(|f| f.class)
    }
    fn mode(&self, unit: UnitId) -> u8 {
        self.unit(unit).mode
    }
    fn set_mode(&mut self, unit: UnitId, mode: u8) {
        self.log.push(format!("mode {} {mode}", unit.0));
    }
    fn clear_unit_flag(&mut self, _: UnitId, _: u32) {}
    /// Chebyshev distance (the fake does not model the game's own distance).
    fn distance(&self, a: UnitId, b: UnitId) -> i32 {
        let (a, b) = (self.unit(a), self.unit(b));
        (a.x - b.x).abs().max((a.y - b.y).abs())
    }
    fn axis_check(&self, player: UnitId, npc: UnitId) -> u32 {
        u32::from(self.distance(player, npc) > 50)
    }
    fn same_act(&self, player: UnitId, npc: UnitId) -> bool {
        self.unit(player).act == self.unit(npc).act
    }
    fn unit_check(&self, _: UnitId, guid: u32) -> u32 {
        u32::from(self.by_guid(1, guid).is_none())
    }
    fn player_busy(&self, player: UnitId) -> u32 {
        u32::from(self.interact.contains_key(&player))
    }
    fn start_allowed(&self, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn tristram_cain_busy(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn clear_path(&mut self, unit: UnitId) {
        self.log.push(format!("clear path {}", unit.0));
    }
    fn npc_ai_param(&mut self, _: UnitId, _: u32) {}
    fn reschedule_ai_think(&mut self, _: UnitId) {}
    fn approach(&mut self, player: UnitId, npc: UnitId) {
        self.log.push(format!("approach {} {}", player.0, npc.0));
    }
    fn interaction(&mut self, npc: UnitId) -> Option<&mut InteractionList> {
        self.lists.get_mut(&npc)
    }
    fn interact_unit(&self, player: UnitId) -> Option<(u8, u32)> {
        self.interact.get(&player).copied()
    }
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        self.interact.entry(player).or_insert((unit_type, guid));
    }
    fn reset_interact(&mut self, player: UnitId) {
        self.interact.remove(&player);
    }
    fn pet(&self, _: UnitId, _: u8, _: u8) -> Option<UnitId> {
        None
    }
    fn pets(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn stat(&self, unit: UnitId, stat: u16) -> u32 {
        self.stats.get(&(unit, stat)).copied().unwrap_or(0)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> u32 {
        NpcWorld::stat(self, unit, stat)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: u32) {
        self.stats.insert((unit, stat), value);
    }
    fn set_stat_send(&mut self, player: UnitId, stat: u16, value: u32) {
        NpcWorld::set_stat(self, player, stat, value);
    }
    fn max_life(&self, _: UnitId) -> u32 {
        0
    }
    fn max_mana(&self, _: UnitId) -> u32 {
        0
    }
    fn max_stamina(&self, _: UnitId) -> u32 {
        0
    }
    fn states_count(&self) -> u16 {
        0
    }
    fn has_state(&self, _: UnitId, _: u16) -> bool {
        false
    }
    fn curable(&self, _: u16) -> bool {
        false
    }
    fn has_state_list(&self, _: UnitId, _: u16) -> bool {
        false
    }
    fn remove_state_list(&mut self, _: UnitId, _: u16) {}
    fn attach_sound(&mut self, unit: UnitId, sound: u16) {
        self.log.push(format!("sound {} {sound}", unit.0));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn quest_flags(&self, player: UnitId) -> QuestFlags {
        self.quests
            .get(&player)
            .map(|q| q.flags[0])
            .unwrap_or_default()
    }
    fn quest_text_list(&mut self, player: UnitId, npc: UnitId) -> TextList {
        let mut list = TextList::new();
        self.with_ctl(|c, w| c.npc_activate(w, player, npc, &mut list));
        list
    }
    /// The text-list encoding is not modelled: zeros.
    fn encode_text_list(&self, _: &TextList) -> [u8; 34] {
        [0; 34]
    }
    fn send_game_quests(&mut self, player: UnitId) {
        self.with_ctl(|c, w| c.send_game_flags(w, player));
    }
    fn send_player_quests(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        quests::send_player_flags(self, player, unit_type, guid);
    }
    fn quest_chat_end(&mut self, player: UnitId, npc: UnitId) {
        self.with_ctl(|c, w| c.npc_deactivate(w, player, npc));
    }
    fn respec_offer(&mut self, _: UnitId) {}
    fn respec_done(&mut self, _: UnitId) {}
    fn imbue_granted(&mut self, _: UnitId) {}
    fn socket_granted(&mut self, _: UnitId) {}
    fn personalize_granted(&mut self, _: UnitId) {}
    fn act_completion(&mut self, _: UnitId, _: UnitId, _: u32, _: u32) {}
    fn reset_stats(&mut self, _: UnitId) {}
    fn reset_skills(&mut self, _: UnitId) {}
    fn respec_sound(&mut self, _: UnitId) {}
    fn player_name(&self, _: UnitId) -> Vec<u8> {
        Vec::new()
    }
    fn act_change(&mut self, _: UnitId, _: u32, _: u32) {}
    fn activate_waypoint(&mut self, _: UnitId, _: u32) {}
    fn inventory(&self, player: UnitId) -> Vec<InvEntry> {
        self.inventory.get(&player).cloned().unwrap_or_default()
    }
    fn identify(&mut self, item: UnitId) {
        self.log.push(format!("identify {}", item.0));
        if let Some(f) = self.units.get_mut(&item) {
            f.flags |= 0x10;
        }
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
    fn set_item_page(&mut self, _: UnitId, _: u8) {}
    fn set_item_flag(&mut self, _: UnitId, _: u32) {}
    fn personal_name(&self, _: UnitId) -> Vec<u8> {
        Vec::new()
    }
    fn set_personal_name(&mut self, _: UnitId, _: &[u8]) {}
    fn place_or_drop(&mut self, _: UnitId, _: UnitId) {}
    fn max_sockets(&self, _: UnitId) -> u32 {
        0
    }
    fn add_sockets(&mut self, _: UnitId, _: u32) {}
    fn item_seed(&mut self, _: UnitId) -> &mut Seed {
        &mut self.seed
    }
    fn spawn_mercenary(&mut self, _: UnitId, _: u32, _: u8) -> Option<UnitId> {
        None
    }
    fn init_mercenary(&mut self, _: UnitId, _: UnitId, _: &MercInit) {}
    fn revive_mercenary(&mut self, _: UnitId, _: UnitId) {}
}

impl NpcVendors for Fake {
    fn open_trade(
        &mut self,
        _: &mut NpcControl,
        player: UnitId,
        npc: UnitId,
        single: bool,
        gamble: bool,
    ) -> Result<(), NpcError> {
        self.log.push(format!(
            "open trade {} {} {single} {gamble}",
            player.0, npc.0
        ));
        Ok(())
    }
    fn drop_gamble_list(&mut self, player: UnitId, npc: UnitId) {
        self.log
            .push(format!("drop gamble list {} {}", player.0, npc.0));
    }
    fn pay(&mut self, player: UnitId, cost: u32) -> bool {
        let gold = NpcWorld::stat(self, player, npc::stat::GOLD);
        if gold < cost {
            return false;
        }
        NpcWorld::set_stat(self, player, npc::stat::GOLD, gold - cost);
        true
    }
    fn repair(&mut self, _: UnitId) {}
}

impl NpcLink for Fake {
    fn npc_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.by_guid(1, guid).filter(|u| self.lists.contains_key(u))
    }
    fn npc_class(&self, npc: UnitId) -> u16 {
        self.unit(npc).class
    }
    fn is_interact_unit(&self, player: UnitId, npc: UnitId) -> bool {
        self.interact.get(&player) == Some(&(1, self.unit(npc).guid))
    }
    fn interaction_empty(&self, npc: UnitId) -> bool {
        self.lists.get(&npc).is_none_or(|l| l.nodes.is_empty())
    }
    fn hire_list_made(&self, _: u16) -> bool {
        false
    }
    fn set_hire_list_made(&mut self, _: u16) {}
    fn make_hire_list(&mut self, _: u16, _: &mut d2_sim::rng::Seed) {}
}

impl VendorWorld for Fake {
    fn difficulty(&self) -> u8 {
        0
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn item_format(&self) -> u16 {
        0
    }
    fn game_type(&self) -> u8 {
        0
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.unit(unit).guid
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.by_guid(0, guid)
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.by_guid(4, guid)
    }
    fn stat(&self, unit: UnitId, id: u16, _: u16) -> i32 {
        NpcWorld::stat(self, unit, id) as i32
    }
    fn base_stat(&self, unit: UnitId, id: u16, _: u16) -> i32 {
        NpcWorld::stat(self, unit, id) as i32
    }
    fn set_stat(&mut self, unit: UnitId, id: u16, _: u16, value: i32) {
        NpcWorld::set_stat(self, unit, id, value as u32);
    }
    fn quest_slot(&self, _: UnitId, _: u8, _: u32) -> u16 {
        0
    }
    fn players_in_level(&self, _: u16) -> i32 {
        1
    }
    fn player_level_id(&self, _: UnitId) -> u16 {
        1
    }
    fn town_entered(&mut self, _: UnitId, _: u16) {}
    fn gold_cap(&self, _: UnitId) -> i32 {
        i32::MAX
    }
    fn stash_cap(&self, _: UnitId) -> i32 {
        i32::MAX
    }
    fn drop_gold(&mut self, _: UnitId, _: i32) {}
    fn last_bought(&self, player: UnitId) -> u32 {
        self.last_bought.get(&player).copied().unwrap_or(u32::MAX)
    }
    fn set_last_bought(&mut self, player: UnitId, guid: u32) {
        self.last_bought.insert(player, guid);
    }
    fn has_cursor_item(&self, _: UnitId) -> bool {
        false
    }
    fn create_item(&mut self, _: u16, _: usize, _: u8, _: i32) -> Option<UnitId> {
        None
    }
    fn copy_item(&mut self, _: UnitId) -> Option<UnitId> {
        None
    }
    fn destroy_item(&mut self, _: UnitId) {}
    fn item_record(&self, _: UnitId) -> usize {
        0
    }
    fn item_quality(&self, _: UnitId) -> u8 {
        2
    }
    fn item_file_index(&self, _: UnitId) -> i32 {
        0
    }
    fn item_flags(&self, item: UnitId) -> u32 {
        self.unit(item).flags
    }
    fn set_item_flags(&mut self, item: UnitId, flags: u32) {
        if let Some(f) = self.units.get_mut(&item) {
            f.flags = flags;
        }
    }
    fn or_unit_flags(&mut self, _: UnitId, _: u32) {}
    fn item_mode(&self, item: UnitId) -> u32 {
        u32::from(self.unit(item).mode)
    }
    fn set_item_mode(&mut self, _: UnitId, _: u32) {}
    fn set_item_page(&mut self, _: UnitId, _: u8) {}
    fn has_filled_sockets(&self, _: UnitId) -> bool {
        false
    }
    fn price_item(&self, _: UnitId) -> Option<PriceItem> {
        None
    }
    fn recharge(&mut self, _: UnitId) {}
    fn repair_broken(&mut self, _: UnitId) {}
    fn identify(&mut self, item: UnitId) {
        NpcWorld::identify(self, item);
    }
    fn send_item_stat(&mut self, _: UnitId, _: UnitId, _: u16) {}
    /// S→C 0x2A through the NPC module's transaction builder.
    fn send_transaction(&mut self, player: UnitId, t: Transaction) {
        let m = npc::transaction(t.kind, t.code, t.guid, t.gold as u32);
        self.sent.push((player, m.to_vec()));
    }
    fn new_store_inventory(&mut self, _: u16, _: Option<UnitId>) {}
    fn place_in_store(&mut self, _: u16, _: UnitId) -> bool {
        false
    }
    fn remove_store_item(&mut self, _: u16, _: UnitId) {}
    fn take_from_store(&mut self, _: u16, _: UnitId) {}
    fn place_in_gamble(&mut self, _: u16, _: u32, _: UnitId) -> bool {
        false
    }
    fn remove_gamble_item(&mut self, _: u16, _: u32, _: UnitId) {}
    fn refresh_npc_inventory(&mut self, _: UnitId) {}
    fn add_trade_inventory(&mut self, _: u16, _: UnitId) {}
    fn owns_item(&self, player: UnitId, item: UnitId) -> bool {
        self.unit(item).owner == Some(player)
    }
    fn in_inventory(&self, player: UnitId, item: UnitId) -> bool {
        VendorWorld::owns_item(self, player, item)
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

impl QuestWorld for Fake {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        0
    }
    fn difficulty(&self) -> u8 {
        0
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn game_type(&self) -> u8 {
        0
    }
    fn has_act2(&self) -> bool {
        false
    }
    fn players(&self) -> Vec<UnitId> {
        self.units
            .iter()
            .filter(|(_, f)| f.ty == 0)
            .map(|(&u, _)| u)
            .collect()
    }
    fn first_client_player(&self) -> Option<UnitId> {
        QuestWorld::players(self).first().copied()
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.unit(unit).guid
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.by_guid(0, guid)
    }
    fn quests(&mut self, player: UnitId) -> Option<&mut PlayerQuests> {
        self.quests.get_mut(&player)
    }
    fn unit_act(&self, unit: UnitId) -> Option<u8> {
        self.units.get(&unit).map(|f| f.act)
    }
    fn unit_level(&self, unit: UnitId) -> Option<u32> {
        self.units.get(&unit).map(|_| 1)
    }
    fn player_class(&self, _: UnitId) -> u8 {
        0
    }
    fn unit_seed(&mut self, _: UnitId) -> &mut Seed {
        &mut self.seed
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        NpcWorld::stat(self, unit, stat) as i32
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        NpcWorld::base_stat(self, unit, stat) as i32
    }
    fn add_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        let v = QuestWorld::stat(self, unit, stat).wrapping_add(delta);
        NpcWorld::set_stat(self, unit, stat, v as u32);
    }
    fn attach_sound(&mut self, player: UnitId, sound: u16) {
        NpcWorld::attach_sound(self, player, sound);
    }
    fn player_byte_4c(&self, player: UnitId) -> u8 {
        self.byte_4c.get(&player).copied().unwrap_or(0)
    }
    fn set_player_byte_4c(&mut self, player: UnitId, v: u8) {
        self.byte_4c.insert(player, v);
    }
    fn quest_chain(&mut self, unit: UnitId) -> Option<&mut QuestChain> {
        Some(self.chains.entry(unit).or_default())
    }
    fn unit_kind(&self, unit: UnitId) -> UnitKind {
        match self.units.get(&unit) {
            Some(f) if f.ty == 0 => UnitKind::Player,
            Some(f) if f.ty == 1 => UnitKind::Monster {
                class: u32::from(f.class),
                superunique: None,
                owner: None,
            },
            _ => UnitKind::Other,
        }
    }
    fn monster_by_guid(&self, guid: u32) -> Option<(UnitId, u16)> {
        let u = self.by_guid(1, guid)?;
        Some((u, self.unit(u).class))
    }
    fn monster_class(&self, unit: UnitId) -> Option<u16> {
        NpcWorld::monster_class(self, unit)
    }
    fn players_near(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn party_members(&self, _: UnitId) -> Option<Vec<UnitId>> {
        None
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        NpcWorld::send(self, player, msg);
    }
    /// The 0x27 list bytes are not modelled: zeros.
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, _: &[(u16, u32)]) {
        let mut m = vec![0x27, 1];
        m.extend_from_slice(&self.unit(npc).guid.to_le_bytes());
        m.extend_from_slice(&[0; 34]);
        self.sent.push((player, m));
    }
    fn has_item(&self, _: UnitId, _: [u8; 4]) -> bool {
        false
    }
    fn delete_item(&mut self, _: UnitId, _: [u8; 4]) {}
    fn reward_item(&mut self, _: UnitId, _: [u8; 4], _: i32, _: u8, _: bool) -> Option<UnitId> {
        None
    }
    fn drop_item_at(&mut self, _: UnitId, _: [u8; 4], _: u8) -> bool {
        false
    }
    fn quest_items(&self, _: UnitId) -> Vec<(UnitId, u8)> {
        Vec::new()
    }
    fn den_region(&self) -> (u32, u32, u32, u32) {
        (0, 0, 0, 0)
    }
    fn true_tomb_level(&self) -> u32 {
        66
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
    fn object_by_guid(&self, _: u32) -> Option<(UnitId, u16)> {
        None
    }
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
    fn item_code(&self, _: UnitId) -> Option<[u8; 4]> {
        None
    }
    fn unhandled(&mut self, chain: u8, function: u32) {
        self.log.push(format!("unhandled {chain} {function:#x}"));
    }
    fn spawn_monster_flags(
        &mut self,
        _: d2_sim::units::RoomId,
        _: i32,
        _: i32,
        _: u16,
        _: u8,
        _: i32,
        _: u32,
    ) -> Option<UnitId> {
        None
    }
    fn open_portal(
        &mut self,
        _: Option<UnitId>,
        _: d2_sim::units::RoomId,
        _: i32,
        _: i32,
        _: u32,
        _: u16,
        _: bool,
    ) -> Option<UnitId> {
        None
    }
    fn create_missile(
        &mut self,
        _: UnitId,
        _: u16,
        _: u8,
        _: u16,
        _: i32,
        _: i32,
    ) -> Option<UnitId> {
        None
    }
    fn set_missile_target(&mut self, _: UnitId, _: u32, _: u32) {}
    fn refresh_room(&mut self, _: UnitId) {}
    fn spawn_object(
        &mut self,
        _: d2_sim::units::RoomId,
        _: i32,
        _: i32,
        _: u16,
        _: i32,
    ) -> Option<UnitId> {
        None
    }
    fn client_save_flags(&self, _: UnitId) -> Option<u16> {
        None
    }
    fn set_client_save_flags(&mut self, _: UnitId, _: u16) {}
}

/// The world host of the tests: the real `NpcControl`, vendor records and
/// `QuestControl` with the fake seams.
pub struct FakeHost {
    /// The real NPC control block.
    pub npc: NpcControl,
    /// The vendor tables.
    pub tables: VendorTables,
    /// The vendor records.
    pub records: Vec<VendorRecord>,
    /// The fake seam provider.
    pub w: Fake,
    /// Faults the handlers reported.
    pub faults: Vec<WorldFault>,
    /// Faults a call met but could not return ([`WorldHost::take_faults`]).
    pub late: Vec<WorldError>,
}

/// Monstats with `npc` and `interact` set for the classes the tests use.
fn monstats() -> Vec<Monstats> {
    let mut rows: Vec<Monstats> = (0..=520)
        .map(|_| Monstats::decode(&vec![0u8; Monstats::SIZE]))
        .collect();
    for c in [
        npc::class::AKARA,
        npc::class::CHARSI,
        npc::class::CAIN5,
        npc::class::KASHYA,
    ] {
        rows[usize::from(c)].npc = true;
        rows[usize::from(c)].interact = true;
    }
    rows
}

impl Default for FakeHost {
    fn default() -> Self {
        let mut seed = Seed::init_low(0x1234);
        let npc = NpcControl::new(&monstats(), Vec::new(), false, 0, &mut seed).unwrap();
        let ctl = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
        let records = npc
            .records
            .iter()
            .map(|r| VendorRecord {
                class: r.class,
                act: r.act,
                ..VendorRecord::default()
            })
            .collect();
        Self {
            npc,
            tables: VendorTables::default(),
            records,
            w: Fake {
                ctl: Some(ctl),
                ..Fake::default()
            },
            faults: Vec::new(),
            late: Vec::new(),
        }
    }
}

/// No timer events in these tests.
#[derive(Default)]
pub struct NoEvents;

impl EventDispatch for NoEvents {
    fn run_event(&mut self, _: &mut Game, _: &TimerRun) {}
}

impl TickHooks for NoEvents {}

impl WorldHost<NoEvents> for FakeHost {
    fn npc<C: NpcCall>(&mut self, _: &mut Game, _: &mut NoEvents, call: C) -> Option<C::Out> {
        Some(call.call(&mut self.npc, &mut self.w))
    }
    fn vendors<C: VendorCall>(
        &mut self,
        _: &mut Game,
        _: &mut NoEvents,
        call: C,
    ) -> Option<C::Out> {
        Some(call.call(&self.tables, &mut self.records, &mut self.w))
    }
    fn quests<C: QuestCall>(&mut self, _: &mut Game, _: &mut NoEvents, call: C) -> Option<C::Out> {
        let mut ctl = self.w.ctl.take()?;
        let r = call.call(&mut ctl, &mut self.w);
        self.w.ctl = Some(ctl);
        Some(r)
    }
    fn take_sent(&mut self, _: &mut NoEvents) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.w.sent)
    }
    fn take_faults(&mut self) -> Vec<WorldError> {
        std::mem::take(&mut self.late)
    }
    fn fault(&mut self, fault: WorldFault) {
        self.faults.push(fault);
    }
}

/// The sim of the fake tests.
pub type FakeSim = SimGame<NoEvents, FakeHost>;

/// Fake unit id of Charsi (the fake ids are outside the game's unit lists).
pub const CHARSI_U: UnitId = UnitId(1006);
/// Fake unit id of Akara.
pub const AKARA_U: UnitId = UnitId(1016);
/// Fake unit id of Cain.
pub const CAIN_U: UnitId = UnitId(1020);
/// GUID of Charsi.
pub const CHARSI: u32 = 6;
/// GUID of Akara.
pub const AKARA: u32 = 0x10;
/// GUID of Cain.
pub const CAIN: u32 = 0x20;

/// A game with one player for client 0 at (100, 100) in act 0 with 500
/// gold and an empty quest record, Charsi (GUID 6) at distance 5, Akara
/// (GUID 0x10) and Cain (GUID 0x20) at distance 6, each with an
/// interaction list. The quest control has run the player's game entry.
pub fn sim() -> (FakeSim, UnitId) {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let room = game.lists.create_room(0).unwrap();
    game.lists.activate_room(room).unwrap();
    let player = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    let pguid = game.lists.unit(player).unwrap().guid;
    let mut s: FakeSim = SimGame::with_events(game, NoEvents);
    s.join(0, Some(player), Some(room), client_state::IN_GAME)
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
    let w = &mut s.world.w;
    w.add(
        player,
        FUnit {
            guid: pguid,
            ty: 0,
            x: 100,
            y: 100,
            ..FUnit::default()
        },
    );
    for (u, guid, class, x) in [
        (CHARSI_U, CHARSI, npc::class::CHARSI, 105),
        (AKARA_U, AKARA, npc::class::AKARA, 106),
        (CAIN_U, CAIN, npc::class::CAIN5, 94),
    ] {
        w.add(
            u,
            FUnit {
                guid,
                ty: 1,
                class,
                mode: 1,
                x,
                y: 100,
                ..FUnit::default()
            },
        );
        w.lists.insert(u, InteractionList::default());
    }
    w.quests.insert(player, PlayerQuests::default());
    NpcWorld::set_stat(w, player, npc::stat::GOLD, 500);
    let host = &mut s.world;
    let mut ctl = host.w.ctl.take().unwrap();
    ctl.player_enters(&mut host.w, player, 0).unwrap();
    host.w.ctl = Some(ctl);
    host.w.sent.clear();
    host.w.log.clear();
    (s, player)
}
