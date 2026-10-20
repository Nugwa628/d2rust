//! The monster mode message in the client pass of the action wiring
//! with the path provider on: a kill sends 0x69 code 8 in the next
//! client pass, the end of the death animation sets mode 12 and sends
//! code 9, the room clean-up makes each one a single message.

use std::sync::Arc;

use d2_formats::animdata::{self, AnimData, AnimRecord};

use super::super::reaction::kill;
use super::super::tests::Fx;
use super::*;
use crate::units::lists::client_state;

const DEATH: &[u8; 8] = b"M0DTHTH\0";
/// Frames of the fixture's death animation (24 frames between code 8 and
/// code 9).
const DEATH_FRAMES: u32 = 24;

fn anim_data() -> AnimData {
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    let len = DEATH.iter().position(|&b| b == 0).unwrap();
    a.buckets[animdata::hash(&DEATH[..len])].push(AnimRecord {
        name: *DEATH,
        frames: DEATH_FRAMES,
        speed: 256,
        events: [0; animdata::EVENTS],
    });
    a
}

/// The fixture with the path provider on, a player with a client in
/// room A and a monster (class 0) next to it; one tick has run and the
/// messages it sent are dropped.
fn setup() -> (Fx, UnitId, UnitId) {
    let mut fx = Fx::new();
    let h = fx.sim.hooks();
    h.enable_paths().expect("embedded tables");
    h.anim_data = Some(Arc::new(anim_data()));
    h.x.names.insert((UnitType::Monster, 0), *DEATH);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 1, a, 10, 10);
    let m = fx.spawn(UnitType::Monster, 0, a, 13, 10);
    // The client joins room A through the room switch of its first
    // per-client update, which fills the rooms' client arrays.
    fx.game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.tick();
    fx.sim.hooks().x.sent.clear();
    (fx, p, m)
}

fn kill_now(fx: &mut Fx, m: UnitId, p: UnitId) {
    let game = &mut fx.game;
    fx.sim.combat(game, |cv, _| kill(cv, m, p));
}

fn guid(fx: &Fx, u: UnitId) -> u32 {
    fx.game.lists.unit(u).unwrap().guid
}

fn path(fx: &mut Fx, u: UnitId) -> crate::path::DynamicPath {
    fx.sim
        .hooks()
        .paths
        .as_ref()
        .unwrap()
        .dynamic(u)
        .unwrap()
        .clone()
}

fn sent(fx: &mut Fx) -> Vec<(UnitId, Vec<u8>)> {
    std::mem::take(&mut fx.sim.hooks().x.sent)
}

/// The death clean-up's direction snap toward the killer. Recorded in
/// Blood Moor (`traces/checks/combat-kill-fallen.check`, state channel): a
/// fallen walking with d = 0 is killed at (5145, 4265) by the player at
/// (5143, 4263) and has d = 32 from frame 36 on, through mode 12. The
/// same (−2, −2) geometry here.
#[test]
fn the_dead_monster_faces_its_killer() {
    let mut fx = Fx::new();
    let h = fx.sim.hooks();
    h.enable_paths().expect("embedded tables");
    h.anim_data = Some(Arc::new(anim_data()));
    h.x.names.insert((UnitType::Monster, 0), *DEATH);
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 1, a, 10, 10);
    let m = fx.spawn(UnitType::Monster, 0, a, 12, 12);
    assert_eq!(path(&mut fx, m).direction, 0);
    kill_now(&mut fx, m, p);
    for _ in 0..30 {
        fx.tick();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 12);
    assert_eq!(path(&mut fx, m).direction, 32);
    fx.assert_clean();
}

/// The kill sets mode 0 with flag 0x1, so the
/// next client pass sends 0x69 code 8 at the path target with d = the
/// path direction (snapped toward the killer by the death clean-up) and
/// e = the unit's hit class (0 here: [`Pending::unit_b0`]'s
/// default); the clean-up then clears flag 0x1 and nothing more is sent
/// until event 1 of the DT animation (f + 24 for 24 frames at
/// speed 256) sets mode 12, whose message (code 9, the unit's cell,
/// e = 0) goes out in that same tick.
#[test]
fn kill_sends_code_8_then_the_death_end_code_9() {
    let (mut fx, p, m) = setup();
    {
        let d = fx
            .sim
            .hooks()
            .paths
            .as_mut()
            .unwrap()
            .dynamic_mut(m)
            .unwrap();
        d.direction = 0x38;
        d.target_x = 0x1295;
        d.target_y = 0x1555;
    }
    let f = fx.game.frame;
    kill_now(&mut fx, m, p);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 0);
    let g = guid(&fx, m);
    fx.tick();
    let mut want = vec![0x69];
    want.extend(g.to_le_bytes());
    // d: the death clean-up snapped the path direction (0x38 before) to
    // the kill's direction toward the killer: from (13, 10) to (10, 10).
    // (a, b): the path end, (0, 0) for a path with no points; never the
    // target point set above.
    want.extend([0x08, 0x00, 0x00, 0x00, 0x00, 0x17, 0x00]);
    assert_eq!(sent(&mut fx), vec![(p, want)]);
    assert_eq!(
        fx.sim.sys.units.get(m).unwrap().flags & (flags::CHANGED | flags::MODE_CHANGING),
        0
    );
    // Event 1 at f + 24: nothing before it.
    let end = f + DEATH_FRAMES as i32;
    while fx.game.frame < end - 1 {
        fx.tick();
        assert_eq!(sent(&mut fx), vec![], "frame {}", fx.game.frame);
    }
    fx.tick();
    assert_eq!(fx.game.frame, end);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 12);
    let d = path(&mut fx, m);
    let mut want = vec![0x69];
    want.extend(g.to_le_bytes());
    want.push(0x09);
    want.extend((d.x() as u16).to_le_bytes());
    want.extend((d.y() as u16).to_le_bytes());
    want.extend([0x17, 0x00]);
    assert_eq!(sent(&mut fx), vec![(p, want)]);
    for _ in 0..30 {
        fx.tick();
        assert_eq!(sent(&mut fx), vec![]);
    }
    fx.assert_clean();
}

/// Event 1 of DT runs unit event 13 after the mode set; `SplEndDeath` 0
/// runs no death action.
#[test]
fn death_end_runs_unit_event_13() {
    let (mut fx, p, m) = setup();
    kill_now(&mut fx, m, p);
    for _ in 0..DEATH_FRAMES {
        fx.tick();
    }
    let log = &fx.sim.hooks().x.log;
    let ev = format!("event 13 Some({})", m.0);
    assert_eq!(log.iter().filter(|l| **l == ev).count(), 1, "{log:?}");
    assert!(
        !log.iter().any(|l| l.starts_with("death action")),
        "{log:?}"
    );
}

/// A monster of base id 78 steps and sets mode 12 only when its
/// animation is complete; every other monster at once.
#[test]
fn anim_complete_reads_frame_speed_and_count() {
    let mut a = Anim {
        frame: 0x500,
        frame_count: 0x600,
        speed: 0x100,
        ..Anim::default()
    };
    assert!(anim_complete(&a));
    a.speed = 0xFF;
    assert!(!anim_complete(&a));
    a.sequence = Some(crate::units::record::Sequence {
        frame_count: 0,
        speed: 0,
        pos: 0,
        events: Vec::new(),
        drawn: Vec::new(),
    });
    a.frame_count = 0;
    assert!(anim_complete(&a));
    a.frame_count = 1;
    assert!(!anim_complete(&a));
}

/// The room clean-up clears the listed unit flags and flag-ex bits, and
/// the per-type flag bits.
#[test]
fn room_cleanup_clears_the_listed_flags() {
    let (mut fx, p, m) = setup();
    for u in [p, m] {
        let r = fx.sim.sys.units.get_mut(u).unwrap();
        r.flags = u32::MAX;
        r.flags2 = u32::MAX;
    }
    if let Some(d) = fx.sim.hooks().paths.as_mut().unwrap().dynamic_mut(m) {
        d.flags = u32::MAX;
    }
    let s = &mut fx.sim.sys;
    let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
    v.room_cleanup(p);
    v.room_cleanup(m);
    let rp = fx.sim.sys.units.get(p).unwrap();
    assert_eq!(rp.flags, !(0x1 | 0x10 | 0x400 | 0x8000 | 0x100));
    assert_eq!(rp.flags2, !(0x800 | 0x1000 | 0x10000 | 0x200000));
    let rm = fx.sim.sys.units.get(m).unwrap();
    assert_eq!(rm.flags, !(0x1 | 0x10 | 0x400 | 0x8000 | 0x100 | 0x800));
    assert_eq!(rm.flags2, !(0x800 | 0x1000 | 0x10000 | 0x200000));
    assert_eq!(path(&mut fx, m).flags, !0x2);
}

/// The room clean-up zeroes the state-changed bits and sets a player's
/// stat 29 `lastexp` := −1.
#[test]
fn room_cleanup_resets_changed_states_and_lastexp() {
    let (mut fx, p, _) = setup();
    fx.sim.sys.stats.set_state_changed(p, 1, true);
    let s = &mut fx.sim.sys;
    let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
    v.room_cleanup(p);
    let (_, changed) = fx.sim.sys.stats.state_bits(p).expect("state bits");
    assert!(changed.iter().all(|&w| w == 0), "{changed:?}");
    assert_eq!(fx.sim.sys.stats.unit_base(p, 29, 0), -1);
}

/// With the provider on and no path record, the mode
/// message is the fatal 0xE6 (logged, nothing sent).
#[test]
fn a_monster_without_a_path_is_the_fatal() {
    let (mut fx, p, m) = setup();
    fx.sim.hooks().paths.as_mut().unwrap().records.remove(&m);
    fx.sim.sys.units.get_mut(m).unwrap().flags |= flags::CHANGED;
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    assert_eq!(sent(&mut fx), vec![]);
    assert_eq!(
        fx.sim.hooks().errors,
        vec![WiringError::ModeMessage(ModeMessageError::NoPath(m))]
    );
    let _ = p;
}

/// A path target unit that is gone is cleared by the
/// refresh and gives no target (0x69 code 6 of mode 3 to the cell, not
/// 0x6A); one in a room the client is in is kept (0x6A for mode 6).
#[test]
fn the_target_is_refreshed_and_tested_against_the_client_rooms() {
    let (mut fx, p, m) = setup();
    let gp = guid(&fx, p);
    let gm = guid(&fx, m);
    let set_target = |fx: &mut Fx, unit: UnitId, guid: u32| {
        let d = fx
            .sim
            .hooks()
            .paths
            .as_mut()
            .unwrap()
            .dynamic_mut(m)
            .unwrap();
        d.target_unit = Some(crate::path::TargetUnit {
            unit,
            ty: UnitType::Player,
            guid,
        });
    };
    // Mode 6 with the player as target: the player's room holds the client.
    set_target(&mut fx, p, gp);
    fx.sim.sys.units.get_mut(m).unwrap().mode = 6;
    fx.sim.sys.units.get_mut(m).unwrap().flags |= flags::CHANGED;
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    let mut want = vec![0x6A];
    want.extend(gm.to_le_bytes());
    want.extend([18, 0]);
    want.extend(gp.to_le_bytes());
    want.push(0);
    assert_eq!(sent(&mut fx), vec![(p, want)]);
    // A stale GUID: cleared, mode 6 without a target sends (0, −1).
    set_target(&mut fx, p, gp + 100);
    fx.sim.sys.units.get_mut(m).unwrap().flags |= flags::CHANGED;
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    let mut want = vec![0x69];
    want.extend(gm.to_le_bytes());
    want.extend([18, 0, 0, 0xFF, 0xFF, 0, 0]);
    assert_eq!(sent(&mut fx), vec![(p, want)]);
    assert_eq!(path(&mut fx, m).target_unit, None);
    fx.assert_clean();
}

/// Flag-ex 0x10000 sends S→C 0x15 (type 1, GUID,
/// the path's cell, flag 1), then the room-change messages (nothing
/// without path flag 0x2); then unit flag 0x100 with no
/// hover text sends 0x76. The 0x15 comes first. The room
/// clean-up clears both bits, so the next pass sends nothing.
#[test]
fn reassign_and_overhead_steps_of_the_monster_update() {
    let (mut fx, p, m) = setup();
    let g = guid(&fx, m).to_le_bytes();
    {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.flags |= flags::HOVER_FREED;
        r.flags2 |= 0x1_0000;
    }
    fx.game.lists.queue_update(m).unwrap();
    let d = path(&mut fx, m);
    fx.tick();
    let mut reassign = vec![0x15, 1, g[0], g[1], g[2], g[3]];
    reassign.extend((d.x() as u16).to_le_bytes());
    reassign.extend((d.y() as u16).to_le_bytes());
    reassign.push(1);
    let overhead = vec![0x76, 1, g[0], g[1], g[2], g[3]];
    assert_eq!(sent(&mut fx), vec![(p, reassign), (p, overhead)]);
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    assert_eq!(sent(&mut fx), vec![]);
    fx.assert_clean();
}

/// Perturbation of the test above: the same bits on a monster that is
/// not queued for update send nothing (the client pass walks the update
/// queues).
#[test]
fn unqueued_monster_bits_send_nothing() {
    let (mut fx, _, m) = setup();
    {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.flags |= flags::HOVER_FREED;
        r.flags2 |= 0x1_0000;
    }
    fx.tick();
    assert_eq!(sent(&mut fx), vec![]);
}

/// The overhead message: no hover → 0x76; after
/// `replace_overhead` (text "yo", byte +8 = 4) the overhead chat 0x26
/// form 5 carries the kept record (a player the receiver has no
/// relation to).
#[test]
fn overhead_message_sends_the_kept_record() {
    let (mut fx, p, _) = setup();
    let g = guid(&fx, p);
    fx.sim.sys.units.get_mut(p).unwrap().hover = None;
    let s = &mut fx.sim.sys;
    let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
    v.overhead_message(p, p, 0, g);
    v.replace_overhead(p, b"yo", 4, 77);
    v.overhead_message(p, p, 0, g);
    assert_eq!(fx.sim.sys.units.get(p).unwrap().hover, Some(77));
    let gb = g.to_le_bytes();
    assert_eq!(
        sent(&mut fx),
        vec![
            (p, vec![0x76, 0, gb[0], gb[1], gb[2], gb[3]]),
            (p, crate::units::messages::overhead_chat(4, 0, g, b"yo")),
        ]
    );
}

/// The environment report: the act's
/// record advances once per tick; the 2176th tick reports and the
/// in-game client of that act gets `53 02000000 80080000 00`.
#[test]
fn environment_report_sends_0x53_to_the_act_s_clients() {
    let (mut fx, p, _) = setup();
    let act = fx.game.lists.room(fx.a).unwrap().act;
    // Not built by a join: no advance (the setup tick left it untouched).
    assert_eq!(
        fx.game.lists.act(act).unwrap().environment,
        crate::world::environment::Environment::CREATED
    );
    fx.game.lists.act_mut(act).unwrap().built = true;
    fx.tick();
    let env = |fx: &mut Fx| {
        let s = std::mem::take(&mut fx.sim.hooks().x.sent);
        s.into_iter()
            .filter(|(_, m)| m[0] == 0x53)
            .collect::<Vec<_>>()
    };
    for _ in 1..2175 {
        fx.tick();
    }
    assert_eq!(env(&mut fx), vec![]);
    fx.tick();
    assert_eq!(
        env(&mut fx),
        vec![(p, vec![0x53, 2, 0, 0, 0, 0x80, 0x08, 0, 0, 0])]
    );
    assert_eq!(fx.game.lists.act(act).unwrap().environment.last_hour, 17);
}

// ---- skill messages 0x4C / 0x4D and the hit message 0x0C ---------------------------

fn use_skill(fx: &mut Fx, u: UnitId, skill: i32, base: i32) {
    fx.sim.hooks().x.used.insert(
        u,
        crate::skills::SkillEntry {
            skill,
            base,
            level_bonus: 1,
            owner_guid: -1,
            charges: 0,
            has_charges: false,
        },
    );
}

fn target(fx: &mut Fx, u: UnitId, t: UnitId, ty: UnitType) {
    let guid = guid(fx, t);
    let d = fx
        .sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(u)
        .unwrap();
    d.target_unit = Some(crate::path::TargetUnit { unit: t, ty, guid });
    d.target_x = 0x1234;
    d.target_y = 0x0567;
}

fn path_target(fx: &mut Fx, u: UnitId, t: (u16, u16)) {
    let d = fx
        .sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(u)
        .unwrap();
    d.target_unit = None;
    d.target_x = t.0;
    d.target_y = t.1;
}

fn changed(fx: &mut Fx, u: UnitId, mode: u32) {
    let r = fx.sim.sys.units.get_mut(u).unwrap();
    r.mode = mode;
    r.flags |= flags::CHANGED;
    fx.game.lists.queue_update(u).unwrap();
}

/// The skill message builders (flag 0): 0x4C's 16 bytes and 0x4D's 17,
/// at the message layout offsets.
#[test]
fn skill_message_builders_write_the_tsv_offsets() {
    use skill_message::{skill_on_point, skill_on_unit};
    assert_eq!(
        skill_on_unit(1, 0x0403_0201, 0x0A0B, 3, 0, 0x0D0C_0B0A, 0),
        [0x4C, 1, 1, 2, 3, 4, 0x0B, 0x0A, 3, 0, 0x0A, 0x0B, 0x0C, 0x0D, 0, 0]
    );
    assert_eq!(
        skill_on_point(0, 7, 0x1_0002, 5, 0x1234, 0x0567, 0),
        [0x4D, 0, 7, 0, 0, 0, 2, 0, 1, 0, 5, 0x34, 0x12, 0x67, 0x05, 0, 0]
    );
}

/// A monster with a skill in use and its target in the
/// client's rooms sends 0x4C (type 1, GUID, skill, level, target type
/// and GUID) instead of a mode message; nothing is logged.
#[test]
fn a_monster_using_a_skill_on_its_target_sends_0x4c() {
    let (mut fx, p, m) = setup();
    use_skill(&mut fx, m, 0x2F, 2);
    target(&mut fx, m, p, UnitType::Player);
    changed(&mut fx, m, 4);
    fx.tick();
    // Level byte: skill 0x2F has no record in the test tables, so
    // `skill_level` adds no bonus: base 2.
    let want = skill_message::skill_on_unit(1, guid(&fx, m), 0x2F, 2, 0, guid(&fx, p), 0);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())]);
    assert_eq!(fx.sim.hooks().errors, vec![]);
}

/// No target (mode 14 SQ drops none, but the path has no
/// target unit): 0x4D at the path target.
#[test]
fn a_monster_using_a_skill_without_a_target_sends_0x4d() {
    let (mut fx, p, m) = setup();
    use_skill(&mut fx, m, 0x2F, 0);
    target(&mut fx, m, p, UnitType::Player);
    fx.sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(m)
        .unwrap()
        .target_unit = None;
    changed(&mut fx, m, 14);
    fx.tick();
    let want = skill_message::skill_on_point(1, guid(&fx, m), 0x2F, 0, 0x1234, 0x0567, 0);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())]);
    assert_eq!(fx.sim.hooks().errors, vec![]);
}

/// Unit flag 0x8000 sends 0x0C (type 1, GUID, 0x13,
/// the hit class, the life fraction minus 1) and stores the fraction as stat
/// 352; the clean-up clears the flag, so it is sent once.
#[test]
fn a_hit_monster_sends_0x0c_once() {
    let (mut fx, p, m) = setup();
    // Stat 7 `maxhp` 100, stat 6 `hitpoints` 50 (both << 8).
    let max = 100 << 8;
    fx.stats(m, &[(7, max), (6, max / 2)]);
    let p128 = crate::stats::life_fraction(max / 2, max);
    assert!(p128 < 0x80, "max life {max}");
    fx.sim.sys.units.get_mut(m).unwrap().flags |= HIT;
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    let want = skill_message::monster_hit(guid(&fx, m), 0, p128 as u8, false);
    assert_eq!(want[6], 0x13);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())]);
    assert_eq!(fx.sim.sys.stats.unit_base(m, 352, 0), p128);
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    assert_eq!(sent(&mut fx), vec![]);
}

/// The 0x0C life byte: p − 1 above 1, else p; | 0x80 with the 0x100 flag.
#[test]
fn monster_hit_life_byte() {
    assert_eq!(skill_message::monster_hit(9, 4, 0x80, false)[8], 0x7F);
    assert_eq!(skill_message::monster_hit(9, 4, 1, false)[8], 1);
    assert_eq!(skill_message::monster_hit(9, 4, 0, true)[8], 0x80);
    assert_eq!(skill_message::monster_hit(9, 4, 0x40, true)[7..], [4, 0xBF]);
}

/// A player entering a skill
/// mode (A1) with a used skill sends its own client nothing; with E
/// flags bit 0x4
/// (dodge / avoid) it sends 0x4C on its path's target unit, else 0x4D
/// at the path target; a walk mode still sends nothing to it. Block
/// sends the 0x0D stop row.
#[test]
fn a_player_attacking_a_monster_sends_0x4c() {
    let (mut fx, p, m) = setup();
    use_skill(&mut fx, p, 0, 1);
    target(&mut fx, p, m, UnitType::Monster);
    changed(&mut fx, p, 7);
    fx.tick();
    assert_eq!(sent(&mut fx), vec![], "no E flag 0x4");
    fx.sim.hooks().x.entry_flags.insert(p, 0x4);
    changed(&mut fx, p, 7);
    fx.tick();
    let want = skill_message::skill_on_unit(0, guid(&fx, p), 0, 2, 1, guid(&fx, m), 0);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())]);
    // Without a target unit: 0x4D at the path target.
    fx.sim
        .hooks()
        .paths
        .as_mut()
        .unwrap()
        .dynamic_mut(p)
        .unwrap()
        .target_unit = None;
    changed(&mut fx, p, 10);
    fx.tick();
    let want = skill_message::skill_on_point(0, guid(&fx, p), 0, 2, 0x1234, 0x0567, 0);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())]);
    // Neutral: nothing to the own client.
    changed(&mut fx, p, 1);
    fx.tick();
    assert_eq!(sent(&mut fx), vec![], "mode 1");
    // Block (BL 9): 0x0D code 0x12 to every client, own included.
    changed(&mut fx, p, 9);
    fx.tick();
    let g = guid(&fx, p).to_le_bytes();
    let want = vec![0x0D, 0, g[0], g[1], g[2], g[3], 0x12, 10, 0, 10, 0, 0, 0];
    assert_eq!(sent(&mut fx), vec![(p, want)], "mode 9");
}

/// The skill message's level byte is the low byte of `skill_level(unit,
/// E, 1)`: the entry's base + its bonus level, so stat 127
/// `item_allskills` counts; a charged entry (not native) is its base.
#[test]
fn the_skill_message_level_is_the_skill_level() {
    let (mut fx, p, m) = setup();
    fx.sim.hooks().x.entry_flags.insert(p, 0x4);
    use_skill(&mut fx, p, 0, 1);
    target(&mut fx, p, m, UnitType::Monster);
    fx.stats(p, &[(127, 3)]);
    changed(&mut fx, p, 7);
    fx.tick();
    let (g, gm) = (guid(&fx, p), guid(&fx, m));
    let want = skill_message::skill_on_unit(0, g, 0, 5, 1, gm, 0);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())], "1 + 1 + 3");
    fx.sim.hooks().x.used.get_mut(&p).unwrap().owner_guid = 7;
    changed(&mut fx, p, 7);
    fx.tick();
    let want = skill_message::skill_on_unit(0, g, 0, 1, 1, gm, 0);
    assert_eq!(sent(&mut fx), vec![(p, want.to_vec())], "charged: base");
}

/// The player stop rows: NU / TN (other clients only), GH / BL / DD
/// (every client) 0x0D, DT (0x0D, then the own stat 175 message) and KB
/// (0x0F code 0x14 with byte 11 = the unit's hit class).
#[test]
fn player_stop_rows_reach_every_client_or_only_the_others() {
    let (mut fx, p, _) = setup();
    let a = fx.a;
    let q = fx.spawn(UnitType::Player, 1, a, 12, 10);
    fx.game
        .lists
        .add_client(Some(q), None, client_state::IN_GAME);
    fx.tick();
    sent(&mut fx);
    let g = guid(&fx, p).to_le_bytes();
    // The per-client order is the client list's; compare per client.
    let by_client = |mut v: Vec<(UnitId, Vec<u8>)>| {
        v.sort_by_key(|(u, _)| u.0);
        v
    };
    let stop = |code: u8| vec![0x0D, 0, g[0], g[1], g[2], g[3], code, 10, 0, 10, 0, 0, 0];
    // NU 1 and TN 5: code 7 to the other client only.
    for mode in [1, 5] {
        changed(&mut fx, p, mode);
        fx.tick();
        assert_eq!(by_client(sent(&mut fx)), vec![(q, stop(7))], "mode {mode}");
    }
    // GH 4, BL 9: every client.
    for (mode, code) in [(4, 6), (9, 0x12)] {
        changed(&mut fx, p, mode);
        fx.tick();
        assert_eq!(
            by_client(sent(&mut fx)),
            vec![(p, stop(code)), (q, stop(code))],
            "mode {mode}"
        );
    }
    // KB 19: 0x0F code 0x14, target x, y, hit class, x, y.
    path_target(&mut fx, p, (0x1234, 0x0567));
    changed(&mut fx, p, 19);
    fx.tick();
    let kb = vec![
        0x0F, 0, g[0], g[1], g[2], g[3], 0x14, 0x34, 0x12, 0x67, 0x05, 0, 10, 0, 10, 0,
    ];
    assert_eq!(by_client(sent(&mut fx)), vec![(p, kb.clone()), (q, kb)]);
    // DT 0 (stat 175 = 300): 0x0D code 8 to both, then the own 0x1E.
    fx.stats(p, &[(175, 300)]);
    changed(&mut fx, p, 0);
    fx.tick();
    assert_eq!(
        by_client(sent(&mut fx)),
        vec![(p, stop(8)), (p, vec![0x1E, 175, 0x2C, 0x01]), (q, stop(8)),]
    );
}

/// A state toggled on a unit reaches the client as 0xA7
/// (no list entries) in the next client pass, once; toggled off, 0xA9.
#[test]
fn state_changes_are_sent_once_as_a_7_then_a_9() {
    let (mut fx, p, m) = setup();
    let g = guid(&fx, m).to_le_bytes();
    let state_msgs = |v: Vec<(UnitId, Vec<u8>)>| -> Vec<Vec<u8>> {
        v.into_iter()
            .map(|(_, b)| b)
            .filter(|b| matches!(b[0], 0xA7..=0xA9))
            .collect()
    };
    let s = &mut fx.sim.sys;
    let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
    v.set_state(m, 1, true);
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    let on = state_msgs(sent(&mut fx));
    assert_eq!(on, vec![vec![0xA7, 1, g[0], g[1], g[2], g[3], 1]]);
    fx.tick();
    assert_eq!(state_msgs(sent(&mut fx)), Vec::<Vec<u8>>::new());
    let s = &mut fx.sim.sys;
    let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
    v.set_state(m, 1, false);
    fx.game.lists.queue_update(m).unwrap();
    fx.tick();
    let off = state_msgs(sent(&mut fx));
    assert_eq!(off, vec![vec![0xA9, 1, g[0], g[1], g[2], g[3], 1]]);
    let _ = p;
}

/// 0xA8 carries the state list's entries.
#[test]
fn set_state_message_layout() {
    let m = crate::units::messages::set_state(1, 0x0102_0304, 9, &[], |_| None);
    // size = 8 + stream (the 0x1FF end: 9 bits = 2 bytes).
    assert_eq!(m[..8], [0xA8, 1, 4, 3, 2, 1, 10, 9]);
    assert_eq!(m.len(), 10);
}

/// A walk-in-radius request whose point is
/// the monster's own cell is still made. Type 13 finds no path, the retry
/// leaves type 15 with 0 points, the staged velocity request is consumed,
/// the WL start fails into neutral (mode 1, think at f + `aidel`), no draw
/// is made, and the next client pass sends one mode message at the cell.
#[test]
fn a_zero_length_walk_goes_neutral_and_sends_code_7() {
    use crate::monsters::ai::seams::AiModes;
    use crate::monsters::ai::VelocityRequest;
    use crate::path::record::path_types;
    use crate::tick::events::event;
    let (mut fx, p, m) = setup();
    fx.game.timers.cancel_unit_events(m, event::AI_THINK, None);
    let at = fx.sim.hooks().path_position(m);
    let seed = fx.sim.sys.units.get(m).unwrap().seed;
    let f = fx.game.frame;
    let mut v = VelocityRequest {
        method: 0,
        speed: 40,
        steps: 0,
    };
    // The result is not read.
    fx.sim.with(&mut fx.game, |g, view| {
        AiModes::walk_in_radius(view, g, m, m, 3, 0, &mut v)
    });
    assert_eq!(v, VelocityRequest::default());
    assert_eq!(fx.sim.hooks().paths.as_ref().unwrap().mode_velocity, None);
    let d = path(&mut fx, m);
    assert_eq!((d.path_type, d.point_count), (path_types::WALL_FOLLOW, 0));
    assert_eq!(
        (i32::from(d.target_x), i32::from(d.target_y), d.target_unit),
        (at.0, at.1, None)
    );
    assert_eq!(d.repath_budget, 20);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 1);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().seed, seed);
    let thinks: Vec<_> = fx
        .timers(m)
        .into_iter()
        .filter(|&(e, _)| e == event::AI_THINK)
        .collect();
    assert_eq!(thinks, [(event::AI_THINK, f + 15)]);
    fx.tick();
    // The neutral start sends mode 1 as 0x6D at the cell, not 0x67 code 7.
    // One mode message.
    let all = sent(&mut fx);
    assert!(!all.iter().any(|(_, b)| b[0] == 0x68), "{all:?}");
    let msgs: Vec<_> = all
        .into_iter()
        .filter(|(_, b)| (0x67..=0x6D).contains(&b[0]))
        .collect();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    let (to, b) = &msgs[0];
    assert_eq!(*to, p);
    assert_eq!(b[0], 0x6D);
    assert_eq!(b[1..5], guid(&fx, m).to_le_bytes());
    assert_eq!(
        (
            u16::from_le_bytes([b[5], b[6]]),
            u16::from_le_bytes([b[7], b[8]])
        ),
        (at.0 as u16, at.1 as u16)
    );
    assert_eq!(fx.sim.hooks().errors, vec![]);
}
