//! Coverage additions for the section texts and edge cases of the
//! missile bodies specs (Live notes, seed chaining, row arithmetic).

use super::ext::{others, set_data, set_frames, skill, sub, tpoint, world};
use super::*;

use super::ext::{srv_do, srv_hit};

#[test]
fn cairn_stones_sparks_window_and_missing_record() {
    // Row cairnstones (288): Param1 40, Param2 2, Param3 17, Param4 38,
    // Param5 100, SubMissile1 cairnstonessky, Range 300.
    let mut r = row();
    (r.param1, r.param2, r.param3, r.param4, r.param5) = (40, 2, 17, 38, 100);
    r.range = 300;
    r.submissile1 = 1;
    let (mut w, m) = world(r, &[sub()]);
    // Live: sparks between frames left 260 and 40, every 2 elapsed
    // frames within range 17. Frames left f = 300 - elapsed.
    let mut made = Vec::new();
    for f in 1..=300i16 {
        set_frames(&mut w, m, 300, f);
        let before = others(&w, m).len();
        // Keep the portal out of the way (Param4 38 opens at f <= 140).
        w.store.get_mut(m).unwrap().target.0 = 1;
        srv_do(&mut w, 17, m);
        if others(&w, m).len() > before {
            made.push(f);
        }
    }
    assert!(made.iter().all(|&f| f < 260 && f > 40));
    assert!(made.iter().all(|&f| (300 - f) % 2 == 0));
    assert_eq!(made.first(), Some(&42));
    assert_eq!(made.last(), Some(&258));
    assert_eq!(made.len(), 109);
    // Server-do 17 with no record: removed.
    w.store.get_mut(m).unwrap().class = 9;
    assert_eq!(srv_do(&mut w, 17, m), 2);
}

#[test]
fn baal_taunt_control_picks_the_same_slot_every_frame() {
    let mut r = row();
    (r.param1, r.param2, r.param3, r.param4) = (25, 3, 45, 0);
    r.submissile1 = 1;
    r.submissile2 = 2;
    let mut s2 = sub();
    s2.psrvdofunc = 0;
    let (mut w, m) = world(r, &[sub(), s2]);
    // Elapsed 90 is a multiple of both intervals (3 and 45): the slot
    // is whatever roll(2) gives, reset from the same x every run.
    let mut classes = Vec::new();
    for _ in 0..4 {
        set_frames(&mut w, m, 200, 110);
        let before = w.store.missiles().count();
        srv_do(&mut w, 34, m);
        assert_eq!(w.store.missiles().count(), before + 1);
        let last = w.store.missiles().last().unwrap();
        classes.push(w.store.get(last).unwrap().class);
    }
    assert!(classes.iter().all(|&c| c == classes[0]), "{classes:?}");
    // The same slot as a fresh seed from x gives.
    let mut s = Seed::init_low(100);
    let j = s.roll(2) as i32;
    assert_eq!(i32::from(classes[0]), j + 1);
}

#[test]
fn baal_taunt_lightning_ignores_the_unit_and_reseeds_from_x() {
    let mut r = row();
    r.shitpar1 = 10;
    r.hitsubmissile1 = 1;
    let (mut w, m) = world(r, &[sub()]);
    let mon = w.monster;
    assert_eq!(srv_hit(&mut w, 58, m, Some(mon)), 1);
    assert_eq!(srv_hit(&mut w, 58, m, None), 1);
    let n = others(&w, m);
    assert_eq!(n.len(), 2);
    // The unit argument is not read; equal positions give equal choices.
    assert_eq!(tpoint(&w, n[0]), tpoint(&w, n[1]));
}

#[test]
fn plague_javelin_live_rows() {
    // plaguejavelin: sHitPar1 1, 2, 3; clouds (Param1 2, Param2 4).
    let mut r = row();
    (r.shitpar1, r.shitpar2, r.shitpar3) = (1, 2, 3);
    r.hitsubmissile1 = 1;
    let mut cloud = sub();
    (cloud.param1, cloud.param2) = (2, 4);
    cloud.range = 10;
    cloud.subloop = 1;
    cloud.substart = 0;
    cloud.substop = 5;
    let (mut w, m) = world(r.clone(), &[cloud.clone()]);
    let mon = w.monster;
    srv_hit(&mut w, 2, m, Some(mon));
    let n = others(&w, m);
    // 8 clouds (even directions) and 15 more, all with 3 loops.
    assert_eq!(n.len(), 8 + 15);
    for &u in &n {
        assert_eq!(w.store.get(u).unwrap().total, 10 + 3 * 5);
    }
    // Param1 2 -> 256, Param2 4 -> 512; step 7 takes 75 %: 192 / 384.
    let vels: Vec<_> = w
        .fake
        .calls
        .iter()
        .filter(|c| c.starts_with("vel ") && *c != "vel 0")
        .cloned()
        .collect();
    assert_eq!(vels.len(), 23);
    assert!(vels[..8].iter().all(|v| v == "vel 192"));
    assert!(vels[8..].iter().all(|v| v == "vel 384"));
    // rancidgasepotion (a = 0): only the 8.
    r.shitpar1 = 0;
    let (mut w, m) = world(r.clone(), &[cloud.clone()]);
    srv_hit(&mut w, 2, m, None);
    assert_eq!(others(&w, m).len(), 8);
    // plaguejavlinexplode: no Param1 / Param2: the pieces do not move.
    cloud.param1 = 0;
    cloud.param2 = 0;
    let (mut w, m) = world(r, &[cloud]);
    srv_hit(&mut w, 2, m, None);
    assert!(w
        .fake
        .calls
        .iter()
        .all(|c| !c.starts_with("vel ") || c == "vel 0"));
    assert_eq!(others(&w, m).len(), 8);
}

#[test]
fn volcano_without_an_owner_still_flies() {
    let mut r = row();
    (r.param1, r.param2, r.param3, r.param4, r.param5) = (0, 0, 2, 128, 30);
    r.submissile1 = 1;
    let (mut w, m) = world(r, &[sub()]);
    skill(&mut w, [0, 0, 4, 0, 3]);
    set_data(&mut w, m, 77, 0);
    set_frames(&mut w, m, 50, 47);
    let o = w.owner;
    w.game.remove_unit(o).unwrap();
    let before = w.store.get(m).unwrap().current;
    let res = srv_do(&mut w, 28, m);
    assert_ne!(res, 2);
    assert_eq!(others(&w, m).len(), 0, "the throw is skipped");
    assert_eq!(w.store.get(m).unwrap().target.0, 77, "no draw");
    assert_eq!(w.store.get(m).unwrap().current, before - 1, "it flew");
}

#[test]
fn tiger_fury_never_positions_its_trail() {
    let mut r = row();
    r.submissile1 = 1;
    let (mut w, m) = world(r, &[sub()]);
    w.fake.mb.new_step = true;
    srv_do(&mut w, 32, m);
    // The sub-missile's start is (0, 0), not the missile's (100, 100):
    // nothing is created at the missile.
    assert!(others(&w, m).iter().all(|&u| w.fake.pos[&u] != (100, 100)));
}

#[test]
fn spawn_for_level_dies_on_empty_or_unspawnable_lists() {
    let (mut w, m) = world(row(), &[]);
    w.fake.mb.room_seed = Some(Seed::init_low(5));
    // No `mon` entries.
    w.fake.mb.mon_list = vec![];
    srv_hit(&mut w, 39, m, None);
    assert_eq!(w.store.unhandled.len(), 1);
    // An invalid class in the list (not in the spawn table).
    let (mut w, m) = world(row(), &[]);
    w.fake.mb.room_seed = Some(Seed::init_low(5));
    w.fake.mb.mon_list = vec![10];
    w.fake.mb.spawnable.clear();
    srv_hit(&mut w, 39, m, None);
    assert_eq!(
        w.store.unhandled,
        [Unhandled::Fatal {
            addr: 0x005B3570,
            missile: m
        }]
    );
}

#[test]
fn server_do_8_10_17_25_all_run_the_seeded_helper() {
    // Each body reads the level and skill of the missile data and its
    // frames left, then asks the helper for a sub-missile: the helper
    // re-seeds {x + elapsed, 666} and draws dx, dy of roll(2(r - 1)).
    // (range, interval) per body: 8 from Param1..3 and the level (q =
    // 10 / 4 = 2: range 3 + 2, interval max(4 - 2, 3)); 10 and 25 from
    // the skill's calc1 / calc2; 17 from Param3 / Param2.
    for (body, range, interval) in [(8i16, 5i32, 3i32), (10, 5, 3), (25, 5, 3), (17, 5, 3)] {
        let mut r = row();
        (r.param1, r.param2, r.param3) = (3, 4, 4);
        r.submissile1 = 1;
        if body == 17 {
            (r.param1, r.param2, r.param3) = (0, interval as _, range as _);
            r.range = 300;
        }
        let (mut w, m) = world(r, &[sub()]);
        skill(&mut w, [range, interval, 0, 0, 0]);
        let total = if body == 17 { 300 } else { 50 };
        // Elapsed 6: a multiple of 3 (frames left 44 / 294: inside the
        // window of body 17).
        set_frames(&mut w, m, total, total - 6);
        let x = w.fake.pos[&m].0;
        srv_do(&mut w, body, m);
        assert_eq!(others(&w, m).len(), 1, "body {body}");
        let mut s = Seed::init_low((x + 6) as u32);
        s.roll(2 * (range - 1));
        s.roll(2 * (range - 1));
        assert_eq!(*w.fake.seed(m), s, "body {body}");
    }
}

#[test]
fn only_create_missile_allocates_missiles() {
    // Missile creation is the only creator: outside the seam trait, the real
    // implementation and the test fake, `alloc_missile` is called from
    // `create.rs` alone.
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut callers = Vec::new();
    for f in files {
        if f.ends_with("cov_text.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") || t.starts_with("fn ") || t.starts_with("pub fn ") {
                continue;
            }
            if t.contains("alloc_missile(") {
                callers.push(format!(
                    "{}:{}",
                    // `/` on every OS (Windows paths use `\`)
                    f.strip_prefix(&root)
                        .unwrap()
                        .display()
                        .to_string()
                        .replace('\\', "/"),
                    i + 1
                ));
            }
        }
    }
    assert_eq!(callers.len(), 1, "{callers:?}");
    assert!(callers[0].starts_with("missiles/create.rs"), "{callers:?}");
}

#[test]
fn fire_head_heal_roll_and_damage_stage_roll_are_different() {
    let rows = || {
        let mut r = row();
        r.etype = 1;
        r.collidekill = 0;
        r.psrvhitfunc = 31;
        r.collidetype = 3;
        r
    };
    let setup = |w: &mut World, m: UnitId| {
        let o = w.owner;
        w.fake.stats.insert((o, 6), 100);
        w.fake.mb.max_life.insert(o, 5000);
        w.fake.stats.insert((m, 48), 40);
        w.fake.stats.insert((m, 49), 50);
        w.fake.stats.insert((m, 21), 256);
        w.fake.stats.insert((m, 22), 512);
    };
    // The body alone: one roll of roll(50 - 40) on the missile seed.
    let (mut w, m) = world(rows(), &[]);
    setup(&mut w, m);
    let mon = w.monster;
    let before = *w.fake.seed(m);
    assert_eq!(srv_hit(&mut w, 31, m, Some(mon)), 2);
    let mut one = before;
    let v = 40 + one.roll(10) as i32;
    assert_eq!(*w.fake.seed(m), one);
    assert_eq!(w.fake.stats[&(w.owner, 6)], 100 + v);
    // Through the hit handler the damage stage (result 2) then rolls the
    // hit's damage again on the same seed: more draws than the heal's.
    let (mut w, m) = world(rows(), &[]);
    setup(&mut w, m);
    let mon = w.monster;
    w.hit(m, Some(mon), false);
    assert_eq!(w.fake.stats[&(w.owner, 6)], 100 + v);
    assert_ne!(*w.fake.seed(m), one, "the damage stage drew again");
}
