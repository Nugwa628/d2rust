//! The Necromancer's right-hand skills in the play preview, headless on
//! the user's install (`real_rig`): a necromancer joins, walks out of the
//! Rogue Encampment and casts through C→S 0x0C (the right skill at a
//! point) → the server's skill use → mana → the missile / state / pet in
//! the server's store → the client's model of it. The skills are the
//! install's `skills` rows, found by class and body function (the
//! `srvdofunc` of the body under test, `tables/skills/functions.tsv`);
//! every expected number is read from those rows or from the server.

use crate::real_rig;

use d2_client::bridge::BridgeResource;
use d2_data::tables::Skills;
use d2_sim::units::UnitType;
use real_rig::Rig;

use crate::app_support;

/// The class's skill rows that run the body `dofunc`, lowest `reqlevel`
/// first (the level-1 skill of the body is the one under test).
fn rows_of(class: &str, dofunc: u16) -> Vec<Skills> {
    let class_id = d2_client::app::single_player::parse_class(class).unwrap();
    let all: Vec<Skills> = app_support::live().tables.rows().expect("skills");
    let mut rows: Vec<Skills> = all
        .into_iter()
        .filter(|r| r.charclass == class_id && r.srvdofunc == dofunc)
        .collect();
    rows.sort_by_key(|r| (r.reqlevel, r.skill));
    rows
}

/// The one skill of the class whose body is `dofunc`.
fn only(class: &str, dofunc: u16) -> Skills {
    let rows = rows_of(class, dofunc);
    assert_eq!(rows.len(), 1, "one {class} skill runs srvdofunc {dofunc}");
    rows.into_iter().next().unwrap()
}

fn necromancer(skill: usize) -> Rig {
    let mut r = Rig::new("necromancer", &[skill]);
    r.leave_town();
    r.strengthen();
    r.select_right(skill);
    r
}

/// The most missiles alive on the server over `frames` frames.
fn missiles_seen(r: &mut Rig, frames: usize) -> usize {
    let mut most = 0;
    for _ in 0..frames {
        r.step(1);
        most = most.max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
    }
    most
}

fn monsters(r: &mut Rig) -> usize {
    r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Monster).len())
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn teeth_spends_mana_and_makes_missiles() {
    let row = only("necromancer", 8);
    assert!(row.mana > 0, "Teeth costs mana");
    let mut r = necromancer(usize::from(row.skill));
    let mana0 = r.mana();
    let mark = r.s2c_mark();
    r.right_click_point(6, 0);
    let server = missiles_seen(&mut r, 40);
    let errors = r.errors();
    assert!(r.mana() < mana0, "mana spent; {errors}");
    assert!(server > 0, "teeth made missiles; {errors}");
    assert!(r.s2c_contains_since(mark, 0x4D), "{errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn poison_nova_makes_a_ring_of_missiles() {
    let row = only("necromancer", 22);
    assert!(row.mana > 0, "Poison Nova costs mana");
    let mut r = necromancer(usize::from(row.skill));
    let mana0 = r.mana();
    r.right_click_point(0, 0);
    let most = missiles_seen(&mut r, 40);
    let errors = r.errors();
    assert!(r.mana() < mana0, "mana spent; {errors}");
    assert!(most > 1, "the ring made missiles: {most}; {errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn bone_armor_turns_its_state_on() {
    let row = only("necromancer", 18);
    let state = row.aurastate;
    assert!(row.mana > 0, "Bone Armor costs mana");
    let mut r = necromancer(usize::from(row.skill));
    let me = r.player();
    assert!(!r.has_state(me, state), "no state before");
    let mana0 = r.mana();
    r.right_click_point(0, 0);
    r.step(40);
    let errors = r.errors();
    assert!(r.mana() < mana0, "mana spent; {errors}");
    assert!(r.has_state(me, state), "state on; {errors}");
    let w = r.app.world().resource::<BridgeResource>().0.world();
    let on = w.local().is_some_and(|p| p.states.contains(&(state as u8)));
    assert!(on, "the client heard of the state: {errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn clay_golem_summons_a_pet() {
    // Clay, Blood and Fire Golem share the body: the lowest `reqlevel`.
    let row = rows_of("necromancer", 56).remove(0);
    assert!(row.mana > 0, "a golem costs mana");
    let mut r = necromancer(usize::from(row.skill));
    let (before, mana0) = (monsters(&mut r), r.mana());
    r.right_click_point(2, 0);
    r.step(40);
    let errors = r.errors();
    assert!(r.mana() < mana0, "mana spent; {errors}");
    assert_eq!(monsters(&mut r), before + 1, "a golem exists; {errors}");
    assert!(r.pets() > 0, "the client heard of the pet; {errors}");
}
