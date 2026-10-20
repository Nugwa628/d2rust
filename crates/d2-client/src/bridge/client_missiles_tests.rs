//! The client create on synthetic rows.

use super::*;
use crate::bridge::drlg::DrlgRoomId;
use crate::bridge::world::{ActiveRoom, ClientUnit};
use crate::rules::lighting::records::LightKind;

fn rows() -> Vec<ClientMissileRow> {
    vec![
        ClientMissileRow::default(),
        ClientMissileRow {
            vel: 8,
            vel_lev: 4,
            range: 10,
            lev_range: 2,
            activate: 3,
            anim_len: 6,
            anim_speed: 16,
            light: 10,
            rgb: (255, 64, 48),
            ..ClientMissileRow::default()
        },
    ]
}

fn at(x: i32, y: i32) -> CreateRecord {
    CreateRecord {
        flags: flag::POSITION,
        class: 1,
        x,
        y,
        level: 3,
        ..CreateRecord::default()
    }
}

#[test]
fn a_created_missile_holds_its_frames_velocity_and_target() {
    let mut w = ClientWorld::default();
    // No row → none.
    let mut none = at(100, 100);
    none.class = 9;
    assert_eq!(create(&mut w, &rows(), &none, true).unwrap(), None);
    // Flag 1: start (100, 100); flag 2: target 5 sub-tiles east.
    let mut east = at(100, 100);
    east.flags |= flag::TARGET_RELATIVE;
    east.tx = 5;
    let k = create(&mut w, &rows(), &east, true).unwrap().unwrap();
    assert_eq!(k.unit_type, MISSILE);
    let u = &w.objclient.set_c[&k];
    assert_eq!((u.class, u.position), (1, Some((100, 100))));
    let m = w.objclient.missiles[&k];
    // F = Range + LevRange × level = 10 + 2 · 3; activate =
    // F − Activate.
    assert_eq!((m.total, m.current, m.activate), (16, 16, 13));
    // (Vel + VelLev · level / 8) << 8 = 9 << 8, then × 75 / 100.
    assert_eq!(m.velocity, (9 << 8) * 75 / 100);
    // Length AnimLen << 8, speed AnimSpeed << 4.
    assert_eq!((m.anim_len, m.anim_speed), (6 << 8, 16 << 4));
    assert_eq!(m.target_point, (105, 100));
    // Without flag 2 / 0x20 the target is the start: the path's point is
    // the position, so its velocity is 0.
    let k0 = create(&mut w, &rows(), &at(100, 100), true)
        .unwrap()
        .unwrap();
    let m0 = w.objclient.missiles[&k0];
    assert_eq!((m0.target_point, m0.velocity), ((100, 100), 0));
    // Flag 2: relative target; flag 0x20: absolute.
    let mut rel = at(100, 100);
    rel.flags |= flag::TARGET_RELATIVE;
    (rel.tx, rel.ty) = (3, -2);
    let k = create(&mut w, &rows(), &rel, true).unwrap().unwrap();
    assert_eq!(w.objclient.missiles[&k].target_point, (103, 98));
    // Flag 0x8000 range, 0x800 activate, 0x200 start frame.
    let mut r = at(100, 100);
    r.flags |= flag::RANGE | flag::ACTIVATE | flag::START_FRAME;
    (r.range, r.activate, r.start_frame) = (40, 5, 4);
    let k = create(&mut w, &rows(), &r, true).unwrap().unwrap();
    let m = w.objclient.missiles[&k];
    assert_eq!((m.total, m.activate, m.frame), (36, 31, 4 << 8));
}

#[test]
fn frames_from_the_distance() {
    let mut w = ClientWorld::default();
    let mut r = at(100, 100);
    r.flags |= flag::TARGET_ABSOLUTE | flag::FRAMES_FROM_DISTANCE;
    (r.tx, r.ty) = (110, 104);
    let k = create(&mut w, &rows(), &r, true).unwrap().unwrap();
    let m = w.objclient.missiles[&k];
    // D = max(10, 4) + 4 / 2 = 12; frames = (d << 16) / (v << 4).
    let v = ((9 << 8) * 75 / 100) as u32;
    let n = ((12u32 << 16) / (v << 4)) as i32;
    assert_eq!((m.total, m.current), (n, n));
}

#[test]
fn the_missile_light_follows_quality_flag_and_row() {
    let mut w = ClientWorld::default();
    let k = create(&mut w, &rows(), &at(100, 100), true)
        .unwrap()
        .unwrap();
    let (_, rec) = w.lights.iter().next().unwrap();
    // missile row: kind 1, radius `Light`, the row's
    // colour; owner the missile (set C).
    assert_eq!(
        (
            rec.owner_type,
            rec.owner_guid,
            rec.lookup_flag,
            rec.kind,
            rec.radius,
            (rec.r, rec.g, rec.b)
        ),
        (3, k.guid, true, LightKind::Plain, 80, (255, 64, 48))
    );
    // Low quality, or flag 0x4000: no light.
    let mut w = ClientWorld::default();
    create(&mut w, &rows(), &at(100, 100), false).unwrap();
    let mut r = at(100, 100);
    r.flags |= flag::NO_LIGHT;
    create(&mut w, &rows(), &r, true).unwrap();
    assert!(w.lights.is_empty());
}

#[test]
fn no_room_at_the_start_creates_nothing() {
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    w.local_player = Some(p);
    w.active_rooms = Some(vec![ActiveRoom {
        x0: 90,
        y0: 90,
        w: 20,
        h: 20,
        level: 8,
        room: DrlgRoomId(1),
    }]);
    w.room_units.place(p, Some(DrlgRoomId(1)));
    // (no owner: from the local player's room): in the room → made.
    assert!(create(&mut w, &rows(), &at(95, 95), true)
        .unwrap()
        .is_some());
    // Outside every room → none.
    assert_eq!(create(&mut w, &rows(), &at(300, 300), true).unwrap(), None);
}

fn row(f: u16) -> ClientMissileRow {
    ClientMissileRow {
        range: 4,
        anim_len: 3,
        anim_speed: 16,
        light: 5,
        rgb: (1, 2, 3),
        clt_do_func: f,
        ..ClientMissileRow::default()
    }
}

/// A world with one missile of `rows[1]` at (100, 100).
fn made(rows: &[ClientMissileRow]) -> (ClientWorld, UnitKey) {
    let mut w = ClientWorld::default();
    let mut r = at(100, 100);
    r.level = 0;
    let k = create(&mut w, rows, &r, true).unwrap().unwrap();
    (w, k)
}

#[test]
fn the_default_step_animates_counts_down_and_ends() {
    let rows = vec![ClientMissileRow::default(), row(FN_DEFAULT_STEP)];
    let (mut w, k) = made(&rows);
    // F = 4, activate = 4 (`Activate` 0): active from the start; speed
    // 16 << 4 = 256 per update, length 3 << 8, no loop: the frame holds
    // at the end.
    let frames = |w: &ClientWorld| w.objclient.missiles.get(&k).map(|m| (m.current, m.frame));
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(frames(&w), Some((3, 256)));
    update(&mut w, &rows, k, true).unwrap();
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(frames(&w), Some((1, 512)), "frame + speed ≥ length: held");
    // Frames left reach 0: end(none, 0): the missile removed, its light
    // dying.
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k) && !w.objclient.missiles.contains_key(&k));
    let (_, light) = w.lights.iter().next().unwrap();
    assert!(light.dying && light.target == 0);
}

#[test]
fn the_den_light_never_expires_and_function_11_lies_flat() {
    let rows = vec![ClientMissileRow::default(), row(FN_DEN_LIGHT)];
    let (mut w, k) = made(&rows);
    for _ in 0..600 {
        update(&mut w, &rows, k, true).unwrap();
    }
    // 23: frames left < 100 → 500 before each step.
    let m = w.objclient.missiles[&k];
    assert!(m.current >= 99 && m.current <= 500, "{}", m.current);
    assert_eq!(w.lights.len(), 1);
    let rows = vec![ClientMissileRow::default(), row(FN_FLAT_AT_END)];
    let (mut w, k) = made(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.missiles[&k].flat);
    update(&mut w, &rows, k, true).unwrap();
    // At the end (frame 512 + 256 ≥ 768): flag 0x10000, light removed,
    // no countdown: the missile stays.
    update(&mut w, &rows, k, true).unwrap();
    let m = w.objclient.missiles[&k];
    assert!(m.flat);
    assert!(w.lights.is_empty());
    let left = m.current;
    for _ in 0..10 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(w.objclient.missiles[&k].current, left);
}

#[test]
fn no_row_removes_and_init_steps_hide_until_active() {
    let rows = vec![ClientMissileRow::default(), row(FN_DEFAULT_STEP)];
    let (mut w, k) = made(&rows);
    update(&mut w, &[], k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k) && w.lights.is_empty());
    // `InitSteps` 2: not drawn until elapsed > 2.
    let mut r = row(FN_DEFAULT_STEP);
    (r.init_steps, r.range) = (2, 10);
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, k) = made(&rows);
    let hidden = |w: &ClientWorld| w.objclient.set_c[&k].flag_ex & FLAG_EX_NOT_DRAWN != 0;
    assert!(hidden(&w));
    for _ in 0..3 {
        update(&mut w, &rows, k, true).unwrap();
        assert!(hidden(&w));
    }
    update(&mut w, &rows, k, true).unwrap();
    assert!(!hidden(&w), "elapsed 3 > InitSteps 2");
}

#[test]
fn always_explode_makes_the_explosion_missile_with_the_owners_aim() {
    let mut r = row(FN_DEFAULT_STEP);
    (r.always_explode, r.explosion_missile) = (true, 2);
    let mut boom = row(FN_DEFAULT_STEP);
    boom.explosion_missile = -1;
    let rows = vec![ClientMissileRow::default(), r, boom];
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    // An owner aiming at its own cell (no target) without velocity: no
    // nudge (runs only with v ≠ 0).
    let mut rec = at(100, 100);
    rec.owner = Some(p);
    rec.flags |= flag::RANDOM_DIRECTION;
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    let dir = w.objclient.missiles[&k].direction;
    let x = end(&mut w, &rows, k, false, true)
        .unwrap()
        .expect("exploded");
    let xm = w.objclient.missiles[&x];
    assert_eq!(
        (w.objclient.set_c[&x].class, xm.owner, xm.direction),
        (2, Some(p), dir)
    );
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn the_aim_nudge_steps_off_the_owners_cell_by_its_direction() {
    let rows = rows();
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    let mut rec = at(100, 100);
    rec.owner = Some(p);
    // Without the owner's direction the nudge cannot run.
    assert!(create(&mut w, &rows, &rec, true).is_err());
    // dir64 20 → d = 2: (DX, DY) = (−2, 0).
    rec.owner_dir64 = Some(20);
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    assert_eq!(w.objclient.missiles[&k].target_point, (98, 100));
}

#[test]
fn a_moving_missile_steps_along_its_path_and_ends_after_its_frames() {
    let mut r = row(FN_DEFAULT_STEP);
    (r.vel, r.range, r.collide_type) = (16, 20, 3);
    let rows = vec![ClientMissileRow::default(), r];
    let mut w = ClientWorld::default();
    let mut rec = at(100, 100);
    rec.flags |= flag::TARGET_RELATIVE;
    rec.tx = 10;
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    let m = w.objclient.missiles[&k];
    assert_eq!(m.dir_vec.1, 0, "due east");
    assert!(m.dir_vec.0 > 0);
    let start = m.pos;
    update(&mut w, &rows, k, true).unwrap();
    let m = w.objclient.missiles[&k];
    // ((v × 0x400) >> 6) × dir >> 12 per update, v = (16 <<
    // 8) × 75 / 100.
    let v = (16 << 8) * 75 / 100;
    let step = (((v * 0x400) >> 6) * m.dir_vec.0) >> 12;
    assert_eq!((m.pos.0 - start.0) as i32, step);
    assert_eq!(m.pos.1, start.1);
    assert_eq!(
        w.objclient.set_c[&k].position,
        Some(((m.pos.0 >> 16) as u16, (m.pos.1 >> 16) as u16))
    );
    // No client DRLG: no wall; it flies its frames.
    for _ in 0..30 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert!(
        !w.objclient.set_c.contains_key(&k),
        "ended after its frames"
    );
}

#[test]
fn function_5_loops_the_fire_frames() {
    let mut r = row(FN_SUB_LOOP_FIRE);
    (r.range, r.sub_start, r.sub_stop, r.anim_len, r.anim_speed) = (10, 4, 8, 12, 0);
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, k) = made(&rows);
    let set = |w: &mut ClientWorld, frame: i32, left: i32| {
        let m = w.objclient.missiles.get_mut(&k).unwrap();
        (m.frame, m.current) = (frame, left);
    };
    let frame = |w: &ClientWorld| w.objclient.missiles[&k].frame;
    // f = A − 1 = 3 → frame := (3 + rnd(Z − A)) << 8, one draw on m's
    // seed (none without a seed: rnd reads 0).
    set(&mut w, 3 << 8, 9);
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(frame(&w), 3 << 8);
    // left = A → max(A − 3, 0) << 8.
    set(&mut w, 6 << 8, 4);
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(frame(&w), 1 << 8);
    // left < A → max(f − 2, 0) << 8.
    set(&mut w, 6 << 8, 3);
    update(&mut w, &rows, k, true).unwrap();
    assert_eq!(frame(&w), 4 << 8);
}

#[test]
fn function_43_follows_its_owner_and_ends_with_its_death() {
    let rows = vec![ClientMissileRow::default(), row(FN_FOLLOW_OWNER)];
    let mut w = ClientWorld::default();
    let mon = UnitKey::new(super::super::world::MONSTER, 4);
    let mut u = ClientUnit::new(mon);
    u.position = Some((120, 130));
    u.mode = 1;
    w.units.insert(mon, u);
    let mut rec = at(100, 100);
    rec.owner = Some(mon);
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    let left = w.objclient.missiles[&k].current;
    update(&mut w, &rows, k, true).unwrap();
    // The owner's position; no countdown.
    assert_eq!(w.objclient.set_c[&k].position, Some((120, 130)));
    assert_eq!(w.objclient.missiles[&k].current, left);
    // A dead owner (monster mode 12): end(none, 0).
    w.units.get_mut(&mon).unwrap().mode = 12;
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
    // No owner: removed.
    let (mut w, k) = made(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

#[test]
fn functions_60_and_63_turn_around_their_centre() {
    for f in [FN_ORBIT, FN_ORBIT_EVEN] {
        let rows = vec![ClientMissileRow::default(), row(f)];
        let (mut w, k) = made(&rows);
        w.objclient.missiles.get_mut(&k).unwrap().d28 = 95;
        w.objclient.missiles.get_mut(&k).unwrap().d2c = 100;
        update(&mut w, &rows, k, true).unwrap();
        // (x, y) = (100, 100), centre (95, 100): target (100 + 0, 100 −
        // 5) on elapsed 0 (60: even elapsed only).
        assert_eq!(w.objclient.missiles[&k].target_point, (100, 95), "f {f}");
        w.objclient.missiles.get_mut(&k).unwrap().target_point = (0, 0);
        update(&mut w, &rows, k, true).unwrap();
        let t = w.objclient.missiles[&k].target_point;
        if f == FN_ORBIT {
            assert_eq!(t, (100, 95));
        } else {
            assert_eq!(t, (0, 0), "60 skips odd elapsed");
        }
    }
}

#[test]
fn function_8_leaves_a_trail_on_each_new_sub_tile() {
    let mut parent = row(FN_TRAIL);
    (parent.vel, parent.range, parent.clt_sub) = (16, 12, [2, -1, -1]);
    let mut child = row(FN_DEFAULT_STEP);
    child.light = 0;
    let rows = vec![ClientMissileRow::default(), parent, child];
    let mut w = ClientWorld::default();
    let mut rec = at(100, 100);
    rec.flags |= flag::TARGET_RELATIVE;
    rec.tx = 20;
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    // Each update reads the flag its previous step left, then
    // steps; count the steps that entered a new sub-tile.
    let mut expected = 0;
    let mut new_step = false;
    for _ in 0..8 {
        let before: Vec<UnitKey> = w.objclient.set_c.keys().copied().collect();
        let at = w.objclient.missiles[&k].pos;
        update(&mut w, &rows, k, true).unwrap();
        let made: Vec<UnitKey> = w
            .objclient
            .set_c
            .keys()
            .copied()
            .filter(|c| !before.contains(c))
            .collect();
        if new_step {
            expected += 1;
            assert_eq!(made.len(), 1);
            let c = &w.objclient.set_c[&made[0]];
            assert_eq!(c.class, 2);
            assert_eq!(c.position, Some(((at.0 >> 16) as u16, (at.1 >> 16) as u16)));
        } else {
            assert!(made.is_empty());
        }
        new_step = w.objclient.missiles[&k].new_step;
    }
    assert!(expected >= 4, "{expected}");
    // S1 < 0 → removed.
    let mut r = row(FN_TRAIL);
    r.clt_sub = [-1, -1, -1];
    let rows = vec![ClientMissileRow::default(), r];
    let (mut w, k) = made(&rows);
    update(&mut w, &rows, k, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
}

/// Gives `k` the seed `{lo, 666}`.
fn seeded(w: &mut ClientWorld, k: UnitKey, lo: u32) -> Seed {
    w.objclient.set_c.get_mut(&k).unwrap().seed = Some((lo, 666));
    Seed::new(lo, 666)
}

fn children(w: &ClientWorld, class: u32) -> Vec<UnitKey> {
    w.objclient
        .set_c
        .iter()
        .filter(|(_, u)| u.class == class)
        .map(|(k, _)| *k)
        .collect()
}

#[test]
fn function_4_scatters_its_sub_missiles() {
    let mut r = row(FN_SCATTER);
    // chance 1 (rnd(1) = 0 always passes), 2 per update, spread 3, S1 2.
    (r.clt_param, r.clt_sub) = ([1, 2, 3, 0, 0], [2, -1, -1]);
    let mut child = row(FN_DEFAULT_STEP);
    child.light = 0;
    let rows = vec![ClientMissileRow::default(), r, child];
    let (mut w, k) = made(&rows);
    let mut seed = seeded(&mut w, k, 1234);
    let p1 = w.objclient.missiles[&k].owner;
    assert_eq!(p1, None);
    update(&mut w, &rows, k, true).unwrap();
    // rnd(chance 1) draws one step, then dx, dy per create.
    seed.roll(1);
    let mut want = Vec::new();
    for _ in 0..2 {
        let dx = seed.roll(6) as i32 - 3;
        let dy = seed.roll(6) as i32 - 3;
        let s = |v: i32| if v < 0 { -1 } else { 1 };
        want.push((100 + dx + s(dx) * 3, 100 + dy + s(dy) * 3));
    }
    let mut got: Vec<_> = children(&w, 2)
        .iter()
        .map(|c| w.objclient.missiles[c].target_point)
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(w.objclient.set_c[&k].seed, Some((seed.lo, seed.hi)));
}

#[test]
fn functions_25_and_49_emit_on_their_period() {
    let mut mist = row(FN_MIST);
    (mist.range, mist.clt_param, mist.clt_sub) = (10, [2, 4, 3, 0, 0], [2, -1, -1]);
    let mut child = row(FN_DEFAULT_STEP);
    child.light = 0;
    let rows = vec![ClientMissileRow::default(), mist, child];
    let (mut w, k) = made(&rows);
    let mut seed = seeded(&mut w, k, 77);
    // Elapsed 0, 3, 6: one mist each; the draws u, v, sx, sy in order.
    for _ in 0..7 {
        update(&mut w, &rows, k, true).unwrap();
    }
    assert_eq!(children(&w, 2).len(), 3);
    let u = seed.roll(5) as i32 - 2;
    let v = seed.roll(5) as i32 - 2;
    let sx = 100 + seed.roll(9) as i32 - 4;
    let sy = 100 + seed.roll(9) as i32 - 4;
    assert!(children(&w, 2).iter().any(|c| {
        let m = w.objclient.missiles[c];
        let at = w.objclient.set_c[c].position;
        at == Some((sx as u16, sy as u16)) && m.target_point == (sx + u, sy + v)
    }));
    // 49: spawn facing every max(P1, 1) elapsed, owner required.
    let mut vines = row(FN_SPAWN_FACING);
    (vines.range, vines.clt_param, vines.clt_sub) = (10, [2, 0, 0, 0, 0], [2, -1, -1]);
    let rows = vec![ClientMissileRow::default(), vines, child];
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    let mut rec = at(100, 100);
    rec.owner = Some(p);
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().direction = 17;
    for _ in 0..4 {
        update(&mut w, &rows, k, true).unwrap();
    }
    let kids = children(&w, 2);
    assert_eq!(kids.len(), 2, "elapsed 0 and 2");
    assert!(kids.iter().all(
        |c| w.objclient.missiles[c].direction == 17 && w.objclient.missiles[c].owner == Some(p)
    ));
}

#[test]
fn function_6_lays_its_wall_on_each_new_sub_tile() {
    let mut maker = row(FN_WALL_MAKER);
    (maker.vel, maker.range, maker.clt_sub, maker.clt_param) =
        (16, 12, [2, 3, -1], [2, 0, 0, 0, 0]);
    let mut fire = row(FN_DEFAULT_STEP);
    (fire.range, fire.light) = (30, 5);
    let rows = vec![ClientMissileRow::default(), maker, fire, fire];
    let mut w = ClientWorld::default();
    let mut rec = at(100, 100);
    rec.flags |= flag::TARGET_RELATIVE;
    rec.tx = 20;
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    let mut seed = seeded(&mut w, k, 4321);
    let mut want = Vec::new();
    let mut new_step = false;
    for _ in 0..6 {
        let at = w.objclient.missiles[&k].pos;
        update(&mut w, &rows, k, true).unwrap();
        if new_step {
            // rnd(2) picks S2 (3) or S1 (2); rnd(P1 = 2) ≠ 0 → no light.
            let c = if seed.roll(2) == 0 { 3 } else { 2 };
            let lit = seed.roll(2) == 0;
            want.push((c, at, lit));
        }
        new_step = w.objclient.missiles[&k].new_step;
    }
    assert!(!want.is_empty());
    let mut got: Vec<_> = w
        .objclient
        .missiles
        .iter()
        .filter(|(c, _)| **c != k)
        .map(|(c, m)| {
            let class = w.objclient.set_c[c].class;
            let lit = w
                .lights
                .iter()
                .any(|(_, l)| l.owner_guid == c.guid && l.lookup_flag);
            (class, m.pos, lit)
        })
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(
        w.objclient
            .missiles
            .values()
            .filter(|m| m.total == 30)
            .count(),
        want.len()
    );
}

#[test]
fn the_view_resolves_a_client_missile_at_its_path_position_and_direction() {
    use crate::bridge::world::view_key;
    use crate::rules::lighting::records::Owner;
    let mut r = row(FN_DEFAULT_STEP);
    (r.vel, r.range) = (16, 20);
    let rows = vec![ClientMissileRow::default(), r];
    let mut w = ClientWorld::default();
    let mut rec = at(100, 100);
    rec.flags |= flag::TARGET_RELATIVE;
    rec.tx = 10;
    let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
    update(&mut w, &rows, k, true).unwrap();
    let m = w.objclient.missiles[&k];
    let vk = view_key(k);
    let u = w.view_unit(&vk).expect("a client missile is a view unit");
    assert_eq!(u.precise, Some(m.pos));
    assert_eq!(
        u.position,
        Some(((m.pos.0 >> 16) as u16, (m.pos.1 >> 16) as u16))
    );
    assert_eq!(w.view_direction(&vk), Some(m.direction));
    assert!(w.view_units().iter().any(|u| u.key == vk));
    // The light owner lookup reads set C for a client-only missile.
    let owner = |client_only| Owner {
        unit_type: u32::from(MISSILE),
        guid: k.guid,
        client_only,
    };
    assert_eq!(w.client_missile_pos(&owner(true)), Some(m.pos));
    assert_eq!(w.client_missile_pos(&owner(false)), None);
    // The server-side key (no view bit) is not the client missile.
    assert!(w.view_unit(&k).is_none());
}

#[test]
fn relink_takes_a_roomless_or_ended_missile_out_of_the_room_lists() {
    use crate::bridge::world::view_key;
    let rows = vec![ClientMissileRow::default(), row(FN_DEFAULT_STEP)];
    let (mut w, k) = made(&rows);
    let vk = view_key(k);
    // A stale entry: the path has no room (no client DRLG) → no list.
    w.room_units.place(vk, Some(DrlgRoomId(1)));
    relink(&mut w, k);
    assert_eq!(w.room_units.room_of(vk), None);
    // Ended (out of set C) → no list either.
    w.room_units.place(vk, Some(DrlgRoomId(1)));
    w.objclient.set_c.remove(&k);
    relink(&mut w, k);
    assert_eq!(w.room_units.room_of(vk), None);
}

#[test]
fn function_8_lifts_its_child_by_a_seeded_height_when_p2_is_set() {
    for p2 in [0, 16] {
        let mut parent = row(FN_TRAIL);
        (parent.vel, parent.range, parent.clt_sub) = (16, 12, [2, -1, -1]);
        let mut child = row(FN_DEFAULT_STEP);
        child.light = 0;
        child.clt_param[1] = p2;
        let rows = vec![ClientMissileRow::default(), parent, child];
        let mut w = ClientWorld::default();
        let mut rec = at(100, 100);
        rec.flags |= flag::TARGET_RELATIVE;
        rec.tx = 20;
        let k = create(&mut w, &rows, &rec, true).unwrap().unwrap();
        let mut zs = Vec::new();
        for _ in 0..8 {
            update(&mut w, &rows, k, true).unwrap();
        }
        for (key, u) in &w.objclient.set_c {
            if u.class == 2 {
                zs.push(w.objclient.missiles[key].motion.pos[2]);
            }
        }
        assert!(!zs.is_empty());
        for z in zs {
            assert_eq!(z % 2048, 0, "{z}");
            if p2 == 0 {
                assert_eq!(z, 0);
            } else {
                assert!((-8 * 2048..8 * 2048).contains(&z), "{z}");
            }
        }
    }
}
