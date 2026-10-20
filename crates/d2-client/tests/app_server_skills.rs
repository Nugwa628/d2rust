//! The play host's server-side skills, headless, on the install's
//! single-player game behind the app's link (`single_player::start`):
//! the server player's skill list is d2-sim's (`ActionHooks::skill_lists`)
//! and the world's skill slot (`WiredSkills`) runs the skill handlers on
//! it (C→S 0x3C selects a hand's skill on the server's list). A new
//! character's join sends no S→C 0x94 (its stub load reads no skills
//! section).
//!
//! Real data: the game runs on the install's `skills` and
//! `charstats` rows; the expected native skills are what the join reads
//! from the class's `charstats` row of that same install.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, Link, DEFAULT_SEED, PLAYER_CLASS};
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_server::seams::Clock;
use d2_sim::skills::SkillEntry;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Game {
    link: ThreadLink<Link<StepClock>>,
    ms: Arc<AtomicU32>,
}

impl Game {
    /// The started game with the test-local skill rows, before the
    /// session sequence.
    fn started() -> Self {
        let ms = Arc::new(AtomicU32::new(1000));
        let (mut link, _) = single_player::start(
            app_support::game_data(),
            DEFAULT_SEED,
            StepClock(ms.clone()),
        )
        .unwrap();
        let _ = &mut link;
        Self { link, ms }
    }

    /// C→S 0x67 and 0x6B and three more ticks (the join runs rule 8 on
    /// the player's list, module docs); every S→C message received.
    fn join(&mut self) -> Vec<Vec<u8>> {
        let req = single_player::create_request();
        self.link.send(SendQueue::System, &req.encode()).unwrap();
        let mut got = self.ticks(1);
        self.link.send(SendQueue::System, &[0x6B]).unwrap();
        got.extend(self.ticks(3));
        got
    }

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

    /// What the join gives the player from the install's rows: skill 0,
    /// then each `charstats` `Skill 1`…`Skill 10` inside the skills table
    /// (the start skill's validity decides one more 0x23 at the join).
    fn native_rule(&mut self) -> (Vec<i32>, Option<i32>) {
        self.link
            .with(|l| {
                let h = l.host_mut().game.events.action.hooks();
                let rows = h.tables.skills.skills.len();
                let cs = &h.vitals.as_ref().expect("vitals").charstats[PLAYER_CLASS as usize];
                let mut ids = vec![0i32];
                for id in [
                    cs.skill_1,
                    cs.skill_2,
                    cs.skill_3,
                    cs.skill_4,
                    cs.skill_5,
                    cs.skill_6,
                    cs.skill_7,
                    cs.skill_8,
                    cs.skill_9,
                    cs.skill_10,
                ] {
                    let id = id as i16;
                    if id >= 0 && (id as usize) < rows && !ids.contains(&i32::from(id)) {
                        ids.push(i32::from(id));
                    }
                }
                let start = usize::from(cs.startskill) < rows;
                (ids, start.then_some(i32::from(cs.startskill)))
            })
            .unwrap()
    }

    /// The server player's skill list, left and right skill and GUID.
    fn player_skills(&mut self) -> (Vec<SkillEntry>, Option<SkillEntry>, Option<SkillEntry>, u32) {
        self.link
            .with(|l| {
                let s = &mut l.host_mut().game;
                let (p, guid) = single_player::local_player(s).expect("joined");
                let l = &s.events.action.hooks().skill_lists[&p];
                let v = l.view();
                let at = |i: Option<usize>| i.and_then(|i| v.get(i).copied());
                (v.clone(), at(l.left), at(l.right), guid)
            })
            .unwrap()
    }
}

fn native(skill: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_server_player_list_holds_its_native_skills_and_a_new_join_sends_no_0x94() {
    let mut g = Game::started();
    let got = g.join();
    let (ids, start) = g.native_rule();
    let start_skill = start.is_some();
    let (list, left, right, _) = g.player_skills();
    let mut want: Vec<_> = ids.iter().copied().map(native).collect();
    // The start items' staff carries the class's `StartSkill` (stat 107,
    // as the start items give it): an entry of level 0 from the item,
    // after the natives (as a known new sorceress join shows).
    if let Some(k) = start.filter(|k| !ids.contains(k)) {
        want.push(SkillEntry {
            skill: k,
            base: 0,
            owner_guid: -1,
            ..SkillEntry::default()
        });
    }
    assert_eq!(
        list, want,
        "skill 0, then the class skills inside the table"
    );
    assert_eq!((left, right), (Some(native(0)), Some(native(0))));
    assert!(
        got.iter().all(|m| m.first() != Some(&0x94)),
        "a new character's stub load sends no 0x94"
    );
    assert_eq!(
        got.iter().filter(|m| m.first() == Some(&0x23)).count(),
        2 + usize::from(start_skill),
        "the join's two 0x23 and the loader's one for a valid StartSkill (game-join.md §3 r2)"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn select_skill_runs_on_the_server_skill_list() {
    let mut g = Game::started();
    g.join();
    // 0x3C: the first native skill after 0 in the right hand (bit 31
    // clear), owner −1.
    let (ids, _) = g.native_rule();
    let pick = ids[1];
    let mut m = vec![0x3C];
    m.extend_from_slice(&(pick as u32).to_le_bytes());
    m.extend_from_slice(&(-1i32).to_le_bytes());
    g.link.send(SendQueue::Game, &m).unwrap();
    g.ticks(2);
    let (_, left, right, _) = g.player_skills();
    assert_eq!(right, Some(native(pick)), "the slot ran use.md §7");
    assert_eq!(left, Some(native(0)));
}
