//! Tests that kill mutants which survived the existing missile tests
//! Each asserts an outcome the missile rules state.

use super::*;

#[test]
fn init_clears_unit_flag_bit_1_by_value() {
    // Missile init clears unit flag bit 1 (value 2)
    // and bit 3 (value 8); other bits stay. Read the raw flags word, so a
    // wrong bit constant cannot pass.
    let mut w = World::new(row());
    w.fake.alloc_flags = 0x2 | 0x8 | 0x1;
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.flags[&m], 0x1);
}

#[test]
fn slow_needs_can_slow() {
    // Step 6 runs only with `CanSlow`: an owner in state 87 does not slow a
    // row without it. Vel 24 → (24 << 8) × 75 / 100 = 4608.
    let mut r = row();
    r.vel = 24;
    r.canslow = false;
    let mut w = World::new(r);
    w.fake.states.insert((w.owner, state::SLOWMISSILES));
    w.fake.stats.insert((w.owner, stat::SKILL_HANDOFATHENA), 50);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.velocity(m), 4608);
}

#[test]
fn sub_missile_helper_creates_the_given_class() {
    // The created missile is the sub-missile class argument,
    // not the parent's.
    let (mut w, m) = helper_world();
    let mut sub_row = row();
    sub_row.vel = 0;
    w.tables.push(sub_row);
    let mut cx = cx!(w);
    let sub = catalogue::create_with_collision_check(&mut w.game, &mut cx, m, 4, 4, 1, 0).unwrap();
    assert_eq!(w.store.get(sub).unwrap().class, 1);
    assert_eq!(w.store.get(m).unwrap().class, 0);
}

#[test]
fn store_lists_and_removes_missiles() {
    // D2RUST store API (no game rule): `missiles` lists every missile with
    // data in slot order; `remove` hands the data back and drops it.
    let mut w = World::new(row());
    let a = w.create(&w.params()).unwrap();
    let b = w.create(&w.params()).unwrap();
    assert_eq!(w.store.missiles().collect::<Vec<_>>(), [a, b]);
    let d = w.store.remove(a).unwrap();
    assert_eq!(d.class, 0);
    assert!(w.store.get(a).is_none());
    assert_eq!(w.store.missiles().collect::<Vec<_>>(), [b]);
    assert!(w.store.remove(a).is_none());
}

// ----: acceleration ----------------------------------------------

#[test]
fn acceleration_stops_only_above_max() {
    // Reaching max exactly keeps the acceleration; only a velocity above
    // max is capped and stops it.
    let mut pv = PathVelocity {
        velocity: 0,
        max: 10,
        accel: 10,
        counter: 0,
    };
    for _ in 0..5 {
        pv.advance();
    }
    assert_eq!((pv.velocity, pv.accel, pv.counter), (10, 10, 0));
    for _ in 0..5 {
        pv.advance();
    }
    assert_eq!((pv.velocity, pv.accel), (10, 0));
}

// ---- Which units a collide type accepts -------------

/// A missile of `mode` (`CollideKill` 0, so a contact keeps it alive) and
/// `LastCollide`, created by the player owner.
fn contact_world(mode: u8) -> (World, UnitId) {
    let mut r = row();
    r.collidetype = mode;
    r.collidekill = 0;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    (w, m)
}

fn unit_ref(w: &World, u: UnitId) -> UnitRef {
    let e = w.game.lists.unit(u).unwrap();
    UnitRef {
        ty: e.ty,
        guid: e.guid,
    }
}

/// Puts `unit` alone on a crossed subtile whose collision matches every
/// unit bit (no wall, no barrier), runs one frame and reports whether the
/// step-9 search accepted it: the hit handler then records it as the
/// last-collided unit before any filter of its own.
fn accepted(w: &mut World, m: UnitId, unit: UnitId) -> bool {
    w.fake.word = coll::PLAYER;
    w.fake.crossed = vec![(105, 100)];
    w.fake
        .masks
        .insert((105, 100), coll::FOOTPRINT | coll::PLAYER | coll::MONSTER);
    w.fake.units.insert((105, 100), vec![unit]);
    w.frame();
    let want = unit_ref(w, unit);
    w.store.get(m).unwrap().last_collided == Some(want)
}

fn second_player(w: &mut World) -> UnitId {
    let room = w.fake.room.unwrap();
    let p = w
        .game
        .spawn_unit(UnitType::Player, Some(room), false)
        .unwrap();
    w.fake.unit_flags(p);
    p
}

#[test]
fn mode_1_accepts_players_and_good_monsters() {
    let (mut w, m) = contact_world(1);
    let p = second_player(&mut w);
    assert!(accepted(&mut w, m, p));
    let (mut w, m) = contact_world(1);
    let mon = w.monster;
    w.fake.good.insert(mon);
    assert!(accepted(&mut w, m, mon));
    let (mut w, m) = contact_world(1);
    let mon = w.monster;
    assert!(!accepted(&mut w, m, mon));
}

#[test]
fn mode_2_accepts_monsters_only() {
    let (mut w, m) = contact_world(2);
    let mon = w.monster;
    assert!(accepted(&mut w, m, mon));
    let (mut w, m) = contact_world(2);
    let p = second_player(&mut w);
    assert!(!accepted(&mut w, m, p));
}

#[test]
fn mode_7_accepts_missiles_with_can_destroy() {
    for can_destroy in [true, false] {
        let (mut w, m) = contact_world(7);
        let mut target = row();
        target.candestroy = can_destroy;
        w.tables.push(target);
        let mut p = w.params();
        p.class = 1;
        let other = w.create(&p).unwrap();
        w.fake.unit_flags(other);
        assert_eq!(accepted(&mut w, m, other), can_destroy);
    }
    let (mut w, m) = contact_world(7);
    let mon = w.monster;
    assert!(!accepted(&mut w, m, mon));
}

#[test]
fn shared_filter_needs_both_flag_bits() {
    for bits in [
        unit_flag::IS_VALID_TARGET,
        unit_flag::CAN_BE_ATTACKED,
        unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED,
    ] {
        let (mut w, m) = contact_world(3);
        let mon = w.monster;
        w.fake.flags.insert(mon, bits);
        let both = bits == unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
        assert_eq!(accepted(&mut w, m, mon), both);
    }
}

#[test]
fn shared_filter_justhit_needs_next_hit() {
    // Without `NextHit` a unit in state 86 is accepted.
    let (mut w, m) = contact_world(3);
    let mon = w.monster;
    w.fake.states.insert((mon, state::JUSTHIT));
    assert!(accepted(&mut w, m, mon));
    // With `NextHit`, a unit not in state 86 is accepted.
    let mut r = row();
    r.collidekill = 0;
    r.nexthit = 1;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    let mon = w.monster;
    assert!(accepted(&mut w, m, mon));
}

#[test]
fn shared_filter_last_collided_is_one_unit() {
    // The last-collided unit is the owner (a player): another player is
    // still accepted (type and GUID both identify the unit).
    let (mut w, m) = contact_world(3);
    let p = second_player(&mut w);
    assert!(accepted(&mut w, m, p));
}

// ----: hit handler -------------------------------------------------

#[test]
fn mode_filter_in_hit_handler() {
    // Step 2.4: mode 0 rejects every unit; mode 2 rejects non-monsters and
    // accepts a monster. A rejected unit returns 1 with no unit event.
    for (mode, player, accepted) in [(0, false, false), (2, true, false), (2, false, true)] {
        let mut r = row();
        r.collidetype = mode;
        r.collidekill = 0;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        let unit = if player {
            second_player(&mut w)
        } else {
            w.monster
        };
        assert_eq!(w.hit(m, Some(unit), false), 1);
        assert_eq!(
            w.fake.logged("event0"),
            usize::from(accepted),
            "mode {mode}"
        );
    }
}

#[test]
fn no_pierce_left_exits() {
    // `Pierce` row, the owner has a pierce stat, the missile's stat 328 is
    // 0: word 3, so `CollideKill` removes it and stat 328 stays 0.
    let mut r = row();
    r.pierce = true;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    w.fake.stats.insert((w.owner, stat::SKILL_PIERCE), 100);
    w.fake.stats.insert((m, stat::PIERCE_IDX), 0);
    let mon = w.monster;
    assert_eq!(w.hit(m, Some(mon), false), 2);
    assert_eq!(w.fake.stat(m, stat::PIERCE_IDX), 0);
}

// ----: damage stage ------------------------------------------------

#[test]
fn target_ac_only_for_non_hireling_monsters() {
    // A monster that is not a hireling gets the missile's stat 120; a
    // hireling monster and a player get nothing.
    for (who, want) in [(0, true), (1, false), (2, false)] {
        let mut r = row();
        r.collidekill = 0;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.fake.stats.insert((m, stat::ITEM_DAMAGETARGETAC), 7);
        let unit = match who {
            0 => w.monster,
            1 => {
                let mon = w.monster;
                w.fake.hirelings.insert(mon);
                mon
            }
            _ => second_player(&mut w),
        };
        w.hit(m, Some(unit), false);
        let line = format!("ac {} 7", unit.0);
        assert_eq!(w.fake.log.contains(&line), want, "case {who}");
        assert_eq!(w.fake.logged("ac "), usize::from(want), "case {who}");
    }
}

#[test]
fn soft_hit_without_get_hit() {
    use super::hit::result_flag as rf;
    let mut r = row();
    r.softhit = true;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    let mon = w.monster;
    let mut cx = cx!(w);
    assert_eq!(
        result_flags(&w.game, &mut cx, m, mon),
        rf::HIT | rf::SOFTHIT
    );
}

#[test]
fn result_flags_are_distinct_bits() {
    // D2RUST-local values (the rules name the flags, not their bits): each
    // must be its own non-zero bit or a set flag is lost.
    use super::hit::result_flag as rf;
    let all = [rf::HIT, rf::GETHIT, rf::SOFTHIT, rf::KNOCKBACK];
    for (i, a) in all.iter().enumerate() {
        assert_eq!(a.count_ones(), 1);
        for b in &all[i + 1..] {
            assert_eq!(a & b, 0);
        }
    }
}

#[test]
fn knockback_zero_draws_nothing() {
    use super::hit::result_flag as rf;
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    let mon = w.monster;
    let seed = *w.fake.seed(m);
    let mut cx = cx!(w);
    assert_eq!(result_flags(&w.game, &mut cx, m, mon), rf::HIT);
    assert_eq!(*w.fake.seed(m), seed);
}

#[test]
fn knockback_needs_roll_below_value() {
    // `roll(100) < KnockBack`: a roll equal to the value gives no flag,
    // one below it does.
    use super::hit::result_flag as rf;
    let mut probe = World::new(row());
    let pm = probe.create(&probe.params()).unwrap();
    let mut s = *probe.fake.seed(pm);
    let k = s.roll(100);
    assert!(k > 0, "pick another seed");
    for (kb, want) in [(k, 0), (k + 1, rf::KNOCKBACK)] {
        let mut r = row();
        r.knockback = kb as u8;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        assert_eq!(*w.fake.seed(m), *probe.fake.seed(pm));
        let mon = w.monster;
        let mut cx = cx!(w);
        assert_eq!(result_flags(&w.game, &mut cx, m, mon), rf::HIT | want);
    }
}

#[test]
fn every_element_and_length_is_read() {
    // Equal bounds draw nothing and give the bound; each element and
    // length lands in its own field.
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    for (i, &(min, max, _)) in ELEMENTS.iter().enumerate() {
        let v = 10 * (i as i32 + 1);
        w.fake.stats.insert((m, min), v);
        w.fake.stats.insert((m, max), v);
    }
    for (s, v) in [
        (stat::COLDLENGTH, 101),
        (stat::POISONLENGTH, 102),
        (stat::LIFEDRAINMINDAM, 103),
        (stat::MANADRAINMINDAM, 104),
        (stat::STAMDRAINMINDAM, 105),
        (stat::BURNINGLENGTH, 106),
        (stat::STUNLENGTH, 107),
    ] {
        w.fake.stats.insert((m, s), v);
    }
    let mut cx = cx!(w);
    let d = fill_damage(&mut cx, m, None);
    let want = Damage {
        phys: 10,
        fire: 20,
        magic: 30,
        light: 40,
        cold: 50,
        poison: 60,
        burn: 70,
        cold_length: 101,
        poison_length: 102,
        life_drain: 103,
        mana_drain: 104,
        stamina_drain: 105,
        burn_length: 106,
        stun_length: 107,
        ..Damage::default()
    };
    assert_eq!(d, want);
}

#[test]
fn physical_has_no_mastery() {
    // Physical has no mastery stat: a stat 0 value on the missile does not
    // act as one.
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.stats.insert((m, stat::MINDAMAGE), 10);
    w.fake.stats.insert((m, stat::MAXDAMAGE), 10);
    w.fake.stats.insert((m, 0), 100);
    let mut cx = cx!(w);
    assert_eq!(fill_damage(&mut cx, m, None).phys, 10);
}

#[test]
fn missed_to_hit_runs_server_hit_only_with_always_explode() {
    // A missed to-hit missile calls its server-hit function first only with
    // `AlwaysExplode` and a function in 1…70; without result bit 4 it is
    // removed (2). Server-hit 18 (Shout) logs `shout` on
    // an ally and returns 0, so bit 4 is clear.
    for (always, srv_hit, called) in [(1, 18, true), (0, 18, false), (1, 0, false)] {
        let mut r = row();
        r.tohit = 1;
        r.alwaysexplode = always;
        r.psrvhitfunc = srv_hit;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.fake.hits.push_back(false);
        let mon = w.monster;
        w.fake.mb.allies.insert(mon);
        assert_eq!(w.hit(m, Some(mon), false), 2, "{always} {srv_hit}");
        assert_eq!(
            w.fake.logged("shout"),
            usize::from(called),
            "{always} {srv_hit}"
        );
        assert!(w.store.unhandled.is_empty(), "{always} {srv_hit}");
    }
}
