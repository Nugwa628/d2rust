//! Population → DRLG rooms, collision and preset units; units:
//! [`PopWorld`] on [`WorldHost`]. Seeds are the real ones (game seed of
//! the action state, the active-room seed of the DRLG's active
//! room, the unit record's seed); rooms, boxes and collision are the act
//! DRLG's; preset units are the preset room's list. Queries the DRLG
//! code does not describe go to [`WorldPending`]. The DRLG data population
//! reads (coordinate lists, populated level and room count, warp points,
//! the kind-11 location) are the act DRLG's.

use crate::drlg::DrlgRoomId;
use crate::monsters::population::{CoordRect, PopWorld, PresetUnit, RoomBox, TileRec};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

use super::{WorldHost, WorldPending, WorldgenError};
use crate::drlg::CoordRec;
use crate::wiring::action::WiringError;

/// A DRLG coordinate record as population reads it: the clipped box
/// (+0x10), the node flag and the index.
fn coord_rect(c: &CoordRec) -> CoordRect {
    CoordRect {
        rect: c.clipped,
        node_flag: i32::from(c.node),
        index: c.index as i32,
    }
}

impl<X: WorldPending> WorldHost<'_, X> {
    /// The act and DRLG room of an active room.
    pub(super) fn drlg_room(&self, room: RoomId) -> Option<(u8, DrlgRoomId)> {
        let act = self.game.lists.room(room)?.act;
        let d = self.v.h.drlg.dungeon.acts.get(usize::from(act))?.as_ref()?;
        Some((act, d.drlg_room_of(room)?))
    }

    /// The room facts object population reads: L0, the populated level, the
    /// waypoint flags 0x30000, the dirt-path test
    /// (room type 1 and the outdoor room's dirt-path flag 0x80)
    /// and the sub-tile rect.
    pub(super) fn object_room_info(
        &self,
        room: RoomId,
    ) -> crate::world::objects::populate::RoomInfo {
        let b = self.room_box(room);
        let (waypoint, dirt_path) = self.drlg_room(room).map_or((false, false), |(act, r)| {
            let d = self.v.h.drlg.dungeon.acts[usize::from(act)].as_ref();
            let dr = d.map(|d| d.room(r));
            let waypoint =
                dr.is_some_and(|dr| dr.flags & crate::drlg::room_flags::ANY_WAYPOINT != 0);
            let outdoor = dr.is_some_and(|dr| dr.kind == crate::drlg::RoomKind::Outdoor);
            let dirt = outdoor
                && self
                    .w
                    .types
                    .borrow()
                    .act_outdoor(act)
                    .and_then(|o| o.room(r))
                    .is_some_and(|o| o.flags & 0x80 != 0);
            (waypoint, dirt)
        });
        crate::world::objects::populate::RoomInfo {
            room,
            level: self.room_level_id(room) as u32,
            populated: self.populated_level(room) as u32,
            waypoint,
            dirt_path,
            rect: (b.x, b.y, b.width, b.height),
        }
    }

    /// A scratch seed for a room without an active DRLG room (an error is
    /// logged with it).
    fn orphan_seed(&mut self, room: RoomId) -> &mut Seed {
        self.w.errors.push(WorldgenError::NoActiveRoom(room));
        self.v.h.orphan_seed = Seed::init();
        &mut self.v.h.orphan_seed
    }
}

impl<X: WorldPending> PopWorld for WorldHost<'_, X> {
    fn game_seed(&mut self) -> &mut Seed {
        &mut self.v.h.game_seed
    }

    /// The active-room seed.
    fn room_seed(&mut self, room: RoomId) -> &mut Seed {
        let active = self.drlg_room(room).filter(|&(act, r)| {
            self.v.h.drlg.dungeon.acts[usize::from(act)]
                .as_ref()
                .is_some_and(|d| d.active_room(r).is_some())
        });
        let Some((act, r)) = active else {
            return self.orphan_seed(room);
        };
        self.v.h.drlg.dungeon.acts[usize::from(act)]
            .as_mut()
            .and_then(|d| d.active_room_seed_mut(r))
            .expect("checked above")
    }

    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed {
        self.v.seed(unit)
    }

    /// The level id of the room's DRLG level.
    fn room_level(&self, room: RoomId) -> i32 {
        self.room_level_id(room)
    }

    /// The populated level: 0 for a
    /// null room or a flag-0x800000 room, else the level id.
    fn populated_level(&self, room: RoomId) -> i32 {
        self.v
            .h
            .drlg
            .drlg_room(self.game, room)
            .map_or(0, |(d, r)| d.populated_level(r) as i32)
    }

    /// The populated room count on the act's
    /// DRLG: allocates the level when absent. An act without a DRLG, or a
    /// DRLG error (logged), counts 0.
    fn populated_room_count(&mut self, act: u8, level: i32) -> i32 {
        let r = self.v.h.drlg.with_act(act, &mut self.game.lists, |d, svc| {
            d.populated_room_count(svc.data, svc.types, level as u32)
        });
        match r {
            Some(Ok(n)) => n as i32,
            Some(Err(e)) => {
                self.w
                    .errors
                    .push(WorldgenError::Wiring(WiringError::Drlg(e)));
                0
            }
            None => 0,
        }
    }

    /// The room's
    /// coordinate records in `next` order; empty without info (never the
    /// case for a room population visits).
    fn coord_list(&self, room: RoomId) -> Vec<CoordRect> {
        self.v
            .h
            .drlg
            .drlg_room(self.game, room)
            .and_then(|(d, r)| d.coord_first(r).map(|l| l.iter().map(coord_rect).collect()))
            .unwrap_or_default()
    }

    /// The room's coordinate record at the point.
    fn coord_at(&self, room: RoomId, x: i32, y: i32) -> Option<CoordRect> {
        let (d, r) = self.v.h.drlg.drlg_room(self.game, room)?;
        d.coord_at(r, x, y).as_ref().map(coord_rect)
    }

    /// The coordinate record index at the point: the room holding
    /// the point among the room and its adjacency array; none → 0; that
    /// room's record at the point → its index; null record → −1.
    // A column outside the room wraps within the
    // record block ([`crate::drlg::Drlg::coord_at`]); reads outside the
    // block cannot happen with real data and give a null record (−1) here. Only
    // population's spread search can pass such a point.
    fn coord_index_at(&self, room: RoomId, x: i32, y: i32) -> i32 {
        let Some(at) = self.v.h.drlg.find_room(self.game, room, x, y) else {
            return 0;
        };
        self.coord_at(at, x, y).map_or(-1, |c| c.index)
    }

    /// The active room's sub-tile rectangle.
    fn room_box(&self, room: RoomId) -> RoomBox {
        self.v
            .h
            .drlg
            .subtiles(self.game, room)
            .map_or(RoomBox::default(), |r| RoomBox {
                x: r.x,
                y: r.y,
                width: r.w,
                height: r.h,
            })
    }

    /// The
    /// warp-room centres of the room's level (sub-tiles); none for a room
    /// without a DRLG room (a fatal case that population never reaches).
    fn warp_points(&self, room: RoomId) -> Vec<(i32, i32)> {
        self.v
            .h
            .drlg
            .drlg_room(self.game, room)
            .map_or(Vec::new(), |(d, r)| d.warp_points(r).to_vec())
    }

    /// The spawn location `(act of the level, level id, kind)`:
    /// the spawn-room choice for the
    /// room's level with all its effects; tiles, (−1, −1) when no room
    /// was found. A DRLG error is logged and reads as none.
    fn spawn_location(&mut self, room: RoomId, kind: u8) -> Option<(i32, i32)> {
        let level = self.room_level_id(room) as u32;
        let act = crate::drlg::act_of_level(level);
        let r = self
            .v
            .h
            .drlg
            .with_act(act, &mut self.game.lists, |d, svc| {
                if u32::from(kind) == crate::drlg::level::KIND11_TILE {
                    d.kind11_location(svc, level)
                } else {
                    d.spawn_room(svc, level, u32::from(kind))
                        .map(|p| (p.x, p.y))
                }
            })?;
        match r {
            Ok(p) => Some(p),
            Err(e) => {
                self.w
                    .errors
                    .push(WorldgenError::Wiring(WiringError::Drlg(e)));
                None
            }
        }
    }

    /// The room holding the point among `room` and its
    /// adjacency array.
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.v.h.drlg.find_room(self.game, room, x, y)
    }

    /// Collision test on the act's collision grids (with the path provider,
    /// the same footprint shapes as the
    /// missile adapter: size 0, 1 point, 2 plus, 3 box, other 0xFFFF; a
    /// cell without a room reads 0x27).
    // TODO: without the path provider the sub-tile at
    // (x, y) is read for every size.
    fn collides(&self, room: RoomId, x: i32, y: i32, size: i32, mask: u16) -> bool {
        if self.v.h.paths.is_some() {
            return crate::path::collision::size_value(
                &self.v.h.drlg,
                Some(room),
                x,
                y,
                size,
                mask,
            ) != 0;
        }
        self.v.h.drlg.collision(self.game, room, x, y).unwrap_or(0) & mask != 0
    }

    /// The collision mask at a point (with the path
    /// provider).
    fn mask_at(&self, room: RoomId, x: i32, y: i32, mask: u16) -> bool {
        if self.v.h.paths.is_some() {
            return crate::path::collision::point_value(&self.v.h.drlg, Some(room), x, y, mask)
                != 0;
        }
        self.v.h.drlg.collision(self.game, room, x, y).unwrap_or(0) & mask != 0
    }

    /// The room's floor records (the tile
    /// records population scans), positions in level tiles.
    /// A record is water when its tile's DT1 material flags
    /// have bit 0x2 (monsters that cannot swim do not spawn there).
    fn tile_records(&self, room: RoomId) -> Vec<TileRec> {
        let Some((act, r)) = self.drlg_room(room) else {
            return Vec::new();
        };
        let Some(d) = self.v.h.drlg.dungeon.acts[usize::from(act)].as_ref() else {
            return Vec::new();
        };
        let dr = d.room(r);
        dr.tiles().map_or(Vec::new(), |t| {
            t.floors
                .iter()
                .map(|rec| TileRec {
                    water: d.tile_info(rec.tile).material & 2 != 0,
                    x: rec.x + dr.rect.x,
                    y: rec.y + dr.rect.y,
                })
                .collect()
        })
    }

    /// The preset room's unit list (room-relative sub-tiles,
    /// head first: mode, class, path,
    /// flags).
    fn preset_units(&self, room: RoomId) -> Vec<PresetUnit> {
        let Some((act, r)) = self.drlg_room(room) else {
            return Vec::new();
        };
        let types = self.w.types.borrow();
        let Some(p) = types.act_presets(act) else {
            return Vec::new();
        };
        p.room_units(r)
            .iter()
            .map(|u| PresetUnit {
                unit_type: u.unit_type as i32,
                mode: u.mode as u8,
                class: u.class,
                x: u.x,
                y: u.y,
                has_data: u.path.is_some(),
                done: u.flags & 1 != 0,
            })
            .collect()
    }

    /// Client count (room +0x78): the active room's client array.
    fn client_count(&self, room: RoomId) -> u32 {
        let Some((act, r)) = self.drlg_room(room) else {
            return 0;
        };
        self.v.h.drlg.dungeon.acts[usize::from(act)]
            .as_ref()
            .and_then(|d| d.active_room(r))
            .map_or(0, |a| a.clients.len() as u32)
    }

    /// The unit's room.
    fn unit_room(&self, unit: UnitId) -> Option<RoomId> {
        self.room_of(unit)
    }

    fn unit_position(&self, unit: UnitId) -> (i32, i32) {
        self.v.h.path_position(unit)
    }

    fn quest_flag(&self, flag: u8) -> bool {
        self.v.h.x.quest_flag(flag)
    }

    fn chaos_blocks_population(&self) -> bool {
        self.v.h.x.chaos_blocks_population()
    }

    /// Nearest free point with the path provider:
    /// the coarse free-box search with the arguments of its population
    /// caller (mask 0x3C01, size 1);
    /// without it [`WorldPending::nearest_free_point`].
    fn nearest_free_point(&self, room: RoomId, x: i32, y: i32) -> Option<(RoomId, i32, i32)> {
        if self.v.h.paths.is_none() {
            return self.v.h.x.nearest_free_point(room, x, y);
        }
        let mut p = crate::path::coords::Point::new(x, y);
        let r = crate::wiring::path::place::coarse_free_box(
            &self.v.h.drlg,
            room,
            &mut p,
            1,
            u32::from(crate::path::collision::masks::MONSTER_MOVE),
        )?;
        Some((r, p.x, p.y))
    }
}
