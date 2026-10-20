//! Paladin skills in the play preview, headless on the user's install
//! (`real_rig`): a paladin joins, walks out of the Rogue Encampment and
//! casts through C→S 0x0C. The skills are the install's `skills` rows,
//! found by class and body function (`srvdofunc`,
//! `tables/skills/functions.tsv`); every expected number is read from
//! those rows or from the server.

use crate::real_rig;

use d2_client::bridge::BridgeResource;
use d2_data::tables::Skills;
use d2_sim::units::UnitType;
use real_rig::Rig;

use crate::app_support;

fn rows(f: impl Fn(&Skills) -> bool) -> Vec<Skills> {
    let class_id = d2_client::app::single_player::parse_class("paladin").unwrap();
    let all: Vec<Skills> = app_support::live().tables.rows().expect("skills");
    let mut rows: Vec<Skills> = all
        .into_iter()
        .filter(|r| r.charclass == class_id && f(r))
        .collect();
    rows.sort_by_key(|r| (r.reqlevel, r.skill));
    rows
}

/// The one Paladin skill whose body is `dofunc`.
fn only(dofunc: u16) -> Skills {
    let r = rows(|r| r.srvdofunc == dofunc);
    assert_eq!(r.len(), 1, "one paladin skill runs srvdofunc {dofunc}");
    r.into_iter().next().unwrap()
}

fn paladin(skill: usize) -> Rig {
    let mut r = Rig::new("paladin", &[skill]);
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

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn holy_bolt_costs_mana_and_creates_the_missile() {
    // The Paladin's missile skill with no body function of its own: its
    // `srvmissile` (a link, `0xFFFF` = none) is the whole effect.
    let bolt = rows(|r| r.srvdofunc == 0 && r.srvmissile != 0xFFFF && r.mana > 0);
    assert_eq!(bolt.len(), 1, "one paladin missile skill without a do step");
    let mut r = paladin(usize::from(bolt[0].skill));
    let mana0 = r.mana();
    let mark = r.s2c_mark();
    r.right_click_point(6, 0);
    let server = missiles_seen(&mut r, 40);
    let errors = r.errors();
    assert!(r.mana() < mana0, "mana spent; errors: {errors}");
    assert!(server > 0, "the server made the missile; errors: {errors}");
    assert!(r.s2c_contains_since(mark, 0x4D), "{errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn blessed_hammer_spawns_its_missile_through_the_do_step() {
    let hammer = only(73);
    assert!(hammer.mana > 0, "Blessed Hammer costs mana");
    let mut r = paladin(usize::from(hammer.skill));
    let mana0 = r.mana();
    let mark = r.s2c_mark();
    r.right_click_point(6, 0);
    let server = missiles_seen(&mut r, 40);
    let errors = r.errors();
    assert!(r.mana() < mana0, "mana spent; errors: {errors}");
    assert!(
        server > 0,
        "the hammer exists on the server; errors: {errors}"
    );
    assert!(r.s2c_contains_since(mark, 0x4D), "{errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn might_sets_its_state_and_the_client_hears_of_it() {
    // Might is the Paladin's first aura row (level 1, no mana; `srvdofunc`
    // 65 is every aura's body).
    let might = rows(|r| r.srvdofunc == 65 && r.aura).remove(0);
    let state = might.aurastate;
    assert_ne!(state, 0xFFFF, "an aura row names its state");
    let mut r = paladin(usize::from(might.skill));
    let me = r.player();
    assert!(!r.has_state(me, state), "no state before");
    // An aura is switched on by selecting it (C→S 0x3C, item −1; the
    // `immediate` auras run their do step at once), not by a click.
    let mut select = vec![0x3C];
    select.extend(u32::from(might.skill).to_le_bytes());
    select.extend(u32::MAX.to_le_bytes());
    r.send_for_fate(&select);
    r.step(40);
    let errors = r.errors();
    assert!(
        r.has_state(me, state),
        "the aura state is on; errors: {errors}"
    );
    let w = r.app.world().resource::<BridgeResource>().0.world();
    let heard = w.local().is_some_and(|p| p.states.contains(&(state as u8)));
    assert!(heard, "the client heard of the state; errors: {errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn charge_at_a_point_moves_the_player() {
    let charge = only(67);
    let mut r = paladin(usize::from(charge.skill));
    let from = r.pos();
    r.right_click_point(6, 0);
    r.step(60);
    let to = r.pos();
    let errors = r.errors();
    assert_eq!(errors, "[]", "no server error");
    assert!(to.0 > from.0, "the player charged east: {from:?} -> {to:?}");
}
