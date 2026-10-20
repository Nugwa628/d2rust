//! `goto` on D2RUST: a target in the player's active rooms is reached in
//! one step (the player placed by the free-point search next to it, the
//! target's GUID as the result); a target that exists nowhere ends
//! `failed (none in level N)` after the walk has seen every room of the
//! level it can reach, with
//! `pending` steps between ticks while there are rooms left.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::poke::{
    self, Directive, Env, GotoWalk, PokeResult, GOTO_MAX_STEPS, GOTO_NONE_IN_LEVEL,
};
use d2_sim::units::UnitType;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{cheb, Session, Setup};
use test_fixtures::{install, synth};

fn synthetic() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("poke-goto-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn session() -> Session {
    Session::new(
        synthetic(),
        &Setup {
            creation: ActCreation::TownOnly,
            init_seed: 0x1234_5678,
            town: 1,
            game_seed: 1234,
            class: 3,
            known_waypoints: Vec::new(),
        },
    )
}

fn line(s: &mut Session, text: &str) -> PokeResult {
    let player = s.player;
    let sim = s.sim();
    poke::apply_line(&mut sim.game, &mut sim.events, &Env::new(player), text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// Steps `text` (a `goto`) once per frame until it is not pending.
fn walk(s: &mut Session, text: &str) -> (PokeResult, GotoWalk) {
    let Directive::Goto(t) = poke::parse_directive_text(text).unwrap() else {
        panic!("{text}: not a goto");
    };
    let mut w = GotoWalk::default();
    loop {
        let player = s.player;
        let r = {
            let sim = s.sim();
            poke::goto_step(
                &mut sim.game,
                &mut sim.events,
                &Env::new(player),
                &t,
                &mut w,
            )
        };
        if r != PokeResult::Pending {
            return (r, w);
        }
        assert!(w.steps <= GOTO_MAX_STEPS, "a pending step past the limit");
        s.frame();
    }
}

#[test]
fn goto_a_unit_in_the_active_rooms_lands_next_to_it() {
    let mut s = session();
    let PokeResult::Ok(Some(guid)) = line(&mut s, "object 1 @x+8 @y") else {
        panic!("object poke");
    };
    let obj = s
        .sim()
        .game
        .lists
        .find_unit(UnitType::Object, guid)
        .expect("the object");
    let at = s.sim().events.action.hooks().path_position(obj);
    // The first unit of the class, ascending GUID, in the player's level.
    let first = {
        let sim = s.sim();
        let mut v: Vec<u32> = sim
            .game
            .lists
            .units_of_type(UnitType::Object)
            .into_iter()
            .filter(|&u| {
                sim.events
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == 1)
            })
            .filter_map(|u| sim.game.lists.unit(u).map(|e| e.guid))
            .collect();
        v.sort_unstable();
        v[0]
    };
    let (r, w) = walk(&mut s, "goto unit 2:1");
    assert_eq!(r, PokeResult::Ok(Some(first)));
    assert_eq!(w.steps, 1);
    if first == guid {
        let p = s.pos();
        assert!(cheb(p, at) <= 3, "player at {p:?}, object at {at:?}");
    }
    assert_eq!(s.sim().events.errors(), Vec::<String>::new());
}

// `mode <m>` keeps only units in that mode: the doors sweep asks for a
// closed door (mode 0), since a door whose class has `PreOperate` may spawn
// open.
#[test]
fn goto_with_a_mode_takes_only_a_unit_in_that_mode() {
    let mut s = session();
    let PokeResult::Ok(Some(guid)) = line(&mut s, "object 1 @x+8 @y mode 1") else {
        panic!("object poke");
    };
    let obj = s
        .sim()
        .game
        .lists
        .find_unit(UnitType::Object, guid)
        .expect("the object");
    assert_eq!(
        s.sim().events.action.sys.units.get(obj).map(|r| r.mode),
        Some(1)
    );
    // The class-1 objects in mode 1, ascending GUID.
    let first = {
        let sim = s.sim();
        let mut v: Vec<u32> = sim
            .game
            .lists
            .units_of_type(UnitType::Object)
            .into_iter()
            .filter(|&u| {
                sim.events
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == 1 && r.mode == 1)
            })
            .filter_map(|u| sim.game.lists.unit(u).map(|e| e.guid))
            .collect();
        v.sort_unstable();
        v[0]
    };
    let (r, _) = walk(&mut s, "goto unit 2:1 mode 1");
    assert_eq!(r, PokeResult::Ok(Some(first)));
    // No class-1 object in mode 7: the walk ends `none in level`.
    let (r, _) = walk(&mut s, "goto unit 2:1 mode 7");
    assert!(
        matches!(&r, PokeResult::FailedWith(n) if n.starts_with(GOTO_NONE_IN_LEVEL)),
        "{r:?}"
    );
}

#[test]
fn goto_a_missing_target_explores_then_fails() {
    let mut s = session();
    let (r, w) = walk(&mut s, "goto unit 2:4000");
    // Every room was searched: the note names the level, so a sweep can
    // tell a unit this map does not hold from a walk that got stuck.
    let none = |r: &PokeResult| matches!(r, PokeResult::FailedWith(n) if n.starts_with(GOTO_NONE_IN_LEVEL));
    assert!(none(&r), "{r:?}");
    assert!(
        w.steps >= 1 && w.steps <= GOTO_MAX_STEPS,
        "{} steps",
        w.steps
    );
    assert!(!w.seen.is_empty());
    // Every room of the town reachable through the near arrays was seen:
    // one more walk from the end sees nothing new.
    let seen = w.seen.len();
    let (r2, w2) = walk(&mut s, "goto unit 2:4000");
    assert!(none(&r2), "{r2:?}");
    assert!(w2.seen.len() <= seen, "{} > {seen}", w2.seen.len());
    // A one-call `apply` is one step with a fresh walk.
    let r3 = line(&mut s, "goto unit 2:4000");
    assert!(
        none(&r3) || matches!(r3, PokeResult::Failed | PokeResult::Pending),
        "{r3:?}"
    );
}

#[test]
fn pos_free_and_hop_land_on_missile_passable_cells() {
    let mut s = session();
    let player = s.player;
    let (px, py) = s.pos();
    let (tx, ty) = (px + 6, py);
    let mask_at = |s: &mut Session, x, y| {
        let sim = s.sim();
        let room = sim.game.lists.unit(player).and_then(|e| e.room()).unwrap();
        sim.events
            .action
            .sys
            .hooks
            .drlg
            .collision(&sim.game, room, x, y)
            .unwrap()
    };
    // Missile-blocking bit 0x4 on the target cell.
    {
        let sim = s.sim();
        let room = sim.game.lists.unit(player).and_then(|e| e.room()).unwrap();
        *sim.events
            .action
            .sys
            .hooks
            .drlg
            .collision_mut(&sim.game, room, tx, ty)
            .unwrap() |= 0x4;
    }
    assert_eq!(
        line(&mut s, &format!("pos @player {tx} {ty} free")),
        PokeResult::Ok(None)
    );
    let p = s.pos();
    assert_ne!(p, (tx, ty), "the blocked cell itself");
    assert!(cheb(p, (tx, ty)) <= 3, "landed at {p:?}");
    assert_eq!(mask_at(&mut s, p.0, p.1) & poke::LAND_MASK, 0);
    // A hop never ends on a cell with 0x4: block the full step's cell.
    let (hx, hy) = s.pos();
    let (fx, fy) = {
        let sim = s.sim();
        let room = sim.game.lists.unit(player).and_then(|e| e.room()).unwrap();
        let hooks = &mut sim.events.action.sys.hooks;
        let (dx, dy) = [(16, 0), (-16, 0), (0, 16), (0, -16)]
            .into_iter()
            .find(|&(dx, dy)| {
                hooks
                    .drlg
                    .collision(&sim.game, room, hx + dx, hy + dy)
                    .is_some()
            })
            .expect("a cell 16 away in the town grid");
        let (fx, fy) = (hx + dx, hy + dy);
        *hooks.drlg.collision_mut(&sim.game, room, fx, fy).unwrap() |= 0x4;
        (fx, fy)
    };
    assert_eq!(
        line(&mut s, &format!("hop @player {fx} {fy}")),
        PokeResult::Ok(None)
    );
    let p = s.pos();
    assert_ne!(p, (fx, fy));
    assert_eq!(mask_at(&mut s, p.0, p.1) & poke::LAND_MASK, 0);
    assert_eq!(s.sim().events.errors(), Vec::<String>::new());
}
