//! Several clients in one game, in-process: the app's wired game behind
//! one `d2_server::hub::Hub`, each client on its own in-memory transport.
//!
//! The clients act as the bridge does at the start of a game: C→S 0x67
//! (each with its own class and name), then C→S 0x6B once the server's
//! 0x02 arrived. The tests check that every client gets into the game
//! with its own player, that a client can leave while the others play on,
//! and that the same script on the same seed gives the same game, byte
//! for byte, for every client (the hub reads clients in id order, then
//! each client's packets in arrival order).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player::{self, Sim, DEFAULT_SEED};
use d2_client::bridge::local::PendingSession;
use d2_net::{pair, Lane, MemoryEnd, Packet, Transport};
use d2_server::adapters::ProtoSizes;
use d2_server::host::Host;
use d2_server::hub::Hub;
use d2_server::seams::{ClientId, Clock};
use d2_sim::units::lists::client_state;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The classes the clients play, in client order (class ids 0–6).
const CLASSES: [&str; 7] = [
    "amazon",
    "sorceress",
    "necromancer",
    "paladin",
    "barbarian",
    "druid",
    "assassin",
];

/// One game and its clients.
struct Game {
    hub: Hub<Sim, ProtoSizes, PendingSession, StepClock, MemoryEnd>,
    ends: Vec<MemoryEnd>,
    /// Everything each client received, in order (frame marks as `[]`
    /// with the tick flag).
    got: Vec<Vec<Vec<u8>>>,
    /// Whether each client answered 0x02 with its 0x6B yet.
    joined: Vec<bool>,
    ms: Arc<AtomicU32>,
}

impl Game {
    /// The game on `seed` with `n` connected clients, each having sent its
    /// 0x67.
    fn new(seed: u32, n: usize) -> Self {
        let data = app_support::game_data();
        let built = single_player::build(&data, seed).expect("the game builds");
        let ms = Arc::new(AtomicU32::new(1000));
        let host = Host::new(
            built.sim,
            ProtoSizes,
            PendingSession::default(),
            StepClock(ms.clone()),
        );
        let mut hub = Hub::new(host);
        let mut ends = Vec::new();
        for c in 0..n {
            let (end, server_end) = pair();
            hub.connect(c as ClientId, server_end);
            ends.push(end);
        }
        let mut g = Self {
            hub,
            ends,
            got: vec![Vec::new(); n],
            joined: vec![false; n],
            ms,
        };
        for c in 0..n {
            let name = format!("Player{c}");
            let character =
                single_player::new_character(CLASSES[c % CLASSES.len()], &name).unwrap();
            let req = single_player::create_request_for(&character);
            g.send(c, Lane::System, req.encode().to_vec());
        }
        g
    }

    fn send(&mut self, c: usize, lane: Lane, msg: Vec<u8>) {
        self.ends[c].send(Packet::Send { lane, msg }).unwrap();
    }

    /// One frame for everyone: the hub reads the clients, the clock moves
    /// one tick, the host runs its frame and delivers. A client answers
    /// the server's 0x02 with 0x6B, as the bridge does.
    fn step(&mut self) {
        self.hub.poll().unwrap();
        self.ms.fetch_add(40, Ordering::SeqCst);
        self.hub.frame().unwrap();
        self.hub.deliver_all();
        for c in 0..self.ends.len() {
            while let Ok(Some(p)) = self.ends[c].recv() {
                match p {
                    Packet::Deliver(m) => {
                        if m.first() == Some(&0x02) && !self.joined[c] {
                            self.joined[c] = true;
                            self.send(c, Lane::System, vec![0x6B]);
                        }
                        self.got[c].push(m);
                    }
                    Packet::Frame { ticked } => self.got[c].push(vec![u8::from(ticked)]),
                    Packet::Send { .. } => panic!("a server end never sends C→S"),
                    Packet::Ext { .. } => panic!("D2RUST's own game registers no extension"),
                }
            }
        }
    }

    fn sim(&self) -> &Sim {
        &self.hub.host.game
    }

    /// The client's record state (`client_state`), if it has a record.
    fn state(&self, c: usize) -> Option<u32> {
        let s = self.sim();
        let id = s.sim_client(c as ClientId)?;
        s.game.lists.client(id).map(|e| e.state)
    }

    /// Steps until every client is in the game (at most `limit` frames).
    fn until_all_in_game(&mut self, limit: usize) {
        for _ in 0..limit {
            if (0..self.ends.len()).all(|c| self.state(c) == Some(client_state::IN_GAME)) {
                return;
            }
            self.step();
        }
        let states: Vec<_> = (0..self.ends.len()).map(|c| self.state(c)).collect();
        panic!("not every client got into the game: {states:?}");
    }
}

fn every_client_gets_its_own_player(n: usize) {
    let mut g = Game::new(DEFAULT_SEED, n);
    g.until_all_in_game(400);
    let players: Vec<_> = (0..n)
        .map(|c| g.sim().player_of(c as ClientId).expect("a player"))
        .collect();
    let mut unique = players.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), n, "one player unit per client: {players:?}");
    // Each player is the class its client asked for.
    for (c, &p) in players.iter().enumerate() {
        let class = g.sim().events.action.sys.units.get(p).map(|u| u.class);
        assert_eq!(class, Some((c % CLASSES.len()) as u32), "client {c}");
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn two_clients_play_one_game() {
    every_client_gets_its_own_player(2);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn eight_clients_play_one_game() {
    every_client_gets_its_own_player(8);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_client_that_joins_later_gets_the_games_settings() {
    let mut g = Game::new(DEFAULT_SEED, 1);
    g.until_all_in_game(400);
    // A second client connects mid-game and asks for Hell: the game stays
    // as it was created, and S→C 0x01 tells it so.
    let (end, server_end) = pair();
    g.hub.connect(1, server_end);
    g.ends.push(end);
    g.got.push(Vec::new());
    g.joined.push(false);
    let character = single_player::new_character("barbarian", "Later").unwrap();
    let mut req = single_player::create_request_for(&character);
    req.difficulty = 2;
    g.send(1, Lane::System, req.encode().to_vec());
    g.until_all_in_game(400);
    let first = |c: usize| {
        g.got[c]
            .iter()
            .find(|m| m.first() == Some(&0x01))
            .cloned()
            .expect("S→C 0x01")
    };
    assert_eq!(first(1), first(0), "both clients see the same game flags");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_client_leaves_and_the_others_play_on() {
    let mut g = Game::new(DEFAULT_SEED, 3);
    g.until_all_in_game(400);
    let leaver = g.sim().player_of(1).expect("a player");
    let guid = g.sim().game.lists.unit(leaver).map(|e| e.guid).unwrap();
    let before = g.got[0].len();
    g.send(1, Lane::System, vec![0x69]);
    for _ in 0..5 {
        g.step();
    }
    assert_eq!(g.state(1), None, "the leaver's record is gone");
    assert!(
        g.sim().game.lists.unit(leaver).is_none(),
        "its player is gone"
    );
    // The others are told (S→C 0x5C with the leaver's GUID) and stay in.
    let mut left = vec![0x5C];
    left.extend_from_slice(&guid.to_le_bytes());
    for c in [0, 2] {
        assert!(
            g.got[c][before..].iter().any(|m| m.starts_with(&left)),
            "client {c} hears that the player left"
        );
        assert_eq!(g.state(c), Some(client_state::IN_GAME));
    }
    for _ in 0..50 {
        g.step();
    }
    assert_eq!(g.state(0), Some(client_state::IN_GAME));
    assert_eq!(g.state(2), Some(client_state::IN_GAME));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_lost_connection_takes_the_player_out() {
    let mut g = Game::new(DEFAULT_SEED, 2);
    g.until_all_in_game(400);
    let lost = g.sim().player_of(1).expect("a player");
    let guid = g.sim().game.lists.unit(lost).map(|e| e.guid).unwrap();
    let before = g.got[0].len();
    // Client 1's end goes away (no leave message): the hub drops it.
    let (dead, _) = pair();
    drop(std::mem::replace(&mut g.ends[1], dead));
    for _ in 0..3 {
        g.step();
    }
    assert_eq!(g.hub.take_dropped().len(), 1);
    assert_eq!(g.state(1), None);
    assert!(g.sim().game.lists.unit(lost).is_none());
    let mut left = vec![0x5C];
    left.extend_from_slice(&guid.to_le_bytes());
    assert!(g.got[0][before..].iter().any(|m| m.starts_with(&left)));
    assert_eq!(g.state(0), Some(client_state::IN_GAME));
}

/// Runs the same script on the same seed: four clients join, walk and one
/// leaves. What each client receives is returned.
fn scripted_game() -> Vec<Vec<Vec<u8>>> {
    let mut g = Game::new(DEFAULT_SEED, 4);
    g.until_all_in_game(400);
    for i in 0..60u16 {
        // Every client asks to walk somewhere near its player (C→S 0x01,
        // a walk to a point), clients 0 and 2 every frame, 1 and 3 every
        // other frame.
        for c in 0..4usize {
            if c % 2 == 1 && i % 2 == 1 {
                continue;
            }
            let Some(pos) = g.sim().player_pos(c as ClientId) else {
                continue;
            };
            let dx = (i % 7) as i32 - 3 + c as i32;
            let dy = (i % 5) as i32 - 2;
            let x = (pos.x + dx) as u16;
            let y = (pos.y + dy) as u16;
            let mut walk = vec![0x01];
            walk.extend_from_slice(&x.to_le_bytes());
            walk.extend_from_slice(&y.to_le_bytes());
            g.send(c, Lane::Game, walk);
        }
        if i == 30 {
            g.send(3, Lane::System, vec![0x69]);
        }
        g.step();
    }
    g.got
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_same_script_gives_the_same_game_for_every_client() {
    let a = scripted_game();
    let b = scripted_game();
    assert_eq!(a.len(), b.len());
    for (c, (x, y)) in a.iter().zip(&b).enumerate() {
        assert_eq!(x.len(), y.len(), "client {c}: message count");
        assert!(x == y, "client {c}: the received bytes differ");
    }
}
