//! Integration tests: the item-move handlers (`items::moves`) run on the
//! inventory model (`items::inventory`), the real unit allocator, unit
//! lists, stat lists and item creation through [`InvDesk`], on synthetic
//! tables with a fixed game seed. Only the seams this wiring leaves open
//! ([`InvRest`] with its [`MovePending`] part) are a fake ([`Rest`]),
//! which logs every call.

mod belt;
mod bits;
mod buffer;
mod copy;
mod corpse;
mod cube_open;
mod equip;
mod gold;
mod ground;
mod host;
mod identify;
mod item_use;
mod link;
mod load;
mod merc;
mod mod_use;
mod mutant_tests;
mod queries;
mod quest_delete;
mod save_index;
mod socket;
mod stack;
mod town_portal;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Record};

use super::{InvDesk, InvRest, InvState};
use crate::game::Game;
use crate::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec, ItemUseTables};
use crate::items::inventory::{InvTables, UnitKind};
use crate::items::moves::{self, Guid, MovePending, Owner, Spot};
use crate::items::tables::ItemRec;
use crate::items::{q, ItemRequest, ItemTables};
use crate::rng::Seed;
use crate::stats::{ClassStats, StatData, StatHost, StatLists, StatTable, StateTable};
use crate::units::hooks::{Sim, UnitData, UnitHooks};
use crate::units::lifecycle::{allocate, AllocRequest, LifecycleHooks};
use crate::units::record::Units;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::economy::{Economy, GameFields, ItemSpawn, ItemStore};

const N_STATS: usize = 359;
const N_TYPES: usize = 80;

/// Item records (combined index; both table projections use this order).
pub const CAP: usize = 0;
pub const GOLD: usize = 1;
pub const SWORD: usize = 2;
pub const TWO_HANDER: usize = 3;
pub const SHIELD: usize = 4;
pub const HP1: usize = 5;
pub const HP2: usize = 6;
pub const KEY: usize = 7;
pub const KNIFE: usize = 8;
pub const HEAVY_CAP: usize = 9;
pub const ISC: usize = 10;
pub const BOX: usize = 13;

/// Itemtypes rows.
const T_SHIE: u16 = 2;
const T_GOLD: u16 = 4;
const T_SWOR: u16 = 30;
const T_HELM: u16 = 37;
const T_KEY: u16 = 41;
const T_TKNI: u16 = 42;
const T_BOOK: u16 = 18;
const T_SCRO: u16 = 22;
const T_WEAP: u16 = 45;
const T_ARMO: u16 = 50;
const T_MISC: u16 = 52;
const T_HPOT: u16 = 76;
const T_GEM: u16 = 20;
const T_CHARM: u16 = 13;

/// Player class (barbarian: inventory record 4, 10 × 4).
pub const CLASS: u32 = 4;
/// The fixed game seed of every test.
pub const GAME_SEED: u32 = 0x1A7E;

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

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

/// (code, type, invwidth, invheight, reqstr, autobelt, useable, stackable,
/// maxstack, durability) per record.
type Row = ([u8; 4], u16, u8, u8, u16, u8, u8, u8, u32, u8);
const ROWS: [Row; 17] = [
    (*b"cap ", T_HELM, 2, 2, 0, 0, 0, 0, 0, 12),
    (*b"gld ", T_GOLD, 1, 1, 0, 0, 0, 0, 0, 0),
    (*b"ssd ", T_SWOR, 1, 3, 0, 0, 0, 0, 0, 24),
    (*b"2hs ", T_SWOR, 2, 4, 0, 0, 0, 0, 0, 44),
    (*b"buc ", T_SHIE, 2, 2, 0, 0, 0, 0, 0, 12),
    (*b"hp1 ", T_HPOT, 1, 1, 0, 1, 1, 0, 0, 0),
    (*b"hp2 ", T_HPOT, 1, 1, 0, 1, 1, 0, 0, 0),
    (*b"key ", T_KEY, 1, 1, 0, 0, 0, 1, 12, 0),
    (*b"tkf ", T_TKNI, 1, 2, 0, 0, 0, 1, 50, 20),
    (*b"hlm ", T_HELM, 2, 2, 50, 0, 0, 0, 0, 12),
    (*b"isc ", T_MISC, 1, 1, 0, 0, 1, 0, 0, 0),
    (*b"tsc ", T_SCRO, 1, 1, 0, 0, 1, 0, 0, 0),
    (*b"tbk ", T_BOOK, 1, 2, 0, 0, 1, 1, 20, 0),
    (*b"box ", T_MISC, 2, 2, 0, 0, 1, 0, 0, 0),
    (*b"gsw ", T_GEM, 1, 1, 0, 0, 0, 0, 0, 0),
    (*b"cm1 ", T_CHARM, 1, 1, 0, 0, 0, 0, 0, 0),
    (*b"mp1 ", T_HPOT, 1, 1, 0, 1, 1, 0, 0, 0),
];
pub const MP1: usize = 16;

/// The items code of the potion calcs (PUSH8 / PUSH16, END): `30` at 0 and
/// `192` at 3 (`hp1` `calc1` / `len`), `20` at 7 and `128` at 10 (`mp1`),
/// the values of the shipped data.
pub const ITEMS_CODE: [u8; 14] = [
    0x07, 30, 0x00, 0x08, 192, 0, 0x00, 0x07, 20, 0x00, 0x08, 128, 0, 0x00,
];

/// The `misc.txt` use fields of a potion record:
/// state, stat1, `calc1` and `len` offsets into [`ITEMS_CODE`]. `hp2`
/// reuses `hp1`'s (fixture).
fn potion_use(rec: &mut InvItemRec) {
    let (state, stat, calc, len) = match &rec.code {
        b"hp1 " | b"hp2 " => (100, 74, 0, 3),
        b"mp1 " => (106, 26, 7, 10),
        _ => return,
    };
    rec.use_state = state;
    rec.use_stat = [stat, -1, -1];
    rec.use_calc = [calc, u32::MAX, u32::MAX];
    rec.use_len = len;
}
pub const GEM: usize = 14;
pub const CHARM: usize = 15;
pub const TSC: usize = 11;
pub const TBK: usize = 12;

/// The items creation reads (`items::ItemTables`).
pub fn item_tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    let itemtypes = (0..N_TYPES)
        .map(|_| {
            let mut t = Itemtypes::decode(&[0u8; Itemtypes::SIZE]);
            t.class = 0xFF;
            t.staffmods = 0xFF;
            t.rare = 1;
            // An empty `shoots` cell compiles to the link's miss value
            // (link16 miss −1).
            t.shoots = 0xFFFF;
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
                stackable: r.7,
                maxstack: r.8,
                durability: r.9,
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

/// The inventory grid records (0–15) and belt boxes.
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
                reqstr: r.4,
                autobelt: r.5,
                useable: r.6,
                stackable: r.7,
                maxstack: r.8,
                // `pSpell` of the `misc.txt` rows.
                pspell: match &r.0 {
                    b"tsc " | b"tbk " => 2,
                    b"isc " => 1,
                    b"hp1 " | b"hp2 " | b"mp1 " => 3,
                    _ => 0,
                },
                ..InvItemRec::default()
            })
            .map(|mut r| {
                potion_use(&mut r);
                r
            })
            .collect(),
        itemtypes,
        equiv: equiv(),
        books: Vec::new(),
        item_use: ItemUseTables {
            code: ITEMS_CODE.to_vec(),
            maxstat: Vec::new(),
        },
    }
}

#[derive(Default)]
pub struct Hooks {
    /// The remove callbacks of detached lists: (unit, state, callback).
    pub removed: Vec<(UnitId, u32, u32)>,
    /// The corpse pickups the desk handed over (player, corpse).
    pub corpse_pickups: Vec<(UnitId, UnitId)>,
    /// The answer of the corpse pickup's permission checks.
    pub corpse_allowed: bool,
    /// The Town Portal cast's answer (`None`: no cast host) and the
    /// players it was asked for.
    pub cast: Option<(u32, bool)>,
    pub casts: Vec<UnitId>,
    /// The game's rules (D2RUST's unless a test sets a mod game's).
    pub rules: crate::rules::Ruleset,
}
impl StatHost for Hooks {
    fn list_removed(
        &mut self,
        _: &mut StatLists,
        unit: UnitId,
        state: u32,
        _: crate::stats::lists::ListId,
        callback: crate::stats::lists::RemoveCallback,
    ) {
        self.removed.push((unit, state, callback.0));
    }
}
impl UnitHooks for Hooks {
    fn player_corpse_pickup(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        player: UnitId,
        corpse: UnitId,
    ) -> bool {
        self.corpse_pickups.push((player, corpse));
        self.corpse_allowed
    }
}
impl LifecycleHooks for Hooks {
    fn town_portal_cast(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut Seed,
        player: UnitId,
    ) -> Option<(u32, bool)> {
        self.casts.push(player);
        self.cast
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
}

/// The seams without a provider: answers set by the test, every call
/// logged in order.
#[derive(Default)]
pub struct Rest {
    pub log: Vec<String>,
    pub pos: BTreeMap<Owner, (i32, i32)>,
    /// `distance` answer (0x16).
    pub distance: i32,
    /// `free_spot` answer.
    pub spot: Option<Spot>,
    /// `room_at` answer.
    pub room_at: bool,
    pub in_town: bool,
    /// `has_used_skill` answers the opposite (the 0x61 proceeds).
    pub merc_ready: bool,
    pub two_handed: BTreeSet<Guid>,
    /// `use_item` answer.
    pub use_ok: bool,
    /// Gold piles are created (`gold_request` answers).
    pub gold: bool,
    pub sent: Vec<(Owner, Vec<u8>)>,
    /// Calls of `&self` methods worth checking (free-spot searches).
    pub queries: RefCell<Vec<String>>,
    /// `first_pet` answer (any player, type 3).
    pub golem: Option<Owner>,
    /// `monster_skill1_param6` answer for class 290.
    pub golem_param6: Option<i32>,
}

impl Rest {
    pub fn called(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(prefix)).count()
    }
}

impl MovePending for Rest {
    fn distance(&self, _: Owner, _: Owner) -> i32 {
        self.distance
    }
    fn walk_to_item(&mut self, player: Owner, item: Guid, cursor: bool) {
        self.log
            .push(format!("walk_to_item {} {item} {cursor}", player.guid));
    }
    fn room_at(&self, _: i32, _: i32) -> bool {
        self.room_at
    }
    fn has_used_skill(&self, _: Owner) -> bool {
        !self.merc_ready
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
        self.queries.borrow_mut().push(format!(
            "free_spot {start:?} {origin:?} {size} {mask:#x} {mask2:#x} {last}"
        ));
        self.spot
    }
    fn in_town(&self, _: Owner) -> bool {
        self.in_town
    }
    fn room_delete_notice(&mut self, item: Guid) {
        self.log.push(format!("room_delete_notice {item}"));
    }
    fn free_collision(&mut self, item: Guid) {
        self.log.push(format!("free_collision {item}"));
    }
    fn room_change_notice(&mut self, item: Guid, x: i32, y: i32) {
        self.log.push(format!("room_change_notice {item} {x} {y}"));
    }
    fn stat_refresh(&mut self, u: Owner) {
        self.log.push(format!("stat_refresh {}", u.guid));
    }
    fn corpse_taken(&mut self, p: Owner, c: Owner) {
        self.log.push(format!("corpse_taken {} {}", p.guid, c.guid));
    }
    fn sound(&mut self, u: Owner, id: u32) {
        self.log.push(format!("sound {} {id:#x}", u.guid));
    }
    fn pickup_sound(&mut self, player: Owner, item: Guid) {
        self.log
            .push(format!("pickup_sound {} {item}", player.guid));
    }
    fn requirement_sound(&mut self, player: Owner) {
        self.log.push(format!("requirement_sound {}", player.guid));
    }
    fn quest_item_picked(&mut self, _: Owner, item: Guid) {
        self.log.push(format!("quest_item_picked {item}"));
    }
    fn quest_item_dropped(&mut self, item: Guid) {
        self.log.push(format!("quest_item_dropped {item}"));
    }
    fn set_owner(&mut self, item: Guid, owner: Owner) {
        self.log.push(format!("set_owner {item} {}", owner.guid));
    }
    fn rest_pile(&mut self, player: Owner, rest: i32) {
        self.log.push(format!("rest_pile {} {rest}", player.guid));
    }
    fn use_item(&mut self, player: Owner, target: Owner, item: Guid) -> bool {
        self.log
            .push(format!("use_item {} {} {item}", player.guid, target.guid));
        self.use_ok
    }
    fn charge_update(&mut self, _: Owner, item: Guid) {
        self.log.push(format!("charge_update {item}"));
    }
    fn remove_used(&mut self, _: Owner, item: Guid) {
        self.log.push(format!("remove_used {item}"));
    }
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.sent.push((player, bytes));
    }
}

impl InvRest for Rest {
    fn first_pet(&self, _: Owner, t: i32) -> Option<Owner> {
        (t == 3).then_some(self.golem).flatten()
    }
    fn monster_skill1_param6(&self, class: u16) -> Option<i32> {
        (class == 290).then_some(self.golem_param6).flatten()
    }
    fn pos(&self, u: Owner) -> (i32, i32) {
        self.pos.get(&u).copied().unwrap_or((0, 0))
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        self.pos.insert(u, (x, y));
    }
    /// A plain normal-quality `gld` (the request layout is the open seam).
    fn gold_request(&self, _: Owner, gld: usize) -> Option<(ItemRequest, ItemSpawn)> {
        self.gold.then(|| {
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
    fn item_active_on(&self, _: Guid, _: Owner) -> bool {
        false
    }
    fn own_contribution(&self, _: Guid, _: Owner, _: u16) -> i32 {
        0
    }
    fn level_requirement(&self, _: Guid, _: Owner) -> i32 {
        -1
    }
    fn two_handed(&self, item: Guid) -> bool {
        self.two_handed.contains(&item)
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

/// A normal-quality request for a record, never ethereal (request flag
/// 0x02; the same-class check compares the ethereal bit).
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
        Self::with_tables(inv_tables())
    }

    pub fn with_tables(inv: InvTables) -> Self {
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
            hooks: Hooks::default(),
            fields: GameFields::new(Seed::init_low(GAME_SEED), true),
            tables: item_tables(),
            items: ItemStore::new(),
            inv,
            state: InvState::new(),
            rest: Rest {
                distance: 1,
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

    /// One player regeneration tick.
    pub fn regen_tick(&mut self) {
        let p = self.player;
        let mut sim = Sim {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
        };
        crate::units::dispatch::player_regen(&mut sim, &mut self.hooks, p, 0, 0).expect("regen");
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

    /// A normal item on the ground of the room at (x, y), identified
    /// (callers set 0x10).
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

    /// The player's update pass (`items::moves::player_update`) for its
    /// own client, then the clean-up of the pending messages.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        let p = self.pguid();
        let me = self.me();
        self.desk(|d| {
            let out = moves::player_update(d, p, p).expect("update pass");
            d.update_done(me);
            out
        })
    }

    pub fn mode(&self, item: Guid) -> u32 {
        self.units.get(self.unit(item).unwrap()).unwrap().mode
    }
    pub fn data(&self, item: Guid) -> crate::items::inventory::InvItem {
        self.state.items[&self.unit(item).unwrap()]
    }
    pub fn inventory(&self) -> &crate::items::inventory::Inventory {
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
pub fn drop_msg(item: Guid) -> Vec<u8> {
    msg(0x17, &[item])
}
pub fn insert(item: Guid, x: u32, y: u32, page: u32) -> Vec<u8> {
    msg(0x18, &[item, x, y, page])
}
pub fn lift(item: Guid) -> Vec<u8> {
    msg(0x19, &[item])
}
/// 0x1A / 0x1B / 0x1D [item u32][location u32].
pub fn body(id: u8, item: Guid, loc: u32) -> Vec<u8> {
    msg(id, &[item, loc])
}
pub fn unequip(loc: u16) -> Vec<u8> {
    let mut m = vec![0x1C];
    m.extend_from_slice(&loc.to_le_bytes());
    m
}

/// A deferred 0x9C / 0x9D: (id, action, item GUID).
pub fn head(m: &[u8]) -> (u8, u8, Guid) {
    (m[0], m[1], u32::from_le_bytes([m[4], m[5], m[6], m[7]]))
}

/// The item messages of an update pass (0x47 / 0x48 dropped).
pub fn item_msgs(out: &[Vec<u8>]) -> Vec<(u8, u8, Guid)> {
    out.iter()
        .filter(|m| m[0] == 0x9C || m[0] == 0x9D)
        .map(|m| head(m))
        .collect()
}
