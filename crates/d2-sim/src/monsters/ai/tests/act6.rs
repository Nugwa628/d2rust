use super::act2::{act_row, give_skill, logged, param_of, point_mode, seeded, set_param_of, world};
use super::act3::seed_raw;
use super::npc::{seed_with, steps_since, unit_mode};
use super::*;
use crate::monsters::ai::bodies6::{pet_follow, pet_move, search_capped, wander_prime, Mover};

/// Monstats rows up to `n` (copies of row 0) so fixed classes resolve.
pub(super) fn grow(w: &mut World, n: usize) {
    let r = w.monstats[0].clone();
    w.monstats.resize(n, r);
    let m = w.modes[0];
    w.modes.resize(n, m);
}

/// The player is the unit's minion owner.
pub(super) fn own(w: &mut World) {
    let g = w.game.lists.unit(w.player).unwrap().guid;
    w.store.control_mut(w.mon).unwrap().minion_owner = Some(UnitRef {
        ty: UnitType::Player,
        guid: g,
    });
}

fn death() -> String {
    point_mode(mode::DEATH, 0, 0)
}

// ---- pet helpers -------------------------------------------------------

#[test]
fn wander_prime_draws_around_the_unit_with_a_byte() {
    // X (the player) is not read: the point is around the unit; n as a byte.
    let mut w = world(act_row(67, &[]));
    w.seed(12345);
    w.fake.pos.insert(w.player, (150, 150));
    let u = w.mon;
    let started = w.with(|g, cx| wander_prime(g, cx, u, 259));
    assert!(started);
    let mut s = Seed::init_low(12345);
    let (x, y) = wander_point(&mut s, (100, 100), 3);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, x, y)]);
}

#[test]
fn pet_move_loiter_and_make_room() {
    // k 2: roll(100) ≥ 10 → idle 15, 1 (one draw).
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |v| v[0] >= 10);
    let (u, o) = (w.mon, w.player);
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 2, false, 0, 4));
    assert!(r);
    assert_eq!((w.thinks(), steps_since(&w, lo)), (vec![15], 1));
    // k 4: no crowding pet → idle 15.
    let mut w = world(act_row(67, &[]));
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 4, false, 0, 4));
    assert!(r && w.thinks() == [15]);
    // A pet within 1: escape from O by n with delete (walk away).
    let mut w = world(act_row(67, &[]));
    let v = w.add_unit(UnitType::Monster, (101, 100));
    w.fake.y.pets = vec![w.mon, v];
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 4, false, 0, 4));
    assert!(r);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 96, 100)]);
    // Other k: 0.
    let mut w = world(act_row(67, &[]));
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 6, false, 0, 4));
    assert!(!r && w.fake.log.is_empty());
}

#[test]
fn pet_move_ahead_and_catch_up() {
    // k 0: Q := F + 8 × (dx, dy)[j], j = {4, 3, …}[e]; e 0 → j 4 → (0, +8).
    let mut w = world(act_row(67, &[]));
    let (u, o) = (w.mon, w.player);
    w.fake.y.final_point.insert(o, (130, 100));
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 0, false, 50, 0));
    assert!(r);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 130, 108)]);
    assert_eq!((w.vel_request().speed, w.vel_request().steps), (50, 40));
    // Q in O's coordinate area whose walk fails: the midpoint
    // ((U + Q) >> 1) is tried next.
    let mut w = world(act_row(67, &[]));
    w.fake.y.final_point.insert(o, (130, 100));
    w.fake.fail_points.insert((130, 108));
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 0, false, 0, 0));
    assert!(r);
    assert_eq!(
        w.fake.modes(),
        [
            point_mode(mode::WALK, 130, 108),
            point_mode(mode::WALK, 115, 104)
        ]
    );
    // Q in another coordinate area: nothing is tried for it (no
    // midpoint); e 1 → j 3 → Q = F + 8 × (−1, +1).
    let mut w = world(act_row(67, &[]));
    w.fake.y.final_point.insert(o, (130, 100));
    w.fake.y.coord.insert((130, 108), 7);
    w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 0, false, 0, 0));
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 122, 108)]);
    // k 3: free spot for class 363 near O; placed → flag 0x10000, idle 5.
    let mut w = world(act_row(67, &[]));
    grow(&mut w, 364);
    let room = w.room;
    w.fake.y.free_spot = Some((110, 110));
    w.fake.x.room_at = Some(room);
    w.fake.x.place_ok = true;
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 3, false, 0, 0));
    assert!(r);
    assert!(logged(&w, "freespot2 0 363") && logged(&w, "place 110 110"));
    assert!(logged(&w, "flag2 0x10000"));
    assert_eq!(w.thinks(), [5]);
    // No free spot → 0.
    let mut w = world(act_row(67, &[]));
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 3, false, 0, 0));
    assert!(!r && w.thinks().is_empty());
}

#[test]
fn pet_move_retraces_the_history() {
    // k 1, O not walking, speed 0: s := roll(40) + 40; the newest entry
    // farther than 5: velocity (0, s, 100), run (run ≠ 0) to it.
    let (mut w, lo) = seeded(act_row(67, &[]), 1, |_| true);
    let (u, o) = (w.mon, w.player);
    let mut h = [(0, 0); 20];
    h[2] = (120, 100);
    h[1] = (102, 100);
    w.fake.y.history = (3, h);
    let r = w.with(|g, cx| pet_move(g, cx, Mover::Pet, o, u, 1, true, 0, 0));
    assert!(r);
    let mut s = Seed::init_low(lo);
    let speed = (s.step() % 40) as i32 + 40;
    assert_eq!(w.fake.modes(), [point_mode(mode::RUN, 120, 100)]);
    assert_eq!(w.vel_request().speed, speed);
}

#[test]
fn pet_follow_rules() {
    // Quiet and k 2 (O neutral, far enough, same area) → 0.
    let mut w = world(act_row(67, &[]));
    let (u, o) = (w.mon, w.player);
    let r = w.with(|g, cx| pet_follow(g, cx, u, None, o, false, true, 6));
    assert!(!r && w.fake.log.is_empty());
    // S set, out of town, D ≤ 80 → 0.
    let mut w = world(act_row(67, &[]));
    let r = w.with(|g, cx| pet_follow(g, cx, u, Some(o), o, false, false, 6));
    assert!(!r);
    // D ≤ 1, O neutral, M = 0 → pet move k 5: velocity (7, 0, 0), wander
    // near O.
    let mut w = world(act_row(67, &[]));
    w.fake.pos.insert(o, (101, 100));
    w.seed(1);
    let r = w.with(|g, cx| pet_follow(g, cx, u, None, o, false, false, 6));
    assert!(r);
    assert_eq!(w.vel_request().method, 7);
}

#[test]
fn search_capped_drops_far_targets() {
    // An evil unit, the player in slot 0 at no-size distance 7.
    let mut w = world(act_row(67, &[]));
    w.fake.nodes = vec![vec![w.player]];
    w.fake.pos.insert(w.player, (107, 100));
    let u = w.mon;
    let (t, e, _) = w.with(|g, cx| search_capped(g, cx, u, 6));
    assert_eq!((t, e), (None, 7));
    let (t, _, _) = w.with(|g, cx| search_capped(g, cx, u, 7));
    assert_eq!(t, Some(w.player));
}

// ---- NecroPet ----------------------------------------------------------

#[test]
fn necro_pet_melee_far_and_ahead() {
    // No owner, param 2 = 0: idle 10.
    let mut w = world(act_row(67, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [10]);
    // Test vector: D = 60 → pet move k 3 (free spot probe, none here).
    let mut w = world(act_row(67, &[]));
    own(&mut w);
    w.fake.pos.insert(w.player, (161, 100));
    w.think_with(None, 0, false);
    assert!(logged(&w, "freespot2 0 -1"));
    let g = w.game.lists.unit(w.player).unwrap().guid as i32;
    assert_eq!(param_of(&w, 2), g);
    // 28 < D ≤ 50: pet move k 0 with v (`Velocity` 0 → 100).
    let mut w = world(act_row(67, &[]));
    own(&mut w);
    w.fake.pos.insert(w.player, (140, 100));
    w.fake.y.final_point.insert(w.player, (140, 100));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 140, 108)]);
    assert_eq!(w.vel_request().speed, 100);
}

#[test]
fn necro_pet_ranged_shoots_its_secondary_target() {
    // Ranged (param 0 = 1), S at E 5: draw r (no follow: S set, out of
    // town, D ≤ 80), roll(100) < 80 → `Skill1` at S.
    let (mut w, lo) = seeded(act_row(67, &[]), 2, |v| v[0] >= 15 && v[1] < 80);
    give_skill(&mut w, 1, 50, 10);
    own(&mut w);
    set_param_of(&mut w, 0, 1);
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 5));
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, s)]);
    assert_eq!(steps_since(&w, lo), 2);
}

// ---- MinionSpawner, Towner, EvilHole -------------------------------

const SPAWNER: [i16; 5] = [100, 25, 100, 20, 25];

#[test]
fn minion_spawner_vectors() {
    let setup = || {
        let mut w = world(act_row(121, &SPAWNER));
        grow(&mut w, 462);
        w.monstats[453].baseid = 453;
        give_skill(&mut w, 1, 302, 10);
        w
    };
    // n = 3, D = 10, frame ≥ f, 4 minion1 around: n := 4, f := frame + 100,
    // `Skill1` at T.
    let mut w = setup();
    for i in 0..4 {
        let v = w.add_unit(UnitType::Monster, (110 + i, 100));
        w.fake.class.insert(v, 453);
    }
    set_param_of(&mut w, 1, 3);
    w.game.frame = 40;
    w.think_with(Some(w.player), 10, false);
    assert_eq!((param_of(&w, 1), param_of(&w, 0)), (4, 140));
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    // 25 counted minions: idle aip2.
    let mut w = setup();
    for i in 0..25 {
        let v = w.add_unit(UnitType::Monster, (110 + i, 100));
        w.fake.class.insert(v, 453);
    }
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [25]);
    // n = 100: nothing scheduled.
    let mut w = setup();
    set_param_of(&mut w, 1, 100);
    w.think_with(Some(w.player), 10, false);
    assert!(w.thinks().is_empty() && w.fake.modes().is_empty());
    // D > aip4, or frame < f: idle aip2.
    let mut w = setup();
    w.think_with(Some(w.player), 21, false);
    assert_eq!(w.thinks(), [25]);
    let mut w = setup();
    set_param_of(&mut w, 0, 5);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [25]);
}

#[test]
fn towner_goes_home_first() {
    let mut w = world(act_row(41, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    assert_eq!(w.commands(), [[10, 100, 100, 0, 0]]);
    // No map AI, no command: idle 12.
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [12]);
}

#[test]
fn evil_hole_vectors() {
    let hole = [10, 50];
    // f = 0, m = 1, D = 3: f := frame + 50, n := 10, mode 10, wait 20.
    let mut w = world(act_row(76, &hole));
    w.think_with(Some(w.player), 3, false);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (50, 10));
    assert_eq!(w.fake.modes(), [point_mode(10, 0, 0)]);
    assert_eq!(w.thinks(), [20]);
    // m = 10 → mode 11; m = 11, n = 0 → mode 0.
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes().last().unwrap(), &point_mode(11, 0, 0));
    set_param_of(&mut w, 1, 0);
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes().last().unwrap(), &death());
    // The demon hole spawns megademon6 with state 184 and overlay 202.
    let mut w = world(act_row(76, &hole));
    grow(&mut w, 720);
    w.monstats[711].baseid = 321;
    let (mon, room) = (w.mon, w.room);
    w.fake.class.insert(mon, 711);
    w.fake.anim.insert(mon, 11);
    w.fake.x.room_at = Some(room);
    let c = w.add_unit(UnitType::Monster, (100, 100));
    w.fake.x.spawn = Some(c);
    set_param_of(&mut w, 0, 5);
    set_param_of(&mut w, 1, 2);
    w.game.frame = 10;
    w.think_with(Some(w.player), 3, false);
    assert!(logged(&w, "spawn 712 100 100 1 2 0x42"));
    assert!(logged(&w, "flag 0x4020000") && logged(&w, "overlay 202"));
    assert!(w.fake.states.contains(&(c, 184)));
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (60, 1));
    assert_eq!(w.thinks(), [60]);
    // Not neutral, 10 or 11: death.
    let mut w = world(act_row(76, &hole));
    w.fake.anim.insert(w.mon, mode::WALK);
    w.think_with(Some(w.player), 3, false);
    assert_eq!(w.fake.modes(), [death()]);
}

// ---- Hireable ------------------------------------------------------------

/// An act2hire (338) owned by the player, `aip1` 1 (melee).
fn hire_world() -> World {
    let mut w = world(act_row(61, &[1]));
    grow(&mut w, 339);
    let mon = w.mon;
    w.fake.class.insert(mon, 338);
    own(&mut w);
    w
}

#[test]
fn hireable_test_vector() {
    // act2hire, p = 0, level 20, S at d = 2 in melee range, draw 79 (< 98):
    // p := 0; hireling skill (no skill slot: the fallback, A1 at S).
    let mut w = hire_world();
    w.seed(seed_with(1, |v| v[0] == 79));
    w.fake.stats.insert((w.mon, 12), 20);
    let s = w.add_unit(UnitType::Monster, (102, 100));
    w.fake.secondary = Some((s, 2));
    w.fake.melee.insert(s);
    w.fake.y.hire_id = Some(1);
    w.fake.y.hire_row = Some(HireRow::default());
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
    // A miss raises p by 10 and idles 10.
    let mut w = hire_world();
    w.seed(seed_with(1, |v| v[0] >= 98));
    let s = w.add_unit(UnitType::Monster, (102, 100));
    w.fake.secondary = Some((s, 2));
    w.fake.melee.insert(s);
    w.fake.y.hire_id = Some(1);
    w.think_with(None, 0, false);
    assert_eq!((param_of(&w, 0), w.thinks()), (10, vec![10]));
    // No node in the owner's pet lists: idle 10, no draw.
    let mut w = hire_world();
    w.seed(1);
    let s = w.add_unit(UnitType::Monster, (102, 100));
    w.fake.secondary = Some((s, 2));
    w.think_with(None, 0, false);
    assert_eq!((w.thinks(), steps_since(&w, 1)), (vec![10], 0));
}

#[test]
fn hireable_without_owner_and_idle() {
    // No player owner: special state 6 (act2hire) and idle 10.
    let mut w = world(act_row(61, &[1]));
    grow(&mut w, 339);
    w.fake.class.insert(w.mon, 338);
    w.think_with(None, 0, false);
    assert_eq!(w.store.control(w.mon).unwrap().special_state, 6);
    assert_eq!(w.thinks(), [10]);
    // Walking: nothing scheduled; state 12 is cleared first.
    let mut w = hire_world();
    w.fake.states.insert((w.mon, 12));
    w.fake.anim.insert(w.mon, mode::WALK);
    w.think_with(None, 0, false);
    assert!(w.thinks().is_empty());
    assert!(!w.fake.states.contains(&(w.mon, 12)));
    // Near, no target, draw ≥ 5: idle 5.
    let mut w = hire_world();
    w.seed(seed_with(1, |v| v[0] >= 5));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [5]);
}

#[test]
fn hireling_skill_weights() {
    // Two counted slots; r := roll(acc + 1) ≥ DefaultChance picks the first
    // c_i ≥ r: an aura becomes the right skill (idle 10), any other skill
    // is used in its mode at S.
    let row = HireRow {
        level: 10,
        default_chance: 10,
        skill: [1, 2, 0, 0, 0, 0],
        chance: [20, 30, 0, 0, 0, 0],
        chance_per_lvl: [4, 0, 0, 0, 0, 0],
        mode: [9, 10, 0, 0, 0, 0],
    };
    // acc = 10 + (20 + 4 × 10 / 4) = 40 after slot 0, 70 after slot 1.
    let lo = seed_raw(2, |v| v[0] % 100 < 98 && (41..=70).contains(&(v[1] % 71)));
    let mut w = hire_world();
    w.skills = vec![Skills::decode(&vec![0u8; Skills::SIZE]); 3];
    w.fake.x.skill_level.insert(1, 5);
    w.fake.x.skill_level.insert(2, 5);
    w.fake.stats.insert((w.mon, 12), 20);
    let s = w.add_unit(UnitType::Monster, (102, 100));
    w.fake.secondary = Some((s, 2));
    w.fake.melee.insert(s);
    w.fake.y.hire_id = Some(3);
    w.fake.y.hire_row = Some(row);
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, s)]);
    assert!(logged(&w, "skill 2"));
}

// ---- QuillMother, BaalTentacle, ElementalBeast --------------------

#[test]
fn quill_mother_vectors() {
    let qb = [60, 50, 16, 15];
    // C, no AI state, draw 59 (< 60): A1 at T.
    let (mut w, _) = seeded(act_row(75, &qb), 1, |v| v[0] == 59);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // C, draw ≥ 60: idle aip3.
    let (mut w, _) = seeded(act_row(75, &qb), 1, |v| v[0] >= 60);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [16]);
    // Not C: draw ≥ 50 → idle aip4.
    let (mut w, _) = seeded(act_row(75, &qb), 1, |v| v[0] >= 50);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.thinks(), [15]);
    // AI state 3: the order to the minions, then the walk (no draw).
    let mut w = world(act_row(75, &qb));
    w.seed(1);
    w.fake.ai_state = 3;
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
}

#[test]
fn baal_tentacle_vectors() {
    let bt = [70, 24, 10];
    // No owner: killed.
    let mut w = world(act_row(139, &bt));
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, &format!("kill {} None", w.mon.0)));
    // e = 0, frame 1000, roll(10) = 3: e := 1325; not C → idle 24.
    let lo = seed_raw(1, |v| v[0] % 10 == 3);
    let mut w = world(act_row(139, &bt));
    w.fake.x.owners.insert(w.mon, w.player);
    w.seed(lo);
    w.game.frame = 1000;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(param_of(&w, 2), 1325);
    assert_eq!(w.thinks(), [1024]);
    // Expired: killed.
    w.game.frame = 1326;
    w.think_with(Some(w.player), 5, false);
    assert!(logged(&w, &format!("kill {} None", w.mon.0)));
}

#[test]
fn elemental_beast_vectors() {
    let fb = [20, 16, 20];
    // s = 0, D = 10: S1 at T, s := 1.
    let mut w = world(act_row(46, &fb));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::SKILL1, w.player)]);
    assert_eq!(param_of(&w, 0), 1);
    // s = 1, C: S1 again, a MODECHANGE event at frame + 1, s := 2.
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 2);
    let ev: Vec<_> = w
        .game
        .timers
        .unit_timers(w.mon)
        .into_iter()
        .filter_map(|t| {
            w.game
                .timers
                .event(t)
                .map(|e| (e.0, w.game.timers.expire(t)))
        })
        .collect();
    assert!(ev.contains(&(0, Some(1))));
    // s = 2: killed by T.
    w.think_with(Some(w.player), 1, true);
    assert!(logged(
        &w,
        &format!("kill {} Some({})", w.mon.0, w.player.0)
    ));
    // s = 0, far: idle aip3.
    let mut w = world(act_row(46, &fb));
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.thinks(), [20]);
}

// ---- NpcStationary, MosquitoNest, DesertTurret ------------------

#[test]
fn npc_stationary_greets() {
    // Nobody near: idle 20.
    let mut w = world(act_row(54, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    // A player within 24, g = 0: g := 60, sound 18 to P.
    let mut w = world(act_row(54, &[]));
    w.fake.nearest = Some((w.player, false));
    w.think_with(None, 0, false);
    assert!(logged(&w, "sound 18"));
    assert_eq!(param_of(&w, 1), 60);
    // Then the countdown.
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 1), 59);
    // Far (≥ 24) with g = 0: g := 59.
    let mut w = world(act_row(54, &[]));
    w.fake.nearest = Some((w.player, false));
    w.fake.pos.insert(w.player, (130, 100));
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 1), 59);
    // Tyrael (251) with the seam and an empty interaction list: leaves.
    let mut w = world(act_row(54, &[]));
    w.fake.class.insert(w.mon, 251);
    w.fake.y.hooks.insert("TyraelLeave".into());
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [death(), death()]);
    assert!(logged(&w, "hook TyraelGone") && w.thinks().is_empty());
}

#[test]
fn mosquito_nest_rules() {
    let nest = [16, 25, 200];
    let mut w = world(act_row(83, &nest));
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.thinks(), [25]);
    let mut w = world(act_row(83, &nest));
    set_param_of(&mut w, 1, 17);
    w.think_with(Some(w.player), 5, false);
    assert!(logged(&w, "flag 0x20000"));
    assert_eq!(w.fake.modes(), [death()]);
    let mut w = world(act_row(83, &nest));
    give_skill(&mut w, 1, 100, 14);
    w.fake.footprint = true;
    w.game.frame = 3;
    w.think_with(Some(w.player), 5, false);
    assert_eq!((param_of(&w, 1), param_of(&w, 0)), (1, 203));
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
}

#[test]
fn desert_turret_vectors() {
    let tur = [10, 5, 120, 30, 5];
    // f = 0, `Skill1` ≥ 0: the deploy, j := 0, f := frame.
    let mut w = world(act_row(94, &tur));
    give_skill(&mut w, 1, 300, 10);
    w.game.frame = 7;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [point_mode(10, 0, 0)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 2)), (7, 0));
    // e 0, j = J[0][0] = 0, Q := own + 5 × (1, 1): shot at Q, n := 1,
    // f := frame + aip1.
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes()[1], point_mode(10, 105, 105));
    assert!(logged(&w, "pathpoint 105 105"));
    assert_eq!((param_of(&w, 1), param_of(&w, 0)), (1, 17));
    // Out of range: n −= 1, idle 15.
    w.game.frame = 20;
    w.think_with(Some(w.player), 31, false);
    assert_eq!((param_of(&w, 1), w.thinks()), (0, vec![35]));
    // Too early: idle 10.
    let mut w = world(act_row(94, &tur));
    set_param_of(&mut w, 0, 50);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn desert_turret_second_check_is_at_the_target() {
    // Step 5 checks Skill1 at T with Q, then at T's own position.
    let mut w = world(act_row(94, &[10, 5, 120, 30, 5]));
    give_skill(&mut w, 1, 300, 10);
    w.game.frame = 7;
    set_param_of(&mut w, 0, 5);
    let pl = w.player;
    w.fake.pos.insert(pl, (130, 140));
    w.think_with(Some(pl), 10, false);
    let checks: Vec<&String> = w
        .fake
        .log
        .iter()
        .filter(|l| l.starts_with("check "))
        .collect();
    assert_eq!(checks.len(), 2, "{checks:?}");
    assert!(checks[0].starts_with(&format!("check 300 Some({pl:?}) ")));
    assert_eq!(checks[1], &format!("check 300 Some({pl:?}) 130 140"));
}

// ---- AssassinSentry, Catapult, CatapultSpotter ------------------

pub(super) fn sentry_world(ai: u16, aips: &[i16]) -> World {
    let mut w = world(act_row(ai, aips));
    give_skill(&mut w, 1, 1, 9);
    give_skill(&mut w, 2, 2, 14);
    w.skills = vec![Skills::decode(&vec![0u8; Skills::SIZE]); 3];
    w.fake.x.skill_entry.insert(1, (1, 9));
    w.fake.x.skill_level.insert(1, 5);
    own(&mut w);
    w
}

#[test]
fn assassin_sentry_vector() {
    let ls = [100, 10, 15, 25];
    // c = 1, S at E2 10, draw 40 (< 100): c := 0, shot; next think: death.
    let mut w = sentry_world(101, &ls);
    w.seed(seed_with(1, |v| v[0] == 40));
    set_param_of(&mut w, 1, 1);
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 1), 0);
    assert_eq!(w.fake.modes(), [unit_mode(9, s)]);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes().last().unwrap(), &death());
    // Uncounted charges: `calc4` of the entry's level (the fake's 3).
    let mut w = sentry_world(101, &ls);
    w.fake.y.calc = 3;
    w.think_with(None, 0, false);
    assert_eq!((param_of(&w, 1), w.thinks()), (3, vec![15]));
    // The owner gone: death.
    let mut w = sentry_world(101, &ls);
    w.store.control_mut(w.mon).unwrap().minion_owner = None;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [death()]);
}

#[test]
fn catapult_fires_without_a_target() {
    let (mut w, _) = seeded(act_row(123, &[20]), 1, |v| v[0] < 20);
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::ATTACK1, 0, 0)]);
    let (mut w, _) = seeded(act_row(123, &[20]), 1, |v| v[0] >= 20);
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn catapult_spotter_vectors() {
    let sp = [8, 25, 25, 10, 10];
    // f = 500, frame 510, T: draw 50 (> 2, no check), 510 − 500 < 25 →
    // idle 25.
    let (mut w, lo) = seeded(act_row(126, &sp), 1, |v| v[0] == 50);
    set_param_of(&mut w, 2, 500);
    w.game.frame = 510;
    w.think_with(Some(w.player), 10, false);
    assert_eq!((w.thinks(), steps_since(&w, lo)), (vec![535], 1));
    // f = 0: the check (no draw); the catapult 3 rooms back is dead →
    // mode 0 at T.
    let mut w = world(act_row(126, &sp));
    w.fake.y.room_box = Some((0, 0, 10, 10));
    w.fake.y.dead_at.insert((0, -20));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::DEATH, w.player)]);
    // A shot: v < 1 → a := roll(5), v := aip5; the point draws; the free
    // point; skill[a] in mode 4 at it; f := frame.
    let lo = seed_raw(2, |v| v[0] % 100 > 2 && v[1] % 100 < 8);
    let mut w = world(act_row(126, &sp));
    set_param_of(&mut w, 2, 1);
    w.game.frame = 100;
    w.fake.y.free_point = Some((120, 90));
    w.seed(lo);
    w.think_with(Some(w.player), 10, false);
    let mut s = Seed::init_low(lo);
    s.step();
    s.step();
    let a = (s.step() % 5) as usize;
    assert_eq!(param_of(&w, 0), a as i32);
    assert_eq!(param_of(&w, 1), 10);
    assert!(logged(
        &w,
        &format!("skill {}", [287, 288, 303, 304, 305][a])
    ));
    assert_eq!(w.fake.modes(), [point_mode(mode::ATTACK1, 120, 90)]);
    assert_eq!(param_of(&w, 2), 100);
}

// ---- Tentacle, TentacleHead, Hydra, Totem, Vendor -------

const TENT: [i16; 6] = [70, 5, 16, 12, 20, 12];

#[test]
fn tentacle_vectors() {
    let setup = || {
        let mut w = world(act_row(56, &TENT));
        give_skill(&mut w, 1, 20, 14);
        give_skill(&mut w, 2, 21, 8);
        own(&mut w);
        w
    };
    // No owner: killed.
    let mut w = world(act_row(56, &TENT));
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, &format!("kill {} None", w.mon.0)));
    // s = 0: submerge, wait 8, t := frame + 400, s := 1.
    let mut w = setup();
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    assert_eq!(
        (param_of(&w, 1), param_of(&w, 2), w.thinks()),
        (400, 1, vec![8])
    );
    // s = 1, frame > t, C: emerge at itself, t := frame + 300, s := 2.
    let mut w = setup();
    set_param_of(&mut w, 2, 1);
    w.game.frame = 10;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(8, w.mon)]);
    assert_eq!((param_of(&w, 1), param_of(&w, 2)), (310, 2));
    // s = 1, not due: wait aip5.
    let mut w = setup();
    set_param_of(&mut w, 2, 1);
    set_param_of(&mut w, 1, 50);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [20]);
}

#[test]
fn tentacle_head_rules() {
    // s = 2, frame > t, D > aip6: submerge with wait 20.
    let mut w = world(act_row(57, &TENT));
    give_skill(&mut w, 1, 20, 14);
    set_param_of(&mut w, 2, 2);
    w.game.frame = 5;
    w.think_with(Some(w.player), 13, false);
    assert_eq!((w.thinks(), param_of(&w, 2)), (vec![25], 1));
    // No skills: draw < aip1 → A1 at the secondary target.
    let (mut w, _) = seeded(act_row(57, &TENT), 1, |v| v[0] < 70);
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 9));
    w.think_with(Some(w.player), 13, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
}

#[test]
fn hydra_expires() {
    let mut w = world(act_row(86, &[]));
    w.game.frame = 5;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [death()]);
    let (mut w, _) = seeded(act_row(86, &[]), 1, |v| v[0] < 60);
    give_skill(&mut w, 1, 30, 4);
    set_param_of(&mut w, 0, 100);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(4, w.player)]);
}

#[test]
fn totem_rules() {
    let mut w = world(act_row(109, &[20, 30, 30, 20]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [10]);
    // Far (reach distance > aip3): placed at the owner → idle 25.
    let mut w = world(act_row(109, &[20, 30, 30, 20]));
    own(&mut w);
    w.seed(1);
    w.fake.pos.insert(w.player, (150, 100));
    w.fake.x.place_ok = true;
    w.think_with(None, 0, false);
    assert!(logged(&w, "place 150 100"));
    assert_eq!(w.thinks(), [25]);
}

#[test]
fn vendor_animates() {
    let (mut w, _) = seeded(act_row(42, &[]), 1, |v| v[0] < 20);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::SKILL1, 0, 0)]);
    let (mut w, _) = seeded(act_row(42, &[]), 1, |v| v[0] >= 20);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [30]);
}

// ---- Trap-Missile, TrappedSoul --------------------------------------

#[test]
fn trap_missile_vectors() {
    let trap = [25, 1, 15];
    // n = 0, toggle 0: A1, n := 1, toggle := 1.
    let mut w = world(act_row(77, &trap));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, 1));
    // n = 1 = aip2: death.
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes().last().unwrap(), &death());
    assert!(logged(&w, "flag 0x20000"));
}

#[test]
fn trapped_soul_vectors() {
    // Awake, C, frame > f, T at (xu − 1, yu + 1): A1, f := frame + 35.
    let mut w = world(act_row(99, &[]));
    set_param_of(&mut w, 0, 1);
    w.fake.pos.insert(w.player, (99, 101));
    w.game.frame = 10;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(param_of(&w, 1), 45);
    // T at (xu + 1, yu − 1): A2.
    w.fake.pos.insert(w.player, (101, 99));
    w.game.frame = 50;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes()[1], unit_mode(mode::ATTACK2, w.player));
    // Not awake, near: wakes with S2.
    let mut w = world(act_row(99, &[]));
    w.think_with(Some(w.player), 2, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::SKILL2, w.player)]);
    assert_eq!(param_of(&w, 0), 1);
    // Far and asleep: idle 15.
    let mut w = world(act_row(99, &[]));
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.thinks(), [15]);
}

// ---- DruidWolf, CycleOfLife, NpcBarb -----------------------------

#[test]
fn druid_wolf_without_owner_ports_to_the_player() {
    let mut w = world(act_row(108, &[22, 20, 14, 20, 26]));
    grow(&mut w, 421);
    w.fake.class.insert(w.mon, 420);
    give_skill(&mut w, 1, 54, 10);
    let g = w.game.lists.unit(w.player).unwrap().guid as i32;
    set_param_of(&mut w, 2, g);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(10, 105, 100)]);
    assert_eq!(w.thinks(), [8]);
    // Neither: idle 10.
    let mut w = world(act_row(108, &[]));
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [10]);
    // In town: the pet follow, else idle 33.
    let mut w = world(act_row(108, &[]));
    own(&mut w);
    let room = w.room;
    w.fake.town.insert(room);
    w.fake.pos.insert(w.player, (110, 100));
    w.fake.walk_fails = true;
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [33]);
}

#[test]
fn druid_wolf_idles_or_wanders_near_its_owner() {
    // Spirit wolf beside its owner, no target: roll(100) ≥ aip2 → idle 15.
    let (mut w, _) = seeded(act_row(108, &[22, 20, 14, 20, 26]), 1, |v| v[0] >= 20);
    grow(&mut w, 421);
    w.fake.class.insert(w.mon, 420);
    own(&mut w);
    w.fake.pos.insert(w.player, (103, 100));
    w.fake.y.final_point.insert(w.player, (103, 100));
    w.fake.y.target_point.insert(w.player, (103, 100));
    w.fake.y.last_placed = (200, 200);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
    // Fenris likewise.
    let (mut w, _) = seeded(act_row(108, &[22, 20, 25, 24, 30]), 1, |v| v[0] >= 20);
    grow(&mut w, 422);
    w.fake.class.insert(w.mon, 421);
    own(&mut w);
    w.fake.pos.insert(w.player, (103, 100));
    w.fake.y.final_point.insert(w.player, (103, 100));
    w.fake.y.target_point.insert(w.player, (103, 100));
    w.fake.y.last_placed = (200, 200);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn fenris_rage_runs_to_a_corpse() {
    // Rage draw < aip3, a corpse within r / 2 not in melee, M = 0: run to
    // it, param 0 := 1, param 1 := its GUID.
    let (mut w, _) = seeded(act_row(108, &[22, 20, 25, 24, 30]), 1, |v| v[0] < 25);
    grow(&mut w, 422);
    w.fake.class.insert(w.mon, 421);
    give_skill(&mut w, 1, 60, 10);
    own(&mut w);
    w.fake.pos.insert(w.player, (103, 100));
    w.fake.y.final_point.insert(w.player, (103, 100));
    w.fake.y.target_point.insert(w.player, (103, 100));
    w.fake.y.last_placed = (200, 200);
    let k = w.add_unit(UnitType::Monster, (105, 100));
    w.fake.y.corpse = Some(k);
    w.think_with(None, 0, false);
    assert!(logged(&w, "corpsefind 10"));
    assert_eq!(w.fake.modes(), [unit_mode(mode::RUN, k)]);
    let g = w.game.lists.unit(k).unwrap().guid as i32;
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, g));
}

#[test]
fn cycle_of_life_rules() {
    let col = [50, 20, 25, 10, 35];
    // No owner: nothing scheduled.
    let mut w = world(act_row(111, &col));
    w.think_with(None, 0, false);
    assert!(w.thinks().is_empty() && w.fake.log.is_empty());
    // The init: param 0 := 0.
    let mon = w.mon;
    set_param_of(&mut w, 0, 9);
    w.with(|g, cx| run_init(g, cx, 0x005E_C6A0, mon));
    assert_eq!(param_of(&w, 0), 0);
    // Owner beside it, no corpse, no target: the follow loiters (pet move
    // k 2: draw 51 ≥ 10 → idle 15).
    let mut w = world(act_row(111, &col));
    own(&mut w);
    w.fake.pos.insert(w.player, (103, 100));
    w.fake.y.final_point.insert(w.player, (103, 100));
    w.fake.y.target_point.insert(w.player, (103, 100));
    w.fake.y.last_placed = (200, 200);
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn npc_barb_registers_and_roams() {
    let barb = [15, 85, 15];
    // Slot 11: registered in slot 8; C → A1, wait aip1.
    let mut w = world(act_row(127, &barb));
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, "node 8"));
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(w.thinks(), [15]);
    // Registered once.
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.log.iter().filter(|l| *l == "node 8").count(), 1);
    // No target: the first roam point (x + a % 20 − 40, y + b % 20 − 10).
    let mut w = world(act_row(127, &barb));
    w.seed(1);
    w.think_with(None, 0, false);
    let mut s = Seed::init_low(1);
    let a = (s.step() % 20) as i32;
    let b = (s.step() % 20) as i32;
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 60 + a, 90 + b)]);
    // All three walks fail: idle 15 after seven steps.
    let mut w = world(act_row(127, &barb));
    w.seed(1);
    w.fake.walk_fails = true;
    w.think_with(None, 0, false);
    assert_eq!((w.thinks(), steps_since(&w, 1)), (vec![15], 7));
}
