use super::npc::{seed_with, steps_since, unit_mode};
use super::*;

/// A monstats row with AI `ai`, Normal aip1..aipN and no skills.
pub(super) fn act_row(ai: u16, aips: &[i16]) -> Monstats {
    let mut r = monstats(ai, [0; 5], 15);
    let mut v = [0i16; 8];
    v[..aips.len()].copy_from_slice(aips);
    (r.aip1, r.aip2, r.aip3, r.aip4) = (v[0] as u16, v[1] as u16, v[2] as u16, v[3] as u16);
    (r.aip5, r.aip6, r.aip7, r.aip8) = (v[4] as u16, v[5] as u16, v[6] as u16, v[7] as u16);
    r.skill4 = 0xFFFF;
    r.skill5 = 0xFFFF;
    r.skill6 = 0xFFFF;
    r.skill7 = 0xFFFF;
    r.skill8 = 0xFFFF;
    r
}

/// Skill `k` (1…8) := `id` with mode `m`.
pub(super) fn give_skill(w: &mut World, k: usize, id: u16, m: u8) {
    let r = &mut w.monstats[0];
    match k {
        1 => r.skill1 = id,
        2 => r.skill2 = id,
        3 => r.skill3 = id,
        4 => r.skill4 = id,
        5 => r.skill5 = id,
        6 => r.skill6 = id,
        7 => r.skill7 = id,
        _ => r.skill8 = id,
    }
    w.modes[0][k - 1] = m;
}

/// A world on a row whose room is its own adjacent room (the scans see
/// its units).
pub(super) fn world(row: Monstats) -> World {
    let mut w = World::new(row);
    let room = w.room;
    w.game.lists.room_mut(room).unwrap().adjacent = vec![room];
    w
}

/// A world on a row, seeded with the first seed whose draws satisfy
/// `pred` (`k` draws).
pub(super) fn seeded(row: Monstats, k: usize, pred: impl Fn(&[u32]) -> bool) -> (World, u32) {
    let mut w = world(row);
    let lo = seed_with(k, pred);
    w.seed(lo);
    (w, lo)
}

pub(super) fn point_mode(m: u8, x: i32, y: i32) -> String {
    format!("mode {m} Point({x}, {y})")
}

pub(super) fn logged(w: &World, s: &str) -> bool {
    w.fake.log.iter().any(|l| l == s)
}

pub(super) fn param_of(w: &World, n: usize) -> i32 {
    w.store.control(w.mon).unwrap().params[n]
}

pub(super) fn set_param_of(w: &mut World, n: usize, v: i32) {
    w.store.control_mut(w.mon).unwrap().params[n] = v;
}

// ---- Mummy (test vectors) ---------------------------------------------

const MUMMY1: [i16; 5] = [5, 60, 100, 65, 10];

#[test]
fn mummy_vectors() {
    // D = 4, C = 1: 40 (aip3 100 passes), 70 (≥ 65) → A2 at T.
    let (mut w, lo) = seeded(act_row(21, &MUMMY1), 2, |v| v[0] < 100 && v[1] >= 65);
    w.think_with(Some(w.player), 4, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    assert_eq!(steps_since(&w, lo), 2);
    // aip3 = 0: one draw, idle 10, then the walk (both requests).
    let mut row = act_row(21, &MUMMY1);
    row.aip3 = 0;
    let (mut w, lo) = seeded(row, 1, |_| true);
    w.fake.anim.insert(w.mon, mode::WALK);
    w.think_with(Some(w.player), 4, true);
    assert_eq!(
        w.fake.modes(),
        [
            unit_mode(mode::NEUTRAL, w.mon),
            unit_mode(mode::WALK, w.player)
        ]
    );
    assert_eq!(w.thinks(), [10]);
    assert_eq!(steps_since(&w, lo), 1);
    // D = 9, C = 0: 75 (≥ 60) → idle 10.
    let (mut w, _) = seeded(act_row(21, &MUMMY1), 1, |v| v[0] >= 60);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.thinks(), [10]);
    assert!(w.fake.modes().is_empty());
    // AI state 3, not C: walk, no draw.
    let mut w = world(act_row(21, &MUMMY1));
    w.seed(1);
    w.fake.ai_state = 3;
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
    // D ≤ aip1, not C: walk, no draw.
    let mut w = world(act_row(21, &MUMMY1));
    w.seed(1);
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
}

// ---- MaggotLarva, PinHead ----------------------------------------

#[test]
fn maggot_larva_never_bites_twice() {
    let larva = [75, 20, 60, 15];
    // C, param 0 = 1: no draw, param 0 := 0, idle 20.
    let mut w = world(act_row(38, &larva));
    w.seed(1);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!((param_of(&w, 0), w.thinks()), (0, vec![20]));
    assert_eq!(steps_since(&w, 1), 0);
    // C, param 0 = 0, draw < 75: param 0 := 1, A1.
    let (mut w, _) = seeded(act_row(38, &larva), 1, |v| v[0] < 75);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // Not C: param 0 := 0; draw < 60 → walk flags 1 (a failed walk sets
    // control flag 0x40).
    let (mut w, _) = seeded(act_row(38, &larva), 1, |v| v[0] < 60);
    set_param_of(&mut w, 0, 1);
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_ne!(w.store.control(w.mon).unwrap().flags & flag::FORCE_LOS, 0);
}

#[test]
fn pin_head_vectors() {
    let blunderbore1 = [75, 15, 60, 12, 40, 0];
    // C, param 0 = 0, draw 39 (< 40) → param 0 := 1, Smite at T.
    let (mut w, lo) = seeded(act_row(39, &blunderbore1), 1, |v| v[0] < 40);
    give_skill(&mut w, 1, 97, 9);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 1);
    assert!(logged(&w, "skill 97"));
    assert_eq!(w.fake.modes(), [unit_mode(9, w.player)]);
    assert_eq!(steps_since(&w, lo), 1);
    // Param 0 ≠ 0 and P(aip1) fails → idle aip2.
    let (mut w, _) = seeded(act_row(39, &blunderbore1), 1, |v| v[0] >= 75);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [15]);
    // No skills: A1 after the param.
    let (mut w, _) = seeded(act_row(39, &blunderbore1), 1, |v| v[0] < 75);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // Not C: param 0 := 0, draw ≥ 60 → idle 12.
    let (mut w, _) = seeded(act_row(39, &blunderbore1), 1, |v| v[0] >= 60);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!((param_of(&w, 0), w.thinks()), (0, vec![12]));
}

// ---- PantherWoman, PantherJavelin ---------------------------------

#[test]
fn panther_woman_vectors() {
    let pw = [70, 70, 8, 6, 0];
    // Not C, pack mate at squared distance 64: 80 (≥ 70), circle draw 30
    // (≥ 25) → idle 6 (64 is not > 64).
    let (mut w, lo) = seeded(act_row(18, &pw), 2, |v| v[0] >= 70 && v[1] >= 25);
    let mate = w.add_unit(UnitType::Monster, (108, 100));
    let _ = mate;
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [6]);
    assert_eq!(steps_since(&w, lo), 2);
    // At 65: velocity 75, walk to the mate with flags 7, one draw.
    let (mut w, lo) = seeded(act_row(18, &pw), 1, |v| v[0] >= 70);
    let mate = w.add_unit(UnitType::Monster, (108, 101));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, mate)]);
    assert_eq!(w.vel_request().speed, 75);
    assert_eq!(steps_since(&w, lo), 1);
    // A dead mate is no candidate.
    let (mut w, _) = seeded(act_row(18, &pw), 2, |v| v[0] >= 70 && v[1] >= 25);
    let mate = w.add_unit(UnitType::Monster, (120, 100));
    w.fake.anim.insert(mate, mode::DEAD);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [6]);
    // P(aip1) → velocity 75, lunge (method 13).
    let (mut w, _) = seeded(act_row(18, &pw), 1, |v| v[0] < 70);
    w.think_with(Some(w.player), 10, false);
    assert_eq!((w.vel_request().method, w.vel_request().speed), (13, 75));
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
}

#[test]
fn panther_javelin_vectors() {
    let slinger1 = [70, 70, 12, 10, 15, 20];
    // S at E = 25, no pack mate: 50 (< 70, E > 14) → wander near T 4.
    let (mut w, lo) = seeded(act_row(95, &slinger1), 1, |v| v[0] < 70);
    let s = w.add_unit(UnitType::Player, (125, 100));
    w.fake.secondary = Some((s, 25));
    w.think_with(Some(w.player), 25, false);
    let mut seed = Seed::init_low(lo);
    seed.step();
    let (x, y) = tactics::wander_point(&mut seed, (105, 100), 4);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, x, y)]);
    // E ≥ aip6, a pack mate at squared distance 13 > 12 (unsquared aip3):
    // walk to it with flags 7.
    let (mut w, _) = seeded(act_row(95, &slinger1), 1, |v| v[0] >= 70);
    let s = w.add_unit(UnitType::Player, (125, 100));
    w.fake.secondary = Some((s, 25));
    let mate = w.add_unit(UnitType::Monster, (102, 103));
    w.think_with(Some(w.player), 25, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, mate)]);
    // E < 8 and P(aip4): escape by 16 with delete; ends either way.
    let (mut w, _) = seeded(act_row(95, &slinger1), 1, |v| v[0] < 10);
    let s = w.add_unit(UnitType::Player, (103, 100));
    w.fake.secondary = Some((s, 3));
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 84, 100)]);
    // S at E < aip6, P(aip2) → A1 at S.
    let (mut w, _) = seeded(act_row(95, &slinger1), 1, |v| v[0] < 70);
    let s = w.add_unit(UnitType::Player, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
}

// ---- GreaterMummy ---------------------------------------------------

#[test]
fn greater_mummy_scan_heals_and_raises() {
    let unraveler1 = [70, 30, 40, 60, 24];
    let setup = |lo: u32| {
        let mut w = world(act_row(22, &unraveler1));
        w.monstats[0].lundead = true;
        give_skill(&mut w, 1, 220, 14);
        give_skill(&mut w, 2, 221, 14);
        w.seed(lo);
        w
    };
    // A hurt undead mate: P(aip3) → path target, Bestow at it.
    let lo = seed_with(1, |v| v[0] < 40);
    let mut w = setup(lo);
    let mate = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.x.flags.insert(mate, 0x2);
    w.fake.stats.insert((mate, 6), 50);
    w.fake.x.max_life.insert(mate, 100);
    w.think_with(Some(w.player), 10, false);
    assert!(logged(&w, &format!("pathtarget {}", mate.0)));
    assert_eq!(w.fake.modes(), [unit_mode(14, mate)]);
    // A corpse: P(aip2), then the skill check, then the sequence skill.
    let lo = seed_with(2, |v| v[0] < 30);
    let mut w = setup(lo);
    let corpse = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.x.flags.insert(corpse, 0x2);
    w.fake.anim.insert(corpse, mode::DEAD);
    w.think_with(Some(w.player), 10, false);
    assert!(logged(&w, &format!("check 220 Some({corpse:?}) 0 0")));
    assert_eq!(w.fake.modes(), [unit_mode(mode::SEQUENCE, corpse)]);
    // A hidden corpse, an undead state or a missing flag 0x2: not seen;
    // R.seen = 0 → velocity 50, walk with 3 steps.
    for case in 0..4 {
        let mut w = setup(1);
        let v = w.add_unit(UnitType::Monster, (110, 100));
        w.fake.x.flags.insert(v, if case == 2 { 0 } else { 0x2 });
        w.fake.anim.insert(v, mode::DEAD);
        match case {
            0 => {
                w.fake.x.groups.insert((v, 33));
            }
            1 => {
                w.fake.align = 2;
            }
            3 => {
                w.fake.pos.insert(v, (125, 100)); // 625 > 576
            }
            _ => {}
        }
        w.think_with(Some(w.player), 10, false);
        if case == 1 {
            // A good scanner (pairing good/good) but U good: not counted.
            assert!(logged(&w, "steps 3"), "case {case}");
            continue;
        }
        assert!(logged(&w, "steps 3"), "case {case}");
        assert_eq!(w.vel_request().speed, 50);
    }
    // A seen mate that is neither hurt nor a corpse: the circle / idle draw.
    let lo = seed_with(1, |v| v[0] >= 50);
    let mut w = setup(lo);
    let v = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.x.flags.insert(v, 0x2);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [6]);
    // Radament: the quest seam, radius aip5 + 10 (a unit at 30 is seen).
    let mut w = setup(1);
    w.monstats.resize(230, act_row(22, &unraveler1));
    w.fake.class.insert(w.mon, 229);
    w.monstats[229].lundead = true;
    let v = w.add_unit(UnitType::Monster, (130, 100));
    w.fake.class.insert(v, 229);
    w.fake.x.flags.insert(v, 0x2);
    w.think_with(Some(w.player), 10, false);
    assert!(logged(&w, "quest RadamentActivated"));
    assert!(!logged(&w, "steps 3"));
}

// ---- SandLeaper, MaggotEgg ----------------------------------------

#[test]
fn sand_leaper_rules() {
    let leaper = [20, 50, 30, 50];
    // D < 5, P(aip1), the check → Leap at T.
    let (mut w, _) = seeded(act_row(17, &leaper), 1, |v| v[0] < 20);
    give_skill(&mut w, 1, 95, 14);
    w.think_with(Some(w.player), 3, false);
    assert!(logged(&w, &format!("check 95 Some({:?}) 0 0", w.player)));
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    // The check fails: C, draw ≥ 50 → idle 10.
    let (mut w, _) = seeded(act_row(17, &leaper), 2, |v| v[0] < 20 && v[1] >= 50);
    give_skill(&mut w, 1, 95, 14);
    w.fake.x.check_fails = true;
    w.think_with(Some(w.player), 3, true);
    assert_eq!(w.thinks(), [10]);
    // D > 10: velocity 75, wander near T 5, no P draw.
    let (mut w, lo) = seeded(act_row(17, &leaper), 0, |_| true);
    w.think_with(Some(w.player), 11, false);
    assert_eq!(w.vel_request().speed, 75);
    let mut seed = Seed::init_low(lo);
    let (x, y) = tactics::wander_point(&mut seed, (105, 100), 5);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, x, y)]);
    // 30, 50 fail → idle 10.
    let (mut w, _) = seeded(act_row(17, &leaper), 2, |v| v[0] >= 30 && v[1] >= 50);
    w.think_with(Some(w.player), 8, false);
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn maggot_egg_hatches_then_dies() {
    let egg = [250, 18];
    let (mut w, _) = seeded(act_row(40, &egg), 1, |v| v[0] < 18);
    give_skill(&mut w, 1, 108, 14);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    assert_eq!((param_of(&w, 0), w.thinks()), (1, vec![250]));
    // Hatched: killed by the path target (0 when it is itself), no draw.
    w.seed(1);
    w.fake.path_target = Some(w.mon);
    w.think_with(Some(w.player), 10, false);
    assert!(logged(&w, &format!("kill {} None", w.mon.0)));
    assert_eq!(steps_since(&w, 1), 0);
    // P fails: idle aip1.
    let (mut w, _) = seeded(act_row(40, &egg), 1, |v| v[0] >= 18);
    give_skill(&mut w, 1, 108, 14);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [250]);
}

// ---- ClawViper -----------------------------------------------------

#[test]
fn claw_viper_glow_and_charge() {
    let viper = [40, 8, 75, 50, 15, 1];
    // Not C, D < 8: P(aip1) and the check at T's position → charge, state
    // 90 on, param 0 := 1.
    let (mut w, _) = seeded(act_row(16, &viper), 1, |v| v[0] < 40);
    give_skill(&mut w, 1, 107, 14);
    w.think_with(Some(w.player), 5, false);
    assert!(logged(
        &w,
        &format!("check 107 Some({:?}) 105 100", w.player)
    ));
    assert!(w.fake.states.contains(&(w.mon, 90)));
    assert_eq!(param_of(&w, 0), 1);
    // Every later think clears it first (param 0 never cleared).
    let (mut w2, _) = seeded(act_row(16, &viper), 1, |v| v[0] < 50);
    set_param_of(&mut w2, 0, 1);
    w2.fake.states.insert((w2.mon, 90));
    w2.think_with(Some(w2.player), 20, false);
    assert!(!w2.fake.states.contains(&(w2.mon, 90)));
    assert_eq!(w2.fake.modes(), [unit_mode(mode::WALK, w2.player)]);
    // aip6 = 2 → state 91.
    assert_eq!(bodies2::glow(2), Some(91));
    assert_eq!(bodies2::glow(0), None);
    // C: P(aip3) fails → idle aip5; else P(aip4) → A1 / A2.
    let (mut w, _) = seeded(act_row(16, &viper), 1, |v| v[0] >= 75);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [15]);
    let (mut w, _) = seeded(act_row(16, &viper), 2, |v| v[0] < 75 && v[1] >= 50);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
}

// ---- Vulture -------------------------------------------------------

#[test]
fn vulture_takes_off_and_lands() {
    let vulture1 = [70, 8, 75, 30, 40];
    // p = 0, no minion owner, far (squared 225 > 144), roll < 60: take off,
    // p := mask(8) + 24, mode 8 toward T, wait 12.
    let (mut w, lo) = seeded(act_row(23, &vulture1), 1, |v| v[0] < 60);
    w.fake.pos.insert(w.player, (115, 100));
    w.think_with(Some(w.player), 15, false);
    let mut seed = Seed::init_low(lo);
    seed.step();
    let p = (seed.step() & 7) as i32 + 24;
    assert_eq!(param_of(&w, 0), p);
    assert!(logged(&w, "unflag 0xe"));
    assert!(logged(&w, "pattern 5"));
    assert!(logged(&w, &format!("mode-radius 8 {} 8 8", w.player.0)));
    assert_eq!(w.thinks(), [12]);
    // p = 1: land; success → touch down (mode 9 at itself), p := −1.
    let mut w = world(act_row(23, &vulture1));
    w.seed(1);
    set_param_of(&mut w, 0, 1);
    w.fake.x.place_ok = true;
    w.think_with(Some(w.player), 5, false);
    assert!(logged(&w, "flag 0xe"));
    assert!(logged(&w, "stamp 100 100 1 0x100"));
    assert!(logged(&w, "movemask 0x3c01"));
    assert!(logged(&w, &format!("mode-radius 9 {} 2 3", w.mon.0)));
    assert_eq!(param_of(&w, 0), -1);
    // A failed landing still sets the flags; p := 8 and the think goes on
    // with the old p (C: P(aip1) → A1).
    let (mut w, _) = seeded(act_row(23, &vulture1), 1, |v| v[0] < 70);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, "flag 0xe"));
    assert_eq!(param_of(&w, 0), 8);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // p = −1, not C: no aip5 draw; circle or walk in radius; p := 0.
    let (mut w, lo) = seeded(act_row(23, &vulture1), 1, |v| v[0] >= 30);
    set_param_of(&mut w, 0, -1);
    w.think_with(Some(w.player), 5, false);
    assert!(logged(&w, "radius 9 0"));
    assert_eq!((param_of(&w, 0), w.thinks()), (0, vec![12]));
    assert_eq!(steps_since(&w, lo), 1);
}

#[test]
fn vulture_flight_point_keeps_its_distance() {
    // p ≥ 2, no carrion, D ≥ 6, no destination: x, y around T in ±(p + 8),
    // pushed away from O until the path distance ≥ clamp(2r, 12, 36).
    let vulture1 = [70, 8, 75, 30, 40];
    for lo in [1u32, 7, 99, 12345] {
        let mut w = world(act_row(23, &vulture1));
        w.seed(lo);
        set_param_of(&mut w, 0, 2);
        w.fake.pos.insert(w.player, (102, 100));
        // A healthy player is no carrion (life ≤ max / 100 × aip3 is).
        w.fake.stats.insert((w.player, 6), 100);
        w.fake.x.max_life.insert(w.player, 100);
        w.think_with(Some(w.player), 6, false);
        let (x, y) = (param_of(&w, 1), param_of(&w, 2));
        assert!(
            tactics::distance_no_size((100, 100), (x, y)) >= 20,
            "lo {lo}"
        );
        assert_eq!(param_of(&w, 0), 1);
        assert_eq!(w.fake.modes(), [point_mode(8, x, y)]);
        assert_eq!(w.vel_request().method, 7);
    }
}

// ---- BatDemon ------------------------------------------------------

#[test]
fn bat_demon_flight_cycle() {
    let bat = [33, 20, 60, 50, 8];
    // s = 0: mode 10, wait 8, s := 1, h := aip5 × R / 8 on stat 74.
    let mut w = world(act_row(29, &bat));
    w.seed(1);
    let mon = w.mon;
    w.fake.stats.insert((mon, 74), 20);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), ["mode 10 Point(0, 0)"]);
    assert_eq!(w.thinks(), [8]);
    assert_eq!((param_of(&w, 0), param_of(&w, 2)), (1, 20));
    assert_eq!(w.fake.stats[&(mon, 74)], 40);
    // s = 1, c = 0: mode 11, wait 10, c := 1; again: wait 15.
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [10]);
    w.think_with(Some(w.player), 10, false);
    assert_eq!((w.thinks(), param_of(&w, 1)), (vec![15], 2));
    // c > 1 and D < 7: stat 74 −= h, mode 9, wait 6, s := 3.
    w.think_with(Some(w.player), 6, false);
    assert_eq!(w.fake.stats[&(mon, 74)], 20);
    assert_eq!(w.thinks(), [6]);
    assert_eq!(param_of(&w, 0), 3);
    // Any other s, D ≥ 15: s := 4, idle 15. No draws.
    let mut w = world(act_row(29, &bat));
    w.seed(1);
    set_param_of(&mut w, 0, 9);
    w.think_with(Some(w.player), 15, false);
    assert_eq!((param_of(&w, 0), w.thinks()), (4, vec![15]));
    assert_eq!(steps_since(&w, 1), 0);
}

#[test]
fn bat_demon_on_the_ground() {
    let bat = [33, 20, 60, 50, 8];
    // s = 3, C, c > 0 → A2, c := 0.
    let mut w = world(act_row(29, &bat));
    set_param_of(&mut w, 0, 3);
    set_param_of(&mut w, 1, 2);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    assert_eq!(param_of(&w, 1), 0);
    // L < aip1 and the escape starts → s := 4.
    let mut w = world(act_row(29, &bat));
    set_param_of(&mut w, 0, 3);
    w.fake.life = 20;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 4);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 85, 100)]);
}

#[test]
fn bat_demon_alternate() {
    let bat = [33, 20, 60, 50, 8];
    // Anim mode 11 → s := 1, wait 1.
    let mut w = world(act_row(29, &bat));
    let mon = w.mon;
    w.fake.anim.insert(mon, 11);
    w.with(|g, cx| bodies2::bat_demon_alt(g, cx, mon));
    assert_eq!((param_of(&w, 0), w.thinks()), (1, vec![1]));
    // Otherwise a re-install: params cleared, the think installed, idle 1.
    let mut w = world(act_row(29, &bat));
    let mon = w.mon;
    set_param_of(&mut w, 1, 5);
    w.store.control_mut(mon).unwrap().function = 0x005F_4FD0;
    w.with(|g, cx| bodies2::bat_demon_alt(g, cx, mon));
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.function, c.params), (0x005F_5040, [0; 3]));
    assert_eq!(w.thinks(), [1]);
}

// ---- SandMaggotQueen -----------------------------------------------

#[test]
fn sand_maggot_queen_lays_and_rests() {
    let queen = [7, 12];
    let mut w = world(act_row(66, &queen));
    let room = w.room;
    w.seed(1);
    // Laying: mode 8 at its own position, wait 12, param 1 := 1.
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [point_mode(8, 100, 100)]);
    assert_eq!((w.thinks(), param_of(&w, 1)), (vec![12], 1));
    // Spawn at (x + 8, y), mode 8, spread 2, flags 0x42; NOXP flag.
    w.fake.x.room_at = Some(room);
    // The spawn info keys on `BaseId` 284 (maggotqueen1): chain(68
    // sandmaggot1), 0 steps for chain position 0.
    let r = w.monstats[0].clone();
    w.monstats.resize(300, r);
    w.monstats[0].baseid = 284;
    w.monstats[68].baseid = 68;
    let child = w.add_unit(UnitType::Monster, (108, 100));
    w.fake.x.spawn = Some(child);
    w.think_with(None, 0, false);
    assert!(logged(&w, "spawn 68 108 100 8 2 0x42"));
    assert!(logged(&w, "flag 0x4000000"));
    assert_eq!(w.store.control(w.mon).unwrap().params, [1, 0, 1]);
    // Resting: idle aip2 × 25.
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [300]);
    assert_eq!(param_of(&w, 2), 0);
    // At the cap with param 1 = 0: nothing scheduled.
    let mut w = world(act_row(66, &queen));
    w.seed(1);
    set_param_of(&mut w, 0, 7);
    w.think_with(None, 0, false);
    assert!(w.thinks().is_empty() && w.fake.modes().is_empty());
    assert_eq!(steps_since(&w, 1), 0);
}

// ---- Duriel, Summoner -----------------------------------------

#[test]
fn duriel_vectors() {
    let duriel = [5, 33, 50, 0, 0];
    // C = 0, Skill1 ≥ 0: draw (not < 0) → velocity 13, walk flags 7.
    let mut w = world(act_row(44, &duriel));
    w.seed(1);
    give_skill(&mut w, 1, 107, 14);
    give_skill(&mut w, 4, 114, 0);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(steps_since(&w, 1), 1);
    assert_eq!(w.vel_request().method, 13);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // The aura added once: Skill4 at level aip1.
    assert!(logged(&w, "aura 114 5"));
    let n = w.fake.log.iter().filter(|l| l.starts_with("aura")).count();
    w.think_with(Some(w.player), 10, false);
    assert_eq!(
        w.fake.log.iter().filter(|l| l.starts_with("aura")).count(),
        n
    );
    // C: Skill3 by aip2, then Skill2 by aip3, then A1 (aip4 = 0).
    let (mut w, _) = seeded(act_row(44, &duriel), 2, |v| v[0] >= 33 && v[1] < 50);
    give_skill(&mut w, 2, 112, 4);
    give_skill(&mut w, 3, 113, 5);
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, "skill 112"));
    let (mut w, _) = seeded(act_row(44, &duriel), 3, |v| v[0] >= 33 && v[1] >= 50);
    give_skill(&mut w, 2, 112, 4);
    give_skill(&mut w, 3, 113, 5);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
}

#[test]
fn summoner_vector() {
    // D = 3, frame 100, params 1, 2 = 0, K = 1: 50 (≥ 33: no escape), 10
    // (< 85), 7 (≥ 5), S search, 20 (≤ 63) → param 1 := 140, Frost Nova.
    let summoner = [85, 5, 63, 40, 120, 33, 5, 40];
    let (mut w, lo) = seeded(act_row(53, &summoner), 4, |v| {
        v[0] >= 33 && v[1] < 85 && v[2] >= 5 && v[3] <= 63
    });
    for (k, id) in [(1, 64), (2, 44), (3, 47), (4, 51), (5, 72)] {
        give_skill(&mut w, k, id, 10);
    }
    w.game.frame = 100;
    w.think_with(Some(w.player), 3, false);
    assert!(logged(&w, "quest SummonerActivated"));
    assert_eq!(param_of(&w, 0), 1);
    assert_eq!(param_of(&w, 1), 140);
    assert!(logged(&w, "skill 44"));
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(steps_since(&w, lo), 4);
    // P(aip1) fails → wander 4.
    let (mut w, _) = seeded(act_row(53, &summoner), 1, |v| v[0] >= 85);
    w.think_with(Some(w.player), 30, false);
    assert_eq!(w.fake.modes().len(), 1);
    assert!(w.fake.modes()[0].starts_with("mode 2 Point"));
}

// ---- special-state thinks ------------------------------------------

#[test]
fn dim_vision_strikes_or_wanders() {
    // C, no `interact`, mode A1 → A1 at T, no draw.
    let mut w = world(act_row(3, &[]));
    w.seed(1);
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().function = 0x005E_8020;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
    // `interact` class: the draw; ≥ 20 → idle 10.
    let (mut w, _) = seeded(act_row(3, &[]), 1, |v| v[0] >= 20);
    w.monstats[0].interact = true;
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().function = 0x005E_8020;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn terror_flees_then_leaves() {
    let mut w = world(act_row(3, &[]));
    let mon = w.mon;
    w.monstats[0].velocity = 4;
    w.monstats[0].run = 6;
    w.store.control_mut(mon).unwrap().function = 0x005E_8140;
    w.fake.states.insert((mon, 56));
    // First think: param 2 := 1, reset, velocity (2, 50, 0), run away by 30.
    w.think_with(Some(w.player), 5, false);
    assert_eq!(param_of(&w, 2), 1);
    assert!(logged(&w, &format!("reset {}", mon.0)));
    let v = w.vel_request();
    assert_eq!((v.method, v.speed), (2, 50));
    assert_eq!(w.fake.modes(), [point_mode(mode::RUN, 70, 100)]);
    // Beyond the default range 30: idle 10.
    w.think_with(Some(w.player), 31, false);
    assert_eq!(w.thinks(), [10]);
    // State 56 gone: special state 0, think at frame + 1.
    w.fake.states.remove(&(mon, 56));
    w.store.control_mut(mon).unwrap().special_state = 11;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.store.control(mon).unwrap().special_state, 0);
    assert_eq!(w.thinks(), [1]);
    // The init: the path target's type picks the override kind.
    let mut w = world(act_row(3, &[]));
    let mon = w.mon;
    w.fake.path_target = Some(w.player);
    w.with(|g, cx| run_init(g, cx, 0x005E_80E0, mon));
    let g = w.game.lists.unit(w.player).unwrap().guid;
    assert!(logged(&w, &format!("override 1 {g}")));
}

#[test]
fn taunted_charges_the_taunter() {
    let mut w = world(act_row(3, &[]));
    let mon = w.mon;
    let p = w.player;
    w.store.control_mut(mon).unwrap().function = 0x005E_8340;
    w.fake.path_target = Some(p);
    // Param 0 = 0: lunge to P.
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 0), 1);
    assert_eq!(w.vel_request().method, 13);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, p)]);
    // In melee range: A1 at P.
    w.fake.melee.insert(p);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes().last().unwrap(), &unit_mode(mode::ATTACK1, p));
    // P in town: back to state 0.
    w.fake.town.insert(w.room);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [1]);
}

#[test]
fn special_state_steps() {
    // 10 / 17 step 1 and step 2 (draw < 20 → wander 3).
    let (mut w, lo) = seeded(act_row(3, &[]), 1, |v| v[0] < 20);
    w.store.control_mut(w.mon).unwrap().function = 0x005E_8020;
    w.think_with(Some(w.player), 9, false);
    let mut s = Seed::init_low(lo);
    s.step();
    let (x, y) = tactics::wander_point(&mut s, (100, 100), 3);
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, x, y)]);
    // 11 step 4: C, no `interact`, mode A1 → A1; step 5: escape with
    // delete; not started → wander 6.
    let mut w = world(act_row(3, &[]));
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().function = 0x005E_8140;
    w.fake.states.insert((mon, 56));
    set_param_of(&mut w, 2, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.vel_request().method, 2);
    assert_eq!(w.fake.modes().len(), 3);
    // 12 step 1.3: the finder finds a target in combat → param 0 := 0, A1
    // at it; step 2 without a path target: back to state 0.
    let mut w = world(act_row(3, &[]));
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().function = 0x005E_8340;
    let p = w.add_unit(UnitType::Player, (130, 100));
    w.fake.path_target = Some(p);
    set_param_of(&mut w, 0, 1);
    w.fake.nodes = vec![vec![w.player]];
    w.fake.melee.insert(w.player);
    w.think_with(None, 0, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    let mut w = world(act_row(3, &[]));
    w.store.control_mut(w.mon).unwrap().function = 0x005E_8340;
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [1]);
}

#[test]
fn greater_mummy_melee_and_bolt() {
    let unraveler1 = [70, 30, 40, 60, 24];
    // C, P(aip1) → A1; D < 5, P(aip1) → A2.
    let (mut w, _) = seeded(act_row(22, &unraveler1), 1, |v| v[0] < 70);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    let (mut w, _) = seeded(act_row(22, &unraveler1), 1, |v| v[0] < 70);
    w.think_with(Some(w.player), 4, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    // No scan match, `Skill3`, P(aip4), a secondary target → bolt at S.
    let (mut w, _) = seeded(act_row(22, &unraveler1), 1, |v| v[0] < 60);
    give_skill(&mut w, 3, 222, 14);
    let s = w.add_unit(UnitType::Player, (115, 100));
    w.fake.secondary = Some((s, 15));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [unit_mode(14, s)]);
}

#[test]
fn vulture_carrion_and_rooms() {
    let vulture1 = [70, 8, 75, 30, 40];
    // Flying (p ≥ 2), a hurt monster within 11: p := 1, then the landing.
    let mut w = world(act_row(23, &vulture1));
    set_param_of(&mut w, 0, 5);
    let c = w.add_unit(UnitType::Monster, (105, 100));
    w.fake.stats.insert((c, 6), 70);
    w.fake.x.max_life.insert(c, 100);
    w.fake.stats.insert((w.player, 6), 100);
    w.fake.x.max_life.insert(w.player, 100);
    w.fake.x.place_ok = true;
    w.think_with(Some(w.player), 9, false);
    assert!(logged(&w, &format!("mode-radius 9 {} 2 3", w.mon.0)));
    assert_eq!(param_of(&w, 0), -1);
    // T in another room and far, p < 1: walk in radius of T (9, 0).
    let mut w = world(act_row(23, &vulture1));
    let other = w.game.lists.create_room(0).unwrap();
    let t = w
        .game
        .spawn_unit(UnitType::Player, Some(other), true)
        .unwrap();
    w.fake.pos.insert(t, (120, 100));
    w.think_with(Some(t), 20, false);
    assert!(logged(&w, "radius 9 0"));
}

// T = 0 is unreachable (target mode 1) and asserted (open question 6).
#[test]
#[should_panic(expected = "Vulture think without a target")]
fn vulture_asserts_a_target() {
    let mut w = world(act_row(23, &[70, 8, 75, 30, 40]));
    w.think_with(None, 0, false);
}

#[test]
fn bat_demon_hovering() {
    let bat = [33, 20, 60, 50, 8];
    // s = 2: draw < 33 → walk to T, s := 3.
    let (mut w, _) = seeded(act_row(29, &bat), 1, |v| v[0] < 33);
    set_param_of(&mut w, 0, 2);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(param_of(&w, 0), 3);
    // Both draws fail → idle 10.
    let (mut w, _) = seeded(act_row(29, &bat), 2, |v| v[0] >= 33 && v[1] >= 15);
    set_param_of(&mut w, 0, 2);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn summoner_later_steps() {
    let summoner = [85, 5, 63, 40, 120, 33, 5, 40];
    let setup = |w: &mut World| {
        for (k, id) in [(1, 64), (2, 44), (3, 47), (4, 51)] {
            give_skill(w, k, id, 10);
        }
        w.game.frame = 100;
        set_param_of(w, 0, 1);
    };
    // K = 1 (fire ≥ cold), D ≥ aip7, no S: step 8, the firewall at T,
    // param 2 := frame + aip5.
    let (mut w, _) = seeded(act_row(53, &summoner), 3, |v| v[0] < 85 && v[1] <= 63);
    setup(&mut w);
    w.think_with(Some(w.player), 20, false);
    assert!(logged(&w, "skill 51"));
    assert_eq!(param_of(&w, 2), 220);
    // Firewall cooling, S near: step 9, the fire ball at S.
    let (mut w, _) = seeded(act_row(53, &summoner), 3, |v| v[0] < 85 && v[1] > 63);
    setup(&mut w);
    set_param_of(&mut w, 2, 500);
    let s = w.add_unit(UnitType::Player, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.think_with(Some(w.player), 20, false);
    assert!(logged(&w, "skill 47"));
    assert_eq!(w.fake.modes(), [unit_mode(10, s)]);
    // K turned off, no S, nova ready (D < aip7): step 10, the nova (D < 5
    // adds the escape draw first).
    let (mut w, _) = seeded(act_row(53, &summoner), 3, |v| {
        v[0] >= 33 && v[1] < 85 && v[2] > 63
    });
    setup(&mut w);
    set_param_of(&mut w, 2, 500);
    w.think_with(Some(w.player), 3, false);
    assert!(logged(&w, "skill 44"));
    // Nothing else: `Skill5` (step 11) or wander 4 (step 12).
    let (mut w, _) = seeded(act_row(53, &summoner), 3, |v| v[0] < 85 && v[1] > 63);
    setup(&mut w);
    set_param_of(&mut w, 2, 500);
    w.think_with(Some(w.player), 20, false);
    assert!(w.fake.modes()[0].starts_with("mode 2 Point"));
}
