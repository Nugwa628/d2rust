use super::act2::{act_row, give_skill, logged, param_of, point_mode, seeded, set_param_of, world};
use super::npc::{steps_since, unit_mode};
use super::*;

/// The first seed whose raw steps satisfy `pred`.
pub(super) fn seed_raw(k: usize, pred: impl Fn(&[u32]) -> bool) -> u32 {
    (1..1_000_000u32)
        .find(|&lo| {
            let mut s = Seed::init_low(lo);
            let v: Vec<u32> = (0..k).map(|_| s.step()).collect();
            pred(&v)
        })
        .expect("a seed")
}

// ---- Mosquito -------------------------------------------------------

#[test]
fn mosquito_vectors() {
    let m1 = [0, 0, 40, 40, 5];
    // C, n = 0: 90 (≥ 40 but n = 0), 10 (< 40) → n := 1, Skill1 at T.
    let (mut w, lo) = seeded(act_row(24, &m1), 2, |v| v[0] >= 40 && v[1] < 40);
    give_skill(&mut w, 1, 109, 14);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 1), 1);
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    assert_eq!(steps_since(&w, lo), 2);
    // C, n = 2: 50 (≥ 40), 15 (≤ 20) → s := 2, n := 0; velocity (2, 100,
    // 0), escape by 10 with delete, s := 1.
    let (mut w, _) = seeded(act_row(24, &m1), 2, |v| v[0] >= 40 && v[1] <= 20);
    set_param_of(&mut w, 1, 2);
    w.think_with(Some(w.player), 1, true);
    let v = w.vel_request();
    assert_eq!((v.method, v.speed), (2, 100));
    assert_eq!(w.fake.modes(), [point_mode(mode::WALK, 90, 100)]);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, 0));
    // C, draw > 20: idle 15.
    let (mut w, _) = seeded(act_row(24, &m1), 2, |v| v[0] >= 40 && v[1] > 20);
    set_param_of(&mut w, 1, 2);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [15]);
    // s = 0, not C: velocity (13, 100, 0), walk to T; no draw.
    let mut w = world(act_row(24, &m1));
    w.seed(1);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.vel_request().method, 13);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
    // s = 1: wander 4, n += 1; n > aip5 → s, n := 0.
    let mut w = world(act_row(24, &m1));
    set_param_of(&mut w, 0, 1);
    set_param_of(&mut w, 1, 5);
    w.think_with(Some(w.player), 9, false);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (0, 0));
    // Any other s: idle 10.
    let mut w = world(act_row(24, &m1));
    set_param_of(&mut w, 0, 7);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.thinks(), [10]);
}

// ---- ThornHulk ------------------------------------------------------

#[test]
fn thorn_hulk_vectors() {
    let hulk = [80, 15, 10, 30, 5, 3];
    // C, f = 0, c = 0: 50 (< 80), 20 (< 30) → frenzy swing (mode 5), f := 3.
    let (mut w, _) = seeded(act_row(27, &hulk), 2, |v| v[0] < 80 && v[1] < 30);
    give_skill(&mut w, 1, 236, 4);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    assert!(logged(&w, "skill 236"));
    assert_eq!(param_of(&w, 0), 3);
    // f = 0, c = 2: 50, 10 (< 15) → c := 1, A2.
    let (mut w, _) = seeded(act_row(27, &hulk), 2, |v| v[0] < 80 && v[1] < 15);
    set_param_of(&mut w, 1, 2);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 1), 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    // f = 1: swing, f := 0, c := 3; a type-1 timer at frame + 10 → wait
    // 10 + aip5.
    let mut w = world(act_row(27, &hulk));
    w.seed(1);
    set_param_of(&mut w, 0, 1);
    let mon = w.mon;
    w.game.schedule_event(mon, 1, 10, None, 0, 0).unwrap();
    w.think_with(Some(w.player), 1, true);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (0, 3));
    assert_eq!(w.thinks(), [15]);
    assert_eq!(steps_since(&w, 1), 0);
    // Not C: f := 0, lunge.
    let mut w = world(act_row(27, &hulk));
    set_param_of(&mut w, 0, 2);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.vel_request().method, 13);
}

// ---- ZakarumZealot --------------------------------------------------

#[test]
fn zakarum_zealot_vectors() {
    let zealot = [65, 50, 35, 50];
    // No quest, AI state 0, C, param 0 = 1: 70 (≥ 65), 85 (≥ 80) → param 0
    // := 0, circle 4 at T.
    let (mut w, _) = seeded(act_row(48, &zealot), 2, |v| v[0] >= 65 && v[1] >= 80);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    // Param 0 = 0: param 0 := 1; draw < aip2 → A2.
    let (mut w, _) = seeded(act_row(48, &zealot), 1, |v| v[0] < 50);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    // The quest flight: T a player, Act III level, A3Q5 flag 0 → velocity,
    // run away by 8.
    let mut w = world(act_row(48, &zealot));
    w.monstats[0].velocity = 5;
    w.monstats[0].run = 8;
    let mut lv = w.levels[0].clone();
    lv.act = 2;
    w.levels = vec![lv];
    w.fake.x.quest_flag = true;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.vel_request().speed, 60);
    assert_eq!(w.fake.modes(), [point_mode(mode::RUN, 92, 100)]);
    // Not C: param 0 := 0; draw < aip4 → run to T.
    let (mut w, _) = seeded(act_row(48, &zealot), 1, |v| v[0] < 50);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(param_of(&w, 0), 0);
    assert_eq!(w.fake.modes(), [unit_mode(mode::RUN, w.player)]);
}

#[test]
fn zakarum_zealot_hurt_cooldown() {
    let zealot = [65, 50, 35, 50];
    // AI state 3, L < aip3, the run fails → param 1 := 5, then counted down
    // to 4 the same think (step 3).
    let (mut w, _) = seeded(act_row(48, &zealot), 1, |_| true);
    w.fake.ai_state = 3;
    w.fake.life = 20;
    w.fake.fail_modes.insert(mode::RUN);
    w.fake.fail_modes.insert(mode::WALK);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(param_of(&w, 1), 4);
    // Collision 0x40 at the own position, C → circle 4.
    let (mut w, _) = seeded(act_row(48, &zealot), 1, |_| true);
    w.fake.ai_state = 3;
    set_param_of(&mut w, 1, 2);
    w.fake.x.point_collides = true;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(param_of(&w, 1), 2);
}

// ---- ZakarumPriest --------------------------------------------------

#[test]
fn zakarum_priest_vectors() {
    let cantor1 = [25, 5, 50, 25, 120, 36];
    // C = 0, AI state 0, no heal target, clear line, frame 100, params 0:
    // 20 (< 25), 3 (< 5) → MonBlizzard at T, param 1 := 220.
    let (mut w, lo) = seeded(act_row(49, &cantor1), 2, |v| v[0] < 25 && v[1] < 5);
    for (k, id) in [(1, 120), (2, 121), (3, 122), (4, 123)] {
        give_skill(&mut w, k, id, 10);
    }
    w.game.frame = 100;
    w.think_with(Some(w.player), 9, false);
    assert!(logged(&w, "skill 123"));
    assert_eq!(param_of(&w, 1), 220);
    assert_eq!(steps_since(&w, lo), 2);
    // The heal scan keeps the last qualifying unit (edge case 1).
    let (mut w, _) = seeded(act_row(49, &cantor1), 1, |v| v[0] < 25);
    give_skill(&mut w, 1, 120, 10);
    w.monstats[0].baseid = 238;
    let a = w.add_unit(UnitType::Monster, (105, 100));
    let b = w.add_unit(UnitType::Monster, (110, 100));
    for u in [a, b] {
        w.fake.life_of.insert(u, 50);
    }
    w.fake.life_of.insert(w.mon, 100);
    let order = w.game.lists.room_units(w.room);
    let last = *order.iter().rfind(|&&u| u == a || u == b).unwrap();
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.fake.modes(), [unit_mode(10, last)]);
}

#[test]
fn zakarum_priest_teleports_past_the_target() {
    let cantor1 = [25, 5, 50, 25, 120, 36];
    // C, L < 33, frame > param 0: cooldown 4 × aip5, point T + 4(T − U).
    let mut w = world(act_row(49, &cantor1));
    give_skill(&mut w, 3, 122, 10);
    w.game.frame = 10;
    w.fake.life = 20;
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 490);
    assert!(logged(&w, "check 122 None 125 100"));
    assert_eq!(w.fake.modes(), [point_mode(10, 125, 100)]);
    // The check fails: the cooldown is spent anyway.
    let mut w = world(act_row(49, &cantor1));
    give_skill(&mut w, 3, 122, 10);
    w.fake.x.check_fails = true;
    w.game.frame = 10;
    w.fake.life = 20;
    w.fake.ai_state = 3;
    w.think_with(Some(w.player), 9, false);
    assert_eq!(param_of(&w, 0), 490);
    assert!(logged(&w, "check 122 None 110 100"));
}

// ---- FrogDemon ------------------------------------------------------

#[test]
fn frog_demon_vectors() {
    let frog = [65, 20, 50, 50, 20, 12, 15, 9];
    // s = 2, C: 30 (≥ 20), 70 (≥ 65), 60 (≥ 50) → wait 15.
    let (mut w, lo) = seeded(act_row(52, &frog), 3, |v| {
        v[0] >= 20 && v[1] >= 65 && v[2] >= 50
    });
    set_param_of(&mut w, 2, 2);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.thinks(), [15]);
    assert_eq!(steps_since(&w, lo), 3);
    // s = 0, D > 12: submerge at T, s := 1, sink (its wait 12 replaces 8).
    let mut w = world(act_row(52, &frog));
    give_skill(&mut w, 1, 131, 14);
    give_skill(&mut w, 2, 132, 8);
    w.think_with(Some(w.player), 13, false);
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    assert_eq!((param_of(&w, 2), w.thinks()), (1, vec![12]));
    assert!(logged(&w, "pattern 5"));
    // s = 1, D < aip8 and the land succeeds: Emerge at itself, s := 2.
    w.fake.x.place_ok = true;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.fake.modes().last().unwrap(), &unit_mode(8, w.mon));
    assert_eq!(param_of(&w, 2), 2);
    // s = 1, far: sink, wait 24, n += 1.
    let mut w = world(act_row(52, &frog));
    give_skill(&mut w, 2, 132, 8);
    set_param_of(&mut w, 2, 1);
    w.think_with(Some(w.player), 30, false);
    assert_eq!((param_of(&w, 1), w.thinks()), (1, vec![24]));
    // s ≥ 2 with T = 0: wait 32.
    let mut w = world(act_row(52, &frog));
    set_param_of(&mut w, 2, 2);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [32]);
    // s = 0, no skills, the spot is clear: s := 2, idle 12.
    let mut w = world(act_row(52, &frog));
    w.think_with(None, 0, false);
    assert_eq!((param_of(&w, 2), w.thinks()), (2, vec![12]));
}

#[test]
fn frog_demon_alternate() {
    let frog = [65, 20, 50, 50, 20, 12, 15, 9];
    let mut w = world(act_row(52, &frog));
    give_skill(&mut w, 1, 131, 14);
    set_param_of(&mut w, 2, 2);
    w.store.control_mut(w.mon).unwrap().function = 0x005F_81D0;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(param_of(&w, 2), 1);
    assert_eq!(w.fake.modes(), [unit_mode(14, w.player)]);
    // Special state 10: re-install and wait 1.
    let mut w = world(act_row(52, &frog));
    give_skill(&mut w, 1, 131, 14);
    set_param_of(&mut w, 2, 2);
    let c = w.store.control_mut(w.mon).unwrap();
    c.function = 0x005F_81D0;
    c.special_state = 10;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(w.store.control(w.mon).unwrap().params, [0; 3]);
    assert_eq!(w.thinks(), [1]);
}

// ---- FetishShaman ---------------------------------------------------

#[test]
fn fetish_shaman_inferno_and_raise() {
    let shaman = [40, 0, 15, 66, 50];
    // D < R (the skill level 5): command {1, T GUID, type} to the minions,
    // Inferno at T.
    let mut w = world(act_row(65, &shaman));
    give_skill(&mut w, 1, 135, 4);
    w.fake.x.skill_level.insert(135, 5);
    let mon = w.mon;
    let mon_ref = UnitRef {
        ty: UnitType::Monster,
        guid: w.game.lists.unit(mon).unwrap().guid,
    };
    let minion = w.add_unit(UnitType::Monster, (90, 100));
    w.store.entry(minion).control = Some(AiControl::default());
    w.store.control_mut(mon).unwrap().minion_owner = Some(mon_ref);
    let mg = w.game.lists.unit(minion).unwrap().guid;
    w.store.control_mut(mon).unwrap().minions = vec![mg];
    w.think_with(Some(w.player), 4, false);
    let pg = w.game.lists.unit(w.player).unwrap().guid as i32;
    assert_eq!(
        w.store.control(minion).unwrap().commands[0].params,
        [1, pg, 0, 0, 0]
    );
    assert_eq!(w.fake.modes(), [unit_mode(4, w.player)]);
    // A fetish corpse: P(aip1) and the check → command 14; squared d 16 >
    // aip3 15 (unsquared) → wander near it 10.
    let (mut w, _) = seeded(act_row(65, &shaman), 1, |v| v[0] < 40);
    give_skill(&mut w, 3, 136, 14);
    w.monstats[0].baseid = 141;
    let c = w.add_unit(UnitType::Monster, (104, 100));
    w.fake.anim.insert(c, mode::DEAD);
    w.fake.x.flags.insert(c, 0x2);
    let mon = w.mon;
    w.store.entry(c).control = Some(AiControl {
        minion_owner: Some(UnitRef {
            ty: UnitType::Monster,
            guid: w.game.lists.unit(mon).unwrap().guid,
        }),
        ..AiControl::default()
    });
    w.think_with(Some(w.player), 20, false);
    assert!(logged(&w, &format!("check 136 Some({c:?}) 0 0")));
    let m = w.fake.modes();
    assert_eq!(m.len(), 1);
    assert!(m[0].starts_with("mode 2 Point"));
    // At squared d 9 ≤ 15: the sequence skill on it.
    let (mut w, _) = seeded(act_row(65, &shaman), 1, |v| v[0] < 40);
    give_skill(&mut w, 3, 136, 14);
    w.monstats[0].baseid = 141;
    let c = w.add_unit(UnitType::Monster, (103, 100));
    w.fake.anim.insert(c, mode::DEAD);
    w.fake.x.flags.insert(c, 0x2);
    w.fake.align = 0;
    w.monstats[0].aip2 = 1; // H ≠ 0: no minion-owner test
    w.think_with(Some(w.player), 20, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::SEQUENCE, c)]);
    // Nothing: state 12 off; draw < aip4 → circle 4, else idle 10.
    let (mut w, _) = seeded(act_row(65, &shaman), 1, |v| v[0] >= 66);
    w.fake.states.insert((w.mon, 12));
    w.think_with(Some(w.player), 20, false);
    assert!(!w.fake.states.contains(&(w.mon, 12)));
    assert_eq!(w.thinks(), [10]);
}

// ---- HighPriest -----------------------------------------------------

#[test]
fn high_priest_vectors() {
    let council = [75, 25, 125, 40, 70, 8, 15, 30];
    // e = 0, C = 0, D = 40, frame > param 1, `MissS1` `Range` 42: 30 (≥
    // 25: no heal scan), no hydra draw (D ≥ 30), no S1 draw (40 ≥ 42 − 2),
    // 50 (< 80) → walk to T with 6 steps.
    let (mut w, lo) = seeded(act_row(85, &council), 2, |v| v[0] >= 25 && v[1] < 80);
    give_skill(&mut w, 1, 140, 10);
    give_skill(&mut w, 2, 141, 10);
    w.monstats[0].misss1 = 1;
    let mut miss = Missiles::decode(&vec![0u8; Missiles::SIZE]);
    miss.range = 42;
    w.missiles = vec![miss.clone(), miss];
    w.game.frame = 10;
    w.think_with(Some(w.player), 40, false);
    assert!(logged(&w, "steps 6"));
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
    assert_eq!(steps_since(&w, lo), 2);
    // The hydra: D < aip8, draw < aip4, k = mask(4) → T ± 5.
    let lo = seed_raw(3, |v| v[0] % 100 >= 25 && v[1] % 100 < 40);
    let mut w = world(act_row(85, &council));
    w.seed(lo);
    give_skill(&mut w, 1, 140, 10);
    give_skill(&mut w, 2, 141, 10);
    w.game.frame = 10;
    w.think_with(Some(w.player), 20, false);
    let mut s = Seed::init_low(lo);
    s.step();
    s.step();
    let k = (s.step() & 3) as usize;
    // The request holds T and the offset point; T ≠ 0 wins (the mode
    // request record): the point is drawn but
    // not used by the mode set.
    let _off = [(-5, -5), (5, -5), (5, 5), (-5, 5)][k];
    assert_eq!(w.fake.modes(), [unit_mode(10, w.player)]);
    assert_eq!(param_of(&w, 1), 110);
    // e = 0, C: P(aip1) → e := 1, A1.
    let (mut w, _) = seeded(act_row(85, &council), 1, |v| v[0] < 75);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 0), 1);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    // e = 1, C: draw < aip7 → S1 at T.
    let (mut w, _) = seeded(act_row(85, &council), 1, |v| v[0] < 15);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::SKILL1, w.player)]);
    // e = 1, not C: draw ≥ aip7 (D ≥ 6 skips it), < aip6 → e := 0, idle 10.
    let (mut w, _) = seeded(act_row(85, &council), 1, |v| v[0] < 8);
    set_param_of(&mut w, 0, 1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!((param_of(&w, 0), w.thinks()), (0, vec![10]));
}

// ---- FetishBlowgun --------------------------------------------------

#[test]
fn fetish_blowgun_vectors() {
    let blow = [20, 30];
    // s = 0, n = 3, C, S present: 50 (≥ 30), `lo' % 3` = 0 → n := 4 > 3 →
    // s := 1, n := 0; A1 at S.
    let lo = seed_raw(2, |v| v[0] % 100 >= 30 && v[1] % 3 == 0);
    let mut w = world(act_row(96, &blow));
    w.seed(lo);
    set_param_of(&mut w, 1, 3);
    let s = w.add_unit(UnitType::Player, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.think_with(Some(w.player), 1, true);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (1, 0));
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
    // Command 1 naming a unit (type in param 2, GUID in param 1): A1 at it,
    // command freed.
    let mut w = world(act_row(96, &blow));
    let g = w.game.lists.unit(w.player).unwrap().guid as i32;
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().commands = vec![AiCommand {
        params: [1, g, 0, 0, 0],
    }];
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert!(w.store.control(mon).unwrap().commands.is_empty());
    // s = 2, D ≤ 12, the escape fails: A1 at S = 0, s, n := 0.
    let (mut w, _) = seeded(act_row(96, &blow), 1, |v| v[0] >= 30);
    set_param_of(&mut w, 0, 2);
    w.fake.walk_fails = true;
    w.think_with(Some(w.player), 5, false);
    assert_eq!(
        w.fake.modes().last().unwrap(),
        &point_mode(mode::ATTACK1, 0, 0)
    );
    assert_eq!(param_of(&w, 0), 0);
    // Not C, D > aip1: velocity 50, wander near T 6.
    let mut w = world(act_row(96, &blow));
    w.think_with(Some(w.player), 21, false);
    assert_eq!(w.vel_request().speed, 50);
}

// ---- WillOWisp -----------------------------------------------------

#[test]
fn will_o_wisp_ritual() {
    let wisp = [40, 70, 50];
    // The trigger: roll(1000) ≤ difficulty + 2, then the gather of exactly
    // 4 wisps: params (6, 0, slot), idle 337 − frame % 337 on each.
    let lo = seed_raw(1, |v| v[0] % 1000 <= 2);
    let mut w = world(act_row(25, &wisp));
    w.seed(lo);
    w.monstats[0].baseid = 118;
    w.game.frame = 400;
    let mut found = vec![w.mon];
    for i in 0..3 {
        let u = w.add_unit(UnitType::Monster, (90 + i, 100));
        w.store.entry(u).control = Some(AiControl::default());
        found.push(u);
    }
    w.fake.x.wisps = found.clone();
    w.think_with(Some(w.player), 5, false);
    for (i, &u) in found.iter().enumerate() {
        assert_eq!(w.store.control(u).unwrap().params, [6, 0, i as i32 + 1]);
    }
    assert_eq!(w.thinks(), [400 + 337 - 63]);
    // Shape 6, slot 1, near its stand point T + (−10, −3), frame % 67 = 0,
    // n < 3: SC at T + (1, −5).
    let mut w = world(act_row(25, &wisp));
    w.game.frame = 670;
    w.store.control_mut(w.mon).unwrap().params = [6, 0, 1];
    w.fake.pos.insert(w.mon, (95, 97));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [point_mode(7, 106, 95)]);
    assert_eq!(param_of(&w, 1), 1);
    // n ≥ 3: next shape; the buff on a living player after every shape.
    let mut w = world(act_row(25, &wisp));
    w.game.frame = 670;
    w.store.control_mut(w.mon).unwrap().params = [6, 3, 1];
    w.fake.pos.insert(w.mon, (95, 97));
    w.think_with(Some(w.player), 10, false);
    assert_eq!((param_of(&w, 0), param_of(&w, 1)), (7, 0));
    assert!(logged(&w, &format!("buff {} 50 1728670", w.player.0)));
    // Any other count: s := 2, idle 8.
    let lo = seed_raw(1, |v| v[0] % 1000 <= 2);
    let mut w = world(act_row(25, &wisp));
    w.seed(lo);
    w.game.frame = 400;
    w.think_with(Some(w.player), 5, false);
    assert_eq!((param_of(&w, 0), w.thinks()), (2, vec![408]));
}

#[test]
fn will_o_wisp_casts() {
    let wisp = [40, 70, 50];
    // s = 2, not C: SC at T (no draw), s := 0.
    let mut w = world(act_row(25, &wisp));
    w.seed(1);
    set_param_of(&mut w, 0, 2);
    set_param_of(&mut w, 2, 1_000_000);
    w.think_with(Some(w.player), 20, false);
    assert_eq!(w.fake.modes(), [unit_mode(7, w.player)]);
    assert_eq!((param_of(&w, 0), steps_since(&w, 1)), (0, 0));
    // s = 1, n ≤ 0, C: S1 with no target, s := 3.
    let mut w = world(act_row(25, &wisp));
    set_param_of(&mut w, 0, 1);
    set_param_of(&mut w, 2, 1_000_000);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [point_mode(mode::SKILL1, 0, 0)]);
    assert_eq!(param_of(&w, 0), 3);
    // Baptism slot 1 near T + (−5, −5), frame % 67 = 0: SC at T + (6, 3).
    let mut w = world(act_row(25, &wisp));
    w.game.frame = 670;
    w.store.control_mut(w.mon).unwrap().params = [5, 0, 1];
    w.fake.pos.insert(w.mon, (100, 95));
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.fake.modes(), [point_mode(7, 111, 103)]);
}

// ---- Mephisto ------------------------------------------------------

#[test]
fn mephisto_vectors() {
    let meph = [15, 25, 25];
    // C, L = 100: 50 (> 15) → s := 3; 70 (< 80), 85 (≥ 80) → Skill3 at T;
    // param 2 := 0.
    let (mut w, lo) = seeded(act_row(50, &meph), 3, |v| {
        v[0] > 15 && v[1] < 80 && v[2] >= 80
    });
    give_skill(&mut w, 3, 150, 5);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(5, w.player)]);
    assert_eq!(param_of(&w, 2), 0);
    assert_eq!(steps_since(&w, lo), 3);
    // Not C, D > 20: s := 4: velocity 50, wander near T 6.
    let mut w = world(act_row(50, &meph));
    w.think_with(Some(w.player), 21, false);
    assert_eq!(w.vel_request().speed, 50);
    assert_eq!(param_of(&w, 2), 0);
    // Case 2 with c = 0: c := 2, circle 3 with delete.
    let (mut w, _) = seeded(act_row(50, &meph), 1, |v| v[0] <= 15);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(param_of(&w, 1), 2);
    assert_eq!(param_of(&w, 2), 2);
}

#[test]
fn mephisto_roams() {
    let meph = [15, 25, 25];
    // Not C, s = 0, L = 100 (K = 0): draw ≥ 50 → roam: draw ≥ 65 → idle 10.
    let (mut w, _) = seeded(act_row(50, &meph), 2, |v| v[0] >= 50 && v[1] >= 65);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [10]);
    // s > 4 (as unsigned): roam with no state draw.
    let (mut w, lo) = seeded(act_row(50, &meph), 1, |v| v[0] >= 65);
    set_param_of(&mut w, 2, -1);
    w.think_with(Some(w.player), 10, false);
    assert_eq!(w.thinks(), [10]);
    assert_eq!((param_of(&w, 2), steps_since(&w, lo)), (0, 1));
    // Case 2, c > 0: the skill share 100 / n; on Normal r < 2q → Skill4 at
    // the pick (T: no hostile unit).
    let (mut w, _) = seeded(act_row(50, &meph), 3, |v| v[0] <= 15 && v[2] < 33);
    for k in 1..=6 {
        give_skill(&mut w, k, 150 + k as u16, 5);
    }
    set_param_of(&mut w, 1, 1);
    w.think_with(Some(w.player), 1, true);
    assert!(logged(&w, "skill 154"));
    assert_eq!(param_of(&w, 1), 2);
}

#[test]
fn zakarum_priest_blocked_line() {
    let cantor1 = [25, 5, 50, 25, 120, 36];
    // Blocked line: P(aip1), `Skill4`, frame > param 1, draw < aip2 →
    // MonBlizzard; else circle / idle 20.
    let (mut w, _) = seeded(act_row(49, &cantor1), 2, |v| v[0] < 25 && v[1] < 5);
    give_skill(&mut w, 4, 123, 10);
    w.fake.line_blocked.insert(w.player);
    w.game.frame = 10;
    w.think_with(Some(w.player), 9, false);
    assert!(logged(&w, "skill 123"));
    assert_eq!(param_of(&w, 1), 130);
    let (mut w, _) = seeded(act_row(49, &cantor1), 2, |v| v[0] >= 25 && v[1] >= 30);
    give_skill(&mut w, 4, 123, 10);
    w.fake.line_blocked.insert(w.player);
    w.think_with(Some(w.player), 9, false);
    assert_eq!(w.thinks(), [20]);
}
