//! The skill button state of a new sorceress on the user's install, on the
//! play app's own game and client and the install's `skills` /
//! `skilldesc` tables: Fire Bolt learned with a skill point and selected
//! is usable (state 0) outside the town and red (state 1) inside it
//! (`skills.txt` `InTown` is 0 for Fire Bolt).
//! The playtest's red button outside the town did not reproduce here.

use crate::app_support;
use crate::real_rig;

use d2_client::app::hud::hud_tables;
use d2_client::bridge::BridgeResource;
use d2_client::ui::original::hud::{skill_button_state, HudTables};
use real_rig::{skill_named, Rig};

/// (skill, level, in town, state) of the right skill button.
fn right_button(r: &Rig, tables: &HudTables) -> (u16, i32, bool, u8) {
    let w = r.app.world().resource::<BridgeResource>().0.world();
    let key = w.local().expect("local player").key;
    let list = w.units[&key].skills.as_ref().expect("skill list");
    let e = list.right_entry().expect("right skill");
    let town = d2_client::bridge::modes::in_town(w, key);
    let level = e.base + e.level_bonus;
    let k = skill_button_state(tables, e.skill, level, town, (0, 0), (500, 500));
    (e.skill, level, town, k)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_learned_fire_bolt_is_red_in_town_only() {
    let fire_bolt = skill_named("Fire Bolt");
    let mut r = Rig::new("sorceress", &[]);
    r.give_skill_points(1);
    r.add_skill_point(fire_bolt);
    r.step(5);
    // C→S 0x3C: the skill without the left bit, then the item (none).
    let mut sel = vec![0x3C];
    sel.extend_from_slice(&(fire_bolt as u32).to_le_bytes());
    sel.extend_from_slice(&u32::MAX.to_le_bytes());
    r.send(&sel);
    r.step(10);
    let tables = hud_tables(app_support::live().archives.as_ref()).unwrap();
    let (skill, level, town, k) = right_button(&r, &tables);
    eprintln!("in town: skill {skill} level {level} town {town} state {k}");
    assert_eq!((usize::from(skill), level, town), (fire_bolt, 1, true));
    assert_eq!(k, 1, "Fire Bolt lacks InTown: red in town");
    r.leave_town();
    r.step(10);
    let (skill, level, town, k) = right_button(&r, &tables);
    eprintln!("outside: skill {skill} level {level} town {town} state {k}");
    assert_eq!((usize::from(skill), level, town), (fire_bolt, 1, false));
    assert_eq!(k, 0, "usable outside town");
}
