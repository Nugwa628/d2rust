//! A bow skill and a spear skill in the play host (q-amazon), headless,
//! on the user's install (`real_rig`): C→S 0x0C / 0x0D → the item and
//! ammo start check reads the weapon in use and the quiver through the
//! app's weapon seams ([`d2_client::app::weapons`]) → the arrow is taken
//! → the missile in the server's store. The amazon's start kit (a
//! javelin and a buckler) is the install's; the bow, the quiver and the
//! spear are the install's item rows, worn with `Rig::wear` (their codes
//! looked up by name in `weapons.txt` / `misc.txt`).

use crate::real_rig;

use d2_sim::units::{UnitId, UnitType};
use real_rig::{skill_named, skill_row, ExcelTable, Rig};

/// Body locations (`bodylocs`): right arm, left arm.
const RIGHT: u8 = 4;
const LEFT: u8 = 5;

/// The `code` of the item named `name` in the install's `file`.
fn item_code(file: &str, name: &str) -> [u8; 4] {
    let t = ExcelTable::load(file);
    let code = t.cell(t.row(name), "code").to_owned();
    let mut c = *b"    ";
    c[..code.len()].copy_from_slice(code.as_bytes());
    c
}

/// The amazon with `skills` learned in the Blood Moor, both hands
/// emptied of the start kit.
fn amazon(skills: &[&str]) -> Rig {
    let ids: Vec<usize> = skills.iter().map(|n| skill_named(n)).collect();
    let mut r = Rig::new("amazon", &ids);
    r.leave_town();
    r.strengthen();
    r.take_off(RIGHT);
    r.take_off(LEFT);
    r.select_right(ids[0]);
    r
}

/// The quiver unit: the left hand of the app's weapon copy.
fn quiver(r: &mut Rig) -> Option<UnitId> {
    r.with(|sim, p| sim.events.action.sys.hooks.x.weapons.hands.get(&p)?.left)
}

fn arrows(r: &mut Rig, q: UnitId) -> i32 {
    r.stat_of(q, 70)
}

/// A short bow in the right hand, and a quiver of the install's arrows in
/// the left hand holding `count` arrows.
fn bow_and_arrows(r: &mut Rig, count: i32) -> UnitId {
    r.wear(item_code("weapons.txt", "Short Bow"), RIGHT);
    r.wear(item_code("misc.txt", "Arrows"), LEFT);
    let q = quiver(r).expect("the quiver is in the left hand");
    r.with(move |sim, _| {
        sim.events
            .action
            .with(&mut sim.game, |_, v| v.set_base(q, 70, count));
    });
    q
}

/// Right-clicks a point east of the player; the most server missiles
/// alive in a frame over the next frames.
fn shoot(r: &mut Rig) -> usize {
    r.right_click_point(6, 0);
    let mut most = 0;
    for _ in 0..30 {
        r.step(1);
        most = most.max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
    }
    most
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_bow_skill_takes_an_arrow_and_shoots_the_missile() {
    let mut r = amazon(&["Fire Arrow"]);
    let q = bow_and_arrows(&mut r, 30);
    let mana0 = r.mana();
    let mark = r.s2c_mark();
    let most = shoot(&mut r);
    let errors = r.errors();
    // Before: the ammo check found no weapon (`current_weapon` was a
    // default), so the start refused and no missile existed.
    assert!(most > 0, "the arrow flew; errors: {errors}");
    assert_eq!(arrows(&mut r, q), 29, "one arrow taken; errors: {errors}");
    assert!(r.mana() < mana0, "mana spent at the start");
    assert!(r.s2c_contains_since(mark, 0x4D), "{errors}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_bow_skill_without_arrows_does_not_start() {
    let mut r = amazon(&["Fire Arrow"]);
    let q = bow_and_arrows(&mut r, 0);
    let mana0 = r.mana();
    let most = shoot(&mut r);
    let errors = r.errors();
    assert_eq!(most, 0, "no arrow, no missile; errors: {errors}");
    assert_eq!(arrows(&mut r, q), 0);
    assert_eq!(r.mana(), mana0, "no mana spent");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_bow_skill_with_bare_hands_does_not_start() {
    let mut r = amazon(&["Fire Arrow"]);
    let mana0 = r.mana();
    let most = shoot(&mut r);
    assert_eq!(most, 0);
    assert_eq!(r.mana(), mana0);
}

// No rule claim: the Jab body is not checked here.
// This checks the cast path of a unit-target skill: the start function
// (`srvst 5`) reads the kept target of a targetable monster and the
// spear its row wants (`itypea1` spea); the row's own `anim` mode (the
// sequence mode, `SQ`) runs and ends (in the field, neutral is mode 1
// where the camp's was 5).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn jab_on_a_monster_starts_the_attack_mode_and_ends_it() {
    let mut r = amazon(&["Jab"]);
    r.wear(item_code("weapons.txt", "Spear"), RIGHT);
    let m = r.spawn_monster(1);
    r.with(move |sim, _| {
        sim.events.action.with(&mut sim.game, |_, v| {
            v.set_base(m, d2_sim::stats::stat::MAXHP, 100_000 << 8);
            v.set_base(m, d2_sim::stats::stat::HITPOINTS, 100_000 << 8);
        });
    });
    r.right_click_unit(m);
    let mut seen = vec![];
    for _ in 0..40 {
        r.step(1);
        let mode = r.with(|sim, p| sim.events.action.sys.units.get(p).map_or(0, |u| u.mode));
        seen.push(mode);
    }
    let errors = r.errors();
    let jab = skill_named("Jab");
    let mode = u32::from(skill_row(jab).anim);
    assert!(
        seen.contains(&mode),
        "the row's mode ran: {seen:?} {errors}"
    );
    assert_eq!(seen.last(), Some(&1), "and ended in neutral: {seen:?}");
    assert_eq!(r.used_skill(), Some(jab as i32));
}
