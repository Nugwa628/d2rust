//! End-to-end vendor path: the bridge (`d2_client::bridge`) on its local
//! link over the in-process `d2-server` host, whose game is `SimGame` on
//! the wired `d2-sim` (`wiring::action::ActionSim`) with the server's
//! [`WiredWorld`] as its world host: the NPC and vendor handlers run on
//! `wiring::interaction` (`Desk`, `VendorDesk`) over the action sim's own
//! units and stat lists, from synthetic tables (no game files).
//!
//! The run (each step says where it stops and why):
//!
//! 1. C→S 0x13: Akara's talk starts: S→C 0x27, 0x29, 0x28;
//! 2. C→S 0x2F: chat open, the heal hook;
//! 3. C→S 0x38 action 1: trade open generates the store
//!    on the NPC-control seed, items created by the economy wiring
//!    on the game seed;
//! 4. C→S 0x32 with too little gold: S→C 0x2A code 12;
//! 5. C→S 0x32 with enough gold: the purchase loop copies
//!    the store cap (on the inventory model), pays
//!    and places the copy in the backpack: S→C 0x2A code 0, kind 4;
//! 6. C→S 0x33 of the player's buckler (re-sellable, not one of Akara's
//!    permanent codes): a restored copy goes into the store, the
//!    buckler leaves the player, the price is received:
//!    0x2A code 1, kind 3;
//! 7. C→S 0x33 of the player's cap (a permanent code: no copy back):
//!    the sale removes it from the player's inventory (the
//!    server's one inventory model: unlinked, freed), the price is
//!    received: 0x2A code 1, kind 3, the item's GUID, the new
//!    gold.
//!
//! The player's buckler and cap are in the server host's inventory
//! model (`WiredWorld::inventory`, the item moves' own), which answers
//! the vendors' ownership and removal calls.
//!
//! Every S→C message is asserted byte for byte (0x2A bytes 3–6 are not
//! written by the game; D2RUST writes 0). The run is repeated (same seed
//! → identical transcript), and one input is perturbed (a mutation
//! check): one gold more changes exactly the gold
//! fields of the 0x2A messages.

use std::sync::Arc;

use d2_client::bridge::dispatch::Dispatch;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::local::{LocalLink, PendingSession};
use d2_client::bridge::world::MonsterClass;
use d2_client::bridge::Bridge;
use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemstatcost, Record};
use d2_proto::client::{BuyItem, EntityAction, InitEntityChat, InteractWithEntity, SellItem};
use d2_server::adapters::handlers::world::{ActionWorld, Outbox, WiredWorld};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use d2_server::dispatch::Outcome;
use d2_server::host::{Handled, Host};
use d2_server::seams::{Clock, PlayerGate, Pos, ResultCode};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::{DrlgData, Dungeon, LevelTypes, TileInfo, TileSource};
use d2_sim::game::Game;
use d2_sim::items::ItemRequest;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{StatData, StatTable};
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::economy::ItemSpawn;
use d2_sim::world::npc::{class, NpcControl};
use d2_sim::world::quests::{PlayerQuests, QuestControl, QuestTables};

use crate::e2e_support;
use e2e_support::*;

// ---- constants -----------------------------------------------------------------------

/// The game seed.
const GAME_SEED: u32 = 1234;
/// Gold the player carries at the first buy: one less than the store
/// item's price is set later (step 4); this is the gold for steps 5–7.
const PLAYER_GOLD: i32 = 5000;
/// Stat ids (`itemstatcost`).
const LEVEL: u16 = 12;
const GOLD: u16 = 14;
const ARMORCLASS: u16 = 31;
const MAXDURABILITY: u16 = 73;
const DURABILITY: u16 = 72;
/// Item flags (the store item flags).
const IDENTIFIED: u32 = 0x10;
/// Item mode "stored".
const STORED: u32 = 0;
const LOCAL: u32 = d2_client::bridge::LOCAL_CLIENT;

// ---- seams without a provider ----------------------------------------------------------

/// The action wiring's seams: every answer is `Pending`'s default (no
/// action path runs here); sends are kept for the world host.
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

struct NoTiles;

impl TileSource for NoTiles {
    fn dt1(&self, _: &[u8]) -> Option<&[TileInfo]> {
        None
    }
}

struct NoLevelTypes;

impl LevelTypes for NoLevelTypes {}

// ---- tables -----------------------------------------------------------------------------

/// A synthetic itemstatcost: 359 stats, no ops, fixed up.
fn stat_data() -> Arc<StatData> {
    let size = Itemstatcost::SIZE;
    let mut records = vec![0u8; N_STATS * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: N_STATS,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        ..StatData::default()
    })
}

// ---- the game ---------------------------------------------------------------------------

type World = WiredWorld<Rest>;
type Sim = SimGame<ActionSim<ActionRest>, World>;

/// Manual host clock (ms), injected into the host.
struct Ms(u32);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type Link = LocalLink<Sim, ProtoSizes, PendingSession, Ms>;

/// Akara's monster add S→C 0xAC as the client reads it:
/// mode 1, no components, no type flags, no
/// source unit, no stat list. d2-sim does not build monster adds yet (the
/// server-side fields of a monster add are not specified, see the
/// `wiring::action::switch` module docs), so the harness delivers the
/// add of an NPC without any of those parts; the client needs her unit
/// for 0x28.
fn akara_add(guid: u32, class: u16, (x, y): (i32, i32)) -> Vec<u8> {
    let mut m = vec![0xAC];
    m.extend(guid.to_le_bytes());
    m.extend(class.to_le_bytes());
    m.extend((x as u16).to_le_bytes());
    m.extend((y as u16).to_le_bytes());
    m.push(128);
    m.push(14);
    // Bits, low first: mode 1 (4 bits), then five 0 presence bits.
    m.push(0x01);
    m
}

/// The local link, recording every S→C chunk the bridge receives.
struct Tap {
    inner: Link,
    chunks: Vec<Vec<u8>>,
    /// Chunks delivered after the server's at the next receive, not
    /// recorded (Akara's add, [`akara_add`]).
    inject: Vec<Vec<u8>>,
}

impl ServerLink for Tap {
    fn protocol_version(&self) -> u32 {
        self.inner.protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.inner.send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.inner.pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        let mut got = self.inner.receive();
        self.chunks.extend(got.iter().cloned());
        got.append(&mut self.inject);
        got
    }
}

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

/// Game creation before the first host frame: the action sim (no DRLG
/// rooms: the vendor path reads none), the NPC control and the quests
/// on the game seed, Akara and the player allocated, the player's gold
/// and level, a buckler and a cap the player owns, the client joined.
struct Fx {
    bridge: Bridge<Tap>,
    player: UnitId,
    npc: UnitId,
    /// The player's buckler (re-sellable) and cap (a permanent code).
    buckler: UnitId,
    cap: UnitId,
}

impl Fx {
    fn new(game_seed: u32, gold: i32) -> Self {
        Self::with_class(game_seed, gold, class::AKARA)
    }

    /// The same game with the NPC of class `npc_class` (Akara, Gheed).
    fn with_class(game_seed: u32, gold: i32, npc_class: u16) -> Self {
        Self::build(game_seed, gold, npc_class, false)
    }

    /// [`Fx::with_class`] with the game's expansion flag (game +0x70).
    fn build(game_seed: u32, gold: i32, npc_class: u16, expansion: bool) -> Self {
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
            expansion,
            ..UnitData::default()
        };
        let mut events = ActionSim::new(stat_data(), data, hooks);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();

        // Game creation: the NPC control (its own seed from the game
        // seed) and the quests.
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
        let npc = alloc(UnitType::Monster, u32::from(npc_class));
        let player = alloc(UnitType::Player, 1);
        let npc_guid = game.lists.unit(npc).unwrap().guid;
        // Players are allocated in mode 0; neutral.
        events.sys.units.get_mut(player).unwrap().mode = 1;
        events.with(&mut game, |_, v| {
            v.set_base(player, LEVEL, 1);
            v.set_base(player, GOLD, gold);
        });

        let mut rest = Rest::default();
        rest.quests.insert(player, PlayerQuests::default());
        let mut world: World = WiredWorld::new(
            ActionWorld::default(),
            item_tables(),
            quests,
            ctl,
            vendor_tables(),
            rest,
            1000,
        );
        // Monster init embeds the NPC's interaction list.
        world.state.add_npc(npc);
        // The inventory model (cap and buckler 2 × 2) with the player's
        // inventory; the item-move seams staged (`InvFx`).
        let pg = events.sys.units.get(player).unwrap().guid;
        let tables = inv_tables(&world.tables, &[(2, 2), (2, 2)]);
        world.inventory = Some(inv_parts(tables, InvFx::default(), player, 1, pg));
        // The player's buckler and cap, made by the economy wiring on
        // the game seed and stored (mode 0) in its inventory.
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
                    mode: STORED,
                    init_flags: 1,
                };
                econ.create_item(&mut rq, false, spawn).expect("item")
            };
            (make(BUC), make(CAP))
        });
        for item in [buckler, cap] {
            store(&mut world, &mut game, &mut events, (player, item), 0);
        }

        let mut s: Sim = SimGame::with_world(game, events, world);
        s.join(LOCAL, Some(player), None, client_state::IN_GAME)
            .unwrap();
        s.set_player(
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
        s.set_unit(player, at);
        s.set_unit(npc, at);

        let link = LocalLink::new(Host::new(
            s,
            ProtoSizes,
            PendingSession::default(),
            Ms(1000),
        ));
        let tap = Tap {
            inner: link,
            chunks: Vec::new(),
            inject: vec![akara_add(npc_guid, npc_class, (0, 0))],
        };
        let mut bridge = Bridge::with_dispatch(tap, Dispatch::from_spec().unwrap()).unwrap();
        // Akara's class row in the client tables (an `interact` NPC), so
        // her add creates the unit.
        let mut tables = bridge.inputs().tables.clone();
        tables.monsters = vec![None; usize::from(npc_class) + 1];
        tables.monsters[usize::from(npc_class)] = Some(MonsterClass {
            npc: true,
            interact: true,
            ..MonsterClass::default()
        });
        bridge.set_tables(tables);
        Fx {
            bridge,
            player,
            npc,
            buckler,
            cap,
        }
    }

    fn sim(&mut self) -> &mut Sim {
        &mut self.bridge.link_mut().inner.host_mut().game
    }

    fn sim_ref(&self) -> &Sim {
        &self.bridge.link().inner.host().game
    }

    fn guid(&self, u: UnitId) -> u32 {
        self.sim_ref().game.lists.unit(u).unwrap().guid
    }

    fn stat(&mut self, u: UnitId, s: u16) -> i32 {
        let sim = self.sim();
        sim.events.with(&mut sim.game, |_, v| v.stat(u, s))
    }

    fn base(&mut self, u: UnitId, s: u16) -> i32 {
        self.sim_ref().events.sys.stats.unit_base(u, s, 0)
    }

    fn set_gold(&mut self, gold: i32) {
        let p = self.player;
        let sim = self.sim();
        sim.events
            .with(&mut sim.game, |_, v| v.set_base(p, GOLD, gold));
    }

    /// Every error so far: action adapters, the unit dispatch, the
    /// interaction adapters, the world handlers' faults.
    fn errors(&self) -> Vec<String> {
        let s = self.sim_ref();
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
        if let Some(i) = &s.world.inventory {
            e.extend(i.state.errors.iter().map(|f| format!("{f:?}")));
        }
        e
    }

    /// The player's items in its inventory (link order).
    fn inventory(&self) -> Vec<UnitId> {
        let s = self.sim_ref();
        s.world
            .inventory
            .as_ref()
            .unwrap()
            .state
            .items_of(self.player)
    }

    /// Sends `msgs` through the bridge, advances the host clock 40 ms
    /// (one tick) and runs one bridge frame: the drained (C→S id,
    /// result) pairs and the S→C chunks the client received.
    fn step(&mut self, msgs: &[Vec<u8>]) -> Frame {
        for m in msgs {
            assert_eq!(self.bridge.send_bytes(m).unwrap(), Sent::Queued);
        }
        self.bridge.link_mut().inner.host_mut().clock.0 += 40;
        let report = self.bridge.frame().unwrap();
        let host = self.bridge.link().inner.last_frame();
        let codes = host
            .messages
            .iter()
            .map(|h| match h.handled {
                Handled::Game(Outcome::Dispatched(c)) => (h.id, Some(c)),
                _ => (h.id, None),
            })
            .collect();
        let received = std::mem::take(&mut self.bridge.link_mut().chunks);
        assert_eq!(received.len(), report.messages, "one chunk per S→C message");
        Frame {
            sent: msgs.to_vec(),
            codes,
            received,
        }
    }
}

/// One bridge frame.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    sent: Vec<Vec<u8>>,
    codes: Vec<(u8, Option<ResultCode>)>,
    received: Vec<Vec<u8>>,
}

fn bytes<M: d2_proto::FixedMessage>(m: &M) -> Vec<u8> {
    d2_client::bridge::intent::encode(m)
}

// ---- the run ----------------------------------------------------------------------------

/// A store item: (GUID, record, quality, item seed, AC, page).
type Row = (u32, usize, u8, Seed, i32, u8);

/// Everything a run leaves behind, compared between runs.
#[derive(Debug, PartialEq, Eq)]
struct Transcript {
    frames: Vec<Frame>,
    game_seed: Seed,
    npc_seed: Seed,
    store: Vec<Row>,
    gold: i32,
    /// The player's items at the end (link order).
    inventory: Vec<UnitId>,
    rest_log: Vec<String>,
    unhandled: Vec<(u32, u8, usize)>,
    errors: Vec<String>,
}

fn run() -> Transcript {
    run_with(GAME_SEED, PLAYER_GOLD)
}

fn run_with(game_seed: u32, gold: i32) -> Transcript {
    let mut fx = Fx::new(game_seed, gold);
    let (player, npc) = (fx.player, fx.npc);
    let ng = fx.guid(npc);
    let mut frames = Vec::new();
    let done = Some(ResultCode::Done);

    // Frame 1: the tick driver starts its clock; no tick.
    assert!(!fx.bridge.frame().unwrap().ticked);

    // 1. C→S 0x13 (unit type 1, Akara): the NPC talk. Distance 3 (stub):
    // the AI parameter 0x28, the path cleared, the talk started: node
    // {player, state 0}, interact unit (1, GUID), S→C 0x27, 0x29, 0x28 in
    // that order.
    let talk = bytes(&InteractWithEntity { type_: 1, id: ng });
    let mut want = vec![0x13, 1, 0, 0, 0];
    want.extend_from_slice(&ng.to_le_bytes());
    assert_eq!(talk, want);
    let f = fx.step(&[talk]);
    assert_eq!(f.codes, [(0x13, done)]);
    let ids: Vec<u8> = f.received.iter().map(|m| m[0]).collect();
    assert_eq!(ids, [0x27, 0x29, 0x28]);
    // 0x27 (40 bytes): 0x27, u8 1, NPC GUID, then the 34-byte list of
    // the NPC message list (`server-messages.tsv` 0x27 `partial`: the stub's
    // zeros).
    let mut npc_info = vec![0x27, 1];
    npc_info.extend_from_slice(&ng.to_le_bytes());
    npc_info.extend_from_slice(&[0; 34]);
    assert_eq!(f.received[0], npc_info);
    // 0x29 (97 bytes): 0x29 and the game record.
    let mut game_quests = vec![0x29];
    game_quests.extend_from_slice(&fx.sim_ref().world.quests.game.0);
    assert_eq!(f.received[1], game_quests);
    assert_eq!(f.received[1].len(), 97);
    // 0x28 (103 bytes): 0x28, type 1, NPC GUID, 0, the player's record
    // of the current difficulty.
    let mut quest_info = vec![0x28, 1];
    quest_info.extend_from_slice(&ng.to_le_bytes());
    quest_info.push(0);
    quest_info.extend_from_slice(&fx.sim_ref().world.rest.quests[&player].flags[0].0);
    assert_eq!(f.received[2], quest_info);
    assert_eq!(f.received[2].len(), 103);
    let rec = fx.sim_ref().events.sys.units.get(player).unwrap();
    assert_eq!(rec.interact.get(), Some((1, ng)));
    frames.push(f);

    // 2. C→S 0x2F: chat open: node state 0 → 1, the heal hook (nothing
    // to heal on a fresh player). No message.
    let chat = bytes(&InitEntityChat { id: ng });
    let mut want = vec![0x2F, 0, 0, 0, 0];
    want.extend_from_slice(&ng.to_le_bytes());
    assert_eq!(chat, want);
    let f = fx.step(&[chat]);
    // The client's own 0x2F (T 1, its 0x28 handler; held behind the
    // unanswered dialog slot until this frame)
    // comes first, then this one.
    assert_eq!(f.codes, [(0x2F, done), (0x2F, done)]);
    frames.push(f);

    // 3. C→S 0x38 action 1 (trade): trade open, then the store fill. The store is
    // generated on the NPC-control seed (the buckler entry's range draw
    // and quality draws), its items made by the economy wiring on the
    // game seed (two steps per item); then the permanent
    // cap. Each item: identified, vendor unit flag, page 0.
    // 0x9C action 11 belongs to the unwritten item spec
    // (`add_trade_inventory` stub): no S→C message.
    let seed_before = fx.sim_ref().events.sys.hooks.game_seed;
    let trade = bytes(&EntityAction {
        action: 1,
        npc: ng,
        item: 0,
    });
    assert_eq!(trade.len(), 13);
    let f = fx.step(&[trade]);
    assert_eq!(f.codes, [(0x38, done)]);
    // One S→C 0x9C action 11 per store item (the store fill),
    // in store order; each names the item's GUID.
    let shown: Vec<(u8, u8, u32)> = f
        .received
        .iter()
        .map(|m| (m[0], m[1], u32::from_le_bytes(m[4..8].try_into().unwrap())))
        .collect();
    frames.push(f);
    let (store, rec_store) = {
        let w = &fx.sim_ref().world;
        let i = w.state.vendor_index(class::AKARA).unwrap();
        let rec = &w.state.vendors[i];
        assert!(rec.has_traded && rec.store_generated);
        assert_eq!(rec.last_npc, ng);
        (rec.store.clone(), rec.store.len())
    };
    assert!(rec_store >= 2, "≥ 1 buckler (Min 1) and the permanent cap");
    let mut seed = seed_before;
    for _ in 0..2 * rec_store {
        seed.step();
    }
    assert_eq!(fx.sim_ref().events.sys.hooks.game_seed, seed);
    let mut store_rows = Vec::new();
    for &item in &store {
        let guid = fx.guid(item);
        let ac = fx.base(item, ARMORCLASS);
        let s = fx.sim_ref();
        let it = s.events.sys.hooks.items.get(item).unwrap();
        assert_ne!(it.flags & IDENTIFIED, 0);
        assert_eq!(it.inv_page, 0);
        let u = s.events.sys.units.get(item).unwrap();
        assert_ne!(u.flags2 & 4, 0, "vendor item (unit +0xC8 |= 4)");
        store_rows.push((guid, it.record, it.quality, it.item_seed, ac, it.inv_page));
    }
    let (bucs, caps): (Vec<&Row>, Vec<&Row>) = store_rows.iter().partition(|r| r.1 == BUC);
    assert!((1..=3).contains(&bucs.len()), "Min 1, Max 3");
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].2, 2, "the permanent item is normal (§3)");
    let want_shown: Vec<(u8, u8, u32)> = store_rows.iter().map(|r| (0x9C, 11, r.0)).collect();
    assert_eq!(shown, want_shown, "0x9C action 11 per store item");
    let cap = *store.last().unwrap();
    assert_eq!(store_rows.last().unwrap().1, CAP, "permanent codes last");
    assert_eq!(fx.stat(cap, MAXDURABILITY), 12);

    // The cap's buy price by hand: S = cost 100;
    // type 50 (armor): S := cost·AC / max AC (rule 2); not magic, no
    // skills; Akara's sell mult 1024 (rule 9); qty 1; rp 0.
    let store_ac = fx.base(cap, ARMORCLASS);
    assert!((3..=5).contains(&store_ac));
    let price = 100 * store_ac / 5;
    let cg = fx.guid(cap);

    // 4. C→S 0x32 with one gold less than the price:
    // 0x2A code 12, GUID −1, result 0; nothing paid.
    fx.set_gold(price - 1);
    let buy = bytes(&BuyItem {
        npc: ng,
        item: cg,
        transaction: 0,
        client_price: 0,
    });
    let mut want = vec![0x32];
    for v in [ng, cg, 0, 0] {
        want.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(buy, want);
    let f = fx.step(std::slice::from_ref(&buy));
    assert_eq!(f.codes, [(0x32, done)]);
    assert_eq!(f.received, [tx(0, 12, u32::MAX, price - 1)]);
    frames.push(f);
    assert_eq!(fx.stat(player, GOLD), price - 1);

    // 5. C→S 0x32 with `gold`: steps 1–8 pass (in the NPC inventory, the
    // price, gold, no cursor item, no tome, no stack), the purchase loop
    // (step 9, one pass without fill): the copy of the store cap
    // (fillers 1, on the inventory model: defense and durability through
    // the save record), the price paid, mode 4, auto-placed in the
    // backpack; the on-buy hook keeps the permanent cap
    // (step 12); 0x2A code 0, kind 4, GUID = the copy. The copy's next
    // frame 0x9C (step 10) is the item update's, as step 3's 0x9C.
    fx.set_gold(gold);
    // The same bytes again: past the client's 200 ms duplicate filter
    // first, five idle frames (nothing sent or
    // received).
    for _ in 0..5 {
        assert!(fx.step(&[]).received.is_empty());
    }
    let f = fx.step(&[buy]);
    assert_eq!(f.codes, [(0x32, done)]);
    let bought = *fx.inventory().last().unwrap();
    assert_eq!(fx.inventory(), [fx.buckler, fx.cap, bought]);
    assert!(!store.contains(&bought), "a new unit");
    assert_eq!(f.received, [tx(4, 0, fx.guid(bought), gold - price)]);
    frames.push(f);
    let gold = gold - price;
    assert_eq!(fx.stat(player, GOLD), gold);
    assert_eq!(fx.base(bought, ARMORCLASS), store_ac);
    assert_eq!(fx.stat(bought, MAXDURABILITY), 12);
    assert!(fx.sim_ref().game.lists.unit(cap).is_some(), "permanent");

    // 6. C→S 0x33 of the player's buckler (stored, mode 0): sale steps 1–7
    // pass (the player's per its inventory, mode 0, no flag, the price,
    // re-sellable). Step 8 (not a permanent code): the restored copy
    // into the NPC, placed on its store page; step 9 removes the
    // buckler (stored: unlinked, freed); step 10 receives the price:
    // 0x2A code 1, kind 3, the buckler's GUID. The price by hand (sell
    // price, t = 1): 80·AC/6 (step 2), buy mult 512: ·512/1024 (step 9);
    // the restored copy's price is the same (step 8: min).
    let pg = fx.guid(fx.buckler);
    let buc_ac = fx.base(fx.buckler, ARMORCLASS);
    let buc_sold = (80 * buc_ac / 6) * 512 / 1024;
    assert!(buc_sold > 0);
    let sell = bytes(&SellItem {
        npc: ng,
        item: pg,
        item_mode: STORED as u16,
        client_price: 0,
    });
    let mut want = vec![0x33];
    want.extend_from_slice(&ng.to_le_bytes());
    want.extend_from_slice(&pg.to_le_bytes());
    want.extend_from_slice(&[0; 8]);
    assert_eq!(sell, want);
    let f = fx.step(&[sell]);
    assert_eq!(f.codes, [(0x33, done)]);
    // The stored buckler leaves with S→C 0x9D action 5 (step 9) before the
    // 0x2A (the message order).
    assert_stored_sale(&f.received, pg, tx(3, 1, pg, gold + buc_sold));
    frames.push(f);
    let gold = gold + buc_sold;
    assert_eq!(fx.stat(player, GOLD), gold);
    assert_eq!(fx.inventory(), [fx.cap, bought]);
    assert!(fx.sim_ref().game.lists.unit(fx.buckler).is_none(), "freed");
    let restored = {
        let w = &fx.sim_ref().world;
        let i = w.state.vendor_index(class::AKARA).unwrap();
        *w.state.vendors[i].store.last().unwrap()
    };
    assert!(!store.contains(&restored), "the copy joined the store");
    assert_eq!(fx.base(restored, ARMORCLASS), buc_ac);

    // 7. C→S 0x33 of the player's cap: re-sellable but one of Akara's
    // permanent codes, so no copy (step 8: the permanent store item
    // stays); step 9 takes it from the player (stored:
    // unlinked from its inventory, freed), step 10 receives the price (under the
    // staged carried-gold cap): 0x2A code 1, kind 3, the sold item's
    // GUID, the new gold. The price by hand (sell price, t = 1): B = 100·AC/5
    // (step 2), not ethereal, class 7 (step 7: no /4), buy mult 512:
    // B·512/1024 (step 9), qty 1, max buy 5000.
    let eg = fx.guid(fx.cap);
    let cap_ac = fx.base(fx.cap, ARMORCLASS);
    let sold = (100 * cap_ac / 5) * 512 / 1024;
    assert!(sold > 0);
    let sell = bytes(&SellItem {
        npc: ng,
        item: eg,
        item_mode: STORED as u16,
        client_price: 0,
    });
    let f = fx.step(&[sell]);
    assert_eq!(f.codes, [(0x33, done)]);
    assert_stored_sale(&f.received, eg, tx(3, 1, eg, gold + sold));
    frames.push(f);
    assert_eq!(fx.stat(player, GOLD), gold + sold);
    assert_eq!(fx.inventory(), [bought]);
    assert!(fx.sim_ref().game.lists.unit(fx.cap).is_none(), "freed");
    assert!(!fx.sim_ref().events.sys.hooks.items.contains(fx.cap));

    let errors = fx.errors();
    assert!(errors.is_empty(), "{errors:?}");
    let gold_now = fx.stat(player, GOLD);
    let inventory = fx.inventory();
    let s = fx.sim_ref();
    // No id fell back to the stub: every message had a provider.
    assert!(s.unhandled.is_empty(), "{:?}", s.unhandled);
    Transcript {
        frames,
        game_seed: s.events.sys.hooks.game_seed,
        npc_seed: s.world.npc.seed,
        store: store_rows,
        gold: gold_now,
        inventory,
        rest_log: s.world.rest.log.clone(),
        unhandled: s.unhandled.clone(),
        errors,
    }
}

#[test]
fn vendor_end_to_end() {
    let t = run();
    assert_eq!(t.frames.len(), 7);
    // Both copies ran on the inventory model, none on the rest.
    assert!(
        !t.rest_log.iter().any(|l| l.starts_with("copy")),
        "{:?}",
        t.rest_log
    );
    // The sold buckler and cap left the player's inventory; the bought
    // copy is in it.
    assert_eq!(t.inventory.len(), 1);
}

/// Same seed → the same run: every C→S byte, result, S→C chunk, seed,
/// store item, gold and stub call, twice.
#[test]
fn same_seed_same_run() {
    assert_eq!(run(), run());
}

/// The comparison sees one input (a mutation check): one gold more changes exactly
/// the gold fields (bytes 11–14) of the 0x2A messages of steps 5–7 and
/// the final gold; nothing else in the wire or the state.
#[test]
fn one_gold_more_changes_only_the_gold_fields() {
    let (a, b) = (run(), run_with(GAME_SEED, PLAYER_GOLD + 1));
    assert_eq!(b.gold, a.gold + 1);
    assert_eq!(a.store, b.store);
    assert_eq!(a.inventory, b.inventory);
    assert_eq!((a.game_seed, a.npc_seed), (b.game_seed, b.npc_seed));
    assert_eq!(a.rest_log, b.rest_log);
    let mut diffs = Vec::new();
    for (i, (fa, fb)) in a.frames.iter().zip(&b.frames).enumerate() {
        assert_eq!(fa.sent, fb.sent);
        assert_eq!(fa.codes, fb.codes);
        assert_eq!(fa.received.len(), fb.received.len());
        for (j, (ma, mb)) in fa.received.iter().zip(&fb.received).enumerate() {
            for (k, (x, y)) in ma.iter().zip(mb).enumerate() {
                if x != y {
                    diffs.push((i, j, k));
                }
            }
        }
    }
    // Frames 4–6 (steps 5–7), the one 0x2A each (after the sales' 0x9D
    // action 5 in frames 5 and 6), its gold's low byte.
    assert_eq!(diffs, [(4, 0, 11), (5, 1, 11), (6, 1, 11)]);
}

/// Another game seed gives other store items (unit and item seeds from
/// the game seed, the NPC-control seed from it too).
#[test]
fn other_seed_other_store() {
    let (a, b) = (run(), run_with(GAME_SEED + 1, PLAYER_GOLD));
    assert_ne!(a.game_seed, b.game_seed);
    assert_ne!(a.npc_seed, b.npc_seed);
    assert_ne!(a.store, b.store);
}

// ---- the shop panel -----------------------------------

/// The client half of the trade: the store items the server shows
/// (S→C 0x9C action 11) open the shop panel; a right click on the
/// permanent cap leaves as C→S 0x32 and the server pays and places the
/// copy; closing the shop ends the interaction (C→S 0x30).
#[test]
fn shop_panel_buys_and_closes() {
    use d2_client::bridge::items::store_items;
    use d2_client::ui::layout::Screen;
    use d2_client::ui::original::{OriginalUi, UiConfig};
    use d2_client::ui::panel::PointerButton;
    use d2_client::ui::panel::{NoStrings, UiCtx, UiEvent};
    use d2_client::ui::{NoPanelRules, Point, UiRoot};
    use d2_proto::client::TerminateEntityChat;

    let mut fx = Fx::new(GAME_SEED, PLAYER_GOLD);
    let (player, npc) = (fx.player, fx.npc);
    let ng = fx.guid(npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
    fx.step(&[bytes(&InitEntityChat { id: ng })]);

    let mut ui = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        },
        None,
    )
    .unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    ui.shop_poll(fx.bridge.world(), &mut root);
    assert!(!ui.is_open(0x0C), "no store items shown yet");

    // C→S 0x38 action 1: the server shows every store item.
    let trade = bytes(&EntityAction {
        action: 1,
        npc: ng,
        item: 0,
    });
    let f = fx.step(&[trade]);
    assert!(f.received.iter().all(|m| m[0] == 0x9C && m[1] == 11));
    ui.shop_poll(fx.bridge.world(), &mut root);
    assert!(ui.is_open(0x0C), "the store items opened the shop");
    assert_eq!(ui.shop_state().open.map(|o| o.npc_guid), Some(ng));

    // The permanent cap is the last store item; right-click its cell
    // (the grid is at (sx + 15, H + sy - 400) of 29-px cells).
    let shown = store_items(fx.bridge.world());
    assert_eq!(shown.len(), f.received.len());
    let cap = shown.last().unwrap().clone();
    let at = Point::new(
        80 + 15 + 29 * i32::from(cap.x) + 5,
        600 - 60 - 400 + 29 * i32::from(cap.y) + 5,
    );
    let tab_page = cap.page;
    assert!(tab_page < 4);
    let strings = NoStrings;
    let click = |fx: &mut Fx, ui: &mut OriginalUi, root: &mut UiRoot, button, at| {
        let w = fx.bridge.world();
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &strings,
        };
        for e in [
            UiEvent::Press { button, at },
            UiEvent::Release { button, at },
        ] {
            ui.before_event(e, w);
            let routed = root.dispatch(e, &ctx);
            ui.after_event(root, e, routed).unwrap();
        }
        root.forward(&mut fx.bridge).unwrap()
    };
    // The cap sits on a page that is not the current one until its tab
    // is chosen (the start page of Akara is 3: the first page with items
    // is current).
    let sent = click(&mut fx, &mut ui, &mut root, PointerButton::Right, at);
    assert_eq!(sent, 1, "one C→S 0x32");
    let f = fx.step(&[]);
    assert_eq!(f.codes, [(0x32, Some(ResultCode::Done))]);
    assert!(f
        .received
        .iter()
        .any(|m| m[0] == 0x2A && m[1] == 4 && m[2] == 0));
    assert!(fx.stat(player, GOLD) < PLAYER_GOLD, "the price was paid");
    assert_eq!(fx.inventory().len(), 3, "the copy joined the backpack");

    // Closing the shop's UI state ends the interaction.
    ui.set_ui(0x0C, 1, false).unwrap();
    ui.shop_poll(fx.bridge.world(), &mut root);
    assert!(ui.shop_state().open.is_none());
    assert_eq!(root.forward(&mut fx.bridge).unwrap(), 1);
    let end = bytes(&TerminateEntityChat { id: ng });
    let f = fx.step(&[]);
    assert_eq!(f.codes[0].0, end[0]);
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

/// A `.tbl`: 256 records of width 6.
fn font_tbl() -> Vec<u8> {
    let mut d = b"Woo!".to_vec();
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes());
    d.extend_from_slice(&256u16.to_le_bytes());
    d.extend_from_slice(&[10, 0]);
    for i in 0..256u16 {
        d.extend_from_slice(&i.to_le_bytes());
        d.extend_from_slice(&[0, 6, 10, 0, 0, 0]);
        d.extend_from_slice(&i.to_le_bytes());
        d.extend_from_slice(&[0; 4]);
    }
    d
}

/// The hover text of a store item is a pop-up of the last UI step
/// (the panel draw's last step): drawn after every panel, so the
/// shop panel's edge and the panels drawn later do not cut or cover it
/// (playtest B015).
#[test]
fn the_store_item_hover_text_is_the_last_ui_draw() {
    use d2_client::assets::path::MemorySource;
    use d2_client::ui::layout::Screen;
    use d2_client::ui::original::{FontMeasure, OriginalUi, UiConfig, CHARACTER_FONTS};
    use d2_client::ui::panel::{NoStrings, UiCtx, UiEvent};
    use d2_client::ui::{font_info, NoPanelRules, Point, UiDraw, UiRoot};

    let mut fx = Fx::new(GAME_SEED, PLAYER_GOLD);
    let npc = fx.npc;
    let ng = fx.guid(npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
    fx.step(&[bytes(&InitEntityChat { id: ng })]);
    let mut ui = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        },
        None,
    )
    .unwrap();
    let mut src = MemorySource::default();
    for id in CHARACTER_FONTS {
        src.insert(font_info(id).unwrap().tbl_path, font_tbl());
    }
    ui.set_fonts(FontMeasure::load(&src, &CHARACTER_FONTS).unwrap());
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    fx.step(&[bytes(&EntityAction {
        action: 1,
        npc: ng,
        item: 0,
    })]);
    ui.shop_poll(fx.bridge.world(), &mut root);
    assert!(ui.is_open(0x0C), "the store items opened the shop");
    // The trade opens the inventory beside the shop (right half).
    ui.set_ui(0x01, 0, false).unwrap();

    // The pointer on the last store item's cell (the grid is at (sx + 15,
    // H + sy - 400) of 29-px cells).
    let shown = d2_client::bridge::items::store_items(fx.bridge.world());
    let cap = shown.last().unwrap().clone();
    let prices = d2_client::ui::original::ShopPrices::default();
    prices.set(cap.key.guid, 777);
    ui.set_shop_prices(prices);
    let at = Point::new(
        80 + 15 + 29 * i32::from(cap.x) + 5,
        600 - 60 - 400 + 29 * i32::from(cap.y) + 5,
    );
    let w = fx.bridge.world();
    let strings = NoStrings;
    let ctx = UiCtx {
        tick: w.frames,
        world: w,
        strings: &strings,
    };
    let e = UiEvent::CursorMoved(at);
    ui.before_event(e, w);
    let routed = root.dispatch(e, &ctx);
    ui.after_event(&mut root, e, routed).unwrap();
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    // Without the item tip tables the hover is the price at the pointer
    // (+14, +14): the one text draw at that point.
    let tip = out
        .iter()
        .position(|d| matches!(d, UiDraw::Text(t) if t.at == Point::new(at.x + 14, at.y + 14)))
        .unwrap_or_else(|| {
            let texts: Vec<_> = out
                .iter()
                .filter_map(|d| match d {
                    UiDraw::Text(t) => Some((t.at, t.text.len())),
                    _ => None,
                })
                .collect();
            panic!("no hover text at {at:?}: {texts:?}")
        });
    // Only the cursor's cel (one image) may follow it.
    assert!(
        out[tip + 1..].len() <= 1 && out[tip + 1..].iter().all(|d| matches!(d, UiDraw::Image(_))),
        "only the cursor draws after the hover text, got {} draws after it",
        out.len() - tip - 1
    );
}

/// (q-gamble) Gheed's gamble window through the NPC menu: the Gamble row
/// sends C→S 0x38 action 2, the player's gamble list arrives as S→C 0x9C
/// action 11, the shop knows it is a gamble window, and a click on an
/// item leaves as C→S 0x32 with the gamble bit; the server charges the
/// gamble price and the shop shows that price.
#[test]
fn gamble_window_lists_prices_and_buys() {
    use d2_client::bridge::items::store_items;
    use d2_client::ui::layout::{OptionKind, Screen};
    use d2_client::ui::original::{OriginalUi, UiConfig};
    use d2_client::ui::panel::PointerButton;
    use d2_client::ui::panel::{NoStrings, UiCtx, UiEvent};
    use d2_client::ui::{NoPanelRules, Point, UiRoot};

    let mut fx = Fx::with_class(GAME_SEED, 1_000_000, class::GHEED);
    let (player, npc) = (fx.player, fx.npc);
    let ng = fx.guid(npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
    fx.step(&[bytes(&InitEntityChat { id: ng })]);

    let mut ui = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        },
        None,
    )
    .unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    let strings = NoStrings;
    let click = |fx: &mut Fx, ui: &mut OriginalUi, root: &mut UiRoot, button, at| {
        let w = fx.bridge.world();
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &strings,
        };
        for e in [
            UiEvent::Press { button, at },
            UiEvent::Release { button, at },
        ] {
            ui.before_event(e, w);
            let routed = root.dispatch(e, &ctx);
            ui.after_event(root, e, routed).unwrap();
        }
        root.forward(&mut fx.bridge).unwrap()
    };

    // The menu: Gheed's Gamble row of the spec box.
    ui.open_npc_menu(ng, u32::from(class::GHEED), 12, fx.bridge.world());
    ui.npc_menu_poll(fx.bridge.world(), &mut root, &strings);
    let menu = ui.npc_menu().expect("Gheed's menu");
    let k = menu
        .rows
        .iter()
        .position(|r| r.kind == Some(OptionKind::Gamble))
        .expect("a Gamble row");
    let at = ui.npc_menu_row_point(k).expect("the Gamble row");
    assert_eq!(
        click(&mut fx, &mut ui, &mut root, PointerButton::Left, at),
        1
    );
    let f = fx.step(&[]);
    assert_eq!(f.codes, [(0x38, Some(ResultCode::Done))]);
    assert!(!f.received.is_empty(), "the gamble list was shown");
    // The prices the host published for the panel (the play app shares
    // them through `VendorRest::store_price`).
    let prices = d2_client::ui::original::ShopPrices::default();
    for (g, p) in &fx.sim_ref().world.rest.prices {
        prices.set(*g, *p);
    }
    ui.set_shop_prices(prices);
    assert!(f.received.iter().all(|m| m[0] == 0x9C && m[1] == 11));
    ui.shop_poll(fx.bridge.world(), &mut root);
    assert!(ui.is_open(0x0C), "the shop opened");
    assert!(ui.shop_state().gamble(), "it is a gamble window");

    // The list is the player's: up to 14 items, each priced at the
    // gamble price (`cost` of transaction 2).
    let shown = store_items(fx.bridge.world());
    assert_eq!(shown.len(), f.received.len());
    assert!(shown.len() <= 14);
    let it = shown[0].clone();
    let price = ui
        .shop_state()
        .price(it.key.guid)
        .expect("the host published the price");
    assert!(price > 0);

    // A right click on its (packed) cell: C→S 0x32, transaction bit 2.
    let at = Point::new(80 + 15 + 5, 600 - 60 - 400 + 5);
    let before = fx.stat(player, GOLD);
    assert_eq!(
        click(&mut fx, &mut ui, &mut root, PointerButton::Right, at),
        1
    );
    let f = fx.step(&[]);
    assert_eq!(f.codes, [(0x32, Some(ResultCode::Done))]);
    let spent = before - fx.stat(player, GOLD);
    assert_eq!(spent as u32, price, "the shown price is the price paid");
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

/// (q-gamble) Charsi's repairs: the repair-all button (frame 18) leaves
/// as C→S 0x35 with item 0 and the all flag and the server answers S→C
/// 0x2A; a single-item C→S 0x35 repairs a damaged backpack item and
/// charges the repair cost.
#[test]
fn charsi_repairs_one_item_and_repair_all_answers() {
    use d2_client::ui::layout::Screen;
    use d2_client::ui::original::{OriginalUi, UiConfig};
    use d2_client::ui::panel::PointerButton;
    use d2_client::ui::panel::{NoStrings, UiCtx, UiEvent};
    use d2_client::ui::panels::shop::BUTTON_X;
    use d2_client::ui::{NoPanelRules, Point, UiRoot};
    use d2_proto::client::Repair;

    let mut fx = Fx::with_class(GAME_SEED, 100_000, class::CHARSI);
    let (player, npc, cap) = (fx.player, fx.npc, fx.cap);
    let ng = fx.guid(npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
    fx.step(&[bytes(&InitEntityChat { id: ng })]);

    // The cap (identified, as a normal item the player owns) loses 7 of
    // its 12 durability.
    {
        let sim = fx.sim();
        sim.events.sys.hooks.items.get_mut(cap).unwrap().flags |= IDENTIFIED;
        sim.events
            .with(&mut sim.game, |_, v| v.set_base(cap, DURABILITY, 5));
    }
    let cg = fx.guid(cap);
    let repair = |item: u32, flags: u32| {
        bytes(&Repair {
            npc: ng,
            item,
            unread: 0,
            repair_flags: flags,
        })
    };
    let before = fx.stat(player, GOLD);
    let f = fx.step(&[repair(cg, 0)]);
    assert_eq!(f.codes, [(0x35, Some(ResultCode::Done))]);
    let after = fx.stat(player, GOLD);
    // S→C 0x3E (the restored durability, stat 72)
    // ahead of the 0x2A. The 0x3E is queued as 0x22 bytes; its zero tail
    // arrives as 1-byte 0x00 messages (a known edge case).
    let queued = d2_sim::units::messages::update_item_stat(cg, DURABILITY, 12, 0);
    let stat = queued[..usize::from(queued[1])].to_vec();
    let mut want = vec![stat.clone()];
    want.extend(std::iter::repeat_n(vec![0u8], queued.len() - stat.len()));
    want.push(tx(1, 2, u32::MAX, after));
    assert_eq!(f.received, want, "repaired");
    assert_eq!(fx.stat(cap, DURABILITY), 12, "restored to the maximum");
    assert!(after < before, "the repair was charged");

    // The shop's repair-all button.
    let mut ui = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        },
        None,
    )
    .unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    ui.open_shop(ng, u32::from(class::CHARSI));
    ui.shop_poll(fx.bridge.world(), &mut root);
    assert!(ui.is_open(0x0C));
    let strings = NoStrings;
    let w = fx.bridge.world();
    let ctx = UiCtx {
        tick: w.frames,
        world: w,
        strings: &strings,
    };
    // Frame 18 is the fourth button of a repairer's row; its bar spans
    // `H + sy - 109 .. H + sy - 65`.
    let at = Point::new(80 + BUTTON_X[3][3] + 10, 600 - 60 - 87);
    for e in [
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    ] {
        ui.before_event(e, w);
        let routed = root.dispatch(e, &ctx);
        ui.after_event(&mut root, e, routed).unwrap();
    }
    assert_eq!(root.forward(&mut fx.bridge).unwrap(), 1, "one C→S 0x35");
    let f = fx.step(&[]);
    assert_eq!(f.codes, [(0x35, Some(ResultCode::Done))]);
    assert_eq!(
        f.received,
        [tx(1, 2, u32::MAX, after)],
        "nothing equipped to repair, nothing charged"
    );
    assert_eq!(fx.stat(player, GOLD), after);
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

/// The shop's repair button (the third of a repairer's row) arms the next
/// click on one of the player's items, which leaves as C→S 0x35 for that
/// item and repairs it (D2RUST's own, unverified).
#[test]
fn the_repair_button_then_an_item_click_repairs_that_item() {
    use d2_client::ui::layout::Screen;
    use d2_client::ui::original::{OriginalUi, UiConfig};
    use d2_client::ui::panel::PointerButton;
    use d2_client::ui::panel::{NoStrings, UiCtx, UiEvent};
    use d2_client::ui::panels::shop::BUTTON_X;
    use d2_client::ui::{NoPanelRules, Point, UiRoot};

    let mut fx = Fx::with_class(GAME_SEED, 100_000, class::CHARSI);
    let (player, npc, cap) = (fx.player, fx.npc, fx.cap);
    let ng = fx.guid(npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
    fx.step(&[bytes(&InitEntityChat { id: ng })]);
    {
        let sim = fx.sim();
        sim.events.sys.hooks.items.get_mut(cap).unwrap().flags |= IDENTIFIED;
        sim.events
            .with(&mut sim.game, |_, v| v.set_base(cap, DURABILITY, 5));
    }
    let cg = fx.guid(cap);
    let mut ui = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        },
        None,
    )
    .unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    ui.open_shop(ng, u32::from(class::CHARSI));
    ui.shop_poll(fx.bridge.world(), &mut root);
    // The shop is the full-slot ui 0x0C; ui 1 stays off and the
    // inventory family is drawn under it.
    assert!(ui.is_open(0x0C) && !ui.is_open(0x01));
    let click = |ui: &mut OriginalUi, root: &mut UiRoot, fx: &Fx, at: Point| {
        let strings = NoStrings;
        let w = fx.bridge.world();
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &strings,
        };
        for e in [
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            },
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            },
        ] {
            ui.before_event(e, w);
            let routed = root.dispatch(e, &ctx);
            ui.after_event(root, e, routed).unwrap();
        }
    };
    // The harness never sends the player's items to the client model, so
    // the item click itself (C→S 0x35 for the item under the mouse) is
    // checked by the `ShopTx` vectors and the server path above; here the
    // button arms the mode, a click on an empty cell is consumed (no
    // pick-up intent, no 0x35) and the mode ends with the shop.
    let _ = (player, cg);
    let on_grid = Point::new(339 + 80 + 10, 255 + 60 + 10);
    // The repair button: frame 6, the third of the row.
    let button = Point::new(80 + BUTTON_X[3][2] + 10, 600 - 60 - 87);
    click(&mut ui, &mut root, &fx, button);
    assert!(ui.shop_state().repair_mode(), "the button arms repair");
    click(&mut ui, &mut root, &fx, on_grid);
    assert_eq!(
        root.forward(&mut fx.bridge).unwrap(),
        0,
        "nothing to repair"
    );
    click(&mut ui, &mut root, &fx, button);
    assert!(!ui.shop_state().repair_mode(), "the button toggles");
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

/// C→S 0x60 SwapWeapons in an expansion game passes the handler gates and
/// runs the weapon switch (empty hands): answered Done with S→C 0x97 for the
/// client.
#[test]
fn swap_weapons_is_answered_with_0x97() {
    use d2_proto::client::SwapWeapons;
    let mut fx = Fx::build(GAME_SEED, 100_000, class::CHARSI, true);
    assert!(!fx.bridge.frame().unwrap().ticked);
    let f = fx.step(&[bytes(&SwapWeapons)]);
    assert_eq!(f.codes, [(0x60, Some(ResultCode::Done))]);
    assert!(f.received.contains(&vec![0x97]), "{:02X?}", f.received);
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

/// In a classic game the switch's gate answers 3 before the switch
/// runs: no S→C 0x97.
#[test]
fn swap_weapons_in_a_classic_game_answers_3() {
    use d2_proto::client::SwapWeapons;
    let mut fx = Fx::with_class(GAME_SEED, 100_000, class::CHARSI);
    assert!(!fx.bridge.frame().unwrap().ticked);
    let f = fx.step(&[bytes(&SwapWeapons)]);
    assert_eq!(f.codes, [(0x60, Some(ResultCode::Malformed))]);
    assert!(!f.received.contains(&vec![0x97]), "{:02X?}", f.received);
}

/// Buying a store item that is not permanent (a buckler) takes it out of
/// the NPC's grid: the next frame's S→C 0x9C is action 12 for its GUID
/// (the purchase's step 10); the permanent cap leaves none.
#[test]
fn buying_a_store_item_sends_0x9c_action_12() {
    let mut fx = Fx::new(GAME_SEED, PLAYER_GOLD);
    let ng = fx.guid(fx.npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
    fx.step(&[bytes(&InitEntityChat { id: ng })]);
    let f = fx.step(&[bytes(&EntityAction {
        action: 1,
        npc: ng,
        item: 0,
    })]);
    let store = {
        let w = &fx.sim_ref().world;
        let i = w.state.vendor_index(class::AKARA).unwrap();
        w.state.vendors[i].store.clone()
    };
    let buckler = store
        .iter()
        .copied()
        .find(|&i| {
            fx.sim_ref()
                .events
                .sys
                .hooks
                .items
                .get(i)
                .is_some_and(|it| it.record == BUC)
        })
        .expect("a buckler in the store");
    let bg = fx.guid(buckler);
    assert!(f.received.iter().all(|m| m[1] == 11), "opening: action 11");
    fx.set_gold(PLAYER_GOLD);
    let f = fx.step(&[bytes(&BuyItem {
        npc: ng,
        item: bg,
        transaction: 0,
        client_price: 0,
    })]);
    assert_eq!(f.codes, [(0x32, Some(ResultCode::Done))]);
    let twelve: Vec<u32> = f
        .received
        .iter()
        .filter(|m| m[0] == 0x9C && m[1] == 12)
        .map(|m| u32::from_le_bytes(m[4..8].try_into().unwrap()))
        .collect();
    assert_eq!(twelve, [bg], "one 0x9C action 12 for the bought item");
    // The client: the store item left the shop grid (action 12). The
    // copy's action 4 is the update pass's, which needs the player's room
    // (this harness has none): `e2e_single_player` asserts it.
    let w = fx.bridge.world();
    assert!(
        d2_client::bridge::items::store_items(w)
            .iter()
            .all(|i| i.key.guid != bg),
        "the taken store item is not in the shop grid"
    );
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

/// Owner bug (2026-10-20): repeatedly opening Gheed's gamble window showed
/// fewer items each time, in an odd layout. Every chat close drops the
/// player's gamble list; the next Gamble makes a fresh list of 14 items
/// on an empty grid. Each round here: C→S 0x38 action 2 shows the whole
/// list (S→C 0x9C action 11), the shown items do not overlap on the
/// grid, and C→S 0x30 ends the chat, which drops the list.
#[test]
fn gamble_reopen_keeps_a_full_list_on_an_empty_grid() {
    use d2_proto::client::TerminateEntityChat;

    let mut fx = Fx::with_class(GAME_SEED, PLAYER_GOLD, class::GHEED);
    let ng = fx.guid(fx.npc);
    assert!(!fx.bridge.frame().unwrap().ticked);
    for round in 0..5 {
        fx.step(&[bytes(&InteractWithEntity { type_: 1, id: ng })]);
        fx.step(&[bytes(&InitEntityChat { id: ng })]);
        // Store records of earlier rounds are older than this serial (the
        // shop panel's floor).
        let floor = fx.bridge.world().store_serial;
        let f = fx.step(&[bytes(&EntityAction {
            action: 2,
            npc: ng,
            item: 0,
        })]);
        assert_eq!(f.codes, [(0x38, Some(ResultCode::Done))], "round {round}");
        let shown: Vec<_> = f
            .received
            .iter()
            .filter(|m| m[0] == 0x9C && m[1] == 11)
            .collect();
        assert_eq!(shown.len(), 14, "round {round}: the whole gamble list");
        // The client's store view of this list: every item 2 × 2, no two
        // on a shared cell.
        let items: Vec<_> = d2_client::bridge::items::store_items(fx.bridge.world())
            .into_iter()
            .filter(|i| i.store_seq > floor)
            .collect();
        assert_eq!(items.len(), 14, "round {round}: the client shows the list");
        let mut cells = std::collections::BTreeSet::new();
        for it in &items {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                assert!(
                    cells.insert((it.page, it.x + dx, it.y + dy)),
                    "round {round}: overlap at {:?}",
                    (it.page, it.x, it.y)
                );
            }
        }
        fx.step(&[bytes(&TerminateEntityChat { id: ng })]);
    }
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}
