//! Assassin gaps in the play host (q-assassin-gaps), on the user's
//! install (`real_rig`): a finisher spends the charges, Dragon Claw
//! wants a claw in each hand, and a laid sentry shoots its skill at a
//! monster in range until its shots are spent. Skills, states, the
//! sentry's monster class and its shots are the install's rows
//! (q-fixture-migrate).

use crate::real_rig;

use d2_sim::units::{UnitId, UnitType};
use real_rig::{skill_row, Rig};

// The install's `skills` ids.
const TIGER_STRIKE: usize = 254;
const DRAGON_TALON: usize = 255;
const DRAGON_CLAW: usize = 260;
const CHARGED_BOLT_SENTRY: usize = 261;
const LIGHTNING_SENTRY: usize = 271;

/// Body locations (`bodylocs`): right arm, left arm.
const RIGHT: u8 = 4;
const LEFT: u8 = 5;
/// The assassin's start claw (`weapons` code `ktr`).
const KATAR: [u8; 4] = *b"ktr ";

fn assassin(skills: &[usize]) -> Rig {
    let mut r = Rig::new("assassin", skills);
    r.leave_town();
    r.strengthen();
    r
}

/// A monster that survives the strengthened player.
fn tough(r: &mut Rig, m: UnitId) {
    r.with(move |sim, _| {
        sim.events.action.with(&mut sim.game, |_, v| {
            v.set_base(m, d2_sim::stats::stat::MAXHP, 100_000 << 8);
            v.set_base(m, d2_sim::stats::stat::HITPOINTS, 100_000 << 8);
        });
    });
}

// (Tiger Strike charges; the Dragon Talon kick finishes: it hurts and
// the charge state is removed)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_finisher_spends_the_charge() {
    let charge = skill_row(TIGER_STRIKE).aurastate;
    let mut r = assassin(&[TIGER_STRIKE, DRAGON_TALON]);
    let me = r.player();
    let m = r.spawn_monster(1);
    tough(&mut r, m);
    r.select_right(TIGER_STRIKE);
    r.right_click_unit(m);
    r.step(30);
    assert!(r.state_on(me, charge), "charged ({})", r.errors());
    let life0 = r.life(m);
    r.select_right(DRAGON_TALON);
    r.right_click_unit(m);
    let low = r.lowest_life(m, 40);
    assert!(low < life0, "the finisher hurt it ({})", r.errors());
    assert!(
        !r.state_on(me, charge),
        "the charge is spent ({})",
        r.errors()
    );
}

/// One Dragon Claw request on a zombie, with a second katar in the left
/// hand when `both`: the used skill a few frames on and the damage.
fn dragon_claw(both: bool) -> (Option<i32>, i32) {
    let mut r = assassin(&[DRAGON_TALON, DRAGON_CLAW]);
    assert_eq!(r.worn(RIGHT), Some(KATAR), "the start katar");
    if r.worn(LEFT).is_some() {
        r.take_off(LEFT);
    }
    if both {
        r.wear(KATAR, LEFT);
    }
    let m = r.spawn_monster(1);
    tough(&mut r, m);
    r.select_right(DRAGON_CLAW);
    let life0 = r.life(m);
    r.right_click_unit(m);
    r.step(4);
    let used = r.used_skill();
    (used, life0 - r.lowest_life(m, 36))
}

// (Dragon Claw's sets a and b are both `h2h`: one claw fails set b, as
// the case "Double Swing, one sword at 4", and its row's
// `AttackNoMana` makes the request an Attack; two claws use
// Dragon Claw and strike)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn dragon_claw_needs_a_claw_in_each_hand() {
    assert!(skill_row(DRAGON_CLAW).attacknomana);
    let (used, _) = dragon_claw(false);
    assert_eq!(used, Some(0), "one claw: an Attack");
    let (used, hurt) = dragon_claw(true);
    assert_eq!(used, Some(DRAGON_CLAW as i32), "two claws: Dragon Claw");
    assert!(hurt > 0, "two claws strike");
}

/// monstats `AI` AssassinSentry: the laid traps.
const AI_ASSASSIN_SENTRY: u16 = 101;

/// The living laid traps: monsters of AssassinSentry classes, not dying
/// or dead.
fn traps(r: &mut Rig) -> Vec<UnitId> {
    r.with(|sim, _| {
        let s = &sim.events.action.sys;
        sim.game
            .lists
            .units_of_type(UnitType::Monster)
            .into_iter()
            .filter(|&m| {
                s.units.get(m).is_some_and(|u| {
                    !u.is_dead()
                        && s.hooks
                            .tables
                            .combat
                            .monstats
                            .get(u.class as usize)
                            .is_some_and(|c| c.ai == AI_ASSASSIN_SENTRY)
                })
            })
            .collect()
    })
}

/// The first trap's shots left: its AI's charge count (param 1;
/// −1 until its first think counts them).
fn shots_left(r: &mut Rig) -> Option<i32> {
    let t = *traps(r).first()?;
    r.with(move |sim, _| {
        let c = sim.events.action.sys.hooks.ai.as_ref()?.control(t)?;
        Some(c.params[1])
    })
}

/// Lays one sentry of `skill` five sub-tiles east of the player.
fn lay(r: &mut Rig, skill: usize) {
    r.select_right(skill);
    r.right_click_point(5, 0);
    // Until the trap's first think has counted its shots (the summon's
    // think timer is at F + 25).
    for _ in 0..90 {
        if shots_left(r).is_some_and(|s| s >= 0) {
            break;
        }
        r.step(1);
    }
    r.step(4);
}

// (a sentry with no monster in range keeps its shots)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_trap_without_a_target_holds_its_shots() {
    let mut r = assassin(&[CHARGED_BOLT_SENTRY]);
    lay(&mut r, CHARGED_BOLT_SENTRY);
    assert_eq!(traps(&mut r).len(), 1, "a trap is laid ({})", r.errors());
    let shots = shots_left(&mut r);
    assert!(shots.is_some_and(|s| s > 0), "it has shots ({shots:?})");
    r.step(100);
    assert_eq!(shots_left(&mut r), shots, "no shot without a target");
}

// (the sentry shoots a monster in range, spends a shot each time, and
// dies after the last)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_trap_shoots_in_range_then_dies() {
    let mut r = assassin(&[CHARGED_BOLT_SENTRY]);
    lay(&mut r, CHARGED_BOLT_SENTRY);
    assert_eq!(traps(&mut r).len(), 1, "a trap is laid ({})", r.errors());
    let shots = shots_left(&mut r).expect("shots");
    // The trap stands 5 sub-tiles from the player: 4 more puts the monster
    // 4 from the trap. At 1 it is in the trap's melee range and never its
    // target (as measured in the game).
    let m = r.spawn_monster(9);
    tough(&mut r, m);
    let mut fewer = false;
    for _ in 0..1500 {
        if traps(&mut r).is_empty() {
            break;
        }
        fewer |= shots_left(&mut r).is_some_and(|s| s < shots);
        r.step(1);
    }
    let errors = r.errors();
    assert!(fewer, "a shot was spent ({errors})");
    assert!(
        traps(&mut r).is_empty(),
        "the trap is spent and gone ({errors})"
    );
}

// (a laid Lightning Sentry fires its missile: a shot is spent, the
// missile flies and hits)
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_lightning_sentry_fires_its_missile() {
    let mut r = assassin(&[LIGHTNING_SENTRY]);
    lay(&mut r, LIGHTNING_SENTRY);
    assert_eq!(traps(&mut r).len(), 1, "a trap is laid ({})", r.errors());
    let shots = shots_left(&mut r).expect("shots");
    // The trap stands 5 sub-tiles from the player: 4 more puts the monster
    // 4 from the trap. At 1 it is in the trap's melee range and never its
    // target (as measured in the game).
    let m = r.spawn_monster(9);
    tough(&mut r, m);
    let life0 = r.life(m);
    let mut flew = 0;
    let mut fewer = false;
    let mut low = life0;
    for _ in 0..600 {
        if low < life0 {
            break;
        }
        r.step(1);
        low = low.min(r.life(m));
        fewer |= shots_left(&mut r).is_some_and(|s| s < shots);
        flew = flew.max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
    }
    let errors = r.errors();
    assert!(flew > 0, "a missile flew ({errors})");
    assert!(fewer || traps(&mut r).is_empty(), "a shot was spent");
    assert!(low < life0, "the missile hit ({errors})");
}
