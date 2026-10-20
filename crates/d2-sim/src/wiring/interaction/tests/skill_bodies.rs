//! Tests for the skill bodies on the wired host: the start core and the do core of
//! [`crate::skills::use_`] call `srvst` / `srvdo` on
//! [`crate::wiring::interaction::UseView`], which runs the server-start and
//! server-do bodies of [`crate::skills::use_::bodies`] on the real stat lists,
//! states, timers, rooms and combat code. The fixture is the skill use tests'
//! (two 8×8 preset rooms, streamed).

use std::sync::Arc;

use d2_data::tables::Skills;

use crate::rules::{ModRules, Ruleset, SkillFn, SkillHost, SkillProvider};
use crate::skills::fake::skill_rec;
use crate::skills::use_::bodies::{
    self, apply_state, callback, charges_after, dec_quantity, group, BodyTables, StateRequest,
    DO_BODIES, START_BODIES,
};
use crate::skills::use_::{do_core, start, table, SkillFunctions};
use crate::skills::{SkillEntry, SkillTables};
use crate::stats::states::StateTable;
use crate::stats::StatData;
use crate::units::{UnitId, UnitType};

use super::skill_use::Fx;
use super::{stat_data, N_STATES, S_CURABLE};

const NONE16: u16 = 0xFFFF;
const NONE32: u32 = 0xFFFF_FFFF;

/// Skill ids of the test tables.
const KICK: i32 = 1;
const BUFF: i32 = 2;
const AMP: i32 = 3;
const DIM: i32 = 4;
const MIGHT: i32 = 5;
const PRG: i32 = 6;
/// A skill on a mod game's slots: srvst 95 and srvdo 200, both past the
/// tables.
const MODDED: i32 = 7;
const MOD_START: u16 = 95;
const MOD_DO: u16 = 200;
/// The stat the mod bodies set to their slot.
const MOD_MARK: u16 = 300;

/// States.
const AMP_STATE: u16 = 9;
const DIM_STATE: u16 = 23;
const BUFF_STATE: u16 = 30;
const BUFF_STATE_2: u16 = 31;
const MIGHT_STATE: u16 = 33;
const PGSV_STATE: u16 = 122;

/// Stats.
const DAMAGERESIST: u16 = 36;
const ARMORCLASS: u16 = 31;
const CURSE_RESISTANCE: u16 = 109;
const QUANTITY: u16 = 70;

/// Formula offsets in the code buffer (`code`): `push i16 v; end`, 4
/// bytes each.
const F_200: u32 = 0;
const F_10: u32 = 4;
const F_M100: u32 = 8;
const F_175: u32 = 12;
const F_50: u32 = 16;
const F_25: u32 = 20;
const F_M1: u32 = 24;

fn code() -> Vec<u8> {
    let mut c = Vec::new();
    for v in [200i16, 10, -100, 175, 50, 25, -1] {
        c.push(0x08);
        c.extend(v.to_le_bytes());
        c.push(0x00);
    }
    c
}

/// A skills record with no formulas, states, stats, events or missile.
pub(super) fn body_rec() -> Skills {
    let mut r = skill_rec();
    r.srvmissile = NONE16;
    r.intown = true;
    r.delay = NONE32;
    r.perdelay = NONE32;
    r.aurastate = NONE16;
    r.auratargetstate = NONE16;
    r.srvoverlay = NONE16;
    r.tgtoverlay = NONE16;
    r.passivestate = NONE16;
    for s in [
        &mut r.aurastat1,
        &mut r.aurastat2,
        &mut r.aurastat3,
        &mut r.aurastat4,
        &mut r.aurastat5,
        &mut r.aurastat6,
        &mut r.passivestat1,
        &mut r.passivestat2,
        &mut r.passivestat3,
        &mut r.passivestat4,
        &mut r.passivestat5,
        &mut r.auraevent1,
        &mut r.auraevent2,
        &mut r.auraevent3,
    ] {
        *s = NONE16;
    }
    for c in [
        &mut r.aurastatcalc2,
        &mut r.aurastatcalc3,
        &mut r.aurastatcalc4,
        &mut r.aurastatcalc5,
        &mut r.aurastatcalc6,
    ] {
        *c = NONE32;
    }
    r
}

fn tables() -> SkillTables {
    let mut v: Vec<Skills> = (0..8).map(|_| body_rec()).collect();
    // Kick: srvst 2, MinDam 0, HitShift 8.
    let k = &mut v[KICK as usize];
    k.srvstfunc = 2;
    k.hitshift = 8;
    // A defensive buff: srvdo 18, state 30 (group 7, with 31), duration
    // 200, armorclass 25.
    let b = &mut v[BUFF as usize];
    b.srvdofunc = 18;
    b.aurastate = BUFF_STATE;
    b.auralencalc = F_200;
    b.aurastat1 = ARMORCLASS;
    b.aurastatcalc1 = F_25;
    // Amplify Damage: srvdo 30, state 9 (curse), damageresist −100,
    // duration 200, range 10, filter 2 (monsters).
    let a = &mut v[AMP as usize];
    a.srvdofunc = 30;
    a.auratargetstate = AMP_STATE;
    a.auralencalc = F_200;
    a.aurarangecalc = F_10;
    a.aurastat1 = DAMAGERESIST;
    a.aurastatcalc1 = F_M100;
    a.aurafilter = 2;
    // Dim Vision: state 23 (an AI curse), duration ln34 = 175.
    let d = &mut v[DIM as usize];
    d.srvdofunc = 30;
    d.auratargetstate = DIM_STATE;
    d.auralencalc = F_175;
    d.aurarangecalc = F_10;
    d.aurastat1 = DAMAGERESIST;
    d.aurastatcalc1 = F_M1;
    d.aurafilter = 2;
    // Might: srvdo 65, aura state 33 on self, perdelay 50, no mana.
    let m = &mut v[MIGHT as usize];
    m.srvdofunc = 65;
    m.aura = true;
    m.aurastate = MIGHT_STATE;
    m.perdelay = F_50;
    m.aurarangecalc = F_10;
    m.aurafilter = 0x12003;
    // A progressive skill: prgdam 2, ln12 = Param1 10 + (lvl − 1) × 0.
    let p = &mut v[PRG as usize];
    p.prgdam = 2;
    p.param1 = 10;
    p.aurastat1 = 300;
    let o = &mut v[MODDED as usize];
    o.srvstfunc = MOD_START;
    o.srvdofunc = MOD_DO;
    let mut t = crate::skills::fake::skill_tables(v);
    t.skills_code = code();
    t
}

/// The interaction stat data with curse, pgsv and curable flags.
fn stats() -> Arc<StatData> {
    let mut d = (*stat_data()).clone();
    d.states = StateTable::synthetic(
        N_STATES,
        &[
            (u32::from(S_CURABLE), super::super::npc_world::STATE_CURABLE),
            (u32::from(AMP_STATE), group::CURSE),
            (u32::from(DIM_STATE), group::CURSE),
            (u32::from(PGSV_STATE), group::PGSV),
        ],
    );
    Arc::new(d)
}

fn body_tables() -> BodyTables {
    let mut g = vec![0; N_STATES];
    g[usize::from(BUFF_STATE)] = 7;
    g[usize::from(BUFF_STATE_2)] = 7;
    BodyTables {
        stats: vec![bodies::BodyStat::default(); 359],
        state_group: g,
        state_aura: vec![false; N_STATES],
        overlay_count: 0,
        ..BodyTables::default()
    }
}

fn entry(skill: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

/// The fixture with a player at (10, 10) and a monster at (12, 10)
/// (targetable, flags 0x2 | 0x4 | 0x8; monstats2 `isAtt`).
fn fx() -> (Fx, UnitId, UnitId, SkillTables) {
    let t = tables();
    let mut fx = Fx::with(stats(), t.clone());
    {
        let h = &mut fx.sim.sys.hooks;
        h.bodies = Some(Arc::new(body_tables()));
        let at = Arc::make_mut(&mut h.tables);
        at.combat.monstats2[0].isatt = true;
        at.combat.monstats[0].switchai = true;
        at.combat.difficultylevels[2].aicursedivisor = 4;
    }
    let p = fx.spawn(UnitType::Player, 10, 10);
    let m = fx.spawn(UnitType::Monster, 12, 10);
    fx.sim.sys.units.get_mut(m).unwrap().flags |= 0xE;
    (fx, p, m, t)
}

fn timers_12(fx: &Fx, u: UnitId) -> Vec<i32> {
    let t = &fx.game.timers;
    let mut v: Vec<i32> = t
        .unit_timers(u)
        .into_iter()
        .filter(|&id| t.event(id).is_some_and(|e| e.0 == 12))
        .filter_map(|id| t.expire(id))
        .collect();
    v.sort();
    v
}

/// (list expire, flags, stat `s`) of the unit's list of `state`.
fn state_list(fx: &mut Fx, u: UnitId, state: u16, s: u16) -> Option<(i32, u32, i32)> {
    fx.sim.with(&mut fx.game, |_, v| {
        let l = v.state_list(u, state)?;
        Some((v.stats.expire(l), v.stats.flags(l), v.stats.base(l, s, 0)))
    })
}

// ---- srvst ------------------------------------------------------------------

#[test]
fn kick_start_runs_on_the_wired_host() {
    let (mut fx, p, m, t) = fx();
    fx.game.coverage.enabled = true;
    fx.sim.sys.hooks.x.used.insert(p, entry(KICK));
    // No target: 0, no combat entry.
    assert_eq!(fx.sim.skill_use(&mut fx.game, |w| start(w, &t, p)), 0);
    assert!(!fx
        .game
        .coverage
        .rows
        .contains_key(&("skills_cast", KICK as u32)));
    assert!(fx
        .sim
        .sys
        .hooks
        .combat_lists
        .get(&p)
        .is_none_or(Vec::is_empty));
    // With a target: hit flags 2, result 9 (hit, knockback), physical 0,
    // hit class 1, a combat entry for the pair without a roll.
    fx.sim.sys.hooks.x.used.insert(p, entry(KICK));
    fx.sim.sys.hooks.x.targets.insert(p, m);
    let seed = fx.sim.sys.units.get(p).unwrap().seed;
    assert_eq!(fx.sim.skill_use(&mut fx.game, |w| start(w, &t, p)), 1);
    assert_eq!(fx.game.coverage.rows[&("skills_cast", KICK as u32)], 1);
    let list = &fx.sim.sys.hooks.combat_lists[&p];
    assert_eq!(list.len(), 1);
    let r = list[0].record;
    assert_eq!(r.hit_flags & 2, 2);
    assert_eq!((r.result, r.physical, r.hit_class), (9, 0, 1));
    assert_eq!(fx.sim.sys.units.get(p).unwrap().seed, seed, "no roll");
    fx.assert_clean();
}

// ---- srvdo 18 -----------------------------------------------------------------

#[test]
fn buff_do_runs_on_the_wired_host() {
    let (mut fx, p, _, t) = fx();
    fx.game.frame = 1000;
    // A state of the same group is on first: removed by `clear_group`.
    fx.sim.with(&mut fx.game, |_, v| {
        v.create_state_list(p, BUFF_STATE_2, None, 0);
        v.set_state(p, BUFF_STATE_2, true);
    });
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(BUFF)]);
    fx.sim.sys.hooks.x.used.insert(p, entry(BUFF));
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, BUFF, 1, false, false, false)
    });
    assert_eq!(r, 1);
    assert!(!fx.sim.sys.stats.has_state(p, u32::from(BUFF_STATE_2)));
    assert!(state_list(&mut fx, p, BUFF_STATE_2, 0).is_none());
    assert!(fx.sim.sys.stats.has_state(p, u32::from(BUFF_STATE)));
    // Duration 200 → expiry 1200 (flags 2), armorclass 25, 350/351.
    assert_eq!(
        state_list(&mut fx, p, BUFF_STATE, ARMORCLASS),
        Some((1200, 2, 25))
    );
    assert_eq!(state_list(&mut fx, p, BUFF_STATE, 350).unwrap().2, BUFF);
    assert_eq!(state_list(&mut fx, p, BUFF_STATE, 351).unwrap().2, 1);
    assert_eq!(timers_12(&fx, p), [1200]);
    assert_eq!(fx.sim.with(&mut fx.game, |_, v| v.stat(p, ARMORCLASS)), 25);
    // The list's remove callback is the buff's.
    let cb = fx.sim.with(&mut fx.game, |_, v| {
        v.state_list(p, BUFF_STATE)
            .and_then(|l| v.stats.remove_callback(l))
    });
    assert_eq!(cb.map(|c| c.0), Some(callback::BUFF));
    // The unit flag 0x40.
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags & 0x40, 0);
    fx.assert_clean();
}

// ---- srvdo 30 -----------------------------------------------------------------

#[test]
fn amplify_damage_scales_the_resist_of_an_immune_monster() {
    let (mut fx, p, m, t) = fx();
    fx.game.frame = 500;
    fx.set(m, &[(DAMAGERESIST, 100)]);
    fx.sim.sys.hooks.x.aim_at = (12, 10);
    fx.sim.sys.hooks.x.used.insert(p, entry(AMP));
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, AMP, 1, false, false, false)
    });
    assert_eq!(r, 1);
    // Duration 200; stat 36 = −100 / 5 = −20 in the curse list (state 9,
    // flags 0x20 | 2).
    assert!(fx.sim.sys.stats.has_state(m, u32::from(AMP_STATE)));
    assert_eq!(
        state_list(&mut fx, m, AMP_STATE, DAMAGERESIST),
        Some((700, 0x22, -20))
    );
    assert_eq!(timers_12(&fx, m), [700]);
    // The caster (a player, filter 2) is not cursed.
    assert!(!fx.sim.sys.stats.has_state(p, u32::from(AMP_STATE)));
    fx.assert_clean();
}

#[test]
fn dim_vision_duration_in_hell_and_ai_install() {
    let (mut fx, p, m, t) = fx();
    fx.sim.sys.data.difficulty = 2;
    fx.game.frame = 100;
    fx.sim.sys.hooks.x.aim_at = (12, 10);
    fx.sim.sys.hooks.x.used.insert(p, entry(DIM));
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, DIM, 1, false, false, false)
    });
    assert_eq!(r, 1);
    // ln34 = 175, AiCurseDivisor 4 (Hell): 43 frames.
    assert_eq!(
        state_list(&mut fx, m, DIM_STATE, DAMAGERESIST),
        Some((143, 0x22, -1))
    );
    // The curse AI (k = 10 for state 23).
    assert!(fx.sim.sys.hooks.x.log.contains(&format!("ai {} 10", m.0)));
    let cb = fx.sim.with(&mut fx.game, |_, v| {
        v.state_list(m, DIM_STATE)
            .and_then(|l| v.stats.remove_callback(l))
    });
    assert_eq!(cb.map(|c| c.0), Some(callback::AI_CURSE));
    fx.assert_clean();
}

#[test]
fn curse_resistance_halves_the_duration_and_lower_level_recast_is_refused() {
    let (mut fx, p, m, _) = fx();
    fx.game.frame = 10;
    fx.set(m, &[(CURSE_RESISTANCE, 50)]);
    let ct = fx.sim.sys.hooks.tables.combat.clone();
    let req = |lvl| StateRequest {
        source: p,
        target: m,
        skill: AMP,
        level: lvl,
        duration: 200,
        stat: -1,
        value: 0,
        state: i32::from(AMP_STATE),
        callback: 0,
    };
    let made = fx
        .sim
        .skill_use(&mut fx.game, |w| apply_state(w, &ct, req(2)).is_some());
    assert!(made);
    assert_eq!(state_list(&mut fx, m, AMP_STATE, 0).map(|x| x.0), Some(110));
    // Same state and skill at a lower level: refused, the list stays.
    let again = fx
        .sim
        .skill_use(&mut fx.game, |w| apply_state(w, &ct, req(1)).is_some());
    assert!(!again);
    assert_eq!(state_list(&mut fx, m, AMP_STATE, 0).map(|x| x.0), Some(110));
    // Resistance 100: none.
    fx.set(m, &[(CURSE_RESISTANCE, 100)]);
    let none = fx
        .sim
        .skill_use(&mut fx.game, |w| apply_state(w, &ct, req(3)).is_none());
    assert!(none);
    fx.assert_clean();
}

// ---- srvdo 65 -----------------------------------------------------------------

#[test]
fn might_aura_run_and_refresh() {
    let (mut fx, p, _, t) = fx();
    fx.sim.sys.hooks.x.used.insert(p, entry(MIGHT));
    fx.game.frame = 1251;
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, MIGHT, 1, false, false, false)
    });
    assert_eq!(r, 1);
    // perdelay 50: period 1301, duration 51: the self list expires at
    // 1302 with a type-12 timer there.
    assert!(fx.sim.sys.stats.has_state(p, u32::from(MIGHT_STATE)));
    assert_eq!(
        state_list(&mut fx, p, MIGHT_STATE, 0).map(|x| x.0),
        Some(1302)
    );
    assert_eq!(timers_12(&fx, p), [1302]);
    assert_eq!(state_list(&mut fx, p, MIGHT_STATE, 350).unwrap().2, MIGHT);
    let cb = fx.sim.with(&mut fx.game, |_, v| {
        v.state_list(p, MIGHT_STATE)
            .and_then(|l| v.stats.remove_callback(l))
    });
    assert_eq!(cb.map(|c| c.0), Some(callback::SELF_AURA));
    // The next run at 1301 refreshes the same list to 1352.
    fx.game.frame = 1301;
    fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, MIGHT, 1, false, false, false)
    });
    assert_eq!(
        state_list(&mut fx, p, MIGHT_STATE, 0).map(|x| x.0),
        Some(1352)
    );
    assert_eq!(timers_12(&fx, p), [1302, 1352]);
    fx.assert_clean();
}

// ---- helpers --------------------------------------------------------------------

#[test]
fn dec_quantity_uses_one_from_the_right_hand_stack() {
    let (mut fx, p, _, _) = fx();
    let i = fx.spawn(UnitType::Item, 10, 11);
    {
        let x = &mut fx.sim.sys.hooks.x;
        x.items.insert((p, 4), i);
        x.weapon.insert(p, i);
        x.stackable.push(i);
    }
    fx.set(i, &[(QUANTITY, 1)]);
    let r = fx.sim.skill_use(&mut fx.game, |w| dec_quantity(w, p));
    assert_eq!(r, 1);
    assert_eq!(fx.sim.with(&mut fx.game, |_, v| v.stat(i, QUANTITY)), 0);
    let guid = fx.sim.sys.units.get(i).unwrap().guid;
    assert!(fx
        .sim
        .sys
        .hooks
        .x
        .msgs
        .contains(&(p, crate::units::messages::update_item_stat(guid, 70, 0, 0))));
    // Quantity 0: stays 0, returns 0.
    let r = fx.sim.skill_use(&mut fx.game, |w| dec_quantity(w, p));
    assert_eq!(r, 0);
    assert_eq!(fx.sim.with(&mut fx.game, |_, v| v.stat(i, QUANTITY)), 0);
    fx.assert_clean();
}

#[test]
fn progressive_leech_charge_three() {
    let (mut fx, p, _, t) = fx();
    fx.sim.with(&mut fx.game, |_, v| {
        let l = v.create_state_list(p, PGSV_STATE, None, 0).unwrap();
        v.set_list_stat(l, 350, PRG);
        v.set_list_stat(l, 351, 1);
        v.set_list_stat(l, 300, 3);
        v.set_state(p, PGSV_STATE, true);
    });
    let mut rec = crate::combat::DamageRecord {
        result: 1,
        ..Default::default()
    };
    fx.sim
        .skill_use(&mut fx.game, |w| charges_after(w, &t, p, &mut rec));
    // ln12 = 10, n = 3: life and mana leech += 2 × 10.
    assert_eq!((rec.life_leech, rec.mana_leech), (20, 20));
    // No hit: nothing.
    let mut miss = crate::combat::DamageRecord::default();
    fx.sim
        .skill_use(&mut fx.game, |w| charges_after(w, &t, p, &mut miss));
    assert_eq!((miss.life_leech, miss.mana_leech), (0, 0));
    fx.assert_clean();
}

// ---- dispatch -------------------------------------------------------------------

#[test]
fn the_view_runs_exactly_the_specified_slots() {
    let (mut fx, p, _, t) = fx();
    let ct = fx.sim.sys.hooks.tables.combat.clone();
    let (starts, dos) = fx.sim.skill_use(&mut fx.game, |w| {
        let s: Vec<u16> = (0..table::START_SLOTS)
            .filter(|&i| bodies::run_start(w, &t, &ct, i, p, 99, 1).is_some())
            .collect();
        let d: Vec<u16> = (0..table::DO_SLOTS)
            .filter(|&i| bodies::run_do(w, &t, &ct, i, p, 99, 1).is_some())
            .collect();
        (s, d)
    });
    assert_eq!(starts, START_BODIES);
    assert_eq!(dos, DO_BODIES);
    // A slot without a body (only the unreferenced ones are left) goes
    // to the seam (the fixture's fake logs it); a body slot does not.
    fx.sim.sys.hooks.x.log.clear();
    fx.sim.skill_use(&mut fx.game, |w| {
        w.srvdo(53, p, 99, 1, true, false, false);
        w.srvdo(18, p, 99, 1, true, false, false);
    });
    assert_eq!(fx.sim.sys.hooks.x.log, ["srvdo 53"]);
}

// ---- mod skill bodies -----------------------------------------------------------

/// A mod game's skill bodies: start slot 95 and do slot 200 (past the
/// tables), and do slot 18 (the buff) when `buff` is set. Its bodies set
/// stat 300 to the slot index.
#[derive(Debug)]
struct ModBodies {
    buff: bool,
    answer: bool,
}

impl ModBodies {
    fn mark(&self, w: &mut dyn SkillHost, u: UnitId, index: u16) -> Option<i32> {
        if !self.answer {
            return None;
        }
        w.set_stat(u, MOD_MARK, i32::from(index));
        Some(1)
    }
}

impl SkillProvider for ModBodies {
    fn has(&self, kind: SkillFn, index: u16) -> bool {
        match kind {
            SkillFn::Start => index == MOD_START,
            SkillFn::Do => index == MOD_DO || (self.buff && index == 18),
        }
    }
    fn srvst(
        &self,
        w: &mut dyn SkillHost,
        index: u16,
        u: UnitId,
        _skill: i32,
        _lvl: i32,
    ) -> Option<i32> {
        self.mark(w, u, index)
    }
    fn srvdo(
        &self,
        w: &mut dyn SkillHost,
        index: u16,
        u: UnitId,
        _skill: i32,
        _lvl: i32,
        _charge: bool,
        _item: bool,
        _aim: bool,
    ) -> Option<i32> {
        self.mark(w, u, index)
    }
}

impl ModRules for ModBodies {
    fn name(&self) -> &str {
        "mod-bodies"
    }
    fn skills(&self) -> Option<&dyn SkillProvider> {
        Some(self)
    }
}

fn install(fx: &mut Fx, buff: bool, answer: bool) {
    fx.sim.sys.hooks.rules = Ruleset::Mod(Arc::new(ModBodies { buff, answer }));
}

fn mark(fx: &mut Fx, u: UnitId) -> i32 {
    fx.sim.with(&mut fx.game, |_, v| v.stat(u, MOD_MARK))
}

#[test]
fn original_rules_refuse_slots_past_the_tables() {
    let (mut fx, p, _, t) = fx();
    fx.sim.sys.hooks.x.used.insert(p, entry(MODDED));
    assert_eq!(fx.sim.skill_use(&mut fx.game, |w| start(w, &t, p)), 0);
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(MODDED)]);
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, MODDED, 1, false, false, false)
    });
    assert_eq!(r, 0);
    assert_eq!(mark(&mut fx, p), 0);
    assert!(fx.sim.sys.hooks.x.log.is_empty());
}

#[test]
fn a_mod_fills_slots_past_the_tables() {
    let (mut fx, p, _, t) = fx();
    install(&mut fx, false, true);
    fx.sim.sys.hooks.x.used.insert(p, entry(MODDED));
    assert_eq!(fx.sim.skill_use(&mut fx.game, |w| start(w, &t, p)), 1);
    assert_eq!(mark(&mut fx, p), i32::from(MOD_START));
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(MODDED)]);
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, MODDED, 1, false, false, false)
    });
    assert_eq!(r, 1);
    assert_eq!(mark(&mut fx, p), i32::from(MOD_DO));
    assert!(fx.sim.sys.hooks.x.log.is_empty());
}

#[test]
fn a_mod_slot_without_an_answer_keeps_the_original() {
    // Past the tables: refused as under the base game's rules.
    let (mut fx, p, _, t) = fx();
    install(&mut fx, true, false);
    fx.sim.sys.hooks.x.used.insert(p, entry(MODDED));
    assert_eq!(fx.sim.skill_use(&mut fx.game, |w| start(w, &t, p)), 0);
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(MODDED)]);
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, MODDED, 1, false, false, false)
    });
    assert_eq!(r, 0);
    // Inside the tables: the buff's own body runs.
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(BUFF)]);
    fx.sim.sys.hooks.x.used.insert(p, entry(BUFF));
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, BUFF, 1, false, false, false)
    });
    assert_eq!(r, 1);
    assert!(fx.sim.sys.stats.has_state(p, u32::from(BUFF_STATE)));
    assert_eq!(mark(&mut fx, p), 0);
    assert!(fx.sim.sys.hooks.x.log.is_empty());
}

#[test]
fn a_mod_overrides_a_body_inside_the_tables() {
    let (mut fx, p, _, t) = fx();
    install(&mut fx, true, true);
    fx.sim.sys.hooks.x.skills.insert(p, vec![entry(BUFF)]);
    fx.sim.sys.hooks.x.used.insert(p, entry(BUFF));
    let r = fx.sim.skill_use(&mut fx.game, |w| {
        do_core(w, &t, p, BUFF, 1, false, false, false)
    });
    assert_eq!(r, 1);
    assert_eq!(mark(&mut fx, p), 18);
    assert!(!fx.sim.sys.stats.has_state(p, u32::from(BUFF_STATE)));
}
