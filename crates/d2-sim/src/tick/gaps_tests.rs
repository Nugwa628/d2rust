//! Gap tests: tick rules not yet claimed by other tests.

#[allow(unused_imports)]
use super::*;

use super::tests::{id, sched, setup, tick_to};
use super::timer::{flags, TimerId, BUCKETS};
use crate::units::UnitType;

/// Logs the step hooks's table names, with the frame each sees.
#[derive(Default)]
struct Steps {
    log: Vec<String>,
    /// Act whose day/night cycle index changes every tick.
    env_act: Option<u8>,
}

impl EventDispatch for Steps {
    fn run_event(&mut self, _: &mut Game, _: &TimerRun) {}
}

impl TickHooks for Steps {
    fn advance_environment(&mut self, g: &mut Game, act: u8) -> bool {
        self.log.push(format!("env {act} f{}", g.frame));
        self.env_act == Some(act)
    }
    fn environment_changed(&mut self, _: &mut Game, act: u8, c: ClientId) {
        self.log.push(format!("envchg {act} c{}", c.0));
    }
    fn unit_update(&mut self, _: &mut Game, u: UnitId) {
        self.log.push(format!("update u{}", u.0));
    }
    fn clear_arena_flag(&mut self, _: &mut Game) {
        self.log.push("arena".into());
    }
    fn free_removal_records(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("removal r{}", room.0));
    }
    fn update_quests(&mut self, _: &mut Game) {
        self.log.push("quests".into());
    }
    fn room_inactivity(&mut self, _: &mut Game, _: RoomId) -> u32 {
        self.log.push("deactivation".into());
        0
    }
    fn free_inactive_rooms(&mut self, _: &mut Game, act: u8) {
        self.log.push(format!("free rooms {act}"));
    }
    fn delete_inactive_items(&mut self, _: &mut Game, act: u8) {
        self.log.push(format!("items {act}"));
    }
}

#[test]
fn acts_in_index_order_null_skipped() {
    let mut g = Game::new();
    for act in [3, 1, 4] {
        g.lists.ensure_act(act).unwrap();
    }
    let mut s = Steps::default();
    tick(&mut g, &mut s);
    s.log.retain(|l| l.starts_with("env"));
    assert_eq!(s.log, ["env 1 f1", "env 3 f1", "env 4 f1"]);
}

#[test]
fn step_table_conditions() {
    let mut g = Game::new();
    g.lists.ensure_act(0).unwrap();
    g.lists.ensure_act(2).unwrap();
    let ra = g.lists.create_room(0).unwrap();
    let rb = g.lists.create_room(0).unwrap();
    let rc = g.lists.create_room(2).unwrap();
    for r in [ra, rb, rc] {
        g.lists.activate_room(r).unwrap();
    }
    let ua = g.spawn_unit(UnitType::Monster, Some(ra), false).unwrap();
    let ub = g.spawn_unit(UnitType::Monster, Some(rb), false).unwrap();
    let uc = g.spawn_unit(UnitType::Monster, Some(rc), false).unwrap();
    // Act 2 has a queued unit but no pending-update flag: step 6 skips it.
    g.lists.act_mut(2).unwrap().pending_updates = false;
    // Only act 0 has pending removal records (step 7).
    g.lists.act_mut(0).unwrap().pending_removals = true;
    // Clients in a state the client pass ignores.
    let c1 = g.lists.add_client(None, None, 0);
    let c2 = g.lists.add_client(None, None, 0);
    let mut s = Steps {
        env_act: Some(2),
        ..Steps::default()
    };
    tick(&mut g, &mut s);
    let (a, b) = (ra.0, rb.0);
    assert_eq!(
        s.log,
        [
            // Step 0 ran first: the hooks see frame 1.
            "env 0 f1".to_string(),
            "env 2 f1".into(),
            // Step 1: act 2's cycle changed: every client, list order.
            format!("envchg 2 c{}", c2.0),
            format!("envchg 2 c{}", c1.0),
            // Step 6: act 0's rooms in list order, then the arena flag.
            format!("update u{}", ub.0),
            format!("update u{}", ua.0),
            "arena".into(),
            // Step 7: act 0's rooms in list order.
            format!("removal r{b}"),
            format!("removal r{a}"),
        ]
    );
    assert!(g.lists.update_queue(ra).is_empty() && g.lists.update_queue(rb).is_empty());
    assert_eq!(g.lists.update_queue(rc), [uc]);
    let act0 = g.lists.act(0).unwrap();
    assert!(!act0.pending_updates && !act0.pending_removals);
    // Periodic steps 8–11: frame % 20, % 12, % 11, % 1500.
    s.env_act = None;
    let mut seen: [Vec<i32>; 4] = Default::default();
    while g.frame < 3000 {
        s.log.clear();
        tick(&mut g, &mut s);
        for (i, tag) in ["quests", "deactivation", "free rooms 0", "items 0"]
            .iter()
            .enumerate()
        {
            if s.log.iter().any(|l| l == tag) {
                seen[i].push(g.frame);
            }
        }
    }
    for (frames, period) in seen.iter().zip([20, 12, 11, 1500]) {
        let want: Vec<i32> = (2..=3000).filter(|f| f % period == 0).collect();
        assert_eq!(*frames, want, "period {period}");
    }
}

#[test]
fn queue_structure() {
    // Class table: player 0, monster 1, missile 2, object 3, item 4; tile
    // none.
    let classes: Vec<Option<u8>> = UnitType::ALL
        .iter()
        .map(|&t| TimerClass::of(t).map(|c| c as u8))
        .collect();
    // UnitType::ALL is player, monster, object, missile, item, tile.
    assert_eq!(classes, [Some(0), Some(1), Some(3), Some(2), Some(4), None]);
    assert_eq!(BUCKETS, 64);
    assert_eq!(
        (
            flags::EXECUTING,
            flags::FREE,
            flags::EVERY_TICK,
            flags::DELETE
        ),
        (1, 2, 4, 8)
    );
    let (mut g, mut rec, _) = setup(&[
        ("P", UnitType::Player),
        ("M", UnitType::Monster),
        ("I", UnitType::Item),
    ]);
    // Timed: bucket expire % 64 of the unit's class.
    let tp = sched(&mut g, &rec, "P", 70).unwrap();
    let ti = sched(&mut g, &rec, "I", 6).unwrap();
    assert_eq!(g.timers.bucket(TimerClass::Player, 6), [tp]);
    assert_eq!(g.timers.bucket(TimerClass::Item, 6), [ti]);
    assert!(g.timers.bucket(TimerClass::Monster, 6).is_empty());
    assert_eq!(
        (g.timers.expire(tp), g.timers.flags(tp)),
        (Some(70), Some(0))
    );
    // Every tick: the class's own list, expire −1, flag 4.
    let tm = sched(&mut g, &rec, "M", -1).unwrap();
    assert_eq!(g.timers.every_tick(TimerClass::Monster), [tm]);
    assert!(g.timers.every_tick(TimerClass::Player).is_empty());
    assert_eq!(
        (g.timers.expire(tm), g.timers.flags(tm)),
        (Some(-1), Some(flags::EVERY_TICK))
    );
    // Event type and arguments are stored on the record.
    let m = id(&rec, "M");
    let t = g.schedule_event(m, 9, 40, None, 5, 6).unwrap().unwrap();
    assert_eq!(g.timers.event(t), Some((9, 5, 6)));
    // Flag 1 while its callback runs; the bucket index is frame % 64.
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s2 = seen.clone();
    rec.on_run = Some(Box::new(move |g: &mut Game, run: &TimerRun| {
        s2.borrow_mut().push((
            g.timers.flags(run.timer).unwrap(),
            g.timers.current_bucket(),
        ));
    }));
    tick_to(&mut g, &mut rec, 6);
    assert_eq!(
        seen.borrow().last(),
        Some(&(flags::EXECUTING, 6)),
        "item timer at frame 6"
    );
    assert!(seen.borrow().iter().all(|(f, _)| f & flags::EXECUTING != 0));
    // The player timer for 70 is still in bucket 6, not run.
    assert_eq!(g.timers.bucket(TimerClass::Player, 6), [tp]);
}

#[test]
fn bucket_keeps_insertion_order_across_expiry_frames() {
    let (mut g, mut rec, _) = setup(&[
        ("A", UnitType::Monster),
        ("B", UnitType::Monster),
        ("C", UnitType::Monster),
        ("D", UnitType::Monster),
    ]);
    g.frame = 10;
    let b = sched(&mut g, &rec, "B", 76).unwrap();
    let a = sched(&mut g, &rec, "A", 12).unwrap();
    let d = sched(&mut g, &rec, "D", 140).unwrap();
    let c = sched(&mut g, &rec, "C", 12).unwrap();
    assert_eq!(g.timers.bucket(TimerClass::Monster, 12), [b, a, d, c]);
    assert_eq!(tick_to(&mut g, &mut rec, 12), ["A", "C"]);
    assert_eq!(g.timers.bucket(TimerClass::Monster, 12), [b, d]);
}

#[test]
fn cancelled_timer_goes_to_the_free_list() {
    let (mut g, rec, _) = setup(&[("A", UnitType::Object)]);
    let a = id(&rec, "A");
    let t1 = sched(&mut g, &rec, "A", 20).unwrap();
    let t2 = sched(&mut g, &rec, "A", 20).unwrap();
    g.timers.cancel(t1);
    // Free: no longer a live timer (flags = 2), in no list.
    assert_eq!((g.timers.flags(t1), g.timers.expire(t1)), (None, None));
    assert_eq!(g.timers.unit_timers(a), [t2]);
    assert_eq!(g.timers.bucket(TimerClass::Object, 20), [t2]);
    // Cancelling a free timer does nothing.
    g.timers.cancel(t1);
    assert_eq!(g.timers.unit_timers(a), [t2]);
    // The next schedule takes it back from the free list.
    let t3: TimerId = sched(&mut g, &rec, "A", 30).unwrap();
    assert_eq!(t3.slot(), t1.slot());
    assert_eq!(g.timers.unit_timers(a), [t3, t2]);
}

/// A stale [`TimerId`] (its timer freed, its record reused by a later
/// schedule) names no timer: cancelling it is cancelling a free timer
/// (nothing), and the new timer stays.
#[test]
fn stale_timer_id_does_not_reach_the_reused_record() {
    let (mut g, rec, _) = setup(&[("A", UnitType::Object)]);
    let a = id(&rec, "A");
    let t1 = sched(&mut g, &rec, "A", 20).unwrap();
    g.timers.cancel(t1);
    let t3 = sched(&mut g, &rec, "A", 30).unwrap();
    assert_eq!(t3.slot(), t1.slot());
    assert_ne!(t3, t1);
    assert_eq!(
        (g.timers.flags(t1), g.timers.expire(t1), g.timers.event(t1)),
        (None, None, None)
    );
    g.timers.cancel(t1);
    assert_eq!(g.timers.expire(t3), Some(30));
    assert_eq!(g.timers.unit_timers(a), [t3]);
    assert_eq!(g.timers.bucket(TimerClass::Object, 30), [t3]);
}
