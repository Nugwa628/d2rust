//! Tests of the shared helpers and the bodies used by several
//! monster skills of batch 4 on [`super::fake::BodyFake`].

use super::b4_helpers::*;
use super::b4_mon::*;
use super::fake::BodyFake;
use super::tests2::{body_rec, tabs, world, Code};
use super::*;
use crate::combat::CombatTables;
use crate::skills::fake::{blank, combat_tables, monster_rec, FItem, FUnit};
use crate::skills::use_::UseWorld;
use crate::skills::{SkillEntry, SkillUnits};
use crate::units::UnitType;
use d2_data::tables::Monstats2;

/// A monster caster of class `class` at `at` that uses skill 1.
fn caster(f: &mut BodyFake, at: (i32, i32), class: i32) -> usize {
    let mut m = FUnit::new(UnitType::Monster, class);
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

/// Combat tables of `n` monster rows; `ms2` is the shared monstats2 row.
fn ctabs(n: usize, ms2: Monstats2) -> CombatTables {
    let mut ct = combat_tables(vec![monster_rec(); n]);
    ct.monstats2 = vec![ms2];
    ct
}

fn count(log: &[String], what: &str) -> usize {
    log.iter().filter(|s| s.contains(what)).count()
}

// -----------------------------------------------------------------

#[test]
fn spawn_class_columns_skill_and_fallback() {
    let mut r = body_rec();
    r.summon = 1;
    r.summode = 7;
    let mut t = tabs(r, Code::new(), 1);
    let mut rows = vec![monster_rec(); 2];
    // Class 0: spawn 1, spawnx -2, spawny 3, spawnmode 5.
    rows[0].spawn = 1;
    rows[0].spawnx = 0xFE;
    rows[0].spawny = 3;
    rows[0].spawnmode = 5;
    // Class 1: spawn 0 (valid), spawnmode 20 (outside 0…15).
    rows[1].spawn = 0;
    rows[1].spawnmode = 20;
    let mut ct = combat_tables(rows);
    ct.monstats2 = vec![blank()];
    let (mut f, p) = world();
    f.pos.insert(p, (4, 6));
    let m0 = f.add(FUnit::new(UnitType::Monster, 0), (10, 10));
    let m1 = f.add(FUnit::new(UnitType::Monster, 1), (10, 10));
    let m7 = f.add(FUnit::new(UnitType::Monster, 7), (10, 10));
    // The monstats columns; outside 0…15 the mode is 1.
    assert_eq!(spawn_class(&f, &t, &ct, m0, 1), (1, 5, (8, 13)));
    assert_eq!(spawn_class(&f, &t, &ct, m1, 1), (0, 1, (10, 10)));
    // A monster class without a record → −1 (no AI-control class).
    assert_eq!(spawn_class(&f, &t, &ct, m7, 1).0, -1);
    // Any other unit takes the skill path, position = the unit's.
    assert_eq!(spawn_class(&f, &t, &ct, p, 1), (1, 7, (4, 6)));
    t.skills[1].summode = 0xF0;
    assert_eq!(spawn_class(&f, &t, &ct, p, 1).1, 1, "mode −16 → 1");
    // An invalid class falls back to the AI control's class with the
    // outputs unwritten (0); that class must be 1…count − 1.
    f.spawn_class = Some(1);
    assert_eq!(spawn_class(&f, &t, &ct, p, 999), (1, 0, (0, 0)));
    f.spawn_class = Some(0);
    assert_eq!(spawn_class(&f, &t, &ct, p, 999).0, -1);
    f.spawn_class = Some(2);
    assert_eq!(spawn_class(&f, &t, &ct, p, 999).0, -1);
    t.skills[1].summon = 5;
    assert_eq!(spawn_class(&f, &t, &ct, p, 1).0, -1, "summon ≥ count");
}

// -----------------------------------------------------------------

#[test]
fn inferno_channel_helpers() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(0);
    r.monanim = 14;
    let mut t = tabs(r, c, 3);
    t.missiles[2].param2 = 4;
    t.missiles[2].animlen = 16;
    let mut ms2: Monstats2 = blank();
    ms2.infernolen = 7;
    ms2.infernoanim = 5;
    let ct = ctabs(1, ms2);
    let (mut f, p) = world();
    f.c.frame = 100;
    let m = caster(&mut f, (0, 0), 0);
    // End: type-0 timers deleted, type-1 timer at F + InfernoLen; state 12
    // untouched.
    channel_end(&mut f, &ct, m);
    assert_eq!(f.take_log(), ["deltimers 1 0 0", "schedule 1 1 107 0 0"]);
    // A non-monster: 11.
    channel_end(&mut f, &ct, p);
    assert_eq!(f.take_log(), ["deltimers 0 0 0", "schedule 0 1 111 0 0"]);
    // A monster class without a record: 11.
    let lost = f.add(FUnit::new(UnitType::Monster, 9), (0, 0));
    channel_end(&mut f, &ct, lost);
    assert!(f.take_log().contains(&format!("schedule {lost} 1 111 0 0")));

    // Frames (test vector): calc1 0, Param2 4, L 3, AnimLen 16 → n = 6,
    // speed (16 << 8) / 6 = 682.
    let mk = f.add(FUnit::new(UnitType::Missile, 2), (0, 0));
    assert_eq!(channel_frames(&mut f, &t, m, Some(mk), 1, 3), 1);
    assert_eq!(f.anim_speed[&mk], 682);
    assert_eq!(f.mframes[&mk], (6, 6));
    assert!(f.take_log().contains(&format!("path {mk} Steps(6)")));
    // R invalid, M none, M without a missiles record → 0.
    assert_eq!(channel_frames(&mut f, &t, m, Some(mk), 99, 3), 0);
    assert_eq!(channel_frames(&mut f, &t, m, None, 1, 3), 0);
    let far = f.add(FUnit::new(UnitType::Missile, 50), (0, 0));
    assert_eq!(channel_frames(&mut f, &t, m, Some(far), 1, 3), 0);
    // n < 2 → 1; n > 255 → 255; speed clamped to 0x7FFF.
    let mut c2 = Code::new();
    let mut r2 = body_rec();
    r2.calc1 = c2.f(1);
    let mut t2 = tabs(r2, c2, 3);
    t2.missiles[2].animlen = 255;
    assert_eq!(channel_frames(&mut f, &t2, m, Some(mk), 1, 3), 1);
    assert_eq!(f.mframes[&mk], (1, 1));
    assert_eq!(f.anim_speed[&mk], 0x7FFF);
    let mut c3 = Code::new();
    let mut r3 = body_rec();
    r3.calc1 = c3.f(300);
    let mut t3 = tabs(r3, c3, 3);
    t3.missiles[2].animlen = 16;
    channel_frames(&mut f, &t3, m, Some(mk), 1, 3);
    assert_eq!(f.mframes[&mk], (255, 255));
    assert_eq!(f.anim_speed[&mk], (16 << 8) / 255);

    // Animation: a non-monster → event index 8, frame count 0x500.
    channel_anim(&mut f, &ct, &t.skills[1], p);
    assert_eq!((f.frame_index[&p], f.frame_count[&p]), (8, 0x500));
    // A monster without a monstats2 record: nothing.
    channel_anim(&mut f, &ct, &t.skills[1], lost);
    assert!(!f.frame_index.contains_key(&lost));
    // monanim 14: event index a − 1, frame count (a + 1) << 8.
    channel_anim(&mut f, &ct, &t.skills[1], m);
    assert_eq!((f.frame_index[&m], f.frame_count[&m]), (4, 6 << 8));
    // Otherwise the current frame := a << 8.
    let mut other = t.skills[1].clone();
    other.monanim = 3;
    channel_anim(&mut f, &ct, &other, m);
    assert_eq!(f.anim_frame[&m], 5 << 8);
}

// -----------------------------------------------------------------

/// A monster thrower with `types` item(s): `(loc 4 item, loc 5 item)`.
fn thrower(
    f: &mut BodyFake,
    loc4: Option<FItem>,
    loc5: Option<FItem>,
    in_use: Option<u8>,
) -> usize {
    let m = caster(f, (0, 0), 0);
    f.inventory = true;
    f.tpos.insert(m, (5, 5));
    for (loc, it) in [(4u8, loc4), (5u8, loc5)] {
        if let Some(it) = it {
            let i = f.c.add_item(it);
            f.c.units[m].items.insert(loc, i);
            if in_use == Some(loc) {
                f.c.units[m].weapon = Some(i);
            }
        }
    }
    m
}

fn throwing(types: Vec<i32>) -> FItem {
    FItem {
        types,
        throw: true,
        ..FItem::default()
    }
}

#[test]
fn throw_picks_the_weapon_and_throws() {
    let t = tabs(body_rec(), Code::new(), 1);
    // No inventory → 0.
    let (mut f, _) = world();
    let m = thrower(&mut f, Some(throwing(vec![45])), None, Some(4));
    f.inventory = false;
    assert_eq!(throw(&mut f, &t, m, 1, 1, false), 0);
    assert!(f.missiles.is_empty());
    // No weapon-type item → I none → 0.
    let (mut f, _) = world();
    let m = thrower(&mut f, Some(throwing(vec![12])), None, None);
    assert_eq!(throw(&mut f, &t, m, 1, 1, false), 0);
    // srvdo 3 (right) with the weapon in use: I = the picked item; a
    // throwable one is thrown (flags |= 0x40; straight; → 1).
    let (mut f, _) = world();
    let m = thrower(&mut f, Some(throwing(vec![45])), None, Some(4));
    assert_eq!(throw(&mut f, &t, m, 1, 4, false), 1);
    assert_ne!(f.unit_flags(m) & 0x40, 0);
    assert_eq!(f.missiles.len(), 1);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.class, q.skill, q.level), (0x21, 0, 1, 4));
    assert_eq!((q.target_x, q.target_y), (5, 5));
    // A missile potion (item type 38) is lobbed.
    let (mut f, _) = world();
    let m = thrower(&mut f, Some(throwing(vec![45, 38])), None, Some(4));
    assert_eq!(throw(&mut f, &t, m, 1, 1, false), 1);
    assert_eq!(f.missiles[0].flags, 0x420);
    assert_eq!(f.missiles[0].origin, Some(m));
    // Srvdo 3 swaps to the other location when the weapon is NOT in
    // use; srvdo 5 when it is. Loc 4 is a plain weapon, loc 5 throwable.
    let plain = FItem {
        types: vec![45],
        ..FItem::default()
    };
    for (left, in_use, thrown) in [
        (false, Some(4), false),
        (false, None, true),
        (true, Some(4), true),
        (true, None, false),
    ] {
        let (mut f, _) = world();
        let m = thrower(
            &mut f,
            Some(plain.clone()),
            Some(throwing(vec![45])),
            in_use,
        );
        let r = throw(&mut f, &t, m, 1, 1, left);
        assert_eq!(r, i32::from(thrown), "left {left} in_use {in_use:?}");
        assert_eq!(f.missiles.len(), usize::from(thrown));
        // The flag is only set after the checks passed.
        assert_eq!(f.unit_flags(m) & 0x40 != 0, thrown);
    }
    // The item_throwable(125) stat also accepts a non-throwable type.
    let (mut f, _) = world();
    let m = thrower(&mut f, Some(plain.clone()), None, Some(4));
    let i = f.c.units[m].items[&4];
    f.item_stats.insert((i, 125), 1);
    assert_eq!(throw(&mut f, &t, m, 1, 1, false), 1);
    // A player with no quantity left cannot throw.
    let (mut f, p) = world();
    f.inventory = true;
    f.tpos.insert(p, (5, 5));
    let i = f.c.add_item(throwing(vec![45]));
    f.c.units[p].items.insert(4, i);
    f.c.units[p].weapon = Some(i);
    assert_eq!(throw(&mut f, &t, p, 1, 1, false), 0);
    assert!(f.missiles.is_empty());
}

#[test]
fn potion_throw_without_a_weapon_link_consumes_only_successful_missiles() {
    let t = tabs(body_rec(), Code::new(), 1);
    let (mut f, player) = world();
    f.inventory = true;
    f.tpos.insert(player, (5, 5));
    let potion = f.c.add_item(throwing(vec![38]));
    f.c.units[player].items.insert(4, potion);
    let shield = f.c.add_item(FItem {
        types: vec![2],
        ..FItem::default()
    });
    f.c.units[player].items.insert(5, shield);
    f.item_stats.insert((potion, 70), 2);
    f.max_stack.insert(potion, 25);
    f.no_missiles = true;
    assert_eq!(throw(&mut f, &t, player, 1, 1, false), 0);
    assert_eq!(f.item_stats[&(potion, 70)], 2);
    f.no_missiles = false;
    assert_eq!(throw(&mut f, &t, player, 1, 1, false), 1);
    assert_eq!(f.item_stats[&(potion, 70)], 1);
    assert_eq!(f.missiles.last().unwrap().flags, 0x420);
    assert_eq!(throw(&mut f, &t, player, 1, 1, false), 1);
    assert_eq!(f.item_stats[&(potion, 70)], 0);
    assert!(f
        .take_log()
        .iter()
        .any(|line| line == &format!("ExhaustStack {{ owner: {player}, item: {potion} }}")));
    assert_eq!(throw(&mut f, &t, player, 1, 1, false), 0);
}

#[test]
fn throw_mastery_is_gated_on_the_used_skill() {
    let mut r = body_rec();
    r.range = 2;
    r.itypea1 = 48;
    let t = tabs(r, Code::new(), 1);
    let setup = |f: &mut BodyFake| {
        let m = thrower(f, Some(throwing(vec![45])), None, Some(4));
        // Throw mastery stats 345 / 346 (layer = item type 45), and the
        // plain mastery stats 342 / 343 that must not be read.
        f.c.units[m].entries.insert(345, vec![(12, 99), (45, 30)]);
        f.c.units[m].entries.insert(346, vec![(45, 40)]);
        f.c.units[m].entries.insert(342, vec![(45, 7)]);
        f.c.units[m].entries.insert(343, vec![(45, 8)]);
        m
    };
    // Gate passes: tohit(19) += 30, damagepercent(25) += 40.
    let (mut f, _) = world();
    let m = setup(&mut f);
    f.c.units[0].stats.clear();
    assert_eq!(throw(&mut f, &t, m, 1, 1, false), 1);
    let mk = f.c.units.len() - 1;
    assert_eq!(f.c.get(mk, 19), 30);
    assert_eq!(f.c.get(mk, 25), 40);
    // Range ≠ 2: both calls return 0 (no fall-through to 342 / 343).
    let mut t2 = t.clone();
    t2.skills[1].range = 1;
    let (mut f, _) = world();
    let m = setup(&mut f);
    assert_eq!(throw(&mut f, &t2, m, 1, 1, false), 1);
    let mk = f.c.units.len() - 1;
    assert_eq!((f.c.get(mk, 19), f.c.get(mk, 25)), (0, 0));
    // itypea1 not `thro`: 0 as well.
    let mut t3 = t.clone();
    t3.skills[1].itypea1 = 47;
    let (mut f, _) = world();
    let m = setup(&mut f);
    throw(&mut f, &t3, m, 1, 1, false);
    let mk = f.c.units.len() - 1;
    assert_eq!((f.c.get(mk, 19), f.c.get(mk, 25)), (0, 0));
    // The item is not a `throwable` type (accepted by stat 125): 0.
    let (mut f, _) = world();
    let m = setup(&mut f);
    let i = f.c.units[m].items[&4];
    f.c.items[i].throw = false;
    f.item_stats.insert((i, 125), 1);
    throw(&mut f, &t, m, 1, 1, false);
    let mk = f.c.units.len() - 1;
    assert_eq!((f.c.get(mk, 19), f.c.get(mk, 25)), (0, 0));
}

// -----------------------------------------------------------------

#[test]
fn next_event_sequence_and_anim_data() {
    let (mut f, u) = world();
    // With a sequence: n = 5 frames, current index c = 1; the event is
    // read at frame c << 8 on every step (Edge case 3).
    f.seq_frames = Some(5 << 8);
    f.frame_index.insert(u, 1);
    f.seq_events.insert(1 << 8, 7);
    f.seq_events.insert(2 << 8, 9);
    assert_eq!(next_event(&f, u), 7, "the byte of the current frame");
    f.seq_events.remove(&(1 << 8));
    assert_eq!(next_event(&f, u), 0, "a later frame's byte is never read");
    // c = n − 1: the loop does not run.
    f.seq_events.insert(4 << 8, 3);
    f.frame_index.insert(u, 4);
    assert_eq!(next_event(&f, u), 0);
    // Without a sequence: n = frame count >> 8, c = current frame >> 8.
    f.seq_frames = None;
    let mut ev = vec![0u8; 10];
    ev[1] = 5; // the current frame itself is skipped
    ev[3] = 4;
    ev[4] = 6;
    f.anim_data = Some((10, ev));
    f.frame_count.insert(u, 6 << 8);
    f.anim_frame.insert(u, 1 << 8);
    assert_eq!(next_event(&f, u), 4, "first non-zero byte after c");
    f.anim_frame.insert(u, 3 << 8);
    assert_eq!(next_event(&f, u), 6);
    f.anim_frame.insert(u, 4 << 8);
    assert_eq!(next_event(&f, u), 0);
    // n above the AnimData record's frame count → 0.
    f.anim_frame.insert(u, 1 << 8);
    f.frame_count.insert(u, 11 << 8);
    assert_eq!(next_event(&f, u), 0);
    // No AnimData record → 0.
    f.anim_data = None;
    f.frame_count.insert(u, 6 << 8);
    assert_eq!(next_event(&f, u), 0);
}

// -----------------------------------------------------------------

#[test]
fn revive_in_place() {
    let (mut f, p) = world();
    let m = f.add(FUnit::new(UnitType::Monster, 0), (3, 4));
    f.c.set(m, 7, 500);
    f.c.set(p, 7, 90);
    f.pos.insert(p, (6, 7));
    // No room → stop, nothing changes.
    f.room_of.remove(&m);
    revive(&mut f, m);
    assert_eq!(f.c.get(m, 6), 0);
    assert_eq!(f.unit_flags(m), 0);
    f.room_of.insert(m, 1);
    // Life := maxhp, flags |= 0xE, monster: no experience / drop,
    // then the calls in order.
    revive(&mut f, m);
    assert_eq!(f.c.get(m, 6), 500);
    assert_eq!(f.unit_flags(m), 0xE | 0x402_0000);
    let log = f.take_log();
    let at = |s: &str| log.iter().position(|l| l == s).unwrap_or(usize::MAX);
    let order = [
        format!("AiRefresh({m})"),
        format!("EvilCounterDec({m})"),
        format!("QuestChainLink {{ u: {m}, room: 1 }}"),
        format!("path {m} FootprintMask(256)"),
        format!("path {m} Reset"),
        format!("PatternStamp {{ room: 1, x: 3, y: 4, u: {m}, mask: 256 }}"),
    ];
    for w in order.windows(2) {
        assert!(at(&w[0]) < at(&w[1]), "{} before {}: {log:?}", w[0], w[1]);
    }
    assert!(log.contains(&order[0]), "{log:?}");
    // Another unit: mask 0x80, no monster flags, no AI calls.
    revive(&mut f, p);
    assert_eq!(f.c.get(p, 6), 90);
    assert_eq!(f.unit_flags(p), 0xE);
    let log = f.take_log();
    assert_eq!(
        log,
        [
            "set 0 6 90".to_string(),
            format!("path {p} FootprintMask(128)"),
            format!("path {p} Reset"),
            format!("PatternStamp {{ room: 1, x: 6, y: 7, u: {p}, mask: 128 }}"),
        ]
    );
}

// -----------------------------------------------------------------

#[test]
fn resurrect_mode_reads_monstats2() {
    let mut t = tabs(body_rec(), Code::new(), 1);
    t.skills.push(body_rec());
    t.skills.push(body_rec());
    let mut ms2: Monstats2 = blank();
    ms2.resurrectskill = 2;
    ms2.resurrectmode = 14;
    let mut ct = ctabs(2, ms2);
    let mut f = BodyFake::new();
    let mut m = FUnit::new(UnitType::Monster, 0);
    m.skills.push(SkillEntry {
        skill: 2,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    });
    let m = f.add(m, (0, 0));
    // f = 0 → 1 and nothing is read.
    assert_eq!(resurrect_mode(&mut f, &t, &ct, m, 0), 1);
    assert!(f.c.units[m].used.is_none());
    // The mode, and the used skill := the unit's entry of skill k.
    assert_eq!(resurrect_mode(&mut f, &t, &ct, m, 1), 14);
    assert_eq!(f.c.units[m].used.map(|e| e.skill), Some(2));
    // A mode of 16 or more → 1.
    ct.monstats2[0].resurrectmode = 16;
    assert_eq!(resurrect_mode(&mut f, &t, &ct, m, 1), 1);
    // k outside 1…skills count − 1 leaves the used skill alone.
    ct.monstats2[0].resurrectskill = 3;
    ct.monstats2[0].resurrectmode = 2;
    f.c.units[m].used = None;
    assert_eq!(resurrect_mode(&mut f, &t, &ct, m, 1), 2);
    assert!(f.c.units[m].used.is_none());
    // No monstats record → 1.
    let lost = f.add(FUnit::new(UnitType::Monster, 9), (0, 0));
    assert_eq!(resurrect_mode(&mut f, &t, &ct, lost, 1), 1);
}

// -----------------------------------------------------------------

#[test]
fn mon_inferno_start_sets_the_deadline_and_the_state() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc2 = c.f(5);
    r.calc1 = c.f(40);
    let t = tabs(r, c, 1);
    let (mut f, _) = world();
    f.c.frame = 100;
    let m = caster(&mut f, (0, 0), 0);
    // E param 1 := F + max(calc2, 1), state 12 on, no state list.
    assert_eq!(mon_inferno_start(&mut f, &t, m, 1, 3, false), 1);
    assert_eq!(f.entries[&(m, 1, 1)], 105);
    assert!(f.c.has_state(m, 12));
    assert!(f.take_log().iter().all(|s| !s.starts_with("list")));
    // The state is not turned on again.
    mon_inferno_start(&mut f, &t, m, 1, 3, false);
    assert_eq!(count(&f.take_log(), "state 1 12 true"), 0);
    // R invalid → 0; E none → 0.
    assert_eq!(mon_inferno_start(&mut f, &t, m, 99, 3, false), 0);
    f.c.units[m].used = None;
    assert_eq!(mon_inferno_start(&mut f, &t, m, 1, 3, false), 0);
    // calc2 ≤ 0 → F + 1.
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc2 = c.f(-4);
    let t0 = tabs(r, c, 1);
    let (mut f, _) = world();
    f.c.frame = 100;
    let m = caster(&mut f, (0, 0), 0);
    mon_inferno_start(&mut f, &t0, m, 1, 3, false);
    assert_eq!(f.entries[&(m, 1, 1)], 101);
}

// -----------------------------------------------------------------

#[test]
fn mon_inferno_makes_a_missile_and_rearms_or_ends() {
    let mut c = Code::new();
    let mut r = body_rec();
    r.srvmissilea = 1;
    r.calc1 = c.f(0);
    r.calc3 = c.f(4);
    r.monanim = 3;
    let mut t = tabs(r, c, 3);
    t.missiles[1].vel = 10;
    t.missiles[1].vellev = 3;
    t.missiles[1].param2 = 4;
    t.missiles[1].animlen = 16;
    let mut ms2: Monstats2 = blank();
    ms2.infernolen = 7;
    ms2.infernoanim = 5;
    let ct = ctabs(1, ms2);
    let setup = || {
        let (mut f, _) = world();
        f.c.frame = 100;
        let m = caster(&mut f, (6, 8), 0);
        f.tpos.insert(m, (20, 30));
        f.entries.insert((m, 1, 1), 120);
        f.c.units[m].states.push(12);
        (f, m)
    };
    // The deadline is ahead and state 12 is on: one missile, next event
    // at F + calc3.
    let (mut f, m) = setup();
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 1);
    let q = f.missiles[0];
    assert_eq!(q.flags, 0x25);
    assert_eq!((q.owner, q.class, q.level, q.skill), (m, 1, 5, 0));
    assert_eq!((q.x, q.y, q.target_x, q.target_y), (6, 8, 20, 30));
    // Velocity: Vel 10 + trunc(VelLev 3 x L 5 / 8) = 11.
    assert_eq!(q.velocity, 11);
    // Frames: calc1 0 -> n = Param2 + L - 1 = 8; speed (16 << 8) / 8.
    let mk = f.c.units.len() - 1;
    assert_eq!(f.mframes[&mk], (8, 8));
    assert_eq!(f.anim_speed[&mk], 512);
    // Animation: monanim != 14 -> current frame := InfernoAnim << 8.
    assert_eq!(f.anim_frame[&m], 5 << 8);
    let log = f.take_log();
    let tail: Vec<&String> = log
        .iter()
        .filter(|s| s.starts_with("deltimers") || s.starts_with("schedule"))
        .collect();
    assert_eq!(
        tail,
        ["deltimers 1 1 0", "deltimers 1 0 0", "schedule 1 0 104 0 0"]
    );
    // The deadline reached: the channel ends (type-1 timer at F + 7).
    let (mut f, m) = setup();
    f.entries.insert((m, 1, 1), 100);
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 1);
    assert_eq!(f.missiles.len(), 1);
    let log = f.take_log();
    assert!(log.contains(&"schedule 1 1 107 0 0".to_string()), "{log:?}");
    assert!(!log.iter().any(|s| s.starts_with("schedule 1 0 ")));
    // State 12 gone: the channel ends.
    let (mut f, m) = setup();
    f.c.units[m].states.clear();
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 1);
    assert!(f.take_log().contains(&"schedule 1 1 107 0 0".to_string()));
    // No target position: channel end, 0, no missile.
    let (mut f, m) = setup();
    f.tpos.clear();
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 0);
    assert!(f.missiles.is_empty());
    assert!(f.take_log().contains(&"schedule 1 1 107 0 0".to_string()));
    // The missile is not created: channel end, 0.
    let (mut f, m) = setup();
    f.no_missiles = true;
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 0);
    assert!(f.take_log().contains(&"schedule 1 1 107 0 0".to_string()));
    // R invalid, E none, srvmissilea outside 0..count - 1: 0 and nothing.
    let (mut f, m) = setup();
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 99, 5), 0);
    f.c.units[m].used = None;
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 0);
    let (mut f, m) = setup();
    t.skills[1].srvmissilea = 3;
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 0);
    t.skills[1].srvmissilea = 0xFFFF;
    assert_eq!(mon_inferno(&mut f, &t, &ct, m, 1, 5), 0);
    assert!(f.take_log().iter().all(|s| !s.starts_with("schedule")));
}

// -----------------------------------------------------------------

#[test]
fn mon_teleport_places_the_unit() {
    let (mut f, _) = world();
    let m = caster(&mut f, (0, 0), 0);
    f.path_target = (20, 21);
    assert_eq!(mon_teleport(&mut f, m), 1);
    let log = f.take_log();
    assert!(
        log.contains(&format!("place {m} Some(1) (20, 21)")),
        "{log:?}"
    );
    assert!(log.contains(&format!("update {m}")));
    assert_eq!(f.unit_c8(m), 0x1_0000);
    // No room at the point: the minion owner's room; without an owner
    // none.
    let (mut f, o) = world();
    let m = caster(&mut f, (0, 0), 0);
    f.path_target = (20, 21);
    f.point_rooms.insert((20, 21), None);
    f.minion_owner.insert(m, o);
    f.room_of.insert(o, 7);
    assert_eq!(mon_teleport(&mut f, m), 1);
    assert!(f
        .take_log()
        .contains(&format!("place {m} Some(7) (20, 21)")));
    f.minion_owner.clear();
    mon_teleport(&mut f, m);
    assert!(f.take_log().contains(&format!("place {m} None (20, 21)")));
    // The placement fails: 0, the unit is not queued and not flagged.
    let (mut f, _) = world();
    let m = caster(&mut f, (0, 0), 0);
    f.path_target = (20, 21);
    f.place_fails = true;
    assert_eq!(mon_teleport(&mut f, m), 0);
    assert!(f.take_log().iter().all(|s| !s.starts_with("update")));
    assert_eq!(f.unit_c8(m), 0);
}

// -----------------------------------------------------------------

#[test]
fn scroll_book_uses_the_first_matching_item() {
    let (mut f, _) = world();
    let m = caster(&mut f, (0, 0), 0);
    let item = |f: &mut BodyFake, ty: i32, row: (i32, i32), qty: i32| {
        let i = f.c.add_item(FItem {
            types: vec![ty],
            ..FItem::default()
        });
        f.books.insert(i, row);
        f.item_stats.insert((i, 70), qty);
        i
    };
    // No inventory -> 0.
    f.inventory = false;
    assert_eq!(scroll_book(&mut f, m, 5), 0);
    f.inventory = true;
    // Book rows are (scrollskill, bookskill).
    let empty_book = item(&mut f, 18, (0, 5), 0);
    let wrong_book = item(&mut f, 18, (5, 6), 3);
    let scroll = item(&mut f, 22, (5, 0), 0);
    let book = item(&mut f, 18, (0, 5), 2);
    let other = item(&mut f, 22, (6, 0), 0);
    // A book with quantity 0 and a book whose bookskill differs are
    // skipped (the scrollskill of a book is not read); the first
    // matching scroll is used, kind 1, no quantity test.
    f.inv_nodes = vec![(empty_book, 1), (wrong_book, 1), (scroll, 1), (book, 2)];
    assert_eq!(scroll_book(&mut f, m, 5), 1);
    let log = f.take_log();
    assert_eq!(count(&log, "UseItem"), 1);
    assert!(log.contains(&format!("UseItem {{ u: {m}, item: {scroll}, kind: 1 }}")));
    // A book with quantity > 0 and kind 2 (section 7.17).
    f.inv_nodes = vec![(other, 1), (empty_book, 1), (book, 2), (scroll, 1)];
    assert_eq!(scroll_book(&mut f, m, 5), 1);
    assert!(f
        .take_log()
        .contains(&format!("UseItem {{ u: {m}, item: {book}, kind: 2 }}")));
    // A match in a node kind other than 1 or 2 returns 0 without looking
    // further (Edge case 8).
    f.inv_nodes = vec![(scroll, 3), (book, 1)];
    assert_eq!(scroll_book(&mut f, m, 5), 0);
    assert_eq!(count(&f.take_log(), "UseItem"), 0);
    // No match -> 0.
    f.inv_nodes = vec![(other, 1), (empty_book, 1)];
    assert_eq!(scroll_book(&mut f, m, 5), 0);
    assert_eq!(scroll_book(&mut f, m, 7), 0);
}

// -----------------------------------------------------------------

#[test]
fn hireable_missile_arrow_bolt_and_special_missiles() {
    let mut r = body_rec();
    r.srvmissilea = 2;
    let mut t = tabs(r, Code::new(), 45);
    let setup = || {
        let (mut f, p) = world();
        let m = caster(&mut f, (1, 1), 0);
        let tg = f.add(FUnit::new(UnitType::Monster, 0), (5, 5));
        f.targets.insert(m, tg);
        (f, m, p)
    };
    // m = 0 (arrow): srvmissilea, straight (R `lob` unset), quantity 0.
    let (mut f, m, _) = setup();
    assert_eq!(hireable_missile(&mut f, &t, m, 1, 4), 1);
    assert_ne!(f.unit_flags(m) & 0x40, 0);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.class, q.skill, q.level), (0x21, 2, 1, 4));
    assert_eq!((q.target_x, q.target_y), (5, 5));
    // R `lob` -> lobbed.
    t.skills[1].lob = true;
    let (mut f, m, _) = setup();
    assert_eq!(hireable_missile(&mut f, &t, m, 1, 4), 1);
    assert_eq!((f.missiles[0].flags, f.missiles[0].class), (0x420, 2));
    // Magic arrow (stat 157): m = 27, L := the arrow stat; straight even
    // with `lob`. Exploding arrow (158): m = 41.
    let (mut f, m, _) = setup();
    f.c.units[m].stats.insert((157, 0), 6);
    assert_eq!(hireable_missile(&mut f, &t, m, 1, 4), 1);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.class, q.level), (0x21, 27, 6));
    let (mut f, m, _) = setup();
    f.c.units[m].stats.insert((158, 0), 2);
    hireable_missile(&mut f, &t, m, 1, 4);
    assert_eq!((f.missiles[0].class, f.missiles[0].level), (41, 2));
    // m without a missiles record -> 0 (magic arrow row beyond the table).
    let mut small = t.clone();
    small.missiles.truncate(10);
    let (mut f, m, _) = setup();
    f.c.units[m].stats.insert((157, 0), 6);
    assert_eq!(hireable_missile(&mut f, &small, m, 1, 4), 0);
    assert!(f.missiles.is_empty());
    // A player without a bow / crossbow class: m = -1 -> 0.
    let (mut f, _, p) = setup();
    f.hand_class = 0;
    let tg = f.add(FUnit::new(UnitType::Monster, 0), (5, 5));
    f.targets.insert(p, tg);
    assert_eq!(hireable_missile(&mut f, &t, p, 1, 4), 0);
    // A crossbow player: m = 31 -> srvmissilea.
    f.hand_class = 7;
    assert_eq!(hireable_missile(&mut f, &t, p, 1, 4), 1);
    assert_eq!(f.missiles[0].class, 2);
    // srvmissilea outside 0..count - 1 -> 0 (the flag is already set).
    let mut bad = t.clone();
    bad.skills[1].srvmissilea = 45;
    let (mut f, m, _) = setup();
    assert_eq!(hireable_missile(&mut f, &bad, m, 1, 4), 0);
    assert_ne!(f.unit_flags(m) & 0x40, 0);
    // R invalid -> 0; T none -> 0 without the flag.
    let (mut f, m, _) = setup();
    assert_eq!(hireable_missile(&mut f, &t, m, 99, 4), 0);
    f.targets.clear();
    assert_eq!(hireable_missile(&mut f, &t, m, 1, 4), 0);
    assert_eq!(f.unit_flags(m), 0);
}

// -----------------------------------------------------------------------

#[test]
fn nest_start_and_do_spawn_through_the_entry_params() {
    let mut r = body_rec();
    r.sumoverlay = 9;
    let t = tabs(r, Code::new(), 1);
    let mut rows = vec![monster_rec(); 3];
    rows[0].spawn = 1;
    rows[0].spawnx = 2;
    rows[0].spawny = 3;
    rows[0].spawnmode = 4;
    rows[2].spawn = 0xFFFF;
    let mut ct = combat_tables(rows);
    ct.monstats2 = vec![blank()];
    let (mut f, _) = world();
    let m = caster(&mut f, (10, 10), 0);
    // 4.6: E params 1..4 := c, x, y, mode; pattern stamp; uninterruptable;
    // type-1 timers deleted.
    assert_eq!(nest_start(&mut f, &t, &ct, m, 1), 1);
    assert_eq!([1, 2, 3, 4].map(|i| f.entries[&(m, 1, i)]), [1, 12, 13, 4]);
    assert!(f.c.has_state(m, 54));
    let log = f.take_log();
    let at = |s: &str| log.iter().position(|l| l.contains(s)).unwrap_or(usize::MAX);
    assert!(
        at("PatternStampN { room: 1, x: 12, y: 13, pattern: 1, mask: 256 }")
            < at("state 1 54 true")
    );
    assert!(at("state 1 54 true") < at("deltimers 1 1 0"), "{log:?}");
    // c < 0 -> 0 and nothing is written; R invalid / E none -> 0.
    let bad = caster(&mut f, (10, 10), 2);
    assert_eq!(nest_start(&mut f, &t, &ct, bad, 1), 0);
    assert!(!f.entries.contains_key(&(bad, 1, 1)));
    assert_eq!(nest_start(&mut f, &t, &ct, m, 99), 0);
    f.c.units[bad].used = None;
    assert_eq!(nest_start(&mut f, &t, &ct, bad, 1), 0);

    // 4.7: the spawn from the params.
    f.take_log();
    assert_eq!(nest(&mut f, &t, m, 1), 1);
    let log = f.take_log();
    let at = |s: &str| log.iter().position(|l| l.contains(s)).unwrap_or(usize::MAX);
    assert!(
        at("deltimers 1 1 0")
            < at("PatternClearN { room: 1, x: 12, y: 13, pattern: 1, mask: 256 }")
    );
    assert!(log.iter().any(|l| l.contains("state 1 54 false")));
    assert!(log.iter().any(
        |l| l.contains("At { room: 1, x: 12, y: 13, class: 1, mode: 4, spread: 1, flags: 0 }")
    ));
    let spawned = f.c.units.len() - 1;
    assert_eq!(f.unit_flags(spawned), 0x402_0000);
    assert!(log.contains(&format!("overlay {spawned} 9")));
    // sumoverlay outside 0..count - 1: no overlay.
    let mut t2 = t.clone();
    t2.skills[1].sumoverlay = 200;
    nest(&mut f, &t2, m, 1);
    assert!(f.take_log().iter().all(|l| !l.starts_with("overlay")));
    // x = 0 or y = 0 -> 0 (after the timers and the state change).
    f.entries.insert((m, 1, 2), 0);
    assert_eq!(nest(&mut f, &t, m, 1), 0);
    let log = f.take_log();
    assert!(log.iter().any(|l| l.contains("deltimers 1 1 0")));
    assert!(log.iter().all(|l| !l.contains("PatternClearN")));
    f.entries.insert((m, 1, 2), 12);
    // No room at (x, y) -> 0; no monster -> 0; R invalid / E none -> 0.
    f.point_rooms.insert((12, 13), None);
    assert_eq!(nest(&mut f, &t, m, 1), 0);
    f.point_rooms.clear();
    f.no_monsters = true;
    assert_eq!(nest(&mut f, &t, m, 1), 0);
    assert_eq!(nest(&mut f, &t, m, 99), 0);
    f.c.units[m].used = None;
    assert_eq!(nest(&mut f, &t, m, 1), 0);
}

// -----------------------------------------------------------------------

/// Caster `m` (a monster), its minion owner `o`, target `tg`.
fn cycler_world() -> (BodyFake, usize, usize, usize) {
    let (mut f, o) = world();
    let m = caster(&mut f, (2, 2), 0);
    let tg = f.add(FUnit::new(UnitType::Monster, 0), (9, 9));
    f.minion_owner.insert(m, o);
    f.targets.insert(m, tg);
    (f, m, o, tg)
}

#[test]
fn corpse_cycler_and_family_bolt_missile_rows() {
    let mut r = body_rec();
    r.srvmissilea = 2;
    let mut t = tabs(r, Code::new(), 6);
    // srvst 63: the missile belongs to the owner and starts on the corpse.
    let (mut f, m, o, tg) = cycler_world();
    assert_eq!(corpse_cycler(&mut f, &t, m, 1, 3), 1);
    assert!(f.c.has_state(tg, 118));
    let q = f.missiles[0];
    assert_eq!(
        (q.flags, q.owner, q.origin, q.class, q.skill, q.level),
        (0, o, Some(tg), 2, 1, 3)
    );
    let log = f.take_log();
    assert!(log.contains(&format!("update {tg}")));
    assert!(log.contains(&format!(
        "MsgA3 {{ u: {m}, target: Some({tg}), skill: 1, lvl: 1, x: 0, y: 0, v: 0 }}"
    )));
    // The corpse is already marked -> 0, nothing made.
    assert_eq!(corpse_cycler(&mut f, &t, m, 1, 3), 0);
    assert_eq!(f.missiles.len(), 1);
    // Refusals: not a monster, no owner, no target, R invalid.
    let (mut f, m, o, _) = cycler_world();
    assert_eq!(corpse_cycler(&mut f, &t, o, 1, 3), 0);
    assert_eq!(corpse_cycler(&mut f, &t, m, 99, 3), 0);
    f.targets.clear();
    assert_eq!(corpse_cycler(&mut f, &t, m, 1, 3), 0);
    let (mut f, m, _, _) = cycler_world();
    f.minion_owner.clear();
    assert_eq!(corpse_cycler(&mut f, &t, m, 1, 3), 0);
    // Edge case 7: srvst 63 refuses srvmissilea = 0 and >= count; srvdo 85
    // accepts 0 and adds the chain position.
    for bad in [0, 6, 0xFFFF] {
        t.skills[1].srvmissilea = bad;
        let (mut f, m, _, _) = cycler_world();
        assert_eq!(corpse_cycler(&mut f, &t, m, 1, 3), 0, "srvmissilea {bad}");
        assert!(f.missiles.is_empty());
    }
    t.skills[1].srvmissilea = 1;
    let (mut f, m, _, _) = cycler_world();
    assert_eq!(corpse_cycler(&mut f, &t, m, 1, 3), 1);

    // srvdo 85: m = srvmissilea + the chain position of the unit's class.
    t.skills[1].srvmissilea = 2;
    let (mut f, m, _, _) = cycler_world();
    f.chains.insert(0, 1);
    assert_eq!(family_bolt(&mut f, &t, m, 1, 4), 1);
    assert_ne!(f.unit_flags(m) & 0x40, 0);
    let q = f.missiles[0];
    assert_eq!((q.flags, q.class, q.skill, q.level), (0x21, 3, 1, 4));
    // 0 is accepted.
    t.skills[1].srvmissilea = 0;
    f.chains.insert(0, 0);
    assert_eq!(family_bolt(&mut f, &t, m, 1, 4), 1);
    assert_eq!(f.missiles[1].class, 0);
    // m >= count -> 0 (the flag was set by step 2); m0 outside -> 0 before
    // the flag.
    t.skills[1].srvmissilea = 4;
    f.chains.insert(0, 2);
    assert_eq!(family_bolt(&mut f, &t, m, 1, 4), 0);
    assert_eq!(f.missiles.len(), 2);
    let (mut f, m, _, _) = cycler_world();
    t.skills[1].srvmissilea = 6;
    assert_eq!(family_bolt(&mut f, &t, m, 1, 4), 0);
    assert_eq!(f.unit_flags(m), 0);
    assert_eq!(family_bolt(&mut f, &t, m, 99, 4), 0);
}

// ------------------------------------------------------------------

#[test]
fn zakarum_heal_rolls_a_percent_of_the_maximum() {
    let heal = |c1: i16, c2: i16, life: i32, max: i32| {
        let mut c = Code::new();
        let mut r = body_rec();
        r.calc1 = c.f(c1);
        r.calc2 = c.f(c2);
        let t = tabs(r, c, 1);
        let (mut f, _) = world();
        let m = caster(&mut f, (0, 0), 0);
        let tg = f.add(FUnit::new(UnitType::Monster, 0), (3, 3));
        f.targets.insert(m, tg);
        f.c.set(tg, 6, life);
        f.c.set(tg, 7, max);
        let before = f.c.units[m].seed;
        let r = zakarum_heal(&mut f, &t, m, 1, 1);
        (f, m, tg, before, r)
    };
    // hi 60, lo 80: lo = 60, no draw, p = 60 -> 600 of 1000.
    let (f, m, tg, before, r) = heal(80, 60, 0, 1000);
    assert_eq!(r, 1);
    assert_eq!(f.c.units[m].seed, before, "no draw when hi = lo");
    assert_eq!(f.c.get(tg, 6), 600);
    assert_ne!(f.unit_flags(m) & 0x40, 0);
    // M = 1000, life 0, p 0 -> stat 6 = 1.
    let (f, _, tg, _, _) = heal(0, 0, 0, 1000);
    assert_eq!(f.c.get(tg, 6), 1);
    // lo 10, hi 30: p = 10 + roll(20) from the unit's seed.
    let (f, m, tg, before, _) = heal(10, 30, 100, 1000);
    let mut s = before;
    let p = 10 + s.roll(20) as i32;
    assert_eq!(f.c.units[m].seed, s);
    assert_eq!(f.c.get(tg, 6), 100 + 1000 * p / 100);
    // hi >= 100 -> 100, a negative lo -> 0: p = roll(100).
    let (f, _, tg, before, _) = heal(-5, 250, 0, 1000);
    let mut s = before;
    let p = s.roll(100) as i32;
    assert_eq!(f.c.get(tg, 6), (1000 * p / 100).max(1));
    // The result never exceeds the maximum.
    let (f, _, tg, _, _) = heal(50, 50, 990, 1000);
    assert_eq!(f.c.get(tg, 6), 1000);
    // T none -> 0 without the flag; R invalid -> 0.
    let (mut f, m, _, _, _) = heal(0, 0, 0, 1000);
    f.set_unit_flags(m, 0);
    f.targets.clear();
    let t = tabs(body_rec(), Code::new(), 1);
    assert_eq!(zakarum_heal(&mut f, &t, m, 1, 1), 0);
    assert_eq!(f.unit_flags(m), 0);
    assert_eq!(zakarum_heal(&mut f, &t, m, 99, 1), 0);
}

// ------------------------------------------------------------------

#[test]
fn resurrect_revives_the_target_in_place() {
    let mut r = body_rec();
    r.srvoverlay = 5;
    let mut t = tabs(r, Code::new(), 1);
    t.skills.push(body_rec());
    let mut ms2: Monstats2 = blank();
    ms2.resurrectmode = 3;
    ms2.resurrectskill = 2;
    let ct = ctabs(1, ms2);
    let setup = || {
        let (mut f, _) = world();
        let m = caster(&mut f, (0, 0), 0);
        let tg = f.add(FUnit::new(UnitType::Monster, 0), (4, 4));
        f.c.units[tg].mode = 12;
        f.c.set(tg, 7, 200);
        f.targets.insert(m, tg);
        (f, m, tg)
    };
    let (mut f, m, tg) = setup();
    assert_eq!(resurrect(&mut f, &t, &ct, m, 1), 1);
    assert_ne!(f.unit_flags(m) & 0x40, 0);
    assert_eq!(f.c.get(tg, 6), 200, "life restored to maxhp");
    assert_eq!(f.unit_flags(tg), 0xE | 0x402_0000);
    let log = f.take_log();
    let at = |s: &str| log.iter().position(|l| l.contains(s)).unwrap_or(usize::MAX);
    let order = [
        format!("path {m} TargetUnit(Some({tg}))"),
        format!("AiRefresh({tg})"),
        format!("ModeRequestBuild {{ m: {tg}, mode: 3 }}"),
        format!("ModeRequestSend {{ m: {tg}, mode: 3, flag: true }}"),
        format!("overlay {tg} 5"),
    ];
    for w in order.windows(2) {
        assert!(at(&w[0]) < at(&w[1]), "{} before {}: {log:?}", w[0], w[1]);
    }
    // resurrect_mode runs twice (the used skill is set each time) and the
    // request flag is m != 14 (Edge case 4).
    assert_eq!(
        count(&log, &format!("used {tg} Some(2)")),
        0,
        "T has no entry"
    );
    let (mut f, m, tg) = setup();
    f.c.units[tg].skills.push(SkillEntry {
        skill: 2,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    });
    resurrect(&mut f, &t, &ct, m, 1);
    assert_eq!(count(&f.take_log(), &format!("used {tg} Some(2)")), 2);
    let mut ct14 = ct.clone();
    ct14.monstats2[0].resurrectmode = 14;
    let (mut f, m, tg) = setup();
    assert_eq!(resurrect(&mut f, &t, &ct14, m, 1), 1);
    let log = f.take_log();
    assert!(log.contains(&format!("ModeRequestBuild {{ m: {tg}, mode: 14 }}")));
    assert!(log.contains(&format!(
        "ModeRequestSend {{ m: {tg}, mode: 14, flag: false }}"
    )));
    // Mode 0 is accepted as well; other cases are refused (flag set, no
    // change).
    let (mut f, m, tg) = setup();
    f.c.units[tg].mode = 0;
    assert_eq!(resurrect(&mut f, &t, &ct, m, 1), 1);
    for refuse in 0..6 {
        let (mut f, m, tg) = setup();
        match refuse {
            0 => f.c.units[tg].mode = 1,
            1 => f.c.units[tg].kind = UnitType::Player,
            2 => {
                f.state_flags.insert((70, 2));
                f.c.units[tg].states.push(70);
            }
            3 => {
                f.room_of.remove(&tg);
            }
            4 => f.collides = true,
            _ => {}
        }
        let want = i32::from(refuse == 5);
        assert_eq!(resurrect(&mut f, &t, &ct, m, 1), want, "case {refuse}");
        if want == 0 {
            assert_eq!(f.c.get(tg, 6), 0);
            assert_ne!(f.unit_flags(m) & 0x40, 0);
        }
    }
    // R invalid -> 0; T none -> 0 without the flag.
    let (mut f, m, _) = setup();
    assert_eq!(resurrect(&mut f, &t, &ct, m, 99), 0);
    f.targets.clear();
    assert_eq!(resurrect(&mut f, &t, &ct, m, 1), 0);
    assert_eq!(f.unit_flags(m), 0);
    // srvoverlay outside 0..count - 1 -> no overlay; 0 is accepted.
    let mut t0 = t.clone();
    t0.skills[1].srvoverlay = 0;
    let (mut f, m, tg) = setup();
    resurrect(&mut f, &t0, &ct, m, 1);
    assert!(f.take_log().contains(&format!("overlay {tg} 0")));
    t0.skills[1].srvoverlay = 200;
    let (mut f, m, _) = setup();
    resurrect(&mut f, &t0, &ct, m, 1);
    assert!(f.take_log().iter().all(|l| !l.starts_with("overlay")));
}

// ---------------------------------------------------------------- 3.4, 4.12, 4.13

/// A monster `m` (class 0) with a target `tg` in melee range, hostile;
/// `calc2` gives 37.
fn swing_world() -> (BodyFake, usize, usize, SkillTables, CombatTables) {
    let mut c = Code::new();
    let mut r = body_rec();
    r.calc1 = c.f(11);
    r.calc2 = c.f(37);
    r.hitflags = 0x10;
    r.hitclass = 3;
    r.aurastate = 40;
    r.auralencalc = c.f(50);
    r.srvstfunc = 64;
    let t = tabs(r, c, 1);
    let ct = ctabs(1, blank());
    let (mut f, _) = world();
    let m = caster(&mut f, (0, 0), 0);
    let tg = f.add(FUnit::new(UnitType::Monster, 0), (1, 1));
    f.targets.insert(m, tg);
    f.c.hostile = true;
    f.c.in_range = true;
    f.c.set(m, 19, 1000);
    f.c.set(m, 12, 20);
    f.c.set(tg, 12, 20);
    (f, m, tg, t, ct)
}

#[test]
fn mon_swing_hit_and_miss() {
    // A hit (some seed): calc2 -> enhanced damage, hit flags / class
    // filled, E param 1 := 1.
    let mut hit = None;
    for lo in 1..400u32 {
        let (mut f, m, tg, t, ct) = swing_world();
        f.c.units[m].seed = crate::rng::Seed::new(lo, 0);
        // A dead attacker (mode 0): apply_melee returns at once and the
        // combat record stays readable (section 5.2 step 4.1).
        f.c.units[m].mode = 0;
        assert_eq!(mon_swing(&mut f, &t, &ct, m, Some(tg), 1, 3), 1);
        if f.entries[&(m, 1, 1)] == 1 {
            hit = Some((f, m, tg));
            break;
        }
    }
    let (mut f, m, tg) = hit.expect("a hitting seed");
    let log = f.take_log();
    assert!(
        log.contains(&format!("path {m} TargetUnit(Some({tg}))")),
        "{log:?}"
    );
    let e = &f.c.units[m].combat;
    assert_eq!(e.len(), 1, "start_combat stored the record");
    let rec = e[0].record;
    assert_eq!(rec.enh_pct, 37);
    assert_eq!(rec.hit_flags & 0x10, 0x10);
    assert_eq!(rec.hit_class, 3);
    assert_ne!(rec.result & 1, 0);
    // A miss (out of melee range): result 0, E param 1 := 0, returns 1.
    let (mut f, m, tg, t, ct) = swing_world();
    f.c.in_range = false;
    f.entries.insert((m, 1, 1), 9);
    assert_eq!(mon_swing(&mut f, &t, &ct, m, Some(tg), 1, 3), 1);
    assert_eq!(f.entries[&(m, 1, 1)], 0);
    assert!(f.c.units[m].combat.iter().all(|e| e.record.enh_pct == 0));
    // R invalid or T none -> 0 and E untouched.
    f.entries.insert((m, 1, 1), 9);
    assert_eq!(mon_swing(&mut f, &t, &ct, m, Some(tg), 99, 3), 0);
    assert_eq!(mon_swing(&mut f, &t, &ct, m, None, 1, 3), 0);
    assert_eq!(f.entries[&(m, 1, 1)], 9);
}

#[test]
fn mon_frenzy_start_needs_a_target() {
    let (mut f, m, _, _, _) = swing_world();
    assert_eq!(mon_frenzy_start(&f, m), 1);
    f.targets.clear();
    assert_eq!(mon_frenzy_start(&f, m), 0);
}

#[test]
fn mon_frenzy_swings_and_charges_on_a_hit() {
    // E none -> 0; T none -> 0.
    let (mut f, m, _, t, ct) = swing_world();
    f.targets.clear();
    assert_eq!(mon_frenzy(&mut f, &t, &ct, m, 1, 3), 0);
    assert_eq!(f.unit_flags(m), 0);
    let (mut f, m, _, t, ct) = swing_world();
    f.c.units[m].used = None;
    assert_eq!(mon_frenzy(&mut f, &t, &ct, m, 1, 3), 0);
    // A hit charges Frenzy (state 40 on); a miss does not. No later
    // action event: no start core.
    let mut seen = [false; 2];
    for lo in 1..400u32 {
        let (mut f, m, _, t, ct) = swing_world();
        f.c.units[m].seed = crate::rng::Seed::new(lo, 0);
        assert_eq!(mon_frenzy(&mut f, &t, &ct, m, 1, 3), 1);
        assert_ne!(f.unit_flags(m) & 0x40, 0);
        let hit = f.entries[&(m, 1, 1)] != 0;
        assert_eq!(f.c.has_state(m, 40), hit, "seed {lo}");
        assert_eq!(count(&f.take_log(), "srvst 64"), 0);
        seen[usize::from(hit)] = true;
        if seen == [true, true] {
            break;
        }
    }
    assert_eq!(seen, [true, true], "both a hit and a miss were seen");
    // A later action event remains: the start core runs first (srvst 64).
    let (mut f, m, _, t, ct) = swing_world();
    f.seq_frames = Some(5 << 8);
    f.frame_index.insert(m, 1);
    f.seq_events.insert(1 << 8, 7);
    assert_eq!(mon_frenzy(&mut f, &t, &ct, m, 1, 3), 1);
    assert_eq!(count(&f.take_log(), &format!("srvst 64 {m} 1 3")), 1);
}
