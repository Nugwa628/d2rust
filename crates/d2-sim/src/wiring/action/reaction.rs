//! The reaction to a hit and the kill that follows a fatal hit.
//!
//! An uninterruptible defender (state 54) only gets `death_delay` (92) when
//! the hit will kill it; a monster defender the hit will kill (result 2) is
//! killed: the kill's guards, its death mode change toward the attacker (the
//! death start through [`Pending::monster_death_start`]) and the experience
//! distribution ([`distribute`]).
//!
//! Everything checked at call level only stays a seam: [`Pending::reaction`]
//! (town rule, hit class store, the player branch, the monster knockback /
//! block / get-hit / soft hit mode changes) and [`Pending::kill_step`] (pet
//! credit, attacker bookkeeping, facing, quest kill, barricade doors).

use crate::combat::result;
use crate::combat::vitals::experience::{distribute, ExpShare};
use crate::combat::vitals::VitalsUnits;
use crate::combat::DamageRecord;
use crate::stats::states::state;
use crate::units::modes::{monster_mode, player_mode};
use crate::units::{UnitId, UnitType};

use crate::combat::damage::element_hit_class;
use crate::combat::CombatWorld;
use crate::monsters::ai::{AiModes, ModeTarget};
use crate::path::coords::Point;
use crate::path::walk::request::WalkTarget;
use crate::units::hooks::Sim;

use super::arena::ArenaRow;
use super::combat::CombatView;
use super::monsters::umod_mode;
use super::units::STATE_DEATH_DELAY;
use super::{KillStep, Pending, View};

/// Unit event 12 `levelup`.
pub const EV_LEVELUP: u8 = 12;

/// Host-only mark `[MARK, player GUID LE u32]` of a level-up (after the S->C
/// 0x75 broadcast): the host runs the item relink and the inventory pass on
/// the inventory model at this place of the tick.
pub const LEVEL_UP_ITEMS_MARK: u8 = 0xF7;

/// Result flag 2, "will die".
pub const RESULT_WILL_DIE: u32 = 2;

/// The reaction of `d` to `a`'s hit: [`Pending::reaction`] (the town rule),
/// then the hit class store, the uninterruptible check (state 54), the
/// monster branch and the player branch.
///
/// Open point: the town rule's early return is the host's
/// ([`Pending::reaction`]); the later steps run after it regardless.
pub fn reaction<X: Pending>(
    cv: &mut CombatView<'_, X>,
    a: UnitId,
    d: UnitId,
    rec: &mut DamageRecord,
) {
    cv.v.h.x.reaction(a, d, rec);
    // Hit class store: an unfixed class without an element nibble takes
    // the element hit class (this steps the process-wide counter); then the
    // defender's hit class is set from the record.
    if rec.result & result::HIT != 0 {
        if rec.hit_class_fixed == 0 && rec.hit_class & 0xF0 == 0 {
            let base = rec.hit_class;
            rec.hit_class = element_hit_class(cv.hit_class_counter(), rec, base);
        }
        if let Some(r) = cv.v.units.get_mut(d) {
            r.hit_class = rec.hit_class;
        }
    }
    let will_die = u32::from(rec.result) & RESULT_WILL_DIE != 0;
    // An uninterruptible defender only gets death delay when the hit kills.
    if cv.v.stats.has_state(d, state::UNINTERRUPTABLE) {
        if will_die {
            cv.v.set_state(d, STATE_DEATH_DELAY, true);
        }
        return;
    }
    match cv.game.lists.unit(d).map(|e| e.ty) {
        Some(UnitType::Monster) => monster_hit(cv, a, d, rec),
        Some(UnitType::Player) => player_hit(cv, a, d, rec),
        _ => {}
    }
}

/// Unit flag 0x8000, the "soft" hit: the client is told with S->C 0x0C.
pub const UNIT_FLAG_SOFT_HIT: u32 = 0x8000;

/// A "soft" hit: queue the unit for update and set unit flag 0x8000.
fn soft<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId) {
    if cv.game.lists.queue_update(d).is_err() {
        return;
    }
    if let Some(r) = cv.v.units.get_mut(d) {
        r.flags |= UNIT_FLAG_SOFT_HIT;
    }
}

/// Whether the unit enters get-hit: the no-get-hit test is false.
fn enters_get_hit<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId, rec: &DamageRecord) -> bool {
    let t = cv.v.h.tables.clone();
    !crate::combat::damage::no_get_hit(cv, &t.combat.hitclass, d, rec, rec.hit_class)
}

/// Monster classes that never play a block:
/// 243 `diablo`, 333 `diabloclone`, 705 `uberdiablo`.
const NO_BLOCK_CLASSES: [u32; 3] = [243, 333, 705];
/// monstats `BaseId` of `sandleaper1`.
const SANDLEAPER_BASE: u16 = 78;
/// AI state of a hit with no mode reaction.
const AI_STATE_HIT: u32 = 19;
/// Stat 352 `last_sent_hp_pct`.
const LAST_SENT_HP_PCT: u16 = 352;
/// State 1 `freeze`.
const STATE_FREEZE: u32 = 1;
/// Monster mode 6 `BL`, 13 `KB`.
const MONSTER_BL: u8 = 6;
const MONSTER_KB: u8 = 13;

/// Requests monster mode `mode` for `d`: the request record starts zeroed,
/// so without a target the request carries the point (0, 0).
fn monster_request<X: Pending>(
    cv: &mut CombatView<'_, X>,
    d: UnitId,
    mode: u8,
    target: Option<UnitId>,
) {
    let t = target.map_or(ModeTarget::Point(0, 0), ModeTarget::Unit);
    let game = &mut *cv.game;
    // The request clears the used skill first: a monster still in its spawn
    // sequence skill (mode 14) that is hit sends 0x69, not the skill message
    // 0x4D.
    AiModes::clear_current_skill(&mut cv.v, d);
    AiModes::change_mode(&mut cv.v, game, d, mode, t);
}

/// Runs the umod dispatcher in mode 4 (get-hit) on the lent monster world.
fn umod_get_hit<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId) {
    let mut sim = Sim {
        game: &mut *cv.game,
        units: &mut *cv.v.units,
        stats: &mut *cv.v.stats,
        data: cv.v.data,
    };
    cv.v.h.run_umods(&mut sim, d, None, umod_mode::GET_HIT);
}

/// The soft-hit reaction: soft, AI state 19, umod mode 4.
fn soft_reaction<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId) {
    soft(cv, d);
    cv.v.h.set_monster_ai_state(d, AI_STATE_HIT);
    umod_get_hit(cv, d);
}

/// The reaction of a monster defender.
fn monster_hit<X: Pending>(
    cv: &mut CombatView<'_, X>,
    a: UnitId,
    d: UnitId,
    rec: &mut DamageRecord,
) {
    let Some((mode, class)) = cv.v.units.get(d).map(|r| (r.mode, r.class)) else {
        return;
    };
    if mode == monster_mode::DT || mode == monster_mode::DD {
        return;
    }
    let has_mode = |cv: &CombatView<'_, X>, m: u8| cv.v.h.x.class_has_mode(class as i32, m);
    // A knockback on a class with no knockback mode becomes a get-hit.
    if rec.result & result::KNOCKBACK != 0 && !has_mode(cv, MONSTER_KB) {
        rec.result = (rec.result & !result::KNOCKBACK) | result::GET_HIT;
    }
    // A get-hit on a sandleaper that is not frozen becomes a knockback.
    if rec.result & result::GET_HIT != 0 {
        let t = cv.v.h.tables.clone();
        let leaper = t
            .combat
            .monstats
            .get(class as usize)
            .is_some_and(|m| m.baseid == SANDLEAPER_BASE);
        if leaper && !cv.v.stats.has_state(d, STATE_FREEZE) {
            rec.result |= result::KNOCKBACK;
        }
    }
    let f = rec.result;
    // A fatal hit kills the monster.
    if f & result::WILL_DIE != 0 {
        kill(cv, d, a);
        return;
    }
    // Knockback: request the knockback mode toward the attacker.
    if f & result::KNOCKBACK != 0 {
        monster_request(cv, d, MONSTER_KB, Some(a));
        return;
    }
    // Block: a soft hit, a class that never blocks or one with no block
    // mode only sets the AI state; otherwise the block mode is requested.
    if f & result::BLOCK != 0 {
        if f & result::SOFT_HIT != 0
            || NO_BLOCK_CLASSES.contains(&class)
            || !has_mode(cv, MONSTER_BL)
        {
            cv.v.h.set_monster_ai_state(d, AI_STATE_HIT);
        } else {
            monster_request(cv, d, MONSTER_BL, None);
        }
        return;
    }
    // Get-hit: a stunned unit or one that enters get-hit plays the get-hit
    // mode; otherwise the soft reaction.
    if f & result::GET_HIT != 0 {
        if cv.v.state_list(d, STATE_STUNNED as u16).is_some() || enters_get_hit(cv, d, rec) {
            monster_request(cv, d, monster_mode::GH as u8, None);
            umod_get_hit(cv, d);
        } else {
            soft_reaction(cv, d);
        }
        return;
    }
    // Soft hit.
    if f & result::SOFT_HIT != 0 {
        soft_reaction(cv, d);
        return;
    }
    // A change of more than 4 in the life percentage since the last one sent
    // is a soft hit, so the client gets the new value.
    if f & result::HIT != 0 && rec.total > 0 {
        let b = cv.v.stats.unit_total(d, LAST_SENT_HP_PCT, 0) & 0xFF;
        let hp = cv.v.stats.unit_total(d, crate::stats::stat::HITPOINTS, 0);
        let c = crate::stats::life_fraction(hp, cv.v.stats.max_life(d)) & 0xFF;
        if b.wrapping_sub(c).wrapping_abs() > 4 {
            soft(cv, d);
        }
    }
}

/// Player modes the reaction requests.
const PLAYER_GH: u32 = 4;
const PLAYER_BL: u32 = 9;
const PLAYER_SEQUENCE: u32 = 13;
const PLAYER_KB: u32 = 19;
/// States 65 `dodge`, 66 `avoid`, 68 `evade`.
const STATE_DODGE: u16 = 65;
const STATE_AVOID: u16 = 66;
const STATE_EVADE: u16 = 68;
/// Stat 350 (the state's skill), 95 `lastblockframe`, 102
/// `item_fasterblockrate`.
const STAT_STATE_SKILL: u16 = 350;
const LAST_BLOCK_FRAME: u16 = 95;
const FASTER_BLOCK_RATE: u16 = 102;
/// Skill entry flag 4, set when the entry is used as a hit reaction.
const ENTRY_FLAG_REACTION: u32 = 4;
/// Sound event 12 (evade).
const SOUND_EVADE: u16 = 12;

/// A player mode request (point form or unit form, re-entry 0): the walk
/// code when the path provider is on, else [`Pending::player_mode_request`].
fn player_request<X: Pending>(
    cv: &mut CombatView<'_, X>,
    d: UnitId,
    skill: Option<u16>,
    mode: u32,
    target: WalkTarget,
) {
    let game = &mut *cv.game;
    if cv.v.h.paths.is_some() {
        crate::wiring::path::walk::player_request(&mut cv.v, game, d, skill, mode, target);
    } else {
        cv.v.h.x.player_mode_request(game, d, skill, mode, target);
    }
}

/// The unit form's target: the attacker's type and GUID.
fn attacker_target<X: Pending>(cv: &CombatView<'_, X>, a: UnitId) -> WalkTarget {
    let (ty, guid) = CombatWorld::ident(cv, a);
    WalkTarget::Unit { ty, guid }
}

/// For the dodge, avoid and evade states: s is stat 350 of the state's list,
/// and the result is the highest entry of skill s (`highest_entry`).
fn state_skill<X: Pending>(
    cv: &CombatView<'_, X>,
    d: UnitId,
    st: u16,
) -> Option<crate::skills::SkillEntry> {
    let l = cv.v.state_list(d, st)?;
    let s = cv.v.stats.base(l, STAT_STATE_SKILL, 0);
    crate::skills::highest_entry(&cv.v.h.skill_list_of(d), s)
}

/// The skill form of the dodge, avoid and evade reactions: the entry's flags
/// get bit 4, then the unit form request (entry, mode 13, attacker, 0).
fn skill_form<X: Pending>(
    cv: &mut CombatView<'_, X>,
    a: UnitId,
    d: UnitId,
    e: crate::skills::SkillEntry,
) {
    let f = cv.v.h.x.entry_flags(d, &e);
    cv.v.h.x.set_entry_flags(d, &e, f | ENTRY_FLAG_REACTION);
    let t = attacker_target(cv, a);
    player_request(cv, d, Some(e.skill as u16), PLAYER_SEQUENCE, t);
}

/// The reaction of a player defender (the first matching result flag wins).
fn player_hit<X: Pending>(cv: &mut CombatView<'_, X>, a: UnitId, d: UnitId, rec: &DamageRecord) {
    let f = rec.result;
    // Dodge or avoid.
    if f & (result::DODGE | result::AVOID) != 0 {
        let st = if f & result::DODGE != 0 {
            STATE_DODGE
        } else {
            STATE_AVOID
        };
        if let Some(e) = state_skill(cv, d, st) {
            skill_form(cv, a, d, e);
        }
        return;
    }
    // Evade.
    if f & result::EVADE != 0 {
        if let Some(e) = state_skill(cv, d, STATE_EVADE) {
            skill_form(cv, a, d, e);
            let t = cv.v.h.tables.clone();
            if t.skills
                .skill(e.skill)
                .is_some_and(|r| r.stsound as i16 > 0)
            {
                if let Err(err) = crate::units::sound::queue_sound(cv.game, d, SOUND_EVADE, Some(d))
                {
                    cv.v.unit_error(crate::game::GameError::from(err).into());
                }
            }
        }
        return;
    }
    // Block.
    if f & (result::BLOCK | result::WEAPON_BLOCK) != 0 {
        if f & result::SOFT_HIT != 0 {
            return;
        }
        let frame = cv.game.frame;
        let last = cv.v.stats.unit_total(d, LAST_BLOCK_FRAME, 0);
        let rate = cv.v.stats.unit_total(d, FASTER_BLOCK_RATE, 0);
        if frame.wrapping_sub(last) > cv.game.ticks(rate / 8 + 15) {
            player_request(cv, d, None, PLAYER_BL, WalkTarget::Point(Point::new(0, 0)));
            cv.v.set_base(d, LAST_BLOCK_FRAME, frame);
        }
        return;
    }
    // Fatal hit: the player's death starts.
    if f & result::WILL_DIE != 0 {
        let mode = cv.v.units.get(d).map_or(0, |r| r.mode);
        if mode == player_mode::DT || mode == player_mode::DD {
            return;
        }
        // The unit form (no skill, mode 0, attacker) resolves the attacker
        // and, past the mode and interrupt checks (allowed for mode 0),
        // runs the death start with the attacker as the killer.
        let game = &mut *cv.game;
        cv.v.start_player_death(game, d, Some(a));
        return;
    }
    // Knockback.
    if f & result::KNOCKBACK != 0 {
        let t = attacker_target(cv, a);
        player_request(cv, d, None, PLAYER_KB, t);
        return;
    }
    // Get-hit.
    if f & result::GET_HIT != 0 {
        if cv.v.state_list(d, STATE_STUNNED as u16).is_none() && !enters_get_hit(cv, d, rec) {
            soft(cv, d);
        } else {
            let p = WalkTarget::Point(Point::new(rec.total, 0));
            player_request(cv, d, None, PLAYER_GH, p);
        }
        return;
    }
    // Soft hit.
    if f & result::SOFT_HIT != 0 {
        soft(cv, d);
    }
}

/// State 21 `stunned`.
const STATE_STUNNED: u32 = 21;

/// Unit flag 0x04000000: no experience for this victim.
pub const UNIT_FLAG_NO_EXPERIENCE: u32 = 0x0400_0000;

/// The kill of `d` by `a`:
///
/// 1. Guards: a player victim in mode 0 or 17, a monster in mode 0 or 12
///    or not `killable`, any other type: stop. A monster victim then
///    gets the pet death bookkeeping (flag 1).
/// 2. The experience distribution unless `d` has unit flag 0x04000000; the
///    arena kill event.
/// 3. A monster victim: the death mode request toward A, the quest kill
///    parse unless D has unit flag 0x80000000, the barricade doors.
pub fn kill<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId, a: UnitId) {
    kill_by(cv, d, Some(a));
}

/// [`kill`] with the killer optional: step 2 above needs a killer, and step 3
/// faces the victim's current direction without one. The monster death by
/// regeneration passes the poison / open-wounds owner, which may be gone.
///
/// Open point: without a killer, the [`KillStep`] seams (pet credit, quest
/// kill parse, barricade doors) take a unit and are not called.
pub fn kill_by<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId, a: Option<UnitId>) {
    let Some(r) = cv.v.units.get(d) else {
        return;
    };
    let (ty, class, mode, flags) = (r.ty, r.class, r.mode, r.flags);
    // Guards.
    match ty {
        UnitType::Player => {
            if mode == player_mode::DT || mode == player_mode::DD {
                return;
            }
        }
        UnitType::Monster => {
            if mode == monster_mode::DT || mode == monster_mode::DD {
                return;
            }
            let t = cv.v.h.tables.clone();
            if !t
                .combat
                .monstats
                .get(class as usize)
                .is_some_and(|m| m.killable)
            {
                return;
            }
            // Flag 1 here, so a hireling with a player owner gets the pet
            // death bookkeeping (on the host that holds the hireling lists,
            // `ActionHooks::pet_deaths`); the other pet types stay on the
            // seam.
            if let Some(q) = cv.v.h.pet_deaths.as_mut() {
                q.push(d);
            }
            let game = &mut *cv.game;
            if let Some(a) = a {
                cv.v.h.x.kill_step(game, KillStep::PetCredit, d, a);
            }
        }
        _ => return,
    }
    // Experience and the arena kill event.
    if let Some(a) = a {
        if flags & UNIT_FLAG_NO_EXPERIENCE == 0 {
            if let Some(t) = cv.v.h.vitals.clone() {
                distribute(cv, &t, a, d);
            }
        }
        // The arena kill event.
        if let (Some(row), Some(ka), Some(kd)) = (
            cv.v.h.tables.arena.first().map(ArenaRow::from),
            cv.v.units.get(a).map(|r| r.ty),
            cv.v.units.get(d).map(|r| r.ty),
        ) {
            if let Some(st) = cv.v.h.arena.as_mut() {
                st.kill_event(&row, &mut *cv.game, (a, ka), (d, kd));
            }
        }
        let game = &mut *cv.game;
        cv.v.h
            .x
            .kill_step(game, KillStep::AttackerBookkeeping, d, a);
    }
    if ty != UnitType::Monster {
        killed_event(cv, d, a);
        return;
    }
    // The death mode request.
    let game = &mut *cv.game;
    if let Some(a) = a {
        // The death request's direction toward the attacker, applied by the
        // mode set's direction snap.
        let at = cv.v.h.path_position(a);
        if let Some(dir) = cv.v.path_dir64(d, at) {
            cv.v.path_snap_direction(d, dir);
        }
        cv.v.h.x.kill_step(game, KillStep::FaceAttacker, d, a);
    }
    // Building the death request clears the used skill: a monster killed
    // while a skill is in use sends the mode message 0x69, not the skill
    // message 0x4D.
    AiModes::clear_current_skill(&mut cv.v, d);
    cv.v.h.mode_target = a;
    cv.v.monster_set_mode(game, d, monster_mode::DT);
    cv.v.h.mode_target = None;
    if let Some(a) = a {
        if !cv.v.h.x.is_revived(d) {
            // The kill parse's player: a monster killer whose minion owner
            // (from its AI control) is a player stands for that player, so a
            // hireling's kill credits its owner.
            let owner = (cv.v.units.get(a).map(|r| r.ty) == Some(UnitType::Monster))
                .then(|| cv.v.h.ai.as_ref()?.control(a)?.minion_owner)
                .flatten()
                .filter(|o| o.ty == UnitType::Player)
                .and_then(|o| game.lists.find_unit(o.ty, o.guid));
            // Inside the kill, after the treasure drop: on the lent quest
            // control, else queued.
            cv.v.quest_kill_now(game, d, owner.unwrap_or(a));
        }
        cv.v.h.x.kill_step(game, KillStep::BarricadeDoors, d, a);
    }
    killed_event(cv, d, a);
}

/// A mod game's kill hook, after D2RUST's kill of `d` by `a`.
fn killed_event<X: Pending>(cv: &mut CombatView<'_, X>, d: UnitId, a: Option<UnitId>) {
    super::mod_events::fire(cv, |e, h| e.unit_killed(h, a, d));
}

/// The distribution's world calls on the action
/// wiring: the credited player, the hireling share and the party stay
/// [`Pending`] seams (single player: no party, so the solo path).
impl<X: Pending> ExpShare for View<'_, X> {
    /// A hireling attacker (`ActionHooks::hireling_ai`): its player owner
    /// (its minion owner); any other monster: the seam. The flag-0x800 stat
    /// list of an `exp` state is not looked up here: a hireling in such a
    /// state is credited to its owner.
    fn credited_player(&self, attacker: UnitId, defender: UnitId) -> Option<UnitId> {
        match self.h.hireling_ai.ids.get(&attacker) {
            Some(&(owner, _)) => Some(owner),
            None => self.h.x.kill_credited_player(attacker, defender),
        }
    }
    /// The hireling's experience share on the published hireling facts:
    /// `hireling_kill_share`; a player without a living hireling there
    /// goes to the seam.
    fn hireling_share(&mut self, p: UnitId, attacker: UnitId, defender: UnitId, e: i32) {
        if !hireling_kill_share(self, p, attacker, defender, e) {
            self.h.x.kill_hireling_share(p, attacker, defender, e);
        }
    }
    fn in_party(&self, p: UnitId) -> bool {
        self.h.x.kill_in_party(p)
    }
    fn party_members(&self, p: UnitId, defender: UnitId) -> Vec<UnitId> {
        self.h.x.kill_party_members(p, defender)
    }
}

/// The distribution on combat's view: [`View`]'s calls, with the game
/// for unit event 12.
impl<X: Pending> ExpShare for CombatView<'_, X> {
    fn credited_player(&self, attacker: UnitId, defender: UnitId) -> Option<UnitId> {
        self.v.credited_player(attacker, defender)
    }
    fn hireling_share(&mut self, p: UnitId, attacker: UnitId, defender: UnitId, e: i32) {
        self.v.hireling_share(p, attacker, defender, e);
    }
    fn in_party(&self, p: UnitId) -> bool {
        self.v.in_party(p)
    }
    fn party_members(&self, p: UnitId, defender: UnitId) -> Vec<UnitId> {
        self.v.party_members(p, defender)
    }
}

/// [`View`]'s vitals calls; unit event 12 through the event registry
/// ([`CombatView::fire_unit_event`]).
impl<X: Pending> VitalsUnits for CombatView<'_, X> {
    type Unit = UnitId;

    fn rules(&self) -> &crate::rules::Ruleset {
        &self.v.h.rules
    }
    fn unit_type(&self, u: UnitId) -> UnitType {
        VitalsUnits::unit_type(&self.v, u)
    }
    fn class_id(&self, u: UnitId) -> i32 {
        VitalsUnits::class_id(&self.v, u)
    }
    fn base_stat(&self, u: UnitId, s: u16) -> i32 {
        VitalsUnits::base_stat(&self.v, u, s)
    }
    fn stat(&self, u: UnitId, s: u16) -> i32 {
        VitalsUnits::stat(&self.v, u, s)
    }
    fn set_base_stat(&mut self, u: UnitId, s: u16, v: i32) {
        self.v.set_base_stat(u, s, v);
    }
    fn add_base_stat(&mut self, u: UnitId, s: u16, v: i32) {
        self.v.add_base_stat(u, s, v);
    }
    fn max_life(&self, u: UnitId) -> i32 {
        VitalsUnits::max_life(&self.v, u)
    }
    fn max_mana(&self, u: UnitId) -> i32 {
        VitalsUnits::max_mana(&self.v, u)
    }
    fn max_stamina(&self, u: UnitId) -> i32 {
        VitalsUnits::max_stamina(&self.v, u)
    }
    /// The unit joins its room's update queue, so its player update sends
    /// the changed stats.
    fn refresh(&mut self, u: UnitId) {
        VitalsUnits::refresh(&mut self.v, u);
        let _ = self.game.lists.queue_update(u);
    }
    /// The level-up: the party roster (not sent), the level-up sound
    /// (event 2), the broadcast (S->C 0x75 to every client), then the item
    /// refresh: the item relink and the inventory pass, then the owner
    /// refresh. The first two need the inventory model, which is the host's:
    /// [`LEVEL_UP_ITEMS_MARK`] at this place of the stream (when
    /// [`super::ActionHooks::item_marks`] is on). The owner refresh is run
    /// here: the player is queued and its second flag word gets bits 0 and 1
    /// set, as the environment refresh of [`super::ActionSim`] does.
    fn level_up_notify(&mut self, u: UnitId) {
        self.v.level_up_notify(u);
        let _ = crate::units::sound::queue_sound(&mut *self.game, u, 2, None);
        let Some((guid, level)) = self
            .v
            .units
            .get(u)
            .map(|r| (r.guid, self.v.stats.unit_total(u, 12, 0) as u16))
        else {
            return;
        };
        for to in super::dying::client_players(self.game) {
            let m = crate::units::messages::player_party_info(guid, level, to == u);
            self.v.h.x.send(to, &m);
        }
        if self.v.h.item_marks {
            let mut mark = [LEVEL_UP_ITEMS_MARK; 5];
            mark[1..].copy_from_slice(&guid.to_le_bytes());
            self.v.h.x.send(u, &mark);
        }
        let _ = self.game.lists.queue_update(u);
        if let Some(r) = self.v.units.get_mut(u) {
            r.flags2 |= 3;
        }
    }
    /// Without the registry: [`Pending::level_up_event`].
    fn level_up_event(&mut self, u: UnitId) {
        if self.v.h.unit_events.is_some() {
            self.fire_unit_event(EV_LEVELUP, Some(u), None, None);
        } else {
            self.v.h.x.level_up_event(u);
        }
    }
}

/// The vitals' view on the action wiring: unit records, stat lists
/// (with the action hooks as stat host) and [`Pending`]'s vitals seams.
impl<X: Pending> VitalsUnits for View<'_, X> {
    type Unit = UnitId;

    fn rules(&self) -> &crate::rules::Ruleset {
        &self.h.rules
    }
    fn unit_type(&self, u: UnitId) -> UnitType {
        self.units.get(u).map_or(UnitType::Tile, |r| r.ty)
    }
    fn class_id(&self, u: UnitId) -> i32 {
        self.units.get(u).map_or(0, |r| r.class as i32)
    }
    /// The base value of stat `s`.
    fn base_stat(&self, u: UnitId, s: u16) -> i32 {
        self.stats.unit_base(u, s, 0)
    }
    /// The total value of stat `s`.
    fn stat(&self, u: UnitId, s: u16) -> i32 {
        self.stats.unit_total(u, s, 0)
    }
    /// Sets the base value of stat `s`.
    fn set_base_stat(&mut self, u: UnitId, s: u16, v: i32) {
        self.stats.unit_set(&mut *self.h, u, s, v, 0);
    }
    /// Adds to the base value of stat `s`.
    fn add_base_stat(&mut self, u: UnitId, s: u16, v: i32) {
        self.stats.unit_add(&mut *self.h, u, s, v, 0);
    }
    fn max_life(&self, u: UnitId) -> i32 {
        self.stats.max_life(u)
    }
    fn max_mana(&self, u: UnitId) -> i32 {
        self.stats.max_mana(u)
    }
    fn max_stamina(&self, u: UnitId) -> i32 {
        self.stats.max_stamina(u)
    }
    fn refresh(&mut self, u: UnitId) {
        self.h.x.stats_refresh(u);
    }
    fn level_up_notify(&mut self, u: UnitId) {
        self.h.x.level_up_notify(u);
    }
    /// Without a game: [`Pending::level_up_event`] (the kill's
    /// distribution runs on [`CombatView`], which fires the event).
    fn level_up_event(&mut self, u: UnitId) {
        self.h.x.level_up_event(u);
    }
}

/// The experience share of player `p`'s living hireling H, from the
/// published hireling facts (`ActionHooks::hireling_ai`: H is the unit whose
/// owner is `p`; its row by node `Id`). The gain and the experience add run
/// inline, so the 0xA1 / 0xA2 / 0xA0 goes out inside the kill, before its
/// drop. A level-up is queued as [`super::HirelingCall::LevelUp`]
/// for the host holding the hireling lists. `false`: no hireling facts
/// for `p` (the seam answers).
fn hireling_kill_share<X: Pending>(
    v: &mut View<'_, X>,
    p: UnitId,
    attacker: UnitId,
    defender: UnitId,
    e: i32,
) -> bool {
    use crate::combat::pct;
    use crate::combat::vitals::level_factor;
    use crate::world::hirelings::level::{exp_delta_message, GAIN_CAP, NON_KILLER_SHARE};
    use crate::world::hirelings::rows::threshold;
    use crate::world::hirelings::stat;
    let Some((h, id)) =
        v.h.hireling_ai
            .ids
            .iter()
            .find(|(_, &(o, _))| o == p)
            .map(|(&m, &(_, id))| (m, id))
    else {
        return false;
    };
    let (Some(rows), Some(t)) = (v.h.hireling_ai.rows.clone(), v.h.vitals.clone()) else {
        return false;
    };
    let expansion = v.data.expansion;
    let row = |l: i32| {
        rows.row_at(expansion, id, l)
            .and_then(|i| rows.rows.get(i))
            .copied()
    };
    // Base reads.
    let alvl = v.base_stat(h, stat::LEVEL);
    let dlvl = v.base_stat(defender, stat::LEVEL);
    let max = t.max_level(0) as i32;
    // The hireling's experience gain.
    let mut g = if e <= 0 {
        1
    } else if alvl >= max {
        0
    } else {
        let mut g = level_factor(e.min(GAIN_CAP), alvl, dlvl);
        g = t.apply_exp_ratio(g, alvl);
        let x = v.stat(h, crate::combat::vitals::experience::ADDEXPERIENCE);
        if x != 0 {
            g = g.wrapping_add(pct(g, x, 100));
        }
        // H is `p`'s, with a row at alvl: the gain is capped by a step of
        // its level table.
        if let Some(r) = row(alvl) {
            let step = (threshold(r.exp_lvl, alvl.wrapping_add(1))
                .wrapping_sub(threshold(r.exp_lvl, alvl)) as u32)
                >> 6;
            g = (g as u32).min(step) as i32;
        }
        g
    };
    if attacker != h {
        g = g.wrapping_mul(NON_KILLER_SHARE) / 256;
    }
    // The experience add and level-up check.
    if g <= 0 || alvl >= v.stat(p, stat::LEVEL) {
        return true;
    }
    let Some(r) = row(alvl) else {
        return true;
    };
    let cap = max.wrapping_sub(1);
    if alvl >= cap {
        return true;
    }
    let old = v.base_stat(h, stat::EXPERIENCE) as u32;
    let new = old.wrapping_add(g.wrapping_mul(2) as u32);
    v.set_base_stat(h, stat::EXPERIENCE, new as i32);
    let guid = v.units.get(h).map_or(0, |u| u.guid);
    let msg = exp_delta_message(stat::EXPERIENCE as u8, guid, old, new);
    v.h.x.send(p, &msg);
    let mut lvl = alvl;
    loop {
        let next = lvl.wrapping_add(1);
        if threshold(r.exp_lvl, next) as u32 > new {
            break;
        }
        lvl = next;
        if next >= cap {
            break;
        }
    }
    if lvl > alvl {
        if let Some(q) = v.h.hireling_calls.as_mut() {
            q.push(super::HirelingCall::LevelUp {
                player: p,
                merc: h,
                level: lvl,
            });
        }
    }
    true
}

#[cfg(test)]
mod level_up_tests {
    use super::LEVEL_UP_ITEMS_MARK;
    use crate::units::UnitType;
    use crate::wiring::action::tests::Fx;

    /// The level-up leaves the item-refresh mark for the host (item relink
    /// and inventory pass) after its 0x75 broadcast, when the item marks are
    /// on; never otherwise.
    #[test]
    fn the_level_up_marks_the_item_refresh_after_the_broadcast() {
        for marks in [false, true] {
            let mut fx = Fx::new();
            let p = fx.spawn(UnitType::Player, 0, fx.a, 10, 10);
            let guid = fx.sim.sys.units.get(p).unwrap().guid;
            fx.sim.sys.hooks.item_marks = marks;
            fx.sim.sys.hooks.x.sent.clear();
            fx.sim.combat(&mut fx.game, |w, _| {
                crate::combat::vitals::VitalsUnits::level_up_notify(w, p)
            });
            let sent = &fx.sim.sys.hooks.x.sent;
            let mark: Vec<u8> = [&[LEVEL_UP_ITEMS_MARK][..], &guid.to_le_bytes()].concat();
            let at = sent.iter().position(|(to, b)| *to == p && *b == mark);
            assert_eq!(at.is_some(), marks);
            if let Some(at) = at {
                assert!(sent[at + 1..].iter().all(|(_, b)| b.first() != Some(&0x75)));
            }
        }
    }
}

#[cfg(test)]
mod quest_kill_inside {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::kill_by;
    use crate::game::Game;
    use crate::units::{UnitId, UnitType};
    use crate::wiring::action::objects::{ObjectRoute, QuestObjectCall, QuestObjectHost};
    use crate::wiring::action::tests::{Fx, TestPending};
    use crate::wiring::action::View;

    /// The (victim, killer) of each kill parse the host ran.
    type Parsed = Rc<RefCell<Vec<(UnitId, Option<UnitId>)>>>;

    struct Lent(Parsed);

    impl QuestObjectHost<TestPending> for Lent {
        fn run(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, TestPending>,
            _: QuestObjectCall,
        ) -> Option<ObjectRoute> {
            None
        }
        fn kill_parse(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, TestPending>,
            victim: UnitId,
            killer: Option<UnitId>,
        ) -> bool {
            self.0.borrow_mut().push((victim, killer));
            true
        }
        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    /// With the quest control lent, the kill parse runs inside the kill
    /// (after the treasure drop, before the barricade step) and nothing is
    /// queued.
    #[test]
    fn the_kill_parse_runs_inside_the_kill_on_the_lent_quest_control() {
        let mut fx = Fx::new();
        let a = fx.a;
        let p = fx.spawn(UnitType::Player, 0, a, 10, 10);
        let m = fx.spawn(UnitType::Monster, 0, a, 12, 12);
        let parsed: Parsed = Rc::new(RefCell::new(Vec::new()));
        fx.sim.sys.hooks.quest_host = Some(Box::new(Lent(parsed.clone())));
        fx.sim.sys.hooks.x.log.clear();
        fx.sim.combat(&mut fx.game, |w, _| kill_by(w, m, Some(p)));
        assert_eq!(*parsed.borrow(), [(m, Some(p))]);
        let log = &fx.sim.sys.hooks.x.log;
        assert!(
            !log.iter().any(|l| l.starts_with("kill QuestKill")),
            "{log:?}"
        );
        assert!(log.contains(&format!("kill BarricadeDoors {} {}", m.0, p.0)));
        assert!(fx.sim.sys.hooks.quest_host.is_some());
    }
}
