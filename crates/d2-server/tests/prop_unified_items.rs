//! State-machine properties of the game's one item store and one
//! inventory model on a wired host (`docs/handoff/unify-items.md`):
//! random sequences of item moves (C→S 0x16–0x29, 0x50, 0x61, 0x63),
//! drops, pick-ups, vendor buys and sells (0x32, 0x33 after a talk,
//! chat and trade with Akara) and cube put-ins and transmutes (0x2A,
//! 0x4F), plus ground items created mid-run by the economy wiring (the
//! creation a death drop runs), one frame each (dispatch, then the host
//! tick with its update pass), on `SimGame<ActionSim<_>, WiredWorld<_>>`.
//!
//! After every frame:
//!
//! 1. **One place.** Every live item unit has its item data in the one
//!    store (`ActionHooks::items`) and the store holds nothing else; it
//!    is in exactly one of: a page grid, the belt, a body location, a
//!    cursor, the ground (a room), a vendor's store or gamble list; its
//!    unit mode matches that place (0 stored, 1 equipped, 2 belt, 3
//!    ground, 4 cursor); a ground item is in no inventory. The one
//!    exception is "limbo" (in none of them), only on these paths: 0x63's
//!    failed slot placement, 0x1F's failed placement, the placement step
//!    that clears a held cursor item (0x2A, a transmute) and the equip
//!    step of an auto pick-up (0x16) that leaves the item in no place;
//!    each is kept as a pinned test and a question in
//!    `docs/handoff/prop-unified-items.md`.
//! 2. **No overlap.** Every grid cell holds an item of the grid's own
//!    list, and each listed item fills exactly its invwidth × invheight
//!    rectangle at its item-data (x, y) (body and belt: 1 × 1 at x =
//!    location / slot); an item is in one grid list at most and its
//!    node grid is that grid + 1.
//! 3. **Views agree.** The inventory's item list is the union of its
//!    grids' lists in link order; the linked count
//!    is the list's length; the cursor item is in no grid;
//!    every listed item's owning inventory is that one; the model's item
//!    data GUID / record / stored page equal the unit record's and the
//!    store's (its mode and flags are copies `InvDesk::new` refreshes per
//!    call, so the unit record's mode is the one checked in 1);
//!    `InvState::holds` / `items_of` / `cursor_of` / `body_items` agree
//!    with the grids.
//! 4. **Ids.** Live item GUIDs are distinct and a unit's GUID never
//!    changes; no inventory or store refers to a freed unit.
//! 5. **The S→C stream matches the state.** Each client buffer splits
//!    whole by the S→C size rule; every 0x9C / 0x9D carries its total
//!    size in byte 2 and names an item that was live in the frame;
//!    0x9D names the player as owner; an update pass that sent item
//!    messages ends with 0x47, 0x48 for the player; the
//!    last deferred message naming an item agrees with where the item is
//!    at the end of the frame (`item-actions.tsv`: 0x9C 1 / 0x12 the
//!    cursor, 0x9C 4 a page, 0x9C 0xD a page or the cursor (both
//!    sides of a swap), 0x9C 0xE the belt, 0x9D 6 the body; not for an
//!    item already on the update list at the frame's start: the next pass sends it);
//!    and every item of the player that moved between the cursor, a page,
//!    the belt and the body in the frame was announced by a 0x9C / 0x9D
//!    (direct or deferred).
//!
//! Seams answered by staging (as `e2e_single_player.rs`
//! stages them): the distance 3 (< 5), the free-spot search answers the
//! start spot in the player's room, the vendor's store grid always has
//! room, the item copy is null, the cube's opening is the
//! staged interaction (type 4, the cube) and its closing ends it after
//! the 0x4F. A ground item's item-data position is written beside the
//! player (the drop fixture's reading); items are identified and the
//! player has strength and dexterity 15 (as the e2e's, so the equip
//! requirements let equips through); Akara stands outside the rooms (as in
//! `e2e_vendor.rs`: no table here gives her monster modes a room tick
//! would run).
//!
//! Default case counts are small; `PROPTEST_CASES` overrides them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::{EquivMatrix, StateMaps};
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Monstats, Record, States};
use d2_proto::transport::split_server_buffer;
use d2_proto::CLIENT_MESSAGES;
use d2_server::adapters::handlers::items::moves::{InvParts, MoveRest};
use d2_server::adapters::handlers::items::{CubeParts, ItemPending};
use d2_server::adapters::handlers::world::{ActionEvents, ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::buffers::ClientBuffers;
use d2_server::dispatch::dispatch;
use d2_server::seams::{PlayerGate, Pos, Tick};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind as DrlgRoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{grid_id, mode, InvItem, InvTables, Inventory, UnitKind as InvKind};
use d2_sim::items::moves::{Guid, MovePending, Owner, Spot};
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{flag, q, ty, ItemRequest, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{ClassStats, StatData, StatTable, StateTable};
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::economy::{GameFields, ItemSpawn, QuestRest};
use d2_sim::wiring::interaction::{HirelingRest, NpcRest, PlayerQuestsRef, VendorRest};
use d2_sim::wiring::inventory::{InvError, InvRest};
use d2_sim::world::cube::{
    input_flags, kind as cube_kind, CraftMod, CubeData, InputSlot, ItemRecord, OutputSlot, Recipe,
};
use d2_sim::world::npc::{self, class, ImbueMods, InvEntry, ItemFacts, NpcControl};
use d2_sim::world::quests::{
    PlayerQuests, QuestChain, QuestControl, QuestTables, TextList, UnitKind,
};
use d2_sim::world::vendors::price::Bonus;
use d2_sim::world::vendors::{
    NpcPrices, Transaction, TypeRec, VendorItem, VendorTables, NO_CODE, XXX,
};
use proptest::prelude::*;
use proptest::test_runner::Config;

/// Proptest config with `default` cases, or `PROPTEST_CASES` when set.
fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

const N_STATS: usize = 359;
const N_TYPES: usize = 80;
const N_MONSTATS: usize = 400;
const GOLD: u16 = 14;
const PLAYER_GOLD: i32 = 5000;

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost (359 stats, no ops, no shifts) through the
/// d2-data fix-up, with an empty states table.
fn stat_data() -> Arc<StatData> {
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

// ---- the field room ----------------------------------------------------------------------

/// Cold Plains (`levels` row 3): one 8 × 8-tile room.
const COLD_PLAINS: u32 = 3;

struct FieldTypes;

impl LevelTypes for FieldTypes {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        if drlg.level(level).id == COLD_PLAINS {
            let r = drlg.alloc_room(level, DrlgRoomKind::Preset, TileRect::new(0, 0, 8, 8));
            drlg.room_mut(r).dt1_mask = 1;
            drlg.link_room(r, LinkAt::Tail);
        }
        Ok(())
    }
    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        let r = drlg.room(room).rect;
        let (w, h) = (r.w as usize + 1, r.h as usize + 1);
        let mut g = CellGrid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                g.set(x, y, d2_sim::drlg::tiles::cell::FLOOR);
            }
        }
        Ok(RoomGrids {
            passes: vec![GridPass {
                cells: g,
                orientation: None,
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        })
    }
}

struct FieldTiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for FieldTiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tile(orientation: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation,
        main,
        sub,
        rarity,
        material: 0,
        subtile_flags: [0; 25],
        roof_height: 0,
        height: 0,
        light_direction: 0,
    }
}

/// Act 0 with the Cold Plains room (the server's handler tests' field
/// DRLG).
fn field_drlg() -> DrlgWorld {
    use d2_sim::drlg::tiles::FIXED_LIBRARY;
    let mut data = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut data.levels {
        l.warp = [-1; 8];
    }
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    data.lvltypes = vec![vec![Vec::new(); 32], files];
    data.levels[COLD_PLAINS as usize].drlg_type = 2;
    data.levels[COLD_PLAINS as usize].level_type = 1;
    let mut types = FieldTypes;
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    let mut t = BTreeMap::new();
    t.insert(b"floor.dt1".to_vec(), vec![tile(0, 0, 0, 1)]);
    let blank = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(FIXED_LIBRARY[0].to_vec(), vec![blank(0), blank(1)]);
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(FieldTiles(t)),
        types: Box::new(types),
    }
}

/// The Cold Plains room, generated and streamed.
fn field_room<X: Pending>(sim: &mut ActionSim<X>, game: &mut Game) -> RoomId {
    game.lists.ensure_act(0).unwrap();
    sim.hooks()
        .drlg
        .with_act(0, &mut game.lists, |d, svc| {
            let l = d.get_or_alloc_level(svc.data, svc.types, COLD_PLAINS)?;
            d.generate_level(svc.data, svc.types, l)?;
            let r = d.level_rooms(l)[0];
            d.stream_room(svc, r)
        })
        .unwrap()
        .unwrap()
        .unwrap()
}

// ---- tables --------------------------------------------------------------------------------

/// Item records (combined index; the vendor, cube and inventory tables
/// use the same order).
const CAP: usize = 0;
const BUC: usize = 1;
const CUBE: usize = 2;
const RING: usize = 3;
const AMULET: usize = 4;
const T_RING: u16 = 10;
const T_BOX: u16 = 11;
const T_AMULET: u16 = 12;
/// (code, type, invwidth, invheight) per record.
const ITEMS: [([u8; 4], u16, u8, u8); 5] = [
    (*b"cap ", ty::HELM, 2, 2),
    (*b"buc ", ty::SHIE, 2, 2),
    (*b"box ", T_BOX, 2, 2),
    (*b"rin ", T_RING, 1, 1),
    (*b"amu ", T_AMULET, 1, 1),
];

/// Every type is its own and type 0's; helm and shield are armor; ring,
/// cube and amulet are misc.
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
    set(usize::from(ty::SHIE), usize::from(ty::ARMO));
    for c in [T_RING, T_BOX, T_AMULET] {
        set(usize::from(c), usize::from(ty::MISC));
    }
    m
}

fn item_tables() -> ItemTables {
    let mut ratio: Itemratio = blank();
    ratio.version = 0;
    let rec = |&(code, t, _, _): &([u8; 4], u16, u8, u8)| {
        let armor = t == ty::HELM || t == ty::SHIE;
        ItemRec {
            code,
            type_: t as i16,
            level: 1,
            durability: if armor { 12 } else { 0 },
            minac: if armor { 3 } else { 0 },
            maxac: if armor { 5 } else { 0 },
            ..ItemRec::default()
        }
    };
    ItemTables {
        items: ITEMS.iter().map(rec).collect(),
        itemtypes: (0..N_TYPES)
            .map(|_| {
                let mut t: Itemtypes = blank();
                t.class = 0xFF;
                t.staffmods = 0xFF;
                // Empty `shoots`: the link miss (link16 −1).
                t.shoots = 0xFFFF;
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

/// Akara's column: the cap (permanent) and the buckler (1–3); the cube,
/// ring and amulet are records she never stocks.
fn vendor_tables() -> VendorTables {
    let item = |&(code, t, _, _): &([u8; 4], u16, u8, u8)| VendorItem {
        code,
        normcode: code,
        ubercode: NO_CODE,
        ultracode: NO_CODE,
        cost: 100,
        type_: t as i16,
        type2: -1,
        level: 1,
        spawnable: 1,
        durability: 12,
        minac: 3,
        maxac: 5,
        nightmare_upgrade: XXX,
        hell_upgrade: XXX,
        ..VendorItem::default()
    };
    let mut items: Vec<VendorItem> = ITEMS.iter().map(item).collect();
    items[CAP].perm_store = 1;
    items[CAP].columns[0] = [1, 1, 0, 0, 0];
    items[BUC].cost = 80;
    items[BUC].columns[0] = [1, 3, 0, 0, 0];
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
        items,
        itemtypes,
        equiv: equiv(),
        stat_shift: 6,
        stat_mask: 0x3F,
        monster_levels: vec![[1, 1, 1]; N_MONSTATS],
        interact: vec![class::AKARA],
        npc: vec![NpcPrices {
            class: u32::from(class::AKARA),
            sell: 1024,
            buy: 512,
            rep: 128,
            quests: [(0, 0, 0, 0); 3],
            max_buy: [5000; 3],
        }],
        difficulty: vec![Default::default(); 3],
        ..VendorTables::default()
    }
}

fn monstats() -> Vec<Monstats> {
    let mut v: Vec<Monstats> = (0..N_MONSTATS).map(|_| blank()).collect();
    v[usize::from(class::AKARA)].npc = true;
    v[usize::from(class::AKARA)].interact = true;
    v
}

/// The cube recipe shape without mods: one ring → a normal amulet.
fn cube_data(t: &ItemTables) -> CubeData {
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
    let recipe = Recipe {
        enabled: 1,
        class: 0xFF,
        numinputs: 1,
        inputs,
        outputs: [out, OutputSlot::default(), OutputSlot::default()],
        ..Recipe::default()
    };
    CubeData {
        recipes: vec![recipe],
        items: t
            .items
            .iter()
            .map(|r| ItemRecord {
                code: r.code,
                level: r.level,
                spawnable: 1,
                ..ItemRecord::default()
            })
            .collect(),
        valshift: vec![0; N_STATS],
        max_level: 99,
    }
}

/// Inventory tables (grid records as measured:
/// player classes 10 × 4, the cube 3 × 4, the stash 6 × 4 / 6 × 8, ...);
/// helms on the head, shields in either hand, rings on either ring
/// finger, amulets on the neck.
fn inv_tables() -> InvTables {
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
    for (t, loc1, loc2) in [
        (ty::HELM, 1, 1),
        (ty::SHIE, 5, 4),
        (T_RING, 6, 7),
        (T_AMULET, 2, 2),
    ] {
        let r = &mut itemtypes[usize::from(t)];
        r.body = 1;
        r.bodyloc1 = loc1;
        r.bodyloc2 = loc2;
    }
    InvTables {
        grids,
        belts: vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16],
        items: ITEMS
            .iter()
            .map(|&(code, t, w, h)| InvItemRec {
                code,
                type_: t as i16,
                invwidth: w,
                invheight: h,
                ..InvItemRec::default()
            })
            .collect(),
        itemtypes,
        equiv: equiv(),
        books: Vec::new(),
        item_use: Default::default(),
    }
}

fn action_tables() -> ActionTables {
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

// ---- seams ---------------------------------------------------------------------------------

/// The action wiring's seams: `Pending`'s defaults; sends kept.
#[derive(Default)]
struct ActionRest {
    sent: Vec<(UnitId, Vec<u8>)>,
}

impl Pending for ActionRest {
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
}

impl Outbox for ActionRest {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// The vendors' player-inventory calls `WiredWorld` answers from its
/// inventory model (`InvVendors`) before they reach a rest.
const MODEL: &str = "WiredWorld answers from the inventory model";

/// The interaction seams staged by the test, as in
/// `e2e_vendor.rs` (talk range, room in the NPC grid, no item copy). The
/// NPC store and gamble lists are kept here (the NPC grid is the
/// rest's), so the properties can find every store item.
#[derive(Default)]
struct Rest {
    quests: BTreeMap<UnitId, PlayerQuests>,
    last_bought: BTreeMap<UnitId, u32>,
    sent: Vec<(UnitId, Vec<u8>)>,
    /// Items in an NPC store or gamble list.
    store: BTreeSet<UnitId>,
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
        unreachable!("{MODEL}")
    }
    /// The item copy is null here (nothing writes it).
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
    /// The NPC grid: always room.
    fn place_in_store(&mut self, _: u16, item: UnitId) -> bool {
        self.store.insert(item);
        true
    }
    fn remove_store_item(&mut self, _: u16, item: UnitId) {
        self.store.remove(&item);
    }
    /// Removed from the grid, then re-added to the trade inventory; the
    /// host frees the unit after its 0x9C action 12 (the Akara buy flow),
    /// so it leaves the store's list.
    fn take_from_store(&mut self, _: u16, item: UnitId) {
        self.store.remove(&item);
    }
    fn place_in_gamble(&mut self, _: u16, _: u32, item: UnitId) -> bool {
        self.store.insert(item);
        true
    }
    fn remove_gamble_item(&mut self, _: u16, _: u32, item: UnitId) {
        self.store.remove(&item);
    }
    fn refresh_npc_inventory(&mut self, _: UnitId) {}
    fn add_trade_inventory(&mut self, _: u16, _: UnitId) {}
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
    fn unhandled(&mut self, _: u8, _: u32) {}
}

/// [`ItemPending`] stand-in: nothing happens (no items spec writes these).
struct CubeRest;

impl ItemPending for CubeRest {
    fn inventory_pass(&mut self, _: UnitId, _: &mut Vec<Vec<u8>>) {}
    fn duplicate(&mut self, _: UnitId, _: bool) -> Option<UnitId> {
        None
    }
    fn tempered_affix(&mut self, _: UnitId, _: bool) -> u16 {
        0
    }
    fn drop_runeword_stats(&mut self, _: UnitId) {}
    fn repair(&mut self, _: UnitId) {}
    fn recharge(&mut self, _: UnitId) {}
    fn quest_item_hook(&mut self, _: UnitId, _: UnitId, _: [u8; 4]) {}
    fn cow_portal(&mut self, _: UnitId) -> bool {
        false
    }
}

/// The item-move seams no d2-sim module provides, staged as
/// `e2e_single_player.rs` stages them: distance 3 (< 5: pick-ups reach),
/// the free-spot search answers the start spot in the player's room, the
/// narrowest requirement / hands / auto-equip answers. Sends collected.
#[derive(Default)]
struct InvFx {
    room: Option<RoomId>,
    pos: BTreeMap<Owner, (i32, i32)>,
    sent: Vec<(Owner, Vec<u8>)>,
}

impl MovePending for InvFx {
    fn distance(&self, _: Owner, _: Owner) -> i32 {
        3
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
        self.room.map(|room| Spot {
            room,
            x: start.0,
            y: start.1,
        })
    }
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.sent.push((player, bytes));
    }
}

impl InvRest for InvFx {
    fn pos(&self, u: Owner) -> (i32, i32) {
        self.pos.get(&u).copied().unwrap_or((0, 0))
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        self.pos.insert(u, (x, y));
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
        std::mem::take(&mut self.sent)
    }
}

// ---- the host ------------------------------------------------------------------------------

type Sim = SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>;

/// The player's position (subtiles) in the field room.
const AT: (i32, i32) = (20, 20);

struct Host {
    sim: Sim,
    out: ClientBuffers,
    player: UnitId,
    npc: UnitId,
    cube: UnitId,
    room: RoomId,
    /// GUID of every item unit seen live, by unit.
    guids: BTreeMap<UnitId, u32>,
    /// Items left in [`Place::Limbo`], with their mode.
    limbo: BTreeMap<UnitId, u8>,
}

fn facts() -> UnitFacts {
    UnitFacts {
        act: 0,
        pos: Pos { x: AT.0, y: AT.1 },
        owner: None,
    }
}

/// The wired host: game creation on the action sim (expansion, the game
/// seed), the NPC control and quests on the game seed, Akara and the
/// player (class 1, 5000 gold) in the field room for client 0; the
/// vendor tables, the cube's parts and the inventory model on
/// `WiredWorld`; the player's buckler, cap and cube stored, a
/// ring and a cap on the ground beside the player.
fn host(game_seed: u32) -> Host {
    let hooks = ActionHooks::new(
        Arc::new(action_tables()),
        field_drlg(),
        Seed::init_low(game_seed),
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
    events.create_game(&GameFields::new(Seed::init_low(game_seed), true));
    let mut game = Game::new();
    let room = field_room(&mut events, &mut game);
    let mut seed = events.hooks().game_seed;
    let ctl = NpcControl::new(&monstats(), Vec::new(), false, 0, &mut seed).expect("npc");
    let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
    events.hooks().game_seed = seed;
    let mut alloc = |ty, class, room| {
        let req = AllocRequest {
            ty,
            class,
            room,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        events
            .with(&mut game, |g, v| v.allocate(g, &req, AT.0, AT.1))
            .expect("allocated")
    };
    // Akara outside the rooms (as `e2e_vendor.rs` allocates her): the
    // field room's tick would run her monster modes, which no table here
    // gives (her talk needs a mode other than 0 and 12).
    let npc = alloc(UnitType::Monster, u32::from(class::AKARA), None);
    let player = alloc(UnitType::Player, 1, Some(room));
    events.sys.units.get_mut(player).unwrap().mode = 1;
    events.with(&mut game, |_, v| {
        v.set_base(player, 12, 1);
        v.set_base(player, GOLD, PLAYER_GOLD);
        // Strength and dexterity ≥ 1 (the equip requirements).
        v.set_base(player, 0, 15);
        v.set_base(player, 2, 15);
    });
    let mut rest = Rest::default();
    rest.quests.insert(player, PlayerQuests::default());
    let mut world = WiredWorld::new(
        ActionWorld::default(),
        item_tables(),
        quests,
        ctl,
        vendor_tables(),
        rest,
        1000,
    );
    world.state.add_npc(npc);
    let pg = game.lists.unit(player).unwrap().guid;
    let mut fx = InvFx {
        room: Some(room),
        ..InvFx::default()
    };
    fx.pos.insert(Owner::player(pg), AT);
    let mut parts = InvParts::new(inv_tables(), Box::new(fx));
    parts
        .state
        .add_inventory(player, InvKind::Player { class: 1 }, pg);
    world.inventory = Some(parts);
    let mut cube_parts = CubeParts::new(cube_data(&world.tables), Box::new(CubeRest));
    cube_parts.staged.local_date = (15, 3);
    world.cube = Some(cube_parts);
    let sim = SimGame::with_world(game, events, world);
    let mut h = Host {
        sim,
        out: ClientBuffers::new(),
        player,
        npc,
        cube: player,
        room,
        guids: BTreeMap::new(),
        limbo: BTreeMap::new(),
    };
    let (buc, cap, cube) = (h.make(BUC, 4), h.make(CAP, 4), h.make(CUBE, 4));
    for item in [buc, cap, cube] {
        h.store(item);
    }
    h.cube = cube;
    h.ground(RING);
    h.ground(CAP);
    h.sim
        .join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    h.out.add_client(0);
    h.sim.set_player(
        player,
        PlayerFields {
            gate: ALIVE,
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    h.sim.set_unit(player, facts());
    h.sim.set_unit(npc, facts());
    h
}

impl Host {
    fn guid(&self, u: UnitId) -> u32 {
        self.sim.game.lists.unit(u).unwrap().guid
    }

    fn guid_or_none(&self, u: UnitId) -> u32 {
        self.sim.game.lists.unit(u).map_or(u32::MAX, |e| e.guid)
    }

    fn inv(&self) -> &InvParts {
        self.sim.world.inventory.as_ref().unwrap()
    }

    /// An item of `record` made by the economy wiring on the game seed in
    /// the game's one store (normal quality, ilvl 5), in `mode` (3: on the
    /// ground in the field room).
    fn make(&mut self, record: usize, mode: u32) -> UnitId {
        let room = (mode == 3).then_some(self.room);
        let sim = &mut self.sim;
        let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
        let item = world.with_economy(game, events, |econ, _| {
            let mut rq = ItemRequest {
                item: record as i32,
                ilvl: 5,
                quality: q::NORMAL,
                format: 1,
                ..ItemRequest::default()
            };
            let spawn = ItemSpawn {
                room,
                mode,
                init_flags: 1,
            };
            econ.create_item(&mut rq, false, spawn).expect("item")
        });
        // Identified, as the e2e's items (equipping needs it).
        let items = &mut self.sim.events.sys.hooks.items;
        items.get_mut(item).unwrap().flags |= flag::IDENTIFIED;
        if room.is_some() {
            self.sim.set_unit(item, facts());
        }
        item
    }

    /// A ground item beside the player, as a drop creates it (the drop's
    /// item creation into the one store, mode 3 in the room); its item
    /// data position beside the player (the drop fixture's reading).
    fn ground(&mut self, record: usize) -> UnitId {
        let item = self.make(record, 3);
        let g = self.guid(item);
        self.sim
            .world
            .inventory
            .as_mut()
            .unwrap()
            .state
            .items
            .insert(
                item,
                InvItem {
                    x: AT.0 + 1,
                    y: AT.1 + 1,
                    ..InvItem::new(g, record)
                },
            );
        item
    }

    /// Puts `item` (on the cursor) on page 0 of the player's inventory
    /// through the store placement (a free position, no "send"): a loaded
    /// character's.
    fn store(&mut self, item: UnitId) {
        let player = self.player;
        let s = &mut self.sim;
        s.events.sys.units.get_mut(item).unwrap().mode = 4;
        s.events.sys.hooks.items.get_mut(item).unwrap().inv_page = 0;
        let (game, events, world) = (&mut s.game, &mut s.events, &mut s.world);
        let placed = world.with_economy(game, events, |econ, p| {
            let inv = p.inventory.as_deref_mut().expect("inventory parts");
            inv.desk(econ).place(player, item, (0, 0), true, false)
        });
        assert!(placed, "stored");
    }

    /// The player's update list (GUIDs).
    fn update_list(&self) -> Vec<u32> {
        self.inv()
            .state
            .of(self.player)
            .map_or_else(Vec::new, |i| i.update_list().to_vec())
    }

    fn live_items(&self) -> Vec<UnitId> {
        let mut v = self.sim.game.lists.units_of_type(UnitType::Item);
        v.sort();
        v
    }

    /// Everything client 0 received since the last drain, split into
    /// messages by the S→C size rule (a buffer
    /// that does not split whole is a stream fault).
    fn drain(&mut self) -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        while let Some(b) = self.out.pop(0) {
            let split = split_server_buffer(&b).expect("S→C buffer splits");
            assert!(split.discarded.is_empty(), "S→C bytes lost: {b:02X?}");
            v.extend(split.messages.iter().map(|m| m.to_vec()));
        }
        v
    }

    /// One frame: the op's messages through the dispatcher, then one host
    /// tick (with its update pass). Returns (dispatch-phase, tick-phase)
    /// messages to the client.
    fn frame(&mut self, op: &Op) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut msgs = self.messages(op);
        for m in &mut msgs {
            // Trailing pad bytes up to the id's fixed size
            // (`client-messages.tsv` transport size).
            let size = CLIENT_MESSAGES[usize::from(m[0])].transport_size.fixed();
            let size = size.expect("fixed-size id");
            assert!(m.len() <= size, "{m:02X?}: longer than {size}");
            m.resize(size, 0);
        }
        for m in &msgs {
            dispatch(
                &mut self.sim,
                &ProtoSizes,
                &mut self.out,
                0,
                ALIVE,
                m,
                m.len(),
            );
        }
        if let Op::Transmute = op {
            // The cube closed again (unspecified as its opening): the
            // staged interaction ends, so a later NPC talk can start.
            let cube = (4, self.guid_or_none(self.cube));
            let rec = self.sim.events.sys.units.get_mut(self.player).unwrap();
            if rec.interact.get() == Some(cube) {
                rec.interact.reset();
            }
        }
        let direct = self.drain();
        self.sim.tick(&mut self.out);
        (direct, self.drain())
    }

    /// The GUID pool: live items (unit order), the player, Akara, none.
    fn pick(&self, sel: u8) -> u32 {
        let items = self.live_items();
        let n = items.len() + 3;
        match sel as usize % n {
            i if i < items.len() => self.guid(items[i]),
            i if i == items.len() => self.guid(self.player),
            i if i == items.len() + 1 => self.guid(self.npc),
            _ => u32::MAX,
        }
    }

    /// A GUID for a message field that names an item in `role`: with
    /// `sel % 4 == 0` any GUID of the pool ([`Host::pick`] of `sel / 4`),
    /// else one of the items in that role (the cursor item, a stored item,
    /// a ground item), so most messages get
    /// past their first checks.
    fn aim(&self, role: Role, sel: u8) -> u32 {
        if sel.is_multiple_of(4) {
            return self.pick(sel / 4);
        }
        let state = &self.inv().state;
        let p = self.player;
        let of: Vec<UnitId> = match role {
            Role::Cursor => state.cursor_of(p).into_iter().collect(),
            Role::Stored => state
                .items_of(p)
                .into_iter()
                .filter(|u| state.items.get(u).is_some_and(|d| d.node_grid > 2))
                .collect(),
            Role::Ground => self
                .live_items()
                .into_iter()
                .filter(|&u| self.sim.game.lists.unit(u).unwrap().room().is_some())
                .collect(),
        };
        match of.get(usize::from(sel / 4) % of.len().max(1)) {
            Some(&u) => self.guid(u),
            None => self.pick(sel / 4),
        }
    }

    /// A body location: mostly one the item's type allows (helm 1,
    /// shield 4 / 5, ring 6 / 7, amulet 2) or an occupied one.
    fn body_loc(&self, item: Option<UnitId>, x: u8) -> u8 {
        let rec = item.and_then(|u| self.sim.events.sys.hooks.items.get(u).map(|i| i.record));
        match (x % 4, rec) {
            (0, _) => x % 14,
            (_, Some(CAP)) => 1,
            (_, Some(BUC)) => 4 + x % 2,
            (_, Some(RING)) => 6 + x % 2,
            (_, Some(AMULET)) => 2,
            _ => {
                let state = &self.inv().state;
                let locs: Vec<u8> = state
                    .body_items(self.player)
                    .iter()
                    .filter_map(|u| state.items.get(u).map(|d| d.body_loc))
                    .collect();
                locs.get(usize::from(x) % locs.len().max(1))
                    .copied()
                    .unwrap_or(x % 14)
            }
        }
    }

    fn messages(&mut self, op: &Op) -> Vec<Vec<u8>> {
        let le = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        let npc = self.guid(self.npc);
        match *op {
            Op::Spawn(record) => {
                self.ground(record as usize % ITEMS.len());
                Vec::new()
            }
            Op::OpenTrade => {
                let mut talk = vec![0x13, 1, 0, 0, 0];
                le(&mut talk, npc);
                let mut chat = vec![0x2F, 1, 0, 0, 0];
                le(&mut chat, npc);
                let mut trade = vec![0x38, 1, 0, 0, 0];
                le(&mut trade, npc);
                le(&mut trade, 0);
                vec![talk, chat, trade]
            }
            Op::Transmute => {
                // The cube's opening (item use) is not driven here: the
                // interaction (type 4, the cube) is staged.
                let cg = self.guid_or_none(self.cube);
                if cg != u32::MAX {
                    let rec = self.sim.events.sys.units.get_mut(self.player).unwrap();
                    rec.interact.reset();
                    rec.interact.set(4, cg);
                }
                vec![vec![0x4F, 0x18, 0, 0, 0, 0, 0]]
            }
            Op::Msg { id, a, b, x, y } => {
                let mut m = vec![id];
                let cursor = self.inv().state.cursor_of(self.player);
                let held = self.aim(Role::Cursor, a);
                let stored = self.aim(Role::Stored, a);
                match id {
                    0x16 => {
                        le(&mut m, if x % 8 == 7 { u32::from(y) % 6 } else { 4 });
                        le(&mut m, self.aim(Role::Ground, a));
                        // Mostly to the cursor; a quarter auto.
                        le(&mut m, u32::from(y % 4 != 0));
                    }
                    0x17 => le(&mut m, held),
                    0x19 | 0x22 | 0x63 => le(&mut m, stored),
                    0x24 => le(&mut m, self.pick(a)),
                    0x18 => {
                        // Mostly the inventory, the cube and the stash.
                        let page = [0u8, 0, 3, 4, 1, 5][usize::from(b % 6)];
                        let (w, hh) = match page {
                            3 => (3, 4),
                            4 => (6, 8),
                            _ => (10, 4),
                        };
                        le(&mut m, held);
                        le(&mut m, u32::from(x % w));
                        le(&mut m, u32::from(y % hh));
                        le(&mut m, u32::from(page));
                    }
                    0x1A | 0x1B | 0x1D | 0x1E => {
                        le(&mut m, held);
                        // bodyloc: u8 @5, then pad to the size (9).
                        le(&mut m, u32::from(self.body_loc(cursor, x)));
                    }
                    0x1C | 0x61 => {
                        let loc = self.body_loc(None, x);
                        m.extend_from_slice(&u16::from(loc).to_le_bytes());
                    }
                    0x1F => {
                        le(&mut m, held);
                        le(&mut m, self.aim(Role::Stored, b));
                        le(&mut m, u32::from(x % 10));
                        le(&mut m, u32::from(y % 4));
                    }
                    0x20 => {
                        le(&mut m, stored);
                        le(&mut m, AT.0 as u32 + u32::from(x % 3));
                        le(&mut m, AT.1 as u32 + u32::from(y % 3));
                    }
                    0x23 => {
                        le(&mut m, held);
                        le(&mut m, u32::from(x % 17));
                    }
                    0x26 => {
                        le(&mut m, self.pick(a));
                        le(&mut m, u32::from(x & 1));
                    }
                    0x21 | 0x25 | 0x27 | 0x28 | 0x29 => {
                        le(&mut m, held);
                        le(&mut m, self.aim(Role::Stored, b));
                    }
                    0x2A => {
                        // The cursor item or a ground item, into the cube
                        // (mostly the real one).
                        let it = if b % 2 == 0 {
                            held
                        } else {
                            self.aim(Role::Ground, a)
                        };
                        le(&mut m, it);
                        let c = if x % 4 == 0 {
                            self.pick(b)
                        } else {
                            self.guid_or_none(self.cube)
                        };
                        le(&mut m, c);
                    }
                    0x50 => {
                        let unit = if x % 4 == 0 {
                            self.pick(a)
                        } else {
                            self.guid(self.player)
                        };
                        le(&mut m, unit);
                        le(&mut m, u32::from(y) * 37);
                    }
                    0x32 => {
                        le(&mut m, npc);
                        // Mostly an NPC store item.
                        let store: Vec<UnitId> =
                            self.sim.world.rest.store.iter().copied().collect();
                        let it = match store.get(usize::from(a) % (store.len() + 1)) {
                            Some(&u) => self.guid(u),
                            None => self.pick(a),
                        };
                        le(&mut m, it);
                        le(&mut m, u32::from(x % 2));
                        le(&mut m, 0);
                    }
                    0x33 => {
                        le(&mut m, npc);
                        le(&mut m, stored);
                        // tab: u16 @9, two pad bytes, cost: u32 @13.
                        le(&mut m, 0);
                        le(&mut m, 0);
                    }
                    _ => unreachable!("op id {id:#x}"),
                }
                vec![m]
            }
        }
    }
}

// ---- ops -----------------------------------------------------------------------------------

/// What a message field names (see [`Host::aim`]).
#[derive(Clone, Copy)]
enum Role {
    Cursor,
    Stored,
    Ground,
}

#[derive(Clone, Debug)]
enum Op {
    /// One C→S message: `a`, `b` aim GUIDs ([`Host::aim`]), `x`, `y` small
    /// fields (positions, pages, body locations, flags).
    Msg { id: u8, a: u8, b: u8, x: u8, y: u8 },
    /// Talk, chat and trade (0x13, 0x2F, 0x38 action 1) with Akara.
    OpenTrade,
    /// C→S 0x4F button 0x18 with the cube open (staged).
    Transmute,
    /// A ground item created beside the player (a drop's creation).
    Spawn(u8),
}

const MOVE_IDS: [u8; 23] = [
    0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25,
    0x26, 0x27, 0x28, 0x29, 0x50, 0x61, 0x63,
];

fn op() -> impl Strategy<Value = Op> {
    let msg = |ids: Vec<u8>| {
        (
            prop::sample::select(ids),
            any::<u8>(),
            any::<u8>(),
            any::<u8>(),
            any::<u8>(),
        )
            .prop_map(|(id, a, b, x, y)| Op::Msg { id, a, b, x, y })
    };
    prop_oneof![
        // The common moves: pick, drop, place, lift, equip, unequip, swap.
        6 => msg(vec![0x16, 0x16, 0x17, 0x18, 0x18, 0x19, 0x1A, 0x1C, 0x1D, 0x1F]),
        3 => msg(MOVE_IDS.to_vec()),
        2 => msg(vec![0x32, 0x33, 0x33]),
        2 => msg(vec![0x2A]),
        1 => Just(Op::OpenTrade),
        1 => Just(Op::Transmute),
        1 => (0u8..5).prop_map(Op::Spawn),
    ]
}

// ---- invariants ----------------------------------------------------------------------------

/// Where an item is (a grid, the cursor, the room or
/// a vendor's lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// (owner, page).
    Page(UnitId, u8),
    Belt(UnitId),
    Body(UnitId),
    Cursor(UnitId),
    Ground,
    Store,
    /// In no inventory, no cursor and no room, on four paths
    /// (`docs/handoff/prop-unified-items.md`), with the mode they leave:
    /// a failed 0x63 slot placement (mode 4); a failed 0x1F placement of
    /// the cursor item after the target took the cursor (Q1; mode 4); the
    /// store placement's "cursor := none" when the placed item is not the
    /// cursor item (0x2A of a ground item, a transmute's output) while
    /// the player holds one (Q2; mode 4); an auto pick-up whose
    /// auto-equip step ran with the item already out of its room (Q3;
    /// mode 3).
    Limbo(u8),
}

impl Place {
    /// The unit mode the place implies (the `mode` constants).
    fn mode(self) -> Option<u8> {
        match self {
            Place::Page(..) => Some(mode::STORED),
            Place::Body(_) => Some(mode::EQUIPPED),
            Place::Belt(_) => Some(mode::BELT),
            Place::Ground => Some(mode::GROUND),
            Place::Cursor(_) => Some(mode::CURSOR),
            Place::Limbo(m) => Some(m),
            Place::Store => None,
        }
    }
}

fn size_of(record: usize, grid: usize) -> (i32, i32) {
    if grid < grid_id::PAGE {
        return (1, 1);
    }
    let (_, _, w, h) = ITEMS[record];
    (i32::from(w), i32::from(h))
}

/// The grids of one inventory: cells, lists, node fields. Returns each listed item's place.
fn check_inventory(
    h: &Host,
    inv: &Inventory,
    places: &mut BTreeMap<UnitId, Vec<Place>>,
) -> Result<(), String> {
    let state = &h.inv().state;
    let owner = inv.owner;
    let mut in_grids = Vec::new();
    for g in 0..inv.grid_count() {
        let Some(grid) = inv.grid(g) else { continue };
        let mut expect = vec![None; grid.cells.len()];
        for &it in &grid.items {
            if in_grids.contains(&it) {
                return Err(format!("{it:?} in two grids of {owner:?}"));
            }
            in_grids.push(it);
            let d = state
                .items
                .get(&it)
                .ok_or(format!("{it:?} in grid {g} without item data"))?;
            if usize::from(d.node_grid) != g + 1 {
                return Err(format!("{it:?} in grid {g}, node grid {}", d.node_grid));
            }
            if d.inv != Some(owner) {
                return Err(format!("{it:?} in {owner:?}'s grid, owning {:?}", d.inv));
            }
            let (w, hh) = size_of(d.record, g);
            for yy in d.y..d.y + hh {
                for xx in d.x..d.x + w {
                    if xx < 0
                        || yy < 0
                        || xx >= i32::from(grid.width)
                        || yy >= i32::from(grid.height)
                    {
                        return Err(format!("{it:?} at ({}, {}) out of grid {g}", d.x, d.y));
                    }
                    let c = &mut expect[yy as usize * usize::from(grid.width) + xx as usize];
                    if let Some(other) = *c {
                        return Err(format!("{it:?} overlaps {other:?} in grid {g}"));
                    }
                    *c = Some(it);
                }
            }
            let place = match g {
                grid_id::BODY => Place::Body(owner),
                grid_id::BELT => Place::Belt(owner),
                _ => Place::Page(owner, (g - grid_id::PAGE) as u8),
            };
            if g >= grid_id::PAGE && usize::from(d.page) != g - grid_id::PAGE {
                return Err(format!("{it:?} in grid {g}, page {}", d.page));
            }
            places.entry(it).or_default().push(place);
        }
        if grid.cells != expect {
            return Err(format!(
                "grid {g} of {owner:?}: cells {:?}, items imply {:?}",
                grid.cells, expect
            ));
        }
    }
    // The item list is every grid's items, in link order.
    let mut listed = inv.items().to_vec();
    let mut sorted = listed.clone();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != listed.len() {
        return Err(format!("{owner:?} lists an item twice: {listed:?}"));
    }
    listed.sort();
    in_grids.sort();
    if listed != in_grids {
        return Err(format!(
            "{owner:?}: item list {:?}, grid items {in_grids:?}",
            inv.items()
        ));
    }
    if inv.count as usize != inv.items().len() {
        return Err(format!(
            "{owner:?}: count {} for {} linked items",
            inv.count,
            inv.items().len()
        ));
    }
    if let Some(c) = inv.cursor() {
        if inv.contains(c) {
            return Err(format!("{owner:?}: cursor item {c:?} is linked in a grid"));
        }
        places.entry(c).or_default().push(Place::Cursor(owner));
    }
    // The host API's reads (`wiring::inventory::host`) agree.
    if state.items_of(owner) != inv.items() || state.cursor_of(owner) != inv.cursor() {
        return Err(format!(
            "{owner:?}: InvState reads differ from the inventory"
        ));
    }
    let body: Vec<UnitId> = inv
        .grid(grid_id::BODY)
        .map(|g| g.cells.iter().flatten().copied().collect())
        .unwrap_or_default();
    if state.body_items(owner) != body {
        return Err(format!("{owner:?}: body_items differs from grid 0"));
    }
    Ok(())
}

/// Invariants 1–4 of the module doc; returns each live item's place.
/// `quirk`: the mode a path of the frame may leave an item in
/// [`Place::Limbo`] with (see [`limbo_mode`]).
fn check_state(h: &mut Host, quirk: Option<u8>) -> Result<BTreeMap<UnitId, Place>, String> {
    let live = h.live_items();
    let s = &h.sim;
    let store = &s.events.sys.hooks.items;
    // 1. The one store holds exactly the live items' item data.
    if store.len() != live.len() {
        return Err(format!(
            "item store holds {} entries for {} live items",
            store.len(),
            live.len()
        ));
    }
    let mut seen = BTreeMap::new();
    for &u in &live {
        if !store.contains(u) {
            return Err(format!("live item {u:?} has no item data in the store"));
        }
        let g = s.game.lists.unit(u).unwrap().guid;
        // 4. GUIDs distinct and stable.
        if let Some(other) = seen.insert(g, u) {
            return Err(format!("GUID {g} live on {other:?} and {u:?}"));
        }
    }
    let inv = h.inv();
    // 4. No inventory or item data refers to a freed unit.
    for (&owner, i) in &inv.state.inventories {
        // An NPC holds its store on an inventory (q-fix-server-store-fill):
        // the owner must still be a unit of the game, not only an item.
        if owner != h.player && !live.contains(&owner) && s.game.lists.unit(owner).is_none() {
            return Err(format!("inventory of freed {owner:?}"));
        }
        for it in i.items().iter().chain(i.cursor().iter()) {
            if !live.contains(it) {
                return Err(format!("{owner:?} holds freed {it:?}"));
            }
        }
    }
    for u in inv.state.items.keys() {
        if !live.contains(u) {
            return Err(format!("item data of freed {u:?} in the inventory state"));
        }
    }
    for u in &s.world.rest.store {
        if !live.contains(u) {
            return Err(format!("store lists freed {u:?}"));
        }
    }
    // 2, 3. Grids, lists, cursor.
    let mut places: BTreeMap<UnitId, Vec<Place>> = BTreeMap::new();
    for i in inv.state.inventories.values() {
        check_inventory(h, i, &mut places)?;
    }
    for &u in &live {
        let e = s.game.lists.unit(u).unwrap();
        if e.room().is_some() {
            places.entry(u).or_default().push(Place::Ground);
        }
        if s.world.rest.store.contains(&u) {
            places.entry(u).or_default().push(Place::Store);
        }
    }
    // 1. Exactly one place each, its mode matching.
    let mut out = BTreeMap::new();
    let mut limbo = BTreeMap::new();
    for &u in &live {
        let p = places.get(&u).map(Vec::as_slice).unwrap_or(&[]);
        let place = match (p, h.limbo.get(&u).copied().or(quirk)) {
            ([place], _) => *place,
            ([], Some(m)) => {
                limbo.insert(u, m);
                Place::Limbo(m)
            }
            _ => return Err(format!("item {u:?} is in {} places: {p:?}", p.len())),
        };
        let unit_mode = s.events.sys.units.get(u).unwrap().mode;
        if let Some(m) = place.mode() {
            if unit_mode as u8 != m {
                return Err(format!("{u:?} at {place:?} has unit mode {unit_mode}"));
            }
        }
        // 3. The model's copies equal their owners' fields.
        if let Some(d) = inv.state.items.get(&u) {
            let it = store.get(u).unwrap();
            let g = s.game.lists.unit(u).unwrap().guid;
            if d.guid != g || d.record != it.record {
                return Err(format!("{u:?}: item data copy {d:?} vs GUID {g}"));
            }
            if matches!(place, Place::Page(..)) && d.page != it.inv_page {
                return Err(format!(
                    "{u:?}: model page {} vs store page {}",
                    d.page, it.inv_page
                ));
            }
        }
        if let Place::Page(owner, _)
        | Place::Body(owner)
        | Place::Belt(owner)
        | Place::Cursor(owner) = place
        {
            if !inv.state.holds(owner, u) {
                return Err(format!("{owner:?} does not hold {u:?} at {place:?}"));
            }
        }
        out.insert(u, place);
    }
    // A unit id freed and allocated again carries the new GUID; the old
    // one is gone with it (GUIDs are per allocation).
    h.guids = live.iter().map(|&u| (u, h.guid(u))).collect();
    h.limbo = limbo;
    Ok(out)
}

/// The player-held class of a place (cursor, page, belt, body).
fn held(p: Option<&Place>) -> Option<Place> {
    match p {
        Some(&p @ (Place::Page(..) | Place::Belt(_) | Place::Body(_) | Place::Cursor(_))) => {
            Some(p)
        }
        _ => None,
    }
}

/// Invariant 5 of the module doc on one frame.
/// The live items of a frame boundary: place and GUID.
type Snap = BTreeMap<UnitId, (Place, u32)>;

fn snap(h: &Host, places: BTreeMap<UnitId, Place>) -> Snap {
    places
        .into_iter()
        .map(|(u, p)| (u, (p, h.guid(u))))
        .collect()
}

/// `stale`: GUIDs already on the player's update list at the start of
/// the frame (marked by a path that ran no owner refresh, the failed 0x1F
/// swap); the next pass sends them, wherever the item is by then.
fn check_stream(
    h: &Host,
    before: &Snap,
    after: &Snap,
    stale: &[u32],
    (direct, ticked): (&[Vec<u8>], &[Vec<u8>]),
) -> Result<(), String> {
    let pg = h.guid(h.player);
    // By GUID: the item's place at the start and at the end of the frame.
    let mut by_guid: BTreeMap<u32, (Option<Place>, Option<Place>)> = BTreeMap::new();
    for (p, g) in before.values() {
        by_guid.entry(*g).or_default().0 = Some(*p);
    }
    for (p, g) in after.values() {
        by_guid.entry(*g).or_default().1 = Some(*p);
    }
    let live_in_frame = |g: u32| by_guid.contains_key(&g);
    let mut announced = BTreeSet::new();
    let mut last: BTreeMap<u32, (u8, u8)> = BTreeMap::new();
    for (phase, msgs) in [("direct", direct), ("tick", ticked)] {
        for m in msgs {
            if m[0] != 0x9C && m[0] != 0x9D {
                continue;
            }
            if m.len() < 8 || usize::from(m[2]) != m.len() {
                return Err(format!("{phase} {m:02X?}: size byte"));
            }
            let g = u32::from_le_bytes([m[4], m[5], m[6], m[7]]);
            if !live_in_frame(g) {
                return Err(format!(
                    "{phase} {m:02X?}: GUID {g} is no item of the frame"
                ));
            }
            if m[0] == 0x9D {
                let owner = u32::from_le_bytes([m[9], m[10], m[11], m[12]]);
                if m.len() < 13 || m[8] != 0 || owner != pg {
                    return Err(format!("{phase} {m:02X?}: owner is not the player"));
                }
            }
            announced.insert(g);
            if phase == "tick" {
                last.insert(g, (m[0], m[1]));
            }
        }
    }
    // A pass that sent item messages ends with 0x47, 0x48, then at most
    // the player's S→C 0x2C (the sound slot after the item messages). A
    // trade open's store records (0x9C action 11) are not the player's
    // pass: they go out in the client pass with no 0x47 / 0x48 after.
    let store = |m: &Vec<u8>| m[0] == 0x9C && m.get(1) == Some(&0x0B);
    let pass: Vec<Vec<u8>> = ticked.iter().filter(|m| !store(m)).cloned().collect();
    if pass.iter().any(|m| m[0] == 0x9C || m[0] == 0x9D) {
        let n = pass.len() - usize::from(pass.last().is_some_and(|m| m[0] == 0x2C));
        let tail: Vec<u8> = pass[n.saturating_sub(2)..n].iter().map(|m| m[0]).collect();
        if tail != [0x47, 0x48] {
            return Err(format!(
                "update pass without 0x47, 0x48 at its end: {pass:02X?}"
            ));
        }
    }
    // The last deferred message names the item's place at the end.
    for (&g, &(id, action)) in last.iter().filter(|(g, _)| !stale.contains(g)) {
        let Some(&(_, Some(p))) = by_guid.get(&g) else {
            continue;
        };
        let ok = match (id, action) {
            (0x9C, 0x01) | (0x9C, 0x12) => matches!(p, Place::Cursor(_)),
            (0x9C, 0x04) => matches!(p, Place::Page(..)),
            // Both sides of a swap (one ends on the cursor).
            (0x9C, 0x0D) => matches!(p, Place::Page(..) | Place::Cursor(_)),
            (0x9C, 0x0E) => matches!(p, Place::Belt(_)),
            (0x9D, 0x06) => matches!(p, Place::Body(_)),
            (0x9D, 0x05) => !matches!(p, Place::Page(..)),
            (0x9C, 0x0F) => !matches!(p, Place::Belt(_)),
            (0x9D, 0x08) => !matches!(p, Place::Body(_)),
            _ => true,
        };
        if !ok {
            return Err(format!(
                "last message {id:02X} {action:02X} for GUID {g}, at {p:?}"
            ));
        }
    }
    // A player item that moved between held places was announced. Not
    // after a new limbo: the failed 0x1F path runs no owner refresh, so
    // the target's move to the cursor is not sent in that frame.
    // 0x1B (Swap2HandedItem): the two-handed item's partner X goes
    // from the body to the cursor with no command flag and no update-list
    // entry of its own; the client learns it from the 0x9D action 7 (indirect swap) of the item put in its place.
    let swapped_in = direct
        .iter()
        .chain(ticked)
        .any(|m| m[0] == 0x9D && m.get(1) == Some(&7));
    let new_limbo = by_guid
        .values()
        .any(|&(a, b)| matches!(b, Some(Place::Limbo(_))) && !matches!(a, Some(Place::Limbo(_))));
    for (&g, &(a, b)) in by_guid.iter().filter(|_| !new_limbo) {
        let (Some(a), Some(b)) = (held(a.as_ref()), held(b.as_ref())) else {
            continue;
        };
        let kind = |p: Place| std::mem::discriminant(&p);
        let moved = kind(a) != kind(b)
            || matches!((a, b), (Place::Page(_, x), Place::Page(_, y)) if x != y);
        let displaced = swapped_in && matches!((a, b), (Place::Body(_), Place::Cursor(_)));
        if moved && !announced.contains(&g) && !displaced {
            return Err(format!("GUID {g} moved {a:?} → {b:?} with no 0x9C / 0x9D"));
        }
    }
    Ok(())
}

/// The handlers' and wiring's recorded faults (asserts, provider errors).
fn faults(h: &Host) -> Vec<String> {
    let s = &h.sim;
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
    if let Some(c) = &s.world.cube {
        e.extend(c.errors.iter().map(|x| format!("{x:?}")));
    }
    e.extend(h.inv().state.errors.iter().map(|x| format!("{x:?}")));
    e.extend(s.tick_faults.iter().map(|f| format!("{f:?}")));
    e
}

/// A buy (0x32) with t ∉ {0, 2} skips the "item is offered" test, so it can
/// name a ground item; the copy of a ground source is a caller error: the
/// copy is refused (code 9, result 1) and the error recorded. Expected
/// here: nothing moves, and the recorded error names an item that was on
/// the ground before the op; it is taken out of the faults. Any other `GroundCopySource` stays a fault.
fn expect_ground_buy_refusal(
    h: &mut Host,
    op: &Op,
    before: &Snap,
    after: &Snap,
) -> Result<(), String> {
    if !matches!(op, Op::Msg { id: 0x32, .. }) {
        return Ok(());
    }
    let errors = &mut h.sim.world.inventory.as_mut().unwrap().state.errors;
    let mut refused = Vec::new();
    errors.retain(|e| match e {
        InvError::GroundCopySource(u)
            if before.get(u).is_some_and(|(p, _)| *p == Place::Ground) =>
        {
            refused.push(*u);
            false
        }
        _ => true,
    });
    if !refused.is_empty() && before != after {
        return Err(format!("refused buy of ground {refused:?} moved items"));
    }
    Ok(())
}

/// The mode a [`Place::Limbo`] item may be left in by the op's paths.
fn limbo_mode(op: &Op) -> Option<u8> {
    match op {
        Op::Transmute
        | Op::Msg {
            id: 0x1F | 0x2A | 0x63,
            ..
        } => Some(mode::CURSOR),
        Op::Msg { id: 0x16, .. } => Some(mode::GROUND),
        _ => None,
    }
}

/// Runs `ops` one frame each, checking every invariant after each.
fn run(seed: u32, ops: &[Op]) -> Result<(), String> {
    let mut h = host(seed);
    let first = check_state(&mut h, None).map_err(|e| format!("initial: {e}"))?;
    let mut before = snap(&h, first);
    for (i, op) in ops.iter().enumerate() {
        let stale = h.update_list();
        let (direct, ticked) = h.frame(op);
        let after =
            check_state(&mut h, limbo_mode(op)).map_err(|e| format!("op {i} {op:?}: {e}"))?;
        let after = snap(&h, after);
        check_stream(&h, &before, &after, &stale, (&direct, &ticked))
            .map_err(|e| format!("op {i} {op:?}: {e}"))?;
        expect_ground_buy_refusal(&mut h, op, &before, &after)
            .map_err(|e| format!("op {i} {op:?}: {e}"))?;
        let f = faults(&h);
        if !f.is_empty() {
            return Err(format!("op {i} {op:?}: faults {f:?}"));
        }
        before = after;
    }
    Ok(())
}

proptest! {
    #![proptest_config(config(64))]

    /// Random item moves, drops, pick-ups, vendor trades and cube use on
    /// one wired host keep the item store, the inventory model and the
    /// S→C stream consistent.
    #[test]
    fn item_moves_keep_one_place(seed in any::<u32>(), ops in prop::collection::vec(op(), 1..40)) {
        if let Err(e) = run(seed, &ops) {
            return Err(TestCaseError::fail(e));
        }
    }
}

/// The fixture is live: a pick-up, a placement, a sale and a transmute
/// each change the state as the e2e run shows, so the property reaches
/// those paths.
#[test]
fn host_reaches_every_system() {
    let mut h = host(1);
    let start = check_state(&mut h, None).unwrap();
    assert_eq!(
        start.values().filter(|p| **p == Place::Ground).count(),
        2,
        "{start:?}"
    );
    let ring = *start
        .iter()
        .find(|(u, p)| {
            **p == Place::Ground && h.sim.events.sys.hooks.items.get(**u).unwrap().record == RING
        })
        .unwrap()
        .0;
    let rg = h.guid(ring);
    // Ring: picked to the cursor, into the cube, transmuted.
    let pool = h.live_items();
    let sel = 4 * pool.iter().position(|&u| u == ring).unwrap() as u8;
    let ops = [
        Op::Msg {
            id: 0x16,
            a: sel,
            b: 0,
            x: 0,
            y: 1,
        },
        Op::Msg {
            id: 0x2A,
            a: sel,
            b: 0,
            x: 1,
            y: 0,
        },
        Op::Transmute,
    ];
    let mut before = start;
    for op in &ops {
        let b = snap(&h, before.clone());
        let (d, t) = h.frame(op);
        let after = check_state(&mut h, None).unwrap();
        check_stream(&h, &b, &snap(&h, after.clone()), &[], (&d, &t)).unwrap();
        before = after;
    }
    assert!(
        h.sim.game.lists.unit(ring).is_none(),
        "ring {rg} transmuted"
    );
    let amulets: Vec<_> = before
        .iter()
        .filter(|(u, _)| h.sim.events.sys.hooks.items.get(**u).unwrap().record == AMULET)
        .collect();
    assert_eq!(amulets.len(), 1, "{before:?}");
    assert_eq!(*amulets[0].1, Place::Page(h.player, 3));
    // A sale after opening the trade frees the sold item.
    let cap = *before
        .iter()
        .find(|(u, p)| {
            matches!(p, Place::Page(_, 0))
                && h.sim.events.sys.hooks.items.get(**u).unwrap().record == CAP
        })
        .unwrap()
        .0;
    h.frame(&Op::OpenTrade);
    let pool = h.live_items();
    let sel = 4 * pool.iter().position(|&u| u == cap).unwrap() as u8;
    h.frame(&Op::Msg {
        id: 0x33,
        a: sel,
        b: 0,
        x: 0,
        y: 0,
    });
    assert!(h.sim.game.lists.unit(cap).is_none(), "cap sold");
    check_state(&mut h, None).unwrap();
    assert!(faults(&h).is_empty(), "{:?}", faults(&h));
}

/// Counterexample kept (seed 0; `docs/handoff/prop-unified-items.md` Q1):
/// 0x1F with the buckler on the cursor and the stored cap as target at
/// (9, 0), where a 2 × 2 item is out of bounds. The swap takes the
/// target first (unlinked, the cursor, mode 4, command flag 0x40000), then
/// the cursor item's grid placement fails → out 1 → result 3: the buckler
/// is left in mode 4, in no grid and not the cursor item, and no owner
/// refresh sends the cap's move. Pinned as D2RUST behaves today until the
/// question in the handoff file is answered.
#[test]
fn swap_with_a_failed_placement_leaves_the_cursor_item_nowhere() {
    let mut h = host(0);
    let start = check_state(&mut h, None).unwrap();
    let held = |h: &Host, record| {
        *start
            .iter()
            .find(|(u, _)| h.sim.events.sys.hooks.items.get(**u).unwrap().record == record)
            .unwrap()
            .0
    };
    let (buc, cap) = (held(&h, BUC), held(&h, CAP));
    let sel = |h: &Host, u| 4 * h.live_items().iter().position(|&x| x == u).unwrap() as u8;
    let lift = Op::Msg {
        id: 0x19,
        a: sel(&h, buc),
        b: 0,
        x: 0,
        y: 0,
    };
    let (_, t) = h.frame(&lift);
    assert_eq!(t[0][..2], [0x9D, 0x05]);
    let (a, b) = (sel(&h, buc), sel(&h, cap));
    let (d, t) = h.frame(&Op::Msg {
        id: 0x1F,
        a,
        b,
        x: 9,
        y: 0,
    });
    assert!(d.is_empty() && t.is_empty(), "{d:02X?} {t:02X?}");
    let places = check_state(&mut h, Some(mode::CURSOR)).unwrap();
    assert_eq!(places[&cap], Place::Cursor(h.player));
    assert_eq!(places[&buc], Place::Limbo(mode::CURSOR));
    assert_eq!(h.sim.events.sys.units.get(buc).unwrap().mode, 4);
    assert!(!h.inv().state.holds(h.player, buc));
    assert!(faults(&h).is_empty(), "{:?}", faults(&h));
}

/// Counterexample kept (seed 0; `docs/handoff/prop-unified-items.md` Q2):
/// the ring picked to the cursor and put in the cube (0x16, 0x2A), the
/// buckler lifted to the cursor (0x19), then a transmute. The
/// transmute places the amulet with the store placement, which sets the
/// cursor to none: the buckler the player held is left in mode 4, in no
/// grid and not the cursor item. Pinned as D2RUST behaves today until the
/// question in the handoff file is answered.
#[test]
fn transmute_with_an_item_on_the_cursor_clears_the_cursor() {
    let mut h = host(0);
    let start = check_state(&mut h, None).unwrap();
    let find = |h: &Host, record, place: fn(&Place) -> bool| {
        *start
            .iter()
            .find(|(u, p)| {
                place(p) && h.sim.events.sys.hooks.items.get(**u).unwrap().record == record
            })
            .unwrap()
            .0
    };
    let ring = find(&h, RING, |p| *p == Place::Ground);
    let buc = find(&h, BUC, |p| matches!(p, Place::Page(..)));
    let sel = |h: &Host, u| 4 * h.live_items().iter().position(|&x| x == u).unwrap() as u8;
    h.frame(&Op::Msg {
        id: 0x16,
        a: sel(&h, ring),
        b: 0,
        x: 0,
        y: 1,
    });
    h.frame(&Op::Msg {
        id: 0x2A,
        a: sel(&h, ring),
        b: 0,
        x: 1,
        y: 0,
    });
    h.frame(&Op::Msg {
        id: 0x19,
        a: sel(&h, buc),
        b: 0,
        x: 0,
        y: 0,
    });
    let held = check_state(&mut h, None).unwrap();
    assert_eq!(held[&buc], Place::Cursor(h.player));
    assert_eq!(held[&ring], Place::Page(h.player, 3));
    h.frame(&Op::Transmute);
    let places = check_state(&mut h, Some(mode::CURSOR)).unwrap();
    assert!(h.sim.game.lists.unit(ring).is_none(), "ring used");
    assert_eq!(places[&buc], Place::Limbo(mode::CURSOR));
    assert_eq!(h.inv().state.cursor_of(h.player), None);
    let amulet: Vec<_> = places
        .iter()
        .filter(|(u, _)| h.sim.events.sys.hooks.items.get(**u).unwrap().record == AMULET)
        .collect();
    assert_eq!(amulet.len(), 1);
    assert_eq!(*amulet[0].1, Place::Page(h.player, 3));
    assert!(faults(&h).is_empty(), "{:?}", faults(&h));
}

/// The generator is live (METHODS M08's other half: the property only
/// proves what it reaches): on a fixed runner, every move class between
/// the ground, the cursor, a page and the body happens, a sale frees an
/// item and places its copy in the NPC's grid, and a transmute replaces
/// the ring.
#[test]
fn generator_reaches_every_move() {
    use proptest::strategy::ValueTree;
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    let kind = |p: Option<&Place>| match p {
        None => "none",
        Some(Place::Page(..)) => "page",
        Some(Place::Body(_)) => "body",
        Some(Place::Belt(_)) => "belt",
        Some(Place::Cursor(_)) => "cursor",
        Some(Place::Ground) => "ground",
        Some(Place::Store) => "store",
        Some(Place::Limbo(_)) => "limbo",
    };
    for _ in 0..150 {
        let tree = prop::collection::vec(op(), 1..40).new_tree(&mut runner);
        let ops = tree.unwrap().current();
        let mut h = host(runner.rng().next_u32());
        let mut before = check_state(&mut h, None).unwrap();
        for op in &ops {
            h.frame(op);
            let after = check_state(&mut h, limbo_mode(op)).unwrap();
            for u in before.keys().chain(after.keys()) {
                let (a, b) = (before.get(u), after.get(u));
                if a != b {
                    let r = match op {
                        Op::Transmute => "transmute",
                        Op::Msg { id: 0x33, .. } => "sell",
                        _ => "",
                    };
                    seen.insert((kind(a), kind(b), r));
                }
            }
            before = after;
        }
    }
    let has = |a, b| seen.iter().any(|&(x, y, _)| (x, y) == (a, b));
    for (a, b) in [
        ("ground", "cursor"),
        ("cursor", "page"),
        ("page", "cursor"),
        ("cursor", "body"),
        ("body", "cursor"),
        ("cursor", "ground"),
        ("none", "ground"),
    ] {
        assert!(has(a, b), "{a} → {b} never reached: {seen:?}");
    }
    assert!(seen.contains(&("page", "none", "sell")), "{seen:?}");
    // The sold copy is placed in the open trade NPC's grid, as the
    // store items are.
    assert!(seen.contains(&("none", "page", "sell")), "{seen:?}");
    assert!(seen.contains(&("page", "none", "transmute")), "{seen:?}");
    assert!(seen.contains(&("none", "page", "transmute")), "{seen:?}");
}

/// The checks can fail (METHODS M08): each invariant, broken by hand on a
/// fresh host or a made-up stream, is reported.
#[test]
fn checks_catch_perturbations() {
    let fresh = || {
        let mut h = host(3);
        let places = check_state(&mut h, None).unwrap();
        (h, places)
    };
    let stored = |h: &Host, places: &BTreeMap<UnitId, Place>| {
        *places
            .iter()
            .find(|(u, p)| matches!(p, Place::Page(..)) && **u != h.cube)
            .unwrap()
            .0
    };
    // 1. A unit mode that disagrees with the place.
    let (mut h, places) = fresh();
    let it = stored(&h, &places);
    h.sim.events.sys.units.get_mut(it).unwrap().mode = 1;
    assert!(check_state(&mut h, None).unwrap_err().contains("unit mode"));
    // 1. An item in two places: stored and the cursor.
    let (mut h, places) = fresh();
    let it = stored(&h, &places);
    let p = h.player;
    let inv = &mut h.sim.world.inventory.as_mut().unwrap().state;
    inv.inventories.get_mut(&p).unwrap().set_cursor(Some(it));
    assert!(check_state(&mut h, None).is_err());
    // 1. An item nowhere (its cells and list entry gone) outside a quirk.
    let (mut h, places) = fresh();
    let it = stored(&h, &places);
    let inv = &mut h.sim.world.inventory.as_mut().unwrap().state;
    let mut i = inv.inventories.remove(&p).unwrap();
    let g = i.grid_or_create(grid_id::PAGE, 10, 4).unwrap();
    g.items.retain(|&x| x != it);
    g.cells
        .iter_mut()
        .filter(|c| **c == Some(it))
        .for_each(|c| *c = None);
    inv.inventories.insert(p, i);
    assert!(check_state(&mut h, None).is_err());
    // 2. Cells that disagree with the item's rectangle.
    let (mut h, places) = fresh();
    let it = stored(&h, &places);
    let inv = &mut h.sim.world.inventory.as_mut().unwrap().state;
    inv.items.get_mut(&it).unwrap().x += 1;
    assert!(check_state(&mut h, None).is_err());
    // 1. Item data without a live unit in the store.
    let (mut h, _) = fresh();
    let inv = &mut h.sim.world.inventory.as_mut().unwrap().state;
    inv.items.insert(UnitId(9999), InvItem::new(9999, RING));
    assert!(check_state(&mut h, None).unwrap_err().contains("freed"));
    // 5. The stream: a bad size byte, a message for no item, a place the
    // message contradicts, a held item moved without a message, an
    // update pass without its 0x47 / 0x48.
    let (h, places) = fresh();
    let it = stored(&h, &places);
    let g = h.guid(it);
    let before = snap(&h, places.clone());
    let x9c = |action: u8, guid: u32| {
        let mut b = vec![0x9C, action, 8, 0];
        b.extend_from_slice(&guid.to_le_bytes());
        b
    };
    let tail = || {
        let mut r = vec![0x47, 0, 0];
        r.extend_from_slice(&h.guid(h.player).to_le_bytes());
        r.extend_from_slice(&[0; 4]);
        let mut r2 = r.clone();
        r2[0] = 0x48;
        vec![r, r2]
    };
    let pass = |m: Vec<Vec<u8>>| [m, tail()].concat();
    let ok = check_stream(&h, &before, &before, &[], (&[], &pass(vec![x9c(4, g)])));
    assert_eq!(ok, Ok(()));
    let mut bad = x9c(4, g);
    bad[2] = 9;
    assert!(check_stream(&h, &before, &before, &[], (&[], &pass(vec![bad]))).is_err());
    let none = x9c(4, 0xDEAD);
    assert!(check_stream(&h, &before, &before, &[], (&[], &pass(vec![none]))).is_err());
    let wrong = x9c(1, g);
    assert!(check_stream(&h, &before, &before, &[], (&[], &pass(vec![wrong.clone()]))).is_err());
    assert!(check_stream(&h, &before, &before, &[g], (&[], &pass(vec![wrong]))).is_ok());
    assert!(check_stream(&h, &before, &before, &[], (&[], &[x9c(4, g)])).is_err());
    let mut moved = places;
    moved.insert(it, Place::Cursor(h.player));
    let after = snap(&h, moved);
    assert!(check_stream(&h, &before, &after, &[], (&[], &[])).is_err());
}

/// Counterexample kept (seed 0; `docs/handoff/prop-unified-items.md` Q3):
/// an auto pick-up (0x16 cursor flag 0) of the ground cap, which
/// auto-equips on the empty head: the pick-up takes the cap out of its
/// room, then the inventory model puts it on the head: mode 1, body
/// location 1, S→C 0x9D action 6 (command flag 0x200) then 0x47 / 0x48.
#[test]
fn auto_pickup_with_auto_equip_equips_the_item() {
    let mut h = host(0);
    let start = check_state(&mut h, None).unwrap();
    let cap = *start
        .iter()
        .find(|(u, p)| {
            **p == Place::Ground && h.sim.events.sys.hooks.items.get(**u).unwrap().record == CAP
        })
        .unwrap()
        .0;
    let a = 4 * h.live_items().iter().position(|&x| x == cap).unwrap() as u8;
    let (d, t) = h.frame(&Op::Msg {
        id: 0x16,
        a,
        b: 0,
        x: 0,
        y: 0,
    });
    assert!(d.is_empty(), "{d:02X?}");
    let ids: Vec<(u8, u8)> = t.iter().map(|m| (m[0], m[1])).collect();
    assert_eq!(ids[0], (0x9D, 0x06), "{t:02X?}");
    assert_eq!(
        ids[1..].iter().map(|m| m.0).collect::<Vec<_>>(),
        [0x47, 0x48, 0x2C]
    );
    let places = check_state(&mut h, None).unwrap();
    assert_eq!(places[&cap], Place::Body(h.player));
    assert_eq!(h.sim.game.lists.unit(cap).unwrap().room(), None);
    assert_eq!(h.inv().state.body_items(h.player), vec![cap]);
    assert!(faults(&h).is_empty(), "{:?}", faults(&h));
}

/// Counterexample kept (seed 271983823; `docs/handoff/
/// prop-unified-items.md` Q4): a buy with t = 1 naming a ground item
/// would put a copy in the backpack while
/// it stayed on the ground (one item in two places). The copy of a ground
/// source is refused: the item stays on the ground alone, the 0x2A
/// answers code 9.
#[test]
fn buying_a_ground_item_is_refused() {
    let mut h = host(271983823);
    h.frame(&Op::OpenTrade);
    h.frame(&Op::Spawn(0));
    let before = check_state(&mut h, None).unwrap();
    let ground: Vec<UnitId> = before
        .iter()
        .filter(|(_, p)| **p == Place::Ground)
        .map(|(u, _)| *u)
        .collect();
    let (d, _) = h.frame(&Op::Msg {
        id: 0x32,
        a: 204,
        b: 0,
        x: 91,
        y: 0,
    });
    let errors = &h.inv().state.errors;
    assert!(
        matches!(errors[..], [InvError::GroundCopySource(u)] if ground.contains(&u)),
        "{errors:?}"
    );
    assert!(
        d.iter()
            .any(|m| m.len() == 15 && m[..3] == [0x2A, 0, 9] && m[7..11] == [0xFF; 4]),
        "{d:02X?}"
    );
    let after = check_state(&mut h, None).unwrap();
    assert_eq!(before, after);
}

/// Counterexample kept (seed 157386857, found by the random property in
/// the coordinator's gate): C→S 0x1B (Swap2HandedItem) with the trade
/// window open moved the equipped partner to the cursor and no message
/// named it. The partner gets no command flag
/// and no update entry; the 0x9D action 7 of the item put in its place
/// carries the swap, so the property accepts the move when that message
/// was sent (the sent message is asserted here).
#[test]
fn swap_two_handed_with_a_trade_open_announces_the_cursor_item() {
    let ops = [
        Op::OpenTrade,
        Op::Spawn(0),
        Op::Msg {
            id: 50,
            a: 185,
            b: 0,
            x: 0,
            y: 0,
        },
        Op::Msg {
            id: 25,
            a: 61,
            b: 0,
            x: 0,
            y: 0,
        },
        Op::Msg {
            id: 27,
            a: 5,
            b: 0,
            x: 46,
            y: 0,
        },
    ];
    run(157_386_857, &ops).unwrap();
    // The swap itself reached the client: a 0x9D action 7 in the frame.
    let mut h = host(157_386_857);
    for op in &ops[..4] {
        h.frame(op);
    }
    let (direct, ticked) = h.frame(&ops[4]);
    assert!(
        direct
            .iter()
            .chain(&ticked)
            .any(|m| m[0] == 0x9D && m.get(1) == Some(&7)),
        "{direct:02X?} {ticked:02X?}"
    );
}
