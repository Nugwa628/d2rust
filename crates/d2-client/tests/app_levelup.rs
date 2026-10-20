//! XP and level-up on the play server (install's data): experience gained
//! on the server reaches the client as stat messages (level, stat points,
//! skill points), C→S 0x3A spends a stat point and C→S 0x3B a skill point,
//! and the client model follows every message.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, Link, DEFAULT_SEED, PLAYER_CLASS};
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_data::tables::{Charstats, Skills};
use d2_sim::combat::vitals::add_experience;
use d2_sim::units::UnitId;
use d2_sim::wiring::action::View;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

type Server = ThreadLink<Link<StepClock>>;

struct Game {
    link: Server,
    ms: Arc<AtomicU32>,
}

impl Game {
    /// Pumps one tick and returns the S→C messages of it.
    fn tick(&mut self) -> Vec<Vec<u8>> {
        self.ms.fetch_add(40, Ordering::SeqCst);
        self.link.pump().unwrap();
        self.link.receive()
    }

    /// Adds `gain` experience to the player on the server (the grant a
    /// kill ends in).
    fn gain(&mut self, p: UnitId, gain: u32) {
        self.link
            .with(move |l| {
                let a = &mut l.host_mut().game.events.action;
                let t = a.hooks().vitals.clone().unwrap();
                let s = &mut a.sys;
                let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
                add_experience(&mut v, &t, p, gain);
            })
            .unwrap();
    }

    fn stat(&mut self, p: UnitId, stat: u16) -> i32 {
        self.link
            .with(move |l| {
                l.host_mut()
                    .game
                    .events
                    .action
                    .sys
                    .stats
                    .unit_base(p, stat, 0)
            })
            .unwrap()
    }

    /// The player's class row of the install's `charstats`.
    fn charstats(&mut self) -> Charstats {
        self.link
            .with(|l| {
                let h = l.host_mut().game.events.action.hooks();
                h.vitals.as_ref().expect("vitals").charstats[PLAYER_CLASS as usize].clone()
            })
            .unwrap()
    }

    /// The first skill of the player's class that a level-2 character may
    /// learn (`reqlevel` ≤ 2, no required skill), from the install's
    /// `skills` rows.
    fn learnable_skill(&mut self) -> u16 {
        self.link
            .with(|l| {
                let h = l.host_mut().game.events.action.hooks();
                let none = |r: u16| r == 0xFFFF || usize::from(r) >= h.tables.skills.skills.len();
                h.tables
                    .skills
                    .skills
                    .iter()
                    .position(|s: &Skills| {
                        s.charclass == PLAYER_CLASS as u8
                            && s.reqlevel <= 2
                            && none(s.reqskill1)
                            && none(s.reqskill2)
                            && none(s.reqskill3)
                    })
                    .expect("a class skill of level 2") as u16
            })
            .unwrap()
    }
}

/// The install's game, joined: the real vitals, `skills` and
/// `itemstatcost` rows, the class's start stats and the town populated.
fn joined() -> (Game, UnitId, u32) {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    link.send(SendQueue::System, &single_player::create_request().encode())
        .unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    let (p, guid) = link
        .with(|l| single_player::local_player(&l.host().game).expect("joined"))
        .unwrap();
    let mut g = Game { link, ms };
    // The next ticks populate the town and put the client in game.
    for _ in 0..3 {
        g.tick();
    }
    (g, p, guid)
}

fn has(msgs: &[Vec<u8>], m: &[u8]) -> bool {
    msgs.iter().any(|x| x == m)
}

/// The 0x1D stat message of a one-byte stat.
fn stat_msg(id: u8, v: u8) -> [u8; 3] {
    [0x1D, id, v]
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn experience_to_level_up_reaches_the_client_as_stat_messages() {
    let (mut g, p, _) = joined();
    g.tick();
    let cs = g.charstats();
    let (points0, skills0) = (g.stat(p, 4), g.stat(p, 5));
    g.gain(p, 500);
    let got = g.tick();
    // Level 2 (`experience` row 2 is 500), `StatPerLevel` stat points, one
    // skill point, experience 500.
    assert_eq!(g.stat(p, 12), 2);
    assert_eq!(g.stat(p, 13), 500);
    assert_eq!(g.stat(p, 4), points0 + i32::from(cs.statperlevel));
    assert_eq!(g.stat(p, 5), skills0 + 1);
    assert!(has(&got, &stat_msg(12, 2)), "{got:02X?}");
    assert!(has(&got, &stat_msg(4, g.stat(p, 4) as u8)), "{got:02X?}");
    assert!(has(&got, &stat_msg(5, g.stat(p, 5) as u8)), "{got:02X?}");
    // Nothing repeats on the next tick.
    let again = g.tick();
    assert!(!has(&again, &stat_msg(12, 2)), "{again:02X?}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_stat_point_spends_on_the_server_and_is_sent_back() {
    let (mut g, p, _) = joined();
    g.gain(p, 500);
    g.tick();
    let (str0, points) = (g.stat(p, 0), g.stat(p, 4));
    assert!(points >= 2);
    // Strength +1 (stat 0, count − 1 = 0).
    g.link.send(SendQueue::Game, &[0x3A, 0, 0]).unwrap();
    let got = g.tick();
    assert_eq!(g.stat(p, 0), str0 + 1);
    assert_eq!(g.stat(p, 4), points - 1);
    assert!(has(&got, &stat_msg(0, (str0 + 1) as u8)), "{got:02X?}");
    assert!(has(&got, &stat_msg(4, (points - 1) as u8)), "{got:02X?}");
    // More asked than left: the loop spends what it can, then fails
    // (result 2).
    g.link.send(SendQueue::Game, &[0x3A, 0, 99]).unwrap();
    g.tick();
    assert_eq!(g.stat(p, 4), 0);
    assert_eq!(g.stat(p, 0), str0 + points);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_skill_point_adds_the_skill_and_tells_the_client() {
    let (mut g, p, guid) = joined();
    g.gain(p, 500);
    g.tick();
    let skill = g.learnable_skill();
    let points = g.stat(p, 5);
    g.link
        .send(SendQueue::Game, &[0x3B, skill as u8, (skill >> 8) as u8])
        .unwrap();
    let got = g.tick();
    assert_eq!(g.stat(p, 5), points - 1, "the point is spent");
    let mut m = vec![0x21, 0, 0];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&[skill as u8, (skill >> 8) as u8, 1, 0, 0]);
    assert!(has(&got, &m), "{got:02X?}");
    assert!(has(&got, &stat_msg(5, (points - 1) as u8)), "{got:02X?}");
    // The spent point is gone: asking again for a skill needing a point
    // beyond the ones left changes nothing once they are all spent.
    for _ in 0..points {
        g.link
            .send(SendQueue::Game, &[0x3B, skill as u8, (skill >> 8) as u8])
            .unwrap();
        g.tick();
    }
    assert_eq!(g.stat(p, 5), 0);
}
