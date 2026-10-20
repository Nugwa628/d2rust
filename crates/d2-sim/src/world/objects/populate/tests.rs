// Spec: specs/world/object-population.md (Test vectors, Edge cases)
use super::super::fake::{blank_level, blank_object, blank_shrine, Call, Fake};
use super::*;
use d2_data::tables::Record;

const ROOM: RoomId = RoomId(3);
/// The populated level of the fixture room.
const LEVEL: u32 = 2;

impl PopulateWorld for Fake {
    fn room_seed(&mut self, _room: RoomId) -> Option<&mut Seed> {
        self.seeds.get_mut(&UnitId(u32::MAX))
    }
    fn populated_room_count(&mut self, act: u8, level: u32) -> i32 {
        self.calls.push(Call::Other(format!("count {act} {level}")));
        self.stats
            .get(&(UnitId(u32::MAX), 0))
            .copied()
            .unwrap_or(20)
    }
    fn box_query(&self, _: RoomId, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32 {
        self.box_log.borrow_mut().push((x, y, sx, sy, mask));
        if self.block.is_some_and(|b| b(x, y, sx, sy, mask)) {
            return 1;
        }
        // Blocked everywhere when stat (MAX, 1) is set.
        self.stats.get(&(UnitId(u32::MAX), 1)).copied().unwrap_or(0) as u32
    }
    fn set_unit_class(&mut self, unit: UnitId, class: u16) {
        self.calls
            .push(Call::Other(format!("class {} {class}", unit.0)));
    }
}

fn room_seed(f: &mut Fake) -> &mut Seed {
    f.seeds.get_mut(&UnitId(u32::MAX)).unwrap()
}

struct Fx {
    ctl: ObjectControl,
    t: ObjectTables,
    f: Fake,
    info: RoomInfo,
}

fn blank_group() -> Objgroup {
    Objgroup::decode(&[0; Objgroup::SIZE])
}

/// Level 2 with slot 0 = group 1 (`ObjPrb` `prb`); group 1 members
/// (class 10, PROB 30) and (class 11, PROB 70), both `PopulateFn` 3 with
/// density 32; a 40 × 40 room at (100, 100); room seed {1, 666}.
fn fx(prb: u8) -> Fx {
    let mut objects = vec![blank_object(); 300];
    for c in [10, 11] {
        objects[c].populatefn = 3;
        objects[c].sizex = 1;
        objects[c].sizey = 1;
    }
    let mut levels = vec![blank_level(); 4];
    levels[LEVEL as usize].objgrp0 = 1;
    levels[LEVEL as usize].objprb0 = prb;
    let mut g = blank_group();
    (g.id0, g.density0, g.prob0) = (10, 32, 30);
    (g.id1, g.density1, g.prob1) = (11, 32, 70);
    let mut shrines = vec![blank_shrine(); 3];
    shrines[2].effectclass = 2;
    shrines[2].levelmin = 1;
    let t = ObjectTables {
        objects,
        shrines,
        levels,
        objgroup: vec![blank_group(), g],
        leveldefs: Vec::new(),
    };
    let (ctl, _) = ObjectControl::new(&mut Seed::new(7, 666), &t);
    let mut f = Fake {
        next_alloc: Some(100),
        ..Fake::default()
    };
    f.seeds.insert(UnitId(u32::MAX), Seed::new(1, 666));
    Fx {
        ctl,
        t,
        f,
        info: RoomInfo {
            room: ROOM,
            level: LEVEL,
            populated: LEVEL,
            waypoint: false,
            dirt_path: false,
            rect: (100, 100, 40, 40),
        },
    }
}

fn run_room(x: &mut Fx) -> Populated {
    populate_room(&mut x.ctl, &x.t, &mut x.f, &x.info).unwrap()
}

fn allocs(f: &Fake) -> Vec<u16> {
    f.calls
        .iter()
        .filter_map(|c| match c {
            Call::Allocate(_, class, ..) => Some(*class),
            _ => None,
        })
        .collect()
}

// Covers: specs/world/object-population.md §1 r3, §5 r1, §5 r2, §5 r4, §5 r6
#[test]
fn slot_roll_and_member_pick() {
    let mut x = fx(60);
    let out = run_room(&mut x);
    // r = 51 ≤ 60; r2 = 31: PROB {30, 70} → member 1 (class 11).
    assert!(!out.objects.is_empty());
    assert!(allocs(&x.f).iter().all(|&c| c == 11));

    let mut x = fx(50);
    let out = run_room(&mut x);
    assert!(out.objects.is_empty());
    // One R step per slot (8), no member draw for slot 0.
    let mut expect = Seed::new(1, 666);
    assert_eq!(expect.step() % 100, 51);
    let after_one = expect;
    assert_eq!(after_one, Seed::new(1_791_398_751, 0));
    for _ in 1..8 {
        expect.step();
    }
    assert_eq!(*room_seed(&mut x.f), expect);
}

// Covers: specs/world/object-population.md §2, §edge-cases-original-bugs r4
#[test]
fn region_tests() {
    let mut r = Region::new(0);
    (r.w08, r.counted) = (20, 16);
    assert!(want_health(&r));
    r.counted = 15;
    assert!(!want_health(&r));
    r.health = 1;
    r.counted = 16;
    assert!(!want_health(&r));
    r.w08 = 0;
    assert!(!want_health(&r));

    let mut r = Region::new(0);
    r.w08 = 30;
    r.shrines = 3;
    assert!(!shrine_cap(&r));
    r.shrines = 4;
    assert!(shrine_cap(&r));
    r.shrines = 10;
    r.w08 = 1000;
    assert!(shrine_cap(&r));
    r.wells = 4;
    assert!(well_cap(&r));

    let mut r = Region::new(0);
    record_shrine(&mut r, (100, 100));
    assert!(!shrine_spaced(&r, 140, 300), "dx 40 < 50");
    assert!(shrine_spaced(&r, 160, 160));
    // Edge case 4: a cross, not a box.
    assert!(!shrine_spaced(&r, 600, 100));
    record_well(&mut r, (100, 100));
    assert!(!well_spaced(&r, 199, 500));
    assert!(well_spaced(&r, 200, 200));
    // Record stops at the cap.
    for i in 0..20 {
        record_shrine(&mut r, (i, i));
    }
    assert_eq!(r.shrines, 10);
}

// Covers: specs/world/object-population.md §4 r2, §4 r6, §edge-cases-original-bugs r2
#[test]
fn theme_gate_draws_and_never_runs() {
    let mut x = fx(0);
    x.ctl.seed = Seed::new(1, 666);
    {
        let r = region(&mut x.ctl, LEVEL).unwrap();
        (r.w08, r.counted) = (20, 15);
    }
    assert!(!theme_gate(&mut x.ctl, LEVEL, 60).unwrap());
    assert_eq!(x.ctl.seed, Seed::new(1_791_398_751, 0), "one C draw");
    // r < p + 12: every pick of {3, 4, 5, 6} fails (t = 3 inactive, others ≥ 4).
    for lo in 0..200u32 {
        let mut s = Seed::new(lo, 0);
        let mut probe = s;
        if probe.step() % 100 >= 22 {
            continue;
        }
        x.ctl.seed = s;
        {
            let r = region(&mut x.ctl, LEVEL).unwrap();
            (r.w08, r.counted) = (20, 19);
        }
        assert!(!theme_gate(&mut x.ctl, LEVEL, 60).unwrap());
        s.step();
        s.roll(4);
        assert_eq!(x.ctl.seed, s, "two C draws");
    }
}

// Test vector: count formula (§7 text, exempt)
#[test]
fn count_formula() {
    let x = fx(0);
    let mut out = Populated::default();
    let mut f = Fake::default();
    let mut ctl = x.ctl.clone();
    let cx = Cx {
        ctl: &mut ctl,
        t: &x.t,
        w: &mut f,
        info: &x.info,
        out: &mut out,
    };
    assert_eq!(cx.count(32), 1);
    assert_eq!(cx.count(125), 5);
}

fn cx_run(x: &mut Fx, f: impl FnOnce(&mut Cx<'_, Fake>) -> Result<(), ObjectError>) -> Populated {
    let mut out = Populated::default();
    let mut cx = Cx {
        ctl: &mut x.ctl,
        t: &x.t,
        w: &mut x.f,
        info: &x.info,
        out: &mut out,
    };
    f(&mut cx).unwrap();
    out
}

// Covers: specs/world/object-population.md §7.4 r1, §7.4 r2
#[test]
fn fn4_first_class() {
    let mut x = fx(0);
    x.t.objects[7].sizex = 1;
    x.t.objects[7].sizey = 1;
    x.info.rect = (100, 100, 1000, 1000);
    x.ctl.seed = Seed::new(1, 666);
    let mut probe = Seed::new(1, 666);
    assert_eq!(probe.step() % 100, 51);
    assert_eq!(probe.step(), 791_599_131);
    cx_run(&mut x, |c| c.fn4(32));
    assert_eq!(allocs(&x.f).first(), Some(&11));
    assert!(allocs(&x.f).len() <= 8, "at most 8 per call");
}

// Covers: specs/world/object-population.md §7.7 r4, §edge-cases-original-bugs r3
#[test]
fn fn7_places_nothing() {
    let mut x = fx(0);
    x.info.rect = (100, 100, 400, 400);
    let out = cx_run(&mut x, |c| c.fn7(10));
    assert!(out.objects.is_empty());
    assert!(allocs(&x.f).is_empty());
    assert_ne!(x.ctl.seed, fx(0).ctl.seed, "draws still happen");
}

// Covers: specs/world/object-population.md §1 r2, §3 r1, §3 r2, §3 r3, §3 r4
#[test]
fn precheck_skips_and_counts() {
    for (wp, dirt, pop) in [
        (true, false, LEVEL),
        (false, true, LEVEL),
        (false, false, 0),
    ] {
        let mut x = fx(90);
        x.info.waypoint = wp;
        x.info.dirt_path = dirt;
        x.info.populated = pop;
        let before = *room_seed(&mut x.f);
        assert!(run_room(&mut x).objects.is_empty());
        assert_eq!(*room_seed(&mut x.f), before, "no slot draws");
    }
    // A town level skips too.
    let mut x = fx(90);
    x.t.levels = vec![blank_level(); 50];
    x.t.levels[40].objgrp0 = 1;
    x.info.level = 40;
    x.info.populated = 40;
    x.ctl = ObjectControl::new(&mut Seed::new(7, 666), &x.t).0;
    let before = *room_seed(&mut x.f);
    run_room(&mut x);
    assert_eq!(*room_seed(&mut x.f), before);

    // Total set once from the room count; counted increments per room.
    let mut x = fx(0);
    run_room(&mut x);
    run_room(&mut x);
    let r = *region(&mut x.ctl, LEVEL).unwrap();
    assert_eq!((r.w08, r.counted), (20, 2));
    let counts =
        x.f.calls_of(|c| matches!(c, Call::Other(s) if s.starts_with("count")));
    assert_eq!(counts.len(), 1);
}

// Covers: specs/world/object-population.md §5 r5, §edge-cases-original-bugs r7
#[test]
fn missing_group_ends_room() {
    let mut x = fx(100);
    x.t.levels[LEVEL as usize].objgrp0 = 9;
    x.t.levels[LEVEL as usize].objgrp1 = 1;
    x.t.levels[LEVEL as usize].objprb1 = 100;
    run_room(&mut x);
    let mut expect = Seed::new(1, 666);
    expect.step();
    assert_eq!(*room_seed(&mut x.f), expect, "only slot 0's draw");
}

// Covers: specs/world/object-population.md §5 r3, §edge-cases-original-bugs r1
#[test]
fn want_health_suppresses_subclass_rows() {
    let mut x = fx(99);
    x.t.objects[1].subclass = 8;
    {
        let r = region(&mut x.ctl, LEVEL).unwrap();
        (r.w08, r.counted) = (20, 16);
    }
    run_room(&mut x);
    assert!(allocs(&x.f).is_empty(), "r = 100 > ObjPrb");
}

// Covers: specs/world/object-population.md §7.2 r3, §7.2 r4, §edge-cases-original-bugs r5
#[test]
fn fn2_forced_health_shrine() {
    let mut x = fx(0);
    x.t.objects[30].parm1 = 77;
    x.t.objects[30].sizex = 1;
    x.t.objects[30].sizey = 1;
    {
        let r = region(&mut x.ctl, LEVEL).unwrap();
        (r.w08, r.counted) = (20, 16);
    }
    let mut got = None;
    cx_run(&mut x, |c| {
        got = c.fn2(30)?;
        Ok(())
    });
    let u = got.expect("placed");
    let d = x.ctl.get(u).unwrap();
    assert_eq!((d.interact, d.shrine, d.class), (2, Some(2), 30));
    let r = *region(&mut x.ctl, LEVEL).unwrap();
    assert_eq!((r.health, r.shrines), (1, 1));
    assert_eq!(r.shrine_points[0], x.f.positions[&u]);
    assert!(x
        .f
        .calls
        .contains(&Call::Other(format!("class {} 77", u.0))));
}

// Covers: specs/world/object-population.md §7.8 r1
#[test]
fn fn8_cap_no_draw() {
    let mut x = fx(0);
    region(&mut x.ctl, LEVEL).unwrap().wells = 4;
    let before = *room_seed(&mut x.f);
    let c0 = x.ctl.seed;
    cx_run(&mut x, |c| c.fn8(10, 30).map(drop));
    assert_eq!(*room_seed(&mut x.f), before);
    assert_eq!(x.ctl.seed, c0);
}

// Covers: specs/world/object-population.md §6
#[test]
fn blocked_room_places_nothing() {
    let mut x = fx(100);
    x.f.stats.insert((UnitId(u32::MAX), 1), 1);
    assert!(run_room(&mut x).objects.is_empty());
}

// Covers: specs/world/object-population.md §5 r6
#[test]
fn fatal_populate_fn_and_density() {
    let mut x = fx(100);
    x.t.objects[11].populatefn = 10;
    assert_eq!(
        populate_room(&mut x.ctl, &x.t, &mut x.f, &x.info),
        Err(ObjectError::PopulateFn(10))
    );
    let mut x = fx(100);
    x.t.objgroup[1].density1 = 128;
    assert_eq!(
        populate_room(&mut x.ctl, &x.t, &mut x.f, &x.info),
        Err(ObjectError::Density(128))
    );
}

// ------------------------------------------------------------------ more

fn adv(s: &mut Seed, n: usize) {
    for _ in 0..n {
        s.step();
    }
}

/// A fixture room with the C seed {1, 666}, every collision free, and the
/// rows in `rows` sized 1 × 1.
fn free_room(rows: &[usize]) -> Fx {
    let mut x = fx(0);
    for &r in rows {
        x.t.objects[r].sizex = 1;
        x.t.objects[r].sizey = 1;
    }
    x.ctl.seed = Seed::new(1, 666);
    x
}

fn blocked_room(rows: &[usize]) -> Fx {
    let mut x = free_room(rows);
    x.f.stats.insert((UnitId(u32::MAX), 1), 1);
    x
}

// Covers: specs/world/object-population.md §1 r1
#[test]
fn entry_needs_a_levels_record() {
    for level in [0, 4, 99] {
        let mut x = fx(0);
        x.info.level = level;
        assert!(matches!(
            populate_room(&mut x.ctl, &x.t, &mut x.f, &x.info),
            Err(ObjectError::NoRow {
                table: "levels",
                ..
            })
        ));
    }
}

// Covers: specs/world/object-population.md §3 r2, §3 r5, §3 r6
#[test]
fn towns_and_level_count_skip_themes_gate_and_slots() {
    for town in [1u32, 40, 75, 103, 109] {
        let mut x = fx(90);
        x.t.levels = vec![blank_level(); 120];
        x.t.levels[town as usize].objgrp0 = 1;
        x.t.levels[town as usize].objprb0 = 100;
        x.info.level = town;
        x.info.populated = town;
        x.ctl = ObjectControl::new(&mut Seed::new(7, 666), &x.t).0;
        let (r0, c0) = (*room_seed(&mut x.f), x.ctl.seed);
        assert!(run_room(&mut x).objects.is_empty());
        assert_eq!((*room_seed(&mut x.f), x.ctl.seed), (r0, c0), "town {town}");
    }
    // Populated level at or above the levels count: nothing.
    let mut x = fx(90);
    x.info.populated = 4;
    let r0 = *room_seed(&mut x.f);
    assert!(run_room(&mut x).objects.is_empty());
    assert_eq!(*room_seed(&mut x.f), r0);

    // Themes ≠ 0 and the gate returning 0: the slots still run (8 R
    // steps); Themes 8 → list {4}, n = 1, no theme passes t < n. C
    // {1, 666}: r = 51 ≥ 12 → exactly one C draw.
    let mut x = free_room(&[]);
    x.t.levels[LEVEL as usize].themes = 8;
    run_room(&mut x);
    let mut c = Seed::new(1, 666);
    c.step();
    assert_eq!(x.ctl.seed, c);
    let mut r = Seed::new(1, 666);
    adv(&mut r, 8);
    assert_eq!(*room_seed(&mut x.f), r);

    // The gate returning 1 (Themes 3 → {1, 2}; pick theme 1, active,
    // returns 1): the room is skipped but still counted.
    let lo = (0..5000u32)
        .find(|&lo| {
            let mut s = Seed::new(lo, 0);
            s.step() % 100 < 12 && s.roll(2) == 0
        })
        .unwrap();
    let mut x = free_room(&[]);
    x.t.levels[LEVEL as usize].themes = 3;
    x.ctl.seed = Seed::new(lo, 0);
    let before = *room_seed(&mut x.f);
    assert!(run_room(&mut x).objects.is_empty());
    assert_eq!(*room_seed(&mut x.f), before, "no slot draws");
    assert_eq!(region(&mut x.ctl, LEVEL).unwrap().counted, 1);
}

fn seed_with_step_mod(target: u32) -> Seed {
    let lo = (0..20_000u32)
        .find(|&lo| Seed::new(lo, 0).step() % 100 == target)
        .unwrap();
    Seed::new(lo, 0)
}

// Covers: specs/world/object-population.md §4 r1, §4 r2, §4 r3, §4 r4, §4 r5
#[test]
fn theme_gate_thresholds_and_pick_draws() {
    // r1: total below theme count · 10 (count 0): return 0, no draw.
    let mut x = fx(0);
    let s0 = Seed::new(1, 666);
    x.ctl.seed = s0;
    region(&mut x.ctl, LEVEL).unwrap().w08 = -1;
    assert!(!theme_gate(&mut x.ctl, LEVEL, 60).unwrap());
    assert_eq!(x.ctl.seed, s0);

    // r2–r4: p = 0 / 5 / 10; the pick (4 bits → roll(4)) is drawn only
    // when r < p + 12. total 20: counted > 10 adds 5, counted > 15 adds 5.
    for (counted, p) in [(10, 0), (11, 5), (15, 5), (16, 10)] {
        for (r, drawn) in [(p + 11, true), (p + 12, false)] {
            let mut x = fx(0);
            let s = seed_with_step_mod(r);
            x.ctl.seed = s;
            {
                let reg = region(&mut x.ctl, LEVEL).unwrap();
                (reg.w08, reg.counted) = (20, counted);
            }
            assert!(!theme_gate(&mut x.ctl, LEVEL, 60).unwrap());
            let mut e = s;
            e.step();
            if drawn {
                e.roll(4);
            }
            assert_eq!(x.ctl.seed, e, "counted {counted} r {r}");
        }
    }

    // r5: no theme bit in 0..=6 (Themes 0x80): n = 0, no pick draw.
    let mut x = fx(0);
    let s = seed_with_step_mod(0);
    x.ctl.seed = s;
    {
        let reg = region(&mut x.ctl, LEVEL).unwrap();
        (reg.w08, reg.counted) = (20, 0);
    }
    assert!(!theme_gate(&mut x.ctl, LEVEL, 0x80).unwrap());
    let mut e = s;
    e.step();
    assert_eq!(x.ctl.seed, e);

    // r5: the list is the set bits in order (theme number = bit + 1):
    // Themes 0b1111 → {1, 2, 3, 4}; a pick of 3 (inactive) returns 0.
    let lo = (0..20_000u32)
        .find(|&lo| {
            let mut s = Seed::new(lo, 0);
            s.step() % 100 < 12 && s.roll(4) == 2
        })
        .unwrap();
    let mut x = fx(0);
    x.ctl.seed = Seed::new(lo, 0);
    {
        let reg = region(&mut x.ctl, LEVEL).unwrap();
        (reg.w08, reg.counted) = (20, 0);
    }
    assert!(!theme_gate(&mut x.ctl, LEVEL, 0b1111).unwrap());
}

// Covers: specs/world/object-population.md §5 r6, §5 r7
#[test]
fn member_walk_gore_zero_prob_and_null_fn() {
    // A member with r2 < acc but Gore > 2 lets the walk go on.
    let mut x = fx(100);
    x.t.objects[10].gore = 3;
    (x.t.objgroup[1].prob0, x.t.objgroup[1].prob1) = (100, 100);
    run_room(&mut x);
    assert!(!allocs(&x.f).is_empty());
    assert!(allocs(&x.f).iter().all(|&c| c == 11));
    // PopulateFn 0: nothing and the walk stops.
    let mut x = fx(100);
    x.t.objects[11].populatefn = 0;
    (x.t.objgroup[1].prob0, x.t.objgroup[1].prob1) = (0, 100);
    x.t.objgroup[1].id0 = 0;
    run_room(&mut x);
    assert!(allocs(&x.f).is_empty());
    // PROB sum below r2: no member (r2 = 31, sum 30).
    let mut x = fx(100);
    (x.t.objgroup[1].prob0, x.t.objgroup[1].prob1) = (30, 0);
    x.t.objgroup[1].id1 = 0;
    assert!(run_room(&mut x).objects.is_empty());
    // 8 slot draws + 1 member draw.
    let mut r = Seed::new(1, 666);
    adv(&mut r, 9);
    assert_eq!(*room_seed(&mut x.f), r);
}

// Covers: specs/world/object-population.md §6
#[test]
fn fits_and_spot_helpers() {
    let mut x = free_room(&[10]);
    let narrow = RoomInfo {
        rect: (100, 100, 1, 40),
        ..x.info
    };
    let mut out = Populated::default();
    let mut c = x.ctl.clone();
    let mut cx = Cx {
        ctl: &mut c,
        t: &x.t,
        w: &mut x.f,
        info: &x.info,
        out: &mut out,
    };
    // Rect (100, 100, 40, 40), sizes 1: Fit A x, y in 102..=136.
    assert!(cx.fit_a(102, 102, 1, 1));
    assert!(!cx.fit_a(101, 102, 1, 1));
    assert!(cx.fit_a(136, 136, 1, 1));
    assert!(!cx.fit_a(137, 136, 1, 1));
    assert!(!cx.fit_a(110, 137, 1, 1));
    // Fit B: 103..=138.
    assert!(cx.fit_b(103, 103, 1, 1));
    assert!(!cx.fit_b(102, 103, 1, 1));
    assert!(cx.fit_b(138, 138, 1, 1));
    assert!(!cx.fit_b(139, 138, 1, 1));
    // Fit C: 101..=138, nonzero, low 16 bits compared.
    assert!(cx.fit_c(101, 101, 1, 1, None));
    assert!(!cx.fit_c(100, 101, 1, 1, None));
    assert!(cx.fit_c(138, 138, 1, 1, None));
    assert!(!cx.fit_c(139, 120, 1, 1, None));
    assert!(cx.fit_c(0x10000 + 110, 110, 1, 1, None));
    cx.w.box_log.borrow_mut().clear();
    assert!(cx.fit_a(110, 110, 3, 2));
    assert!(cx.fit_b(110, 110, 3, 2));
    assert!(cx.fit_c(110, 110, 3, 2, None));
    assert_eq!(
        *cx.w.box_log.borrow(),
        vec![
            (110, 110, 10, 9, 0xC01),
            (110, 110, 3, 2, 0x3F11),
            (110, 110, 5, 4, 0x3F11),
            (110, 110, 9, 8, 0x3F11),
        ]
    );
    // Random spot: C seed, roll(w − sx − 1) twice per try; room too
    // narrow → none with no draw.
    let s0 = cx.ctl.seed;
    cx.info = &narrow;
    assert!(cx.random_spot(10, 1, 1).unwrap().is_none());
    assert_eq!(cx.ctl.seed, s0);
    assert!(cx
        .oriented_spot(10, 1, 1, 0, Filter::Shrine)
        .unwrap()
        .is_none());
    assert!(cx.spread_spot(10, 1, 1, Filter::Well).unwrap().is_none());
}

// Covers: specs/world/object-population.md §6
#[test]
fn spot_helper_draw_orders() {
    // Random spot (C): x then y per try, replayed.
    let mut x = free_room(&[10]);
    let mut e = Seed::new(1, 666);
    let (px, py) = loop {
        let (a, b) = (100 + e.roll(38), 100 + e.roll(38));
        if a > 100 && b > 100 {
            break (a as i32, b as i32);
        }
    };
    let out = cx_run(&mut x, |c| c.random_spot(10, 1, 1).map(drop));
    assert_eq!(out.objects.len(), 1);
    assert_eq!(x.f.positions[&out.objects[0]], (px, py));
    assert_eq!(x.ctl.seed, e);

    // Oriented spot 1 (R): x := x0 + w/4 + roll(w/2), then one step, y =
    // y0 + 1; orientation 2 the other way round.
    for (orient, first_roll) in [(1u8, true), (2, false)] {
        let mut x = free_room(&[10]);
        let mut e = *room_seed(&mut x.f);
        let (ex, ey) = if first_roll {
            let a = 100 + 10 + e.roll(20) as i32;
            e.step();
            (a, 101)
        } else {
            e.step();
            (101, 100 + 10 + e.roll(20) as i32)
        };
        let out = cx_run(&mut x, |c| {
            c.oriented_spot(10, 1, 1, orient, Filter::Shrine).map(drop)
        });
        assert_eq!(x.f.positions[&out.objects[0]], (ex, ey), "orient {orient}");
        assert_eq!(*room_seed(&mut x.f), e);
    }
    // Spread spot (R): too big a size → none, no draw.
    let mut x = free_room(&[10]);
    let r0 = *room_seed(&mut x.f);
    let out = cx_run(&mut x, |c| c.spread_spot(10, 40, 1, Filter::Well).map(drop));
    assert!(out.objects.is_empty());
    assert_eq!(*room_seed(&mut x.f), r0);
    // Blocked: 5 tries of 2 draws on the used seed.
    let mut x = blocked_room(&[10]);
    cx_run(&mut x, |c| c.random_spot(10, 1, 1).map(drop));
    let mut e = Seed::new(1, 666);
    adv(&mut e, 10);
    assert_eq!(x.ctl.seed, e);
    cx_run(&mut x, |c| c.spread_spot(10, 1, 1, Filter::Well).map(drop));
    let mut e = Seed::new(1, 666);
    adv(&mut e, 10);
    assert_eq!(*room_seed(&mut x.f), e);
}

// Covers: specs/world/object-population.md §6
#[test]
fn sel_sets_flag_from_selectable() {
    let mut x = free_room(&[10]);
    let mut got = None;
    cx_run(&mut x, |c| {
        got = c.fn3(32, 10)?;
        Ok(())
    });
    let u = got.expect("placed");
    // Selectable[mode 0] of blank row = 0: flag clear.
    assert_eq!(x.f.flags(u) & oflags::SELECTABLE, 0);
    x.t.objects[10].selectable0 = 1;
    let mut x2 = free_room(&[10]);
    x2.t.objects[10].selectable0 = 1;
    let mut got = None;
    cx_run(&mut x2, |c| {
        got = c.fn3(32, 10)?;
        Ok(())
    });
    assert_ne!(x2.f.flags(got.unwrap()) & oflags::SELECTABLE, 0);
}

fn fn1_blocked(a: u16, tries: usize) {
    let mut x = blocked_room(&[3, 4, 1, 79, 89, 208, 209]);
    cx_run(&mut x, |c| c.fn1(32, a));
    assert!(allocs(&x.f).is_empty());
    // prob step + 3 draws per try.
    let mut e = Seed::new(1, 666);
    adv(&mut e, 1 + 3 * tries);
    assert_eq!(x.ctl.seed, e, "A = {a}");
}

// Covers: specs/world/object-population.md §7.1 r1, §7.1 r2, §7.1 r3, §7.1 r4
#[test]
fn fn1_tries_per_class_and_unknown_class() {
    fn1_blocked(3, 18);
    for a in [1, 79, 4, 89, 208, 209] {
        fn1_blocked(a, 12);
    }
    // Other class, and count < 1: only the probability step.
    for (a, d) in [(5u16, 32), (4, 1)] {
        let mut x = free_room(&[5, 4]);
        cx_run(&mut x, |c| c.fn1(d, a));
        assert!(allocs(&x.f).is_empty());
        let mut e = Seed::new(1, 666);
        e.step();
        assert_eq!(x.ctl.seed, e);
    }
    // The class lists: 3 → {3, 28}; 4 → urns; 1 → {79, 53, 1}.
    for (a, list) in [
        (3u16, vec![3u16, 28]),
        (4, vec![4, 9, 52, 94, 95]),
        (1, vec![79, 53, 1]),
    ] {
        let mut x = free_room(&[3, 4, 1]);
        cx_run(&mut x, |c| c.fn1(125, a));
        assert!(!allocs(&x.f).is_empty());
        assert!(allocs(&x.f).iter().all(|c| list.contains(c)), "A = {a}");
    }
}

// Covers: specs/world/object-population.md §7.1 r4, §edge-cases-original-bugs r6
#[test]
fn fn1_uses_the_argument_row_for_every_list_member() {
    let mut x = free_room(&[1]);
    x.t.objects[1].sizex = 2;
    x.t.objects[1].sizey = 3;
    x.t.objects[79].sizex = 9;
    x.t.objects[79].sizey = 9;
    x.t.objects[53].sizex = 9;
    x.t.objects[53].sizey = 9;
    cx_run(&mut x, |c| c.fn1(125, 1));
    let log = x.f.box_log.borrow();
    assert!(!log.is_empty());
    for &(_, _, sx, sy, mask) in log.iter() {
        let ok = match mask {
            0xC01 => (sx, sy) == (9, 10),
            _ => (sx, sy) == (2, 3) || (sx, sy) == (4, 5),
        };
        assert!(ok, "{sx} {sy} {mask:x}");
    }
}

static ACCEPT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Fit B of a 1 × 1 object queries (sx + 2 = 3): accept the first
/// `ACCEPT` of them, block the rest.
fn accept_first_fit_b(_: i32, _: i32, sx: u32, _: u32, mask: u32) -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    if mask == 0x3F11 && sx == 3 {
        let n = SEEN.fetch_add(1, SeqCst);
        return n >= ACCEPT.load(SeqCst);
    }
    false
}

fn in_b(x: i32, y: i32) -> bool {
    x > 102 && y > 102 && x < 139 && y < 139
}

/// The §7.1 reference for A = 4 (urn list, b = 1, s = 0, walk fit B) in
/// the 40 × 40 room at (100, 100), fit B accepting `accept` queries.
fn fn1_model(mut s: Seed, mut j: i32, accept: usize) -> (Vec<(i32, i32)>, Seed) {
    let wx = [-1, 0, 1, -1, 1, -1, 0, 1];
    let wy = [-1, -1, -1, 0, 0, 1, 1, 1];
    let mut out = vec![];
    s.step();
    let (mut tries, mut seen) = (12, 0);
    while tries >= 1 {
        s.roll(5);
        let (mut x, mut y) = (100 + s.roll(38) as i32, 100 + s.roll(38) as i32);
        if !(x > 101 && y > 101 && x < 137 && y < 137) {
            tries -= 1;
            continue;
        }
        out.push((x, y));
        let (mut n, mut found) = (1, true);
        loop {
            if n >> 1 >= 1 && s.roll(n >> 1) != 0 {
                break;
            }
            if !found {
                break;
            }
            found = false;
            for _ in 0..3 * j.max(4) {
                let d = (s.step() & 7) as usize;
                x += 2 * wx[d];
                y += 2 * wy[d];
                if in_b(x, y) {
                    seen += 1;
                    if seen <= accept {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                s.roll(5);
                out.push((x, y));
                n += 1;
            }
        }
        j -= 1;
        tries -= 1;
        if j < 1 {
            break;
        }
    }
    (out, s)
}

// Covers: specs/world/object-population.md §6, §7.1 r4, §edge-cases-original-bugs r10
#[test]
fn fn1_cluster_walk_matches_reference() {
    use std::sync::atomic::Ordering::SeqCst;
    for accept in [0usize, 1, 2, 3, 5, 100] {
        for d in [32, 64] {
            for lo in 1..40u32 {
                let mut x = free_room(&[4]);
                x.ctl.seed = Seed::new(lo, 666);
                x.f.block = Some(accept_first_fit_b);
                ACCEPT.store(accept, SeqCst);
                SEEN.store(0, SeqCst);
                let j = (12 * d) / 256;
                cx_run(&mut x, |c| c.fn1(d, 4));
                let (want, seed) = fn1_model(Seed::new(lo, 666), j, accept);
                let got: Vec<_> =
                    x.f.calls
                        .iter()
                        .filter_map(|c| match c {
                            Call::Allocate(_, _, px, py, _) => Some((*px, *py)),
                            _ => None,
                        })
                        .collect();
                assert_eq!(got, want, "accept {accept} d {d} lo {lo}");
                assert_eq!(x.ctl.seed, seed, "accept {accept} d {d} lo {lo}");
            }
        }
    }
}

fn r_seed() -> Seed {
    Seed::new(1, 666)
}

// Covers: specs/world/object-population.md §7.2 r1, §7.2 r2, §7.2 r3
#[test]
fn fn2_tries_and_cap_draws() {
    // Not forced: tries = 3 (5 spot tries × 2 draws each), blocked.
    let mut x = blocked_room(&[30]);
    cx_run(&mut x, |c| c.fn2(30).map(drop));
    let mut e = r_seed();
    adv(&mut e, 1 + 3 * 10);
    assert_eq!(*room_seed(&mut x.f), e);
    // Forced (want health): 30 tries, no probability test.
    let mut x = blocked_room(&[30]);
    {
        let r = region(&mut x.ctl, LEVEL).unwrap();
        (r.w08, r.counted) = (20, 16);
    }
    cx_run(&mut x, |c| c.fn2(30).map(drop));
    let mut e = r_seed();
    adv(&mut e, 1 + 30 * 10);
    assert_eq!(*room_seed(&mut x.f), e);
    // Shrine cap: only the probability draw, nothing placed.
    let mut x = free_room(&[30]);
    region(&mut x.ctl, LEVEL).unwrap().shrines = 10;
    let out = cx_run(&mut x, |c| c.fn2(30).map(drop));
    assert!(out.objects.is_empty());
    let mut e = r_seed();
    e.step();
    assert_eq!(*room_seed(&mut x.f), e);
}

// Covers: specs/world/object-population.md §7.2 r4, §edge-cases-original-bugs r9
#[test]
fn fn2_unit_from_init_and_waypoint_shrine_rows() {
    let mut health = 0;
    let mut other = 0;
    for lo in 1..200u32 {
        // Through populate_room: a PopulateFn 2 row (waypoint shrines
        // included) goes through fn 2 and counts as a shrine.
        let mut x = fx(100);
        (x.t.objgroup[1].prob0, x.t.objgroup[1].prob1) = (100, 0);
        x.t.objgroup[1].id1 = 0;
        x.t.objects[10].populatefn = 2;
        x.t.objects[10].initfn = 1;
        x.t.shrines[2].levelmin = 0;
        x.t.objects[10].sizex = 1;
        x.t.objects[10].sizey = 1;
        *room_seed(&mut x.f) = Seed::new(lo, 666);
        x.ctl.seed = Seed::new(lo, 7);
        run_room(&mut x);
        let reg = *region(&mut x.ctl, LEVEL).unwrap();
        let placed = allocs(&x.f);
        assert!(placed.len() <= 1, "at most one shrine per call");
        if placed.is_empty() {
            continue;
        }
        let u = x.f.calls.iter().find_map(|c| match c {
            Call::Allocate(..) => Some(()),
            _ => None,
        });
        assert!(u.is_some());
        assert_eq!(reg.shrines, 1);
        let id = x.ctl.data.keys().next().copied().unwrap();
        let d = x.ctl.get(id).unwrap();
        assert_eq!(reg.shrine_points[0], x.f.positions[&id]);
        if d.interact == 2 {
            assert_eq!(reg.health, 1);
            assert!(!x
                .f
                .calls
                .iter()
                .any(|c| matches!(c, Call::Other(s) if s.starts_with("class"))));
            health += 1;
        } else {
            assert_eq!(reg.health, 0, "not forced: only a health init counts");
            other += 1;
        }
    }
    assert!(health > 0 && other > 0, "{health} {other}");
}

// Covers: specs/world/object-population.md §7.3, §edge-cases-original-bugs r8
#[test]
fn fn3_draws_density_and_last_unit() {
    // Blocked: prob + count × 5 × 2.
    for (d, count) in [(32, 1usize), (64, 3)] {
        let mut x = blocked_room(&[10]);
        let got = cx_run(&mut x, |c| c.fn3(d, 10).map(drop));
        assert!(got.objects.is_empty());
        let mut e = r_seed();
        adv(&mut e, 1 + count * 10);
        assert_eq!(x.ctl.seed, e);
    }
    // Density above 128 is fatal; 128 is fine.
    let mut x = free_room(&[10]);
    let mut out = Populated::default();
    let mut cx = Cx {
        ctl: &mut x.ctl,
        t: &x.t,
        w: &mut x.f,
        info: &x.info,
        out: &mut out,
    };
    assert_eq!(cx.fn3(129, 10), Err(ObjectError::Density(129)));
    assert!(cx.fn3(128, 10).is_ok());
    // Only the last try counts: the first spot places, the rest fail.
    static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    fn after_first(_: i32, _: i32, _: u32, _: u32, _: u32) -> bool {
        FIRST.swap(true, std::sync::atomic::Ordering::SeqCst)
    }
    let mut x = free_room(&[10]);
    x.f.block = Some(after_first);
    let mut last = Some(UnitId(0));
    let out = cx_run(&mut x, |c| {
        last = c.fn3(64, 10)?;
        Ok(())
    });
    assert_eq!(out.objects.len(), 1);
    assert_eq!(last, None, "none although an earlier try placed one");
}

// Covers: specs/world/object-population.md §7.6
#[test]
fn fn6_flies_on_the_last_unit() {
    let (mut with, mut without) = (0, 0);
    for lo in 1..60u32 {
        let setup = || {
            let mut x = free_room(&[10]);
            x.ctl.seed = Seed::new(lo, 666);
            x
        };
        // Reference: fn 3 alone, then the next C step.
        let mut a = setup();
        let mut u = None;
        cx_run(&mut a, |c| {
            u = c.fn3(32, 10)?;
            Ok(())
        });
        let mut probe = a.ctl.seed;
        let flies = u.is_some() && probe.step() % 100 > 70;
        let mut b = setup();
        let out = cx_run(&mut b, |c| c.fn6(32, 10));
        if flies {
            assert_eq!(out.objects.len(), 2);
            let u = u.unwrap();
            assert_eq!(b.f.positions[&out.objects[1]], b.f.positions[&u]);
            assert_eq!(*allocs(&b.f).last().unwrap(), 103);
            with += 1;
        } else {
            assert_eq!(out.objects.len(), usize::from(u.is_some()));
            without += 1;
        }
        // One extra C draw exactly when fn 3 returned a unit.
        let mut e = a.ctl.seed;
        if u.is_some() {
            e.step();
        }
        assert_eq!(b.ctl.seed, e);
    }
    assert!(with > 0 && without > 0);
    // fn 3 returned none: no flies draw.
    let mut x = blocked_room(&[10]);
    cx_run(&mut x, |c| c.fn6(32, 10));
    let mut e = r_seed();
    adv(&mut e, 11);
    assert_eq!(x.ctl.seed, e);
}

// Covers: specs/world/object-population.md §7.4 r1, §7.4 r2
#[test]
fn fn4_draws_cap_and_spacing() {
    // Blocked: prob, then 2j tries of (class, x, y).
    let mut x = blocked_room(&[7]);
    cx_run(&mut x, |c| c.fn4(32));
    let mut e = r_seed();
    adv(&mut e, 1 + 6);
    assert_eq!(x.ctl.seed, e);
    // Density above 128 is fatal (row 7 whatever A is).
    let mut x = free_room(&[7]);
    let mut out = Populated::default();
    let mut cx = Cx {
        ctl: &mut x.ctl,
        t: &x.t,
        w: &mut x.f,
        info: &x.info,
        out: &mut out,
    };
    assert_eq!(cx.fn4(129), Err(ObjectError::Density(129)));
    // Free room, large density: at most 8 barrels, classes 7 / 11.
    let mut x = free_room(&[7]);
    x.t.objects[7].xspace = 3;
    x.t.objects[7].yspace = 3;
    cx_run(&mut x, |c| c.fn4(128));
    let a = allocs(&x.f);
    assert!(!a.is_empty() && a.len() <= 8, "{a:?}");
    assert!(a.iter().all(|c| *c == 7 || *c == 11));
}

// Covers: specs/world/object-population.md §7.5 r1, §7.5 r2
#[test]
fn fn5_draws_classes_and_cap() {
    // Blocked, A = 46: no class draw; A = 4: roll(5) + x + y per try.
    for (a, per_try) in [(46u16, 2usize), (4, 3)] {
        let mut x = blocked_room(&[46, 4]);
        cx_run(&mut x, |c| c.fn5(32, a));
        assert!(allocs(&x.f).is_empty());
        let mut e = r_seed();
        adv(&mut e, 1 + 2 * per_try);
        assert_eq!(x.ctl.seed, e, "A = {a}");
    }
    // count < 1: only the probability step.
    let mut x = free_room(&[4]);
    cx_run(&mut x, |c| c.fn5(1, 4));
    let mut e = r_seed();
    e.step();
    assert_eq!(x.ctl.seed, e);
    // Free room: crates stay class 46; others come from the urn list. The
    // walk allocations are counted (at most 8); the first object of each
    // of the j = 6 clusters is not. (The spec's summary line "at most 9
    // objects per call" disagrees with its own step 3; the steps are
    // followed.)
    let mut x = free_room(&[46, 4]);
    x.t.objects[46].xspace = 2;
    x.t.objects[46].yspace = 2;
    x.t.objects[4].xspace = 2;
    x.t.objects[4].yspace = 2;
    cx_run(&mut x, |c| c.fn5(128, 46));
    assert!(!allocs(&x.f).is_empty() && allocs(&x.f).len() <= 8 + 6);
    assert!(allocs(&x.f).iter().all(|c| *c == 46));
    let mut x = free_room(&[46, 4]);
    x.t.objects[4].xspace = 2;
    x.t.objects[4].yspace = 2;
    cx_run(&mut x, |c| c.fn5(128, 4));
    assert!(!allocs(&x.f).is_empty() && allocs(&x.f).len() <= 8 + 6);
    assert!(allocs(&x.f).iter().all(|c| [4, 9, 52, 94, 95].contains(c)));
}

// Covers: specs/world/object-population.md §7.7 r1, §7.7 r2, §7.7 r3, §7.7 r4
#[test]
fn fn7_draws() {
    // Blocked: prob, roll(4), 8 × (x, y).
    let mut x = blocked_room(&[10]);
    cx_run(&mut x, |c| c.fn7(10));
    assert!(allocs(&x.f).is_empty());
    let mut e = r_seed();
    adv(&mut e, 2 + 16);
    assert_eq!(x.ctl.seed, e);
    // Free: tries until Fit A (5, 5) passes: x in 102..=128 of 100 + 38.
    let mut x = free_room(&[10]);
    cx_run(&mut x, |c| c.fn7(10));
    let mut e = r_seed();
    adv(&mut e, 2);
    for _ in 0..8 {
        let (a, b) = (100 + e.roll(38) as i32, 100 + e.roll(38) as i32);
        if a > 101 && b > 101 && a < 133 && b < 133 {
            break;
        }
    }
    assert_eq!(x.ctl.seed, e);
    assert!(
        allocs(&x.f).is_empty(),
        "object population pattern counts are zero"
    );
    assert_eq!(ROGUE_COUNTS, [0; 4]);
}

// Covers: specs/world/object-population.md §7.8 r2, §7.8 r3, §7.8 r4
#[test]
fn fn8_well_placement_and_spacing() {
    let mut x = free_room(&[10]);
    x.t.objects[10].selectable0 = 1;
    let mut e = r_seed();
    e.step();
    let (px, py) = loop {
        // Spread sizes (3, 3): roll(w − 3 − 1).
        let (a, b) = (100 + e.roll(36) as i32, 100 + e.roll(36) as i32);
        if a > 100 && b > 100 {
            break (a, b);
        }
    };
    let mut got = None;
    cx_run(&mut x, |c| {
        got = c.fn8(32, 10)?;
        Ok(())
    });
    let u = got.expect("placed");
    assert_eq!(x.f.positions[&u], (px, py));
    assert_ne!(x.f.flags(u) & oflags::SELECTABLE, 0, "sel");
    assert_eq!(*room_seed(&mut x.f), e);
    let r = *region(&mut x.ctl, LEVEL).unwrap();
    assert_eq!((r.wells, r.well_points[0]), (1, (px, py)));
    // The 40 × 40 room is within 100 of the recorded well on both axes:
    // the next well is refused after 5 tries, drawing 1 + 10 steps.
    let before = *room_seed(&mut x.f);
    let mut e = before;
    adv(&mut e, 11);
    let mut got = Some(u);
    cx_run(&mut x, |c| {
        got = c.fn8(32, 10)?;
        Ok(())
    });
    assert_eq!(got, None);
    assert_eq!(*room_seed(&mut x.f), e);
    assert_eq!(region(&mut x.ctl, LEVEL).unwrap().wells, 1);
    // Density above 128 is fatal.
    let mut x = free_room(&[10]);
    let mut out = Populated::default();
    let mut cx = Cx {
        ctl: &mut x.ctl,
        t: &x.t,
        w: &mut x.f,
        info: &x.info,
        out: &mut out,
    };
    assert_eq!(cx.fn8(129, 10), Err(ObjectError::Density(129)));
}

// Covers: specs/world/object-population.md §7.9
#[test]
fn fn9_at_most_one_object() {
    // Free room, density 128 (count 6): the first spot places, return.
    let mut x = free_room(&[10]);
    x.t.objects[10].selectable0 = 1;
    let mut got = None;
    cx_run(&mut x, |c| {
        got = c.fn9(128, 10)?;
        Ok(())
    });
    let u = got.expect("placed");
    assert_eq!(x.f.calls_of(|c| matches!(c, Call::Allocate(..))).len(), 1);
    assert_ne!(x.f.flags(u) & oflags::SELECTABLE, 0);
    let mut e = r_seed();
    e.step();
    assert_eq!(*room_seed(&mut x.f), e, "one R step (probability)");
    // Blocked: count × 5 × 2 C draws.
    let mut x = blocked_room(&[10]);
    let mut got = Some(UnitId(0));
    cx_run(&mut x, |c| {
        got = c.fn9(128, 10)?;
        Ok(())
    });
    assert_eq!(got, None);
    let mut e = r_seed();
    adv(&mut e, 60);
    assert_eq!(x.ctl.seed, e);
    // Density above 128 is fatal.
    let mut x = free_room(&[10]);
    let mut out = Populated::default();
    let mut cx = Cx {
        ctl: &mut x.ctl,
        t: &x.t,
        w: &mut x.f,
        info: &x.info,
        out: &mut out,
    };
    assert_eq!(cx.fn9(129, 10), Err(ObjectError::Density(129)));
}

// Covers: specs/world/objects-2.md §22 r1
#[test]
fn population_allocates_in_mode_0() {
    let mut x = free_room(&[3, 4, 7, 46, 10]);
    x.t.objects[46].xspace = 2;
    x.t.objects[46].yspace = 2;
    x.t.objects[7].xspace = 2;
    x.t.objects[7].yspace = 2;
    x.t.objects[4].xspace = 2;
    x.t.objects[4].yspace = 2;
    cx_run(&mut x, |c| c.fn1(125, 3));
    cx_run(&mut x, |c| c.fn3(125, 10).map(drop));
    cx_run(&mut x, |c| c.fn4(125));
    cx_run(&mut x, |c| c.fn5(125, 46));
    cx_run(&mut x, |c| c.fn6(125, 10));
    cx_run(&mut x, |c| c.fn9(125, 10).map(drop));
    let modes: Vec<u8> =
        x.f.calls
            .iter()
            .filter_map(|c| match c {
                Call::Allocate(.., m) => Some(*m),
                _ => None,
            })
            .collect();
    assert!(modes.len() > 5);
    assert!(modes.iter().all(|&m| m == 0));
}
