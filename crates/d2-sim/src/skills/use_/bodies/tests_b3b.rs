//! Coverage tests of the batch 3 bodies on [`super::fake::BodyFake`]
//! (hand-computed values from the rules and test vectors).

use super::fake::{stored, BodyFake};
use super::tests2::{body_rec, monster, tabs, world, Code};
use super::*;
use crate::combat::{CombatTables, DamageRecord};
use crate::rng::Seed;
use crate::skills::fake::{combat_tables, monster_rec, FItem, FUnit};
use crate::skills::{SkillEntry, SkillTables, SkillUnits};
use crate::units::UnitType;
use b4_helpers::throw;

/// A monster at `at` whose used skill is skill 1 (an E record exists).
fn caster(f: &mut BodyFake, at: (i32, i32)) -> usize {
    let mut m = FUnit::new(UnitType::Monster, 0);
    let e = SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    m.skills.push(e);
    m.used = Some(e);
    f.add(m, at)
}

/// The used entry of `u`.
fn ent(f: &BodyFake, u: usize) -> SkillEntry {
    f.c.units[u].used.unwrap()
}

/// `n` monstats rows.
fn ctn(n: usize) -> CombatTables {
    combat_tables(vec![monster_rec(); n])
}

/// `a` hits `d` for sure (hostile, in range, huge to-hit).
fn sure_hit(f: &mut BodyFake, a: usize, d: usize) {
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(a, 12, 1);
    f.c.set(d, 12, 1);
    f.c.set(a, 19, 100_000);
}

/// The log lines starting with `p`.
fn lines(f: &mut BodyFake, p: &str) -> Vec<String> {
    f.take_log()
        .into_iter()
        .filter(|l| l.starts_with(p))
        .collect()
}

// ---------------------------------------------------------------- –

#[test]
fn throw_srvdo_3_and_5_pick_the_item_by_inuse() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, u) = world();
    f.inventory = true;
    let a = f.c.add_item(FItem {
        types: vec![45],
        throw: true,
        ..FItem::default()
    });
    let b = f.c.add_item(FItem {
        types: vec![45],
        ..FItem::default()
    });
    f.c.units[u].items.insert(4, a);
    f.c.units[u].items.insert(5, b);
    f.item_stats.insert((a, 70), 2);
    f.max_stack.insert(a, 25);
    f.tpos.insert(u, (5, 5));
    // The picked weapon (loc 4) is in use: Throw uses it, Left Hand
    // Throw checks the other hand, which is not throwable.
    f.c.units[u].weapon = Some(a);
    assert_eq!(throw(&mut f, &t, u, 1, 1, false), 1);
    assert_eq!(f.item_stats[&(a, 70)], 1);
    assert_ne!(f.c.units[u].flags & 0x40, 0);
    f.c.units[u].flags = 0;
    assert_eq!(throw(&mut f, &t, u, 1, 1, true), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0);
    // Not in use: srvdo 3 throws the other item, srvdo 5 the picked one.
    f.c.units[u].weapon = Some(b);
    f.c.units[u].used.as_mut().unwrap().skill = 4;
    assert_eq!(
        starts::ammo(&mut f, u),
        1,
        "the start checks the selected left stack"
    );
    assert_eq!(throw(&mut f, &t, u, 1, 1, false), 0);
    assert_eq!(throw(&mut f, &t, u, 1, 1, true), 1);
    assert_eq!(
        f.item_stats[&(a, 70)],
        0,
        "Left Hand Throw spends its selected stack"
    );
    assert!(
        !f.item_stats.contains_key(&(b, 70)),
        "the current weapon is untouched"
    );
    assert_eq!(
        starts::ammo(&mut f, u),
        0,
        "the exhausted left stack cannot start another throw"
    );
    // No inventory: 0.
    f.inventory = false;
    assert_eq!(throw(&mut f, &t, u, 1, 1, true), 0);
}

// -----------------------------------------------------------------

#[test]
fn unsummon_removes_the_pet_of_the_stored_guid() {
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let e = ent(&f, u);
    f.set_entry_param_of(u, &e, 1, 77);
    assert_eq!(unsummon_call(&mut f, u), 1);
    assert_eq!(
        lines(&mut f, "PetRemove"),
        ["PetRemove { owner: 0, guid: 77, kill: true }"]
    );
    // E none: 0 and no removal.
    let m = monster(&mut f, (1, 1));
    assert_eq!(unsummon_call(&mut f, m), 0);
    assert!(lines(&mut f, "PetRemove").is_empty());
}

fn unsummon_call(f: &mut BodyFake, u: usize) -> i32 {
    b4_mon::unsummon_do(f, u)
}

// -----------------------------------------------------------------------

fn hit_rec() -> d2_data::tables::Skills {
    let mut r = body_rec();
    r.resultflags = 0x4000;
    // SKIP_ROLL | NO_MISSILE_EVENT: `start_combat` does not roll.
    r.hitflags = 0x82;
    r.hitclass = 5;
    r
}

#[test]
fn fire_hit_start_rolls_then_stores_the_record() {
    let t = tabs(hit_rec(), Code::new(), 1);
    let ct = ctn(1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let m = monster(&mut f, (1, 0));
    // R invalid → 0; T none → 0.
    assert_eq!(b4_mon::fire_hit_start(&mut f, &t, &ct, u, 99, 1), 0);
    assert_eq!(b4_mon::fire_hit_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.targets.insert(u, m);
    // A hit: ResultFlags, HitFlags and HitClass are merged; stored once.
    sure_hit(&mut f, u, m);
    assert_eq!(b4_mon::fire_hit_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.c.units[u].combat.len(), 1);
    let r = f.c.units[u].combat[0].record;
    assert_eq!(r.result & 1, 1);
    assert_eq!(r.result & 0x4000, 0x4000);
    assert_eq!(r.hit_flags & 0x82, 0x82);
    assert_eq!(r.hit_class, 5);
    // A miss: nothing is merged, the record is stored all the same.
    f.c.units[u].combat.clear();
    f.c.hostile = false;
    assert_eq!(b4_mon::fire_hit_start(&mut f, &t, &ct, u, 1, 1), 1);
    let r = f.c.units[u].combat[0].record;
    assert_eq!((r.result, r.hit_flags, r.hit_class), (0, 0, 0));
    // A player runs srvst 32 (Bash) instead.
    let (mut g, p) = world();
    let pm = monster(&mut g, (1, 0));
    g.targets.insert(p, pm);
    let mut h = g.clone();
    let got = b4_mon::fire_hit_start(&mut g, &t, &ct, p, 1, 1);
    let want = starts::bash(&mut h, &t, &ct, p, 1, 1);
    assert_eq!(got, want);
    assert_eq!(g.take_log(), h.take_log());
}

#[test]
fn fire_hit_do_applies_the_melee_and_returns_the_start_core_result() {
    let t = tabs(hit_rec(), Code::new(), 1);
    let ct = ctn(1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let m = monster(&mut f, (1, 0));
    assert_eq!(b4_mon::fire_hit(&mut f, &t, &ct, u, 1, 1), 0, "T none");
    f.targets.insert(u, m);
    stored(&mut f, u, m, DamageRecord::default());
    let mut g = f.clone();
    let want = crate::skills::use_::start_core_of(&mut g, &t, u, 1, 1);
    assert_eq!(b4_mon::fire_hit(&mut f, &t, &ct, u, 1, 1), want);
    assert!(f.c.units[u].combat.is_empty(), "records applied and freed");
}

// ---------------------------------------------------------------- –

#[test]
fn maggot_egg_start_clears_flags_and_the_footprint() {
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    f.c.units[u].flags = 0xFF;
    assert_eq!(b4_mon::maggot_egg_start(&mut f, u), 1);
    assert_eq!(f.c.units[u].flags, 0xF1);
    assert_eq!(lines(&mut f, "DeadFootprint"), ["DeadFootprint(0)"]);
}

fn egg_world(n: i16) -> (BodyFake, SkillTables, CombatTables, usize) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(n);
    let t = tabs(r, c, 1);
    let mut ct = ctn(2);
    ct.monstats[0].spawn = 1;
    ct.monstats[0].spawnmode = 3;
    let mut f = BodyFake::new();
    let u = caster(&mut f, (5, 5));
    (f, t, ct, u)
}

#[test]
fn maggot_egg_spawns_n_eggs_and_kills_the_caster() {
    let (mut f, t, ct, u) = egg_world(3);
    f.c.units[u].flags = 0;
    f.seq_speed.insert(u, 9);
    assert_eq!(
        b4_mon::maggot_egg(&mut f, &t, &ct, u, 99, 1),
        0,
        "R invalid"
    );
    assert_eq!(b4_mon::maggot_egg(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.seq_speed[&u], 0);
    let l = f.take_log();
    let near: Vec<_> = l.iter().filter(|s| s.starts_with("Near")).collect();
    assert_eq!(near.len(), 3);
    assert!(near[0].contains("class: 1, mode: 3, spread: -1"));
    // The extra eggs: mode 8 whatever `spawnmode` says, spread 1.
    assert!(near[1].contains("class: 1, mode: 8, spread: 1"));
    assert!(near[2].contains("class: 1, mode: 8, spread: 1"));
    assert_eq!(
        l.last().unwrap(),
        "KillBy { u: 0, killer: Some(0), b: 1 }",
        "the caster is killed last"
    );
    // The three eggs: no experience.
    for e in 1..=3 {
        assert_eq!(f.c.units[e].flags & 0x400_0000, 0x400_0000);
    }
    // No spawn succeeds: 1 + 4 requests (s = 1..4), the kill still runs.
    let (mut f, t, ct, u) = egg_world(3);
    f.no_monsters = true;
    assert_eq!(b4_mon::maggot_egg(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.take_log();
    assert_eq!(l.iter().filter(|s| s.starts_with("Near")).count(), 5);
    assert!(l.iter().any(|s| s.starts_with("KillBy")));
    // n ≤ 0: no spawn, still killed.
    let (mut f, t, ct, u) = egg_world(0);
    assert_eq!(b4_mon::maggot_egg(&mut f, &t, &ct, u, 1, 1), 1);
    let l = f.take_log();
    assert!(!l.iter().any(|s| s.starts_with("Near")));
    assert!(l.iter().any(|s| s.starts_with("KillBy")));
    // E none: 0, class < 0: 0.
    let (mut f, t, ct, _) = egg_world(3);
    let m = monster(&mut f, (1, 1));
    assert_eq!(b4_mon::maggot_egg(&mut f, &t, &ct, m, 1, 1), 0);
    let (mut f, t, _, u) = egg_world(3);
    let mut ct = ctn(2);
    ct.monstats[0].spawn = 0xFFFF;
    assert_eq!(b4_mon::maggot_egg(&mut f, &t, &ct, u, 1, 1), 0);
}

#[test]
fn maggot_up_moves_sets_flags_and_stamps() {
    let mut f = BodyFake::new();
    let u = caster(&mut f, (7, 9));
    f.place_fails = true;
    assert_eq!(b4_mon::maggot_up(&mut f, u), 0);
    assert_eq!(f.c.units[u].flags, 0);
    f.place_fails = false;
    f.take_log();
    assert_eq!(b4_mon::maggot_up(&mut f, u), 1);
    assert_eq!(f.c.units[u].flags, 0xE);
    let l = f.take_log();
    assert_eq!(l[0], "place 0 Some(1) (7, 9)");
    assert_eq!(
        l[1],
        "PatternStampN { room: 1, x: 7, y: 9, pattern: 1, mask: 4096 }"
    );
}

#[test]
fn maggot_down_start_clears_flags_and_the_pattern() {
    let mut f = BodyFake::new();
    let u = caster(&mut f, (7, 9));
    f.c.units[u].flags = 0x1F;
    assert_eq!(b4_mon::maggot_down_start(&mut f, u), 1);
    assert_eq!(f.c.units[u].flags, 0x11);
    assert_eq!(
        lines(&mut f, "Pattern"),
        ["PatternClearN { room: 1, x: 7, y: 9, pattern: 5, mask: 4096 }"]
    );
}

#[test]
fn maggot_down_heals_a_percent_of_the_current_life() {
    let heal = |p: i16, life: i32, frame: i32| {
        let mut c = Code::new();
        let mut r = body_rec();
        r.calc1 = c.f(p);
        let t = tabs(r, c, 1);
        let mut f = BodyFake::new();
        let u = caster(&mut f, (0, 0));
        f.c.set(u, 6, life);
        f.c.set(u, 7, 150);
        f.anim_frame.insert(u, frame);
        f.seq_speed.insert(u, 9);
        assert_eq!(b4_mon::maggot_down(&mut f, &t, u, 99, 1), 0, "R invalid");
        assert_eq!(f.c.units[u].flags & 0x40, 0);
        assert_eq!(b4_mon::maggot_down(&mut f, &t, u, 1, 1), 1);
        assert_eq!(f.c.units[u].flags & 0x40, 0x40);
        (f.c.get(u, 6), f.seq_speed[&u])
    };
    // 20 % of the current life 100.
    assert_eq!(heal(20, 100, 3 << 8), (120, 9));
    // Capped at the maximum life.
    assert_eq!(heal(60, 100, 3 << 8), (150, 9));
    // p ≤ 0: no heal; frame ≤ 0 stops the sequence.
    assert_eq!(heal(0, 100, 0), (100, 0));
}

fn lay_world() -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut r = body_rec();
    r.srvmissilea = 0xFFFF;
    let t = tabs(r, Code::new(), 1);
    let mut ct = ctn(2);
    ct.monstats[0].spawn = 1;
    ct.monstats[0].spawnmode = 3;
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    let m = monster(&mut f, (30, 10));
    f.targets.insert(u, m);
    (f, t, ct, u, m)
}

#[test]
fn maggot_lay_places_the_egg_by_direction() {
    // The eight offsets by dir8 (direction d = 8k).
    let want = [
        (-2, -2),
        (0, -2),
        (2, -2),
        (2, 0),
        (2, 2),
        (0, 2),
        (-2, 2),
        (-2, 0),
    ];
    for (k, (dx, dy)) in want.into_iter().enumerate() {
        let (mut f, t, ct, u, _) = lay_world();
        f.dirs.insert((30, 10), 8 * k as i32);
        f.action.insert(u, 5);
        assert_eq!(b4_mon::maggot_lay(&mut f, &t, &ct, u, 1), 1, "k {k}");
        assert_eq!(f.action[&u], 0, "action frame cleared");
        let l = lines(&mut f, "At");
        assert_eq!(
            l,
            [format!(
                "At {{ room: 1, x: {}, y: {}, class: 1, mode: 3, spread: -1, flags: 0 }}",
                10 + dx,
                10 + dy
            )]
        );
        // The egg gives no experience.
        assert_eq!(f.c.units.last().unwrap().flags & 0x400_0000, 0x400_0000);
    }
    // Test vector: direction 20 → k 3, egg at (+2, 0).
    let (mut f, t, ct, u, _) = lay_world();
    f.dirs.insert((30, 10), 20);
    assert_eq!(b4_mon::maggot_lay(&mut f, &t, &ct, u, 1), 1);
    assert_eq!(f.pos[&(f.c.units.len() - 1)], (12, 10));
    // R invalid, T none, spawn fails: 0.
    let (mut f, t, ct, u, _) = lay_world();
    assert_eq!(b4_mon::maggot_lay(&mut f, &t, &ct, u, 99), 0);
    f.targets.clear();
    assert_eq!(b4_mon::maggot_lay(&mut f, &t, &ct, u, 1), 0);
    f.targets.insert(u, 1);
    f.no_monsters = true;
    assert_eq!(b4_mon::maggot_lay(&mut f, &t, &ct, u, 1), 0);
    // class < 0: 0.
    let (mut f, t, _, u, _) = lay_world();
    let mut ct = ctn(2);
    ct.monstats[0].spawn = 0xFFFF;
    assert_eq!(b4_mon::maggot_lay(&mut f, &t, &ct, u, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn andrial_spray_aims_by_direction_and_frame() {
    // Start pairs by k, then the sweep offsets of the table.
    let start = [
        (3, 3),
        (0, 3),
        (-3, 3),
        (-3, 0),
        (-3, -3),
        (0, -3),
        (3, -3),
        (3, 0),
    ];
    #[rustfmt::skip]
    let sweep: [[(i32, i32); 8]; 8] = [
        [(-3, 3), (-2, 2), (-1, 2), (-1, 1), (1, -1), (2, -1), (2, -2), (3, -3)],
        [(-3, 0), (-2, 0), (-2, 1), (-1, 0), (1, 0), (2, 1), (2, 0), (3, 0)],
        [(-3, -3), (-2, -2), (-2, -1), (-1, -1), (1, 1), (1, 2), (2, 2), (3, 3)],
        [(0, -3), (0, -2), (-1, -2), (0, -1), (0, 1), (-1, 2), (0, 2), (0, 3)],
        [(3, -3), (2, -2), (1, -2), (1, -1), (-1, 1), (-2, 1), (-2, 2), (-3, 3)],
        [(3, 0), (2, 0), (1, -1), (1, 0), (-1, 0), (-1, -1), (-2, 0), (-3, 0)],
        [(3, 3), (2, 2), (2, 1), (1, 1), (-1, -1), (-1, -2), (-2, -2), (-3, -3)],
        [(0, 3), (0, 2), (1, 2), (0, 1), (0, -1), (1, -2), (0, -2), (0, -3)],
    ];
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    for k in 0..8usize {
        // Frames 4..=12 give f = 0..=8; f = 4 (frame 8) is 99: no sweep.
        for frame in 0..=13i32 {
            let mut f = BodyFake::new();
            let u = caster(&mut f, (10, 10));
            let e = ent(&f, u);
            f.set_entry_param_of(u, &e, 1, 50);
            f.set_entry_param_of(u, &e, 2, 60);
            f.dirs.insert((50, 60), 8 * k as i32);
            f.anim_frame.insert(u, frame << 8);
            assert_eq!(b4_mon::andrial_spray(&mut f, &t, u, 1, 7), 1);
            let ff = (frame - 4).clamp(0, 8);
            let (sx, sy) = start[k];
            let (dx, dy) = match ff {
                0..=3 => sweep[k][ff as usize],
                4 => (0, 0),
                _ => sweep[k][ff as usize - 1],
            };
            assert_eq!(f.missiles.len(), 1);
            let q = f.missiles[0];
            assert_eq!(
                (q.target_x, q.target_y),
                (10 + sx + dx, 10 + sy + dy),
                "k {k} frame {frame}"
            );
            assert_eq!((q.flags, q.origin, q.class, q.level), (0x20, Some(u), 0, 7));
            // Edge case 2: the skill field stays 0 (Attack).
            assert_eq!(q.skill, 0);
        }
    }
    // Test vector: k = 1, frames 3, 8, 9, 12.
    for (frame, want) in [(3, (-3, 3)), (8, (0, 3)), (9, (1, 3)), (12, (3, 3))] {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (10, 10));
        let e = ent(&f, u);
        f.set_entry_param_of(u, &e, 1, 50);
        f.set_entry_param_of(u, &e, 2, 60);
        f.dirs.insert((50, 60), 8);
        f.anim_frame.insert(u, frame << 8);
        b4_mon::andrial_spray(&mut f, &t, u, 1, 1);
        let q = f.missiles[0];
        assert_eq!((q.target_x - 10, q.target_y - 10), want);
    }
    // A zero stored coordinate: the target position instead; none → 0.
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    f.dirs.insert((50, 60), 8);
    assert_eq!(b4_mon::andrial_spray(&mut f, &t, u, 1, 1), 0);
    f.tpos.insert(u, (50, 60));
    assert_eq!(b4_mon::andrial_spray(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.missiles[0].target_y, 13);
    // R invalid, missile out of range, E none: 0.
    assert_eq!(b4_mon::andrial_spray(&mut f, &t, u, 99, 1), 0);
    let mut r = body_rec();
    r.srvmissilea = 3;
    let t3 = tabs(r, Code::new(), 1);
    assert_eq!(b4_mon::andrial_spray(&mut f, &t3, u, 1, 1), 0);
    let m = monster(&mut f, (1, 1));
    assert_eq!(b4_mon::andrial_spray(&mut f, &t, m, 1, 1), 0);
}

// -------------------------------------------------------------------------

fn jump_rec() -> (SkillTables, CombatTables) {
    let mut c = Code::new();
    let mut r = hit_rec();
    r.calc1 = c.f(25);
    (tabs(r, c, 1), ctn(1))
}

#[test]
fn jump_start_mirrors_the_target_and_launches() {
    let (t, ct) = jump_rec();
    let new = || {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (10, 10));
        (f, u)
    };
    // Test vector: from (10, 10) at T (14, 12) → (18, 14).
    let (mut f, u) = new();
    let k = monster(&mut f, (14, 12));
    f.targets.insert(u, k);
    sure_hit(&mut f, u, k);
    let e = ent(&f, u);
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.entry_param(u, &e, 1), f.entry_param(u, &e, 2)), (18, 14));
    assert_eq!(f.entry_flags(u, &e), 0x80, "launch");
    assert_eq!((f.entry_param(u, &e, 3), f.entry_param(u, &e, 4)), (1, 1));
    assert!(f.c.has_state(u, 54), "uninterruptable");
    let r = f.c.units[u].combat[0].record;
    assert_eq!((r.result & 1, r.enh_pct, r.hit_class), (1, 25, 5));
    assert_eq!(r.hit_flags & 0x82, 0x82);
    assert_eq!(
        lines(&mut f, "PatternStamp"),
        ["PatternStamp { room: 1, x: 18, y: 14, u: 0, mask: 256 }"]
    );
    // A miss is stored all the same.
    let (mut f, u) = new();
    let k = monster(&mut f, (14, 12));
    f.targets.insert(u, k);
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 1, 1), 1);
    let r = f.c.units[u].combat[0].record;
    assert_eq!((r.result, r.enh_pct), (0, 0));
    // No target: the path's target point, E param 4 := −1, no combat.
    let (mut f, u) = new();
    f.path_target = (7, 8);
    let e = ent(&f, u);
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_param(u, &e, 4), -1);
    assert_eq!((f.entry_param(u, &e, 1), f.entry_param(u, &e, 2)), (7, 8));
    assert!(f.c.units[u].combat.is_empty());
    // Refusals: R invalid, E none, P none, frozen, no room at (x, y).
    let (mut f, u) = new();
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 99, 1), 0);
    let m = monster(&mut f, (0, 0));
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, m, 1, 1), 0);
    f.path = false;
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.path = true;
    f.c.units[u].states.push(1);
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 1, 1), 0);
    f.c.units[u].states.clear();
    f.path_target = (7, 8);
    f.point_rooms.insert((7, 8), None);
    assert_eq!(b4_mon::jump_start(&mut f, &t, &ct, u, 1, 1), 0);
}

fn jump_world(base: i16) -> (BodyFake, CombatTables, usize, usize) {
    let mut ct = ctn(1);
    ct.monstats[0].baseid = base as _;
    let mut f = BodyFake::new();
    let u = caster(&mut f, (20, 20));
    let k = monster(&mut f, (18, 19));
    let e = ent(&f, u);
    f.set_entry_param_of(u, &e, 1, 20);
    f.set_entry_param_of(u, &e, 2, 20);
    f.set_entry_param_of(u, &e, 3, 1);
    f.set_entry_param_of(u, &e, 4, k as i32);
    (f, ct, u, k)
}

fn path_lines_of(l: &[String], u: usize, ops: &[PathOp<usize>]) -> (Vec<String>, Vec<String>) {
    (
        l.iter()
            .filter(|s| s.starts_with("path"))
            .cloned()
            .collect(),
        ops.iter().map(|o| format!("path {u} {o:?}")).collect(),
    )
}

#[test]
fn jump_do_lands_launches_and_hops() {
    // Refusals and the timers.
    let (mut f, ct, u, k) = jump_world(0);
    assert_eq!(b4_mon::jump(&mut f, &ct, k), 0, "E none");
    f.path = false;
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 0, "P none");
    f.path = true;
    f.take_log();
    // Action frame 2: the stored melee of the jump is applied.
    f.action.insert(u, 2);
    stored(&mut f, u, k, DamageRecord::default());
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
    assert_eq!(f.action[&u], 0);
    assert!(f.c.units[u].combat.is_empty());
    assert!(f.take_log().contains(&"deltimers 0 1 0".to_string()));
    // Landing (in flight, at (x, y)), not a sandleaper: path type 101.
    let (mut f, ct, u, _) = jump_world(0);
    let e = ent(&f, u);
    f.set_entry_flags(u, &e, 0x100);
    f.c.units[u].states.push(54);
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
    assert!(!f.c.has_state(u, 54), "interruptible again");
    let l = f.take_log();
    assert!(l.contains(&"PatternClear { room: 1, x: 20, y: 20, u: 0, mask: 256 }".to_string()));
    let (got, want) = path_lines_of(
        &l,
        0,
        &[
            PathOp::FootprintMask(0x100),
            PathOp::MoveMask(0x3C01),
            PathOp::Type(101),
        ],
    );
    assert_eq!(got, want);
    // Landing as a sandleaper (BaseId 78) with K: the hop aims from K.
    let (mut f, ct, u, _) = jump_world(78);
    let e = ent(&f, u);
    f.set_entry_flags(u, &e, 0x100);
    f.frame_count.insert(u, 0x345);
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
    assert_eq!(f.entry_flags(u, &e), 1);
    assert_eq!(f.frame_index[&u], 12);
    assert_eq!(f.frame_count[&u], 0x400);
    let l = f.take_log();
    // Test vector: (x, y) = (20, 20), K at (18, 19) → (24, 22).
    let (got, want) = path_lines_of(
        &l,
        0,
        &[
            PathOp::FootprintMask(0x100),
            PathOp::MoveMask(0x3C01),
            PathOp::Type(101),
            PathOp::SnapCenter,
            PathOp::Steps(5),
            PathOp::Op649070(1),
            PathOp::TargetUnit(None),
            PathOp::TargetPoint(24, 22),
            PathOp::Type(8),
            PathOp::Op648E40(5),
            PathOp::Compute,
        ],
    );
    assert_eq!(got, want);
    // A sandleaper whose K is gone just lands.
    let (mut f, ct, u, _) = jump_world(78);
    let e = ent(&f, u);
    f.set_entry_flags(u, &e, 0x100);
    f.set_entry_param_of(u, &e, 4, 99);
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
    // Not there yet: event index 8 for the sandleaper, else 0.
    for (base, idx) in [(78, 8), (0, 0)] {
        let (mut f, ct, u, _) = jump_world(base);
        let e = ent(&f, u);
        f.set_entry_flags(u, &e, 0x100);
        f.set_entry_param_of(u, &e, 1, 25);
        f.frame_index.insert(u, 5);
        f.frame_count.insert(u, 0x345);
        assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
        assert_eq!((f.frame_index[&u], f.frame_count[&u]), (idx, 0x400));
        assert_eq!(f.entry_flags(u, &e), 0x100);
    }
    // Launch: the leap path at the walk velocity (Velocity 1 << 8).
    let (mut f, ct, u, _) = jump_world(0);
    let e = ent(&f, u);
    f.set_entry_param_of(u, &e, 1, 25);
    f.set_entry_param_of(u, &e, 2, 26);
    f.set_entry_flags(u, &e, 0x80);
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
    assert_eq!(f.entry_flags(u, &e), 0x101);
    let l = f.take_log();
    let (got, want) = path_lines_of(
        &l,
        0,
        &[
            PathOp::MoveMask(0),
            PathOp::FootprintMask(0),
            PathOp::TargetPoint(25, 26),
            PathOp::Type(9),
            PathOp::Velocity(256),
            PathOp::Compute,
        ],
    );
    assert_eq!(got, want);
    // Neither flag: cleared.
    let (mut f, ct, u, _) = jump_world(0);
    let e = ent(&f, u);
    f.set_entry_flags(u, &e, 0x4);
    assert_eq!(b4_mon::jump(&mut f, &ct, u), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
}

// -------------------------------------------------------------------------

#[test]
fn swarm_move_start_computes_toward_then_astar() {
    let mk = || {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (0, 0));
        let k = monster(&mut f, (5, 5));
        f.targets.insert(u, k);
        (f, u)
    };
    // Toward path finds points: moving skill, no A* run.
    let (mut f, u) = mk();
    f.path_points = 4;
    let e = ent(&f, u);
    assert_eq!(b4_mon::swarm_move_start(&mut f, u), 1);
    assert_eq!(f.entry_flags(u, &e), 1);
    let l = f.take_log();
    let (got, want) = path_lines_of(&l, u, &[PathOp::Steps(5), PathOp::Type(2), PathOp::Compute]);
    assert_eq!(got, want);
    // No points either way: 0, both types tried.
    let (mut f, u) = mk();
    let e = ent(&f, u);
    assert_eq!(b4_mon::swarm_move_start(&mut f, u), 0);
    assert_eq!(f.entry_flags(u, &e), 0);
    let l = f.take_log();
    let (got, want) = path_lines_of(
        &l,
        u,
        &[
            PathOp::Steps(5),
            PathOp::Type(2),
            PathOp::Compute,
            PathOp::Type(1),
            PathOp::Compute,
        ],
    );
    assert_eq!(got, want);
    // T none, P none, E none: 0.
    let (mut f, u) = mk();
    f.targets.clear();
    assert_eq!(b4_mon::swarm_move_start(&mut f, u), 0);
    let (mut f, u) = mk();
    f.path = false;
    assert_eq!(b4_mon::swarm_move_start(&mut f, u), 0);
    let (mut f, _) = mk();
    let m = monster(&mut f, (1, 1));
    f.targets.insert(m, 1);
    assert_eq!(b4_mon::swarm_move_start(&mut f, m), 0);
}

#[test]
fn swarm_move_do_picks_calc1_or_calc2_by_the_move_ended_flag() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(11);
    r.calc2 = c.f(22);
    let t = tabs(r, c, 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let e = ent(&f, u);
    assert_eq!(b4_mon::swarm_move(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.frame_index[&u], 11);
    f.set_entry_flags(u, &e, 2);
    assert_eq!(b4_mon::swarm_move(&mut f, &t, u, 1, 1), 1);
    assert_eq!(f.frame_index[&u], 22);
    assert_eq!(f.entry_flags(u, &e), 0, "flags cleared");
    assert_eq!(b4_mon::swarm_move(&mut f, &t, u, 99, 1), 0);
    let m = monster(&mut f, (1, 1));
    assert_eq!(b4_mon::swarm_move(&mut f, &t, m, 1, 1), 0);
}

// -------------------------------------------------------------------------

#[test]
fn quick_strike_start_stores_a_zeroed_record() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = ctn(1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let k = monster(&mut f, (1, 0));
    assert_eq!(b4_mon::quick_strike_start(&mut f, &t, &ct, u), 0, "T none");
    f.targets.insert(u, k);
    sure_hit(&mut f, u, k);
    assert_eq!(b4_mon::quick_strike_start(&mut f, &t, &ct, u), 1);
    assert_eq!(f.c.units[u].combat.len(), 1);
    assert_eq!(f.c.units[u].combat[0].record, DamageRecord::default());
}

#[test]
fn quick_strike_do_takes_the_mode_missile_only_after_an_action_event() {
    let mut ct = ctn(1);
    {
        let m = &mut ct.monstats[0];
        m.missa1 = 10;
        m.missa2 = 11;
        m.missc = 12;
        m.misss1 = 13;
        m.misss2 = 14;
        m.misss3 = 15;
        m.misss4 = 16;
        m.misssq = 17;
    }
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    f.action.insert(u, 1);
    f.action_event = true;
    for (mode, want) in [
        (4, 10),
        (5, 11),
        (7, 12),
        (8, 13),
        (9, 14),
        (10, 15),
        (11, 16),
        (14, 17),
        (1, -1),
        (6, -1),
        (12, -1),
    ] {
        f.c.units[u].mode = mode;
        assert_eq!(b4_mon::mode_missile(&f, &ct, u), want, "mode {mode}");
    }
    f.c.units[u].mode = 4;
    f.action_event = false;
    assert_eq!(b4_mon::mode_missile(&f, &ct, u), -1, "no action event");
    f.action_event = true;
    f.action.insert(u, 0);
    assert_eq!(b4_mon::mode_missile(&f, &ct, u), -1, "action frame 0");

    // The body: the melee is applied, then the missile.
    let mut ct0 = ctn(1);
    ct0.monstats[0].missa1 = 0;
    let t = tabs(body_rec(), Code::new(), 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let k = monster(&mut f, (5, 6));
    assert_eq!(
        b4_mon::quick_strike(&mut f, &t, &ct0, u, 99, 1),
        0,
        "R invalid"
    );
    assert_eq!(b4_mon::quick_strike(&mut f, &t, &ct0, u, 1, 1), 0, "T none");
    f.targets.insert(u, k);
    stored(&mut f, u, k, DamageRecord::default());
    f.c.units[u].mode = 4;
    // srvmissilea = −1 and no action event: 0 after the melee (Edge 13).
    assert_eq!(b4_mon::quick_strike(&mut f, &t, &ct0, u, 1, 1), 0);
    assert!(f.c.units[u].combat.is_empty(), "melee applied");
    assert!(f.missiles.is_empty());
    // After an action event: the monstats missile, straight, to T.
    f.action.insert(u, 1);
    f.action_event = true;
    assert_eq!(b4_mon::quick_strike(&mut f, &t, &ct0, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 1);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.class, q.target_x, q.target_y), (0x21, 0, 5, 6));
    assert_eq!((q.skill, q.level), (1, 1));
    // A missile row out of range: 0.
    ct0.monstats[0].missa1 = 4;
    f.missiles.clear();
    assert_eq!(b4_mon::quick_strike(&mut f, &t, &ct0, u, 1, 1), 0);
    // srvmissilea ≥ 0 is used as is, with or without an event.
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t2 = tabs(r, Code::new(), 1);
    f.action_event = false;
    assert_eq!(b4_mon::quick_strike(&mut f, &t2, &ct0, u, 1, 1), 1);
    assert_eq!(f.missiles.len(), 1);
    // A player never takes the monstats column.
    let (mut g, p) = world();
    let pk = monster(&mut g, (5, 6));
    g.targets.insert(p, pk);
    g.action.insert(p, 1);
    g.action_event = true;
    assert_eq!(b4_mon::quick_strike(&mut g, &t, &ct0, p, 1, 1), 0);
}

#[test]
fn monster_mode_missile_levels_and_the_melee_fallback() {
    // Test vector "quillrat1 A2, Hell,
    // no flag 0x200 → spike at level 7 + 1 = 8; skill 0".
    let mut ct = ctn(1);
    ct.monstats[0].missa2 = 0;
    ct.monstats[0].missa1 = 0xFFFF;
    ct.difficultylevels[2].monsterskillbonus = 7;
    let mut f = BodyFake::new();
    f.c.difficulty = 2;
    let u = caster(&mut f, (0, 0));
    let k = monster(&mut f, (5, 6));
    f.targets.insert(u, k);
    f.c.units[u].mode = 5;
    // Not moving: the column by mode with no action-frame test.
    f.action.insert(u, 0);
    assert_eq!(b4_mon::monster_mode_missile(&mut f, &ct, u, false), 1);
    assert_eq!(f.missiles.len(), 1);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.class, q.target_x, q.target_y), (0x21, 0, 5, 6));
    assert_eq!((q.skill, q.level), (0, 8));
    // Moving: the action-frame tests of the mode missile `(unit, 1)` apply.
    assert_eq!(b4_mon::monster_mode_missile(&mut f, &ct, u, true), 0);
    assert_eq!(f.missiles.len(), 1);
    // A hireling (flag 0x200) uses its own stat 12.
    f.c.units[u].flags |= 0x200;
    f.c.set(u, 12, 30);
    assert_eq!(b4_mon::monster_mode_missile(&mut f, &ct, u, false), 1);
    assert_eq!(f.missiles[1].level, 30);
    // No missile for the mode (A1 −1): 0, the caller's melee fallback.
    f.c.units[u].mode = 4;
    assert_eq!(b4_mon::monster_mode_missile(&mut f, &ct, u, false), 0);
    assert_eq!(f.missiles.len(), 2);
}

#[test]
fn quill_volley_adds_aip3_spikes_on_the_guid_seed_and_keeps_the_target() {
    let mut ct = ctn(1);
    ct.monstats[0].missa1 = 0;
    ct.monstats[0].baseid = 63;
    ct.monstats[0].aip3_n = 3;
    ct.difficultylevels[1].monsterskillbonus = 3;
    let mut f = BodyFake::new();
    f.c.difficulty = 1;
    let u = caster(&mut f, (0, 0));
    let k = monster(&mut f, (20, 30));
    f.targets.insert(u, k);
    // The fake keeps no path target point: a fixed target position lets
    // every spike be made.
    f.tpos.insert(u, (9, 9));
    f.c.units[u].mode = 4;
    f.take_log();
    assert_eq!(b4_mon::monster_mode_missile(&mut f, &ct, u, false), 1);
    // Around T's position (20, 30): the signs carry over between spikes.
    let g = f.c.units[u].guid;
    let mut seed = Seed::new(0x5345_4953, g);
    let (mut sx, mut sy) = (5, 5);
    let mut want = vec![format!("path {u} TargetUnit(None)")];
    for _ in 0..3 {
        if seed.step() & 1 != 0 {
            sx = -sx;
        }
        if seed.step() & 1 != 0 {
            sy = -sy;
        }
        want.push(format!("path {u} TargetPoint({}, {})", 20 + sx, 30 + sy));
    }
    want.push(format!("path {u} TargetUnit(Some({k}))"));
    assert_eq!(lines(&mut f, "path"), want);
    assert_eq!(f.targets.get(&u), Some(&k));
    // The mode missile at Nightmare's bonus 3 + 1, then 3 spikes at level 1.
    let levels: Vec<_> = f
        .missiles
        .iter()
        .map(|q| (q.class, q.skill, q.level))
        .collect();
    assert_eq!(levels, [(0, 0, 4), (0, 0, 1), (0, 0, 1), (0, 0, 1)]);
    // Any other BaseId: no volley.
    ct.monstats[0].baseid = 62;
    f.missiles.clear();
    f.take_log();
    assert_eq!(b4_mon::monster_mode_missile(&mut f, &ct, u, false), 1);
    assert_eq!(f.missiles.len(), 1);
    assert!(lines(&mut f, "path").is_empty());
}

// ------------------------------------------------------------------

#[test]
fn gargoyle_trap_starts_a_sixth_of_the_way_with_each_axis_quotient() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    // (target, start, offset)
    let cases = [
        // |dx| ≥ |dy|: x' = tx, y' four steps toward ty (stops at ty).
        ((30, 12), (12, 9), (20, 2)),
        // |dx| < |dy|: x' four steps toward tx (stops at tx), y' = ty.
        ((12, 40), (9, 14), (2, 30)),
        ((13, 30), (9, 12), (3, 20)),
        // Truncation toward zero: −15 / 6 = −2, −3 / 6 = 0.
        ((-5, 7), (7, 9), (-15, -3)),
        // Four steps only.
        ((30, 25), (12, 9), (20, 4)),
    ];
    for (tp, start, off) in cases {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (10, 10));
        let k = monster(&mut f, tp);
        f.targets.insert(u, k);
        assert_eq!(b4_mon::gargoyle_trap(&mut f, &t, u, 1, 3), 1);
        assert_eq!(f.c.units[u].flags & 0x40, 0x40);
        assert_eq!(f.missiles.len(), 1);
        let q = f.missiles[0];
        assert_eq!((q.x, q.y), start, "target {tp:?}");
        assert_eq!((q.target_x, q.target_y), off, "target {tp:?}");
        assert_eq!((q.flags, q.skill, q.level, q.class), (3, 1, 3, 0));
    }
    // R invalid, no missile row, T none: 0.
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    assert_eq!(b4_mon::gargoyle_trap(&mut f, &t, u, 99, 1), 0);
    assert_eq!(b4_mon::gargoyle_trap(&mut f, &t, u, 1, 1), 0, "T none");
    assert_eq!(
        f.c.units[u].flags & 0x40,
        0x40,
        "flag set before the T test"
    );
    let mut r = body_rec();
    r.srvmissilea = 9;
    let t9 = tabs(r, Code::new(), 1);
    assert_eq!(b4_mon::gargoyle_trap(&mut f, &t9, u, 1, 1), 0);
}

// ---------------------------------------------------------------- –

#[test]
fn submerge_and_emerge_flags_and_frames() {
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    f.c.units[u].flags = 0xFF;
    assert_eq!(b4_mon::submerge_start(&mut f, u), 1);
    assert_eq!(f.c.units[u].flags, 0xF1);
    assert_eq!(b4_mon::emerge_start(&mut f, u), 1);
    assert_eq!(f.c.units[u].flags, 0xFF);
    f.c.units[u].flags = 0;
    assert_eq!(b4_mon::emerge_start(&mut f, u), 1);
    assert_eq!(f.c.units[u].flags, 0xE);
    // The do: action frame := 0; frame ≤ 0 stops the sequence.
    f.action.insert(u, 3);
    f.anim_frame.insert(u, 2 << 8);
    f.seq_speed.insert(u, 7);
    assert_eq!(b4_mon::submerge(&mut f, u), 1);
    assert_eq!((f.action[&u], f.seq_speed[&u]), (0, 7));
    f.anim_frame.insert(u, 0);
    assert_eq!(b4_mon::submerge(&mut f, u), 1);
    assert_eq!(f.seq_speed[&u], 0);
    // No R, T or E test: a unit without an entry works.
    let m = monster(&mut f, (1, 1));
    assert_eq!(b4_mon::submerge(&mut f, m), 1);
}

// ------------------------------------------------------------------

#[test]
fn fetish_aura_never_applies_its_state() {
    let mut r = body_rec();
    (r.param1, r.param2, r.param3, r.param4) = (10, 20, 5, 7);
    let t = tabs(r, Code::new(), 1);
    let ct = ctn(1);
    for hostile in [false, true] {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (10, 10));
        // A found unit of class 141 (a fetish), alive and flagged 0xE.
        let x = f.add(FUnit::new(UnitType::Monster, 141), (12, 12));
        f.c.units[x].flags = 0xE;
        f.found = vec![x];
        f.c.hostile = hostile;
        // The target position fails: 0, the flag stays clear.
        assert_eq!(b4_mon::fetish_aura(&mut f, &t, &ct, u, 1, 3), 0);
        assert_eq!(f.c.units[u].flags & 0x40, 0);
        f.tpos.insert(u, (20, 20));
        // An invalid skill: refused.
        assert_eq!(b4_mon::fetish_aura(&mut f, &t, &ct, u, 99, 3), 0);
        f.take_log();
        assert_eq!(b4_mon::fetish_aura(&mut f, &t, &ct, u, 1, 3), 1);
        assert_eq!(f.c.units[u].flags & 0x40, 0x40);
        // The two hostility tests exclude each other: no state, no list.
        // The find runs around the target with radius Param4 + 2L = 13
        // and the default filter 0x583.
        assert_eq!(*f.find_args.borrow(), [((20, 20), 13, 0x583)]);
        assert!(f.lists.is_empty(), "hostile {hostile}");
        assert!(!f.c.has_state(x, 25));
        assert!(!f.take_log().iter().any(|l| l.starts_with("changed")));
    }
}

// ------------------------------------------------------------------

const BX: [i32; 16] = [0, 1, 2, 2, 2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1];
const BY: [i32; 16] = [2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1, 0, 1, 2, 2];

fn nova_run(n: i16) -> BodyFake {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(4);
    r.calc2 = c.f(n);
    let mut t = tabs(r, c, 1);
    t.missiles[0].param1 = 3;
    t.missiles[0].param2 = 5;
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 20));
    assert_eq!(b4_mon::prime_poison_nova(&mut f, &t, u, 1, 6), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    f
}

#[test]
fn prime_poison_nova_rings() {
    // n = 4: ring 1 takes every second entry (velocity 3 << 6), ring 2
    // entries 1, 5, 9, 13 (velocity 5 << 6).
    let f = nova_run(4);
    assert_eq!(f.missiles.len(), 12);
    for (j, q) in f.missiles[..8].iter().enumerate() {
        let i = 2 * j;
        assert_eq!((q.target_x, q.target_y), (BX[i], BY[i]));
        assert_eq!(q.velocity, 192);
    }
    for (j, q) in f.missiles[8..].iter().enumerate() {
        let i = 4 * j + 1;
        assert_eq!((q.target_x, q.target_y), (BX[i], BY[i]));
        assert_eq!(q.velocity, 320);
    }
    for q in &f.missiles {
        // Edge case 2: skill field 0; the level is set.
        assert_eq!((q.flags, q.origin, q.class), (0x1F, Some(0), 0));
        assert_eq!((q.x, q.y, q.skill, q.level, q.loops), (10, 20, 0, 6, 4));
    }
    // n ≤ 1 (calc2 = 0 → max 1): no second ring.
    assert_eq!(nova_run(0).missiles.len(), 8);
    assert_eq!(nova_run(1).missiles.len(), 8);
    // n = 2: i = 0, 2, …, 14 (8 missiles) on offsets 1, 3, …, 15.
    let f = nova_run(2);
    assert_eq!(f.missiles.len(), 16);
    assert_eq!(f.missiles[15].target_x, BX[15]);
    // n = 20: only i = 0.
    let f = nova_run(20);
    assert_eq!(f.missiles.len(), 9);
    assert_eq!(f.missiles[8].target_y, BY[1]);
    // R invalid, or a missile without a record: 0.
    let mut r = body_rec();
    r.srvmissilea = 7;
    let t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 20));
    assert_eq!(b4_mon::prime_poison_nova(&mut f, &t, u, 1, 1), 0);
    assert_eq!(b4_mon::prime_poison_nova(&mut f, &t, u, 99, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn diab_light_is_srvdo_95() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc3 = c.f(3);
    let mut t = tabs(r, c, 1);
    // Test vector: Vel 10, VelLev 3, L = 5 → 11.
    t.missiles[0].vel = 10;
    t.missiles[0].vellev = 3;
    let ct = ctn(1);
    let run = |slot: u16| {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (10, 10));
        f.tpos.insert(u, (30, 40));
        assert_eq!(run_do(&mut f, &t, &ct, slot, u, 1, 5), Some(1));
        (f.missiles.clone(), f.take_log())
    };
    let (m95, l95) = run(95);
    let (m152, l152) = run(152);
    assert_eq!(m95, m152);
    assert_eq!(l95, l152);
    assert_eq!(m95.len(), 1);
    // Edge case 2: skill field 0, the level set, velocity 11.
    assert_eq!((m95[0].skill, m95[0].level, m95[0].velocity), (0, 5, 11));
}

// ------------------------------------------------------------------

#[test]
fn diab_cold_freezes_applies_and_overlays() {
    let run = |overlay: u16, life: i32, with_target: bool| {
        let mut r = hit_rec();
        r.srvoverlay = overlay;
        r.elen = 25;
        let t = tabs(r, Code::new(), 1);
        let ct = ctn(1);
        let mut f = BodyFake::new();
        let u = caster(&mut f, (0, 0));
        let k = monster(&mut f, (1, 0));
        f.c.set(k, 6, life);
        f.c.hostile = true;
        if with_target {
            f.targets.insert(u, k);
        }
        assert_eq!(b4_mon::diab_cold(&mut f, &t, &ct, u, 99, 1), 0, "R invalid");
        assert_eq!(f.c.units[u].flags & 0x40, 0);
        assert_eq!(b4_mon::diab_cold(&mut f, &t, &ct, u, 1, 1), 1);
        assert_eq!(f.c.units[u].flags & 0x40, 0x40);
        f
    };
    // No target: still 1, no reaction.
    let mut f = run(5, 100, false);
    assert!(f.c.last_reaction.is_none());
    assert!(!f.take_log().iter().any(|l| l.starts_with("overlay")));
    // A target at 0 life: the record carries 1 | ResultFlags | 2, the
    // hit flags and class, the freeze length; reaction and overlay.
    let mut f = run(5, 0, true);
    let r = f.c.last_reaction.unwrap();
    assert_eq!(r.result & 0x4003, 0x4003);
    assert_eq!(r.hit_flags & 0x82, 0x82);
    assert_eq!(r.hit_class, 5);
    assert_eq!(r.freeze_len, 25);
    let l = f.take_log();
    assert!(l.contains(&"reaction 0 1".to_string()));
    assert!(l.contains(&"overlay 1 5".to_string()));
    // A living target: the result has no bit 2.
    let f = run(5, 100, true);
    let r = f.c.last_reaction.unwrap();
    assert_eq!(r.result & 3, 1);
    // Overlay bounds: 0 and count + 1 refused, 1 and the count accepted.
    for (ov, shown) in [(0u16, false), (1, true), (200, true), (201, false)] {
        let mut f = run(ov, 100, true);
        let on = f.take_log().iter().any(|l| l.starts_with("overlay"));
        assert_eq!(on, shown, "overlay {ov}");
    }
}

// ------------------------------------------------------------------

#[test]
fn finger_mage_spider_offsets_to_the_target() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(9);
    let t = tabs(r, c, 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    let k = monster(&mut f, (14, 12));
    f.targets.insert(u, k);
    assert_eq!(b4_mon::finger_mage_spider(&mut f, &t, u, 1, 3), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    let q = f.missiles[0];
    assert_eq!((q.target_x, q.target_y), (4, 2));
    assert_eq!(
        (q.x, q.y, q.origin, q.skill, q.level),
        (10, 10, Some(u), 1, 3)
    );
    // Edge case 11: loops is set, flag 8 is not.
    assert_eq!((q.flags, q.loops), (3, 9));
    assert!(f.take_log().contains(&format!(
        "path {u} {:?}",
        PathOp::<usize>::TurnToward(14, 12)
    )));
    // No T: the path's target point.
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    f.path_target = (3, 4);
    assert_eq!(b4_mon::finger_mage_spider(&mut f, &t, u, 1, 3), 1);
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (-7, -6));
    // R invalid, no missile row: 0.
    assert_eq!(b4_mon::finger_mage_spider(&mut f, &t, u, 99, 3), 0);
    let mut r = body_rec();
    r.srvmissilea = 6;
    let t6 = tabs(r, Code::new(), 1);
    assert_eq!(b4_mon::finger_mage_spider(&mut f, &t6, u, 1, 3), 0);
}

// ------------------------------------------------------------------

struct Wall {
    frames: i32,
    tx: i32,
    seed: Seed,
    log: Vec<String>,
}

impl JitterMissile for Wall {
    type Missile = ();
    fn total_frames(&self, _: ()) -> i32 {
        self.frames
    }
    fn set_frames(&mut self, _: (), v: i32) {
        self.log.push(format!("frames {v}"));
    }
    fn path_target_x(&self, _: ()) -> i32 {
        self.tx
    }
    fn set_seed(&mut self, _: (), s: Seed) {
        self.seed = s;
    }
    fn set_path_type(&mut self, _: (), ty: u8) {
        self.log.push(format!("type {ty}"));
    }
    fn set_steps(&mut self, _: (), n: i32) {
        self.log.push(format!("steps {n}"));
    }
    fn compute_path(&mut self, _: ()) {
        self.log.push("compute".into());
    }
}

impl PathMissile for Wall {
    fn missile_seed(&mut self, _: ()) -> &mut Seed {
        &mut self.seed
    }
    fn missile_position(&self, _: ()) -> (i32, i32) {
        (0, 0)
    }
    fn path_target(&self, _: ()) -> (i32, i32) {
        (self.tx, 0)
    }
    fn missile_dir64(&self, _: (), _: (i32, i32)) -> i32 {
        0
    }
    fn set_point(&mut self, _: (), _: i32, _: (u16, u16)) {}
    fn set_point_count(&mut self, _: (), _: i32) {}
}

#[test]
fn diab_wall_creates_n_missiles_and_the_callback_leaves_a_fifth_straight() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(3);
    let t = tabs(r, c, 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    // The target position fails: 0.
    assert_eq!(b4_mon::diab_wall(&mut f, &t, u, 1, 4), 0);
    f.tpos.insert(u, (50, 60));
    assert_eq!(b4_mon::diab_wall(&mut f, &t, u, 99, 4), 0);
    assert_eq!(b4_mon::diab_wall(&mut f, &t, u, 1, 4), 1);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    assert_eq!(f.missiles.len(), 3);
    for (i, q) in f.missiles.iter().enumerate() {
        assert_eq!(q.init, Some((init_cb::DIAB_WALL, i as u32)));
        assert_eq!((q.flags, q.x, q.y), (0x21, 10, 10));
        assert_eq!((q.target_x, q.target_y), (50, 60));
        // Edge case 2: the level is set, the skill field stays 0.
        assert_eq!((q.skill, q.level, q.owner), (0, 4, u));
    }
    // The callback: frames capped at 77, seed := init_low(target x + a),
    // one draw `lo' mod 100`; r ≥ 20 → path type 10 over the frames.
    let draw = |a: u32| {
        let mut s = Seed::init_low(40u32.wrapping_add(a));
        s.step() % 100
    };
    let straight = (0..400u32).find(|&a| draw(a) < 20).unwrap();
    let bent = (0..400u32).find(|&a| draw(a) >= 20).unwrap();
    for (a, bends) in [(bent, true), (straight, false)] {
        let mut m = Wall {
            frames: 90,
            tx: 40,
            seed: Seed::new(0, 0),
            log: Vec::new(),
        };
        diab_wall_cb(&mut m, (), a);
        let mut s = Seed::init_low(40 + a);
        s.step();
        assert_eq!(m.seed, s, "one draw of the re-seeded RNG");
        let want: Vec<&str> = if bends {
            vec!["frames 77", "type 10", "steps 77", "compute"]
        } else {
            vec!["frames 77"]
        };
        assert_eq!(m.log, want, "a = {a}");
    }
    // About one wall missile in five stays straight.
    let n = (0..1000u32).filter(|&a| draw(a) < 20).count();
    assert!((120..280).contains(&n), "{n}");
    // Fewer than 78 frames: not capped.
    let mut m = Wall {
        frames: 30,
        tx: 40,
        seed: Seed::new(0, 0),
        log: Vec::new(),
    };
    diab_wall_cb(&mut m, (), bent);
    assert_eq!(m.log, ["type 10", "steps 30", "compute"]);
}

// -------------------------------------------------------------------------

#[test]
fn diab_run_start_stores_the_target() {
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let m = monster(&mut f, (1, 1));
    let e = ent(&f, u);
    // T none: 0; E none: 0.
    assert_eq!(b4_mon::diab_run_start(&mut f, u), 0);
    f.targets.insert(m, u);
    assert_eq!(b4_mon::diab_run_start(&mut f, m), 0);
    let k = monster(&mut f, (9, 8));
    f.targets.insert(u, k);
    f.set_entry_flags(u, &e, 3);
    f.take_log();
    assert_eq!(b4_mon::diab_run_start(&mut f, u), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
    assert_eq!(f.entry_param(u, &e, 1), k as i32, "GUID");
    assert_eq!(f.entry_param(u, &e, 2), 1, "type");
    let l = f.take_log();
    let (got, want) = path_lines_of(
        &l,
        u,
        &[PathOp::TargetUnit(None), PathOp::TargetPoint(9, 8)],
    );
    assert_eq!(got, want);
}

fn run_tabs() -> (SkillTables, CombatTables) {
    let mut c = Code::new();
    let mut r = hit_rec();
    r.param1 = 8;
    r.param2 = 14;
    r.param3 = 5;
    r.param4 = 13;
    r.param5 = 16;
    r.param6 = 6;
    r.calc1 = c.f(3);
    (tabs(r, c, 1), ctn(1))
}

#[test]
fn diab_run_do_loops_the_animation_and_hits_at_action_frame_1() {
    let (t, ct) = run_tabs();
    let mk = || {
        let mut f = BodyFake::new();
        let u = caster(&mut f, (0, 0));
        let k = monster(&mut f, (1, 0));
        let e = ent(&f, u);
        f.set_entry_param_of(u, &e, 1, k as i32);
        f.set_entry_param_of(u, &e, 2, 1);
        (f, u, k, e)
    };
    let (mut f, u, _, _) = mk();
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 99, 1), 0, "R invalid");
    let m = monster(&mut f, (4, 4));
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, m, 1, 1), 0, "E none");
    // The run ended: frame count Param1 << 8, event index Param2.
    let (mut f, u, _, e) = mk();
    f.set_entry_flags(u, &e, 2);
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_flags(u, &e), 0);
    assert_eq!((f.frame_count[&u], f.frame_index[&u]), (8 << 8, 14));
    // Frame Param3 (5): start running; velocity pct(max(calc1 << 8,
    // 0x100), velocitypercent, 100).
    let (mut f, u, _, e) = mk();
    f.anim_frame.insert(u, 5 << 8);
    f.c.set(u, 67, 150);
    f.take_log();
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.entry_flags(u, &e), 1);
    let l = f.take_log();
    let (got, want) = path_lines_of(
        &l,
        u,
        &[PathOp::Velocity(1152), PathOp::Type(1), PathOp::Compute],
    );
    assert_eq!(got, want);
    // The velocity floor 0x100 (calc1 = 0).
    let mut c = Code::new();
    let mut r = hit_rec();
    r.param3 = 5;
    r.calc1 = c.f(0);
    let t0 = tabs(r, c, 1);
    f.c.set(u, 67, 100);
    f.set_entry_flags(u, &e, 0);
    f.take_log();
    b4_mon::diab_run(&mut f, &t0, &ct, u, 1, 1);
    assert_eq!(
        lines(&mut f, "path")[0],
        format!("path {u} {:?}", PathOp::<usize>::Velocity(256))
    );
    // P none: 0.
    f.path = false;
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 0);
    // Frame Param4 (13): loop back.
    let (mut f, u, _, _) = mk();
    f.anim_frame.insert(u, 13 << 8);
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!((f.frame_count[&u], f.frame_index[&u]), (16 << 8, 6));
    // Action frame 1, K in reach: the hit lands.
    let (mut f, u, k, _) = mk();
    sure_hit(&mut f, u, k);
    f.action.insert(u, 1);
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.action[&u], 0);
    let r = f.c.last_reaction.unwrap();
    assert_eq!(r.result & 1, 1);
    assert_eq!(r.hit_flags & 0xA2, 0xA2, "HitFlags | 0x20");
    assert_eq!(r.hit_class, 5);
    assert!(f.take_log().contains(&format!("reaction {u} {k}")));
    // K dead (life 0) after the apply: result bit 2.
    let (mut f, u, k, _) = mk();
    sure_hit(&mut f, u, k);
    f.c.set(k, 6, 0);
    f.action.insert(u, 1);
    b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1);
    assert_eq!(f.c.last_reaction.unwrap().result & 2, 2);
    // Out of reach: the action frame stays 1, no hit.
    let (mut f, u, k, _) = mk();
    sure_hit(&mut f, u, k);
    f.c.in_range = false;
    f.action.insert(u, 1);
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 1);
    assert_eq!(f.action[&u], 1);
    assert!(f.c.last_reaction.is_none());
    // Any other frame, action frame 0: nothing happens.
    let (mut f, u, _, _) = mk();
    assert_eq!(b4_mon::diab_run(&mut f, &t, &ct, u, 1, 1), 1);
    assert!(f.c.last_reaction.is_none());
}

// ------------------------------------------------------------------

fn prison_world(summon: u16) -> (BodyFake, SkillTables, CombatTables, usize, usize) {
    let mut r = body_rec();
    r.summon = summon;
    r.summode = 3;
    let t = tabs(r, Code::new(), 1);
    let ct = ctn(344);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    let k = monster(&mut f, (20, 30));
    f.targets.insert(u, k);
    (f, t, ct, u, k)
}

#[test]
fn diab_prison_spawns_the_four_pieces_around_the_target() {
    // The four pieces: a leader at (+1, +1) (class 340) and the minions
    // of classes 341, 342, 343 at (+1, −1), (−1, −1), (−1, +1).
    let (mut f, t, ct, u, _) = prison_world(340);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 1);
    let l = f.take_log();
    let sp: Vec<_> = l
        .iter()
        .filter(|s| s.starts_with("Leader") || s.starts_with("Minion"))
        .cloned()
        .collect();
    assert_eq!(
        sp,
        [
            "Leader { room: 1, x: 21, y: 31, class: 340, mode: 8, spread: -1 }",
            "Minion { leader: 2, x: 21, y: 29, class: 341, mode: 8, spread: -1 }",
            "Minion { leader: 2, x: 19, y: 29, class: 342, mode: 8, spread: -1 }",
            "Minion { leader: 2, x: 19, y: 31, class: 343, mode: 8, spread: -1 }",
        ]
    );
    // No leader: no minions; still 1.
    let (mut f, t, ct, u, _) = prison_world(340);
    f.no_monsters = true;
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 1);
    assert!(!f.take_log().iter().any(|s| s.starts_with("Minion")));
    // A class other than 340: nothing spawned, 1.
    let (mut f, t, ct, u, _) = prison_world(341);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 1);
    assert!(!f
        .take_log()
        .iter()
        .any(|s| s.starts_with("Leader") || s.starts_with("Minion")));
    // R invalid, class < 0: 0.
    let (mut f, t, ct, u, _) = prison_world(340);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 99), 0);
    let (mut f, t, _, u, _) = prison_world(340);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ctn(2), u, 1), 0);
    // K in a town room: 0.
    let (mut f, t, ct, u, k) = prison_world(340);
    f.town.insert(f.room_of[&k]);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 0);
    // No T: P's target (GUID, 2) names an object K; any other y: 0.
    let (mut f, t, ct, u, _) = prison_world(340);
    f.targets.clear();
    let o = f.add(FUnit::new(UnitType::Object, 0), (40, 50));
    f.path_target = (o as i32, 2);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 1);
    assert!(f
        .take_log()
        .iter()
        .any(|s| s.starts_with("Leader { room: 1, x: 41, y: 51")));
    f.path_target = (o as i32, 3);
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 0);
    f.path = false;
    assert_eq!(b4_mon::diab_prison(&mut f, &t, &ct, u, 1), 0);
}

// ------------------------------------------------------------------

fn turret(t: (i32, i32), n: i16) -> BodyFake {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 0;
    r.calc1 = c.f(n);
    let tb = tabs(r, c, 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (100, 100));
    f.tpos.insert(u, t);
    let ret = b4_mon::desert_turret(&mut f, &tb, u, 1, 2);
    assert_eq!(ret, i32::from(t != (100, 100)));
    f
}

#[test]
fn desert_turret_sweeps_with_the_side_step() {
    // Worked example: u (100, 100), T (112, 106): dx 12, dy 6, step
    // (−2, 2); n = 4, h = 2 → (16, 2), then (14, 4), (12, 6), (10, 8);
    // the start uses the x quotient for both axes (Edge case 14).
    let f = turret((112, 106), 4);
    let got: Vec<_> = f
        .missiles
        .iter()
        .map(|q| (q.x, q.y, q.target_x, q.target_y))
        .collect();
    assert_eq!(
        got,
        [
            (102, 102, 16, 2),
            (102, 102, 14, 4),
            (102, 102, 12, 6),
            (101, 101, 10, 8)
        ]
    );
    for q in &f.missiles {
        assert_eq!((q.flags, q.skill, q.level, q.owner), (3, 0, 2, 0));
    }
    // n = 3: h = 1 → (14, 4), (12, 6), (10, 8).
    let f = turret((112, 106), 3);
    assert_eq!(f.missiles.len(), 3);
    assert_eq!((f.missiles[0].target_x, f.missiles[0].target_y), (14, 4));
    // The side step by the signs of (dx, dy), n = 2 (h = 1).
    #[rustfmt::skip]
    let table = [
        ((-12, -6), (2, -2)), ((-12, 0), (0, -2)), ((-12, 6), (-2, -2)),
        ((0, -6), (2, 0)),                         ((0, 6), (-2, 0)),
        ((12, -6), (2, 2)),   ((12, 0), (0, 2)),   ((12, 6), (-2, 2)),
    ];
    for ((dx, dy), (a, b)) in table {
        let f = turret((100 + dx, 100 + dy), 2);
        assert_eq!(f.missiles.len(), 2);
        let (tx, ty) = (dx - a, dy - b);
        let q = f.missiles[0];
        assert_eq!((q.target_x, q.target_y), (tx, ty), "({dx}, {dy})");
        let s = tx / 6;
        assert_eq!((q.x, q.y), (100 + s, 100 + s));
        let q = f.missiles[1];
        assert_eq!((q.target_x, q.target_y), (tx + a, ty + b));
    }
    // dx = dy = 0: 0; R invalid, no target position: 0.
    let f = turret((100, 100), 4);
    assert!(f.missiles.is_empty());
    let mut r = body_rec();
    r.srvmissilea = 0;
    let t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (100, 100));
    assert_eq!(b4_mon::desert_turret(&mut f, &t, u, 1, 1), 0);
    f.tpos.insert(u, (110, 110));
    assert_eq!(b4_mon::desert_turret(&mut f, &t, u, 99, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn arcane_tower_rings_at_the_leveled_velocity() {
    let mut r = body_rec();
    r.srvmissilea = 0;
    let mut t = tabs(r, Code::new(), 1);
    t.missiles[0].vel = 10;
    t.missiles[0].vellev = 3;
    let mut f = BodyFake::new();
    let u = caster(&mut f, (10, 10));
    let k = monster(&mut f, (20, 20));
    // T none: 0 (the flag is set first).
    assert_eq!(b4_mon::arcane_tower(&mut f, &t, u, 1, 5), 0);
    assert_eq!(f.c.units[u].flags & 0x40, 0x40);
    f.targets.insert(u, k);
    assert_eq!(b4_mon::arcane_tower(&mut f, &t, u, 1, 5), 1);
    // 64 missiles at 10 + trunc(3 × 5 / 8) = 11.
    assert_eq!(f.missiles.len(), 64);
    assert!(f.missiles.iter().all(|q| q.velocity == 11));
    assert!(f.missiles.iter().all(|q| (q.x, q.y) == (10, 10)));
    // R invalid, no missile row: 0.
    assert_eq!(b4_mon::arcane_tower(&mut f, &t, u, 99, 1), 0);
    let mut r = body_rec();
    r.srvmissilea = 4;
    let t4 = tabs(r, Code::new(), 1);
    assert_eq!(b4_mon::arcane_tower(&mut f, &t4, u, 1, 1), 0);
}

// ---------------------------------------------------------------- text, Edge case 10

#[test]
fn the_slots_of_section_5_have_bodies() {
    let t = tabs(body_rec(), Code::new(), 1);
    let ct = ctn(1);
    let (mut f, u) = world();
    for slot in [42u16, 43, 44, 45, 47, 48, 50, 51, 52, 54] {
        assert!(START_BODIES.contains(&slot), "srvst {slot}");
        assert!(run_start(&mut f, &t, &ct, slot, u, 1, 1).is_some());
    }
    for slot in [
        3u16, 4, 5, 83, 84, 86, 87, 88, 89, 90, 92, 93, 94, 99, 100, 101, 102, 103, 104, 105, 106,
        111, 152,
    ] {
        assert!(DO_BODIES.contains(&slot), "srvdo {slot}");
        assert!(run_do(&mut f, &t, &ct, slot, u, 1, 1).is_some());
    }
}

#[test]
fn srvdo_97_and_91_accept_overlay_0() {
    // srvdo 97: srvoverlay 0 → overlay on T (a corpse monster in mode 12).
    let mut r = body_rec();
    r.srvoverlay = 0;
    let t = tabs(r, Code::new(), 1);
    let ct = ctn(1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let k = monster(&mut f, (1, 0));
    f.c.units[k].mode = 12;
    f.targets.insert(u, k);
    assert_eq!(b4_mon::resurrect(&mut f, &t, &ct, u, 1), 1);
    assert!(f.take_log().contains(&format!("overlay {k} 0")));
    // srvdo 91: sumoverlay 0 → overlay on the new monster.
    let mut r = body_rec();
    r.sumoverlay = 0;
    let t = tabs(r, Code::new(), 1);
    let mut f = BodyFake::new();
    let u = caster(&mut f, (0, 0));
    let e = ent(&f, u);
    for (i, v) in [(1, 1), (2, 30), (3, 40), (4, 3)] {
        f.set_entry_param_of(u, &e, i, v);
    }
    assert_eq!(b4_mon::nest(&mut f, &t, u, 1), 1);
    assert!(f.take_log().contains(&"overlay 1 0".to_string()));
}
