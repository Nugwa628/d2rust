//! Assassin skills in the play host (q-assassin), headless, on the user's
//! install (`real_rig`): C→S 0x0C / 0x0D (the right skill at a point or a
//! monster) → the start check → the cast mode → the do step, in the Blood
//! Moor. Skill ids are looked up by name in the install's `skills.txt`;
//! the states are the rows' own `aurastate` (q-fixture-migrate). The
//! charge, finisher and trap bodies are also checked in
//! `app_assassin_gaps.rs` (the finisher's charge and damage, the trap's
//! shots); the tests here are the cast paths.

use crate::real_rig;

use real_rig::{skill_named, skill_row, Rig};

fn assassin(names: &[&str]) -> (Rig, Vec<usize>) {
    let ids: Vec<usize> = names.iter().map(|n| skill_named(n)).collect();
    let mut r = Rig::new("assassin", &ids);
    r.leave_town();
    r.strengthen();
    r.select_right(ids[0]);
    (r, ids)
}

/// A monster that survives the strengthened player.
fn tough(r: &mut Rig, m: d2_sim::units::UnitId) {
    r.with(move |sim, _| {
        sim.events.action.with(&mut sim.game, |_, v| {
            v.set_base(m, d2_sim::stats::stat::MAXHP, 100_000 << 8);
            v.set_base(m, d2_sim::stats::stat::HITPOINTS, 100_000 << 8);
        });
    });
}

// (the pet add reaches the client: the laid trap is in its pet list)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_sentry_trap_is_laid_and_listed_as_a_pet() {
    let (mut r, _) = assassin(&["Charged Bolt Sentry"]);
    let before = r.pets();
    r.right_click_point(5, 0);
    r.step(40);
    let errors = r.errors();
    assert_eq!(r.pets(), before + 1, "pet add; errors: {errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn shadow_warrior_summons_the_shadow() {
    let (mut r, _) = assassin(&["Shadow Warrior"]);
    let before = r.pets();
    r.right_click_point(5, 0);
    r.step(40);
    let errors = r.errors();
    assert_eq!(r.pets(), before + 1, "pet add; errors: {errors}");
}

// (a hit adds a charge: the row's `aurastate` is on the assassin)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn tiger_strike_adds_a_charge_on_a_hit() {
    let (mut r, ids) = assassin(&["Tiger Strike"]);
    let charge = skill_row(ids[0]).aurastate;
    let me = r.player();
    let m = r.spawn_monster(1);
    tough(&mut r, m);
    assert!(!r.state_on(me, charge), "no charge before");
    r.right_click_unit(m);
    r.step(30);
    let errors = r.errors();
    assert!(r.state_on(me, charge), "a charge; errors: {errors}");
}

// (Burst of Speed is the row named Quickness)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn burst_of_speed_turns_its_state_on() {
    let (mut r, ids) = assassin(&["Quickness"]);
    let state = skill_row(ids[0]).aurastate;
    let me = r.player();
    assert!(!r.state_on(me, state), "off before");
    r.right_click_point(0, 0);
    r.step(30);
    let errors = r.errors();
    assert!(r.state_on(me, state), "state on; errors: {errors}");
}

// (asserts the charge and no fault, not the finisher damage: that is
// `app_assassin_gaps::a_finisher_spends_the_charge`)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_finisher_after_a_charge_runs_without_faults() {
    let (mut r, ids) = assassin(&["Tiger Strike", "Dragon Talon"]);
    let charge = skill_row(ids[0]).aurastate;
    let me = r.player();
    let m = r.spawn_monster(1);
    tough(&mut r, m);
    r.right_click_unit(m);
    r.step(30);
    assert!(r.state_on(me, charge), "charged; {}", r.errors());
    r.select_right(ids[1]);
    r.right_click_unit(m);
    r.step(30);
    assert!(r.errors() == "[]", "no faults: {}", r.errors());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dragon_flight_moves_the_assassin_to_the_monster() {
    let (mut r, _) = assassin(&["Dragon Flight"]);
    let m = r.spawn_monster(8);
    tough(&mut r, m);
    let from = r.pos();
    r.right_click_unit(m);
    r.step(40);
    let to = r.pos();
    let errors = r.errors();
    assert!(to.0 > from.0, "flew east: {from:?} -> {to:?}; {errors}");
}
