//! The passive-state stat list of a client unit: the refresh
//! with (unit, skill) over the
//! model's state lists, with the skills formulas evaluated by the `d2-sim`
//! evaluator (`skills::levels::eval_skill`) on a
//! [`SkillUnits`] view of the client model ([`ClientSkills`]).

use std::collections::BTreeMap;

use d2_sim::rng::Seed;
use d2_sim::skills::levels::eval_skill;
use d2_sim::skills::{SkillEntry as SimEntry, SkillUnits};

/// The skills tables the refresh evaluates (`ModelInputs::skill_tables`).
pub type Tables = d2_sim::skills::SkillTables;
use d2_sim::units::UnitType;

use super::dispatch::HandlerError;
use super::skills::{SkillEntry, SkillFx};
use super::world::{ClientWorld, ModelInputs, UnitKey};

/// The client model as the skills formulas read it (context): stats are the model's totals, states its bits and state
/// lists, the skill list the unit's. The client model holds no item
/// stat lists or body items: the item reads answer none / 0.
pub struct ClientSkills<'a> {
    pub world: &'a ClientWorld,
    /// The unit seed the formulas may draw from: a copy of the unit's
    /// client seed (the passive formulas draw none in the base game's tables;
    /// a draw would not reach the model).
    seed: Seed,
}

impl<'a> ClientSkills<'a> {
    pub fn new(world: &'a ClientWorld, key: UnitKey) -> Self {
        let (lo, hi) = world
            .units
            .get(&key)
            .and_then(|u| u.seed)
            .unwrap_or(super::world::INIT_SEED);
        ClientSkills {
            world,
            seed: Seed::new(lo, hi),
        }
    }
}

fn sim_entry(e: &SkillEntry) -> SimEntry {
    SimEntry {
        skill: i32::from(e.skill),
        base: e.base,
        level_bonus: e.level_bonus,
        owner_guid: e.owner as i32,
        charges: e.charges,
        has_charges: e.has_charges,
    }
}

impl SkillUnits for ClientSkills<'_> {
    type Unit = UnitKey;
    type Item = u32;

    fn unit_type(&self, u: UnitKey) -> UnitType {
        UnitType::ALL
            .get(usize::from(u.unit_type))
            .copied()
            .unwrap_or(UnitType::Tile)
    }
    fn class_id(&self, u: UnitKey) -> i32 {
        self.world.units.get(&u).map_or(0, |u| u.class as i32)
    }
    fn stat(&self, u: UnitKey, stat: u16, layer: u16) -> i32 {
        self.world.total(u, stat, layer)
    }
    fn item_stat(&self, u: UnitKey, stat: u16, layer: u16) -> i32 {
        self.world.total(u, stat, layer)
    }
    fn base_stat(&self, u: UnitKey, stat: u16, layer: u16) -> i32 {
        self.world.base(u, stat, layer)
    }
    fn formula_stat(&self, u: UnitKey, stat: u16, _mode: i32) -> i32 {
        self.world.total(u, stat, 0)
    }
    fn stat_entries(&self, u: UnitKey, stat: u16, max: usize) -> Vec<(u16, i32)> {
        let mut by_layer: BTreeMap<u16, i32> = BTreeMap::new();
        if let Some(unit) = self.world.units.get(&u) {
            let v = unit.stat(stat);
            if v != 0 {
                by_layer.insert(0, v);
            }
            for list in unit.state_lists.values() {
                for (&(s, l), &v) in list {
                    if s == stat {
                        *by_layer.entry(l).or_insert(0) += v;
                    }
                }
            }
        }
        by_layer.into_iter().take(max).collect()
    }
    fn has_state(&self, u: UnitKey, state: u16) -> bool {
        u8::try_from(state).is_ok_and(|s| {
            self.world
                .units
                .get(&u)
                .is_some_and(|u| u.states.contains(&s))
        })
    }
    fn state_stat(&self, u: UnitKey, state: u16, stat: u16) -> Option<i32> {
        let s = u8::try_from(state).ok()?;
        let list = self.world.units.get(&u)?.state_lists.get(&s)?;
        Some(list.get(&(stat, 0)).copied().unwrap_or(0))
    }
    fn seed(&mut self, _u: UnitKey) -> &mut Seed {
        &mut self.seed
    }
    fn skill_list(&self, u: UnitKey) -> Vec<SimEntry> {
        self.world
            .units
            .get(&u)
            .and_then(|u| u.skills.as_ref())
            .map_or_else(Vec::new, |l| l.entries.iter().map(sim_entry).collect())
    }
    fn used_skill(&self, u: UnitKey) -> Option<SimEntry> {
        let l = self.world.units.get(&u)?.skills.as_ref()?;
        l.current.and_then(|i| l.entries.get(i)).map(sim_entry)
    }
    fn current_weapon(&self, _u: UnitKey) -> Option<u32> {
        None
    }
    fn weapon(&self, _u: UnitKey) -> Option<u32> {
        None
    }
    fn item_at(&self, _u: UnitKey, _loc: u8) -> Option<u32> {
        None
    }
    fn item_is(&self, _item: u32, _itype: i32) -> bool {
        false
    }
    fn itype_is(&self, _itype: i32, _parent: i32) -> bool {
        false
    }
    fn wield_type(&self, _item: u32) -> i32 {
        0
    }
    fn item_damage(&self, _item: u32, _max: bool) -> i32 {
        0
    }
    fn str_dex_bonus(&self, _item: u32) -> (i32, i32) {
        (0, 0)
    }
    fn item_flag_throw(&self, _item: u32) -> bool {
        false
    }
    fn missile_level(&self, _u: UnitKey) -> i32 {
        0
    }
}

/// The highest entry of `skill` (native first,
/// then the highest base; charged entries skipped), as an index.
fn highest(entries: &[SkillEntry], skill: u16) -> Option<usize> {
    let sims: Vec<SimEntry> = entries.iter().map(sim_entry).collect();
    let best = d2_sim::skills::levels::highest_entry(&sims, i32::from(skill))?;
    sims.iter().position(|e| *e == best)
}

/// The refresh with (unit, skill).
/// Needs the skills tables (`Bridge::set_skill_tables`); without them a
/// passive skill is a handler error (the formulas are an input).
pub fn refresh(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
) -> Result<(), HandlerError> {
    let Some(t) = inputs.skill_tables.as_deref() else {
        return Err(HandlerError::Unspecified(
            "client/msg-skills.md §2 r4: the passive refresh needs the skills tables",
        ));
    };
    let Some(rec) = t.skill(i32::from(skill)) else {
        return Ok(());
    };
    // Only for a passive state p > 0 in range.
    let p = rec.passivestate as i16;
    if p <= 0 || p > i16::from(u8::MAX) {
        return Ok(());
    }
    let p = p as u8;
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    let entries = u.skills.as_ref().map_or(&[][..], |l| &l.entries[..]);
    let e = highest(entries, skill).map(|i| entries[i]);
    let aura = rec.aurastate as i16;
    let aura_on = aura > 0 && u.states.contains(&(aura as u8));
    let Some(e) = e.filter(|_| !aura_on) else {
        // No entry, or the aura state is on: the list of p is freed.
        w.units
            .get_mut(&key)
            .expect("looked up above")
            .state_lists
            .remove(&p);
        return Ok(());
    };
    let rows = &inputs.tables.skills[..];
    let desc = &inputs.tables.skilldesc[..];
    let l = super::msg::skills::level_with_bonuses(w, key, rows, desc, &e);
    if l == 0 {
        // with level 0 removes the list.
        w.units
            .get_mut(&key)
            .expect("looked up above")
            .state_lists
            .remove(&p);
        return Ok(());
    }
    let current_l = w.units[&key]
        .state_lists
        .get(&p)
        .and_then(|list| list.get(&(351, 0)))
        .copied();
    if current_l == Some(l) {
        // The list of p exists with stat 351 = L: unchanged.
        return Ok(());
    }
    // Stat 351 ≠ L: evaluate passivestat_i (i = 1…5, while a valid stat).
    let layer = (rec.passiveitype as i16).max(0) as u16;
    let pairs = [
        (rec.passivestat1, rec.passivecalc1),
        (rec.passivestat2, rec.passivecalc2),
        (rec.passivestat3, rec.passivecalc3),
        (rec.passivestat4, rec.passivecalc4),
        (rec.passivestat5, rec.passivecalc5),
    ];
    let mut sets = Vec::new();
    {
        let mut cs = ClientSkills::new(w, key);
        for (stat, calc) in pairs {
            let s = i32::from(stat as i16);
            if s < 0 || s >= t.stat_count {
                break;
            }
            let v = eval_skill(&mut cs, t, Some(key), calc, i32::from(skill), l);
            sets.push((s as u16, v));
        }
    }
    let list = w
        .units
        .get_mut(&key)
        .expect("looked up above")
        .state_lists
        .entry(p)
        .or_default();
    let mut set = |stat: u16, layer: u16, v: i32| {
        if v == 0 {
            list.remove(&(stat, layer));
        } else {
            list.insert((stat, layer), v);
        }
    };
    for (s, v) in sets {
        set(s, layer, v);
    }
    set(350, 0, i32::from(skill));
    set(351, 0, l);
    Ok(())
}

/// Applies what the skill-list operations owe the unit (steps 2.2 and
/// 4), in order: the passive state bit on / off and the refresh.
pub fn apply(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    fx: Vec<SkillFx>,
) -> Result<(), HandlerError> {
    for f in fx {
        match f {
            SkillFx::StateOn(s) => {
                if let Some(u) = w.units.get_mut(&key) {
                    u.states.insert(s);
                }
            }
            SkillFx::StateOff(s) => {
                if let Some(u) = w.units.get_mut(&key) {
                    u.states.remove(&s);
                }
            }
            SkillFx::Refresh(skill) => refresh(w, inputs, key, skill)?,
        }
    }
    Ok(())
}

/// The passive refresh of a unit (setfunc 9): every passive skill of the unit's list whose state is
/// on is refreshed, in skills-table order.
pub fn refresh_all(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), HandlerError> {
    let rows = &inputs.tables.skills;
    let mut todo = Vec::new();
    if let Some(u) = w.units.get(&key) {
        for (s, r) in rows.iter().enumerate() {
            let has = u.skills.as_ref().is_some_and(|l| {
                l.entries
                    .iter()
                    .any(|e| usize::from(e.skill) == s && !e.has_charges)
            });
            let state = r.passivestate as i16;
            if state > 0 && has && u.states.contains(&(state as u8)) {
                todo.push(s as u16);
            }
        }
    }
    for s in todo {
        refresh(w, inputs, key, s)?;
    }
    Ok(())
}
