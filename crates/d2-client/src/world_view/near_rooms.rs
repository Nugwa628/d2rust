//! The draw order's map-tile feed from the client DRLG: the
//! near-room array of the local player's active room (its adjacency array),
//! per room the tile rectangle, sub-tile origin, the three
//! record arrays of its tile data with each record's DT1 header fields
//! (the entry identity: roof height, height) and coordinate
//! record, and the room's unit list in the
//! client's list order.
//!
//! The draw writes record flags (0x20000, the fade bits) and fade bytes
//! back into the records, where they persist between frames: [`MapState`]
//! keeps them per (room, array, record) while the room stays active, and
//! the units' flag 0x10000000 and flag-ex 0x80 per unit while it lives
//! (the sight test of frame N gates the shadow
//! entry of frame N + 1). The
//! fill's Y sort of each room's unit list
//! persists in the client's list: [`MapState::take_unit_orders`] hands
//! the sorted orders to the bridge.

use std::collections::BTreeMap;

use crate::bridge::drlg::DrlgRoomId;
use crate::bridge::world::{ClientWorld, LevelRow, UnitKey};
use crate::bridge::ClientUnit;
use crate::rules::draw_order::{
    Dt1Facts, Fade, LevelFacts, Logical, NearRooms, Room, RoomUnit, TileArray, TileRecord,
    TileRect, UnitFacts, REC_HIDDEN, UNIT_DRAWN, UNIT_EX_VISIBLE,
};

use super::ViewError;

const SPEC: &str = "render/draw-order.md";

/// A record's DT1 entry: the file's path and the tile index in file order
/// (the entry identity), the key of its block data.
pub type Dt1Entry = (Vec<u8>, u32);

/// The draw state of one record that persists between frames: flags
/// (+0x14) and fade bytes (+0x24…+0x2C).
type DrawState = (u32, Fade);

/// The map part of the model feed: the frame's near rooms and the draw
/// state the records keep between frames.
#[derive(Debug, Clone, Default)]
pub struct MapState {
    /// The bridge frame the near rooms were built for.
    stamp: Option<u64>,
    near: Option<NearRooms>,
    /// The DRLG room of each near room, array order.
    rooms: Vec<DrlgRoomId>,
    /// Per near room, per array (wall, floor, shadow), per record: its
    /// DT1 entry.
    entries: Vec<[Vec<Dt1Entry>; 3]>,
    draw: BTreeMap<(DrlgRoomId, u8, usize), DrawState>,
    /// The client DRLG the draw state belongs to: its act and init seed
    /// ([`Self::drlg_key`]). Room slots are reused by a new act's DRLG,
    /// so a change drops the draw state.
    drlg: Option<(u8, u32)>,
    /// The unit bits the order writes and the next frame reads:
    /// flag 0x10000000 and flag-ex 0x80 of
    /// each unit, as the last frame left them.
    unit_draw: BTreeMap<UnitKey, (u32, u32)>,
    /// The DT1 entry of the act's edge floor record,
    /// `None` in acts IV and V.
    edge: Option<Dt1Entry>,
    /// How many of the client DRLG's automap callback rooms
    /// (`Drlg::automap_rooms`) were handed out ([`Self::automap_rooms`]);
    /// reset with the DRLG.
    automap_taken: usize,
}

fn array_slot(a: TileArray) -> u8 {
    match a {
        TileArray::Wall => 0,
        TileArray::Floor => 1,
        TileArray::Shadow => 2,
        TileArray::Edge => 3,
    }
}

fn unresolved(what: &'static str, message: String) -> ViewError {
    ViewError::Unresolved {
        what,
        spec: SPEC,
        message,
    }
}

impl MapState {
    /// The near rooms of the frame `world.frames`, built once per bridge
    /// frame (the order mutates them during the frame). `None`: no client
    /// DRLG or the local player has no room (no map).
    pub fn near_rooms(
        &mut self,
        world: &ClientWorld,
        levels: Option<&[LevelRow]>,
        facts: impl Fn(&ClientUnit) -> Result<UnitFacts, ViewError>,
    ) -> Result<Option<&mut NearRooms>, ViewError> {
        if self.stamp != Some(world.frames) {
            let key = Self::drlg_key(world);
            if key != self.drlg {
                // A new client DRLG (an act change, 0x03): the old act's
                // records are freed with it, their flags and fades too.
                self.near = None;
                self.draw.clear();
                self.drlg = key;
                self.automap_taken = 0;
            }
            self.save();
            // The perf report: a rebuild that brings in another set of near
            // rooms (walking into new rooms, a warp) is a `room` event.
            let perf =
                d2_server::perf::enabled().then(|| (std::time::Instant::now(), self.rooms.clone()));
            self.rebuild(world, levels, facts)?;
            if let Some((t0, old)) = perf {
                if old != self.rooms {
                    d2_server::perf::span_since("room", t0);
                }
            }
            self.stamp = Some(world.frames);
        }
        Ok(self.near.as_mut())
    }

    /// The identity of the model's client DRLG: (act, init seed).
    fn drlg_key(world: &ClientWorld) -> Option<(u8, u32)> {
        world.drlg.as_ref().map(|d| (d.drlg.act, d.drlg.init_seed))
    }

    /// The DT1 entry of a record of near room `room`.
    pub fn entry(&self, room: usize, array: TileArray, record: usize) -> Option<&Dt1Entry> {
        if array == TileArray::Edge {
            return self.edge.as_ref();
        }
        self.entries
            .get(room)?
            .get(usize::from(array_slot(array)))?
            .get(record)
    }

    /// The DRLG room of near room `room`.
    pub fn room(&self, room: usize) -> Option<DrlgRoomId> {
        self.rooms.get(room).copied()
    }

    /// The unit lists the frame's fill sorted,
    /// for the client's lists; each is handed over once.
    pub fn take_unit_orders(&mut self) -> Vec<(DrlgRoomId, Vec<UnitKey>)> {
        let Some(near) = self.near.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (ri, room) in near.rooms.iter_mut().enumerate() {
            if room.units_sorted {
                room.units_sorted = false;
                out.push((self.rooms[ri], room.units.iter().map(|u| u.key).collect()));
            }
        }
        out
    }

    /// Sets flag 0x10000000 only when
    /// the unit draw returns nonzero. The order set it for
    /// every unit it listed; the units of `keys` (the draw returned 0:
    /// no pose, `Draw` 0, or the COF box pre-test failed) get back the
    /// last frame's bit.
    pub fn units_undrawn(&mut self, keys: &std::collections::BTreeSet<UnitKey>) {
        let Some(near) = self.near.as_mut() else {
            return;
        };
        for u in near.rooms.iter_mut().flat_map(|r| r.units.iter_mut()) {
            if keys.contains(&u.key) {
                let last = self.unit_draw.get(&u.key).map_or(0, |&(f, _)| f);
                u.facts.flags = u.facts.flags & !UNIT_DRAWN | last;
            }
        }
    }

    /// The rooms the client DRLG passed to the automap callback +0x454
    /// since the
    /// last call, in call order, each with its records and their kept
    /// draw state and no units (the base game calls it while generating the level,
    /// before the server sends any unit of it). Hand the records' new
    /// flags back with [`Self::automap_marked`].
    pub fn automap_rooms(
        &mut self,
        world: &ClientWorld,
    ) -> Result<Vec<(DrlgRoomId, Room)>, ViewError> {
        let key = Self::drlg_key(world);
        if key != self.drlg {
            self.near = None;
            self.draw.clear();
            self.drlg = key;
            self.automap_taken = 0;
        }
        let Some(cd) = world.drlg.as_ref() else {
            return Ok(Vec::new());
        };
        let d = &cd.drlg;
        let new = d.automap_rooms.get(self.automap_taken..).unwrap_or(&[]);
        let mut out = Vec::with_capacity(new.len());
        for &id in new {
            // A room freed again before this frame read it has no
            // records left to mark.
            if d.active_room(id).is_none() {
                continue;
            }
            out.push((id, self.build_room(d, id)?.0));
        }
        self.automap_taken = d.automap_rooms.len();
        Ok(out)
    }

    /// The record flag bits `bits` the automap set on `room`'s records
    /// (0x40000) kept on DRLG room `id`'s records,
    /// also in this frame's near rooms when the room is one of them.
    pub fn automap_marked(&mut self, id: DrlgRoomId, room: &Room, bits: u32) {
        for (array, recs) in [
            (TileArray::Wall, &room.walls),
            (TileArray::Floor, &room.floors),
            (TileArray::Shadow, &room.shadows),
        ] {
            let slot = array_slot(array);
            for (i, r) in recs.iter().enumerate() {
                let set = r.flags & bits;
                if set == 0 {
                    continue;
                }
                self.draw
                    .entry((id, slot, i))
                    .or_insert((r.flags, r.fade))
                    .0 |= set;
            }
        }
        let Some(near) = self.near.as_mut() else {
            return;
        };
        let Some(ri) = self.rooms.iter().position(|&r| r == id) else {
            return;
        };
        let n = &mut near.rooms[ri];
        for (mine, theirs) in [
            (&mut n.walls, &room.walls),
            (&mut n.floors, &room.floors),
            (&mut n.shadows, &room.shadows),
        ] {
            for (m, t) in mine.iter_mut().zip(theirs) {
                m.flags |= t.flags & bits;
            }
        }
    }

    /// Keeps the last frame's record flags and fades, and the units' draw
    /// bits.
    fn save(&mut self) {
        let Some(near) = &self.near else {
            return;
        };
        for (ri, room) in near.rooms.iter().enumerate() {
            for u in &room.units {
                self.unit_draw.insert(
                    u.key,
                    (
                        u.facts.flags & UNIT_DRAWN,
                        u.facts.flag_ex & UNIT_EX_VISIBLE,
                    ),
                );
            }
            let id = self.rooms[ri];
            for (array, recs) in [
                (TileArray::Wall, &room.walls),
                (TileArray::Floor, &room.floors),
                (TileArray::Shadow, &room.shadows),
            ] {
                for (i, r) in recs.iter().enumerate() {
                    self.draw
                        .insert((id, array_slot(array), i), (r.flags, r.fade));
                }
            }
        }
    }

    /// One room's facts from the client DRLG: its
    /// tile rectangle, sub-tile origin and record arrays with the draw
    /// state the records keep; no units. Also each record's DT1 entry.
    fn build_room(
        &self,
        d: &d2_sim::drlg::Drlg,
        id: DrlgRoomId,
    ) -> Result<(Room, [Vec<Dt1Entry>; 3]), ViewError> {
        let a = d
            .active_room(id)
            .ok_or_else(|| unresolved("near rooms", format!("room {id:?} is not active")))?;
        let r = d.room(id);
        let level = d.level(r.level).id;
        let mut room = Room {
            level,
            tiles: TileRect {
                x: r.rect.x,
                y: r.rect.y,
                w: r.rect.w,
                h: r.rect.h,
            },
            subtile_origin: (a.subtiles.x, a.subtiles.y),
            ..Room::default()
        };
        let mut entries: [Vec<Dt1Entry>; 3] = Default::default();
        if let Some(t) = r.tiles() {
            for (array, recs) in [
                (TileArray::Wall, &t.walls),
                (TileArray::Floor, &t.floors),
                (TileArray::Shadow, &t.shadows),
            ] {
                let slot = array_slot(array);
                let mut out = Vec::with_capacity(recs.len());
                for (i, rec) in recs.iter().enumerate() {
                    let info = d.tile_info(rec.tile);
                    let (flags, fade) = self
                        .draw
                        .get(&(id, slot, i))
                        .copied()
                        .unwrap_or((rec.flags, Fade::OPAQUE));
                    // The hidden bit is the model's: the animated-tile
                    // tick sets and clears it.
                    let flags = flags & !REC_HIDDEN | rec.flags & REC_HIDDEN;
                    // Record +0x10: walls only.
                    let logical = (array == TileArray::Wall)
                        .then(|| d.wall_coord(id, rec.x, rec.y))
                        .flatten()
                        .map(|c| Logical {
                            x0: c.boxr[0],
                            y0: c.boxr[1],
                            index: c.index as i32,
                        });
                    out.push(TileRecord {
                        tile: (rec.x, rec.y),
                        flags,
                        ty: rec.kind,
                        dt1: Dt1Facts {
                            orientation: info.orientation,
                            main: info.main,
                            sub: info.sub,
                            roof_height: i32::from(info.roof_height),
                            height: info.height,
                            material: info.material,
                            light_direction: info.light_direction,
                        },
                        fade,
                        logical,
                    });
                    entries[usize::from(slot)]
                        .push((d.dt1_path(rec.tile).to_vec(), rec.tile.index));
                }
                match array {
                    TileArray::Wall => room.walls = out,
                    TileArray::Floor => room.floors = out,
                    TileArray::Shadow | TileArray::Edge => room.shadows = out,
                }
            }
        }
        Ok((room, entries))
    }

    fn rebuild(
        &mut self,
        world: &ClientWorld,
        levels: Option<&[LevelRow]>,
        facts: impl Fn(&ClientUnit) -> Result<UnitFacts, ViewError>,
    ) -> Result<(), ViewError> {
        self.near = None;
        self.rooms.clear();
        self.entries.clear();
        self.edge = None;
        let active = world.active_rooms.as_deref().unwrap_or(&[]);
        // A room no longer active has freed its records.
        self.draw
            .retain(|(id, _, _), _| active.iter().any(|a| a.room == *id));
        // A freed unit's bits go with it.
        self.unit_draw.retain(|k, _| world.view_unit(k).is_some());
        let (Some(cd), Some(own)) = (world.drlg.as_ref(), world.local_room().copied()) else {
            return Ok(());
        };
        let d = &cd.drlg;
        let mut rooms = Vec::new();
        // A neighbour that is not active (not yet built, or freed in the
        // frames of a level transition) has no records to draw: it is
        // left out of this frame's near rooms, never an error.
        for id in cd.view_rooms(own.room) {
            if d.active_room(id).is_none() {
                continue;
            }
            let (mut room, entries) = self.build_room(d, id)?;
            for &key in world.room_units.list(id) {
                let unit = world.view_unit(&key).ok_or_else(|| {
                    unresolved(
                        "room unit list",
                        format!("unit {key:?} listed but not in the model"),
                    )
                })?;
                let mut f = facts(&unit)?;
                // The bits of the last frame's
                // draw (the sight test's 0x80 gates this frame's shadow).
                if let Some(&(flags, ex)) = self.unit_draw.get(&key) {
                    f.flags = f.flags & !UNIT_DRAWN | flags;
                    f.flag_ex = f.flag_ex & !UNIT_EX_VISIBLE | ex;
                }
                room.units.push(RoomUnit { key, facts: f });
            }
            rooms.push(room);
            self.rooms.push(id);
            self.entries.push(entries);
        }
        // The player's path sub-tile / 5:
        // from the
        // player's room: none → 0, else the record index at the point.
        // The local player's own cell.
        let (sx, sy) = world.local_cell().unwrap_or((0, 0));
        let (sx, sy) = (i32::from(sx), i32::from(sy));
        let player_logical = match world.cell_lookup(&own, sx, sy) {
            None => 0,
            Some(r) => d.coord_at(r.room, sx, sy).map_or(-1, |c| c.index as i32),
        };
        let row = levels
            .ok_or_else(|| unresolved("level facts", "no Levels rows (DrawEdges)".into()))?
            .get(usize::from(own.level))
            .ok_or_else(|| {
                unresolved(
                    "level facts",
                    format!("level {} past the Levels rows", own.level),
                )
            })?;
        // The act's edge floor record (open question
        // 2): its lookup failing matters only to a level that draws edges.
        let edge = match cd.edge_tile() {
            Ok(t) => t,
            Err(m) if row.draw_edges => return Err(unresolved("act edge record", m)),
            Err(_) => None,
        };
        let edge = edge.map(|t| {
            self.edge = Some((t.path, t.index));
            // Filled at position 0 from a zeroed record.
            TileRecord {
                tile: (0, 0),
                flags: 0,
                ty: 0,
                dt1: Dt1Facts {
                    orientation: t.info.orientation,
                    main: t.info.main,
                    sub: t.info.sub,
                    roof_height: i32::from(t.info.roof_height),
                    height: t.info.height,
                    material: t.info.material,
                    light_direction: t.info.light_direction,
                },
                fade: Fade::OPAQUE,
                logical: None,
            }
        });
        self.near = Some(NearRooms {
            rooms,
            player_tile: (sx / 5, sy / 5),
            player_subtile: (sx, sy),
            edge,
            player_logical,
            level: LevelFacts {
                id: u32::from(own.level),
                draw_edges: row.draw_edges,
                fade_geometric: false,
            },
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::draw_order::{Room, RoomUnit};

    #[test]
    fn a_draw_that_returned_zero_keeps_the_last_drawn_bit() {
        let (a, b, c) = (UnitKey::new(2, 1), UnitKey::new(2, 2), UnitKey::new(2, 3));
        let unit = |key| RoomUnit {
            key,
            facts: UnitFacts {
                flags: UNIT_DRAWN | 2,
                ..UnitFacts::default()
            },
        };
        let mut m = MapState {
            near: Some(NearRooms {
                rooms: vec![Room {
                    units: vec![unit(a), unit(b), unit(c)],
                    ..Room::default()
                }],
                ..NearRooms::default()
            }),
            ..MapState::default()
        };
        // `b` was drawn last frame; `a` was not; `c` drew this frame.
        m.unit_draw.insert(a, (0, 0));
        m.unit_draw.insert(b, (UNIT_DRAWN, 0));
        m.units_undrawn(&[a, b].into_iter().collect());
        let flags: Vec<u32> = m.near.as_ref().unwrap().rooms[0]
            .units
            .iter()
            .map(|u| u.facts.flags)
            .collect();
        assert_eq!(flags, vec![2, UNIT_DRAWN | 2, UNIT_DRAWN | 2]);
    }
}
