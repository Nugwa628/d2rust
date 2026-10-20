use super::timer::{flags, frame_mod, CallbackId, TimerError, TimerOwner};
use super::*;
use crate::units::lists::client_state;
use crate::units::{UnitId, UnitType};

pub(super) type OnRun = Box<dyn FnMut(&mut Game, &TimerRun)>;

/// Logs every hook call and timer run; optional per-run action.
#[derive(Default)]
pub(super) struct Rec {
    pub(super) log: Vec<String>,
    pub(super) names: Vec<(UnitId, &'static str)>,
    pub(super) on_run: Option<OnRun>,
    pub(super) room_ready: bool,
    pub(super) inactivity: u32,
    pub(super) allow_removal: bool,
}

impl Rec {
    pub(super) fn name(&self, u: UnitId) -> &'static str {
        self.names
            .iter()
            .find(|(id, _)| *id == u)
            .map(|(_, n)| *n)
            .unwrap_or("?")
    }

    pub(super) fn runs(&self) -> Vec<String> {
        self.log
            .iter()
            .filter(|s| s.starts_with("run "))
            .map(|s| s[4..].to_string())
            .collect()
    }

    pub(super) fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }
}

impl EventDispatch for Rec {
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        let n = self.name(run.owner.unit);
        self.log.push(format!("run {n}"));
        if let Some(f) = self.on_run.as_mut() {
            f(game, run);
        }
    }
}

impl TickHooks for Rec {
    fn advance_environment(&mut self, _: &mut Game, act: u8) -> bool {
        self.log.push(format!("env {act}"));
        false
    }
    fn ambient_spawns(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("ambient {}", room.0));
    }
    fn spawn_presets(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("presets {}", room.0));
    }
    fn restore_inactive_units(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("restore {}", room.0));
    }
    fn populate_objects(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("objects {}", room.0));
    }
    fn populate_monsters(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("monsters {}", room.0));
    }
    fn client_room_ready(&mut self, _: &mut Game, _: ClientId) -> bool {
        self.room_ready
    }
    fn send_load_complete(&mut self, g: &mut Game, c: ClientId) {
        let s = g.lists.client(c).unwrap().state;
        self.log.push(format!("msg4 state{s}"));
    }
    fn refresh_inventory(&mut self, g: &mut Game, c: ClientId) {
        let s = g.lists.client(c).unwrap().state;
        self.log.push(format!("inventory state{s}"));
    }
    fn join_sequence(&mut self, _: &mut Game, _: ClientId) {
        self.log.push("join".into());
    }
    fn send_removed_units(&mut self, _: &mut Game, _: ClientId) {
        self.log.push("removals".into());
    }
    fn send_unit_update(&mut self, _: &mut Game, _: ClientId, u: UnitId) {
        let n = self.name(u);
        self.log.push(format!("send {n}"));
    }
    fn client_update_messages(&mut self, _: &mut Game, _: ClientId) {
        self.log.push("client msgs".into());
    }
    fn client_level_change(&mut self, g: &mut Game, c: ClientId) {
        self.log.push("level change".into());
        let p = g.lists.client(c).unwrap().player.unwrap();
        let room = g.lists.unit(p).unwrap().room();
        g.lists.client_mut(c).unwrap().room = room;
    }
    fn arena_sync(&mut self, _: &mut Game, _: ClientId) {
        self.log.push("arena sync".into());
    }
    fn unit_update(&mut self, _: &mut Game, u: UnitId) {
        let n = self.name(u);
        self.log.push(format!("update {n}"));
    }
    fn clear_arena_flag(&mut self, _: &mut Game) {
        self.log.push("step6 end".into());
    }
    fn free_removal_records(&mut self, _: &mut Game, room: RoomId) {
        self.log.push(format!("removal records {}", room.0));
    }
    fn update_quests(&mut self, _: &mut Game) {
        self.log.push("step8".into());
    }
    fn room_inactivity(&mut self, _: &mut Game, room: RoomId) -> u32 {
        if !self.log.iter().any(|s| s == "step9") {
            self.log.push("step9".into());
        }
        self.log.push(format!("inactivity {}", room.0));
        self.inactivity
    }
    fn act_allows_room_removal(&mut self, _: &mut Game, _: u8, _: RoomId) -> bool {
        self.allow_removal
    }
    fn compress_unit(&mut self, g: &mut Game, u: UnitId) {
        let n = self.name(u);
        self.log.push(format!("compress {n}"));
        g.remove_unit(u).unwrap();
    }
    fn free_inactive_rooms(&mut self, _: &mut Game, act: u8) {
        self.log.push(format!("step10 {act}"));
    }
    fn delete_inactive_items(&mut self, _: &mut Game, act: u8) {
        self.log.push(format!("step11 items {act}"));
    }
    fn expire_inactive_unit_items(&mut self, _: &mut Game, act: u8) {
        self.log.push(format!("step11 nodes {act}"));
    }
}

/// A game with act 0, one active room, and named units in it.
pub(super) fn setup(units: &[(&'static str, UnitType)]) -> (Game, Rec, RoomId) {
    let mut g = Game::new();
    g.lists.ensure_act(0).unwrap();
    let room = g.lists.create_room(0).unwrap();
    g.lists.activate_room(room).unwrap();
    let mut rec = Rec::default();
    for &(n, ty) in units {
        let u = g.spawn_unit(ty, Some(room), false).unwrap();
        rec.names.push((u, n));
    }
    (g, rec, room)
}

pub(super) fn id(rec: &Rec, n: &str) -> UnitId {
    rec.names.iter().find(|(_, m)| *m == n).unwrap().0
}

pub(super) fn sched(g: &mut Game, rec: &Rec, n: &str, expire: i32) -> Option<timer::TimerId> {
    g.schedule_event(id(rec, n), 0, expire, None, 0, 0).unwrap()
}

/// Runs ticks until the frame counter equals `frame`, returning the runs
/// of the last tick.
pub(super) fn tick_to(g: &mut Game, rec: &mut Rec, frame: i32) -> Vec<String> {
    while g.frame < frame {
        rec.log.clear();
        tick(g, rec);
    }
    rec.runs()
}

#[test]
fn tick_length() {
    // Vector: rate 25 → 40 ms.
    assert_eq!(TICK_LENGTH_MS, 40);
}

#[test]
fn first_tick_is_frame_one() {
    // Vector: frame 0 → tick → frame 1; bucket 1.
    let (mut g, mut rec, _) = setup(&[]);
    assert_eq!(g.frame, 0);
    tick(&mut g, &mut rec);
    assert_eq!(g.frame, 1);
    assert_eq!(g.timers.current_bucket(), 1);
}

#[test]
fn signed_remainders() {
    // Vector: frame −1 → bucket −1 (signed); edge case 1.
    assert_eq!(frame_mod(-1, 64), -1);
    assert_eq!(frame_mod(-65, 64), -1);
    assert!(is_due(-20, 20));
    assert!(!is_due(-1, 20));
    assert_eq!(frame_mod(i32::MIN, 64), 0);
}

#[test]
fn frame_counter_wraps() {
    let (mut g, mut rec, _) = setup(&[]);
    g.frame = i32::MAX - 63;
    tick(&mut g, &mut rec);
    assert_eq!(g.frame, i32::MAX - 62);
    g.frame = i32::MAX;
    // The counter wraps to i32::MIN like the base game's; i32::MIN % 64
    // is 0, so this tick's bucket is still valid.
    tick(&mut g, &mut rec);
    assert_eq!(g.frame, i32::MIN);
}

fn periodic_steps(frame: i32) -> Vec<String> {
    let (mut g, mut rec, _) = setup(&[]);
    g.frame = frame - 1;
    tick(&mut g, &mut rec);
    rec.log
        .into_iter()
        .filter(|s| s.starts_with("step") && !s.starts_with("step6"))
        .collect()
}

#[test]
fn periodic_steps_at_660() {
    // Vector: 660 → steps 8, 9, 10 in order; 11 not.
    assert_eq!(periodic_steps(660), ["step8", "step9", "step10 0"]);
}

#[test]
fn periodic_steps_at_1500() {
    // Vector: 1500 → 8, 9, 11; 10 not (1500 % 11 = 4).
    assert_eq!(
        periodic_steps(1500),
        ["step8", "step9", "step11 items 0", "step11 nodes 0"]
    );
    assert!(periodic_steps(1501).is_empty());
}

#[test]
fn bucket_aliasing() {
    // Vector: at frame 10 schedule A(12), B(76), C(12) → bucket 12 =
    // [A, B, C]; frame 12 runs A, C; frame 76 runs B. Edge case 2.
    let (mut g, mut rec, _) = setup(&[
        ("A", UnitType::Monster),
        ("B", UnitType::Monster),
        ("C", UnitType::Monster),
    ]);
    g.frame = 10;
    let a = sched(&mut g, &rec, "A", 12).unwrap();
    let b = sched(&mut g, &rec, "B", 76).unwrap();
    let c = sched(&mut g, &rec, "C", 12).unwrap();
    assert_eq!(g.timers.bucket(TimerClass::Monster, 12), [a, b, c]);
    assert_eq!(tick_to(&mut g, &mut rec, 11), Vec::<String>::new());
    assert_eq!(tick_to(&mut g, &mut rec, 12), ["A", "C"]);
    assert_eq!(g.timers.bucket(TimerClass::Monster, 12), [b]);
    assert!(tick_to(&mut g, &mut rec, 75).is_empty());
    assert_eq!(tick_to(&mut g, &mut rec, 76), ["B"]);
}

#[test]
fn past_expire_moves_to_next_frame() {
    // Vector: at frame 10, expire 5 → 11. Also expire == frame.
    let (mut g, rec, _) = setup(&[("A", UnitType::Monster)]);
    g.frame = 10;
    let t = sched(&mut g, &rec, "A", 5).unwrap();
    assert_eq!(g.timers.expire(t), Some(11));
    let t = sched(&mut g, &rec, "A", 10).unwrap();
    assert_eq!(g.timers.expire(t), Some(11));
}

#[test]
fn every_tick_runs_newest_first() {
    // Vector: every-tick X then Y → Y, X each tick.
    let (mut g, mut rec, _) = setup(&[("X", UnitType::Player), ("Y", UnitType::Player)]);
    sched(&mut g, &rec, "X", -1).unwrap();
    sched(&mut g, &rec, "Y", -1).unwrap();
    assert_eq!(tick_to(&mut g, &mut rec, 1), ["Y", "X"]);
    assert_eq!(tick_to(&mut g, &mut rec, 2), ["Y", "X"]);
}

#[test]
fn class_run_order() {
    // Vector: frame 12, due P1, M1, S1; every-tick S2 → S2, S1, P1, M1.
    let (mut g, mut rec, _) = setup(&[
        ("M1", UnitType::Monster),
        ("P1", UnitType::Player),
        ("S1", UnitType::Missile),
        ("S2", UnitType::Missile),
        ("O1", UnitType::Object),
        ("I1", UnitType::Item),
    ]);
    g.frame = 10;
    for n in ["I1", "O1", "M1", "P1", "S1"] {
        sched(&mut g, &rec, n, 12);
    }
    tick_to(&mut g, &mut rec, 11);
    sched(&mut g, &rec, "S2", -1);
    assert_eq!(
        tick_to(&mut g, &mut rec, 12),
        ["S2", "S1", "P1", "M1", "O1", "I1"]
    );
}

#[test]
fn scheduled_for_now_during_run_moves_to_next_frame() {
    // Vector: during M1's event at 12, schedule M2 expire 12 → expire 13,
    // runs at 13. Edge case 5.
    let (mut g, mut rec, _) = setup(&[("M1", UnitType::Monster), ("M2", UnitType::Monster)]);
    let m2 = id(&rec, "M2");
    g.frame = 10;
    sched(&mut g, &rec, "M1", 12);
    rec.on_run = Some(Box::new(move |g, run| {
        if run.expire == 12 {
            let t = g.schedule_event(m2, 0, 12, None, 0, 0).unwrap().unwrap();
            assert_eq!(g.timers.expire(t), Some(13));
        }
    }));
    assert_eq!(tick_to(&mut g, &mut rec, 12), ["M1"]);
    assert_eq!(tick_to(&mut g, &mut rec, 13), ["M2"]);
}

#[test]
fn cancel_later_timer_during_run() {
    // Vector: during M1's event, cancel M3 (due 12, later in bucket).
    let (mut g, mut rec, _) = setup(&[
        ("M1", UnitType::Monster),
        ("M2", UnitType::Monster),
        ("M3", UnitType::Monster),
    ]);
    g.frame = 10;
    sched(&mut g, &rec, "M1", 12);
    sched(&mut g, &rec, "M2", 12);
    let t3 = sched(&mut g, &rec, "M3", 12).unwrap();
    let m1 = id(&rec, "M1");
    rec.on_run = Some(Box::new(move |g, run| {
        if run.owner.unit == m1 {
            g.timers.cancel(t3);
        }
    }));
    assert_eq!(tick_to(&mut g, &mut rec, 12), ["M1", "M2"]);
    assert_eq!(tick_to(&mut g, &mut rec, 76), Vec::<String>::new());
}

#[test]
fn cancel_cursor_target_advances_cursor() {
    // Consequence 2: cancelling the next timer while the one
    // before it runs.
    let (mut g, mut rec, _) = setup(&[
        ("A", UnitType::Monster),
        ("B", UnitType::Monster),
        ("C", UnitType::Monster),
    ]);
    g.frame = 10;
    sched(&mut g, &rec, "A", 12);
    let tb = sched(&mut g, &rec, "B", 12).unwrap();
    sched(&mut g, &rec, "C", 12);
    let a = id(&rec, "A");
    rec.on_run = Some(Box::new(move |g, run| {
        if run.owner.unit == a {
            g.timers.cancel(tb);
        }
    }));
    assert_eq!(tick_to(&mut g, &mut rec, 12), ["A", "C"]);
}

#[test]
fn event_type_15_not_scheduled() {
    // Vector: event type 15 → not scheduled (timed and every-tick).
    let (mut g, rec, _) = setup(&[("A", UnitType::Monster)]);
    let a = id(&rec, "A");
    assert_eq!(g.schedule_event(a, 15, 20, None, 0, 0), Ok(None));
    assert_eq!(g.schedule_event(a, 15, -1, None, 0, 0), Ok(None));
    assert!(g.schedule_event(a, 14, 20, None, 0, 0).unwrap().is_some());
    assert_eq!(g.timers.unit_timers(a).len(), 1);
}

#[test]
fn tiles_have_no_timer_class() {
    let (mut g, rec, _) = setup(&[("T", UnitType::Tile)]);
    assert_eq!(
        g.schedule_event(id(&rec, "T"), 0, 20, None, 0, 0),
        Err(TimerError::NoTimerClass(UnitType::Tile).into())
    );
}

#[test]
fn every_tick_scheduled_during_run_waits_a_tick() {
    // Edge case 4: scheduled inside the run → not this tick; scheduled in
    // step 3 → this tick's step 4.
    let (mut g, mut rec, _) = setup(&[("X", UnitType::Missile), ("Y", UnitType::Missile)]);
    let y = id(&rec, "Y");
    sched(&mut g, &rec, "X", -1);
    let mut done = false;
    rec.on_run = Some(Box::new(move |g, _| {
        if !done {
            done = true;
            g.schedule_event(y, 0, -1, None, 0, 0).unwrap();
        }
    }));
    assert_eq!(tick_to(&mut g, &mut rec, 1), ["X"]);
    assert_eq!(tick_to(&mut g, &mut rec, 2), ["Y", "X"]);
}

#[test]
fn every_tick_scheduled_before_step_4_runs_same_tick() {
    struct Pre {
        inner: Rec,
        unit: UnitId,
    }
    impl EventDispatch for Pre {
        fn run_event(&mut self, g: &mut Game, r: &TimerRun) {
            self.inner.run_event(g, r)
        }
    }
    impl TickHooks for Pre {
        fn ambient_spawns(&mut self, g: &mut Game, _: RoomId) {
            g.schedule_event(self.unit, 0, -1, None, 0, 0).unwrap();
        }
    }
    let (mut g, rec, _) = setup(&[("X", UnitType::Monster)]);
    let unit = id(&rec, "X");
    let mut pre = Pre { inner: rec, unit };
    tick(&mut g, &mut pre);
    assert_eq!(pre.inner.runs(), ["X"]);
}

#[test]
fn unit_removed_during_its_event() {
    // Consequence 3: the unit's other timers are cancelled, the executing
    // one freed after the callback; the slot can be reused.
    let (mut g, mut rec, room) = setup(&[("S", UnitType::Missile), ("T", UnitType::Missile)]);
    let s = id(&rec, "S");
    sched(&mut g, &rec, "S", -1);
    sched(&mut g, &rec, "S", 1);
    sched(&mut g, &rec, "S", 2);
    sched(&mut g, &rec, "T", 1);
    rec.on_run = Some(Box::new(move |g, run| {
        if run.owner.unit == s && run.list == TimerList::EveryTick {
            g.remove_unit(s).unwrap();
            // Reuse the slot at once.
            let n = g.spawn_unit(UnitType::Missile, Some(room), false).unwrap();
            assert_eq!(n, s);
            assert!(g.timers.unit_timers(n).is_empty());
            let t = g.schedule_event(n, 3, 5, None, 0, 0).unwrap().unwrap();
            assert_eq!(g.timers.unit_timers(n), [t]);
        }
    }));
    assert_eq!(tick_to(&mut g, &mut rec, 1), ["S", "T"]);
    rec.on_run = None;
    // The new unit (same slot, new GUID 3) keeps only its own timer.
    assert_eq!(g.timers.unit_timers(s).len(), 1);
    assert!(g.timers.every_tick(TimerClass::Missile).is_empty());
    assert!(tick_to(&mut g, &mut rec, 4).is_empty());
    assert_eq!(tick_to(&mut g, &mut rec, 5), ["S"]);
}

#[test]
fn every_tick_cancelled_during_its_run_is_freed_after() {
    let (mut g, mut rec, _) = setup(&[("X", UnitType::Player), ("Y", UnitType::Player)]);
    let tx = sched(&mut g, &rec, "X", -1).unwrap();
    sched(&mut g, &rec, "Y", -1).unwrap();
    let x = id(&rec, "X");
    rec.on_run = Some(Box::new(move |g, run| {
        if run.owner.unit == x {
            g.timers.cancel(run.timer);
            assert_eq!(
                g.timers.flags(run.timer),
                Some(flags::EVERY_TICK | flags::EXECUTING | flags::DELETE)
            );
        }
    }));
    assert_eq!(tick_to(&mut g, &mut rec, 1), ["Y", "X"]);
    assert_eq!(g.timers.flags(tx), None);
    assert_eq!(tick_to(&mut g, &mut rec, 2), ["Y"]);
}

#[test]
fn cancel_helpers() {
    // By type, by type and argument, by type and callback.
    let (mut g, mut rec, _) = setup(&[("A", UnitType::Player)]);
    let a = id(&rec, "A");
    let cb = Some(CallbackId(7));
    let t5a = g.schedule_event(a, 5, 20, None, 100, 1).unwrap().unwrap();
    let t5b = g.schedule_event(a, 5, 20, None, 200, 1).unwrap().unwrap();
    let t0 = g.schedule_event(a, 0, -1, None, 0, 0).unwrap().unwrap();
    let t3 = g.schedule_event(a, 3, 20, cb, 0, 0).unwrap().unwrap();
    let t3n = g.schedule_event(a, 3, 20, None, 0, 0).unwrap().unwrap();
    assert_eq!(g.timers.unit_timers(a), [t3n, t3, t0, t5b, t5a]);
    g.timers.cancel_unit_events(a, 5, Some(200));
    assert_eq!(g.timers.unit_timers(a), [t3n, t3, t0, t5a]);
    g.timers.cancel_unit_events_with_callback(a, 3, cb);
    assert_eq!(g.timers.unit_timers(a), [t3n, t0, t5a]);
    g.timers.cancel_unit_events(a, 0, None);
    assert_eq!(g.timers.unit_timers(a), [t3n, t5a]);
    assert!(g.timers.every_tick(TimerClass::Player).is_empty());
    g.timers.cancel_unit_timers(a);
    assert!(g.timers.unit_timers(a).is_empty());
    assert!(tick_to(&mut g, &mut rec, 20).is_empty());
}

#[test]
fn every_tick_schedule_drops_the_callback() {
    // expire −1 through the timed scheduler: an every-tick event with the
    // same type and arguments but a null callback, so the class default
    // handler runs; a cancel by that callback no longer finds it.
    let (mut g, mut rec, _) = setup(&[("A", UnitType::Monster)]);
    let a = id(&rec, "A");
    let cb = Some(CallbackId(9));
    let t = g.schedule_event(a, 2, -1, cb, 11, 22).unwrap().unwrap();
    assert_eq!(g.timers.every_tick(TimerClass::Monster), [t]);
    g.timers.cancel_unit_events_with_callback(a, 2, cb);
    assert_eq!(g.timers.unit_timers(a), [t]);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s2 = seen.clone();
    rec.on_run = Some(Box::new(move |_, run| s2.borrow_mut().push(*run)));
    tick_to(&mut g, &mut rec, 1);
    let runs = seen.borrow();
    assert_eq!(runs.len(), 1);
    let r = runs[0];
    assert_eq!(
        (r.list, r.event, r.expire, r.arg1, r.arg2, r.callback),
        (TimerList::EveryTick, 2, -1, 11, 22, None)
    );
}

#[test]
fn timer_run_record_carries_trace_fields() {
    let (mut g, mut rec, _) = setup(&[("A", UnitType::Monster)]);
    let a = id(&rec, "A");
    g.schedule_event(a, 2, 3, Some(CallbackId(9)), 11, 22)
        .unwrap();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s2 = seen.clone();
    rec.on_run = Some(Box::new(move |_, run| s2.borrow_mut().push(*run)));
    tick_to(&mut g, &mut rec, 3);
    let runs = seen.borrow();
    assert_eq!(runs.len(), 1);
    let r = runs[0];
    assert_eq!(
        (r.class, r.list, r.event, r.expire, r.arg1, r.arg2, r.callback),
        (
            TimerClass::Monster,
            TimerList::Due,
            2,
            3,
            11,
            22,
            Some(CallbackId(9))
        )
    );
    assert_eq!(
        r.owner,
        TimerOwner {
            unit: a,
            unit_type: UnitType::Monster,
            guid: 1
        }
    );
}

#[test]
fn uninterruptable_check_applies_to_timed_monster_ai_think() {
    use timer::needs_uninterruptable_check as f;
    assert!(f(UnitType::Monster, 2, 30));
    assert!(!f(UnitType::Monster, 2, -1));
    assert!(!f(UnitType::Monster, 3, 30));
    assert!(!f(UnitType::Player, 2, 30));
}

#[test]
fn step_order() {
    // Environment, rooms, timers, clients, update queues, removal
    // records, then periodic steps.
    let (mut g, mut rec, room) = setup(&[("P", UnitType::Player)]);
    let p = id(&rec, "P");
    g.lists
        .add_client(Some(p), Some(room), client_state::IN_GAME);
    g.lists.room_mut(room).unwrap().adjacent = vec![room];
    g.lists.act_mut(0).unwrap().pending_removals = true;
    sched(&mut g, &rec, "P", -1);
    g.frame = 659;
    tick(&mut g, &mut rec);
    let r = room.0;
    assert_eq!(
        rec.take(),
        [
            "env 0".to_string(),
            format!("ambient {r}"),
            format!("presets {r}"),
            format!("restore {r}"),
            format!("objects {r}"),
            format!("monsters {r}"),
            "run P".into(),
            "removals".into(),
            "send P".into(),
            "client msgs".into(),
            "arena sync".into(),
            "update P".into(),
            "step6 end".into(),
            format!("removal records {r}"),
            "step8".into(),
            "step9".into(),
            format!("inactivity {r}"),
            "step10 0".into(),
        ]
    );
    assert!(!g.lists.act(0).unwrap().pending_removals);
}

#[test]
fn room_pass_newest_first_and_flags() {
    // Rooms activated together are populated newest first; the act
    // flag is cleared; a populated room whose units are inactive only
    // restores.
    let mut g = Game::new();
    let mut rec = Rec::default();
    g.lists.ensure_act(1).unwrap();
    let r1 = g.lists.create_room(1).unwrap();
    let r2 = g.lists.create_room(1).unwrap();
    g.lists.activate_room(r1).unwrap();
    g.lists.activate_room(r2).unwrap();
    tick(&mut g, &mut rec);
    let pass: Vec<_> = rec
        .take()
        .into_iter()
        .filter(|s| !s.starts_with("env") && !s.starts_with("step"))
        .collect();
    let (a, b) = (r2.0, r1.0);
    assert_eq!(
        pass,
        [
            format!("ambient {a}"),
            format!("presets {a}"),
            format!("restore {a}"),
            format!("objects {a}"),
            format!("monsters {a}"),
            format!("ambient {b}"),
            format!("presets {b}"),
            format!("restore {b}"),
            format!("objects {b}"),
            format!("monsters {b}"),
        ]
    );
    assert!(!g.lists.act(1).unwrap().pending_rooms);
    for r in [r1, r2] {
        let e = g.lists.room(r).unwrap();
        assert!(e.populated && e.units_active);
    }
    // No pending flag: no pass.
    tick(&mut g, &mut rec);
    assert!(!rec.take().iter().any(|s| s.starts_with("ambient")));
    // Units marked inactive, act flagged again: restore only; populated
    // and active: ambient only.
    g.lists.room_mut(r1).unwrap().units_active = false;
    g.lists.act_mut(1).unwrap().pending_rooms = true;
    tick(&mut g, &mut rec);
    let pass: Vec<_> = rec
        .take()
        .into_iter()
        .filter(|s| !s.starts_with("env") && !s.starts_with("step"))
        .collect();
    assert_eq!(
        pass,
        [
            format!("ambient {a}"),
            format!("ambient {b}"),
            format!("restore {b}")
        ]
    );
}

#[test]
fn off_tick_population_runs_now_and_step3_revisits() {
    // The room body at once, outside step 3, without the act
    // flag test and without clearing act +0x54; the next step 3 visits the
    // room again: one more ambient call, then nothing.
    let mut g = Game::new();
    let mut rec = Rec::default();
    g.lists.ensure_act(1).unwrap();
    let r = g.lists.create_room(1).unwrap();
    g.lists.activate_room(r).unwrap();
    assert!(g.lists.act(1).unwrap().pending_rooms);
    super::populate_room(&mut g, &mut rec, r);
    let n = r.0;
    assert_eq!(
        rec.take(),
        [
            format!("ambient {n}"),
            format!("presets {n}"),
            format!("restore {n}"),
            format!("objects {n}"),
            format!("monsters {n}"),
        ]
    );
    let e = g.lists.room(r).unwrap();
    assert!(e.populated && e.units_active);
    assert!(g.lists.act(1).unwrap().pending_rooms);
    // Without the act flag the call still runs (ambient only now).
    g.lists.act_mut(1).unwrap().pending_rooms = false;
    super::populate_room(&mut g, &mut rec, r);
    assert_eq!(rec.take(), [format!("ambient {n}")]);
    g.lists.act_mut(1).unwrap().pending_rooms = true;
    tick(&mut g, &mut rec);
    let pass: Vec<_> = rec
        .take()
        .into_iter()
        .filter(|s| !s.starts_with("env") && !s.starts_with("step"))
        .collect();
    assert_eq!(pass, [format!("ambient {n}")]);
    assert!(!g.lists.act(1).unwrap().pending_rooms);
}

#[test]
fn client_pass_join_and_updates() {
    //– and unit-order–: newest client first; update
    // messages most recently queued first; step 6 walks and clears.
    let (mut g, mut rec, room) = setup(&[
        ("P", UnitType::Player),
        ("A", UnitType::Monster),
        ("B", UnitType::Monster),
    ]);
    g.lists.room_mut(room).unwrap().adjacent = vec![room];
    let p = id(&rec, "P");
    let c = g
        .lists
        .add_client(Some(p), Some(room), client_state::JOINING);
    // Not ready: update only, stays joining.
    tick(&mut g, &mut rec);
    let log = rec.take();
    let i = log.iter().position(|s| s == "removals").unwrap();
    assert_eq!(
        log[i..i + 6],
        [
            "removals",
            "send B",
            "send A",
            "send P",
            "client msgs",
            "arena sync"
        ]
    );
    assert_eq!(g.lists.client(c).unwrap().state, client_state::JOINING);
    assert_eq!(g.lists.client(c).unwrap().update_count, 1);
    // The player is queued at the end of the client update, then step 6
    // walks and clears every queue.
    assert!(log.contains(&"update P".to_string()));
    assert!(g.lists.update_queue(room).is_empty());
    assert!(!g.lists.act(0).unwrap().pending_updates);
    // Ready: message 4, state 4, inventory, join.
    rec.room_ready = true;
    tick(&mut g, &mut rec);
    let log = rec.take();
    let i = log.iter().position(|s| s == "msg4 state3").unwrap();
    assert_eq!(log[i..i + 3], ["msg4 state3", "inventory state4", "join"]);
    assert_eq!(g.lists.client(c).unwrap().state, client_state::IN_GAME);
    // The player moves to another room: level change.
    let r2 = g.lists.create_room(0).unwrap();
    g.lists.activate_room(r2).unwrap();
    g.lists.change_room(p, r2).unwrap();
    tick(&mut g, &mut rec);
    assert!(rec.take().contains(&"level change".to_string()));
    assert_eq!(g.lists.client(c).unwrap().room, Some(r2));
}

#[test]
fn changing_act_client_does_not_rejoin() {
    let (mut g, mut rec, room) = setup(&[("P", UnitType::Player)]);
    let p = id(&rec, "P");
    let c = g
        .lists
        .add_client(Some(p), Some(room), client_state::CHANGING_ACT);
    rec.room_ready = true;
    tick(&mut g, &mut rec);
    let log = rec.take();
    assert!(log.contains(&"msg4 state5".to_string()));
    assert!(!log.contains(&"join".to_string()));
    assert_eq!(g.lists.client(c).unwrap().state, client_state::IN_GAME);
}

#[test]
fn room_deactivation_compresses_and_unlinks() {
    // Step 9: units compressed in room-list order with the next saved
    // first (the hook removes them); the room leaves the act list.
    let (mut g, mut rec, room) = setup(&[("A", UnitType::Monster), ("B", UnitType::Monster)]);
    let r2 = g.lists.create_room(0).unwrap();
    g.lists.activate_room(r2).unwrap();
    rec.inactivity = 11;
    rec.allow_removal = true;
    g.frame = 11;
    tick(&mut g, &mut rec);
    let log = rec.take();
    let compress: Vec<_> = log.iter().filter(|s| s.starts_with("compress")).collect();
    assert_eq!(compress, ["compress B", "compress A"]);
    assert!(g.lists.active_rooms(0).is_empty());
    assert!(g.lists.room_units(room).is_empty());
    // Threshold: 10 is not enough.
    let (mut g, mut rec, _) = setup(&[("A", UnitType::Monster)]);
    rec.inactivity = 10;
    rec.allow_removal = true;
    g.frame = 11;
    tick(&mut g, &mut rec);
    assert_eq!(g.lists.active_rooms(0).len(), 1);
}

#[test]
fn hash_helpers() {
    // unit-order and edge case 3.
    let (mut g, rec, room) = setup(&[]);
    let _ = rec;
    for _ in 0..3 {
        g.spawn_unit(UnitType::Monster, Some(room), false).unwrap();
    }
    g.spawn_unit(UnitType::Object, Some(room), false).unwrap();
    assert_eq!(
        g.find_unit_of_type(UnitType::Object, |_, _| false, |_, _| true),
        None
    );
    let mut seen = Vec::new();
    let found = g.find_unit_of_type(
        UnitType::Monster,
        |g, u| g.lists.unit(u).unwrap().guid == 1, // "has state 7"
        |g, u| {
            let guid = g.lists.unit(u).unwrap().guid;
            seen.push(guid);
            guid == 3
        },
    );
    assert_eq!(seen, [2, 3]);
    assert_eq!(g.lists.unit(found.unwrap()).unwrap().guid, 3);
    for _ in 0..2 {
        g.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    }
    let mut seen = Vec::new();
    g.for_each_player(
        |_, _| false,
        |g, u| seen.push(g.lists.unit(u).unwrap().guid),
    );
    assert_eq!(seen, [1, 2]);
}

#[test]
fn dispatch_data_matches_spec() {
    // tables.
    let set = |f: fn(u8) -> bool| (0..15).filter(|&t| f(t)).collect::<Vec<u8>>();
    assert_eq!(
        set(events::monster_dropped_when_frozen),
        [0, 1, 2, 6, 7, 9, 10, 11, 13, 14]
    );
    assert_eq!(
        set(events::player_has_handler),
        [0, 1, 3, 5, 6, 8, 9, 11, 12, 13]
    );
    assert_eq!(
        set(events::monster_has_handler),
        [0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 12]
    );
    assert!(!events::player_has_handler(15));
}

/// Records [`Game::character_save_due`] when the per-client loop runs.
#[derive(Default)]
struct SaveSeen(Vec<(i32, bool)>);

impl EventDispatch for SaveSeen {
    fn run_event(&mut self, _: &mut Game, _: &TimerRun) {}
}

impl TickHooks for SaveSeen {
    fn send_removed_units(&mut self, g: &mut Game, _: ClientId) {
        self.0.push((g.frame, g.character_save_due));
    }
}

#[test]
fn the_client_pass_raises_the_character_save_every_8192_frames() {
    let mut g = Game::new();
    g.lists.ensure_act(0).unwrap();
    let room = g.lists.create_room(0).unwrap();
    g.lists.activate_room(room).unwrap();
    g.lists.add_client(None, Some(room), client_state::IN_GAME);
    let mut h = SaveSeen::default();
    g.frame = 8190;
    // Frame 8191: no save; the host clears the flag after each tick.
    tick(&mut g, &mut h);
    assert!(!g.character_save_due);
    // Frame 8192: raised before the per-client loop.
    tick(&mut g, &mut h);
    assert!(g.character_save_due);
    assert_eq!(h.0, [(8191, false), (8192, true)]);
    g.character_save_due = false;
    // The next period, and not between.
    g.frame = 2 * 8192 - 2;
    tick(&mut g, &mut h);
    assert!(!g.character_save_due);
    tick(&mut g, &mut h);
    assert!(g.character_save_due);
}

/// The day clock (S→C 0x53) counts base ticks: every tick at 25 Hz, 25 of
/// any 60 consecutive ticks at 60 Hz.
#[test]
fn the_day_clock_advances_once_per_base_tick() {
    let mut g = Game::new();
    for f in 0..50 {
        g.frame = f;
        assert!(environment_due(&g));
    }
    g.tick_rate = crate::tick::TickRate::new(60).unwrap();
    for start in [0, 7, 1000] {
        let n = (start..start + 60)
            .filter(|&f| {
                g.frame = f;
                environment_due(&g)
            })
            .count();
        assert_eq!(n, 25, "from frame {start}");
    }
}
