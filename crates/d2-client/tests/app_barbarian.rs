//! The Barbarian's skills in the play preview, headless on the user's
//! install (`real_rig`): a barbarian joins, walks out of the Rogue
//! Encampment into the Blood Moor (a monster in a town room is not a
//! legal target of Bash) and casts through
//! C→S 0x0D / 0x0C. Skill ids and every expected number are the
//! install's `skills` rows; the kicks are the Assassin's (an assassin
//! casts them). Some expected values are provisional (unverified).

use crate::real_rig;

use real_rig::{skill_row, Rig};

/// The install's `skills.txt` `Id`s.
const BASH: usize = 126;
const LEAP: usize = 132;
const DOUBLE_SWING: usize = 133;
const SHOUT: usize = 138;
const LEAP_ATTACK: usize = 143;
const BATTLE_ORDERS: usize = 149;
const WHIRLWIND: usize = 151;
const NATURAL_RESISTANCE: usize = 153;
const DRAGON_TALON: usize = 255;
const DRAGON_FLIGHT: usize = 275;

/// Body location (`bodylocs`) of the left arm.
const LEFT: u8 = 5;

fn barbarian(skills: &[usize]) -> Rig {
    let mut r = Rig::new("barbarian", skills);
    r.leave_town();
    r.strengthen();
    r
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn bash_on_a_monster_costs_mana_and_hurts_it() {
    let mut r = barbarian(&[BASH]);
    let m = r.spawn_monster(1);
    r.select_right(BASH);
    let (mana0, life0) = (r.mana(), r.life(m));
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    let errors = r.errors();
    assert!(r.mana() < mana0, "Bash spent mana ({errors})");
    assert!(low < life0, "Bash hurt the monster ({errors})");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn shout_and_battle_orders_put_their_state_on_the_barbarian() {
    let mut r = barbarian(&[SHOUT, BATTLE_ORDERS]);
    let me = r.player();
    for skill in [SHOUT, BATTLE_ORDERS] {
        // The warcry's state: its row's `aurastate`.
        let state = skill_row(skill).aurastate;
        r.select_right(skill);
        let mana0 = r.mana();
        assert!(!r.has_state(me, state), "no state before");
        r.right_click_point(8, 0);
        r.step(30);
        let errors = r.errors();
        assert!(
            r.has_state(me, state),
            "skill {skill}: state {state} ({errors})"
        );
        // Battle Orders' own state raises current mana with the maximum
        // (its `aurastat` max mana %), so only Shout's cost shows in the
        // total.
        if skill == SHOUT {
            assert!(r.mana() < mana0, "skill {skill} spent mana ({errors})");
        }
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn double_swing_hits_two_monsters_in_two_swings() {
    let mut r = barbarian(&[DOUBLE_SWING]);
    // Double Swing's sets a and b are both `mele`: the
    // start kit's buckler leaves the left hand for a second hand axe
    // (staging; with one weapon its `AttackNoMana` makes it an Attack).
    if r.worn(LEFT).is_some() {
        r.take_off(LEFT);
    }
    r.wear(*b"hax ", LEFT);
    let (a, b) = (r.spawn_monster(1), r.spawn_monster(2));
    r.select_right(DOUBLE_SWING);
    let (mana0, la, lb) = (r.mana(), r.life(a), r.life(b));
    r.right_click_unit(a);
    let (mut lowa, mut lowb) = (la, lb);
    for _ in 0..30 {
        r.step(1);
        lowa = lowa.min(r.life(a));
        lowb = lowb.min(r.life(b));
    }
    let errors = r.errors();
    assert!(r.mana() < mana0, "Double Swing spent mana ({errors})");
    assert!(lowa < la, "the first swing hurt the target ({errors})");
    assert!(
        lowb < lb,
        "the second swing hurt the next monster ({errors})"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_learned_mastery_puts_its_passive_stats_in_its_state() {
    // Learning it needs its row's prerequisite (`reqskill1`, Iron Skin)
    // and level 30 (`reqlevel`; the rig stages 30).
    let req = usize::from(skill_row(NATURAL_RESISTANCE).reqskill1);
    let mut r = barbarian(&[req]);
    // The passive's state and first stat, and its level-1 value: the
    // install's row is `dm12` on (Param1, Param2) = (0, 80), the
    // diminishing curve (q = 110·1 / 7, a + q·(b − a) / 100).
    let row = skill_row(NATURAL_RESISTANCE);
    let (state, st) = (row.passivestate, row.passivestat1);
    let (a, b) = (row.param1 as i32, row.param2 as i32);
    let want = a + (110 / 7) * (b - a) / 100;
    r.give_skill_points(1);
    assert_eq!(r.state_stat(state, st), None, "not learned");
    r.add_skill_point(NATURAL_RESISTANCE);
    r.step(5);
    let errors = r.errors();
    assert_eq!(
        r.state_stat(state, st),
        Some(want),
        "the passive state holds passivestat1 = passivecalc1 ({errors})"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn leap_spends_mana_and_moves_the_barbarian() {
    let mut r = barbarian(&[LEAP]);
    r.select_right(LEAP);
    let (mana0, from) = (
        r.mana(),
        r.with(|sim, p| sim.events.action.sys.hooks.path_position(p)),
    );
    r.right_click_point(5, 0);
    r.step(40);
    let to = r.with(|sim, p| sim.events.action.sys.hooks.path_position(p));
    let errors = r.errors();
    assert!(r.mana() < mana0, "Leap spent mana ({errors})");
    assert!(
        to.0 > from.0,
        "the barbarian leapt east: {from:?} -> {to:?}"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn whirlwind_spends_mana_and_moves_the_barbarian() {
    let mut r = barbarian(&[WHIRLWIND]);
    r.select_right(WHIRLWIND);
    let (mana0, from) = (
        r.mana(),
        r.with(|sim, p| sim.events.action.sys.hooks.path_position(p)),
    );
    r.right_click_point(8, 0);
    r.step(40);
    let to = r.with(|sim, p| sim.events.action.sys.hooks.path_position(p));
    let errors = r.errors();
    assert!(r.mana() < mana0, "Whirlwind spent mana ({errors})");
    assert!(
        to.0 > from.0,
        "the barbarian whirled east: {from:?} -> {to:?}"
    );
}

// The landing of a player's Leap is a knockback (result 9) with a zeroed
// damage record: the barbarian lands exactly on the
// aimed point and the monster standing beside it loses no life.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn leap_lands_on_the_aimed_point_and_deals_no_damage() {
    let mut r = barbarian(&[LEAP]);
    r.select_right(LEAP);
    // Beside the aimed point (a zombie's footprint on the point itself
    // would end the leap short of it).
    let m = r.spawn_monster_at(4, 2);
    let (life0, from) = (
        r.life(m),
        r.with(|sim, p| sim.events.action.sys.hooks.path_position(p)),
    );
    r.right_click_point(5, 0);
    let low = r.lowest_life(m, 40);
    let to = r.with(|sim, p| sim.events.action.sys.hooks.path_position(p));
    let errors = r.errors();
    assert_eq!(
        to,
        (from.0 + 5, from.1),
        "landed on the aimed point ({errors})"
    );
    assert_eq!(low, life0, "the landing is a knockback only");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn leap_attack_leaps_to_the_monster_and_strikes_it() {
    let mut r = barbarian(&[LEAP_ATTACK]);
    r.select_right(LEAP_ATTACK);
    let m = r.spawn_monster(6);
    let (mana0, life0) = (r.mana(), r.life(m));
    r.right_click_unit(m);
    let low = r.lowest_life(m, 60);
    let errors = r.errors();
    assert!(r.mana() < mana0, "Leap Attack spent mana ({errors})");
    assert!(low < life0, "the strike hurt the monster ({errors})");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dragon_talon_kick_hurts_a_monster() {
    let mut r = Rig::new("assassin", &[DRAGON_TALON]);
    r.leave_town();
    r.strengthen();
    r.select_right(DRAGON_TALON);
    let m = r.spawn_monster(1);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    let errors = r.errors();
    assert!(low < life0, "the kick hurt the monster ({errors})");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dragon_flight_kick_hurts_a_monster() {
    let mut r = Rig::new("assassin", &[DRAGON_FLIGHT]);
    r.leave_town();
    r.strengthen();
    r.select_right(DRAGON_FLIGHT);
    let m = r.spawn_monster(1);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    let errors = r.errors();
    assert!(low < life0, "the kick hurt the monster ({errors})");
}
