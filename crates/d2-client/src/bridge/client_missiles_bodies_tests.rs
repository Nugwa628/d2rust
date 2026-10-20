//! The client missile bodies of [`super::bodies`] and the motion record
//! of the create, on synthetic rows.

use super::*;
use crate::bridge::world::UnitOrigin;
use crate::rules::unit_composite::motion;
use d2_data::tables::{Missiles, Record, Skilldesc};
use d2_sim::skills::SkillTables;

fn row(f: u16) -> ClientMissileRow {
    ClientMissileRow {
        range: 40,
        anim_len: 3,
        anim_speed: 16,
        clt_do_func: f,
        ..ClientMissileRow::default()
    }
}

/// The sounds queued so far, without the
/// create's owner stop.
fn prog_sounds(w: &ClientWorld) -> Vec<MissileSound> {
    w.objclient
        .missile_sounds
        .iter()
        .copied()
        .filter(|s| !matches!(s, MissileSound::StopOwnerGroup { .. }))
        .collect()
}

fn at(x: i32, y: i32, class: u32) -> CreateRecord {
    CreateRecord {
        flags: flag::POSITION,
        class,
        x,
        y,
        level: 3,
        ..CreateRecord::default()
    }
}

fn env(rows: &[ClientMissileRow]) -> Env<'_> {
    Env {
        rows,
        lights: true,
        skills: None,
        monsters: &[],
        model: None,
    }
}

#[test]
fn function_2_holds_blood_on_screen_and_ends_it_off_screen() {
    let rows = vec![ClientMissileRow::default(), row(FN_BLOOD)];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    // No drawn frame yet: the origin is unknown.
    assert!(update(&mut w, &rows, k, true).is_err());
    let m = w.objclient.missiles[&k];
    let p = crate::rules::camera::moving_to_client(m.pos.0, m.pos.1);
    // On screen: (10, 10) from the origin getters.
    w.unit_origin = Some(UnitOrigin {
        x: p.x - 10,
        y: p.y - 10,
        width: 800,
        play_height: 560,
    });
    update(&mut w, &rows, k, true).unwrap();
    // Frames left := 128, then the step counts one down.
    assert_eq!(w.objclient.missiles[&k].current, 127);
    // The box is inclusive: y up to H − 40 + 64.
    w.unit_origin = Some(UnitOrigin {
        x: p.x,
        y: p.y - 624,
        width: 800,
        play_height: 560,
    });
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.missiles[&k].current, 127);
    // At the animation end: flat and path velocity 0.
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    (m.frame, m.velocity) = (2 << 8, 99);
    update(&mut w, &rows, k, true).unwrap();
    let m = w.objclient.missiles[&k];
    assert!(m.flat && m.velocity == 0);
    // Off screen (one pixel past the right edge): end(none, 0).
    w.unit_origin = Some(UnitOrigin {
        x: p.x - 801,
        y: p.y,
        width: 800,
        play_height: 560,
    });
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Skills tables whose `misscode` holds the constant 2 at offset 0
/// (0x07 n pushes n, 0x00 ends).
fn tables() -> SkillTables {
    SkillTables {
        skills: Vec::new(),
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![blank::<Missiles>(); 3],
        skills_code: Vec::new(),
        miss_code: vec![0x07, 2, 0x00],
        level_cap: 99,
        stat_count: 359,
    }
}

#[test]
fn function_3_drops_a_looping_sub_missile_on_each_new_sub_tile() {
    let mut r = row(FN_SUB_AT_STEP);
    r.clt_sub = [2, -1, -1];
    let mut child = row(FN_DEFAULT_STEP);
    (child.range, child.sub_loop, child.sub_start, child.sub_stop) = (4, 1, 1, 3);
    let t = tables();
    for (calc, loops) in [(0, true), (99, false)] {
        r.clt_calc1 = calc;
        let rows = vec![ClientMissileRow::default(), r, child];
        let e = Env {
            skills: Some(&t),
            ..env(&rows)
        };
        let mut w = ClientWorld::default();
        let k = create(&mut w, &rows, &at(100, 100, 1), true)
            .unwrap()
            .unwrap();
        // No new sub-tile: nothing made.
        update_with(&mut w, &e, k).unwrap();
        assert_eq!(w.objclient.set_c.len(), 1);
        // The flag of the last step (a stopped missile keeps it).
        w.objclient.missiles.get_mut(&k).unwrap().new_step = true;
        update_with(&mut w, &e, k).unwrap();
        let kids: Vec<_> = w
            .objclient
            .set_c
            .iter()
            .filter(|(_, u)| u.class == 2)
            .map(|(c, u)| (*c, u.position))
            .collect();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].1, Some((100, 100)));
        let c = w.objclient.missiles[&kids[0].0];
        // v = 2 > 0: flag 8 with R+0x34 = 2 × 3 − 2 = 4 loops: F = 4 +
        // (3 − 1) × 4; no formula (offset past the buffer) → v = 0, F = 4.
        assert_eq!(c.total, if loops { 12 } else { 4 }, "calc {calc}");
        assert_eq!((c.velocity, c.skill, c.level), (0, 0, 3));
    }
    // Without the skills tables the evaluator cannot run.
    let rows = vec![ClientMissileRow::default(), r, child];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().new_step = true;
    assert!(update(&mut w, &rows, k, true).is_err());
}

#[test]
fn function_59_lives_while_its_height_is_in_its_window() {
    let rows = vec![ClientMissileRow::default(), row(FN_HEIGHT_WINDOW)];
    let made = |vz: i32, d2c: i32| {
        let mut w = ClientWorld::default();
        let k = create(&mut w, &rows, &at(100, 100, 1), true)
            .unwrap()
            .unwrap();
        let m = w.objclient.missiles.get_mut(&k).unwrap();
        (m.motion.vel[2], m.d2c, m.d28) = (vz, d2c, 0);
        update(&mut w, &rows, k, true).unwrap();
        (w, k)
    };
    // z = 10 ≪ above d2C = 0: removed.
    let (w, k) = made(10 << 11, 0);
    assert!(!w.objclient.set_c.contains_key(&k));
    // Inside the window: stepped.
    let (w, k) = made(10 << 11, 20 << 11);
    assert_eq!(w.objclient.missiles[&k].current, 39);
    // Falling below the limits stops the record at z = 0: inside.
    let (w, k) = made(-(10 << 11), 0);
    assert_eq!(w.objclient.missiles[&k].motion.pos[2], 0);
    assert!(w.objclient.set_c.contains_key(&k));
}

/// Two `baalfx spirit` missiles: U (d28 = `k`) and m (d28 = U's GUID,
/// d2C = `b`), m's seed {lo, 666}.
fn spirits(rows: &[ClientMissileRow], k: i32, b: i32, lo: u32) -> (ClientWorld, UnitKey) {
    let mut w = ClientWorld::default();
    let u = create(&mut w, rows, &at(95, 100, 1), true)
        .unwrap()
        .unwrap();
    let m = create(&mut w, rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    w.objclient.missiles.get_mut(&u).unwrap().d28 = k;
    let mm = w.objclient.missiles.get_mut(&m).unwrap();
    (mm.d28, mm.d2c) = (u.guid as i32, b);
    w.objclient.set_c.get_mut(&m).unwrap().seed = Some((lo, 666));
    (w, m)
}

#[test]
fn function_65_wakes_circles_and_leaves_for_the_fixed_point() {
    let rows = vec![ClientMissileRow::default(), row(FN_SPIRIT)];
    let e = env(&rows);
    // (b = 1, k = 1 < 2: no rnd(25)): a quarter turn around U.
    let (mut w, m) = spirits(&rows, 1, 1, 5);
    update_with(&mut w, &e, m).unwrap();
    // (dx, dy) = (5, 0), negated when m's GUID has bit 1.
    let want = if m.guid & 2 != 0 {
        (100, 95)
    } else {
        (100, 105)
    };
    assert_eq!(w.objclient.missiles[&m].target_point, want);
    assert_eq!(w.objclient.set_c[&m].seed, Some((5, 666)), "no draw");
    // (b = 0, k = 0): one seed step; lo' mod 10 = 0 wakes it.
    let (wake, sleep) = {
        let mut wake = None;
        let mut sleep = None;
        for lo in 1..200u32 {
            let mut s = Seed::new(lo, 666);
            if s.step().is_multiple_of(10) {
                wake.get_or_insert(lo);
            } else {
                sleep.get_or_insert(lo);
            }
        }
        (wake.unwrap(), sleep.unwrap())
    };
    let (mut w, m) = spirits(&rows, 0, 0, sleep);
    update_with(&mut w, &e, m).unwrap();
    assert_eq!(w.objclient.missiles[&m].d2c, 0);
    let (mut w, m) = spirits(&rows, 0, 0, wake);
    update_with(&mut w, &e, m).unwrap();
    let mut s = Seed::new(wake, 666);
    s.step();
    let up = s.roll(2) != 0;
    let mm = w.objclient.missiles[&m];
    assert_eq!((mm.d2c, mm.velocity), (1, 0xF00));
    assert_eq!(mm.motion.flags & motion::RESTARTED, motion::RESTARTED);
    assert_eq!(mm.motion.vel, if up { [0, 0, 1 << 11] } else { [0; 3] });
    assert_eq!(w.objclient.set_c[&m].seed, Some((s.lo, s.hi)));
    // (b = 0 ≠ k = 2): rnd(25) = 0 → d2C := 2, off to (15135, 5900).
    let lo = (1..500u32)
        .find(|&lo| Seed::new(lo, 666).roll(25) == 0)
        .unwrap();
    let (mut w, m) = spirits(&rows, 2, 0, lo);
    update_with(&mut w, &e, m).unwrap();
    let mm = w.objclient.missiles[&m];
    assert_eq!(
        (mm.d2c, mm.velocity, mm.target_point),
        (2, 0xF00, (15135, 5900))
    );
    // No U: step only.
    let (mut w, m) = spirits(&rows, 2, 0, lo);
    w.objclient.missiles.get_mut(&m).unwrap().d28 = 9999;
    update_with(&mut w, &e, m).unwrap();
    assert_eq!(w.objclient.missiles[&m].d2c, 0);
    assert_eq!(w.objclient.set_c[&m].seed, Some((lo, 666)));
}

#[test]
fn a_landed_create_arc_holds_until_its_frames_run_out() {
    let rows = vec![ClientMissileRow::default(), row(FN_DEFAULT_STEP)];
    let mut w = ClientWorld::default();
    // Without flag 0x100: restarted, never "not moving".
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    assert_eq!(w.objclient.missiles[&k].motion.flags, motion::RESTARTED);
    update(&mut w, &rows, k, true).unwrap();
    assert!(w.objclient.missiles[&k].motion.moving());
    // Flag 0x100: the timed arc over the 40 frames (height 6), then one
    // motion update at the create.
    let mut rec = at(100, 100, 1);
    rec.flags |= flag::ARC | flag::TARGET_ABSOLUTE;
    (rec.tx, rec.ty, rec.arc_height) = (103, 104, 6);
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    let m = w.objclient.missiles[&k];
    assert_eq!(m.direction, ((103 + 104) & 63) as u8);
    // The arc ORs flag 2 onto the restart's flag 8.
    assert_eq!(
        (m.motion.flags, m.motion.ticks_left),
        (motion::RESTARTED | motion::TIMED, 39)
    );
    let n = 40;
    let vz = (-6 * 2048 - (-0x1000 * n * n / 2)) / n;
    assert_eq!(m.motion.acc[2], -0x1000);
    assert_eq!(m.motion.pos[2], (6 << 11) + vz);
    assert_eq!(m.motion.vel[2], vz - 0x1000);
    // Landed (ticks run out): flag 1 set but flag 8 stays, so does
    // not end it; it lives on while frames are left.
    w.objclient.missiles.get_mut(&k).unwrap().motion.ticks_left = 0;
    update(&mut w, &rows, k, true).unwrap();
    let m = w.objclient.missiles[&k];
    assert_eq!(m.motion.flags & motion::DONE, motion::DONE);
    assert!(m.motion.moving());
    assert!(w.objclient.set_c.contains_key(&k));
}

#[test]
fn the_explosion_takes_the_missiles_motion_position() {
    let mut r = row(FN_DEFAULT_STEP);
    (r.always_explode, r.explosion_missile) = (true, 2);
    let mut boom = row(FN_DEFAULT_STEP);
    boom.explosion_missile = -1;
    let rows = vec![ClientMissileRow::default(), r, boom];
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = crate::bridge::world::ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    let mut rec = at(100, 100, 1);
    rec.owner = Some(p);
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().motion.pos = [1, 2, 3 << 11];
    let x = end(&mut w, &rows, k, false, true).unwrap().unwrap();
    let xm = w.objclient.missiles[&x].motion;
    // The getters return the stored value >> 11 (Open question
    // 10): the low 11 bits are cleared. Done then restarted:
    // flag 8, flag 1 clear.
    assert_eq!((xm.pos, xm.flags), ([0, 0, 3 << 11], motion::RESTARTED));
}

/// One active room (90, 90)–(110, 110) of `level` (1 = the Rogue
/// Encampment, a town).
fn room_world(level: u16) -> ClientWorld {
    use crate::bridge::drlg::DrlgRoomId;
    use crate::bridge::world::ActiveRoom;
    ClientWorld {
        active_rooms: Some(vec![ActiveRoom {
            x0: 90,
            y0: 90,
            w: 20,
            h: 20,
            level,
            room: DrlgRoomId(1),
        }]),
        ..ClientWorld::default()
    }
}

#[test]
fn town_rooms_remove_missiles_without_town_and_clamp_src_town() {
    let mut town_row = row(FN_DEFAULT_STEP);
    town_row.town = true;
    let rows = vec![ClientMissileRow::default(), row(FN_DEFAULT_STEP), town_row];
    // No `Town` in a town room → removed; with `Town` stepped.
    let mut w = room_world(1);
    let gone = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    let kept = create(&mut w, &rows, &at(100, 100, 2), true)
        .unwrap()
        .unwrap();
    update(&mut w, &rows, gone, true).unwrap();
    update(&mut w, &rows, kept, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&gone));
    assert_eq!(w.objclient.missiles[&kept].current, 39);
    // Out of town: stepped.
    let mut w = room_world(2);
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.missiles[&k].current, 39);
    // `CltSrcTown` 5, no owner: frames left 39 ≥ 5 → 5, frame
    // (`AnimLen` 8 − 5) << 8; a looping row keeps its frame.
    for looping in [false, true] {
        let mut r = row(FN_DEFAULT_STEP);
        (r.clt_src_town, r.anim_len, r.anim_speed, r.loop_anim) = (5, 8, 0, looping);
        let rows = vec![ClientMissileRow::default(), r];
        let mut w = ClientWorld::default();
        let k = create(&mut w, &rows, &at(100, 100, 1), true)
            .unwrap()
            .unwrap();
        update(&mut w, &rows, k, true).unwrap();
        let m = w.objclient.missiles[&k];
        assert_eq!(m.current, 5);
        assert_eq!(m.frame, if looping { 0 } else { 3 << 8 });
        // An owner out of town: no clamp.
        let mut w = ClientWorld::default();
        let p = UnitKey::new(PLAYER, 1);
        let mut u = crate::bridge::world::ClientUnit::new(p);
        u.position = Some((100, 100));
        w.units.insert(p, u);
        let mut rec = at(100, 100, 1);
        rec.owner = Some(p);
        let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
        update(&mut w, &rows, k, true).unwrap();
        assert_eq!(w.objclient.missiles[&k].current, 39);
    }
}

#[test]
fn function_7_steers_toward_a_hostile_living_target_in_range() {
    use crate::bridge::world::{ClientUnit, MonsterClass, MonsterSetup, MONSTER};
    let mut r = row(FN_GUIDED);
    (r.vel, r.clt_param) = (8, [2, 0, 0, 0, 0]);
    let rows = vec![ClientMissileRow::default(), r];
    let monsters = vec![
        Some(MonsterClass::default()),
        Some(MonsterClass {
            setup: Some(MonsterSetup {
                align: 1,
                ..MonsterSetup::default()
            }),
            ..MonsterClass::default()
        }),
    ];
    let e = Env {
        monsters: &monsters,
        ..env(&rows)
    };
    let p = UnitKey::new(PLAYER, 1);
    let t = UnitKey::new(MONSTER, 9);
    let run = |class: u32, at_t: (u16, u16), mode: u32, d28: i32| {
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(p);
        u.position = Some((100, 100));
        w.units.insert(p, u);
        let mut u = ClientUnit::new(t);
        (u.position, u.class, u.mode) = (Some(at_t), class, mode);
        w.units.insert(t, u);
        let mut rec = at(100, 100, 1);
        rec.flags |= flag::TARGET_RELATIVE;
        (rec.owner, rec.target, rec.tx) = (Some(p), Some(t), 1);
        rec.owner_dir64 = Some(0);
        let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
        let m = w.objclient.missiles.get_mut(&k).unwrap();
        // Aim away first, so a re-path shows.
        (m.dir_vec, m.d28) = ((-4096, 0), d28);
        update_with(&mut w, &e, k).unwrap();
        (w.objclient.missiles[&k].dir_vec, w)
    };
    // Hostile, alive, distance 10 at elapsed 0: re-path toward T (east).
    let (v, _) = run(0, (110, 100), 1, 0);
    assert!(v.0 > 0 && v.1 == 0, "{v:?}");
    // Too near (distance 2 < 4), an ally (`Align` 1), or dead: no re-path.
    assert_eq!(run(0, (102, 100), 1, 0).0, (-4096, 0));
    assert_eq!(run(1, (110, 100), 1, 0).0, (-4096, 0));
    assert_eq!(run(0, (110, 100), 12, 0).0, (-4096, 0));
    // A town room removes it (even with `Town`, which passes).
    r.town = true;
    let rows = vec![ClientMissileRow::default(), r];
    let e = Env {
        monsters: &monsters,
        ..env(&rows)
    };
    let mut w = room_world(1);
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().velocity = 0;
    update_with(&mut w, &e, k).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
    // d28 bit 0: a target unit gone from the model is dropped.
    let (_, mut w) = run(0, (110, 100), 1, 1);
    w.units.remove(&t);
    let k = *w.objclient.missiles.keys().next().unwrap();
    update_with(&mut w, &e, k).unwrap();
    assert_eq!(w.objclient.missiles[&k].target_unit, None);
}

#[test]
fn function_9_drops_its_meteor_and_grows_its_light() {
    use d2_data::tables::Skills;
    let mut r = row(FN_METEOR);
    (r.light, r.clt_sub, r.clt_param) = (2, [2, 3, -1], [5, 3, 2, 0, 0]);
    let mut child = row(FN_DEFAULT_STEP);
    child.light = 0;
    let rows = vec![ClientMissileRow::default(), r, child, child];
    // Skill 0's calc1 at offset 0: the constant 6 → q = 10.
    let mut t = tables();
    t.skills = vec![blank::<Skills>()];
    t.skills_code = vec![0x07, 6, 0x00];
    let e = Env {
        skills: Some(&t),
        ..env(&rows)
    };
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    update_with(&mut w, &e, k).unwrap();
    // Elapsed 0: S1 then S2 (> 0), motion (−c·a, 0, b·a) ≪ = (−10, 0,
    // 15) ≪, velocity (2, 0, −3) ≪ (a, b, c = 5, 3, 2).
    for class in [2, 3] {
        let kids: Vec<_> = w
            .objclient
            .set_c
            .iter()
            .filter(|(_, u)| u.class == class)
            .map(|(c, _)| *c)
            .collect();
        assert_eq!(kids.len(), 1, "class {class}");
        let mm = w.objclient.missiles[&kids[0]].motion;
        assert_eq!(mm.pos, [-10 << 11, 0, 15 << 11]);
        assert_eq!(mm.vel, [2 << 11, 0, -3 << 11]);
    }
    // The light (radius 2) grows by one on (elapsed + 1) mod 10 = 0, up
    // to n = 6.
    let id = super::light_of(&w, k).unwrap();
    for _ in 1..9 {
        update_with(&mut w, &e, k).unwrap();
    }
    assert_eq!(w.lights.radius(id), Some(2));
    update_with(&mut w, &e, k).unwrap();
    assert_eq!(w.lights.radius(id), Some(3));
    // Elapsed a − 2 = 3: the head detach
    // (`ProgSound` 0: no request), once.
    assert_eq!(
        w.objclient.missile_sounds,
        vec![MissileSound::DetachHead { missile: k }]
    );
    // No skills tables: an error; a skill outside the table: removed.
    assert!(update(&mut w, &rows, k, true).is_err());
    w.objclient.missiles.get_mut(&k).unwrap().skill = 5;
    update_with(&mut w, &e, k).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn hostility_between_client_units() {
    use crate::bridge::combat::hostile_between;
    use crate::bridge::world::{ClientUnit, MonsterClass, MonsterSetup, ITEM, MONSTER};
    let monsters = vec![
        Some(MonsterClass::default()),
        Some(MonsterClass {
            setup: Some(MonsterSetup {
                align: 1,
                ..MonsterSetup::default()
            }),
            ..MonsterClass::default()
        }),
    ];
    let mut w = ClientWorld::default();
    let (p, q) = (UnitKey::new(PLAYER, 1), UnitKey::new(PLAYER, 2));
    let (foe, ally) = (UnitKey::new(MONSTER, 3), UnitKey::new(MONSTER, 4));
    for (k, class) in [(p, 0), (q, 0), (foe, 0), (ally, 1)] {
        let mut u = ClientUnit::new(k);
        u.class = class;
        w.units.insert(k, u);
    }
    let h = |a, b| hostile_between(&w, &monsters, a, b);
    assert!(h(p, foe) && h(foe, p) && h(ally, foe));
    assert!(!h(p, ally) && !h(p, p) && !h(p, q) && !h(foe, foe));
    assert!(h(p, UnitKey::new(ITEM, 5)));
}

/// A world with a player owner at (100, 100) and one missile of
/// `rows[1]` it owns at (100, 100).
fn owned(rows: &[ClientMissileRow]) -> (ClientWorld, UnitKey, UnitKey) {
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = crate::bridge::world::ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    let mut rec = at(100, 100, 1);
    rec.owner = Some(p);
    let k = create(&mut w, rows, &rec, true).unwrap().unwrap();
    (w, p, k)
}

/// The set-C missiles of `class` in GUID order (creation order).
fn kids(w: &ClientWorld, class: u32) -> Vec<UnitKey> {
    w.objclient
        .set_c
        .iter()
        .filter(|(_, u)| u.class == class)
        .map(|(c, _)| *c)
        .collect()
}

fn cells(w: &ClientWorld, class: u32) -> Vec<(i32, i32)> {
    kids(w, class)
        .iter()
        .map(|c| super::cell_of(&w.objclient.missiles[c]))
        .collect()
}

#[test]
fn function_39_rewinds_an_owned_missile() {
    let mut r = row(FN_FRAME_ZERO);
    r.anim_speed = 0;
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().frame = 2 << 8;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.missiles[&k].frame, 0);
    // No owner: the frame stays.
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().frame = 2 << 8;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.missiles[&k].frame, 2 << 8);
}

#[test]
fn function_17_and_the_disc() {
    let mut r = row(FN_CURSE_CENTRE);
    (r.clt_sub, r.clt_param) = ([2, -1, -1], [7, 0, 0, 0, 0]);
    let mut c = row(FN_DEFAULT_STEP);
    c.rand_start = 4;
    let rows = vec![ClientMissileRow::default(), r, c];
    // The disc r = 3, s = 1, no chance: 29 points, row by row (y outer),
    // x inner; each frame offset rnd(`RandStart` 4) on m's seed.
    let (mut w, _, k) = owned(&rows);
    let mut seed = seeded(&mut w, k, 99);
    super::bodies::disc(&mut w, &env(&rows), k, 3, 2, 0, 1).unwrap();
    let got = cells(&w, 2);
    assert_eq!(got.len(), 29);
    assert_eq!(&got[..2], &[(100, 97), (98, 98)]);
    for c in kids(&w, 2) {
        let f = seed.roll(4) as i32;
        assert_eq!(w.objclient.missiles[&c].frame, f << 8);
    }
    // No owner: nothing.
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    super::bodies::disc(&mut w, &env(&rows), k, 3, 2, 0, 1).unwrap();
    assert!(kids(&w, 2).is_empty());
    // Function 17: d04 0 → S1; d06 = n = 1: disc r 1, chance 1, s 1 on
    // elapsed 0, 3, 6 (< P1 7): 5 points each.
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().d06 = 1;
    for _ in 0..8 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(kids(&w, 2).len(), 15);
    assert_eq!(
        &cells(&w, 2)[..5],
        &[(100, 99), (99, 100), (100, 100), (101, 100), (100, 101)]
    );
    // d06 ≤ 0: removed.
    let (mut w, _, k) = owned(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn function_18_lays_its_trail_with_the_spears_facing_and_height() {
    let mut r = row(FN_SPEAR_TRAIL);
    r.clt_sub = [2, -1, -1];
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    let (mut w, p, k) = owned(&rows);
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    (m.new_step, m.direction) = (true, 21);
    m.pos.0 += 0x1234;
    m.motion.vel[2] = 7 << 11;
    let pos = m.pos;
    update(&mut w, &rows, k, true).unwrap();
    let c = w.objclient.missiles[&kids(&w, 2)[0]];
    assert_eq!((c.direction, c.pos, c.owner), (21, pos, Some(p)));
    // The child takes m's z after m's motion update of this update.
    assert_eq!(c.motion.pos, [0, 0, 7 << 11]);
    // A stopped missile keeps the flag of its last step: one
    // more; with the flag clear, none.
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(kids(&w, 2).len(), 2);
    w.objclient.missiles.get_mut(&k).unwrap().new_step = false;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(kids(&w, 2).len(), 2);
}

#[test]
fn a_trail_draws_at_its_creation_centre_not_the_copied_point() {
    let mut r = row(FN_SPEAR_TRAIL);
    r.clt_sub = [2, -1, -1];
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    let (mut w, _, k) = owned(&rows);
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    m.new_step = true;
    (m.pos.0, m.pos.1) = ((100 << 16) | 0x1234, (100 << 16) | 0xF000);
    let pos = m.pos;
    update(&mut w, &rows, k, true).unwrap();
    let c = kids(&w, 2)[0];
    // The raw copy moves path +0x00 / +0x04 only; the pixel pair stays
    // at the creation cell's centre.
    assert_eq!(w.objclient.missiles[&c].pos, pos);
    let centre = (
        (u32::from(w.objclient.set_c[&c].cell().0) << 16) | 0x8000,
        (u32::from(w.objclient.set_c[&c].cell().1) << 16) | 0x8000,
    );
    let u = w.view_unit(&crate::bridge::world::view_key(c)).unwrap();
    assert_eq!(u.precise, Some(centre));
    // A path step computes the pair again.
    w.objclient.missiles.get_mut(&c).unwrap().drawn_at = None;
    let u = w.view_unit(&crate::bridge::world::view_key(c)).unwrap();
    assert_eq!(u.precise, Some(pos));
}

#[test]
fn function_27_drops_fire_and_wanders() {
    let mut r = row(FN_WANDER_MAKER);
    (r.clt_sub, r.clt_param) = ([2, -1, -1], [1, 0, 0, 0, 0]);
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    for want_s in [0u32, 1, 2] {
        let lo = (1..1000u32)
            .find(|&lo| Seed::new(lo, 666).step() & 3 == want_s)
            .unwrap();
        let (mut w, _, k) = owned(&rows);
        seeded(&mut w, k, lo);
        let m = w.objclient.missiles.get_mut(&k).unwrap();
        (m.d28, m.d2c, m.new_step) = (4, 0, true);
        update(&mut w, &rows, k, true).unwrap();
        // The child starts at m's position (flags 0: the origin's).
        assert_eq!(cells(&w, 2), vec![(100, 100)]);
        // Test vectors: (4, 0) → s 0 (3, 3), s 2 (3,
        // −3), s 1 / 3 (4, 0).
        let d = match want_s {
            0 => (3, 3),
            2 => (3, -3),
            _ => (4, 0),
        };
        let m = w.objclient.missiles[&k];
        assert_eq!((m.d28, m.d2c), d, "s {want_s}");
        assert_eq!(m.target_point, (100 + d.0, 100 + d.1));
    }
}

#[test]
fn functions_46_and_52_lay_children_on_both_sides() {
    let mut r = row(FN_JAVELIN_TRAIL);
    (r.vel, r.clt_sub, r.clt_param, r.light) = (8, [2, -1, -1], [3, 0, 0, 0, 0], 5);
    let mut child = row(FN_DEFAULT_STEP);
    (child.sub_loop, child.sub_start, child.sub_stop) = (1, 1, 2);
    let rows = vec![ClientMissileRow::default(), r, child];
    let mut w = ClientWorld::default();
    let mut rec = at(100, 100, 1);
    rec.flags |= flag::TARGET_RELATIVE;
    rec.tx = 10;
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().new_step = true;
    update(&mut w, &rows, k, true).unwrap();
    // Elapsed 0 < 2: (d28, d2C) = (y − ty, tx − x) = (0, 10); children at
    // (x, y) aiming at ±(0, 10), loops P1 3 (frames 40 + 3).
    let m = w.objclient.missiles[&k];
    assert_eq!((m.d28, m.d2c), (0, 10));
    let targets: Vec<_> = kids(&w, 2)
        .iter()
        .map(|c| {
            (
                w.objclient.missiles[c].target_point,
                w.objclient.missiles[c].total,
            )
        })
        .collect();
    assert_eq!(targets, vec![((100, 110), 43), ((100, 90), 43)]);
    // S1 < 0: removed directly, its light left in the list (Edge case 2).
    r.clt_sub = [-1, -1, -1];
    let rows = vec![ClientMissileRow::default(), r];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k) && !w.lights.is_empty());
    // 52: no owner → removed (light too); with one: children from m.
    let mut r = row(FN_WAKE_MAKER);
    (r.clt_sub, r.light) = ([2, -1, -1], 5);
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k) && w.lights.is_empty());
    let (mut w, _, k) = owned(&rows);
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    (m.d28, m.d2c, m.new_step) = (2, -3, true);
    update(&mut w, &rows, k, true).unwrap();
    let targets: Vec<_> = kids(&w, 2)
        .iter()
        .map(|c| w.objclient.missiles[c].target_point)
        .collect();
    assert_eq!(targets, vec![(102, 97), (98, 103)]);
}

#[test]
fn function_51_bursts_then_hides_its_owner() {
    let mut r = row(FN_RECYCLER);
    (r.clt_sub, r.clt_param) = ([2, 3, -1], [0, 1, 2, 3, 0]);
    let rows = vec![
        ClientMissileRow::default(),
        r,
        row(FN_DEFAULT_STEP),
        row(FN_DEFAULT_STEP),
    ];
    let (mut w, p, k) = owned(&rows);
    let mut seed = seeded(&mut w, k, 31);
    update(&mut w, &rows, k, true).unwrap();
    // Elapsed 0 = P1: S1 at (x, y), then P3 = 2 more at x − 3 + rnd(7),
    // y − 3 + rnd(7).
    let mut want = vec![(100, 100)];
    for _ in 0..2 {
        let x = 97 + seed.roll(7) as i32;
        let y = 97 + seed.roll(7) as i32;
        want.push((x, y));
    }
    assert_eq!(cells(&w, 2), want);
    assert_eq!(w.units[&p].flag_ex & FLAG_EX_NOT_DRAWN, 0);
    // Elapsed 1 = P2: the owner hidden, S2 from m.
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.units[&p].flag_ex & FLAG_EX_NOT_DRAWN, FLAG_EX_NOT_DRAWN);
    assert_eq!(cells(&w, 3), vec![(100, 100)]);
    // `ProgSound` 0: no sound.
    assert!(prog_sounds(&w).is_empty());
}

#[test]
fn function_51_requests_its_prog_sound_on_the_new_missile() {
    let mut r = row(FN_RECYCLER);
    (r.prog_sound, r.clt_sub, r.clt_param) = (9, [2, 3, -1], [0, 1, 0, 0, 0]);
    let rows = vec![
        ClientMissileRow::default(),
        r,
        row(FN_DEFAULT_STEP),
        row(FN_DEFAULT_STEP),
    ];
    let (mut w, _, k) = owned(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(prog_sounds(&w).is_empty());
    update(&mut w, &rows, k, true).unwrap();
    let s2 = kids(&w, 3);
    assert_eq!(s2.len(), 1);
    assert_eq!(
        prog_sounds(&w),
        vec![MissileSound::Request {
            id: 9,
            missile: s2[0]
        }]
    );
}

/// Gives `k` the seed `{lo, 666}`.
fn seeded(w: &mut ClientWorld, k: UnitKey, lo: u32) -> Seed {
    w.objclient.set_c.get_mut(&k).unwrap().seed = Some((lo, 666));
    Seed::new(lo, 666)
}

#[test]
fn functions_19_and_20_the_orb_and_its_nova() {
    let mut orb = row(FN_FROZEN_ORB);
    (orb.clt_sub, orb.clt_param) = ([2, -1, -1], [1, 19, 0, 0, 0]);
    let rows = vec![ClientMissileRow::default(), orb, row(FN_DEFAULT_STEP)];
    // Test vectors: d28 5, P2 19 → offset (OX[5], OY[5]) = (26, 14),
    // d28 := 24; d28 −70 → d = 6.
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().d28 = 5;
    update(&mut w, &rows, k, true).unwrap();
    let c = w.objclient.missiles[&kids(&w, 2)[0]];
    assert_eq!(c.target_point, (126, 114));
    assert_eq!(w.objclient.missiles[&k].d28, 24);
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().d28 = -70;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.missiles[&k].d28, 25);
    // 20, test vector: (30, 0), P1 6, P2 2.
    let mut nova = row(FN_ORB_NOVA);
    nova.clt_param = [6, 2, 0, 0, 0];
    let rows = vec![ClientMissileRow::default(), nova];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().d28 = 30;
    let mut seen = Vec::new();
    for _ in 0..5 {
        update(&mut w, &rows, k, true).unwrap();
        let m = w.objclient.missiles[&k];
        seen.push((m.target_point, (m.d28, m.d2c)));
    }
    assert_eq!(seen[0], ((115, 115), (15, 15)));
    assert_eq!(seen[2], ((100, 115), (0, 15)));
    assert_eq!(seen[4], ((93, 107), (-7, 7)));
}

#[test]
fn functions_10_and_13_drop_falling_shards() {
    let mut mon = row(FN_MON_BLIZZARD);
    (mon.clt_sub, mon.clt_param) = ([2, -1, -1], [5, 8, 4, 0, 0]);
    let mut shard = row(FN_DEFAULT_STEP);
    shard.clt_param = [120, 5, 0, 0, 0];
    let rows = vec![ClientMissileRow::default(), mon, shard];
    // Test vector: level 12 → (r, k) = (8, 5): a shard at elapsed 0 and 5.
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().level = 12;
    update(&mut w, &rows, k, true).unwrap();
    let mut s = Seed::init_low(100);
    let sx = 100 + 7 - s.roll(14) as i32;
    let sy = 100 + 7 - s.roll(14) as i32;
    assert_eq!(cells(&w, 2), vec![(sx, sy)]);
    assert_eq!(w.objclient.set_c[&k].seed, Some((s.lo, s.hi)), "reseeded");
    // Fall (test vector: P1 120, P2 5): z 120 ≪, vz −5 ≪, 24 frames.
    let c = w.objclient.missiles[&kids(&w, 2)[0]];
    assert_eq!(
        (c.motion.pos, c.motion.vel),
        ([0, 0, 120 << 11], [0, 0, -5 << 11])
    );
    assert_eq!((c.total, c.current), (24, 24));
    for _ in 0..5 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(kids(&w, 2).len(), 2, "elapsed 5");
    // No owner: no shard.
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    update(&mut w, &rows, k, true).unwrap();
    assert!(kids(&w, 2).is_empty());
    // 13: S1 2 < S2 3 → c := 2 + rnd(2) on m's seed; r = eval(calc1) = 3,
    // k = eval(calc2) = 2 (skill 0's formulas).
    let mut bliz = row(FN_BLIZZARD);
    bliz.clt_sub = [2, 3, -1];
    let rows = vec![ClientMissileRow::default(), bliz, shard, shard];
    let mut t = tables();
    let mut sk = blank::<d2_data::tables::Skills>();
    (sk.calc1, sk.calc2) = (0, 3);
    t.skills = vec![sk];
    t.skills_code = vec![0x07, 3, 0x00, 0x07, 2, 0x00];
    let e = Env {
        skills: Some(&t),
        ..env(&rows)
    };
    let (mut w, _, k) = owned(&rows);
    let mut seed = seeded(&mut w, k, 555);
    update_with(&mut w, &e, k).unwrap();
    let c = 2 + seed.roll(2);
    let mut s = Seed::init_low(100);
    let sx = 100 + 2 - s.roll(4) as i32;
    let sy = 100 + 2 - s.roll(4) as i32;
    assert_eq!(cells(&w, c), vec![(sx, sy)]);
    // k = 0 → removed.
    t.skills_code = vec![0x07, 3, 0x00, 0x07, 0, 0x00];
    let e = Env {
        skills: Some(&t),
        ..env(&rows)
    };
    let (mut w, _, k) = owned(&rows);
    update_with(&mut w, &e, k).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn functions_44_45_53_and_68() {
    use crate::bridge::skills::{SkillEntry, SkillList};
    // 44: m keeps to its owner while the owner uses m's skill.
    let mut d = row(FN_DISTRACTION);
    d.clt_sub = [2, -1, -1];
    let rows = vec![ClientMissileRow::default(), d, row(FN_DEFAULT_STEP)];
    let (mut w, p, k) = owned(&rows);
    let o = w.units.get_mut(&p).unwrap();
    o.position = Some((105, 103));
    o.skills = Some(SkillList {
        entries: vec![SkillEntry {
            skill: 0,
            ..SkillEntry::default()
        }],
        current: Some(0),
        ..SkillList::default()
    });
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.set_c[&k].position, Some((105, 103)));
    assert!(w.objclient.missiles[&k].new_step, "a new sub-tile");
    // The next update spawns S1 from m.
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(cells(&w, 2), vec![(105, 103)]);
    // Another skill in use: removed directly (its light stays).
    w.objclient.missiles.get_mut(&k).unwrap().skill = 9;
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
    // 45: S1 < 0 → removed.
    let mut fog = row(FN_DISTRACTION_FOG);
    fog.clt_sub = [-1, -1, -1];
    let rows = vec![ClientMissileRow::default(), fog];
    let (mut w, _, k) = owned(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
    // 53: on a new sub-tile S1 from m, then function 7 (steps).
    let mut tf = row(FN_TIGER_FURY);
    tf.clt_sub = [2, -1, -1];
    let rows = vec![ClientMissileRow::default(), tf, row(FN_DEFAULT_STEP)];
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().new_step = true;
    update_with(&mut w, &env(&rows), k).unwrap();
    assert_eq!(cells(&w, 2), vec![(100, 100)]);
    assert_eq!(w.objclient.missiles[&k].current, 39);
    // 68: S1 0 → removed; else on a new sub-tile S1 with loops P1.
    let mut sf = row(FN_SUC_FIREBALL);
    (sf.clt_sub, sf.clt_param) = ([2, -1, -1], [3, 0, 0, 0, 0]);
    let mut child = row(FN_DEFAULT_STEP);
    (child.sub_loop, child.sub_start, child.sub_stop) = (1, 1, 2);
    let rows = vec![ClientMissileRow::default(), sf, child];
    let (mut w, _, k) = owned(&rows);
    w.objclient.missiles.get_mut(&k).unwrap().new_step = true;
    update(&mut w, &rows, k, true).unwrap();
    let c = w.objclient.missiles[&kids(&w, 2)[0]];
    assert_eq!(c.total, 43);
    sf.clt_sub = [0, -1, -1];
    let rows = vec![ClientMissileRow::default(), sf];
    let (mut w, _, k) = owned(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn function_48_erupts_at_reseeded_points() {
    let mut r = row(FN_ERUPTION);
    r.clt_sub = [2, 3, -1];
    let rows = vec![
        ClientMissileRow::default(),
        r,
        row(FN_DEFAULT_STEP),
        row(FN_DEFAULT_STEP),
    ];
    let mut t = tables();
    let mut sk = blank::<d2_data::tables::Skills>();
    (sk.calc1, sk.calc2) = (0, 3);
    t.skills = vec![sk];
    t.skills_code = vec![0x07, 3, 0x00, 0x07, 1, 0x00];
    let e = Env {
        skills: Some(&t),
        ..env(&rows)
    };
    let (mut w, _, k) = owned(&rows);
    update_with(&mut w, &e, k).unwrap();
    // r 3, k 1: reseed(x + 0); P := (x + rnd(4) − 2, y + rnd(4) − 2).
    let mut s = Seed::init_low(100);
    let px = 100 + s.roll(4) as i32 - 2;
    let py = 100 + s.roll(4) as i32 - 2;
    assert_eq!(cells(&w, 2), vec![(px, py)]);
    assert_eq!(cells(&w, 3), vec![(px, py)]);
    assert!(w.objclient.missiles[&kids(&w, 2)[0]].flat, "dead flag");
    assert!(!w.objclient.missiles[&kids(&w, 3)[0]].flat);
    // No owner: removed.
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    update_with(&mut w, &e, k).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn function_58_turns_on_its_reseeded_draw() {
    let mut r = row(FN_CHAOS_ICE);
    r.param = [1, 0];
    let rows = vec![ClientMissileRow::default(), r];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    // Test vector: (a, b) = (8, 0): lo' even → (8, −2), odd → (8, 2).
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    (m.d28, m.d2c) = (77, 8);
    update(&mut w, &rows, k, true).unwrap();
    let mut s = Seed::init_low(77);
    let even = s.step() & 1 == 0;
    let dy = if even { -2 } else { 2 };
    let m = w.objclient.missiles[&k];
    assert_eq!(m.target_point, (108, 100 + dy));
    assert_eq!((m.d28, m.d2c), (s.lo as i32, (dy << 16) + 8));
}

#[test]
fn function_47_keeps_its_bounces_then_lays_fire() {
    let mut r = row(FN_MOLTEN_BOULDER);
    (r.prog_sound, r.clt_sub) = (1, [-1, -1, -1]);
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, _, k) = owned(&rows);
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    (m.motion.bounces_left, m.new_step) = (3, true);
    update(&mut w, &rows, k, true).unwrap();
    // d28 := bounces left; then function 6: S1 < 0 → removed.
    assert!(!w.objclient.set_c.contains_key(&k));
    r.clt_sub = [2, -1, -1];
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    let (mut w, _, k) = owned(&rows);
    w.objclient
        .missiles
        .get_mut(&k)
        .unwrap()
        .motion
        .bounces_left = 3;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(w.objclient.missiles[&k].d28, 3);
    // The sound: d28 was 0 → none; d28 3 ≠
    // 2 bounces left → a request on m; d28 2 = 2 → none.
    assert!(prog_sounds(&w).is_empty());
    let set = |w: &mut ClientWorld, b| {
        w.objclient
            .missiles
            .get_mut(&k)
            .unwrap()
            .motion
            .bounces_left = b
    };
    set(&mut w, 2);
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(
        prog_sounds(&w),
        vec![MissileSound::Request { id: 1, missile: k }]
    );
    w.objclient.missile_sounds.clear();
    update(&mut w, &rows, k, true).unwrap();
    assert!(prog_sounds(&w).is_empty());
}

#[test]
fn a_bodys_child_nudges_by_its_parents_direction() {
    // Function 27's flags-0 child with velocity aims at its own start:
    // the nudge reads the owner's direction, here the parent's
    // (PROVISIONAL).
    let mut r = row(FN_WANDER_MAKER);
    (r.clt_sub, r.clt_param) = ([2, -1, -1], [99, 0, 0, 0, 0]);
    let mut fire = row(FN_DEFAULT_STEP);
    fire.vel = 8;
    let rows = vec![ClientMissileRow::default(), r, fire];
    let (mut w, _, k) = owned(&rows);
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    // Direction 20 → octant 2: (DX, DY) = (−2, 0) from the owner.
    (m.direction, m.new_step) = (20, true);
    update(&mut w, &rows, k, true).unwrap();
    let c = w.objclient.missiles[&kids(&w, 2)[0]];
    assert_eq!(c.target_point, (98, 100));
}

#[test]
fn client_functions_start_their_shakes() {
    use crate::rules::camera::Shake;
    let shake = |w: &ClientWorld| w.shake.map(|s| (s.shake, s.start_tick));
    let s = |a, b, c, d| Shake::start(a, b, c, d).unwrap();
    // 29, test vector: missile 307 (P1 25, P2 7), owner present: elapsed
    // 90 → (8, 2000, 3000, 2000); 89 and 91 none.
    let mut r = row(29);
    (r.range, r.clt_param) = (200, [25, 7, 0, 0, 0]);
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, _, k) = owned(&rows);
    w.server_ticks = 7;
    for _ in 0..90 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(shake(&w), None, "elapsed 89");
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(shake(&w), Some((s(8, 2000, 3000, 2000), 7)));
    w.shake = None;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(shake(&w), None, "elapsed 91");
    // 12 at elapsed 0 by the formula (P1 5, P2 4: q 1 → (5, 1000, 2000,
    // 1000)); P2 0 → removed.
    let mut r = row(12);
    r.clt_param = [5, 4, 0, 0, 0];
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, _, k) = owned(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(shake(&w), Some((s(5, 1000, 2000, 1000), 0)));
    r.clt_param = [5, 0, 0, 0, 0];
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, _, k) = owned(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k) && w.shake.is_none());
    // 31 / 37 at frames left 150, 36 at 325, 54 at elapsed 5, 38 at
    // elapsed 10 with its object.
    for (f, left, want) in [
        (31, 150, s(25, 0, 4000, 0)),
        (37, 150, s(25, 0, 4000, 0)),
        (36, 325, s(20, 0, 6000, 0)),
    ] {
        let rows = vec![ClientMissileRow::default(), row(f)];
        let (mut w, _, k) = owned(&rows);
        w.objclient.missiles.get_mut(&k).unwrap().current = left + 1;
        update(&mut w, &rows, k, true).unwrap();
        assert_eq!(shake(&w), None, "f {f}");
        update(&mut w, &rows, k, true).unwrap();
        assert_eq!(shake(&w).map(|x| x.0), Some(want), "f {f}");
    }
    let rows = vec![ClientMissileRow::default(), row(54)];
    let (mut w, _, k) = owned(&rows);
    for _ in 0..6 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(shake(&w).map(|x| x.0), Some(s(3, 600, 4000, 3000)));
    let rows = vec![ClientMissileRow::default(), row(38)];
    let (mut w, _, k) = owned(&rows);
    for _ in 0..11 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(shake(&w), None, "no object (d28 0)");
    let (mut w, _, k) = owned(&rows);
    let obj = UnitKey::new(crate::bridge::world::OBJECT, 4);
    w.units
        .insert(obj, crate::bridge::world::ClientUnit::new(obj));
    w.objclient.missiles.get_mut(&k).unwrap().d28 = 4;
    for _ in 0..11 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(shake(&w).map(|x| x.0), Some(s(4, 80, 80, 400)));
}

#[test]
fn rule_w_shakes_in_the_worldstone_levels() {
    use crate::bridge::drlg::DrlgRoomId;
    use crate::bridge::world::ActiveRoom;
    let rows = vec![ClientMissileRow::default(), row(66)];
    let world_in = |level: u16| {
        let (mut w, p, k) = owned(&rows);
        w.active_rooms = Some(vec![ActiveRoom {
            x0: 90,
            y0: 90,
            w: 20,
            h: 20,
            level,
            room: DrlgRoomId(1),
        }]);
        w.room_units.place(p, Some(DrlgRoomId(1)));
        (w, k)
    };
    // Test vector: level 131, d28 0 → the shake, d28 := 1, no seed step
    // before the start test (then one step for the mod-10 test).
    let (mut w, k) = world_in(131);
    let mut seed = seeded(&mut w, k, 40);
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(
        w.shake.map(|s| s.shake),
        crate::rules::camera::Shake::start(6, 2000, 3000, 2000)
    );
    assert_eq!(w.objclient.missiles[&k].d28, 1);
    let lo = seed.step();
    if lo.is_multiple_of(10) {
        seed.step();
        seed.step();
    }
    assert_eq!(w.objclient.set_c[&k].seed, Some((seed.lo, seed.hi)));
    // Level 130: the default function, no shake.
    let (mut w, k) = world_in(130);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k) && w.shake.is_none());
}

#[test]
fn missile_sounds_at_the_create_and_the_end() {
    let mut r = row(FN_DEFAULT_STEP);
    (r.travel_sound, r.hit_sound, r.explosion_missile) = (5, 7, -1);
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, p, k) = owned(&rows);
    // The travel sound on m, then the owner's sound-314 group
    // stop.
    assert_eq!(
        w.objclient.missile_sounds,
        vec![
            MissileSound::Request { id: 5, missile: k },
            MissileSound::StopOwnerGroup { owner: p, id: 314 },
        ]
    );
    w.objclient.missile_sounds.clear();
    // the hit sound, the travel sound detached.
    end(&mut w, &rows, k, true, true).unwrap();
    assert_eq!(
        w.objclient.missile_sounds,
        vec![
            MissileSound::Request { id: 7, missile: k },
            MissileSound::DetachTravel { missile: k, id: 5 },
        ]
    );
    // No owner, travel sound 0, hit sound −1: none.
    let mut r = row(FN_DEFAULT_STEP);
    (r.travel_sound, r.hit_sound, r.explosion_missile) = (0, -1, -1);
    let rows = vec![ClientMissileRow::default(), r];
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 1), true)
        .unwrap()
        .unwrap();
    end(&mut w, &rows, k, true, true).unwrap();
    assert_eq!(
        w.objclient.missile_sounds,
        vec![MissileSound::DetachTravel { missile: k, id: 0 }]
    );
}

#[test]
fn a_multishot_unique_monster_doubles_its_client_missiles() {
    use crate::bridge::world::{ClientUnit, KindData, MonsterData, MONSTER};
    let mut r = row(FN_DEFAULT_STEP);
    r.vel = 8;
    let mut no_ms = r;
    no_ms.no_multishot = true;
    let rows = vec![ClientMissileRow::default(), r, no_ms];
    let world_with = |flags: u8| {
        let mut w = ClientWorld::default();
        let o = UnitKey::new(MONSTER, 3);
        let mut u = ClientUnit::new(o);
        u.position = Some((100, 100));
        u.kind = KindData::Monster(Box::new(MonsterData {
            flags,
            umods: [5, 29, 0, 0, 0, 0, 0, 0, 0],
            ..MonsterData::default()
        }));
        w.units.insert(o, u);
        (w, o)
    };
    let shot = |class: u32, o: UnitKey| CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(o),
        class,
        x: 100,
        y: 100,
        tx: 110,
        ty: 104,
        ..CreateRecord::default()
    };
    // Unique (flag 8): two copies at (tx − sy, ty + sx), (tx + sy, ty −
    // sx), (sx, sy) = signs of (100 − 110, 100 − 104) = (−1, −1); the
    // copies run the hook too but the guard stops them.
    let (mut w, o) = world_with(8);
    create(&mut w, &rows, &shot(1, o), true).unwrap().unwrap();
    let mut targets: Vec<_> = w
        .objclient
        .missiles
        .values()
        .map(|m| m.target_point)
        .collect();
    targets.sort();
    assert_eq!(targets, vec![(109, 105), (110, 104), (111, 103)]);
    assert!(w.objclient.multishot_guard.is_empty());
    // Not unique, or `NoMultiShot`: none.
    let (mut w, o) = world_with(0);
    create(&mut w, &rows, &shot(1, o), true).unwrap().unwrap();
    assert_eq!(w.objclient.missiles.len(), 1);
    let (mut w, o) = world_with(8);
    create(&mut w, &rows, &shot(2, o), true).unwrap().unwrap();
    assert_eq!(w.objclient.missiles.len(), 1);
}

#[test]
fn function_15_steps_toward_its_target_and_drops_its_spawn() {
    use crate::bridge::world::{ClientUnit, MONSTER};
    let mut r = row(FN_FINGER_MAGE_SPIDER);
    // P1 2 (spawn period), P2 3 (life below 0), P3 0 (period 5), P4 20
    // (distance), P5 2 (step); S1 = row 2.
    (r.vel, r.clt_param, r.clt_sub) = (8, [2, 3, 0, 20, 2], [2, -1, -1]);
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    let e = env(&rows);
    let p = UnitKey::new(PLAYER, 1);
    let t = UnitKey::new(MONSTER, 9);
    let run = |d28: i32, current: i32, owner: bool| {
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(p);
        u.position = Some((100, 100));
        w.units.insert(p, u);
        let mut u = ClientUnit::new(t);
        u.position = Some((106, 103));
        w.units.insert(t, u);
        let mut rec = at(100, 100, 1);
        rec.flags |= flag::TARGET_RELATIVE;
        (rec.owner, rec.target, rec.tx) = (Some(p), Some(t), 1);
        rec.owner_dir64 = Some(0);
        let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
        let m = w.objclient.missiles.get_mut(&k).unwrap();
        (m.d28, m.current) = (d28, current);
        if !owner {
            w.units.remove(&p);
        }
        update_with(&mut w, &e, k).unwrap();
        (k, w)
    };
    // Alive (d28 1), frames left 40 (mod 5 and mod 2 = 0): the target
    // point moves 2 toward T on each axis, the target unit is cleared and
    // one S1 missile with d28 0 is made at m's cell.
    let (k, w) = run(1, 40, true);
    let m = w.objclient.missiles[&k];
    assert_eq!((m.target_point, m.target_unit), ((102, 102), None));
    let kids: Vec<_> = w.objclient.missiles.keys().filter(|c| **c != k).collect();
    assert_eq!(kids.len(), 1);
    assert_eq!(w.objclient.set_c[kids[0]].class, 2);
    assert_eq!(w.objclient.missiles[kids[0]].d28, 0);
    // Frames left 41: neither period hits, nothing changes but the step.
    let (k, w) = run(1, 41, true);
    assert_eq!(w.objclient.missiles[&k].target_unit, Some(t));
    assert_eq!(w.objclient.missiles.len(), 1);
    // d28 0: counts down to −1, no re-path and no spawn.
    let (k, w) = run(0, 40, true);
    let m = w.objclient.missiles[&k];
    assert_eq!((m.d28, m.target_unit), (-1, Some(t)));
    assert_eq!(w.objclient.missiles.len(), 1);
    // d28 −3 (≤ −P2) or no owner: removed.
    let (k, w) = run(-3, 40, true);
    assert!(!w.objclient.set_c.contains_key(&k));
    let (k, w) = run(1, 40, false);
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn function_16_lays_a_wall_piece_on_each_new_step_and_needs_its_owner() {
    let mut r = row(FN_DIABLO_WALL_MAKER);
    r.clt_sub = [2, -1, -1];
    let rows = vec![ClientMissileRow::default(), r, row(FN_DEFAULT_STEP)];
    let (mut w, p, k) = owned(&rows);
    let m = w.objclient.missiles.get_mut(&k).unwrap();
    m.new_step = true;
    let at = cell_of(m);
    update(&mut w, &rows, k, true).unwrap();
    let c = w.objclient.missiles[&kids(&w, 2)[0]];
    assert_eq!((cell_of(&c), c.target_point, c.owner), (at, at, Some(p)));
    // No new step: no piece.
    w.objclient.missiles.get_mut(&k).unwrap().new_step = false;
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(kids(&w, 2).len(), 1);
    // The owner gone: the maker is removed.
    w.objclient.missiles.get_mut(&k).unwrap().owner = None;
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.missiles.contains_key(&k));
}

#[test]
fn function_35_drops_a_glob_every_fourth_frame_left() {
    use d2_sim::rng::Seed;
    let mut rows = vec![ClientMissileRow::default(); 355];
    rows[353] = row(FN_QUEEN_DEATH_CENTRE);
    rows[353].range = 8;
    rows[354] = row(FN_DEFAULT_STEP);
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows, &at(100, 100, 353), true)
        .unwrap()
        .unwrap();
    w.objclient.set_c.get_mut(&k).unwrap().seed = Some((7, 666));
    let globs = |w: &ClientWorld| {
        w.objclient
            .set_c
            .iter()
            .filter(|(_, u)| u.class == 354)
            .map(|(k, _)| w.objclient.missiles[k].target_point)
            .collect::<Vec<_>>()
    };
    let left = w.objclient.missiles[&k].current;
    update(&mut w, &rows, k, true).unwrap();
    let mut s = Seed::new(7, 666);
    if left % 4 == 0 {
        let (a, b) = (s.step(), s.step());
        let t = (100 - (a % 12) as i32, 100 + (b % 13) as i32 - 6);
        assert_eq!(globs(&w), vec![t]);
    } else {
        assert!(globs(&w).is_empty());
    }
    // Over the next frames left, one glob per multiple of 4.
    let before = globs(&w).len();
    let mut due = 0;
    for _ in 0..3 {
        let Some(m) = w.objclient.missiles.get(&k) else {
            break;
        };
        due += usize::from(m.current % 4 == 0);
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(globs(&w).len(), before + due);
}
