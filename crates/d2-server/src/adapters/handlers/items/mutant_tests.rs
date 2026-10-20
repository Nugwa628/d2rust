//! Mutation-testing additions for the unified item host: the vendor world
//! on the inventory model ([`InvVendors`]), the cube's world
//! ([`ServerCube`]) on the same model, and the wired host's vendor entry
//! point. Both adapters are
//! driven directly, over the wired host's own economy and inventory
//! parts (the `tests` fixture), so every call they answer or pass on is
//! observed: the model's answers, and the calls each
//! adapter hands unchanged to the world it wraps (the module contracts
//! of `vendor_inv.rs` and `cube_world.rs`), against a rest that logs
//! every call and answers by argument.

use std::collections::{BTreeMap, BTreeSet};

use d2_sim::items::inventory::grid::place_at_body;
use d2_sim::items::inventory::{Inventory, UnitKind as InvKind};
use d2_sim::items::tables::{PropSlot, PropertyRec};
use d2_sim::items::{flag, q, ItemGame};
use d2_sim::rng::Seed;
use d2_sim::units::hooks::MonsterInfo;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::ActionHooks;
use d2_sim::wiring::economy::{EconomyCube, GameFields, QuestRest};
use d2_sim::wiring::interaction::{Desk, HirelingRest, NpcRest, PlayerQuestsRef, VendorRest};
use d2_sim::world::cube::{CraftProperty, CubeWorld, StatRead};
use d2_sim::world::npc::{HireList, ImbueMods, InteractionList, InvEntry, ItemFacts, NpcRecord};
use d2_sim::world::quests::{PlayerQuests, QuestChain, TextList, UnitKind};
use d2_sim::world::vendors::price::Bonus;
use d2_sim::world::vendors::{NpcLink, Transaction, VendorWorld};

use super::super::cube_world::{InfoRest, ServerCube};
use super::*;
use crate::adapters::handlers::world::tests::trade_quests::ActionRest;
use crate::adapters::handlers::world::{VendorCall, WorldHost};

type Hooks = ActionHooks<ActionRest>;

// ---- a rest that logs and answers by argument ---------------------------------

fn odd(u: UnitId) -> bool {
    u.0 % 2 == 1
}

/// The interaction, quest and vendor seams: the quest tests' staged
/// answers (`trade_quests::Rest`), with every vendor call logged and
/// answered from its arguments (bools: an odd unit id).
#[derive(Default)]
pub struct Probe {
    pub quests: BTreeMap<UnitId, PlayerQuests>,
    pub guids: BTreeMap<UnitId, u32>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
    pub log: Vec<String>,
}

impl PlayerQuestsRef for Probe {
    fn quests_ref(&self, player: UnitId) -> Option<&PlayerQuests> {
        self.quests.get(&player)
    }
}

impl NpcRest for Probe {
    fn item_format(&self) -> u16 {
        101
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
    /// The monster spawn: none, so the quest mercenary
    /// stops after S→C 0x50.
    fn spawn_mercenary(&mut self, _: UnitId, class: u32, mode: u8) -> Option<UnitId> {
        self.log.push(format!("spawn merc {class} {mode}"));
        None
    }
}

impl HirelingRest for Probe {
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

impl QuestRest for Probe {
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
    /// S→C 0x27: type 1, the NPC's GUID, then 34 bytes (staged zero).
    /// The entries are logged.
    fn send_text_list(&mut self, player: UnitId, npc: UnitId, list: &[(u16, u32)]) {
        self.log.push(format!("text list {} {list:?}", npc.0));
        let mut m = vec![0x27, 1];
        m.extend_from_slice(&self.guids[&npc].to_le_bytes());
        m.extend_from_slice(&[0; 34]);
        self.sent.push((player, m));
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

impl VendorRest for Probe {
    fn players_in_level(&self, level: u16) -> i32 {
        i32::from(level) + 100
    }
    fn player_level_id(&self, _: UnitId) -> u16 {
        7
    }
    fn gold_cap(&self, _: UnitId) -> i32 {
        1234
    }
    fn stash_cap(&self, _: UnitId) -> i32 {
        4321
    }
    fn drop_gold(&mut self, p: UnitId, amount: i32) {
        self.log.push(format!("drop_gold {} {amount}", p.0));
    }
    fn last_bought(&self, _: UnitId) -> u32 {
        0xBEEF
    }
    fn set_last_bought(&mut self, p: UnitId, guid: u32) {
        self.log.push(format!("set_last_bought {} {guid}", p.0));
    }
    fn has_cursor_item(&self, _: UnitId) -> bool {
        unreachable!("the model answers has_cursor_item")
    }
    fn copy_item(&mut self, item: UnitId) -> Option<UnitId> {
        Some(UnitId(item.0 + 1000))
    }
    fn has_filled_sockets(&self, item: UnitId) -> bool {
        odd(item)
    }
    fn socketed(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn price_bonuses(&self, _: UnitId) -> Vec<Bonus> {
        Vec::new()
    }
    fn recharge(&mut self, item: UnitId) {
        self.log.push(format!("recharge {}", item.0));
    }
    fn repair_broken(&mut self, item: UnitId) {
        self.log.push(format!("repair_broken {}", item.0));
    }
    fn send_transaction(&mut self, p: UnitId, t: Transaction) {
        self.log.push(format!("send_transaction {} {t:?}", p.0));
    }
    fn new_store_inventory(&mut self, c: u16, npc: Option<UnitId>) {
        self.log.push(format!("new_store_inventory {c} {npc:?}"));
    }
    fn place_in_store(&mut self, c: u16, item: UnitId) -> bool {
        self.log.push(format!("place_in_store {c} {}", item.0));
        odd(item)
    }
    fn remove_store_item(&mut self, c: u16, item: UnitId) {
        self.log.push(format!("remove_store_item {c} {}", item.0));
    }
    fn take_from_store(&mut self, c: u16, item: UnitId) {
        self.log.push(format!("take_from_store {c} {}", item.0));
    }
    fn place_in_gamble(&mut self, c: u16, p: u32, item: UnitId) -> bool {
        self.log.push(format!("place_in_gamble {c} {p} {}", item.0));
        odd(item)
    }
    fn remove_gamble_item(&mut self, c: u16, p: u32, item: UnitId) {
        self.log
            .push(format!("remove_gamble_item {c} {p} {}", item.0));
    }
    fn refresh_npc_inventory(&mut self, npc: UnitId) {
        self.log.push(format!("refresh_npc_inventory {}", npc.0));
    }
    fn add_trade_inventory(&mut self, c: u16, item: UnitId) {
        self.log.push(format!("add_trade_inventory {c} {}", item.0));
    }
    fn owns_item(&self, _: UnitId, _: UnitId) -> bool {
        unreachable!("the model answers owns_item")
    }
    fn in_inventory(&self, _: UnitId, _: UnitId) -> bool {
        unreachable!("the model answers in_inventory")
    }
    fn equipped_items(&self, _: UnitId) -> Vec<UnitId> {
        unreachable!("the model answers equipped_items")
    }
    fn find_tome(&mut self, _: UnitId, scroll: UnitId) -> Option<(UnitId, i32)> {
        Some((UnitId(scroll.0 + 1), 3))
    }
    fn add_to_tome(&mut self, tome: UnitId, k: i32) {
        self.log.push(format!("add_to_tome {} {k}", tome.0));
    }
    fn find_partial_stack(&mut self, _: UnitId, item: UnitId) -> Option<(UnitId, i32)> {
        Some((UnitId(item.0 + 2), 4))
    }
    fn can_belt(&mut self, _: UnitId, item: UnitId) -> bool {
        odd(item)
    }
    fn put_in_belt(&mut self, p: UnitId, item: UnitId) -> bool {
        self.log.push(format!("put_in_belt {} {}", p.0, item.0));
        odd(item)
    }
    fn equip_ammo(&mut self, p: UnitId, item: UnitId) -> bool {
        self.log.push(format!("equip_ammo {} {}", p.0, item.0));
        odd(item)
    }
    fn place_in_backpack(&mut self, _: UnitId, _: UnitId) -> bool {
        unreachable!("the model answers place_in_backpack")
    }
    fn take_from_cursor(&mut self, p: UnitId, item: UnitId) -> bool {
        self.log
            .push(format!("take_from_cursor {} {}", p.0, item.0));
        odd(item)
    }
    fn lower_book_skill(&mut self, p: UnitId, item: UnitId, n: i32) {
        self.log
            .push(format!("lower_book_skill {} {} {n}", p.0, item.0));
    }
    fn remove_stored(&mut self, _: UnitId, _: UnitId) {
        unreachable!("the model answers remove_stored")
    }
    fn unequip(&mut self, p: UnitId, item: UnitId) -> bool {
        self.log.push(format!("unequip {} {}", p.0, item.0));
        odd(item)
    }
}

// ---- harnesses ------------------------------------------------------------------

/// The wired host's vendor world on `probe`: the interaction desk over
/// the host's economy (game fields edited by `fields` first), the NPC
/// control block, and the inventory model, wrapped as the host wraps it.
fn with_vendors<O>(
    t: &mut T,
    probe: &mut Probe,
    fields: impl FnOnce(&mut GameFields),
    f: impl FnOnce(&mut InvVendors<'_, '_, '_, '_, Hooks, Probe>) -> O,
) -> O {
    let sim = &mut t.host.game;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    world.with_economy(game, events, |econ, p| {
        fields(econ.fields);
        let mut desk = Desk {
            econ,
            quests: &mut *p.quests,
            vendor_tables: p.vendor_tables,
            state: &mut *p.state,
            rest: probe,
            now: p.now,
            inv: None,
        };
        let inner = desk.vendors(Some(&mut *p.npc));
        let mut w = InvVendors::new(inner, p.inventory.as_deref_mut());
        f(&mut w)
    })
}

/// The cube's world of the host for `t.player`, as `CubeRun` builds it
/// (game fields edited by `fields` first); `f`'s result, then what the
/// call sent and its errors (`ServerCube::finish`).
fn with_cube<O>(
    t: &mut T,
    fields: impl FnOnce(&mut GameFields),
    f: impl FnOnce(&mut ServerCube<'_, '_, '_, Hooks>) -> O,
) -> (O, Vec<Vec<u8>>, Vec<ItemError>) {
    let player = t.player;
    let sim = &mut t.host.game;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    world.with_economy(game, events, |econ, p| {
        fields(econ.fields);
        let CubeParts {
            staged,
            creation,
            pending,
            ..
        } = p.cube.as_deref_mut().unwrap();
        let mut info = InfoRest(creation);
        let mut w = ServerCube::new(
            EconomyCube::new(econ, &mut info),
            staged,
            pending.as_mut(),
            p.inventory.as_deref_mut(),
            player,
        );
        let out = f(&mut w);
        let (sent, errors) = w.finish();
        (out, sent, errors)
    })
}

/// Runs `f` with `owner`'s inventory taken out of the model and a desk
/// over the host's economy (the inventory routines' own world).
fn on_inventory<O>(
    t: &mut T,
    owner: UnitId,
    f: impl FnOnce(
        &mut Inventory,
        &mut d2_sim::wiring::inventory::InvDesk<'_, '_, Hooks, dyn moves::MoveRest + Send + Sync>,
    ) -> O,
) -> O {
    let sim = &mut t.host.game;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    world.with_economy(game, events, |econ, p| {
        let parts = p.inventory.as_deref_mut().unwrap();
        let mut inv = parts.state.inventories.remove(&owner).unwrap();
        let out = {
            let mut d = parts.desk(econ);
            f(&mut inv, &mut d)
        };
        parts.state.inventories.insert(owner, inv);
        out
    })
}

/// A unit of type `ty` and class `class` on the action sim.
fn alloc(t: &mut T, ty: UnitType, class: u32) -> UnitId {
    let req = AllocRequest {
        ty,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let sim = &mut t.host.game;
    if ty == UnitType::Monster {
        let m = &mut sim.events.sys.data.monsters;
        m.resize(m.len().max(class as usize + 1), MonsterInfo::default());
        m[class as usize].enabled = true;
    }
    sim.events
        .with(&mut sim.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap()
}

// ---- InvVendors: the model's answers ----------------------------------------------

/// Player-inventory calls on the model: the cursor
/// item, ownership (the item list or the cursor), the body items
/// (grid 0), placement (a cursor item is
/// placed, a ground item refused) and the stored item's removal (unlink
/// and free). Without inventory parts:
/// nothing held, placement fails.
#[test]
fn inv_vendors_answer_from_the_model() {
    let mut t = setup(4);
    let (player, cube, ring) = (t.player, t.cube, t.ring);
    let other = alloc(&mut t, UnitType::Item, RING as u32);
    let mut probe = Probe::default();

    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(!w.has_cursor_item(player));
            assert!(w.owns_item(player, cube));
            assert!(w.in_inventory(player, cube));
            assert!(!w.owns_item(player, ring));
            assert!(!w.in_inventory(player, ring));
            assert!(!w.owns_item(player, other));
            assert!(w.equipped_items(player).is_empty());
        },
    );
    t.inventory().set_cursor(Some(ring));
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(w.has_cursor_item(player));
            assert!(w.owns_item(player, ring));
            assert!(w.in_inventory(player, ring));
            // The cursor ring is placed on its page (0), mode 0.
            assert!(w.place_in_backpack(player, ring));
            assert!(!w.has_cursor_item(player));
            assert!(w.sent.is_empty());
        },
    );
    assert_eq!(t.inventory().items(), [cube, ring]);
    assert_eq!(t.mode(ring), 0);

    // Grid 0: the ring moved to body location 4.
    assert!(on_inventory(&mut t, player, |inv, d| {
        inv.unlink(d, ring) && place_at_body(inv, d, ring, 4)
    }));
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert_eq!(w.equipped_items(player), [ring]);
            // The stored cube is removed and freed.
            w.remove_stored(player, cube);
        },
    );
    assert_eq!(t.inventory().items(), [ring]);
    assert!(!t.items().contains(cube));
    assert!(t.host.game.game.lists.unit(cube).is_none());

    // A ground item is refused by the placement.
    let g = item(&mut t.host.game, AMULET, 3);
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(!w.place_in_backpack(player, g));
        },
    );
    // No inventory parts: the empty inventory.
    t.world().inventory = None;
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(!w.owns_item(player, ring));
            assert!(!w.in_inventory(player, ring));
            assert!(w.equipped_items(player).is_empty());
            assert!(!w.has_cursor_item(player));
            assert!(!w.place_in_backpack(player, g));
            w.remove_stored(player, ring);
        },
    );
    assert!(t.items().contains(ring));
    assert!(probe.log.is_empty(), "{:?}", probe.log);
}

/// Sell mode 4 on the model: only the player's cursor item is taken;
/// it leaves the cursor and is freed, and S→C 0x42 names the player
/// whose cursor cleared. Without inventory
/// parts the wrapped world answers.
#[test]
fn inv_vendors_take_the_cursor_item_on_the_model() {
    let mut t = setup(4);
    let (player, cube, ring) = (t.player, t.cube, t.ring);
    let pg = t.guid(player);
    let mut probe = Probe::default();
    t.inventory().set_cursor(Some(ring));
    let sent = with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(!w.take_from_cursor(player, cube), "not the cursor item");
            assert!(w.take_from_cursor(player, ring));
            assert!(!w.has_cursor_item(player));
            w.sent.clone()
        },
    );
    let mut clear = vec![0x42, 0];
    clear.extend_from_slice(&pg.to_le_bytes());
    assert_eq!(sent, [(player, clear)]);
    assert!(!t.items().contains(ring), "the sold item is freed");
    assert!(t.host.game.game.lists.unit(ring).is_none());
    assert_eq!(t.inventory().items(), [cube]);
    assert!(probe.log.is_empty(), "{:?}", probe.log);
    // No inventory parts: the wrapped world's answer.
    t.world().inventory = None;
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(w.take_from_cursor(player, UnitId(1)));
            assert!(!w.take_from_cursor(player, UnitId(2)));
        },
    );
    let p = player.0;
    assert_eq!(
        probe.log,
        [
            format!("take_from_cursor {p} 1"),
            format!("take_from_cursor {p} 2")
        ]
    );
}

// ---- InvVendors: every other call reaches the wrapped world -----------------------

/// `vendor_inv.rs`: "every other call goes to the wrapped world
/// unchanged". Each `VendorWorld` / `NpcLink` call not on the model
/// answers what the wrapped `VendorDesk` answers (the economy's fields,
/// unit records, stats and items, the NPC control block, the rest), and
/// each write lands where the desk writes it.
#[test]
fn inv_vendors_pass_other_calls_through() {
    let mut t = setup(4);
    let (player, ring) = (t.player, t.ring);
    let npc = alloc(&mut t, UnitType::Monster, 150);
    let ctl = &mut t.world().npc;
    for (class, made, hire) in [
        (150, true, None),
        (151, false, None),
        (152, false, Some(HireList::default())),
    ] {
        ctl.records.push(NpcRecord {
            class,
            hire_made: made,
            hire,
            ..NpcRecord::default()
        });
    }
    t.world().state.lists.insert(
        npc,
        InteractionList {
            nodes: vec![(player, 1)],
        },
    );
    {
        let it = t.items().get_mut(ring).unwrap();
        it.file_index = 5;
        it.flags = flag::IDENTIFIED | 0x4;
    }
    let (pg, ng) = (t.guid(player), t.guid(npc));
    let mut probe = Probe::default();
    {
        let r = t.units().get_mut(player).unwrap();
        r.interact.reset();
        r.interact.set(1, ng);
    }
    let mut quests = PlayerQuests::default();
    quests.flags[1].0[4..6].copy_from_slice(&0x1234u16.to_le_bytes());
    quests.intro[2] = (0..1000).collect::<BTreeSet<u16>>();
    probe.quests.insert(player, quests);
    let (one, two) = (UnitId(1), UnitId(2));

    let made = with_vendors(
        &mut t,
        &mut probe,
        |f| {
            f.difficulty = 2;
            f.game_type = 3;
            f.expansion = false;
        },
        |w| {
            // Game.
            assert_eq!(w.difficulty(), 2);
            assert_eq!(w.game_type(), 3);
            assert!(!w.expansion());
            assert_eq!(w.item_format(), 101);
            // NPCs.
            assert_eq!(w.npc_by_guid(ng), Some(npc));
            assert_eq!(w.npc_by_guid(ng + 100), None);
            assert_eq!(w.npc_class(npc), 150);
            assert_eq!(w.npc_class(player), CLASS as u16);
            assert!(w.is_interact_unit(player, npc));
            assert!(!w.is_interact_unit(player, ring));
            assert!(!w.interaction_empty(npc));
            assert!(w.interaction_empty(ring));
            assert!(w.hire_list_made(150));
            assert!(!w.hire_list_made(151));
            w.set_hire_list_made(151);
            assert!(w.hire_list_made(151));
            let mut seed = Seed::init_low(1);
            w.make_hire_list(152, &mut seed);
            assert!(w.hire_list_made(152));
            // Units and stats.
            for u in [player, ring, npc] {
                assert_eq!(w.guid(u), w.inner.guid(u));
            }
            assert_eq!(w.player_by_guid(pg), Some(player));
            let rg = w.guid(ring);
            assert_eq!(w.item_by_guid(rg), Some(ring));
            w.set_stat(player, 12, 0, 37);
            assert_eq!(w.stat(player, 12, 0), 37);
            assert_eq!(w.base_stat(player, 12, 0), 37);
            assert_eq!(w.quest_slot(player, 1, 2), 0x1234);
            assert_eq!(w.players_in_level(5), 105);
            assert_eq!(w.player_level_id(player), 7);
            // The caps are the desk's (gold: level × 10000, stash:
            // a constant), not the rest's.
            let level = w.stat(player, 12, 0);
            assert_eq!(w.gold_cap(player), level * 10_000);
            assert_eq!(w.stash_cap(player), 2_500_000);
            assert_eq!(w.last_bought(player), 0xBEEF);
            w.drop_gold(player, 9);
            w.set_last_bought(player, 77);
            w.town_entered(player, 1);
            // Items.
            let made = w.create_item(0, AMULET, q::NORMAL, 5).expect("created");
            assert_eq!(w.item_record(made), AMULET);
            // The copy is the inventory model's (`InvDesk::copy_of`),
            // not the rest's.
            let copy = w.copy_item(ring).expect("copied");
            assert_ne!(copy, UnitId(ring.0 + 1000));
            assert_eq!(w.item_record(copy), w.item_record(ring));
            assert_eq!(w.item_quality(ring), q::NORMAL);
            assert_eq!(w.item_file_index(ring), 5);
            // The copied source carries 0x8000000.
            assert_eq!(w.item_flags(ring), flag::IDENTIFIED | 0x4 | 0x800_0000);
            w.set_item_flags(ring, 0x30);
            assert_eq!(w.item_flags(ring), 0x30);
            w.or_unit_flags(ring, 0x40);
            assert_eq!(w.item_mode(ring), 4);
            w.set_item_mode(ring, 3);
            assert_eq!(w.item_mode(ring), 3);
            w.set_item_page(ring, 5);
            w.destroy_item(made);
            // The rest's item and store calls.
            w.recharge(one);
            w.repair_broken(one);
            w.identify(one);
            // S→C 0x3E is built from the item's base stat (no rest call).
            w.set_stat(ring, 70, 0, 300);
            w.send_item_stat(player, ring, 70);
            let tr = Transaction {
                kind: 1,
                code: 2,
                guid: 3,
                gold: 4,
            };
            w.send_transaction(player, tr);
            w.new_store_inventory(5, Some(one));
            assert!(w.place_in_store(5, one));
            assert!(!w.place_in_store(5, two));
            w.remove_store_item(5, one);
            w.take_from_store(5, one);
            assert!(w.place_in_gamble(5, 6, one));
            assert!(!w.place_in_gamble(5, 6, two));
            w.remove_gamble_item(5, 6, one);
            w.refresh_npc_inventory(one);
            w.add_trade_inventory(5, one);
            made
        },
    );
    with_vendors(&mut t, &mut probe, |_| {}, |w| assert!(w.expansion()));

    assert_eq!(
        t.host.game.events.sys.units.get(ring).unwrap().flags2 & 0x40,
        0x40
    );
    assert_eq!(t.page(ring), 5);
    assert!(!t.items().contains(made));
    assert!(t.world().state.errors.is_empty());
    // The town of act I (level 1): the act's intro
    // NPCs heard at the game's difficulty (2) in one 0x91.
    assert_eq!(probe.sent.len(), 2);
    assert_eq!(probe.sent[0].0, player);
    assert_eq!(&probe.sent[0].1[..2], [0x91, 0]);
    let rg = t.units().get(ring).unwrap().guid;
    assert_eq!(
        probe.sent[1],
        (
            player,
            d2_sim::units::messages::update_item_stat(rg, 70, 300, 0)
        )
    );
    let p = player.0;
    assert_eq!(
        probe.log,
        [
            format!("drop_gold {p} 9"),
            format!("set_last_bought {p} 77"),
            "recharge 1".into(),
            "repair_broken 1".into(),
            "identify 1".into(),
            format!(
                "send_transaction {p} {tr:?}",
                tr = Transaction {
                    kind: 1,
                    code: 2,
                    guid: 3,
                    gold: 4
                }
            ),
            "new_store_inventory 5 Some(UnitId(1))".into(),
            "place_in_store 5 1".into(),
            "place_in_store 5 2".into(),
            "remove_store_item 5 1".into(),
            "take_from_store 5 1".into(),
            "place_in_gamble 5 6 1".into(),
            "place_in_gamble 5 6 2".into(),
            "remove_gamble_item 5 6 1".into(),
            "refresh_npc_inventory 1".into(),
            "add_trade_inventory 5 1".into(),
        ]
    );
    // The player-inventory calls the model answers reach the wrapped
    // world only without inventory parts.
    probe.log.clear();
    t.world().inventory = None;
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(w.has_filled_sockets(one));
            assert!(!w.has_filled_sockets(two));
            let price = w.price_item(ring);
            assert!(price.is_some());
            assert_ne!(price, Some(Default::default()));
            assert_eq!(price, w.inner.price_item(ring));
            assert_eq!(w.find_tome(player, one), Some((two, 3)));
            w.add_to_tome(one, 2);
            assert_eq!(w.find_partial_stack(player, one), Some((UnitId(3), 4)));
            assert!(w.can_belt(player, one));
            assert!(!w.can_belt(player, two));
            assert!(w.put_in_belt(player, one));
            assert!(!w.put_in_belt(player, two));
            assert!(w.equip_ammo(player, one));
            assert!(!w.equip_ammo(player, two));
            w.lower_book_skill(player, one, 2);
            assert!(w.unequip(player, one));
            assert!(!w.unequip(player, two));
        },
    );
    assert_eq!(
        probe.log,
        [
            "add_to_tome 1 2".to_string(),
            format!("put_in_belt {p} 1"),
            format!("put_in_belt {p} 2"),
            format!("equip_ammo {p} 1"),
            format!("equip_ammo {p} 2"),
            format!("lower_book_skill {p} 1 2"),
            format!("unequip {p} 1"),
            format!("unequip {p} 2"),
        ]
    );
}

/// The wired host's vendor entry point runs the call on its vendor world
/// (`InvVendors` over the host's inventory model): a cursor item placed
/// by the auto-place is in the model.
#[test]
fn wired_vendors_run_on_the_model() {
    struct Place(UnitId, UnitId);
    impl VendorCall for Place {
        type Out = bool;
        fn call<W: VendorWorld>(
            self,
            _: &d2_sim::world::vendors::VendorTables,
            _: &mut [d2_sim::world::vendors::VendorRecord],
            w: &mut W,
        ) -> bool {
            w.has_cursor_item(self.0) && w.place_in_backpack(self.0, self.1)
        }
    }
    let mut t = setup(4);
    let (player, cube, ring) = (t.player, t.cube, t.ring);
    t.inventory().set_cursor(Some(ring));
    let sim = &mut t.host.game;
    let out = WorldHost::vendors(
        &mut sim.world,
        &mut sim.game,
        &mut sim.events,
        Place(player, ring),
    );
    assert_eq!(out, Some(true));
    assert_eq!(t.inventory().items(), [cube, ring]);
    assert_eq!(t.inventory().cursor(), None);
}

// ---- ServerCube --------------------------------------------------------------------

/// The cube's world answers its game, unit, stat and item calls from the
/// host's economy (`cube_world.rs`: "the economy wiring's
/// `EconomyCube` for items, stats, unit records and creation"), and
/// each write lands there.
#[test]
fn server_cube_answers_from_the_economy() {
    let mut t = setup(4);
    let (player, cube, ring) = (t.player, t.cube, t.ring);
    {
        let tb = &mut t.world().tables;
        tb.items[CUBE].gemsockets = 4;
        tb.items[CUBE].invwidth = 2;
        tb.items[CUBE].invheight = 2;
        tb.itemtypes[usize::from(T_BOX)].maxsock1 = 3;
    }
    {
        let it = t.items().get_mut(ring).unwrap();
        it.file_index = 5;
        it.flags = flag::IDENTIFIED | 0x4;
    }
    let guids = [t.guid(player), t.guid(cube), t.guid(ring)];
    let ((), _, errors) = with_cube(
        &mut t,
        |f| {
            f.difficulty = 2;
            f.game_type = 3;
            f.ladder = true;
            f.expansion = false;
        },
        |w| {
            assert_eq!(w.difficulty(), 2);
            assert_eq!(w.game_type(), 3);
            assert!(w.ladder());
            assert!(!w.expansion());
            *w.game_seed() = Seed::init_low(77);
            assert_eq!(w.player_class(player), CLASS as u8);
            w.set_stat(player, 12, 37);
            assert_eq!(w.stat(player, StatRead::Value, 12), 37);
            assert_eq!(w.stat(player, StatRead::Base, 12), 37);
            for (u, g) in [player, cube, ring].into_iter().zip(guids) {
                assert_eq!(w.item_guid(u), g);
            }
            assert_eq!(w.item_class(ring), Some(RING as u32));
            assert!(w.class_is_type(AMULET as u32, T_AMULET));
            assert!(!w.class_is_type(RING as u32, T_AMULET));
            w.set_item_class(ring, AMULET as u32);
            assert_eq!(w.item_class(ring), Some(AMULET as u32));
            assert_eq!(w.item_quality(ring), q::NORMAL);
            assert_eq!(w.item_file_index(ring), 5);
            assert_eq!(w.item_level(ring), 5);
            w.set_item_level(ring, 9);
            assert_eq!(w.item_level(ring), 9);
            assert_eq!(w.item_flags(ring), flag::IDENTIFIED | 0x4);
            w.set_item_mode(ring, 3);
            assert_eq!(w.item_mode(ring), 3);
            // min(gemsockets 4, maxsock1 3) at
            // ilvl 5; the add sets 2 of them.
            assert_eq!(w.max_sockets(cube), 3);
            w.add_sockets(cube, 2);
            assert_eq!(w.item_sockets(cube), 2);
            *w.item_seed(ring) = Seed::init_low(9);
            w.set_tempered(ring, 5, 6);
            assert!(!w.unique_found(7));
            w.set_unique_found(7, true);
            assert!(w.unique_found(7));
            assert!(!w.unique_found(8));
        },
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(t.host.game.events.sys.hooks.game_seed, Seed::init_low(77));
    assert!(t.host.game.events.sys.hooks.uniques.get(7));
    let rec = t.host.game.events.sys.units.get(ring).unwrap();
    assert_eq!((rec.mode, rec.class), (3, AMULET as u32));
    assert_eq!(rec.seed, Seed::init_low(9));
    let it = t.items().get(ring).unwrap().clone();
    assert_eq!(
        (it.quality, it.rare_prefix, it.rare_suffix),
        (q::TEMPERED, 5, 6)
    );
    assert_eq!(
        t.items().get(cube).unwrap().flags & flag::SOCKETED,
        flag::SOCKETED
    );

    // Expansion on, ladder off; the freed ring leaves the store.
    let ((), _, errors) = with_cube(
        &mut t,
        |_| {},
        |w| {
            assert!(w.expansion());
            assert!(!w.ladder());
            w.free_item(ring);
        },
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!t.items().contains(ring));
}

/// Trading is interaction type 0 with a live player;
/// the stash is interaction type 2 with object class
/// 0x10B.
#[test]
fn server_cube_trading_and_stash() {
    let mut t = setup(4);
    let (player, ring) = (t.player, t.ring);
    let stash = alloc(&mut t, UnitType::Object, 0x10B);
    let chest = alloc(&mut t, UnitType::Object, 0x10C);
    let (pg, rg, sg, cg) = (t.guid(player), t.guid(ring), t.guid(stash), t.guid(chest));
    let cases = [
        ((0, pg), true, false),
        ((1, pg), false, false),
        ((0, rg), false, false),
        ((2, sg), false, true),
        ((2, cg), false, false),
        ((3, sg), false, false),
    ];
    for (active, trading, stash) in cases {
        {
            let r = t.units().get_mut(player).unwrap();
            r.interact.reset();
            r.interact.set(active.0, active.1);
        }
        let ((tr, st), _, _) = with_cube(
            &mut t,
            |_| {},
            |w| (w.trading(player), w.interacting_with_stash(player)),
        );
        assert_eq!((tr, st), (trading, stash), "{active:?}");
    }
}

/// The socket fillers are the item's own inventory, read through the
/// model.
#[test]
fn server_cube_socketed_reads_the_model() {
    let mut t = setup(0);
    let (player, cube, ring) = (t.player, t.cube, t.ring);
    let cg = t.guid(cube);
    let mut inv = Inventory::new(cube, InvKind::Item, cg);
    on_inventory(&mut t, player, |_, d| inv.link(d, ring, None));
    t.world()
        .inventory
        .as_mut()
        .unwrap()
        .state
        .inventories
        .insert(cube, inv);
    let (fillers, _, _) = with_cube(&mut t, |_| {}, |w| w.socketed(cube));
    assert_eq!(fillers, [ring]);
}

/// The cube's direct 0x9D on another player's item goes to
/// the item's owner, not the acting player, so it is not sent on the
/// acting client; it is an error (`CubeWorld::send`'s rule for every
/// cube message).
#[test]
fn server_cube_message_to_another_player_is_an_error() {
    let mut t = setup(0);
    let other = alloc(&mut t, UnitType::Player, CLASS);
    let og = t.guid(other);
    t.world().inventory.as_mut().unwrap().state.add_inventory(
        other,
        InvKind::Player { class: CLASS as u8 },
        og,
    );
    let amu = item(&mut t.host.game, AMULET, 4);
    t.items().get_mut(amu).unwrap().inv_page = 0;
    store(&mut t.host.game, other, amu, 0);
    let ((), sent, errors) = with_cube(&mut t, |_| {}, |w| w.remove_cube_item(other, amu));
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(errors, [ItemError::OtherPlayer(other)]);
    assert!(!t.items().contains(amu));
}

/// The cube's item routines on the economy: the item format is the game
/// fields', item init returns the
/// item, and a craft property (`min` added to the slot's stat) lands
/// on the item's stats.
#[test]
fn server_cube_item_routines_reach_the_economy() {
    let mut t = setup(4);
    let ring = t.ring;
    let mut slots = [PropSlot::default(); 7];
    slots[0] = PropSlot {
        func: 15,
        stat: 12,
        set: 0,
        val: 0,
    };
    t.world().tables.properties = vec![PropertyRec { slots }];
    let mut format = 0;
    let ((fmt, init, value), _, errors) = with_cube(
        &mut t,
        |f| format = ItemGame::item_format(&*f),
        |w| {
            let fmt = w.item_format();
            let init = w.item_init(ring);
            let prop = CraftProperty {
                property: 0,
                param: 0,
                min: 7,
                max: 7,
            };
            w.add_craft_property(ring, &prop);
            (fmt, init, w.stat(ring, StatRead::Value, 12))
        },
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_ne!(format, 1);
    assert_eq!(fmt, format);
    assert_eq!(init, Some(ring));
    assert_eq!(value, 7);
}

// ---- InvVendors: the player-inventory seams on the model ---------------------------

/// A page-0 tome of the scroll's
/// spell below its max stack (20) is found with its free space;
/// `add_to_tome` adds k to it.
#[test]
fn inv_vendors_find_a_tome_and_add_to_it() {
    let mut t = setup(4);
    let player = t.player;
    let tome = item(&mut t.host.game, TOME, 4);
    t.store(tome, 0);
    t.set_stat(tome, 70, 5);
    let scroll = item(&mut t.host.game, SCROLL, 4);
    let mut probe = Probe::default();
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert_eq!(w.find_tome(player, scroll), Some((tome, 15)));
            w.add_to_tome(tome, 3);
        },
    );
    assert_eq!(t.stat(tome, 70), 8);
    // A full tome is not found.
    t.set_stat(tome, 70, 20);
    with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| assert_eq!(w.find_tome(player, scroll), None),
    );
}

/// The partial-stack search: the equipped stack first,
/// then page 0; a full stack is skipped. The auto-belt gate is true
/// for every item.
#[test]
fn inv_vendors_find_the_equipped_partial_stack_first() {
    let mut t = setup(4);
    let player = t.player;
    let worn = item(&mut t.host.game, ARROWS, 4);
    let packed = item(&mut t.host.game, ARROWS, 4);
    t.store(packed, 0);
    assert!(on_inventory(&mut t, player, |inv, d| place_at_body(
        inv, d, worn, 5
    )));
    t.set_stat(worn, 70, 350);
    t.set_stat(packed, 70, 100);
    let bought = item(&mut t.host.game, ARROWS, 4);
    let mut probe = Probe::default();
    let found = with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| {
            assert!(w.can_belt(player, bought));
            w.find_partial_stack(player, bought)
        },
    );
    assert_eq!(found, Some((worn, 150)));
    t.set_stat(worn, 70, 500);
    let found = with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| w.find_partial_stack(player, bought),
    );
    assert_eq!(found, Some((packed, 400)));
}

/// The equip try: no weapon in use and `aqv` → no try; a bow in
/// use → arrows are not a weapon and match the bow's hand class, so the
/// try runs; a weapon copy with a weapon in use → no try.
#[test]
fn inv_vendors_equip_try_follows_the_weapon_table() {
    let mut t = setup(4);
    let player = t.player;
    let arrows = item(&mut t.host.game, ARROWS, 4);
    let mut probe = Probe::default();
    assert!(!with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| w.equip_ammo(player, arrows)
    ));
    assert_eq!(t.mode(arrows), 4, "no try: the copy stays as it was");
    // A bow in the right hand, the weapon in use.
    let bow = item(&mut t.host.game, BOW, 4);
    assert!(on_inventory(&mut t, player, |inv, d| {
        let ok = place_at_body(inv, d, bow, 4);
        inv.weapon_guid = d.guid_of(bow);
        ok
    }));
    let other = item(&mut t.host.game, BOW, 4);
    assert!(!with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| w.equip_ammo(player, other)
    ));
    assert_eq!(t.mode(other), 4);
    // Arrows with the bow: tried, and the equip rules put them in
    // the free left hand (requirements: stats ≥ 1, identified).
    for s in [0, 2, 12] {
        t.set_stat(player, s, 1);
    }
    t.items().get_mut(arrows).unwrap().flags |= flag::IDENTIFIED;
    assert!(with_vendors(
        &mut t,
        &mut probe,
        |_| {},
        |w| w.equip_ammo(player, arrows)
    ));
    assert_eq!(t.mode(arrows), 1);
}
