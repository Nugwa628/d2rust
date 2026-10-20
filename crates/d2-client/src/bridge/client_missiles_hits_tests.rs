//! The client hit functions on synthetic rows, through the end,
//! with the test vectors of the body specs.

use super::*;
use crate::bridge::world::{ClientUnit, MonsterClass};
use crate::rules::unit_composite::motion;

fn row() -> ClientMissileRow {
    ClientMissileRow {
        range: 40,
        clt_do_func: FN_DEFAULT_STEP,
        explosion_missile: -1,
        ..ClientMissileRow::default()
    }
}

/// `n` rows, all [`row`]; the hit row at 1 with `h`, H and c.
fn rows(n: usize, h: i16, sub: [i16; 4], par: [i32; 3]) -> Vec<ClientMissileRow> {
    let mut v = vec![row(); n];
    (v[1].clt_hit_func, v[1].clt_hit_sub, v[1].c_hit_par) = (h, sub, par);
    v
}

/// A player owner at (100, 100) and its missile of class 1 there.
fn owned(rows: &[ClientMissileRow], level: i32) -> (ClientWorld, UnitKey) {
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((100, 100));
    w.units.insert(p, u);
    let rec = CreateRecord {
        flags: flag::POSITION,
        owner: Some(p),
        class: 1,
        x: 100,
        y: 100,
        level,
        ..CreateRecord::default()
    };
    let k = create(&mut w, rows, &rec, true).unwrap().unwrap();
    (w, k)
}

fn made(w: &ClientWorld, class: u32) -> Vec<ClientMissile> {
    w.objclient
        .set_c
        .iter()
        .filter(|(_, u)| u.class == class)
        .map(|(k, _)| w.objclient.missiles[k])
        .collect()
}

fn offsets(w: &ClientWorld, class: u32) -> Vec<(i32, i32)> {
    made(w, class)
        .iter()
        .map(|m| (m.target_point.0 - 100, m.target_point.1 - 100))
        .collect()
}

#[test]
fn hit_2_rings_of_clouds() {
    // Test vector: row 43 (c1 1, c2 2, c3 3, H1 221 with `Param1` 2,
    // `Param2` 4).
    let mut r = rows(3, 2, [2, -1, -1, -1], [1, 2, 3]);
    r[2].param = [2, 4];
    let (mut w, k) = owned(&r, 4);
    end(&mut w, &r, k, true, true).unwrap();
    let clouds = made(&w, 2);
    assert_eq!(clouds.len(), 8 + 15);
    assert_eq!(
        &offsets(&w, 2)[..8],
        &[
            (0, 2),
            (2, 2),
            (2, 0),
            (2, -2),
            (0, -2),
            (-2, -2),
            (-2, 0),
            (-2, 2)
        ]
    );
    // Velocity 256 → 192 after 75 %, then 512 → 384; loops 3: frames 40
    // (no `SubLoop`).
    assert!(clouds[..8].iter().all(|m| m.velocity == 192));
    assert!(clouds[8..].iter().all(|m| m.velocity == 384));
    // Row 47 (c1 0): ring 1 only.
    let mut r = rows(3, 2, [2, -1, -1, -1], [0, 2, 3]);
    r[2].param = [2, 4];
    let (mut w, k) = owned(&r, 4);
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(made(&w, 2).len(), 8);
}

#[test]
fn hit_1_the_fire_disc() {
    // Test vector: row 62 (c1 3, c2 1): 29 `fireexplosion2`; per point
    // rnd(1) (one draw, always 0) then rnd(`RandStart` 5).
    let mut r = rows(266, 1, [0, -1, -1, -1], [3, 1, 0]);
    r[265].rand_start = 5;
    let (mut w, k) = owned(&r, 1);
    w.objclient.set_c.get_mut(&k).unwrap().seed = Some((9, 666));
    let mut s = d2_sim::rng::Seed::new(9, 666);
    let mut frames = Vec::new();
    for _ in 0..29 {
        s.roll(1);
        frames.push((s.roll(5) as i32) << 8);
    }
    end(&mut w, &r, k, true, true).unwrap();
    let fire: Vec<_> = made(&w, 265).iter().map(|m| m.frame).collect();
    assert_eq!(fire, frames);
}

#[test]
fn hits_3_19_24_and_31() {
    // 3: spawn(H1), then a dead-flagged spawn of H2 + rnd(H3 − H2 + 1).
    let r = rows(6, 3, [2, 3, 5, -1], [0; 3]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(made(&w, 2).len(), 1);
    let picked: Vec<_> = (3..=5).flat_map(|c| made(&w, c)).collect();
    assert_eq!(picked.len(), 1);
    assert!(picked[0].flat);
    // 19: H2 ≤ H1 → H1.
    let r = rows(4, 19, [2, 1, -1, -1], [0; 3]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(made(&w, 2).len(), 1);
    // 24: on a unit, a spawn and 0: the missile stays.
    let r = rows(3, 24, [2, -1, -1, -1], [0; 3]);
    let (mut w, k) = owned(&r, 1);
    let mon = UnitKey::new(MONSTER, 7);
    let mut u = ClientUnit::new(mon);
    (u.mode, u.flag_4) = (1, true);
    w.units.insert(mon, u);
    end_with(
        &mut w,
        &Env {
            rows: &r,
            lights: true,
            skills: None,
            monsters: &[],
            model: None,
        },
        k,
        Some(mon),
        false,
    )
    .unwrap();
    assert!(w.objclient.set_c.contains_key(&k));
    assert_eq!(made(&w, 2).len(), 1);
    // 31: m of class 273 → 345, animation speed 0x80, m's facing.
    let mut r = rows(346, 0, [-1; 4], [0; 3]);
    r[273].clt_hit_func = 31;
    let mut w = ClientWorld::default();
    let rec = CreateRecord {
        flags: flag::POSITION,
        class: 273,
        x: 100,
        y: 100,
        ..CreateRecord::default()
    };
    let k = create(&mut w, &r, &rec, true).unwrap().unwrap();
    w.objclient.missiles.get_mut(&k).unwrap().direction = 9;
    end(&mut w, &r, k, true, true).unwrap();
    let ice = made(&w, 345);
    assert_eq!(
        ice.iter()
            .map(|m| (m.anim_speed, m.direction))
            .collect::<Vec<_>>(),
        vec![(0x80, 9)]
    );
}

#[test]
fn hit_14_shards_take_a_seeded_pattern() {
    let r = rows(4, 14, [2, 3, -1, -1], [0; 3]);
    let (mut w, k) = owned(&r, 1);
    w.objclient.set_c.get_mut(&k).unwrap().seed = Some((21, 666));
    let mut s = d2_sim::rng::Seed::new(21, 666);
    let a = (s.step() % 3) as usize;
    let b = (s.step() & 7) as usize;
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(made(&w, 2).len(), 1);
    let patterns: [&[usize]; 3] = [&[0, 1, 4, 5], &[0, 2, 4, 6], &[0, 3, 5]];
    // Each shard faces the sub-tile of its octant: (DX8[e], DY8[e]).
    const DX8: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
    const DY8: [i32; 8] = [1, 1, 0, -1, -1, -1, 0, 1];
    let t = d2_sim::path::tables::PathTables::spec().unwrap();
    let want: Vec<u8> = patterns[a]
        .iter()
        .map(|&e0| {
            let e = (e0 + b) & 7;
            let to = (
                ((100 + DX8[e]) as u32) << 16 | 0x8000,
                ((100 + DY8[e]) as u32) << 16 | 0x8000,
            );
            d2_sim::path::walk::geom::direction_vector(
                &t,
                (100 << 16 | 0x8000, 100 << 16 | 0x8000),
                to,
            )
            .1 & 63
        })
        .collect();
    let got: Vec<u8> = made(&w, 3).iter().map(|m| m.direction).collect();
    assert_eq!(got, want);
}

#[test]
fn hits_28_and_30_ring_of_8_and_the_orb_nova() {
    // 28, test vector: row 239 (c1 0): 8 at R8, flags 0x1F, loops level
    // − 1 (with `SubLoop` the frames show the loops).
    let mut r = rows(3, 28, [2, -1, -1, -1], [0; 3]);
    (r[2].sub_loop, r[2].sub_start, r[2].sub_stop) = (1, 1, 2);
    let (mut w, k) = owned(&r, 4);
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(
        offsets(&w, 2),
        vec![
            (0, 2),
            (2, 2),
            (2, 0),
            (2, -2),
            (0, -2),
            (-2, -2),
            (-2, 0),
            (-2, 2)
        ]
    );
    assert!(made(&w, 2).iter().all(|m| m.total == 40 + 3));
    // 30, test vector: row 260 (c1 4): 16 novas at i = 0, 4, …; each
    // child's (d28, d2C) its offset.
    let r = rows(3, 30, [2, -1, -1, -1], [4, 0, 0]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    let novas = made(&w, 2);
    assert_eq!(novas.len(), 16);
    assert_eq!(
        &offsets(&w, 2)[..5],
        &[(30, 0), (27, 11), (21, 21), (11, 27), (0, 30)]
    );
    assert!(novas
        .iter()
        .all(|m| (m.d28, m.d2c) == (m.target_point.0 - 100, m.target_point.1 - 100)));
}

#[test]
fn hit_18_the_meteor_explosion() {
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
    fn blank<T: Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }
    // Test vector: row 101 (c1 5, c2 3, c3 15), H1–H4 = 2, 3, 4, 5; skill
    // `Param3` 10, `Param4` 2, level 3 → frames 14.
    let mut r = rows(6, 18, [2, 3, 4, 5], [5, 3, 15]);
    r[3].light = 4;
    let mut sk = blank::<Skills>();
    (sk.param3, sk.param4) = (10, 2);
    let t = d2_sim::skills::SkillTables {
        skills: vec![sk],
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![blank::<Missiles>()],
        skills_code: Vec::new(),
        miss_code: Vec::new(),
        level_cap: 99,
        stat_count: 359,
    };
    let env = Env {
        rows: &r,
        lights: true,
        skills: Some(&t),
        monsters: &[],
        model: None,
    };
    let (mut w, k) = owned(&r, 3);
    end_with(&mut w, &env, k, None, true).unwrap();
    let cells = |w: &ClientWorld, c: u32| -> Vec<(i32, i32)> {
        made(w, c)
            .iter()
            .map(|m| ((m.pos.0 >> 16) as i32 - 100, (m.pos.1 >> 16) as i32 - 100))
            .collect()
    };
    assert_eq!(cells(&w, 2), vec![(0, 0), (4, 0), (0, 4), (-4, 0), (0, -4)]);
    let light = made(&w, 3);
    assert_eq!(light.iter().map(|m| m.total).collect::<Vec<_>>(), vec![14]);
    let (id, rec) = w
        .lights
        .iter()
        .find(|(_, l)| l.owner_type == 3 && !l.dying)
        .unwrap();
    assert_eq!((w.lights.radius(id), rec.radius), (Some(12), 96));
    assert_eq!(cells(&w, 4), vec![(2, -2), (-2, -2), (0, 2)]);
    let fires = cells(&w, 5);
    assert_eq!(fires.len(), 15);
    assert_eq!(fires[0], (0, 5), "F18 entry 3");
    assert!(made(&w, 5).iter().all(|m| m.total == 14));
}

#[test]
fn hits_44_54_55_56_and_52() {
    // 44 / 54 / 56: the child faces m; 44 copies d28.
    for h in [44, 54, 56] {
        let r = rows(3, h, [2, -1, -1, -1], [0; 3]);
        let (mut w, k) = owned(&r, 1);
        let m = w.objclient.missiles.get_mut(&k).unwrap();
        (m.direction, m.d28) = (13, 6);
        end(&mut w, &r, k, true, true).unwrap();
        let c = made(&w, 2);
        assert_eq!(c.len(), 1, "hit {h}");
        assert_eq!(c[0].direction, 13, "hit {h}");
        assert_eq!(c[0].d28, if h == 44 { 6 } else { 0 }, "hit {h}");
    }
    // 55: H1, H2, H3 each.
    let r = rows(5, 55, [2, 3, -1, -1], [0; 3]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(
        (made(&w, 2).len(), made(&w, 3).len(), made(&w, 4).len()),
        (1, 1, 0)
    );
    // 52 (no unit): H1 at m, rocks: n = c1 4 flying rocks within 2n, the
    // ones farther than 3 made; f = H3 at the 18 points.
    let r = rows(457, 52, [2, 0, 3, -1], [4, 9, 0]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    assert_eq!(made(&w, 2).len(), 1);
    assert_eq!(made(&w, 3).len(), 18);
    let rocks = offsets(&w, 456);
    assert!(rocks.len() <= 4);
    assert!(rocks.iter().all(|&(dx, dy)| {
        let (a, b) = (dx.abs(), dy.abs());
        a <= 8 && b <= 8 && a.max(b) + a.min(b) / 2 > 3
    }));
}

#[test]
fn hit_52_bursts_only_on_a_large_monster() {
    // Monstats2 flag 11 `large`; any other unit keeps m.
    let mut r = rows(3, 52, [2, -1, -1, -1], [0; 3]);
    // The hostility test is not this one's; a burst ends m.
    (r[1].collide_friend, r[1].collide_kill) = (true, true);
    let small = MonsterClass::default();
    let large = MonsterClass {
        large: true,
        ..MonsterClass::default()
    };
    for (class, burst) in [(0u32, false), (1, true), (2, false)] {
        let (mut w, k) = owned(&r, 1);
        let u = UnitKey::new(MONSTER, 7);
        let mut v = ClientUnit::new(u);
        (v.class, v.mode, v.flag_4) = (class, 1, true);
        v.position = Some((101, 100));
        w.units.insert(u, v);
        let monsters = [Some(small), Some(large)];
        let env = Env {
            rows: &r,
            lights: true,
            skills: None,
            monsters: &monsters,
            model: None,
        };
        end_with(&mut w, &env, k, Some(u), false).unwrap();
        assert_eq!(made(&w, 2).len(), usize::from(burst), "class {class}");
        assert_eq!(w.objclient.set_c.contains_key(&k), !burst, "class {class}");
    }
}

#[test]
fn hit_53_rolls_a_bouncing_boulder_toward_m_s_target() {
    // `moltenboulderemerge` 453: c1 2 bounces, c2 50 %, c3 10 updates.
    let r = rows(3, 53, [2, -1, -1, -1], [2, 50, 10]);
    let (mut w, k) = owned(&r, 3);
    w.objclient.missiles.get_mut(&k).unwrap().target_point = (110, 104);
    end(&mut w, &r, k, true, true).unwrap();
    let b = made(&w, 2);
    assert_eq!(b.len(), 1);
    assert_eq!((b[0].target_point, b[0].level), ((110, 104), 3));
    let mo = b[0].motion;
    // Restarted by the create (8), bounce (4); the arc's flag 2 cleared.
    assert_eq!(mo.flags, motion::RESTARTED | motion::BOUNCE);
    assert_eq!(
        (mo.bounces_left, mo.bounce_factor, mo.ticks_left),
        (2, 50, 10)
    );
    // vz := (0 − trunc(−0x1000·100 / 2)) / 10.
    assert_eq!((mo.pos[2], mo.vel[2], mo.acc[2]), (0, 20480, -0x1000));
    // No H1: nothing made.
    let r = rows(3, 53, [-1; 4], [2, 50, 10]);
    let (mut w, k) = owned(&r, 3);
    end(&mut w, &r, k, true, true).unwrap();
    assert!(made(&w, 2).is_empty());
}

#[test]
fn hit_9_explodes_on_undead_and_flies_past_the_rest() {
    // Classes 0 plain, 1 undead, 2 demon.
    let monsters = [
        Some(MonsterClass::default()),
        Some(MonsterClass {
            undead: true,
            ..MonsterClass::default()
        }),
        Some(MonsterClass {
            demon: true,
            ..MonsterClass::default()
        }),
    ];
    // Row 55 `holybolt`: H1 `teethexplode`, c1 1, c2 1; returns whether
    // m ended and the class-204 count.
    let run = |class: u32, par: [i32; 3], h1: i16| {
        let mut r = rows(205, 9, [h1, -1, -1, -1], par);
        r[1].collide_kill = true;
        let (mut w, k) = owned(&r, 1);
        let mon = UnitKey::new(MONSTER, 7);
        let mut u = ClientUnit::new(mon);
        (u.mode, u.class, u.flag_4) = (1, class, true);
        w.units.insert(mon, u);
        let env = Env {
            rows: &r,
            lights: true,
            skills: None,
            monsters: &monsters,
            model: None,
        };
        end_with(&mut w, &env, k, Some(mon), false).unwrap();
        (!w.objclient.set_c.contains_key(&k), made(&w, 204).len())
    };
    // A plain monster (no pet record: no ally): 0, m flies on.
    assert_eq!(run(0, [1, 1, 0], 204), (false, 0));
    // Undead: 1, class 204 (fixed, whatever H1 names) at m.
    assert_eq!(run(1, [1, 1, 0], 3), (true, 1));
    // H1 < 0 only gates the create.
    assert_eq!(run(1, [1, 1, 0], -1), (true, 0));
    // c2 = 2: the demon test instead.
    assert_eq!(run(1, [1, 2, 0], 204), (false, 0));
    assert_eq!(run(2, [1, 2, 0], 204), (true, 1));
    // No unit: 1.
    let r = rows(205, 9, [204, -1, -1, -1], [1, 1, 0]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    assert!(!w.objclient.set_c.contains_key(&k));
    assert_eq!(made(&w, 204).len(), 0);
    // A player U with c1 = 0 (no ally clause) and c2 ≠ 0: 0.
    let r = rows(205, 9, [204, -1, -1, -1], [0, 1, 0]);
    let (mut w, k) = owned(&r, 1);
    let p2 = UnitKey::new(PLAYER, 2);
    w.units.insert(p2, ClientUnit::new(p2));
    let env = Env {
        rows: &r,
        lights: true,
        skills: None,
        monsters: &monsters,
        model: None,
    };
    end_with(&mut w, &env, k, Some(p2), false).unwrap();
    assert!(w.objclient.set_c.contains_key(&k));
}

#[test]
fn hit_12_lays_a_fire_patch() {
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
    fn blank<T: Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }
    // `SHitCalc1` at offset 0: the constant 100 (0x07 n, 0x00 ends).
    let t = d2_sim::skills::SkillTables {
        skills: vec![blank::<Skills>()],
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![blank::<Missiles>(); 5],
        skills_code: Vec::new(),
        miss_code: vec![0x07, 100, 0x00],
        level_cap: 99,
        stat_count: 359,
    };
    // Row 85 shape: list (2, 3, 4), k = c1 1, chance c2 100.
    let r = rows(5, 12, [2, 3, 4, -1], [1, 100, 0]);
    let (mut w, k) = owned(&r, 1);
    let env = Env {
        rows: &r,
        lights: true,
        skills: Some(&t),
        monsters: &[],
        model: None,
    };
    end_with(&mut w, &env, k, None, true).unwrap();
    let made: Vec<ClientMissile> = (2..=4).flat_map(|c| made(&w, c)).collect();
    // k = 1: the five points of the disc, every one made at chance 100.
    let mut at: Vec<(i32, i32)> = made
        .iter()
        .map(|m| {
            (
                ((m.pos.0 >> 16) as i32) - 100,
                ((m.pos.1 >> 16) as i32) - 100,
            )
        })
        .collect();
    at.sort();
    assert_eq!(at, vec![(-1, 0), (0, -1), (0, 0), (0, 1), (1, 0)]);
    // Frames v + (lo' mod 25) − 12 = 88 … 112.
    assert!(
        made.iter().all(|m| (88..=112).contains(&m.total)),
        "{made:?}"
    );
    // An empty list (H1 < 0) makes nothing.
    let r = rows(5, 12, [-1, 3, 4, -1], [1, 100, 0]);
    let (mut w, k) = owned(&r, 1);
    let env = Env {
        rows: &r,
        lights: true,
        skills: Some(&t),
        monsters: &[],
        model: None,
    };
    end_with(&mut w, &env, k, None, true).unwrap();
    assert_eq!(w.objclient.set_c.len(), 0);
}

#[test]
fn hit_53_throws_a_bouncing_boulder() {
    use crate::rules::unit_composite::motion::{BOUNCE, TIMED};
    // Row 453 shape: H1 2, c1 bounces 3, c2 factor 50, c3 ticks 20.
    let r = rows(3, 53, [2, -1, -1, -1], [3, 50, 20]);
    let (mut w, k) = owned(&r, 1);
    w.objclient.missiles.get_mut(&k).unwrap().target_point = (110, 104);
    end(&mut w, &r, k, true, true).unwrap();
    let b = made(&w, 2);
    assert_eq!(b.len(), 1);
    let m = &b[0].motion;
    assert_eq!((m.bounces_left, m.bounce_factor, m.ticks_left), (3, 50, 20));
    assert_ne!(m.flags & BOUNCE, 0);
    assert_eq!(m.flags & TIMED, 0);
    assert_eq!(b[0].target_point, (110, 104));
    // H1 < 0: nothing.
    let r = rows(3, 53, [-1, -1, -1, -1], [3, 50, 20]);
    let (mut w, k) = owned(&r, 1);
    end(&mut w, &r, k, true, true).unwrap();
    assert!(made(&w, 2).is_empty());
}

#[test]
fn hit_10_puts_prog_overlay_on_the_hit_unit() {
    use crate::bridge::hit_overlays::HitOverlay;
    use crate::bridge::overlay_lights::OverlayLightRow;
    let mut inputs = crate::bridge::world::ModelInputs {
        expansion_installed: true,
        ..Default::default()
    };
    inputs.tables.overlays = vec![OverlayLightRow::default(); 3];
    let calls = |prog: i16, on_unit: bool| -> Vec<HitOverlay> {
        let mut r = rows(3, 10, [-1; 4], [0; 3]);
        r[1].prog_overlay = prog;
        let (mut w, k) = owned(&r, 1);
        let mon = UnitKey::new(MONSTER, 7);
        let mut u = ClientUnit::new(mon);
        (u.mode, u.flag_4) = (1, true);
        u.position = Some((101, 100));
        w.units.insert(mon, u);
        let env = Env {
            rows: &r,
            lights: true,
            skills: None,
            monsters: &[],
            model: Some(&inputs),
        };
        end_with(&mut w, &env, k, on_unit.then_some(mon), false).unwrap();
        w.hit_overlays
    };
    // Test vector: ProgOverlay 2 on U → one kind-2 create of id 2, no
    // `1ofN` roll, A = B = C = 0.
    let got = calls(2, true);
    assert_eq!(
        got.iter()
            .map(|c| (c.unit, c.id, c.kind, c.args, c.remove))
            .collect::<Vec<_>>(),
        vec![(UnitKey::new(MONSTER, 7), 2, 2, [0; 3], false)]
    );
    // No unit, or ProgOverlay ≤ 0 → nothing.
    assert!(calls(2, false).is_empty());
    assert!(calls(0, true).is_empty());
    assert!(calls(-1, true).is_empty());
}

#[test]
fn the_unmodelled_hits_that_return_0_on_a_unit_let_the_missile_fly_on() {
    let u = UnitKey::new(MONSTER, 7);
    for (h, on_unit) in [
        (36, false),
        (37, false),
        (38, false),
        (40, false),
        (41, false),
        (57, false),
        (61, false),
        (4, true),
        (53, true),
    ] {
        let r = rows(3, h, [-1; 4], [0; 3]);
        let (mut w, k) = owned(&r, 1);
        let env = Env {
            rows: &r,
            lights: false,
            skills: None,
            monsters: &[],
            model: None,
        };
        // 53 reads as non-zero.
        assert_eq!(
            hits::call(&mut w, &env, k, Some(u), h).unwrap(),
            on_unit,
            "hit {h} on a unit"
        );
        assert!(
            hits::call(&mut w, &env, k, None, h).unwrap(),
            "hit {h} with no unit"
        );
    }
}

#[test]
fn a_monster_has_flags_4_and_8_only_when_attackable_and_alive() {
    let mut w = ClientWorld::default();
    let (a, b) = (UnitKey::new(MONSTER, 1), UnitKey::new(MONSTER, 2));
    for (k, att) in [(a, true), (b, false)] {
        let mut u = ClientUnit::new(k);
        (u.mode, u.flag_4) = (1, att);
        w.units.insert(k, u);
    }
    assert!(flags_4_8(&w, a));
    assert!(!flags_4_8(&w, b), "monstats2 isAtt clear");
    w.units.get_mut(&a).unwrap().mode = 0;
    assert!(!flags_4_8(&w, a), "dead");
}

#[test]
fn hit_39_splats_on_an_even_seed_step() {
    use d2_sim::rng::Seed;
    let r = rows(4, 39, [-1; 4], [0; 3]);
    let (mut odd, mut even) = (0, 0);
    for lo in 1..40u32 {
        let (mut w, k) = owned(&r, 1);
        w.objclient.set_c.get_mut(&k).unwrap().seed = Some((lo, 666));
        w.objclient.missiles.get_mut(&k).unwrap().direction = 13;
        let mut s = Seed::new(lo, 666);
        let a = s.step();
        let b = s.step();
        end(&mut w, &r, k, true, true).unwrap();
        let (two, three) = (made(&w, 2), made(&w, 3));
        if a & 1 != 0 {
            odd += 1;
            assert!(two.is_empty() && three.is_empty(), "lo {lo}");
            continue;
        }
        even += 1;
        let c = if b & 1 == 0 { two } else { three };
        assert_eq!(c.len(), 1, "lo {lo}");
        assert_eq!(c[0].direction, 13);
        assert!(c[0].keep_facing);
    }
    assert!(odd > 0 && even > 0);
}
