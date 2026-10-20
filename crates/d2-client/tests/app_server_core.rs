//! The play host's server core, headless, on the synthetic single-player
//! game behind the app's link (`single_player::start`): the play host
//! stages no `UnitFacts`, so the point and unit-target parse reads them
//! from the sim's own unit and path records (`SimGame::set_facts_source`,
//! `world_sim_facts`). Without them every C→S walk was `Invalid` and the
//! walk the client drew was only its own prediction.
//!
//! Test-local fill: the synthetic game has no `charstats` (walk velocity
//! 0), so the joined game's action tables get one row per class with
//! `WalkVelocity` 6 / `RunVelocity` 9; the shared synthetic set is
//! unchanged.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, Link, DEFAULT_SEED};
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_client::bridge::LOCAL_CLIENT;
use d2_data::tables::{Charstats, Monstats, Monstats2, Record, Skills};
use d2_proto::client::Walk;
use d2_server::seams::{Clock, Pos};
use d2_sim::skills::list::ListOwner;

use crate::app_support;
use crate::real_rig;
use real_rig::Rig;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A `charstats` row with the walk and run velocities (the synthetic
/// game has no `charstats`, so its players stand still: velocity 0).
fn charstats_row() -> Charstats {
    let mut c = Charstats::decode(&[0u8; Charstats::SIZE]);
    c.walkvelocity = 6;
    c.runvelocity = 9;
    c
}

struct Game {
    link: ThreadLink<Link<StepClock>>,
    ms: Arc<AtomicU32>,
}

impl Game {
    /// The game after the session sequence (0x67, 0x6B) and two more
    /// ticks: the player is in its room.
    fn joined() -> Self {
        let ms = Arc::new(AtomicU32::new(1000));
        let (link, _) = single_player::start(
            app_support::game_data(),
            DEFAULT_SEED,
            StepClock(ms.clone()),
        )
        .unwrap();
        let mut g = Self { link, ms };
        // The test-local skill rows: eight zero `skills` records with
        // Attack's `anim` and `range`.
        g.link
            .with(|l| {
                let h = l.host_mut().game.events.action.hooks();
                let mut t = (*h.tables).clone();
                t.skills.skills = vec![Skills::decode(&[0u8; Skills::SIZE]); 8];
                // Attack: `anim` A1 (mode 7), `range` h2h (1).
                t.skills.skills[0].anim = 7;
                t.skills.skills[0].range = 1;
                t.skills.level_cap = d2_sim::skills::LEVEL_CAP_114D;
                h.tables = Arc::new(t);
            })
            .unwrap();
        let req = single_player::create_request();
        g.link.send(SendQueue::System, &req.encode()).unwrap();
        g.ticks(1);
        g.link.send(SendQueue::System, &[0x6B]).unwrap();
        g.ticks(3);
        g.link
            .with(|l| {
                let sim = &mut l.host_mut().game;
                let p = sim.player_of(LOCAL_CLIENT).expect("joined");
                let h = &mut sim.events.action.sys.hooks;
                let mut t = (*h.tables).clone();
                t.combat.charstats = (0..7).map(|_| charstats_row()).collect();
                t.combat.monstats = vec![Monstats::decode(&[0u8; Monstats::SIZE])];
                t.combat.monstats2 = vec![Monstats2::decode(&[0u8; Monstats2::SIZE])];
                h.tables = Arc::new(t);
                // The synthetic game has no vitals tables, so the join's
                // native skills did not run:
                // give the player its list here (skill 0 in both hands).
                let rows = h.tables.skills.skills.clone();
                h.skill_lists
                    .entry(p)
                    .or_default()
                    .init_player(&rows, ListOwner::player(1), Some(&[0xFFFF; 10]))
                    .unwrap();
            })
            .unwrap();
        g
    }

    /// Pumps `n` ticked frames; returns every S→C chunk received.
    fn ticks(&mut self, n: usize) -> Vec<Vec<u8>> {
        let mut got = Vec::new();
        for _ in 0..n {
            self.link.pump().unwrap();
            self.ms.fetch_add(40, Ordering::SeqCst);
            self.link.pump().unwrap();
            got.extend(self.link.receive());
        }
        got
    }

    /// The server-side position of the local player: its path record,
    /// the position the point parse reads.
    fn player_pos(&mut self) -> Option<Pos> {
        self.link
            .with(|l| {
                let s = &mut l.host_mut().game;
                let p = s.player_of(LOCAL_CLIENT)?;
                let (x, y) = s.events.action.hooks().path_position(p);
                Some(Pos { x, y })
            })
            .unwrap()
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_server_side_player_walks_on_a_client_walk() {
    let mut g = Game::joined();
    let start = g
        .player_pos()
        .expect("the player's facts come from the sim");
    assert_ne!(start, Pos { x: 0, y: 0 }, "placed by game entry");
    let to = Walk {
        x: (start.x + 4) as u16,
        y: start.y as u16,
    };
    g.link.send(SendQueue::Game, &to.encode()).unwrap();
    g.ticks(25);
    let end = g.player_pos().unwrap();
    assert!(
        end.x > start.x,
        "the server moved the player: {start:?} → {end:?}"
    );
}

/// A monster of class 0 allocated by the server next to the player, in
/// the player's room; its GUID.
fn monster_next_to_player(g: &mut Game) -> u32 {
    monster_at(g, 2)
}

/// The same, `dx` sub-tiles east of the player.
fn monster_at(g: &mut Game, dx: i32) -> u32 {
    use d2_sim::units::lifecycle::AllocRequest;
    use d2_sim::units::UnitType;
    g.link
        .with(move |l| {
            let s = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(s).expect("joined");
            let room = s.game.lists.unit(p).and_then(|e| e.room());
            let (x, y) = s.events.action.hooks().path_position(p);
            let pos = Pos { x, y };
            let monsters = &mut s.events.action.sys.data.monsters;
            if monsters.is_empty() {
                monsters.push(d2_sim::units::hooks::MonsterInfo {
                    enabled: true,
                    ..Default::default()
                });
            }
            let req = AllocRequest {
                ty: UnitType::Monster,
                class: 0,
                room,
                add: true,
                fixed_guid: None,
                mode: 1,
                allied: false,
            };
            let m = s
                .events
                .action
                .with(&mut s.game, |gm, v| v.allocate(gm, &req, pos.x + dx, pos.y))
                .unwrap_or_else(|| {
                    let h = s.events.action.hooks();
                    panic!("monster allocated: {:?} {:?}", h.x.log, h.errors)
                });
            s.game.lists.unit(m).unwrap().guid
        })
        .unwrap()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_left_skill_on_a_monster_next_to_the_player_starts_the_attack() {
    let mut g = Game::joined();
    let guid = monster_next_to_player(&mut g);
    let mut msg = vec![0x06, 1, 0, 0, 0];
    msg.extend(guid.to_le_bytes());
    g.link.send(SendQueue::Game, &msg).unwrap();
    g.link.pump().unwrap();
    let handled = g
        .link
        .with(|l| format!("{:?}", l.last_frame().messages))
        .unwrap();
    // Before: no facts for the monster → `Refused`; no skill slot → the
    // stub; no left skill → `Malformed` (code 3).
    assert!(handled.contains("Dispatched(Done)"), "{handled}");
    g.ticks(2);
    let (log, errors) = g
        .link
        .with(|l| {
            let h = l.host_mut().game.events.action.hooks();
            (h.x.skills.log.clone(), format!("{:?}", h.errors))
        })
        .unwrap();
    // In melee reach: the skill's mode starts at once, no
    // run to the target. The install has the animdata rows, so the attack
    // animation itself runs (the synthetic game failed it with
    // `Anim(NoRecord)`).
    assert!(log.iter().all(|l| !l.starts_with("run to")), "{log:?}");
    assert!(!errors.contains("NoRecord"), "{errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn an_out_of_range_walk_resyncs_the_client_with_0x15() {
    let mut g = Game::joined();
    // More than 25 frames since the last accepted point (`+0x168`).
    g.ticks(30);
    let start = g.player_pos().unwrap();
    let to = Walk {
        x: (start.x + 200) as u16,
        y: start.y as u16,
    };
    g.link.send(SendQueue::Game, &to.encode()).unwrap();
    let got = g.ticks(3);
    let r = got
        .iter()
        .find(|c| c[0] == 0x15)
        .expect("S→C 0x15 ReassignPlayer");
    assert_eq!(r.len(), 11);
    assert_eq!(u16::from_le_bytes([r[6], r[7]]) as i32, start.x);
    assert_eq!(u16::from_le_bytes([r[8], r[9]]) as i32, start.y);
    assert_eq!(r[10], 1);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_left_skill_on_a_far_monster_runs_the_server_player_to_it() {
    // A real Blood Moor zombie 12 sub-tiles east of a sorceress who left
    // the town (the left skill is skill 0, Attack).
    let mut r = Rig::new("sorceress", &[]);
    r.leave_town();
    r.strengthen();
    let start = r.pos();
    let m = r.spawn_monster(12);
    let guid = r.with(move |sim, _| sim.game.lists.unit(m).map(|e| e.guid).expect("zombie"));
    let mut msg = vec![0x06, 1, 0, 0, 0];
    msg.extend(guid.to_le_bytes());
    r.send_for_fate(&msg);
    r.step(20);
    let end = r.pos();
    let log = r.with(|sim, _| sim.events.action.hooks().x.skills.log.clone());
    assert!(log.iter().all(|l| !l.starts_with("run to")), "{log:?}");
    assert!(
        end.0 > start.0,
        "the player ran toward it: {start:?} → {end:?}"
    );
}

/// An item unit (class 0) in the player's room, `dx` sub-tiles east of
/// the player; owned by the player in the inventory model when `owned`.
fn item_at(g: &mut Game, dx: i32, owned: bool) -> u32 {
    use d2_sim::units::lifecycle::AllocRequest;
    use d2_sim::units::UnitType;
    g.link
        .with(move |l| {
            let s = &mut l.host_mut().game;
            let (p, pguid) = single_player::local_player(s).expect("joined");
            let room = s.game.lists.unit(p).and_then(|e| e.room());
            let (x, y) = s.events.action.hooks().path_position(p);
            let req = AllocRequest {
                ty: UnitType::Item,
                class: 0,
                room,
                add: true,
                fixed_guid: None,
                mode: 3,
                allied: false,
            };
            let it = s
                .events
                .action
                .with(&mut s.game, |gm, v| v.allocate(gm, &req, x + dx, y))
                .expect("item allocated");
            let guid = s.game.lists.unit(it).unwrap().guid;
            if owned {
                // The synthetic game has no inventory model: an empty one.
                let inv = s.world.inventory.get_or_insert_with(|| {
                    d2_server::adapters::handlers::world::preview_inv_parts(Default::default())
                });
                let mut rec = d2_sim::items::inventory::InvItem::new(guid, 0);
                rec.owner_guid = pguid;
                inv.state.items.insert(it, rec);
            }
            guid
        })
        .unwrap()
}

/// The result of one C→S 0x04 (run to unit) on an item, as the dispatcher
/// reports it.
fn run_to_item(g: &mut Game, guid: u32) -> String {
    let mut msg = vec![0x04, 4, 0, 0, 0];
    msg.extend(guid.to_le_bytes());
    g.link.send(SendQueue::Game, &msg).unwrap();
    g.link.pump().unwrap();
    g.link
        .with(|l| format!("{:?}", l.last_frame().messages))
        .unwrap()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn an_item_target_is_staged_with_its_owner() {
    let mut g = Game::joined();
    // Far outside the 50-subtile reach: only an owned item passes.
    let far = item_at(&mut g, 200, false);
    let refused = run_to_item(&mut g, far);
    assert!(!refused.contains("Dispatched(Done)"), "{refused}");
    let mine = item_at(&mut g, 200, true);
    let accepted = run_to_item(&mut g, mine);
    assert!(accepted.contains("Dispatched(Done)"), "{accepted}");
}
