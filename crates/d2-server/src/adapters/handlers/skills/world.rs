//! [`World`]: the skill seams of `d2-sim` for one message, on the skill
//! use pipeline's own `d2-sim` provider
//! ([`d2_sim::wiring::interaction::UseView`]: the action wiring's units,
//! stats, timers, missiles and its `Pending` value as `UseRest`).
//!
//! Every seam call goes to [`UseView`] except what the server holds for
//! the message ([`super::Staged`]): the last accepted point-target frame, the staged
//! positions, the owned-item and other-act results of the unit-target
//! lookup; the server messages, collected for the caller
//! ([`World::sends`]); and the mode start of the skill use pipeline,
//! which [`UseView`] hands to `UseRest::start_mode` without the unit
//! records: here it runs on them.
//! The skill-point calls (`LearnUnits`) go to the same `Pending` value
//! ([`super::LearnRest`]).

use d2_sim::combat::RoomKind;
use d2_sim::rng::Seed;
use d2_sim::skills::list::ListOwner;
use d2_sim::skills::use_::bodies::BodyWorld;
use d2_sim::skills::use_::{
    MissileAim, ModeTarget, ServerMsg, SkillFunctions, UseMissiles, UseState, UseWorld,
};
use d2_sim::skills::{LearnUnits, ManaUnits, SkillEntry, SkillUnits};
use d2_sim::units::hooks::Sim;
use d2_sim::units::record::flags;
use d2_sim::units::{modes, UnitId, UnitType};
use d2_sim::wiring::action::WiringError;
use d2_sim::wiring::interaction::{UseRest, UseView};

use super::{LearnRest, SkillRest, Staged};
use crate::dispatch::in_range;
use crate::seams::Pos;

/// One message's view of the wired sim.
pub struct World<'v, 'a, X> {
    /// The skill use pipeline's provider.
    pub u: &'v mut UseView<'a, X>,
    pub staged: Staged,
    /// Server messages the pipeline sent, in order.
    pub sends: Vec<ServerMsg>,
    /// The last accepted point-target frame written by the point validator.
    pub point_accept: Option<i32>,
}

impl<'v, 'a, X: SkillRest> World<'v, 'a, X> {
    pub fn new(u: &'v mut UseView<'a, X>, staged: Staged) -> Self {
        Self {
            u,
            staged,
            sends: Vec::new(),
            point_accept: None,
        }
    }

    /// The action wiring's `Pending` value (the skill list's owner).
    pub fn x_mut(&mut self) -> &mut X {
        &mut self.u.cv.v.h.x
    }

    fn x(&self) -> &X {
        &self.u.cv.v.h.x
    }

    /// The staged position of the message's unit, else the provider's.
    fn pos(&self, u: UnitId) -> Pos {
        match self.staged.positions.iter().find(|p| p.0 == u) {
            Some(&(_, p)) => p,
            None => {
                let (x, y) = UseWorld::position(&*self.u, u);
                Pos { x, y }
            }
        }
    }

    /// Runs `f` on the [`Sim`] of the wired unit system; a unit error is
    /// recorded in the wiring's error list.
    fn with_sim<R>(
        &mut self,
        f: impl FnOnce(
            &mut Sim<'_>,
            &mut d2_sim::wiring::action::ActionHooks<X>,
        ) -> Result<R, modes::UnitError>,
    ) -> Option<R> {
        let cv = &mut self.u.cv;
        let v = &mut cv.v;
        let r = {
            let mut sim = Sim {
                game: &mut *cv.game,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            };
            f(&mut sim, &mut *v.h)
        };
        match r {
            Ok(r) => Some(r),
            Err(e) => {
                v.h.errors.push(WiringError::Unit(e));
                None
            }
        }
    }
}

impl<X: SkillRest> SkillUnits for World<'_, '_, X> {
    type Unit = UnitId;
    type Item = UnitId;

    fn unit_type(&self, u: UnitId) -> UnitType {
        SkillUnits::unit_type(&*self.u, u)
    }
    fn class_id(&self, u: UnitId) -> i32 {
        SkillUnits::class_id(&*self.u, u)
    }
    fn stat(&self, u: UnitId, stat: u16, layer: u16) -> i32 {
        SkillUnits::stat(&*self.u, u, stat, layer)
    }
    fn item_stat(&self, u: UnitId, stat: u16, layer: u16) -> i32 {
        self.u.item_stat(u, stat, layer)
    }
    fn base_stat(&self, u: UnitId, stat: u16, layer: u16) -> i32 {
        SkillUnits::base_stat(&*self.u, u, stat, layer)
    }
    fn formula_stat(&self, u: UnitId, stat: u16, mode: i32) -> i32 {
        self.u.formula_stat(u, stat, mode)
    }
    fn stat_entries(&self, u: UnitId, stat: u16, max: usize) -> Vec<(u16, i32)> {
        self.u.stat_entries(u, stat, max)
    }
    fn has_state(&self, u: UnitId, state: u16) -> bool {
        SkillUnits::has_state(&*self.u, u, state)
    }
    fn state_stat(&self, u: UnitId, state: u16, stat: u16) -> Option<i32> {
        self.u.state_stat(u, state, stat)
    }
    fn seed(&mut self, u: UnitId) -> &mut Seed {
        SkillUnits::seed(&mut *self.u, u)
    }
    fn skill_list(&self, u: UnitId) -> Vec<SkillEntry> {
        self.u.skill_list(u)
    }
    fn used_skill(&self, u: UnitId) -> Option<SkillEntry> {
        self.u.used_skill(u)
    }
    fn current_weapon(&self, u: UnitId) -> Option<UnitId> {
        self.u.current_weapon(u)
    }
    fn weapon(&self, u: UnitId) -> Option<UnitId> {
        self.u.weapon(u)
    }
    fn item_at(&self, u: UnitId, loc: u8) -> Option<UnitId> {
        self.u.item_at(u, loc)
    }
    fn item_is(&self, item: UnitId, itype: i32) -> bool {
        self.u.item_is(item, itype)
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        self.u.itype_is(itype, parent)
    }
    fn wield_type(&self, item: UnitId) -> i32 {
        self.u.wield_type(item)
    }
    fn item_damage(&self, item: UnitId, max: bool) -> i32 {
        self.u.item_damage(item, max)
    }
    fn str_dex_bonus(&self, item: UnitId) -> (i32, i32) {
        self.u.str_dex_bonus(item)
    }
    fn item_flag_throw(&self, item: UnitId) -> bool {
        self.u.item_flag_throw(item)
    }
    fn missile_level(&self, u: UnitId) -> i32 {
        self.u.missile_level(u)
    }
}

impl<X: SkillRest> ManaUnits for World<'_, '_, X> {
    fn shapeshifted(&self, u: UnitId) -> bool {
        self.u.shapeshifted(u)
    }
    fn consume_charges(&mut self, u: UnitId, entry: &SkillEntry) -> bool {
        self.u.consume_charges(u, entry)
    }
    fn pay_life(&mut self, u: UnitId, cost: i32) -> bool {
        self.u.pay_life(u, cost)
    }
    fn set_stat(&mut self, u: UnitId, stat: u16, value: i32) {
        ManaUnits::set_stat(&mut *self.u, u, stat, value);
    }
}

impl<X: SkillRest> SkillFunctions for World<'_, '_, X> {
    fn srvst(&mut self, index: u16, u: UnitId, skill: i32, lvl: i32) -> i32 {
        SkillFunctions::srvst(&mut *self.u, index, u, skill, lvl)
    }
    fn srvdo(
        &mut self,
        index: u16,
        u: UnitId,
        skill: i32,
        lvl: i32,
        charge: bool,
        item: bool,
        aim: bool,
    ) -> i32 {
        SkillFunctions::srvdo(&mut *self.u, index, u, skill, lvl, charge, item, aim)
    }
}

impl<X: SkillRest> UseMissiles for World<'_, '_, X> {
    fn create_skill_missile(
        &mut self,
        u: UnitId,
        skill: i32,
        lvl: i32,
        missile: u16,
        lob: bool,
        aim: MissileAim,
    ) {
        self.u
            .create_skill_missile(u, skill, lvl, missile, lob, aim);
    }
}

impl<X: SkillRest> UseWorld for World<'_, '_, X> {
    fn record_cast(&mut self, skill: i32) {
        self.u.record_cast(skill);
    }
    fn frame(&self) -> i32 {
        UseWorld::frame(&*self.u)
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        UseWorld::tick_rate(&*self.u)
    }
    /// Collected for the caller (0x15 → resync, the rest recorded).
    fn send(&mut self, _u: UnitId, msg: ServerMsg) {
        self.sends.push(msg);
    }

    // ---- staged player data and positions
    fn has_player_data(&self, u: UnitId) -> bool {
        u == self.staged.player && self.staged.last_accept.is_some()
    }
    fn last_point_frame(&self, u: UnitId) -> i32 {
        match self.point_accept {
            Some(f) if u == self.staged.player => f,
            _ => self
                .staged
                .last_accept
                .filter(|_| u == self.staged.player)
                .unwrap_or(0),
        }
    }
    fn set_last_point_frame(&mut self, u: UnitId, frame: i32) {
        if u == self.staged.player {
            self.point_accept = Some(frame);
        }
    }
    fn position(&self, u: UnitId) -> (i32, i32) {
        let p = self.pos(u);
        (p.x, p.y)
    }
    fn find_unit(&self, ty: u32, guid: u32) -> Option<UnitId> {
        self.u.find_unit(ty, guid)
    }
    fn in_own_inventory(&self, u: UnitId, item: UnitId) -> bool {
        u == self.staged.player && self.staged.owned_item == Some(item)
    }
    /// The staged acts of the message's units; other pairs:
    /// the provider's (the unit records' acts).
    fn same_act(&self, a: UnitId, b: UnitId) -> bool {
        if self.staged.other_act.is_some_and(|t| t == a || t == b) {
            return false;
        }
        let staged = |u| self.staged.positions.iter().any(|p| p.0 == u);
        if staged(a) && staged(b) {
            return true;
        }
        self.u.same_act(a, b)
    }
    /// The reach test on the positions.
    fn within_reach(&self, a: UnitId, b: UnitId) -> bool {
        in_range(self.pos(a), self.pos(b))
    }
    fn owner(&self, u: UnitId) -> Option<UnitId> {
        self.u.owner(u)
    }

    // ---- skills
    fn left_skill(&self, u: UnitId) -> Option<SkillEntry> {
        self.u.left_skill(u)
    }
    fn right_skill(&self, u: UnitId) -> Option<SkillEntry> {
        self.u.right_skill(u)
    }
    fn set_left_skill(&mut self, u: UnitId, e: SkillEntry) {
        self.u.set_left_skill(u, e);
    }
    fn set_right_skill(&mut self, u: UnitId, e: SkillEntry) {
        self.u.set_right_skill(u, e);
    }
    fn find_entry(&self, u: UnitId, skill: i32) -> Option<SkillEntry> {
        self.u.find_entry(u, skill)
    }
    fn find_entry_owned(&self, u: UnitId, skill: i32, owner: i32) -> Option<SkillEntry> {
        self.u.find_entry_owned(u, skill, owner)
    }
    fn owns_skill(&self, u: UnitId, skill: i32) -> bool {
        self.u.owns_skill(u, skill)
    }
    fn set_used_skill(&mut self, u: UnitId, e: Option<SkillEntry>) {
        self.u.set_used_skill(u, e);
    }
    fn used_skill_flags(&self, u: UnitId) -> u32 {
        self.u.used_skill_flags(u)
    }
    fn set_used_skill_flags(&mut self, u: UnitId, f: u32) {
        self.u.set_used_skill_flags(u, f);
    }
    fn entry_mode(&self, u: UnitId, e: &SkillEntry) -> u32 {
        self.u.entry_mode(u, e)
    }
    fn attack_param4(&self, u: UnitId) -> i32 {
        self.u.attack_param4(u)
    }
    fn set_attack_param4(&mut self, u: UnitId, v: i32) {
        self.u.set_attack_param4(u, v);
    }
    /// The level test reads `skill_level(unit, entry, 1)`: the base level
    /// plus the bonus levels (item stats 97 / 107 of a worn oskill item, all-skills,
    /// class and tab bonuses), not the entry's stored bonus alone.
    fn use_state(&mut self, u: UnitId, e: &SkillEntry) -> UseState {
        let level = d2_sim::skills::levels::skill_level(
            &*self,
            &self.u.cv.v.h.tables.skills,
            Some(u),
            Some(e),
            true,
        );
        let e = SkillEntry {
            base: level,
            level_bonus: 0,
            ..*e
        };
        self.u.use_state(u, &e)
    }
    fn dec_quantity(&mut self, u: UnitId, skill: i32) {
        self.u.dec_quantity(u, skill);
    }

    // ---- equipment
    fn can_dual_wield(&self, u: UnitId) -> bool {
        self.u.can_dual_wield(u)
    }
    fn equippable(&self, item: UnitId) -> bool {
        self.u.equippable(item)
    }
    fn bow_equipped(&self, u: UnitId) -> bool {
        self.u.bow_equipped(u)
    }
    fn state_mask(&self, u: UnitId, mask: u32) -> bool {
        self.u.state_mask(u, mask)
    }
    fn in_melee_range(&self, u: UnitId, target: UnitId) -> bool {
        self.u.in_melee_range(u, target)
    }

    // ---- modes
    fn mode(&self, u: UnitId) -> u32 {
        self.u.mode(u)
    }
    fn cursor_item(&self, u: UnitId) -> bool {
        self.u.cursor_item(u)
    }
    fn endanim_expire(&self, u: UnitId) -> i32 {
        self.u.endanim_expire(u)
    }
    fn set_mode(&mut self, u: UnitId, mode: u32) {
        self.u.set_mode(u, mode);
    }
    /// The mode start, in its order: set the mode, clear the target, keep
    /// the new one, delete type-0/1 timers, schedule the frame events
    /// (`animate`), clear flag 0x40.
    ///
    /// [`UseView`] hands the start to `UseRest::start_mode`, which cannot
    /// reach the unit records; until it runs the start itself (wanted
    /// change, `docs/handoff/host-merge.md`), it runs here on the same
    /// providers.
    fn start_mode(&mut self, u: UnitId, mode: u32, target: ModeTarget<UnitId>) {
        // Where the target goes: the point form stores the point (target
        // unit cleared), the unit form stores the unit; the seam value keeps
        // it (`UseRest::keep_target`) for the skill's missile and checks.
        if self
            .with_sim(|sim, h| modes::set_mode(sim, h, u, mode))
            .is_none()
        {
            return;
        }
        UseRest::clear_target(self.x_mut(), u);
        UseRest::keep_target(self.x_mut(), u, target);
        self.with_sim(|sim, h| modes::animate(sim, h, u));
        d2_sim::wiring::interaction::body_path::point_target(&mut self.u.cv.v, u, target);
        if let Some(r) = self.u.cv.v.units.get_mut(u) {
            r.flags &= !flags::ATTACK_PENDING;
        }
    }
    fn run_to(&mut self, u: UnitId, target: UnitId, e: SkillEntry) {
        self.u.run_to(u, target, e);
    }
    fn target(&self, u: UnitId) -> Option<UnitId> {
        UseWorld::target(&*self.u, u)
    }
    fn clear_target(&mut self, u: UnitId) {
        self.u.clear_target(u);
    }
    fn unit_flags(&self, u: UnitId) -> u32 {
        self.u.unit_flags(u)
    }
    fn set_unit_flags(&mut self, u: UnitId, f: u32) {
        self.u.set_unit_flags(u, f);
    }
    fn event_arg(&self, u: UnitId) -> i32 {
        self.u.event_arg(u)
    }
    fn set_event_arg(&mut self, u: UnitId, a: i32) {
        self.u.set_event_arg(u, a);
    }
    fn step_path(&mut self, u: UnitId) -> i32 {
        self.u.step_path(u)
    }
    fn is_alive(&self, u: UnitId) -> bool {
        self.u.is_alive(u)
    }

    // ---- relations, rooms, line of sight
    fn is_hostile(&self, a: UnitId, b: UnitId) -> bool {
        self.u.is_hostile(a, b)
    }
    fn is_pet(&self, a: UnitId, b: UnitId) -> bool {
        self.u.is_pet(a, b)
    }
    fn is_ally(&self, a: UnitId, b: UnitId) -> bool {
        self.u.is_ally(a, b)
    }
    fn room(&self, u: UnitId) -> RoomKind {
        self.u.room(u)
    }
    fn target_position(&self, u: UnitId) -> Option<(i32, i32)> {
        self.u.target_position(u)
    }
    fn line_clear(&self, u: UnitId, to: (i32, i32), mask: u32) -> bool {
        self.u.line_clear(u, to, mask)
    }

    // ---- timers and stat lists
    fn schedule(&mut self, u: UnitId, kind: u8, frame: i32, arg1: i32, arg2: i32) {
        self.u.schedule(u, kind, frame, arg1, arg2);
    }
    fn delete_timers(&mut self, u: UnitId, kind: u8, arg1: i32) {
        self.u.delete_timers(u, kind, arg1);
    }
    fn has_state_list(&self, u: UnitId, state: u16) -> bool {
        self.u.has_state_list(u, state)
    }
    fn create_delay_list(&mut self, u: UnitId, expire: i32) {
        self.u.create_delay_list(u, expire);
    }
    fn set_state_list_expiry(&mut self, u: UnitId, state: u16, expire: i32) {
        self.u.set_state_list_expiry(u, state, expire);
    }
    fn free_aura_state(&mut self, u: UnitId, state: u16) {
        self.u.free_aura_state(u, state);
    }
    fn set_aura_state(&mut self, u: UnitId, state: u16, skill: i32, lvl: i32) {
        self.u.set_aura_state(u, state, skill, lvl);
    }
}

impl<X: SkillRest> LearnUnits for World<'_, '_, X> {
    /// The skill's `charclass` is the player's class.
    fn is_class_skill(&self, u: UnitId, skill: i32) -> bool {
        let h = &*self.u.cv.v.h;
        let class = SkillUnits::class_id(&*self.u, u);
        LearnRest::is_class_skill(self.x(), u, skill)
            || h.tables
                .skills
                .skill(skill)
                .is_some_and(|r| i32::from(r.charclass as i8) == class)
    }
    /// After the cost check: the cost comes off `newskills(5)`, the native
    /// entry of the player's list gains a level (`SkillList::add`), and the
    /// client is told (S→C 0x21: unit type, remove 0, GUID, skill, base
    /// level, bonus level).
    fn add_skill_level(&mut self, u: UnitId, skill: i32, cost: i32) {
        d2_sim::combat::vitals::VitalsUnits::add_base_stat(
            &mut self.u.cv.v,
            u,
            5,
            cost.wrapping_neg(),
        );
        let class = SkillUnits::class_id(&*self.u, u);
        let guid = self.u.cv.v.units.get(u).map_or(0, |r| r.guid);
        let h = &mut *self.u.cv.v.h;
        let rows = &h.tables.skills.skills;
        let list = h.skill_lists.entry(u).or_default();
        let Some(i) = list.add(rows, ListOwner::player(class), skill) else {
            return;
        };
        let level = list.entries[i].base;
        let entry = d2_sim::skills::levels::highest_entry(&list.view(), skill);
        // The skill's passive state on, then the refresh (Critical Strike,
        // Dodge ...).
        let passive = h
            .tables
            .skills
            .skill(skill)
            .map_or(-1, |r| i32::from(r.passivestate as i16));
        if passive > 0 {
            self.u.cv.v.set_state(u, passive as u16, true);
            if let Some(entry) = entry {
                BodyWorld::passive_state_apply(&mut *self.u, u, &entry);
            }
        }
        // The native entry's bonus levels.
        let native = self
            .u
            .cv
            .v
            .h
            .skill_lists
            .get(&u)
            .and_then(|l| l.view().get(i).copied());
        let bonus = native.map_or(0, |e| {
            d2_sim::skills::levels::bonus_level(&*self, &self.u.cv.v.h.tables.skills, u, &e)
        });
        let h = &mut *self.u.cv.v.h;
        let mut m = vec![0x21, 0, 0];
        m.extend_from_slice(&guid.to_le_bytes());
        m.extend_from_slice(&(skill as u16).to_le_bytes());
        m.extend_from_slice(&[level as u8, bonus as u8, 0]);
        d2_sim::wiring::action::Pending::send(&mut h.x, u, &m);
        LearnRest::add_skill_level(self.x_mut(), u, skill, cost);
    }
}
