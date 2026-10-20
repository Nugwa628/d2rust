//! Tests for monster AI on the wired units (modes, timer events): the think
//! run by the unit dispatch, the freeze drop, mode changes through the real
//! monster mode set, the state-54 rule before a think is scheduled, the AI
//! state, the monster attack events, and a few AI-facing lookups (target
//! lists, skill levels, monstats flags, reach).

use super::*;
use crate::monsters::ai::{install, mode, AiControl, AiModes, ModeTarget};
use crate::stats::states::state;
use crate::tick::events::event;

/// A monster of class 0 (AI 1, Idle) in room A with its AI installed.
fn monster(fx: &mut Fx) -> UnitId {
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    fx.sim
        .ai(&mut fx.game, |g, cx| {
            cx.store.entry(m).control = Some(AiControl::default());
            install(g, cx, m, 0);
        })
        .unwrap();
    m
}

fn thinks(fx: &Fx, m: UnitId) -> Vec<i32> {
    fx.timers(m)
        .into_iter()
        .filter(|t| t.0 == event::AI_THINK)
        .map(|t| t.1)
        .collect()
}

#[test]
fn think_runs_through_the_unit_dispatch_and_reschedules() {
    // Idle AI: the next think comes 200 frames later, scheduled through the
    // real timer queue.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), 1, None, 0, 0)
        .unwrap();
    fx.frame();
    assert_eq!(thinks(&fx, m), [201]);
    // Nothing runs until then; at 201 it thinks again.
    for _ in 0..199 {
        fx.frame();
    }
    assert_eq!(thinks(&fx, m), [201]);
    fx.frame();
    assert_eq!(thinks(&fx, m), [401]);
    assert!(fx.sim.hooks().ai_store().unhandled.is_empty());
    fx.assert_clean();
}

#[test]
fn frozen_monster_drops_think_and_reset() {
    // Types 2 and 10 are dropped while the monster has state 1 and is
    // alive; a dropped think is not rescheduled.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.sim.with(&mut fx.game, |_, v| {
        v.set_state(m, state::FREEZE as u16, true)
    });
    for ev in [event::AI_THINK, event::AI_RESET] {
        fx.game
            .schedule_event(m, u32::from(ev), 1, None, 0, 0)
            .unwrap();
    }
    fx.frame();
    assert!(thinks(&fx, m).is_empty());
    assert!(fx.timers(m).is_empty());
    // Unfrozen: the think runs.
    fx.sim.with(&mut fx.game, |_, v| {
        v.set_state(m, state::FREEZE as u16, false)
    });
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), 2, None, 0, 0)
        .unwrap();
    fx.frame();
    assert_eq!(thinks(&fx, m), [202]);
    fx.assert_clean();
}

#[test]
fn ai_mode_change_runs_the_monster_mode_set() {
    // The neutral start is mode 1 and a think at frame + aidel (15) unless
    // one is pending later.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.sim.sys.units.get_mut(m).unwrap().mode = u32::from(mode::WALK);
    fx.game.frame = 30;
    let ok = fx.sim.with(&mut fx.game, |g, v| {
        v.change_mode(g, m, mode::NEUTRAL, ModeTarget::Unit(m))
    });
    assert!(ok);
    assert_eq!(
        fx.sim.sys.units.get(m).unwrap().mode,
        u32::from(mode::NEUTRAL)
    );
    assert_eq!(thinks(&fx, m), [45]);
    // State 54: the mode set is refused (and logged as an error).
    fx.sim.with(&mut fx.game, |_, v| {
        v.set_state(m, state::UNINTERRUPTABLE as u16, true)
    });
    let ok = fx.sim.with(&mut fx.game, |g, v| {
        v.change_mode(g, m, mode::WALK, ModeTarget::Point(12, 10))
    });
    assert!(!ok);
    assert_eq!(fx.sim.hooks().errors.len(), 1);
}

#[test]
fn ai_state_toggle_queues_the_unit_update() {
    // The AI's state toggle sets the state and then always inserts the unit
    // into its room's update queue, also when the bit did not change; a
    // state outside the states table does nothing.
    use crate::monsters::ai::AiUnits;
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    let _ = fx.game.lists.clear_update_queue(fx.a);
    fx.sim.with(&mut fx.game, |g, v| {
        AiUnits::set_state(v, g, m, u16::MAX, true)
    });
    assert!(fx.game.lists.update_queue(fx.a).is_empty());
    let freeze = state::FREEZE as u16;
    fx.sim.with(&mut fx.game, |g, v| {
        AiUnits::set_state(v, g, m, freeze, true)
    });
    assert!(fx.sim.sys.stats.has_state(m, state::FREEZE));
    assert_eq!(fx.game.lists.update_queue(fx.a), [m]);
    let _ = fx.game.lists.clear_update_queue(fx.a);
    // Unchanged bit: queued again.
    fx.sim.with(&mut fx.game, |g, v| {
        AiUnits::set_state(v, g, m, freeze, true)
    });
    assert_eq!(fx.game.lists.update_queue(fx.a), [m]);
    fx.assert_clean();
}

#[test]
fn think_scheduled_with_state_54_clears_it_first() {
    // Scheduling a think clears state 54 and cancels the monster's type-2
    // events first (here through the neutral start).
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), 5, None, 0, 0)
        .unwrap();
    fx.sim.with(&mut fx.game, |_, v| {
        v.set_state(m, state::UNINTERRUPTABLE as u16, true)
    });
    fx.game.frame = 10;
    let r = fx.sim.sys.with(&mut fx.game, |sim, hooks| {
        crate::units::modes::monster_neutral(sim, hooks, m)
    });
    assert_eq!(r, Ok(()));
    assert!(!fx.sim.sys.stats.has_state(m, state::UNINTERRUPTABLE));
    assert_eq!(thinks(&fx, m), [25]);
    fx.assert_clean();
}

#[test]
fn good_npc_ranged_takes_ai_turns() {
    // GoodNpcRanged on the wired host: a class-0 monster with AI 60. There
    // is no secondary target (the pending default), so each think ends by
    // drawing `lo' % 100`: below 20 it wanders (5), else it idles (10).
    let mut fx = Fx::new();
    std::sync::Arc::get_mut(&mut fx.sim.sys.hooks.tables)
        .expect("tables not shared yet")
        .combat
        .monstats[0]
        .ai = 60;
    let m = monster(&mut fx);
    assert_eq!(
        fx.sim.hooks().ai_store().control(m).map(|c| c.function),
        Some(0x005E_7AC0)
    );
    fx.seed(m, seed_giving(50));
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), 1, None, 0, 0)
        .unwrap();
    fx.frame();
    // 50 ≥ 20 → idle 10 (neutral already: no mode change).
    assert_eq!(thinks(&fx, m), [11]);
    assert_eq!(
        fx.sim.sys.units.get(m).unwrap().mode,
        u32::from(mode::NEUTRAL)
    );
    // Another idle-10 turn at 11.
    fx.seed(m, seed_giving(99));
    for _ in 0..10 {
        fx.frame();
    }
    assert_eq!(thinks(&fx, m), [21]);
    // The turn at 21 draws 5 < 20 → wander 5: exactly the wander draws
    // around its own position, and a walk request the real monster mode
    // set runs. Without the path provider the monster has no path, so its
    // point count is 0: the walk start fails into the neutral start, which
    // schedules the next think at 21 + aidel (0, so 15).
    fx.seed(m, seed_giving(5));
    let pos = fx.sim.hooks().x.position(m);
    let mut want = seed_giving(5);
    want.step();
    crate::monsters::ai::wander_point(&mut want, pos, 5);
    for _ in 0..10 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().seed, want);
    assert_eq!(thinks(&fx, m), [36]);
    assert_eq!(
        fx.sim.sys.units.get(m).unwrap().mode,
        u32::from(mode::NEUTRAL)
    );
    assert!(fx.sim.hooks().ai_store().unhandled.is_empty());
    fx.assert_clean();
}

/// The log lines of `m` starting with `p`.
fn logged(fx: &Fx, p: &str, m: UnitId) -> Vec<String> {
    let p = format!("{p} {}", m.0);
    fx.sim
        .sys
        .hooks
        .x
        .log
        .iter()
        .filter(|l| l.starts_with(&p))
        .cloned()
        .collect()
}

/// Type-0 events of `m` with frame codes `codes`, one per frame from 1.
fn frame_events(fx: &mut Fx, m: UnitId, codes: &[u32]) {
    {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        // The frame advance reads the code from the animation record: one
        // frame per event, the codes at frames 1, 2, ...
        let mut events = [0u8; crate::units::record::ANIM_EVENTS];
        for (i, &c) in codes.iter().enumerate() {
            events[1 + i] = c as u8;
        }
        r.anim.record = Some(crate::units::record::AnimRecord {
            frames: 1 << 8,
            byte_0f: 0,
            events,
        });
        r.anim.speed = 256;
        r.anim.frame = 0;
        r.anim.frame_count = 1 << 16;
    }
    for (i, &c) in codes.iter().enumerate() {
        fx.game
            .schedule_event(m, u32::from(event::MODE_CHANGE), 1 + i as i32, None, c, 0)
            .unwrap();
    }
    for _ in codes {
        fx.frame();
    }
}

#[test]
fn attack_event0_without_a_used_skill_strikes_once_per_frame_code_event() {
    // With no used skill and a mode that does not move (A2: class 0's
    // monstats2 mv bits are A1 only), the strike (mode missile, else melee
    // on the path target) happens on every event 0, whatever its frame
    // code. The action frame is the frame advance's.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.sim.sys.units.get_mut(m).unwrap().mode = u32::from(mode::ATTACK2);
    frame_events(&mut fx, m, &[1, 2, 4]);
    let want = vec![format!("attack strike {} false", m.0); 3];
    assert_eq!(logged(&fx, "attack strike", m), want);
    assert!(logged(&fx, "attack skill", m).is_empty());
    assert!(logged(&fx, "sequence frame", m).is_empty());
    // A non-moving mode strikes with no frame advance: the action frame is
    // untouched.
    assert_eq!(fx.sim.sys.units.get(m).unwrap().anim.action_frame, 0);
}

#[test]
fn attack_event0_of_a_moving_mode_strikes_only_at_its_trigger_frame() {
    // A1 moves for class 0 (mv bit 4): step, refresh, then the strike
    // (moving flag set) only when the trigger holds, i.e. the action frame
    // is 1.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.mode = u32::from(mode::ATTACK1);
        r.anim.frame_count = 1 << 16;
        // The frame advance writes the action frame from the record's event
        // bytes: frames 1, 2, 3 carry 2, 1, 3.
        r.anim.speed = 256;
        let mut events = [0u8; crate::units::record::ANIM_EVENTS];
        events[1..4].copy_from_slice(&[2, 1, 3]);
        r.anim.record = Some(crate::units::record::AnimRecord {
            frames: 256,
            byte_0f: 0,
            events,
        });
    }
    let x0 = fx.sim.sys.hooks.x.position(m).0;
    frame_events(&mut fx, m, &[2, 1, 3]);
    assert_eq!(
        logged(&fx, "attack strike", m),
        [format!("attack strike {} true", m.0)]
    );
    // One path step per event (the fake step moves one subtile).
    assert_eq!(fx.sim.sys.hooks.x.position(m).0, x0 + 3);
}

#[test]
fn attack_event0_with_a_used_skill_runs_its_branch_on_every_event() {
    // With a used skill the do runs on each event 0 regardless of the
    // action frame (no frame-code test), and the no-skill strike never runs.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.sim.sys.units.get_mut(m).unwrap().mode = u32::from(mode::ATTACK1);
    fx.sim.sys.hooks.x.used.insert(
        m,
        crate::skills::SkillEntry {
            skill: 0,
            base: 1,
            owner_guid: -1,
            ..crate::skills::SkillEntry::default()
        },
    );
    frame_events(&mut fx, m, &[3, 0, 1]);
    assert_eq!(logged(&fx, "attack skill", m).len(), 3);
    assert!(logged(&fx, "attack strike", m).is_empty());
}

/// A monster world holding only monster data (the AI-state tests).
struct DataWorld(std::collections::BTreeMap<UnitId, crate::monsters::init::MonsterData>);

impl<X> crate::wiring::action::monsters::MonsterWorld<X> for DataWorld {
    fn type_init(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
    ) {
    }
    fn umods(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
        _: Option<UnitId>,
        _: u8,
    ) {
    }
    fn assign_umod(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
        _: u8,
    ) {
    }
    fn forget(&mut self, _: UnitId) {}
    fn monster(&self, unit: UnitId) -> Option<&crate::monsters::init::MonsterData> {
        self.0.get(&unit)
    }
    fn monster_mut(&mut self, unit: UnitId) -> Option<&mut crate::monsters::init::MonsterData> {
        self.0.get_mut(&unit)
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

fn ai_state_of(fx: &mut Fx, m: UnitId) -> u32 {
    fx.sim.hooks().monster_data(m).unwrap().ai_state
}

fn leave(fx: &mut Fx, m: UnitId, from: u8, state: u32) {
    fx.sim.hooks().set_monster_ai_state(m, state);
    fx.sim.sys.units.get_mut(m).unwrap().mode = u32::from(from);
    fx.game.frame += 1;
    let ok = fx.sim.with(&mut fx.game, |g, v| {
        v.change_mode(g, m, mode::NEUTRAL, ModeTarget::Unit(m))
    });
    assert!(ok);
}

#[test]
fn ai_state_is_stored_and_follows_the_mode_set() {
    // The monster data's `dwAiState` is 0 at creation; setting it stores a
    // value (a soft hit: 19) and the monster mode set applies the AI-state
    // rule to the mode it leaves: not for mode 1; state >= 16 becomes
    // state - 16; 13 leaving mode 3 stays; else the state becomes that mode.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    let world = DataWorld([(m, Default::default())].into_iter().collect());
    fx.sim.sys.hooks.monster_world = Some(Box::new(world));
    assert_eq!(ai_state_of(&mut fx, m), 0);
    fx.sim.hooks().set_monster_ai_state(m, 19);
    assert_eq!(ai_state_of(&mut fx, m), 19);
    // 19 leaving A2 (mode 5) gives 3.
    leave(&mut fx, m, mode::ATTACK2, 19);
    assert_eq!(ai_state_of(&mut fx, m), 3);
    // 0 leaving A2 gives 5.
    leave(&mut fx, m, mode::ATTACK2, 0);
    assert_eq!(ai_state_of(&mut fx, m), 5);
    // Leaving mode 1: unchanged.
    leave(&mut fx, m, mode::NEUTRAL, 19);
    assert_eq!(ai_state_of(&mut fx, m), 19);
    // 13 leaving mode 3 stays 13; leaving mode 4 it becomes 4.
    leave(&mut fx, m, mode::GETHIT, 13);
    assert_eq!(ai_state_of(&mut fx, m), 13);
    leave(&mut fx, m, mode::ATTACK1, 13);
    assert_eq!(ai_state_of(&mut fx, m), 4);
    fx.assert_clean();
}

/// A good NPC's registration (NpcBarb, slot 8) puts it at the head of target
/// list 8 and sets the unit's node index; the main search's lists then hold
/// it after the host's own nodes, newest first, and a removed unit leaves its
/// list.
#[test]
fn registered_good_npcs_join_target_list_8_newest_first() {
    use crate::monsters::ai::{AiSummons, AiTargets};
    let mut fx = Fx::new();
    let a = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let b = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    let lists = |fx: &mut Fx| {
        let s = &mut fx.sim.sys;
        let v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
        v.target_nodes(&fx.game)
    };
    for u in [a, b] {
        let s = &mut fx.sim.sys;
        let mut v = View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
        assert_eq!(v.target_slot(u), 11);
        v.register_target_node(&mut fx.game, u, 8);
        assert_eq!(v.target_slot(u), 8);
    }
    assert_eq!(lists(&mut fx).good, vec![b, a]);
    assert!(lists(&mut fx).walls.is_empty());
    fx.game.remove_unit(b).unwrap();
    assert_eq!(lists(&mut fx).good, vec![a]);
}

#[test]
fn natural_skill_level_is_read_by_the_ai() {
    // Monster init gives the monster its skill entry; the AI reads its base
    // level. The fallback to 1 belongs to the AI body.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    fx.sim.with(&mut fx.game, |_, v| {
        assert_eq!(
            crate::monsters::ai::AiActs::skill_level(&*v, m, 352, false),
            None
        );
        v.h.natural_skills.entry(m).or_default().insert(352, 4);
        assert_eq!(
            crate::monsters::ai::AiActs::skill_level(&*v, m, 352, false),
            Some(4)
        );
        assert_eq!(
            crate::monsters::ai::AiActs::skill_level(&*v, m, 300, false),
            None
        );
        // A summon's entry (`monster_skills`) wins.
        v.h.monster_skills.entry(m).or_default().insert(352, 9);
        assert_eq!(
            crate::monsters::ai::AiActs::skill_level(&*v, m, 352, false),
            Some(9)
        );
    });
    fx.assert_clean();
}

#[test]
fn used_skill_takes_the_monster_entry_level() {
    // A monster without a skill list casts with its init entry's base
    // level (for example doomknight2's DoomKnightMissile at `Sk1lvl` 3 makes
    // a level 3 missile), a summon's entry winning; a skill with no entry
    // keeps the seam's answer.
    let mut fx = Fx::new();
    let m = monster(&mut fx);
    let entry = |skill| crate::skills::SkillEntry {
        skill,
        base: 1,
        owner_guid: -1,
        ..Default::default()
    };
    fx.sim.with(&mut fx.game, |_, v| {
        v.h.x.used.insert(m, entry(335));
        assert_eq!(v.h.used_skill_of(m).map(|e| e.base), Some(1));
        v.h.natural_skills.entry(m).or_default().insert(335, 3);
        assert_eq!(v.h.used_skill_of(m).map(|e| e.base), Some(3));
        v.h.monster_skills.entry(m).or_default().insert(335, 7);
        assert_eq!(v.h.used_skill_of(m).map(|e| e.base), Some(7));
        v.h.x.used.insert(m, entry(300));
        assert_eq!(v.h.used_skill_of(m).map(|e| e.base), Some(1));
    });
    fx.assert_clean();
}

/// The boss, demon, undead and prime-evil tests read the monstats flags of
/// the monster's class: a boss such as Griswold gets the boss-sound idle,
/// which defers its first think by 20 frames. A unit without monster data
/// keeps the `Pending` answer (false here).
#[test]
fn monstats_flags_answer_boss_demon_undead_prime_evil() {
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let flags = |fx: &mut Fx| {
        let h = fx.sim.hooks();
        (
            h.is_boss(m),
            h.is_demon(m),
            h.is_undead(m),
            h.is_prime_evil(m),
        )
    };
    let data = crate::monsters::init::MonsterData::default();
    fx.sim.sys.hooks.monster_world = Some(Box::new(super::sound::DataOnly(
        [(m, data)].into_iter().collect(),
    )));
    assert_eq!(flags(&mut fx), (false, false, false, false));
    let mut tables = (*fx.sim.sys.hooks.tables).clone();
    let row = &mut tables.combat;
    row.monstats[0].boss = true;
    row.monstats[0].hundead = true;
    fx.sim.sys.hooks.tables = std::sync::Arc::new(tables);
    assert_eq!(flags(&mut fx), (true, false, true, false));
    let mut tables = (*fx.sim.sys.hooks.tables).clone();
    let row = &mut tables.combat;
    row.monstats[0].demon = true;
    row.monstats[0].primeevil = true;
    fx.sim.sys.hooks.tables = std::sync::Arc::new(tables);
    assert_eq!(flags(&mut fx), (true, true, true, true));
}

/// The wisp gather's find has a callback, so the filter's radius never
/// applies: a circle not strictly inside the room takes the adjacency array,
/// and every unit of those rooms comes back, in room then list order,
/// however far away.
#[test]
fn wisp_find_returns_every_unit_of_the_adjacent_rooms() {
    use crate::monsters::ai::AiActs;
    let mut fx = Fx::new();
    let w = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let near = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    let far = fx.spawn(UnitType::Monster, 0, fx.b, 75, 10);
    let found = fx.sim.with(&mut fx.game, |g, v| v.wisp_find(g, w));
    for u in [w, near, far] {
        assert!(found.contains(&u), "{u:?} not found in {found:?}");
    }
    let (a, b) = (
        fx.game.lists.room_units(fx.a),
        fx.game.lists.room_units(fx.b),
    );
    let order: Vec<UnitId> = fx
        .game
        .lists
        .room(fx.a)
        .unwrap()
        .adjacent
        .iter()
        .flat_map(|&r| {
            if r == fx.a {
                a.clone()
            } else if r == fx.b {
                b.clone()
            } else {
                Vec::new()
            }
        })
        .collect();
    assert_eq!(found, order);
}

#[test]
fn melee_rng_255_gives_reach_2_for_a_2ht_monster_outside_death() {
    // `MeleeRng` 255 reads the weapon class in the current mode: monstats2
    // `BaseW` outside DT/DD; hth in DT/DD unless `compositeDeath`.
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let reach = |fx: &mut Fx| fx.sim.with(&mut fx.game, |_, v| v.monster_reach(m));
    fn row(fx: &mut Fx) -> &mut d2_data::tables::Monstats2 {
        &mut std::sync::Arc::make_mut(&mut fx.sim.sys.hooks.tables)
            .combat
            .monstats2[0]
    }
    row(&mut fx).meleerng = 3;
    assert_eq!(reach(&mut fx), Some(3));
    row(&mut fx).meleerng = 255;
    row(&mut fx).basew = *b"1hs ";
    assert_eq!(reach(&mut fx), Some(0));
    row(&mut fx).basew = *b"2ht ";
    assert_eq!(reach(&mut fx), Some(2));
    for mode in [0, 12] {
        fx.sim.sys.units.get_mut(m).unwrap().mode = mode;
        row(&mut fx).compositedeath = false;
        assert_eq!(reach(&mut fx), Some(0), "mode {mode}: hth");
        row(&mut fx).compositedeath = true;
        assert_eq!(reach(&mut fx), Some(2), "mode {mode}: compositeDeath");
    }
}

/// Every player gets its own target list: the first eight take lists 0–7,
/// the next ones the lists past the eighth, so monsters can find all of
/// them; a freed list is taken by the next player.
#[test]
fn every_player_heads_its_own_target_list_past_eight() {
    use crate::game::node;
    let mut fx = Fx::new();
    let players: Vec<UnitId> = (0..10)
        .map(|i| fx.spawn(UnitType::Player, 0, fx.a, 10 + i, 10))
        .collect();
    let codes: Vec<u32> = players
        .iter()
        .map(|&p| fx.sim.sys.units.get(p).unwrap().node_index)
        .collect();
    let want: Vec<u32> = (0..10).map(node::player).collect();
    assert_eq!(codes, want);
    // A merc joins its player's list, the tenth player's too.
    let merc = fx.spawn(UnitType::Monster, 0, fx.a, 40, 10);
    fx.game
        .target_nodes
        .push_front(node::player(9) as i32, merc);
    assert_eq!(fx.game.target_nodes.slot(node::player(9) as usize), &[merc]);
    // Player 3 leaves; the next player takes its list.
    fx.game.remove_unit(players[3]).unwrap();
    fx.sim.sys.units.remove(players[3]);
    let next = fx.spawn(UnitType::Player, 0, fx.a, 30, 10);
    assert_eq!(fx.sim.sys.units.get(next).unwrap().node_index, 3);
}
