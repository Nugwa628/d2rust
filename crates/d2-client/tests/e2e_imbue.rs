//! End-to-end Charsi imbue (Tools of the Trade reward) on the real
//! inventory model: the bridge on its local link over the in-process
//! `d2-server` host, `WiredWorld` with the inventory parts lent to the NPC
//! desk (`Desk::inv`). Synthetic tables only; nothing here is verified.

use std::sync::Arc;

use d2_client::bridge::dispatch::Dispatch;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::local::{LocalLink, PendingSession};
use d2_client::bridge::world::MonsterClass;
use d2_client::bridge::Bridge;
use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemstatcost, Record};
use d2_proto::client::EntityAction;
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
/// Stat ids (`itemstatcost`).
const LEVEL: u16 = 12;
const GOLD: u16 = 14;
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

/// Charsi's monster add S→C 0xAC as the client reads it:
/// mode 1, no components, no type flags, no
/// source unit, no stat list. d2-sim does not build monster adds yet (the
/// server-side fields of a monster add are not specified, see the
/// `wiring::action::switch` module docs), so the harness delivers the
/// add of an NPC without any of those parts; the client needs her unit
/// for 0x28.
fn npc_add(guid: u32, class: u16, (x, y): (i32, i32)) -> Vec<u8> {
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
    /// recorded (Charsi's add, [`npc_add`]).
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
/// on the game seed, Charsi and the player allocated, the player's gold
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
            ..UnitData::default()
        };
        let mut events = ActionSim::new(stat_data(), data, hooks);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();

        // Game creation: the NPC control (its own seed from the game
        // seed) and the quests.
        let mut seed = events.hooks().game_seed;
        let ctl =
            NpcControl::new(&charsi_monstats(), Vec::new(), false, 0, &mut seed).expect("npc");
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
        let npc = alloc(UnitType::Monster, u32::from(class::CHARSI));
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
            inject: vec![npc_add(npc_guid, class::CHARSI, (0, 0))],
        };
        let mut bridge = Bridge::with_dispatch(tap, Dispatch::from_spec().unwrap()).unwrap();
        // Charsi's class row in the client tables (an `interact` NPC), so
        // her add creates the unit.
        let mut tables = bridge.inputs().tables.clone();
        tables.monsters = vec![None; usize::from(class::CHARSI) + 1];
        tables.monsters[usize::from(class::CHARSI)] = Some(MonsterClass {
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

/// Monstats: Charsi is `npc` and `interact`.
fn charsi_monstats() -> Vec<d2_data::tables::Monstats> {
    let mut v = monstats();
    v[usize::from(class::CHARSI)].npc = true;
    v[usize::from(class::CHARSI)].interact = true;
    v
}

const IMBUE: u32 = 0;
/// Quest slot 3 (Tools of the Trade) bit 1 (reward pending, gate of the imbue).
const GATE: (u8, u8) = (3, 1);

fn pickup(fx: &mut Fx, item: UnitId) -> Frame {
    let mut m = vec![0x19];
    m.extend(fx.guid(item).to_le_bytes());
    fx.step(&[m])
}

fn imbue(fx: &mut Fx, item: UnitId) -> Frame {
    let (ng, ig) = (fx.guid(fx.npc), fx.guid(item));
    fx.step(&[bytes(&EntityAction {
        action: IMBUE,
        npc: ng,
        item: ig,
    })])
}

#[test]
fn charsi_imbues_the_cursor_item_and_the_quest_reward_is_taken() {
    let mut fx = Fx::new(GAME_SEED, 0);
    let (player, buckler) = (fx.player, fx.buckler);
    assert!(!fx.bridge.frame().unwrap().ticked);
    {
        let w = &mut fx.sim().world;
        // Only a record with bitfield1 bit 0 ("may be magic") can be imbued.
        w.tables.items[BUC].bitfield1 |= 1;
        w.rest.quests.get_mut(&player).unwrap().flags[0].set(GATE.0, GATE.1);
    }
    {
        let sim = fx.sim();
        sim.events
            .with(&mut sim.game, |_, v| v.set_base(player, LEVEL, 10));
    }
    pickup(&mut fx, buckler);
    // The dialog takes no click within 400 ms of its open: the client
    // frames run on (40 ms each).
    for _ in 0..10 {
        fx.step(&[]);
    }
    assert_eq!(
        fx.sim_ref()
            .world
            .inventory
            .as_ref()
            .unwrap()
            .state
            .cursor_of(player),
        Some(buckler)
    );

    let f = imbue(&mut fx, buckler);
    let ng = fx.guid(fx.npc);
    // S→C 0x58: [npc guid][6 done].
    let done: Vec<&Vec<u8>> = f.received.iter().filter(|m| m[0] == 0x58).collect();
    assert_eq!(done.len(), 1, "{:?} {:?}", f.received, fx.errors());
    assert_eq!(done[0][1..5], ng.to_le_bytes());
    assert_eq!(done[0][5], 6, "done; errors: {:?}", fx.errors());
    // The buckler left the cursor and its unit is gone; a new item of the
    // same record, at the imbue's item level (base level 10 + 4), is in
    // the backpack. D2RUST's own, unverified: the synthetic item tables carry
    // no rare affix data, so quality 6 downgrades here (the request asks
    // for 6).
    let items = fx.inventory();
    assert_eq!(items.len(), 2, "cap + the imbued item: {items:?}");
    assert!(!items.contains(&buckler));
    assert!(fx.sim_ref().game.lists.unit(buckler).is_none());
    let new = items.iter().copied().find(|&i| i != fx.cap).unwrap();
    let it = fx.sim_ref().events.sys.hooks.items.get(new).unwrap();
    assert_eq!((it.record, it.ilvl), (BUC, 14));
    assert_eq!(
        fx.sim_ref()
            .world
            .inventory
            .as_ref()
            .unwrap()
            .state
            .cursor_of(player),
        None
    );
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
    // The imbue clears the pending bit and sets the granted bit.
    let flags = fx.sim_ref().world.rest.quests[&player].flags[0];
    assert!(!flags.get(GATE.0, GATE.1));
}

#[test]
fn charsi_refuses_without_the_quest_reward() {
    let mut fx = Fx::new(GAME_SEED, 0);
    let (player, buckler) = (fx.player, fx.buckler);
    assert!(!fx.bridge.frame().unwrap().ticked);
    fx.sim().world.tables.items[BUC].bitfield1 |= 1;
    pickup(&mut fx, buckler);
    // The dialog takes no click within 400 ms of its open: the client
    // frames run on (40 ms each).
    for _ in 0..10 {
        fx.step(&[]);
    }
    let f = imbue(&mut fx, buckler);
    let r: Vec<_> = f.received.iter().filter(|m| m[0] == 0x58).collect();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0][5], 7, "refused");
    assert!(
        fx.inventory().contains(&buckler)
            || fx
                .sim_ref()
                .world
                .inventory
                .as_ref()
                .unwrap()
                .state
                .cursor_of(player)
                == Some(buckler)
    );
}

/// The client half: Charsi's menu offers
/// Imbue; its row opens the dialog; with the buckler on the cursor a
/// click in the item area places it and the imbue button leaves as C→S
/// 0x38 action 0, which the server answers with S→C 0x58 `[npc][6]`; the
/// 0x58 closes the dialog.
#[test]
fn menu_click_to_imbue_done() {
    use d2_client::ui::layout::Screen;
    use d2_client::ui::original::{OriginalUi, UiConfig};
    use d2_client::ui::panel::{NoStrings, PointerButton, UiCtx, UiEvent};
    use d2_client::ui::{NoPanelRules, Point, UiRoot};

    let mut fx = Fx::new(GAME_SEED, 0);
    let (player, buckler) = (fx.player, fx.buckler);
    assert!(!fx.bridge.frame().unwrap().ticked);
    {
        let w = &mut fx.sim().world;
        w.tables.items[BUC].bitfield1 |= 1;
        w.rest.quests.get_mut(&player).unwrap().flags[0].set(GATE.0, GATE.1);
    }
    {
        let sim = fx.sim();
        sim.events
            .with(&mut sim.game, |_, v| v.set_base(player, LEVEL, 10));
    }
    let ng = fx.guid(fx.npc);
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
    ui.open_npc_menu(ng, 154, 10, fx.bridge.world());
    ui.npc_menu_poll(fx.bridge.world(), &mut root, &NoStrings);
    let menu = ui.npc_menu().expect("the menu is up");
    assert!(
        menu.rows
            .iter()
            .any(|r| r.kind == Some(d2_client::ui::layout::OptionKind::Imbue)),
        "Charsi's menu has an Imbue row"
    );

    let strings = NoStrings;
    let click = |fx: &mut Fx, ui: &mut OriginalUi, root: &mut UiRoot, at: Point| {
        let w = fx.bridge.world();
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &strings,
        };
        let button = PointerButton::Left;
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
    // The Imbue row (Talk, Trade, Imbue) of the spec box.
    let k = menu
        .rows
        .iter()
        .position(|r| r.kind == Some(d2_client::ui::layout::OptionKind::Imbue))
        .unwrap();
    let at = ui.npc_menu_row_point(k).expect("the Imbue row");
    assert_eq!(click(&mut fx, &mut ui, &mut root, at), 0);
    assert!(
        ui.npc_menu().is_none() && ui.npc_menu_up(),
        "the dialog replaced the menu"
    );

    pickup(&mut fx, buckler);
    // The dialog takes no click within 400 ms of its open: the client
    // frames run on (40 ms each).
    for _ in 0..10 {
        fx.step(&[]);
    }
    // The fixture's client model has no item stream, so the placing click
    // (covered by the unit test of `npc_talk`) goes through its seam.
    ui.imbue_place(fx.guid(buckler));
    assert_eq!(
        click(&mut fx, &mut ui, &mut root, Point::new(130, 230)),
        1,
        "one C→S 0x38"
    );
    let f = fx.step(&[]);
    assert_eq!(f.codes[0], (0x38, Some(ResultCode::Done)), "{f:?}");
    let done: Vec<&Vec<u8>> = f.received.iter().filter(|m| m[0] == 0x58).collect();
    assert_eq!(done.len(), 1, "{:?} {:?}", f.received, fx.errors());
    assert_eq!(done[0][5], 6, "done; errors: {:?}", fx.errors());
    assert!(!fx.inventory().contains(&buckler), "the buckler was imbued");
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
    // Delivering the 0x58 to the UI closes the dialog.
    let w = fx.bridge.world();
    ui.apply_output(
        &d2_client::bridge::output::Output::OpenUi {
            guid: ng,
            code: 6,
            arg: 0,
        },
        w,
    )
    .unwrap();
    assert!(!ui.npc_menu_up());
}
