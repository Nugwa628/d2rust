//! The unit path record and the path seams of the action
//! adapters that need no game: position, room, size and shape
//! (the path-placement inputs), footprints, the path setters of the missile set-up,
//! the cached collision word and the crossed
//! sub-tiles, the path part of `SUNIT_Add` at allocation
//! and the path free at removal.
//!
//! Each method answers from [`super::PathState`] when the provider is on
//! and from [`Pending`] otherwise (the module doc of [`super`]).

use crate::game::Game;
use crate::path::footprint::{
    add_footprint, make_corpse_footprint, remove_footprint, reset_pattern, set_foot_mask,
};
use crate::path::record::{alloc_dynamic_path, TargetUnit};
use crate::path::{
    DynamicKind, FootShape, Footprint, MonsterShape, RemoveRule, StaticPath, UnitPath, UnitShape,
};
use crate::units::{UnitId, UnitType};
use crate::wiring::action::{ActionHooks, Pending, View, WiringError};

/// monstats `Velocity` and the `npc` bit, the walk velocity source of a
/// monster.
pub type MonsterVelocity = (i32, bool);

impl<X: Pending> ActionHooks<X> {
    /// The objects.txt shape of an object class for the footprint
    /// (size and collision masks); `None` for another unit type or
    /// without the object state.
    pub(crate) fn object_footprint_shape(
        &self,
        ty: Option<UnitType>,
        class: u32,
    ) -> Option<crate::path::ObjectShape> {
        if ty != Some(UnitType::Object) {
            return None;
        }
        let st = self.objects.as_ref()?;
        let o = st.tables.object(u16::try_from(class).ok()?).ok()?;
        Some(crate::wiring::action::objects::object_shape(o))
    }

    /// Position in sub-tiles: the path's; a unit without a path
    /// reads (0, 0).
    pub fn path_position(&self, unit: UnitId) -> (i32, i32) {
        match &self.paths {
            Some(p) => crate::path::record::unit_position(p.record(unit)),
            None => self.x.position(unit),
        }
    }

    /// The unit has a path.
    pub fn path_has(&self, unit: UnitId) -> bool {
        match &self.paths {
            Some(p) => p.record(unit).is_some(),
            None => self.x.has_path(unit),
        }
    }

    /// Path velocity (+0x7C).
    pub fn path_velocity(&self, unit: UnitId) -> i32 {
        match &self.paths {
            Some(p) => p.dynamic(unit).map_or(0, |d| d.velocity),
            None => self.x.velocity(unit),
        }
    }

    /// Sets the path velocity.
    pub fn path_set_velocity(&mut self, unit: UnitId, v: i32) {
        match &mut self.paths {
            Some(p) => {
                if let Some(d) = p.dynamic_mut(unit) {
                    crate::path::walk::velocity::set_velocity(d, v);
                }
            }
            None => self.x.set_velocity(unit, v),
        }
    }

    /// Sets the target point; the target unit is cleared.
    pub fn path_set_target_point(&mut self, unit: UnitId, x: i32, y: i32) {
        match &mut self.paths {
            Some(p) => {
                if let Some(d) = p.dynamic_mut(unit) {
                    d.set_target_point(x as u16, y as u16);
                }
            }
            None => self.x.set_target_point(unit, x, y),
        }
    }

    /// Sets the move-test mask (which collision bits stop the unit).
    pub fn path_set_move_mask(&mut self, unit: UnitId, mask: u16) {
        match &mut self.paths {
            Some(p) => {
                if let Some(d) = p.dynamic_mut(unit) {
                    d.move_mask = mask;
                }
            }
            None => self.x.set_move_mask(unit, mask),
        }
    }

    /// Path acceleration and maximum velocity
    /// (the missile set-up's accelerating missiles).
    pub fn path_set_acceleration(&mut self, unit: UnitId, accel: i32, max_velocity: i32) {
        match &mut self.paths {
            Some(p) => {
                if let Some(d) = p.dynamic_mut(unit) {
                    d.acceleration = accel;
                    d.max_velocity = max_velocity;
                }
            }
            None => self.x.set_acceleration(unit, accel, max_velocity),
        }
    }

    /// The collision word the last step cached in the path (the
    /// collided-with mask, read by the missile collision step).
    pub fn path_cached_word(&self, unit: UnitId) -> Option<u16> {
        match &self.paths {
            Some(p) => p.dynamic(unit).map(|d| d.collided_mask),
            None => self.x.cached_collision_word(unit),
        }
    }

    /// The sub-tiles the last step crossed, in path order
    /// (the saved steps).
    pub fn path_crossed(&self, unit: UnitId) -> Vec<(i32, i32)> {
        match &self.paths {
            Some(p) => p.dynamic(unit).map_or_else(Vec::new, |d| {
                d.saved_steps
                    .iter()
                    .take(d.saved_count as usize)
                    .map(|s| (i32::from(s.x), i32::from(s.y)))
                    .collect()
            }),
            None => self.x.crossed_subtiles(unit),
        }
    }

    /// The path free at removal (the path is freed with the unit; types 0–3
    /// also remove their footprint). Removal clears
    /// the footprint with force, so the
    /// mode conditions on footprint removal do not apply.
    pub(crate) fn path_free(&mut self, unit: UnitId, ty: Option<UnitType>, class: u32, mode: u32) {
        let object = self.object_footprint_shape(ty, class);
        let Some(p) = self.paths.as_mut() else {
            return;
        };
        if matches!(
            ty,
            Some(UnitType::Player | UnitType::Monster | UnitType::Object | UnitType::Missile)
        ) {
            if let Some((fp, _)) = footprint_of(p.record(unit), ty, mode, object) {
                remove_footprint(&mut self.drlg, &fp, RemoveRule::Other, true);
            }
        }
        p.records.remove(&unit);
        p.history.remove(&unit);
        p.setup.remove(&unit);
    }
}

/// The footprint of a unit with path `rec` (players and monsters
/// by pattern, missiles, items and tiles by size, objects by box) and
/// its removal rule. Objects need their objects.txt shape; `None` when
/// the shape is unknown or the unit has no path.
pub(crate) fn footprint_of(
    rec: Option<&UnitPath>,
    ty: Option<UnitType>,
    mode: u32,
    object: Option<crate::path::ObjectShape>,
) -> Option<(Footprint, RemoveRule)> {
    let rec = rec?;
    let ty = ty?;
    let (room, (x, y)) = (rec.room(), rec.position());
    let fp = |shape, mask| Footprint {
        room,
        x,
        y,
        shape,
        mask,
    };
    Some(match (ty, rec) {
        (UnitType::Player, UnitPath::Dynamic(d)) => (
            fp(FootShape::Pattern(d.pattern), d.foot_mask),
            RemoveRule::Player { mode },
        ),
        (UnitType::Monster, UnitPath::Dynamic(d)) => (
            fp(FootShape::Pattern(d.pattern), d.foot_mask),
            RemoveRule::Monster { mode },
        ),
        (UnitType::Missile, UnitPath::Dynamic(d)) => (
            fp(FootShape::Size(d.unit_size), d.foot_mask),
            RemoveRule::Other,
        ),
        (UnitType::Object, UnitPath::Static(_)) => {
            let o = object?;
            (
                fp(
                    FootShape::Box {
                        size_x: o.size_x,
                        size_y: o.size_y,
                    },
                    o.foot_mask(),
                ),
                RemoveRule::Object { mode, shape: o },
            )
        }
        (UnitType::Item, UnitPath::Static(_)) => (
            fp(
                FootShape::Size(UnitShape::Item.size()),
                UnitShape::Item.foot_mask(None),
            ),
            RemoveRule::Other,
        ),
        (UnitType::Tile, UnitPath::Static(_)) => (
            fp(
                FootShape::Size(UnitShape::Tile.size()),
                UnitShape::Tile.foot_mask(None),
            ),
            RemoveRule::Other,
        ),
        _ => return None,
    })
}

impl<X: Pending> View<'_, X> {
    /// The size and shape inputs of a unit: monstats / monstats2 of a monster
    /// (`SizeX` signed, `BaseId`, `flying`, `opendoors`, `npc`, `inTown`,
    /// `interact`, unit flag bit 31), missiles `Size`. Objects: `None`.
    ///
    /// TODO: objects.txt is not in `ActionTables`, so object sizes and
    /// footprint masks (`SizeX`, `SizeY`, `IsDoor`, `BlocksVis`,
    /// `BlockMissile`, `SubClass`, `HasCollision0..7`) have no table
    /// here; objects get a static path without a footprint.
    pub fn path_shape(&self, unit: UnitId) -> Option<UnitShape> {
        let r = self.units.get(unit)?;
        Some(match r.ty {
            UnitType::Player => UnitShape::Player,
            UnitType::Monster => UnitShape::Monster(self.monster_shape(unit)?),
            UnitType::Missile => UnitShape::Missile {
                size: i32::from(self.h.tables.missiles.get(r.class as usize)?.size),
            },
            UnitType::Item => UnitShape::Item,
            UnitType::Tile => UnitShape::Tile,
            UnitType::Object => {
                UnitShape::Object(self.h.object_footprint_shape(Some(r.ty), r.class)?)
            }
        })
    }

    /// A monster's size and shape inputs.
    pub fn monster_shape(&self, unit: UnitId) -> Option<MonsterShape> {
        let r = self.units.get(unit)?;
        let t = &self.h.tables.combat;
        let m = t.monstats.get(r.class as usize)?;
        let size_x = t
            .monstats2
            .get(usize::from(m.monstatsex))
            .map_or(0, |m2| i32::from(m2.sizex as i8));
        Some(MonsterShape {
            size_x,
            base_id: u32::from(m.baseid),
            flying: m.flying,
            open_doors: m.opendoors,
            npc: m.npc,
            in_town: m.intown,
            unit_flag_31: r.flags & 0x8000_0000 != 0,
            interact: m.interact,
        })
    }

    /// Unit size; without the provider [`Pending::size`].
    pub fn path_size(&self, unit: UnitId) -> i32 {
        if self.h.paths.is_none() {
            return self.h.x.size(unit);
        }
        // An object's size is its `objects.txt` `SizeX`: a walk to a sized
        // target lifts the target's own footprint, so the run to the stash
        // aims at the stash's own cell.
        if let Some(r) = self.units.get(unit).filter(|r| r.ty == UnitType::Object) {
            return self
                .h
                .object_footprint_shape(Some(r.ty), r.class)
                .map_or(0, |s| s.size_x as i32);
        }
        self.path_shape(unit).map_or(0, |s| s.size())
    }

    /// Sets the target unit, its type and GUID.
    pub fn path_set_target_unit(&mut self, unit: UnitId, target: UnitId) {
        let Some(p) = self.h.paths.as_mut() else {
            self.h.x.set_target_unit(unit, target);
            return;
        };
        let Some(t) = self.units.get(target) else {
            return;
        };
        let tu = TargetUnit {
            unit: target,
            ty: t.ty,
            guid: t.guid,
        };
        if let Some(d) = p.dynamic_mut(unit) {
            d.target_unit = Some(tu);
        }
    }

    /// Footprint mask change with restamp
    /// (missiles by size).
    pub fn path_set_foot_mask(&mut self, unit: UnitId, mask: u16) {
        let missile = self
            .units
            .get(unit)
            .is_some_and(|r| r.ty == UnitType::Missile);
        let h = &mut *self.h;
        let Some(p) = h.paths.as_mut() else {
            h.x.set_footprint_mask(unit, mask);
            return;
        };
        if let Some(d) = p.dynamic_mut(unit) {
            set_foot_mask(&mut h.drlg, d, missile, mask);
        }
    }

    /// The path part of adding a unit to the world for a unit just
    /// allocated at (x, y) in its list room (the
    /// the list part — room list, hash, update queue — is
    /// `UnitLists::add_unit`'s, already done). Without the provider:
    /// [`Pending::place`].
    ///
    /// Per type: player, monster, missile: dynamic path allocation
    /// (stamps the footprint when a room is given); item in mode 3
    /// and tile: static set and footprint; object: static set (footprint:
    /// see [`View::path_shape`]); item in another mode: no path.
    ///
    /// The three dynamic allocations pass `set0x10` = 0.
    // The monster calls after the allocation (the monster's added hooks)
    // run in [`View::add_allocated`]
    // (`monster_added`). The corpse path settings
    // run at the end (player mode 0 / 17, monster mode 0 / 12
    // without monstats2 `deadCol`).
    pub fn path_place(&mut self, game: &Game, unit: UnitId, x: i32, y: i32) {
        if self.h.paths.is_none() {
            self.h.x.place(unit, x, y);
            return;
        }
        let Some(r) = self.units.get(unit) else {
            return;
        };
        let (ty, mode) = (r.ty, r.mode);
        let room = game.lists.unit(unit).and_then(|e| e.room());
        let kind = match ty {
            UnitType::Player => Some(DynamicKind::Player),
            UnitType::Monster => self.monster_shape(unit).map(DynamicKind::Monster),
            UnitType::Missile => match self.path_shape(unit) {
                Some(UnitShape::Missile { size }) => Some(DynamicKind::Missile { size }),
                _ => None,
            },
            _ => None,
        };
        // A corpse keeps a footprint unless monstats2 flag 19
        // (`deadCol`) is set.
        let dead_body = match ty {
            UnitType::Player => matches!(mode, 0 | 17),
            UnitType::Monster => {
                matches!(mode, 0 | 12)
                    && !self
                        .units
                        .get(unit)
                        .and_then(|r| self.h.tables.combat.monstats.get(r.class as usize))
                        .and_then(|m| {
                            self.h
                                .tables
                                .combat
                                .monstats2
                                .get(usize::from(m.monstatsex))
                        })
                        .is_some_and(|m2| m2.deadcol)
            }
            _ => false,
        };
        let object = self
            .h
            .object_footprint_shape(Some(ty), self.units.get(unit).map_or(0, |r| r.class));
        let h = &mut *self.h;
        let p = h.paths.as_mut().expect("checked above");
        let rec = match (ty, kind) {
            (_, Some(kind)) => {
                match alloc_dynamic_path(&p.tables, &mut h.drlg, kind, unit, room, x, y, false) {
                    Ok(d) => UnitPath::Dynamic(Box::new(d)),
                    Err(e) => {
                        h.errors.push(WiringError::Path(e));
                        return;
                    }
                }
            }
            (UnitType::Item, _) if mode != 3 => return,
            (UnitType::Object | UnitType::Item | UnitType::Tile, _) => {
                let mut s = StaticPath::default();
                s.set(room, x, y);
                let rec = UnitPath::Static(s);
                // `SUNIT_Add`: an object's footprint only when
                // `HasCollision[mode]` is set.
                if ty != UnitType::Object || object.is_some_and(|o| o.collides_in(mode)) {
                    if let Some((fp, _)) = footprint_of(Some(&rec), Some(ty), mode, object) {
                        add_footprint(&mut h.drlg, &fp);
                    }
                }
                rec
            }
            // A monster class without a monstats row or a missile class
            // without a missiles row: no path.
            _ => return,
        };
        p.records.insert(unit, rec);
        if dead_body {
            if let Some(d) = p.dynamic_mut(unit) {
                make_corpse_footprint(&mut h.drlg, d);
            }
        }
    }

    /// The unit's footprint and removal rule (the footprint add and
    /// remove inputs).
    pub(crate) fn path_footprint(&self, unit: UnitId) -> Option<(Footprint, RemoveRule)> {
        let r = self.units.get(unit)?;
        let p = self.h.paths.as_ref()?;
        let object = self.h.object_footprint_shape(Some(r.ty), r.class);
        footprint_of(p.record(unit), Some(r.ty), r.mode, object)
    }

    /// Footprint add at the unit's stored position.
    pub fn path_add_footprint(&mut self, unit: UnitId) {
        if let Some((fp, _)) = self.path_footprint(unit) {
            add_footprint(&mut self.h.drlg, &fp);
        }
    }

    /// Footprint remove `(unit, force)` at the stored position;
    /// whether it cleared.
    pub fn path_remove_footprint(&mut self, unit: UnitId, force: bool) -> bool {
        match self.path_footprint(unit) {
            Some((fp, rule)) => remove_footprint(&mut self.h.drlg, &fp, rule, force),
            None => false,
        }
    }

    /// The death clean-up's footprint step:
    /// the class's monstats2 flag 0x13 (`deadCol`) clear → the dead-body
    /// footprint `(U, 1)`: the
    /// footprint removed (forced), a 3 × 3 box of mask 0x8000 stamped at
    /// the path position, the path snapped to its sub-tile centre. A unit
    /// without a dynamic path: nothing.
    pub fn death_footprint(&mut self, unit: UnitId) {
        let dead_col = self
            .units
            .get(unit)
            .and_then(|r| self.h.tables.combat.monstats.get(r.class as usize))
            .and_then(|m| {
                self.h
                    .tables
                    .combat
                    .monstats2
                    .get(usize::from(m.monstatsex))
            })
            .is_some_and(|m2| m2.deadcol);
        if dead_col {
            return;
        }
        self.dead_body_footprint(unit);
    }

    /// The dead-body footprint `(U, 1)` without the class
    /// gate: the player's dead clean-up calls it directly.
    pub fn dead_body_footprint(&mut self, unit: UnitId) {
        let Some((room, x, y)) = self
            .h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(unit))
            .map(|d| (d.room, d.x(), d.y()))
        else {
            return;
        };
        self.path_remove_footprint(unit, true);
        crate::path::collision::box_apply(&mut self.h.drlg, room, x, y, (3, 3), 0x8000, true);
        if let Some(d) = self.h.paths.as_mut().and_then(|p| p.dynamic_mut(unit)) {
            d.precise_x = crate::path::coords::to_fp16_center(x);
            d.precise_y = crate::path::coords::to_fp16_center(y);
        }
    }

    /// Path reset: the
    /// pattern recomputed from the stored size and restamped. A unit
    /// without a dynamic path or without the provider: nothing.
    pub fn path_reset(&mut self, unit: UnitId) {
        let Some(shape) = self.path_shape(unit) else {
            return;
        };
        let h = &mut *self.h;
        let Some(p) = h.paths.as_mut() else {
            return;
        };
        let tables = p.tables.clone();
        if let Some(d) = p.dynamic_mut(unit) {
            reset_pattern(&mut h.drlg, &tables, d, &shape);
        }
    }

    /// monstats `Velocity` and `npc` of a monster.
    pub fn monster_velocity(&self, unit: UnitId) -> MonsterVelocity {
        self.units
            .get(unit)
            .and_then(|r| self.h.tables.combat.monstats.get(r.class as usize))
            .map_or((0, false), |m| (i32::from(m.velocity), m.npc))
    }

    /// The velocity base `(type, class)` of the unit's draw identity:
    /// with flag-ex disguise and a gfx state on, that state's `gfxtype`
    /// (1: a monster, 2: a player) and `gfxclass`; else the unit's own
    /// type and class. A player identity reads charstats `WalkVelocity`,
    /// a monster one monstats `Velocity` (the Shadow Warrior,
    /// monstats `Velocity` 0, walks on the assassin's 6; a shapeshifted
    /// Druid on its wolf or bear row).
    pub fn velocity_base(&self, unit: UnitId) -> i32 {
        let Some(r) = self.units.get(unit) else {
            return 0;
        };
        let (mut player, mut class) = (r.ty == UnitType::Player, r.class as usize);
        if r.flags2 & crate::units::record::flags2::DISGUISE != 0 {
            let states = &self.stats.data().states;
            let shown = states
                .gfx_states()
                .iter()
                .find(|&&(s, ..)| self.stats.has_state(unit, s))
                .copied();
            match shown {
                Some((_, 1, c)) => (player, class) = (false, usize::from(c)),
                Some((_, 2, c)) => (player, class) = (true, usize::from(c)),
                _ => {}
            }
        }
        let combat = &self.h.tables.combat;
        if player {
            combat
                .charstats
                .get(class)
                .map_or(0, |c| i32::from(c.walkvelocity))
        } else {
            combat
                .monstats
                .get(class)
                .map_or(0, |m| i32::from(m.velocity))
        }
    }

    /// charstats `WalkVelocity`, `RunVelocity`, `RunDrain` of a player.
    pub fn charstats_velocity(&self, unit: UnitId) -> (i32, i32, i32) {
        self.units
            .get(unit)
            .and_then(|r| self.h.tables.combat.charstats.get(r.class as usize))
            .map_or((0, 0, 0), |c| {
                (
                    i32::from(c.walkvelocity),
                    i32::from(c.runvelocity),
                    i32::from(c.rundrain),
                )
            })
    }
}

impl<X: Pending> View<'_, X> {
    /// Whether the line from `a` to `b` is blocked for `mask`
    /// ([`crate::path::line::units_line_blocked`]) on the path records and
    /// the DRLG rooms: `a`'s room, the path positions
    /// and sizes. `None` without the path provider (the
    /// caller keeps its [`Pending`] answer).
    pub fn units_line_blocked(&self, game: &Game, a: UnitId, b: UnitId, mask: u16) -> Option<bool> {
        Some(self.units_line_block_point(game, a, b, mask)?.is_some())
    }

    /// [`Self::units_line_blocked`] with the blocking cell (`Some(None)`:
    /// the line is clear).
    pub fn units_line_block_point(
        &self,
        game: &Game,
        a: UnitId,
        b: UnitId,
        mask: u16,
    ) -> Option<Option<crate::path::Point>> {
        self.h.paths.as_ref()?;
        let end = |u: UnitId| {
            let (x, y) = self.h.path_position(u);
            crate::path::line::LineUnit {
                room: game.lists.unit(u).and_then(|e| e.room()),
                x,
                y,
                size: self.path_size(u),
            }
        };
        Some(crate::path::line::units_line_block_point(
            &self.h.drlg,
            &end(a),
            &end(b),
            mask,
        ))
    }
}
