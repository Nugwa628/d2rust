//! The item handlers through the real host frame (drain → tick →
//! flush) on the wired host: `SimGame<ActionSim, WiredWorld>` whose
//! [`CubeParts`] come from synthetic tables (the economy wiring's
//! fixture shape), the real `d2-proto` sizes, real item units from
//! `Economy::create_item` on the action sim's own unit records and stat
//! lists (one unit world), and the player's inventory in the host's one
//! inventory model ([`moves::InvParts`]: the same lists, checks and
//! placement the item moves use). The player's interaction is the unit
//! record's interact info. The fakes are
//! [`ItemPending`] (no owner yet: it logs) and the item-move rest
//! (`moves::tests::MRest`: positions, sounds, the item bit stream).

use std::sync::{Arc, Mutex};

use d2_data::bin::BinTable;
use d2_data::fixup::maps::{EquivMatrix, StateMaps};
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemratio, Itemstatcost, Itemtypes, Record, States};
use d2_sim::combat::CombatTables;
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::{GridRec, InvItemRec, InvTypeRec};
use d2_sim::items::inventory::{InvTables, Inventory, UnitKind};
use d2_sim::items::moves::Owner;
use d2_sim::items::tables::ItemRec;
use d2_sim::items::{flag, q, ty, ItemRequest, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{ClassStats, StatData, StatTable, StateTable};
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::record::Units;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables};
use d2_sim::wiring::economy::{GameFields, ItemSpawn, ItemStore};
use d2_sim::world::cube::{
    input_flags, kind, CraftMod, CubeData, InputSlot, ItemRecord, OutputSlot, Recipe, CUBE_PAGE,
};
use d2_sim::world::npc::NpcControl;
use d2_sim::world::quests::{QuestControl, QuestTables};
use d2_sim::world::vendors::VendorTables;

use super::moves::tests::MRest;
use super::*;
use crate::adapters::handlers::world::tests::trade_quests::{ActionRest, Rest};
use crate::adapters::handlers::world::tests::waypoints::field_drlg;
use crate::adapters::handlers::world::{ActionEvents, ActionWorld, WiredWorld};
use crate::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame};
use crate::dispatch::Outcome;
use crate::host::{Handled, Host};
use crate::seams::{Clock, PlayerGate, SessionHandler};

const N_STATS: usize = 359;
const N_TYPES: usize = 80;
const T_RING: u16 = 10;
const T_BOX: u16 = 11;
const T_AMULET: u16 = 12;
/// Item records (combined index).
const CUBE: usize = 0;
const RING: usize = 1;
const AMULET: usize = 2;
/// A tome, a scroll, arrows and a bow; itemtypes ids where the fixture
/// names them (18 `book`, 22 `scro`, 27 `bow`, 45 `weap`).
const TOME: usize = 3;
const SCROLL: usize = 4;
const ARROWS: usize = 5;
const BOW: usize = 6;
const T_BOWQ: u16 = 5;
const T_BOW: u16 = 27;
const GAME_SEED: u32 = 0x5EED;
/// The player's class (sorceress: inventory record 2, 10 × 4; the cube
/// page is record 9, 3 × 4).
const CLASS: u32 = 2;

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// A plain itemstatcost through the d2-data fix-up.
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
    for c in [T_RING, T_BOX, T_AMULET] {
        set(usize::from(c), usize::from(ty::MISC));
    }
    set(usize::from(T_BOW), 45);
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

/// Items: the cube (`box `), a ring, an amulet.
fn tables() -> ItemTables {
    let mut ratio = Itemratio::decode(&[0u8; Itemratio::SIZE]);
    ratio.version = 1;
    let mut itemtypes: Vec<Itemtypes> = (0..N_TYPES)
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
    // The bow shoots the arrows' type.
    itemtypes[usize::from(T_BOW)].shoots = T_BOWQ;
    ItemTables {
        items: vec![
            item_rec(T_BOX, b"box "),
            item_rec(T_RING, b"rin "),
            item_rec(T_AMULET, b"amu "),
            item_rec(ty::BOOK, b"tbk "),
            item_rec(ty::SCRO, b"tsc "),
            ItemRec {
                stackable: 1,
                maxstack: 500,
                ..item_rec(T_BOWQ, b"aqv ")
            },
            item_rec(T_BOW, b"sbw "),
        ],
        itemtypes,
        equiv: equiv(),
        itemratio: vec![ratio],
        valshift: vec![0; N_STATS],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    }
}

/// A recipe shape without mods: one ring → a normal amulet.
fn ring_to_amulet() -> Recipe {
    let mut inputs = [InputSlot::default(); 7];
    inputs[0] = InputSlot {
        flags: input_flags::USEANY,
        item: RING as u16,
        ..InputSlot::default()
    };
    let out = OutputSlot {
        kind: kind::ITEMCODE,
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

fn cube_data(t: &ItemTables) -> CubeData {
    CubeData {
        recipes: vec![ring_to_amulet()],
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

/// The inventory tables of the same items (grid
/// records; the cube 2 × 2, ring and amulet 1 × 1).
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
    let rec = |code: &[u8; 4], t: u16, w, h| InvItemRec {
        code: *code,
        type_: t as i16,
        invwidth: w,
        invheight: h,
        ..InvItemRec::default()
    };
    InvTables {
        grids,
        belts: vec![12, 8, 4, 16, 8, 12, 16, 12, 8, 4, 16, 8, 12, 16],
        items: vec![
            rec(b"box ", T_BOX, 2, 2),
            rec(b"rin ", T_RING, 1, 1),
            rec(b"amu ", T_AMULET, 1, 1),
            InvItemRec {
                stackable: 1,
                maxstack: 20,
                ..rec(b"tbk ", ty::BOOK, 1, 2)
            },
            rec(b"tsc ", ty::SCRO, 1, 1),
            InvItemRec {
                stackable: 1,
                maxstack: 500,
                ..rec(b"aqv ", T_BOWQ, 1, 3)
            },
            InvItemRec {
                twohanded: 1,
                wclass: *b"bow ",
                ..rec(b"sbw ", T_BOW, 2, 3)
            },
        ],
        itemtypes: {
            let mut t = vec![
                InvTypeRec {
                    class: 7,
                    ..InvTypeRec::default()
                };
                N_TYPES
            ];
            // The ring on either ring finger, the arrows and the bow in
            // either hand.
            for (ty, a, b) in [(T_RING, 6, 7), (T_BOWQ, 4, 5), (T_BOW, 4, 5)] {
                let r = &mut t[usize::from(ty)];
                r.body = 1;
                r.bodyloc1 = a;
                r.bodyloc2 = b;
            }
            t[usize::from(T_BOWQ)].quiver = 1;
            t
        },
        equiv: equiv(),
        books: Vec::new(),
        item_use: Default::default(),
    }
}

/// [`ItemPending`] fake: every call is logged.
#[derive(Clone, Default)]
struct Pending(Arc<Mutex<Vec<String>>>);

impl Pending {
    fn log(&self, s: String) {
        self.0.lock().unwrap().push(s);
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

impl ItemPending for Pending {
    fn inventory_pass(&mut self, _: UnitId, _: &mut Vec<Vec<u8>>) {
        self.log("inventory_pass".into());
    }
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

#[derive(Default)]
struct Session;

impl SessionHandler for Session {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

struct Manual(u32);

impl Clock for Manual {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type Sim = SimGame<ActionSim<ActionRest>, WiredWorld<Rest>>;
type TestHost = Host<Sim, ProtoSizes, Session, Manual>;

/// A host for client 0 with a player (class 2), a cube and a ring in
/// mode `ring_mode`; the cube is stored (mode 0, page 0) in the player's
/// inventory. One frame has run (it only sets the tick driver).
struct T {
    host: TestHost,
    player: UnitId,
    cube: UnitId,
    ring: UnitId,
    pending: Pending,
    rest: MRest,
}

const ALIVE: PlayerFields = PlayerFields {
    gate: PlayerGate {
        mode: 1,
        uninterruptable: false,
    },
    data: Some(PlayerData { last_accept: 0 }),
};

fn item(sim: &mut Sim, class: usize, mode: u32) -> UnitId {
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

/// Places `item` (mode 4, page `page` first) into the player's inventory
/// through the inventory placement (free position, no "send"): a fixture's
/// stored item, as a loaded character's would be.
fn store(sim: &mut Sim, player: UnitId, item: UnitId, page: u8) {
    sim.events.sys.units.get_mut(item).unwrap().mode = 4;
    sim.events.sys.hooks.items.get_mut(item).unwrap().inv_page = page;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    let placed = world.with_economy(game, events, |econ, p| {
        let inv = p.inventory.as_deref_mut().unwrap();
        inv.desk(econ).place(player, item, (0, 0), true, false)
    });
    assert!(placed);
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

/// Game creation on the action sim (expansion, the game seed), the
/// NPC control and the quests on the game seed, the wired host with the
/// cube's parts and the inventory model; a player (class 2) with an
/// inventory, a cube and a ring in mode `ring_mode`, real units of the
/// action sim; the cube is stored (mode 0, page 0) in the player's
/// inventory. The player stands at (100, 100).
fn setup(ring_mode: u32) -> T {
    let pending = Pending::default();
    let rest = MRest::default();
    let tables = tables();
    let hooks = ActionHooks::new(
        Arc::new(action_tables()),
        field_drlg(),
        Seed::init(),
        ActionRest::default(),
    );
    let mut events = ActionSim::new(stat_data(), UnitData::default(), hooks);
    events.create_game(&GameFields::new(Seed::init_low(GAME_SEED), true));
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
    let mut parts = CubeParts::new(cube_data(&world.tables), Box::new(pending.clone()));
    parts.staged.local_date = (15, 3);
    world.cube = Some(parts);
    world.inventory = Some(InvParts::new(inv_tables(), Box::new(rest.clone())));
    let req = AllocRequest {
        ty: UnitType::Player,
        class: CLASS,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let player = events
        .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    let mut sim: Sim = SimGame::with_world(game, events, world);
    let pg = sim.events.sys.units.get(player).unwrap().guid;
    let inv = sim.world.inventory.as_mut().unwrap();
    inv.state
        .add_inventory(player, UnitKind::Player { class: CLASS as u8 }, pg);
    rest.with(|r| r.pos.insert(Owner::player(pg), (100, 100)));
    let cube = item(&mut sim, CUBE, 4);
    store(&mut sim, player, cube, 0);
    let ring = item(&mut sim, RING, ring_mode);
    sim.events.sys.hooks.items.get_mut(ring).unwrap().inv_page = 0;
    sim.join(0, Some(player), None, client_state::IN_GAME)
        .unwrap();
    sim.set_player(player, ALIVE);
    let mut host = Host::new(sim, ProtoSizes, Session, Manual(1000));
    host.connect(0);
    host.frame().unwrap();
    T {
        host,
        player,
        cube,
        ring,
        pending,
        rest,
    }
}

impl T {
    fn world(&mut self) -> &mut WiredWorld<Rest> {
        &mut self.host.game.world
    }
    /// The cube's parts of the host.
    fn cube(&mut self) -> &mut CubeParts {
        self.world().cube.as_mut().unwrap()
    }
    /// The game's one item store.
    fn items(&mut self) -> &mut ItemStore {
        &mut self.host.game.events.sys.hooks.items
    }
    /// The action sim's unit records (the game's one unit store).
    fn units(&mut self) -> &mut Units {
        &mut self.host.game.events.sys.units
    }
    fn guid(&mut self, u: UnitId) -> u32 {
        self.units().get(u).unwrap().guid
    }
    fn set_stat(&mut self, u: UnitId, s: u16, v: i32) {
        let sys = &mut self.host.game.events.sys;
        sys.stats.unit_set(&mut sys.hooks, u, s, v, 0);
    }
    fn stat(&mut self, u: UnitId, s: u16) -> i32 {
        self.host.game.events.sys.stats.unit_total(u, s, 0)
    }
    /// The player's inventory in the host's one inventory model.
    fn inventory(&mut self) -> &mut Inventory {
        let p = self.player;
        let inv = self.world().inventory.as_mut().unwrap();
        inv.state.inventories.get_mut(&p).unwrap()
    }
    /// Stores `item` in the player's inventory on `page`.
    fn store(&mut self, item: UnitId, page: u8) {
        let p = self.player;
        store(&mut self.host.game, p, item, page);
    }
    /// The player's interaction, at its one owner (the unit record).
    fn interact(&mut self, unit_type: u8, guid: u32) {
        let p = self.player;
        let r = self.units().get_mut(p).unwrap();
        r.interact.reset();
        r.interact.set(unit_type, guid);
    }
    /// 0x2A with the ring and the cube.
    fn put_msg(&mut self) -> Vec<u8> {
        let mut m = vec![ITEM_TO_CUBE];
        let (r, c) = (self.ring, self.cube);
        m.extend_from_slice(&self.guid(r).to_le_bytes());
        m.extend_from_slice(&self.guid(c).to_le_bytes());
        m
    }
    /// One frame (drain → tick → flush) with `msg` sent: the dispatch
    /// result and the bytes client 0 receives. Frames are 240 ms apart,
    /// past the client's 200 ms duplicate filter, so a test can resend a
    /// message.
    fn frame(&mut self, msg: &[u8]) -> (ResultCode, Vec<Vec<u8>>) {
        self.host.clock.0 += 240;
        self.host.send_game(0, msg).unwrap();
        let r = self.host.frame().unwrap();
        assert!(r.ticked);
        assert_eq!(r.messages.len(), 1, "{:?}", r.messages);
        let Handled::Game(Outcome::Dispatched(code)) = r.messages[0].handled else {
            panic!("{:?}", r.messages[0]);
        };
        if let Some(c) = &self.host.game.world.cube {
            assert_eq!(c.errors, Vec::new());
        }
        if let Some(i) = &self.host.game.world.inventory {
            assert_eq!(i.state.errors, Vec::new());
        }
        assert!(self.host.game.events.sys.hooks.errors.is_empty());
        (code, self.host.receive(0))
    }
    fn page(&mut self, u: UnitId) -> u8 {
        self.items().get(u).unwrap().inv_page
    }
    fn mode(&mut self, u: UnitId) -> u32 {
        self.units().get(u).unwrap().mode
    }
}

fn click(button: u16) -> [u8; 7] {
    let b = button.to_le_bytes();
    [CLICK_BUTTON, b[0], b[1], 0, 0, 0, 0]
}

const NO_BYTES: Vec<Vec<u8>> = Vec::new();

/// S→C 0x3F (code 0xFF, the item's GUID, 0xFFFF) of the targeting reset
/// (the owner, a player, is the probed unit, so every flagged item sends
/// one).
fn untarget(guid: u32) -> Vec<Vec<u8>> {
    let mut m = vec![0x3F, 0xFF];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&[0xFF, 0xFF]);
    vec![m]
}

/// 0x2A with the cursor item: checks pass, the targeting reset clears
/// flag 0x4 on the inventory items (S→C 0x3F for the flagged cube), the
/// item gets page 3 and is placed into the cube's
/// grid (mode 0, the cursor cleared, linked after the cube); result 0,
/// nothing else sent now (the placement's message is the deferred update
/// pass's).
#[test]
fn item_to_cube_puts_the_cursor_item_in() {
    let mut t = setup(4);
    let (ring, cube) = (t.ring, t.cube);
    t.inventory().set_cursor(Some(ring));
    t.items().get_mut(cube).unwrap().flags |= 0x4 | flag::IDENTIFIED;
    let m = t.put_msg();
    let sent = untarget(t.guid(cube));
    assert_eq!(t.frame(&m), (ResultCode::Done, sent));
    assert_eq!(t.page(ring), CUBE_PAGE);
    assert_eq!(t.items().get(cube).unwrap().flags & 0x14, flag::IDENTIFIED);
    assert_eq!(t.mode(ring), 0);
    assert_eq!(t.inventory().cursor(), None);
    assert_eq!(t.inventory().items(), [cube, ring]);
    // The cube grid (page 3 = grid 5, 3 × 4): a 1 × 1 item of a player's
    // inventory takes the weighted search from (w − 1, h − 1); on
    // the empty grid every corner weighs 2, the first one is kept.
    assert_eq!(
        t.inventory().item_at(2 + usize::from(CUBE_PAGE), 2, 3),
        Some(ring)
    );
    assert!(t.pending.take().is_empty());
    assert!(t.host.game.unhandled.is_empty());
}

/// The item stored in the backpack (mode 0) passes the first checks and
/// is refused at the stored-item test → 3 after the targeting reset ran
/// (flag 0x4 cleared, S→C 0x3F for it); nothing moves.
#[test]
fn item_to_cube_stored_item_is_refused() {
    let mut t = setup(4);
    let (ring, cube) = (t.ring, t.cube);
    t.store(ring, 0);
    t.items().get_mut(cube).unwrap().flags |= 0x4;
    let m = t.put_msg();
    let sent = untarget(t.guid(cube));
    assert_eq!(t.frame(&m), (ResultCode::Malformed, sent));
    assert_eq!(t.page(ring), 0);
    assert_eq!(t.mode(ring), 0);
    assert_eq!(t.inventory().items(), [cube, ring]);
    assert_eq!(t.items().get(cube).unwrap().flags & 0x4, 0, "reset ran");
    assert!(t.pending.take().is_empty());
}

/// The first two checks refuse before the targeting reset (flag 0x4
/// stays).
#[test]
fn item_to_cube_checks() {
    // Missing item → 1.
    let mut t = setup(4);
    let cube = t.cube;
    t.items().get_mut(cube).unwrap().flags |= 0x4;
    let mut m = t.put_msg();
    m[1..5].copy_from_slice(&0xDEADu32.to_le_bytes());
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    // Cursor-mode item that is not the cursor item → 1.
    let m = t.put_msg();
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    // Mode > 4 → 1.
    let ring = t.ring;
    t.units().get_mut(ring).unwrap().mode = 5;
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    // Cube not in the inventory → 1.
    t.units().get_mut(ring).unwrap().mode = 4;
    t.inventory().set_cursor(Some(ring));
    let p = t.player;
    let sim = &mut t.host.game;
    let (game, events, world) = (&mut sim.game, &mut sim.events, &mut sim.world);
    let removed = world.with_economy(game, events, |econ, parts| {
        let inv = parts.inventory.as_deref_mut().unwrap();
        inv.desk(econ).remove(p, cube)
    });
    assert!(removed);
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    // Cube not stored → 1.
    t.store(cube, 0);
    t.units().get_mut(cube).unwrap().mode = 4;
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    assert_ne!(t.items().get(cube).unwrap().flags & 0x4, 0, "no reset");
    assert!(t.pending.take().is_empty());
}

/// Ground items (mode 3) on the model: another act
/// → 2; the range-10 test on both axes between the player's
/// position and the item data's (10 passes, 11 fails). Passing, the
/// item gets page 3; the placement (the item must be on the
/// cursor) refuses a ground item and its result is ignored:
/// result 0, the ring stays on the ground, unlinked.
#[test]
fn item_to_cube_ground_item() {
    let mut t = setup(3);
    let ring = t.ring;
    let m = t.put_msg();
    let at = |t: &mut T, x, y| {
        let inv = t.world().inventory.as_mut().unwrap();
        let d = inv.state.items.get_mut(&ring).expect("item data");
        (d.x, d.y) = (x, y);
    };
    // The item data copy exists once a desk has run (any frame).
    assert_eq!(t.frame(&click(0x01)).0, ResultCode::Done);
    t.units().get_mut(ring).unwrap().act = 1;
    at(&mut t, 100, 100);
    assert_eq!(t.frame(&m).0, ResultCode::Invalid);
    t.units().get_mut(ring).unwrap().act = 0;
    at(&mut t, 111, 100);
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    at(&mut t, 100, 89);
    assert_eq!(t.frame(&m).0, ResultCode::Refused);
    assert_eq!(t.page(ring), 0);
    at(&mut t, 110, 90);
    assert_eq!(t.frame(&m), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.page(ring), CUBE_PAGE);
    assert_eq!(t.mode(ring), 3);
    let cube = t.cube;
    assert_eq!(t.inventory().items(), [cube]);
    assert!(t.pending.take().is_empty());
}

/// Trading and the cube's page ≠ 0: sound event 19, result 0, nothing
/// moved.
#[test]
fn item_to_cube_while_trading() {
    let mut t = setup(4);
    let (ring, cube, player) = (t.ring, t.cube, t.player);
    t.inventory().set_cursor(Some(ring));
    let g = t.guid(player);
    t.interact(0, g);
    t.items().get_mut(cube).unwrap().inv_page = 1;
    let m = t.put_msg();
    assert_eq!(t.frame(&m), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.cube().staged.sounds, [(player, 19)]);
    assert_eq!(t.page(ring), 0);
    assert_eq!(t.inventory().cursor(), Some(ring));
    assert!(t.pending.take().is_empty());
    // Page 0: not refused.
    t.items().get_mut(cube).unwrap().inv_page = 0;
    assert_eq!(t.frame(&m).0, ResultCode::Done);
    assert_eq!(t.page(ring), CUBE_PAGE);
    assert_eq!(t.inventory().items(), [cube, ring]);
}

/// 0x4F with no active interaction queues 0x77 0x0C (any button);
/// result 0. The exact bytes reach the client after the tick's flush.
#[test]
fn click_button_without_interaction() {
    let mut t = setup(4);
    for button in [0x18, 0x17, 0x01] {
        assert_eq!(
            t.frame(&click(button)),
            (ResultCode::Done, vec![vec![0x77, 0x0C]])
        );
    }
    assert!(t.host.game.unhandled.is_empty());
}

/// Buttons 0x17 / 0x18 with an interaction that is not the cube's → 3;
/// another button with a non-player interaction → 0x77 0x0D, result 3;
/// with a player interaction (type 0) it is the player-trade switch,
/// which has no owner yet: it stays the stub.
#[test]
fn click_button_other_interaction() {
    let mut t = setup(4);
    t.interact(2, 77);
    assert_eq!(t.frame(&click(0x17)), (ResultCode::Malformed, NO_BYTES));
    assert_eq!(t.frame(&click(0x18)), (ResultCode::Malformed, NO_BYTES));
    assert_eq!(
        t.frame(&click(0x01)),
        (ResultCode::Malformed, vec![vec![0x77, 0x0D]])
    );
    assert!(t.host.game.unhandled.is_empty());
    let g = t.guid(t.player);
    t.interact(0, g);
    assert_eq!(t.frame(&click(0x01)), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.host.game.unhandled, [(0, CLICK_BUTTON, 7)]);
    assert!(t.pending.take().is_empty());
}

/// The stash buttons through the handler:
/// with the cube's interaction (type 4) → result 1, nothing sent, the
/// gold unchanged; with a type-2 interaction whose GUID is no stash
/// object, the common checks fail: result 0, nothing.
#[test]
fn stash_buttons_through_the_handler() {
    let mut t = setup(4);
    let p = t.player;
    t.interact(4, 9);
    let mut withdraw = click(0x13);
    withdraw[5] = 10; // p2 = 10: v = 10.
    for m in [click(0x12), withdraw, click(0x14)] {
        assert_eq!(t.frame(&m), (ResultCode::Refused, NO_BYTES));
    }
    t.interact(2, 77);
    assert_eq!(t.frame(&withdraw), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.frame(&click(0x12)), (ResultCode::Done, NO_BYTES));
    let r = t.units().get(p).unwrap().interact.get();
    assert_eq!(r, Some((2, 77)), "the common checks fail before the reset");
    assert!(t.host.game.unhandled.is_empty());
    assert!(t.cube().staged.sounds.is_empty());
}

/// 0x17 with the cube open: the interaction resets (GUID −1, type 6,
/// inactive), then the inventory pass; result 0.
#[test]
fn click_button_closes_the_cube() {
    let mut t = setup(4);
    let (cube, player) = (t.cube, t.player);
    let g = t.guid(cube);
    t.interact(4, g);
    assert_eq!(t.frame(&click(0x17)), (ResultCode::Done, NO_BYTES));
    // Reset (GUID −1, type 6, inactive) at the owner: no interaction.
    assert_eq!(t.units().get(player).unwrap().interact.get(), None);
    assert_eq!(t.pending.take(), ["inventory_pass"]);
    // Closed: the next click has no interaction.
    assert_eq!(
        t.frame(&click(0x18)),
        (ResultCode::Done, vec![vec![0x77, 0x0C]])
    );
}

/// 0x18 with the cube open: the ring in the cube (page 3 of the
/// player's inventory) matches the recipe; the amulet is created on a
/// real unit; the ring gets S→C 0x9D action 5 now (owner the player,
/// the ring's item bit stream; its stored page
/// set to 3), is unlinked and freed; sound 4; the amulet is placed
/// into the cube (page 3) and identified. Empty cube: nothing.
/// Any type-4 GUID transmutes.
#[test]
fn click_button_transmutes() {
    let mut t = setup(0);
    let (ring, cube, player) = (t.ring, t.cube, t.player);
    t.interact(4, 12345);
    assert_eq!(t.frame(&click(0x18)), (ResultCode::Done, NO_BYTES));
    assert!(t.pending.take().is_empty());
    assert!(t.cube().staged.sounds.is_empty());

    t.store(ring, CUBE_PAGE);
    assert_eq!(t.inventory().items(), [cube, ring]);
    let (rg, pg) = (t.guid(ring), t.guid(player));
    let mut x9d = vec![0x9D, 5, 13, 0];
    x9d.extend_from_slice(&rg.to_le_bytes());
    x9d.push(0);
    x9d.extend_from_slice(&pg.to_le_bytes());
    let lookup_tables = t.host.game.world.tables.clone();
    let (code, mut sent) = t.frame(&click(0x18));
    assert_eq!(code, ResultCode::Done);
    assert_eq!(sent.len(), 1);
    // The ring's item bit stream: its code, in the
    // cube page (stored page 3 → page + 1 = 4), exact length.
    let m = sent.pop().unwrap();
    assert_eq!(usize::from(m[2]), m.len());
    let bits = d2_proto::item_bits::decode(
        &m[13..],
        &crate::adapters::item_bits::TablesLookup(&lookup_tables),
    )
    .unwrap();
    assert_eq!(&bits.code, b"rin ");
    let Some(d2_proto::item_bits::Location::Slot { page1, .. }) = bits.location else {
        panic!("{bits:?}");
    };
    assert_eq!(page1, 4);
    x9d[2] = m[2];
    x9d.extend_from_slice(&m[13..]);
    assert_eq!(m, x9d);
    assert!(t.pending.take().is_empty());
    assert!(!t.items().contains(ring));
    assert!(t.host.game.game.lists.unit(ring).is_none());
    let items = t.inventory().items().to_vec();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0], cube);
    let amu = items[1];
    let it = t.items().get(amu).unwrap().clone();
    assert_eq!(
        (it.record, it.quality, it.inv_page),
        (AMULET, q::NORMAL, CUBE_PAGE)
    );
    assert_eq!(t.mode(amu), 0);
    assert_ne!(it.flags & flag::IDENTIFIED, 0);
    assert_eq!(t.cube().staged.sounds, [(player, 4)]);
    assert!(t.rest.take_log().is_empty());
}

/// Item ids nothing owns yet stay stubs (recorded, result 0), as do
/// owned ids whose system the host lacks (0x17 without the inventory
/// parts, `moves`); the table names an owner for exactly the handled
/// ids.
#[test]
fn unowned_item_ids_stay_stubs() {
    let mut t = setup(4);
    let g = t.guid(t.ring);
    let mut tmog = vec![0x4C];
    tmog.extend_from_slice(&g.to_le_bytes());
    assert_eq!(t.frame(&tmog), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.host.game.unhandled, [(0, 0x4C, 5)]);
    let owned: Vec<u8> = ITEM_IDS
        .iter()
        .filter(|(_, o)| o.is_some())
        .map(|&(id, _)| id)
        .collect();
    let mut want: Vec<u8> = moves::MOVE_IDS.iter().map(|&(id, _, _)| id).collect();
    want.extend([ITEM_TO_CUBE, CLICK_BUTTON]);
    want.sort_unstable();
    assert_eq!(owned, want);
    assert!(ITEM_IDS.windows(2).all(|w| w[0].0 < w[1].0));

    // Without the inventory parts on the host the item moves are stubs.
    t.host.game.world.inventory = None;
    let mut drop = vec![0x17];
    drop.extend_from_slice(&g.to_le_bytes());
    assert_eq!(t.frame(&drop), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.host.game.unhandled.last(), Some(&(0, 0x17, 5)));
    // Without the cube on the host the owned ids are stubs too.
    t.host.game.world.cube = None;
    assert_eq!(t.frame(&click(0x18)), (ResultCode::Done, NO_BYTES));
    assert_eq!(t.host.game.unhandled.last(), Some(&(0, CLICK_BUTTON, 7)));
}

#[path = "mutant_tests.rs"]
mod mutant_tests;
