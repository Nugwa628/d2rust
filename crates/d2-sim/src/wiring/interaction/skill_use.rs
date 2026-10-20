//! Skill use → missiles, combat and the timers: the seams of
//! [`crate::skills::use_`] on the action wiring's providers.
//!
//! [`UseView`] wraps the action wiring's
//! [`crate::wiring::action::combat::CombatView`] (game, unit records,
//! stat lists, shared action state), so [`SkillUnits`] is combat's.
//! Real here: the frame, GUID lookups, the act test, unit modes (the
//! plain mode set), unit flags, the alive test, the ENDANIM expire,
//! hostility and melee range (the action wiring's seams), the room kind,
//! timer scheduling and deletion, the skill-delay and aura state lists,
//! stat writes, and missile creation through
//! [`crate::missiles::create_missile`] on the real missile store. The
//! skill bodies that are written run on this view ([`BodyWorld`], below):
//! real stat lists, states, timers, rooms, the combat code and the
//! handler lists of [`ActionHooks`]; the parts of other unwritten systems
//! through [`Pending`]'s skill-body seams.
//! The skill list, player data, paths, items and the other per-skill
//! bodies have no provider: [`UseRest`], implemented by the same
//! [`Pending`] value as the action wiring's other open seams (one owner
//! for the skill list, which combat reads through
//! [`Pending::skill_list`]).

use crate::combat::events::{self, EventTables, EventWorld, ItemCastMsg, RaiseStep};
use crate::combat::{CombatWorld, RoomKind};
use crate::game::Game;
use crate::missiles::{self, MissileParams};
use crate::rng::Seed;
use crate::skills::use_::bodies::{self, BodyWorld};
use crate::skills::use_::table::{self, Kind};
use crate::skills::use_::{
    MissileAim, ModeTarget, ServerMsg, SkillFunctions, UseMissiles, UseState, UseWorld,
};
use crate::skills::{KickItems, ManaUnits, SkillEntry, SkillUnits};
use crate::stats::lists::{ListId, RemoveCallback};
use crate::tick::events::event;
use crate::units::anim::{self, Form};
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::combat::CombatView;
use crate::wiring::action::units::ALIGNMENT_CALLBACKS;
use crate::wiring::action::{ActionSim, Pending, View, WiringError};
use crate::wiring::interaction::body_path;

/// `skilldelay` (state 121).
const STATE_SKILL_DELAY: u16 = 121;
/// Host-only request to remove an equipped stack after its final throw.
pub const EXHAUST_STACK_MARK: u8 = 0xF5;
/// State 114 `blood_mana` (test 6 pays from life) and 12 `inferno` (test 8).
const STATE_BLOOD_MANA: u32 = 114;
const STATE_INFERNO: u32 = 12;
/// The were-forms' `srvdofunc`.
const SRVDOFUNC_WERE: i16 = 116;
/// List flags of the delay list.
const DELAY_LIST_FLAGS: u32 = 2;
/// The delay list's remove callback (an opaque id; the stat host gives it
/// its meaning).
pub const DELAY_REMOVE_CALLBACK: RemoveCallback = RemoveCallback(0x0056_E900);

/// The skill use calls with no provider of their own yet. Implemented by
/// the action wiring's [`Pending`] provider (the skill list's one owner).
pub trait UseRest {
    // ---- messages (d2-server)
    fn send(&mut self, u: UnitId, msg: ServerMsg);
    // ---- player data
    fn has_player_data(&self, u: UnitId) -> bool;
    fn last_point_frame(&self, u: UnitId) -> i32;
    fn set_last_point_frame(&mut self, u: UnitId, frame: i32);
    fn cursor_item(&self, u: UnitId) -> bool;
    // ---- relations, reach (rules not written yet)
    fn in_own_inventory(&self, u: UnitId, item: UnitId) -> bool;
    fn within_reach(&self, a: UnitId, b: UnitId) -> bool;
    fn owner(&self, u: UnitId) -> Option<UnitId>;
    fn is_pet(&self, a: UnitId, b: UnitId) -> bool;
    fn is_ally(&self, a: UnitId, b: UnitId) -> bool;
    // ---- the unit's skill list (units / skills)
    fn left_skill(&self, u: UnitId) -> Option<SkillEntry>;
    fn right_skill(&self, u: UnitId) -> Option<SkillEntry>;
    fn set_left_skill(&mut self, u: UnitId, e: SkillEntry);
    fn set_right_skill(&mut self, u: UnitId, e: SkillEntry);
    fn find_entry(&self, u: UnitId, skill: i32) -> Option<SkillEntry>;
    fn find_entry_owned(&self, u: UnitId, skill: i32, owner: i32) -> Option<SkillEntry>;
    fn owns_skill(&self, u: UnitId, skill: i32) -> bool;
    /// The entry [`Pending::used_skill`] then returns.
    fn set_used_skill(&mut self, u: UnitId, e: Option<SkillEntry>);
    fn used_skill_flags(&self, u: UnitId) -> u32;
    fn set_used_skill_flags(&mut self, u: UnitId, f: u32);
    fn entry_mode(&self, u: UnitId, e: &SkillEntry) -> u32;
    fn attack_param4(&self, u: UnitId) -> i32;
    fn set_attack_param4(&mut self, u: UnitId, v: i32);
    /// The skill use state check, in parts; the order of the parts is
    /// not known.
    fn use_state(&mut self, u: UnitId, e: &SkillEntry) -> UseState;
    // ---- mana
    fn shapeshifted(&self, u: UnitId) -> bool;
    fn consume_charges(&mut self, u: UnitId, e: &SkillEntry) -> bool;
    fn pay_life(&mut self, u: UnitId, cost: i32) -> bool;
    // ---- equipment (items, inventory)
    fn can_dual_wield(&self, u: UnitId) -> bool;
    fn equippable(&self, item: UnitId) -> bool;
    fn bow_equipped(&self, u: UnitId) -> bool;
    fn state_mask(&self, u: UnitId, mask: u32) -> bool;
    /// The weapon in use and the item on a body location as the skill
    /// bodies see them (ammo check, bow missile), ahead of combat's
    /// [`Pending::current_weapon`] / [`Pending::item_at`]: a host whose
    /// equipped items do not yet feed the stat lists lets the skills see
    /// its weapon without changing the melee damage rules. Default: not
    /// answered (combat's).
    fn skill_weapon(&self, _u: UnitId) -> Option<UnitId> {
        None
    }
    fn skill_item_at(&self, _u: UnitId, _loc: u8) -> Option<UnitId> {
        None
    }
    /// The facts of a worn item the item type test's matched-item rules
    /// read: item flag 0x4000 or 0x100, the `shoots` type of its primary
    /// type (itemtypes; 0 none) and the items-row `stackable`. Default:
    /// not answered (the rules do not act: no flag, no `shoots`).
    fn hand_item_facts(&self, _item: UnitId) -> Option<HandItemFacts> {
        None
    }
    /// The damage weapon of a unit using a skill with `weapsel`; `idx` is
    /// the frame event index, `flag` the used entry's bit 0x2000.
    /// Default: not answered.
    fn skill_damage_weapon(
        &self,
        _u: UnitId,
        _weapsel: u8,
        _idx: i32,
        _flag: bool,
    ) -> Option<Option<UnitId>> {
        None
    }
    // ---- modes and paths
    fn start_mode(&mut self, game: &mut Game, u: UnitId, mode: u32, target: ModeTarget<UnitId>);
    fn run_to(&mut self, u: UnitId, target: UnitId, e: SkillEntry);
    fn target(&self, u: UnitId) -> Option<UnitId>;
    fn clear_target(&mut self, u: UnitId);
    /// The target of a mode start: the point or unit the skill's missile
    /// and checks read back through [`UseRest::target`] and
    /// [`UseRest::target_position`]. Default: not kept.
    fn keep_target(&mut self, _u: UnitId, _target: ModeTarget<UnitId>) {}
    fn event_arg(&self, u: UnitId) -> i32;
    fn set_event_arg(&mut self, u: UnitId, a: i32);
    fn step_path(&mut self, u: UnitId) -> i32;
    fn target_position(&self, u: UnitId) -> Option<(i32, i32)>;
    fn line_clear(&self, u: UnitId, to: (i32, i32), mask: u32) -> bool;
    // ---- aura state (list contents not written)
    fn set_aura_state(&mut self, u: UnitId, state: u16, skill: i32, lvl: i32);
    // ---- skill code (the slots `bodies` does not cover)
    fn srvst(&mut self, index: u16, u: UnitId, skill: i32, lvl: i32) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn srvdo(
        &mut self,
        index: u16,
        u: UnitId,
        skill: i32,
        lvl: i32,
        charge: bool,
        item: bool,
        aim: bool,
    ) -> i32;
}

/// The skill use pipeline's world: combat's view plus the skill use
/// seams.
pub struct UseView<'a, X> {
    pub cv: CombatView<'a, X>,
}

impl<X: Pending + UseRest> UseView<'_, X> {
    /// Refresh all for the skill stat callbacks (stats 83, 126, 127, 188)
    /// the stat write that just returned raised: run now, not at the
    /// unit's next update.
    fn drain_skill_refresh(&mut self) {
        let v = &mut self.cv.v;
        if v.h.skill_refresh.is_empty() {
            return;
        }
        let mut sim = crate::units::hooks::Sim {
            game: &mut *self.cv.game,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        super::skill_events::drain_skill_refresh(&mut *v.h, &mut sim);
    }

    /// `can_afford(unit, entry)` (use state test 6): an item entry needs
    /// charges; else L = base + bonus (≥ 0, at most the level cap), cost =
    /// (`mana` + `lvlmana` · max(L − 1, 0)) << `manashift`, paid from life
    /// (stat 6) with state 114, else a were-form skill (`srvdofunc` 116)
    /// while shapeshifted passes, else from mana (stat 8).
    fn can_afford(&self, u: UnitId, e: &SkillEntry) -> bool {
        if !e.is_native() {
            return e.charges > 0;
        }
        let t = &self.cv.v.h.tables.skills;
        let Some(r) = t.skill(e.skill) else {
            return false;
        };
        let l = e.base.wrapping_add(e.level_bonus).max(0).min(t.level_cap);
        let n = (l - 1).max(0);
        let cost = i32::from(r.mana as i16)
            .wrapping_add(i32::from(r.lvlmana as i16).wrapping_mul(n))
            .wrapping_shl(u32::from(r.manashift as u8) & 31);
        let stats = &self.cv.v.stats;
        if stats.has_state(u, STATE_BLOOD_MANA) {
            return cost <= stats.unit_total(u, 6, 0);
        }
        if r.srvdofunc as i16 == SRVDOFUNC_WERE && self.x().shapeshifted(u) {
            return true;
        }
        cost <= stats.unit_total(u, 8, 0)
    }
    /// The start stat test (use state test 8): a player with a
    /// `startmana` > 0 skill needs mana ≥ `startmana` << 8, unless it has
    /// state 12 (`inferno`).
    fn start_stat_ok(&self, u: UnitId, e: &SkillEntry) -> bool {
        let Some(r) = self.cv.v.h.tables.skills.skill(e.skill) else {
            return true;
        };
        let start = i32::from(r.startmana as i16);
        let player = self
            .cv
            .v
            .units
            .get(u)
            .is_some_and(|r| r.ty == UnitType::Player);
        if !player || start <= 0 {
            return true;
        }
        let stats = &self.cv.v.stats;
        stats.unit_total(u, 8, 0) >= start << 8 || stats.has_state(u, STATE_INFERNO)
    }
    /// The item type test of `use_state` test 5, the shared
    /// [`crate::skills::use_::item_type::item_type_test`] over this view:
    /// the hands, the inventory presence, the weapon in use, and the
    /// matched item's flags, `shoots` and stackable from the host
    /// ([`UseRest::hand_item_facts`]), its stats 157 / 70 from the stat
    /// lists (check `w1-w7-ama-bow-noquiver`: a bow and no quiver is
    /// refused).
    fn weapon_type_ok(&self, u: UnitId, skill: i32) -> bool {
        let Some(r) = self.cv.v.h.tables.skills.skill(skill) else {
            return false;
        };
        let k = crate::skills::use_::item_type::ItemTypeSkill::of(skill, r);
        crate::skills::use_::item_type::item_type_test(&UseHands { v: self, u }, &k)
    }
}

/// The skill use view's answers to the item type test.
struct UseHands<'v, 'a, X> {
    v: &'v UseView<'a, X>,
    u: UnitId,
}

impl<X: Pending + UseRest> crate::skills::use_::item_type::ItemTypeView for UseHands<'_, '_, X> {
    type Item = UnitId;
    fn hand_item(&self, loc: u8) -> Option<UnitId> {
        self.v.item_at(self.u, loc)
    }
    fn has_inventory(&self) -> bool {
        self.v.x().has_inventory(self.u)
    }
    fn weapon_in_use(&self) -> Option<UnitId> {
        self.v.current_weapon(self.u)
    }
    fn is_a(&self, item: UnitId, itype: i32) -> bool {
        self.v.item_is(item, itype)
    }
    fn ammo_free(&self, item: UnitId) -> bool {
        self.v
            .x()
            .hand_item_facts(item)
            .is_some_and(|f| f.ammo_free)
    }
    fn shoots(&self, item: UnitId) -> i32 {
        self.v.x().hand_item_facts(item).map_or(0, |f| f.shoots)
    }
    fn stat(&self, item: UnitId, stat: u16) -> i32 {
        self.v.cv.v.stats.unit_total(item, stat, 0)
    }
    fn stackable(&self, item: UnitId) -> bool {
        self.v
            .x()
            .hand_item_facts(item)
            .is_some_and(|f| f.stackable)
    }
}

/// [`UseRest::hand_item_facts`]: what the item type test's matched-item
/// rules read of one worn item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandItemFacts {
    /// Item flag 0x4000 or 0x100 set.
    pub ammo_free: bool,
    /// The primary type's itemtypes `shoots` row, 0 none.
    pub shoots: i32,
    /// items `stackable` (> 0).
    pub stackable: bool,
}

impl<X: Pending + UseRest> ActionSim<X> {
    /// Runs `f` with the skill use pipeline's view (message handlers,
    /// the do / periodic event handlers, tests).
    pub fn skill_use<R>(&mut self, game: &mut Game, f: impl FnOnce(&mut UseView<'_, X>) -> R) -> R {
        let s = &mut self.sys;
        let mut w = UseView {
            cv: CombatView {
                game,
                v: View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks),
            },
        };
        f(&mut w)
    }
}

impl<X: Pending + UseRest> UseView<'_, X> {
    /// The unit's list in `ActionHooks::skill_lists`, when it has one: the
    /// list calls answer from it, the seam otherwise.
    fn list(&self, u: UnitId) -> Option<&crate::skills::list::SkillList> {
        self.cv.v.h.skill_lists.get(&u)
    }
    fn dynamic_path(&self, u: UnitId) -> Option<&crate::path::DynamicPath> {
        self.cv.v.h.paths.as_ref()?.dynamic(u)
    }
    fn list_entry(&self, u: UnitId, e: &SkillEntry) -> Option<&crate::skills::list::ListEntry> {
        let l = self.list(u)?;
        l.entries.get(l.find(e.skill, e.owner_guid)?)
    }
    fn list_entry_mut(
        &mut self,
        u: UnitId,
        e: &SkillEntry,
    ) -> Option<&mut crate::skills::list::ListEntry> {
        let l = self.list_mut(u)?;
        let i = l.find(e.skill, e.owner_guid)?;
        l.entries.get_mut(i)
    }
    fn list_mut(&mut self, u: UnitId) -> Option<&mut crate::skills::list::SkillList> {
        self.cv.v.h.skill_lists.get_mut(&u)
    }
    pub(super) fn x(&self) -> &X {
        &self.cv.v.h.x
    }
    pub(super) fn xm(&mut self) -> &mut X {
        &mut self.cv.v.h.x
    }
    fn error(&mut self, e: WiringError) {
        self.cv.v.h.errors.push(e);
    }
    /// Runs an animation variant on the unit's own animation fields:
    /// cancel its type-0 / type-1 events, reschedule, and update the
    /// unit's animation record.
    fn anim_variant(&mut self, u: UnitId, form: Form) {
        let Some(rec) = self.cv.v.units.get_mut(u) else {
            return;
        };
        if let Err(e) = anim::run(self.cv.game, u, &mut rec.anim, form) {
            self.error(WiringError::Unit(e.into()));
        }
    }
}

// ---- SkillUnits: combat's ------------------------------------------------

impl<X: Pending + UseRest> SkillUnits for UseView<'_, X> {
    type Unit = UnitId;
    type Item = UnitId;

    fn rules(&self) -> &crate::rules::Ruleset {
        self.cv.rules()
    }

    fn unit_type(&self, u: UnitId) -> UnitType {
        self.cv.unit_type(u)
    }
    fn class_id(&self, u: UnitId) -> i32 {
        self.cv.class_id(u)
    }
    fn stat(&self, u: UnitId, stat: u16, layer: u16) -> i32 {
        SkillUnits::stat(&self.cv, u, stat, layer)
    }
    fn item_stat(&self, u: UnitId, stat: u16, layer: u16) -> i32 {
        SkillUnits::item_stat(&self.cv, u, stat, layer)
    }
    fn base_stat(&self, u: UnitId, stat: u16, layer: u16) -> i32 {
        SkillUnits::base_stat(&self.cv, u, stat, layer)
    }
    fn formula_stat(&self, u: UnitId, stat: u16, mode: i32) -> i32 {
        self.cv.formula_stat(u, stat, mode)
    }
    fn stat_entries(&self, u: UnitId, stat: u16, max: usize) -> Vec<(u16, i32)> {
        self.cv.stat_entries(u, stat, max)
    }
    fn has_state(&self, u: UnitId, state: u16) -> bool {
        SkillUnits::has_state(&self.cv, u, state)
    }
    fn state_stat(&self, u: UnitId, state: u16, stat: u16) -> Option<i32> {
        SkillUnits::state_stat(&self.cv, u, state, stat)
    }
    fn seed(&mut self, u: UnitId) -> &mut Seed {
        SkillUnits::seed(&mut self.cv, u)
    }
    fn skill_list(&self, u: UnitId) -> Vec<SkillEntry> {
        self.cv.skill_list(u)
    }
    fn used_skill(&self, u: UnitId) -> Option<SkillEntry> {
        self.cv.used_skill(u)
    }
    fn current_weapon(&self, u: UnitId) -> Option<UnitId> {
        if let Some(r) = self
            .cv
            .used_skill(u)
            .and_then(|e| self.cv.v.h.tables.skills.skill(e.skill))
        {
            let idx = self.x().event_arg(u);
            let flag = self.used_skill_flags(u) & 0x2000 != 0;
            if let Some(w) = self.x().skill_damage_weapon(u, r.weapsel, idx, flag) {
                return w;
            }
        }
        self.x()
            .skill_weapon(u)
            .or_else(|| self.cv.current_weapon(u))
    }
    fn weapon(&self, u: UnitId) -> Option<UnitId> {
        self.x().skill_weapon(u).or_else(|| self.cv.weapon(u))
    }
    fn item_at(&self, u: UnitId, loc: u8) -> Option<UnitId> {
        self.x()
            .skill_item_at(u, loc)
            .or_else(|| self.cv.item_at(u, loc))
    }
    fn item_is(&self, item: UnitId, itype: i32) -> bool {
        self.cv.item_is(item, itype)
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        self.cv.itype_is(itype, parent)
    }
    fn wield_type(&self, item: UnitId) -> i32 {
        self.cv.wield_type(item)
    }
    fn item_damage(&self, item: UnitId, max: bool) -> i32 {
        self.cv.item_damage(item, max)
    }
    fn str_dex_bonus(&self, item: UnitId) -> (i32, i32) {
        self.cv.str_dex_bonus(item)
    }
    fn item_flag_throw(&self, item: UnitId) -> bool {
        self.cv.item_flag_throw(item)
    }
    fn missile_level(&self, u: UnitId) -> i32 {
        self.cv.missile_level(u)
    }
}

impl<X: Pending + UseRest> ManaUnits for UseView<'_, X> {
    fn shapeshifted(&self, u: UnitId) -> bool {
        self.x().shapeshifted(u)
    }
    fn consume_charges(&mut self, u: UnitId, entry: &SkillEntry) -> bool {
        self.xm().consume_charges(u, entry)
    }
    fn pay_life(&mut self, u: UnitId, cost: i32) -> bool {
        self.xm().pay_life(u, cost)
    }
    /// Sets the unit's base stat.
    fn set_stat(&mut self, u: UnitId, stat: u16, value: i32) {
        self.cv.v.set_base(u, stat, value);
    }
}

/// A mod game's slots ([`SkillFunctions::mod_slot`]) ask its
/// [`crate::rules::SkillProvider`] first; without an answer they do what D2RUST does
/// for the slot. The bodies of `functions.tsv` status `spec'd-here` run
/// here ([`bodies::run_start`] / [`bodies::run_do`] on this view); every
/// other slot goes to [`UseRest::srvst`] / [`UseRest::srvdo`].
impl<X: Pending + UseRest> SkillFunctions for UseView<'_, X> {
    fn mod_slot(&self, kind: Kind, index: u16) -> bool {
        self.cv.v.h.rules.has_skill_slot(kind, index)
    }
    fn srvst(&mut self, index: u16, u: UnitId, skill: i32, lvl: i32) -> i32 {
        if self.mod_slot(Kind::Start, index) {
            let rules = self.cv.v.h.rules.clone();
            if let Some(v) = rules
                .skills()
                .and_then(|p| p.srvst(self, index, u, skill, lvl))
            {
                return v;
            }
            // The start core's empty slots (past the table: 0; no
            // function: 1) and its bodies that read nothing.
            if index >= table::START_SLOTS {
                return 0;
            }
            if table::lookup(Kind::Start, index).is_none() {
                return 1;
            }
            if let Some(v) = bodies::start(index) {
                return v;
            }
        }
        let t = self.cv.v.h.tables.clone();
        match bodies::run_start(self, &t.skills, &t.combat, index, u, skill, lvl) {
            Some(v) => v,
            None => self.xm().srvst(index, u, skill, lvl),
        }
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
        if self.mod_slot(Kind::Do, index) {
            let rules = self.cv.v.h.rules.clone();
            let answer = rules
                .skills()
                .and_then(|p| p.srvdo(self, index, u, skill, lvl, charge, item, aim));
            if let Some(v) = answer {
                return v;
            }
            // The do core skips an empty slot: nothing happened.
            if table::lookup(Kind::Do, index).is_none() {
                return 0;
            }
        }
        let t = self.cv.v.h.tables.clone();
        match bodies::run_do(self, &t.skills, &t.combat, index, u, skill, lvl) {
            Some(v) => v,
            None => self.xm().srvdo(index, u, skill, lvl, charge, item, aim),
        }
    }
}

impl<X: Pending + UseRest> UseMissiles for UseView<'_, X> {
    /// The `srvmissile` path: `skill_missile` (a different routine for
    /// `lob`) with `quant` = 0, on the real missile store
    /// ([`BodyWorld::spawn_missile`]).
    fn create_skill_missile(
        &mut self,
        u: UnitId,
        skill: i32,
        lvl: i32,
        missile: u16,
        lob: bool,
        aim: MissileAim,
    ) {
        let (d, at) = match aim {
            MissileAim::None => ((0, 0), (0, 0)),
            MissileAim::At { offset, aim } => (offset, aim),
        };
        bodies::skill_missile(self, i32::from(missile), u, skill, lvl, d, at, false, lob);
    }
}

impl<X: Pending + UseRest> UseWorld for UseView<'_, X> {
    fn record_cast(&mut self, skill: i32) {
        self.cv.game.coverage.hit("skills_cast", || skill as u32);
    }
    fn frame(&self) -> i32 {
        self.cv.game.frame
    }
    fn tick_rate(&self) -> crate::tick::TickRate {
        self.cv.game.tick_rate
    }
    fn send(&mut self, u: UnitId, msg: ServerMsg) {
        UseRest::send(self.xm(), u, msg);
    }
    fn has_player_data(&self, u: UnitId) -> bool {
        self.x().has_player_data(u)
    }
    fn last_point_frame(&self, u: UnitId) -> i32 {
        self.x().last_point_frame(u)
    }
    fn set_last_point_frame(&mut self, u: UnitId, frame: i32) {
        self.xm().set_last_point_frame(u, frame);
    }
    /// The path position (`ActionHooks::path_position`: the path
    /// provider's, else [`Pending::position`]).
    fn position(&self, u: UnitId) -> (i32, i32) {
        self.cv.v.h.path_position(u)
    }
    /// Looks the GUID up in the type's hash.
    fn find_unit(&self, ty: u32, guid: u32) -> Option<UnitId> {
        let ty = *UnitType::ALL.get(usize::try_from(ty).ok()?)?;
        self.cv.game.lists.find_unit(ty, guid)
    }
    fn in_own_inventory(&self, u: UnitId, item: UnitId) -> bool {
        self.x().in_own_inventory(u, item)
    }
    /// The act fields are equal.
    fn same_act(&self, a: UnitId, b: UnitId) -> bool {
        let act = |u| self.cv.v.units.get(u).map(|r| r.act);
        act(a).is_some() && act(a) == act(b)
    }
    fn within_reach(&self, a: UnitId, b: UnitId) -> bool {
        self.x().within_reach(a, b)
    }
    fn owner(&self, u: UnitId) -> Option<UnitId> {
        self.x().owner(u)
    }

    fn left_skill(&self, u: UnitId) -> Option<SkillEntry> {
        match self.list(u) {
            Some(l) => l.left.and_then(|i| l.view().get(i).copied()),
            None => self.x().left_skill(u),
        }
    }
    fn right_skill(&self, u: UnitId) -> Option<SkillEntry> {
        match self.list(u) {
            Some(l) => l.right.and_then(|i| l.view().get(i).copied()),
            None => self.x().right_skill(u),
        }
    }
    fn set_left_skill(&mut self, u: UnitId, e: SkillEntry) {
        match self.list_mut(u) {
            Some(l) => l.left = l.find(e.skill, e.owner_guid),
            None => self.xm().set_left_skill(u, e),
        }
    }
    fn set_right_skill(&mut self, u: UnitId, e: SkillEntry) {
        match self.list_mut(u) {
            Some(l) => l.right = l.find(e.skill, e.owner_guid),
            None => self.xm().set_right_skill(u, e),
        }
    }
    fn find_entry(&self, u: UnitId, skill: i32) -> Option<SkillEntry> {
        match self.list(u) {
            Some(l) => l.view().into_iter().find(|e| e.skill == skill),
            None => self
                .x()
                .find_entry(u, skill)
                .or_else(|| self.cv.v.h.monster_entry_of(u, skill)),
        }
    }
    fn find_entry_owned(&self, u: UnitId, skill: i32, owner: i32) -> Option<SkillEntry> {
        match self.list(u) {
            Some(l) => l.find(skill, owner).and_then(|i| l.view().get(i).copied()),
            None => self.x().find_entry_owned(u, skill, owner),
        }
    }
    fn owns_skill(&self, u: UnitId, skill: i32) -> bool {
        match self.list(u) {
            Some(l) => l.has(skill),
            None => self.x().owns_skill(u, skill),
        }
    }
    fn set_used_skill(&mut self, u: UnitId, e: Option<SkillEntry>) {
        match self.list_mut(u) {
            Some(l) => {
                l.current = e.and_then(|e| l.find(e.skill, e.owner_guid));
                if l.current.is_none() {
                    self.xm().set_used_skill(u, e);
                }
            }
            None => self.xm().set_used_skill(u, e),
        }
    }
    /// The used entry's flags word: the skill list's entry when the unit
    /// has one, else the seam.
    fn used_skill_flags(&self, u: UnitId) -> u32 {
        match self.list(u).and_then(|l| l.entries.get(l.current?)) {
            Some(e) => e.flags,
            None => self.x().used_skill_flags(u),
        }
    }
    fn set_used_skill_flags(&mut self, u: UnitId, f: u32) {
        match self.list_mut(u).and_then(|l| l.entries.get_mut(l.current?)) {
            Some(e) => e.flags = f,
            None => self.xm().set_used_skill_flags(u, f),
        }
    }
    fn entry_mode(&self, u: UnitId, e: &SkillEntry) -> u32 {
        match self.list(u) {
            Some(l) => l
                .find(e.skill, e.owner_guid)
                .map_or(0, |i| l.entries[i].mode),
            None => self.x().entry_mode(u, e),
        }
    }
    fn attack_param4(&self, u: UnitId) -> i32 {
        self.x().attack_param4(u)
    }
    fn set_attack_param4(&mut self, u: UnitId, v: i32) {
        self.xm().set_attack_param4(u, v);
    }
    /// `use_state`: tests 1–4 from the host, test 5's item type test,
    /// test 6 (`can_afford`) and test 8 (the start stat) here: checks
    /// `w2-04-skill-cast-nomana` / `-xy` (Fire Bolt with the server's mana
    /// poked to 1: no mode, and a point cast answers S→C 0x5A 0x0E).
    // PROVISIONAL: tests 7 (shape), 9 (charges) and 10 (cooldown) are
    // not run on the server yet.
    fn use_state(&mut self, u: UnitId, e: &SkillEntry) -> UseState {
        let st = self.xm().use_state(u, e);
        if st != UseState::Usable {
            return st;
        }
        if !self.weapon_type_ok(u, e.skill) {
            return UseState::NoQuantity;
        }
        if !self.can_afford(u, e) || !self.start_stat_ok(u, e) {
            return UseState::NoMana;
        }
        st
    }
    /// Decrements the quantity of the unit's used item.
    fn dec_quantity(&mut self, u: UnitId, _skill: i32) {
        bodies::dec_quantity(self, u);
    }

    fn can_dual_wield(&self, u: UnitId) -> bool {
        self.x().can_dual_wield(u)
    }
    fn equippable(&self, item: UnitId) -> bool {
        self.x().equippable(item)
    }
    fn bow_equipped(&self, u: UnitId) -> bool {
        self.x().bow_equipped(u)
    }
    fn state_mask(&self, u: UnitId, mask: u32) -> bool {
        self.x().state_mask(u, mask)
    }
    /// Melee range, with the third argument the combat code takes: whether
    /// the target is in a moving mode, 0 or 1 (the action wiring's
    /// seams).
    fn in_melee_range(&self, u: UnitId, target: UnitId) -> bool {
        use crate::combat::CombatWorld;
        let moving = i32::from(self.cv.moving_mode(target));
        self.cv.in_melee_range(u, target, moving)
    }

    /// The unit record's mode.
    fn mode(&self, u: UnitId) -> u32 {
        self.cv.v.units.get(u).map_or(0, |r| r.mode)
    }
    fn cursor_item(&self, u: UnitId) -> bool {
        self.x().cursor_item(u)
    }
    /// The smallest positive expire frame of the unit's type-1 timers, 0
    /// if none.
    fn endanim_expire(&self, u: UnitId) -> i32 {
        let t = &self.cv.game.timers;
        t.unit_timers(u)
            .into_iter()
            .filter(|&id| t.event(id).is_some_and(|e| e.0 == event::END_ANIM))
            .filter_map(|id| t.expire(id))
            .filter(|&e| e > 0)
            .min()
            .unwrap_or(0)
    }
    /// The plain mode set.
    fn set_mode(&mut self, u: UnitId, mode: u32) {
        let r = {
            let v = &mut self.cv.v;
            let mut sim = crate::units::hooks::Sim {
                game: self.cv.game,
                units: v.units,
                stats: v.stats,
                data: v.data,
            };
            crate::units::modes::set_mode(&mut sim, &mut *v.h, u, mode)
        };
        if let Err(e) = r {
            self.error(WiringError::Unit(e));
        }
    }
    fn start_mode(&mut self, u: UnitId, mode: u32, target: ModeTarget<UnitId>) {
        let game = &mut *self.cv.game;
        self.cv.v.h.x.start_mode(game, u, mode, target);
        self.cv.v.h.x.keep_target(u, target);
    }
    /// The unit-form run request on the path provider; without one the
    /// host's seam.
    fn run_to(&mut self, u: UnitId, target: UnitId, e: SkillEntry) {
        if self.cv.v.h.paths.is_some() {
            let mut p = crate::wiring::path::walk::PathCtx::of(&mut self.cv.v, &mut *self.cv.game);
            p.run_to_unit(u, target, e.skill as u16);
        } else {
            self.xm().run_to(u, target, e);
        }
    }
    fn target(&self, u: UnitId) -> Option<UnitId> {
        UseRest::target(self.x(), u)
    }
    fn clear_target(&mut self, u: UnitId) {
        self.xm().clear_target(u);
    }
    /// The unit record's flags.
    fn unit_flags(&self, u: UnitId) -> u32 {
        self.cv.v.units.get(u).map_or(0, |r| r.flags)
    }
    fn set_unit_flags(&mut self, u: UnitId, f: u32) {
        if let Some(r) = self.cv.v.units.get_mut(u) {
            r.flags = f;
        }
    }
    fn event_arg(&self, u: UnitId) -> i32 {
        self.x().event_arg(u)
    }
    fn set_event_arg(&mut self, u: UnitId, a: i32) {
        self.xm().set_event_arg(u, a);
    }
    /// The path provider's step (2 when the path is finished); without
    /// one the host's seam.
    fn step_path(&mut self, u: UnitId) -> i32 {
        if self.cv.v.h.paths.is_none() {
            return self.xm().step_path(u);
        }
        let mut p = crate::wiring::path::walk::PathCtx::of(&mut self.cv.v, &mut *self.cv.game);
        let st = p.step(u);
        match st {
            Some(crate::path::walk::Step::Stopped) => 2,
            _ => 0,
        }
    }
    /// The alive test on the unit record.
    fn is_alive(&self, u: UnitId) -> bool {
        !self.cv.v.units.is_dead(u)
    }

    /// Hostility (the combat view's [`CombatWorld::hostile`]).
    fn is_hostile(&self, a: UnitId, b: UnitId) -> bool {
        CombatWorld::hostile(&self.cv, a, b)
    }
    fn is_pet(&self, a: UnitId, b: UnitId) -> bool {
        self.x().is_pet(a, b)
    }
    fn is_ally(&self, a: UnitId, b: UnitId) -> bool {
        self.x().is_ally(a, b)
    }
    /// Combat's room kind.
    fn room(&self, u: UnitId) -> RoomKind {
        CombatWorld::room(&self.cv, u)
    }
    /// The host's answer, else (a monster's skill, which has no player
    /// skill target) the path's target: the target unit's position, or
    /// the path's target point (read-only here: no stale-target
    /// clearing). Both coordinates must be non-zero.
    fn target_position(&self, u: UnitId) -> Option<(i32, i32)> {
        if let Some(p) = self.x().target_position(u) {
            return Some(p);
        }
        let d = self.cv.v.h.paths.as_ref()?.dynamic(u)?;
        let (x, y) = match d.target_unit {
            Some(t)
                if t.unit != u && self.cv.game.lists.find_unit(t.ty, t.guid) == Some(t.unit) =>
            {
                self.cv.v.h.path_position(t.unit)
            }
            _ => (i32::from(d.target_x), i32::from(d.target_y)),
        };
        (x != 0 && y != 0).then_some((x, y))
    }
    fn line_clear(&self, u: UnitId, to: (i32, i32), mask: u32) -> bool {
        self.rooms_line_clear(u, to, mask)
            .unwrap_or_else(|| self.x().line_clear(u, to, mask))
    }

    /// Schedules a timer event of `kind` for the unit.
    fn schedule(&mut self, u: UnitId, kind: u8, frame: i32, arg1: i32, arg2: i32) {
        let r =
            self.cv
                .game
                .schedule_event(u, u32::from(kind), frame, None, arg1 as u32, arg2 as u32);
        if let Err(e) = r {
            self.error(WiringError::Unit(e.into()));
        }
    }
    /// Deletes the unit's timers of `kind` with that arg1; argument 0 =
    /// any argument (monster regeneration relies on it, and a monster's
    /// AI cancels every think this way).
    fn delete_timers(&mut self, u: UnitId, kind: u8, arg1: i32) {
        let arg = (arg1 != 0).then_some(arg1 as u32);
        self.cv.game.timers.cancel_unit_events(u, kind, arg);
    }

    fn has_state_list(&self, u: UnitId, state: u16) -> bool {
        self.cv.v.state_list(u, state).is_some()
    }
    /// A list with flags 2, expire `e`, the unit as owner, state 121,
    /// the delay remove callback; attached; state 121 on.
    ///
    /// TODO: the attach `reset` argument is not known; reset = 1 as the
    /// action wiring's state lists.
    fn create_delay_list(&mut self, u: UnitId, expire: i32) {
        let Some((ty, guid)) = self.cv.v.units.get(u).map(|r| (r.ty, r.guid)) else {
            return;
        };
        let v = &mut self.cv.v;
        let l = v
            .stats
            .alloc(DELAY_LIST_FLAGS, expire, ty.index() as u32, guid);
        v.stats.set_expire(l, expire);
        v.stats.set_state(l, u32::from(STATE_SKILL_DELAY));
        v.stats.set_remove_callback(l, Some(DELAY_REMOVE_CALLBACK));
        v.stats.attach(&mut *v.h, u, l, true);
        // The toggle, then the unit queued for update (always), so the
        // next update sends 0xA7 for state 121 (`gen-skill-sor-51` frame
        // 26).
        v.set_state(u, STATE_SKILL_DELAY, true);
        BodyWorld::queue_update(self, u);
    }
    fn set_state_list_expiry(&mut self, u: UnitId, state: u16, expire: i32) {
        if let Some(l) = self.cv.v.state_list(u, state) {
            self.cv.v.stats.set_expire(l, expire);
        }
    }
    /// The aura state's list freed and the state off.
    fn free_aura_state(&mut self, u: UnitId, state: u16) {
        let v = &mut self.cv.v;
        v.stats.free_state_list(&mut *v.h, u, u32::from(state));
        v.set_state(u, state, false);
        BodyWorld::queue_update(self, u);
    }
    /// The aura select's non-immediate branch: state on; the state's list
    /// (the existing one, else
    /// a new one: flags 0, expire 0, the unit as owner) gets the state,
    /// skill and level, is
    /// attached (reset 1) and holds only the markers
    /// 350 := skill, 351 := level (the first
    /// update's 0xA8 of the aura state shows them). Then the host's own hook.
    // An existing list is not attached a second time (the
    // load never makes such a list, so this never changes a result).
    fn set_aura_state(&mut self, u: UnitId, state: u16, skill: i32, lvl: i32) {
        if let Some((ty, guid)) = self.cv.v.units.get(u).map(|r| (r.ty, r.guid)) {
            let v = &mut self.cv.v;
            v.set_state(u, state, true);
            let l = match v.state_list(u, state) {
                Some(l) => l,
                None => {
                    let l = v.stats.alloc(0, 0, ty.index() as u32, guid);
                    v.stats.set_state(l, u32::from(state));
                    v.stats.set_skill(l, skill as u32, lvl as u32);
                    v.stats.attach(&mut *v.h, u, l, true);
                    l
                }
            };
            v.stats.set_state(l, u32::from(state));
            v.stats.set_skill(l, skill as u32, lvl as u32);
            v.set_list_stat(l, bodies::passive::PASSIVE_SKILL as u16, skill);
            v.set_list_stat(l, bodies::passive::PASSIVE_LEVEL as u16, lvl);
        }
        self.xm().set_aura_state(u, state, skill, lvl);
    }
}

// ---- BodyWorld: the skill bodies on the wired units ----------------------

impl<X: Pending + UseRest> UseView<'_, X> {
    fn body_tables(&self) -> Option<&bodies::BodyTables> {
        self.cv.v.h.bodies.as_deref()
    }
}

impl<X: Pending + UseRest> KickItems for UseView<'_, X> {
    fn toggle_weapon_lists(&mut self, u: UnitId, on: bool) {
        Pending::toggle_weapon_lists(self.xm(), u, on);
    }
    fn boots_damage(&self, item: UnitId) -> (i32, i32) {
        Pending::boots_damage(self.x(), item)
    }
}

impl<'a, X: Pending + UseRest> BodyWorld for UseView<'a, X> {
    type List = ListId;
    type Room = RoomId;
    type Combat = CombatView<'a, X>;

    fn combat(&mut self) -> &mut CombatView<'a, X> {
        &mut self.cv
    }

    fn stat_info(&self, s: i32) -> Option<bodies::BodyStat> {
        self.body_tables()?.stat(s)
    }
    fn state_count(&self) -> i32 {
        i32::try_from(self.cv.v.stats.data().states.count()).unwrap_or(i32::MAX)
    }
    fn state_flag(&self, s: i32, g: usize) -> bool {
        u32::try_from(s).is_ok_and(|s| self.cv.v.stats.data().states.has_flag(s, g))
    }
    fn state_group(&self, s: i32) -> i32 {
        let i = usize::try_from(s).ok();
        self.body_tables()
            .and_then(|b| b.state_group.get(i?).copied())
            .unwrap_or(0)
    }
    fn state_is_aura(&self, s: i32) -> bool {
        let i = usize::try_from(s).ok();
        self.body_tables()
            .and_then(|b| b.state_aura.get(i?).copied())
            .unwrap_or(false)
    }
    fn overlay_count(&self) -> i32 {
        self.body_tables().map_or(0, |b| b.overlay_count)
    }

    fn has_group(&self, u: UnitId, g: usize) -> bool {
        self.cv.v.stats.has_group(u, g)
    }
    /// The state toggle and the unit queued for update.
    fn state_on(&mut self, u: UnitId, s: i32, on: bool) {
        let Ok(s) = u16::try_from(s) else { return };
        self.cv.v.set_state(u, s, on);
        BodyWorld::queue_update(self, u);
    }
    /// The state-changed bit, then the unit queued for update: an aura
    /// run that only changes its list's values still sends 0xA8
    /// (`gen-skill-pal-102` frame 51).
    fn mark_state_changed(&mut self, u: UnitId, s: i32) {
        if let Ok(s) = u32::try_from(s) {
            if s < self.state_count() as u32 {
                self.cv.v.stats.set_state_changed(u, s, true);
                BodyWorld::queue_update(self, u);
            }
        }
    }
    fn clear_group_states(&mut self, u: UnitId, g: usize) {
        for s in 0..self.state_count() {
            let s32 = s as u32;
            if self.cv.v.stats.data().states.has_flag(s32, g) && self.cv.v.stats.has_state(u, s32) {
                self.cv.v.stats.set_state_changed(u, s32, true);
                self.cv.v.set_state(u, s as u16, false);
            }
        }
        BodyWorld::queue_update(self, u);
    }
    /// Queues the unit for update.
    fn queue_update(&mut self, u: UnitId) {
        if self.cv.game.lists.queue_update(u).is_err() {
            self.error(WiringError::Unit(
                crate::units::modes::UnitError::UnknownUnit(u),
            ));
        }
    }
    /// The stat host's stays-on-death test for state `s`.
    fn stays_on_death(&self, u: UnitId, s: i32) -> bool {
        use crate::stats::lists::StatHost;
        let v = &self.cv.v;
        v.h.stays_on_death(v.stats, u, s as u32)
    }

    fn state_list(&self, u: UnitId, s: i32) -> Option<ListId> {
        self.cv.v.state_list(u, u16::try_from(s).ok()?)
    }
    /// The unit's first list with the given flags.
    fn first_list_with_flags(&self, u: UnitId, flags: u32) -> Option<ListId> {
        let st = &self.cv.v.stats;
        st.list_by_flags(st.unit_list(u)?, flags)
    }
    fn alloc_list(&mut self, flags: u32, expire: i32, owner: Option<UnitId>) -> Option<ListId> {
        let (ty, guid) = match owner {
            Some(o) => {
                let r = self.cv.v.units.get(o)?;
                (r.ty.index() as u32, r.guid)
            }
            None => (6, u32::MAX),
        };
        Some(self.cv.v.stats.alloc(flags, expire, ty, guid))
    }
    fn list_state(&self, l: ListId) -> i32 {
        self.cv.v.stats.state(l) as i32
    }
    fn set_list_state(&mut self, l: ListId, s: i32) {
        self.cv.v.stats.set_state(l, s as u32);
    }
    fn list_skill(&self, l: ListId) -> (i32, i32) {
        let (s, v) = self.cv.v.stats.skill(l);
        (s as i32, v as i32)
    }
    fn set_list_skill(&mut self, l: ListId, skill: i32, lvl: i32) {
        self.cv.v.stats.set_skill(l, skill as u32, lvl as u32);
    }
    fn list_expire(&self, l: ListId) -> i32 {
        self.cv.v.stats.expire(l)
    }
    fn set_list_expire(&mut self, l: ListId, e: i32) {
        self.cv.v.stats.set_expire(l, e);
    }
    fn list_get(&self, l: ListId, s: i32) -> i32 {
        u16::try_from(s).map_or(0, |s| self.cv.v.stats.base(l, s, 0))
    }
    fn list_set(&mut self, l: ListId, s: i32, v: i32) {
        if let Ok(s) = u16::try_from(s) {
            self.cv.v.set_list_stat(l, s, v);
        }
    }
    fn list_set_layer(&mut self, l: ListId, s: i32, v: i32, layer: u16) {
        if let Ok(s) = u16::try_from(s) {
            self.cv.v.stats.set(&mut *self.cv.v.h, l, s, v, layer, None);
        }
    }
    fn attach(&mut self, u: UnitId, l: ListId) {
        let v = &mut self.cv.v;
        v.stats.attach(&mut *v.h, u, l, true);
        self.drain_skill_refresh();
    }
    fn set_remove_callback(&mut self, l: ListId, cb: u32) {
        self.cv
            .v
            .stats
            .set_remove_callback(l, Some(RemoveCallback(cb)));
    }
    /// Detach, the list's remove callback as the bodies give it
    /// ([`bodies::remove_callback`]; the stat host runs none), then free.
    fn detach_free(&mut self, u: UnitId, l: ListId) {
        let st = self.cv.v.stats.state(l) as i32;
        let cb = self.cv.v.stats.remove_callback(l);
        {
            let v = &mut self.cv.v;
            let n = v.h.removed_lists.len();
            v.stats.detach(&mut *v.h, l);
            // The alignment callback the detach queued for the expiry
            // runner runs below, once.
            let mut i = 0;
            v.h.removed_lists.retain(|e| {
                i += 1;
                i <= n || !ALIGNMENT_CALLBACKS.contains(&e.2)
            });
        }
        if let Some(cb) = cb {
            let t = self.cv.v.h.tables.clone();
            bodies::remove_callback(self, &t.skills, &t.combat, u, st, cb.0, l);
        }
        let v = &mut self.cv.v;
        v.stats.free_plain(&mut *v.h, l);
        self.drain_skill_refresh();
    }

    fn add_handler(&mut self, u: UnitId, h: bodies::Handler) {
        self.cv.v.h.handlers.entry(u).or_default().insert(0, h);
    }
    /// TODO: the deferred free of a running record (flags bit 0 → flags
    /// |= 2) needs the handler iteration, which has no provider; records
    /// are unlinked at once.
    fn remove_handlers(&mut self, u: UnitId, key_type: i32, key: i32) {
        if let Some(v) = self.cv.v.h.handlers.get_mut(&u) {
            v.retain(|h| !(h.key_type == key_type && h.key == key));
        }
    }

    /// The adjacency array of the source's room, or of the room
    /// containing `at`; each room's town test and unit list.
    fn scan_rooms(
        &self,
        source: UnitId,
        at: Option<(i32, i32)>,
    ) -> Option<Vec<bodies::ScanRoom<UnitId>>> {
        let game = &*self.cv.game;
        let drlg = &self.cv.v.h.drlg;
        let mut room = game.lists.unit(source)?.room()?;
        if let Some((x, y)) = at {
            room = drlg.find_room(game, room, x, y)?;
        }
        let adjacent = game.lists.room(room)?.adjacent.clone();
        Some(
            adjacent
                .into_iter()
                .map(|r| bodies::ScanRoom {
                    town: drlg.in_town(game, r),
                    units: game.lists.room_units(r),
                })
                .collect(),
        )
    }
    /// Ally test: a monster stands for its minion owner (the AI control's
    /// owner link looked up by GUID; none: the monster itself); the same
    /// unit after that → allies (an Oak Sage's aura reaches its druid).
    /// Two different players (the party test) and the rest: the seam's.
    fn allied(&self, a: UnitId, b: UnitId) -> bool {
        let owner = |u: UnitId| {
            let link = self
                .cv
                .v
                .h
                .ai
                .as_ref()
                .and_then(|s| s.control(u))
                .and_then(|c| c.minion_owner);
            link.and_then(|r| self.cv.game.lists.find_unit(r.ty, r.guid))
                .unwrap_or(u)
        };
        let (ra, rb) = (owner(a), owner(b));
        if ra == rb {
            return true;
        }
        let both_players =
            self.unit_type(ra) == UnitType::Player && self.unit_type(rb) == UnitType::Player;
        if both_players {
            return self.x().allied(ra, rb);
        }
        // Anything else: the alignment pairing (both evil or both good).
        let (aa, ab) = (
            CombatWorld::alignment(&self.cv, ra),
            CombatWorld::alignment(&self.cv, rb),
        );
        (aa == 0 && ab == 0) || (aa == 2 && ab == 2)
    }
    /// The missile store's owner.
    fn missile_owner(&self, u: UnitId) -> Option<UnitId> {
        let Some(m) = self.cv.v.h.missiles.as_ref().and_then(|s| s.get(u)) else {
            return self.cv.v.h.unit_source.get(&u).copied();
        };
        let o = m.owner?;
        self.cv.game.lists.find_unit(o.ty, o.guid)
    }
    fn minion_owner(&self, u: UnitId) -> Option<UnitId> {
        self.x().minion_owner(u)
    }
    /// The owner link of the monster's AI control: the unit, if it still
    /// exists.
    fn pack_owner(&self, u: UnitId) -> Option<UnitId> {
        let r = self.cv.v.h.ai.as_ref()?.control(u)?.minion_owner?;
        self.cv.game.lists.find_unit(r.ty, r.guid)
    }
    fn pet_unsummonable(&self, u: UnitId, pet: UnitId) -> bool {
        self.x().pet_unsummonable(u, pet)
    }

    fn frame_bonus(&self, u: UnitId) -> i32 {
        self.cv
            .v
            .h
            .frame_bonus_in(self.cv.v.units, self.cv.v.stats, u)
    }
    fn set_anim_frame(&mut self, u: UnitId, v: i32) {
        if let Some(r) = self.cv.v.units.get_mut(u) {
            r.anim.frame = v;
        }
    }
    fn set_entry_param(&mut self, u: UnitId, i: u8, v: i32) {
        self.xm().set_entry_param(u, i, v);
    }
    fn stat_max(&self, u: UnitId, s: u16) -> i32 {
        let st = &self.cv.v.stats;
        match s {
            6 => st.max_life(u),
            8 => st.max_mana(u),
            _ => st.max_stamina(u),
        }
    }

    fn composit_weapon_class(&self, u: UnitId) -> i32 {
        self.x().composit_weapon_class(u)
    }
    fn hand_class(&self, u: UnitId) -> i32 {
        self.x().hand_class(u)
    }
    fn item_shoots(&self, item: UnitId) -> bool {
        self.x().item_shoots(item)
    }
    fn item_stackable(&self, item: UnitId) -> bool {
        self.x().item_stackable(item)
    }
    fn item_stat_of(&self, item: UnitId, s: u16) -> i32 {
        self.cv.v.stats.unit_total(item, s, 0)
    }
    fn set_item_stat(&mut self, item: UnitId, s: u16, v: i32) {
        self.cv.v.set_base(item, s, v);
    }
    fn item_max_stack(&self, item: UnitId) -> i32 {
        self.x().item_max_stack(item)
    }
    /// [`Pending::item_max_durability`]; unknown → the current
    /// durability (nothing is changed).
    fn item_max_durability(&self, item: UnitId) -> i32 {
        self.x()
            .item_max_durability(item)
            .unwrap_or_else(|| self.cv.v.stats.unit_total(item, 72, 0))
    }
    fn quantity_timer(&mut self, item: UnitId) {
        let game = &mut *self.cv.game;
        self.cv.v.h.x.quantity_timer(game, item);
    }
    /// S→C 0x3E to the unit's client through the transport seam
    /// ([`Pending::send`]). Field widths: see
    /// `units::messages::update_item_stat`.
    fn send_item_stat(&mut self, u: UnitId, item: UnitId, s: u16, v: i32) {
        let guid = self.cv.v.units.get(item).map_or(0, |r| r.guid);
        let msg = crate::units::messages::update_item_stat(guid, s, v, 0);
        Pending::send(self.xm(), u, &msg);
    }
    fn attack_cleanup(&mut self, u: UnitId) {
        self.xm().attack_cleanup(u);
    }
    fn weapon_cleanup(&mut self, u: UnitId) {
        self.xm().weapon_cleanup(u);
    }

    /// Creates a missile on the real missile store.
    fn spawn_missile(&mut self, req: bodies::MissileRequest<UnitId>) -> Option<UnitId> {
        let p = MissileParams {
            flags: req.flags,
            owner: Some(req.owner),
            origin: req.origin,
            target: req.target,
            class: req.class,
            x: req.x,
            y: req.y,
            target_x: req.target_x,
            target_y: req.target_y,
            velocity: req.velocity,
            skill: req.skill,
            level: req.level,
            loops: req.loops,
            activate: req.activate,
            attack_bonus: req.attack_bonus,
            range: req.range,
            init: req.init,
            ..MissileParams::default()
        };
        let Some(mut store) = self.cv.v.h.missiles.take() else {
            self.error(WiringError::Reentrant("missiles"));
            return None;
        };
        let t = self.cv.v.h.tables.clone();
        let made = {
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut self.cv.v,
            };
            missiles::create_missile(self.cv.game, &mut cx, &p)
        };
        self.cv.v.h.missiles = Some(store);
        made
    }
    /// Every passive skill of the unit whose state is on
    /// ([`bodies::passive`]), then the host's own hook.
    fn passive_refresh(&mut self, u: UnitId) {
        let t = self.cv.v.h.tables.clone();
        for e in self.skill_list(u) {
            let p = t
                .skills
                .skill(e.skill)
                .map_or(-1, |r| i32::from(r.passivestate as i16));
            if p > 0 && SkillUnits::has_state(self, u, p as u16) {
                bodies::passive::refresh(self, &t.skills, u, e.skill);
            }
        }
        self.xm().passive_refresh(u);
    }
    /// The passive refresh, then for a player the skill resync (shout,
    /// battle orders and battle command re-send the passives' states,
    /// `bar-battle-command`).
    fn buff_refresh(&mut self, u: UnitId) {
        BodyWorld::passive_refresh(self, u);
        if self
            .cv
            .v
            .units
            .get(u)
            .is_some_and(|r| r.ty == UnitType::Player)
        {
            BodyWorld::skill_resync(self, u);
        }
        self.xm().buff_refresh(u);
    }
    fn skill_resync(&mut self, u: UnitId) {
        self.resync_pet_maxima(u);
        self.xm().skill_resync(u);
    }
    /// The passive state's stat list ([`bodies::passive`]), then the
    /// host's own hook.
    fn passive_state_apply(&mut self, u: UnitId, e: &SkillEntry) {
        let t = self.cv.v.h.tables.clone();
        bodies::passive::refresh(self, &t.skills, u, e.skill);
        self.xm().passive_state_apply(u, e);
    }
    /// Installs the AI state `k` through the AI module on the wiring's
    /// store, for a unit with an AI control: the summon's AI is installed
    /// again after its creation installed it, so its init runs twice
    /// (dru-raven's raven draws its orbit side twice). Without the store
    /// or a control: the seam's.
    fn set_ai_state(&mut self, u: UnitId, k: i32) {
        use crate::monsters::ai;
        let has_control = self
            .cv
            .v
            .h
            .ai
            .as_ref()
            .is_some_and(|s| s.control(u).is_some());
        if !has_control {
            self.xm().set_ai_state(u, k);
            return;
        }
        let Some(mut store) = self.cv.v.h.ai.take() else {
            return;
        };
        let t = self.cv.v.h.tables.clone();
        let info = self.cv.v.h.ai_info;
        {
            let mut cx = ai::Ctx {
                tables: ai::AiTables {
                    monstats: &t.combat.monstats,
                    monstats2: &t.combat.monstats2,
                    levels: &t.levels,
                    skill_modes: &t.skill_modes,
                    skills: &t.skills.skills,
                    missiles: &t.skills.missiles,
                },
                info,
                store: &mut store,
                world: &mut self.cv.v,
            };
            ai::install(&mut *self.cv.game, &mut cx, u, k as u32);
        }
        self.cv.v.h.ai = Some(store);
    }
    fn blood_mana(&mut self, u: UnitId, cost: i32) {
        self.xm().blood_mana(u, cost);
    }
    /// An 0xA3 record {n, k, lvl, unit, T, r, 0} on `u` (x = the roll,
    /// y = 0), the unit queued for update.
    fn queue_progressive(&mut self, u: UnitId, msg: bodies::ProgressiveMsg<UnitId>) {
        use crate::wiring::action::event_records::EventRecord;
        let units = &self.cv.v.units;
        let of = |id: UnitId| {
            units
                .get(id)
                .map_or((0, u32::MAX), |r| (r.ty.index() as u8, r.guid))
        };
        let r = EventRecord::Progressive {
            charges: msg.charges,
            skill: msg.skill,
            level: msg.level,
            unit: of(msg.unit),
            target: of(msg.target),
            x: msg.roll,
            y: 0,
        };
        self.cv.v.h.event_records.push(u, r);
        let _ = self.cv.game.lists.queue_update(u);
    }
    // ---- batch 2 and 3

    /// [`Pending::body_effect`]; the D2RUST fault
    /// [`bodies::BodyEffect::EndlessProgressive`] is a
    /// [`WiringError::EndlessProgressive`] instead.
    fn effect(&mut self, e: bodies::BodyEffect<UnitId, UnitId, RoomId>) {
        if let bodies::BodyEffect::ExhaustStack { owner, item } = e {
            if let Some(unit) = self.cv.game.lists.unit(item) {
                let mut mark = [EXHAUST_STACK_MARK; 5];
                mark[1..].copy_from_slice(&unit.guid.to_le_bytes());
                Pending::send(self.xm(), owner, &mark);
            }
            return;
        }
        if let bodies::BodyEffect::EndlessProgressive { unit, skill, step } = e {
            self.error(WiringError::EndlessProgressive { unit, skill, step });
            return;
        }
        if let bodies::BodyEffect::SourceFields { m, owner } = e {
            // The record carries the owner link with the source flag
            // 0x400, which the missile store's owner lookup reads.
            let link = owner.and_then(|o| {
                let ty = self.cv.v.units.get(o)?.ty;
                let guid = self.cv.game.lists.unit(o)?.guid;
                Some((ty.index() as u32, guid))
            });
            if let Some(r) = self.cv.v.units.get_mut(m) {
                match link {
                    Some(l) => {
                        r.source = l;
                        r.flags2 |= 0x400;
                    }
                    None => {
                        r.source = (0, 0);
                        r.flags2 &= !0x400;
                    }
                }
            }
            match owner {
                Some(o) => self.cv.v.h.unit_source.insert(m, o),
                None => self.cv.v.h.unit_source.remove(&m),
            };
            return;
        }
        // Umod assignment (the summon's `sumumod`, arg 1 = unique): on the
        // lent monster world.
        if let bodies::BodyEffect::Umod { m, umod, arg } = e {
            if let Ok(umod) = u8::try_from(umod) {
                let cv = &mut self.cv;
                let mut sim = crate::units::hooks::Sim {
                    game: &mut *cv.game,
                    units: &mut *cv.v.units,
                    stats: &mut *cv.v.stats,
                    data: cv.v.data,
                };
                cv.v.h.assign_umod_arg(&mut sim, m, umod, arg != 0);
            }
            return;
        }
        // Class change of the Overseer Whip's transform.
        if let bodies::BodyEffect::ClassChange { t, class, mode } = e {
            let cv = &mut self.cv;
            let mut sim = crate::units::hooks::Sim {
                game: &mut *cv.game,
                units: &mut *cv.v.units,
                stats: &mut *cv.v.stats,
                data: cv.v.data,
            };
            cv.v.h.reinit_class(&mut sim, t, class, mode.max(0) as u32);
            return;
        }
        // Target-node insert: the summon linked right after its owner's
        // head in list `slot` (newest first), node index := slot, so
        // monster AIs find it with its owner. `node_insert` checked m's
        // slot and type.
        // Target-list prepend: Attract, Confuse and Conversion put U at
        // the head of list 8 / 9; the stored list is the whole list, so
        // both are a push to the front.
        if let bodies::BodyEffect::NodeInsert { m: u, slot }
        | bodies::BodyEffect::NodePrepend { u, slot } = e
        {
            if let (Some(r), Ok(s)) = (self.cv.v.units.get_mut(u), u8::try_from(slot)) {
                r.node_index = s.into();
                self.cv.game.target_nodes.push_front(slot, u);
            }
            return;
        }
        // Target-list remove: U unlinked, node index := 11 (the
        // Conversion / Mind Blast remove callback).
        if let bodies::BodyEffect::NodeRemove(u) = e {
            self.cv.game.target_nodes.remove(u);
            if let Some(r) = self.cv.v.units.get_mut(u) {
                r.node_index = crate::units::record::INITIAL_NODE_INDEX;
            }
            return;
        }
        // The forced-target override (Attract kind 1 / 2, Confuse kind 3)
        // and its clear.
        if let bodies::BodyEffect::TargetOverride { m, kind, guid } = e {
            self.cv.v.set_target_override(m, kind as u32, guid as u32);
            return;
        }
        if let bodies::BodyEffect::ClearTargetOverride(m) = e {
            self.cv.v.clear_target_override(m);
            return;
        }
        // Component byte k of the monster data, the skeleton components:
        // on the lent monster world, before the summon's AssignMonster
        // goes out.
        if let bodies::BodyEffect::Component { m, k, v } = e {
            if let Some(d) = self.cv.v.h.monster_data_mut(m) {
                if let Some(c) = usize::try_from(k)
                    .ok()
                    .and_then(|k| d.components.get_mut(k))
                {
                    *c = v as u8;
                }
            }
            return;
        }
        // The room delete record of a consumed corpse (Raise Skeleton):
        // prepended to its room's delete list, whose S→C 0x0A the
        // per-client update sends.
        if let bodies::BodyEffect::RoomDelete(u) = e {
            let room = self.cv.game.lists.unit(u).and_then(|r| r.room());
            let rec = self.cv.v.units.get(u).map(|r| (r.ty as u8, r.guid));
            if let (Some(room), Some((ty, guid))) = (room, rec) {
                let game = &mut *self.cv.game;
                self.cv.v.room_delete(game, room, ty, guid);
            }
            return;
        }
        // Unit removal of a monster (a consumed corpse, a replaced pet):
        // off the room lists and freed. A missile's removal stays with the
        // seam: the missile world holds it.
        if let bodies::BodyEffect::RemoveUnit(u) = e {
            if self.cv.v.units.get(u).map(|r| r.ty) != Some(UnitType::Monster) {
                self.xm().body_effect(e);
                return;
            }
            let cv = &mut self.cv;
            let mut sim = crate::units::hooks::Sim {
                game: &mut *cv.game,
                units: &mut *cv.v.units,
                stats: &mut *cv.v.stats,
                data: cv.v.data,
            };
            let _ = crate::units::lifecycle::remove(&mut sim, &mut *cv.v.h, u);
            return;
        }
        // The AI command {type, x, y, tx, ty} inserted before the current
        // one, which it becomes (the Blade Sentinel's walk).
        if let bodies::BodyEffect::AiCommand {
            m,
            kind,
            x,
            y,
            tx,
            ty,
        } = e
        {
            if let Some(c) = self.cv.v.h.ai.as_mut().and_then(|s| s.control_mut(m)) {
                let at = c.cur.min(c.commands.len());
                c.commands.insert(
                    at,
                    crate::monsters::ai::AiCommand {
                        params: [kind, x, y, tx, ty],
                    },
                );
                c.cur = at;
                return;
            }
        }
        // Leash owner: the AI control's owner that the first precheck
        // reads; none clears it (GUID −1).
        if let bodies::BodyEffect::LeashOwner { m, owner } = e {
            let leash = owner.and_then(|o| self.cv.game.lists.unit(o)).map(|u| {
                crate::monsters::ai::UnitRef {
                    ty: u.ty,
                    guid: u.guid,
                }
            });
            if let Some(c) = self.cv.v.h.ai.as_mut().and_then(|s| s.control_mut(m)) {
                c.owner = leash;
                return;
            }
        }
        // Leave pack.
        if let bodies::BodyEffect::LeaveLeader(t) = e {
            let leader = self.pack_owner(t);
            let cv = &mut self.cv;
            if let (Some(l), Some(store)) = (leader, cv.v.h.ai.as_mut()) {
                leave_leader(store, cv.game, l, t);
            }
            return;
        }
        if let bodies::BodyEffect::DissolvePack(t) = e {
            let cv = &mut self.cv;
            if let Some(store) = cv.v.h.ai.as_mut() {
                dissolve_pack(store, cv.game, t);
            }
            return;
        }
        // "wait N": delete the thinks and schedule one at frame + N; the
        // mode is not changed. A fresh spawn has no uninterruptable state
        // to clear.
        if let bodies::BodyEffect::WaitThink { m, frames } = e {
            let game = &mut *self.cv.game;
            crate::monsters::ai::delete_thinks(game, m);
            let at = game
                .frame
                .wrapping_add(if frames == 0 { 1 } else { frames });
            let _ = game.schedule_event(
                m,
                u32::from(crate::monsters::ai::EVENT_THINK),
                at,
                None,
                0,
                0,
            );
            return;
        }
        // Find Item on the game's drop state
        // ([`super::super::action::ActionHooks::object_drops`]); a game
        // without it drops nothing. The quality value is ignored.
        if let bodies::BodyEffect::TreasureDrop { corpse, killer, .. } = e {
            let cv = &mut self.cv;
            if let Some(mut d) = cv.v.h.object_drops.take() {
                let mut sim = crate::units::hooks::Sim {
                    game: &mut *cv.game,
                    units: &mut *cv.v.units,
                    stats: &mut *cv.v.stats,
                    data: cv.v.data,
                };
                crate::wiring::economy::find_item_drop(
                    &mut *cv.v.h,
                    &mut sim,
                    &mut d,
                    &mut crate::wiring::economy::StartSpot,
                    corpse,
                    killer,
                );
                cv.v.h.object_drops = Some(d);
            }
            return;
        }
        // The item drop of Find Potion: the items index of the code (none
        // → nothing), the floor spot from `at`'s position, the item level
        // of `at`, request unit `at`, spawn mode 3, init flags 1. That is
        // the code drop ([`code_drop`]) with the same arguments; a game
        // without the drop state drops nothing.
        //
        // [`code_drop`]: crate::wiring::economy::drop_helpers::code_drop
        if let bodies::BodyEffect::DropItem { at, code, quality } = e {
            let cv = &mut self.cv;
            if let Some(mut d) = cv.v.h.object_drops.take() {
                let t = cv.v.h.tables.clone();
                let mut sim = crate::units::hooks::Sim {
                    game: &mut *cv.game,
                    units: &mut *cv.v.units,
                    stats: &mut *cv.v.stats,
                    data: cv.v.data,
                };
                crate::wiring::economy::drop_helpers::code_drop(
                    &mut *cv.v.h,
                    &mut sim,
                    &mut d,
                    &t.levels,
                    &mut crate::wiring::economy::StartSpot,
                    at,
                    u32::from_le_bytes(code),
                    quality as u8,
                );
                cv.v.h.object_drops = Some(d);
            }
            return;
        }
        // Summon equipment on the game's drop state (the item tables); a
        // game without it makes no equipment.
        if let bodies::BodyEffect::Equipment {
            owner,
            m,
            lvl,
            ilvl,
            ..
        } = e
        {
            let cv = &mut self.cv;
            if let Some(mut d) = cv.v.h.object_drops.take() {
                let mut sim = crate::units::hooks::Sim {
                    game: &mut *cv.game,
                    units: &mut *cv.v.units,
                    stats: &mut *cv.v.stats,
                    data: cv.v.data,
                };
                crate::wiring::economy::summon_equipment(
                    &mut *cv.v.h,
                    &mut sim,
                    &mut d,
                    owner,
                    m,
                    lvl,
                    ilvl,
                );
                cv.v.h.object_drops = Some(d);
            }
            return;
        }
        // Missile data words 0x28 / 0x2C on the missile store: the seed
        // word Volcano's server-do 28 re-seeds from.
        if let bodies::BodyEffect::MissileData28 { missile, v }
        | bodies::BodyEffect::MissileData2C { missile, v } = e
        {
            let d28 = matches!(e, bodies::BodyEffect::MissileData28 { .. });
            match self.cv.v.h.missiles.as_mut() {
                Some(store) => {
                    if let Some(d) = store.get_mut(missile) {
                        if d28 {
                            d.target.0 = v;
                        } else {
                            d.target.1 = v;
                        }
                    }
                }
                None => self.error(WiringError::Reentrant("missiles")),
            }
            return;
        }
        // Alignment: the Hydras of a monster owner take the owner's
        // alignment, so the Council's stay hostile to the player instead
        // of the class row's good.
        if let bodies::BodyEffect::Alignment { u, a, .. } = e {
            if let Ok(a) = u8::try_from(a) {
                self.cv.v.set_alignment(&mut *self.cv.game, u, a);
            }
            return;
        }
        // Dead-body footprint: the egg's start gives up its monster
        // footprint.
        if let bodies::BodyEffect::DeadFootprint(u) = e {
            self.cv.v.dead_body_footprint(u);
            return;
        }
        // Kill: the egg hatch's last step.
        if let bodies::BodyEffect::KillBy { u, killer, .. } = e {
            crate::wiring::action::reaction::kill_by(&mut self.cv, u, killer);
            return;
        }
        // The 0xA3 record {v, skill, lvl, unit, T, x, y}, the unit queued.
        if let bodies::BodyEffect::MsgA3 {
            u,
            target,
            skill,
            lvl,
            x,
            y,
            v,
        } = e
        {
            use crate::wiring::action::event_records::EventRecord;
            let units = &self.cv.v.units;
            let of = |id: Option<UnitId>| {
                id.and_then(|id| units.get(id))
                    .map_or((0, u32::MAX), |r| (r.ty.index() as u8, r.guid))
            };
            let r = EventRecord::Progressive {
                charges: v as u8,
                skill: skill as u16,
                level: lvl as u16,
                unit: of(Some(u)),
                target: of(target),
                x: x as u32,
                y: y as u32,
            };
            self.cv.v.h.event_records.push(u, r);
            let _ = self.cv.game.lists.queue_update(u);
            return;
        }
        // 0x7F about the pet m, sent at once to the caster's client.
        if let bodies::BodyEffect::AllyInfo { u, m } = e {
            let life = bodies::b4_helpers::life_percent(self, m).clamp(0, 0xFFFF) as u16;
            let game = &*self.cv.game;
            let level = game
                .lists
                .unit(m)
                .and_then(|e| e.room())
                .and_then(|r| self.cv.v.h.drlg.level_id(game, r))
                .unwrap_or(0) as u16;
            if let Some(r) = self.cv.v.units.get(m) {
                let mut msg = [0u8; 10];
                msg[0] = 0x7F;
                msg[1] = u8::from(r.ty == UnitType::Player);
                msg[2..4].copy_from_slice(&life.to_le_bytes());
                msg[4..8].copy_from_slice(&r.guid.to_le_bytes());
                msg[8..10].copy_from_slice(&level.to_le_bytes());
                Pending::send(self.xm(), u, &msg);
            }
            return;
        }
        // The 0xA5 record on the unit, the unit queued for update.
        if let bodies::BodyEffect::MsgA5 { u, skill } = e {
            use crate::wiring::action::event_records::EventRecord;
            let r = EventRecord::Landing {
                skill: skill as u16,
            };
            self.cv.v.h.event_records.push(u, r);
            let _ = self.cv.game.lists.queue_update(u);
        }
        // Resurrect's split mode request: the builder clears the used
        // skill, then `resurrect_mode` sets it again, then the mode set
        // sets the mode (the record has no target:
        // [`BodyWorld::mode_request`] with none); its flag clears the used
        // skill when the unit is in the requested mode after the set.
        // Without this a revived corpse kept mode 12 (`fx-bone-spear-later`
        // Fallen 29, revived by shaman 28 at tick 392).
        if let bodies::BodyEffect::ModeRequestBuild { m, .. } = e {
            crate::skills::use_::UseWorld::set_used_skill(self, m, None);
            return;
        }
        if let bodies::BodyEffect::ModeRequestSend { m, mode, flag } = e {
            bodies::BodyWorld::mode_request(self, m, mode, None);
            if flag
                && u32::try_from(mode)
                    .is_ok_and(|mode| crate::skills::use_::UseWorld::mode(self, m) == mode)
            {
                crate::skills::use_::UseWorld::set_used_skill(self, m, None);
            }
            return;
        }
        // Swing: the player's mode request, unit form with the entry of
        // skill 0, mode 7, re-entry 1: the used skill, the mode start (the
        // path code's, which reaches the unit records) and then the unit
        // form's target.
        if let bodies::BodyEffect::UnitModeRequest {
            u,
            skill,
            mode,
            target,
        } = e
        {
            let Some(entry) = crate::skills::use_::UseWorld::find_entry(self, u, skill) else {
                return;
            };
            self.set_used_skill(u, Some(entry));
            let cv = &mut self.cv;
            let Some((ty, guid)) = cv.v.units.get(target).map(|r| (r.ty, r.guid)) else {
                return;
            };
            let wt = crate::path::walk::request::WalkTarget::Unit { ty, guid };
            if cv.v.h.paths.is_some() {
                crate::wiring::path::walk::player_request_reentry(
                    &mut cv.v,
                    &mut *cv.game,
                    u,
                    None,
                    mode as u32,
                    wt,
                );
            } else {
                cv.v.h
                    .x
                    .player_mode_request(&mut *cv.game, u, Some(skill as u16), mode as u32, wt);
            }
            // The re-entry request calls the start of the used skill
            // (Attack) after the mode's schedule, which writes frame
            // bonus · 256 into the unit record (it does not survive a
            // player's attack start).
            let tables = self.cv.v.h.tables.clone();
            crate::skills::use_::start(self, &tables.skills, u);
            let cv = &mut self.cv;
            let t = ModeTarget::Unit(target);
            crate::wiring::interaction::body_path::point_target(&mut cv.v, u, t);
            self.xm().keep_target(u, t);
            return;
        }
        // Alignment: the unit's alignment stat list, e.g. a monster's
        // Hydra takes its owner's alignment.
        if let bodies::BodyEffect::Alignment { u, a, .. } = e {
            if let Ok(a) = u8::try_from(a) {
                let cv = &mut self.cv;
                cv.v.set_alignment(&mut *cv.game, u, a);
            }
            return;
        }
        // The Iron Golem's take of its item. T leaves its room: the
        // delete record (the S→C 0x0A of the next per-client update), then
        // the room list. A ground item holds no collision on this wiring,
        // so there is no collision to free.
        if let bodies::BodyEffect::ItemLeaveRoom(k) = e {
            let room = self.cv.game.lists.unit(k).and_then(|r| r.room());
            let rec = self.cv.v.units.get(k).map(|r| (r.ty as u8, r.guid));
            if let (Some(room), Some((ty, guid))) = (room, rec) {
                let game = &mut *self.cv.game;
                self.cv.v.room_delete(game, room, ty, guid);
            }
            let _ = self.cv.game.lists.room_remove(k);
            return;
        }
        // T mode := 4, the mode the equip step asks for.
        if let bodies::BodyEffect::ItemMode { item, mode } = e {
            if let (Some(r), Ok(mode)) = (self.cv.v.units.get_mut(item), u32::try_from(mode)) {
                r.mode = mode;
            }
            return;
        }
        // Equip on the monster: the wiring's monster holdings, whose S→C
        // 0x9D action 6 follows m's 0xAC. T must be an item in mode 4; L 0
        // (no allowed location) puts it nowhere.
        if let bodies::BodyEffect::Equip { m, item, loc } = e {
            let ok = self
                .cv
                .v
                .units
                .get(item)
                .is_some_and(|r| r.ty == UnitType::Item && r.mode == 4);
            if let (true, Ok(loc @ 1..)) = (ok, u8::try_from(loc)) {
                let cv = &mut self.cv;
                let mut sim = crate::units::hooks::Sim {
                    game: &mut *cv.game,
                    units: &mut *cv.v.units,
                    stats: &mut *cv.v.stats,
                    data: cv.v.data,
                };
                crate::wiring::economy::monster_put_on(&mut *cv.v.h, &mut sim, m, item, loc);
            }
            return;
        }
        if let Some(e) = self.pet_effect(e) {
            self.xm().body_effect(e);
        }
    }
    fn path_op(&mut self, u: UnitId, op: bodies::PathOp<UnitId>) -> i32 {
        let cv = &mut self.cv;
        match body_path::path_op(&mut cv.v, &mut *cv.game, u, op) {
            Some(r) => {
                // The host also hears of a retarget, so the unit's kept
                // target follows its path target (Double Swing's second
                // swing).
                if matches!(op, bodies::PathOp::TargetUnit(_)) {
                    self.xm().body_path_op(u, op);
                }
                r
            }
            None => self.xm().body_path_op(u, op),
        }
    }
    fn monlvl(&self) -> &[d2_data::tables::Monlvl] {
        self.body_tables().map_or(&[], |b| &b.monlvl)
    }
    fn pettype_count(&self) -> i32 {
        self.body_tables().map_or(0, |b| b.pettype_count)
    }
    fn l_flag(&self) -> bool {
        self.x().l_flag()
    }
    /// The unit record's second flags word.
    fn unit_c8(&self, u: UnitId) -> u32 {
        self.cv.v.units.get(u).map_or(0, |r| r.flags2)
    }
    fn set_unit_c8(&mut self, u: UnitId, v: u32) {
        if let Some(r) = self.cv.v.units.get_mut(u) {
            r.flags2 = v;
        }
    }
    /// The unit record's animation frame.
    fn anim_frame(&self, u: UnitId) -> i32 {
        self.cv.v.units.get(u).map_or(0, |r| r.anim.frame)
    }
    fn frame_event_index(&self, u: UnitId) -> i32 {
        self.x().frame_event_index(u)
    }
    fn set_frame_event_index(&mut self, u: UnitId, i: i32) {
        // Stores into the unit's frame event index, which for a unit in a
        // sequence is the sequence position: the Mosquito bite rewinds it.
        if let Some(seq) = self
            .cv
            .v
            .units
            .get_mut(u)
            .and_then(|r| r.anim.sequence.as_mut())
        {
            seq.pos = (seq.pos & 0xFF).wrapping_add(i.wrapping_mul(0x100));
        }
        self.xm().set_frame_event_index(u, i);
    }
    /// The unit record's animation frame count.
    fn frame_count(&self, u: UnitId) -> i32 {
        self.cv.v.units.get(u).map_or(0, |r| r.anim.frame_count)
    }
    fn set_frame_count(&mut self, u: UnitId, v: i32) {
        if let Some(r) = self.cv.v.units.get_mut(u) {
            r.anim.frame_count = v;
        }
    }
    /// The unit record's target-node index.
    fn node_slot(&self, u: UnitId) -> i32 {
        self.cv
            .v
            .units
            .get(u)
            .map_or(crate::units::record::INITIAL_NODE_INDEX as i32, |r| {
                r.node_index as i32
            })
    }
    /// Whether the unit record has a stat holder.
    fn has_stat_holder(&self, u: UnitId) -> bool {
        self.cv.v.units.get(u).is_some_and(|r| r.stats.is_some())
    }
    /// [`Pending::size`].
    fn unit_size(&self, u: UnitId) -> i32 {
        Pending::size(self.x(), u)
    }
    /// The AI control's spawn class.
    fn minion_spawn_class(&self, u: UnitId) -> Option<i32> {
        self.cv
            .v
            .h
            .ai
            .as_ref()
            .and_then(|s| s.control(u))
            .map(|c| c.spawn_class)
            .or_else(|| self.cv.v.h.spawn_class_notes.get(&u).copied())
            .or_else(|| self.x().minion_spawn_class(u))
    }
    fn linked_unit(&self, u: UnitId) -> Option<UnitId> {
        self.x().linked_unit(u)
    }
    fn killer_of(&self, u: UnitId) -> Option<UnitId> {
        self.x().killer_of(u)
    }
    fn minion_owner_ident(&self, u: UnitId) -> Option<(u32, u32)> {
        self.x().minion_owner_ident(u)
    }
    /// Adds `v` to the base value.
    fn add_stat(&mut self, u: UnitId, s: u16, v: i32) {
        let b = SkillUnits::base_stat(&self.cv, u, s, 0);
        self.cv.v.set_base(u, s, b.wrapping_add(v));
    }
    fn list_add(&mut self, l: ListId, s: i32, v: i32) {
        if let Ok(s) = u16::try_from(s) {
            let v2 = &mut self.cv.v;
            v2.stats.add(&mut *v2.h, l, s, v, 0);
        }
    }
    fn list_clear(&mut self, l: ListId) {
        let v = &mut self.cv.v;
        v.stats.remove_all(&mut *v.h, l);
    }
    fn has_handler(&self, u: UnitId, key_type: i32, key: i32, skill: i32) -> bool {
        self.cv.v.h.handlers.get(&u).is_some_and(|v| {
            v.iter()
                .any(|h| h.key_type == key_type && h.key == key && h.skill == skill)
        })
    }
    fn entry_param(&self, u: UnitId, e: &SkillEntry, i: u8) -> i32 {
        match self.list_entry(u, e).zip(i.checked_sub(1)) {
            Some((l, k)) => l.params.get(usize::from(k)).copied().unwrap_or(0),
            None => Pending::entry_param(self.x(), u, e, i),
        }
    }
    fn set_entry_param_of(&mut self, u: UnitId, e: &SkillEntry, i: u8, v: i32) {
        match self.list_entry_mut(u, e) {
            Some(l) => {
                if let Some(p) = i
                    .checked_sub(1)
                    .and_then(|k| l.params.get_mut(usize::from(k)))
                {
                    *p = v;
                }
            }
            None => self.xm().set_entry_param_of(u, e, i, v),
        }
    }
    fn entry_flags(&self, u: UnitId, e: &SkillEntry) -> u32 {
        match self.list_entry(u, e) {
            Some(l) => l.flags,
            None => self.x().entry_flags(u, e),
        }
    }
    fn set_entry_flags(&mut self, u: UnitId, e: &SkillEntry, f: u32) {
        match self.list_entry_mut(u, e) {
            Some(l) => l.flags = f,
            None => self.xm().set_entry_flags(u, e, f),
        }
    }
    fn set_entry_mode(&mut self, u: UnitId, e: &SkillEntry, m: u32) {
        self.xm().set_entry_mode(u, e, m);
    }
    fn disguise_mode(&self, u: UnitId, m: u32) -> u32 {
        self.x().disguise_mode(u, m)
    }
    fn skill_sequence(&self, u: UnitId) -> Option<Vec<[u8; 6]>> {
        self.x().skill_sequence(u)
    }
    /// The animation variant that rewinds by `p` percent.
    fn anim_rewind(&mut self, u: UnitId, p: i32) {
        self.anim_variant(u, Form::Percent(p));
    }
    /// The animation variant that restarts with `v` frames.
    fn anim_restart(&mut self, u: UnitId, v: i32) {
        self.anim_variant(u, Form::Frames(v));
    }
    /// The animation variant that starts at frame `f`: Leap's and Leap
    /// Attack's frame-10 rewind, Whirlwind's frame 3.
    fn anim_from(&mut self, u: UnitId, f: i32) {
        self.anim_variant(u, Form::StartFrame(f));
    }
    /// The room the unit is in.
    fn unit_room(&self, u: UnitId) -> Option<RoomId> {
        self.cv.game.lists.unit(u)?.room()
    }
    /// The room containing (x, y), searched from `from`.
    fn room_at(&self, from: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.cv.v.h.drlg.find_room(&*self.cv.game, from, x, y)
    }
    /// Whether the room is in a town.
    fn room_in_town(&self, r: RoomId) -> bool {
        self.cv.v.h.drlg.in_town(&*self.cv.game, r)
    }
    fn room_act(&self, r: RoomId) -> i32 {
        self.x().room_act(r)
    }
    fn room_teleport(&self, r: RoomId) -> Option<i32> {
        self.level_teleport(r).or_else(|| self.x().room_teleport(r))
    }
    fn free_point(
        &mut self,
        r: RoomId,
        at: (i32, i32),
        size: i32,
        mask: u32,
        fallback: bool,
    ) -> Option<(RoomId, (i32, i32))> {
        match self.rooms_free_point(r, at, size, mask, fallback) {
            Some(found) => found,
            None => self.xm().free_point(r, at, size, mask, fallback),
        }
    }
    fn pattern_collides(&self, r: RoomId, at: (i32, i32), u: UnitId, mask: u32) -> bool {
        self.rooms_pattern_collides(r, at, u, mask)
            .unwrap_or_else(|| self.x().pattern_collides(r, at, u, mask))
    }
    fn box_collides(&self, r: RoomId, at: (i32, i32), size: i32, mask: u32) -> bool {
        self.rooms_box_collides(r, at, size, mask)
            .unwrap_or_else(|| self.x().box_collides(r, at, size, mask))
    }
    /// The line test ([`crate::path::line::line_test`]) on the DRLG rooms
    /// with the path provider; else [`Pending`].
    fn line_blocked(&self, r: RoomId, from: (i32, i32), to: (i32, i32), mask: u32) -> bool {
        if self.cv.v.h.paths.is_some() {
            use crate::path::Point;
            return crate::path::line::line_test(
                &self.cv.v.h.drlg,
                Some(r),
                Point::new(from.0, from.1),
                Point::new(to.0, to.1),
                mask as u16,
            )
            .blocked();
        }
        self.x().body_line_blocked(r, from, to, mask)
    }
    fn place_unit(&mut self, u: UnitId, r: Option<RoomId>, at: (i32, i32)) -> bool {
        match self.rooms_place_unit(u, r, at) {
            Some(placed) => placed,
            None => self.xm().place_unit(u, r, at),
        }
    }
    fn has_path(&self, u: UnitId) -> bool {
        self.cv.v.h.path_has(u)
    }
    /// The path record's point count, on the path provider.
    fn path_point_count(&self, u: UnitId) -> i32 {
        match self.dynamic_path(u) {
            Some(d) => d.point_count as i32,
            None => self.x().path_point_count(u),
        }
    }
    fn path_last_point(&self, u: UnitId) -> (i32, i32) {
        match self.dynamic_path(u) {
            Some(d) => d.live_points().last().map_or((0, 0), |p| (p.x, p.y)),
            None => self.x().path_last_point(u),
        }
    }
    fn path_target_point(&self, u: UnitId) -> (i32, i32) {
        self.cast_target_point(u)
    }
    fn create_monster(
        &mut self,
        r: RoomId,
        at: (i32, i32),
        class: i32,
        mode: i32,
        spread: i32,
    ) -> Option<UnitId> {
        if let Some(m) = self.xm().create_monster(r, at, class, mode, spread) {
            return Some(m);
        }
        // Placement search and creation through population on the monster
        // world (spread, flags 0x42); `Some(None)` = nothing placed (no
        // fallback). Without the world:
        // the plain allocation at the point (module docs of `summon`).
        let game = &mut *self.cv.game;
        let v = &mut self.cv.v;
        let mut sim = crate::units::hooks::Sim {
            game,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
        };
        let mode8 = u8::try_from(mode).unwrap_or(1);
        let placed = if v.h.monster_world.is_some() {
            v.h.with_monster_world(|w, h| {
                w.spawn_at(&mut sim, h, r, at.0, at.1, class, mode8, spread, 0x42)
            })
            .flatten()
        } else {
            None
        };
        match placed {
            Some(m) => m,
            None => self.alloc_monster(r, at, class, mode),
        }
    }
    fn mode_request(&mut self, m: UnitId, mode: i32, target: Option<UnitId>) -> i32 {
        let r = Pending::mode_request(self.xm(), m, mode, target);
        // D2RUST's own: without a host
        // answer the request is the monster mode set itself
        // (the AI's mode request), so a revived corpse
        // stands up.
        let monster = self
            .cv
            .v
            .units
            .get(m)
            .is_some_and(|r| r.ty == UnitType::Monster);
        match u32::try_from(mode) {
            Ok(mode) if r == 0 && monster => {
                if let Some(t) = target {
                    let t = crate::monsters::ai::ModeTarget::Unit(t);
                    self.cv.v.h.x.set_mode_target(m, t);
                }
                i32::from(self.cv.v.monster_set_mode(&mut *self.cv.game, m, mode))
            }
            _ => r,
        }
    }
    /// An item unit is its own item handle here.
    fn as_item(&self, u: UnitId) -> Option<UnitId> {
        (self.cv.v.units.get(u)?.ty == UnitType::Item).then_some(u)
    }
    fn inventory_busy(&self, u: UnitId) -> bool {
        self.x().inventory_busy(u)
    }
    fn has_inventory(&self, u: UnitId) -> bool {
        self.x().has_inventory(u)
    }
    fn weapon_in_use(&self, u: UnitId) -> Option<UnitId> {
        self.x().weapon_in_use(u)
    }
    fn body_loc(&self, i: UnitId) -> i32 {
        self.x().body_loc(i)
    }
    fn item_usable(&self, i: UnitId) -> bool {
        self.x().item_usable(i)
    }
    fn item_active(&self, i: UnitId) -> bool {
        self.x().item_active(i)
    }
    /// The same breakability test combat's wear reads
    /// ([`View::item_has_durability`]).
    fn item_breakable(&self, i: UnitId) -> bool {
        self.cv.v.item_has_durability(i)
    }
    fn shield(&self, u: UnitId) -> Option<UnitId> {
        Pending::shield(self.x(), u)
    }
    fn shield_damage(&self, i: UnitId) -> Option<(i32, i32)> {
        self.x().shield_damage(i)
    }
    fn item_missile_type(&self, i: UnitId) -> i32 {
        self.x().item_missile_type(i)
    }
    fn golem_item(&self, t: UnitId) -> bool {
        self.x().golem_item(t)
    }
    /// Itemtypes `bodyloc1` of the item's type, from the drop state's item
    /// tables; without them, the seam's answer.
    fn item_first_loc(&self, t: UnitId) -> i32 {
        let h = &self.cv.v.h;
        match (h.object_drops.as_ref(), h.items.get(t)) {
            (Some(d), Some(i)) => d
                .tables
                .items
                .itype_of(i.record)
                .map_or(0, |r| i32::from(r.bodyloc1)),
            _ => self.x().item_first_loc(t),
        }
    }
    fn two_melee_weapons(&self, u: UnitId) -> bool {
        self.x().two_melee_weapons(u)
    }
    fn attack_frames(&self, u: UnitId, w: UnitId) -> Option<i32> {
        self.x().attack_frames(u, w)
    }
    // ---- batch 4

    fn dir64(&self, u: UnitId, at: (i32, i32)) -> i32 {
        // Through the path provider; the host's answer only without one
        // (MagottLay egg side, `gen-mon-72`).
        self.cv
            .v
            .path_dir64(u, at)
            .unwrap_or_else(|| self.x().body_dir64(u, at))
    }
    /// The animation's action frame.
    fn action_frame(&self, u: UnitId) -> i32 {
        self.cv
            .v
            .units
            .get(u)
            .map_or(0, |r| i32::from(r.anim.action_frame))
    }
    fn set_action_frame(&mut self, u: UnitId, v: i32) {
        if let Some(r) = self.cv.v.units.get_mut(u) {
            r.anim.action_frame = v as u8;
        }
    }
    /// The sequence's speed (no sequence: nothing kept).
    fn set_seq_speed(&mut self, u: UnitId, v: i32) {
        if let Some(q) = self
            .cv
            .v
            .units
            .get_mut(u)
            .and_then(|r| r.anim.sequence.as_mut())
        {
            q.speed = v;
        }
    }
    /// The animation speed (i16).
    fn anim_speed(&self, u: UnitId) -> i32 {
        self.cv
            .v
            .units
            .get(u)
            .map_or(0, |r| i32::from(r.anim.speed))
    }
    fn set_anim_speed(&mut self, u: UnitId, v: i32) {
        if let Some(r) = self.cv.v.units.get_mut(u) {
            r.anim.speed = v as i16;
        }
    }
    /// The missile's total frames from the real missile store (0 without
    /// missile data); the host's hook only while the store is lent out.
    /// The bodies count a missile's lifetime in base ticks: the store's
    /// tick count is converted at the game's rate.
    fn missile_frames(&self, m: UnitId) -> i32 {
        match self.cv.v.h.missiles.as_ref() {
            Some(s) => {
                let total = s.get(m).map_or(0, |d| i32::from(d.total));
                self.cv.game.tick_rate.base_ticks(total)
            }
            None => self.x().body_missile_frames(m),
        }
    }
    /// Total frames and frames left of a missile in the store
    /// (the missile's range in frames), given in base ticks and stored
    /// as ticks at the game's rate.
    fn set_missile_frames(&mut self, m: UnitId, total: i32, left: i32) {
        let rate = self.cv.game.tick_rate;
        if let Some(d) = self.cv.v.h.missiles.as_mut().and_then(|s| s.get_mut(m)) {
            d.total = missiles::clamp_frame(rate.ticks(total));
            d.current = missiles::clamp_frame(rate.ticks(left));
        }
        self.xm().body_set_missile_frames(m, total, left);
    }
    /// The sequence's frame count.
    fn sequence_frames(&self, u: UnitId) -> Option<i32> {
        let r = self.cv.v.units.get(u)?;
        r.anim.sequence.as_ref().map(|q| q.frame_count)
    }
    /// The sequence's event byte of frame `f >> 8`.
    fn sequence_event(&self, u: UnitId, f: i32) -> i32 {
        let Some(q) = self
            .cv
            .v
            .units
            .get(u)
            .and_then(|r| r.anim.sequence.as_ref())
        else {
            return 0;
        };
        usize::try_from(f >> 8)
            .ok()
            .and_then(|i| q.events.get(i))
            .map_or(0, |&e| i32::from(e))
    }
    /// The AnimData record of the unit's animation.
    fn anim_data(&self, u: UnitId) -> Option<(u32, Vec<u8>)> {
        let a = self.cv.v.units.get(u)?.anim.record.as_ref()?;
        Some((a.frames, a.events.to_vec()))
    }
    fn action_event_between(&self, u: UnitId, a: i32, b: i32) -> bool {
        self.x().body_action_event_between(u, a, b)
    }
    /// [`Pending::ai_chain_index`].
    fn chain_position(&self, class: i32) -> i32 {
        self.cv.v.h.monster_chain_position(class)
    }
    /// [`Pending::ai_class_for_level`].
    fn class_for_level(&self, room: Option<RoomId>, class: i32) -> i32 {
        self.x().ai_class_for_level(self.cv.game, room, class)
    }
    fn book_skills(&self, i: UnitId) -> Option<(i32, i32)> {
        self.x().body_book_skills(i)
    }
    fn inventory_nodes(&self, u: UnitId) -> Vec<(UnitId, i32)> {
        self.x().body_inventory_nodes(u)
    }
    fn unit_find(&self, room: RoomId, at: (i32, i32), r: i32, f: u32) -> Vec<UnitId> {
        self.x().body_unit_find(room, at, r, f)
    }
    fn point_collides(&self, room: RoomId, at: (i32, i32), mask: u32) -> bool {
        self.rooms_point_collides(room, at, mask)
            .unwrap_or_else(|| self.x().body_point_collides(room, at, mask))
    }
    fn spawn_monster(&mut self, q: bodies::MonsterSpawn<UnitId, RoomId>) -> Option<UnitId> {
        // Creation through the lent monster world (`MonsterWorld::
        // spawn_at`): the placement and the allocation draw on the game
        // seed as the game's creation does. Without a lent world, the
        // host's answer.
        if let bodies::MonsterSpawn::At {
            room,
            x,
            y,
            class,
            mode,
            spread,
            flags,
        } = q
        {
            let game = &mut *self.cv.game;
            let v = &mut self.cv.v;
            let mut sim = crate::units::hooks::Sim {
                game,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            };
            let placed =
                v.h.with_monster_world(|w, h| {
                    w.spawn_at(
                        &mut sim,
                        h,
                        room,
                        x,
                        y,
                        class,
                        u8::try_from(mode).unwrap_or(0),
                        spread,
                        flags as u16,
                    )
                })
                .flatten();
            if let Some(placed) = placed {
                return placed;
            }
        }
        if let bodies::MonsterSpawn::Near {
            unit,
            class,
            mode,
            spread,
            flags,
        } = q
        {
            let game = &mut *self.cv.game;
            let v = &mut self.cv.v;
            let mut sim = crate::units::hooks::Sim {
                game,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
            };
            let placed =
                v.h.with_monster_world(|w, h| {
                    w.spawn_near(
                        &mut sim,
                        h,
                        unit,
                        class,
                        u8::try_from(mode).unwrap_or(0),
                        spread,
                        flags as u16,
                    )
                })
                .flatten();
            if let Some(placed) = placed {
                return placed;
            }
        }
        self.xm().body_spawn_monster(q)
    }
    /// a = 0: [`BodyWorld::place_unit`]'s provider.
    fn place_unit_flag(&mut self, u: UnitId, r: Option<RoomId>, at: (i32, i32), a: i32) -> bool {
        if a == 0 {
            BodyWorld::place_unit(self, u, r, at)
        } else {
            match self.rooms_place_unit_exact(u, r, at, true) {
                Some(placed) => placed,
                None => self.xm().body_place_unit_flag(u, r, at, a),
            }
        }
    }
    /// Monster data component `k` from the lent monster world;
    /// [`Pending::ai_component`] without one.
    fn component(&self, u: UnitId, k: usize) -> i32 {
        match self.cv.v.h.monster_data(u) {
            Some(d) => i32::from(d.components.get(k).copied().unwrap_or(0)),
            None => i32::from(self.x().ai_component(u, k)),
        }
    }
}

// ---- unit events ---------------------------------------------------------

/// The event functions' calls beyond [`BodyWorld`]: the handler lists of
/// [`crate::wiring::action::ActionHooks::handlers`], GUIDs and list owners
/// are real; the rest are [`Pending`]'s event seams.
impl<X: Pending + UseRest> EventWorld for UseView<'_, X> {
    /// The layer split from the body tables' itemstatcost; else the
    /// seam's.
    fn layer_split(&self) -> (u32, u32) {
        self.cv
            .v
            .h
            .bodies
            .as_ref()
            .and_then(|b| b.layer_split)
            .unwrap_or_else(|| self.x().event_layer_split())
    }
    /// The list's owner type / GUID resolved in the game's unit hash.
    fn list_owner(&self, l: ListId) -> Option<UnitId> {
        let stats = &self.cv.v.stats;
        let ty = *UnitType::ALL.get(stats.owner_type(l) as usize)?;
        self.cv.game.lists.find_unit(ty, stats.owner_guid(l))
    }
    fn guid(&self, u: UnitId) -> u32 {
        self.cv.v.units.get(u).map_or(0, |r| r.guid)
    }
    fn terror(&mut self, source: UnitId, unit: UnitId, skill: i32, a: i32, b: i32) {
        let game = &mut *self.cv.game;
        self.cv.v.h.x.event_terror(game, source, unit, skill, a, b);
    }
    fn point_free(&self, u: UnitId, at: (i32, i32)) -> bool {
        self.x().event_point_free(u, at)
    }
    fn corpse_near(&mut self, t0: UnitId) -> Option<UnitId> {
        let game = &mut *self.cv.game;
        self.cv.v.h.x.event_corpse_near(game, t0)
    }
    fn queue_item_cast(&mut self, u: UnitId, msg: ItemCastMsg) {
        // The 0x99 / 0x9A record on the unit, the unit queued for update.
        // The record's level byte is the low byte of the cast level, w
        // (u16) the `aim` byte widened.
        use crate::wiring::action::event_records::EventRecord;
        let r = match msg {
            ItemCastMsg::Unit {
                skill,
                level,
                target,
                aim,
            } => EventRecord::CastOnUnit {
                skill: skill as u16,
                level: level as u8,
                target: (target.0 as u8, target.1),
                w: u16::from(aim),
            },
            ItemCastMsg::Point {
                skill,
                level,
                at,
                aim,
            } => EventRecord::CastOnPoint {
                skill: skill as u32,
                level: level as u8,
                x: at.0 as u16,
                y: at.1 as u16,
                w: u16::from(aim),
            },
        };
        self.cv.v.h.event_records.push(u, r);
        let _ = self.cv.game.lists.queue_update(u);
        self.xm().queue_item_cast(u, msg);
    }
    fn raise_test(&self, v: UnitId) -> bool {
        self.x().raise_test(v)
    }
    fn clear_pattern(&mut self, v: UnitId) {
        self.xm().clear_pattern(v);
    }
    fn raise_step(&mut self, n: UnitId, step: RaiseStep<UnitId>) {
        let game = &mut *self.cv.game;
        self.cv.v.h.x.raise_step(game, n, step);
    }
    fn handlers_of(&self, u: UnitId) -> Vec<bodies::Handler> {
        self.cv.v.h.handlers.get(&u).cloned().unwrap_or_default()
    }
    /// The path record alone, without the mode-start fallback of
    /// `cast_target_point`: the game sends a level-up Nova (Rainbow Facet)
    /// at (0, 0) after an attack on a unit (netp-item-cast-levelup, 0x9A
    /// x / y 0).
    fn path_point(&self, u: UnitId) -> (i32, i32) {
        self.cv
            .v
            .h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(u))
            .map(|d| (i32::from(d.target_x), i32::from(d.target_y)))
            .unwrap_or_else(|| self.x().path_target_point(u))
    }
    fn remove_handler(&mut self, u: UnitId, h: &bodies::Handler) {
        if let Some(v) = self.cv.v.h.handlers.get_mut(&u) {
            if let Some(i) = v.iter().position(|x| x == h) {
                v.remove(i);
            }
        }
    }
    /// The game's item provider (`docs/plans/mods/seams.md` MS7b) when it
    /// has function `func`, on this view as its [`crate::rules::SkillHost`].
    fn mod_event(
        &mut self,
        func: i32,
        c: events::EventCall<UnitId>,
        r: Option<&mut crate::combat::DamageRecord>,
    ) -> Option<i32> {
        if !self
            .cv
            .v
            .h
            .rules
            .items()
            .is_some_and(|p| p.has_event_fn(func))
        {
            return None;
        }
        let rules = self.cv.v.h.rules.clone();
        rules.items()?.event_fn(self, func, &c, r)
    }
}

/// The unit event iteration on the action wiring: [`events::run`] over
/// [`UseView`] (a [`crate::wiring::action::combat::UnitEventFn`]).
pub fn run_unit_event<X: Pending + UseRest>(
    cv: &mut CombatView<'_, X>,
    event: u8,
    unit: Option<UnitId>,
    other: Option<UnitId>,
    record: Option<&mut crate::combat::DamageRecord>,
) -> i32 {
    let t = cv.v.h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: &mut *cv.game,
            v: View::of(&mut *cv.v.units, &mut *cv.v.stats, cv.v.data, &mut *cv.v.h),
        },
    };
    let tb = EventTables {
        skills: &t.skills,
        combat: &t.combat,
    };
    events::run(&mut w, tb, event, unit, other, record)
}

/// The curse helper (`apply_state`) on the action wiring, as
/// [`crate::combat::CombatWorld::curse`] calls it (prevent heal): `stat`
/// and `value` give the list's one stat, the default remove callback (a
/// [`crate::wiring::action::combat::ApplyStateFn`]).
#[allow(clippy::too_many_arguments)]
pub fn run_apply_state<X: Pending + UseRest>(
    cv: &mut CombatView<'_, X>,
    target: UnitId,
    owner: UnitId,
    state: u16,
    stat: u16,
    value: i32,
    frames: i32,
    skill: i32,
    level: i32,
) {
    let t = cv.v.h.tables.clone();
    let mut w = UseView {
        cv: CombatView {
            game: &mut *cv.game,
            v: View::of(&mut *cv.v.units, &mut *cv.v.stats, cv.v.data, &mut *cv.v.h),
        },
    };
    let q = bodies::StateRequest {
        source: owner,
        target,
        skill,
        level,
        duration: frames,
        stat: i32::from(stat),
        value,
        state: i32::from(state),
        callback: 0,
    };
    let _ = bodies::apply_state(&mut w, &t.combat, q);
}

impl<X: Pending + UseRest> crate::wiring::action::ActionHooks<X> {
    /// Turns the unit event registry on: from now on the unit event
    /// iteration runs [`events::run`] on [`Self::handlers`]
    /// ([`run_unit_event`]) instead of [`Pending::unit_event`] /
    /// [`Pending::level_up_event`], and the combat curse runs
    /// `apply_state` ([`run_apply_state`]) instead of [`Pending::curse`].
    pub fn enable_unit_events(&mut self) {
        self.unit_events = Some(run_unit_event::<X>);
        self.apply_state = Some(run_apply_state::<X>);
    }
}

/// Leader `l` drops minion `t`: t's GUID is looked up in the leader's
/// minion list; found, t's owner is cleared and the entry removed.
/// Absent: nothing.
fn leave_leader(store: &mut crate::monsters::ai::AiStore, game: &Game, l: UnitId, t: UnitId) {
    let Some(g) = game.lists.unit(t).map(|e| e.guid) else {
        return;
    };
    let Some(lc) = store.control_mut(l) else {
        return;
    };
    // Minions are appended at the end: the leader's search
    // from the newest minion is a search from the end.
    let Some(i) = lc.minions.iter().rposition(|&m| m == g) else {
        return;
    };
    lc.minions.remove(i);
    if let Some(tc) = store.control_mut(t) {
        tc.minion_owner = None;
    }
}

/// `t` leads its pack; every minion's owner is cleared and the list
/// emptied. t keeps its own owner.
fn dissolve_pack(store: &mut crate::monsters::ai::AiStore, game: &Game, t: UnitId) {
    let Some(tc) = store.control_mut(t) else {
        return;
    };
    let list = std::mem::take(&mut tc.minions);
    for g in list.into_iter().rev() {
        let m = game.lists.find_unit(UnitType::Monster, g);
        if let Some(mc) = m.and_then(|m| store.control_mut(m)) {
            mc.minion_owner = None;
        }
    }
}
