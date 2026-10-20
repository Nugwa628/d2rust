//! Tests for combat ↔ stats and unit fields: the real hit test, damage application
//! and state lists on unit records, stat lists and timers.

use super::*;
use crate::combat::{self as cb, result, CombatWorld, DamageRecord};
use crate::skills::SkillUnits;
use crate::stats::stat as st;
use crate::tick::events::event;

const TOHIT: u16 = 19;
const LEVEL_STAT: u16 = 12;
const ARMORCLASS: u16 = 31;
const NORMAL_DR: u16 = 34;
const DAMAGERESIST: u16 = 36;

/// A monster attacker with AR 300, level 10, against a player with
/// defense 100, level 8: hit chance 83.
fn duel(fx: &mut Fx) -> (UnitId, UnitId) {
    let a = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let d = fx.spawn(UnitType::Player, 0, fx.a, 12, 10);
    fx.stats(a, &[(TOHIT, 300), (LEVEL_STAT, 10)]);
    fx.stats(d, &[(ARMORCLASS, 100), (LEVEL_STAT, 8)]);
    (a, d)
}

#[test]
fn hit_test_reads_real_stats_and_draws_on_the_attacker_seed() {
    let mut fx = Fx::new();
    let (a, d) = duel(&mut fx);
    // Seed (1, 0): lo′ = 1791398085, mod 100 = 85 ≥ 83 → miss.
    fx.seed(a, Seed::new(1, 0));
    let hit = fx.sim.combat(&mut fx.game, |w, t| {
        cb::hit_test(w, &t.skills, &t.combat, Some(a), Some(d), 0, false)
    });
    assert!(!hit);
    assert_eq!(
        fx.sim.sys.units.get(a).unwrap().seed,
        Seed::new(0x6AC6_90C5, 0)
    );
    for (x, want) in [(82, true), (83, false)] {
        fx.seed(a, seed_giving(x));
        let hit = fx.sim.combat(&mut fx.game, |w, t| {
            cb::hit_test(w, &t.skills, &t.combat, Some(a), Some(d), 0, false)
        });
        assert_eq!(hit, want, "draw {x}");
    }
    fx.assert_clean();
}

#[test]
fn missile_damage_lowers_real_life_after_resistances() {
    // Physical 2560, normal DR 3, damage resist 20 (player)
    // → (2560 − 768) × 80 / 100 = 1433.
    let mut fx = Fx::new();
    let (a, d) = duel(&mut fx);
    fx.stats(
        d,
        &[
            (st::MAXHP, 25600),
            (st::HITPOINTS, 25600),
            (NORMAL_DR, 3),
            (DAMAGERESIST, 20),
        ],
    );
    let mut rec = DamageRecord {
        result: result::HIT,
        physical: 2560,
        ..DamageRecord::default()
    };
    fx.sim.combat(&mut fx.game, |w, t| {
        cb::apply(w, &t.combat, a, d, true, &mut rec)
    });
    assert_eq!(rec.total, 1433);
    assert_eq!(fx.stat(d, st::HITPOINTS), 25600 - 1433);
    assert_eq!(rec.result & result::WILL_DIE, 0);
    // Event 11 (absorb damage) from the totals, then event 2 (damaged by
    // missile) on the defender; no `domissiledamage` without the "rolled"
    // hit flag.
    assert_eq!(
        fx.sim.hooks().x.log,
        [
            format!("event 11 Some({})", d.0),
            format!("event 2 Some({})", d.0)
        ]
    );
    fx.assert_clean();
}

#[test]
fn damage_reschedules_monster_regen_and_kills_at_zero_life() {
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    fx.stats(
        m,
        &[(st::MAXHP, 5120), (st::HITPOINTS, 5120), (st::HPREGEN, 1)],
    );
    fx.game.frame = 40;
    let mut rec = DamageRecord {
        result: result::HIT,
        physical: 1024,
        ..DamageRecord::default()
    };
    fx.sim.combat(&mut fx.game, |w, t| {
        cb::apply(w, &t.combat, p, m, true, &mut rec)
    });
    // Life > 0, hpregen ≠ 0 → type 3 at frame + 1.
    assert_eq!(fx.stat(m, st::HITPOINTS), 4096);
    assert!(fx.timers(m).contains(&(event::STAT_REGEN, 41)));
    // A blow past the remaining life: life 0 (< 256), "will die", events
    // 10 (killed) on the defender and 9 (kill) on the attacker.
    fx.sim.hooks().x.log.clear();
    let mut rec = DamageRecord {
        result: result::HIT,
        physical: 8192,
        ..DamageRecord::default()
    };
    fx.sim.combat(&mut fx.game, |w, t| {
        cb::apply(w, &t.combat, p, m, true, &mut rec)
    });
    assert_eq!(fx.stat(m, st::HITPOINTS), 0);
    assert_ne!(rec.result & result::WILL_DIE, 0);
    assert_eq!(
        fx.sim.hooks().x.log,
        [
            format!("event 11 Some({})", m.0),
            format!("event 2 Some({})", m.0),
            format!("event 10 Some({})", m.0),
            format!("event 9 Some({})", p.0),
        ]
    );
    fx.assert_clean();
}

/// Poison (a state-2 list with stat 74 below 0) takes a monster's life to 0
/// in its regeneration event; the monster is killed (death mode) by the
/// list's owner, and the death events follow: killed (10) on the unit, then
/// kill (9) on the owner. Recorded: check `combat-elements` f78, the quill
/// rat poisoned by the player's javelin dies at life 0.
#[test]
fn poison_kills_a_monster_at_zero_life_by_its_owner() {
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    fx.stats(m, &[(st::MAXHP, 5120), (st::HITPOINTS, 100)]);
    const POISON: u16 = 2;
    fx.sim.combat(&mut fx.game, |w, _| {
        w.create_state_list(m, POISON, p, 50);
        w.set_state_list_stat(m, POISON, st::HPREGEN, -87);
        w.set_state(m, POISON, true);
    });
    fx.game
        .schedule_event(m, u32::from(event::STAT_REGEN), 1, None, 0, 0)
        .unwrap();
    fx.sim.hooks().x.log.clear();
    fx.frame();
    assert_eq!(fx.stat(m, st::HITPOINTS), 13);
    assert_ne!(fx.sim.sys.units.get(m).unwrap().mode, 0);
    fx.frame();
    assert_eq!(fx.stat(m, st::HITPOINTS), 0);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 0);
    let log = &fx.sim.hooks().x.log;
    let killed = format!("event 10 Some({})", m.0);
    let kill = format!("event 9 Some({})", p.0);
    let i = log.iter().position(|l| *l == killed).expect("killed event");
    assert_eq!(log.get(i + 1), Some(&kill));
}

#[test]
fn state_lists_expire_through_the_type_12_event() {
    // Combat's state-list effects on the real stat lists, expired by the
    // unit dispatch's event 12.
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    const COLD: u16 = 11;
    fx.sim.combat(&mut fx.game, |w, _| {
        w.create_state_list(m, COLD, p, 5);
        w.set_state_list_stat(m, COLD, 67, -50);
        w.set_state(m, COLD, true);
        w.schedule_timer(m, event::REMOVE_STATE, 5);
        assert_eq!(w.state_list_expiry(m, COLD), Some(5));
        assert_eq!(w.state_stat(m, COLD, 67), Some(-50));
        assert!(w.has_state(m, COLD));
        // The list's stat counts on the unit.
        assert_eq!(w.stat(m, 67, 0), -50);
    });
    for _ in 0..5 {
        fx.frame();
    }
    fx.sim.combat(&mut fx.game, |w, _| {
        assert_eq!(w.state_list_expiry(m, COLD), None);
        assert_eq!(w.stat(m, 67, 0), 0);
    });
    fx.assert_clean();
}

#[test]
fn an_expired_inferno_list_runs_its_remove_callback() {
    // Inferno / Arctic Blast start: a flags-2 list of state 12 expiring at
    // F + 20 with the Inferno remove callback. Its expiry by event 12 runs
    // the callback: state 12 off and unit flag 0x40 set, which stops the
    // sequence's do events (dru-arctic-blast's channel ends at F + 20).
    use crate::skills::use_::bodies::callback::INFERNO;
    use crate::stats::lists::RemoveCallback;
    const STATE_INFERNO: u32 = 12;
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    fx.sim.with(&mut fx.game, |g, v| {
        let guid = v.units.get(p).unwrap().guid;
        let l = v.stats.alloc(2, 5, 0, guid);
        v.stats.attach(&mut *v.h, p, l, true);
        v.stats
            .set_remove_callback(l, Some(RemoveCallback(INFERNO)));
        v.stats.set_state(l, STATE_INFERNO);
        v.stats.toggle_state(p, STATE_INFERNO, true);
        g.schedule_event(p, u32::from(event::REMOVE_STATE), 5, None, 0, 0)
            .unwrap();
        assert_eq!(v.units.get(p).unwrap().flags & 0x40, 0);
    });
    for _ in 0..5 {
        fx.frame();
    }
    fx.sim.with(&mut fx.game, |_, v| {
        assert!(!v.stats.has_state(p, STATE_INFERNO));
        assert_ne!(v.units.get(p).unwrap().flags & 0x40, 0);
    });
    fx.assert_clean();
}

#[test]
fn a_set_alignment_makes_the_state_105_list() {
    // Stat 172 in the state-105 list, the state on and its changed bit set
    // (a 0xA8 on the next update); the player's first 0xAA then carries
    // these bytes.
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    fx.sim.with(&mut fx.game, |g, v| {
        assert_eq!(v.state_list(p, 105), None);
        v.set_alignment(g, p, 2);
        let l = v.state_list(p, 105).expect("the list is made");
        assert_eq!(v.stats.base_entries(l).len(), 1);
        assert_eq!(v.state_stat(p, 105, 172), Some(2));
        assert!(v.stats.has_state(p, 105));
        let (_, changed) = v.stats.state_bits(p).expect("state bits");
        assert_ne!(changed[105 / 32] & (1 << (105 % 32)), 0);
        // A second set reuses the list; 0 empties it (the list stays).
        v.set_alignment(g, p, 0);
        assert_eq!(v.state_list(p, 105), Some(l));
        assert!(v.stats.base_entries(l).is_empty());
        // A value of 3 is not accepted; nothing changes.
        v.set_alignment(g, p, 3);
        assert!(v.stats.base_entries(l).is_empty());
        v.set_alignment(g, p, 2);
        assert_eq!(v.state_stat(p, 105, 172), Some(2));
        assert!(g.lists.unit(p).is_some_and(|e| e.allied));
    });
    fx.assert_clean();
}

/// B033: a monster's poison on a player. The state-2 list with stat 74
/// below 0 drains the player's life in the player regeneration event (life
/// never below 256) and the type-12 event takes the list and the state off
/// at its expiry.
#[test]
fn poison_drains_a_player_and_ends_at_its_expiry() {
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    fx.stats(p, &[(st::MAXHP, 25600), (st::HITPOINTS, 25600)]);
    fx.game
        .schedule_event(p, u32::from(event::STAT_REGEN), 1, None, 0, 0)
        .unwrap();
    fx.sim
        .combat(&mut fx.game, |w, _| cb::poison(w, m, p, 100, 10));
    let has = |fx: &mut Fx| fx.sim.combat(&mut fx.game, |w, _| w.has_state(p, 2));
    assert!(has(&mut fx), "poisoned");
    fx.frame();
    fx.frame();
    assert_eq!(fx.stat(p, st::HITPOINTS), 25600 - 200, "100 a frame");
    for _ in 0..10 {
        fx.frame();
    }
    assert!(!has(&mut fx), "the state ends with the list");
    let hp = fx.stat(p, st::HITPOINTS);
    fx.frame();
    fx.frame();
    assert_eq!(fx.stat(p, st::HITPOINTS), hp, "no drain after the expiry");
    assert!(hp < 25600 - 200 && hp > 25600 - 1200);
}

/// B033, burn (state 115) and stun (state 21): their lists carry the
/// default remove callback too, so the state ends with the list.
#[test]
fn burn_and_stun_states_end_with_their_lists() {
    for state in [115u16, 21] {
        let mut fx = Fx::new();
        let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
        let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
        fx.stats(p, &[(st::MAXHP, 25600), (st::HITPOINTS, 25600)]);
        fx.sim.combat(&mut fx.game, |w, _| {
            w.create_state_list(p, state, m, 5);
            w.set_state(p, state, true);
            w.schedule_timer(p, event::REMOVE_STATE, 5);
        });
        for _ in 0..6 {
            fx.frame();
        }
        let on = fx.sim.combat(&mut fx.game, |w, _| w.has_state(p, state));
        assert!(!on, "state {state} ends with its list");
    }
}

/// Freeze end (the freeze list's expiry; the AI resumes
/// with a fresh think): the freeze list carries it, so at the expiry state 1 goes
/// off, the monster is set to mode 1 and its thinks are replaced by one
/// at frame + `aidel` (w1c2-cs-frozen-armor: Fallen 1:27 frozen in GH at
/// f310, neutral at f340).
#[test]
fn a_freeze_ends_with_its_list_in_neutral_with_a_new_think() {
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    fx.sim.sys.units.get_mut(m).unwrap().mode = 5;
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), 100, None, 0, 0)
        .unwrap();
    fx.sim.combat(&mut fx.game, |w, _| {
        w.set_state(m, 1, true);
        w.create_state_list(m, 1, p, 5);
        w.schedule_timer(m, event::REMOVE_STATE, 5);
    });
    for _ in 0..6 {
        fx.frame();
    }
    let on = fx.sim.combat(&mut fx.game, |w, _| w.has_state(m, 1));
    assert!(!on, "state 1 ends with the freeze list");
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 1, "neutral");
    let thinks: Vec<i32> = fx
        .timers(m)
        .into_iter()
        .filter(|t| t.0 == event::AI_THINK)
        .map(|t| t.1)
        .collect();
    assert_eq!(thinks.len(), 1, "{thinks:?}");
    assert!(thinks[0] != 100 && thinks[0] > 5, "{thinks:?}");
}

/// Combat's weapon is the damage weapon chosen by the used skill's
/// `weapsel`; a kick (weapsel 4) has none, so durability makes no attacker
/// draw. Without a used skill, or unanswered, the weapon in use.
#[test]
fn current_weapon_is_the_used_skills_damage_weapon() {
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    let weapon = fx.spawn(UnitType::Item, 0, fx.a, 10, 10);
    let mut t = (*fx.sim.sys.hooks.tables).clone();
    t.skills.skills[0].weapsel = 4;
    fx.sim.sys.hooks.tables = std::sync::Arc::new(t);
    fx.sim.sys.hooks.x.weapons.insert(p, weapon);
    let cw = |fx: &mut Fx| {
        fx.sim
            .combat(&mut fx.game, |w, _| SkillUnits::current_weapon(w, p))
    };
    assert_eq!(cw(&mut fx), Some(weapon));
    let hc = |fx: &mut Fx| {
        fx.sim
            .combat(&mut fx.game, |w, _| CombatWorld::weapon_hit_class(w, p))
    };
    fx.sim.sys.hooks.x.used.insert(
        p,
        crate::skills::SkillEntry {
            skill: 0,
            base: 1,
            owner_guid: -1,
            ..crate::skills::SkillEntry::default()
        },
    );
    // Not answered: the weapon in use, and the host's hit class.
    assert_eq!(cw(&mut fx), Some(weapon));
    assert_eq!(hc(&mut fx), 0);
    fx.sim
        .sys
        .hooks
        .x
        .damage_weapons
        .insert((p, 4, false), None);
    assert_eq!(cw(&mut fx), None);
    // No weapon: hit class 1, whatever the host's weapon.
    assert_eq!(hc(&mut fx), 1);
    // No weapon: durability leaves the attacker's seed alone.
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    let seed = Seed::new(1, 0);
    fx.seed(p, seed);
    fx.sim.combat(&mut fx.game, |w, _| cb::durability(w, p, m));
    assert_eq!(fx.sim.sys.units.get(p).unwrap().seed, seed);
}

/// Past the items-row test, an item has durability only with a maximum
/// durability (a base stat 73) and without stat 152
/// (`item_indesctructible`).
#[test]
fn durability_needs_max_durability_and_not_indestructible() {
    let mut fx = Fx::new();
    let item = fx.spawn(UnitType::Item, 0, fx.a, 10, 10);
    let has = |fx: &mut Fx| {
        fx.sim
            .with(&mut fx.game, |_, v| v.item_has_durability(item))
    };
    // Row test fails: no durability whatever the stats.
    fx.stats(item, &[(73, 250)]);
    assert!(!has(&mut fx));
    fx.sim.sys.hooks.x.durable_rows.push(item);
    assert!(has(&mut fx));
    fx.stats(item, &[(152, 1)]);
    assert!(!has(&mut fx));
    fx.stats(item, &[(152, 0), (73, 0)]);
    assert!(!has(&mut fx));
}

/// The level-up's item refresh ends in the owner refresh: the player is
/// queued for update and its update flag bits 0 and 1 are set.
#[test]
fn the_level_up_refresh_queues_the_player_with_both_update_bits() {
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
    let a = fx.a;
    fx.game.lists.clear_update_queue(a).unwrap();
    if let Some(r) = fx.sim.sys.units.get_mut(p) {
        r.flags2 = 0;
    }
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::combat::vitals::VitalsUnits::level_up_notify(w, p)
    });
    assert_eq!(fx.sim.sys.units.get(p).unwrap().flags2 & 3, 3);
    assert_eq!(fx.game.lists.update_queue(a), [p]);
}

/// The fixture with the shrine states 128-137 in the `curse` group (11),
/// as in the game's states table (test `shrine_states_are_in_the_curse_group`),
/// and an empty object state.
fn shrine_fx() -> Fx {
    let mut fx = Fx::new();
    let mut data = (*crate::stats::tests::data()).clone();
    let flags: Vec<(u32, usize)> = (128..=137).map(|s| (s, 11)).collect();
    data.states = crate::stats::StateTable::synthetic(185, &flags);
    fx.sim.sys.stats = crate::stats::StatLists::new(Arc::new(data));
    fx.sim
        .create_objects(Arc::new(crate::world::objects::ObjectTables {
            objects: Vec::new(),
            shrines: Vec::new(),
            levels: Vec::new(),
            objgroup: Vec::new(),
            leveldefs: Vec::new(),
        }));
    fx
}

fn shrine_state(fx: &mut Fx, src: UnitId, p: UnitId, state: u16, duration: i32) -> bool {
    use crate::world::objects::shrines::{RemoveCallback, StateRequest};
    let req = StateRequest {
        target: p,
        source: src,
        duration,
        state,
        stat: 0,
        value: 0,
        remove: RemoveCallback::Default,
    };
    fx.sim
        .objects(&mut fx.game, |_, _, w| {
            crate::world::objects::ShrineWorld::apply_state(w, req).is_some()
        })
        .expect("object state")
}

fn shrine_list(fx: &mut Fx, p: UnitId, state: u16) -> Option<(u32, i32)> {
    fx.sim.with(&mut fx.game, |_, v| {
        v.state_list(p, state)
            .map(|l| (v.stats.flags(l), v.stats.expire(l)))
    })
}

/// A shrine's state: a second shrine's state replaces the first one's (the
/// target's first list with flag 0x20), the same state is refreshed, and
/// the new list carries flags 0x20 | 2.
#[test]
fn a_shrine_state_replaces_another_shrines_list() {
    let mut fx = shrine_fx();
    let a = fx.a;
    let src = fx.spawn(UnitType::Object, 0, a, 20, 20);
    let p = fx.spawn(UnitType::Player, 0, a, 22, 20);
    let f = fx.game.frame;
    assert!(shrine_state(&mut fx, src, p, 129, 100));
    assert_eq!(shrine_list(&mut fx, p, 129), Some((0x22, f + 100)));
    assert!(shrine_state(&mut fx, src, p, 130, 50));
    assert_eq!(shrine_list(&mut fx, p, 129), None, "the old list is freed");
    assert_eq!(shrine_list(&mut fx, p, 130), Some((0x22, f + 50)));
    // The same state again: refreshed in place.
    assert!(shrine_state(&mut fx, src, p, 130, 80));
    assert_eq!(shrine_list(&mut fx, p, 130), Some((0x22, f + 80)));
    fx.frame();
    fx.sim.combat(&mut fx.game, |w, _| {
        assert!(!w.has_state(p, 129), "the old state goes off");
        assert!(w.has_state(p, 130));
    });
}

/// Curse resistance (109) ≥ 100 refuses a shrine state, a lower one
/// shortens it by that percent; state 57 (`attract`) refuses it.
#[test]
fn curse_resistance_and_attract_gate_a_shrine_state() {
    let mut fx = shrine_fx();
    let a = fx.a;
    let src = fx.spawn(UnitType::Object, 0, a, 20, 20);
    let p = fx.spawn(UnitType::Player, 0, a, 22, 20);
    let f = fx.game.frame;
    fx.stats(p, &[(109, 100)]);
    assert!(!shrine_state(&mut fx, src, p, 129, 100));
    assert_eq!(shrine_list(&mut fx, p, 129), None);
    fx.stats(p, &[(109, 25)]);
    assert!(shrine_state(&mut fx, src, p, 129, 100));
    assert_eq!(shrine_list(&mut fx, p, 129), Some((0x22, f + 75)));
    let q = fx.spawn(UnitType::Player, 0, a, 24, 20);
    fx.sim.with(&mut fx.game, |_, v| v.set_state(q, 57, true));
    assert!(!shrine_state(&mut fx, src, q, 129, 100));
}

/// A monster world that logs its umod dispatches; a mode-2 dispatch on
/// `victim` stands for a callback's area damage (the fire death burst)
/// hitting with `attacker` as the attacker, the world lent back around
/// the hit or not.
#[derive(Clone)]
struct HitWorld {
    log: std::rc::Rc<std::cell::RefCell<Vec<(UnitId, &'static str)>>>,
    victim: UnitId,
    attacker: UnitId,
    lend_back: bool,
}

impl crate::wiring::action::monsters::MonsterWorld<TestPending> for HitWorld {
    fn type_init(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<TestPending>,
        _: UnitId,
    ) {
    }
    fn umods(
        &mut self,
        sim: &mut crate::units::hooks::Sim<'_>,
        h: &mut ActionHooks<TestPending>,
        unit: UnitId,
        _: Option<UnitId>,
        mode: u8,
    ) {
        let tag = match mode {
            2 => "mode 2",
            3 => "mode 3",
            _ => "other",
        };
        self.log.borrow_mut().push((unit, tag));
        if mode != 2 || unit != self.victim {
            return;
        }
        let lent = self
            .lend_back
            .then(|| h.relend_monster_world(Box::new(self.clone())));
        View::of(&mut *sim.units, &mut *sim.stats, sim.data, h)
            .combat(sim.game)
            .monster_hit_hook(self.attacker);
        if let Some(out) = lent {
            assert!(h.take_relent_monster_world::<HitWorld>(out).is_some());
        }
        self.log.borrow_mut().push((unit, "callback goes on"));
    }
    fn assign_umod(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<TestPending>,
        _: UnitId,
        _: u8,
    ) {
    }
    fn forget(&mut self, _: UnitId) {}
    fn monster(&self, _: UnitId) -> Option<&crate::monsters::init::MonsterData> {
        None
    }
    fn monster_mut(&mut self, _: UnitId) -> Option<&mut crate::monsters::init::MonsterData> {
        None
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// Runs the victim's mode-2 dispatch on a [`HitWorld`]; the log.
fn umod_area_hit(lend_back: bool) -> (Fx, Vec<(UnitId, &'static str)>, UnitId, UnitId) {
    let mut fx = Fx::new();
    let attacker = fx.spawn(UnitType::Monster, 0, fx.a, 10, 10);
    let victim = fx.spawn(UnitType::Monster, 0, fx.a, 12, 10);
    let log = std::rc::Rc::default();
    fx.sim.sys.hooks.monster_world = Some(Box::new(HitWorld {
        log: std::rc::Rc::clone(&log),
        victim,
        attacker,
        lend_back,
    }));
    let ran = fx.sim.with(&mut fx.game, |g, v| {
        let mut sim = crate::units::hooks::Sim {
            game: g,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        v.h.run_umods(&mut sim, victim, None, 2)
    });
    assert!(ran);
    let out = log.borrow().clone();
    (fx, out, attacker, victim)
}

#[test]
fn a_umod_area_hit_runs_the_attackers_mode_3_nested() {
    // With the world lent back around the hit, the attacker's mode-3
    // dispatch runs inside it, before the mode-2 callback goes on.
    let (fx, log, a, v) = umod_area_hit(true);
    assert_eq!(log, [(v, "mode 2"), (a, "mode 3"), (v, "callback goes on")]);
    fx.assert_clean();
    // The world not lent back: a wiring error, the dispatch runs after
    // the outer one.
    let (fx, log, a, v) = umod_area_hit(false);
    assert_eq!(log, [(v, "mode 2"), (v, "callback goes on"), (a, "mode 3")]);
    assert_eq!(
        fx.sim.sys.hooks.errors,
        [WiringError::Reentrant("monster world")]
    );
}
