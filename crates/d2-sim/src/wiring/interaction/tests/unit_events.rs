//! Tests for the unit event registry on the action wiring: it runs the
//! handlers of `ActionHooks::handlers` through `combat::events::run` on
//! the skill use view once `ActionHooks::enable_unit_events` is called;
//! combat's events and the missile's event 0 reach it.

use crate::combat::{CombatWorld, DamageRecord};
use crate::missiles::MissileCombat;
use crate::skills::use_::bodies::Handler;
use crate::units::{UnitId, UnitType};

use super::skill_use::{skills, Fx};
use super::stat_data;

/// `restinpeace`.
const RESTINPEACE: u32 = 172;
/// Function 29: state 172 on O.
const FN_REST_IN_PEACE: i32 = 29;

fn handler(event: u8, key_type: i32, key: i32) -> Handler {
    Handler {
        event,
        key_type,
        key,
        skill: 0,
        level: 1,
        func: FN_REST_IN_PEACE,
    }
}

fn fx() -> (Fx, UnitId, UnitId) {
    let mut fx = Fx::with(stat_data(), skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    let m = fx.spawn(UnitType::Monster, 12, 10);
    (fx, p, m)
}

fn rest(fx: &Fx, u: UnitId) -> bool {
    fx.sim.sys.stats.has_state(u, RESTINPEACE)
}

#[test]
fn registry_runs_matching_handlers_and_drops_key_type_zero() {
    let (mut fx, p, m) = fx();
    fx.sim.sys.hooks.enable_unit_events();
    // Head first: a one-shot (key type 0) of event 9, a kept one (key
    // type 1) of event 9, one of another event.
    fx.sim.sys.hooks.handlers.insert(
        p,
        vec![handler(9, 0, 5), handler(9, 1, 7), handler(8, 1, 8)],
    );
    let mut rec = DamageRecord::default();
    fx.sim.with(&mut fx.game, |g, v| {
        CombatWorld::unit_event(&mut v.combat(g), 9, p, m, &mut rec)
    });
    assert!(rest(&fx, m));
    assert!(!rest(&fx, p));
    assert_eq!(
        fx.sim.sys.hooks.handlers[&p],
        vec![handler(9, 1, 7), handler(8, 1, 8)]
    );
    fx.assert_clean();
}

#[test]
fn missile_event_zero_reaches_the_registry() {
    let (mut fx, p, m) = fx();
    fx.sim.sys.hooks.enable_unit_events();
    fx.sim.sys.hooks.handlers.insert(m, vec![handler(0, 1, 1)]);
    // O is the missile (here the player stands in for it) and the
    // unit may be none.
    fx.sim.with(&mut fx.game, |g, v| {
        v.hit_by_missile_event(g, p, None);
        v.hit_by_missile_event(g, p, Some(m));
    });
    assert!(rest(&fx, p));
    assert_eq!(fx.sim.sys.hooks.handlers[&m].len(), 1);
}

#[test]
fn without_the_registry_events_stay_on_the_seam() {
    let (mut fx, p, m) = fx();
    fx.sim.sys.hooks.handlers.insert(p, vec![handler(9, 0, 5)]);
    let mut rec = DamageRecord::default();
    fx.sim.with(&mut fx.game, |g, v| {
        CombatWorld::unit_event(&mut v.combat(g), 9, p, m, &mut rec)
    });
    assert!(!rest(&fx, m));
    assert_eq!(fx.sim.sys.hooks.handlers[&p].len(), 1);
}

/// `item_skillonhit` (198): layer skill × 64 + level, value the chance.
const SKILLONHIT: u16 = 198;
/// Event 5 (`domeleedamage`), function 20 (skill on hit).
const EV_DOMELEEDAMAGE: u8 = 5;

/// [`stat_data`] with stat 198's `fCallback` and item event 1 = 5,
/// function 20, as the `itemstatcost.txt` row has it (event 2 left unset).
fn skillonhit_data() -> std::sync::Arc<crate::stats::StatData> {
    let mut t = super::itemstatcost();
    let size = t.record_size;
    let r = &mut t.records[usize::from(SKILLONHIT) * size..][..size];
    r[5] |= 0x08; // fCallback
    r[72..74].copy_from_slice(&u16::from(EV_DOMELEEDAMAGE).to_le_bytes());
    r[76..78].copy_from_slice(&20u16.to_le_bytes());
    let mut d = (*stat_data()).clone();
    d.stats = crate::stats::StatTable::from_fixed(&t).expect("itemstatcost");
    std::sync::Arc::new(d)
}

/// [`skills`] plus skill 1 with an item effect (`core` skips skill 0
/// and effect 0).
fn item_cast_skills() -> crate::skills::SkillTables {
    let base = skills();
    let mut s1 = base.skills[0].clone();
    s1.itemeffect = 1;
    let mut t = crate::skills::fake::skill_tables(vec![base.skills[0].clone(), s1]);
    t.missiles = base.missiles;
    t
}

#[test]
fn skill_on_hit_stat_registers_and_queues_the_item_cast() {
    use crate::wiring::action::event_records::EventRecord;
    let mut fx = Fx::with(skillonhit_data(), item_cast_skills());
    let p = fx.spawn(UnitType::Player, 10, 10);
    let m = fx.spawn(UnitType::Monster, 12, 10);
    fx.sim.sys.hooks.enable_unit_events();
    // Skill 1 level 1 at 100 %.
    let layer = 64 + 1;
    let k = (i32::from(SKILLONHIT) << 16) | layer;
    fx.sim.with(&mut fx.game, |_, v| {
        v.stats
            .unit_set(&mut *v.h, p, SKILLONHIT, 100, layer as u16);
    });
    let h = &fx.sim.sys.hooks.handlers[&p];
    assert_eq!(h.len(), 1);
    assert_eq!(
        (h[0].event, h[0].key_type, h[0].key, h[0].skill, h[0].func),
        (EV_DOMELEEDAMAGE, 2, k, k, 20)
    );
    let mut rec = DamageRecord {
        hit_flags: 0x20,
        ..DamageRecord::default()
    };
    fx.sim.with(&mut fx.game, |g, v| {
        CombatWorld::unit_event(&mut v.combat(g), EV_DOMELEEDAMAGE, p, m, &mut rec)
    });
    let casts: Vec<_> = fx
        .sim
        .sys
        .hooks
        .event_records
        .of(p)
        .iter()
        .map(|r| match r {
            EventRecord::CastOnUnit { skill, level, .. } => Some((*skill, *level)),
            _ => None,
        })
        .collect();
    assert_eq!(casts, vec![Some((1, 1))]);
    // The stat cleared drops the handler.
    fx.sim.with(&mut fx.game, |_, v| {
        v.stats.unit_set(&mut *v.h, p, SKILLONHIT, 0, layer as u16);
    });
    assert!(fx.sim.sys.hooks.handlers[&p].is_empty());
}

/// Prevent heal's curse: state 52.
const STATE_PREVENTHEAL: u32 = 52;

#[test]
fn combat_curse_applies_the_state_once_the_registry_is_on() {
    let (mut fx, p, m) = fx();
    // A player target (a monster one needs monstats2 `isatt`, which the
    // fake class lacks). Without the registry the curse stays on the seam.
    fx.sim.with(&mut fx.game, |g, v| {
        CombatWorld::curse(&mut v.combat(g), p, m, 52, 31, 0, 120000, 0, 1)
    });
    assert!(!fx.sim.sys.stats.has_state(p, STATE_PREVENTHEAL));
    fx.sim.sys.hooks.enable_unit_events();
    fx.sim.with(&mut fx.game, |g, v| {
        CombatWorld::curse(&mut v.combat(g), p, m, 52, 31, 0, 120000, 0, 1)
    });
    assert!(fx.sim.sys.stats.has_state(p, STATE_PREVENTHEAL));
    // The target is queued for the next client pass.
    assert!(fx.game.lists.unit(p).is_some_and(|e| e.is_queued()));
}
