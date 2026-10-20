//! The bridge on the in-process server ([`LocalLink`]), headless: a real
//! `d2-server` host (`d2-proto` sizes, the tick driver, the flush) over
//! `SimGame` on the wired `d2-sim` (`ActionSim`, a two-act synthetic
//! DRLG, a fixed seed). The waypoint handler (C→S 0x49) is the
//! server-owned intent: travelling sends
//! S→C 0x0D. Only the action wiring's `Pending` seams (positions,
//! interaction, warp, transport) are a test provider, as in the server's
//! own waypoint tests. Handlers registered here are synthetic.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Levels, Objects, Record};
use d2_proto::client::{PlayAudio, TakeOrCloseWp};
use d2_proto::transport::Classified as ProtoClassified;
use d2_proto::PROTOCOL_VERSION;
use d2_server::adapters::handlers::world::{ActionWorld, Outbox};
use d2_server::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame};
use d2_server::dispatch::Outcome;
use d2_server::host::{Handled, Host};
use d2_server::seams::{Clock, PlayerGate, ResultCode};
use d2_sim::combat::CombatTables;
use d2_sim::drlg::room::LinkAt;
use d2_sim::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind, TileInfo, TileRect, TileSource,
};
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::skills::SkillTables;
use d2_sim::stats::StatData;
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending};
use d2_sim::world::waypoints::{WaypointData, NO_WAYPOINT};

use super::dispatch::{Dispatch, HandlerError, Message};
use super::intent::IntentError;
use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink, LOCAL_CLIENT};
use super::local::{LocalLink, PendingSession};
use super::world::{ClientUnit, ClientWorld, UnitKey};
use super::{Bridge, BridgeError};

/// The action wiring's seams without a provider: positions, warp and
/// arrival mode (logged), and the transport (`send`, handed to the world
/// handlers through [`Outbox`]).
#[derive(Default)]
struct TestPending {
    pos: BTreeMap<UnitId, (i32, i32)>,
    sent: Vec<(UnitId, Vec<u8>)>,
    log: Vec<String>,
}

impl Pending for TestPending {
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
}

impl Outbox for TestPending {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// One 8×8-tile floor room per listed level.
struct Types(BTreeMap<u32, TileRect>);

impl LevelTypes for Types {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = drlg.level(level).id;
        if let Some(&rect) = self.0.get(&id) {
            let r = drlg.alloc_room(level, RoomKind::Preset, rect);
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

struct Tiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation: o,
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

fn tiles() -> Tiles {
    use d2_sim::drlg::tiles::FIXED_LIBRARY;
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
    Tiles(t)
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Rogue Encampment (act 0, waypoint index 0; it holds the waypoint
/// object), Cold Plains (act 0, waypoint index 1) and Lut Gholein (act 1).
const ROGUE_ENCAMPMENT: u32 = 1;
const COLD_PLAINS: u32 = 3;
const ACT2_TOWN: u32 = 40;
/// The game's fixed seed.
const SEED: u32 = 1234;

/// `levels` rows (count 150) and one waypoint object class 0.
fn waypoint_data() -> WaypointData {
    let mut levels = vec![blank::<Levels>(); 150];
    for (i, l) in levels.iter_mut().enumerate() {
        l.waypoint = NO_WAYPOINT;
        l.act = if i >= 40 { 1 } else { 0 };
    }
    levels[ROGUE_ENCAMPMENT as usize].waypoint = 0;
    levels[COLD_PLAINS as usize].waypoint = 1;
    levels[ACT2_TOWN as usize].waypoint = 9;
    let mut o: Objects = blank();
    o.operatefn = 23;
    o.initfn = 17;
    o.framecnt1 = 15 << 8;
    WaypointData::new(&levels, &[o])
}

type Sim = SimGame<ActionSim<TestPending>, ActionWorld>;

/// Manual host clock (ms), injected into the host.
struct Ms(u32);

impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type Link = LocalLink<Sim, ProtoSizes, PendingSession, Ms>;

/// The local game: a sorceress (class 1) for the local client at
/// (42, 20) in Cold Plains, a waypoint object at (20, 20) in the Rogue
/// Encampment (same act, so travelling to Cold Plains is not the
/// object's own level:), and her Cold
/// Plains waypoint known. Returns the game, the player and the
/// waypoint GUID.
fn game() -> (Sim, UnitId, u32) {
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
    for id in [ROGUE_ENCAMPMENT, COLD_PLAINS, ACT2_TOWN] {
        data.levels[id as usize].drlg_type = 2;
        data.levels[id as usize].level_type = 1;
    }
    let mut types = Types(BTreeMap::from([
        (ROGUE_ENCAMPMENT, TileRect::new(0, 0, 8, 8)),
        (COLD_PLAINS, TileRect::new(0, 0, 8, 8)),
        (ACT2_TOWN, TileRect::new(0, 0, 8, 8)),
    ]));
    let mut dungeon = Dungeon::default();
    dungeon.acts[0] = Some(Drlg::create(0, 1, 0, 0, false, &data, &mut types).unwrap());
    dungeon.acts[1] = Some(Drlg::create(1, 2, 0, 0, false, &data, &mut types).unwrap());
    let world = DrlgWorld {
        dungeon,
        data: Arc::new(data),
        tiles: Box::new(tiles()),
        types: Box::new(types),
    };
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
    let hooks = ActionHooks::new(
        Arc::new(tables),
        world,
        Seed::init_low(SEED),
        TestPending::default(),
    );
    let mut sim = ActionSim::new(Arc::new(StatData::default()), UnitData::default(), hooks);
    let mut game = Game::new();
    let mut rooms = Vec::new();
    for (act, level) in [(0u8, COLD_PLAINS), (1, ACT2_TOWN), (0, ROGUE_ENCAMPMENT)] {
        game.lists.ensure_act(act).unwrap();
        let r = sim
            .hooks()
            .drlg
            .with_act(act, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, level)?;
                d.generate_level(svc.data, svc.types, l)?;
                let r = d.level_rooms(l)[0];
                d.stream_room(svc, r)
            })
            .unwrap()
            .unwrap()
            .unwrap();
        rooms.push(r);
    }
    let spawn = |sim: &mut ActionSim<TestPending>,
                 game: &mut Game,
                 ty: UnitType,
                 class: u32,
                 room: RoomId,
                 x: i32| {
        let req = AllocRequest {
            ty,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        sim.with(game, |g, v| v.allocate(g, &req, x, 20)).unwrap()
    };
    let o = spawn(&mut sim, &mut game, UnitType::Object, 0, rooms[2], 20);
    let player = spawn(&mut sim, &mut game, UnitType::Player, 1, rooms[0], 42);
    sim.sys.units.get_mut(player).unwrap().mode = 1;
    sim.hooks()
        .waypoints
        .entry(player)
        .or_default()
        .get_mut(0)
        .set(1)
        .unwrap();
    let wp = game.lists.unit(o).unwrap().guid;
    let mut s: Sim = SimGame::with_events(game, sim);
    s.world.waypoints = Some(waypoint_data());
    s.join(
        LOCAL_CLIENT,
        Some(player),
        Some(rooms[0]),
        client_state::IN_GAME,
    )
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
    (s, player, wp)
}

/// Records every chunk the bridge receives; optionally reports another
/// protocol version.
struct Tap<L> {
    inner: L,
    version: Option<u32>,
    pumps: usize,
    chunks: Vec<Vec<u8>>,
}

impl<L> Tap<L> {
    fn new(inner: L) -> Self {
        Self {
            inner,
            version: None,
            pumps: 0,
            chunks: Vec::new(),
        }
    }
}

impl<L: ServerLink> ServerLink for Tap<L> {
    fn protocol_version(&self) -> u32 {
        self.version
            .unwrap_or_else(|| self.inner.protocol_version())
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }

    fn server_frame(&mut self) -> Option<i32> {
        self.inner.server_frame()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.inner.send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.pumps += 1;
        self.inner.pump()
    }
    fn set_display_catch_up(&mut self, enabled: bool) {
        self.inner.set_display_catch_up(enabled)
    }
    fn tick_pending(&mut self) -> bool {
        self.inner.tick_pending()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        let got = self.inner.receive();
        self.chunks.extend(got.iter().cloned());
        got
    }
}

struct Fx {
    bridge: Bridge<Tap<Link>>,
    player: UnitId,
    wp: u32,
}

impl Fx {
    fn link(&mut self) -> &mut Link {
        &mut self.bridge.link_mut().inner
    }

    fn sim(&mut self) -> &mut Sim {
        &mut self.link().host_mut().game
    }

    fn pending(&mut self) -> &mut TestPending {
        &mut self.sim().events.hooks().x
    }

    /// The player's interact info (the unit record's).
    fn interact(&mut self) -> &mut d2_sim::units::record::InteractInfo {
        let p = self.player;
        &mut self.sim().events.sys.units.get_mut(p).unwrap().interact
    }

    /// Advances the host clock by `ms`.
    fn advance(&mut self, ms: u32) {
        self.link().host_mut().clock.0 += ms;
    }

    fn guid(&mut self) -> u32 {
        let p = self.player;
        self.sim().game.lists.unit(p).unwrap().guid
    }
}

/// The local server on clock 1000 ms, behind a bridge with `dispatch`.
fn fixture(dispatch: Dispatch) -> Fx {
    let (sim, player, wp) = game();
    let link = LocalLink::new(Host::new(
        sim,
        ProtoSizes,
        PendingSession::default(),
        Ms(1000),
    ));
    let bridge = Bridge::with_dispatch(Tap::new(link), dispatch).unwrap();
    Fx { bridge, player, wp }
}

/// Synthetic handler: the addressed unit enters the model.
fn add_unit(world: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let key = msg.unit.ok_or(HandlerError::Invalid("no unit"))?;
    world.units.insert(key, ClientUnit::new(key));
    Ok(())
}

/// S→C 0x0D (13 bytes) for the player arriving at (45, 23): the bytes
/// the server's waypoint test asserts.
fn player_stop(guid: u32) -> Vec<u8> {
    let mut want = vec![0x0D, 0x00];
    want.extend_from_slice(&guid.to_le_bytes());
    want.push(1);
    want.extend_from_slice(&45u16.to_le_bytes());
    want.extend_from_slice(&23u16.to_le_bytes());
    want.extend_from_slice(&[0, 0]);
    want
}

#[test]
fn waypoint_travel_end_to_end() {
    let mut d = Dispatch::empty();
    d.set(0x0D, "test", add_unit);
    let mut fx = fixture(d);

    // Frame 1 at 1000 ms: the driver starts its clock, no tick.
    let r = fx.bridge.frame().unwrap();
    assert!(!r.ticked);
    assert_eq!(r.chunks, 0);

    // The waypoint menu is open; the client asks to travel to Cold
    // Plains. The bytes reach the server queue during the frame.
    let (p, wp) = (fx.player, fx.wp);
    fx.interact().set(2, wp);
    let sent = fx
        .bridge
        .send(&TakeOrCloseWp {
            wp,
            level: COLD_PLAINS as u16,
        })
        .unwrap();
    assert_eq!(sent, Sent::Queued);
    let mut want_c2s = vec![0x49];
    want_c2s.extend_from_slice(&wp.to_le_bytes());
    want_c2s.extend_from_slice(&[0x03, 0x00, 0x00, 0x00]);
    assert_eq!(
        fx.link()
            .host()
            .queues
            .len(d2_server::transport::Queue::Game),
        1
    );

    // Frame 2 at 1040 ms: drain → handler → tick → flush → split →
    // dispatch.
    fx.advance(40);
    let r = fx.bridge.frame().unwrap();
    let host = fx.link().last_frame().clone();
    assert!(host.ticked);
    assert_eq!(host.messages.len(), 1);
    assert_eq!(host.messages[0].id, 0x49);
    assert_eq!(host.messages[0].size, want_c2s.len());
    assert_eq!(
        host.messages[0].handled,
        Handled::Game(Outcome::Dispatched(ResultCode::Done))
    );
    let guid = fx.guid();
    assert_eq!(fx.bridge.link().chunks, vec![player_stop(guid)]);
    assert_eq!(
        (r.ticked, r.chunks, r.messages, r.handled, r.unowned),
        (true, 1, 1, 1, 0)
    );

    // The client model: the addressed player unit (type 0, its GUID).
    let key = UnitKey { unit_type: 0, guid };
    let world = fx.bridge.world();
    assert_eq!((world.frames, world.server_ticks), (2, 1));
    assert_eq!(world.units.keys().copied().collect::<Vec<_>>(), vec![key]);

    // The outcome is the server's: the sim warped the player and closed
    // the menu.
    assert_eq!(
        fx.pending().log,
        [format!("warp {} 3 0", p.0), format!("arrival mode {}", p.0)]
    );
    assert_eq!(fx.interact().get(), None);
    assert!(fx.sim().world.faults.is_empty());
    assert!(fx.sim().unhandled.is_empty());

    // Frame 3 without time passing: no tick, nothing delivered.
    let r = fx.bridge.frame().unwrap();
    assert_eq!((r.ticked, r.chunks), (false, 0));
    assert_eq!(fx.bridge.world().server_ticks, 1);
}

#[test]
fn spec_table_drops_a_unit_message_for_an_unknown_unit() {
    let mut fx = fixture(Dispatch::from_spec().unwrap());
    fx.bridge.frame().unwrap();
    let wp = fx.wp;
    fx.interact().set(2, wp);
    fx.bridge
        .send(&TakeOrCloseWp {
            wp,
            level: COLD_PLAINS as u16,
        })
        .unwrap();
    fx.advance(40);
    let r = fx.bridge.frame().unwrap();
    let guid = fx.guid();
    assert_eq!(fx.bridge.link().chunks, vec![player_stop(guid)]);
    // 0x0D has a unit handler; the player was never announced (no 0x59),
    // so the message is dropped at receive.
    assert_eq!(
        (r.messages, r.handled, r.queued, r.dropped, r.unowned),
        (1, 0, 0, 1, 0)
    );
    assert_eq!(fx.bridge.log().dropped, BTreeMap::from([(0x0D, 1)]));
    assert!(fx.bridge.log().unowned.is_empty());
    assert!(fx.bridge.world().units.is_empty());
}

#[test]
fn duplicate_filter_is_the_links() {
    let mut fx = fixture(Dispatch::empty());
    fx.bridge.frame().unwrap();
    let msg = TakeOrCloseWp {
        wp: fx.wp,
        level: 0,
    };
    assert_eq!(fx.bridge.send(&msg).unwrap(), Sent::Queued);
    // Same bytes within 200 ms of the host clock: dropped by the sender.
    fx.advance(10);
    assert_eq!(fx.bridge.send(&msg).unwrap(), Sent::Filtered);
    fx.advance(30);
    fx.bridge.frame().unwrap();
    assert_eq!(fx.link().last_frame().messages.len(), 1);
}

#[test]
fn protocol_version_check() {
    let (sim, _, _) = game();
    let link = LocalLink::new(Host::new(sim, ProtoSizes, PendingSession::default(), Ms(0)));
    assert_eq!(link.protocol_version(), PROTOCOL_VERSION);
    let mut tap = Tap::new(link);
    tap.version = Some(PROTOCOL_VERSION + 1);
    let Err(err) = Bridge::new(tap) else {
        panic!("a link of another version was accepted");
    };
    assert!(matches!(
        err,
        BridgeError::Version { client, server }
            if client == PROTOCOL_VERSION && server == PROTOCOL_VERSION + 1
    ));
    // The real link is accepted with the dispatch table.
    let (sim, _, _) = game();
    let link = LocalLink::new(Host::new(sim, ProtoSizes, PendingSession::default(), Ms(0)));
    let b = Bridge::new(Tap::new(link)).unwrap();
    assert_eq!(b.link().pumps, 0);
}

#[test]
fn unknown_and_unowned_ids() {
    let mut fx = fixture(Dispatch::from_spec().unwrap());
    fx.bridge.frame().unwrap();

    // C→S id 0x80: refused by the classifier, nothing reaches the host.
    let err = fx.bridge.send_bytes(&[0x80, 0x80, 0x80, 0x80, 0x80]);
    assert!(matches!(
        err,
        Err(BridgeError::Intent(IntentError::NotSendable(
            ProtoClassified::Invalid
        )))
    ));
    assert!(fx.link().host().queues.is_empty());

    // C→S 0x3F: owned by the sim's PlayAudio intent (step 5);
    // sound 7 is outside 25–32 → result 0, nothing done, nothing sent
    // back, and no longer recorded as an unhandled stub.
    fx.bridge.send(&PlayAudio { sound: 7 }).unwrap();
    // A system message: drained to the session handler, unanswered.
    assert_eq!(fx.bridge.send_bytes(&[0x6B]).unwrap(), Sent::Queued);
    // An S→C id with the shared no-op handler (0x79, out of scope),
    // sent by the server directly.
    fx.link()
        .host_mut()
        .send_direct(LOCAL_CLIENT, &[0x79, 1, 2, 3, 4, 5])
        .unwrap();
    fx.advance(40);
    let r = fx.bridge.frame().unwrap();
    let ids: Vec<(u8, Handled)> = fx
        .link()
        .last_frame()
        .messages
        .iter()
        .map(|m| (m.id, m.handled))
        .collect();
    assert_eq!(
        ids,
        [
            (0x6B, Handled::System),
            (0x3F, Handled::Game(Outcome::Dispatched(ResultCode::Done))),
        ]
    );
    assert_eq!(fx.sim().unhandled, vec![]);
    assert_eq!(
        fx.link().host().session.received,
        vec![(LOCAL_CLIENT, vec![0x6B], 1)]
    );
    assert_eq!(fx.bridge.link().chunks, vec![vec![0x79, 1, 2, 3, 4, 5]]);
    assert_eq!((r.chunks, r.unowned, r.handled), (1, 0, 1));
    assert_eq!(fx.bridge.log().unowned, BTreeMap::new());
    assert_eq!(fx.bridge.world().units, BTreeMap::new());
}

#[test]
fn local_bridge_uses_server_game_frame_after_joining_late() {
    let mut fx = fixture(Dispatch::empty());
    fx.sim().game.frame = 1001;
    fx.bridge.frame().unwrap();
    assert_eq!(fx.bridge.world.server_frame, Some(1001));
    assert_eq!(fx.bridge.world.server_ticks, 0);
    fx.advance(40);
    fx.bridge.frame().unwrap();
    assert_eq!(fx.bridge.world.server_frame, Some(1002));
    assert_eq!(fx.bridge.world.server_ticks, 1);
}

/// Exercise the shipped thread/boxed-link path without a window or game assets.
fn display_fixture() -> (
    Bridge<super::mirror::DynLink>,
    Arc<std::sync::atomic::AtomicU32>,
) {
    use crate::app::server_thread::ThreadLink;
    use std::sync::atomic::{AtomicU32, Ordering};
    struct SharedMs(Arc<AtomicU32>);
    impl Clock for SharedMs {
        fn now_ms(&mut self) -> u32 {
            self.0.load(Ordering::Relaxed)
        }
    }
    let now = Arc::new(AtomicU32::new(1000));
    let clock = now.clone();
    let link = ThreadLink::spawn(move || {
        let (mut sim, _, _) = game();
        sim.game.tick_rate = d2_sim::tick::TickRate::new(60).unwrap();
        Ok::<_, String>(LocalLink::new(Host::new(
            sim,
            ProtoSizes,
            PendingSession::default(),
            SharedMs(clock),
        )))
    })
    .unwrap();
    let link: super::mirror::DynLink = Box::new(super::predict::PredictLink::new(link));
    (Bridge::with_dispatch(link, Dispatch::empty()).unwrap(), now)
}

#[test]
fn slow_display_frames_still_run_sixty_ticks_per_second() {
    use std::sync::atomic::Ordering;
    for fps in [4, 5, 10, 15, 20, 30, 60, 144] {
        use super::mirror::{BridgePlugin, BridgeResource, InputOnlyPass, ScriptedClock};
        use bevy::prelude::*;
        let (mut bridge, now) = display_fixture();
        bridge.world.in_game = true;
        let key = UnitKey::new(super::world::PLAYER, 999);
        let mut player = ClientUnit::new(key);
        player.mode = 1;
        player.speed = Some(213);
        player.frame_count = 1_000_000;
        bridge.world.units.insert(key, player);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(BridgePlugin)
            .insert_resource(BridgeResource(bridge))
            .insert_resource(ScriptedClock(now.clone()))
            .init_resource::<InputOnlyPass>();
        app.update();
        for frame in 1..=fps * 10 {
            now.store(1000 + frame * 1000 / fps, Ordering::Relaxed);
            app.update();
        }
        let bridge = &app.world().resource::<BridgeResource>().0;
        assert_eq!(bridge.world().server_ticks, 600, "{fps} fps");
        assert_eq!(bridge.world().server_frame, Some(600), "{fps} fps");
        assert_eq!(
            bridge.world().units[&key].frame,
            213 * 250,
            "client updates at {fps} fps"
        );
    }
}

#[test]
fn display_catch_up_is_bounded_and_capture_keeps_one_tick_per_pass() {
    use std::sync::atomic::Ordering;
    let (mut bridge, now) = display_fixture();
    bridge.display_frame(true).unwrap();
    now.store(11000, Ordering::Relaxed);
    bridge.display_frame(true).unwrap();
    assert!((1..=16).contains(&bridge.world().server_ticks));
    let ticks = bridge.world().server_ticks;
    bridge.display_frame(true).unwrap();
    assert_eq!(bridge.world().server_ticks, ticks, "old stall debt dropped");
    now.store(12000, Ordering::Relaxed);
    bridge.display_frame(false).unwrap();
    assert_eq!(bridge.world().server_ticks, ticks + 1);
}

#[test]
fn display_resume_does_not_catch_up_the_paused_time() {
    use std::sync::atomic::Ordering;
    let (mut bridge, now) = display_fixture();
    let key = UnitKey::new(super::world::PLAYER, 999);
    let mut player = ClientUnit::new(key);
    player.position = Some((0, 0));
    bridge.world.units.insert(key, player);
    bridge.world.local_player = Some(key);
    bridge.display_frame(true).unwrap();
    bridge.set_paused(true);
    now.store(11000, Ordering::Relaxed);
    bridge.display_frame(true).unwrap();
    assert_eq!(bridge.world().server_ticks, 0);
    bridge.set_paused(false);
    bridge.display_frame(true).unwrap();
    assert_eq!(bridge.world().server_ticks, 1);
    bridge.display_frame(true).unwrap();
    assert!(
        bridge.world().server_ticks <= 2,
        "paused time was discarded"
    );
}
