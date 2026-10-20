//! DRLG rooms, rooms-near arrays, statuses and activation: status lists, set/unset
//! handlers, propagation, streaming and the build sequence.
//!
//! A room's status says how close it is to a client (0 = the client is in it, up to 3, and
//! 4 = none). Moving a client changes reference counts on the room and its neighbours, and
//! the status handlers load data and build rooms as statuses fall.

use crate::rng::Seed;

use super::active::ActiveRoom;
use super::data::DrlgData;
use super::level::{BuildCursor, Drlg};
use super::logic::LogicInfo;
use super::seams::{LevelTypes, Services};
use super::tiles::RoomTiles;
use super::{is_town, room_flags, DrlgError, DrlgRoomId, LevelIdx, TileRect};
use crate::units::RoomId;

/// Room type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    /// 1: outdoor-grid room (outdoor and maze-style code).
    Outdoor,
    /// 2: preset (DS1) room.
    Preset,
    /// Any other value (no type data).
    Other(u32),
}

/// A warp link (prepended to a room's list): the linked room in the target level, enabled
/// flag, and the lvlwarp row of the slot (direction `'b'`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarpLink {
    /// The linked room in the target level.
    pub target: DrlgRoomId,
    /// Whether the link is enabled.
    pub enabled: bool,
    /// The lvlwarp row of the slot.
    pub lvlwarp_row: usize,
}

/// Where [`Drlg::link_room`] adds a room to its level's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkAt {
    /// Before the current first room.
    Head,
    /// After the current last room.
    Tail,
}

/// Status "none": the room is on no status list.
pub const STATUS_NONE: u8 = 4;

/// Near-gap limit: two rooms are near when both gaps are < 6.
pub const NEAR_GAP: i32 = 6;

/// The view gap of a game whose frame is `width` pixels wide: the near
/// gap up to 800 wide, plus one tile per 320 columns (or part) beyond.
///
/// The view's farthest tile from the player, along a tile axis, is
/// (half-width / 80 + half-height / 40) / 2 tiles: 6.15 at 800 × 600 and
/// 6.99 at 1068 × 600. Each 320 extra columns move it one tile further, so
/// the 16:9 frame needs rooms one tile beyond the near ring (gap 7) to
/// keep the 4:3 view's margin.
pub fn view_gap_for_width(width: i32) -> i32 {
    NEAR_GAP + ((width - 800).max(0) + 319) / 320
}

/// The view gap of a game whose frame is `width` x `height` pixels: the
/// near gap plus enough tiles along either projected isometric axis to
/// cover the whole frame. One extra tile needs 320 columns or 160 rows.
/// Positive growth on each axis is independent, so a short wide frame
/// keeps the established horizontal margin.
pub fn view_gap_for_frame(width: i32, height: i32) -> i32 {
    let extra = i64::from(width.saturating_sub(800).max(0))
        + 2 * i64::from(height.saturating_sub(600).max(0));
    (i64::from(NEAR_GAP) + (extra + 319) / 320).min(i64::from(i32::MAX)) as i32
}

/// The level whose rooms never get warp links.
pub const NO_WARP_LINK_LEVEL: u32 = 133;

/// A DRLG room (RoomEx): rect, seed, flags, status, neighbours, tiles and active room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrlgRoom {
    /// Level.
    pub level: LevelIdx,
    /// Room type.
    pub kind: RoomKind,
    /// Tile rect.
    pub rect: TileRect,
    /// Room seed.
    pub seed: Seed,
    /// `dwInitSeed`: the value the room seed is reset to when the room is built.
    pub init_seed: u32,
    /// Flags ([`room_flags`]).
    pub flags: u32,
    /// Other flags (bit 0: the room was populated).
    pub other_flags: u32,
    /// DT1 mask, set by the level-type generator.
    pub dt1_mask: u32,
    /// Status: 0..3, 4 none.
    pub status: u8,
    /// Status reference counts, one per status 0..3.
    pub counts: [u16; 4],
    /// Warp links, head first.
    pub warp_links: Vec<WarpLink>,
    pub(super) near: Option<Vec<DrlgRoomId>>,
    pub(super) next: Option<DrlgRoomId>,
    /// Tile library slots: DT1 file ids of the DRLG cache.
    pub(super) library: Vec<u32>,
    pub(super) tiles: Option<RoomTiles>,
    pub(super) active: Option<ActiveRoom>,
    /// Logical-room info, built with the tiles.
    pub(super) logic: Option<LogicInfo>,
}

impl DrlgRoom {
    /// The rooms-near array, if built.
    pub fn near(&self) -> Option<&[DrlgRoomId]> {
        self.near.as_deref()
    }

    /// The tile grid, if built.
    pub fn tiles(&self) -> Option<&RoomTiles> {
        self.tiles.as_ref()
    }

    /// The active room, if any.
    pub fn active(&self) -> Option<&ActiveRoom> {
        self.active.as_ref()
    }

    /// The logical-room info (coordinate lists), if built.
    pub fn logic(&self) -> Option<&LogicInfo> {
        self.logic.as_ref()
    }
}

/// Gaps of `r` relative to `a` in x and y; negative or zero when they overlap or touch.
pub fn near_gaps(a: &TileRect, r: &TileRect) -> (i32, i32) {
    let gx = if a.x < r.x {
        r.x - a.w - a.x
    } else {
        a.x - r.w - r.x
    };
    let gy = if a.y < r.y {
        r.y - a.h - a.y
    } else {
        a.y - r.h - r.y
    };
    (gx, gy)
}

/// Whether `r` is near `a`: both gaps are below `NEAR_GAP`.
pub fn is_near(a: &TileRect, r: &TileRect) -> bool {
    let (gx, gy) = near_gaps(a, r);
    gx < NEAR_GAP && gy < NEAR_GAP
}

/// The rooms-near bubble sort: n − 1 passes over i = 0..n−2, swapping when
/// `a[i].x ≥ a[i+1].x + a[i+1].w` or `a[i].y ≥ a[i+1].y + a[i+1].h`. This is not a
/// total order, so the loop itself defines the result.
pub fn sort_near<T: Copy>(items: &mut [T], rect: impl Fn(T) -> TileRect) {
    let n = items.len();
    for _ in 1..n {
        for i in 0..n - 1 {
            let (a, b) = (rect(items[i]), rect(items[i + 1]));
            if a.x >= b.x + b.w || a.y >= b.y + b.h {
                items.swap(i, i + 1);
            }
        }
    }
}

impl Drlg {
    // ---- room access ------------------------------------------------------

    /// The room with this id; panics if it has been freed.
    pub fn room(&self, id: DrlgRoomId) -> &DrlgRoom {
        self.rooms[id.0 as usize].as_ref().expect("live DRLG room")
    }

    /// The room with this id, mutable; panics if it has been freed.
    pub fn room_mut(&mut self, id: DrlgRoomId) -> &mut DrlgRoom {
        self.rooms[id.0 as usize].as_mut().expect("live DRLG room")
    }

    /// The room if it is live (not freed).
    pub fn try_room(&self, id: DrlgRoomId) -> Option<&DrlgRoom> {
        self.rooms.get(id.0 as usize)?.as_ref()
    }

    /// The status list `s` (0..3), head first.
    pub fn status_list(&self, s: u8) -> &[DrlgRoomId] {
        &self.status_lists[s as usize]
    }

    // ---- creation ----------------------------------------------------------

    /// Allocates a new room of `level` with status 4. Draws one level-seed step (room seed
    /// := `init_low(lo')`) and one room-seed step (`dwInitSeed` := `lo'`). The caller (the
    /// level-type generator) does the type data init, sets flags and DT1 mask, and links
    /// the room ([`Drlg::link_room`]).
    pub fn alloc_room(&mut self, level: LevelIdx, kind: RoomKind, rect: TileRect) -> DrlgRoomId {
        let seed = self.level_mut(level).seed.derive();
        let mut room = DrlgRoom {
            level,
            kind,
            rect,
            seed,
            init_seed: 0,
            flags: 0,
            other_flags: 0,
            dt1_mask: 0,
            status: STATUS_NONE,
            counts: [0; 4],
            warp_links: Vec::new(),
            near: None,
            next: None,
            library: Vec::new(),
            tiles: None,
            active: None,
            logic: None,
        };
        room.init_seed = room.seed.step();
        if self.level(level).flags & super::level::LEVEL_FLAG_CLIENT != 0 {
            room.flags |= room_flags::AUTOMAP_REVEAL;
        }
        let id = DrlgRoomId(self.rooms.len() as u32);
        self.rooms.push(Some(room));
        id
    }

    /// Adds a room to its level's list and counts it.
    pub fn link_room(&mut self, id: DrlgRoomId, at: LinkAt) {
        let l = self.room(id).level;
        match at {
            LinkAt::Head => {
                let head = self.level(l).first_room;
                self.room_mut(id).next = head;
                self.level_mut(l).first_room = Some(id);
            }
            LinkAt::Tail => match self.level_rooms(l).last() {
                Some(&last) => self.room_mut(last).next = Some(id),
                None => self.level_mut(l).first_room = Some(id),
            },
        }
        self.level_mut(l).room_count += 1;
    }

    /// Frees a room: it leaves its status list and its slot is released.
    pub(super) fn free_room(&mut self, id: DrlgRoomId) {
        let s = self.room(id).status;
        if s < STATUS_NONE {
            self.status_lists[s as usize].retain(|&r| r != id);
        }
        self.rooms[id.0 as usize] = None;
    }

    // ---- rooms-near arrays -----------------------------------------------

    /// Builds the room's rooms-near array, its warp links (which may generate linked
    /// levels) and the town-border flag.
    pub fn build_near(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        id: DrlgRoomId,
    ) -> Result<(), DrlgError> {
        let a = self.room(id).rect;
        let level = self.room(id).level;
        // Every candidate is kept; the array is not bounded.
        let mut near: Vec<DrlgRoomId> = self
            .level_rooms(level)
            .into_iter()
            .filter(|&r| is_near(&a, &self.room(r).rect))
            .collect();
        self.sort_rooms(&mut near);
        self.room_mut(id).near = Some(near);

        let level_id = self.level(level).id;
        if self.room(id).flags & room_flags::WARP_MASK != 0 && level_id != NO_WARP_LINK_LEVEL {
            self.build_warp_links(data, types, id, level_id)?;
        }

        // Town border: rooms next to a town get no population.
        if !is_town(level_id) {
            let near = self.room(id).near.clone().unwrap_or_default();
            if near
                .iter()
                .any(|&n| is_town(self.level(self.room(n).level).id))
            {
                self.room_mut(id).flags |= room_flags::NO_POPULATION;
            }
        }
        Ok(())
    }

    /// The view extras of room `id`: the rooms outside its near list with
    /// both gaps below [`Drlg::view_gap`], from its own level and the
    /// levels of its near rooms, in rooms-near order. They are what a view
    /// wider than 4:3 shows beyond the near ring. Empty when the view gap
    /// is the near gap (every 4:3 game) or the near list is not built.
    pub fn view_extras(&self, id: DrlgRoomId) -> Vec<DrlgRoomId> {
        if self.view_gap <= NEAR_GAP {
            return Vec::new();
        }
        let room = self.room(id);
        let Some(near) = room.near.as_deref() else {
            return Vec::new();
        };
        let a = room.rect;
        let mut levels = vec![room.level];
        for &n in near {
            if let Some(r) = self.try_room(n) {
                if !levels.contains(&r.level) {
                    levels.push(r.level);
                }
            }
        }
        let mut out = Vec::new();
        for l in levels {
            for r in self.level_rooms(l) {
                if r == id || near.contains(&r) || out.contains(&r) {
                    continue;
                }
                let (gx, gy) = near_gaps(&a, &self.room(r).rect);
                if gx < self.view_gap && gy < self.view_gap {
                    out.push(r);
                }
            }
        }
        self.sort_rooms(&mut out);
        out
    }

    /// The rooms a client in room `id` sees: the active room's adjacency
    /// array, then its active view extras ([`Drlg::view_extras`]). Just
    /// the adjacency array in a 4:3 game.
    pub fn view_rooms(&self, id: DrlgRoomId) -> Vec<DrlgRoomId> {
        let Some(a) = self.try_room(id).and_then(|r| r.active.as_ref()) else {
            return Vec::new();
        };
        let mut out = a.adjacency.clone();
        for v in self.view_extras(id) {
            if self.room(v).active.is_some() && !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }

    fn sort_rooms(&self, list: &mut [DrlgRoomId]) {
        sort_near(list, |r| self.room(r).rect);
    }

    fn append_near(&mut self, id: DrlgRoomId, t: DrlgRoomId) {
        let mut near = self.room_mut(id).near.take().unwrap_or_default();
        near.push(t);
        self.sort_rooms(&mut near);
        self.room_mut(id).near = Some(near);
    }

    /// Builds the warp links of a room: for each warp flag, links to the matching room of
    /// the Vis level (generating that level if needed).
    fn build_warp_links(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        id: DrlgRoomId,
        level_id: u32,
    ) -> Result<(), DrlgError> {
        let vis = self.vis_array(data, level_id)?;
        for i in 0..8 {
            if self.room(id).flags & (room_flags::WARP_0 << i) == 0 {
                continue;
            }
            let v = vis[i];
            let l = self.get_or_alloc_level(data, types, v)?;
            let w = self.warp_id(data, level_id, i)?;
            if self.level(l).first_room.is_none() {
                self.generate_level(data, types, l)?;
            }
            let c = (0..i).filter(|&j| vis[j] == v).count();
            let lvis = self.vis_array(data, v)?;
            let slots: Vec<usize> = (0..8).filter(|&m| lvis[m] == level_id).collect();
            let mut linked = false;
            if w != -1 {
                if let Some(&m) = slots.get(c) {
                    linked = self.try_warp_link(data, id, l, m, i, w, level_id)?;
                }
            }
            if !linked {
                for &m in &slots {
                    if self.try_warp_link(data, id, l, m, i, w, level_id)? {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// One link attempt for slot `m` of level `l`.
    #[allow(clippy::too_many_arguments)]
    fn try_warp_link(
        &mut self,
        data: &DrlgData,
        id: DrlgRoomId,
        l: LevelIdx,
        m: usize,
        i: usize,
        w: i32,
        level_id: u32,
    ) -> Result<bool, DrlgError> {
        let flag = room_flags::WARP_0 << m;
        let targets: Vec<DrlgRoomId> = self
            .level_rooms(l)
            .into_iter()
            .filter(|&t| self.room(t).flags & flag != 0)
            .collect();
        if w != -1 {
            let Some(&t) = targets.first() else {
                return Ok(false);
            };
            self.append_near(id, t);
            let row = self.lvlwarp_row(data, level_id, i, b'b')?;
            self.room_mut(id).warp_links.insert(
                0,
                WarpLink {
                    target: t,
                    enabled: true,
                    lvlwarp_row: row,
                },
            );
            return Ok(true);
        }
        let a = self.room(id).rect;
        let mut linked = false;
        for t in targets {
            if is_near(&a, &self.room(t).rect) {
                self.append_near(id, t);
                linked = true;
            }
        }
        Ok(linked)
    }

    // ---- statuses -----------------------------------------------------------

    /// Forces a status: unlinks the room, relinks it at the tail of list `s` (s < 4), and
    /// sets status := s.
    pub(super) fn force_status(&mut self, id: DrlgRoomId, s: u8) {
        let old = self.room(id).status;
        if old == s {
            return;
        }
        if old < STATUS_NONE {
            self.status_lists[old as usize].retain(|&r| r != id);
        }
        if s < STATUS_NONE {
            self.status_lists[s as usize].push(id);
        }
        self.room_mut(id).status = s;
    }

    /// Runs set handler `s`: what happens when a room first reaches status `s`.
    fn set_handler(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        s: u8,
    ) -> Result<(), DrlgError> {
        match s {
            0 => {
                // This handler follows D2MOO.
                if self.room(id).status > 0 {
                    self.force_status(id, 0);
                }
            }
            1 => {
                let r = self.room(id);
                if r.active.is_none() && r.flags & room_flags::HAS_ROOM == 0 {
                    self.bring_up(svc, id)?;
                    // The build timer restarts after a build.
                    self.build_timer = self.build_timer_reset();
                }
                if self.room(id).status > 1 {
                    self.force_status(id, 1);
                }
            }
            2 => {
                // This handler follows D2MOO.
                let r = self.room(id);
                let ready = r.flags & room_flags::TILE_LIB_LOADED != 0
                    && (r.kind != RoomKind::Preset
                        || r.flags & room_flags::PRESET_UNITS_ADDED != 0);
                if ready && r.status > 2 {
                    self.force_status(id, 2);
                    if self.room(id).counts[1] != 0 {
                        self.set_handler(svc, id, 1)?;
                    }
                }
            }
            _ => {
                self.load_room_data(svc, id)?;
                if self.room(id).status > 3 {
                    self.force_status(id, 3);
                }
            }
        }
        Ok(())
    }

    /// Recomputes a room's status from its reference counts: the lowest status with a
    /// non-zero count, or none.
    fn recompute_status(&mut self, id: DrlgRoomId) {
        let r = self.room(id);
        if r.status >= STATUS_NONE || r.counts[r.status as usize] == 0 {
            let s = (0..4u8)
                .find(|&s| r.counts[s as usize] != 0)
                .unwrap_or(STATUS_NONE);
            self.force_status(id, s);
        }
    }

    /// Runs unset handler `s`: what happens when a count at status `s` drops.
    fn unset_handler(&mut self, svc: &mut Services<'_>, id: DrlgRoomId, s: u8) {
        if s < 3 {
            // Handlers 0..2 follow D2MOO.
            self.recompute_status(id);
        } else if self.room(id).status != STATUS_NONE {
            self.recompute_status(id);
            if self.room(id).status == STATUS_NONE && self.on_client {
                self.free_room_tiles(svc, id);
            }
        }
    }

    /// Propagates status `s` to the rooms near `id` (recursing to `s + 1` below status 3).
    fn propagate(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        s: u8,
    ) -> Result<(), DrlgError> {
        if self.room(id).near.is_none() {
            self.build_near(svc.data, svc.types, id)?;
        }
        let mut i = 0;
        // Count and array re-read every iteration.
        while let Some(&n) = self.room(id).near.as_ref().and_then(|a| a.get(i)) {
            if s < 3 {
                self.propagate(svc, n, s + 1)?;
            }
            self.set_step(svc, n, s)?;
            i += 1;
        }
        Ok(())
    }

    /// Applies status `s` to one room: runs the set handler if no count at or below `s`
    /// holds it, then increments its count.
    fn set_step(&mut self, svc: &mut Services<'_>, n: DrlgRoomId, s: u8) -> Result<(), DrlgError> {
        let r = self.room(n);
        if r.status >= s && r.counts[..=s as usize].iter().all(|&c| c == 0) {
            self.set_handler(svc, n, s)?;
        }
        let c = &mut self.room_mut(n).counts[s as usize];
        *c = c.wrapping_add(1);
        Ok(())
    }

    /// Sets status `s` on a room and propagates `s + 1` to its neighbours.
    pub fn set_and_propagate(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
        s: u8,
    ) -> Result<(), DrlgError> {
        self.propagate(svc, id, s + 1)?;
        self.set_step(svc, id, s)
    }

    /// Undoes `propagate` for the rooms near `id`: decrements counts and runs unset
    /// handlers.
    pub fn unpropagate(&mut self, svc: &mut Services<'_>, id: DrlgRoomId, s: u8) {
        let mut i = 0;
        while let Some(&n) = self.room(id).near.as_ref().and_then(|a| a.get(i)) {
            let c = &mut self.room_mut(n).counts[s as usize];
            *c = c.wrapping_sub(1);
            self.unset_handler(svc, n, s);
            if s < 3 {
                self.unpropagate(svc, n, s + 1);
            }
            i += 1;
        }
    }

    /// DRLG status update for a client moving from `old` to `new`. See
    /// [`Drlg::client_changes_room`] for the whole server room change.
    pub fn change_status_room(
        &mut self,
        svc: &mut Services<'_>,
        old: Option<DrlgRoomId>,
        new: Option<DrlgRoomId>,
    ) -> Result<(), DrlgError> {
        if old == new {
            return Ok(());
        }
        if let Some(n) = new {
            self.set_and_propagate(svc, n, 0)?;
            // A view wider than 4:3: the room's view extras are held at
            // status 1 too (built and active, as a near room is), and the
            // rooms held are remembered so the leave releases exactly them.
            if !self.on_client && self.view_gap > NEAR_GAP {
                let extras = self.view_extras(n);
                for &v in &extras {
                    self.set_and_propagate(svc, v, 1)?;
                }
                self.view_holds.entry(n).or_default().push(extras);
            }
        }
        if let Some(o) = old {
            if self.room(o).counts[0] != 0 {
                self.room_mut(o).counts[0] -= 1;
                self.unset_handler(svc, o, 0);
                self.unpropagate(svc, o, 1);
                let held = self.view_holds.get_mut(&o).and_then(Vec::pop);
                if self.view_holds.get(&o).is_some_and(Vec::is_empty) {
                    self.view_holds.remove(&o);
                }
                for v in held.unwrap_or_default() {
                    if self.try_room(v).is_some_and(|r| r.counts[1] != 0) {
                        self.room_mut(v).counts[1] -= 1;
                        self.unset_handler(svc, v, 1);
                        self.unpropagate(svc, v, 2);
                    }
                }
            }
        }
        Ok(())
    }

    // ---- client in sight by coordinates ------------------------------------

    /// The room lookup for the in-sight calls: get-or-allocate the level `level_id`, then
    /// the room at tile (x, y) in it, with `hint` as the hint room only when it is in
    /// that level.
    fn sight_room(
        &mut self,
        svc: &mut Services<'_>,
        level_id: u32,
        x: i32,
        y: i32,
        hint: Option<DrlgRoomId>,
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        let l = self.get_or_alloc_level(svc.data, svc.types, level_id)?;
        let hint = hint.filter(|&h| self.try_room(h).is_some_and(|r| r.level == l));
        // A roomless level is generated here, with the client automap streaming since the
        // services are at hand.
        if self.level(l).first_room.is_none() {
            self.generate_level_svc(svc, l)?;
        }
        self.room_at(svc.data, svc.types, x, y, hint, Some(l))
    }

    /// Sets a client in sight by coordinates: if the room's status-1 count is 0,
    /// set-and-propagate it with status 1. Returns the room, `None` when the level has no
    /// room at the point.
    pub fn set_in_sight_at(
        &mut self,
        svc: &mut Services<'_>,
        level_id: u32,
        x: i32,
        y: i32,
        hint: Option<DrlgRoomId>,
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        let Some(id) = self.sight_room(svc, level_id, x, y, hint)? else {
            return Ok(None);
        };
        if self.room(id).counts[1] == 0 {
            self.set_and_propagate(svc, id, 1)?;
        }
        Ok(Some(id))
    }

    /// Unsets a client in sight by coordinates: the same lookup; a room whose status-1
    /// count is not 0 gets count − 1, unset handler 1 and unpropagate(room, 2). Returns
    /// the room, `None` as for [`Drlg::set_in_sight_at`].
    pub fn unset_in_sight_at(
        &mut self,
        svc: &mut Services<'_>,
        level_id: u32,
        x: i32,
        y: i32,
        hint: Option<DrlgRoomId>,
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        let Some(id) = self.sight_room(svc, level_id, x, y, hint)? else {
            return Ok(None);
        };
        if self.room(id).counts[1] != 0 {
            self.room_mut(id).counts[1] -= 1;
            self.unset_handler(svc, id, 1);
            self.unpropagate(svc, id, 2);
        }
        Ok(Some(id))
    }

    // ---- streaming and build ---------------------------------------------------

    /// Loads the room's DT1 tile library and adds its preset units (set handler 3).
    fn load_room_data(&mut self, svc: &mut Services<'_>, id: DrlgRoomId) -> Result<(), DrlgError> {
        if self.room(id).flags & room_flags::TILE_LIB_LOADED == 0 {
            self.load_library(svc, id)?;
        }
        let r = self.room(id);
        if r.kind == RoomKind::Preset && r.flags & room_flags::PRESET_UNITS_ADDED == 0 {
            svc.types.add_preset_units(self, id)?;
            self.room_mut(id).flags |= room_flags::PRESET_UNITS_ADDED;
        }
        Ok(())
    }

    /// Streams a room: loads data and builds it if needed, with no status change. Returns
    /// the active room.
    pub fn stream_room(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
    ) -> Result<Option<RoomId>, DrlgError> {
        self.bring_up(svc, id)?;
        Ok(self.room(id).active.as_ref().map(|a| a.id))
    }

    /// Loads a room's data and builds it (near array, seed reset, tiles, active room) if it
    /// has not been built.
    fn bring_up(&mut self, svc: &mut Services<'_>, id: DrlgRoomId) -> Result<(), DrlgError> {
        self.load_room_data(svc, id)?;
        if self.room(id).flags & room_flags::HAS_ROOM != 0 {
            return Ok(());
        }
        // The rooms-near array is needed for the adjacency of the active room.
        if self.room(id).near.is_none() {
            self.build_near(svc.data, svc.types, id)?;
        }
        // Reset the room seed and get the level type's source grids.
        let r = self.room_mut(id);
        r.seed = Seed::init_low(r.init_seed);
        let grids = svc.types.room_grids(self, svc.data, id)?;
        // Tiles.
        self.fill_room_tiles(svc, id, &grids)?;
        self.room_mut(id).flags |= room_flags::HAS_ROOM;
        // Active room.
        self.create_active_room(svc, id)?;
        // Build statistics.
        self.builds_since_update = self.builds_since_update.wrapping_add(1);
        self.rooms_built = self.rooms_built.wrapping_add(1);
        Ok(())
    }

    // ---- client build timer ------------------------------------------------

    /// Reset value of the build timer: 5 for a client copy, else 7.
    pub fn build_timer_reset(&self) -> u8 {
        if self.on_client {
            5
        } else {
            7
        }
    }

    /// The client build timer, run once per client update: builds at most one pending
    /// status-2 room every few updates. Returns the rooms it built, in order.
    pub fn client_build_timer(
        &mut self,
        svc: &mut Services<'_>,
    ) -> Result<Vec<DrlgRoomId>, DrlgError> {
        // Skip a round after builds elsewhere.
        if self.builds_since_update > 1 {
            self.builds_since_update = 0;
            return Ok(Vec::new());
        }
        // Count down; nothing is built until the timer runs out.
        self.build_timer = self.build_timer.wrapping_sub(1);
        if self.build_timer != 0 {
            return Ok(Vec::new());
        }
        // A cursor that is unset or on a room whose status is not 2 moves to the first
        // status-2 room (the head node when the list is empty). The head node reads as
        // status 2, so it is kept.
        self.build_timer = self.build_timer_reset();
        let list = self.status_lists[2].clone();
        let n = list.len();
        let start = match self.build_cursor {
            BuildCursor::Head => n,
            BuildCursor::Room(c) if self.try_room(c).is_some_and(|r| r.status == 2) => {
                list.iter().position(|&r| r == c).unwrap_or(0)
            }
            _ => 0,
        };
        // Circular walk from the cursor; index n (`None`) is the head node, never built.
        let walk = (0..=n).map(|k| {
            let i = (start + k) % (n + 1);
            list.get(i).copied()
        });
        let mut built = Vec::new();
        let mut next = BuildCursor::Head;
        for (k, node) in walk.enumerate() {
            if let Some(id) = node {
                let r = self.room(id);
                if r.active.is_none() && r.flags & room_flags::HAS_ROOM == 0 {
                    self.bring_up(svc, id)?;
                    built.push(id);
                }
            }
            // The cursor moves to the room after the last one examined.
            let after = (start + k + 1) % (n + 1);
            next = list
                .get(after)
                .map_or(BuildCursor::Head, |&r| BuildCursor::Room(r));
            if k + 1 == n + 1 || self.builds_since_update >= 1 {
                break;
            }
        }
        self.build_cursor = next;
        self.builds_since_update = 0;
        Ok(built)
    }
}

/// The membership changes of a live view resize, without moving the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewResize {
    pub client: crate::units::ClientId,
    pub anchor: DrlgRoomId,
    pub joined: Vec<DrlgRoomId>,
    pub left: Vec<DrlgRoomId>,
}

impl Drlg {
    /// Reconcile every retained view hold and client membership with a new
    /// gap. Replacement holds are acquired before old ones are released,
    /// including when several clients share an anchor. Player positions and
    /// near-room counts do not change.
    pub fn resize_view_gap(
        &mut self,
        svc: &mut Services<'_>,
        gap: i32,
        clients: &[(crate::units::ClientId, DrlgRoomId)],
    ) -> Result<Vec<ViewResize>, DrlgError> {
        let gap = gap.max(NEAR_GAP);
        if self.view_gap == gap {
            return Ok(Vec::new());
        }
        let before: Vec<_> = clients
            .iter()
            .map(|&(client, anchor)| {
                let rooms = self
                    .active_rooms()
                    .into_iter()
                    .filter_map(|(_, r)| {
                        self.active_room(r)
                            .is_some_and(|a| a.clients.contains(&client))
                            .then_some(r)
                    })
                    .collect::<Vec<_>>();
                (client, anchor, rooms)
            })
            .collect();
        let old_holds = self.view_holds.clone();
        let old_gap = self.view_gap;
        self.view_gap = gap;
        let mut replacements =
            std::collections::BTreeMap::<DrlgRoomId, Vec<Vec<DrlgRoomId>>>::new();
        let mut acquired = Vec::new();
        for &(_, anchor) in clients {
            let index = replacements.get(&anchor).map_or(0, Vec::len);
            let old = old_holds.get(&anchor).and_then(|v| v.get(index));
            let new = self.view_extras(anchor);
            for &room in &new {
                if !old.is_some_and(|v| v.contains(&room)) {
                    if let Err(e) = self.set_and_propagate(svc, room, 1) {
                        // The caller withholds its acknowledgement on failure.
                        for room in acquired {
                            self.room_mut(room).counts[1] -= 1;
                            self.unset_handler(svc, room, 1);
                            self.unpropagate(svc, room, 2);
                        }
                        self.view_gap = old_gap;
                        return Err(e);
                    }
                    acquired.push(room);
                }
            }
            replacements.entry(anchor).or_default().push(new);
        }
        let mut changes = Vec::new();
        for (client, anchor, old) in before {
            let new = self.view_rooms(anchor);
            let joined: Vec<_> = new.iter().copied().filter(|r| !old.contains(r)).collect();
            let left: Vec<_> = old.iter().copied().filter(|r| !new.contains(r)).collect();
            for &room in &joined {
                self.add_room_client(room, client);
            }
            for &room in &left {
                self.remove_room_client(room, client);
            }
            changes.push(ViewResize {
                client,
                anchor,
                joined,
                left,
            });
        }
        for (anchor, holds) in old_holds {
            for (index, old) in holds.into_iter().enumerate() {
                let new = replacements.get(&anchor).and_then(|v| v.get(index));
                for room in old {
                    if !new.is_some_and(|v| v.contains(&room)) {
                        self.room_mut(room).counts[1] -= 1;
                        self.unset_handler(svc, room, 1);
                        self.unpropagate(svc, room, 2);
                    }
                }
            }
        }
        self.view_holds = replacements;
        Ok(changes)
    }
}
