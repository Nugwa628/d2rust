//! The client DRLG copy: the `d2-sim` DRLG act
//! built from S→C 0x03's fields with the client flag, owned by the bridge
//! and never shared with the server's. 0x07 / 0x08 set and unset its
//! rooms in sight by coordinates; a room
//! in sight is built and becomes an active room, prepended to the act's
//! room list. [`ClientDrlg::active_rooms`] is that
//! list in list order, the input of the room of a point.
//! Each new active room is recorded for the act room callback
//! (the light cache, [`ClientDrlg::take_created`]),
//! and the client update runs the build timer and the level free
//! ([`ClientDrlg::client_update`]).
//!
//! The DRLG's table view, tile headers and level types are inputs
//! ([`DrlgSource`]), as the server's are; each act build gets its own
//! level-type state from the source's factory. Plain Rust, no Bevy.

use std::sync::Arc;

use d2_sim::drlg::tiles::{first_entry, rec_flags, RecordKind, RoomTiles, ACT_EDGE_TILE};
use d2_sim::drlg::{
    ActRooms, Drlg, DrlgData, DrlgError, LevelTypes, Services, TileInfo, TileRect, TileSource,
    WarpDef,
};
use d2_sim::rng::Seed;
use d2_sim::units::RoomId;

use super::dispatch::HandlerError;
use super::world::ActiveRoom;
use crate::rules::lighting::records::LightError;

/// A DRLG room of the client act, by slot (`d2_sim::drlg::DrlgRoomId`).
pub use d2_sim::drlg::DrlgRoomId;

/// The level types of one client act:
/// its own state, never the server's.
pub type ClientTypes = Box<dyn LevelTypes + Send + Sync>;

/// Builds the level-type state of a new client act.
pub type TypesFactory = Arc<dyn Fn() -> ClientTypes + Send + Sync>;

/// What the client DRLG reads that is not model state: the DRLG table
/// view, the DT1 tile headers and a factory of level-type state (the
/// same data the server's DRLG reads).
#[derive(Clone)]
pub struct DrlgSource {
    pub data: Arc<DrlgData>,
    pub tiles: Arc<dyn TileSource + Send + Sync>,
    pub types: TypesFactory,
}

impl std::fmt::Debug for DrlgSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrlgSource")
            .field("levels", &self.data.levels.len())
            .finish_non_exhaustive()
    }
}

/// A highlighted map record: its room, array and index in that array.
type WarpHighlightRecord = (DrlgRoomId, RecordKind, usize);

/// How many world tiles around a floor entrance's footprint its own wall
/// art can stand (a cave mouth's arch stands one or two tiles from the warp
/// floor).
const ENTRANCE_ART_REACH: i32 = 2;

/// The wall records that are the art of a floor entrance: the walls of the
/// DT1 files of its hidden warp floors (`files`) within
/// [`ENTRANCE_ART_REACH`] tiles of the floor `footprint` (world tiles,
/// `(x0, y0, x1, y1)` inclusive), with the chained half of a corner. A cave
/// entrance is a floor patch under a rock arch: the arch is what the
/// highlight follows, a sprite-shaped silhouette, not the ground under it.
fn entrance_art_walls(
    rooms: &[(DrlgRoomId, TileRect, &RoomTiles)],
    files: &[u32],
    footprint: (i32, i32, i32, i32),
) -> Vec<WarpHighlightRecord> {
    let (x0, y0, x1, y1) = footprint;
    let mut out: Vec<WarpHighlightRecord> = Vec::new();
    for &(id, rect, tiles) in rooms {
        for (i, t) in tiles.walls.iter().enumerate() {
            let (wx, wy) = (rect.x + t.x, rect.y + t.y);
            let near = (x0 - ENTRANCE_ART_REACH..=x1 + ENTRANCE_ART_REACH).contains(&wx)
                && (y0 - ENTRANCE_ART_REACH..=y1 + ENTRANCE_ART_REACH).contains(&wy);
            if !near || !files.contains(&t.tile.file) {
                continue;
            }
            for index in std::iter::once(i).chain(t.half) {
                let rec = (id, RecordKind::Wall, index);
                if !out.contains(&rec) {
                    out.push(rec);
                }
            }
        }
    }
    out
}

/// The map records of the warp entrance of warp tile `class` at world
/// sub-tile `position`, read from `rooms` (the tile's room and the rooms
/// near). A wall entrance is the exit wall records at its cell; a floor
/// entrance is the wall art standing around its hidden warp floors
/// ([`entrance_art_walls`]), else the four floor cells ending at its cell
/// (an exit with no art of its own). The candidate cell
/// is recovered from each lvlwarp row of the class: `position - offset` over
/// 5 is the world tile, and the row whose footprint holds the warp's records
/// (an exit wall or a hidden warp floor) wins, so the other direction's row
/// cannot shift the footprint. Records are returned with their room, since a
/// linked border record may live in a room near the tile's.
fn warp_highlight_records_in(
    warps: &[WarpDef],
    rooms: &[(DrlgRoomId, TileRect, &RoomTiles)],
    class: u32,
    position: (u16, u16),
) -> Vec<WarpHighlightRecord> {
    let (px, py) = (i32::from(position.0), i32::from(position.1));
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for w in warps.iter().filter(|w| w.id == (class & 0xff) as i32) {
        let x = px - w.offset_x;
        let y = py - w.offset_y;
        if x % 5 != 0 || y % 5 != 0 {
            continue;
        }
        let cell = (x / 5, y / 5);
        if !cells.contains(&cell) {
            cells.push(cell);
        }
    }

    let mut best: Option<(u8, Vec<WarpHighlightRecord>)> = None;
    for (cx, cy) in cells {
        let mut walls = Vec::new();
        let mut floors = Vec::new();
        let mut hidden_floor = false;
        // The DT1 files of the hidden warp floors, and the world tiles the
        // footprint's floors cover: the entrance's own art is found from them.
        let mut warp_files: Vec<u32> = Vec::new();
        let mut footprint: Option<(i32, i32, i32, i32)> = None;
        for &(id, rect, tiles) in rooms {
            let (rx, ry) = (cx - rect.x, cy - rect.y);
            for (i, t) in tiles.walls.iter().enumerate() {
                if t.x == rx && t.y == ry && matches!(t.kind, 10 | 11) {
                    let rec = (id, RecordKind::Wall, i);
                    if !walls.contains(&rec) {
                        walls.push(rec);
                    }
                }
            }
            for (i, t) in tiles.floors.iter().enumerate() {
                if (rx - 1..=rx).contains(&t.x) && (ry - 1..=ry).contains(&t.y) {
                    if t.flags & rec_flags::HIDDEN != 0 {
                        hidden_floor = true;
                        if !warp_files.contains(&t.tile.file) {
                            warp_files.push(t.tile.file);
                        }
                    }
                    let (wx, wy) = (rect.x + t.x, rect.y + t.y);
                    footprint = Some(footprint.map_or((wx, wy, wx, wy), |(x0, y0, x1, y1)| {
                        (x0.min(wx), y0.min(wy), x1.max(wx), y1.max(wy))
                    }));
                    let rec = (id, RecordKind::Floor, i);
                    if !floors.contains(&rec) {
                        floors.push(rec);
                    }
                }
            }
        }
        if walls.is_empty() && hidden_floor {
            if let Some(area) = footprint {
                walls = entrance_art_walls(rooms, &warp_files, area);
            }
        }
        let (score, records) = if !walls.is_empty() {
            (3, walls)
        } else if hidden_floor {
            (2, floors)
        } else if !floors.is_empty() {
            (1, floors)
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, records));
        }
    }
    best.map_or_else(Vec::new, |(_, records)| records)
}

/// The client act's room list (act +0x10, next active room +0x7C): new
/// active rooms are prepended, removed ones unlinked in place.
/// Record ids are
/// never reused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActList {
    /// Head first.
    pub rooms: Vec<RoomId>,
    next: u32,
    /// Records created since the last [`ClientDrlg::take_created`], in
    /// creation order (the act callback's calls).
    created: Vec<RoomId>,
    /// The records whose flags (+0x34) have bit 0 set: populated by the
    /// client room pass or created with it.
    pub populated: std::collections::BTreeSet<RoomId>,
}

impl ActRooms for ActList {
    fn create_active_room(&mut self, _act: u8, flags: u32) -> RoomId {
        let id = RoomId(self.next);
        self.next += 1;
        if flags & 1 != 0 {
            self.populated.insert(id);
        }
        self.rooms.insert(0, id);
        self.created.push(id);
        id
    }

    /// The adjacency array lives in the DRLG's active room;
    /// the list keeps no copy.
    fn set_adjacent(&mut self, _room: RoomId, _adjacent: &[RoomId]) {}

    fn remove_active_room(&mut self, room: RoomId) -> u32 {
        self.rooms.retain(|&r| r != room);
        u32::from(self.populated.remove(&room))
    }
}

/// A client DRLG operation failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClientDrlgError {
    /// A fatal error of the DRLG code, or a level-type error.
    #[error(transparent)]
    Drlg(#[from] DrlgError),
    /// A cloned [`ClientDrlg`] holds no level-type state and cannot
    /// generate (see [`ClientDrlg`]'s `Clone`).
    #[error("client DRLG snapshot: no level-type state")]
    Snapshot,
}

/// The act room callback's fatal cases as
/// handler errors.
impl From<LightError> for HandlerError {
    fn from(e: LightError) -> Self {
        match e {
            LightError::Fatal0x591 => HandlerError::Fatal(0x591),
            LightError::OwnerWithoutRoom => HandlerError::Unspecified(
                "render/lighting.md §6.4: a cached light record's owner has no room",
            ),
            _ => HandlerError::Invalid("light record error outside the new-room rule"),
        }
    }
}

/// The act's edge floor record (act +0x18,
/// open question 2): the
/// first entry of its fixed key in the act's base tile library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeTile {
    /// The base library's DT1 path and the tile's index in file order.
    pub path: Vec<u8>,
    pub index: u32,
    pub info: TileInfo,
}

/// The client DRLG act.
///
/// `Clone` makes a snapshot: the DRLG and the room list, without the
/// level-type state (which is not cloneable); a snapshot answers every
/// query, and an operation that needs the level types fails with
/// [`ClientDrlgError::Snapshot`]. Equality is that of the DRLG's whole
/// state (its `Debug` form) and the room list.
pub struct ClientDrlg {
    pub drlg: Drlg,
    pub list: ActList,
    types: Option<ClientTypes>,
    data: Arc<DrlgData>,
    tiles: Arc<dyn TileSource + Send + Sync>,
}

impl Clone for ClientDrlg {
    fn clone(&self) -> Self {
        Self {
            drlg: self.drlg.clone(),
            list: self.list.clone(),
            types: None,
            data: self.data.clone(),
            tiles: self.tiles.clone(),
        }
    }
}

impl PartialEq for ClientDrlg {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list && format!("{:?}", self.drlg) == format!("{:?}", other.drlg)
    }
}

impl Eq for ClientDrlg {}

impl std::fmt::Debug for ClientDrlg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientDrlg")
            .field("act", &self.drlg.act)
            .field("init_seed", &self.drlg.init_seed)
            .field("list", &self.list)
            .finish_non_exhaustive()
    }
}

impl ClientDrlg {
    /// The hover rectangle (`SelectX`, `SelectY`, `SelectDX`, `SelectDY`)
    /// of the lvlwarp record a warp tile of `class` names (matches the
    /// record `Id` against the tile's class byte, first row in file
    /// order).
    pub fn warp_select(&self, class: u32) -> Option<(i32, i32, i32, i32)> {
        self.data
            .warps
            .iter()
            .find(|w| w.id == (class & 0xFF) as i32)
            .map(|w| w.select)
    }

    /// The id of the level the warp of tile class `class` in DRLG room
    /// `room` leads to: the first warp link
    /// of the room (list order) whose lvlwarp record has `Id` = the
    /// class byte gives the destination room; its level's id. `None`: no
    /// such link.
    pub fn warp_dest_level(&self, room: DrlgRoomId, class: u32) -> Option<u32> {
        let link = self.drlg.room(room).warp_links.iter().find(|l| {
            self.data
                .warps
                .get(l.lvlwarp_row)
                .is_some_and(|w| w.id == (class & 0xFF) as i32)
        })?;
        let dest = self.drlg.room(link.target);
        Some(self.drlg.level(dest.level).id)
    }

    /// Map records belonging to a warp entrance, by room. A wall entrance is
    /// its exit wall records (the cells linked to a room near are included);
    /// a floor entrance is the wall art around its hidden warp floors, else the
    /// four floor cells ending at the preset's cell, recovered from the
    /// matching lvlwarp offsets. Hidden warp records are returned too in that
    /// case: the map draw skips them, the visible records beside them carry
    /// the highlight.
    pub fn warp_highlight_records(
        &self,
        room: DrlgRoomId,
        class: u32,
        position: (u16, u16),
    ) -> Vec<WarpHighlightRecord> {
        let rooms: Vec<(DrlgRoomId, TileRect, &RoomTiles)> = self
            .adjacency(room)
            .into_iter()
            .filter_map(|id| {
                let r = self.drlg.room(id);
                Some((id, r.rect, r.tiles()?))
            })
            .collect();
        warp_highlight_records_in(&self.data.warps, &rooms, class, position)
    }

    /// with the client flag:
    /// the DRLG of `act` from `init_seed`
    /// (0x03 u32@2) and the game's difficulty, town id 0 and DRLG flags 1
    /// (client copy), so no town is generated.
    pub fn build(
        src: &DrlgSource,
        act: u8,
        init_seed: u32,
        difficulty: u8,
    ) -> Result<Self, DrlgError> {
        let mut types = (src.types)();
        let mut drlg = Drlg::create(
            act,
            init_seed,
            difficulty,
            0,
            true,
            &src.data,
            types.as_mut(),
        )?;
        // The view gap of the frame, the value this client gave the game
        // at creation: which rooms beyond the near ring it draws.
        let frame = crate::rules::camera::FrameSize::play();
        drlg.view_gap = d2_sim::drlg::view_gap_for_frame(frame.width, frame.height);
        Ok(Self {
            drlg,
            list: ActList::default(),
            types: Some(types),
            data: src.data.clone(),
            tiles: src.tiles.clone(),
        })
    }

    fn split(&mut self) -> Result<(&mut Drlg, Services<'_>), ClientDrlgError> {
        let types = self.types.as_mut().ok_or(ClientDrlgError::Snapshot)?;
        Ok((
            &mut self.drlg,
            Services {
                data: &self.data,
                tiles: &*self.tiles,
                types: types.as_mut(),
                rooms: &mut self.list,
            },
        ))
    }

    /// 0x07: the room at tile (x, y)
    /// of level `level` set in sight with propagation when its status-1
    /// count is 0; `hint` is the local player's DRLG room. `None`: the
    /// level has no room at the point.
    pub fn set_in_sight(
        &mut self,
        level: u8,
        x: u16,
        y: u16,
        hint: Option<DrlgRoomId>,
    ) -> Result<Option<DrlgRoomId>, ClientDrlgError> {
        let (d, mut svc) = self.split()?;
        Ok(d.set_in_sight_at(&mut svc, level.into(), x.into(), y.into(), hint)?)
    }

    /// 0x08.
    pub fn unset_in_sight(
        &mut self,
        level: u8,
        x: u16,
        y: u16,
        hint: Option<DrlgRoomId>,
    ) -> Result<Option<DrlgRoomId>, ClientDrlgError> {
        let (d, mut svc) = self.split()?;
        Ok(d.unset_in_sight_at(&mut svc, level.into(), x.into(), y.into(), hint)?)
    }

    /// The client update's DRLG part (step 1): the build timer (steps
    /// 4–8), then, when
    /// `free_levels` (every 13th call, last paragraph), the level
    /// free (step 2). Returns the rooms the
    /// timer built.
    pub fn client_update(&mut self, free_levels: bool) -> Result<Vec<DrlgRoomId>, ClientDrlgError> {
        let (d, mut svc) = self.split()?;
        let built = d.client_build_timer(&mut svc)?;
        if free_levels {
            d.free_inactive_levels(svc.data, svc.types)?;
        }
        Ok(built)
    }

    /// The animated-tile tick of the client update over the near rooms
    /// of `room`, the
    /// local player's room.
    pub fn animate_tiles(&mut self, room: DrlgRoomId) {
        self.drlg.animate_tiles(room);
    }

    /// The local player's room change: the phase copy from `old` (none
    /// when it is not an active room of the act any more) to `new`.
    pub fn copy_anim_phase(&mut self, old: Option<DrlgRoomId>, new: DrlgRoomId) {
        let old = old.filter(|&o| self.drlg.active_room(o).is_some());
        self.drlg.copy_anim_phase(old, new);
    }

    /// The DRLG rooms whose active rooms were created since the last call,
    /// in creation order (the act room callback runs once
    /// for each). A record already removed again is
    /// left out.
    pub fn take_created(&mut self) -> Vec<DrlgRoomId> {
        std::mem::take(&mut self.list.created)
            .into_iter()
            .filter_map(|id| self.drlg.drlg_room_of(id))
            .collect()
    }

    /// The act's active rooms in list order (act +0x10, newest first):
    /// each one's sub-tile rectangle (+0x4C…+0x58), its level id and its
    /// DRLG room.
    pub fn active_rooms(&self) -> Vec<ActiveRoom> {
        self.list
            .rooms
            .iter()
            .map(|&id| {
                let room = self
                    .drlg
                    .drlg_room_of(id)
                    .expect("every listed record has its DRLG room");
                let a = self
                    .drlg
                    .active_room(room)
                    .expect("listed rooms are active");
                let level = self.drlg.level(self.drlg.room(room).level).id;
                ActiveRoom {
                    x0: a.subtiles.x,
                    y0: a.subtiles.y,
                    w: a.subtiles.w,
                    h: a.subtiles.h,
                    level: level as u16,
                    room,
                }
            })
            .collect()
    }

    /// The act's edge floor record ([`EdgeTile`]): `Ok(None)` for acts IV
    /// and V (no lookup, the record stays zero). A base library without
    /// the key is fatal 0x44C; a base library the tile source lacks is an
    /// input error.
    pub fn edge_tile(&self) -> Result<Option<EdgeTile>, String> {
        let Some(&Some((path, key))) = ACT_EDGE_TILE.get(usize::from(self.drlg.act)) else {
            return Ok(None);
        };
        let name = || String::from_utf8_lossy(path).into_owned();
        let tiles = self
            .tiles
            .dt1(path)
            .ok_or_else(|| format!("act base library {} not loaded", name()))?;
        let index = first_entry(tiles, key)
            .ok_or_else(|| format!("fatal 0x44C: no tile {key:?} in {}", name()))?;
        Ok(Some(EdgeTile {
            path: path.to_vec(),
            index,
            info: tiles[index as usize].clone(),
        }))
    }

    /// The adjacency array of an active DRLG room (room order;
    /// it holds the room itself), as DRLG rooms. Empty for a room
    /// without an active room.
    pub fn adjacency(&self, room: DrlgRoomId) -> Vec<DrlgRoomId> {
        self.drlg
            .active_room(room)
            .map_or_else(Vec::new, |a| a.adjacency.clone())
    }

    /// The rooms seen from `room` ([`Drlg::view_rooms`]): its adjacency
    /// array, then, in a view wider than 4:3, the built rooms beyond it up
    /// to the frame's edges. What the world view draws.
    pub fn view_rooms(&self, room: DrlgRoomId) -> Vec<DrlgRoomId> {
        self.drlg.view_rooms(room)
    }

    /// The client presets of an active DRLG room (its DS1 preset units
    /// with flag bit 0, in list order,
    /// room-relative). None without the level-type state (a snapshot).
    pub fn client_presets(&self, room: DrlgRoomId) -> Vec<d2_sim::drlg::ClientPreset> {
        self.types
            .as_ref()
            .map_or_else(Vec::new, |t| t.client_presets(&self.drlg, room))
    }

    /// The active room `room` populated by the client room pass
    /// (active-room flags +0x34 bit 0).
    pub fn populated(&self, room: DrlgRoomId) -> bool {
        self.drlg
            .active_room(room)
            .is_some_and(|a| self.list.populated.contains(&a.id))
    }

    /// Sets bit 0 of the active room's flags.
    pub fn set_populated(&mut self, room: DrlgRoomId) {
        if let Some(a) = self.drlg.active_room(room) {
            self.list.populated.insert(a.id);
        }
    }

    /// The active room's seed (+0x6C), the critter pass's R.
    pub fn room_seed(&mut self, room: DrlgRoomId) -> Option<&mut Seed> {
        self.drlg.active_room_seed_mut(room)
    }

    /// The unit seed of a unit created in `room` (,
    /// Randomness step 1): the active room's seed (+0x6C) is
    /// stepped once and the unit seed is `init_low(lo')`. `None` for a
    /// room without an active room.
    pub fn unit_seed(&mut self, room: DrlgRoomId) -> Option<Seed> {
        self.drlg.active_room_seed_mut(room).map(Seed::derive)
    }
}

#[cfg(test)]
mod warp_highlight_tests {
    use super::*;
    use d2_sim::drlg::tiles::{TileRecord, TileRef};

    fn rec(x: i32, y: i32, kind: u32, flags: u32) -> TileRecord {
        TileRecord {
            x,
            y,
            kind,
            flags,
            tile: TileRef { file: 0, index: 0 },
            cell: 0,
            half: None,
        }
    }

    fn room(floors: Vec<TileRecord>, walls: Vec<TileRecord>) -> RoomTiles {
        RoomTiles {
            floors,
            walls,
            ..RoomTiles::default()
        }
    }

    fn room_slices(
        records: &[(DrlgRoomId, TileRect, RoomTiles)],
    ) -> Vec<(DrlgRoomId, TileRect, &RoomTiles)> {
        records
            .iter()
            .map(|(id, rect, tiles)| (*id, *rect, tiles))
            .collect()
    }

    fn warp(id: i32, offset_x: i32, offset_y: i32) -> WarpDef {
        WarpDef {
            id,
            offset_x,
            offset_y,
            ..WarpDef::default()
        }
    }

    /// A floor entrance is the four cells ending at the preset's cell,
    /// recovered through the lvlwarp offsets; the hidden warp records and the
    /// visible floor records beside them both come back, so a highlight lands
    /// on a drawn record.
    #[test]
    fn a_floor_entrance_uses_the_offset_cell_and_its_visible_footprint() {
        let rect = TileRect::new(100, 200, 8, 8);
        let mut floors: Vec<TileRecord> = Vec::new();
        for (x, y) in [(2, 4), (3, 4), (2, 5), (3, 5)] {
            floors.push(rec(x, y, 0, rec_flags::HIDDEN));
            floors.push(rec(x, y, 0, 0));
        }
        floors.push(rec(4, 6, 0, 0));
        let tiles = room(floors, Vec::new());
        let records = [(DrlgRoomId(7), rect, tiles)];
        let rooms = room_slices(&records);
        // lx, ly = (3, 5) with the tile's row offset (-1, 1).
        let got = warp_highlight_records_in(
            &[warp(2, -1, 1)],
            &rooms,
            2,
            (((100 + 3) * 5 - 1) as u16, ((200 + 5) * 5 + 1) as u16),
        );
        assert_eq!(got.len(), 8, "{got:?}");
        assert!(got
            .iter()
            .all(|r| r.0 == DrlgRoomId(7) && r.1 == RecordKind::Floor));
        assert!(
            got.iter().all(|r| r.2 != 8),
            "a record outside the footprint: {got:?}"
        );
        for i in 0..8 {
            assert!(
                got.contains(&(DrlgRoomId(7), RecordKind::Floor, i)),
                "{i}: {got:?}"
            );
        }
    }

    /// The exit wall record of a border cell may be linked into a room near
    /// the warp tile's room; both halves come back and an unrelated exit wall
    /// of the same room does not.
    #[test]
    fn a_wall_entrance_keeps_both_halves_across_a_room_border() {
        let a = TileRect::new(10, 10, 9, 8);
        let b = TileRect::new(18, 10, 8, 8);
        // World cell (18, 12): a relative (8, 2), b relative (0, 2).
        let a_tiles = room(Vec::new(), vec![rec(8, 2, 10, 0), rec(2, 2, 10, 0)]);
        let b_tiles = room(Vec::new(), vec![rec(0, 2, 11, 0)]);
        let records = [(DrlgRoomId(1), a, a_tiles), (DrlgRoomId(2), b, b_tiles)];
        let rooms = room_slices(&records);
        let got = warp_highlight_records_in(
            &[warp(7, 0, 0)],
            &rooms,
            7,
            ((18 * 5) as u16, (12 * 5) as u16),
        );
        assert_eq!(
            got,
            vec![
                (DrlgRoomId(1), RecordKind::Wall, 0),
                (DrlgRoomId(2), RecordKind::Wall, 0),
            ]
        );
    }

    /// Two lvlwarp rows of the class name different cells; the row whose
    /// footprint holds the hidden warp floors wins, so a direction's offset
    /// cannot move the highlight.
    #[test]
    fn another_direction_row_cannot_shift_the_floor_footprint() {
        let rect = TileRect::new(0, 0, 8, 8);
        let mut floors = Vec::new();
        for (x, y) in [(1, 2), (2, 2), (1, 3), (2, 3)] {
            floors.push(rec(x, y, 0, rec_flags::HIDDEN));
        }
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            floors.push(rec(x, y, 0, 0));
        }
        let tiles = room(floors, Vec::new());
        let records = [(DrlgRoomId(3), rect, tiles)];
        let rooms = room_slices(&records);
        // lx, ly = (2, 3); the wrong row's offset_y 11 names ly = 1.
        let got = warp_highlight_records_in(
            &[warp(9, -1, 1), warp(9, -1, 11)],
            &rooms,
            9,
            ((2 * 5 - 1) as u16, (3 * 5 + 1) as u16),
        );
        assert_eq!(got.len(), 4, "{got:?}");
        let recs: Vec<usize> = got.iter().map(|r| r.2).collect();
        assert_eq!(recs, vec![0, 1, 2, 3], "{got:?}");
    }

    /// A cave mouth is a floor patch under a rock arch of walls from the same
    /// DT1 file: the highlight follows the arch (a sprite silhouette), not the
    /// ground. Walls of other files and walls far from the footprint stay out.
    #[test]
    fn a_floor_entrance_with_wall_art_highlights_the_art_not_the_ground() {
        let with_file = |r: TileRecord, file: u32| TileRecord {
            tile: TileRef { file, index: 0 },
            ..r
        };
        let rect = TileRect::new(100, 200, 12, 12);
        let mut floors: Vec<TileRecord> = Vec::new();
        for (x, y) in [(5, 5), (6, 5), (5, 6), (6, 6)] {
            floors.push(with_file(rec(x, y, 0, rec_flags::HIDDEN), 17));
            floors.push(with_file(rec(x, y, 0, 0), 17));
        }
        let mut arch_corner = with_file(rec(5, 3, 3, 0), 17);
        arch_corner.half = Some(4);
        let walls = vec![
            with_file(rec(6, 4, 2, 0), 17),   // arch, two tiles up
            with_file(rec(4, 5, 1, 0), 17),   // arch, left of the footprint
            with_file(rec(5, 4, 2, 0), 6),    // a tree wall from another file
            with_file(rec(11, 11, 2, 0), 17), // same file, far away
            arch_corner,                      // arch corner ...
            with_file(rec(5, 3, 4, 0), 17),   // ... and its chained half
        ];
        let tiles = room(floors, walls);
        let records = [(DrlgRoomId(9), rect, tiles)];
        let rooms = room_slices(&records);
        // lx, ly = (6, 6) with the tile's row offset (-1, 1).
        let got = warp_highlight_records_in(
            &[warp(4, -1, 1)],
            &rooms,
            4,
            (((100 + 6) * 5 - 1) as u16, ((200 + 6) * 5 + 1) as u16),
        );
        assert!(
            got.iter().all(|r| r.1 == RecordKind::Wall),
            "no ground record: {got:?}"
        );
        let mut idx: Vec<usize> = got.iter().map(|r| r.2).collect();
        idx.sort_unstable();
        assert_eq!(idx, vec![0, 1, 4, 5], "{got:?}");
    }

    #[test]
    fn an_unknown_warp_class_has_no_records() {
        let tiles = room(vec![rec(0, 0, 0, 0)], Vec::new());
        let records = [(DrlgRoomId(1), TileRect::new(0, 0, 8, 8), tiles)];
        let rooms = room_slices(&records);
        assert!(warp_highlight_records_in(&[warp(7, 0, 0)], &rooms, 8, (0, 0)).is_empty());
    }
}
