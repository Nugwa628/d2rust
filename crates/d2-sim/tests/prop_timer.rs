// Spec: specs/sim/tick.md §5; specs/sim/unit-order.md §3.2, §8
//! State-machine property test of the timer queue (`d2_sim::tick::timer`
//! through `d2_sim::game::Game` and `d2_sim::tick::tick`) against a
//! reference model written from `tick.md` §5:
//!
//! - scheduling (§5.2, §5.3): types ≥ 15 ignored, expire −1 = every
//!   tick, expire ≤ frame → frame + 1; every-tick lists newest first,
//!   bucket `expire % 64` in scheduling order; the unit's timer list
//!   newest first (`unit-order.md` §8);
//! - running (§5.5): classes missile, player, monster, object, item; per
//!   class the every-tick timers newest first, then the due timers
//!   (expire = frame) in scheduling order, each list read when the run
//!   reaches it (r3); consequences 1–4: nothing scheduled during the run
//!   runs in a list already started (see below), a timer cancelled
//!   (alone, by unit helpers or by unit removal) before it is reached
//!   never runs, a due timer is freed after its run, an every-tick timer
//!   stays unless cancelled;
//! - rejection: tile units have no timer class and are refused without a
//!   state change.
//!
//! Consequence 1 says an event scheduled during the run never runs in the
//! same tick. That holds for the list being visited and the ones before
//! it, not for a later class: by r3 a class's every-tick list is read
//! from its head when the run reaches it, so an every-tick event of a
//! later class (e.g. a monster mode change scheduled from a missile hit)
//! runs in the same tick. The model follows the rules (as the code and
//! the `sim/tick` trace replays do); the wording is reported in
//! `docs/handoff/prop-sim-core.md`.
//!
//! Each timer carries a unique handle in arg1, so a run identifies its
//! model timer whatever slot the queue reused.

use std::collections::VecDeque;

use d2_sim::game::{Game, GameError};
use d2_sim::tick::timer::{flags, TimerClass, TimerError, TimerId, TimerList, TimerRun, BUCKETS};
use d2_sim::tick::{tick, EventDispatch, TickHooks};
use d2_sim::units::{UnitId, UnitType};
use proptest::prelude::*;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// How a schedule picks its requested expire.
#[derive(Clone, Copy, Debug)]
enum Expire {
    EveryTick,
    /// frame + n (n may be ≤ 0: moved to frame + 1).
    Rel(i32),
    /// An absolute value (any i32 but those that would wrap the frame).
    Abs(i32),
}

/// One action, either at the top level or inside a running event.
/// Selectors pick among the live units / timers at the time.
#[derive(Clone, Copy, Debug)]
enum Act {
    Spawn(UnitType),
    Schedule {
        unit: usize,
        event: u32,
        expire: Expire,
    },
    Cancel(usize),
    /// Cancel the running timer (only inside an event; else a no-op).
    CancelSelf,
    CancelUnitEvents {
        unit: usize,
        event: u8,
        by_arg: Option<usize>,
    },
    CancelUnitTimers(usize),
    Remove(usize),
    /// Remove the running timer's unit (inside an event).
    RemoveSelf,
}

fn unit_type() -> impl Strategy<Value = UnitType> {
    prop_oneof![
        3 => Just(UnitType::Player),
        3 => Just(UnitType::Monster),
        3 => Just(UnitType::Missile),
        2 => Just(UnitType::Object),
        2 => Just(UnitType::Item),
        1 => Just(UnitType::Tile),
    ]
}

fn expire() -> impl Strategy<Value = Expire> {
    prop_oneof![
        3 => Just(Expire::EveryTick),
        6 => (-3i32..8).prop_map(Expire::Rel),
        3 => (60i32..140).prop_map(Expire::Rel),
        1 => any::<i32>().prop_map(Expire::Abs),
    ]
}

fn act() -> impl Strategy<Value = Act> {
    let sel = any::<usize>();
    prop_oneof![
        3 => unit_type().prop_map(Act::Spawn),
        8 => (sel, prop_oneof![9 => 0u32..15, 1 => 15u32..300], expire())
            .prop_map(|(unit, event, expire)| Act::Schedule { unit, event, expire }),
        3 => sel.prop_map(Act::Cancel),
        1 => Just(Act::CancelSelf),
        1 => (sel, 0u8..15, prop::option::of(sel))
            .prop_map(|(unit, event, by_arg)| Act::CancelUnitEvents { unit, event, by_arg }),
        1 => sel.prop_map(Act::CancelUnitTimers),
        1 => sel.prop_map(Act::Remove),
        1 => Just(Act::RemoveSelf),
    ]
}

#[derive(Clone, Debug)]
enum Step {
    Act(Act),
    /// One tick; each run consumes the next entry of the callback script
    /// (the actions done inside that event).
    Tick,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => act().prop_map(Step::Act),
        1 => Just(Step::Tick),
    ]
}

#[derive(Clone, Debug)]
struct MUnit {
    id: UnitId,
    ty: UnitType,
    guid: u32,
}

#[derive(Clone, Debug)]
struct MTimer {
    /// Unique handle, also the timer's arg1.
    handle: u32,
    id: TimerId,
    unit: UnitId,
    class: TimerClass,
    event: u8,
    /// −1 for every-tick.
    expire: i32,
    live: bool,
}

#[derive(Default)]
struct Model {
    units: Vec<MUnit>,
    timers: Vec<MTimer>,
    next_handle: u32,
}

fn class_of(ty: UnitType) -> Option<TimerClass> {
    match ty {
        UnitType::Player => Some(TimerClass::Player),
        UnitType::Monster => Some(TimerClass::Monster),
        UnitType::Missile => Some(TimerClass::Missile),
        UnitType::Object => Some(TimerClass::Object),
        UnitType::Item => Some(TimerClass::Item),
        UnitType::Tile => None,
    }
}

impl Model {
    fn live_timers(&self) -> impl Iterator<Item = &MTimer> {
        self.timers.iter().filter(|t| t.live)
    }

    fn timer_by_handle(&mut self, h: u32) -> &mut MTimer {
        &mut self.timers[h as usize]
    }

    fn kill_unit_timers(&mut self, unit: UnitId, pred: impl Fn(&MTimer) -> bool) {
        for t in self.timers.iter_mut() {
            if t.live && t.unit == unit && pred(t) {
                t.live = false;
            }
        }
    }
}

/// Applies one action to the game and the model. `running` is the timer
/// whose event is executing, if any.
fn apply(game: &mut Game, m: &mut Model, a: Act, running: Option<&TimerRun>) {
    let pick_unit = |m: &Model, s: usize| (!m.units.is_empty()).then(|| s % m.units.len());
    match a {
        Act::Spawn(ty) => {
            let id = game
                .spawn_unit(ty, None, false)
                .expect("spawn without a room");
            let guid = game.lists.unit(id).unwrap().guid;
            m.units.push(MUnit { id, ty, guid });
        }
        Act::Schedule {
            unit,
            event,
            expire,
        } => {
            let Some(i) = pick_unit(m, unit) else { return };
            let u = m.units[i].clone();
            let frame = game.frame;
            let req = match expire {
                Expire::EveryTick => -1,
                Expire::Rel(n) => frame + n,
                Expire::Abs(v) => v,
            };
            let handle = m.next_handle;
            let got = game.schedule_event(u.id, event, req, None, handle, 7);
            // §5.2 rule 1; tiles have no class (refused, tick.md edge
            // case 3 / handoff T4).
            if event >= 15 {
                assert_eq!(got, Ok(None), "type {event} must not schedule");
                return;
            }
            let Some(class) = class_of(u.ty) else {
                assert_eq!(
                    got,
                    Err(GameError::Timer(TimerError::NoTimerClass(UnitType::Tile)))
                );
                return;
            };
            let id = got.expect("schedule").expect("a timer");
            let expire = match req {
                -1 => -1,
                e if e <= frame => frame + 1,
                e => e,
            };
            assert_eq!(game.timers.expire(id), Some(expire));
            m.timers.push(MTimer {
                handle,
                id,
                unit: u.id,
                class,
                event: event as u8,
                expire,
                live: true,
            });
            m.next_handle += 1;
        }
        Act::Cancel(s) => {
            let live: Vec<u32> = m.live_timers().map(|t| t.handle).collect();
            if live.is_empty() {
                return;
            }
            let h = live[s % live.len()];
            let t = m.timer_by_handle(h);
            t.live = false;
            let id = t.id;
            game.timers.cancel(id);
        }
        Act::CancelSelf => {
            if let Some(r) = running {
                // Cancelling an executing timer only marks it (§5.4 r1).
                game.timers.cancel(r.timer);
                assert_ne!(game.timers.flags(r.timer).unwrap() & flags::DELETE, 0);
                m.timer_by_handle(r.arg1).live = false;
            }
        }
        Act::CancelUnitEvents {
            unit,
            event,
            by_arg,
        } => {
            let Some(i) = pick_unit(m, unit) else { return };
            let u = m.units[i].id;
            let arg = by_arg.and_then(|s| {
                let mine: Vec<u32> = m
                    .live_timers()
                    .filter(|t| t.unit == u)
                    .map(|t| t.handle)
                    .collect();
                (!mine.is_empty()).then(|| mine[s % mine.len()])
            });
            game.timers.cancel_unit_events(u, event, arg);
            m.kill_unit_timers(u, |t| t.event == event && arg.is_none_or(|a| t.handle == a));
        }
        Act::CancelUnitTimers(s) => {
            let Some(i) = pick_unit(m, s) else { return };
            let u = m.units[i].id;
            game.timers.cancel_unit_timers(u);
            m.kill_unit_timers(u, |_| true);
        }
        Act::Remove(s) => {
            let Some(i) = pick_unit(m, s) else { return };
            let u = m.units.remove(i).id;
            game.remove_unit(u).expect("live unit");
            m.kill_unit_timers(u, |_| true);
        }
        Act::RemoveSelf => {
            let Some(r) = running else { return };
            let Some(i) = m.units.iter().position(|u| u.id == r.owner.unit) else {
                return;
            };
            let u = m.units.remove(i).id;
            game.remove_unit(u).expect("live unit");
            m.kill_unit_timers(u, |_| true);
        }
    }
}

/// The lists of one tick in run order (§5.5 r2): per class of
/// `RUN_ORDER`, the every-tick list, then the due bucket.
const SEGMENTS: usize = 10;

fn segment(i: usize) -> (TimerClass, TimerList) {
    let list = if i.is_multiple_of(2) {
        TimerList::EveryTick
    } else {
        TimerList::Due
    };
    (TimerClass::RUN_ORDER[i / 2], list)
}

/// The model's visit order of one list at `frame`, taken when the run
/// reaches it (§5.5 r3: the cursor starts at the list's head then):
/// every-tick timers newest first, due timers (expire = frame) in
/// scheduling order. A timer runs only if still live when reached
/// (consequence 2); timers added to this list after it started are
/// behind the cursor or not due (consequence 1).
fn visit_order(m: &Model, seg: usize, frame: i32) -> VecDeque<u32> {
    let (class, list) = segment(seg);
    let mut out: Vec<u32> = m
        .live_timers()
        .filter(|t| t.class == class)
        .filter(|t| match list {
            TimerList::EveryTick => t.expire == -1,
            TimerList::Due => t.expire == frame,
        })
        .map(|t| t.handle)
        .collect();
    if list == TimerList::EveryTick {
        out.reverse();
    }
    out.into()
}

struct Driver {
    model: Model,
    frame: i32,
    /// Current list of the run and what is left of its visit order.
    seg: usize,
    expected: VecDeque<u32>,
    script: VecDeque<Vec<Act>>,
    runs: usize,
}

impl Driver {
    fn begin_tick(&mut self, frame: i32) {
        self.frame = frame;
        self.seg = 0;
        self.expected = visit_order(&self.model, 0, frame);
    }

    /// The next timer the model runs, moving to later lists as each is
    /// exhausted; `None` after the last list.
    fn next_expected(&mut self) -> Option<(u32, TimerList)> {
        loop {
            while let Some(&h) = self.expected.front() {
                if self.model.timers[h as usize].live {
                    return Some((h, segment(self.seg).1));
                }
                self.expected.pop_front();
            }
            if self.seg + 1 == SEGMENTS {
                return None;
            }
            self.seg += 1;
            self.expected = visit_order(&self.model, self.seg, self.frame);
        }
    }
}

impl EventDispatch for Driver {
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        let (h, list) = self
            .next_expected()
            .expect("a run the model does not expect");
        self.expected.pop_front();
        assert_eq!((run.arg1, run.list), (h, list), "run order (tick.md §5.5)");
        let t = self.model.timers[h as usize].clone();
        assert_eq!(run.timer, t.id);
        assert_eq!(run.class, t.class);
        assert_eq!(run.event, t.event);
        assert_eq!(run.expire, t.expire);
        assert_eq!(run.arg2, 7);
        assert_eq!(run.owner.unit, t.unit);
        let u = self
            .model
            .units
            .iter()
            .find(|u| u.id == t.unit)
            .expect("owner live");
        assert_eq!((run.owner.unit_type, run.owner.guid), (u.ty, u.guid));
        assert_ne!(game.timers.flags(run.timer).unwrap() & flags::EXECUTING, 0);
        self.runs += 1;
        let acts = self.script.pop_front().unwrap_or_default();
        for a in acts {
            apply(game, &mut self.model, a, Some(run));
        }
        // A due timer is freed after its run (§5.5 r3).
        if list == TimerList::Due {
            self.model.timers[h as usize].live = false;
        }
    }
}

impl TickHooks for Driver {}

/// The queue's lists equal the model's (§5.1–§5.3, `unit-order.md` §8).
fn check(game: &Game, m: &Model) {
    let handle = |id: TimerId| game.timers.event(id).expect("live timer").1;
    for class in TimerClass::RUN_ORDER {
        let got: Vec<u32> = game
            .timers
            .every_tick(class)
            .into_iter()
            .map(handle)
            .collect();
        let mut want: Vec<u32> = m
            .live_timers()
            .filter(|t| t.class == class && t.expire == -1)
            .map(|t| t.handle)
            .collect();
        want.reverse();
        assert_eq!(got, want, "every-tick list of {class:?}");
        for b in 0..BUCKETS {
            let got: Vec<u32> = game
                .timers
                .bucket(class, b as usize)
                .into_iter()
                .map(handle)
                .collect();
            let want: Vec<u32> = m
                .live_timers()
                .filter(|t| t.class == class && t.expire != -1 && t.expire % BUCKETS == b)
                .map(|t| t.handle)
                .collect();
            assert_eq!(got, want, "bucket {b} of {class:?}");
        }
    }
    for t in m.live_timers() {
        let f = if t.expire == -1 { flags::EVERY_TICK } else { 0 };
        assert_eq!(game.timers.flags(t.id), Some(f), "flags of a resting timer");
        assert_eq!(game.timers.expire(t.id), Some(t.expire));
    }
    for u in &m.units {
        let got: Vec<u32> = game
            .timers
            .unit_timers(u.id)
            .into_iter()
            .map(handle)
            .collect();
        let mut want: Vec<u32> = m
            .live_timers()
            .filter(|t| t.unit == u.id)
            .map(|t| t.handle)
            .collect();
        want.reverse();
        assert_eq!(got, want, "timer list of unit {:?}", u.id);
    }
}

fn run(start: i32, steps: Vec<Step>, script: Vec<Vec<Act>>) {
    let mut game = Game::new();
    game.frame = start;
    let mut d = Driver {
        model: Model::default(),
        frame: 0,
        seg: 0,
        expected: VecDeque::new(),
        script: script.into(),
        runs: 0,
    };
    for s in steps {
        match s {
            Step::Act(a) => apply(&mut game, &mut d.model, a, None),
            Step::Tick => {
                let frame = game.frame + 1;
                d.begin_tick(frame);
                tick(&mut game, &mut d);
                assert_eq!(game.frame, frame);
                assert_eq!(game.timers.current_bucket(), frame % BUCKETS);
                // Every timer the model visits ran (or was cancelled first).
                assert_eq!(
                    d.next_expected(),
                    None,
                    "a timer the model runs did not run"
                );
            }
        }
        check(&game, &d.model);
    }
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn timer_queue_matches_the_model(
        start in prop_oneof![Just(0i32), 0i32..100_000, Just(i32::MAX - 10_000)],
        steps in prop::collection::vec(step(), 0..120),
        script in prop::collection::vec(prop::collection::vec(act(), 0..3), 0..200),
    ) {
        run(start, steps, script);
    }
}

/// `tick.md` Test vectors, the run-order rows, through the model driver.
#[test]
fn spec_vectors_through_the_driver() {
    use Act::*;
    let s = |unit, expire| {
        Step::Act(Schedule {
            unit,
            event: 0,
            expire,
        })
    };
    let steps = vec![
        Step::Act(Spawn(UnitType::Player)),
        Step::Act(Spawn(UnitType::Monster)),
        Step::Act(Spawn(UnitType::Missile)),
        s(0, Expire::Rel(3)),
        s(1, Expire::Rel(3)),
        s(2, Expire::Rel(3)),
        s(2, Expire::EveryTick),
        s(1, Expire::EveryTick),
        s(1, Expire::Rel(67)),
        Step::Tick,
        Step::Tick,
        Step::Tick,
        Step::Tick,
    ];
    run(9, steps, Vec::new());
}
