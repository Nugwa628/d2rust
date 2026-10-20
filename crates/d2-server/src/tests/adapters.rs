//! Tests of the adapters on the real `d2-proto` and `d2-sim`: the size vectors,
//! agreement with the TSV-driven fake, the `Intents` lookups, and one
//! single-player host frame end to end (drain, tick, flush). Part of the
//! `d2-server` test suite (see `mod.rs`); it checks that the real adapters behave
//! like the fakes in `fakes.rs` that the other test files rely on.

use d2_sim::game::Game;
use d2_sim::tick::timer::TimerRun;
use d2_sim::tick::{EventDispatch, TickHooks};
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};

use super::fakes::*;
use super::messages::size_vectors;
use crate::adapters::*;
use crate::dispatch::Outcome;
use crate::host::{Handled, Host};
use crate::seams::*;
use crate::transport::{classify, Classified};

// The real protocol tables give the same message sizes as the shared size vectors.
#[test]
fn proto_size_vectors() {
    size_vectors(&ProtoSizes);
}

/// Deterministic test bytes (xorshift); not game randomness.
fn noise(seed: &mut u32, n: usize) -> Vec<u8> {
    (0..n)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            (*seed >> 24) as u8
        })
        .collect()
}

/// Inputs for one id: every length 1..=40 over constant fills, chat-like
/// strings, long buffers and noise.
fn inputs(id: u8, seed: &mut u32) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for fill in [0x00, 0x01, 0x05, 0x7F, 0x80, 0xFD, 0xFE, 0xFF] {
        for n in 1..=40 {
            let mut m = vec![fill; n];
            m[0] = id;
            out.push(m);
        }
    }
    for tail in [0x00, 0x05, 0x80, 0xFF] {
        let mut m = vec![id, 1, 0, b'h', b'i', 0, b'b', b'o', b'b', 0, tail];
        out.push(m.clone());
        m.resize(300, 0);
        out.push(m);
    }
    let mut m26 = vec![id; 10];
    m26.extend_from_slice(b"a\0bc\0");
    out.push(m26);
    for n in [1, 2, 3, 9, 13, 34, 64, 0x200, 0x204] {
        for _ in 0..8 {
            let mut m = noise(seed, n);
            m[0] = id;
            out.push(m);
        }
    }
    out
}

/// Inputs on which `d2-proto` and `fake` disagree, per direction.
fn disagreements(fake: &TsvSizes) -> Vec<String> {
    let mut seed = 0x1234_5678;
    let mut out = Vec::new();
    for id in 0..=0xFFu8 {
        for m in inputs(id, &mut seed) {
            if id <= 0x70 && ProtoSizes.client_size(&m) != fake.client_size(&m) {
                out.push(format!("C→S {m:02X?}"));
            }
            if ProtoSizes.server_size(&m) != fake.server_size(&m) {
                out.push(format!("S→C {m:02X?}"));
            }
        }
    }
    out
}

#[test]
fn proto_sizes_agree_with_the_tsv_fake() {
    assert_eq!(disagreements(&TsvSizes::new()), Vec::<String>::new());
}

/// METHODS M08: one changed rule per direction is reported.
#[test]
fn agreement_catches_perturbations() {
    let mut fake = TsvSizes::new();
    fake.client[0x01] = Rule::Fixed(6);
    assert!(disagreements(&fake)
        .iter()
        .all(|d| d.starts_with("C→S [01")));
    assert!(!disagreements(&fake).is_empty());
    let mut fake = TsvSizes::new();
    fake.server[0x16] = parse_rule("u16@1;min=12");
    assert!(disagreements(&fake)
        .iter()
        .all(|d| d.starts_with("S→C [16")));
    assert!(!disagreements(&fake).is_empty());
}

// A chat message whose declared length is negative gets its own size result.
#[test]
fn negative_chat_size_is_its_own_result() {
    let chat = [0x15, 0x01, 0x00, b'h', b'i', 0, b'b', b'o', b'b', 0, 0x80];
    assert_eq!(
        ProtoSizes.client_size(&chat),
        Err(SizeError::Negative(-117))
    );
    assert_eq!(
        classify(&ProtoSizes, &chat, true),
        Classified::NegativeSize(-117)
    );
}

const ALIVE_PLAYER: PlayerFields = PlayerFields {
    gate: ALIVE,
    data: Some(PlayerData { last_accept: 0 }),
};

/// A game with act 0, one active room, a player at (100, 100) for
/// transport client 0, a monster at (150, 100) and an item the player
/// owns.
struct World<D: EventDispatch> {
    sim: SimGame<D>,
    player: UnitId,
    monster: UnitId,
    item: UnitId,
}

fn world<D: EventDispatch>(events: D) -> World<D> {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let room = game.lists.create_room(0).unwrap();
    game.lists.activate_room(room).unwrap();
    let player = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    let monster = game
        .spawn_unit(UnitType::Monster, Some(room), false)
        .unwrap();
    let item = game.spawn_unit(UnitType::Item, None, false).unwrap();
    let mut sim = SimGame::with_events(game, events);
    sim.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    sim.set_player(player, ALIVE_PLAYER);
    let at = |x, y, owner| UnitFacts {
        act: 0,
        pos: Pos { x, y },
        owner,
    };
    sim.set_unit(player, at(100, 100, None));
    sim.set_unit(monster, at(150, 100, None));
    sim.set_unit(item, at(0, 0, Some(player)));
    World {
        sim,
        player,
        monster,
        item,
    }
}

fn guid(sim: &SimGame<impl EventDispatch>, u: UnitId) -> u32 {
    sim.game.lists.unit(u).unwrap().guid
}

// Player lookup by client id: in-game player, or not in game.
#[test]
fn player_lookup() {
    let mut w = world(Unspecified);
    assert_eq!(w.sim.player(0), PlayerLookup::Player(ALIVE));
    assert_eq!(w.sim.player(7), PlayerLookup::NotInGame);
    // A client whose player is a monster, and one without a player.
    w.sim
        .join(1, Some(w.monster), None, client_state::IN_GAME)
        .unwrap();
    w.sim.join(2, None, None, client_state::IN_GAME).unwrap();
    assert_eq!(w.sim.player(1), PlayerLookup::NoPlayer);
    assert_eq!(w.sim.player(2), PlayerLookup::NoPlayer);
    assert_eq!(
        w.sim.join(2, None, None, 0),
        Err(AdapterError::AlreadyJoined(2))
    );
    w.sim.leave(2).unwrap();
    assert_eq!(w.sim.player(2), PlayerLookup::NotInGame);
    assert_eq!(w.sim.leave(2), Err(AdapterError::NotJoined(2)));
}

// The client list follows the sim's client list order.
#[test]
fn client_order_is_the_sim_client_list() {
    let mut w = world(Unspecified);
    w.sim.join(5, None, None, client_state::JOINING).unwrap();
    w.sim.join(3, None, None, client_state::JOINING).unwrap();
    // Joining clients are prepended to the sim's client list.
    assert_eq!(w.sim.clients(), vec![3, 5, 0]);
    let ids: Vec<_> = [3, 5, 0]
        .iter()
        .map(|&c| w.sim.sim_client(c).unwrap())
        .collect();
    assert_eq!(w.sim.game.lists.clients(), ids);
    w.sim.leave(5).unwrap();
    assert_eq!(w.sim.clients(), vec![3, 0]);
}

#[test]
fn point_state_and_accept() {
    let mut w = world(Unspecified);
    let state = |s: &SimGame| s.point_state(0).unwrap();
    assert_eq!(state(&w.sim).player, Pos { x: 100, y: 100 });
    w.sim.set_point_accept(0, 9);
    assert_eq!(state(&w.sim).last_accept, 9);
    w.sim.set_player(
        w.player,
        PlayerFields {
            gate: ALIVE,
            data: None,
        },
    );
    assert_eq!(w.sim.point_state(0), None);
}

#[test]
fn unit_target_order() {
    let mut w = world(Unspecified);
    let (m, i) = (guid(&w.sim, w.monster), guid(&w.sim, w.item));
    assert_eq!(
        w.sim.unit_target(0, 1, m),
        UnitTarget::At {
            player: Pos { x: 100, y: 100 },
            target: Pos { x: 150, y: 100 },
        }
    );
    assert_eq!(w.sim.unit_target(0, 4, i), UnitTarget::OwnedItem);
    assert_eq!(w.sim.unit_target(0, 1, m + 100), UnitTarget::Missing);
    // Same GUID, wrong type: another hash list.
    assert_eq!(w.sim.unit_target(0, 2, m), UnitTarget::Missing);
    w.sim.set_unit(
        w.monster,
        UnitFacts {
            act: 1,
            pos: Pos { x: 150, y: 100 },
            owner: None,
        },
    );
    assert_eq!(w.sim.unit_target(0, 1, m), UnitTarget::OtherAct);
}

/// Logs every timer run.
#[derive(Default)]
struct RunLog(Vec<TimerRun>);

impl EventDispatch for RunLog {
    fn run_event(&mut self, _game: &mut Game, run: &TimerRun) {
        self.0.push(*run);
    }
}

impl TickHooks for RunLog {}

/// One single-player host frame (drain → tick → flush) on the real
/// adapters, from synthetic vectors for point-message parsing (range and act
/// checks) and for message sizes.
#[test]
fn single_player_host_frame() {
    let mut w = world(RunLog::default());
    let m = guid(&w.sim, w.monster);
    let timer = w
        .sim
        .game
        .schedule_event(w.monster, 0, 1, None, 0, 0)
        .unwrap()
        .unwrap();
    let mut host = Host::new(w.sim, ProtoSizes, FakeSession::default(), ManualClock(1000));
    host.connect(0);

    // Frame 1: the first driver call only sets `last`; no tick, no flush.
    let accepted = [0x01, 150, 0, 50, 0]; // (x+50, y−50)
    let refused = [0x01, 151, 0, 100, 0]; // (x+51, y)
    let mut unit = vec![0x04, 1, 0, 0, 0];
    unit.extend_from_slice(&m.to_le_bytes());
    for msg in [&accepted[..], &refused, &unit, &[0x01, 0, 0, 0]] {
        host.send_game(0, msg).unwrap();
    }
    host.send_system(0, &[0x6B]).unwrap();
    // Server messages produced before the tick: no `d2-sim` step sends
    // one yet, so the test queues them (0x1A AddExpByte, 0xAF 00).
    host.buffers.queue(0, &[0x1A, 0x05]).unwrap();
    host.buffers.queue(0, &[0xAF, 0x00]).unwrap();
    let r = host.frame().unwrap();
    let got: Vec<_> = r.messages.iter().map(|h| (h.id, h.handled)).collect();
    use ResultCode::*;
    assert_eq!(
        got,
        vec![
            (0x6B, Handled::System),
            (0x01, Handled::Game(Outcome::Dispatched(Done))),
            (0x01, Handled::Game(Outcome::Dispatched(Refused))),
            (0x04, Handled::Game(Outcome::Dispatched(Done))),
        ]
    );
    assert!(!r.ticked);
    assert_eq!(host.game.unhandled, vec![(0, 0x01, 5), (0, 0x04, 9)]);
    assert_eq!(host.game.resyncs, Vec::<ClientId>::new());
    assert_eq!(host.receive(0), Vec::<Vec<u8>>::new());

    // Frame 2, 40 ms later: tick (frame 1, the monster's timer runs),
    // then flush; delivery splits the buffer with d2-proto's sizes,
    // system list first.
    host.clock.0 += 40;
    let r = host.frame().unwrap();
    assert!(r.ticked);
    assert_eq!((r.flushed_buffers, r.discarded_bytes), (1, 0));
    assert_eq!(host.game.game.frame, 1);
    let runs = &host.game.events.0;
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].timer, runs[0].owner.unit), (timer, w.monster));
    assert_eq!(host.receive(0), vec![vec![0xAF, 0x00], vec![0x1A, 0x05]]);

    // 26 more ticks: a refused point now resyncs (> 25 frames since the
    // accept at frame 0).
    for _ in 0..26 {
        host.clock.0 += 40;
        assert!(host.frame().unwrap().ticked);
    }
    assert_eq!(host.game.game.frame, 27);
    host.send_game(0, &refused).unwrap();
    host.clock.0 += 40;
    host.frame().unwrap();
    assert_eq!(host.game.resyncs, vec![0]);
    let p = host.game.player_fields(w.player).unwrap();
    assert_eq!(p.data, Some(PlayerData { last_accept: 0 }));
}

/// A world that knows where its units are (the action wiring's path
/// records), for the point / unit parse refresh.
#[derive(Default)]
struct LiveWorld(std::collections::BTreeMap<UnitId, UnitFacts>);

impl<D> crate::adapters::handlers::world::WorldHost<D> for LiveWorld {
    fn live_facts(&mut self, _game: &Game, _events: &mut D, unit: UnitId) -> Option<UnitFacts> {
        self.0.get(&unit).copied()
    }
    fn fault(&mut self, _fault: crate::adapters::handlers::world::WorldFault) {}
}

/// q-proto-audit (rubber-banding): the app stages its player at (0, 0)
/// act 0 and the tick only moves the staged position. The point-message
/// parse (range and act checks) reads the world's live position and act whenever the
/// world has the unit, so an in-range walk is accepted (not refused, and
/// no S→C 0x15 after 25 frames) and a target in the player's real act is
/// not "other act".
#[test]
fn point_parse_reads_the_live_position_over_a_stale_staged_one() {
    let w = world(Unspecified);
    let (player, monster) = (w.player, w.monster);
    let m = guid(&w.sim, monster);
    let mut live = LiveWorld::default();
    let at = |act, x, y| UnitFacts {
        act,
        pos: Pos { x, y },
        owner: None,
    };
    live.0.insert(player, at(1, 5000, 4000));
    live.0.insert(monster, at(1, 5030, 4000));
    let mut sim = SimGame::with_world(w.sim.game, Unspecified, live);
    sim.join(0, Some(player), None, client_state::IN_GAME)
        .unwrap();
    sim.set_player(player, ALIVE_PLAYER);
    // The join's staged copy: (0, 0), act 0 (`single_player.rs` join).
    sim.set_unit(player, at(0, 0, 0));
    sim.set_unit(monster, at(0, 0, 0));
    let mut out = crate::buffers::ClientBuffers::default();
    let walk = [0x01, 0xBA, 0x13, 0xA0, 0x0F]; // (5050, 4000): in range
    for frame in [0, 30] {
        sim.game.frame = frame;
        let code = crate::dispatch::dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &walk, 5);
        assert_eq!(code, ResultCode::Done, "frame {frame}");
    }
    assert_eq!(sim.resyncs, Vec::<ClientId>::new());
    assert_eq!(sim.point_state(0).unwrap().player, Pos { x: 5000, y: 4000 });
    let mut unit = vec![0x04, 1, 0, 0, 0];
    unit.extend_from_slice(&m.to_le_bytes());
    let code = crate::dispatch::dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &unit, 9);
    assert_eq!(code, ResultCode::Done);
    assert_eq!(
        sim.unit_target(0, 1, m),
        UnitTarget::At {
            player: Pos { x: 5000, y: 4000 },
            target: Pos { x: 5030, y: 4000 },
        }
    );
    // Out of range (51 sub-tiles) is still refused.
    let far = [0x01, 0xBB, 0x13, 0xA0, 0x0F];
    let code = crate::dispatch::dispatch(&mut sim, &ProtoSizes, &mut out, 0, ALIVE, &far, 5);
    assert_eq!(code, ResultCode::Refused);
}
