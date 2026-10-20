//! Tests for missiles ↔ combat, units and the DRLG: a missile created by the real
//! creation code, run every frame by the unit dispatch, flown through a
//! real room's collision grid, hitting a real unit through the real
//! to-hit test and damage application.

use super::*;
use crate::drlg::collision::bits;
use crate::missiles::{create_missile, param_flags, unit_flag, MissileParams};
use crate::stats::stat as st;

const TOHIT: u16 = 19;
const LEVEL_STAT: u16 = 12;
const ARMORCLASS: u16 = 31;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;

struct Shot {
    fx: Fx,
    owner: UnitId,
    target: UnitId,
}

/// A monster owner at (10, 10) (AR 300, level 10) and a player at
/// (13, 10) (defense 100, level 8, life 100): to-hit chance 83. The
/// player's collision bit is set by the fixture, not by movement code.
fn shot() -> Shot {
    let mut fx = Fx::new();
    let owner = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let target = fx.spawn(UnitType::Player, 0, fx.a, 13, 10);
    fx.stats(owner, &[(TOHIT, 300), (LEVEL_STAT, 10)]);
    fx.stats(
        target,
        &[
            (ARMORCLASS, 100),
            (LEVEL_STAT, 8),
            (st::MAXHP, 25600),
            (st::HITPOINTS, 25600),
        ],
    );
    // Flags 1-3 come from the player type init.
    assert_eq!(fx.sim.sys.units.get(target).unwrap().flags & 0x0E, 0x0E);
    fx.mark(target, bits::PLAYER);
    Shot { fx, owner, target }
}

impl Shot {
    /// Missile 0 from the owner toward (tx, 10), damage 10 points.
    fn fire(&mut self, tx: i32) -> UnitId {
        let p = MissileParams {
            owner: Some(self.owner),
            origin: Some(self.owner),
            class: 0,
            flags: param_flags::TARGET_ABSOLUTE,
            target_x: tx,
            target_y: 10,
            ..MissileParams::default()
        };
        let m = self
            .fx
            .sim
            .missiles(&mut self.fx.game, |g, cx| create_missile(g, cx, &p))
            .unwrap()
            .expect("created");
        // The missile's damage setup is not part of this wiring:
        // the missile's damage stats are set here.
        self.fx.stats(m, &[(MINDAMAGE, 2560), (MAXDAMAGE, 2560)]);
        m
    }

    fn alive(&mut self, m: UnitId) -> bool {
        let in_store = self.fx.sim.hooks().missile_store().get(m).is_some();
        let in_lists = self.fx.game.lists.unit(m).is_some();
        assert_eq!(in_store, in_lists);
        assert_eq!(in_lists, self.fx.sim.sys.units.get(m).is_some());
        in_lists
    }
}

#[test]
fn missile_flies_hits_a_real_unit_and_deals_real_damage() {
    let mut s = shot();
    let (owner, target) = (s.owner, s.target);
    s.fx.seed(owner, seed_giving(10));
    let m = s.fire(13);
    // Allocation: GUID, unit seed from the game seed, the every-tick
    // type-0 event.
    assert_eq!(s.fx.sim.hooks().x.position(m), (10, 10));
    assert_eq!(s.fx.timers(m), [(0, -1)]);
    let frames = s.fx.sim.hooks().missile_store().get(m).unwrap().total;
    assert_eq!(frames, 50);
    // Runs 1 and 2 (frames 1, 2): one sub-tile each, empty grid.
    s.fx.frame();
    s.fx.frame();
    assert!(s.alive(m));
    assert_eq!(s.fx.sim.hooks().x.position(m), (12, 10));
    assert_eq!(s.fx.stat(target, st::HITPOINTS), 25600);
    // Run 3: the player's sub-tile; to-hit draw 10 < 83 on the owner's
    // seed; damage 2560 (equal bounds: no draw); collide-kill → removed.
    s.fx.frame();
    assert!(!s.alive(m));
    assert!(s.fx.game.timers.unit_timers(m).is_empty());
    assert_eq!(s.fx.stat(target, st::HITPOINTS), 25600 - 2560);
    let mut want = seed_giving(10);
    want.step();
    assert_eq!(s.fx.sim.sys.units.get(owner).unwrap().seed, want);
    let t = target.0;
    assert_eq!(
        s.fx.sim.hooks().x.log,
        [
            format!("event 0 Some({t})"),
            format!("event 11 Some({t})"),
            format!("event 2 Some({t})"),
            format!("reaction {} {t} 0x1", owner.0),
        ]
    );
    s.fx.assert_clean();
}

#[test]
fn missed_to_hit_removes_the_missile_without_damage() {
    let mut s = shot();
    s.fx.seed(s.owner, seed_giving(90));
    let m = s.fire(13);
    for _ in 0..3 {
        s.fx.frame();
    }
    assert!(!s.alive(m));
    assert_eq!(s.fx.stat(s.target, st::HITPOINTS), 25600);
    assert_eq!(
        s.fx.sim.hooks().x.log,
        [format!("event 0 Some({})", s.target.0)]
    );
    s.fx.assert_clean();
}

#[test]
fn missile_barrier_in_the_room_grid_removes_without_a_hit() {
    // Room A's tile column 4 is "fill LOS": collision 0x4 on sub-tiles
    // x = 20..24. The missile is removed without the hit handler on the
    // run that enters it.
    let mut s = shot();
    let game = &s.fx.game;
    let a = s.fx.a;
    let grid = |x| s.fx.sim.sys.hooks.drlg.collision(game, a, x, 20);
    assert_eq!(grid(BARRIER_X - 1), Some(0));
    assert_eq!(grid(BARRIER_X), Some(bits::MISSILE_BARRIER));
    // Fly along y = 20, away from the player.
    s.fx.sim.hooks().x.place(s.owner, 10, 20);
    let p = MissileParams {
        owner: Some(s.owner),
        origin: Some(s.owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: 30,
        target_y: 20,
        ..MissileParams::default()
    };
    let m =
        s.fx.sim
            .missiles(&mut s.fx.game, |g, cx| create_missile(g, cx, &p))
            .unwrap()
            .unwrap();
    let mut runs = 0;
    while s.alive(m) {
        s.fx.frame();
        runs += 1;
        assert!(runs < 60);
    }
    assert_eq!(runs, BARRIER_X - 10);
    assert!(s.fx.sim.hooks().x.log.is_empty());
    s.fx.assert_clean();
}

#[test]
fn missile_expires_after_range_runs() {
    // N = Range = 50 frames → removed on run 50 (expiry hit, no unit, no
    // damage). Flown inside room B's open floor.
    let mut s = shot();
    s.fx.sim.hooks().x.place(s.owner, 41, 30);
    let p = MissileParams {
        owner: Some(s.owner),
        origin: Some(s.owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: 79,
        target_y: 30,
        ..MissileParams::default()
    };
    let m =
        s.fx.sim
            .missiles(&mut s.fx.game, |g, cx| create_missile(g, cx, &p))
            .unwrap()
            .unwrap();
    let mut runs = 0;
    while s.alive(m) {
        s.fx.frame();
        runs += 1;
        assert!(runs < 60);
    }
    assert_eq!(runs, 50);
    s.fx.assert_clean();
}

/// The event log of one hit (to-hit 10 < 83) by a missile with data
/// flags `data`.
fn hit_log(data: u32) -> Vec<String> {
    let mut s = shot();
    s.fx.seed(s.owner, seed_giving(10));
    let m = s.fire(13);
    let store = s.fx.sim.hooks().missiles.as_mut().expect("not lent out");
    store.get_mut(m).unwrap().flags |= data;
    for _ in 0..3 {
        s.fx.frame();
    }
    assert!(!s.alive(m));
    assert_eq!(s.fx.stat(s.target, st::HITPOINTS), 25600 - 2560);
    s.fx.assert_clean();
    std::mem::take(&mut s.fx.sim.hooks().x.log)
}

#[test]
fn missile_data_flags_set_the_domissiledamage_hit_flags() {
    let s = shot();
    let (o, t) = (s.owner.0, s.target.0);
    let ev = |n: u8| format!("event {n} Some({t})");
    // Flag 1 → hit flag 0x20: the owner's `domissiledamage` (6) runs
    // before the target's `damagedbymissile` (2).
    let l = hit_log(1);
    assert_eq!(l[..4], [ev(0), ev(11), format!("event 6 Some({o})"), ev(2)]);
    // Flag 2 → 0x80 suppresses it, with or without flag 1 (flags 0 and 2
    // alone give the same log as 3).
    for data in [0, 2, 3] {
        let l = hit_log(data);
        assert_eq!(l[..3], [ev(0), ev(11), ev(2)], "data flags {data}");
    }
}

#[test]
fn missile_hit_class_merges_the_element_nibble() {
    use crate::combat::DamageRecord;
    use crate::wiring::action::missiles::merge_hit_class;
    // A record hit class of 0x60 survives as HitClass | 0x60, and the
    // fixed-class field is set to 1.
    let mut r = DamageRecord {
        hit_class: 0x60,
        ..DamageRecord::default()
    };
    merge_hit_class(&mut r, 0x03);
    assert_eq!((r.hit_class, r.hit_class_fixed), (0x63, 1));
    // The record's low nibble is replaced, not or-ed.
    let mut r = DamageRecord {
        hit_class: 0x05,
        ..DamageRecord::default()
    };
    merge_hit_class(&mut r, 0x02);
    assert_eq!((r.hit_class, r.hit_class_fixed), (0x02, 0));
    // A HitClass with an element bit sets the fixed-class field alone (the
    // record's 0xF0 bits are empty here).
    let mut r = DamageRecord::default();
    merge_hit_class(&mut r, 0x41);
    assert_eq!((r.hit_class, r.hit_class_fixed), (0x41, 1));
    // Neither has an element bit: the fixed-class field is kept.
    let mut r = DamageRecord {
        hit_class_fixed: 1,
        ..DamageRecord::default()
    };
    merge_hit_class(&mut r, 0x01);
    assert_eq!((r.hit_class, r.hit_class_fixed), (0x01, 1));
}

#[test]
fn allocation_steps_game_seed_guid_and_init() {
    let mut s = shot();
    let ty = UnitType::Missile;
    let guid_before = s.fx.game.lists.guids.get(ty);
    let mut game_seed = s.fx.sim.hooks().game_seed;
    let m = s.fire(13);
    // One game-seed step derives the missile's unit seed.
    let derived = game_seed.step();
    assert_eq!(s.fx.sim.hooks().game_seed, game_seed);
    assert_eq!(s.fx.sim.sys.units.get(m).unwrap().init_seed, derived);
    // The GUID counter of type 3 advanced by one.
    assert_eq!(s.fx.game.lists.guids.get(ty), guid_before + 1);
    // Missile init: data, unit flags 1 and 3 clear, the every-tick
    // type-0 event with args 0, 0 and no other timer.
    assert!(s.fx.sim.hooks().missile_store().get(m).is_some());
    let f = s.fx.sim.sys.units.get(m).unwrap().flags;
    assert_eq!(f & (unit_flag::BIT1 | unit_flag::IS_VALID_TARGET), 0);
    assert_eq!(s.fx.timers(m), [(0, -1)]);
}

/// The unit
/// search takes the unit's size (path shape) against the missile's: a
/// unit one sub-tile beside the line is hit when the shapes overlap, two
/// away it is not, and a dead one (player mode 0) on the line is
/// skipped. Recorded: `check-combat-champion-pack` f58, the fire bolt
/// kills the fallen at (5145, 4264) from its path cell (5144, 4264).
#[test]
fn missile_unit_search_uses_the_shapes_and_skips_the_dead() {
    let probe = |dy: i32, dead: bool| {
        let mut s = shot();
        // The shot's own target leaves the line.
        let t = s.target;
        s.fx.sim.sys.units.get_mut(t).unwrap().mode = 0;
        let victim = s.fx.spawn(UnitType::Player, 0, s.fx.a, 13, 10 + dy);
        s.fx.stats(
            victim,
            &[
                (ARMORCLASS, 100),
                (LEVEL_STAT, 8),
                (st::MAXHP, 25600),
                (st::HITPOINTS, 25600),
            ],
        );
        // A player's shape is size 2.
        s.fx.sim.hooks().x.sizes.insert(victim, 2);
        if dead {
            s.fx.sim.sys.units.get_mut(victim).unwrap().mode = 0;
        }
        let size = s.fx.sim.with(&mut s.fx.game, |_, v| v.path_size(victim));
        // The line's cell carries the player bit, as the victim's plus
        // footprint stamps it (also with the victim on the cell).
        let room = s.fx.a;
        let game = &s.fx.game;
        *s.fx
            .sim
            .sys
            .hooks
            .drlg
            .collision_mut(game, room, 13, 10)
            .unwrap() |= bits::PLAYER;
        s.fx.seed(s.owner, seed_giving(10));
        let m = s.fire(16);
        for _ in 0..3 {
            s.fx.frame();
        }
        (size, s.alive(m), s.fx.stat(victim, st::HITPOINTS))
    };
    let (size, alive, hp) = probe(1, false);
    assert_eq!(size, 2);
    assert!(!alive);
    assert_eq!(hp, 25600 - 2560);
    let (_, alive, hp) = probe(2, false);
    assert!(alive);
    assert_eq!(hp, 25600);
    let (_, alive, hp) = probe(0, true);
    assert!(alive);
    assert_eq!(hp, 25600);
}

// The state-list seams of the server-hit bodies on the real stat lists
// (a fresh list: flags 2, the owner's GUID, state on; its expiry read and
// moved).
#[test]
fn missile_state_list_seams_use_the_real_stat_lists() {
    use crate::missiles::MissileBodies;
    let mut s = shot();
    let (owner, target) = (s.owner, s.target);
    let state = 1;
    let r =
        s.fx.sim
            .missiles(&mut s.fx.game, |g, cx| {
                let w = &mut *cx.world;
                assert!(w.states_count() > state);
                assert_eq!(w.state_list_expiry(target, state), None);
                assert!(w.new_state_list(g, target, state, 40, owner));
                let e = w.state_list_expiry(target, state);
                w.set_state_list_expiry(target, state, 55);
                w.mark_state_changed(target, state);
                (e, w.state_list_expiry(target, state))
            })
            .unwrap();
    assert_eq!(r, (Some(40), Some(55)));
    let v = &s.fx.sim.sys.stats;
    assert!(v.has_state(target, state as u32));
    let l = v
        .unit_list(target)
        .and_then(|r| v.list_of_state(r, state as u32))
        .expect("list");
    assert_eq!(v.flags(l) & 2, 2);
}

// The Healing Vortex heal rolls between `phys_min` and
// `phys_max(O, k, L, 1)`; the action's missile world answers them (a
// skill without a row: 1 and 2), not the seam's (0, 0) default that left
// every heal at 0.
#[test]
fn missile_skill_phys_is_the_skill_damage_range() {
    use crate::missiles::MissileBodies;
    let mut s = shot();
    let owner = s.owner;
    let r =
        s.fx.sim
            .missiles(&mut s.fx.game, |g, cx| {
                cx.world.skill_phys(g, Some(owner), 0x7FFF, 1)
            })
            .unwrap();
    assert_eq!(r, (1, 2));
}
