//! Skill gaps of the Paladin, Necromancer and Assassin in the play host
//! (q-skill-gaps), on the user's install (`real_rig`): a new character
//! of the class with its start kit leaves the camp (a monster in a town
//! room is no target and takes no damage), a zombie / corpse stands
//! beside, and the install's skill is used through C→S 0x0D / 0x0C.
//! Each test asserts an outcome (damage, state, pet or missile); the
//! skill ids, states and item-type sets are the install's `skills` rows
//! (q-fixture-migrate).

use crate::real_rig;

use d2_sim::units::{UnitId, UnitType};
use real_rig::{skill_row, Rig};

// The install's `skills` ids.
const AMPLIFY_DAMAGE: usize = 66;
const RAISE_SKELETON: usize = 70;
const CORPSE_EXPLOSION: usize = 74;
const REVIVE: usize = 95;
const SACRIFICE: usize = 96;
const SMITE: usize = 97;
const ZEAL: usize = 106;
/// `Fire Trauma` in `skills.txt` (shown as Fire Blast).
const FIRE_BLAST: usize = 251;
const TIGER_STRIKE: usize = 254;
const FISTS_OF_FIRE: usize = 259;

/// Body locations (`bodylocs`): right arm, left arm.
const RIGHT: u8 = 4;
const LEFT: u8 = 5;

/// A `class` character with `skills` learned, out of town and
/// strengthened (`Rig::strengthen`).
fn game(class: &str, skills: &[usize]) -> Rig {
    let mut r = Rig::new(class, skills);
    r.leave_town();
    r.strengthen();
    r
}

// (Smite with the paladin's start shield)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn smite_with_a_shield_hurts_the_monster() {
    let mut r = game("paladin", &[SMITE]);
    assert!(r.worn(LEFT).is_some(), "the start kit's shield");
    let m = r.spawn_monster(1);
    r.select_right(SMITE);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    assert!(low < life0, "Smite hurt it ({})", r.errors());
}

// (no shield: the item type test fails, the case "Smite, sword
// at 4, nothing at 5": `use_state` 2; Smite's row has `AttackNoMana`, so
// the use check uses Attack (skill 0) instead: no Smite, no mana spent)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn smite_without_a_shield_is_a_plain_attack() {
    assert!(
        skill_row(SMITE).attacknomana,
        "Smite's row has AttackNoMana"
    );
    let mut r = game("paladin", &[SMITE]);
    r.take_off(LEFT);
    assert!(r.worn(RIGHT).is_some(), "the start sword stays");
    let m = r.spawn_monster(1);
    r.select_right(SMITE);
    let mana0 = r.mana();
    r.right_click_unit(m);
    r.step(4);
    assert_eq!(r.used_skill(), Some(0), "Attack in use ({})", r.errors());
    r.step(26);
    assert_eq!(r.mana(), mana0, "no mana spent");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn sacrifice_hurts_the_monster_and_the_caster() {
    let mut r = game("paladin", &[SACRIFICE]);
    let p = r.player();
    let m = r.spawn_monster(1);
    r.select_right(SACRIFICE);
    let (life0, mine0) = (r.life(m), r.life(p));
    r.right_click_unit(m);
    let (mut low, mut mine) = (life0, mine0);
    for _ in 0..30 {
        r.step(1);
        low = low.min(r.life(m));
        mine = mine.min(r.life(p));
    }
    let errors = r.errors();
    assert!(low < life0, "the monster is hurt ({errors})");
    assert!(mine < mine0, "the caster is hurt ({errors})");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn zeal_strikes_the_monster() {
    let mut r = game("paladin", &[SACRIFICE, ZEAL]);
    let m = r.spawn_monster(1);
    r.select_right(ZEAL);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 60);
    assert!(low < life0, "Zeal hurt it ({})", r.errors());
}

/// A dead zombie (mode 12, no life) `dx` sub-tiles east of the player.
fn spawn_corpse(r: &mut Rig, dx: i32) -> UnitId {
    let m = r.spawn_monster(dx);
    r.with(move |sim, _| {
        sim.events.action.with(&mut sim.game, |_, v| {
            v.set_base(m, d2_sim::stats::stat::HITPOINTS, 0);
        });
        sim.events.action.sys.units.get_mut(m).unwrap().mode = 12;
    });
    m
}

fn monsters(r: &mut Rig) -> usize {
    r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Monster).len())
}

// (Amplify Damage: its `auratargetstate` on the monster)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_curse_puts_its_state_on_the_monster() {
    let state = skill_row(AMPLIFY_DAMAGE).auratargetstate;
    let mut r = game("necromancer", &[AMPLIFY_DAMAGE]);
    let m = r.spawn_monster(3);
    r.select_right(AMPLIFY_DAMAGE);
    assert!(!r.has_state(m, state));
    r.right_click_point(3, 0);
    r.step(30);
    assert!(r.has_state(m, state), "cursed ({})", r.errors());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn corpse_explosion_hurts_the_monsters_around_the_corpse() {
    let mut r = game("necromancer", &[CORPSE_EXPLOSION]);
    let corpse = spawn_corpse(&mut r, 2);
    let near = r.spawn_monster(3);
    r.select_right(CORPSE_EXPLOSION);
    let life0 = r.life(near);
    r.right_click_unit(corpse);
    let low = r.lowest_life(near, 30);
    assert!(low < life0, "the blast hurt it ({})", r.errors());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn raise_skeleton_turns_a_corpse_into_a_pet() {
    let mut r = game("necromancer", &[RAISE_SKELETON]);
    let corpse = spawn_corpse(&mut r, 2);
    let before = monsters(&mut r);
    r.select_right(RAISE_SKELETON);
    r.right_click_unit(corpse);
    r.step(30);
    assert!(
        r.pets() > 0,
        "a skeleton follows ({}, {before})",
        r.errors()
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn revive_stands_the_corpse_up_as_a_pet() {
    let mut r = game("necromancer", &[REVIVE]);
    let corpse = spawn_corpse(&mut r, 2);
    r.select_right(REVIVE);
    r.right_click_unit(corpse);
    r.step(30);
    assert!(r.pets() > 0, "a revived pet follows ({})", r.errors());
    let mode = r.with(move |sim, _| sim.events.action.sys.units.get(corpse).map(|u| u.mode));
    assert_ne!(mode, Some(12), "it stands up ({})", r.errors());
}

// (Tiger Strike with the start katar: a hit, and its `aurastate` charge)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn tiger_strike_with_a_claw_hurts_and_charges() {
    let charge = skill_row(TIGER_STRIKE).aurastate;
    let mut r = game("assassin", &[TIGER_STRIKE]);
    let me = r.player();
    let m = r.spawn_monster(1);
    r.select_right(TIGER_STRIKE);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    let errors = r.errors();
    assert!(low < life0, "the strike hurt it ({errors})");
    assert!(r.state_on(me, charge), "a charge ({errors})");
}

// (the weapon-type test; the case "Tiger Strike
// (`itypea1` `mele`), no hand item": pass, bare-handed)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn tiger_strike_bare_handed_strikes() {
    let charge = skill_row(TIGER_STRIKE).aurastate;
    let mut r = game("assassin", &[TIGER_STRIKE]);
    for loc in [RIGHT, LEFT] {
        if r.worn(loc).is_some() {
            r.take_off(loc);
        }
    }
    let me = r.player();
    let m = r.spawn_monster(1);
    r.select_right(TIGER_STRIKE);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    let errors = r.errors();
    assert!(low < life0, "the bare-handed strike ({errors})");
    assert!(r.state_on(me, charge), "a charge ({errors})");
}

// (Fists of Fire, srvdo 35, `itypea1` `h2h`: the katar strikes and its
// `aurastate` charges)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_charge_up_claw_skill_hurts_the_monster_and_charges() {
    let charge = skill_row(FISTS_OF_FIRE).aurastate;
    let mut r = game("assassin", &[FISTS_OF_FIRE]);
    let me = r.player();
    let m = r.spawn_monster(1);
    r.select_right(FISTS_OF_FIRE);
    let life0 = r.life(m);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 30);
    let errors = r.errors();
    assert!(low < life0, "the strike hurt it ({errors})");
    assert!(r.state_on(me, charge), "its charge ({errors})");
}

/// The most missiles alive at once while the skill runs.
fn missiles_seen(r: &mut Rig, ticks: usize) -> usize {
    let mut most = 0;
    for _ in 0..ticks {
        r.step(1);
        most = most.max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
    }
    most
}

// (the srvmissile path: `bomb in air`)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_blast_throws_a_missile() {
    let mut r = game("assassin", &[FIRE_BLAST]);
    r.select_right(FIRE_BLAST);
    r.right_click_point(8, 0);
    let most = missiles_seen(&mut r, 30);
    assert!(most > 0, "a missile flew ({})", r.errors());
}
