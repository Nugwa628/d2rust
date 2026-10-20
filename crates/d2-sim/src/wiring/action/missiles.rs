//! Missiles ↔ units, DRLG and combat: [`View`] implements
//! [`crate::missiles::MissileWorld`] and its sub-seams (`MissileUnits`,
//! `MissilePath`, `MissileRooms`, `MissileCombat`, `MissileHooks`,
//! `MissileBodies`). Real providers: unit allocation and removal, seeds,
//! stats, states and state lists, unit flags, the hireling test, the
//! `justhit` list and its type-12 timer; the room search, town test and
//! collision grids of the DRLG; the to-hit test and the damage
//! application. Path, hostility, skill setup and server-damage functions go
//! to [`Pending`].

use crate::combat::{self, result, DamageRecord};
use crate::game::Game;
use crate::missiles::{
    result_flag, stat, Damage, MissileCombat, MissileData, MissileHooks, MissilePath, MissileRooms,
    MissileStore, MissileUnits,
};
use crate::rng::Seed;
use crate::tick::events::event;
use crate::units::hooks::Sim;
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};

use super::combat::CombatView;
use super::monsters::umod_mode;
use super::units::STATE_JUSTHIT;
use super::{Pending, View, WiringError};

/// Pierce percent stat of the damage record (stat 327).
const PIERCE_PERCENT_STAT: u16 = stat::DAMAGE_FRAMERATE;
/// Armor class (stat 31).
const ARMORCLASS: u16 = 31;

impl<X: Pending> View<'_, X> {
    /// A [`CombatView`] on `game` over this view's parts.
    pub fn combat<'b>(&'b mut self, game: &'b mut Game) -> CombatView<'b, X> {
        CombatView {
            game,
            v: View {
                units: &mut *self.units,
                stats: &mut *self.stats,
                data: self.data,
                h: &mut *self.h,
            },
        }
    }

    fn unit_room(game: &Game, u: UnitId) -> Option<RoomId> {
        game.lists.unit(u).and_then(|e| e.room())
    }
}

impl<X: Pending> MissileUnits for View<'_, X> {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.h.rules
    }
    fn alloc_missile(
        &mut self,
        game: &mut Game,
        class: u16,
        x: i32,
        y: i32,
        room: RoomId,
        mode: u8,
    ) -> Option<UnitId> {
        // TODO: whether a missile counts toward the room's allied count is
        // not known; it does not.
        let req = AllocRequest {
            ty: UnitType::Missile,
            class: u32::from(class),
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: u32::from(mode),
            allied: false,
        };
        self.allocate(game, &req, x, y)
    }
    fn remove_unit(&mut self, game: &mut Game, unit: UnitId) {
        self.remove(game, unit);
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        View::seed(self, unit)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.h.path_position(unit)
    }
    fn size(&self, unit: UnitId) -> i32 {
        self.path_size(unit)
    }
    fn stat(&self, unit: UnitId, s: u16) -> i32 {
        View::stat(self, unit, s)
    }
    fn base_stat(&self, unit: UnitId, s: u16) -> i32 {
        self.stats.unit_base(unit, s, 0)
    }
    fn set_stat(&mut self, unit: UnitId, s: u16, value: i32) {
        self.set_base(unit, s, value);
    }
    fn has_state(&self, unit: UnitId, s: u16) -> bool {
        self.stats.has_state(unit, u32::from(s))
    }
    fn state_stat(&self, unit: UnitId, s: u16, st: u16) -> Option<i32> {
        View::state_stat(self, unit, s, st)
    }
    /// The missile's stat list: unit allocation already made it; nothing
    /// more.
    fn alloc_stat_list(&mut self, _: UnitId) {}
    fn unit_flag(&self, unit: UnitId, bit: u32) -> bool {
        self.units.get(unit).is_some_and(|r| r.flags & bit != 0)
    }
    fn set_unit_flag(&mut self, unit: UnitId, bit: u32, on: bool) {
        if let Some(r) = self.units.get_mut(unit) {
            if on {
                r.flags |= bit;
            } else {
                r.flags &= !bit;
            }
        }
    }
    fn may_attack(&self, attacker: UnitId, defender: UnitId) -> bool {
        self.h.x.may_attack(attacker, defender)
    }
    fn alignment(&self, unit: UnitId) -> u8 {
        self.h.x.alignment(unit)
    }
    fn is_hireling(&self, unit: UnitId) -> bool {
        self.units.get(unit).is_some_and(|r| {
            r.ty == UnitType::Monster && super::combat::HIRELING_CLASSES.contains(&r.class)
        })
    }
    /// The `justhit` mark: a stat list with state 86 expiring at `expire`,
    /// state 86 on, a type-12 timer at `expire`.
    fn apply_justhit(&mut self, game: &mut Game, unit: UnitId, expire: i32) {
        // TODO: the list's owner is not known; the hit unit's own type and
        // GUID are used.
        let list = self.create_state_list(unit, STATE_JUSTHIT, None, expire);
        // The list's remove callback: state off and the update queue, so the
        // next missile can hit the unit once the delay has passed.
        if let Some(l) = list {
            self.stats.set_remove_callback(
                l,
                Some(crate::stats::lists::RemoveCallback(
                    crate::skills::use_::bodies::callback::JUSTHIT,
                )),
            );
        }
        self.set_state(unit, STATE_JUSTHIT, true);
        if let Err(e) =
            game.schedule_event(unit, u32::from(event::REMOVE_STATE), expire, None, 0, 0)
        {
            self.unit_error(e.into());
        }
    }
}

impl<X: Pending> MissilePath for View<'_, X> {
    fn has_path(&self, unit: UnitId) -> bool {
        self.h.path_has(unit)
    }
    fn set_velocity(&mut self, unit: UnitId, v: i32) {
        self.h.path_set_velocity(unit, v);
    }
    fn velocity(&self, unit: UnitId) -> i32 {
        self.h.path_velocity(unit)
    }
    fn set_target_unit(&mut self, unit: UnitId, target: UnitId) {
        self.path_set_target_unit(unit, target);
    }
    fn set_target_point(&mut self, unit: UnitId, x: i32, y: i32) {
        self.h.path_set_target_point(unit, x, y);
    }
    fn set_footprint_mask(&mut self, unit: UnitId, mask: u16) {
        self.path_set_foot_mask(unit, mask);
    }
    fn set_move_mask(&mut self, unit: UnitId, mask: u16) {
        self.h.path_set_move_mask(unit, mask);
    }
    /// Builds the unit's path.
    fn build(&mut self, game: &mut Game, unit: UnitId) {
        if self.h.paths.is_some() {
            crate::wiring::path::walk::build_path(self, game, unit);
        } else {
            self.h.x.build_path(game, unit);
        }
    }
    fn set_acceleration(&mut self, unit: UnitId, accel: i32, max_velocity: i32) {
        self.h.path_set_acceleration(unit, accel, max_velocity);
    }
    /// The distance to the target point: max(|dx|, |dy|) + ⌊min(|dx|, |dy|) / 2⌋
    /// from the path position to the path target point, with the path
    /// provider; [`Pending`] without it.
    fn target_distance(&self, unit: UnitId) -> i32 {
        match self.path_target_xy(unit) {
            Some((tx, ty)) => {
                let (x, y) = self.h.path_position(unit);
                crate::path::walk::resync::resync_distance(
                    crate::path::Point::new(x, y),
                    crate::path::Point::new(tx, ty),
                )
            }
            None => self.h.x.target_distance(unit),
        }
    }
    /// The unit step: false when the step returns 2.
    fn step(&mut self, game: &mut Game, unit: UnitId) -> bool {
        if self.h.paths.is_some() {
            crate::wiring::path::walk::unit_step(self, game, unit)
        } else {
            self.h.x.step(game, unit)
        }
    }
    /// The collision word: recomputed at the current position when the
    /// path velocity is 0 (the room's collision mask with the missile's
    /// size, all bits), otherwise the word the last step cached in the
    /// path.
    fn collision_word(&self, game: &Game, unit: UnitId) -> u16 {
        let cached = self.h.path_cached_word(unit);
        let (x, y) = self.h.path_position(unit);
        match cached {
            Some(w) if self.h.path_velocity(unit) != 0 => w,
            _ if self.h.paths.is_some() => crate::path::collision::size_value(
                &self.h.drlg,
                Self::unit_room(game, unit),
                x,
                y,
                self.path_size(unit),
                0xFFFF,
            ),
            // TODO: without a path provider caching the word, it is read from the grid at the
            // current position.
            _ => Self::unit_room(game, unit)
                .and_then(|r| self.h.drlg.collision(game, r, x, y))
                .unwrap_or(0),
        }
    }
    fn crossed_subtiles(&self, unit: UnitId) -> Vec<(i32, i32)> {
        self.h.path_crossed(unit)
    }
}

impl<X: Pending> MissileRooms for View<'_, X> {
    fn find_room(&self, game: &Game, near: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.h.drlg.find_room(game, near, x, y)
    }
    fn in_town(&self, game: &Game, room: RoomId) -> bool {
        self.h.drlg.in_town(game, room)
    }
    /// The collision mask at (x, y) (with the path provider, the size
    /// shapes of the path placement).
    ///
    /// TODO: without the path provider the sub-tile at (x, y) is read for every size.
    fn collision_mask(
        &self,
        game: &Game,
        room: RoomId,
        x: i32,
        y: i32,
        size: i32,
        mask: u16,
    ) -> u16 {
        if self.h.paths.is_some() {
            return crate::path::collision::size_value(&self.h.drlg, Some(room), x, y, size, mask);
        }
        self.h.drlg.collision(game, room, x, y).unwrap_or(0) & mask
    }
    /// The collision value at a point (with the path provider, the path
    /// placement's point test).
    fn collision_at(&self, game: &Game, room: RoomId, x: i32, y: i32, mask: u16) -> u16 {
        if self.h.paths.is_some() {
            return crate::path::collision::point_value(&self.h.drlg, Some(room), x, y, mask);
        }
        self.h.drlg.collision(game, room, x, y).unwrap_or(0) & mask
    }
    /// Clears bit 0x40 under the unit (with the path provider: the size
    /// clear at the path position, the unit's size).
    ///
    /// TODO: without the path provider the unit's sub-tile
    /// only.
    fn clear_footprint(&mut self, game: &mut Game, unit: UnitId) {
        let (x, y) = self.h.path_position(unit);
        if self.h.paths.is_some() {
            let size = self.path_size(unit);
            let room = Self::unit_room(game, unit);
            crate::path::footprint::clear_size(
                &mut self.h.drlg,
                room,
                x,
                y,
                size,
                crate::drlg::collision::bits::MISSILE,
            );
            return;
        }
        if let Some(r) = Self::unit_room(game, unit) {
            if let Some(m) = self.h.drlg.collision_mut(game, r, x, y) {
                *m &= !crate::drlg::collision::bits::MISSILE;
            }
        }
    }
    /// The units found at a point without the accept call: `r` outside 1..3 finds none;
    /// the room and its adjacency array (the room first), units in
    /// room-list order; players in mode 0 / 17 and monsters in mode 0 /
    /// 12 are skipped, objects, items and tiles never found; a unit of
    /// size `s` (> 3 counts 3, ≤ 0 skipped) is found when the query
    /// shape of size `r` overlaps its shape.
    fn units_at(&self, game: &Game, room: RoomId, x: i32, y: i32, r: i32) -> Vec<UnitId> {
        if !(1..=3).contains(&r) {
            return Vec::new();
        }
        let adjacent = game
            .lists
            .room(room)
            .map(|r| r.adjacent.clone())
            .unwrap_or_default();
        std::iter::once(room)
            .chain(adjacent.into_iter().filter(|&r| r != room))
            .flat_map(|r| game.lists.room_units(r))
            .filter(|&u| {
                let Some(rec) = self.units.get(u) else {
                    return false;
                };
                match rec.ty {
                    UnitType::Player if matches!(rec.mode, 0 | 17) => return false,
                    UnitType::Monster if matches!(rec.mode, 0 | 12) => return false,
                    UnitType::Player | UnitType::Monster | UnitType::Missile => {}
                    _ => return false,
                }
                let s = self.path_size(u).min(3);
                if s <= 0 {
                    return false;
                }
                let (ux, uy) = self.h.path_position(u);
                let (dx, dy) = ((x - ux).abs(), (y - uy).abs());
                match (r, s) {
                    (1, 1) => dx == 0 && dy == 0,
                    (1, 2) | (2, 1) => dx + dy <= 1,
                    (1, 3) | (3, 1) => dx <= 1 && dy <= 1,
                    (2, 2) => dx + dy <= 2,
                    (2, 3) | (3, 2) => (dx <= 2 && dy <= 1) || (dx <= 1 && dy <= 2),
                    _ => dx <= 2 && dy <= 2,
                }
            })
            .collect()
    }
}

/// The missile's result flags (D2RUST-local bits of
/// [`crate::missiles::result_flag`]) as the damage result bits:
/// hit 1, get-hit 4, knockback 8, soft hit 0x4000.
pub fn result_bits(missile: u32) -> u16 {
    let mut r = 0;
    for (m, c) in [
        (result_flag::HIT, result::HIT),
        (result_flag::GETHIT, result::GET_HIT),
        (result_flag::SOFTHIT, result::SOFT_HIT),
        (result_flag::KNOCKBACK, result::KNOCKBACK),
    ] {
        if missile & m != 0 {
            r |= c;
        }
    }
    r
}

/// The hit-class merge of a missile hit: the record's hit class :=
/// `HitClass` | (the record's hit class & 0xF0); the fixed-hit-class byte
/// := 1 when either has a bit in 0xF0 (else kept). So the 0x60 of
/// server-damage functions 7 and 9 survives as `HitClass` | 0x60.
pub fn merge_hit_class(rec: &mut DamageRecord, hit_class: u32) {
    let element = rec.hit_class & 0xF0;
    if element != 0 || hit_class & 0xF0 != 0 {
        rec.hit_class_fixed = 1;
    }
    rec.hit_class = hit_class | element;
}

/// The combat damage record of a missile damage record, with the missile's result flags.
pub fn damage_record(d: &Damage) -> DamageRecord {
    let mut rec = d.to_record();
    rec.result |= result_bits(d.result);
    rec
}

impl<X: Pending> MissileCombat for View<'_, X> {
    /// The damage setup on combat's view.
    fn damage_setup(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        origin: Option<UnitId>,
        missile: UnitId,
        skill: i32,
        level: i32,
    ) -> u32 {
        let t = self.h.tables.clone();
        let mut w = self.combat(game);
        crate::missiles::damage::setup(&mut w, &t.skills, owner, origin, missile, skill, level)
    }
    /// The to-hit test of a missile attack on combat's view.
    fn hit_test(&mut self, game: &mut Game, owner: UnitId, defender: UnitId, tohit: i32) -> bool {
        let t = self.h.tables.clone();
        let mut w = self.combat(game);
        combat::hit_test(
            &mut w,
            &t.skills,
            &t.combat,
            Some(owner),
            Some(defender),
            tohit,
            true,
        )
    }
    /// The rest of the missile hit: the record with the missile's hit class
    /// (`HitClass`) and pierce percent (stat 327), then the damage
    /// application (`missile = 1`) and the reaction. No owner: nothing.
    #[allow(clippy::too_many_arguments)]
    fn apply_damage(
        &mut self,
        game: &mut Game,
        store: &mut MissileStore,
        owner: Option<UnitId>,
        missile: UnitId,
        unit: UnitId,
        damage: &mut Damage,
        data: Option<&MissileData>,
    ) {
        let Some(owner) = owner else {
            return;
        };
        let mut rec = damage_record(damage);
        // The hit holds the store; a kill in the apply runs the victim's
        // mode set, and its umod callbacks create missiles nested in the
        // hit (the queen's death burst of 8 `queenpoisoncloud`, recorded in
        // mon-maggotqueen-death-cs f36), so the
        // store goes back on the hooks for the call (as in
        // `unique_mod_missile`).
        let lent = self.h.missiles.is_none();
        if lent {
            self.h.missiles = Some(std::mem::take(store));
        }
        self.apply_missile_record(game, owner, missile, unit, &mut rec, data);
        if lent {
            *store = self.h.missiles.take().unwrap_or_default();
        }
        damage.result = rec.result.into();
    }
    /// Unit event 0, also with no unit.
    fn hit_by_missile_event(&mut self, game: &mut Game, missile: UnitId, unit: Option<UnitId>) {
        self.combat(game)
            .fire_unit_event(0, unit, Some(missile), None);
    }
    /// The target damage bonus: stat 121 for demons, 122 for undead, 180 by
    /// monster type (entries whose layer matches the unit's montype), on
    /// the missile.
    fn target_damage_bonus(&self, missile: UnitId, unit: UnitId) -> i32 {
        let mut p = 0i32;
        if self.h.is_demon(unit) {
            p = p.wrapping_add(View::stat(self, missile, 121));
        }
        if self.h.is_undead(unit) {
            p = p.wrapping_add(View::stat(self, missile, 122));
        }
        let class = self.units.get(unit).map_or(0, |r| r.class as i32);
        if let Some(l) = self.stats.unit_list(missile) {
            for (k, v) in self.stats.full_entries(l) {
                if crate::stats::key_stat(k) == 180
                    && self.h.x.montype_matches(crate::stats::key_layer(k), class)
                {
                    p = p.wrapping_add(v);
                }
            }
        }
        p
    }
    /// Armor += delta, clamped at ≥ 0 (the sign is as the stat gives it).
    fn add_target_ac(&mut self, _: &mut Game, unit: UnitId, delta: i32) {
        let ac = self
            .stats
            .unit_base(unit, ARMORCLASS, 0)
            .wrapping_add(delta)
            .max(0);
        self.set_base(unit, ARMORCLASS, ac);
    }
}

impl<X: Pending> View<'_, X> {
    /// The damage part of the missile hit on a record: the missile's hit
    /// class (`HitClass`), the hit flags from its data flags 1, 2 and pierce
    /// percent (stat 327), then the damage application (`missile = 1`) and
    /// the reaction.
    fn apply_missile_record(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        missile: UnitId,
        unit: UnitId,
        rec: &mut DamageRecord,
        data: Option<&MissileData>,
    ) {
        let (class, data_flags) = data.map_or((None, 0), |d| (Some(d.class), d.flags));
        let t = self.h.tables.clone();
        if let Some(row) = class.and_then(|c| t.missiles.get(usize::from(c))) {
            merge_hit_class(rec, u32::from(row.hitclass));
        }
        rec.pierce_pct = View::stat(self, missile, PIERCE_PERCENT_STAT);
        let mut w = self.combat(game);
        // The order is: block/dodge on the
        // unit's seed (avoid 1, block = physical != 0), the hit-class
        // merge above, the monster crit on the owner's seed, then apply
        // only while the hit bit stands, and the reaction always.
        let b = combat::block_or_dodge(&mut w, &t.combat, owner, unit, true, rec.physical != 0);
        rec.result |= match b {
            combat::BlockResult::Avoid => result::AVOID,
            combat::BlockResult::Dodge => result::DODGE,
            combat::BlockResult::WeaponBlock => result::WEAPON_BLOCK,
            combat::BlockResult::Block => result::BLOCK,
            combat::BlockResult::Evade | combat::BlockResult::None => 0,
        };
        if b != combat::BlockResult::None {
            rec.result &= !result::HIT;
        }
        combat::monster_crit(&mut w, &t.combat, owner, unit, rec);
        // Missile data flags: 1 → hit flag 0x20 ("rolled",
        // so `domissiledamage` fires), 2 → 0x80 (no `domissiledamage`).
        if data_flags & 1 != 0 {
            rec.hit_flags |= combat::hitflag::ROLLED;
        }
        if data_flags & 2 != 0 {
            rec.hit_flags |= combat::hitflag::NO_MISSILE_EVENT;
        }
        if rec.result & result::HIT != 0 {
            combat::apply(&mut w, &t.combat, owner, unit, true, rec);
        }
        crate::combat::CombatWorld::reaction(&mut w, owner, unit, rec);
    }

    /// The missile hit with a caller's record: the missile's result flags
    /// ([`crate::missiles::result_flags`]: hit, get-hit / soft-hit, the knockback roll on the
    /// missile's seed) or-ed into a copy of the record, then the damage part of the missile
    /// hit. The caller has checked the owner.
    pub fn missile_record_hit(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        missile: UnitId,
        unit: UnitId,
        mut rec: DamageRecord,
    ) {
        let Some(mut store) = self.h.missiles.take() else {
            self.h.errors.push(WiringError::Reentrant("missiles"));
            return;
        };
        let t = self.h.tables.clone();
        let flags = {
            let mut cx = crate::missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut *self,
            };
            crate::missiles::result_flags(game, &mut cx, missile, unit)
        };
        let data = store.get(missile).cloned();
        self.h.missiles = Some(store);
        rec.result |= result_bits(flags);
        self.apply_missile_record(game, owner, missile, unit, &mut rec, data.as_ref());
    }
}

impl<X: Pending> MissileHooks for View<'_, X> {
    fn init_callback(&mut self, game: &mut Game, missile: UnitId, callback: u32, arg: u32) {
        self.h.x.missile_init_callback(game, missile, callback, arg);
    }
    /// The umod dispatcher in mode 5 (the callbacks get the missile) on the lent monster world;
    /// without one, [`Pending::unique_mod_missile`].
    fn unique_mod_missile(
        &mut self,
        game: &mut Game,
        store: &mut MissileStore,
        owner: UnitId,
        missile: UnitId,
    ) {
        // Made by a umod callback while the world is out (the cross bursts
        // of umod 23 / 17): the game dispatches mode 5 nested in the
        // creation; nothing is drawn after that point, so the world host
        // runs it right after the creation returns (umod_create_missile).
        if self.h.monster_world_out {
            self.h.deferred_missile_umods.push((owner, missile));
            return;
        }
        // The creation holds the store; the callbacks (multishot copies)
        // need it on the hooks for the call.
        let lent = self.h.missiles.is_none();
        if lent {
            self.h.missiles = Some(std::mem::take(store));
        }
        let mut sim = Sim {
            game: &mut *game,
            units: &mut *self.units,
            stats: &mut *self.stats,
            data: self.data,
        };
        if !self
            .h
            .run_umods(&mut sim, owner, Some(missile), umod_mode::MISSILE)
        {
            self.h.x.unique_mod_missile(game, owner, missile);
        }
        if lent {
            *store = self.h.missiles.take().unwrap_or_default();
        }
    }
}

/// The server-do / server-hit bodies' seams on the wired units: formulas on the skill tables,
/// the path provider's new-step flag and target, collision writes on the DRLG grids, unit
/// records, and the area hit on the combat view. The area scan runs on the skill pipeline
/// ([`Pending::missile_area_units`]).
impl<X: Pending> crate::missiles::MissileBodies for View<'_, X> {
    /// The `mon` list of the room's level record on the lent monster
    /// world.
    fn level_mon_list(&self, game: &Game, room: RoomId) -> Vec<i32> {
        let Some(level) = self.h.drlg.level_id(game, room) else {
            return Vec::new();
        };
        self.h
            .monster_world
            .as_ref()
            .and_then(|w| w.level_mon_list(level))
            .unwrap_or_default()
    }
    fn monster_is_spawn(&self, class: i32) -> Option<bool> {
        self.h.monster_world.as_ref()?.class_is_spawn(class)
    }
    /// Creates a monster {room, class, mode 1, GUID 0, x, y, spread 5,
    /// flags 0} on the lent monster world.
    fn create_monster(
        &mut self,
        game: &mut Game,
        room: RoomId,
        class: i32,
        at: (i32, i32),
    ) -> bool {
        with_sim(self, game, |h, sim| {
            h.with_monster_world(|w, h| w.spawn_at(sim, h, room, at.0, at.1, class, 1, 5, 0))
        })
        .flatten()
        .is_some_and(|placed| placed.is_some())
    }
    /// Max life (the heal of server-hit 31 reads it).
    fn max_life(&self, unit: UnitId) -> i32 {
        self.stats.max_life(unit)
    }
    /// Max mana.
    fn max_mana(&self, unit: UnitId) -> i32 {
        self.stats.max_mana(unit)
    }
    /// The `pettype.txt` row count.
    fn pet_type_count(&self) -> i32 {
        self.h.bodies.as_ref().map_or(0, |b| b.pettype_count)
    }
    /// The summon class on the skill pipeline ([`Pending::missile_summon_class`]).
    fn summon_class(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) -> crate::missiles::SummonClass {
        let (class, mode) = with_sim(self, game, |h, sim| {
            X::missile_summon_class(h, sim, owner, skill, level)
        });
        crate::missiles::SummonClass { class, mode }
    }
    /// The summon spawn (flags 0xD) on the skill pipeline
    /// ([`Pending::missile_summon_spawn`]).
    fn summon_spawn(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        class: i32,
        mode: i32,
        at: (i32, i32),
        pet_type: i32,
    ) -> Option<UnitId> {
        with_sim(self, game, |h, sim| {
            X::missile_summon_spawn(h, sim, owner, class, mode, at, pet_type)
        })
    }
    /// The ally test ([`Pending::missile_ally_test`]).
    fn ally_test(&mut self, game: &mut Game, a: UnitId, b: UnitId) -> bool {
        with_sim(self, game, |h, sim| X::missile_ally_test(h, sim, a, b))
    }
    /// `shout_state` on the skill use view
    /// ([`Pending::missile_shout_state`]).
    fn shout_state(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) {
        with_sim(self, game, |h, sim| {
            X::missile_shout_state(h, sim, unit, owner, skill, level)
        });
    }
    /// Terror install on the skill use view
    /// ([`Pending::missile_terror`]); `skill` and `a` are not read.
    fn terror(
        &mut self,
        game: &mut Game,
        source: UnitId,
        unit: UnitId,
        _skill: i32,
        _a: i32,
        b: i32,
    ) {
        with_sim(self, game, |h, sim| {
            X::missile_terror(h, sim, source, unit, b)
        });
    }
    /// Skill server-do `index` with the missile as the caster
    /// on the skill use view
    /// ([`Pending::missile_skill_srv_do`]). The store is lent to the hooks
    /// for the call: the body's hostility test reads the missile's owner.
    fn skill_srv_do(
        &mut self,
        game: &mut Game,
        store: &mut MissileStore,
        caster: UnitId,
        index: i32,
        skill: i32,
        level: i32,
    ) {
        let lent = self.h.missiles.is_none();
        if lent {
            self.h.missiles = Some(std::mem::take(store));
        }
        with_sim(self, game, |h, sim| {
            X::missile_skill_srv_do(h, sim, caster, index, skill, level)
        });
        if lent {
            *store = self.h.missiles.take().unwrap_or_default();
        }
    }
    /// Binds a bone wall piece ([`Pending::missile_bone_wall_piece`]).
    fn bind_bone_wall_piece(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        anchor: UnitId,
        piece: UnitId,
        skill: i32,
        level: i32,
    ) {
        with_sim(self, game, |h, sim| {
            X::missile_bone_wall_piece(h, sim, owner, anchor, piece, skill, level)
        });
    }
    /// The quest control's published answer
    /// ([`Pending::quest_not_intro`]).
    fn quest_test(&self, _game: &Game, id: i32) -> bool {
        u8::try_from(id).is_ok_and(|c| self.h.x.quest_not_intro(c))
    }
    /// Spawns Tyrael: queued for the quest control with the missile's
    /// position ([`Pending::missile_spawn_tyrael`]).
    fn spawn_tyrael(&mut self, _game: &mut Game, room: Option<RoomId>, missile: UnitId) {
        let (x, y) = self.h.path_position(missile);
        self.h.x.missile_spawn_tyrael(room, missile, x, y);
    }
    /// Evaluates a skill calc for the missile (`eval_missile`).
    fn missile_calc(
        &mut self,
        game: &mut Game,
        missile: UnitId,
        owner: Option<UnitId>,
        field: u32,
        class: i32,
        level: i32,
    ) -> i32 {
        let t = self.h.tables.clone();
        let mut cv = self.combat(game);
        crate::skills::eval_missile(
            &mut cv,
            &t.skills,
            Some(missile),
            owner,
            field,
            class,
            level,
        )
    }
    fn skill_exists(&self, skill: i32) -> bool {
        self.h.tables.skills.skill(skill).is_some()
    }
    /// The raw skills columns.
    fn skill_field(&self, skill: i32, field: crate::missiles::SkillField) -> i32 {
        use crate::missiles::SkillField as F;
        let Some(r) = self.h.tables.skills.skill(skill) else {
            return 0;
        };
        match field {
            F::Param(n) => [
                r.param1, r.param2, r.param3, r.param4, r.param5, r.param6, r.param7, r.param8,
            ]
            .get(usize::from(n).wrapping_sub(1))
            .map_or(0, |&v| v as i32),
            F::AuraFilter => r.aurafilter as i32,
            F::AuraTargetState => i32::from(r.auratargetstate as i16),
            F::PetType => i32::from(r.pettype),
        }
    }
    /// `phys_min(O, k, L, 1)` then `phys_max(O, k, L, 1)` on the combat
    /// view, as `Self::skill_calc` evaluates (the heal of server-hit 43,
    /// Healing Vortex).
    fn skill_phys(
        &mut self,
        game: &mut Game,
        owner: Option<UnitId>,
        skill: i32,
        level: i32,
    ) -> (i32, i32) {
        let t = self.h.tables.clone();
        let mut cv = self.combat(game);
        let lo = crate::skills::phys_min(&mut cv, &t.skills, owner, skill, level, true);
        let hi = crate::skills::phys_max(&mut cv, &t.skills, owner, skill, level, true);
        (lo, hi)
    }
    fn skill_calc(
        &mut self,
        game: &mut Game,
        owner: Option<UnitId>,
        skill: i32,
        calc: crate::missiles::SkillCalc,
        level: i32,
    ) -> i32 {
        use crate::missiles::SkillCalc as C;
        let t = self.h.tables.clone();
        let Some(r) = t.skills.skill(skill) else {
            return 0;
        };
        let field = match calc {
            C::Calc1 => r.calc1,
            C::Calc2 => r.calc2,
            C::AuraRange => r.aurarangecalc,
            C::AuraLen => r.auralencalc,
            C::Calc4 => r.calc4,
        };
        let mut cv = self.combat(game);
        crate::skills::eval_skill(&mut cv, &t.skills, owner, field, skill, level)
    }
    /// The path's moved bit (`path::record::flags::MOVED`) with the path
    /// provider; false without it.
    fn path_new_step(&self, unit: UnitId) -> bool {
        self.h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(unit))
            .is_some_and(|d| d.flags & crate::path::record::flags::MOVED != 0)
    }
    /// The dynamic path's target unit when it still resolves to the same
    /// unit (a stale target reads as none, the
    /// path field is left to the path code).
    fn path_target(&mut self, game: &Game, unit: UnitId) -> Option<UnitId> {
        let t = self.h.paths.as_ref()?.dynamic(unit)?.target_unit?;
        (game.lists.find_unit(t.ty, t.guid) == Some(t.unit)).then_some(t.unit)
    }
    /// Ors `bits` into the one cell at (x, y), in the room found from `room`.
    fn or_collision(&mut self, game: &mut Game, room: RoomId, x: i32, y: i32, bits: u16) {
        let Some(r) = self.h.drlg.find_room(game, room, x, y) else {
            return;
        };
        if let Some(c) = self.h.drlg.collision_mut(game, r, x, y) {
            *c |= bits;
        }
    }
    /// Stamps the collision bits at the path position with the unit's size.
    fn stamp_collision(&mut self, game: &mut Game, unit: UnitId, bits: u16) {
        let (x, y) = self.h.path_position(unit);
        let size = self.path_size(unit);
        let room = Self::unit_room(game, unit);
        crate::path::footprint::stamp_size(&mut self.h.drlg, room, x, y, size, bits);
    }
    fn anim_frame(&self, unit: UnitId) -> i32 {
        self.units.get(unit).map_or(0, |r| r.anim.frame)
    }
    fn set_anim_frame(&mut self, unit: UnitId, v: i32) {
        if let Some(r) = self.units.get_mut(unit) {
            r.anim.frame = v;
        }
    }
    fn is_dead(&self, unit: UnitId) -> bool {
        self.units.is_dead(unit)
    }
    fn is_demon(&self, unit: UnitId) -> bool {
        self.h.is_demon(unit)
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.h.is_undead(unit)
    }
    fn area_units(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
    ) -> Vec<UnitId> {
        with_sim(self, game, |h, sim| {
            X::missile_area_units(h, sim, owner, at, r, f, false)
        })
    }
    /// `scan_unit(…, noaura)` on the same pipeline: with `noaura` 1 the
    /// scan skips monsters whose monstats `noAura` is set.
    fn scan_units(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
        noaura: bool,
    ) -> Vec<UnitId> {
        with_sim(self, game, |h, sim| {
            X::missile_area_units(h, sim, owner, at, r, f, noaura)
        })
    }
    /// The area hit on the combat view.
    fn area_hit(&mut self, game: &mut Game, owner: UnitId, unit: UnitId, record: &DamageRecord) {
        let t = self.h.tables.clone();
        let mut cv = self.combat(game);
        crate::missiles::bodies::area_hit(&mut cv, &t.combat, owner, unit, record);
    }
    /// The active room's seed on the DRLG.
    fn room_seed(&mut self, _: &mut Game, room: RoomId) -> Option<&mut crate::rng::Seed> {
        self.h.drlg.active_seed_mut(room)
    }
    /// The line test (`room`, `from`, `to`, `mask`) on the DRLG's
    /// collision grids: result 1 (blocked).
    fn line_hits(
        &self,
        _: &Game,
        room: RoomId,
        from: (i32, i32),
        to: (i32, i32),
        mask: u16,
    ) -> bool {
        use crate::path::Point;
        crate::path::line::line_test(
            &self.h.drlg,
            Some(room),
            Point::new(from.0, from.1),
            Point::new(to.0, to.1),
            mask,
        )
        .blocked()
    }
    /// Path target point with the path
    /// provider ([`crate::wiring::path::missiles`]); (0, 0) without it.
    fn path_target_point(&self, unit: UnitId) -> (i32, i32) {
        self.path_target_xy(unit).unwrap_or((0, 0))
    }
    /// The target position with the path provider; none without it.
    fn target_position(&mut self, game: &Game, unit: UnitId) -> Option<(i32, i32)> {
        self.path_target_position(game, unit).flatten()
    }
    /// Sets the path type with the path provider.
    fn set_path_type(&mut self, unit: UnitId, ty: i32) {
        let _ = View::path_set_type(self, unit, ty);
    }
    /// Sets the step counts with the path provider.
    fn set_path_distance(&mut self, unit: UnitId, d: i32) {
        let _ = self.path_set_step_counts(unit, d);
    }
    /// The 64-direction toward a point with the path provider; 0 without it.
    fn dir64(&self, unit: UnitId, at: (i32, i32)) -> i32 {
        self.path_dir64(unit, at).unwrap_or(0)
    }
    /// Path point i with the path provider.
    fn set_path_point(&mut self, unit: UnitId, i: i32, at: (u16, u16)) {
        let _ = self.path_set_point(unit, i, at);
    }
    /// Sets the path point count with the path provider.
    fn set_path_point_count(&mut self, unit: UnitId, n: i32) {
        let _ = self.path_set_point_count(unit, n);
    }
    /// Teleports with the path provider.
    fn path_teleport(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        room: Option<RoomId>,
        x: i32,
        y: i32,
    ) {
        let _ = self.path_teleport_to(game, unit, room, x, y);
    }
    /// The active room's sub-tile rectangle on the DRLG.
    fn room_subtiles(&self, _: &Game, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        use crate::path::CollisionRooms;
        self.h.drlg.subtile_rect(room).map(|t| (t.x, t.y, t.w, t.h))
    }
    /// The states count.
    fn states_count(&self) -> i32 {
        i32::try_from(self.stats.data().states.count()).unwrap_or(i32::MAX)
    }
    /// The expiry of the unit's list of `s`.
    fn state_list_expiry(&self, unit: UnitId, s: i32) -> Option<i32> {
        let l = self.state_list(unit, u16::try_from(s).ok()?)?;
        Some(self.stats.expire(l))
    }
    /// A new state list: flags 2, expiry `expire` and `owner`'s type and
    /// GUID, state `s`, the default state-off remove callback, attached
    /// with reset 1, then state `s` on and the unit queued for update.
    fn new_state_list(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        s: i32,
        expire: i32,
        owner: UnitId,
    ) -> bool {
        let Ok(s) = u16::try_from(s) else {
            return false;
        };
        let Some((ty, guid)) = game.lists.unit(owner).map(|e| (e.ty, e.guid)) else {
            return false;
        };
        let l = self.stats.alloc(2, expire, ty.index() as u32, guid);
        self.stats.set_state(l, u32::from(s));
        self.stats.set_remove_callback(
            l,
            Some(crate::wiring::interaction::skill_use::DELAY_REMOVE_CALLBACK),
        );
        self.stats.attach(&mut *self.h, unit, l, true);
        self.set_state(unit, s, true);
        if let Err(e) = game.lists.queue_update(unit) {
            self.unit_error(crate::game::GameError::from(e).into());
        }
        true
    }
    /// Fills the unit's list of `s` for `skill` at `level`: `aurastat1–6` evaluated on `unit`
    /// with `aurastatcalc1–6`, each non-zero value set in the list (attack rate also as the
    /// other anim rate), then the anim rate refreshed. Nothing without a list.
    fn aura_fill(&mut self, game: &mut Game, unit: UnitId, s: i32, skill: i32, level: i32) {
        use crate::combat::CombatWorld;
        let Some(l) = u16::try_from(s).ok().and_then(|s| self.state_list(unit, s)) else {
            return;
        };
        let t = self.h.tables.clone();
        let Some(r) = t.skills.skill(skill) else {
            return;
        };
        let pairs = [
            (r.aurastat1, r.aurastatcalc1),
            (r.aurastat2, r.aurastatcalc2),
            (r.aurastat3, r.aurastatcalc3),
            (r.aurastat4, r.aurastatcalc4),
            (r.aurastat5, r.aurastatcalc5),
            (r.aurastat6, r.aurastatcalc6),
        ];
        let mut cv = self.combat(game);
        for (st, calc) in pairs {
            let st = i32::from(st as i16);
            if !(0..t.skills.stat_count).contains(&st) {
                continue;
            }
            let v = crate::skills::eval_skill(&mut cv, &t.skills, Some(unit), calc, skill, level);
            if v != 0 {
                cv.v.set_list_stat(l, st as u16, v);
                // Stats 68 (attackrate) and 69 (other_animrate).
                if st == 68 {
                    cv.v.set_list_stat(l, 69, v);
                }
            }
        }
        cv.refresh_anim_rate(unit);
    }
    /// Sets the state-changed bit.
    fn mark_state_changed(&mut self, unit: UnitId, s: i32) {
        if let Ok(s) = u32::try_from(s) {
            self.stats.set_state_changed(unit, s, true);
        }
    }
    /// Sets the expiry of the unit's list of `s`.
    fn set_state_list_expiry(&mut self, unit: UnitId, s: i32, expire: i32) {
        if let Some(l) = u16::try_from(s).ok().and_then(|s| self.state_list(unit, s)) {
            self.stats.set_expire(l, expire);
        }
    }
}

/// Runs `f` on the hooks and a [`crate::units::hooks::Sim`] over the
/// view's parts and `game`.
pub(super) fn with_sim<X: Pending, R>(
    v: &mut View<'_, X>,
    game: &mut Game,
    f: impl FnOnce(&mut super::ActionHooks<X>, &mut crate::units::hooks::Sim<'_>) -> R,
) -> R {
    let mut sim = crate::units::hooks::Sim {
        game,
        units: &mut *v.units,
        stats: &mut *v.stats,
        data: v.data,
    };
    f(&mut *v.h, &mut sim)
}
