// (one test
// per rule group; fakes from the parent test module)
use super::*;

/// The first unit-seed low word whose first `k` raw draws satisfy `pred`.
fn seed_where(k: usize, pred: impl Fn(&[u32]) -> bool) -> u32 {
    (1..1_000_000u32)
        .find(|&lo| {
            let mut s = Seed::init_low(lo);
            let v: Vec<u32> = (0..k).map(|_| s.step()).collect();
            pred(&v)
        })
        .expect("a seed")
}

fn pc(v: u32) -> u32 {
    v % 100
}

/// Steps the monster's seed took since it was set to `init_low(lo)`.
fn draws(w: &World, lo: u32) -> usize {
    let now = w.fake.seeds[&w.mon];
    let mut s = Seed::init_low(lo);
    for n in 0..64 {
        if s == now {
            return n;
        }
        s.step();
    }
    panic!("seed not reached");
}

/// The wander point and its draw count after `skip` draws of `lo`.
fn wander_after(lo: u32, skip: usize, center: (i32, i32), n: i32) -> ((i32, i32), usize) {
    let mut s = Seed::init_low(lo);
    for _ in 0..skip {
        s.step();
    }
    let start = s;
    let pt = wander_point(&mut s, center, n);
    let mut t = start;
    let mut k = 0;
    while t != s {
        t.step();
        k += 1;
    }
    (pt, k)
}

fn at_unit(m: u8, u: UnitId) -> String {
    format!("mode {m} Unit({u:?})")
}

fn param(target: Option<UnitId>, distance: i32, combat: bool) -> TickParam {
    TickParam {
        target,
        distance,
        combat,
        class: 0,
        class2: 0,
    }
}

fn cmd(params: [i32; 5]) -> AiCommand {
    AiCommand { params }
}

impl World {
    fn think_now(&mut self) {
        let mon = self.mon;
        self.with(|g, cx| think(g, cx, mon));
    }

    /// Runs the current AI function with an explicit record.
    fn run_p(&mut self, p: TickParam) {
        let mon = self.mon;
        self.with(|g, cx| {
            let f = cx.store.control(mon).unwrap().function;
            run_function(g, cx, f, mon, &p)
        });
    }

    fn spawn(&mut self, ty: UnitType, at: (i32, i32)) -> UnitId {
        let u = self.game.spawn_unit(ty, Some(self.room), false).unwrap();
        self.fake.pos.insert(u, at);
        u
    }

    fn guid(&self, u: UnitId) -> u32 {
        self.game.lists.unit(u).unwrap().guid
    }

    fn logged(&self, s: &str) -> bool {
        self.fake.log.iter().any(|l| l == s)
    }

    fn control(&mut self) -> &mut AiControl {
        self.store.control_mut(self.mon).unwrap()
    }

    fn velocity(&self) -> VelocityRequest {
        self.store.get(self.mon).unwrap().velocity
    }
}

// -------------------------------------------------------------------

#[test]
fn think_rhythm_table() {
    // Target mode 1, no target, nearest player at d.
    for (d, want) in [(Some(40), 25), (Some(30), 20), (Some(20), 10), (None, 25)] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.game.frame = 100;
        w.monstats[0].aidist = 1; // nobody becomes a target
        if let Some(d) = d {
            w.fake.pos.insert(w.player, (100 + d, 100));
            w.fake.nodes = vec![vec![w.player]];
        }
        w.think_now();
        assert_eq!(w.thinks(), [100 + want], "d {d:?}");
    }
    // Target modes 4 (SandMaggot) and 5 (FrogDemon), no target: 20, mode
    // unchanged. Mode 4 stops the think; mode 5 goes on to the body with
    // T = 0, so its finder is checked alone.
    for ai in [15, 52] {
        let mut w = World::new(monstats(ai, [0; 5], 15));
        w.game.frame = 100;
        let mon = w.mon;
        w.fake.anim.insert(mon, mode::WALK);
        if ai == 15 {
            w.think_now();
        } else {
            let mut p = TickParam {
                target: None,
                distance: 0,
                combat: false,
                class: 0,
                class2: 0,
            };
            assert!(!w.with(|g, cx| precheck_b(g, cx, mon, &mut p)));
            assert_eq!(p.target, None);
        }
        assert_eq!(w.thinks(), [120], "ai {ai}");
        assert!(w.fake.modes().is_empty());
        assert_eq!(w.fake.anim_mode(mon), mode::WALK);
    }
    // "idle N" → N.
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.with(|g, cx| idle(g, cx, mon, 7));
    assert_eq!(w.thinks(), [107]);
    // An attack start schedules nothing; the mode end asks for neutral,
    // whose start adds aidel (Normal 15).
    let mut w = World::new(monstats(3, [30, 10, 0, 20, 0], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.seed(1);
    w.run(true, 1);
    assert_eq!(last_mode(&w), at_unit(mode::ATTACK2, w.player));
    assert!(w.thinks().is_empty());
    // The attack's path target unit is the player: the mode-end request
    // targets it.
    w.fake.path_target = Some(w.player);
    w.with(|g, cx| mode_end(g, cx, mon, mode::ATTACK2));
    assert_eq!(last_mode(&w), at_unit(mode::NEUTRAL, w.player));
    w.with(|g, cx| neutral_mode_start(g, cx, mon));
    assert_eq!(w.thinks(), [115]);
    // A walk end thinks inline (Idle: +200 from this frame); AI Idle →
    // 200.
    let mut w = World::new(monstats(1, [0; 5], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.fake.anim.insert(mon, mode::WALK);
    w.with(|g, cx| mode_end(g, cx, mon, mode::WALK));
    assert_eq!(w.thinks(), [300]);
    // A failed move start without flag 4 leaves the neutral start's aidel
    // think; with flag 4 it is deleted.
    for (flags, want) in [(0, vec![115]), (4, vec![])] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.game.frame = 100;
        w.fake.walk_fails = true;
        w.fake.fail_think = Some(15);
        let (mon, pl) = (w.mon, w.player);
        w.with(|g, cx| walk_to(g, cx, mon, Some(pl), flags));
        assert_eq!(w.thinks(), want, "flags {flags}");
    }
    // A player enters the room of a neutral monster: 2.
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 100;
    let room = w.room;
    w.with(|g, cx| client_entered_room(g, cx, room));
    assert_eq!(w.thinks(), [102]);
}

// ---- dispatch ------------------------------------------------------

#[test]
fn tick_record_initial_values_and_missing_rows() {
    // AI 0 (target mode 0) running the Zombie body: combat 0 (no A1/A2)
    // and distance 0 < aip2 = 1 (one P(aip1) draw, 51 ≥ 0), then wander 3
    // from the second draw: (97, 98) as in the D = 5 vector.
    let mut w = World::new(monstats(0, [0, 1, 0, 0, 0], 15));
    w.control().function = 0x005E_FE20;
    w.seed(1);
    w.think_now();
    assert_eq!(last_mode(&w), walk_point(97, 98));
    // The Skeleton body walking to T: target 0.
    let mut w = World::new(monstats(0, [100, 0, 0, 0, 0], 15));
    w.control().function = 0x005E_FCF0;
    w.think_now();
    assert_eq!(last_mode(&w), "mode 2 Point(0, 0)");
    // A missing monstats or monstats2 record: nothing runs (not even the
    // stun precheck).
    for missing2 in [false, true] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        let mon = w.mon;
        w.fake.states.insert((mon, state::STUNNED));
        if missing2 {
            w.monstats[0].monstatsex = 1;
        } else {
            w.fake.class.insert(mon, 1);
        }
        w.think_now();
        assert!(w.thinks().is_empty());
        assert!(w.fake.log.is_empty());
        assert!(w.store.unhandled.is_empty());
    }
}

// Is not claimed: its fatal assert on a bad code pointer has no
// D2RUST counterpart (an unknown address logs a stub call).
#[test]
fn dispatch_order_and_stops() {
    let row = || monstats(3, [30, 10, 0, 20, 0], 15);
    let flags = |w: &World| w.store.control(w.mon).unwrap().flags;
    // 1. Precheck A stops: no search (flag 0x08 stays clear), no AI draw.
    let mut w = World::new(row());
    w.fake.nodes = vec![vec![w.player]];
    w.fake.melee.insert(w.player);
    w.fake.states.insert((w.mon, state::STUNNED));
    w.seed(1);
    w.think_now();
    assert_eq!(w.thinks(), [3]);
    assert!(w.fake.modes().is_empty());
    assert_eq!(flags(&w) & flag::TARGET_SEEN, 0);
    assert_eq!(draws(&w, 1), 0);
    // 2. Precheck B stops (mode 1, no target): the AI does not run.
    let mut w = World::new(row());
    w.seed(1);
    w.think_now();
    assert_eq!(w.thinks(), [25]);
    assert!(w.fake.modes().is_empty());
    assert_eq!(draws(&w, 1), 0);
    // 3. Precheck C stops (boss sound): the AI does not run.
    let mut w = World::new(row());
    w.fake.unique = true;
    w.fake.nodes = vec![vec![w.player]];
    w.fake.melee.insert(w.player);
    w.seed(1);
    w.think_now();
    assert_eq!(w.thinks(), [20]);
    assert!(w.fake.modes().is_empty());
    assert_eq!(draws(&w, 1), 0);
    // 4. All pass: the control's function runs with the record (Zombie,
    // C, 51 ≥ 20 → A2).
    let mut w = World::new(row());
    w.fake.nodes = vec![vec![w.player]];
    w.fake.melee.insert(w.player);
    w.seed(1);
    w.think_now();
    assert_eq!(last_mode(&w), at_unit(mode::ATTACK2, w.player));
    // The control's current function, not the table's think.
    let mut w = World::new(row());
    w.fake.nodes = vec![vec![w.player]];
    w.control().function = table::IDLE_FN;
    w.think_now();
    assert_eq!(w.thinks(), [200]);
}

#[test]
fn precheck_a_doors() {
    for (opendoors, blocked, door, stops) in [
        (true, true, Some(true), true),
        (true, true, Some(false), false),
        (true, true, None, false),
        (false, true, Some(true), false),
        (true, false, Some(true), false),
    ] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.monstats[0].opendoors = opendoors;
        w.fake.blocked_path = blocked;
        let d = w.spawn(UnitType::Object, (101, 100));
        w.fake.door = door.map(|ok| (d, ok));
        let mon = w.mon;
        let stop = w.with(|g, cx| precheck_a(g, cx, mon, &param(None, 0, false)));
        assert_eq!(stop, stops);
        assert_eq!(w.logged(&format!("door {d:?}")), stops);
        assert_eq!(w.thinks(), if stops { vec![5] } else { vec![] });
    }
}

#[test]
fn precheck_a_leash() {
    let leash = |ty: UnitType, at: (i32, i32)| {
        let mut w = World::new(monstats(3, [0; 5], 15));
        let o = w.spawn(ty, at);
        let guid = w.guid(o);
        w.control().owner = Some(UnitRef { ty, guid });
        (w, o)
    };
    let pa = |w: &mut World| {
        let mon = w.mon;
        w.with(|g, cx| precheck_a(g, cx, mon, &param(None, 0, false)))
    };
    // d ≤ 1, owner a player: walk away 19 with think delete.
    let (mut w, _) = leash(UnitType::Player, (101, 100));
    w.game.schedule_event(w.mon, 2, 9, None, 0, 0).unwrap();
    assert!(pa(&mut w));
    assert_eq!(last_mode(&w), walk_point(81, 100));
    assert_eq!(w.velocity().steps, 19);
    assert_eq!(w.thinks(), [9], "a started escape deletes nothing");
    // ... the escape fails: wander near the owner 19.
    let (mut w, _) = leash(UnitType::Player, (101, 100));
    w.fake.walk_fails = true;
    w.seed(1);
    assert!(pa(&mut w));
    let ((x, y), _) = wander_after(1, 0, (101, 100), 19);
    assert_eq!(last_mode(&w), walk_point(x, y));
    // d ≤ 1 but the owner is a monster: continue.
    let (mut w, _) = leash(UnitType::Monster, (101, 100));
    assert!(!pa(&mut w));
    assert!(w.fake.modes().is_empty());
    // d > 20: velocity (method 7, speed 0, steps 40), wander near 19.
    let (mut w, _) = leash(UnitType::Monster, (130, 100));
    w.seed(1);
    assert!(pa(&mut w));
    let v = w.velocity();
    assert_eq!((v.method, v.speed, v.steps), (7, 0, 40));
    let ((x, y), _) = wander_after(1, 0, (130, 100), 19);
    assert_eq!(last_mode(&w), walk_point(x, y));
    // d = 20 (full size: 21 − 1): continue, owner kept.
    let (mut w, _) = leash(UnitType::Monster, (121, 100));
    assert!(!pa(&mut w));
    assert!(w.fake.modes().is_empty());
    assert!(w.store.control(w.mon).unwrap().owner.is_some());
    // Class cannot walk / owner dead / owner gone: owner := −1, continue.
    for case in 0..3 {
        let (mut w, o) = leash(UnitType::Monster, (130, 100));
        match case {
            0 => w.fake.can_walk_off = true,
            1 => {
                w.fake.dead.insert(o);
            }
            _ => w.game.remove_unit(o).unwrap(),
        }
        assert!(!pa(&mut w), "case {case}");
        assert!(w.fake.modes().is_empty());
        assert_eq!(w.store.control(w.mon).unwrap().owner, None);
    }
}

#[test]
fn precheck_a_draws_nothing_of_its_own() {
    // Stun, door, leash continue, leash push: no draws.
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.fake.states.insert((w.mon, state::STUNNED));
    w.seed(1);
    let mon = w.mon;
    assert!(w.with(|g, cx| precheck_a(g, cx, mon, &param(None, 0, false))));
    assert_eq!(draws(&w, 1), 0);
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.monstats[0].opendoors = true;
    w.fake.blocked_path = true;
    let d = w.spawn(UnitType::Object, (101, 100));
    w.fake.door = Some((d, true));
    w.seed(1);
    let mon = w.mon;
    assert!(w.with(|g, cx| precheck_a(g, cx, mon, &param(None, 0, false))));
    assert_eq!(draws(&w, 1), 0);
    for (ty, at, stops) in [
        (UnitType::Monster, (110, 100), false),
        (UnitType::Player, (100, 101), true),
    ] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        let o = w.spawn(ty, at);
        let guid = w.guid(o);
        w.control().owner = Some(UnitRef { ty, guid });
        w.seed(1);
        let mon = w.mon;
        let stop = w.with(|g, cx| precheck_a(g, cx, mon, &param(None, 0, false)));
        assert_eq!(stop, stops);
        assert_eq!(draws(&w, 1), 0);
    }
    // The wander helper's draws only.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let o = w.spawn(UnitType::Monster, (130, 100));
    let guid = w.guid(o);
    w.control().owner = Some(UnitRef {
        ty: UnitType::Monster,
        guid,
    });
    w.seed(1);
    let mon = w.mon;
    w.with(|g, cx| precheck_a(g, cx, mon, &param(None, 0, false)));
    let (_, k) = wander_after(1, 0, (130, 100), 19);
    assert_eq!(draws(&w, 1), k);
}

#[test]
fn mode_1_no_target_wanders() {
    let ((x, y), _) = wander_after(1, 0, (100, 100), 5);
    for (ai_state, collides, walk_off, wanders) in [
        (3, false, false, true),
        (19, false, false, true),
        (3, false, true, false),
        (0, true, false, true),
        (0, true, true, false),
        (0, false, false, false),
    ] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.fake.ai_state = ai_state;
        w.fake.collides = collides;
        w.fake.can_walk_off = walk_off;
        w.seed(1);
        w.think_now();
        if wanders {
            assert_eq!(last_mode(&w), walk_point(x, y));
            assert!(w.thinks().is_empty());
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [25]);
        }
    }
}

#[test]
fn mode_4_no_target_wanders_only_on_collision_with_a_walk_mode() {
    // Collision (mask 0x40) and the class can walk → wander
    // 5; collision without a walk mode, or no collision → idle 20, no
    // mode change, no draw.
    let ((x, y), _) = wander_after(1, 0, (100, 100), 5);
    for (collides, walk_off, wanders) in [
        (true, false, true),
        (true, true, false),
        (false, false, false),
    ] {
        let mut w = World::new(monstats(15, [0; 5], 15));
        w.game.frame = 100;
        w.fake.collides = collides;
        w.fake.can_walk_off = walk_off;
        w.seed(1);
        w.think_now();
        let case = format!("{collides} {walk_off}");
        if wanders {
            assert_eq!(last_mode(&w), walk_point(x, y), "{case}");
            assert!(w.thinks().is_empty(), "{case}");
        } else {
            assert!(w.fake.modes().is_empty(), "{case}");
            assert_eq!(w.thinks(), [120], "{case}");
            assert_eq!(draws(&w, 1), 0, "{case}");
        }
    }
}

#[test]
fn precheck_c_needs_a_target() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.fake.unique = true;
    w.control().flags |= flag::MAY_TELEPORT;
    w.monstats[0].ismelee = true;
    w.levels[0].monspcwalk = 1;
    w.fake.reach_fails = true;
    w.seed(4_014_346_870); // first draw 0: the teleport gate would pass
    let mon = w.mon;
    let mut p = param(None, 5, false);
    assert!(!w.with(|g, cx| precheck_c(g, cx, mon, &mut p)));
    assert!(w.fake.log.is_empty());
    assert_eq!(draws(&w, 4_014_346_870), 0);
    assert!(w.thinks().is_empty());
}

#[test]
fn precheck_c_teleport() {
    struct Case {
        lo: u32,
        flag: bool,
        dead: bool,
        life: i32,
        melee: bool,
        base: u16,
        d: i32,
        spot: Option<bool>, // Some(in town)
        heal_block: bool,
        stops: bool,
        draws: usize,
        life_added: bool,
    }
    let pass1 = seed_where(1, |v| pc(v[0]) < 40);
    let fail1 = seed_where(1, |v| pc(v[0]) >= 40);
    let pass2 = seed_where(2, |v| pc(v[0]) < 40 && pc(v[1]) < 15);
    let pass3 = seed_where(3, |v| pc(v[0]) < 40 && pc(v[1]) < 15 && pc(v[2]) < 25);
    let fail3 = seed_where(3, |v| pc(v[0]) < 40 && pc(v[1]) < 15 && pc(v[2]) >= 25);
    let base = Case {
        lo: pass2,
        flag: true,
        dead: false,
        life: 100,
        melee: false,
        base: 0,
        d: 5,
        spot: Some(false),
        heal_block: false,
        stops: true,
        draws: 2,
        life_added: false,
    };
    let cases = [
        // Not melee, D < 10: move.
        Case { ..base },
        // Flag 0x20 clear / dead: nothing drawn.
        Case {
            flag: false,
            stops: false,
            draws: 0,
            ..base
        },
        Case {
            dead: true,
            stops: false,
            draws: 0,
            ..base
        },
        // lo' % 100 ≥ 40: continue after one draw.
        Case {
            lo: fail1,
            stops: false,
            draws: 1,
            ..base
        },
        // Melee (isMelee or base 10) and not hurt: no second draw.
        Case {
            lo: pass1,
            melee: true,
            stops: false,
            draws: 1,
            ..base
        },
        Case {
            lo: pass1,
            base: 10,
            stops: false,
            draws: 1,
            ..base
        },
        // Not melee but D = 10: no second draw.
        Case {
            lo: pass1,
            d: 10,
            stops: false,
            draws: 1,
            ..base
        },
        // Hurt: heal level × 256 after a third draw.
        Case {
            lo: pass3,
            life: 20,
            melee: true,
            draws: 3,
            life_added: true,
            ..base
        },
        Case {
            lo: pass3,
            life: 20,
            melee: true,
            heal_block: true,
            draws: 3,
            ..base
        },
        Case {
            lo: fail3,
            life: 20,
            melee: true,
            draws: 3,
            ..base
        },
        // Edge case 9: no spot → both draws made, continue.
        Case {
            lo: pass3,
            life: 20,
            spot: None,
            stops: false,
            draws: 2,
            ..base
        },
        // The spot's room is in town: continue.
        Case {
            spot: Some(true),
            stops: false,
            ..base
        },
    ];
    for (i, c) in cases.iter().enumerate() {
        let mut w = World::new(monstats(3, [0; 5], 15));
        if c.flag {
            w.control().flags |= flag::MAY_TELEPORT;
        }
        let mon = w.mon;
        if c.dead {
            w.fake.dead.insert(mon);
        }
        if c.heal_block {
            w.fake.states.insert((mon, state::PREVENTHEAL));
        }
        w.fake.life = c.life;
        w.monstats[0].ismelee = c.melee;
        w.monstats[0].baseid = c.base;
        let spot_room = w.game.lists.create_room(0).unwrap();
        if c.spot == Some(true) {
            w.fake.town.insert(spot_room);
        }
        w.fake.spot = c.spot.map(|_| (120, 130, spot_room));
        w.seed(c.lo);
        let mut p = param(Some(w.player), c.d, false);
        let stop = w.with(|g, cx| precheck_c(g, cx, mon, &mut p));
        assert_eq!(stop, c.stops, "case {i}");
        assert_eq!(draws(&w, c.lo), c.draws, "case {i}");
        assert_eq!(w.logged("life 1280"), c.life_added, "case {i}");
        assert_eq!(w.logged("mode 4 Point(120, 130)"), c.stops, "case {i}");
        assert_eq!(w.logged("skill 184"), c.stops, "case {i}");
    }
}

#[test]
fn precheck_c_special_walk() {
    let setup = || {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.monstats[0].ismelee = true;
        w.levels[0].monspcwalk = 5;
        w.fake.reach_fails = true;
        w.seed(1);
        w
    };
    let pc_run = |w: &mut World, d: i32| {
        let mon = w.mon;
        let mut p = param(Some(w.player), d, false);
        let stop = w.with(|g, cx| precheck_c(g, cx, mon, &mut p));
        (stop, p)
    };
    // No alternative: wander 4, stop.
    let mut w = setup();
    let (stop, _) = pc_run(&mut w, 8);
    assert!(stop);
    assert!(w.logged("scan 11"));
    let ((x, y), _) = wander_after(1, 0, (100, 100), 4);
    assert_eq!(last_mode(&w), walk_point(x, y));
    // An alternative replaces target and distance; continue.
    let mut w = setup();
    let alt = w.spawn(UnitType::Monster, (103, 100));
    w.fake.special_walk = Some((alt, 3));
    let (stop, p) = pc_run(&mut w, 8);
    assert!(!stop);
    assert_eq!((p.target, p.distance), (Some(alt), 3));
    // Each condition false: no scan.
    for case in 0..6 {
        let mut w = setup();
        let mut d = 8;
        match case {
            0 => d = 5, // MonSpcWalk not smaller than D
            1 => w.fake.reach_fails = false,
            2 => w.monstats[0].ismelee = false,
            3 => w.levels[0].monspcwalk = 0,
            4 => w.fake.can_walk_off = true,
            _ => {
                // No room.
                let m = w.game.spawn_unit(UnitType::Monster, None, false).unwrap();
                w.mon = m;
                w.seed(1);
            }
        }
        let (stop, p) = pc_run(&mut w, d);
        assert!(!stop, "case {case}");
        assert!(!w.logged("scan 11"), "case {case}");
        assert_eq!(p.target, Some(w.player));
    }
}

// -------------------------------------------------------------------

#[test]
fn ai_control_record() {
    assert_eq!(
        [
            flag::TARGET_SEEN,
            flag::BOSS_SOUND_DONE,
            flag::MAY_TELEPORT,
            flag::FORCE_LOS
        ],
        [0x08, 0x10, 0x20, 0x40]
    );
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    // "AI state" is true for 3 and 19 only.
    for v in 0..64 {
        w.fake.ai_state = v;
        assert_eq!(w.with(|_, cx| cx.ai_state_set(mon)), v == 3 || v == 19);
    }
    // The special state selects the special-state table.
    w.control().special_state = 8;
    assert_eq!(w.with(|_, cx| cx.record(mon)), SPECIAL_TABLE[8]);
    // The minion owner (+0x2C/+0x30) is what returns.
    let leader = w.spawn(UnitType::Monster, (0, 0));
    let guid = w.guid(leader);
    w.control().minion_owner = Some(UnitRef {
        ty: UnitType::Monster,
        guid,
    });
    let got = w.with(|g, cx| minion_owner(g, cx, mon));
    assert_eq!(got, Some(leader));
}

#[test]
#[should_panic(expected = "state 54")]
fn install_asserts_on_state_54() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.fake.states.insert((mon, state::UNINTERRUPTABLE));
    w.with(|g, cx| install(g, cx, mon, 0));
}

#[test]
fn ai_parameters() {
    let mut row = monstats(3, [10, 0, 0, 0, 0], 15);
    row.aip1_n = 20;
    row.aip1_h = (-5i16) as u16;
    row.aip8 = 7;
    row.aip8_n = 8;
    row.aip8_h = 9;
    row.aidist_n = 12;
    row.aidel_n = 14;
    let mut w = World::new(row);
    let p = param(None, 0, false);
    for (d, aip1, aip8, aidist, aidel) in
        [(0, 10, 7, 35, 15), (1, 20, 8, 12, 15), (2, -5, 9, 35, 15)]
    {
        // Game type 0: aip and aidist by difficulty, aidel gated.
        let got = w.with(|_, cx| {
            cx.info.difficulty = d;
            (cx.aip(&p, 1), cx.aip(&p, 8), cx.aidist(0), cx.aidel(0))
        });
        assert_eq!(got, (aip1, aip8, aidist, aidel), "difficulty {d}");
    }
    // P(v) = lo' % 100 < v, signed, always one draw.
    let mon = w.mon;
    for (lo, v, want) in [
        (4_014_346_870, 0, false), // 0 < 0
        (4_014_346_870, 1, true),
        (4_014_346_870, -5, false), // edge case 3: negative never passes
        (12345, 100, true),         // 87
        (12345, 87, false),
        (12345, 88, true),
    ] {
        w.seed(lo);
        assert_eq!(w.with(|_, cx| cx.chance(mon, v)), want, "{lo} {v}");
        assert_eq!(draws(&w, lo), 1, "the draw happens either way");
    }
    // roll(100) gives the same value as lo' % 100.
    for lo in SEEDS {
        let (mut a, mut b) = (Seed::init_low(lo), Seed::init_low(lo));
        assert_eq!(a.roll(100), b.step() % 100);
    }
    // An aip passed as a byte (QuillRat wander, aip4 = 0x105 → 5).
    let mut w = World::new(monstats(14, [10, 35, 0, 0x105, 0], 15));
    w.seed(1);
    w.run(false, 10);
    let ((x, y), _) = wander_after(1, 0, (100, 100), 5);
    assert_eq!(last_mode(&w), walk_point(x, y));
}

// -------------------------------------------------------------------

#[test]
fn main_search_without_room() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.fake.nodes = vec![vec![w.player]];
    let m = w.game.spawn_unit(UnitType::Monster, None, false).unwrap();
    let s = w.with(|g, cx| main_search(g, cx, m));
    assert_eq!((s.target, s.distance, s.combat), (None, 0, false));
}

#[test]
fn main_search_line_of_sight_flag() {
    // The player's line is blocked: found only when T = 0.
    struct Case {
        flags: u16,
        no_los_draw: bool,
        vision: Option<u32>,
        t: bool,
    }
    let cases = [
        Case {
            flags: 0,
            no_los_draw: false,
            vision: None,
            t: false,
        },
        Case {
            flags: flag::FORCE_LOS,
            no_los_draw: false,
            vision: None,
            t: true,
        },
        Case {
            flags: 0,
            no_los_draw: true,
            vision: None,
            t: true,
        },
        Case {
            flags: flag::TARGET_SEEN,
            no_los_draw: true,
            vision: None,
            t: false,
        },
        Case {
            flags: 0,
            no_los_draw: true,
            vision: Some(5),
            t: false,
        },
        Case {
            flags: 0,
            no_los_draw: true,
            vision: Some(0),
            t: true,
        },
        Case {
            flags: flag::TARGET_SEEN,
            no_los_draw: true,
            vision: Some(0),
            t: false,
        },
    ];
    for (i, c) in cases.iter().enumerate() {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.fake.nodes = vec![vec![w.player]];
        w.fake.line_blocked.insert(w.player);
        w.fake.no_los_draw = c.no_los_draw;
        w.fake.vision = c.vision;
        w.control().flags = c.flags;
        let mon = w.mon;
        let s = w.with(|g, cx| main_search(g, cx, mon));
        assert_eq!(s.target.is_none(), c.t, "case {i}");
        // Flag 0x40 is consumed.
        assert_eq!(
            w.store.control(mon).unwrap().flags & flag::FORCE_LOS,
            0,
            "case {i}"
        );
    }
}

#[test]
fn main_search_forced_and_not_evil() {
    // Forced target first, whatever the nodes hold.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let other = w.spawn(UnitType::Monster, (140, 100));
    w.fake.nodes = vec![vec![w.player]];
    w.fake.forced = Some((other, 39));
    let mon = w.mon;
    let s = w.with(|g, cx| main_search(g, cx, mon));
    assert_eq!((s.target, s.distance), (Some(other), 39));
    // Not evil: the scan's result with T, not the nodes.
    for (align, no_los_draw) in [(1, false), (2, true)] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        let other = w.spawn(UnitType::Monster, (110, 100));
        w.fake.nodes = vec![vec![w.player]];
        w.fake.align = align;
        w.fake.no_los_draw = no_los_draw;
        w.fake.good = Some((other, 10));
        let mon = w.mon;
        let s = w.with(|g, cx| main_search(g, cx, mon));
        assert_eq!((s.target, s.distance), (Some(other), 10));
        assert!(w.logged(&format!("good {no_los_draw}")));
        w.fake.good = None;
        let s = w.with(|g, cx| main_search(g, cx, mon));
        assert_eq!(s.target, None);
    }
}

#[test]
fn main_search_ties_and_no_draws() {
    for first in [false, true] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        let other = w.spawn(UnitType::Player, (100, 105));
        w.fake.pos.insert(w.player, (105, 100));
        let (a, b) = if first {
            (other, w.player)
        } else {
            (w.player, other)
        };
        w.fake.nodes = vec![vec![a], vec![b]];
        w.seed(1);
        let mon = w.mon;
        let s = w.with(|g, cx| main_search(g, cx, mon));
        assert_eq!(s.target, Some(a), "ties keep the earlier node");
        assert_eq!(draws(&w, 1), 0);
    }
}

#[test]
fn main_search_dead_head_keeps_attached_units() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let pet = w.spawn(UnitType::Monster, (103, 100));
    w.fake.pos.insert(w.player, (101, 100));
    w.fake.dead.insert(w.player);
    w.fake.nodes = vec![vec![w.player, pet]];
    let mon = w.mon;
    let s = w.with(|g, cx| main_search(g, cx, mon));
    assert_eq!((s.target, s.distance), (Some(pet), 3));
}

// -------------------------------------------------------------------

#[test]
fn mode_requests() {
    let mut row = monstats(3, [0; 5], 15);
    row.skill1 = 7;
    let mut w = World::new(row);
    w.modes[0] = [8, 9, 14, 0, 0, 0, 0, 0];
    let (mon, pl) = (w.mon, w.player);
    let p = param(None, 0, false);
    assert_eq!(w.with(|_, cx| cx.skill(&p, 1)), (7, 8));
    assert_eq!(w.with(|_, cx| cx.skill(&p, 2)), (-1, 9));
    // UseSkill: skill, flag 0x40, path step 1, then the mode.
    w.with(|g, cx| use_skill(g, cx, mon, 8, 7, ModeTarget::Unit(pl)));
    let tail = w.fake.log[w.fake.log.len() - 4..].to_vec();
    assert_eq!(
        tail,
        ["skill 7", "skillflag", "steps 1", &at_unit(8, pl)].map(String::from)
    );
    assert!(w.thinks().is_empty());
    // Failed start: idle 10.
    w.fake.fail_modes.insert(8);
    w.with(|g, cx| use_skill(g, cx, mon, 8, 7, ModeTarget::Unit(pl)));
    assert_eq!(w.thinks(), [10]);
    // Mode ≥ 16: nothing.
    let n = w.fake.log.len();
    w.with(|g, cx| use_skill(g, cx, mon, 16, 7, ModeTarget::Unit(pl)));
    assert_eq!(w.fake.log.len(), n);
    // UseSequenceSkill: mode 14, skill, path step 1, no flag, no fallback.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let (mon, pl) = (w.mon, w.player);
    w.fake.fail_modes.insert(mode::SEQUENCE);
    w.with(|g, cx| use_sequence_skill(g, cx, mon, 5, ModeTarget::Unit(pl)));
    assert_eq!(
        w.fake.log,
        ["skill 5", "steps 1", &at_unit(14, pl)].map(String::from)
    );
    assert!(w.thinks().is_empty());
    // Skill id out of range: nothing after the skill set.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let (mon, pl) = (w.mon, w.player);
    w.with(|g, cx| use_sequence_skill(g, cx, mon, -1, ModeTarget::Unit(pl)));
    assert_eq!(w.fake.log, ["skill -1"]);
    // Mode with a target unit.
    w.with(|g, cx| mode_at(g, cx, mon, mode::ATTACK1, Some(pl)));
    assert_eq!(last_mode(&w), at_unit(4, pl));
}

// ---- per-AI --------------------------------------------------------

#[test]
fn zombie_never_schedules() {
    let row = || monstats(3, [30, 10, 0, 20, 0], 15);
    for (combat, d, level) in [(true, 1, 0), (false, 12, 0), (false, 5, 0), (false, 12, 17)] {
        for lo in SEEDS {
            let mut w = World::new(row());
            w.fake.level = level;
            w.seed(lo);
            w.run(combat, d);
            assert!(w.thinks().is_empty(), "{combat} {d} {level} {lo}");
            assert!(!w.fake.modes().is_empty());
        }
    }
    // The draw happens only when D < aip2: at D = 12 only the wander's.
    let mut w = World::new(row());
    w.seed(1);
    w.run(false, 12);
    assert_eq!(draws(&w, 1), wander_after(1, 0, (100, 100), 3).1);
    // A failed move: only the aidel think of the failed start.
    let mut w = World::new(row());
    w.fake.walk_fails = true;
    w.fake.fail_think = Some(15);
    w.seed(1);
    w.run(false, 12);
    assert_eq!(w.thinks(), [15]);
}

fn fallen_world() -> World {
    World::new(monstats(6, [30, 10, 50, 20, 0], 15))
}

#[test]
fn fallen_corpse_check() {
    let sound0 = seed_where(1, |v| v[0] % 20 == 0);
    let sound1 = seed_where(1, |v| v[0] % 20 != 0);
    for (lo, sound) in [(sound0, true), (sound1, false)] {
        let mut w = fallen_world();
        let room = w.room;
        w.game.lists.room_mut(room).unwrap().adjacent = vec![room];
        let corpse = w.spawn(UnitType::Monster, (105, 105));
        w.fake.anim.insert(corpse, mode::DEATH);
        w.fake
            .last_dead
            .insert(room, [None, Some(corpse), None, None]);
        w.control().commands = vec![cmd([1, 0, 0, 0, 0])];
        w.game.schedule_event(w.mon, 2, 30, None, 0, 0).unwrap();
        w.seed(lo);
        w.run(false, 20);
        // Escape from T (105, 100) by 12: (88, 100).
        assert_eq!(w.fake.modes(), [walk_point(88, 100)]);
        let v = w.velocity();
        assert_eq!((v.speed, v.steps), (50, 12));
        let c = w.store.control(w.mon).unwrap();
        assert_eq!(c.params[0], 1);
        assert!(c.commands.is_empty(), "current command freed");
        assert_eq!(w.logged("sound 17"), sound);
        assert_eq!(draws(&w, lo), 1);
        assert!(w.thinks().is_empty());
    }
    // The escape fails: the check stops (one escape for two corpses) and
    // the think goes on (5.3: 51 ≥ 30 → idle 10).
    let mut w = fallen_world();
    let room = w.room;
    w.game.lists.room_mut(room).unwrap().adjacent = vec![room];
    let c1 = w.spawn(UnitType::Monster, (105, 105));
    let c2 = w.spawn(UnitType::Monster, (95, 95));
    w.fake.anim.insert(c1, mode::DEATH);
    w.fake.anim.insert(c2, mode::DEATH);
    w.fake
        .last_dead
        .insert(room, [Some(c1), Some(c2), None, None]);
    w.fake.walk_fails = true;
    w.seed(1);
    w.run(false, 20);
    assert_eq!(w.fake.modes(), [walk_point(88, 100)]);
    assert_eq!(w.thinks(), [10]);
    assert_eq!(draws(&w, 1), 1);
    // Not a corpse: dead mode (12), or at distance ≥ 15.
    for (anim, at) in [(mode::DEAD, (105, 105)), (mode::DEATH, (115, 100))] {
        let mut w = fallen_world();
        let room = w.room;
        w.game.lists.room_mut(room).unwrap().adjacent = vec![room];
        let c = w.spawn(UnitType::Monster, at);
        w.fake.anim.insert(c, anim);
        w.fake.last_dead.insert(room, [Some(c), None, None, None]);
        w.seed(1);
        w.run(false, 20);
        assert!(w.fake.modes().is_empty());
        assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    }
}

#[test]
fn fallen_mode_and_command() {
    // 3. Not neutral: idle 10 (via neutral), no draws.
    let mut w = fallen_world();
    let mon = w.mon;
    w.fake.anim.insert(mon, mode::ATTACK1);
    w.seed(1);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::NEUTRAL, mon)]);
    assert_eq!(w.thinks(), [10]);
    assert_eq!(draws(&w, 1), 0);
    // 4. Command type ≠ 1: freed, idle 10.
    let mut w = fallen_world();
    w.control().commands = vec![cmd([2, 0, 0, 0, 0])];
    w.seed(1);
    w.run(true, 1);
    assert!(w.store.control(w.mon).unwrap().commands.is_empty());
    assert_eq!(w.thinks(), [10]);
    assert_eq!(draws(&w, 1), 0);
    // Type 1, C: P(aip3) fails → idle 5; else P(aip4) → A1 / A2.
    let a1 = seed_where(2, |v| pc(v[0]) < 50 && pc(v[1]) < 20);
    for (lo, want) in [(1, None), (4_014_346_870, Some(5)), (a1, Some(4))] {
        let mut w = fallen_world();
        w.control().commands = vec![cmd([1, 0, 0, 0, 0])];
        w.seed(lo);
        w.run(true, 1);
        match want {
            None => {
                assert!(w.fake.modes().is_empty());
                assert_eq!(w.thinks(), [5]);
            }
            Some(m) => assert_eq!(last_mode(&w), at_unit(m, w.player)),
        }
        assert_eq!(w.store.control(w.mon).unwrap().commands.len(), 1);
    }
    // Type 1, not C: walk to T (flags 0); a failed walk frees the command
    // and draws nothing.
    for fails in [false, true] {
        let mut w = fallen_world();
        w.control().commands = vec![cmd([1, 0, 0, 0, 0])];
        w.fake.walk_fails = fails;
        w.seed(1);
        w.run(false, 20);
        assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
        assert_eq!(
            w.store.control(w.mon).unwrap().commands.len(),
            !fails as usize
        );
        assert_eq!(draws(&w, 1), 0);
        assert_eq!(w.store.control(w.mon).unwrap().flags & flag::FORCE_LOS, 0);
    }
}

#[test]
fn fallen_without_command() {
    // 5.1: not C, AI state 3: walk to T, no draws.
    let mut w = fallen_world();
    w.fake.ai_state = 3;
    w.seed(1);
    w.run(false, 5);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
    assert_eq!(draws(&w, 1), 0);
    // 5.2: pack leader, D < 15, P(aip1): command 1 to the minions and to
    // itself, S2 at T.
    let leader = |w: &mut World| {
        let me = w.guid(w.mon);
        let m = w.spawn(UnitType::Monster, (0, 0));
        w.store.entry(m).control = Some(AiControl::default());
        let mg = w.guid(m);
        let c = w.control();
        c.minion_owner = Some(UnitRef {
            ty: UnitType::Monster,
            guid: me,
        });
        c.minions = vec![mg];
        m
    };
    let mut w = fallen_world();
    let m = leader(&mut w);
    w.seed(4_014_346_870); // 0 < 30
    w.run(false, 10);
    assert_eq!(w.fake.modes(), [at_unit(mode::SKILL2, w.player)]);
    let one = [cmd([1, 0, 0, 0, 0])];
    assert_eq!(w.store.control(m).unwrap().commands, one);
    assert_eq!(w.store.control(w.mon).unwrap().commands, one);
    // ... P(aip1) fails (51): 5.3, D ≤ aip2 → walk flags 7.
    let mut w = fallen_world();
    leader(&mut w);
    w.seed(1);
    w.run(false, 10);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
    assert_eq!(draws(&w, 1), 1);
    // Not the leader: no 5.2 draw; walk flags 7 (a failed walk takes the
    // flag-2 fallback and sets flag 0x40).
    let mut w = fallen_world();
    w.fake.walk_fails = true;
    w.seed(12345); // fallback draw 87 ≥ 70 → idle 10
    w.run(false, 10);
    assert_eq!(draws(&w, 12345), 1);
    assert_eq!(w.thinks(), [10]);
    assert_ne!(w.store.control(w.mon).unwrap().flags & flag::FORCE_LOS, 0);
    // 5.3: D > aip2: pct(30) → wander 3, else idle 10.
    let mut w = fallen_world();
    w.seed(1);
    w.run(false, 12);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [10]);
    let mut w = fallen_world();
    w.seed(4_014_346_870);
    w.run(false, 12);
    let ((x, y), _) = wander_after(4_014_346_870, 1, (100, 100), 3);
    assert_eq!(last_mode(&w), walk_point(x, y));
    // 5.4: C, param 0 = 0, P(aip3) fails: pct(30) → S2, else idle 10.
    let s2 = seed_where(2, |v| pc(v[0]) >= 50 && pc(v[1]) < 30);
    let mut w = fallen_world();
    w.seed(s2);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::SKILL2, w.player)]);
    // Param 0 = 1: no aip3 draw, param 0 := 0, P(aip4) (51) → A2.
    let mut w = fallen_world();
    w.control().params[0] = 1;
    w.seed(1);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK2, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    assert_eq!(draws(&w, 1), 1);
}

#[test]
fn fallen_failed_walk_leaves_only_aidel() {
    // Steps 4 and 5.1: the fallen deleted its thinks; the failed start
    // leaves its aidel think only.
    for step4 in [true, false] {
        let mut w = fallen_world();
        w.game.frame = 100;
        w.game.schedule_event(w.mon, 2, 103, None, 0, 0).unwrap();
        w.fake.walk_fails = true;
        w.fake.fail_think = Some(15);
        if step4 {
            w.control().commands = vec![cmd([1, 0, 0, 0, 0])];
        } else {
            w.fake.ai_state = 19;
        }
        w.run(false, 20);
        assert_eq!(w.thinks(), [115], "step 4: {step4}");
    }
    // Edge case 5: an attack ends the think with nothing scheduled; a
    // room entry then gives +2.
    let mut w = fallen_world();
    w.game.frame = 100;
    w.game.schedule_event(w.mon, 2, 103, None, 0, 0).unwrap();
    w.seed(4_014_346_870);
    w.run(true, 1);
    assert_eq!(last_mode(&w), at_unit(mode::ATTACK2, w.player));
    assert!(w.thinks().is_empty());
    w.fake.anim.insert(w.mon, mode::NEUTRAL);
    let room = w.room;
    w.with(|g, cx| client_entered_room(g, cx, room));
    assert_eq!(w.thinks(), [102]);
}

#[test]
fn brute_tests_aip3_twice() {
    let row = |aip2, aip3| monstats(7, [0, aip2, aip3, 45, 0], 15);
    let circle = seed_where(2, |v| pc(v[0]) >= 50 && pc(v[1]) < 50);
    let idle15 = seed_where(2, |v| pc(v[0]) >= 50 && pc(v[1]) >= 50);
    let a1 = seed_where(2, |v| pc(v[0]) < 50 && pc(v[1]) < 45);
    let a2 = seed_where(2, |v| pc(v[0]) < 50 && pc(v[1]) >= 45);
    // aip2 = 0: the circle chance is aip3's.
    let mut w = World::new(row(0, 50));
    w.seed(circle);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
    assert_eq!(w.velocity().steps, 4);
    assert_eq!(draws(&w, circle), 3);
    let mut w = World::new(row(0, 50));
    w.seed(idle15);
    w.run(true, 1);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [15]);
    assert_eq!(draws(&w, idle15), 2);
    for (lo, m) in [(a1, mode::ATTACK1), (a2, mode::ATTACK2)] {
        let mut w = World::new(row(0, 50));
        w.seed(lo);
        w.run(true, 1);
        assert_eq!(w.fake.modes(), [at_unit(m, w.player)]);
    }
    // aip2 = 100, aip3 = 0: never circles.
    let mut w = World::new(row(100, 0));
    w.seed(circle);
    w.run(true, 1);
    assert_eq!(w.thinks(), [15]);
}

fn shaman_world() -> World {
    let mut row = monstats(13, [45, 60, 100, 24, 15], 15);
    row.skill1 = 5;
    row.skill2 = 6;
    let mut w = World::new(row);
    w.modes[0] = [14, 8, 0, 0, 0, 0, 0, 0];
    w
}

/// A dead minion of the shaman at `at`: the shaman is its own minion
/// owner and lists it.
fn own_corpse(w: &mut World, at: (i32, i32)) -> UnitId {
    let me = UnitRef {
        ty: UnitType::Monster,
        guid: w.guid(w.mon),
    };
    let c = w.spawn(UnitType::Monster, at);
    w.fake.anim.insert(c, mode::DEAD);
    w.fake.x.flags.insert(c, 2);
    w.store.entry(c).control = Some(AiControl {
        minion_owner: Some(me),
        ..AiControl::default()
    });
    let g = w.guid(c);
    w.control().minion_owner = Some(me);
    w.control().minions.push(g);
    c
}

fn corpses(w: &mut World, max: i32, own: bool) -> (Option<UnitId>, u32) {
    let mon = w.mon;
    w.with(|g, cx| super::super::functions::shaman_corpses(g, cx, mon, max, own))
}

#[test]
fn fallen_shaman_steps() {
    // 1. C and P(aip3) → A1; not C: no step-1 draw.
    let mut w = shaman_world();
    w.seed(1);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, w.player)]);
    assert_eq!(draws(&w, 1), 1);
    // 2. Corpse scan: the kind (scan 9 unless unique and not champion).
    for (unique, champion, found) in [
        (false, false, true),
        (true, false, false),
        (true, true, true),
    ] {
        let mut w = shaman_world();
        w.monstats[0].aip1 = 100;
        w.fake.unique = unique;
        w.fake.champion = champion;
        let c = own_corpse(&mut w, (101, 101));
        w.run(false, 30);
        // Scan 1 finds no fallen in the (unlinked) adjacent rooms.
        let check = format!("check 5 Some({c:?}) 0 0");
        assert_eq!(w.logged(&check), found, "{unique} {champion}");
    }
    // 3. P(aip1) → command 1 to the minions, not to itself.
    let mut w = shaman_world();
    w.monstats[0].aip1 = 100;
    let me = w.guid(w.mon);
    let m = w.spawn(UnitType::Monster, (0, 0));
    w.store.entry(m).control = Some(AiControl::default());
    let mg = w.guid(m);
    w.control().minion_owner = Some(UnitRef {
        ty: UnitType::Monster,
        guid: me,
    });
    w.control().minions = vec![mg];
    w.seed(1);
    w.run(false, 30);
    assert_eq!(w.store.control(m).unwrap().commands, [cmd([1, 0, 0, 0, 0])]);
    assert!(w.store.control(w.mon).unwrap().commands.is_empty());
    // 4. A corpse: P(aip1) and the skill check → sequence skill 1 on it.
    let mut w = shaman_world();
    w.monstats[0].aip1 = 100;
    let c = own_corpse(&mut w, (101, 101));
    w.run(false, 30);
    assert!(w.logged(&format!("check 5 Some({c:?}) 0 0")));
    assert_eq!(w.fake.modes(), [at_unit(mode::SEQUENCE, c)]);
    // ... skill check fails: on to 5 (T at D < aip5 → Skill2 in Sk2mode).
    let mut w = shaman_world();
    w.monstats[0].aip1 = 100;
    w.monstats[0].aip2 = 100;
    own_corpse(&mut w, (101, 101));
    w.fake.x.check_fails = true;
    w.run(false, 14);
    assert_eq!(w.fake.modes(), [at_unit(8, w.player)]);
    assert!(w.logged("skill 6"));
    // 5. D = aip5: no step-5 draw; 6. S at E < aip5 → Skill2 at S.
    let mut w = shaman_world();
    w.monstats[0].aip1 = 0;
    w.monstats[0].aip2 = 100;
    let s = w.spawn(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 14));
    w.seed(1);
    w.run(false, 15);
    assert_eq!(w.fake.modes(), [at_unit(8, s)]);
    assert_eq!(draws(&w, 1), 2, "steps 3 and 6");
    // E = aip5: no step-6 draw; 7. P(aip3) → circle 3 at T.
    let mut w = shaman_world();
    w.fake.secondary = Some((s, 15));
    w.seed(1);
    w.run(false, 15);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
    assert_eq!(w.velocity().steps, 3);
    assert_eq!(draws(&w, 1), 3, "steps 3, 7 and the circle");
    // ... else idle 10.
    let mut w = shaman_world();
    w.monstats[0].aip3 = 0;
    w.seed(1);
    w.run(false, 15);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn fallen_shaman_draw_order() {
    // Not C, every test reached: 3, 4, 5, 6, 7 and the circle.
    let mut w = shaman_world();
    w.monstats[0].aip1 = 0;
    w.monstats[0].aip2 = 0;
    own_corpse(&mut w, (101, 101));
    let s = w.spawn(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 5));
    w.seed(1);
    w.run(false, 5);
    assert_eq!(draws(&w, 1), 6);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
    // C with P(aip3) failing: 1, 3, 4, 5, 6, 7.
    let mut w = shaman_world();
    w.monstats[0].aip1 = 0;
    w.monstats[0].aip2 = 0;
    w.monstats[0].aip3 = 0;
    own_corpse(&mut w, (101, 101));
    let s = w.spawn(UnitType::Monster, (110, 100));
    w.fake.secondary = Some((s, 5));
    w.seed(1);
    w.run(true, 5);
    assert_eq!(draws(&w, 1), 6);
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn fallen_shaman_corpse_scans() {
    // Scan 9: the owner's minion list from its head (D2RUST: the end), so
    // the earliest-listed match is the last one kept.
    let mut w = shaman_world();
    let a = own_corpse(&mut w, (101, 100));
    let b = own_corpse(&mut w, (102, 100));
    assert_eq!(corpses(&mut w, 576, true), (Some(a), 2));
    // The distance gate: full-size distance (not squared) ≤ max, though
    // the shaman passes aip4².
    w.fake.pos.insert(a, (130, 100));
    assert_eq!(corpses(&mut w, 576, true), (Some(a), 2));
    assert_eq!(corpses(&mut w, 20, true), (Some(b), 1));
    w.fake.x.groups.insert((b, bodies2::GROUP_UDEAD));
    assert_eq!(corpses(&mut w, 20, true), (None, 0));
    // Not dead, flag 2 clear, or another owner: no match.
    let mut w = shaman_world();
    let a = own_corpse(&mut w, (101, 100));
    w.fake.anim.insert(a, mode::NEUTRAL);
    assert_eq!(corpses(&mut w, 576, true).1, 0);
    w.fake.anim.insert(a, mode::DEAD);
    w.fake.x.flags.insert(a, 0);
    assert_eq!(corpses(&mut w, 576, true).1, 0);
    w.fake.x.flags.insert(a, 2);
    w.store.control_mut(a).unwrap().minion_owner = None;
    assert_eq!(corpses(&mut w, 576, true).1, 0);
    // Scan 1: dead fallen1 / fallenshaman1 in the adjacent rooms, any
    // owner; squared distance ≤ max.
    let mut w = shaman_world();
    let room = w.room;
    w.game.lists.room_mut(room).unwrap().adjacent = vec![room];
    for base in [19, 58, 7] {
        let mut r = monstats(0, [0; 5], 0);
        r.baseid = base;
        w.monstats.push(r);
    }
    let dead = |w: &mut World, class: i32, at: (i32, i32)| {
        let c = w.spawn(UnitType::Monster, at);
        w.fake.class.insert(c, class);
        w.fake.anim.insert(c, mode::DEAD);
        w.fake.x.flags.insert(c, 2);
        c
    };
    let f = dead(&mut w, 1, (110, 100));
    let s = dead(&mut w, 2, (100, 110));
    dead(&mut w, 3, (101, 100));
    // Room list order (newest first): the earliest spawned is kept.
    assert_eq!(corpses(&mut w, 100, false), (Some(f), 2));
    assert_eq!(corpses(&mut w, 99, false), (None, 0));
    w.fake.x.groups.insert((f, bodies2::GROUP_HIDE));
    assert_eq!(corpses(&mut w, 100, false), (Some(s), 1));
    w.fake.champion = true;
    assert_eq!(corpses(&mut w, 100, false), (None, 0));
}

#[test]
fn quill_rat_steps() {
    let row = || monstats(14, [10, 0, 0, 2, 0], 15);
    // 1. The command's unit exists: A2 at T, command freed.
    let mut w = World::new(row());
    let g = w.guid(w.player) as i32;
    w.control().commands = vec![cmd([1, 0, g, 0, 0])];
    w.seed(1);
    w.run(false, 5);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK2, w.player)]);
    assert!(w.store.control(w.mon).unwrap().commands.is_empty());
    assert_eq!(draws(&w, 1), 0);
    // ... it does not: freed, go on (2. C → A1).
    let mut w = World::new(row());
    w.control().commands = vec![cmd([1, 1, 9999, 0, 0])];
    w.seed(1);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, w.player)]);
    assert!(w.store.control(w.mon).unwrap().commands.is_empty());
    assert_eq!(draws(&w, 1), 0);
    // 3. AI state 3/19 → A2.
    let mut w = World::new(row());
    w.fake.ai_state = 3;
    w.seed(1);
    w.run(false, 5);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK2, w.player)]);
    assert_eq!(draws(&w, 1), 0);
    // 7. The escape fails: D > 3 → wander max(aip4, 3); else A2.
    let mut w = World::new(row());
    w.fake.walk_fails = true;
    w.seed(1);
    w.run(false, 5);
    let ((x, y), _) = wander_after(1, 1, (100, 100), 3);
    assert_eq!(last_mode(&w), walk_point(x, y));
    let mut w = World::new(row());
    w.fake.walk_fails = true;
    w.seed(1);
    w.run(false, 3);
    assert_eq!(last_mode(&w), at_unit(mode::ATTACK2, w.player));
}

fn lancer_world() -> World {
    let mut w = World::new(monstats(36, [60, 75, 9, 0, 15], 15));
    w.modes[0] = [8, 9, 14, 0, 0, 0, 0, 0];
    w
}

#[test]
fn corrupt_lancer_combat() {
    // No skills: one draw at most.
    let mut w = lancer_world();
    w.seed(1); // 51 < 75
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, w.player)]);
    assert_eq!(draws(&w, 1), 1);
    let mut w = lancer_world();
    w.seed(12345); // 87 ≥ 75 → idle aip3
    w.run(true, 1);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [9]);
    assert_eq!(draws(&w, 12345), 1);
    let mut w = lancer_world();
    w.control().params[0] = 1;
    w.seed(12345);
    w.run(true, 1);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, w.player)]);
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 0);
    assert_eq!(draws(&w, 12345), 0);
    // Skills in order with aip6..aip8; a missing skill skips its draw.
    let mut w = lancer_world();
    w.monstats[0].skill1 = 3;
    w.monstats[0].aip6 = 100;
    w.control().params[0] = 1;
    w.run(true, 1);
    assert!(w.logged("skill 3"));
    assert_eq!(last_mode(&w), at_unit(8, w.player));
    let mut w = lancer_world();
    w.monstats[0].skill1 = 3;
    w.monstats[0].skill2 = 4;
    w.monstats[0].aip7 = 100;
    w.control().params[0] = 1;
    w.seed(1);
    w.run(true, 1);
    assert_eq!(last_mode(&w), at_unit(9, w.player));
    assert_eq!(draws(&w, 1), 2);
    let mut w = lancer_world();
    w.monstats[0].skill1 = 3;
    w.monstats[0].skill3 = 6;
    w.monstats[0].aip8 = 100;
    w.control().params[0] = 1;
    w.seed(1);
    w.run(true, 1);
    assert_eq!(last_mode(&w), at_unit(14, w.player));
    assert_eq!(draws(&w, 1), 2);
}

fn navi_world() -> World {
    World::new(monstats(58, [0; 5], 15))
}

#[test]
fn navi_steps() {
    // 1. Interacting: idle 10.
    let mut w = navi_world();
    w.fake.interacting = true;
    w.seed(1);
    w.run_p(param(None, 0, false));
    assert_eq!(w.thinks(), [10]);
    assert_eq!(draws(&w, 1), 0);
    // 2. Greeting.
    let yes = seed_where(1, |v| v[0] % 3 != 0);
    let no = seed_where(1, |v| v[0] % 3 == 0);
    let mut w = navi_world();
    let pl = w.player;
    w.fake.nearest = Some((pl, true));
    w.seed(yes);
    w.run_p(param(None, 0, false));
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 60);
    assert!(w.logged("sound 18") && w.logged(&format!("sound-to {pl:?}")));
    assert_eq!(w.thinks(), [20]);
    // Param 1 counts down.
    let mut w = navi_world();
    w.fake.nearest = Some((pl, true));
    w.control().params[1] = 5;
    w.seed(yes);
    w.run_p(param(None, 0, false));
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 4);
    assert!(!w.logged("sound 18"));
    assert_eq!(w.thinks(), [20]);
    // A taken player at d >= 4 ("not close") greets too: the close word
    // is never cleared ([ebp-4]).
    let mut w = navi_world();
    w.fake.nearest = Some((pl, false));
    w.seed(yes);
    w.run_p(param(None, 0, false));
    assert_eq!(w.store.control(w.mon).unwrap().params[1], 60);
    assert_eq!(w.thinks(), [20]);
    // roll(3) = 0, busy, or no player: step 3; roll(3) drawn only for a
    // non-busy player.
    for (case, lo, close, busy, player, want_draws) in [
        (0, no, true, false, true, 1),
        (2, yes, true, true, true, 0),
        (3, yes, true, false, false, 0),
    ] {
        let mut w = navi_world();
        let who = if player { w.player } else { w.mon };
        w.fake.nearest = Some((who, close));
        if busy {
            w.fake.busy.insert(who);
        }
        w.seed(lo);
        w.run_p(param(None, 0, false));
        assert_eq!(w.thinks(), [50], "case {case}");
        assert_eq!(draws(&w, lo), want_draws, "case {case}");
        assert!(!w.logged("sound 18"));
    }
    // 3. Secondary target within 25: A1 at it.
    for (e, a1) in [(24, true), (25, false)] {
        let mut w = navi_world();
        let s = w.spawn(UnitType::Monster, (110, 100));
        w.fake.secondary = Some((s, e));
        w.run_p(param(None, 0, false));
        if a1 {
            assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, s)]);
        } else {
            assert_eq!(w.thinks(), [50]);
        }
    }
}

#[test]
fn navi_target_mode_and_town_rogue() {
    assert_eq!(AI_TABLE[58].target_mode, 0);
    // TownRogue (62): step 3 only.
    for (e, a1) in [(24, true), (25, false)] {
        let mut w = World::new(monstats(62, [0; 5], 15));
        w.fake.interacting = true;
        w.fake.nearest = Some((w.player, true));
        let s = w.spawn(UnitType::Monster, (110, 100));
        w.fake.secondary = Some((s, e));
        w.seed(1);
        w.run_p(param(None, 0, false));
        if a1 {
            assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, s)]);
        } else {
            assert_eq!(w.thinks(), [50]);
        }
        assert_eq!(draws(&w, 1), 0);
        assert!(!w.logged("sound 18"));
    }
}

#[test]
fn skeleton_pattern() {
    // (AI, aip1, aip3, aip4, combat, seed, expected last mode / idle)
    let a1 = |w: &World| at_unit(mode::ATTACK1, w.player);
    let a2 = |w: &World| at_unit(mode::ATTACK2, w.player);
    let walk = |w: &World| at_unit(mode::WALK, w.player);
    for ai in [2u16, 9, 12, 19] {
        // C, P(aip3): Skeleton tests aip4 (seed 1: 51, 31); others A1
        // with one draw.
        for (aip4, skel_a1) in [(50, true), (20, false)] {
            let mut w = World::new(monstats(ai, [0, 7, 100, aip4, 0], 15));
            w.seed(1);
            w.run(true, 1);
            if ai != 2 || skel_a1 {
                assert_eq!(last_mode(&w), a1(&w), "ai {ai}");
            } else {
                assert_eq!(last_mode(&w), a2(&w), "ai {ai}");
            }
            assert_eq!(draws(&w, 1), if ai == 2 { 2 } else { 1 });
        }
        // C, P(aip3) fails: idle aip2.
        let mut w = World::new(monstats(ai, [100, 7, 0, 100, 0], 15));
        w.run(true, 1);
        assert!(w.fake.modes().is_empty());
        assert_eq!(w.thinks(), [7]);
        // Not C, P(aip1): walk flags 7 / Wraith walk in radius (12, 0).
        let mut w = World::new(monstats(ai, [100, 7, 0, 0, 0], 15));
        w.run(false, 9);
        if ai == 9 {
            assert!(w.logged("radius 12 0"));
            assert!(w.fake.modes().is_empty());
        } else {
            assert_eq!(last_mode(&w), walk(&w));
        }
        // Not C, P(aip1) fails: idle aip2.
        let mut w = World::new(monstats(ai, [0, 7, 100, 100, 0], 15));
        w.run(false, 9);
        assert!(w.fake.modes().is_empty());
        assert_eq!(w.thinks(), [7]);
    }
    // Flags 7 on the walk: a failed walk deletes thinks (4), sets 0x40
    // (1) and takes the fallback draw (2).
    let mut w = World::new(monstats(2, [100, 7, 0, 0, 0], 15));
    w.game.schedule_event(w.mon, 2, 30, None, 0, 0).unwrap();
    w.fake.walk_fails = true;
    w.seed(12345); // aip1 87 < 100; fallback 64 < 70 → wander 4 (fails)
    w.run(false, 9);
    assert!(w.thinks().is_empty());
    assert_ne!(w.store.control(w.mon).unwrap().flags & flag::FORCE_LOS, 0);
    let ((x, y), k) = wander_after(12345, 2, (100, 100), 4);
    assert_eq!(last_mode(&w), walk_point(x, y));
    assert_eq!(draws(&w, 12345), 2 + k);
}

fn andariel_world() -> World {
    let mut row = monstats(34, [0; 5], 15);
    row.skill1 = 10;
    row.skill2 = 11;
    let mut w = World::new(row);
    w.modes[0] = [8, 9, 0, 0, 0, 0, 0, 0];
    w
}

#[test]
fn andariel_steps() {
    let set = |w: &mut World, aips: [u16; 4]| {
        let r = &mut w.monstats[0];
        (r.aip1, r.aip2, r.aip3, r.aip4) = (aips[0], aips[1], aips[2], aips[3]);
    };
    // 1. C: Skill1 by P(aip1), else A1; no Skill1 → A1 without a draw.
    let mut w = andariel_world();
    set(&mut w, [100, 0, 0, 0]);
    w.run(true, 1);
    assert!(w.logged("skill 10"));
    assert_eq!(last_mode(&w), at_unit(8, w.player));
    let mut w = andariel_world();
    w.seed(1);
    w.run(true, 1);
    assert_eq!(last_mode(&w), at_unit(mode::ATTACK1, w.player));
    assert_eq!(draws(&w, 1), 1);
    let mut w = andariel_world();
    w.monstats[0].skill1 = 0xFFFF;
    set(&mut w, [100, 0, 0, 0]);
    w.seed(1);
    w.run(true, 1);
    assert_eq!(last_mode(&w), at_unit(mode::ATTACK1, w.player));
    assert_eq!(draws(&w, 1), 0);
    // 2. P(aip2) → idle 5.
    let mut w = andariel_world();
    set(&mut w, [0, 100, 0, 0]);
    w.run(false, 9);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [5]);
    // 3. P(aip3): Skill1 by P(aip4), else Skill2.
    let mut w = andariel_world();
    set(&mut w, [0, 0, 100, 100]);
    w.run(false, 9);
    assert_eq!(last_mode(&w), at_unit(8, w.player));
    let mut w = andariel_world();
    set(&mut w, [0, 0, 100, 0]);
    w.run(false, 9);
    assert!(w.logged("skill 11"));
    assert_eq!(last_mode(&w), at_unit(9, w.player));
    // ... no Skill2: on to 4.
    let mut w = andariel_world();
    w.monstats[0].skill2 = 0xFFFF;
    set(&mut w, [0, 0, 100, 0]);
    w.run(false, 9);
    assert_eq!(last_mode(&w), at_unit(mode::WALK, w.player));
    // 4. Velocity method 1 → 7; walk to T with flags 7.
    let mut w = andariel_world();
    w.fake.walk_fails = true;
    w.seed(12345);
    w.run(false, 9);
    assert_eq!(w.velocity().method, 7);
    assert_eq!(w.fake.modes()[0], at_unit(mode::WALK, w.player));
    assert_ne!(w.store.control(w.mon).unwrap().flags & flag::FORCE_LOS, 0);
}

fn archer_world() -> (World, UnitId) {
    let mut w = World::new(monstats(35, [0, 0, 8, 0, 15], 15));
    w.modes[0] = [8, 9, 14, 0, 0, 0, 0, 0];
    let s = w.spawn(UnitType::Monster, (110, 100));
    (w, s)
}

#[test]
fn corrupt_archer_steps() {
    // 1. No S: lo' % 100 > 49 → idle aip3; else circle 3 at T.
    let (mut w, _) = archer_world();
    w.seed(1); // 51
    w.run(false, 9);
    assert_eq!(w.thinks(), [8]);
    let (mut w, _) = archer_world();
    w.seed(4_014_346_870); // 0
    w.run(false, 9);
    assert_eq!(w.fake.modes(), [at_unit(mode::WALK, w.player)]);
    assert_eq!(w.velocity().steps, 3);
    // 2. Not C, AI state 3/19 → A1 at S.
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 10));
    w.fake.ai_state = 3;
    w.run(false, 9);
    assert_eq!(w.fake.modes(), [at_unit(mode::ATTACK1, s)]);
    // 3. E < 6 and P(aip4): speed 100, run away from S by 12.
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 5));
    w.monstats[0].aip4 = 100;
    w.run(false, 9);
    assert_eq!(w.fake.modes(), ["mode 15 Point(88, 100)"]);
    let v = w.velocity();
    assert_eq!((v.speed, v.steps), (100, 12));
    // ... the escape fails: on to 4 (0 < aip8 = 4 < E, P(aip1): speed 10,
    // walk to S with aip8 steps).
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 5));
    w.fake.fail_modes.insert(mode::RUN);
    (w.monstats[0].aip4, w.monstats[0].aip8, w.monstats[0].aip1) = (100, 4, 100);
    w.run(false, 9);
    assert_eq!(last_mode(&w), at_unit(mode::WALK, s));
    assert!(w.logged("steps 4"));
    assert_eq!(w.velocity().speed, 10);
    // E = 6: no aip4 draw; aip8 = E: no aip1 draw.
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 6));
    (w.monstats[0].aip4, w.monstats[0].aip8, w.monstats[0].aip1) = (100, 6, 100);
    w.seed(1);
    w.run(false, 9);
    assert_eq!(draws(&w, 1), 1, "only step 6");
    // 5. E > aip5: speed 100, run to S with aip5 steps.
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 20));
    w.run(false, 9);
    assert_eq!(last_mode(&w), at_unit(mode::RUN, s));
    assert!(w.logged("steps 15"));
    assert_eq!(w.velocity().speed, 100);
    // 6. P(aip2) fails → idle aip3.
    let (mut w, s) = archer_world();
    w.fake.secondary = Some((s, 10));
    w.run(false, 9);
    assert!(w.fake.modes().is_empty());
    assert_eq!(w.thinks(), [8]);
    // 7. Skill2 / Skill3 by aip6 / aip7; no Skill1 → A1; else Skill1.
    let seven = |skills: [u16; 3], aip6: u16, aip7: u16| {
        let (mut w, s) = archer_world();
        w.fake.secondary = Some((s, 10));
        let r = &mut w.monstats[0];
        (r.skill1, r.skill2, r.skill3) = (skills[0], skills[1], skills[2]);
        (r.aip2, r.aip6, r.aip7) = (100, aip6, aip7);
        w.run(true, 9);
        (last_mode(&w), s)
    };
    let none = 0xFFFF;
    let (m, s) = seven([3, 4, 5], 100, 0);
    assert_eq!(m, at_unit(9, s));
    let (m, s) = seven([3, 4, 5], 0, 100);
    assert_eq!(m, at_unit(14, s));
    let (m, s) = seven([none, 4, 5], 0, 0);
    assert_eq!(m, at_unit(mode::ATTACK1, s));
    let (m, s) = seven([3, none, none], 100, 100);
    assert_eq!(m, at_unit(8, s));
}

#[test]
fn corrupt_archer_circles_toward_no_target() {
    let (mut w, _) = archer_world();
    w.game.frame = 100;
    w.fake.walk_fails = true;
    w.fake.fail_think = Some(15);
    w.seed(4_014_346_870);
    w.run_p(param(None, 0, false));
    // The circle walks toward target 0; the failed start leaves aidel.
    assert_eq!(w.fake.modes(), ["mode 2 Point(0, 0)"]);
    assert_eq!(w.thinks(), [115]);
}

// ---- catalogue ---------------------------------------------

#[test]
fn no_act1_ai_is_d2moo_only() {
    // ("None left"): the Act 1 AIs that were D2MOO-only are read from
    // the base game and have bodies here.
    for i in [4usize, 5, 10, 37, 43, 59] {
        let row: Vec<&str> = AI_FUNCTIONS_TSV
            .lines()
            .nth(i + 1)
            .unwrap()
            .split('\t')
            .collect();
        assert_eq!(row[0], i.to_string());
        assert_eq!(row[10], "spec'd-here", "index {i}");
        assert_ne!(row[9], "-", "index {i} has a summary");
        let addr = AI_TABLE[i].think;
        assert!(implemented(addr), "index {i}");
        let mut w = World::new(monstats(i as u16, [0; 5], 15));
        w.store.unhandled.clear();
        w.run(false, 5);
        assert!(
            !w.store
                .unhandled
                .contains(&Unhandled::Function { addr, unit: w.mon }),
            "index {i}"
        );
    }
}

#[test]
fn catalogue_shape() {
    let mut lines = AI_FUNCTIONS_TSV.lines();
    assert_eq!(
        lines.next().unwrap(),
        "index\tmonai\tthink_1_14d\tinit_1_14d\talt_1_14d\ttarget_mode\td2moo_name\tmonstats_rows\taip_meaning\tsummary\tstatus"
    );
    let rows: Vec<Vec<&str>> = lines
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 148);
    let addr =
        |s: &str| s.len() == 10 && s.starts_with("0x") && u32::from_str_radix(&s[2..], 16).is_ok();
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r.len(), 11, "row {i}");
        assert_eq!(r[0], i.to_string());
        assert!(!r[1].is_empty());
        assert!(addr(r[2]), "row {i} think");
        assert!(r[3] == "-" || addr(r[3]), "row {i} init");
        assert!(r[4] == "-" || addr(r[4]), "row {i} alt");
        assert!(["0", "1", "2", "4", "5"].contains(&r[5]), "row {i}");
        let count = r[7].split(':').next().unwrap();
        assert!(count.parse::<u32>().is_ok(), "row {i} monstats_rows");
        assert!(
            ["spec'd-here", "summarized", "D2MOO-only", "unread"].contains(&r[10]),
            "row {i} status"
        );
        assert_eq!(r[9] == "-", r[10] == "unread", "row {i} summary");
    }
}

// Tests written against surviving mutants; a child module so
// they share the fakes and helpers of this module and its parent.
#[path = "../../mutant_tests/ai.rs"]
mod mutant_tests;
