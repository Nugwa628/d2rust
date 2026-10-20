//! The unit side of the wiring: the unit hooks of [`ActionHooks`] (the
//! missile class handler for missile events, the AI think and reset for
//! monster events 2 and 10, the state-54 rule before a think is
//! scheduled, the town test, the combat list drop, the kind frees, the
//! skill events 5 / 8 / 9 through [`Pending::skill_event`], the player
//! action frame through [`Pending::action_frame`], the AnimData record
//! of a unit's mode and the monster death
//! start through [`Pending::monster_death_start`]; with a lent monster
//! world ([`super::monsters`]): the monster type init of the allocator,
//! the monster state's part of a free, event 7 and the mode change's
//! umod callbacks), and
//! the unit-field helpers of [`View`] the other adapters share (stats,
//! states, state lists, seeds).

use crate::game::Game;
use crate::missiles;
use crate::monsters::ai;
use crate::rng::Seed;
use crate::stats::lists::RemoveCallback;
use crate::stats::states::state;
use crate::stats::{ListId, StatHost, StatLists};
use crate::tick::events::event;
use crate::units::anim_rate::{mode_fallback, MONSTER_TO_PLAYER, PLAYER_TO_MONSTER};
use crate::units::hooks::{Sim, UnitHooks};
use crate::units::lifecycle::{ActTail, AllocRequest, LifecycleHooks};
use crate::units::modes::UnitError;
use crate::units::modes::MONSTER_MODES;
use crate::units::record::{flags2, AnimRecord, Sequence, ANIM_EVENTS};
use crate::units::{UnitId, UnitType};
use d2_data::tables::{Monstats, Monstats2};
use d2_formats::animdata::AnimData;

use super::combat::HIRELING_CLASSES;
use super::monsters::umod_mode;
use super::{ActionHooks, Pending, SkillEvent, View, WiringError};

/// The client status word's dead bit.
pub const STATUS_DEAD: u16 = 0x08;
/// The spread of the mercenary's creation.
const HIRE_SPREAD: i32 = 4;

/// Stat-list state of `justhit`.
pub const STATE_JUSTHIT: u16 = 86;
/// State 92 (`death_delay`), cleared for players.
pub const STATE_DEATH_DELAY: u16 = 92;

/// The remove callbacks that reset the alignment: Confuse, Attract,
/// Conversion and Mind Blast. The expiry walk frees an expired list
/// through a detach that runs any list's callback, so a timed Conversion
/// ends with its alignment reset. Their bodies need the skill use view,
/// so they run as [`super::SkillEvent::ListRemoved`].
pub const ALIGNMENT_CALLBACKS: [u32; 4] = {
    use crate::skills::use_::bodies::callback::{ATTRACT, CONFUSE, CONVERSION, MIND_BLAST};
    [CONFUSE, ATTRACT, CONVERSION, MIND_BLAST]
};

impl<X: Pending> StatHost for ActionHooks<X> {
    fn mod_stat_callback(
        &mut self,
        lists: &mut StatLists,
        ev: &crate::stats::CallbackEvent,
    ) -> Option<()> {
        let rules = self.rules.clone();
        let provider = rules.items()?;
        if !provider.has_stat_callback(crate::stats::key_stat(ev.key)) {
            return None;
        }
        provider.stat_callback(lists, self, ev)
    }

    /// The base time of the unit's act (the environment
    /// record's ticks / speed), kept for the players.
    fn act_time(&self, unit: UnitId) -> Option<i32> {
        self.act_times.get(&unit).copied()
    }
    /// Queues the callbacks this wiring runs after the expiry
    /// walk ([`UnitHooks::lists_expired`]): the default one, the shrine
    /// ones, and Inferno's / Blade Fury's (state off, unit flags |= 0x40;
    /// the timer 12
    /// expiry is what ends a player's Inferno / Arctic Blast channel), and
    /// the alignment ones ([`ALIGNMENT_CALLBACKS`], run by the skill
    /// bodies' dispatcher). The others are run by their skill bodies.
    fn list_removed(
        &mut self,
        _lists: &mut StatLists,
        unit: UnitId,
        state: u32,
        list: ListId,
        callback: RemoveCallback,
    ) {
        use crate::skills::use_::bodies::callback::{
            BLADE_FURY, CHARGE, COLD, DEFAULT, FREEZE_END, INFERNO, JUSTHIT,
        };
        use crate::world::objects::shrines::{SKILL_REMOVE, STAMINA_REMOVE};
        if matches!(
            callback.0,
            JUSTHIT
                | CHARGE
                | DEFAULT
                | COLD
                | FREEZE_END
                | SKILL_REMOVE
                | STAMINA_REMOVE
                | INFERNO
                | BLADE_FURY
        ) || ALIGNMENT_CALLBACKS.contains(&callback.0)
        {
            self.removed_lists.push((unit, state, callback.0, list));
        }
    }
    /// The skill handlers of stats 83,
    /// 126, 127 and 188 refresh every passive state of the unit
    /// (from inside the stat write). Queued
    /// here, where the lists are borrowed, and run as soon as the outer
    /// write returns (the skill-use view's
    /// attach and free, [`UnitHooks::lists_expired`], else the unit's
    /// next update). Stats 97 and 107 of a player are queued
    /// (`oskill_changes`); 98, 151 and 204 are run by other paths of this
    /// wiring.
    fn skill_stat_changed(
        &mut self,
        lists: &mut StatLists,
        owner: UnitId,
        key: i32,
        _old: i32,
        new: i32,
    ) {
        let stat = crate::stats::key_stat(key);
        // Stats 97 / 107 of a player: queued for the oskill callback
        // ([`ActionHooks::oskill_changes`]).
        if matches!(stat, 97 | 107)
            && lists
                .unit_list(owner)
                .is_some_and(|r| lists.owner_type(r) == crate::stats::lists::owner::PLAYER)
        {
            self.oskill_changes
                .push((owner, stat, key as u32 as u16, new));
        }
        if matches!(stat, 83 | 126 | 127 | 188) {
            let e = (owner, stat, key as u32 as u16);
            if !self.skill_refresh.contains(&e) {
                self.skill_refresh.push(e);
            }
        }
    }
    /// Item event registration on the owner: a record of kind 2, key k is
    /// looked up; `itemevent1`
    /// / `itemevent2` are added with their functions, kind 2, key k, skill k,
    /// level 0; a new value of 0 removes the records. The
    /// events then run from the damage events (e.g. a unique with a
    /// skill-on-hit stat).
    fn item_event_register(&mut self, owner: UnitId, key: i32, new: i32, events: [(i16, i16); 2]) {
        use crate::skills::use_::bodies::{event_func_filled, Handler};
        let list = self.handlers.entry(owner).or_default();
        if new == 0 {
            list.retain(|h| !(h.key_type == 2 && h.key == key));
            return;
        }
        if list
            .iter()
            .any(|h| h.key_type == 2 && h.key == key && h.skill == key)
        {
            return;
        }
        for (event, func) in events {
            if event <= 0 {
                break;
            }
            // An unknown function is refused.
            if !event_func_filled(i32::from(func)) {
                continue;
            }
            list.insert(
                0,
                Handler {
                    event: event as u8,
                    key_type: 2,
                    key,
                    skill: key,
                    level: 0,
                    func: i32::from(func),
                },
            );
        }
    }
    /// Whether `state` stays on a dead unit: state in
    /// range and flag `monstaydeath` for a monster, `plrstaydeath` for any
    /// other unit.
    fn stays_on_death(&self, lists: &StatLists, unit: UnitId, state: u32) -> bool {
        use crate::stats::states::group;
        let monster = lists
            .unit_list(unit)
            .is_some_and(|r| lists.owner_type(r) == crate::stats::lists::owner::MONSTER);
        let g = if monster {
            group::MON_STAY_DEATH
        } else {
            group::PLR_STAY_DEATH
        };
        lists.data().states.has_flag(state, g)
    }
}

impl<X: Pending> ActionHooks<X> {
    /// Freeze end, the remove callback of the freeze list: a dead unit
    /// whose state stays on death keeps it; otherwise the
    /// state goes off (with the update-queue insert), the plain mode set
    /// to mode 1, and for a monster its thinks (timer
    /// type 2) are cancelled and one is scheduled at frame + `aidel`
    /// (0 → 15).
    fn freeze_end(&mut self, sim: &mut Sim<'_>, u: UnitId, state: u32) {
        let Some((ty, class, dead)) = sim.units.get(u).map(|r| (r.ty, r.class, r.is_dead())) else {
            return;
        };
        if dead && self.stays_on_death(sim.stats, u, state) {
            return;
        }
        // A dead unit whose state 1 does not stay on death (the death
        // sweep after the DT start's mode 0): it stays dead,
        // the keep mask clears the state (recorded,
        // `w2-09-frozen-kill`: Fallen 1:27 killed while frozen at f325,
        // DT then DD at f345 in the game, never mode 1).
        if dead {
            return;
        }
        sim.stats.toggle_state(u, state, false);
        if let Err(e) = sim.game.lists.queue_update(u) {
            self.errors
                .push(WiringError::Unit(UnitError::Game(e.into())));
        }
        if let Err(e) = crate::units::modes::set_mode(sim, self, u, 1) {
            self.errors.push(WiringError::Unit(e));
        }
        if ty != UnitType::Monster {
            return;
        }
        let at = sim.game.frame.wrapping_add(
            sim.game
                .ticks(crate::units::modes::neutral_ai_delay(sim, class)),
        );
        sim.game.timers.cancel_unit_events(u, event::AI_THINK, None);
        if let Err(e) = sim
            .game
            .schedule_event(u, u32::from(event::AI_THINK), at, None, 0, 0)
        {
            self.errors.push(WiringError::Unit(UnitError::Game(e)));
        }
    }

    /// The draw identity
    /// of `unit`'s (type, class, mode): its own unless flag-ex bit 3
    /// ([`flags2::DISGUISE`]) is set; then the first state of
    /// [`crate::stats::states::StateTable::gfx_states`] the unit has gives
    /// the type and class, and a player shown as a monster (`gfxtype` 1)
    /// or a monster shown as a player (2) has its mode mapped. The
    /// animation lookup and rate read it.
    pub(crate) fn draw_identity(
        &self,
        sim: &Sim<'_>,
        unit: UnitId,
    ) -> Option<(UnitType, u32, u32)> {
        self.draw_identity_in(sim.units, sim.stats, unit)
    }

    /// [`Self::draw_identity`] on the unit records and stat lists.
    pub(crate) fn draw_identity_in(
        &self,
        units: &super::Units,
        stats: &StatLists,
        unit: UnitId,
    ) -> Option<(UnitType, u32, u32)> {
        let r = units.get(unit)?;
        let own = (r.ty, r.class, r.mode);
        if r.flags2 & flags2::DISGUISE == 0 {
            return Some(own);
        }
        let states = &stats.data().states;
        let Some(&(_, gfx, class)) = states
            .gfx_states()
            .iter()
            .find(|&&(s, ..)| stats.has_state(unit, s))
        else {
            return Some(own);
        };
        let class = u32::from(class);
        Some(match (gfx, r.ty) {
            (1, UnitType::Player) => {
                let combat = &self.tables.combat;
                let row = combat
                    .monstats
                    .get(class as usize)
                    .and_then(|m| combat.monstats2.get(usize::from(m.monstatsex)));
                let has = |m: u32| {
                    row.is_some_and(|r| {
                        [
                            r.mdt, r.mnu, r.mwl, r.mgh, r.ma1, r.ma2, r.mbl, r.msc, r.ms1, r.ms2,
                            r.ms3, r.ms4, r.mdd, r.mkb, r.msq, r.mrn,
                        ]
                        .get(m as usize)
                        .copied()
                        .unwrap_or(false)
                    })
                };
                let mut m = PLAYER_TO_MONSTER.get(r.mode as usize).copied().unwrap_or(1);
                while m != 1 && !has(m) {
                    m = mode_fallback(m);
                }
                (UnitType::Monster, class, m)
            }
            (1, _) => (UnitType::Monster, class, r.mode),
            (_, UnitType::Monster) => {
                let m = if class == 6 && r.mode == 4 {
                    12
                } else {
                    MONSTER_TO_PLAYER.get(r.mode as usize).copied().unwrap_or(1)
                };
                (UnitType::Player, class, m)
            }
            _ => (UnitType::Player, class, r.mode),
        })
    }

    /// The frame bonus (through [`crate::units::anim_rate::frame_bonus`]):
    /// the draw
    /// identity (T, C, M), dual-wield capability (player
    /// class 4 or 6, monster class 417 or 418), the attack weapon
    /// and its type class
    /// ([`Pending::item_type_class`]).
    pub(crate) fn frame_bonus_in(
        &self,
        units: &super::Units,
        stats: &StatLists,
        unit: UnitId,
    ) -> i32 {
        let Some((t, c, m)) = self.draw_identity_in(units, stats, unit) else {
            return 0;
        };
        let dual = match t {
            UnitType::Player => matches!(c, 4 | 6),
            UnitType::Monster => matches!(c, 417 | 418),
            _ => false,
        };
        let tc = self
            .x
            .attack_weapon(unit)
            .map(|w| self.x.item_type_class(w));
        crate::units::anim_rate::frame_bonus(t as u8, c, m, dual, tc)
    }

    /// Whether the unit can dual-wield: a player of class 4 or 6, a monster
    /// of class 417 or 418.
    fn dual_capable(ty: UnitType, class: u32) -> bool {
        match ty {
            UnitType::Player => matches!(class, 4 | 6),
            UnitType::Monster => matches!(class, 417 | 418),
            _ => false,
        }
    }

    /// The dual-wield stat toggle: which weapon's stat list is live while
    /// the attack rate is read. "Active" is the list attached to the unit;
    /// "on" / "off" attach / detach the item's list.
    fn dual_wield_stat_toggle(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let s = self.x.attack_weapon(unit);
        let w = self.x.weapon(unit);
        let other = w.and_then(|w| {
            [4u8, 5]
                .into_iter()
                .filter_map(|l| self.x.item_at(unit, l))
                .find(|&i| i != w)
        });
        let active = |sim: &Sim<'_>, i: UnitId| {
            sim.stats
                .unit_list(i)
                .is_some_and(|l| sim.stats.attached_unit(l) == Some(unit))
        };
        let set = |this: &mut Self, sim: &mut Sim<'_>, i: UnitId, on: bool| {
            let l = sim.stats.unit_list(i);
            sim.stats.equip(this, unit, l, false, on);
        };
        match (s, w) {
            (None, Some(w)) => {
                if active(sim, w) {
                    set(self, sim, w, true);
                }
            }
            (Some(s), Some(w)) if s == w => {
                if active(sim, w) {
                    set(self, sim, w, true);
                }
                if let Some(o) = other.filter(|&o| active(sim, o)) {
                    set(self, sim, o, false);
                }
            }
            (Some(s), Some(w)) => {
                if active(sim, s) {
                    set(self, sim, s, true);
                }
                if active(sim, w) {
                    set(self, sim, w, false);
                }
            }
            _ => {}
        }
    }

    /// The shared part of the animation rate
    /// ([`crate::units::anim_rate::anim_rate`]) on the draw identity, with
    /// `record` the AnimData record of that identity (s = its speed;
    /// its frames · 256 are the frame count the were-form speed reads, the
    /// record the mode start stores). The knockback and velocity parts are
    /// [`Self::movement_rate`]'s; a velocity mode without the path
    /// provider, or a type other than player and monster, is `None` (the
    /// host's [`Pending::anim_rate`]). The item/skill getter is the unit
    /// total.
    fn spec_anim_rate(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        record: &d2_formats::animdata::AnimRecord,
        frame_count: i32,
    ) -> Option<(i16, Option<i32>)> {
        use crate::units::anim_rate::{anim_rate, mode_row, Rate, RateInput};
        let (r_ty, r_flags2, r_mode) = {
            let r = sim.units.get(unit)?;
            (r.ty, r.flags2, r.mode)
        };
        let (ty, class, mode) = self.draw_identity(sim, unit)?;
        let t = match ty {
            UnitType::Player => 0,
            UnitType::Monster => 1,
            _ => return None,
        };
        if mode_row(t, class, mode).v && self.paths.is_none() {
            return None;
        }
        let used = self
            .used_skill_of(unit)
            .and_then(|e| self.tables.skills.skill(e.skill))
            .map(|row| (i32::from(row.seqtrans as i8), row.useattackrate));
        let row = mode_row(t, class, mode);
        // The dual-wield stat toggle runs first, in the modes
        // that read the attack rate.
        if (row.a || (row.a_skill && used.is_some_and(|u| u.1))) && Self::dual_capable(ty, class) {
            self.dual_wield_stat_toggle(sim, unit);
        }
        let total = |k: u16| sim.stats.unit_total(unit, k, 0);
        // Were-form speed: a player in a were-form (flag-ex bit 3) with a
        // state of group 38 `meleeonly`.
        let were_speed = (r_ty == UnitType::Player
            && r_flags2 & flags2::DISGUISE != 0
            && sim.stats.has_group(unit, MELEE_ONLY))
        .then(|| {
            let n = match self.x.attack_weapon(unit) {
                Some(w) => self.x.attack_frames(unit, w).unwrap_or(0),
                None => 19,
            };
            if n <= 0 {
                0
            } else {
                (frame_count & !0xFF) / n
            }
        });
        // A dual-capable unit with usable `weap`
        // items at body locations 4 and 5
        // averages their attack rates (stat 68).
        let dual = Self::dual_capable(ty, class)
            .then(|| {
                let pick = |loc: u8| {
                    self.x
                        .item_at(unit, loc)
                        .filter(|&i| self.x.item_usable(i) && self.x.item_is(i, WEAP_TYPE))
                };
                Some((pick(4)?, pick(5)?))
            })
            .flatten()
            .map(|(a, b)| {
                (
                    sim.stats.unit_total(a, ATTACK_RATE, 0),
                    sim.stats.unit_total(b, ATTACK_RATE, 0),
                )
            });
        let input = RateInput {
            applies: true,
            is_object_or_missile: false,
            t,
            c: class,
            m: mode,
            s: record.speed as i32,
            item: [total(93), total(99), total(105), total(102), total(96)],
            velocitypercent: total(67),
            attackrate: total(68),
            other_animrate: total(69),
            used_seqtrans: used.map(|u| u.0),
            use_attack_rate: used.is_some_and(|u| u.1),
            holyshield: sim.stats.has_state(unit, HOLY_SHIELD),
            has_path: self.path_has(unit),
            w: 0,
            velocity_mode: false,
            dual,
            player_mode_18: r_ty == UnitType::Player && r_mode == 18,
            were_speed,
        };
        match anim_rate(&input) {
            Ok(Rate::Set {
                speed, seq_speed, ..
            }) => Some((speed as i16, seq_speed)),
            _ => None,
        }
    }

    /// The anim refresh outside a mode start (a state's stat fill):
    /// the rate of the unit's current mode on its draw identity, with the
    /// frame count as the mode start left it. `None`: no rate
    /// (nothing written).
    pub(crate) fn rate_refresh(&mut self, sim: &mut Sim<'_>, unit: UnitId) -> Option<i16> {
        if let Some(v) = self.movement_rate(sim, unit) {
            return Some(v);
        }
        let record = self.anim_lookup(sim, unit)?;
        let fc = sim.units.get(unit)?.anim.frame_count;
        self.spec_anim_rate(sim, unit, &record, fc).map(|r| r.0)
    }

    /// The AnimData record of the COF name the
    /// composer ([`Pending::anim_name`]) builds for the unit's type,
    /// class and mode; a name the file lacks gets the
    /// default record. `None` when no table is loaded, the unit has
    /// no record, or the composer gives no name.
    fn anim_lookup(
        &mut self,
        sim: &Sim<'_>,
        unit: UnitId,
    ) -> Option<d2_formats::animdata::AnimRecord> {
        let data = self.anim_data.clone()?;
        let (ty, class, mode) = self.draw_identity(sim, unit)?;
        let name = self.x.anim_name(unit, ty, class, mode)?;
        match data.record(&name) {
            Ok(rec) => Some(rec.clone()),
            Err(e) => {
                self.errors.push(WiringError::AnimData(e));
                None
            }
        }
    }
}

/// State flag group 38 `meleeonly`.
const MELEE_ONLY: usize = 38;
/// Item type 45 `weap`.
const WEAP_TYPE: i32 = 45;
/// Stat 68 `attackrate`.
const ATTACK_RATE: u16 = 68;
/// State 101 `holyshield`.
const HOLY_SHIELD: u32 = 101;

/// The fields the mode code reads from an AnimData record: frames,
/// byte +0x0F (the speed's high byte, read as event index −1 by the
/// variants) and the 144 event bytes.
pub fn anim_record(r: &d2_formats::animdata::AnimRecord) -> AnimRecord {
    let mut events = [0u8; ANIM_EVENTS];
    events.copy_from_slice(&r.events[..ANIM_EVENTS]);
    AnimRecord {
        frames: r.frames,
        byte_0f: (r.speed >> 24) as u8,
        events,
    }
}

impl<X: Pending> UnitHooks for ActionHooks<X> {
    /// Timer event 11: the pet
    /// refresh sends the hirelings' ally info (S→C 0x7F)
    /// to the owner's client, throttled:
    /// the life percent must differ from the one last sent, by more than
    /// 8 points or on the 7th unsent change in a row. (The party refresh
    /// sends nothing without a party; summoned pets with
    /// `partysend` are not refreshed yet.)
    fn player_refresh(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let mercs: Vec<UnitId> = self
            .hireling_ai
            .ids
            .iter()
            .filter(|(_, &(owner, _))| owner == unit)
            .map(|(&m, _)| m)
            .collect();
        for m in mercs {
            let max = sim.stats.max_life(m) >> 8;
            let life = if max <= 0 {
                0
            } else {
                ((sim.stats.unit_total(m, 6, 0) >> 8).wrapping_mul(100) / max) as u8
            };
            let life = i32::from(life);
            let Some(guid) = sim.units.get(m).map(|r| r.guid) else {
                continue;
            };
            let level = sim
                .game
                .lists
                .unit(m)
                .and_then(|e| e.room())
                .and_then(|r| self.drlg.level_id(sim.game, r))
                .unwrap_or(0) as u16;
            let send = self
                .with_monster_world(|w, _| {
                    let d = w.monster_mut(m)?;
                    let diff = life - d.ally_life;
                    if diff == 0 {
                        return Some(false);
                    }
                    if diff.abs() > 8 {
                        d.ally_count = 0;
                        d.ally_life = life;
                        return Some(true);
                    }
                    d.ally_count += 1;
                    if d.ally_count > 6 {
                        d.ally_count = 0;
                        d.ally_life = life;
                        return Some(true);
                    }
                    Some(false)
                })
                .flatten()
                .unwrap_or(false);
            if send {
                let mut msg = [0u8; 10];
                msg[0] = 0x7F;
                msg[2..4].copy_from_slice(&(life as u16).to_le_bytes());
                msg[4..8].copy_from_slice(&guid.to_le_bytes());
                msg[8..10].copy_from_slice(&level.to_le_bytes());
                self.x.send(unit, &msg);
            }
        }
    }

    /// Monster death by regeneration: the
    /// kill with the poison / open-wounds owner, then the
    /// death events: killed (10) on the unit, then kill (9)
    /// on the killer, with no damage record.
    fn monster_death(&mut self, sim: &mut Sim<'_>, unit: UnitId, killer: Option<UnitId>) {
        use crate::combat::{EV_KILL, EV_KILLED};
        let mut v = View::of(sim.units, sim.stats, sim.data, self);
        let mut cv = v.combat(sim.game);
        super::reaction::kill_by(&mut cv, unit, killer);
        cv.fire_unit_event(EV_KILLED, Some(unit), killer, None);
        if let Some(k) = killer {
            cv.fire_unit_event(EV_KILL, Some(k), Some(unit), None);
        }
    }

    /// Runs the queued remove callbacks of the lists the expiry walk
    /// freed.
    // The shrine callbacks: the first runs the
    // default callback, then a skill refresh and pet resync
    // (which repeats the default's own tail); the second runs the default
    // then sets stamina to the unit's max stamina.
    fn lists_expired(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        use crate::skills::use_::bodies::callback::{
            BLADE_FURY, COLD, DEFAULT, FREEZE_END, INFERNO,
        };
        use crate::skills::use_::bodies::helpers::FLAG_40;
        use crate::world::objects::shrines::{SKILL_REMOVE, STAMINA_REMOVE};
        // The frees of the walk raised these inside their stat writes.
        for (u, stat, layer) in std::mem::take(&mut self.skill_refresh) {
            X::skill_stat_refresh(self, sim, u, stat, layer);
        }
        for (u, state, cb, list) in std::mem::take(&mut self.removed_lists) {
            if ALIGNMENT_CALLBACKS.contains(&cb) {
                let ev = super::SkillEvent::ListRemoved {
                    unit: u,
                    state,
                    cb,
                    list,
                };
                X::skill_event(self, sim, ev);
                continue;
            }
            if cb == FREEZE_END {
                self.freeze_end(sim, u, state);
                continue;
            }
            let t = sim.stats.toggle_state(u, state, false);
            // "State off" is the toggle with the update-queue insert, so
            // the client pass sends S→C 0xA9. Recorded: the
            // `manapot` list freed at full mana (no mana change that tick)
            // still gives 0xA9 106 the next frame
            // (`facts/items/a1-town-potions-low.tsv` n 35).
            if let Err(e) = sim.game.lists.queue_update(u) {
                self.errors
                    .push(WiringError::Unit(crate::units::modes::UnitError::Game(
                        e.into(),
                    )));
            }
            if let (Some(d), Some(r)) = (t.disguise, sim.units.get_mut(u)) {
                if d {
                    r.flags2 |= flags2::DISGUISE;
                } else {
                    r.flags2 &= !flags2::DISGUISE;
                }
            }
            // The default callback's anim refresh: the
            // rate stats of the freed list (cold: 67-69) leave the
            // unit's speed and velocity (gen-mon-295 frame 71).
            if cb == COLD
                && sim.units.get(u).is_some_and(|r| {
                    let dead = if r.ty == UnitType::Player { 17 } else { 12 };
                    r.mode != 0 && r.mode != dead
                })
            {
                // A living unit's shatter (107) goes off.
                sim.stats.toggle_state(u, 107, false);
            }
            if matches!(cb, DEFAULT | COLD | SKILL_REMOVE | STAMINA_REMOVE) {
                let speed = self.rate_refresh(sim, u);
                if let (Some(s), Some(r)) = (speed, sim.units.get_mut(u)) {
                    r.anim.speed = s;
                }
                self.monster_mode_velocity(sim, u);
            }
            // After the default, stamina (10) is set on the
            // unit's list to its total max stamina (11), so
            // the shrine's end refills it.
            if cb == STAMINA_REMOVE {
                use crate::stats::stat;
                let m = sim.stats.unit_total(u, stat::MAXSTAMINA, 0);
                sim.stats.unit_set(self, u, stat::STAMINA, m, 0);
            }
            // Unit flags |= 0x40, so
            // the sequence's later do events do not run.
            if matches!(cb, INFERNO | BLADE_FURY) {
                if let Some(r) = sim.units.get_mut(u) {
                    r.flags |= FLAG_40;
                }
            }
        }
        let _ = unit;
    }
    /// The death penalties ([`super::death`]).
    /// Then the client status bit 0x08, softcore too.
    fn player_death(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        self.death_penalties(sim, unit);
        sim.game.lists.set_player_status(unit, STATUS_DEAD);
        // Dead clean-up: the
        // dead-body footprint replaces the player's
        // 0x80 footprint, so missiles no longer see the body.
        View::of(sim.units, sim.stats, sim.data, self).dead_body_footprint(unit);
        // The dead clean-up: base
        // hitpoints := 0, the death list sweep, and the
        // states but the keep mask (`plrstaydeath`) cleared; a poisoned
        // player is told the state ended (recorded `items-drops-hel-02`
        // frame 38: 0xA9 state 2).
        sim.stats.unit_set(&mut *self, unit, 6, 0, 0);
        sim.stats.death(&mut *self, unit);
        sim.stats
            .clear_states_except(unit, crate::stats::states::group::PLR_STAY_DEATH);
        self.death_notice(sim, unit);
    }
    /// The corpse creation ([`super::death`]), then the owner-death
    /// handling of the player's hirelings in every game type:
    /// queued for the host that holds the hireling lists
    /// ([`ActionHooks::owner_deaths`]).
    /// Then the client status bit 0x08.
    fn player_corpse(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        self.corpse_creation(sim, unit);
        // The corpse is made before the mode set
        // queues the player, so the
        // player's update goes out before the corpse's (recorded
        // `items-drops-cha-00` frame 96: 0x0D of the player, then 0x59).
        let _ = sim.game.lists.unqueue_update(unit);
        let _ = sim.game.lists.queue_update(unit);
        if let Some(q) = self.owner_deaths.as_mut() {
            q.push(unit);
        }
        sim.game.lists.set_player_status(unit, STATUS_DEAD);
    }
    /// Picks up a corpse ([`super::death`]; the experience it returns is not
    /// read by the 0x16 caller).
    fn player_corpse_pickup(&mut self, sim: &mut Sim<'_>, player: UnitId, corpse: UnitId) -> bool {
        self.corpse_pickup(sim, player, corpse).is_some()
    }
    /// The AnimData record of the unit's mode.
    fn anim_record(&mut self, sim: &Sim<'_>, unit: UnitId) -> Option<AnimRecord> {
        self.anim_lookup(sim, unit).map(|r| anim_record(&r))
    }

    /// The sequence load for a player in mode 18: the used skill's
    /// `seqnum` and the
    /// unit's COF weapon class ([`Pending::composit_weapon_class`], the
    /// class the animation names use) pick the frame list; its event bytes,
    /// length · 256 and speed 256. A monster in mode 14: its class's slot
    /// of the used skill picks the `monseq` list
    /// ([`ActionHooks::monster_sequences`]). `None`
    /// (the plain animation): no used skill, `seqnum` 0, a null list.
    fn load_sequence(&mut self, sim: &Sim<'_>, unit: UnitId) -> Option<Sequence> {
        let rec = sim.units.get(unit)?;
        if rec.ty == UnitType::Monster {
            let used = self.used_skill_of(unit)?;
            let class = usize::try_from(rec.class).ok()?;
            let frames = self.monster_sequences.as_ref()?.lookup(class, used.skill)?;
            return Some(Sequence {
                frame_count: (frames.len() as i32) * 256,
                speed: 256,
                pos: 0,
                events: frames.iter().map(|f| f.event).collect(),
                drawn: frames.iter().map(|f| f.frame).collect(),
            });
        }
        if rec.ty != UnitType::Player {
            return None;
        }
        let used = self.used_skill_of(unit)?;
        let row = self
            .tables
            .skills
            .skills
            .get(usize::try_from(used.skill).ok()?)?;
        let class = usize::try_from(self.x.composit_weapon_class(unit)).ok()?;
        let frames = crate::skills::sequences::lookup(row.seqnum, class)?;
        Some(Sequence {
            frame_count: (frames.len() as i32) * 256,
            speed: 256,
            pos: 0,
            events: frames.iter().map(|f| f.event).collect(),
            drawn: frames.iter().map(|f| f.frame).collect(),
        })
    }

    /// The animation re-init of a mode change, for players and monsters:
    /// action frame := 0, frame := frame bonus · 256; in the sequence mode
    /// (player 18, monster 14) the sequence load then the rate;
    /// otherwise sequence := none, the rate with its
    /// velocity half, then frame count := the record's frames
    /// · 256. The run list of player modes 3 / 19 is
    /// attached by the run start. Objects run their
    /// own branch ([`crate::world::objects`]), items the one below, the
    /// rest nothing here.
    fn reinit_anim(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let Some(ty) = sim.units.get(unit).map(|r| r.ty) else {
            return;
        };
        if ty == UnitType::Item {
            // The item branch: a ground item (mode 3)
            // rests on frame 0x1000 of 0x1100 with speed 0; a dropping
            // item (mode 5) starts at frame 0 with speed 0x100.
            if let Some(r) = sim.units.get_mut(unit) {
                let (frame, speed) = match r.mode {
                    3 => (0x1000, 0),
                    5 => (0, 0x100),
                    _ => return,
                };
                r.anim.frame = frame;
                r.anim.frame_count = 0x1100;
                r.anim.speed = speed;
            }
            return;
        }
        if !matches!(ty, UnitType::Player | UnitType::Monster) {
            return;
        }
        let bonus = self.frame_bonus_in(sim.units, sim.stats, unit);
        let sequence_mode = sim.units.get(unit).is_some_and(|r| match ty {
            UnitType::Player => r.mode == crate::units::modes::player_mode::SEQUENCE,
            _ => r.mode == crate::units::modes::monster_mode::SEQUENCE,
        });
        let sequence = if sequence_mode {
            UnitHooks::load_sequence(self, sim, unit)
        } else {
            None
        };
        // Every player and monster in a game has a path; the rate of a
        // unit without one (unit tests) is left as it was.
        let rate = self.path_has(unit).then(|| match &sequence {
            Some(seq) => UnitHooks::sequence_rate(self, sim, unit, seq.frame_count),
            None => (UnitHooks::anim_rate(self, sim, unit), None),
        });
        if rate.is_some() {
            UnitHooks::anim_velocity(self, sim, unit);
        }
        let record = UnitHooks::anim_record(self, sim, unit);
        let Some(r) = sim.units.get_mut(unit) else {
            return;
        };
        let a = &mut r.anim;
        a.action_frame = 0;
        a.frame = bonus.wrapping_mul(256);
        a.record = record;
        if let Some((speed, _)) = rate {
            a.speed = speed;
        }
        match sequence {
            Some(mut seq) => {
                // The load's speed step, as in the prepare step.
                if let Some((_, Some(v))) = rate {
                    seq.speed = v;
                }
                a.frame_count = seq.frame_count;
                if let Some(&d) = seq.drawn.first() {
                    a.frame = i32::from(d) << 8;
                }
                a.action_frame = seq.events.first().copied().unwrap_or(0);
                a.sequence = Some(seq);
            }
            None => {
                a.sequence = None;
                if let Some(rec) = record {
                    a.frame_count = (rec.frames as i32).wrapping_mul(256);
                }
            }
        }
    }

    /// The animation rate from the record's speed: the knockback and
    /// velocity-mode parts here, with the
    /// path provider (`ActionHooks::movement_rate`); every other part
    /// is the host's ([`Pending::anim_rate`]).
    fn anim_rate(&mut self, sim: &mut Sim<'_>, unit: UnitId) -> i16 {
        if let Some(v) = self.movement_rate(sim, unit) {
            return v;
        }
        let record = self.anim_lookup(sim, unit);
        // The mode start stores the new record and its frame count before
        // the rate reads the frame count.
        if let Some(v) = record.as_ref().and_then(|r| {
            let fc = (r.frames as i32).wrapping_mul(256);
            self.spec_anim_rate(sim, unit, r, fc)
        }) {
            return v.0;
        }
        self.x.anim_rate(unit, record.map(|r| r.speed))
    }

    /// [`UnitHooks::anim_rate`] for a sequence:
    /// the same rate with the frame count = the sequence's count · 256, and
    /// the sequence speed the cast and attack cases write.
    fn sequence_rate(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        frame_count: i32,
    ) -> (i16, Option<i32>) {
        if let Some(v) = self.movement_rate(sim, unit) {
            return (v, None);
        }
        let record = self.anim_lookup(sim, unit);
        if let Some(v) = record
            .as_ref()
            .and_then(|r| self.spec_anim_rate(sim, unit, r, frame_count))
        {
            return v;
        }
        (self.x.anim_rate(unit, record.map(|r| r.speed)), None)
    }

    /// The velocity half of the animation rate for monsters with the path
    /// provider ([`crate::wiring::path::monsters`]).
    fn anim_velocity(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        self.monster_mode_velocity(sim, unit);
    }

    /// The path part of the monster mode request ([`crate::wiring::path::monsters`]);
    /// the requested mode is kept for the start function.
    fn monster_mode_bookkeeping(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u32) {
        self.monster_request = mode;
        self.monster_path_setup(sim, unit, mode);
    }

    /// The live unit's footprint stamped at
    /// its path position, every mode request.
    fn monster_mode_footprint(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        if sim.units.get(unit).map(|r| r.ty) != Some(UnitType::Monster) {
            return;
        }
        View::of(sim.units, sim.stats, sim.data, self).path_add_footprint(unit);
    }

    /// Leaves the previous monster mode (`Pending::leave_monster_mode`), every mode request.
    fn monster_leave_mode(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let left = sim.units.get(unit).map(|r| r.mode);
        if let Some(m) = left {
            self.leave_monster_mode(unit, m);
        }
    }

    /// The monster mode's damage step ([`Pending::monster_mode_damage`]).
    fn monster_mode_damage(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u32) {
        X::monster_mode_damage(self, sim, unit, mode);
    }

    /// The frame bonus.
    fn frame_bonus(&mut self, sim: &Sim<'_>, unit: UnitId) -> i32 {
        self.frame_bonus_in(sim.units, sim.stats, unit)
    }

    fn has_path(&mut self, _: &Sim<'_>, unit: UnitId) -> bool {
        self.path_has(unit)
    }

    /// The provider's static path record.
    fn static_position(&self, unit: UnitId) -> Option<(i32, i32)> {
        match self.paths.as_ref()?.record(unit)? {
            crate::path::UnitPath::Static(s) => Some((s.x, s.y)),
            crate::path::UnitPath::Dynamic(_) => None,
        }
    }

    /// Player event 0 in modes 2, 3, 6, 19: the player step
    /// with the path provider; the step result (2:
    /// stopped, the ENDANIM handler follows). Without the provider: the
    /// trait default (1, nothing moves).
    fn player_movement_step(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) -> u32 {
        let _ = (a1, a2);
        if self.paths.is_none() {
            return 1;
        }
        let mut v = View::of(sim.units, sim.stats, sim.data, self);
        crate::wiring::path::walk::player_step(&mut v, sim.game, unit)
    }

    /// Player event 0 in attack, cast and skill modes,
    /// through [`Pending::action_frame`].
    fn player_action_frame(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) -> u32 {
        X::action_frame(self, sim, unit, a1, a2)
    }

    /// Monster mode functions: the start and event
    /// functions ([`crate::wiring::path::monsters`]); the
    /// death start goes to [`Pending::monster_death_start`]
    /// with the mode change's target and the death clean-up
    /// ([`super::monster_death`]), the DD start runs the same
    /// there; DT's event functions
    /// end the death in mode 12
    /// ([`super::unit_update::death_function`]); every other
    /// function keeps the default (started, nothing done).
    fn monster_mode_function(&mut self, sim: &mut Sim<'_>, unit: UnitId, address: u32) -> bool {
        if let Some(started) = self.monster_motion_function(sim, unit, address) {
            return started;
        }
        if address == MONSTER_MODES[0].start {
            let target = self.mode_target;
            return self.monster_death(sim, unit, target);
        }
        if address == MONSTER_MODES[12].start {
            self.monster_dead_start(sim, unit);
            return true;
        }
        super::unit_update::death_function(self, sim, unit, address);
        true
    }

    /// The unit's own entries leave its combat list
    /// (entries are (attacker, defender) records).
    fn drop_combat_entries(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let Some(e) = sim.game.lists.unit(unit) else {
            return;
        };
        let id = (e.ty, e.guid);
        if let Some(list) = self.combat_lists.get_mut(&unit) {
            list.retain(|c| c.attacker != id);
        }
    }

    /// Stops the unit's dynamic path record
    /// (`crate::wiring::path::monsters::stop_path`).
    fn stop_path_now(&mut self, unit: UnitId) -> bool {
        crate::wiring::path::monsters::stop_path(self, unit).is_some()
    }

    /// AI param 0 of the unit's AI control;
    /// false while the store is lent or the unit has no AI.
    fn set_ai_param0(&mut self, unit: UnitId, v: i32) -> bool {
        self.ai
            .as_mut()
            .and_then(|a| a.control_mut(unit))
            .map(|c| c.params[0] = v)
            .is_some()
    }

    /// Sets AI param `index + 1` on the unit's AI control.
    fn set_ai_param(&mut self, unit: UnitId, index: usize, v: i32) -> bool {
        self.ai
            .as_mut()
            .and_then(|a| a.control_mut(unit))
            .and_then(|c| c.params.get_mut(index))
            .map(|p| *p = v)
            .is_some()
    }

    /// The room flag of the unit's room.
    fn room_flag(&mut self, sim: &Sim<'_>, unit: UnitId) -> bool {
        sim.game
            .lists
            .unit(unit)
            .and_then(|e| e.room())
            .is_some_and(|r| self.drlg.in_town(sim.game, r))
    }

    /// Before a think is scheduled on a monster with
    /// state 54: state 54 off, the
    /// monster's type-2 events cancelled.
    fn uninterruptable_check(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let mut v = View::of(sim.units, sim.stats, sim.data, self);
        v.set_state(unit, state::UNINTERRUPTABLE as u16, false);
        sim.game
            .timers
            .cancel_unit_events(unit, event::AI_THINK, None);
    }

    /// Event 2, the AI think. The freeze drop already ran in the unit
    /// dispatch.
    fn ai_think(&mut self, sim: &mut Sim<'_>, unit: UnitId, _: u32, _: u32) {
        let Some(mut store) = self.ai.take() else {
            self.errors.push(WiringError::Reentrant("ai"));
            return;
        };
        let t = self.tables.clone();
        let info = self.ai_info;
        {
            let mut v = View::of(sim.units, sim.stats, sim.data, self);
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
                world: &mut v,
            };
            ai::think(sim.game, &mut cx, unit);
        }
        self.ai = Some(store);
    }

    /// The mode-set sites of the umod dispatcher
    /// on the lent monster world; without one, nothing.
    fn monster_umods(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u8) {
        self.run_umods(sim, unit, None, mode);
    }

    /// Event 7, the umod dispatcher in mode 2,
    /// on the lent monster world ([`super::monsters`]); without one,
    /// nothing (the trait default). The unit dispatch already ran the
    /// handler-table checks and the frozen-monster drop.
    fn monster_umod(&mut self, sim: &mut Sim<'_>, unit: UnitId, _: u32, _: u32) {
        self.run_umods(sim, unit, None, umod_mode::EVENT7);
    }

    /// Event 10, the AI reset: the monster
    /// data's override kind and GUID := 0, then the host's part.
    fn ai_reset(&mut self, _: &mut Sim<'_>, unit: UnitId, _: u32, _: u32) {
        if let Some(d) = self.monster_data_mut(unit) {
            d.override_kind = 0;
            d.override_guid = 0;
        }
        self.x.ai_reset(unit);
    }

    /// Event 5: the skills'
    /// active-state function, through [`Pending::skill_event`].
    fn active_state(&mut self, sim: &mut Sim<'_>, unit: UnitId, f: u16, skill: u32, a2: u32) {
        X::skill_event(
            self,
            sim,
            SkillEvent::ActiveState {
                unit,
                f,
                skill,
                arg2: a2,
            },
        );
    }

    /// Event 8, the periodic skills, through [`Pending::skill_event`].
    fn periodic_skills(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {
        X::skill_event(
            self,
            sim,
            SkillEvent::Periodic {
                unit,
                arg1: a1,
                arg2: a2,
            },
        );
    }

    /// Event 9, the item aura, after its checks, through
    /// [`Pending::skill_event`].
    fn apply_item_aura(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        a1: u32,
        skill: u32,
        level: i32,
    ) {
        X::skill_event(
            self,
            sim,
            SkillEvent::ItemAura {
                unit,
                arg1: a1,
                skill,
                level,
            },
        );
    }

    /// The 0xAB record {f} on the unit, the unit queued for update.
    fn send_life_fraction(&mut self, sim: &mut Sim<'_>, unit: UnitId, fraction: i32) {
        use super::event_records::EventRecord;
        let life = fraction.clamp(0, 255) as u8;
        self.event_records.push(unit, EventRecord::NpcHeal { life });
        let _ = sim.game.lists.queue_update(unit);
    }

    /// Object events on the object state
    /// ([`super::objects`]); a game without one keeps the default.
    fn object_event(&mut self, sim: &mut Sim<'_>, unit: UnitId, event: u8) {
        View::of(sim.units, sim.stats, sim.data, self).object_event(sim.game, unit, event);
    }

    /// Missile events.
    fn missile_do(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let Some(mut store) = self.missiles.take() else {
            self.errors.push(WiringError::Reentrant("missiles"));
            return;
        };
        let t = self.tables.clone();
        {
            let mut v = View::of(sim.units, sim.stats, sim.data, self);
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut v,
            };
            missiles::class_handler(sim.game, &mut cx, unit);
        }
        self.missiles = Some(store);
    }
}

/// Flags 2 bit 0x10000: the S→C 0x15 of the
/// next update.
const PET_WARP_REASSIGN: u32 = 0x10000;

impl<X: Pending> LifecycleHooks for ActionHooks<X> {
    fn skill_tables(&self) -> Option<&crate::skills::SkillTables> {
        Some(&self.tables.skills)
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    /// The monster branch of [`super::ai`]'s `assign_skill`: the entry
    /// next to the init entries ([`ActionHooks::natural_skills`]) in
    /// [`ActionHooks::monster_skills`], then the passive refresh.
    fn set_monster_skill_level(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) -> bool {
        if sim.units.get(unit).map(|r| r.ty) != Some(UnitType::Monster) {
            return false;
        }
        self.assign_monster_skill(sim, unit, skill, level);
        true
    }
    fn request_act_change(&mut self, player: UnitId, level: u32, arg: u32) {
        self.act_changes.push((player, level, arg));
    }
    fn hold_act_tail(&mut self, tail: ActTail) -> bool {
        let (ActTail::Completion { player, .. } | ActTail::Waypoint { player, .. }) = tail;
        let held = self.act_changes.iter().any(|c| c.0 == player);
        if held {
            self.act_tails.push(tail);
        }
        held
    }
    fn town_room(&self, game: &Game, room: crate::units::RoomId) -> bool {
        self.drlg.in_town(game, room)
    }
    fn room_level(&self, game: &Game, room: crate::units::RoomId) -> Option<u32> {
        self.drlg.level_id(game, room)
    }
    fn path_xy(&self, unit: UnitId) -> Option<(i32, i32)> {
        self.path_has(unit).then(|| self.path_position(unit))
    }
    /// The drop's ground placement: the static path set at
    /// the spot (a dynamic path is not an item's), then the
    /// footprint stamped there (box of size 1, mask 0x200). Freed by
    /// [`Self::ground_item_lifted`].
    fn ground_item_placed(&mut self, item: UnitId, room: crate::units::RoomId, x: i32, y: i32) {
        let Some(p) = self.paths.as_mut() else {
            return;
        };
        match p.records.get_mut(&item) {
            Some(crate::path::record::UnitPath::Static(s)) => s.set(Some(room), x, y),
            Some(_) => {}
            None => {
                let mut s = crate::path::record::StaticPath::default();
                s.set(Some(room), x, y);
                p.records
                    .insert(item, crate::path::record::UnitPath::Static(s));
            }
        }
        crate::path::collision::box_apply(
            &mut self.drlg,
            Some(room),
            x,
            y,
            (1, 1),
            crate::drlg::collision::bits::ITEM,
            true,
        );
    }
    /// The collision free of a ground item leaving the
    /// ground (the pickups, the gold pile's
    /// take): the size-1, mask-0x200 box
    /// cleared at its static path. The general unit free does not clear
    /// an item's (it covers types 0–3 only).
    fn ground_item_lifted(&mut self, item: UnitId) {
        let Some(p) = self.paths.as_ref().and_then(|p| p.record(item)) else {
            return;
        };
        let crate::path::record::UnitPath::Static(_) = p else {
            return;
        };
        let (room, (x, y)) = (p.room(), p.position());
        crate::path::collision::box_apply(
            &mut self.drlg,
            room,
            x,
            y,
            (1, 1),
            crate::drlg::collision::bits::ITEM,
            false,
        );
    }
    /// [`View::town_portal_cast`] with the call's game seed lent to the
    /// hooks; no object state: `None`.
    fn town_portal_cast(
        &mut self,
        sim: &mut Sim<'_>,
        seed: &mut crate::rng::Seed,
        player: UnitId,
    ) -> Option<(u32, bool)> {
        self.objects.as_ref()?;
        self.game_seed = *seed;
        let r = View::of(sim.units, sim.stats, sim.data, self).town_portal_cast(sim.game, player);
        *seed = self.game_seed;
        Some(r)
    }
    /// The monster type init
    /// (the allocator's per-kind init of a monster) on the lent
    /// monster world ([`super::monsters`]); the object data and init
    /// of an object on the object state
    /// ([`View::object_init`]); other kinds, and a game
    /// without the lent world or the object state, keep the default
    /// (nothing).
    fn init_kind(&mut self, sim: &mut Sim<'_>, unit: UnitId, req: &AllocRequest) {
        // The init's room is the allocation's.
        self.alloc_rooms.push((unit, req.room));
        if req.ty == UnitType::Player {
            // Player type init: unit flags |= 0x0E first, so the missile
            // target filter accepts the player.
            if let Some(r) = sim.units.get_mut(unit) {
                r.flags |= 0x0E;
            }
            // Then, unless mode 17, the player (node index 11, no list)
            // heads the first free player target list and its node index
            // := that list's code, so its merc and summons can join after
            // it.
            if req.mode != 17 {
                let taken = |slot: u32| {
                    sim.units.iter().any(|(id, r)| {
                        id != unit && r.ty == UnitType::Player && r.node_index == slot
                    })
                };
                // The first free player list; a game has as many as its
                // players (the first eight are codes 0-7).
                let free = (0..).map(crate::game::node::player).find(|&s| !taken(s));
                if let (Some(slot), Some(r)) = (free, sim.units.get_mut(unit)) {
                    if r.node_index == crate::units::record::INITIAL_NODE_INDEX {
                        r.node_index = slot;
                    }
                }
            }
        }
        if req.ty == UnitType::Monster {
            self.with_monster_world(|w, h| w.type_init(sim, h, unit));
        } else if req.ty == UnitType::Object {
            // Inside `View::allocate`: run once its seed step is back in
            // the hooks (the init may allocate and draw itself).
            if let Some(q) = self.deferred_inits.as_mut() {
                q.push(unit);
                return;
            }
            View::of(sim.units, sim.stats, sim.data, self).object_init(sim.game, unit);
        }
    }

    /// The mercenary's creation: the placement
    /// and creation of the population code around `near`'s path position
    /// in its room (spread 4: rings 3 … 12 on the active-room seed),
    /// with the call's game seed lent to the hooks for the allocation's
    /// unit-seed step. `None` when nothing was placed.
    /// Without the lent monster world: a plain allocation at (+2, +2)
    /// from the point (D2RUST's own, unverified: hosts with no population
    /// state).
    fn spawn_near(
        &mut self,
        sim: &mut Sim<'_>,
        seed: &mut Seed,
        near: UnitId,
        class: u32,
        mode: u8,
    ) -> Option<UnitId> {
        self.spawn_near_spread(sim, seed, near, class, mode, HIRE_SPREAD)
    }
    fn spawn_near_spread(
        &mut self,
        sim: &mut Sim<'_>,
        seed: &mut Seed,
        near: UnitId,
        class: u32,
        mode: u8,
        spread: i32,
    ) -> Option<UnitId> {
        let room = sim.game.lists.unit(near)?.room()?;
        let (x, y) = self.path_position(near);
        self.game_seed = *seed;
        let placed = self
            .with_monster_world(|w, h| {
                w.spawn_at(sim, h, room, x, y, class as i32, mode, spread, 0)
            })
            .flatten();
        let u = match placed {
            Some(placed) => placed,
            None => {
                let req = AllocRequest {
                    ty: UnitType::Monster,
                    class,
                    room: Some(room),
                    add: true,
                    fixed_guid: None,
                    mode: u32::from(mode),
                    allied: false,
                };
                View::of(sim.units, sim.stats, sim.data, self).allocate(
                    sim.game,
                    &req,
                    x + 2,
                    y + 2,
                )
            }
        };
        *seed = self.game_seed;
        u
    }

    /// The minion owner of the unit's AI control record: a unit without AI
    /// control keeps none. The lookup
    /// is by GUID at every use, so an
    /// owner GUID that names no unit (−1) drops the link.
    fn set_ai_owner(&mut self, unit: UnitId, owner_type: u8, owner_guid: u32) {
        let Some(ty) = UnitType::ALL.get(usize::from(owner_type)).copied() else {
            return;
        };
        if ty == UnitType::Player {
            self.hireling_units.insert(unit);
        }
        if let Some(c) = self.ai.as_mut().and_then(|s| s.control_mut(unit)) {
            c.minion_owner = Some(ai::UnitRef {
                ty,
                guid: owner_guid,
            });
        }
    }

    fn queue_removal_notice(&mut self, unit_type: u8, guid: u32) {
        self.removal_notices.push((unit_type, guid));
    }

    fn queue_unit_stat(&mut self, unit: UnitId, stat: u16, value: u32) {
        self.event_records.push(
            unit,
            super::event_records::EventRecord::UnitStat { stat, value },
        );
    }

    /// On the lent monster world (none: nothing).
    fn assign_umod(&mut self, sim: &mut Sim<'_>, unit: UnitId, umod: u8) {
        self.with_monster_world(|w, h| w.assign_umod(sim, h, unit, umod));
    }

    fn set_alignment(&mut self, sim: &mut Sim<'_>, unit: UnitId, value: u8) {
        View::of(sim.units, sim.stats, sim.data, self).set_alignment(sim.game, unit, value);
    }

    fn clear_hireling_components(&mut self, unit: UnitId) {
        self.with_monster_world(|w, _| {
            if let Some(m) = w.monster_mut(unit) {
                for i in [0, 1, 5, 6, 7] {
                    m.components[i] = 0;
                }
            }
        });
    }

    /// Warps a hireling pet: the pet placed at the
    /// player's room and point (the path provider's
    /// teleport, which also leaves the old room's list), queued for
    /// update with flags 2 |= 0x10000 (the reassign of the next update),
    /// then the pet's AI is
    /// re-attached. No player
    /// room or no path provider: nothing.
    // TODO: the path reset "when not moving" is not run; it is not clear
    // what "moving" means here.
    fn warp_pet(&mut self, sim: &mut Sim<'_>, pet: UnitId, player: UnitId) {
        let Some(room) = sim.game.lists.unit(player).and_then(|e| e.room()) else {
            return;
        };
        let (x, y) = self.path_position(player);
        let old = sim.game.lists.unit(pet).and_then(|e| e.room());
        let placed = View::of(sim.units, sim.stats, sim.data, self).path_teleport_to(
            sim.game,
            pet,
            Some(room),
            x,
            y,
        );
        if placed.is_none() {
            return;
        }
        if old != Some(room) {
            View::of(sim.units, sim.stats, sim.data, self).path_pet_room_change(sim.game, pet, old);
        }
        if let Err(e) = sim.game.lists.queue_update(pet) {
            self.errors
                .push(WiringError::Unit(UnitError::Game(e.into())));
        }
        if let Some(r) = sim.units.get_mut(pet) {
            r.flags2 |= PET_WARP_REASSIGN;
        }
        let Some(mut store) = self.ai.take() else {
            return;
        };
        let t = self.tables.clone();
        let info = self.ai_info;
        {
            let mut v = View::of(sim.units, sim.stats, sim.data, self);
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
                world: &mut v,
            };
            ai::update_ai_callback(sim.game, &mut cx, pet);
        }
        self.ai = Some(store);
    }

    /// The unit was added to its list: its room is the list's from now on.
    fn added(&mut self, _: &mut Sim<'_>, unit: UnitId) {
        self.alloc_rooms.retain(|&(u, _)| u != unit);
    }

    /// The per-kind state of the action modules leaves with the unit:
    /// AI control (`AiStore::remove`), a summon's skill entries, missile
    /// data, combat list; then
    /// the lent monster world's part (monster data, minion list, owner
    /// link); an object's object data.
    fn room_delete_record(&mut self, game: &mut Game, guid: u32, room: crate::units::RoomId) {
        // One record per unit: a pickup records it at the room removal,
        // the later free finds it there.
        if !self.removed_items.contains(&(guid, room)) {
            self.removed_items.push((guid, room));
        }
        let act = game.lists.room(room).map(|r| r.act);
        if let Some(a) = act.and_then(|a| game.lists.act_mut(a)) {
            a.pending_removals = true;
        }
    }

    fn free_kind(&mut self, sim: &mut Sim<'_>, unit: UnitId) {
        let (ty, class, mode) = sim
            .units
            .get(unit)
            .map_or((None, 0, 0), |r| (Some(r.ty), r.class, r.mode));
        // A ground item leaves the clients' rooms: its removal record
        // (`ActionHooks::removed_items`), in the room its path
        // was in.
        if ty == Some(UnitType::Item) && mode == u32::from(crate::items::moves::mode::GROUND) {
            let room = self
                .paths
                .as_ref()
                .and_then(|p| p.record(unit))
                .and_then(|r| r.room())
                .or_else(|| sim.game.lists.unit(unit).and_then(|e| e.room()));
            let guid = sim.units.get(unit).map(|r| r.guid);
            if let (Some(room), Some(guid)) = (room, guid) {
                self.room_delete_record(sim.game, guid, room);
            }
        }
        self.path_free(unit, ty, class, mode);
        // The monster's inventory goes with it.
        if ty == Some(UnitType::Monster) {
            crate::wiring::economy::free_monster_inventory(self, sim, unit, class);
        }
        self.monster_skills.remove(&unit);
        self.natural_skills.remove(&unit);
        self.unit_source.remove(&unit);
        // A summon's skill list (the aura assignment of
        // `interaction::summon`); players keep theirs (the save).
        if ty == Some(UnitType::Monster) {
            self.skill_lists.remove(&unit);
        }
        if let Some(ai) = self.ai.as_mut() {
            ai.remove(unit);
        }
        if let Some(m) = self.missiles.as_mut() {
            m.remove(unit);
        }
        self.combat_lists.remove(&unit);
        self.handlers.remove(&unit);
        if let Some(st) = self.objects.as_mut() {
            st.control.data.remove(&unit);
        }
        self.with_monster_world(|w, _| w.forget(unit));
    }
}

impl<X: Pending> View<'_, X> {
    /// Records an error of a unit operation.
    pub fn unit_error(&mut self, e: UnitError) {
        self.h.errors.push(WiringError::Unit(e));
    }

    /// The unit seed. A unit without a record gets a scratch
    pub fn seed(&mut self, u: UnitId) -> &mut Seed {
        if self.units.get(u).is_none() {
            self.h
                .errors
                .push(WiringError::Unit(UnitError::UnknownUnit(u)));
            self.h.orphan_seed = Seed::init();
            return &mut self.h.orphan_seed;
        }
        &mut self.units.get_mut(u).expect("checked").seed
    }

    /// Unit stat getter (layer 0).
    pub fn stat(&self, u: UnitId, s: u16) -> i32 {
        self.stats.unit_total(u, s, 0)
    }

    /// Unit base stat setter (layer 0).
    pub fn set_base(&mut self, u: UnitId, s: u16, value: i32) {
        self.stats.unit_set(&mut *self.h, u, s, value, 0);
    }

    /// Set a stat of a list (layer 0).
    pub fn set_list_stat(&mut self, l: ListId, s: u16, value: i32) {
        self.stats.set(&mut *self.h, l, s, value, 0, None);
    }

    /// State toggle with the disguise
    /// bit of the unit's flag-ex.
    pub fn set_state(&mut self, u: UnitId, s: u16, on: bool) {
        let t = self.stats.toggle_state(u, u32::from(s), on);
        if let (Some(d), Some(r)) = (t.disguise, self.units.get_mut(u)) {
            if d {
                r.flags2 |= flags2::DISGUISE;
            } else {
                r.flags2 &= !flags2::DISGUISE;
            }
        }
    }

    /// The unit's stat list of `state`.
    pub fn state_list(&self, u: UnitId, s: u16) -> Option<ListId> {
        let r = self.stats.unit_list(u)?;
        self.stats.list_of_state(r, u32::from(s))
    }

    /// `stat` of the unit's list of `state` (its own base value).
    pub fn state_stat(&self, u: UnitId, s: u16, st: u16) -> Option<i32> {
        let l = self.state_list(u, s)?;
        Some(self.stats.base(l, st, 0))
    }

    /// A plain stat list for `state` attached to the unit: allocated with
    /// the owner's type and GUID, expire set (NEWLENGTH when > 0), state
    /// field, attached.
    ///
    /// TODO: the callers' allocation flags and the
    /// attach `reset` argument are not known for state lists; flags 0
    /// and reset = 1 (no DYNAMIC) are used. Without an owner the unit's
    /// own type and GUID are used.
    pub fn create_state_list(
        &mut self,
        u: UnitId,
        s: u16,
        owner: Option<(UnitType, u32)>,
        expire: i32,
    ) -> Option<ListId> {
        let (ty, guid) = match owner {
            Some(o) => o,
            None => {
                let r = self.units.get(u)?;
                (r.ty, r.guid)
            }
        };
        let l = self.stats.alloc(0, 0, ty.index() as u32, guid);
        self.stats.set_expire(l, expire);
        self.stats.set_state(l, u32::from(s));
        self.stats.attach(&mut *self.h, u, l, true);
        Some(l)
    }

    /// Sets the unit's alignment: stat 172 (`alignment`) := `a` in the
    /// unit's state-105 (`alignment`) list, allocated with flags 0 when
    /// missing, the state on, its state-changed bit
    /// set again (the "resend") and the unit queued for update. The
    /// game's allied mark follows a good (2) alignment. `a` > 2 is an
    /// assertion failure in the game: nothing here.
    ///
    /// Gate: all of it runs only for classes 351–353,
    /// or `a` ≠ 0, or unit flag bit 31 clear. The tail: for a
    /// monster whose old alignment (the cleared stat 172, or 4 when the
    /// list is new) differs from `a`, standing in a room and not in mode
    /// 0 (DT) or 12 (DD), the path is reset. A new monster's first set
    /// therefore resets its pattern (a wraith's 5 becomes its size
    /// pattern).
    pub fn set_alignment(&mut self, game: &mut Game, u: UnitId, a: u8) {
        const STATE: u16 = crate::combat::range::STATE_ALIGNMENT;
        const STAT: u16 = crate::combat::range::STAT_ALIGNMENT;
        if a > 2 {
            return;
        }
        let Some(r) = self.units.get(u) else {
            return;
        };
        let (ty, mode) = (r.ty, r.mode);
        if !((351..=353).contains(&r.class) || a != 0 || r.flags & 0x8000_0000 == 0) {
            return;
        }
        let (l, old) = match self.state_list(u, STATE) {
            Some(l) => (l, self.stats.base(l, STAT, 0)),
            None => match self.create_state_list(u, STATE, None, 0) {
                Some(l) => (l, 4),
                None => return,
            },
        };
        self.set_list_stat(l, STAT, i32::from(a));
        self.set_state(u, STATE, true);
        self.stats.set_state_changed(u, u32::from(STATE), true);
        game.lists.set_allied(u, a == 2);
        let _ = game.lists.queue_update(u);
        let in_room = self
            .h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(u))
            .is_some_and(|d| d.room.is_some());
        if ty == UnitType::Monster && old != i32::from(a) && in_room && mode != 0 && mode != 12 {
            self.path_reset(u);
        }
    }

    /// Hireling test: a monster of a hireling class.
    pub fn is_hireling(&self, game: &Game, u: UnitId) -> bool {
        game.lists
            .unit(u)
            .is_some_and(|e| e.ty == UnitType::Monster)
            && self
                .units
                .get(u)
                .is_some_and(|r| HIRELING_CLASSES.contains(&r.class))
    }

    /// Unit allocation on the game seed:
    /// the allocator's steps with the per-kind init (an object's after the
    /// seed step is written back), then the add: the list part and the
    /// path part ([`View::path_place`]; without
    /// the path provider [`Pending::place`]). An object allocated from
    /// inside an object call is left unlinked: its caller runs the init
    /// (`objects::allocate`) and then [`View::add_allocated`].
    pub fn allocate(
        &mut self,
        game: &mut Game,
        req: &AllocRequest,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        let u = self.allocate_unadded(game, req)?;
        if req.ty == UnitType::Object && self.h.objects_out {
            return Some(u);
        }
        self.add_allocated(game, u, req, x, y).then_some(u)
    }

    /// The player's unit seed on the game seed:
    /// what the character load runs on the player
    /// it allocated, before anything else of the load draws. `false`: no
    /// such unit (no draw).
    pub fn init_player_seed(&mut self, u: UnitId) -> bool {
        crate::units::lifecycle::init_player_seed(self.units, u, &mut self.h.game_seed)
    }

    /// [`View::allocate`] up to the kind init: the
    /// unit is not added yet. Its caller runs what the allocator runs
    /// before the add and then [`View::add_allocated`].
    pub fn allocate_unadded(&mut self, game: &mut Game, req: &AllocRequest) -> Option<UnitId> {
        let mut seed = self.h.game_seed;
        let outer = self.h.deferred_inits.replace(Vec::new());
        let r = {
            let mut sim = Sim {
                game,
                units: self.units,
                stats: self.stats,
                data: self.data,
            };
            crate::units::lifecycle::allocate_unlinked(&mut sim, &mut *self.h, &mut seed, req)
        };
        self.h.game_seed = seed;
        // The object init is the allocation's last step before the add
        // (after the unit's seed step): run it now that the step is in
        // the hooks.
        let inits = std::mem::replace(&mut self.h.deferred_inits, outer).unwrap_or_default();
        for u in inits {
            self.object_init(game, u);
        }
        match r {
            Ok(Some(u)) => Some(u),
            Ok(None) => None,
            Err(e) => {
                self.unit_error(e);
                None
            }
        }
    }

    /// The add step of an allocation: add to the list in the
    /// allocation's room, then the path part at (x, y). `false`: the add
    /// failed (logged).
    pub fn add_allocated(
        &mut self,
        game: &mut Game,
        u: UnitId,
        req: &AllocRequest,
        x: i32,
        y: i32,
    ) -> bool {
        let r = {
            let mut sim = Sim {
                game,
                units: self.units,
                stats: self.stats,
                data: self.data,
            };
            crate::units::lifecycle::add(&mut sim, &mut *self.h, u, req)
        };
        if let Err(e) = r {
            self.unit_error(e);
            return false;
        }
        self.path_place(game, u, x, y);
        if self.h.paths.is_some() {
            self.monster_added(game, u, x, y);
        }
        true
    }

    /// The monster branch of the unit add, after the path:
    /// the path velocity := monstats `Velocity` · 256, then the monster
    /// mode set of the creation mode, whose start function
    /// schedules the first think, then the think restart (the recorded
    /// "+aidel, cancel, +2" pairs), gated by whether the unit has a
    /// room and that active room's client count is nonzero; else
    /// the room clean-up ([`View::room_cleanup`]) and no
    /// think. Not a monster: nothing. The request's target point is
    /// (x, y): the mode set writes it to the path's target point.
    fn monster_added(&mut self, game: &mut Game, u: UnitId, x: i32, y: i32) {
        let Some((class, mode)) = self
            .units
            .get(u)
            .filter(|r| r.ty == UnitType::Monster)
            .map(|r| (r.class, r.mode))
        else {
            return;
        };
        let velocity = self
            .h
            .tables
            .combat
            .monstats
            .get(class as usize)
            .map_or(0, |m| i32::from(m.velocity))
            << 8;
        self.h.path_set_velocity(u, velocity);
        crate::wiring::path::monsters::stage_request(
            self.h,
            u,
            crate::monsters::ai::ModeTarget::Point(x, y),
            None,
        );
        let r = {
            let mut sim = Sim {
                game: &mut *game,
                units: self.units,
                stats: self.stats,
                data: self.data,
            };
            crate::units::modes::monster_set_mode(&mut sim, &mut *self.h, u, mode)
        };
        // A creation mode the path set-up skips (GH) leaves the
        // staged target unread: it must not reach a later request.
        if let Some(p) = self.h.paths.as_mut() {
            if matches!(p.mode_request, Some((v, _)) if v == u) {
                p.mode_request = None;
            }
        }
        if let Err(e) = r {
            self.unit_error(e);
            return;
        }
        if self.room_has_clients(game, u) {
            self.think_restart(game, u);
        } else {
            self.room_cleanup(u);
        }
    }

    /// Whether the unit's
    /// room exists and its active room's client count is nonzero.
    fn room_has_clients(&self, game: &Game, u: UnitId) -> bool {
        let Some(room) = game.lists.unit(u).and_then(|e| e.room()) else {
            return false;
        };
        self.h
            .drlg
            .drlg_room(game, room)
            .and_then(|(d, id)| d.active_room(id))
            .is_some_and(|a| !a.clients.is_empty())
    }

    /// The allocation room of a unit between the kind init and the add,
    /// else the room it stands in.
    pub fn init_room(&self, game: &Game, u: UnitId) -> Option<crate::units::RoomId> {
        match self.h.alloc_rooms.iter().rev().find(|&&(v, _)| v == u) {
            Some(&(_, room)) => room,
            None => game.lists.unit(u).and_then(|e| e.room()),
        }
    }

    /// A unit leaves its room and is kept: its room's delete record,
    /// with `free_collision` the collision free,
    /// then the unlink (room list, update queue, allied count)
    /// and the path's room cleared. The unit is not freed.
    /// False: not in the lists.
    pub fn leave_room(&mut self, game: &mut Game, u: UnitId, free_collision: bool) -> bool {
        let Some((ty, guid, room)) = game.lists.unit(u).map(|e| (e.ty, e.guid, e.room())) else {
            return false;
        };
        if let Some(room) = room {
            self.room_delete(game, room, ty as u8, guid);
        }
        if free_collision {
            self.free_unit_collision(game, u);
        }
        if let Err(e) = game.lists.room_remove(u) {
            self.unit_error(crate::game::GameError::from(e).into());
        }
        if let Some(p) = self.h.paths.as_mut().and_then(|p| p.records.get_mut(&u)) {
            match p {
                crate::path::UnitPath::Static(s) => s.room = None,
                crate::path::UnitPath::Dynamic(d) => d.room = None,
            }
        }
        true
    }

    /// The collision free for a monster or an object: the box clear
    /// at the path's room and position, sized by
    /// monstats2 `SizeX` / `SizeY` (signed), masked
    /// by the unit's collision mask (a monster: its path's mask, 0xFFFF
    /// without a path); a
    /// size ≤ 0 clears nothing. Objects: [`Self::free_object_footprint`].
    fn free_unit_collision(&mut self, game: &Game, u: UnitId) {
        let Some(r) = self.units.get(u) else { return };
        match r.ty {
            UnitType::Object => {
                self.free_object_footprint(game, u);
            }
            UnitType::Monster => {
                let t = &self.h.tables.combat;
                let Some(m2) = t
                    .monstats
                    .get(r.class as usize)
                    .and_then(|m| t.monstats2.get(usize::from(m.monstatsex)))
                else {
                    return;
                };
                let (sx, sy) = (i32::from(m2.sizex as i8), i32::from(m2.sizey as i8));
                if sx <= 0 || sy <= 0 {
                    return;
                }
                let Some(p) = self.h.paths.as_ref().and_then(|p| p.record(u)) else {
                    return;
                };
                let mask = match p {
                    crate::path::UnitPath::Dynamic(d) => d.foot_mask,
                    crate::path::UnitPath::Static(_) => 0xFFFF,
                };
                let (room, (x, y)) = (p.room(), p.position());
                crate::path::collision::box_apply(
                    &mut self.h.drlg,
                    room,
                    x,
                    y,
                    (sx as u32, sy as u32),
                    mask,
                    false,
                );
            }
            _ => {}
        }
    }

    /// Unit removal.
    pub fn remove(&mut self, game: &mut Game, u: UnitId) {
        let r = {
            let mut sim = Sim {
                game,
                units: self.units,
                stats: self.stats,
                data: self.data,
            };
            crate::units::lifecycle::remove(&mut sim, &mut *self.h, u)
        };
        if let Err(e) = r {
            self.unit_error(e);
        }
    }

    /// A monster mode change. Its umod
    /// callbacks run inside it ([`UnitHooks::monster_umods`]: mode 0
    /// before the start function, mode 1 after the animation prepare)
    /// on the lent monster world.
    pub fn monster_set_mode(&mut self, game: &mut Game, u: UnitId, mode: u32) -> bool {
        let mut sim = Sim {
            game,
            units: self.units,
            stats: self.stats,
            data: self.data,
        };
        let r = crate::units::modes::monster_set_mode_started(&mut sim, &mut *self.h, u, mode);
        match r {
            Ok(started) => started,
            Err(e) => {
                self.unit_error(e);
                false
            }
        }
    }
}

/// Clears state 54 and, for players, state 92 (the check before a think,
/// without its timer part).
pub fn clear_uninterruptable<X: Pending>(v: &mut View<'_, X>, game: &Game, u: UnitId) {
    v.set_state(u, state::UNINTERRUPTABLE as u16, false);
    if game.lists.unit(u).is_some_and(|e| e.ty == UnitType::Player) {
        v.set_state(u, STATE_DEATH_DELAY, false);
    }
}

/// Largest speed (at most 0x7FFF, i.e. 32,767).
const SPEED_MAX: u32 = 0x7FFF;

/// monstats rows below this take their run base from the walk speed.
const RUN_BASE_ROWS: usize = 410;

/// The monmode tokens the speed lookups name (2 → `WL`, 15 → `RN`).
fn monster_mode_token(mode: u32) -> Option<&'static [u8; 4]> {
    match mode {
        2 => Some(b"WL\0\0"),
        15 => Some(b"RN\0\0"),
        _ => None,
    }
}

impl<X: Pending> ActionHooks<X> {
    /// Pet follow for the summoned pet types of [`ActionHooks::pet_lists`]
    /// (types one up to count − 1 but 7, the hireling's, which has its own
    /// list): by the `pettype` flag byte, `warp` (bit 0) → every living pet is
    /// moved to the player ([`LifecycleHooks::warp_pet`]); else `range`
    /// (bit 1) → the GUIDs of the living pets farther than 1600 (squared
    /// distance) are returned for the caller's remove with
    /// kill. TODO: a type with neither flag frees its
    /// nodes; the game data has no such summon row.
    pub(crate) fn summon_follow(&mut self, sim: &mut Sim<'_>, player: UnitId) -> Vec<i32> {
        use crate::units::modes::monster_mode;
        const HIRELING_TYPE: usize = 7;
        const RANGE_LIMIT: i32 = 1600;
        let Some(b) = self.bodies.clone() else {
            return Vec::new();
        };
        let Some(lists) = self.pet_lists.get(&player).cloned() else {
            return Vec::new();
        };
        let (px, py) = self.path_position(player);
        let mut far = Vec::new();
        for (t, e) in lists.entries.iter().enumerate().skip(1) {
            if t == HIRELING_TYPE {
                continue;
            }
            let flags = b.pettype_flags.get(t).copied().unwrap_or(0);
            for n in &e.nodes {
                let Some(unit) = sim.game.lists.find_unit(UnitType::Monster, n.guid as u32) else {
                    continue;
                };
                let alive = sim
                    .units
                    .get(unit)
                    .is_some_and(|r| r.mode != monster_mode::DT && r.mode != monster_mode::DD);
                if !alive {
                    continue;
                }
                if flags & 1 != 0 {
                    self.warp_pet(sim, unit, player);
                } else if flags & 2 != 0 {
                    let (x, y) = self.path_position(unit);
                    let (dx, dy) = (x.wrapping_sub(px), y.wrapping_sub(py));
                    let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                    if d > RANGE_LIMIT {
                        far.push(n.guid);
                    }
                }
            }
        }
        far
    }

    /// The knockback and velocity parts of the animation rate: knockback
    /// (player mode 19, monster mode 13) → w clamped to 0..0x7FFF; a mode
    /// with the velocity modifier → w · p / 100 (i32,
    /// truncating; 0 if ≤ 0, at most 0x7FFF). w: player 101
    /// in mode 3, else 213; monster the run speed in mode 15, else
    /// the walk speed. `None`: another
    /// part applies, the unit has no path (nothing), or there is
    /// no path provider.
    // TODO: (T, C, M) are the unit's own; the
    // disguise substitution is not applied here.
    pub(crate) fn movement_rate(&self, sim: &Sim<'_>, unit: UnitId) -> Option<i16> {
        use crate::path::walk::velocity::{velocity_percent, VelocityFacts, STAT_VELOCITYPERCENT};
        let paths = self.paths.as_ref()?;
        let (ty, class, mode) = self.draw_identity(sim, unit)?;
        let knockback = matches!((ty, mode), (UnitType::Player, 19) | (UnitType::Monster, 13));
        let w = |h: &Self| -> i32 {
            match ty {
                UnitType::Player => {
                    if mode == 3 {
                        101
                    } else {
                        213
                    }
                }
                _ if mode == 15 => h.monster_run_speed(class as usize) as i32,
                _ => h.monster_walk_speed(class as usize) as i32,
            }
        };
        if !matches!(ty, UnitType::Player | UnitType::Monster) {
            return None;
        }
        if knockback {
            return Some(w(self).clamp(0, SPEED_MAX as i32) as i16);
        }
        if !self.path_has(unit) {
            return None;
        }
        let [_, _, scale_stat] = paths.tables.animstat[4];
        let facts = VelocityFacts {
            ty,
            class,
            npc: ty == UnitType::Monster
                && self
                    .tables
                    .combat
                    .monstats
                    .get(class as usize)
                    .is_some_and(|m| m.npc),
            used_flags: self
                .used_skill_of(unit)
                .map(|e| self.entry_flags_of(unit, &e)),
            // The item/skill getter is the unit total: Burst of Speed's
            // state list counts.
            item_fastermove: sim.stats.unit_total(unit, scale_stat as u16, 0),
            velocitypercent: sim.stats.unit_total(unit, STAT_VELOCITYPERCENT, 0),
        };
        let p = velocity_percent(&paths.tables, &facts, mode)?;
        let v = w(self).wrapping_mul(p) / 100;
        Some(if v <= 0 { 0 } else { v.min(SPEED_MAX as i32) } as i16)
    }

    /// The tables of the monster speeds ([`MonsterSpeeds`]).
    fn monster_speeds(&self) -> MonsterSpeeds<'_> {
        MonsterSpeeds {
            monstats: &self.tables.combat.monstats,
            monstats2: &self.tables.combat.monstats2,
            anim: self.anim_data.as_deref(),
        }
    }

    /// The walk speed of row `r` ([`MonsterSpeeds::walk`]).
    fn monster_walk_speed(&self, r: usize) -> u32 {
        self.monster_speeds().walk(r)
    }

    /// The run speed of row `r` ([`MonsterSpeeds::run`]).
    fn monster_run_speed(&self, r: usize) -> u32 {
        self.monster_speeds().run(r)
    }
}

/// The monstats walk / run speeds the loader
/// computes, from the
/// compiled rows and AnimData. The typed rows carry no walk / run speed,
/// so the rate call computes them here; the values equal d2-data's
/// fixup (`d2_data::fixup::records::monstats_speeds`).
pub(crate) struct MonsterSpeeds<'a> {
    /// The monstats rows.
    pub monstats: &'a [Monstats],
    /// The monstats2 rows.
    pub monstats2: &'a [Monstats2],
    /// The AnimData file, if loaded.
    pub anim: Option<&'a AnimData>,
}

impl MonsterSpeeds<'_> {
    /// The AnimData speed of monster class `class` in `mode`
    /// (2 or 15) under the fixup's own COF name (composed with no unit):
    /// the monstats `Code`,
    /// the monmode token and the monstats2 `BaseW` of the row's
    /// `MonStatsEx` (`hth` when out of range), each cut at its first
    /// space or NUL. Not the name the unit is drawn with. A missing name
    /// or table reads the default record's 256.
    fn anim_speed(&self, class: usize, mode: u32) -> u32 {
        let (Some(data), Some(token), Some(m)) = (
            self.anim,
            monster_mode_token(mode),
            self.monstats.get(class),
        ) else {
            return 256;
        };
        let wclass = self
            .monstats2
            .get(usize::from(m.monstatsex))
            .map_or(*b"hth\0", |m2| m2.basew);
        let mut name = Vec::with_capacity(7);
        for code in [&m.code, token, &wclass] {
            name.extend(code[..3].iter().take_while(|&&b| b != 0x20 && b != 0));
        }
        data.record(&name).map_or(256, |r| r.speed)
    }

    /// Row `r`'s `BaseId` after the repair (out of range →
    /// `r`).
    fn base(&self, r: usize) -> usize {
        let b = self.monstats.get(r).map_or(-1, |m| m.baseid as i16);
        if b < 0 || b as usize >= self.monstats.len() {
            r
        } else {
            b as usize
        }
    }

    /// The walk speed of row `r`.
    pub fn walk(&self, r: usize) -> u32 {
        let b = self.base(r);
        let mut w = self.anim_speed(b, 2);
        let vel = |i: usize| self.monstats.get(i).map_or(0, |m| m.velocity as i16);
        if b != r && vel(b) > 0 {
            w = (i32::from(vel(r)) as u32).wrapping_mul(w) / vel(b) as u32;
        }
        w.min(SPEED_MAX)
    }

    /// The run speed of row `r` (a base row after
    /// `r` reads its compiled walk speed, 0).
    pub fn run(&self, r: usize) -> u32 {
        let b = self.base(r);
        let mut run = if r < RUN_BASE_ROWS {
            if b > r {
                0
            } else {
                (i32::from(self.walk(b) as i16) / 2) as u32
            }
        } else {
            self.anim_speed(b, 15)
        };
        let rn = |i: usize| self.monstats.get(i).map_or(0, |m| m.run as i16);
        if b != r && rn(b) > 0 {
            run = (i32::from(rn(r)) as u32).wrapping_mul(run) / rn(b) as u32;
        }
        run.min(SPEED_MAX)
    }
}

#[cfg(test)]
mod speed_tests {
    use super::MonsterSpeeds;
    use d2_data::bin::{self, BinTable};
    use d2_data::fixup;
    use d2_data::tables::{decode_all, Monstats, Monstats2};
    use d2_formats::mpq::ArchiveSet;

    fn u16_at(t: &BinTable, r: usize, off: usize) -> u32 {
        let rec = t.record(r);
        u32::from(u16::from_le_bytes([rec[off], rec[off + 1]]))
    }

    /// Every monstats row of the real game data: the rate call's walk /
    /// run speeds equal
    /// the loader's (d2-data's fixup of the live set).
    #[test]
    #[ignore = "needs the game install in D2_GAME_DIR"]
    #[allow(clippy::disallowed_methods)]
    fn real_speeds_match_the_fixup() {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bin = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set");
        let anim = fixup::read_animdata(&set).expect("AnimData.d2");
        let fixed = fixup::apply(&bin, &anim).expect("fix-ups apply");
        let ms: Vec<Monstats> = decode_all(bin.table("monstats").unwrap()).unwrap();
        let ms2: Vec<Monstats2> = decode_all(bin.table("monstats2").unwrap()).unwrap();
        let f = fixed.table("monstats").unwrap();
        let s = MonsterSpeeds {
            monstats: &ms,
            monstats2: &ms2,
            anim: Some(&anim),
        };
        let mut bad = Vec::new();
        for r in 0..ms.len() {
            let want = (u16_at(f, r, 0x36), u16_at(f, r, 0x38));
            if (s.walk(r), s.run(r)) != want {
                bad.push((r, s.walk(r), s.run(r), want));
            }
        }
        assert!(
            bad.is_empty(),
            "{} rows differ: {:?}",
            bad.len(),
            &bad[..bad.len().min(8)]
        );
    }
}
