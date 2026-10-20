use super::*;

/// AI indices (monstats `AI`).
const SMITH: u16 = 98;
const GRISWOLD: u16 = 90;
const GOOD_NPC_RANGED: u16 = 60;
const NPC_OUT_OF_TOWN: u16 = 31;

/// The first unit-seed low word whose first `k` `lo' % 100` values
/// satisfy `pred`.
pub(super) fn seed_with(k: usize, pred: impl Fn(&[u32]) -> bool) -> u32 {
    (1..1_000_000u32)
        .find(|&lo| {
            let mut s = Seed::init_low(lo);
            let v: Vec<u32> = (0..k).map(|_| s.step() % 100).collect();
            pred(&v)
        })
        .expect("a seed")
}

/// Steps the monster's seed took since it was set to `init_low(lo)`.
pub(super) fn steps_since(w: &World, lo: u32) -> usize {
    let now = w.fake.seeds[&w.mon];
    let mut s = Seed::init_low(lo);
    (0..64)
        .find(|_| {
            let hit = s == now;
            s.step();
            hit
        })
        .expect("seed not reached")
}

pub(super) fn unit_mode(m: u8, u: UnitId) -> String {
    format!("mode {m} Unit({u:?})")
}

impl World {
    /// Runs the current AI function with `p`.
    pub(super) fn think_with(&mut self, target: Option<UnitId>, distance: i32, combat: bool) {
        let p = TickParam {
            target,
            distance,
            combat,
            class: 0,
            class2: 0,
        };
        let mon = self.mon;
        self.with(|g, cx| {
            let f = cx.store.control(mon).unwrap().function;
            run_function(g, cx, f, mon, &p)
        });
    }

    pub(super) fn add_unit(&mut self, ty: UnitType, at: (i32, i32)) -> UnitId {
        let u = self.game.spawn_unit(ty, Some(self.room), false).unwrap();
        self.fake.pos.insert(u, at);
        u
    }

    pub(super) fn commands(&self) -> Vec<[i32; 5]> {
        let c = self.store.control(self.mon).unwrap();
        c.commands.iter().map(|k| k.params).collect()
    }

    pub(super) fn vel_request(&self) -> VelocityRequest {
        self.store.get(self.mon).unwrap().velocity
    }
}

// ---- Smith, Griswold --------------------------------------------

#[test]
fn smith_attacks_or_walks_by_life() {
    // C → A1 at T, no draw.
    let mut w = World::new(monstats(SMITH, [0; 5], 15));
    w.seed(1);
    w.think_with(Some(w.player), 1, true);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
    assert!(w.thinks().is_empty());
    // Not C: speed (100 − L) >> 1 with L clamped to 0..100, method and
    // steps unchanged; walk to T with flags 7. No draws.
    for (life, speed) in [(60, 20), (-10, 50), (150, 0), (99, 0), (97, 1)] {
        let mut w = World::new(monstats(SMITH, [0; 5], 15));
        w.store.entry(w.mon).velocity = VelocityRequest {
            method: 3,
            speed: 9,
            steps: 4,
        };
        w.fake.life = life;
        w.seed(1);
        w.think_with(Some(w.player), 10, false);
        let want_speed = if speed == 0 { 9 } else { speed };
        assert_eq!(
            w.vel_request(),
            VelocityRequest {
                method: 3,
                speed: want_speed,
                steps: 4
            },
            "life {life}"
        );
        assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
        assert_eq!(steps_since(&w, 1), 0);
    }
    // Flags 7 on a failed walk: control flag 0x40, delete thinks, the
    // flag-2 fallback draw.
    let mut w = World::new(monstats(SMITH, [0; 5], 15));
    w.fake.walk_fails = true;
    w.game
        .schedule_event(w.mon, u32::from(EVENT_THINK), 50, None, 0, 0)
        .unwrap();
    w.seed(4_014_346_870); // 0 < 70 → wander 4
    w.think_with(Some(w.player), 10, false);
    assert_eq!(
        w.store.control(w.mon).unwrap().flags & flag::FORCE_LOS,
        flag::FORCE_LOS
    );
    assert!(w.thinks().is_empty());
    assert_eq!(w.fake.modes().len(), 2);
}

#[test]
fn griswold_vectors() {
    // Draws `lo' % 100` 51, 87, 53, 0 (Test vectors).
    let combat = [true, false, true, true];
    for (s, a1) in SEEDS.iter().zip(combat) {
        let mut w = World::new(monstats(GRISWOLD, [0; 5], 15));
        w.seed(*s);
        w.think_with(Some(w.player), 1, true);
        if a1 {
            assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
            assert!(w.thinks().is_empty());
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [10]);
        }
        assert_eq!(steps_since(&w, *s), 1, "seed {s}");
    }
    let walk = [false, false, false, true];
    for (s, walks) in SEEDS.iter().zip(walk) {
        let mut w = World::new(monstats(GRISWOLD, [0; 5], 15));
        w.seed(*s);
        w.think_with(Some(w.player), 10, false);
        if walks {
            assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, w.player)]);
            assert!(w.thinks().is_empty());
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [10]);
        }
        assert_eq!(steps_since(&w, *s), 1, "seed {s}");
    }
}

// ---- GoodNpcRanged ----------------------------------------------

fn ranged_world() -> World {
    World::new(monstats(GOOD_NPC_RANGED, [0; 5], 15))
}

#[test]
fn good_npc_ranged_not_neutral_idles_5() {
    let mut w = ranged_world();
    w.fake.anim.insert(w.mon, mode::WALK);
    w.seed(4_014_346_870);
    w.think_with(None, 0, false);
    // "idle 5" first returns the unit to neutral.
    assert_eq!(w.fake.modes(), [unit_mode(mode::NEUTRAL, w.mon)]);
    assert_eq!(w.thinks(), [5]);
    assert_eq!(steps_since(&w, 4_014_346_870), 0);
    // It is also the think of special state 5.
    assert_eq!(SPECIAL_TABLE[5].think, AI_TABLE[60].think);
    assert!(implemented(SPECIAL_TABLE[5].think));
}

#[test]
fn good_npc_ranged_in_town_wanders_or_idles() {
    // In town: no secondary search; `lo' % 100` < 20 → wander 5, else
    // idle 10. Draws 51, 87, 53, 0.
    for (i, s) in SEEDS.iter().enumerate() {
        let mut w = ranged_world();
        w.fake.town.insert(w.room);
        let other = w.add_unit(UnitType::Monster, (105, 100));
        w.fake.secondary = Some((other, 5));
        w.seed(*s);
        w.think_with(None, 0, false);
        if i == 3 {
            let mut seed = Seed::init_low(*s);
            seed.step();
            let (x, y) = wander_point(&mut seed, (100, 100), 5);
            assert_eq!(w.fake.modes(), [format!("mode 2 Point({x}, {y})")]);
            assert_eq!(w.fake.seeds[&w.mon], seed);
        } else {
            assert!(w.fake.modes().is_empty(), "seed {s}");
            assert_eq!(w.thinks(), [10]);
            assert_eq!(steps_since(&w, *s), 1);
        }
    }
}

#[test]
fn good_npc_ranged_out_of_town() {
    // S at E < 20: roll(100) < 30 → A1 at S (any class but 271).
    let mut w = ranged_world();
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(4_014_346_870); // 0
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
    assert_eq!(steps_since(&w, 4_014_346_870), 1);

    // roguehire (271): `Skill1` in `Sk1mode` at S.
    let mut w = ranged_world();
    w.monstats[0].skill1 = 7;
    w.modes[0] = [10, 0, 0, 0, 0, 0, 0, 0];
    w.fake.class.insert(w.mon, 271);
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 19));
    w.seed(4_014_346_870);
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"skill 7".to_string()));
    assert!(w.fake.log.contains(&"skillflag".to_string()));
    assert_eq!(w.fake.modes(), [unit_mode(10, s)]);

    // First test fails, second roll(100) < 30 → circle 4 at S (one more
    // step for the method), no delete.
    let lo = seed_with(3, |v| v[0] >= 30 && v[1] < 30);
    let mut w = ranged_world();
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.game
        .schedule_event(w.mon, u32::from(EVENT_THINK), 40, None, 0, 0)
        .unwrap();
    w.seed(lo);
    w.think_with(None, 0, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::WALK, s)]);
    assert_eq!(w.vel_request().steps, 4);
    assert_eq!(steps_since(&w, lo), 3);
    assert_eq!(w.thinks(), [40], "circle keeps the pending think");

    // Both fail → idle 10.
    let lo = seed_with(2, |v| v[0] >= 30 && v[1] >= 30);
    let mut w = ranged_world();
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(lo);
    w.think_with(None, 0, false);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [10]);
    assert_eq!(steps_since(&w, lo), 2);

    // No S, or S at E ≥ 20: step 3 (20 % wander 5, else idle 10).
    for sec in [None, Some(20)] {
        let mut w = ranged_world();
        if let Some(e) = sec {
            let s = w.add_unit(UnitType::Monster, (120, 100));
            w.fake.secondary = Some((s, e));
        }
        w.seed(1); // 51 → idle 10
        w.think_with(None, 0, false);
        assert!(w.fake.modes().is_empty());
        assert_eq!(w.thinks(), [10]);
        assert_eq!(steps_since(&w, 1), 1);
    }
}

// ---- the interaction handler -------------------------------------

/// An NPC-AI monster (NpcOutOfTown's index, think unused) with an interaction
/// block and a player at `d` tiles (full-size distance, size 1).
fn interaction_world(d: i32) -> World {
    let mut w = World::new(monstats(NPC_OUT_OF_TOWN, [0; 5], 15));
    w.fake.npc_block = true;
    // Full-size distance with size 1 on one axis: (dx − 1) · 2 / 2.
    w.fake.pos.insert(w.player, (100 + d + 1, 100));
    w.fake.nearest = Some((w.player, d < 4));
    w
}

fn interact(w: &mut World) -> bool {
    let mon = w.mon;
    w.with(|g, cx| super::super::npc::npc_interaction_think(g, cx, mon))
}

#[test]
fn interaction_needs_a_block_and_a_player() {
    let mut w = interaction_world(10);
    w.fake.npc_block = false;
    assert!(!interact(&mut w));
    assert!(w.thinks().is_empty());
    // Not a monster.
    let mut w = interaction_world(10);
    let npc = w.add_unit(UnitType::Object, (100, 100));
    let r = w.with(|g, cx| super::super::npc::npc_interaction_think(g, cx, npc));
    assert!(!r);
    // No player: the NPC itself.
    let mut w = interaction_world(10);
    w.fake.nearest = None;
    assert!(!interact(&mut w));
    assert!(w.thinks().is_empty());
}

#[test]
fn interaction_talking_or_busy() {
    let set = |w: &mut World, p: [i32; 3]| {
        w.store.control_mut(w.mon).unwrap().params = p;
    };
    // Param 0 > 36 and (param 1, param 2) farther than 2 → walk there.
    let mut w = interaction_world(10);
    set(&mut w, [40, 120, 100]);
    assert!(interact(&mut w));
    assert_eq!(w.fake.modes(), ["mode 2 Point(120, 100)"]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 39);
    // Near: stop the path, idle 8.
    let mut w = interaction_world(10);
    set(&mut w, [40, 101, 100]);
    assert!(interact(&mut w));
    assert_eq!(w.fake.stops, 1);
    assert_eq!(w.thinks(), [8]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 39);
    // Talking with param 0 ≤ 0: return 1, nothing scheduled (edge 11).
    let mut w = interaction_world(10);
    w.fake.interacting = true;
    set(&mut w, [0, 0, 0]);
    assert!(interact(&mut w));
    assert!(w.thinks().is_empty());
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.store.control(w.mon).unwrap().params[0], -1);
    // A negative param 0 becomes 0.
    let mut w = interaction_world(10);
    w.fake.interacting = true;
    set(&mut w, [-5, 0, 0]);
    assert!(interact(&mut w));
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    // Busy: P in the interaction list, or a busy player.
    for list in [true, false] {
        let mut w = interaction_world(10);
        if list {
            w.fake.npc_list.insert(w.player);
        } else {
            w.fake.busy.insert(w.player);
        }
        set(&mut w, [0, 0, 0]);
        assert!(interact(&mut w));
        assert!(w.thinks().is_empty());
    }
}

#[test]
fn interaction_greets_close_or_far_players() {
    for d in [2, 24] {
        let mut w = interaction_world(d);
        assert!(interact(&mut w));
        assert_eq!(w.fake.stops, 1);
        assert_eq!(w.store.control(w.mon).unwrap().params[1], 60);
        assert!(w.fake.log.contains(&"sound 18".to_string()));
        assert!(w.fake.log.contains(&format!("sound-to {:?}", w.player)));
        assert_eq!(w.thinks(), [20]);
        // Next time the countdown runs, no sound.
        w.fake.log.clear();
        assert!(interact(&mut w));
        assert_eq!(w.store.control(w.mon).unwrap().params[1], 59);
        assert!(!w.fake.log.iter().any(|l| l.starts_with("sound")));
        assert_eq!(w.thinks(), [20]);
    }
    // A negative countdown becomes 0.
    let mut w = interaction_world(2);
    w.store.control_mut(w.mon).unwrap().params[1] = -3;
    assert!(interact(&mut w));
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 0);
    // The nearest "player" is a monster: no sound.
    let mut w = interaction_world(2);
    let m = w.add_unit(UnitType::Monster, (103, 100));
    w.fake.nearest = Some((m, true));
    assert!(interact(&mut w));
    assert!(!w.fake.log.iter().any(|l| l.starts_with("sound")));
}

#[test]
fn interaction_home_check_and_walk_around() {
    // No home command: the home step makes one at the NPC's position.
    let mut w = interaction_world(10);
    assert!(interact(&mut w));
    assert_eq!(w.commands(), [[10, 100, 100, 0, 0]]);
    assert_eq!(w.thinks(), [20]);
    // Home within 16: velocity method 1 → 7, walk in radius of P.
    assert!(interact(&mut w));
    assert_eq!(w.vel_request().method, 7);
    assert_eq!(w.fake.log.last().unwrap(), "radius 3 2");
    let mut w = interaction_world(4);
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [10, 90, 100, 0, 0],
    }];
    assert!(interact(&mut w));
    assert_eq!(w.fake.log.last().unwrap(), "radius 2 2");
    // Home farther than 16: command 4 := (H.x, H.y, 12, 10), idle 10.
    let mut w = interaction_world(10);
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand {
        params: [10, 70, 100, 0, 0],
    }];
    assert!(interact(&mut w));
    assert!(w.commands().contains(&[4, 70, 100, 12, 10]));
    assert_eq!(w.thinks(), [10]);
}

// ---- NpcOutOfTown -----------------------------------------------

fn cain() -> World {
    let mut w = World::new(monstats(NPC_OUT_OF_TOWN, [0; 5], 15));
    w.fake.class.insert(w.mon, 146);
    w
}

fn portal_cmd(w: &World) -> [i32; 5] {
    *w.commands().iter().find(|c| c[0] == 3).unwrap()
}

fn set_portal(w: &mut World, p: [i32; 5]) {
    w.store.control_mut(w.mon).unwrap().commands = vec![AiCommand { params: p }];
}

#[test]
fn npc_out_of_town_portal_setup() {
    let mut w = cain();
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(w.commands(), [[3, 103, 103, 1, 0]]);
    assert!(w.fake.log.contains(&"quest setup Cain".to_string()));
    assert_eq!(w.thinks(), [1]);
    assert_eq!(steps_since(&w, 1), 0, "no draws");
    // Setup fails: leave (town portal, life 0, dead at its position).
    let mut w = cain();
    w.fake.portal_setup_fails = true;
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"quest town portal Cain".to_string()));
    assert!(w.fake.log.contains(&format!("setlife {} 0", w.mon.0)));
    assert!(w
        .fake
        .modes()
        .contains(&"mode 12 Point(100, 100)".to_string()));
    assert_eq!(w.commands(), [[3, 103, 103, 1, 0]]);
    // drehyaiced uses the Act 5 functions.
    let mut w = cain();
    w.fake.class.insert(w.mon, 527);
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"quest setup Drehya".to_string()));
    // Any other class takes cain1's Act 1 quest 4 functions.
    let mut w = cain();
    w.fake.class.insert(w.mon, 147);
    w.think_with(None, 0, false);
    assert!(w.fake.log.contains(&"quest setup Cain".to_string()));
    assert_eq!(w.commands(), [[3, 103, 103, 1, 0]]);
    assert_eq!(w.thinks(), [1]);
}

#[test]
fn npc_out_of_town_gates() {
    // Talking → idle 40 (drehyaiced's update runs first).
    let mut w = cain();
    w.fake.class.insert(w.mon, 527);
    set_portal(&mut w, [3, 103, 103, 1, 0]);
    w.fake.interacting = true;
    w.think_with(None, 0, false);
    assert_eq!(w.fake.log.first().unwrap(), "quest drehya update");
    assert_eq!(w.thinks(), [40]);
    // Dead → nothing scheduled.
    let mut w = cain();
    set_portal(&mut w, [3, 103, 103, 1, 0]);
    w.fake.anim.insert(w.mon, mode::DEAD);
    w.think_with(None, 0, false);
    assert!(w.thinks().is_empty());
    assert!(w.fake.modes().is_empty());
    // The interaction handler runs, its result ignored: a close player
    // greets (idle 20), then step 8 walks (no idle of its own).
    let mut w = cain();
    set_portal(&mut w, [3, 110, 100, 1, 0]);
    w.fake.npc_block = true;
    w.fake.pos.insert(w.player, (102, 100));
    w.fake.nearest = Some((w.player, true));
    w.think_with(None, 0, false);
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 60);
    assert_eq!(w.fake.modes(), ["mode 2 Point(110, 100)"]);
    assert_eq!(w.thinks(), [20]);
}

#[test]
fn npc_out_of_town_walks_to_the_portal_point() {
    // Farther than 1 and fewer than 6 tries: tries + 1, walk.
    let mut w = cain();
    set_portal(&mut w, [3, 110, 100, 1, 5]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w), [3, 110, 100, 1, 6]);
    assert_eq!(w.fake.modes(), ["mode 2 Point(110, 100)"]);
    assert!(w.thinks().is_empty());
    // drehyaiced with its gate set: idle 20, tries := 0.
    let mut w = cain();
    w.fake.class.insert(w.mon, 527);
    w.fake.drehya_wait = true;
    set_portal(&mut w, [3, 110, 100, 1, 3]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w), [3, 110, 100, 1, 0]);
    assert_eq!(w.thinks(), [20]);
    // The gate is drehyaiced's only: cain1 walks.
    let mut w = cain();
    w.fake.drehya_wait = true;
    set_portal(&mut w, [3, 110, 100, 1, 3]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w), [3, 110, 100, 1, 4]);
    assert_eq!(w.fake.modes(), ["mode 2 Point(110, 100)"]);
    assert!(!w.fake.log.iter().any(|l| l.starts_with("quest drehya")));
    // Tries used up, phase 1: spawn outside; success → phase 2, idle 20.
    let mut w = cain();
    w.fake.portal_spawn_ok = true;
    set_portal(&mut w, [3, 110, 100, 1, 6]);
    w.think_with(None, 0, false);
    assert!(w
        .fake
        .log
        .contains(&"quest outside portal Cain".to_string()));
    assert_eq!(portal_cmd(&w), [3, 110, 100, 2, 6]);
    assert_eq!(w.thinks(), [20]);
    // Failure → phase 1, tries 1, idle 20.
    let mut w = cain();
    set_portal(&mut w, [3, 101, 100, 1, 0]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w), [3, 101, 100, 1, 1]);
    assert_eq!(w.thinks(), [20]);
    // Phase 0 near the point: idle 20 only.
    let mut w = cain();
    set_portal(&mut w, [3, 101, 100, 0, 0]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w), [3, 101, 100, 0, 0]);
    assert!(!w.fake.log.iter().any(|l| l.starts_with("quest outside")));
    assert_eq!(w.thinks(), [20]);
}

#[test]
fn npc_out_of_town_goes_through_the_portal() {
    // Phase ≥ 2: phase + 1; coordinates found and not there → walk.
    let mut w = cain();
    w.fake.portal = Some((120, 100));
    set_portal(&mut w, [3, 103, 103, 2, 0]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w)[3], 3);
    assert_eq!(w.fake.modes(), ["mode 2 Point(120, 100)"]);
    // At the portal (path distance 0) → leave.
    let mut w = cain();
    w.fake.portal = Some((100, 100));
    set_portal(&mut w, [3, 103, 103, 2, 0]);
    w.think_with(None, 0, false);
    assert!(w
        .fake
        .modes()
        .contains(&"mode 12 Point(100, 100)".to_string()));
    assert!(w.fake.log.contains(&"quest town portal Cain".to_string()));
    // Phase reaches 8 → leave even when away from it.
    let mut w = cain();
    w.fake.portal = Some((120, 100));
    set_portal(&mut w, [3, 103, 103, 7, 0]);
    w.think_with(None, 0, false);
    assert_eq!(portal_cmd(&w)[3], 8);
    assert!(w
        .fake
        .modes()
        .contains(&"mode 12 Point(100, 100)".to_string()));
    // No coordinates → idle 20.
    let mut w = cain();
    set_portal(&mut w, [3, 103, 103, 2, 0]);
    w.think_with(None, 0, false);
    assert_eq!(w.thinks(), [20]);
    assert!(w.fake.modes().is_empty());
}

#[test]
fn command_search_and_create() {
    let mut w = cain();
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().commands = vec![
        AiCommand {
            params: [4, 1, 0, 0, 0],
        },
        AiCommand {
            params: [10, 2, 0, 0, 0],
        },
        AiCommand {
            params: [4, 3, 0, 0, 0],
        },
    ];
    // From current's next round to current: with current 0 the type-4
    // search finds index 2 first; `set` makes it current.
    let r = w.with(|_, cx| find_command(cx, mon, 4, false));
    assert_eq!(r, Some(2));
    let r = w.with(|_, cx| find_command(cx, mon, 4, true));
    assert_eq!(r, Some(2));
    assert_eq!(w.store.control(mon).unwrap().cur, 2);
    // From current 2: index 0 is next.
    let r = w.with(|_, cx| find_command(cx, mon, 4, false));
    assert_eq!(r, Some(0));
    // The current one is searched last.
    let r = w.with(|_, cx| find_command(cx, mon, 10, false));
    assert_eq!(r, Some(1));
    // Absent: none; creates (type, 0, 0, 0, 0).
    assert_eq!(w.with(|_, cx| find_command(cx, mon, 7, false)), None);
    let r = w.with(|_, cx| get_or_create_command(cx, mon, 7, false));
    let i = r.unwrap();
    assert_eq!(
        w.store.control(mon).unwrap().commands[i].params,
        [7, 0, 0, 0, 0]
    );
    assert_eq!(w.store.control(mon).unwrap().commands.len(), 4);
    // Found: no new command.
    let r = w.with(|_, cx| get_or_create_command(cx, mon, 10, false));
    assert!(r.is_some());
    assert_eq!(w.store.control(mon).unwrap().commands.len(), 4);
}

#[test]
fn good_npc_ranged_without_a_room_counts_as_out_of_town() {
    // The town test of no room returns 0: a unit with no room takes the
    // out-of-town step 2 even when every room is a town.
    let mut w = ranged_world();
    w.fake.town.insert(w.room);
    let u = w.game.spawn_unit(UnitType::Monster, None, false).unwrap();
    w.fake.pos.insert(u, (100, 100));
    w.fake.seeds.insert(u, Seed::init_low(4_014_346_870)); // first draw 0
    w.store.entry(u).control = Some(AiControl::default());
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 10));
    let p = TickParam {
        target: None,
        distance: 0,
        combat: false,
        class: 0,
        class2: 0,
    };
    w.with(|g, cx| run_function(g, cx, AI_TABLE[60].think, u, &p));
    // S at E < 20 and roll(100) = 0 < 30 → A1 at S (the in-town path
    // would not have looked for S).
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, s)]);
    // The same unit inside the town room: step 3 only (draw 51 → idle
    // 10), the secondary target is not even looked at.
    let mut w = ranged_world();
    w.fake.town.insert(w.room);
    let s = w.add_unit(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 10));
    w.seed(1);
    w.think_with(None, 0, false);
    assert!(w.fake.modes().is_empty(), "in town: no secondary search");
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn body_conventions_p_idle_and_attack_requests() {
    use super::super::bodies::{a1, a2};
    // P(aipN) is one unit-seed step tested `lo' % 100 < N` (strict).
    let mut w = ranged_world();
    let mon = w.mon;
    for &lo in &SEEDS {
        let mut s = Seed::init_low(lo);
        let draw = (s.step() % 100) as i32;
        for (n, want) in [(draw, false), (draw + 1, true)] {
            w.seed(lo);
            let got = w.with(|_, cx| cx.chance(mon, n));
            assert_eq!(got, want, "seed {lo} N {n}");
            assert_eq!(steps_since(&w, lo), 1);
        }
    }
    // A1 / A2 are mode requests 4 / 5 at T; "idle N" schedules a think N
    // frames out and returns the unit to neutral.
    let p = w.player;
    w.fake.log.clear();
    w.with(|g, cx| a1(g, cx, mon, Some(p)));
    w.with(|g, cx| a2(g, cx, mon, Some(p)));
    assert_eq!(
        w.fake.modes(),
        [unit_mode(mode::ATTACK1, p), unit_mode(mode::ATTACK2, p)]
    );
    let mut w = ranged_world();
    w.game.frame = 40;
    let mon = w.mon;
    w.with(|g, cx| idle(g, cx, mon, 7));
    assert_eq!(w.thinks(), [47]);
    // A body that does not reach a test draws nothing: GoodNpcRanged
    // idles 5 in a non-neutral mode before any draw.
    let mut w = ranged_world();
    w.fake.anim.insert(w.mon, mode::WALK);
    w.seed(1);
    w.think_with(None, 0, false);
    assert_eq!(steps_since(&w, 1), 0);
}

#[test]
fn quill_rat_attack_two_is_the_quill() {
    // QuillRat step 3: AI state 3 / 19 → A2 at T (mode 5, the `spike1`
    // missile mode), no draw.
    let mut w = World::new(monstats(14, [10, 35, 0, 2, 0], 15));
    w.fake.ai_state = 3;
    w.seed(1);
    w.think_with(Some(w.player), 20, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK2, w.player)]);
    assert_eq!(steps_since(&w, 1), 0);
}

// ---- SpecialState06 ---------------------------------------------

fn state6(w: &mut World, combat: bool) {
    let p = TickParam {
        target: None,
        distance: 0,
        combat,
        class: 0,
        class2: 0,
    };
    let mon = w.mon;
    w.with(|g, cx| run_function(g, cx, 0x005E_7C10, mon, &p));
}

#[test]
fn special_state_06_not_neutral_idles_5() {
    let mut w = ranged_world();
    w.fake.anim.insert(w.mon, mode::WALK);
    w.seed(4_014_346_870);
    state6(&mut w, false);
    assert_eq!(w.fake.modes(), [unit_mode(mode::NEUTRAL, w.mon)]);
    assert_eq!(w.thinks(), [5]);
    assert_eq!(steps_since(&w, 4_014_346_870), 0);
}

#[test]
fn special_state_06_town_or_no_target_takes_the_wander_draw() {
    // Draws `lo' % 100` 51, 87, 53, 0: only the last one is below 20.
    for (i, s) in SEEDS.iter().enumerate() {
        let mut w = ranged_world();
        w.fake.town.insert(w.room);
        w.seed(*s);
        state6(&mut w, false);
        if i == 3 {
            assert_eq!(w.fake.modes().len(), 1, "wander");
        } else {
            assert!(w.fake.modes().is_empty(), "seed {s}");
            assert_eq!(w.thinks(), [10]);
            assert_eq!(steps_since(&w, *s), 1, "seed {s}");
        }
    }
}

#[test]
fn special_state_06_acts_on_the_main_search_target() {
    // Out of town with a player in range: no contact → roll < 30 walks
    // to it with flags 7, else idles 10; contact → roll < 80 is A1.
    let rolls: [(bool, [bool; 4]); 2] = [
        (false, [false, false, false, true]),
        (true, [true, false, true, true]),
    ];
    for (combat, acts) in rolls {
        for (s, acts) in SEEDS.iter().zip(acts) {
            let mut w = ranged_world();
            w.fake.nodes = vec![vec![w.player]];
            w.fake.pos.insert(w.player, (105, 100));
            if combat {
                w.fake.melee.insert(w.player);
            }
            w.seed(*s);
            state6(&mut w, combat);
            assert_eq!(acts, !w.fake.modes().is_empty(), "{combat} seed {s}");
            if acts && combat {
                assert_eq!(w.fake.modes(), [unit_mode(mode::ATTACK1, w.player)]);
            }
            if !acts {
                assert_eq!(w.thinks(), [10]);
            }
            assert_eq!(steps_since(&w, *s), 1, "{combat} seed {s}");
        }
    }
}
