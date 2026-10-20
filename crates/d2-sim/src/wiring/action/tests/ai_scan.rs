//! Tests for the secondary target search on the wired units: target scan 6's
//! main and alternative slots, its filter and the alternative choice, plus
//! scan 5 and the target override. Runs through the `Fx` test fixture.

use super::*;
use crate::wiring::action::ai_scan::{Scan6, UNIT_FLAG_4};

/// Monstats `threat` of class 0.
fn set_threat(fx: &mut Fx, threat: u8) {
    let mut t = (*fx.sim.hooks().tables).clone();
    t.combat.monstats[0].threat = threat;
    fx.sim.hooks().tables = Arc::new(t);
}

/// Alignment `a` (stat 172 in the state-105 list).
fn align(fx: &mut Fx, u: UnitId, a: u8) {
    fx.sim.with(&mut fx.game, |g, v| v.set_alignment(g, u, a));
}

/// A scanner monster at (10, 10), a good (alignment 2) monster at
/// (14, 10) (outside the scanner's melee range) and a player
/// at (20, 10), each with unit flag 0x4.
fn three(fx: &mut Fx) -> (UnitId, UnitId, UnitId) {
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let c = fx.spawn(UnitType::Monster, 0, fx.a, 14, 10);
    let p = fx.spawn(UnitType::Player, 1, fx.a, 20, 10);
    for u in [m, c, p] {
        fx.sim.sys.units.get_mut(u).unwrap().flags |= UNIT_FLAG_4;
    }
    align(fx, c, 2);
    (m, c, p)
}

fn scan(fx: &mut Fx, m: UnitId) -> Scan6 {
    fx.sim.with(&mut fx.game, |g, v| v.scan6(g, m, None))
}

fn secondary(fx: &mut Fx, m: UnitId) -> (Option<UnitId>, i32, bool) {
    fx.sim
        .with(&mut fx.game, |g, v| v.secondary_search(g, m, None))
}

#[test]
fn threat_below_2_is_the_alternative_and_the_main_target_stays() {
    // A monster of threat 0 competes for the alternative; the player
    // (threat 14) for the main slot. Full-size distances (size 1):
    // 4 - 1 = 3 and 10 - 1 = 9. The alternative is within 5, but this host
    // has no path provider: the trial path is not run and the main target
    // is kept.
    let mut fx = Fx::new();
    set_threat(&mut fx, 0);
    let (m, c, p) = three(&mut fx);
    assert_eq!(
        scan(&mut fx, m),
        Scan6 {
            main: Some((p, 9)),
            alt: Some((c, 3)),
        }
    );
    assert_eq!(secondary(&mut fx, m), (Some(p), 9, false));
    fx.assert_clean();
}

#[test]
fn nearer_threatening_monster_takes_the_main_slot() {
    let mut fx = Fx::new();
    set_threat(&mut fx, 5);
    let (m, c, _) = three(&mut fx);
    assert_eq!(
        scan(&mut fx, m),
        Scan6 {
            main: Some((c, 3)),
            alt: None,
        }
    );
    assert_eq!(secondary(&mut fx, m).0, Some(c));
}

#[test]
fn filter_skips_without_flag_4_and_when_not_hostile() {
    // Unit flag 0x4 is required; so is hostility.
    let mut fx = Fx::new();
    set_threat(&mut fx, 5);
    let (m, c, p) = three(&mut fx);
    fx.sim.sys.units.get_mut(c).unwrap().flags &= !UNIT_FLAG_4;
    assert_eq!(scan(&mut fx, m).main, Some((p, 9)));
    fx.sim.sys.units.get_mut(c).unwrap().flags |= UNIT_FLAG_4;
    assert_eq!(scan(&mut fx, m).main, Some((c, 3)));
    // Both evil: not hostile.
    align(&mut fx, c, 0);
    assert_eq!(scan(&mut fx, m).main, Some((p, 9)));
}

#[test]
fn hostility_reads_the_live_alignment_and_the_minion_owner() {
    // Hostility: a scanner whose alignment is 2 for the moment (confused)
    // is on the player's side; a scanner whose minion owner is a good
    // monster takes that owner's side, and a candidate whose owner is the
    // scanner's owner is its own side.
    let mut fx = Fx::new();
    set_threat(&mut fx, 5);
    let (m, c, p) = three(&mut fx);
    align(&mut fx, m, 2);
    assert_eq!(scan(&mut fx, m).main, None);
    align(&mut fx, m, 0);
    let l = fx.spawn(UnitType::Monster, 0, fx.a, 40, 40);
    align(&mut fx, l, 2);
    let owner = |u: UnitId, fx: &mut Fx| crate::monsters::ai::AiControl {
        minion_owner: Some(crate::monsters::ai::UnitRef {
            ty: UnitType::Monster,
            guid: fx.sim.sys.units.get(u).unwrap().guid,
        }),
        ..Default::default()
    };
    let ctl = owner(l, &mut fx);
    fx.sim.hooks().ai.as_mut().unwrap().entry(m).control = Some(ctl.clone());
    assert_eq!(scan(&mut fx, m).main, None);
    align(&mut fx, l, 0);
    assert_eq!(scan(&mut fx, m).main, Some((c, 3)));
    fx.sim.hooks().ai.as_mut().unwrap().entry(c).control = Some(ctl);
    assert_eq!(scan(&mut fx, m).main, Some((p, 9)));
}

#[test]
fn monster_in_melee_reach_without_invis_is_kept() {
    // The melee-range test runs only for a candidate with state 146; a
    // monster next to the scanner is a candidate.
    let mut fx = Fx::new();
    set_threat(&mut fx, 5);
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let c = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    fx.sim.sys.units.get_mut(c).unwrap().flags |= UNIT_FLAG_4;
    align(&mut fx, c, 2);
    assert_eq!(scan(&mut fx, m).main.map(|(u, _)| u), Some(c));
}

#[test]
fn nothing_within_49_reports_no_distance() {
    // A distance of 49 or more is skipped; with no candidate the distance is
    // 0x7FFFFFFF.
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let p = fx.spawn(UnitType::Player, 1, fx.a, 60, 10);
    fx.sim.sys.units.get_mut(p).unwrap().flags |= UNIT_FLAG_4;
    // 50 - 1 = 49: out.
    assert_eq!(secondary(&mut fx, m), (None, 0x7FFF_FFFF, false));
}

fn scan5(fx: &mut Fx, m: UnitId) -> Scan6 {
    fx.sim.with(&mut fx.game, |g, v| v.scan5(g, m, false, None))
}

#[test]
fn scan5_walks_only_rooms_with_clients() {
    // Scan 5 skips a room with client count 0 whole (so the Harrogath
    // barbarian loses its minion target once the player's client has left
    // its rooms). The distance is the full-size one with the candidate's
    // size, 4 - 1 = 3.
    let mut fx = Fx::new();
    set_threat(&mut fx, 5);
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 10, 10);
    let c = fx.spawn(UnitType::Monster, 0, a, 14, 10);
    fx.sim.sys.units.get_mut(c).unwrap().flags |= UNIT_FLAG_4;
    align(&mut fx, c, 2);
    let none = Scan6 {
        main: None,
        alt: None,
    };
    assert_eq!(scan5(&mut fx, m), none);
    let p = fx.spawn(UnitType::Player, 1, a, 30, 30);
    fx.game
        .lists
        .add_client(Some(p), None, crate::units::lists::client_state::IN_GAME);
    fx.tick();
    assert_eq!(scan5(&mut fx, m).main.map(|(u, _)| u), Some(c));
}

#[test]
fn a_kind_2_override_wins_the_secondary_search_until_cleared() {
    // The threat-0 monster would only be the alternative (the player
    // stays main); a kind-2 override with its GUID forces it, at its
    // full-size distance (4 - 1 = 3). The setter needs `switchai`.
    let mut fx = Fx::new();
    set_threat(&mut fx, 0);
    let (m, c, p) = three(&mut fx);
    fx.sim.sys.hooks.monster_world = Some(Box::new(super::sound::DataOnly(
        [(m, crate::monsters::init::MonsterData::default())]
            .into_iter()
            .collect(),
    )));
    let g = fx.sim.sys.units.get(c).unwrap().guid;
    fx.sim
        .with(&mut fx.game, |_, v| v.set_target_override(m, 2, g));
    assert_eq!(secondary(&mut fx, m).0, Some(p), "no switchai: not set");
    let mut t = (*fx.sim.hooks().tables).clone();
    t.combat.monstats[0].switchai = true;
    fx.sim.hooks().tables = Arc::new(t);
    fx.sim
        .with(&mut fx.game, |_, v| v.set_target_override(m, 2, g));
    assert_eq!(secondary(&mut fx, m), (Some(c), 3, false));
    fx.sim.with(&mut fx.game, |_, v| v.clear_target_override(m));
    assert_eq!(secondary(&mut fx, m).0, Some(p));
    fx.assert_clean();
}
