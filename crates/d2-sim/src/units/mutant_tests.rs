//! Tests written against surviving mutants: the variant
//! animation start index and the regeneration
//! conditions, on a small recording fake.

use std::sync::Arc;

use crate::game::Game;
use crate::rng::Seed;
use crate::stats::states::{group, state, StateTable};
use crate::stats::{stat, StatData, StatHost};

use super::anim::{schedule, Events, Form};
use super::dispatch::{monster_regen, player_regen, UnitSystem};
use super::hooks::{Sim, UnitHooks};
use super::lifecycle::{allocate, AllocRequest, LifecycleHooks};
use super::modes::{monster_neutral, player_mode};
use super::record::flags2;
use super::tests::{bytes, data, pending};
use super::{UnitId, UnitType};

// ---- variants ---------------------------------------------------------------

/// (event, expire, a1, a2) of a variant schedule at f = 100.
fn variant(form: Form, cur: i32, ev: &[(usize, u8)]) -> Vec<(u8, i32, u32, u32)> {
    let events = bytes(ev);
    let ev = Events::Record {
        byte_0f: 0,
        events: &events,
    };
    schedule(form, 100, 256, 12 * 256, cur, ev)
        .unwrap()
        .unwrap()
        .events
        .iter()
        .map(|e| (e.event, e.expire, e.a1, e.a2))
        .collect()
}

/// Variants loop from start index c − 1: an action
/// byte at c − 1 is scheduled at the first frame.
#[test]
fn variants_start_at_c_minus_one() {
    // Percent 50, cur 96: c = (50 · 4) / 100 = 2.
    assert_eq!(
        variant(Form::Percent(50), 96 * 256, &[(1, 1)]),
        [(0, 101, 1, 0), (1, 110, 0, 0)]
    );
    // Frames 2, cur 96: c = 100 − 96 − 2 = 2.
    assert_eq!(
        variant(Form::Frames(2), 96 * 256, &[(1, 1)]),
        [(0, 101, 1, 0), (1, 110, 0, 0)]
    );
}

// ---- regeneration --------------------------------------------------------------

#[derive(Default)]
struct Rec {
    deaths: Vec<UnitId>,
}

impl StatHost for Rec {}

impl UnitHooks for Rec {
    fn monster_death(&mut self, _: &mut Sim<'_>, unit: UnitId, _: Option<UnitId>) {
        self.deaths.push(unit);
    }
}

impl LifecycleHooks for Rec {}

fn system_with(stats: Arc<StatData>) -> UnitSystem<Rec> {
    UnitSystem::new(stats, data(), Rec::default())
}

fn system() -> UnitSystem<Rec> {
    system_with(crate::stats::tests::data())
}

fn spawn(game: &mut Game, sys: &mut UnitSystem<Rec>, ty: UnitType, mode: u32) -> UnitId {
    let mut seed = Seed::init();
    sys.with(game, |sim, hooks| {
        allocate(
            sim,
            hooks,
            &mut seed,
            &AllocRequest {
                ty,
                class: 0,
                room: None,
                add: true,
                fixed_guid: None,
                mode,
                allied: false,
            },
        )
    })
    .map(|u| {
        let u = u.expect("unit");
        sys.units.get_mut(u).unwrap().mode = mode;
        u
    })
    .expect("allocate")
}

fn set(game: &mut Game, sys: &mut UnitSystem<Rec>, u: UnitId, s: u16, v: i32) {
    sys.with(game, |sim, hooks| sim.stats.unit_set(hooks, u, s, v, 0));
}

fn total(sys: &UnitSystem<Rec>, u: UnitId, s: u16) -> i32 {
    sys.stats.unit_total(u, s, 0)
}

/// A plain state list of `state` attached to `u`.
fn state_list(game: &mut Game, sys: &mut UnitSystem<Rec>, u: UnitId, s: u32) {
    sys.with(game, |sim, hooks| {
        let l = sim.stats.alloc(0, 0, 0, 1);
        sim.stats.set_state(l, s);
        sim.stats.set(hooks, l, 20, 1, 0, None);
        sim.stats.attach(hooks, u, l, true);
    });
}

fn has_state_list(sys: &UnitSystem<Rec>, u: UnitId, s: u32) -> bool {
    sys.stats.state_list_owner(u, s).is_some()
}

fn regen_player(game: &mut Game, sys: &mut UnitSystem<Rec>, p: UnitId) {
    sys.with(game, |sim, hooks| player_regen(sim, hooks, p, 0, 0))
        .unwrap();
}

fn regen_monster(game: &mut Game, sys: &mut UnitSystem<Rec>, m: UnitId) {
    sys.with(game, |sim, hooks| monster_regen(sim, hooks, m))
        .unwrap();
}

/// Player life: healing to exactly max life keeps the
/// healthpot list (only hp > max frees it); hp < 256 becomes 256.
#[test]
fn player_life_bounds() {
    let mut game = Game::new();
    let mut sys = system();
    let p = spawn(&mut game, &mut sys, UnitType::Player, player_mode::NU);
    set(&mut game, &mut sys, p, stat::MAXHP, 12800);
    set(&mut game, &mut sys, p, stat::HITPOINTS, 12700);
    set(&mut game, &mut sys, p, stat::HPREGEN, 100);
    state_list(&mut game, &mut sys, p, state::HEALTHPOT);
    regen_player(&mut game, &mut sys, p);
    assert_eq!(total(&sys, p, stat::HITPOINTS), 12800);
    assert!(has_state_list(&sys, p, state::HEALTHPOT));
    // r < 0 below 256 points: raised to 256.
    set(&mut game, &mut sys, p, stat::HPREGEN, -12700);
    regen_player(&mut game, &mut sys, p);
    assert_eq!(total(&sys, p, stat::HITPOINTS), 256);
}

/// Player stamina: mode 2 with s < 256 stops, whatever
/// the recovery bonus.
#[test]
fn stamina_mode_2_low_stops_even_with_bonus() {
    let mut game = Game::new();
    let mut sys = system();
    let p = spawn(&mut game, &mut sys, UnitType::Player, 2);
    set(&mut game, &mut sys, p, stat::MAXSTAMINA, 2560);
    set(&mut game, &mut sys, p, stat::STAMINARECOVERYBONUS, 1000);
    regen_player(&mut game, &mut sys, p);
    assert_eq!(total(&sys, p, stat::STAMINA), 0);
}

/// Player mana: the manapot list goes only when v ≥ m and
/// i > 0.
#[test]
fn manapot_freed_only_when_full_and_regenerating() {
    // (max, current, stat 26, nomanaregen, freed)
    for (m, v, recovery, noregen, freed) in [
        (7500, 7500, 0, false, true),
        (7500, 8000, 0, false, true),
        (7500, 7499, 0, false, false),
        (7500, 7500, 0, true, false),
        (7500, 7500, -1, true, false),
    ] {
        let mut game = Game::new();
        let mut sys = system();
        let p = spawn(&mut game, &mut sys, UnitType::Player, player_mode::NU);
        set(&mut game, &mut sys, p, stat::MAXMANA, m);
        set(&mut game, &mut sys, p, stat::MANA, v);
        set(&mut game, &mut sys, p, stat::MANARECOVERY, recovery);
        if noregen {
            sys.stats.toggle_state(p, state::NOMANAREGEN, true);
        }
        state_list(&mut game, &mut sys, p, state::MANAPOT);
        regen_player(&mut game, &mut sys, p);
        let case = format!("{m} {v} {recovery} {noregen}");
        assert_eq!(!has_state_list(&sys, p, state::MANAPOT), freed, "{case}");
    }
}

/// Monster life (monster rules 4–6).
#[test]
fn monster_life_bounds() {
    // hp exactly 256 with r < 0 and no room: continues (only hp < 256
    // stops).
    let mut game = Game::new();
    let mut sys = system();
    let m = spawn(&mut game, &mut sys, UnitType::Monster, 1);
    set(&mut game, &mut sys, m, stat::MAXHP, 25600);
    set(&mut game, &mut sys, m, stat::HITPOINTS, 256);
    set(&mut game, &mut sys, m, stat::HPREGEN, -10);
    regen_monster(&mut game, &mut sys, m);
    assert_eq!(total(&sys, m, stat::HITPOINTS), 246);
    // Ending at 1: kept (only hp < 1 becomes 0).
    set(&mut game, &mut sys, m, stat::HITPOINTS, 300);
    set(&mut game, &mut sys, m, stat::HPREGEN, -299);
    regen_monster(&mut game, &mut sys, m);
    assert_eq!(total(&sys, m, stat::HITPOINTS), 1);
    assert!(sys.hooks.deaths.is_empty());
    // Below 1: 0, and the death.
    set(&mut game, &mut sys, m, stat::HITPOINTS, 300);
    set(&mut game, &mut sys, m, stat::HPREGEN, -1000);
    regen_monster(&mut game, &mut sys, m);
    assert_eq!(total(&sys, m, stat::HITPOINTS), 0);
    assert_eq!(sys.hooks.deaths, [m]);
}

/// Healing to exactly max keeps the regen event (only hp > m cancels);
/// a living monster with hp > 0 does not die.
#[test]
fn monster_heal_to_max_keeps_regen() {
    let mut game = Game::new();
    let mut sys = system();
    let m = spawn(&mut game, &mut sys, UnitType::Monster, 1);
    set(&mut game, &mut sys, m, stat::MAXHP, 25600);
    set(&mut game, &mut sys, m, stat::HITPOINTS, 25500);
    set(&mut game, &mut sys, m, stat::HPREGEN, 100);
    regen_monster(&mut game, &mut sys, m);
    assert_eq!(total(&sys, m, stat::HITPOINTS), 25600);
    assert_eq!(pending(&game, m), [(3, 1, 0, 0)]);
    assert!(sys.hooks.deaths.is_empty());
}

/// Death with `uninterruptable` (monster step 6): state 92 on,
/// which sets the disguise flag when 92 has the disguise flag.
#[test]
fn uninterruptable_death_sets_disguise() {
    let mut stats = (*crate::stats::tests::data()).clone();
    stats.states = StateTable::synthetic(185, &[(state::DEATH_DELAY, group::DISGUISE)]);
    let mut game = Game::new();
    let mut sys = system_with(Arc::new(stats));
    let m = spawn(&mut game, &mut sys, UnitType::Monster, 1);
    set(&mut game, &mut sys, m, stat::MAXHP, 25600);
    set(&mut game, &mut sys, m, stat::HITPOINTS, 300);
    set(&mut game, &mut sys, m, stat::HPREGEN, -1000);
    sys.stats.toggle_state(m, state::UNINTERRUPTABLE, true);
    regen_monster(&mut game, &mut sys, m);
    assert!(sys.stats.has_state(m, state::DEATH_DELAY));
    assert_ne!(sys.units.get(m).unwrap().flags2 & flags2::DISGUISE, 0);
    assert!(sys.hooks.deaths.is_empty());
}

// ---- neutral start ------------------------------------------------------------

/// Neutral start: the pending test uses the smallest
/// **positive** expire of the type-2 timers, and skips only when it is
/// > f.
#[test]
fn neutral_start_pending_ai_event() {
    // A type-2 timer at expire 0 does not count; one at 50 > f does.
    let mut game = Game::new();
    game.frame = -1;
    let mut sys = system();
    let m = spawn(&mut game, &mut sys, UnitType::Monster, 1);
    game.schedule_event(m, 2, 0, None, 0, 0).unwrap();
    game.schedule_event(m, 2, 50, None, 0, 0).unwrap();
    game.frame = 10;
    sys.with(&mut game, |sim, hooks| monster_neutral(sim, hooks, m))
        .unwrap();
    assert_eq!(pending(&game, m), [(2, 0, 0, 0), (2, 50, 0, 0)]);
    // Expire = f is not > f: event 2 at f + 15 (aidel 0 → 15).
    let mut game = Game::new();
    game.frame = 10;
    let mut sys = system();
    let m = spawn(&mut game, &mut sys, UnitType::Monster, 1);
    game.schedule_event(m, 2, 11, None, 0, 0).unwrap();
    game.frame = 11;
    sys.with(&mut game, |sim, hooks| monster_neutral(sim, hooks, m))
        .unwrap();
    assert_eq!(pending(&game, m), [(2, 11, 0, 0), (2, 26, 0, 0)]);
}

// ---- unit-order lists -----------------------------------------------------------------

/// A room in its act's room list is active.
#[test]
fn activated_room_is_active() {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let room = game.lists.create_room(0).unwrap();
    assert!(!game.lists.room(room).unwrap().is_active());
    game.lists.activate_room(room).unwrap();
    assert!(game.lists.room(room).unwrap().is_active());
}

/// `unit_mut` reaches the same entry as `unit`.
#[test]
fn unit_mut_is_the_unit_entry() {
    let mut game = Game::new();
    let u = game.spawn_unit(UnitType::Monster, None, false).unwrap();
    game.lists.unit_mut(u).unwrap().allied = true;
    assert!(game.lists.unit(u).unwrap().allied);
    assert!(game.lists.unit_mut(UnitId(99)).is_none());
}
