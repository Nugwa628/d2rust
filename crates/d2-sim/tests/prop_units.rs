//! Property tests of the unit lifecycle and mode schedules
//! (`d2_sim::units::{lifecycle, modes, anim}`):
//!
//! - §4.2 the animation schedule against a closed form of its loop: with
//!   a_j = a0 + (j − 1)·s the frame position of iteration j, the loop
//!   runs J = ⌈(F − a0) / s⌉ iterations, event byte i is read in the
//!   first iteration with a_j ≥ 256·i, and the end event is at
//!   f + max(J, 1) + 1; a loop whose step would pass 2^31 − 1 before
//!   reaching F must be refused by the scheduler, as must negative speed,
//!   so invalid inputs cannot hang;
//! - §3.1 / §3.2 allocation and removal under random sequences: class
//!   checks with no draw, seed steps per kind (`rng.md` §5.3), GUIDs,
//!   mode, flags, stat lists, the missile's every-tick event, and
//!   rejected requests leaving no trace;
//! - §4.1 set mode, §4.4 every-tick movement and the animated starts
//!   through the timer queue.

use std::sync::Arc;

use d2_sim::game::{Game, GameError};
use d2_sim::rng::Seed;
use d2_sim::stats::lists::StatHost;
use d2_sim::stats::{StatData, StatLists};
use d2_sim::tick::timer::{flags as tflags, TimerClass};
use d2_sim::units::anim::{schedule, AnimError, Events, Form, Schedule, Scheduled};
use d2_sim::units::hooks::{MonsterInfo, Sim, UnitData, UnitHooks};
use d2_sim::units::lifecycle::{allocate, remove, AllocRequest, LifecycleHooks};
use d2_sim::units::modes::{player_start, set_mode, UnitError};
use d2_sim::units::record::{flags, flags2, AnimRecord, Units, ANIM_EVENTS};
use d2_sim::units::{ListError, RoomId, UnitId, UnitType};
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

// ---- §4.2 ----------------------------------------------------------------

/// The closed form of §4.2 (see the module docs), in i64.
fn reference(
    form: Form,
    f: i32,
    s: i32,
    frame_count: i32,
    cur: i32,
    byte_0f: u8,
    events: &[u8; ANIM_EVENTS],
) -> Result<Option<Schedule>, AnimError> {
    let variant = !matches!(form, Form::Main { .. });
    if matches!(form, Form::Frames(p) if p <= 0) {
        return Ok(None);
    }
    let end = |at: i32| Scheduled {
        event: 1,
        expire: at,
        a1: 0,
        a2: 0,
    };
    if s == 0 {
        return Ok(Some(Schedule {
            cancels: variant,
            events: vec![end(f.wrapping_add(1))],
            frame: None,
        }));
    }
    if s < 0 {
        return Err(AnimError::NegativeSpeed(s));
    }
    let cur_frame = cur >> 8;
    // Match the scheduler's wrapping i32 arithmetic for c and the start index.
    let c: i32 = match form {
        Form::Main { bonus } => bonus,
        Form::Percent(p) => (100i32.wrapping_sub(p)).wrapping_mul(f.wrapping_sub(cur_frame)) / 100,
        Form::Frames(p) => f.wrapping_sub(cur_frame).wrapping_sub(p),
        Form::StartFrame(p) => p,
    };
    let start = if variant {
        i64::from(c.wrapping_sub(1))
    } else {
        i64::from(c)
    };
    let a0 = i64::from(c.wrapping_mul(256).wrapping_add(s));
    let (s64, big_f) = (i64::from(s), i64::from(frame_count));
    let iterations = if a0 >= big_f {
        0
    } else {
        (big_f - a0 + s64 - 1) / s64
    };
    if iterations > 0 && a0 + iterations * s64 > i64::from(i32::MAX) {
        // The final a + s would exceed i32::MAX; the scheduler rejects it.
        return Err(AnimError::Endless {
            speed: s,
            frame_count,
        });
    }
    let mut out = Vec::new();
    if iterations > 0 {
        let last_read = (a0 + (iterations - 1) * s64) >> 8;
        let top = if variant {
            last_read
        } else {
            last_read.min(ANIM_EVENTS as i64 - 1)
        };
        if variant && start <= top {
            if start < -1 {
                return Err(AnimError::EventIndex(start as i32));
            }
            if top >= ANIM_EVENTS as i64 {
                return Err(AnimError::EventIndex(start.max(ANIM_EVENTS as i64) as i32));
            }
        }
        let mut k = 0;
        let mut i = start;
        while i <= top {
            // First iteration j ≥ 1 with a_j ≥ 256·i.
            let need = 256 * i - a0;
            let j = if need <= 0 {
                1
            } else {
                (need + s64 - 1) / s64 + 1
            };
            let n = f.wrapping_add(j as i32);
            let e = if i == -1 { byte_0f } else { events[i as usize] };
            if variant {
                if matches!(e, 1..=4) {
                    out.push(Scheduled {
                        event: 0,
                        expire: n,
                        a1: e.into(),
                        a2: 0,
                    });
                }
            } else if matches!(e, 1 | 2 | 4) {
                out.push(Scheduled {
                    event: 0,
                    expire: n,
                    a1: e.into(),
                    a2: k,
                });
                k += 1;
            } else if e == 3 {
                out.push(Scheduled {
                    event: 0,
                    expire: n,
                    a1: 3,
                    a2: 0,
                });
            }
            i += 1;
        }
    }
    out.push(end(f
        .wrapping_add(iterations.max(1) as i32)
        .wrapping_add(1)));
    let frame = if variant {
        f.wrapping_sub(c).wrapping_mul(256)
    } else {
        f.wrapping_mul(256)
    };
    Ok(Some(Schedule {
        cancels: variant,
        events: out,
        frame: Some(frame),
    }))
}

/// Iterations of the §4.2 loop (0 when s ≤ 0).
fn loop_length(form: Form, f: i32, s: i32, frame_count: i32, cur: i32) -> i64 {
    if s <= 0 {
        return 0;
    }
    let c: i32 = match form {
        Form::Main { bonus } => bonus,
        Form::Percent(p) => (100i32.wrapping_sub(p)).wrapping_mul(f.wrapping_sub(cur >> 8)) / 100,
        Form::Frames(p) => f.wrapping_sub(cur >> 8).wrapping_sub(p),
        Form::StartFrame(p) => p,
    };
    let a0 = i64::from(c.wrapping_mul(256).wrapping_add(s));
    ((i64::from(frame_count) - a0).max(0) + i64::from(s) - 1) / i64::from(s)
}

fn form() -> impl Strategy<Value = Form> {
    prop_oneof![
        3 => (0i32..=10).prop_map(|bonus| Form::Main { bonus }),
        1 => (-50i32..160).prop_map(Form::Percent),
        1 => (-5i32..200).prop_map(Form::Frames),
        1 => (-5i32..200).prop_map(Form::StartFrame),
        1 => prop_oneof![Just(i32::MIN), Just(i32::MAX), any::<i32>()].prop_map(Form::StartFrame),
        1 => prop_oneof![Just(i32::MIN), Just(i32::MAX), any::<i32>()].prop_map(Form::Frames),
        1 => any::<i32>().prop_map(Form::Percent),
    ]
}

/// Longest §4.2 loop the closed-form property runs (2^31 / s iterations
/// are possible for a wrapped start).
const MAX_LOOP: i64 = 200_000;

/// Form, frame, step, frame count and current frame of the closed-form
/// property. A step whose loop would exceed [`MAX_LOOP`] is doubled until
/// it does not, so (nearly) no case is rejected (`ci-nightly-props`).
fn schedule_inputs() -> impl Strategy<Value = (Form, i32, i32, i32, i32)> {
    (
        form(),
        prop_oneof![0i32..1_000_000, Just(i32::MAX - 600)],
        prop_oneof![1 => Just(0i32), 1 => -300i32..0, 8 => 1i32..2000],
        prop_oneof![-1000i32..80_000, Just(0)],
        0i32..80_000,
    )
        .prop_map(|(form, f, mut s, frame_count, cur)| {
            while s > 0
                && s <= i32::MAX / 2
                && loop_length(form, f, s, frame_count, cur) >= MAX_LOOP
            {
                s *= 2;
            }
            (form, f, s, frame_count, cur)
        })
}

fn event_bytes() -> impl Strategy<Value = [u8; ANIM_EVENTS]> {
    prop::collection::vec(prop_oneof![6 => Just(0u8), 1 => 1u8..=6], ANIM_EVENTS)
        .prop_map(|v| v.try_into().unwrap())
}

proptest! {
    #![proptest_config(config(1024))]

    #[test]
    fn anim_schedule_matches_the_closed_form(
        (form, f, s, frame_count, cur) in schedule_inputs(),
        byte_0f in any::<u8>(),
        events in event_bytes(),
    ) {
        // Keep the implementation's loop short (2^31 / s iterations are
        // possible for a wrapped start).
        prop_assume!(loop_length(form, f, s, frame_count, cur) < MAX_LOOP);
        let got = schedule(form, f, s, frame_count, cur, Events::Record { byte_0f, events: &events });
        let want = reference(form, f, s, frame_count, cur, byte_0f, &events);
        prop_assert_eq!(&got, &want);
        if let Ok(Some(sched)) = got {
            // §4.2 "So": events in frame order, the end last and strictly
            // after every action event.
            let ex: Vec<i32> = sched.events.iter().map(|e| e.expire.wrapping_sub(f)).collect();
            prop_assert!(ex.windows(2).all(|w| w[0] <= w[1]));
            let last = sched.events.last().unwrap();
            prop_assert_eq!(last.event, 1);
            prop_assert!(sched.events[..sched.events.len() - 1].iter().all(|e| e.event == 0
                && e.expire.wrapping_sub(f) < last.expire.wrapping_sub(f)));
            if let Form::Main { bonus } = form {
                if s > 0 {
                    let need = i64::from(frame_count) - 256 * i64::from(bonus);
                    let steps = (need + i64::from(s) - 1).div_euclid(i64::from(s));
                    prop_assert_eq!(i64::from(last.expire.wrapping_sub(f)), steps.max(2));
                }
            }
        }
    }

    // Frame counts near 2^31 − 1 exercise the scheduler's overflow guard
    // when the final step would wrap before reaching F. Require an answer,
    // never a hang (inputs keep the finite loops short).
    #[test]
    fn anim_schedule_near_the_i32_limit(
        d in 0i32..4,
        r in 0i32..600,
        s in 1i32..1000,
        bonus in 0i32..4,
    ) {
        let frame_count = i32::MAX - r;
        let p = (frame_count >> 8) - d;
        let events = [1u8; ANIM_EVENTS];
        for form in [Form::StartFrame(p), Form::Main { bonus: p - bonus }] {
            let got = schedule(form, 0, s, frame_count, 0, Events::Record { byte_0f: 0, events: &events });
            let want = reference(form, 0, s, frame_count, 0, 0, &events);
            prop_assert_eq!(got, want);
        }
    }
}

// ---- §3 lifecycle, §4.1 / §4.4 modes ------------------------------------

struct World {
    game: Game,
    units: Units,
    stats: StatLists,
    data: UnitData,
}

impl World {
    fn new() -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        game.lists.ensure_act(2).unwrap();
        let data = UnitData {
            monsters: vec![
                MonsterInfo {
                    enabled: true,
                    ..Default::default()
                },
                MonsterInfo {
                    enabled: false,
                    ..Default::default()
                },
                MonsterInfo {
                    enabled: true,
                    aidel: [3, 4, 5],
                    moves: 0,
                    mode_chart: false,
                },
            ],
            ..Default::default()
        };
        Self {
            game,
            units: Units::new(),
            stats: StatLists::new(Arc::new(StatData::default())),
            data,
        }
    }

    fn sim(&mut self) -> Sim<'_> {
        Sim {
            game: &mut self.game,
            units: &mut self.units,
            stats: &mut self.stats,
            data: &self.data,
        }
    }
}

#[derive(Default)]
struct Hooks {
    record: Option<AnimRecord>,
    rate: i16,
}

impl StatHost for Hooks {}
impl UnitHooks for Hooks {
    fn anim_record(&mut self, _sim: &Sim<'_>, _unit: UnitId) -> Option<AnimRecord> {
        self.record
    }
    fn anim_rate(&mut self, _sim: &mut Sim<'_>, _unit: UnitId) -> i16 {
        self.rate
    }
    fn has_path(&mut self, _sim: &Sim<'_>, _unit: UnitId) -> bool {
        true
    }
}
impl LifecycleHooks for Hooks {}

#[derive(Clone, Debug)]
enum Op {
    Alloc {
        ty: UnitType,
        class: u32,
        room: Option<usize>,
        add: bool,
        fixed: Option<Option<usize>>,
        mode: u32,
    },
    Remove(usize),
    RemoveAgain,
    NewRoom(bool),
    UnknownRoom(UnitType),
    SetMode(usize, u32),
    PlayerStart(usize, u32),
    Tick,
}

fn op() -> impl Strategy<Value = Op> {
    let ty = prop::sample::select(UnitType::ALL.to_vec());
    prop_oneof![
        8 => (ty.clone(), prop_oneof![4 => 0u32..3, 1 => 3u32..12], prop::option::of(any::<usize>()),
              prop::bool::weighted(0.95), prop::option::weighted(0.2, prop::option::of(any::<usize>())), 0u32..20)
            .prop_map(|(ty, class, room, add, fixed, mode)| Op::Alloc { ty, class, room, add, fixed, mode }),
        3 => any::<usize>().prop_map(Op::Remove),
        1 => Just(Op::RemoveAgain),
        2 => any::<bool>().prop_map(Op::NewRoom),
        1 => ty.prop_map(Op::UnknownRoom),
        2 => (any::<usize>(), 0u32..20).prop_map(|(u, m)| Op::SetMode(u, m)),
        2 => (any::<usize>(), 0u32..22).prop_map(|(u, m)| Op::PlayerStart(u, m)),
        2 => Just(Op::Tick),
    ]
}

#[derive(Clone, Debug)]
struct MUnit {
    id: UnitId,
    ty: UnitType,
    guid: u32,
    room: Option<RoomId>,
}

/// Everything a rejected request must leave as it was.
fn fingerprint(w: &World, seed: Seed) -> String {
    format!(
        "{seed:?} {:?} {:?} {:?}",
        UnitType::ALL.map(|t| w.game.lists.guids.get(t)),
        UnitType::ALL.map(|t| w.game.lists.units_of_type(t)),
        TimerClass::RUN_ORDER.map(|c| w.game.timers.every_tick(c)),
    )
}

fn run_ops(ops: Vec<Op>) {
    let mut w = World::new();
    let mut hooks = Hooks {
        record: Some(AnimRecord {
            frames: 12,
            byte_0f: 0,
            events: [0; ANIM_EVENTS],
        }),
        rate: 256,
    };
    hooks.record.as_mut().unwrap().events[5] = 1;
    let mut game_seed = Seed::new(1234, 666);
    let mut units: Vec<MUnit> = Vec::new();
    let mut rooms: Vec<RoomId> = Vec::new();
    let mut removed: Option<UnitId> = None;
    for op in ops {
        match op {
            Op::NewRoom(act2) => {
                let r = w.game.lists.create_room(if act2 { 2 } else { 0 }).unwrap();
                w.game.lists.activate_room(r).unwrap();
                rooms.push(r);
            }
            Op::Alloc {
                ty,
                class,
                room,
                add,
                fixed,
                mode,
            } => {
                let room = room.and_then(|s| (!rooms.is_empty()).then(|| rooms[s % rooms.len()]));
                let fixed_guid = fixed.map(|d| {
                    let same: Vec<u32> = units
                        .iter()
                        .filter(|u| u.ty == ty)
                        .map(|u| u.guid)
                        .collect();
                    d.filter(|_| !same.is_empty())
                        .map_or(1_000_000, |s| same[s % same.len()])
                });
                let req = AllocRequest {
                    ty,
                    class,
                    room,
                    add,
                    fixed_guid,
                    mode,
                    allied: false,
                };
                let before = fingerprint(&w, game_seed);
                let mut seed = game_seed;
                let counter = w.game.lists.guids.get(ty);
                let got = allocate(&mut w.sim(), &mut hooks, &mut seed, &req);
                // §3.1 r1: class checks, nothing allocated, no draw.
                let class_ok = match ty {
                    UnitType::Monster => w.data.monster(class).is_some_and(|m| m.enabled),
                    UnitType::Player => class < 7,
                    _ => true,
                };
                if !class_ok {
                    assert_eq!(got, Ok(None));
                    assert_eq!(fingerprint(&w, seed), before);
                    continue;
                }
                if !add {
                    // §3.1 r9: the unit is returned, in no list, with no
                    // duplicate-GUID check; the draws stay.
                    let id = got.expect("allocate").expect("returned");
                    let rec = w.units.get(id).expect("record").clone();
                    assert_eq!((rec.ty, rec.class), (ty, class));
                    assert_ne!(w.game.lists.find_unit(ty, rec.guid), Some(id));
                    // Free it again; the model of linked units is unchanged.
                    remove(&mut w.sim(), &mut hooks, id).expect("remove");
                    game_seed = seed;
                    continue;
                }
                let guid = match (ty, fixed_guid) {
                    (UnitType::Monster, Some(g)) => g,
                    _ => {
                        let n = counter.wrapping_add(1);
                        if n == u32::MAX {
                            1
                        } else {
                            n
                        }
                    }
                };
                if units.iter().any(|u| u.ty == ty && u.guid == guid) {
                    // §2.1 fatal duplicate: refused with no trace.
                    assert_eq!(
                        got,
                        Err(UnitError::Game(GameError::List(ListError::DuplicateGuid {
                            ty,
                            guid
                        })))
                    );
                    assert_eq!(
                        fingerprint(&w, seed),
                        before,
                        "rejected allocation left a trace"
                    );
                    continue;
                }
                let id = got.expect("allocate").expect("accepted");
                // rng.md §5.3: players draw nothing; units one step;
                // items a second one for the item seed.
                let mut want_seed = game_seed;
                let rec = w.units.get(id).expect("record").clone();
                if ty != UnitType::Player {
                    let lo = want_seed.step();
                    assert_eq!((rec.seed, rec.init_seed), (Seed::init_low(lo), lo));
                } else {
                    assert_eq!((rec.seed, rec.init_seed), (Seed::init(), 0));
                }
                if ty == UnitType::Item {
                    let lo = want_seed.step();
                    assert_eq!(rec.item_seed, Some((Seed::init_low(lo), lo)));
                } else {
                    assert_eq!(rec.item_seed, None);
                }
                assert_eq!(seed, want_seed);
                game_seed = seed;
                assert_eq!((rec.ty, rec.class, rec.guid), (ty, class, guid));
                let want_mode = if matches!(ty, UnitType::Player | UnitType::Tile) {
                    0
                } else {
                    mode
                };
                assert_eq!(rec.mode, want_mode);
                // Flag 0x2 (skill target): tiles and items (`units.md` row 4).
                let want_flags = flags::SEED_SET
                    | if matches!(ty, UnitType::Tile | UnitType::Item) {
                        flags::TILE
                    } else {
                        0
                    };
                assert_eq!(rec.flags, want_flags);
                assert_eq!(rec.flags2, flags2::SERVER);
                assert_eq!(
                    rec.act,
                    room.map_or(0, |r| w.game.lists.room(r).unwrap().act)
                );
                let has_list = matches!(
                    ty,
                    UnitType::Player | UnitType::Monster | UnitType::Item | UnitType::Missile
                );
                assert_eq!(rec.stats.is_some(), has_list);
                assert_eq!(w.stats.unit_list(id), rec.stats);
                // Missile setup (§6.3, tick.md §5.3): one every-tick type 0.
                let timers = w.game.timers.unit_timers(id);
                if ty == UnitType::Missile {
                    assert_eq!(timers.len(), 1);
                    assert_eq!(w.game.timers.event(timers[0]), Some((0, 0, 0)));
                    assert_eq!(w.game.timers.flags(timers[0]), Some(tflags::EVERY_TICK));
                } else {
                    assert!(timers.is_empty());
                }
                assert_eq!(w.game.lists.find_unit(ty, guid), Some(id));
                if let Some(r) = room {
                    assert_eq!(w.game.lists.room_units(r).first(), Some(&id));
                }
                units.push(MUnit { id, ty, guid, room });
            }
            Op::UnknownRoom(ty) => {
                let before = fingerprint(&w, game_seed);
                let mut seed = game_seed;
                let req = AllocRequest {
                    ty,
                    class: 0,
                    room: Some(RoomId(1 << 30)),
                    add: true,
                    fixed_guid: None,
                    mode: 0,
                    allied: false,
                };
                let got = allocate(&mut w.sim(), &mut hooks, &mut seed, &req);
                assert!(got.is_err(), "unknown room accepted: {got:?}");
                assert_eq!(
                    fingerprint(&w, seed),
                    before,
                    "rejected allocation left a trace"
                );
            }
            Op::Remove(s) => {
                if units.is_empty() {
                    continue;
                }
                let u = units.remove(s % units.len());
                remove(&mut w.sim(), &mut hooks, u.id).expect("live unit");
                assert!(w.units.get(u.id).is_none());
                assert!(w.game.lists.unit(u.id).is_none());
                assert!(w.game.timers.unit_timers(u.id).is_empty());
                assert_eq!(w.stats.unit_list(u.id), None);
                if let Some(r) = u.room {
                    assert!(!w.game.lists.room_units(r).contains(&u.id));
                }
                removed = Some(u.id);
            }
            Op::RemoveAgain => {
                let Some(id) = removed else { continue };
                if units.iter().any(|u| u.id == id) {
                    continue; // slot reused by a live unit
                }
                let before = fingerprint(&w, game_seed);
                assert!(remove(&mut w.sim(), &mut hooks, id).is_err());
                assert_eq!(fingerprint(&w, game_seed), before);
            }
            Op::SetMode(s, mode) => {
                if units.is_empty() {
                    continue;
                }
                let u = units[s % units.len()].clone();
                let old = w.units.get(u.id).unwrap().clone();
                let timers_before = w.game.timers.unit_timers(u.id);
                w.game.lists.clear_update_queue_all();
                set_mode(&mut w.sim(), &mut hooks, u.id, mode).expect("live unit");
                let new = w.units.get(u.id).unwrap();
                let queued = w.game.lists.unit(u.id).unwrap().is_queued();
                // §4.1: tile nothing; monster staying in mode 1 nothing;
                // else mode written, flag 1, queued (when in a room, §6.2).
                if u.ty == UnitType::Tile
                    || (u.ty == UnitType::Monster && old.mode == 1 && mode == 1)
                {
                    assert_eq!(new, &old);
                    assert!(!queued);
                } else {
                    assert_eq!(new.mode, mode);
                    assert_eq!(new.flags, old.flags | flags::CHANGED);
                    assert_eq!(queued, u.room.is_some());
                }
                // Setting a mode schedules nothing (§4.1).
                assert_eq!(w.game.timers.unit_timers(u.id), timers_before);
            }
            Op::PlayerStart(s, mode) => {
                let players: Vec<UnitId> = units
                    .iter()
                    .filter(|u| u.ty == UnitType::Player)
                    .map(|u| u.id)
                    .collect();
                if players.is_empty() {
                    continue;
                }
                let p = players[s % players.len()];
                let before = w.game.timers.unit_timers(p);
                let got = player_start(&mut w.sim(), &mut hooks, p, mode);
                if mode >= 20 {
                    assert_eq!(
                        got,
                        Err(UnitError::BadMode {
                            ty: UnitType::Player,
                            mode
                        })
                    );
                    continue;
                }
                assert_eq!(got, Ok(true));
                let timers: Vec<(u8, i32)> = w
                    .game
                    .timers
                    .unit_timers(p)
                    .into_iter()
                    .map(|t| {
                        (
                            w.game.timers.event(t).unwrap().0,
                            w.game.timers.expire(t).unwrap(),
                        )
                    })
                    .collect();
                let f = w.game.frame;
                match mode {
                    // §4.4: one every-tick type 0, no type 1.
                    2 | 3 | 6 | 19 => assert_eq!(timers, [(0, -1)]),
                    // Neutral: cancel 0/1, mode only.
                    1 | 5 => assert!(timers.iter().all(|&(e, _)| e > 1)),
                    // Corpse: mode only, no schedule.
                    17 => assert_eq!(w.game.timers.unit_timers(p), before),
                    // §4.2 main form with the hook's record (12 frames,
                    // speed 256, action byte at 5): newest first.
                    _ => assert_eq!(timers, [(1, f + 12), (0, f + 5)]),
                }
            }
            Op::Tick => {
                w.game.frame += 1;
            }
        }
    }
}

trait ClearAll {
    fn clear_update_queue_all(&mut self);
}

impl ClearAll for d2_sim::units::UnitLists {
    fn clear_update_queue_all(&mut self) {
        for act in 0..5 {
            for r in self.active_rooms(act) {
                self.clear_update_queue(r).unwrap();
            }
        }
    }
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn lifecycle_and_modes(ops in prop::collection::vec(op(), 0..80)) {
        run_ops(ops);
    }
}
