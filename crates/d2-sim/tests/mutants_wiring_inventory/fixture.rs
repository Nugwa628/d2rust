// Spec: specs/items/inventory.md (fixture of the inventory wiring's mutation tests)
// Spec: specs/items/inventory-moves.md (§6–§11, split out of `inventory.md`)
//! The inventory wiring's test world through the public API: synthetic
//! tables (the shape of `wiring/inventory/tests/mod.rs`: a barbarian,
//! inventory record 4, 10 × 4, in one room of act 0, fixed game seed),
//! the real unit allocator, unit lists, stat lists and item store. Only
//! the seams without a d2-sim provider are a fake ([`Rest`]): every call
//! is logged with its arguments and every answer comes from
//! [`Answers`].

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::{EquivMatrix, StateMaps};
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Record, States};
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{InvItem, InvTables, Inventory, UnitKind};
use d2_sim::items::moves::{self, Guid, MovePending, Owner, Spot};
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{q, ItemRequest, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::stats::{ClassStats, StatData, StatHost, StatLists, StatTable, StateTable};
use d2_sim::units::hooks::{Sim, UnitData, UnitHooks};
use d2_sim::units::lifecycle::{allocate, AllocRequest, LifecycleHooks};
use d2_sim::units::record::Units;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::economy::{Economy, GameFields, ItemSpawn, ItemStore};
use d2_sim::wiring::inventory::{InvDesk, InvRest, InvState};

const N_STATS: usize = 359;
pub const N_TYPES: usize = 80;

/// Item records (combined index; both table projections use this order).
pub const CAP: usize = 0;
pub const GOLD: usize = 1;
pub const SWORD: usize = 2;
pub const TWO_HANDER: usize = 3;
pub const SHIELD: usize = 4;
pub const HP1: usize = 5;
pub const KEY: usize = 6;
pub const KNIFE: usize = 7;
pub const NODUR_KEY: usize = 8;
pub const HEAVY_CAP: usize = 9;

pub const T_SHIE: u16 = 2;
pub const T_GOLD: u16 = 4;
pub const T_SWOR: u16 = 30;
pub const T_HELM: u16 = 37;
pub const T_KEY: u16 = 41;
pub const T_TKNI: u16 = 42;
pub const T_WEAP: u16 = 45;
pub const T_ARMO: u16 = 50;
pub const T_MISC: u16 = 52;
pub const T_HPOT: u16 = 76;

/// Barbarian: inventory record 4 (10 × 4, §1.3).
pub const CLASS: u32 = 4;
pub const GAME_SEED: u32 = 0x1A7E;

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn stat_data() -> Arc<StatData> {
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
    let states = BinTable {
        name: "states".into(),
        source: "synthetic".into(),
        count: 0,
        record_size: States::SIZE,
        records: Vec::new(),
    };
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        states: StateTable::new(&states, &StateMaps::default()).expect("states"),
        damage_regen: vec![0; 8],
        aurastate: vec![0; 8],
        rescale_precision: d2_sim::stats::DEFAULT_RESCALE_PRECISION,
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
        (T_HELM, T_ARMO),
        (T_SHIE, T_ARMO),
        (T_SWOR, T_WEAP),
        (T_GOLD, T_MISC),
        (T_HPOT, T_MISC),
        (T_KEY, T_MISC),
        (T_TKNI, T_WEAP),
    ] {
        set(usize::from(c), usize::from(p));
    }
    m
}

/// (code, type, invwidth, invheight, autobelt, useable, stackable,
/// maxstack, durability, nodurability, quest, component, reqstr) per
/// record.
type Row = ([u8; 4], u16, u8, u8, u8, u8, u8, u32, u8, u8, u8, u8, u16);
const ROWS: [Row; 10] = [
    (*b"cap ", T_HELM, 2, 2, 0, 0, 0, 0, 12, 0, 0, 0, 0),
    (*b"gld ", T_GOLD, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0),
    (*b"ssd ", T_SWOR, 1, 3, 0, 0, 0, 0, 24, 0, 0, 0, 0),
    (*b"2hs ", T_SWOR, 2, 4, 0, 0, 0, 0, 44, 0, 0, 0, 0),
    (*b"buc ", T_SHIE, 2, 2, 0, 0, 0, 0, 12, 0, 0, 0, 0),
    (*b"hp1 ", T_HPOT, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0),
    (*b"key ", T_KEY, 1, 1, 0, 0, 1, 12, 0, 0, 3, 5, 0),
    (*b"tkf ", T_TKNI, 1, 2, 0, 0, 1, 50, 20, 0, 0, 0, 0),
    (*b"ky2 ", T_KEY, 1, 1, 0, 0, 1, 12, 20, 1, 0, 0, 0),
    (*b"hlm ", T_HELM, 2, 2, 0, 0, 0, 0, 12, 0, 0, 0, 20),
];

/// The items creation reads (`items::ItemTables`).
pub fn item_tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    let itemtypes = (0..N_TYPES)
        .map(|_| {
            let mut t = Itemtypes::decode(&[0u8; Itemtypes::SIZE]);
            t.class = 0xFF;
            t.staffmods = 0xFF;
            // Empty `shoots`: the link miss (link16 −1).
            t.shoots = 0xFFFF;
            t.rare = 1;
            t
        })
        .collect();
    ItemTables {
        items: ROWS
            .iter()
            .map(|r| ItemRec {
                code: r.0,
                type_: r.1 as i16,
                level: 1,
                invwidth: r.2,
                invheight: r.3,
                stackable: r.6,
                maxstack: r.7,
                durability: r.8,
                nodurability: r.9,
                ..ItemRec::default()
            })
            .collect(),
        itemtypes,
        equiv: equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// The measured grid records of §1.3 (0–15) and belt boxes of D2.
pub fn inv_tables() -> InvTables {
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
        N_TYPES
    ];
    for (t, loc1, loc2) in [(T_HELM, 1, 1), (T_SWOR, 4, 5), (T_SHIE, 5, 4)] {
        let r = &mut itemtypes[usize::from(t)];
        r.body = 1;
        r.bodyloc1 = loc1;
        r.bodyloc2 = loc2;
    }
    itemtypes[usize::from(T_HPOT)].beltable = 1;
    InvTables {
        grids,
        belts: vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16],
        items: ROWS
            .iter()
            .map(|r| InvItemRec {
                code: r.0,
                type_: r.1 as i16,
                invwidth: r.2,
                invheight: r.3,
                autobelt: r.4,
                useable: r.5,
                stackable: r.6,
                maxstack: r.7,
                quest: r.10,
                component: r.11,
                reqstr: r.12,
                ..InvItemRec::default()
            })
            .collect(),
        itemtypes,
        equiv: equiv(),
        books: Vec::new(),
        item_use: Default::default(),
    }
}

#[derive(Default)]
pub struct Hooks;
impl StatHost for Hooks {}
impl UnitHooks for Hooks {}
impl LifecycleHooks for Hooks {}

/// The answers of every seam of [`Rest`] that returns something. The
/// default is the scenario world's (pickups in range, nothing else
/// special); the forwarding tests set non-default values.
#[derive(Clone, Debug, Default)]
pub struct Answers {
    pub distance: i32,
    pub collides: bool,
    pub room_at: bool,
    pub spot: Option<Spot>,
    pub in_town: bool,
    pub flag: bool,
    pub owners: Vec<Owner>,
    pub copy: Option<Guid>,
    pub owner: Option<Owner>,
    pub share_id: i32,
    pub code: u32,
    pub bits: Vec<u8>,
    pub msgs: Vec<Vec<u8>>,
    pub two_handed: BTreeSet<Guid>,
    pub gold: bool,
    pub number: i32,
    pub ammo: Option<i16>,
    pub trade_gate: Option<bool>,
    /// `stack_quality_ok`, `has_allowed_location`, `auto_equip_allows`.
    pub open_ok: bool,
    pub code2: u32,
    pub level_req: i32,
}

/// The seams without a provider: answers from [`Answers`], every call
/// logged in order (`name args`).
#[derive(Default)]
pub struct Rest {
    pub log: RefCell<Vec<String>>,
    pub pos: BTreeMap<Owner, (i32, i32)>,
    pub a: Answers,
    pub sent: Vec<(Owner, Vec<u8>)>,
}

impl Rest {
    fn note(&self, s: String) {
        self.log.borrow_mut().push(s);
    }
    pub fn last(&self) -> String {
        self.log.borrow().last().cloned().unwrap_or_default()
    }
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
    pub fn called(&self, prefix: &str) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|l| l.starts_with(prefix))
            .count()
    }
}

fn og(o: Owner) -> String {
    format!("{}:{}", o.ty, o.guid)
}

impl MovePending for Rest {
    fn distance(&self, a: Owner, b: Owner) -> i32 {
        self.note(format!("distance {} {}", og(a), og(b)));
        self.a.distance
    }
    fn collides(&self, a: Owner, b: Owner, mask: u32) -> bool {
        self.note(format!("collides {} {} {mask}", og(a), og(b)));
        self.a.collides
    }
    fn walk_to_item(&mut self, player: Owner, item: Guid, cursor: bool) {
        self.note(format!("walk_to_item {} {item} {cursor}", og(player)));
    }
    fn room_at(&self, x: i32, y: i32) -> bool {
        self.note(format!("room_at {x} {y}"));
        self.a.room_at
    }
    fn free_spot(
        &self,
        start: (i32, i32),
        origin: (i32, i32),
        size: u32,
        mask: u32,
        mask2: u32,
        last: u32,
    ) -> Option<Spot> {
        self.note(format!(
            "free_spot {start:?} {origin:?} {size} {mask} {mask2} {last}"
        ));
        self.a.spot
    }
    fn in_town(&self, player: Owner) -> bool {
        self.note(format!("in_town {}", og(player)));
        self.a.in_town
    }
    fn room_delete_notice(&mut self, item: Guid) {
        self.note(format!("room_delete_notice {item}"));
    }
    fn free_collision(&mut self, item: Guid) {
        self.note(format!("free_collision {item}"));
    }
    fn room_change_notice(&mut self, item: Guid, x: i32, y: i32) {
        self.note(format!("room_change_notice {item} {x} {y}"));
    }
    fn stat_refresh(&mut self, u: Owner) {
        self.note(format!("stat_refresh {}", og(u)));
    }
    fn stat_refresh_unlink(&mut self, u: Owner, b: u32) {
        self.note(format!("stat_refresh_unlink {} {b}", og(u)));
    }
    fn stat_link(&mut self, owner: Owner, item: Guid) {
        self.note(format!("stat_link {} {item}", og(owner)));
    }
    fn charm_relink(&mut self, owner: Owner, item: Guid) {
        self.note(format!("charm_relink {} {item}", og(owner)));
    }
    fn charm_unlink(&mut self, owner: Owner, item: Guid) {
        self.note(format!("charm_unlink {} {item}", og(owner)));
    }
    fn is_active(&self, owner: Owner, item: Guid) -> bool {
        self.note(format!("is_active {} {item}", og(owner)));
        self.a.flag
    }
    fn inventory_pass(&mut self, owner: Owner) {
        self.note(format!("inventory_pass {}", og(owner)));
    }
    fn weapon_in_use_update(&mut self, owner: Owner) {
        self.note(format!("weapon_in_use_update {}", og(owner)));
    }
    fn weapon_bookkeeping(&mut self, owner: Owner) {
        self.note(format!("weapon_bookkeeping {}", og(owner)));
    }
    fn body_leave_effects(&mut self, owner: Owner, item: Guid) {
        self.note(format!("body_leave_effects {} {item}", og(owner)));
    }
    fn hireling_owner_pass(&mut self, owner: Owner) {
        self.note(format!("hireling_owner_pass {}", og(owner)));
    }
    fn belt_remove_allowed(&self, player: Owner) -> bool {
        self.note(format!("belt_remove_allowed {}", og(player)));
        self.a.flag
    }
    fn sound(&mut self, u: Owner, id: u32) {
        self.note(format!("sound {} {id}", og(u)));
    }
    fn pickup_sound(&mut self, player: Owner, item: Guid) {
        self.note(format!("pickup_sound {} {item}", og(player)));
    }
    fn requirement_sound(&mut self, player: Owner) {
        self.note(format!("requirement_sound {}", og(player)));
    }
    fn merc_sound(&mut self, player: Owner) {
        self.note(format!("merc_sound {}", og(player)));
    }
    fn quest_flag(&self, player: Owner, quest: u8, flag: u8) -> bool {
        self.note(format!("quest_flag {} {quest} {flag}", og(player)));
        self.a.flag
    }
    fn quest_item_picked(&mut self, player: Owner, item: Guid) {
        self.note(format!("quest_item_picked {} {item}", og(player)));
    }
    fn quest_item_dropped(&mut self, item: Guid) {
        self.note(format!("quest_item_dropped {item}"));
    }
    fn carry_one(&self, item: Guid) -> bool {
        self.note(format!("carry_one {item}"));
        self.a.flag
    }
    fn held_test_units(&self, player: Owner) -> Vec<Owner> {
        self.note(format!("held_test_units {}", og(player)));
        self.a.owners.clone()
    }
    fn copy_item(&mut self, item: Guid) -> Option<Guid> {
        self.note(format!("copy_item {item}"));
        self.a.copy
    }
    fn give_cursor_item(&mut self, player: Owner, item: Guid) {
        self.note(format!("give_cursor_item {} {item}", og(player)));
    }
    fn consume_one(&mut self, item: Guid) -> bool {
        self.note(format!("consume_one {item}"));
        self.a.flag
    }
    fn set_owner(&mut self, item: Guid, owner: Owner) {
        self.note(format!("set_owner {item} {}", og(owner)));
    }
    fn pile_owner(&self, item: Guid) -> Option<Owner> {
        self.note(format!("pile_owner {item}"));
        self.a.owner
    }
    fn gold_drop_has_no_owner(&self) -> bool {
        self.note("gold_drop_has_no_owner".into());
        self.a.flag
    }
    fn party_share_id(&self, player: Owner) -> i32 {
        self.note(format!("party_share_id {}", og(player)));
        self.a.share_id
    }
    fn party_share(&mut self, player: Owner, take: i32) {
        self.note(format!("party_share {} {take}", og(player)));
    }
    fn owned_gold_pickup(&mut self, player: Owner, pile: Guid, take: i32) {
        self.note(format!("owned_gold_pickup {} {pile} {take}", og(player)));
    }
    fn rest_pile(&mut self, player: Owner, rest: i32) {
        self.note(format!("rest_pile {} {rest}", og(player)));
    }
    fn book_count_changed(&mut self, player: Owner, _book: Guid, n: i32) {
        self.note(format!("book_count_changed {} {n}", og(player)));
    }
    fn use_item(&mut self, player: Owner, target: Owner, item: Guid) -> bool {
        self.note(format!("use_item {} {} {item}", og(player), og(target)));
        self.a.flag
    }
    fn charge_update(&mut self, player: Owner, item: Guid) {
        self.note(format!("charge_update {} {item}", og(player)));
    }
    fn remove_used(&mut self, player: Owner, item: Guid) {
        self.note(format!("remove_used {} {item}", og(player)));
    }
    fn equip_picked(&mut self, player: Owner, item: Guid) -> bool {
        self.note(format!("equip_picked {} {item}", og(player)));
        self.a.flag
    }
    fn filler_linked(&mut self, filler: Guid, target: Guid) {
        self.note(format!("filler_linked {filler} {target}"));
    }
    fn runeword(&mut self, player: Owner, target: Guid) -> bool {
        self.note(format!("runeword {} {target}", og(player)));
        self.a.flag
    }
    fn hireling(&self, player: Owner) -> Option<Owner> {
        self.note(format!("hireling {}", og(player)));
        self.a.owner
    }
    fn owns_hireling(&self, player: Owner, merc: Owner) -> bool {
        self.note(format!("owns_hireling {} {}", og(player), og(merc)));
        self.a.flag
    }
    fn equip_on_merc(&mut self, merc: Owner, item: Guid) {
        self.note(format!("equip_on_merc {} {item}", og(merc)));
    }
    fn merc_after_take(&mut self, merc: Owner) {
        self.note(format!("merc_after_take {}", og(merc)));
    }
    fn pick_npc(&mut self, player: Owner, guid: Guid, cursor: u32) -> u32 {
        self.note(format!("pick_npc {} {guid} {cursor}", og(player)));
        self.a.code
    }
    fn pick_object(&mut self, player: Owner, guid: Guid, cursor: u32) -> u32 {
        self.note(format!("pick_object {} {guid} {cursor}", og(player)));
        self.a.code
    }
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.note(format!("send {} {bytes:?}", og(player)));
        self.sent.push((player, bytes));
    }
    fn item_bits(&self, item: Guid, flags: u32, page: u8) -> Vec<u8> {
        self.note(format!("item_bits {item} {flags} {page}"));
        self.a.bits.clone()
    }
    fn store_messages(&mut self, client: Owner, item: Guid) -> Vec<Vec<u8>> {
        self.note(format!("store_messages {} {item}", og(client)));
        self.a.msgs.clone()
    }
}

impl InvRest for Rest {
    fn pos(&self, u: Owner) -> (i32, i32) {
        self.pos.get(&u).copied().unwrap_or((0, 0))
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        self.note(format!("set_pos {} {x} {y}", og(u)));
        self.pos.insert(u, (x, y));
    }
    /// A plain normal-quality `gld` (the request layout is the open seam).
    fn gold_request(&self, unit: Owner, gld: usize) -> Option<(ItemRequest, ItemSpawn)> {
        self.note(format!("gold_request {} {gld}", og(unit)));
        self.a.gold.then(|| {
            (
                plain(gld),
                ItemSpawn {
                    room: None,
                    mode: 3,
                    init_flags: 1,
                },
            )
        })
    }
    fn socket_link(&mut self, target: Guid, item: Guid, kind: u8) -> bool {
        self.note(format!("socket_link {target} {item} {kind}"));
        self.a.flag
    }
    fn link_into_item(&mut self, target: Guid, filler: Guid) -> bool {
        self.note(format!("link_into_item {target} {filler}"));
        self.a.flag
    }
    fn socket_filled(&self, item: Guid) -> bool {
        self.note(format!("socket_filled {item}"));
        self.a.flag
    }
    fn socket_filler(&self, item: Guid) -> bool {
        self.note(format!("socket_filler {item}"));
        self.a.flag
    }
    fn spell(&self, item: Guid) -> i32 {
        self.note(format!("spell {item}"));
        self.a.number
    }
    fn trade_hook(&mut self, owner: Owner, item: Guid) {
        self.note(format!("trade_hook {} {item}", og(owner)));
    }
    fn item_active_on(&self, item: Guid, unit: Owner) -> bool {
        self.note(format!("item_active_on {item} {}", og(unit)));
        self.a.flag
    }
    fn own_contribution(&self, item: Guid, unit: Owner, stat: u16) -> i32 {
        self.note(format!("own_contribution {item} {} {stat}", og(unit)));
        self.a.number
    }
    fn level_requirement(&self, item: Guid, unit: Owner) -> i32 {
        self.note(format!("level_requirement {item} {}", og(unit)));
        self.a.level_req
    }
    fn two_handed(&self, item: Guid) -> bool {
        self.a.two_handed.contains(&item)
    }
    fn one_or_two_handed(&self, unit: Owner, item: Guid) -> bool {
        self.note(format!("one_or_two_handed {} {item}", og(unit)));
        self.a.flag
    }
    fn ammo_type(&self, item: Guid) -> Option<i16> {
        self.note(format!("ammo_type {item}"));
        self.a.ammo
    }
    fn has_allowed_location(&self, item: Guid) -> bool {
        self.note(format!("has_allowed_location {item}"));
        self.a.open_ok
    }
    fn quiver_kind(&self, item: Guid) -> bool {
        self.note(format!("quiver_kind {item}"));
        self.a.flag
    }
    fn player_data_4c(&self, player: Owner) -> u32 {
        self.note(format!("player_data_4c {}", og(player)));
        self.a.code
    }
    fn player_data_50(&self, player: Owner) -> u32 {
        self.note(format!("player_data_50 {}", og(player)));
        self.a.code2
    }
    fn npc_talking(&self, npc: Owner, player: Owner) -> bool {
        self.note(format!("npc_talking {} {}", og(npc), og(player)));
        self.a.flag
    }
    fn player_trade_gate(&self, player: Owner) -> Option<bool> {
        self.note(format!("player_trade_gate {}", og(player)));
        self.a.trade_gate
    }
}

/// A normal-quality request for a record, never ethereal (request flag
/// 0x02, `generation.md` §1.5).
pub fn plain(record: usize) -> ItemRequest {
    ItemRequest {
        item: record as i32,
        format: 101,
        ilvl: 1,
        quality: q::NORMAL,
        flags2: 0x2,
        ..ItemRequest::default()
    }
}

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
    pub inv: InvTables,
    pub state: InvState,
    pub rest: Rest,
    pub room: RoomId,
    pub player: UnitId,
}

impl World {
    /// A barbarian (class 4) in one room of act 0, with an inventory,
    /// level 1, strength and dexterity 10, at (10, 10).
    pub fn new() -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = game.lists.create_room(0).unwrap();
        let mut w = Self {
            game,
            units: Units::new(),
            stats: StatLists::new(stat_data()),
            data: UnitData {
                expansion: true,
                ..UnitData::default()
            },
            hooks: Hooks,
            fields: GameFields::new(Seed::init_low(GAME_SEED), true),
            tables: item_tables(),
            items: ItemStore::new(),
            inv: inv_tables(),
            state: InvState::new(),
            rest: Rest {
                a: Answers {
                    distance: 1,
                    open_ok: true,
                    level_req: -1,
                    ..Answers::default()
                },
                ..Rest::default()
            },
            room,
            player: UnitId(0),
        };
        let p = w.alloc(UnitType::Player, CLASS);
        w.player = p;
        let guid = w.units.get(p).unwrap().guid;
        w.state
            .add_inventory(p, UnitKind::Player { class: CLASS as u8 }, guid);
        w.set_stat(p, moves::stat::LEVEL, 1);
        w.set_stat(p, 0, 10);
        w.set_stat(p, 2, 10);
        w.rest.pos.insert(Owner::player(guid), (10, 10));
        w
    }

    pub fn alloc(&mut self, ty: UnitType, class: u32) -> UnitId {
        let req = AllocRequest {
            ty,
            class,
            room: Some(self.room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        let mut sim = Sim {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
        };
        allocate(&mut sim, &mut self.hooks, &mut self.fields.seed, &req)
            .expect("allocate")
            .expect("allocated")
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

    /// Runs `f` on a desk over the whole state.
    pub fn desk<T>(&mut self, f: impl FnOnce(&mut InvDesk<'_, '_, Hooks, Rest>) -> T) -> T {
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
        let mut d = InvDesk::new(&mut econ, &self.inv, &mut self.state, &mut self.rest);
        f(&mut d)
    }

    pub fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.stats.unit_set(&mut self.hooks, unit, stat, value, 0);
    }

    pub fn pguid(&self) -> Guid {
        self.units.get(self.player).unwrap().guid
    }
    pub fn me(&self) -> Owner {
        Owner::player(self.pguid())
    }

    /// A normal item made in the room (mode 3), identified.
    pub fn ground_item(&mut self, record: usize, x: i32, y: i32) -> Guid {
        let room = self.room;
        let u = self
            .econ()
            .create_item(
                &mut plain(record),
                false,
                ItemSpawn {
                    room: Some(room),
                    mode: 3,
                    init_flags: 1,
                },
            )
            .expect("create");
        self.items.get_mut(u).unwrap().flags |= 0x10;
        let g = self.units.get(u).unwrap().guid;
        self.desk(|d| moves::MoveUnits::set_pos(d, Owner::item(g), x, y));
        g
    }

    /// A ground item picked to the cursor through C→S 0x16, the deferred
    /// messages drained.
    pub fn cursor_item(&mut self, record: usize) -> Guid {
        let g = self.ground_item(record, 11, 11);
        assert_eq!(self.handle(&pick(g, 1)), Ok(0));
        self.drain();
        g
    }

    pub fn unit(&self, item: Guid) -> Option<UnitId> {
        self.game.lists.find_unit(UnitType::Item, item)
    }

    /// One C→S message through `items::moves::handle`.
    pub fn handle(&mut self, msg: &[u8]) -> Result<u32, moves::MoveFatal> {
        let p = self.pguid();
        self.desk(|d| moves::handle(d, p, msg)).expect("handled id")
    }

    /// The player's update pass for its own client, then the clean-up of
    /// §6.1 rule 4.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        let p = self.pguid();
        let me = self.me();
        self.desk(|d| {
            let out = moves::player_update(d, p, p).expect("update pass");
            d.update_done(me);
            out
        })
    }

    pub fn data(&self, item: Guid) -> InvItem {
        self.state.items[&self.unit(item).unwrap()]
    }
    pub fn inventory(&self) -> &Inventory {
        &self.state.inventories[&self.player]
    }
    pub fn in_room(&self, item: Guid) -> bool {
        self.game
            .lists
            .unit(self.unit(item).unwrap())
            .unwrap()
            .room()
            .is_some()
    }
}

// ---- C→S messages (`client-messages.tsv` layouts) ------------------------

pub fn msg(id: u8, fields: &[u32]) -> Vec<u8> {
    let mut m = vec![id];
    for f in fields {
        m.extend_from_slice(&f.to_le_bytes());
    }
    m
}
/// 0x16 [type][GUID][cursor].
pub fn pick(item: Guid, cursor: u32) -> Vec<u8> {
    msg(0x16, &[4, item, cursor])
}
pub fn insert(item: Guid, x: u32, y: u32, page: u32) -> Vec<u8> {
    msg(0x18, &[item, x, y, page])
}
/// 0x1A / 0x1B / 0x1D [item u32][location u32].
pub fn body(id: u8, item: Guid, loc: u32) -> Vec<u8> {
    msg(id, &[item, loc])
}
