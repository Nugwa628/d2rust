//! The rate equivalence report: the same game on the synthetic Act I
//! install ([`test_fixtures::act1`]) at the base 25 Hz and at 60 Hz,
//! each timing measured in ticks and compared in real time
//! (milliseconds, [`TickRate::ms_for_ticks`]). Runs in CI.
//!
//! Tolerance: a 25 Hz timing is known to within one 25 Hz tick (the
//! event happened at most 40 ms before the tick that shows it) and a
//! 60 Hz timing to within one 60 Hz tick, so the 60 Hz time must fall in
//! `(t25 − 40 ms, t25 + 1000/60 ms)`. The report prints every row; a row
//! outside its window fails the test.
//!
//! Rows so far:
//! - the town walk and the town run (arrival time and end point);
//! - regeneration: the player's stamina and mana from empty to full,
//!   and life from 1 to full, standing in town;
//! - a monster's attack: its hit event, its first AI think and the
//!   attack's end, for both synthetic monsters (12 and 16 frames);
//! - hit recovery for both monsters and the player, and the player's
//!   cast (each mode from its start to its last frame);
//! - an arrow: its lifetime and how far it flies;
//! - timed states on the player: poison, chill and stun, each from the
//!   hit to the state ending;
//! - a spawner: the interval between two child monsters appearing;
//! - a skill cooldown: from a successful use until it stops blocking.
//!
//! Still open: the life a poison takes
//! (the player loses none in this session yet, at either rate).

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::combat;
use d2_sim::missiles::{create_missile, param_flags, MissileParams};
use d2_sim::skills::list::ListOwner;
use d2_sim::skills::use_::{cooldown_blocks, do_skill};
use d2_sim::skills::SkillUnits;
use d2_sim::stats::{stat, StatLists};
use d2_sim::tick::events::event;
use d2_sim::tick::TickRate;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::modes::{self, monster_mode, player_mode};
use d2_sim::units::{UnitId, UnitType};
use test_fixtures::act1::{self, TOWN};
use test_fixtures::animdata::Anim;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup, MOVING};
use test_fixtures::install;

const INIT: u32 = 644_409_375;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
/// Idle frames before a measurement (the join settles).
const SETTLE: usize = 30;
/// A leg's length in sub-tiles.
const LEG: i32 = 10;
/// A bound on the ticks a measured leg may take.
const MAX_TICKS: u32 = 1000;
/// A bound on the ticks a refill or an animation may take (mana from
/// empty is 32 s).
const MAX_LONG: u32 = 4000;
/// The monster attack mode (A1).
const A1: u32 = 4;
/// Frames of the get-hit animations [`timed_act1`] adds: `beast1`,
/// `ghoul1` (both at speed 256, one frame a base tick).
const GH_FRAMES: [u32; 2] = [8, 6];
/// Frames of the player's get-hit and cast animations.
const PLAYER_GH_FRAMES: u32 = 7;
const PLAYER_SC_FRAMES: u32 = 14;
/// The player's cast mode (SC, `plrmode.txt` row 10).
const PLAYER_SC: u32 = 10;
/// The player's life regeneration in [`regeneration_takes_the_same_real_time_at_60_hz`]:
/// 37/256 of a point a base tick (not a whole share of a 60 Hz tick).
const HP_REGEN: i32 = 37;
/// The stun state, the last state row [`timed_act1`] adds.
const STATE_STUNNED: usize = 21;
/// The arrow's velocity [`timed_act1`] sets (row 0 of `missiles.txt`).
const ARROW_VEL: u32 = 2;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("rate-equivalence-{}", std::process::id()));
        let i = install::build(&dir, &timed_act1()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// The synthetic Act I set with what the report's timed rows need and
/// the base set leaves out: get-hit animations for both monsters, the
/// player's get-hit and cast animations (named by [`anims`]), and a
/// velocity for the arrow (row 0 of `missiles.txt`), a delayed Shoot,
/// and the spawner's AI, parent, child and lay animation.
fn timed_act1() -> test_fixtures::synth::Synthetic {
    let mut s = act1::act1();
    for (name, frames) in [
        ("BEGHHTH", GH_FRAMES[0]),
        ("GHGHHTH", GH_FRAMES[1]),
        ("C3GHHTH", PLAYER_GH_FRAMES),
        ("C3SCHTH", PLAYER_SC_FRAMES),
        ("KES1HTH", 5),
    ] {
        s.animdata.push(Anim {
            name: name.into(),
            frames,
            speed: 256,
            events: Vec::new(),
        });
    }
    // The arrow flies with the default flight (server function 1) and
    // may fly in town, where the report's sessions start.
    s.tables.set("missiles", 0, "Vel", &ARROW_VEL.to_string());
    s.tables.set("missiles", 0, "pSrvDoFunc", "1");
    s.tables.set("missiles", 0, "Town", "1");
    // Stats up to `hpregen` (74), the life regeneration a poison lowers,
    // and states up to stunned (21), so cold (11) and stun can be set.
    let stats = s.tables.file("itemstatcost.txt").rows.len();
    for i in stats..=usize::from(stat::HPREGEN) {
        let name = if i == usize::from(stat::HPREGEN) {
            "hpregen".to_string()
        } else {
            format!("stat{i}")
        };
        s.tables
            .row("itemstatcost", &[("stat", &name), ("send bits", "10")]);
    }
    let states = s.tables.file("states.txt").rows.len();
    for i in states..=STATE_STUNNED {
        s.tables.row("states", &[("state", &format!("state{i}"))]);
    }
    // Shoot succeeds through the real missile body and sets a non-round delay.
    s.tables.set("skills", 1, "InTown", "1");
    s.tables.set("skills", 1, "delay", "37");
    let states = s.tables.file("states.txt").rows.len();
    for i in states..=121 {
        s.tables.row("states", &[("state", &format!("state{i}"))]);
    }
    // SandMaggotQueen's AI uses fixed rows 284 (parent) and 68 (child).
    // Pad with keeper rows; only these two made-up monsters are allocated.
    let keeper = s.tables.file("monstats.txt").rows[2].clone();
    for i in s.tables.file("monstats.txt").rows.len()..=284 {
        s.tables
            .files
            .get_mut("monstats.txt")
            .unwrap()
            .rows
            .push(keeper.clone());
        let id = format!("spawn{i}");
        s.tables.set("monstats", i, "Id", &id);
        s.tables.set("monstats", i, "BaseId", &id);
    }
    for i in s.tables.file("monai.txt").rows.len()..=66 {
        let name = if i == 66 {
            "SandMaggotQueen".to_string()
        } else {
            format!("unused{i}")
        };
        s.tables.row("monai", &[("AI", &name)]);
    }
    s.tables.set("monstats", 284, "AI", "SandMaggotQueen");
    s.tables.set("monstats", 284, "aip1", "2");
    s.tables.set("monstats", 284, "aip2", "10");
    s
}

fn session(rate: TickRate) -> Session {
    let setup = Setup {
        creation: ActCreation::Full,
        init_seed: INIT,
        town: TOWN,
        game_seed: GAME_SEED,
        class: CLASS,
        known_waypoints: Vec::new(),
    };
    let mut fx = Session::new_at(data(), &setup, rate);
    for _ in 0..SETTLE {
        fx.frame();
    }
    fx
}

/// A move request `id` (0x01 walk, 0x03 run) `dx` sub-tiles east: the
/// ticks from the request's tick until the player stopped (the request's
/// tick counts), and the end point.
fn leg(rate: TickRate, id: u8, dx: i32) -> (u32, (i32, i32)) {
    let mut fx = session(rate);
    let start = fx.pos();
    let to = (start.0 + dx, start.1);
    let mut m = vec![id];
    m.extend_from_slice(&(to.0 as u16).to_le_bytes());
    m.extend_from_slice(&(to.1 as u16).to_le_bytes());
    fx.send(&m);
    let mut ticks = 1;
    while MOVING.contains(&fx.mode()) && ticks < MAX_TICKS {
        fx.frame();
        ticks += 1;
    }
    assert!(
        ticks < MAX_TICKS,
        "{id:#04x}: still moving after {ticks} ticks"
    );
    fx.assert_clean("rate leg");
    (ticks, fx.pos())
}

/// One report row: the timing at both rates, in ticks and milliseconds,
/// and whether the 60 Hz time is inside the window.
struct Row {
    what: &'static str,
    base: (u32, u64),
    fast: (u32, u64),
    ok: bool,
}

fn row(what: &'static str, base_ticks: u32, fast: TickRate, fast_ticks: u32) -> Row {
    let (bh, fh) = (u64::from(TickRate::BASE.hz()), u64::from(fast.hz()));
    // Times in units of 1 / (bh · fh) seconds, where one tick of either
    // rate is a whole number: a base tick is `fh` units, a fast tick `bh`.
    let (b, f) = (u64::from(base_ticks) * fh, u64::from(fast_ticks) * bh);
    Row {
        what,
        base: (base_ticks, TickRate::BASE.ms_for_ticks(base_ticks)),
        fast: (fast_ticks, fast.ms_for_ticks(fast_ticks)),
        ok: b < f + fh && f < b + bh,
    }
}

fn report(rows: &[Row], fast: TickRate) {
    println!(
        "{:<24} {:>8} {:>8} {:>8} {:>8}  ok",
        "timing",
        "25 tk",
        "25 ms",
        format!("{} tk", fast.hz()),
        format!("{} ms", fast.hz())
    );
    for r in rows {
        println!(
            "{:<24} {:>8} {:>8} {:>8} {:>8}  {}",
            r.what, r.base.0, r.base.1, r.fast.0, r.fast.1, r.ok
        );
    }
    let bad: Vec<&str> = rows.iter().filter(|r| !r.ok).map(|r| r.what).collect();
    assert!(bad.is_empty(), "outside the window: {bad:?}");
}

#[test]
fn town_moves_take_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    let mut rows = Vec::new();
    for (what, id, dx) in [
        ("town walk east", 0x01, LEG),
        ("town walk west", 0x01, -LEG),
        ("town run east", 0x03, LEG),
        ("town run west", 0x03, -LEG),
    ] {
        let (bt, bend) = leg(TickRate::BASE, id, dx);
        let (ft, fend) = leg(fast, id, dx);
        assert_eq!(bend, fend, "{what}: the same end point at both rates");
        rows.push(row(what, bt, fast, ft));
    }
    report(&rows, fast);
}

/// Ticks from emptying the player's vital `s` until it is back at
/// `max`, standing in town. The regeneration event is scheduled the way
/// the join schedules it (the next tick, then every tick).
fn refill(rate: TickRate, s: u16, max: fn(&StatLists, UnitId) -> i32) -> u32 {
    refill_from(rate, s, 0, max, None)
}

/// [`refill`] from `low`, with the base stat `extra` set first (life
/// regenerates only with `hpregen`).
fn refill_from(
    rate: TickRate,
    s: u16,
    low: i32,
    max: fn(&StatLists, UnitId) -> i32,
    extra: Option<(u16, i32)>,
) -> u32 {
    let mut fx = session(rate);
    let p = fx.player;
    let sim = fx.sim();
    let sys = &mut sim.events.action.sys;
    if let Some((e, v)) = extra {
        sys.stats.unit_set(&mut sys.hooks, p, e, v, 0);
    }
    let full = max(&sys.stats, p);
    sys.stats.unit_set(&mut sys.hooks, p, s, low, 0);
    let f = sim.game.frame;
    sim.game
        .schedule_event(p, u32::from(event::STAT_REGEN), f + 1, None, 0, 0)
        .unwrap();
    let mut ticks = 0;
    while ticks < MAX_LONG {
        fx.frame();
        ticks += 1;
        if fx.sim().events.action.sys.stats.unit_total(p, s, 0) >= full {
            break;
        }
    }
    assert!(ticks < MAX_LONG, "stat {s}: not full after {ticks} ticks");
    ticks
}

/// Names the synthetic monsters' animations: class 0 (`beast1`) is `BE`,
/// class 1 (`ghoul1`) is `GH`, weapon class HTH. The synthetic AnimData
/// has their NU, WL and A1.
fn monster_anims(ty: UnitType, class: u32, mode: u32) -> Option<[u8; 8]> {
    if ty != UnitType::Monster {
        return None;
    }
    let code = [b"BE", b"GH"].get(class as usize)?;
    let m = [b"DT", b"NU", b"WL", b"GH", b"A1"].get(mode as usize)?;
    let mut n = [0; 8];
    n[..2].copy_from_slice(&code[..]);
    n[2..4].copy_from_slice(&m[..]);
    n[4..7].copy_from_slice(b"HTH");
    Some(n)
}

/// A monster of `class`, standing in its neutral mode 4 sub-tiles east
/// of the player.
fn monster(fx: &mut Session, class: u32) -> UnitId {
    let (p, pos) = (fx.player, fx.pos());
    let s = fx.sim();
    let room = s.game.lists.unit(p).unwrap().room();
    let req = AllocRequest {
        ty: UnitType::Monster,
        class,
        room,
        add: true,
        fixed_guid: None,
        mode: monster_mode::NU,
        allied: false,
    };
    s.events
        .action
        .with(&mut s.game, |g, v| v.allocate(g, &req, pos.0 + 4, pos.1))
        .expect("the monster")
}

/// Ticks until `unit` has left `mode` (the request's tick counts).
fn ticks_in_mode(fx: &mut Session, unit: UnitId, mode: u32) -> u32 {
    let mut ticks = 0;
    while ticks < MAX_LONG {
        fx.frame();
        ticks += 1;
        if fx.sim().events.action.sys.units.get(unit).unwrap().mode != mode {
            break;
        }
    }
    assert!(ticks < MAX_LONG, "{unit:?}: still in mode {mode}");
    ticks
}

/// A monster of `class` set to its attack beside the player: the ticks
/// until its hit event (the mode-change event the attack's frame event
/// schedules) and its first AI think are due, and the ticks until the
/// attack has ended (the monster is back in another mode).
fn attack(rate: TickRate, class: u32) -> (u32, u32, u32) {
    let mut fx = session(rate);
    fx.sim().events.action.sys.hooks.x.anim_names = Some(monster_anims);
    let m = monster(&mut fx, class);
    let s = fx.sim();
    s.events
        .action
        .sys
        .with(&mut s.game, |sim, h| modes::monster_set_mode(sim, h, m, A1))
        .expect("the attack starts");
    let f = s.game.frame;
    let due = |ty: u8| {
        let t = &s.game.timers;
        let at = t
            .unit_timers(m)
            .into_iter()
            .find(|&id| t.event(id).is_some_and(|e| e.0 == ty))
            .and_then(|id| t.expire(id))
            .unwrap_or_else(|| panic!("class {class}: no event {ty}"));
        (at - f) as u32
    };
    let (hit, think) = (due(event::MODE_CHANGE), due(event::AI_THINK));
    let mut ticks = 0;
    while ticks < MAX_LONG {
        fx.frame();
        ticks += 1;
        if fx.sim().events.action.sys.units.get(m).unwrap().mode != A1 {
            break;
        }
    }
    assert!(ticks < MAX_LONG, "class {class}: still attacking");
    (hit, think, ticks)
}

#[test]
fn regeneration_takes_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    let mut rows = Vec::new();
    for (what, s, max) in [
        (
            "stamina empty to full",
            stat::STAMINA,
            StatLists::max_stamina as fn(&StatLists, UnitId) -> i32,
        ),
        ("mana empty to full", stat::MANA, StatLists::max_mana),
    ] {
        rows.push(row(
            what,
            refill(TickRate::BASE, s, max),
            fast,
            refill(fast, s, max),
        ));
    }
    // Life from one point to full at `hpregen` HP_REGEN (256ths of a
    // point a base tick).
    let life = |rate| {
        refill_from(
            rate,
            stat::HITPOINTS,
            256,
            StatLists::max_life,
            Some((stat::HPREGEN, HP_REGEN)),
        )
    };
    rows.push(row(
        "life 1 to full",
        life(TickRate::BASE),
        fast,
        life(fast),
    ));
    report(&rows, fast);
}

#[test]
fn monster_attacks_take_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    let mut rows = Vec::new();
    for (class, [hit, think, end]) in [
        (
            0,
            ["beast attack hit", "beast first think", "beast attack end"],
        ),
        (
            1,
            ["ghoul attack hit", "ghoul first think", "ghoul attack end"],
        ),
    ] {
        let b = attack(TickRate::BASE, class);
        let f = attack(fast, class);
        rows.push(row(hit, b.0, fast, f.0));
        rows.push(row(think, b.1, fast, f.1));
        rows.push(row(end, b.2, fast, f.2));
    }
    report(&rows, fast);
}

#[test]
fn the_window_is_one_tick_of_each_rate() {
    let fast = TickRate::new(60).unwrap();
    // 10 base ticks = 400 ms; the window is (360, 416.7) ms.
    assert!(!row("", 10, fast, 21).ok, "350 ms");
    assert!(row("", 10, fast, 22).ok, "366.7 ms");
    assert!(row("", 10, fast, 24).ok, "400 ms");
    assert!(!row("", 10, fast, 25).ok, "416.7 ms is the edge, outside");
}

/// Names [`monster_anims`]'s animations and the player's get-hit and cast
/// ([`timed_act1`]), so a player mode lasts its animation.
fn all_anims(ty: UnitType, class: u32, mode: u32) -> Option<[u8; 8]> {
    if ty != UnitType::Player {
        return monster_anims(ty, class, mode);
    }
    let m = match mode {
        player_mode::GH => b"GH",
        PLAYER_SC => b"SC",
        _ => return None,
    };
    let mut n = *b"C3__HTH\0";
    n[2..4].copy_from_slice(m);
    Some(n)
}

/// Ticks a monster of `class` spends in its get-hit animation.
fn monster_hit_recovery(rate: TickRate, class: u32) -> u32 {
    let mut fx = session(rate);
    fx.sim().events.action.sys.hooks.x.anim_names = Some(all_anims);
    let m = monster(&mut fx, class);
    let s = fx.sim();
    s.events
        .action
        .sys
        .with(&mut s.game, |sim, h| {
            modes::monster_set_mode(sim, h, m, monster_mode::GH)
        })
        .expect("the get-hit starts");
    ticks_in_mode(&mut fx, m, monster_mode::GH)
}

/// Ticks the player spends in `mode` (get-hit or cast) once set.
fn player_anim(rate: TickRate, mode: u32) -> u32 {
    let mut fx = session(rate);
    let p = fx.player;
    let s = fx.sim();
    s.events.action.sys.hooks.x.anim_names = Some(all_anims);
    let started = s
        .events
        .action
        .sys
        .with(&mut s.game, |sim, h| modes::player_start(sim, h, p, mode))
        .expect("the mode starts");
    assert!(started, "mode {mode} accepted");
    ticks_in_mode(&mut fx, p, mode)
}

#[test]
fn hit_recovery_and_casts_take_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    let mut rows = Vec::new();
    for (class, what) in [(0, "beast hit recovery"), (1, "ghoul hit recovery")] {
        rows.push(row(
            what,
            monster_hit_recovery(TickRate::BASE, class),
            fast,
            monster_hit_recovery(fast, class),
        ));
    }
    for (mode, what) in [
        (player_mode::GH, "player hit recovery"),
        (PLAYER_SC, "player cast"),
    ] {
        rows.push(row(
            what,
            player_anim(TickRate::BASE, mode),
            fast,
            player_anim(fast, mode),
        ));
    }
    report(&rows, fast);
}

/// An arrow shot east from the player's spot: the ticks until it is gone
/// and the sub-tiles it flew. A monster owns it, since a player's missile
/// in town is removed at once.
fn arrow(rate: TickRate) -> (u32, i32) {
    let mut fx = session(rate);
    let (p, pos) = (fx.player, fx.pos());
    let owner = monster(&mut fx, 0);
    let params = MissileParams {
        owner: Some(owner),
        origin: Some(p),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: pos.0 + 40,
        target_y: pos.1,
        ..MissileParams::default()
    };
    let s = fx.sim();
    let m = s
        .events
        .action
        .missiles(&mut s.game, |g, cx| create_missile(g, cx, &params))
        .expect("the missile store")
        .expect("the arrow");
    let mut last = pos;
    let mut ticks = 0;
    while ticks < MAX_LONG {
        fx.frame();
        ticks += 1;
        let s = fx.sim();
        if s.game.lists.unit(m).is_none() {
            break;
        }
        last = s.events.action.hooks().path_position(m);
    }
    assert!(ticks < MAX_LONG, "the arrow still flies");
    (ticks, last.0 - pos.0)
}

#[test]
fn missiles_live_and_fly_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    let ((bt, bd), (ft, fd)) = (arrow(TickRate::BASE), arrow(fast));
    println!("arrow flight: {bd} sub-tiles at 25 Hz, {fd} at 60 Hz");
    assert!(bd > 0, "the arrow flies");
    assert!(
        (bd - fd).abs() <= ARROW_VEL as i32,
        "the arrow flies as far at both rates (within one base tick)"
    );
    report(&[row("arrow lifetime", bt, fast, ft)], fast);
}

/// A timed effect [`timed_state`] puts on the player.
#[derive(Clone, Copy)]
enum Effect {
    /// Poison: life (in 256ths) over base ticks.
    Poison(i32, i32),
    /// Chill for base ticks.
    Cold(i32),
    /// Stun for base ticks.
    Stun(i32),
}

/// Ticks the player carries `state` after `effect` (the effect's tick
/// counts).
fn timed_state(rate: TickRate, effect: Effect, state: u16) -> u32 {
    let mut fx = session(rate);
    let p = fx.player;
    let s = fx.sim();
    match effect {
        Effect::Poison(damage, length) => s
            .events
            .action
            .combat(&mut s.game, |w, _| combat::poison(w, p, p, damage, length)),
        Effect::Cold(length) => s
            .events
            .action
            .combat(&mut s.game, |w, t| combat::cold(w, &t.combat, p, p, length)),
        Effect::Stun(n) => s
            .events
            .action
            .combat(&mut s.game, |w, t| combat::stun(w, &t.combat, p, p, n)),
    }
    let on = |fx: &mut Session| {
        let s = fx.sim();
        s.events
            .action
            .combat(&mut s.game, |w, _| w.has_state(p, state))
    };
    assert!(on(&mut fx), "state {state} is on");
    let mut ticks = 0;
    while ticks < MAX_LONG {
        fx.frame();
        ticks += 1;
        if !on(&mut fx) {
            break;
        }
    }
    assert!(ticks < MAX_LONG, "state {state} is still on");
    ticks
}

#[test]
fn timed_states_last_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    let mut rows = Vec::new();
    for (what, effect, state) in [
        ("poison 50", Effect::Poison(20 << 8, 50), 2),
        ("chill 75", Effect::Cold(75), 11),
        ("stun 25", Effect::Stun(25), 21),
    ] {
        let (bt, ft) = (
            timed_state(TickRate::BASE, effect, state),
            timed_state(fast, effect, state),
        );
        rows.push(row(what, bt, fast, ft));
    }
    report(&rows, fast);
}

/// Successful Shoot use through the real skill do pipeline. Observe the shared
/// delay gate each session tick, including its last blocked tick and expiry.
fn skill_cooldown(rate: TickRate) -> u32 {
    let mut fx = session(rate);
    let p = fx.player;
    let pos = fx.pos();
    let s = fx.sim();
    s.events.action.sys.hooks.x.skill_target = Some((pos.0 + 10, pos.1));
    let t = s.events.action.sys.hooks.tables.clone();
    let list = s.events.action.sys.hooks.skill_lists.entry(p).or_default();
    list.set_base(&t.skills.skills, ListOwner::player(CLASS as i32), 1, 1);
    list.current = list.native(1);
    let blocked = |fx: &mut Session| {
        let s = fx.sim();
        s.events
            .action
            .skill_use(&mut s.game, |w| cooldown_blocks(w, &t.skills, p, 1, 1))
    };
    assert!(!blocked(&mut fx), "usable before the cast");
    let s = fx.sim();
    let before = s.game.lists.units_of_type(UnitType::Missile).len();
    let used = s
        .events
        .action
        .skill_use(&mut s.game, |w| do_skill(w, &t.skills, p, 1, 1));
    assert_eq!(used, 1, "the skill succeeds");
    assert_eq!(
        s.game.lists.units_of_type(UnitType::Missile).len(),
        before + 1,
        "the skill creates its missile"
    );
    assert!(blocked(&mut fx), "the use starts a cooldown");
    let mut ticks = 0;
    while ticks < MAX_LONG {
        fx.frame();
        ticks += 1;
        if !blocked(&mut fx) {
            break;
        }
    }
    assert!(
        ticks > 1 && ticks < MAX_LONG,
        "cooldown expires after a measured interval"
    );
    let s = fx.sim();
    assert_eq!(
        s.events
            .action
            .skill_use(&mut s.game, |w| do_skill(w, &t.skills, p, 1, 1)),
        1,
        "the skill succeeds again after expiry"
    );
    assert!(blocked(&mut fx), "the next use starts a fresh cooldown");
    fx.assert_clean("skill cooldown");
    ticks
}

#[test]
fn skill_cooldowns_take_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    report(
        &[row(
            "skill cooldown",
            skill_cooldown(TickRate::BASE),
            fast,
            skill_cooldown(fast),
        )],
        fast,
    );
}

/// The spawner AI lays, spawns a child, rests and lays again. Measure the
/// interval between actual children appearing, rather than a timer's due date.
fn spawn_interval(rate: TickRate) -> u32 {
    let mut fx = session(rate);
    let (p, pos) = (fx.player, fx.pos());
    let s = fx.sim();
    s.events.action.sys.hooks.x.anim_names = Some(|ty, class, mode| {
        (ty == UnitType::Monster && matches!(class, 68 | 284) && mode == 8).then_some(*b"KES1HTH\0")
    });
    let room = s.game.lists.unit(p).unwrap().room().unwrap();
    let parent = s
        .events
        .population(&mut s.game, |cx| {
            d2_sim::monsters::population::placement::place_at(
                cx,
                room,
                None,
                pos.0 + 4,
                pos.1,
                284,
                1,
                -1,
                0,
            )
            .unit()
        })
        .expect("the spawner");
    assert!(
        s.events
            .action
            .sys
            .hooks
            .ai
            .as_ref()
            .unwrap()
            .control(parent)
            .is_some(),
        "the spawner has real AI control"
    );
    let mut first = None;
    for tick in 1..MAX_LONG {
        fx.frame();
        let s = fx.sim();
        let children = s
            .game
            .lists
            .units_of_type(UnitType::Monster)
            .into_iter()
            .filter(|&u| {
                s.events
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == 68)
            })
            .count();
        match (children, first) {
            (1, None) => first = Some(tick),
            (2, Some(start)) => {
                assert!(
                    tick > start + 1,
                    "the children appear on separate spawn cycles"
                );
                fx.assert_clean("spawn interval");
                return tick - start;
            }
            _ => {}
        }
    }
    panic!("two children never appeared; first spawn: {first:?}");
}

#[test]
fn spawns_take_the_same_real_time_at_60_hz() {
    let fast = TickRate::new(60).unwrap();
    report(
        &[row(
            "spawn interval",
            spawn_interval(TickRate::BASE),
            fast,
            spawn_interval(fast),
        )],
        fast,
    );
}
