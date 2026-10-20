//! The umod callbacks' seams ([`crate::monsters::init::InitHost`],
//! "callbacks" section) on [`WorldHost`]: what the world host reaches
//! itself. Stats, states, positions and rooms are the action systems'
//! ([`crate::wiring::action::View`]); missile creation is [`crate::missiles::create_missile`] on
//! the missile store; the umod missile hit is the missile
//! damage path of `wiring::action::missiles` with the caller's record;
//! the unit find runs [`find::find_units`] over the act room
//! lists; the footprint test is the path collision query on the act
//! DRLG; owners are the missile store's (missiles) or
//! [`crate::wiring::action::Pending::ai_owner`]; minion owners and
//! lists are [`super::WorldState`]'s.
//!
//! A mode set from a callback ([`WorldHost::umod_set_mode`]) lends the
//! world state back to the action hooks for the call, so the nested
//! dispatcher runs.
//!
//! The rest (skill use, AI params, quests, items, pets, targets) is
//! [`WorldPending`]'s.

use crate::combat::DamageRecord;
use crate::missiles::{self, MissileParams};
use crate::monsters::init::callbacks::{missile_flag, AuraFields, SkillCalc};
use crate::monsters::init::find::{self, FindQuery, FindWorld};
use crate::skills::use_::bodies::MissileRequest;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::WiringError;

use super::{WorldHost, WorldPending, WorldgenError};

/// The footprint test's pattern and mask (pattern 1, mask
/// 0x3C01: anything that blocks a spawn).
const FOOTPRINT_PATTERN: u32 = 1;
const FOOTPRINT_MASK: u16 = 0x3C01;
/// The line mask of the area test (walls and missile blockers).
const AREA_LINE_MASK: u32 = 0x805;

impl<X: WorldPending> WorldHost<'_, X> {
    /// "Base list set": the unit's list with flag 1 (its base
    /// list); nothing without one.
    pub(super) fn umod_set_base_list(&mut self, unit: UnitId, stat: u16, value: i32) {
        let st = &self.v.stats;
        if let Some(l) = st.unit_list(unit).and_then(|l| st.list_by_flags(l, 1)) {
            self.v.set_list_stat(l, stat, value);
        }
    }

    /// A monster mode set from a callback, with the world state lent to
    /// the action hooks so the nested umod dispatcher runs.
    pub(super) fn umod_set_mode(&mut self, unit: UnitId, mode: u32) {
        if self.v.units.get(unit).map(|r| r.ty) != Some(UnitType::Monster) {
            return;
        }
        let placeholder = self.w.placeholder();
        let real = std::mem::replace(&mut *self.w, placeholder);
        let lent = self.v.h.relend_monster_world(Box::new(real));
        self.v.monster_set_mode(self.game, unit, mode);
        match self.v.h.take_relent_monster_world(lent) {
            Some(w) => *self.w = w,
            None => self
                .w
                .errors
                .push(WorldgenError::Wiring(WiringError::Reentrant(
                    "umod mode set",
                ))),
        }
    }

    /// Owner: a missile's from the missile store, else
    /// [`crate::wiring::action::Pending::ai_owner`].
    pub(super) fn umod_owner(&self, unit: UnitId) -> Option<UnitId> {
        let r = self.v.units.get(unit)?;
        if r.ty == UnitType::Missile {
            let o = self.v.h.missiles.as_ref()?.get(unit)?.owner?;
            return self.game.lists.find_unit(o.ty, o.guid);
        }
        self.v.h.x.ai_owner(self.game, unit)
    }

    /// Target: the path's target unit while it
    /// still resolves to the same unit; the unit itself counts as none.
    pub(super) fn umod_path_target(&self, unit: UnitId) -> Option<UnitId> {
        let t = self.v.h.paths.as_ref()?.dynamic(unit)?.target_unit?;
        (self.game.lists.find_unit(t.ty, t.guid) == Some(t.unit) && t.unit != unit)
            .then_some(t.unit)
    }

    /// Missile creation on the missile store.
    pub(super) fn umod_create_missile(&mut self, req: MissileRequest<UnitId>) -> Option<UnitId> {
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
        let Some(mut store) = self.v.h.missiles.take() else {
            // A missile made inside a missile's creation (the mode-5
            // callbacks): the store is lent to the outer creation.
            self.w
                .errors
                .push(WorldgenError::Wiring(WiringError::Reentrant("missiles")));
            return None;
        };
        let t = self.v.h.tables.clone();
        let made = {
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut self.v,
            };
            missiles::create_missile(self.game, &mut cx, &p)
        };
        self.v.h.missiles = Some(store);
        // Mode-5 dispatches the creation asked while the world was out
        // (`ActionHooks::deferred_missile_umods`).
        while !self.v.h.deferred_missile_umods.is_empty() {
            let (owner, missile) = self.v.h.deferred_missile_umods.remove(0);
            let t = self.w.tables.clone();
            crate::monsters::init::dispatch(
                &t.init(),
                self,
                owner,
                Some(missile),
                crate::wiring::action::monsters::umod_mode::MISSILE,
            );
        }
        made
    }

    /// missiles row flags (+0x04) as the callbacks test them.
    pub(super) fn umod_missile_flags(&self, class: i32) -> Option<u32> {
        let row = self.v.h.tables.missiles.get(usize::try_from(class).ok()?)?;
        let mut f = 0;
        for (on, bit) in [
            (row.explosion, missile_flag::EXPLOSION),
            (row.nomultishot, missile_flag::NO_MULTI_SHOT),
            (row.nouniquemod, missile_flag::NO_UNIQUE_MOD),
        ] {
            if on {
                f |= bit;
            }
        }
        Some(f)
    }

    /// Missile velocity `(m, L)`: `Vel` + trunc(`VelLev` × L / 8).
    pub(super) fn umod_missile_velocity(&self, class: i32, level: i32) -> i32 {
        usize::try_from(class)
            .ok()
            .and_then(|c| self.v.h.tables.missiles.get(c))
            .map_or(0, |row| {
                i32::from(row.vel).wrapping_add(level.wrapping_mul(i32::from(row.vellev)) / 8)
            })
    }

    /// A missile's (skill, level) from the missile store.
    pub(super) fn umod_missile_skill_level(&self, m: UnitId) -> (i32, i32) {
        self.v
            .h
            .missiles
            .as_ref()
            .and_then(|s| s.get(m))
            .map_or((0, 0), |d| (i32::from(d.skill), i32::from(d.level)))
    }

    /// The unit find from the room containing (x, y), searched
    /// from `near`'s room.
    pub(super) fn umod_find_units(&mut self, near: UnitId, q: FindQuery) -> Vec<UnitId> {
        let start = self
            .room_of(near)
            .and_then(|r| self.v.h.drlg.find_room(self.game, r, q.x, q.y));
        let mut w = Finder { h: self };
        find::find_units(&mut w, start, &q)
    }

    /// The area test: the line from (x, y) to the unit's position in its
    /// room is clear under mask 0x805.
    pub(super) fn umod_line_clear(&self, from: (i32, i32), unit: UnitId) -> bool {
        let Some(room) = self.room_of(unit) else {
            return false;
        };
        let to = self.v.h.path_position(unit);
        !crate::path::line::line_test(
            &self.v.h.drlg,
            Some(room),
            crate::path::Point::new(from.0, from.1),
            crate::path::Point::new(to.0, to.1),
            AREA_LINE_MASK as u16,
        )
        .blocked()
    }

    /// The umod missile hit `(game, src, V, rec)`: nothing
    /// unless `src` is a missile with an owner; the missile's result
    /// flags on a copy of the record, then the missile damage path.
    pub(super) fn umod_missile_hit(&mut self, src: UnitId, unit: UnitId, rec: &DamageRecord) {
        let Some(store) = self.v.h.missiles.take() else {
            self.w
                .errors
                .push(WorldgenError::Wiring(WiringError::Reentrant("missiles")));
            return;
        };
        let owner = store.get(src).and_then(|d| d.owner);
        self.v.h.missiles = Some(store);
        let Some(owner) = owner.and_then(|o| self.game.lists.find_unit(o.ty, o.guid)) else {
            return;
        };
        // The hit's damage apply runs the owner's mode-3 umods
        // (an attacker's umods such as a cold or lightning enchant): the world goes
        // back to the hooks for the nested dispatch, as for a mode set.
        let placeholder = self.w.placeholder();
        let real = std::mem::replace(&mut *self.w, placeholder);
        let lent = self.v.h.relend_monster_world(Box::new(real));
        self.v.missile_record_hit(self.game, owner, src, unit, *rec);
        match self.v.h.take_relent_monster_world(lent) {
            Some(w) => *self.w = w,
            None => self
                .w
                .errors
                .push(WorldgenError::Wiring(WiringError::Reentrant(
                    "umod missile hit",
                ))),
        }
    }

    /// The aura columns of a skills row.
    pub(super) fn umod_aura_fields(&self, skill: u16) -> Option<AuraFields> {
        let r = self.v.h.tables.skills.skills.get(usize::from(skill))?;
        let s = |v: u16| i32::from(v as i16);
        Some(AuraFields {
            stats: [
                s(r.aurastat1),
                s(r.aurastat2),
                s(r.aurastat3),
                s(r.aurastat4),
                s(r.aurastat5),
                s(r.aurastat6),
            ],
            target_state: s(r.auratargetstate),
        })
    }

    /// `eval(unit, field, skill, L)` of a skills.txt calc field.
    pub(super) fn umod_skill_calc(
        &mut self,
        unit: UnitId,
        skill: u16,
        c: SkillCalc,
        l: i32,
    ) -> i32 {
        let t = self.v.h.tables.clone();
        let Some(r) = t.skills.skills.get(usize::from(skill)) else {
            return 0;
        };
        let field = match c {
            SkillCalc::AuraRange => r.aurarangecalc,
            SkillCalc::AuraLen => r.auralencalc,
            SkillCalc::AuraStat(1) => r.aurastatcalc1,
            SkillCalc::AuraStat(2) => r.aurastatcalc2,
            SkillCalc::AuraStat(3) => r.aurastatcalc3,
            SkillCalc::AuraStat(4) => r.aurastatcalc4,
            SkillCalc::AuraStat(5) => r.aurastatcalc5,
            SkillCalc::AuraStat(6) => r.aurastatcalc6,
            SkillCalc::AuraStat(_) => return 0,
        };
        let mut cv = self.v.combat(self.game);
        crate::skills::eval_skill(&mut cv, &t.skills, Some(unit), field, i32::from(skill), l)
    }

    /// itemstatcost and states counts.
    pub(super) fn umod_counts(&self) -> (i32, i32) {
        let st = i32::try_from(self.v.stats.data().states.count()).unwrap_or(i32::MAX);
        (self.v.h.tables.skills.stat_count, st)
    }

    /// The footprint test `(room, x, y, 1, 0x3C01)` ≠ 0 on the act DRLG.
    pub(super) fn umod_footprint_occupied(&self, unit: UnitId) -> bool {
        let room = self.room_of(unit);
        let (x, y) = self.v.h.path_position(unit);
        crate::path::collision::pattern_value(
            &self.v.h.drlg,
            room,
            x,
            y,
            FOOTPRINT_PATTERN,
            FOOTPRINT_MASK,
        ) != 0
    }

    /// The area level of the unit's room: `None` without a level id.
    pub(super) fn umod_room_area_level(&self, unit: UnitId) -> Option<i32> {
        let id = self.room_of(unit).map_or(0, |r| self.room_level_id(r));
        if id == 0 {
            return None;
        }
        let info = self.w.init_info;
        Some(crate::monsters::init::area_level(
            &self.w.tables.levels,
            id,
            info.d(),
            info.expansion,
        ))
    }

    /// Clears the owner data `(game, unit, −1, 1, 0, 0)`: the owner link of the wired
    /// world goes; the owner data itself is the host's.
    pub(super) fn umod_clear_owner_data(&mut self, unit: UnitId) {
        self.w.owners.remove(&unit);
        self.v.h.x.clear_owner_data(unit);
    }
}

/// [`FindWorld`] over the world host.
struct Finder<'h, 'a, X> {
    h: &'h mut WorldHost<'a, X>,
}

impl<X: WorldPending> FindWorld for Finder<'_, '_, X> {
    fn room_box(&self, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        let t = self.h.v.h.drlg.subtiles(self.h.game, room)?;
        Some((t.x, t.y, t.w, t.h))
    }
    fn adjacent(&self, room: RoomId) -> Vec<RoomId> {
        self.h
            .game
            .lists
            .room(room)
            .map_or_else(Vec::new, |r| r.adjacent.clone())
    }
    fn room_in_town(&self, room: RoomId) -> bool {
        self.h.v.h.drlg.in_town(self.h.game, room)
    }
    fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        self.h.game.lists.room_units(room)
    }
    fn unit_type(&self, unit: UnitId) -> Option<UnitType> {
        self.h.v.units.get(unit).map(|r| r.ty)
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.h.v.units.get(unit).map_or(0, |r| r.mode)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.h.v.h.path_position(unit)
    }
    fn unit_flags(&self, unit: UnitId) -> u32 {
        self.h.v.units.get(unit).map_or(0, |r| r.flags)
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.h.v.h.is_undead(unit)
    }
    fn missile_explosion(&self, unit: UnitId) -> Option<bool> {
        let class = self.h.v.units.get(unit)?.class;
        let f = self.h.umod_missile_flags(class as i32)?;
        Some(f & missile_flag::EXPLOSION != 0)
    }
    /// Flag 0x200 (a line walk with a mask-4 test at each step):
    /// no umod caller passes it and the wired world has no
    /// provider of that walk, so a unit tested with it is not found.
    fn line_blocked(&self, _: (i32, i32), _: UnitId) -> bool {
        true
    }
}
