//! The Druid's skills in the play host, headless, on the user's install
//! (`real_rig`): C→S 0x0C → the right skill's do step (srvdo 117
//! Firestorm, 118 Tornado, 116 Wearwolf) in the Blood Moor. The skill
//! ids are looked up by name in the install's `skills.txt`; the number of
//! missiles and the shape state are that skill's own row (`calc1`,
//! `Param1`, `aurastate`), nothing is typed in (q-fixture-migrate).

use crate::real_rig;

use d2_sim::units::UnitType;
use real_rig::{skill_named, skill_row, ExcelTable, Rig};

fn druid(skill: usize) -> Rig {
    let mut r = Rig::new("druid", &[skill]);
    r.leave_town();
    r.strengthen();
    r.select_right(skill);
    r
}

/// `eval(calc1)` of the skill at level 1: a constant, or
/// `ln12` = `Param1` + (L - 1) * `Param2`.
fn calc1_at_level_1(name: &str) -> usize {
    let t = ExcelTable::load("skills.txt");
    let row = t.row(name);
    let calc = t.cell(row, "calc1");
    let n = if calc == "ln12" {
        t.cell(row, "Param1")
    } else {
        calc
    };
    n.parse()
        .unwrap_or_else(|_| panic!("{name}: calc1 {calc:?} is a count at level 1"))
}

/// Casts the skill at a point east of the player; the most server
/// missiles alive in a frame over the next frames, and the mana spent.
fn cast_makes_missiles(name: &str) -> usize {
    let skill = skill_named(name);
    let mut r = druid(skill);
    let mana0 = r.mana();
    r.right_click_point(6, 0);
    let mut most = 0;
    for _ in 0..30 {
        r.step(1);
        most = most.max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
    }
    let errors = r.errors();
    assert!(r.mana() < mana0, "{name}: mana spent; errors: {errors}");
    most
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn firestorm_fans_out_missiles() {
    let n = calc1_at_level_1("Firestorm");
    assert!(n > 1, "Firestorm's row is a fan ({n})");
    assert!(cast_makes_missiles("Firestorm") >= n);
}

// (the install's Tornado row has `calc1` 1 at every level: the fan is
// one missile; the check is that a cast makes `calc1` missiles)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn tornado_fans_out_missiles() {
    let n = calc1_at_level_1("Tornado");
    assert!(cast_makes_missiles("Tornado") >= n);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn werewolf_turns_the_player_into_the_shape_and_the_seam_sees_it() {
    let skill = skill_named("Wearwolf");
    let wolf = skill_row(skill).aurastate;
    let mut r = druid(skill);
    let me = r.player();
    assert!(!r.has_state(me, wolf), "human before");
    r.right_click_point(1, 0);
    r.step(30);
    let errors = r.errors();
    let shifted = r.with(|sim, p| sim.events.action.sys.hooks.x.skills.shifted.contains(&p));
    // Before: `shapeshifted` was always false (no form was ever seen).
    assert!(r.state_on(me, wolf), "the state is on; errors: {errors}");
    assert!(shifted, "the use pipeline sees the form");
}
