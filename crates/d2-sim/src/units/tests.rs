//! Unit tests of the unit code: the animation-schedule synthetic vectors, the TSV
//! checks (with perturbations), dispatch, regeneration, modes
//! and allocation, on fake hooks.

use std::sync::Arc;

use crate::game::Game;
use crate::rng::Seed;
use crate::stats::states::state;
use crate::stats::{key, StatHost, StatLists};
use crate::tick::events::{event, monster_has_handler, player_has_handler};
use crate::tick::run_timer_events;
use crate::tick::timer::TimerClass;

use super::anim::{schedule, Events, Form, Scheduled};
use super::dispatch::{handler, replenish_delay, replenish_start_delay, UnitSystem, HANDLERS};
use super::hooks::{MonsterInfo, Sim, UnitData, UnitHooks};
use super::lifecycle::{allocate, remove, AllocRequest, LifecycleHooks};
use super::modes::{self, player_mode, UnitError};
use super::record::{AnimRecord, ANIM_EVENTS};
use super::{UnitId, UnitType};

// ---- synthetic vectors ---------------------------------------------------

pub(super) fn bytes(pairs: &[(usize, u8)]) -> [u8; ANIM_EVENTS] {
    let mut e = [0u8; ANIM_EVENTS];
    for &(i, v) in pairs {
        e[i] = v;
    }
    e
}

fn sched(
    form: Form,
    s: i32,
    frames: i32,
    cur: i32,
    ev: &[(usize, u8)],
) -> Vec<(u8, i32, u32, u32)> {
    let events = bytes(ev);
    schedule(
        form,
        100,
        s,
        frames * 256,
        cur,
        Events::Record {
            byte_0f: 0,
            events: &events,
        },
    )
    .expect("schedule")
    .map(|s| {
        s.events
            .iter()
            .map(|e: &Scheduled| (e.event, e.expire, e.a1, e.a2))
            .collect()
    })
    .unwrap_or_default()
}

const MAIN: Form = Form::Main { bonus: 0 };

#[test]
fn anim_vectors() {
    let six = [(6, 1)];
    assert_eq!(
        sched(MAIN, 256, 12, 0, &six),
        [(0, 106, 1, 0), (1, 112, 0, 0)]
    );
    assert_eq!(
        sched(MAIN, 128, 12, 0, &six),
        [(0, 112, 1, 0), (1, 124, 0, 0)]
    );
    assert_eq!(
        sched(MAIN, 300, 12, 0, &[(3, 1), (4, 2), (6, 3), (7, 4)]),
        [
            (0, 103, 1, 0),
            (0, 104, 2, 1),
            (0, 106, 3, 0),
            (0, 106, 4, 2),
            (1, 111, 0, 0)
        ]
    );
    assert_eq!(
        sched(Form::Main { bonus: 5 }, 256, 16, 0, &[(2, 1), (9, 1)]),
        [(0, 104, 1, 0), (1, 111, 0, 0)]
    );
    assert_eq!(sched(MAIN, 256, 1, 0, &[]), [(1, 102, 0, 0)]);
    assert_eq!(sched(MAIN, 0, 12, 0, &six), [(1, 101, 0, 0)]);
    assert_eq!(
        sched(Form::StartFrame(3), 256, 12, 0, &six),
        [(0, 103, 1, 0), (1, 109, 0, 0)]
    );
    assert_eq!(
        sched(Form::Percent(50), 256, 12, 96 * 256, &six),
        [(0, 104, 1, 0), (1, 110, 0, 0)]
    );
    assert_eq!(
        sched(Form::Frames(2), 256, 12, 96 * 256, &six),
        [(0, 104, 1, 0), (1, 110, 0, 0)]
    );
    assert_eq!(sched(Form::Frames(0), 256, 12, 96 * 256, &six), []);
}

#[test]
fn anim_frame_and_cancel_flags() {
    let e = bytes(&[]);
    let ev = Events::Record {
        byte_0f: 0,
        events: &e,
    };
    let s = schedule(MAIN, 100, 256, 3072, 0, ev).unwrap().unwrap();
    assert_eq!((s.cancels, s.frame), (false, Some(100 * 256)));
    let s = schedule(Form::StartFrame(3), 100, 256, 3072, 0, ev)
        .unwrap()
        .unwrap();
    assert_eq!((s.cancels, s.frame), (true, Some(97 * 256)));
    assert!(schedule(Form::Frames(0), 100, 256, 3072, 0, ev)
        .unwrap()
        .is_none());
    let s = schedule(MAIN, 100, 0, 3072, 0, ev).unwrap().unwrap();
    assert_eq!(s.frame, None);
}

#[test]
fn anim_edge_cases() {
    // Variants start at index −1: AnimData +0x0F (edge case 1).
    let e = bytes(&[]);
    let ev = Events::Record {
        byte_0f: 2,
        events: &e,
    };
    let s = schedule(Form::StartFrame(0), 100, 256, 3 * 256, 0, ev)
        .unwrap()
        .unwrap();
    assert_eq!(
        s.events[0],
        Scheduled {
            event: 0,
            expire: 101,
            a1: 2,
            a2: 0
        }
    );
    // Main form: no events past index 143 (edge case 2).
    let e = bytes(&[(143, 1)]);
    let ev = Events::Record {
        byte_0f: 0,
        events: &e,
    };
    let s = schedule(MAIN, 0, 256, 200 * 256, 0, ev).unwrap().unwrap();
    assert_eq!(s.events.len(), 2);
    assert_eq!(s.events[1].expire, 200);
    // Variants have no bound: past the record is an error, not a guess.
    assert!(schedule(Form::StartFrame(0), 0, 256, 200 * 256, 0, ev).is_err());
    // Negative speed never ends in the base game (edge case 3).
    assert!(schedule(MAIN, 0, -1, 256, 0, ev).is_err());
}

// ---- TSV checks -----------------------------------------------------------

const HANDLERS_TSV: &str = include_str!("../../../../tables/sim/unit-handlers.tsv");
pub(super) const EVENTS_TSV: &str = include_str!("../../../../tables/sim/unit-events.tsv");

fn class_of(name: &str) -> TimerClass {
    match name {
        "player" => TimerClass::Player,
        "monster" => TimerClass::Monster,
        "object" => TimerClass::Object,
        "item" => TimerClass::Item,
        "missile" => TimerClass::Missile,
        _ => panic!("unknown class {name}"),
    }
}

fn hex(s: &str) -> u32 {
    if s == "null" {
        return 0;
    }
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex")
}

/// (class, type) pairs whose TSV handler differs from [`HANDLERS`].
fn handler_mismatches(tsv: &str) -> Vec<(TimerClass, u8)> {
    let mut lines = tsv.lines();
    assert_eq!(
        lines.next(),
        Some("class\ttype\thandler\tdispatcher\ttable")
    );
    let mut seen = 0;
    let mut bad = Vec::new();
    for l in lines.filter(|l| !l.is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        assert_eq!(c.len(), 5, "{l}");
        let (class, ty) = (class_of(c[0]), c[1].parse::<u8>().expect("type"));
        if handler(class, ty) != hex(c[2]) {
            bad.push((class, ty));
        }
        seen += 1;
    }
    assert_eq!(seen, 75);
    bad
}

#[test]
fn handlers_match_tsv() {
    assert_eq!(handler_mismatches(HANDLERS_TSV), []);
}

#[test]
fn handler_check_catches_perturbation() {
    let changed = HANDLERS_TSV.replacen("monster\t7\t0x005A4370", "monster\t7\tnull", 1);
    assert_ne!(changed, HANDLERS_TSV);
    assert_eq!(handler_mismatches(&changed), [(TimerClass::Monster, 7)]);
}

#[test]
fn handlers_agree_with_tick_tables() {
    for ev in 0..15u8 {
        assert_eq!(handler(TimerClass::Player, ev) != 0, player_has_handler(ev));
        assert_eq!(
            handler(TimerClass::Monster, ev) != 0,
            monster_has_handler(ev)
        );
    }
    assert_eq!(HANDLERS.len(), 5);
}

/// Scheduler sites D2RUST reproduces, with the TSV columns its code
/// follows: site, api, type, classes, expire, a1, a2, callback.
const IMPLEMENTED_SITES: &[[&str; 8]] = &[
    ["0x00534b02", "timed", "3", "player", "f+1", "0", "0", "0"],
    [
        "0x00534b1d",
        "timed",
        "11",
        "player",
        "f+250",
        "0",
        "0",
        "0",
    ],
    [
        "0x00553a00",
        "timed",
        "1",
        "player,monster",
        "f+1",
        "reg",
        "reg",
        "0",
    ],
    [
        "0x00553a8f",
        "timed",
        "0",
        "player,monster",
        "anim",
        "reg",
        "reg",
        "0",
    ],
    [
        "0x00553aa5",
        "timed",
        "0",
        "player,monster",
        "anim",
        "reg",
        "0",
        "0",
    ],
    [
        "0x00553ae2",
        "timed",
        "1",
        "player,monster",
        "anim",
        "0",
        "0",
        "0",
    ],
    [
        "0x00553c1e",
        "timed",
        "0",
        "player,monster",
        "anim",
        "reg",
        "0",
        "0",
    ],
    [
        "0x00553c55",
        "timed",
        "1",
        "player,monster",
        "anim",
        "0",
        "0",
        "0",
    ],
    [
        "0x00553d7e",
        "timed",
        "0",
        "player,monster",
        "anim",
        "reg",
        "0",
        "0",
    ],
    [
        "0x00553db1",
        "timed",
        "1",
        "player,monster",
        "anim",
        "0",
        "0",
        "0",
    ],
    [
        "0x00553eb2",
        "timed",
        "0",
        "player,monster",
        "anim",
        "reg",
        "0",
        "0",
    ],
    [
        "0x00553ee5",
        "timed",
        "1",
        "player,monster",
        "anim",
        "0",
        "0",
        "0",
    ],
    [
        "0x00553f19",
        "every",
        "0",
        "player,monster",
        "-1",
        "0",
        "0",
        "0",
    ],
    [
        "0x0058082c",
        "timed",
        "3",
        "player",
        "f+1",
        "reg",
        "reg",
        "0",
    ],
    ["0x00580bc8", "timed", "6", "player", "hover", "0", "0", "0"],
    ["0x00580c08", "timed", "11", "player", "f+30", "0", "0", "0"],
    ["0x0059f8d3", "every", "0", "missile", "-1", "0", "0", "0"],
    // Umod callbacks and the think restart
    // and the population class-438 schedule.
    ["0x0054ea84", "timed", "7", "monster", "calc", "0", "0", "0"],
    ["0x005737c0", "timed", "2", "monster", "f+2", "0", "0", "0"],
    ["0x00573825", "timed", "2", "monster", "f+2", "0", "0", "0"],
    ["0x005a1359", "timed", "7", "monster", "f+75", "0", "0", "0"],
    ["0x005a2613", "timed", "7", "monster", "f+4", "0", "0", "0"],
    ["0x005a2ed0", "timed", "7", "monster", "f+2", "0", "0", "0"],
    ["0x005a37f3", "timed", "7", "monster", "f+2", "0", "0", "0"],
    ["0x005a3829", "timed", "7", "monster", "f+4", "0", "0", "0"],
    ["0x005a38ff", "timed", "7", "monster", "calc", "0", "0", "0"],
    ["0x005a39dc", "timed", "7", "monster", "calc", "0", "0", "0"],
    ["0x005a3a6c", "timed", "2", "monster", "f+51", "0", "0", "0"],
    ["0x005a3adb", "timed", "7", "monster", "f+3", "0", "0", "0"],
    ["0x005a3eb4", "timed", "7", "monster", "f+4", "0", "0", "0"],
    ["0x005a3eda", "timed", "7", "monster", "f+4", "0", "0", "0"],
    ["0x005a425e", "timed", "7", "monster", "f+75", "0", "0", "0"],
    ["0x005a697a", "timed", "3", "monster", "f+1", "0", "0", "0"],
    ["0x005a7423", "timed", "2", "monster", "f+45", "0", "0", "0"],
    [
        "0x005a747d",
        "timed",
        "2",
        "monster",
        "f+aidel",
        "0",
        "0",
        "0",
    ],
    [
        "0x005a7f58",
        "timed",
        "6",
        "monster",
        "hover",
        "0",
        "0",
        "0",
    ],
    [
        "0x00562d11",
        "timed",
        "3",
        "item",
        "f+max(2500/r+1,125)",
        "0",
        "0",
        "0",
    ],
    [
        "0x00558574",
        "timed",
        "3",
        "item",
        "f+2500/r+1",
        "reg",
        "reg",
        "0",
    ],
    [
        "0x00555046",
        "timed_cb",
        "14",
        "player",
        "f+50",
        "0",
        "0",
        "0x554570",
    ],
];

fn site_mismatches(tsv: &str) -> Vec<String> {
    let rows: Vec<Vec<&str>> = tsv
        .lines()
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 269);
    IMPLEMENTED_SITES
        .iter()
        .filter(|want| {
            let Some(r) = rows.iter().find(|r| r[0] == want[0]) else {
                return true;
            };
            [r[2], r[3], r[4], r[5], r[6], r[7], r[8]] != want[1..]
        })
        .map(|w| w[0].to_string())
        .collect()
}

#[test]
fn implemented_sites_match_tsv() {
    assert_eq!(site_mismatches(EVENTS_TSV), Vec::<String>::new());
}

#[test]
fn site_check_catches_perturbation() {
    let changed = EVENTS_TSV.replacen(
        "0x00580c08\t0x00580be0\ttimed\t11\tplayer\tf+30",
        "0x00580c08\t0x00580be0\ttimed\t11\tplayer\tf+31",
        1,
    );
    assert_ne!(changed, EVENTS_TSV);
    assert_eq!(site_mismatches(&changed), ["0x00580c08"]);
}

/// U1: every object and item schedule of the TSV has a handler.
#[test]
fn object_and_item_sites_have_handlers() {
    for l in EVENTS_TSV.lines().skip(1).filter(|l| !l.is_empty()) {
        let c: Vec<&str> = l.split('\t').collect();
        let ty: u8 = c[3].parse().expect("type");
        for class in c[4].split(',') {
            if matches!(class, "object" | "item") {
                assert_ne!(handler(class_of(class), ty), 0, "{l}");
            }
        }
    }
}

#[test]
fn replenish_delays() {
    assert_eq!(replenish_start_delay(100), 26);
    assert_eq!(replenish_delay(100), 125);
    assert_eq!(replenish_delay(5), 501);
}

// ---- fake hooks -------------------------------------------------------------------

#[derive(Default)]
struct Fake {
    log: Vec<String>,
    anim: Option<AnimRecord>,
    rate: i16,
    has_path: bool,
    town: bool,
    request_ok: bool,
    action_result: u32,
    fractions: Vec<i32>,
    /// What each per-kind init saw: (found by hash lookup, in the update
    /// queue), then "added" at step 8.
    inits: Vec<String>,
}

impl StatHost for Fake {}

impl UnitHooks for Fake {
    fn anim_record(&mut self, _: &Sim<'_>, _: UnitId) -> Option<AnimRecord> {
        self.anim
    }
    fn anim_rate(&mut self, _: &mut Sim<'_>, _: UnitId) -> i16 {
        self.rate
    }
    fn has_path(&mut self, _: &Sim<'_>, _: UnitId) -> bool {
        self.has_path
    }
    fn room_flag(&mut self, _: &Sim<'_>, _: UnitId) -> bool {
        self.town
    }
    fn player_request_check(&mut self, _: &mut Sim<'_>, _: UnitId, _: u32) -> bool {
        self.request_ok
    }
    fn player_action_frame(&mut self, _: &mut Sim<'_>, _: UnitId, a1: u32, a2: u32) -> u32 {
        self.log.push(format!("action {a1} {a2}"));
        self.action_result
    }
    fn player_corpse(&mut self, _: &mut Sim<'_>, _: UnitId) {
        self.log.push("corpse".into());
    }
    fn player_refresh(&mut self, sim: &mut Sim<'_>, _: UnitId) {
        self.log.push(format!("refresh {}", sim.game.frame));
    }
    fn monster_mode_function(&mut self, sim: &mut Sim<'_>, unit: UnitId, address: u32) -> bool {
        self.log.push(format!("mfn {address:#x}"));
        // A start function sets its mode (monster); this fake knows
        // only the attack start.
        if address == 0x005A75C0 {
            let _ = modes::set_mode(sim, self, unit, 4);
        }
        true
    }
    fn ai_think(&mut self, sim: &mut Sim<'_>, _: UnitId, _: u32, _: u32) {
        self.log.push(format!("ai {}", sim.game.frame));
    }
    fn monster_death(&mut self, _: &mut Sim<'_>, _: UnitId, killer: Option<UnitId>) {
        self.log.push(format!("death {killer:?}"));
    }
    fn send_life_fraction(&mut self, _: &mut Sim<'_>, _: UnitId, f: i32) {
        self.fractions.push(f);
    }
    fn object_event(&mut self, _: &mut Sim<'_>, _: UnitId, ev: u8) {
        self.log.push(format!("object {ev}"));
    }
    fn missile_do(&mut self, sim: &mut Sim<'_>, _: UnitId) {
        self.log.push(format!("missile {}", sim.game.frame));
    }
    fn free_hover(&mut self, _: &mut Sim<'_>, _: UnitId) {
        self.log.push("hover".into());
    }
    fn apply_item_aura(&mut self, _: &mut Sim<'_>, _: UnitId, a1: u32, skill: u32, l: i32) {
        self.log.push(format!("aura {a1} {skill} {l}"));
    }
}

impl LifecycleHooks for Fake {
    fn init_kind(&mut self, sim: &mut Sim<'_>, unit: UnitId, req: &AllocRequest) {
        let guid = sim.units.get(unit).map(|r| r.guid).unwrap_or(0);
        let found = sim.game.lists.find_unit(req.ty, guid);
        let room = sim.game.lists.unit(unit).and_then(|e| e.room());
        self.inits
            .push(format!("init found {found:?} room {room:?}"));
    }
    fn added(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let guid = sim.units.get(unit).map(|r| r.guid).unwrap_or(0);
        let found = sim.game.lists.find_unit(UnitType::Monster, guid) == Some(unit);
        self.inits.push(format!("added found {found}"));
    }
}

pub(super) fn data() -> UnitData {
    UnitData {
        monsters: vec![
            MonsterInfo {
                enabled: true,
                aidel: [0, 5, 7],
                moves: 1 << 4,
                mode_chart: false,
            },
            MonsterInfo {
                enabled: true,
                aidel: [9, 5, 7],
                moves: 0,
                mode_chart: false,
            },
            MonsterInfo::default(),
        ],
        ..UnitData::default()
    }
}

fn system() -> UnitSystem<Fake> {
    let hooks = Fake {
        request_ok: true,
        action_result: 1,
        ..Fake::default()
    };
    UnitSystem::new(crate::stats::tests::data(), data(), hooks)
}

fn spawn(game: &mut Game, sys: &mut UnitSystem<Fake>, ty: UnitType, class: u32) -> UnitId {
    let mut seed = Seed::init();
    sys.with(game, |sim, hooks| {
        allocate(
            sim,
            hooks,
            &mut seed,
            &AllocRequest {
                ty,
                class,
                room: None,
                add: true,
                fixed_guid: None,
                mode: 0,
                allied: false,
            },
        )
    })
    .expect("allocate")
    .expect("unit")
}

fn step(game: &mut Game, sys: &mut UnitSystem<Fake>) {
    game.frame += 1;
    run_timer_events(game, sys);
}

/// The unit's pending timers as (type, expire, a1, a2), sorted.
pub(super) fn pending(game: &Game, unit: UnitId) -> Vec<(u8, i32, u32, u32)> {
    let mut v: Vec<_> = game
        .timers
        .unit_timers(unit)
        .into_iter()
        .filter_map(|t| {
            let (e, a1, a2) = game.timers.event(t)?;
            Some((e, game.timers.expire(t)?, a1, a2))
        })
        .collect();
    v.sort();
    v
}

fn stats(sys: &UnitSystem<Fake>) -> &StatLists {
    &sys.stats
}

// ---- allocation ----------------------------------------------------------------------

/// The per-kind init runs before
/// `SUNIT_Add`, so a hash lookup of the unit's own GUID (CountessChest's
/// chest list) misses it; step 8 links it.
#[test]
fn per_kind_init_runs_before_the_unit_is_linked() {
    let mut game = Game::new();
    let mut sys = system();
    let mut seed = Seed::init();
    let req = AllocRequest {
        ty: UnitType::Monster,
        class: 0,
        room: None,
        add: true,
        fixed_guid: Some(5),
        mode: 1,
        allied: false,
    };
    let m = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        sys.hooks.inits,
        ["init found None room None", "added found true"]
    );
    assert_eq!(game.lists.find_unit(UnitType::Monster, 5), Some(m));
    // A duplicate fixed GUID fails before any init, undoing the draw.
    let before = seed;
    let r = sys.with(&mut game, |sim, hooks| {
        allocate(sim, hooks, &mut seed, &req)
    });
    assert!(r.is_err());
    assert_eq!(seed, before);
    assert_eq!(sys.hooks.inits.len(), 2);
}

#[test]
fn the_player_load_draws_the_recorded_unit_seed_before_the_town() {
    // Measured with `-seed 1234` (game seed {1234, 666} unstepped):
    // the four creation steps, then the
    // player's unit seed (seq 2349), then the town's first object.
    let mut game = Game::new();
    let mut sys = system();
    let mut seed = Seed::init_low(1234);
    let creation: Vec<u32> = (0..4).map(|_| seed.step()).collect();
    assert_eq!(creation, [2972047412, 1542758918, 1961566614, 2016663226]);
    let mut req = AllocRequest {
        ty: UnitType::Player,
        class: 0,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let p = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    // The allocation itself draws nothing for a player …
    assert_eq!(sys.units.get(p).unwrap().init_seed, 0);
    // … the load does.
    assert!(super::lifecycle::init_player_seed(
        &mut sys.units,
        p,
        &mut seed
    ));
    let rec = sys.units.get(p).unwrap();
    assert_eq!(
        (rec.init_seed, rec.seed),
        (4048349444, Seed::init_low(4048349444))
    );
    req.ty = UnitType::Monster;
    let m = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    assert_eq!(sys.units.get(m).unwrap().init_seed, 108806926);
    // An unknown unit: no draw.
    let before = seed;
    sys.units.remove(m);
    assert!(!super::lifecycle::init_player_seed(
        &mut sys.units,
        m,
        &mut seed
    ));
    assert_eq!(seed, before);
}

#[test]
fn allocation_seeds_and_rejections() {
    let mut game = Game::new();
    let mut sys = system();
    let mut seed = Seed::init();
    let mut req = AllocRequest {
        ty: UnitType::Monster,
        class: 2,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    // Disabled monster: nothing allocated, no draw.
    let r = sys.with(&mut game, |sim, hooks| {
        allocate(sim, hooks, &mut seed, &req)
    });
    assert_eq!(r, Ok(None));
    assert_eq!(seed, Seed::init());
    req.class = 0;
    req.fixed_guid = Some(77);
    let m = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    let mut expect = Seed::init();
    let lo = expect.step();
    let rec = sys.units.get(m).unwrap();
    assert_eq!(
        (rec.init_seed, rec.seed, rec.guid, rec.mode),
        (lo, Seed::init_low(lo), 77, 1)
    );
    assert_eq!(seed, expect);
    assert!(sys.stats.unit_list(m).is_some());
    // Items draw twice.
    req.ty = UnitType::Item;
    let i = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    let (a, b) = (expect.step(), expect.step());
    let rec = sys.units.get(i).unwrap();
    assert_eq!(
        (rec.init_seed, rec.item_seed),
        (a, Some((Seed::init_low(b), b)))
    );
    // Players: no draw; class ≥ 7 rejected.
    req.ty = UnitType::Player;
    req.class = 7;
    let r = sys.with(&mut game, |sim, hooks| {
        allocate(sim, hooks, &mut seed, &req)
    });
    assert_eq!(r, Ok(None));
    req.class = 0;
    let p = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    assert_eq!(seed, expect);
    assert_eq!(sys.units.get(p).unwrap().node_index, 11);
    // Flags bit 1 clear: the unit is returned after step 7 in no
    // list (no hash entry, no duplicate-GUID check), seeds and GUID drawn.
    req.add = false;
    req.ty = UnitType::Monster;
    req.fixed_guid = Some(77);
    let before = seed;
    let u = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    assert_ne!(seed, before);
    assert_eq!(sys.units.get(u).unwrap().guid, 77);
    assert_ne!(game.lists.find_unit(UnitType::Monster, 77), Some(u));
    assert_eq!(game.lists.find_unit(UnitType::Monster, 77), Some(m));
    assert!(sys.stats.unit_list(u).is_some());
    sys.with(&mut game, |sim, hooks| remove(sim, hooks, u))
        .unwrap();
    assert!(sys.units.get(u).is_none());
    assert_eq!(game.lists.find_unit(UnitType::Monster, 77), Some(m));
}

#[test]
fn missile_setup_and_removal() {
    let mut game = Game::new();
    let mut sys = system();
    let m = spawn(&mut game, &mut sys, UnitType::Missile, 0);
    assert_eq!(pending(&game, m), [(0, -1, 0, 0)]);
    step(&mut game, &mut sys);
    step(&mut game, &mut sys);
    assert_eq!(sys.hooks.log, ["missile 1", "missile 2"]);
    sys.with(&mut game, |sim, hooks| remove(sim, hooks, m))
        .unwrap();
    assert!(pending(&game, m).is_empty());
    assert!(sys.units.get(m).is_none());
    assert!(sys.stats.unit_list(m).is_none());
}

// ---- players --------------------------------------------------------------------------

fn player(game: &mut Game, sys: &mut UnitSystem<Fake>) -> UnitId {
    let p = spawn(game, sys, UnitType::Player, 0);
    sys.with(game, |sim, hooks| {
        let l = sim.stats.unit_list(p).unwrap();
        for (s, v) in [(0, 30), (12, 10), (3, 25), (7, 12800), (6, 12800)] {
            sim.stats.set(hooks, l, s, v, 0, None);
        }
    });
    p
}

#[test]
fn join_schedules_regen_and_refresh() {
    let mut game = Game::new();
    game.frame = 10;
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.with(&mut game, |sim, hooks| modes::player_join(sim, hooks, p))
        .unwrap();
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::NU);
    assert_eq!(pending(&game, p), [(3, 11, 0, 0), (11, 260, 0, 0)]);
    for _ in 0..250 {
        step(&mut game, &mut sys);
    }
    assert_eq!(sys.hooks.log, ["refresh 260"]);
    // Regen every frame, refresh every 30.
    assert_eq!(pending(&game, p), [(3, 261, 0, 0), (11, 290, 0, 0)]);
    assert!(sys.errors.is_empty());
}

#[test]
fn join_in_town_is_town_neutral() {
    let mut game = Game::new();
    let mut sys = system();
    sys.hooks.town = true;
    let p = player(&mut game, &mut sys);
    sys.with(&mut game, |sim, hooks| modes::player_join(sim, hooks, p))
        .unwrap();
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::TN);
}

/// test vector step 5: life regeneration.
#[test]
fn player_life_regen_vector() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().mode = player_mode::NU;
    sys.with(&mut game, |sim, hooks| {
        sim.stats.unit_set(hooks, p, 74, 100, 0);
        sim.stats.unit_set(hooks, p, 6, 10000, 0);
        super::dispatch::player_regen(sim, hooks, p, 0, 0)
    })
    .unwrap();
    assert_eq!(stats(&sys).unit_total(p, 6, 0), 10100);
    // f = (39 << 7) / 50 = 99, sent once (stat 352 was 0).
    assert_eq!(sys.hooks.fractions, [99]);
    assert_eq!(stats(&sys).unit_total(p, 352, 0), 99);
    assert_eq!(pending(&game, p), [(3, 1, 0, 0)]);
}

#[test]
fn player_life_regen_caps_and_frees_healthpot() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().mode = player_mode::NU;
    sys.with(&mut game, |sim, hooks| {
        let pot = sim.stats.alloc(0, 0, 0, 1);
        sim.stats.set_state(pot, state::HEALTHPOT);
        sim.stats.set(hooks, pot, 74, 500, 0, None);
        sim.stats.attach(hooks, p, pot, true);
        sim.stats.unit_set(hooks, p, 6, 12700, 0);
        super::dispatch::player_regen(sim, hooks, p, 0, 0)
    })
    .unwrap();
    assert_eq!(stats(&sys).unit_total(p, 6, 0), 12800);
    assert_eq!(stats(&sys).unit_total(p, 74, 0), 0);
    let r = stats(&sys).unit_list(p).unwrap();
    assert!(stats(&sys).active_chain(r).is_empty());
}

#[test]
fn dead_player_only_reschedules() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().mode = player_mode::DT;
    sys.with(&mut game, |sim, hooks| {
        sim.stats.unit_set(hooks, p, 74, 100, 0);
        sim.stats.unit_set(hooks, p, 6, 10000, 0);
        super::dispatch::player_regen(sim, hooks, p, 4, 5)
    })
    .unwrap();
    assert_eq!(stats(&sys).unit_total(p, 6, 0), 10000);
    assert_eq!(pending(&game, p), [(3, 1, 4, 5)]);
}

#[test]
fn stamina_regen_by_mode() {
    // (mode, bonus 28, max 11, current 10, expected 10).
    for (mode, bonus, max, s, expect) in [
        (1, 0, 2560, 0, 10),
        (1, 0, 2560, 2555, 2560),
        (1, 0, 2560, 2560, 2560),
        (5, 50, 2560, 0, 15),
        (2, 0, 2560, 0, 0),
        (2, 0, 2560, 256, 256 + 5),
        (6, 0, 2560, 0, 5),
        (7, 0, 2560, 0, 0),
        (7, 1000, 2560, 0, 10 + 100),
    ] {
        let mut game = Game::new();
        let mut sys = system();
        let p = player(&mut game, &mut sys);
        sys.units.get_mut(p).unwrap().mode = mode;
        sys.with(&mut game, |sim, hooks| {
            sim.stats.unit_set(hooks, p, 11, max, 0);
            sim.stats.unit_set(hooks, p, 10, s, 0);
            sim.stats.unit_set(hooks, p, 28, bonus, 0);
            super::dispatch::player_regen(sim, hooks, p, 0, 0)
        })
        .unwrap();
        assert_eq!(
            stats(&sys).unit_total(p, 10, 0),
            expect,
            "mode {mode} bonus {bonus} current {s}"
        );
    }
}

#[test]
fn mana_regen() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().mode = player_mode::NU;
    sys.with(&mut game, |sim, hooks| {
        sim.stats.unit_set(hooks, p, 9, 750_000, 0);
        sim.stats.unit_set(hooks, p, 8, 1000, 0);
        sim.stats.unit_set(hooks, p, 27, 50, 0);
        sim.stats.unit_set(hooks, p, 26, 3, 0);
        super::dispatch::player_regen(sim, hooks, p, 0, 0)
    })
    .unwrap();
    // ManaRegen 0 → q = 7500: 100, +50 % = 150, +3.
    assert_eq!(stats(&sys).unit_total(p, 8, 0), 1153);
    // nomanaregen: only stat 26; capped at max.
    sys.with(&mut game, |sim, hooks| {
        sim.stats.toggle_state(p, state::NOMANAREGEN, true);
        sim.stats.unit_set(hooks, p, 8, 749_999, 0);
        super::dispatch::player_regen(sim, hooks, p, 0, 0)
    })
    .unwrap();
    assert_eq!(stats(&sys).unit_total(p, 8, 0), 750_000);
}

#[test]
fn player_mode_starts() {
    let mut game = Game::new();
    game.frame = 100;
    let mut sys = system();
    sys.hooks.anim = Some(AnimRecord {
        frames: 12,
        byte_0f: 0,
        events: bytes(&[(6, 1)]),
    });
    sys.hooks.rate = 256;
    sys.hooks.has_path = true;
    let p = player(&mut game, &mut sys);
    sys.with(&mut game, |sim, hooks| {
        modes::player_start(sim, hooks, p, player_mode::A1)
    })
    .unwrap();
    assert_eq!(pending(&game, p), [(0, 106, 1, 0), (1, 112, 0, 0)]);
    let rec = sys.units.get(p).unwrap();
    assert_eq!(
        (rec.mode, rec.anim.frame, rec.anim.frame_count),
        (7, 100 * 256, 12 * 256)
    );
    // Running: RN without stamina walks; movement is every tick.
    sys.with(&mut game, |sim, hooks| {
        modes::player_start(sim, hooks, p, player_mode::RN)
    })
    .unwrap();
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::WL);
    assert_eq!(pending(&game, p), [(0, -1, 0, 0)]);
    // A rejected request changes nothing.
    sys.hooks.request_ok = false;
    let ok = sys
        .with(&mut game, |sim, hooks| {
            modes::player_start(sim, hooks, p, player_mode::A1)
        })
        .unwrap();
    assert!(!ok);
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::WL);
}

#[test]
fn player_animation_runs_to_neutral() {
    let mut game = Game::new();
    game.frame = 100;
    let mut sys = system();
    sys.hooks.anim = Some(AnimRecord {
        frames: 12,
        byte_0f: 0,
        events: bytes(&[(6, 1)]),
    });
    sys.hooks.rate = 256;
    sys.hooks.has_path = true;
    let p = player(&mut game, &mut sys);
    sys.with(&mut game, |sim, hooks| {
        modes::player_start(sim, hooks, p, player_mode::A1)
    })
    .unwrap();
    for _ in 0..12 {
        step(&mut game, &mut sys);
    }
    assert_eq!(sys.hooks.log, ["action 1 0"]);
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::NU);
    // Death runs to the corpse mode.
    sys.with(&mut game, |sim, hooks| {
        modes::player_start(sim, hooks, p, player_mode::DT)
    })
    .unwrap();
    for _ in 0..12 {
        step(&mut game, &mut sys);
    }
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::DD);
    assert_eq!(sys.hooks.log.last().unwrap(), "corpse");
}

#[test]
fn action_result_two_ends_the_animation() {
    let mut game = Game::new();
    let mut sys = system();
    sys.hooks.action_result = 2;
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().mode = player_mode::A1;
    sys.with(&mut game, |sim, hooks| {
        modes::player_event0(sim, hooks, p, 1, 0)
    })
    .unwrap();
    assert_eq!(sys.units.get(p).unwrap().mode, player_mode::NU);
}

#[test]
fn hover_and_expiry_events() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().hover = Some(3);
    game.schedule_event(p, 6, 1, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(pending(&game, p), [(6, 3, 0, 0)]);
    step(&mut game, &mut sys);
    step(&mut game, &mut sys);
    assert_eq!(sys.hooks.log, ["hover"]);
    assert_eq!(sys.units.get(p).unwrap().hover, None);
    assert_ne!(
        sys.units.get(p).unwrap().flags & super::record::flags::HOVER_FREED,
        0
    );
    // Event 12 frees the due state list (test vector 6).
    let s = sys.with(&mut game, |sim, hooks| {
        let s = sim.stats.alloc(0, 0, 0, 1);
        sim.stats.set_state(s, 30);
        sim.stats.set_expire(s, 5);
        sim.stats.set(hooks, s, 0, 5, 0, None);
        sim.stats.attach(hooks, p, s, true);
        s
    });
    assert_eq!(stats(&sys).unit_total(p, 0, 0), 35);
    game.schedule_event(p, 12, 5, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert!(stats(&sys).is_live(s));
    step(&mut game, &mut sys);
    assert!(!stats(&sys).is_live(s));
    assert_eq!(stats(&sys).unit_total(p, 0, 0), 30);
}

#[test]
fn periodic_stats_event() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    sys.units.get_mut(p).unwrap().mode = player_mode::NU;
    game.schedule_event(p, 9, 1, None, 3, 4).unwrap();
    game.schedule_event(p, 9, 5, None, 3, 4).unwrap();
    game.schedule_event(p, 9, 5, None, 2, 4).unwrap();
    // item_aura of skill 4 is 0: the type-9 events with a1 = 3 go.
    step(&mut game, &mut sys);
    assert_eq!(pending(&game, p), [(9, 5, 2, 4)]);
    sys.with(&mut game, |sim, hooks| {
        sim.stats.unit_set(hooks, p, 151, 7, 4);
    });
    for _ in 0..4 {
        step(&mut game, &mut sys);
    }
    assert_eq!(sys.hooks.log, ["aura 2 4 7"]);
    assert_eq!(stats(&sys).unit_total(p, 151, 4), 7);
    let _ = key(151, 4);
}

// ---- monsters ---------------------------------------------------------------------------

fn monster(game: &mut Game, sys: &mut UnitSystem<Fake>, class: u32) -> UnitId {
    let m = spawn(game, sys, UnitType::Monster, class);
    sys.with(game, |sim, hooks| {
        sim.stats.unit_set(hooks, m, 7, 25600, 0);
        sim.stats.unit_set(hooks, m, 6, 25600, 0);
    });
    m
}

#[test]
fn monster_neutral_ai_delay() {
    let mut game = Game::new();
    game.frame = 100;
    let mut sys = system();
    let m = monster(&mut game, &mut sys, 0);
    sys.with(&mut game, |sim, hooks| {
        modes::monster_set_mode(sim, hooks, m, 1)
    })
    .unwrap();
    // aidel 0 → 15.
    assert_eq!(pending(&game, m), [(2, 115, 0, 0)]);
    // A pending AI event later than f: nothing more.
    sys.with(&mut game, |sim, hooks| {
        modes::monster_neutral(sim, hooks, m)
    })
    .unwrap();
    assert_eq!(pending(&game, m), [(2, 115, 0, 0)]);
    let m1 = monster(&mut game, &mut sys, 1);
    sys.with(&mut game, |sim, hooks| {
        modes::monster_neutral(sim, hooks, m1)
    })
    .unwrap();
    assert_eq!(pending(&game, m1), [(2, 109, 0, 0)]);
    // Difficulty column only with game +0x6A / +0x74.
    sys.data.difficulty = 2;
    let m2 = monster(&mut game, &mut sys, 1);
    sys.with(&mut game, |sim, hooks| {
        modes::monster_neutral(sim, hooks, m2)
    })
    .unwrap();
    assert_eq!(pending(&game, m2), [(2, 109, 0, 0)]);
    sys.data.aidel_by_difficulty = true;
    let m3 = monster(&mut game, &mut sys, 1);
    sys.with(&mut game, |sim, hooks| {
        modes::monster_neutral(sim, hooks, m3)
    })
    .unwrap();
    assert_eq!(pending(&game, m3), [(2, 107, 0, 0)]);
    // State 21: f + 45.
    let m4 = monster(&mut game, &mut sys, 1);
    sys.with(&mut game, |sim, hooks| {
        sim.stats.toggle_state(m4, state::AI_DELAY, true);
        modes::monster_neutral(sim, hooks, m4)
    })
    .unwrap();
    assert_eq!(pending(&game, m4), [(2, 145, 0, 0)]);
}

#[test]
fn monster_mode_set_schedules() {
    let mut game = Game::new();
    game.frame = 100;
    let mut sys = system();
    sys.hooks.anim = Some(AnimRecord {
        frames: 12,
        byte_0f: 0,
        events: bytes(&[(6, 1)]),
    });
    sys.hooks.rate = 256;
    sys.hooks.has_path = true;
    // Class 1: A1 does not move → the animation schedule.
    let m = monster(&mut game, &mut sys, 1);
    sys.with(&mut game, |sim, hooks| {
        modes::monster_set_mode(sim, hooks, m, 4)
    })
    .unwrap();
    assert_eq!(pending(&game, m), [(0, 106, 1, 0), (1, 112, 0, 0)]);
    assert_eq!(sys.hooks.log, ["mfn 0x5a75c0"]);
    // Class 0: A1 moves → every tick.
    let m0 = monster(&mut game, &mut sys, 0);
    sys.with(&mut game, |sim, hooks| {
        modes::monster_set_mode(sim, hooks, m0, 4)
    })
    .unwrap();
    assert_eq!(pending(&game, m0), [(0, -1, 0, 0)]);
    // Event 0 calls the mode's event-0 function.
    sys.hooks.log.clear();
    for _ in 0..6 {
        step(&mut game, &mut sys);
    }
    assert!(sys.hooks.log.contains(&"mfn 0x5a7670".to_string()));
    // State 54: fatal assertion.
    let r = sys.with(&mut game, |sim, hooks| {
        sim.stats.toggle_state(m, state::UNINTERRUPTABLE, true);
        modes::monster_set_mode(sim, hooks, m, 4)
    });
    assert_eq!(r, Err(UnitError::Uninterruptable));
}

#[test]
fn monster_regen_rules() {
    let mut game = Game::new();
    let mut sys = system();
    let m = monster(&mut game, &mut sys, 0);
    // r = 0: rescheduled, then all type-3 events cancelled.
    game.schedule_event(m, 3, 1, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert!(pending(&game, m).is_empty());
    // Healing up to max cancels the regen.
    sys.with(&mut game, |sim, hooks| {
        sim.stats.unit_set(hooks, m, 74, 100, 0);
        sim.stats.unit_set(hooks, m, 6, 25550, 0);
    });
    game.schedule_event(m, 3, 2, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(stats(&sys).unit_total(m, 6, 0), 25600);
    assert!(pending(&game, m).is_empty());
    // preventheal with r ≥ 0: no reschedule.
    sys.with(&mut game, |sim, hooks| {
        sim.stats.unit_set(hooks, m, 6, 100, 0);
        sim.stats.toggle_state(m, state::PREVENTHEAL, true);
    });
    game.schedule_event(m, 3, 3, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(stats(&sys).unit_total(m, 6, 0), 100);
    assert!(pending(&game, m).is_empty());
}

#[test]
fn monster_regen_death_and_killer() {
    let mut game = Game::new();
    let mut sys = system();
    sys.units.get(UnitId(0));
    let p = player(&mut game, &mut sys);
    let m = monster(&mut game, &mut sys, 0);
    sys.units.get_mut(m).unwrap().mode = 1;
    // The poisoning hit's class (poison | 6), cleared by the death.
    sys.units.get_mut(m).unwrap().hit_class = 0x56;
    let pguid = sys.units.get(p).unwrap().guid;
    sys.with(&mut game, |sim, hooks| {
        let poison = sim.stats.alloc(0, 0, 0, pguid);
        sim.stats.set_state(poison, state::POISON);
        sim.stats.set(hooks, poison, 74, -300, 0, None);
        sim.stats.attach(hooks, m, poison, true);
        sim.stats.unit_set(hooks, m, 6, 300, 0);
    });
    // Without a room the r < 0, hp < 256 test stops; first tick hp 300 → 0.
    game.schedule_event(m, 3, 1, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(stats(&sys).unit_total(m, 6, 0), 0);
    assert_eq!(sys.hooks.log, [format!("death {:?}", Some(p))]);
    assert_eq!(sys.units.get(m).unwrap().hit_class, 0);
}

#[test]
fn frozen_monsters_drop_events() {
    let mut game = Game::new();
    let mut sys = system();
    let m = monster(&mut game, &mut sys, 0);
    sys.units.get_mut(m).unwrap().mode = 1;
    sys.with(&mut game, |sim, _| {
        sim.stats.toggle_state(m, state::FREEZE, true);
    });
    game.schedule_event(m, 2, 1, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert!(sys.hooks.log.is_empty());
    // Dead monsters are not dropped.
    sys.units.get_mut(m).unwrap().mode = 12;
    game.schedule_event(m, 2, 2, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(sys.hooks.log, ["ai 2"]);
}

// ---- objects and items ------------------------------------------------------------------

#[test]
fn object_and_item_dispatch() {
    let mut game = Game::new();
    let mut sys = system();
    let o = spawn(&mut game, &mut sys, UnitType::Object, 0);
    game.schedule_event(o, 5, 1, None, 9, 9).unwrap();
    game.schedule_event(o, 13, 1, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(sys.hooks.log, ["object 5"]);
    assert_eq!(sys.errors.len(), 1);
    assert_eq!(
        sys.errors[0].1,
        UnitError::NullHandler {
            ty: UnitType::Object,
            event: 13
        }
    );
    // Item expiry frees due lists on the item's own list.
    let i = spawn(&mut game, &mut sys, UnitType::Item, 0);
    let s = sys.with(&mut game, |sim, hooks| {
        let s = sim.stats.alloc(0, 0, 4, 0);
        sim.stats.set_expire(s, 2);
        sim.stats.attach(hooks, i, s, true);
        s
    });
    game.schedule_event(i, 12, 2, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert!(!stats(&sys).is_live(s));
    game.schedule_event(i, 4, 3, None, 0, 0).unwrap();
    step(&mut game, &mut sys);
    assert_eq!(sys.errors.len(), 1);
}

#[test]
fn cooldown_callback() {
    let mut game = Game::new();
    let mut sys = system();
    let p = player(&mut game, &mut sys);
    game.schedule_event(p, 14, 50, Some(super::dispatch::SKILL_COOLDOWN), 0, 0)
        .unwrap();
    game.schedule_event(p, 14, 50, Some(crate::tick::timer::CallbackId(1)), 0, 0)
        .unwrap();
    game.frame = 49;
    step(&mut game, &mut sys);
    assert_eq!(sys.errors.len(), 1);
    assert_eq!(sys.errors[0].1, UnitError::UnknownCallback(1));
    let _ = Arc::new(());
    let _ = event::MODE_CHANGE;
}

#[test]
fn expire_minus_one_becomes_every_tick_without_callback() {
    use crate::tick::timer::CallbackId;
    let mut game = Game::new();
    let mut sys = system();
    let m = spawn(&mut game, &mut sys, UnitType::Monster, 0);
    let cb = Some(CallbackId(9));
    let t = game
        .schedule_event(m, 2, -1, cb, 11, 22)
        .unwrap()
        .expect("scheduled");
    assert_eq!(game.timers.every_tick(TimerClass::Monster), [t]);
    // The callback is lost: a cancel by it no longer finds the timer.
    game.timers.cancel_unit_events_with_callback(m, 2, cb);
    assert!(pending(&game, m).contains(&(2, -1, 11, 22)));
}

// ---- the frame advance's sequence branch ------------

#[test]
fn the_sequence_advance_counts_down_and_reads_the_crossed_event_bytes() {
    use super::anim::advance_sequence;
    use super::record::{Anim, Sequence};
    let mut a = Anim {
        sequence: Some(Sequence {
            frame_count: 4 * 256,
            speed: 256,
            pos: 0,
            events: vec![0, 0, 1, 0],
            drawn: Vec::new(),
        }),
        frame_count: 4 * 256,
        action_frame: 9,
        ..Anim::default()
    };
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert!(advance_sequence(&mut a));
        seen.push((
            a.sequence.as_ref().unwrap().pos,
            a.frame_count,
            a.action_frame,
        ));
    }
    // +0x38 wraps once past +0x34; +0x48 counts down to 0 (complete);
    // +0x4E is the event byte of the frame crossed into.
    assert_eq!(
        seen,
        vec![(256, 768, 0), (512, 512, 1), (768, 256, 0), (0, 0, 0)]
    );
    // A step of 0 (p = b) reads that frame's byte; a step over several
    // frames reads the last non-zero byte crossed.
    let seq = a.sequence.as_mut().unwrap();
    seq.pos = 2 * 256;
    seq.speed = 0;
    advance_sequence(&mut a);
    assert_eq!(a.action_frame, 1);
    let seq = a.sequence.as_mut().unwrap();
    seq.pos = 0;
    seq.speed = 3 * 256;
    advance_sequence(&mut a);
    assert_eq!(a.action_frame, 1);
    // No sequence: nothing.
    a.sequence = None;
    a.action_frame = 7;
    assert!(!advance_sequence(&mut a));
    assert_eq!(a.action_frame, 7);
}

#[test]
fn the_sequence_advance_stores_the_drawn_frame() {
    use super::anim::advance_sequence;
    use super::record::{Anim, Sequence};
    let mut a = Anim {
        sequence: Some(Sequence {
            frame_count: 3 * 256,
            speed: 256,
            pos: 0,
            events: vec![0, 0, 0],
            drawn: vec![0, 5, 9],
        }),
        frame_count: 3 * 256,
        ..Anim::default()
    };
    let mut seen = Vec::new();
    for _ in 0..3 {
        advance_sequence(&mut a);
        seen.push(a.frame);
    }
    // Frame 1, frame 2, then the wrap to frame 0.
    assert_eq!(seen, vec![5 << 8, 9 << 8, 0]);
}

#[test]
fn the_inner_mode_write_queues_and_flags_the_unit() {
    // Run alone by the inline think of a walk end:
    // the client hears of it through flag 0x1
    // (the Rogue Encampment arrival's 0x6D for Warriv at tick 67).
    let mut game = Game::new();
    let mut sys = system();
    let mut seed = Seed::init();
    let req = AllocRequest {
        ty: UnitType::Monster,
        class: 0,
        room: None,
        add: false,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let m = sys
        .with(&mut game, |sim, hooks| {
            allocate(sim, hooks, &mut seed, &req)
        })
        .unwrap()
        .unwrap();
    sys.units.get_mut(m).unwrap().mode = 2;
    sys.units.get_mut(m).unwrap().flags &= !super::record::flags::CHANGED;
    sys.with(&mut game, |sim, hooks| {
        super::modes::write_mode(sim, hooks, m, 1)
    })
    .unwrap();
    let r = sys.units.get(m).unwrap();
    assert_eq!(r.mode, 1);
    assert_ne!(r.flags & super::record::flags::CHANGED, 0);
    // A monster staying in mode 1: nothing.
    sys.units.get_mut(m).unwrap().flags &= !super::record::flags::CHANGED;
    sys.with(&mut game, |sim, hooks| {
        super::modes::write_mode(sim, hooks, m, 1)
    })
    .unwrap();
    assert_eq!(
        sys.units.get(m).unwrap().flags & super::record::flags::CHANGED,
        0
    );
}
