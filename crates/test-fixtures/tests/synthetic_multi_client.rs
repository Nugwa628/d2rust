//! Several clients in one game on the synthetic install, in CI: the
//! one-room town of the synthetic data behind one `Host`, a session flow
//! whose loader gives each client a fresh character of its own, and N
//! scripted clients that create-or-join (C→S 0x67), join (C→S 0x6B),
//! play frames and leave (C→S 0x69).
//!
//! What it pins:
//! - every client of a game gets its own player, up to the game's player
//!   limit (8 by default), and the game counts them for monster scaling;
//! - a client past the limit is refused and gets no record;
//! - the limit is one value: raised to 64, 64 clients play one game;
//! - the same script on the same seed gives the same game, byte for byte,
//!   for every client (the host reads clients in id order, then each
//!   client's messages in arrival order), with a leave in the middle.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_server::adapters::handlers::world::ActionWorld;
use d2_server::adapters::session::Entry;
use d2_server::adapters::session_flow::{
    CharacterLoader, CreateGame, CreateRefusal, Loaded, SessionFault, SessionFlow,
    DEFAULT_MAX_PLAYERS,
};
use d2_server::adapters::{ProtoSizes, SimGame};
use d2_server::host::Host;
use d2_server::seams::{ClientId, Clock, MessageSink, SessionHandler};
use d2_sim::game::Game;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::worldgen::WorldSim;
use test_fixtures::game::{ActCreation, GameData, Seams, Sim};
use test_fixtures::{install, synth};

/// The town level of act 1.
const TOWN: u32 = 1;
/// The game's init seed and its seed (any fixed values).
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
/// Expansion (bit 20) and bit 2, the flags every 0x67 here carries.
const EXPANSION_FLAGS: u32 = (1 << 20) | 0x4;
/// Milliseconds per frame (the 25 frames a second the host ticks at).
const FRAME_MS: u32 = 40;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("synthetic-multi-client-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

struct NoSession;
impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}

struct Ms(u32);
impl Clock for Ms {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

type TestHost = Host<Sim, ProtoSizes, NoSession, Ms>;

/// The character name of client `c`: `client<c>`, NUL-padded.
fn char_name(c: ClientId) -> [u8; 16] {
    let mut n = [0u8; 16];
    let s = format!("client{c}");
    n[..s.len()].copy_from_slice(s.as_bytes());
    n
}

/// Client `c`'s 0x67: the game "game", Normal, its own character name
/// and a class by client (0–6 in turn).
fn create_request(c: ClientId) -> CreateGame {
    let mut game_name = [0u8; 16];
    game_name[..4].copy_from_slice(b"game");
    CreateGame {
        game_name,
        class: (c % 7) as u8,
        char_name: char_name(c),
        flags: EXPANSION_FLAGS,
        ..CreateGame::default()
    }
}

/// The player as the save loader leaves it: no room, at (0, 0), mode 1.
fn alloc_player(s: &mut Sim, class: u32) -> UnitId {
    let req = AllocRequest {
        ty: UnitType::Player,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = s
        .events
        .action
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .expect("allocated");
    s.events.action.sys.units.get_mut(p).unwrap().mode = 1;
    p
}

/// The synthetic town (act 0 with its town room streamed) behind a host
/// with a session flow of player limit `max_players`; the loader makes
/// each client a new character of the class and name it asked for.
fn host(max_players: usize) -> TestHost {
    let d = data();
    let (mut sim, _) = d
        .world_sim(
            ActCreation::TownOnly,
            INIT,
            TOWN,
            GAME_SEED,
            Seams::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    sim.action
        .hooks()
        .drlg
        .with_act(0, &mut game.lists, |dr, svc| {
            let lv = dr.get_or_alloc_level(svc.data, svc.types, TOWN)?;
            let r = dr.level_rooms(lv)[0];
            dr.stream_room(svc, r)
        })
        .expect("act 0 has a DRLG")
        .unwrap_or_else(|e| panic!("town: {e:?}"))
        .expect("the town room is active");
    let world = ActionWorld {
        waypoints: Some(d.waypoints().unwrap()),
        ..ActionWorld::default()
    };
    let mut s: Sim = SimGame::with_world(game, sim, world);
    let loader: CharacterLoader<WorldSim<Seams>, ActionWorld> =
        Box::new(|s: &mut Sim, _: ClientId, r: &CreateGame| {
            let player = alloc_player(s, u32::from(r.class));
            Ok(Loaded {
                player,
                entry: Entry::new(0, r.char_name),
            })
        });
    s.set_session(SessionFlow::new(loader).with_max_players(max_players));
    Host::new(s, ProtoSizes, NoSession, Ms(1000))
}

/// One scripted game: its host and what each client received so far.
struct Clients {
    host: TestHost,
    got: Vec<Vec<Vec<u8>>>,
}

impl Clients {
    /// `n` connected clients on a fresh host of limit `max_players`.
    fn new(n: usize, max_players: usize) -> Self {
        let mut host = host(max_players);
        for c in 0..n {
            host.connect(c as ClientId);
        }
        Self {
            host,
            got: vec![Vec::new(); n],
        }
    }

    /// `frames` frames, each client's messages collected after each.
    fn step(&mut self, frames: usize) {
        for _ in 0..frames {
            self.host.clock.0 += FRAME_MS;
            self.host.frame().expect("frame");
            for (c, got) in self.got.iter_mut().enumerate() {
                got.extend(self.host.receive(c as ClientId));
            }
        }
    }

    /// Every client sends its 0x67 in the same frame, in client order,
    /// then its 0x6B once the 0x67's answer is out.
    fn join_all(&mut self) {
        for c in 0..self.got.len() as ClientId {
            let r = create_request(c).encode();
            self.host.send_system(c, &r).expect("queued");
        }
        self.step(2);
        for c in 0..self.got.len() as ClientId {
            self.host.send_system(c, &[0x6B]).expect("queued");
        }
        self.step(4);
    }

    fn players(&self) -> Vec<Option<UnitId>> {
        (0..self.got.len() as ClientId)
            .map(|c| self.host.game.player_of(c))
            .collect()
    }

    fn player_count(&self) -> i32 {
        self.host.game.events.world.init_info.players
    }

    fn faults(&self) -> &[(ClientId, SessionFault)] {
        &self.host.game.session().expect("a session flow").faults
    }
}

/// `n` clients join a game of limit `n`: each has its own player of the
/// class it asked for, and the game counts `n` players.
fn every_client_gets_its_own_player(n: usize) {
    let mut g = Clients::new(n, n);
    g.join_all();
    assert_eq!(g.faults(), &[], "no session fault");
    let players: Vec<UnitId> = g
        .players()
        .into_iter()
        .enumerate()
        .map(|(c, p)| p.unwrap_or_else(|| panic!("client {c} has no player")))
        .collect();
    let mut unique = players.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), n, "one player unit per client: {players:?}");
    for (c, &p) in players.iter().enumerate() {
        let class = g.host.game.events.action.sys.units.get(p).map(|u| u.class);
        assert_eq!(class, Some((c % 7) as u32), "client {c}'s class");
    }
    assert_eq!(g.player_count(), n as i32);
}

#[test]
fn two_clients_play_one_game() {
    every_client_gets_its_own_player(2);
}

#[test]
fn eight_clients_play_one_game() {
    every_client_gets_its_own_player(DEFAULT_MAX_PLAYERS);
}

/// The player limit is the only cap: raised to 64, 64 clients play one
/// game (monster target lists, player counts and messages all follow).
#[test]
fn sixty_four_clients_play_one_game_at_a_raised_limit() {
    every_client_gets_its_own_player(64);
}

/// A game at its limit refuses the next client's 0x67: that client gets
/// no record and no player, and the game plays on with its eight.
#[test]
fn a_client_past_the_limit_is_refused() {
    let n = DEFAULT_MAX_PLAYERS;
    let mut g = Clients::new(n + 1, n);
    g.join_all();
    let late = n as ClientId;
    // Its 0x6B then finds no record, as for any client without a game.
    assert_eq!(
        g.faults(),
        &[
            (
                late,
                SessionFault::CreateRefused(CreateRefusal::GameFull(n))
            ),
            (late, SessionFault::NoClientRecord),
        ]
    );
    assert_eq!(g.host.game.sim_client(late), None, "no client record");
    assert!(g.got[n].is_empty(), "nothing sent to the refused client");
    assert!(g.players()[..n].iter().all(Option::is_some));
    assert_eq!(g.player_count(), n as i32);
}

/// A client that leaves frees its place: the next client joins.
#[test]
fn a_leave_makes_room_for_another_client() {
    let n = DEFAULT_MAX_PLAYERS;
    let mut g = Clients::new(n + 1, n);
    g.join_all();
    g.host.send_system(0, &[0x69]).expect("queued");
    g.step(1);
    assert_eq!(g.player_count(), (n - 1) as i32);
    let late = n as ClientId;
    let r = create_request(late).encode();
    g.host.send_system(late, &r).expect("queued");
    g.step(2);
    g.host.send_system(late, &[0x6B]).expect("queued");
    g.step(4);
    assert!(
        g.host.game.player_of(late).is_some(),
        "joined after the leave"
    );
    assert_eq!(g.player_count(), n as i32);
}

/// Four clients join, play, one leaves, the rest play on: what every
/// client received, and the players' units.
fn scripted_run() -> (Vec<Vec<Vec<u8>>>, Vec<Option<UnitId>>) {
    let mut g = Clients::new(4, DEFAULT_MAX_PLAYERS);
    g.join_all();
    g.step(10);
    g.host.send_system(2, &[0x69]).expect("queued");
    g.step(10);
    let players = g.players();
    (g.got, players)
}

/// The same script on the same seed gives the same game for every
/// client, byte for byte: join order, leave and frames are replayable.
#[test]
fn the_same_script_gives_the_same_game() {
    let a = scripted_run();
    let b = scripted_run();
    assert!(
        a.0.iter().all(|got| !got.is_empty()),
        "every client heard the game"
    );
    assert_eq!(a.1[2], None, "client 2 left");
    assert_eq!(a, b);
}
