use super::*;
use crate::bridge::overlay_lights::{self, OverlayLightRow};
use crate::bridge::world::{ClientUnit, StateOverlayLinks, StateRow, PLAYER};

const P: UnitKey = UnitKey::new(PLAYER, 1);
const SEED: (u32, u32) = (0x1234_5678, 666);

/// Overlays: 2 the cast (2 frames, one per update, unlit); 3 and 4 a
/// lit `1ofN` 2 group; 5 lit; 6 unlit (a `removerlay`).
fn inputs() -> ModelInputs {
    let mut i = ModelInputs {
        expansion_installed: true,
        ..ModelInputs::default()
    };
    let row = |frames, radius, one_of_n| OverlayLightRow {
        frames,
        anim_rate: 16,
        init_radius: radius,
        radius,
        one_of_n,
        ..OverlayLightRow::default()
    };
    i.tables.overlays = vec![
        OverlayLightRow::default(),
        OverlayLightRow::default(),
        row(2, 0, 1),
        row(4, 4, 2),
        row(4, 4, 1),
        row(4, 2, 1),
        row(4, 0, 1),
    ];
    let state = |cast, overlay1, overlay2, remove, aura| StateRow {
        links: StateOverlayLinks {
            cast,
            overlay1,
            overlay2,
            remove,
        },
        aura,
        ..StateRow::default()
    };
    i.tables.states = vec![
        StateRow::default(),
        // 1: cast 2, ov1 the group 3, ov2 5, removerlay 6.
        state(2, 3, 5, 6, false),
        // 2: an aura, cast 2, ov1 5.
        state(2, 5, -1, -1, true),
        // 3: `nooverlays`.
        StateRow {
            nooverlays: true,
            ..state(2, 3, 5, -1, false)
        },
        // 4: an other aura, no overlays.
        state(-1, -1, -1, -1, true),
    ];
    i
}

fn world() -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut u = ClientUnit::new(P);
    u.seed = Some(SEED);
    w.units.insert(P, u);
    w
}

/// U's seed after the rolls `ns`, and their results.
fn rolls(ns: &[i32]) -> ((u32, u32), Vec<u32>) {
    let mut s = d2_sim::rng::Seed::new(SEED.0, SEED.1);
    let r = ns.iter().map(|&n| s.roll(n)).collect();
    ((s.lo, s.hi), r)
}

/// State on: the bit, then the overlays (the order of `state_on`).
fn state_on(w: &mut ClientWorld, i: &ModelInputs, s: u8) {
    w.units.get_mut(&P).unwrap().states.insert(s);
    on(w, i, P, s);
}

fn calls(w: &ClientWorld) -> Vec<(u16, u8, [i32; 3], bool)> {
    w.hit_overlays
        .iter()
        .map(|c| (c.id, c.kind, c.args, c.remove))
        .collect()
}

fn records(w: &ClientWorld) -> Vec<(u16, u8)> {
    w.overlay_lights.iter().map(|o| (o.id, o.kind)).collect()
}

#[test]
fn a_cast_state_makes_one_kind_4_with_the_rolled_follow_ups() {
    let mut w = world();
    let i = inputs();
    state_on(&mut w, &i, 1);
    // cast, ov1, ov2 each roll their `1ofN` on U's seed, in that order.
    let (seed, r) = rolls(&[1, 2, 1]);
    let ov1 = 3 + r[1] as i32;
    assert_eq!(w.units[&P].seed, Some(seed));
    assert_eq!(calls(&w), vec![(2, 4, [ov1, 5, 0], false)]);
    assert_eq!(records(&w), vec![(2, 4)]);
    // Two frames: the cast ends, leaves, and makes A and B as kind 3
    // (lit), advanced in the same update.
    overlay_lights::advance(&mut w, &i, P);
    assert_eq!(records(&w), vec![(2, 4)]);
    overlay_lights::advance(&mut w, &i, P);
    let mut got = records(&w);
    got.sort_unstable();
    assert_eq!(got, vec![(ov1 as u16, 3), (5, 3)]);
    assert!(w
        .overlay_lights
        .iter()
        .all(|o| o.light.is_some() && o.frame == 256));
}

#[test]
fn an_aura_state_makes_kind_9_and_its_follow_ups_need_the_state() {
    let i = inputs();
    let mut w = world();
    // Its own bit does not count as an other aura.
    state_on(&mut w, &i, 2);
    assert_eq!(calls(&w), vec![(2, 9, [5, -1, 2], false)]);
    assert!(w.units[&P].states.contains(&2));
    overlay_lights::advance(&mut w, &i, P);
    overlay_lights::advance(&mut w, &i, P);
    assert_eq!(records(&w), vec![(5, 8)]);
    assert_eq!(w.overlay_lights[0].state, 2);
    // Kind 8 leaves with its state.
    w.units.get_mut(&P).unwrap().states.remove(&2);
    overlay_lights::advance(&mut w, &i, P);
    assert!(records(&w).is_empty());

    // The state gone before the cast ends: no follow-up.
    let mut w = world();
    state_on(&mut w, &i, 2);
    w.units.get_mut(&P).unwrap().states.remove(&2);
    overlay_lights::advance(&mut w, &i, P);
    overlay_lights::advance(&mut w, &i, P);
    assert!(records(&w).is_empty());
}

#[test]
fn an_other_aura_drops_the_cast_to_kind_8() {
    let i = inputs();
    let mut w = world();
    w.units.get_mut(&P).unwrap().states.insert(4);
    state_on(&mut w, &i, 2);
    // Both rolls still drawn.
    assert_eq!(w.units[&P].seed, Some(rolls(&[1, 1]).0));
    assert_eq!(calls(&w), vec![(5, 8, [2, 0, 0], false)]);
    assert_eq!(records(&w), vec![(5, 8)]);
    // A kind-8 create of an id U already has makes nothing.
    state_on(&mut w, &i, 2);
    assert_eq!(records(&w), vec![(5, 8)]);
}

#[test]
fn nooverlays_or_no_row_makes_nothing_and_draws_nothing() {
    let i = inputs();
    let mut w = world();
    state_on(&mut w, &i, 3);
    state_on(&mut w, &i, 99);
    assert!(calls(&w).is_empty() && records(&w).is_empty());
    assert_eq!(w.units[&P].seed, Some(SEED));
}

#[test]
fn state_off_makes_the_removerlay_and_removes_the_groups() {
    let i = inputs();
    let mut w = world();
    for id in [3, 4, 5] {
        overlay_lights::create(&mut w, &i, P, id, 3);
    }
    w.hit_overlays.clear();
    off(&mut w, &i, P, 1);
    assert_eq!(w.units[&P].seed, Some(rolls(&[1]).0));
    assert_eq!(
        calls(&w),
        vec![
            (6, 4, [-1, -1, 0], false),
            (3, 0, [0; 3], true),
            (4, 0, [0; 3], true),
            (5, 0, [0; 3], true),
        ]
    );
    assert_eq!(records(&w), vec![(6, 4)]);
    assert!(w.lights.is_empty(), "the lights are freed");
}

/// State 5: a `pgsv` state with `pgsvoverlay` 3 and stat 7 (the
/// progressive charges); 7 overlay rows, 8 `itemstatcost` rows.
fn pgsv_inputs() -> ModelInputs {
    let mut i = inputs();
    i.tables.stats = vec![Default::default(); 8];
    i.tables.states.push(StateRow {
        pgsv: true,
        pgsv_overlay: 3,
        stat: 7,
        setfunc: 1,
        remfunc: 1,
        ..StateRow::default()
    });
    i
}

fn charges(w: &mut ClientWorld, n: i32) {
    w.units
        .get_mut(&P)
        .unwrap()
        .state_lists
        .insert(5, [((7, 0), n)].into());
}

#[test]
fn pgsv_shows_the_overlay_of_the_charge_count() {
    let i = pgsv_inputs();
    let mut w = world();
    // Three charges: the three removed, then base + 3 − 1, kind 3.
    charges(&mut w, 3);
    pgsv_set(&mut w, &i, P, 5);
    let rm = |id| (id, 0, [0; 3], true);
    assert_eq!(calls(&w), vec![rm(3), rm(4), rm(5), (5, 3, [0; 3], false)]);
    // No roll on U's seed.
    assert_eq!(w.units[&P].seed, Some(SEED));
    // Remfunc 1: the three removed, nothing made.
    w.hit_overlays.clear();
    pgsv_rem(&mut w, &i, P, 5);
    assert_eq!(calls(&w), vec![rm(3), rm(4), rm(5)]);
    // 0 and 4 charges make nothing after the removals.
    for n in [0, 4] {
        w.hit_overlays.clear();
        charges(&mut w, n);
        pgsv_set(&mut w, &i, P, 5);
        assert_eq!(calls(&w), vec![rm(3), rm(4), rm(5)], "{n} charges");
    }
}

/// One change to the pgsv state row or the inputs.
type Edit = fn(&mut StateRow, &mut ModelInputs);

#[test]
fn pgsv_guard_needs_the_flag_and_rows_in_range() {
    let base = pgsv_inputs();
    let cases: [(&str, Edit); 5] = [
        ("no pgsv flag", |r, _| r.pgsv = false),
        ("overlay 0", |r, _| r.pgsv_overlay = 0),
        ("overlay = count", |_, i| i.tables.overlays.truncate(3)),
        ("stat < 0", |r, _| r.stat = -1),
        ("stat > count", |_, i| i.tables.stats.truncate(6)),
    ];
    for (what, edit) in cases {
        let mut i = base.clone();
        let mut row = i.tables.states[5];
        edit(&mut row, &mut i);
        i.tables.states[5] = row;
        let mut w = world();
        charges(&mut w, 2);
        pgsv_set(&mut w, &i, P, 5);
        pgsv_rem(&mut w, &i, P, 5);
        assert!(calls(&w).is_empty(), "{what}");
    }
    // stat = count passes (≤, as the base game).
    let mut i = base.clone();
    i.tables.stats.truncate(7);
    let mut w = world();
    charges(&mut w, 2);
    pgsv_set(&mut w, &i, P, 5);
    assert_eq!(calls(&w).last(), Some(&(4, 3, [0; 3], false)));
}
